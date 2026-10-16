//! 云备份八频道：显式披露确认、项目租约、账户/绑定修订、双向追加与取消。
//! 账户明文仅在Rust调用栈与DPAPI存储器内流转，不进入返回信封或错误日志。
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::State;

use crate::security::ProjectSessionContext;
use crate::services::cloud_backup_state::CloudBackupState;
use crate::services::cloud_project_binding_store::{
    BindingInput, BindingMode, CloudProjectBinding,
};
use crate::services::portable_project_archive as files;
use crate::services::webdav_backup::{AppendInput, WebDavAccount, WebDavService};
use crate::state::{ActiveProject, AppState};

fn failure(code: &str, id: Option<&str>) -> Value {
    let mapped = match code {
        "WEBDAV_AUTH" => "CLOUD_BACKUP_AUTH_FAILED",
        "WEBDAV_CANCELLED" | "PORTABLE_RESTORE_CANCELLED" => "CLOUD_BACKUP_CANCELLED",
        "WEBDAV_NETWORK" => "CLOUD_BACKUP_NETWORK_FAILED",
        "WEBDAV_NOT_FOUND"
        | "WEBDAV_REMOTE_CONFLICT"
        | "WEBDAV_REMOTE_INVALID"
        | "WEBDAV_DAV_RESPONSE_INVALID"
        | "WEBDAV_RESPONSE_TOO_LARGE" => "CLOUD_BACKUP_REMOTE_INVALID",
        "CLOUD_BINDING_REVISION_CONFLICT" => "CLOUD_BACKUP_BINDING_CONFLICT",
        "CLOUD_BACKUP_INPUT_INVALID"
        | "CLOUD_BACKUP_DISCLOSURE_CONFIRMATION_REQUIRED"
        | "CLOUD_BACKUP_NOT_CONFIGURED"
        | "CLOUD_BACKUP_CREDENTIAL_UNAVAILABLE"
        | "CLOUD_BACKUP_BINDING_READ_ONLY"
        | "CLOUD_BACKUP_BINDING_CONFLICT"
        | "CLOUD_BACKUP_OPERATION_CONFLICT"
        | "CLOUD_BACKUP_OPERATION_NOT_FOUND"
        | "CLOUD_BACKUP_AUTH_FAILED"
        | "CLOUD_BACKUP_NETWORK_FAILED"
        | "CLOUD_BACKUP_REMOTE_INVALID"
        | "CLOUD_BACKUP_ARCHIVE_FAILED"
        | "CLOUD_BACKUP_RESTORE_FAILED"
        | "CLOUD_BACKUP_LOCAL_STATE_NOT_SAVED"
        | "CLOUD_BACKUP_CANCELLED"
        | "CLOUD_BACKUP_FAILED" => code,
        _ => "CLOUD_BACKUP_FAILED",
    };
    let mut value = json!({"success":false,"state":if mapped=="CLOUD_BACKUP_CANCELLED"{"cancelled"}else{"failed"},"errorCode":mapped});
    if let Some(id) = id.filter(|id| crate::services::cloud_credential_store::valid_account_id(id))
    {
        value["operationId"] = id.into();
    }
    value
}
fn response(result: Result<Value, String>, id: Option<&str>) -> Value {
    result.unwrap_or_else(|err| failure(&err, id))
}
fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|_| "CLOUD_BACKUP_INPUT_INVALID".into())
}
fn current(state: &AppState, session: &ProjectSessionContext) -> Result<ActiveProject, String> {
    let active = state.active_project_snapshot();
    crate::security::assert_current_project_context(session, active.as_ref())
        .map_err(|_| "CLOUD_BACKUP_FAILED".to_string())?;
    active.ok_or_else(|| "CLOUD_BACKUP_FAILED".into())
}
fn account(cloud: &CloudBackupState, id: &str) -> Result<WebDavAccount, String> {
    let value = cloud
        .local()?
        .credentials
        .resolve(id)
        .map_err(|_| "CLOUD_BACKUP_CREDENTIAL_UNAVAILABLE".to_string())?
        .ok_or_else(|| "CLOUD_BACKUP_CREDENTIAL_UNAVAILABLE".to_string())?;
    let secret = value
        .secret
        .filter(|secret| !secret.is_empty())
        .ok_or_else(|| "CLOUD_BACKUP_CREDENTIAL_UNAVAILABLE".to_string())?;
    if value.metadata.account_id != id {
        return Err("CLOUD_BACKUP_CREDENTIAL_UNAVAILABLE".into());
    }
    Ok(WebDavAccount {
        endpoint: value.metadata.endpoint,
        username: value.metadata.username,
        secret,
    })
}
fn now() -> String {
    crate::commands::project::iso8601_utc_from_millis(crate::commands::project::epoch_millis_now())
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectRequest {
    account_id: Option<String>,
    endpoint: String,
    username: String,
    secret: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfirmRequest {
    project_session: ProjectSessionContext,
    local_endpoint_account_id: String,
    cloud_book_id: Option<String>,
    last_selected_parent_generation_ids: Vec<String>,
    expected_revision: Option<u64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupRequest {
    operation_id: String,
    project_session: ProjectSessionContext,
    disclosure_confirmed: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListRequest {
    local_endpoint_account_id: String,
    cloud_book_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreRequest {
    operation_id: String,
    local_endpoint_account_id: String,
    cloud_book_id: String,
    generation_id: String,
    target_grant_id: String,
}

async fn connect_inner(state: &AppState, raw: Value) -> Value {
    response(
        async {
            if !cfg!(windows) {
                return Err("CLOUD_BACKUP_CREDENTIAL_UNAVAILABLE".into());
            }
            let request: ConnectRequest = parse(raw)?;
            crate::services::cloud_credential_store::validate_input(
                request.account_id.as_deref(),
                &request.endpoint,
                &request.username,
                &request.secret,
            )
            .map_err(|_| "CLOUD_BACKUP_INPUT_INVALID".to_string())?;
            let remote = WebDavAccount {
                endpoint: request.endpoint.clone(),
                username: request.username.clone(),
                secret: request.secret.clone(),
            };
            WebDavService::new()?
                .check_connection(&remote, &crate::services::webdav_backup::cancellation())
                .await?;
            let saved = state
                .cloud_backup
                .local()?
                .credentials
                .save(
                    request.account_id.as_deref(),
                    &request.endpoint,
                    &request.username,
                    &request.secret,
                )
                .map_err(|_| "CLOUD_BACKUP_LOCAL_STATE_NOT_SAVED".to_string())?;
            let mut value = json!({"success":true,"state":"connected","account":saved.metadata});
            if let Some(warning) = saved.warning {
                value["warning"] = warning.into();
            }
            Ok(value)
        }
        .await,
        None,
    )
}
fn view_inner(state: &AppState, raw: Value) -> Value {
    response(
        (|| {
            let session: ProjectSessionContext = parse(raw)?;
            let active = current(state, &session)?;
            let local = state.cloud_backup.local()?;
            let binding = local.bindings.get(&active.project_id)?;
            let resolved = binding
                .as_ref()
                .map(|binding| {
                    local
                        .credentials
                        .resolve(&binding.local_endpoint_account_id)
                })
                .transpose()
                .map_err(|_| "CLOUD_BACKUP_CREDENTIAL_UNAVAILABLE".to_string())?
                .flatten();
            let configured = binding
                .as_ref()
                .is_some_and(|binding| binding.mode != BindingMode::Unconfigured)
                && resolved.as_ref().is_some_and(|r| r.secret.is_some());
            Ok(
                json!({"success":true,"state":if configured{"configured"}else{"unconfigured"},
            "binding":binding,"account":resolved.map(|r|r.metadata)}),
            )
        })(),
        None,
    )
}
fn confirm_inner(state: &AppState, raw: Value) -> Value {
    response(
        (|| {
            if raw.get("expectedRevision").is_none() {
                return Err("CLOUD_BACKUP_INPUT_INVALID".into());
            }
            let request: ConfirmRequest = parse(raw)?;
            let active = current(state, &request.project_session)?;
            account(&state.cloud_backup, &request.local_endpoint_account_id)?;
            let book = request
                .cloud_book_id
                .unwrap_or_else(crate::project_access::random_uuid_v4);
            let mut local = state.cloud_backup.local()?;
            let previous = local
                .bindings
                .get(&active.project_id)?
                .map(|b| b.local_endpoint_account_id);
            let saved = local.bindings.save_writable(
                BindingInput {
                    local_project_id: &active.project_id,
                    cloud_book_id: &book,
                    account_id: &request.local_endpoint_account_id,
                    parents: &request.last_selected_parent_generation_ids,
                },
                request.expected_revision,
            )?;
            let mut stale = false;
            if let Some(old) = previous.filter(|id| *id != request.local_endpoint_account_id) {
                match local.bindings.list() {
                    Ok(bindings)
                        if !bindings.iter().any(|b| b.local_endpoint_account_id == old) =>
                    {
                        stale = local.credentials.clear(&old).is_err();
                    }
                    Err(_) => stale = true,
                    _ => {}
                }
            }
            let mut value = json!({"success":true,"state":"configured","binding":saved});
            if stale {
                value["staleCredentialRetained"] = true.into();
            }
            Ok(value)
        })(),
        None,
    )
}
fn clear_inner(state: &AppState, id: &str) -> Value {
    response(
        (|| {
            let mut local = state.cloud_backup.local()?;
            let affected = local
                .bindings
                .mark_account_unconfigured(id)
                .map_err(|_| "CLOUD_BACKUP_LOCAL_STATE_NOT_SAVED".to_string())?;
            local
                .credentials
                .clear(id)
                .map_err(|_| "CLOUD_BACKUP_LOCAL_STATE_NOT_SAVED".to_string())?;
            Ok(json!({"success":true,"state":"unconfigured","affectedBindings":affected}))
        })(),
        None,
    )
}
fn cancel_inner(state: &AppState, id: &str) -> Value {
    response(
        state
            .cloud_backup
            .cancel(id)
            .map(|_| json!({"success":true,"cancelled":true})),
        None,
    )
}
/// 暂存清理不能覆盖已经完成的远端/恢复结果；只清理已登记身份，绝不递归删陌生文件。
struct Staging {
    attempt: files::StagingAttempt,
    pub archive: PathBuf,
}
impl Staging {
    fn new(cloud: &CloudBackupState, name: &str) -> Result<Self, String> {
        let parent = cloud.root.join("cloud-backup-staging");
        std::fs::create_dir_all(&parent)
            .map_err(|_| "CLOUD_BACKUP_LOCAL_STATE_NOT_SAVED".to_string())?;
        files::safe_directory(&parent)
            .map_err(|_| "CLOUD_BACKUP_LOCAL_STATE_NOT_SAVED".to_string())?;
        let mut attempt = files::prepare_staging(&parent)
            .map_err(|_| "CLOUD_BACKUP_LOCAL_STATE_NOT_SAVED".to_string())?;
        let archive = files::target_for(&mut attempt, name)
            .map_err(|_| "CLOUD_BACKUP_LOCAL_STATE_NOT_SAVED".to_string())?;
        Ok(Self { attempt, archive })
    }
    fn record_archive(&mut self) -> Result<(), String> {
        files::assert_attempt_directory(&self.attempt, &self.attempt.root)
            .map_err(|_| "CLOUD_BACKUP_LOCAL_STATE_NOT_SAVED".to_string())?;
        let identity = files::path_identity(&self.archive)
            .map_err(|_| "CLOUD_BACKUP_LOCAL_STATE_NOT_SAVED".to_string())?;
        if !identity.is_file
            || identity.is_symlink
            || files::real_path_text(&self.archive) != files::literal_path_text(&self.archive)
        {
            return Err("CLOUD_BACKUP_LOCAL_STATE_NOT_SAVED".into());
        }
        self.attempt.record_file(&self.archive, identity);
        Ok(())
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        files::cleanup_attempt(&self.attempt);
    }
}

fn assert_binding(cloud: &CloudBackupState, captured: &CloudProjectBinding) -> Result<(), String> {
    let current = cloud
        .local()?
        .bindings
        .get(&captured.local_project_id)?
        .ok_or_else(|| "CLOUD_BACKUP_NOT_CONFIGURED".to_string())?;
    if current.mode == BindingMode::Unconfigured {
        return Err("CLOUD_BACKUP_NOT_CONFIGURED".into());
    }
    if current.mode != BindingMode::Writable {
        return Err("CLOUD_BACKUP_BINDING_READ_ONLY".into());
    }
    if &current != captured {
        return Err("CLOUD_BACKUP_BINDING_CONFLICT".into());
    }
    Ok(())
}
async fn backup_inner(state: &AppState, raw: Value) -> Value {
    let request: BackupRequest = match parse(raw) {
        Ok(r) => r,
        Err(e) => return failure(&e, None),
    };
    let operation = match state.cloud_backup.begin(&request.operation_id) {
        Ok(operation) => operation,
        Err(e) => return failure(&e, Some(&request.operation_id)),
    };
    let result = async {
        if !request.disclosure_confirmed {
            return Err("CLOUD_BACKUP_DISCLOSURE_CONFIRMATION_REQUIRED".into());
        }
        let active = current(state, &request.project_session)?;
        let captured = state
            .cloud_backup
            .local()?
            .bindings
            .get(&active.project_id)?
            .ok_or_else(|| "CLOUD_BACKUP_NOT_CONFIGURED".to_string())?;
        assert_binding(&state.cloud_backup, &captured)?;
        let captured_account = account(&state.cloud_backup, &captured.local_endpoint_account_id)?;
        let mut staging = Staging::new(&state.cloud_backup, "backup.ainovel")?;
        let archive = staging.archive.clone();
        let parent = staging.attempt.root.clone();
        let source = PathBuf::from(&active.root_path);
        let origin = active.project_id.clone();
        let created = now();
        let receipt = tauri::async_runtime::spawn_blocking(move || {
            crate::services::portable_project_export::export_portable_project(
                &crate::services::portable_project_export::ExportPortableProjectInput {
                    source_project_root: &source,
                    origin_project_id: &origin,
                    target_archive_path: &archive,
                    attempt_parent_path: &parent,
                    snapshot_generation: None,
                    created_at: &created,
                    limits: None,
                },
            )
        })
        .await
        .map_err(|_| "CLOUD_BACKUP_ARCHIVE_FAILED".to_string())?
        .map_err(|_| {
            if operation.cancel.load(std::sync::atomic::Ordering::SeqCst) {
                "CLOUD_BACKUP_CANCELLED"
            } else {
                "CLOUD_BACKUP_ARCHIVE_FAILED"
            }
            .to_string()
        })?;
        staging.record_archive()?;
        crate::services::webdav_backup::check_cancel(&operation.cancel)?;
        current(state, &request.project_session)?;
        assert_binding(&state.cloud_backup, &captured)?;
        let remote = account(&state.cloud_backup, &captured.local_endpoint_account_id)?;
        if remote.endpoint != captured_account.endpoint
            || remote.username != captured_account.username
        {
            return Err("CLOUD_BACKUP_BINDING_CONFLICT".into());
        }
        let generation = crate::project_access::random_uuid_v4();
        let remote_generation = WebDavService::new()?
            .append(AppendInput {
                account: &remote,
                cloud_book_id: &captured.cloud_book_id,
                archive: &staging.archive,
                origin_project_id: &receipt.origin_project_id,
                snapshot_generation: &receipt.snapshot_generation,
                parents: &captured.last_selected_parent_generation_ids,
                generation_id: &generation,
                created_at: &now(),
                cancel: &operation.cancel,
            })
            .await?;
        Ok(finish_backup(
            &state.cloud_backup,
            &captured,
            &remote_generation,
            &receipt.snapshot_generation,
            &operation.id,
        ))
    }
    .await;
    response(result, Some(&operation.id))
}
/// 远端已经完成：本机CAS/磁盘失败只能降级为binding-not-saved，不可伪报备份失败。
fn finish_backup(
    cloud: &CloudBackupState,
    captured: &CloudProjectBinding,
    generation: &crate::services::webdav_backup::WebDavGeneration,
    backup_point: &str,
    operation_id: &str,
) -> Value {
    let parents = vec![generation.generation_id.clone()];
    let saved = cloud.local().and_then(|mut local| {
        local.bindings.save_writable(
            BindingInput {
                local_project_id: &captured.local_project_id,
                cloud_book_id: &captured.cloud_book_id,
                account_id: &captured.local_endpoint_account_id,
                parents: &parents,
            },
            Some(captured.revision),
        )
    });
    json!({"success":true,"state":if saved.is_ok(){"backup-complete"}else{"binding-not-saved"},
        "operationId":operation_id,"generation":generation,"backupPoint":backup_point,
        "bindingSaved":saved.is_ok(),"binding":saved.ok()})
}

async fn list_inner(state: &AppState, raw: Value) -> Value {
    response(
        async {
            let request: ListRequest = parse(raw)?;
            let remote = account(&state.cloud_backup, &request.local_endpoint_account_id)?;
            let generations = WebDavService::new()?
                .list(
                    &remote,
                    &request.cloud_book_id,
                    &crate::services::webdav_backup::cancellation(),
                )
                .await?;
            Ok(json!({"success":true,"state":"listed","generations":generations}))
        }
        .await,
        None,
    )
}
async fn restore_inner(state: &AppState, raw: Value) -> Value {
    restore_with_recent(
        state,
        raw,
        crate::commands::project_archive::register_restored_recent,
    )
    .await
}
async fn restore_with_recent(
    state: &AppState,
    raw: Value,
    register: impl FnOnce(
            &crate::services::portable_project_restore_install::RestorePortableProjectReceipt,
        ) -> Result<(), String>
        + Send,
) -> Value {
    let request: RestoreRequest = match parse(raw) {
        Ok(r) => r,
        Err(e) => return failure(&e, None),
    };
    let operation = match state.cloud_backup.begin(&request.operation_id) {
        Ok(operation) => operation,
        Err(e) => return failure(&e, Some(&request.operation_id)),
    };
    let result=async{
        let target=crate::commands::project_archive::resolve_granted_path(
            state,&request.target_grant_id,crate::external_grant::GrantOperation::Create,
        ).map_err(|_|"CLOUD_BACKUP_INPUT_INVALID".to_string())?;
        let remote=account(&state.cloud_backup,&request.local_endpoint_account_id)?;
        let mut staging=Staging::new(&state.cloud_backup,"restore.ainovel")?;
        WebDavService::new()?.download(&remote,&request.cloud_book_id,&request.generation_id,&staging.archive,&operation.cancel).await?;
        staging.record_archive()?;
        let archive=staging.archive.clone();let flag=operation.cancel.clone();let created=now();
        let receipt=tauri::async_runtime::spawn_blocking(move||{
            crate::services::portable_project_restore_install::restore_portable_project(
                &crate::services::portable_project_restore_install::RestorePortableProjectInput{
                    archive_path:&archive,target_project_root:&target,created_at:&created,target_project_id:None,cancel:Some(&flag),limits:None,
                }
            )
        }).await.map_err(|_|"CLOUD_BACKUP_RESTORE_FAILED".to_string())?
            .map_err(|e|if e=="PORTABLE_RESTORE_CANCELLED"{"CLOUD_BACKUP_CANCELLED"}else{"CLOUD_BACKUP_RESTORE_FAILED"}.to_string())?;
        let parents=vec![request.generation_id.clone()];
        let binding=(||{
            account(&state.cloud_backup,&request.local_endpoint_account_id)?;
            state.cloud_backup.local()?.bindings.save_origin_readonly(BindingInput{
                local_project_id:&receipt.target_project_id,cloud_book_id:&request.cloud_book_id,
                account_id:&request.local_endpoint_account_id,parents:&parents,
            })
        })();
        let recent=register(&receipt).is_ok();
        Ok(json!({"success":true,"state":if binding.is_ok(){"restore-complete"}else{"binding-not-saved"},
            "operationId":operation.id,"receipt":receipt,"bindingSaved":binding.is_ok(),"binding":binding.ok(),
            "recentProjectUpdated":recent}))
    }.await;
    response(result, Some(&operation.id))
}

#[cfg(test)]
#[path = "cloud_backup_tests.rs"]
mod tests;

#[tauri::command]
pub async fn cloud_backup_connect(
    state: State<'_, AppState>,
    request: Value,
) -> Result<Value, String> {
    Ok(connect_inner(state.inner(), request).await)
}
#[tauri::command]
pub async fn cloud_backup_view(
    state: State<'_, AppState>,
    project_session: Value,
) -> Result<Value, String> {
    Ok(view_inner(state.inner(), project_session))
}
#[tauri::command]
pub async fn cloud_backup_confirm_binding(
    state: State<'_, AppState>,
    request: Value,
) -> Result<Value, String> {
    Ok(confirm_inner(state.inner(), request))
}
#[tauri::command]
pub async fn cloud_backup_backup(
    state: State<'_, AppState>,
    request: Value,
) -> Result<Value, String> {
    Ok(backup_inner(state.inner(), request).await)
}
#[tauri::command]
pub async fn cloud_backup_list(
    state: State<'_, AppState>,
    request: Value,
) -> Result<Value, String> {
    Ok(list_inner(state.inner(), request).await)
}
#[tauri::command]
pub async fn cloud_backup_restore_copy(
    state: State<'_, AppState>,
    request: Value,
) -> Result<Value, String> {
    Ok(restore_inner(state.inner(), request).await)
}
#[tauri::command]
pub async fn cloud_backup_cancel(
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<Value, String> {
    Ok(cancel_inner(state.inner(), &operation_id))
}
#[tauri::command]
pub async fn cloud_backup_clear_credential(
    state: State<'_, AppState>,
    local_endpoint_account_id: String,
) -> Result<Value, String> {
    Ok(clear_inner(state.inner(), &local_endpoint_account_id))
}
