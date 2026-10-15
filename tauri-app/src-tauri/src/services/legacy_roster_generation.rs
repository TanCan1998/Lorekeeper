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

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::repositories::generation_run_repository::{
    DurableGenerationRun, GenerationRunRepository,
};
use crate::services::character_proposal::{
    read_legacy_roster_generation_proof, CharacterProposalService,
};
use crate::services::legacy_roster_generation_context::{
    legacy_roster_generation_task, read_legacy_roster_generation_context,
};
use crate::services::legacy_roster_generation_pure::GenerationTask;
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

/// 旧名册生成服务（平移 `electron/services/legacy-roster-generation.ts`）。
///
/// 旧名册修复生成的生命周期管理：运行身份校验（`require`）、
/// 任务构造（`task`）、产物回执引用（`artifact_for`）、持久化
/// 效应读回（`saved` / `proposal`）、可变性断言（`assert_mutable`）
/// 与阶段提交（`stage`）。
pub struct LegacyRosterGeneration<'a> {
    conn: &'a Connection,
    runs: GenerationRunRepository,
    project_id: &'a str,
    proposals: &'a CharacterProposalService<'a>,
}

/// 持久化的旧名册生成效应（对齐上游 `StoredLegacyProposal`，
/// 序列化后写入 `generation_attempts.usage_receipt_json` 的
/// `legacyRosterProposal` 键）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredLegacyProposal {
    version: i64,
    artifact: Value,
    context_hash: String,
    proposal_batch_id: String,
    source_hash: String,
}

impl<'a> LegacyRosterGeneration<'a> {
    pub fn new(
        conn: &'a Connection,
        runs: GenerationRunRepository,
        project_id: &'a str,
        proposals: &'a CharacterProposalService<'a>,
    ) -> Self {
        Self {
            conn,
            runs,
            project_id,
            proposals,
        }
    }

    /// 运行身份校验（对齐上游 `require`）：句柄键集 +
    /// `projectId` / `rootActionId` / `epoch`（originEpoch 或
    /// binding epoch）一致性。
    pub fn require(&self, handle: &Value) -> Result<DurableGenerationRun, String> {
        if !keys_subset(handle, &["projectId", "epoch", "rootActionId", "runId"]) {
            return Err("GENERATION_RUN_IDENTITY_MISMATCH".to_string());
        }
        let run = self
            .runs
            .get(self.conn, handle["runId"].as_str().unwrap_or_default())?;
        let context = read_legacy_roster_generation_context(&run)?;
        if handle["projectId"].as_str() != Some(self.project_id)
            || run.binding.project_id != *self.project_id
            || handle["rootActionId"].as_str() != Some(run.root_action_id.as_str())
            || (handle["epoch"].as_str() != Some(context.origin_epoch.as_str())
                && handle["epoch"].as_str() != Some(run.binding.epoch.as_str()))
        {
            return Err("GENERATION_RUN_IDENTITY_MISMATCH".to_string());
        }
        Ok(run)
    }

    fn handle(&self, run: &DurableGenerationRun) -> Value {
        json!({
            "projectId": self.project_id,
            "epoch": run.binding.epoch,
            "rootActionId": run.root_action_id,
            "runId": run.run_id,
        })
    }

    pub fn task(&self, run: &DurableGenerationRun) -> Result<GenerationTask, String> {
        let context = read_legacy_roster_generation_context(run)?;
        Ok(legacy_roster_generation_task(&context))
    }

