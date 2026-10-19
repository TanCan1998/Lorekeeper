//! 角色头像命令（`character-avatar:*` 4 频道）—— 平移自
//! `electron/controllers/character-avatar-controller.ts`。
//!
//! | 契约频道 | 命令 |
//! |---|---|
//! | `character-avatar:choose` | [`character_avatar_choose`] |
//! | `character-avatar:commit` | [`character_avatar_commit`] |
//! | `character-avatar:read-batch` | [`character_avatar_read_batch`] |
//! | `character-avatar:remove` | [`character_avatar_remove`] |
//!
//! 项目域频道（契约不写 `projectSession`，由渲染层自动注入尾参）：命令先校验会话租约与
//! 项目数据根，再进入服务层；失败按契约返回判别联合，而非 invoke reject。
//!
//! 与基线的必然差异：
//! - 无 `webContents` / 多窗口语义：基线 `validSender` 的等价物是「主窗口存在」；
//! - 读面缓存按项目、租约、数据根与资产修订隔离；成功修改后失效，最多 512 项；
//! - 基线把 `recordReclaimableOrphans` 挂在 `resolveService` 上（每次请求惰性执行），
//!   此处同样在每次进入服务层前执行，失败按 `AVATAR_SAVE_FAILED` 上报；
//! - `choose` 需要「校验 → 弹窗 → 预览」两段，`with_project_db` 是同步闭包，
//!   故两次取连接（中间不持有数据库锁）。

pub(crate) mod cache;

use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::commands::project::file_path_to_string;
use crate::security::{assert_current_project_context, ProjectSessionContext};
use crate::services::character_asset_service as avatar;
use crate::state::AppState;

/// 主窗口标签，与 `tauri.conf.json` 的 `app.windows[0].label` 一致。
const MAIN_WINDOW_LABEL: &str = "main";
/// 对话框等待上限，仅作安全网（与 `commands/kb.rs`、`commands/import.rs` 一致）
const DIALOG_TIMEOUT: Duration = Duration::from_secs(600);

/// 契约 `CharacterAvatarError`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterAvatarError {
    pub code: String,
    pub message: String,
}

/// 契约 `CharacterAvatarChooseResponse`
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterAvatarChooseResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancelled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<avatar::CharacterAvatarPreview>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CharacterAvatarError>,
}

/// 契约 `CharacterAvatarCommitResponse`
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterAvatarCommitResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<avatar::CharacterAvatarView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CharacterAvatarError>,
}

/// 契约 `CharacterAvatarBatchReadResponse`
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterAvatarBatchReadResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatars: Option<Vec<avatar::CharacterAvatarView>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CharacterAvatarError>,
}

/// 契约 `CharacterAvatarRemoveResponse`
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterAvatarRemoveResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CharacterAvatarError>,
}

/// 错误码与本地化分离；不把内部路径或 IO 细节带到渲染层。
fn failure_copy(code: &str) -> (&'static str, &'static str) {
    match code {
        "INVALID_SENDER" => (
            "当前窗口无权访问角色头像。",
            "This window cannot access character avatars.",
        ),
        "INVALID_CHARACTER_ID" => (
            "角色身份无效，头像未修改。",
            "The character identity is invalid.",
        ),
        "PROJECT_NOT_OPEN" => ("尚未打开项目，无法访问头像。", "No project is open."),
        "CHARACTER_NOT_FOUND" => (
            "该角色已不存在，头像未修改。",
            "That character no longer exists.",
        ),
        "IMAGE_READ_FAILED" => (
            "无法读取所选图片，请重新选择。",
            "The selected image could not be read.",
        ),
        "IMAGE_FORMAT_INVALID" => (
            "请选择有效的 PNG、JPEG 或 WebP 图片。",
            "Choose a valid PNG, JPEG, or WebP image.",
        ),
        "IMAGE_TOO_LARGE" => (
            "图片超过 4 MB，请换一张更小的。",
            "The image is larger than 4 MB.",
        ),
        _ => (
            "头像保存失败，原头像保持不变。",
            "The avatar could not be saved; the previous avatar is unchanged.",
        ),
    }
}

