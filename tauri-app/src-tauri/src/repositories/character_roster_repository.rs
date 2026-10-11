//! CharacterRosterRepository —— 结构化角色名单的深 module
//!
//! 平移自 `electron/repositories/character-roster-repository.ts`。
//!
//! 外部接口只有 [`read`] / [`commit`]：校验、投影、幂等、事务与回读验证全部留在
//! implementation 内。角色条目本体始终持久化在 `characters` 表，`character_roster_meta`
//! 只保存并发/迁移/投影元数据，**绝不引入第二份 JSON 事实源**。
//!
//! 与基线的**有意差异**（已确认 Tauri 版不再与原项目复用同一 SQLite）：
//! - 幂等用的规范 JSON 由 Rust 侧自建（字段顺序 = 结构体声明序），不再逐字节复刻
//!   `JSON.stringify`；哈希仍为 SHA-256 小写 hex，口径一致；
//! - `provenance` 用 `BTreeMap` 序列化，键序确定（基线为对象插入序）；
//! - 空 `provenance` 一律折叠为「键缺失」——写入恒为 `{}`，读回恒为 `None`，
//!   使「提交事实」与「回读事实」的规范 JSON 严格一致（回读校验的前提）。
//! - 身份 schema 投影（`identityProjectionEntries`）的关系不含
//!   `targetCharacterId`：本栈 `CharacterRosterRelationship` 仅
//!   {target, relation}（与基线类型同形），`canonicalIdentityEntries`
//!   的关系排序以 (target, relation) 代替基线的
//!   (targetCharacterId ?? '', target, relation)；栈内哈希自洽即可；
//! - 边缘字符大小写折叠用 Unicode 默认 `to_lowercase`，而非基线
//!   `toLocaleLowerCase('en-US')`（İ 等少数字符结果不同）；排序按
//!   UTF-8 字节序，与基线 UTF-16 码元序在极少数字符上可能不同
//!   （L3/L4 文档化保留）。

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::character_role::CharacterRole;
use crate::db::schema::migrate_character_roster_schema;
use crate::repositories::character_repository::{
    self as characters, CharacterData, CharacterStateData, CharacterStateFieldProvenance,
    CharacterStateProvenance, CharacterStateTextField, FinalizedSourceIdentity,
};
use crate::repositories::project_core_repository as project_core;
use regex::Regex;
use std::sync::LazyLock;

/// 角色名单 schema 版本（对齐 `CHARACTER_ROSTER_SCHEMA_VERSION`）
pub const CHARACTER_ROSTER_SCHEMA_VERSION: i64 = 1;

/// 角色名单允许的定位取值（对齐 `CHARACTER_ROSTER_ROLES`，**不做别名归一**）
pub const CHARACTER_ROSTER_ROLES: [&str; 4] = ["protagonist", "supporting", "antagonist", "minor"];

/// JS `Number.MAX_SAFE_INTEGER`
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

// ============================================================================
// 通用取值工具（逐条对齐 TS 的 requiredText / compareText / hasOwn 语义）
// ============================================================================

/// 必须为字符串，否则 `{label}必须是文本`（返回去空白后的值）
fn required_text(value: Option<&serde_json::Value>, label: &str) -> Result<String, String> {
    match value {
        Some(serde_json::Value::String(text)) => Ok(text.trim().to_string()),
        _ => Err(format!("{label}必须是文本")),
    }
}

/// 域内显式允许数值标量表达的字段（当前仅 `age`）走这里；其余仍走 [`required_text`]
fn required_text_or_finite_number(
    value: Option<&serde_json::Value>,
    label: &str,
) -> Result<String, String> {
    if let Some(serde_json::Value::Number(number)) = value {
        if let Some(text) = number_as_js_text(number) {
            return Ok(text);
        }
    }
    required_text(value, label)
}

/// 复刻 JS `String(number)`：整数不带小数点，3.0 输出 `3`
fn number_as_js_text(number: &serde_json::Number) -> Option<String> {
    if let Some(value) = number.as_i64() {
        return Some(value.to_string());
    }
    if let Some(value) = number.as_u64() {
        return Some(value.to_string());
    }
    let value = number.as_f64()?;
    if !value.is_finite() {
        return None;
    }
    Some(format!("{value}"))
}

/// 复刻 JS `Number.isSafeInteger(value)`
fn safe_int(value: &serde_json::Value) -> Option<i64> {
    if let Some(value) = value.as_i64() {
        return (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER)
            .contains(&value)
            .then_some(value);
    }
    let float = value.as_f64()?;
    if float.is_finite() && float.fract() == 0.0 && float.abs() <= MAX_SAFE_INTEGER as f64 {
        Some(float as i64)
    } else {
        None
    }
}

/// 复刻 `Object.hasOwn`（键存在即为真，值为 `null` 也算存在）
fn has_own(value: &serde_json::Value, key: &str) -> bool {
    value.get(key).is_some()
}

fn is_roster_role(value: Option<&serde_json::Value>) -> bool {
    value
        .and_then(|value| value.as_str())
        .map(|value| CHARACTER_ROSTER_ROLES.contains(&value))
        .unwrap_or(false)
}

/// 角色身份键（对齐 `characterRosterIdentityKey` —— `trim().toLocaleLowerCase('en-US')`）
pub fn identity_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// 规范 JSON 的 SHA-256 小写 hex（供角色名单与蓝图提交共享）
pub(crate) fn hash_text(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// 文本比较（基线为 JS 字符串比较；BMP 范围内与 UTF-8 字节序一致）
fn compare_text(left: &str, right: &str) -> std::cmp::Ordering {
    left.cmp(right)
}

fn sort_relationships(relationships: &mut [CharacterRosterRelationship]) {
    relationships.sort_by(|left, right| {
        compare_text(&left.target, &right.target)
            .then_with(|| compare_text(&left.relation, &right.relation))
    });
}

fn has_manual_text_value(value: &str) -> bool {
    !value.trim().is_empty()
}

fn has_manual_chapter_value(value: Option<i64>) -> bool {
    matches!(value, Some(chapter) if chapter != 0)
}

/// `currentState` 六字段的读写视图（避免到处 match 字段名）
impl CharacterStateTextField {
    /// 全部六个文本字段，顺序与基线一致
    pub const ALL: [CharacterStateTextField; 6] = [
        CharacterStateTextField::Location,
        CharacterStateTextField::PowerLevel,
        CharacterStateTextField::PhysicalState,
        CharacterStateTextField::MentalState,
        CharacterStateTextField::KeyItems,
        CharacterStateTextField::RecentEvents,
    ];

    /// 前端/落库使用的驼峰字段名
    pub fn json_name(self) -> &'static str {
        match self {
            CharacterStateTextField::Location => "location",
            CharacterStateTextField::PowerLevel => "powerLevel",
            CharacterStateTextField::PhysicalState => "physicalState",
            CharacterStateTextField::MentalState => "mentalState",
            CharacterStateTextField::KeyItems => "keyItems",
            CharacterStateTextField::RecentEvents => "recentEvents",
        }
    }

    /// 读取该字段的文本值
    pub fn read(self, state: &CharacterStateData) -> &str {
        match self {
            CharacterStateTextField::Location => &state.location,
            CharacterStateTextField::PowerLevel => &state.power_level,
            CharacterStateTextField::PhysicalState => &state.physical_state,
            CharacterStateTextField::MentalState => &state.mental_state,
            CharacterStateTextField::KeyItems => &state.key_items,
            CharacterStateTextField::RecentEvents => &state.recent_events,
        }
    }

    /// 写入该字段的文本值
    pub fn write(self, state: &mut CharacterStateData, value: String) {
        match self {
            CharacterStateTextField::Location => state.location = value,
            CharacterStateTextField::PowerLevel => state.power_level = value,
            CharacterStateTextField::PhysicalState => state.physical_state = value,
            CharacterStateTextField::MentalState => state.mental_state = value,
            CharacterStateTextField::KeyItems => state.key_items = value,
            CharacterStateTextField::RecentEvents => state.recent_events = value,
        }
    }
}

// ============================================================================
// 对外契约类型
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterRosterRelationship {
    pub target: String,
    pub relation: String,
}

/// 角色名单中的一个结构化事实条目
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterRosterEntry {
    /// 角色身份主键（m02；对齐基线 `characterId?`）。
    /// `None` = 新建角色，由写入路径分配 UUID；`draft:<UUID>`
    /// 仅是前端本地创建选择键，不落库。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_id: Option<String>,
    pub name: String,
    pub role: CharacterRole,
    #[serde(default)]
    pub gender: String,
    #[serde(default)]
    pub age: String,
    #[serde(default)]
    pub appearance: String,
    #[serde(default)]
    pub personality: String,
    #[serde(default)]
    pub background: String,
    #[serde(default)]
    pub abilities: String,
    #[serde(default)]
    pub motivation: String,
    #[serde(default)]
    pub relationships: Vec<CharacterRosterRelationship>,
    #[serde(default)]
    pub arc: String,
    #[serde(default)]
    pub notes: String,
    /// 缺失即「没有动态状态」（`JSON.stringify` 会省略 `undefined`，此处等价于省略键）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_state: Option<CharacterStateData>,
    /// 旧 `characters.relationships` 的自由文本证据：只会由 read 返回，
    /// 或由 `manual_edit` 原样回写
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_relationship_notes: Option<String>,
}

/// 提交意图（对齐 `CharacterRosterCommitIntent`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterRosterCommitIntent {
    Initialize,
    ArchitectureGeneration,
    LegacyRepair,
    LegacyCardsAdoption,
    ManualEdit,
    NovelImport,
    BlueprintSync,
    ChapterProgress,
}

/// 持久化迁移状态（对齐 `CharacterRosterMigrationState`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterRosterMigrationState {
    Empty,
    LegacyCardsPreserved,
    LegacyMarkdownPending,
    Ready,
}

/// 面向界面的可执行状态（对齐 `CharacterRosterStatus`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterRosterStatus {
    Empty,
    Ready,
    LegacyRepairRequired,
    Inconsistent,
}

/// 手工改名的身份映射
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterRosterRename {
    pub original_name: String,
    pub new_name: String,
}

/// 身份 schema 下的角色别名（对齐 `character_aliases` 投影）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterRosterAlias {
    pub character_id: String,
    pub name: String,
    pub source_key: String,
    pub valid_from: i64,
    pub valid_through: Option<i64>,
}

/// 角色名单快照（对齐 `CharacterRosterSnapshot`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterRosterSnapshot {
    pub schema_version: i64,
    pub revision: i64,
    pub migration_state: CharacterRosterMigrationState,
    pub status: CharacterRosterStatus,
    pub entries: Vec<CharacterRosterEntry>,
    pub rendered_markdown: String,
    pub projection_hash: String,
    pub fact_hash: String,
    /// 升级前的 `characters_arch` 原文，仅作迁移证据，绝不反向解析
    /// 身份 schema 下的 `character_identity_meta.revision`（仅身份项目）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_revision: Option<i64>,
    /// 身份 schema 下的活跃别名投影（仅身份项目）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<CharacterRosterAlias>>,
    /// 仅含角色名的事实哈希（对齐 `nameOnlyFactHash` = `meta.fact_hash`）
    #[serde(default)]
    pub name_only_fact_hash: String,
    /// 升级前的 `characters_arch` 原文，仅作迁移证据，绝不反向解析
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_markdown: Option<String>,
}

/// 提交回执（对齐 `CharacterRosterCommitReceipt`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterRosterCommitReceipt {
    pub operation_id: String,
    pub payload_hash: String,
    pub revision: i64,
    pub idempotent: bool,
    pub snapshot: CharacterRosterSnapshot,
}

// ============================================================================
// 内部结构
// ============================================================================

/// 校验通过后的提交请求（外部不可见；规范化后的唯一事实）
#[derive(Debug, Clone)]
pub struct NormalizedRosterRequest {
    pub operation_id: String,
    pub expected_revision: i64,
    pub schema_version: i64,
    pub entries: Vec<CharacterRosterEntry>,
    pub source: Option<FinalizedSourceIdentity>,
    pub intent: CharacterRosterCommitIntent,
    pub renames: Option<Vec<CharacterRosterRename>>,
    pub expected_legacy_markdown: Option<String>,
}

#[derive(Debug, Clone)]
struct RosterMetaRow {
    revision: i64,
    migration_state: CharacterRosterMigrationState,
    legacy_markdown: String,
    projection_hash: String,
    fact_hash: String,
}

/// payload_hash 的规范 JSON 形状（字段顺序 = 基线 `JSON.stringify` 的键序）
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PayloadShape {
    schema_version: i64,
    intent: CharacterRosterCommitIntent,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_legacy_markdown: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    renames: Option<Vec<CharacterRosterRename>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<FinalizedSourceIdentity>,
    entries: Vec<CharacterRosterEntry>,
}

fn is_manual_edit_intent(intent: CharacterRosterCommitIntent) -> bool {
    intent == CharacterRosterCommitIntent::ManualEdit
}

fn is_legacy_evidence_intent(intent: CharacterRosterCommitIntent) -> bool {
    intent == CharacterRosterCommitIntent::LegacyRepair
        || intent == CharacterRosterCommitIntent::LegacyCardsAdoption
}

fn normalize_role(value: &str) -> Option<CharacterRole> {
    match value {
        "protagonist" => Some(CharacterRole::Protagonist),
        "supporting" => Some(CharacterRole::Supporting),
        "antagonist" => Some(CharacterRole::Antagonist),
        "minor" => Some(CharacterRole::Minor),
        _ => None,
    }
}

// ============================================================================
// 规范化（逐条复刻基线 `normalizeRequest` 及其子函数与错误文案）
// ============================================================================

fn normalize_finalized_source(
    value: &serde_json::Value,
) -> Result<FinalizedSourceIdentity, String> {
    if !value.is_object() {
        return Err("定稿来源收据格式无效".to_string());
    }
    let finalization_id = required_text(value.get("finalizationId"), "定稿来源 ID")?;
    let content_hash = required_text(value.get("contentHash"), "定稿正文哈希")?;
    let draft_id = value.get("draftId").and_then(safe_int);
    let chapter_number = value.get("chapterNumber").and_then(safe_int);
    let content_hash_shape_ok = content_hash.len() == 64
        && content_hash
            .chars()
            .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character));
    if draft_id.map(|id| id < 1).unwrap_or(true)
        || chapter_number.map(|chapter| chapter < 1).unwrap_or(true)
        || finalization_id.is_empty()
        || !content_hash_shape_ok
    {
        return Err("定稿来源收据格式无效".to_string());
    }
    Ok(FinalizedSourceIdentity {
        draft_id: draft_id.unwrap_or(0),
        finalization_id,
        chapter_number: chapter_number.unwrap_or(0),
        content_hash,
    })
}

fn normalize_field_provenance(
    value: &serde_json::Value,
) -> Result<CharacterStateFieldProvenance, String> {
    if !value.is_object() {
        return Err("角色状态来源格式无效".to_string());
    }
    match value.get("kind").and_then(|kind| kind.as_str()) {
        Some("legacy") => Ok(CharacterStateFieldProvenance::Legacy),
        Some("author") => match value.get("chapterNumber").and_then(safe_int) {
            Some(chapter_number) if chapter_number >= 0 => {
                Ok(CharacterStateFieldProvenance::Author { chapter_number })
            }
            _ => Err("作者角色状态章节无效".to_string()),
        },
        Some("derived") => {
            let source = value
                .get("source")
                .ok_or_else(|| "定稿来源收据格式无效".to_string())?;
            Ok(CharacterStateFieldProvenance::Derived {
                source: normalize_finalized_source(source)?,
            })
        }
        _ => Err("角色状态来源类型无效".to_string()),
    }
}

fn normalize_state(
    value: Option<&serde_json::Value>,
) -> Result<Option<CharacterStateData>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if !value.is_object() {
        return Err("角色动态状态格式无效".to_string());
    }
    let updated_at_chapter = value
        .get("updatedAtChapter")
        .and_then(safe_int)
        .filter(|chapter| *chapter >= 0)
        .ok_or_else(|| "角色动态状态章节号无效".to_string())?;
    let raw_provenance = match value.get("provenance") {
        Some(provenance) if provenance.is_object() => provenance.clone(),
        Some(_) => return Err("角色状态来源格式无效".to_string()),
        None => serde_json::Value::Object(serde_json::Map::new()),
    };

    let mut provenance: CharacterStateProvenance = BTreeMap::new();
    for field in CharacterStateTextField::ALL {
        let normalized_value = required_text(
            value.get(field.json_name()),
            &format!("角色状态 {}", field.json_name()),
        )?;
        if has_own(&raw_provenance, field.json_name()) {
            let entry = raw_provenance
                .get(field.json_name())
                .ok_or_else(|| "角色状态来源格式无效".to_string())?;
            provenance.insert(
                field.json_name().to_string(),
                normalize_field_provenance(entry)?,
            );
        } else if !normalized_value.is_empty() {
            // v2 之前的生成值与非手工生成值仍然可见，但绝不升级为作者事实。
            provenance.insert(
                field.json_name().to_string(),
                CharacterStateFieldProvenance::Legacy,
            );
        }
    }

    let mut state = CharacterStateData {
        location: required_text(value.get("location"), "角色当前位置")?,
        power_level: required_text(value.get("powerLevel"), "角色修为境界")?,
        physical_state: required_text(value.get("physicalState"), "角色身体状态")?,
        mental_state: required_text(value.get("mentalState"), "角色心理状态")?,
        key_items: required_text(value.get("keyItems"), "角色关键道具")?,
        recent_events: required_text(value.get("recentEvents"), "角色最近事件")?,
        updated_at_chapter,
        provenance: None,
    };
    // 空映射折叠为「键缺失」，与读回的 `parse_provenance` 语义严格一致。
    if !provenance.is_empty() {
        state.provenance = Some(provenance);
    }
    Ok(Some(state))
}

