//! 批次草稿待审的真实 owner 准入回归，不调用模型、不开放自动定稿。

use super::tests::source_owner_fixture;
use super::*;

fn batch_request(nonce: &str) -> Value {
    json!({
        "uiActionNonce":nonce,"modelId":"fixture","mode":"draft_review",
        "range":{"startChapter":1,"endChapter":2},"targetUnits":10,
        "promptKeys":["edit_selected_text"],"skillStages":[],
        "authorInputs":[{"id":"draft:author-config","text":"{}"},
            {"id":"draft:target-units","text":"10"}]
    })
}

fn child(batch: &Value, chapter: i64) -> Value {
    json!({"operation":"chapter-draft","uiActionNonce":format!("chapter-{chapter}"),
        "modelId":"fixture","chapterNumber":chapter,
        "parentRootActionId":batch["rootHandle"]["rootActionId"],"batchId":batch["batchId"],
        "selectedDraftIds":[],"selectedFinalizedDraftIds":[],"selectedBlueprintChapterNumbers":[],
        "promptKeys":["edit_selected_text"],"skillStages":[],"output":"visible-text",
        "authorInputs":batch["authorInputs"]})
}

fn count_runs(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM generation_runs", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn draft_review_batch_root_is_admitted_and_replayed_without_duplicate_roots_test() {
    let (_temp, conn, owner, _) = source_owner_fixture();
    let request = batch_request("batch-root");
    let batch = owner.begin_batch(&conn, &request).unwrap();
    assert_eq!(batch["mode"], "draft_review");
    assert_eq!(batch["nextChapterNumber"], 1);
    assert_eq!(batch["completedChapters"], json!([]));
    let leases = owner.lease_store.lock().unwrap().live_lease_count();
    let count = count_runs(&conn);
    assert_eq!(owner.begin_batch(&conn, &request).unwrap(), batch);
    assert_eq!(count_runs(&conn), count);
    assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), leases);
    let mut auto = request;
    auto["mode"] = json!("auto_finalize");
    auto["uiActionNonce"] = json!("must-stay-disabled");
    assert!(owner.begin_batch(&conn, &auto).is_err());
    assert_eq!(count_runs(&conn), count);
    assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), leases);
}

