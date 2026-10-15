//! 定稿连续性投影（finalized continuity）—— 批次 E。
//!
//! 平移自 `electron/repositories/summary-repository.ts` 的定稿连续性段落 +
//! `continuity_projection_meta` 代际指针。领域约束（`docs/product-domain.md`）：
//! - 投影是**派生数据**，写入时必须绑定不可变定稿来源（outbox 收据 + 内容哈希）；
//! - `generation` 为全局失效水位：源正文变化即推进，过期结果按水位拒绝；
//! - 引文（`evidence`）必须能在绑定正文中精确定位，否则拒绝落盘。
//!
//! 身份域（m02 对齐，W2 审计 H6–H10）：
//! - 事实的 `characterRefs` 是唯一身份引用（展示名仅可读标签），写入时必须在
//!   冻结身份快照中解析（`FINALIZED_CHARACTER_IDENTITY_REQUIRED`）；
//! - 候选归一走身份分支：候选携带 `characterId`，按冻结快照校验展示名唯一性、
//!   evidence 跨度与角色在职状态；
//! - 列表的 `sourceStatus` 在来源当前时按身份绑定程度降级
//!   `current → legacy → stale`。
//! - ⚠️ Tauri 侧无上游 `freezeFinalizedCharacterSnapshot`（`fcs:` 提案行）的
//!   写入点，冻结身份快照以 `character_identity_repository::read_character_identity_snapshot`
//!   的实时身份快照为冻结源（语义疑点已登记于 W2 审计报告）。
//!
//! 与基线的一致性：错误文案逐字对齐；`parseFacts` / `parseCandidates` 对
//! 历史脏数据**静默降级**（解析失败返回空），只有入参校验失败才报错。
//!
use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::repositories::character_identity_repository::{
    has_character_identity_schema, read_character_identity_snapshot,
};
use crate::repositories::character_repository::{CharacterStateTextField, FinalizedSourceIdentity};
use crate::repositories::character_roster_repository as roster;

const FACT_CATEGORIES: [&str; 4] = ["character-state", "timeline", "open-thread", "plot"];

pub(crate) fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

// ===== 共享类型（对齐 `src/shared/finalized-continuity.ts`） =====

/// 定稿事实中的角色身份引用（`characterId` + 定稿时展示名快照）。
///
/// 展示名（`entities` 中的可读标签）本身不是身份关联；只有这里的 proven ID
/// 是身份引用（上游 `FinalizedCharacterReference`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedCharacterReference {
    pub character_id: String,
    pub display_name_snapshot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedContinuityFact {
    pub category: String,
    #[serde(default)]
    pub entities: Vec<String>,
    pub statement: String,
    pub source_chapter: i64,
    pub evidence: String,
    /// 可选身份引用；键缺失（`None`）时序列化省略该键。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_refs: Option<Vec<FinalizedCharacterReference>>,
}

/// 候选 evidence 跨度（偏移按 JS UTF-16 码元计，正文不可归一化）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedCharacterEvidence {
    pub start: i64,
    pub end: i64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedCharacterStateCandidate {
    /// 仅历史遗留定位器可缺省；新写入必须绑定身份。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_id: Option<String>,
    pub character_name: String,
    pub field: CharacterStateTextField,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<FinalizedCharacterEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_value: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// 当前源绑定决策才携带；历史保留行可缺省。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<FinalizedSourceIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_field_revision: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_field_value_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedSourceSnapshot {
    pub source: FinalizedSourceIdentity,
    pub chapter_title: String,
    pub content: String,
    pub projection_generation: i64,
}

