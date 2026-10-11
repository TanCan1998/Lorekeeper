//! CharacterProposalService —— 角色提案（批次 B1）。
//!
//! 平移自 `electron/services/character-proposal-service.ts`（含文件尾部的
//! `findCharacterProposalEvidence`）与 `electron/services/generation-character-proposal-proof.ts`
//! 的 `proveCharacterProposal` 五分支结构。
//!
//! 与基线的一致性：
//! - 错误码逐字对齐（`CHARACTER_PROPOSAL_RECEIPT_INVALID` / `CHARACTER_PROPOSAL_SOURCE_CHANGED` /
//!   `CHARACTER_APPROVAL_NONCE_CONFLICT` …）；「Error: 」前缀由命令层 `mutating_error` 补齐。
//! - `isDeepStrictEqual` → `serde_json::Value` 相等比较；`structuredClone` → `Value::clone`；
//!   `db.transaction(fn).immediate()` → `conn.unchecked_transaction()` + `tx.commit()`
//!   （错误提前返回时 `tx` drop 即回滚，与基线一致）。
//! - 提案信封 / 批次 / 决策 / 选择项一律 `serde_json::Value` 动态表示（对齐上游动态风格）。
//!
//! ⚠️ 待移植叶函数（本栈尚未迁移，双栈隔离约定下不要求与 Node 逐字节一致）：
//! `proveCharacterProposal` 的 legacy-roster-generation / finalized-generation /
//! generation(architecture|planning-material) 分支与 import 分支的推理解码
//! （`readLegacyRosterGenerationProof` / `proveFinalizedCharacterGeneration` /
//! `parseArchitectureCharacterProposal` / `parsePlanningMaterialCharacterProposal` /
//! `decodeImportInferenceJson` 等）以 `CHARACTER_PROPOSAL_PROOF_PORT_PENDING`
//! 错误码显式桩住（非上游错误码），待后续批次接线。`directory` 分支完整可用。

use std::collections::{BTreeMap, HashSet};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::repositories::character_identity_repository;
use crate::repositories::character_repository::{self as characters, CharacterData};
use crate::repositories::character_roster_repository::{
    self as character_roster, CharacterRosterEntry, CharacterRosterRelationship,
    CharacterRosterStatus,
};
use crate::repositories::generation_run_repository::{
    DurableGenerationRun, GenerationRunRepository,
};
use crate::repositories::project_core_repository::DEFAULT_WRITING_LANGUAGE;

/// 静态字段白名单（对齐基线 `fields` / `CharacterStaticFields`）。
const CHARACTER_PROPOSAL_FIELDS: [&str; 11] = [
    "name",
    "role",
    "gender",
    "age",
    "appearance",
    "personality",
    "background",
    "abilities",
    "motivation",
    "arc",
    "notes",
];

/// 角色定位白名单（对齐 `src/shared/character-role.ts` 的 `CHARACTER_ROLES`）。
const CHARACTER_ROLES: [&str; 4] = ["protagonist", "antagonist", "supporting", "minor"];

/// 待移植叶函数的统一错误码（**非上游错误码**，仅用于本迁移栈内部标记）。
const PENDING_MIGRATION_CODE: &str = "CHARACTER_PROPOSAL_PROOF_PORT_PENDING";

/// JS `Number.MAX_SAFE_INTEGER` 边界。
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// 规范 JSON 的 SHA-256 小写 hex（对齐 `generation-run-repository` 的私有 `textHash`；
/// 该函数在 Rust 侧为私有，本文件自建等价物）。
fn text_hash(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// `Number.isSafeInteger(value)`。
fn safe_integer(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .filter(|number| (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(number))
}

/// 提案证明（对齐 `CharacterProposalProof`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterProposalProof {
    pub items: Vec<Value>,
    pub source_hash: String,
    /// 未证实的 legacy/import 候选仍可读；不能被静默采纳。
    pub provenance: Option<Value>,
}

/// 作用域化身份解析结果（对齐 `ScopedCharacterResolution`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ScopedCharacterResolution {
    Resolved { character_id: String },
    Ambiguous { candidate_ids: Vec<String> },
    Unresolved { candidate_ids: Vec<String> },
}

/// 持久化提案信封（对齐服务内的 `Envelope`；动态字段保持 `Value`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    #[serde(default)]
    version: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    architecture_derivation_version: Option<i64>,
    #[serde(default)]
    project_id: String,
    proof: CharacterProposalProof,
    #[serde(default)]
    batch: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    decision: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    created: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    approval_proof: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    author_edit_proof: Option<Value>,
}

/// 角色提案服务。`prove` 由调用方注入（对齐基线构造器注入的证明函数）。
pub struct CharacterProposalService<'a> {
    conn: &'a Connection,
    project_id: &'a str,
    prove: Box<dyn Fn(&Value, bool, u8) -> Result<CharacterProposalProof, String> + 'a>,
}

impl<'a> CharacterProposalService<'a> {
    pub fn new(
        conn: &'a Connection,
        project_id: &'a str,
        prove: impl Fn(&Value, bool, u8) -> Result<CharacterProposalProof, String> + 'a,
    ) -> Self {
        Self {
            conn,
            project_id,
            prove: Box::new(prove),
        }
    }

    /// 源与持久化信封共享同一 JSON 表示（含被省略的可选字段）。
    fn source_proof(
        &self,
        source: &Value,
        for_write: bool,
        version: u8,
    ) -> Result<CharacterProposalProof, String> {
        // `JSON.parse(JSON.stringify(...))` 的等价物：序列化往返。
        let proof = (self.prove)(source, for_write, version)?;
        let encoded = serde_json::to_value(&proof).map_err(|error| error.to_string())?;
        serde_json::from_value(encoded).map_err(|error| error.to_string())
    }

