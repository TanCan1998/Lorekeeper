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
