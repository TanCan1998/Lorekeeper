//! 启动门禁服务层（B7 启动态子集：`startup:*` 3 频道）。
//!
//! 平移自 `electron/controllers/startup-controller.ts` 的信任发送方 + 快照 + 读回确认三段语义，
//! 并把 `electron/main.ts` 的 `prepareGlobalRuntime()` 落成 [`StartupGate::prepare_at`]：
//! 全局数据根不可用、门禁状态文件不可读或版本不认识 → `GLOBAL_DATA_BLOCKED`；
//! 状态文件可读但皮肤取值非法 → `SKIN_NOT_READY`。
//!
//! **持久化位置（用户 2026-10-15 决策）**：外观门禁状态（生成号 + 皮肤修订 + 背景皮肤）
//! 落在 Lorekeeper 全局根 `AI_NOVEL_LOREKEEPER_HOME` / `~/.lorekeeper` 的 `appearance.json`；
//! 作者外观偏好档案仍由渲染层 `localStorage` 持有（与基线一致），主进程只记录
//! **本次窗口**的读回修订，不落盘——避免渲染层存储被清空或替换后把作者锁在启动门禁外。
//!
//! **刻意偏离**：
//! - 基线从旧数据根（`.vela`）迁移并可能报 `GLOBAL_MODEL_CREDENTIALS_DIFFER`；
//!   Tauri 两栈隔离（AGENTS.md），不读基线目录，故该阻塞码在此实现中永不产生；
//!   同理 `migrationNotice` 恒为 `None`（没有旧来源可忽略或保留）。
//! - 生成号取门禁状态文件的内容修订（FNV-1a 64 内容摘要），而非基线的主进程迁移令牌；
//!   它在握手期间稳定，仅在皮肤状态真正变化时前移。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::app_paths;

/// 门禁状态文件版本（读到其他版本视为不可读，保持原文件不动）。
pub const APPEARANCE_STATE_VERSION: u32 = 1;

/// 渲染层外观档案的存储键（逐字对齐 `src/shared/appearance-profile.ts`）。
pub const APPEARANCE_STORAGE_KEY: &str = "ai-novel-writer-appearance";

/// 默认背景皮肤（对齐基线皮肤服务初始状态）。
pub const DEFAULT_BACKGROUND_SKIN: &str = "classic";

/// 合法皮肤标识（对齐 `src/shared/skin-types.ts` 的 `SkinId`）。
pub const SKIN_IDS: [&str; 3] = ["classic", "anime", "custom"];

/// 启动阻塞码（契约字面量，对齐 `src/shared/startup-contract.ts`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupBlockedCode {
    GlobalDataBlocked,
    GlobalModelCredentialsDiffer,
    SkinNotReady,
}

impl StartupBlockedCode {
    pub fn as_str(self) -> &'static str {
        match self {
            StartupBlockedCode::GlobalDataBlocked => "GLOBAL_DATA_BLOCKED",
            StartupBlockedCode::GlobalModelCredentialsDiffer => "GLOBAL_MODEL_CREDENTIALS_DIFFER",
            StartupBlockedCode::SkinNotReady => "SKIN_NOT_READY",
        }
    }
}

/// 全局外观门禁状态（`appearance.json` 的内容）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceState {
    pub version: u32,
    pub skin_revision: u64,
    pub background_skin: String,
}

impl AppearanceState {
    fn initial() -> Self {
        AppearanceState {
            version: APPEARANCE_STATE_VERSION,
            skin_revision: 0,
            background_skin: DEFAULT_BACKGROUND_SKIN.to_string(),
        }
    }
}

/// 皮肤快照（契约字段：globalGeneration / skinRevision / backgroundSkin）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceSkinSnapshot {
    pub global_generation: String,
    pub skin_revision: u64,
    pub background_skin: String,
}

/// 旧来源迁移告知（Tauri 两栈隔离下恒不产生；保留契约形状）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupMigrationNotice {
    pub legacy_source_ignored: bool,
    pub preserved_unknown_count: u64,
}

/// `startup:get-state` 的返回状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupState {
    Blocked {
        code: Option<StartupBlockedCode>,
    },
    Pending,
    Ready {
        global_generation: String,
        skin_revision: u64,
        migration_notice: Option<StartupMigrationNotice>,
    },
}

