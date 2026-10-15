//! finalization_generation_source —— 定稿生成源（G4-2-D，1:1 平移
//! `electron/services/finalization-generation-source.ts`，59 行）。
//!
//! 职责：从定稿数据库读取不可变定稿上下文（`FinalizedCharacterContext`），
//! 捕获生成用快照（`capture`）、从持久化 run 的 `sourceManifest` 回读并
//! 校验（`read`）、组装 `GenerationTask`（system/user 消息 + 不可变输出合同）。
//!
//! # 移植说明（偏差与收口待办）
//! - `readFinalizedCharacterContext`（基线 `SummaryRepository` 静态方法，
//!   `electron/repositories/summary-repository.ts:400-420`）在本模块**私有实现**
//!   （`read_finalized_character_context`）：`character_proposal.rs:1958` 现为
//!   PENDING 桩且该文件禁改；**编排者收口时评估提升为 `pub` 复用**。
//!   基线用 `db.transaction()` 包裹只读查询（一致性快照）；Rust 侧为纯读、
//!   不显式开事务（`rusqlite` 无自动 SAVEPOINT 嵌套语义），单语句读已一致。
//! - `finalizedCharacterPromptCards`（`src/shared/finalized-continuity.ts:126-141`）
//!   与 `decideDerivedPatch`（`src/shared/character-identity.ts:67-94`）及其私有
//!   辅助（`same_source` / `same_provenance` / `advances_project_continuity`）
//!   一并移植进本模块（Rust 侧原缺失）。
//! - 基线的键集校验（`Object.keys(slot)` 白名单）在 Rust 由类型结构保证
//!   （未知键在反序列化时被忽略）；`typeof` 守卫由类型系统保证。
//! - `text_hash` 私有实现（UTF-8 SHA-256 hex，同 C 轮处理）；跨栈哈希不要求
//!   逐字节一致（AGENTS.md 定案）。
//! - `render_prompt` / `generation_task` 为同波产物，签名按其落地契约引用。
//!
//! # 错误码（逐字照搬基线）
//! `GENERATION_FINALIZATION_SLOT_INVALID` / `GENERATION_FINALIZATION_SOURCE_CHANGED` /
//! `GENERATION_FINALIZATION_IDENTITY_CHANGED` / `GENERATION_FINALIZATION_CONTEXT_INVALID` /
//! `FINALIZED_CHARACTER_SCOPE_REQUIRED` / `FINALIZED_CHARACTER_SOURCE_CHANGED` /
//! `FINALIZED_CHARACTER_ID_RETIRED` / `FINALIZED_CHARACTER_FIELD_REVISION_INVALID` /
//! `FINALIZED_CHARACTER_SNAPSHOT_INVALID`。

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::repositories::character_repository::{
    has_character_identity_schema, CharacterStateFieldProvenance, CharacterStateTextField,
    FinalizedSourceIdentity,
};
use crate::repositories::generation_run_repository::{DurableGenerationRun, ProjectEpoch};
use crate::services::generation_task::{
    GenerationMessage, GenerationMessageRole, GenerationOutput, GenerationReasoningStage,
    GenerationTask,
};
use crate::services::prompt_system_role::{
    compose_prompt_system_role, PromptTemplate, WritingLanguage,
};
use crate::services::render_prompt::render_prompt;
use crate::services::source_ref::{is_content_hash, is_finalized_source_identity};

// ============================================================================
// 错误码（与基线 fail(...) 调用逐字对应）
// ============================================================================

const GENERATION_FINALIZATION_SLOT_INVALID: &str = "GENERATION_FINALIZATION_SLOT_INVALID";
const GENERATION_FINALIZATION_SOURCE_CHANGED: &str = "GENERATION_FINALIZATION_SOURCE_CHANGED";
const GENERATION_FINALIZATION_IDENTITY_CHANGED: &str = "GENERATION_FINALIZATION_IDENTITY_CHANGED";
const GENERATION_FINALIZATION_CONTEXT_INVALID: &str = "GENERATION_FINALIZATION_CONTEXT_INVALID";
const FINALIZED_CHARACTER_SCOPE_REQUIRED: &str = "FINALIZED_CHARACTER_SCOPE_REQUIRED";
const FINALIZED_CHARACTER_SOURCE_CHANGED: &str = "FINALIZED_CHARACTER_SOURCE_CHANGED";
const FINALIZED_CHARACTER_ID_RETIRED: &str = "FINALIZED_CHARACTER_ID_RETIRED";
const FINALIZED_CHARACTER_FIELD_REVISION_INVALID: &str =
    "FINALIZED_CHARACTER_FIELD_REVISION_INVALID";
const FINALIZED_CHARACTER_SNAPSHOT_INVALID: &str = "FINALIZED_CHARACTER_SNAPSHOT_INVALID";

/// 冻结角色快照的 proposal 种类（`FINALIZED_CHARACTER_SNAPSHOT_KIND`）。
const FINALIZED_CHARACTER_SNAPSHOT_KIND: &str = "finalized-character-snapshot:v1";

/// JS `Number.isSafeInteger` 的 i64 等价（|v| ≤ 2^53-1）。
fn is_safe_integer(value: i64) -> bool {
    value >= -9007199254740991 && value <= 9007199254740991
}

/// UTF-8 字节的 SHA-256 小写 hex（基线 `textHash`，私有化同 C 轮处理）。
fn text_hash(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

// ============================================================================
// 共享契约类型（`src/shared/finalization-generation.ts` + `finalized-continuity.ts`）
// ============================================================================

/// 定稿生成步骤（TS 联合 `'chapter_notes' | 'character_cards'`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalizationGenerationStep {
    ChapterNotes,
    CharacterCards,
}

/// 定稿生成槽位（`FinalizationGenerationSlot`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizationGenerationSlot {
    pub source: FinalizedSourceIdentity,
    pub step_key: FinalizationGenerationStep,
}

/// 身份绑定状态（TS 联合 `'bound' | 'legacy'`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FinalizedIdentityStatus {
    Bound,
    Legacy,
}

/// 派生字段的顺序锚点（`DerivedSourceOrder`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivedSourceOrder {
    pub continuity_epoch: String,
    pub chapter_number: i64,
    pub authoritative_finalization_revision: i64,
}

/// 角色字段快照（`CharacterFieldSnapshot`；`sourceOrder` 仅在存在时序列化）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterFieldSnapshot {
    pub project_id: String,
    pub epoch: String,
    pub character_id: String,
    pub field: CharacterStateTextField,
    pub revision: i64,
    pub value: String,
    pub value_hash: String,
    pub provenance: CharacterStateFieldProvenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_order: Option<DerivedSourceOrder>,
}

/// 定稿角色引用（`FinalizedCharacterReference & { aliases, fields }`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedCharacterReference {
    pub character_id: String,
    pub display_name_snapshot: String,
    pub aliases: Vec<String>,
    pub fields: Vec<CharacterFieldSnapshot>,
}

/// 定稿角色上下文（`FinalizedCharacterContext`，extends `ProjectEpoch`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedCharacterContext {
    pub project_id: String,
    pub epoch: String,
    pub source: FinalizedSourceIdentity,
    pub content: String,
    pub projection_generation: i64,
    pub identity_revision: i64,
    pub identity_status: FinalizedIdentityStatus,
    pub source_order: DerivedSourceOrder,
    pub characters: Vec<FinalizedCharacterReference>,
}

/// 定稿笔记基线的蓝图部分（`notesBaseline.blueprint`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizationNotesBlueprint {
    pub exists: bool,
    pub notes: String,
}

/// 定稿笔记基线（`notesBaseline`，仅作提交时对比）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizationNotesBaseline {
    pub blueprint: FinalizationNotesBlueprint,
    pub continuity_hash: String,
}

/// 定稿生成上下文（`FinalizationGenerationContext`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizationGenerationContext {
    pub slot: FinalizationGenerationSlot,
    pub identity: FinalizedCharacterContext,
    pub chapter_title: String,
    pub chapter_entities: Vec<String>,
    pub writing_language: WritingLanguage,
    pub template: PromptTemplate,
    pub notes_baseline: FinalizationNotesBaseline,
}

// ============================================================================
// 内部类型（基线私有接口 / 快照行）
// ============================================================================

/// 冻结角色快照（`character_identity_proposals.raw_value` 的存储形态）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrozenCharacterSnapshot {
    pub(crate) kind: String,
    pub(crate) source: FinalizedSourceIdentity,
    pub(crate) identity_revision: i64,
    pub(crate) characters: Vec<FrozenCharacterReference>,
}

