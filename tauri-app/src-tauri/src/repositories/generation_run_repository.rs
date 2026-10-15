//! 生成账本仓储（批次 B1）——行为等价平移自
//! `electron/repositories/generation-run-repository.ts`（707 行）。
//!
//! 方法映射（TS → Rust，签名一一对应；DB 句柄由基线「构造时捕获」改为
//! 按调用传入 `&Connection`——命令层在 `project_access` 门禁内打开项目库后
//! 传入，「同一项目库内完成在途写入」的基线语义由该门禁等价保证）：
//!
//! | TS 方法 | Rust 方法 |
//! |---|---|
//! | `get` | `get` |
//! | `authorEditedRoot`（private） | `author_edited_root` |
//! | `assertPlotOutlineWritable` | `assert_plot_outline_writable` |
//! | `isPrivatePlanningHistory` | `is_private_planning_history` |
//! | `plotOutlineAuthorEditReceipt` | `plot_outline_author_edit_receipt` |
//! | `readPlotOutlineAuthorEdit` | `read_plot_outline_author_edit` |
//! | `recordPlotOutlineAuthorEdit` | `record_plot_outline_author_edit` / `_in_transaction` |
//! | `readPlanningContinuation` | `read_planning_continuation` |
//! | `readBlueprintAuthorEdit` | `read_blueprint_author_edit` |
//! | `recordBlueprintAuthorEdit` | `record_blueprint_author_edit` / `_in_transaction` |
//! | `recordPlanningContinuation` | `record_planning_continuation` / `_in_transaction` |
//! | `budget` | `budget` |
//! | `attemptCount` | `attempt_count` |
//! | `hasNonCancelledAttempt` | `has_non_cancelled_attempt` |
//! | `open` | `open` |
//! | `replaceUnstartedBinding` | `replace_unstarted_binding` |
//! | `findInvocation` | `find_invocation` |
//! | `reserve` | `reserve` |
//! | `receipt` | `receipt` |
//! | `hydrateReceipt`（private） | `hydrate_receipt` |
//! | `readState`（private） | `read_state` |
//! | `readRunView` | `read_run_view` |
//! | `markDispatched` | `mark_dispatched` |
//! | `readPlotOutline` | `read_plot_outline` |
//! | `plotOutlineFrom`（private） | `plot_outline_from` |
//! | `assertPlotOutlineRequest` | `assert_plot_outline_request` |
//! | `visibleComposition`（private） | `visible_composition` |
//! | `listDirectoryProgress` | `list_directory_progress` |
//! | `recordDirectoryProgress` | `record_directory_progress` / `_in_transaction` |
//! | `activateRootForCommittedStage` | `activate_root_for_committed_stage` / `_in_transaction` |
//! | `bindDirectoryContinuation` | `bind_directory_continuation` / `_in_transaction` |
//! | `readVisibleComposition` | `read_visible_composition` |
//! | `compositionFrom`（private） | `composition_from` |
//! | `composeVisible` | `compose_visible` |
//! | `settle` | `settle` |
//! | `stopClockIfIdle`（private） | `stop_clock_if_idle` |
//! | `snapshot` | `snapshot` |
//! | `block` | `block` |
//! | `pause` | `pause` / `pause_in_transaction` |
//! | `resume` | `resume` |
//! | `recoverInterrupted` | `recover_interrupted` |
//! | `frozenRoot`（private） | `frozen_root` |
//! | `listCandidates` | `list_candidates` |
//! | `discardCandidate` | `discard_candidate` |
//!
//! # 刻意偏离（均有上游语义理由）
//!
//! - **连接获取**：基线构造时捕获 `getProjectDb()` 句柄并在每次访问时检查
//!   `db.open`（失败抛 `GENERATION_DATABASE_NOT_READY`）；Tauri 仓储按调用
//!   接收 `&Connection`（`repositories/mod.rs` 头注约定），该错误码结构上
//!   不可达，常量保留以对齐基线文案。
//! - **事务**：基线 `db.transaction(fn).immediate()`（IMMEDIATE 锁）；
//!   rusqlite `unchecked_transaction()` 为 DEFERRED——与 fork 内既有仓储
//!   一致（单写入者模型下行为等价）。基线 `if (!db.inTransaction)` 的六处
//!   一致（单写入者模型下行为等价）。基线 `if (!db.inTransaction)` 的六处
//!   前置检查恢复为运行时守卫：公开方法先校验 `conn.is_in_transaction()`
//!   （失败返回对应契约码），再直接于环境事务内执行 `*_in_transaction`；
//!   `bindDirectoryContinuation` 的守卫与行查找合并判定（同基线）。
//!   事务嵌套自动降级为 SAVEPOINT；rusqlite 无嵌套事务，改为
//!   `pause_in_transaction` 直接复用外层事务。
//! - **`GenerationTask` / `TaskBudgetDecision` / `sourceManifest`**：TS 为
//!   鸭子类型（`unknown` / `Record<string, unknown>`），Rust 以
//!   `serde_json::Value` 原样存取，不做结构校验（全保真）。
//! - **正则边界**：`headings` / `validPlotOutlineEntry` 依赖 JS 正则的
//!   前瞻断言（`(?=...)`），`regex` crate 不支持前瞻，改为逐行手工匹配
//!   （算法等价，含可选组的回溯语义）。`plotOutlineRequestContract` 等
//!   双语模板逐字复制自上游。
//! - **字符串偏移**：JS `String.slice` 按 UTF-16 码元；Rust 按 UTF-8 字节
//!   偏移切片。小说正文为 BMP 文本时二者切片结果一致（`headings` 返回
//!   字节偏移）。
//! - **`Number()` 语义**：authorInputs 的 `text` 为字符串，TS 用 `Number()`
//!   转换（接受十进制/十六进制/指数/空白）；Rust `js_number` 手工实现
//!   等价转换，病态输入（超 long 数字）解析失败视为非法。
//! - **`snapshot` 的 `current.artifact!`**：TS 非空断言在 artifact 为 null
//!   时抛 TypeError；Rust 返回 `ARTIFACT_SNAPSHOT_CONFLICT` 错误。
//! - **`assertPlotOutlineRequest` 的 `task?.messages.some`**：TS 对畸形 task
//!   抛 TypeError；Rust 视为条件不满足，返回 `GENERATION_DISPATCH_PLOT_OUTLINE_CURSOR`。
//! - **JSON 编解码失败**：TS 抛 `SyntaxError`（非业务码）；Rust 返回
//!   `GENERATION_JSON_PARSE_ERROR` / `GENERATION_JSON_ENCODE_ERROR` 前缀的
//!   错误字符串。
//! - **默认参数**：TS 构造器注入 `now` / `isFrozen`、`pause(cancel=false)`、
//!   `settle(finishReason=null)` 等默认参数；Rust 全部改为显式参数（仓储
//!   结构体以函数指针持有 `now_ms` / `is_frozen`，`Default` 对应 `Date.now`
//!   与恒 `false`）。
//! - **类型化收窄**：TS 的运行时形状校验（如 `!receipt.committedRange`、
//!   `Object.keys(receipt)` 键集检查）在 Rust 部分由反序列化编译期保证；
//!   键集检查保留为 `check_value_keys` 运行时检查（`$runEffects` 信封的
//!   未知键校验语义）。`receipt.actualTokens` 为 `undefined` 时 TS 键消失，
//!   Rust 以 `skip_serializing_if = "Option::is_none"` 等价。

use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::draft_units::count_draft_units;
use crate::project_access::random_uuid_v4;

// ============================================================================
// 错误码（字符串值逐字对齐上游 fail(...) 调用）
// ============================================================================

const INVALID_ROOT_ACTION: &str = "INVALID_ROOT_ACTION";
const INVALID_BUDGET: &str = "INVALID_BUDGET";
const INVALID_RESERVATION: &str = "INVALID_RESERVATION";
const INVALID_OUTPUT_RESERVATION: &str = "INVALID_OUTPUT_RESERVATION";
const RESERVATION_CONFLICT: &str = "RESERVATION_CONFLICT";
const INVALID_LEDGER: &str = "INVALID_LEDGER";
const ROOT_BUDGET_EXHAUSTED: &str = "ROOT_BUDGET_EXHAUSTED";
const INVALID_ATTEMPT_TRANSITION: &str = "INVALID_ATTEMPT_TRANSITION";
const GENERATION_BINDING_INVALID: &str = "GENERATION_BINDING_INVALID";
/// 结构上不可达（连接按调用传入，永不为 null）；保留以对齐基线文案。
#[allow(dead_code)]
const GENERATION_DATABASE_NOT_READY: &str = "GENERATION_DATABASE_NOT_READY";
#[allow(dead_code)]
const GENERATION_RUN_MISSING: &str = "GENERATION_RUN_MISSING";
const GENERATION_ATTEMPT_MISSING: &str = "GENERATION_ATTEMPT_MISSING";
const GENERATION_PLOT_OUTLINE_ENTRY_INVALID: &str = "GENERATION_PLOT_OUTLINE_ENTRY_INVALID";
const GENERATION_COMPOSITION_OUTPUT_INVALID: &str = "GENERATION_COMPOSITION_OUTPUT_INVALID";
const GENERATION_JSON_PARSE_ERROR: &str = "GENERATION_JSON_PARSE_ERROR";
const GENERATION_PLANNING_HISTORY_READ_ONLY: &str = "GENERATION_PLANNING_HISTORY_READ_ONLY";
const GENERATION_PLOT_OUTLINE_AUTHOR_SAVED: &str = "GENERATION_PLOT_OUTLINE_AUTHOR_SAVED";
const GENERATION_PLOT_OUTLINE_AUTHOR_EDIT_INVALID: &str =
    "GENERATION_PLOT_OUTLINE_AUTHOR_EDIT_INVALID";
const GENERATION_PLOT_OUTLINE_AUTHOR_RECEIPT_INVALID: &str =
    "GENERATION_PLOT_OUTLINE_AUTHOR_RECEIPT_INVALID";
const GENERATION_PLOT_OUTLINE_AUTHOR_RECEIPT_CONFLICT: &str =
    "GENERATION_PLOT_OUTLINE_AUTHOR_RECEIPT_CONFLICT";
#[allow(dead_code)]
const GENERATION_PLOT_OUTLINE_TRANSACTION_REQUIRED: &str =
    "GENERATION_PLOT_OUTLINE_TRANSACTION_REQUIRED";
const GENERATION_PLANNING_CONTINUATION_INVALID: &str = "GENERATION_PLANNING_CONTINUATION_INVALID";
const GENERATION_PLANNING_CONTINUATION_EXISTS: &str = "GENERATION_PLANNING_CONTINUATION_EXISTS";
const GENERATION_PLANNING_CONTINUATION_CONFLICT: &str = "GENERATION_PLANNING_CONTINUATION_CONFLICT";
const GENERATION_BLUEPRINT_AUTHOR_RECEIPT_INVALID: &str =
    "GENERATION_BLUEPRINT_AUTHOR_RECEIPT_INVALID";
const GENERATION_BLUEPRINT_AUTHOR_RECEIPT_CONFLICT: &str =
    "GENERATION_BLUEPRINT_AUTHOR_RECEIPT_CONFLICT";
#[allow(dead_code)]
const GENERATION_DIRECTORY_TRANSACTION_REQUIRED: &str = "GENERATION_DIRECTORY_TRANSACTION_REQUIRED";
const GENERATION_ROOT_MISSING: &str = "GENERATION_ROOT_MISSING";
const GENERATION_NONCE_CONFLICT: &str = "GENERATION_NONCE_CONFLICT";
const GENERATION_PARENT_INVALID: &str = "GENERATION_PARENT_INVALID";
const GENERATION_INVOCATION_CONFLICT: &str = "GENERATION_INVOCATION_CONFLICT";
const GENERATION_INVOCATION_INVALID: &str = "GENERATION_INVOCATION_INVALID";
const GENERATION_DISPATCH_BLOCKED: &str = "GENERATION_DISPATCH_BLOCKED";
const GENERATION_EPOCH_STALE: &str = "GENERATION_EPOCH_STALE";
const GENERATION_DISPATCH_PLOT_OUTLINE_CURSOR: &str = "GENERATION_DISPATCH_PLOT_OUTLINE_CURSOR";
const GENERATION_COMPOSITION_ALGORITHM_INVALID: &str = "GENERATION_COMPOSITION_ALGORITHM_INVALID";
const GENERATION_COMPOSITION_INVALID: &str = "GENERATION_COMPOSITION_INVALID";
const GENERATION_COMPOSITION_SOURCE_INVALID: &str = "GENERATION_COMPOSITION_SOURCE_INVALID";
const GENERATION_COMPOSITION_SOURCE_NOT_TERMINAL: &str =
    "GENERATION_COMPOSITION_SOURCE_NOT_TERMINAL";
const GENERATION_COMPOSITION_SOURCE_UNTRUSTED: &str = "GENERATION_COMPOSITION_SOURCE_UNTRUSTED";
const GENERATION_COMPOSITION_NO_PROGRESS: &str = "GENERATION_COMPOSITION_NO_PROGRESS";
const GENERATION_COMPOSITION_INTEGRITY_FAILED: &str = "GENERATION_COMPOSITION_INTEGRITY_FAILED";
const GENERATION_COMPOSITION_ALGORITHM_CHANGED: &str = "GENERATION_COMPOSITION_ALGORITHM_CHANGED";
const GENERATION_COMPOSITION_HASH_MISMATCH: &str = "GENERATION_COMPOSITION_HASH_MISMATCH";
const GENERATION_COMPOSITION_REGRESSION: &str = "GENERATION_COMPOSITION_REGRESSION";
const GENERATION_DIRECTORY_PROGRESS_INVALID: &str = "GENERATION_DIRECTORY_PROGRESS_INVALID";
const GENERATION_DIRECTORY_PROGRESS_CONFLICT: &str = "GENERATION_DIRECTORY_PROGRESS_CONFLICT";
const GENERATION_DIRECTORY_OUTPUT_INVALID: &str = "GENERATION_DIRECTORY_OUTPUT_INVALID";
const GENERATION_DIRECTORY_ATTEMPT_NOT_TERMINAL: &str = "GENERATION_DIRECTORY_ATTEMPT_NOT_TERMINAL";
const GENERATION_DIRECTORY_CONTINUATION_BLOCKED: &str = "GENERATION_DIRECTORY_CONTINUATION_BLOCKED";
const GENERATION_DIRECTORY_CONTINUATION_EXISTS: &str = "GENERATION_DIRECTORY_CONTINUATION_EXISTS";
const GENERATION_PLOT_OUTLINE_ACCEPT_CONFLICT: &str = "GENERATION_PLOT_OUTLINE_ACCEPT_CONFLICT";
const GENERATION_MATERIAL_DECISION_TOO_LATE: &str = "GENERATION_MATERIAL_DECISION_TOO_LATE";
const GENERATION_BINDING_CONFLICT: &str = "GENERATION_BINDING_CONFLICT";
const GENERATION_RESUME_REFUSED: &str = "GENERATION_RESUME_REFUSED";
const GENERATION_PLANNING_TARGET_INVALID: &str = "GENERATION_PLANNING_TARGET_INVALID";
const GENERATION_PLOT_OUTLINE_POLICY_INVALID: &str = "GENERATION_PLOT_OUTLINE_POLICY_INVALID";
const GENERATION_PLOT_OUTLINE_SOURCE_INVALID: &str = "GENERATION_PLOT_OUTLINE_SOURCE_INVALID";
const GENERATION_PLOT_OUTLINE_ATTEMPT_CONFLICT: &str = "GENERATION_PLOT_OUTLINE_ATTEMPT_CONFLICT";
const GENERATION_PLOT_OUTLINE_PREFIX_INVALID: &str = "GENERATION_PLOT_OUTLINE_PREFIX_INVALID";
const ARTIFACT_INTEGRITY_FAILED: &str = "ARTIFACT_INTEGRITY_FAILED";
const ARTIFACT_ATTEMPT_TERMINAL: &str = "ARTIFACT_ATTEMPT_TERMINAL";
const ARTIFACT_SNAPSHOT_CONFLICT: &str = "ARTIFACT_SNAPSHOT_CONFLICT";
const USAGE_EXCEEDED_RESERVATION: &str = "USAGE_EXCEEDED_RESERVATION";
const DRAFT_TARGET_UNITS_INVALID: &str = "DRAFT_TARGET_UNITS_INVALID";

// ============================================================================
// 共享契约常量（逐字平移自 src/shared/*.ts）
// ============================================================================

const VISIBLE_CONTINUATION_VERSION: &str = "visible-append-v1";
const CONTINUATION_VISIBLE_TAIL_CHARS: usize = 1600;
const MIN_VISIBLE_OVERLAP_CHARS: usize = 48;
const DRAFT_VISIBLE_TEXT_VERSION: &str = "draft-visible-v2";
const DRAFT_CONDENSE_PURPOSE: &str = "chapter-draft-condense";
const DRAFT_RECONCILE_PURPOSE: &str = "chapter-draft-reconcile";
const DRAFT_SHORT_OUTLINE_PURPOSE: &str = "chapter-draft-short-outline";
const DEFAULT_PLANNING_TARGET_UNITS: i64 = 600;
const PLANNING_TARGET_UNITS_LIMIT: i64 = 1000;
const PLOT_OUTLINE_PROTOCOL: &str = "chapter-outline-v3";
const PLOT_OUTLINE_CONTENT: PlotOutlineContent = PlotOutlineContent {
    max_attempts_per_chapter: 2,
};

/// `authorEffectPath`：`$runEffects` 信封在 `binding_json` 内的 JSON 路径。
const AUTHOR_EFFECT_PATH: &str = "$.$runEffects";

// ============================================================================
// 基础辅助
// ============================================================================

fn encode<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| format!("GENERATION_JSON_ENCODE_ERROR: {e}"))
}

fn parse_json<T: DeserializeOwned>(json: &str) -> Result<T, String> {
    serde_json::from_str(json).map_err(|e| format!("GENERATION_JSON_PARSE_ERROR: {e}"))
}

/// `isDeepStrictEqual` 的等价物：两侧序列化为 `serde_json::Value` 后比较
/// （对象键序无关，与 JS 深比较语义一致）。
fn values_equal<T: Serialize + ?Sized, U: Serialize + ?Sized>(a: &T, b: &U) -> bool {
    match (serde_json::to_value(a), serde_json::to_value(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn text_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_content_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// TS `sameProjectEpoch` 的等价物：TS 靠结构类型把 `SourceRef`/`RunBinding`/
/// `RootAction`/`VisibleArtifact` 都当作 `ProjectEpoch` 传入；Rust 侧用
/// `ProjectEpochFields` 显式暴露同一对字段，调用点保持 TS 的多态形态。
trait ProjectEpochFields {
    fn project_epoch_fields(&self) -> (&str, &str);
}

impl ProjectEpochFields for ProjectEpoch {
    fn project_epoch_fields(&self) -> (&str, &str) {
        (&self.project_id, &self.epoch)
    }
}

impl ProjectEpochFields for SourceRef {
    fn project_epoch_fields(&self) -> (&str, &str) {
        (&self.project_id, &self.epoch)
    }
}

impl ProjectEpochFields for RootAction {
    fn project_epoch_fields(&self) -> (&str, &str) {
        (&self.project_id, &self.epoch)
    }
}

impl ProjectEpochFields for VisibleArtifact {
    fn project_epoch_fields(&self) -> (&str, &str) {
        (&self.project_id, &self.epoch)
    }
}

impl ProjectEpochFields for RunBinding {
    fn project_epoch_fields(&self) -> (&str, &str) {
        (&self.project_id, &self.epoch)
    }
}

fn same_project_epoch<T: ProjectEpochFields, U: ProjectEpochFields>(a: &T, b: &U) -> bool {
    let (a_project, a_epoch) = a.project_epoch_fields();
    let (b_project, b_epoch) = b.project_epoch_fields();
    !a_project.is_empty() && !a_epoch.is_empty() && a_project == b_project && a_epoch == b_epoch
}

/// `Number.isSafeInteger` 等价物（JS 安全整数为 ±2^53-1；Rust i64 可能超出）。
fn is_safe_integer_value(value: i64) -> bool {
    (-9007199254740991..=9007199254740991).contains(&value)
}

/// `Number.isSafeInteger` 等价物（`serde_json::Value` 侧；兼容 JSON 浮点 600.0）。
fn safe_integer(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => {
            if let Some(integer) = number.as_i64() {
                Some(integer)
            } else {
                number.as_f64().and_then(safe_integer_from_f64)
            }
        }
        _ => None,
    }
}

fn safe_integer_from_f64(value: f64) -> Option<i64> {
    if value.is_finite()
        && value.fract() == 0.0
        && value >= -9007199254740992.0
        && value <= 9007199254740992.0
    {
        Some(value as i64)
    } else {
        None
    }
}

/// TS `Number()` 的等价物（接受十进制/十六进制/指数/空白；空串为 0）。
fn js_number(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::String(text)) => js_number_from_str(text),
        Some(Value::Number(_)) => safe_integer(value),
        _ => None,
    }
}

fn js_number_from_str(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some(0);
    }
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        return i64::from_str_radix(hex, 16)
            .ok()
            .filter(|&value| is_safe_integer_value(value));
    }
    trimmed.parse::<f64>().ok().and_then(safe_integer_from_f64)
}

/// TS `completeHandle`：对象恰含 4 个键 `projectId/epoch/rootActionId/runId`
/// 且各值为非空（trim 后）字符串。
fn complete_handle(handle: &Value) -> bool {
    let map = match handle {
        Value::Object(map) => map,
        _ => return false,
    };
    map.len() == 4
        && ["projectId", "epoch", "rootActionId", "runId"].iter().all(
            |key| matches!(map.get(*key), Some(Value::String(value)) if !value.trim().is_empty()),
        )
}

/// TS `Object.keys(value).some(key => !allowed.includes(key))` 的否定形式。
fn check_value_keys(value: &Value, allowed: &[&str]) -> bool {
    match value {
        Value::Object(map) => map.keys().all(|key| allowed.contains(&key.as_str())),
        _ => false,
    }
}

fn contains_any_ci(text: &str, needles: &[&str]) -> bool {
    let lower = text.to_lowercase();
    needles
        .iter()
        .any(|needle| lower.contains(&needle.to_lowercase()))
}

