//! 纸墨启动门禁：全局根就绪 → 外观 v3 写入读回 → 主窗口确认。
//!
//! 旧 appearance.json 仅作只读兼容检查，不再保存或恢复图片皮肤。
//! 新的渲染层外观档案使用独立键，旧键、旧文件与图片均不删除。
//! 不读取 Vela 或 Electron 数据根；世代令牌在本次进程装载后保持稳定。

use std::collections::HashMap;
use std::path::Path;

use serde_json::{json, Value};

use crate::app_paths;

pub const APPEARANCE_STORAGE_KEY: &str = "ai-novel-writer-appearance-v3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupBlockedCode {
    GlobalDataBlocked,
}

impl StartupBlockedCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GlobalDataBlocked => "GLOBAL_DATA_BLOCKED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupState {
    Blocked { code: Option<StartupBlockedCode> },
    Pending,
    Ready { global_generation: String },
}

impl StartupState {
    pub fn to_json(&self) -> Value {
        match self {
            Self::Blocked { code: Some(code) } => {
                json!({ "state": "blocked", "code": code.as_str() })
            }
            Self::Blocked { code: None } => json!({ "state": "blocked" }),
            Self::Pending => json!({ "state": "pending" }),
            Self::Ready { global_generation } => {
                json!({ "state": "ready", "globalGeneration": global_generation })
            }
        }
    }
}

/// 仅保存本次进程的世代与各窗口已确认修订，重启不会信任旧读回。
#[derive(Debug, Default)]
pub struct StartupGate {
    generation: Option<String>,
    blocked: Option<StartupBlockedCode>,
    acknowledged: HashMap<String, u64>,
}

