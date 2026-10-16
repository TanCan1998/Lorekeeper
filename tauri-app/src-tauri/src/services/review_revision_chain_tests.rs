//! 全部材料均为合成数据，沿真实仓储完成初审、人工确认、修订、合并与复核。
use super::*;

fn generate(
    f: &Fixture,
    prepared: &PreparedReviewRevisionContext,
    text: &str,
) -> (DurableGenerationRun, VisibleArtifact) {
    generate_with_finish(f, prepared, text, "stop")
}

fn generate_with_finish(
    f: &Fixture,
    prepared: &PreparedReviewRevisionContext,
    text: &str,
    finish_reason: &str,
) -> (DurableGenerationRun, VisibleArtifact) {
    let request = OpenGenerationRunRequest {
        project_id: f.run.binding.project_id.clone(),
        epoch: f.run.binding.epoch.clone(),
        fingerprint: f.run.binding.fingerprint.clone(),
        context_snapshot_id: random_uuid_v4(),
        source_manifest: serde_json::from_value(json!({
            "operation": prepared.context.operation.as_str(),
            "outputContract": "visible-text",
            "reviewRevisionContext": prepared.context,
            "reviewRevisionContextHash": ReviewRevisionGeneration::context_hash(&prepared.context),
            "authorInputs": [{"id":"review-revision-context","text":serde_json::to_string(&prepared.context).unwrap()}],
            "modelReceipt": {"modelId":"synthetic-model"}
        })).unwrap(),
        source_refs: vec![],
        operation: prepared.context.operation.as_str().into(),
        ui_action_nonce: random_uuid_v4(),
        frozen_input_hash: "a".repeat(64),
        budget: f.service.runs.budget(&f.conn, &f.run.root_action_id).unwrap().policy,
        parent_root_action_id: prepared.parent_root_action_id.clone(),
    };
    let run = f.service.runs.open(&f.conn, &request).unwrap();
    let artifact = append_artifact(f, &run, text, finish_reason);
    (run, artifact)
}

fn append_artifact(
    f: &Fixture,
    run: &DurableGenerationRun,
    text: &str,
    finish_reason: &str,
) -> VisibleArtifact {
    let attempt = f
        .service
        .runs
        .reserve(
            &f.conn,
            &run.run_id,
            &random_uuid_v4(),
            &"b".repeat(64),
            100,
            100,
            None,
            Some(run.binding.source_manifest["operation"].as_str().unwrap()),
            None,
            None,
            false,
        )
        .unwrap()
        .attempt
        .attempt_id;
    f.service.runs.mark_dispatched(&f.conn, &attempt).unwrap();
    let artifact = f.service.runs.snapshot(&f.conn, &attempt, 0, text).unwrap();
    f.service
        .runs
        .settle(
            &f.conn,
            &attempt,
            None,
            Some(finish_reason.into()),
            None,
            None,
        )
        .unwrap();
    artifact
}

fn prepare(
    f: &Fixture,
    source: &ExpectedDraftSource,
    operation: ReviewRevisionOperation,
    confirmation: Option<(i64, String)>,
    recheck: Option<&str>,
) -> PreparedReviewRevisionContext {
    f.service
        .prepare(
            &f.conn,
            &PrepareReviewRevisionRequest {
                operation,
                draft_id: source.id,
                expected_draft: ExpectedDraftRef {
                    chapter_number: source.chapter_number,
                    version: source.version,
                    status: source.status.clone(),
                    content_hash: text_hash(&source.content),
                },
                review_source_id: confirmation.as_ref().map(|c| c.0),
                confirmed_review_content: confirmation.map(|c| c.1),
                review_cycle_id: recheck.map(str::to_string),
                expected_merged_hash: recheck.map(|_| text_hash(&source.content)),
                author_inputs: vec![],
                ui_locale: "zh-CN".into(),
            },
            || Ok(()),
        )
        .unwrap()
}

fn merged_fixture() -> (Fixture, String) {
    merged_fixture_with_failure(false)
}