/// 冻结角色引用（`FinalizedCharacterReference`，不含 fields）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrozenCharacterReference {
    pub(crate) character_id: String,
    pub(crate) display_name_snapshot: String,
    pub(crate) aliases: Vec<String>,
}

/// `readFinalizedSourceFromDb` 的行投影。
#[derive(Debug, Clone)]
struct FinalizedSourceSnapshot {
    source: FinalizedSourceIdentity,
    #[allow(dead_code)]
    chapter_title: String,
    /// `contents.body`（仅作一致性校验；上下文取 `content_snapshot`）。
    #[allow(dead_code)]
    body: String,
    content: String,
    projection_generation: i64,
    status: String,
}

/// `characters` 表的状态列投影（`stateSnapshot` 的 `row`）。
#[derive(Debug)]
pub(crate) struct CharacterStateRow {
    pub(crate) character_id: String,
    pub(crate) cs_location: Option<String>,
    pub(crate) cs_power_level: Option<String>,
    pub(crate) cs_physical_state: Option<String>,
    pub(crate) cs_mental_state: Option<String>,
    pub(crate) cs_key_items: Option<String>,
    pub(crate) cs_recent_events: Option<String>,
    pub(crate) cs_provenance: Option<String>,
}

impl CharacterStateRow {
    /// `STATE_COLUMNS[field]`（`row[STATE_COLUMNS[field]] ?? ''`）。
    fn state_column(&self, field: CharacterStateTextField) -> Option<&str> {
        match field {
            CharacterStateTextField::Location => self.cs_location.as_deref(),
            CharacterStateTextField::PowerLevel => self.cs_power_level.as_deref(),
            CharacterStateTextField::PhysicalState => self.cs_physical_state.as_deref(),
            CharacterStateTextField::MentalState => self.cs_mental_state.as_deref(),
            CharacterStateTextField::KeyItems => self.cs_key_items.as_deref(),
            CharacterStateTextField::RecentEvents => self.cs_recent_events.as_deref(),
        }
    }
}

/// `CHARACTER_STATE_TEXT_FIELDS` 的字段名键（`cs_provenance` 映射的键）。
fn field_key(field: CharacterStateTextField) -> &'static str {
    match field {
        CharacterStateTextField::Location => "location",
        CharacterStateTextField::PowerLevel => "powerLevel",
        CharacterStateTextField::PhysicalState => "physicalState",
        CharacterStateTextField::MentalState => "mentalState",
        CharacterStateTextField::KeyItems => "keyItems",
        CharacterStateTextField::RecentEvents => "recentEvents",
    }
}

// ============================================================================
// 公开函数（1:1 平移）
// ============================================================================

/// `finalizationSlotKey`：槽位结构校验 + 不可变键哈希。
pub fn finalization_slot_key(slot: &FinalizationGenerationSlot) -> Result<String, String> {
    // 基线的键集白名单（`Object.keys(...)`）由类型结构保证；此处校验取值域。
    let valid = is_safe_integer(slot.source.draft_id)
        && slot.source.draft_id >= 1
        && is_safe_integer(slot.source.chapter_number)
        && slot.source.chapter_number >= 1
        && !slot.source.finalization_id.trim().is_empty()
        && is_content_hash(&slot.source.content_hash);
    if !valid {
        return Err(GENERATION_FINALIZATION_SLOT_INVALID.to_string());
    }
    Ok(text_hash(
        &json!([
            slot.source.draft_id,
            slot.source.finalization_id,
            slot.source.chapter_number,
            slot.source.content_hash,
            slot.step_key,
        ])
        .to_string(),
    ))
}