    fn revision(&self) -> Result<i64, String> {
        self.conn
            .query_row(
                "SELECT revision FROM character_identity_meta WHERE id='main'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
    }

    fn write(&self, envelope: &Envelope) -> Result<(), String> {
        let proposal_batch_id = envelope.batch["proposalBatchId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let encoded = serde_json::to_string(envelope).map_err(|error| error.to_string())?;
        self.conn
            .execute(
                "UPDATE character_identity_proposals SET source_hash=?,raw_value=? WHERE proposal_id=?",
                [text_hash(&encoded), encoded, proposal_batch_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// 定稿来源的时效性断言（仅 finalized-generation 提案需要）。
    fn assert_finalized_source_current(&self, envelope: &Envelope) -> Result<(), String> {
        if envelope.batch["source"]["kind"].as_str() != Some("finalized-generation") {
            return Ok(());
        }
        let source = envelope
            .proof
            .provenance
            .as_ref()
            .and_then(|provenance| provenance.get("source"));
        let Some(source) = source else {
            return Err("CHARACTER_PROPOSAL_SOURCE_CHANGED".to_string());
        };
        let source_id = source["sourceId"].as_str().unwrap_or_default();
        let Some(draft_id) = self
            .conn
            .query_row(
                "SELECT draft_id FROM finalization_outbox WHERE finalization_id=?",
                [source_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
        else {
            return Err("CHARACTER_PROPOSAL_SOURCE_CHANGED".to_string());
        };
        let epoch = envelope.batch["source"]["handle"]["epoch"]
            .as_str()
            .unwrap_or_default();
        let scope = json!({ "projectId": self.project_id, "epoch": epoch });
        // readFinalizedCharacterContext 待移植（见私有桩函数）；上游对任何
        // 失败统一折叠为 `CHARACTER_PROPOSAL_SOURCE_CHANGED`。
        if read_finalized_character_context(self.conn, draft_id, &scope).is_err() {
            return Err("CHARACTER_PROPOSAL_SOURCE_CHANGED".to_string());
        }
        Ok(())
    }

    fn read_envelope(&self, proposal_batch_id: &str) -> Result<Envelope, String> {
        let row = self
            .conn
            .query_row(
                "SELECT source_key,source_hash,raw_value FROM character_identity_proposals WHERE proposal_id=?",
                [proposal_batch_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((source_key, source_hash, raw_value)) = row else {
            return Err("CHARACTER_PROPOSAL_RECEIPT_INVALID".to_string());
        };
        if source_key != format!("character-proposal-v1:{}", self.project_id)
            || text_hash(&raw_value) != source_hash
        {
            return Err("CHARACTER_PROPOSAL_RECEIPT_INVALID".to_string());
        }
        let mut envelope: Envelope = serde_json::from_str(&raw_value)
            .map_err(|_| "CHARACTER_PROPOSAL_RECEIPT_INVALID".to_string())?;
        if envelope.version != 1
            || envelope.project_id != self.project_id
            || envelope.batch["proposalBatchId"].as_str() != Some(proposal_batch_id)
        {
            return Err("CHARACTER_PROPOSAL_RECEIPT_INVALID".to_string());
        }
        if let Some(version) = envelope.architecture_derivation_version {
            if version != 2
                || envelope.batch["source"]["kind"].as_str() != Some("generation")
                || envelope.batch["source"]["inputKind"].as_str() != Some("architecture")
            {
                return Err("CHARACTER_PROPOSAL_RECEIPT_INVALID".to_string());
            }
        }
        let current_proof = match self.source_proof(
            &envelope.batch["source"],
            false,
            envelope.architecture_derivation_version.unwrap_or(1) as u8,
        ) {
            Ok(proof) => proof,
            Err(error) => {
                if envelope.batch["source"]["kind"].as_str() == Some("finalized-generation")
                    && error != "CHARACTER_PROPOSAL_SOURCE_INVALID"
                {
                    return Err("CHARACTER_PROPOSAL_SOURCE_CHANGED".to_string());
                }
                return Err(error);
            }
        };
        if envelope.proof != current_proof {
            // import 升级特例：pending-approval 且旧证明无 provenance 时，
            // 允许把新 provenance 补进信封而非判为来源已变。
            let upgradeable = envelope.batch["source"]["kind"].as_str() == Some("import")
                && envelope.batch["status"].as_str() == Some("pending-approval")
                && envelope.proof.provenance.is_none()
                && current_proof.provenance.is_some();
            if !upgradeable {
                return Err("CHARACTER_PROPOSAL_SOURCE_CHANGED".to_string());
            }
            let mut upgraded = envelope.proof.clone();
            upgraded.provenance = current_proof.provenance.clone();
            if upgraded != current_proof {
                return Err("CHARACTER_PROPOSAL_SOURCE_CHANGED".to_string());
            }
            envelope.proof = current_proof;
        }
        self.assert_finalized_source_current(&envelope)?;
        if envelope.batch["status"].as_str() == Some("approved") {
            let operation_id = envelope.batch["approvalOperationId"]
                .as_str()
                .unwrap_or_default();
            let receipt = self
                .conn
                .query_row(
                    "SELECT receipt_json FROM character_identity_approvals WHERE operation_id=?",
                    [operation_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            let adoption_revision = receipt
                .as_ref()
                .and_then(|json| serde_json::from_str::<Value>(json).ok())
                .and_then(|value| value["revision"].as_i64());
            if receipt.is_none() || envelope.decision.is_none() {
                return Err("CHARACTER_PROPOSAL_APPROVAL_INVALID".to_string());
            }
            let decision = envelope.decision.as_ref().unwrap();
            if envelope.author_edit_proof.is_some() {
                let author_operation_id = format!(
                    "character-proposal-edit:{}",
                    decision["operationId"].as_str().unwrap_or_default()
                );
                let author_receipt = self
                    .conn
                    .query_row(
                        "SELECT payload_hash,receipt_json FROM character_identity_approvals WHERE operation_id=?",
                        [author_operation_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(|error| error.to_string())?;
                let Some((author_payload_hash, author_receipt_json)) = author_receipt else {
                    return Err("CHARACTER_PROPOSAL_APPROVAL_INVALID".to_string());
                };
                let batch_revision = envelope.batch["revision"].as_i64().unwrap_or_default();
                if author_payload_hash
                    != envelope.author_edit_proof.as_ref().unwrap()["payloadHash"]
                        .as_str()
                        .unwrap_or_default()
                    || text_hash(&author_receipt_json)
                        != envelope.author_edit_proof.as_ref().unwrap()["receiptHash"]
                            .as_str()
                            .unwrap_or_default()
                    || serde_json::from_str::<Value>(&author_receipt_json)
                        .ok()
                        .and_then(|value| value["revision"].as_i64())
                        != Some(batch_revision)
                    || adoption_revision != Some(batch_revision - 1)
                {
                    return Err("CHARACTER_PROPOSAL_APPROVAL_INVALID".to_string());
                }
            } else if adoption_revision != envelope.batch["revision"].as_i64() {
                return Err("CHARACTER_PROPOSAL_APPROVAL_INVALID".to_string());
            }
            if envelope.batch["source"]["kind"].as_str() == Some("legacy-roster-generation") {
                let (payload_hash, receipt_json) = self
                    .conn
                    .query_row(
                        "SELECT payload_hash,receipt_json FROM character_identity_approvals WHERE operation_id=?",
                        [operation_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                            ))
                        },
                    )
                    .map_err(|error| error.to_string())?;
                let saved = serde_json::from_str::<Value>(&receipt_json)
                    .map_err(|error| error.to_string())?;
                let approval_proof = envelope.approval_proof.as_ref().unwrap();
                let created_matches = match (&envelope.created, saved.get("created")) {
                    (Some(expected), Some(actual)) => expected == actual,
                    (None, None) => true,
                    _ => false,
                };
                if approval_proof["payloadHash"].as_str() != Some(payload_hash.as_str())
                    || approval_proof["receiptHash"].as_str()
                        != Some(text_hash(&receipt_json).as_str())
                    || saved["approval"]["operationId"] != decision["operationId"]
                    || envelope.batch["approvalOperationId"] != decision["operationId"]
                    || !created_matches
                {
                    return Err("CHARACTER_PROPOSAL_APPROVAL_INVALID".to_string());
                }
            }
        }
        Ok(envelope)
    }

    fn resolve_items(
        &self,
        proof: &CharacterProposalProof,
        source: &Value,
        snapshot: &Value,
    ) -> Result<Vec<Value>, String> {
        let characters = &snapshot["characters"];
        let aliases: Vec<Value> = snapshot["aliases"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .filter(|alias| {
                characters
                    .as_array()
                    .map(|items| {
                        items.iter().any(|character| {
                            character["characterId"] == alias["characterId"]
                                && !character["retired"].as_bool().unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            })
            .map(|alias| {
                let mut alias = alias.clone();
                alias["projectId"] = json!(self.project_id);
                alias
            })
            .collect();
        proof
            .items
            .iter()
            .map(|item| {
                let lookup = json!({
                    "name": item["fields"]["name"],
                    "projectId": self.project_id,
                    "sourceKey": item["sourceId"],
                    "revision": snapshot["revision"],
                });
                let mut resolution = resolve_scoped_character_identity(&aliases, &lookup)?;
                if source["kind"].as_str() != Some("finalized-generation")
                    && matches!(resolution, ScopedCharacterResolution::Unresolved { .. })
                {
                    let fallback = json!({
                        "name": item["fields"]["name"],
                        "projectId": self.project_id,
                        "revision": snapshot["revision"],
                    });
                    resolution = resolve_scoped_character_identity(&aliases, &fallback)?;
                }
                let mut item = item.clone();
                item["resolution"] =
                    serde_json::to_value(&resolution).map_err(|error| error.to_string())?;
                Ok(item)
            })
            .collect()
    }

    /// 事务内的 `read` 主体（调用方持有事务；见 `read`）。
    fn read_in_tx(&self, proposal_batch_id: &str) -> Result<Value, String> {
        let mut envelope = self.read_envelope(proposal_batch_id)?;
        if envelope.batch["status"].as_str() == Some("pending-approval")
            && envelope.batch["revision"].as_i64() != Some(self.revision()?)
        {
            let snapshot = read_identity_snapshot(self.conn)?;
            let revision = snapshot["revision"].as_i64().unwrap_or_default();
            let items =
                self.resolve_items(&envelope.proof, &envelope.batch["source"], &snapshot)?;
            envelope.batch["revision"] = json!(revision);
            envelope.batch["items"] = json!(items);
            self.write(&envelope)?;
        }
        Ok(envelope.batch.clone())
    }

    pub fn read(&self, proposal_batch_id: &str) -> Result<Value, String> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        let result = self.read_in_tx(proposal_batch_id);
        result.and_then(|value| {
            tx.commit()
                .map(|_| value)
                .map_err(|error| error.to_string())
        })
    }

    pub fn list_pending_finalized(&self) -> Result<Vec<Value>, String> {
        let source_key = format!("character-proposal-v1:{}", self.project_id);
        let rows: Vec<String> = self
            .conn
            .prepare(
                "SELECT proposal_id FROM character_identity_proposals WHERE source_key=? ORDER BY proposal_id",
            )
            .map_err(|error| error.to_string())?
            .query_map([source_key], |row| row.get(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let mut summaries = Vec::new();
        for proposal_id in rows {
            // 上游对单个信封的解析失败折叠为空（不中断列表）。
            if let Ok(envelope) = self.read_envelope(&proposal_id) {
                let provenance = envelope.proof.provenance.as_ref();
                let Some(source) = provenance.and_then(|value| value.get("source")) else {
                    continue;
                };
                if envelope.batch["status"].as_str() != Some("pending-approval") {
                    continue;
                }
                match envelope.batch["source"]["kind"].as_str() {
                    Some("import") => summaries.push(json!({
                        "proposalBatchId": proposal_id,
                        "revision": envelope.batch["revision"],
                        "importOperationId": envelope.batch["source"]["operationId"],
                    })),
                    Some("finalized-generation") => summaries.push(json!({
                        "proposalBatchId": proposal_id,
                        "revision": envelope.batch["revision"],
                        "finalizationId": source["sourceId"],
                    })),
                    _ => {}
                }
            }
        }
        Ok(summaries)
    }

    pub fn read_pending_finalized(&self, proposal_batch_id: &str) -> Result<Value, String> {
        let batch = self.read(proposal_batch_id)?;
        if batch["status"].as_str() != Some("pending-approval")
            || !matches!(
                batch["source"]["kind"].as_str(),
                Some("finalized-generation") | Some("import")
            )
        {
            return Err("CHARACTER_PROPOSAL_NOT_PENDING_FINALIZED".to_string());
        }
        Ok(batch)
    }

    pub fn evidence(&self, proposal_batch_id: &str) -> Result<Value, String> {
        let envelope = self.read_envelope(proposal_batch_id)?;
        Ok(json!({
            "proposalBatchId": proposal_batch_id,
            "sourceHash": envelope.proof.source_hash,
        }))
    }

    pub fn stage(&self, source: &Value) -> Result<Value, String> {
        if self.conn.is_autocommit() {
            let tx = self
                .conn
                .unchecked_transaction()
                .map_err(|error| error.to_string())?;
            let result = self.stage_in_transaction(source);
            result.and_then(|value| {
                tx.commit()
                    .map(|_| value)
                    .map_err(|error| error.to_string())
            })
        } else {
            // 调用方已持有事务（上游 `db.transaction()` 内调用 `stage` 的形
            // 态）：语句直接加入调用方事务，不重复 BEGIN（rusqlite 不支持
            // 嵌套事务）。
            self.stage_in_transaction(source)
        }
    }

    /// 与 `stage` 相同的写入逻辑，但由调用方持有事务。rusqlite 事务是
    /// 连接级状态，语句直接在 `self.conn` 上执行并随调用方事务一起
    /// 提交/回滚（对齐上游 `db.transaction(() => stage(...))()` 的原子性）。
    pub fn stage_in_transaction(&self, source: &Value) -> Result<Value, String> {
        let mut source = source.clone();
        if source["kind"].as_str() == Some("legacy-roster-generation") {
            let runs = GenerationRunRepository::default();
            let result =
                read_legacy_roster_generation_proof(self.conn, &runs, self.project_id, &source)?;
            source = result["source"].clone();
            let proposal_batch_id = proposal_batch_id_for(self.project_id, &source);
            if proposal_exists(self.conn, &proposal_batch_id)? {
                return self.read_in_tx(&proposal_batch_id);
            }
        }
        if source["kind"].as_str() == Some("finalized-generation") {
            let runs = GenerationRunRepository::default();
            let normalized = normalize_finalized_character_proposal_source(
                self.conn,
                &runs,
                self.project_id,
                &source,
            )?;
            source = normalized["source"].clone();
            // 仅专用主上下文支持历史 ACK；legacy 种类保留写入门禁。
            if normalized["stableFinalizationSource"]
                .as_bool()
                .unwrap_or(false)
            {
                let proposal_batch_id = proposal_batch_id_for(self.project_id, &source);
                if proposal_exists(self.conn, &proposal_batch_id)? {
                    return self.read_in_tx(&proposal_batch_id);
                }
            }
        }
        let proposal_batch_id = proposal_batch_id_for(self.project_id, &source);
        if proposal_exists(self.conn, &proposal_batch_id)? {
            let envelope = self.read_envelope(&proposal_batch_id)?;
            let version = envelope.architecture_derivation_version.unwrap_or(1) as u8;
            self.source_proof(&source, true, version)?;
            return self.read_in_tx(&proposal_batch_id);
        }
        let architecture_derivation_version = if source["kind"].as_str() == Some("generation")
            && source["inputKind"].as_str() == Some("architecture")
        {
            Some(2)
        } else {
            None
        };
        let version = architecture_derivation_version.unwrap_or(1) as u8;
        let proof = self.source_proof(&source, true, version)?;
        let revision = self.revision()?;
        let snapshot = read_identity_snapshot(self.conn)?;
        let items = self.resolve_items(&proof, &source, &snapshot)?;
        let batch = json!({
            "proposalBatchId": proposal_batch_id,
            "revision": revision,
            "status": "pending-approval",
            "source": source,
            "items": items,
        });
        let envelope = Envelope {
            version: 1,
            architecture_derivation_version,
            project_id: self.project_id.to_string(),
            proof: proof.clone(),
            batch: batch.clone(),
            decision: None,
            created: None,
            approval_proof: None,
            author_edit_proof: None,
        };
        self.assert_finalized_source_current(&envelope)?;
        let encoded = serde_json::to_string(&envelope).map_err(|error| error.to_string())?;
        self.conn
            .execute(
                "INSERT INTO character_identity_proposals(proposal_id,owner_character_id,source_key,source_hash,raw_value,candidate_ids_json) VALUES(?,NULL,?,?,?,?)",
                [
                    proposal_batch_id.clone(),
                    format!("character-proposal-v1:{}", self.project_id),
                    text_hash(&encoded),
                    encoded,
                    "[]".to_string(),
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(batch)
    }

    pub fn cancel(&self, request: &Value) -> Result<Value, String> {
        let valid = request.is_object()
            && request["proposalBatchId"]
                .as_str()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false)
            && safe_integer(&request["expectedRevision"]).is_some()
            && request
                .as_object()
                .map(|fields| {
                    fields
                        .keys()
                        .all(|key| key == "proposalBatchId" || key == "expectedRevision")
                })
                .unwrap_or(false);
        if !valid {
            return Err("CHARACTER_APPROVAL_INVALID".to_string());
        }
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        let result = (|| -> Result<Value, String> {
            let mut envelope =
                self.read_envelope(request["proposalBatchId"].as_str().unwrap_or_default())?;
            if request["expectedRevision"].as_i64() != envelope.batch["revision"].as_i64() {
                return Err("CHARACTER_ID_REVISION_CONFLICT".to_string());
            }
            if envelope.batch["status"].as_str() == Some("approved") {
                return Err("CHARACTER_PROPOSAL_ALREADY_APPROVED".to_string());
            }
            if envelope.batch["status"].as_str() == Some("cancelled") {
                return Ok(envelope.batch.clone());
            }
            envelope.batch["status"] = json!("cancelled");
            self.write(&envelope)?;
            Ok(envelope.batch.clone())
        })();
        result.and_then(|value| {
            tx.commit()
                .map(|_| value)
                .map_err(|error| error.to_string())
        })
    }

    pub fn approve(&self, request: &Value) -> Result<Value, String> {
        let valid = request.is_object()
            && request["operationId"]
                .as_str()
                .map(|value| !value.trim().is_empty() && value.len() <= 256)
                .unwrap_or(false)
            && request["selections"].is_array()
            && request
                .as_object()
                .map(|fields| {
                    fields.keys().all(|key| {
                        matches!(
                            key.as_str(),
                            "proposalBatchId"
                                | "expectedRevision"
                                | "operationId"
                                | "selections"
                                | "relationships"
                                | "edits"
                        )
                    })
                })
                .unwrap_or(false);
        if !valid {
            return Err("CHARACTER_APPROVAL_INVALID".to_string());
        }
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        let result = (|| -> Result<Value, String> {
            let mut envelope =
                self.read_envelope(request["proposalBatchId"].as_str().unwrap_or_default())?;
            let batch = envelope.batch.clone();
            let by_key: BTreeMap<String, Value> = batch["items"]
                .as_array()
                .unwrap_or(&Vec::new())
                .iter()
                .filter_map(|item| {
                    item["selectionKey"]
                        .as_str()
                        .map(|key| (key.to_string(), item.clone()))
                })
                .collect();
            if request.get("edits").is_some() {
                let Some(edits) = request["edits"].as_array() else {
                    return Err("CHARACTER_PROPOSAL_EDIT_INVALID".to_string());
                };
                if batch["source"]["kind"].as_str() != Some("generation")
                    || batch["source"]["inputKind"].as_str() != Some("planning-material")
                    || edits.len() > by_key.len()
                {
                    return Err("CHARACTER_PROPOSAL_EDIT_INVALID".to_string());
                }
            }
            let mut edit_keys: HashSet<String> = HashSet::new();
            let mut normalized_edits: Vec<Value> = Vec::new();
            if let Some(edits) = request["edits"].as_array() {
                for edit in edits {
                    let selection_key = edit["selectionKey"].as_str().unwrap_or_default();
                    let Some(fields) = edit.get("fields") else {
                        return Err("CHARACTER_PROPOSAL_EDIT_INVALID".to_string());
                    };
                    if !edit.is_object()
                        || !edit["selectionKey"].is_string()
                        || edit_keys.contains(selection_key)
                        || !by_key.contains_key(selection_key)
                        || !fields.is_object()
                    {
                        return Err("CHARACTER_PROPOSAL_EDIT_INVALID".to_string());
                    }
                    edit_keys.insert(selection_key.to_string());
                    let field_entries: Vec<(String, Value)> = fields
                        .as_object()
                        .map(|map| {
                            map.iter()
                                .map(|(key, value)| (key.clone(), value.clone()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let invalid = field_entries.is_empty()
                        || field_entries.iter().any(|(key, value)| {
                            !CHARACTER_PROPOSAL_FIELDS.contains(&key.as_str())
                                || !value.is_string()
                                || (key == "name"
                                    && value
                                        .as_str()
                                        .map(|text| text.trim().is_empty())
                                        .unwrap_or(true))
                                || (key == "role"
                                    && !CHARACTER_ROLES
                                        .contains(&value.as_str().unwrap_or_default()))
                        });
                    if invalid {
                        return Err("CHARACTER_PROPOSAL_EDIT_INVALID".to_string());
                    }
                    let mut normalized = Map::new();
                    for field in CHARACTER_PROPOSAL_FIELDS {
                        if let Some(value) = fields.get(field) {
                            normalized.insert(field.to_string(), value.clone());
                        }
                    }
                    normalized_edits.push(json!({
                        "selectionKey": edit["selectionKey"],
                        "fields": Value::Object(normalized),
                    }));
                }
                normalized_edits.sort_by(|left, right| {
                    left["selectionKey"]
                        .as_str()
                        .unwrap_or_default()
                        .cmp(right["selectionKey"].as_str().unwrap_or_default())
                });
            }
            let mut decision = request.clone();
            if request.get("edits").is_some() {
                decision["edits"] = json!(normalized_edits);
            }
            if batch["status"].as_str() == Some("approved") {
                if envelope.decision.as_ref() != Some(&decision) {
                    return Err("CHARACTER_APPROVAL_NONCE_CONFLICT".to_string());
                }
                let created = envelope.created.clone().unwrap_or_else(|| json!([]));
                return Ok(json!({ "batch": batch, "created": created }));
            }
            if batch["status"].as_str() != Some("pending-approval") {
                return Err("CHARACTER_PROPOSAL_CANCELLED".to_string());
            }
            if batch["source"]["kind"].as_str() != Some("legacy-roster-generation") {
                let roster = character_roster::read(self.conn)?;
                if !matches!(
                    roster.status,
                    CharacterRosterStatus::Ready | CharacterRosterStatus::Empty
                ) {
                    return Err("CHARACTER_AUTHOR_ROSTER_NOT_READY".to_string());
                }
            }
            if request["expectedRevision"].as_i64() != envelope.batch["revision"].as_i64()
                || Some(self.revision()?) != envelope.batch["revision"].as_i64()
            {
                return Err("CHARACTER_ID_REVISION_CONFLICT".to_string());
            }
            let proof = self.source_proof(
                &batch["source"],
                true,
                envelope.architecture_derivation_version.unwrap_or(1) as u8,
            )?;
            let Some(provenance) = proof.provenance.clone() else {
                return Err("CHARACTER_PROPOSAL_PROVENANCE_REQUIRED".to_string());
            };
            if !matches!(
                provenance["kind"].as_str(),
                Some("generated") | Some("derived")
            ) {
                return Err("CHARACTER_PROPOSAL_PROVENANCE_REQUIRED".to_string());
            }
            let selections = request["selections"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let unique_selection_keys: HashSet<&str> = selections
                .iter()
                .filter_map(|selection| selection["selectionKey"].as_str())
                .collect();
            if selections.len() != by_key.len() || unique_selection_keys.len() != by_key.len() {
                return Err("CHARACTER_APPROVAL_SELECTION_INVALID".to_string());
            }
            if normalized_edits.iter().any(|edit| {
                let selection_key = edit["selectionKey"].as_str().unwrap_or_default();
                !selections.iter().any(|selection| {
                    selection["selectionKey"].as_str() == Some(selection_key)
                        && selection["action"].as_str() != Some("keep-unresolved")
                })
            }) {
                return Err("CHARACTER_PROPOSAL_EDIT_INVALID".to_string());
            }
            if batch["source"]["kind"].as_str() == Some("legacy-roster-generation")
                && !selections.iter().any(|selection| {
                    matches!(selection["action"].as_str(), Some("create") | Some("map"))
                })
            {
                return Err("CHARACTER_APPROVAL_SELECTION_REQUIRED".to_string());
            }
            let action = if batch["source"]["kind"].as_str() == Some("legacy-roster-generation")
                || (batch["source"]["kind"].as_str() == Some("generation")
                    && batch["source"]["inputKind"].as_str() == Some("architecture"))
            {
                "adopt-generated"
            } else {
                "adopt-import"
            };
            let mut identity_request = json!({
                "approval": {
                    "operationId": request["operationId"],
                    "expectedRevision": request["expectedRevision"],
                    "source": provenance,
                    "action": action,
                },
                "changes": [],
                "creations": [],
                "retireIds": [],
                "relationships": [],
                "resolutions": [],
            });
            let existing_characters = read_identity_snapshot(self.conn)?["characters"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for selection in selections.clone() {
                let selection_key = selection["selectionKey"].as_str().unwrap_or_default();
                let Some(item) = by_key.get(selection_key).cloned() else {
                    return Err("CHARACTER_APPROVAL_SELECTION_INVALID".to_string());
                };
                let selection_action = selection["action"].as_str().unwrap_or_default();
                let keys_valid = selection
                    .as_object()
                    .map(|fields| {
                        fields.keys().all(|key| {
                            key == "selectionKey"
                                || key == "action"
                                || (selection_action == "map" && key == "characterId")
                        })
                    })
                    .unwrap_or(false);
                if !matches!(selection_action, "create" | "map" | "keep-unresolved") || !keys_valid
                {
                    return Err("CHARACTER_APPROVAL_SELECTION_INVALID".to_string());
                }
                if selection_action == "create" {
                    identity_request["creations"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({
                            "selectionKey": item["selectionKey"],
                            "fields": item["fields"],
                        }));
                }
                // 定稿出现携带历史展示标签，不是重命名既有身份的许可。
                if selection_action == "map"
                    && batch["source"]["kind"].as_str() != Some("finalized-generation")
                {
                    let character_id = selection["characterId"].as_str().unwrap_or_default();
                    let Some(existing) = existing_characters
                        .iter()
                        .find(|character| {
                            character["characterId"].as_str() == Some(character_id)
                                && !character["retired"].as_bool().unwrap_or(false)
                        })
                        .cloned()
                    else {
                        return Err("CHARACTER_ID_UNKNOWN".to_string());
                    };
                    let mut adopted = Map::new();
                    for (key, value) in item["fields"]
                        .as_object()
                        .map(|map| {
                            map.iter()
                                .map(|(field, value)| (field.clone(), value.clone()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                    {
                        let current = existing["fields"][key.as_str()]
                            .as_str()
                            .unwrap_or_default();
                        let origin = existing["provenance"]["fields"][key.as_str()]["kind"]
                            .as_str()
                            .map(|value| value.to_string())
                            .or_else(|| {
                                existing["provenance"]["kind"]
                                    .as_str()
                                    .map(|value| value.to_string())
                            })
                            .unwrap_or_default();
                        if current != value.as_str().unwrap_or_default()
                            && (current.trim().is_empty()
                                || (origin != "author" && origin != "legacy"))
                        {
                            adopted.insert(key, value);
                        }
                    }
                    if !adopted.is_empty() {
                        identity_request["changes"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!({
                                "characterId": character_id,
                                "fields": Value::Object(adopted),
                            }));
                    }
                }
                if selection_action == "map"
                    && batch["source"]["kind"].as_str() == Some("finalized-generation")
                    && !existing_characters.iter().any(|character| {
                        character["characterId"].as_str()
                            == Some(selection["characterId"].as_str().unwrap_or_default())
                            && !character["retired"].as_bool().unwrap_or(false)
                    })
                {
                    return Err("CHARACTER_ID_UNKNOWN".to_string());
                }
            }
            for relation in request
                .get("relationships")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default()
            {
                let from = selections.iter().find(|selection| {
                    selection["selectionKey"].as_str() == relation["sourceSelectionKey"].as_str()
                });
                let to = selections.iter().find(|selection| {
                    selection["selectionKey"].as_str() == relation["targetSelectionKey"].as_str()
                });
                let target_character_id = relation["targetCharacterId"].as_str();
                let valid = from.is_some()
                    && from.and_then(|selection| selection["action"].as_str())
                        != Some("keep-unresolved")
                    && to.and_then(|selection| selection["action"].as_str())
                        != Some("keep-unresolved")
                    && to.is_some() == target_character_id.is_some()
                    && by_key
                        .get(
                            from.and_then(|selection| selection["selectionKey"].as_str())
                                .unwrap_or_default(),
                        )
                        .and_then(|item| item["relationships"].as_array())
                        .map(|relationships| {
                            relationships.iter().any(|relationship| {
                                relationship["relation"].as_str() == relation["relation"].as_str()
                                    && match relationship["targetSelectionKey"].as_str() {
                                        Some(target_selection_key) => {
                                            Some(target_selection_key)
                                                == relation["targetSelectionKey"].as_str()
                                        }
                                        None => relationship["targetName"].as_str().is_some(),
                                    }
                            })
                        })
                        .unwrap_or(false);
                if !valid {
                    return Err("CHARACTER_RELATIONSHIP_PROPOSAL_INVALID".to_string());
                }
                let from = from.cloned().unwrap();
                let mut pushed = Map::new();
                if from["action"].as_str() == Some("create") {
                    pushed.insert(
                        "sourceSelectionKey".to_string(),
                        from["selectionKey"].clone(),
                    );
                } else {
                    pushed.insert("sourceCharacterId".to_string(), from["characterId"].clone());
                }
                match to {
                    Some(to) if to["action"].as_str() == Some("create") => {
                        pushed.insert("targetSelectionKey".to_string(), to["selectionKey"].clone());
                    }
                    Some(to) => {
                        pushed.insert("targetCharacterId".to_string(), to["characterId"].clone());
                    }
                    None => {
                        pushed.insert(
                            "targetCharacterId".to_string(),
                            relation["targetCharacterId"].clone(),
                        );
                    }
                }
                pushed.insert("relation".to_string(), relation["relation"].clone());
                identity_request["relationships"]
                    .as_array_mut()
                    .unwrap()
                    .push(Value::Object(pushed));
            }
            let frozen_request = identity_request.to_string();
            let receipt = character_identity_repository::commit_character_identities(
                self.conn,
                &identity_request,
                |candidate| candidate.to_string() == frozen_request,
            )?;
            let created_receipt =
                serde_json::to_value(&receipt.created).map_err(|error| error.to_string())?;
            let mut final_revision = receipt.revision;
            let mut author_changes: Vec<Value> = Vec::new();
            for edit in &normalized_edits {
                let selection_key = edit["selectionKey"].as_str().unwrap_or_default();
                let item = by_key.get(selection_key).cloned().unwrap();
                let mut changed = Map::new();
                for (key, value) in edit["fields"]
                    .as_object()
                    .map(|map| {
                        map.iter()
                            .map(|(field, value)| (field.clone(), value.clone()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
                {
                    if item["fields"][key.as_str()] != value {
                        changed.insert(key, value);
                    }
                }
                if changed.is_empty() {
                    continue;
                }
                let selection = selections
                    .iter()
                    .find(|selection| selection["selectionKey"].as_str() == Some(selection_key))
                    .cloned()
                    .unwrap();
                let character_id = if selection["action"].as_str() == Some("map") {
                    selection["characterId"]
                        .as_str()
                        .map(|value| value.to_string())
                } else {
                    receipt
                        .created
                        .iter()
                        .find(|created| created.selection_key == selection_key)
                        .map(|created| created.character_id.clone())
                };
                let Some(character_id) = character_id else {
                    return Err("CHARACTER_PROPOSAL_EDIT_INVALID".to_string());
                };
                author_changes.push(json!({
                    "characterId": character_id,
                    "fields": Value::Object(changed),
                }));
            }
            if !author_changes.is_empty() {
                if batch["source"]["kind"].as_str() != Some("generation")
                    || batch["source"]["inputKind"].as_str() != Some("planning-material")
                {
                    return Err("CHARACTER_PROPOSAL_EDIT_INVALID".to_string());
                }
                let epoch = batch["source"]["handle"]["epoch"]
                    .as_str()
                    .unwrap_or_default();
                let edits_content_hash = text_hash(
                    &serde_json::to_string(&normalized_edits).map_err(|error| error.to_string())?,
                );
                let author_request = json!({
                    "approval": {
                        "operationId": format!("character-proposal-edit:{}", request["operationId"]),
                        "expectedRevision": receipt.revision,
                        "action": "author-edit",
                        "source": {
                            "kind": "author",
                            "source": {
                                "projectId": self.project_id,
                                "epoch": epoch,
                                "sourceId": format!("character-proposal-edit:{}", proposal_batch_id_for(self.project_id, &batch["source"])),
                                "revision": receipt.revision,
                                "contentHash": edits_content_hash,
                            },
                        },
                    },
                    "changes": author_changes,
                    "creations": [],
                    "retireIds": [],
                    "relationships": [],
                    "resolutions": [],
                });
                let frozen_author_request = author_request.to_string();
                final_revision = character_identity_repository::commit_character_identities(
                    self.conn,
                    &author_request,
                    |candidate| candidate.to_string() == frozen_author_request,
                )?
                .revision;
                let author_operation_id = format!(
                    "character-proposal-edit:{}",
                    request["operationId"].as_str().unwrap_or_default()
                );
                let (author_payload_hash, author_receipt_json) = self
                    .conn
                    .query_row(
                        "SELECT payload_hash,receipt_json FROM character_identity_approvals WHERE operation_id=?",
                        [author_operation_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                            ))
                        },
                    )
                    .map_err(|error| error.to_string())?;
                envelope.author_edit_proof = Some(json!({
                    "payloadHash": author_payload_hash,
                    "receiptHash": text_hash(&author_receipt_json),
                }));
            }
            refresh_character_identity_projection(self.conn)?;
            let mut approved_batch = batch.clone();
            approved_batch["status"] = json!("approved");
            approved_batch["revision"] = json!(final_revision);
            approved_batch["approvalOperationId"] = request["operationId"].clone();
            envelope.batch = approved_batch;
            envelope.decision = Some(decision);
            envelope.created = Some(created_receipt.clone());
            if batch["source"]["kind"].as_str() == Some("legacy-roster-generation") {
                let operation_id = request["operationId"].as_str().unwrap_or_default();
                let (payload_hash, receipt_json) = self
                    .conn
                    .query_row(
                        "SELECT payload_hash,receipt_json FROM character_identity_approvals WHERE operation_id=?",
                        [operation_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                            ))
                        },
                    )
                    .map_err(|error| error.to_string())?;
                envelope.approval_proof = Some(json!({
                    "payloadHash": payload_hash,
                    "receiptHash": text_hash(&receipt_json),
                }));
            }
            self.write(&envelope)?;
            Ok(json!({
                "batch": envelope.batch.clone(),
                "created": created_receipt,
            }))
        })();
        result.and_then(|value| {
            tx.commit()
                .map(|_| value)
                .map_err(|error| error.to_string())
        })
    }

    pub fn identity_snapshot(&self) -> Result<Value, String> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        let result = read_identity_snapshot(self.conn);
        result.and_then(|value| {
            tx.commit()
                .map(|_| value)
                .map_err(|error| error.to_string())
        })
    }
}

/// `findCharacterProposalEvidence` —— 只查精确的持久化来源，
/// 绝不回退到最新同名或最新提案。
pub fn find_character_proposal_evidence(
    conn: &Connection,
    source: &Value,
    prove: impl Fn(&str, &Value) -> Result<CharacterProposalProof, String>,
) -> Result<Value, String> {
    let mut source = source.clone();
    if source["kind"].as_str() == Some("finalized-generation") {
        let runs = GenerationRunRepository::default();
        let project_id = source["handle"]["projectId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        source = normalize_finalized_character_proposal_source(conn, &runs, &project_id, &source)?
            ["source"]
            .clone();
    }
    let rows: Vec<String> = conn
        .prepare(
            "SELECT raw_value FROM character_identity_proposals WHERE source_key LIKE 'character-proposal-v1:%'",
        )
        .map_err(|error| error.to_string())?
        .query_map([], |row| row.get(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut matches = Vec::new();
    for raw_value in rows {
        let envelope: Value =
            serde_json::from_str(&raw_value).map_err(|error| error.to_string())?;
        if envelope["batch"]["source"] == source {
            matches.push(envelope);
        }
    }
    if matches.len() != 1 {
        return Err("CHARACTER_PROPOSAL_EVIDENCE_REQUIRED".to_string());
    }
    let envelope = matches.into_iter().next().unwrap();
    let project_id = envelope["projectId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let service =
        CharacterProposalService::new(conn, &project_id, |candidate, _for_write, _version| {
            prove(&project_id, candidate)
        });
    let proposal_batch_id = envelope["batch"]["proposalBatchId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    service.evidence(&proposal_batch_id)
}

/// `proveCharacterProposal` —— 五分支证明结构。
///
/// 未移植叶函数以 [`PENDING_MIGRATION_CODE`] 显式桩住（`directory` 分支完整可用）。
#[allow(clippy::too_many_arguments)]
pub fn prove_character_proposal(
    conn: &Connection,
    runs: &GenerationRunRepository,
    project_id: &str,
    source: &Value,
    for_write: bool,
    assert_sources: impl Fn(&Value, Option<&[i64]>) -> Result<(), String>,
    architecture_derivation_version: u8,
) -> Result<CharacterProposalProof, String> {
    if !source.is_object() {
        return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
    }
    match source["kind"].as_str() {
        Some("legacy-roster-generation") => {
            let result = read_legacy_roster_generation_proof(conn, runs, project_id, source)?;
            if for_write {
                assert_sources(&result["currentHandle"], None)?;
            }
            let proof = result["proof"].clone();
            Ok(CharacterProposalProof {
                items: proof["items"].as_array().cloned().unwrap_or_default(),
                source_hash: proof["sourceHash"].as_str().unwrap_or_default().to_string(),
                provenance: (!proof["provenance"].is_null()).then(|| proof["provenance"].clone()),
            })
        }
        Some("finalized-generation") => {
            if !keys_subset(source, &["kind", "handle", "artifact"]) {
                return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
            }
            // TODO(B1 后续批次)：移植 `proveFinalizedCharacterGeneration`
            // （依赖 `parseFinalizedCharacterStateResponse` / `FinalizedCharacterContext`
            // 等定稿连续性叶函数，尚未迁移）。
            Err(PENDING_MIGRATION_CODE.to_string())
        }
        Some("import") => {
            if !keys_subset(source, &["kind", "operationId"]) {
                return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
            }
            let operation_id = source["operationId"].as_str().unwrap_or_default();
            let Some((payload_hash, receipt_json)) = conn
                .query_row(
                    "SELECT payload_hash,receipt_json FROM import_global_fact_operations WHERE operation_id=?",
                    [operation_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                        ))
                    },
                )
                .optional()
                .map_err(|error| error.to_string())?
            else {
                return Err("CHARACTER_PROPOSAL_SOURCE_MISSING".to_string());
            };
            let receipt: Value =
                serde_json::from_str(&receipt_json).map_err(|error| error.to_string())?;
            if receipt["proposalSource"].is_null()
                || receipt["payloadHash"].as_str() != Some(payload_hash.as_str())
                || receipt["operationId"].as_str() != Some(operation_id)
                || text_hash(
                    &serde_json::to_string(&receipt["proposalSource"])
                        .map_err(|error| error.to_string())?,
                ) != payload_hash
            {
                return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
            }
            let character_entries = receipt["proposalSource"]["characterEntries"].clone();
            let entries = character_entries
                .as_array()
                .ok_or_else(|| "CHARACTER_PROPOSAL_SOURCE_INVALID".to_string())?;
            let items: Vec<Value> = entries
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    let mut fields = entry.clone();
                    let relationships = fields.get("relationships").cloned();
                    fields.as_object_mut().map(|map| {
                        map.remove("relationships");
                        map.remove("currentState");
                        map.remove("legacyRelationshipNotes");
                    });
                    let relationships =
                        relationships
                            .as_ref()
                            .and_then(Value::as_array)
                            .cloned()
                            .ok_or_else(|| "CHARACTER_PROPOSAL_SOURCE_INVALID".to_string())?;
                    let mapped_relationships: Vec<Value> = relationships
                        .iter()
                        .map(|relation| {
                            json!({
                                "targetName": relation["target"],
                                "relation": relation["relation"],
                                "raw": relation.clone(),
                            })
                        })
                        .collect();
                    Ok(json!({
                        "selectionKey": format!("import:{index}"),
                        "sourceId": format!("{operation_id}:{index}"),
                        "fields": fields,
                        "relationships": mapped_relationships,
                        "rawValue": entry.clone(),
                    }))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let source_hash = text_hash(
                &serde_json::to_string(&json!([payload_hash, character_entries]))
                    .map_err(|error| error.to_string())?,
            );
            let effects: Vec<Value> = {
                let mut statement = conn
                    .prepare(
                        "SELECT run_id,batch_id,state,payload_json,payload_hash FROM import_run_receipts \
                         WHERE stage='global' AND kind='project-global-facts' \
                           AND json_extract(payload_json,'$.operationId')=?",
                    )
                    .map_err(|error| error.to_string())?;
                let rows = statement
                    .query_map([operation_id], |row| {
                        Ok(json!({
                            "run_id": row.get::<_, String>(0)?,
                            "batch_id": row.get::<_, String>(1)?,
                            "state": row.get::<_, String>(2)?,
                            "payload_json": row.get::<_, String>(3)?,
                            "payload_hash": row.get::<_, String>(4)?,
                        }))
                    })
                    .map_err(|error| error.to_string())?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?
            };
            if effects.len() != 1 {
                return Ok(CharacterProposalProof {
                    items,
                    source_hash,
                    provenance: None,
                });
            }
            let effect = &effects[0];
            let payload: Value =
                serde_json::from_str(effect["payload_json"].as_str().unwrap_or_default())
                    .map_err(|error| error.to_string())?;
            let handle = payload["generationRunHandle"].clone();
            if text_hash(effect["payload_json"].as_str().unwrap_or_default())
                != effect["payload_hash"].as_str().unwrap_or_default()
                || payload["characterEntries"] != character_entries
            {
                return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
            }
            if handle.is_null() {
                return Ok(CharacterProposalProof {
                    items,
                    source_hash,
                    provenance: None,
                });
            }
            let run = runs.get(conn, handle["runId"].as_str().unwrap_or_default())?;
            if handle["projectId"].as_str() != Some(project_id)
                || run.binding.project_id != project_id
                || handle["rootActionId"].as_str() != Some(run.root_action_id.as_str())
                || run
                    .binding
                    .source_manifest
                    .get("operation")
                    .and_then(Value::as_str)
                    != Some("import-global-facts")
                || run.binding.source_manifest.get("importSlot")
                    != Some(&json!({
                        "runId": effect["run_id"],
                        "stage": "global",
                        "batchId": effect["batch_id"],
                    }))
            {
                return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
            }
            if for_write && effect["state"].as_str() != Some("committed") {
                return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
            }
            let generated = import_character_entries_from_artifacts(conn, runs, &run.run_id)?;
            let Some(generated) = generated else {
                return Ok(CharacterProposalProof {
                    items,
                    source_hash,
                    provenance: None,
                });
            };
            if generated != character_entries {
                return Ok(CharacterProposalProof {
                    items,
                    source_hash,
                    provenance: None,
                });
            }
            let model_revision = run
                .binding
                .source_manifest
                .get("modelReceipt")
                .and_then(|receipt| receipt.get("modelRevision"))
                .and_then(Value::as_str)
                .map(|value| value.to_string());
            let Some(model_revision) = model_revision else {
                return Err("CHARACTER_PROPOSAL_PROVENANCE_REQUIRED".to_string());
            };
            if model_revision.len() != 64
                || !model_revision
                    .chars()
                    .all(|character| matches!(character, '0'..='9' | 'a'..='f'))
            {
                return Err("CHARACTER_PROPOSAL_PROVENANCE_REQUIRED".to_string());
            }
            let provenance = json!({
                "kind": "generated",
                "modelRevision": model_revision,
                "source": {
                    "projectId": project_id,
                    "epoch": handle["epoch"],
                    "sourceId": source["operationId"],
                    "revision": 0,
                    "contentHash": source_hash,
                },
            });
            Ok(CharacterProposalProof {
                items,
                source_hash,
                provenance: Some(provenance),
            })
        }
        Some("directory") => {
            if !keys_subset(source, &["kind", "operationId"]) || !source["operationId"].is_string()
            {
                return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
            }
            let operation_id = source["operationId"].as_str().unwrap_or_default();
            let Some(operation) = conn
                .query_row(
                    "SELECT c.operation_id,c.payload_hash,c.character_sync_input,s.blueprint_commit_payload_hash,s.character_sync_input AS sync_input \
                     FROM blueprint_character_sync_operations s \
                     JOIN blueprint_commit_operations c ON c.operation_id=s.blueprint_commit_operation_id \
                     WHERE s.operation_id=?",
                    [operation_id],
                    |row| {
                        Ok(json!({
                            "operation_id": row.get::<_, String>(0)?,
                            "payload_hash": row.get::<_, String>(1)?,
                            "character_sync_input": row.get::<_, String>(2)?,
                            "blueprint_commit_payload_hash": row.get::<_, String>(3)?,
                            "sync_input": row.get::<_, String>(4)?,
                        }))
                    },
                )
                .optional()
                .map_err(|error| error.to_string())?
            else {
                return Err("CHARACTER_PROPOSAL_SOURCE_MISSING".to_string());
            };
            if operation["blueprint_commit_payload_hash"] != operation["payload_hash"]
                || operation["sync_input"] != operation["character_sync_input"]
            {
                return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
            }
            let blueprints: Value = serde_json::from_str(
                operation["character_sync_input"]
                    .as_str()
                    .unwrap_or_default(),
            )
            .map_err(|error| error.to_string())?;
            let Some(blueprints) = blueprints.as_array() else {
                return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
            };
            if for_write {
                for blueprint in blueprints {
                    let chapter_number = blueprint["chapterNumber"].as_i64().unwrap_or_default();
                    let Some(current) = conn
                        .query_row(
                            "SELECT title,role,purpose,key_events,characters,suspense_hook,user_guidance FROM blueprints WHERE chapter_number=?",
                            [chapter_number],
                            |row| {
                                Ok(json!({
                                    "title": row.get::<_, String>(0)?,
                                    "role": row.get::<_, String>(1)?,
                                    "purpose": row.get::<_, String>(2)?,
                                    "key_events": row.get::<_, String>(3)?,
                                    "characters": row.get::<_, String>(4)?,
                                    "suspense_hook": row.get::<_, String>(5)?,
                                    "user_guidance": row.get::<_, String>(6)?,
                                }))
                            },
                        )
                        .optional()
                        .map_err(|error| error.to_string())?
                    else {
                        return Err("CHARACTER_PROPOSAL_SOURCE_CHANGED".to_string());
                    };
                    let current_characters: Value =
                        serde_json::from_str(current["characters"].as_str().unwrap_or_default())
                            .map_err(|error| error.to_string())?;
                    let current_view = json!([
                        current["title"],
                        current["role"],
                        current["purpose"],
                        current["key_events"],
                        current_characters,
                        current["suspense_hook"],
                        current["user_guidance"],
                    ]);
                    let blueprint_view = json!([
                        blueprint["title"],
                        blueprint["role"],
                        blueprint["purpose"],
                        blueprint["keyEvents"],
                        blueprint["characters"],
                        blueprint["suspenseHook"],
                        blueprint["userGuidance"],
                    ]);
                    if current_view != blueprint_view {
                        return Err("CHARACTER_PROPOSAL_SOURCE_CHANGED".to_string());
                    }
                }
            }
            let mut items: Vec<Value> = Vec::new();
            for blueprint in blueprints {
                let chapter_number = blueprint["chapterNumber"].as_i64().unwrap_or_default();
                let candidates = blueprint["newCharacterCandidates"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let characters: Vec<Value> = blueprint["characters"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let hints: Vec<Value> = blueprint["relationshipHints"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                for (index, candidate) in candidates.iter().enumerate() {
                    let name = candidate["name"].as_str().unwrap_or_default();
                    let role = candidate["role"].as_str().unwrap_or_default();
                    if name.trim().is_empty()
                        || !CHARACTER_ROLES.contains(&role)
                        || !characters
                            .iter()
                            .any(|character| character.as_str() == Some(name))
                    {
                        return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
                    }
                    let relationships: Vec<Value> = hints
                        .iter()
                        .filter(|relation| {
                            relation.is_object() && relation["from"].as_str() == Some(name)
                        })
                        .map(|relation| {
                            let mut mapped = Map::new();
                            if let Some(target) = relation["to"].as_str() {
                                mapped.insert("targetName".to_string(), json!(target));
                            }
                            let relation_value = if relation["relation"].is_string() {
                                relation["relation"].clone()
                            } else {
                                json!("")
                            };
                            mapped.insert("relation".to_string(), relation_value);
                            mapped.insert("raw".to_string(), relation.clone());
                            Value::Object(mapped)
                        })
                        .collect();
                    items.push(json!({
                        "selectionKey": format!("{chapter_number}:{index}"),
                        "sourceId": format!("{operation_id}:{chapter_number}:{index}"),
                        "fields": { "name": name, "role": role },
                        "relationships": relationships,
                        "rawValue": {
                            "candidate": candidate.clone(),
                            "characters": blueprint["characters"].clone(),
                            "relationshipHints": blueprint["relationshipHints"].clone(),
                        },
                    }));
                }
            }
            let progress = runs
                .list_directory_progress(conn)?
                .into_iter()
                .find(|item| item.operation_id == operation_id);
            if let Some(progress) = &progress {
                if progress.source_handle.project_id != project_id {
                    return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
                }
                if for_write {
                    let chapter_numbers: Vec<i64> = blueprints
                        .iter()
                        .map(|blueprint| blueprint["chapterNumber"].as_i64().unwrap_or_default())
                        .collect();
                    assert_sources(
                        &json!(progress.source_handle),
                        Some(chapter_numbers.as_slice()),
                    )?;
                }
            }
            let model_revision = progress
                .as_ref()
                .map(|progress| runs.get(conn, &progress.source_handle.run_id))
                .transpose()?
                .and_then(|run| run.binding.source_manifest.get("modelReceipt").cloned())
                .and_then(|receipt| {
                    receipt["modelRevision"]
                        .as_str()
                        .map(|value| value.to_string())
                });
            let source_hash = text_hash(
                &serde_json::to_string(&json!([
                    operation["payload_hash"],
                    operation["character_sync_input"],
                ]))
                .map_err(|error| error.to_string())?,
            );
            let provenance = match (&progress, model_revision) {
                (Some(progress), Some(model_revision)) => Some(json!({
                    "kind": "generated",
                    "modelRevision": model_revision,
                    "source": {
                        "projectId": project_id,
                        "epoch": progress.source_handle.epoch.clone(),
                        "sourceId": operation["operation_id"].clone(),
                        "revision": 0,
                        "contentHash": source_hash,
                    },
                })),
                _ => None,
            };
            Ok(CharacterProposalProof {
                items,
                source_hash,
                provenance,
            })
        }
        Some("generation") => {
            let input_kind = source["inputKind"].as_str();
            let Some(artifacts_refs) = source["artifacts"].as_array() else {
                return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
            };
            let artifact_ids: HashSet<&str> = artifacts_refs
                .iter()
                .filter_map(|reference| reference["artifactId"].as_str())
                .collect();
            if !matches!(input_kind, Some("architecture") | Some("planning-material"))
                || !keys_subset(
                    source,
                    &[
                        "kind",
                        "inputKind",
                        "handle",
                        "artifacts",
                        "manifestArtifactId",
                    ],
                )
                || artifacts_refs.is_empty()
                || artifacts_refs.len() > 64
                || artifact_ids.len() != artifacts_refs.len()
            {
                return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
            }
            let run = runs.get(conn, source["handle"]["runId"].as_str().unwrap_or_default())?;
            if run.binding.project_id != project_id
                || source["handle"]["projectId"].as_str() != Some(project_id)
                || run.root_action_id
                    != source["handle"]["rootActionId"]
                        .as_str()
                        .unwrap_or_default()
            {
                return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
            }
            if for_write {
                assert_sources(&source["handle"], None)?;
            }
            let artifacts: Vec<Value> = artifacts_refs
                .iter()
                .map(|reference| {
                    let Some((artifact_run_id, attempt_id, status)) = conn
                        .query_row(
                            "SELECT run_id,attempt_id,status FROM generation_artifacts WHERE artifact_id=?",
                            [reference["artifactId"].as_str().unwrap_or_default()],
                            |row| {
                                Ok((
                                    row.get::<_, String>(0)?,
                                    row.get::<_, String>(1)?,
                                    row.get::<_, String>(2)?,
                                ))
                            },
                        )
                        .optional()
                        .map_err(|error| error.to_string())?
                    else {
                        return Err("CHARACTER_PROPOSAL_ARTIFACT_INVALID".to_string());
                    };
                    if artifact_run_id != run.run_id
                        || status == "discarded"
                        || !keys_subset(reference, &["artifactId", "revision", "textHash"])
                    {
                        return Err("CHARACTER_PROPOSAL_ARTIFACT_INVALID".to_string());
                    }
                    let receipt = runs.receipt(conn, &attempt_id, None)?;
                    let Some(artifact) = receipt.artifact else {
                        return Err("CHARACTER_PROPOSAL_ARTIFACT_INVALID".to_string());
                    };
                    if reference["revision"].as_i64() != Some(artifact.revision)
                        || reference["textHash"].as_str() != Some(artifact.text_hash.as_str())
                        || !matches!(receipt.attempt.status.as_str(), "settled" | "unknown")
                        || receipt
                            .result
                            .as_ref()
                            .and_then(|result| result.finish_reason.as_deref())
                            != Some("stop")
                    {
                        return Err("CHARACTER_PROPOSAL_ARTIFACT_INVALID".to_string());
                    }
                    Ok(json!({
                        "artifactId": artifact.artifact_id.clone(),
                        "text": artifact.text.clone(),
                    }))
                })
                .collect::<Result<Vec<_>, String>>()?;
            if input_kind == Some("architecture") {
                let manifest_artifact_id = source["manifestArtifactId"].as_str();
                let manifest = artifacts
                    .iter()
                    .find(|artifact| artifact["artifactId"].as_str() == manifest_artifact_id);
                if manifest.is_none()
                    || run
                        .binding
                        .source_manifest
                        .get("operation")
                        .and_then(Value::as_str)
                        != Some("character-architecture")
                {
                    return Err("CHARACTER_PROPOSAL_MANIFEST_INVALID".to_string());
                }
                let details: Vec<Value> = artifacts
                    .iter()
                    .filter(|artifact| artifact["artifactId"].as_str() != manifest_artifact_id)
                    .cloned()
                    .collect();
                // TODO(B1 后续批次)：移植 `parseArchitectureCharacterProposal`
                // （`character-proposal-parser.ts`，依赖 ArchitectureDerivationVersion
                // 结构化解析，尚未迁移）。
                let items = parse_architecture_character_proposal(
                    manifest.unwrap(),
                    &details,
                    architecture_derivation_version,
                )?;
                return generated_proof(project_id, source, &run, items);
            }
            if source.get("manifestArtifactId").is_some()
                || run
                    .binding
                    .source_manifest
                    .get("operation")
                    .and_then(Value::as_str)
                    != Some("planning-material-character-extraction")
            {
                return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
            }
            let author_inputs = run
                .binding
                .source_manifest
                .get("authorInputs")
                .and_then(Value::as_array)
                .ok_or_else(|| "CHARACTER_PROPOSAL_SOURCE_INVALID".to_string())?;
            let mut materials: Vec<Value> = Vec::new();
            for (index, input) in author_inputs.iter().enumerate() {
                if input["id"].as_str() != Some(format!("planning-material:{index}").as_str()) {
                    return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
                }
                let material: Value =
                    serde_json::from_str(input["text"].as_str().unwrap_or_default())
                        .map_err(|error| error.to_string())?;
                if !material["fileName"].is_string() || !material["text"].is_string() {
                    return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
                }
                materials.push(material);
            }
            // TODO(B1 后续批次)：移植 `characterProposalMaterialChunks` 与
            // `parsePlanningMaterialCharacterProposal`（`character-proposal-parser.ts`，
            // 尚未迁移）。
            let items =
                character_proposal_material_chunks(&materials).and_then(|expected_source_ids| {
                    parse_planning_material_character_proposal(&artifacts, &expected_source_ids)
                })?;
            generated_proof(project_id, source, &run, items)
        }
        _ => Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string()),
    }
}

/// generation 分支收口：上游 architecture / planning-material 两子分支共用的
/// `sourceHash` 与 `provenance` 构造（`text_hash([source, items])` +
/// `sourceManifest.modelReceipt.modelRevision`）。
fn generated_proof(
    project_id: &str,
    source: &Value,
    run: &DurableGenerationRun,
    items: Vec<Value>,
) -> Result<CharacterProposalProof, String> {
    let source_hash = text_hash(
        &serde_json::to_string(&json!([source, items])).map_err(|error| error.to_string())?,
    );
    let model_revision = run
        .binding
        .source_manifest
        .get("modelReceipt")
        .and_then(|model| model.get("modelRevision"))
        .cloned()
        .unwrap_or(Value::Null);
    let provenance = json!({
        "kind": "generated",
        "modelRevision": model_revision,
        "source": {
            "projectId": project_id,
            "epoch": source["handle"]["epoch"],
            "sourceId": run.run_id,
            "revision": 0,
            "contentHash": source_hash,
        },
    });
    Ok(CharacterProposalProof {
        items,
        source_hash,
        provenance: Some(provenance),
    })
}

/// 上游 `Object.keys(source).some(key => !allowed.includes(key))` 的等价物。
fn keys_subset(source: &Value, allowed: &[&str]) -> bool {
    source
        .as_object()
        .map(|fields| {
            fields
                .keys()
                .all(|key| allowed.iter().any(|allowed| *allowed == key.as_str()))
        })
        .unwrap_or(false)
}

fn proposal_batch_id_for(project_id: &str, source: &Value) -> String {
    let encoded = json!([project_id, source]).to_string();
    format!("cpb:{}", text_hash(&encoded))
}

fn proposal_exists(conn: &Connection, proposal_batch_id: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT 1 FROM character_identity_proposals WHERE proposal_id=?",
        [proposal_batch_id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map(|row| row.is_some())
    .map_err(|error| error.to_string())
}

/// `readFinalizedCharacterContext` —— 待移植桩（依赖
/// `electron/repositories/summary-repository.ts` 的 `readFinalizedSourceFromDb` /
/// `readFrozenCharacterSnapshot` / `stateSnapshot`，尚未迁移）。
fn read_finalized_character_context(
    _conn: &Connection,
    _draft_id: i64,
    _scope: &Value,
) -> Result<Value, String> {
    // TODO(B1 后续批次)：平移 summary-repository.ts 的定稿上下文读取。
    Err(PENDING_MIGRATION_CODE.to_string())
}

/// `readLegacyRosterGenerationProof` —— 待移植桩（依赖
/// `parseLegacyRosterJson` / `validateLegacyRosterCandidate` /
/// `readLegacyRosterGenerationContext`，尚未迁移）。键集约束先行校验，
/// 保持上游失败语义。
fn read_legacy_roster_generation_proof(
    _conn: &Connection,
    _runs: &GenerationRunRepository,
    _project_id: &str,
    source: &Value,
) -> Result<Value, String> {
    // TODO(B1 后续批次)：平移 legacy-roster-generation-proof.ts。
    if source["kind"].as_str() != Some("legacy-roster-generation")
        || !keys_subset(source, &["kind", "handle", "artifact"])
    {
        return Err("GENERATION_LEGACY_PROOF_INVALID".to_string());
    }
    Err(PENDING_MIGRATION_CODE.to_string())
}

/// `normalizeFinalizedCharacterProposalSource` —— 待移植桩（依赖
/// `proveFinalizedCharacterGeneration`，尚未迁移）。
fn normalize_finalized_character_proposal_source(
    _conn: &Connection,
    _runs: &GenerationRunRepository,
    _project_id: &str,
    source: &Value,
) -> Result<Value, String> {
    // TODO(B1 后续批次)：平移 finalized-character-generation-proof.ts。
    if source["kind"].as_str() != Some("finalized-generation")
        || !keys_subset(source, &["kind", "handle", "artifact"])
    {
        return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
    }
    Err(PENDING_MIGRATION_CODE.to_string())
}

/// `parseArchitectureCharacterProposal` —— 待移植桩。
fn parse_architecture_character_proposal(
    _manifest: &Value,
    _details: &[Value],
    _version: u8,
) -> Result<Vec<Value>, String> {
    // TODO(B1 后续批次)：平移 character-proposal-parser.ts。
    Err(PENDING_MIGRATION_CODE.to_string())
}

/// `parsePlanningMaterialCharacterProposal` —— 待移植桩。
fn parse_planning_material_character_proposal(
    _artifacts: &[Value],
    _expected_source_ids: &[Value],
) -> Result<Vec<Value>, String> {
    // TODO(B1 后续批次)：平移 character-proposal-parser.ts。
    Err(PENDING_MIGRATION_CODE.to_string())
}

/// `characterProposalMaterialChunks` —— 待移植桩。
fn character_proposal_material_chunks(_materials: &[Value]) -> Result<Vec<Value>, String> {
    // TODO(B1 后续批次)：平移 character-proposal-parser.ts。
    Err(PENDING_MIGRATION_CODE.to_string())
}

/// `importCharacterEntriesFromArtifacts` —— 推理产物回放。
///
/// 产物校验部分已平移；`preservesStructuredJsonEvidence` /
/// `decodeImportInferenceJson` / `decodeImportInferenceWithEndpointDelta`
/// 待移植，以 [`PENDING_MIGRATION_CODE`] 桩住。
fn import_character_entries_from_artifacts(
    conn: &Connection,
    runs: &GenerationRunRepository,
    run_id: &str,
) -> Result<Option<Value>, String> {
    let rows: Vec<(String, String)> = {
        let mut statement = conn
            .prepare(
                "SELECT t.attempt_id,json_extract(t.usage_receipt_json,'$.purpose') AS purpose \
                 FROM generation_attempts t JOIN generation_artifacts a ON a.attempt_id=t.attempt_id \
                 WHERE t.run_id=? AND a.status<>'discarded' ORDER BY t.rowid",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([run_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?
    };
    let inference: Vec<&(String, String)> = rows
        .iter()
        .filter(|(_, purpose)| purpose == "import-inference")
        .collect();
    let syntax: Vec<&(String, String)> = rows
        .iter()
        .filter(|(_, purpose)| purpose == "import-inference:structured-syntax-repair")
        .collect();
    let endpoint: Vec<&(String, String)> = rows
        .iter()
        .filter(|(_, purpose)| purpose == "import-inference:endpoint-card-recovery")
        .collect();
    if inference.len() != 1 || syntax.len() > 1 || endpoint.len() > 1 {
        return Ok(None);
    }
    let output = |row: &(String, String)| -> Result<String, String> {
        let receipt = runs.receipt(conn, &row.0, None)?;
        let Some(artifact) = receipt.artifact else {
            return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
        };
        if receipt.failure_code.is_some()
            || receipt
                .result
                .as_ref()
                .and_then(|result| result.finish_reason.as_deref())
                != Some("stop")
            || !matches!(receipt.attempt.status.as_str(), "settled" | "unknown")
            || text_hash(&artifact.text) != artifact.text_hash
        {
            return Err("CHARACTER_PROPOSAL_SOURCE_INVALID".to_string());
        }
        Ok(artifact.text)
    };
    let original = output(inference[0])?;
    if !syntax.is_empty() || !endpoint.is_empty() {
        // TODO(B1 后续批次)：移植 `preservesStructuredJsonEvidence`（语法修复证据保持）
        // 与 `decodeImportInferenceWithEndpointDelta`（端点卡片增量解码）。
        let _ = original;
        return Err(PENDING_MIGRATION_CODE.to_string());
    }
    // TODO(B1 后续批次)：移植 `decodeImportInferenceJson`（import-inference-contract）。
    let _ = original;
    Err(PENDING_MIGRATION_CODE.to_string())
}

/// `resolveScopedCharacterIdentity` —— 精确、作用域化、版本化的别名解析
/// （姓名本身绝不是身份证明）。
fn resolve_scoped_character_identity(
    aliases: &[Value],
    lookup: &Value,
) -> Result<ScopedCharacterResolution, String> {
    let nonempty = |value: &Value| {
        value
            .as_str()
            .map(|text| !text.trim().is_empty())
            .unwrap_or(false)
    };
    let is_revision = |value: &Value| safe_integer(value).is_some_and(|revision| revision >= 0);
    if !nonempty(&lookup["name"])
        || !nonempty(&lookup["projectId"])
        || !is_revision(&lookup["revision"])
        || (lookup.get("sourceKey").is_some() && !nonempty(&lookup["sourceKey"]))
        || !aliases.iter().all(|alias| {
            nonempty(&alias["characterId"])
                && nonempty(&alias["name"])
                && nonempty(&alias["projectId"])
                && nonempty(&alias["sourceKey"])
                && is_revision(&alias["validFrom"])
                && (alias["validThrough"].is_null()
                    || (is_revision(&alias["validThrough"])
                        && alias["validThrough"].as_i64().unwrap_or_default()
                            >= alias["validFrom"].as_i64().unwrap_or_default()))
        })
    {
        return Err("INVALID_CHARACTER_LOOKUP".to_string());
    }
    let name = lookup["name"].as_str().unwrap_or_default();
    let project_id = lookup["projectId"].as_str().unwrap_or_default();
    let revision = lookup["revision"].as_i64().unwrap_or_default();
    let source_key = lookup.get("sourceKey").and_then(|value| value.as_str());
    let mut candidates: Vec<String> = aliases
        .iter()
        .filter(|alias| {
            alias["name"].as_str() == Some(name)
                && alias["projectId"].as_str() == Some(project_id)
                && alias["validFrom"].as_i64().unwrap_or_default() <= revision
                && (alias["validThrough"].is_null()
                    || alias["validThrough"].as_i64().unwrap_or_default() >= revision)
                && (source_key.is_none() || alias["sourceKey"].as_str() == source_key)
        })
        .filter_map(|alias| alias["characterId"].as_str().map(|value| value.to_string()))
        .collect();
    candidates.sort();
    candidates.dedup();
    if candidates.len() > 1 {
        return Ok(ScopedCharacterResolution::Ambiguous {
            candidate_ids: candidates,
        });
    }
    if candidates.len() == 1 && source_key.is_some() {
        return Ok(ScopedCharacterResolution::Resolved {
            character_id: candidates.into_iter().next().unwrap(),
        });
    }
    Ok(ScopedCharacterResolution::Unresolved {
        candidate_ids: candidates,
    })
}

/// `readCharacterIdentitySnapshot` 的服务层组装（对齐基线 `identitySnapshot`：
/// `SELECT *` 原始行 → 驼峰快照）。事务内调用时经 `self.conn` 读取，
/// 与基线嵌套事务（savepoint）语义等价。
fn read_identity_snapshot(conn: &Connection) -> Result<Value, String> {
    let revision: i64 = conn
        .query_row(
            "SELECT revision FROM character_identity_meta WHERE id='main'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let characters: Vec<Value> = {
        let mut statement = conn
            .prepare("SELECT * FROM characters ORDER BY character_id")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                let mut fields = Map::new();
                for field in CHARACTER_PROPOSAL_FIELDS {
                    let value = row.get::<_, Option<String>>(field)?.unwrap_or_default();
                    fields.insert(field.to_string(), json!(value));
                }
                Ok((
                    row.get::<_, String>("character_id")?,
                    Value::Object(fields),
                    row.get::<_, i64>("retired")? != 0,
                    row.get::<_, i64>("identity_revision")?,
                    row.get::<_, String>("static_provenance")?,
                ))
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| -> Result<Value, String> {
            let (character_id, fields, retired, revision, static_provenance) =
                row.map_err(|error| error.to_string())?;
            Ok(json!({
                "characterId": character_id,
                "fields": fields,
                "retired": retired,
                "revision": revision,
                "provenance": serde_json::from_str::<Value>(&static_provenance)
                    .map_err(|error| error.to_string())?,
            }))
        })
        .collect::<Result<Vec<_>, _>>()?
    };
    let aliases: Vec<Value> = {
        let mut statement = conn
            .prepare("SELECT * FROM character_aliases ORDER BY character_id,name,valid_from")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(json!({
                    "characterId": row.get::<_, String>("character_id")?,
                    "name": row.get::<_, String>("name")?,
                    "sourceKey": row.get::<_, String>("source_key")?,
                    "validFrom": row.get::<_, i64>("valid_from")?,
                    "validThrough": row.get::<_, Option<i64>>("valid_through")?,
                }))
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?
    };
    let relationships: Vec<Value> = {
        let mut statement = conn
            .prepare("SELECT * FROM character_relationships ORDER BY relationship_id")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(json!({
                    "relationshipId": row.get::<_, String>("relationship_id")?,
                    "sourceCharacterId": row.get::<_, String>("source_character_id")?,
                    "targetCharacterId": row.get::<_, String>("target_character_id")?,
                    "relation": row.get::<_, String>("relation")?,
                }))
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?
    };
    Ok(json!({
        "revision": revision,
        "characters": characters,
        "aliases": aliases,
        "relationships": relationships,
    }))
}

/// `refreshCharacterIdentityProjection` —— 身份 schema 下的角色名单投影刷新。
///
/// 基线该函数位于 `character-roster-repository.ts`，Rust 侧尚未实现
/// （`character_roster_repository` 仅承载非身份 schema 的名单域），故在本文件
/// 内移植为私有函数。上游的 `!db.inTransaction` 前置检查由唯一调用点
/// （`approve`，恒在事务内）保证等价语义。
fn refresh_character_identity_projection(conn: &Connection) -> Result<(), String> {
    if !character_identity_repository::has_character_identity_schema(conn) {
        return Err("CHARACTER_ID_TRANSACTION_REQUIRED".to_string());
    }
    let entries = identity_projection_entries(conn)?;
    let writing_language = conn
        .query_row(
            "SELECT writing_language FROM project_core WHERE id='main'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    let language = if writing_language == "en-US" {
        "en-US"
    } else {
        DEFAULT_WRITING_LANGUAGE
    };
    let projection = character_roster::render_character_roster_markdown(&entries, language);
    conn.execute(
        "UPDATE project_core SET characters_arch=? WHERE id='main'",
        [projection.clone()],
    )
    .map_err(|error| error.to_string())?;
    let fact_hash = full_fact_hash(&entries)?;
    conn.execute(
        "UPDATE character_roster_meta SET revision=revision+1,migration_state='ready',projection_hash=?,fact_hash=?,updated_at=datetime('now') WHERE id='main'",
        [projection, fact_hash],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// `identityProjectionEntries`（`include_ids = false` 公开事实形态）：
/// 活跃角色的 `entry_from_character` 投影，关系一律改写为
/// `character_relationships` 中目标未退役的结构化关系（按 `relationship_id` 序）。
fn identity_projection_entries(conn: &Connection) -> Result<Vec<CharacterRosterEntry>, String> {
    let relationships: Vec<(String, String, String, String)> = {
        let mut statement = conn
            .prepare(
                "SELECT r.source_character_id, r.target_character_id, c.name, r.relation \
                 FROM character_relationships r \
                 JOIN characters c ON c.character_id = r.target_character_id \
                 WHERE c.retired = 0 \
                 ORDER BY r.relationship_id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        let mut collected = Vec::new();
        for row in rows {
            collected.push(row.map_err(|error| error.to_string())?);
        }
        collected
    };
    let active_ids: HashSet<String> = {
        let mut statement = conn
            .prepare("SELECT character_id FROM characters WHERE retired = 0")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        let mut collected = HashSet::new();
        for row in rows {
            collected.insert(row.map_err(|error| error.to_string())?);
        }
        collected
    };
    let raw_entries: Vec<CharacterRosterEntry> = characters::get_all(conn)?
        .into_iter()
        .filter(|character| {
            character
                .character_id
                .as_deref()
                .map(|id| active_ids.contains(id))
                .unwrap_or(false)
        })
        .map(|character| {
            let mut entry = entry_from_character(&character);
            entry.character_id = None;
            let source_id = character.character_id.clone().unwrap_or_default();
            entry.relationships = relationships
                .iter()
                .filter(|(source_character_id, _, _, _)| *source_character_id == source_id)
                .map(|(_, _, target, relation)| CharacterRosterRelationship {
                    target: target.clone(),
                    relation: relation.clone(),
                })
                .collect();
            entry
        })
        .collect();
    Ok(sorted_entries(&raw_entries))
}

/// `entry_from_character` —— 角色卡到名单条目的投影。
fn entry_from_character(character: &CharacterData) -> CharacterRosterEntry {
    let structured = is_structured_relationships(&character.relationships);
    let mut relationships: Vec<CharacterRosterRelationship> = if structured {
        serde_json::from_str(&character.relationships).unwrap_or_default()
    } else {
        Vec::new()
    };
    sort_relationships(&mut relationships);
    CharacterRosterEntry {
        character_id: character.character_id.clone(),
        name: character.name.clone(),
        role: character.role.clone(),
        gender: character.gender.clone(),
        age: character.age.clone(),
        appearance: character.appearance.clone(),
        personality: character.personality.clone(),
        background: character.background.clone(),
        abilities: character.abilities.clone(),
        motivation: character.motivation.clone(),
        relationships,
        arc: character.arc.clone(),
        notes: character.notes.clone(),
        current_state: character.current_state.clone(),
        legacy_relationship_notes: if structured || character.relationships.is_empty() {
            None
        } else {
            Some(character.relationships.clone())
        },
    }
}

/// `characters.relationships` 是否为结构化 JSON 数组。
fn is_structured_relationships(value: &str) -> bool {
    let Ok(parsed) = serde_json::from_str::<Value>(value) else {
        return false;
    };
    let Some(items) = parsed.as_array() else {
        return false;
    };
    items.iter().all(|relationship| {
        relationship.is_object()
            && relationship
                .get("target")
                .map(|value| value.is_string())
                .unwrap_or(false)
            && relationship
                .get("relation")
                .map(|value| value.is_string())
                .unwrap_or(false)
    })
}

/// 文本比较（对齐基线 `compare_text`；UTF-8 字节序为栈内文档化差异）。
fn compare_text(left: &str, right: &str) -> std::cmp::Ordering {
    left.cmp(right)
}

fn sort_relationships(relationships: &mut [CharacterRosterRelationship]) {
    relationships.sort_by(|left, right| {
        compare_text(&left.target, &right.target)
            .then_with(|| compare_text(&left.relation, &right.relation))
    });
}

/// 条目排序：角色定位权重 → 姓名（对齐基线 `sorted_entries`）。
fn sorted_entries(entries: &[CharacterRosterEntry]) -> Vec<CharacterRosterEntry> {
    let mut sorted = entries.to_vec();
    sorted.sort_by(|left, right| {
        left.role
            .sort_weight()
            .cmp(&right.role.sort_weight())
            .then_with(|| compare_text(&left.name, &right.name))
    });
    sorted
}

/// `fullFactHash`：覆盖角色资料、结构化关系与 `currentState` 的完整事实哈希。
fn full_fact_hash(entries: &[CharacterRosterEntry]) -> Result<String, String> {
    let canonical = character_roster::canonical_entries(entries);
    let encoded = serde_json::to_string(&canonical).map_err(|error| error.to_string())?;
    Ok(character_roster::hash_text(&encoded))
}
