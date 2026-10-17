//! 专用生成入口的真实owner回归；不执行模型或外部工具。

use super::tests::source_owner_fixture;
use super::*;

fn editor_input(action: &str) -> Value {
    json!({"action":action,"documentText":"合成文字","selectedText":"合成","from":0,"to":2})
}

fn begin(
    owner: &MainGenerationOwner,
    conn: &Connection,
    kind: &str,
    input: &Value,
    nonce: &str,
) -> Result<Value, String> {
    let request = json!({"input":input,"modelId":"fixture","uiActionNonce":nonce});
    if kind == "graph" {
        owner.begin_graph_generation(conn, &request)
    } else {
        owner.begin_editor_inline(conn, &request)
    }
}

fn seed_graph_facts(conn: &Connection) {
    conn.execute("INSERT INTO blueprints (chapter_number,title,purpose,key_events) VALUES (1,'合成首章','核对示意标记','测试角色完成第一轮记录，走入中庭')", []).unwrap();
}

#[test]
fn editor_inline_owner_round_trip_preserves_context_and_task_hashes_test() {
    // 不同HashMap随机种子的独立夹具不能导致随机上下文拒绝或伪源漂移。
    for index in 0..12 {
        let (_temp, conn, owner, _) = source_owner_fixture();
        let recovery = begin(
            &owner,
            &conn,
            "editor",
            &editor_input("refine"),
            &format!("editor-control-{index}"),
        )
        .unwrap();
        assert_eq!(recovery["sourceStatus"], "current");
        let id = recovery["view"]["handle"]["runId"].as_str().unwrap();
        let run = owner.repository.get(&conn, id).unwrap();
        for _ in 0..8 {
            crate::services::editor_inline_generation::read_editor_inline_context(&run).unwrap();
            crate::services::editor_inline_generation::read_editor_inline_task(&run).unwrap();
            assert_eq!(
                owner.rebuild_binding(&conn, &run.binding).unwrap(),
                run.binding
            );
        }
    }
}

#[test]
fn graph_owner_internal_admission_keeps_public_begin_closed_and_identity_stable_test() {
    let (_temp, conn, owner, _) = source_owner_fixture();
    seed_graph_facts(&conn);
    let request =
        json!({"input":{"kind":"plot"},"modelId":"fixture","uiActionNonce":"graph-control"});
    let recovery = owner.begin_graph_generation(&conn, &request).unwrap();
    let handle = recovery["view"]["handle"].clone();
    assert_eq!(recovery["sourceStatus"], "current");
    assert_eq!(
        owner.begin_graph_generation(&conn, &request).unwrap()["view"]["handle"],
        handle
    );
    let run = owner
        .repository
        .get(&conn, handle["runId"].as_str().unwrap())
        .unwrap();
    let context =
        crate::services::graph_generation_source::read_graph_generation_context(&run).unwrap();
    assert_eq!(context.key().len(), 36);
    let graphs = crate::services::graph_generation::GraphGeneration::new(
        &conn,
        owner.repository.clone(),
        &owner.session.project_id,
    );
    graphs.task(&run).unwrap();
    assert_eq!(
        owner.rebuild_binding(&conn, &run.binding).unwrap(),
        run.binding
    );
    let public = json!({"operation":"plot-tree-snapshot","modelId":"fixture","uiActionNonce":"public-graph-forgery",
        "selectedDraftIds":[],"selectedFinalizedDraftIds":[],"promptKeys":["graph-plot"],"skillStages":[],
        "output":"structured-data","graphGenerationInput":{"kind":"plot"},"graphGenerationKey":"forged"});
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM generation_runs", [], |row| row.get(0))
        .unwrap();
    assert!(owner.begin(&conn, &public, false, false).is_err());
    let mut changed = request.clone();
    changed["input"] = json!({"kind":"plan","chapterNumber":1});
    assert_eq!(
        owner.begin_graph_generation(&conn, &changed).unwrap_err(),
        mutating_error("GENERATION_NONCE_CONFLICT")
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM generation_runs", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        count
    );
    let mut forged = request;
    forged["graphGenerationKey"] = json!("external");
    assert_eq!(
        owner.begin_graph_generation(&conn, &forged).unwrap_err(),
        mutating_error("GENERATION_GRAPH_INPUT_INVALID")
    );
}

