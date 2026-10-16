//! 便携项目归档的对话框频道（`dialog:select-project-archive*`，3 频道）。
//!
//! | 契约频道 | 命令 | 迁移自 |
//! |---|---|---|
//! | `dialog:select-project-archive-export` | [`dialog_select_project_archive_export`] | `electron/controllers/project-archive-controller.ts:142` |
//! | `dialog:select-project-archive` | [`dialog_select_project_archive`] | 同上 `:163` |
//! | `dialog:select-project-restore-target` | [`dialog_select_project_restore_target`] | 同上 `:181` |
//!
//! 三个频道只做「用户显式选择 → 签发不透明授权」：返回值仅含 `grantId` 与展示名，绝对路径
//! 只留在进程内授权注册表（ADR 0002）。消费这些授权的 `project:archive-export` /
//! `project:archive-restore`（便携归档 ZIP 方案待确认）尚未迁移，届时经
//! [`ExternalGrantRegistry::resolve_exact_path`](crate::external_grant::ExternalGrantRegistry::resolve_exact_path)
//! 取路径。
//!
//! 与基线的两点必然调整（语义等价，与 `dialog:select-folder` 同因）：
//! 1. 命令为 `async` + `spawn_blocking` 等待：非 async 命令在主线程上会与对话框消息循环死锁；
//! 2. 显式 `set_parent`：无边框主窗口会遮挡无父窗口的对话框。
//!
//! `dialog:select-*` 属能力域频道，不接收 `projectSession` 尾参（见 `ipc-channels.ts`）。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::commands::project::file_path_to_string;
use crate::external_grant::GrantOperation;
use crate::state::AppState;

/// 主窗口标签，与 `tauri.conf.json` 的 `app.windows[0].label` 一致。
const MAIN_WINDOW_LABEL: &str = "main";
/// 便携存档扩展名（对齐基线 `ARCHIVE_EXTENSION = 'ainovel'`）
const ARCHIVE_EXTENSION: &str = "ainovel";
/// 选择器签发的一次性授权 TTL（对齐基线 `ARCHIVE_GRANT_TTL_MS = 10 分钟`）
const ARCHIVE_GRANT_TTL: Duration = Duration::from_secs(10 * 60);
/// 授权使用次数（对齐基线 `maxUses: 1`）
const ARCHIVE_GRANT_MAX_USES: u32 = 1;
/// 对话框等待上限，仅作安全网（与 `commands/kb.rs`、`commands/import.rs` 一致）
const DIALOG_TIMEOUT: Duration = Duration::from_secs(600);
/// 授权注册表锁中毒文案（与 `commands/project.rs` 一致）
const GRANT_REGISTRY_POISONED: &str = "外部文件授权状态被污染";
/// 导出目标已存在的报错文案（基线逐字）
const ARCHIVE_TARGET_EXISTS_MESSAGE: &str =
    "目标文件已存在，请选择其他文件名。项目存档不会覆盖已有文件。";
/// 恢复副本重名到上限时的错误码（对齐基线 `PORTABLE_RESTORE_TARGET_EXISTS`）
const PORTABLE_RESTORE_TARGET_EXISTS: &str = "PORTABLE_RESTORE_TARGET_EXISTS";
/// 恢复副本的兜底名（对齐基线 `'restored-project'`）
const RESTORE_TARGET_FALLBACK_NAME: &str = "restored-project";
/// 建议名（导出文件名 / 恢复副本名）的最大 UTF-16 码元数（对齐基线 `.slice(0, 80)`）
const SUGGESTED_NAME_MAX_UNITS: usize = 80;

/// 对齐契约 `ExternalFileGrant`：只携带展示名与不透明能力标识，绝对路径不经过 IPC。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableArchiveGrant {
    pub grant_id: String,
    pub display_name: String,
}

/// 建议名净化（对齐基线 `project-archive-controller.ts` 的 `safeSuggestedName`）。
///
/// 控制字符与 `<>:"/\|?*` 一律替换为 `-`，去掉结尾的点与空格，截断到 80 个 UTF-16 码元；
/// 结果为空时用 `fallback`。与基线的唯一差异：基线 `slice` 可能切出孤立代理码元，本实现
/// 按码元边界回退，不产生非法 UTF-8 序列。
fn safe_suggested_name(value: &str, fallback: &str) -> String {
    let mapped: String = value
        .trim()
        .chars()
        .map(|character| {
            if (character as u32) < 32 || "<>:\"/\\|?*".contains(character) {
                '-'
            } else {
                character
            }
        })
        .collect();
    let trimmed = mapped.trim_end_matches(['.', ' ']);
    let truncated = truncate_utf16_units(trimmed, SUGGESTED_NAME_MAX_UNITS);
    if truncated.is_empty() {
        fallback.to_string()
    } else {
        truncated
    }
}

