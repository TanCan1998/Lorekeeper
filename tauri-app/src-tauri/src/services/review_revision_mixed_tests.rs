//! 必现、已锚定目标与作者豁免混合链；全部材料为合成数据。
use super::*;

fn mixed_merged_fixture() -> (Fixture, String) {
    let f = Fixture::with_report(
        ReviewRevisionOperation::ReviewChapter,
        Some(("调整合成目标", "甲\r\n乙")),
        Some(json!({
            "worldSetting":"【第1章必现】合成信号灯亮起",
            "items":[{"category":"文风","severity":"warning","description":"合成文风建议","quote":"e\u{301}"}],
            "goalReviews":[
                {"id":"ch1:keyEvents:1","status":"unmet","description":"合成目标待调整","evidence":[{"quote":"甲\r\n乙"}]},
                {"id":"ch1:mustShow:1","status":"unknown","description":"原稿未展示","evidence":[]}
            ]
        })),
    );
    let reviewed = f.review().unwrap();
    let report: Value = serde_json::from_str(&reviewed.content).unwrap();
    let cycle: String = f
        .conn
        .query_row("SELECT cycle_id FROM review_cycles", [], |r| r.get(0))
        .unwrap();
    assert_eq!(f.count("review_findings"), 3);
    let items: Vec<Value> = report["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["severity"] != "pass")
        .map(|item| {
            let finding: String = f
                .conn
                .query_row(
                    "SELECT finding_id FROM review_findings WHERE problem_text=?1",
                    [item["description"].as_str().unwrap()],
                    |r| r.get(0),
                )
                .unwrap();
            let mut chosen = item.clone();
            chosen["origin"] = json!("ai");
            chosen["decision"] = json!(if item["category"] == "文风" {
                "waive"
            } else {
                "apply"
            });
            chosen["findingId"] = json!(finding);
            chosen
        })
        .collect();
    let value = json!({
        "kind":"human-confirmed-review","schemaVersion":2,"cycleId":cycle,
        "sourceReviewId":reviewed.id,"sourceDraft":reviewed.source,"goalReview":report["goalReview"],
        "summary":"作者确认合成目标并豁免文风建议","authorGuidance":"保留文风，只调整目标",
        "items":items
    });
    let snapshot: crate::services::review_revision_context::HumanConfirmedReviewSnapshot =
        serde_json::from_value(value).unwrap();
    let confirmation = serde_json::to_string_pretty(&snapshot).unwrap();
    let confirmed = review_repo::create(
        &f.conn,
        &ReviewCreateParams {
            base_draft_id: reviewed.source.id,
            review_index: None,
            content: confirmation.clone(),
            expected_source: Some(reviewed.source.clone()),
        },
    )
    .unwrap();
    let prepared = prepare(
        &f,
        &reviewed.source,
        ReviewRevisionOperation::RefineFromReview,
        Some((confirmed.id, confirmation)),
        None,
    );
    let revised =
        "🔑开场。甲与乙完成合成修订。合成信号灯亮起，两人继续核对合成材料。e\u{301}尾声。";
    let (run, artifact) = generate(&f, &prepared, revised);
    let composition = f
        .service
        .runs
        .compose_visible(
            &f.conn,
            &run.run_id,
            &[artifact.artifact_id],
            None,
            "visible-append-v1",
        )
        .unwrap();
    let saved = f
        .service
        .commit_revision(
            &f.conn,
            &RevisionGenerationCommitRequest {
                context_id: prepared.context_id,
                handle: ReviewRevisionGeneration::handle_of(&run),
                expected_composition_hash: composition.text_hash,
            },
            |_| Ok(()),
        )
        .unwrap();
    revision_repo::merge_into_draft(
        &f.conn,
        &revision_repo::MergeRevisionRequest {
            revision_id: saved.id,
            target_draft_id: reviewed.source.id,
            expected_draft_content: reviewed.source.content,
            merged_content: revised.into(),
            word_count: count_draft_units(revised),
        },
    )
    .unwrap();
    assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    (f, cycle)
}

fn recheck_mixed(f: &Fixture, cycle: &str, resolved: bool) -> ReviewRevisionCommitReceipt {
    let plan = review_cycle::plan_recheck(cycle, &f.conn).unwrap();
    let context = plan.context.unwrap();
    assert_eq!(context.findings.len(), 2, "作者豁免不能再次进入复核");
    assert_eq!(
        context
            .findings
            .iter()
            .filter(|f| f.must_show == Some(true))
            .count(),
        1
    );
    let row =
        crate::repositories::draft_repository::get_full(&f.conn, f.prepared.context.source.id)
            .unwrap()
            .unwrap();
    let source = ExpectedDraftSource {
        id: row.meta.id,
        chapter_number: row.meta.chapter_number,
        version: row.meta.version,
        status: row.meta.status,
        content: row.content,
    };
    let prepared = prepare(
        f,
        &source,
        ReviewRevisionOperation::ReviewChapter,
        None,
        Some(cycle),
    );
    let items: Vec<Value> = context.findings.iter().map(|finding| json!({
        "findingId":finding.finding_id,"targetId":finding.target_id,"resolved":resolved,
        "evidenceQuote": if finding.must_show == Some(true) { "合成信号灯亮起" } else { "甲与乙完成合成修订" },
        "reason":"合成复核仍需作者核实"
    })).collect();
    let (run, artifact) = generate(
        f,
        &prepared,
        &json!({"summary":"合成混合复核","items":items}).to_string(),
    );
    f.service
        .commit_review(
            &f.conn,
            &ReviewGenerationCommitRequest {
                context_id: prepared.context_id,
                handle: ReviewRevisionGeneration::handle_of(&run),
                artifact: ReviewArtifactRef {
                    artifact_id: artifact.artifact_id,
                    revision: artifact.revision,
                    text_hash: artifact.text_hash,
                },
            },
            |_| Ok(()),
        )
        .unwrap()
}

#[test]
fn mixed_review_preserves_author_waiver_and_unanchored_must_show_after_recheck() {
    let (f, cycle) = mixed_merged_fixture();
    let receipt = recheck_mixed(&f, &cycle, false);
    assert_eq!(receipt.review_cycle.unwrap().recheck_count, 1);
    assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    let statuses: Vec<(String, String)> = f
        .conn
        .prepare("SELECT category,status FROM review_findings ORDER BY category,status")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(statuses.contains(&("文风".into(), "author-waived".into())));
    assert!(statuses.contains(&("本章目标".into(), "unverified".into())));
    assert!(statuses.contains(&("本章目标".into(), "unresolved".into())));
    assert_eq!(f.count("generation_attempts"), 3);
}

#[test]
fn mixed_review_model_resolution_cannot_promote_must_show_or_author_waiver() {
    let (f, cycle) = mixed_merged_fixture();
    recheck_mixed(&f, &cycle, true);
    assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    let count: i64 = f
        .conn
        .query_row(
            "SELECT COUNT(*) FROM review_findings WHERE status='resolved'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn mixed_review_rejects_waiver_index_not_bound_to_author_choice() {
    let (f, _) = mixed_merged_fixture();
    let (id, original): (String,i64) = f.conn.query_row(
        "SELECT finding_id,confirmation_item_index FROM review_findings WHERE status='author-waived'",
        [], |r| Ok((r.get(0)?,r.get(1)?)),
    ).unwrap();
    for index in [999, original + 1] {
        f.conn
            .execute(
                "UPDATE review_findings SET confirmation_item_index=?1 WHERE finding_id=?2",
                rusqlite::params![index, id],
            )
            .unwrap();
        assert!(
            !review_cycle::verify_m03_review_cycle(&f.conn),
            "作者豁免索引必须绑定原确认项"
        );
        f.conn
            .execute(
                "UPDATE review_findings SET confirmation_item_index=?1 WHERE finding_id=?2",
                rusqlite::params![original, id],
            )
            .unwrap();
        assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    }
}

#[test]
fn mixed_review_rejects_forged_waiver_on_an_apply_choice() {
    let (f, cycle) = mixed_merged_fixture();
    let (fid, status): (String, String) = f
        .conn
        .query_row(
            "SELECT finding_id,status FROM review_findings WHERE target_id IS NOT NULL",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let (hash, index): (String,i64) = f.conn.query_row(
        "SELECT evidence_hash,confirmation_item_index FROM review_findings WHERE status='author-waived'", [],
        |r| Ok((r.get(0)?,r.get(1)?)),
    ).unwrap();
    f.conn.execute("UPDATE review_findings SET status='author-waived',evidence_hash=?1,confirmation_item_index=?2 WHERE finding_id=?3",
        rusqlite::params![hash,index,fid]).unwrap();
    assert!(!review_cycle::verify_m03_review_cycle(&f.conn));
    f.conn.execute("UPDATE review_findings SET status=?1,evidence_hash=NULL,confirmation_item_index=NULL WHERE finding_id=?2",
        rusqlite::params![status,fid]).unwrap();
    assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    assert_eq!(
        review_cycle::plan_recheck(&cycle, &f.conn)
            .unwrap()
            .context
            .unwrap()
            .findings
            .len(),
        2
    );
}

#[test]
fn mixed_review_accepts_historical_v1_resolution_without_authorizing_new_execution() {
    let (f, cycle) = mixed_merged_fixture();
    recheck_mixed(&f, &cycle, true);
    let (attempt, run, raw_binding, raw_usage, artifact_text): (
        String,
        String,
        String,
        String,
        String,
    ) = f
        .conn
        .query_row(
            "SELECT a.attempt_id,a.run_id,r.binding_json,a.usage_receipt_json,
                json_extract(v.artifact_json,'$.text')
         FROM generation_attempts a JOIN generation_runs r ON r.run_id=a.run_id
         JOIN generation_artifacts v ON v.attempt_id=a.attempt_id
         WHERE json_extract(a.usage_receipt_json,'$.reviewCycleRecheck.cycleId')=?1",
            [&cycle],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    let mut binding: Value = serde_json::from_str(&raw_binding).unwrap();
    let mut usage: Value = serde_json::from_str(&raw_usage).unwrap();
    let mut context: ReviewRevisionContext =
        serde_json::from_value(binding["sourceManifest"]["reviewRevisionContext"].clone()).unwrap();
    context.recheck.as_mut().unwrap().version = 1;
    let author_text = serde_json::to_string(&context).unwrap();
    let ctx_hash = text_hash(&author_text);
    binding["sourceManifest"]["reviewRevisionContext"] = serde_json::to_value(&context).unwrap();
    binding["sourceManifest"]["reviewRevisionContextHash"] = json!(ctx_hash);
    binding["sourceManifest"]["authorInputs"][0]["text"] = json!(author_text);
    usage["reviewRevisionEffect"]["contextHash"] = json!(ctx_hash);
    usage["reviewRevisionEffect"]
        .as_object_mut()
        .unwrap()
        .remove("reportVersion");
    let policy = RecheckPolicy {
        context: recheck_policy_context(context.recheck.as_ref().unwrap()),
        version: 1,
    };
    let report = build_review_cycle_recheck_report(
        &artifact_text,
        &context.source.content,
        &policy,
        &context.ui_locale,
        1,
    );
    let body = serde_json::to_string_pretty(&ReviewReportBody {
        summary: report.summary,
        items: report.items.clone(),
    })
    .unwrap();
    let mut mappings = Vec::new();
    for decision in &report.decisions {
        if decision.status == "unknown" {
            continue;
        }
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
        let finding = policy
            .context
            .findings
            .iter()
            .find(|f| f.finding_id == decision.finding_id)
            .unwrap();
        if finding.must_show != Some(true) {
            f.conn
                .execute(
                    "UPDATE review_findings SET status=?1,evidence_hash=?2 WHERE finding_id=?3",
                    rusqlite::params![decision.status, evidence, decision.finding_id],
                )
                .unwrap();
        }
    }
    usage["reviewCycleRecheck"]["version"] = json!(1);
    usage["reviewCycleRecheck"]["findings"] = json!(mappings);
    usage["reviewRevisionEffect"]["contentHash"] = json!(text_hash(&body));
    let review_id = usage["reviewRevisionEffect"]["id"].as_i64().unwrap();
    f.conn
        .execute(
            "UPDATE contents SET body=?1 WHERE id=(SELECT content_id FROM reviews WHERE id=?2)",
            rusqlite::params![body, review_id],
        )
        .unwrap();
    f.conn
        .execute(
            "UPDATE generation_runs SET binding_json=?1 WHERE run_id=?2",
            rusqlite::params![binding.to_string(), run],
        )
        .unwrap();
    f.conn
        .execute(
            "UPDATE generation_attempts SET usage_receipt_json=?1 WHERE attempt_id=?2",
            rusqlite::params![usage.to_string(), attempt],
        )
        .unwrap();
    assert!(
        review_cycle::verify_m03_review_cycle(&f.conn),
        "真实历史v1允许模型resolved证据，不能套用新执行策略抹除历史"
    );
    assert_eq!(f.count("generation_attempts"), 3);
    let recovery = f
        .service
        .read_recovery(
            &f.conn,
            &ReviewRevisionGeneration::handle_of(&f.service.runs.get(&f.conn, &run).unwrap()),
            |_| false,
            |_| None,
        )
        .unwrap();
    assert!(recovery.saved.is_some());
    assert!(!recovery.can_resume);
    assert!(recovery.context_id.is_none());
}

#[test]
fn mixed_review_rejects_removing_a_persisted_author_waiver() {
    let (f, _) = mixed_merged_fixture();
    let (id,hash,index): (String,String,i64) = f.conn.query_row(
        "SELECT finding_id,evidence_hash,confirmation_item_index FROM review_findings WHERE status='author-waived'",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).unwrap();
    f.conn.execute("UPDATE review_findings SET status='unverified',evidence_hash=NULL,confirmation_item_index=NULL WHERE finding_id=?1",
        [&id]).unwrap();
    assert!(
        !review_cycle::verify_m03_review_cycle(&f.conn),
        "不能清除持久豁免而绕过作者确认绑定"
    );
    f.conn.execute("UPDATE review_findings SET status='author-waived',evidence_hash=?1,confirmation_item_index=?2 WHERE finding_id=?3",
        rusqlite::params![hash,index,id]).unwrap();
    assert!(review_cycle::verify_m03_review_cycle(&f.conn));
}

#[test]
fn mixed_review_rejects_downgrading_or_unknown_persisted_recheck_policy() {
    let (f, cycle) = mixed_merged_fixture();
    recheck_mixed(&f, &cycle, false);
    let (id, raw): (String, String) = f
        .conn
        .query_row(
            "SELECT attempt_id,usage_receipt_json FROM generation_attempts
         WHERE json_extract(usage_receipt_json,'$.reviewCycleRecheck.cycleId')=?1",
            [&cycle],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    for version in [0, 1, 3] {
        let mut usage: Value = serde_json::from_str(&raw).unwrap();
        usage["reviewCycleRecheck"]["version"] = json!(version);
        f.conn
            .execute(
                "UPDATE generation_attempts SET usage_receipt_json=?1 WHERE attempt_id=?2",
                rusqlite::params![usage.to_string(), id],
            )
            .unwrap();
        assert!(
            !review_cycle::verify_m03_review_cycle(&f.conn),
            "当前v2上下文不能仅靠降低或未知回执版本切换策略"
        );
        f.conn
            .execute(
                "UPDATE generation_attempts SET usage_receipt_json=?1 WHERE attempt_id=?2",
                rusqlite::params![raw, id],
            )
            .unwrap();
        assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    }
}