/// `finalizationNotesBaseline`：蓝图笔记存在性 + 连续性快照行哈希。
pub fn finalization_notes_baseline(
    conn: &Connection,
    slot: &FinalizationGenerationSlot,
) -> FinalizationNotesBaseline {
    // `blueprint?.notes ?? ''`：行存在但 notes 为 NULL 时回落空串。
    let (blueprint_exists, blueprint_notes) = match conn
        .query_row(
            "SELECT notes FROM blueprints WHERE chapter_number=?",
            [slot.source.chapter_number],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
    {
        Ok(Some(notes)) => (true, notes.unwrap_or_default()),
        _ => (false, String::new()),
    };
    let mut stmt = conn
        .prepare(
            "SELECT chapter_number,chapter_notes,continuity_facts,source_finalization_id,source_content_hash,projection_generation FROM summary_snapshots WHERE draft_id=? ORDER BY id",
        )
        .expect("summary_snapshots 列名稳定");
    let rows: Vec<Value> = stmt
        .query_map([slot.source.draft_id], |row| {
            Ok(json!({
                "chapter_number": row.get::<_, i64>(0)?,
                "chapter_notes": row.get::<_, Option<String>>(1)?,
                "continuity_facts": row.get::<_, Option<String>>(2)?,
                "source_finalization_id": row.get::<_, Option<String>>(3)?,
                "source_content_hash": row.get::<_, Option<String>>(4)?,
                "projection_generation": row.get::<_, i64>(5)?,
            }))
        })
        .expect("summary_snapshots 查询列类型稳定")
        .collect::<Result<_, _>>()
        .expect("summary_snapshots 行可序列化");
    FinalizationNotesBaseline {
        blueprint: FinalizationNotesBlueprint {
            exists: blueprint_exists,
            notes: blueprint_notes,
        },
        continuity_hash: text_hash(&Value::Array(rows).to_string()),
    }
}

/// `captureFinalizationGenerationContext`：捕获不可变生成上下文。
///
/// `frozen` 给出时按其身份/槽位/基线回填，并校验身份与槽位均未变化。
pub fn capture_finalization_generation_context(
    conn: &Connection,
    slot: &FinalizationGenerationSlot,
    scope: &ProjectEpoch,
    writing_language: WritingLanguage,
    template: &PromptTemplate,
    frozen: Option<&FinalizationGenerationContext>,
) -> Result<FinalizationGenerationContext, String> {
    finalization_slot_key(slot)?;
    // `frozen?.identity ?? scope`：冻结时以冻结身份的项目/纪元为作用域。
    let effective_scope = match frozen {
        Some(frozen) => ProjectEpoch {
            project_id: frozen.identity.project_id.clone(),
            epoch: frozen.identity.epoch.clone(),
        },
        None => ProjectEpoch {
            project_id: scope.project_id.clone(),
            epoch: scope.epoch.clone(),
        },
    };
    let identity = read_finalized_character_context(conn, slot.source.draft_id, &effective_scope)?;
    if identity.source != slot.source {
        return Err(GENERATION_FINALIZATION_SOURCE_CHANGED.to_string());
    }
    if let Some(frozen) = frozen {
        // `immutableIdentity` 剥离 fields 后深比较（fields 是派生投影，可重算）。
        if frozen.slot != *slot
            || immutable_identity(&identity) != immutable_identity(&frozen.identity)
        {
            return Err(GENERATION_FINALIZATION_IDENTITY_CHANGED.to_string());
        }
    }
    // 基线在缺 finalization_outbox 行时抛 TypeError；readFinalizedSourceFromDb
    // 已 JOIN finalization_outbox，故此处行必存在，缺失按空串兜底（文档化偏差）。
    let chapter_title = conn
        .query_row(
            "SELECT chapter_title FROM finalization_outbox WHERE draft_id=?",
            [slot.source.draft_id],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_default();
    // `JSON.parse(blueprint?.characters ?? '[]')`：蓝图角色名列表（string[]）。
    let chapter_entities: Vec<String> = match conn
        .query_row(
            "SELECT characters FROM blueprints WHERE chapter_number=?",
            [slot.source.chapter_number],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
    {
        Ok(Some(Some(characters))) => serde_json::from_str(characters.as_str()).unwrap_or_default(),
        _ => Vec::new(),
    };
    Ok(FinalizationGenerationContext {
        slot: slot.clone(),
        identity: frozen
            .map(|frozen| frozen.identity.clone())
            .unwrap_or(identity),
        chapter_title,
        chapter_entities,
        writing_language,
        template: template.clone(),
        notes_baseline: frozen
            .map(|frozen| frozen.notes_baseline.clone())
            .unwrap_or_else(|| finalization_notes_baseline(conn, slot)),
    })
}

/// `readFinalizationGenerationContext`：从持久化 run 的 `sourceManifest`
/// 回读并逐项校验定稿生成上下文（哈希 / 槽位键 / 项目纪元 / 操作名）。
pub fn read_finalization_generation_context(
    run: &DurableGenerationRun,
) -> Result<FinalizationGenerationContext, String> {
    let manifest = &run.binding.source_manifest;
    let context: FinalizationGenerationContext = serde_json::from_value(
        manifest
            .get("finalizationGenerationContext")
            .cloned()
            .unwrap_or(Value::Null),
    )
    .map_err(|_| GENERATION_FINALIZATION_CONTEXT_INVALID.to_string())?;
    let context_json = serde_json::to_value(&context)
        .map_err(|_| GENERATION_FINALIZATION_CONTEXT_INVALID.to_string())?;
    let slot_key = finalization_slot_key(&context.slot)
        .map_err(|_| GENERATION_FINALIZATION_CONTEXT_INVALID.to_string())?;
    let expected_operation = match context.slot.step_key {
        FinalizationGenerationStep::ChapterNotes => "finalized-chapter-notes",
        FinalizationGenerationStep::CharacterCards => "finalized-character-state",
    };
    let valid = Some(text_hash(&context_json.to_string()).as_str())
        == manifest
            .get("finalizationGenerationContextHash")
            .and_then(Value::as_str)
        && Some(slot_key.as_str())
            == manifest
                .get("finalizationGenerationSlotKey")
                .and_then(Value::as_str)
        && context.identity.project_id == run.binding.project_id
        && Some(context.identity.epoch.as_str())
            == manifest
                .get("finalizationGenerationOriginEpoch")
                .and_then(Value::as_str)
        && context.slot.source == context.identity.source
        && text_hash(&context.identity.content) == context.slot.source.content_hash
        && manifest.get("operation").and_then(Value::as_str) == Some(expected_operation);
    if !valid {
        return Err(GENERATION_FINALIZATION_CONTEXT_INVALID.to_string());
    }
    Ok(context)
}

/// `finalizationGenerationTask`：组装定稿生成任务（system 合同 + user prompt）。
pub fn finalization_generation_task(
    context: &FinalizationGenerationContext,
    origin_project_id: Option<&str>,
) -> GenerationTask {
    let characters = matches!(
        context.slot.step_key,
        FinalizationGenerationStep::CharacterCards
    );
    let contract = match context.writing_language {
        WritingLanguage::EnUs => EN_US_FINAL_OUTPUT_CONTRACT,
        WritingLanguage::ZhCn => ZH_CN_FINAL_OUTPUT_CONTRACT,
    };
    let cards = if characters {
        finalized_character_prompt_cards(&context.identity, origin_project_id)
    } else {
        context
            .identity
            .characters
            .iter()
            .map(|character| FinalizedCharacterPromptCard {
                character_id: character.character_id.clone(),
                name: character.display_name_snapshot.clone(),
                aliases: character.aliases.clone(),
                fields: character
                    .fields
                    .iter()
                    .map(|field| FinalizedCharacterPromptCardField {
                        field: field.field,
                        value: field.value.clone(),
                        provenance: field.provenance.clone(),
                    })
                    .collect(),
            })
            .collect()
    };
    let mut variables = HashMap::new();
    variables.insert(
        "chapter_content".to_string(),
        context.identity.content.clone(),
    );
    variables.insert(
        "chapter_number".to_string(),
        context.slot.source.chapter_number.to_string(),
    );
    variables.insert("chapter_title".to_string(), context.chapter_title.clone());
    variables.insert(
        "existing_cards_json".to_string(),
        serde_json::to_string_pretty(&cards).unwrap_or_default(),
    );
    let prompt = render_prompt(&context.template, &variables, context.writing_language);
    let user_content = if characters {
        format!("{prompt}\n\n{contract}")
    } else {
        prompt
    };
    GenerationTask {
        purpose: if characters {
            "finalized-character-state"
        } else {
            "finalized-chapter-notes"
        }
        .to_string(),
        budget_demand: None,
        reasoning_stage: Some(GenerationReasoningStage::Review),
        output: if characters {
            GenerationOutput::StructuredData
        } else {
            GenerationOutput::VisibleText
        },
        messages: vec![
            GenerationMessage {
                role: GenerationMessageRole::System,
                content: compose_prompt_system_role(&context.template, context.writing_language),
            },
            GenerationMessage {
                role: GenerationMessageRole::User,
                content: user_content,
            },
        ],
        prompt_budget: None,
    }
}

// ============================================================================
// 私有辅助
// ============================================================================

/// `immutableIdentity`：剥除每个角色的 `fields`（派生投影，不进不可变身份）。
fn immutable_identity(context: &FinalizedCharacterContext) -> Value {
    let mut value = serde_json::to_value(context).expect("FinalizedCharacterContext 可序列化");
    if let Some(characters) = value.get_mut("characters").and_then(Value::as_array_mut) {
        for character in characters {
            if let Value::Object(character) = character {
                character.remove("fields");
            }
        }
    }
    value
}

/// `SummaryRepository.readFinalizedCharacterContext`（基线
/// `electron/repositories/summary-repository.ts:400-420`）的 1:1 移植。
///
/// **复用边界**：B98 起本函数提升为 `pub`，供
/// `finalized_character_service` 的 `finalized-character:read-context` 频道复用；
/// `character_proposal.rs:1958` 的同名 PENDING 桩解除后可一并切换。
pub fn read_finalized_character_context(
    conn: &Connection,
    draft_id: i64,
    scope: &ProjectEpoch,
) -> Result<FinalizedCharacterContext, String> {
    if scope.project_id.trim().is_empty() || scope.epoch.trim().is_empty() {
        return Err(FINALIZED_CHARACTER_SCOPE_REQUIRED.to_string());
    }
    let source = read_finalized_source_from_db(conn, draft_id)
        .ok_or(FINALIZED_CHARACTER_SOURCE_CHANGED.to_string())?;
    let latest = conn
        .query_row(
            "SELECT id,version FROM drafts WHERE chapter_number=? AND status='finalized' ORDER BY version DESC,id DESC LIMIT 1",
            [source.source.chapter_number],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|err| err.to_string())?;
    let Some(latest) = latest else {
        return Err(FINALIZED_CHARACTER_SOURCE_CHANGED.to_string());
    };
    if latest.0 != draft_id {
        return Err(FINALIZED_CHARACTER_SOURCE_CHANGED.to_string());
    }
    let frozen = read_frozen_character_snapshot(conn, &source.source)?;
    let characters = frozen
        .as_ref()
        .map(|snapshot| snapshot.characters.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|character| -> Result<FinalizedCharacterReference, String> {
            let row = conn
                .query_row(
                    "SELECT character_id, cs_location, cs_power_level, cs_physical_state, cs_mental_state, cs_key_items, cs_recent_events, cs_provenance FROM characters WHERE character_id=? AND retired=0",
                    [character.character_id.as_str()],
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
                .map_err(|err| err.to_string())?;
            let Some(row) = row else {
                return Err(FINALIZED_CHARACTER_ID_RETIRED.to_string());
            };
            let fields = [
                CharacterStateTextField::Location,
                CharacterStateTextField::PowerLevel,
                CharacterStateTextField::PhysicalState,
                CharacterStateTextField::MentalState,
                CharacterStateTextField::KeyItems,
                CharacterStateTextField::RecentEvents,
            ]
            .iter()
            .map(|field| state_snapshot(&row, *field, scope))
            .collect::<Result<Vec<_>, _>>()?;
            Ok(FinalizedCharacterReference {
                character_id: character.character_id.clone(),
                display_name_snapshot: character.display_name_snapshot.clone(),
                aliases: character.aliases.clone(),
                fields,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(FinalizedCharacterContext {
        project_id: scope.project_id.clone(),
        epoch: scope.epoch.clone(),
        source: source.source.clone(),
        content: source.content,
        projection_generation: source.projection_generation,
        identity_revision: frozen
            .as_ref()
            .map(|snapshot| snapshot.identity_revision)
            .unwrap_or(0),
        identity_status: if frozen.is_some() {
            FinalizedIdentityStatus::Bound
        } else {
            FinalizedIdentityStatus::Legacy
        },
        source_order: DerivedSourceOrder {
            continuity_epoch: format!("{}:{}", scope.project_id, source.projection_generation),
            chapter_number: source.source.chapter_number,
            authoritative_finalization_revision: latest.1,
        },
        characters,
    })
}

/// `readFinalizedSourceFromDb`（基线 `summary-repository.ts:275-321`）。
fn read_finalized_source_from_db(
    conn: &Connection,
    draft_id: i64,
) -> Option<FinalizedSourceSnapshot> {
    conn.query_row(
        "SELECT drafts.id AS draftId, drafts.chapter_number AS chapterNumber, drafts.status,
                contents.body AS content, finalization_outbox.finalization_id AS finalizationId,
                finalization_outbox.chapter_title AS chapterTitle,
                finalization_outbox.content_hash AS contentHash,
                finalization_outbox.content_snapshot AS contentSnapshot,
                continuity_projection_meta.generation AS projectionGeneration
         FROM drafts
         JOIN contents ON contents.id = drafts.content_id
         JOIN finalization_outbox ON finalization_outbox.draft_id = drafts.id
         JOIN continuity_projection_meta ON continuity_projection_meta.id = 'main'
         WHERE drafts.id = ?",
        [draft_id],
        |row| {
            Ok(FinalizedSourceSnapshot {
                source: FinalizedSourceIdentity {
                    draft_id: row.get("draftId")?,
                    finalization_id: row.get("finalizationId")?,
                    chapter_number: row.get("chapterNumber")?,
                    content_hash: row.get("contentHash")?,
                },
                chapter_title: row.get("chapterTitle")?,
                body: row.get("content")?,
                content: row.get("contentSnapshot")?,
                projection_generation: row.get("projectionGeneration")?,
                status: row.get("status")?,
            })
        },
    )
    .optional()
    .ok()
    .flatten()
    .filter(|snapshot| {
        // 任一不变量破坏即视为定稿源变化（基线返回 null → 调用方抛错）。
        snapshot.status == "finalized"
            && !snapshot.source.finalization_id.trim().is_empty()
            && is_content_hash(&snapshot.source.content_hash)
            && snapshot.body == snapshot.content
            && text_hash(&snapshot.content) == snapshot.source.content_hash
    })
    .map(|snapshot| FinalizedSourceSnapshot {
        source: snapshot.source,
        chapter_title: snapshot.chapter_title,
        body: snapshot.body,
        content: snapshot.content,
        projection_generation: snapshot.projection_generation,
        status: snapshot.status,
    })
}

/// `readFrozenCharacterSnapshot`（基线 `summary-repository.ts:62-76`）。
pub(crate) fn read_frozen_character_snapshot(
    conn: &Connection,
    source: &FinalizedSourceIdentity,
) -> Result<Option<FrozenCharacterSnapshot>, String> {
    if !has_character_identity_schema(conn) {
        return Ok(None);
    }
    let row = match conn
        .query_row(
            "SELECT source_key,source_hash,raw_value FROM character_identity_proposals WHERE proposal_id=?",
            [format!("fcs:{}", source.finalization_id)],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        )
        .optional()
    {
        Ok(row) => row,
        Err(err) => return Err(err.to_string()),
    };
    let Some((source_key, source_hash, raw_value)) = row else {
        return Ok(None);
    };
    let snapshot: FrozenCharacterSnapshot = serde_json::from_str(&raw_value)
        .map_err(|_| FINALIZED_CHARACTER_SNAPSHOT_INVALID.to_string())?;
    let mut character_ids: Vec<&str> = snapshot
        .characters
        .iter()
        .map(|item| item.character_id.as_str())
        .collect();
    let total = character_ids.len();
    character_ids.sort_unstable();
    character_ids.dedup();
    // 基线 `typeof` 守卫（displayNameSnapshot 为串 / aliases 为串数组）由类型保证。
    let valid = source_key == FINALIZED_CHARACTER_SNAPSHOT_KIND
        && text_hash(&raw_value) == source_hash
        && snapshot.kind == FINALIZED_CHARACTER_SNAPSHOT_KIND
        && same_source(&snapshot.source, source)
        && is_safe_integer(snapshot.identity_revision)
        && snapshot.identity_revision >= 0
        && character_ids.len() == total
        && snapshot
            .characters
            .iter()
            .all(|item| !item.character_id.is_empty());
    if !valid {
        return Err(FINALIZED_CHARACTER_SNAPSHOT_INVALID.to_string());
    }
    Ok(Some(snapshot))
}

/// `stateSnapshot`（基线 `summary-repository.ts:77-87`）。
pub(crate) fn state_snapshot(
    row: &CharacterStateRow,
    field: CharacterStateTextField,
    scope: &ProjectEpoch,
) -> Result<CharacterFieldSnapshot, String> {
    // `JSON.parse(String(row.cs_provenance || '{}'))`：空串/非法 JSON 回落空映射
    // （基线对非法 JSON 抛 SyntaxError；cs_provenance 写路径恒为合法 JSON）。
    let provenance: HashMap<String, Value> =
        serde_json::from_str(row.cs_provenance.as_deref().unwrap_or_default()).unwrap_or_default();
    let stored = provenance.get(field_key(field)).cloned();
    let field_provenance = match stored
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|map| map.get("kind"))
        .and_then(Value::as_str)
    {
        Some("derived") => CharacterStateFieldProvenance::Derived {
            // 基线 `{ kind: 'derived', source: {...stored.source} }`；source 缺失时
            // 序列化省略该键，Rust 以零值兜底（写路径恒带 source，文档化偏差）。
            source: serde_json::from_value(
                stored
                    .as_ref()
                    .and_then(|stored| stored.get("source"))
                    .cloned()
                    .unwrap_or(Value::Null),
            )
            .unwrap_or(FinalizedSourceIdentity {
                draft_id: 0,
                finalization_id: String::new(),
                chapter_number: 0,
                content_hash: String::new(),
            }),
        },
        Some("author") => CharacterStateFieldProvenance::Author {
            chapter_number: stored
                .as_ref()
                .and_then(|stored| stored.get("chapterNumber"))
                .and_then(Value::as_i64)
                .unwrap_or(0),
        },
        _ => CharacterStateFieldProvenance::Legacy,
    };
    let revision = stored
        .as_ref()
        .and_then(|stored| stored.get("revision"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if !is_safe_integer(revision) || revision < 0 {
        return Err(FINALIZED_CHARACTER_FIELD_REVISION_INVALID.to_string());
    }
    let value = row.state_column(field).unwrap_or_default().to_string();
    Ok(CharacterFieldSnapshot {
        project_id: scope.project_id.clone(),
        epoch: scope.epoch.clone(),
        character_id: row.character_id.clone(),
        field,
        revision,
        value_hash: text_hash(&value),
        provenance: field_provenance,
        value,
        source_order: stored
            .as_ref()
            .and_then(|stored| stored.get("sourceOrder"))
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok()),
    })
}

/// `sameSource`（基线 `character-identity.ts`）：四字段全等。
fn same_source(left: &FinalizedSourceIdentity, right: &FinalizedSourceIdentity) -> bool {
    left == right
}

// ============================================================================
// finalizedCharacterPromptCards + decideDerivedPatch 移植
// （src/shared/finalized-continuity.ts:126-141 + character-identity.ts:67-94）
// ============================================================================

/// 生成 prompt 的角色卡（`finalizedCharacterPromptCards` 的元素类型）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FinalizedCharacterPromptCard {
    character_id: String,
    name: String,
    aliases: Vec<String>,
    fields: Vec<FinalizedCharacterPromptCardField>,
}

/// 生成 prompt 的角色卡字段（`{ field, value, provenance }`）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FinalizedCharacterPromptCardField {
    field: CharacterStateTextField,
    value: String,
    provenance: CharacterStateFieldProvenance,
}

/// `finalizedCharacterPromptCards`：渲染用角色卡；被当前定稿取代的
/// `recentEvents`（derived 且来自更早定稿）不进入 prompt。
fn finalized_character_prompt_cards(
    context: &FinalizedCharacterContext,
    origin_project_id: Option<&str>,
) -> Vec<FinalizedCharacterPromptCard> {
    context
        .characters
        .iter()
        .map(|character| {
            let fields = character
                .fields
                .iter()
                .filter(|field| {
                    let Some(previous) = field.source_order.as_ref() else {
                        return true;
                    };
                    let derived_source = match &field.provenance {
                        CharacterStateFieldProvenance::Derived { source } => Some(source),
                        _ => None,
                    };
                    let superseded = matches!(field.field, CharacterStateTextField::RecentEvents)
                        && field.character_id == character.character_id
                        && derived_source.is_some()
                        && derived_source.is_some_and(|source| {
                            // TS 的 `typeof source?.finalizationId === 'string'`
                            // 在 Rust 恒真（source 非 Option），照录为恒真条件。
                            is_finalized_source_identity(source)
                                && source.chapter_number == context.source.chapter_number
                                && source.finalization_id != context.source.finalization_id
                        })
                        && previous.authoritative_finalization_revision
                            < context.source_order.authoritative_finalization_revision
                        && decide_derived_patch(
                            field,
                            &DerivedCharacterPatch {
                                project_id: context.project_id.clone(),
                                epoch: context.epoch.clone(),
                                character_id: field.character_id.clone(),
                                field: field.field,
                                value: field.value.clone(),
                                value_hash: field.value_hash.clone(),
                                base_field_revision: field.revision,
                                base_value_hash: field.value_hash.clone(),
                                base_provenance: field.provenance.clone(),
                                source: context.source.clone(),
                                source_order: context.source_order.clone(),
                            },
                            &DerivedPatchAuthority {
                                source: context.source.clone(),
                                order: context.source_order.clone(),
                                origin_project_id: origin_project_id.map(str::to_string),
                            },
                        ) == DerivedPatchDecision::ApplyDerived;
                    !superseded
                })
                .map(|field| FinalizedCharacterPromptCardField {
                    field: field.field,
                    value: field.value.clone(),
                    provenance: field.provenance.clone(),
                })
                .collect();
            FinalizedCharacterPromptCard {
                character_id: character.character_id.clone(),
                name: character.display_name_snapshot.clone(),
                aliases: character.aliases.clone(),
                fields,
            }
        })
        .collect()
}

/// 派生补丁决策（`DerivedPatchDecision` 联合）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DerivedPatchDecision {
    ApplyDerived,
    AlreadyApplied,
    ProposalRequired,
    SourceConflict,
    FieldConflict,
}

/// `decideDerivedPatch` 的 `patch` 入参（`DerivedCharacterPatch`）。
#[derive(Debug, Clone)]
struct DerivedCharacterPatch {
    project_id: String,
    epoch: String,
    character_id: String,
    field: CharacterStateTextField,
    /// 上游 `DerivedCharacterPatch` 合同字段：随 patch 携带，
    /// `decideDerivedPatch` 只读 `value_hash`/`base_value_hash`，
    /// 不读 `value`（基线 `character-identity.ts:67-94` 同型）。
    #[allow(dead_code)]
    value: String,
    value_hash: String,
    base_field_revision: i64,
    base_value_hash: String,
    base_provenance: CharacterStateFieldProvenance,
    source: FinalizedSourceIdentity,
    source_order: DerivedSourceOrder,
}

/// `decideDerivedPatch` 的 `authority` 入参。
#[derive(Debug, Clone)]
struct DerivedPatchAuthority {
    source: FinalizedSourceIdentity,
    order: DerivedSourceOrder,
    origin_project_id: Option<String>,
}

/// `decideDerivedPatch`（基线 `src/shared/character-identity.ts:67-94`，1:1）。
fn decide_derived_patch(
    current: &CharacterFieldSnapshot,
    patch: &DerivedCharacterPatch,
    authority: &DerivedPatchAuthority,
) -> DerivedPatchDecision {
    let order = &patch.source_order;
    // `sameProjectEpoch(current, patch)`：双方 projectId/epoch 非空且相等。
    if current.project_id.is_empty()
        || current.epoch.is_empty()
        || current.project_id != patch.project_id
        || current.epoch != patch.epoch
        || !is_finalized_source_identity(&patch.source)
        || !is_finalized_source_identity(&authority.source)
        || !same_source(&patch.source, &authority.source)
        || !is_content_hash(&patch.source.content_hash)
        || order.continuity_epoch.is_empty()
        || order.continuity_epoch != authority.order.continuity_epoch
        || order.chapter_number != authority.order.chapter_number
        || order.chapter_number != patch.source.chapter_number
        || order.authoritative_finalization_revision
            != authority.order.authoritative_finalization_revision
        || !is_safe_integer(order.chapter_number)
        || order.chapter_number < 1
        || !is_safe_integer(order.authoritative_finalization_revision)
        || order.authoritative_finalization_revision < 1
    {
        return DerivedPatchDecision::SourceConflict;
    }
    if current.character_id != patch.character_id
        || current.field != patch.field
        || patch.character_id.is_empty()
        || !is_content_hash(&patch.value_hash)
        || !is_content_hash(&patch.base_value_hash)
        || !is_safe_integer(current.revision)
        || current.revision < 0
    {
        return DerivedPatchDecision::FieldConflict;
    }
    let previous = current.source_order.as_ref();
    let current_derived_source = match &current.provenance {
        CharacterStateFieldProvenance::Derived { source } => Some(source),
        _ => None,
    };
    if let Some(derived_source) = current_derived_source {
        let mismatched = match previous {
            None => true,
            Some(previous) => previous.chapter_number != derived_source.chapter_number,
        };
        if mismatched {
            return DerivedPatchDecision::SourceConflict;
        }
    }
    if let Some(previous) = previous {
        if previous.continuity_epoch.is_empty()
            || !is_safe_integer(previous.chapter_number)
            || previous.chapter_number < 1
            || !is_safe_integer(previous.authoritative_finalization_revision)
            || previous.authoritative_finalization_revision < 1
        {
            return DerivedPatchDecision::SourceConflict;
        }
        let epoch_advanced = previous.continuity_epoch != order.continuity_epoch
            && !(current_derived_source.is_some()
                && advances_project_continuity(
                    &patch.project_id,
                    &previous.continuity_epoch,
                    &order.continuity_epoch,
                    authority.origin_project_id.as_deref(),
                ));
        if epoch_advanced
            || previous.chapter_number > order.chapter_number
            || (previous.chapter_number == order.chapter_number
                && previous.authoritative_finalization_revision
                    > order.authoritative_finalization_revision)
        {
            return DerivedPatchDecision::SourceConflict;
        }
    }
    if current_derived_source.is_some_and(|derived_source| {
        same_source(derived_source, &patch.source)
    } && current.value_hash == patch.value_hash)
    {
        return DerivedPatchDecision::AlreadyApplied;
    }
    if current.revision != patch.base_field_revision
        || current.value_hash != patch.base_value_hash
        || !same_provenance(&current.provenance, &patch.base_provenance)
    {
        return DerivedPatchDecision::FieldConflict;
    }
    if current_derived_source.is_none()
        && !(matches!(current.provenance, CharacterStateFieldProvenance::Legacy)
            && current.value.is_empty())
    {
        return DerivedPatchDecision::ProposalRequired;
    }
    DerivedPatchDecision::ApplyDerived
}

/// `sameProvenance`（基线 `character-identity.ts`）。
fn same_provenance(
    left: &CharacterStateFieldProvenance,
    right: &CharacterStateFieldProvenance,
) -> bool {
    matches!(
        (left, right),
        (
            CharacterStateFieldProvenance::Legacy,
            CharacterStateFieldProvenance::Legacy
        )
    ) || matches!(
        (left, right),
        (
            CharacterStateFieldProvenance::Author {
                chapter_number: left_chapter
            },
            CharacterStateFieldProvenance::Author {
                chapter_number: right_chapter
            }
        ) if left_chapter == right_chapter
    ) || matches!(
        (left, right),
        (
            CharacterStateFieldProvenance::Derived {
                source: left_source
            },
            CharacterStateFieldProvenance::Derived {
                source: right_source
            }
        ) if same_source(left_source, right_source)
    )
}

/// `generation(epoch, project)`：`{project}:{number}` 纪元的代数提取。
fn epoch_generation(epoch: &str, project: &str) -> Option<i64> {
    if project.is_empty() || !epoch.starts_with(project) || !epoch[project.len()..].starts_with(':')
    {
        return None;
    }
    let value = &epoch[project.len() + 1..];
    let valid = value == "0"
        || (value.starts_with(|digit: char| digit.is_ascii_digit() && digit != '0')
            && value.chars().skip(1).all(|digit| digit.is_ascii_digit()));
    if !valid {
        return None;
    }
    let parsed = value.parse::<i64>().ok()?;
    if !is_safe_integer(parsed) {
        return None;
    }
    Some(parsed)
}

/// `advancesProjectContinuity`（基线 `character-identity.ts`）。
fn advances_project_continuity(
    project_id: &str,
    previous: &str,
    current: &str,
    origin_project_id: Option<&str>,
) -> bool {
    let after = epoch_generation(current, project_id);
    let before = epoch_generation(previous, project_id);
    let from_origin = if before.is_none() {
        origin_project_id.and_then(|origin| epoch_generation(previous, origin))
    } else {
        None
    };
    after.is_some()
        && (before.is_some_and(|before| after.unwrap() > before)
            || from_origin.is_some_and(|from_origin| after.unwrap() >= from_origin))
}

// ============================================================================
// 最终输出合同（基线 finalization-generation-source.ts:42-43，逐字移植）
// ============================================================================

const EN_US_FINAL_OUTPUT_CONTRACT: &str = r#"[Final output contract: this overrides the [JSON output contract] above] Do not copy that example: its newCharacters list and full-field layout do not apply. A person with an existing character card must use that card's exact characterId, and every update must include evidence. Every update must include recentEvents (this character's latest state at the end of the chapter, within 50 words). The recentEvents of each update states the latest state of this character at the end of the chapter: when later prose corrects, withdraws, or postpones an earlier plan, follow the last correction and prioritize that correction and its current conditions over a more prominent earlier event; do not write the superseded plan or describe a still-pending plan as executed. Without a correction, retain this character's relevant event or still-pending plan. Do not apply another character's change of plan to this character; evidence quotes the one sentence that directly supports that latest state. List other fields only when the value differs from the existing card and matches the field (location is only the place the chapter prose explicitly states the character is currently in, never an event or progress; when the prose only states a plan, a decision, or an intention to go somewhere, the character is still where they were and must not be written as en route or arrived; when the prose states no clear change of place, do not list location). Return one JSON object: {"updates":[{"characterId":"exact ID from the frozen list","currentState":{"recentEvents":"this chapter's event","location":"new place"},"evidence":{"text":"exact source quote"}}]}. Copy a quote that occurs exactly once in the unmodified chapter; its offsets will be calculated. If you supply start/end, they must be exact JS UTF-16 offsets. evidence.text must be one contiguous, verbatim span of the frozen chapter that occurs exactly once; never join sentences across paragraphs or blank lines, and do not alter punctuation or whitespace. If unsure of the position, omit start/end and let the program calculate them. Only supplied dynamic fields are proposed. Do not infer identity from a name. Unknown or ambiguous people use name without characterId and remain proposals. Do not return static character facts or author provenance."#;

const ZH_CN_FINAL_OUTPUT_CONTRACT: &str = r#"【最终输出合同，覆盖上文【输出格式（JSON）】】不得照抄上文示例：其中的 newCharacters 与全字段写法不适用；现有角色卡中的人物必须使用其精确 characterId，每个 update 都必须含 evidence；每个 update 必须填写 recentEvents（本章结束时该角色的最新状态，50字以内）；每个 update 的 recentEvents 写本章结束时该角色的最新状态：正文后文更正、撤回或推迟了前文安排时，以最后的更正为准，优先保留该更正及当前条件，不得被更显著的旧事件挤掉；不写已被更正的旧安排，也不把仍待执行的安排写成已执行。没有更正时，保留该角色有关事件或仍待执行的安排；不得将其他角色的安排变化套到该角色；evidence 引用直接支持该最新状态的一句。其余字段只在值与现有角色卡不同且名实相符时列出（location 只写正文明确写出的人物当前所在地点，不写事件或进度；正文只写了计划、决定或打算前往某处时，人物仍在原处，不得写成“前往途中”“已到达”；正文没有明确写出地点变化时不要列出 location）。只返回一个 JSON 对象：{"updates":[{"characterId":"冻结名单中的精确ID","currentState":{"recentEvents":"本章事件","location":"新地点"},"evidence":{"text":"原文精确引用"}}]}。引用必须在未改写正文中仅出现一次，位置由程序精确计算；若提供 start/end，必须是准确的 JS UTF-16 位置。evidence.text 必须是冻结正文中单一连续、逐字、只出现一次的片段；不得跨段落或空行拼接多句，不得删改标点或空白；不确定位置时省略 start/end，由程序计算。只提议明确返回的动态字段；不可凭名字推断身份。未知或歧义人物只返回 name、不填 characterId，保留为待确认提议。不得输出静态角色事实或作者来源。"#;

// ============================================================================
// 单元测试：slotKey 稳定性 + capture/read 往返 + task 组装
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::generation_run_repository::RunBinding;
    use serde_json::Map;

    const TEST_PROJECT_ID: &str = "proj-1";
    const TEST_EPOCH: &str = "proj-1:3";

    fn test_template() -> PromptTemplate {
        PromptTemplate {
            key: "finalization_test_template".to_string(),
            writing_language: None,
            name: "测试模板".to_string(),
            description: "用途说明".to_string(),
            content: "正文 {{chapter_title}}（{{chapter_number}}） 角色：{{existing_cards_json}}"
                .to_string(),
            system_suffix: None,
            system_role: Some("Novelist".to_string()),
            task_guidance: None,
            variables: HashMap::new(),
            required_context_variables: None,
        }
    }

    fn test_scope() -> ProjectEpoch {
        ProjectEpoch {
            project_id: TEST_PROJECT_ID.to_string(),
            epoch: TEST_EPOCH.to_string(),
        }
    }

    fn test_slot(
        draft_id: i64,
        finalization_id: &str,
        content_hash: &str,
    ) -> FinalizationGenerationSlot {
        FinalizationGenerationSlot {
            source: FinalizedSourceIdentity {
                draft_id,
                finalization_id: finalization_id.to_string(),
                chapter_number: 7,
                content_hash: content_hash.to_string(),
            },
            step_key: FinalizationGenerationStep::ChapterNotes,
        }
    }

    /// 最小内存库：定稿草稿 + 出箱 + 角色 + 蓝图（legacy 路径，无身份 schema）。
    fn setup_legacy_db() -> (Connection, i64, String) {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE drafts (id INTEGER PRIMARY KEY AUTOINCREMENT, chapter_number INTEGER NOT NULL, version INTEGER NOT NULL DEFAULT 1, status TEXT NOT NULL DEFAULT 'draft', content_id INTEGER NOT NULL);
             CREATE TABLE contents (id INTEGER PRIMARY KEY AUTOINCREMENT, body TEXT NOT NULL);
             CREATE TABLE finalization_outbox (finalization_id TEXT PRIMARY KEY, draft_id INTEGER UNIQUE NOT NULL, chapter_number INTEGER NOT NULL, chapter_title TEXT NOT NULL DEFAULT '', content_hash TEXT NOT NULL, content_snapshot TEXT NOT NULL);
             CREATE TABLE continuity_projection_meta (id TEXT PRIMARY KEY CHECK (id = 'main'), generation INTEGER NOT NULL DEFAULT 0, stale_from_chapter INTEGER);
             CREATE TABLE characters (character_id TEXT PRIMARY KEY, retired INTEGER NOT NULL DEFAULT 0, cs_location TEXT, cs_power_level TEXT, cs_physical_state TEXT, cs_mental_state TEXT, cs_key_items TEXT, cs_recent_events TEXT, cs_provenance TEXT NOT NULL DEFAULT '{}');
             CREATE TABLE blueprints (chapter_number INTEGER PRIMARY KEY, characters TEXT NOT NULL DEFAULT '[]', notes TEXT NOT NULL DEFAULT '');
             CREATE TABLE summary_snapshots (id INTEGER PRIMARY KEY AUTOINCREMENT, draft_id INTEGER NOT NULL, chapter_number INTEGER NOT NULL, character_states TEXT, chapter_notes TEXT, continuity_facts TEXT, character_state_candidates TEXT, source_finalization_id TEXT, source_content_hash TEXT, projection_generation INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL DEFAULT (datetime('now')));",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO continuity_projection_meta (id, generation) VALUES ('main', 3)",
            [],
        )
        .unwrap();
        let content = "测试正文";
        let content_hash = text_hash(content);
        conn.execute("INSERT INTO contents (body) VALUES (?1)", [content])
            .unwrap();
        let content_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, status, content_id) VALUES (7, 5, 'finalized', ?1)",
            [content_id],
        )
        .unwrap();
        let draft_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO finalization_outbox (finalization_id, draft_id, chapter_number, chapter_title, content_hash, content_snapshot) VALUES ('fin-1', ?1, 7, '第七章', ?2, ?3)",
            rusqlite::params![draft_id, content_hash, content],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO characters (character_id, cs_location, cs_recent_events, cs_provenance) VALUES ('char-1', '仓库', '抵达仓库', '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO blueprints (chapter_number, characters, notes) VALUES (7, ?1, ?2)",
            ["[\"角色A\"]", "笔记"],
        )
        .unwrap();
        (conn, draft_id, content_hash)
    }

    fn test_generation_context(template: PromptTemplate) -> FinalizationGenerationContext {
        FinalizationGenerationContext {
            slot: test_slot(42, "fin-1", &"a".repeat(64)),
            identity: FinalizedCharacterContext {
                project_id: TEST_PROJECT_ID.to_string(),
                epoch: TEST_EPOCH.to_string(),
                source: FinalizedSourceIdentity {
                    draft_id: 42,
                    finalization_id: "fin-1".to_string(),
                    chapter_number: 7,
                    content_hash: "a".repeat(64),
                },
                content: "测试正文".to_string(),
                projection_generation: 3,
                identity_revision: 0,
                identity_status: FinalizedIdentityStatus::Legacy,
                source_order: DerivedSourceOrder {
                    continuity_epoch: TEST_EPOCH.to_string(),
                    chapter_number: 7,
                    authoritative_finalization_revision: 5,
                },
                characters: vec![FinalizedCharacterReference {
                    character_id: "char-1".to_string(),
                    display_name_snapshot: "角色一".to_string(),
                    aliases: vec!["一".to_string()],
                    fields: vec![CharacterFieldSnapshot {
                        project_id: TEST_PROJECT_ID.to_string(),
                        epoch: TEST_EPOCH.to_string(),
                        character_id: "char-1".to_string(),
                        field: CharacterStateTextField::RecentEvents,
                        revision: 1,
                        value: "抵达仓库".to_string(),
                        value_hash: text_hash("抵达仓库"),
                        provenance: CharacterStateFieldProvenance::Legacy,
                        source_order: None,
                    }],
                }],
            },
            chapter_title: "第七章".to_string(),
            chapter_entities: vec!["角色A".to_string()],
            writing_language: WritingLanguage::ZhCn,
            template,
            notes_baseline: FinalizationNotesBaseline {
                blueprint: FinalizationNotesBlueprint {
                    exists: true,
                    notes: "笔记".to_string(),
                },
                continuity_hash: text_hash("[]"),
            },
        }
    }

    #[test]
    fn slot_key_is_stable_and_rejects_invalid_slots() {
        let slot = test_slot(42, "fin-1", &"a".repeat(64));
        let first = finalization_slot_key(&slot).unwrap();
        // 同一槽位键稳定（同输入同输出）。
        assert_eq!(first, finalization_slot_key(&slot).unwrap());
        // stepKey 参与键：不同步骤不同键。
        let cards_slot = FinalizationGenerationSlot {
            step_key: FinalizationGenerationStep::CharacterCards,
            ..slot.clone()
        };
        assert_ne!(first, finalization_slot_key(&cards_slot).unwrap());
        // draftId < 1 → GENERATION_FINALIZATION_SLOT_INVALID。
        let invalid_draft = test_slot(0, "fin-1", &"a".repeat(64));
        assert_eq!(
            finalization_slot_key(&invalid_draft),
            Err(GENERATION_FINALIZATION_SLOT_INVALID.to_string())
        );
        // chapterNumber < 1 → 同上。
        let invalid_chapter = FinalizationGenerationSlot {
            source: FinalizedSourceIdentity {
                chapter_number: 0,
                ..slot.source.clone()
            },
            ..slot.clone()
        };
        assert_eq!(
            finalization_slot_key(&invalid_chapter),
            Err(GENERATION_FINALIZATION_SLOT_INVALID.to_string())
        );
        // finalizationId 空 trim → 同上。
        let blank_finalization = test_slot(42, "   ", &"a".repeat(64));
        assert_eq!(
            finalization_slot_key(&blank_finalization),
            Err(GENERATION_FINALIZATION_SLOT_INVALID.to_string())
        );
        // contentHash 非 64 位小写 hex → 同上。
        let invalid_hash = test_slot(42, "fin-1", "not-a-hash");
        assert_eq!(
            finalization_slot_key(&invalid_hash),
            Err(GENERATION_FINALIZATION_SLOT_INVALID.to_string())
        );
    }

    #[test]
    fn capture_and_read_round_trip_test() {
        let (conn, draft_id, content_hash) = setup_legacy_db();
        let slot = test_slot(draft_id, "fin-1", &content_hash);
        let scope = test_scope();
        let template = test_template();
        let context = capture_finalization_generation_context(
            &conn,
            &slot,
            &scope,
            WritingLanguage::ZhCn,
            &template,
            None,
        )
        .unwrap();

        // 捕获断言：身份 / 源序 / 角色字段 / 蓝图实体 / 笔记基线。
        assert_eq!(context.slot, slot);
        assert_eq!(context.identity.source, slot.source);
        assert_eq!(context.identity.project_id, TEST_PROJECT_ID);
        assert_eq!(context.identity.epoch, TEST_EPOCH);
        assert_eq!(context.identity.content, "测试正文");
        assert_eq!(context.identity.projection_generation, 3);
        assert_eq!(context.identity.identity_revision, 0);
        assert_eq!(
            context.identity.identity_status,
            FinalizedIdentityStatus::Legacy
        );
        assert_eq!(
            context.identity.source_order,
            DerivedSourceOrder {
                continuity_epoch: TEST_EPOCH.to_string(),
                chapter_number: 7,
                authoritative_finalization_revision: 5,
            }
        );
        // legacy 路径（无冻结快照）角色列表为空（上游 readFinalizedCharacterContext：characters = frozen?.characters ?? []）。
        assert!(context.identity.characters.is_empty());
        assert_eq!(context.chapter_title, "第七章");
        assert_eq!(context.chapter_entities, vec!["角色A".to_string()]);
        assert!(context.notes_baseline.blueprint.exists);
        assert_eq!(context.notes_baseline.blueprint.notes, "笔记");

        // 以捕获上下文构造 sourceManifest → read 回读应逐字段相等。
        let context_value = serde_json::to_value(&context).unwrap();
        let mut manifest = Map::new();
        manifest.insert(
            "finalizationGenerationContext".to_string(),
            context_value.clone(),
        );
        manifest.insert(
            "finalizationGenerationContextHash".to_string(),
            Value::String(text_hash(&context_value.to_string())),
        );
        manifest.insert(
            "finalizationGenerationSlotKey".to_string(),
            Value::String(finalization_slot_key(&slot).unwrap()),
        );
        manifest.insert(
            "finalizationGenerationOriginEpoch".to_string(),
            Value::String(TEST_EPOCH.to_string()),
        );
        manifest.insert(
            "operation".to_string(),
            Value::String("finalized-chapter-notes".to_string()),
        );
        let binding: RunBinding = serde_json::from_value(json!({
            "projectId": TEST_PROJECT_ID,
            "epoch": TEST_EPOCH,
            "fingerprint": {
                "chapterBriefHash": "", "authorGuidanceHash": "", "dependencyHash": "",
                "contextSnapshotHash": "", "templateHash": "", "skillSnapshotHash": "",
                "modelLeaseRevision": "", "policyHash": "", "outputContractHash": ""
            },
            "contextSnapshotId": "snapshot-1",
            "sourceManifest": manifest,
            "sourceRefs": [],
        }))
        .unwrap();
        let run = DurableGenerationRun {
            run_id: "run-1".to_string(),
            root_action_id: "root-1".to_string(),
            binding,
            status: "active".to_string(),
        };
        let read = read_finalization_generation_context(&run).unwrap();
        assert_eq!(read, context);

        // 篡改上下文哈希 → GENERATION_FINALIZATION_CONTEXT_INVALID。
        let mut tampered = run.clone();
        tampered.binding.source_manifest.insert(
            "finalizationGenerationContextHash".to_string(),
            Value::String("0".repeat(64)),
        );
        assert_eq!(
            read_finalization_generation_context(&tampered),
            Err(GENERATION_FINALIZATION_CONTEXT_INVALID.to_string())
        );
        // 操作名与 stepKey 不匹配 → 同上。
        let mut wrong_operation = run.clone();
        wrong_operation.binding.source_manifest.insert(
            "operation".to_string(),
            Value::String("finalized-character-state".to_string()),
        );
        assert_eq!(
            read_finalization_generation_context(&wrong_operation),
            Err(GENERATION_FINALIZATION_CONTEXT_INVALID.to_string())
        );
    }

    #[test]
    fn capture_binds_frozen_identity_snapshot_test() {
        let (conn, draft_id, content_hash) = setup_legacy_db();
        // 身份 schema 落地 → readFrozenCharacterSnapshot 进入绑定路径。
        conn.execute_batch(
            "CREATE TABLE character_identity_meta (id TEXT PRIMARY KEY);
             CREATE TABLE character_identity_proposals (proposal_id TEXT PRIMARY KEY, owner_character_id TEXT, source_key TEXT NOT NULL, source_hash TEXT NOT NULL, raw_value TEXT NOT NULL, candidate_ids_json TEXT, resolved_character_id TEXT, approval_id TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO character_identity_meta (id) VALUES ('main')",
            [],
        )
        .unwrap();
        let raw = json!({
            "kind": FINALIZED_CHARACTER_SNAPSHOT_KIND,
            "source": {
                "draftId": draft_id,
                "finalizationId": "fin-1",
                "chapterNumber": 7,
                "contentHash": content_hash,
            },
            "identityRevision": 2,
            "characters": [
                { "characterId": "char-1", "displayNameSnapshot": "角色一", "aliases": ["一"] }
            ],
        });
        let raw_value = serde_json::to_string(&raw).unwrap();
        conn.execute(
            "INSERT INTO character_identity_proposals (proposal_id, source_key, source_hash, raw_value) VALUES (?1, ?2, ?3, ?4)",
            [
                "fcs:fin-1".to_string(),
                FINALIZED_CHARACTER_SNAPSHOT_KIND.to_string(),
                text_hash(&raw_value),
                raw_value,
            ],
        )
        .unwrap();
        let slot = test_slot(draft_id, "fin-1", &content_hash);
        let scope = test_scope();
        let context = capture_finalization_generation_context(
            &conn,
            &slot,
            &scope,
            WritingLanguage::ZhCn,
            &test_template(),
            None,
        )
        .unwrap();
        assert_eq!(
            context.identity.identity_status,
            FinalizedIdentityStatus::Bound
        );
        assert_eq!(context.identity.identity_revision, 2);
        assert_eq!(
            context.identity.characters[0].display_name_snapshot,
            "角色一"
        );
        assert_eq!(
            context.identity.characters[0].aliases,
            vec!["一".to_string()]
        );
        // fields 仍从 characters 表现算（冻结快照不含 fields）。
        assert_eq!(context.identity.characters[0].fields.len(), 6);

        // 冻结再捕获：身份 / 槽位 / 基线均未变化 → 通过并回填冻结值。
        let frozen = context.clone();
        let recaptured = capture_finalization_generation_context(
            &conn,
            &slot,
            &scope,
            WritingLanguage::ZhCn,
            &test_template(),
            Some(&frozen),
        )
        .unwrap();
        assert_eq!(recaptured, frozen);

        // 槽位变化 → GENERATION_FINALIZATION_IDENTITY_CHANGED。
        // 槽位变化（仅 stepKey）→ GENERATION_FINALIZATION_IDENTITY_CHANGED（上游 proof 测试 'slot' 用例同构）。
        let moved_slot = FinalizationGenerationSlot {
            step_key: FinalizationGenerationStep::CharacterCards,
            ..slot.clone()
        };
        assert_eq!(
            capture_finalization_generation_context(
                &conn,
                &moved_slot,
                &scope,
                WritingLanguage::ZhCn,
                &test_template(),
                Some(&frozen),
            ),
            Err(GENERATION_FINALIZATION_IDENTITY_CHANGED.to_string())
        );
    }

    #[test]
    fn task_assembles_notes_and_cards_variants_test() {
        let context = test_generation_context(test_template());

        // chapter_notes：visible-text，user 消息仅渲染 prompt（不含合同）。
        let notes_task = finalization_generation_task(&context, None);
        assert_eq!(notes_task.purpose, "finalized-chapter-notes");
        assert_eq!(
            notes_task.reasoning_stage,
            Some(GenerationReasoningStage::Review)
        );
        assert_eq!(notes_task.output, GenerationOutput::VisibleText);
        assert_eq!(notes_task.messages.len(), 2);
        assert_eq!(notes_task.messages[0].role, GenerationMessageRole::System);
        assert_eq!(notes_task.messages[1].role, GenerationMessageRole::User);
        assert!(notes_task.messages[0].content.starts_with("Novelist"));
        assert!(notes_task.messages[1].content.contains("第七章"));
        assert!(notes_task.messages[1].content.contains("（7）"));
        assert!(!notes_task.messages[1]
            .content
            .contains("[Final output contract"));

        // character_cards：structured-data，user 消息为 prompt + 不可变合同。
        let mut cards_context = context.clone();
        cards_context.slot.step_key = FinalizationGenerationStep::CharacterCards;
        let cards_task = finalization_generation_task(&cards_context, Some("origin-1"));
        assert_eq!(cards_task.purpose, "finalized-character-state");
        assert_eq!(
            cards_task.reasoning_stage,
            Some(GenerationReasoningStage::Review)
        );
        assert_eq!(cards_task.output, GenerationOutput::StructuredData);
        assert_eq!(cards_task.messages.len(), 2);
        // existing_cards_json 变量进入 prompt（序列化角色卡）。
        assert!(cards_task.messages[1].content.contains("characterId"));
        assert!(cards_task.messages[1].content.contains("角色一"));
        // user 内容以逐字合同结尾（prompt\n\n合同）。
        assert!(cards_task.messages[1]
            .content
            .ends_with(ZH_CN_FINAL_OUTPUT_CONTRACT));

        // en-US 合同分支。
        let mut en_context = cards_context.clone();
        en_context.writing_language = WritingLanguage::EnUs;
        let en_task = finalization_generation_task(&en_context, None);
        assert!(en_task.messages[1]
            .content
            .ends_with(EN_US_FINAL_OUTPUT_CONTRACT));
    }

    #[test]
    fn prompt_cards_drop_superseded_recent_events_test() {
        let previous_hash = "b".repeat(64);
        let current_hash = "c".repeat(64);
        let identity = FinalizedCharacterContext {
            project_id: TEST_PROJECT_ID.to_string(),
            epoch: TEST_EPOCH.to_string(),
            source: FinalizedSourceIdentity {
                draft_id: 42,
                finalization_id: "fin-1".to_string(),
                chapter_number: 7,
                content_hash: current_hash,
            },
            content: "正文".to_string(),
            projection_generation: 3,
            identity_revision: 1,
            identity_status: FinalizedIdentityStatus::Bound,
            source_order: DerivedSourceOrder {
                continuity_epoch: TEST_EPOCH.to_string(),
                chapter_number: 7,
                authoritative_finalization_revision: 5,
            },
            characters: vec![
                FinalizedCharacterReference {
                    character_id: "char-1".to_string(),
                    display_name_snapshot: "角色一".to_string(),
                    aliases: vec![],
                    fields: vec![
                        // 上一定稿派生的 recentEvents：被当前定稿取代 → 不进 prompt。
                        CharacterFieldSnapshot {
                            project_id: TEST_PROJECT_ID.to_string(),
                            epoch: TEST_EPOCH.to_string(),
                            character_id: "char-1".to_string(),
                            field: CharacterStateTextField::RecentEvents,
                            revision: 2,
                            value: "旧事件".to_string(),
                            value_hash: text_hash("旧事件"),
                            provenance: CharacterStateFieldProvenance::Derived {
                                source: FinalizedSourceIdentity {
                                    draft_id: 41,
                                    finalization_id: "fin-0".to_string(),
                                    chapter_number: 7,
                                    content_hash: previous_hash,
                                },
                            },
                            source_order: Some(DerivedSourceOrder {
                                continuity_epoch: "proj-1:2".to_string(),
                                chapter_number: 7,
                                authoritative_finalization_revision: 4,
                            }),
                        },
                        // 作者字段：保留。
                        CharacterFieldSnapshot {
                            project_id: TEST_PROJECT_ID.to_string(),
                            epoch: TEST_EPOCH.to_string(),
                            character_id: "char-1".to_string(),
                            field: CharacterStateTextField::Location,
                            revision: 1,
                            value: "仓库".to_string(),
                            value_hash: text_hash("仓库"),
                            provenance: CharacterStateFieldProvenance::Author { chapter_number: 7 },
                            source_order: None,
                        },
                    ],
                },
                FinalizedCharacterReference {
                    character_id: "char-2".to_string(),
                    display_name_snapshot: "角色二".to_string(),
                    aliases: vec![],
                    fields: vec![
                        // legacy recentEvents（值非空）→ proposal-required → 保留。
                        CharacterFieldSnapshot {
                            project_id: TEST_PROJECT_ID.to_string(),
                            epoch: TEST_EPOCH.to_string(),
                            character_id: "char-2".to_string(),
                            field: CharacterStateTextField::RecentEvents,
                            revision: 1,
                            value: "仍在".to_string(),
                            value_hash: text_hash("仍在"),
                            provenance: CharacterStateFieldProvenance::Legacy,
                            source_order: None,
                        },
                    ],
                },
            ],
        };
        let cards = finalized_character_prompt_cards(&identity, Some("origin-1"));
        assert_eq!(cards.len(), 2);
        // char-1：recentEvents 被取代丢弃，仅保留 location。
        assert_eq!(cards[0].fields.len(), 1);
        assert_eq!(cards[0].fields[0].field, CharacterStateTextField::Location);
        assert_eq!(cards[0].fields[0].value, "仓库");
        // char-2：legacy recentEvents 保留。
        assert_eq!(cards[1].fields.len(), 1);
        assert_eq!(
            cards[1].fields[0].field,
            CharacterStateTextField::RecentEvents
        );
        assert_eq!(cards[1].fields[0].value, "仍在");
    }
}
