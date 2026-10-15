//! finalized_character_service —— 定稿角色状态候选读面与决策（批次 B98）。
//!
//! 1:1 移植基线：
//! - `electron/repositories/summary-repository.ts:271-575`：候选键与决策载荷哈希、
//!   当前候选行读取、待决候选投影，以及候选列表 / 读取 / 决策三个入口；
//! - `electron/services/finalized-character-generation.ts:21-35`：`readContext` 的
//!   `draftId` 校验与上下文读取（`contextId` 签发、LRU 缓存、`originProjectId`
//!   属 service 实例状态，落在 owner 层，与本服务同寿命）。
//!
//! `finalized-character:commit` 频道依赖 `finalized-character-generation-proof.ts`
//! 的产物来源证明与 `FinalizedCharacterGeneration.admit` 准入，留待后续批次。

use std::sync::LazyLock;

use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{json, Map, Value};

use crate::repositories::character_repository::{
    CharacterStateFieldProvenance, CharacterStateTextField, FinalizedSourceIdentity,
};
use crate::repositories::finalized_continuity_repository::{
    is_lowercase_hex64, parse_character_state_candidate_store, read_finalized_source_from_db,
    CharacterStateCandidateStore, CharacterStateDecision, FinalizedCharacterStateCandidate,
    SaveFinalizedCharacterStateCandidatesRequest, StoredCharacterStateDecisionReceipt,
};
use crate::repositories::generation_run_repository::ProjectEpoch;
use crate::services::finalization_generation::text_hash;
use crate::services::finalization_generation_source::{
    decide_derived_patch, immutable_identity, read_finalized_character_context,
    read_frozen_character_snapshot, state_snapshot, CharacterStateRow, DerivedCharacterPatch,
    DerivedPatchAuthority, DerivedPatchDecision, FinalizedCharacterContext,
    FinalizedIdentityStatus,
};

// ============================================================================
// 错误码（与基线 fail / throw 调用逐字对应）
// ============================================================================

pub const GENERATION_CHARACTER_DRAFT_INVALID: &str = "GENERATION_CHARACTER_DRAFT_INVALID";
pub const FINALIZED_CHARACTER_STATE_CANDIDATE_INVALID: &str =
    "FINALIZED_CHARACTER_STATE_CANDIDATE_INVALID";
pub const FINALIZED_CHARACTER_STATE_CANDIDATE_NOT_FOUND: &str =
    "FINALIZED_CHARACTER_STATE_CANDIDATE_NOT_FOUND";
pub const FINALIZED_CHARACTER_STATE_DECISION_INVALID: &str =
    "FINALIZED_CHARACTER_STATE_DECISION_INVALID";
pub const FINALIZED_CHARACTER_STATE_OPERATION_CONFLICT: &str =
    "FINALIZED_CHARACTER_STATE_OPERATION_CONFLICT";
pub const FINALIZED_CHARACTER_STATE_SOURCE_CHANGED: &str =
    "FINALIZED_CHARACTER_STATE_SOURCE_CHANGED";
pub const FINALIZED_CHARACTER_STATE_FIELD_CONFLICT: &str =
    "FINALIZED_CHARACTER_STATE_FIELD_CONFLICT";

/// `CHARACTER_STATE_TEXT_FIELDS` 的 TS 字面量名（参与 `JSON.stringify` 键）。
fn field_name(field: CharacterStateTextField) -> &'static str {
    match field {
        CharacterStateTextField::Location => "location",
        CharacterStateTextField::PowerLevel => "powerLevel",
        CharacterStateTextField::PhysicalState => "physicalState",
        CharacterStateTextField::MentalState => "mentalState",
        CharacterStateTextField::KeyItems => "keyItems",
        CharacterStateTextField::RecentEvents => "recentEvents",
    }
}

/// 基线 `STATE_COLUMNS[field]`（`summary-repository.ts:37-40`）。
fn state_column(field: CharacterStateTextField) -> &'static str {
    match field {
        CharacterStateTextField::Location => "cs_location",
        CharacterStateTextField::PowerLevel => "cs_power_level",
        CharacterStateTextField::PhysicalState => "cs_physical_state",
        CharacterStateTextField::MentalState => "cs_mental_state",
        CharacterStateTextField::KeyItems => "cs_key_items",
        CharacterStateTextField::RecentEvents => "cs_recent_events",
    }
}

/// 候选升级 / 决策探测用的占位 scope（基线 `candidate-upgrade` /
/// `candidate-decision`）：`stateSnapshot` 只把 projectId/epoch 回填进快照。
fn probe_scope(tag: &str) -> ProjectEpoch {
    ProjectEpoch {
        project_id: tag.to_string(),
        epoch: tag.to_string(),
    }
}

/// 基线 `characterStateCandidateKey(source, characterId, field, value)`：
/// `fcs-state:<sha256(JSON.stringify([draftId, finalizationId, contentHash,
/// characterId, field, sha256(value)]))>`。
pub(crate) fn character_state_candidate_key(
    source: &FinalizedSourceIdentity,
    character_id: &str,
    field: CharacterStateTextField,
    value: &str,
) -> String {
    let payload = json!([
        source.draft_id,
        source.finalization_id,
        source.content_hash,
        character_id,
        field_name(field),
        text_hash(value),
    ]);
    format!(
        "fcs-state:{}",
        text_hash(&serde_json::to_string(&payload).unwrap_or_default())
    )
}

/// 基线 `stateDecisionPayloadHash(request)`：
/// `sha256(JSON.stringify([draftId, candidateKey, characterId, field,
/// expectedFieldRevision, expectedFieldValueHash, operationId, decision]))`。
pub(crate) fn state_decision_payload_hash(request: &Value) -> String {
    let payload = json!([
        request.get("draftId"),
        request.get("candidateKey"),
        request.get("characterId"),
        request.get("field"),
        request.get("expectedFieldRevision"),
        request.get("expectedFieldValueHash"),
        request.get("operationId"),
        request.get("decision"),
    ]);
    text_hash(&serde_json::to_string(&payload).unwrap_or_default())
}

// ============================================================================
// 当前候选行（基线 `CharacterStateCandidateRow` /
// `readCurrentCharacterStateCandidateRow`）
// ============================================================================

pub(crate) struct CharacterStateCandidateRow {
    pub(crate) id: i64,
    pub(crate) draft_id: i64,
    pub(crate) chapter_number: i64,
    pub(crate) candidates: String,
    pub(crate) finalization_id: String,
    pub(crate) content_hash: String,
    /// 保留基线行形状；来源一致性校验在读取时用局部变量完成。
    #[allow(dead_code)]
    pub(crate) projection_generation: i64,
}