/// 按 UTF-16 码元上限截断（对齐 JS `String.prototype.slice`）
fn truncate_utf16_units(value: &str, limit: usize) -> String {
    let mut units = 0usize;
    let mut end = 0usize;
    for (index, character) in value.char_indices() {
        let width = character.len_utf16();
        if units + width > limit {
            break;
        }
        units += width;
        end = index + character.len_utf8();
    }
    value[..end].to_string()
}

/// 恢复副本目标：父目录下不与既有文件/目录重名的名字（对齐基线 `availableRestoreTarget`）。
fn available_restore_target(parent: &Path, suggested_name: &str) -> Result<PathBuf, String> {
    let base = parent.join(safe_suggested_name(
        suggested_name,
        RESTORE_TARGET_FALLBACK_NAME,
    ));
    if !base.exists() {
        return Ok(base);
    }
    for suffix in 2..=999u32 {
        let candidate = PathBuf::from(format!("{}-{suffix}", base.to_string_lossy()));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(PORTABLE_RESTORE_TARGET_EXISTS.to_string())
}

/// `dialog:select-project-archive-export` —— 选择导出位置并签发 `create` 一次性授权。
///
/// 迁移自基线同频道：保存对话框预填 `<净化名>.ainovel`；取消返回 `null`；目标已存在则报错
/// （便携存档**不覆盖**既有文件）。签发的是「待创建子项」精确授权（父目录固定 + 文件名固定）。
#[tauri::command]
pub async fn dialog_select_project_archive_export(
    app: tauri::AppHandle,
    suggested_name: String,
) -> Result<Option<PortableArchiveGrant>, String> {
    let file_name = format!(
        "{}.{ARCHIVE_EXTENSION}",
        safe_suggested_name(&suggested_name, "project-backup")
    );
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut builder = app
        .dialog()
        .file()
        .set_title("导出项目存档")
        .set_file_name(file_name)
        .add_filter("AI Novel Archive", &[ARCHIVE_EXTENSION]);
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        builder = builder.set_parent(&window);
    }
    builder.save_file(move |selection| {
        let _ = sender.send(selection);
    });
    let picked = tauri::async_runtime::spawn_blocking(move || {
        receiver.recv_timeout(DIALOG_TIMEOUT).ok().flatten()
    })
    .await
    .ok()
    .flatten();
    let Some(target) = picked.and_then(file_path_to_string) else {
        return Ok(None);
    };
    let target_path = Path::new(&target);
    if target_path.exists() {
        return Err(ARCHIVE_TARGET_EXISTS_MESSAGE.to_string());
    }
    let Some(parent) = target_path.parent() else {
        return Ok(None);
    };
    let Some(name) = target_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
    else {
        return Ok(None);
    };
    let display_name = name.clone();
    let state = app.state::<AppState>();
    let mut registry = state
        .external_grants
        .lock()
        .map_err(|_| GRANT_REGISTRY_POISONED.to_string())?;
    let grant_id = registry.issue_new_child(
        parent,
        &name,
        vec![GrantOperation::Create],
        ARCHIVE_GRANT_TTL,
        Some(ARCHIVE_GRANT_MAX_USES),
    )?;
    Ok(Some(PortableArchiveGrant {
        grant_id,
        display_name,
    }))
}

/// `dialog:select-project-archive` —— 选择既有便携存档并签发 `read` 一次性授权。
#[tauri::command]
pub async fn dialog_select_project_archive(
    app: tauri::AppHandle,
) -> Result<Option<PortableArchiveGrant>, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut builder = app
        .dialog()
        .file()
        .set_title("选择项目存档")
        .add_filter("AI Novel Archive", &[ARCHIVE_EXTENSION]);
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        builder = builder.set_parent(&window);
    }
    builder.pick_file(move |selection| {
        let _ = sender.send(selection);
    });
    let picked = tauri::async_runtime::spawn_blocking(move || {
        receiver.recv_timeout(DIALOG_TIMEOUT).ok().flatten()
    })
    .await
    .ok()
    .flatten();
    let Some(archive) = picked.and_then(file_path_to_string) else {
        return Ok(None);
    };
    let display_name = Path::new(&archive)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| archive.clone());
    let state = app.state::<AppState>();
    let mut registry = state
        .external_grants
        .lock()
        .map_err(|_| GRANT_REGISTRY_POISONED.to_string())?;
    let grant_id = registry.issue_file(
        Path::new(&archive),
        ARCHIVE_GRANT_TTL,
        Some(ARCHIVE_GRANT_MAX_USES),
    )?;
    Ok(Some(PortableArchiveGrant {
        grant_id,
        display_name,
    }))
}