fn merged_fixture_with_failure(inject_failure: bool) -> (Fixture, String) {
    merged_fixture_with_chunks(
        inject_failure,
        &["🔑开场。甲与乙共同完成了合成任务，问题已经调整。e\u{301}尾声。"],
    )
}

fn merged_fixture_with_chunks(inject_failure: bool, chunks: &[&str]) -> (Fixture, String) {
    let f = Fixture::with_report(
        ReviewRevisionOperation::ReviewChapter,
        Some(("展示合成证据", "甲\r\n乙")),
        Some(
            json!({"goalStatus":"unmet","items":[{"category":"剧情连贯性","severity":"pass",
            "description":"合成检查"}]}),
        ),
    );
    merge_reviewed_fixture(f, inject_failure, chunks)
}

fn merge_reviewed_fixture(f: Fixture, inject_failure: bool, chunks: &[&str]) -> (Fixture, String) {
    let reviewed = f.review().unwrap();
    let cycle: String = f
        .conn
        .query_row("SELECT cycle_id FROM review_cycles", [], |r| r.get(0))
        .unwrap();
    let finding: String = f
        .conn
        .query_row(
            "SELECT finding_id FROM review_findings WHERE target_id='ch1:keyEvents:1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let confirmation = json!({
        "kind":"human-confirmed-review","schemaVersion":2,
        "cycleId":cycle,"sourceReviewId":reviewed.id,"sourceDraft":reviewed.source,
        "summary":"作者确认合成问题","authorGuidance":"修正合成问题",
        "items":[{"category":"本章目标","severity":"error","description":"合成目标需要修订",
            "quote":"甲\r\n乙","origin":"ai","decision":"apply","findingId":finding}]
    });
    let confirmation: crate::services::review_revision_context::HumanConfirmedReviewSnapshot =
        serde_json::from_value(confirmation).unwrap();
    let confirmation = serde_json::to_string_pretty(&confirmation).unwrap();
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
    assert!(!chunks.is_empty());
    let (run, first) = generate_with_finish(
        &f,
        &prepared,
        chunks[0],
        if chunks.len() > 1 { "length" } else { "stop" },
    );
    let mut artifact_ids = vec![first.artifact_id];
    for chunk in &chunks[1..] {
        artifact_ids.push(append_artifact(&f, &run, chunk, "stop").artifact_id);
    }
    let composition = f
        .service
        .runs
        .compose_visible(
            &f.conn,
            &run.run_id,
            &artifact_ids,
            None,
            "visible-append-v1",
        )
        .unwrap();
    let revised = composition.text.as_str();
    let receipt = f
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
    assert!(
        review_cycle::verify_m03_review_cycle(&f.conn),
        "生成修稿链必须合法"
    );
    if inject_failure {
        f.conn
            .execute_batch(
                "CREATE TRIGGER reject_merge_snapshot BEFORE INSERT ON review_cycle_merges
            BEGIN SELECT RAISE(ABORT,'synthetic-merge-failure'); END;",
            )
            .unwrap();
        let error = revision_repo::merge_into_draft(
            &f.conn,
            &revision_repo::MergeRevisionRequest {
                revision_id: receipt.id,
                target_draft_id: reviewed.source.id,
                expected_draft_content: reviewed.source.content.clone(),
                merged_content: revised.into(),
                word_count: count_draft_units(revised),
            },
        )
        .unwrap_err();
        assert!(error.contains("synthetic-merge-failure"));
        let draft = crate::repositories::draft_repository::get_full(&f.conn, reviewed.source.id)
            .unwrap()
            .unwrap();
        assert_eq!(draft.content, reviewed.source.content);
        assert_eq!(draft.meta.status, reviewed.source.status);
        assert_eq!(
            revision_repo::get_full(&f.conn, receipt.id)
                .unwrap()
                .unwrap()
                .meta
                .status,
            "pending"
        );
        assert_eq!(f.count("review_cycle_merges"), 0);
        assert!(review_cycle::verify_m03_review_cycle(&f.conn));
        f.conn
            .execute_batch("DROP TRIGGER reject_merge_snapshot;")
            .unwrap();
    }

    revision_repo::merge_into_draft(
        &f.conn,
        &revision_repo::MergeRevisionRequest {
            revision_id: receipt.id,
            target_draft_id: reviewed.source.id,
            expected_draft_content: reviewed.source.content,
            merged_content: revised.into(),
            word_count: count_draft_units(revised),
        },
    )
    .unwrap();
    assert!(
        review_cycle::verify_m03_review_cycle(&f.conn),
        "合并链必须合法"
    );
    (f, cycle)
}