#[test]
fn portable_specialized_nonce_reads_preserve_current_conflicts_and_frozen_writes_test() {
    use crate::services::{
        portable_project_guard, portable_project_history, portable_project_sanitize,
        portable_runtime_freeze,
    };
    for (kind, input, changed) in [
        (
            "graph",
            json!({"kind":"plot"}),
            json!({"kind":"plan","chapterNumber":1}),
        ),
        ("editor", editor_input("refine"), editor_input("expand")),
    ] {
        let (_temp, source_conn, owner, _) = source_owner_fixture();
        seed_graph_facts(&source_conn);
        let old = begin(&owner, &source_conn, kind, &input, "source-control").unwrap();
        let source =
            PathBuf::from(&owner.session.project_path).join(format!("nonce-source-{kind}.db"));
        let target =
            PathBuf::from(&owner.session.project_path).join(format!("nonce-portable-{kind}.db"));
        source_conn
            .execute("VACUUM INTO ?1", [source.to_str().unwrap()])
            .unwrap();
        let source_bytes = std::fs::read(&source).unwrap();
        portable_project_sanitize::sanitize_portable_database(
            &portable_project_sanitize::SanitizePortableDatabaseInput {
                source_database_path: &source,
                target_database_path: &target,
                limits: None,
            },
        )
        .unwrap();
        let restored = Connection::open(&target).unwrap();
        assert!(begin(&owner, &restored, kind, &input, "fresh-current")
            .unwrap_err()
            .contains("malformed JSON"));
        let document = portable_project_history::freeze_document(
            &restored,
            "123e4567-e89b-12d3-a456-426614174000",
            "nonce-read",
        )
        .unwrap();
        let guard = portable_runtime_freeze::parse_portable_runtime_freeze(
            &serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        portable_project_guard::install_guard_for_test(&restored, guard).unwrap();
        let fresh = begin(&owner, &restored, kind, &input, "fresh-current").unwrap();
        let handle = fresh["view"]["handle"].clone();
        assert_ne!(handle, old["view"]["handle"]);
        let count: i64 = restored
            .query_row("SELECT COUNT(*) FROM generation_runs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            begin(&owner, &restored, kind, &input, "fresh-current").unwrap()["view"]["handle"],
            handle
        );
        assert_eq!(
            begin(&owner, &restored, kind, &changed, "fresh-current").unwrap_err(),
            mutating_error("GENERATION_NONCE_CONFLICT")
        );
        assert_eq!(
            restored
                .query_row("SELECT COUNT(*) FROM generation_runs", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            count
        );
        restored
            .execute(
                "UPDATE generation_roots SET action_json='invalid' WHERE root_action_id=?1",
                [handle["rootActionId"].as_str().unwrap()],
            )
            .unwrap();
        assert!(begin(&owner, &restored, kind, &input, "another-current")
            .unwrap_err()
            .contains("malformed JSON"));
        assert!(restored
            .execute(
                "UPDATE generation_runs SET status='running' WHERE run_id=?1",
                [old["view"]["handle"]["runId"].as_str().unwrap()]
            )
            .unwrap_err()
            .to_string()
            .contains("PORTABLE_RUNTIME_FROZEN"));
        assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    }
}

#[test]
fn specialized_failures_roll_back_roots_runs_and_model_leases_test() {
    for kind in ["graph", "editor"] {
        let (_temp, conn, owner, _) = source_owner_fixture();
        seed_graph_facts(&conn);
        let input = if kind == "graph" {
            json!({"kind":"plot"})
        } else {
            editor_input("refine")
        };
        let counts = || {
            conn.query_row("SELECT (SELECT COUNT(*) FROM generation_roots),(SELECT COUNT(*) FROM generation_runs),(SELECT COUNT(*) FROM generation_attempts)", [], |row|Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?))).unwrap()
        };
        let before = counts();
        let leases = owner.lease_store.lock().unwrap().live_lease_count();
        conn.execute_batch("CREATE TEMP TRIGGER reject_specialized_run BEFORE INSERT ON generation_runs BEGIN SELECT RAISE(ABORT,'forced specialized rollback'); END;").unwrap();
        assert!(begin(&owner, &conn, kind, &input, "rollback-control").is_err());
        assert_eq!(counts(), before);
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), leases);
        conn.execute_batch("DROP TRIGGER reject_specialized_run;")
            .unwrap();
        let recovery = begin(&owner, &conn, kind, &input, "rollback-control").unwrap();
        assert_eq!(recovery["modelId"], "fixture");
        let leases = owner.lease_store.lock().unwrap().live_lease_count();
        let count = counts();
        assert_eq!(
            begin(&owner, &conn, kind, &input, "rollback-control").unwrap()["view"]["handle"],
            recovery["view"]["handle"]
        );
        assert_eq!(counts(), count);
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), leases);
    }
}