impl StartupState {
    /// 契约序列化：blocked（可带码）/ pending / ready。
    pub fn to_json(&self) -> Value {
        match self {
            StartupState::Blocked { code } => match code {
                Some(code) => json!({ "state": "blocked", "code": code.as_str() }),
                None => json!({ "state": "blocked" }),
            },
            StartupState::Pending => json!({ "state": "pending" }),
            StartupState::Ready {
                global_generation,
                skin_revision,
                migration_notice,
            } => {
                let mut value = json!({
                    "state": "ready",
                    "globalGeneration": global_generation,
                    "skinRevision": skin_revision,
                });
                if let Some(notice) = migration_notice {
                    value["migrationNotice"] = json!({
                        "legacySourceIgnored": notice.legacy_source_ignored,
                        "preservedUnknownCount": notice.preserved_unknown_count,
                    });
                }
                value
            }
        }
    }
}

/// 门禁状态文件读取失败分类 → 阻塞码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppearanceLoadError {
    /// 不可读 / JSON 非法 / 版本不认识：保持原文件不动，按全局数据阻塞处理。
    GlobalDataBlocked,
    /// 可读但皮肤取值非法。
    SkinNotReady,
}

/// 皮肤状态写入失败分类（皮肤命令据此映射契约错误码）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkinStateWriteError {
    /// 皮肤标识不属于 `SkinId`。
    InvalidSkinId,
    /// 门禁尚未装载（进程启动期尚未 prepare）。
    NotPrepared,
    /// 持久化失败（原文件与内存状态均保持不动）。
    StorageFailed(String),
}

/// 内容修订（FNV-1a 64）：同一内容恒得同一生成号，皮肤状态变化必然前移。
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn content_generation(state: &AppearanceState) -> String {
    // 生成号只取真正的内容（文件版本 + 背景皮肤）：恢复到同一皮肤即回到同一生成号。
    // skinRevision 作为独立单调计数随快照一并返回，不参与内容摘要。
    let canonical = format!("{}|{}", state.version, state.background_skin);
    format!("v{}-{:016x}", state.version, fnv1a64(canonical.as_bytes()))
}

fn is_valid_skin_id(value: &str) -> bool {
    SKIN_IDS.contains(&value)
}

fn parse_state(raw: &[u8]) -> Result<AppearanceState, AppearanceLoadError> {
    let state: AppearanceState =
        serde_json::from_slice(raw).map_err(|_| AppearanceLoadError::GlobalDataBlocked)?;
    if state.version != APPEARANCE_STATE_VERSION {
        return Err(AppearanceLoadError::GlobalDataBlocked);
    }
    if !is_valid_skin_id(&state.background_skin) {
        return Err(AppearanceLoadError::SkinNotReady);
    }
    Ok(state)
}

fn write_state(path: &Path, state: &AppearanceState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建全局数据目录 {}：{error}", parent.display()))?;
    }
    let serialized = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("外观门禁状态无法序列化：{error}"))?;
    std::fs::write(path, serialized)
        .map_err(|error| format!("外观门禁状态无法写入 {}：{error}", path.display()))
}

/// 启动门禁状态机。先经 [`StartupGate::prepare_at`] 装载，再接受命令查询。
///
/// 未装载（无状态、无阻塞码）时 `startup:get-state` 回 pending，与基线「主进程尚未就绪」一致。
#[derive(Debug, Default)]
pub struct StartupGate {
    state_path: Option<PathBuf>,
    state: Option<AppearanceState>,
    blocked: Option<StartupBlockedCode>,
    /// 读回修订：按窗口标签记录本次进程内已确认的最高档案修订（对齐基线
    /// `acknowledgedRevisions`，进程内存态、不落盘）。
    acknowledged: HashMap<String, u64>,
}

impl StartupGate {
    pub fn new() -> Self {
        StartupGate::default()
    }

