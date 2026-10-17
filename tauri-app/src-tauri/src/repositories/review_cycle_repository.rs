//! 审稿周期仓储（批次 G4-3a）——行为等价平移自
//! `electron/repositories/review-cycle-repository.ts`（789 行）的 5 个公共方法
//! `create` / `attachGeneratedRevision` / `planRecheck` / `commitRecheck`
//! / `getByReviewId`
//! 及其调用图内的全部私有辅助；共享纯函数（`canonicalM03FindingSetHash`、
//! `parseSavedReport`、`parseHumanConfirmedReviewSnapshot`、
//! `parseChapterGoalReview`、`classifyFindingEvidenceChange`、
//! `unshownMustShowRecheckFindings`）随调用图一并平移为本文件私有项，
//! 待后续共享逻辑移植波次再抽公共模块。
//!
//! # 方法映射（TS → Rust，签名一一对应）
//!
//! | TS 方法 | Rust 方法 |
//! |---|---|
//! | `create` | `create` |
//! | `attachGeneratedRevision` | `attach_generated_revision` |
//! | `planRecheck` | `plan_recheck` |
//! | `commitRecheck` | `commit_recheck` |
//! | `getByReviewId` | `get_by_review_id` |
//!
//! # 刻意偏离（均有上游语义理由）
//!
//! - **事务嵌套**：基线在「调用方已开事务」（`REVIEW_CYCLE_TRANSACTION_REQUIRED`
//!   门禁）后用 `db.transaction(fn)()` 把方法体包进嵌套保存点；Tauri 仓储按
//!   约定只接收 `&Connection`（`rusqlite` 的保存点 API 需 `&mut Connection`），
//!   方法体直接在调用方事务内执行——失败时由调用方事务整体回滚，错误码与
//!   列映射不变（对齐 `blueprint_repository` / `character_identity_repository`
//!   的 `*_in_transaction` 既有惯例）。
//! - **`readRecheckCycle` 的合并快照分支**：基线按 `review_cycle_merges` 是否
//!   已安装（M03 已应用而 M06 未应用）选择 `m.body` / `c.body`；Tauri 侧
//!   不建迁移运行时，三表由 `db/schema.rs` 一次性创建，该分支恒走快照列，
//!   基线双分支简化为单分支（行为等价）。
//! - **周期效果证明（进行中）**：已接入初审、修稿与登记复核 attempt 的效果校验，
//!   保留回执、上下文与可见产物的绑定验证；复核 evidenceHash 的完整基线重算、
//!   真实 SQLite 篡改回归及冻结恢复边界的专用验证仍待补齐，不代表完整等价。
//!
//! 错误码（`REVIEW_CYCLE_TRANSACTION_REQUIRED` / `REVIEW_CYCLE_INPUT_INVALID`
//! / `REVIEW_CYCLE_CONFLICT`）逐字对齐基线；「Error: 」前缀由命令层添加。

#[cfg(test)]
#[path = "review_cycle_repository_tests.rs"]
mod evidence_tests;

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::draft_source_guard::ExpectedDraftSource;
use crate::draft_units::count_draft_units;

const TRANSACTION_REQUIRED: &str = "REVIEW_CYCLE_TRANSACTION_REQUIRED";
const INPUT_INVALID: &str = "REVIEW_CYCLE_INPUT_INVALID";
const CONFLICT: &str = "REVIEW_CYCLE_CONFLICT";

fn invalid<T>() -> Result<T, String> {
    Err(INPUT_INVALID.to_string())
}

fn conflict<T>() -> Result<T, String> {
    Err(CONFLICT.to_string())
}

/// SHA-256 十六进制摘要（对齐基线 `createHash('sha256')`）
fn hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 基线 `HASH = /^[a-f0-9]{64}$/u`
fn is_content_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// 基线 `nonempty`：非空字符串且 `trim` 后非空
fn is_nonempty_str(value: &str) -> bool {
    !value.trim().is_empty()
}

/// 基线 `nonempty` 的 JSON 值版本（仅字符串可满足）
fn is_nonempty_string(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::String(s)) if !s.trim().is_empty())
}

/// 基线 `String(value)`：字符串原样，其余类型按 JS 语义字符串化
fn js_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => "null".to_string(),
        _ => "[object Object]".to_string(),
    }
}

/// 基线 `Number.isSafeInteger(value) && value >= minimum` 的 JSON 值版本
fn is_safe_integer(value: Option<&Value>, minimum: i64) -> bool {
    matches!(value, Some(Value::Number(n)) if n.as_i64().is_some_and(|v| v >= minimum && v.abs() <= 9007199254740991))
}

/// 基线 `record`：非 null 对象且非数组
fn record(value: Option<&Value>) -> Option<&Map<String, Value>> {
    value.and_then(Value::as_object)
}

/// 基线 `exactKeys`：键集与给定集合完全一致（`serde_json::Map` 按键排序迭代，
/// 与基线 `Object.keys(value).sort()` 同序；键均为 ASCII，排序等价）
fn exact_keys(value: &Map<String, Value>, keys: &[&str]) -> bool {
    let mut expected: Vec<&str> = keys.to_vec();
    expected.sort_unstable();
    value.keys().map(String::as_str).collect::<Vec<_>>() == expected
}

/// 基线严格相等 `jsonValue === expectedString`（非字符串值不匹配）
fn json_string_eq(value: Option<&Value>, expected: &str) -> bool {
    value == Some(&Value::String(expected.to_string()))
}

/// 字符串 → JSON 字符串字面量（对齐 `JSON.stringify` 的转义）
fn json_string(value: &str) -> String {
    serde_json::to_string(value).expect("字符串 JSON 序列化不得失败")
}

fn json_string_or_null(value: Option<&str>) -> String {
    match value {
        Some(s) => json_string(s),
        None => "null".to_string(),
    }
}

fn json_integer_or_null(value: Option<i64>) -> String {
    match value {
        Some(n) => n.to_string(),
        None => "null".to_string(),
    }
}

// ===== 输入 / 输出类型（对齐基线 27-80 行）=====

/// 审稿周期的生成源身份（`ReviewCycleSource`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCycleSource {
    pub project_id: String,
    pub epoch: String,
    pub source_id: String,
    pub revision: i64,
    pub content_hash: String,
}

/// `CreateReviewCycleInput`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateReviewCycleInput {
    pub root_action_id: String,
    pub review_id: i64,
    pub review_content_hash: String,
    pub source: ReviewCycleSource,
}

/// `CreatedReviewCycle`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedReviewCycle {
    pub cycle_id: String,
    pub finding_set_hash: String,
    pub finding_count: i64,
}

/// `AttachGeneratedRevisionInput`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachGeneratedRevisionInput {
    pub cycle_id: String,
    pub root_action_id: String,
    pub confirmation_review_id: i64,
    pub confirmation_content_hash: String,
    pub revision_id: i64,
    pub revision_content_hash: String,
}

/// `GeneratedRevisionBinding`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedRevisionBinding {
    pub cycle_id: String,
    pub confirmation_review_id: i64,
    pub revision_id: i64,
    pub revision_status: GeneratedRevisionStatus,
    pub idempotent: bool,
}

/// `GeneratedRevisionBinding.revisionStatus`（恒为 `'generated'`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GeneratedRevisionStatus {
    Generated,
}

/// `ReviewCycleRecheckPlan`
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCycleRecheckPlan {
    pub disposition: RecheckDisposition,
    pub context: Option<ReviewCycleRecheckContext>,
    pub root_action_id: String,
    pub review_id: i64,
}

/// 复核处置（`'required' | 'not-required' | 'completed'`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecheckDisposition {
    Required,
    NotRequired,
    Completed,
}

/// `CommittedReviewCycleRecheck`（`recheckCount` 恒为 1）
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommittedReviewCycleRecheck {
    pub cycle_id: String,
    pub attempt_id: String,
    pub recheck_count: i64,
    pub idempotent: bool,
}

/// `ReviewCycleFindingProjection`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCycleFindingProjection {
    pub finding_id: String,
    pub review_item_index: Option<i64>,
    pub category: String,
    pub kind: FindingKind,
    pub status: FindingStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

/// `ReviewCycleProjection`（`mergedHash` / `recheckDisposition` 按基线
/// 条件展开：缺席时键不出现在序列化结果中）
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCycleProjection {
    pub cycle_id: String,
    pub review_id: i64,
    pub revision_status: RevisionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged_hash: Option<String>,
    pub recheck_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recheck_disposition: Option<RecheckDisposition>,
    pub findings: Vec<ReviewCycleFindingProjection>,
}

/// 周期修订状态（`'not-generated' | 'generated' | 'merge-committed'`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RevisionStatus {
    NotGenerated,
    Generated,
    MergeCommitted,
}

impl RevisionStatus {
    /// 基线 `cycle.revision_status as ReviewCycleProjection['revisionStatus']`
    /// 的未检查转换；`review_cycles.revision_status` 的 CHECK 约束保证取值域，
    /// 解析不会失败。
    fn parse(value: &str) -> Self {
        match value {
            "generated" => Self::Generated,
            "merge-committed" => Self::MergeCommitted,
            _ => Self::NotGenerated,
        }
    }
}

// ===== M03 finding 描述符（对齐 `electron/migrations/m03-review-cycle.ts`）=====

/// `M03FindingDescriptor.kind`（`'objective' | 'literary'`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindingKind {
    Objective,
    Literary,
}

impl FindingKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Objective => "objective",
            Self::Literary => "literary",
        }
    }

    /// 基线 `row.kind as M03FindingDescriptor['kind']` 的未检查转换；
    /// `review_findings.kind` 的 CHECK 约束保证取值域，解析不会失败。
    fn parse(value: &str) -> Self {
        match value {
            "literary" => Self::Literary,
            _ => Self::Objective,
        }
    }
}

/// `M03FindingDescriptor.status`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindingStatus {
    Unverified,
    Unresolved,
    Unknown,
    Resolved,
    AuthorWaived,
}

impl FindingStatus {
    /// 基线 `row.status as M03FindingDescriptor['status']` 的未检查转换；
    /// `review_findings.status` 的 CHECK 约束保证取值域，解析不会失败。
    fn parse(value: &str) -> Self {
        match value {
            "unresolved" => Self::Unresolved,
            "unknown" => Self::Unknown,
            "resolved" => Self::Resolved,
            "author-waived" => Self::AuthorWaived,
            _ => Self::Unverified,
        }
    }
}

/// `M03FindingDescriptor.source.span`（`unit` 恒为 `'utf16-code-unit'`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingSpan {
    pub start: i64,
    pub end: i64,
    pub unit: String,
}

/// `M03FindingDescriptor.source`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct M03FindingSource {
    pub project_id: String,
    pub epoch: String,
    pub source_id: String,
    pub revision: i64,
    pub content_hash: String,
    pub span: Option<FindingSpan>,
}

/// `M03FindingDescriptor`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct M03FindingDescriptor {
    pub finding_id: String,
    pub source: M03FindingSource,
    pub excerpt_hash: Option<String>,
    pub occurrence: Option<i64>,
    pub category: String,
    pub target_id: Option<String>,
    pub kind: FindingKind,
    pub problem_text: String,
    pub expected_text: Option<String>,
    pub status: FindingStatus,
    pub evidence_hash: Option<String>,
    pub confirmation_item_index: Option<i64>,
}

// ===== 审稿报告解析（基线 `parseSavedReport`，私有）=====

#[derive(Debug, Clone)]
struct SavedReport {
    items: Vec<SavedItem>,
    goals: Vec<SavedGoal>,
}

#[derive(Debug, Clone)]
struct SavedItem {
    index: i64,
    category: String,
    severity: String,
    description: String,
    quote: Option<String>,
    goal_id: Option<String>,
    stable_fact_key: Option<String>,
}

#[derive(Debug, Clone)]
struct SavedGoal {
    id: String,
    text: String,
    description: String,
    status: String,
    evidence: Vec<SavedEvidence>,
}

#[derive(Debug, Clone)]
struct SavedEvidence {
    quote: String,
    start: i64,
    end: i64,
}

