//! 历史版本与可选字段矩阵；仅修改合成SQLite，不生成或提升真实收据。
use super::*;

struct History {
    f: Fixture,
    attempt: String,
    run: String,
    context: ReviewRevisionContext,
    usage: Value,
}

fn persist(history: &History) {
    let raw: String = history
        .f
        .conn
        .query_row(
            "SELECT binding_json FROM generation_runs WHERE run_id=?1",
            [&history.run],
            |r| r.get(0),
        )
        .unwrap();
    let mut binding: Value = serde_json::from_str(&raw).unwrap();
    let text = serde_json::to_string(&history.context).unwrap();
    let hash = text_hash(&text);
    binding["sourceManifest"]["reviewRevisionContext"] =
        serde_json::to_value(&history.context).unwrap();
    binding["sourceManifest"]["reviewRevisionContextHash"] = json!(hash);
    binding["sourceManifest"]["authorInputs"][0]["text"] = json!(text);
    let mut usage = history.usage.clone();
    usage["reviewRevisionEffect"]["contextHash"] = json!(hash);
    history
        .f
        .conn
        .execute(
            "UPDATE generation_runs SET binding_json=?1 WHERE run_id=?2",
            rusqlite::params![binding.to_string(), history.run],
        )
        .unwrap();
    history
        .f
        .conn
        .execute(
            "UPDATE generation_attempts SET usage_receipt_json=?1 WHERE attempt_id=?2",
            rusqlite::params![usage.to_string(), history.attempt],
        )
        .unwrap();
}

fn history_fixture(context_version: i64, receipt_version: i64, report_version: i64) -> History {
    let (f, cycle) = mixed_merged_fixture();
    recheck_mixed(&f, &cycle, false);
    let (attempt, run, binding, raw_usage, artifact): (String,String,String,String,String) = f.conn.query_row(
        "SELECT a.attempt_id,a.run_id,r.binding_json,a.usage_receipt_json,json_extract(v.artifact_json,'$.text')
         FROM generation_attempts a JOIN generation_runs r ON r.run_id=a.run_id
         JOIN generation_artifacts v ON v.attempt_id=a.attempt_id
         WHERE json_extract(a.usage_receipt_json,'$.reviewCycleRecheck.cycleId')=?1",
        [&cycle], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).unwrap();
    let binding: Value = serde_json::from_str(&binding).unwrap();
    let mut context: ReviewRevisionContext =
        serde_json::from_value(binding["sourceManifest"]["reviewRevisionContext"].clone()).unwrap();
    context.recheck.as_mut().unwrap().version = context_version;
    let policy = RecheckPolicy {
        context: recheck_policy_context(context.recheck.as_ref().unwrap()),
        version: receipt_version,
    };
    let report = build_review_cycle_recheck_report(
        &artifact,
        &context.source.content,
        &policy,
        &context.ui_locale,
        report_version,
    );
    let body = serde_json::to_string_pretty(&ReviewReportBody {
        summary: report.summary,
        items: report.items.clone(),
    })
    .unwrap();
    let mut mappings = Vec::new();
    for decision in report.decisions.iter().filter(|d| d.status != "unknown") {
        let item = &report.items[decision.review_item_index as usize];
        let resolved = decision.status == "resolved";
        let evidence = recheck_evidence_hash(
            &decision.finding_id,
            &decision.target_id,
            decision.review_item_index,
            item,
            resolved,
        );
        mappings.push(json!({"findingId":decision.finding_id,"targetId":decision.target_id,
            "reviewItemIndex":decision.review_item_index,"evidenceHash":evidence,"resolved":resolved}));
        if !policy
            .context
            .findings
            .iter()
            .any(|f| f.finding_id == decision.finding_id && f.must_show == Some(true))
        {
            f.conn
                .execute(
                    "UPDATE review_findings SET status=?1,evidence_hash=?2 WHERE finding_id=?3",
                    rusqlite::params![decision.status, evidence, decision.finding_id],
                )
                .unwrap();
        }
    }
    let mut usage: Value = serde_json::from_str(&raw_usage).unwrap();
    usage["reviewCycleRecheck"]["version"] = json!(receipt_version);
    usage["reviewCycleRecheck"]["findings"] = json!(mappings);
    usage["reviewRevisionEffect"]["contentHash"] = json!(text_hash(&body));
    if report_version == 1 {
        usage["reviewRevisionEffect"]
            .as_object_mut()
            .unwrap()
            .remove("reportVersion");
    }
    let review_id = usage["reviewRevisionEffect"]["id"].as_i64().unwrap();
    f.conn
        .execute(
            "UPDATE contents SET body=?1 WHERE id=(SELECT content_id FROM reviews WHERE id=?2)",
            rusqlite::params![body, review_id],
        )
        .unwrap();
    let history = History {
        f,
        attempt,
        run,
        context,
        usage,
    };
    persist(&history);
    assert!(review_cycle::verify_m03_review_cycle(&history.f.conn));
    history
}

