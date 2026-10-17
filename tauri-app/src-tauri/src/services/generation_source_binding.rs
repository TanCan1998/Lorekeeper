//! 生成源绑定（平移自 `electron/services/generation-source-binding.ts`，615 行）。
//!
//! 纯服务层平移：无命令注册、无前端改动。入口：
//! - [`build_generation_source_binding`]（基线 `buildGenerationSourceBinding`）
//! - [`rebuild_generation_source_binding`]（基线 `rebuildGenerationSourceBinding`）
//! - [`compare_generation_source_bindings`]（基线 `compareGenerationSourceBindings`）
//!
//! 基线语义对齐要点：
//! - 哈希为 UTF-8 字节 SHA-256 小写 hex（基线 `createHash('sha256')`，
//!   `text_hash` 形态对齐 `editor_inline_generation` 先例）；
//! - `stable` 为递归键排序后的紧凑 JSON 序列化（基线 `stable` 的
//!   `localeCompare` 排序；本域键均为 ASCII camelCase，字节序与
//!   `localeCompare('en-US')` 一致，对齐 `blueprint_repository` /
//!   `llm::lease::canonical_json` 先例）；
//! - `isDeepStrictEqual` 为 `serde_json::Value` 深度相等（对齐
//!   `review_revision_generation::is_deep_strict_equal` 先例）；
//! - 基线 `db.transaction(fn)` 读快照用 `unchecked_transaction` 包裹
//!   （失败时 `Transaction` drop 即回滚，与基线语义一致）。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use rusqlite::{Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::repositories::generation_run_repository::{
    FrozenInputFingerprint, GenerationAuthorInput, PlotOutlineContent, PlotOutlineRange,
    ProjectCoreSynopsisExpected, ProjectEpoch, RunBinding, SourceRef,
};
use crate::services::agent_generation_context::{
    build_agent_generation_context, validate_agent_generation_input, AgentGenerationInput,
};
use crate::services::current_character_projection::current_character_projection;
use crate::services::editor_inline_generation::{
    build_editor_inline_context, editor_inline_task, validate_editor_inline_input,
};
use crate::services::finalization_generation_source::{
    capture_finalization_generation_context, finalization_generation_task, finalization_slot_key,
    FinalizationGenerationContext, FinalizationGenerationSlot, FinalizationGenerationStep,
};
use crate::services::graph_generation::{proven_graph_own_event_ids, GraphGenerationTask};
use crate::services::graph_generation_source::{
    capture_graph_generation_context, graph_generation_task, validate_graph_generation_input,
    GraphGenerationContext, GraphGenerationInput, GraphGenerationScope,
};
use crate::services::import_generation::ImportGenerationSlot;
use crate::services::import_generation_source::{
    capture_import_generation_context, import_generation_slot_key,
};
use crate::services::legacy_roster_generation::LegacyRosterGenerationContext;
use crate::services::legacy_roster_generation_context::{
    capture_legacy_roster_generation_context, legacy_roster_generation_task,
    LegacyRosterGenerationScope,
};
use crate::services::portable_current_authority::{
    read_portable_current_authority, PortableCurrentAuthority, ReadPortableCurrentAuthorityInput,
};
use crate::services::prompt_system_role::{PromptTemplate, WritingLanguage};
use crate::services::review_revision_context::{
    capture_review_revision_context, review_revision_request, HumanConfirmedReviewItem,
    HumanConfirmedReviewSnapshot, ReviewRevisionContext, ReviewRevisionOperation,
};
use crate::services::source_ref::{
    is_content_hash, ContextEstimate, ContextSnapshot, ContextSource, ContextSourceSlot,
    TransferAuthority,
};
use crate::writing_skills::{inspect_writing_skill_markdown, WRITING_SKILL_STAGES};

// ============================================================================
// 基线错误码（逐字对齐 electron 基线 throw 的 Error 消息）
// ============================================================================

const GENERATION_SOURCE_BINDING_INVALID: &str = "GENERATION_SOURCE_BINDING_INVALID";
const GENERATION_PROMPT_SELECTION_REQUIRED: &str = "GENERATION_PROMPT_SELECTION_REQUIRED";
const GENERATION_LEGACY_SELECTION_INVALID: &str = "GENERATION_LEGACY_SELECTION_INVALID";
const GENERATION_GRAPH_SELECTION_INVALID: &str = "GENERATION_GRAPH_SELECTION_INVALID";
const GENERATION_BLUEPRINT_SELECTION_INVALID: &str = "GENERATION_BLUEPRINT_SELECTION_INVALID";
const GENERATION_CHAPTER_INVALID: &str = "GENERATION_CHAPTER_INVALID";
const GENERATION_SELECTION_INVALID: &str = "GENERATION_SELECTION_INVALID";
const GENERATION_SELECTION_AMBIGUOUS: &str = "GENERATION_SELECTION_AMBIGUOUS";
const GENERATION_ASSET_ID_INVALID: &str = "GENERATION_ASSET_ID_INVALID";
const GENERATION_MODEL_RECEIPT_INVALID: &str = "GENERATION_MODEL_RECEIPT_INVALID";
const GENERATION_MATERIAL_DECISION_INVALID: &str = "GENERATION_MATERIAL_DECISION_INVALID";
const GENERATION_METADATA_INVALID: &str = "GENERATION_METADATA_INVALID";
const GENERATION_METADATA_PRIVATE_PATH: &str = "GENERATION_METADATA_PRIVATE_PATH";
const GENERATION_METADATA_PRIVATE_FIELD: &str = "GENERATION_METADATA_PRIVATE_FIELD";
const GENERATION_ASSET_PATH_INVALID: &str = "GENERATION_ASSET_PATH_INVALID";
const GENERATION_ASSET_UNSAFE: &str = "GENERATION_ASSET_UNSAFE";
const GENERATION_AUTHOR_INPUT_INVALID: &str = "GENERATION_AUTHOR_INPUT_INVALID";
const GENERATION_AUTHOR_INPUT_TOO_LARGE: &str = "GENERATION_AUTHOR_INPUT_TOO_LARGE";
const GENERATION_PROJECT_CORE_MISSING: &str = "GENERATION_PROJECT_CORE_MISSING";
const GENERATION_REVIEW_CONTEXT_CHANGED: &str = "GENERATION_REVIEW_CONTEXT_CHANGED";
const GENERATION_SOURCE_MISSING: &str = "GENERATION_SOURCE_MISSING";
const GENERATION_DRAFT_CONTINUITY_RANGE_INVALID: &str = "GENERATION_DRAFT_CONTINUITY_RANGE_INVALID";
const GENERATION_FINALIZED_SOURCE_NOT_CURRENT: &str = "GENERATION_FINALIZED_SOURCE_NOT_CURRENT";
const GENERATION_DRAFT_NOT_CURRENT: &str = "GENERATION_DRAFT_NOT_CURRENT";
const GENERATION_CHAPTER_MISSING: &str = "GENERATION_CHAPTER_MISSING";
const GENERATION_BUILTIN_PROMPT_MISSING: &str = "GENERATION_BUILTIN_PROMPT_MISSING";
const GENERATION_PROMPT_INVALID: &str = "GENERATION_PROMPT_INVALID";
const GENERATION_SKILL_BINDINGS_INVALID: &str = "GENERATION_SKILL_BINDINGS_INVALID";
const GENERATION_SKILL_ID_INVALID: &str = "GENERATION_SKILL_ID_INVALID";
const GENERATION_SKILL_MISSING: &str = "GENERATION_SKILL_MISSING";
const GENERATION_SKILL_INCOMPATIBLE: &str = "GENERATION_SKILL_INCOMPATIBLE";
const GENERATION_ASSET_CHANGED_DURING_SNAPSHOT: &str = "GENERATION_ASSET_CHANGED_DURING_SNAPSHOT";
const GENERATION_SOURCE_CHANGED_DURING_SNAPSHOT: &str = "GENERATION_SOURCE_CHANGED_DURING_SNAPSHOT";
const GENERATION_AGENT_PROMPT_REQUIRED: &str = "GENERATION_AGENT_PROMPT_REQUIRED";
const GENERATION_SOURCE_MANIFEST_INVALID: &str = "GENERATION_SOURCE_MANIFEST_INVALID";
const GENERATION_PLOT_OUTLINE_POLICY_INVALID: &str = "GENERATION_PLOT_OUTLINE_POLICY_INVALID";
const GENERATION_PLOT_OUTLINE_SOURCE_INVALID: &str = "GENERATION_PLOT_OUTLINE_SOURCE_INVALID";
const GENERATION_PLOT_OUTLINE_PREFIX_INVALID: &str = "GENERATION_PLOT_OUTLINE_PREFIX_INVALID";

// ============================================================================
// 基线 generation-owner-contract 常量（逐字对齐 src/shared/generation-owner-contract.ts）
// ============================================================================

const MATERIAL_DECISION_RECEIPT_VERSION: i64 = 1;
const MATERIAL_DECISION_UNIT_METHOD_VERSION: &str = "utf8-bytes-v1";
const MATERIAL_DECISION_MAX_INPUT_UNITS: i64 = 8 * 1024 * 1024;
const MATERIAL_DECISION_MAX_SOURCES: usize = 1_024;
const MATERIAL_DECISION_CATEGORIES: [&str; 5] = [
    "author",
    "finalized-history",
    "future-plan",
    "derived-locator",
    "reference",
];
const MATERIAL_DECISION_OMISSION_REASONS: [&str; 12] = [
    "unknown-provenance",
    "locator-statement-not-evidence",
    "candidate-not-admitted",
    "invalid-source-ref",
    "plot-tree-not-manuscript",
    "duplicate-content",
    "duplicate-source-ref",
    "budget",
    "source-invalid",
    "evidence-not-locatable",
    "no-relevant-passage",
    "deduplicated-against-finalized",
];

/// 基线 `MATERIAL_DECISION_SOURCE_ID`（`/^(?:author:required|(?:finalized|candidate|review:confirmed):[1-9]\d*|reference:(?:0|[1-9]\d*))$/u`）。
static MATERIAL_DECISION_SOURCE_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:author:required|(?:finalized|candidate|review:confirmed):[1-9][0-9]*|reference:(?:0|[1-9][0-9]*))$")
        .unwrap()
});

/// 基线 `promptKeys` 键形态（`/^[A-Za-z0-9_-]{1,128}$/`）。
static PROMPT_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_-]{1,128}$").unwrap());

/// 基线 `authorInputs[].id` 形态（`/^[A-Za-z0-9][A-Za-z0-9:_-]{0,127}$/`）。
static AUTHOR_INPUT_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9:_-]{0,127}$").unwrap());

/// 基线 `safeMetadata` 的私有路径形态（`/^(?:[A-Za-z]:[\\/]|\\\\|file:\/\/)/`）。
static PRIVATE_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[A-Za-z]:[\\/]|\\\\|file://)").unwrap());

/// 基线 `safeMetadata` 的私有字段名（`/(?:api.?key|password|secret|credential|grant|absolute.?path|baseUrl|leaseId|expiresAt|createdAt)/i`）。
static PRIVATE_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:api.?key|password|secret|credential|grant|absolute.?path|baseUrl|leaseId|expiresAt|createdAt)",
    )
    .unwrap()
});

/// 基线 `skillBindings.bindings[stage]` 形态（`/^(builtin|user|project):[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/`）。
static SKILL_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(builtin|user|project):[A-Za-z0-9][A-Za-z0-9._-]{0,127}$").unwrap()
});

pub const PLOT_OUTLINE_PROTOCOL: &str = "chapter-outline-v3";
const PLOT_OUTLINE_MAX_ATTEMPTS_PER_CHAPTER: i64 = 2;

/// 基线 `CHINESE_DIGITS`（plot-outline-contract 标题数字解析）。
const CHINESE_DIGITS: &str = "零〇一二两三四五六七八九";

// ============================================================================
// 基线类型
// ============================================================================

/// 基线 `SafeGenerationModelReceipt`
/// （`Omit<ModelExecutionLeaseReceipt, 'leaseId' | 'createdAt' | 'expiresAt'>`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeGenerationModelReceipt {
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    pub model_revision: String,
    pub endpoint_fingerprint: String,
    /// 基线为 `ModelExecutionCapabilityEvidence` 结构；本服务只做真值与
    /// 脱敏校验并原样冻结（基线 `modelProjection` 的透传语义），故全保真 `Value`。
    pub capability_evidence: Value,
}

/// 基线 `GenerationKnowledgeItem`（`src/shared/generation-knowledge.ts`；
/// Tauri 侧此前无定义，按 shared 类型平移形态定义于本模块）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationKnowledgeItem {
    pub revision: i64,
    pub chunk_id: String,
    pub document_id: String,
    pub chunk_index: i64,
    pub text: String,
    pub file_name: String,
    pub score: f64,
    pub corpus_kind: KnowledgeCorpusKind,
    pub content_hash: String,
    pub document_chunk_count: i64,
    pub document_chunk_set_hash: String,
}

/// 基线 `corpusKind: 'reference' | 'project-knowledge' | 'unknown'`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KnowledgeCorpusKind {
    Reference,
    ProjectKnowledge,
    Unknown,
}

/// 基线 `state: 'empty' | 'stored'`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KnowledgeState {
    Empty,
    Stored,
}

/// 基线 `storageState: 'absent' | 'empty' | 'present'`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KnowledgeStorageState {
    Absent,
    Empty,
    Present,
}

/// 基线 `GenerationKnowledgeSnapshot`（`src/shared/generation-knowledge.ts`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationKnowledgeSnapshot {
    pub version: i64,
    pub state: KnowledgeState,
    pub storage_state: KnowledgeStorageState,
    pub query: String,
    pub top_k: i64,
    pub canonical_revision: Option<i64>,
    pub documents_revision: Option<i64>,
    pub items: Vec<GenerationKnowledgeItem>,
}

/// 基线 `GenerationSourceBindingInput`。
///
/// 子对象按 Tauri 依赖签名建模：渲染层原始输入（`agentInput` /
/// `editorInlineInput` / `graphGenerationInput` / `materialDecision` 等）
/// 保持全保真 `Value`（对应基线的运行时 `unknown` 校验入口），
/// 其余按既有共享类型强类型化。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationSourceBindingInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_outline_source: Option<Value>,
    pub project_id: String,
    pub epoch: String,
    pub operation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_number: Option<i64>,
    pub selected_draft_ids: Vec<i64>,
    pub selected_finalized_draft_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_blueprint_chapter_numbers: Option<Vec<i64>>,
    pub prompt_keys: Vec<String>,
    pub skill_stages: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_inputs: Option<Vec<GenerationAuthorInput>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_intent: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_snapshot: Option<GenerationKnowledgeSnapshot>,
    /// 仅在 owner 签发主定稿身份上下文后注入。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalized_character_context_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_revision_context: Option<ReviewRevisionContext>,
    /// 内部 Agent 准入输入；绝不接受泛型 generation:begin。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_workflow_registration_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_slot: Option<ImportGenerationSlot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor_inline_input: Option<Value>,
    /// 主进程冻结初始身份，供丢响应恢复导航跨 resume 使用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor_inline_origin_epoch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalization_generation_slot: Option<FinalizationGenerationSlot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalization_generation_context: Option<FinalizationGenerationContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_generation_input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_generation_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_generation_context: Option<GraphGenerationContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_roster_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_roster_context: Option<LegacyRosterGenerationContext>,
    pub model_receipt: SafeGenerationModelReceipt,
    pub policy: Value,
    pub output_contract: Value,
    /// S10B 章节材料准入裁决的脱敏收据：渲染层是唯一作者，本服务只校验
    /// 形状、原样冻进 sourceManifest，并把它的哈希折进 contextSnapshotHash。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_decision: Option<Value>,
}

/// 基线 `GenerationSourceBindingDependencies`。
pub struct GenerationSourceBindingDependencies<'a> {
    pub db: &'a Connection,
    pub project_storage_root: &'a Path,
    pub global_data_root: &'a Path,
    /// 基线 `readBuiltinPrompt`（返回 `JSON.stringify(template)`；
    /// 模板缺失时返回 `undefined` → `None`，由调用点抛
    /// `GENERATION_BUILTIN_PROMPT_MISSING`）。
    pub read_builtin_prompt: &'a dyn Fn(&str, WritingLanguage) -> Option<String>,
    /// 基线 `readBuiltinSkill?`（可选注入；缺失时内置技能解析为
    /// `GENERATION_SKILL_MISSING`）。
    pub read_builtin_skill: Option<&'a dyn Fn(&str, WritingLanguage) -> Option<String>>,
}

/// 基线 `GenerationSourceBindingResult` 的 `materials` 条目
/// （`{ ref: SourceRef; text: string }`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationSourceMaterial {
    #[serde(rename = "ref")]
    pub source_ref: SourceRef,
    pub text: String,
}