/// 基线 `readCurrentCharacterStateCandidateRow`：候选行来源必须与冻结定稿源同一、
/// 投影代不得落后于当前水位（除非该章未失效），且该草稿仍是本章最新定稿稿。
pub(crate) fn read_current_character_state_candidate_row(
    conn: &Connection,
    draft_id: i64,
) -> Result<CharacterStateCandidateRow, String> {
    let row: Option<(i64, i64, i64, String, String, String, i64)> = conn
        .query_row(
            "SELECT id, draft_id, chapter_number, character_state_candidates, \
             source_finalization_id, source_content_hash, projection_generation \
             FROM summary_snapshots WHERE draft_id=?1",
            [draft_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some(snapshot) = read_finalized_source_from_db(conn, draft_id)? else {
        return Err(FINALIZED_CHARACTER_STATE_SOURCE_CHANGED.to_string());
    };
    let Some((
        id,
        row_draft_id,
        chapter_number,
        candidates,
        finalization_id,
        content_hash,
        projection_generation,
    )) = row
    else {
        return Err(FINALIZED_CHARACTER_STATE_SOURCE_CHANGED.to_string());
    };
    let source = FinalizedSourceIdentity {
        draft_id: row_draft_id,
        finalization_id: finalization_id.clone(),
        chapter_number,
        content_hash: content_hash.clone(),
    };
    if source != snapshot.source || projection_generation > snapshot.projection_generation {
        return Err(FINALIZED_CHARACTER_STATE_SOURCE_CHANGED.to_string());
    }
    if projection_generation < snapshot.projection_generation {
        let stale_from_chapter: Option<i64> = conn
            .query_row(
                "SELECT stale_from_chapter FROM continuity_projection_meta WHERE id='main'",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .flatten();
        if stale_from_chapter.is_some_and(|from| chapter_number >= from) {
            return Err(FINALIZED_CHARACTER_STATE_SOURCE_CHANGED.to_string());
        }
    }
    let latest: Option<i64> = conn
        .query_row(
            "SELECT id FROM drafts WHERE chapter_number=?1 AND status='finalized' \
             ORDER BY version DESC,id DESC LIMIT 1",
            [snapshot.source.chapter_number],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if latest != Some(draft_id) {
        return Err(FINALIZED_CHARACTER_STATE_SOURCE_CHANGED.to_string());
    }
    Ok(CharacterStateCandidateRow {
        id,
        draft_id: row_draft_id,
        chapter_number,
        candidates,
        finalization_id,
        content_hash,
        projection_generation,
    })
}

// ============================================================================
// 待决候选（基线 `pendingCharacterStateCandidate`）
// ============================================================================

pub(crate) struct PendingFinalizedCharacterStateCandidate {
    pub(crate) candidate: FinalizedCharacterStateCandidate,
    pub(crate) candidate_key: String,
    pub(crate) character_id: String,
    pub(crate) source: FinalizedSourceIdentity,
    pub(crate) expected_field_revision: i64,
    pub(crate) expected_field_value_hash: String,
}

impl PendingFinalizedCharacterStateCandidate {
    /// 基线的返回形状：`{...candidate, candidateKey, source,
    /// expectedFieldRevision, expectedFieldValueHash}`（绑定键覆盖候选自带键）。
    pub(crate) fn to_value(&self) -> Result<Value, String> {
        let mut value = serde_json::to_value(&self.candidate).map_err(|error| error.to_string())?;
        let map = value
            .as_object_mut()
            .ok_or_else(|| "角色状态候选形状无效".to_string())?;
        map.insert(
            "candidateKey".to_string(),
            Value::String(self.candidate_key.clone()),
        );
        map.insert(
            "source".to_string(),
            serde_json::to_value(&self.source).map_err(|error| error.to_string())?,
        );
        map.insert(
            "expectedFieldRevision".to_string(),
            json!(self.expected_field_revision),
        );
        map.insert(
            "expectedFieldValueHash".to_string(),
            Value::String(self.expected_field_value_hash.clone()),
        );
        Ok(value)
    }
}

/// 基线 `pendingCharacterStateCandidate` 内的 `characters` 行读取（`retired=0`）。
fn read_character_state_row(
    conn: &Connection,
    character_id: &str,
) -> Result<Option<CharacterStateRow>, String> {
    conn.query_row(
        "SELECT character_id, cs_location, cs_power_level, cs_physical_state, \
         cs_mental_state, cs_key_items, cs_recent_events, cs_provenance \
         FROM characters WHERE character_id=?1 AND retired=0",
        [character_id],
        |row| {
            Ok(CharacterStateRow {
                character_id: row.get(0)?,
                cs_location: row.get(1)?,
                cs_power_level: row.get(2)?,
                cs_physical_state: row.get(3)?,
                cs_mental_state: row.get(4)?,
                cs_key_items: row.get(5)?,
                cs_recent_events: row.get(6)?,
                cs_provenance: row.get(7)?,
            })
        },
    )
    .optional()
    .map_err(|error| error.to_string())
}

/// 基线 `pendingCharacterStateCandidate`：候选必须绑定当前源（或可由冻结快照与当前
/// 角色行升级出绑定元数据），否则不作为待决候选暴露。
pub(crate) fn pending_character_state_candidate(
    conn: &Connection,
    row: &CharacterStateCandidateRow,
    candidate: &FinalizedCharacterStateCandidate,
) -> Result<Option<PendingFinalizedCharacterStateCandidate>, String> {
    let source = FinalizedSourceIdentity {
        draft_id: row.draft_id,
        finalization_id: row.finalization_id.clone(),
        chapter_number: row.chapter_number,
        content_hash: row.content_hash.clone(),
    };
    let Some(character_id) = candidate
        .character_id
        .as_deref()
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let expected_key =
        character_state_candidate_key(&source, character_id, candidate.field, &candidate.value);
    let has_bound_metadata = candidate.candidate_key.is_some()
        || candidate.source.is_some()
        || candidate.expected_field_revision.is_some()
        || candidate.expected_field_value_hash.is_some();
    if has_bound_metadata {
        let bound_ok = candidate
            .candidate_key
            .as_deref()
            .is_some_and(|key| !key.is_empty())
            && candidate
                .source
                .as_ref()
                .is_some_and(|bound| *bound == source)
            && candidate
                .expected_field_revision
                .is_some_and(|revision| revision >= 0)
            && candidate
                .expected_field_value_hash
                .as_deref()
                .is_some_and(is_lowercase_hex64)
            && candidate.candidate_key.as_deref() == Some(expected_key.as_str());
        if !bound_ok {
            return Ok(None);
        }
    }
    let Some(frozen) = read_frozen_character_snapshot(conn, &source)? else {
        return Ok(None);
    };
    if !frozen.characters.iter().any(|item| {
        item.character_id == character_id && item.display_name_snapshot == candidate.character_name
    }) {
        return Ok(None);
    }
    if has_bound_metadata {
        return Ok(Some(PendingFinalizedCharacterStateCandidate {
            candidate: candidate.clone(),
            candidate_key: candidate.candidate_key.clone().unwrap_or_default(),
            character_id: character_id.to_string(),
            source,
            expected_field_revision: candidate.expected_field_revision.unwrap_or(0),
            expected_field_value_hash: candidate
                .expected_field_value_hash
                .clone()
                .unwrap_or_default(),
        }));
    }
    let Some(character_row) = read_character_state_row(conn, character_id)? else {
        return Ok(None);
    };
    let current = state_snapshot(
        &character_row,
        candidate.field,
        &probe_scope("candidate-upgrade"),
    )?;
    Ok(Some(PendingFinalizedCharacterStateCandidate {
        candidate: candidate.clone(),
        candidate_key: expected_key,
        character_id: character_id.to_string(),
        source,
        expected_field_revision: current.revision,
        expected_field_value_hash: current.value_hash,
    }))
}

/// 未绑定元数据的候选升级写回形态（基线 `store.pending[index] = candidate`）。
fn bind_pending_candidate(
    candidate: &FinalizedCharacterStateCandidate,
    pending: &PendingFinalizedCharacterStateCandidate,
) -> FinalizedCharacterStateCandidate {
    let mut bound = candidate.clone();
    bound.candidate_key = Some(pending.candidate_key.clone());
    bound.source = Some(pending.source.clone());
    bound.expected_field_revision = Some(pending.expected_field_revision);
    bound.expected_field_value_hash = Some(pending.expected_field_value_hash.clone());
    bound
}

fn write_candidate_store(
    tx: &Transaction<'_>,
    row_id: i64,
    store: &CharacterStateCandidateStore,
) -> Result<(), String> {
    tx.execute(
        "UPDATE summary_snapshots SET character_state_candidates=?1,created_at=datetime('now') \
         WHERE id=?2",
        params![
            serde_json::to_string(store).map_err(|error| error.to_string())?,
            row_id
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

// ============================================================================
// 三个频道入口
// ============================================================================

/// 基线 `SummaryRepository.listPendingFinalizedCharacterStateCandidates`：
/// 单行失效（来源变化 / 快照非法）静默跳过，不影响其它行的候选。
pub fn list_pending_state_candidates(conn: &Connection) -> Result<Value, String> {
    let mut stmt = conn
        .prepare("SELECT draft_id FROM summary_snapshots WHERE character_state_candidates<>'[]'")
        .map_err(|error| error.to_string())?;
    let draft_ids = stmt
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(stmt);
    let mut summaries: Vec<(i64, String, Value)> = Vec::new();
    for draft_id in draft_ids {
        let Ok(row) = read_current_character_state_candidate_row(conn, draft_id) else {
            continue;
        };
        let store = parse_character_state_candidate_store(&row.candidates);
        for candidate in &store.pending {
            let Ok(Some(pending)) = pending_character_state_candidate(conn, &row, candidate) else {
                continue;
            };
            summaries.push((
                draft_id,
                pending.candidate_key.clone(),
                json!({
                    "draftId": draft_id,
                    "candidateKey": pending.candidate_key,
                    "finalizationId": pending.source.finalization_id,
                    "characterId": pending.character_id,
                    "characterName": pending.candidate.character_name,
                    "field": serde_json::to_value(pending.candidate.field)
                        .map_err(|error| error.to_string())?,
                }),
            ));
        }
    }
    summaries.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    Ok(Value::Array(
        summaries.into_iter().map(|(_, _, value)| value).collect(),
    ))
}

/// 基线 `SummaryRepository.readPendingFinalizedCharacterStateCandidate`：未绑定元数据
/// 的历史候选在读取时补齐并写回。
pub fn read_pending_state_candidate(
    conn: &Connection,
    draft_id: i64,
    candidate_key: &str,
) -> Result<Value, String> {
    if draft_id < 1 || candidate_key.trim().is_empty() {
        return Err(FINALIZED_CHARACTER_STATE_CANDIDATE_INVALID.to_string());
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let row = read_current_character_state_candidate_row(&tx, draft_id)?;
    let mut store = parse_character_state_candidate_store(&row.candidates);
    let mut index: Option<usize> = None;
    for (position, item) in store.pending.iter().enumerate() {
        let Some(pending) = pending_character_state_candidate(&tx, &row, item)? else {
            continue;
        };
        if pending.candidate_key == candidate_key {
            index = Some(position);
            break;
        }
    }
    let Some(index) = index else {
        return Err(FINALIZED_CHARACTER_STATE_CANDIDATE_NOT_FOUND.to_string());
    };
    let pending = pending_character_state_candidate(&tx, &row, &store.pending[index])?
        .ok_or_else(|| FINALIZED_CHARACTER_STATE_CANDIDATE_NOT_FOUND.to_string())?;
    if store.pending[index].candidate_key.is_none() {
        let bound = bind_pending_candidate(&store.pending[index].clone(), &pending);
        store.pending[index] = bound;
        write_candidate_store(&tx, row.id, &store)?;
    }
    let value = pending.to_value()?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(value)
}

/// 基线 `SummaryRepository.decideFinalizedCharacterStateCandidate`。
pub fn decide_state_candidate(conn: &Connection, request: &Value) -> Result<Value, String> {
    let obj = request
        .as_object()
        .ok_or_else(|| FINALIZED_CHARACTER_STATE_DECISION_INVALID.to_string())?;
    let draft_id = obj
        .get("draftId")
        .and_then(Value::as_i64)
        .filter(|value| *value >= 1)
        .ok_or_else(|| FINALIZED_CHARACTER_STATE_DECISION_INVALID.to_string())?;
    let candidate_key = obj
        .get("candidateKey")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| FINALIZED_CHARACTER_STATE_DECISION_INVALID.to_string())?;
    let character_id = obj
        .get("characterId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| FINALIZED_CHARACTER_STATE_DECISION_INVALID.to_string())?;
    let field = obj
        .get("field")
        .and_then(Value::as_str)
        .and_then(|text| {
            serde_json::from_value::<CharacterStateTextField>(Value::String(text.to_string())).ok()
        })
        .ok_or_else(|| FINALIZED_CHARACTER_STATE_DECISION_INVALID.to_string())?;
    let expected_revision = obj
        .get("expectedFieldRevision")
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0)
        .ok_or_else(|| FINALIZED_CHARACTER_STATE_DECISION_INVALID.to_string())?;
    let expected_hash = obj
        .get("expectedFieldValueHash")
        .and_then(Value::as_str)
        .filter(|value| is_lowercase_hex64(value))
        .ok_or_else(|| FINALIZED_CHARACTER_STATE_DECISION_INVALID.to_string())?;
    let operation_id = obj
        .get("operationId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| FINALIZED_CHARACTER_STATE_DECISION_INVALID.to_string())?;
    let decision = match obj.get("decision").and_then(Value::as_str) {
        Some("accept") => CharacterStateDecision::Accept,
        Some("decline") => CharacterStateDecision::Decline,
        _ => return Err(FINALIZED_CHARACTER_STATE_DECISION_INVALID.to_string()),
    };
    let payload_hash = state_decision_payload_hash(request);
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    // 幂等重放：同一 operationId 携带不同载荷必须冲突。
    {
        let mut stmt = tx
            .prepare(
                "SELECT character_state_candidates FROM summary_snapshots \
                 WHERE character_state_candidates<>'[]'",
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        for raw in rows {
            let store = parse_character_state_candidate_store(&raw);
            for receipt in store.receipts {
                if receipt.operation_id == operation_id {
                    if receipt.payload_hash != payload_hash {
                        return Err(FINALIZED_CHARACTER_STATE_OPERATION_CONFLICT.to_string());
                    }
                    let mut value =
                        serde_json::to_value(&receipt).map_err(|error| error.to_string())?;
                    if let Some(map) = value.as_object_mut() {
                        map.insert("idempotent".to_string(), json!(true));
                    }
                    return Ok(value);
                }
            }
        }
    }
    let row = read_current_character_state_candidate_row(&tx, draft_id)?;
    let mut store = parse_character_state_candidate_store(&row.candidates);
    let mut matched: Option<PendingFinalizedCharacterStateCandidate> = None;
    for item in &store.pending {
        let Some(pending) = pending_character_state_candidate(&tx, &row, item)? else {
            continue;
        };
        if pending.candidate_key == candidate_key {
            matched = Some(pending);
            break;
        }
    }
    let Some(candidate) = matched else {
        return Err(FINALIZED_CHARACTER_STATE_CANDIDATE_NOT_FOUND.to_string());
    };
    if candidate.character_id != character_id
        || candidate.candidate.field != field
        || candidate.expected_field_revision != expected_revision
        || candidate.expected_field_value_hash != expected_hash
    {
        return Err(FINALIZED_CHARACTER_STATE_CANDIDATE_NOT_FOUND.to_string());
    }
    if decision == CharacterStateDecision::Accept {
        let Some(character_row) = read_character_state_row(&tx, &candidate.character_id)? else {
            return Err(FINALIZED_CHARACTER_STATE_CANDIDATE_NOT_FOUND.to_string());
        };
        let current = state_snapshot(
            &character_row,
            candidate.candidate.field,
            &probe_scope("candidate-decision"),
        )?;
        if current.revision != expected_revision || current.value_hash != expected_hash {
            return Err(FINALIZED_CHARACTER_STATE_FIELD_CONFLICT.to_string());
        }
        let mut provenance = parse_provenance_map(character_row.cs_provenance.as_deref())?;
        provenance.insert(
            field_name(candidate.candidate.field).to_string(),
            json!({
                "kind": "author",
                "chapterNumber": candidate.source.chapter_number,
                "revision": current.revision + 1,
            }),
        );
        let updated = tx
            .execute(
                &format!(
                    "UPDATE characters SET {}=?1,cs_provenance=?2, \
                     cs_updated_at_chapter=MAX(COALESCE(cs_updated_at_chapter,0),?3), \
                     updated_at=datetime('now') WHERE character_id=?4 AND retired=0",
                    state_column(candidate.candidate.field)
                ),
                params![
                    candidate.candidate.value,
                    serde_json::to_string(&provenance).map_err(|error| error.to_string())?,
                    candidate.source.chapter_number,
                    candidate.character_id,
                ],
            )
            .map_err(|error| error.to_string())?;
        if updated != 1 {
            return Err(FINALIZED_CHARACTER_STATE_FIELD_CONFLICT.to_string());
        }
        crate::repositories::character_roster_repository::refresh_character_state_projection(&tx)?;
    }
    let receipt = StoredCharacterStateDecisionReceipt {
        candidate_key: candidate.candidate_key.clone(),
        operation_id: operation_id.to_string(),
        payload_hash: payload_hash.clone(),
        decision,
        source: candidate.source.clone(),
    };
    store
        .pending
        .retain(|item| item.candidate_key.as_deref() != Some(candidate.candidate_key.as_str()));
    store.receipts.push(receipt.clone());
    write_candidate_store(&tx, row.id, &store)?;
    tx.commit().map_err(|error| error.to_string())?;
    let mut value = serde_json::to_value(&receipt).map_err(|error| error.to_string())?;
    if let Some(map) = value.as_object_mut() {
        map.insert("idempotent".to_string(), json!(false));
    }
    Ok(value)
}

/// 基线 `JSON.parse(String(row.cs_provenance || '{}'))`：空串回落空映射，非对象
/// 形态按解析失败处理（写路径恒为对象）。
fn parse_provenance_map(raw: Option<&str>) -> Result<Map<String, Value>, String> {
    match raw {
        None => Ok(Map::new()),
        Some(text) if text.is_empty() => Ok(Map::new()),
        Some(text) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) => Ok(map),
            _ => Err("角色状态来源映射无效".to_string()),
        },
    }
}

// ============================================================================
// read-context
// ============================================================================

/// 基线 `FinalizedCharacterGeneration.readContext` 的上下文部分（`draftId` 校验 +
/// 不可变上下文读取）。`contextId` 签发、LRU 缓存与 `originProjectId` 由 owner 层按
/// service 实例状态处理。
pub fn read_context(
    conn: &Connection,
    scope: &ProjectEpoch,
    draft_id: i64,
) -> Result<Value, String> {
    if draft_id < 1 {
        return Err(GENERATION_CHARACTER_DRAFT_INVALID.to_string());
    }
    let context = read_finalized_character_context(conn, draft_id, scope)?;
    serde_json::to_value(&context).map_err(|error| error.to_string())
}

/// `currentState` 的六个文本字段（对齐 `CHARACTER_STATE_TEXT_FIELDS`）。
const CHARACTER_STATE_TEXT_FIELDS: [CharacterStateTextField; 6] = [
    CharacterStateTextField::Location,
    CharacterStateTextField::PowerLevel,
    CharacterStateTextField::PhysicalState,
    CharacterStateTextField::MentalState,
    CharacterStateTextField::KeyItems,
    CharacterStateTextField::RecentEvents,
];

// ============================================================================
// 定稿角色状态响应解析（基线 `src/shared/finalized-continuity.ts:171-224`）
// ============================================================================

/// `updates` 项（`{characterId, currentState, evidence}` 与原始 JSON）。
#[derive(Debug)]
pub(crate) struct ParsedStateUpdate {
    pub(crate) character_id: String,
    /// `currentState` 中实际出现的字段与值（按字段声明序）。
    pub(crate) current_state: Vec<(CharacterStateTextField, String)>,
    pub(crate) evidence: Value,
    /// 基线 `rawValue: structuredClone(update)`。
    pub(crate) raw: Value,
}

#[derive(Debug)]
pub(crate) struct ParsedStateResponse {
    pub(crate) updates: Vec<ParsedStateUpdate>,
    pub(crate) unresolved: Vec<Value>,
}

/// 代码围栏剥离（基线 `^```(?:json)?\s*\n?` 与 `\n?```\s*$`）。
static FENCE_HEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^```(?:json)?\s*\n?").expect("围栏头正则合法"));
static FENCE_TAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\n?```\s*$").expect("围栏尾正则合法"));

/// JS UTF-16 码元偏移 → 字节偏移（偏移落在字符中间时返回 `None`）。
fn utf16_boundary_to_byte(text: &str, units: usize) -> Option<usize> {
    let mut seen = 0usize;
    for (index, character) in text.char_indices() {
        if seen == units {
            return Some(index);
        }
        seen += character.len_utf16();
    }
    if seen == units {
        Some(text.len())
    } else {
        None
    }
}

/// `content.slice(start, end)` 的等价物（UTF-16 码元区间）。
fn utf16_slice(text: &str, start: usize, end: usize) -> Option<String> {
    let from = utf16_boundary_to_byte(text, start)?;
    let to = utf16_boundary_to_byte(text, end)?;
    if from > to {
        return None;
    }
    Some(text[from..to].to_string())
}

/// `content.indexOf(needle)` 的 UTF-16 码元偏移。
fn utf16_index_of(text: &str, needle: &str) -> Option<usize> {
    utf16_index_of_from(text, needle, 0)
}

/// `content.indexOf(needle, from)` 的 UTF-16 码元偏移。
fn utf16_index_of_from(text: &str, needle: &str, from_units: usize) -> Option<usize> {
    let byte_from = utf16_boundary_to_byte(text, from_units)?;
    let byte = text.get(byte_from..)?.find(needle)?;
    Some(text[..byte_from + byte].encode_utf16().count())
}

/// 基线 `parseFinalizedCharacterStateResponse`：解析模型产物中的角色状态
/// 更新，未证明身份的出现保留为待确认（绝不自动归并）。
pub(crate) fn parse_finalized_character_state_response(
    content: &str,
    context: &FinalizedCharacterContext,
) -> Result<ParsedStateResponse, String> {
    let text = FENCE_TAIL
        .replace(&FENCE_HEAD.replace(content.trim(), ""), "")
        .to_string();
    let parsed: Value = serde_json::from_str(&text)
        .map_err(|_| "FINALIZED_CHARACTER_UPDATES_INVALID".to_string())?;
    let Some(items) = parsed.get("updates").and_then(Value::as_array) else {
        return Err("FINALIZED_CHARACTER_UPDATES_INVALID".to_string());
    };
    if items.len() > 1_000 {
        return Err("FINALIZED_CHARACTER_UPDATES_INVALID".to_string());
    }
    let content_units = context.content.encode_utf16().count() as i64;
    let mut result = ParsedStateResponse {
        updates: Vec::new(),
        unresolved: Vec::new(),
    };
    let mut seen: Vec<String> = Vec::new();
    for (index, input) in items.iter().enumerate() {
        let Some(record) = input.as_object() else {
            return Err("FINALIZED_CHARACTER_UPDATE_INVALID".to_string());
        };
        let Some(current_state_obj) = record.get("currentState").and_then(Value::as_object) else {
            return Err("FINALIZED_CHARACTER_UPDATE_INVALID".to_string());
        };
        let Some(evidence) = record.get("evidence").and_then(Value::as_object) else {
            return Err("FINALIZED_CHARACTER_EVIDENCE_MISSING".to_string());
        };
        let located_text = evidence.get("text").and_then(Value::as_str);
        let mut start = evidence.get("start").and_then(Value::as_i64);
        let mut end = evidence.get("end").and_then(Value::as_i64);
        // 只给 text 时按唯一子串定位（基线 :179-184）。
        if !evidence.contains_key("start") && !evidence.contains_key("end") {
            let Some(needle) = located_text.filter(|value| !value.is_empty()) else {
                return Err("FINALIZED_CHARACTER_EVIDENCE_INVALID".to_string());
            };
            let Some(located) = utf16_index_of(&context.content, needle) else {
                return Err("FINALIZED_CHARACTER_EVIDENCE_NOT_UNIQUE".to_string());
            };
            if utf16_index_of_from(&context.content, needle, located + 1).is_some() {
                return Err("FINALIZED_CHARACTER_EVIDENCE_NOT_UNIQUE".to_string());
            }
            start = Some(located as i64);
            end = Some(located as i64 + needle.encode_utf16().count() as i64);
        }
        let (Some(start), Some(end)) = (start, end) else {
            return Err("FINALIZED_CHARACTER_EVIDENCE_INVALID".to_string());
        };
        let Some(needle) = located_text else {
            return Err("FINALIZED_CHARACTER_EVIDENCE_INVALID".to_string());
        };
        if start < 0
            || end <= start
            || end > content_units
            || utf16_slice(&context.content, start as usize, end as usize).as_deref()
                != Some(needle)
        {
            return Err("FINALIZED_CHARACTER_EVIDENCE_INVALID".to_string());
        }
        let mut current_state: Vec<(CharacterStateTextField, String)> = Vec::new();
        for field in CHARACTER_STATE_TEXT_FIELDS {
            let Some(value) = current_state_obj.get(field_name(field)) else {
                continue;
            };
            let Some(value) = value.as_str() else {
                return Err("FINALIZED_CHARACTER_FIELD_INVALID".to_string());
            };
            current_state.push((field, value.to_string()));
        }
        if current_state.is_empty() {
            return Err("FINALIZED_CHARACTER_FIELD_INVALID".to_string());
        }
        if let Some(chapter) = current_state_obj.get("updatedAtChapter") {
            if chapter.as_i64() != Some(context.source.chapter_number) {
                return Err("FINALIZED_CHARACTER_FIELD_INVALID".to_string());
            }
        }
        let character_id = record
            .get("characterId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let original_text = record
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let character = if context.identity_status == FinalizedIdentityStatus::Bound {
            context
                .characters
                .iter()
                .find(|item| item.character_id == character_id)
        } else {
            None
        };
        let exact_evidence = json!({
            "start": start,
            "end": end,
            "text": needle,
        });
        // 冻结名单里同名标签不唯一时，模型给出的 ID 也不足以判定身份。
        let shared_label = character.is_some_and(|item| {
            context
                .characters
                .iter()
                .filter(|other| other.display_name_snapshot == item.display_name_snapshot)
                .count()
                > 1
        });
        if character.is_none() || shared_label {
            if original_text.trim().is_empty() && character_id.is_empty() {
                return Err("FINALIZED_CHARACTER_IDENTITY_REQUIRED".to_string());
            }
            let mut candidate_ids: Vec<String> = Vec::new();
            for item in &context.characters {
                let matches = (!character_id.is_empty() && item.character_id == character_id)
                    || (shared_label
                        && character.is_some_and(|current| {
                            item.display_name_snapshot == current.display_name_snapshot
                        }))
                    || item.display_name_snapshot == original_text
                    || item.aliases.iter().any(|alias| alias == &original_text);
                if matches {
                    candidate_ids.push(item.character_id.clone());
                }
            }
            let display_name = if original_text.trim().is_empty() {
                character
                    .map(|current| current.display_name_snapshot.clone())
                    .unwrap_or_else(|| original_text.clone())
            } else {
                original_text.clone()
            };
            let reason = if context.identity_status == FinalizedIdentityStatus::Legacy {
                "legacy"
            } else if shared_label {
                "ambiguous"
            } else if character_id.is_empty() {
                "name-only"
            } else {
                "unknown-id"
            };
            let mut current_state_map = Map::new();
            for (field, value) in &current_state {
                current_state_map
                    .insert(field_name(*field).to_string(), Value::String(value.clone()));
            }
            result.unresolved.push(json!({
                "selectionKey": format!("update:{}", index),
                "displayName": display_name,
                "originalText": original_text,
                "candidateIds": candidate_ids,
                "currentState": Value::Object(current_state_map),
                "evidence": exact_evidence,
                "rawValue": input.clone(),
                "reason": reason,
            }));
            continue;
        }
        if seen.iter().any(|item| item == &character_id) {
            return Err("FINALIZED_CHARACTER_DUPLICATE_ID".to_string());
        }
        seen.push(character_id.clone());
        result.updates.push(ParsedStateUpdate {
            character_id,
            current_state,
            evidence: exact_evidence,
            raw: input.clone(),
        });
    }
    Ok(result)
}

/// 基线 `SummaryRepository.commitFinalizedCharacterStates`
/// （`summary-repository.ts:423-473`）：逐字段按派生补丁判定后落库，需要作者
/// 确认的保留为候选；任一出现未被证明身份即拒绝提交。
///
/// 调用方必须已持有 `IMMEDIATE` 事务（基线的 `db.transaction(...)` 内嵌套调用）。
pub fn commit_finalized_character_states_in_transaction(
    conn: &Connection,
    context: &FinalizedCharacterContext,
    response: &Value,
    origin_project_id: Option<&str>,
) -> Result<Value, String> {
    let scope = ProjectEpoch {
        project_id: context.project_id.clone(),
        epoch: context.epoch.clone(),
    };
    let current = read_finalized_character_context(conn, context.source.draft_id, &scope)?;
    if immutable_identity(&current) != immutable_identity(context) {
        return Err("FINALIZED_CHARACTER_CONTEXT_CHANGED".to_string());
    }
    if context.characters.iter().any(|character| {
        character.fields.iter().any(|field| {
            field.character_id != character.character_id
                || field.project_id != context.project_id
                || field.epoch != context.epoch
        })
    }) {
        return Err("FINALIZED_CHARACTER_CONTEXT_CHANGED".to_string());
    }
    let updates = response
        .get("updates")
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    let verified = parse_finalized_character_state_response(
        &json!({ "updates": updates }).to_string(),
        context,
    )?;
    if !verified.unresolved.is_empty() {
        return Err("FINALIZED_CHARACTER_IDENTITY_REQUIRED".to_string());
    }
    let mut applied = 0i64;
    let mut unchanged = 0i64;
    let mut candidates: Vec<Value> = Vec::new();
    for update in &verified.updates {
        let Some(captured) = context
            .characters
            .iter()
            .find(|item| item.character_id == update.character_id)
        else {
            return Err("FINALIZED_CHARACTER_CONTEXT_CHANGED".to_string());
        };
        let Some(current_character) = current
            .characters
            .iter()
            .find(|item| item.character_id == update.character_id)
        else {
            return Err("FINALIZED_CHARACTER_CONTEXT_CHANGED".to_string());
        };
        let provenance_raw: String = conn
            .query_row(
                "SELECT cs_provenance FROM characters WHERE character_id=?1 AND retired=0",
                [update.character_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let mut provenance = parse_provenance_map(Some(provenance_raw.as_str()))?;
        let mut changed = false;
        for field in CHARACTER_STATE_TEXT_FIELDS {
            let Some((_, value)) = update
                .current_state
                .iter()
                .find(|(candidate_field, _)| *candidate_field == field)
            else {
                continue;
            };
            let Some(base) = captured.fields.iter().find(|item| item.field == field) else {
                return Err("FINALIZED_CHARACTER_CONTEXT_CHANGED".to_string());
            };
            if text_hash(&base.value) != base.value_hash {
                return Err("FINALIZED_CHARACTER_CONTEXT_CHANGED".to_string());
            }
            let Some(actual) = current_character
                .fields
                .iter()
                .find(|item| item.field == field)
            else {
                return Err("FINALIZED_CHARACTER_CONTEXT_CHANGED".to_string());
            };
            let decision = decide_derived_patch(
                actual,
                &DerivedCharacterPatch {
                    project_id: context.project_id.clone(),
                    epoch: context.epoch.clone(),
                    character_id: update.character_id.clone(),
                    field,
                    value: value.clone(),
                    value_hash: text_hash(value),
                    base_field_revision: base.revision,
                    base_value_hash: base.value_hash.clone(),
                    base_provenance: base.provenance.clone(),
                    source: context.source.clone(),
                    source_order: context.source_order.clone(),
                },
                &DerivedPatchAuthority {
                    source: current.source.clone(),
                    order: current.source_order.clone(),
                    origin_project_id: origin_project_id.map(str::to_string),
                },
            );
            match decision {
                DerivedPatchDecision::SourceConflict => {
                    return Err("FINALIZED_CHARACTER_SOURCE_CONFLICT".to_string())
                }
                DerivedPatchDecision::FieldConflict => {
                    return Err("FINALIZED_CHARACTER_FIELD_CONFLICT".to_string())
                }
                DerivedPatchDecision::AlreadyApplied => {
                    unchanged += 1;
                    continue;
                }
                DerivedPatchDecision::ProposalRequired => {
                    if value == &actual.value {
                        unchanged += 1;
                        continue;
                    }
                    let reason = if matches!(
                        actual.provenance,
                        CharacterStateFieldProvenance::Author { .. }
                    ) {
                        "author-protected"
                    } else {
                        "legacy-protected"
                    };
                    candidates.push(json!({
                        "candidateKey": character_state_candidate_key(
                            &context.source,
                            &update.character_id,
                            field,
                            value
                        ),
                        "source": &context.source,
                        "expectedFieldRevision": actual.revision,
                        "expectedFieldValueHash": &actual.value_hash,
                        "characterId": &update.character_id,
                        "characterName": &captured.display_name_snapshot,
                        "displayName": &captured.display_name_snapshot,
                        "selectionKey": format!("state:{}:{}", update.character_id, field_name(field)),
                        "field": field_name(field),
                        "value": value,
                        "evidence": &update.evidence,
                        "rawValue": &update.raw,
                        "reason": reason,
                    }));
                    continue;
                }
                DerivedPatchDecision::ApplyDerived => {
                    conn.execute(
                        &format!(
                            "UPDATE characters SET {}=?1 WHERE character_id=?2 AND retired=0",
                            state_column(field)
                        ),
                        params![value, update.character_id.as_str()],
                    )
                    .map_err(|error| error.to_string())?;
                    provenance.insert(
                        field_name(field).to_string(),
                        json!({
                            "kind": "derived",
                            "source": &context.source,
                            "revision": actual.revision + 1,
                            "sourceOrder": &context.source_order,
                        }),
                    );
                    applied += 1;
                    changed = true;
                }
            }
        }
        if changed {
            conn.execute(
                "UPDATE characters SET cs_provenance=?1, \
                 cs_updated_at_chapter=MAX(COALESCE(cs_updated_at_chapter,0),?2), \
                 updated_at=datetime('now') WHERE character_id=?3",
                params![
                    serde_json::to_string(&provenance).map_err(|error| error.to_string())?,
                    context.source.chapter_number,
                    update.character_id.as_str()
                ],
            )
            .map_err(|error| error.to_string())?;
        }
    }
    if !candidates.is_empty() {
        let request = SaveFinalizedCharacterStateCandidatesRequest {
            draft_id: context.source.draft_id,
            chapter_number: context.source.chapter_number,
            candidates: Some(candidates.clone()),
            projection_generation: context.projection_generation,
            source: context.source.clone(),
        };
        crate::repositories::finalized_continuity_repository::save_finalized_character_state_candidates_in_transaction(
            conn,
            &request,
        )?;
    }
    if applied > 0 {
        crate::repositories::character_roster_repository::refresh_character_state_projection(conn)?;
    }
    Ok(json!({
        "applied": applied,
        "unchanged": unchanged,
        "candidates": candidates,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== B99: 定稿角色状态领域提交 =====

    fn commit_in_tx(
        conn: &Connection,
        context: &FinalizedCharacterContext,
        response: &Value,
    ) -> Result<Value, String> {
        let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let receipt =
            commit_finalized_character_states_in_transaction(&tx, context, response, None)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(receipt)
    }

    #[test]
    fn parse_state_response_locates_evidence_and_validates_fields() {
        let (conn, draft_id) = setup();
        let context = read_finalized_character_context(&conn, draft_id, &scope()).unwrap();
        let parsed = parse_finalized_character_state_response(
            &json!({"updates": [{
                "characterId": "char-1",
                "currentState": { "recentEvents": "抵达仓库" },
                "evidence": { "text": "测试" }
            }]})
            .to_string(),
            &context,
        )
        .unwrap();
        assert_eq!(parsed.updates.len(), 1);
        assert_eq!(parsed.updates[0].character_id, "char-1");
        assert_eq!(parsed.updates[0].evidence["start"], json!(0));
        assert_eq!(parsed.updates[0].evidence["end"], json!(2));
        assert!(parsed.unresolved.is_empty());

        // 冻结名单外 / 名字-only 的出现保留为待确认。
        let parsed = parse_finalized_character_state_response(
            &json!({"updates": [{
                "name": "陌生人",
                "currentState": { "location": "某处" },
                "evidence": { "text": "正文" }
            }]})
            .to_string(),
            &context,
        )
        .unwrap();
        assert!(parsed.updates.is_empty());
        assert_eq!(parsed.unresolved.len(), 1);
        assert_eq!(parsed.unresolved[0]["reason"], json!("name-only"));
        assert_eq!(parsed.unresolved[0]["selectionKey"], json!("update:0"));

        // 证据越界 / 文本不匹配 / 字段非法 / 缺证据。
        for (payload, expected) in [
            (
                json!({"updates": [{"characterId": "char-1", "currentState": {"location": "北境"},
                    "evidence": {"start": 0, "end": 1, "text": "测试"}}]}),
                "FINALIZED_CHARACTER_EVIDENCE_INVALID",
            ),
            (
                json!({"updates": [{"characterId": "char-1", "currentState": {"location": "北境"},
                    "evidence": {"text": "不存在的引用"}}]}),
                "FINALIZED_CHARACTER_EVIDENCE_NOT_UNIQUE",
            ),
            (
                json!({"updates": [{"characterId": "char-1", "currentState": {"location": 7},
                    "evidence": {"text": "测试"}}]}),
                "FINALIZED_CHARACTER_FIELD_INVALID",
            ),
            (
                json!({"updates": [{"characterId": "char-1", "currentState": {},
                    "evidence": {"text": "测试"}}]}),
                "FINALIZED_CHARACTER_FIELD_INVALID",
            ),
            (
                json!({"updates": [{"characterId": "char-1", "currentState": {"location": "北境"}}]}),
                "FINALIZED_CHARACTER_EVIDENCE_MISSING",
            ),
            (json!({ "updates": [] }), ""),
        ] {
            let result = parse_finalized_character_state_response(&payload.to_string(), &context);
            if expected.is_empty() {
                assert!(result.unwrap().updates.is_empty());
            } else {
                assert_eq!(result.unwrap_err(), expected.to_string());
            }
        }
    }

    #[test]
    fn commit_applies_derived_fields_then_reports_already_applied() {
        let (conn, draft_id) = setup();
        let context = read_finalized_character_context(&conn, draft_id, &scope()).unwrap();
        // `mentalState` 当前为空且无来源 → 派生补丁可直接应用（作者值需确认）。
        let response = json!({"updates": [{
            "characterId": "char-1",
            "currentState": { "mentalState": "动摇" },
            "evidence": { "start": 0, "end": 2, "text": "测试" }
        }]});
        let receipt = commit_in_tx(&conn, &context, &response).unwrap();
        assert_eq!(receipt["applied"], json!(1));
        assert_eq!(receipt["unchanged"], json!(0));
        assert!(receipt["candidates"].as_array().unwrap().is_empty());
        let mental_state: String = conn
            .query_row(
                "SELECT cs_mental_state FROM characters WHERE character_id=?1",
                ["char-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mental_state, "动摇");
        let provenance: String = conn
            .query_row(
                "SELECT cs_provenance FROM characters WHERE character_id=?1",
                ["char-1"],
                |row| row.get(0),
            )
            .unwrap();
        let provenance: Value = serde_json::from_str(&provenance).unwrap();
        assert_eq!(provenance["mentalState"]["kind"], json!("derived"));
        assert_eq!(provenance["mentalState"]["revision"], json!(1));
        assert_eq!(
            provenance["mentalState"]["source"]["finalizationId"],
            json!(FINALIZATION_ID)
        );

        // 同一结果再次提交：派生值已应用，不再改写。
        let current = read_finalized_character_context(&conn, draft_id, &scope()).unwrap();
        let again = commit_in_tx(&conn, &current, &response).unwrap();
        assert_eq!(again["applied"], json!(0));
        assert_eq!(again["unchanged"], json!(1));
    }

    #[test]
    fn commit_keeps_author_protected_field_as_pending_candidate() {
        let (conn, draft_id) = setup();
        conn.execute(
            "UPDATE characters SET cs_location=?1, cs_provenance=?2 WHERE character_id=?3",
            params![
                "旧地",
                r#"{"location":{"kind":"author","chapterNumber":6,"revision":3}}"#,
                "char-1"
            ],
        )
        .unwrap();
        seed_candidate(&conn, draft_id);
        let context = read_finalized_character_context(&conn, draft_id, &scope()).unwrap();
        let response = json!({"updates": [{
            "characterId": "char-1",
            "currentState": { "location": "北境" },
            "evidence": { "start": 0, "end": 2, "text": "测试" }
        }]});
        let receipt = commit_in_tx(&conn, &context, &response).unwrap();
        assert_eq!(receipt["applied"], json!(0));
        let candidates = receipt["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0]["reason"], json!("author-protected"));
        assert_eq!(candidates[0]["expectedFieldRevision"], json!(3));
        assert_eq!(
            candidates[0]["expectedFieldValueHash"],
            json!(text_hash("旧地"))
        );
        assert_eq!(
            candidates[0]["selectionKey"],
            json!("state:char-1:location")
        );
        // 作者值未被改写，候选进入待决列表。
        let location: String = conn
            .query_row(
                "SELECT cs_location FROM characters WHERE character_id=?1",
                ["char-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(location, "旧地");
        assert_eq!(
            list_pending_state_candidates(&conn)
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn commit_rejects_unproven_identity_and_changed_context() {
        let (conn, draft_id) = setup();
        let context = read_finalized_character_context(&conn, draft_id, &scope()).unwrap();
        let stranger = json!({"updates": [{
            "name": "陌生人",
            "currentState": { "location": "某处" },
            "evidence": { "start": 0, "end": 2, "text": "测试" }
        }]});
        assert_eq!(
            commit_in_tx(&conn, &context, &stranger).unwrap_err(),
            "FINALIZED_CHARACTER_IDENTITY_REQUIRED".to_string()
        );

        // 冻结身份已变化（作者改名后重建快照）→ CONTEXT_CHANGED。
        let mut stale = context.clone();
        stale.identity_revision += 1;
        let ok = json!({"updates": [{
            "characterId": "char-1",
            "currentState": { "location": "北境" },
            "evidence": { "start": 0, "end": 2, "text": "测试" }
        }]});
        assert_eq!(
            commit_in_tx(&conn, &stale, &ok).unwrap_err(),
            "FINALIZED_CHARACTER_CONTEXT_CHANGED".to_string()
        );
    }

    const PROJECT_ID: &str = "project-1";
    const EPOCH: &str = "lease-1";
    const FINALIZATION_ID: &str = "fin-1";
    const CONTENT: &str = "测试正文";

    fn scope() -> ProjectEpoch {
        ProjectEpoch {
            project_id: PROJECT_ID.to_string(),
            epoch: EPOCH.to_string(),
        }
    }

    /// 最小完整库：定稿草稿 + 出箱 + 冻结身份快照 + 角色行。
    fn setup() -> (Connection, i64) {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        conn.execute(
            "UPDATE continuity_projection_meta SET generation=3 WHERE id=?1",
            params!["main"],
        )
        .unwrap();
        conn.execute("INSERT INTO contents (body) VALUES (?1)", params![CONTENT])
            .unwrap();
        let content_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, status, content_id) VALUES (7, 5, ?1, ?2)",
            params!["finalized", content_id],
        )
        .unwrap();
        let draft_id = conn.last_insert_rowid();
        let content_hash = text_hash(CONTENT);
        conn.execute(
            "INSERT INTO finalization_outbox (finalization_id, draft_id, chapter_number, chapter_title, content_hash, content_revision, content_snapshot, target_file_name) VALUES (?1, ?2, 7, ?3, ?4, 5, ?5, ?6)",
            params![
                FINALIZATION_ID,
                draft_id,
                "第七章",
                content_hash,
                CONTENT,
                "chapter-0007.md"
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO characters (character_id, name, role, cs_location, cs_recent_events, cs_provenance) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["char-1", "角色一", "supporting", "仓库", "抵达仓库", "{}"],
        )
        .unwrap();
        let raw_value = json!({
            "kind": "finalized-character-snapshot:v1",
            "source": {
                "draftId": draft_id,
                "finalizationId": FINALIZATION_ID,
                "chapterNumber": 7,
                "contentHash": content_hash,
            },
            "identityRevision": 1,
            "characters": [{
                "characterId": "char-1",
                "displayNameSnapshot": "角色一",
                "aliases": ["一"],
            }],
        })
        .to_string();
        conn.execute(
            "INSERT INTO character_identity_proposals (proposal_id, owner_character_id, source_key, source_hash, raw_value, candidate_ids_json) VALUES (?1, NULL, ?2, ?3, ?4, ?5)",
            params![
                format!("fcs:{}", FINALIZATION_ID),
                "finalized-character-snapshot:v1",
                text_hash(&raw_value),
                raw_value,
                "[]"
            ],
        )
        .unwrap();
        (conn, draft_id)
    }

    /// 写入一条未绑定元数据的待决候选（历史行形态）。
    fn seed_candidate(conn: &Connection, draft_id: i64) {
        let store = json!({
            "version": 2,
            "pending": [{
                "characterId": "char-1",
                "characterName": "角色一",
                "field": "location",
                "value": "北境",
                "evidence": { "start": 0, "end": 2, "text": "测试" },
            }],
            "receipts": [],
        });
        conn.execute(
            "INSERT INTO summary_snapshots (draft_id, chapter_number, character_state_candidates, source_finalization_id, source_content_hash, projection_generation) VALUES (?1, 7, ?2, ?3, ?4, 3)",
            params![draft_id, store.to_string(), FINALIZATION_ID, text_hash(CONTENT)],
        )
        .unwrap();
    }

    fn candidate_store(conn: &Connection, draft_id: i64) -> CharacterStateCandidateStore {
        let raw: String = conn
            .query_row(
                "SELECT character_state_candidates FROM summary_snapshots WHERE draft_id=?1",
                params![draft_id],
                |row| row.get(0),
            )
            .unwrap();
        parse_character_state_candidate_store(&raw)
    }

    #[test]
    fn candidate_key_and_payload_hash_are_stable_and_value_sensitive() {
        let source = FinalizedSourceIdentity {
            draft_id: 7,
            finalization_id: FINALIZATION_ID.to_string(),
            chapter_number: 7,
            content_hash: "a".repeat(64),
        };
        let key = character_state_candidate_key(
            &source,
            "char-1",
            CharacterStateTextField::Location,
            "北境",
        );
        assert!(key.starts_with("fcs-state:"));
        assert_eq!(
            key,
            character_state_candidate_key(
                &source,
                "char-1",
                CharacterStateTextField::Location,
                "北境"
            )
        );
        // 值 / 字段 / 角色任一变化都会换键。
        assert_ne!(
            key,
            character_state_candidate_key(
                &source,
                "char-1",
                CharacterStateTextField::Location,
                "南境"
            )
        );
        assert_ne!(
            key,
            character_state_candidate_key(
                &source,
                "char-1",
                CharacterStateTextField::RecentEvents,
                "北境"
            )
        );
        assert_ne!(
            key,
            character_state_candidate_key(
                &source,
                "char-2",
                CharacterStateTextField::Location,
                "北境"
            )
        );
        let request = json!({
            "draftId": 7,
            "candidateKey": key,
            "characterId": "char-1",
            "field": "location",
            "expectedFieldRevision": 0,
            "expectedFieldValueHash": "b".repeat(64),
            "operationId": "op-1",
            "decision": "accept",
        });
        let payload_hash = state_decision_payload_hash(&request);
        assert_eq!(payload_hash.len(), 64);
        assert_eq!(payload_hash, state_decision_payload_hash(&request));
        let mut other = request.clone();
        other["operationId"] = json!("op-2");
        assert_ne!(payload_hash, state_decision_payload_hash(&other));
    }

    #[test]
    fn list_read_and_decide_round_trip() {
        let (conn, draft_id) = setup();
        seed_candidate(&conn, draft_id);

        let list = list_pending_state_candidates(&conn).unwrap();
        let items = list.as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["draftId"], json!(draft_id));
        assert_eq!(items[0]["finalizationId"], json!(FINALIZATION_ID));
        assert_eq!(items[0]["characterId"], json!("char-1"));
        assert_eq!(items[0]["characterName"], json!("角色一"));
        assert_eq!(items[0]["field"], json!("location"));
        let key = items[0]["candidateKey"].as_str().unwrap().to_string();

        // 未绑定元数据的历史行在读取时升级写回。
        let candidate = read_pending_state_candidate(&conn, draft_id, &key).unwrap();
        assert_eq!(candidate["candidateKey"], json!(key));
        assert_eq!(candidate["value"], json!("北境"));
        assert_eq!(candidate["expectedFieldRevision"], json!(0));
        assert_eq!(
            candidate["expectedFieldValueHash"],
            json!(text_hash("仓库"))
        );
        let upgraded = candidate_store(&conn, draft_id);
        assert_eq!(
            upgraded.pending[0].candidate_key.as_deref(),
            Some(key.as_str())
        );
        assert_eq!(upgraded.pending[0].expected_field_revision, Some(0));
        assert!(upgraded.pending[0].source.is_some());

        let request = json!({
            "draftId": draft_id,
            "candidateKey": key,
            "characterId": "char-1",
            "field": "location",
            "expectedFieldRevision": 0,
            "expectedFieldValueHash": text_hash("仓库"),
            "operationId": "op-1",
            "decision": "accept",
        });
        let receipt = decide_state_candidate(&conn, &request).unwrap();
        assert_eq!(receipt["idempotent"], json!(false));
        assert_eq!(receipt["decision"], json!("accept"));
        assert_eq!(receipt["operationId"], json!("op-1"));

        // 作者决定落库：字段值 + author provenance（章节号 / 新版本号）。
        let location: String = conn
            .query_row(
                "SELECT cs_location FROM characters WHERE character_id=?1",
                params!["char-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(location, "北境");
        let provenance: String = conn
            .query_row(
                "SELECT cs_provenance FROM characters WHERE character_id=?1",
                params!["char-1"],
                |row| row.get(0),
            )
            .unwrap();
        let provenance: Value = serde_json::from_str(&provenance).unwrap();
        assert_eq!(provenance["location"]["kind"], json!("author"));
        assert_eq!(provenance["location"]["chapterNumber"], json!(7));
        assert_eq!(provenance["location"]["revision"], json!(1));

        // 候选出队、收据入库。
        let after = candidate_store(&conn, draft_id);
        assert!(after.pending.is_empty());
        assert_eq!(after.receipts.len(), 1);
        assert!(list_pending_state_candidates(&conn)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty());

        // 同 operationId 重放：同载荷幂等，异载荷冲突。
        let replay = decide_state_candidate(&conn, &request).unwrap();
        assert_eq!(replay["idempotent"], json!(true));
        assert_eq!(replay["payloadHash"], receipt["payloadHash"]);
        let mut conflict = request.clone();
        conflict["decision"] = json!("decline");
        assert_eq!(
            decide_state_candidate(&conn, &conflict).unwrap_err(),
            FINALIZED_CHARACTER_STATE_OPERATION_CONFLICT.to_string()
        );
        // 冲突不重复落库。
        let location: String = conn
            .query_row(
                "SELECT cs_location FROM characters WHERE character_id=?1",
                params!["char-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(location, "北境");
    }

    #[test]
    fn decide_rejects_stale_field_revision() {
        let (conn, draft_id) = setup();
        seed_candidate(&conn, draft_id);
        let list = list_pending_state_candidates(&conn).unwrap();
        let key = list[0]["candidateKey"].as_str().unwrap().to_string();
        // 读取会写回绑定元数据；其后字段变化才能触发 CAS 冲突（未绑定候选会
        // 在决策时按当前行重新计算，因而走不到冲突分支）。
        let _ = read_pending_state_candidate(&conn, draft_id, &key).unwrap();
        let request = json!({
            "draftId": draft_id,
            "candidateKey": key,
            "characterId": "char-1",
            "field": "location",
            "expectedFieldRevision": 0,
            "expectedFieldValueHash": text_hash("仓库"),
            "operationId": "op-stale",
            "decision": "accept",
        });
        // 读取之后字段被作者改到下一版本 → CAS 冲突。
        conn.execute(
            "UPDATE characters SET cs_location=?1, cs_provenance=?2 WHERE character_id=?3",
            params![
                "野外",
                r#"{"location":{"kind":"author","chapterNumber":6,"revision":1}}"#,
                "char-1"
            ],
        )
        .unwrap();
        assert_eq!(
            decide_state_candidate(&conn, &request).unwrap_err(),
            FINALIZED_CHARACTER_STATE_FIELD_CONFLICT.to_string()
        );
    }

    #[test]
    fn validation_and_source_failures() {
        let (conn, draft_id) = setup();
        assert_eq!(
            read_context(&conn, &scope(), 0).unwrap_err(),
            GENERATION_CHARACTER_DRAFT_INVALID.to_string()
        );
        assert_eq!(list_pending_state_candidates(&conn).unwrap(), json!([]));
        assert_eq!(
            read_pending_state_candidate(&conn, 0, "k").unwrap_err(),
            FINALIZED_CHARACTER_STATE_CANDIDATE_INVALID.to_string()
        );
        assert_eq!(
            read_pending_state_candidate(&conn, draft_id, "   ").unwrap_err(),
            FINALIZED_CHARACTER_STATE_CANDIDATE_INVALID.to_string()
        );
        // 缺少同代连续性投影行 → 来源失效。
        assert_eq!(
            read_pending_state_candidate(&conn, draft_id, "fcs-state:x").unwrap_err(),
            FINALIZED_CHARACTER_STATE_SOURCE_CHANGED.to_string()
        );
        // 决策请求形状校验。
        for invalid in [
            json!({ "draftId": 1 }),
            json!({ "draftId": 0, "candidateKey": "k", "characterId": "c", "field": "location",
                "expectedFieldRevision": 0, "expectedFieldValueHash": "a".repeat(64),
                "operationId": "op", "decision": "accept" }),
            json!({ "draftId": 1, "candidateKey": "", "characterId": "c", "field": "location",
                "expectedFieldRevision": 0, "expectedFieldValueHash": "a".repeat(64),
                "operationId": "op", "decision": "accept" }),
            json!({ "draftId": 1, "candidateKey": "k", "characterId": "c", "field": "updatedAt",
                "expectedFieldRevision": 0, "expectedFieldValueHash": "a".repeat(64),
                "operationId": "op", "decision": "accept" }),
            json!({ "draftId": 1, "candidateKey": "k", "characterId": "c", "field": "location",
                "expectedFieldRevision": 0, "expectedFieldValueHash": "short",
                "operationId": "op", "decision": "accept" }),
            json!({ "draftId": 1, "candidateKey": "k", "characterId": "c", "field": "location",
                "expectedFieldRevision": 0, "expectedFieldValueHash": "a".repeat(64),
                "operationId": "op", "decision": "maybe" }),
        ] {
            assert_eq!(
                decide_state_candidate(&conn, &invalid).unwrap_err(),
                FINALIZED_CHARACTER_STATE_DECISION_INVALID.to_string()
            );
        }
        // 未知候选键 → NOT_FOUND。
        seed_candidate(&conn, draft_id);
        assert_eq!(
            read_pending_state_candidate(&conn, draft_id, "fcs-state:missing").unwrap_err(),
            FINALIZED_CHARACTER_STATE_CANDIDATE_NOT_FOUND.to_string()
        );
    }

    #[test]
    fn stale_projection_hides_candidates_without_error() {
        let (conn, draft_id) = setup();
        seed_candidate(&conn, draft_id);
        // 投影水位推进且该章进入 stale 区间 → 候选行不再暴露（列表静默跳过）。
        conn.execute(
            "UPDATE continuity_projection_meta SET generation=4, stale_from_chapter=7 WHERE id=?1",
            params!["main"],
        )
        .unwrap();
        assert!(list_pending_state_candidates(&conn)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            read_pending_state_candidate(&conn, draft_id, "fcs-state:x").unwrap_err(),
            FINALIZED_CHARACTER_STATE_SOURCE_CHANGED.to_string()
        );
    }
}