    /// 最近一次 attempt 的产物回执引用（对齐上游 `artifactFor`）：
    /// 产物存在且 proof 校验通过时返回引用，否则 `None`。
    pub fn artifact_for(&self, run: &DurableGenerationRun) -> Result<Option<Value>, String> {
        let row: Option<(String,)> = self
            .conn
            .query_row(
                "SELECT attempt_id FROM generation_attempts WHERE run_id = ?1 ORDER BY rowid DESC LIMIT 1",
                [run.run_id.as_str()],
                |row| Ok((row.get(0)?,)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((attempt_id,)) = row else {
            return Ok(None);
        };
        let receipt = self.runs.receipt(self.conn, &attempt_id, None)?;
        let Some(artifact) = receipt.artifact else {
            return Ok(None);
        };
        let reference = json!({
            "artifactId": artifact.artifact_id,
            "revision": artifact.revision,
            "textHash": artifact.text_hash,
        });
        match read_legacy_roster_generation_proof(
            self.conn,
            &self.runs,
            self.project_id,
            &json!({
                "kind": "legacy-roster-generation",
                "handle": self.handle(run),
                "artifact": reference,
            }),
        ) {
            Ok(_) => Ok(Some(reference)),
            Err(_) => Ok(None),
        }
    }

    fn saved(&self, run: &DurableGenerationRun) -> Result<Option<StoredLegacyProposal>, String> {
        let context = read_legacy_roster_generation_context(run)?;
        let context_json = serde_json::to_value(&context).map_err(|error| error.to_string())?;
        let context_payload =
            serde_json::to_string(&context_json).map_err(|error| error.to_string())?;
        let context_hash = text_hash(&context_payload);
        let mut statement = self
            .conn
            .prepare(
                "SELECT attempt_id, usage_receipt_json FROM generation_attempts WHERE run_id = ?1 ORDER BY rowid",
            )
            .map_err(|error| format!("准备语句失败: {error}"))?;
        let rows = statement
            .query_map([run.run_id.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("查询失败: {error}"))?;
        let mut result: Option<StoredLegacyProposal> = None;
        for row in rows {
            let (attempt_id, usage_json) = row.map_err(|error| format!("读取失败: {error}"))?;
            let usage: Value = serde_json::from_str(&usage_json)
                .map_err(|error| format!("解析用量回执失败: {error}"))?;
            let Some(effect) = usage.get("legacyRosterProposal") else {
                continue;
            };
            if result.is_some()
                || effect.get("version").and_then(Value::as_i64) != Some(1)
                || effect.get("contextHash").and_then(Value::as_str) != Some(context_hash.as_str())
            {
                return Err("GENERATION_LEGACY_EFFECT_INVALID".to_string());
            }
            let effect: StoredLegacyProposal = serde_json::from_value(effect.clone())
                .map_err(|_| "GENERATION_LEGACY_EFFECT_INVALID".to_string())?;
            let proof = read_legacy_roster_generation_proof(
                self.conn,
                &self.runs,
                self.project_id,
                &json!({
                    "kind": "legacy-roster-generation",
                    "handle": self.handle(run),
                    "artifact": effect.artifact,
                }),
            )?;
            let batch = self.proposals.read(&effect.proposal_batch_id)?;
            let evidence = self.proposals.evidence(&effect.proposal_batch_id)?;
            if proof["attemptId"].as_str() != Some(attempt_id.as_str())
                || effect.source_hash != proof["proof"]["sourceHash"].as_str().unwrap_or_default()
                || evidence["sourceHash"].as_str() != Some(effect.source_hash.as_str())
                || batch["source"] != proof["source"]
            {
                return Err("GENERATION_LEGACY_EFFECT_INVALID".to_string());
            }
            result = Some(effect);
        }
        Ok(result)
    }

    pub fn proposal(&self, run: &DurableGenerationRun) -> Result<Option<Value>, String> {
        match self.saved(run)? {
            Some(effect) => Ok(Some(self.proposals.read(&effect.proposal_batch_id)?)),
            None => Ok(None),
        }
    }

    pub fn assert_mutable(&self, run: &DurableGenerationRun) -> Result<(), String> {
        if run
            .binding
            .source_manifest
            .get("legacyRosterContext")
            .is_some()
            && self.saved(run)?.is_some()
        {
            return Err("GENERATION_LEGACY_EFFECT_SEALED".to_string());
        }
        Ok(())
    }

    /// 阶段提交（对齐上游 `stage`）：幂等重放（已持久化效应
    /// 且 artifact 一致时直接读回批次）；否则 proof 校验 →
    /// 来源断言 → 提案阶段提交 → 效应写入
    /// `generation_attempts.usage_receipt_json`。
    ///
    /// 事务形态对齐上游外层 `db.transaction().immediate()`：
    /// rusqlite 0.40 无 `&self` + Immediate 事务 API，用
    /// `unchecked_transaction` 保证一致写快照；内层
    /// `proposals.stage` 经 `is_autocommit()` 检测自动加入
    /// 调用方事务（等价上游嵌套事务的 savepoint 语义）。
    pub fn stage(
        &self,
        handle: &Value,
        artifact: &Value,
        assert_sources: &dyn Fn(&DurableGenerationRun) -> Result<(), String>,
    ) -> Result<Value, String> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|error| format!("开启事务失败: {error}"))?;
        let run = self.require(handle)?;
        let previous = self.saved(&run)?;
        if let Some(previous) = previous {
            if previous.artifact != *artifact {
                return Err("GENERATION_LEGACY_EFFECT_CONFLICT".to_string());
            }
            // 幂等重放：直接读回持久化批次
            let batch = self.proposals.read(&previous.proposal_batch_id)?;
            tx.commit()
                .map_err(|error| format!("提交事务失败: {error}"))?;
            return Ok(batch);
        }
        let current_artifact = self.artifact_for(&run)?;
        if current_artifact.as_ref() != Some(artifact) {
            return Err("GENERATION_LEGACY_ARTIFACT_INVALID".to_string());
        }
        let proof_source = json!({
            "kind": "legacy-roster-generation",
            "handle": handle,
            "artifact": artifact,
        });
        let proof = read_legacy_roster_generation_proof(
            self.conn,
            &self.runs,
            self.project_id,
            &proof_source,
        )?;
        assert_sources(&run)?;
        let batch = self.proposals.stage(&proof_source)?;
        let context_hash = text_hash(
            &serde_json::to_string(&proof["context"]).map_err(|error| error.to_string())?,
        );
        let effect = json!({
            "version": 1,
            "artifact": artifact,
            "contextHash": context_hash,
            "proposalBatchId": batch["proposalBatchId"],
            "sourceHash": proof["proof"]["sourceHash"],
        });
        let attempt_id = proof["attemptId"].as_str().unwrap_or_default();
        let usage_json: String = tx
            .query_row(
                "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id = ?1",
                [attempt_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("读取用量回执失败: {error}"))?;
        let mut usage: Map<String, Value> = serde_json::from_str(&usage_json)
            .map_err(|error| format!("解析用量回执失败: {error}"))?;
        usage.insert("legacyRosterProposal".to_string(), effect);
        let updated = serde_json::to_string(&Value::Object(usage))
            .map_err(|error| format!("序列化用量回执失败: {error}"))?;
        tx.execute(
            "UPDATE generation_attempts SET usage_receipt_json = ?1 WHERE attempt_id = ?2",
            rusqlite::params![updated, attempt_id],
        )
        .map_err(|error| format!("更新用量回执失败: {error}"))?;
        tx.commit()
            .map_err(|error| format!("提交事务失败: {error}"))?;
        Ok(batch)
    }
}

/// 键集子集校验（对齐上游 `Object.keys(source).some(key =>
/// !allowed.includes(key))` 的失败语义）。
pub(crate) fn keys_subset(source: &Value, allowed: &[&str]) -> bool {
    source
        .as_object()
        .map(|fields| {
            fields
                .keys()
                .all(|key| allowed.iter().any(|allowed| *allowed == key.as_str()))
        })
        .unwrap_or(false)
}

/// SHA-256 hex（对齐上游 `textHash`；G4-6c 就地实现先例——
/// generation_run_repository 的同名函数为模块私有）。
fn text_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