#[test]
fn draft_review_batch_child_checks_parent_model_order_and_author_inputs_before_effects_test() {
    let (_temp, conn, owner, ordinary) = source_owner_fixture();
    conn.execute(
        "INSERT INTO blueprints (chapter_number,title) VALUES (1,'合成章'),(2,'合成次章')",
        [],
    )
    .unwrap();
    let batch = owner
        .begin_batch(&conn, &batch_request("batch-child"))
        .unwrap();
    let selection = child(&batch, 1);
    let count = count_runs(&conn);
    let leases = owner.lease_store.lock().unwrap().live_lease_count();
    for (key, value) in [
        ("parentRootActionId", json!(ordinary.root_action_id)),
        ("parentRootActionId", Value::Null),
        ("operation", json!("brainstorm")),
        ("chapterNumber", json!(2)),
        ("modelId", json!("different-model")),
        ("selectedDraftIds", json!([999])),
        ("authorInputs", json!([])),
        ("batchIntent", batch_request("bad-intent")),
        ("continueDirectoryOperationId", json!("directory")),
        ("agentWorkflowRegistrationId", json!("registration")),
    ] {
        let mut bad = selection.clone();
        bad[key] = value;
        assert!(owner.begin(&conn, &bad, false, false).is_err(), "{key}");
        assert_eq!(count_runs(&conn), count, "{key}");
        assert_eq!(
            owner.lease_store.lock().unwrap().live_lease_count(),
            leases,
            "{key}"
        );
    }
    let run = owner.begin(&conn, &selection, false, false).unwrap();
    assert_eq!(
        run["handle"]["rootActionId"],
        batch["rootHandle"]["rootActionId"]
    );
    assert_eq!(count_runs(&conn), count + 1);
    let root = owner
        .repository
        .get(&conn, batch["batchId"].as_str().unwrap())
        .unwrap();
    let actual = owner
        .repository
        .get(&conn, run["handle"]["runId"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        actual.binding.source_manifest["policy"],
        root.binding.source_manifest["policy"]
    );
    assert_eq!(actual.binding.source_manifest["batchId"], batch["batchId"]);
}

fn commit_child(
    conn: &Connection,
    owner: &MainGenerationOwner,
    batch: &Value,
    view: &Value,
    chapter: i64,
) -> Value {
    let run_id = view["handle"]["runId"].as_str().unwrap();
    let receipt = owner
        .repository
        .reserve(
            conn,
            run_id,
            &format!("batch-attempt-{chapter}"),
            &"a".repeat(64),
            100,
            10,
            None,
            None,
            None,
            None,
            false,
        )
        .unwrap();
    owner
        .repository
        .mark_dispatched(conn, &receipt.attempt.attempt_id)
        .unwrap();
    let prior = receipt.artifact.unwrap();
    let artifact = owner
        .repository
        .snapshot(
            conn,
            &receipt.attempt.attempt_id,
            prior.revision,
            "合成正文甲乙丙丁戊己",
        )
        .unwrap();
    owner
        .repository
        .settle(
            conn,
            &receipt.attempt.attempt_id,
            None,
            Some("stop".to_string()),
            None,
            None,
        )
        .unwrap();
    let composition = owner
        .repository
        .compose_visible(
            conn,
            run_id,
            &[artifact.artifact_id],
            None,
            "draft-visible-v2",
        )
        .unwrap();
    owner.commit_draft(conn, &json!({"handle":view["handle"],"source":"write",
        "chapterNumber":chapter,"batchId":batch["batchId"],"expectedCompositionHash":composition.text_hash})).unwrap()
}

#[test]
fn draft_review_batch_progress_requires_exact_predecessor_and_preserves_one_budget_test() {
    let (_temp, conn, owner, _) = source_owner_fixture();
    conn.execute(
        "INSERT INTO blueprints (chapter_number,title) VALUES (1,'合成章'),(2,'合成次章')",
        [],
    )
    .unwrap();
    let batch = owner
        .begin_batch(&conn, &batch_request("batch-progress"))
        .unwrap();
    let first = owner.begin(&conn, &child(&batch, 1), false, false).unwrap();
    let saved = commit_child(&conn, &owner, &batch, &first, 1);
    let progress = GenerationDraftEffects
        .read_batch(
            &conn,
            &owner.repository,
            batch["batchId"].as_str().unwrap(),
            &owner.session.project_id,
        )
        .unwrap();
    assert_eq!(progress["nextChapterNumber"], 2);
    assert_eq!(progress["completedChapters"].as_array().unwrap().len(), 1);
    let count = count_runs(&conn);
    let leases = owner.lease_store.lock().unwrap().live_lease_count();
    assert!(owner.begin(&conn, &child(&batch, 1), false, false).is_err());
    assert!(owner.begin(&conn, &child(&batch, 2), false, false).is_err());
    let mut second = child(&batch, 2);
    second["selectedDraftIds"] = json!([saved["id"]]);
    let mut extra = second.clone();
    extra["selectedDraftIds"] = json!([saved["id"], 999]);
    assert!(owner.begin(&conn, &extra, false, false).is_err());
    let mut changed = second.clone();
    changed["authorInputs"][0]["text"] = json!("changed");
    assert!(owner.begin(&conn, &changed, false, false).is_err());
    assert_eq!(count_runs(&conn), count);
    assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), leases);
    let next = owner.begin(&conn, &second, false, false).unwrap();
    let root_id = batch["rootHandle"]["rootActionId"].as_str().unwrap();
    assert_eq!(next["handle"]["rootActionId"], root_id);
    commit_child(&conn, &owner, &batch, &next, 2);
    let complete = GenerationDraftEffects
        .read_batch(
            &conn,
            &owner.repository,
            batch["batchId"].as_str().unwrap(),
            &owner.session.project_id,
        )
        .unwrap();
    assert!(complete["nextChapterNumber"].is_null());
    assert_eq!(complete["completedChapters"].as_array().unwrap().len(), 2);
    let attempts = owner.repository.budget(&conn, root_id).unwrap();
    assert_eq!(attempts.attempts.len(), 2);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM finalization_outbox", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(owner.begin(&conn, &second, false, false).is_err());
    assert_eq!(count_runs(&conn), count + 1);
}

