//! Skin 频道的 Tauri 命令实现
//!
//! 对应 `src/shared/ipc-channels.ts` 的 `SkinChannels`：
//! - `skin:get-state` → `skin_get_state`
//! - `skin:execute` → `skin_execute`
//! - `skin:read-custom-asset` → `skin_read_custom_asset`
//!
//! **B7 启动态子集（2026-10-15）**：activeSkin 的唯一权威改为全局外观门禁状态
//! （`services/startup_gate.rs` 的 `appearance.json`，与 `startup:skin-snapshot` 同源），
//! 因此 `activate` 现在会**持久化**皮肤并前移皮肤修订，进程重启后仍生效。
//! 自定义皮肤资产（导入 / 读取）仍是进程内存占位实现：`import-custom` 尚未接
//! 文件选择与压缩（见下方 TODO），不会改变 activeSkin。

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::services::startup_gate::{SkinStateWriteError, DEFAULT_BACKGROUND_SKIN, SKIN_IDS};
use crate::state::AppState;

/// 皮肤状态
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinState {
    pub active_skin: String,
    pub custom_skin: Option<CustomSkin>,
}

/// 自定义皮肤信息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomSkin {
    pub mime: String,
    pub revision: String,
    pub width: u32,
    pub height: u32,
}

/// 皮肤执行命令（tag 对齐契约字面量：activate / import-custom / remove-custom）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SkinCommand {
    #[serde(rename_all = "camelCase")]
    Activate {
        skin_id: String,
    },
    ImportCustom,
    RemoveCustom,
}

/// 皮肤执行响应
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SkinExecuteResponse {
    Success {
        success: bool,
        state: SkinState,
        cancelled: bool,
    },
    Failure {
        success: bool,
        state: SkinState,
        error: SkinError,
    },
}

/// 皮肤错误
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkinError {
    pub code: String,
    pub message: String,
}

/// 自定义皮肤的命令存储（进程内存态）。
///
/// activeSkin 不在此处：它由全局外观门禁状态持有并持久化，查询时按需组合，
/// 避免「内存一份、磁盘一份」的双写漂移。
#[derive(Debug, Clone, Default)]
pub struct SkinCommandStore {
    custom_skin: Option<CustomSkin>,
}

impl SkinCommandStore {
    pub(crate) fn new() -> Self {
        SkinCommandStore { custom_skin: None }
    }

    /// 以门禁给出的 activeSkin 组装契约状态。
    fn state_with(&self, active_skin: &str) -> SkinState {
        SkinState {
            active_skin: active_skin.to_string(),
            custom_skin: self.custom_skin.clone(),
        }
    }

    fn apply_custom_command(&mut self, command: &SkinCommand) -> Option<SkinError> {
        match command {
            SkinCommand::RemoveCustom => {
                self.custom_skin = None;
                None
            }
            SkinCommand::ImportCustom => {
                // TODO: 实际导入逻辑（文件选择 + 压缩 + 落盘）尚未迁移；
                // 这里刻意不报告「导入成功」，也不改变 activeSkin。
                Some(SkinError {
                    code: "CUSTOM_SKIN_UNAVAILABLE".to_string(),
                    message: "自定义皮肤导入尚未迁移".to_string(),
                })
            }
            SkinCommand::Activate { .. } => None,
        }
    }

    fn read_custom_asset(&self) -> Result<SkinAsset, SkinError> {
        match &self.custom_skin {
            Some(custom) => Ok(SkinAsset {
                mime: custom.mime.clone(),
                revision: custom.revision.clone(),
                bytes: vec![],
            }),
            None => Err(SkinError {
                code: "CUSTOM_ASSET_UNAVAILABLE".to_string(),
                message: "未找到自定义皮肤资产".to_string(),
            }),
        }
    }
}

/// 皮肤资产（用于读取自定义皮肤）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkinAsset {
    pub mime: String,
    pub revision: String,
    pub bytes: Vec<u8>,
}

/// Skin:read-custom-asset 的响应（对齐契约 SkinReadCustomAssetResponse 判别联合；
/// 失败作为成功响应返回，而不是 invoke reject）
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum SkinReadCustomAssetResponse {
    /// 成功：展开 SkinAsset 字段并附 success: true
    Success {
        success: bool,
        mime: String,
        revision: String,
        bytes: Vec<u8>,
    },
    /// 失败：附当前状态与错误
    Failure {
        success: bool,
        state: SkinState,
        error: SkinError,
    },
}