/// 基线 `parseSavedReport`：校验已保存审稿报告 JSON（`summary` / `items` /
/// `goalReview` 三键，goalReview 必须存在且通过机械校验）。非法返回 `None`
/// （基线抛 `REVIEW_CYCLE_INPUT_INVALID`）。
fn parse_saved_report(body: &str) -> Option<SavedReport> {
    let raw: Value = serde_json::from_str(body).ok()?;
    let report = record(Some(&raw))?;
    if !exact_keys(report, &["summary", "items", "goalReview"])
        || !is_nonempty_string(report.get("summary"))
        || !matches!(report.get("items"), Some(Value::Array(items)) if !items.is_empty())
    {
        return None;
    }
    let goal_review = record(report.get("goalReview"))?;
    if !exact_keys(
        goal_review,
        &["version", "chapterNumber", "coverage", "items"],
    ) || goal_review.get("version")?.as_i64()? != 1
        || !is_safe_integer(goal_review.get("chapterNumber"), 1)
        || !["complete", "unknown", "not_configured"]
            .contains(&js_string(goal_review.get("coverage")?).as_str())
        || !goal_review.get("items")?.is_array()
    {
        return None;
    }

    let mut goal_ids = Vec::new();
    let mut goals = Vec::new();
    for value in goal_review.get("items")?.as_array()? {
        let item = record(Some(value))?;
        if !exact_keys(item, &["id", "text", "status", "description", "evidence"])
            || !is_nonempty_string(item.get("id"))
            || goal_ids.contains(&js_string(item.get("id")?))
            || !is_nonempty_string(item.get("text"))
            || !is_nonempty_string(item.get("description"))
            || !["completed", "unmet", "unknown"].contains(&js_string(item.get("status")?).as_str())
            || !item.get("evidence")?.is_array()
        {
            return None;
        }
        goal_ids.push(js_string(item.get("id")?));
        let mut evidence = Vec::new();
        for entry in item.get("evidence")?.as_array()? {
            let proof = record(Some(entry))?;
            if !exact_keys(proof, &["quote", "start", "end"])
                || !is_nonempty_string(proof.get("quote"))
                || !is_safe_integer(proof.get("start"), 0)
                || proof.get("end")?.as_i64()?
                    != proof.get("start")?.as_i64()?
                        + js_string(proof.get("quote")?).encode_utf16().count() as i64
            {
                return None;
            }
            evidence.push(SavedEvidence {
                quote: js_string(proof.get("quote")?),
                start: proof.get("start")?.as_i64()?,
                end: proof.get("end")?.as_i64()?,
            });
        }
        let status = js_string(item.get("status")?);
        if status != "unknown" && evidence.is_empty() {
            return None;
        }
        goals.push(SavedGoal {
            id: js_string(item.get("id")?),
            text: js_string(item.get("text")?),
            description: js_string(item.get("description")?),
            status,
            evidence,
        });
    }

    let items_array = report.get("items")?.as_array()?;
    let mut items = Vec::new();
    for (index, value) in items_array.iter().enumerate() {
        let item = record(Some(value))?;
        if item.keys().any(|key| {
            ![
                "category",
                "severity",
                "description",
                "quote",
                "goalId",
                "stableFactKey",
                "sourceChapter",
            ]
            .contains(&key.as_str())
        }) || !is_nonempty_string(item.get("category"))
            || !is_nonempty_string(item.get("description"))
            || !["error", "warning", "pass", "unknown"]
                .contains(&js_string(item.get("severity")?).as_str())
            || (item.contains_key("quote") && !is_nonempty_string(item.get("quote")))
            || (item.contains_key("goalId")
                && (!is_nonempty_string(item.get("goalId"))
                    || !goal_ids.contains(&js_string(item.get("goalId")?))))
            || (item.contains_key("stableFactKey")
                && !is_nonempty_string(item.get("stableFactKey")))
            || (item.contains_key("goalId") && item.contains_key("stableFactKey"))
            || (item.contains_key("stableFactKey")
                && (js_string(item.get("severity")?) != "warning"
                    || !is_nonempty_string(item.get("quote"))
                    || !is_safe_integer(item.get("sourceChapter"), 1)))
            || (item.contains_key("sourceChapter")
                && (!item.contains_key("stableFactKey")
                    || !is_safe_integer(item.get("sourceChapter"), 1)))
        {
            return None;
        }
        items.push(SavedItem {
            index: index as i64,
            category: js_string(item.get("category")?),
            severity: js_string(item.get("severity")?),
            description: js_string(item.get("description")?),
            quote: item
                .get("quote")
                .and_then(Value::as_str)
                .map(str::to_string),
            goal_id: item
                .get("goalId")
                .and_then(Value::as_str)
                .map(str::to_string),
            stable_fact_key: item
                .get("stableFactKey")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }
    for goal in &goals {
        let projections: Vec<&SavedItem> = items
            .iter()
            .filter(|item| item.goal_id.as_deref() == Some(goal.id.as_str()))
            .collect();
        let expected_severity = match goal.status.as_str() {
            "completed" => "pass",
            "unmet" => "error",
            _ => "unknown",
        };
        let expected_quote = if goal.evidence.is_empty() {
            None
        } else {
            Some(
                goal.evidence
                    .iter()
                    .map(|e| e.quote.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        };
        let projection = projections.first()?;
        if projections.len() != 1
            || projection.severity != expected_severity
            || projection.quote.as_deref() != expected_quote.as_deref()
        {
            return None;
        }
    }
    Some(SavedReport { items, goals })
}

// ===== finding 构建（基线 `locateUnique` / `finding` / `buildFindings`，私有）=====

/// 基线 `locateUnique`：引文在源文中唯一出现时返回其 UTF-16 码元区间
fn locate_unique(source: &str, quote: &str) -> Option<(i64, i64)> {
    if quote.is_empty() {
        return None;
    }
    let start = utf16_find(source, quote, 0)?;
    if utf16_find(source, quote, start + 1).is_some() {
        return None;
    }
    Some((
        start,
        start + quote.chars().map(char::len_utf16).sum::<usize>() as i64,
    ))
}

/// 在 UTF-16 码元偏移 `from` 起查找子串（基线 `String.prototype.indexOf`）
fn utf16_find(text: &str, needle: &str, from: i64) -> Option<i64> {
    let from = from.max(0) as usize;
    let mut offset = 0usize;
    let mut code_units = 0i64;
    for (byte_index, ch) in text.char_indices() {
        if code_units >= from as i64 {
            if let Some(found) = text[byte_index..].find(needle) {
                let _ = offset;
                return Some(code_units + utf16_len(&text[byte_index..byte_index + found]));
            }
            return None;
        }
        offset = byte_index;
        code_units += ch.len_utf16() as i64;
    }
    let _ = offset;
    None
}

/// 基线 `String.prototype.length`（UTF-16 码元数）
fn utf16_len(text: &str) -> i64 {
    text.chars().map(char::len_utf16).sum::<usize>() as i64
}

/// 按 UTF-16 码元区间切片（基线 `String.prototype.slice`，区间已由调用方校验）
fn utf16_slice(text: &str, start: i64, end: i64) -> Option<&str> {
    let (start, end) = (start.max(0) as usize, end.max(0) as usize);
    let mut byte_start = None;
    let mut byte_end = None;
    let mut code_units = 0usize;
    for (byte_index, ch) in text.char_indices() {
        if code_units == start && byte_start.is_none() {
            byte_start = Some(byte_index);
        }
        if code_units == end && byte_end.is_none() {
            byte_end = Some(byte_index);
        }
        code_units += ch.len_utf16();
    }
    if byte_end.is_none() && code_units == end {
        byte_end = Some(text.len());
    }
    let (byte_start, byte_end) = (byte_start?, byte_end?);
    if byte_start <= byte_end && byte_end <= text.len() {
        Some(&text[byte_start..byte_end])
    } else {
        None
    }
}

struct FindingInput {
    category: String,
    target_id: Option<String>,
    kind: FindingKind,
    problem_text: String,
    expected_text: Option<String>,
    quote: Option<String>,
    anchored_status: FindingStatus,
    logical_identity: String,
}

/// 基线 `finding`：构建 M03 finding 描述符并派生 `findingId`
fn finding(
    source: &ReviewCycleSource,
    source_text: &str,
    input: FindingInput,
) -> M03FindingDescriptor {
    let span = input
        .quote
        .as_deref()
        .and_then(|quote| locate_unique(source_text, quote));
    let anchored = input.target_id.is_some() && span.is_some();
    let mut descriptor = M03FindingDescriptor {
        finding_id: String::new(),
        source: M03FindingSource {
            project_id: source.project_id.clone(),
            epoch: source.epoch.clone(),
            source_id: source.source_id.clone(),
            revision: source.revision,
            content_hash: source.content_hash.clone(),
            span: if anchored {
                let (start, end) = span.expect("已确认锚定");
                Some(FindingSpan {
                    start,
                    end,
                    unit: "utf16-code-unit".to_string(),
                })
            } else {
                None
            },
        },
        excerpt_hash: if anchored {
            Some(hash(input.quote.as_deref().expect("已确认锚定")))
        } else {
            None
        },
        occurrence: if anchored { Some(1) } else { None },
        category: input.category,
        target_id: if anchored { input.target_id } else { None },
        kind: input.kind,
        problem_text: input.problem_text,
        expected_text: input.expected_text,
        status: if anchored {
            input.anchored_status
        } else {
            FindingStatus::Unverified
        },
        evidence_hash: None,
        confirmation_item_index: None,
    };
    let identity = format!(
        "{{\"sourceProjectId\":{},\"sourceEpoch\":{},\"sourceId\":{},\"sourceRevision\":{},\"sourceContentHash\":{},\"spanStart\":{},\"spanEnd\":{},\"spanUnit\":{},\"excerptHash\":{},\"occurrence\":{},\"category\":{},\"targetId\":{},\"kind\":{},\"logicalIdentity\":{}}}",
        json_string(&source.project_id),
        json_string(&source.epoch),
        json_string(&source.source_id),
        source.revision,
        json_string(&source.content_hash),
        json_integer_or_null(descriptor.source.span.as_ref().map(|span| span.start)),
        json_integer_or_null(descriptor.source.span.as_ref().map(|span| span.end)),
        json_string_or_null(descriptor.source.span.as_ref().map(|span| span.unit.as_str())),
        json_string_or_null(descriptor.excerpt_hash.as_deref()),
        json_integer_or_null(descriptor.occurrence),
        json_string(&descriptor.category),
        json_string_or_null(descriptor.target_id.as_deref()),
        json_string(input.kind.as_str()),
        json_string(&input.logical_identity),
    );
    descriptor.finding_id = format!("finding:{}", hash(&identity));
    descriptor
}

/// 基线 `buildFindings`：未完成目标 + 非 pass 的自由项 → finding 列表
fn build_findings(
    report: &SavedReport,
    source: &ReviewCycleSource,
    source_text: &str,
) -> Result<Vec<M03FindingDescriptor>, String> {
    let mut result: Vec<M03FindingDescriptor> = Vec::new();
    let mut finding_by_id: Vec<(String, usize)> = Vec::new();
    let mut add = |descriptor: M03FindingDescriptor,
                   result: &mut Vec<M03FindingDescriptor>|
     -> Result<(), String> {
        match finding_by_id
            .iter()
            .find(|(id, _)| *id == descriptor.finding_id)
        {
            None => {
                finding_by_id.push((descriptor.finding_id.clone(), result.len()));
                result.push(descriptor);
                Ok(())
            }
            Some((_, index)) => {
                if result[*index].status != descriptor.status
                    && descriptor.status == FindingStatus::Unresolved
                {
                    result[*index].status = FindingStatus::Unresolved;
                }
                Ok(())
            }
        }
    };
    for goal in &report.goals {
        if goal.evidence.iter().any(|item| {
            utf16_slice(source_text, item.start, item.end)
                .map(|slice| slice != item.quote)
                .unwrap_or(true)
        }) {
            return invalid();
        }
        if goal.status == "completed" {
            continue;
        }
        let projection = report
            .items
            .iter()
            .find(|item| item.goal_id.as_deref() == Some(goal.id.as_str()))
            .expect("parseSavedReport 已保证每个目标恰有一个投影项");
        let quote = if goal.evidence.len() == 1 {
            Some(goal.evidence[0].quote.clone())
        } else {
            None
        };
        let descriptor = finding(
            source,
            source_text,
            FindingInput {
                category: projection.category.clone(),
                target_id: Some(goal.id.clone()),
                kind: FindingKind::Objective,
                problem_text: projection.description.clone(),
                expected_text: Some(format!("{}\n{}", goal.text, goal.description)),
                quote,
                anchored_status: if goal.status == "unknown" {
                    FindingStatus::Unknown
                } else {
                    FindingStatus::Unresolved
                },
                logical_identity: format!("goal:{}", goal.id),
            },
        );
        add(descriptor, &mut result)?;
    }
    for item in &report.items {
        if item.severity == "pass" || item.goal_id.is_some() {
            continue;
        }
        let objective = item.stable_fact_key.is_some();
        let descriptor = finding(
            source,
            source_text,
            FindingInput {
                category: item.category.clone(),
                target_id: item.stable_fact_key.clone(),
                kind: if objective {
                    FindingKind::Objective
                } else {
                    FindingKind::Literary
                },
                problem_text: item.description.clone(),
                expected_text: None,
                quote: item.quote.clone(),
                anchored_status: FindingStatus::Unresolved,
                logical_identity: if objective {
                    format!(
                        "preflight:{}",
                        item.stable_fact_key.as_deref().expect("已确认稳定事实键")
                    )
                } else {
                    format!("free:{}", item.index)
                },
            },
        );
        add(descriptor, &mut result)?;
    }
    Ok(result)
}

/// 基线 `cycleId`：`cycle:` + SHA-256(`JSON.stringify([rootActionId, reviewId, reviewContentHash])`)
fn cycle_id(input: &CreateReviewCycleInput) -> String {
    let json = format!(
        "[{},{},{}]",
        json_string(&input.root_action_id),
        input.review_id,
        json_string(&input.review_content_hash),
    );
    format!("cycle:{}", hash(&json))
}

// ===== finding 集合哈希（基线 `canonicalM03FindingSetHash`，私有）=====

/// 单个 finding 的规范化 JSON（键序逐字对齐基线 `canonicalM03FindingSetHash`
/// 的 `immutable` 对象字面量；`JSON.stringify` 按插入序输出键）
fn finding_canonical_json(item: &M03FindingDescriptor) -> String {
    format!(
        "{{\"findingId\":{},\"sourceProjectId\":{},\"sourceEpoch\":{},\"sourceId\":{},\"sourceRevision\":{},\"sourceContentHash\":{},\"spanStart\":{},\"spanEnd\":{},\"spanUnit\":{},\"excerptHash\":{},\"occurrence\":{},\"category\":{},\"targetId\":{},\"kind\":{},\"problemText\":{},\"expectedText\":{}}}",
        json_string(&item.finding_id),
        json_string(&item.source.project_id),
        json_string(&item.source.epoch),
        json_string(&item.source.source_id),
        item.source.revision,
        json_string(&item.source.content_hash),
        json_integer_or_null(item.source.span.as_ref().map(|span| span.start)),
        json_integer_or_null(item.source.span.as_ref().map(|span| span.end)),
        json_string_or_null(item.source.span.as_ref().map(|span| span.unit.as_str())),
        json_string_or_null(item.excerpt_hash.as_deref()),
        json_integer_or_null(item.occurrence),
        json_string(&item.category),
        json_string_or_null(item.target_id.as_deref()),
        json_string(item.kind.as_str()),
        json_string(&item.problem_text),
        json_string_or_null(item.expected_text.as_deref()),
    )
}

/// 基线 `canonicalM03FindingSetHash`：按 `findingId` 排序后的规范化集合哈希。
/// `findingId` 恒为 ASCII（`finding:` + 64 位十六进制），Rust 字节序与基线
/// JS UTF-16 串比较等价。
fn canonical_m03_finding_set_hash(findings: &[M03FindingDescriptor]) -> String {
    let mut sorted: Vec<&M03FindingDescriptor> = findings.iter().collect();
    sorted.sort_by(|left, right| left.finding_id.cmp(&right.finding_id));
    let array = sorted
        .iter()
        .map(|item| finding_canonical_json(item))
        .collect::<Vec<_>>()
        .join(",");
    hash(&format!("[{array}]"))
}

// ===== review_findings 行（持久化映射）=====

#[derive(Debug, Clone)]
struct FindingRow {
    finding_id: String,
    source_project_id: String,
    source_epoch: String,
    source_id: String,
    source_revision: i64,
    source_content_hash: String,
    span_start: Option<i64>,
    span_end: Option<i64>,
    span_unit: Option<String>,
    excerpt_hash: Option<String>,
    occurrence: Option<i64>,
    category: String,
    target_id: Option<String>,
    kind: String,
    problem_text: String,
    expected_text: Option<String>,
    status: String,
    evidence_hash: Option<String>,
    confirmation_item_index: Option<i64>,
}

fn finding_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FindingRow> {
    Ok(FindingRow {
        finding_id: row.get("finding_id")?,
        source_project_id: row.get("source_project_id")?,
        source_epoch: row.get("source_epoch")?,
        source_id: row.get("source_id")?,
        source_revision: row.get("source_revision")?,
        source_content_hash: row.get("source_content_hash")?,
        span_start: row.get("span_start")?,
        span_end: row.get("span_end")?,
        span_unit: row.get("span_unit")?,
        excerpt_hash: row.get("excerpt_hash")?,
        occurrence: row.get("occurrence")?,
        category: row.get("category")?,
        target_id: row.get("target_id")?,
        kind: row.get("kind")?,
        problem_text: row.get("problem_text")?,
        expected_text: row.get("expected_text")?,
        status: row.get("status")?,
        evidence_hash: row.get("evidence_hash")?,
        confirmation_item_index: row.get("confirmation_item_index")?,
    })
}

/// 基线 `persistedFinding`：数据库行 → M03 finding 描述符
fn persisted_finding(row: &FindingRow) -> M03FindingDescriptor {
    M03FindingDescriptor {
        finding_id: row.finding_id.clone(),
        source: M03FindingSource {
            project_id: row.source_project_id.clone(),
            epoch: row.source_epoch.clone(),
            source_id: row.source_id.clone(),
            revision: row.source_revision,
            content_hash: row.source_content_hash.clone(),
            span: if row.span_start.is_none() {
                None
            } else {
                Some(FindingSpan {
                    start: row.span_start.expect("已确认非空"),
                    end: row.span_end.expect("CHECK 约束保证成对非空"),
                    unit: row.span_unit.clone().expect("CHECK 约束保证非空"),
                })
            },
        },
        excerpt_hash: row.excerpt_hash.clone(),
        occurrence: row.occurrence,
        category: row.category.clone(),
        target_id: row.target_id.clone(),
        kind: FindingKind::parse(&row.kind),
        problem_text: row.problem_text.clone(),
        expected_text: row.expected_text.clone(),
        status: FindingStatus::parse(&row.status),
        evidence_hash: row.evidence_hash.clone(),
        confirmation_item_index: row.confirmation_item_index,
    }
}

// ===== 绑定校验辅助（基线 `sourceMatches` / `validateGeneratedBinding`）=====

#[derive(Debug, Clone)]
struct SourceBoundRow {
    base_draft_id: i64,
    source_draft_chapter_number: Option<i64>,
    source_draft_version: Option<i64>,
    source_draft_status: Option<String>,
    source_content: Option<String>,
    body: String,
}

fn source_bound_row(
    conn: &Connection,
    table: &str,
    id: i64,
) -> Result<Option<SourceBoundRow>, String> {
    conn.query_row(
        &format!(
            "SELECT r.base_draft_id,r.source_draft_chapter_number,r.source_draft_version,
                    r.source_draft_status,r.source_content,c.body
             FROM {table} r JOIN contents c ON c.id=r.content_id WHERE r.id=?1"
        ),
        [id],
        |row| {
            Ok(SourceBoundRow {
                base_draft_id: row.get("base_draft_id")?,
                source_draft_chapter_number: row.get("source_draft_chapter_number")?,
                source_draft_version: row.get("source_draft_version")?,
                source_draft_status: row.get("source_draft_status")?,
                source_content: row.get("source_content")?,
                body: row.get("body")?,
            })
        },
    )
    .optional()
    .map_err(|error| format!("读取源绑定行失败：{error}"))
}

#[derive(Debug, Clone)]
struct RevisionBoundRow {
    row: SourceBoundRow,
    review_source_id: Option<i64>,
    status: String,
}

fn revision_bound_row(conn: &Connection, id: i64) -> Result<Option<RevisionBoundRow>, String> {
    conn.query_row(
        "SELECT r.base_draft_id,r.source_draft_chapter_number,r.source_draft_version,
                r.source_draft_status,r.source_content,r.review_source_id,r.status,c.body
         FROM revisions r JOIN contents c ON c.id=r.content_id WHERE r.id=?1",
        [id],
        |row| {
            Ok(RevisionBoundRow {
                row: SourceBoundRow {
                    base_draft_id: row.get("base_draft_id")?,
                    source_draft_chapter_number: row.get("source_draft_chapter_number")?,
                    source_draft_version: row.get("source_draft_version")?,
                    source_draft_status: row.get("source_draft_status")?,
                    source_content: row.get("source_content")?,
                    body: row.get("body")?,
                },
                review_source_id: row.get("review_source_id")?,
                status: row.get("status")?,
            })
        },
    )
    .optional()
    .map_err(|error| format!("读取修稿绑定行失败：{error}"))
}

/// 基线 `sourceMatches`：章号/版本/状态/源正文四列一致且源正文非空
fn source_matches(left: &SourceBoundRow, right: &SourceBoundRow) -> bool {
    left.base_draft_id == right.base_draft_id
        && left.source_draft_chapter_number == right.source_draft_chapter_number
        && left.source_draft_version == right.source_draft_version
        && left.source_draft_status == right.source_draft_status
        && left.source_content.is_some()
        && left.source_content == right.source_content
}

/// 基线 `validateGeneratedBinding`：校验生成修稿绑定链（审稿 → 确认 → 修稿）
fn validate_generated_binding(
    input: &AttachGeneratedRevisionInput,
    cycle: &CycleBindingRow,
    conn: &Connection,
) -> Result<(i64, HumanConfirmedReviewSnapshot), String> {
    let review = source_bound_row(conn, "reviews", cycle.review_id)?;
    let confirmation = source_bound_row(conn, "reviews", input.confirmation_review_id)?;
    let revision = revision_bound_row(conn, input.revision_id)?;
    let snapshot = confirmation
        .as_ref()
        .and_then(|row| parse_human_confirmed_review_snapshot(&row.body));
    let valid = review.is_some()
        && confirmation.is_some()
        && revision.is_some()
        && snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.source_draft.is_some())
        && hash(
            review
                .as_ref()
                .expect("已确认存在")
                .source_content
                .as_deref()
                .unwrap_or(""),
        ) == cycle.source_hash
        && source_matches(
            review.as_ref().expect("已确认存在"),
            confirmation.as_ref().expect("已确认存在"),
        )
        && source_matches(
            review.as_ref().expect("已确认存在"),
            &revision.as_ref().expect("已确认存在").row,
        )
        && hash(&confirmation.as_ref().expect("已确认存在").body)
            == input.confirmation_content_hash
        && hash(&revision.as_ref().expect("已确认存在").row.body) == input.revision_content_hash
        && snapshot.as_ref().expect("已确认存在").source_review_id == cycle.review_id
        && snapshot
            .as_ref()
            .expect("已确认存在")
            .source_draft
            .as_ref()
            .expect("已确认存在")
            .id
            == review.as_ref().expect("已确认存在").base_draft_id
        && Some(
            snapshot
                .as_ref()
                .expect("已确认存在")
                .source_draft
                .as_ref()
                .expect("已确认存在")
                .chapter_number,
        ) == review
            .as_ref()
            .expect("已确认存在")
            .source_draft_chapter_number
        && Some(
            snapshot
                .as_ref()
                .expect("已确认存在")
                .source_draft
                .as_ref()
                .expect("已确认存在")
                .version,
        ) == review.as_ref().expect("已确认存在").source_draft_version
        && Some(
            snapshot
                .as_ref()
                .expect("已确认存在")
                .source_draft
                .as_ref()
                .expect("已确认存在")
                .status
                .as_str(),
        ) == review
            .as_ref()
            .expect("已确认存在")
            .source_draft_status
            .as_deref()
        && Some(
            snapshot
                .as_ref()
                .expect("已确认存在")
                .source_draft
                .as_ref()
                .expect("已确认存在")
                .content
                .as_str(),
        ) == review
            .as_ref()
            .expect("已确认存在")
            .source_content
            .as_deref()
        && revision.as_ref().expect("已确认存在").review_source_id
            == Some(input.confirmation_review_id)
        && revision.as_ref().expect("已确认存在").status == "pending";
    if !valid {
        return invalid();
    }
    Ok((
        review.expect("已确认存在").base_draft_id,
        snapshot.expect("已确认存在"),
    ))
}

// ===== 作者豁免绑定（基线 `bindAuthorWaivers`，私有）=====

/// 基线 `bindAuthorWaivers`：把确认快照中 `waive` 决策的 AI 项写回
/// `review_findings`（`apply=false` 时只校验幂等重放）。返回豁免项数。
fn bind_author_waivers(
    cycle_id: &str,
    confirmation_content_hash: &str,
    snapshot: &HumanConfirmedReviewSnapshot,
    conn: &Connection,
    apply: bool,
) -> Result<i64, String> {
    if snapshot.schema_version == 1 {
        return Ok(0);
    }
    if snapshot.cycle_id.as_deref() != Some(cycle_id) {
        return invalid();
    }
    let mut stmt = conn
        .prepare(
            "SELECT finding_id,status,evidence_hash,confirmation_item_index
             FROM review_findings WHERE cycle_id=?1",
        )
        .map_err(|error| format!("读取周期 findings 失败：{error}"))?;
    let findings: Vec<(String, String, Option<String>, Option<i64>)> = stmt
        .query_map([cycle_id], |row| {
            Ok((
                row.get::<_, String>("finding_id")?,
                row.get::<_, String>("status")?,
                row.get::<_, Option<String>>("evidence_hash")?,
                row.get::<_, Option<i64>>("confirmation_item_index")?,
            ))
        })
        .map_err(|error| format!("读取周期 findings 失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("读取周期 findings 失败：{error}"))?;
    let mut seen = Vec::new();
    let mut waived = 0;
    for (index, item) in snapshot.items.iter().enumerate() {
        if item.origin != "ai" || item.finding_id.is_none() {
            continue;
        }
        let finding_id = item.finding_id.as_deref().expect("已确认存在");
        if seen.iter().any(|id: &String| id == finding_id)
            || !findings.iter().any(|(id, _, _, _)| id == finding_id)
        {
            return invalid();
        }
        seen.push(finding_id.to_string());
        if item.decision != "waive" {
            continue;
        }
        waived += 1;
        let current = findings
            .iter()
            .find(|(id, _, _, _)| id == finding_id)
            .expect("已确认存在");
        if !apply {
            if current.1 != "author-waived"
                || current.2.as_deref() != Some(confirmation_content_hash)
                || current.3 != Some(index as i64)
            {
                return conflict();
            }
            continue;
        }
        if current.1 == "resolved" {
            return conflict();
        }
        let changes = conn
            .execute(
                "UPDATE review_findings
                 SET status='author-waived',evidence_hash=?1,confirmation_item_index=?2
                 WHERE cycle_id=?3 AND finding_id=?4 AND status<>'resolved'",
                rusqlite::params![
                    confirmation_content_hash,
                    index as i64,
                    cycle_id,
                    finding_id,
                ],
            )
            .map_err(|error| format!("写回作者豁免失败：{error}"))?;
        if changes != 1 {
            return conflict();
        }
    }
    Ok(waived)
}

// ===== 废弃修稿和解（基线 `reconcileDiscardedRevisionIds` / `detachDiscardedGeneratedCycles`）=====

/// 基线 `reconcileDiscardedRevisionIds`：把指向废弃修稿的生成周期解绑。
/// 返回解绑的周期数。
fn reconcile_discarded_revision_ids(
    conn: &Connection,
    revision_ids: &[i64],
) -> Result<i64, String> {
    let mut cycles: Vec<(String, i64)> = Vec::new();
    for revision_id in revision_ids {
        let status: Option<String> = conn
            .query_row(
                "SELECT status FROM revisions WHERE id=?1",
                [revision_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("读取修稿状态失败：{error}"))?;
        let Some(status) = status else {
            return conflict();
        };
        if status != "discarded" {
            return conflict();
        }
        let cycle: Option<(String, String)> = conn
            .query_row(
                "SELECT cycle_id,revision_status FROM review_cycles WHERE revision_id=?1",
                [revision_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| format!("读取周期绑定失败：{error}"))?;
        if let Some((cycle_id, revision_status)) = cycle {
            if revision_status != "generated" {
                return conflict();
            }
            cycles.push((cycle_id, *revision_id));
        }
    }
    for (cycle_id, revision_id) in &cycles {
        let changes = conn
            .execute(
                "UPDATE review_cycles SET revision_id=NULL,revision_status='not-generated'
                 WHERE cycle_id=?1 AND revision_id=?2 AND revision_status='generated'",
                rusqlite::params![cycle_id, revision_id],
            )
            .map_err(|error| format!("解绑废弃周期失败：{error}"))?;
        if changes != 1 {
            return conflict();
        }
    }
    Ok(cycles.len() as i64)
}

/// 基线 `detachDiscardedGeneratedCycles`：解绑同草稿下其他指向废弃修稿的生成周期
fn detach_discarded_generated_cycles(
    conn: &Connection,
    base_draft_id: i64,
    excluded_cycle_id: &str,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "SELECT rc.revision_id FROM review_cycles rc
             JOIN revisions r ON r.id=rc.revision_id
             WHERE rc.revision_status='generated' AND r.status='discarded'
               AND r.base_draft_id=?1 AND rc.cycle_id<>?2",
        )
        .map_err(|error| format!("读取废弃生成周期失败：{error}"))?;
    let rows: Vec<i64> = stmt
        .query_map(rusqlite::params![base_draft_id, excluded_cycle_id], |row| {
            row.get(0)
        })
        .map_err(|error| format!("读取废弃生成周期失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("读取废弃生成周期失败：{error}"))?;
    reconcile_discarded_revision_ids(conn, &rows)?;
    Ok(())
}

// ===== 复核读取（基线 `isCurrentMergedCycle` / `readRecheckCycle` / `recheckContext`）=====

#[derive(Debug, Clone)]
struct CycleBindingRow {
    root_action_id: String,
    review_id: i64,
    confirmation_review_id: Option<i64>,
    confirmation_content_hash: Option<String>,
    revision_id: Option<i64>,
    source_hash: String,
    revision_status: String,
}

fn cycle_binding_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CycleBindingRow> {
    Ok(CycleBindingRow {
        root_action_id: row.get("root_action_id")?,
        review_id: row.get("review_id")?,
        confirmation_review_id: row.get("confirmation_review_id")?,
        confirmation_content_hash: row.get("confirmation_content_hash")?,
        revision_id: row.get("revision_id")?,
        source_hash: row.get("source_hash")?,
        revision_status: row.get("revision_status")?,
    })
}

#[derive(Debug, Clone)]
struct RecheckCycleRow {
    cycle_id: String,
    root_action_id: String,
    review_id: i64,
    finding_set_hash: String,
    comparison_version: i64,
    revision_status: String,
    merged_hash: Option<String>,
    recheck_count: i64,
    recheck_attempt_id: Option<String>,
    source_content: Option<String>,
    merged_body: String,
}

/// 基线 `isCurrentMergedCycle`：合并快照仍是最新合并修稿且哈希匹配
fn is_current_merged_cycle(
    cycle_id: &str,
    merged_hash: &str,
    conn: &Connection,
) -> Result<bool, String> {
    let current: Option<(String, i64, Option<i64>)> = conn
        .query_row(
            "SELECT c.body,v.revision_index,
                    (SELECT MAX(newer.revision_index) FROM revisions newer
                     WHERE newer.base_draft_id=v.base_draft_id AND newer.status='merged') AS latest_merged_index
             FROM review_cycles rc JOIN revisions v ON v.id=rc.revision_id
             JOIN drafts d ON d.id=v.base_draft_id JOIN contents c ON c.id=d.content_id
             WHERE rc.cycle_id=?1",
            [cycle_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| format!("读取合并快照失败：{error}"))?;
    Ok(
        current.is_some_and(|(body, revision_index, latest_merged_index)| {
            Some(revision_index) == latest_merged_index && hash(&body) == merged_hash
        }),
    )
}

/// 基线 `readRecheckCycle`：读取可复核的合并提交周期（失败即 `REVIEW_CYCLE_CONFLICT`）。
///
/// 偏离：基线按 `review_cycle_merges` 是否已安装选择快照正文列；Tauri 侧三表
/// 由 `schema.rs` 一次性创建，恒走 `m.body` 快照分支（见模块头注）。
fn read_recheck_cycle(cycle_id: &str, conn: &Connection) -> Result<RecheckCycleRow, String> {
    let cycle: Option<RecheckCycleRow> = conn
        .query_row(
            "SELECT rc.cycle_id,rc.root_action_id,rc.review_id,rc.finding_set_hash,rc.comparison_version,
                    rc.revision_status,rc.merged_hash,rc.recheck_count,rc.recheck_attempt_id,r.source_content,
                    m.body AS merged_body
             FROM review_cycles rc JOIN reviews r ON r.id=rc.review_id JOIN revisions v ON v.id=rc.revision_id
             JOIN drafts d ON d.id=v.base_draft_id JOIN contents c ON c.id=d.content_id
             JOIN review_cycle_merges m ON m.cycle_id=rc.cycle_id WHERE rc.cycle_id=?1",
            [cycle_id],
            |row| {
                Ok(RecheckCycleRow {
                    cycle_id: row.get("cycle_id")?,
                    root_action_id: row.get("root_action_id")?,
                    review_id: row.get("review_id")?,
                    finding_set_hash: row.get("finding_set_hash")?,
                    comparison_version: row.get("comparison_version")?,
                    revision_status: row.get("revision_status")?,
                    merged_hash: row.get("merged_hash")?,
                    recheck_count: row.get("recheck_count")?,
                    recheck_attempt_id: row.get("recheck_attempt_id")?,
                    source_content: row.get("source_content")?,
                    merged_body: row.get("merged_body")?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("读取复核周期失败：{error}"))?;
    let Some(cycle) = cycle else {
        return conflict();
    };
    let valid = cycle.revision_status == "merge-committed"
        && cycle.merged_hash.is_some()
        && is_content_hash(cycle.merged_hash.as_deref().expect("已确认非空"))
        && cycle.source_content.is_some()
        && hash(&cycle.merged_body) == cycle.merged_hash.as_deref().expect("已确认非空")
        && is_current_merged_cycle(
            cycle_id,
            cycle.merged_hash.as_deref().expect("已确认非空"),
            conn,
        )?
        && verify_m03_review_cycle(conn);
    if !valid {
        return conflict();
    }
    Ok(cycle)
}

/// 基线 `recheckContext`：构建复核上下文（证据变化的 finding + 未展示必现目标）
fn recheck_context(
    cycle: &RecheckCycleRow,
    conn: &Connection,
) -> Result<ReviewCycleRecheckContext, String> {
    rebuild_recheck_context(cycle, conn, false)
}

/// 历史效果验证须保留已resolved项；新执行计划仍排除已解决项，不能据历史重新准入。
fn rebuild_recheck_context(
    cycle: &RecheckCycleRow,
    conn: &Connection,
    include_resolved_history: bool,
) -> Result<ReviewCycleRecheckContext, String> {
    let rows: Vec<RecheckFindingRow> = {
        let mut stmt = conn
            .prepare(
                "SELECT finding_id,target_id,category,kind,problem_text,expected_text,status,span_start,span_end,span_unit,occurrence
                 FROM review_findings WHERE cycle_id=?1 ORDER BY finding_id",
            )
            .map_err(|error| format!("读取周期 findings 失败：{error}"))?;
        let rows = stmt
            .query_map([cycle.cycle_id.clone()], |row| {
                Ok(RecheckFindingRow {
                    finding_id: row.get("finding_id")?,
                    target_id: row.get("target_id")?,
                    category: row.get("category")?,
                    kind: row.get("kind")?,
                    problem_text: row.get("problem_text")?,
                    expected_text: row.get("expected_text")?,
                    status: row.get("status")?,
                    span_start: row.get("span_start")?,
                    span_end: row.get("span_end")?,
                    span_unit: row.get("span_unit")?,
                    occurrence: row.get("occurrence")?,
                })
            })
            .map_err(|error| format!("读取周期 findings 失败：{error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("读取周期 findings 失败：{error}"))?;
        rows
    };
    let source_content = cycle
        .source_content
        .as_deref()
        .expect("readRecheckCycle 已校验非空");
    let merged_body = &cycle.merged_body;
    let mut findings = Vec::new();
    for row in &rows {
        if ["unverified", "author-waived"].contains(&row.status.as_str())
            || (!include_resolved_history && row.status == "resolved")
            || row.target_id.is_none()
            || row.span_start.is_none()
            || row.span_end.is_none()
            || row.span_unit.as_deref() != Some("utf16-code-unit")
            || row.occurrence.is_none()
        {
            continue;
        }
        let finding = ReviewCycleRecheckFinding {
            finding_id: row.finding_id.clone(),
            target_id: row.target_id.clone().expect("已确认非空"),
            category: row.category.clone(),
            kind: FindingKind::parse(&row.kind),
            problem: row.problem_text.clone(),
            expected: row.expected_text.clone(),
            source_span: Some(FindingSpan {
                start: row.span_start.expect("已确认非空"),
                end: row.span_end.expect("已确认非空"),
                unit: "utf16-code-unit".to_string(),
            }),
            occurrence: row.occurrence,
            source_excerpt: Some(
                utf16_slice(
                    source_content,
                    row.span_start.expect("已确认非空"),
                    row.span_end.expect("已确认非空"),
                )
                .ok_or_else(|| "复核证据区间越界".to_string())?
                .to_string(),
            ),
            must_show: None,
        };
        if classify_finding_evidence_change(source_content, merged_body, &finding)
            == EvidenceChange::Changed
        {
            findings.push(finding);
        }
    }
    let (review_body, confirmation_body): (String, Option<String>) = conn
        .query_row(
            "SELECT c.body AS review_body, cc.body AS confirmation_body
             FROM review_cycles rc JOIN reviews r ON r.id=rc.review_id JOIN contents c ON c.id=r.content_id
             LEFT JOIN reviews cr ON cr.id=rc.confirmation_review_id LEFT JOIN contents cc ON cc.id=cr.content_id
             WHERE rc.cycle_id=?1",
            [cycle.cycle_id.clone()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("读取周期正文失败：{error}"))?;
    let original_report: Value =
        serde_json::from_str(&review_body).map_err(|_| INPUT_INVALID.to_string())?;
    let snapshot = confirmation_body
        .as_deref()
        .and_then(parse_human_confirmed_review_snapshot);
    let unshown =
        unshown_must_show_recheck_findings(Some(&original_report), snapshot.as_ref(), &rows)
            .into_iter()
            .filter(|finding| {
                classify_finding_evidence_change(source_content, merged_body, finding)
                    == EvidenceChange::Changed
            })
            .collect::<Vec<_>>();
    findings.extend(unshown.iter().cloned());
    Ok(ReviewCycleRecheckContext {
        version: 2,
        cycle_id: cycle.cycle_id.clone(),
        comparison_version: cycle.comparison_version,
        merged_hash: cycle
            .merged_hash
            .clone()
            .expect("readRecheckCycle 已校验非空"),
        finding_set_hash: cycle.finding_set_hash.clone(),
        source_content: if unshown.is_empty() {
            None
        } else {
            Some(source_content.to_string())
        },
        findings,
    })
}

#[derive(Debug, Clone)]
struct RecheckFindingRow {
    finding_id: String,
    target_id: Option<String>,
    category: String,
    kind: String,
    problem_text: String,
    expected_text: Option<String>,
    status: String,
    span_start: Option<i64>,
    span_end: Option<i64>,
    span_unit: Option<String>,
    occurrence: Option<i64>,
}

// ===== 复核上下文类型（对齐 `src/shared/review-cycle.ts`）=====

/// `ReviewCycleRecheckFinding`
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCycleRecheckFinding {
    pub finding_id: String,
    pub target_id: String,
    pub category: String,
    pub kind: FindingKind,
    pub problem: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_span: Option<FindingSpan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_excerpt: Option<String>,
    /// 基线 `mustShow?: true`——仅作者明确纳入的必现目标置 `Some(true)`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub must_show: Option<bool>,
}

/// `ReviewCycleRecheckContext`
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCycleRecheckContext {
    pub version: i64,
    pub cycle_id: String,
    pub comparison_version: i64,
    pub merged_hash: String,
    pub finding_set_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_content: Option<String>,
    pub findings: Vec<ReviewCycleRecheckFinding>,
}

// ===== 证据变化分类（基线 `classifyFindingEvidenceChange` 等，私有）=====

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvidenceChange {
    Changed,
    Unchanged,
    Ambiguous,
}

/// 基线 `isNoopRevision`：忽略空白与格式控制符后正文相同
fn is_noop_revision(before: &str, after: &str) -> bool {
    let visible = |value: &str| {
        value
            .chars()
            .filter(|ch| !ch.is_whitespace() && !ch.is_control())
            .collect::<String>()
    };
    visible(before) == visible(after)
}

fn count_occurrences(text: &str, excerpt: &str) -> i64 {
    if excerpt.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut offset = 0;
    while offset <= text.len() {
        let Some(found) = text[offset..].find(excerpt) else {
            break;
        };
        let found = offset + found;
        count += 1;
        offset = found + excerpt.len().max(1);
    }
    count
}

/// 基线 `objectiveText`：剔除非字母数字字符（`\p{L}` / `\p{N}`）
fn objective_text(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphabetic() || ch.is_numeric())
        .collect()
}

/// 基线 `classifyFindingEvidenceChange`：确定性准入——证据未变或歧义时不消耗模型请求
fn classify_finding_evidence_change(
    source: &str,
    merged: &str,
    finding: &ReviewCycleRecheckFinding,
) -> EvidenceChange {
    if finding.must_show == Some(true) {
        return if objective_text(source) == objective_text(merged) {
            EvidenceChange::Unchanged
        } else {
            EvidenceChange::Changed
        };
    }
    let Some(span) = &finding.source_span else {
        return EvidenceChange::Ambiguous;
    };
    let Some(occurrence) = finding.occurrence else {
        return EvidenceChange::Ambiguous;
    };
    if span.unit != "utf16-code-unit"
        || span.start < 0
        || span.end <= span.start
        || span.end > utf16_len(source)
        || occurrence < 1
    {
        return EvidenceChange::Ambiguous;
    }
    let Some(excerpt) = utf16_slice(source, span.start, span.end) else {
        return EvidenceChange::Ambiguous;
    };
    if excerpt.is_empty() || count_occurrences(source, excerpt) < occurrence {
        return EvidenceChange::Ambiguous;
    }
    if is_noop_revision(source, merged) {
        return EvidenceChange::Unchanged;
    }
    if finding.kind == FindingKind::Objective {
        let objective_excerpt = objective_text(excerpt);
        return if !objective_excerpt.is_empty()
            && objective_text(merged).contains(&objective_excerpt)
        {
            EvidenceChange::Unchanged
        } else {
            EvidenceChange::Changed
        };
    }
    if merged.contains(excerpt) {
        EvidenceChange::Unchanged
    } else {
        EvidenceChange::Changed
    }
}

// ===== 未展示必现目标（基线 `unshownMustShowRecheckFindings`，私有）=====

/// 基线 `/^ch\d+:mustShow:\d+$/u`
fn is_must_show_goal_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("ch") else {
        return false;
    };
    let Some((chapter_digits, rest)) = split_digits(rest) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(":mustShow:") else {
        return false;
    };
    let Some((index_digits, end)) = split_digits(rest) else {
        return false;
    };
    !chapter_digits.is_empty() && !index_digits.is_empty() && end.is_empty()
}

fn split_digits(value: &str) -> Option<(&str, &str)> {
    let end = value.len()
        - value
            .trim_start_matches(|ch: char| ch.is_ascii_digit())
            .len();
    if end == 0 {
        return None;
    }
    Some((&value[..end], &value[end..]))
}

/// 基线 `unshownMustShowRecheckFindings`：从原始报告与确认快照投影出
/// 「作者已纳入但旧正文未展示」的必现目标 finding
fn unshown_must_show_recheck_findings(
    raw_report: Option<&Value>,
    snapshot: Option<&HumanConfirmedReviewSnapshot>,
    rows: &[RecheckFindingRow],
) -> Vec<ReviewCycleRecheckFinding> {
    let Some(raw_report) = raw_report else {
        return Vec::new();
    };
    let Some(snapshot) = snapshot else {
        return Vec::new();
    };
    if !raw_report.is_object() {
        return Vec::new();
    }
    let report = raw_report.as_object().expect("已确认是对象");
    let Some(goals) = parse_chapter_goal_review(report.get("goalReview").unwrap_or(&Value::Null))
    else {
        return Vec::new();
    };
    let Some(report_items) = report.get("items").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for goal in &goals.items {
        if !is_must_show_goal_id(&goal.id) || goal.status != "unknown" || !goal.evidence.is_empty()
        {
            continue;
        }
        let projections: Vec<&Value> = report_items
            .iter()
            .filter(|item| item.get("goalId").and_then(Value::as_str) == Some(goal.id.as_str()))
            .collect();
        let choices: Vec<&HumanConfirmedReviewItem> = snapshot
            .items
            .iter()
            .filter(|item| {
                item.goal_id.as_deref() == Some(goal.id.as_str())
                    && item.origin == "ai"
                    && item.decision == "apply"
                    && item.finding_id.is_some()
            })
            .collect();
        if projections.len() != 1 || choices.len() != 1 {
            continue;
        }
        let projection = projections[0];
        let choice = choices[0];
        let finding_id = choice.finding_id.as_deref().expect("已确认存在");
        let expected_text = format!("{}\n{}", goal.text, goal.description);
        let matches: Vec<&RecheckFindingRow> = rows
            .iter()
            .filter(|row| {
                row.finding_id == finding_id
                    && row.kind == "objective"
                    && row.status == "unverified"
                    && row.target_id.is_none()
                    && row.expected_text.as_deref() == Some(expected_text.as_str())
                    && row.problem_text
                        == projection
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
            })
            .collect();
        if matches.len() == 1 && matches[0].finding_id == finding_id {
            result.push(ReviewCycleRecheckFinding {
                finding_id: finding_id.to_string(),
                target_id: goal.id.clone(),
                category: projection
                    .get("category")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                kind: FindingKind::Objective,
                problem: projection
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                expected: Some(expected_text),
                source_span: None,
                occurrence: None,
                source_excerpt: None,
                must_show: Some(true),
            });
        }
    }
    result
}

// ===== 章目标核对（基线 `parseChapterGoalReview`，私有）=====

#[derive(Debug, Clone, PartialEq)]
struct ChapterGoalEvidence {
    quote: String,
    start: i64,
    end: i64,
}

#[derive(Debug, Clone, PartialEq)]
struct ChapterGoalReviewItem {
    id: String,
    text: String,
    status: String,
    description: String,
    evidence: Vec<ChapterGoalEvidence>,
}

#[derive(Debug, Clone, PartialEq)]
struct ChapterGoalReview {
    chapter_number: i64,
    coverage: String,
    items: Vec<ChapterGoalReviewItem>,
}

/// 基线 `parseChapterGoalReview`：读取持久化的章目标核对（机械校验，语义判断仍属模型）
fn parse_chapter_goal_review(value: &Value) -> Option<ChapterGoalReview> {
    let parsed = record(Some(value))?;
    if parsed.get("version")?.as_i64()? != 1
        || !is_safe_integer(parsed.get("chapterNumber"), 1)
        || !["complete", "unknown", "not_configured"]
            .contains(&js_string(parsed.get("coverage")?).as_str())
        || !parsed.get("items")?.is_array()
    {
        return None;
    }
    let mut ids = Vec::new();
    let mut items = Vec::new();
    for value in parsed.get("items")?.as_array()? {
        let item = record(Some(value))?;
        if !is_nonempty_string(item.get("id"))
            || ids.contains(&js_string(item.get("id")?))
            || !is_nonempty_string(item.get("text"))
            || !is_nonempty_string(item.get("description"))
            || !["completed", "unmet", "unknown"].contains(&js_string(item.get("status")?).as_str())
            || !item.get("evidence")?.is_array()
        {
            return None;
        }
        ids.push(js_string(item.get("id")?));
        let mut evidence = Vec::new();
        for value in item.get("evidence")?.as_array()? {
            let proof = record(Some(value))?;
            if !is_nonempty_string(proof.get("quote"))
                || !is_safe_integer(proof.get("start"), 0)
                || proof.get("end")?.as_i64()?
                    != proof.get("start")?.as_i64()?
                        + js_string(proof.get("quote")?).encode_utf16().count() as i64
            {
                return None;
            }
            evidence.push(ChapterGoalEvidence {
                quote: js_string(proof.get("quote")?),
                start: proof.get("start")?.as_i64()?,
                end: proof.get("end")?.as_i64()?,
            });
        }
        let status = js_string(item.get("status")?);
        if status != "unknown" && evidence.is_empty() {
            return None;
        }
        items.push(ChapterGoalReviewItem {
            id: js_string(item.get("id")?),
            text: js_string(item.get("text")?),
            status,
            description: js_string(item.get("description")?),
            evidence,
        });
    }
    Some(ChapterGoalReview {
        chapter_number: parsed.get("chapterNumber")?.as_i64()?,
        coverage: js_string(parsed.get("coverage")?),
        items,
    })
}

// ===== 人工确认快照（基线 `parseHumanConfirmedReviewSnapshot` 等，私有）=====

#[derive(Debug, Clone, PartialEq)]
struct HumanConfirmedReviewItem {
    category: String,
    severity: String,
    description: String,
    quote: Option<String>,
    stable_fact_key: Option<String>,
    source_chapter: Option<i64>,
    goal_id: Option<String>,
    finding_id: Option<String>,
    decision: String,
    origin: String,
}

#[derive(Debug, Clone, PartialEq)]
struct HumanConfirmedReviewSnapshot {
    schema_version: i64,
    cycle_id: Option<String>,
    source_review_id: i64,
    source_draft: Option<ExpectedDraftSource>,
    summary: String,
    author_guidance: String,
    items: Vec<HumanConfirmedReviewItem>,
    goal_review: Option<ChapterGoalReview>,
}

/// 基线 `parseSourceDraft`：确认快照的源稿快照（缺失仅允许在旧式确认上，整体必须存在）
fn parse_source_draft(value: &Value) -> Option<ExpectedDraftSource> {
    let source = record(Some(value))?;
    if !is_safe_integer(source.get("id"), 1)
        || !is_safe_integer(source.get("chapterNumber"), 1)
        || !is_safe_integer(source.get("version"), 1)
        || !["draft", "revised", "reviewed", "finalized", "archived"]
            .contains(&js_string(source.get("status")?).as_str())
        || !source.get("content")?.is_string()
        || count_draft_units(&js_string(source.get("content")?)) < 1
    {
        return None;
    }
    Some(ExpectedDraftSource {
        id: source.get("id")?.as_i64()?,
        chapter_number: source.get("chapterNumber")?.as_i64()?,
        version: source.get("version")?.as_i64()?,
        status: js_string(source.get("status")?),
        content: js_string(source.get("content")?),
    })
}

/// 基线 `parseItem`：单个确认项（v1 不允许 `findingId` / `waive`）
fn parse_human_confirmed_item(
    value: &Value,
    schema_version: i64,
) -> Option<HumanConfirmedReviewItem> {
    let item = record(Some(value))?;
    let category = item.get("category").and_then(Value::as_str)?;
    if !is_nonempty_str(category) {
        return None;
    }
    let category = category.trim().to_string();
    let severity = item.get("severity").and_then(Value::as_str)?;
    if !is_nonempty_str(severity) {
        return None;
    }
    let severity = severity.trim().to_string();
    let description = item.get("description").and_then(Value::as_str)?;
    if !is_nonempty_str(description) {
        return None;
    }
    let description = description.trim().to_string();
    let quote = match item.get("quote") {
        None => None,
        Some(value) => Some(value.as_str()?.to_string()),
    };
    let stable_fact_key = match item.get("stableFactKey") {
        None => None,
        Some(value) => {
            let text = value.as_str()?;
            if !is_nonempty_str(text) {
                return None;
            }
            Some(text.trim().to_string())
        }
    };
    let source_chapter = match item.get("sourceChapter") {
        None => None,
        Some(value) => {
            if !is_safe_integer(Some(value), 1) {
                return None;
            }
            Some(value.as_i64()?)
        }
    };
    let goal_id = match item.get("goalId") {
        None => None,
        Some(value) => {
            let text = value.as_str()?;
            if !is_nonempty_str(text) {
                return None;
            }
            Some(text.trim().to_string())
        }
    };
    let finding_id = match item.get("findingId") {
        None => None,
        Some(value) => {
            let text = value.as_str()?;
            if !is_nonempty_str(text) {
                return None;
            }
            Some(text.trim().to_string())
        }
    };
    let decision = js_string(item.get("decision")?);
    let origin = js_string(item.get("origin")?);
    if !["apply", "ignore", "waive"].contains(&decision.as_str())
        || !["ai", "author"].contains(&origin.as_str())
    {
        return None;
    }
    if schema_version == 1 && (finding_id.is_some() || decision == "waive") {
        return None;
    }
    if decision == "waive" && (schema_version != 2 || origin != "ai" || finding_id.is_none()) {
        return None;
    }
    Some(HumanConfirmedReviewItem {
        category,
        severity,
        description,
        quote,
        stable_fact_key,
        source_chapter,
        goal_id,
        finding_id,
        decision,
        origin,
    })
}

/// 基线 `validateHumanConfirmedReviewSnapshot`：候选快照的规范化冻结校验
fn validate_human_confirmed_review_snapshot(value: &Value) -> Option<HumanConfirmedReviewSnapshot> {
    let record = record(Some(value))?;
    let schema_version = record.get("schemaVersion")?.as_i64()?;
    if js_string(record.get("kind")?) != "human-confirmed-review"
        || (schema_version != 1 && schema_version != 2)
        || !is_safe_integer(record.get("sourceReviewId"), 1)
        || !record.get("items")?.is_array()
    {
        return None;
    }
    let cycle_id = match record.get("cycleId") {
        None => None,
        Some(value) => {
            let text = value.as_str()?;
            if !is_nonempty_str(text) {
                return None;
            }
            Some(text.trim().to_string())
        }
    };
    if cycle_id.is_none()
        || (schema_version == 1 && record.contains_key("cycleId"))
        || (schema_version == 2 && cycle_id.is_none())
    {
        return None;
    }
    let summary = record.get("summary").and_then(Value::as_str)?.to_string();
    let author_guidance = record
        .get("authorGuidance")
        .and_then(Value::as_str)?
        .to_string();
    let source_draft = match record.get("sourceDraft") {
        None => None,
        Some(value) => Some(parse_source_draft(value)?),
    };
    let goal_review = match record.get("goalReview") {
        None => None,
        Some(value) => Some(parse_chapter_goal_review(value)?),
    };
    let items = record
        .get("items")?
        .as_array()?
        .iter()
        .map(|item| parse_human_confirmed_item(item, schema_version))
        .collect::<Option<Vec<_>>>()?;
    Some(HumanConfirmedReviewSnapshot {
        schema_version,
        cycle_id,
        source_review_id: record.get("sourceReviewId")?.as_i64()?,
        source_draft,
        summary,
        author_guidance,
        items,
        goal_review,
    })
}

/// 基线 `parseHumanConfirmedReviewSnapshot`：解析确认正文；原始 AI 报告、
/// 畸形 JSON 与不支持的 schema 均返回 `None`
fn parse_human_confirmed_review_snapshot(content: &str) -> Option<HumanConfirmedReviewSnapshot> {
    let value: Value = serde_json::from_str(content).ok()?;
    validate_human_confirmed_review_snapshot(&value)
}

// ===== 完整性校验（基线 `verifyM03ReviewCycle`，私有）=====

/// 基线 `exactSchema`：列序与 SQL 片段双校验
fn exact_schema(conn: &Connection, table: &str, columns: &[&str], fragments: &[&str]) -> bool {
    let actual: Vec<String> = {
        let mut stmt = match conn.prepare(&format!("PRAGMA table_info({table})")) {
            Ok(stmt) => stmt,
            Err(_) => return false,
        };
        let actual = match stmt.query_map([], |row| row.get::<_, String>(1)) {
            Ok(rows) => rows.collect::<Result<Vec<_>, _>>().unwrap_or_default(),
            Err(_) => return false,
        };
        actual
    };
    let sql: Option<String> = match conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
        [table],
        |row| row.get(0),
    ) {
        Ok(value) => value,
        Err(_) => return false,
    };
    if actual.iter().map(String::as_str).collect::<Vec<_>>() != columns {
        return false;
    }
    let compact = sql.unwrap_or_default().replace(' ', "").to_lowercase();
    fragments
        .iter()
        .all(|fragment| compact.contains(&fragment.replace(' ', "").to_lowercase()))
}

/// 效果身份属于业务行，不属于root；跨root复制收据同样必须拒绝。
fn has_unique_effect_claim(conn: &Connection, kind: &str, id: i64) -> bool {
    matches!(
        conn.query_row(
            "SELECT COUNT(*) FROM generation_attempts
         WHERE json_extract(usage_receipt_json,'$.reviewRevisionEffect.kind')=?1
           AND json_type(usage_receipt_json,'$.reviewRevisionEffect.id')='integer'
           AND json_extract(usage_receipt_json,'$.reviewRevisionEffect.id')=?2",
            rusqlite::params![kind, id],
            |r| r.get::<_, i64>(0),
        ),
        Ok(1)
    )
}

/// 从冻结上下文、回执与可见产物验证初审效果；仅接受一个匹配的attempt。
fn prove_review_effect(
    conn: &Connection,
    cycle: &CycleVerifyRow,
    action: &Map<String, Value>,
    source: &crate::draft_source_guard::ExpectedDraftSource,
    review_body: &str,
) -> bool {
    if !has_unique_effect_claim(conn, "review", cycle.review_id) {
        return false;
    }
    let mut stmt = match conn.prepare("SELECT a.attempt_id,a.run_id,a.root_action_id,a.attempt_json,a.usage_receipt_json,
        r.root_action_id,r.binding_json FROM generation_attempts a JOIN generation_runs r ON r.run_id=a.run_id
        WHERE a.root_action_id=?1 ORDER BY a.rowid") { Ok(stmt) => stmt, Err(_) => return false };
    let rows = match stmt.query_map([cycle.root_action_id.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
        ))
    }) {
        Ok(rows) => rows,
        Err(_) => return false,
    };
    let mut matches = 0usize;
    for row in rows {
        let (attempt_id, run_id, root_action_id, attempt_json, usage_json, run_root, binding_json) =
            match row {
                Ok(v) => v,
                Err(_) => return false,
            };
        if run_root != cycle.root_action_id {
            return false;
        }
        let binding: Value = match serde_json::from_str(&binding_json) {
            Ok(v) => v,
            Err(_) => return false,
        };
        if binding.get("projectId") != action.get("projectId")
            || binding
                .get("epoch")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        {
            return false;
        }
        let usage: Value = match serde_json::from_str(&usage_json) {
            Ok(v) => v,
            Err(_) => return false,
        };
        let effect = match usage.get("reviewRevisionEffect").and_then(Value::as_object) {
            Some(v) => v,
            None => continue,
        };
        if effect.get("kind").and_then(Value::as_str) != Some("review")
            || effect.get("id").and_then(Value::as_i64) != Some(cycle.review_id)
        {
            continue;
        }
        let mut effect_keys = effect.keys().map(String::as_str).collect::<Vec<_>>();
        effect_keys.sort_unstable();
        let mut expected_keys = vec![
            "artifact",
            "contentHash",
            "contextHash",
            "id",
            "index",
            "kind",
        ];
        if effect.contains_key("reportVersion") {
            if effect.get("reportVersion").and_then(Value::as_i64) != Some(2) {
                return false;
            }
            expected_keys.push("reportVersion");
        }
        if effect_keys != expected_keys {
            return false;
        }

        let attempt: Value = match serde_json::from_str(&attempt_json) {
            Ok(v) => v,
            Err(_) => return false,
        };
        if !matches!(
            attempt.get("status").and_then(Value::as_str),
            Some("settled" | "unknown")
        ) || usage
            .pointer("/result/finishReason")
            .and_then(Value::as_str)
            != Some("stop")
            || usage.pointer("/result/failureCode").is_some()
            || attempt.get("attemptId").and_then(Value::as_str) != Some(attempt_id.as_str())
            || attempt.get("rootActionId").and_then(Value::as_str) != Some(root_action_id.as_str())
        {
            return false;
        }
        let index = match conn.query_row(
            "SELECT review_index FROM reviews WHERE id=?1",
            [cycle.review_id],
            |r| r.get::<_, i64>(0),
        ) {
            Ok(v) => v,
            Err(_) => return false,
        };
        if effect.get("index").and_then(Value::as_i64) != Some(index)
            || effect.get("contentHash").and_then(Value::as_str)
                != Some(cycle.review_content_hash.as_str())
        {
            return false;
        }
        let artifact = match effect.get("artifact").and_then(Value::as_object) {
            Some(v) => v,
            None => return false,
        };
        if artifact.len() != 3
            || artifact
                .get("artifactId")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        {
            return false;
        }
        let artifact_id = artifact.get("artifactId").and_then(Value::as_str).unwrap();
        let artifact_row:Option<(String,String,String,i64,String)>=match conn.query_row("SELECT attempt_id,run_id,artifact_json,revision,status FROM generation_artifacts WHERE artifact_id=?1",[artifact_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional(){Ok(v)=>v,Err(_)=>return false};
        let Some((artifact_attempt, artifact_run, artifact_json, revision, status)) = artifact_row
        else {
            return false;
        };
        let art: Value = match serde_json::from_str(&artifact_json) {
            Ok(v) => v,
            Err(_) => return false,
        };
        let artifact_hash = match art.get("textHash").and_then(Value::as_str) {
            Some(v) => v,
            None => return false,
        };
        if artifact_attempt != attempt_id
            || artifact_run != run_id
            || status == "discarded"
            || artifact.get("revision").and_then(Value::as_i64) != Some(revision)
            || art.get("revision").and_then(Value::as_i64) != Some(revision)
            || artifact.get("textHash").and_then(Value::as_str) != Some(artifact_hash)
            || art.get("artifactId").and_then(Value::as_str) != Some(artifact_id)
            || art.get("attemptId").and_then(Value::as_str) != Some(attempt_id.as_str())
            || art.get("rootActionId").and_then(Value::as_str) != Some(root_action_id.as_str())
            || art.get("projectId") != binding.get("projectId")
            || art
                .get("epoch")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            || art.get("fingerprint") != binding.get("fingerprint")
        {
            return false;
        }
        let text = match art.get("text").and_then(Value::as_str) {
            Some(v) => v,
            None => return false,
        };
        if hash(text) != artifact_hash
            || usage
                .pointer("/artifactIdentity/artifactId")
                .and_then(Value::as_str)
                != Some(artifact_id)
            || usage.pointer("/artifactIdentity/epoch") != art.get("epoch")
            || usage.pointer("/artifactIdentity/fingerprint") != art.get("fingerprint")
        {
            return false;
        }
        let manifest = match binding.pointer("/sourceManifest") {
            Some(v) => v,
            None => return false,
        };
        let context = match manifest.get("reviewRevisionContext") {
            Some(v) => v,
            None => return false,
        };
        if manifest.get("operation").and_then(Value::as_str) != Some("review-chapter")
            || context.get("version").and_then(Value::as_i64) != Some(1)
            || context.get("operation").and_then(Value::as_str) != Some("review-chapter")
            || context.get("sourceHash").and_then(Value::as_str)
                != Some(hash(&source.content).as_str())
            || context
                .get("source")
                .and_then(Value::as_object)
                .is_none_or(|v| {
                    v.len() != 5
                        || v.get("id").and_then(Value::as_i64) != Some(source.id)
                        || v.get("chapterNumber").and_then(Value::as_i64)
                            != Some(source.chapter_number)
                        || v.get("version").and_then(Value::as_i64) != Some(source.version)
                        || v.get("status").and_then(Value::as_str) != Some(source.status.as_str())
                        || v.get("content").and_then(Value::as_str) != Some(source.content.as_str())
                })
            || context.get("confirmation").is_some()
        {
            return false;
        }
        let author_input = match manifest.get("authorInputs").and_then(Value::as_array) {
            Some(values)
                if values.len() == 1
                    && values[0].get("id").and_then(Value::as_str)
                        == Some("review-revision-context") =>
            {
                match values[0].get("text").and_then(Value::as_str) {
                    Some(text) => text,
                    None => return false,
                }
            }
            _ => return false,
        };
        let revision_context: crate::services::review_revision_context::ReviewRevisionContext =
            match serde_json::from_str(author_input) {
                Ok(v) => v,
                Err(_) => return false,
            };
        let context_value: Value = match serde_json::from_str(author_input) {
            Ok(v) => v,
            Err(_) => return false,
        };
        if context_value != *context {
            return false;
        }
        let ctx_hash = hash(author_input);
        if effect.get("contextHash").and_then(Value::as_str) != Some(ctx_hash.as_str())
            || manifest
                .get("reviewRevisionContextHash")
                .and_then(Value::as_str)
                != Some(ctx_hash.as_str())
        {
            return false;
        }
        let version = effect
            .get("reportVersion")
            .and_then(Value::as_i64)
            .unwrap_or(1);
        if effect.get("reportVersion").is_some() && version != 2 {
            return false;
        }
        let recheck_version = usage
            .pointer("/reviewCycleRecheck/version")
            .and_then(Value::as_i64);
        if revision_context.recheck.is_some() != recheck_version.is_some() {
            return false;
        }
        if crate::services::review_revision_generation::rebuild_review_report_hash(
            &revision_context,
            text,
            version,
            recheck_version,
        )
        .as_deref()
            != Some(cycle.review_content_hash.as_str())
            || hash(review_body) != cycle.review_content_hash
        {
            return false;
        }
        matches += 1;
    }
    matches == 1
}
fn cycle_attempts(
    conn: &Connection,
    root_id: &str,
) -> Vec<(String, String, String, String, String, String, String)> {
    let mut stmt=match conn.prepare("SELECT a.attempt_id,a.run_id,a.root_action_id,a.attempt_json,a.usage_receipt_json,r.root_action_id,r.binding_json FROM generation_attempts a JOIN generation_runs r ON r.run_id=a.run_id WHERE a.root_action_id=?1 ORDER BY a.rowid"){Ok(v)=>v,Err(_)=>return Vec::new()};
    let mapped = stmt.query_map([root_id], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
        ))
    });
    match mapped {
        Ok(rows) => rows.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}
fn prove_revision_effect(
    conn: &Connection,
    cycle: &CycleVerifyRow,
    action: &Map<String, Value>,
    source: &crate::draft_source_guard::ExpectedDraftSource,
    confirmation_review_id: i64,
    confirmation_body: &str,
    confirmation_hash: &str,
    revision_id: i64,
    revision_index: i64,
    revision_body: &str,
    attempts: &[(String, String, String, String, String, String, String)],
) -> bool {
    if !has_unique_effect_claim(conn, "revision", revision_id) {
        return false;
    }
    let mut matched = 0usize;
    for (attempt_id, run_id, root_id, attempt_json, usage_json, run_root, binding_json) in attempts
    {
        if run_root != &cycle.root_action_id {
            return false;
        }
        let binding: Value = match serde_json::from_str(binding_json) {
            Ok(v) => v,
            Err(_) => return false,
        };
        if binding.get("projectId") != action.get("projectId")
            || binding
                .get("epoch")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        {
            return false;
        }
        let usage: Value = match serde_json::from_str(usage_json) {
            Ok(v) => v,
            Err(_) => return false,
        };
        let Some(effect) = usage.get("reviewRevisionEffect").and_then(Value::as_object) else {
            continue;
        };
        if effect.get("kind").and_then(Value::as_str) != Some("revision")
            || effect.get("id").and_then(Value::as_i64) != Some(revision_id)
        {
            continue;
        }
        let mut effect_keys = effect.keys().map(String::as_str).collect::<Vec<_>>();
        effect_keys.sort_unstable();
        if effect_keys
            != [
                "artifact",
                "compositionHash",
                "contentHash",
                "contextHash",
                "id",
                "index",
                "kind",
            ]
        {
            return false;
        }
        if effect.get("index").and_then(Value::as_i64) != Some(revision_index)
            || effect.get("contentHash").and_then(Value::as_str)
                != Some(hash(revision_body).as_str())
            || effect.get("compositionHash").and_then(Value::as_str)
                != Some(hash(revision_body).as_str())
        {
            return false;
        }
        let attempt: Value = match serde_json::from_str(attempt_json) {
            Ok(v) => v,
            Err(_) => return false,
        };
        if attempt.get("attemptId").and_then(Value::as_str) != Some(attempt_id.as_str())
            || attempt.get("rootActionId").and_then(Value::as_str) != Some(root_id.as_str())
        {
            return false;
        }
        if !matches!(
            attempt.get("status").and_then(Value::as_str),
            Some("settled" | "unknown")
        ) || usage
            .pointer("/result/finishReason")
            .and_then(Value::as_str)
            != Some("stop")
            || usage.pointer("/result/failureCode").is_some()
        {
            return false;
        }
        let Some(artifact_ref) = effect.get("artifact").and_then(Value::as_object) else {
            return false;
        };
        let mut artifact_keys = artifact_ref.keys().map(String::as_str).collect::<Vec<_>>();
        artifact_keys.sort_unstable();
        if artifact_keys != ["artifactId", "revision", "textHash"] {
            return false;
        }
        let Some(artifact_id) = artifact_ref.get("artifactId").and_then(Value::as_str) else {
            return false;
        };
        let row:Option<(String,String,String,i64,String)>=match conn.query_row("SELECT attempt_id,run_id,artifact_json,revision,status FROM generation_artifacts WHERE artifact_id=?1",[artifact_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional(){Ok(v)=>v,Err(_)=>return false};
        let Some((owner_attempt, owner_run, raw, rev, status)) = row else {
            return false;
        };
        let art: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => return false,
        };
        if owner_attempt != *attempt_id
            || owner_run != *run_id
            || status == "discarded"
            || artifact_ref.get("revision").and_then(Value::as_i64) != Some(rev)
            || art.get("revision").and_then(Value::as_i64) != Some(rev)
            || art
                .get("epoch")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            || artifact_ref.get("textHash").and_then(Value::as_str)
                != art.get("textHash").and_then(Value::as_str)
            || art.get("artifactId").and_then(Value::as_str) != Some(artifact_id)
            || art.get("attemptId").and_then(Value::as_str) != Some(attempt_id.as_str())
            || art.get("rootActionId").and_then(Value::as_str) != Some(root_id.as_str())
            || art.get("projectId") != binding.get("projectId")
            || art.get("fingerprint") != binding.get("fingerprint")
            || usage
                .pointer("/artifactIdentity/artifactId")
                .and_then(Value::as_str)
                != Some(artifact_id)
            || usage.pointer("/artifactIdentity/fingerprint") != art.get("fingerprint")
            || usage.pointer("/artifactIdentity/epoch") != art.get("epoch")
        {
            return false;
        }
        let context = match binding.pointer("/sourceManifest/reviewRevisionContext") {
            Some(v) => v,
            None => return false,
        };
        if binding
            .pointer("/sourceManifest/operation")
            .and_then(Value::as_str)
            != Some("refine-from-review")
            || context.get("operation").and_then(Value::as_str) != Some("refine-from-review")
            || context.get("version").and_then(Value::as_i64) != Some(1)
            || context.pointer("/source/id").and_then(Value::as_i64) != Some(source.id)
            || context.pointer("/source/content").and_then(Value::as_str)
                != Some(source.content.as_str())
            || context.pointer("/sourceHash").and_then(Value::as_str)
                != Some(hash(&source.content).as_str())
        {
            return false;
        }
        let Some(authors) = binding
            .pointer("/sourceManifest/authorInputs")
            .and_then(Value::as_array)
        else {
            return false;
        };
        let Some(text) = authors
            .iter()
            .find(|x| x.get("id").and_then(Value::as_str) == Some("review-revision-context"))
            .and_then(|x| x.get("text"))
            .and_then(Value::as_str)
        else {
            return false;
        };
        let ctx: crate::services::review_revision_context::ReviewRevisionContext =
            match serde_json::from_str(text) {
                Ok(v) => v,
                Err(_) => return false,
            };
        if authors.len() != 1
            || authors[0].as_object().is_none_or(|v| v.len() != 2)
            || serde_json::to_value(&ctx).ok().as_ref() != Some(context)
            || &ctx.source != source
            || ctx.recheck.is_some()
        {
            return false;
        }
        let Some(confirm) = ctx.confirmation else {
            return false;
        };
        if confirm.review_source_id != confirmation_review_id
            || confirm.content != confirmation_body
            || serde_json::from_str::<Value>(confirmation_body).ok()
                != serde_json::to_value(&confirm.snapshot).ok()
            || confirm.snapshot.source_review_id != cycle.review_id
            || confirm.snapshot.source_draft.as_ref() != Some(source)
            || confirm.original_review_content_hash != cycle.review_content_hash
            || hash(text)
                != effect
                    .get("contextHash")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            || binding
                .pointer("/sourceManifest/reviewRevisionContextHash")
                .and_then(Value::as_str)
                != Some(hash(text).as_str())
            || hash(confirmation_body) != confirmation_hash
        {
            return false;
        }
        let composition = match usage.get("visibleComposition").and_then(Value::as_object) {
            Some(v) => v,
            None => return false,
        };
        if composition.get("algorithm").and_then(Value::as_str) != Some("visible-append-v1")
            || composition.get("textHash").and_then(Value::as_str)
                != Some(hash(revision_body).as_str())
        {
            return false;
        }
        let Some(ids) = composition.get("artifactIds").and_then(Value::as_array) else {
            return false;
        };
        let Some(sources) = composition.get("sources").and_then(Value::as_array) else {
            return false;
        };
        if ids.is_empty()
            || ids.len() > 32
            || ids.last().and_then(Value::as_str) != Some(artifact_id)
            || sources.len() != ids.len()
        {
            return false;
        }
        let mut text_acc = String::new();
        let mut previous = 0i64;
        let mut seen = std::collections::HashSet::new();
        for (n, idval) in ids.iter().enumerate() {
            let Some(id) = idval.as_str() else {
                return false;
            };
            if !seen.insert(id) {
                return false;
            }
            let data: Option<(String, String, i64, String, String, String, String, String, String)> =
                match conn.query_row(
                    "SELECT a.attempt_id,a.run_id,a.revision,a.status,a.artifact_json,
                            t.attempt_json,t.usage_receipt_json,t.run_id,t.root_action_id
                     FROM generation_artifacts a JOIN generation_attempts t ON t.attempt_id=a.attempt_id
                     WHERE a.artifact_id=?1",
                    [id],
                    |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,
                        r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)),
                ).optional() { Ok(v) => v, Err(_) => return false };
            let Some((
                aid,
                rid,
                revision,
                status,
                raw,
                attempt_raw,
                usage_raw,
                attempt_run,
                attempt_root,
            )) = data
            else {
                return false;
            };
            if rid != *run_id
                || attempt_run != *run_id
                || attempt_root != *root_id
                || status == "discarded"
            {
                return false;
            }
            let art: Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(_) => return false,
            };
            let physical: Value = match serde_json::from_str(&attempt_raw) {
                Ok(v) => v,
                Err(_) => return false,
            };
            let receipt: Value = match serde_json::from_str(&usage_raw) {
                Ok(v) => v,
                Err(_) => return false,
            };
            if !matches!(
                physical.get("status").and_then(Value::as_str),
                Some("settled" | "unknown")
            ) || !matches!(
                receipt
                    .pointer("/result/finishReason")
                    .and_then(Value::as_str),
                Some("stop" | "length")
            ) || receipt.pointer("/result/failureCode").is_some()
                || physical.get("attemptId").and_then(Value::as_str) != Some(aid.as_str())
                || physical.get("rootActionId").and_then(Value::as_str) != Some(root_id.as_str())
                || receipt
                    .pointer("/artifactIdentity/artifactId")
                    .and_then(Value::as_str)
                    != Some(id)
                || receipt.pointer("/artifactIdentity/epoch") != art.get("epoch")
                || receipt.pointer("/artifactIdentity/fingerprint") != art.get("fingerprint")
                || art
                    .get("epoch")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
            {
                return false;
            }
            let Some(chunk) = art.get("text").and_then(Value::as_str) else {
                return false;
            };
            let Some(chunk_hash) = art.get("textHash").and_then(Value::as_str) else {
                return false;
            };
            if hash(chunk) != chunk_hash {
                return false;
            }
            let previous_units = count_visible_prose_units_for_proof(&text_acc);
            if n == 0 {
                text_acc = chunk.trim().to_string();
            } else {
                text_acc =
                    crate::repositories::generation_run_repository::compose_visible_continuation(
                        &text_acc, chunk,
                    );
            }
            if count_visible_prose_units_for_proof(&text_acc) <= previous_units {
                return false;
            }
            if art.get("artifactId").and_then(Value::as_str) != Some(id)
                || art.get("attemptId").and_then(Value::as_str) != Some(aid.as_str())
                || art.get("revision").and_then(Value::as_i64) != Some(revision)
                || art.get("rootActionId").and_then(Value::as_str) != Some(root_id.as_str())
                || art.get("projectId") != binding.get("projectId")
                || art.get("fingerprint") != binding.get("fingerprint")
            {
                return false;
            }
            let expected =
                serde_json::json!({"artifactId":id,"revision":revision,"textHash":chunk_hash});
            if sources.get(n) != Some(&expected) {
                return false;
            }
            let ordinal: i64 = match conn.query_row(
                "SELECT rowid FROM generation_attempts WHERE attempt_id=?1",
                [aid],
                |r| r.get(0),
            ) {
                Ok(v) => v,
                Err(_) => return false,
            };
            if ordinal <= previous {
                return false;
            }
            previous = ordinal;
        }
        if hash(&text_acc) != hash(revision_body) {
            return false;
        }
        matched += 1;
    }
    matched == 1
}
fn count_visible_prose_units_for_proof(text: &str) -> usize {
    text.chars().filter(|ch| ch.is_alphanumeric()).count()
}

/// 从持久 finding 与合并快照重建准入集合；不信任回执自述的覆盖范围和状态。
fn verify_recheck_projection(
    conn: &Connection,
    cycle: &CycleVerifyRow,
    source_content: &str,
    merged_body: &str,
    attempt_id: &str,
    receipt: &Map<String, Value>,
    report: &Value,
) -> bool {
    let rebuilt = match rebuild_recheck_context(
        &RecheckCycleRow {
            cycle_id: cycle.cycle_id.clone(),
            root_action_id: cycle.root_action_id.clone(),
            review_id: cycle.review_id,
            finding_set_hash: cycle.finding_set_hash.clone(),
            comparison_version: cycle.comparison_version,
            revision_status: cycle.revision_status.clone(),
            merged_hash: cycle.merged_hash.clone(),
            recheck_count: cycle.recheck_count,
            recheck_attempt_id: cycle.recheck_attempt_id.clone(),
            source_content: Some(source_content.to_string()),
            merged_body: merged_body.to_string(),
        },
        conn,
        true,
    ) {
        Ok(context) => context,
        Err(_) => return false,
    };
    let raw: String = match conn.query_row(
        "SELECT r.binding_json FROM generation_attempts a JOIN generation_runs r ON r.run_id=a.run_id
         WHERE a.attempt_id=?1", [attempt_id], |r| r.get(0),
    ) { Ok(raw) => raw, Err(_) => return false };
    let binding: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let Some(frozen) = binding.pointer("/sourceManifest/reviewRevisionContext/recheck") else {
        return false;
    };
    let mut expected = match serde_json::to_value(&rebuilt) {
        Ok(v) => v,
        Err(_) => return false,
    };
    // 历史策略版本仍由回执决定，来源集合与证据定位不得随版本降级而变化。
    expected["version"] = frozen.get("version").cloned().unwrap_or(Value::Null);
    if !matches!(expected["version"].as_i64(), Some(1 | 2)) || &expected != frozen {
        return false;
    }
    let Some(items) = report.get("items").and_then(Value::as_array) else {
        return false;
    };
    if items.len() != rebuilt.findings.len() {
        return false;
    }
    let mut mappings = Vec::new();
    for (index, (finding, item)) in rebuilt.findings.iter().zip(items).enumerate() {
        if item.get("findingId").and_then(Value::as_str) != Some(finding.finding_id.as_str())
            || item.get("targetId").and_then(Value::as_str) != Some(finding.target_id.as_str())
        {
            return false;
        }
        let known = matches!(
            item.get("severity").and_then(Value::as_str),
            Some("pass" | "warning")
        );
        let resolved = item
            .get("resolved")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let evidence = if known {
            let evidence = crate::services::review_revision_generation::recheck_evidence_hash(
                &finding.finding_id,
                &finding.target_id,
                index as i64,
                item,
                resolved,
            );
            mappings.push(serde_json::json!({
                "findingId":finding.finding_id, "targetId":finding.target_id,
                "reviewItemIndex":index, "evidenceHash":evidence, "resolved":resolved
            }));
            Some(evidence)
        } else {
            None
        };
        if finding.must_show == Some(true) {
            continue;
        }
        let persisted: Option<(String, Option<String>, Option<i64>)> = match conn
            .query_row(
                "SELECT status,evidence_hash,confirmation_item_index FROM review_findings
             WHERE cycle_id=?1 AND finding_id=?2",
                rusqlite::params![cycle.cycle_id, finding.finding_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
        {
            Ok(v) => v,
            Err(_) => return false,
        };
        let expected_status = if !known {
            "unknown"
        } else if resolved {
            "resolved"
        } else {
            "unresolved"
        };
        if persisted != Some((expected_status.to_string(), evidence, None)) {
            return false;
        }
    }
    receipt.len() == 6 && receipt.get("findings") == Some(&Value::Array(mappings))
}

pub(crate) fn verify_m03_review_cycle(conn: &Connection) -> bool {
    if !verify_foreign_keys(conn) {
        return false;
    }
    let merge_table: bool = match conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name='review_cycle_merges'",
        [],
        |row| row.get(0),
    ) {
        Ok(value) => value,
        Err(_) => false,
    };
    if !merge_table {
        return false;
    }
    if !exact_schema(
        conn,
        "review_cycle_merges",
        &["cycle_id", "body"],
        &[
            "cycle_id TEXT PRIMARY KEY REFERENCES review_cycles(cycle_id) ON DELETE CASCADE",
            "body TEXT NOT NULL",
        ],
    ) {
        return false;
    }
    if !exact_schema(
        conn,
        "review_cycles",
        &[
            "cycle_id",
            "root_action_id",
            "review_id",
            "review_content_hash",
            "confirmation_review_id",
            "confirmation_content_hash",
            "revision_id",
            "source_hash",
            "finding_set_hash",
            "comparison_version",
            "revision_status",
            "merged_hash",
            "recheck_count",
            "recheck_attempt_id",
        ],
        &[
            "root_action_id TEXT NOT NULL REFERENCES generation_roots(root_action_id)",
            "review_id INTEGER NOT NULL UNIQUE REFERENCES reviews(id) ON DELETE CASCADE",
            "confirmation_review_id INTEGER UNIQUE REFERENCES reviews(id)",
            "revision_id INTEGER UNIQUE REFERENCES revisions(id)",
            "recheck_attempt_id TEXT UNIQUE REFERENCES generation_attempts(attempt_id)",
            "CHECK(recheck_count IN (0,1))",
            "revision_status='merge-committed'",
        ],
    ) || !exact_schema(
        conn,
        "review_findings",
        &[
            "cycle_id",
            "finding_id",
            "source_project_id",
            "source_epoch",
            "source_id",
            "source_revision",
            "source_content_hash",
            "span_start",
            "span_end",
            "span_unit",
            "excerpt_hash",
            "occurrence",
            "category",
            "target_id",
            "kind",
            "problem_text",
            "expected_text",
            "status",
            "evidence_hash",
            "confirmation_item_index",
        ],
        &[
            "cycle_id TEXT NOT NULL REFERENCES review_cycles(cycle_id) ON DELETE CASCADE",
            "status IN ('unverified','author-waived')",
            "span_unit='utf16-code-unit'",
            "status='author-waived'",
        ],
    ) {
        return false;
    }
    let cycles: Vec<CycleVerifyRow> = {
        let mut stmt = match conn.prepare("SELECT * FROM review_cycles ORDER BY cycle_id") {
            Ok(stmt) => stmt,
            Err(_) => return false,
        };
        let cycles = match stmt.query_map([], cycle_verify_row) {
            Ok(rows) => rows.collect::<Result<Vec<_>, _>>().unwrap_or_default(),
            Err(_) => return false,
        };
        cycles
    };
    let snapshots: Vec<(String, String)> = {
        let mut stmt = match conn.prepare("SELECT cycle_id,body FROM review_cycle_merges") {
            Ok(stmt) => stmt,
            Err(_) => return false,
        };
        let snapshots = match stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?))) {
            Ok(rows) => rows.collect::<Result<Vec<_>, _>>().unwrap_or_default(),
            Err(_) => return false,
        };
        snapshots
    };
    let merge_committed = cycles
        .iter()
        .filter(|cycle| cycle.revision_status == "merge-committed")
        .count();
    if snapshots.len() != merge_committed
        || cycles.iter().any(|cycle| {
            cycle.revision_status != "merge-committed"
                && snapshots.iter().any(|(id, _)| id == &cycle.cycle_id)
        })
    {
        return false;
    }
    for cycle in &cycles {
        // 认证冻结的便携历史：审修原始行按 H 策略保留，但执行链收据（R/X 列）已中和，
        // 无法也不应重放证明。跳过该行的执行链校验，不代表其获得执行权威：
        // 读面（get_by_review_id）不发布投影，连接守卫仍拒绝一切改写。
        // 非冻结行继续逐行严格校验，当前损坏仍须拒绝。
        if crate::services::portable_project_guard::frozen_row(
            conn,
            "review_cycles",
            &cycle.cycle_id,
        ) {
            continue;
        }
        if !verify_cycle_row(cycle, conn) {
            return false;
        }
        let findings: Vec<FindingRow> = {
            let mut stmt = match conn
                .prepare("SELECT * FROM review_findings WHERE cycle_id=?1 ORDER BY finding_id")
            {
                Ok(stmt) => stmt,
                Err(_) => return false,
            };
            let findings = match stmt.query_map([cycle.cycle_id.clone()], finding_row) {
                Ok(rows) => rows.collect::<Result<Vec<_>, _>>().unwrap_or_default(),
                Err(_) => return false,
            };
            findings
        };
        if canonical_m03_finding_set_hash(
            &findings
                .iter()
                .map(|row| persisted_finding(row))
                .collect::<Vec<_>>(),
        ) != cycle.finding_set_hash
        {
            return false;
        }
        if !verify_finding_origins_and_unchanged_states(conn, cycle, &findings) {
            return false;
        }
        let mut seen_anchors = Vec::new();
        for finding in &findings {
            if !verify_finding_row(finding, cycle, conn, &mut seen_anchors) {
                return false;
            }
        }
    }
    true
}

/// 先从已证明的初审报告重建集合，再区分有复核证据的项与必须保持初始状态的项。
fn verify_finding_origins_and_unchanged_states(
    conn: &Connection,
    cycle: &CycleVerifyRow,
    findings: &[FindingRow],
) -> bool {
    let review = match source_bound_row(conn, "reviews", cycle.review_id) {
        Ok(Some(row)) => row,
        _ => return false,
    };
    let report = match parse_saved_report(&review.body) {
        Some(report) => report,
        None => return false,
    };
    let binding_raw: String = match conn.query_row(
        "SELECT r.binding_json FROM generation_attempts a
         JOIN generation_runs r ON r.run_id=a.run_id
         WHERE a.root_action_id=?1
           AND json_extract(a.usage_receipt_json,'$.reviewRevisionEffect.kind')='review'
           AND json_extract(a.usage_receipt_json,'$.reviewRevisionEffect.id')=?2",
        rusqlite::params![cycle.root_action_id, cycle.review_id],
        |r| r.get(0),
    ) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let binding: Value = match serde_json::from_str(&binding_raw) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let (Some(project), Some(epoch), Some(revision), Some(content)) = (
        binding.get("projectId").and_then(Value::as_str),
        binding.get("epoch").and_then(Value::as_str),
        review.source_draft_version,
        review.source_content.as_deref(),
    ) else {
        return false;
    };
    let rebuilt = match build_findings(
        &report,
        &ReviewCycleSource {
            project_id: project.to_string(),
            epoch: epoch.to_string(),
            source_id: format!("draft:{}", review.base_draft_id),
            revision,
            content_hash: hash(content),
        },
        content,
    ) {
        Ok(findings) => findings,
        Err(_) => return false,
    };
    if rebuilt.len() != findings.len()
        || canonical_m03_finding_set_hash(&rebuilt) != cycle.finding_set_hash
    {
        return false;
    }
    // 只有已证明的复核上下文才能授权状态变化；无锚点必现项并不获得可写事实身份。
    let mut rechecked = std::collections::HashSet::new();
    if cycle.recheck_count == 1 {
        let Some(attempt) = cycle.recheck_attempt_id.as_deref() else {
            return false;
        };
        let raw: String = match conn.query_row(
            "SELECT r.binding_json FROM generation_attempts a
             JOIN generation_runs r ON r.run_id=a.run_id WHERE a.attempt_id=?1",
            [attempt],
            |r| r.get(0),
        ) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let value: Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let Some(context) = value.pointer("/sourceManifest/reviewRevisionContext") else {
            return false;
        };
        let context: crate::services::review_revision_context::ReviewRevisionContext =
            match serde_json::from_value(context.clone()) {
                Ok(value) => value,
                Err(_) => return false,
            };
        let Some(recheck) = context.recheck else {
            return false;
        };
        for item in recheck.findings {
            if item.must_show != Some(true) {
                rechecked.insert(item.finding_id);
            }
        }
    }
    for item in &rebuilt {
        let Some(persisted) = findings
            .iter()
            .find(|finding| finding.finding_id == item.finding_id)
        else {
            return false;
        };
        if persisted.status == "author-waived" || rechecked.contains(&item.finding_id) {
            // 作者豁免与参与复核项的状态/证据已由独立证明分支严格核对。
            continue;
        }
        if FindingStatus::parse(&persisted.status) != item.status
            || persisted.evidence_hash.is_some()
            || persisted.confirmation_item_index.is_some()
        {
            return false;
        }
    }
    true
}

#[derive(Debug, Clone)]
struct CycleVerifyRow {
    cycle_id: String,
    root_action_id: String,
    review_id: i64,
    review_content_hash: String,
    confirmation_review_id: Option<i64>,
    confirmation_content_hash: Option<String>,
    revision_id: Option<i64>,
    source_hash: String,
    finding_set_hash: String,
    comparison_version: i64,
    revision_status: String,
    merged_hash: Option<String>,
    recheck_count: i64,
    recheck_attempt_id: Option<String>,
}

fn cycle_verify_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CycleVerifyRow> {
    Ok(CycleVerifyRow {
        cycle_id: row.get("cycle_id")?,
        root_action_id: row.get("root_action_id")?,
        review_id: row.get("review_id")?,
        review_content_hash: row.get("review_content_hash")?,
        confirmation_review_id: row.get("confirmation_review_id")?,
        confirmation_content_hash: row.get("confirmation_content_hash")?,
        revision_id: row.get("revision_id")?,
        source_hash: row.get("source_hash")?,
        finding_set_hash: row.get("finding_set_hash")?,
        comparison_version: row.get("comparison_version")?,
        revision_status: row.get("revision_status")?,
        merged_hash: row.get("merged_hash")?,
        recheck_count: row.get("recheck_count")?,
        recheck_attempt_id: row.get("recheck_attempt_id")?,
    })
}

fn verify_foreign_keys(conn: &Connection) -> bool {
    let foreign_keys: i64 = match conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0)) {
        Ok(value) => value,
        Err(_) => return false,
    };
    if foreign_keys != 1 {
        return false;
    }
    let violations: i64 = match conn.query_row("PRAGMA foreign_key_check", [], |row| {
        let text: String = row.get(0)?;
        let value: Value = serde_json::from_str(&text).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        Ok(value
            .as_array()
            .map(|array| array.len())
            .unwrap_or_default() as i64)
    }) {
        Ok(value) => value,
        // `PRAGMA foreign_key_check` 无违规时不返回行，query_row 报 QueryReturnedNoRows
        Err(rusqlite::Error::QueryReturnedNoRows) => 0,
        Err(_) => return false,
    };
    violations == 0
}

/// 逐周期校验：字段域、审稿哈希绑定、根动作、已接入的效果证明、
/// 确认 / 修稿绑定链与状态机不变量
fn verify_cycle_row(cycle: &CycleVerifyRow, conn: &Connection) -> bool {
    if !is_nonempty_str(&cycle.cycle_id)
        || !is_nonempty_str(&cycle.root_action_id)
        || cycle.review_id < 1
        || !is_content_hash(&cycle.review_content_hash)
        || !is_content_hash(&cycle.source_hash)
        || !is_content_hash(&cycle.finding_set_hash)
        || cycle.comparison_version < 1
        || !["not-generated", "generated", "merge-committed"]
            .contains(&cycle.revision_status.as_str())
        || cycle.recheck_count < 0
        || cycle.recheck_count > 1
    {
        return false;
    }
    let review: Option<(
        i64,
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<String>,
        String,
    )> = match conn.query_row(
        "SELECT r.base_draft_id,r.source_draft_chapter_number,r.source_draft_version,
                r.source_draft_status,r.source_content,c.body
         FROM reviews r JOIN contents c ON c.id=r.content_id WHERE r.id=?1",
        [cycle.review_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        },
    ) {
        Ok(value) => Some(value),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(_) => return false,
    };
    let Some((
        base_draft_id,
        source_draft_chapter_number,
        source_draft_version,
        source_draft_status,
        source_content,
        body,
    )) = review
    else {
        return false;
    };
    if source_draft_chapter_number.map_or(true, |value| value < 1)
        || source_draft_version.map_or(true, |value| value < 1)
        || !source_draft_status.as_deref().is_some_and(is_nonempty_str)
        || source_content.is_none()
        || hash(&body) != cycle.review_content_hash
        || hash(source_content.as_deref().unwrap_or("")) != cycle.source_hash
    {
        return false;
    }
    let root: Option<String> = match conn.query_row(
        "SELECT action_json FROM generation_roots WHERE root_action_id=?1",
        [cycle.root_action_id.clone()],
        |row| row.get(0),
    ) {
        Ok(value) => Some(value),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(_) => return false,
    };
    let action: Option<Value> = root
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok());
    let action = match action {
        Some(Value::Object(map)) => map,
        _ => return false,
    };
    if !is_nonempty_string(action.get("projectId"))
        || !is_nonempty_string(action.get("epoch"))
        || action.get("rootActionId").and_then(Value::as_str) != Some(cycle.root_action_id.as_str())
        || action.get("operation").and_then(Value::as_str) != Some("review-chapter")
    {
        return false;
    }
    let Some(source_content_value) = source_content.as_ref() else {
        return false;
    };
    let source = crate::draft_source_guard::ExpectedDraftSource {
        id: base_draft_id,
        chapter_number: source_draft_chapter_number.unwrap_or_default(),
        version: source_draft_version.unwrap_or_default(),
        status: source_draft_status.clone().unwrap_or_default(),
        content: source_content_value.clone(),
    };
    if !prove_review_effect(conn, cycle, &action, &source, &body) {
        return false;
    }
    // 确认绑定（基线：确认行必须与周期同源稿且哈希匹配，快照字段一致）
    let confirmation = match cycle.confirmation_review_id {
        None => {
            if cycle.confirmation_content_hash.is_some() {
                return false;
            }
            None
        }
        Some(confirmation_review_id) => {
            let Some(ref confirmation_content_hash) = cycle.confirmation_content_hash else {
                return false;
            };
            let row: Option<(Option<String>, String)> = match conn.query_row(
                "SELECT r.source_content,c.body FROM reviews r JOIN contents c ON c.id=r.content_id
                 WHERE r.id=?1 AND r.base_draft_id=?2",
                [confirmation_review_id, base_draft_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ) {
                Ok(value) => Some(value),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(_) => return false,
            };
            let Some((confirmation_source_content, confirmation_body)) = row else {
                return false;
            };
            if hash(&confirmation_body) != confirmation_content_hash.as_str()
                || confirmation_source_content.is_none()
                || hash(confirmation_source_content.as_deref().unwrap_or("")) != cycle.source_hash
            {
                return false;
            }
            let snapshot = match parse_human_confirmed_review_snapshot(&confirmation_body) {
                Some(snapshot) => snapshot,
                None => return false,
            };
            let source_draft = match snapshot.source_draft.as_ref() {
                Some(source_draft) => source_draft,
                None => return false,
            };
            if snapshot.source_review_id != cycle.review_id
                || confirmation_source_content.is_none()
                || hash(confirmation_source_content.as_deref().unwrap_or("")) != cycle.source_hash
                || source_draft.id != base_draft_id
                || Some(source_draft.version) != source_draft_version
                || hash(&source_draft.content) != cycle.source_hash
            {
                return false;
            }
            if bind_author_waivers(
                &cycle.cycle_id,
                confirmation_content_hash,
                &snapshot,
                conn,
                false,
            )
            .is_err()
            {
                return false;
            }
            Some(snapshot)
        }
    };
    // 修稿绑定（基线：修稿必须源自同一确认与源稿且处于 pending）
    let revision = match cycle.revision_id {
        None => None,
        Some(revision_id) => {
            let Some(confirmation_snapshot) = confirmation else {
                return false;
            };
            let row: Option<(
                i64,
                Option<i64>,
                Option<i64>,
                Option<i64>,
                Option<i64>,
                Option<String>,
                Option<String>,
                Option<i64>,
                String,
            )> = match conn.query_row(
                "SELECT r.id,r.base_draft_id,r.review_source_id,r.source_draft_chapter_number,
                        r.source_draft_version,r.source_draft_status,r.source_content,r.merged_to_draft_id,r.status
                 FROM revisions r WHERE r.id=?1",
                [revision_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                    ))
                },
            ) {
                Ok(value) => Some(value),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(_) => return false,
            };
            let Some((
                _id,
                revision_base_draft_id,
                review_source_id,
                revision_chapter_number,
                revision_version,
                revision_status_source,
                revision_source_content,
                revision_merged_to_draft_id,
                revision_status,
            )) = row
            else {
                return false;
            };
            if revision_base_draft_id != Some(base_draft_id)
                || review_source_id != cycle.confirmation_review_id
                || revision_source_content.is_none()
                || revision_chapter_number != source_draft_chapter_number
                || revision_version != source_draft_version
                || revision_status_source != source_draft_status
                || revision_source_content != source_content
                || hash(revision_source_content.as_deref().unwrap_or("")) != cycle.source_hash
            {
                return false;
            }
            let confirmation_id = match cycle.confirmation_review_id {
                Some(value) => value,
                None => return false,
            };
            let confirmation_hash = match cycle.confirmation_content_hash.as_deref() {
                Some(value) => value,
                None => return false,
            };
            let confirmation_body: String = match conn.query_row(
                "SELECT c.body FROM reviews r JOIN contents c ON c.id=r.content_id WHERE r.id=?1",
                [confirmation_id],
                |row| row.get(0),
            ) {
                Ok(value) => value,
                Err(_) => return false,
            };
            let revision_body: String = match conn.query_row(
                "SELECT c.body FROM revisions r JOIN contents c ON c.id=r.content_id WHERE r.id=?1",
                [revision_id],
                |row| row.get(0),
            ) {
                Ok(value) => value,
                Err(_) => return false,
            };
            let revision_index: i64 = match conn.query_row(
                "SELECT revision_index FROM revisions WHERE id=?1",
                [revision_id],
                |row| row.get(0),
            ) {
                Ok(value) => value,
                Err(_) => return false,
            };
            let source = crate::draft_source_guard::ExpectedDraftSource {
                id: base_draft_id,
                chapter_number: source_draft_chapter_number.unwrap_or_default(),
                version: source_draft_version.unwrap_or_default(),
                status: source_draft_status.clone().unwrap_or_default(),
                content: source_content.clone().unwrap_or_default(),
            };
            let attempts = cycle_attempts(conn, &cycle.root_action_id);
            if !prove_revision_effect(
                conn,
                cycle,
                &action,
                &source,
                confirmation_id,
                &confirmation_body,
                confirmation_hash,
                revision_id,
                revision_index,
                &revision_body,
                &attempts,
            ) {
                return false;
            }
            Some((
                revision_id,
                revision_base_draft_id,
                revision_merged_to_draft_id,
                revision_status,
                confirmation_snapshot,
            ))
        }
    };
    match cycle.revision_status.as_str() {
        "not-generated" => {
            if revision.is_some()
                || cycle.merged_hash.is_some()
                || cycle.recheck_count != 0
                || cycle.recheck_attempt_id.is_some()
            {
                return false;
            }
        }
        "generated" => {
            let Some((_, _, _, revision_status, _)) = &revision else {
                return false;
            };
            if revision_status != "pending"
                || cycle.merged_hash.is_some()
                || cycle.recheck_count != 0
                || cycle.recheck_attempt_id.is_some()
            {
                return false;
            }
        }
        "merge-committed" => {
            let Some((
                revision_id,
                revision_base_draft_id,
                revision_merged_to_draft_id,
                revision_status,
                _,
            )) = revision
            else {
                return false;
            };
            if revision_status != "merged"
                || revision_merged_to_draft_id != revision_base_draft_id
                || cycle.merged_hash.is_none()
                || !is_content_hash(cycle.merged_hash.as_deref().unwrap_or(""))
            {
                return false;
            }
            // 合并快照正文哈希必须与周期 merged_hash 匹配（基线经 review_cycle_merges 或草稿正文验证）
            let merged_body: Option<String> = match conn.query_row(
                "SELECT m.body FROM review_cycle_merges m WHERE m.cycle_id=?1",
                [cycle.cycle_id.clone()],
                |row| row.get(0),
            ) {
                Ok(value) => Some(value),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(_) => return false,
            };
            let merged_hash = cycle.merged_hash.as_deref().unwrap_or("");
            if merged_body.is_none() || hash(merged_body.as_deref().unwrap_or("")) != merged_hash {
                return false;
            }
            if cycle.recheck_count == 1 {
                let Some(attempt_id) = cycle.recheck_attempt_id.as_deref() else {
                    return false;
                };
                let recheck: Option<(i64,String,String,Option<String>,Option<i64>,Option<i64>,Option<String>)> = match conn.query_row(
                    "SELECT r.id,c.body,a.usage_receipt_json,r.source_content,r.base_draft_id,r.source_draft_chapter_number,r.source_draft_status FROM generation_attempts a JOIN reviews r ON r.id=CAST(json_extract(a.usage_receipt_json,'$.reviewRevisionEffect.id') AS INTEGER) JOIN contents c ON c.id=r.content_id WHERE a.attempt_id=?1 AND a.root_action_id=?2 AND json_extract(a.usage_receipt_json,'$.reviewRevisionEffect.kind')='review'",
                    rusqlite::params![attempt_id, cycle.root_action_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))) { Ok(v)=>Some(v), Err(rusqlite::Error::QueryReturnedNoRows)=>None, Err(_)=>return false };
                let Some((
                    recheck_id,
                    recheck_body,
                    usage_raw,
                    recheck_source,
                    base,
                    chapter,
                    status,
                )) = recheck
                else {
                    return false;
                };
                if base != Some(revision_base_draft_id.unwrap_or_default())
                    || recheck_source
                        .as_deref()
                        .is_none_or(|v| hash(v) != merged_hash)
                {
                    return false;
                }
                let usage: Value = match serde_json::from_str(&usage_raw) {
                    Ok(v) => v,
                    Err(_) => return false,
                };
                let receipt = match usage.get("reviewCycleRecheck").and_then(Value::as_object) {
                    Some(v) => v,
                    None => return false,
                };
                if receipt.get("cycleId").and_then(Value::as_str) != Some(cycle.cycle_id.as_str())
                    || receipt.get("comparisonVersion").and_then(Value::as_i64)
                        != Some(cycle.comparison_version)
                    || receipt.get("mergedHash").and_then(Value::as_str) != Some(merged_hash)
                    || receipt.get("findingSetHash").and_then(Value::as_str)
                        != Some(cycle.finding_set_hash.as_str())
                    || !matches!(receipt.get("version").and_then(Value::as_i64), Some(1 | 2))
                    || receipt.get("findings").and_then(Value::as_array).is_none()
                {
                    return false;
                }
                let report: Value = match serde_json::from_str(&recheck_body) {
                    Ok(v) => v,
                    Err(_) => return false,
                };
                if !verify_recheck_projection(
                    conn,
                    cycle,
                    source.content.as_str(),
                    merged_body.as_deref().unwrap_or(""),
                    attempt_id,
                    receipt,
                    &report,
                ) {
                    return false;
                }
                let index: i64 = match conn.query_row(
                    "SELECT review_index FROM reviews WHERE id=?1",
                    [recheck_id],
                    |r| r.get(0),
                ) {
                    Ok(v) => v,
                    Err(_) => return false,
                };
                let version: i64 = match conn.query_row(
                    "SELECT source_draft_version FROM reviews WHERE id=?1",
                    [recheck_id],
                    |r| r.get(0),
                ) {
                    Ok(v) => v,
                    Err(_) => return false,
                };
                let source = crate::draft_source_guard::ExpectedDraftSource {
                    id: base.unwrap_or_default(),
                    chapter_number: chapter.unwrap_or_default(),
                    version,
                    status: status.unwrap_or_default(),
                    content: recheck_source.unwrap_or_default(),
                };
                let mut recheck_cycle = cycle.clone();
                recheck_cycle.review_id = recheck_id;
                recheck_cycle.review_content_hash = hash(&recheck_body);
                recheck_cycle.source_hash = hash(&source.content);
                if !prove_review_effect(conn, &recheck_cycle, &action, &source, &recheck_body) {
                    return false;
                }
                let duplicate_effects: i64 = match conn.query_row(
                    "SELECT COUNT(*) FROM generation_attempts WHERE root_action_id=?1 AND json_extract(usage_receipt_json,'$.reviewRevisionEffect.kind')='review' AND CAST(json_extract(usage_receipt_json,'$.reviewRevisionEffect.id') AS INTEGER)=?2",
                    rusqlite::params![cycle.root_action_id, recheck_id], |r| r.get(0)) { Ok(v)=>v, Err(_)=>return false };
                if duplicate_effects != 1 {
                    return false;
                }
                let _ = index;
            } else if cycle.recheck_attempt_id.is_some() {
                return false;
            }
            let _ = revision_id;
        }
        _ => return false,
    }
    true
}

/// 基线逐 finding 校验（省略 `proveEffect` 段）：锚点唯一性、occurrence 计数、
/// 引文哈希、证据哈希格式、状态域与 author-waived 绑定
fn verify_finding_row(
    finding: &FindingRow,
    cycle: &CycleVerifyRow,
    conn: &Connection,
    seen_anchors: &mut Vec<String>,
) -> bool {
    if !is_nonempty_str(&finding.finding_id)
        || !is_nonempty_str(&finding.source_project_id)
        || !is_nonempty_str(&finding.source_epoch)
        || !is_nonempty_str(&finding.source_id)
        || finding.source_revision < 0
        || !is_content_hash(&finding.source_content_hash)
        || !is_nonempty_str(&finding.category)
        || !is_nonempty_str(&finding.problem_text)
        || finding
            .expected_text
            .as_deref()
            .is_some_and(|text| !is_nonempty_str(text))
        || !["objective", "literary"].contains(&finding.kind.as_str())
        || ![
            "unverified",
            "unresolved",
            "unknown",
            "resolved",
            "author-waived",
        ]
        .contains(&finding.status.as_str())
    {
        return false;
    }
    let empty_anchor = finding.span_start.is_none()
        && finding.span_end.is_none()
        && finding.span_unit.is_none()
        && finding.excerpt_hash.is_none()
        && finding.occurrence.is_none()
        && finding.target_id.is_none();
    if finding.status == "unverified" || (finding.status == "author-waived" && empty_anchor) {
        if !empty_anchor
            || (finding.status == "unverified"
                && (finding.evidence_hash.is_some() || finding.confirmation_item_index.is_some()))
        {
            return false;
        }
    } else {
        let source_content = match conn.query_row(
            "SELECT r.source_content FROM reviews r WHERE r.id=?1",
            [cycle.review_id],
            |row| row.get::<_, Option<String>>(0),
        ) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let Some(source_content) = source_content else {
            return false;
        };
        let Some((span_start, span_end)) = finding.span_start.zip(finding.span_end) else {
            return false;
        };
        if span_end <= span_start
            || span_end > utf16_len(&source_content)
            || finding.span_unit.as_deref() != Some("utf16-code-unit")
            || finding.excerpt_hash.is_none()
            || !is_content_hash(finding.excerpt_hash.as_deref().unwrap_or(""))
            || finding.occurrence.map_or(true, |occurrence| occurrence < 1)
            || !is_nonempty_str(finding.target_id.as_deref().unwrap_or(""))
        {
            return false;
        }
        let excerpt = match utf16_slice(&source_content, span_start, span_end) {
            Some(excerpt) => excerpt,
            None => return false,
        };
        if hash(excerpt) != finding.excerpt_hash.as_deref().unwrap_or("")
            || count_occurrences(&source_content, excerpt)
                != finding.occurrence.expect("已确认非空")
        {
            return false;
        }
        let anchor = format!(
            "[{},{{\"start\":{},\"end\":{},\"unit\":{}}},{},{},{},{}]",
            json_string(&finding.source_content_hash),
            span_start,
            span_end,
            json_string(finding.span_unit.as_deref().unwrap_or("utf16-code-unit")),
            finding.occurrence.expect("已确认非空"),
            json_string_or_null(finding.excerpt_hash.as_deref()),
            json_string(&finding.category),
            json_string_or_null(finding.target_id.as_deref()),
        );
        if seen_anchors.iter().any(|existing| existing == &anchor) {
            return false;
        }
        seen_anchors.push(anchor);
    }
    if finding
        .evidence_hash
        .as_ref()
        .is_some_and(|hash_value| !is_content_hash(&hash_value))
    {
        return false;
    }
    if finding.status == "author-waived" {
        let confirmation_content_hash = match cycle.confirmation_content_hash {
            Some(ref value) => value,
            None => return false,
        };
        if finding.evidence_hash.as_deref() != Some(confirmation_content_hash.as_str())
            || finding.confirmation_item_index.is_none()
        {
            return false;
        }
        let Some(confirmation_id) = cycle.confirmation_review_id else {
            return false;
        };
        let body: String = match conn.query_row(
            "SELECT c.body FROM reviews r JOIN contents c ON c.id=r.content_id WHERE r.id=?1",
            [confirmation_id],
            |r| r.get(0),
        ) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let Some(snapshot) = parse_human_confirmed_review_snapshot(&body) else {
            return false;
        };
        let Some(index) = finding
            .confirmation_item_index
            .and_then(|index| usize::try_from(index).ok())
        else {
            return false;
        };
        let Some(item) = snapshot.items.get(index) else {
            return false;
        };
        if snapshot.schema_version != 2
            || snapshot.cycle_id.as_deref() != Some(cycle.cycle_id.as_str())
            || item.finding_id.as_deref() != Some(finding.finding_id.as_str())
            || item.origin != "ai"
            || item.decision != "waive"
        {
            return false;
        }
    } else if finding.confirmation_item_index.is_some() {
        return false;
    }
    true
}

// ===== 公共方法 ======

/// 基线 `ReviewCycleRepository.create`：为一次审稿创建审稿周期并持久化
/// finding 集合（幂等：同一 `review_id` 重放且指纹一致时直接返回既有周期）。
///
/// 调用方必须已开启事务（否则 `REVIEW_CYCLE_TRANSACTION_REQUIRED`）。
pub fn create(
    conn: &Connection,
    input: &CreateReviewCycleInput,
) -> Result<CreatedReviewCycle, String> {
    if conn.is_autocommit() {
        return Err(TRANSACTION_REQUIRED.to_string());
    }
    if !is_nonempty_str(&input.root_action_id)
        || input.review_id < 1
        || !is_content_hash(&input.review_content_hash)
        || !is_nonempty_str(&input.source.project_id)
        || !is_nonempty_str(&input.source.epoch)
        || !is_nonempty_str(&input.source.source_id)
        || input.source.revision < 1
        || !is_content_hash(&input.source.content_hash)
    {
        return invalid();
    }

    // 基线 `db.transaction(fn)()`：见模块头注（事务嵌套偏离），方法体直接在
    // 调用方事务内执行，任一失败即返回错误码，由调用方事务回滚。
    let root_action_json: Option<String> = conn
        .query_row(
            "SELECT action_json FROM generation_roots WHERE root_action_id=?1",
            [input.root_action_id.clone()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("读取生成根动作失败：{error}"))?;
    let action: Option<Value> = root_action_json
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok());
    let Some(action) = record(action.as_ref()) else {
        return invalid();
    };
    if !json_string_eq(action.get("rootActionId"), &input.root_action_id)
        || !json_string_eq(action.get("projectId"), &input.source.project_id)
        || !json_string_eq(action.get("epoch"), &input.source.epoch)
        || !json_string_eq(action.get("operation"), "review-chapter")
    {
        return invalid();
    }

    let review: Option<(i64, Option<i64>, Option<String>, String)> = conn
        .query_row(
            "SELECT r.base_draft_id,r.source_draft_version,r.source_content,c.body
             FROM reviews r JOIN contents c ON c.id=r.content_id WHERE r.id=?1",
            [input.review_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| format!("读取审稿失败：{error}"))?;
    let Some((base_draft_id, source_draft_version, source_content, body)) = review else {
        return invalid();
    };
    let Some(source_content) = source_content else {
        return invalid();
    };
    let Some(source_draft_version) = source_draft_version else {
        return invalid();
    };
    if source_draft_version < 1
        || input.source.source_id != format!("draft:{base_draft_id}")
        || input.source.revision != source_draft_version
        || hash(&source_content) != input.source.content_hash
        || hash(&body) != input.review_content_hash
    {
        return invalid();
    }

    let report = parse_saved_report(&body).ok_or_else(|| INPUT_INVALID.to_string())?;
    let findings = build_findings(&report, &input.source, &source_content)?;
    let finding_set_hash = canonical_m03_finding_set_hash(&findings);
    let id = cycle_id(input);

    let existing: Option<(String, String, String, String, String, i64)> = conn
        .query_row(
            "SELECT cycle_id,root_action_id,review_content_hash,source_hash,finding_set_hash,comparison_version
             FROM review_cycles WHERE review_id=?1",
            [input.review_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("读取既有周期失败：{error}"))?;
    if let Some((
        existing_cycle_id,
        existing_root_action_id,
        existing_review_content_hash,
        existing_source_hash,
        existing_finding_set_hash,
        existing_comparison_version,
    )) = existing
    {
        let stored: Vec<FindingRow> = {
            let mut stmt = conn
                .prepare("SELECT * FROM review_findings WHERE cycle_id=?1 ORDER BY finding_id")
                .map_err(|error| format!("读取既有 findings 失败：{error}"))?;
            let stored = stmt
                .query_map([existing_cycle_id.clone()], finding_row)
                .map_err(|error| format!("读取既有 findings 失败：{error}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("读取既有 findings 失败：{error}"))?;
            stored
        };
        let stored_hash = canonical_m03_finding_set_hash(
            &stored
                .iter()
                .map(|row| persisted_finding(row))
                .collect::<Vec<_>>(),
        );
        if existing_cycle_id != id
            || existing_root_action_id != input.root_action_id
            || existing_review_content_hash != input.review_content_hash
            || existing_source_hash != input.source.content_hash
            || existing_finding_set_hash != finding_set_hash
            || existing_comparison_version != 1
            || stored_hash != finding_set_hash
        {
            return conflict();
        }
        if !verify_m03_review_cycle(conn) {
            return conflict();
        }
        return Ok(CreatedReviewCycle {
            cycle_id: id,
            finding_set_hash,
            finding_count: findings.len() as i64,
        });
    }

    conn.execute(
        "INSERT INTO review_cycles(cycle_id,root_action_id,review_id,review_content_hash,source_hash,
         finding_set_hash,comparison_version,revision_status,recheck_count)
         VALUES(?1,?2,?3,?4,?5,?6,1,'not-generated',0)",
        rusqlite::params![
            id,
            input.root_action_id,
            input.review_id,
            input.review_content_hash,
            input.source.content_hash,
            finding_set_hash,
        ],
    )
    .map_err(|error| format!("写入审稿周期失败：{error}"))?;

    let mut insert = conn
        .prepare(
            "INSERT INTO review_findings(cycle_id,finding_id,source_project_id,source_epoch,source_id,
             source_revision,source_content_hash,span_start,span_end,span_unit,excerpt_hash,occurrence,
             category,target_id,kind,problem_text,expected_text,status,evidence_hash,confirmation_item_index)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
        )
        .map_err(|error| format!("写入审稿 findings 失败：{error}"))?;
    for item in &findings {
        insert
            .execute(rusqlite::params![
                id,
                item.finding_id,
                item.source.project_id,
                item.source.epoch,
                item.source.source_id,
                item.source.revision,
                item.source.content_hash,
                item.source.span.as_ref().map(|span| span.start),
                item.source.span.as_ref().map(|span| span.end),
                item.source.span.as_ref().map(|span| span.unit.clone()),
                item.excerpt_hash,
                item.occurrence,
                item.category,
                item.target_id,
                item.kind.as_str(),
                item.problem_text,
                item.expected_text,
                item.clone().status_as_str(),
                item.evidence_hash,
                item.confirmation_item_index,
            ])
            .map_err(|error| format!("写入审稿 findings 失败：{error}"))?;
    }
    if !verify_m03_review_cycle(conn) {
        return invalid();
    }
    Ok(CreatedReviewCycle {
        cycle_id: id,
        finding_set_hash,
        finding_count: findings.len() as i64,
    })
}

/// 基线 `ReviewCycleRepository.attachGeneratedRevision`：把生成修稿绑定到周期
/// （幂等：同一绑定重放时只校验不再写入）。
///
/// 调用方必须已开启事务（否则 `REVIEW_CYCLE_TRANSACTION_REQUIRED`）。
pub fn attach_generated_revision(
    conn: &Connection,
    input: &AttachGeneratedRevisionInput,
) -> Result<GeneratedRevisionBinding, String> {
    if conn.is_autocommit() {
        return Err(TRANSACTION_REQUIRED.to_string());
    }
    if !is_nonempty_str(&input.cycle_id)
        || !is_nonempty_str(&input.root_action_id)
        || input.confirmation_review_id < 1
        || !is_content_hash(&input.confirmation_content_hash)
        || input.revision_id < 1
        || !is_content_hash(&input.revision_content_hash)
    {
        return invalid();
    }

    let cycle: Option<CycleBindingRow> = conn
        .query_row(
            "SELECT root_action_id,review_id,confirmation_review_id,confirmation_content_hash,
             revision_id,source_hash,revision_status FROM review_cycles WHERE cycle_id=?1",
            [input.cycle_id.clone()],
            cycle_binding_row,
        )
        .optional()
        .map_err(|error| format!("读取审稿周期失败：{error}"))?;
    let Some(cycle) = cycle else {
        return invalid();
    };
    if cycle.root_action_id != input.root_action_id {
        return invalid();
    }
    if cycle.revision_status == "merge-committed" {
        return conflict();
    }
    if cycle.revision_status == "generated" {
        if cycle.confirmation_review_id != Some(input.confirmation_review_id)
            || cycle.confirmation_content_hash.as_deref()
                != Some(input.confirmation_content_hash.as_str())
            || cycle.revision_id != Some(input.revision_id)
        {
            return conflict();
        }
        let (_, snapshot) = validate_generated_binding(input, &cycle, conn)?;
        bind_author_waivers(
            &input.cycle_id,
            &input.confirmation_content_hash,
            &snapshot,
            conn,
            false,
        )?;
        if !verify_m03_review_cycle(conn) {
            return conflict();
        }
        return Ok(GeneratedRevisionBinding {
            cycle_id: input.cycle_id.clone(),
            confirmation_review_id: input.confirmation_review_id,
            revision_id: input.revision_id,
            revision_status: GeneratedRevisionStatus::Generated,
            idempotent: true,
        });
    }
    if cycle.revision_id.is_some()
        || (cycle.confirmation_review_id.is_some()
            && (cycle.confirmation_review_id != Some(input.confirmation_review_id)
                || cycle.confirmation_content_hash.as_deref()
                    != Some(input.confirmation_content_hash.as_str())))
    {
        return conflict();
    }
    let revision_owner: Option<String> = conn
        .query_row(
            "SELECT cycle_id FROM review_cycles WHERE revision_id=?1",
            [input.revision_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("读取修稿归属周期失败：{error}"))?;
    let confirmation_owner: Option<String> = conn
        .query_row(
            "SELECT cycle_id FROM review_cycles WHERE confirmation_review_id=?1",
            [input.confirmation_review_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("读取确认归属周期失败：{error}"))?;
    if (revision_owner.is_some() && revision_owner.as_deref() != Some(input.cycle_id.as_str()))
        || (confirmation_owner.is_some()
            && confirmation_owner.as_deref() != Some(input.cycle_id.as_str()))
    {
        return conflict();
    }
    let (base_draft_id, snapshot) = validate_generated_binding(input, &cycle, conn)?;
    bind_author_waivers(
        &input.cycle_id,
        &input.confirmation_content_hash,
        &snapshot,
        conn,
        cycle.confirmation_review_id.is_none(),
    )?;
    detach_discarded_generated_cycles(conn, base_draft_id, &input.cycle_id)?;
    let changes = conn
        .execute(
            "UPDATE review_cycles SET confirmation_review_id=?1,confirmation_content_hash=?2,revision_id=?3,
             revision_status='generated'
             WHERE cycle_id=?4 AND revision_status='not-generated' AND revision_id IS NULL",
            rusqlite::params![
                input.confirmation_review_id,
                input.confirmation_content_hash,
                input.revision_id,
                input.cycle_id,
            ],
        )
        .map_err(|error| format!("绑定生成修稿失败：{error}"))?;
    if changes != 1 || !verify_m03_review_cycle(conn) {
        return invalid();
    }
    Ok(GeneratedRevisionBinding {
        cycle_id: input.cycle_id.clone(),
        confirmation_review_id: input.confirmation_review_id,
        revision_id: input.revision_id,
        revision_status: GeneratedRevisionStatus::Generated,
        idempotent: false,
    })
}

/// 基线 commitMergedRevision：与正文合并共用事务，冻结复核使用的合并快照。
pub(crate) fn commit_merged_revision(
    conn: &Connection,
    revision_id: i64,
    merged_hash: &str,
) -> Result<(), String> {
    if conn.is_autocommit() {
        return Err(TRANSACTION_REQUIRED.to_string());
    }
    if revision_id < 1 || !is_content_hash(merged_hash) {
        return invalid();
    }
    let cycle: Option<(String, String, Option<String>)> = conn
        .query_row(
            "SELECT cycle_id,revision_status,merged_hash FROM review_cycles WHERE revision_id=?1",
            [revision_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((cycle_id, status, prior_hash)) = cycle else {
        return Ok(());
    };
    if !["generated", "merge-committed"].contains(&status.as_str()) {
        return conflict();
    }
    let merged: Option<(String, i64, Option<i64>, String)> = conn
        .query_row(
            "SELECT r.status,r.base_draft_id,r.merged_to_draft_id,c.body
         FROM revisions r JOIN drafts d ON d.id=r.base_draft_id
         JOIN contents c ON c.id=d.content_id WHERE r.id=?1",
            [revision_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((revision_status, base, target, body)) = merged else {
        return conflict();
    };
    if revision_status != "merged" || target != Some(base) || hash(&body) != merged_hash {
        return conflict();
    }
    if status == "merge-committed" {
        if prior_hash.as_deref() != Some(merged_hash) || !verify_m03_review_cycle(conn) {
            return conflict();
        }
        return Ok(());
    }
    let changes = conn.execute(
        "UPDATE review_cycles SET revision_status='merge-committed',merged_hash=?1
         WHERE cycle_id=?2 AND revision_id=?3 AND revision_status='generated' AND merged_hash IS NULL",
        rusqlite::params![merged_hash, cycle_id, revision_id],
    ).map_err(|error| error.to_string())?;
    if changes != 1 {
        return conflict();
    }
    conn.execute(
        "INSERT INTO review_cycle_merges(cycle_id,body) VALUES(?1,?2)",
        rusqlite::params![cycle_id, body],
    )
    .map_err(|error| error.to_string())?;
    if !verify_m03_review_cycle(conn) {
        return conflict();
    }
    Ok(())
}

/// 基线 `ReviewCycleRepository.planRecheck`：判定合并提交周期是否需要复核
/// （`required` / `not-required` / `completed`），并构建复核上下文。
pub fn plan_recheck(cycle_id: &str, conn: &Connection) -> Result<ReviewCycleRecheckPlan, String> {
    if !is_nonempty_str(cycle_id) {
        return invalid();
    }
    let cycle = read_recheck_cycle(cycle_id, conn)?;
    if cycle.recheck_count == 1 {
        return Ok(ReviewCycleRecheckPlan {
            disposition: RecheckDisposition::Completed,
            context: None,
            root_action_id: cycle.root_action_id,
            review_id: cycle.review_id,
        });
    }
    if cycle.recheck_count != 0 || cycle.recheck_attempt_id.is_some() {
        return conflict();
    }
    let context = recheck_context(&cycle, conn)?;
    Ok(ReviewCycleRecheckPlan {
        disposition: if context.findings.is_empty() {
            RecheckDisposition::NotRequired
        } else {
            RecheckDisposition::Required
        },
        context: if context.findings.is_empty() {
            None
        } else {
            Some(context)
        },
        root_action_id: cycle.root_action_id,
        review_id: cycle.review_id,
    })
}

/// 基线 `ReviewCycleRepository.commitRecheck`：提交一次复核结果，按回执映射
/// 更新 finding 状态并把周期推进到已复核（幂等：同一 `attemptId` 重放直接返回）。
///
/// 调用方必须已开启事务（否则 `REVIEW_CYCLE_TRANSACTION_REQUIRED`）。
pub fn commit_recheck(
    cycle_id: &str,
    attempt_id: &str,
    conn: &Connection,
) -> Result<CommittedReviewCycleRecheck, String> {
    if conn.is_autocommit() {
        return Err(TRANSACTION_REQUIRED.to_string());
    }
    if !is_nonempty_str(cycle_id) || !is_nonempty_str(attempt_id) {
        return invalid();
    }

    let current = read_recheck_cycle(cycle_id, conn)?;
    if current.recheck_count == 1 {
        if current.recheck_attempt_id.as_deref() != Some(attempt_id) {
            return conflict();
        }
        return Ok(CommittedReviewCycleRecheck {
            cycle_id: cycle_id.to_string(),
            attempt_id: attempt_id.to_string(),
            recheck_count: 1,
            idempotent: true,
        });
    }
    let plan = plan_recheck(cycle_id, conn)?;
    let Some(context) = plan.context else {
        return conflict();
    };
    if plan.disposition != RecheckDisposition::Required {
        return conflict();
    }
    let attempt: Option<(String, String)> = conn
        .query_row(
            "SELECT root_action_id,usage_receipt_json FROM generation_attempts WHERE attempt_id=?1",
            [attempt_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("读取生成尝试失败：{error}"))?;
    let Some((attempt_root_action_id, usage_receipt_json)) = attempt else {
        return conflict();
    };
    let usage: Option<Value> = serde_json::from_str(&usage_receipt_json).ok();
    let usage = record(usage.as_ref());
    let receipt = usage
        .and_then(|usage| usage.get("reviewCycleRecheck"))
        .and_then(Value::as_object);
    let valid = attempt_root_action_id == plan.root_action_id
        && receipt.is_some()
        && matches!(
            receipt
                .and_then(|receipt| receipt.get("version"))
                .and_then(Value::as_i64),
            Some(1) | Some(2)
        )
        && receipt
            .and_then(|receipt| receipt.get("cycleId"))
            .and_then(Value::as_str)
            == Some(cycle_id)
        && receipt
            .and_then(|receipt| receipt.get("comparisonVersion"))
            .and_then(Value::as_i64)
            == Some(context.comparison_version)
        && receipt
            .and_then(|receipt| receipt.get("mergedHash"))
            .and_then(Value::as_str)
            == Some(context.merged_hash.as_str())
        && receipt
            .and_then(|receipt| receipt.get("findingSetHash"))
            .and_then(Value::as_str)
            == Some(context.finding_set_hash.as_str())
        && receipt
            .and_then(|receipt| receipt.get("findings"))
            .is_some_and(Value::is_array);
    if !valid {
        return conflict();
    }
    let receipt = receipt.expect("已确认存在");
    let mappings: Vec<Option<&Map<String, Value>>> = receipt
        .get("findings")
        .and_then(Value::as_array)
        .expect("已确认是数组")
        .iter()
        .map(|value| record(Some(value)))
        .collect();
    if mappings.iter().any(|mapping| {
        mapping.and_then(|mapping| mapping.get("resolved")) == Some(&Value::Bool(true))
    }) {
        return conflict();
    }
    let expected_targets: Vec<(&str, &str)> = context
        .findings
        .iter()
        .map(|finding| (finding.finding_id.as_str(), finding.target_id.as_str()))
        .collect();
    if mappings.iter().any(|mapping| {
        let Some(mapping) = mapping else {
            return true;
        };
        let finding_id = mapping
            .get("findingId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let target_id = mapping.get("targetId").and_then(Value::as_str);
        expected_targets
            .iter()
            .find(|(id, _)| *id == finding_id)
            .map(|(_, target)| Some(*target) != target_id)
            .unwrap_or(true)
    }) {
        return conflict();
    }
    let mut item_indexes: Vec<(i64, i64)> = Vec::new();
    for mapping in &mappings {
        if let Some(mapping) = mapping {
            if let Some(review_item_index) = mapping.get("reviewItemIndex").and_then(Value::as_i64)
            {
                if review_item_index >= 0 {
                    match item_indexes
                        .iter_mut()
                        .find(|(index, _)| *index == review_item_index)
                    {
                        Some((_, count)) => *count += 1,
                        None => item_indexes.push((review_item_index, 1)),
                    }
                }
            }
        }
    }
    let mut update = conn
        .prepare(
            "UPDATE review_findings SET status=?1,evidence_hash=?2,confirmation_item_index=NULL
             WHERE cycle_id=?3 AND finding_id=?4 AND status NOT IN ('resolved','author-waived')",
        )
        .map_err(|error| format!("更新复核 findings 失败：{error}"))?;
    for finding in &context.findings {
        if finding.must_show == Some(true) {
            // 基线：旧 finding 无引文，在 M03 中保持未核实
            continue;
        }
        let candidates: Vec<&Option<&Map<String, Value>>> = mappings
            .iter()
            .filter(|mapping| {
                mapping
                    .and_then(|mapping| mapping.get("findingId"))
                    .and_then(Value::as_str)
                    == Some(finding.finding_id.as_str())
            })
            .collect();
        let mapping = if candidates.len() == 1 {
            *candidates[0]
        } else {
            None
        };
        let valid = mapping.is_some_and(|mapping| {
            mapping.get("targetId").and_then(Value::as_str) == Some(finding.target_id.as_str())
                && is_safe_integer(mapping.get("reviewItemIndex"), 0)
                && item_indexes
                    .iter()
                    .find(|(index, _)| {
                        Some(*index) == mapping.get("reviewItemIndex").and_then(Value::as_i64)
                    })
                    .map(|(_, count)| *count == 1)
                    .unwrap_or(false)
                && is_content_hash(
                    mapping
                        .get("evidenceHash")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                )
                && mapping.get("resolved").is_some_and(Value::is_boolean)
        });
        let (status, evidence_hash): (&str, Option<String>) = if valid {
            let mapping = mapping.expect("已确认有效");
            if mapping.get("resolved") == Some(&Value::Bool(true)) {
                (
                    "resolved",
                    mapping
                        .get("evidenceHash")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                )
            } else {
                (
                    "unresolved",
                    mapping
                        .get("evidenceHash")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                )
            }
        } else {
            ("unknown", None)
        };
        let changes = update
            .execute(rusqlite::params![
                status,
                evidence_hash,
                cycle_id,
                finding.finding_id
            ])
            .map_err(|error| format!("更新复核 findings 失败：{error}"))?;
        if changes != 1 {
            return conflict();
        }
    }
    let changes = conn
        .execute(
            "UPDATE review_cycles SET recheck_count=1,recheck_attempt_id=?1
             WHERE cycle_id=?2 AND revision_status='merge-committed' AND recheck_count=0 AND recheck_attempt_id IS NULL",
            [attempt_id, cycle_id],
        )
        .map_err(|error| format!("提交复核状态失败：{error}"))?;
    if changes != 1 || !verify_m03_review_cycle(conn) {
        return conflict();
    }
    Ok(CommittedReviewCycleRecheck {
        cycle_id: cycle_id.to_string(),
        attempt_id: attempt_id.to_string(),
        recheck_count: 1,
        idempotent: false,
    })
}

/// 基线 `ReviewCycleRepository.getByReviewId`：按审稿 ID 读取周期投影。
/// 审稿尚无周期行时返回 `Ok(None)`；周期存在但数据不完整（校验失败、
/// 审稿正文缺失、报告非法、仅有持久化 findings 而无源行）时按基线抛
/// `REVIEW_CYCLE_CONFLICT` / `REVIEW_CYCLE_INPUT_INVALID`。
pub fn get_by_review_id(
    review_id: i64,
    conn: &Connection,
) -> Result<Option<ReviewCycleProjection>, String> {
    if review_id < 1 {
        return invalid();
    }
    let cycle: Option<(String, i64, String, Option<String>, i64)> = conn
        .query_row(
            "SELECT cycle_id,review_id,revision_status,merged_hash,recheck_count
             FROM review_cycles WHERE review_id=?1",
            [review_id],
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
        .map_err(|error| format!("读取审稿周期失败：{error}"))?;
    let Some((cycle_id, review_id, revision_status, merged_hash, recheck_count)) = cycle else {
        return Ok(None);
    };
    // 已认证冻结周期缺少脱敏后的执行证明，不向当前会话发布可操作投影。
    // 审稿、修稿与 finding 原始行仍保留；不得把冻结材料当成执行验证成功。
    if crate::services::portable_project_guard::frozen_row(conn, "review_cycles", &cycle_id) {
        return Ok(None);
    }
    if !verify_m03_review_cycle(conn) {
        return conflict();
    }
    let review: Option<(Option<String>, String)> = conn
        .query_row(
            "SELECT r.source_content,c.body FROM reviews r
             JOIN contents c ON c.id=r.content_id WHERE r.id=?1",
            [review_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("读取审稿失败：{error}"))?;
    let Some((source_content, body)) = review else {
        return conflict();
    };
    let Some(source_content) = source_content else {
        return conflict();
    };
    let report = parse_saved_report(&body).ok_or_else(|| INPUT_INVALID.to_string())?;
    let source_row: Option<(String, String, String, i64, String)> = conn
        .query_row(
            "SELECT source_project_id,source_epoch,source_id,source_revision,source_content_hash
             FROM review_findings WHERE cycle_id=?1 LIMIT 1",
            [cycle_id.clone()],
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
        .map_err(|error| format!("读取周期 findings 失败：{error}"))?;
    let persisted: Vec<(String, String, Option<String>, String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT finding_id,category,target_id,kind,status FROM review_findings
                 WHERE cycle_id=?1 ORDER BY finding_id",
            )
            .map_err(|error| format!("读取周期 findings 失败：{error}"))?;
        let rows = stmt
            .query_map([cycle_id.clone()], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .map_err(|error| format!("读取周期 findings 失败：{error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("读取周期 findings 失败：{error}"))?;
        rows
    };
    if source_row.is_none() && !persisted.is_empty() {
        return conflict();
    }
    // 基线 `itemIndex`：报告项 → findingId 的首现下标映射（仅当周期
    // 存在源行时构建；逻辑与 `buildFindings` 的目标/自由项分支同构）
    let mut item_index: HashMap<String, i64> = HashMap::new();
    if let Some((project_id, epoch, source_id, revision, content_hash)) = source_row {
        let source = ReviewCycleSource {
            project_id,
            epoch,
            source_id,
            revision,
            content_hash,
        };
        for item in &report.items {
            if item.severity == "pass" {
                continue;
            }
            let goal = item
                .goal_id
                .as_deref()
                .and_then(|goal_id| report.goals.iter().find(|goal| goal.id == goal_id));
            if goal.is_some_and(|goal| goal.status == "completed") {
                continue;
            }
            let objective = item.stable_fact_key.is_some() || goal.is_some();
            let (target_id, quote, expected_text, anchored_status, logical_identity) =
                if let Some(goal) = goal {
                    (
                        Some(goal.id.clone()),
                        if goal.evidence.len() == 1 {
                            Some(goal.evidence[0].quote.clone())
                        } else {
                            None
                        },
                        Some(format!("{}\n{}", goal.text, goal.description)),
                        if goal.status == "unknown" {
                            FindingStatus::Unknown
                        } else {
                            FindingStatus::Unresolved
                        },
                        format!("goal:{}", goal.id),
                    )
                } else {
                    (
                        item.stable_fact_key.clone(),
                        item.quote.clone(),
                        None,
                        FindingStatus::Unresolved,
                        if objective {
                            format!(
                                "preflight:{}",
                                item.stable_fact_key
                                    .as_deref()
                                    .expect("objective 自由项必有稳定事实键")
                            )
                        } else {
                            format!("free:{}", item.index)
                        },
                    )
                };
            let descriptor = finding(
                &source,
                &source_content,
                FindingInput {
                    category: item.category.clone(),
                    target_id,
                    kind: if objective {
                        FindingKind::Objective
                    } else {
                        FindingKind::Literary
                    },
                    problem_text: item.description.clone(),
                    expected_text,
                    quote,
                    anchored_status,
                    logical_identity,
                },
            );
            item_index
                .entry(descriptor.finding_id)
                .or_insert(item.index);
        }
    }
    let findings = persisted
        .into_iter()
        .map(|(finding_id, category, target_id, kind, status)| {
            let review_item_index = item_index.get(&finding_id).copied();
            ReviewCycleFindingProjection {
                finding_id,
                review_item_index,
                category,
                kind: FindingKind::parse(&kind),
                status: FindingStatus::parse(&status),
                target_id,
            }
        })
        .collect();
    // 基线尾部条件展开：merge-committed 且合并哈希仍是当前合并快照时，
    // 追加 `recheckDisposition`（`planRecheck` 可按基线抛 CONFLICT）
    let recheck_disposition = if revision_status == "merge-committed" {
        if let Some(merged_hash) = merged_hash.as_deref() {
            if is_current_merged_cycle(&cycle_id, merged_hash, conn)? {
                Some(plan_recheck(&cycle_id, conn)?.disposition)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    Ok(Some(ReviewCycleProjection {
        cycle_id,
        review_id,
        revision_status: RevisionStatus::parse(&revision_status),
        merged_hash,
        recheck_count,
        recheck_disposition,
        findings,
    }))
}

impl M03FindingDescriptor {
    /// `review_findings.status` 写入值（基线 `item.status`）
    fn status_as_str(self) -> &'static str {
        match self.status {
            FindingStatus::Unverified => "unverified",
            FindingStatus::Unresolved => "unresolved",
            FindingStatus::Unknown => "unknown",
            FindingStatus::Resolved => "resolved",
            FindingStatus::AuthorWaived => "author-waived",
        }
    }
}