#[test]
fn specialized_foreign_handles_and_changed_author_sources_are_not_execution_authority_test() {
    let (_temp, conn, owner, _) = source_owner_fixture();
    seed_graph_facts(&conn);
    let graph = begin(
        &owner,
        &conn,
        "graph",
        &json!({"kind":"plot"}),
        "graph-source-drift",
    )
    .unwrap();
    let editor = begin(
        &owner,
        &conn,
        "editor",
        &editor_input("refine"),
        "editor-model-control",
    )
    .unwrap();
    let before: i64 = conn
        .query_row("SELECT COUNT(*) FROM generation_runs", [], |row| row.get(0))
        .unwrap();
    let foreign = MainGenerationOwner::new(ProjectSessionContext {
        project_id: "foreign-project".into(),
        lease_id: "foreign-lease".into(),
        project_path: owner.session.project_path.clone(),
    });
    assert!(foreign
        .read_graph_generation(&conn, &json!({"handle":graph["view"]["handle"]}))
        .is_err());
    assert!(foreign
        .read_editor_inline_recovery(&conn, &editor["view"]["handle"])
        .is_err());
    assert_eq!(owner.begin_editor_inline(&conn,&json!({"input":editor_input("refine"),"modelId":"different-model","uiActionNonce":"editor-model-control"})).unwrap_err(),mutating_error("GENERATION_NONCE_CONFLICT"));
    let old = owner
        .repository
        .get(&conn, graph["view"]["handle"]["runId"].as_str().unwrap())
        .unwrap();
    conn.execute("UPDATE blueprints SET key_events='作者已修改事件，不能借旧上下文执行' WHERE chapter_number=1",[]).unwrap();
    assert_ne!(
        owner.rebuild_binding(&conn, &old.binding).unwrap(),
        old.binding
    );
    let current = owner.graph_recovery(&conn, &old).unwrap();
    assert_eq!(current["sourceStatus"], "conflict");
    assert!(owner
        .prepare_graph_generation_execution(&conn, &json!({"handle":graph["view"]["handle"]}))
        .is_err());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM generation_runs", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM generation_attempts", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

fn prepare_specialized(
    owner: &MainGenerationOwner,
    conn: &Connection,
    kind: &str,
    handle: &Value,
) -> Result<PreparedMainGenerationExecution, String> {
    if kind == "graph" {
        owner.prepare_graph_generation_execution(conn, &json!({"handle":handle}))
    } else {
        owner.prepare_editor_inline_execution(conn, handle)
    }
}

fn reserve_prepared(
    owner: &MainGenerationOwner,
    conn: &Connection,
    prepared: &PreparedMainGenerationExecution,
    nonce: Option<&str>,
    hash: Option<&str>,
) -> GenerationExecutionReceipt {
    let PreparedExecutionPlan::Dispatch(request) = &prepared.plan else {
        panic!("首次准备应为分发计划");
    };
    owner
        .repository
        .reserve(
            conn,
            &request.run_id,
            nonce.unwrap_or(&request.invocation_nonce),
            hash.unwrap_or(&request.request_hash),
            request.reserved_tokens,
            request.requested_output_tokens,
            Some(&request.usage_policy),
            request.purpose.as_deref(),
            request.replay_task.as_ref(),
            request.budget_decision.as_ref(),
            request.reconciliation_injected,
        )
        .unwrap()
}

#[test]
fn specialized_settled_invocations_replay_without_new_attempts_or_author_source_authority_test() {
    for kind in ["graph", "editor"] {
        let (_temp, conn, owner, _) = source_owner_fixture();
        seed_graph_facts(&conn);
        let input = if kind == "graph" {
            json!({"kind":"plot"})
        } else {
            editor_input("refine")
        };
        let recovery = begin(&owner, &conn, kind, &input, "settled-replay").unwrap();
        let handle = &recovery["view"]["handle"];
        let first = prepare_specialized(&owner, &conn, kind, handle).unwrap();
        let receipt = reserve_prepared(&owner, &conn, &first, None, None);
        owner
            .repository
            .mark_dispatched(&conn, &receipt.attempt.attempt_id)
            .unwrap();
        let artifact = receipt.artifact.unwrap();
        owner
            .repository
            .snapshot(
                &conn,
                &receipt.attempt.attempt_id,
                artifact.revision,
                "合成回执输出",
            )
            .unwrap();
        owner
            .repository
            .settle(
                &conn,
                &receipt.attempt.attempt_id,
                None,
                Some("stop".into()),
                None,
                None,
            )
            .unwrap();
        drop(first);
        // 改变作者来源后仍只能读回原收据，不能因此再次分发或伪造当前来源。
        conn.execute(
            "UPDATE blueprints SET key_events='作者已更新事件' WHERE chapter_number=1",
            [],
        )
        .unwrap();
        let before: i64 = conn
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        let leases = owner.lease_store.lock().unwrap().live_lease_count();
        let replay = prepare_specialized(&owner, &conn, kind, handle).unwrap();
        assert!(
            matches!(&replay.plan, PreparedExecutionPlan::Replay(_)),
            "{kind}必须是纯收据重放"
        );
        // 仅在证明Replay后调用，不允许测试走真实网络分发。
        let result = replay.execute_blocking(&conn).unwrap();
        let outcome = owner
            .finish_prepared_execution(&conn, &replay, &result)
            .unwrap();
        assert_eq!(outcome["outcome"]["content"], "合成回执输出");
        assert_eq!(outcome["run"]["ledger"]["physicalRequests"], 1);
        assert_eq!(
            conn.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            before
        );
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), leases);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM generation_attempts", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

#[test]
fn specialized_pending_invocations_reuse_frozen_request_and_pending_guards_test() {
    for kind in ["graph", "editor"] {
        for dispatched in [false, true] {
            let (_temp, conn, owner, _) = source_owner_fixture();
            seed_graph_facts(&conn);
            let input = if kind == "graph" {
                json!({"kind":"plot"})
            } else {
                editor_input("refine")
            };
            let recovery = begin(&owner, &conn, kind, &input, "pending-replay").unwrap();
            let handle = &recovery["view"]["handle"];
            let first = prepare_specialized(&owner, &conn, kind, handle).unwrap();
            let receipt = reserve_prepared(&owner, &conn, &first, None, None);
            if dispatched {
                owner
                    .repository
                    .mark_dispatched(&conn, &receipt.attempt.attempt_id)
                    .unwrap();
            }
            let again = prepare_specialized(&owner, &conn, kind, handle).unwrap();
            let (PreparedExecutionPlan::Dispatch(left), PreparedExecutionPlan::Dispatch(right)) =
                (&first.plan, &again.plan)
            else {
                panic!("在途请求交回已有服务合并，不冒充完成收据");
            };
            assert_eq!(left.request_hash, right.request_hash);
            assert_eq!(left.invocation_nonce, right.invocation_nonce);
            assert_eq!(
                conn.query_row("SELECT COUNT(*) FROM generation_attempts", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(owner.pending_invocations.lock().unwrap().len(), 1);
            drop(again);
            assert_eq!(owner.pending_invocations.lock().unwrap().len(), 1);
            drop(first);
            assert!(owner.pending_invocations.lock().unwrap().is_empty());
        }
    }
}

#[test]
fn specialized_unrelated_attempts_and_request_hash_collisions_stay_rejected_test() {
    for kind in ["graph", "editor"] {
        for collision in [false, true] {
            let (_temp, conn, owner, _) = source_owner_fixture();
            seed_graph_facts(&conn);
            let input = if kind == "graph" {
                json!({"kind":"plot"})
            } else {
                editor_input("refine")
            };
            let recovery = begin(&owner, &conn, kind, &input, "conflict-replay").unwrap();
            let handle = &recovery["view"]["handle"];
            let first = prepare_specialized(&owner, &conn, kind, handle).unwrap();
            let wrong = "b".repeat(64);
            reserve_prepared(
                &owner,
                &conn,
                &first,
                if collision { None } else { Some("unrelated") },
                if collision { Some(&wrong) } else { None },
            );
            let before: i64 = conn
                .query_row("SELECT total_changes()", [], |r| r.get(0))
                .unwrap();
            let error = match prepare_specialized(&owner, &conn, kind, handle) {
                Ok(_) => panic!("冲突不得重用"),
                Err(error) => error,
            };
            let expected = if collision {
                "GENERATION_INVOCATION_CONFLICT"
            } else if kind == "graph" {
                "GENERATION_GRAPH_ATTEMPT_CONFLICT"
            } else {
                "GENERATION_EDITOR_ATTEMPT_CONFLICT"
            };
            assert_eq!(error, mutating_error(expected));
            assert_eq!(
                conn.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                before
            );
        }
    }
}

#[test]
fn specialized_receipt_replay_does_not_admit_old_epochs_or_reactivate_cancelled_roots_test() {
    for kind in ["graph", "editor"] {
        let (_temp, conn, owner, _) = source_owner_fixture();
        seed_graph_facts(&conn);
        let input = if kind == "graph" {
            json!({"kind":"plot"})
        } else {
            editor_input("refine")
        };
        let recovery = begin(&owner, &conn, kind, &input, "epoch-replay").unwrap();
        let handle = &recovery["view"]["handle"];
        let first = prepare_specialized(&owner, &conn, kind, handle).unwrap();
        let receipt = reserve_prepared(&owner, &conn, &first, None, None);
        owner
            .repository
            .mark_dispatched(&conn, &receipt.attempt.attempt_id)
            .unwrap();
        owner
            .repository
            .settle(
                &conn,
                &receipt.attempt.attempt_id,
                None,
                Some("stop".into()),
                None,
                None,
            )
            .unwrap();
        drop(first);
        let next = MainGenerationOwner::new(ProjectSessionContext {
            project_id: owner.session.project_id.clone(),
            lease_id: "new-epoch".into(),
            project_path: owner.session.project_path.clone(),
        });
        let before: i64 = conn
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        let error = match prepare_specialized(&next, &conn, kind, handle) {
            Ok(_) => panic!("旧epoch不得借收据执行"),
            Err(error) => error,
        };
        assert_eq!(error, mutating_error("GENERATION_EPOCH_STALE"));
        assert_eq!(
            conn.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            before
        );
        owner
            .run_service
            .cancel(&conn, handle["rootActionId"].as_str().unwrap())
            .unwrap();
        let before: i64 = conn
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        let replay = prepare_specialized(&owner, &conn, kind, handle).unwrap();
        assert!(matches!(&replay.plan, PreparedExecutionPlan::Replay(_)));
        assert_eq!(
            owner
                .repository
                .budget(&conn, handle["rootActionId"].as_str().unwrap())
                .unwrap()
                .root
                .status,
            "cancelled"
        );
        assert_eq!(
            conn.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            before
        );
    }
}

#[test]
fn specialized_unverifiable_receipts_remain_attempt_conflicts_without_side_effects_test() {
    for kind in ["graph", "editor"] {
        for metadata in [
            None,
            Some("{}"),
            Some("invalid"),
            Some("{\"requestHash\":null}"),
        ] {
            let (_temp, conn, owner, _) = source_owner_fixture();
            seed_graph_facts(&conn);
            let input = if kind == "graph" {
                json!({"kind":"plot"})
            } else {
                editor_input("refine")
            };
            let recovery = begin(&owner, &conn, kind, &input, "unverifiable-replay").unwrap();
            let handle = &recovery["view"]["handle"];
            let first = prepare_specialized(&owner, &conn, kind, handle).unwrap();
            let receipt = reserve_prepared(&owner, &conn, &first, None, None);
            conn.execute(
                "UPDATE generation_attempts SET usage_receipt_json=?1 WHERE attempt_id=?2",
                rusqlite::params![metadata, receipt.attempt.attempt_id],
            )
            .unwrap();
            let before: i64 = conn
                .query_row("SELECT total_changes()", [], |r| r.get(0))
                .unwrap();
            let leases = owner.lease_store.lock().unwrap().live_lease_count();
            let error = match prepare_specialized(&owner, &conn, kind, handle) {
                Ok(_) => panic!("不可验证尝试不得放行"),
                Err(error) => error,
            };
            let expected = if kind == "graph" {
                // 图域原有可变性守卫可能先报坏收据，不能为统一文案改写它。
                let run = owner
                    .repository
                    .get(&conn, handle["runId"].as_str().unwrap())
                    .unwrap();
                crate::services::graph_generation::GraphGeneration::new(
                    &conn,
                    owner.repository.clone(),
                    &owner.session.project_id,
                )
                .assert_mutable(&run)
                .err()
                .map(|error| mutating_error(&error))
                .unwrap_or_else(|| mutating_error("GENERATION_GRAPH_ATTEMPT_CONFLICT"))
            } else {
                mutating_error("GENERATION_EDITOR_ATTEMPT_CONFLICT")
            };
            assert_eq!(error, expected);
            assert_eq!(
                conn.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                before
            );
            assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), leases);
        }
    }
}

// 新租约必须显式恢复；只更新运行绑定，冻结任务/历史收据不重新采样。
#[test]
fn specialized_resume_revalidates_new_epoch_without_replacing_frozen_context_test() {
    for kind in ["graph", "editor"] {
        let (_temp, conn, mut owner, _) = source_owner_fixture();
        seed_graph_facts(&conn);
        let input = if kind == "graph" {
            json!({"kind":"plan","chapterNumber":1})
        } else {
            editor_input("refine")
        };
        let recovery = begin(&owner, &conn, kind, &input, "resume-ready").unwrap();
        let old = &recovery["view"]["handle"];
        let before = owner
            .repository
            .get(&conn, old["runId"].as_str().unwrap())
            .unwrap();
        let mut next = MainGenerationOwner::new(ProjectSessionContext {
            lease_id: "resumed-epoch".into(),
            ..owner.session.clone()
        });
        next.global_data_root = owner.global_data_root.clone();
        owner.suspend_with_database(&conn).unwrap();
        assert!(prepare_specialized(&next, &conn, kind, old).is_err());
        let resumed = next.resume(&conn, old).unwrap();
        let handle = &resumed["handle"];
        assert_eq!(handle["epoch"], "resumed-epoch");
        assert_eq!(handle["runId"], old["runId"]);
        assert_eq!(handle["rootActionId"], old["rootActionId"]);
        let run = next
            .repository
            .get(&conn, handle["runId"].as_str().unwrap())
            .unwrap();
        assert_eq!(run.binding.source_manifest, before.binding.source_manifest);
        assert_eq!(run.binding.fingerprint, before.binding.fingerprint);
        assert!(prepare_specialized(&next, &conn, kind, old).is_err());
        let prepared = prepare_specialized(&next, &conn, kind, handle).unwrap();
        assert!(matches!(prepared.plan, PreparedExecutionPlan::Dispatch(_)));
        assert_eq!(resumed["ledger"]["physicalRequests"], 0);
    }
}

#[test]
fn specialized_resume_rejects_drift_cancelled_task_tampering_foreign_scope_and_outer_transaction_test(
) {
    for kind in ["graph", "editor"] {
        for failure in [
            "source",
            "model",
            "cancelled",
            "task",
            "foreign",
            "transaction",
        ] {
            let (_temp, conn, mut owner, _) = source_owner_fixture();
            seed_graph_facts(&conn);
            let input = if kind == "graph" {
                json!({"kind":"plot"})
            } else {
                editor_input("refine")
            };
            let recovery = begin(&owner, &conn, kind, &input, "resume-reject").unwrap();
            let handle = &recovery["view"]["handle"];
            if failure == "cancelled" {
                owner.cancel(&conn, handle).unwrap();
            }
            let mut next = MainGenerationOwner::new(ProjectSessionContext {
                lease_id: "rejected-epoch".into(),
                ..owner.session.clone()
            });
            next.global_data_root = owner.global_data_root.clone();
            owner.suspend_with_database(&conn).unwrap();
            match failure {
                "source" => {
                    conn.execute(
                        "UPDATE project_core SET writing_language='en-US' WHERE id='main'",
                        [],
                    )
                    .unwrap();
                }
                "model" => {
                    let path = crate::app_paths::models_config_path_at(&next.global_data_root);
                    let mut models: Value =
                        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
                    models[0]["modelName"] = json!("synthetic-changed");
                    std::fs::write(path, serde_json::to_string(&models).unwrap()).unwrap();
                }
                "task" => {
                    let field = if kind == "graph" {
                        "$.sourceManifest.graphGenerationTaskHash"
                    } else {
                        "$.sourceManifest.editorInlineTaskHash"
                    };
                    conn.execute("UPDATE generation_runs SET binding_json=json_set(binding_json,?1,?2) WHERE run_id=?3",rusqlite::params![field,"a".repeat(64),handle["runId"].as_str().unwrap()]).unwrap();
                }
                "foreign" => {
                    next.session.project_id = "foreign-project".into();
                }
                "transaction" => {
                    conn.execute_batch("BEGIN").unwrap();
                }
                _ => {}
            }
            let changes: i64 = conn
                .query_row("SELECT total_changes()", [], |r| r.get(0))
                .unwrap();
            let leases = next.lease_store.lock().unwrap().live_lease_count();
            assert!(
                next.resume(&conn, handle).is_err(),
                "{kind}/{failure}不得恢复"
            );
            assert_eq!(
                conn.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                changes,
                "{kind}/{failure}不得写入"
            );
            assert_eq!(
                next.lease_store.lock().unwrap().live_lease_count(),
                leases,
                "{kind}/{failure}不得泄漏租约"
            );
            if failure == "transaction" {
                conn.execute_batch("ROLLBACK").unwrap();
            }
        }
    }
}

#[test]
fn specialized_resume_preserves_historical_attempt_artifact_and_receipt_without_duplicate_dispatch_test(
) {
    for kind in ["graph", "editor"] {
        let (_temp, conn, mut owner, _) = source_owner_fixture();
        seed_graph_facts(&conn);
        let input = if kind == "graph" {
            json!({"kind":"plot"})
        } else {
            editor_input("refine")
        };
        let recovery = begin(&owner, &conn, kind, &input, "resume-settled").unwrap();
        let old = &recovery["view"]["handle"];
        let prepared = prepare_specialized(&owner, &conn, kind, old).unwrap();
        let receipt = reserve_prepared(&owner, &conn, &prepared, None, None);
        let attempt_id = receipt.attempt.attempt_id.clone();
        owner
            .repository
            .mark_dispatched(&conn, &attempt_id)
            .unwrap();
        owner
            .repository
            .snapshot(
                &conn,
                &attempt_id,
                receipt.artifact.unwrap().revision,
                "合成历史输出",
            )
            .unwrap();
        owner
            .repository
            .settle(&conn, &attempt_id, None, Some("stop".into()), None, None)
            .unwrap();
        drop(prepared);
        let original = owner.repository.receipt(&conn, &attempt_id, None).unwrap();
        let metadata: String = conn
            .query_row(
                "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id=?1",
                [&attempt_id],
                |r| r.get(0),
            )
            .unwrap();
        let mut next = MainGenerationOwner::new(ProjectSessionContext {
            lease_id: "history-resumed-epoch".into(),
            ..owner.session.clone()
        });
        next.global_data_root = owner.global_data_root.clone();
        owner.suspend_with_database(&conn).unwrap();
        let resumed = next.resume(&conn, old).unwrap();
        let handle = &resumed["handle"];
        let historical = next.repository.receipt(&conn, &attempt_id, None).unwrap();
        assert_eq!(
            serde_json::to_value(&historical.attempt).unwrap(),
            serde_json::to_value(&original.attempt).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&historical.artifact).unwrap(),
            serde_json::to_value(&original.artifact).unwrap()
        );
        assert_eq!(
            conn.query_row(
                "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id=?1",
                [&attempt_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            metadata
        );
        let changes: i64 = conn
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        let replay = prepare_specialized(&next, &conn, kind, handle).unwrap();
        assert!(matches!(&replay.plan, PreparedExecutionPlan::Replay(_)));
        let result = replay.execute_blocking(&conn).unwrap();
        let outcome = next
            .finish_prepared_execution(&conn, &replay, &result)
            .unwrap();
        assert_eq!(outcome["outcome"]["content"], "合成历史输出");
        assert_eq!(outcome["run"]["ledger"]["physicalRequests"], 1);
        assert_eq!(
            conn.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            changes
        );
    }
}

#[test]
fn specialized_resume_refuses_real_connection_frozen_full_context_without_lease_or_writes_test() {
    use crate::services::{
        portable_project_guard, portable_project_history, portable_runtime_freeze,
    };
    for kind in ["graph", "editor"] {
        let (_temp, conn, mut owner, _) = source_owner_fixture();
        seed_graph_facts(&conn);
        let input = if kind == "graph" {
            json!({"kind":"plot"})
        } else {
            editor_input("refine")
        };
        let recovery = begin(&owner, &conn, kind, &input, "resume-frozen").unwrap();
        let handle = &recovery["view"]["handle"];
        let mut next = MainGenerationOwner::new(ProjectSessionContext {
            lease_id: "frozen-next-epoch".into(),
            ..owner.session.clone()
        });
        next.global_data_root = owner.global_data_root.clone();
        owner.suspend_with_database(&conn).unwrap();
        let document = portable_project_history::freeze_document(
            &conn,
            "123e4567-e89b-12d3-a456-426614174000",
            "specialized-resume",
        )
        .unwrap();
        let guard = portable_runtime_freeze::parse_portable_runtime_freeze(
            &serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        portable_project_guard::install_guard_for_test(&conn, guard).unwrap();
        assert!(portable_project_guard::frozen_row(
            &conn,
            "generation_runs",
            handle["runId"].as_str().unwrap()
        ));
        let changes: i64 = conn
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        assert!(next.resume(&conn, handle).is_err());
        assert_eq!(
            conn.query_row("SELECT total_changes()", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            changes
        );
        assert_eq!(next.lease_store.lock().unwrap().live_lease_count(), 0);
    }
}
