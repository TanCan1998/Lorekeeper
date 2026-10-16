//! 云频道合同回归；只用临时目录、测试凭据与本地DAV夹具，不触碰真实最近项目列表。
use super::*;
#[cfg(windows)]
use crate::services::webdav_backup::tests::Server;

const PROJECT: &str = "123e4567-e89b-42d3-a456-426614174000";
const OP: &str = "11111111-1111-4111-8111-111111111111";
fn root() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "lorekeeper-cloud-command-{}",
        crate::project_access::random_uuid_v4()
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::canonicalize(root).unwrap()
}
fn state(root: &std::path::Path) -> AppState {
    let mut state = AppState::new();
    state.cloud_backup = CloudBackupState::new(&root.join("cloud"));
    state
}
fn session(active: &ActiveProject) -> Value {
    json!({"projectId":active.project_id,"leaseId":active.lease_id,"projectPath":active.root_path})
}
#[test]
fn invalid_inputs_unknown_errors_and_operation_lifecycle_stay_in_contract() {
    let root = root();
    let state = state(&root);
    assert_eq!(
        confirm_inner(&state, json!({}))["errorCode"],
        "CLOUD_BACKUP_INPUT_INVALID"
    );
    assert_eq!(
        cancel_inner(&state, OP)["errorCode"],
        "CLOUD_BACKUP_OPERATION_NOT_FOUND"
    );
    let operation = state.cloud_backup.begin(OP).unwrap();
    assert_eq!(
        cancel_inner(&state, OP),
        json!({"success":true,"cancelled":true})
    );
    assert!(operation.cancel.load(std::sync::atomic::Ordering::SeqCst));
    drop(operation);
    let value = failure("secret-path-server-error-should-not-escape", Some(OP));
    assert_eq!(value["errorCode"], "CLOUD_BACKUP_FAILED");
    assert!(!value.to_string().contains("secret-path"));
    assert!(
        view_inner(
            &state,
            json!({"projectId":"wrong","leaseId":"old","projectPath":"C:/wrong"})
        )["success"]
            == false
    );
    let _ = std::fs::remove_dir_all(root);
}
#[tokio::test]
async fn disclosure_and_duplicate_operation_fail_before_snapshot_or_network() {
    let root = root();
    let state = state(&root);
    let request = json!({"operationId":OP,"projectSession":{"projectId":PROJECT,"leaseId":"lease","projectPath":"C:/not-open"},"disclosureConfirmed":false});
    assert_eq!(
        backup_inner(&state, request.clone()).await["errorCode"],
        "CLOUD_BACKUP_DISCLOSURE_CONFIRMATION_REQUIRED"
    );
    let operation = state.cloud_backup.begin(OP).unwrap();
    assert_eq!(
        backup_inner(&state, request).await["errorCode"],
        "CLOUD_BACKUP_OPERATION_CONFLICT"
    );
    drop(operation);
    let value=restore_with_recent(&state,json!({"operationId":OP,"localEndpointAccountId":PROJECT,"cloudBookId":"book","generationId":OP,"targetGrantId":"invalid"}),|_|Ok(())).await;
    assert_eq!(value["errorCode"], "CLOUD_BACKUP_INPUT_INVALID");
    assert!(!root.join("cloud").exists());
    let _ = std::fs::remove_dir_all(root);
}
#[test]
fn committed_generation_survives_binding_revision_conflict() {
    let root = root();
    let state = state(&root);
    let captured = state
        .cloud_backup
        .local()
        .unwrap()
        .bindings
        .save_writable(
            BindingInput {
                local_project_id: PROJECT,
                cloud_book_id: "book",
                account_id: PROJECT,
                parents: &[],
            },
            None,
        )
        .unwrap();
    state
        .cloud_backup
        .local()
        .unwrap()
        .bindings
        .mark_account_unconfigured(PROJECT)
        .unwrap();
    let generation = crate::services::webdav_backup::WebDavGeneration {
        cloud_book_id: "book".into(),
        generation_id: OP.into(),
        parent_generation_ids: vec![],
        created_at: "2026-10-16T10:00:00.000Z".into(),
        origin_project_id: PROJECT.into(),
        portable_snapshot_generation: "snapshot".into(),
        archive_sha256: "a".repeat(64),
        archive_byte_size: 32,
        has_sibling: false,
        sibling_generation_ids: vec![],
    };
    let value = finish_backup(&state.cloud_backup, &captured, &generation, "snapshot", OP);
    assert_eq!(value["success"], true);
    assert_eq!(value["state"], "binding-not-saved");
    assert_eq!(value["bindingSaved"], false);
    assert_eq!(value["generation"]["generationId"], OP);
    let _ = std::fs::remove_dir_all(root);
}
#[test]
fn captured_binding_cannot_be_reused_after_revision_or_mode_changes() {
    let root = root();
    let state = state(&root);
    let input = || BindingInput {
        local_project_id: PROJECT,
        cloud_book_id: "book",
        account_id: PROJECT,
        parents: &[],
    };
    let captured = state
        .cloud_backup
        .local()
        .unwrap()
        .bindings
        .save_writable(input(), None)
        .unwrap();
    assert_binding(&state.cloud_backup, &captured).unwrap();
    state
        .cloud_backup
        .local()
        .unwrap()
        .bindings
        .save_writable(input(), Some(1))
        .unwrap();
    assert_eq!(
        assert_binding(&state.cloud_backup, &captured).unwrap_err(),
        "CLOUD_BACKUP_BINDING_CONFLICT"
    );
    state
        .cloud_backup
        .local()
        .unwrap()
        .bindings
        .mark_account_unconfigured(PROJECT)
        .unwrap();
    assert_eq!(
        assert_binding(&state.cloud_backup, &captured).unwrap_err(),
        "CLOUD_BACKUP_NOT_CONFIGURED"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(windows)]
#[tokio::test]
async fn connect_bind_backup_list_restore_readonly_and_clear_are_end_to_end() {
    let root = root();
    let state = state(&root);
    let server = Server::start();
    let project = root.join("project");
    std::fs::create_dir(&project).unwrap();
    let active = ActiveProject {
        project_id: PROJECT.into(),
        lease_id: "lease".into(),
        root_path: project.to_string_lossy().into(),
    };
    state.activate_project(active.clone()).unwrap();
    state
        .with_project_db(|conn| {
            conn.execute(
                "INSERT INTO project_core (id,project_name) VALUES ('main','云测试')",
                [],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .unwrap();
    let connected = connect_inner(
        &state,
        json!({"endpoint":server.url,"username":"u","secret":"s"}),
    )
    .await;
    assert_eq!(connected["state"], "connected", "{connected}");
    assert!(connected["account"].get("secret").is_none());
    let account = connected["account"]["accountId"]
        .as_str()
        .unwrap()
        .to_string();
    let configured = confirm_inner(
        &state,
        json!({"projectSession":session(&active),"localEndpointAccountId":account,
        "cloudBookId":"book","lastSelectedParentGenerationIds":[],"expectedRevision":null}),
    );
    assert_eq!(configured["state"], "configured", "{configured}");
    let bad=connect_inner(&state,json!({"accountId":account,"endpoint":server.url,"username":"wrong","secret":"bad-test-only"})).await;
    assert_eq!(bad["errorCode"], "CLOUD_BACKUP_AUTH_FAILED");
    assert_eq!(
        state
            .cloud_backup
            .local()
            .unwrap()
            .credentials
            .resolve(&account)
            .unwrap()
            .unwrap()
            .metadata
            .username,
        "u"
    );
    assert_eq!(view_inner(&state, session(&active))["state"], "configured");
    let backup = backup_inner(
        &state,
        json!({"operationId":OP,"projectSession":session(&active),"disclosureConfirmed":true}),
    )
    .await;
    assert_eq!(backup["state"], "backup-complete", "{backup}");
    let generation = backup["generation"]["generationId"]
        .as_str()
        .unwrap()
        .to_string();
    let listed = list_inner(
        &state,
        json!({"localEndpointAccountId":account,"cloudBookId":"book"}),
    )
    .await;
    assert_eq!(listed["generations"].as_array().unwrap().len(), 1);
    let grant = state
        .external_grants
        .lock()
        .unwrap()
        .issue_new_child(
            &root,
            "restored",
            vec![crate::external_grant::GrantOperation::Create],
            std::time::Duration::from_secs(60),
            Some(1),
        )
        .unwrap();
    let restored = restore_with_recent(
        &state,
        json!({"operationId":OP,"localEndpointAccountId":account,
        "cloudBookId":"book","generationId":generation,"targetGrantId":grant}),
        |_| Err("模拟最近列表写入失败".into()),
    )
    .await;
    assert_eq!(restored["state"], "restore-complete", "{restored}");
    assert_eq!(restored["recentProjectUpdated"], false);
    assert_eq!(restored["binding"]["mode"], "origin-readonly");
    assert_eq!(state.active_project_snapshot(), Some(active.clone()));
    assert!(crate::commands::project_archive::resolve_granted_path(
        &state,
        &grant,
        crate::external_grant::GrantOperation::Create
    )
    .is_err());
    let target = root.join("restored");
    assert!(crate::db::project_database_path(&target).exists());
    let restored_active = ActiveProject {
        project_id: restored["receipt"]["targetProjectId"]
            .as_str()
            .unwrap()
            .into(),
        lease_id: "restored-lease".into(),
        root_path: target.to_string_lossy().into(),
    };
    state.activate_project(restored_active.clone()).unwrap();
    let blocked=backup_inner(&state,json!({"operationId":OP,"projectSession":session(&restored_active),"disclosureConfirmed":true})).await;
    assert_eq!(blocked["errorCode"], "CLOUD_BACKUP_BINDING_READ_ONLY");
    // 旧会话在项目切换后不能重新确认原项目绑定。
    assert_eq!(
        confirm_inner(
            &state,
            json!({"projectSession":session(&active),"localEndpointAccountId":account,
        "cloudBookId":"book","lastSelectedParentGenerationIds":[],"expectedRevision":2})
        )["success"],
        false
    );
    let cleared = clear_inner(&state, &account);
    assert_eq!(cleared["affectedBindings"], 2);
    assert_eq!(
        view_inner(&state, session(&restored_active))["state"],
        "unconfigured"
    );
    state.invalidate_current_session().unwrap();
    assert!(std::fs::read_dir(root.join("cloud/cloud-backup-staging"))
        .unwrap()
        .next()
        .is_none());
    let _ = std::fs::remove_dir_all(root);
}