fn rechecked_fixture(model_resolved: Option<bool>) -> (Fixture, String, MainGenerationRunHandle) {
    let (f, cycle) = merged_fixture();
    let plan = review_cycle::plan_recheck(&cycle, &f.conn).unwrap();
    let context = plan.context.unwrap();
    assert_eq!(context.findings.len(), 1);
    let source =
        crate::repositories::draft_repository::get_full(&f.conn, f.prepared.context.source.id)
            .unwrap()
            .unwrap();
    let source = ExpectedDraftSource {
        id: source.meta.id,
        chapter_number: source.meta.chapter_number,
        version: source.meta.version,
        status: source.meta.status,
        content: source.content,
    };
    let prepared = prepare(
        &f,
        &source,
        ReviewRevisionOperation::ReviewChapter,
        None,
        Some(&cycle),
    );
    let finding = &context.findings[0];
    let text = json!({"summary":"合成复核","items":[{
        "findingId":finding.finding_id,"targetId":finding.target_id,
        "resolved":model_resolved.unwrap_or(false),
        "evidenceQuote":if model_resolved.is_some() { "甲与乙共同完成了合成任务" } else { "不存在的合成证据" },
        "reason":"仍需作者核实"
    }]}).to_string();
    let (run, artifact) = generate(&f, &prepared, &text);
    let receipt = f
        .service
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
        .unwrap();
    assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    assert_eq!(receipt.review_cycle.unwrap().recheck_count, 1);
    assert_eq!(f.count("generation_attempts"), 3);
    assert_eq!(f.count("reviews"), 3);
    let original = review_repo::get_full(&f.conn, 1).unwrap().unwrap();
    assert_eq!(
        original.source_draft.unwrap().content,
        f.prepared.context.source.content
    );
    (f, cycle, ReviewRevisionGeneration::handle_of(&run))
}

#[test]
fn real_sqlite_chain_commits_recheck_and_preserves_author_material() {
    rechecked_fixture(Some(false));
}

#[test]
fn real_sqlite_chain_keeps_unverifiable_and_model_resolved_findings_unknown() {
    for decision in [None, Some(true)] {
        let (f, cycle, _) = rechecked_fixture(decision);
        let (status, evidence): (String, Option<String>) = f
            .conn
            .query_row(
                "SELECT status,evidence_hash FROM review_findings WHERE cycle_id=?1",
                [&cycle],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "unknown");
        assert!(evidence.is_none());
    }
}