/// `dialog:select-project-restore-target` —— 选择恢复副本父目录并签发 `create` 一次性授权。
///
/// 迁移自基线同频道：目录选择器选出父目录后，按 `<净化建议名>`（重名则 `-2`…`-999`）算出
/// 不冲突的副本名，再签发「待创建子项」精确授权；`displayName` 是**副本名**而非父目录名。
#[tauri::command]
pub async fn dialog_select_project_restore_target(
    app: tauri::AppHandle,
    suggested_name: String,
) -> Result<Option<PortableArchiveGrant>, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut builder = app.dialog().file().set_title("选择恢复副本所在文件夹");
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        builder = builder.set_parent(&window);
    }
    builder.pick_folder(move |selection| {
        let _ = sender.send(selection);
    });
    let picked = tauri::async_runtime::spawn_blocking(move || {
        receiver.recv_timeout(DIALOG_TIMEOUT).ok().flatten()
    })
    .await
    .ok()
    .flatten();
    let Some(parent) = picked.and_then(file_path_to_string) else {
        return Ok(None);
    };
    let target = available_restore_target(Path::new(&parent), &suggested_name)?;
    let Some(name) = target
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
    else {
        return Ok(None);
    };
    let display_name = name.clone();
    let state = app.state::<AppState>();
    let mut registry = state
        .external_grants
        .lock()
        .map_err(|_| GRANT_REGISTRY_POISONED.to_string())?;
    let grant_id = registry.issue_new_child(
        Path::new(&parent),
        &name,
        vec![GrantOperation::Create],
        ARCHIVE_GRANT_TTL,
        Some(ARCHIVE_GRANT_MAX_USES),
    )?;
    Ok(Some(PortableArchiveGrant {
        grant_id,
        display_name,
    }))
}

// ===== project:archive-export =====

/// 便携归档错误码全集（对齐基线 `PORTABLE_ERROR_CODES`）：
/// 只有集合内的码才进 `errorCode`，其余折叠为仅带 `error` 文本的失败信封。
const PORTABLE_ERROR_CODES: &[&'static str] = &[
    "PORTABLE_ARCHIVE_INVALID",
    "PORTABLE_ARCHIVE_LIMIT_EXCEEDED",
    "PORTABLE_ARCHIVE_SOURCE_CHANGED",
    "PORTABLE_ARCHIVE_SOURCE_UNSAFE",
    "PORTABLE_ARCHIVE_PUBLISH_UNSUPPORTED",
    "PORTABLE_ARCHIVE_TARGET_EXISTS",
    "PORTABLE_ASSET_MISSING",
    "PORTABLE_ASSET_UNSAFE",
    "PORTABLE_CONTEXT_INVALID",
    "PORTABLE_GRANT_INVALID",
    "PORTABLE_SCHEMA_UNSUPPORTED",
    "PORTABLE_SOURCE_CHANGED",
    "PORTABLE_TARGET_INSIDE_SOURCE",
    "PORTABLE_UNSAFE_PROJECTION",
    "PORTABLE_RESTORE_CANCELLED",
    "PORTABLE_RESTORE_INVALID",
    "PORTABLE_RESTORE_TARGET_EXISTS",
    "PORTABLE_RESTORE_UNSAFE_TARGET",
];

/// 契约 `PortableProjectExportRequest`
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableProjectExportRequest {
    /// `dialog:select-project-archive-export` 签发的 `create` 授权（渲染进程不携带路径）
    pub target_archive_grant_id: String,
    pub project_session: crate::security::ProjectSessionContext,
}

/// 契约联合 `{ success: true; receipt } | PortableProjectOperationFailure`
#[derive(Debug, Clone, serde::Serialize)]
#[serde(untagged)]
pub enum PortableProjectExportResponse {
    Success {
        success: bool,
        receipt: crate::services::portable_project_export::ExportPortableProjectReceipt,
    },
    Failure {
        success: bool,
        error: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_code: Option<String>,
    },
}