// ============================================================================
// 共享契约类型（平移自 src/shared/source-ref.ts）
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEpoch {
    pub project_id: String,
    pub epoch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub project_id: String,
    pub epoch: String,
    pub source_id: String,
    pub revision: i64,
    /// 未修改 UTF-8 字节的 SHA-256；绝不做作者正文归一化。
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<SourceSpan>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceSpan {
    pub start: i64,
    pub end: i64,
    pub unit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrozenInputFingerprint {
    pub chapter_brief_hash: String,
    pub author_guidance_hash: String,
    pub dependency_hash: String,
    pub context_snapshot_hash: String,
    pub template_hash: String,
    pub skill_snapshot_hash: String,
    pub model_lease_revision: String,
    pub policy_hash: String,
    pub output_contract_hash: String,
}

// ============================================================================
// 共享契约类型（平移自 src/shared/generation-contract.ts）
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RootAction {
    pub project_id: String,
    pub epoch: String,
    pub root_action_id: String,
    pub operation: String,
    pub ui_action_nonce: String,
    pub frozen_input_hash: String,
    /// TS 为联合类型 `'active' | 'paused' | 'cancelled' | 'sealed'`；仓储
    /// 只做字符串比较，以 `String` 鸭子类型保真。
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PhysicalAttempt {
    pub attempt_id: String,
    pub reservation_id: String,
    pub root_action_id: String,
    /// TS 为五值联合类型；以 `String` 鸭子类型保真。
    pub status: String,
    pub reserved_tokens: i64,
    pub requested_output_tokens: i64,
    /// 实际用量永不裁剪为估计值；`undefined` 时键消失（等价 TS）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_tokens: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RootBudget {
    pub max_physical_requests: i64,
    pub max_token_liability: i64,
    pub max_output_per_request: i64,
    pub max_active_elapsed_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsagePolicy {
    pub estimator_version: String,
    pub safety_margin_tokens: i64,
    /// TS 为联合类型；以 `String` 鸭子类型保真。
    pub reasoning: String,
    pub can_bound_total_liability: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VisibleArtifact {
    pub project_id: String,
    pub epoch: String,
    pub artifact_id: String,
    pub attempt_id: String,
    pub root_action_id: String,
    pub revision: i64,
    pub text: String,
    pub text_hash: String,
    pub fingerprint: FrozenInputFingerprint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationTransportDiagnostics {
    pub started_at: i64,
    pub elapsed_ms: i64,
    pub first_response_ms: Option<i64>,
    pub last_response_ms: Option<i64>,
    pub last_output_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<i64>,
    /// TS 为四值联合类型；以 `String` 鸭子类型保真。
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause_code: Option<String>,
    pub visible_events: i64,
    pub reasoning_events: i64,
}

/// `rootActionIdempotencyKey`：五个字段均非空（trim 后）且
/// `frozenInputHash` 为内容哈希，否则抛 `INVALID_ROOT_ACTION`。
fn root_action_idempotency_key(action: &RootAction) -> Result<String, String> {
    let non_empty = ![
        action.project_id.as_str(),
        action.epoch.as_str(),
        action.operation.as_str(),
        action.ui_action_nonce.as_str(),
        action.root_action_id.as_str(),
    ]
    .iter()
    .all(|value| !value.trim().is_empty());
    if non_empty || !is_content_hash(&action.frozen_input_hash) {
        return Err(INVALID_ROOT_ACTION.to_string());
    }
    // 重开渲染器/会话不能让同一作者 nonce 变成新的预算根。
    encode(&[
        action.project_id.as_str(),
        action.operation.as_str(),
        action.ui_action_nonce.as_str(),
        action.frozen_input_hash.as_str(),
    ])
}

fn token_liability(attempt: &PhysicalAttempt) -> i64 {
    if attempt.status == "cancelled-before-dispatch" {
        0
    } else if attempt.status == "settled" && attempt.actual_tokens.is_some() {
        attempt.actual_tokens.unwrap_or_default()
    } else {
        attempt.reserved_tokens
    }
}

/// 必须在根账本事务内、插入唯一预留之前运行（序贯校验，错误码顺序逐字对齐）。
fn assert_reservation(
    root: &RootAction,
    policy: &RootBudget,
    attempts: &[PhysicalAttempt],
    next: &PhysicalAttempt,
    elapsed_ms: i64,
) -> Result<(), String> {
    root_action_idempotency_key(root)?;
    let integers = [
        policy.max_physical_requests,
        policy.max_token_liability,
        policy.max_output_per_request,
        policy.max_active_elapsed_ms,
        next.reserved_tokens,
        next.requested_output_tokens,
    ];
    if integers
        .iter()
        .any(|&value| !is_safe_integer_value(value) || value <= 0)
        || !is_safe_integer_value(elapsed_ms)
        || elapsed_ms < 0
    {
        return Err(INVALID_BUDGET.to_string());
    }
    if root.status != "active"
        || next.root_action_id != root.root_action_id
        || next.attempt_id.is_empty()
        || next.reservation_id.is_empty()
        || next.status != "reserved"
    {
        return Err(INVALID_RESERVATION.to_string());
    }
    if next.requested_output_tokens > policy.max_output_per_request
        || next.reserved_tokens < next.requested_output_tokens
    {
        return Err(INVALID_OUTPUT_RESERVATION.to_string());
    }
    if attempts.iter().any(|item| {
        item.root_action_id != root.root_action_id
            || item.attempt_id == next.attempt_id
            || item.reservation_id == next.reservation_id
    }) {
        return Err(RESERVATION_CONFLICT.to_string());
    }
    if attempts.iter().any(|item| {
        !is_safe_integer_value(item.reserved_tokens)
            || item.reserved_tokens <= 0
            || item
                .actual_tokens
                .is_some_and(|actual| !is_safe_integer_value(actual) || actual < 0)
    }) {
        return Err(INVALID_LEDGER.to_string());
    }
    let attempt_ids: HashSet<&str> = attempts
        .iter()
        .map(|item| item.attempt_id.as_str())
        .collect();
    let reservation_ids: HashSet<&str> = attempts
        .iter()
        .map(|item| item.reservation_id.as_str())
        .collect();
    if attempt_ids.len() != attempts.len()
        || reservation_ids.len() != attempts.len()
        || attempts.iter().any(|item| {
            ![
                "reserved",
                "dispatch-marked",
                "settled",
                "unknown",
                "cancelled-before-dispatch",
            ]
            .contains(&item.status.as_str())
        })
    {
        return Err(INVALID_LEDGER.to_string());
    }
    // 硬约束违规在结算时用存储的 usage policy 持久阻断。
    // 估计超支只消耗其全部实际根用量。
    let non_cancelled = attempts
        .iter()
        .filter(|item| item.status != "cancelled-before-dispatch")
        .count() as i64;
    let liability: i64 = attempts.iter().map(token_liability).sum();
    if non_cancelled >= policy.max_physical_requests
        || liability + next.reserved_tokens > policy.max_token_liability
        || elapsed_ms >= policy.max_active_elapsed_ms
    {
        return Err(ROOT_BUDGET_EXHAUSTED.to_string());
    }
    Ok(())
}

fn assert_attempt_transition(before: &str, after: &str) -> Result<(), String> {
    let valid = (before == "reserved"
        && (after == "dispatch-marked" || after == "cancelled-before-dispatch"))
        || (before == "dispatch-marked" && (after == "settled" || after == "unknown"));
    if !valid {
        return Err(INVALID_ATTEMPT_TRANSITION.to_string());
    }
    Ok(())
}

// ============================================================================
// 共享契约类型（平移自 src/shared/plot-outline-contract.ts）
// ============================================================================

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlotOutlineRange {
    pub from: i64,
    pub to: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlotOutlineContent {
    pub max_attempts_per_chapter: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCoreSynopsisExpected {
    pub synopsis: String,
    pub premise: String,
    pub characters_arch: String,
    pub worldbuilding: String,
    pub genre: String,
    pub total_chapters: i64,
    pub words_per_chapter: i64,
    pub writing_language: String,
    pub plot_structure: String,
    pub narrative_pov: String,
    pub global_guidance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlotOutlineAuthorEditReceipt {
    pub kind: String,
    pub operation_id: String,
    pub source_handle: MainGenerationRunHandle,
    pub requested_range: PlotOutlineRange,
    pub committed_range: PlotOutlineRange,
    pub remaining_range: Option<PlotOutlineRange>,
    pub synopsis_hash: String,
    pub payload_hash: String,
    pub idempotent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PlotOutlineCursor {
    Request {
        chapter_number: i64,
        attempt: String,
    },
    Accept {
        chapter_number: i64,
        artifact_ids: Vec<String>,
        text: String,
    },
    Complete,
    Stopped {
        chapter_number: i64,
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlotOutlineProgress {
    pub protocol: String,
    pub range: PlotOutlineRange,
    pub content: PlotOutlineContent,
    pub target_units: i64,
    pub cursor: PlotOutlineCursor,
    pub source_expected: ProjectCoreSynopsisExpected,
    pub confirmed_prefix: String,
    pub composition: Option<VisibleCompositionReceipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlotOutlineAttempt {
    purpose: String,
    status: String,
    finish_reason: Option<String>,
    artifact: Option<PlotOutlineAttemptArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlotOutlineAttemptArtifact {
    artifact_id: String,
    text: String,
    revision: i64,
    text_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
struct PlotOutlinePurpose {
    chapter_number: i64,
    attempt: String,
}

#[derive(Debug, Clone, Copy)]
struct PlotOutlinePolicy {
    range: PlotOutlineRange,
    content: PlotOutlineContent,
}

fn parse_planning_target_units(value: Option<Value>) -> Result<i64, String> {
    match value {
        None => Ok(DEFAULT_PLANNING_TARGET_UNITS),
        Some(value) => match safe_integer(Some(&value)) {
            Some(integer) if integer >= 1 && integer <= PLANNING_TARGET_UNITS_LIMIT => Ok(integer),
            _ => Err(GENERATION_PLANNING_TARGET_INVALID.to_string()),
        },
    }
}

fn planning_target_instruction(kind: &str, target_units: i64, language: &str) -> String {
    let english = language == "en-US";
    let field = if kind == "outline" {
        if english {
            "outline body"
        } else {
            "大纲正文"
        }
    } else if english {
        "keyEvents narrative"
    } else {
        "keyEvents 叙述"
    };
    if english {
        format!(
            "Aim for about {target_units} words per chapter in the {field}. This is a soft target, excluding JSON, field names, identities and relationship metadata. Complete content above or below the target is accepted; do not truncate content to meet it. Keep supporting fields concise."
        )
    } else {
        format!(
            "每章{field}目标约 {target_units} 字。此值是软目标，不包括 JSON、字段名、身份及关系元数据。完整内容可以高于或低于目标，不得为凑字数截断内容。辅助字段可简写。"
        )
    }
}

fn plot_outline_policy(manifest: &Map<String, Value>) -> Result<Option<PlotOutlinePolicy>, String> {
    if manifest.get("operation").and_then(Value::as_str) != Some("generate-plot-outline") {
        return Ok(None);
    }
    let policy = match manifest.get("policy") {
        Some(Value::Object(map)) => map,
        _ => return Ok(None),
    };
    let planning = match policy.get("planning") {
        Some(Value::Object(map)) => map,
        _ => return Ok(None),
    };
    if planning.get("kind").and_then(Value::as_str) != Some("architecture")
        || planning.get("outlineProtocol").and_then(Value::as_str) != Some(PLOT_OUTLINE_PROTOCOL)
    {
        return Ok(None);
    }
    let intent = match planning.get("intent") {
        Some(Value::Object(map)) => map,
        _ => return Err(GENERATION_PLOT_OUTLINE_POLICY_INVALID.to_string()),
    };
    let synopsis_range = match intent.get("synopsisRange") {
        Some(Value::Object(map)) => map,
        _ => return Err(GENERATION_PLOT_OUTLINE_POLICY_INVALID.to_string()),
    };
    let outline_content = match planning.get("outlineContent") {
        Some(Value::Object(map)) => map,
        _ => return Err(GENERATION_PLOT_OUTLINE_POLICY_INVALID.to_string()),
    };
    if outline_content.len() != 1
        || outline_content
            .get("maxAttemptsPerChapter")
            .and_then(Value::as_i64)
            != Some(PLOT_OUTLINE_CONTENT.max_attempts_per_chapter)
    {
        return Err(GENERATION_PLOT_OUTLINE_POLICY_INVALID.to_string());
    }
    let budget = match policy.get("budget") {
        Some(Value::Object(map)) => map,
        _ => return Err(GENERATION_PLOT_OUTLINE_POLICY_INVALID.to_string()),
    };
    let from = safe_integer(synopsis_range.get("from"));
    let to = safe_integer(synopsis_range.get("to"));
    let (Some(from), Some(to)) = (from, to) else {
        return Err(GENERATION_PLOT_OUTLINE_POLICY_INVALID.to_string());
    };
    let max_physical_requests = match budget.get("maxPhysicalRequests") {
        Some(Value::Number(_)) => budget.get("maxPhysicalRequests").and_then(Value::as_f64),
        _ => return Err(GENERATION_PLOT_OUTLINE_POLICY_INVALID.to_string()),
    };
    if from < 1
        || to < from
        || max_physical_requests.is_none_or(|limit| (to - from + 1) as f64 > limit)
    {
        return Err(GENERATION_PLOT_OUTLINE_POLICY_INVALID.to_string());
    }
    Ok(Some(PlotOutlinePolicy {
        range: PlotOutlineRange { from, to },
        content: PLOT_OUTLINE_CONTENT,
    }))
}

fn plot_outline_expected(value: &Value) -> Result<ProjectCoreSynopsisExpected, String> {
    let record = match value {
        Value::Object(map) => map,
        _ => return Err(GENERATION_PLOT_OUTLINE_SOURCE_INVALID.to_string()),
    };
    if safe_integer(record.get("version")) != Some(1) {
        return Err(GENERATION_PLOT_OUTLINE_SOURCE_INVALID.to_string());
    }
    let core = match record.get("core") {
        Some(Value::Object(map)) => map,
        _ => return Err(GENERATION_PLOT_OUTLINE_SOURCE_INVALID.to_string()),
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
        if !matches!(core.get(key), Some(Value::String(_))) {
            return Err(GENERATION_PLOT_OUTLINE_SOURCE_INVALID.to_string());
        }
    }
    if safe_integer(core.get("total_chapters")).is_none()
        || safe_integer(core.get("words_per_chapter")).is_none()
    {
        return Err(GENERATION_PLOT_OUTLINE_SOURCE_INVALID.to_string());
    }
    let text = |key: &str| {
        core.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let writing_language = text("writing_language");
    Ok(ProjectCoreSynopsisExpected {
        synopsis: text("synopsis"),
        premise: text("premise"),
        characters_arch: text("characters_arch"),
        worldbuilding: text("worldbuilding"),
        genre: text("genre"),
        total_chapters: safe_integer(core.get("total_chapters")).unwrap_or_default(),
        words_per_chapter: safe_integer(core.get("words_per_chapter")).unwrap_or_default(),
        writing_language: if writing_language == "en-US" {
            "en-US".to_string()
        } else {
            "zh-CN".to_string()
        },
        plot_structure: text("plot_structure"),
        narrative_pov: text("narrative_pov"),
        global_guidance: text("global_guidance"),
    })
}

fn plot_outline_purpose(chapter_number: i64, attempt: &str) -> String {
    format!("plot-outline:chapter:{chapter_number}:{attempt}")
}

fn parse_plot_outline_purpose(purpose: &str) -> Option<PlotOutlinePurpose> {
    // ^plot-outline:chapter:([1-9]\d*):(normal|compact)$
    let parts: Vec<&str> = purpose.split(':').collect();
    if parts.len() != 4 || parts[0] != "plot-outline" || parts[1] != "chapter" {
        return None;
    }
    let digits = parts[2];
    let first = digits.chars().next()?;
    if !first.is_ascii_digit() || first == '0' || !digits.chars().all(|char| char.is_ascii_digit())
    {
        return None;
    }
    let attempt = parts[3];
    if attempt != "normal" && attempt != "compact" {
        return None;
    }
    let chapter_number = digits.parse::<u64>().ok()?;
    if chapter_number > 9007199254740991 {
        return None;
    }
    Some(PlotOutlinePurpose {
        chapter_number: chapter_number as i64,
        attempt: attempt.to_string(),
    })
}

const CHINESE_DIGITS: &str = "零〇一二两三四五六七八九";

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

fn chinese_unit(char: char) -> Option<i64> {
    match char {
        '十' => Some(10),
        '百' => Some(100),
        '千' => Some(1000),
        _ => None,
    }
}

fn is_chinese_number_char(char: char) -> bool {
    char.is_ascii_digit() || CHINESE_DIGITS.contains(char) || matches!(char, '十' | '百' | '千')
}

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

/// `headings` 的一行匹配结果；`index` / `end` 为 UTF-8 字节偏移
/// （JS 按 UTF-16 码元，BMP 文本切片结果一致）。
#[derive(Debug, Clone, PartialEq)]
struct HeadingMatch {
    from: i64,
    to: i64,
    index: usize,
    end: usize,
    grouped: bool,
}

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

/// 逐行匹配 JS 正则
/// `^[\t ]*(?:#{1,6}[\t ]+|[-*+][\t ]+)?(?:第(NUMBER)(?:[–—-](NUMBER))?章|Chapters?[\t ]+(\d+)(?:[\t ]*[–—-][\t ]*(\d+))?)(?=[\t ]*(?:[:：.．—-]|$))[^\r\n]*`
/// （`regex` crate 不支持前瞻断言，手工实现等价算法，含可选组回溯语义）。
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

/// `^## (?:第{chapter}章|Chapter {chapter})[:：][\t ]*\S[^\r\n]*\r?\n`（对 trimmed 文本）。
fn starts_with_entry_heading(trimmed: &str, chapter: i64) -> bool {
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() < 3 || chars[0] != '#' || chars[1] != '#' || chars[2] != ' ' {
        return false;
    }
    let mut position = 3;
    let chapter_digits = chapter.to_string();
    let chinese: Vec<char> = format!("第{chapter_digits}章").chars().collect();
    let english: Vec<char> = "Chapter ".chars().collect();
    if position + chinese.len() <= chars.len()
        && chars[position..position + chinese.len()] == chinese[..]
    {
        position += chinese.len();
    } else if position + english.len() + chapter_digits.chars().count() <= chars.len()
        && chars[position..position + english.len()] == english[..]
        && chars
            [position + english.len()..position + english.len() + chapter_digits.chars().count()]
            .iter()
            .collect::<String>()
            == chapter_digits
    {
        position += english.len() + chapter_digits.chars().count();
    } else {
        return false;
    }
    if position >= chars.len() || (chars[position] != ':' && chars[position] != '：') {
        return false;
    }
    position += 1;
    while position < chars.len() && (chars[position] == '\t' || chars[position] == ' ') {
        position += 1;
    }
    if position >= chars.len() || chars[position].is_whitespace() {
        return false;
    }
    position += 1;
    while position < chars.len() && chars[position] != '\r' && chars[position] != '\n' {
        position += 1;
    }
    if position < chars.len() && chars[position] == '\r' {
        position += 1;
    }
    position < chars.len() && chars[position] == '\n'
}

/// 正文不得有任何行匹配
/// `^[\t ]*(?:#{1,6}[\t ]+|(?:[-*+][\t ]+)?(?:第[^\r\n]*章|Chapters?\b))`。
fn body_has_heading_like_line(body: &str) -> bool {
    let is_word_char = |char: char| char.is_ascii_alphanumeric() || char == '_';
    for line in body.split('\n') {
        let chars: Vec<char> = line.chars().collect();
        let mut position = 0;
        while position < chars.len() && (chars[position] == '\t' || chars[position] == ' ') {
            position += 1;
        }
        if position < chars.len() && chars[position] == '#' {
            let mut hashes = position;
            while hashes < chars.len() && chars[hashes] == '#' && hashes - position < 6 {
                hashes += 1;
            }
            if hashes - position >= 1
                && hashes < chars.len()
                && (chars[hashes] == '\t' || chars[hashes] == ' ')
            {
                return true;
            }
        }
        let mut marker = position;
        if marker < chars.len()
            && (chars[marker] == '-' || chars[marker] == '*' || chars[marker] == '+')
        {
            let after = marker + 1;
            if after < chars.len() && (chars[after] == '\t' || chars[after] == ' ') {
                let mut after = after;
                while after < chars.len() && (chars[after] == '\t' || chars[after] == ' ') {
                    after += 1;
                }
                marker = after;
            } else {
                marker = position;
            }
        }
        if marker < chars.len()
            && chars[marker] == '第'
            && chars[marker..].iter().any(|&char| char == '章')
        {
            return true;
        }
        if marker + 7 <= chars.len()
            && chars[marker..marker + 7]
                .iter()
                .collect::<String>()
                .eq_ignore_ascii_case("chapter")
        {
            let after = marker + 7;
            let with_s = after < chars.len() && (chars[after] == 's' || chars[after] == 'S');
            let boundary = if with_s { after + 1 } else { after };
            if boundary >= chars.len() || !is_word_char(chars[boundary]) {
                return true;
            }
        }
    }
    false
}

pub fn valid_plot_outline_entry(text: &str, chapter: i64) -> bool {
    if contains_any_ci(
        text,
        &[
            "```",
            "~~~",
            "大纲批次进度",
            "后续概览",
            "Outline batch progress",
            "Later overview",
        ],
    ) {
        return false;
    }
    if !starts_with_entry_heading(text.trim(), chapter) {
        return false;
    }
    let entries = headings(text);
    let Some(heading) = entries.first() else {
        return false;
    };
    if entries.len() != 1
        || heading.grouped
        || heading.from != chapter
        || heading.to != chapter
        || !text[..heading.index].trim().is_empty()
        || text[heading.end..].trim().is_empty()
    {
        return false;
    }
    let body = &text[heading.end..];
    !body_has_heading_like_line(body)
}

fn derive_plot_outline_cursor(
    range: &PlotOutlineRange,
    accepted_count: i64,
    attempts: &[PlotOutlineAttempt],
) -> Result<PlotOutlineCursor, String> {
    let chapter = range.from + accepted_count;
    if chapter > range.to {
        return Ok(PlotOutlineCursor::Complete);
    }
    for kind in ["normal", "compact"] {
        let matching: Vec<&PlotOutlineAttempt> = attempts
            .iter()
            .filter(|attempt| {
                attempt.purpose == plot_outline_purpose(chapter, kind)
                    && attempt.status != "cancelled-before-dispatch"
            })
            .collect();
        if matching.is_empty() {
            return Ok(PlotOutlineCursor::Request {
                chapter_number: chapter,
                attempt: kind.to_string(),
            });
        }
        if matching.len() != 1 {
            return Err(GENERATION_PLOT_OUTLINE_ATTEMPT_CONFLICT.to_string());
        }
        let attempt = matching[0];
        if attempt.status == "reserved" || attempt.status == "dispatch-marked" {
            return Ok(PlotOutlineCursor::Stopped {
                chapter_number: chapter,
                reason: "in-flight".to_string(),
            });
        }
        let finish_reason = attempt.finish_reason.as_deref().unwrap_or_default();
        if finish_reason != "stop" && finish_reason != "length" {
            return Ok(PlotOutlineCursor::Stopped {
                chapter_number: chapter,
                reason: "unknown-completion".to_string(),
            });
        }
        if finish_reason == "stop" {
            if let Some(artifact) = &attempt.artifact {
                if valid_plot_outline_entry(&artifact.text, chapter) {
                    return Ok(PlotOutlineCursor::Accept {
                        chapter_number: chapter,
                        artifact_ids: vec![artifact.artifact_id.clone()],
                        text: artifact.text.clone(),
                    });
                }
            }
        }
    }
    Ok(PlotOutlineCursor::Stopped {
        chapter_number: chapter,
        reason: "attempts-exhausted".to_string(),
    })
}

fn join_plot_outline_entries(prefix: &str, next: &str) -> String {
    let parts: Vec<&str> = [prefix.trim(), next.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect();
    parts.join("\n\n")
}

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

fn plot_outline_confirmed_prefix(
    expected: &ProjectCoreSynopsisExpected,
    range: &PlotOutlineRange,
) -> Result<String, String> {
    let entries = headings(&expected.synopsis);
    if entries.iter().any(|entry| {
        (entry.from < range.from && entry.to >= range.from)
            || (entry.from <= range.to && entry.to > range.to)
    }) {
        return Err(GENERATION_PLOT_OUTLINE_PREFIX_INVALID.to_string());
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
            return Err(GENERATION_PLOT_OUTLINE_PREFIX_INVALID.to_string());
        }
        next = entry.to + 1;
    }
    if next != range.from || before.is_empty() {
        return Err(GENERATION_PLOT_OUTLINE_PREFIX_INVALID.to_string());
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
        return Err(GENERATION_PLOT_OUTLINE_PREFIX_INVALID.to_string());
    }
    let prefix = expected.synopsis[title.len()..expected.synopsis.len() - suffix.len()]
        .trim()
        .to_string();
    if prefix.is_empty()
        || render_plot_outline_synopsis(&prefix, range.from - 1, expected) != expected.synopsis
    {
        return Err(GENERATION_PLOT_OUTLINE_PREFIX_INVALID.to_string());
    }
    Ok(prefix)
}

pub fn plot_outline_request_contract(
    progress: &PlotOutlineProgress,
    chapter: i64,
    attempt: &str,
) -> String {
    let prefix = join_plot_outline_entries(
        &progress.confirmed_prefix,
        progress
            .composition
            .as_ref()
            .map(|composition| composition.text.as_str())
            .unwrap_or_default(),
    );
    let english = progress.source_expected.writing_language == "en-US";
    let instruction = planning_target_instruction(
        "outline",
        progress.target_units,
        &progress.source_expected.writing_language,
    );
    let compact = attempt == "compact";
    if english {
        let mut contract = format!(
            "[Accepted outline prefix]\n{prefix}\n[/Accepted outline prefix]\n\nWrite only Chapter {chapter}, inside the selected range {}-{}. Use exactly one level-two Markdown heading \"## Chapter {chapter}: Title\" with Arabic chapter digits, followed by a non-empty body. No chapter groups, list headings, other chapters, preface, progress line or later overview. {instruction} Preserve all author facts, required events and ending requirements. Do not rewrite the accepted prefix.",
            progress.range.from, progress.range.to
        );
        if compact {
            contract.push_str("\nRebuild this chapter once from the original frozen facts and accepted prefix. The failed candidate is not a source. Produce a complete concise replacement, not a continuation or a cut-off excerpt.");
        }
        contract
    } else {
        let mut contract = format!(
            "【已接受大纲前缀】\n{prefix}\n【已接受大纲前缀结束】\n\n本次只写第 {chapter} 章，所选范围为第 {}–{} 章。只用一个二级 Markdown 章标题“## 第{chapter}章：标题”，章号使用阿拉伯数字，标题下一行写非空正文。不得输出章组、列表标题、其他章、开场说明、进度行或后续概览。{instruction}保留作者硬事实、明确事件和尾部要求，不改写已接受前缀。",
            progress.range.from, progress.range.to
        );
        if compact {
            contract.push_str("\n本次从原始冻结事实和已接受前缀重新构建此章。失败候选不是来源。输出一份完整简洁的替代大纲，不得续接失败候选或截取其片段。");
        }
        contract
    }
}

// ============================================================================
// 共享契约类型（平移自 src/shared/generation-owner-contract.ts）
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MainGenerationRunHandle {
    pub project_id: String,
    pub epoch: String,
    pub root_action_id: String,
    pub run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GenerationAuthorInput {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChapterRange {
    pub start_chapter: i64,
    pub end_chapter: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryGenerationProgress {
    pub operation_id: String,
    pub payload_hash: String,
    pub source_handle: MainGenerationRunHandle,
    pub requested_range: ChapterRange,
    pub committed_range: ChapterRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_range: Option<ChapterRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_handle: Option<MainGenerationRunHandle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_inputs: Option<Vec<GenerationAuthorInput>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlanningContinuationReceipt {
    pub kind: String,
    pub source_handle: MainGenerationRunHandle,
    pub next_handle: MainGenerationRunHandle,
    pub request_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintAuthorEditReceipt {
    pub kind: String,
    pub operation_id: String,
    pub source_handle: MainGenerationRunHandle,
    pub requested_range: PlotOutlineRange,
    pub committed_range: PlotOutlineRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_range: Option<PlotOutlineRange>,
    pub payload_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompositionSource {
    pub artifact_id: String,
    pub revision: i64,
    pub text_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompositionChapter {
    pub chapter_number: i64,
    pub artifact_ids: Vec<String>,
    pub text_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VisibleCompositionReceipt {
    pub algorithm: String,
    pub text: String,
    pub text_hash: String,
    pub artifact_ids: Vec<String>,
    pub sources: Vec<CompositionSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapters: Option<Vec<CompositionChapter>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_inputs: Option<Vec<GenerationAuthorInput>>,
}

// ============================================================================
// 共享契约函数（平移自 src/shared/visible-continuation.ts）
// ============================================================================

fn last_chars(text: &str, count: usize) -> Vec<char> {
    let chars: Vec<char> = text.chars().collect();
    let start = chars.len().saturating_sub(count);
    chars[start..].to_vec()
}

fn first_chars(text: &str, count: usize) -> Vec<char> {
    text.chars().take(count).collect()
}

fn overlapping_visible_prefix_length(existing_text: &str, addition: &str) -> usize {
    let existing_tail: Vec<char> = last_chars(existing_text, CONTINUATION_VISIBLE_TAIL_CHARS)
        .into_iter()
        .filter(|char| !char.is_whitespace())
        .collect();
    let addition_head: Vec<char> = first_chars(addition, CONTINUATION_VISIBLE_TAIL_CHARS)
        .into_iter()
        .filter(|char| !char.is_whitespace())
        .collect();
    let maximum = existing_tail.len().min(addition_head.len());
    for length in (MIN_VISIBLE_OVERLAP_CHARS..=maximum).rev() {
        if existing_tail[existing_tail.len() - length..] == addition_head[..length] {
            return length;
        }
    }
    0
}

fn remove_leading_non_whitespace_characters(text: &str, count: usize) -> String {
    if count == 0 {
        return text.to_string();
    }
    let mut consumed = 0;
    for (index, char) in text.char_indices() {
        if !char.is_whitespace() {
            consumed += 1;
        }
        if consumed >= count {
            return text[index + char.len_utf8()..].trim_start().to_string();
        }
    }
    String::new()
}

/// 输入已是可见文本；源 artifact 字节绝不修改。
pub fn compose_visible_continuation(existing: &str, addition: &str) -> String {
    let visible_existing = existing.trim();
    let visible_addition = addition.trim();
    let overlap = overlapping_visible_prefix_length(visible_existing, visible_addition);
    let new_visible_text = remove_leading_non_whitespace_characters(visible_addition, overlap);
    let parts: Vec<&str> = [visible_existing, new_visible_text.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect();
    parts.join("\n\n").trim().to_string()
}

// ============================================================================
// 共享契约函数（平移自 src/shared/draft-visible-text.ts）
// ============================================================================

static RE_THINKING_PAIR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<thinking>[\s\S]*?(?:</thinking>|$)").unwrap());
static RE_ORPHAN_THINKING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</think>").unwrap());
static RE_THINKING_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</?think>").unwrap());
static RE_REASONING_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?iu)^\s*(?:思考|推理|分析|reasoning|analysis)").unwrap());
static RE_STRUCTURED_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?iu)^\s*(?:```(?:json)?\s*)?[{\[]").unwrap());
static RE_DRAFT_CONTINUE_HINT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^\s*(?:点我继续生成后续内容|继续生成后续内容|请点击继续|未完待续)\s*$")
        .unwrap()
});
static RE_NEWLINES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());
static RE_PARAGRAPH_SPLIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n\s*\n").unwrap());
static RE_WHITESPACE_RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static RE_THINK_TAG_BOUNDARY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?iu)</?think\b").unwrap());

fn is_draft_visible_text_version(value: &str) -> bool {
    value == "draft-visible-v1" || value == DRAFT_VISIBLE_TEXT_VERSION
}

fn strip_draft_thinking_tags(text: &str) -> String {
    if text.is_empty() {
        return text.to_string();
    }
    // 支持只有 </think> 没有闭合标签的情况。
    let without_paired_thinking = RE_THINKING_PAIR.replace_all(text, "").into_owned();
    let Some(orphan) = RE_ORPHAN_THINKING.find(&without_paired_thinking) else {
        return RE_THINKING_TAG
            .replace_all(&without_paired_thinking, "")
            .trim()
            .to_string();
    };
    let hidden_prefix = &without_paired_thinking[..orphan.start()];
    let visible_suffix = &without_paired_thinking[orphan.end()..];
    // 缺失开始标签仅在以下情况可安全视为隐藏推理：前缀自证为推理，
    // 或其后紧跟结构化输出。否则保留普通正文，仅删除畸形标签。
    let looks_like_reasoning = RE_REASONING_PREFIX.is_match(hidden_prefix);
    let has_structured_visible_suffix = RE_STRUCTURED_SUFFIX.is_match(visible_suffix);
    let cleaned = if looks_like_reasoning || has_structured_visible_suffix {
        visible_suffix.to_string()
    } else {
        format!("{hidden_prefix}{visible_suffix}")
    };
    RE_THINKING_TAG.replace_all(&cleaned, "").trim().to_string()
}

pub fn sanitize_draft_text(text: &str, version: &str) -> String {
    let cleaned = strip_draft_thinking_tags(text);
    let cleaned = RE_DRAFT_CONTINUE_HINT
        .replace_all(&cleaned, "")
        .into_owned();
    let cleaned = RE_NEWLINES.replace_all(&cleaned, "\n\n").into_owned();
    let cleaned = cleaned.trim().to_string();
    let paragraphs: Vec<String> = RE_PARAGRAPH_SPLIT
        .split(&cleaned)
        .map(|paragraph| paragraph.trim().to_string())
        .filter(|paragraph| !paragraph.is_empty())
        .collect();
    if version != "draft-visible-v1" {
        return paragraphs.join("\n\n");
    }
    // 既有 v1 收据在恢复时必须保留其精确投影与哈希。
    let mut seen = HashSet::new();
    paragraphs
        .into_iter()
        .filter(|paragraph| {
            let key = RE_WHITESPACE_RUN.replace_all(paragraph, "").into_owned();
            if key.chars().count() >= 40 && seen.contains(&key) {
                return false;
            }
            if key.chars().count() >= 40 {
                seen.insert(key);
            }
            true
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn compose_draft_visible_continuation(existing: &str, addition: &str, version: &str) -> String {
    sanitize_draft_text(
        &compose_visible_continuation(
            &sanitize_draft_text(existing, version),
            &sanitize_draft_text(addition, version),
        ),
        version,
    )
}

// ============================================================================
// 共享契约函数（平移自 src/shared/draft-units.ts）
// ============================================================================

#[derive(Debug, Clone, Copy)]
struct DraftTargetUnitRange {
    /// 契约字段（draft-units.ts）：draft 目标区间下限；B6b 命令层校验使用，当前仅测试断言读取。
    #[allow(dead_code)]
    minimum: i64,
    maximum: i64,
}

fn draft_target_unit_range(target_units: i64) -> Result<DraftTargetUnitRange, String> {
    if !is_safe_integer_value(target_units) || target_units < 1 {
        return Err(DRAFT_TARGET_UNITS_INVALID.to_string());
    }
    let target = target_units as f64;
    Ok(DraftTargetUnitRange {
        minimum: (target * 0.7).floor() as i64,
        maximum: (target * 1.3).ceil() as i64,
    })
}

// ============================================================================
// 仓储类型
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunBinding {
    pub project_id: String,
    pub epoch: String,
    pub fingerprint: FrozenInputFingerprint,
    pub context_snapshot_id: String,
    pub source_manifest: Map<String, Value>,
    pub source_refs: Vec<SourceRef>,
    /// TS `const { $runEffects, ...frozen }` 解构的等价物：未知键（含
    /// `$runEffects`）保留在 `extra` 中，`get()` 时剥离 `$runEffects`。
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenGenerationRunRequest {
    pub project_id: String,
    pub epoch: String,
    pub fingerprint: FrozenInputFingerprint,
    pub context_snapshot_id: String,
    pub source_manifest: Map<String, Value>,
    pub source_refs: Vec<SourceRef>,
    pub operation: String,
    pub ui_action_nonce: String,
    pub frozen_input_hash: String,
    pub budget: RootBudget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_root_action_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DurableGenerationRun {
    pub run_id: String,
    pub root_action_id: String,
    pub binding: RunBinding,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationBudgetReceipt {
    pub root: RootAction,
    pub policy: RootBudget,
    pub active_elapsed_ms: i64,
    pub attempts: Vec<PhysicalAttempt>,
    pub blocked_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentResponse {
    pub version: i64,
    pub visible_text: String,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub name: String,
    pub arguments: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationUsageReceipt {
    pub policy: ProviderUsagePolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_tokens: Option<i64>,
    pub trusted: bool,
    /// 主进程解析的执行控制，永不属于可见小说 artifact。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_response: Option<AgentResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactIdentity {
    pub artifact_id: String,
    pub epoch: String,
    pub fingerprint: FrozenInputFingerprint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<GenerationUsageReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<GenerationTransportDiagnostics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationExecutionReceipt {
    pub run: DurableGenerationRun,
    pub attempt: PhysicalAttempt,
    pub artifact: Option<VisibleArtifact>,
    pub budget: GenerationBudgetReceipt,
    pub result: Option<ResultPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<GenerationTransportDiagnostics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_decision: Option<Value>,
    /// 基线 `unsavedTail?: string`（generation-run-repository.ts:63）：
    /// 存储失败时主进程保留的可复制尾文本（纯运行时投影，
    /// 不落库）。B3-2b 起由 `generation_run_service` 在
    /// storageFailed 路径附加。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsaved_tail: Option<String>,
}

/// `receiptSelect` 投影的 usage 形状（`{purpose, artifactIdentity, result, budgetDecision}`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_identity: Option<ArtifactIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ResultPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_decision: Option<Value>,
}

#[derive(Debug, Clone)]
struct ReceiptWithUsage {
    usage: ReceiptUsage,
    receipt: GenerationExecutionReceipt,
}

#[derive(Debug, Clone)]
pub struct RunView {
    pub budget: GenerationBudgetReceipt,
    pub plot_outline: Option<PlotOutlineProgress>,
    pub attempts: Vec<AttemptView>,
}

#[derive(Debug, Clone)]
pub struct AttemptView {
    pub usage: ReceiptUsage,
    pub receipt: GenerationExecutionReceipt,
    pub candidate: bool,
}

#[derive(Debug, Clone)]
struct ReceiptRow {
    attempt_id: String,
    run_id: String,
    ordinal: i64,
    attempt_json: String,
    usage_receipt_json: String,
    artifact_id: Option<String>,
    artifact_run_id: Option<String>,
    artifact_json: Option<String>,
    revision: Option<i64>,
    artifact_status: Option<String>,
}

fn map_receipt_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReceiptRow> {
    Ok(ReceiptRow {
        attempt_id: row.get("attempt_id")?,
        run_id: row.get("run_id")?,
        ordinal: row.get("ordinal")?,
        attempt_json: row.get("attempt_json")?,
        usage_receipt_json: row.get("usage_receipt_json")?,
        artifact_id: row.get("artifact_id")?,
        artifact_run_id: row.get("artifact_run_id")?,
        artifact_json: row.get("artifact_json")?,
        revision: row.get("revision")?,
        artifact_status: row.get("artifact_status")?,
    })
}

const RECEIPT_SELECT: &str = "SELECT t.attempt_id,t.run_id,t.rowid AS ordinal,t.attempt_json,
    json_object('purpose',json_extract(t.usage_receipt_json,'$.purpose'),
        'artifactIdentity',json_extract(t.usage_receipt_json,'$.artifactIdentity'),
        'result',json_extract(t.usage_receipt_json,'$.result'),
        'budgetDecision',json_extract(t.usage_receipt_json,'$.budgetDecision')) AS usage_receipt_json,
    a.artifact_id,a.run_id AS artifact_run_id,a.artifact_json,a.revision,a.status AS artifact_status
    FROM generation_attempts t LEFT JOIN generation_artifacts a ON a.attempt_id=t.attempt_id";

/// 冻结表名（`PortableRuntimeFreezeTable` 的生成账本子集，
/// 平移自 `electron/services/portable-runtime-freeze.ts`）。
pub type GenerationFreezeTable = &'static str;

pub const FREEZE_TABLE_GENERATION_ROOTS: GenerationFreezeTable = "generation_roots";
pub const FREEZE_TABLE_GENERATION_RUNS: GenerationFreezeTable = "generation_runs";
pub const FREEZE_TABLE_GENERATION_ATTEMPTS: GenerationFreezeTable = "generation_attempts";

#[derive(Clone, Copy)]
pub struct GenerationRunRepository {
    /// 基线构造器注入的 `now`（默认 `Date.now`）。
    pub now_ms: fn() -> i64,
    /// 基线构造器注入的 `isFrozen`（默认恒 `false`）。
    pub is_frozen: fn(GenerationFreezeTable, &str) -> bool,
}

impl Default for GenerationRunRepository {
    fn default() -> Self {
        Self {
            now_ms: default_now_ms,
            is_frozen: default_is_frozen,
        }
    }
}

fn default_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn default_is_frozen(_table: GenerationFreezeTable, _record_id: &str) -> bool {
    false
}

/// 事务包装（基线 `db.transaction(fn).immediate()` 的等价物；
/// 失败时 `Transaction` drop 即回滚，与基线语义一致）。
fn in_transaction<T>(
    conn: &Connection,
    operation: impl FnOnce(&Transaction<'_>) -> Result<T, String>,
) -> Result<T, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let result = operation(&tx)?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(result)
}

/// 生成账本仓储。所有方法行为等价平移自
/// `electron/repositories/generation-run-repository.ts`。
///
/// 仓储只接收 `&Connection`（`repositories/mod.rs` 头注约定），不持有全局
/// 状态；命令层负责会话门禁与错误形状。基线「构造时捕获 DB 句柄、在途写入
/// 不被项目切换重定向」的语义由 `project_access` 的项目库门禁等价保证。
pub struct GenerationRunRepositoryInner;

impl GenerationRunRepository {
    // ---------------------------------------------------------------------
    // 读取与校验
    // ---------------------------------------------------------------------

    pub fn get(&self, conn: &Connection, run_id: &str) -> Result<DurableGenerationRun, String> {
        let (root_action_id, binding_json, status) = conn
            .query_row(
                "SELECT root_action_id,binding_json,status FROM generation_runs WHERE run_id=?",
                [run_id],
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
            .ok_or_else(|| GENERATION_RUN_MISSING.to_string())?;
        let mut binding: RunBinding = parse_json(&binding_json)?;
        // const { $runEffects, ...frozen } = JSON.parse(row.binding_json)
        binding.extra.remove("$runEffects");
        Ok(DurableGenerationRun {
            run_id: run_id.to_string(),
            root_action_id,
            binding,
            status,
        })
    }

    fn author_edited_root(&self, conn: &Connection, root_id: &str) -> Result<bool, String> {
        conn.query_row(
            "SELECT 1 FROM generation_runs WHERE root_action_id=? AND json_type(binding_json,?) IS NOT NULL LIMIT 1",
            params![root_id, AUTHOR_EFFECT_PATH],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| error.to_string())
        .map(|value| value.is_some())
    }

    pub fn assert_plot_outline_writable(
        &self,
        conn: &Connection,
        run_id: &str,
    ) -> Result<(), String> {
        if self.is_private_planning_history(conn, run_id)? {
            return Err(GENERATION_PLANNING_HISTORY_READ_ONLY.to_string());
        }
        if self.author_edited_root(conn, &self.get(conn, run_id)?.root_action_id)? {
            return Err(GENERATION_PLOT_OUTLINE_AUTHOR_SAVED.to_string());
        }
        Ok(())
    }

    pub fn is_private_planning_history(
        &self,
        conn: &Connection,
        run_id: &str,
    ) -> Result<bool, String> {
        conn.query_row(
            "SELECT 1 FROM generation_runs WHERE root_action_id=(SELECT root_action_id FROM generation_runs WHERE run_id=?) AND json_extract(binding_json,'$.sourceManifest.policy.planning.outlineProtocol')='chapter-parts-v2' LIMIT 1",
            [run_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| error.to_string())
        .map(|value| value.is_some())
    }

    pub fn plot_outline_author_edit_receipt(
        &self,
        conn: &Connection,
        run_id: &str,
        operation_id: &str,
        committed_range: &PlotOutlineRange,
        synopsis_hash: &str,
    ) -> Result<PlotOutlineAuthorEditReceipt, String> {
        let run = self.get(conn, run_id)?;
        let policy = plot_outline_policy(&run.binding.source_manifest)?;
        let Some(policy) = policy else {
            return Err(GENERATION_PLOT_OUTLINE_AUTHOR_EDIT_INVALID.to_string());
        };
        if !operation_id.chars().enumerate().all(|(index, char)| {
            (index == 0 && char.is_ascii_alphanumeric())
                || (index > 0 && (char.is_ascii_alphanumeric() || matches!(char, ':' | '_' | '-')))
        }) || operation_id.chars().count() > 128
            || !is_content_hash(synopsis_hash)
            || committed_range.from != policy.range.from
            || !is_safe_integer_value(committed_range.to)
            || committed_range.to < committed_range.from
            || committed_range.to > policy.range.to
        {
            return Err(GENERATION_PLOT_OUTLINE_AUTHOR_EDIT_INVALID.to_string());
        }
        let review_cycles_exists = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='review_cycles'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .is_some();
        if review_cycles_exists {
            let has_review_cycle = conn
                .query_row(
                    "SELECT 1 FROM review_cycles WHERE root_action_id=? LIMIT 1",
                    [run.root_action_id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?
                .is_some();
            if has_review_cycle {
                return Err(GENERATION_PLOT_OUTLINE_AUTHOR_EDIT_INVALID.to_string());
            }
        }
        let source_handle = MainGenerationRunHandle {
            project_id: run.binding.project_id.clone(),
            epoch: run.binding.epoch.clone(),
            root_action_id: run.root_action_id.clone(),
            run_id: run_id.to_string(),
        };
        let expected = plot_outline_expected(
            run.binding
                .source_manifest
                .get("plotOutlineSource")
                .unwrap_or(&Value::Null),
        )?;
        let payload_hash = text_hash(&encode(&(
            &source_handle,
            operation_id,
            committed_range.from,
            committed_range.to,
            synopsis_hash,
            &expected,
        ))?);
        let remaining_range = if committed_range.to < policy.range.to {
            Some(PlotOutlineRange {
                from: committed_range.to + 1,
                to: policy.range.to,
            })
        } else {
            None
        };
        Ok(PlotOutlineAuthorEditReceipt {
            kind: "author-edit".to_string(),
            operation_id: operation_id.to_string(),
            source_handle,
            requested_range: policy.range,
            committed_range: *committed_range,
            remaining_range,
            synopsis_hash: synopsis_hash.to_string(),
            payload_hash,
            idempotent: false,
        })
    }

    pub fn read_plot_outline_author_edit(
        &self,
        conn: &Connection,
        run_id: &str,
    ) -> Result<Option<PlotOutlineAuthorEditReceipt>, String> {
        let stored = conn
            .query_row(
                "SELECT json_extract(binding_json,?) FROM generation_runs WHERE run_id=?",
                params![AUTHOR_EFFECT_PATH, run_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .flatten();
        let Some(stored) = stored else {
            return Ok(None);
        };
        let value: Value = parse_json(&stored)?;
        if !check_value_keys(
            &value,
            &[
                "plotOutlineAuthorEdit",
                "blueprintAuthorEdit",
                "planningContinuation",
            ],
        ) {
            return Err(GENERATION_PLOT_OUTLINE_AUTHOR_RECEIPT_INVALID.to_string());
        }
        let Some(receipt_value) = value.get("plotOutlineAuthorEdit").cloned() else {
            return Ok(None);
        };
        let receipt: PlotOutlineAuthorEditReceipt = serde_json::from_value(receipt_value)
            .map_err(|_| GENERATION_PLOT_OUTLINE_AUTHOR_RECEIPT_INVALID.to_string())?;
        if !values_equal(
            &receipt,
            &self.plot_outline_author_edit_receipt(
                conn,
                run_id,
                &receipt.operation_id,
                &receipt.committed_range,
                &receipt.synopsis_hash,
            )?,
        ) {
            return Err(GENERATION_PLOT_OUTLINE_AUTHOR_RECEIPT_INVALID.to_string());
        }
        Ok(Some(receipt))
    }

    pub fn record_plot_outline_author_edit(
        &self,
        conn: &Connection,
        receipt: &PlotOutlineAuthorEditReceipt,
    ) -> Result<(), String> {
        if conn.is_autocommit() {
            return Err(GENERATION_PLOT_OUTLINE_TRANSACTION_REQUIRED.to_string());
        }
        self.record_plot_outline_author_edit_in_transaction(conn, receipt)
    }

    fn record_plot_outline_author_edit_in_transaction(
        &self,
        tx: &Connection,
        receipt: &PlotOutlineAuthorEditReceipt,
    ) -> Result<(), String> {
        let run_id = receipt.source_handle.run_id.clone();
        self.assert_plot_outline_writable(tx, &run_id)?;
        let recomputed = self.plot_outline_author_edit_receipt(
            tx,
            &run_id,
            &receipt.operation_id,
            &receipt.committed_range,
            &receipt.synopsis_hash,
        )?;
        if !values_equal(receipt, &recomputed) {
            return Err(GENERATION_PLOT_OUTLINE_AUTHOR_RECEIPT_INVALID.to_string());
        }
        let original: String = tx
            .query_row(
                "SELECT binding_json FROM generation_runs WHERE run_id=?",
                [&run_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let mut binding: Value = parse_json(&original)?;
        binding
            .as_object_mut()
            .ok_or_else(|| GENERATION_PLOT_OUTLINE_AUTHOR_RECEIPT_CONFLICT.to_string())?
            .insert(
                "$runEffects".to_string(),
                json!({ "plotOutlineAuthorEdit": receipt }),
            );
        let updated = encode(&binding)?;
        let changes = tx
            .execute(
                "UPDATE generation_runs SET binding_json=? WHERE run_id=? AND binding_json=?",
                params![updated, run_id, original],
            )
            .map_err(|error| error.to_string())?;
        if changes != 1 {
            return Err(GENERATION_PLOT_OUTLINE_AUTHOR_RECEIPT_CONFLICT.to_string());
        }
        Ok(())
    }

    pub fn read_planning_continuation(
        &self,
        conn: &Connection,
        run_id: &str,
    ) -> Result<Option<PlanningContinuationReceipt>, String> {
        let stored = conn
            .query_row(
                "SELECT json_extract(binding_json,?) FROM generation_runs WHERE run_id=?",
                params![format!("{AUTHOR_EFFECT_PATH}.planningContinuation"), run_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .flatten();
        let Some(stored) = stored else {
            return Ok(None);
        };
        let value: Value = parse_json(&stored)?;
        let receipt: PlanningContinuationReceipt = serde_json::from_value(value.clone())
            .map_err(|_| GENERATION_PLANNING_CONTINUATION_INVALID.to_string())?;
        let source = self.get(conn, run_id)?;
        if receipt.kind != "planning-continuation"
            || !complete_handle(value.get("sourceHandle").unwrap_or(&Value::Null))
            || !complete_handle(value.get("nextHandle").unwrap_or(&Value::Null))
            || !is_content_hash(&receipt.request_hash)
            || !check_value_keys(
                &value,
                &["kind", "sourceHandle", "nextHandle", "requestHash"],
            )
            || receipt.source_handle.run_id != run_id
            || receipt.source_handle.root_action_id != source.root_action_id
            || receipt.source_handle.project_id != source.binding.project_id
            || receipt.next_handle.project_id != source.binding.project_id
            || receipt.next_handle.root_action_id == source.root_action_id
        {
            return Err(GENERATION_PLANNING_CONTINUATION_INVALID.to_string());
        }
        let next = self.get(conn, &receipt.next_handle.run_id)?;
        if next.root_action_id != receipt.next_handle.root_action_id
            || next.binding.project_id != source.binding.project_id
        {
            return Err(GENERATION_PLANNING_CONTINUATION_INVALID.to_string());
        }
        let mut receipt = receipt;
        receipt.next_handle.epoch = next.binding.epoch.clone();
        Ok(Some(receipt))
    }

    pub fn read_blueprint_author_edit(
        &self,
        conn: &Connection,
        run_id: &str,
    ) -> Result<Option<BlueprintAuthorEditReceipt>, String> {
        let stored = conn
            .query_row(
                "SELECT json_extract(binding_json,?) FROM generation_runs WHERE run_id=?",
                params![format!("{AUTHOR_EFFECT_PATH}.blueprintAuthorEdit"), run_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .flatten();
        let Some(stored) = stored else {
            return Ok(None);
        };
        let value: Value = parse_json(&stored)?;
        let receipt: BlueprintAuthorEditReceipt = serde_json::from_value(value.clone())
            .map_err(|_| GENERATION_BLUEPRINT_AUTHOR_RECEIPT_INVALID.to_string())?;
        let run = self.get(conn, run_id)?;
        let input = run
            .binding
            .source_manifest
            .get("authorInputs")
            .and_then(Value::as_array)
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("id").and_then(Value::as_str) == Some("directory:requested-range")
                })
            });
        let range: Option<ChapterRange> = input
            .and_then(|item| item.get("text").and_then(Value::as_str))
            .map(|text| parse_json::<Value>(text))
            .transpose()?
            .and_then(|value| serde_json::from_value(value).ok());
        let Some(range) = range else {
            return Err(GENERATION_BLUEPRINT_AUTHOR_RECEIPT_INVALID.to_string());
        };
        if receipt.kind != "blueprint-author-edit"
            || run
                .binding
                .source_manifest
                .get("operation")
                .and_then(Value::as_str)
                != Some("chapter-blueprint-directory")
            || !complete_handle(value.get("sourceHandle").unwrap_or(&Value::Null))
            || receipt.source_handle.run_id != run_id
            || receipt.source_handle.root_action_id != run.root_action_id
            || receipt.source_handle.project_id != run.binding.project_id
            || !is_content_hash(&receipt.payload_hash)
            || !values_equal(
                &receipt.requested_range,
                &PlotOutlineRange {
                    from: range.start_chapter,
                    to: range.end_chapter,
                },
            )
            || !is_safe_integer_value(receipt.committed_range.from)
            || !is_safe_integer_value(receipt.committed_range.to)
            || receipt.committed_range.from < range.start_chapter
            || receipt.committed_range.to < receipt.committed_range.from
            || receipt.committed_range.to > range.end_chapter
            || !values_equal(
                &receipt.remaining_range,
                &if receipt.committed_range.to < range.end_chapter {
                    Some(PlotOutlineRange {
                        from: receipt.committed_range.to + 1,
                        to: range.end_chapter,
                    })
                } else {
                    None
                },
            )
            || !check_value_keys(
                &value,
                &[
                    "kind",
                    "operationId",
                    "sourceHandle",
                    "requestedRange",
                    "committedRange",
                    "remainingRange",
                    "payloadHash",
                ],
            )
        {
            return Err(GENERATION_BLUEPRINT_AUTHOR_RECEIPT_INVALID.to_string());
        }
        let committed = conn
            .query_row(
                "SELECT payload_hash,start_chapter,end_chapter FROM blueprint_commit_operations WHERE operation_id=?",
                [receipt.operation_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((payload_hash, start_chapter, end_chapter)) = committed else {
            return Err(GENERATION_BLUEPRINT_AUTHOR_RECEIPT_INVALID.to_string());
        };
        if payload_hash != receipt.payload_hash
            || start_chapter != receipt.committed_range.from
            || end_chapter != receipt.committed_range.to
        {
            return Err(GENERATION_BLUEPRINT_AUTHOR_RECEIPT_INVALID.to_string());
        }
        Ok(Some(receipt))
    }

    pub fn record_blueprint_author_edit(
        &self,
        conn: &Connection,
        receipt: &BlueprintAuthorEditReceipt,
    ) -> Result<(), String> {
        if conn.is_autocommit() {
            return Err(GENERATION_DIRECTORY_TRANSACTION_REQUIRED.to_string());
        }
        self.record_blueprint_author_edit_in_transaction(conn, receipt)
    }

    fn record_blueprint_author_edit_in_transaction(
        &self,
        tx: &Connection,
        receipt: &BlueprintAuthorEditReceipt,
    ) -> Result<(), String> {
        self.assert_plot_outline_writable(tx, &receipt.source_handle.run_id)?;
        let original: String = tx
            .query_row(
                "SELECT binding_json FROM generation_runs WHERE run_id=?",
                [&receipt.source_handle.run_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let mut binding: Value = parse_json(&original)?;
        // { ...binding, $runEffects: { ...binding.$runEffects, blueprintAuthorEdit: receipt } }
        let run_effects = binding
            .as_object_mut()
            .ok_or_else(|| GENERATION_BLUEPRINT_AUTHOR_RECEIPT_CONFLICT.to_string())?
            .entry("$runEffects".to_string())
            .or_insert(json!({}));
        run_effects
            .as_object_mut()
            .ok_or_else(|| GENERATION_BLUEPRINT_AUTHOR_RECEIPT_CONFLICT.to_string())?
            .insert(
                "blueprintAuthorEdit".to_string(),
                serde_json::to_value(receipt).map_err(|error| error.to_string())?,
            );
        let updated = encode(&binding)?;
        let changes = tx
            .execute(
                "UPDATE generation_runs SET binding_json=? WHERE run_id=? AND binding_json=?",
                params![updated, receipt.source_handle.run_id, original],
            )
            .map_err(|error| error.to_string())?;
        if changes != 1 {
            return Err(GENERATION_BLUEPRINT_AUTHOR_RECEIPT_CONFLICT.to_string());
        }
        self.read_blueprint_author_edit(tx, &receipt.source_handle.run_id)?;
        Ok(())
    }

    pub fn record_planning_continuation(
        &self,
        conn: &Connection,
        receipt: &PlanningContinuationReceipt,
    ) -> Result<(), String> {
        if conn.is_autocommit() {
            return Err(GENERATION_PLOT_OUTLINE_TRANSACTION_REQUIRED.to_string());
        }
        self.record_planning_continuation_in_transaction(conn, receipt)
    }

    fn record_planning_continuation_in_transaction(
        &self,
        tx: &Connection,
        receipt: &PlanningContinuationReceipt,
    ) -> Result<(), String> {
        if self
            .read_planning_continuation(tx, &receipt.source_handle.run_id)?
            .is_some()
        {
            return Err(GENERATION_PLANNING_CONTINUATION_EXISTS.to_string());
        }
        let original: String = tx
            .query_row(
                "SELECT binding_json FROM generation_runs WHERE run_id=?",
                [&receipt.source_handle.run_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let mut binding: Value = parse_json(&original)?;
        let run_effects = binding
            .as_object_mut()
            .ok_or_else(|| GENERATION_PLANNING_CONTINUATION_CONFLICT.to_string())?
            .entry("$runEffects".to_string())
            .or_insert(json!({}));
        run_effects
            .as_object_mut()
            .ok_or_else(|| GENERATION_PLANNING_CONTINUATION_CONFLICT.to_string())?
            .insert(
                "planningContinuation".to_string(),
                serde_json::to_value(receipt).map_err(|error| error.to_string())?,
            );
        let updated = encode(&binding)?;
        let changes = tx
            .execute(
                "UPDATE generation_runs SET binding_json=? WHERE run_id=? AND binding_json=?",
                params![updated, receipt.source_handle.run_id, original],
            )
            .map_err(|error| error.to_string())?;
        if changes != 1 {
            return Err(GENERATION_PLANNING_CONTINUATION_CONFLICT.to_string());
        }
        self.read_planning_continuation(tx, &receipt.source_handle.run_id)?;
        Ok(())
    }

    // ---------------------------------------------------------------------
    // 预算与尝试
    // ---------------------------------------------------------------------

    pub fn budget(
        &self,
        conn: &Connection,
        root_id: &str,
    ) -> Result<GenerationBudgetReceipt, String> {
        let (action_json, budget_json, active_elapsed_ms, active_since_ms, blocked_code) = conn
            .query_row(
                "SELECT action_json,budget_json,active_elapsed_ms,active_since_ms,blocked_code FROM generation_roots WHERE root_action_id=?",
                [root_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| GENERATION_ROOT_MISSING.to_string())?;
        let attempts = self
            .query_attempt_json(conn, root_id)?
            .into_iter()
            .map(|attempt_json| parse_json::<PhysicalAttempt>(&attempt_json))
            .collect::<Result<Vec<_>, String>>()?;
        let active_elapsed_ms = active_elapsed_ms
            + match active_since_ms {
                None => 0,
                Some(active_since_ms) => ((self.now_ms)() - active_since_ms).max(0),
            };
        Ok(GenerationBudgetReceipt {
            root: parse_json(&action_json)?,
            policy: parse_json(&budget_json)?,
            active_elapsed_ms,
            attempts,
            blocked_code,
        })
    }

    fn query_attempt_json(&self, conn: &Connection, root_id: &str) -> Result<Vec<String>, String> {
        let mut stmt = conn
            .prepare("SELECT attempt_json FROM generation_attempts WHERE root_action_id=? ORDER BY rowid")
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([root_id], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(rows)
    }

    pub fn attempt_count(&self, conn: &Connection, run_id: &str) -> Result<i64, String> {
        conn.query_row(
            "SELECT COUNT(*) FROM generation_attempts WHERE run_id=?",
            [run_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
    }

    pub fn has_non_cancelled_attempt(
        &self,
        conn: &Connection,
        run_id: &str,
    ) -> Result<bool, String> {
        // 此处必须按 run_id，而不是预算账本的 root_action_id：父根内其他子运行不算首稿。
        let mut statement = conn
            .prepare("SELECT attempt_json FROM generation_attempts WHERE run_id=? ORDER BY rowid")
            .map_err(|error| error.to_string())?;
        let attempts = statement
            .query_map([run_id], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        for attempt_json in attempts {
            let attempt: PhysicalAttempt =
                parse_json(&attempt_json.map_err(|error| error.to_string())?)?;
            if attempt.status != "cancelled-before-dispatch" {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // ---------------------------------------------------------------------
    // 打开与绑定
    // ---------------------------------------------------------------------

    fn binding(request: &RunBinding) -> Result<RunBinding, String> {
        if request.source_manifest.is_empty()
            || request.project_id.is_empty()
            || request.epoch.is_empty()
            || request.context_snapshot_id.is_empty()
            // Object.keys(request.fingerprint).length !== 9 的等价物：
            // FrozenInputFingerprint 恰有 9 个序列化键，键数不符即拒绝。
            || serde_json::to_value(&request.fingerprint)
                .ok()
                .and_then(|value| value.as_object().map(|map| map.len() != 9))
                .unwrap_or(true)
            || ![
                request.fingerprint.chapter_brief_hash.as_str(),
                request.fingerprint.author_guidance_hash.as_str(),
                request.fingerprint.dependency_hash.as_str(),
                request.fingerprint.context_snapshot_hash.as_str(),
                request.fingerprint.template_hash.as_str(),
                request.fingerprint.skill_snapshot_hash.as_str(),
                request.fingerprint.model_lease_revision.as_str(),
                request.fingerprint.policy_hash.as_str(),
                request.fingerprint.output_contract_hash.as_str(),
            ]
            .iter()
            .all(|value| is_content_hash(value))
            || request.source_refs.iter().any(|source_ref| {
                !same_project_epoch(source_ref, request)
                    || source_ref.source_id.is_empty()
                    || !is_content_hash(&source_ref.content_hash)
                    || !is_safe_integer_value(source_ref.revision)
                    || source_ref.revision < 0
            })
        {
            return Err(GENERATION_BINDING_INVALID.to_string());
        }
        let frozen = RunBinding {
            project_id: request.project_id.clone(),
            epoch: request.epoch.clone(),
            fingerprint: request.fingerprint.clone(),
            context_snapshot_id: request.context_snapshot_id.clone(),
            source_manifest: request.source_manifest.clone(),
            source_refs: request.source_refs.clone(),
            extra: Map::new(),
        };
        // JSON.parse(encode(...)) 的等价物：序列化再反序列化，丢弃未知键。
        parse_json(&encode(&frozen)?)
    }

    pub fn open(
        &self,
        conn: &Connection,
        request: &OpenGenerationRunRequest,
    ) -> Result<DurableGenerationRun, String> {
        in_transaction(conn, |tx| self.open_in_existing_transaction(tx, request))
    }

    /// 调用方已持有事务时复用其写入边界，不嵌套 BEGIN 或提前提交。
    pub(crate) fn open_in_existing_transaction(
        &self,
        conn: &Connection,
        request: &OpenGenerationRunRequest,
    ) -> Result<DurableGenerationRun, String> {
        if conn.is_autocommit() {
            return Err(GENERATION_BINDING_INVALID.to_string());
        }
        let frozen = Self::binding(&RunBinding {
            project_id: request.project_id.clone(),
            epoch: request.epoch.clone(),
            fingerprint: request.fingerprint.clone(),
            context_snapshot_id: request.context_snapshot_id.clone(),
            source_manifest: request.source_manifest.clone(),
            source_refs: request.source_refs.clone(),
            extra: Map::new(),
        })?;
        self.open_in_transaction(conn, request, &frozen)
    }

    fn open_in_transaction(
        &self,
        tx: &Connection,
        request: &OpenGenerationRunRequest,
        frozen: &RunBinding,
    ) -> Result<DurableGenerationRun, String> {
        let mut action = RootAction {
            project_id: request.project_id.clone(),
            epoch: request.epoch.clone(),
            operation: request.operation.clone(),
            ui_action_nonce: request.ui_action_nonce.clone(),
            frozen_input_hash: request.frozen_input_hash.clone(),
            root_action_id: random_uuid_v4(),
            status: "active".to_string(),
        };
        let key = root_action_idempotency_key(&action)?;
        let open_key = encode(&[
            Value::String(key.clone()),
            match &request.parent_root_action_id {
                Some(parent) => Value::String(parent.clone()),
                None => Value::Null,
            },
        ])?;
        let existing = tx
            .query_row(
                "SELECT run_id FROM generation_runs WHERE open_key=?",
                [open_key.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some(existing_run_id) = existing {
            self.assert_plot_outline_writable(tx, &existing_run_id)?;
            let prior = self.get(tx, &existing_run_id)?;
            if !values_equal(&prior.binding.source_manifest, &frozen.source_manifest)
                || !values_equal(
                    &source_refs_without_epoch(&prior.binding.source_refs),
                    &source_refs_without_epoch(&frozen.source_refs),
                )
                || encode(&prior.binding.fingerprint)? != encode(&frozen.fingerprint)?
                || prior.binding.context_snapshot_id != frozen.context_snapshot_id
                || encode(&self.budget(tx, &prior.root_action_id)?.policy)?
                    != encode(&request.budget)?
            {
                return Err(GENERATION_NONCE_CONFLICT.to_string());
            }
            return Ok(prior);
        }
        if let Some(parent_root_action_id) = &request.parent_root_action_id {
            if self.author_edited_root(tx, parent_root_action_id)? {
                return Err(GENERATION_PLOT_OUTLINE_AUTHOR_SAVED.to_string());
            }
            let parent = self.budget(tx, parent_root_action_id)?;
            if parent.root.project_id != request.project_id
                || parent.root.epoch != request.epoch
                || parent.root.status != "active"
                || parent.blocked_code.is_some()
            {
                return Err(GENERATION_PARENT_INVALID.to_string());
            }
            action = parent.root;
        } else {
            if [
                request.budget.max_physical_requests,
                request.budget.max_token_liability,
                request.budget.max_output_per_request,
                request.budget.max_active_elapsed_ms,
            ]
            .iter()
            .any(|&value| !is_safe_integer_value(value) || value <= 0)
            {
                return Err(INVALID_BUDGET.to_string());
            }
            tx.execute(
                "INSERT INTO generation_roots(root_action_id,idempotency_key,action_json,budget_json) VALUES(?,?,?,?)",
                params![action.root_action_id, key, encode(&action)?, encode(&request.budget)?],
            )
            .map_err(|error| error.to_string())?;
        }
        let run_id = random_uuid_v4();
        tx.execute(
            "INSERT INTO generation_runs VALUES(?,?,?,?,?,?)",
            params![
                run_id,
                action.root_action_id,
                encode(frozen)?,
                "running",
                (self.now_ms)(),
                open_key
            ],
        )
        .map_err(|error| error.to_string())?;
        self.get(tx, &run_id)
    }

    pub fn replace_unstarted_binding(
        &self,
        conn: &Connection,
        run_id: &str,
        expected: &RunBinding,
        next: &RunBinding,
    ) -> Result<DurableGenerationRun, String> {
        let frozen = Self::binding(next)?;
        in_transaction(conn, |tx| {
            let run = self.get(tx, run_id)?;
            let budget = self.budget(tx, &run.root_action_id)?;
            self.assert_plot_outline_writable(tx, run_id)?;
            if !values_equal(&run.binding, expected) {
                return Err(GENERATION_BINDING_CONFLICT.to_string());
            }
            if run.status != "running"
                || budget.root.status != "active"
                || self.attempt_count(tx, run_id)? > 0
                || budget.blocked_code.is_some()
            {
                return Err(GENERATION_MATERIAL_DECISION_TOO_LATE.to_string());
            }
            tx.execute(
                "UPDATE generation_runs SET binding_json=? WHERE run_id=?",
                params![encode(&frozen)?, run_id],
            )
            .map_err(|error| error.to_string())?;
            self.get(tx, run_id)
        })
    }

    // ---------------------------------------------------------------------
    // 调用与预留
    // ---------------------------------------------------------------------

    pub fn find_invocation(
        &self,
        conn: &Connection,
        run_id: &str,
        nonce: &str,
        request_hash: &str,
    ) -> Result<Option<GenerationExecutionReceipt>, String> {
        let row = conn
            .query_row(
                "SELECT attempt_id,usage_receipt_json FROM generation_attempts WHERE run_id=? AND invocation_nonce=?",
                params![run_id, nonce],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((attempt_id, usage_receipt_json)) = row else {
            return Ok(None);
        };
        let stored: Value = parse_json(&usage_receipt_json)?;
        if stored.get("requestHash").and_then(Value::as_str) != Some(request_hash) {
            return Err(GENERATION_INVOCATION_CONFLICT.to_string());
        }
        self.receipt(conn, &attempt_id, None).map(Some)
    }

    pub fn reserve(
        &self,
        conn: &Connection,
        run_id: &str,
        nonce: &str,
        request_hash: &str,
        reserved_tokens: i64,
        requested_output_tokens: i64,
        usage_policy: Option<&ProviderUsagePolicy>,
        purpose: Option<&str>,
        replay_task: Option<&Value>,
        budget_decision: Option<&Value>,
        reconciliation_injected: bool,
    ) -> Result<GenerationExecutionReceipt, String> {
        in_transaction(conn, |tx| {
            self.assert_plot_outline_writable(tx, run_id)?;
            if let Some(prior) = self.find_invocation(tx, run_id, nonce, request_hash)? {
                return Ok(prior);
            }
            if nonce.is_empty() || !is_content_hash(request_hash) {
                return Err(GENERATION_INVOCATION_INVALID.to_string());
            }
            let run = self.get(tx, run_id)?;
            // 短锁准备后可能有另一调用先完成预留：在同一事务内再次锁定“对账最先一次”。
            if purpose == Some("chapter-draft-reconcile")
                && run
                    .binding
                    .source_manifest
                    .get("operation")
                    .and_then(Value::as_str)
                    == Some("chapter-draft")
                && run
                    .binding
                    .source_manifest
                    .get("materialDecision")
                    .and_then(|decision| decision.get("reconciliationPromptHash"))
                    .is_some()
                && self.has_non_cancelled_attempt(tx, run_id)?
            {
                return Err("GENERATION_MATERIAL_PROMPT_MISMATCH".to_string());
            }
            let budget = self.budget(tx, &run.root_action_id)?;
            if run.status != "running" || budget.blocked_code.is_some() {
                return Err(GENERATION_DISPATCH_BLOCKED.to_string());
            }
            if !same_project_epoch(&run.binding, &budget.root) {
                return Err(GENERATION_EPOCH_STALE.to_string());
            }
            self.assert_plot_outline_request(tx, run_id, purpose, replay_task, None)?;
            let attempt = PhysicalAttempt {
                attempt_id: random_uuid_v4(),
                reservation_id: random_uuid_v4(),
                root_action_id: run.root_action_id.clone(),
                status: "reserved".to_string(),
                reserved_tokens,
                requested_output_tokens,
                actual_tokens: None,
            };
            assert_reservation(
                &budget.root,
                &budget.policy,
                &budget.attempts,
                &attempt,
                budget.active_elapsed_ms,
            )?;
            let initial_receipt = json!({
                "requestHash": request_hash,
                "usagePolicy": usage_policy,
            });
            tx.execute(
                "INSERT INTO generation_attempts VALUES(?,?,?,?,?,?,?)",
                params![
                    attempt.attempt_id,
                    attempt.reservation_id,
                    run_id,
                    run.root_action_id,
                    encode(&attempt)?,
                    encode(&initial_receipt)?,
                    nonce
                ],
            )
            .map_err(|error| error.to_string())?;
            let artifact = VisibleArtifact {
                artifact_id: random_uuid_v4(),
                attempt_id: attempt.attempt_id.clone(),
                root_action_id: run.root_action_id.clone(),
                project_id: run.binding.project_id.clone(),
                epoch: run.binding.epoch.clone(),
                fingerprint: run.binding.fingerprint.clone(),
                revision: 0,
                text: String::new(),
                text_hash: text_hash(""),
            };
            let identity = ArtifactIdentity {
                artifact_id: artifact.artifact_id.clone(),
                epoch: artifact.epoch.clone(),
                fingerprint: artifact.fingerprint.clone(),
            };
            let mut updated_receipt = initial_receipt;
            if let Some(object) = updated_receipt.as_object_mut() {
                if let Some(purpose) = purpose.filter(|purpose| !purpose.is_empty()) {
                    object.insert("purpose".to_string(), Value::String(purpose.to_string()));
                }
                if let Some(replay_task) = replay_task {
                    object.insert("replayTask".to_string(), replay_task.clone());
                }
                if let Some(budget_decision) = budget_decision {
                    object.insert("budgetDecision".to_string(), budget_decision.clone());
                }
                if reconciliation_injected {
                    object.insert("reconciliationInjected".to_string(), Value::Bool(true));
                }
                object.insert(
                    "artifactIdentity".to_string(),
                    serde_json::to_value(&identity).map_err(|error| error.to_string())?,
                );
            }
            tx.execute(
                "UPDATE generation_attempts SET usage_receipt_json=? WHERE attempt_id=?",
                params![updated_receipt.to_string(), attempt.attempt_id],
            )
            .map_err(|error| error.to_string())?;
            tx.execute(
                "INSERT INTO generation_artifacts VALUES(?,?,?,?,?,?)",
                params![
                    artifact.artifact_id,
                    attempt.attempt_id,
                    run_id,
                    encode(&artifact)?,
                    0,
                    "partial"
                ],
            )
            .map_err(|error| error.to_string())?;
            self.receipt(tx, &attempt.attempt_id, None)
        })
    }

    pub fn receipt(
        &self,
        conn: &Connection,
        attempt_id: &str,
        budget: Option<GenerationBudgetReceipt>,
    ) -> Result<GenerationExecutionReceipt, String> {
        let row = conn
            .query_row(
                &format!("{RECEIPT_SELECT} WHERE t.attempt_id=?"),
                [attempt_id],
                map_receipt_row,
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| GENERATION_ATTEMPT_MISSING.to_string())?;
        let run = self.get(conn, &row.run_id)?;
        let usage: ReceiptUsage = parse_json(&row.usage_receipt_json)?;
        let budget = match &budget {
            Some(budget) if budget.root.root_action_id == run.root_action_id => budget.clone(),
            _ => self.budget(conn, &run.root_action_id)?,
        };
        self.hydrate_receipt(&row, &run, &usage, &budget)
    }

    fn hydrate_receipt(
        &self,
        row: &ReceiptRow,
        run: &DurableGenerationRun,
        usage: &ReceiptUsage,
        budget: &GenerationBudgetReceipt,
    ) -> Result<GenerationExecutionReceipt, String> {
        let visible: Option<VisibleArtifact> = match &row.artifact_json {
            None => None,
            Some(artifact_json) => parse_json(artifact_json)?,
        };
        let identity = &usage.artifact_identity;
        if let Some(visible) = &visible {
            if visible.attempt_id != row.attempt_id
                || visible.root_action_id != run.root_action_id
                || identity.is_none()
                || row.artifact_id.as_deref() != Some(visible.artifact_id.as_str())
                || identity
                    .as_ref()
                    .is_none_or(|identity| identity.artifact_id != visible.artifact_id)
                || identity
                    .as_ref()
                    .is_none_or(|identity| identity.epoch != visible.epoch)
                || identity.as_ref().is_none_or(|identity| {
                    !values_equal(&identity.fingerprint, &visible.fingerprint)
                })
                || !values_equal(&visible.fingerprint, &run.binding.fingerprint)
                || visible.project_id != run.binding.project_id
                || row.revision != Some(visible.revision)
                || text_hash(&visible.text) != visible.text_hash
            {
                return Err(ARTIFACT_INTEGRITY_FAILED.to_string());
            }
        }
        Ok(GenerationExecutionReceipt {
            run: run.clone(),
            attempt: parse_json(&row.attempt_json)?,
            artifact: visible,
            budget: budget.clone(),
            result: usage.result.clone(),
            diagnostics: usage
                .result
                .as_ref()
                .and_then(|result| result.diagnostics.clone()),
            failure_code: usage
                .result
                .as_ref()
                .and_then(|result| result.failure_code.clone()),
            budget_decision: usage.budget_decision.clone(),
            unsaved_tail: None,
        })
    }

    // ---------------------------------------------------------------------
    // 运行视图与大纲
    // ---------------------------------------------------------------------

    fn read_state<'a>(
        &'a self,
        conn: &'a Connection,
        run: DurableGenerationRun,
    ) -> Result<GenerationRead<'a>, String> {
        let budget = self.budget(conn, &run.root_action_id)?;
        let mut stmt = conn
            .prepare(&format!(
                "{RECEIPT_SELECT} WHERE t.root_action_id=? ORDER BY t.rowid"
            ))
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([run.root_action_id.as_str()], map_receipt_row)
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let mut attempts = HashMap::new();
        let mut artifacts = HashMap::new();
        for (index, row) in rows.iter().enumerate() {
            attempts.insert(row.attempt_id.clone(), index);
            if let Some(artifact_id) = &row.artifact_id {
                artifacts.insert(artifact_id.clone(), index);
            }
        }
        let mut runs = HashMap::new();
        runs.insert(run.run_id.clone(), run.clone());
        Ok(GenerationRead {
            repo: self,
            conn,
            run,
            budget,
            rows,
            attempts,
            artifacts,
            runs,
            receipts: HashMap::new(),
        })
    }

    pub fn read_run_view(&self, conn: &Connection, run_id: &str) -> Result<RunView, String> {
        let run = self.get(conn, run_id)?;
        let mut read = self.read_state(conn, run)?;
        let plot_outline = self.plot_outline_from(&mut read, None, None)?;
        let candidate_ids: HashSet<String> = read
            .rows
            .iter()
            .filter(|row| {
                row.artifact_run_id.as_deref() == Some(run_id)
                    && row.artifact_status.as_deref() != Some("discarded")
                    && !(self.is_frozen)(FREEZE_TABLE_GENERATION_ATTEMPTS, &row.attempt_id)
            })
            .filter_map(|row| row.artifact_id.clone())
            .collect();
        let budget_attempts = read.budget.attempts.clone();
        let mut attempts = Vec::new();
        for attempt in &budget_attempts {
            let value = read.receipt(&attempt.attempt_id)?;
            let candidate = value
                .receipt
                .artifact
                .as_ref()
                .is_some_and(|artifact| candidate_ids.contains(&artifact.artifact_id));
            attempts.push(AttemptView {
                usage: value.usage,
                receipt: value.receipt,
                candidate,
            });
        }
        Ok(RunView {
            budget: read.budget,
            plot_outline,
            attempts,
        })
    }

    pub fn mark_dispatched(&self, conn: &Connection, attempt_id: &str) -> Result<(), String> {
        in_transaction(conn, |tx| {
            let receipt = self.receipt(tx, attempt_id, None)?;
            if receipt.budget.active_elapsed_ms >= receipt.budget.policy.max_active_elapsed_ms
                || receipt.budget.root.status != "active"
                || receipt.budget.blocked_code.is_some()
            {
                return Err(GENERATION_DISPATCH_BLOCKED.to_string());
            }
            assert_attempt_transition(&receipt.attempt.status, "dispatch-marked")?;
            let stored: Value = tx
                .query_row(
                    "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id=?",
                    [attempt_id],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| error.to_string())
                .and_then(|json| parse_json(&json))?;
            self.assert_plot_outline_request(
                tx,
                &receipt.run.run_id,
                stored.get("purpose").and_then(Value::as_str),
                stored.get("replayTask"),
                Some(attempt_id),
            )?;
            let mut attempt = receipt.attempt.clone();
            attempt.status = "dispatch-marked".to_string();
            tx.execute(
                "UPDATE generation_attempts SET attempt_json=? WHERE attempt_id=?",
                params![encode(&attempt)?, attempt_id],
            )
            .map_err(|error| error.to_string())?;
            tx.execute(
                "UPDATE generation_roots SET active_since_ms=COALESCE(active_since_ms,?) WHERE root_action_id=?",
                params![(self.now_ms)(), receipt.run.root_action_id],
            )
            .map_err(|error| error.to_string())?;
            Ok(())
        })
    }

    pub fn read_plot_outline(
        &self,
        conn: &Connection,
        run_id: &str,
        omit_attempt_id: Option<&str>,
    ) -> Result<Option<PlotOutlineProgress>, String> {
        let run = self.get(conn, run_id)?;
        if plot_outline_policy(&run.binding.source_manifest)?.is_some() {
            let mut read = self.read_state(conn, run)?;
            self.plot_outline_from(&mut read, omit_attempt_id, None)
        } else {
            Ok(None)
        }
    }

    fn plot_outline_from(
        &self,
        read: &mut GenerationRead<'_>,
        omit_attempt_id: Option<&str>,
        composition: Option<Option<VisibleCompositionReceipt>>,
    ) -> Result<Option<PlotOutlineProgress>, String> {
        let run = read.run.clone();
        let Some(policy) = plot_outline_policy(&run.binding.source_manifest)? else {
            return Ok(None);
        };
        let composition = match composition {
            Some(composition) => composition,
            None => self.composition_from(read)?,
        };
        let source_expected = plot_outline_expected(
            run.binding
                .source_manifest
                .get("plotOutlineSource")
                .unwrap_or(&Value::Null),
        )?;
        let confirmed_prefix = plot_outline_confirmed_prefix(&source_expected, &policy.range)?;
        let accepted_count = composition
            .as_ref()
            .and_then(|composition| composition.chapters.as_ref())
            .map(|chapters| chapters.len() as i64)
            .unwrap_or_default();
        let chapter = policy.range.from + accepted_count;
        let purpose_normal = plot_outline_purpose(chapter, "normal");
        let purpose_compact = plot_outline_purpose(chapter, "compact");
        let mut stmt = read
            .conn
            .prepare(
                "SELECT attempt_id,json_extract(usage_receipt_json,'$.purpose') AS purpose FROM generation_attempts WHERE run_id=? AND json_extract(usage_receipt_json,'$.purpose') IN (?,?) ORDER BY rowid",
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(
                params![run.run_id.as_str(), purpose_normal, purpose_compact],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<(String, Option<String>)>, _>>()
            .map_err(|error| error.to_string())?;
        let target_units = match run
            .binding
            .source_manifest
            .get("authorInputs")
            .and_then(Value::as_array)
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("id").and_then(Value::as_str) == Some("planning:target-units")
                })
            })
            .and_then(|item| item.get("text"))
        {
            Some(text) => parse_planning_target_units(js_number(Some(text)).map(Value::from))?,
            None => DEFAULT_PLANNING_TARGET_UNITS,
        };
        let mut attempts = Vec::new();
        for (attempt_id, purpose) in rows {
            if Some(attempt_id.as_str()) == omit_attempt_id {
                continue;
            }
            let value = read.receipt(&attempt_id)?;
            let artifact = if value.receipt.artifact.is_some()
                && value
                    .receipt
                    .artifact
                    .as_ref()
                    .map(|artifact| {
                        read.artifacts
                            .get(&artifact.artifact_id)
                            .map(|&index| {
                                read.rows[index].artifact_status.as_deref() != Some("discarded")
                            })
                            .unwrap_or(true)
                    })
                    .unwrap_or(false)
                && !(self.is_frozen)(FREEZE_TABLE_GENERATION_ATTEMPTS, &attempt_id)
            {
                value.receipt.artifact.clone()
            } else {
                None
            };
            attempts.push(PlotOutlineAttempt {
                purpose: purpose.unwrap_or_default(),
                status: value.receipt.attempt.status.clone(),
                finish_reason: value
                    .receipt
                    .result
                    .as_ref()
                    .and_then(|result| result.finish_reason.clone()),
                artifact: artifact.map(|artifact| PlotOutlineAttemptArtifact {
                    artifact_id: artifact.artifact_id,
                    text: artifact.text,
                    revision: artifact.revision,
                    text_hash: artifact.text_hash,
                }),
            });
        }
        Ok(Some(PlotOutlineProgress {
            protocol: PLOT_OUTLINE_PROTOCOL.to_string(),
            range: policy.range,
            content: policy.content,
            target_units,
            cursor: derive_plot_outline_cursor(&policy.range, accepted_count, &attempts)?,
            source_expected,
            confirmed_prefix,
            composition,
        }))
    }

    pub fn assert_plot_outline_request(
        &self,
        conn: &Connection,
        run_id: &str,
        purpose: Option<&str>,
        task: Option<&Value>,
        omit_attempt_id: Option<&str>,
    ) -> Result<(), String> {
        self.assert_plot_outline_writable(conn, run_id)?;
        let Some(outline) = self.read_plot_outline(conn, run_id, omit_attempt_id)? else {
            return Ok(());
        };
        let cursor = &outline.cursor;
        let PlotOutlineCursor::Request {
            chapter_number,
            attempt,
        } = cursor
        else {
            return Err(GENERATION_DISPATCH_PLOT_OUTLINE_CURSOR.to_string());
        };
        if purpose != Some(plot_outline_purpose(*chapter_number, attempt).as_str()) {
            return Err(GENERATION_DISPATCH_PLOT_OUTLINE_CURSOR.to_string());
        }
        let contract = plot_outline_request_contract(&outline, *chapter_number, attempt);
        let matched = task
            .and_then(|task| task.get("messages"))
            .and_then(|messages| messages.as_array())
            .map(|messages| {
                messages.iter().any(|message| {
                    message.get("role").and_then(Value::as_str) == Some("user")
                        && message
                            .get("content")
                            .and_then(Value::as_str)
                            .is_some_and(|content| content.ends_with(&contract))
                })
            })
            .unwrap_or(false);
        if !matched {
            return Err(GENERATION_DISPATCH_PLOT_OUTLINE_CURSOR.to_string());
        }
        Ok(())
    }

    // ---------------------------------------------------------------------
    // 组合
    // ---------------------------------------------------------------------

    fn visible_composition(
        &self,
        read: &mut GenerationRead<'_>,
        artifact_ids: &[String],
        algorithm: &str,
    ) -> Result<VisibleCompositionReceipt, String> {
        let run = read.run.clone();
        let outline = plot_outline_policy(&run.binding.source_manifest)?;
        if outline.is_some() {
            if algorithm != PLOT_OUTLINE_PROTOCOL {
                return Err(GENERATION_COMPOSITION_ALGORITHM_INVALID.to_string());
            }
        } else if algorithm != VISIBLE_CONTINUATION_VERSION
            && !is_draft_visible_text_version(algorithm)
        {
            return Err(GENERATION_COMPOSITION_ALGORITHM_INVALID.to_string());
        }
        let maximum_artifacts = match &outline {
            Some(policy) => policy.range.to - policy.range.from + 1,
            None => 32,
        };
        let mut seen_ids = HashSet::new();
        if artifact_ids.is_empty()
            || artifact_ids.len() as i64 > maximum_artifacts
            || artifact_ids
                .iter()
                .any(|artifact_id| artifact_id.is_empty())
            || !artifact_ids
                .iter()
                .all(|artifact_id| seen_ids.insert(artifact_id.clone()))
        {
            return Err(GENERATION_COMPOSITION_INVALID.to_string());
        }
        let mut text = String::new();
        let mut previous_ordinal = -1;
        let mut sources = Vec::new();
        let mut chapters = Vec::new();
        for (index, artifact_id) in artifact_ids.iter().enumerate() {
            let Some(&row_index) = read.artifacts.get(artifact_id) else {
                return Err(GENERATION_COMPOSITION_SOURCE_INVALID.to_string());
            };
            let row = read.rows[row_index].clone();
            if row.artifact_run_id.as_deref() != Some(run.run_id.as_str())
                || row.artifact_status.as_deref() == Some("discarded")
                || row.ordinal <= previous_ordinal
            {
                return Err(GENERATION_COMPOSITION_SOURCE_INVALID.to_string());
            }
            let value = read.receipt(&row.attempt_id)?;
            let artifact = value
                .receipt
                .artifact
                .clone()
                .ok_or_else(|| GENERATION_COMPOSITION_SOURCE_INVALID.to_string())?;
            if value.usage.purpose.as_deref() == Some(DRAFT_RECONCILE_PURPOSE)
                || value.usage.purpose.as_deref() == Some(DRAFT_SHORT_OUTLINE_PURPOSE)
            {
                return Err(GENERATION_COMPOSITION_SOURCE_INVALID.to_string());
            }
            if !["settled", "unknown"].contains(&value.receipt.attempt.status.as_str())
                || artifact.text.trim().is_empty()
            {
                return Err(GENERATION_COMPOSITION_SOURCE_NOT_TERMINAL.to_string());
            }
            let finish_reason = value
                .receipt
                .result
                .as_ref()
                .and_then(|result| result.finish_reason.as_deref())
                .unwrap_or_default();
            if finish_reason != "stop" && finish_reason != "length" {
                return Err(GENERATION_COMPOSITION_SOURCE_UNTRUSTED.to_string());
            }
            if let Some(policy) = &outline {
                let purpose =
                    parse_plot_outline_purpose(value.usage.purpose.as_deref().unwrap_or_default());
                let chapter = policy.range.from + chapters.len() as i64;
                let Some(purpose) = purpose else {
                    return Err(GENERATION_PLOT_OUTLINE_ENTRY_INVALID.to_string());
                };
                if purpose.chapter_number != chapter
                    || chapter > policy.range.to
                    || value
                        .receipt
                        .result
                        .as_ref()
                        .and_then(|result| result.finish_reason.as_deref())
                        != Some("stop")
                    || !valid_plot_outline_entry(&artifact.text, chapter)
                {
                    return Err(GENERATION_PLOT_OUTLINE_ENTRY_INVALID.to_string());
                }
                chapters.push(CompositionChapter {
                    chapter_number: chapter,
                    artifact_ids: vec![artifact_id.clone()],
                    text_hash: text_hash(&artifact.text),
                });
                text = join_plot_outline_entries(&text, &artifact.text);
                previous_ordinal = row.ordinal;
                sources.push(CompositionSource {
                    artifact_id: artifact_id.clone(),
                    revision: artifact.revision,
                    text_hash: artifact.text_hash.clone(),
                });
                continue;
            }
            let draft_algorithm = if is_draft_visible_text_version(algorithm) {
                Some(algorithm)
            } else {
                None
            };
            let condense = draft_algorithm.is_some()
                && value.usage.purpose.as_deref() == Some(DRAFT_CONDENSE_PURPOSE);
            if condense && (sources.is_empty() || index != artifact_ids.len() - 1) {
                return Err(GENERATION_COMPOSITION_SOURCE_INVALID.to_string());
            }
            let next = if condense {
                sanitize_draft_text(&artifact.text, draft_algorithm.unwrap_or_default())
            } else if let Some(draft_algorithm) = draft_algorithm {
                if !sources.is_empty() {
                    compose_draft_visible_continuation(&text, &artifact.text, draft_algorithm)
                } else {
                    sanitize_draft_text(&artifact.text, draft_algorithm)
                }
            } else if !sources.is_empty() {
                compose_visible_continuation(&text, &artifact.text)
            } else {
                artifact.text.trim().to_string()
            };
            if next.trim().is_empty() {
                return Err(GENERATION_COMPOSITION_NO_PROGRESS.to_string());
            }
            if condense {
                let target = js_number(
                    run.binding
                        .source_manifest
                        .get("authorInputs")
                        .and_then(Value::as_array)
                        .and_then(|items| {
                            items.iter().find(|item| {
                                item.get("id").and_then(Value::as_str) == Some("draft:target-units")
                            })
                        })
                        .and_then(|item| item.get("text")),
                );
                let Some(target) = target else {
                    return Err(GENERATION_COMPOSITION_SOURCE_INVALID.to_string());
                };
                if !is_safe_integer_value(target) || target < 1 {
                    return Err(GENERATION_COMPOSITION_SOURCE_INVALID.to_string());
                }
                let finish_is_stop = value
                    .receipt
                    .result
                    .as_ref()
                    .and_then(|result| result.finish_reason.as_deref())
                    == Some("stop");
                if !finish_is_stop
                    || count_draft_units(&text) <= draft_target_unit_range(target)?.maximum
                {
                    return Err(GENERATION_COMPOSITION_NO_PROGRESS.to_string());
                }
            } else {
                if draft_algorithm.is_some()
                    && !sources.is_empty()
                    && value
                        .receipt
                        .result
                        .as_ref()
                        .and_then(|result| result.finish_reason.as_deref())
                        == Some("length")
                    && count_draft_units(&next) - count_draft_units(&text) < 300
                {
                    return Err(GENERATION_COMPOSITION_NO_PROGRESS.to_string());
                }
                if !sources.is_empty()
                    && count_letters_and_numbers(&next) <= count_letters_and_numbers(&text)
                {
                    return Err(GENERATION_COMPOSITION_NO_PROGRESS.to_string());
                }
            }
            text = next;
            previous_ordinal = row.ordinal;
            sources.push(CompositionSource {
                artifact_id: artifact_id.clone(),
                revision: artifact.revision,
                text_hash: artifact.text_hash.clone(),
            });
        }
        let text_hash = text_hash(&text);
        Ok(VisibleCompositionReceipt {
            algorithm: algorithm.to_string(),
            text,
            text_hash,
            artifact_ids: artifact_ids.to_vec(),
            sources,
            chapters: if outline.is_some() {
                Some(chapters)
            } else {
                None
            },
            author_inputs: None,
        })
    }

    pub fn list_directory_progress(
        &self,
        conn: &Connection,
    ) -> Result<Vec<DirectoryGenerationProgress>, String> {
        let rows = self.query_usage_receipts(conn, None)?;
        let mut result = Vec::new();
        for (run_id, usage_receipt_json) in rows {
            let stored_value: Value = parse_json(&usage_receipt_json)?;
            let Some(stored) = stored_value
                .get("directoryProgress")
                .filter(|value| !value.is_null())
                .cloned()
            else {
                continue;
            };
            let mut progress: DirectoryGenerationProgress = serde_json::from_value(stored)
                .map_err(|error| format!("{GENERATION_DIRECTORY_PROGRESS_INVALID}: {error}"))?;
            if !complete_handle(&json!(progress.source_handle))
                || progress.source_handle.run_id != run_id
                || progress.source_handle.epoch.as_str()
                    != stored_value
                        .get("artifactIdentity")
                        .and_then(|identity| identity.get("epoch"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                || !is_safe_integer_value(progress.requested_range.start_chapter)
                || progress.requested_range.start_chapter <= 0
                || !is_safe_integer_value(progress.requested_range.end_chapter)
                || progress.requested_range.end_chapter <= 0
                || !is_safe_integer_value(progress.committed_range.start_chapter)
                || progress.committed_range.start_chapter <= 0
                || !is_safe_integer_value(progress.committed_range.end_chapter)
                || progress.committed_range.end_chapter <= 0
                || progress.requested_range.end_chapter < progress.requested_range.start_chapter
                || progress.requested_range.end_chapter - progress.requested_range.start_chapter
                    >= 10000
                || progress.committed_range.end_chapter < progress.committed_range.start_chapter
            {
                return Err(GENERATION_DIRECTORY_PROGRESS_INVALID.to_string());
            }
            let run = self.get(conn, &progress.source_handle.run_id)?;
            let selected = match run
                .binding
                .source_manifest
                .get("selectedBlueprintChapterNumbers")
            {
                Some(Value::Array(items)) => {
                    items.iter().map(Value::as_i64).collect::<Option<Vec<_>>>()
                }
                _ => None,
            };
            let selected =
                selected.ok_or_else(|| GENERATION_DIRECTORY_PROGRESS_INVALID.to_string())?;
            if (progress.requested_range.start_chapter..=progress.requested_range.end_chapter)
                .any(|chapter| !selected.contains(&chapter))
            {
                return Err(GENERATION_DIRECTORY_PROGRESS_INVALID.to_string());
            }
            let committed = conn
                .query_row(
                    "SELECT payload_hash,start_chapter,end_chapter FROM blueprint_commit_operations WHERE operation_id=?",
                    [progress.operation_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(|error| error.to_string())?;
            let Some((payload_hash, start_chapter, end_chapter)) = committed else {
                return Err(GENERATION_DIRECTORY_PROGRESS_INVALID.to_string());
            };
            if payload_hash != progress.payload_hash
                || run.root_action_id != progress.source_handle.root_action_id
                || run.binding.project_id != progress.source_handle.project_id
                || start_chapter != progress.committed_range.start_chapter
                || end_chapter != progress.committed_range.end_chapter
                || progress.requested_range.start_chapter != start_chapter
                || progress.requested_range.end_chapter < end_chapter
                || !values_equal(
                    &progress.remaining_range,
                    &if end_chapter < progress.requested_range.end_chapter {
                        Some(ChapterRange {
                            start_chapter: end_chapter + 1,
                            end_chapter: progress.requested_range.end_chapter,
                        })
                    } else {
                        None
                    },
                )
            {
                return Err(GENERATION_DIRECTORY_PROGRESS_INVALID.to_string());
            }
            if let Some(continuation_handle) = &mut progress.continuation_handle {
                if !complete_handle(&json!(continuation_handle.clone())) {
                    return Err(GENERATION_DIRECTORY_PROGRESS_INVALID.to_string());
                }
                let child = self.get(conn, &continuation_handle.run_id)?;
                if child.root_action_id != run.root_action_id
                    || child.binding.project_id != run.binding.project_id
                    || continuation_handle.root_action_id != child.root_action_id
                    || continuation_handle.project_id != child.binding.project_id
                    || child.run_id == run.run_id
                {
                    return Err(GENERATION_DIRECTORY_PROGRESS_INVALID.to_string());
                }
                continuation_handle.epoch = child.binding.epoch.clone();
            }
            result.push(progress);
        }
        Ok(result)
    }

    fn query_usage_receipts(
        &self,
        conn: &Connection,
        run_id: Option<&str>,
    ) -> Result<Vec<(String, String)>, String> {
        let sql = match run_id {
            Some(_) => "SELECT run_id,usage_receipt_json FROM generation_attempts WHERE run_id=? ORDER BY rowid",
            None => "SELECT run_id,usage_receipt_json FROM generation_attempts ORDER BY rowid",
        };
        let mut stmt = conn.prepare(sql).map_err(|error| error.to_string())?;
        let read_receipt_row =
            |row: &rusqlite::Row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?));
        let rows = match run_id {
            Some(run_id) => stmt.query_map([run_id], read_receipt_row),
            None => stmt.query_map([], read_receipt_row),
        }
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
        Ok(rows)
    }

    pub fn record_directory_progress(
        &self,
        conn: &Connection,
        progress: &DirectoryGenerationProgress,
    ) -> Result<(), String> {
        if conn.is_autocommit() {
            return Err(GENERATION_DIRECTORY_TRANSACTION_REQUIRED.to_string());
        }
        self.record_directory_progress_in_transaction(conn, progress)
    }

    fn record_directory_progress_in_transaction(
        &self,
        tx: &Connection,
        progress: &DirectoryGenerationProgress,
    ) -> Result<(), String> {
        self.assert_plot_outline_writable(tx, &progress.source_handle.run_id)?;
        let existing = self
            .list_directory_progress(tx)?
            .into_iter()
            .find(|item| item.operation_id == progress.operation_id);
        if let Some(existing) = existing {
            if !values_equal(&existing, progress) {
                return Err(GENERATION_DIRECTORY_PROGRESS_CONFLICT.to_string());
            }
            return Ok(());
        }
        let run = self.get(tx, &progress.source_handle.run_id)?;
        if run
            .binding
            .source_manifest
            .get("outputContract")
            .and_then(Value::as_str)
            != Some("structured-data")
        {
            return Err(GENERATION_DIRECTORY_OUTPUT_INVALID.to_string());
        }
        let row = tx
            .query_row(
                "SELECT attempt_id,usage_receipt_json FROM generation_attempts WHERE run_id=? ORDER BY rowid DESC LIMIT 1",
                [progress.source_handle.run_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((attempt_id, usage_receipt_json)) = row else {
            return Err(GENERATION_DIRECTORY_PROGRESS_CONFLICT.to_string());
        };
        let mut stored: Value = parse_json(&usage_receipt_json)?;
        if matches!(stored.get("directoryProgress"), Some(value) if !value.is_null()) {
            return Err(GENERATION_DIRECTORY_PROGRESS_CONFLICT.to_string());
        }
        if !["settled", "unknown"]
            .contains(&self.receipt(tx, &attempt_id, None)?.attempt.status.as_str())
        {
            return Err(GENERATION_DIRECTORY_ATTEMPT_NOT_TERMINAL.to_string());
        }
        stored
            .as_object_mut()
            .ok_or_else(|| GENERATION_DIRECTORY_PROGRESS_CONFLICT.to_string())?
            .insert(
                "directoryProgress".to_string(),
                serde_json::to_value(progress).map_err(|error| error.to_string())?,
            );
        tx.execute(
            "UPDATE generation_attempts SET usage_receipt_json=? WHERE attempt_id=?",
            params![encode(&stored)?, attempt_id],
        )
        .map_err(|error| error.to_string())?;
        self.list_directory_progress(tx)?;
        Ok(())
    }

    pub fn activate_root_for_committed_stage(
        &self,
        conn: &Connection,
        root_id: &str,
        project: &ProjectEpoch,
    ) -> Result<(), String> {
        if conn.is_autocommit() {
            return Err(GENERATION_DIRECTORY_TRANSACTION_REQUIRED.to_string());
        }
        self.activate_root_for_committed_stage_in_transaction(conn, root_id, project)
    }

    fn activate_root_for_committed_stage_in_transaction(
        &self,
        tx: &Connection,
        root_id: &str,
        project: &ProjectEpoch,
    ) -> Result<(), String> {
        if self.author_edited_root(tx, root_id)? {
            return Err(GENERATION_PLOT_OUTLINE_AUTHOR_SAVED.to_string());
        }
        let mut budget = self.budget(tx, root_id)?;
        if budget.root.project_id != project.project_id
            || !["active", "paused"].contains(&budget.root.status.as_str())
            || budget.blocked_code.is_some()
            || budget
                .attempts
                .iter()
                .any(|attempt| ["reserved", "dispatch-marked"].contains(&attempt.status.as_str()))
        {
            return Err(GENERATION_DIRECTORY_CONTINUATION_BLOCKED.to_string());
        }
        budget.root.epoch = project.epoch.clone();
        budget.root.status = "active".to_string();
        tx.execute(
            "UPDATE generation_roots SET action_json=? WHERE root_action_id=?",
            params![encode(&budget.root)?, root_id],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn bind_directory_continuation(
        &self,
        conn: &Connection,
        operation_id: &str,
        handle: &MainGenerationRunHandle,
    ) -> Result<(), String> {
        self.bind_directory_continuation_in_transaction(conn, operation_id, handle)
    }

    fn bind_directory_continuation_in_transaction(
        &self,
        tx: &Connection,
        operation_id: &str,
        handle: &MainGenerationRunHandle,
    ) -> Result<(), String> {
        self.assert_plot_outline_writable(tx, &handle.run_id)?;
        let rows = self.query_usage_receipts(tx, None)?;
        let row = rows.iter().find(|(_, usage_receipt_json)| {
            parse_json::<Value>(usage_receipt_json)
                .ok()
                .and_then(|value| value.get("directoryProgress").cloned())
                .and_then(|progress| progress.get("operationId").cloned())
                .and_then(|id| id.as_str().map(|id| id == operation_id))
                .unwrap_or(false)
        });
        let Some((attempt_id, usage_receipt_json)) = row else {
            return Err(GENERATION_DIRECTORY_PROGRESS_INVALID.to_string());
        };
        if tx.is_autocommit() {
            return Err(GENERATION_DIRECTORY_PROGRESS_INVALID.to_string());
        }
        let mut receipt: Value = parse_json(usage_receipt_json)?;
        if matches!(
            receipt.get("directoryProgress").and_then(|progress| progress.get("continuationHandle")),
            Some(value) if !value.is_null()
        ) {
            return Err(GENERATION_DIRECTORY_CONTINUATION_EXISTS.to_string());
        }
        let directory_progress = receipt
            .get_mut("directoryProgress")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| GENERATION_DIRECTORY_PROGRESS_INVALID.to_string())?;
        directory_progress.insert(
            "continuationHandle".to_string(),
            serde_json::to_value(handle).map_err(|error| error.to_string())?,
        );
        tx.execute(
            "UPDATE generation_attempts SET usage_receipt_json=? WHERE attempt_id=?",
            params![encode(&receipt)?, attempt_id],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn read_visible_composition(
        &self,
        conn: &Connection,
        run_id: &str,
    ) -> Result<Option<VisibleCompositionReceipt>, String> {
        let run = self.get(conn, run_id)?;
        let mut read = self.read_state(conn, run)?;
        self.composition_from(&mut read)
    }

    fn composition_from(
        &self,
        read: &mut GenerationRead<'_>,
    ) -> Result<Option<VisibleCompositionReceipt>, String> {
        let mut stmt = read
            .conn
            .prepare(
                "SELECT json_quote(json_extract(usage_receipt_json,'$.visibleComposition')) AS composition_json FROM generation_attempts WHERE run_id=? ORDER BY rowid DESC",
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([read.run.run_id.as_str()], |row| {
                row.get::<_, Option<String>>(0)
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        for composition_json in rows {
            let Some(composition_json) = composition_json else {
                continue;
            };
            let stored: Value = parse_json(&composition_json)?;
            if stored.is_null() {
                continue;
            }
            let artifact_ids: Vec<String> = stored
                .get("artifactIds")
                .and_then(Value::as_array)
                .and_then(|items| {
                    items
                        .iter()
                        .map(|item| item.as_str().map(|id| id.to_string()))
                        .collect()
                })
                .ok_or_else(|| GENERATION_COMPOSITION_INVALID.to_string())?;
            let algorithm = stored
                .get("algorithm")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let current = self.visible_composition(read, &artifact_ids, &algorithm)?;
            if algorithm != current.algorithm
                || stored.get("textHash").and_then(Value::as_str)
                    != Some(current.text_hash.as_str())
                || !values_equal(&stored.get("sources").cloned(), &current.sources)
                || !values_equal(&stored.get("chapters").cloned(), &current.chapters)
            {
                return Err(GENERATION_COMPOSITION_INTEGRITY_FAILED.to_string());
            }
            return Ok(Some(current));
        }
        Ok(None)
    }

    pub fn compose_visible(
        &self,
        conn: &Connection,
        run_id: &str,
        artifact_ids: &[String],
        expected_text_hash: Option<&str>,
        algorithm: &str,
    ) -> Result<VisibleCompositionReceipt, String> {
        in_transaction(conn, |tx| {
            self.assert_plot_outline_writable(tx, run_id)?;
            let run = self.get(tx, run_id)?;
            if run
                .binding
                .source_manifest
                .get("outputContract")
                .and_then(Value::as_str)
                != Some("visible-text")
            {
                return Err(GENERATION_COMPOSITION_OUTPUT_INVALID.to_string());
            }
            let mut read = self.read_state(tx, run)?;
            let next = self.visible_composition(&mut read, artifact_ids, algorithm)?;
            let previous = self.composition_from(&mut read)?;
            if let Some(previous) = &previous {
                if previous.algorithm != algorithm {
                    return Err(GENERATION_COMPOSITION_ALGORITHM_CHANGED.to_string());
                }
            }
            if let Some(expected_text_hash) = expected_text_hash {
                if next.text_hash != expected_text_hash {
                    return Err(GENERATION_COMPOSITION_HASH_MISMATCH.to_string());
                }
            }
            if let Some(previous) = &previous {
                if previous
                    .artifact_ids
                    .iter()
                    .enumerate()
                    .any(|(index, artifact_id)| {
                        next.artifact_ids
                            .get(index)
                            .is_none_or(|next_id| next_id != artifact_id)
                    })
                {
                    return Err(GENERATION_COMPOSITION_REGRESSION.to_string());
                }
            }
            let outline = self.plot_outline_from(&mut read, None, Some(previous.clone()))?;
            if outline.is_some()
                && !values_equal(
                    &next.artifact_ids,
                    &previous
                        .as_ref()
                        .map(|previous| previous.artifact_ids.clone()),
                )
            {
                let PlotOutlineCursor::Accept {
                    artifact_ids: cursor_artifact_ids,
                    ..
                } = &outline.unwrap().cursor
                else {
                    return Err(GENERATION_PLOT_OUTLINE_ACCEPT_CONFLICT.to_string());
                };
                let offset = previous
                    .as_ref()
                    .map(|previous| previous.artifact_ids.len())
                    .unwrap_or_default();
                if !values_equal(&next.artifact_ids[offset..].to_vec(), cursor_artifact_ids) {
                    return Err(GENERATION_PLOT_OUTLINE_ACCEPT_CONFLICT.to_string());
                }
            }
            let last_artifact_id = artifact_ids
                .last()
                .ok_or_else(|| GENERATION_COMPOSITION_INVALID.to_string())?;
            let (attempt_id, usage_receipt_json) = tx
                .query_row(
                    "SELECT t.attempt_id,t.usage_receipt_json FROM generation_attempts t JOIN generation_artifacts a ON a.attempt_id=t.attempt_id WHERE a.artifact_id=?",
                    [last_artifact_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .map_err(|error| error.to_string())?;
            // const { text: visibleText, ...stored } = next —— 存储时剥离正文。
            let mut stored = serde_json::to_value(&next).map_err(|error| error.to_string())?;
            if let Some(object) = stored.as_object_mut() {
                object.remove("text");
            }
            let mut updated: Value = parse_json(&usage_receipt_json)?;
            updated
                .as_object_mut()
                .ok_or_else(|| GENERATION_COMPOSITION_OUTPUT_INVALID.to_string())?
                .insert("visibleComposition".to_string(), stored);
            tx.execute(
                "UPDATE generation_attempts SET usage_receipt_json=? WHERE attempt_id=?",
                params![encode(&updated)?, attempt_id],
            )
            .map_err(|error| error.to_string())?;
            Ok(next)
        })
    }

    // ---------------------------------------------------------------------
    // 结算与快照
    // ---------------------------------------------------------------------

    pub fn settle(
        &self,
        conn: &Connection,
        attempt_id: &str,
        usage: Option<GenerationUsageReceipt>,
        finish_reason: Option<String>,
        failure_code: Option<String>,
        diagnostics: Option<GenerationTransportDiagnostics>,
    ) -> Result<GenerationExecutionReceipt, String> {
        in_transaction(conn, |tx| {
            let receipt = self.receipt(tx, attempt_id, None)?;
            self.assert_plot_outline_writable(tx, &receipt.run.run_id)?;
            let settled_status = if usage.as_ref().is_some_and(|usage| usage.trusted) {
                "settled"
            } else {
                "unknown"
            };
            assert_attempt_transition(&receipt.attempt.status, settled_status)?;
            let mut attempt = receipt.attempt.clone();
            attempt.status = settled_status.to_string();
            if usage.as_ref().is_some_and(|usage| usage.trusted) {
                attempt.actual_tokens = usage.as_ref().and_then(|usage| usage.actual_tokens);
            }
            let old: String = tx
                .query_row(
                    "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id=?",
                    [attempt_id],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            let mut old_value: Value = parse_json(&old)?;
            let mut result = json!({
                "usage": usage,
                "finishReason": finish_reason,
            });
            if let Some(failure_code) = &failure_code {
                result["failureCode"] = json!(failure_code);
            }
            if let Some(diagnostics) = &diagnostics {
                result["diagnostics"] = json!(diagnostics);
            }
            old_value
                .as_object_mut()
                .ok_or_else(|| GENERATION_JSON_PARSE_ERROR.to_string())?
                .insert("result".to_string(), result);
            tx.execute(
                "UPDATE generation_attempts SET attempt_json=?,usage_receipt_json=? WHERE attempt_id=?",
                params![encode(&attempt)?, encode(&old_value)?, attempt_id],
            )
            .map_err(|error| error.to_string())?;
            // 无 policy 的旧收据保留其原有硬约束规则。
            // 估计超支保持为实际根用量，绝不裁剪或抹除。
            let can_bound = old_value
                .get("usagePolicy")
                .and_then(|policy| policy.get("canBoundTotalLiability"))
                != Some(&Value::Bool(false));
            let actual_tokens = usage.as_ref().and_then(|usage| usage.actual_tokens);
            if can_bound
                && actual_tokens.is_some()
                && actual_tokens.unwrap_or_default() > receipt.attempt.reserved_tokens
            {
                self.block(tx, &receipt.run.root_action_id, USAGE_EXCEEDED_RESERVATION)?;
            }
            self.stop_clock_if_idle(tx, &receipt.run.root_action_id)?;
            self.receipt(tx, attempt_id, None)
        })
    }

    fn stop_clock_if_idle(&self, conn: &Connection, root_id: &str) -> Result<(), String> {
        if self
            .budget(conn, root_id)?
            .attempts
            .iter()
            .any(|attempt| attempt.status == "dispatch-marked")
        {
            return Ok(());
        }
        // 基线此处二次读取 budget（now() 可能前进），保持两次调用以对齐行为。
        let elapsed = self.budget(conn, root_id)?.active_elapsed_ms;
        conn.execute(
            "UPDATE generation_roots SET active_elapsed_ms=?,active_since_ms=NULL WHERE root_action_id=?",
            params![elapsed, root_id],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn snapshot(
        &self,
        conn: &Connection,
        attempt_id: &str,
        expected_revision: i64,
        text: &str,
    ) -> Result<VisibleArtifact, String> {
        in_transaction(conn, |tx| {
            let current = self.receipt(tx, attempt_id, None)?;
            let prior = current
                .artifact
                .clone()
                .ok_or_else(|| ARTIFACT_SNAPSHOT_CONFLICT.to_string())?;
            self.assert_plot_outline_writable(tx, &current.run.run_id)?;
            if !same_project_epoch(&prior, &current.budget.root) {
                return Err(GENERATION_EPOCH_STALE.to_string());
            }
            if prior.revision == expected_revision + 1 && prior.text == text {
                return Ok(prior);
            }
            if ["settled", "unknown", "cancelled-before-dispatch"]
                .contains(&current.attempt.status.as_str())
            {
                return Err(ARTIFACT_ATTEMPT_TERMINAL.to_string());
            }
            if prior.revision != expected_revision
                || !text.starts_with(&prior.text)
                || RE_THINK_TAG_BOUNDARY.is_match(text)
            {
                return Err(ARTIFACT_SNAPSHOT_CONFLICT.to_string());
            }
            let next = VisibleArtifact {
                text: text.to_string(),
                text_hash: text_hash(text),
                revision: expected_revision + 1,
                ..prior
            };
            let changes = tx
                .execute(
                    "UPDATE generation_artifacts SET artifact_json=?,revision=? WHERE attempt_id=? AND revision=?",
                    params![encode(&next)?, next.revision, attempt_id, expected_revision],
                )
                .map_err(|error| error.to_string())?;
            if changes != 1 {
                return Err(ARTIFACT_SNAPSHOT_CONFLICT.to_string());
            }
            Ok(next)
        })
    }

    // ---------------------------------------------------------------------
    // 根控制
    // ---------------------------------------------------------------------

    pub fn block(&self, conn: &Connection, root_id: &str, code: &str) -> Result<(), String> {
        if self.author_edited_root(conn, root_id)? {
            return Err(GENERATION_PLOT_OUTLINE_AUTHOR_SAVED.to_string());
        }
        conn.execute(
            "UPDATE generation_roots SET blocked_code=? WHERE root_action_id=?",
            params![code, root_id],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn pause(&self, conn: &Connection, root_id: &str, cancel: bool) -> Result<(), String> {
        in_transaction(conn, |tx| self.pause_in_transaction(tx, root_id, cancel))
    }

    fn pause_in_transaction(
        &self,
        tx: &Transaction<'_>,
        root_id: &str,
        cancel: bool,
    ) -> Result<(), String> {
        if self.author_edited_root(tx, root_id)? {
            return Ok(());
        }
        let mut budget = self.budget(tx, root_id)?;
        if ["cancelled", "sealed"].contains(&budget.root.status.as_str()) {
            return Ok(());
        }
        budget.root.status = if cancel { "cancelled" } else { "paused" }.to_string();
        tx.execute(
            "UPDATE generation_roots SET action_json=?,active_elapsed_ms=?,active_since_ms=NULL WHERE root_action_id=?",
            params![encode(&budget.root)?, budget.active_elapsed_ms, root_id],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            "UPDATE generation_runs SET status=? WHERE root_action_id=?",
            params![if cancel { "cancelled" } else { "paused" }, root_id],
        )
        .map_err(|error| error.to_string())?;
        for attempt in &mut budget.attempts {
            if attempt.status == "reserved" {
                attempt.status = "cancelled-before-dispatch".to_string();
                tx.execute(
                    "UPDATE generation_attempts SET attempt_json=? WHERE attempt_id=?",
                    params![encode(attempt)?, attempt.attempt_id],
                )
                .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    pub fn resume(&self, conn: &Connection, run_id: &str, next: &RunBinding) -> Result<(), String> {
        in_transaction(conn, |tx| {
            self.assert_plot_outline_writable(tx, run_id)?;
            let run = self.get(tx, run_id)?;
            let budget = self.budget(tx, &run.root_action_id)?;
            Self::binding(next)?;
            if budget
                .attempts
                .iter()
                .any(|attempt| attempt.status == "dispatch-marked")
                || budget.root.status != "paused"
                || budget.blocked_code.is_some()
                || next.project_id != run.binding.project_id
            {
                return Err(GENERATION_RESUME_REFUSED.to_string());
            }
            let mut root = budget.root;
            root.epoch = next.epoch.clone();
            root.status = "active".to_string();
            tx.execute(
                "UPDATE generation_roots SET action_json=? WHERE root_action_id=?",
                params![encode(&root)?, run.root_action_id],
            )
            .map_err(|error| error.to_string())?;
            tx.execute(
                "UPDATE generation_runs SET binding_json=?,status=? WHERE run_id=?",
                params![encode(next)?, "running", run_id],
            )
            .map_err(|error| error.to_string())?;
            Ok(())
        })
    }

    pub fn recover_interrupted(&self, conn: &Connection) -> Result<(), String> {
        in_transaction(conn, |tx| {
            let mut stmt = tx
                .prepare("SELECT root_action_id FROM generation_roots")
                .map_err(|error| error.to_string())?;
            let root_ids = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            for root_id in root_ids {
                if self.frozen_root(tx, &root_id)? {
                    continue;
                }
                let mut budget = self.budget(tx, &root_id)?;
                for attempt in &mut budget.attempts {
                    if attempt.status == "dispatch-marked" {
                        attempt.status = "unknown".to_string();
                        tx.execute(
                            "UPDATE generation_attempts SET attempt_json=? WHERE attempt_id=?",
                            params![encode(attempt)?, attempt.attempt_id],
                        )
                        .map_err(|error| error.to_string())?;
                    }
                }
                self.pause_in_transaction(tx, &root_id, false)?;
            }
            Ok(())
        })
    }

    fn frozen_root(&self, conn: &Connection, root_id: &str) -> Result<bool, String> {
        if self.author_edited_root(conn, root_id)? {
            return Ok(true);
        }
        if (self.is_frozen)(FREEZE_TABLE_GENERATION_ROOTS, root_id) {
            return Ok(true);
        }
        let run_ids = self.query_column(
            conn,
            "SELECT run_id FROM generation_runs WHERE root_action_id=?",
            root_id,
        )?;
        if run_ids
            .iter()
            .any(|run_id| (self.is_frozen)(FREEZE_TABLE_GENERATION_RUNS, run_id))
        {
            return Ok(true);
        }
        let attempt_ids = self.query_column(
            conn,
            "SELECT attempt_id FROM generation_attempts WHERE root_action_id=?",
            root_id,
        )?;
        Ok(attempt_ids
            .iter()
            .any(|attempt_id| (self.is_frozen)(FREEZE_TABLE_GENERATION_ATTEMPTS, attempt_id)))
    }

    fn query_column(
        &self,
        conn: &Connection,
        sql: &str,
        parameter: &str,
    ) -> Result<Vec<String>, String> {
        let mut stmt = conn.prepare(sql).map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([parameter], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(rows)
    }

    pub fn list_candidates(
        &self,
        conn: &Connection,
        run_id: Option<&str>,
    ) -> Result<Vec<Option<VisibleArtifact>>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT attempt_id FROM generation_artifacts WHERE status!='discarded' AND (? IS NULL OR run_id=?) ORDER BY rowid",
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![run_id, run_id], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let budget = match run_id {
            Some(run_id) => Some(self.budget(conn, &self.get(conn, run_id)?.root_action_id)?),
            None => None,
        };
        let mut artifacts = Vec::new();
        for attempt_id in rows {
            if (self.is_frozen)(FREEZE_TABLE_GENERATION_ATTEMPTS, &attempt_id) {
                continue;
            }
            // 便携历史保留可见文本但没有当前执行收据。
            let artifact = self.receipt(conn, &attempt_id, budget.clone())?.artifact;
            artifacts.push(artifact);
        }
        Ok(artifacts)
    }

    pub fn discard_candidate(&self, conn: &Connection, artifact_id: &str) -> Result<(), String> {
        let run_id = conn
            .query_row(
                "SELECT run_id FROM generation_artifacts WHERE artifact_id=?",
                [artifact_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some(run_id) = run_id {
            self.assert_plot_outline_writable(conn, &run_id)?;
        }
        conn.execute(
            "UPDATE generation_artifacts SET status='discarded' WHERE artifact_id=?",
            [artifact_id],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }
}

/// `read_state` 的内部游标：备忘化 `receipt` 计算与跨 run 查找缓存。
struct GenerationRead<'a> {
    repo: &'a GenerationRunRepository,
    conn: &'a Connection,
    run: DurableGenerationRun,
    budget: GenerationBudgetReceipt,
    rows: Vec<ReceiptRow>,
    /// attempt_id → `rows` 索引。
    attempts: HashMap<String, usize>,
    /// artifact_id → `rows` 索引。
    artifacts: HashMap<String, usize>,
    /// run_id → 已加载的 `DurableGenerationRun`。
    runs: HashMap<String, DurableGenerationRun>,
    /// attempt_id → 备忘化的 `{usage, receipt}`。
    receipts: HashMap<String, ReceiptWithUsage>,
}

impl<'a> GenerationRead<'a> {
    fn receipt(&mut self, attempt_id: &str) -> Result<ReceiptWithUsage, String> {
        if let Some(prior) = self.receipts.get(attempt_id) {
            return Ok(prior.clone());
        }
        let row_index = *self
            .attempts
            .get(attempt_id)
            .ok_or_else(|| GENERATION_ATTEMPT_MISSING.to_string())?;
        let row = self.rows[row_index].clone();
        let owner = match self.runs.get(&row.run_id) {
            Some(owner) => owner.clone(),
            None => {
                let owner = self.repo.get(self.conn, &row.run_id)?;
                self.runs.insert(owner.run_id.clone(), owner.clone());
                owner
            }
        };
        let usage: ReceiptUsage = parse_json(&row.usage_receipt_json)?;
        let budget = if owner.root_action_id == self.budget.root.root_action_id {
            self.budget.clone()
        } else {
            self.repo.budget(self.conn, &owner.root_action_id)?
        };
        let receipt = self.repo.hydrate_receipt(&row, &owner, &usage, &budget)?;
        let value = ReceiptWithUsage { usage, receipt };
        self.receipts.insert(attempt_id.to_string(), value.clone());
        Ok(value)
    }
}

/// `[\p{L}\p{N}]/gu` 的等价计数（字母 + 数字）。
fn count_letters_and_numbers(text: &str) -> usize {
    text.chars()
        .filter(|char| char.is_alphabetic() || char.is_numeric())
        .count()
}

/// `open()` 的 sourceRefs 去 epoch 深比较（TS `delete copy.epoch`）。
fn source_refs_without_epoch(source_refs: &[SourceRef]) -> Vec<Value> {
    source_refs
        .iter()
        .map(|source_ref| {
            let mut value = serde_json::to_value(source_ref).unwrap_or_default();
            if let Some(object) = value.as_object_mut() {
                object.remove("epoch");
            }
            value
        })
        .collect()
}

// ============================================================================
// 单测
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        conn
    }

    fn test_fingerprint() -> FrozenInputFingerprint {
        FrozenInputFingerprint {
            chapter_brief_hash: "a".repeat(64),
            author_guidance_hash: "b".repeat(64),
            dependency_hash: "c".repeat(64),
            context_snapshot_hash: "d".repeat(64),
            template_hash: "e".repeat(64),
            skill_snapshot_hash: "f".repeat(64),
            model_lease_revision: "0".repeat(64),
            policy_hash: "1".repeat(64),
            output_contract_hash: "2".repeat(64),
        }
    }

    fn test_root_action() -> RootAction {
        RootAction {
            project_id: "project-1".to_string(),
            epoch: "epoch-1".to_string(),
            root_action_id: "root-1".to_string(),
            operation: "generate-chapter".to_string(),
            ui_action_nonce: "nonce-1".to_string(),
            frozen_input_hash: "a".repeat(64),
            status: "active".to_string(),
        }
    }

    fn test_physical_attempt(status: &str) -> PhysicalAttempt {
        PhysicalAttempt {
            attempt_id: "attempt-1".to_string(),
            reservation_id: "reservation-1".to_string(),
            root_action_id: "root-1".to_string(),
            status: status.to_string(),
            reserved_tokens: 100,
            requested_output_tokens: 50,
            actual_tokens: None,
        }
    }

    fn test_policy() -> RootBudget {
        RootBudget {
            max_physical_requests: 4,
            max_token_liability: 1000,
            max_output_per_request: 512,
            max_active_elapsed_ms: 60000,
        }
    }

    #[test]
    fn text_hash_known_vectors_test() {
        assert_eq!(
            text_hash(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            text_hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // 中文正文按 UTF-8 字节哈希（与上游 createHash('sha256').update(text,'utf8') 一致）。
        assert_eq!(
            text_hash("第一章"),
            "9134b008252f267ca0728ab30ffbe577efb573baa06e54cb3338e2d277860ce0"
        );
        assert_ne!(text_hash("第一章"), text_hash("第一章 "));
    }

    #[test]
    fn is_content_hash_test() {
        assert!(is_content_hash(&"a".repeat(64)));
        assert!(is_content_hash(&"0123456789abcdef".repeat(4)));
        assert!(!is_content_hash(&"A".repeat(64))); // 大写不合法
        assert!(!is_content_hash(&"a".repeat(63)));
        assert!(!is_content_hash(&"g".repeat(64)));
        assert!(!is_content_hash(""));
    }

    #[test]
    fn root_action_idempotency_key_test() {
        let action = test_root_action();
        let key = root_action_idempotency_key(&action).unwrap();
        let expected = json!(["project-1", "generate-chapter", "nonce-1", "a".repeat(64)]);
        assert_eq!(key, encode(&expected).unwrap());
        // 五个字段任一为空（trim 后）→ INVALID_ROOT_ACTION
        let mut blank = action.clone();
        blank.ui_action_nonce = "   ".to_string();
        assert_eq!(
            root_action_idempotency_key(&blank).unwrap_err(),
            INVALID_ROOT_ACTION
        );
        // frozenInputHash 非内容哈希 → INVALID_ROOT_ACTION
        let mut bad_hash = action;
        bad_hash.frozen_input_hash = "xyz".to_string();
        assert_eq!(
            root_action_idempotency_key(&bad_hash).unwrap_err(),
            INVALID_ROOT_ACTION
        );
    }

    #[test]
    fn token_liability_branches_test() {
        let cancelled = PhysicalAttempt {
            status: "cancelled-before-dispatch".to_string(),
            reserved_tokens: 100,
            actual_tokens: Some(500),
            ..test_physical_attempt("cancelled-before-dispatch")
        };
        assert_eq!(token_liability(&cancelled), 0);
        let settled_with_actual = PhysicalAttempt {
            status: "settled".to_string(),
            actual_tokens: Some(300),
            ..test_physical_attempt("settled")
        };
        assert_eq!(token_liability(&settled_with_actual), 300);
        let settled_without_actual = PhysicalAttempt {
            status: "settled".to_string(),
            actual_tokens: None,
            ..test_physical_attempt("settled")
        };
        assert_eq!(token_liability(&settled_without_actual), 100);
        let reserved = test_physical_attempt("reserved");
        assert_eq!(token_liability(&reserved), 100);
    }

    #[test]
    fn assert_attempt_transition_test() {
        assert!(assert_attempt_transition("reserved", "dispatch-marked").is_ok());
        assert!(assert_attempt_transition("reserved", "cancelled-before-dispatch").is_ok());
        assert!(assert_attempt_transition("dispatch-marked", "settled").is_ok());
        assert!(assert_attempt_transition("dispatch-marked", "unknown").is_ok());
        assert_eq!(
            assert_attempt_transition("reserved", "settled").unwrap_err(),
            INVALID_ATTEMPT_TRANSITION
        );
        assert_eq!(
            assert_attempt_transition("settled", "dispatch-marked").unwrap_err(),
            INVALID_ATTEMPT_TRANSITION
        );
        assert_eq!(
            assert_attempt_transition("unknown", "settled").unwrap_err(),
            INVALID_ATTEMPT_TRANSITION
        );
    }

    #[test]
    fn assert_reservation_error_codes_test() {
        let root = test_root_action();
        let policy = test_policy();
        let next = test_physical_attempt("reserved");
        // 合法预留通过
        assert!(assert_reservation(&root, &policy, &[], &next, 0).is_ok());
        // 预算字段非正 → INVALID_BUDGET
        let mut bad_policy = policy.clone();
        bad_policy.max_physical_requests = 0;
        assert_eq!(
            assert_reservation(&root, &bad_policy, &[], &next, 0).unwrap_err(),
            INVALID_BUDGET
        );
        // elapsedMs 为负 → INVALID_BUDGET
        assert_eq!(
            assert_reservation(&root, &policy, &[], &next, -1).unwrap_err(),
            INVALID_BUDGET
        );
        // root 非 active → INVALID_RESERVATION
        let mut paused_root = root.clone();
        paused_root.status = "paused".to_string();
        assert_eq!(
            assert_reservation(&paused_root, &policy, &[], &next, 0).unwrap_err(),
            INVALID_RESERVATION
        );
        // 输出预留超限 → INVALID_OUTPUT_RESERVATION
        let mut oversized = next.clone();
        oversized.requested_output_tokens = 600;
        assert_eq!(
            assert_reservation(&root, &policy, &[], &oversized, 0).unwrap_err(),
            INVALID_OUTPUT_RESERVATION
        );
        // attempt_id 重复 → RESERVATION_CONFLICT
        assert_eq!(
            assert_reservation(&root, &policy, &[next.clone()], &next, 0).unwrap_err(),
            RESERVATION_CONFLICT
        );
        // 账本 attempt 状态非法 → INVALID_LEDGER
        let bad_status = PhysicalAttempt {
            attempt_id: "ledger-attempt".to_string(),
            reservation_id: "ledger-reservation".to_string(),
            ..test_physical_attempt("bogus")
        };
        assert_eq!(
            assert_reservation(&root, &policy, &[bad_status], &next, 0).unwrap_err(),
            INVALID_LEDGER
        );
        // 根预算耗尽 → ROOT_BUDGET_EXHAUSTED
        let exhausted = PhysicalAttempt {
            attempt_id: "exhausted-attempt".to_string(),
            reservation_id: "exhausted-reservation".to_string(),
            reserved_tokens: 900,
            ..test_physical_attempt("settled")
        };
        assert_eq!(
            assert_reservation(&root, &policy, &[exhausted], &next, 60000).unwrap_err(),
            ROOT_BUDGET_EXHAUSTED
        );
    }

    #[test]
    fn parse_planning_target_units_test() {
        assert_eq!(
            parse_planning_target_units(None).unwrap(),
            DEFAULT_PLANNING_TARGET_UNITS
        );
        assert_eq!(parse_planning_target_units(Some(json!(600))).unwrap(), 600);
        assert_eq!(parse_planning_target_units(Some(json!(1))).unwrap(), 1);
        assert_eq!(
            parse_planning_target_units(Some(json!(1000))).unwrap(),
            1000
        );
        assert_eq!(
            parse_planning_target_units(Some(json!(0))).unwrap_err(),
            GENERATION_PLANNING_TARGET_INVALID
        );
        assert_eq!(
            parse_planning_target_units(Some(json!(1001))).unwrap_err(),
            GENERATION_PLANNING_TARGET_INVALID
        );
        assert_eq!(
            parse_planning_target_units(Some(json!("600"))).unwrap_err(),
            GENERATION_PLANNING_TARGET_INVALID
        );
        assert_eq!(
            parse_planning_target_units(Some(json!(null))).unwrap_err(),
            GENERATION_PLANNING_TARGET_INVALID
        );
    }

    #[test]
    fn plot_outline_purpose_round_trip_test() {
        assert_eq!(
            plot_outline_purpose(5, "normal"),
            "plot-outline:chapter:5:normal"
        );
        assert_eq!(
            plot_outline_purpose(12, "compact"),
            "plot-outline:chapter:12:compact"
        );
        let parsed = parse_plot_outline_purpose("plot-outline:chapter:5:normal").unwrap();
        assert_eq!(parsed.chapter_number, 5);
        assert_eq!(parsed.attempt, "normal");
        assert!(parse_plot_outline_purpose("plot-outline:chapter:0:normal").is_none()); // 前导零
        assert!(parse_plot_outline_purpose("plot-outline:chapter:5:brief").is_none());
        assert!(parse_plot_outline_purpose("plot-outline:chapter:5").is_none());
        assert!(parse_plot_outline_purpose("other:chapter:5:normal").is_none());
    }

    #[test]
    fn chapter_number_parsing_test() {
        assert_eq!(chapter_number("1"), 1);
        assert_eq!(chapter_number("123"), 123);
        assert_eq!(chapter_number("十"), 10);
        assert_eq!(chapter_number("二十"), 20);
        assert_eq!(chapter_number("二十一"), 21);
        assert_eq!(chapter_number("一百零三"), 103);
        assert_eq!(chapter_number("一千零五"), 1005);
        assert_eq!(chapter_number("九十九"), 99);
        assert_eq!(chapter_number("两"), 2);
    }

    #[test]
    fn headings_parsing_test() {
        // 简单中文章标题
        let entries = headings("第1章：开端\n正文");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].from, 1);
        assert_eq!(entries[0].to, 1);
        assert!(!entries[0].grouped);
        // 中文范围
        let entries = headings("第1–3章：卷一");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].from, 1);
        assert_eq!(entries[0].to, 3);
        assert!(entries[0].grouped);
        // 英文章标题
        let entries = headings("Chapters 1-3: Range");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].from, 1);
        assert_eq!(entries[0].to, 3);
        assert!(entries[0].grouped);
        // 单一英文标题不 group
        let entries = headings("Chapter 5: Single");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].from, 5);
        assert_eq!(entries[0].to, 5);
        assert!(!entries[0].grouped);
        // markdown 前缀
        let entries = headings("## 第7章：带前缀");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].from, 7);
        // 非标题行
        assert!(headings("普通正文\n没有标题").is_empty());
        // 前瞻标点集
        assert!(headings("第2章.").len() == 1);
        assert!(headings("第2章").len() == 1);
        // 行偏移为字节偏移
        let entries = headings("前言\n第9章：偏移");
        assert_eq!(entries[0].index, "前言\n".len());
        assert_eq!(entries[0].end, "前言\n第9章：偏移".len());
    }

    #[test]
    fn valid_plot_outline_entry_test() {
        let valid = "## 第5章：标题\n正文内容";
        assert!(valid_plot_outline_entry(valid, 5));
        // 缺换行（单行）→ 无效
        assert!(!valid_plot_outline_entry("## 第5章：标题", 5));
        // 章号不匹配 → 无效
        assert!(!valid_plot_outline_entry("## 第5章：标题\n正文", 6));
        // 禁用标记 → 无效
        assert!(!valid_plot_outline_entry("## 第5章：标题\n```\n代码\n", 5));
        assert!(!valid_plot_outline_entry("## 第5章：标题\n后续概览\n", 5));
        // 正文含标题行 → 无效
        assert!(!valid_plot_outline_entry(
            "## 第5章：标题\n## 第6章：其他\n",
            5
        ));
        // 分组标题 → 无效
        assert!(!valid_plot_outline_entry("## 第5章–第7章：组\n正文", 5));
        // 标题前有内容 → 无效
        assert!(!valid_plot_outline_entry("前言\n## 第5章：标题\n正文", 5));
        // 标题后无内容 → 无效
        assert!(!valid_plot_outline_entry("## 第5章：标题\n", 5));
    }

    #[test]
    fn derive_plot_outline_cursor_test() {
        let range = PlotOutlineRange { from: 1, to: 3 };
        // 无匹配 attempt → request
        let cursor = derive_plot_outline_cursor(&range, 0, &[]).unwrap();
        assert!(matches!(
            cursor,
            PlotOutlineCursor::Request {
                chapter_number: 1,
                ..
            }
        ));
        // 超出范围 → complete
        let cursor = derive_plot_outline_cursor(&range, 3, &[]).unwrap();
        assert!(matches!(cursor, PlotOutlineCursor::Complete));
        // in-flight
        let attempts = vec![PlotOutlineAttempt {
            purpose: plot_outline_purpose(1, "normal"),
            status: "reserved".to_string(),
            finish_reason: None,
            artifact: None,
        }];
        let cursor = derive_plot_outline_cursor(&range, 0, &attempts).unwrap();
        assert!(
            matches!(cursor, PlotOutlineCursor::Stopped { reason, .. } if reason == "in-flight")
        );
        // 未知完成
        let attempts = vec![PlotOutlineAttempt {
            purpose: plot_outline_purpose(1, "normal"),
            status: "settled".to_string(),
            finish_reason: Some("error".to_string()),
            artifact: None,
        }];
        let cursor = derive_plot_outline_cursor(&range, 0, &attempts).unwrap();
        assert!(
            matches!(cursor, PlotOutlineCursor::Stopped { reason, .. } if reason == "unknown-completion")
        );
        // 合法接受
        let attempts = vec![PlotOutlineAttempt {
            purpose: plot_outline_purpose(1, "normal"),
            status: "settled".to_string(),
            finish_reason: Some("stop".to_string()),
            artifact: Some(PlotOutlineAttemptArtifact {
                artifact_id: "artifact-1".to_string(),
                text: "## 第1章：标题\n正文".to_string(),
                revision: 0,
                text_hash: "x".repeat(64),
            }),
        }];
        let cursor = derive_plot_outline_cursor(&range, 0, &attempts).unwrap();
        assert!(matches!(
            cursor,
            PlotOutlineCursor::Accept {
                chapter_number: 1,
                ..
            }
        ));
        // 重复 attempt → ATTEMPT_CONFLICT
        let attempts = vec![
            PlotOutlineAttempt {
                purpose: plot_outline_purpose(1, "normal"),
                status: "settled".to_string(),
                finish_reason: Some("stop".to_string()),
                artifact: None,
            },
            PlotOutlineAttempt {
                purpose: plot_outline_purpose(1, "normal"),
                status: "settled".to_string(),
                finish_reason: Some("stop".to_string()),
                artifact: None,
            },
        ];
        assert_eq!(
            derive_plot_outline_cursor(&range, 0, &attempts).unwrap_err(),
            GENERATION_PLOT_OUTLINE_ATTEMPT_CONFLICT
        );
        // 两遍耗尽
        let attempts = vec![
            PlotOutlineAttempt {
                purpose: plot_outline_purpose(1, "normal"),
                status: "settled".to_string(),
                finish_reason: Some("stop".to_string()),
                artifact: Some(PlotOutlineAttemptArtifact {
                    artifact_id: "artifact-1".to_string(),
                    text: "无效正文".to_string(),
                    revision: 0,
                    text_hash: "x".repeat(64),
                }),
            },
            PlotOutlineAttempt {
                purpose: plot_outline_purpose(1, "compact"),
                status: "settled".to_string(),
                finish_reason: Some("stop".to_string()),
                artifact: Some(PlotOutlineAttemptArtifact {
                    artifact_id: "artifact-2".to_string(),
                    text: "无效正文".to_string(),
                    revision: 0,
                    text_hash: "x".repeat(64),
                }),
            },
        ];
        let cursor = derive_plot_outline_cursor(&range, 0, &attempts).unwrap();
        assert!(
            matches!(cursor, PlotOutlineCursor::Stopped { reason, .. } if reason == "attempts-exhausted")
        );
    }

    #[test]
    fn render_plot_outline_synopsis_test() {
        let expected = ProjectCoreSynopsisExpected {
            synopsis: String::new(),
            premise: String::new(),
            characters_arch: String::new(),
            worldbuilding: String::new(),
            genre: String::new(),
            total_chapters: 10,
            words_per_chapter: 600,
            writing_language: "zh-CN".to_string(),
            plot_structure: String::new(),
            narrative_pov: String::new(),
            global_guidance: String::new(),
        };
        let rendered = render_plot_outline_synopsis("正文", 3, &expected);
        assert!(rendered.starts_with("# 情节大纲\n\n"));
        assert!(rendered
            .contains("> 本大纲已覆盖至第 3 章（全书 10 章），其余章节将在后续批次继续生成。"));
        assert!(rendered.ends_with("继续生成。"));
        // 覆盖全部章节时无尾部
        let rendered = render_plot_outline_synopsis("正文", 10, &expected);
        assert!(!rendered.contains("本大纲已覆盖"));
        // 英文
        let mut english = expected.clone();
        english.writing_language = "en-US".to_string();
        let rendered = render_plot_outline_synopsis("body", 3, &english);
        assert!(rendered.starts_with("# Plot Outline\n\n"));
        assert!(rendered.contains("> This outline covers chapters 1-3 of 10;"));
    }

    #[test]
    fn join_plot_outline_entries_test() {
        assert_eq!(join_plot_outline_entries("a", "b"), "a\n\nb");
        assert_eq!(join_plot_outline_entries(" a ", " b "), "a\n\nb");
        assert_eq!(join_plot_outline_entries("", "b"), "b");
        assert_eq!(join_plot_outline_entries("a", ""), "a");
        assert_eq!(join_plot_outline_entries("", ""), "");
    }

    #[test]
    fn plot_outline_confirmed_prefix_test() {
        let expected = ProjectCoreSynopsisExpected {
            synopsis: "# 情节大纲\n\n## 第1章：一\n\n正文一\n\n## 第2章：二\n\n正文二\n\n> 本大纲已覆盖至第 2 章（全书 5 章），其余章节将在后续批次继续生成。".to_string(),
            premise: String::new(),
            characters_arch: String::new(),
            worldbuilding: String::new(),
            genre: String::new(),
            total_chapters: 5,
            words_per_chapter: 600,
            writing_language: "zh-CN".to_string(),
            plot_structure: String::new(),
            narrative_pov: String::new(),
            global_guidance: String::new(),
        };
        // from=1 → 空前缀
        assert_eq!(
            plot_outline_confirmed_prefix(&expected, &PlotOutlineRange { from: 1, to: 5 }).unwrap(),
            ""
        );
        // 范围与既有章组重叠 → 抛错
        let grouped = ProjectCoreSynopsisExpected {
            synopsis: "# 情节大纲\n\n## 第1–2章：组\n\n正文组".to_string(),
            ..expected.clone()
        };
        let overlapping = PlotOutlineRange { from: 2, to: 2 };
        assert_eq!(
            plot_outline_confirmed_prefix(&grouped, &overlapping).unwrap_err(),
            GENERATION_PLOT_OUTLINE_PREFIX_INVALID
        );
        // 连续前缀
        let prefix =
            plot_outline_confirmed_prefix(&expected, &PlotOutlineRange { from: 3, to: 5 }).unwrap();
        assert_eq!(prefix, "## 第1章：一\n\n正文一\n\n## 第2章：二\n\n正文二");
        // 不连续（缺第1章）→ 抛错
        let broken = ProjectCoreSynopsisExpected {
            synopsis: "# 情节大纲\n\n## 第2章：二\n\n正文二".to_string(),
            ..expected.clone()
        };
        assert_eq!(
            plot_outline_confirmed_prefix(&broken, &PlotOutlineRange { from: 3, to: 5 })
                .unwrap_err(),
            GENERATION_PLOT_OUTLINE_PREFIX_INVALID
        );
    }

    #[test]
    fn compose_visible_continuation_test() {
        // 重叠去重（重叠达到 48 字符阈值时裁剪重复前缀）
        let overlap = "共同重叠正文".repeat(8);
        let composed =
            compose_visible_continuation(&format!("前文{overlap}"), &format!("{overlap}后续"));
        assert_eq!(composed, format!("前文{overlap}\n\n后续"));
        // 无重叠
        let composed = compose_visible_continuation("甲乙丙", "丁戊己");
        assert_eq!(composed, "甲乙丙\n\n丁戊己");
        // 空输入
        assert_eq!(compose_visible_continuation("", "新内容"), "新内容");
        assert_eq!(compose_visible_continuation("旧内容", ""), "旧内容");
        assert_eq!(compose_visible_continuation("", ""), "");
        // 重叠阈值：小于 48 字符的重叠不裁剪
        let short_overlap = compose_visible_continuation("abcdefghij", "fghijklmno");
        assert_eq!(short_overlap, "abcdefghij\n\nfghijklmno");
    }

    #[test]
    fn strip_draft_thinking_tags_test() {
        // 配对标签剥离
        assert_eq!(
            strip_draft_thinking_tags("<thinking>思考内容</thinking>可见正文"),
            "可见正文"
        );
        // 无标签
        assert_eq!(strip_draft_thinking_tags("纯正文"), "纯正文");
        // 孤儿闭合标签 + 推理前缀 → 视为隐藏推理
        assert_eq!(
            strip_draft_thinking_tags("推理分析</think>结构化"),
            "结构化"
        );
        // 孤儿闭合标签 + 普通前缀 → 保留正文
        assert_eq!(
            strip_draft_thinking_tags("普通前缀</think>正文"),
            "普通前缀正文"
        );
        // 孤儿闭合标签 + 结构化后缀 → 视为隐藏推理
        assert_eq!(
            strip_draft_thinking_tags("前缀</think>{\"a\":1}"),
            "{\"a\":1}"
        );
    }

    #[test]
    fn sanitize_draft_text_test() {
        // thinking 剥离 + 继续提示移除 + 多空行折叠
        let text = "<thinking>思考</thinking>第一段\n\n\n\n点我继续生成后续内容\n\n第二段";
        assert_eq!(
            sanitize_draft_text(text, DRAFT_VISIBLE_TEXT_VERSION),
            "第一段\n\n第二段"
        );
        // v1 去重（key 长度 >= 40 的重复段丢弃）
        let long = "a".repeat(50);
        let v1_text = format!("{long}\n\n{long}\n\n短段");
        assert_eq!(
            sanitize_draft_text(&v1_text, "draft-visible-v1"),
            format!("{long}\n\n短段")
        );
        // v2 不去重
        assert_eq!(
            sanitize_draft_text(&v1_text, DRAFT_VISIBLE_TEXT_VERSION),
            format!("{long}\n\n{long}\n\n短段")
        );
        // 空白 key 不足 40 不去重
        let short = "短";
        let v1_short = format!("{short}\n\n{short}");
        assert_eq!(
            sanitize_draft_text(&v1_short, "draft-visible-v1"),
            format!("{short}\n\n{short}")
        );
    }

    #[test]
    fn draft_target_unit_range_test() {
        let range = draft_target_unit_range(100).unwrap();
        assert_eq!(range.minimum, 70);
        assert_eq!(range.maximum, 130);
        let range = draft_target_unit_range(1).unwrap();
        assert_eq!(range.minimum, 0);
        assert_eq!(range.maximum, 2);
        assert_eq!(
            draft_target_unit_range(0).unwrap_err(),
            DRAFT_TARGET_UNITS_INVALID
        );
        assert_eq!(
            draft_target_unit_range(-5).unwrap_err(),
            DRAFT_TARGET_UNITS_INVALID
        );
    }

    #[test]
    fn complete_handle_test() {
        let valid = json!({
            "projectId": "p",
            "epoch": "e",
            "rootActionId": "r",
            "runId": "u"
        });
        assert!(complete_handle(&valid));
        // 键数不足
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove("runId");
        assert!(!complete_handle(&missing));
        // 键数过多
        let mut extra = valid.clone();
        extra
            .as_object_mut()
            .unwrap()
            .insert("extra".to_string(), json!("x"));
        assert!(!complete_handle(&extra));
        // 空值
        let mut blank = valid.clone();
        blank
            .as_object_mut()
            .unwrap()
            .insert("runId".to_string(), json!("  "));
        assert!(!complete_handle(&blank));
        // 非对象
        assert!(!complete_handle(&json!("string")));
        assert!(!complete_handle(&Value::Null));
    }

    #[test]
    fn plot_outline_policy_test() {
        let mut manifest = Map::new();
        manifest.insert("operation".to_string(), json!("generate-plot-outline"));
        manifest.insert(
            "policy".to_string(),
            json!({
                "planning": {
                    "kind": "architecture",
                    "outlineProtocol": PLOT_OUTLINE_PROTOCOL,
                    "intent": { "synopsisRange": { "from": 2, "to": 6 } },
                    "outlineContent": { "maxAttemptsPerChapter": 2 }
                },
                "budget": { "maxPhysicalRequests": 10 }
            }),
        );
        let policy = plot_outline_policy(&manifest).unwrap().unwrap();
        assert_eq!(policy.range, PlotOutlineRange { from: 2, to: 6 });
        assert_eq!(policy.content.max_attempts_per_chapter, 2);
        // 非大纲操作 → None
        let mut other = manifest.clone();
        other.insert("operation".to_string(), json!("generate-chapter"));
        assert!(plot_outline_policy(&other).unwrap().is_none());
        // 非法 outlineContent → 抛错
        let mut invalid = manifest.clone();
        invalid
            .get_mut("policy")
            .unwrap()
            .get_mut("planning")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(
                "outlineContent".to_string(),
                json!({ "maxAttemptsPerChapter": 3 }),
            );
        assert_eq!(
            plot_outline_policy(&invalid).unwrap_err(),
            GENERATION_PLOT_OUTLINE_POLICY_INVALID
        );
        // 章号范围超过 maxPhysicalRequests → 抛错
        let mut exhausted = manifest.clone();
        exhausted
            .get_mut("policy")
            .unwrap()
            .get_mut("planning")
            .unwrap()
            .get_mut("intent")
            .unwrap()
            .get_mut("synopsisRange")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("to".to_string(), json!(20));
        assert_eq!(
            plot_outline_policy(&exhausted).unwrap_err(),
            GENERATION_PLOT_OUTLINE_POLICY_INVALID
        );
    }

    #[test]
    fn plot_outline_expected_test() {
        let valid = json!({
            "version": 1,
            "core": {
                "synopsis": "梗概",
                "premise": "前提",
                "characters_arch": "角色",
                "worldbuilding": "世界观",
                "genre": "类型",
                "writing_language": "en-US",
                "plot_structure": "结构",
                "narrative_pov": "视角",
                "global_guidance": "全局指导",
                "total_chapters": 10,
                "words_per_chapter": 600
            }
        });
        let expected = plot_outline_expected(&valid).unwrap();
        assert_eq!(expected.synopsis, "梗概");
        assert_eq!(expected.total_chapters, 10);
        assert_eq!(expected.writing_language, "en-US");
        // 非法 → 抛错
        assert_eq!(
            plot_outline_expected(&json!(null)).unwrap_err(),
            GENERATION_PLOT_OUTLINE_SOURCE_INVALID
        );
        let mut missing_field = valid.clone();
        missing_field
            .get_mut("core")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("synopsis");
        assert_eq!(
            plot_outline_expected(&missing_field).unwrap_err(),
            GENERATION_PLOT_OUTLINE_SOURCE_INVALID
        );
    }

    #[test]
    fn plot_outline_request_contract_test() {
        let progress = PlotOutlineProgress {
            protocol: PLOT_OUTLINE_PROTOCOL.to_string(),
            range: PlotOutlineRange { from: 2, to: 6 },
            content: PLOT_OUTLINE_CONTENT,
            target_units: 600,
            cursor: PlotOutlineCursor::Request {
                chapter_number: 5,
                attempt: "normal".to_string(),
            },
            source_expected: ProjectCoreSynopsisExpected {
                synopsis: String::new(),
                premise: String::new(),
                characters_arch: String::new(),
                worldbuilding: String::new(),
                genre: String::new(),
                total_chapters: 10,
                words_per_chapter: 600,
                writing_language: "zh-CN".to_string(),
                plot_structure: String::new(),
                narrative_pov: String::new(),
                global_guidance: String::new(),
            },
            confirmed_prefix: "已确认前缀".to_string(),
            composition: None,
        };
        let contract = plot_outline_request_contract(&progress, 5, "normal");
        assert!(contract.contains("【已接受大纲前缀】\n已确认前缀\n【已接受大纲前缀结束】"));
        assert!(contract.contains("本次只写第 5 章，所选范围为第 2–6 章"));
        assert!(contract.contains("每章大纲正文目标约 600 字"));
        assert!(contract.contains("保留作者硬事实、明确事件和尾部要求，不改写已接受前缀。"));
        // compact 附加重建说明
        let compact = plot_outline_request_contract(&progress, 5, "compact");
        assert!(compact.contains("本次从原始冻结事实和已接受前缀重新构建此章"));
        // 英文模板
        let mut english = progress.clone();
        english.source_expected.writing_language = "en-US".to_string();
        let contract = plot_outline_request_contract(&english, 5, "normal");
        assert!(contract.contains("[Accepted outline prefix]"));
        assert!(contract.contains("Write only Chapter 5, inside the selected range 2-6"));
        assert!(contract.contains("Aim for about 600 words per chapter in the outline body"));
    }

    #[test]
    fn js_number_test() {
        assert_eq!(js_number(Some(&json!("600"))), Some(600));
        assert_eq!(js_number(Some(&json!("  600  "))), Some(600));
        assert_eq!(js_number(Some(&json!(""))), Some(0));
        assert_eq!(js_number(Some(&json!("0x10"))), Some(16));
        assert_eq!(js_number(Some(&json!("1e3"))), Some(1000));
        assert_eq!(js_number(Some(&json!("abc"))), None);
        assert_eq!(js_number(Some(&json!(600))), Some(600));
        assert_eq!(js_number(None), None);
    }

    #[test]
    fn safe_integer_test() {
        assert!(is_safe_integer_value(9007199254740991));
        assert!(!is_safe_integer_value(9007199254740992));
        assert_eq!(safe_integer(Some(&json!(600))), Some(600));
        assert_eq!(safe_integer(Some(&json!(600.0))), Some(600));
        assert_eq!(safe_integer(Some(&json!("600"))), None);
        assert_eq!(safe_integer(Some(&json!(null))), None);
    }

    #[test]
    fn generation_run_lifecycle_test() {
        let repo = GenerationRunRepository::default();
        let conn = memory_db();
        let mut manifest = Map::new();
        manifest.insert("operation".to_string(), json!("generate-chapter"));
        let request = OpenGenerationRunRequest {
            project_id: "project-1".to_string(),
            epoch: "epoch-1".to_string(),
            fingerprint: test_fingerprint(),
            context_snapshot_id: "snapshot-1".to_string(),
            source_manifest: manifest,
            source_refs: vec![SourceRef {
                project_id: "project-1".to_string(),
                epoch: "epoch-1".to_string(),
                source_id: "source-1".to_string(),
                revision: 0,
                content_hash: "a".repeat(64),
                span: None,
            }],
            operation: "generate-chapter".to_string(),
            ui_action_nonce: "nonce-1".to_string(),
            frozen_input_hash: "a".repeat(64),
            budget: test_policy(),
            parent_root_action_id: None,
        };
        let run = repo.open(&conn, &request).unwrap();
        assert_eq!(run.binding.project_id, "project-1");
        assert_eq!(run.status, "running");
        assert!(!run.root_action_id.is_empty());
        assert!(!run.run_id.is_empty());
        // budget 读取
        let budget = repo.budget(&conn, &run.root_action_id).unwrap();
        assert_eq!(budget.root.status, "active");
        assert_eq!(budget.policy.max_physical_requests, 4);
        assert_eq!(budget.attempts.len(), 0);
        assert_eq!(budget.active_elapsed_ms, 0);
        // reserve
        let receipt = repo
            .reserve(
                &conn,
                &run.run_id,
                "nonce-1",
                &"b".repeat(64),
                100,
                10,
                None,
                None,
                None,
                None,
                false,
            )
            .unwrap();
        assert_eq!(receipt.attempt.status, "reserved");
        assert_eq!(receipt.attempt.reserved_tokens, 100);
        assert!(receipt.artifact.is_some());
        // find_invocation 重放
        let replayed = repo
            .find_invocation(&conn, &run.run_id, "nonce-1", &"b".repeat(64))
            .unwrap()
            .unwrap();
        assert_eq!(replayed.attempt.attempt_id, receipt.attempt.attempt_id);
        // mark_dispatched
        repo.mark_dispatched(&conn, &receipt.attempt.attempt_id)
            .unwrap();
        let view = repo.read_run_view(&conn, &run.run_id).unwrap();
        assert_eq!(view.attempts.len(), 1);
        assert_eq!(view.attempts[0].receipt.attempt.status, "dispatch-marked");
        // snapshot
        let artifact = repo
            .snapshot(&conn, &receipt.attempt.attempt_id, 0, "第一章正文")
            .unwrap();
        assert_eq!(artifact.revision, 1);
        assert_eq!(artifact.text, "第一章正文");
        assert_eq!(artifact.text_hash, text_hash("第一章正文"));
        // settle
        let usage = GenerationUsageReceipt {
            policy: ProviderUsagePolicy {
                estimator_version: "v1".to_string(),
                safety_margin_tokens: 10,
                reasoning: "unknown".to_string(),
                can_bound_total_liability: true,
            },
            input_tokens: Some(50),
            completion_tokens: Some(120),
            reasoning_tokens: None,
            actual_tokens: Some(150),
            trusted: true,
            agent_response: None,
        };
        let settled = repo
            .settle(
                &conn,
                &receipt.attempt.attempt_id,
                Some(usage),
                Some("stop".to_string()),
                None,
                None,
            )
            .unwrap();
        assert_eq!(settled.attempt.status, "settled");
        assert_eq!(settled.attempt.actual_tokens, Some(150));
        assert_eq!(
            settled.result.as_ref().unwrap().finish_reason.as_deref(),
            Some("stop")
        );
        // 候选列表
        let candidates = repo.list_candidates(&conn, Some(&run.run_id)).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].as_ref().unwrap().artifact_id,
            artifact.artifact_id
        );
        // discard
        repo.discard_candidate(&conn, &artifact.artifact_id)
            .unwrap();
        let candidates = repo.list_candidates(&conn, Some(&run.run_id)).unwrap();
        assert!(candidates.is_empty());
    }

    #[test]
    fn open_idempotency_test() {
        let repo = GenerationRunRepository::default();
        let conn = memory_db();
        let mut manifest = Map::new();
        manifest.insert("operation".to_string(), json!("generate-chapter"));
        let request = OpenGenerationRunRequest {
            project_id: "project-1".to_string(),
            epoch: "epoch-1".to_string(),
            fingerprint: test_fingerprint(),
            context_snapshot_id: "snapshot-1".to_string(),
            source_manifest: manifest.clone(),
            source_refs: vec![],
            operation: "generate-chapter".to_string(),
            ui_action_nonce: "nonce-1".to_string(),
            frozen_input_hash: "a".repeat(64),
            budget: test_policy(),
            parent_root_action_id: None,
        };
        let first = repo.open(&conn, &request).unwrap();
        let second = repo.open(&conn, &request).unwrap();
        assert_eq!(first.run_id, second.run_id);
        assert_eq!(first.root_action_id, second.root_action_id);
        // 绑定内容不同 → GENERATION_NONCE_CONFLICT
        let mut conflicting = request.clone();
        conflicting.context_snapshot_id = "snapshot-2".to_string();
        assert_eq!(
            repo.open(&conn, &conflicting).unwrap_err(),
            GENERATION_NONCE_CONFLICT
        );
    }

    #[test]
    fn reserve_replay_and_settle_transitions_test() {
        let repo = GenerationRunRepository::default();
        let conn = memory_db();
        let mut manifest = Map::new();
        manifest.insert("operation".to_string(), json!("generate-chapter"));
        let request = OpenGenerationRunRequest {
            project_id: "project-1".to_string(),
            epoch: "epoch-1".to_string(),
            fingerprint: test_fingerprint(),
            context_snapshot_id: "snapshot-1".to_string(),
            source_manifest: manifest,
            source_refs: vec![],
            operation: "generate-chapter".to_string(),
            ui_action_nonce: "nonce-1".to_string(),
            frozen_input_hash: "a".repeat(64),
            budget: test_policy(),
            parent_root_action_id: None,
        };
        let run = repo.open(&conn, &request).unwrap();
        let receipt = repo
            .reserve(
                &conn,
                &run.run_id,
                "nonce-1",
                &"b".repeat(64),
                100,
                10,
                None,
                None,
                None,
                None,
                false,
            )
            .unwrap();
        // 非法状态迁移：reserved → settled（未 dispatch）
        assert_eq!(
            repo.settle(&conn, &receipt.attempt.attempt_id, None, None, None, None)
                .unwrap_err(),
            INVALID_ATTEMPT_TRANSITION
        );
        // dispatch 后再结算
        repo.mark_dispatched(&conn, &receipt.attempt.attempt_id)
            .unwrap();
        let settled = repo
            .settle(
                &conn,
                &receipt.attempt.attempt_id,
                None,
                None,
                Some("error".to_string()),
                None,
            )
            .unwrap();
        assert_eq!(settled.attempt.status, "unknown");
        assert_eq!(settled.failure_code.as_deref(), Some("error"));
        assert!(settled.result.as_ref().unwrap().usage.is_none());
    }
}