#[test]
fn real_sqlite_chain_rejects_missing_duplicate_and_forged_recheck_mappings() {
    let (f, cycle, handle) = rechecked_fixture(Some(false));
    let (attempt, raw): (String, String) = f
        .conn
        .query_row(
            "SELECT attempt_id,usage_receipt_json FROM generation_attempts
         WHERE json_extract(usage_receipt_json,'$.reviewCycleRecheck.cycleId')=?1",
            [&cycle],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let original: Value = serde_json::from_str(&raw).unwrap();
    let mapping = original["reviewCycleRecheck"]["findings"][0].clone();
    let mut changed = mapping.clone();
    changed["evidenceHash"] = json!("0".repeat(64));
    for mappings in [
        json!([]),
        json!([mapping.clone(), mapping]),
        json!([changed]),
    ] {
        let mut value = original.clone();
        value["reviewCycleRecheck"]["findings"] = mappings;
        f.conn
            .execute(
                "UPDATE generation_attempts SET usage_receipt_json=?1 WHERE attempt_id=?2",
                rusqlite::params![value.to_string(), attempt],
            )
            .unwrap();
        assert!(!review_cycle::verify_m03_review_cycle(&f.conn));
        f.conn
            .execute(
                "UPDATE generation_attempts SET usage_receipt_json=?1 WHERE attempt_id=?2",
                rusqlite::params![raw, attempt],
            )
            .unwrap();
        assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    }
    let (fid, evidence): (String, Option<String>) = f
        .conn
        .query_row(
            "SELECT finding_id,evidence_hash FROM review_findings WHERE cycle_id=?1",
            [&cycle],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    for (status, hash) in [
        ("unknown", evidence.clone()),
        ("resolved", evidence.clone()),
        ("unresolved", Some("0".repeat(64))),
        ("unresolved", None),
    ] {
        f.conn
            .execute(
                "UPDATE review_findings SET status=?1,evidence_hash=?2 WHERE finding_id=?3",
                rusqlite::params![status, hash, fid],
            )
            .unwrap();
        assert!(!review_cycle::verify_m03_review_cycle(&f.conn));
        f.conn.execute("UPDATE review_findings SET status='unresolved',evidence_hash=?1 WHERE finding_id=?2",
            rusqlite::params![evidence, fid]).unwrap();
        assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    }
    let recovery = f
        .service
        .read_recovery(&f.conn, &handle, |_| false, |_| None)
        .unwrap();
    assert!(recovery.saved.is_some());
    assert!(!recovery.can_resume);
    assert!(recovery.context_id.is_none());
    assert_eq!(f.count("generation_attempts"), 3);
}

#[test]
fn real_sqlite_chain_rolls_back_author_text_revision_and_cycle_on_snapshot_failure() {
    merged_fixture_with_failure(true);
}

#[test]
fn real_sqlite_chain_rejects_self_consistently_rehashed_revision_source_and_confirmation() {
    let (f, _) = merged_fixture();
    let (run_id, attempt_id, binding_raw, usage_raw): (String, String, String, String) = f
        .conn
        .query_row(
            "SELECT r.run_id,a.attempt_id,r.binding_json,a.usage_receipt_json
         FROM generation_runs r JOIN generation_attempts a ON a.run_id=r.run_id
         WHERE json_extract(a.usage_receipt_json,'$.reviewRevisionEffect.kind')='revision'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    for (pointer, value) in [
        ("/source/chapterNumber", json!(2)),
        ("/source/version", json!(2)),
        ("/source/status", json!("reviewed")),
        ("/confirmation/snapshot/sourceReviewId", json!(999)),
        (
            "/confirmation/snapshot/sourceDraft/content",
            json!("伪造的合成来源"),
        ),
    ] {
        let mut binding: Value = serde_json::from_str(&binding_raw).unwrap();
        let mut usage: Value = serde_json::from_str(&usage_raw).unwrap();
        let manifest = &mut binding["sourceManifest"];
        let mut context = manifest["reviewRevisionContext"].clone();
        *context.pointer_mut(pointer).unwrap() = value;
        let typed: ReviewRevisionContext = serde_json::from_value(context).unwrap();
        let author_text = serde_json::to_string(&typed).unwrap();
        let context_hash = text_hash(&author_text);
        manifest["reviewRevisionContext"] = serde_json::to_value(typed).unwrap();
        manifest["reviewRevisionContextHash"] = json!(context_hash);
        manifest["authorInputs"][0]["text"] = json!(author_text);
        usage["reviewRevisionEffect"]["contextHash"] = json!(context_hash);
        f.conn
            .execute(
                "UPDATE generation_runs SET binding_json=?1 WHERE run_id=?2",
                rusqlite::params![binding.to_string(), run_id],
            )
            .unwrap();
        f.conn
            .execute(
                "UPDATE generation_attempts SET usage_receipt_json=?1 WHERE attempt_id=?2",
                rusqlite::params![usage.to_string(), attempt_id],
            )
            .unwrap();
        assert!(!review_cycle::verify_m03_review_cycle(&f.conn), "{pointer}");
        f.conn
            .execute(
                "UPDATE generation_runs SET binding_json=?1 WHERE run_id=?2",
                rusqlite::params![binding_raw, run_id],
            )
            .unwrap();
        f.conn
            .execute(
                "UPDATE generation_attempts SET usage_receipt_json=?1 WHERE attempt_id=?2",
                rusqlite::params![usage_raw, attempt_id],
            )
            .unwrap();
        assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    }
}

#[test]
fn real_sqlite_chain_redacted_history_is_readable_but_cannot_replay() {
    use crate::services::{
        portable_project_guard, portable_project_history, portable_project_sanitize,
        portable_runtime_freeze,
    };
    let (f, cycle, handle) = rechecked_fixture(Some(false));
    let root = std::env::temp_dir().join(format!("lorekeeper-review-freeze-{}", random_uuid_v4()));
    std::fs::create_dir(&root).unwrap();
    let source = root.join("source.db");
    let target = root.join("portable.db");
    f.conn
        .execute("VACUUM INTO ?1", [source.to_str().unwrap()])
        .unwrap();
    portable_project_sanitize::sanitize_portable_database(
        &portable_project_sanitize::SanitizePortableDatabaseInput {
            source_database_path: &source,
            target_database_path: &target,
            limits: None,
        },
    )
    .unwrap();
    let conn = Connection::open(&target).unwrap();
    let document = portable_project_history::freeze_document(
        &conn,
        "123e4567-e89b-12d3-a456-426614174000",
        "synthetic-generation",
    )
    .unwrap();
    for record in document["records"].as_array().unwrap() {
        assert_eq!(record["originalReceiptVerified"], false);
        assert_eq!(record["nonReplayable"], true);
    }
    let guard = portable_runtime_freeze::parse_portable_runtime_freeze(
        &serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
    portable_project_guard::install_guard_for_test(&conn, guard).unwrap();
    let author_body =
        crate::repositories::draft_repository::get_full(&conn, f.prepared.context.source.id)
            .unwrap()
            .unwrap()
            .content;
    assert!(author_body.contains("合成任务"));
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM reviews", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 3);
    assert!(
        !review_cycle::verify_m03_review_cycle(&conn),
        "脱敏历史没有原始收据，不能声称执行效果已验证"
    );
    assert!(f
        .service
        .read_recovery(&conn, &handle, |_| false, |_| None)
        .is_err());
    let error = conn
        .execute(
            "UPDATE review_cycles SET recheck_count=0 WHERE cycle_id=?1",
            [&cycle],
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("PORTABLE_RUNTIME_FROZEN"));
    let error = f
        .service
        .runs
        .reserve(
            &conn,
            &handle.run_id,
            "forbidden-replay",
            &"b".repeat(64),
            100,
            100,
            None,
            Some("review-chapter"),
            None,
            None,
            false,
        )
        .unwrap_err();
    // 脱敏绑定可更早拒绝解析；无论在哪一层拒绝，都不得产生新的执行。
    assert!(!error.is_empty());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM generation_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        crate::repositories::draft_repository::get_full(&conn, f.prepared.context.source.id)
            .unwrap()
            .unwrap()
            .content,
        author_body
    );
    drop(conn);
    std::fs::remove_dir_all(root).unwrap();
}

#[path = "review_revision_continuation_tests.rs"]
mod continuation_tests;

#[path = "review_revision_mixed_tests.rs"]
mod mixed_tests;

#[path = "review_revision_finding_tests.rs"]
mod finding_tests;

#[path = "review_revision_effect_tests.rs"]
mod effect_tests;
