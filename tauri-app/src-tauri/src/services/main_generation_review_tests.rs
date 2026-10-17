//! 审修公共准入使用真实作者草稿、上下文捕获、模型与来源绑定。
use super::tests::source_owner_fixture;
use super::*;
use crate::services::review_revision_context::{
    ExpectedDraftRef, PrepareReviewRevisionRequest, ReviewRevisionOperation,
};
use crate::services::review_revision_generation::{ReviewRevisionGeneration, ReviewRevisionScope};

fn prepare_review(
    owner: &MainGenerationOwner,
    conn: &Connection,
    operation: ReviewRevisionOperation,
) -> (ReviewRevisionGeneration, Value) {
    let body = "隔离合成审修正文。".repeat(20);
    conn.execute("INSERT INTO contents(body) VALUES (?1)", [&body])
        .unwrap();
    let content_id = conn.last_insert_rowid();
    conn.execute("INSERT INTO drafts(chapter_number,version,source,content_id,word_count,source_dependencies) VALUES(1,1,'write',?1,0,'[]')", [content_id]).unwrap();
    let id = conn.last_insert_rowid();
    let service = ReviewRevisionGeneration::new(
        owner.repository.clone(),
        ReviewRevisionScope {
            project_id: owner.session.project_id.clone(),
            epoch: owner.session.lease_id.clone(),
        },
    );
    let prepared = service
        .prepare(
            conn,
            &PrepareReviewRevisionRequest {
                operation,
                draft_id: id,
                expected_draft: ExpectedDraftRef {
                    chapter_number: 1,
                    version: 1,
                    status: "draft".into(),
                    content_hash: text_hash(&body),
                },
                review_source_id: None,
                confirmed_review_content: None,
                review_cycle_id: None,
                expected_merged_hash: None,
                author_inputs: vec![],
                ui_locale: "zh-CN".into(),
            },
            || Ok(()),
        )
        .unwrap();
    let review = operation == ReviewRevisionOperation::ReviewChapter;
    let selection = json!({"operation":operation.as_str(),"uiActionNonce":format!("review-{}",operation.as_str()),"modelId":"fixture","chapterNumber":1,"reviewRevisionContextId":prepared.context_id,"selectedDraftIds":[id],"selectedFinalizedDraftIds":[],"selectedBlueprintChapterNumbers":[],"promptKeys":[if review{"consistency_check"}else{"refine_chapter"}],"skillStages":[if review{"review"}else{"refinement"}],"output":if review{"structured-data"}else{"visible-text"},"authorInputs":[{"id":"review-revision-context","text":serde_json::to_value(&prepared.context).unwrap().to_string()}]});
    (service, selection)
}

#[test]
fn review_owner_prepared_context_enters_public_begin_and_real_recovery_test() {
    for operation in [
        ReviewRevisionOperation::ReviewChapter,
        ReviewRevisionOperation::RefineDraft,
    ] {
        let (_temp, conn, owner, _) = source_owner_fixture();
        let (service, selection) = prepare_review(&owner, &conn, operation);
        let view = owner
            .begin(&conn, &selection, false, false)
            .expect("真实已准备的审修上下文应通过有界准入");
        let handle: MainGenerationRunHandle =
            serde_json::from_value(view["handle"].clone()).unwrap();
        let recovery = service
            .read_recovery(
                &conn,
                &handle,
                |run| owner.review_revision_sources_match(&conn, run),
                |run| owner.review_revision_candidate_snapshot(&conn, run),
            )
            .unwrap();
        assert_eq!(recovery.context.operation, operation);
        assert_eq!(recovery.source_status, "current");
        assert!(recovery.can_resume);
        assert_eq!(
            owner.begin(&conn, &selection, false, false).unwrap()["handle"],
            view["handle"]
        );
    }
}

#[test]
fn review_owner_rejects_forged_context_fields_without_writes_or_leases_test() {
    let (_temp, conn, owner, _) = source_owner_fixture();
    let (_service, selection) =
        prepare_review(&owner, &conn, ReviewRevisionOperation::ReviewChapter);
    let changes = conn.total_changes();
    for (key, value) in [
        ("reviewRevisionContextId", json!("未签发的上下文")),
        ("chapterNumber", json!(2)),
        ("selectedDraftIds", json!([])),
        ("selectedFinalizedDraftIds", json!([1])),
        ("selectedBlueprintChapterNumbers", json!([1])),
        ("promptKeys", json!(["edit_selected_text"])),
        ("skillStages", json!(["refinement"])),
        ("output", json!("visible-text")),
        ("outputOverrides", json!(null)),
        ("parentRootActionId", json!("不存在的父根")),
        ("batchId", json!("伪造批次")),
        ("authorInputs", json!([])),
        ("reviewRevisionContext", json!({})),
    ] {
        let mut bad = selection.clone();
        bad[key] = value;
        assert!(owner.begin(&conn, &bad, false, false).is_err(), "{key}");
        assert_eq!(conn.total_changes(), changes, "{key}不得写入");
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 0);
    }
    let mut context: Value =
        serde_json::from_str(selection["authorInputs"][0]["text"].as_str().unwrap()).unwrap();
    context["source"]["content"] = json!("伪造作者正文");
    let mut bad = selection.clone();
    bad["authorInputs"][0]["text"] = json!(context.to_string());
    assert!(owner.begin(&conn, &bad, false, false).is_err());
    assert_eq!(conn.total_changes(), changes);
    assert!(owner.begin(&conn, &selection, false, false).is_ok());
}