/// 皮肤状态写入失败 → 契约错误码（对齐 `SkinErrorCode`）。
fn gate_error(error: SkinStateWriteError) -> SkinError {
    match error {
        SkinStateWriteError::InvalidSkinId => SkinError {
            code: "INVALID_COMMAND".to_string(),
            message: format!("皮肤标识非法（可用：{}）", SKIN_IDS.join(" / ")),
        },
        SkinStateWriteError::NotPrepared => SkinError {
            code: "SKIN_SERVICE_UNAVAILABLE".to_string(),
            message: "全局外观门禁尚未装载".to_string(),
        },
        SkinStateWriteError::StorageFailed(detail) => SkinError {
            code: "SKIN_STORAGE_FAILED".to_string(),
            message: detail,
        },
    }
}

/// 当前 activeSkin：门禁未装载（或阻塞）时退化为发布默认皮肤，
/// 与渲染层 `RELEASE_DEFAULT_SHELL` 的读法一致（不制造第二份状态）。
fn active_background_skin(state: &AppState) -> String {
    state
        .startup_gate
        .lock()
        .unwrap()
        .background_skin()
        .unwrap_or(DEFAULT_BACKGROUND_SKIN)
        .to_string()
}

/// 皮肤命令核心（不依赖 Tauri `State`，便于单测）。
///
/// 锁顺序固定为「门禁 → 自定义皮肤存储」，且两者不嵌套持有。
pub(crate) fn execute_skin_command(
    store: &std::sync::Mutex<SkinCommandStore>,
    gate: &std::sync::Mutex<crate::services::startup_gate::StartupGate>,
    command: SkinCommand,
) -> SkinExecuteResponse {
    let response_state =
        |store: &std::sync::Mutex<SkinCommandStore>,
         gate: &std::sync::Mutex<crate::services::startup_gate::StartupGate>| {
            let active = gate
                .lock()
                .unwrap()
                .background_skin()
                .unwrap_or(DEFAULT_BACKGROUND_SKIN)
                .to_string();
            store.lock().unwrap().state_with(&active)
        };
    match command {
        SkinCommand::Activate { skin_id } => {
            // 先绑定结果再匹配：`match` 直接以守卫为 scrutinee 时，临时锁会活到整个 match，
            // 分支内再取同一锁即自死锁（本项目 2026-10-15 实测）。
            let outcome = gate.lock().unwrap().set_background_skin(&skin_id);
            match outcome {
                Ok(_) => SkinExecuteResponse::Success {
                    success: true,
                    state: response_state(store, gate),
                    cancelled: false,
                },
                Err(error) => SkinExecuteResponse::Failure {
                    success: false,
                    state: response_state(store, gate),
                    error: gate_error(error),
                },
            }
        }
        other => {
            let custom_error = store.lock().unwrap().apply_custom_command(&other);
            match custom_error {
                None => SkinExecuteResponse::Success {
                    success: true,
                    state: response_state(store, gate),
                    cancelled: false,
                },
                Some(error) => SkinExecuteResponse::Failure {
                    success: false,
                    state: response_state(store, gate),
                    error,
                },
            }
        }
    }
}

/// Skin:get-state 命令
#[tauri::command]
pub fn skin_get_state(state: State<'_, crate::state::AppState>) -> SkinState {
    let active = active_background_skin(&state);
    state.skin.lock().unwrap().state_with(&active)
}

/// Skin:execute 命令
#[tauri::command]
pub fn skin_execute(
    state: State<'_, crate::state::AppState>,
    command: SkinCommand,
) -> SkinExecuteResponse {
    execute_skin_command(&state.skin, &state.startup_gate, command)
}