fn failure(code: &str) -> CharacterAvatarError {
    let (zh, en) = failure_copy(code);
    CharacterAvatarError {
        code: code.to_string(),
        message: crate::commands::config::main_text_with_system_locale(zh, en),
    }
}

/// 基线 `errorCode`：只放行五个领域码，其余一律折叠为 `AVATAR_SAVE_FAILED`
fn error_code(error: &str) -> &'static str {
    match error {
        "INVALID_CHARACTER_ID" => "INVALID_CHARACTER_ID",
        "CHARACTER_NOT_FOUND" => "CHARACTER_NOT_FOUND",
        "IMAGE_READ_FAILED" => "IMAGE_READ_FAILED",
        "IMAGE_FORMAT_INVALID" => "IMAGE_FORMAT_INVALID",
        "IMAGE_TOO_LARGE" => "IMAGE_TOO_LARGE",
        _ => "AVATAR_SAVE_FAILED",
    }
}

/// 基线 `validSender` 的等价物：主窗口存在（Tauri 单窗口语义）
fn valid_sender(app: &tauri::AppHandle) -> bool {
    app.get_webview_window(MAIN_WINDOW_LABEL).is_some()
}

/// 解析项目数据根（`<项目根>/.lore`）。
///
/// 对应基线 `resolveService`：会话租约必须与活跃项目一致，且数据根必须已存在；
/// 任一条件不满足即视为「项目未打开」（基线返回 null → `PROJECT_NOT_OPEN`）。
fn resolve_data_root(
    state: &AppState,
    session: Option<&ProjectSessionContext>,
) -> Result<std::path::PathBuf, String> {
    let Some(context) = session else {
        return Err("PROJECT_NOT_OPEN".to_string());
    };
    let active = state.active_project_snapshot();
    assert_current_project_context(context, active.as_ref())
        .map_err(|_| "PROJECT_NOT_OPEN".to_string())?;
    let Some(active) = active else {
        return Err("PROJECT_NOT_OPEN".to_string());
    };
    let data_root = std::path::PathBuf::from(&active.root_path).join(crate::db::PROJECT_DIR_NAME);
    if !data_root.is_dir() {
        return Err("PROJECT_NOT_OPEN".to_string());
    }
    Ok(data_root)
}