fn normalize_relationships(
    value: Option<&serde_json::Value>,
    owner_name: &str,
) -> Result<Vec<CharacterRosterRelationship>, String> {
    let Some(serde_json::Value::Array(items)) = value else {
        return Err(format!("角色「{owner_name}」的关系必须是列表"));
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut relationships = Vec::with_capacity(items.len());
    for (index, relationship) in items.iter().enumerate() {
        if !relationship.is_object() {
            return Err(format!(
                "角色「{owner_name}」的第 {} 条关系格式无效",
                index + 1
            ));
        }
        let target = required_text(
            relationship.get("target"),
            &format!("角色「{owner_name}」的关系目标"),
        )?;
        let relation = required_text(
            relationship.get("relation"),
            &format!("角色「{owner_name}」的关系说明"),
        )?;
        if target.is_empty() {
            return Err(format!("角色「{owner_name}」的关系目标不能为空"));
        }
        if relation.is_empty() {
            return Err(format!("角色「{owner_name}」的关系说明不能为空"));
        }
        if target == owner_name {
            return Err(format!("角色「{owner_name}」不能建立自指关系"));
        }
        if !seen.insert(format!("{target}\u{0}{relation}")) {
            return Err(format!("角色「{owner_name}」存在重复关系"));
        }
        relationships.push(CharacterRosterRelationship { target, relation });
    }
    sort_relationships(&mut relationships);
    Ok(relationships)
}

fn normalize_entry(
    value: &serde_json::Value,
    allow_legacy_relationship_notes: bool,
) -> Result<CharacterRosterEntry, String> {
    if !value.is_object() {
        return Err("角色名单条目格式无效".to_string());
    }
    let name = required_text(value.get("name"), "角色名")?;
    if name.is_empty() {
        return Err("角色名不能为空".to_string());
    }
    if !is_roster_role(value.get("role")) {
        return Err(format!("角色「{name}」的定位无效"));
    }
    let role = value
        .get("role")
        .and_then(|role| role.as_str())
        .and_then(normalize_role)
        .ok_or_else(|| format!("角色「{name}」的定位无效"))?;
    let character_id = value
        .get("characterId")
        .and_then(|id| id.as_str())
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty());

    if has_own(value, "legacyRelationshipNotes") && !allow_legacy_relationship_notes {
        return Err("只有手工角色管理可以提交自由文本关系".to_string());
    }
    let legacy_relationship_notes = if allow_legacy_relationship_notes {
        match value.get("legacyRelationshipNotes") {
            Some(serde_json::Value::String(notes)) => {
                let trimmed = notes.trim().to_string();
                (!trimmed.is_empty()).then_some(trimmed)
            }
            _ => None,
        }
    } else {
        None
    };

    Ok(CharacterRosterEntry {
        character_id,
        name: name.clone(),
        role,
        gender: required_text(value.get("gender"), &format!("角色「{name}」的性别"))?,
        age: required_text_or_finite_number(value.get("age"), &format!("角色「{name}」的年龄"))?,
        appearance: required_text(value.get("appearance"), &format!("角色「{name}」的外貌"))?,
        personality: required_text(value.get("personality"), &format!("角色「{name}」的性格"))?,
        background: required_text(value.get("background"), &format!("角色「{name}」的背景"))?,
        abilities: required_text(value.get("abilities"), &format!("角色「{name}」的能力"))?,
        motivation: required_text(value.get("motivation"), &format!("角色「{name}」的动机"))?,
        relationships: normalize_relationships(value.get("relationships"), &name)?,
        arc: required_text(value.get("arc"), &format!("角色「{name}」的弧光"))?,
        notes: required_text(value.get("notes"), &format!("角色「{name}」的备注"))?,
        current_state: normalize_state(value.get("currentState"))?,
        legacy_relationship_notes,
    })
}

fn normalize_renames(
    value: Option<&serde_json::Value>,
    intent: CharacterRosterCommitIntent,
) -> Result<Option<Vec<CharacterRosterRename>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if !is_manual_edit_intent(intent) {
        return Err("只有手工角色管理可以提交角色改名映射".to_string());
    }
    let serde_json::Value::Array(items) = value else {
        return Err("角色改名映射必须是列表".to_string());
    };
    let mut renames = Vec::with_capacity(items.len());
    for raw in items {
        if !raw.is_object() {
            return Err("角色改名映射格式无效".to_string());
        }
        let original_name = required_text(raw.get("originalName"), "角色改名原名")?;
        let new_name = required_text(raw.get("newName"), "角色改名新名")?;
        if original_name.is_empty() || new_name.is_empty() {
            return Err("角色改名原名和新名不能为空".to_string());
        }
        if original_name == new_name {
            return Err("角色改名必须产生新的身份".to_string());
        }
        renames.push(CharacterRosterRename {
            original_name,
            new_name,
        });
    }
    let originals: HashSet<&str> = renames
        .iter()
        .map(|rename| rename.original_name.as_str())
        .collect();
    let targets: HashSet<&str> = renames
        .iter()
        .map(|rename| rename.new_name.as_str())
        .collect();
    if originals.len() != renames.len() || targets.len() != renames.len() {
        return Err("角色改名原名和目标名必须唯一".to_string());
    }
    Ok(Some(renames))
}

fn parse_intent(value: Option<&serde_json::Value>) -> Result<CharacterRosterCommitIntent, String> {
    match value {
        None => Ok(CharacterRosterCommitIntent::Initialize),
        Some(serde_json::Value::String(intent)) => match intent.as_str() {
            "initialize" => Ok(CharacterRosterCommitIntent::Initialize),
            "architecture_generation" => Ok(CharacterRosterCommitIntent::ArchitectureGeneration),
            "legacy_repair" => Ok(CharacterRosterCommitIntent::LegacyRepair),
            "legacy_cards_adoption" => Ok(CharacterRosterCommitIntent::LegacyCardsAdoption),
            "manual_edit" => Ok(CharacterRosterCommitIntent::ManualEdit),
            "novel_import" => Ok(CharacterRosterCommitIntent::NovelImport),
            "blueprint_sync" => Ok(CharacterRosterCommitIntent::BlueprintSync),
            "chapter_progress" => Ok(CharacterRosterCommitIntent::ChapterProgress),
            _ => Err("角色名单提交意图无效".to_string()),
        },
        Some(_) => Err("角色名单提交意图无效".to_string()),
    }
}

/// 复刻基线 `normalizeRequest`：外部输入是 `unknown`，校验与错误文案都在此处收口
pub fn normalize_request(value: &serde_json::Value) -> Result<NormalizedRosterRequest, String> {
    if !value.is_object() {
        return Err("角色名单提交请求格式无效".to_string());
    }
    let operation_id = required_text(value.get("operationId"), "操作 ID")?;
    if operation_id.is_empty() {
        return Err("操作 ID 不能为空".to_string());
    }
    if value.get("schemaVersion").and_then(safe_int) != Some(CHARACTER_ROSTER_SCHEMA_VERSION) {
        return Err("角色名单 schema 版本不受支持".to_string());
    }
    let expected_revision = value
        .get("expectedRevision")
        .and_then(safe_int)
        .filter(|revision| *revision >= 0)
        .ok_or_else(|| "角色名单 revision 无效".to_string())?;
    let intent = parse_intent(value.get("intent"))?;
    let raw_entries = match value.get("entries") {
        Some(serde_json::Value::Array(entries)) => entries,
        _ => return Err("角色名单不能为空".to_string()),
    };
    if raw_entries.is_empty() && !is_manual_edit_intent(intent) {
        return Err("角色名单不能为空".to_string());
    }

    let source = match value.get("source") {
        Some(source) => Some(normalize_finalized_source(source)?),
        None => None,
    };
    if intent == CharacterRosterCommitIntent::ChapterProgress && source.is_none() {
        return Err("章节状态更新缺少定稿来源收据".to_string());
    }
    if intent != CharacterRosterCommitIntent::ChapterProgress && source.is_some() {
        return Err("只有章节状态更新可携带定稿来源收据".to_string());
    }

    let allow_legacy = is_manual_edit_intent(intent);
    let mut entries = Vec::with_capacity(raw_entries.len());
    for raw in raw_entries {
        entries.push(normalize_entry(raw, allow_legacy)?);
    }
    let names: HashSet<String> = entries
        .iter()
        .map(|entry| identity_key(&entry.name))
        .collect();
    if names.len() != entries.len() {
        return Err("角色名必须唯一".to_string());
    }
    // 批量生成/导入/蓝图/定稿的增量候选可以引用「本次没有变化」的既有角色；
    // 只有空项目初始化与旧图谱修复需要在候选本身上直接闭合。
    if intent == CharacterRosterCommitIntent::Initialize
        || intent == CharacterRosterCommitIntent::LegacyRepair
    {
        for entry in &entries {
            for relationship in &entry.relationships {
                if !names.contains(&identity_key(&relationship.target)) {
                    return Err(format!(
                        "角色「{}」引用了不存在的关系目标「{}」",
                        entry.name, relationship.target
                    ));
                }
            }
        }
    }
    let renames = normalize_renames(value.get("renames"), intent)?;
    let expected_legacy_markdown = if is_legacy_evidence_intent(intent) {
        match value.get("expectedLegacyMarkdown") {
            Some(serde_json::Value::String(markdown)) => Some(markdown.clone()),
            _ => return Err("旧角色图谱证据缺失，已拒绝修复".to_string()),
        }
    } else {
        None
    };

    Ok(NormalizedRosterRequest {
        operation_id,
        expected_revision,
        schema_version: CHARACTER_ROSTER_SCHEMA_VERSION,
        entries,
        source,
        intent,
        renames,
        expected_legacy_markdown,
    })
}

// ============================================================================
// 规范 JSON 与哈希
// ============================================================================

/// 规范条目序列：关系排序 + 按名字排序（对齐 `canonicalEntries`）
pub fn canonical_entries(entries: &[CharacterRosterEntry]) -> Vec<CharacterRosterEntry> {
    let mut canonical: Vec<CharacterRosterEntry> = entries
        .iter()
        .map(|entry| {
            let mut next = entry.clone();
            sort_relationships(&mut next.relationships);
            next
        })
        .collect();
    canonical.sort_by(|left, right| compare_text(&left.name, &right.name));
    canonical
}

fn canonical_entries_json(entries: &[CharacterRosterEntry]) -> Result<String, String> {
    serde_json::to_string(&canonical_entries(entries)).map_err(|error| error.to_string())
}

/// 覆盖角色资料、结构化关系与 `currentState` 的完整事实哈希
fn full_fact_hash(entries: &[CharacterRosterEntry]) -> Result<String, String> {
    Ok(hash_text(&canonical_entries_json(entries)?))
}

/// 身份 schema 下的规范条目序列（对齐 `canonicalIdentityEntries`）：
/// 关系按 (target, relation) 排序，条目按 (characterId ?? '', name) 排序。
///
/// 基线关系排序键为 (targetCharacterId ?? '', target, relation)；
/// 本栈关系不含 targetCharacterId（见文件头差异声明）。
fn canonical_identity_entries(entries: &[CharacterRosterEntry]) -> Vec<CharacterRosterEntry> {
    let mut canonical: Vec<CharacterRosterEntry> = entries
        .iter()
        .map(|entry| {
            let mut next = entry.clone();
            next.relationships.sort_by(|left, right| {
                compare_text(&left.target, &right.target)
                    .then_with(|| compare_text(&left.relation, &right.relation))
            });
            next
        })
        .collect();
    canonical.sort_by(|left, right| {
        compare_text(
            left.character_id.as_deref().unwrap_or(""),
            right.character_id.as_deref().unwrap_or(""),
        )
        .then_with(|| compare_text(&left.name, &right.name))
    });
    canonical
}

fn canonical_identity_entries_json(entries: &[CharacterRosterEntry]) -> Result<String, String> {
    serde_json::to_string(&canonical_identity_entries(entries)).map_err(|error| error.to_string())
}

/// 身份 schema 下的完整事实哈希（对齐 `fullIdentityFactHash`）
fn full_identity_fact_hash(entries: &[CharacterRosterEntry]) -> Result<String, String> {
    Ok(hash_text(&canonical_identity_entries_json(entries)?))
}

/// 幂等键：对规范化请求的规范 JSON 取 SHA-256
fn payload_hash(request: &NormalizedRosterRequest) -> Result<String, String> {
    let shape = PayloadShape {
        schema_version: request.schema_version,
        intent: request.intent,
        expected_legacy_markdown: if is_legacy_evidence_intent(request.intent) {
            request.expected_legacy_markdown.clone()
        } else {
            None
        },
        renames: if is_manual_edit_intent(request.intent) {
            Some(request.renames.clone().unwrap_or_default())
        } else {
            None
        },
        source: request.source.clone(),
        entries: canonical_entries(&request.entries),
    };
    Ok(hash_text(
        &serde_json::to_string(&shape).map_err(|error| error.to_string())?,
    ))
}

// ============================================================================
// 角色卡 ↔ 名单条目
// ============================================================================