/// Skin:read-custom-asset 命令（失败走契约判别联合而非 invoke reject）
#[tauri::command]
pub fn skin_read_custom_asset(
    state: State<'_, crate::state::AppState>,
) -> SkinReadCustomAssetResponse {
    let active = active_background_skin(&state);
    let guard = state.skin.lock().unwrap();
    match guard.read_custom_asset() {
        Ok(asset) => SkinReadCustomAssetResponse::Success {
            success: true,
            mime: asset.mime,
            revision: asset.revision,
            bytes: asset.bytes,
        },
        Err(error) => SkinReadCustomAssetResponse::Failure {
            success: false,
            state: guard.state_with(&active),
            error,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::startup_gate::StartupGate;
    use std::path::PathBuf;
    use std::sync::Mutex;

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lorekeeper-skin-{}-{}",
            tag,
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn prepared_gate(tag: &str) -> (PathBuf, Mutex<StartupGate>) {
        let home = temp_home(tag);
        let mut gate = StartupGate::new();
        gate.prepare_at(&home);
        (home, Mutex::new(gate))
    }

    #[test]
    fn skin_state_initializes_correctly() {
        let store = SkinCommandStore::new();
        let state = store.state_with("classic");
        assert_eq!(state.active_skin, "classic");
        assert!(state.custom_skin.is_none());
    }

    #[test]
    fn skin_command_deserializes_kebab_tags_from_frontend() {
        // 前端传 `{ type: 'import-custom' }`（kebab-case tag），Rust 侧必须可反序列化
        let cmd: SkinCommand = serde_json::from_str(r#"{"type":"import-custom"}"#).unwrap();
        assert!(matches!(cmd, SkinCommand::ImportCustom));
        let cmd: SkinCommand =
            serde_json::from_str(r#"{"type":"activate","skinId":"anime"}"#).unwrap();
        assert!(matches!(cmd, SkinCommand::Activate { .. }));
        let cmd: SkinCommand = serde_json::from_str(r#"{"type":"remove-custom"}"#).unwrap();
        assert!(matches!(cmd, SkinCommand::RemoveCustom));
    }

    #[test]
    fn resolve_close_decision_accepts_lowercase_literals() {
        let d: crate::commands::window::ResolveCloseDecision =
            serde_json::from_str(r#""proceed""#).unwrap();
        assert!(matches!(
            d,
            crate::commands::window::ResolveCloseDecision::Proceed
        ));
    }

    #[test]
    fn skin_execute_activate_persists_and_bumps_revision_test() {
        let (home, gate) = prepared_gate("activate");
        let store = Mutex::new(SkinCommandStore::new());
        let response = execute_skin_command(
            &store,
            &gate,
            SkinCommand::Activate {
                skin_id: "anime".to_string(),
            },
        );
        match response {
            SkinExecuteResponse::Success {
                success: true,
                state,
                cancelled: false,
            } => assert_eq!(state.active_skin, "anime"),
            other => panic!("期望成功响应：{other:?}"),
        }
        assert_eq!(gate.lock().unwrap().background_skin(), Some("anime"));
        assert_eq!(gate.lock().unwrap().skin_revision(), Some(1));

        // 重新装载：皮肤已持久化（重启后仍生效）
        let mut reloaded = StartupGate::new();
        reloaded.prepare_at(&home);
        assert_eq!(reloaded.background_skin(), Some("anime"));
        assert_eq!(reloaded.skin_revision(), Some(1));

        // 非法皮肤：契约拒绝，状态与修订不动
        let rejected = execute_skin_command(
            &store,
            &gate,
            SkinCommand::Activate {
                skin_id: "neon".to_string(),
            },
        );
        match rejected {
            SkinExecuteResponse::Failure {
                success: false,
                state,
                error,
            } => {
                assert_eq!(error.code, "INVALID_COMMAND");
                assert_eq!(state.active_skin, "anime");
            }
            other => panic!("期望失败响应：{other:?}"),
        }
        assert_eq!(gate.lock().unwrap().skin_revision(), Some(1));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn skin_execute_without_prepared_gate_reports_service_unavailable_test() {
        let store = Mutex::new(SkinCommandStore::new());
        let gate = Mutex::new(StartupGate::new());
        let response = execute_skin_command(
            &store,
            &gate,
            SkinCommand::Activate {
                skin_id: "classic".to_string(),
            },
        );
        match response {
            SkinExecuteResponse::Failure { error, state, .. } => {
                assert_eq!(error.code, "SKIN_SERVICE_UNAVAILABLE");
                assert_eq!(state.active_skin, "classic");
            }
            other => panic!("期望失败响应：{other:?}"),
        }
    }

    #[test]
    fn skin_execute_remove_custom_succeeds() {
        let (home, gate) = prepared_gate("remove-custom");
        let store = Mutex::new(SkinCommandStore::new());
        store.lock().unwrap().custom_skin = Some(CustomSkin {
            mime: "image/png".to_string(),
            revision: "1.0".to_string(),
            width: 800,
            height: 600,
        });
        let command = SkinCommand::RemoveCustom;
        let response = execute_skin_command(&store, &gate, command);
        match response {
            SkinExecuteResponse::Success {
                success: true,
                state,
                ..
            } => {
                assert!(state.custom_skin.is_none());
            }
            other => panic!("期望成功响应：{other:?}"),
        }
        assert!(store.lock().unwrap().custom_skin.is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn skin_execute_import_custom_reports_unavailable_without_fake_success_test() {
        let (home, gate) = prepared_gate("import-custom");
        let store = Mutex::new(SkinCommandStore::new());
        let response = execute_skin_command(&store, &gate, SkinCommand::ImportCustom);
        match response {
            SkinExecuteResponse::Failure {
                success: false,
                state,
                error,
            } => {
                assert_eq!(error.code, "CUSTOM_SKIN_UNAVAILABLE");
                assert_eq!(state.active_skin, "classic");
            }
            other => panic!("导入占位不得报告成功：{other:?}"),
        }
        assert_eq!(gate.lock().unwrap().skin_revision(), Some(0));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn skin_read_custom_asset_reports_unavailable_without_custom_skin_test() {
        let store = SkinCommandStore::new();
        let error = store.read_custom_asset().unwrap_err();
        assert_eq!(error.code, "CUSTOM_ASSET_UNAVAILABLE");
    }
}