/// 基线 `GenerationSourceBindingResult`。
#[derive(Debug, Clone, PartialEq)]
pub struct GenerationSourceBindingResult {
    pub binding: RunBinding,
    pub context: ContextSnapshot,
    pub materials: Vec<GenerationSourceMaterial>,
}

/// 基线 `MaterialDecisionIncludedSource`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialDecisionIncludedSource {
    pub source_id: String,
    pub revision: i64,
    pub content_hash: String,
    pub category: String,
    pub required: bool,
    pub units: i64,
}

/// 基线 `MaterialDecisionOmittedSource`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialDecisionOmittedSource {
    pub source_id: String,
    pub revision: i64,
    pub content_hash: String,
    pub reason: String,
    pub category: String,
    pub required: bool,
}

/// 基线 `MaterialDecisionReceipt.capacity`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialDecisionCapacity {
    pub max_input_units: i64,
    pub method_version: String,
    pub admitted_units: i64,
}

/// 基线 `MaterialDecisionReceipt.coverage`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialDecisionCoverage {
    pub required: i64,
    pub included: i64,
    pub complete: bool,
}

/// 基线 `MaterialDecisionReceipt`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialDecisionReceipt {
    pub version: i64,
    pub verdict: String,
    pub prompt_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconciliation_prompt_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_outline_prompt_hash: Option<String>,
    pub capacity: MaterialDecisionCapacity,
    pub coverage: MaterialDecisionCoverage,
    pub included: Vec<MaterialDecisionIncludedSource>,
    pub omitted: Vec<MaterialDecisionOmittedSource>,
}

// ============================================================================
// 基线工具函数
// ============================================================================

/// 基线 `fail(code)`：以错误码字符串作为错误值抛出。
fn fail(code: &str) -> String {
    code.to_string()
}

/// 基线 `hash`（`createHash('sha256').update(text).digest('hex')`）。
fn hash(text: &str) -> String {
    hash_bytes(text.as_bytes())
}

/// 基线 `hash` 的 Buffer 重载（原始字节）。
fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 基线 `stable`：递归键排序后 `JSON.stringify`。
fn stable(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            Value::Object(record) => {
                let mut entries: Vec<(String, Value)> = record
                    .iter()
                    .map(|(key, entry)| (key.clone(), sorted(entry)))
                    .collect();
                entries.sort_by(|left, right| left.0.cmp(&right.0));
                Value::Object(entries.into_iter().collect())
            }
            _ => value.clone(),
        }
    }
    serde_json::to_string(&sorted(value)).unwrap_or_default()
}

/// 基线 `JSON.stringify(value)`（键序保持序列化声明序，与基线对象字面量
/// 插入序一致；用于 `JSON.stringify(context)` / `JSON.stringify(task)` 等
/// **非** `stable` 调用点）。
fn stringify<T: Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// 基线 `isDeepStrictEqual` 的 JSON 等价物（对齐
/// `review_revision_generation::is_deep_strict_equal` 先例）。
fn is_deep_strict_equal(left: &Value, right: &Value) -> bool {
    left == right
}

/// 基线 `Number.isSafeInteger`（JS 安全整数为 ±2^53-1；Rust `i64` 可能超出）。
fn is_safe_integer(value: i64) -> bool {
    (-9007199254740991..=9007199254740991).contains(&value)
}

/// JS 真值判断（基线 `!value` 的反面；`0` / `""` / `null` / `false` 为假）。
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|value| value != 0.0),
        Value::String(text) => !text.is_empty(),
        _ => true,
    }
}

/// 基线 `writingLanguageText`（`src/shared/writing-language.ts`）。
fn writing_language_text(language: &str, zh_cn: &str, en_us: &str) -> String {
    if language == "en-US" {
        en_us.to_string()
    } else {
        zh_cn.to_string()
    }
}

/// 基线 `` `${key}:${facts.language}` `` 的语言名（`WritingLanguage` 序列化名）。
fn writing_language_name(language: WritingLanguage) -> &'static str {
    match language {
        WritingLanguage::ZhCn => "zh-CN",
        WritingLanguage::EnUs => "en-US",
    }
}

// ============================================================================
// 基线 safeMetadata / modelProjection
// ============================================================================

/// 基线 `safeMetadata`：拒绝私有路径形态与私有字段名（递归）。
/// JSON 值域不含 `undefined` / `function` / `symbol` / `bigint`，
/// 基线对它们抛 `GENERATION_METADATA_INVALID` 的分支在 JSON 域不可达。
fn safe_metadata(value: &Value) -> Result<(), String> {
    if let Value::String(text) = value {
        if PRIVATE_PATH.is_match(text) {
            return Err(fail(GENERATION_METADATA_PRIVATE_PATH));
        }
    }
    if let Value::Array(items) = value {
        for item in items {
            safe_metadata(item)?;
        }
        return Ok(());
    }
    if let Value::Object(record) = value {
        for (key, entry) in record {
            if PRIVATE_FIELD.is_match(key) {
                return Err(fail(GENERATION_METADATA_PRIVATE_FIELD));
            }
            safe_metadata(entry)?;
        }
    }
    Ok(())
}

/// 基线 `modelProjection`：脱敏模型收据（仅保留七个非密钥字段）。
fn model_projection(value: &Value) -> Result<Value, String> {
    let mut safe = Map::new();
    for key in [
        "modelId",
        "provider",
        "protocol",
        "modelName",
        "modelRevision",
        "endpointFingerprint",
        "capabilityEvidence",
    ] {
        // 基线解构：键缺失 → `undefined` → `safeMetadata(undefined)` 抛
        // `GENERATION_METADATA_INVALID`。
        let entry = value
            .get(key)
            .ok_or_else(|| fail(GENERATION_METADATA_INVALID))?;
        safe_metadata(entry)?;
        safe.insert(key.to_string(), entry.clone());
    }
    for key in [
        "modelId",
        "modelRevision",
        "endpointFingerprint",
        "capabilityEvidence",
    ] {
        if !truthy(safe.get(key).expect("键已插入")) {
            return Err(fail(GENERATION_MODEL_RECEIPT_INVALID));
        }
    }
    // 基线 `JSON.parse(stable(safe))`：排序序列化后回读（值不变）。
    Ok(Value::Object(safe))
}

// ============================================================================
// 基线 readFile / guidanceNames / without
// ============================================================================

/// 基线 `readFile`：路径合法性 + 逐段符号链接防护后读取字节。
fn read_file(root: &Path, relative: &str, optional: bool) -> Result<Option<Vec<u8>>, String> {
    if !root.is_absolute()
        || Path::new(relative).is_absolute()
        || relative
            .split(['/', '\\'])
            .any(|part| part == ".." || part == ".")
    {
        return Err(fail(GENERATION_ASSET_PATH_INVALID));
    }
    let absolute = root.join(relative);
    // 基线 `path.relative(root, absolute)` 的前缀检查：`relative` 已排除
    // `..` / `.` / 绝对路径，`join` 结果必然位于 `root` 之下。
    let components: Vec<std::path::Component<'_>> = Path::new(relative).components().collect();
    let mut cursor = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        cursor.push(component.as_os_str());
        let stat = match fs::symlink_metadata(&cursor) {
            Ok(stat) => stat,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && optional => {
                return Ok(None)
            }
            Err(error) => return Err(error.to_string()),
        };
        let is_last = index + 1 == components.len();
        if stat.file_type().is_symlink()
            || (!is_last && !stat.is_dir())
            || (is_last && (!stat.is_file() || hard_link_count(&cursor, &stat) != 1))
        {
            return Err(fail(GENERATION_ASSET_UNSAFE));
        }
    }
    fs::read(&absolute)
        .map(Some)
        .map_err(|error| error.to_string())
}

/// 基线 `stat.nlink !== 1`：末段文件硬链接计数须为 1（防硬链接共享
/// 绕过路径安全检查）。std 稳定 API 不提供 nlink，对齐
/// `portable_runtime_freeze` / `import::parsing` 先例：Unix 走
/// `MetadataExt::nlink`，Windows 直调 Win32 `GetFileInformationByHandle`
/// 取 `nNumberOfLinks`（`number_of_links` 属 unstable `windows_by_handle`）。
#[cfg(unix)]
fn hard_link_count(_path: &Path, stat: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    stat.nlink()
}

#[cfg(windows)]
fn hard_link_count(path: &Path, _stat: &fs::Metadata) -> u64 {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let Some(file) = fs::File::open(path).ok() else {
        // 安全检查失败即关闭：无法取链接数时按非 1（不安全）处置。
        return 0;
    };
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `file` 持有有效句柄，`information` 为栈上可写结构；
    // 调用失败时 Win32 返回 0（不做任何写越界/悬垂访问）。
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) };
    if succeeded == 0 {
        0
    } else {
        u64::from(information.nNumberOfLinks)
    }
}