    /// 装载门禁状态（幂等）：数据根不可用 → GLOBAL_DATA_BLOCKED；
    /// 状态文件缺失则写入初值；文件不可读 / 版本不认识 → GLOBAL_DATA_BLOCKED；
    /// 皮肤取值非法 → SKIN_NOT_READY。
    pub fn prepare_at(&mut self, home: &Path) {
        self.state = None;
        self.blocked = None;
        self.acknowledged.clear();
        let path = app_paths::appearance_state_path_at(home);
        self.state_path = Some(path.clone());
        if let Err(error) = app_paths::ensure_lorekeeper_home_at(home) {
            eprintln!("[Lorekeeper] {error}");
            self.blocked = Some(StartupBlockedCode::GlobalDataBlocked);
            return;
        }
        match std::fs::read(&path) {
            Ok(raw) => match parse_state(&raw) {
                Ok(state) => self.state = Some(state),
                Err(AppearanceLoadError::GlobalDataBlocked) => {
                    self.blocked = Some(StartupBlockedCode::GlobalDataBlocked)
                }
                Err(AppearanceLoadError::SkinNotReady) => {
                    self.blocked = Some(StartupBlockedCode::SkinNotReady)
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let state = AppearanceState::initial();
                if write_state(&path, &state).is_err() {
                    self.blocked = Some(StartupBlockedCode::GlobalDataBlocked);
                    return;
                }
                self.state = Some(state);
            }
            Err(_) => self.blocked = Some(StartupBlockedCode::GlobalDataBlocked),
        }
    }

    pub fn blocked_code(&self) -> Option<StartupBlockedCode> {
        self.blocked
    }

    pub fn state_path(&self) -> Option<&Path> {
        self.state_path.as_deref()
    }

    /// 当前背景皮肤（未装载时为 None）。
    pub fn background_skin(&self) -> Option<&str> {
        self.state
            .as_ref()
            .map(|state| state.background_skin.as_str())
    }

    pub fn skin_revision(&self) -> Option<u64> {
        self.state.as_ref().map(|state| state.skin_revision)
    }

    fn snapshot(&self) -> Option<AppearanceSkinSnapshot> {
        self.state.as_ref().map(|state| AppearanceSkinSnapshot {
            global_generation: content_generation(state),
            skin_revision: state.skin_revision,
            background_skin: state.background_skin.clone(),
        })
    }

    /// `startup:get-state` 语义（调用方已通过信任发送方校验）。
    pub fn startup_state(&self) -> StartupState {
        if let Some(code) = self.blocked {
            return StartupState::Blocked { code: Some(code) };
        }
        match self.snapshot() {
            Some(snapshot) => StartupState::Ready {
                global_generation: snapshot.global_generation,
                skin_revision: snapshot.skin_revision,
                // Tauri 不读基线目录，故没有旧来源可告知（刻意偏离，见模块文档）。
                migration_notice: None,
            },
            None => StartupState::Pending,
        }
    }

    /// `startup:get-state` 的信任发送方失败分支：基线回 blocked（不带码）。
    pub fn blocked_for_untrusted_sender(&self) -> StartupState {
        StartupState::Blocked { code: None }
    }

    /// `startup:skin-snapshot` 语义：阻塞或快照漂移一律以错误拒绝（基线 throw）。
    pub fn skin_snapshot(&self, expected: &Value) -> Result<AppearanceSkinSnapshot, String> {
        if self.blocked.is_some() {
            return Err("启动快照访问被拒绝".to_string());
        }
        let current = self
            .snapshot()
            .ok_or_else(|| "全局配置或皮肤尚未就绪，已保留现有设置".to_string())?;
        let expected = expected
            .as_object()
            .ok_or_else(|| "启动快照已变化，请重新检查".to_string())?;
        let expected_state = expected.get("state").and_then(Value::as_str);
        let expected_generation = expected.get("globalGeneration").and_then(Value::as_str);
        let expected_revision = expected.get("skinRevision").and_then(Value::as_u64);
        if expected_state != Some("ready")
            || expected_generation != Some(current.global_generation.as_str())
            || expected_revision != Some(current.skin_revision)
        {
            return Err("启动快照已变化，请重新检查".to_string());
        }
        Ok(current)
    }

    /// `startup:appearance-ack` 语义：形状 / 存储键 / 修订下界 / 生成号 / 单调性逐条校验，
    /// 任一不满足回 false（基线不 reject）。
    pub fn acknowledge(&mut self, window_label: &str, ack: &Value) -> bool {
        if self.blocked.is_some() {
            return false;
        }
        let Some(current) = self.snapshot() else {
            return false;
        };
        let Some(ack) = ack.as_object() else {
            return false;
        };
        const KEYS: [&str; 4] = [
            "storageKey",
            "profileRevision",
            "globalGeneration",
            "skinRevision",
        ];
        if ack.len() != KEYS.len() || !KEYS.iter().all(|key| ack.contains_key(*key)) {
            return false;
        }
        let Some(storage_key) = ack.get("storageKey").and_then(Value::as_str) else {
            return false;
        };
        if storage_key != APPEARANCE_STORAGE_KEY {
            return false;
        }
        let Some(profile_revision) = ack.get("profileRevision").and_then(Value::as_u64) else {
            return false;
        };
        if profile_revision < 1 {
            return false;
        }
        let Some(generation) = ack.get("globalGeneration").and_then(Value::as_str) else {
            return false;
        };
        let Some(skin_revision) = ack.get("skinRevision").and_then(Value::as_u64) else {
            return false;
        };
        if generation != current.global_generation || skin_revision != current.skin_revision {
            return false;
        }
        if profile_revision < self.acknowledged.get(window_label).copied().unwrap_or(0) {
            return false;
        }
        self.acknowledged
            .insert(window_label.to_string(), profile_revision);
        true
    }

    /// 皮肤状态变更（皮肤命令调用）：写入成功后才前移修订；同一取值不重复前移。
    pub fn set_background_skin(&mut self, skin_id: &str) -> Result<bool, SkinStateWriteError> {
        if !is_valid_skin_id(skin_id) {
            return Err(SkinStateWriteError::InvalidSkinId);
        }
        let Some(path) = self.state_path.clone() else {
            return Err(SkinStateWriteError::NotPrepared);
        };
        let Some(current) = self.state.clone() else {
            return Err(SkinStateWriteError::NotPrepared);
        };
        if current.background_skin == skin_id {
            return Ok(false);
        }
        let next = AppearanceState {
            version: current.version,
            skin_revision: current.skin_revision + 1,
            background_skin: skin_id.to_string(),
        };
        write_state(&path, &next).map_err(SkinStateWriteError::StorageFailed)?;
        self.state = Some(next);
        // 生成号已前移：旧窗口的读回修订自然失效（与基线换皮肤后要求重新读回一致）。
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lorekeeper-startup-gate-{}-{}",
            tag,
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn prepared(tag: &str) -> (PathBuf, StartupGate) {
        let home = temp_home(tag);
        let mut gate = StartupGate::new();
        gate.prepare_at(&home);
        (home, gate)
    }

    fn ready_snapshot(gate: &StartupGate) -> AppearanceSkinSnapshot {
        gate.snapshot().expect("已装载的门禁应有快照")
    }

    #[test]
    fn prepare_creates_state_file_and_reports_ready_test() {
        let (home, gate) = prepared("create");
        assert_eq!(gate.blocked_code(), None);
        assert!(
            home.join("appearance.json").is_file(),
            "首次装载应写入外观门禁状态文件"
        );
        assert_eq!(gate.background_skin(), Some("classic"));
        assert_eq!(gate.skin_revision(), Some(0));
        let state = gate.startup_state();
        assert!(matches!(state, StartupState::Ready { .. }));
        assert_eq!(state.to_json()["state"], "ready");
        assert_eq!(state.to_json()["skinRevision"], 0);
        assert!(state.to_json().get("migrationNotice").is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn unprepared_gate_reports_pending_test() {
        let gate = StartupGate::new();
        assert_eq!(gate.blocked_code(), None);
        assert_eq!(gate.startup_state(), StartupState::Pending);
        assert_eq!(gate.startup_state().to_json()["state"], "pending");
        assert!(gate.skin_snapshot(&json!({ "state": "ready" })).is_err());
        assert!(gate.background_skin().is_none());
    }

    #[test]
    fn generation_is_content_revision_and_moves_only_on_change_test() {
        let (home, mut gate) = prepared("generation");
        let before = ready_snapshot(&gate).global_generation;
        // 同一皮肤：不重复前移
        assert_eq!(gate.set_background_skin("classic"), Ok(false));
        assert_eq!(ready_snapshot(&gate).global_generation, before);
        // 真正变更：修订与生成号同时前移
        assert_eq!(gate.set_background_skin("anime"), Ok(true));
        let after = ready_snapshot(&gate);
        assert_eq!(after.skin_revision, 1);
        assert_ne!(after.global_generation, before);
        assert_eq!(gate.set_background_skin("anime"), Ok(false));
        assert_eq!(
            ready_snapshot(&gate).global_generation,
            after.global_generation
        );

        // 重新装载同一文件：内容未变则生成号一致（内容修订语义）
        let mut reloaded = StartupGate::new();
        reloaded.prepare_at(&home);
        assert_eq!(ready_snapshot(&reloaded), after);

        // 恢复原皮肤：内容回到初值，生成号回到初值，但皮肤修订继续单调前移
        assert_eq!(gate.set_background_skin("classic"), Ok(true));
        assert_eq!(ready_snapshot(&gate).global_generation, before);
        assert_eq!(ready_snapshot(&gate).skin_revision, 2);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn corrupt_or_unknown_version_state_blocks_without_overwriting_test() {
        let home = temp_home("corrupt");
        app_paths::ensure_lorekeeper_home_at(&home).unwrap();
        let path = home.join("appearance.json");
        std::fs::write(&path, b"{ not json").unwrap();
        let mut gate = StartupGate::new();
        gate.prepare_at(&home);
        assert_eq!(
            gate.blocked_code(),
            Some(StartupBlockedCode::GlobalDataBlocked)
        );
        assert_eq!(
            gate.startup_state().to_json(),
            json!({ "state": "blocked", "code": "GLOBAL_DATA_BLOCKED" })
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"{ not json",
            "坏文件不得被默认值覆盖"
        );

        std::fs::write(
            &path,
            br#"{ "version": 99, "skinRevision": 0, "backgroundSkin": "classic" }"#,
        )
        .unwrap();
        let mut gate = StartupGate::new();
        gate.prepare_at(&home);
        assert_eq!(
            gate.blocked_code(),
            Some(StartupBlockedCode::GlobalDataBlocked)
        );
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("\"version\": 99"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn invalid_skin_value_blocks_as_skin_not_ready_test() {
        let home = temp_home("skin-not-ready");
        app_paths::ensure_lorekeeper_home_at(&home).unwrap();
        std::fs::write(
            home.join("appearance.json"),
            br#"{ "version": 1, "skinRevision": 4, "backgroundSkin": "neon" }"#,
        )
        .unwrap();
        let mut gate = StartupGate::new();
        gate.prepare_at(&home);
        assert_eq!(gate.blocked_code(), Some(StartupBlockedCode::SkinNotReady));
        assert_eq!(
            gate.startup_state().to_json(),
            json!({ "state": "blocked", "code": "SKIN_NOT_READY" })
        );
        assert!(gate.skin_snapshot(&json!({ "state": "ready" })).is_err());
        assert!(!gate.acknowledge(
            "main",
            &json!({ "storageKey": APPEARANCE_STORAGE_KEY, "profileRevision": 1,
                "globalGeneration": "x", "skinRevision": 1 })
        ));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn missing_home_parent_blocks_as_global_data_blocked_test() {
        let home = temp_home("unusable");
        std::fs::write(&home, b"not a directory").unwrap();
        let mut gate = StartupGate::new();
        gate.prepare_at(&home);
        assert_eq!(
            gate.blocked_code(),
            Some(StartupBlockedCode::GlobalDataBlocked)
        );
        let _ = std::fs::remove_file(&home);
    }

    #[test]
    fn skin_snapshot_rejects_drift_and_blocked_gate_test() {
        let (home, gate) = prepared("snapshot-drift");
        let snapshot = ready_snapshot(&gate);
        let expected = json!({
            "state": "ready",
            "globalGeneration": snapshot.global_generation,
            "skinRevision": snapshot.skin_revision,
        });
        assert_eq!(gate.skin_snapshot(&expected), Ok(snapshot.clone()));
        // 额外字段（如 migrationNotice）不影响判定，对齐基线只比较三个字段
        let with_notice = json!({
            "state": "ready",
            "globalGeneration": snapshot.global_generation,
            "skinRevision": snapshot.skin_revision,
            "migrationNotice": { "legacySourceIgnored": false, "preservedUnknownCount": 0 },
        });
        assert_eq!(gate.skin_snapshot(&with_notice), Ok(snapshot.clone()));
        for drifted in [
            json!({ "state": "ready", "globalGeneration": "other", "skinRevision": snapshot.skin_revision }),
            json!({ "state": "ready", "globalGeneration": snapshot.global_generation, "skinRevision": 7 }),
            json!({ "state": "pending" }),
            json!(null),
        ] {
            assert!(gate.skin_snapshot(&drifted).is_err(), "漂移请求必须被拒绝");
        }

        // 阻塞态：一律回「访问被拒绝」，不泄漏快照
        let mut blocked = StartupGate::new();
        blocked.blocked = Some(StartupBlockedCode::SkinNotReady);
        assert_eq!(
            blocked.skin_snapshot(&expected).unwrap_err(),
            "启动快照访问被拒绝"
        );
        // 不信任发送方的 get-state：回 blocked 但不带码
        assert_eq!(
            blocked.blocked_for_untrusted_sender().to_json(),
            json!({ "state": "blocked" })
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn appearance_ack_validates_shape_and_monotonicity_per_window_test() {
        let (home, mut gate) = prepared("ack");
        let snapshot = ready_snapshot(&gate);
        let valid = json!({
            "storageKey": APPEARANCE_STORAGE_KEY,
            "profileRevision": 2,
            "globalGeneration": snapshot.global_generation,
            "skinRevision": snapshot.skin_revision,
        });
        assert!(gate.acknowledge("main", &valid));
        // 同一窗口：相同或更高的修订仍接受
        assert!(gate.acknowledge("main", &valid));
        assert!(gate.acknowledge(
            "main",
            &json!({ "storageKey": APPEARANCE_STORAGE_KEY, "profileRevision": 3,
                "globalGeneration": snapshot.global_generation, "skinRevision": snapshot.skin_revision })
        ));
        // 同一窗口：更低的修订被拒绝（回滚的档案不得再次确认）
        assert!(!gate.acknowledge("main", &valid));
        // 另一窗口独立（对齐基线按 sender 记录）
        assert!(gate.acknowledge("second", &valid));

        for invalid in [
            json!({ "storageKey": APPEARANCE_STORAGE_KEY, "profileRevision": 4,
                "globalGeneration": snapshot.global_generation }),
            json!({ "storageKey": APPEARANCE_STORAGE_KEY, "profileRevision": 4,
                "globalGeneration": snapshot.global_generation, "skinRevision": snapshot.skin_revision, "extra": 1 }),
            json!({ "storageKey": "other-key", "profileRevision": 4,
                "globalGeneration": snapshot.global_generation, "skinRevision": snapshot.skin_revision }),
            json!({ "storageKey": APPEARANCE_STORAGE_KEY, "profileRevision": 0,
                "globalGeneration": snapshot.global_generation, "skinRevision": snapshot.skin_revision }),
            json!({ "storageKey": APPEARANCE_STORAGE_KEY, "profileRevision": 4.5,
                "globalGeneration": snapshot.global_generation, "skinRevision": snapshot.skin_revision }),
            json!({ "storageKey": APPEARANCE_STORAGE_KEY, "profileRevision": 4,
                "globalGeneration": "stale", "skinRevision": snapshot.skin_revision }),
            json!({ "storageKey": APPEARANCE_STORAGE_KEY, "profileRevision": 4,
                "globalGeneration": snapshot.global_generation, "skinRevision": 99 }),
            json!([1, 2, 3]),
        ] {
            assert!(!gate.acknowledge("main", &invalid), "非法读回必须拒绝");
        }

        // 换皮肤后旧生成号失效，必须用新快照读回
        assert_eq!(gate.set_background_skin("custom"), Ok(true));
        assert!(!gate.acknowledge("main", &valid));
        let moved = ready_snapshot(&gate);
        assert!(gate.acknowledge(
            "window-after-skin",
            &json!({ "storageKey": APPEARANCE_STORAGE_KEY, "profileRevision": 1,
                "globalGeneration": moved.global_generation, "skinRevision": moved.skin_revision })
        ));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn set_background_skin_rejects_unknown_id_and_unprepared_gate_test() {
        let (home, mut gate) = prepared("skin-id");
        assert!(gate.set_background_skin("neon").is_err());
        assert_eq!(ready_snapshot(&gate).skin_revision, 0);
        let mut unprepared = StartupGate::new();
        assert!(unprepared.set_background_skin("anime").is_err());
        let _ = std::fs::remove_dir_all(&home);
    }
}