fn assert_saved_rejected(history: &History) {
    let run = history
        .f
        .service
        .runs
        .get(&history.f.conn, &history.run)
        .unwrap();
    let saved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        history.f.service.saved(&history.f.conn, &run)
    }));
    assert!(saved.is_ok(), "不可信历史应返回错误而不是panic");
    assert!(
        saved.unwrap().is_err(),
        "不能把未知或矛盾的复核策略静默当成已保存效果"
    );
    assert_eq!(history.f.count("generation_attempts"), 3);
}

#[test]
fn historical_review_accepts_known_report_and_policy_combinations_read_only() {
    for (context, receipt) in [(1, 1), (1, 2), (2, 2)] {
        for report in [1, 2] {
            let h = history_fixture(context, receipt, report);
            let run = h.f.service.runs.get(&h.f.conn, &h.run).unwrap();
            let recovery =
                h.f.service
                    .read_recovery(
                        &h.f.conn,
                        &ReviewRevisionGeneration::handle_of(&run),
                        |_| false,
                        |_| None,
                    )
                    .unwrap();
            assert!(recovery.saved.is_some());
            assert!(!recovery.can_resume);
            assert!(recovery.context_id.is_none());
            assert_eq!(h.f.count("generation_attempts"), 3);
        }
    }
}

#[test]
fn historical_review_rejects_unknown_frozen_context_in_proof_and_saved_ack() {
    let mut h = history_fixture(2, 2, 2);
    for version in [0, 3, -1] {
        h.context.recheck.as_mut().unwrap().version = version;
        persist(&h);
        assert!(!review_cycle::verify_m03_review_cycle(&h.f.conn));
        assert_saved_rejected(&h);
    }
}

#[test]
fn historical_review_rejects_missing_invalid_or_downgraded_saved_policy() {
    let mut h = history_fixture(2, 2, 2);
    let original = h.usage.clone();
    for version in [Value::Null, json!(0), json!(1), json!(3), json!("2")] {
        h.usage = original.clone();
        h.usage["reviewCycleRecheck"]["version"] = version;
        persist(&h);
        assert!(!review_cycle::verify_m03_review_cycle(&h.f.conn));
        assert_saved_rejected(&h);
    }
    for remove_version_only in [true, false] {
        h.usage = original.clone();
        if remove_version_only {
            h.usage["reviewCycleRecheck"]
                .as_object_mut()
                .unwrap()
                .remove("version");
        } else {
            h.usage
                .as_object_mut()
                .unwrap()
                .remove("reviewCycleRecheck");
        }
        persist(&h);
        assert!(!review_cycle::verify_m03_review_cycle(&h.f.conn));
        assert_saved_rejected(&h);
    }
}

#[test]
fn historical_review_preserves_optional_field_contract_without_inventing_evidence() {
    for version in [1, 2] {
        let mut h = history_fixture(version, version, 1);
        let original = h.context.clone();
        let recheck = h.context.recheck.as_ref().unwrap();
        assert!(recheck.source_content.is_some(), "必现项需要冻结原稿");
        assert!(recheck.findings.iter().any(|f| f.must_show.is_none()));
        assert!(recheck.findings.iter().any(|f| f.source_span.is_none()));
        for omit_source_content in [true, false] {
            h.context = original.clone();
            let recheck = h.context.recheck.as_mut().unwrap();
            if omit_source_content {
                recheck.source_content = None;
            } else {
                let anchored = recheck
                    .findings
                    .iter_mut()
                    .find(|f| f.source_span.is_some())
                    .unwrap();
                anchored.source_excerpt = None;
            }
            persist(&h);
            assert!(
                !review_cycle::verify_m03_review_cycle(&h.f.conn),
                "历史版本不能省略有实际来源的证据字段来伪装可选缺省"
            );
        }
        h.context = original;
        persist(&h);
        assert!(review_cycle::verify_m03_review_cycle(&h.f.conn));
    }
}

#[test]
fn historical_review_invalid_initial_artifact_returns_error_without_business_writes() {
    for text in ["不是JSON", "[]", "{}", r#"{"items":[{}]}"#] {
        let f = Fixture::new(ReviewRevisionOperation::ReviewChapter);
        let artifact = append_artifact(&f, &f.run, text, "stop");
        let request = ReviewGenerationCommitRequest {
            context_id: f.prepared.context_id.clone(),
            handle: ReviewRevisionGeneration::handle_of(&f.run),
            artifact: ReviewArtifactRef {
                artifact_id: artifact.artifact_id,
                revision: artifact.revision,
                text_hash: artifact.text_hash,
            },
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.service.commit_review(&f.conn, &request, |_| Ok(()))
        }));
        assert!(result.is_ok(), "首次提交也必须失败关闭，不能panic");
        let error = result.unwrap().unwrap_err();
        assert_eq!(error, "Error: GENERATION_REVIEW_ARTIFACT_INVALID");
        assert_eq!(f.count("reviews"), 0);
        assert_eq!(f.count("review_cycles"), 0);
        assert_eq!(f.count("review_findings"), 0);
        let effects: i64 = f.conn.query_row(
            "SELECT COUNT(*) FROM generation_attempts WHERE json_extract(usage_receipt_json,'$.reviewRevisionEffect') IS NOT NULL",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(effects, 0);
    }
}