/// 基线 `guidanceNames`：项目 prompts 目录下的 `.md` guidance 文件名（排序）。
fn guidance_names(project_storage_root: &Path) -> Result<Vec<String>, String> {
    let directory = project_storage_root.join("prompts");
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let stat = fs::symlink_metadata(&directory).map_err(|error| error.to_string())?;
    if !stat.is_dir() || stat.file_type().is_symlink() {
        return Err(fail(GENERATION_ASSET_UNSAFE));
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !file_type.is_dir() && name.ends_with(".md") {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// 基线 `without(row, excluded)`：剔除指定键的记录投影。
fn without(row: &Map<String, Value>, excluded: &[&str]) -> Map<String, Value> {
    row.iter()
        .filter(|(key, _)| !excluded.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

// ============================================================================
// 基线 freezeAuthorInputs
// ============================================================================

/// 基线 `freezeAuthorInputs`：作者输入冻结（键集闭合、ID 形态、唯一性、
/// 8MiB UTF-8 字节上限）。
fn freeze_author_inputs(value: Option<&Value>) -> Result<Vec<GenerationAuthorInput>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Value::Array(items) = value else {
        return Err(fail(GENERATION_AUTHOR_INPUT_INVALID));
    };
    if items.len() > 64 {
        return Err(fail(GENERATION_AUTHOR_INPUT_INVALID));
    }
    let mut bytes = 0usize;
    let mut ids = HashSet::new();
    let mut frozen = Vec::new();
    for item in items {
        let Some(record) = item.as_object() else {
            return Err(fail(GENERATION_AUTHOR_INPUT_INVALID));
        };
        // 基线 `Object.keys(item).some(key => key !== 'id' && key !== 'text')`。
        if record.len() != 2 || !record.contains_key("id") || !record.contains_key("text") {
            return Err(fail(GENERATION_AUTHOR_INPUT_INVALID));
        }
        let Some(id) = record.get("id").and_then(Value::as_str) else {
            return Err(fail(GENERATION_AUTHOR_INPUT_INVALID));
        };
        if !AUTHOR_INPUT_ID.is_match(id) || ids.contains(id) {
            return Err(fail(GENERATION_AUTHOR_INPUT_INVALID));
        }
        let Some(text) = record.get("text").and_then(Value::as_str) else {
            return Err(fail(GENERATION_AUTHOR_INPUT_INVALID));
        };
        ids.insert(id.to_string());
        bytes += text.len();
        if bytes > 8 * 1024 * 1024 {
            return Err(fail(GENERATION_AUTHOR_INPUT_TOO_LARGE));
        }
        frozen.push(GenerationAuthorInput {
            id: id.to_string(),
            text: text.to_string(),
        });
    }
    Ok(frozen)
}

// ============================================================================
// 基线材料准入裁决（freezeMaterialDecision，S10B 步骤 3）
// ============================================================================

/// 基线 `decisionCount`：安全整数边界检查。
fn decision_count(value: &Value, minimum: i64, maximum: i64) -> Result<i64, String> {
    let Some(number) = value.as_i64() else {
        return Err(fail(GENERATION_MATERIAL_DECISION_INVALID));
    };
    if !is_safe_integer(number) || number < minimum || number > maximum {
        return Err(fail(GENERATION_MATERIAL_DECISION_INVALID));
    }
    Ok(number)
}

/// 基线 `decisionEnum`：闭合枚举校验。
fn decision_enum<'a>(value: &'a Value, allowed: &'a [&str]) -> Result<&'a str, String> {
    let Some(text) = value.as_str() else {
        return Err(fail(GENERATION_MATERIAL_DECISION_INVALID));
    };
    if !allowed.contains(&text) {
        return Err(fail(GENERATION_MATERIAL_DECISION_INVALID));
    }
    Ok(text)
}

/// 基线 `decisionFields`：键集闭合校验（键数量与键名逐项闭合）。
fn decision_fields<'a>(
    value: &'a Value,
    allowed: &'a [&'a str],
) -> Result<&'a Map<String, Value>, String> {
    let Some(record) = value.as_object() else {
        return Err(fail(GENERATION_MATERIAL_DECISION_INVALID));
    };
    if record.len() != allowed.len() || !record.keys().all(|key| allowed.contains(&key.as_str())) {
        return Err(fail(GENERATION_MATERIAL_DECISION_INVALID));
    }
    Ok(record)
}

/// 基线 `decisionIdentity`（`JSON.stringify([sourceId, revision, contentHash])`）。
fn decision_identity(source_id: &str, revision: i64, content_hash: &str) -> String {
    stringify(&Value::Array(vec![
        Value::String(source_id.to_string()),
        Value::Number(revision.into()),
        Value::String(content_hash.to_string()),
    ]))
}

/// 基线 `compareCodeUnit`（字符串字节序比较；本域为 ASCII，与
/// `localeCompare('en-US')` 一致）。
fn compare_code_unit(left: &str, right: &str) -> std::cmp::Ordering {
    left.cmp(right)
}

/// 准入裁决来源条目的冻结中间形态（included / omitted 共有字段的并集；
/// 对应基线两个接口的联合序列化形状）。
#[derive(Debug, Clone, PartialEq)]
struct MaterialDecisionSourceEntry {
    source_id: String,
    revision: i64,
    content_hash: String,
    category: String,
    required: bool,
    units: Option<i64>,
    reason: Option<String>,
}

/// 基线 `decisionSources` 的 `kind` 参数（`'included' | 'omitted'`）。
enum DecisionSourceKind {
    Included,
    Omitted,
}

/// 基线 `decisionSources`：来源条目冻结（来源身份、闭合枚举、来源 ID
/// 语义逐项校验；脱敏来自有限来源 ID / 枚举合同，不把「看起来像安全
/// 字符串」的正则当成隐私证明）。
fn decision_sources(
    value: &Value,
    kind: DecisionSourceKind,
) -> Result<Vec<MaterialDecisionSourceEntry>, String> {
    let Some(items) = value.as_array() else {
        return Err(fail(GENERATION_MATERIAL_DECISION_INVALID));
    };
    if items.len() > MATERIAL_DECISION_MAX_SOURCES {
        return Err(fail(GENERATION_MATERIAL_DECISION_INVALID));
    }
    let allowed: &[&str] = match kind {
        DecisionSourceKind::Included => &[
            "sourceId",
            "revision",
            "contentHash",
            "category",
            "required",
            "units",
        ],
        DecisionSourceKind::Omitted => &[
            "sourceId",
            "revision",
            "contentHash",
            "reason",
            "category",
            "required",
        ],
    };
    items
        .iter()
        .map(|entry| {
            let record = decision_fields(entry, allowed)?;
            let source_id = record
                .get("sourceId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !MATERIAL_DECISION_SOURCE_ID.is_match(source_id)
                || record
                    .get("required")
                    .map(|value| !value.is_boolean())
                    .unwrap_or(true)
            {
                return Err(fail(GENERATION_MATERIAL_DECISION_INVALID));
            }
            let required = record
                .get("required")
                .and_then(Value::as_bool)
                .unwrap_or_default();
            let revision = decision_count(
                record.get("revision").unwrap_or(&Value::Null),
                0,
                9007199254740991,
            )?;
            let content_hash = record
                .get("contentHash")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !is_content_hash(content_hash) {
                return Err(fail(GENERATION_MATERIAL_DECISION_INVALID));
            }
            let category = decision_enum(
                record.get("category").unwrap_or(&Value::Null),
                &MATERIAL_DECISION_CATEGORIES,
            )?
            .to_string();
            // 基线 `Number(sourceId.slice(sourceId.lastIndexOf(':') + 1))`。
            let suffix: i64 = source_id
                .rsplit(':')
                .next()
                .unwrap_or_default()
                .parse()
                .unwrap_or_default();
            let semantic = if source_id == "author:required" {
                category == "author" && required && revision == 1
            } else if source_id.starts_with("review:confirmed:") {
                (category == "author" || category == "future-plan")
                    && required
                    && revision == suffix
            } else if source_id.starts_with("finalized:") {
                category == "finalized-history" && revision == suffix
            } else if source_id.starts_with("candidate:") {
                category == "finalized-history" && revision >= 1
            } else if source_id.starts_with("reference:") {
                category == "reference" && !required && revision == 1
            } else {
                false
            };
            if !semantic {
                return Err(fail(GENERATION_MATERIAL_DECISION_INVALID));
            }
            Ok(match kind {
                DecisionSourceKind::Included => MaterialDecisionSourceEntry {
                    source_id: source_id.to_string(),
                    revision,
                    content_hash: content_hash.to_string(),
                    category,
                    required,
                    units: Some(decision_count(
                        record.get("units").unwrap_or(&Value::Null),
                        1,
                        MATERIAL_DECISION_MAX_INPUT_UNITS,
                    )?),
                    reason: None,
                },
                DecisionSourceKind::Omitted => MaterialDecisionSourceEntry {
                    source_id: source_id.to_string(),
                    revision,
                    content_hash: content_hash.to_string(),
                    reason: Some(
                        decision_enum(
                            record.get("reason").unwrap_or(&Value::Null),
                            &MATERIAL_DECISION_OMISSION_REASONS,
                        )?
                        .to_string(),
                    ),
                    category,
                    required,
                    units: None,
                },
            })
        })
        .collect()
}

/// 基线 `compareDecisionIdentity`。
fn compare_decision_identity(
    left: &MaterialDecisionSourceEntry,
    right: &MaterialDecisionSourceEntry,
) -> std::cmp::Ordering {
    compare_code_unit(&left.source_id, &right.source_id)
        .then(left.revision.cmp(&right.revision))
        .then(compare_code_unit(&left.content_hash, &right.content_hash))
}

/// 基线 omitted 排序比较器（`compareDecisionIdentity || reason || category || required`）。
fn compare_omitted(
    left: &MaterialDecisionSourceEntry,
    right: &MaterialDecisionSourceEntry,
) -> std::cmp::Ordering {
    compare_decision_identity(left, right)
        .then(compare_code_unit(
            left.reason.as_deref().unwrap_or_default(),
            right.reason.as_deref().unwrap_or_default(),
        ))
        .then(compare_code_unit(&left.category, &right.category))
        // 基线 `Number(left.required) - Number(right.required)`：false(0) < true(1)。
        .then(left.required.cmp(&right.required))
}

/// 基线 `freezeMaterialDecision`：材料准入收据冻结。
fn freeze_material_decision(value: &Value) -> Result<MaterialDecisionReceipt, String> {
    let reconciled = value.get("reconciliationPromptHash").is_some();
    let outlined = value.get("shortOutlinePromptHash").is_some();
    let mut allowed: Vec<&str> = vec!["version", "verdict", "promptHash"];
    if reconciled {
        allowed.push("reconciliationPromptHash");
    }
    if outlined {
        allowed.push("shortOutlinePromptHash");
    }
    allowed.extend(["capacity", "coverage", "included", "omitted"]);
    let record = decision_fields(value, &allowed)?;
    if record.get("version") != Some(&Value::Number(MATERIAL_DECISION_RECEIPT_VERSION.into()))
        || record.get("verdict").and_then(Value::as_str) != Some("admitted")
    {
        return Err(fail(GENERATION_MATERIAL_DECISION_INVALID));
    }
    let prompt_hash = record
        .get("promptHash")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !is_content_hash(prompt_hash)
        || reconciled
            && !record
                .get("reconciliationPromptHash")
                .and_then(Value::as_str)
                .is_some_and(is_content_hash)
        || outlined
            && (reconciled
                || !record
                    .get("shortOutlinePromptHash")
                    .and_then(Value::as_str)
                    .is_some_and(is_content_hash))
    {
        return Err(fail(GENERATION_MATERIAL_DECISION_INVALID));
    }
    let capacity = decision_fields(
        record.get("capacity").unwrap_or(&Value::Null),
        &["maxInputUnits", "methodVersion", "admittedUnits"],
    )?;
    let coverage = decision_fields(
        record.get("coverage").unwrap_or(&Value::Null),
        &["required", "included", "complete"],
    )?;
    let included_entries = decision_sources(
        record.get("included").unwrap_or(&Value::Null),
        DecisionSourceKind::Included,
    )?;
    let omitted_entries = decision_sources(
        record.get("omitted").unwrap_or(&Value::Null),
        DecisionSourceKind::Omitted,
    )?;
    let included: Vec<MaterialDecisionIncludedSource> = included_entries
        .iter()
        .map(|entry| MaterialDecisionIncludedSource {
            source_id: entry.source_id.clone(),
            revision: entry.revision,
            content_hash: entry.content_hash.clone(),
            category: entry.category.clone(),
            required: entry.required,
            units: entry.units.unwrap_or_default(),
        })
        .collect();
    let omitted: Vec<MaterialDecisionOmittedSource> = omitted_entries
        .iter()
        .map(|entry| MaterialDecisionOmittedSource {
            source_id: entry.source_id.clone(),
            revision: entry.revision,
            content_hash: entry.content_hash.clone(),
            reason: entry.reason.clone().unwrap_or_default(),
            category: entry.category.clone(),
            required: entry.required,
        })
        .collect();
    let frozen = MaterialDecisionReceipt {
        version: MATERIAL_DECISION_RECEIPT_VERSION,
        verdict: "admitted".to_string(),
        prompt_hash: prompt_hash.to_string(),
        reconciliation_prompt_hash: reconciled.then(|| {
            record
                .get("reconciliationPromptHash")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        }),
        short_outline_prompt_hash: outlined.then(|| {
            record
                .get("shortOutlinePromptHash")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        }),
        capacity: MaterialDecisionCapacity {
            max_input_units: decision_count(
                capacity.get("maxInputUnits").unwrap_or(&Value::Null),
                1,
                MATERIAL_DECISION_MAX_INPUT_UNITS,
            )?,
            method_version: decision_enum(
                capacity.get("methodVersion").unwrap_or(&Value::Null),
                &[MATERIAL_DECISION_UNIT_METHOD_VERSION],
            )?
            .to_string(),
            admitted_units: decision_count(
                capacity.get("admittedUnits").unwrap_or(&Value::Null),
                0,
                MATERIAL_DECISION_MAX_INPUT_UNITS,
            )?,
        },
        coverage: MaterialDecisionCoverage {
            required: decision_count(
                coverage.get("required").unwrap_or(&Value::Null),
                0,
                9007199254740991,
            )?,
            included: decision_count(
                coverage.get("included").unwrap_or(&Value::Null),
                0,
                9007199254740991,
            )?,
            complete: coverage.get("complete") == Some(&Value::Bool(true)),
        },
        included,
        omitted,
    };
    // 跨项闭合校验（稳定排序、身份唯一、覆盖计数、容量与准入一致性）。
    let included_identities: Vec<String> = frozen
        .included
        .iter()
        .map(|item| decision_identity(&item.source_id, item.revision, &item.content_hash))
        .collect();
    let omitted_events: Vec<String> = frozen
        .omitted
        .iter()
        .map(|item| {
            format!(
                "{}\0{}",
                decision_identity(&item.source_id, item.revision, &item.content_hash),
                item.reason
            )
        })
        .collect();
    let mut sorted_included = included_entries.clone();
    sorted_included.sort_by(compare_decision_identity);
    let mut sorted_omitted = omitted_entries.clone();
    sorted_omitted.sort_by(compare_omitted);
    let included_hashes: HashSet<&str> = frozen
        .included
        .iter()
        .map(|item| item.content_hash.as_str())
        .collect();
    // 基线 `Map`（后写覆盖先写）：included 在前、omitted 在后。
    let mut required_identities: HashMap<String, String> = HashMap::new();
    for entry in included_entries.iter().chain(omitted_entries.iter()) {
        if entry.required {
            required_identities.insert(
                decision_identity(&entry.source_id, entry.revision, &entry.content_hash),
                entry.content_hash.clone(),
            );
        }
    }
    let covered_required = required_identities
        .values()
        .filter(|content_hash| included_hashes.contains(content_hash.as_str()))
        .count();
    let admitted_units: i64 = frozen.included.iter().map(|item| item.units).sum();
    let disallowed_intersection = frozen.omitted.iter().any(|item| {
        let identity = decision_identity(&item.source_id, item.revision, &item.content_hash);
        included_identities.contains(&identity)
            && item.reason != "locator-statement-not-evidence"
            && item.reason != "evidence-not-locatable"
    });
    let distinct_included: HashSet<&String> = included_identities.iter().collect();
    let distinct_events: HashSet<&String> = omitted_events.iter().collect();
    if frozen.included.len() + frozen.omitted.len() > MATERIAL_DECISION_MAX_SOURCES
        || distinct_included.len() != included_identities.len()
        || distinct_events.len() != omitted_events.len()
        || disallowed_intersection
        || included_entries != sorted_included
        || omitted_entries != sorted_omitted
        || admitted_units != frozen.capacity.admitted_units
        || admitted_units > frozen.capacity.max_input_units
        || frozen.coverage.required != required_identities.len() as i64
        || frozen.coverage.included != covered_required as i64
        || !frozen.coverage.complete
        || frozen.coverage.included != frozen.coverage.required
    {
        return Err(fail(GENERATION_MATERIAL_DECISION_INVALID));
    }
    safe_metadata(&serde_json::to_value(&frozen).unwrap_or_default())?;
    // 基线 `JSON.parse(stable(frozen))`：排序键序回读；`serde_json::Map`
    // 为 BTreeMap（键序天然排序），类型化回读与该口径逐字节一致。
    Ok(frozen)
}

// ============================================================================
// 基线 plot-outline 纯函数（src/shared/plot-outline-contract.ts；
// `repositories/generation_run_repository.rs` 的同型私有 fn 未公开，
// 本文件按基线逐字移植，对齐 commands/generation.rs 同类先例）
// ============================================================================

/// 基线 `plotOutlinePolicy` 的策略投影。对齐
/// `repositories/generation_run_repository.rs` 的同型私有结构（未公开），
/// 与上方注释所述 `plot_outline_policy` 移植例一致，本文件按基线逐字定义。
#[derive(Debug, Clone, Copy)]
struct PlotOutlinePolicy {
    range: PlotOutlineRange,
    /// 基线契约形状（`maxAttemptsPerChapter`）保留：绑定流仅读
    /// `range`（基线 `plotOutlineConfirmedPrefix(expected, policy.range)`），
    /// `content` 的消费方在生成流程侧。
    #[allow(dead_code)]
    content: PlotOutlineContent,
}
/// 基线 `plotOutlinePolicy`。
fn plot_outline_policy(manifest: &Value) -> Result<Option<PlotOutlinePolicy>, String> {
    if manifest.get("operation").and_then(Value::as_str) != Some("generate-plot-outline") {
        return Ok(None);
    }
    let policy = manifest.get("policy");
    let planning = policy.and_then(|policy| policy.get("planning"));
    if policy.map(|policy| !policy.is_object()).unwrap_or(true)
        || planning
            .map(|planning| !planning.is_object())
            .unwrap_or(true)
        || planning
            .and_then(|planning| planning.get("kind"))
            .and_then(Value::as_str)
            != Some("architecture")
        || planning
            .and_then(|planning| planning.get("outlineProtocol"))
            .and_then(Value::as_str)
            != Some(PLOT_OUTLINE_PROTOCOL)
    {
        return Ok(None);
    }
    let intent = planning.and_then(|planning| planning.get("intent"));
    let outline_content = planning.and_then(|planning| planning.get("outlineContent"));
    if !intent.map(|value| value.is_object()).unwrap_or(false)
        || !outline_content
            .map(|value| value.is_object())
            .unwrap_or(false)
        || outline_content
            .and_then(|content| content.as_object())
            .map(|content| {
                content.len() != 1
                    || content.get("maxAttemptsPerChapter")
                        != Some(&Value::Number(PLOT_OUTLINE_MAX_ATTEMPTS_PER_CHAPTER.into()))
            })
            .unwrap_or(true)
    {
        return Err(fail(GENERATION_PLOT_OUTLINE_POLICY_INVALID));
    }
    let synopsis_range = intent.and_then(|intent| intent.get("synopsisRange"));
    let from = synopsis_range
        .and_then(|range| range.get("from"))
        .and_then(Value::as_i64);
    let to = synopsis_range
        .and_then(|range| range.get("to"))
        .and_then(Value::as_i64);
    let max_physical_requests = policy
        .and_then(|policy| policy.get("budget"))
        .and_then(|budget| budget.get("maxPhysicalRequests"))
        .and_then(Value::as_i64);
    let Some(from) = from else {
        return Err(fail(GENERATION_PLOT_OUTLINE_POLICY_INVALID));
    };
    let Some(to) = to else {
        return Err(fail(GENERATION_PLOT_OUTLINE_POLICY_INVALID));
    };
    let Some(max_physical_requests) = max_physical_requests else {
        return Err(fail(GENERATION_PLOT_OUTLINE_POLICY_INVALID));
    };
    if !is_safe_integer(from)
        || !is_safe_integer(to)
        || from < 1
        || to < from
        || to - from + 1 > max_physical_requests
    {
        return Err(fail(GENERATION_PLOT_OUTLINE_POLICY_INVALID));
    }
    Ok(Some(PlotOutlinePolicy {
        range: PlotOutlineRange { from, to },
        content: PlotOutlineContent {
            max_attempts_per_chapter: PLOT_OUTLINE_MAX_ATTEMPTS_PER_CHAPTER,
        },
    }))
}

/// 基线 `plotOutlineExpected`。
fn plot_outline_expected(value: &Value) -> Result<ProjectCoreSynopsisExpected, String> {
    let Some(record) = value.as_object() else {
        return Err(fail(GENERATION_PLOT_OUTLINE_SOURCE_INVALID));
    };
    if record.get("version") != Some(&Value::Number(1.into())) {
        return Err(fail(GENERATION_PLOT_OUTLINE_SOURCE_INVALID));
    }
    let Some(core) = record.get("core").and_then(Value::as_object) else {
        return Err(fail(GENERATION_PLOT_OUTLINE_SOURCE_INVALID));
    };
    for key in [
        "synopsis",
        "premise",
        "characters_arch",
        "worldbuilding",
        "genre",
        "writing_language",
        "plot_structure",
        "narrative_pov",
        "global_guidance",
    ] {
        if !core
            .get(key)
            .map(|value| value.is_string())
            .unwrap_or(false)
        {
            return Err(fail(GENERATION_PLOT_OUTLINE_SOURCE_INVALID));
        }
    }
    let Some(total_chapters) = core.get("total_chapters").and_then(Value::as_i64) else {
        return Err(fail(GENERATION_PLOT_OUTLINE_SOURCE_INVALID));
    };
    let Some(words_per_chapter) = core.get("words_per_chapter").and_then(Value::as_i64) else {
        return Err(fail(GENERATION_PLOT_OUTLINE_SOURCE_INVALID));
    };
    if !is_safe_integer(total_chapters) || !is_safe_integer(words_per_chapter) {
        return Err(fail(GENERATION_PLOT_OUTLINE_SOURCE_INVALID));
    }
    let text = |key: &str| {
        core.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    Ok(ProjectCoreSynopsisExpected {
        synopsis: text("synopsis"),
        premise: text("premise"),
        characters_arch: text("characters_arch"),
        worldbuilding: text("worldbuilding"),
        genre: text("genre"),
        total_chapters,
        words_per_chapter,
        writing_language: if core.get("writing_language").and_then(Value::as_str) == Some("en-US") {
            "en-US".to_string()
        } else {
            "zh-CN".to_string()
        },
        plot_structure: text("plot_structure"),
        narrative_pov: text("narrative_pov"),
        global_guidance: text("global_guidance"),
    })
}

/// 基线 `HeadingMatch`（标题匹配结果；`index` / `end` 为 UTF-8 字节偏移）。
#[derive(Debug, Clone, PartialEq)]
struct HeadingMatch {
    from: i64,
    to: i64,
    index: usize,
    end: usize,
    grouped: bool,
}

/// 基线 `chineseDigit`。
fn chinese_digit(char: char) -> Option<i64> {
    match char {
        '零' | '〇' => Some(0),
        '一' => Some(1),
        '二' | '两' => Some(2),
        '三' => Some(3),
        '四' => Some(4),
        '五' => Some(5),
        '六' => Some(6),
        '七' => Some(7),
        '八' => Some(8),
        '九' => Some(9),
        _ => None,
    }
}

/// 基线 `chineseUnit`。
fn chinese_unit(char: char) -> Option<i64> {
    match char {
        '十' => Some(10),
        '百' => Some(100),
        '千' => Some(1000),
        _ => None,
    }
}

/// 基线 `isChineseNumberChar`。
fn is_chinese_number_char(char: char) -> bool {
    char.is_ascii_digit() || CHINESE_DIGITS.contains(char) || matches!(char, '十' | '百' | '千')
}

/// 基线 `chapterNumber`（中文数字串 → 整数）。
fn chapter_number(token: &str) -> i64 {
    if !token.is_empty() && token.chars().all(|char| char.is_ascii_digit()) {
        return token.parse::<i64>().unwrap_or_default();
    }
    let mut result = 0;
    let mut digit = 0;
    for char in token.chars() {
        if let Some(value) = chinese_digit(char) {
            digit = value;
        } else if let Some(unit) = chinese_unit(char) {
            result += digit.max(1) * unit;
            digit = 0;
        }
    }
    result + digit
}

/// 基线 `matchChineseNumber`。
fn match_chinese_number(chars: &[char], start: usize) -> Option<(i64, usize)> {
    if start >= chars.len() || !is_chinese_number_char(chars[start]) {
        return None;
    }
    let mut end = start;
    while end < chars.len() && is_chinese_number_char(chars[end]) {
        end += 1;
    }
    let token: String = chars[start..end].iter().collect();
    Some((chapter_number(&token), end))
}

/// 基线 `matchAsciiDigits`。
fn match_ascii_digits(chars: &[char], start: usize) -> Option<(i64, usize)> {
    if start >= chars.len() || !chars[start].is_ascii_digit() {
        return None;
    }
    let mut end = start;
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
    }
    let token: String = chars[start..end].iter().collect();
    token.parse::<i64>().ok().map(|value| (value, end))
}

/// 中文支 `第 NUMBER([–—-]NUMBER)?章`；返回 `(from, to, 是否有第二数字)`。
fn match_chinese_chapter(chars: &[char], start: usize) -> Option<(i64, i64, bool)> {
    let (from, mut position) = match_chinese_number(chars, start + 1)?;
    let mut to = from;
    let mut grouped = false;
    if position < chars.len()
        && (chars[position] == '–' || chars[position] == '—' || chars[position] == '-')
    {
        if let Some((second, end)) = match_chinese_number(chars, position + 1) {
            to = second;
            grouped = true;
            position = end;
        }
    }
    if position >= chars.len() || chars[position] != '章' {
        return None;
    }
    position += 1;
    let mut lookahead = position;
    while lookahead < chars.len() && (chars[lookahead] == '\t' || chars[lookahead] == ' ') {
        lookahead += 1;
    }
    if !(lookahead == chars.len()
        || matches!(chars[lookahead], ':' | '：' | '.' | '．' | '—' | '-'))
    {
        return None;
    }
    Some((from, to, grouped))
}

/// 阿拉伯支 `Chapters?[\t ]+(\d+)([\t ]*[–—-][\t ]*(\d+))?`。
fn match_english_chapter(chars: &[char], start: usize) -> Option<(i64, i64, bool)> {
    if start + 7 > chars.len()
        || !chars[start..start + 7]
            .iter()
            .collect::<String>()
            .eq_ignore_ascii_case("chapter")
    {
        return None;
    }
    let mut position = start + 7;
    if position < chars.len() && (chars[position] == 's' || chars[position] == 'S') {
        position += 1;
    }
    let whitespace_start = position;
    while position < chars.len() && (chars[position] == '\t' || chars[position] == ' ') {
        position += 1;
    }
    if position == whitespace_start {
        return None;
    }
    let (from, mut end) = match_ascii_digits(chars, position)?;
    let mut to = from;
    let mut grouped = false;
    let mut range = end;
    while range < chars.len() && (chars[range] == '\t' || chars[range] == ' ') {
        range += 1;
    }
    if range < chars.len() && (chars[range] == '–' || chars[range] == '—' || chars[range] == '-')
    {
        let mut dash_end = range + 1;
        while dash_end < chars.len() && (chars[dash_end] == '\t' || chars[dash_end] == ' ') {
            dash_end += 1;
        }
        if let Some((second, second_end)) = match_ascii_digits(chars, dash_end) {
            to = second;
            grouped = true;
            end = second_end;
        }
    }
    let mut lookahead = end;
    while lookahead < chars.len() && (chars[lookahead] == '\t' || chars[lookahead] == ' ') {
        lookahead += 1;
    }
    if !(lookahead == chars.len()
        || matches!(chars[lookahead], ':' | '：' | '.' | '．' | '—' | '-'))
    {
        return None;
    }
    Some((from, to, grouped))
}

/// `/Chapters\s/iu`：匹配文本含大小写不敏感 "chapters" 紧接空白字符。
fn contains_chapters_followed_by_whitespace(text: &str) -> bool {
    let lower: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let needle: Vec<char> = "chapters".chars().collect();
    if lower.len() < needle.len() {
        return false;
    }
    for start in 0..=(lower.len() - needle.len()) {
        if lower[start..start + needle.len()] == needle[..] {
            let after = start + needle.len();
            if after < lower.len() && lower[after].is_whitespace() {
                return true;
            }
        }
    }
    false
}

/// 逐行匹配基线正则
/// `^[\t ]*(?:#{1,6}[\t ]+|[-*+][\t ]+)?(?:第(NUMBER)(?:[–—-](NUMBER))?章|Chapters?[\t ]+(\d+)(?:[\t ]*[–—-][\t ]*(\d+))?)(?=[\t ]*(?:[:：.．—-]|$))[^\r\n]*`
/// （`regex` crate 不支持前瞻断言，手工实现等价算法，含可选组回溯语义，
/// 对齐 commands/generation.rs 同类先例）。
fn match_heading_line(line: &str) -> Option<(i64, i64, bool)> {
    let chars: Vec<char> = line.chars().collect();
    let mut position = 0;
    while position < chars.len() && (chars[position] == '\t' || chars[position] == ' ') {
        position += 1;
    }
    // 可选 markdown 列表前缀 #{1,6}[\t ]+ 或 [-*+][\t ]+；不构成时整组回退为空。
    if position < chars.len() && chars[position] == '#' {
        let mut hashes = position;
        while hashes < chars.len() && chars[hashes] == '#' && hashes - position < 6 {
            hashes += 1;
        }
        if hashes - position >= 1
            && hashes < chars.len()
            && (chars[hashes] == '\t' || chars[hashes] == ' ')
        {
            while hashes < chars.len() && (chars[hashes] == '\t' || chars[hashes] == ' ') {
                hashes += 1;
            }
            position = hashes;
        }
    } else if position < chars.len()
        && (chars[position] == '-' || chars[position] == '*' || chars[position] == '+')
    {
        let marker = position + 1;
        if marker < chars.len() && (chars[marker] == '\t' || chars[marker] == ' ') {
            let mut marker = marker;
            while marker < chars.len() && (chars[marker] == '\t' || chars[marker] == ' ') {
                marker += 1;
            }
            position = marker;
        }
    }
    if position < chars.len() && chars[position] == '第' {
        if let Some(result) = match_chinese_chapter(&chars, position) {
            return Some(result);
        }
        return None;
    }
    match_english_chapter(&chars, position)
}

/// 基线 `headings`（逐行解析章号标题；`index` / `end` 累计 UTF-8 字节偏移）。
fn headings(text: &str) -> Vec<HeadingMatch> {
    let mut result = Vec::new();
    let mut byte_offset = 0;
    for line in text.split('\n') {
        let line_len = line.len();
        let match_len = if line.ends_with('\r') {
            line_len - 1
        } else {
            line_len
        };
        if let Some((from, to, grouped)) = match_heading_line(line) {
            let match_text = &line[..match_len];
            result.push(HeadingMatch {
                from,
                to,
                index: byte_offset,
                end: byte_offset + match_len,
                grouped: grouped || contains_chapters_followed_by_whitespace(match_text),
            });
        }
        byte_offset += line_len + 1;
    }
    result
}

/// 基线 `joinPlotOutlineEntries`。
fn join_plot_outline_entries(prefix: &str, next: &str) -> String {
    [prefix.trim(), next.trim()]
        .iter()
        .filter(|entry| !entry.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 基线 `renderPlotOutlineSynopsis`。
fn render_plot_outline_synopsis(
    body: &str,
    covered_to: i64,
    expected: &ProjectCoreSynopsisExpected,
) -> String {
    let english = expected.writing_language == "en-US";
    let tail = if covered_to < expected.total_chapters {
        if english {
            format!(
                "> This outline covers chapters 1-{covered_to} of {}; the remaining chapters will be generated in later batches.",
                expected.total_chapters
            )
        } else {
            format!(
                "> 本大纲已覆盖至第 {} 章（全书 {} 章），其余章节将在后续批次继续生成。",
                covered_to, expected.total_chapters
            )
        }
    } else {
        String::new()
    };
    let title = if english {
        "Plot Outline"
    } else {
        "情节大纲"
    };
    format!("# {title}\n\n{}", join_plot_outline_entries(body, &tail))
        .trim()
        .to_string()
}

/// 基线 `plotOutlineConfirmedPrefix`——已确认前缀的一致性校验与提取。
fn plot_outline_confirmed_prefix(
    expected: &ProjectCoreSynopsisExpected,
    range: &PlotOutlineRange,
) -> Result<String, String> {
    let entries = headings(&expected.synopsis);
    if entries.iter().any(|entry| {
        (entry.from < range.from && entry.to >= range.from)
            || (entry.from <= range.to && entry.to > range.to)
    }) {
        return Err(fail(GENERATION_PLOT_OUTLINE_PREFIX_INVALID));
    }
    if range.from == 1 {
        return Ok(String::new());
    }
    let before: Vec<&HeadingMatch> = entries
        .iter()
        .filter(|entry| entry.to < range.from)
        .collect();
    let mut next = 1;
    for entry in &before {
        if entry.from != next || entry.to < entry.from {
            return Err(fail(GENERATION_PLOT_OUTLINE_PREFIX_INVALID));
        }
        next = entry.to + 1;
    }
    if next != range.from || before.is_empty() {
        return Err(fail(GENERATION_PLOT_OUTLINE_PREFIX_INVALID));
    }
    if let Some(following) = entries.iter().find(|entry| entry.from >= range.from) {
        return Ok(expected.synopsis[before[0].index..following.index]
            .trim()
            .to_string());
    }
    let title = format!(
        "# {}\n\n",
        if expected.writing_language == "en-US" {
            "Plot Outline"
        } else {
            "情节大纲"
        }
    );
    let suffix =
        render_plot_outline_synopsis("", range.from - 1, expected)[title.len()..].to_string();
    if !expected.synopsis.starts_with(&title)
        || suffix.is_empty()
        || !expected.synopsis.ends_with(&suffix)
    {
        return Err(fail(GENERATION_PLOT_OUTLINE_PREFIX_INVALID));
    }
    let prefix = expected.synopsis[title.len()..expected.synopsis.len() - suffix.len()]
        .trim()
        .to_string();
    if prefix.is_empty()
        || render_plot_outline_synopsis(&prefix, range.from - 1, expected) != expected.synopsis
    {
        return Err(fail(GENERATION_PLOT_OUTLINE_PREFIX_INVALID));
    }
    Ok(prefix)
}

// ============================================================================
// 基线 review-revision-generation 纯函数
// （src/shared/review-revision-generation.ts + human-confirmed-review.ts；
// Tauri 侧此前无定义，按基线逐字移植为私有辅助）
// ============================================================================

/// 基线 `renderHumanConfirmedReviewBrief`（`src/shared/human-confirmed-review.ts:226`）。
fn render_human_confirmed_review_brief(
    snapshot: &HumanConfirmedReviewSnapshot,
    writing_language: &str,
) -> String {
    let text = |zh: &str, en: &str| writing_language_text(writing_language, zh, en);
    let applied_items: Vec<&HumanConfirmedReviewItem> = snapshot
        .items
        .iter()
        .filter(|item| item.decision == "apply")
        .collect();
    let mut sections: Vec<String> = Vec::new();

    if applied_items.iter().any(|item| item.origin == "ai") {
        sections.push(text(
            "【AI 意见边界】选择 AI 意见只授权处理问题，不把建议中的替换事实变成作者事实。先核对作者材料、前驱和正文；错误或无依据的改法不得照抄。作者亲写要求另行标注。",
            "[AI suggestion boundary] Selecting an AI finding authorizes addressing the issue; proposed replacement facts do not become author facts. Check author material, predecessor and manuscript first; do not copy an incorrect or unsupported remedy. Author-written requests are labeled separately.",
        ));
    }

    if applied_items.iter().any(|item| item.severity == "unknown") {
        sections.push(text(
            "【待核实项处理边界】作者纳入待核实项不等于确认其为错误。不得据此编造缺失前史或事实；仅按作者已有指导及正文证据处理，证据不足时保留不确定性。",
            "[Unverified item boundary] Including an unverified item does not confirm an error. Do not invent missing history or facts; follow existing author guidance and draft evidence, retaining uncertainty when evidence is insufficient.",
        ));
    }

    if !applied_items.is_empty() {
        let mut lines = vec![text(
            "【已确认纳入本次修稿的审稿项】",
            "[Confirmed review items included in this revision]",
        )];
        for (index, item) in applied_items.iter().enumerate() {
            let quote = item
                .quote
                .as_deref()
                .map(|quote| quote.trim())
                .filter(|quote| !quote.is_empty())
                .map(|quote| {
                    text(
                        &format!("\n  相关原文：{quote}"),
                        &format!("\n  Source excerpt: {quote}"),
                    )
                })
                .unwrap_or_default();
            let origin = text(
                if item.origin == "author" {
                    "作者亲写"
                } else {
                    "AI 意见"
                },
                if item.origin == "author" {
                    "Author-written"
                } else {
                    "AI suggestion"
                },
            );
            lines.push(format!(
                "{}. [{} / {}] [{origin}] {}{}",
                index + 1,
                item.category,
                item.severity,
                item.description,
                quote
            ));
        }
        sections.push(lines.join("\n"));
    }

    let author_guidance = snapshot.author_guidance.trim();
    if !author_guidance.is_empty() {
        sections.push(text(
            &format!("【作者补充修稿指导】\n{author_guidance}"),
            &format!("[Confirmed author guidance]\n{author_guidance}"),
        ));
    }

    sections.join("\n\n")
}

/// 基线 `reviewRevisionAiBrief`（`src/shared/review-revision-generation.ts:82`）。
fn review_revision_ai_brief(context: &ReviewRevisionContext) -> String {
    let Some(snapshot) = context
        .confirmation
        .as_ref()
        .map(|confirmation| &confirmation.snapshot)
    else {
        return String::new();
    };
    let mut brief_snapshot = snapshot.clone();
    brief_snapshot.author_guidance = String::new();
    brief_snapshot.items.retain(|item| item.origin == "ai");
    render_human_confirmed_review_brief(&brief_snapshot, &context.writing_language)
}

/// 基线 `reviewRevisionAuthorMaterial`
/// （`src/shared/review-revision-generation.ts:89`）。
fn review_revision_author_material(context: &ReviewRevisionContext) -> String {
    let text = |zh: &str, en: &str| writing_language_text(&context.writing_language, zh, en);
    let snapshot = context
        .confirmation
        .as_ref()
        .map(|confirmation| &confirmation.snapshot);
    // 各来源标签保留，但仅在最长来源处输出相同文本/前缀；
    // 冻结原文保持不变，供恢复与来源校验。
    let mut sources: Vec<(&str, Option<&str>)> = vec![
        (
            "config.worldSetting",
            Some(context.config.world_setting.as_str()),
        ),
        (
            "config.coreOutline",
            Some(context.config.core_outline.as_str()),
        ),
        ("worldbuilding", Some(context.worldbuilding.as_str())),
    ];
    sources.sort_by(|left, right| {
        (right.1.map(|value| value.len()).unwrap_or(0))
            .cmp(&(left.1.map(|value| value.len()).unwrap_or(0)))
    });
    let author_text = |source: &str, value: Option<&str>| -> Option<String> {
        let original = value.and_then(|value| {
            sources.iter().find(|(_, candidate)| {
                candidate.is_some_and(|candidate| candidate.starts_with(value))
            })
        });
        match original {
            Some((label, _)) if label != &source => Some(text(
                &format!("（本来源原文与 {label} 的完整前缀相同，见该来源原文。）"),
                &format!(
                    "(This source exactly matches a complete prefix of {label}; see that original text.)"
                ),
            )),
            _ => value.map(|value| value.to_string()),
        }
    };
    let mut parts: Vec<String> = Vec::new();
    parts.push(text(
        "【作者确认项目配置｜约束而非已发生事实】",
        "[Author-confirmed project configuration | constraints, not established history]",
    ));
    // 基线 `JSON.stringify({ ...config, ...(refine-draft ? { writingStyle: undefined } : {}),
    // globalGuidance: undefined, coreOutline: ..., worldSetting: ... }, null, 2)`。
    let mut config_value = serde_json::to_value(&context.config).unwrap_or_default();
    if let Value::Object(record) = &mut config_value {
        record.remove("globalGuidance");
        if matches!(context.operation, ReviewRevisionOperation::RefineDraft) {
            record.remove("writingStyle");
        }
        match author_text(
            "config.coreOutline",
            Some(context.config.core_outline.as_str()),
        ) {
            Some(core_outline) => {
                record.insert("coreOutline".to_string(), Value::String(core_outline));
            }
            None => {
                record.remove("coreOutline");
            }
        }
        match author_text(
            "config.worldSetting",
            Some(context.config.world_setting.as_str()),
        ) {
            Some(world_setting) => {
                record.insert("worldSetting".to_string(), Value::String(world_setting));
            }
            None => {
                record.remove("worldSetting");
            }
        }
    }
    parts.push(serde_json::to_string_pretty(&config_value).unwrap_or_default());
    parts.push(text(
        "【作者全局创作指导｜约束而非已发生事实】",
        "[Author global creative guidance | constraints, not established history]",
    ));
    parts.push(context.config.global_guidance.clone());
    parts.push(text("【世界观设定】", "[Worldbuilding]"));
    parts.push(
        author_text("worldbuilding", Some(context.worldbuilding.as_str())).unwrap_or_default(),
    );
    parts.push(text(
        "【作者角色状态｜以标注时点为准】",
        "[Author character state | scoped to its annotated time]",
    ));
    parts.push(context.character_states.clone());
    parts.push(text(
        "【本章作者指导｜约束】",
        "[Current chapter author guidance | constraints]",
    ));
    let chapter_blueprints: Vec<Value> = context
        .blueprints
        .iter()
        .filter(|blueprint| blueprint.chapter_number == context.source.chapter_number)
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_default();
    parts.push(serde_json::to_string_pretty(&Value::Array(chapter_blueprints)).unwrap_or_default());
    parts.push(text(
        "【后续蓝图/计划｜非既定历史】",
        "[Future blueprints/plans | not established history]",
    ));
    let future_blueprints: Vec<Value> = context
        .blueprints
        .iter()
        .filter(|blueprint| blueprint.chapter_number != context.source.chapter_number)
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_default();
    parts.push(serde_json::to_string_pretty(&Value::Array(future_blueprints)).unwrap_or_default());
    parts.push(text(
        "【本章冻结目标｜须由正文证明】",
        "[Frozen chapter goals | require manuscript evidence]",
    ));
    parts.push(stringify(&context.frozen_goals));
    for input in &context.author_inputs {
        parts.push(input.text.clone());
    }
    if let Some(snapshot) = snapshot {
        let mut author_snapshot = snapshot.clone();
        author_snapshot.items.retain(|item| item.origin == "author");
        parts.push(render_human_confirmed_review_brief(
            &author_snapshot,
            &context.writing_language,
        ));
    }
    // 基线 `.filter(Boolean).join('\n\n')`：空串条目剔除。
    parts.retain(|part| !part.is_empty());
    parts.join("\n\n")
}

// ============================================================================
// 辅助类型：来源收集器 / 资产观察器 / 事务事实
// ============================================================================

/// 基线 `materials` / `contextSources` 双数组与 `add` 闭包的等价物。
struct SourceCollector {
    project_id: String,
    epoch: String,
    materials: Vec<GenerationSourceMaterial>,
    context_sources: Vec<ContextSource>,
}

impl SourceCollector {
    fn new(project_id: &str, epoch: &str) -> Self {
        Self {
            project_id: project_id.to_string(),
            epoch: epoch.to_string(),
            materials: Vec::new(),
            context_sources: Vec::new(),
        }
    }

    /// 基线 `add(sourceId, revision, text, slot, reason)`。
    fn add(
        &mut self,
        source_id: &str,
        revision: i64,
        text: &str,
        slot: ContextSourceSlot,
        reason: &str,
    ) {
        let source_ref = SourceRef {
            project_id: self.project_id.clone(),
            epoch: self.epoch.clone(),
            source_id: source_id.to_string(),
            revision,
            content_hash: hash(text),
            span: None,
        };
        self.materials.push(GenerationSourceMaterial {
            source_ref: source_ref.clone(),
            text: text.to_string(),
        });
        self.context_sources.push(ContextSource {
            source_ref,
            slot,
            reason: reason.to_string(),
        });
    }

    /// 基线 `contextSources.push({ ref, slot, reason })`（已构造的 ref）。
    fn add_source(&mut self, source_ref: SourceRef, slot: ContextSourceSlot, reason: &str) {
        self.context_sources.push(ContextSource {
            source_ref,
            slot,
            reason: reason.to_string(),
        });
    }
}

/// 基线 `observed` 数组与 `observe` 闭包的等价物（TOCTOU 复检）。
#[derive(Default)]
struct AssetObserver {
    observed: Vec<ObservedFile>,
}

#[derive(Clone)]
struct ObservedFile {
    root: PathBuf,
    relative: String,
    bytes: Option<Vec<u8>>,
}

impl AssetObserver {
    fn observe(
        &mut self,
        root: &Path,
        relative: &str,
        optional: bool,
    ) -> Result<Option<Vec<u8>>, String> {
        let bytes = read_file(root, relative, optional)?;
        self.observed.push(ObservedFile {
            root: root.to_path_buf(),
            relative: relative.to_string(),
            bytes: bytes.clone(),
        });
        Ok(bytes)
    }
}

/// 基线 `assets` 条目（`{ scope, identity, hash }`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetEntry {
    scope: String,
    identity: String,
    hash: String,
}

/// 基线 `skills` 条目（`{ stage, identity, hash }`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillEntry {
    stage: String,
    identity: String,
    hash: String,
}

/// 基线事务闭包返回值 `{ core, brief, language }`。
struct Facts {
    core: Map<String, Value>,
    brief: Value,
    language: WritingLanguage,
}

/// 基线 `SELECT d.*,c.body` 的草稿行投影。
struct DraftRow {
    /// 基线 `d.*` 投影形状保留：绑定流以循环变量 `id` 做收据
    /// 比对（`latest !== id`），行内 `id` 在基线流中亦不读取。
    #[allow(dead_code)]
    id: i64,
    chapter_number: i64,
    version: i64,
    status: String,
    body: String,
}

/// 基线 `SELECT * FROM finalization_outbox` 的定稿回执行投影。
struct FinalizationReceiptRow {
    finalization_id: String,
    chapter_number: i64,
    content_hash: String,
    content_snapshot: String,
    content_revision: i64,
}

// ============================================================================
// SQLite 读取辅助
// ============================================================================

/// `PRAGMA data_version`（基线 `db.pragma('data_version', { simple: true })`）。
fn pragma_data_version(conn: &Connection) -> Result<i64, String> {
    conn.query_row("PRAGMA data_version", [], |row| row.get(0))
        .map_err(|error| error.to_string())
}

/// `SELECT total_changes()`（基线 `db.prepare('SELECT total_changes()').pluck().get()`）。
fn total_changes(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT total_changes()", [], |row| row.get(0))
        .map_err(|error| error.to_string())
}

/// 动态行 → `serde_json::Map`（键为 SQLite 原始列名，与基线 `SELECT *`
/// 的行对象一致；对齐 current_character_projection 先例）。
fn row_to_map(row: &Row<'_>, columns: &[String]) -> Result<Map<String, Value>, String> {
    let mut record = Map::new();
    for (index, column) in columns.iter().enumerate() {
        let value = row.get_ref(index).map_err(|error| error.to_string())?;
        record.insert(column.clone(), value_to_json(value)?);
    }
    Ok(record)
}

fn value_to_json(value: rusqlite::types::ValueRef<'_>) -> Result<Value, String> {
    use rusqlite::types::ValueRef;
    match value {
        ValueRef::Null => Ok(Value::Null),
        ValueRef::Integer(number) => Ok(Value::Number(number.into())),
        ValueRef::Real(number) => serde_json::Number::from_f64(number)
            .map(Value::Number)
            .ok_or_else(|| "无效的浮点数值".to_string()),
        ValueRef::Text(text) => Ok(Value::String(String::from_utf8_lossy(text).into_owned())),
        ValueRef::Blob(blob) => Ok(Value::Array(
            blob.iter()
                .map(|byte| Value::Number((*byte as i64).into()))
                .collect(),
        )),
    }
}

/// 基线 `db.prepare(sql).get()`（`SELECT *` 单行 → `Map`）。
fn query_row_map<P: rusqlite::Params>(
    conn: &Connection,
    sql: &str,
    params: P,
) -> Result<Option<Map<String, Value>>, String> {
    let mut statement = conn.prepare(sql).map_err(|error| error.to_string())?;
    let columns: Vec<String> = statement
        .column_names()
        .iter()
        .map(|name| name.to_string())
        .collect();
    let mut rows = statement.query(params).map_err(|error| error.to_string())?;
    match rows.next().map_err(|error| error.to_string())? {
        Some(row) => Ok(Some(row_to_map(row, &columns)?)),
        None => Ok(None),
    }
}

/// 基线 `db.prepare(sql).all()`（`SELECT *` 多行 → `Map` 数组）。
fn query_rows_map<P: rusqlite::Params>(
    conn: &Connection,
    sql: &str,
    params: P,
) -> Result<Vec<Map<String, Value>>, String> {
    let mut statement = conn.prepare(sql).map_err(|error| error.to_string())?;
    let columns: Vec<String> = statement
        .column_names()
        .iter()
        .map(|name| name.to_string())
        .collect();
    let mut rows = statement.query(params).map_err(|error| error.to_string())?;
    let mut records = Vec::new();
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        records.push(row_to_map(row, &columns)?);
    }
    Ok(records)
}

// ============================================================================
// 公开入口
// ============================================================================

/// 基线 `buildGenerationSourceBinding`：冻结生成源绑定（运行指纹 +
/// 上下文快照 + 材料清单）。
pub fn build_generation_source_binding(
    deps: &GenerationSourceBindingDependencies,
    input: &GenerationSourceBindingInput,
) -> Result<GenerationSourceBindingResult, String> {
    if input.project_id.is_empty() || input.epoch.is_empty() || input.operation.is_empty() {
        return Err(fail(GENERATION_SOURCE_BINDING_INVALID));
    }
    if input.prompt_keys.is_empty() {
        return Err(fail(GENERATION_PROMPT_SELECTION_REQUIRED));
    }
    let agent_input: Option<AgentGenerationInput> = input
        .agent_input
        .as_ref()
        .map(validate_agent_generation_input)
        .transpose()?;
    let editor_inline_input = input
        .editor_inline_input
        .as_ref()
        .map(validate_editor_inline_input)
        .transpose()?;
    let graph_input: Option<GraphGenerationInput> = input
        .graph_generation_input
        .as_ref()
        .map(validate_graph_generation_input)
        .transpose()?;
    if input.legacy_roster_key.is_some()
        && (input.operation != "legacy-character-roster-repair"
            || input.prompt_keys.as_slice() != ["legacy-roster"]
            || !input.skill_stages.is_empty()
            || !input.selected_draft_ids.is_empty()
            || !input.selected_finalized_draft_ids.is_empty()
            || input.chapter_number.is_some()
            || input
                .author_inputs
                .as_ref()
                .is_some_and(|items| !items.is_empty())
            || graph_input.is_some())
    {
        return Err(fail(GENERATION_LEGACY_SELECTION_INVALID));
    }
    if let Some(graph_input) = &graph_input {
        let expected_operation = match graph_input {
            GraphGenerationInput::Plot => "plot-tree-snapshot",
            GraphGenerationInput::Plan { .. } => "narrative-thread-plan-candidate",
            GraphGenerationInput::Event { .. } => "narrative-thread-event-candidate",
        };
        if input.operation != expected_operation
            || input
                .graph_generation_key
                .as_deref()
                .map(str::is_empty)
                .unwrap_or(true)
            || input.prompt_keys != vec![format!("graph-{}", graph_input.kind().as_str())]
            || !input.skill_stages.is_empty()
            || !input.selected_draft_ids.is_empty()
            || !input.selected_finalized_draft_ids.is_empty()
            || input.chapter_number.is_some()
            || input
                .author_inputs
                .as_ref()
                .is_some_and(|items| !items.is_empty())
        {
            return Err(fail(GENERATION_GRAPH_SELECTION_INVALID));
        }
    }
    // 基线 `freezeAuthorInputs(input.authorInputs)`：输入已类型化，
    // 序列化回 JSON 后走同一冻结校验（键集闭合 / ID 形态 / 唯一性 / 8MiB 上限）。
    let author_inputs_value = input
        .author_inputs
        .as_ref()
        .map(|items| serde_json::to_value(items).unwrap_or_default());
    let author_inputs = freeze_author_inputs(author_inputs_value.as_ref())?;
    let blueprint_chapters: Vec<i64> = input
        .selected_blueprint_chapter_numbers
        .clone()
        .unwrap_or_default();
    if blueprint_chapters.len() > 10_000
        || {
            let unique: HashSet<i64> = blueprint_chapters.iter().copied().collect();
            unique.len() != blueprint_chapters.len()
        }
        || blueprint_chapters
            .iter()
            .any(|chapter| !is_safe_integer(*chapter) || *chapter < 1)
    {
        return Err(fail(GENERATION_BLUEPRINT_SELECTION_INVALID));
    }
    if input
        .chapter_number
        .is_some_and(|chapter_number| !is_safe_integer(chapter_number) || chapter_number < 1)
    {
        return Err(fail(GENERATION_CHAPTER_INVALID));
    }
    for ids in [
        &input.selected_draft_ids,
        &input.selected_finalized_draft_ids,
    ] {
        let unique: HashSet<i64> = ids.iter().copied().collect();
        if unique.len() != ids.len() || ids.iter().any(|id| !is_safe_integer(*id) || *id < 1) {
            return Err(fail(GENERATION_SELECTION_INVALID));
        }
    }
    let merged: HashSet<i64> = input
        .selected_draft_ids
        .iter()
        .chain(input.selected_finalized_draft_ids.iter())
        .copied()
        .collect();
    if merged.len() != input.selected_draft_ids.len() + input.selected_finalized_draft_ids.len() {
        return Err(fail(GENERATION_SELECTION_AMBIGUOUS));
    }
    if input
        .prompt_keys
        .iter()
        .any(|key| !PROMPT_KEY.is_match(key))
        || input
            .skill_stages
            .iter()
            .any(|stage| !WRITING_SKILL_STAGES.contains(&stage.as_str()))
    {
        return Err(fail(GENERATION_ASSET_ID_INVALID));
    }
    let data_version = pragma_data_version(deps.db)?;
    let changes = total_changes(deps.db)?;
    let model_receipt =
        model_projection(&serde_json::to_value(&input.model_receipt).unwrap_or_default())?;
    let material_decision: Option<MaterialDecisionReceipt> = input
        .material_decision
        .as_ref()
        .map(freeze_material_decision)
        .transpose()?;
    if material_decision.is_some()
        && ![
            "chapter-draft",
            "review-chapter",
            "refine-draft",
            "refine-from-review",
        ]
        .contains(&input.operation.as_str())
    {
        return Err(fail(GENERATION_MATERIAL_DECISION_INVALID));
    }
    if let Some(decision) = &material_decision {
        if input.operation == "chapter-draft" {
            if !decision
                .included
                .iter()
                .any(|item| item.source_id == "author:required")
                || decision.included.iter().any(|item| {
                    item.source_id.starts_with("candidate:")
                        && !input.selected_draft_ids.contains(
                            &item.source_id["candidate:".len()..]
                                .parse::<i64>()
                                .unwrap_or_default(),
                        )
                })
                || decision.included.iter().any(|item| {
                    item.source_id.starts_with("finalized:")
                        && !input.selected_finalized_draft_ids.contains(
                            &item.source_id["finalized:".len()..]
                                .parse::<i64>()
                                .unwrap_or_default(),
                        )
                })
            {
                return Err(fail(GENERATION_MATERIAL_DECISION_INVALID));
            }
        }
        if ["review-chapter", "refine-draft", "refine-from-review"]
            .contains(&input.operation.as_str())
        {
            let context = input.review_revision_context.as_ref();
            let review_source_id = context
                .and_then(|context| context.confirmation.as_ref())
                .map(|confirmation| confirmation.review_source_id);
            // 历史确认曾有一个 author 类别收据。保留该冻结合同。
            let legacy_confirmation = input.operation == "refine-from-review"
                && review_source_id.is_some_and(|id| is_safe_integer(id) && id > 0)
                && decision.included.len() == 1
                && decision.omitted.is_empty()
                && decision.included[0].source_id
                    == format!("review:confirmed:{}", review_source_id.unwrap_or_default())
                && decision.included[0].revision == review_source_id.unwrap_or_default()
                && decision.included[0].category == "author";
            let admitted: HashSet<String> = context
                .map(|context| {
                    context
                        .history
                        .iter()
                        .chain(context.predecessor.iter())
                        .filter_map(|item| item.identity.as_ref())
                        .map(|identity: &crate::services::review_revision_context::ReviewMaterialIdentity| {
                            decision_identity(
                                &identity.source_id,
                                identity.revision,
                                &identity.content_hash,
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let invalid = !legacy_confirmation
                && decision
                    .included
                    .iter()
                    .map(|item| {
                        (
                            item.source_id.as_str(),
                            item.revision,
                            item.content_hash.as_str(),
                            item.category.as_str(),
                        )
                    })
                    .chain(decision.omitted.iter().map(|item| {
                        (
                            item.source_id.as_str(),
                            item.revision,
                            item.content_hash.as_str(),
                            item.category.as_str(),
                        )
                    }))
                    .any(|(source_id, revision, content_hash, category)| {
                        if source_id == "author:required" {
                            context.is_none()
                                || content_hash
                                    != hash(&review_revision_author_material(
                                        context.expect("author:required 分支已守卫"),
                                    ))
                        } else if source_id.starts_with("review:confirmed:") {
                            input.operation != "refine-from-review"
                                || context
                                    .and_then(|context| context.confirmation.as_ref())
                                    .is_none()
                                || source_id
                                    != format!(
                                        "review:confirmed:{}",
                                        review_source_id.unwrap_or_default()
                                    )
                                || category != "future-plan"
                                || content_hash
                                    != hash(&review_revision_ai_brief(
                                        context.expect("review:confirmed 分支已守卫"),
                                    ))
                        } else {
                            !admitted.contains(&decision_identity(
                                source_id,
                                revision,
                                content_hash,
                            ))
                        }
                    });
            if invalid {
                return Err(fail(GENERATION_MATERIAL_DECISION_INVALID));
            }
        }
    }
    let material_decision_hash = material_decision
        .as_ref()
        .map(|decision| hash(&stable(&serde_json::to_value(decision).unwrap_or_default())));
    let project_storage_root = deps.project_storage_root.to_string_lossy().into_owned();
    let transfer_authority = read_portable_current_authority(&ReadPortableCurrentAuthorityInput {
        database: deps.db,
        project_storage_root: &project_storage_root,
        project_id: &input.project_id,
        test_hooks: None,
    })
    .map_err(|error| error.code().to_string())?;
    safe_metadata(&input.policy)?;
    safe_metadata(&input.output_contract)?;
    let mut collector = SourceCollector::new(&input.project_id, &input.epoch);
    for item in &author_inputs {
        collector.add(
            &format!("author-action:{}", item.id),
            0,
            &item.text,
            ContextSourceSlot::AuthorConstraint,
            "explicit immutable author input for this action; not a database or file identity",
        );
    }
    // SQLite 供给一次读事务；文件系统选择在下文读取两次。
    let mut graph_context: Option<GraphGenerationContext> = None;
    let mut legacy_context: Option<LegacyRosterGenerationContext> = None;
    let facts = {
        let mut read_facts = |connection: &Connection| {
            build_facts_in_transaction(
                connection,
                deps,
                input,
                agent_input.is_some(),
                &author_inputs,
                &blueprint_chapters,
                &transfer_authority,
                &mut collector,
                &mut graph_context,
                &mut legacy_context,
            )
        };
        if deps.db.is_autocommit() {
            let transaction = deps
                .db
                .unchecked_transaction()
                .map_err(|error| error.to_string())?;
            let facts = read_facts(&transaction)?;
            transaction.commit().map_err(|error| error.to_string())?;
            facts
        } else {
            // 子代效应已持有 immediate 事务：共享句柄，不嵌套 BEGIN，也不提交调用方事务。
            read_facts(deps.db)?
        }
    };
    let mut assets: Vec<AssetEntry> = Vec::new();
    let mut observer = AssetObserver::default();
    let selected_guidance: Vec<String> = if input.operation == "chapter-draft" {
        guidance_names(deps.project_storage_root)?
    } else {
        Vec::new()
    };
    for name in &selected_guidance {
        let Some(bytes) =
            observer.observe(deps.project_storage_root, &format!("prompts/{name}"), false)?
        else {
            return Err(fail(GENERATION_ASSET_UNSAFE));
        };
        assets.push(AssetEntry {
            scope: "project-guidance".to_string(),
            identity: name.clone(),
            hash: hash_bytes(&bytes),
        });
        collector.add(
            &format!("project-guidance:{name}"),
            0,
            &String::from_utf8_lossy(&bytes),
            ContextSourceSlot::AuthorConstraint,
            "project Markdown guidance consumed by drafting",
        );
    }
    let mut agent_prompt: Option<(String, String)> = None;
    let mut import_prompts: Map<String, Value> = Map::new();
    let language_name = writing_language_name(facts.language);
    for key in &input.prompt_keys {
        if let Some(legacy_context) = &legacy_context {
            assets.push(AssetEntry {
                scope: "builtin".to_string(),
                identity: format!("prompt:{key}:{language_name}"),
                hash: hash(&stringify(&legacy_roster_generation_task(legacy_context))),
            });
            continue;
        }
        if let Some(graph_context) = &graph_context {
            let task = graph_generation_task(graph_context)?;
            assets.push(AssetEntry {
                scope: "builtin".to_string(),
                identity: format!("prompt:{key}:{language_name}"),
                hash: hash(&stringify(&task)),
            });
            continue;
        }
        let builtin = (deps.read_builtin_prompt)(key, facts.language)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| fail(GENERATION_BUILTIN_PROMPT_MISSING))?;
        assets.push(AssetEntry {
            scope: "builtin".to_string(),
            identity: format!("prompt:{key}:{language_name}"),
            hash: hash(&builtin),
        });
        let mut selected: Option<Vec<u8>> = None;
        let mut scope = String::new();
        for (candidate_scope, root) in [
            ("project", deps.project_storage_root),
            ("global", deps.global_data_root),
        ] {
            let mut relatives = vec![format!("prompts/{key}.{language_name}.json")];
            if language_name == "zh-CN" {
                relatives.push(format!("prompts/{key}.json"));
            }
            for relative in &relatives {
                let bytes = observer.observe(root, relative, true)?;
                if selected.is_none() && bytes.is_some() {
                    let bytes = bytes.expect("bytes 已确认非空");
                    let parsed: Value = serde_json::from_slice(&bytes)
                        .map_err(|_| fail(GENERATION_PROMPT_INVALID))?;
                    if parsed.get("key").and_then(Value::as_str) != Some(key.as_str())
                        || parsed
                            .get("writingLanguage")
                            .and_then(Value::as_str)
                            .unwrap_or("zh-CN")
                            != language_name
                    {
                        return Err(fail(GENERATION_PROMPT_INVALID));
                    }
                    selected = Some(bytes);
                    scope = format!("{candidate_scope}:{relative}");
                }
            }
            if selected.is_some() {
                break;
            }
        }
        if let Some(selected) = &selected {
            assets.push(AssetEntry {
                scope: scope.clone(),
                identity: format!("prompt:{key}:{language_name}"),
                hash: hash_bytes(selected),
            });
        }
        if agent_input.is_some() && key == "assistant_writing_identity" {
            agent_prompt = Some((
                selected
                    .as_ref()
                    .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                    .unwrap_or_else(|| builtin.clone()),
                builtin.clone(),
            ));
        }
        if input.import_slot.is_some()
            || editor_inline_input.is_some()
            || input.finalization_generation_slot.is_some()
        {
            let text = selected
                .as_ref()
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                .unwrap_or_else(|| builtin.clone());
            let parsed: Value =
                serde_json::from_str(&text).map_err(|_| fail(GENERATION_PROMPT_INVALID))?;
            import_prompts.insert(key.clone(), parsed);
        }
    }
    let bindings_bytes =
        observer.observe(deps.project_storage_root, "writing-skills.json", true)?;
    let bindings: Value = bindings_bytes
        .as_ref()
        .map(|bytes| {
            serde_json::from_slice(bytes).map_err(|_| fail(GENERATION_SKILL_BINDINGS_INVALID))
        })
        .transpose()?
        .unwrap_or_else(|| {
            let mut bindings = Map::new();
            bindings.insert("version".to_string(), Value::Number(1.into()));
            bindings.insert("bindings".to_string(), Value::Object(Map::new()));
            Value::Object(bindings)
        });
    if bindings.get("version") != Some(&Value::Number(1.into()))
        || bindings
            .get("bindings")
            .map(|bindings| !bindings.is_object())
            .unwrap_or(true)
    {
        return Err(fail(GENERATION_SKILL_BINDINGS_INVALID));
    }
    let mut skills: Vec<SkillEntry> = Vec::new();
    for stage in &input.skill_stages {
        let bound = bindings
            .get("bindings")
            .and_then(|bindings| bindings.get(stage.as_str()));
        let Some(bound) = bound else {
            skills.push(SkillEntry {
                stage: stage.clone(),
                identity: "unbound".to_string(),
                hash: hash(""),
            });
            continue;
        };
        let Some(id) = bound.as_str() else {
            return Err(fail(GENERATION_SKILL_ID_INVALID));
        };
        if !SKILL_ID.is_match(id) {
            return Err(fail(GENERATION_SKILL_ID_INVALID));
        }
        let mut parts = id.split(':');
        let scope = parts.next().unwrap_or_default();
        let name = parts.next().unwrap_or_default();
        if name == "." || name == ".." {
            return Err(fail(GENERATION_SKILL_ID_INVALID));
        }
        let text = if scope == "builtin" {
            (deps.read_builtin_skill).and_then(|reader| reader(name, facts.language))
        } else {
            let root = if scope == "project" {
                deps.project_storage_root
            } else {
                deps.global_data_root
            };
            let Some(bytes) = observer.observe(root, &format!("skills/{name}/SKILL.md"), false)?
            else {
                return Err(fail(GENERATION_ASSET_UNSAFE));
            };
            Some(String::from_utf8_lossy(&bytes).into_owned())
        };
        let Some(text) = text else {
            return Err(fail(GENERATION_SKILL_MISSING));
        };
        let inspected = inspect_writing_skill_markdown(&text)?;
        // 内置技能是既有产品能力，经 trusted pure getter 选择；
        // 外部导入保留更严格的门禁。
        if scope != "builtin"
            && (!inspected.compatible
                || ![inspected.metadata.language.as_str(), "bilingual"].contains(&language_name))
        {
            return Err(fail(GENERATION_SKILL_INCOMPATIBLE));
        }
        skills.push(SkillEntry {
            stage: stage.clone(),
            identity: id.to_string(),
            hash: hash(&text),
        });
    }
    for file in &observer.observed {
        let now = read_file(&file.root, &file.relative, true)?;
        if now != file.bytes {
            return Err(fail(GENERATION_ASSET_CHANGED_DURING_SNAPSHOT));
        }
    }
    if input.operation == "chapter-draft"
        && selected_guidance != guidance_names(deps.project_storage_root)?
    {
        return Err(fail(GENERATION_ASSET_CHANGED_DURING_SNAPSHOT));
    }
    if let Some(knowledge_snapshot) = &input.knowledge_snapshot {
        collector.add(
            "knowledge-selection",
            0,
            &stable(&serde_json::to_value(knowledge_snapshot).unwrap_or_default()),
            ContextSourceSlot::UnconfirmedContinuity,
            "main-verified immutable knowledge selection and source identities",
        );
        for item in &knowledge_snapshot.items {
            collector.add(
                &format!("kb:{}:{}", item.document_id, item.chunk_id),
                item.revision,
                &item.text,
                ContextSourceSlot::UnconfirmedContinuity,
                "verified original knowledge passage",
            );
        }
    }
    let outline_policy = plot_outline_policy(&json_operation_policy(input))?;
    let plot_outline_source =
        if outline_policy.is_some() {
            Some(input.plot_outline_source.clone().unwrap_or_else(
                || json!({ "version": 1, "core": Value::Object(facts.core.clone()) }),
            ))
        } else {
            None
        };
    if let (Some(source), Some(policy)) = (&plot_outline_source, &outline_policy) {
        let expected = plot_outline_expected(source)?;
        plot_outline_confirmed_prefix(&expected, &policy.range)?;
        collector.add(
            "plot-outline:origin",
            0,
            &stable(source),
            ContextSourceSlot::AuthorConstraint,
            "main captured original project facts and synopsis for final outline compare-and-set",
        );
    }
    let agent_context = match (&agent_input, &agent_prompt) {
        (Some(agent_input), Some(agent_prompt)) => Some(build_agent_generation_context(
            agent_input,
            &Value::Object(facts.core.clone()),
            facts.language,
            &agent_prompt.0,
            &agent_prompt.1,
        )?),
        _ => None,
    };
    let editor_inline_context = if let Some(editor_inline_input) = &input.editor_inline_input {
        let template_value = import_prompts
            .get("edit_selected_text")
            .ok_or_else(|| "edit_selected_text prompt template missing".to_string())?;
        let template: PromptTemplate = serde_json::from_value(template_value.clone())
            .map_err(|_| fail(GENERATION_PROMPT_INVALID))?;
        Some(build_editor_inline_context(
            editor_inline_input,
            facts.language,
            &template,
        )?)
    } else {
        None
    };
    let finalization_context = if let Some(slot) = &input.finalization_generation_slot {
        let template_key = if matches!(slot.step_key, FinalizationGenerationStep::ChapterNotes) {
            "generate_chapter_notes"
        } else {
            "update_character_cards"
        };
        let template_value = import_prompts
            .get(template_key)
            .ok_or_else(|| format!("finalization prompt template missing: {template_key}"))?;
        let template: PromptTemplate = serde_json::from_value(template_value.clone())
            .map_err(|_| fail(GENERATION_PROMPT_INVALID))?;
        Some(capture_finalization_generation_context(
            deps.db,
            slot,
            &ProjectEpoch {
                project_id: input.project_id.clone(),
                epoch: input.epoch.clone(),
            },
            facts.language,
            &template,
            input.finalization_generation_context.as_ref(),
        )?)
    } else {
        None
    };
    if let Some(context) = &finalization_context {
        collector.add(
            "finalization-generation-context",
            0,
            &stringify(context),
            ContextSourceSlot::FinalizedFact,
            "main captured finalization identity, original field baselines and selected prompt",
        );
    }
    if let Some(context) = &editor_inline_context {
        collector.add(
            "editor-inline:author-draft",
            0,
            &stringify(context),
            ContextSourceSlot::AuthorConstraint,
            "explicit unsaved author manuscript and UTF16 selection; not a canonical document",
        );
    }
    let mut import_context = if let Some(slot) = &input.import_slot {
        Some(capture_import_generation_context(deps.db, slot)?)
    } else {
        None
    };
    if let Some(context) = import_context.as_mut() {
        context.prompts = import_prompts.clone();
    }
    if let Some(context) = &import_context {
        collector.add(
            &format!(
                "import-manifest:{}",
                import_generation_slot_key(&context.slot)?
            ),
            0,
            &stable(&serde_json::to_value(context).unwrap_or_default()),
            ContextSourceSlot::UnconfirmedContinuity,
            "persisted reference import material for this exact checkpoint",
        );
    }
    if agent_input.is_some() && agent_context.is_none() {
        return Err(fail(GENERATION_AGENT_PROMPT_REQUIRED));
    }
    if let Some(context) = &agent_context {
        collector.add(
            "agent-context",
            0,
            &stringify(context),
            ContextSourceSlot::AuthorConstraint,
            "explicit conversation and tool definitions with main-read project facts and identity template",
        );
    }
    if pragma_data_version(deps.db)? != data_version || total_changes(deps.db)? != changes {
        return Err(fail(GENERATION_SOURCE_CHANGED_DURING_SNAPSHOT));
    }
    let mut source_manifest: Map<String, Value> = Map::new();
    source_manifest.insert("version".to_string(), Value::Number(1.into()));
    if let Some(context_hash) = &input.finalized_character_context_hash {
        source_manifest.insert(
            "finalizedCharacterContextHash".to_string(),
            Value::String(context_hash.clone()),
        );
    }
    source_manifest.insert(
        "operation".to_string(),
        Value::String(input.operation.clone()),
    );
    if let Some(snapshot) = &input.knowledge_snapshot {
        source_manifest.insert(
            "knowledgeSnapshot".to_string(),
            serde_json::to_value(snapshot).unwrap_or_default(),
        );
    }
    if let Some(batch_id) = &input.batch_id {
        source_manifest.insert("batchId".to_string(), Value::String(batch_id.clone()));
    }
    if let Some(batch_intent) = &input.batch_intent {
        source_manifest.insert("batchIntent".to_string(), batch_intent.clone());
    }
    if let Some(chapter_number) = input.chapter_number {
        source_manifest.insert(
            "chapterNumber".to_string(),
            Value::Number(chapter_number.into()),
        );
    }
    source_manifest.insert(
        "selectedDraftIds".to_string(),
        serde_json::to_value(&input.selected_draft_ids).unwrap_or_default(),
    );
    source_manifest.insert(
        "selectedFinalizedDraftIds".to_string(),
        serde_json::to_value(&input.selected_finalized_draft_ids).unwrap_or_default(),
    );
    if !blueprint_chapters.is_empty() {
        source_manifest.insert(
            "selectedBlueprintChapterNumbers".to_string(),
            serde_json::to_value(&blueprint_chapters).unwrap_or_default(),
        );
    }
    source_manifest.insert(
        "promptKeys".to_string(),
        serde_json::to_value(&input.prompt_keys).unwrap_or_default(),
    );
    source_manifest.insert(
        "skillStages".to_string(),
        serde_json::to_value(&input.skill_stages).unwrap_or_default(),
    );
    if !author_inputs.is_empty() {
        source_manifest.insert(
            "authorInputs".to_string(),
            serde_json::to_value(&author_inputs).unwrap_or_default(),
        );
    }
    if let Some(decision) = &material_decision {
        source_manifest.insert(
            "materialDecision".to_string(),
            serde_json::to_value(decision).unwrap_or_default(),
        );
        source_manifest.insert(
            "materialDecisionHash".to_string(),
            Value::String(
                material_decision_hash
                    .clone()
                    .expect("materialDecision 存在时哈希必存在"),
            ),
        );
    }
    source_manifest.insert("modelReceipt".to_string(), model_receipt.clone());
    source_manifest.insert("policy".to_string(), input.policy.clone());
    source_manifest.insert("outputContract".to_string(), input.output_contract.clone());
    if let Some(authority) = &transfer_authority {
        source_manifest.insert(
            "portableTransferAuthority".to_string(),
            serde_json::to_value(authority).unwrap_or_default(),
        );
    }
    if let Some(source) = &plot_outline_source {
        source_manifest.insert("plotOutlineSource".to_string(), source.clone());
    }
    if let Some(context) = &input.review_revision_context {
        source_manifest.insert(
            "reviewRevisionContext".to_string(),
            serde_json::to_value(context).unwrap_or_default(),
        );
        source_manifest.insert(
            "reviewRevisionContextHash".to_string(),
            Value::String(hash(&stringify(context))),
        );
    }
    if let Some(context) = &legacy_context {
        let task = legacy_roster_generation_task(context);
        source_manifest.insert(
            "legacyRosterKey".to_string(),
            Value::String(context.key.clone()),
        );
        source_manifest.insert(
            "legacyRosterContext".to_string(),
            serde_json::to_value(context).unwrap_or_default(),
        );
        source_manifest.insert(
            "legacyRosterContextHash".to_string(),
            Value::String(hash(&stringify(context))),
        );
        source_manifest.insert(
            "legacyRosterOriginEpoch".to_string(),
            Value::String(context.origin_epoch.clone()),
        );
        source_manifest.insert(
            "legacyRosterTask".to_string(),
            serde_json::to_value(&task).unwrap_or_default(),
        );
        source_manifest.insert(
            "legacyRosterTaskHash".to_string(),
            Value::String(hash(&stringify(&task))),
        );
    }
    if let Some(context) = &graph_context {
        let task: GraphGenerationTask = graph_generation_task(context)?;
        // 读面校验的是持久化JSON值；不能混用结构体字段序生成另一份摘要。
        let context_value = serde_json::to_value(context).unwrap_or_default();
        let context_hash = hash(&stringify(&context_value));
        let task_value = serde_json::to_value(&task).unwrap_or_default();
        let task_hash = hash(&stringify(&task_value));
        source_manifest.insert(
            "graphGenerationInput".to_string(),
            serde_json::to_value(context.input()).unwrap_or_default(),
        );
        source_manifest.insert(
            "graphGenerationKey".to_string(),
            Value::String(context.key().to_string()),
        );
        source_manifest.insert("graphGenerationContext".to_string(), context_value);
        source_manifest.insert(
            "graphGenerationContextHash".to_string(),
            Value::String(context_hash),
        );
        source_manifest.insert(
            "graphGenerationOriginEpoch".to_string(),
            Value::String(context.origin_epoch().to_string()),
        );
        source_manifest.insert("graphGenerationTask".to_string(), task_value);
        source_manifest.insert(
            "graphGenerationTaskHash".to_string(),
            Value::String(task_hash),
        );
    }
    if let Some(context) = &finalization_context {
        let task = finalization_generation_task(
            context,
            transfer_authority
                .as_ref()
                .map(|authority| authority.origin_project_id.as_str()),
        );
        source_manifest.insert(
            "finalizationGenerationSlot".to_string(),
            serde_json::to_value(&context.slot).unwrap_or_default(),
        );
        source_manifest.insert(
            "finalizationGenerationSlotKey".to_string(),
            Value::String(finalization_slot_key(&context.slot)?),
        );
        source_manifest.insert(
            "finalizationGenerationContext".to_string(),
            serde_json::to_value(context).unwrap_or_default(),
        );
        source_manifest.insert(
            "finalizationGenerationContextHash".to_string(),
            Value::String(hash(&stringify(context))),
        );
        source_manifest.insert(
            "finalizationGenerationOriginEpoch".to_string(),
            Value::String(context.identity.epoch.clone()),
        );
        source_manifest.insert(
            "finalizationGenerationTask".to_string(),
            serde_json::to_value(&task).unwrap_or_default(),
        );
        source_manifest.insert(
            "finalizationGenerationTaskHash".to_string(),
            Value::String(hash(&stringify(&task))),
        );
    }
    if let Some(context) = &agent_context {
        source_manifest.insert(
            "agentInput".to_string(),
            input.agent_input.clone().unwrap_or_default(),
        );
        source_manifest.insert(
            "agentContext".to_string(),
            serde_json::to_value(context).unwrap_or_default(),
        );
        source_manifest.insert(
            "agentContextHash".to_string(),
            Value::String(hash(&stringify(context))),
        );
        if let Some(session) = &input.agent_session {
            source_manifest.insert("agentSession".to_string(), session.clone());
        }
    }
    if let Some(registration_id) = &input.agent_workflow_registration_id {
        source_manifest.insert(
            "agentWorkflowRegistrationId".to_string(),
            Value::String(registration_id.clone()),
        );
    }
    if let Some(context) = &editor_inline_context {
        let task = editor_inline_task(context);
        source_manifest.insert(
            "editorInlineInput".to_string(),
            input.editor_inline_input.clone().unwrap_or_default(),
        );
        source_manifest.insert(
            "editorInlineOriginEpoch".to_string(),
            Value::String(
                input
                    .editor_inline_origin_epoch
                    .clone()
                    .unwrap_or_else(|| input.epoch.clone()),
            ),
        );
        source_manifest.insert(
            "editorInlineContext".to_string(),
            serde_json::to_value(context).unwrap_or_default(),
        );
        source_manifest.insert(
            "editorInlineContextHash".to_string(),
            Value::String(hash(&stringify(context))),
        );
        source_manifest.insert(
            "editorInlineTask".to_string(),
            serde_json::to_value(&task).unwrap_or_default(),
        );
        source_manifest.insert(
            "editorInlineTaskHash".to_string(),
            Value::String(hash(&stringify(&task))),
        );
    }
    if let Some(context) = &import_context {
        source_manifest.insert(
            "importSlot".to_string(),
            serde_json::to_value(&context.slot).unwrap_or_default(),
        );
        source_manifest.insert(
            "importSlotKey".to_string(),
            Value::String(import_generation_slot_key(&context.slot)?),
        );
        source_manifest.insert(
            "importContext".to_string(),
            serde_json::to_value(context).unwrap_or_default(),
        );
    }
    if let Some(authority) = &transfer_authority {
        let text = stable(&serde_json::to_value(authority).unwrap_or_default());
        collector.add_source(
            SourceRef {
                project_id: input.project_id.clone(),
                epoch: input.epoch.clone(),
                source_id: format!("portable-transfer:{}", authority.receipt_id),
                revision: 0,
                content_hash: hash(&text),
                span: None,
            },
            ContextSourceSlot::UnconfirmedContinuity,
            "restored portable receipt identity; not prose or execution authority",
        );
    }
    let context_sources_hash = hash(&stable(&Value::Array(
        collector
            .context_sources
            .iter()
            .map(|source| {
                let mut entry = Map::new();
                entry.insert(
                    "slot".to_string(),
                    serde_json::to_value(&source.slot).unwrap_or_default(),
                );
                entry.insert("reason".to_string(), Value::String(source.reason.clone()));
                entry.insert(
                    "ref".to_string(),
                    source_ref_without_epoch(&source.source_ref),
                );
                Value::Object(entry)
            })
            .collect(),
    )));
    // S10B 步骤 3：准入裁决折进冻结上下文哈希。没有裁决时逐字节保持
    // 原来的口径，避免改动无关操作的指纹；有裁决时同一份 source 集配上
    // 不同准入也必须得到不同的 contextSnapshotId，恢复无法悄悄换一份
    // 材料裁决继续。
    let context_hash = match &material_decision_hash {
        Some(decision_hash) => hash(&stable(&Value::Array(vec![
            Value::String(context_sources_hash.clone()),
            Value::String(decision_hash.clone()),
        ]))),
        None => context_sources_hash.clone(),
    };
    let context = ContextSnapshot {
        project_id: input.project_id.clone(),
        epoch: input.epoch.clone(),
        id: format!("context:{context_hash}"),
        hash: context_hash.clone(),
        transfer_authority: transfer_authority
            .as_ref()
            .map(|authority| TransferAuthority {
                receipt_id: authority.receipt_id.clone(),
                origin_project_id: authority.origin_project_id.clone(),
                snapshot_generation: authority.snapshot_generation.clone(),
            }),
        sources: collector.context_sources.clone(),
        omissions: Vec::new(),
        estimate: ContextEstimate {
            method_version: MATERIAL_DECISION_UNIT_METHOD_VERSION.to_string(),
            input_units: collector
                .materials
                .iter()
                .map(|material| material.text.len() as i64)
                .sum(),
        },
    };
    let dependency_refs: Vec<Value> = collector
        .materials
        .iter()
        .filter(|material| {
            material.source_ref.source_id.starts_with("draft:")
                || material.source_ref.source_id.starts_with("finalized:")
        })
        .map(|material| source_ref_without_epoch(&material.source_ref))
        .collect();
    // 基线 `authorInputs.length === 0 ? { ...facts.core }
    // : [{ ...facts.core }, authorInputs]` 的稳定序列化输入投影。
    let author_guidance = if author_inputs.is_empty() {
        Value::Object(facts.core.clone())
    } else {
        Value::Array(vec![
            Value::Object(facts.core.clone()),
            serde_json::to_value(&author_inputs).unwrap_or_default(),
        ])
    };
    let fingerprint = FrozenInputFingerprint {
        chapter_brief_hash: hash(&stable(&facts.brief)),
        author_guidance_hash: hash(&stable(&author_guidance)),
        dependency_hash: hash(&stable(&Value::Array(dependency_refs))),
        context_snapshot_hash: context_hash,
        template_hash: hash(&stable(&Value::Array(
            assets
                .iter()
                .map(|asset| serde_json::to_value(asset).unwrap_or_default())
                .collect(),
        ))),
        skill_snapshot_hash: hash(&stable(&Value::Array(
            skills
                .iter()
                .map(|skill| serde_json::to_value(skill).unwrap_or_default())
                .collect(),
        ))),
        model_lease_revision: hash(&stable(&model_receipt)),
        policy_hash: hash(&stable(&input.policy)),
        output_contract_hash: hash(&stable(&input.output_contract)),
    };
    let binding = RunBinding {
        project_id: input.project_id.clone(),
        epoch: input.epoch.clone(),
        fingerprint,
        context_snapshot_id: context.id.clone(),
        source_manifest,
        source_refs: collector
            .materials
            .iter()
            .map(|material| material.source_ref.clone())
            .collect(),
        extra: Map::new(),
    };
    Ok(GenerationSourceBindingResult {
        binding,
        context,
        materials: collector.materials,
    })
}

/// 基线 `plotOutlinePolicy({ operation: input.operation, policy: input.policy })`
/// 的入参对象。
fn json_operation_policy(input: &GenerationSourceBindingInput) -> Value {
    let mut manifest = Map::new();
    manifest.insert(
        "operation".to_string(),
        Value::String(input.operation.clone()),
    );
    manifest.insert("policy".to_string(), input.policy.clone());
    Value::Object(manifest)
}

/// 基线事务闭包：一次读事务内的项目事实冻结。
#[allow(clippy::too_many_arguments)]
fn build_facts_in_transaction(
    transaction: &Connection,
    deps: &GenerationSourceBindingDependencies,
    input: &GenerationSourceBindingInput,
    has_agent_input: bool,
    author_inputs: &[GenerationAuthorInput],
    blueprint_chapters: &[i64],
    transfer_authority: &Option<PortableCurrentAuthority>,
    collector: &mut SourceCollector,
    graph_context: &mut Option<GraphGenerationContext>,
    legacy_context: &mut Option<LegacyRosterGenerationContext>,
) -> Result<Facts, String> {
    let _ = author_inputs;
    let _ = deps;
    let raw = query_row_map(
        transaction,
        "SELECT * FROM project_core WHERE id='main'",
        [],
    )?;
    let Some(raw) = raw else {
        return Err(fail(GENERATION_PROJECT_CORE_MISSING));
    };
    let core = without(
        &raw,
        &[
            "created_at",
            "updated_at",
            "character_states",
            "plot_tree_snapshot",
        ],
    );
    let language = if core.get("writing_language").and_then(Value::as_str) == Some("en-US") {
        WritingLanguage::EnUs
    } else {
        WritingLanguage::ZhCn
    };
    collector.add(
        "project-core:main",
        0,
        &stable(&Value::Object(core.clone())),
        ContextSourceSlot::AuthorConstraint,
        "current project settings",
    );
    if let Some(legacy_roster_key) = &input.legacy_roster_key {
        *legacy_context = Some(capture_legacy_roster_generation_context(
            transaction,
            &LegacyRosterGenerationScope {
                project_id: input.project_id.clone(),
                epoch: input.epoch.clone(),
            },
            legacy_roster_key,
            input.legacy_roster_context.as_ref(),
        )?);
        let context = legacy_context.as_ref().expect("legacy 上下文刚捕获");
        collector.add(
            "legacy-roster-context",
            0,
            &stringify(context),
            ContextSourceSlot::AuthorConstraint,
            "actual preserved legacy evidence and identity baseline",
        );
    }
    if let Some(graph_input) = &input.graph_generation_input {
        let ignored_own_event_ids = input
            .graph_generation_context
            .as_ref()
            .map(|context| proven_graph_own_event_ids(transaction, context))
            .transpose()?
            .unwrap_or_default();
        *graph_context = Some(capture_graph_generation_context(
            transaction,
            graph_input,
            &GraphGenerationScope {
                project_id: input.project_id.clone(),
                epoch: input.epoch.clone(),
            },
            input.graph_generation_key.as_deref().unwrap_or_default(),
            input.graph_generation_context.as_ref(),
            &ignored_own_event_ids,
        )?);
        let context = graph_context.as_ref().expect("graph 上下文刚捕获");
        collector.add(
            "graph-generation-context",
            0,
            &stringify(context),
            ContextSourceSlot::AuthorConstraint,
            "main captured graph sources and original target baseline",
        );
    }
    if has_agent_input {
        let blueprints = query_rows_map(
            transaction,
            "SELECT * FROM blueprints ORDER BY chapter_number",
            [],
        )?;
        let stripped: Vec<Value> = blueprints
            .iter()
            .map(|row| {
                Value::Object(without(
                    row,
                    &["created_at", "updated_at", "notes_updated_at"],
                ))
            })
            .collect();
        collector.add(
            "agent-blueprints:all",
            0,
            &stable(&Value::Array(stripped)),
            ContextSourceSlot::UnconfirmedContinuity,
            "actual author plans guarded across Agent tool confirmations",
        );
    }
    if let Some(review_revision_context) = &input.review_revision_context {
        let request = review_revision_request(review_revision_context);
        let frozen_recheck_version = review_revision_context
            .recheck
            .as_ref()
            .map(|recheck| recheck.version);
        let current_context = capture_review_revision_context(
            transaction,
            &request,
            &input.project_id,
            frozen_recheck_version,
        )?;
        let current_value = serde_json::to_value(&current_context).unwrap_or_default();
        let input_value = serde_json::to_value(review_revision_context).unwrap_or_default();
        if !is_deep_strict_equal(&input_value, &current_value) {
            return Err(fail(GENERATION_REVIEW_CONTEXT_CHANGED));
        }
        collector.add(
            "review-revision-context",
            0,
            &stringify(&current_context),
            ContextSourceSlot::AuthorConstraint,
            "main-verified frozen manuscript, author decisions and review material",
        );
    }
    if input.operation == "chapter-draft" {
        let projection = current_character_projection(
            transaction,
            &input.project_id,
            transfer_authority
                .as_ref()
                .map(|authority| authority.origin_project_id.as_str()),
        )?;
        collector.add(
            "characters:all",
            0,
            &stable(&Value::Array(
                projection.into_iter().map(Value::Object).collect(),
            )),
            ContextSourceSlot::AuthorConstraint,
            "current structured character facts consumed by drafting",
        );
        let relationships = query_rows_map(
            transaction,
            "SELECT * FROM character_relationships ORDER BY relationship_id",
            [],
        )?;
        let stripped: Vec<Value> = relationships
            .iter()
            .map(|row| Value::Object(without(row, &["created_at", "updated_at"])))
            .collect();
        collector.add(
            "character-relationships:all",
            0,
            &stable(&Value::Array(stripped)),
            ContextSourceSlot::AuthorConstraint,
            "current identity relationships projected into character profiles",
        );
        let plans = query_rows_map(
            transaction,
            "SELECT * FROM narrative_thread_plans ORDER BY id",
            [],
        )?;
        let events = query_rows_map(
            transaction,
            "SELECT c.id,c.plan_id,c.draft_id,c.event_type,c.evidence,c.reason,d.chapter_number,o.chapter_title \
             FROM narrative_thread_confirmations c \
             JOIN drafts d ON d.id=c.draft_id AND d.status='finalized' \
             JOIN finalization_outbox o ON o.draft_id=d.id \
             ORDER BY d.chapter_number,c.id",
            [],
        )?;
        let mut narrative = Map::new();
        let stripped_plans: Vec<Value> = plans
            .iter()
            .map(|row| Value::Object(without(row, &["created_at", "updated_at"])))
            .collect();
        narrative.insert("plans".to_string(), Value::Array(stripped_plans));
        narrative.insert(
            "events".to_string(),
            Value::Array(events.into_iter().map(Value::Object).collect()),
        );
        collector.add(
            "narrative-thread-context:all",
            0,
            &stable(&Value::Object(narrative)),
            ContextSourceSlot::UnconfirmedContinuity,
            "author plans and confirmed event evidence used to select active narrative context",
        );
        let chapter_number = input.chapter_number.expect("chapter-draft 章节号必存在");
        let current_draft = query_row_map(
            transaction,
            "SELECT d.id,d.version,d.status,c.body FROM drafts d \
             JOIN contents c ON c.id=d.content_id WHERE d.chapter_number=? \
             ORDER BY d.version DESC,d.id DESC LIMIT 1",
            [chapter_number],
        )?;
        collector.add(
            &format!("draft-target:{chapter_number}"),
            0,
            &stable(&current_draft.map(Value::Object).unwrap_or(Value::Null)),
            ContextSourceSlot::AuthorConstraint,
            "prevent a later draft or author edit from being silently superseded",
        );
        let catalog = query_rows_map(
            transaction,
            "SELECT d.id,d.chapter_number,d.version,d.status,c.body,o.finalization_id,o.content_hash,o.content_snapshot \
             FROM drafts d \
             JOIN contents c ON c.id=d.content_id \
             LEFT JOIN finalization_outbox o ON o.draft_id=d.id \
             WHERE d.chapter_number < ? AND (d.status='finalized' OR NOT EXISTS ( \
                 SELECT 1 FROM drafts newer WHERE newer.chapter_number=d.chapter_number \
                 AND newer.status IN ('draft','revised','finalized') AND newer.version>d.version)) \
             ORDER BY d.id",
            [chapter_number],
        )?;
        let stripped: Vec<Value> = catalog
            .iter()
            .map(|row| {
                let mut record = without(row, &["body", "content_snapshot"]);
                record.insert(
                    "bodyHash".to_string(),
                    Value::String(hash(
                        &row.get("body").and_then(Value::as_str).unwrap_or_default(),
                    )),
                );
                record.insert(
                    "snapshotHash".to_string(),
                    match row.get("content_snapshot") {
                        Some(Value::Null) | None => Value::Null,
                        Some(snapshot) => {
                            Value::String(hash(&snapshot.as_str().unwrap_or_default()))
                        }
                    },
                );
                Value::Object(record)
            })
            .collect();
        collector.add(
            "draft-continuity-catalog",
            0,
            &stable(&Value::Array(stripped)),
            ContextSourceSlot::UnconfirmedContinuity,
            "source catalog frozen before renderer selects continuity material",
        );
        let locators = query_rows_map(
            transaction,
            "SELECT * FROM summary_snapshots WHERE draft_id IS NOT NULL AND chapter_number < ? ORDER BY draft_id,id",
            [chapter_number],
        )?;
        let watermark = query_row_map(
            transaction,
            "SELECT * FROM continuity_projection_meta WHERE id='main'",
            [],
        )?;
        let mut continuity = Map::new();
        let stripped_locators: Vec<Value> = locators
            .iter()
            .map(|row| Value::Object(without(row, &["created_at", "character_states"])))
            .collect();
        continuity.insert("rows".to_string(), Value::Array(stripped_locators));
        // 基线 `watermark` 为 `undefined`（无行）时键省略。
        if let Some(watermark) = watermark {
            continuity.insert("watermark".to_string(), Value::Object(watermark));
        }
        collector.add(
            "continuity-locators",
            0,
            &stable(&Value::Object(continuity)),
            ContextSourceSlot::UnconfirmedContinuity,
            "derived locators and invalidation watermark; never promoted to author facts",
        );
    }
    let mut brief = Value::Null;
    if let Some(chapter_number) = input.chapter_number {
        let row = query_row_map(
            transaction,
            "SELECT * FROM blueprints WHERE chapter_number=?",
            [chapter_number],
        )?;
        if row.is_none()
            && input.review_revision_context.is_none()
            && input.finalization_generation_slot.is_none()
        {
            return Err(fail(GENERATION_CHAPTER_MISSING));
        }
        brief = row
            .as_ref()
            .map(|row| {
                Value::Object(without(
                    row,
                    &["created_at", "updated_at", "notes", "notes_updated_at"],
                ))
            })
            .unwrap_or(Value::Null);
        collector.add(
            &format!("blueprint:{chapter_number}"),
            0,
            &stable(&brief),
            ContextSourceSlot::AuthorConstraint,
            "selected chapter brief",
        );
    }
    for chapter in blueprint_chapters {
        if Some(*chapter) == input.chapter_number {
            continue;
        }
        let row = query_row_map(
            transaction,
            "SELECT * FROM blueprints WHERE chapter_number=?",
            [chapter],
        )?;
        // 请求的空槽也是证据：后续创建必须使该快照失效。
        let stripped = row
            .as_ref()
            .map(|row| {
                Value::Object(without(
                    row,
                    &["created_at", "updated_at", "notes", "notes_updated_at"],
                ))
            })
            .unwrap_or(Value::Null);
        collector.add(
            &format!("blueprint:{chapter}"),
            0,
            &stable(&stripped),
            ContextSourceSlot::AuthorConstraint,
            "explicit planning range; current row or absent slot",
        );
    }
    for (ids, finalized) in [
        (&input.selected_draft_ids, false),
        (&input.selected_finalized_draft_ids, true),
    ] {
        for id in ids {
            let row = transaction
                .query_row(
                    "SELECT d.id,d.chapter_number,d.version,d.status,c.body FROM drafts d \
                     JOIN contents c ON c.id=d.content_id WHERE d.id=?",
                    [id],
                    |row| {
                        Ok(DraftRow {
                            id: row.get(0)?,
                            chapter_number: row.get(1)?,
                            version: row.get(2)?,
                            status: row.get(3)?,
                            body: row.get(4)?,
                        })
                    },
                )
                .optional()
                .map_err(|error| error.to_string())?;
            let Some(row) = row else {
                return Err(fail(GENERATION_SOURCE_MISSING));
            };
            if input.operation == "chapter-draft"
                && input
                    .chapter_number
                    .is_some_and(|chapter_number| row.chapter_number >= chapter_number)
            {
                return Err(fail(GENERATION_DRAFT_CONTINUITY_RANGE_INVALID));
            }
            if finalized {
                let latest = transaction
                    .query_row(
                        "SELECT id FROM drafts WHERE chapter_number=? AND status='finalized' \
                         ORDER BY version DESC,id DESC LIMIT 1",
                        [row.chapter_number],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()
                    .map_err(|error| error.to_string())?;
                let receipt = transaction
                    .query_row(
                        "SELECT * FROM finalization_outbox WHERE draft_id=?",
                        [id],
                        |row| {
                            Ok(FinalizationReceiptRow {
                                finalization_id: row.get("finalization_id")?,
                                chapter_number: row.get("chapter_number")?,
                                content_hash: row.get("content_hash")?,
                                content_snapshot: row.get("content_snapshot")?,
                                content_revision: row.get("content_revision")?,
                            })
                        },
                    )
                    .optional()
                    .map_err(|error| error.to_string())?;
                let valid = receipt.as_ref().is_some_and(|receipt| {
                    row.status == "finalized"
                        && latest == Some(*id)
                        && !receipt.finalization_id.is_empty()
                        && receipt.chapter_number == row.chapter_number
                        && receipt.content_snapshot == row.body
                        && receipt.content_hash == hash(&row.body)
                        && is_safe_integer(receipt.content_revision)
                        && receipt.content_revision >= 1
                });
                if !valid {
                    return Err(fail(GENERATION_FINALIZED_SOURCE_NOT_CURRENT));
                }
                let receipt = receipt.expect("回执已确认非空");
                collector.add(
                    &format!("finalized:{}:{}", id, receipt.finalization_id),
                    receipt.content_revision,
                    &row.body,
                    ContextSourceSlot::FinalizedFact,
                    "explicit current finalized source",
                );
            } else {
                let latest = transaction
                    .query_row(
                        "SELECT id FROM drafts WHERE chapter_number=? AND status IN ('draft','revised','finalized') \
                         ORDER BY version DESC,id DESC LIMIT 1",
                        [row.chapter_number],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()
                    .map_err(|error| error.to_string())?;
                if !["draft", "revised"].contains(&row.status.as_str()) || latest != Some(*id) {
                    return Err(fail(GENERATION_DRAFT_NOT_CURRENT));
                }
                collector.add(
                    &format!("draft:{id}"),
                    row.version,
                    &row.body,
                    ContextSourceSlot::UnconfirmedContinuity,
                    "explicit selected current draft",
                );
            }
        }
    }
    Ok(Facts {
        core,
        brief,
        language,
    })
}

/// 基线 `rebuildGenerationSourceBinding`：以新纪元（可选新模型收据 /
/// 策略 / 输出合同）从上一绑定重建。
pub fn rebuild_generation_source_binding(
    deps: &GenerationSourceBindingDependencies,
    previous: &RunBinding,
    next_epoch: &str,
    model_receipt: Option<&SafeGenerationModelReceipt>,
    policy: Option<&Value>,
    output_contract: Option<&Value>,
) -> Result<GenerationSourceBindingResult, String> {
    let manifest = &previous.source_manifest;
    if manifest.get("version") != Some(&Value::Number(1.into())) {
        return Err(fail(GENERATION_SOURCE_MANIFEST_INVALID));
    }
    // 基线 `{ ...manifest, projectId, epoch, modelReceipt: modelReceipt ?? manifest.modelReceipt,
    // policy: policy ?? manifest.policy, outputContract: outputContract ?? manifest.outputContract }`。
    let mut input_value = Value::Object(manifest.clone());
    if let Value::Object(record) = &mut input_value {
        record.insert(
            "projectId".to_string(),
            Value::String(previous.project_id.clone()),
        );
        record.insert("epoch".to_string(), Value::String(next_epoch.to_string()));
        if let Some(receipt) = model_receipt {
            record.insert(
                "modelReceipt".to_string(),
                serde_json::to_value(receipt).unwrap_or_default(),
            );
        }
        if let Some(policy) = policy {
            record.insert("policy".to_string(), policy.clone());
        }
        if let Some(contract) = output_contract {
            record.insert("outputContract".to_string(), contract.clone());
        }
    }
    let input: GenerationSourceBindingInput = serde_json::from_value(input_value)
        .map_err(|_| fail(GENERATION_SOURCE_MANIFEST_INVALID))?;
    build_generation_source_binding(deps, &input)
}

/// 基线 `compareGenerationSourceBindings`：跨纪元比较（剥离 `SourceRef.epoch`
/// 会话租约后逐项深度相等）。
pub fn compare_generation_source_bindings(previous: &RunBinding, next: &RunBinding) -> bool {
    previous.project_id == next.project_id
        && previous.fingerprint == next.fingerprint
        && previous.context_snapshot_id == next.context_snapshot_id
        && previous.source_manifest == next.source_manifest
        && previous
            .source_refs
            .iter()
            .map(source_ref_without_epoch)
            .eq(next.source_refs.iter().map(source_ref_without_epoch))
}

/// 基线 `ref => { const copy = { ...ref }; delete copy.epoch; return copy }`。
fn source_ref_without_epoch(reference: &SourceRef) -> Value {
    let mut record = Map::new();
    record.insert(
        "projectId".to_string(),
        Value::String(reference.project_id.clone()),
    );
    record.insert(
        "sourceId".to_string(),
        Value::String(reference.source_id.clone()),
    );
    record.insert(
        "revision".to_string(),
        Value::Number(reference.revision.into()),
    );
    record.insert(
        "contentHash".to_string(),
        Value::String(reference.content_hash.clone()),
    );
    if let Some(span) = &reference.span {
        record.insert(
            "span".to_string(),
            serde_json::to_value(span).unwrap_or_default(),
        );
    }
    Value::Object(record)
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_matches_sha256_hex_test() {
        // 独立口径校验：SHA-256 已知十六进制摘要（"hello" / 空串）。
        assert_eq!(
            hash("hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(
            hash(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // Buffer 重载与文本重载口径一致。
        assert_eq!(hash_bytes(b"hello"), hash("hello"));
    }

    #[test]
    fn stable_sorts_object_keys_recursively_test() {
        let value = serde_json::json!({ "b": 1, "a": { "d": 2, "c": 3 } });
        assert_eq!(stable(&value), r#"{"a":{"c":3,"d":2},"b":1}"#);
    }

    #[test]
    fn safe_metadata_rejects_private_path_and_field_test() {
        assert!(safe_metadata(&Value::String("C:\\secret".to_string())).is_err());
        assert!(safe_metadata(&Value::String("\\\\unc\\share".to_string())).is_err());
        assert!(safe_metadata(&Value::String("file:///etc/passwd".to_string())).is_err());
        let mut record = Map::new();
        record.insert("apiKey".to_string(), Value::String("x".to_string()));
        assert!(safe_metadata(&Value::Object(record)).is_err());
        let mut record = Map::new();
        record.insert("safe".to_string(), Value::String("x".to_string()));
        assert!(safe_metadata(&Value::Object(record)).is_ok());
    }

    #[test]
    fn compare_generation_source_bindings_ignores_epoch_test() {
        let reference = SourceRef {
            project_id: "p".to_string(),
            epoch: "e1".to_string(),
            source_id: "draft:1".to_string(),
            revision: 1,
            content_hash: "h".to_string(),
            span: None,
        };
        let binding = |epoch: &str| RunBinding {
            project_id: "p".to_string(),
            epoch: epoch.to_string(),
            fingerprint: FrozenInputFingerprint {
                chapter_brief_hash: String::new(),
                author_guidance_hash: String::new(),
                dependency_hash: String::new(),
                context_snapshot_hash: String::new(),
                template_hash: String::new(),
                skill_snapshot_hash: String::new(),
                model_lease_revision: String::new(),
                policy_hash: String::new(),
                output_contract_hash: String::new(),
            },
            context_snapshot_id: "context:x".to_string(),
            source_manifest: Map::new(),
            source_refs: vec![SourceRef {
                epoch: epoch.to_string(),
                ..reference.clone()
            }],
            extra: Map::new(),
        };
        assert!(compare_generation_source_bindings(
            &binding("e1"),
            &binding("e2")
        ));
    }
}