impl StartupGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// 幂等装载。旧门禁文件损坏或版本未知时仍失败关闭，不覆盖原文件。
    pub fn prepare_at(&mut self, home: &Path) {
        if self.generation.is_some() && self.blocked.is_none() {
            return;
        }
        self.generation = None;
        self.acknowledged.clear();
        self.blocked = None;
        if app_paths::ensure_lorekeeper_home_at(home).is_err() {
            self.blocked = Some(StartupBlockedCode::GlobalDataBlocked);
            return;
        }
        let legacy = app_paths::appearance_state_path_at(home);
        match std::fs::read(&legacy) {
            Ok(bytes) => {
                let valid = serde_json::from_slice::<Value>(&bytes)
                    .ok()
                    .is_some_and(|value| {
                        value.is_object() && value.get("version").and_then(Value::as_u64) == Some(1)
                    });
                if !valid {
                    self.blocked = Some(StartupBlockedCode::GlobalDataBlocked);
                    return;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                self.blocked = Some(StartupBlockedCode::GlobalDataBlocked);
                return;
            }
        }
        // 旧皮肤名与修订不再参与握手。新进程得到新世代，旧确认不能重放。
        self.generation = Some(crate::project_access::random_uuid_v4());
    }

    pub fn blocked_code(&self) -> Option<StartupBlockedCode> {
        self.blocked
    }

    pub fn startup_state(&self) -> StartupState {
        if let Some(code) = self.blocked {
            StartupState::Blocked { code: Some(code) }
        } else if let Some(generation) = &self.generation {
            StartupState::Ready {
                global_generation: generation.clone(),
            }
        } else {
            StartupState::Pending
        }
    }

    pub fn blocked_for_untrusted_sender(&self) -> StartupState {
        StartupState::Blocked { code: None }
    }

    /// 校验当前世代、新存储键、安全整数修订、每窗口单调性与精确字段集。
    /// 调用方须先验证主窗口与文档来源；不接受旧 skinRevision 字段。
    pub fn acknowledge(&mut self, window_label: &str, ack: &Value) -> bool {
        if self.blocked.is_some() {
            return false;
        }
        let Some(current) = self.generation.as_deref() else {
            return false;
        };
        let Some(ack) = ack.as_object() else {
            return false;
        };
        const KEYS: [&str; 3] = ["storageKey", "profileRevision", "globalGeneration"];
        if ack.len() != KEYS.len() || !KEYS.iter().all(|key| ack.contains_key(*key)) {
            return false;
        }
        if ack.get("storageKey").and_then(Value::as_str) != Some(APPEARANCE_STORAGE_KEY)
            || ack.get("globalGeneration").and_then(Value::as_str) != Some(current)
        {
            return false;
        }
        let Some(revision) = ack.get("profileRevision").and_then(Value::as_u64) else {
            return false;
        };
        if !(1..=9_007_199_254_740_991).contains(&revision)
            || revision < self.acknowledged.get(window_label).copied().unwrap_or(0)
        {
            return false;
        }
        self.acknowledged.insert(window_label.to_string(), revision);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Home(std::path::PathBuf);
    impl Home {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "lorekeeper-stage-startup-{}",
                crate::project_access::random_uuid_v4()
            )))
        }
    }
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn ready() -> (Home, StartupGate, String) {
        let home = Home::new();
        let mut gate = StartupGate::new();
        gate.prepare_at(&home.0);
        let StartupState::Ready { global_generation } = gate.startup_state() else {
            panic!("隔离根应就绪");
        };
        (home, gate, global_generation)
    }
    fn ack(generation: &str, revision: u64) -> Value {
        json!({
            "storageKey": APPEARANCE_STORAGE_KEY,
            "profileRevision": revision,
            "globalGeneration": generation,
        })
    }
    #[test]
    fn pending_and_untrusted_do_not_disclose_generation() {
        let gate = StartupGate::new();
        assert_eq!(gate.startup_state().to_json(), json!({"state":"pending"}));
        assert_eq!(
            gate.blocked_for_untrusted_sender().to_json(),
            json!({"state":"blocked"})
        );
    }
    #[test]
    fn v3_ack_exact_shape_monotonic_and_window_scoped() {
        let (_home, mut gate, generation) = ready();
        assert!(gate.acknowledge("main", &ack(&generation, 3)));
        assert!(gate.acknowledge("main", &ack(&generation, 3)));
        assert!(!gate.acknowledge("main", &ack(&generation, 2)));
        assert!(gate.acknowledge("another", &ack(&generation, 1)));
        assert!(!gate.acknowledge("main", &ack("stale-generation", 4)));
        assert!(!gate.acknowledge("main", &ack(&generation, 0)));
        assert!(!gate.acknowledge("main", &ack(&generation, 9_007_199_254_740_992)));
        let mut legacy = ack(&generation, 4);
        legacy["storageKey"] = json!("ai-novel-writer-appearance");
        assert!(!gate.acknowledge("main", &legacy));
        let mut extra = ack(&generation, 4);
        extra["skinRevision"] = json!(0);
        assert!(!gate.acknowledge("main", &extra));
        assert!(!gate.acknowledge("main", &json!(null)));
        let mut missing = ack(&generation, 4);
        missing.as_object_mut().unwrap().remove("storageKey");
        assert!(!gate.acknowledge("main", &missing));
        let mut negative = ack(&generation, 4);
        negative["profileRevision"] = json!(-1);
        assert!(!gate.acknowledge("main", &negative));
        let mut fraction = ack(&generation, 4);
        fraction["profileRevision"] = json!(1.5);
        assert!(!gate.acknowledge("main", &fraction));
    }
    #[test]
    fn fresh_start_does_not_create_legacy_skin_state_and_restart_revokes_ack() {
        let (home, mut gate, generation) = ready();
        assert!(!app_paths::appearance_state_path_at(&home.0).exists());
        gate.prepare_at(&home.0);
        assert_eq!(
            gate.startup_state(),
            StartupState::Ready {
                global_generation: generation.clone()
            }
        );
        let mut restarted = StartupGate::new();
        restarted.prepare_at(&home.0);
        assert!(!restarted.acknowledge("main", &ack(&generation, 1)));
    }
    #[test]
    fn legacy_skin_file_is_preserved_byte_for_byte() {
        for skin in ["classic", "anime", "custom", "obsolete-skin"] {
            let home = Home::new();
            app_paths::ensure_lorekeeper_home_at(&home.0).unwrap();
            let path = app_paths::appearance_state_path_at(&home.0);
            let bytes =
                format!("{{\"version\":1,\"skinRevision\":17,\"backgroundSkin\":\"{skin}\"}}")
                    .into_bytes();
            std::fs::write(&path, &bytes).unwrap();
            let mut gate = StartupGate::new();
            gate.prepare_at(&home.0);
            assert!(matches!(gate.startup_state(), StartupState::Ready { .. }));
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }
    #[test]
    fn malformed_or_future_legacy_file_fails_closed_without_rewrite() {
        for bytes in [b"{broken".as_slice(), b"{\"version\":99}", b"null", b"{}"] {
            let home = Home::new();
            app_paths::ensure_lorekeeper_home_at(&home.0).unwrap();
            let path = app_paths::appearance_state_path_at(&home.0);
            std::fs::write(&path, bytes).unwrap();
            let mut gate = StartupGate::new();
            gate.prepare_at(&home.0);
            assert_eq!(
                gate.blocked_code(),
                Some(StartupBlockedCode::GlobalDataBlocked)
            );
            assert!(!gate.acknowledge("main", &ack("anything", 1)));
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }
}