/// 基线 `failure(error)`：错误码只在集合内时上报，其余折叠为纯文本失败。
fn portable_failure(error: String) -> PortableProjectExportResponse {
    let error_code = if PORTABLE_ERROR_CODES.contains(&error.as_str()) {
        Some(error)
    } else {
        None
    };
    PortableProjectExportResponse::Failure {
        success: false,
        error: error_code
            .clone()
            .unwrap_or_else(|| "便携归档操作失败".to_string()),
        error_code,
    }
}

/// 基线 `resolveGrantedPath(grants, event, grantId, operation)`：
/// 无授权 / 范围不符 / 过期 / 已消费一律折叠为 `PORTABLE_GRANT_INVALID`（不含路径）。
fn resolve_granted_path(
    state: &AppState,
    grant_id: &str,
    operation: GrantOperation,
) -> Result<std::path::PathBuf, String> {
    if grant_id.is_empty() {
        return Err("PORTABLE_GRANT_INVALID".to_string());
    }
    let mut registry = state
        .external_grants
        .lock()
        .map_err(|_| GRANT_REGISTRY_POISONED.to_string())?;
    registry
        .resolve_exact_path(grant_id, operation)
        .map_err(|_| "PORTABLE_GRANT_INVALID".to_string())
}

/// `project:archive-export` —— 把当前项目导出为便携归档（`.ainovel`）。
///
/// 迁移自 `electron/controllers/project-archive-controller.ts` 的同名频道：
/// 目标位置只以 `dialog:select-project-archive-export` 签发的一次性 `create` 授权表达，
/// 由主进程解析成路径（渲染进程从不携带路径）；会话租约在此完成校验。
#[tauri::command]
pub fn project_archive_export(
    state: State<'_, AppState>,
    request: PortableProjectExportRequest,
) -> Result<PortableProjectExportResponse, String> {
    let state = state.inner();
    let active = state.active_project_snapshot();
    if crate::security::assert_current_project_context(&request.project_session, active.as_ref())
        .is_err()
    {
        return Ok(portable_failure("PORTABLE_CONTEXT_INVALID".to_string()));
    }
    let Some(active) = active else {
        return Ok(portable_failure("PORTABLE_CONTEXT_INVALID".to_string()));
    };
    let target = match resolve_granted_path(
        state,
        &request.target_archive_grant_id,
        GrantOperation::Create,
    ) {
        Ok(path) => path,
        Err(error) => return Ok(portable_failure(error)),
    };
    let attempt_parent = match target.parent() {
        Some(parent) => parent.to_path_buf(),
        None => return Ok(portable_failure("PORTABLE_ARCHIVE_INVALID".to_string())),
    };
    let created_at = crate::commands::project::iso8601_utc_from_millis(
        crate::commands::project::epoch_millis_now(),
    );
    match crate::services::portable_project_export::export_portable_project(
        &crate::services::portable_project_export::ExportPortableProjectInput {
            source_project_root: std::path::Path::new(&active.root_path),
            origin_project_id: &active.project_id,
            target_archive_path: &target,
            attempt_parent_path: &attempt_parent,
            snapshot_generation: None,
            created_at: &created_at,
            limits: None,
        },
    ) {
        Ok(receipt) => Ok(PortableProjectExportResponse::Success {
            success: true,
            receipt,
        }),
        Err(error) => Ok(portable_failure(error)),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_grant::ExternalGrantRegistry;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lorekeeper-archive-{name}-{}",
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn safe_suggested_name_sanitizes_and_truncates_test() {
        assert_eq!(
            safe_suggested_name("  第一部小说  ", "fallback"),
            "第一部小说"
        );
        assert_eq!(safe_suggested_name("a<b>c:d", "fallback"), "a-b-c-d");
        assert_eq!(
            safe_suggested_name("风格|名字?*", "fallback"),
            "风格-名字--"
        );
        assert_eq!(safe_suggested_name("名字...", "fallback"), "名字");
        assert_eq!(safe_suggested_name("名字 . . ", "fallback"), "名字");
        assert_eq!(safe_suggested_name("\u{1}控制", "fallback"), "-控制");
        assert_eq!(safe_suggested_name("   ", "fallback"), "fallback");
        assert_eq!(safe_suggested_name("", "fallback"), "fallback");
        // 上限按 UTF-16 码元；代理对不被切断
        assert_eq!(
            safe_suggested_name(&"字".repeat(100), "fallback")
                .chars()
                .count(),
            80
        );
        let emoji_tail = format!("{}{}", "a".repeat(79), "😀".repeat(3));
        assert_eq!(safe_suggested_name(&emoji_tail, "fallback"), "a".repeat(79));
    }

    #[test]
    fn available_restore_target_appends_suffix_test() {
        let dir = temp_dir("restore-name");
        assert_eq!(
            available_restore_target(&dir, "副本").unwrap(),
            dir.join("副本")
        );
        std::fs::create_dir_all(dir.join("副本")).unwrap();
        assert_eq!(
            available_restore_target(&dir, "副本").unwrap(),
            dir.join("副本-2")
        );
        std::fs::create_dir_all(dir.join("副本-2")).unwrap();
        assert_eq!(
            available_restore_target(&dir, "副本").unwrap(),
            dir.join("副本-3")
        );
        // 建议名净化后为空 → 兜底名
        assert_eq!(
            available_restore_target(&dir, " ").unwrap(),
            dir.join(RESTORE_TARGET_FALLBACK_NAME)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_target_grant_freezes_selected_name_test() {
        let dir = temp_dir("restore-grant");
        let target = available_restore_target(&dir, "副本").unwrap();
        let name = target.file_name().unwrap().to_string_lossy().to_string();
        let mut registry = ExternalGrantRegistry::default();
        let grant_id = registry
            .issue_new_child(
                &dir,
                &name,
                vec![GrantOperation::Create],
                ARCHIVE_GRANT_TTL,
                Some(ARCHIVE_GRANT_MAX_USES),
            )
            .unwrap();
        let resolved = registry
            .resolve_exact_path(&grant_id, GrantOperation::Create)
            .unwrap();
        assert_eq!(resolved, std::fs::canonicalize(&dir).unwrap().join(&name));
        assert!(!resolved.exists(), "授权只固定目标名，不预先创建它");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn portable_failure_reports_only_contract_codes_test() {
        let known = portable_failure("PORTABLE_ARCHIVE_TARGET_EXISTS".to_string());
        match known {
            PortableProjectExportResponse::Failure {
                success,
                error,
                error_code,
            } => {
                assert!(!success);
                assert_eq!(error, "PORTABLE_ARCHIVE_TARGET_EXISTS");
                assert_eq!(
                    error_code.as_deref(),
                    Some("PORTABLE_ARCHIVE_TARGET_EXISTS")
                );
            }
            other => panic!("应为失败信封：{other:?}"),
        }
        // 集合外的内部码折叠为纯文本失败（不把内部码泄漏到契约面）
        match portable_failure("PORTABLE_MANIFEST_INVALID".to_string()) {
            PortableProjectExportResponse::Failure {
                error, error_code, ..
            } => {
                assert!(error_code.is_none(), "集合外错误码不得上报");
                assert_eq!(error, "便携归档操作失败");
            }
            other => panic!("应为失败信封：{other:?}"),
        }
    }

    #[test]
    fn resolve_granted_path_rejects_missing_grant_test() {
        // 空授权 id 一律折叠为 PORTABLE_GRANT_INVALID（不含路径）
        let state = AppState::new();
        assert_eq!(
            resolve_granted_path(&state, "", GrantOperation::Create).unwrap_err(),
            "PORTABLE_GRANT_INVALID"
        );
        assert_eq!(
            resolve_granted_path(&state, "no-such-grant", GrantOperation::Create).unwrap_err(),
            "PORTABLE_GRANT_INVALID"
        );
    }

    #[test]
    fn resolve_granted_path_consumes_create_grant_once_test() {
        let dir = temp_dir("export-grant");
        let target = dir.join("book.ainovel");
        let state = AppState::new();
        let grant_id = {
            let mut registry = state.external_grants.lock().unwrap();
            registry
                .issue_new_child(
                    &dir,
                    "book.ainovel",
                    vec![GrantOperation::Create],
                    ARCHIVE_GRANT_TTL,
                    Some(1),
                )
                .unwrap()
        };
        let resolved = resolve_granted_path(&state, &grant_id, GrantOperation::Create).unwrap();
        assert_eq!(
            resolved,
            std::fs::canonicalize(&dir).unwrap().join("book.ainovel"),
            "授权只解析出目标路径，不预先创建它"
        );
        assert!(!target.exists());
        // 一次性授权再取即失败（对齐基线 maxUses: 1 的消费语义）
        assert_eq!(
            resolve_granted_path(&state, &grant_id, GrantOperation::Create).unwrap_err(),
            "PORTABLE_GRANT_INVALID"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