/// 对齐契约 `FinalizedSourceReadResult`（discriminated union，`status` 为判别键）
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum FinalizedSourceReadResult {
    Valid {
        snapshot: FinalizedSourceSnapshot,
    },
    Legacy {
        draft_id: i64,
        chapter_number: i64,
        chapter_title: String,
        content: String,
    },
    Invalid,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedContinuityProjection {
    pub draft_id: i64,
    /// 来源过期但当前定稿稿可读时，指向当前可读草稿（仅渲染回退用）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_finalized_draft_id: Option<i64>,
    pub chapter_number: i64,
    pub chapter_title: String,
    pub chapter_notes: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facts: Option<Vec<FinalizedContinuityFact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_state_candidates: Option<Vec<FinalizedCharacterStateCandidate>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<FinalizedSourceIdentity>,
    pub source_status: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFinalizedContinuityRequest {
    pub draft_id: i64,
    pub chapter_number: i64,
    #[serde(default)]
    pub chapter_notes: String,
    #[serde(default)]
    pub facts: Option<Vec<serde_json::Value>>,
    pub projection_generation: i64,
    pub source: FinalizedSourceIdentity,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFinalizedCharacterStateCandidatesRequest {
    pub draft_id: i64,
    pub chapter_number: i64,
    #[serde(default)]
    pub candidates: Option<Vec<serde_json::Value>>,
    pub projection_generation: i64,
    pub source: FinalizedSourceIdentity,
}

// ===== 候选存储（对齐基线 `CharacterStateCandidateStore`） =====

/// 候选决策收据（对齐 `StoredCharacterStateDecision`，不含 `idempotent`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredCharacterStateDecisionReceipt {
    pub(crate) candidate_key: String,
    pub(crate) operation_id: String,
    pub(crate) payload_hash: String,
    pub(crate) decision: CharacterStateDecision,
    pub(crate) source: FinalizedSourceIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CharacterStateDecision {
    Accept,
    Decline,
}

/// 候选持久化形态：`{version: 2, pending, receipts}`（旧纯数组格式读时兼容）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CharacterStateCandidateStore {
    pub(crate) version: i64,
    pub(crate) pending: Vec<FinalizedCharacterStateCandidate>,
    pub(crate) receipts: Vec<StoredCharacterStateDecisionReceipt>,
}

impl CharacterStateCandidateStore {
    fn empty() -> Self {
        CharacterStateCandidateStore {
            version: 2,
            pending: Vec::new(),
            receipts: Vec::new(),
        }
    }
}

// ===== 内部工具 =====

fn same_source(left: &FinalizedSourceIdentity, right: &FinalizedSourceIdentity) -> bool {
    left == right
}

/// 对齐基线 `isFinalizedSource`：判别一个 JSON 值是否为合法定稿来源收据。
fn is_finalized_source_json(value: &serde_json::Value) -> bool {
    value
        .get("draftId")
        .and_then(|v| v.as_i64())
        .is_some_and(|id| id > 0)
        && value
            .get("chapterNumber")
            .and_then(|v| v.as_i64())
            .is_some_and(|number| number > 0)
        && value
            .get("finalizationId")
            .and_then(|v| v.as_str())
            .is_some_and(|id| !id.is_empty())
        && value
            .get("contentHash")
            .and_then(|v| v.as_str())
            .is_some_and(|hash| is_lowercase_hex64(hash))
}

pub(crate) fn is_lowercase_hex64(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// JS `String.length`（UTF-16 码元数；非 BMP 字符计 2 码元）。
fn utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}

/// 对齐基线 `normalizedFacts`：逐条宽松校验后规范化（实体去重保序）
fn normalized_facts(
    value: &[serde_json::Value],
    chapter_number: i64,
) -> Result<Vec<FinalizedContinuityFact>, String> {
    if value.len() > 12 {
        return Err("连续性事实参数无效".to_string());
    }
    value
        .iter()
        .map(|fact| {
            let category = fact.get("category").and_then(|v| v.as_str()).unwrap_or("");
            let source_chapter = fact.get("sourceChapter").and_then(|v| v.as_i64());
            let entities: Vec<String> = match fact.get("entities") {
                Some(serde_json::Value::Array(arr)) => arr
                    .iter()
                    .map(|e| e.as_str().unwrap_or("").trim().to_string())
                    .collect(),
                _ => Vec::new(),
            };
            let statement = fact
                .get("statement")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let evidence = fact
                .get("evidence")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            // 上游无陈述/引文长度上限；entity 长度按 UTF-16 码元计（>80 拒绝）。
            if !FACT_CATEGORIES.contains(&category)
                || source_chapter != Some(chapter_number)
                || entities.len() > 8
                || entities.iter().any(|e| e.is_empty() || utf16_len(e) > 80)
                || statement.is_empty()
                || evidence.is_empty()
            {
                return Err("连续性事实参数无效".to_string());
            }
            // characterRefs 可选数组：每项须 {characterId: 非空 string,
            // displayNameSnapshot: string} 且 displayNameSnapshot ∈ entities。
            let character_refs = match fact.get("characterRefs") {
                Some(serde_json::Value::Array(refs)) => Some(
                    refs.iter()
                        .map(|value| {
                            let character_id = value
                                .get("characterId")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let display_name_snapshot = value
                                .get("displayNameSnapshot")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if character_id.trim().is_empty()
                                || !entities.contains(&display_name_snapshot.to_string())
                            {
                                return Err("连续性角色身份参数无效".to_string());
                            }
                            Ok(FinalizedCharacterReference {
                                character_id: character_id.to_string(),
                                display_name_snapshot: display_name_snapshot.to_string(),
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                ),
                _ => None,
            };
            let mut seen = HashSet::new();
            let unique_entities = entities
                .into_iter()
                .filter(|e| seen.insert(e.clone()))
                .collect();
            Ok(FinalizedContinuityFact {
                category: category.to_string(),
                entities: unique_entities,
                statement,
                source_chapter: chapter_number,
                evidence,
                character_refs,
            })
        })
        .collect()
}

/// 对齐基线 `parseFacts`：历史脏数据静默降级为空
fn parse_facts(value: &str, chapter_number: i64) -> Vec<FinalizedContinuityFact> {
    match serde_json::from_str::<Vec<serde_json::Value>>(value) {
        Ok(items) => normalized_facts(&items, chapter_number).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn field_matches(field: &CharacterStateTextField, text: &str) -> bool {
    let expected = serde_json::to_value(field)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    expected == text
}

/// 候选扩展字段提取（对齐基线 `parseCharacterStateCandidates` 的保留规则）。
#[derive(Debug, Clone, Default)]
struct CandidateOptionalFields {
    character_id: Option<String>,
    evidence: Option<FinalizedCharacterEvidence>,
    selection_key: Option<String>,
    display_name: Option<String>,
    raw_value: Option<serde_json::Value>,
    reason: Option<String>,
    candidate_key: Option<String>,
    source: Option<FinalizedSourceIdentity>,
    expected_field_revision: Option<i64>,
    expected_field_value_hash: Option<String>,
}

fn extract_optional_fields(input: &serde_json::Value) -> CandidateOptionalFields {
    CandidateOptionalFields {
        character_id: input
            .get("characterId")
            .and_then(|v| v.as_str())
            .filter(|id| !id.is_empty())
            .map(str::to_string),
        evidence: input
            .get("evidence")
            .filter(|v| !v.is_null())
            .and_then(|v| serde_json::from_value::<FinalizedCharacterEvidence>(v.clone()).ok()),
        selection_key: input
            .get("selectionKey")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        display_name: input
            .get("displayName")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        raw_value: input.get("rawValue").cloned(),
        reason: match input.get("reason").and_then(|v| v.as_str()) {
            Some("author-protected") | Some("legacy-protected") => input
                .get("reason")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            _ => None,
        },
        candidate_key: input
            .get("candidateKey")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        source: input
            .get("source")
            .filter(|v| is_finalized_source_json(v))
            .and_then(|v| serde_json::from_value::<FinalizedSourceIdentity>(v.clone()).ok()),
        expected_field_revision: input
            .get("expectedFieldRevision")
            .and_then(|v| v.as_i64())
            .filter(|revision| *revision >= 0),
        expected_field_value_hash: input
            .get("expectedFieldValueHash")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    }
}

/// 对齐基线 `parseCharacterStateCandidates` 的单条保留规则：扩展字段原样保留；
/// `value` 仅在未绑定 source 时 trim。
fn parse_preserved_candidate(
    input: &serde_json::Value,
) -> Option<FinalizedCharacterStateCandidate> {
    let bound_keys = [
        "candidateKey",
        "source",
        "expectedFieldRevision",
        "expectedFieldValueHash",
    ];
    let has_bound_metadata = bound_keys.iter().any(|key| input.get(*key).is_some());
    let has_valid_bound_metadata = {
        let candidate_key = input.get("candidateKey").and_then(|v| v.as_str());
        let source = input.get("source");
        let revision = input.get("expectedFieldRevision").and_then(|v| v.as_i64());
        let hash = input.get("expectedFieldValueHash").and_then(|v| v.as_str());
        candidate_key.is_some_and(|key| !key.is_empty())
            && source.is_some_and(is_finalized_source_json)
            && revision.is_some_and(|value| value >= 0)
            && hash.is_some_and(is_lowercase_hex64)
    };
    if has_bound_metadata && !has_valid_bound_metadata {
        return None;
    }
    let character_name = input.get("characterName").and_then(|v| v.as_str())?;
    if character_name.trim().is_empty() {
        return None;
    }
    let field_text = input.get("field").and_then(|v| v.as_str())?;
    let field = CharacterStateTextField::ALL
        .iter()
        .copied()
        .find(|f| field_matches(f, field_text))?;
    let value = input.get("value").and_then(|v| v.as_str())?;
    let optional = extract_optional_fields(input);
    let source_bound = optional.source.is_some();
    Some(FinalizedCharacterStateCandidate {
        character_id: optional.character_id,
        character_name: character_name.trim().to_string(),
        field,
        value: if source_bound {
            value.to_string()
        } else {
            value.trim().to_string()
        },
        evidence: optional.evidence,
        selection_key: optional.selection_key,
        display_name: optional.display_name,
        raw_value: optional.raw_value,
        reason: optional.reason,
        candidate_key: optional.candidate_key,
        source: optional.source,
        expected_field_revision: optional.expected_field_revision,
        expected_field_value_hash: optional.expected_field_value_hash,
    })
}

/// 对齐基线 `parseCharacterStateCandidates`：兼容旧数组与 v2 store 两种格式。
fn parse_character_state_candidates(value: &str) -> Vec<FinalizedCharacterStateCandidate> {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(value) else {
        return Vec::new();
    };
    let pending: Vec<&serde_json::Value> = match &parsed {
        serde_json::Value::Array(items) => items.iter().collect(),
        serde_json::Value::Object(map)
            if map.get("version").and_then(|v| v.as_i64()) == Some(2)
                && matches!(map.get("pending"), Some(serde_json::Value::Array(_))) =>
        {
            match map.get("pending") {
                Some(serde_json::Value::Array(items)) => items.iter().collect(),
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    };
    pending
        .iter()
        .filter_map(|input| parse_preserved_candidate(input))
        .collect()
}

/// 对齐基线 `parseCharacterStateCandidateStore` 的单条收据保留规则。
fn parse_preserved_receipt(
    input: &serde_json::Value,
) -> Option<StoredCharacterStateDecisionReceipt> {
    let candidate_key = input.get("candidateKey").and_then(|v| v.as_str())?;
    let operation_id = input.get("operationId").and_then(|v| v.as_str())?;
    let payload_hash = input.get("payloadHash").and_then(|v| v.as_str())?;
    let decision = match input.get("decision").and_then(|v| v.as_str()) {
        Some("accept") => CharacterStateDecision::Accept,
        Some("decline") => CharacterStateDecision::Decline,
        _ => return None,
    };
    let source = input
        .get("source")
        .filter(|v| is_finalized_source_json(v))
        .and_then(|v| serde_json::from_value::<FinalizedSourceIdentity>(v.clone()).ok())?;
    if candidate_key.is_empty() || operation_id.is_empty() || !is_lowercase_hex64(payload_hash) {
        return None;
    }
    Some(StoredCharacterStateDecisionReceipt {
        candidate_key: candidate_key.to_string(),
        operation_id: operation_id.to_string(),
        payload_hash: payload_hash.to_string(),
        decision,
        source,
    })
}

/// 对齐基线 `parseCharacterStateCandidateStore`：旧数组与 v2 store 两种持久化
/// 格式均可解析；非法形态降级为空 store（不报错）。
pub(crate) fn parse_character_state_candidate_store(value: &str) -> CharacterStateCandidateStore {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(value) else {
        return CharacterStateCandidateStore::empty();
    };
    if matches!(parsed, serde_json::Value::Array(_)) {
        return CharacterStateCandidateStore {
            version: 2,
            pending: parse_character_state_candidates(value),
            receipts: Vec::new(),
        };
    }
    let serde_json::Value::Object(map) = &parsed else {
        return CharacterStateCandidateStore::empty();
    };
    let valid_shape = map.get("version").and_then(|v| v.as_i64()) == Some(2)
        && matches!(map.get("pending"), Some(serde_json::Value::Array(_)))
        && matches!(map.get("receipts"), Some(serde_json::Value::Array(_)));
    if !valid_shape {
        return CharacterStateCandidateStore::empty();
    }
    let receipts = match map.get("receipts") {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|input| parse_preserved_receipt(input))
            .collect(),
        _ => Vec::new(),
    };
    CharacterStateCandidateStore {
        version: 2,
        pending: parse_character_state_candidates(value),
        receipts,
    }
}

/// 对齐基线 `normalizeCharacterStateCandidates`：身份 schema 恒存时走身份分支
/// （上游 :219-234），否则走上游遗留分支（按角色名单身份键归一，原样保留）。
fn normalize_character_state_candidates(
    conn: &Connection,
    value: &[serde_json::Value],
    source: Option<&FinalizedSourceIdentity>,
) -> Result<Vec<FinalizedCharacterStateCandidate>, String> {
    if value.len() > 1_000 {
        return Err("角色状态候选参数无效".to_string());
    }
    if has_character_identity_schema(conn) {
        // ===== 身份分支（上游 :219-234） =====
        let Some(source) = source else {
            return Err("FINALIZED_CHARACTER_SNAPSHOT_REQUIRED".to_string());
        };
        // 冻结身份快照：Tauri 侧以身份仓储的实时快照为冻结源（见文件头注记）。
        let identities = read_character_identity_snapshot(conn)
            .map_err(|_| "FINALIZED_CHARACTER_SNAPSHOT_REQUIRED".to_string())?;
        let snapshot = read_finalized_source_from_db(conn, source.draft_id)?;
        let Some(snapshot) = snapshot else {
            return Err("FINALIZED_CHARACTER_EVIDENCE_INVALID".to_string());
        };
        return value
            .iter()
            .map(|input| {
                let character_id = input.get("characterId").and_then(|v| v.as_str());
                let character = identities
                    .characters
                    .iter()
                    .find(|item| Some(item.character_id.as_str()) == character_id);
                let Some(character) = character else {
                    return Err("FINALIZED_CHARACTER_IDENTITY_REQUIRED".to_string());
                };
                let character_name = input.get("characterName").and_then(|v| v.as_str());
                let field_text = input.get("field").and_then(|v| v.as_str()).unwrap_or("");
                let Some(field) = CharacterStateTextField::ALL
                    .iter()
                    .copied()
                    .find(|f| field_matches(f, field_text))
                else {
                    return Err("FINALIZED_CHARACTER_IDENTITY_REQUIRED".to_string());
                };
                let Some(value_text) = input.get("value").and_then(|v| v.as_str()) else {
                    return Err("FINALIZED_CHARACTER_IDENTITY_REQUIRED".to_string());
                };
                if character_name != Some(character.name.as_str())
                    || identities
                        .characters
                        .iter()
                        .filter(|item| item.name == character.name)
                        .count()
                        != 1
                {
                    return Err("FINALIZED_CHARACTER_IDENTITY_REQUIRED".to_string());
                }
                // evidence 跨度校验：start/end 为安全整数、0 ≤ start < end ≤
                // 正文长度（UTF-16 码元），且 slice 文本精确匹配。
                let evidence = input.get("evidence");
                let evidence_start = evidence
                    .and_then(|item| item.get("start"))
                    .and_then(|v| v.as_i64());
                let evidence_end = evidence
                    .and_then(|item| item.get("end"))
                    .and_then(|v| v.as_i64());
                let evidence_text = evidence
                    .and_then(|item| item.get("text"))
                    .and_then(|v| v.as_str());
                let units: Vec<u16> = snapshot.content.encode_utf16().collect();
                let evidence_valid = match (evidence, evidence_start, evidence_end, evidence_text) {
                    (Some(_), Some(start), Some(end), Some(text)) => {
                        start >= 0
                            && end > start
                            && (end as usize) <= units.len()
                            && String::from_utf16(&units[start as usize..end as usize])
                                .map(|slice| slice == text)
                                .unwrap_or(false)
                    }
                    _ => false,
                };
                if !evidence_valid {
                    return Err("FINALIZED_CHARACTER_EVIDENCE_INVALID".to_string());
                }
                if conn
                    .query_row(
                        "SELECT 1 FROM characters WHERE character_id = ?1 AND retired = 0",
                        [character.character_id.as_str()],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?
                    .is_none()
                {
                    return Err("FINALIZED_CHARACTER_ID_RETIRED".to_string());
                }
                // 身份分支原样保留候选全部字段（上游 structuredClone）。
                let optional = extract_optional_fields(input);
                Ok(FinalizedCharacterStateCandidate {
                    character_id: optional.character_id,
                    character_name: character_name.unwrap_or_default().to_string(),
                    field,
                    value: value_text.to_string(),
                    evidence: optional.evidence,
                    selection_key: optional.selection_key,
                    display_name: optional.display_name,
                    raw_value: optional.raw_value,
                    reason: optional.reason,
                    candidate_key: optional.candidate_key,
                    source: optional.source,
                    expected_field_revision: optional.expected_field_revision,
                    expected_field_value_hash: optional.expected_field_value_hash,
                })
            })
            .collect();
    }

    // ===== 遗留分支（上游 else :241-262，按角色名单身份键归一） =====
    let mut stmt = conn
        .prepare("SELECT name FROM characters")
        .map_err(|e| e.to_string())?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let mut by_identity = std::collections::HashMap::new();
    for name in names {
        let identity = roster::identity_key(&name);
        if identity.is_empty() || by_identity.contains_key(&identity) {
            return Err("角色名单存在同名冲突".to_string());
        }
        by_identity.insert(identity, name);
    }
    let mut normalized = std::collections::HashMap::new();
    for input in value {
        let character_name = input
            .get("characterName")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let identity = roster::identity_key(&character_name);
        let stored_name = by_identity.get(&identity).cloned();
        let field_text = input.get("field").and_then(|v| v.as_str()).unwrap_or("");
        let field = CharacterStateTextField::ALL
            .iter()
            .copied()
            .find(|f| field_matches(f, field_text));
        let value_text = input.get("value").and_then(|v| v.as_str());
        let (Some(stored_name), Some(field), Some(value_text)) = (stored_name, field, value_text)
        else {
            return Err("角色状态候选参数无效".to_string());
        };
        let candidate = FinalizedCharacterStateCandidate {
            character_name: stored_name,
            field,
            value: value_text.trim().to_string(),
            character_id: None,
            evidence: None,
            selection_key: None,
            display_name: None,
            raw_value: None,
            reason: None,
            candidate_key: None,
            source: None,
            expected_field_revision: None,
            expected_field_value_hash: None,
        };
        normalized.insert(
            format!(
                "{}\u{0}{}",
                roster::identity_key(&candidate.character_name),
                serde_json::to_value(candidate.field)
                    .unwrap()
                    .as_str()
                    .unwrap_or("")
            ),
            candidate,
        );
    }
    Ok(normalized.into_values().collect())
}

/// 对齐基线 `readFinalizedSourceFromDb`：正文/快照/哈希三重一致性校验
pub(crate) fn read_finalized_source_from_db(
    conn: &Connection,
    draft_id: i64,
) -> Result<Option<FinalizedSourceSnapshot>, String> {
    let row = conn
        .query_row(
            "SELECT drafts.chapter_number, drafts.status,
                    contents.body, finalization_outbox.finalization_id,
                    finalization_outbox.chapter_title,
                    finalization_outbox.content_hash,
                    finalization_outbox.content_snapshot,
                    continuity_projection_meta.generation
             FROM drafts
             JOIN contents ON contents.id = drafts.content_id
             JOIN finalization_outbox ON finalization_outbox.draft_id = drafts.id
             JOIN continuity_projection_meta ON continuity_projection_meta.id = 'main'
             WHERE drafts.id = ?1",
            [draft_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((
        chapter_number,
        status,
        content,
        finalization_id,
        chapter_title,
        content_hash,
        content_snapshot,
        generation,
    )) = row
    else {
        return Ok(None);
    };
    let content_hash = content_hash.trim().to_string();
    let valid_hash = content_hash.len() == 64
        && content_hash
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
    if status != "finalized"
        || finalization_id.trim().is_empty()
        || !valid_hash
        || content != content_snapshot
        || sha256_hex(&content_snapshot) != content_hash
    {
        return Ok(None);
    }
    Ok(Some(FinalizedSourceSnapshot {
        source: FinalizedSourceIdentity {
            draft_id,
            finalization_id,
            chapter_number,
            content_hash,
        },
        chapter_title,
        content: content_snapshot,
        projection_generation: generation,
    }))
}

// ===== 领域操作 =====

/// 基线 `SummaryRepository.saveFinalizedContinuity`
pub fn save_finalized_continuity(
    conn: &Connection,
    request: &SaveFinalizedContinuityRequest,
) -> Result<(), String> {
    let chapter_notes = request.chapter_notes.trim().to_string();
    let empty: Vec<serde_json::Value> = Vec::new();
    let normalized = normalized_facts(
        request.facts.as_ref().unwrap_or(&empty),
        request.chapter_number,
    )?;
    let facts = serde_json::to_string(&normalized).map_err(|e| e.to_string())?;
    if request.draft_id < 1
        || request.chapter_number < 1
        || request.projection_generation < 0
        || chapter_notes.is_empty()
    {
        return Err("连续性投影参数无效".to_string());
    }
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let snapshot = read_finalized_source_from_db(conn, request.draft_id)?;
    let Some(snapshot) = snapshot else {
        return Err("连续性投影来源已失效，已拒绝过期结果".to_string());
    };
    if !same_source(&snapshot.source, &request.source)
        || request.chapter_number != snapshot.source.chapter_number
    {
        return Err("连续性投影来源已失效，已拒绝过期结果".to_string());
    }
    if normalized
        .iter()
        .any(|fact| !snapshot.content.contains(&fact.evidence))
    {
        return Err("连续性事实引文无法在绑定定稿正文中精确定位".to_string());
    }
    // 身份绑定校验（上游 :591-604，identity schema 恒存）：每个 characterRef
    // 必须在冻结身份快照中解析；命中角色展示名的 entity 必须唯一解析到恰好
    // 一个角色。身份快照读取失败按基线「无快照」语义处理（characterRef 无法
    // 解析即拒绝；无 characterRef 的事实不受影响）。
    let identities = read_character_identity_snapshot(conn).ok();
    for fact in &normalized {
        let refs_unresolved = fact.character_refs.as_deref().is_some_and(|refs| {
            refs.iter().any(|reference| {
                !identities.as_ref().is_some_and(|identities| {
                    identities.characters.iter().any(|character| {
                        character.character_id == reference.character_id
                            && character.name == reference.display_name_snapshot
                    })
                })
            })
        });
        if refs_unresolved {
            return Err("FINALIZED_CHARACTER_IDENTITY_REQUIRED".to_string());
        }
        let entity_ambiguous = fact.entities.iter().any(|entity| {
            let Some(identities) = identities.as_ref() else {
                return false;
            };
            if !identities
                .characters
                .iter()
                .any(|character| character.name == *entity)
            {
                return false;
            }
            let refs: Vec<&FinalizedCharacterReference> = fact
                .character_refs
                .as_deref()
                .unwrap_or_default()
                .iter()
                .filter(|reference| reference.display_name_snapshot == *entity)
                .collect();
            refs.len() != 1
                || !identities.characters.iter().any(|character| {
                    character.character_id == refs[0].character_id && character.name == *entity
                })
                || identities
                    .characters
                    .iter()
                    .filter(|character| character.name == *entity)
                    .count()
                    != 1
        });
        if entity_ambiguous {
            return Err("FINALIZED_CHARACTER_IDENTITY_REQUIRED".to_string());
        }
    }
    let generation: i64 = tx
        .query_row(
            "SELECT generation FROM continuity_projection_meta WHERE id = 'main'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if request.projection_generation != generation {
        return Err("连续性投影失效水位已推进，已拒绝过期结果".to_string());
    }
    let updated = tx
        .execute(
            "UPDATE summary_snapshots
             SET chapter_number = ?1, chapter_notes = ?2, continuity_facts = ?3,
                 source_finalization_id = ?4, source_content_hash = ?5, projection_generation = ?6,
                 created_at = datetime('now')
             WHERE draft_id = ?7",
            rusqlite::params![
                request.chapter_number,
                chapter_notes,
                facts,
                request.source.finalization_id,
                request.source.content_hash,
                request.projection_generation,
                request.draft_id,
            ],
        )
        .map_err(|e| e.to_string())?;
    if updated == 0 {
        tx.execute(
            "INSERT INTO summary_snapshots (
                draft_id, chapter_number, chapter_notes, continuity_facts,
                source_finalization_id, source_content_hash, projection_generation
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                request.draft_id,
                request.chapter_number,
                chapter_notes,
                facts,
                request.source.finalization_id,
                request.source.content_hash,
                request.projection_generation,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

/// 基线 `SummaryRepository.saveFinalizedCharacterStateCandidates`
pub fn save_finalized_character_state_candidates(
    conn: &Connection,
    request: &SaveFinalizedCharacterStateCandidatesRequest,
) -> Result<(), String> {
    if request.draft_id < 1 || request.chapter_number < 1 || request.projection_generation < 0 {
        return Err("角色状态候选参数无效".to_string());
    }
    let empty: Vec<serde_json::Value> = Vec::new();
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let snapshot = read_finalized_source_from_db(conn, request.draft_id)?;
    let Some(snapshot) = snapshot else {
        return Err("角色状态候选来源已失效，已拒绝过期结果".to_string());
    };
    if !same_source(&snapshot.source, &request.source)
        || request.chapter_number != snapshot.source.chapter_number
    {
        return Err("角色状态候选来源已失效，已拒绝过期结果".to_string());
    }
    if request.projection_generation != snapshot.projection_generation {
        return Err("连续性投影失效水位已推进，已拒绝过期结果".to_string());
    }
    let row = tx
        .query_row(
            "SELECT character_state_candidates, source_finalization_id,
                    source_content_hash, projection_generation
             FROM summary_snapshots WHERE draft_id = ?1",
            [request.draft_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((candidates_json, finalization_id, content_hash, projection_generation)) = row else {
        return Err("角色状态候选缺少同代定稿连续性投影".to_string());
    };
    if finalization_id != request.source.finalization_id
        || content_hash != request.source.content_hash
        || projection_generation != request.projection_generation
    {
        return Err("角色状态候选缺少同代定稿连续性投影".to_string());
    }

    // 合并（上游 :668-681）：v2 store 解析 → 身份分支归一 → 跳过已被 decision
    // receipt 覆盖的候选键 → 按 characterId（或 legacy 身份键）+ field 合并。
    let store = parse_character_state_candidate_store(&candidates_json);
    let incoming = normalize_character_state_candidates(
        conn,
        request.candidates.as_ref().unwrap_or(&empty),
        Some(&request.source),
    )?;
    let mut merged = std::collections::BTreeMap::new();
    for candidate in store.pending.into_iter().chain(incoming) {
        let covered_by_receipt = candidate.candidate_key.as_deref().is_some_and(|key| {
            store
                .receipts
                .iter()
                .any(|receipt| receipt.candidate_key == key)
        });
        if covered_by_receipt {
            continue;
        }
        let identity_key = match &candidate.character_id {
            Some(character_id) => character_id.clone(),
            None => format!("legacy:{}", roster::identity_key(&candidate.character_name)),
        };
        let field_text = serde_json::to_value(candidate.field)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        merged.insert(format!("{}\u{0}{}", identity_key, field_text), candidate);
    }
    let updated_store = CharacterStateCandidateStore {
        version: 2,
        pending: merged.into_values().collect(),
        receipts: store.receipts,
    };
    tx.execute(
        "UPDATE summary_snapshots
         SET character_state_candidates = ?1, created_at = datetime('now')
         WHERE draft_id = ?2",
        rusqlite::params![
            serde_json::to_string(&updated_store).map_err(|e| e.to_string())?,
            request.draft_id
        ],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

/// 基线 `SummaryRepository.listFinalizedContinuityBefore`
///
/// 「当前草稿」语义（上游 :720-749）：LEFT JOIN 同章当前最新定稿稿（与快照所属
/// 草稿无关）——被取代的旧快照行仍返回，`sourceStatus` 计为 `stale`；当前定稿稿
/// 被撤销时 current 列为 NULL。`newer_summary` 去重：同章只回最新快照行。
pub fn list_finalized_continuity_before(
    conn: &Connection,
    chapter_number: i64,
) -> Result<Vec<FinalizedContinuityProjection>, String> {
    if chapter_number < 1 {
        return Err("连续性投影目标章节无效".to_string());
    }
    let mut stmt = conn
        .prepare(
            "SELECT summary_snapshots.draft_id,
                    summary_snapshots.chapter_number,
                    COALESCE(current_outbox.chapter_title, '') AS chapter_title,
                    summary_snapshots.chapter_notes,
                    summary_snapshots.continuity_facts,
                    summary_snapshots.character_state_candidates,
                    summary_snapshots.source_finalization_id,
                    summary_snapshots.source_content_hash,
                    summary_snapshots.projection_generation,
                    current_drafts.id AS current_draft_id,
                    current_drafts.status AS current_draft_status,
                    current_outbox.finalization_id AS current_finalization_id,
                    current_outbox.content_hash AS current_content_hash,
                    current_outbox.content_snapshot AS content_snapshot,
                    current_contents.body AS current_content,
                    continuity_projection_meta.generation AS current_generation,
                    continuity_projection_meta.stale_from_chapter AS stale_from_chapter
             FROM summary_snapshots
             JOIN drafts ON drafts.id = summary_snapshots.draft_id
             JOIN contents ON contents.id = drafts.content_id
             LEFT JOIN drafts current_drafts ON current_drafts.chapter_number = drafts.chapter_number
               AND current_drafts.status = 'finalized'
               AND NOT EXISTS (
                 SELECT 1 FROM drafts newer
                 WHERE newer.chapter_number = current_drafts.chapter_number
                   AND newer.status = 'finalized'
                   AND (newer.version > current_drafts.version OR (newer.version = current_drafts.version AND newer.id > current_drafts.id))
               )
             LEFT JOIN contents current_contents ON current_contents.id = current_drafts.content_id
             LEFT JOIN finalization_outbox current_outbox ON current_outbox.draft_id = current_drafts.id
             JOIN continuity_projection_meta ON continuity_projection_meta.id = 'main'
             WHERE summary_snapshots.draft_id IS NOT NULL
               AND summary_snapshots.chapter_number < ?1
               AND summary_snapshots.chapter_notes <> ''
               AND drafts.status = 'finalized'
               AND NOT EXISTS (
                 SELECT 1 FROM summary_snapshots newer_summary
                 WHERE newer_summary.chapter_number = summary_snapshots.chapter_number
                   AND newer_summary.chapter_notes <> '' AND newer_summary.id > summary_snapshots.id
               )
             ORDER BY summary_snapshots.chapter_number ASC, summary_snapshots.draft_id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([chapter_number], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<String>>(12)?,
                row.get::<_, Option<String>>(13)?,
                row.get::<_, Option<String>>(14)?,
                row.get::<_, i64>(15)?,
                row.get::<_, Option<i64>>(16)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(
            |(
                draft_id,
                chapter_number,
                chapter_title,
                chapter_notes,
                facts_json,
                candidates_json,
                source_finalization_id,
                source_content_hash,
                projection_generation,
                current_draft_id,
                current_draft_status,
                current_finalization_id,
                current_content_hash,
                content_snapshot,
                current_content,
                current_generation,
                stale_from_chapter,
            )| {
                let has_bound_source = source_finalization_id
                    .as_deref()
                    .is_some_and(|s| !s.is_empty())
                    && source_content_hash
                        .as_deref()
                        .is_some_and(|s| !s.is_empty());
                let source_current = has_bound_source
                    && current_finalization_id.as_deref() == source_finalization_id.as_deref()
                    && current_content_hash.as_deref() == source_content_hash.as_deref()
                    && content_snapshot == current_content
                    && current_content.as_deref().is_some_and(|content| {
                        sha256_hex(content) == current_content_hash.as_deref().unwrap_or("")
                    });
                let fallback_current = current_draft_id.is_some_and(|id| id > 0)
                    && current_draft_status.as_deref() == Some("finalized")
                    && current_content.is_some()
                    && content_snapshot.is_some()
                    && content_snapshot == current_content
                    && current_content_hash.is_some()
                    && current_content.as_deref().is_some_and(|content| {
                        sha256_hex(content) == current_content_hash.as_deref().unwrap_or("")
                    });
                let invalidated = stale_from_chapter.is_some_and(|stale| {
                    chapter_number >= stale && projection_generation < current_generation
                });
                let facts = parse_facts(&facts_json, chapter_number);
                let candidates = parse_character_state_candidates(&candidates_json);
                let source = FinalizedSourceIdentity {
                    draft_id,
                    finalization_id: source_finalization_id.clone().unwrap_or_default(),
                    chapter_number,
                    content_hash: source_content_hash.clone().unwrap_or_default(),
                };
                // 身份状态精细化（上游 :784-819）：来源当前时读冻结身份快照，
                // 按事实/候选的身份绑定程度降级 current → legacy → stale。
                let mut identity_status = "current";
                if has_character_identity_schema(conn) && source_current {
                    identity_status = match read_character_identity_snapshot(conn) {
                        Err(_) => "stale",
                        Ok(identities) => {
                            let characters = &identities.characters;
                            let bound = |character_id: &str, display_name: &str| -> bool {
                                characters.iter().any(|character| {
                                    character.character_id == character_id
                                        && character.name == display_name
                                }) && characters
                                    .iter()
                                    .filter(|character| character.name == display_name)
                                    .count()
                                    == 1
                            };
                            let degraded_legacy = facts.iter().any(|fact| {
                                fact.entities.iter().any(|entity| {
                                    characters.iter().any(|character| character.name == *entity)
                                        && !fact
                                            .character_refs
                                            .as_deref()
                                            .unwrap_or_default()
                                            .iter()
                                            .any(|reference| {
                                                reference.display_name_snapshot == *entity
                                            })
                                })
                            }) || candidates
                                .iter()
                                .any(|candidate| candidate.character_id.is_none());
                            if degraded_legacy {
                                "legacy"
                            } else {
                                let degraded_stale = facts.iter().any(|fact| {
                                    fact.character_refs
                                        .as_deref()
                                        .unwrap_or_default()
                                        .iter()
                                        .any(|reference| {
                                            !bound(
                                                &reference.character_id,
                                                &reference.display_name_snapshot,
                                            )
                                        })
                                        || fact.entities.iter().any(|entity| {
                                            if !characters
                                                .iter()
                                                .any(|character| character.name == *entity)
                                            {
                                                return false;
                                            }
                                            let refs: Vec<&FinalizedCharacterReference> = fact
                                                .character_refs
                                                .as_deref()
                                                .unwrap_or_default()
                                                .iter()
                                                .filter(|reference| {
                                                    reference.display_name_snapshot == *entity
                                                })
                                                .collect();
                                            refs.len() != 1 || !bound(&refs[0].character_id, entity)
                                        })
                                }) || candidates.iter().any(|candidate| {
                                    !bound(
                                        candidate.character_id.as_deref().unwrap_or(""),
                                        &candidate.character_name,
                                    )
                                });
                                if degraded_stale {
                                    "stale"
                                } else {
                                    "current"
                                }
                            }
                        }
                    };
                }
                FinalizedContinuityProjection {
                    draft_id,
                    current_finalized_draft_id: if !source_current && fallback_current {
                        current_draft_id
                    } else {
                        None
                    },
                    chapter_number,
                    chapter_title,
                    chapter_notes,
                    facts: Some(facts),
                    character_state_candidates: if candidates.is_empty() {
                        None
                    } else {
                        Some(candidates)
                    },
                    source: has_bound_source.then_some(source),
                    source_status: if !has_bound_source {
                        "legacy".to_string()
                    } else if source_current && !invalidated {
                        identity_status.to_string()
                    } else {
                        "stale".to_string()
                    },
                }
            },
        )
        .collect())
}

/// 基线 `SummaryRepository.readFinalizedSource`
pub fn read_finalized_source(
    conn: &Connection,
    draft_id: i64,
) -> Result<FinalizedSourceReadResult, String> {
    if draft_id < 1 {
        return Err("定稿来源身份无效".to_string());
    }
    let row = conn
        .query_row(
            "SELECT drafts.chapter_number, drafts.status, contents.body,
                    COALESCE(finalization_outbox.chapter_title, blueprints.title, ''),
                    finalization_outbox.draft_id,
                    finalization_outbox.finalization_id
             FROM drafts
             JOIN contents ON contents.id = drafts.content_id
             LEFT JOIN finalization_outbox ON finalization_outbox.draft_id = drafts.id
             LEFT JOIN blueprints ON blueprints.chapter_number = drafts.chapter_number
             WHERE drafts.id = ?1",
            [draft_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((chapter_number, status, content, chapter_title, receipt_draft_id, _)) = row else {
        return Ok(FinalizedSourceReadResult::Invalid);
    };
    if status != "finalized" {
        return Ok(FinalizedSourceReadResult::Invalid);
    }
    if receipt_draft_id.is_none() {
        return Ok(FinalizedSourceReadResult::Legacy {
            draft_id,
            chapter_number,
            chapter_title,
            content,
        });
    }
    let snapshot = read_finalized_source_from_db(conn, draft_id)?;
    Ok(match snapshot {
        Some(snapshot) => FinalizedSourceReadResult::Valid { snapshot },
        None => FinalizedSourceReadResult::Invalid,
    })
}

/// 基线 `invalidateContinuityProjectionFrom`：推进全局失效水位
pub fn invalidate_continuity_projection_from(
    conn: &Connection,
    chapter_number: i64,
) -> Result<(), String> {
    if chapter_number < 1 {
        return Err("连续性投影失效章节无效".to_string());
    }
    conn.execute(
        "UPDATE continuity_projection_meta
         SET generation = generation + 1,
             stale_from_chapter = CASE
               WHEN stale_from_chapter IS NULL OR stale_from_chapter > ?1 THEN ?1
               ELSE stale_from_chapter
             END
         WHERE id = 'main'",
        [chapter_number],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        conn
    }

    const BODY: &str = "第十章正文：林决在城门与黑袍人相遇。";

    /// 建一个已定稿草稿 + outbox 收据（正文快照/哈希一致），返回 draft_id
    fn finalize_draft(conn: &Connection, chapter: i64, title: &str) -> i64 {
        finalize_draft_version(conn, chapter, 1, title)
    }

    /// 同 `finalize_draft`，但指定草稿版本号（用于同章重定稿场景）。
    fn finalize_draft_version(conn: &Connection, chapter: i64, version: i64, title: &str) -> i64 {
        conn.execute("INSERT INTO contents (body) VALUES (?1)", [BODY])
            .unwrap();
        let content_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, status, content_id) VALUES (?1, ?2, 'finalized', ?3)",
            rusqlite::params![chapter, version, content_id],
        )
        .unwrap();
        let draft_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO finalization_outbox (
                finalization_id, draft_id, chapter_number, chapter_title,
                content_hash, content_revision, content_snapshot, target_file_name
            ) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7)",
            rusqlite::params![
                format!("fin-{chapter}-v{version}"),
                draft_id,
                chapter,
                title,
                sha256_hex(BODY),
                BODY,
                format!("ch{chapter}.md"),
            ],
        )
        .unwrap();
        draft_id
    }

    /// 建一个带身份 ID 的角色（m02 形态）。
    fn insert_character(conn: &Connection, character_id: &str, name: &str, retired: bool) {
        conn.execute(
            "INSERT INTO characters (character_id, name, role, retired) VALUES (?1, ?2, 'protagonist', ?3)",
            rusqlite::params![character_id, name, i64::from(retired)],
        )
        .unwrap();
    }

    fn fact(evidence: &str) -> serde_json::Value {
        serde_json::json!({
            "category": "plot",
            "entities": ["林决"],
            "statement": "林决与黑袍人相遇",
            "sourceChapter": 10,
            "evidence": evidence
        })
    }

    /// 绑定身份的 fact：entity 命中角色展示名时须携带 characterRef。
    fn bound_fact(evidence: &str, character_id: &str, display_name: &str) -> serde_json::Value {
        serde_json::json!({
            "category": "plot",
            "entities": [display_name],
            "statement": "林决与黑袍人相遇",
            "sourceChapter": 10,
            "evidence": evidence,
            "characterRefs": [{ "characterId": character_id, "displayNameSnapshot": display_name }]
        })
    }

    fn save_request(draft_id: i64, gen: i64, evidence: &str) -> SaveFinalizedContinuityRequest {
        SaveFinalizedContinuityRequest {
            draft_id,
            chapter_number: 10,
            chapter_notes: "本章连续性备注".to_string(),
            facts: Some(vec![fact(evidence)]),
            projection_generation: gen,
            source: FinalizedSourceIdentity {
                draft_id,
                finalization_id: format!("fin-10-v1"),
                chapter_number: 10,
                content_hash: sha256_hex(BODY),
            },
        }
    }

    fn bound_save_request(
        draft_id: i64,
        gen: i64,
        evidence: &str,
        character_id: &str,
        display_name: &str,
    ) -> SaveFinalizedContinuityRequest {
        SaveFinalizedContinuityRequest {
            draft_id,
            chapter_number: 10,
            chapter_notes: "本章连续性备注".to_string(),
            facts: Some(vec![bound_fact(evidence, character_id, display_name)]),
            projection_generation: gen,
            source: FinalizedSourceIdentity {
                draft_id,
                finalization_id: format!("fin-10-v1"),
                chapter_number: 10,
                content_hash: sha256_hex(BODY),
            },
        }
    }

    fn candidates_request(
        draft_id: i64,
        gen: i64,
        candidates: Vec<serde_json::Value>,
    ) -> SaveFinalizedCharacterStateCandidatesRequest {
        SaveFinalizedCharacterStateCandidatesRequest {
            draft_id,
            chapter_number: 10,
            candidates: Some(candidates),
            projection_generation: gen,
            source: FinalizedSourceIdentity {
                draft_id,
                finalization_id: format!("fin-10-v1"),
                chapter_number: 10,
                content_hash: sha256_hex(BODY),
            },
        }
    }

    fn projection_generation(conn: &Connection) -> i64 {
        conn.query_row(
            "SELECT generation FROM continuity_projection_meta WHERE id='main'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn save_and_list_roundtrip_test() {
        let conn = memory_db();
        let draft_id = finalize_draft(&conn, 10, "第十章");
        let generation = projection_generation(&conn);

        save_finalized_continuity(&conn, &save_request(draft_id, generation, "城门")).unwrap();

        // 列表： chapter < 11 才可见，状态 current
        let list = list_finalized_continuity_before(&conn, 11).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].source_status, "current");
        assert_eq!(list[0].facts.as_ref().unwrap()[0].evidence, "城门");
        assert!(list[0].source.is_some());

        // 引文不在正文中 → 拒绝
        let bad = save_request(draft_id, generation, "不存在的引文");
        assert_eq!(
            save_finalized_continuity(&conn, &bad).unwrap_err(),
            "连续性事实引文无法在绑定定稿正文中精确定位"
        );
    }

    #[test]
    fn save_rejects_stale_watermark_and_bad_source_test() {
        let conn = memory_db();
        let draft_id = finalize_draft(&conn, 10, "第十章");
        let generation = projection_generation(&conn);

        // 水位推进后旧代结果拒绝
        save_finalized_continuity(&conn, &save_request(draft_id, generation, "城门")).unwrap();
        invalidate_continuity_projection_from(&conn, 5).unwrap();
        let stale = save_request(draft_id, generation, "城门");
        assert_eq!(
            save_finalized_continuity(&conn, &stale).unwrap_err(),
            "连续性投影失效水位已推进，已拒绝过期结果"
        );

        // 来源收据不匹配拒绝
        let draft2 = finalize_draft(&conn, 11, "第十一章");
        let mut wrong_source = save_request(draft2, 1, "城门");
        wrong_source.source.finalization_id = "fin-99".to_string();
        assert_eq!(
            save_finalized_continuity(&conn, &wrong_source).unwrap_err(),
            "连续性投影来源已失效，已拒绝过期结果"
        );
    }

    #[test]
    fn read_source_discriminates_valid_legacy_invalid_test() {
        let conn = memory_db();
        // 未定稿草稿 → invalid
        conn.execute("INSERT INTO contents (body) VALUES ('草稿')", [])
            .unwrap();
        let cid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, status, content_id) VALUES (1, 1, 'draft', ?1)",
            [cid],
        )
        .unwrap();
        let draft_id = conn.last_insert_rowid();
        assert!(matches!(
            read_finalized_source(&conn, draft_id).unwrap(),
            FinalizedSourceReadResult::Invalid
        ));

        // 无收据的定稿 → legacy
        conn.execute(
            "UPDATE drafts SET status = 'finalized' WHERE id = ?1",
            [draft_id],
        )
        .unwrap();
        match read_finalized_source(&conn, draft_id).unwrap() {
            FinalizedSourceReadResult::Legacy { draft_id: id, .. } => assert_eq!(id, draft_id),
            other => panic!("应为 legacy，实际 {:?}", other),
        }

        // 有收据且快照一致 → valid
        let finalized = finalize_draft(&conn, 10, "第十章");
        match read_finalized_source(&conn, finalized).unwrap() {
            FinalizedSourceReadResult::Valid { snapshot } => {
                assert_eq!(snapshot.content, BODY);
                assert_eq!(snapshot.projection_generation, 0);
            }
            other => panic!("应为 valid，实际 {:?}", other),
        }
    }

    #[test]
    fn character_state_candidates_merge_and_validate_test() {
        let conn = memory_db();
        insert_character(&conn, "char-linjue", "林决", false);
        let draft_id = finalize_draft(&conn, 10, "第十章");
        let generation = projection_generation(&conn);

        // 先落连续性投影（候选必须绑定同代投影；entity 命中角色名须带 characterRef）
        save_finalized_continuity(
            &conn,
            &bound_save_request(draft_id, generation, "城门", "char-linjue", "林决"),
        )
        .unwrap();

        let candidates_req = candidates_request(
            draft_id,
            generation,
            vec![serde_json::json!({
                "characterId": "char-linjue",
                "characterName": "林决",
                "field": "location",
                "value": " 城门 ",
                "evidence": { "start": 9, "end": 11, "text": "城门" }
            })],
        );
        save_finalized_character_state_candidates(&conn, &candidates_req).unwrap();

        // 候选归一（读回时未绑定 source 的 value 会 trim）并合入投影行
        let list = list_finalized_continuity_before(&conn, 11).unwrap();
        let merged = list[0].character_state_candidates.as_ref().unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].value, "城门");
        assert_eq!(merged[0].character_id.as_deref(), Some("char-linjue"));

        // 不在身份快照中的候选拒绝（身份分支：无 characterId 即无法解析）
        let mut bad = candidates_req.clone();
        bad.candidates = Some(vec![serde_json::json!({
            "characterName": "路人甲",
            "field": "location",
            "value": "哪都"
        })]);
        assert_eq!(
            save_finalized_character_state_candidates(&conn, &bad).unwrap_err(),
            "FINALIZED_CHARACTER_IDENTITY_REQUIRED"
        );
    }

    #[test]
    fn invalidate_advances_generation_and_stale_window_test() {
        let conn = memory_db();
        invalidate_continuity_projection_from(&conn, 5).unwrap();
        let (generation, stale): (i64, i64) = conn
            .query_row(
                "SELECT generation, stale_from_chapter FROM continuity_projection_meta WHERE id='main'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(generation, 1);
        assert_eq!(stale, 5);

        // 更早章节再次失效 → stale_from_chapter 收窄，代际继续递增
        invalidate_continuity_projection_from(&conn, 3).unwrap();
        let (generation, stale): (i64, i64) = conn
            .query_row(
                "SELECT generation, stale_from_chapter FROM continuity_projection_meta WHERE id='main'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(generation, 2);
        assert_eq!(stale, 3);

        // 非法章节拒绝
        assert_eq!(
            invalidate_continuity_projection_from(&conn, 0).unwrap_err(),
            "连续性投影失效章节无效"
        );
    }

    #[test]
    fn continuity_facts_require_identity_binding_test() {
        let conn = memory_db();
        insert_character(&conn, "char-linjue", "林决", false);
        let draft_id = finalize_draft(&conn, 10, "第十章");
        let generation = projection_generation(&conn);

        // entity 命中角色展示名但无 characterRef → 身份绑定缺失
        let unbound = save_request(draft_id, generation, "城门");
        assert_eq!(
            save_finalized_continuity(&conn, &unbound).unwrap_err(),
            "FINALIZED_CHARACTER_IDENTITY_REQUIRED"
        );

        // characterRef 无法在身份快照解析 → 身份绑定缺失
        let ghost = SaveFinalizedContinuityRequest {
            facts: Some(vec![bound_fact("城门", "char-ghost", "林决")]),
            ..bound_save_request(draft_id, generation, "城门", "char-linjue", "林决")
        };
        assert_eq!(
            save_finalized_continuity(&conn, &ghost).unwrap_err(),
            "FINALIZED_CHARACTER_IDENTITY_REQUIRED"
        );

        // 展示名唯一解析且引用一致 → 接受，characterRefs 随事实持久化
        save_finalized_continuity(
            &conn,
            &bound_save_request(draft_id, generation, "城门", "char-linjue", "林决"),
        )
        .unwrap();
        let list = list_finalized_continuity_before(&conn, 11).unwrap();
        let refs = list[0].facts.as_ref().unwrap()[0]
            .character_refs
            .as_ref()
            .unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].character_id, "char-linjue");
        assert_eq!(refs[0].display_name_snapshot, "林决");

        // 入参校验：displayNameSnapshot 不在 entities 中 → 角色身份参数无效
        let invalid_ref = SaveFinalizedContinuityRequest {
            // displayNameSnapshot 不在 entities 中 → normalizedFacts 校验拒绝
            //（上游 summary-repository.ts :113-116）。
            facts: Some(vec![serde_json::json!({
                "category": "plot",
                "entities": ["城门"],
                "statement": "林决与黑袍人相遇",
                "sourceChapter": 10,
                "evidence": "城门",
                "characterRefs": [{ "characterId": "char-linjue", "displayNameSnapshot": "林冲" }]
            })]),
            ..bound_save_request(draft_id, generation, "城门", "char-linjue", "林决")
        };
        assert_eq!(
            save_finalized_continuity(&conn, &invalid_ref).unwrap_err(),
            "连续性角色身份参数无效"
        );

        // 展示名不唯一（两个同名角色）→ 身份绑定缺失
        insert_character(&conn, "char-linjue-2", "林决", false);
        let ambiguous = bound_save_request(draft_id, generation, "城门", "char-linjue", "林决");
        assert_eq!(
            save_finalized_continuity(&conn, &ambiguous).unwrap_err(),
            "FINALIZED_CHARACTER_IDENTITY_REQUIRED"
        );
    }

    #[test]
    fn character_state_candidates_identity_branch_test() {
        let conn = memory_db();
        insert_character(&conn, "char-linjue", "林决", false);
        insert_character(&conn, "char-retired", "退场角色", true);
        let draft_id = finalize_draft(&conn, 10, "第十章");
        let generation = projection_generation(&conn);
        save_finalized_continuity(
            &conn,
            &bound_save_request(draft_id, generation, "城门", "char-linjue", "林决"),
        )
        .unwrap();

        // 缺少 characterId → 身份必填
        let req = candidates_request(
            draft_id,
            generation,
            vec![serde_json::json!({
                "characterName": "林决", "field": "location", "value": "城门",
                "evidence": { "start": 9, "end": 11, "text": "城门" }
            })],
        );
        assert_eq!(
            save_finalized_character_state_candidates(&conn, &req).unwrap_err(),
            "FINALIZED_CHARACTER_IDENTITY_REQUIRED"
        );

        // displayName 与身份快照不符 → 身份必填
        let req = candidates_request(
            draft_id,
            generation,
            vec![serde_json::json!({
                "characterId": "char-linjue", "characterName": "林冲",
                "field": "location", "value": "城门",
                "evidence": { "start": 9, "end": 11, "text": "城门" }
            })],
        );
        assert_eq!(
            save_finalized_character_state_candidates(&conn, &req).unwrap_err(),
            "FINALIZED_CHARACTER_IDENTITY_REQUIRED"
        );

        // evidence 文本与正文跨度不匹配 → 证据无效
        let req = candidates_request(
            draft_id,
            generation,
            vec![serde_json::json!({
                "characterId": "char-linjue", "characterName": "林决",
                "field": "location", "value": "城门",
                "evidence": { "start": 9, "end": 11, "text": "不符" }
            })],
        );
        assert_eq!(
            save_finalized_character_state_candidates(&conn, &req).unwrap_err(),
            "FINALIZED_CHARACTER_EVIDENCE_INVALID"
        );

        // evidence 缺失 → 证据无效
        let req = candidates_request(
            draft_id,
            generation,
            vec![serde_json::json!({
                "characterId": "char-linjue", "characterName": "林决",
                "field": "location", "value": "城门"
            })],
        );
        assert_eq!(
            save_finalized_character_state_candidates(&conn, &req).unwrap_err(),
            "FINALIZED_CHARACTER_EVIDENCE_INVALID"
        );

        // 角色已退休 → 身份已退休
        let req = candidates_request(
            draft_id,
            generation,
            vec![serde_json::json!({
                "characterId": "char-retired", "characterName": "退场角色",
                "field": "location", "value": "退场",
                "evidence": { "start": 9, "end": 11, "text": "城门" }
            })],
        );
        assert_eq!(
            save_finalized_character_state_candidates(&conn, &req).unwrap_err(),
            "FINALIZED_CHARACTER_ID_RETIRED"
        );

        // 合法候选原样保留（身份分支不 trim value）
        let req = candidates_request(
            draft_id,
            generation,
            vec![serde_json::json!({
                "characterId": "char-linjue", "characterName": "林决",
                "field": "location", "value": " 城门 ",
                "evidence": { "start": 9, "end": 11, "text": "城门" }
            })],
        );
        save_finalized_character_state_candidates(&conn, &req).unwrap();
        let stored: String = conn
            .query_row(
                "SELECT character_state_candidates FROM summary_snapshots WHERE draft_id = ?1",
                [draft_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            stored.contains(r#""value":" 城门 ""#),
            "身份分支应原样保留 value"
        );
    }

    #[test]
    fn list_keeps_superseded_projection_as_stale_test() {
        let conn = memory_db();
        let draft_v1 = finalize_draft(&conn, 10, "第十章·初稿");
        let generation = projection_generation(&conn);
        save_finalized_continuity(&conn, &save_request(draft_v1, generation, "城门")).unwrap();

        // 同章新版本定稿（尚未重算投影）：旧投影仍返回，标 stale 并指向当前稿
        let draft_v2 = finalize_draft_version(&conn, 10, 2, "第十章·修订");
        let list = list_finalized_continuity_before(&conn, 11).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].draft_id, draft_v1);
        assert_eq!(list[0].source_status, "stale");
        assert_eq!(list[0].current_finalized_draft_id, Some(draft_v2));
        // 章节标题取自当前最新定稿稿
        assert_eq!(list[0].chapter_title, "第十章·修订");

        // 当前定稿稿被撤销定稿：旧投影仍返回，current 回退到上一版定稿稿
        conn.execute(
            "UPDATE drafts SET status = 'draft' WHERE id = ?1",
            [draft_v2],
        )
        .unwrap();
        let list = list_finalized_continuity_before(&conn, 11).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].draft_id, draft_v1);
        assert_eq!(list[0].source_status, "current");
        assert_eq!(list[0].current_finalized_draft_id, None);
        assert_eq!(list[0].chapter_title, "第十章·初稿");

        // 恢复定稿并落新版投影后，同章只回最新快照行（newer_summary 去重）
        conn.execute(
            "UPDATE drafts SET status = 'finalized' WHERE id = ?1",
            [draft_v2],
        )
        .unwrap();
        let mut new_projection = save_request(draft_v2, generation, "城门");
        // 新版定稿的 outbox 收据按版本号生成为 fin-10-v2，来源身份须与
        // 之对齐，否则 same_source 校验会拒绝（上游测试用新稿的真实收据）。
        new_projection.source.finalization_id = format!("fin-10-v2");
        save_finalized_continuity(&conn, &new_projection).unwrap();
        let list = list_finalized_continuity_before(&conn, 11).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].draft_id, draft_v2);
        assert_eq!(list[0].source_status, "current");
        assert_eq!(list[0].current_finalized_draft_id, None);
    }

    #[test]
    fn character_state_candidate_store_v2_roundtrip_test() {
        let conn = memory_db();
        insert_character(&conn, "char-linjue", "林决", false);
        let draft_id = finalize_draft(&conn, 10, "第十章");
        let generation = projection_generation(&conn);
        save_finalized_continuity(
            &conn,
            &bound_save_request(draft_id, generation, "城门", "char-linjue", "林决"),
        )
        .unwrap();
        let bound_source = serde_json::json!({
            "draftId": draft_id,
            "finalizationId": format!("fin-10-v1"),
            "chapterNumber": 10,
            "contentHash": sha256_hex(BODY)
        });

        // 带完整绑定元数据的候选 → v2 store 持久化，读回保留扩展字段且 value 不 trim
        let req = candidates_request(
            draft_id,
            generation,
            vec![serde_json::json!({
                "characterId": "char-linjue",
                "characterName": "林决",
                "field": "location",
                "value": " 城门 ",
                "candidateKey": "ck-1",
                "source": bound_source,
                "expectedFieldRevision": 3,
                "expectedFieldValueHash": "a".repeat(64),
                "evidence": { "start": 9, "end": 11, "text": "城门" },
                "selectionKey": "state:char-linjue:location",
                "displayName": "林决",
                "rawValue": { "original": true },
                "reason": "author-protected"
            })],
        );
        save_finalized_character_state_candidates(&conn, &req).unwrap();

        let list = list_finalized_continuity_before(&conn, 11).unwrap();
        let candidate = list[0].character_state_candidates.as_ref().unwrap()[0].clone();
        assert_eq!(candidate.character_id.as_deref(), Some("char-linjue"));
        assert_eq!(candidate.value, " 城门 ");
        assert_eq!(candidate.candidate_key.as_deref(), Some("ck-1"));
        assert_eq!(candidate.expected_field_revision, Some(3));
        assert_eq!(
            candidate.expected_field_value_hash.as_deref(),
            Some("a".repeat(64).as_str())
        );
        assert_eq!(candidate.evidence.as_ref().unwrap().text, "城门");
        assert_eq!(
            candidate.selection_key.as_deref(),
            Some("state:char-linjue:location")
        );
        assert_eq!(candidate.display_name.as_deref(), Some("林决"));
        assert_eq!(
            candidate.raw_value,
            Some(serde_json::json!({ "original": true }))
        );
        assert_eq!(candidate.reason.as_deref(), Some("author-protected"));
        assert!(candidate.source.is_some());

        // 收据覆盖：候选键已被 decision receipt 覆盖 → 跳过合并
        let receipt_store = serde_json::json!({
            "version": 2,
            "pending": [],
            "receipts": [{
                "candidateKey": "ck-skip",
                "operationId": "op-1",
                "payloadHash": "b".repeat(64),
                "decision": "accept",
                "source": bound_source
            }]
        })
        .to_string();
        conn.execute(
            "UPDATE summary_snapshots SET character_state_candidates = ?1 WHERE draft_id = ?2",
            rusqlite::params![receipt_store, draft_id],
        )
        .unwrap();
        let req = candidates_request(
            draft_id,
            generation,
            vec![serde_json::json!({
                "characterId": "char-linjue", "characterName": "林决",
                "field": "location", "value": "新值",
                "candidateKey": "ck-skip",
                "source": bound_source,
                "expectedFieldRevision": 1,
                "expectedFieldValueHash": "c".repeat(64),
                "evidence": { "start": 9, "end": 11, "text": "城门" }
            })],
        );
        save_finalized_character_state_candidates(&conn, &req).unwrap();
        let list = list_finalized_continuity_before(&conn, 11).unwrap();
        assert!(list[0].character_state_candidates.is_none());

        // 旧数组格式（无 characterId）→ 解析为 legacy 候选（value 未绑定 source 会
        // trim），身份状态降级 legacy
        conn.execute(
            "UPDATE summary_snapshots SET character_state_candidates = ?1 WHERE draft_id = ?2",
            rusqlite::params![
                r#"[{"characterName":"林决","field":"location","value":" 旧值 "}]"#,
                draft_id
            ],
        )
        .unwrap();
        let list = list_finalized_continuity_before(&conn, 11).unwrap();
        let candidates = list[0].character_state_candidates.as_ref().unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].value, "旧值");
        assert_eq!(list[0].source_status, "legacy");

        // 候选身份无法在快照解析 → 身份状态降级 stale
        conn.execute(
            "UPDATE summary_snapshots SET character_state_candidates = ?1 WHERE draft_id = ?2",
            rusqlite::params![
                r#"[{"characterId":"char-ghost","characterName":"林决","field":"location","value":"幽灵"}]"#,
                draft_id
            ],
        )
        .unwrap();
        let list = list_finalized_continuity_before(&conn, 11).unwrap();
        assert_eq!(list[0].source_status, "stale");
    }

    #[test]
    fn continuity_facts_length_limits_align_with_upstream_test() {
        let conn = memory_db();
        let draft_id = finalize_draft(&conn, 10, "第十章");
        let generation = projection_generation(&conn);

        // 上游无 statement/evidence 长度上限：长陈述可落盘（引文仍须精确定位）
        let long_statement = "长".repeat(300);
        let req = SaveFinalizedContinuityRequest {
            facts: Some(vec![serde_json::json!({
                "category": "plot",
                "entities": [],
                "statement": long_statement,
                "sourceChapter": 10,
                "evidence": "城门"
            })]),
            ..save_request(draft_id, generation, "城门")
        };
        save_finalized_continuity(&conn, &req).unwrap();
        let list = list_finalized_continuity_before(&conn, 11).unwrap();
        assert_eq!(list[0].facts.as_ref().unwrap()[0].statement, long_statement);

        // entity 长度按 UTF-16 码元计：40 个非 BMP 字符 = 80 码元，可接受
        let bmp_boundary = "𠮷".repeat(40);
        let req = SaveFinalizedContinuityRequest {
            facts: Some(vec![serde_json::json!({
                "category": "plot",
                "entities": [bmp_boundary],
                "statement": "边界",
                "sourceChapter": 10,
                "evidence": "城门"
            })]),
            ..save_request(draft_id, generation, "城门")
        };
        save_finalized_continuity(&conn, &req).unwrap();

        // 41 个非 BMP 字符 = 82 码元 > 80 → 拒绝
        let over_boundary = "𠮷".repeat(41);
        let req = SaveFinalizedContinuityRequest {
            facts: Some(vec![serde_json::json!({
                "category": "plot",
                "entities": [over_boundary],
                "statement": "超界",
                "sourceChapter": 10,
                "evidence": "城门"
            })]),
            ..save_request(draft_id, generation, "城门")
        };
        assert_eq!(
            save_finalized_continuity(&conn, &req).unwrap_err(),
            "连续性事实参数无效"
        );
    }
}