#[test]
fn review_owner_prepared_ids_do_not_cross_project_or_epoch_and_source_changes_test() {
    let (_temp, conn, owner, _) = source_owner_fixture();
    let (_service, selection) = prepare_review(&owner, &conn, ReviewRevisionOperation::RefineDraft);
    let changes = conn.total_changes();
    for project_change in [false, true] {
        let mut session = owner.session.clone();
        if project_change {
            session.project_id = "另一个项目".into();
        } else {
            session.lease_id = "新会话租约".into();
        }
        let mut next = MainGenerationOwner::new(session);
        next.global_data_root = owner.global_data_root.clone();
        assert_eq!(
            next.begin(&conn, &selection, false, false).unwrap_err(),
            mutating_error("GENERATION_REVIEW_CONTEXT_REQUIRED")
        );
        assert_eq!(conn.total_changes(), changes);
        assert_eq!(next.lease_store.lock().unwrap().live_lease_count(), 0);
    }
    let tx = conn.unchecked_transaction().unwrap();
    assert!(owner.begin(&tx, &selection, false, false).is_err());
    tx.rollback().unwrap();
    assert_eq!(conn.total_changes(), changes);
    assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 0);
    conn.execute("UPDATE contents SET body='作者更新正文'", [])
        .unwrap();
    let changed = conn.total_changes();
    assert_eq!(
        owner.begin(&conn, &selection, false, false).unwrap_err(),
        mutating_error("GENERATION_REVIEW_CONTEXT_CHANGED")
    );
    assert_eq!(conn.total_changes(), changed);
    assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 0);
}