/// `character-avatar:choose`
#[tauri::command]
pub async fn character_avatar_choose(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    character_id: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<CharacterAvatarChooseResponse, String> {
    if !valid_sender(&app) {
        return Ok(CharacterAvatarChooseResponse {
            success: false,
            cancelled: None,
            image: None,
            error: Some(failure("INVALID_SENDER")),
        });
    }
    let state = state.inner();
    let data_root = match resolve_data_root(state, project_session.as_ref()) {
        Ok(root) => root,
        Err(code) => {
            return Ok(CharacterAvatarChooseResponse {
                success: false,
                cancelled: None,
                image: None,
                error: Some(failure(&code)),
            })
        }
    };
    // 基线顺序：先做惰性清理与角色存在性判定，再打开系统文件对话框
    let has_character = match state.with_project_db(|conn| {
        avatar::record_reclaimable_orphans(conn, &data_root)?;
        avatar::has_character(conn, &character_id)
    }) {
        Ok(has_character) => has_character,
        Err(error) => {
            return Ok(CharacterAvatarChooseResponse {
                success: false,
                cancelled: None,
                image: None,
                error: Some(failure(error_code(&error))),
            })
        }
    };
    if !has_character {
        return Ok(CharacterAvatarChooseResponse {
            success: false,
            cancelled: None,
            image: None,
            error: Some(failure("CHARACTER_NOT_FOUND")),
        });
    }

    let (sender, receiver) = std::sync::mpsc::channel();
    let mut builder = app
        .dialog()
        .file()
        .set_title(crate::commands::config::main_text_with_system_locale(
            "选择角色头像",
            "Choose a character avatar",
        ))
        .add_filter("PNG / JPEG / WebP", &["png", "jpg", "jpeg", "webp"]);
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
    let Some(path) = picked.and_then(file_path_to_string) else {
        return Ok(CharacterAvatarChooseResponse {
            success: true,
            cancelled: Some(true),
            image: None,
            error: None,
        });
    };
    match state.with_project_db(|conn| avatar::preview_file(conn, &character_id, Path::new(&path)))
    {
        Ok(image) => Ok(CharacterAvatarChooseResponse {
            success: true,
            cancelled: Some(false),
            image: Some(image),
            error: None,
        }),
        Err(error) => Ok(CharacterAvatarChooseResponse {
            success: false,
            cancelled: None,
            image: None,
            error: Some(failure(error_code(&error))),
        }),
    }
}

/// `character-avatar:commit`
#[tauri::command]
pub fn character_avatar_commit(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    character_id: String,
    base64: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<CharacterAvatarCommitResponse, String> {
    let state = state.inner();
    let fail = |code: &str| CharacterAvatarCommitResponse {
        success: false,
        avatar: None,
        error: Some(failure(code)),
    };
    if !valid_sender(&app) {
        return Ok(fail("INVALID_SENDER"));
    }
    let data_root = match resolve_data_root(state, project_session.as_ref()) {
        Ok(root) => root,
        Err(code) => return Ok(fail(&code)),
    };
    match state.with_project_db(|conn| {
        avatar::record_reclaimable_orphans(conn, &data_root)?;
        let image = avatar::commit(conn, &data_root, &character_id, &base64)?;
        let mut cache = state
            .avatar_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        cache.invalidate(&character_id);
        Ok(image)
    }) {
        Ok(avatar) => Ok(CharacterAvatarCommitResponse {
            success: true,
            avatar: Some(avatar),
            error: None,
        }),
        Err(error) => Ok(fail(error_code(&error))),
    }
}

/// `character-avatar:read-batch`
#[tauri::command]
pub fn character_avatar_read_batch(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    character_ids: Vec<String>,
    project_session: Option<ProjectSessionContext>,
) -> Result<CharacterAvatarBatchReadResponse, String> {
    let state = state.inner();
    let fail = |code: &str| CharacterAvatarBatchReadResponse {
        success: false,
        avatars: None,
        error: Some(failure(code)),
    };
    if !valid_sender(&app) {
        return Ok(fail("INVALID_SENDER"));
    }
    let data_root = match resolve_data_root(state, project_session.as_ref()) {
        Ok(root) => root,
        Err(code) => return Ok(fail(&code)),
    };
    match state.with_project_db(|conn| {
        avatar::record_reclaimable_orphans(conn, &data_root)?;
        let context = project_session
            .as_ref()
            .ok_or_else(|| "PROJECT_NOT_OPEN".to_string())?;
        assert_current_project_context(context, state.active_project_snapshot().as_ref())
            .map_err(|_| "PROJECT_NOT_OPEN".to_string())?;
        let mut cache = state
            .avatar_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        cache.enter_scope(context, &data_root);
        cache.read_many(conn, &data_root, &character_ids)
    }) {
        Ok(avatars) => Ok(CharacterAvatarBatchReadResponse {
            success: true,
            avatars: Some(avatars),
            error: None,
        }),
        Err(error) => Ok(fail(error_code(&error))),
    }
}

/// `character-avatar:remove`
#[tauri::command]
pub fn character_avatar_remove(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    character_id: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<CharacterAvatarRemoveResponse, String> {
    let state = state.inner();
    let fail = |code: &str| CharacterAvatarRemoveResponse {
        success: false,
        error: Some(failure(code)),
    };
    if !valid_sender(&app) {
        return Ok(fail("INVALID_SENDER"));
    }
    let data_root = match resolve_data_root(state, project_session.as_ref()) {
        Ok(root) => root,
        Err(code) => return Ok(fail(&code)),
    };
    match state.with_project_db(|conn| {
        avatar::record_reclaimable_orphans(conn, &data_root)?;
        avatar::remove(conn, &data_root, &character_id)?;
        state
            .avatar_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .invalidate(&character_id);
        Ok(())
    }) {
        Ok(()) => Ok(CharacterAvatarRemoveResponse {
            success: true,
            error: None,
        }),
        Err(error) => Ok(fail(error_code(&error))),
    }
}