#[test]
fn draft_review_batch_foreign_project_and_transaction_failure_do_not_leak_effects_test() {
    let (_temp, conn, owner, _) = source_owner_fixture();
    conn.execute(
        "INSERT INTO blueprints (chapter_number,title) VALUES (1,'合成章')",
        [],
    )
    .unwrap();
    let batch = owner
        .begin_batch(&conn, &batch_request("batch-rollback"))
        .unwrap();
    let selection = child(&batch, 1);
    let foreign = MainGenerationOwner::new(crate::security::ProjectSessionContext {
        project_id: "foreign-project".to_string(),
        lease_id: "foreign-lease".to_string(),
        project_path: owner.session.project_path.clone(),
    });
    let count = count_runs(&conn);
    assert!(foreign.begin(&conn, &selection, false, false).is_err());
    assert_eq!(count_runs(&conn), count);
    let root_id = batch["rootHandle"]["rootActionId"].as_str().unwrap();
    owner.repository.pause(&conn, root_id, false).unwrap();
    let before = owner.repository.budget(&conn, root_id).unwrap();
    let leases = owner.lease_store.lock().unwrap().live_lease_count();
    conn.execute_batch("CREATE TEMP TRIGGER reject_batch_child BEFORE INSERT ON generation_runs BEGIN SELECT RAISE(ABORT,'synthetic-child-failure'); END;").unwrap();
    assert!(owner.begin(&conn, &selection, false, false).is_err());
    assert_eq!(count_runs(&conn), count);
    assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), leases);
    let after = owner.repository.budget(&conn, root_id).unwrap();
    assert_eq!(before.root.status, after.root.status);
    assert_eq!(before.policy, after.policy);
    assert_eq!(after.attempts.len(), 0);
}

#[test]
fn draft_review_batch_ignores_unrelated_frozen_receipts_but_rejects_current_corruption_test() {
    use crate::services::{
        portable_project_guard, portable_project_history, portable_runtime_freeze,
    };
    let (_temp, conn, owner, _) = source_owner_fixture();
    conn.execute_batch("INSERT INTO generation_roots (root_action_id,idempotency_key,action_json,budget_json) VALUES ('redacted-root','redacted-key','','');
        INSERT INTO generation_runs (run_id,root_action_id,binding_json,status,created_at_ms,open_key) VALUES ('redacted-run','redacted-root','','completed',0,'redacted-open');
        INSERT INTO generation_attempts VALUES ('redacted-attempt','redacted-reservation','redacted-run','redacted-root','','','redacted-nonce');
        INSERT INTO generation_attempts VALUES ('redacted-null-attempt','redacted-null-reservation','redacted-run','redacted-root','',NULL,'redacted-null-nonce');").unwrap();
    let document = portable_project_history::freeze_document(
        &conn,
        "123e4567-e89b-12d3-a456-426614174000",
        "batch-restored",
    )
    .unwrap();
    let guard = portable_runtime_freeze::parse_portable_runtime_freeze(
        &serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
    portable_project_guard::install_guard_for_test(&conn, guard).unwrap();
    let batch = owner
        .begin_batch(&conn, &batch_request("fresh-restored-batch"))
        .unwrap();
    assert_eq!(batch["nextChapterNumber"], 1);
    // 当前非冻结坏账本不能因有历史守卫就被悄悄吞掉。
    conn.execute_batch("INSERT INTO generation_roots (root_action_id,idempotency_key,action_json,budget_json) VALUES ('current-bad-root','current-bad-key','{}','{}');
        INSERT INTO generation_runs (run_id,root_action_id,binding_json,status,created_at_ms,open_key) VALUES ('current-bad-run','current-bad-root','{}','completed',0,'current-bad-open');
        INSERT INTO generation_attempts VALUES ('current-bad-attempt','current-bad-reservation','current-bad-run','current-bad-root','{}','invalid','current-bad-nonce');").unwrap();
    assert!(GenerationDraftEffects
        .read_batch(
            &conn,
            &owner.repository,
            batch["batchId"].as_str().unwrap(),
            &owner.session.project_id
        )
        .is_err());
    conn.execute(
        "UPDATE generation_runs SET binding_json='invalid' WHERE run_id='current-bad-run'",
        [],
    )
    .unwrap();
    let mut fresh_owner = MainGenerationOwner::new(owner.session.clone());
    fresh_owner.global_data_root = owner.global_data_root.clone();
    let count = count_runs(&conn);
    assert!(fresh_owner
        .begin_batch(&conn, &batch_request("must-reject-current-binding"))
        .is_err());
    assert_eq!(count_runs(&conn), count);
    let old: String = conn.query_row("SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id='redacted-attempt'", [], |row| row.get(0)).unwrap();
    assert!(old.is_empty(), "冻结历史不得在启动恢复中被改写");
}
