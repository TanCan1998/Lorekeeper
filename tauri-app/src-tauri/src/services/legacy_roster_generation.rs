//! 旧名册生成共享类型（移植自 `src/shared/legacy-roster-generation.ts`）。
//!
//! 纯类型模块：无逻辑。`LegacyRosterGenerationChannels` 是上游的
//! Electron IPC 频道映射表，Tauri 侧频道契约由
//! `tauri-app/src/shared/ipc-channels.ts` 统一维护，故不移植
//! （Rust 侧无对应物）。
//!
//! `view` / `proposal` 两项的上游类型（`MainGenerationRunView` /
//! `CharacterProposalBatch`）尚未移植为 Rust 类型，这里保留 JSON 值
//! 占位，待后续波次接入。

use serde::{Deserialize, Serialize};

use crate::services::legacy_roster_source::LegacyRosterSource;

/// 旧名册生成上下文（对齐上游 `LegacyRosterGenerationContext`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LegacyRosterGenerationContext {
    pub project_id: String,
    pub origin_epoch: String,
    pub key: String,
    pub created_at: String,
    pub source: LegacyRosterSource,
}

/// 旧名册生成产物（对齐上游 `LegacyRosterGenerationArtifact`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LegacyRosterGenerationArtifact {
    pub artifact_id: String,
    pub revision: i64,
    pub text_hash: String,
}

/// 恢复视图中的来源状态（对齐上游 `'current' | 'conflict'`）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LegacyRosterSourceStatus {
    Current,
    Conflict,
}

/// 旧名册生成恢复视图（对齐上游 `LegacyRosterGenerationRecovery`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LegacyRosterGenerationRecovery {
    pub view: serde_json::Value,
    pub model_id: String,
    pub context: LegacyRosterGenerationContext,
    pub source_status: LegacyRosterSourceStatus,
    pub attempt_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<LegacyRosterGenerationArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<serde_json::Value>,
}