#[test]
fn review_owner_real_commit_seals_candidate_but_keeps_saved_ack_and_invocation_replay_test() {
    use crate::services::review_revision_generation::{
        ReviewArtifactRef, ReviewGenerationCommitRequest,
    };
    let (_temp, conn, owner, _) = source_owner_fixture();
    let (service, selection) =
        prepare_review(&owner, &conn, ReviewRevisionOperation::ReviewChapter);
    let view = owner.begin(&conn, &selection, false, false).unwrap();
    let handle = &view["handle"];
    let prompt = "合成审稿提示";
    let material = json!({"version":1,"verdict":"admitted","promptHash":text_hash(prompt),"capacity":{"maxInputUnits":1000,"methodVersion":"utf8-bytes-v1","admittedUnits":0},"coverage":{"required":0,"included":0,"complete":true},"included":[],"omitted":[]});
    owner
        .bind_material_decision(&conn, handle, &material)
        .unwrap();
    let request = json!({"handle":handle,"invocationNonce":"review-first","task":{"purpose":"review-chapter","output":"structured-data","messages":[{"role":"user","content":prompt}]}});
    let prepared = owner.prepare_execution(&conn, &request, false).unwrap();
    let PreparedExecutionPlan::Dispatch(dispatch) = &prepared.plan else {
        panic!("首次准备必须分发");
    };
    let receipt = owner
        .repository
        .reserve(
            &conn,
            &dispatch.run_id,
            &dispatch.invocation_nonce,
            &dispatch.request_hash,
            dispatch.reserved_tokens,
            dispatch.requested_output_tokens,
            Some(&dispatch.usage_policy),
            dispatch.purpose.as_deref(),
            dispatch.replay_task.as_ref(),
            dispatch.budget_decision.as_ref(),
            dispatch.reconciliation_injected,
        )
        .unwrap();
    let attempt = &receipt.attempt.attempt_id;
    owner.repository.mark_dispatched(&conn, attempt).unwrap();
    let text=json!({"summary":"隔离合成审稿","items":[{"category":"剧情连贯性","severity":"warning","quote":"隔离合成审修正文。","description":"同一事件重复呈现，需去除重复。"}]}).to_string();
    let artifact = owner.repository.snapshot(&conn, attempt, 0, &text).unwrap();
    owner
        .repository
        .settle(&conn, attempt, None, Some("stop".into()), None, None)
        .unwrap();
    drop(prepared);
    let commit = ReviewGenerationCommitRequest {
        context_id: selection["reviewRevisionContextId"]
            .as_str()
            .unwrap()
            .into(),
        handle: serde_json::from_value(handle.clone()).unwrap(),
        artifact: ReviewArtifactRef {
            artifact_id: artifact.artifact_id.clone(),
            revision: artifact.revision,
            text_hash: artifact.text_hash.clone(),
        },
    };
    let saved = service
        .commit_review(&conn, &commit, |handle| {
            owner.assert_review_sources_current(&conn, handle)
        })
        .unwrap();
    let changes = conn.total_changes();
    assert_eq!(
        service
            .commit_review(&conn, &commit, |_| panic!("已保存回执不得重验或写入"))
            .unwrap(),
        saved
    );
    assert_eq!(
        owner.resume(&conn, handle).unwrap_err(),
        mutating_error("GENERATION_REVIEW_ALREADY_SAVED")
    );
    assert_eq!(
        owner
            .discard_candidate(&conn, handle, &artifact.artifact_id)
            .unwrap_err(),
        mutating_error("GENERATION_REVIEW_ALREADY_SAVED")
    );
    let mut next = request.clone();
    next["invocationNonce"] = json!("review-new");
    assert_eq!(
        owner.prepare_execution(&conn, &next, false).err().unwrap(),
        mutating_error("GENERATION_REVIEW_ALREADY_SAVED")
    );
    let replay = owner.prepare_execution(&conn, &request, false).unwrap();
    assert!(matches!(&replay.plan, PreparedExecutionPlan::Replay(_)));
    assert_eq!(conn.total_changes(), changes);
    assert_eq!(
        owner
            .repository
            .attempt_count(&conn, handle["runId"].as_str().unwrap())
            .unwrap(),
        1
    );
    drop(replay);
    // 作者确认的原报告证明谱系，修稿阶段仍复用同一个根与原冻结预算。
    let context: crate::services::review_revision_context::ReviewRevisionContext =
        serde_json::from_str(selection["authorInputs"][0]["text"].as_str().unwrap()).unwrap();
    let mut revise = crate::services::review_revision_context::review_revision_request(&context);
    revise.operation = ReviewRevisionOperation::RefineFromReview;
    let cycle = crate::repositories::review_cycle_repository::get_by_review_id(saved.id, &conn)
        .unwrap()
        .unwrap();
    let snapshot=json!({"kind":"human-confirmed-review","schemaVersion":2,"sourceReviewId":saved.id,
        "cycleId":cycle.cycle_id,"sourceDraft":saved.source,"summary":"作者确认合成审稿意见","authorGuidance":"",
        "items":[{"category":"剧情连贯性","severity":"warning","description":"同一事件重复呈现，需去除重复。",
            "quote":"隔离合成审修正文。","findingId":cycle.findings[0].finding_id,"decision":"apply","origin":"ai"}]}).to_string();
    let approved = crate::repositories::review_repository::create(
        &conn,
        &crate::repositories::review_repository::ReviewCreateParams {
            base_draft_id: context.source.id,
            review_index: None,
            content: snapshot.clone(),
            expected_source: Some(context.source.clone()),
        },
    )
    .unwrap();
    revise.review_source_id = Some(approved.id);
    revise.confirmed_review_content = Some(snapshot);
    let confirmed = service.prepare(&conn, &revise, || Ok(())).unwrap();
    assert_eq!(
        confirmed.parent_root_action_id.as_deref(),
        handle["rootActionId"].as_str()
    );
    let mut child = selection.clone();
    child["operation"] = json!("refine-from-review");
    child["uiActionNonce"] = json!("confirmed-revision");
    child["reviewRevisionContextId"] = json!(confirmed.context_id);
    child["authorInputs"] = json!([{"id":"review-revision-context","text":serde_json::to_value(&confirmed.context).unwrap().to_string()}]);
    child["parentRootActionId"] = handle["rootActionId"].clone();
    child["promptKeys"] = json!(["refine_from_review"]);
    child["skillStages"] = json!(["refinement"]);
    child["output"] = json!("visible-text");
    let child_view = owner.begin(&conn, &child, false, false).unwrap();
    assert_eq!(child_view["handle"]["rootActionId"], handle["rootActionId"]);
    assert_ne!(child_view["handle"]["runId"], handle["runId"]);
    assert_eq!(child_view["ledger"]["physicalRequests"], 1);
    assert_eq!(
        owner.begin(&conn, &child, false, false).unwrap()["handle"],
        child_view["handle"]
    );
}

#[test]
fn revision_owner_accepts_draft_visible_composition_but_review_rejects_test() {
    for operation in [
        ReviewRevisionOperation::RefineDraft,
        ReviewRevisionOperation::ReviewChapter,
    ] {
        let (_temp, conn, owner, _) = source_owner_fixture();
        let (_service, selection) = prepare_review(&owner, &conn, operation);
        let view = owner.begin(&conn, &selection, false, false).unwrap();
        let result =
            owner.compose_visible(&conn, &view["handle"], &[], None, Some("draft-visible-v2"));
        let error = result.unwrap_err();
        if operation == ReviewRevisionOperation::RefineDraft {
            assert!(
                !error.contains("GENERATION_COMPOSITION_ALGORITHM_INVALID"),
                "修稿使用同一正文合成算法：{error}"
            );
        } else {
            assert_eq!(
                error,
                mutating_error("GENERATION_COMPOSITION_ALGORITHM_INVALID")
            );
        }
    }
}