/// `characters.relationships` 是否为结构化 JSON 数组
fn is_structured_relationships(value: &str) -> bool {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(value) else {
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
        role: character.role,
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

/// 身份 schema 下的公开事实投影（对齐 `identityProjectionEntries`）：
/// 活跃角色的 [`entry_from_character`] 投影，关系一律改写为
/// `character_relationships` 中 target 未退役的结构化关系
/// （按 `relationship_id` 序，不二次排序）。
///
/// `include_ids = false` 清除条目级 `characterId`（公开事实）；
/// `true` 时保留。基线关系投影含 `targetCharacterId`，本栈
/// `CharacterRosterRelationship` 不含该字段（见文件头差异声明）。
fn identity_projection_entries(
    conn: &Connection,
    include_ids: bool,
) -> Result<Vec<CharacterRosterEntry>, String> {
    let relationships: Vec<(String, String, String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT r.source_character_id, r.target_character_id, c.name, r.relation \
                 FROM character_relationships r \
                 JOIN characters c ON c.character_id = r.target_character_id \
                 WHERE c.retired = 0 \
                 ORDER BY r.relationship_id",
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
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
        let mut stmt = conn
            .prepare("SELECT character_id FROM characters WHERE retired = 0")
            .map_err(|error| error.to_string())?;
        let rows = stmt
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
            entry.character_id = if include_ids {
                character.character_id.clone()
            } else {
                None
            };
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

fn character_from_entry(entry: &CharacterRosterEntry) -> CharacterData {
    CharacterData {
        character_id: entry.character_id.clone(),
        name: entry.name.clone(),
        role: entry.role,
        gender: entry.gender.clone(),
        age: entry.age.clone(),
        appearance: entry.appearance.clone(),
        personality: entry.personality.clone(),
        background: entry.background.clone(),
        abilities: entry.abilities.clone(),
        motivation: entry.motivation.clone(),
        // 旧手工卡的自由文本关系尚无安全的字段级迁移；安全重生成必须原样保留，
        // 不能为了写入新候选而把它静默替换为 JSON。
        relationships: match entry.legacy_relationship_notes.as_deref().map(str::trim) {
            Some(notes) if !notes.is_empty() => {
                entry.legacy_relationship_notes.clone().unwrap_or_default()
            }
            _ => serde_json::to_string(&entry.relationships).unwrap_or_else(|_| "[]".to_string()),
        },
        arc: entry.arc.clone(),
        notes: entry.notes.clone(),
        current_state: entry.current_state.clone(),
    }
}

// ============================================================================
// manual-wins 合并（架构重生成）
// ============================================================================

fn merge_current_state_manual_wins(
    existing: Option<&CharacterStateData>,
    generated: Option<&CharacterStateData>,
) -> Option<CharacterStateData> {
    // `{ ...generated, ...existing }`：先铺生成值，再用已有值覆盖
    let mut merged = match (existing, generated) {
        (Some(existing), Some(generated)) => CharacterStateData {
            location: existing.location.clone(),
            power_level: existing.power_level.clone(),
            physical_state: existing.physical_state.clone(),
            mental_state: existing.mental_state.clone(),
            key_items: existing.key_items.clone(),
            recent_events: existing.recent_events.clone(),
            updated_at_chapter: existing.updated_at_chapter,
            provenance: existing
                .provenance
                .clone()
                .or_else(|| generated.provenance.clone()),
        },
        (Some(existing), None) => existing.clone(),
        (None, Some(generated)) => generated.clone(),
        (None, None) => return None,
    };
    let Some(generated) = generated else {
        return Some(merged);
    };
    for field in CharacterStateTextField::ALL {
        if !has_manual_text_value(field.read(&merged)) {
            field.write(&mut merged, field.read(generated).to_string());
        }
    }
    if !has_manual_chapter_value(existing.map(|state| state.updated_at_chapter)) {
        merged.updated_at_chapter = generated.updated_at_chapter;
    }
    Some(merged)
}

fn merge_existing_entry_manual_wins(
    existing: &CharacterRosterEntry,
    generated: &CharacterRosterEntry,
) -> CharacterRosterEntry {
    let mut merged = existing.clone();
    merged.name = existing.name.trim().to_string();
    // 基线上 `role` 必然是非空字符串，`!hasManualValue(existing.role)` 恒为 false：
    // 架构重生成永不改写既有角色的定位。
    if !has_manual_text_value(&merged.gender) && has_manual_text_value(&generated.gender) {
        merged.gender = generated.gender.clone();
    }
    if !has_manual_text_value(&merged.age) && has_manual_text_value(&generated.age) {
        merged.age = generated.age.clone();
    }
    if !has_manual_text_value(&merged.appearance) && has_manual_text_value(&generated.appearance) {
        merged.appearance = generated.appearance.clone();
    }
    if !has_manual_text_value(&merged.personality) && has_manual_text_value(&generated.personality)
    {
        merged.personality = generated.personality.clone();
    }
    if !has_manual_text_value(&merged.background) && has_manual_text_value(&generated.background) {
        merged.background = generated.background.clone();
    }
    if !has_manual_text_value(&merged.abilities) && has_manual_text_value(&generated.abilities) {
        merged.abilities = generated.abilities.clone();
    }
    if !has_manual_text_value(&merged.motivation) && has_manual_text_value(&generated.motivation) {
        merged.motivation = generated.motivation.clone();
    }
    if !has_manual_text_value(&merged.arc) && has_manual_text_value(&generated.arc) {
        merged.arc = generated.arc.clone();
    }
    if !has_manual_text_value(&merged.notes) && has_manual_text_value(&generated.notes) {
        merged.notes = generated.notes.clone();
    }
    let keep_existing_relationships = existing
        .legacy_relationship_notes
        .as_deref()
        .map(has_manual_text_value)
        .unwrap_or(false)
        || !existing.relationships.is_empty();
    merged.relationships = if keep_existing_relationships {
        existing.relationships.clone()
    } else {
        generated.relationships.clone()
    };
    merged.current_state = merge_current_state_manual_wins(
        existing.current_state.as_ref(),
        generated.current_state.as_ref(),
    );
    merged
}

fn assert_relationship_closure(entries: &[CharacterRosterEntry]) -> Result<(), String> {
    let names: HashSet<String> = entries
        .iter()
        .map(|entry| identity_key(&entry.name))
        .collect();
    if names.len() != entries.len() || entries.iter().any(|entry| entry.name.trim().is_empty()) {
        return Err("已有角色身份不安全，已拒绝合并".to_string());
    }
    for entry in entries {
        let owner = identity_key(&entry.name);
        let mut keys: HashSet<String> = HashSet::new();
        for relationship in &entry.relationships {
            let target = identity_key(&relationship.target);
            if relationship.target.trim().is_empty()
                || relationship.relation.trim().is_empty()
                || target == owner
                || !names.contains(&target)
            {
                return Err("已有角色关系不完整，已拒绝合并".to_string());
            }
            if !keys.insert(format!(
                "{}\u{0}{}",
                relationship.target, relationship.relation
            )) {
                return Err("已有角色关系存在重复，已拒绝合并".to_string());
            }
        }
    }
    Ok(())
}

/// 正常架构重新生成延续旧的 manual-wins 安全规则：非空旧字段与未出现在本轮
/// 候选中的旧角色都保留；空字段才由新候选补齐。该策略不猜测字段来源。
fn merge_generated_entries_with_existing(
    generated_entries: &[CharacterRosterEntry],
    existing_entries: &[CharacterRosterEntry],
) -> Result<Vec<CharacterRosterEntry>, String> {
    let generated_by_name: HashMap<String, &CharacterRosterEntry> = generated_entries
        .iter()
        .map(|entry| (identity_key(&entry.name), entry))
        .collect();
    let mut merged: Vec<CharacterRosterEntry> = existing_entries
        .iter()
        .map(
            |existing| match generated_by_name.get(&identity_key(&existing.name)) {
                Some(generated) => merge_existing_entry_manual_wins(existing, generated),
                None => existing.clone(),
            },
        )
        .collect();
    let existing_names: HashSet<String> = existing_entries
        .iter()
        .map(|entry| identity_key(&entry.name))
        .collect();
    merged.extend(
        generated_entries
            .iter()
            .filter(|entry| !existing_names.contains(&identity_key(&entry.name)))
            .cloned(),
    );
    assert_relationship_closure(&merged)?;
    Ok(merged)
}

// ============================================================================
// 定稿来源校验与增量合并
// ============================================================================

fn has_finalized_draft(conn: &Connection, chapter_number: i64) -> Result<bool, String> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM drafts WHERE chapter_number = ?1 AND status = 'finalized' LIMIT 1",
            [chapter_number],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(found.is_some())
}

fn assert_current_finalized_source(
    conn: &Connection,
    source: &FinalizedSourceIdentity,
) -> Result<(), String> {
    let row: Option<(i64, String, String, String)> = conn
        .query_row(
            "SELECT drafts.chapter_number AS chapterNumber, drafts.status, \
             finalization_outbox.finalization_id AS finalizationId, \
             finalization_outbox.content_hash AS contentHash \
             FROM drafts \
             JOIN finalization_outbox ON finalization_outbox.draft_id = drafts.id \
             WHERE drafts.id = ?1",
            [source.draft_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let matches = row
        .as_ref()
        .map(|(chapter_number, status, finalization_id, content_hash)| {
            status == "finalized"
                && *chapter_number == source.chapter_number
                && finalization_id == &source.finalization_id
                && content_hash == &source.content_hash
        })
        .unwrap_or(false);
    if !matches {
        return Err("角色状态来源已失效，已拒绝过期后处理写入".to_string());
    }
    Ok(())
}

fn merge_derived_current_state(
    existing: Option<&CharacterStateData>,
    candidate: &CharacterStateData,
    source: &FinalizedSourceIdentity,
) -> Result<Option<CharacterStateData>, String> {
    let mut merged = match existing {
        Some(existing) => existing.clone(),
        None => CharacterStateData {
            location: String::new(),
            power_level: String::new(),
            physical_state: String::new(),
            mental_state: String::new(),
            key_items: String::new(),
            recent_events: String::new(),
            updated_at_chapter: source.chapter_number,
            provenance: None,
        },
    };
    let mut changed = false;
    for field in CharacterStateTextField::ALL {
        let Some(provenance) = candidate
            .provenance
            .as_ref()
            .and_then(|provenance| provenance.get(field.json_name()))
        else {
            continue;
        };
        let CharacterStateFieldProvenance::Derived {
            source: field_source,
        } = provenance
        else {
            continue;
        };
        if field_source.draft_id != source.draft_id
            || field_source.finalization_id != source.finalization_id
            || field_source.chapter_number != source.chapter_number
            || field_source.content_hash != source.content_hash
        {
            return Err("角色状态字段与定稿来源不一致".to_string());
        }
        let previous_source = existing
            .and_then(|state| state.provenance.as_ref())
            .and_then(|provenance| provenance.get(field.json_name()));
        let previous_value = existing.map(|state| field.read(state)).unwrap_or("");
        // 未知的历史（legacy）与明确的作者值绝不静默重分类或覆盖。
        if !previous_value.is_empty()
            && !matches!(
                previous_source,
                Some(CharacterStateFieldProvenance::Derived { .. })
            )
        {
            continue;
        }
        if matches!(
            previous_source,
            Some(CharacterStateFieldProvenance::Author { .. })
                | Some(CharacterStateFieldProvenance::Legacy)
        ) {
            continue;
        }
        field.write(&mut merged, field.read(candidate).to_string());
        merged
            .provenance
            .get_or_insert_with(BTreeMap::new)
            .insert(field.json_name().to_string(), provenance.clone());
        changed = true;
    }
    if !changed {
        return Ok(existing.cloned());
    }
    merged.updated_at_chapter = source.chapter_number;
    Ok(Some(merged))
}

fn merge_incremental_entries_with_existing(
    conn: &Connection,
    candidates: &[CharacterRosterEntry],
    existing_entries: &[CharacterRosterEntry],
    intent: CharacterRosterCommitIntent,
    source: Option<&FinalizedSourceIdentity>,
) -> Result<Vec<CharacterRosterEntry>, String> {
    if intent == CharacterRosterCommitIntent::ChapterProgress {
        let Some(source) = source else {
            return Err("章节状态更新缺少定稿来源收据".to_string());
        };
        assert_current_finalized_source(conn, source)?;
        for candidate in candidates {
            let chapter_matches = candidate
                .current_state
                .as_ref()
                .map(|state| state.updated_at_chapter == source.chapter_number)
                .unwrap_or(false);
            if !chapter_matches {
                return Err(format!(
                    "角色「{}」的章节状态尚未定稿，已拒绝后处理写入",
                    candidate.name
                ));
            }
        }
    }

    let candidate_by_name: HashMap<String, &CharacterRosterEntry> = candidates
        .iter()
        .map(|entry| (identity_key(&entry.name), entry))
        .collect();
    let mut merged: Vec<CharacterRosterEntry> =
        Vec::with_capacity(existing_entries.len() + candidates.len());
    for existing in existing_entries {
        let Some(candidate) = candidate_by_name.get(&identity_key(&existing.name)) else {
            merged.push(existing.clone());
            continue;
        };
        if intent == CharacterRosterCommitIntent::ChapterProgress {
            if let (Some(candidate_state), Some(existing_state)) = (
                candidate.current_state.as_ref(),
                existing.current_state.as_ref(),
            ) {
                if candidate_state.updated_at_chapter < existing_state.updated_at_chapter
                    && has_finalized_draft(conn, existing_state.updated_at_chapter)?
                {
                    return Err(format!(
                        "角色「{}」已由较新章节更新，已拒绝旧章节后处理覆盖",
                        existing.name
                    ));
                }
            }
        }
        // 蓝图同步只附加结构化关系；章节定稿则以本轮已验证的状态补丁推进
        // currentState。其他资料保留已有事实，避免工作流重写人工档案。
        let mut next = existing.clone();
        if !candidate.relationships.is_empty() {
            next.relationships = candidate.relationships.clone();
        }
        if intent == CharacterRosterCommitIntent::ChapterProgress {
            if let (Some(candidate_state), Some(source)) =
                (candidate.current_state.as_ref(), source)
            {
                next.current_state = merge_derived_current_state(
                    existing.current_state.as_ref(),
                    candidate_state,
                    source,
                )?;
            }
        }
        merged.push(next);
    }
    let existing_names: HashSet<String> = existing_entries
        .iter()
        .map(|entry| identity_key(&entry.name))
        .collect();
    if intent == CharacterRosterCommitIntent::ChapterProgress {
        if let Some(addition) = candidates
            .iter()
            .find(|candidate| !existing_names.contains(&identity_key(&candidate.name)))
        {
            return Err(format!(
                "章节定稿后处理不能创建未知角色「{}」",
                addition.name
            ));
        }
    }
    merged.extend(
        candidates
            .iter()
            .filter(|candidate| !existing_names.contains(&identity_key(&candidate.name)))
            .cloned(),
    );
    assert_relationship_closure(&merged)?;
    Ok(merged)
}

// ============================================================================
// 手工快照解析（manual_edit）
// ============================================================================

fn map_manual_relationship_targets(
    entry: &CharacterRosterEntry,
    rename_by_original: &HashMap<String, String>,
    final_names: &HashSet<String>,
) -> CharacterRosterEntry {
    let mut seen: HashSet<String> = HashSet::new();
    let mut relationships: Vec<CharacterRosterRelationship> = Vec::new();
    for relationship in &entry.relationships {
        let target = rename_by_original
            .get(&relationship.target)
            .cloned()
            .unwrap_or_else(|| relationship.target.clone());
        // 被省略的手工条目即删除；其结构化边必须在同一次提交内一并移除，
        // 不允许残留失效关系。
        if !final_names.contains(&target) || target == entry.name {
            continue;
        }
        if !seen.insert(format!("{target}\u{0}{}", relationship.relation)) {
            continue;
        }
        relationships.push(CharacterRosterRelationship {
            target,
            relation: relationship.relation.clone(),
        });
    }
    sort_relationships(&mut relationships);
    CharacterRosterEntry {
        relationships,
        ..entry.clone()
    }
}

fn resolve_manual_entries(
    request: &NormalizedRosterRequest,
    existing_entries: &[CharacterRosterEntry],
) -> Result<(Vec<CharacterRosterEntry>, HashMap<String, String>), String> {
    let no_renames: Vec<CharacterRosterRename> = Vec::new();
    let renames = request.renames.as_ref().unwrap_or(&no_renames);
    let existing_by_name: HashMap<&str, &CharacterRosterEntry> = existing_entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry))
        .collect();
    let final_names: HashSet<String> = request
        .entries
        .iter()
        .map(|entry| entry.name.clone())
        .collect();
    let rename_by_original: HashMap<String, String> = renames
        .iter()
        .map(|rename| (rename.original_name.clone(), rename.new_name.clone()))
        .collect();
    let rename_by_new: HashMap<String, String> = renames
        .iter()
        .map(|rename| (rename.new_name.clone(), rename.original_name.clone()))
        .collect();

    for rename in renames {
        if !existing_by_name.contains_key(rename.original_name.as_str()) {
            return Err(format!("角色「{}」不存在，无法改名", rename.original_name));
        }
        if !final_names.contains(&rename.new_name) {
            return Err(format!(
                "角色改名「{} → {}」与保存内容不一致",
                rename.original_name, rename.new_name
            ));
        }
    }

    let mut entries = Vec::with_capacity(request.entries.len());
    for candidate in &request.entries {
        let original_name = rename_by_new
            .get(&candidate.name)
            .cloned()
            .unwrap_or_else(|| candidate.name.clone());
        let existing = existing_by_name.get(original_name.as_str()).copied();
        let mut mapped =
            map_manual_relationship_targets(candidate, &rename_by_original, &final_names);
        if let Some(notes) = existing
            .and_then(|existing| existing.legacy_relationship_notes.as_deref())
            .filter(|notes| !notes.is_empty())
        {
            if mapped.legacy_relationship_notes.is_none() {
                mapped.legacy_relationship_notes = Some(notes.to_string());
            }
        }
        let Some(mut current_state) = mapped.current_state.clone() else {
            entries.push(mapped);
            continue;
        };
        let mut provenance = current_state.provenance.clone().unwrap_or_default();
        for field in CharacterStateTextField::ALL {
            let existing_value = existing
                .and_then(|existing| existing.current_state.as_ref())
                .map(|state| field.read(state))
                .unwrap_or("");
            if existing_value != field.read(&current_state) {
                provenance.insert(
                    field.json_name().to_string(),
                    CharacterStateFieldProvenance::Author {
                        chapter_number: current_state.updated_at_chapter,
                    },
                );
            } else if let Some(existing_provenance) = existing
                .and_then(|existing| existing.current_state.as_ref())
                .and_then(|state| state.provenance.as_ref())
                .and_then(|provenance| provenance.get(field.json_name()))
            {
                provenance.insert(field.json_name().to_string(), existing_provenance.clone());
            }
        }
        // 空映射折叠为「键缺失」，保证提交事实与回读事实一致。
        current_state.provenance = (!provenance.is_empty()).then_some(provenance);
        mapped.current_state = Some(current_state);
        entries.push(mapped);
    }
    assert_relationship_closure(&entries)?;
    Ok((entries, rename_by_original))
}

fn update_blueprint_references_for_manual_edit(
    conn: &Connection,
    rename_by_original: &HashMap<String, String>,
    final_names: &HashSet<String>,
) -> Result<(), String> {
    let blueprints: Vec<(i64, String)> = {
        let mut stmt = conn
            .prepare("SELECT chapter_number, characters FROM blueprints")
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ))
            })
            .map_err(|error| error.to_string())?;
        let mut collected = Vec::new();
        for row in rows {
            collected.push(row.map_err(|error| error.to_string())?);
        }
        collected
    };
    for (chapter_number, characters_json) in blueprints {
        let parsed: serde_json::Value = serde_json::from_str(&characters_json)
            .map_err(|_| format!("第 {chapter_number} 章蓝图角色列表损坏"))?;
        let Some(items) = parsed.as_array() else {
            return Err(format!("第 {chapter_number} 章蓝图角色列表格式错误"));
        };
        let mut next_names: Vec<String> = Vec::new();
        for item in items {
            let Some(name) = item.as_str() else {
                return Err(format!("第 {chapter_number} 章蓝图角色列表格式错误"));
            };
            let renamed = rename_by_original
                .get(name)
                .cloned()
                .unwrap_or_else(|| name.to_string());
            if !final_names.contains(&renamed) || next_names.contains(&renamed) {
                continue;
            }
            next_names.push(renamed);
        }
        let payload = serde_json::to_string(&next_names).map_err(|error| error.to_string())?;
        if payload != serde_json::to_string(&parsed).unwrap_or_default() {
            conn.execute(
                "UPDATE blueprints SET characters = ?1, updated_at = datetime('now') WHERE chapter_number = ?2",
                rusqlite::params![payload, chapter_number],
            )
            .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

// ============================================================================
// 展示投影
// ============================================================================

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

/// 只从结构化角色名单生成展示 Markdown。此函数绝不读取或解释旧 Markdown。
pub fn render_character_roster_markdown(
    entries: &[CharacterRosterEntry],
    writing_language: &str,
) -> String {
    let canonical = sorted_entries(entries);
    if canonical.is_empty() {
        return String::new();
    }
    let english = writing_language == "en-US";
    let separator = if english { ": " } else { "：" };
    let mut blocks: Vec<String> = Vec::with_capacity(canonical.len());
    for entry in &canonical {
        let mut lines: Vec<String> = vec![format!(
            "## {}{separator}{}",
            entry.role.label(english),
            entry.name
        )];
        let fields: [(&str, &str); 9] = [
            (if english { "Gender" } else { "性别" }, &entry.gender),
            (if english { "Age" } else { "年龄" }, &entry.age),
            (
                if english { "Appearance" } else { "外貌" },
                &entry.appearance,
            ),
            (
                if english { "Personality" } else { "性格" },
                &entry.personality,
            ),
            (
                if english { "Background" } else { "背景" },
                &entry.background,
            ),
            (if english { "Abilities" } else { "能力" }, &entry.abilities),
            (
                if english { "Motivation" } else { "动机" },
                &entry.motivation,
            ),
            (if english { "Arc" } else { "弧光" }, &entry.arc),
            (if english { "Notes" } else { "备注" }, &entry.notes),
        ];
        for (label, value) in fields {
            if !value.is_empty() {
                lines.push(format!("- {label}{separator}{value}"));
            }
        }
        for relationship in &entry.relationships {
            lines.push(if english {
                format!(
                    "- Relationship: {} ({})",
                    relationship.target, relationship.relation
                )
            } else {
                format!(
                    "- 关系：{}（{}）",
                    relationship.target, relationship.relation
                )
            });
        }
        if let Some(notes) = &entry.legacy_relationship_notes {
            lines.push(if english {
                format!("- Relationship notes: {notes}")
            } else {
                format!("- 关系备注：{notes}")
            });
        }
        blocks.push(lines.join("\n"));
    }
    let mut all: Vec<String> = vec![if english {
        "# Character graph".to_string()
    } else {
        "# 角色图谱".to_string()
    }];
    all.extend(blocks);
    all.join("\n\n")
}

// ============================================================================
// 读取与状态推导
// ============================================================================

fn read_current_projection(conn: &Connection) -> String {
    conn.query_row(
        "SELECT COALESCE(characters_arch, '') AS characters_arch FROM project_core WHERE id = 'main'",
        [],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or_default()
}

fn parse_migration_state(value: &str) -> Result<CharacterRosterMigrationState, String> {
    match value {
        "empty" => Ok(CharacterRosterMigrationState::Empty),
        "legacy_cards_preserved" => Ok(CharacterRosterMigrationState::LegacyCardsPreserved),
        "legacy_markdown_pending" => Ok(CharacterRosterMigrationState::LegacyMarkdownPending),
        "ready" => Ok(CharacterRosterMigrationState::Ready),
        _ => Err("角色名单迁移状态无效".to_string()),
    }
}

fn read_meta(conn: &Connection) -> Result<RosterMetaRow, String> {
    let row: Option<(i64, String, String, String, String)> = conn
        .query_row(
            "SELECT revision, migration_state, legacy_markdown, projection_hash, fact_hash \
             FROM character_roster_meta WHERE id = 'main'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((revision, migration_state, legacy_markdown, projection_hash, fact_hash)) = row else {
        return Err("角色名单元数据未初始化".to_string());
    };
    Ok(RosterMetaRow {
        revision,
        migration_state: parse_migration_state(&migration_state)?,
        legacy_markdown,
        projection_hash,
        fact_hash,
    })
}

/// 严格区分「可安全使用的既有角色卡」与「需要作者显式修复的旧 Markdown」。
/// 所有自动打开/读取路径只做分类，绝不在这里写入或解析旧文本。
fn derive_roster_status(
    meta: &RosterMetaRow,
    entries: &[CharacterRosterEntry],
    rendered_markdown: &str,
    current_projection: &str,
) -> Result<CharacterRosterStatus, String> {
    match meta.migration_state {
        CharacterRosterMigrationState::Empty => Ok(
            if entries.is_empty()
                && meta.legacy_markdown.trim().is_empty()
                && current_projection.trim().is_empty()
            {
                CharacterRosterStatus::Empty
            } else {
                CharacterRosterStatus::Inconsistent
            },
        ),
        // 已有角色卡本身是受保护的结构化事实，但旧项目还未用这些事实重建
        // 确定性只读图谱。不能在打开项目时自动写入，也不能直接标记 ready。
        CharacterRosterMigrationState::LegacyCardsPreserved => {
            Ok(CharacterRosterStatus::Inconsistent)
        }
        CharacterRosterMigrationState::LegacyMarkdownPending => Ok(
            if entries.is_empty() && !meta.legacy_markdown.trim().is_empty() {
                CharacterRosterStatus::LegacyRepairRequired
            } else {
                CharacterRosterStatus::Inconsistent
            },
        ),
        // read/commit 是唯一写入 seam。投影和包含 currentState 的完整事实哈希
        // 都必须与表内事实一致，才能让 UI/工作流继续写入。
        CharacterRosterMigrationState::Ready => {
            if meta.projection_hash != hash_text(rendered_markdown)
                || meta.fact_hash != full_fact_hash(entries)?
                || current_projection != rendered_markdown
            {
                return Ok(CharacterRosterStatus::Inconsistent);
            }
            Ok(if entries.is_empty() {
                CharacterRosterStatus::Empty
            } else {
                CharacterRosterStatus::Ready
            })
        }
    }
}

fn read_snapshot(conn: &Connection) -> Result<CharacterRosterSnapshot, String> {
    let meta = read_meta(conn)?;
    let has_identity_schema = characters::has_character_identity_schema(conn);
    let writing_language = project_core::get(conn)?
        .map(|core| core.writing_language)
        .unwrap_or_else(|| project_core::DEFAULT_WRITING_LANGUAGE.to_string());
    let current_projection = read_current_projection(conn);
    // 身份 schema：公开事实走身份投影（`identityProjectionEntries`）——
    // `entries` 不含稳定 ID（界面展示/状态判定用），`identity_entries`
    // 含稳定 ID（快照事实源）；否则走 legacy 角色卡投影。
    let (entries, identity_entries, fact_hash) = if has_identity_schema {
        let entries = identity_projection_entries(conn, false)?;
        let identity_entries = identity_projection_entries(conn, true)?;
        let fact_hash = full_identity_fact_hash(&identity_entries)?;
        (entries, identity_entries, fact_hash)
    } else {
        let mut raw_entries: Vec<CharacterRosterEntry> = Vec::new();
        for character in characters::get_all(conn)? {
            raw_entries.push(entry_from_character(&character));
        }
        let entries = sorted_entries(&raw_entries);
        let fact_hash = meta.fact_hash.clone();
        (entries.clone(), entries, fact_hash)
    };
    let localized_projection = render_character_roster_markdown(&entries, &writing_language);
    let previous_language = if writing_language == "en-US" {
        "zh-CN"
    } else {
        "en-US"
    };
    let previous_language_projection =
        render_character_roster_markdown(&entries, previous_language);
    // 项目可能在 ready 名单提交后变更写作语言。既有的历史投影必须保持可读；
    // 下一次 roster 提交会在当前项目语言下重写它，而不改变任何事实。
    let rendered_markdown = if meta.migration_state == CharacterRosterMigrationState::Ready
        && current_projection == previous_language_projection
        && meta.projection_hash == hash_text(&previous_language_projection)
    {
        previous_language_projection
    } else {
        localized_projection
    };
    let projection_hash = hash_text(&rendered_markdown);
    let status = derive_roster_status(&meta, &entries, &rendered_markdown, &current_projection)?;
    let (identity_revision, aliases) = if has_identity_schema {
        let identity_revision: i64 = conn
            .query_row(
                "SELECT revision FROM character_identity_meta WHERE id = 'main'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let aliases: Vec<CharacterRosterAlias> = {
            let mut stmt = conn
                .prepare(
                    "SELECT a.character_id, a.name, a.source_key, a.valid_from, a.valid_through \
                     FROM character_aliases a \
                     JOIN characters c ON c.character_id = a.character_id \
                     WHERE c.retired = 0 \
                     ORDER BY a.character_id, a.name, a.source_key, a.valid_from",
                )
                .map_err(|error| error.to_string())?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(CharacterRosterAlias {
                        character_id: row.get(0)?,
                        name: row.get(1)?,
                        source_key: row.get(2)?,
                        valid_from: row.get(3)?,
                        valid_through: row.get(4)?,
                    })
                })
                .map_err(|error| error.to_string())?;
            let mut collected = Vec::new();
            for row in rows {
                collected.push(row.map_err(|error| error.to_string())?);
            }
            collected
        };
        (Some(identity_revision), Some(aliases))
    } else {
        (None, None)
    };
    Ok(CharacterRosterSnapshot {
        schema_version: CHARACTER_ROSTER_SCHEMA_VERSION,
        revision: meta.revision,
        migration_state: meta.migration_state,
        status,
        entries: identity_entries,
        rendered_markdown,
        projection_hash,
        fact_hash,
        identity_revision,
        aliases,
        name_only_fact_hash: meta.fact_hash.clone(),
        legacy_markdown: if meta.legacy_markdown.is_empty() {
            None
        } else {
            Some(meta.legacy_markdown.clone())
        },
    })
}

fn assert_read_back(
    conn: &Connection,
    expected_revision: i64,
    expected_entries: &[CharacterRosterEntry],
    expected_projection: &str,
    expected_projection_hash: &str,
    expected_fact_hash: &str,
) -> Result<CharacterRosterSnapshot, String> {
    let snapshot = read_snapshot(conn)?;
    let expected_status = if expected_entries.is_empty() {
        CharacterRosterStatus::Empty
    } else {
        CharacterRosterStatus::Ready
    };
    let entries_match =
        canonical_entries_json(&snapshot.entries)? == canonical_entries_json(expected_entries)?;
    if snapshot.revision != expected_revision
        || snapshot.migration_state != CharacterRosterMigrationState::Ready
        || snapshot.status != expected_status
        || snapshot.projection_hash != expected_projection_hash
        || snapshot.fact_hash != expected_fact_hash
        || snapshot.rendered_markdown != expected_projection
        || !entries_match
    {
        return Err("角色名单提交回读校验失败".to_string());
    }
    let characters_arch: Option<String> = conn
        .query_row(
            "SELECT characters_arch FROM project_core WHERE id = 'main'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if characters_arch.as_deref().unwrap_or("") != expected_projection {
        return Err("角色图谱投影回读校验失败".to_string());
    }
    Ok(snapshot)
}

/// `db:character-roster-read` —— 读取角色名单快照（含一致性判定）
pub fn read(conn: &Connection) -> Result<CharacterRosterSnapshot, String> {
    migrate_character_roster_schema(conn).map_err(|error| error.to_string())?;
    read_snapshot(conn)
}

/// `db:character-roster-commit` —— 名单的唯一提交 seam
///
/// 外部输入保持 `serde_json::Value`（而非强类型）：校验与错误文案与基线
/// `normalizeRequest` 逐条对齐，确保失败信息仍与 Electron 一致。
/// 对齐基线 commit 的可选 assertGenerationSources 钩子（生成源校验，事务内生效前调用）。
pub fn commit(
    conn: &Connection,
    payload: &serde_json::Value,
    assert_generation_sources: Option<&dyn Fn()>,
) -> Result<CharacterRosterCommitReceipt, String> {
    migrate_character_roster_schema(conn).map_err(|error| error.to_string())?;
    // 身份 schema 下名单写路径由 `character_identity_repository` 承载
    // （对齐基线 `commit` 在 `ensureCharacterRosterSchema` 之后的
    // `CHARACTER_ID_WRITE_REQUIRED` guard）
    if characters::has_character_identity_schema(conn) {
        return Err("CHARACTER_ID_WRITE_REQUIRED".to_string());
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let receipt = commit_in_transaction(&tx, payload, assert_generation_sources)?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(receipt)
}

/// 事务体内的提交逻辑（批次 G3a 抽取）：供 [`commit`] 与
/// `import_global_facts_repository` 的跨仓原子事务复用。
/// 错误提前返回时 `tx` drop 即回滚，与基线 `db.transaction()` 语义一致。
/// 对齐基线 commit 的可选 assertGenerationSources 钩子（生成源校验，事务内生效前调用）。
pub(crate) fn commit_in_transaction(
    tx: &Transaction<'_>,
    payload: &serde_json::Value,
    assert_generation_sources: Option<&dyn Fn()>,
) -> Result<CharacterRosterCommitReceipt, String> {
    let request = normalize_request(payload)?;
    let request_payload_hash = payload_hash(&request)?;

    let existing_operation: Option<String> = tx
        .query_row(
            "SELECT payload_hash FROM character_roster_operations WHERE operation_id = ?1",
            [request.operation_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some(existing_payload_hash) = existing_operation {
        if existing_payload_hash != request_payload_hash {
            return Err("操作 ID 已被用于不同的角色名单，已拒绝覆盖".to_string());
        }
        // 幂等 replay 只是「该操作已被观察到」的无写入查询，不能把历史
        // committed_revision 冒充为当前事实；返回读取时的完整当前快照。
        let snapshot = read_snapshot(&tx)?;
        return Ok(CharacterRosterCommitReceipt {
            operation_id: request.operation_id.clone(),
            payload_hash: request_payload_hash,
            revision: snapshot.revision,
            idempotent: true,
            snapshot,
        });
    }

    if let Some(assert_sources) = assert_generation_sources {
        assert_sources();
    }

    let meta = read_meta(&tx)?;
    if request.expected_revision != meta.revision {
        return Err("角色名单 revision 已过期，已拒绝覆盖".to_string());
    }
    let existing_entries: Vec<CharacterRosterEntry> = characters::get_all(&tx)?
        .iter()
        .map(entry_from_character)
        .collect();
    let current_snapshot = read_snapshot(&tx)?;
    let intent = request.intent;
    let may_safely_regenerate = intent == CharacterRosterCommitIntent::ArchitectureGeneration;
    let is_legacy_repair = intent == CharacterRosterCommitIntent::LegacyRepair;
    let is_legacy_cards_adoption = intent == CharacterRosterCommitIntent::LegacyCardsAdoption;
    let is_manual_edit = is_manual_edit_intent(intent);
    let is_incremental = intent == CharacterRosterCommitIntent::BlueprintSync
        || intent == CharacterRosterCommitIntent::ChapterProgress;
    let is_novel_import = intent == CharacterRosterCommitIntent::NovelImport;

    if current_snapshot.status == CharacterRosterStatus::LegacyRepairRequired && !is_legacy_repair {
        return Err("检测到旧角色图谱且没有角色卡；只能通过显式旧角色图谱修复写入".to_string());
    }
    if current_snapshot.status == CharacterRosterStatus::Inconsistent && !is_legacy_cards_adoption {
        if meta.migration_state == CharacterRosterMigrationState::LegacyCardsPreserved {
            return Err("已有角色数据受到保护；请使用后续的显式迁移或编辑流程".to_string());
        }
        return Err("角色名单状态不一致，已拒绝覆盖；请保留原数据并联系支持".to_string());
    }
    if is_legacy_repair {
        if current_snapshot.status != CharacterRosterStatus::LegacyRepairRequired
            || !existing_entries.is_empty()
        {
            return Err("当前项目不需要旧角色图谱修复，已拒绝覆盖".to_string());
        }
        if request.expected_legacy_markdown.as_deref() != Some(meta.legacy_markdown.as_str()) {
            return Err("旧角色图谱已变更，已拒绝将过期修复结果写入项目".to_string());
        }
    } else if is_legacy_cards_adoption {
        if meta.migration_state != CharacterRosterMigrationState::LegacyCardsPreserved
            || existing_entries.is_empty()
        {
            return Err("当前项目没有可安全采用的既有角色卡，已拒绝重建图谱".to_string());
        }
        if request.expected_legacy_markdown.as_deref() != Some(meta.legacy_markdown.as_str()) {
            return Err("旧角色图谱已变更，已拒绝使用过期快照重建图谱".to_string());
        }
        let candidate_names: HashSet<&str> = request
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        let existing_names: HashSet<&str> = existing_entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        if candidate_names.len() != existing_names.len()
            || candidate_names
                .iter()
                .any(|name| !existing_names.contains(name))
        {
            return Err("既有角色卡已变更，已拒绝使用过期快照重建图谱".to_string());
        }
    } else if !may_safely_regenerate
        && !is_manual_edit
        && !is_incremental
        && !is_novel_import
        && (meta.migration_state != CharacterRosterMigrationState::Empty
            || !existing_entries.is_empty())
    {
        return Err("已有角色数据受到保护；请使用后续的显式迁移或编辑流程".to_string());
    }

    let mut rename_by_original: HashMap<String, String> = HashMap::new();
    let committed_entries = if is_legacy_cards_adoption {
        existing_entries.clone()
    } else if is_manual_edit {
        let (entries, renames) = resolve_manual_entries(&request, &existing_entries)?;
        rename_by_original = renames;
        entries
    } else if may_safely_regenerate || is_novel_import {
        merge_generated_entries_with_existing(&request.entries, &existing_entries)?
    } else if is_incremental {
        merge_incremental_entries_with_existing(
            &tx,
            &request.entries,
            &existing_entries,
            intent,
            request.source.as_ref(),
        )?
    } else {
        request.entries.clone()
    };

    // m02：新建角色（character_id 缺省）在此分配身份 UUID。
    // character_id 进入条目规范 JSON，必须先于投影/事实哈希分配，
    // 写入与回读校验（assert_read_back）才一致。
    let mut committed_entries = committed_entries;
    for entry in &mut committed_entries {
        if entry
            .character_id
            .as_deref()
            .map(str::is_empty)
            .unwrap_or(true)
        {
            entry.character_id = Some(crate::project_access::random_uuid_v4());
        }
    }
    let writing_language = project_core::get(&tx)?
        .map(|core| core.writing_language)
        .unwrap_or_else(|| project_core::DEFAULT_WRITING_LANGUAGE.to_string());
    let projection = render_character_roster_markdown(&committed_entries, &writing_language);
    let projection_hash = hash_text(&projection);
    let fact_hash = full_fact_hash(&committed_entries)?;
    let next_revision = meta.revision + 1;

    // adoption 的唯一职责是以已有结构化卡片重建只读投影。它不能重写 cards 表，
    // 否则“采用已有卡片”会变成一次隐式数据迁移。
    if is_manual_edit {
        // 手工保存提交的是完整名单快照：先清空再回填使删除、改名（含交换）
        // 与资料变更受同一事务保护。
        tx.execute("DELETE FROM characters", [])
            .map_err(|error| error.to_string())?;
        for entry in &committed_entries {
            characters::upsert(&tx, &character_from_entry(entry))?;
        }
        let final_names: HashSet<String> = committed_entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        update_blueprint_references_for_manual_edit(&tx, &rename_by_original, &final_names)?;
    } else if !is_legacy_cards_adoption {
        for entry in &committed_entries {
            characters::upsert(&tx, &character_from_entry(entry))?;
        }
    }

    let core_update = tx
        .execute(
            "UPDATE project_core SET characters_arch = ?1 WHERE id = 'main'",
            [&projection],
        )
        .map_err(|error| error.to_string())?;
    if core_update != 1 {
        return Err("项目主台账未初始化，已拒绝提交角色名单".to_string());
    }
    tx.execute(
        "UPDATE character_roster_meta SET revision = ?1, migration_state = 'ready', \
         projection_hash = ?2, fact_hash = ?3, updated_at = datetime('now') WHERE id = 'main'",
        rusqlite::params![next_revision, projection_hash, fact_hash],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "INSERT INTO character_roster_operations (\
           operation_id, payload_hash, committed_revision, projection_hash\
         ) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            request.operation_id,
            request_payload_hash,
            next_revision,
            projection_hash
        ],
    )
    .map_err(|error| error.to_string())?;

    let snapshot = assert_read_back(
        &tx,
        next_revision,
        &committed_entries,
        &projection,
        &projection_hash,
        &fact_hash,
    )?;
    Ok(CharacterRosterCommitReceipt {
        operation_id: request.operation_id.clone(),
        payload_hash: request_payload_hash,
        revision: next_revision,
        idempotent: false,
        snapshot,
    })
}

// ---------------------------------------------------------------------------
// 作者退休路径（project_clear creativeFields）
// ---------------------------------------------------------------------------
// Port of electron/services/legacy-roster-source.ts + character-roster-author.ts
// + character-roster-repository.ts:855-890 (clearLegacyCharacterArchitecture /
// refreshCharacterIdentityProjection)。
// 事务说明：rusqlite 不支持嵌套 BEGIN，而
// character_identity_repository::commit_character_identities 内部自开事务，
// 故 commit_author_character_roster 拆为三阶段（见函数注释）。

/// 手工作者角色名单请求（对齐 `AuthorRosterRequest`）。
#[derive(Debug, Clone)]
struct AuthorRosterRequest {
    operation_id: String,
    expected_revision: i64,
    expected_identity_revision: i64,
    entries: Vec<AuthorRosterEntry>,
}

/// 归一化后的手工条目（保留 `characterId` 原文，不 trim）。
#[derive(Debug, Clone)]
struct AuthorRosterEntry {
    character_id: String,
    entry: CharacterRosterEntry,
    relationships: Vec<AuthorRosterRelationship>,
}

/// 手工关系（含 `targetCharacterId` 原文；`target` 在依赖校验后回填名称）。
#[derive(Debug, Clone)]
struct AuthorRosterRelationship {
    target_character_id: String,
    target: String,
    relation: String,
}

/// 旧架构来源快照（对齐 `LegacyRosterSource`）。
#[derive(Debug, Clone)]
pub struct LegacyRosterSource {
    pub snapshot: CharacterRosterSnapshot,
    pub raw_legacy: String,
    pub legacy_hash: String,
    pub identity_revision: i64,
    pub active_ids: Vec<String>,
    pub facts_hash: String,
    pub genre: Option<String>,
    pub writing_language: String,
}

/// JS `Boolean(value)` 语义（null / false / 0 / "" 为 falsy）。
fn js_truthy(value: Option<&serde_json::Value>) -> bool {
    match value {
        None | Some(serde_json::Value::Null) | Some(serde_json::Value::Bool(false)) => false,
        Some(serde_json::Value::Bool(true)) => true,
        Some(serde_json::Value::Number(number)) => {
            number.as_f64().map(|value| value != 0.0).unwrap_or(true)
        }
        Some(serde_json::Value::String(text)) => !text.is_empty(),
        Some(serde_json::Value::Array(items)) => !items.is_empty(),
        Some(serde_json::Value::Object(map)) => !map.is_empty(),
    }
}

/// 非空字符串（`typeof value === 'string' && value.trim().length > 0`）。
fn js_nonempty(value: Option<&serde_json::Value>) -> bool {
    matches!(value,
        Some(serde_json::Value::String(text)) if !text.trim().is_empty())
}

/// 安全整数（`Number.isSafeInteger(v) && v >= 0`）。
fn js_safe_nonnegative_int(value: Option<&serde_json::Value>) -> Option<i64> {
    let number = value?.as_i64()?;
    (number >= 0 && number <= MAX_SAFE_INTEGER).then_some(number)
}

/// 关系键（对齐 `relationKey`）：`JSON.stringify([from, to, text])`。
fn relation_key(from: &str, to: &str, text: &str) -> String {
    serde_json::json!([from, to, text]).to_string()
}

/// 草稿选择键正则（`^draft:<UUID v4>$`，忽略大小写）。
static DRAFT_ID_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("(?i)^draft:[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$")
        .expect("draft id regex")
});

fn is_draft_id(character_id: &str) -> bool {
    DRAFT_ID_PATTERN.is_match(character_id)
}

/// 从角色表读取某角色当前生效的关系对 `(target_character_id, relation)`
/// （与 `identity_projection_entries` 的查询一致：目标未退休、按 relationship_id 排序）。
fn active_relationship_pairs(
    conn: &Connection,
    source_character_id: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT r.target_character_id, r.relation FROM character_relationships r 
             JOIN characters c ON c.character_id = r.target_character_id 
             WHERE r.source_character_id = ?1 AND c.retired = 0 ORDER BY r.relationship_id",
        )
        .map_err(|error| format!("读取角色关系失败: {error}"))?;
    let rows = stmt
        .query_map([source_character_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("读取角色关系失败: {error}"))?;
    let mut pairs = Vec::new();
    for row in rows {
        pairs.push(row.map_err(|error| format!("读取角色关系失败: {error}"))?);
    }
    Ok(pairs)
}

/// 刷新身份投影（对齐 `refreshCharacterIdentityProjection`）。
/// 必须在事务内调用。
pub fn refresh_character_identity_projection(conn: &Connection) -> Result<(), String> {
    if conn.is_autocommit() || !characters::has_character_identity_schema(conn) {
        return Err("CHARACTER_ID_TRANSACTION_REQUIRED".to_string());
    }
    let entries = identity_projection_entries(conn, false)?;
    let writing_language: String = conn
        .query_row(
            "SELECT writing_language FROM project_core WHERE id = 'main'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("读取项目核心配置失败: {error}"))?
        .filter(|value| value == "en-US")
        .unwrap_or_else(|| project_core::DEFAULT_WRITING_LANGUAGE.to_string());
    let projection = render_character_roster_markdown(&entries, &writing_language);
    conn.execute(
        "UPDATE project_core SET characters_arch = ?1 WHERE id = 'main'",
        [&projection],
    )
    .map_err(|error| format!("更新项目核心配置失败: {error}"))?;
    conn.execute(
        "UPDATE character_roster_meta SET revision = revision + 1, 
         migration_state = 'ready', projection_hash = ?1, fact_hash = ?2, 
         updated_at = datetime('now') WHERE id = 'main'",
        rusqlite::params![hash_text(&projection), full_fact_hash(&entries)?],
    )
    .map_err(|error| format!("更新角色名单元数据失败: {error}"))?;
    Ok(())
}

/// 清空旧架构文本（对齐 `clearLegacyCharacterArchitecture`）。
/// 必须在事务内调用。
pub fn clear_legacy_character_architecture(conn: &Connection) -> Result<(), String> {
    if conn.is_autocommit() {
        return Err("CHARACTER_ID_TRANSACTION_REQUIRED".to_string());
    }
    conn.execute(
        "UPDATE character_roster_meta SET legacy_markdown = '' WHERE id = 'main'",
        [],
    )
    .map_err(|error| format!("清空旧架构文本失败: {error}"))?;
    refresh_character_identity_projection(conn)
}

/// 读取旧架构来源（对齐 `readLegacyRosterSource`）。
pub fn read_legacy_roster_source(conn: &Connection) -> Result<LegacyRosterSource, String> {
    if !characters::has_character_identity_schema(conn) {
        return Err("LEGACY_ROSTER_ID_SCHEMA_REQUIRED".to_string());
    }
    let core: Option<(Option<String>, String)> = conn
        .query_row(
            "SELECT genre, writing_language FROM project_core WHERE id = 'main'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("读取项目核心配置失败: {error}"))?;
    let identity_revision: Option<i64> = conn
        .query_row(
            "SELECT revision FROM character_identity_meta WHERE id = 'main'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("读取身份元数据失败: {error}"))?;
    let raw_legacy: Option<String> = conn
        .query_row(
            "SELECT legacy_markdown FROM character_roster_meta WHERE id = 'main'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("读取角色名单元数据失败: {error}"))?;
    let (genre, core_writing_language) = core.ok_or("LEGACY_ROSTER_SOURCE_INVALID")?;
    let identity_revision = identity_revision.ok_or("LEGACY_ROSTER_SOURCE_INVALID")?;
    let raw_legacy = raw_legacy.ok_or("LEGACY_ROSTER_SOURCE_INVALID")?;

    let mut characters: Vec<(String, i64)> = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT character_id, retired FROM characters ORDER BY character_id")
            .map_err(|error| format!("读取角色失败: {error}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|error| format!("读取角色失败: {error}"))?;
        for row in rows {
            let (character_id, retired): (String, i64) =
                row.map_err(|error| format!("读取角色失败: {error}"))?;
            if character_id.trim().is_empty() {
                return Err("LEGACY_ROSTER_ID_INVALID".to_string());
            }
            characters.push((character_id, retired));
        }
    }
    let active_ids: Vec<String> = characters
        .iter()
        .filter(|(_, retired)| *retired == 0)
        .map(|(character_id, _)| character_id.clone())
        .collect();

    let map_rows = |table: &str,
                    order: &str|
     -> Result<Vec<serde_json::Map<String, serde_json::Value>>, String> {
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
            .map_err(|error| format!("读取 {table} 失败: {error}"))?;
        let columns: Vec<String> = stmt
            .column_names()
            .iter()
            .map(|name| name.to_string())
            .collect();
        let rows = stmt
            .query_map([], |row| {
                let mut map = serde_json::Map::new();
                for (index, name) in columns.iter().enumerate() {
                    let value: rusqlite::types::Value = row.get(index)?;
                    map.insert(
                        name.clone(),
                        match value {
                            rusqlite::types::Value::Null => serde_json::Value::Null,
                            rusqlite::types::Value::Integer(number) => serde_json::json!(number),
                            rusqlite::types::Value::Real(number) => serde_json::json!(number),
                            rusqlite::types::Value::Text(text) => serde_json::json!(text),
                            rusqlite::types::Value::Blob(blob) => serde_json::json!(blob),
                        },
                    );
                }
                Ok(map)
            })
            .map_err(|error| format!("读取 {table} 失败: {error}"))?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row.map_err(|error| format!("读取 {table} 失败: {error}"))?);
        }
        Ok(items)
    };
    let facts = serde_json::json!({
        "characters": map_rows("characters", "character_id")?,
        "aliases": map_rows("character_aliases", "character_id, name, source_key, valid_from")?,
        "relationships": map_rows("character_relationships", "relationship_id")?,
    });

    let legacy_hash = hash_text(&raw_legacy);

    Ok(LegacyRosterSource {
        snapshot: read(conn)?,
        raw_legacy,
        legacy_hash,
        identity_revision,
        active_ids,
        facts_hash: hash_text(&facts.to_string()),
        genre,
        writing_language: resolve_writing_language(&core_writing_language),
    })
}

/// 书写语言归一化（对齐 `resolveWritingLanguage`）：仅接受 zh-CN / en-US。
fn resolve_writing_language(value: &str) -> String {
    match value {
        "zh-CN" | "en-US" => value.to_string(),
        _ => project_core::DEFAULT_WRITING_LANGUAGE.to_string(),
    }
}

/// 旧卡片收养（对齐 `adoptLegacyCards`）。内部自开事务（上游 `db.transaction()`）。
pub fn adopt_legacy_cards(
    conn: &Connection,
    request: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let object = request
        .as_object()
        .ok_or("LEGACY_ROSTER_ADOPTION_INVALID")?;
    let allowed = [
        "operationId",
        "expectedRevision",
        "expectedLegacyHash",
        "expectedIdentityRevision",
        "expectedFactsHash",
    ];
    if object.keys().any(|key| !allowed.contains(&key.as_str()))
        || !js_nonempty(object.get("operationId"))
        || object
            .get("operationId")
            .and_then(serde_json::Value::as_str)
            .map(|value| value.len() > 200)
            .unwrap_or(true)
        || js_safe_nonnegative_int(object.get("expectedRevision")).is_none()
        || js_safe_nonnegative_int(object.get("expectedIdentityRevision")).is_none()
        || !is_64_hex(object.get("expectedLegacyHash"))
        || !is_64_hex(object.get("expectedFactsHash"))
    {
        return Err("LEGACY_ROSTER_ADOPTION_INVALID".to_string());
    }
    if !characters::has_character_identity_schema(conn) {
        return Err("LEGACY_ROSTER_ID_SCHEMA_REQUIRED".to_string());
    }
    let original = serde_json::json!({
        "operationId": object.get("operationId").cloned().unwrap_or(serde_json::Value::Null),
        "expectedRevision": object.get("expectedRevision").cloned().unwrap_or(serde_json::Value::Null),
        "expectedLegacyHash": object.get("expectedLegacyHash").cloned().unwrap_or(serde_json::Value::Null),
        "expectedIdentityRevision": object.get("expectedIdentityRevision").cloned().unwrap_or(serde_json::Value::Null),
        "expectedFactsHash": object.get("expectedFactsHash").cloned().unwrap_or(serde_json::Value::Null),
    });
    let request_hash = hash_text(&original.to_string());
    let operation_id = format!(
        "legacy-cards-adoption:{}",
        object
            .get("operationId")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
    );

    if let Some((payload_hash, receipt_json)) = conn
        .query_row(
            "SELECT payload_hash, receipt_json FROM character_identity_approvals WHERE operation_id = ?1",
            [&operation_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| format!("读取审批记录失败: {error}"))?
    {
        let saved: serde_json::Value = serde_json::from_str(&receipt_json)
            .map_err(|_| "LEGACY_ROSTER_ADOPTION_RECEIPT_INVALID".to_string())?;
        let saved_receipt = saved.get("receipt").cloned().unwrap_or_default();
        let saved_request_hash = saved.get("requestHash").and_then(serde_json::Value::as_str);
        if !(saved.get("version").and_then(serde_json::Value::as_i64) == Some(1)
            && saved.get("kind").and_then(serde_json::Value::as_str) == Some("legacy-cards-adoption")
            && saved_request_hash == Some(payload_hash.as_str())
            && hash_text(&saved.get("request").cloned().unwrap_or_default().to_string())
                == saved_request_hash.unwrap_or_default()
            && hash_text(&saved_receipt.to_string())
                == saved
                    .get("receiptHash")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
            && saved_receipt.get("success") == Some(&serde_json::Value::Bool(true))
            && saved_receipt.get("operationId") == object.get("operationId")
            && saved_receipt
                .get("activeIds")
                .map(serde_json::Value::is_array)
                .unwrap_or(false)
            && saved_receipt
                .get("snapshot")
                .and_then(|snapshot| snapshot.get("migrationState"))
                .and_then(serde_json::Value::as_str)
                == Some("ready")
            && saved_receipt
                .get("snapshot")
                .and_then(|snapshot| snapshot.get("status"))
                .and_then(serde_json::Value::as_str)
                == Some("ready"))
        {
            return Err("LEGACY_ROSTER_ADOPTION_RECEIPT_INVALID".to_string());
        }
        if request_hash != saved_request_hash.unwrap_or_default()
            || !deep_strict_equal(
                &original,
                saved.get("request").unwrap_or(&serde_json::Value::Null),
            )
        {
            return Err("LEGACY_ROSTER_ADOPTION_NONCE_CONFLICT".to_string());
        }
        return Ok(saved_receipt);
    }

    let source = read_legacy_roster_source(conn)?;
    let roster = read(conn)?;
    if roster.migration_state != CharacterRosterMigrationState::LegacyCardsPreserved
        || roster.status != CharacterRosterStatus::Inconsistent
        || source.active_ids.is_empty()
    {
        return Err("LEGACY_ROSTER_CARDS_REQUIRED".to_string());
    }
    let expected_revision = js_safe_nonnegative_int(object.get("expectedRevision"))
        .ok_or("LEGACY_ROSTER_ADOPTION_INVALID")?;
    let expected_identity_revision =
        js_safe_nonnegative_int(object.get("expectedIdentityRevision"))
            .ok_or("LEGACY_ROSTER_ADOPTION_INVALID")?;
    if roster.revision != expected_revision
        || roster.identity_revision != Some(expected_identity_revision)
        || source.legacy_hash
            != object
                .get("expectedLegacyHash")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
        || source.facts_hash
            != object
                .get("expectedFactsHash")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
    {
        return Err("LEGACY_ROSTER_ADOPTION_SOURCE_CHANGED".to_string());
    }

    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败: {error}"))?;
    refresh_character_identity_projection(&tx)?;
    let snapshot = read(&tx)?;
    if snapshot.status != CharacterRosterStatus::Ready
        || snapshot.migration_state != CharacterRosterMigrationState::Ready
        || snapshot.legacy_markdown
            != (if source.raw_legacy.is_empty() {
                None
            } else {
                Some(source.raw_legacy.clone())
            })
    {
        return Err("LEGACY_ROSTER_ADOPTION_READBACK_INVALID".to_string());
    }
    let receipt = serde_json::json!({
        "success": true,
        "operationId": object.get("operationId").cloned().unwrap_or(serde_json::Value::Null),
        "snapshot": snapshot,
        "activeIds": source.active_ids,
    });
    let envelope = serde_json::json!({
        "version": 1,
        "kind": "legacy-cards-adoption",
        "request": original,
        "requestHash": request_hash,
        "receipt": receipt,
        "receiptHash": hash_text(&receipt.to_string()),
    });
    tx.execute(
        "INSERT INTO character_identity_approvals (operation_id, payload_hash, receipt_json) VALUES (?1, ?2, ?3)",
        rusqlite::params![operation_id, request_hash, envelope.to_string()],
    )
    .map_err(|error| format!("写入审批记录失败: {error}"))?;
    tx.commit()
        .map_err(|error| format!("提交事务失败: {error}"))?;
    Ok(receipt)
}

/// 64 位十六进制（对齐上游 64-hex 校验）。
fn is_64_hex(value: Option<&serde_json::Value>) -> bool {
    matches!(value,
        Some(serde_json::Value::String(text)) if text.len() == 64 && text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
}

/// `isDeepStrictEqual` 的 JSON 近似（深度相等，数组顺序敏感）。
fn deep_strict_equal(left: &serde_json::Value, right: &serde_json::Value) -> bool {
    match (left, right) {
        (serde_json::Value::Array(left_items), serde_json::Value::Array(right_items)) => {
            left_items.len() == right_items.len()
                && left_items
                    .iter()
                    .zip(right_items.iter())
                    .all(|(left_item, right_item)| deep_strict_equal(left_item, right_item))
        }
        (serde_json::Value::Object(left_map), serde_json::Value::Object(right_map)) => {
            left_map.len() == right_map.len()
                && left_map.iter().all(|(key, value)| {
                    right_map
                        .get(key)
                        .map(|right_value| deep_strict_equal(value, right_value))
                        .unwrap_or(false)
                })
        }
        _ => left == right,
    }
}

/// 手工角色名单提交（对齐 `commitAuthorCharacterRoster`）。
///
/// 事务说明（与上游单事务的偏差）：rusqlite 不支持嵌套 BEGIN，而
/// `character_identity_repository::commit_character_identities` 内部自开事务，
/// 故拆为三阶段——
/// 1. 阶段一（无事务）：守卫 / 归一化 / 幂等读回 / 版本校验 / 身份请求构造；
/// 2. 阶段二：`commit_character_identities`（自开事务，写身份表）；
/// 3. 阶段三（自开事务）：过期关系删除 / 角色字段更新 / 投影刷新 / 读回校验 /
///    审批信封写入。
pub fn commit_author_character_roster(
    conn: &Connection,
    candidate: &serde_json::Value,
    project_id: &str,
    epoch: &str,
    assert_current: impl Fn(),
) -> Result<serde_json::Value, String> {
    if !characters::has_character_identity_schema(conn)
        || project_id.trim().is_empty()
        || epoch.trim().is_empty()
    {
        return Err("CHARACTER_AUTHOR_SCOPE_REQUIRED".to_string());
    }
    let request = normalize_author_request(candidate)?;
    let request_hash = hash_text(
        &serde_json::json!({ "projectId": project_id, "request": candidate }).to_string(),
    );
    let operation_key = format!("author-roster-request:{}", request.operation_id);
    let approval_id = format!("author-roster-effect:{}", request.operation_id);

    // ---- 阶段一：幂等读回 + 版本校验 + 身份请求构造（无事务） ----
    assert_current();
    if let Some((payload_hash, receipt_json)) = conn
        .query_row(
            "SELECT payload_hash, receipt_json FROM character_identity_approvals WHERE operation_id = ?1",
            [&operation_key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| format!("读取审批记录失败: {error}"))?
    {
        if payload_hash != request_hash {
            return Err("CHARACTER_APPROVAL_NONCE_CONFLICT".to_string());
        }
        let saved: serde_json::Value = serde_json::from_str(&receipt_json)
            .map_err(|_| "CHARACTER_AUTHOR_RECEIPT_INVALID".to_string())?;
        let approval: Option<(String, String)> = conn
            .query_row(
                "SELECT payload_hash, receipt_json FROM character_identity_approvals WHERE operation_id = ?1",
                [&approval_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| format!("读取审批记录失败: {error}"))?;
        let approval = approval.ok_or("CHARACTER_AUTHOR_RECEIPT_INVALID")?;
        let saved_receipt = saved.get("receipt").cloned().unwrap_or_default();
        if !(saved.get("version").and_then(serde_json::Value::as_i64) == Some(1)
            && saved.get("kind").and_then(serde_json::Value::as_str) == Some("author-roster")
            && saved.get("requestHash").and_then(serde_json::Value::as_str) == Some(request_hash.as_str())
            && hash_text(&saved_receipt.to_string())
                == saved
                    .get("receiptHash")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
            && saved.get("approvalId").and_then(serde_json::Value::as_str) == Some(approval_id.as_str())
            && saved_receipt.get("operationId") == Some(&serde_json::Value::String(request.operation_id.clone()))
            && saved_receipt.get("payloadHash").and_then(serde_json::Value::as_str) == Some(request_hash.as_str())
            && saved.get("approvalPayloadHash").and_then(serde_json::Value::as_str) == Some(approval.0.as_str())
            && hash_text(&approval.1)
                == saved
                    .get("approvalReceiptHash")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default())
        {
            return Err("CHARACTER_AUTHOR_RECEIPT_INVALID".to_string());
        }
        let snapshot = read(conn)?;
        let mut receipt = saved_receipt;
        receipt["idempotent"] = serde_json::Value::Bool(true);
        receipt["revision"] = serde_json::json!(snapshot.revision);
        receipt["snapshot"] =
            serde_json::to_value(&snapshot).map_err(|_| "CHARACTER_AUTHOR_RECEIPT_INVALID".to_string())?;
        return Ok(receipt);
    }

    let current = read(conn)?;
    if current.revision != request.expected_revision
        || current.identity_revision != Some(request.expected_identity_revision)
    {
        return Err("CHARACTER_ID_REVISION_CONFLICT".to_string());
    }
    if !matches!(
        current.status,
        CharacterRosterStatus::Ready | CharacterRosterStatus::Empty
    ) {
        return Err("CHARACTER_AUTHOR_ROSTER_NOT_READY".to_string());
    }
    let original: HashMap<&str, &CharacterRosterEntry> = current
        .entries
        .iter()
        .filter_map(|entry| {
            entry
                .character_id
                .as_deref()
                .map(|character_id| (character_id, entry))
        })
        .collect();
    let kept: HashSet<&str> = request
        .entries
        .iter()
        .filter(|entry| !is_draft_id(&entry.character_id))
        .map(|entry| entry.character_id.as_str())
        .collect();
    for entry in &request.entries {
        if !original.contains_key(entry.character_id.as_str()) && !is_draft_id(&entry.character_id)
        {
            return Err("CHARACTER_ID_UNKNOWN".to_string());
        }
    }

    let mut identity = serde_json::json!({
        "approval": {
            "operationId": approval_id,
            "expectedRevision": current.identity_revision,
            "action": "author-edit",
            "source": {
                "kind": "author",
                "source": {
                    "projectId": project_id,
                    "epoch": epoch,
                    "sourceId": operation_key,
                    "revision": current.identity_revision,
                    "contentHash": request_hash,
                },
            },
        },
        "changes": [],
        "creations": [],
        "retireIds": original.keys().copied().collect::<Vec<_>>().into_iter()
            .filter(|character_id| !kept.contains(*character_id))
            .collect::<Vec<_>>(),
        "relationships": [],
        "resolutions": [],
    });
    let fields = [
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
    for entry in &request.entries {
        let before = original.get(entry.character_id.as_str()).copied();
        let mut changed = serde_json::Map::new();
        for field in fields {
            if before.is_none()
                || entry_field(&entry.entry, field)
                    != before.and_then(|before_entry| entry_field(before_entry, field))
            {
                if let Some(value) = entry_field_value(&entry.entry, field) {
                    changed.insert(field.to_string(), value);
                }
            }
        }
        if before.is_none() {
            let mut creation_fields = changed.clone();
            creation_fields.insert("name".to_string(), serde_json::json!(entry.entry.name));
            identity["creations"]
                .as_array_mut()
                .expect("array")
                .push(serde_json::json!({
                    "selectionKey": entry.character_id,
                    "fields": creation_fields,
                }));
        } else if !changed.is_empty() {
            identity["changes"]
                .as_array_mut()
                .expect("array")
                .push(serde_json::json!({
                    "characterId": entry.character_id,
                    "fields": changed,
                }));
        }
    }

    let existing_relations: Vec<(String, String, String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT r.relationship_id, r.source_character_id, r.target_character_id, r.relation 
                 FROM character_relationships r 
                 JOIN characters a ON a.character_id = r.source_character_id 
                 JOIN characters b ON b.character_id = r.target_character_id 
                 WHERE a.retired = 0 AND b.retired = 0",
            )
            .map_err(|error| format!("读取角色关系失败: {error}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|error| format!("读取角色关系失败: {error}"))?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row.map_err(|error| format!("读取角色关系失败: {error}"))?);
        }
        items
    };
    let mut desired: HashSet<String> = HashSet::new();
    for entry in &request.entries {
        for relationship in &entry.relationships {
            desired.insert(relation_key(
                &entry.character_id,
                &relationship.target_character_id,
                &relationship.relation,
            ));
        }
    }
    let previous_keys: HashSet<String> = existing_relations
        .iter()
        .map(|(_, source, target, relation)| relation_key(source, target, relation))
        .collect();
    let stale_relations: Vec<String> = existing_relations
        .into_iter()
        .filter(|(_, source, target, relation)| {
            let key = relation_key(source, target, relation);
            kept.contains(source.as_str())
                && kept.contains(target.as_str())
                && !desired.contains(&key)
        })
        .map(|(relationship_id, _, _, _)| relationship_id)
        .collect();
    for entry in &request.entries {
        for relationship in &entry.relationships {
            let key = relation_key(
                &entry.character_id,
                &relationship.target_character_id,
                &relationship.relation,
            );
            if previous_keys.contains(&key) {
                continue;
            }
            let mut link = serde_json::Map::new();
            if is_draft_id(&entry.character_id) {
                link.insert(
                    "sourceSelectionKey".to_string(),
                    serde_json::json!(entry.character_id),
                );
            } else {
                link.insert(
                    "sourceCharacterId".to_string(),
                    serde_json::json!(entry.character_id),
                );
            }
            if is_draft_id(&relationship.target_character_id) {
                link.insert(
                    "targetSelectionKey".to_string(),
                    serde_json::json!(relationship.target_character_id),
                );
            } else {
                link.insert(
                    "targetCharacterId".to_string(),
                    serde_json::json!(relationship.target_character_id),
                );
            }
            link.insert(
                "relation".to_string(),
                serde_json::json!(relationship.relation),
            );
            identity["relationships"]
                .as_array_mut()
                .expect("array")
                .push(serde_json::Value::Object(link));
        }
    }

    // ---- 阶段二：身份提交（自开事务） ----
    let committed =
        crate::repositories::character_identity_repository::commit_character_identities(
            conn,
            &identity,
            |_| {
                assert_current();
                true
            },
        )?;
    let created: HashMap<&str, &str> = committed
        .created
        .iter()
        .map(|item| (item.selection_key.as_str(), item.character_id.as_str()))
        .collect();

    // ---- 阶段三：角色字段更新 + 投影刷新 + 读回校验 + 审批信封 ----
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败: {error}"))?;
    for relationship_id in &stale_relations {
        tx.execute(
            "DELETE FROM character_relationships WHERE relationship_id = ?1",
            [relationship_id],
        )
        .map_err(|error| format!("删除过期关系失败: {error}"))?;
    }
    for entry in &request.entries {
        let id = created
            .get(entry.character_id.as_str())
            .copied()
            .unwrap_or(entry.character_id.as_str());
        let before = original.get(entry.character_id.as_str()).copied();
        let before_pairs: Vec<(String, String)> =
            match before.and_then(|before_entry| before_entry.character_id.as_deref()) {
                Some(before_id) => active_relationship_pairs(&tx, before_id)?,
                None => Vec::new(),
            };
        let entry_pairs: Vec<(String, String)> = entry
            .relationships
            .iter()
            .map(|relationship| {
                (
                    relationship.target_character_id.clone(),
                    relationship.relation.clone(),
                )
            })
            .collect();
        let relation_keys = |pairs: &[(String, String)]| -> String {
            let mut keys: Vec<String> = pairs
                .iter()
                .map(|(target, relation)| serde_json::json!([target, relation]).to_string())
                .collect();
            keys.sort();
            keys.join("\n")
        };
        let hidden_notes = if before
            .and_then(|before_entry| before_entry.legacy_relationship_notes.as_deref())
            .map(|notes| !notes.is_empty())
            .unwrap_or(false)
            && !before_pairs.is_empty()
            && relation_keys(&before_pairs) == relation_keys(&entry_pairs)
        {
            before
                .and_then(|before_entry| before_entry.legacy_relationship_notes.clone())
                .unwrap_or_default()
        } else {
            String::new()
        };
        let legacy_notes = entry
            .entry
            .legacy_relationship_notes
            .clone()
            .unwrap_or(hidden_notes);
        let before_notes = before
            .and_then(|before_entry| before_entry.legacy_relationship_notes.clone())
            .unwrap_or_default();
        if legacy_notes != before_notes {
            tx.execute(
                "UPDATE characters SET relationships = ?1 WHERE character_id = ?2",
                rusqlite::params![legacy_notes, id],
            )
            .map_err(|error| format!("更新角色关系文本失败: {error}"))?;
        }
        if let Some(state) = &entry.entry.current_state {
            let old_state = before.and_then(|before_entry| before_entry.current_state.as_ref());
            let changed_fields: Vec<CharacterStateTextField> = CharacterStateTextField::ALL
                .into_iter()
                .filter(|field| {
                    old_state.is_none()
                        || state_text(old_state.expect("checked"), *field)
                            != state_text(state, *field)
                })
                .collect();
            let updated_at_changed =
                old_state.map(|state| state.updated_at_chapter) != Some(state.updated_at_chapter);
            if !changed_fields.is_empty() || updated_at_changed {
                let mut provenance = old_state
                    .and_then(|state| state.provenance.clone())
                    .unwrap_or_default();
                for field in &changed_fields {
                    provenance.insert(
                        field.json_name().to_string(),
                        CharacterStateFieldProvenance::Author {
                            chapter_number: state.updated_at_chapter,
                        },
                    );
                }
                tx.execute(
                    "UPDATE characters SET cs_location = ?1, cs_power_level = ?2, cs_physical_state = ?3, 
                     cs_mental_state = ?4, cs_key_items = ?5, cs_recent_events = ?6, 
                     cs_updated_at_chapter = ?7, cs_provenance = ?8 WHERE character_id = ?9",
                    rusqlite::params![
                        state.location,
                        state.power_level,
                        state.physical_state,
                        state.mental_state,
                        state.key_items,
                        state.recent_events,
                        state.updated_at_chapter,
                        serde_json::to_string(&provenance)
                            .map_err(|_| "CHARACTER_AUTHOR_READBACK_FAILED".to_string())?,
                        id,
                    ],
                )
                .map_err(|error| format!("更新角色状态失败: {error}"))?;
            }
        }
    }
    refresh_character_identity_projection(&tx)?;
    let snapshot = read(&tx)?;

    // 读回校验（对齐上游 readback gate）。
    if snapshot.identity_revision != Some(committed.revision)
        || snapshot.revision != current.revision + 1
        || !matches!(
            snapshot.status,
            CharacterRosterStatus::Ready | CharacterRosterStatus::Empty
        )
        || snapshot.entries.len() != request.entries.len()
    {
        return Err("CHARACTER_AUTHOR_READBACK_FAILED".to_string());
    }
    for entry in &request.entries {
        let observed_id = created
            .get(entry.character_id.as_str())
            .copied()
            .unwrap_or(entry.character_id.as_str());
        let observed = snapshot
            .entries
            .iter()
            .find(|item| item.character_id.as_deref() == Some(observed_id))
            .ok_or("CHARACTER_AUTHOR_READBACK_FAILED".to_string())?;
        for field in fields {
            if entry_field(&entry.entry, field) != entry_field(observed, field) {
                return Err("CHARACTER_AUTHOR_READBACK_FAILED".to_string());
            }
        }
        let mut observed_keys: Vec<String> = active_relationship_pairs(&tx, observed_id)?
            .into_iter()
            .map(|(target, relation)| relation_key(observed_id, &target, &relation))
            .collect();
        observed_keys.sort();
        let mut expected_keys: Vec<String> = entry
            .relationships
            .iter()
            .map(|relationship| {
                let target_id = created
                    .get(relationship.target_character_id.as_str())
                    .copied()
                    .unwrap_or(relationship.target_character_id.as_str());
                relation_key(observed_id, target_id, &relationship.relation)
            })
            .collect();
        expected_keys.sort();
        if observed_keys != expected_keys {
            return Err("CHARACTER_AUTHOR_READBACK_FAILED".to_string());
        }
        let expected_state = entry.entry.current_state.as_ref().or_else(|| {
            original
                .get(entry.character_id.as_str())
                .and_then(|entry| entry.current_state.as_ref())
        });
        if let Some(expected_state) = expected_state {
            let observed_state = observed
                .current_state
                .as_ref()
                .ok_or("CHARACTER_AUTHOR_READBACK_FAILED".to_string())?;
            if CharacterStateTextField::ALL
                .into_iter()
                .any(|field| state_text(observed_state, field) != state_text(expected_state, field))
            {
                return Err("CHARACTER_AUTHOR_READBACK_FAILED".to_string());
            }
        }
    }

    let receipt = serde_json::json!({
        "operationId": request.operation_id,
        "payloadHash": request_hash,
        "revision": snapshot.revision,
        "idempotent": false,
        "snapshot": snapshot,
        "created": committed.created,
    });
    let approval: (String, String) = tx
        .query_row(
            "SELECT payload_hash, receipt_json FROM character_identity_approvals WHERE operation_id = ?1",
            [&approval_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "CHARACTER_AUTHOR_READBACK_FAILED".to_string())?;
    let saved = serde_json::json!({
        "version": 1,
        "kind": "author-roster",
        "requestHash": request_hash,
        "receipt": receipt,
        "receiptHash": hash_text(&receipt.to_string()),
        "approvalId": approval_id,
        "approvalPayloadHash": approval.0,
        "approvalReceiptHash": hash_text(&approval.1),
    });
    tx.execute(
        "INSERT INTO character_identity_approvals (operation_id, payload_hash, receipt_json) VALUES (?1, ?2, ?3)",
        rusqlite::params![operation_key, request_hash, saved.to_string()],
    )
    .map_err(|error| format!("写入审批记录失败: {error}"))?;
    tx.commit()
        .map_err(|error| format!("提交事务失败: {error}"))?;
    Ok(receipt)
}

/// 归一化手工请求（对齐 `character-roster-author.ts` 的 `normalize()`）。
fn normalize_author_request(candidate: &serde_json::Value) -> Result<AuthorRosterRequest, String> {
    let object = candidate
        .as_object()
        .ok_or("CHARACTER_AUTHOR_REQUEST_INVALID")?;
    if object.get("intent").and_then(serde_json::Value::as_str) != Some("manual_edit")
        || object
            .get("schemaVersion")
            .and_then(serde_json::Value::as_i64)
            != Some(CHARACTER_ROSTER_SCHEMA_VERSION)
        || !js_nonempty(object.get("operationId"))
        || object
            .get("operationId")
            .and_then(serde_json::Value::as_str)
            .map(|value| value.len() > 200)
            .unwrap_or(true)
        || js_safe_nonnegative_int(object.get("expectedRevision")).is_none()
        || js_safe_nonnegative_int(object.get("expectedIdentityRevision")).is_none()
        || !object
            .get("entries")
            .map(serde_json::Value::is_array)
            .unwrap_or(false)
        || js_truthy(object.get("generationRunHandle"))
        || js_truthy(object.get("source"))
        || object.get("expectedLegacyMarkdown").is_some()
    {
        return Err("CHARACTER_AUTHOR_REQUEST_INVALID".to_string());
    }
    let mut entries: Vec<AuthorRosterEntry> = Vec::new();
    for raw in object
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let raw_object = raw.as_object().ok_or("CHARACTER_ID_WRITE_REQUIRED")?;
        let character_id = raw_object
            .get("characterId")
            .and_then(serde_json::Value::as_str)
            .ok_or("CHARACTER_ID_WRITE_REQUIRED")?;
        if character_id.trim().is_empty()
            || !raw_object
                .get("relationships")
                .map(serde_json::Value::is_array)
                .unwrap_or(false)
        {
            return Err("CHARACTER_ID_WRITE_REQUIRED".to_string());
        }
        let mut normalized = raw_object.clone();
        normalized.insert("relationships".to_string(), serde_json::json!([]));
        let mut entry = normalize_entry(&serde_json::Value::Object(normalized), true)?;
        entry.character_id = Some(character_id.to_string());
        for field in [
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
        ] {
            if let Some(serde_json::Value::String(text)) = raw_object.get(field) {
                set_entry_field(&mut entry, field, text.clone());
            }
        }
        if raw_object.get("legacyRelationshipNotes").is_some() {
            let legacy_notes = raw_object
                .get("legacyRelationshipNotes")
                .and_then(serde_json::Value::as_str)
                .ok_or("CHARACTER_AUTHOR_ENTRY_INVALID")?;
            entry.legacy_relationship_notes = Some(legacy_notes.to_string());
        }
        if let Some(serde_json::Value::Object(raw_state)) = raw_object.get("currentState") {
            if let Some(state) = entry.current_state.as_mut() {
                for field in CharacterStateTextField::ALL {
                    if let Some(serde_json::Value::String(text)) = raw_state.get(field.json_name())
                    {
                        set_state_field(state, field, text.clone());
                    }
                }
            }
        }
        let relationships: Vec<AuthorRosterRelationship> = raw_object
            .get("relationships")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|relationship| {
                let item = relationship
                    .as_object()
                    .ok_or("CHARACTER_RELATIONSHIP_ID_REQUIRED")?;
                let target_character_id = item
                    .get("targetCharacterId")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("CHARACTER_RELATIONSHIP_ID_REQUIRED")?;
                let relation = item
                    .get("relation")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("CHARACTER_RELATIONSHIP_ID_REQUIRED")?;
                if target_character_id.trim().is_empty() || relation.trim().is_empty() {
                    return Err("CHARACTER_RELATIONSHIP_ID_REQUIRED".to_string());
                }
                Ok(AuthorRosterRelationship {
                    target_character_id: target_character_id.to_string(),
                    target: String::new(),
                    relation: relation.to_string(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        entries.push(AuthorRosterEntry {
            character_id: character_id.to_string(),
            entry,
            relationships,
        });
    }
    for entry in &mut entries {
        entry.relationships.sort_by(|left, right| {
            serde_json::json!([left.target_character_id, left.relation])
                .to_string()
                .cmp(&serde_json::json!([right.target_character_id, right.relation]).to_string())
        });
    }
    entries.sort_by(|left, right| left.character_id.cmp(&right.character_id));
    let mut names: HashMap<String, String> = HashMap::new();
    for entry in &entries {
        if names
            .insert(entry.character_id.clone(), entry.entry.name.clone())
            .is_some()
        {
            return Err("CHARACTER_ID_DUPLICATE".to_string());
        }
    }
    for entry in &entries {
        let mut seen: HashSet<String> = HashSet::new();
        for relationship in &entry.relationships {
            let key = serde_json::json!([relationship.target_character_id, relationship.relation])
                .to_string();
            if !names.contains_key(&relationship.target_character_id)
                || relationship.target_character_id == entry.character_id
                || !seen.insert(key)
            {
                return Err("CHARACTER_RELATIONSHIP_INVALID".to_string());
            }
        }
    }
    for entry in &mut entries {
        for relationship in &mut entry.relationships {
            relationship.target = names
                .get(&relationship.target_character_id)
                .cloned()
                .unwrap_or_default();
        }
    }
    if object.get("renames").is_some() {
        let renames = object
            .get("renames")
            .and_then(serde_json::Value::as_array)
            .ok_or("CHARACTER_ID_WRITE_REQUIRED")?;
        for rename in renames {
            let item = rename.as_object().ok_or("CHARACTER_ID_WRITE_REQUIRED")?;
            let character_id = item
                .get("characterId")
                .and_then(serde_json::Value::as_str)
                .ok_or("CHARACTER_ID_WRITE_REQUIRED")?;
            let new_name = item
                .get("newName")
                .and_then(serde_json::Value::as_str)
                .ok_or("CHARACTER_ID_WRITE_REQUIRED")?;
            let target_name = names
                .get(character_id)
                .ok_or("CHARACTER_ID_WRITE_REQUIRED")?;
            if character_id.trim().is_empty() || target_name != new_name {
                return Err("CHARACTER_ID_WRITE_REQUIRED".to_string());
            }
        }
    }
    Ok(AuthorRosterRequest {
        operation_id: object
            .get("operationId")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        expected_revision: js_safe_nonnegative_int(object.get("expectedRevision"))
            .ok_or("CHARACTER_AUTHOR_REQUEST_INVALID")?,
        expected_identity_revision: js_safe_nonnegative_int(object.get("expectedIdentityRevision"))
            .ok_or("CHARACTER_AUTHOR_REQUEST_INVALID")?,
        entries,
    })
}

/// 读取条目文本字段（`before[field] !== entry[field]` 比较用）。
fn entry_field(entry: &CharacterRosterEntry, field: &str) -> Option<String> {
    match field {
        "name" => Some(entry.name.clone()),
        "role" => Some(entry.role.as_str().to_string()),
        "gender" => Some(entry.gender.clone()),
        "age" => Some(entry.age.clone()),
        "appearance" => Some(entry.appearance.clone()),
        "personality" => Some(entry.personality.clone()),
        "background" => Some(entry.background.clone()),
        "abilities" => Some(entry.abilities.clone()),
        "motivation" => Some(entry.motivation.clone()),
        "arc" => Some(entry.arc.clone()),
        "notes" => Some(entry.notes.clone()),
        _ => None,
    }
}

fn entry_field_value(entry: &CharacterRosterEntry, field: &str) -> Option<serde_json::Value> {
    entry_field(entry, field).map(serde_json::Value::String)
}

fn set_entry_field(entry: &mut CharacterRosterEntry, field: &str, value: String) {
    match field {
        "name" => entry.name = value,
        "role" => entry.role = normalize_role(&value).unwrap_or(entry.role),
        "gender" => entry.gender = value,
        "age" => entry.age = value,
        "appearance" => entry.appearance = value,
        "personality" => entry.personality = value,
        "background" => entry.background = value,
        "abilities" => entry.abilities = value,
        "motivation" => entry.motivation = value,
        "arc" => entry.arc = value,
        "notes" => entry.notes = value,
        _ => {}
    }
}

fn state_text(state: &CharacterStateData, field: CharacterStateTextField) -> String {
    match field {
        CharacterStateTextField::Location => state.location.clone(),
        CharacterStateTextField::PowerLevel => state.power_level.clone(),
        CharacterStateTextField::PhysicalState => state.physical_state.clone(),
        CharacterStateTextField::MentalState => state.mental_state.clone(),
        CharacterStateTextField::KeyItems => state.key_items.clone(),
        CharacterStateTextField::RecentEvents => state.recent_events.clone(),
    }
}

fn set_state_field(state: &mut CharacterStateData, field: CharacterStateTextField, value: String) {
    match field {
        CharacterStateTextField::Location => state.location = value,
        CharacterStateTextField::PowerLevel => state.power_level = value,
        CharacterStateTextField::PhysicalState => state.physical_state = value,
        CharacterStateTextField::MentalState => state.mental_state = value,
        CharacterStateTextField::KeyItems => state.key_items = value,
        CharacterStateTextField::RecentEvents => state.recent_events = value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;
    use serde_json::json;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        create_tables(&conn).expect("建表失败");
        // 本文件测试覆盖 legacy 写路径（与基线测试同构）：拆除身份六表，
        // 使 has_character_identity_schema 判定为无身份 schema。
        // rusqlite 默认不启用外键级联，拆除无碍。
        conn.execute_batch(
            "DROP TABLE IF EXISTS character_identity_origins;\
             DROP TABLE IF EXISTS character_aliases;\
             DROP TABLE IF EXISTS character_identity_proposals;\
             DROP TABLE IF EXISTS character_identity_approvals;\
             DROP TABLE IF EXISTS character_relationships;\
             DROP TABLE IF EXISTS character_identity_meta;",
        )
        .expect("拆除身份表失败");
        project_core::init(&conn, "测试项目", project_core::DEFAULT_WRITING_LANGUAGE)
            .expect("主台账初始化失败");
        conn
    }

    /// 全量 schema（含身份六表）：用于 guard 断言与身份形态读取测试
    fn identity_schema_db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        create_tables(&conn).expect("建表失败");
        project_core::init(&conn, "测试项目", project_core::DEFAULT_WRITING_LANGUAGE)
            .expect("主台账初始化失败");
        conn
    }

    fn entry_payload(name: &str, role: &str) -> serde_json::Value {
        json!({
            "name": name,
            "role": role,
            "gender": "",
            "age": "",
            "appearance": "",
            "personality": "",
            "background": "",
            "abilities": "",
            "motivation": "",
            "relationships": [],
            "arc": "",
            "notes": ""
        })
    }

    fn initialize_payload(
        operation_id: &str,
        expected_revision: i64,
        entries: Vec<serde_json::Value>,
    ) -> serde_json::Value {
        json!({
            "operationId": operation_id,
            "expectedRevision": expected_revision,
            "schemaVersion": 1,
            "intent": "initialize",
            "entries": entries,
        })
    }

    fn empty_entry() -> CharacterRosterEntry {
        CharacterRosterEntry {
            character_id: None,
            name: String::new(),
            role: CharacterRole::Supporting,
            gender: String::new(),
            age: String::new(),
            appearance: String::new(),
            personality: String::new(),
            background: String::new(),
            abilities: String::new(),
            motivation: String::new(),
            relationships: Vec::new(),
            arc: String::new(),
            notes: String::new(),
            current_state: None,
            legacy_relationship_notes: None,
        }
    }

    /// 往库里插一条已定稿草稿 + 发布投影（`chapter_progress` 的来源校验依据）
    fn insert_finalized_draft(
        conn: &Connection,
        chapter_number: i64,
        finalization_id: &str,
        content_hash: &str,
    ) -> i64 {
        conn.execute("INSERT INTO contents (body) VALUES ('正文')", [])
            .unwrap();
        let content_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, status, content_id) \
             VALUES (?1, 1, 'finalized', ?2)",
            rusqlite::params![chapter_number, content_id],
        )
        .unwrap();
        let draft_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO finalization_outbox (\
               finalization_id, draft_id, chapter_number, content_hash, \
               content_revision, target_file_name\
             ) VALUES (?1, ?2, ?3, ?4, 1, 'ch.md')",
            rusqlite::params![finalization_id, draft_id, chapter_number, content_hash],
        )
        .unwrap();
        draft_id
    }

    #[test]
    fn initialize_commit_then_read_ready_test() {
        let conn = memory_db();
        let receipt = commit(
            &conn,
            &initialize_payload("op-1", 0, vec![entry_payload("林清玄", "protagonist")]),
            None,
        )
        .expect("首次建档应成功");

        assert!(!receipt.idempotent);
        assert_eq!(receipt.revision, 1);
        assert_eq!(receipt.payload_hash.len(), 64);
        assert_eq!(receipt.snapshot.status, CharacterRosterStatus::Ready);
        assert_eq!(
            receipt.snapshot.migration_state,
            CharacterRosterMigrationState::Ready
        );
        assert_eq!(receipt.snapshot.entries.len(), 1);
        assert!(receipt.snapshot.rendered_markdown.contains("# 角色图谱"));

        let snapshot = read(&conn).expect("读取应成功");
        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.status, CharacterRosterStatus::Ready);
        assert_eq!(snapshot.projection_hash, receipt.snapshot.projection_hash);
        assert_eq!(snapshot.fact_hash, receipt.snapshot.fact_hash);
        assert_eq!(
            snapshot.rendered_markdown,
            receipt.snapshot.rendered_markdown
        );

        let arch: String = conn
            .query_row(
                "SELECT characters_arch FROM project_core WHERE id = 'main'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(arch, receipt.snapshot.rendered_markdown);
    }

    #[test]
    fn committing_same_operation_again_is_idempotent_test() {
        let conn = memory_db();
        let payload = initialize_payload("op-1", 0, vec![entry_payload("林清玄", "protagonist")]);
        let first = commit(&conn, &payload, None).unwrap();
        let replay = commit(&conn, &payload, None).expect("幂等 replay 应成功");

        assert!(replay.idempotent);
        assert_eq!(replay.revision, first.revision);
        assert_eq!(replay.payload_hash, first.payload_hash);
        assert_eq!(replay.snapshot.revision, replay.revision);

        let conflict = commit(
            &conn,
            &initialize_payload("op-1", 0, vec![entry_payload("苏白", "supporting")]),
            None,
        );
        assert_eq!(
            conflict.unwrap_err(),
            "操作 ID 已被用于不同的角色名单，已拒绝覆盖"
        );
    }

    #[test]
    fn stale_revision_is_rejected_test() {
        let conn = memory_db();
        commit(
            &conn,
            &initialize_payload("op-1", 0, vec![entry_payload("林清玄", "protagonist")]),
            None,
        )
        .unwrap();
        let error = commit(
            &conn,
            &initialize_payload("op-2", 0, vec![entry_payload("苏白", "supporting")]),
            None,
        )
        .unwrap_err();
        assert_eq!(error, "角色名单 revision 已过期，已拒绝覆盖");
    }

    #[test]
    fn normalization_rejects_invalid_payloads_test() {
        let conn = memory_db();
        let mut bad_version = initialize_payload("op", 0, vec![entry_payload("甲", "protagonist")]);
        bad_version["schemaVersion"] = json!(2);
        assert_eq!(
            commit(&conn, &bad_version, None).unwrap_err(),
            "角色名单 schema 版本不受支持"
        );

        let missing_operation = json!({
            "expectedRevision": 0, "schemaVersion": 1, "entries": []
        });
        assert_eq!(
            commit(&conn, &missing_operation, None).unwrap_err(),
            "操作 ID必须是文本"
        );

        let empty_entries = initialize_payload("op", 0, vec![]);
        assert_eq!(
            commit(&conn, &empty_entries, None).unwrap_err(),
            "角色名单不能为空"
        );

        let bad_intent = json!({
            "operationId": "op",
            "expectedRevision": 0,
            "schemaVersion": 1,
            "intent": "unknown",
            "entries": [entry_payload("甲", "protagonist")],
        });
        assert_eq!(
            commit(&conn, &bad_intent, None).unwrap_err(),
            "角色名单提交意图无效"
        );

        let duplicates = initialize_payload(
            "op",
            0,
            vec![
                entry_payload("甲", "protagonist"),
                entry_payload("甲", "minor"),
            ],
        );
        assert_eq!(
            commit(&conn, &duplicates, None).unwrap_err(),
            "角色名必须唯一"
        );

        let bad_role = initialize_payload("op", 0, vec![entry_payload("甲", "hero")]);
        assert_eq!(
            commit(&conn, &bad_role, None).unwrap_err(),
            "角色「甲」的定位无效"
        );

        let mut dangling = entry_payload("甲", "protagonist");
        dangling["relationships"] = json!([{"target": "不存在", "relation": "朋友"}]);
        let payload = initialize_payload("op", 0, vec![dangling]);
        assert_eq!(
            commit(&conn, &payload, None).unwrap_err(),
            "角色「甲」引用了不存在的关系目标「不存在」"
        );
    }

    #[test]
    fn manual_edit_renames_and_prunes_blueprint_references_test() {
        let conn = memory_db();
        commit(
            &conn,
            &initialize_payload(
                "op-1",
                0,
                vec![
                    entry_payload("旧名", "protagonist"),
                    entry_payload("路人", "minor"),
                ],
            ),
            None,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO blueprints (chapter_number, characters) VALUES (1, ?1)",
            rusqlite::params![serde_json::to_string(&vec!["旧名", "路人"]).unwrap()],
        )
        .unwrap();

        let payload = json!({
            "operationId": "op-2",
            "expectedRevision": 1,
            "schemaVersion": 1,
            "intent": "manual_edit",
            "entries": [entry_payload("新名", "protagonist")],
            "renames": [{"originalName": "旧名", "newName": "新名"}],
        });
        let receipt = commit(&conn, &payload, None).expect("手工保存应成功");
        assert_eq!(receipt.revision, 2);
        let names: Vec<String> = receipt
            .snapshot
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        assert_eq!(names, vec!["新名"]);

        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM characters", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 1);
        let blueprint: String = conn
            .query_row(
                "SELECT characters FROM blueprints WHERE chapter_number = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(blueprint, serde_json::to_string(&vec!["新名"]).unwrap());
    }

    #[test]
    fn manual_edit_accepts_empty_roster_test() {
        let conn = memory_db();
        commit(
            &conn,
            &initialize_payload("op-1", 0, vec![entry_payload("林清玄", "protagonist")]),
            None,
        )
        .unwrap();
        let payload = json!({
            "operationId": "op-2",
            "expectedRevision": 1,
            "schemaVersion": 1,
            "intent": "manual_edit",
            "entries": [],
        });
        let receipt = commit(&conn, &payload, None).expect("空名单应可保存");
        assert_eq!(receipt.snapshot.status, CharacterRosterStatus::Empty);
        assert_eq!(receipt.snapshot.rendered_markdown, "");
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM characters", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn chapter_progress_merges_derived_state_test() {
        let conn = memory_db();
        commit(
            &conn,
            &initialize_payload("op-1", 0, vec![entry_payload("林清玄", "protagonist")]),
            None,
        )
        .unwrap();
        let content_hash = "a".repeat(64);
        insert_finalized_draft(&conn, 1, "fin-1", &content_hash);

        let source = json!({
            "draftId": 1,
            "finalizationId": "fin-1",
            "chapterNumber": 1,
            "contentHash": content_hash.clone(),
        });
        let mut candidate = entry_payload("林清玄", "protagonist");
        candidate["currentState"] = json!({
            "location": "青云宗",
            "powerLevel": "",
            "physicalState": "",
            "mentalState": "",
            "keyItems": "",
            "recentEvents": "",
            "updatedAtChapter": 1,
            "provenance": {"location": {"kind": "derived", "source": source.clone()}},
        });
        let payload = json!({
            "operationId": "op-2",
            "expectedRevision": 1,
            "schemaVersion": 1,
            "intent": "chapter_progress",
            "source": source.clone(),
            "entries": [candidate],
        });
        let receipt = commit(&conn, &payload, None).expect("章节推进应成功");
        let state = receipt.snapshot.entries[0]
            .current_state
            .clone()
            .expect("应写入动态状态");
        assert_eq!(state.location, "青云宗");
        assert_eq!(state.updated_at_chapter, 1);
        let provenance = state.provenance.expect("应有字段级来源");
        assert_eq!(
            provenance.get("location"),
            Some(&CharacterStateFieldProvenance::Derived {
                source: FinalizedSourceIdentity {
                    draft_id: 1,
                    finalization_id: "fin-1".to_string(),
                    chapter_number: 1,
                    content_hash: content_hash.clone(),
                },
            })
        );
    }

    #[test]
    fn chapter_progress_requires_valid_finalized_source_test() {
        let conn = memory_db();
        commit(
            &conn,
            &initialize_payload("op-1", 0, vec![entry_payload("林清玄", "protagonist")]),
            None,
        )
        .unwrap();

        let missing = json!({
            "operationId": "op-2",
            "expectedRevision": 1,
            "schemaVersion": 1,
            "intent": "chapter_progress",
            "entries": [entry_payload("林清玄", "protagonist")],
        });
        assert_eq!(
            commit(&conn, &missing, None).unwrap_err(),
            "章节状态更新缺少定稿来源收据"
        );

        let mut candidate = entry_payload("林清玄", "protagonist");
        candidate["currentState"] = json!({
            "location": "别处",
            "powerLevel": "",
            "physicalState": "",
            "mentalState": "",
            "keyItems": "",
            "recentEvents": "",
            "updatedAtChapter": 1,
        });
        let stale = json!({
            "operationId": "op-3",
            "expectedRevision": 1,
            "schemaVersion": 1,
            "intent": "chapter_progress",
            "source": {
                "draftId": 9,
                "finalizationId": "fin-9",
                "chapterNumber": 1,
                "contentHash": "b".repeat(64),
            },
            "entries": [candidate],
        });
        assert_eq!(
            commit(&conn, &stale, None).unwrap_err(),
            "角色状态来源已失效，已拒绝过期后处理写入"
        );
    }

    #[test]
    fn markdown_renders_in_project_language_test() {
        let entries = vec![CharacterRosterEntry {
            name: "林清玄".to_string(),
            role: CharacterRole::Protagonist,
            gender: "男".to_string(),
            relationships: vec![CharacterRosterRelationship {
                target: "苏白".to_string(),
                relation: "同门".to_string(),
            }],
            ..empty_entry()
        }];
        let zh = render_character_roster_markdown(&entries, "zh-CN");
        assert!(zh.starts_with("# 角色图谱"));
        assert!(zh.contains("## 主角：林清玄"));
        assert!(zh.contains("- 性别：男"));
        assert!(zh.contains("- 关系：苏白（同门）"));

        let en = render_character_roster_markdown(&entries, "en-US");
        assert!(en.starts_with("# Character graph"));
        assert!(en.contains("## Protagonist: 林清玄"));
        assert!(en.contains("- Gender: 男"));
        assert!(en.contains("- Relationship: 苏白 (同门)"));
    }

    #[test]
    fn legacy_markdown_requires_explicit_repair_test() {
        let conn = memory_db();
        conn.execute(
            "UPDATE character_roster_meta SET migration_state = 'legacy_markdown_pending', \
             legacy_markdown = '旧图谱原文' WHERE id = 'main'",
            [],
        )
        .unwrap();

        let snapshot = read(&conn).expect("读取应成功");
        assert_eq!(snapshot.status, CharacterRosterStatus::LegacyRepairRequired);
        assert_eq!(snapshot.legacy_markdown.as_deref(), Some("旧图谱原文"));

        let mismatch = commit(
            &conn,
            &initialize_payload("op-1", 0, vec![entry_payload("甲", "protagonist")]),
            None,
        );
        assert_eq!(
            mismatch.unwrap_err(),
            "检测到旧角色图谱且没有角色卡；只能通过显式旧角色图谱修复写入"
        );
    }

    #[test]
    fn architecture_generation_keeps_manual_fields_test() {
        let conn = memory_db();
        let mut existing = entry_payload("林清玄", "protagonist");
        existing["gender"] = json!("男");
        commit(&conn, &initialize_payload("op-1", 0, vec![existing]), None).unwrap();

        let mut generated = entry_payload("林清玄", "supporting");
        generated["gender"] = json!("女");
        generated["appearance"] = json!("青衫");
        let payload = json!({
            "operationId": "op-2",
            "expectedRevision": 1,
            "schemaVersion": 1,
            "intent": "architecture_generation",
            "entries": [generated],
        });
        let receipt = commit(&conn, &payload, None).expect("架构重生成应成功");
        let entry = &receipt.snapshot.entries[0];
        assert_eq!(entry.gender, "男", "非空旧字段必须保留");
        assert_eq!(entry.appearance, "青衫", "空字段可由新候选补齐");
        assert_eq!(
            entry.role,
            CharacterRole::Protagonist,
            "定位永不被重生成改写"
        );
    }

    #[test]
    fn blueprint_sync_appends_relationships_test() {
        let conn = memory_db();
        commit(
            &conn,
            &initialize_payload(
                "op-1",
                0,
                vec![
                    entry_payload("林清玄", "protagonist"),
                    entry_payload("苏白", "supporting"),
                ],
            ),
            None,
        )
        .unwrap();

        let mut candidate = entry_payload("林清玄", "protagonist");
        candidate["appearance"] = json!("不该写入");
        candidate["relationships"] = json!([{"target": "苏白", "relation": "同门"}]);
        let payload = json!({
            "operationId": "op-2",
            "expectedRevision": 1,
            "schemaVersion": 1,
            "intent": "blueprint_sync",
            "entries": [candidate],
        });
        let receipt = commit(&conn, &payload, None).expect("蓝图同步应成功");
        let entry = receipt
            .snapshot
            .entries
            .iter()
            .find(|entry| entry.name == "林清玄")
            .expect("角色应存在");
        assert_eq!(entry.relationships.len(), 1);
        assert_eq!(entry.relationships[0].target, "苏白");
        assert_eq!(entry.appearance, "", "蓝图同步不重写既有资料");
    }

    #[test]
    fn commit_requires_legacy_schema_on_identity_projects_test() {
        // H3：身份 schema 项目 roster commit 永久抛 CHARACTER_ID_WRITE_REQUIRED
        let conn = identity_schema_db();
        let error = commit(
            &conn,
            &initialize_payload("op-1", 0, vec![entry_payload("林清玄", "protagonist")]),
            None,
        )
        .unwrap_err();
        assert_eq!(error, "CHARACTER_ID_WRITE_REQUIRED");
    }

    #[test]
    fn read_returns_identity_projection_on_identity_schema_test() {
        // H4：身份 schema 下 read 返回身份形态（identityRevision/aliases/
        // 带稳定 ID 的 entries/关系来自 character_relationships）
        let conn = identity_schema_db();
        conn.execute(
            "INSERT INTO characters (character_id, name, role) \
             VALUES ('char-1', '林清玄', 'protagonist')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO characters (character_id, name, role) \
             VALUES ('char-2', '苏白', 'supporting')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO character_aliases (character_id, name, source_key, valid_from) \
             VALUES ('char-1', '林近妖', 'src-1', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO character_identity_approvals \
             (operation_id, payload_hash, receipt_json) \
             VALUES ('approval-1', 'hash', '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO character_relationships (relationship_id, source_character_id, \
             target_character_id, relation, source_display_snapshot, target_display_snapshot, \
             provenance_json, approval_id) \
             VALUES ('rel-1', 'char-1', 'char-2', '盟友', '林清玄', '苏白', '{}', 'approval-1')",
            [],
        )
        .unwrap();

        let snapshot = read(&conn).expect("身份项目读取应成功");
        assert_eq!(snapshot.identity_revision, Some(0));
        assert_eq!(
            snapshot.aliases,
            Some(vec![CharacterRosterAlias {
                character_id: "char-1".to_string(),
                name: "林近妖".to_string(),
                source_key: "src-1".to_string(),
                valid_from: 1,
                valid_through: None,
            }])
        );
        // identityEntries 使用 includeIds=true 变体：条目带稳定 ID
        assert_eq!(snapshot.entries.len(), 2);
        assert_eq!(snapshot.entries[0].character_id.as_deref(), Some("char-1"));
        assert_eq!(snapshot.entries[0].name, "林清玄");
        assert_eq!(snapshot.entries[0].relationships.len(), 1);
        assert_eq!(snapshot.entries[0].relationships[0].target, "苏白");
        assert_eq!(snapshot.entries[0].relationships[0].relation, "盟友");
        assert_eq!(snapshot.entries[1].character_id.as_deref(), Some("char-2"));
        assert!(snapshot.entries[1].relationships.is_empty());
        // 身份形态 factHash = fullIdentityFactHash(identityEntries)；
        // nameOnlyFactHash 恒为 meta.fact_hash（空项目种子为 ''）
        assert_eq!(snapshot.fact_hash.len(), 64);
        assert_eq!(snapshot.name_only_fact_hash, "");
    }
}
