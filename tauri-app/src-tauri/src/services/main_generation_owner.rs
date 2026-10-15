//! main_generation_owner —— 主生成 owner（批次 B1-W3 骨架）。
//!
//! 事实源（上游 Electron 基线）：
//! - `electron/services/main-generation-owner.ts`（1401 行）—— owner 服务本体：
//!   `createMainGenerationOwner`（:108）每库一份，捕获 DB 句柄 / 会话 /
//!   模型执行租约；`assertCurrent`（:117-120）入口门禁；
//!   `suspendForProjectClose`（:1390-1399）项目关闭钩子。
//! - `electron/controllers/generation-controller.ts`（266 行）——
//!   per-database owner 缓存 `owners`（:37）+ `ownerSessions`（:38）、
//!   `authorizedOwner` 门控（:86-130）、`onProjectDatabaseBeforeClose`
//!   钩子（:82-85）、`generation:snapshot` / `generation:reasoning` 事件
//!   发射（:117-124）、register 的错误码映射（:160-162）。
//!
//! # 本次落地（骨架级，不接命令层）
//! 1. per-database owner 缓存 [`MainGenerationOwnerCache`]
//!    （`Mutex<HashMap>` 线程安全包装；键 = 归一化项目根路径，
//!    等价基线以 `Database` 对象身份为键——Tauri 侧一项目根 ⇔ 唯一项目库
//!    连接，见 `db/mod.rs` 头注）；
//! 2. [`MainGenerationOwnerCache::authorized_owner`] 门控（逐条对齐基线
//!    `authorizedOwner` 语义）；
//! 3. 错误码映射 [`map_generation_error`]（基线透传正则 + 兜底码）与
//!    MUTATING 前缀 [`mutating_error`]；
//! 4. [`MainGenerationOwner::suspend_for_project_close`] 钩子
//!    （签名 + 缓存清理）；
//! 5. 事件发射接线点 [`emit_generation_snapshot`] /
//!    [`emit_generation_reasoning`]（D-2 既定决策：广播 +
//!    客户端过滤；真实发射器在命令层——见「事件发射接线点」节）。
//!
//! # 刻意偏离（均有上游语义理由）
//! - **缓存键**：基线以 `better-sqlite3` 的 `Database` 对象身份为键；
//!   Tauri 侧 `Connection` 不可哈希，且项目库连接随
//!   `AppState::project_db` 单一生命周期管理，改用
//!   `security::normalized_project_path` 的项目根路径为键，语义等价。
//! - **会话身份**：基线把捕获会话放缓存条目（`entry.session`）并另建
//!   `ownerSessions` WeakMap；本骨架把捕获会话并入 owner（`session`
//!   字段），门禁比对读 `owner.session`，等价。
//! - **订阅者集合**：基线 `Set<WebContents>`（IPC 发送端）；Tauri 事件
//!   经 `tauri::Emitter` 按窗口标签寻址，骨架以 `HashSet<String>`
//!   （窗口标签）占位，B3 收口。
//! - **DB 句柄**：基线 owner 构造时捕获 DB 句柄；Tauri 仓储约定按调用
//!   传入 `&Connection`（`repositories/generation_run_repository.rs`
//!   头注），owner 只捕获会话身份，连接由命令层在 `project_access`
//!   门禁内传入。
//! - **会话再认证**：基线 `deps.assertCurrent()` 在每个 owner 方法入口
//!   重验会话与库身份；Tauri 侧由命令层在缓存访问前经
//!   `security::assert_current_project_context` 等价执行一次
//!   （持守卫期间项目库 / 会话态被同一把锁保护，结构上不会被并发切换）。

use std::path::PathBuf;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard};

use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use rusqlite::{Connection, OptionalExtension};

use crate::repositories::generation_run_repository::{
    ChapterRange, DurableGenerationRun, FrozenInputFingerprint, GenerationExecutionReceipt,
    GenerationRunRepository, GenerationTransportDiagnostics, MainGenerationRunHandle,
    PhysicalAttempt, PlotOutlineProgress, RootBudget, RunBinding, RunView,
};

use crate::security::{self, ProjectSessionContext};
use crate::services::agent_generation::{
    AdmittedWorkflow, AgentDomainToolReceipt, AgentGeneration, AgentGenerationRecovery, AgentHost,
    AgentToolAction, AgentToolClaim, AgentWorkflowRegistration,
};
use crate::services::character_proposal::{
    prove_character_proposal, read_legacy_roster_generation_proof, CharacterProposalService,
};
use crate::services::generation_draft_effects::{
    assert_generation_batch_intent, GenerationDraftEffects,
};
use crate::services::generation_source_binding::compare_generation_source_bindings;
use crate::services::legacy_roster_generation::{keys_subset, LegacyRosterGeneration};
use crate::services::legacy_roster_generation_context::read_legacy_roster_generation_context;
use crate::services::legacy_roster_source::{
    adopt_legacy_cards, read_legacy_roster_source, AdoptLegacyCardsRequest,
};
use crate::state::ActiveProject;
// ============================================================================
// 错误码（字符串值逐字对齐上游 throw 调用）
// ============================================================================

/// 基线 `authorizedOwner`：当前项目库未打开
/// （generation-controller.ts:89）。
///
/// 结构上不可达：Tauri 侧会话门禁（`security::assert_current_project_context`）
/// 已要求活跃项目存在；常量保留以对齐基线文案。
const GENERATION_DATABASE_NOT_READY: &str = "GENERATION_DATABASE_NOT_READY";
/// 基线 owner `assertCurrent`：owner 已随项目关闭挂起
/// （main-generation-owner.ts:118）。
const GENERATION_OWNER_CLOSED: &str = "GENERATION_OWNER_CLOSED";
/// 基线 owner `assertCurrent` 依赖：owner 存活期间项目库被替换
/// （generation-controller.ts:108）。
///
/// 结构上不可达：Tauri 侧项目库连接随 `AppState::project_db` 单一生命周期
/// 管理，且门禁已认证会话与活跃项目一致；常量保留以对齐基线文案。
#[allow(dead_code)]
const GENERATION_DATABASE_CHANGED: &str = "GENERATION_DATABASE_CHANGED";
/// 基线事务内辅助函数：当前库无 owner 缓存条目
/// （generation-controller.ts:43 等）。
#[allow(dead_code)]
const GENERATION_OWNER_REQUIRED: &str = "GENERATION_OWNER_REQUIRED";
/// 基线 register catch 兜底码（generation-controller.ts:161）。
const GENERATION_REQUEST_FAILED: &str = "GENERATION_REQUEST_FAILED";
/// 基线透传字面量（generation-controller.ts:160）。
const SOURCE_DRAFT_CHANGED: &str = "SOURCE_DRAFT_CHANGED";

/// 基线 `draftPreparationBinding` 请求键集白名单
/// （main-generation-owner.ts:324）。
const PREPARATION_ALLOWED_KEYS: [&str; 8] = [
    "chapterNumber",
    "modelId",
    "promptKeys",
    "skillStages",
    "authorInputs",
    "query",
    "selectedDraftIds",
    "batchId",
];
/// 基线 `preparations` LRU 容量上限
/// （main-generation-owner.ts:350：超限逐出最旧条目）。
const PREPARATION_CAPACITY: usize = 64;
/// 基线 `preparationProjection` 的 manifest 剔除键集
/// （main-generation-owner.ts:296；`excludeKnowledge`
/// 为真时追加 `knowledgeSnapshot`）。
const PREPARATION_MANIFEST_EXCLUDED: [&str; 4] = [
    "selectedDraftIds",
    "selectedFinalizedDraftIds",
    "materialDecision",
    "materialDecisionHash",
];
const PREPARATION_MANIFEST_EXCLUDED_WITH_KNOWLEDGE: [&str; 5] = [
    "selectedDraftIds",
    "selectedFinalizedDraftIds",
    "materialDecision",
    "materialDecisionHash",
    "knowledgeSnapshot",
];
/// 基线 `preparationProjection` 的 fingerprint 剔除键集
/// （main-generation-owner.ts:297）。
const PREPARATION_FINGERPRINT_EXCLUDED: [&str; 2] = ["contextSnapshotHash", "dependencyHash"];

/// 基线 `readMainGenerationPolicy` 的版本允许集（main-generation-plan.ts:139）。
const POLICY_VERSIONS: [&str; 3] = [
    "s07-planning-v1",
    "s07-capacity-v2",
    "s07-task-and-liability-v1",
];
/// 基线 `MAIN_GENERATION_POLICY`（main-generation-plan.ts:26-33）的 `budget` 键集。
const POLICY_BUDGET_KEYS: [&str; 4] = [
    "maxActiveElapsedMs",
    "maxOutputPerRequest",
    "maxPhysicalRequests",
    "maxTokenLiability",
];
/// 基线 `DRAFT_RECONCILE_PURPOSE`（src/shared/draft-reconciliation.ts）。
const DRAFT_RECONCILE_PURPOSE: &str = "chapter-draft-reconcile";
/// 基线 `DRAFT_SHORT_OUTLINE_PURPOSE`（src/shared/draft-short-outline.ts）。
const DRAFT_SHORT_OUTLINE_PURPOSE: &str = "chapter-draft-short-outline";
/// 基线 `DRAFT_VISIBLE_TEXT_VERSION`（src/shared/draft-visible-text.ts）。
const DRAFT_VISIBLE_TEXT_VERSION: &str = "draft-visible-v2";

/// 基线 `tokenLiability`（src/shared/generation-contract.ts:66-68）：
/// 取消前不计负债；已结算且有实际用量按实际值；其余按预留值。
fn token_liability(attempt: &PhysicalAttempt) -> i64 {
    if attempt.status == "cancelled-before-dispatch" {
        0
    } else if attempt.status == "settled" && attempt.actual_tokens.is_some() {
        attempt.actual_tokens.unwrap_or(0)
    } else {
        attempt.reserved_tokens
    }
}

/// 基线 `Date.now()`（`viewOf` 的 `deadlineAt` 计算，main-generation-owner.ts:287）。
fn default_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn owner_now_ms() -> u64 {
    default_now_ms().max(0) as u64
}

/// `Number.isSafeInteger` 等价物（JS 安全整数为 ±2^53-1；Rust i64 可能超出）。
/// 与 `repositories::generation_run_repository::is_safe_integer_value` 同义
/// （该函数为模块私有，此处就地实现）。
fn is_safe_integer_value(value: i64) -> bool {
    (-9007199254740991..=9007199254740991).contains(&value)
}

/// 基线 `handleOf(run)`（main-generation-owner.ts:125-126）。
fn handle_of(run: &DurableGenerationRun) -> Value {
    json!({
        "projectId": run.binding.project_id,
        "epoch": run.binding.epoch,
        "rootActionId": run.root_action_id,
        "runId": run.run_id,
    })
}

/// 基线 `isDraftVisibleTextVersion`（src/shared/draft-visible-text.ts）：
/// 仅 `draft-visible-v1` / `draft-visible-v2` 命中。
fn is_draft_visible_text_version(value: &str) -> bool {
    value == "draft-visible-v1" || value == DRAFT_VISIBLE_TEXT_VERSION
}

/// 基线 `planningRecord`（main-generation-plan.ts:58-61）：非对象（含数组）
/// → `GENERATION_PLANNING_INTENT_INVALID`。
fn planning_record(value: Option<&Value>) -> Result<&Map<String, Value>, String> {
    match value {
        Some(Value::Object(map)) => Ok(map),
        _ => Err("GENERATION_PLANNING_INTENT_INVALID".to_string()),
    }
}

/// 基线 `readMainGenerationPolicy`（main-generation-plan.ts:131-158）的读面
/// 投影：版本允许集 → `GENERATION_POLICY_UNSUPPORTED`；`budget` 键集与正整数
/// 约束 → `GENERATION_PLANNING_BUDGET_INVALID`；其余字段（`taskBudgetPolicy` /
/// `planning` 意图内部结构）由写面（`begin` / `execute`，B3）在写入前校验，
/// 读面不重复深校验——读面只消费 `version` 与 `budget` 两个字段
/// （`requireRun` / `rootPolicy` 的预算比对）。
fn read_main_generation_policy(value: Option<&Value>) -> Result<Value, String> {
    let record = planning_record(value)?;
    let version = record
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !POLICY_VERSIONS.contains(&version) {
        return Err("GENERATION_POLICY_UNSUPPORTED".to_string());
    }
    let budget = planning_record(record.get("budget"))?;
    if budget.len() != POLICY_BUDGET_KEYS.len()
        || !POLICY_BUDGET_KEYS.iter().all(|key| {
            budget
                .get(*key)
                .and_then(Value::as_i64)
                .is_some_and(|n| n > 0)
        })
    {
        return Err("GENERATION_PLANNING_BUDGET_INVALID".to_string());
    }
    Ok(Value::Object(record.clone()))
}

// ============================================================================
// 读面契约投影类型（serde camelCase，对齐 src/shared 的 TS 契约形状）
// ============================================================================

/// `MainGenerationSnapshot`（src/services/generation/generation-runtime.ts:76-88）。
/// `fingerprint` / `nonReplayable` 仅候选（`candidates`）携带——基线在
/// `snapshotOf` 结果上展开这两个字段（main-generation-owner.ts:251-252）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainGenerationSnapshotProjection {
    project_id: String,
    epoch: String,
    root_action_id: String,
    run_id: String,
    artifact_id: String,
    attempt_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostics: Option<GenerationTransportDiagnostics>,
    revision: i64,
    durable_revision: i64,
    text: String,
    text_hash: String,
    status: String,
    composition_eligible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    fingerprint: Option<FrozenInputFingerprint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    non_replayable: Option<bool>,
}

/// `MainGenerationRunView['budgetDiagnostics'][number]`
/// （src/services/generation/task-budget-planner.ts 的 `GenerationBudgetDiagnostic`）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationBudgetDiagnosticProjection {
    attempt_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    planner_version: Option<String>,
    requested_output_tokens: i64,
    reserved_tokens: i64,
    actual: Option<GenerationActualUsageProjection>,
    actual_state: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    failure_code: Option<String>,
    reasons: Vec<Value>,
}

/// 诊断的受信实际用量（`actual`，基线仅 `usage.trusted` 时输出）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationActualUsageProjection {
    input: Option<i64>,
    completion: Option<i64>,
    reasoning: Option<i64>,
    total: Option<i64>,
}

/// `GenerationSessionBudget`（src/services/generation/generation-harness.ts）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationSessionBudgetProjection {
    max_attempts: i64,
    max_requested_output_tokens: i64,
    max_requested_output_tokens_per_attempt: i64,
    deadline_at: i64,
}

/// `MainGenerationRunView['ledger']`（generation-runtime.ts:100）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MainGenerationLedgerProjection {
    policy: RootBudget,
    token_liability: i64,
    physical_requests: i64,
    active_elapsed_ms: i64,
    blocked_code: Option<String>,
}

/// `MainGenerationRunView`（src/services/generation/generation-runtime.ts:90-104）。
/// 基线为宽松对象（`plotOutline?` / `candidates?` 等均可缺省）；本投影按
/// `viewOf` 的实际输出建模——`plotOutline` 仅在存在时输出（`skip_serializing_if`），
/// 其余字段基线恒输出；未保存尾来自 owner 的运行服务，并按根账本尝试身份过滤。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MainGenerationRunViewProjection {
    handle: Value,
    status: String,
    operation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    plot_outline: Option<PlotOutlineProgress>,
    budget_diagnostics: Vec<GenerationBudgetDiagnosticProjection>,
    non_replayable: bool,
    budget: GenerationSessionBudgetProjection,
    ledger: MainGenerationLedgerProjection,
    artifacts: Vec<MainGenerationSnapshotProjection>,
    candidates: Vec<MainGenerationSnapshotProjection>,
    unsaved_tails: Vec<Value>,
}

/// `attemptPurpose(attemptId)`（main-generation-owner.ts:165-169）：
/// 读 `generation_attempts.usage_receipt_json.purpose`，缺失 → `"unknown"`。
fn attempt_purpose(conn: &Connection, attempt_id: &str) -> Result<String, String> {
    let row: Option<String> = conn
        .query_row(
            "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id=?1",
            [attempt_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
    let Some(row) = row else {
        return Ok("unknown".to_string());
    };
    let purpose = serde_json::from_str::<Value>(&row).ok().and_then(|value| {
        value
            .get("purpose")
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    Ok(purpose.unwrap_or_else(|| "unknown".to_string()))
}

/// 基线 `snapshotOf` 的制品存活性校验（main-generation-owner.ts:235）：
/// 无 `visiblePurpose`（`artifacts` 面）时要求制品未被丢弃
/// （`generation_artifacts.status <> 'discarded'`）。
fn artifact_is_kept(conn: &Connection, artifact_id: &str) -> Result<bool, String> {
    let kept: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM generation_artifacts WHERE artifact_id=?1 AND status<>'discarded'",
            [artifact_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
    Ok(kept.is_some())
}
/// 基线 register catch 的透传正则（generation-controller.ts:160）：
/// `^(?:GENERATION|ROOT_BUDGET|ARTIFACT|MAIN|CHARACTER|FINALIZED_CHARACTER|LEGACY_ROSTER)_[A-Z_]+$`
static OWNER_ERROR_CODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:GENERATION|ROOT_BUDGET|ARTIFACT|MAIN|CHARACTER|FINALIZED_CHARACTER|LEGACY_ROSTER)_[A-Z_]+$",
    )
    .expect("owner error code pattern is a valid regex")
});

/// 对齐基线 register catch 的错误码映射（generation-controller.ts:160-162）：
///
/// - [`SOURCE_DRAFT_CHANGED`] 字面量，或匹配 [`OWNER_ERROR_CODE`] 正则的
///   错误码 → **原样透传**；
/// - 其余（含会话守卫的中文文案）→ 一律收敛为 [`GENERATION_REQUEST_FAILED`]。
pub fn map_generation_error(message: &str) -> &str {
    if message == SOURCE_DRAFT_CHANGED || OWNER_ERROR_CODE.is_match(message) {
        message
    } else {
        GENERATION_REQUEST_FAILED
    }
}

/// MUTATING 失败信封前缀（复刻 `commands::db::mutating_error`，保持两栈
/// 前端展示文案逐字一致：基线渲染层把 `throw new Error(code)` 序列化为
/// `Error: <code>`）。
pub fn mutating_error(code: &str) -> String {
    format!("Error: {code}")
}

// ============================================================================
// 主生成 owner
// ============================================================================

/// 基线 `preparations` 缓存条目
/// （`{ binding: RunBinding; knowledgeSnapshot:
/// GenerationKnowledgeSnapshot }`，main-generation-
/// owner.ts:115；`knowledgeSnapshot` 以全保真 `Value`
/// 存储，语义见 [`MainGenerationOwner::preparations`]）。
struct PreparationEntry {
    /// 基线 binding 随 `deps.buildBinding` 持久化
    /// （main-generation-owner.ts:334）——源绑定构造
    /// 依赖未迁移（见 [`MainGenerationOwner::
    /// draft_preparation_binding`] 头注）；基线同样
    /// 只写不读（`preparedKnowledge` 仅返回
    /// `knowledgeSnapshot`），字段保留为缓存条目
    /// 形状锚点，待依赖迁移后接入读取面。
    #[allow(dead_code)]
    binding: RunBinding,
    knowledge_snapshot: Value,
}

struct PendingOwnerInvocation {
    request_hash: String,
    holders: usize,
}

struct PendingOwnerInvocationGuard {
    pending: Arc<Mutex<HashMap<String, PendingOwnerInvocation>>>,
    key: String,
    request_hash: String,
}

impl Drop for PendingOwnerInvocationGuard {
    fn drop(&mut self) {
        // 析构不抛错；只清理自身哈希的登记，不能撤销其他准备对象。
        if let Ok(mut pending) = self.pending.lock() {
            if let Some(entry) = pending.get_mut(&self.key) {
                if entry.request_hash == self.request_hash {
                    entry.holders -= 1;
                    if entry.holders == 0 {
                        pending.remove(&self.key);
                    }
                }
            }
        }
    }
}

enum PreparedExecutionPlan {
    Replay(Value),
    Dispatch(Box<crate::services::generation_run_service::ExecuteGenerationRequest>),
}

/// 准备阶段冻结任务、模型请求与服务；不借用 owner 或数据库连接。
/// 生产短回调与兼容直接连接路径共用冻结计划。
pub(crate) struct PreparedMainGenerationExecution {
    task: Value,
    plan: PreparedExecutionPlan,
    service: Arc<crate::services::generation_run_service::GenerationRunService>,
    _pending: PendingOwnerInvocationGuard,
}

pub(crate) enum PreparedMainGenerationResult {
    Replay(Value),
    Receipt(Box<GenerationExecutionReceipt>),
}

impl PreparedMainGenerationExecution {
    pub(crate) fn execute_blocking(
        &self,
        conn: &Connection,
    ) -> Result<PreparedMainGenerationResult, String> {
        self.execute_blocking_with_database(conn)
    }

    pub(crate) fn execute_blocking_with_database<
        D: crate::services::generation_run_service::GenerationDatabaseAccess,
    >(
        &self,
        database: &D,
    ) -> Result<PreparedMainGenerationResult, String> {
        match &self.plan {
            PreparedExecutionPlan::Replay(outcome) => {
                Ok(PreparedMainGenerationResult::Replay(outcome.clone()))
            }
            PreparedExecutionPlan::Dispatch(request) => self
                .service
                .execute_blocking_with_database(database, request.as_ref().clone())
                .map(|receipt| PreparedMainGenerationResult::Receipt(Box::new(receipt)))
                .map_err(|error| mutating_error(map_generation_error(&error))),
        }
    }
}

/// 主生成 owner（对齐基线 `createMainGenerationOwner` 的闭包状态，
/// main-generation-owner.ts:108-1399）。
///
/// 骨架阶段仅持有会话身份与生成账本仓储（W1）；生成编排方法
/// （`begin` / `execute` / `list` / …）随 B3 批次平移，方法体以
/// `todo!()` 占位。
pub struct MainGenerationOwner {
    /// 基线 `Object.freeze({ ...session })` 捕获的会话身份
    /// （generation-controller.ts:97）——项目生命周期内不变，
    /// 门禁比对（`projectId` / `leaseId`）与 `ownerSessions`
    /// WeakMap 读取均使用它。
    session: ProjectSessionContext,

    /// 捕获 Lorekeeper 全局根，模型与全局提示词在同一数据边界读取。
    global_data_root: PathBuf,

    /// 生成账本仓储（W1；`repositories/generation_run_repository.rs`）。
    repository: GenerationRunRepository,
    /// 基线 `runLeases: Map<string, string>`（attemptId → leaseId，
    /// main-generation-owner.ts:112）。挂起时逐 lease 关闭并清空。
    run_leases: Mutex<HashMap<String, String>>,
    /// owner 在途哈希登记；共享所有权使准备对象脱离 owner 守卫后仍有效。
    /// 同键同哈希允许服务层合并重放，最后一个准备守卫退出时才撤销登记。
    pending_invocations: Arc<Mutex<HashMap<String, PendingOwnerInvocation>>>,
    /// 与 owner 同寿命的服务与执行租约，不能在每次 IPC 中重建。
    lease_store: Arc<Mutex<crate::llm::lease::LlmLeaseStore>>,
    run_service: Arc<crate::services::generation_run_service::GenerationRunService>,
    /// 启动恢复只在本 owner 成功的独立事务中执行一次。
    agent_recovery_done: AtomicBool,
    /// 基线 `preparations: Map<string, { binding: RunBinding;
    /// knowledgeSnapshot: GenerationKnowledgeSnapshot }>`
    /// （main-generation-owner.ts:115）——草稿上下文准备
    /// 缓存（`prepareDraftContext` 签发，`preparedKnowledge`
    /// 读取，容量 64 的 LRU：超限逐出最旧条目，:350；
    /// 项目关闭挂起时清空，:1391）。
    ///
    /// 以 `Vec` 保序等价基线 `Map` 插入序——基线
    /// 「超限删最旧」依赖插入序（Rust `HashMap` 无序，
    /// 无法实现逐出最旧语义）；容量上限 64，线性
    /// 查找无性能顾虑。`knowledgeSnapshot` 以全保真
    /// `Value` 存储（基线为 `GenerationKnowledgeSnapshot`
    /// 强类型；IPC 边界为 `Value`，基线 `structuredClone`
    /// 的等价物为 `Value::clone` 深拷贝）。
    preparations: Vec<(String, PreparationEntry)>,
    /// 不依赖业务锁的事件出口与当前 owner 生命周期围栏。
    events: Arc<OwnerEventDelivery>,
    /// 基线 `closed` 标志（main-generation-owner.ts:116）。
    closed: bool,
}

/// 原生层事件出口；载荷保持现有契约，services 不依赖 Tauri。
#[derive(Clone)]
pub(crate) enum MainGenerationEvent {
    Snapshot(Box<MainGenerationSnapshotProjection>),
    Reasoning(MainGenerationReasoningEventProjection),
}

pub(crate) type MainGenerationEventSink = Arc<dyn Fn(MainGenerationEvent) + Send + Sync>;
type SharedGenerationEventSink = Arc<Mutex<Option<MainGenerationEventSink>>>;

struct OwnerEventDelivery {
    session: ProjectSessionContext,
    active: AtomicBool,
    sink: SharedGenerationEventSink,
}

impl OwnerEventDelivery {
    fn is_listening(&self) -> bool {
        self.active.load(Ordering::Acquire)
            && self.sink.lock().map(|sink| sink.is_some()).unwrap_or(false)
    }

    fn publish(&self, event: MainGenerationEvent) {
        let (project_id, epoch) = match &event {
            MainGenerationEvent::Snapshot(snapshot) => (&snapshot.project_id, &snapshot.epoch),
            MainGenerationEvent::Reasoning(reasoning) => (&reasoning.project_id, &reasoning.epoch),
        };
        if project_id != &self.session.project_id
            || epoch != &self.session.lease_id
            || !self.active.load(Ordering::Acquire)
        {
            return;
        }
        let sink = self.sink.lock().ok().and_then(|sink| sink.clone());
        if let Some(sink) = sink {
            // 出口异常不能改变账本；调用时不持出口锁，允许安全替换出口。
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(event)));
        }
    }

    fn close(&self) {
        self.active.store(false, Ordering::Release);
    }
}

/// 基线 `graphOperations`（main-generation-owner.ts:55）：图生成操作集
/// （`plot` / `plan` / `event` 三类图生成操作的准入与子代禁令判定）。
const GRAPH_OPERATIONS: [&str; 3] = [
    "plot-tree-snapshot",
    "narrative-thread-plan-candidate",
    "narrative-thread-event-candidate",
];

/// 基线 `begin` 选择键集白名单（main-generation-owner.ts:358-363，
/// IPC 边界六个初始化标志恒为 false——对齐 generation-controller.ts:173-178
/// 的注册形态，故不含 `agentInput` / `agentSession` / `editorInlineInput` /
/// `finalizationGenerationSlot` / `graphGenerationInput` / `graphGenerationKey` /
/// `legacyRosterKey` 七个仅主进程内部初始化调用可携带的键）。
const BEGIN_ALLOWED_KEYS: [&str; 23] = [
    "operation",
    "uiActionNonce",
    "modelId",
    "chapterNumber",
    "selectedDraftIds",
    "selectedFinalizedDraftIds",
    "selectedBlueprintChapterNumbers",
    "promptKeys",
    "skillStages",
    "authorInputs",
    "output",
    "outputOverrides",
    "parentRootActionId",
    "continueDirectoryOperationId",
    "batchId",
    "batchIntent",
    "preparationId",
    "finalizedCharacterContextId",
    "reviewRevisionContextId",
    "agentWorkflowRegistrationId",
    "materialDecision",
    "importSlot",
    "importExecution",
];

impl MainGenerationOwner {
    /// 基线 owner 构造的骨架等价物（generation-controller.ts:106-125 的
    /// `createMainGenerationOwner({...})` 调用，裁剪到当前已迁移的依赖面：
    /// 仅会话身份与生成账本仓储；`leases` / `loadModel` / `buildBinding` /
    /// `onSnapshot` / `onReasoning` 等依赖随对应批次接入）。
    #[cfg(test)]
    fn new(session: ProjectSessionContext) -> Self {
        Self::with_lease_store(
            session,
            Arc::new(Mutex::new(crate::llm::lease::LlmLeaseStore::new())),
        )
    }

    #[cfg(test)]
    fn with_lease_store(
        session: ProjectSessionContext,
        lease_store: Arc<Mutex<crate::llm::lease::LlmLeaseStore>>,
    ) -> Self {
        Self::with_event_sink(session, lease_store, Arc::new(Mutex::new(None)))
    }

    fn with_event_sink(
        session: ProjectSessionContext,
        lease_store: Arc<Mutex<crate::llm::lease::LlmLeaseStore>>,
        sink: SharedGenerationEventSink,
    ) -> Self {
        let events = Arc::new(OwnerEventDelivery {
            session: session.clone(),
            active: AtomicBool::new(true),
            sink,
        });
        let global_data_root = crate::app_paths::lorekeeper_home();
        let repository = GenerationRunRepository::default();
        let run_service = Arc::new(crate::services::generation_run_service::create_generation_run_service(
            Self::dependencies_with_events(
                repository.clone(),
                events.clone(),
                Arc::new(crate::services::main_generation_dispatcher::ConfiguredMainGenerationDispatcher::new(global_data_root.clone())),
            ),
        ));
        Self {
            global_data_root,
            lease_store,
            run_service,
            agent_recovery_done: AtomicBool::new(false),
            events,
            session,
            repository,
            run_leases: Mutex::new(HashMap::new()),
            pending_invocations: Arc::new(Mutex::new(HashMap::new())),
            preparations: Vec::new(),
            closed: false,
        }
    }

    fn dependencies_with_events(
        repository: GenerationRunRepository,
        events: Arc<OwnerEventDelivery>,
        dispatch: Arc<dyn crate::services::generation_run_service::GenerationDispatcher>,
    ) -> crate::services::generation_run_service::GenerationRunServiceDependencies {
        let snapshot_repository = repository.clone();
        let snapshots = events.clone();
        crate::services::generation_run_service::GenerationRunServiceDependencies {
            repository,
            dispatch,
            validate_recovery: None,
            on_snapshot: None,
            on_durable_snapshot: Some(Arc::new(move |conn, event| {
                if event.storage_failed || !snapshots.is_listening() {
                    return;
                }
                // 使用服务正在借用的同一连接，不重新进入业务门禁。
                if let Ok(receipt) = snapshot_repository.receipt(conn, &event.attempt_id, None) {
                    if let Ok(Some(mut snapshot)) = Self::project_snapshot(conn, &receipt, None) {
                        snapshot.diagnostics = event.diagnostics.or(snapshot.diagnostics);
                        snapshots.publish(MainGenerationEvent::Snapshot(Box::new(snapshot)));
                    }
                }
            })),
            on_reasoning: Some(Arc::new(move |event| {
                events.publish(MainGenerationEvent::Reasoning(
                    MainGenerationReasoningEventProjection {
                        project_id: events.session.project_id.clone(),
                        epoch: events.session.lease_id.clone(),
                        root_action_id: event.root_action_id,
                        run_id: event.run_id,
                        attempt_id: event.attempt_id,
                        text: event.text,
                    },
                ));
            })),
        }
    }

    /// 用实时模型证据重建源；只读收据不进入可执行租约注册表。
    fn rebuild_binding(
        &self,
        conn: &Connection,
        previous: &RunBinding,
    ) -> Result<RunBinding, String> {
        self.assert_current()?;
        crate::services::main_generation_sources::rebuild_binding(
            conn,
            &PathBuf::from(&self.session.project_path).join(".lore"),
            &self.global_data_root,
            &self.session.lease_id,
            previous,
        )
        .map_err(|error| mutating_error(map_generation_error(&error)))
    }

    fn build_binding(
        &self,
        conn: &Connection,
        selection: &Value,
        receipt: &crate::services::generation_source_binding::SafeGenerationModelReceipt,
        policy: &Value,
        output_contract: &Value,
    ) -> Result<RunBinding, String> {
        self.assert_current()?;
        let mut input = selection.clone();
        input["projectId"] = json!(self.session.project_id);
        input["epoch"] = json!(self.session.lease_id);
        input["modelReceipt"] = serde_json::to_value(receipt).expect("安全模型收据可序列化");
        input["policy"] = policy.clone();
        input["outputContract"] = output_contract.clone();
        let input: crate::services::generation_source_binding::GenerationSourceBindingInput =
            serde_json::from_value(input)
                .map_err(|_| mutating_error("GENERATION_SOURCE_BINDING_INVALID"))?;
        crate::services::main_generation_sources::with_source_dependencies(
            conn,
            &PathBuf::from(&self.session.project_path).join(".lore"),
            &self.global_data_root,
            |deps| {
                Ok(
                    crate::services::generation_source_binding::build_generation_source_binding(
                        deps, &input,
                    )?
                    .binding,
                )
            },
        )
        .map_err(|error| mutating_error(map_generation_error(&error)))
    }

    fn assert_binding_current(
        &self,
        conn: &Connection,
        previous: &RunBinding,
    ) -> Result<(), String> {
        if self.rebuild_binding(conn, previous)? != *previous {
            return Err(mutating_error("GENERATION_SOURCE_CHANGED"));
        }
        Ok(())
    }

    /// 基线 `handleOf(run)` 的句柄比对等价物（main-generation-owner.ts:145）：
    /// 运行的项目归属 + 句柄四元组（`projectId` / `epoch` / `rootActionId` /
    /// `runId`）必须与入参逐字段一致，否则 `GENERATION_RUN_IDENTITY_MISMATCH`。
    pub(crate) fn require_run(
        &self,
        conn: &Connection,
        handle: &Value,
        execution: bool,
    ) -> Result<DurableGenerationRun, String> {
        self.assert_current()?;
        let run_id = handle
            .get("runId")
            .and_then(Value::as_str)
            .ok_or_else(|| mutating_error("GENERATION_RUN_IDENTITY_MISMATCH"))?;
        let run = self
            .repository
            .get(conn, run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        if run.binding.project_id != self.session.project_id
            || handle_of(&run).as_object() != handle.as_object()
        {
            return Err(mutating_error("GENERATION_RUN_IDENTITY_MISMATCH"));
        }
        if execution {
            self.repository
                .assert_plot_outline_writable(conn, &run.run_id)
                .map_err(|error| mutating_error(map_generation_error(&error)))?;
            if run.binding.epoch != self.session.lease_id {
                return Err(mutating_error("GENERATION_EPOCH_STALE"));
            }
        }
        if self
            .repository
            .is_private_planning_history(conn, &run.run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?
        {
            return Ok(run);
        }
        let policy = read_main_generation_policy(run.binding.source_manifest.get("policy"))
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        if policy.get("version").and_then(Value::as_str) != Some("s07-task-and-liability-v1")
            && policy.get("budget")
                != Some(
                    &self
                        .root_policy(conn, &run.root_action_id)?
                        .get("budget")
                        .cloned()
                        .unwrap_or(Value::Null),
                )
        {
            return Err(mutating_error("GENERATION_POLICY_BUDGET_MISMATCH"));
        }
        Ok(run)
    }

    /// 基线 `rootPolicy(rootActionId)`（main-generation-owner.ts:126-137）：
    /// 取根下首 run（`ORDER BY rowid LIMIT 1`）的策略并与当前根账本预算比对。
    fn root_policy(&self, conn: &Connection, root_action_id: &str) -> Result<Value, String> {
        let run_id: Option<String> = conn
            .query_row(
                "SELECT run_id FROM generation_runs WHERE root_action_id=?1 ORDER BY rowid LIMIT 1",
                [root_action_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
        let Some(run_id) = run_id else {
            return Err(mutating_error("GENERATION_ROOT_MISSING"));
        };
        let run = self
            .repository
            .get(conn, &run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        let policy = read_main_generation_policy(run.binding.source_manifest.get("policy"))
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        if policy.get("version").and_then(Value::as_str) != Some("s07-task-and-liability-v1") {
            let budget = self
                .repository
                .budget(conn, root_action_id)
                .map_err(|error| mutating_error(map_generation_error(&error)))?
                .policy;
            if policy.get("budget") != Some(&serde_json::to_value(budget).unwrap_or(Value::Null)) {
                return Err(mutating_error("GENERATION_POLICY_BUDGET_MISMATCH"));
            }
        }
        Ok(policy)
    }

    /// 基线 `snapshotOf(receipt, visiblePurpose?)`（main-generation-owner.ts:224-241）：
    /// 执行回执 → 运行快照（`MainGenerationSnapshot`）。`visible_purpose` 为
    /// `Some` 时（候选面）跳过制品存活性 SQL 校验；`None` 时（`artifacts` 面）
    /// 要求制品未被丢弃。`fingerprint` / `non_replayable` 由调用方按候选面补齐。
    fn snapshot_of(
        &self,
        conn: &Connection,
        receipt: &GenerationExecutionReceipt,
        visible_purpose: Option<&str>,
    ) -> Result<Option<MainGenerationSnapshotProjection>, String> {
        Self::project_snapshot(conn, receipt, visible_purpose)
    }

    fn project_snapshot(
        conn: &Connection,
        receipt: &GenerationExecutionReceipt,
        visible_purpose: Option<&str>,
    ) -> Result<Option<MainGenerationSnapshotProjection>, String> {
        let Some(artifact) = receipt.artifact.as_ref() else {
            return Ok(None);
        };
        let attempt = &receipt.attempt;
        let terminal = matches!(
            attempt.status.as_str(),
            "settled" | "unknown" | "cancelled-before-dispatch"
        );
        let status = if !terminal {
            "running"
        } else if receipt.failure_code.is_some() {
            "failed"
        } else if receipt
            .result
            .as_ref()
            .and_then(|result| result.finish_reason.as_deref())
            == Some("stop")
        {
            "completed"
        } else if attempt.status == "cancelled-before-dispatch"
            || receipt.budget.root.status == "cancelled"
        {
            "cancelled"
        } else if receipt
            .result
            .as_ref()
            .and_then(|result| result.finish_reason.as_deref())
            .is_some()
        {
            "failed"
        } else {
            "unknown"
        };
        let purpose = visible_purpose
            .map(str::to_string)
            .map(Ok)
            .unwrap_or_else(|| attempt_purpose(conn, &attempt.attempt_id))?;
        let composition_eligible = matches!(attempt.status.as_str(), "settled" | "unknown")
            && receipt.failure_code.is_none()
            && receipt.budget.root.status != "cancelled"
            && matches!(
                receipt
                    .result
                    .as_ref()
                    .and_then(|result| result.finish_reason.as_deref()),
                Some("stop") | Some("length")
            )
            && !artifact.text.trim().is_empty()
            && (visible_purpose.is_some() || artifact_is_kept(conn, &artifact.artifact_id)?)
            && purpose != DRAFT_RECONCILE_PURPOSE
            && purpose != DRAFT_SHORT_OUTLINE_PURPOSE;
        Ok(Some(MainGenerationSnapshotProjection {
            project_id: receipt.run.binding.project_id.clone(),
            epoch: artifact.epoch.clone(),
            root_action_id: receipt.run.root_action_id.clone(),
            run_id: receipt.run.run_id.clone(),
            artifact_id: artifact.artifact_id.clone(),
            attempt_id: artifact.attempt_id.clone(),
            diagnostics: receipt.diagnostics.clone(),
            revision: artifact.revision,
            durable_revision: artifact.revision,
            text: artifact.text.clone(),
            text_hash: artifact.text_hash.clone(),
            status: status.to_string(),
            composition_eligible,
            fingerprint: None,
            non_replayable: None,
        }))
    }

    /// 基线 `viewOf(run)`（main-generation-owner.ts:256-290）：持久账本 →
    /// 只读运行视图（`MainGenerationRunView`）。易失性回执（基线
    /// `volatileReceipts`）属执行编排面（`execute`，B3），读面恒用持久回执。
    fn view_of(&self, conn: &Connection, run: &DurableGenerationRun) -> Result<Value, String> {
        let RunView {
            budget,
            plot_outline,
            attempts,
        } = self
            .repository
            .read_run_view(conn, &run.run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        // 基线 `visibleIds`：候选 attempt 的制品 ID → 可见用途（:262）。
        let visible_purposes: HashMap<String, String> = attempts
            .iter()
            .filter(|item| item.candidate)
            .filter_map(|item| {
                item.receipt
                    .artifact
                    .as_ref()
                    .map(|artifact| (artifact.artifact_id.clone(), String::new()))
            })
            .collect();
        let mut candidates: Vec<MainGenerationSnapshotProjection> = Vec::new();
        let mut artifacts: Vec<MainGenerationSnapshotProjection> = Vec::new();
        let mut budget_diagnostics: Vec<GenerationBudgetDiagnosticProjection> =
            Vec::with_capacity(attempts.len());
        for item in &attempts {
            let receipt = &item.receipt;
            let visible_purpose = receipt
                .artifact
                .as_ref()
                .and_then(|artifact| visible_purposes.get(&artifact.artifact_id))
                .map(|purpose| purpose.as_str());
            if receipt.run.run_id == run.run_id
                && receipt.artifact.is_some()
                && visible_purpose.is_some()
            {
                if let Some(mut snapshot) = self.snapshot_of(conn, receipt, visible_purpose)? {
                    // 基线候选面展开 `{ ...snapshotOf(...), fingerprint, nonReplayable: true }`
                    // （main-generation-owner.ts:251-252）。
                    snapshot.fingerprint =
                        Some(receipt.artifact.as_ref().unwrap().fingerprint.clone());
                    snapshot.non_replayable = Some(true);
                    if snapshot.epoch == run.binding.epoch {
                        artifacts.push(snapshot.clone());
                    }
                    candidates.push(snapshot);
                }
            }
            let usage = receipt
                .result
                .as_ref()
                .and_then(|result| result.usage.as_ref());
            budget_diagnostics.push(GenerationBudgetDiagnosticProjection {
                attempt_id: receipt.attempt.attempt_id.clone(),
                planner_version: receipt
                    .budget_decision
                    .as_ref()
                    .and_then(|decision| decision.get("policyVersion"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                requested_output_tokens: receipt.attempt.requested_output_tokens,
                reserved_tokens: receipt.attempt.reserved_tokens,
                actual: usage.filter(|usage| usage.trusted).map(|usage| {
                    GenerationActualUsageProjection {
                        input: usage.input_tokens,
                        completion: usage.completion_tokens,
                        reasoning: usage.reasoning_tokens,
                        total: usage.actual_tokens,
                    }
                }),
                actual_state: if receipt.attempt.status == "cancelled-before-dispatch" {
                    "not-dispatched"
                } else if receipt.attempt.status == "settled" {
                    "settled"
                } else if matches!(
                    receipt.attempt.status.as_str(),
                    "reserved" | "dispatch-marked"
                ) {
                    "reserved"
                } else {
                    "unknown"
                }
                .to_string(),
                finish_reason: receipt
                    .result
                    .as_ref()
                    .and_then(|result| result.finish_reason.clone()),
                failure_code: receipt
                    .failure_code
                    .clone()
                    .or_else(|| budget.blocked_code.clone()),
                reasons: receipt
                    .budget_decision
                    .as_ref()
                    .and_then(|decision| decision.get("reasons"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            });
        }
        let plot_outline_author_edit = match plot_outline {
            Some(_) => self
                .repository
                .read_plot_outline_author_edit(conn, &run.run_id)
                .map(|edit| edit.is_some())
                .map_err(|error| mutating_error(map_generation_error(&error)))?,
            None => false,
        };
        let non_replayable = run.binding.epoch != self.session.lease_id
            || run.status != "running"
            || budget.blocked_code.is_some()
            || self
                .repository
                .is_private_planning_history(conn, &run.run_id)
                .map_err(|error| mutating_error(map_generation_error(&error)))?
            || self
                .repository
                .read_planning_continuation(conn, &run.run_id)
                .map_err(|error| mutating_error(map_generation_error(&error)))?
                .is_some()
            || plot_outline_author_edit;
        let token_liability: i64 = budget.attempts.iter().map(token_liability).sum();
        let physical_requests: i64 = budget
            .attempts
            .iter()
            .filter(|attempt| attempt.status != "cancelled-before-dispatch")
            .count() as i64;
        let view = MainGenerationRunViewProjection {
            handle: handle_of(run),
            status: run.status.clone(),
            operation: run
                .binding
                .source_manifest
                .get("operation")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            plot_outline,
            budget_diagnostics,
            non_replayable,
            budget: GenerationSessionBudgetProjection {
                max_attempts: budget.policy.max_physical_requests,
                max_requested_output_tokens: budget.policy.max_token_liability,
                max_requested_output_tokens_per_attempt: budget.policy.max_output_per_request,
                deadline_at: default_now_ms()
                    + std::cmp::max(
                        0,
                        budget.policy.max_active_elapsed_ms - budget.active_elapsed_ms,
                    ),
            },
            ledger: MainGenerationLedgerProjection {
                policy: budget.policy.clone(),
                token_liability,
                physical_requests,
                active_elapsed_ms: budget.active_elapsed_ms,
                blocked_code: budget.blocked_code.clone(),
            },
            artifacts,
            candidates,
            unsaved_tails: self
                .run_service
                .read_unsaved_tails()
                .into_iter()
                .filter(|tail| {
                    budget
                        .attempts
                        .iter()
                        .any(|attempt| attempt.attempt_id == tail.attempt_id)
                })
                .map(serde_json::to_value)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?,
        };
        serde_json::to_value(view)
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))
    }

    /// 基线 `read(handle)`（main-generation-owner.ts:1315）：
    /// `generation:read` 频道——句柄 → 运行视图。
    pub fn read_run(&self, conn: &Connection, handle: &Value) -> Result<Value, String> {
        let run = self.require_run(conn, handle, false)?;
        self.view_of(conn, &run)
    }

    /// 基线 `list()`（main-generation-owner.ts:292-296）：`generation:list`
    /// 频道——按创建时间倒序（`created_at_ms DESC, rowid DESC`）列出当前
    /// 项目的全部运行视图。
    pub fn list(&self, conn: &Connection) -> Result<Value, String> {
        self.assert_current()?;
        let mut statement = conn
            .prepare("SELECT run_id FROM generation_runs ORDER BY created_at_ms DESC, rowid DESC")
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
        let run_ids: Vec<String> = statement
            .query_map([], |row| row.get(0))
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?
            .collect::<Result<_, _>>()
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
        let views: Vec<Value> = run_ids
            .iter()
            .map(|run_id| self.repository.get(conn, run_id))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| mutating_error(map_generation_error(&error)))?
            .into_iter()
            .filter(|run| run.binding.project_id == self.session.project_id)
            .map(|run| self.view_of(conn, &run))
            .collect::<Result<_, _>>()?;
        Ok(Value::Array(views))
    }

    /// 基线 `listDirectoryProgress()`（main-generation-owner.ts:1328-1331）：
    /// `generation:list-directory-progress` 频道——目录生成进度清单，
    /// 每条附源运行 `sourceManifest.authorInputs`（基线 `structuredClone`）。
    pub fn list_directory_progress(&self, conn: &Connection) -> Result<Value, String> {
        self.assert_current()?;
        let progress = self
            .repository
            .list_directory_progress(conn)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        let enriched: Vec<Value> = progress
            .iter()
            .map(|item| -> Result<Value, String> {
                let run = self
                    .repository
                    .get(conn, &item.source_handle.run_id)
                    .map_err(|error| mutating_error(map_generation_error(&error)))?;
                let mut value = serde_json::to_value(item)
                    .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
                // 基线 `{ ...progress, authorInputs: structuredClone(...) }`：
                // 仓储结构体的 `authorInputs` 恒为 `None`（读面不消费），
                // 这里按源运行 manifest 的实际值覆写（缺失 → 键删除，等价
                // 基线 `undefined` 在 JSON 序列化时缺省）。
                let author_inputs = run
                    .binding
                    .source_manifest
                    .get("authorInputs")
                    .cloned()
                    .filter(|value| !value.is_null());
                let object = value
                    .as_object_mut()
                    .expect("directory progress projection serializes to an object");
                match author_inputs {
                    Some(inputs) => {
                        object.insert("authorInputs".to_string(), inputs);
                    }
                    None => {
                        object.remove("authorInputs");
                    }
                }
                Ok(value)
            })
            .collect::<Result<_, _>>()?;
        Ok(Value::Array(enriched))
    }

    /// 基线 `readVisibleComposition(handle)`（main-generation-owner.ts:1332-1336）：
    /// `generation:read-visible-composition` 频道——可见合成回执（含源运行
    /// `authorInputs`），无合成记录 → `null`。
    pub fn read_visible_composition(
        &self,
        conn: &Connection,
        handle: &Value,
    ) -> Result<Value, String> {
        let run = self.require_run(conn, handle, false)?;
        let composition = self
            .repository
            .read_visible_composition(conn, &run.run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        let Some(composition) = composition else {
            return Ok(Value::Null);
        };
        let mut value = serde_json::to_value(composition)
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
        // 基线 `{ ...composition, authorInputs: structuredClone(...) }`（同上）。
        let author_inputs = run
            .binding
            .source_manifest
            .get("authorInputs")
            .cloned()
            .filter(|value| !value.is_null());
        let object = value
            .as_object_mut()
            .expect("visible composition projection serializes to an object");
        match author_inputs {
            Some(inputs) => {
                object.insert("authorInputs".to_string(), inputs);
            }
            None => {
                object.remove("authorInputs");
            }
        }
        Ok(value)
    }

    /// 基线 `composeVisible(handle, artifactIds, expectedTextHash, algorithm?)`
    /// （main-generation-owner.ts:1320-1327）：`generation:compose-visible`
    /// 频道——按可见制品 ID 列表合成可见正文（追加/草稿可见文本算法）。
    ///
    /// ⚠️ 基线在 `requireRun(handle, true)` 前还执行 5 个可变性守卫
    /// （`reviewRevisions` / `imports` / `finalizations` / `graphs` /
    /// `legacyRosters` 的 `assertMutable`，:1322-1326）——对应服务尚未迁移，
    /// 按 B2 范围此处**刻意省略**（B3 写面收口时补齐）。
    pub fn compose_visible(
        &self,
        conn: &Connection,
        handle: &Value,
        artifact_ids: &[String],
        expected_text_hash: Option<&str>,
        algorithm: Option<&str>,
    ) -> Result<Value, String> {
        let algorithm = algorithm.unwrap_or("visible-continuation-v1");
        let first = self.require_run(conn, handle, false)?;
        if first
            .binding
            .source_manifest
            .get("operation")
            .and_then(Value::as_str)
            == Some("agent-round")
        {
            return Err(mutating_error("GENERATION_AGENT_CONTROL_IMMUTABLE"));
        }
        let run = self.require_run(conn, handle, true)?;
        let budget = self
            .repository
            .budget(conn, &run.root_action_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        if budget.root.status == "cancelled" {
            return Err(mutating_error("GENERATION_ACTION_CANCELLED"));
        }
        if is_draft_visible_text_version(algorithm)
            && first
                .binding
                .source_manifest
                .get("operation")
                .and_then(Value::as_str)
                != Some("chapter-draft")
        {
            return Err(mutating_error("GENERATION_COMPOSITION_ALGORITHM_INVALID"));
        }
        let composition = self
            .repository
            .compose_visible(
                conn,
                &run.run_id,
                artifact_ids,
                expected_text_hash,
                algorithm,
            )
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        serde_json::to_value(composition)
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))
    }
    /// 对齐基线 `assertCurrent`（main-generation-owner.ts:117-120）：
    /// owner 已挂起 → `GENERATION_OWNER_CLOSED`。
    ///
    /// 基线另外执行 `deps.assertCurrent()`（会话再认证 +
    /// `getProjectDb() !== database` → `GENERATION_DATABASE_CHANGED`）；
    /// Tauri 侧由命令层在缓存访问前经 `security::assert_current_project_context`
    /// 等价完成（见 [`MainGenerationOwnerCache::authorized_owner`]）。
    pub fn assert_current(&self) -> Result<(), String> {
        if self.closed {
            return Err(mutating_error(GENERATION_OWNER_CLOSED));
        }
        Ok(())
    }

    /// 生成账本仓储访问点（B3 迁移生成编排时使用；基线 owner 闭包内
    /// 直接捕获 `repository`）。
    pub fn repository(&self) -> &GenerationRunRepository {
        &self.repository
    }

    /// 从当前 Lorekeeper 模型配置冻结临时租约；成功绑定运行前由守卫负责回收。
    fn acquire_model_lease(&self, model_id: &str) -> Result<OwnerModelLease, String> {
        let now = owner_now_ms();
        let mut store = self
            .lease_store
            .lock()
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
        let receipt = store
            .begin_at(
                &crate::app_paths::models_config_path_at(&self.global_data_root),
                model_id,
                crate::project_access::random_uuid_v4(),
                now,
            )
            .map_err(|error| mutating_error(map_generation_error(error.message())))?;
        let lease = OwnerModelLease {
            store: Arc::clone(&self.lease_store),
            receipt,
            attached: false,
        };
        let model = store.resolve_model_at(&lease.receipt.lease_id, now);
        // 必须先释放注册表锁，错误分支才可由租约守卫安全清理。
        drop(store);
        let model = model.map_err(|error| mutating_error(map_generation_error(&error)))?;
        crate::services::main_generation_sources::validate_model(&model)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        Ok(lease)
    }

    pub fn record_run_lease(&self, run_id: String, lease_id: String) {
        let previous = self
            .run_leases
            .lock()
            .unwrap()
            .insert(run_id, lease_id.clone());
        if let Some(previous) = previous.filter(|previous| previous != &lease_id) {
            self.lease_store
                .lock()
                .unwrap()
                .close(&previous, owner_now_ms());
        }
    }

    /// 关闭前必须在数据库仍可用时 flush/suspend。失败时保留 owner 与租约供重试。
    pub(crate) fn suspend_with_database(&mut self, conn: &Connection) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        self.run_service
            .suspend_for_project_close(conn)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        self.suspend_for_project_close();
        Ok(())
    }

    /// 对齐基线 `suspendForProjectClose`（main-generation-owner.ts:1390-1399）：
    /// 项目关闭前挂起 owner——清准备态、关子生成器、挂起运行服务、
    /// 关闭全部在途模型执行租约、置 `closed`。
    ///
    /// 幂等：基线 `if (closed) return` 之后的清理只在首次执行
    /// （:1394-1398）；`preparations` 清理（:1391）随
    /// `prepareDraftContext` 迁移（G4-6c）接入；子生成器的
    /// 清理（:1392-1393）在对应服务迁移后接入。
    pub fn suspend_for_project_close(&mut self) {
        // 基线 :1391 `preparations.clear()`——草稿上下文
        // 准备缓存清空（G4-6c：`prepareDraftContext` 已迁移）。
        self.preparations.clear();
        // TODO(B3+)：`finalizedCharacters.close()` / `reviewRevisions.close()`
        //   （main-generation-owner.ts:1392-1393；子生成器迁移后接入）。
        if self.closed {
            return;
        }
        // 生产关闭路径先经 suspend_with_database 持久化服务状态；此处仅清进程资源。
        self.closed = true;
        self.events.close();
        // 逐个关闭 owner 登记的模型租约，不影响同注册表中其它 IPC 租约。
        let leases: Vec<String> = self
            .run_leases
            .lock()
            .unwrap()
            .drain()
            .map(|(_, lease)| lease)
            .collect();
        let mut store = self.lease_store.lock().unwrap();
        for lease in leases {
            store.close(&lease, owner_now_ms());
        }
    }

    // -----------------------------------------------------------------
    // B3 迁移占位：生成编排方法（方法体平移自 main-generation-owner.ts
    // 的同名闭包；契约类型随 `src/shared/generation-owner-contract.ts`
    // 迁移落地后补齐签名）。
    // -----------------------------------------------------------------

    /// 基线 `begin`（选择 → 建运行 → 返回运行视图，main-generation-owner.ts:351-444）。
    ///
    /// 本波次就位**准入校验面**（错误码 / 边界条件逐字对齐基线）：
    /// - 选择形状校验（:357-363）→ `GENERATION_BEGIN_INVALID`（操作名
    ///   正则 `^[a-z0-9][a-z0-9:_-]{0,127}$`、`promptKeys` 非空数组、
    ///   `parentRootActionId` 字符串非空、`uiActionNonce` 字符串非空且
    ///   ≤256、键集白名单 [`BEGIN_ALLOWED_KEYS`]——IPC 边界初始化
    ///   标志恒为 false，对齐 generation-controller.ts:173-178 注册形态；
    ///   `batch_initialization` 标志仅主进程内部 `beginBatch` 初始化
    ///   调用置 true（IPC 边界恒 false，见
    ///   [`MainGenerationOwner::begin_batch`]）；`agent_initialization`
    ///   标志仅主进程内部 agent 子代路径置 true（基线 :897
    ///   `begin(selection, false, true)`，IPC 边界恒 false——
    ///   放行 `agentInput` / `agentSession` 条件键、豁免
    ///   agent-round 准入拒绝与 `admitWorkflow` 子代准入）；
    /// - 子生成器准入（:364-369、:376）→ 五个 `*_ADMISSION_REQUIRED`；
    /// - agent 子代准入（:367）→ `agents.admitWorkflow` 校验链
    ///   （`GENERATION_AGENT_*`，见 [`Self::admit_workflow`]）；
    /// - agent 子代幂等早退（:371）→ 已登记子代存在时返回其视图；
    /// - 根行动子代禁令（:379-383，经 `repository.budget`）→ 三个
    ///   `*_CHILD_FORBIDDEN`；
    /// - 批量意图主进程独占（:387）→ `GENERATION_BATCH_INTENT_MAIN_ONLY`
    ///   （`batch_initialization = false` 的 IPC 边界语义；主进程
    ///   内部批量初始化调用传 `true` 豁免该检查）。
    /// 普通根运行已接入真实策略、模型租约、源绑定和 immediate 写入。
    /// 内部 Agent 初始化与后续轮次沿用同一父预算；普通工作流子代、导入、
    /// 批量、目录续接及审阅修订仍不由此入口豁免准入与生命周期合同。
    pub fn begin(
        &self,
        conn: &Connection,
        selection: &Value,
        batch_initialization: bool,
        agent_initialization: bool,
    ) -> Result<Value, String> {
        self.assert_current()?;
        // 基线 :357-363 `GENERATION_BEGIN_INVALID` 选择形状校验。
        let Some(operation) = selection.get("operation").and_then(Value::as_str) else {
            return Err(mutating_error("GENERATION_BEGIN_INVALID"));
        };
        let operation_valid = operation
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
            && operation.len() <= 128
            && operation.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || c == ':' || c == '_' || c == '-'
            });
        let prompt_keys_valid = selection
            .get("promptKeys")
            .and_then(Value::as_array)
            .is_some_and(|keys| !keys.is_empty());
        let parent_invalid = match selection.get("parentRootActionId") {
            Some(value) => value.as_str().map_or(true, |text| text.trim().is_empty()),
            None => false,
        };
        let nonce_valid = selection
            .get("uiActionNonce")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty() && text.chars().count() <= 256);
        let keys_valid = selection.as_object().is_some_and(|map| {
            map.keys().all(|key| {
                BEGIN_ALLOWED_KEYS.contains(&key.as_str())
                    || (agent_initialization
                        && matches!(key.as_str(), "agentInput" | "agentSession"))
            })
        });
        if !operation_valid || !prompt_keys_valid || parent_invalid || !nonce_valid || !keys_valid {
            return Err(mutating_error("GENERATION_BEGIN_INVALID"));
        }
        // 基线 :364-369 与 :376 子生成器准入（IPC 边界初始化标志恒为
        // false，故 editor-inline / 定稿 / 图 / 旧 roster / agent-round
        // 操作由渲染层直调一律拒绝——与基线 IPC 行为逐字一致）。
        if operation == "editor-inline" {
            return Err(mutating_error("GENERATION_EDITOR_ADMISSION_REQUIRED"));
        }
        if matches!(
            operation,
            "finalized-chapter-notes" | "finalized-character-state"
        ) {
            return Err(mutating_error("GENERATION_FINALIZATION_ADMISSION_REQUIRED"));
        }
        if GRAPH_OPERATIONS.contains(&operation) {
            return Err(mutating_error("GENERATION_GRAPH_ADMISSION_REQUIRED"));
        }
        if operation == "legacy-character-roster-repair" {
            return Err(mutating_error("GENERATION_LEGACY_ADMISSION_REQUIRED"));
        }
        if operation == "agent-round" && !agent_initialization {
            return Err(mutating_error("GENERATION_AGENT_ADMISSION_REQUIRED"));
        }
        // 基线 :367——主进程内部 agent 子代准入（agent 初始化
        // 路径跳过）。方法级构造 AgentGeneration 对齐基线 :897
        // owner 闭包内 `new AgentGeneration`（见 [`Self::admit_workflow`]
        // 头注的已知偏差说明）。
        let agent_workflow: Option<AdmittedWorkflow> = if agent_initialization {
            None
        } else {
            self.admit_workflow(conn, selection)
                .map_err(|error| mutating_error(&error))?
        };
        // 基线 :379-383 根行动子代禁令（editor / graph / legacy 子代）。
        if let Some(parent_root_action_id) =
            selection.get("parentRootActionId").and_then(Value::as_str)
        {
            let parent_operation = self
                .repository
                .budget(conn, parent_root_action_id)
                .map_err(|error| mutating_error(map_generation_error(&error)))?
                .root
                .operation;
            if parent_operation == "editor-inline" {
                return Err(mutating_error("GENERATION_EDITOR_CHILD_FORBIDDEN"));
            }
            if GRAPH_OPERATIONS.contains(&parent_operation.as_str()) {
                return Err(mutating_error("GENERATION_GRAPH_CHILD_FORBIDDEN"));
            }
            if parent_operation == "legacy-character-roster-repair" {
                return Err(mutating_error("GENERATION_LEGACY_CHILD_FORBIDDEN"));
            }
            // 内部 Agent 轮次只能延续 Agent 根，不借本切片开放其他领域的父根。
            if agent_initialization && parent_operation != "agent-round" {
                return Err(mutating_error("GENERATION_AGENT_ADMISSION_REQUIRED"));
            }
        }
        // 基线 :371——agent 子代幂等早退：admitWorkflow 定位到
        // 已登记子代时直接返回其运行视图，不重复创建。
        if let Some(agent_workflow) = agent_workflow {
            if let Some(existing) = agent_workflow.existing {
                let run = self
                    .repository
                    .get(conn, &existing.run_id)
                    .map_err(|error| mutating_error(map_generation_error(&error)))?;
                return self.view_of(conn, &run);
            }
        }
        // 基线 :387-389 批量意图主进程独占（IPC 边界 batchInitialization
        // 恒 false；主进程内部 beginBatch 初始化调用传 true 豁免）。
        if selection
            .get("batchIntent")
            .is_some_and(|intent| !intent.is_null())
            && !batch_initialization
        {
            return Err(mutating_error("GENERATION_BATCH_INTENT_MAIN_ONLY"));
        }
        let output_contract =
            generation_output_contract(selection).map_err(|error| mutating_error(&error))?;
        // Agent 初始化只能由内部 AgentHost 进入；普通 IPC 的拒绝守卫保留。
        if agent_initialization && operation != "agent-round" {
            return Err(mutating_error("GENERATION_AGENT_ADMISSION_REQUIRED"));
        }
        if ["importSlot", "importExecution"]
            .iter()
            .any(|key| selection.get(*key).is_some_and(|value| !value.is_null()))
            || matches!(operation, "import-global-facts" | "import-blueprints")
        {
            return Err(mutating_error("GENERATION_IMPORT_SLOT_REQUIRED"));
        }
        if selection
            .get("finalizedCharacterContextId")
            .is_some_and(|value| !value.is_null())
        {
            return Err(mutating_error("GENERATION_CHARACTER_CONTEXT_INVALID"));
        }
        if crate::services::review_revision_context::REVIEW_REVISION_OPERATIONS.contains(&operation)
            || selection
                .get("reviewRevisionContextId")
                .is_some_and(|value| !value.is_null())
            || (!agent_initialization
                && selection
                    .get("parentRootActionId")
                    .is_some_and(|value| !value.is_null()))
            || [
                "continueDirectoryOperationId",
                "batchId",
                "batchIntent",
                "agentWorkflowRegistrationId",
            ]
            .iter()
            .any(|key| selection.get(*key).is_some_and(|value| !value.is_null()))
        {
            return Err(mutating_error(GENERATION_REQUEST_FAILED));
        }
        // 普通根与内部 Agent 轮次均自建事务；外层事务的租约移交仍待后续接入。
        if !conn.is_autocommit() {
            return Err(mutating_error(GENERATION_REQUEST_FAILED));
        }
        let model_id = selection
            .get("modelId")
            .and_then(Value::as_str)
            .ok_or_else(|| mutating_error("GENERATION_BEGIN_INVALID"))?;
        let lease = self.acquire_model_lease(model_id)?;
        let model = {
            let mut store = lease
                .store
                .lock()
                .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
            store.resolve_model_at(&lease.receipt.lease_id, owner_now_ms())
        }
        .map_err(|error| mutating_error(map_generation_error(&error)))?;
        let receipt = crate::services::main_generation_sources::safe_model_receipt(&lease.receipt);
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
        let mut frozen_selection = selection.clone();
        let scope = crate::services::main_generation_plan::planning_generation_scope(selection)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        if let Some(scope) = &scope {
            use crate::services::main_generation_plan::PlanningGenerationScope;
            let range_end = match scope {
                PlanningGenerationScope::Directory { requested_range } => {
                    Some(requested_range.end_chapter)
                }
                PlanningGenerationScope::ArchitectureLegacyRange { intent }
                | PlanningGenerationScope::ArchitectureOutline { intent } => {
                    intent.synopsis_range.map(|range| range.to)
                }
            };
            let total = crate::repositories::project_core_repository::get(&tx)
                .map_err(|error| mutating_error(map_generation_error(&error)))?
                .map_or(0, |core| core.total_chapters);
            if range_end.is_some_and(|end| end > total) {
                return Err(mutating_error("GENERATION_PLANNING_RANGE_INVALID"));
            }
            let units = crate::services::main_generation_plan::planning_target_units(selection)
                .map_err(|error| mutating_error(map_generation_error(&error)))?;
            let mut inputs = match selection.get("authorInputs") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(inputs)) => inputs.clone(),
                _ => return Err(mutating_error("GENERATION_SOURCE_BINDING_INVALID")),
            };
            inputs.retain(|input| {
                input.get("id").and_then(Value::as_str) != Some("planning:target-units")
            });
            inputs.push(json!({"id":"planning:target-units", "text":units.to_string()}));
            frozen_selection["authorInputs"] = json!(inputs);
        }
        let parent_root_action_id = selection.get("parentRootActionId").and_then(Value::as_str);
        let (policy_value, budget) = if let Some(parent) = parent_root_action_id {
            // 沿用首轮冻结策略与实际父账本；不可用当前模型重新生成第二份预算。
            (
                self.root_policy(&tx, parent)?,
                self.repository
                    .budget(&tx, parent)
                    .map_err(|error| mutating_error(map_generation_error(&error)))?
                    .policy,
            )
        } else {
            let policy = crate::services::main_generation_plan::new_main_generation_policy(
                &frozen_selection,
                &model,
            )
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
            (policy.to_value(), policy.budget)
        };
        let prepared = match selection.get("preparationId") {
            None => None,
            Some(id) => Some(
                self.preparations
                    .iter()
                    .find(|(key, _)| Some(key.as_str()) == id.as_str())
                    .filter(|_| operation == "chapter-draft")
                    .map(|(_, entry)| entry)
                    .ok_or_else(|| mutating_error("GENERATION_DRAFT_PREPARATION_REQUIRED"))?,
            ),
        };
        if let Some(prepared) = prepared {
            if selection.get("materialDecision").is_none_or(Value::is_null) {
                return Err(mutating_error("GENERATION_MATERIAL_DECISION_REQUIRED"));
            }
            frozen_selection["knowledgeSnapshot"] = prepared.knowledge_snapshot.clone();
        }
        let binding = self.build_binding(
            &tx,
            &frozen_selection,
            &receipt,
            &policy_value,
            &output_contract,
        )?;
        if binding.source_manifest.get("policy") != Some(&policy_value) {
            return Err(mutating_error("GENERATION_POLICY_BINDING_MISMATCH"));
        }
        if prepared.is_some_and(|prepared| !preparation_matches(&prepared.binding, &binding, false))
        {
            return Err(mutating_error("GENERATION_DRAFT_PREPARATION_CHANGED"));
        }
        if binding.project_id != self.session.project_id || binding.epoch != self.session.lease_id {
            return Err(mutating_error("GENERATION_BINDING_INVALID"));
        }
        let frozen_input_hash = text_hash(
            &json!([
                binding.fingerprint,
                binding.context_snapshot_id,
                selection["output"]
            ])
            .to_string(),
        );
        if let Some(parent) = parent_root_action_id {
            self.repository
                .activate_root_for_committed_stage(
                    &tx,
                    parent,
                    &crate::repositories::generation_run_repository::ProjectEpoch {
                        project_id: binding.project_id.clone(),
                        epoch: binding.epoch.clone(),
                    },
                )
                .map_err(|error| mutating_error(map_generation_error(&error)))?;
        }
        let run = self
            .run_service
            .open_in_transaction(
                &tx,
                &crate::repositories::generation_run_repository::OpenGenerationRunRequest {
                    project_id: binding.project_id,
                    epoch: binding.epoch,
                    fingerprint: binding.fingerprint,
                    context_snapshot_id: binding.context_snapshot_id,
                    source_manifest: binding.source_manifest,
                    source_refs: binding.source_refs,
                    operation: operation.to_string(),
                    ui_action_nonce: selection["uiActionNonce"].as_str().unwrap().to_string(),
                    frozen_input_hash,
                    budget,
                    parent_root_action_id: parent_root_action_id.map(str::to_string),
                },
            )
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        let view = self.view_of(&tx, &run)?;
        tx.commit()
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
        let should_attach = run.binding.epoch == self.session.lease_id
            && !self
                .run_leases
                .lock()
                .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?
                .contains_key(&run.run_id);
        if should_attach {
            lease.attach(self, &run.run_id);
        }
        Ok(view)
    }

    /// 方法级借用视图共享 owner 的启动恢复标记，不重复修复当前活跃认领。
    fn agent_service<'a>(
        &'a self,
        conn: &'a Connection,
        assert_current: &'a dyn Fn() -> Result<(), String>,
    ) -> Result<AgentGeneration<'a>, String> {
        self.assert_current()?;
        if self.agent_recovery_done.load(Ordering::Acquire) {
            return Ok(AgentGeneration::without_recovery(
                conn,
                self.repository.clone(),
                &self.session.project_id,
                &self.session.lease_id,
                assert_current,
                self,
            ));
        }
        let agent = AgentGeneration::new(
            conn,
            self.repository.clone(),
            &self.session.project_id,
            &self.session.lease_id,
            assert_current,
            self,
        )?;
        // 借用外层事务时不能记住成功；其后回滚必须仍可重做启动恢复。
        if conn.is_autocommit() {
            self.agent_recovery_done.store(true, Ordering::Release);
        }
        Ok(agent)
    }

    /// 基线 agents.admitWorkflow 子代准入；恢复由 owner 生命周期控制。
    /// 错误码由调用方按修改类信封包装。
    fn admit_workflow(
        &self,
        conn: &Connection,
        selection: &Value,
    ) -> Result<Option<AdmittedWorkflow>, String> {
        let assert_current = || self.assert_current();
        let agent = self.agent_service(conn, &assert_current)?;
        agent.admit_workflow(selection)
    }

    // ── 批次 G4-10c：agent-generation 域 8 频道 owner 委托 ──
    // （基线 generation-controller.ts:217-230 的
    // 方法级重建借用视图，但启动恢复由 owner 生命周期控制，只执行一次。
    // 错误码（`GENERATION_AGENT_*`）经命令层折叠为
    // `mutating_error`（基线 register catch 错误映射）。

    /// 基线 `owner.agents.begin(request)`
    /// （controller:217）——幂等恢复入口：按 nonce 派生
    /// session key，命中且 context / modelId 一致则返回
    /// 既有 recovery，否则开第 0 轮 run。
    pub fn agent_begin(
        &self,
        conn: &Connection,
        request: &Value,
    ) -> Result<AgentGenerationRecovery, String> {
        let assert_current = || self.assert_current();
        let agent = self.agent_service(conn, &assert_current)?;
        agent.begin(request)
    }

    /// 基线 `owner.agents.read(request.handle)`
    /// （controller:218）——纯读：session 全轮次重建 +
    /// nextRound 计算 + sourceStatus。
    pub fn agent_read(
        &self,
        conn: &Connection,
        handle: &Value,
    ) -> Result<AgentGenerationRecovery, String> {
        let assert_current = || self.assert_current();
        let agent = self.agent_service(conn, &assert_current)?;
        agent.read(handle)
    }

    /// 基线 `owner.agents.resume(request.handle)`
    /// （controller:226）——已完成轮次走控制纪元写入，
    /// 否则 host.resume（LLM 续跑）。
    ///
    /// 未完成阶段仅经内部 AgentHost 恢复授权与账本；此方法自身不派发模型。
    /// 未知调用仍由轮次准备阶段只读重放，不因恢复而变成新请求。
    pub fn agent_resume(
        &self,
        conn: &Connection,
        handle: &Value,
    ) -> Result<AgentGenerationRecovery, String> {
        let assert_current = || self.assert_current();
        let agent = self.agent_service(conn, &assert_current)?;
        agent.resume(handle)
    }

    /// 基线 `owner.agents.executeRound(request)`
    /// （controller:227）——执行指定轮次（0..7）：构造
    /// messages 后 host.execute 派发 LLM。
    ///
    /// ⚠️ 本波占位：主体依赖 LLM 双阶段执行循环迁移，
    /// 当前 fail-closed 于 `GENERATION_AGENT_ROUND_UNMIGRATED`
    /// （迁移占位码、非基线码，经错误映射透传）。
    pub fn agent_round(
        &self,
        conn: &Connection,
        request: &Value,
    ) -> Result<AgentGenerationRecovery, String> {
        let assert_current = || self.assert_current();
        let agent = self.agent_service(conn, &assert_current)?;
        agent.execute_round(request)
    }

    /// 冻结轮次准备计划；仅 Agent 频道内部使用，普通 execute 仍拒绝 Agent 准入。
    pub(crate) fn prepare_agent_round(
        &self,
        conn: &Connection,
        request: &Value,
    ) -> Result<crate::services::agent_generation::PreparedAgentRound, String> {
        let assert_current = || self.assert_current();
        self.agent_service(conn, &assert_current)?
            .prepare_execute_round(request)
    }

    /// 基线 `owner.agents.claimTool(request)`
    /// （controller:228）——作者认领工具调用：
    /// pending→running（confirmed）或 declined；返回
    /// `{ action, execute }`。
    pub fn agent_claim_tool(
        &self,
        conn: &Connection,
        request: &Value,
    ) -> Result<AgentToolClaim, String> {
        let assert_current = || self.assert_current();
        let agent = self.agent_service(conn, &assert_current)?;
        agent.claim_tool(request)
    }

    /// 基线 `owner.agents.finishTool(request)`
    /// （controller:229）——工具结果回执写入
    /// （幂等重放 / 冲突检测）。
    pub fn agent_finish_tool(
        &self,
        conn: &Connection,
        request: &Value,
    ) -> Result<AgentToolAction, String> {
        let assert_current = || self.assert_current();
        let agent = self.agent_service(conn, &assert_current)?;
        agent.finish_tool(request)
    }

    /// 基线 `owner.agents.registerWorkflow(request.ref)`
    /// （controller:230 register-workflow 项）——
    /// `start_workflow` 工具确认后注册子生成工作流。
    pub fn agent_register_workflow(
        &self,
        conn: &Connection,
        ref_: &Value,
    ) -> Result<AgentWorkflowRegistration, String> {
        let assert_current = || self.assert_current();
        let agent = self.agent_service(conn, &assert_current)?;
        agent.register_workflow(ref_)
    }

    /// 基线 `owner.agents.commitDomainTool(request.ref)`
    /// （controller:230 commit-domain-tool 项）——在捕获
    /// 的事务内提交 `propose_novel_config` /
    /// `propose_chapter_blueprint` 域效应。
    pub fn agent_commit_domain_tool(
        &self,
        conn: &Connection,
        ref_: &Value,
    ) -> Result<AgentDomainToolReceipt, String> {
        let assert_current = || self.assert_current();
        let agent = self.agent_service(conn, &assert_current)?;
        agent.commit_domain_tool(ref_)
    }

    /// 普通 IPC 执行：准备、分发与回执投影共用原业务合同。
    ///
    /// 准备阶段已覆盖身份、子域准入、语义任务、输出合同、任务哈希、
    /// 幂等读回、源重建与冻结计划。材料提示哈希、短细纲重试和特殊域仍
    /// fail-closed；快照/推理事件及生产短锁接线尚未完成。
    pub fn execute(&self, conn: &Connection, request: &Value) -> Result<Value, String> {
        // IPC 边界执行标志恒 false（基线 :501 默认参数）。
        self.execute_inner(conn, request, false)
    }

    /// 基线 `execute` 主体（main-generation-owner.ts:501-623
    /// 单函数体 + 执行标志参数分流）：`agent_execution` 仅
    /// 主进程内部 agent 执行路径传 true（基线 :899 owner 闭包
    /// `execute: (handle, invocationNonce, task) =>
    /// execute({ handle, invocationNonce, task }, true)`，见
    /// [`Self::execute_agent_round`]）——豁免 agent-round 的
    /// IPC 准入拒绝（:504），其余准入校验面逐字不变。
    ///
    /// 兼容原 owner/内部 Agent 调用，保持准备守卫到回执投影完成。
    fn execute_inner(
        &self,
        conn: &Connection,
        request: &Value,
        agent_execution: bool,
    ) -> Result<Value, String> {
        let prepared = self.prepare_execution(conn, request, agent_execution)?;
        let result = prepared.execute_blocking(conn)?;
        self.finish_prepared_execution(conn, &prepared, &result)
    }

    /// 同步短准备阶段；无网络分发、无预算预留，返回值不借用本方法参数。
    /// agent_execution 仅供既有内部 Agent 路径，IPC 始终传 false。
    pub(crate) fn prepare_execution(
        &self,
        conn: &Connection,
        request: &Value,
        agent_execution: bool,
    ) -> Result<PreparedMainGenerationExecution, String> {
        let handle = request.get("handle").cloned().unwrap_or(Value::Null);
        let run = self.require_run(conn, &handle, true)?;
        // 基线 :512-521 子生成器执行准入（IPC 边界执行标志恒为
        // false；主进程内部 agent 执行路径传 true 豁免 agent-round）。
        let manifest = &run.binding.source_manifest;
        let run_operation = manifest
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if run_operation == "agent-round" && !agent_execution {
            return Err(mutating_error("GENERATION_AGENT_ADMISSION_REQUIRED"));
        }
        if manifest
            .get("importSlot")
            .is_some_and(|slot| !slot.is_null())
        {
            return Err(mutating_error("GENERATION_IMPORT_ADMISSION_REQUIRED"));
        }
        if run_operation == "editor-inline" {
            return Err(mutating_error("GENERATION_EDITOR_ADMISSION_REQUIRED"));
        }
        if matches!(
            run_operation.as_str(),
            "finalized-chapter-notes" | "finalized-character-state"
        ) {
            return Err(mutating_error("GENERATION_FINALIZATION_ADMISSION_REQUIRED"));
        }
        if GRAPH_OPERATIONS.contains(&run_operation.as_str()) {
            return Err(mutating_error("GENERATION_GRAPH_ADMISSION_REQUIRED"));
        }
        if run_operation == "legacy-character-roster-repair" {
            return Err(mutating_error("GENERATION_LEGACY_ADMISSION_REQUIRED"));
        }
        // 基线 :516-517——任务形状断言（`assertSemanticGenerationTask`）：
        // 语义任务合同在任何持久化 / 分发之前先行拒绝非法形状。
        let requested_task = request.get("task").cloned().unwrap_or(Value::Null);
        crate::services::main_generation_plan::assert_semantic_generation_task(&requested_task)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        // 基线 :518 `structuredClone(request.task)`——Rust 值语义克隆。
        let mut task = requested_task;
        // 基线 :519-526 规划目标单位指令注入（`planning:target-units`）。
        self.inject_planning_target_instruction(conn, &run, &mut task)?;
        // 基线 :532-537 材料决策必需性（审稿 / 精修类操作与带准备的
        // chapter-draft 必须已绑定材料决策收据）。
        let material_decision = manifest
            .get("materialDecision")
            .filter(|value| !value.is_null());
        if (material_decision.is_none()
            && matches!(
                run_operation.as_str(),
                "review-chapter" | "refine-draft" | "refine-from-review"
            ))
            || (material_decision.is_none()
                && run_operation == "chapter-draft"
                && manifest.get("preparationId").is_some())
        {
            return Err(mutating_error("GENERATION_MATERIAL_DECISION_REQUIRED"));
        }
        // 基线 :538-590 的材料决策提示哈希族（短细纲块编解码、生成前
        // 定稿对账输出重算与逐字哈希比对）尚未迁移：一旦运行绑定了材料
        // 决策，这里必须 fail-closed，绝不带着未经校验的提示词进入分发。
        if material_decision.is_some() {
            return Err(mutating_error("GENERATION_MATERIAL_PROMPT_UNVERIFIED"));
        }
        // 基线 :585-590 输出合同一致性（task.output 必须与绑定输出
        // 合同——字符串主合同或 purpose 覆盖合同——逐字一致）。
        let task_purpose = task
            .get("purpose")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let expected_output = match manifest.get("outputContract") {
            Some(Value::String(primary)) => primary.clone(),
            Some(composite) => composite
                .get("overrides")
                .and_then(Value::as_array)
                .and_then(|overrides| {
                    overrides.iter().find(|item| {
                        item.get("purpose").and_then(Value::as_str) == Some(task_purpose)
                    })
                })
                .and_then(|item| {
                    item.get("output")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .or_else(|| {
                    composite
                        .get("primary")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_default(),
            _ => String::new(),
        };
        if task
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or_default()
            != expected_output
        {
            return Err(mutating_error("GENERATION_OUTPUT_CONTRACT_CHANGED"));
        }
        // 基线 :568 大纲请求断言（目的 / 任务与持久化大纲请求一致）。
        self.repository
            .assert_plot_outline_request(conn, &run.run_id, Some(task_purpose), Some(&task), None)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        // 基线 :617——agent 执行路径的 agentToolNames 提取
        // （`run.binding.sourceManifest.agentContext.input.tools`
        // 逐项取 `name`，纯 JSON 遍历）。预置提取逻辑、
        // 暂无消费者——随分发核心（generation-run-service）
        // 波次接线 `service.execute` 的
        // providerRequest.agentToolNames。
        let agent_tool_names: Vec<String> = if agent_execution {
            manifest
                .get("agentContext")
                .and_then(|context| context.get("input"))
                .and_then(|input| input.get("tools"))
                .and_then(Value::as_array)
                .map(|tools| {
                    tools
                        .iter()
                        .filter_map(|tool| {
                            tool.get("name").and_then(Value::as_str).map(str::to_string)
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        // 基线 :591-599——`requestHash`（语义任务 + 冻结模型租约修订 +
        // 策略哈希）与在途键 `JSON.stringify([runId, invocationNonce])`。
        let invocation_nonce = request
            .get("invocationNonce")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let request_hash = Self::owner_text_hash(
            &serde_json::to_string(&json!([
                task,
                run.binding.fingerprint.model_lease_revision,
                run.binding.fingerprint.policy_hash
            ]))
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?,
        );
        let in_flight_key = json!([run.run_id, invocation_nonce]).to_string();
        let pending = self.register_pending_invocation(&in_flight_key, &request_hash)?;
        // 构造失败也由局部守卫回收；成功则随准备对象持有到最终投影退出。
        let plan = self.prepare_execution_plan(
            conn,
            &run,
            &task,
            manifest,
            task_purpose,
            if agent_execution {
                Some(agent_tool_names)
            } else {
                None
            },
            &invocation_nonce,
            &request_hash,
        )?;
        Ok(PreparedMainGenerationExecution {
            task,
            plan,
            service: self.run_service.clone(),
            _pending: pending,
        })
    }

    fn register_pending_invocation(
        &self,
        key: &str,
        request_hash: &str,
    ) -> Result<PendingOwnerInvocationGuard, String> {
        let mut pending = self
            .pending_invocations
            .lock()
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
        if let Some(existing) = pending.get_mut(key) {
            if existing.request_hash != request_hash {
                return Err(mutating_error("GENERATION_INVOCATION_CONFLICT"));
            }
            existing.holders += 1;
        } else {
            pending.insert(
                key.to_string(),
                PendingOwnerInvocation {
                    request_hash: request_hash.to_string(),
                    holders: 1,
                },
            );
        }
        Ok(PendingOwnerInvocationGuard {
            pending: self.pending_invocations.clone(),
            key: key.to_string(),
            request_hash: request_hash.to_string(),
        })
    }

    /// 调用方在生产短锁接线时必须先重新认证 owner/项目/连接身份。
    pub(crate) fn finish_prepared_execution(
        &self,
        conn: &Connection,
        prepared: &PreparedMainGenerationExecution,
        result: &PreparedMainGenerationResult,
    ) -> Result<Value, String> {
        match result {
            PreparedMainGenerationResult::Replay(outcome) => Ok(outcome.clone()),
            PreparedMainGenerationResult::Receipt(receipt) => {
                if let Some(snapshot) = self.snapshot_of(conn, receipt, None)? {
                    self.events
                        .publish(MainGenerationEvent::Snapshot(Box::new(snapshot)));
                }
                self.outcome_of(conn, receipt, &prepared.task)
            }
        }
    }

    /// 准备阶段：幂等读回 → 短细纲准入 → 源重建比对 → 运行租约 →
    /// 策略 / 预算 / 能力证据 → 冻结计划与 provider 请求；不执行网络 Future。
    ///
    /// ⚠️ 诚实化边界（B3-3e 切片）：材料决策提示哈希族未迁移（已在
    /// [`Self::prepare_execution`] fail-closed），短细纲重试准入在此显式拒绝；
    /// 子生成器可变性断言所属域（`reviewRevisions` / `imports` /
    /// `finalizations` / `graphs` / `legacyRosters`）的 `begin` 准入仍未
    /// 开放，运行不可能到达此处。快照在持久 flush 和最终收据时投影；推理只走易失出口。
    #[allow(clippy::too_many_arguments)]
    fn prepare_execution_plan(
        &self,
        conn: &Connection,
        run: &DurableGenerationRun,
        task: &Value,
        manifest: &Map<String, Value>,
        task_purpose: &str,
        agent_tool_names: Option<Vec<String>>,
        invocation_nonce: &str,
        request_hash: &str,
    ) -> Result<PreparedExecutionPlan, String> {
        // 基线 :607-610——幂等读回（`findInvocation`）：已结算且不处于
        // 在途状态（`dispatch-marked` / `reserved`）的调用重放既有收据。
        if let Some(existing) = self
            .repository
            .find_invocation(conn, &run.run_id, invocation_nonce, request_hash)
            .map_err(|error| mutating_error(map_generation_error(&error)))?
        {
            if existing.attempt.status != "dispatch-marked" && existing.attempt.status != "reserved"
            {
                return self
                    .outcome_of(conn, &existing, task)
                    .map(PreparedExecutionPlan::Replay);
            }
        }
        // 基线 :611-616——短细纲重试准入（`assertShortOutlineRetryAvailable`
        // 与任务逐字比对）随短细纲块编解码波次迁移；此前显式拒绝。
        if task_purpose == DRAFT_SHORT_OUTLINE_PURPOSE {
            return Err(mutating_error("GENERATION_DRAFT_SHORT_OUTLINE_UNVERIFIED"));
        }
        // ① 基线 :626-630——真实模型证据重建源并逐字比对。
        let current = self.rebuild_binding(conn, &run.binding)?;
        if current != run.binding {
            return Err(mutating_error("GENERATION_SOURCE_CHANGED"));
        }
        // ② 基线 :631-633——本运行的模型执行租约。
        let lease_id = self
            .run_leases
            .lock()
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?
            .get(&run.run_id)
            .cloned()
            .ok_or_else(|| mutating_error("GENERATION_RESUME_REQUIRED"))?;
        let model = {
            let mut store = self
                .lease_store
                .lock()
                .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
            store.resolve_model_at(&lease_id, owner_now_ms())
        }
        .map_err(|error| mutating_error(map_generation_error(&error)))?;
        // ③ 基线 :634-637——创作策略、冻结策略、预算与模型能力证据。
        let creative_strategy = crate::repositories::project_core_repository::get(conn)
            .map_err(|error| mutating_error(map_generation_error(&error)))?
            .map(|core| {
                crate::llm::reasoning::CreativeStrategy::parse(Some(&core.creative_strategy))
            })
            .unwrap_or(crate::llm::reasoning::CreativeStrategy::Auto);
        let policy: crate::services::main_generation_plan::MainGenerationPolicy =
            crate::services::main_generation_plan::read_main_generation_policy(
                &read_main_generation_policy(manifest.get("policy"))?,
            )
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        let budget = self
            .repository
            .budget(conn, &run.root_action_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        // 能力证据由当前模型快照重新推导（基线 `modelReceipt(binding)` 的
        // `capabilityEvidence` 与绑定时冻结值已由 `rebuild_binding` 逐字比对）。
        let evidence = crate::llm::lease::resolve_model_execution_capability_evidence(
            &crate::llm::lease::LeaseModelProfile::from_value(&model),
        )
        .map_err(|error| mutating_error(map_generation_error(error.message())))?;
        // ④ 基线 :638-644——主生成计划（输入上界 / 物理责任 / 预算决策）。
        let plan = crate::services::main_generation_plan::build_main_generation_plan(
            &model,
            &evidence,
            task,
            &budget,
            creative_strategy,
            &policy,
        )
        .map_err(|error| mutating_error(map_generation_error(&error)))?;
        // ⑤ 基线 :646-... ——进程内 provider 请求与内部执行请求。
        let semantic_task =
            crate::services::main_generation_plan::assert_semantic_generation_task(task)
                .map_err(|error| mutating_error(map_generation_error(&error)))?;
        let provider_request = crate::services::main_generation_dispatcher::provider_request(
            &model,
            &semantic_task,
            &plan,
            &run.root_action_id,
        );
        let replay_task = manifest
            .get("importSlot")
            .is_some_and(|slot| !slot.is_null())
            || manifest
                .get("materialDecision")
                .and_then(|decision| decision.get("shortOutlinePromptHash"))
                .is_some()
            || self
                .repository
                .read_plot_outline(conn, &run.run_id, None)
                .map_err(|error| mutating_error(map_generation_error(&error)))?
                .is_some();
        let execution_request = crate::services::generation_run_service::ExecuteGenerationRequest {
            run_id: run.run_id.clone(),
            invocation_nonce: invocation_nonce.to_string(),
            request_hash: request_hash.to_string(),
            provider_request,
            reserved_tokens: plan.reserved_tokens,
            requested_output_tokens: plan.requested_output_tokens,
            input_upper_bound_tokens: plan.input_upper_bound_tokens,
            reasoning_upper_bound_tokens: plan.reasoning_upper_bound_tokens,
            usage_policy: plan.usage_policy.clone(),
            purpose: Some(task_purpose.to_string()),
            budget_decision: plan
                .budget_decision
                .as_ref()
                .map(Self::task_budget_decision_value),
            agent_tool_names,
            replay_task: replay_task.then(|| task.clone()),
            reconciliation_injected: false,
        };
        Ok(PreparedExecutionPlan::Dispatch(Box::new(execution_request)))
    }

    /// 基线 `outcomeOf(receipt, task)`（main-generation-owner.ts:440-500）：
    /// 执行回执 → `MainGenerationExecuteReceipt`
    /// （`{ outcome: { status, content, finishReason, receipt }, run }`）。
    fn outcome_of(
        &self,
        conn: &Connection,
        receipt: &GenerationExecutionReceipt,
        task: &Value,
    ) -> Result<Value, String> {
        let frozen_model = receipt
            .run
            .binding
            .source_manifest
            .get("modelReceipt")
            .cloned()
            .unwrap_or(Value::Null);
        let capability = frozen_model
            .get("capabilityEvidence")
            .cloned()
            .unwrap_or(Value::Null);
        let stored = receipt
            .result
            .as_ref()
            .and_then(|result| result.finish_reason.clone());
        let finish_reason = if receipt.failure_code.is_some() {
            "error".to_string()
        } else if matches!(
            stored.as_deref(),
            Some("stop" | "length" | "content_filter" | "error" | "unknown")
        ) {
            stored.clone().unwrap_or_default()
        } else {
            "unknown".to_string()
        };
        // 失败码只放行两个已知安全码（基线 `safeFailureCode`）。
        let safe_failure_code = match receipt.failure_code.as_deref() {
            Some(code @ ("GENERATION_PROVIDER_FAILED" | "NETWORK_ERROR")) => Some(code.to_string()),
            _ => None,
        };
        let attempt_number = receipt
            .budget
            .attempts
            .iter()
            .position(|attempt| attempt.attempt_id == receipt.attempt.attempt_id)
            .map(|index| index as i64 + 1)
            .unwrap_or(0);
        let cumulative_requested: i64 = receipt
            .budget
            .attempts
            .iter()
            .filter(|attempt| attempt.status != "cancelled-before-dispatch")
            .map(|attempt| attempt.requested_output_tokens)
            .sum();
        let mut details = Map::new();
        details.insert(
            "purpose".to_string(),
            task.get("purpose").cloned().unwrap_or(Value::Null),
        );
        if let Some(diagnostics) = receipt.diagnostics.as_ref() {
            details.insert(
                "diagnostics".to_string(),
                serde_json::to_value(diagnostics).unwrap_or(Value::Null),
            );
        }
        if let Some(code) = safe_failure_code {
            details.insert("failureCode".to_string(), Value::String(code));
        }
        if let Some(artifact) = receipt.artifact.as_ref() {
            details.insert(
                "visibleArtifact".to_string(),
                json!({
                    "artifactId": artifact.artifact_id,
                    "attemptId": artifact.attempt_id,
                    "revision": artifact.revision,
                    "textHash": artifact.text_hash,
                }),
            );
        }
        details.insert(
            "model".to_string(),
            json!({
                "id": frozen_model.get("modelId").cloned().unwrap_or(Value::Null),
                "configurationRevision": frozen_model.get("modelRevision").cloned().unwrap_or(Value::Null),
                "endpointFingerprint": frozen_model.get("endpointFingerprint").cloned().unwrap_or(Value::Null),
            }),
        );
        details.insert(
            "capabilities".to_string(),
            json!({
                "contextWindowTokens": capability.get("contextWindowTokens").cloned().unwrap_or(Value::Null),
                "maxOutputTokens": capability.get("maxOutputTokens").cloned().unwrap_or(Value::Null),
                "reasoning": capability.get("reasoning").cloned().unwrap_or(Value::Null),
                "structuredOutput": capability.get("structuredOutput").cloned().unwrap_or(Value::Null),
                "usage": capability.get("usage").cloned().unwrap_or(Value::Null),
                "source": capability.get("source").cloned().unwrap_or(Value::Null),
            }),
        );
        details.insert(
            "budget".to_string(),
            json!({
                "attempt": attempt_number,
                "maxAttempts": receipt.budget.policy.max_physical_requests,
                "requestedOutputTokens": receipt.attempt.requested_output_tokens,
                "cumulativeRequestedOutputTokens": cumulative_requested,
                "maxRequestedOutputTokens": receipt.budget.policy.max_token_liability,
                "maxRequestedOutputTokensPerAttempt": receipt.budget.policy.max_output_per_request,
                "deadlineAt": owner_now_ms() as i64
                    + (receipt.budget.policy.max_active_elapsed_ms - receipt.budget.active_elapsed_ms).max(0),
            }),
        );
        details.insert(
            "finishReason".to_string(),
            Value::String(finish_reason.clone()),
        );
        let content = receipt
            .artifact
            .as_ref()
            .map(|artifact| artifact.text.clone())
            .unwrap_or_default();
        let outcome = if finish_reason == "stop" {
            json!({ "status": "completed", "content": content, "finishReason": finish_reason, "receipt": details })
        } else {
            json!({ "status": "incomplete", "content": content, "finishReason": finish_reason, "receipt": details })
        };
        Ok(json!({ "outcome": outcome, "run": self.view_of(conn, &receipt.run)? }))
    }

    /// 基线 :519-526——规划类操作（大纲 / 蓝图目录）在语义任务末尾注入
    /// 「目标单位」系统指令（仅当显式存在 `planning:target-units` 作者
    /// 输入时）；指令按项目写作语言生成，且仅当任务中尚无同一 system
    /// 消息承载该指令时追加。
    fn inject_planning_target_instruction(
        &self,
        conn: &Connection,
        run: &DurableGenerationRun,
        task: &mut Value,
    ) -> Result<(), String> {
        let manifest = &run.binding.source_manifest;
        let kind = match manifest
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "generate-plot-outline" => "outline",
            "chapter-blueprint-directory" => "blueprint",
            _ => return Ok(()),
        };
        let Some(target) = manifest
            .get("authorInputs")
            .and_then(Value::as_array)
            .and_then(|inputs| {
                inputs.iter().find(|input| {
                    input.get("id").and_then(Value::as_str) == Some("planning:target-units")
                })
            })
            .and_then(|input| input.get("text").and_then(Value::as_str))
        else {
            return Ok(());
        };
        let language = crate::repositories::project_core_repository::get(conn)
            .map_err(|error| mutating_error(map_generation_error(&error)))?
            .map(|core| core.writing_language)
            .unwrap_or_else(|| "zh-CN".to_string());
        let language = if language == "en-US" {
            "en-US"
        } else {
            "zh-CN"
        };
        let target_units = crate::commands::generation::parse_planning_target_units(Some(target))
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        let instruction =
            crate::commands::generation::planning_target_instruction(kind, target_units, language);
        let messages = task
            .get_mut("messages")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| mutating_error(GENERATION_REQUEST_FAILED))?;
        let injected = messages.iter().any(|message| {
            message.get("role").and_then(Value::as_str) == Some("system")
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|content| content.contains(&instruction))
        });
        if !injected {
            messages.push(json!({ "role": "system", "content": instruction }));
        }
        Ok(())
    }

    /// 基线 `textHash`（SHA-256 小写十六进制）——owner 侧 `requestHash`
    /// 计算（与 `generation_run_repository::text_hash` 同算法）。
    fn owner_text_hash(text: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(text.as_bytes());
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// 基线 `TaskBudgetDecision`（`src/services/generation/task-budget-planner.ts:110`）
    /// 的 JSON 投影：camelCase 逐字段、`decision` / `demandKind` / 原因码按
    /// 基线字面量联合取值。Rust 侧该类型刻意不派发 `Serialize`，投影在此
    /// 显式书写，保证写入 attempt usage 的冻结形状与基线一致。
    fn task_budget_decision_value(
        decision: &crate::services::task_budget_planner::TaskBudgetDecision,
    ) -> Value {
        use crate::services::task_budget_planner::{
            TaskBudgetDecisionKind, TaskBudgetDemandKind, TaskBudgetReasonCode,
        };
        let decision_kind = match decision.decision {
            TaskBudgetDecisionKind::Ready => "ready",
            TaskBudgetDecisionKind::SplitRequired => "split-required",
            TaskBudgetDecisionKind::CapacityConflict => "capacity-conflict",
        };
        let demand_kind = match decision.demand_kind {
            TaskBudgetDemandKind::DraftUnits => "draft-units",
            TaskBudgetDemandKind::StructuredItems => "structured-items",
        };
        let reasons: Vec<Value> = decision
            .reasons
            .iter()
            .map(|reason| {
                let code = match reason.code {
                    TaskBudgetReasonCode::TaskDemand => "task-demand",
                    TaskBudgetReasonCode::OutputAllocationCeiling => "output-allocation-ceiling",
                    TaskBudgetReasonCode::InputUpperBound => "input-upper-bound",
                    TaskBudgetReasonCode::SafetyMargin => "safety-margin",
                    TaskBudgetReasonCode::ModelContextCap => "model-context-cap",
                    TaskBudgetReasonCode::ModelOutputCap => "model-output-cap",
                    TaskBudgetReasonCode::UserContextCap => "user-context-cap",
                    TaskBudgetReasonCode::UserOutputCap => "user-output-cap",
                    TaskBudgetReasonCode::RootOutputCap => "root-output-cap",
                    TaskBudgetReasonCode::RootRemainingCap => "root-remaining-cap",
                    TaskBudgetReasonCode::ProtocolReasoningReserve => "protocol-reasoning-reserve",
                    TaskBudgetReasonCode::ProtocolTotalBound => "protocol-total-bound",
                    TaskBudgetReasonCode::ModelCapabilityUnknown => "model-capability-unknown",
                    TaskBudgetReasonCode::LiabilityBoundUnknown => "liability-bound-unknown",
                    TaskBudgetReasonCode::RequiredInputCapacityConflict => {
                        "required-input-capacity-conflict"
                    }
                    TaskBudgetReasonCode::SingleItemCapacityConflict => {
                        "single-item-capacity-conflict"
                    }
                    TaskBudgetReasonCode::DraftSegmentationDisabled => {
                        "draft-segmentation-disabled"
                    }
                    TaskBudgetReasonCode::ScopeSplit => "scope-split",
                };
                let mut projected = Map::new();
                projected.insert("code".to_string(), Value::String(code.to_string()));
                if let Some(value) = reason.value_tokens {
                    projected.insert("valueTokens".to_string(), json!(value));
                }
                projected.insert("selected".to_string(), json!(reason.selected));
                Value::Object(projected)
            })
            .collect();
        json!({
            "policyVersion": decision.policy_version,
            "outputEstimatorVersion": decision.output_estimator_version,
            "inputEstimatorVersion": decision.input_estimator_version.clone(),
            "decision": decision_kind,
            "stage": serde_json::to_value(decision.stage).unwrap_or(Value::Null),
            "writingLanguage": decision.writing_language.clone(),
            "demandKind": demand_kind,
            "requestedQuantity": decision.requested_quantity,
            "selectedQuantity": decision.selected_quantity,
            "remainingQuantity": decision.remaining_quantity,
            "requestedOutputTokens": decision.requested_output_tokens,
            "reservedOutputTokens": decision.reserved_output_tokens,
            "reasoningUpperBoundTokens": decision.reasoning_upper_bound_tokens,
            "reservationLiabilityTokens": decision.reservation_liability_tokens,
            "inputUpperBoundTokens": decision.input_upper_bound_tokens,
            "reasons": reasons,
        })
    }

    /// 基线 owner 闭包 :899 的 host.execute 注入等价物——
    /// `execute({ handle, invocationNonce, task }, true)`
    /// （主进程内部 agent 执行路径，豁免 agent-round 的
    /// IPC 准入拒绝）。
    fn execute_agent_round(
        &self,
        conn: &Connection,
        handle: &MainGenerationRunHandle,
        nonce: &str,
        task: &Value,
    ) -> Result<Value, String> {
        let request = json!({
            "handle": handle,
            "invocationNonce": nonce,
            "task": task,
        });
        self.execute_inner(conn, &request, true)
    }

    /// 基线 `bindMaterialDecision`（main-generation-owner.ts:646-665）：
    /// 材料决策收据绑定到未启动运行的源清单。
    ///
    /// 本波次就位**准入与冲突校验面**（逐字对齐基线）：运行身份
    /// （`require_run`，execution 语义）、操作准入（:649-651，仅
    /// 审稿 / 精修类操作可绑定）→ `GENERATION_MATERIAL_DECISION_INVALID`、
    /// 幂等重放与冲突检测（:652-656，已绑定且逐字段一致 → 返回运行
    /// 视图；已绑定但不一致 → `GENERATION_MATERIAL_DECISION_CONFLICT`）。
    ///
    /// ⚠️ 诚实化占位（B3 波次说明）：基线 :657-664 的候选绑定重建
    /// （`deps.rebuildBinding`——`rebuild_generation_source_binding`
    /// 逻辑层 G4-6a 已就位，但其 `GenerationSourceBindingDependencies`
    /// 的 `readBuiltinPrompt` 依赖 electron/src/services/
    /// builtin-prompt-templates.ts 的 1506 行内置提示词模板库，尚未
    /// 移植到 Rust 侧）与投影比对 / `replaceUnstartedBinding` 持久化
    /// 随该模板库迁移波次补齐。影响面：首次绑定收到「尚未迁移」
    /// 拒绝；重复绑定（幂等）与冲突分支的行为与基线逐字一致。
    pub fn bind_material_decision(
        &self,
        conn: &Connection,
        handle: &Value,
        material_decision: &Value,
    ) -> Result<Value, String> {
        let run = self.require_run(conn, handle, true)?;
        // 基线 :649-651：仅审稿 / 精修类操作可绑定材料决策。
        let operation = run
            .binding
            .source_manifest
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !["review-chapter", "refine-draft", "refine-from-review"].contains(&operation.as_str()) {
            return Err(mutating_error("GENERATION_MATERIAL_DECISION_INVALID"));
        }
        // 基线 :652-656：已绑定 → 逐字段一致幂等返回视图，否则冲突。
        if let Some(existing) = run.binding.source_manifest.get("materialDecision") {
            if existing != material_decision {
                return Err(mutating_error("GENERATION_MATERIAL_DECISION_CONFLICT"));
            }
            return self.view_of(conn, &run);
        }
        // ⚠️ B3 占位：基线 :657-664 的候选绑定重建（依赖内置提示词
        // 模板库）/ 投影比对 / replaceUnstartedBinding 持久化未迁移——
        // 重建点 fail-closed（命令层收敛为 `GENERATION_REQUEST_FAILED`）。
        Err(mutating_error(
            "材料决策绑定尚未迁移：内置提示词模板库（builtin-prompt-templates）待后续批次移植",
        ))
    }

    /// 基线 `resume`（运行恢复重分发，main-generation-owner.ts:625-646）。
    ///
    /// 运行身份校验与大纲可写性断言保持基线顺序。
    /// 普通运行恢复：真实源重建、临时模型租约校验、服务授权与账本恢复。
    /// 保守跳过规划早退优化；未迁移特殊域仍按准入码拒绝。执行重分发入口另行接线。
    pub fn resume(&self, conn: &Connection, handle: &Value) -> Result<Value, String> {
        self.resume_with_agent_admission(conn, handle, false)
    }

    /// 仅 AgentHost 可恢复未完成的 Agent 阶段；普通 IPC 不继承这个豁免。
    fn resume_with_agent_admission(
        &self,
        conn: &Connection,
        handle: &Value,
        agent_initialization: bool,
    ) -> Result<Value, String> {
        let run = self.require_run(conn, handle, false)?;
        self.repository
            .assert_plot_outline_writable(conn, &run.run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        // 特殊域的可变性合同尚未接入，不能因恢复接线绕过其准入。
        let manifest = &run.binding.source_manifest;
        let operation = manifest
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if agent_initialization && operation != "agent-round" {
            return Err(mutating_error("GENERATION_AGENT_ADMISSION_REQUIRED"));
        }
        // 未完成阶段的租约移交只在独立写入边界成功后生效。
        if agent_initialization && !conn.is_autocommit() {
            return Err(mutating_error(GENERATION_REQUEST_FAILED));
        }
        let blocked = if operation == "editor-inline" {
            Some("GENERATION_EDITOR_ADMISSION_REQUIRED")
        } else if !agent_initialization
            && (operation == "agent-round"
                || ["agentInput", "agentSession", "agentAdmission"]
                    .iter()
                    .any(|key| manifest.get(*key).is_some_and(|value| !value.is_null())))
        {
            Some("GENERATION_AGENT_ADMISSION_REQUIRED")
        } else if manifest
            .get("importSlot")
            .is_some_and(|value| !value.is_null())
        {
            Some("GENERATION_IMPORT_ADMISSION_REQUIRED")
        } else if matches!(
            operation,
            "finalized-chapter-notes" | "finalized-character-state"
        ) {
            Some("GENERATION_FINALIZATION_ADMISSION_REQUIRED")
        } else if GRAPH_OPERATIONS.contains(&operation) {
            Some("GENERATION_GRAPH_ADMISSION_REQUIRED")
        } else if operation == "legacy-character-roster-repair" {
            Some("GENERATION_LEGACY_ADMISSION_REQUIRED")
        } else {
            None
        };
        if let Some(blocked) = blocked {
            return Err(mutating_error(blocked));
        }
        let receipt = crate::services::main_generation_sources::current_model_receipt(
            &self.global_data_root,
            &run.binding,
        )
        .map_err(|error| mutating_error(map_generation_error(&error)))?;
        let next = crate::services::main_generation_sources::rebuild_binding_with_receipt(
            conn,
            &PathBuf::from(&self.session.project_path).join(".lore"),
            &self.global_data_root,
            &self.session.lease_id,
            &run.binding,
            &receipt,
        )
        .map_err(|error| mutating_error(map_generation_error(&error)))?;
        let lease = self.acquire_model_lease(&receipt.model_id)?;
        if crate::services::main_generation_sources::safe_model_receipt(&lease.receipt) != receipt {
            return Err(mutating_error("GENERATION_MODEL_CHANGED"));
        }
        let resumed = self
            .run_service
            .resume_with_authorization(conn, &run.run_id, next, |previous, current| {
                Ok(
                    crate::services::generation_source_binding::compare_generation_source_bindings(
                        previous, current,
                    ),
                )
            })
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        self.assert_current()?;
        lease.attach(self, &run.run_id);
        self.view_of(conn, &resumed)
    }

    /// 基线 `pause`（运行暂停，main-generation-owner.ts:1336）：
    /// budget root 状态迁移为 `paused`（`generation_roots` +
    /// `generation_runs` 双写，`reserved` 尝试标记
    /// `cancelled-before-dispatch`），返回最新运行视图。
    ///
    /// 本波次就位**完整状态迁移面**（逐字对齐基线）：运行身份
    /// 与执行准入（`require_run`，execution 语义，:1336）与
    /// `repository.pause(rootActionId)` 状态迁移（仓储层已逐字
    /// 移植，generation-run-repository.ts:641-657）。
    ///
    /// 命令层先通过已认证的独立控制登记唤醒进行中分发，再取得业务锁；
    /// 此处完成持久暂停与再次中止。网络结算后才迁移根状态，不能把
    /// 易失中止信号当作已经持久暂停的收据。
    pub fn pause(&self, conn: &Connection, handle: &Value) -> Result<Value, String> {
        let run = self.require_run(conn, handle, true)?;
        self.run_service
            .pause(conn, &run.root_action_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        let run = self
            .repository
            .get(conn, &run.run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        self.view_of(conn, &run)
    }

    /// 基线 `cancel`（运行取消，main-generation-owner.ts:1337）：
    /// budget root 状态迁移为 `cancelled`（`repository.pause` 的
    /// `cancel=true` 形态），返回最新运行视图。
    ///
    /// 本波次就位**完整状态迁移面**（逐字对齐基线）：运行身份
    /// 校验（`require_run`，非执行语义，:1337）与
    /// `repository.pause(rootActionId, true)` 状态迁移。
    ///
    /// 命令层的提前中止与暂停共用独立控制登记；此处仍验证运行身份，
    /// 完成持久取消并返回最终视图，不绕过数据库/会话门禁。
    pub fn cancel(&self, conn: &Connection, handle: &Value) -> Result<Value, String> {
        let run = self.require_run(conn, handle, false)?;
        self.run_service
            .cancel(conn, &run.root_action_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        let run = self
            .repository
            .get(conn, &run.run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        self.view_of(conn, &run)
    }

    /// 基线 `restart`（运行重启，main-generation-owner.ts:1338-1375）：
    /// 以新选择重新 `begin` 并处置旧 run（`service.cancel`）。
    ///
    /// 本波次就位**准入校验面**（逐字对齐基线）：运行身份校验
    /// （`require_run`，非执行语义，:1339）、旧 roster / 图生成
    /// 上下文拒绝（:1346-1349）→ `GENERATION_LEGACY_ADMISSION_REQUIRED`
    /// / `GENERATION_GRAPH_ADMISSION_REQUIRED`、重启 nonce 新鲜性
    /// （:1350）→ `GENERATION_RESTART_NEW_NONCE_REQUIRED`。
    ///
    /// ⚠️ 诚实化占位（B3 波次说明）：基线 :1344-1345 的子生成器
    /// 登记册可变性断言（imports / finalizations / graphs /
    /// legacyRosters，未迁移）、:1351-1364 的规划续作面
    /// （`planningContinuation` 推导 / `readPlanningContinuation`
    /// 幂等重放 / `recordPlanningContinuation` 持久化）、
    /// `begin(selection)`（主生成策略规划器，fail-closed）与
    /// `service.cancel` 的进行中分发中止尚未迁移——重启入口
    /// fail-closed（命令层收敛为 `GENERATION_REQUEST_FAILED`）。
    pub fn restart(
        &self,
        conn: &Connection,
        handle: &Value,
        selection: &Value,
    ) -> Result<Value, String> {
        let old = self.require_run(conn, handle, false)?;
        // 基线 :1346-1349：旧 roster / 图生成上下文的运行不可重启。
        let manifest = &old.binding.source_manifest;
        if manifest.get("legacyRosterContext").is_some() {
            return Err(mutating_error("GENERATION_LEGACY_ADMISSION_REQUIRED"));
        }
        if manifest.get("graphGenerationContext").is_some() {
            return Err(mutating_error("GENERATION_GRAPH_ADMISSION_REQUIRED"));
        }
        // 基线 :1350：重启选择必须携带新 nonce 且不得指定父根。
        let parent_root_action_id = selection
            .get("parentRootActionId")
            .cloned()
            .unwrap_or(Value::Null);
        let ui_action_nonce = selection
            .get("uiActionNonce")
            .cloned()
            .unwrap_or(Value::Null);
        let root_nonce = self
            .repository
            .budget(conn, &old.root_action_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?
            .root
            .ui_action_nonce;
        if !parent_root_action_id.is_null() || ui_action_nonce == Value::String(root_nonce) {
            return Err(mutating_error("GENERATION_RESTART_NEW_NONCE_REQUIRED"));
        }
        // ⚠️ B3 占位：基线 :1344-1345 的子生成器登记册可变性断言、
        // :1351-1364 的规划续作面、`begin(selection)` 重开与
        // `service.cancel` 旧 run 处置未迁移——重启入口 fail-closed
        // （命令层收敛为 `GENERATION_REQUEST_FAILED`）。
        Err(mutating_error(
            "主生成重启尚未迁移：主生成策略规划器（newMainGenerationPolicy）与生成运行服务（generation-run-service）待后续批次移植",
        ))
    }

    /// 基线 `discardCandidate`（候选丢弃，main-generation-owner.ts:1376-1389）：
    /// 候选制品标记 `discarded`（`generation_artifacts.status`），
    /// 返回最新运行视图。
    ///
    /// 本波次就位**完整状态迁移面**（逐字对齐基线）：运行身份
    /// 校验（`require_run`，非执行语义，:1377）、agent-round 操作
    /// 拒绝（:1378）→ `GENERATION_AGENT_CONTROL_IMMUTABLE`、候选
    /// 可丢弃性校验（:1385-1386，候选存在且非 `running`）→
    /// `GENERATION_CANDIDATE_NOT_DISCARDABLE`、`repository.
    /// discardCandidate(artifactId)` 状态迁移（仓储层已逐字移植，
    /// generation-run-repository.ts:701-705）。
    ///
    /// 特殊域可变性与易失回执接线仍待续；候选清理委托运行服务回收未保存尾。
    pub fn discard_candidate(
        &self,
        conn: &Connection,
        handle: &Value,
        artifact_id: &str,
    ) -> Result<Value, String> {
        let run = self.require_run(conn, handle, false)?;
        // 基线 :1378：agent-round 操作的候选不可控制。
        if run
            .binding
            .source_manifest
            .get("operation")
            .and_then(Value::as_str)
            == Some("agent-round")
        {
            return Err(mutating_error("GENERATION_AGENT_CONTROL_IMMUTABLE"));
        }
        // 基线 :1385-1386：候选必须存在且非进行中。
        let view = self.view_of(conn, &run)?;
        let discardable = view
            .get("candidates")
            .and_then(Value::as_array)
            .map(|candidates| {
                candidates.iter().any(|item| {
                    item.get("artifactId").and_then(Value::as_str) == Some(artifact_id)
                        && item.get("status").and_then(Value::as_str) != Some("running")
                })
            })
            .unwrap_or(false);
        if !discardable {
            return Err(mutating_error("GENERATION_CANDIDATE_NOT_DISCARDABLE"));
        }
        self.run_service
            .discard_candidate(conn, artifact_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        self.view_of(conn, &run)
    }

    /// 基线 `commitDraft`（main-generation-owner.ts:1292-1300）：
    /// 草稿提交——把运行当前可见合成正文落库为草稿行
    /// （`GenerationDraftEffects.commit`），返回提交回执
    /// （`GenerationDraftCommitReceipt`，契约
    /// `generation:commit-draft`，generation-owner-contract.ts:278）。
    ///
    /// 本波次就位**幂等重放面**（逐字对齐基线）：运行身份校验
    /// （`require_run`，非执行语义——基线 :1293 `requireRun`
    /// 单参调用，默认 `execution = false`，:138）→ 已存提交回执
    /// 读取（外层 `draftEffects.readCommit`，:1295；正文被改写时
    /// `GENERATION_DRAFT_RECEIPT_INVALID`）→ 目标校验（基线
    /// `GenerationDraftEffects.commit` :129-133：`source` 恒
    /// `'write'`、章节号 / 操作 / 批次身份与源清单一致，否则
    /// `GENERATION_DRAFT_TARGET_INVALID`）→ 已提交重放（:134-138：
    /// `expectedCompositionHash` 与已存回执一致 → 返回原回执子集
    /// `{ success, id, version, content, contentHash }`；不一致 →
    /// `GENERATION_DRAFT_COMMIT_CONFLICT`）。
    ///
    /// 新提交在同一 immediate 事务内校验真实源、创建草稿、写回收据
    /// 并记录代理子效应；任一步失败全部回滚。已提交重放不重新断言源。
    pub fn commit_draft(&self, conn: &Connection, request: &Value) -> Result<Value, String> {
        let handle = request.get("handle").cloned().unwrap_or(Value::Null);
        let run = self.require_run(conn, &handle, false)?;
        // 基线 :1295 外层重放分支判定：已存提交回执（正文被改写
        // 时 readCommit 抛 GENERATION_DRAFT_RECEIPT_INVALID）。
        let previous = GenerationDraftEffects.read_commit(conn, &self.repository, &run.run_id)?;
        let manifest = &run.binding.source_manifest;
        // 基线 commit :129-133 目标校验（`source` 恒 'write'、
        // 章节号 / 操作 / 批次身份与源清单一致；基线 `undefined`
        // 缺席语义与 Rust `None` 等价）。
        if request.get("source").and_then(Value::as_str) != Some("write")
            || request.get("chapterNumber").and_then(Value::as_i64)
                != manifest.get("chapterNumber").and_then(Value::as_i64)
            || manifest.get("operation").and_then(Value::as_str) != Some("chapter-draft")
            || request.get("batchId").and_then(Value::as_str)
                != manifest.get("batchId").and_then(Value::as_str)
        {
            return Err(mutating_error("GENERATION_DRAFT_TARGET_INVALID"));
        }
        if let Some(previous) = previous {
            // 基线 commit :134-138：幂等重放——期望哈希与已存
            // 回执一致时返回原回执子集（GenerationDraftCommitReceipt
            // 形状，契约 generation-owner-contract.ts:111-117），
            // 不一致 → GENERATION_DRAFT_COMMIT_CONFLICT。
            if previous.get("contentHash").and_then(Value::as_str)
                != request
                    .get("expectedCompositionHash")
                    .and_then(Value::as_str)
            {
                return Err(mutating_error("GENERATION_DRAFT_COMMIT_CONFLICT"));
            }
            return Ok(json!({
                "success": true,
                "id": previous.get("id").cloned().unwrap_or(Value::Null),
                "version": previous.get("version").cloned().unwrap_or(Value::Null),
                "contentHash": previous.get("contentHash").cloned().unwrap_or(Value::Null),
                "content": previous.get("content").cloned().unwrap_or(Value::Null),
            }));
        }
        // 事务覆盖服务构造、效应前后源投影与草稿/收据写入，不能提前提交草稿。
        let commit = |conn: &Connection| {
            let assert_current = || self.assert_current();
            let agent = self
                .agent_service(conn, &assert_current)
                .map_err(|error| mutating_error(&error))?;
            agent
                .with_child_effect(&handle, |conn| {
                    GenerationDraftEffects.commit_in_transaction(
                        conn,
                        &self.repository,
                        request,
                        || self.assert_sources_current(conn, &handle, None),
                    )
                })
                .map_err(|error| {
                    if error.starts_with("Error: ") {
                        error
                    } else {
                        mutating_error(map_generation_error(&error))
                    }
                })
        };
        if !conn.is_autocommit() {
            let savepoint = DraftCommitSavepoint::new(conn)?;
            let receipt = commit(conn)?;
            savepoint.commit()?;
            return Ok(receipt);
        }
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
        let receipt = commit(&tx)?;
        tx.commit()
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
        Ok(receipt)
    }

    /// 基线 `retryDraftShortOutline`（main-generation-owner.ts:619-624）：
    /// 短细纲失败重试——以 `draft-short-outline-retry:{failedAttemptId}`
    /// 调用 nonce 重放失败的短细纲 attempt（`execute` 的重试调用形态，
    /// 携带 `shortOutlineRetryAttemptId` 标志，契约
    /// `generation:retry-draft-short-outline`，
    /// generation-owner-contract.ts:279）。
    ///
    /// 本波次就位**运行身份前缀**（逐字对齐基线）：`require_run`
    /// （执行语义——基线 :620 `requireRun(request.handle, true)`）。
    ///
    /// ⚠️ B3 占位：基线 :621 的 `readFailedShortOutlineTask`
    /// （:157-178——失败 attempt 的任务形状 / 提示哈希 / 源一致性
    /// 重建校验，依赖未迁移的 `volatileReceipts` 易失回执缓存、
    /// `repository.receipt` 尝试回执投影与 `compareGenerationSourceBindings`
    /// 绑定比对）与 :622 的 `execute` 重试分发（LLM 分发循环未迁移，
    /// 见 [`Self::execute`]）未迁移——重试入口 fail-closed（命令层收敛为
    /// `GENERATION_REQUEST_FAILED`）。
    pub fn retry_draft_short_outline(
        &self,
        conn: &Connection,
        request: &Value,
    ) -> Result<Value, String> {
        let handle = request.get("handle").cloned().unwrap_or(Value::Null);
        self.require_run(conn, &handle, true)?;
        // ⚠️ B3 占位：readFailedShortOutlineTask 与 execute 重试分发
        // （见方法头注）未迁移——重试入口 fail-closed（命令层收敛为
        // GENERATION_REQUEST_FAILED）。
        Err(mutating_error(
            "主生成短细纲重试尚未迁移：失败任务重建（readFailedShortOutlineTask）与生成运行服务（generation-run-service）的 LLM 分发循环待后续批次移植",
        ))
    }

    /// 基线 `beginBatch`（main-generation-owner.ts:1279-1285）：
    /// 批量创作初始化——批量意图准入（`assertGenerationBatchIntent`，
    /// generation-draft-effects.ts:18-27，Rust 侧
    /// [`assert_generation_batch_intent`] 已迁移）后，以
    /// `batch-chapters` 操作 + 空选择集 + `batchIntent` 构造
    /// begin 选择（基线 :1281-1283），经 `begin`（主进程内部
    /// 初始化调用，`batchInitialization = true`，豁免
    /// `GENERATION_BATCH_INTENT_MAIN_ONLY`）创建批量根运行，
    /// 再经 `readBatch(run.handle.runId)` 返回批次进度快照。
    ///
    /// ⚠️ 执行依赖型：基线 :1282 的 `begin` 创建核心（策略构造器
    /// / 模型租约 / buildBinding / service.open）随 G4-6b-1 在
    /// 策略构造点 fail-closed——合法请求当前收到「尚未迁移」
    /// 拒绝而非批次进度（命令层收敛为 `GENERATION_REQUEST_FAILED`）；
    /// 意图准入（`GENERATION_BATCH_INTENT_INVALID`）与 begin
    /// 选择形状校验的错误码与基线逐字一致。
    pub fn begin_batch(&self, conn: &Connection, request: &Value) -> Result<Value, String> {
        // 基线 :1280 `assertGenerationBatchIntent(request)`。
        assert_generation_batch_intent(request)?;
        // 基线 :1281 `{ ...intent, operation: 'batch-chapters',
        // selectedDraftIds: [], selectedFinalizedDraftIds: [],
        // output: 'visible-text', batchIntent: { mode, range, targetUnits } }`
        // ——`intent` 为 request 剔除 mode / range / targetUnits 的
        // 剩余键（未知键随展开携带，由 begin 的 BEGIN_ALLOWED_KEYS
        // 白名单拦截，与基线 `...intent` 展开语义一致）。
        let mut selection = Map::new();
        if let Some(object) = request.as_object() {
            for (key, value) in object {
                if matches!(key.as_str(), "mode" | "range" | "targetUnits") {
                    continue;
                }
                selection.insert(key.clone(), value.clone());
            }
        }
        selection.insert(
            "operation".to_string(),
            Value::String("batch-chapters".to_string()),
        );
        selection.insert("selectedDraftIds".to_string(), Value::Array(Vec::new()));
        selection.insert(
            "selectedFinalizedDraftIds".to_string(),
            Value::Array(Vec::new()),
        );
        selection.insert(
            "output".to_string(),
            Value::String("visible-text".to_string()),
        );
        selection.insert(
            "batchIntent".to_string(),
            json!({
                "mode": request.get("mode").cloned().unwrap_or(Value::Null),
                "range": request.get("range").cloned().unwrap_or(Value::Null),
                "targetUnits": request.get("targetUnits").cloned().unwrap_or(Value::Null),
            }),
        );
        // 基线 :1282 `begin(..., true)`：主进程内部初始化调用。
        // 基线 :1282 `begin(..., true)`：主进程内部初始化调用
        // （agent 初始化标志 IPC 边界恒 false）。
        let run = self.begin(conn, &Value::Object(selection), true, false)?;
        // 基线 :1284 `readBatch(run.handle.runId)`（owner 闭包
        // :880：`assertCurrent()` 后 `draftEffects.readBatch`）。
        let run_id = run
            .get("handle")
            .and_then(|handle| handle.get("runId"))
            .and_then(Value::as_str)
            .ok_or_else(|| mutating_error("主生成运行创建尚未迁移：批量根运行视图缺少句柄身份"))?;
        GenerationDraftEffects.read_batch(conn, &self.repository, run_id, &self.session.project_id)
    }

    /// 基线 `confirmBatchFinalization`（main-generation-owner.ts:1287-1293）：
    /// 批次定稿确认——`readBatch(batchId)` 读取批次进度快照后，
    /// 校验目标章节存在 `finalizationId` 匹配且后处理完成
    /// （`postProcessComplete`）的已提交章，否则
    /// `GENERATION_BATCH_FINALIZATION_REQUIRED`；校验通过原样
    /// 返回批次进度（纯验证面，基线无写入）。
    pub fn confirm_batch_finalization(
        &self,
        conn: &Connection,
        batch_id: &str,
        chapter_number: i64,
        finalization_id: &str,
    ) -> Result<Value, String> {
        // 基线 :1288 `readBatch(request.batchId)`（owner 闭包
        // :880：`assertCurrent()` 后 `draftEffects.readBatch(batchId,
        // deps.projectId)`）。
        self.assert_current()?;
        let batch = GenerationDraftEffects.read_batch(
            conn,
            &self.repository,
            batch_id,
            &self.session.project_id,
        )?;
        // 基线 :1289-1291：completedChapters 中存在章节号 / 定稿
        // id 匹配且后处理完成的已提交章（基线 `undefined` 缺席
        // 语义与 Rust `None` 等价，不匹配）。
        let confirmed = batch
            .get("completedChapters")
            .and_then(Value::as_array)
            .is_some_and(|chapters| {
                chapters.iter().any(|chapter| {
                    chapter.get("chapterNumber").and_then(Value::as_i64) == Some(chapter_number)
                        && chapter.get("finalizationId").and_then(Value::as_str)
                            == Some(finalization_id)
                        && chapter.get("postProcessComplete").and_then(Value::as_bool) == Some(true)
                })
            });
        if !confirmed {
            return Err(mutating_error("GENERATION_BATCH_FINALIZATION_REQUIRED"));
        }
        Ok(batch)
    }

    /// 基线 `preparedKnowledge(preparationId)`
    /// （main-generation-owner.ts:309-314）：按
    /// `preparationId` 读取准备缓存的知识快照（基线
    /// `structuredClone` 深拷贝等价物为 `Value::clone`）；
    /// 未命中 → `GENERATION_DRAFT_PREPARATION_REQUIRED`
    /// （`generation:begin` 的 chapter-draft 准入依赖，
    /// generation-controller.ts:177）。
    pub fn prepared_knowledge(&self, preparation_id: &str) -> Result<Value, String> {
        self.assert_current()?;
        self.preparations
            .iter()
            .find(|(id, _)| id == preparation_id)
            .map(|(_, entry)| entry.knowledge_snapshot.clone())
            .ok_or_else(|| mutating_error("GENERATION_DRAFT_PREPARATION_REQUIRED"))
    }

    /// 基线 `draftPreparationBinding(request, knowledgeSnapshot?)`
    /// （main-generation-owner.ts:321-335）：草稿上下文
    /// 准备的绑定构造（`generation:prepare-draft-context`
    /// 命令的第一步，controller :167 的 `baseline`）。
    ///
    /// 选择形状与查询一致性校验后，从同一模型快照构造只读证据，
    /// 读取批次根策略或计算独立策略，再冻结六章蓝图窗口与知识快照。
    /// 不创建运行、不签发可执行租约；知识快照采集仍由命令层负责。
    pub fn draft_preparation_binding(
        &self,
        conn: &Connection,
        request: &Value,
        knowledge_snapshot: Option<&Value>,
    ) -> Result<RunBinding, String> {
        self.assert_current()?;
        // 基线 :322-324 `GENERATION_DRAFT_PREPARATION_INVALID`
        // 校验面（`!request || typeof request.query !== 'string'
        // || knowledgeSnapshot && request.query !== knowledgeSnapshot.query
        // || 键集超白名单`）。
        let invalid = request.as_object().is_none()
            || request.get("query").and_then(Value::as_str).is_none()
            || knowledge_snapshot
                .is_some_and(|snapshot| snapshot.get("query") != request.get("query"))
            || request.as_object().is_some_and(|map| {
                map.keys()
                    .any(|key| !PREPARATION_ALLOWED_KEYS.contains(&key.as_str()))
            });
        if invalid {
            return Err(mutating_error("GENERATION_DRAFT_PREPARATION_INVALID"));
        }
        let model_id = request
            .get("modelId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let model =
            crate::services::main_generation_sources::load_model(&self.global_data_root, model_id)
                .map_err(|error| mutating_error(map_generation_error(&error)))?;
        let receipt = crate::services::main_generation_sources::model_read_only_receipt(&model)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        // 基线 :328-331：六章蓝图窗口及知识快照，不保存运行、不签发可执行租约。
        let selection = {
            let mut selection = Map::new();
            selection.insert(
                "operation".to_string(),
                Value::String("chapter-draft".to_string()),
            );
            selection.insert(
                "uiActionNonce".to_string(),
                Value::String("prepare-only".to_string()),
            );
            selection.insert(
                "modelId".to_string(),
                request.get("modelId").cloned().unwrap_or(Value::Null),
            );
            selection.insert(
                "chapterNumber".to_string(),
                request.get("chapterNumber").cloned().unwrap_or(Value::Null),
            );
            selection.insert(
                "selectedDraftIds".to_string(),
                request
                    .get("selectedDraftIds")
                    .cloned()
                    .unwrap_or(Value::Array(Vec::new())),
            );
            selection.insert(
                "selectedFinalizedDraftIds".to_string(),
                Value::Array(Vec::new()),
            );
            // 基线 `Array.from({ length: 6 }, (_, index) =>
            // request.chapterNumber + index)`——6 章蓝图窗口
            // （chapterNumber..chapterNumber+5）。
            let blueprint_window: Vec<Value> = (0..6)
                .map(|index| {
                    request
                        .get("chapterNumber")
                        .and_then(Value::as_i64)
                        .map_or(Value::Null, |base| Value::Number((base + index).into()))
                })
                .collect();
            selection.insert(
                "selectedBlueprintChapterNumbers".to_string(),
                Value::Array(blueprint_window),
            );
            selection.insert(
                "promptKeys".to_string(),
                request
                    .get("promptKeys")
                    .cloned()
                    .unwrap_or(Value::Array(Vec::new())),
            );
            selection.insert(
                "skillStages".to_string(),
                request
                    .get("skillStages")
                    .cloned()
                    .unwrap_or(Value::Array(Vec::new())),
            );
            selection.insert(
                "authorInputs".to_string(),
                request
                    .get("authorInputs")
                    .cloned()
                    .unwrap_or(Value::Array(Vec::new())),
            );
            selection.insert(
                "output".to_string(),
                Value::String("visible-text".to_string()),
            );
            if let Some(batch_id) = request.get("batchId") {
                selection.insert("batchId".to_string(), batch_id.clone());
            }
            if let Some(snapshot) = knowledge_snapshot {
                selection.insert("knowledgeSnapshot".to_string(), snapshot.clone());
            }
            Value::Object(selection)
        };
        let policy = if let Some(batch_id) = request.get("batchId").and_then(Value::as_str) {
            let batch = GenerationDraftEffects.read_batch(
                conn,
                &self.repository,
                batch_id,
                &self.session.project_id,
            )?;
            let root_id = batch
                .pointer("/rootHandle/rootActionId")
                .and_then(Value::as_str)
                .ok_or_else(|| mutating_error("GENERATION_BATCH_INVALID"))?;
            self.root_policy(conn, root_id)?
        } else {
            crate::services::main_generation_plan::new_main_generation_policy(&selection, &model)
                .map_err(|error| mutating_error(map_generation_error(&error)))?
                .to_value()
        };
        self.build_binding(conn, &selection, &receipt, &policy, &json!("visible-text"))
    }

    /// 基线 `prepareDraftContext(request, knowledgeSnapshot,
    /// beforeKnowledgeRead?)`（main-generation-owner.ts:336-354）：
    /// 草稿上下文准备——事务内构造绑定并与准备前快照比对、
    /// 读取选中草稿正文、签发 `preparationId` 并冻结进
    /// LRU 准备缓存（容量 64）。
    ///
    /// 本波次就位**验证面**（基线 :339-353 逐字平移）：
    /// - `beforeKnowledgeRead` 校验（:339-340）：
    ///   `preparationMatches(before, binding, true)` 不匹配
    ///   → `GENERATION_DRAFT_PREPARATION_CHANGED`（辅助
    ///   [`preparation_matches`]）；
    /// - selectedDrafts 读取（:342-346）：`drafts` JOIN
    ///   `contents`，`contentHash` 为正文 SHA-256 hex
    ///   （[`text_hash`]，形态对齐 generation_source_binding
    ///   G4-6a 的 `hash`）；
    /// - `preparationId` 签发（基线 `randomUUID` 等价物
    ///   [`crate::project_access::random_uuid_v4`]）与 LRU
    ///   缓存（:348-351，容量 [`PREPARATION_CAPACITY`]，
    ///   超限逐出最旧条目）。
    ///
    /// ⚠️ 执行依赖（本波次既定 fail-closed）：事务内首步
    /// `draftPreparationBinding`（基线 :338）在模型加载 /
    /// 策略规划器构造点 fail-closed（见
    /// [`Self::draft_preparation_binding`] 头注），故绑定
    /// 构造核心未迁移前的合法请求在事务入口即被拒绝；
    /// KB 知识快照捕获在命令层 fail-closed（见
    /// `commands::generation_prepare_draft_context` 头注），
    /// `beforeKnowledgeRead` 恒为 `None`（IPC 边界）。
    pub fn prepare_draft_context(
        &mut self,
        conn: &Connection,
        request: &Value,
        knowledge_snapshot: &Value,
        before_knowledge_read: Option<&RunBinding>,
    ) -> Result<Value, String> {
        // 基线 :337 `deps.database.transaction(() => {...})()`：
        // Tauri 侧命令层在 `project_access` 门禁内传入单一
        // 项目库连接，事务原子性由调用点保证（与既有 owner
        // 事务方法同范式）。
        let binding = self.draft_preparation_binding(conn, request, Some(knowledge_snapshot))?;
        // 基线 :339-340 `beforeKnowledgeRead` 校验。
        if before_knowledge_read.is_some_and(|before| !preparation_matches(before, &binding, true))
        {
            return Err(mutating_error("GENERATION_DRAFT_PREPARATION_CHANGED"));
        }
        // 基线 :342-346 selectedDrafts 读取。
        let selected_drafts = self.read_preparation_drafts(conn, request)?;
        // 基线 :348-351 `preparationId` 签发与 LRU 缓存
        // （容量 64，超限逐出最旧条目）。
        let preparation_id = crate::project_access::random_uuid_v4();
        if self.preparations.len() >= PREPARATION_CAPACITY {
            if let Some(oldest) = self.preparations.first().map(|(id, _)| id.clone()) {
                self.preparations.retain(|(id, _)| *id != oldest);
            }
        }
        self.preparations.push((
            preparation_id.clone(),
            PreparationEntry {
                binding,
                knowledge_snapshot: knowledge_snapshot.clone(),
            },
        ));
        // 基线 :353 `structuredClone(knowledgeSnapshot)`
        // 等价物：`Value::clone` 深拷贝。
        Ok(json!({
            "preparationId": preparation_id,
            "knowledgeSnapshot": knowledge_snapshot.clone(),
            "selectedDrafts": selected_drafts,
        }))
    }

    /// 基线 :342-346 selectedDrafts 读取（`prepareDraftContext`
    /// 事务内）：`drafts` JOIN `contents` 取章节号 / 版本 /
    /// 正文，`contentHash` 为正文 SHA-256 hex。缺失草稿
    /// 的基线行为：`row.chapter_number` 抛 `TypeError`，
    /// 经基线 register catch 收敛为 `GENERATION_REQUEST_FAILED`。
    fn read_preparation_drafts(
        &self,
        conn: &Connection,
        request: &Value,
    ) -> Result<Vec<Value>, String> {
        let draft_ids = request
            .get("selectedDraftIds")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        draft_ids
            .iter()
            .map(|draft_id| {
                // 基线 `deps.database.prepare(...).get(draftId)`
                // ——非数字 draftId 抛 `TypeError`，经基线
                // register catch 收敛为 `GENERATION_REQUEST_FAILED`。
                let draft_id = draft_id
                    .as_i64()
                    .ok_or_else(|| mutating_error(GENERATION_REQUEST_FAILED))?;
                let row = conn
                    .query_row(
                        "SELECT d.chapter_number, d.version, c.body FROM drafts d JOIN contents c ON c.id = d.content_id WHERE d.id=?1",
                        [draft_id],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, i64>(1)?,
                                row.get::<_, String>(2)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
                let Some((chapter_number, version, body)) = row else {
                    return Err(mutating_error(GENERATION_REQUEST_FAILED));
                };
                Ok(json!({
                    "draftId": draft_id,
                    "chapterNumber": chapter_number,
                    "version": version,
                    "contentHash": text_hash(&body),
                    "content": body,
                }))
            })
            .collect()
    }

    /// 基线 `assertSourcesCurrent(handle, blueprintRange?)`
    /// （main-generation-owner.ts:662-678）：生成源当前性断言。基线经
    /// `generation-controller.ts:40-47` 的 `assertGenerationSourcesCurrent`
    /// 在事务内生效前调用（`db:blueprint-commit-range` 的提交闭包与
    /// `db:character-roster-commit` 的名单提交钩子），失败即中断提交。
    ///
    /// 验证面（逐字平移）：
    /// 1. `requireRun(handle, true)` —— 运行身份 / 可写性 / 策略比对；
    /// 2. 根预算已取消 → `GENERATION_ACTION_CANCELLED`；
    /// 3. `blueprint_range` 存在时：
    ///    - 目录续接运行（`listDirectoryProgress` 中
    ///      `continuationHandle.runId === run.runId`）的 `remainingRange`
    ///      与断言范围不一致 → `GENERATION_DIRECTORY_CONTINUATION_RANGE_CHANGED`；
    ///    - `selectedBlueprintChapterNumbers` 缺失 / 范围非安全整数 /
    ///      `startChapter < 1` / `endChapter < startChapter` / 跨度 ≥ 10000 /
    ///      范围内任一章未选中 → `GENERATION_FORMAL_RANGE_NOT_SELECTED`；
    /// 4. 实时模型只读证据与源绑定深比较；漂移 → GENERATION_SOURCE_CHANGED。
    pub fn assert_sources_current(
        &self,
        conn: &Connection,
        handle: &Value,
        blueprint_range: Option<&ChapterRange>,
    ) -> Result<(), String> {
        let run = self.require_run(conn, handle, true)?;
        if self
            .repository
            .budget(conn, &run.root_action_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?
            .root
            .status
            == "cancelled"
        {
            return Err(mutating_error("GENERATION_ACTION_CANCELLED"));
        }
        let Some(range) = blueprint_range else {
            // 基线无 blueprintRange 时跳过第 3 步（roster 提交钩子形态）。
            return self.assert_binding_current(conn, &run.binding);
        };
        // 基线 :665-666 —— 目录续接运行的剩余范围一致性
        let continued = self
            .repository
            .list_directory_progress(conn)
            .map_err(|error| mutating_error(map_generation_error(&error)))?
            .into_iter()
            .find(|item| {
                item.continuation_handle
                    .as_ref()
                    .is_some_and(|continuation| continuation.run_id == run.run_id)
            });
        if let Some(item) = continued {
            if item.remaining_range.as_ref() != Some(range) {
                return Err(mutating_error(
                    "GENERATION_DIRECTORY_CONTINUATION_RANGE_CHANGED",
                ));
            }
        }
        // 基线 :667-674 —— 正式范围选中校验（`as number[] | undefined`
        // 强转的等价物：非整数数组（含缺失）按未选中处理）
        let selected = run
            .binding
            .source_manifest
            .get("selectedBlueprintChapterNumbers")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(Value::as_i64)
                    .collect::<Option<Vec<i64>>>()
            })
            .flatten();
        let formal_range_selected = selected.is_some()
            && is_safe_integer_value(range.start_chapter)
            && is_safe_integer_value(range.end_chapter)
            && range.start_chapter >= 1
            && range.end_chapter >= range.start_chapter
            && range.end_chapter - range.start_chapter < 10000
            && !(range.start_chapter..=range.end_chapter)
                .any(|chapter| !selected.as_deref().unwrap_or_default().contains(&chapter));
        if !formal_range_selected {
            return Err(mutating_error("GENERATION_FORMAL_RANGE_NOT_SELECTED"));
        }
        self.assert_binding_current(conn, &run.binding)
    }

    /// 构造角色提案服务（对齐基线 main-generation-owner.ts:880-896
    /// `characters` 依赖）：prove 闭包内 `assertCurrent` +
    /// `proveCharacterProposal`，其 `assertSources` 闭包做
    /// `requireRun` + budget 取消态检查 →
    /// `CHARACTER_PROPOSAL_SOURCE_CHANGED`。
    ///
    /// ⚠️ 基线 :676-677 的 `rebuildBinding(run.binding,
    /// currentModelReceipt(run.binding))` 深比较 —— fail-closed
    /// 占位（`currentModelReceipt → loadModel` 依赖未迁移，
    /// G4-6c-1 既定范式）：闭包内保守判定为一致。
    fn character_proposal_service<'a>(
        &'a self,
        conn: &'a Connection,
    ) -> CharacterProposalService<'a> {
        let project_id = self.session.project_id.clone();
        CharacterProposalService::new(
            conn,
            &self.session.project_id,
            move |source, for_write, version| {
                self.assert_current()?;
                prove_character_proposal(
                    conn,
                    &self.repository,
                    &project_id,
                    source,
                    for_write,
                    |handle, _committed_blueprint_chapters| {
                        let run = self.require_run(conn, handle, false)?;
                        if self
                            .repository
                            .budget(conn, &run.root_action_id)
                            .map_err(|error| mutating_error(map_generation_error(&error)))?
                            .root
                            .status
                            == "cancelled"
                        {
                            return Err(mutating_error("CHARACTER_PROPOSAL_SOURCE_CHANGED"));
                        }
                        // 基线 :676-677 rebuildBinding 深比较 —— fail-closed 占位（见方法头注）。
                        Ok(())
                    },
                    version,
                )
            },
        )
    }

    /// 构造旧名册生成恢复视图（对齐基线 main-generation-owner.ts:953-961
    /// `legacyRosterRecovery`）：`view` / `modelId` / `context` /
    /// `sourceStatus`（`assertFinalizationSources` 等价物——
    /// `assert_sources_current` 无 blueprintRange 形态）/
    /// `attemptCount`，条件展开 `artifact` / `proposal`。
    fn legacy_roster_recovery(
        &self,
        conn: &Connection,
        run: &DurableGenerationRun,
        proposal: Option<Value>,
        artifact: Option<Value>,
    ) -> Result<Value, String> {
        self.assert_current()?;
        let context = read_legacy_roster_generation_context(run)?;
        // 基线 :956-958 assertFinalizationSources(run) 等价物：
        // assert_sources_current 无 blueprintRange 时（roster 提交
        // 钩子形态）跳过范围检查，仅做身份校验 + budget 取消态检查。
        let handle = handle_of(run);
        let current = self.assert_sources_current(conn, &handle, None).is_ok();
        // 基线 :959 modelReceipt(run.binding).modelId
        let model_id = run
            .binding
            .source_manifest
            .get("modelReceipt")
            .and_then(|receipt| receipt.get("modelId"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        // 基线 :960 attemptCount
        let attempt_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM generation_attempts WHERE run_id=?1",
                [run.run_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
        let view = self.view_of(conn, run)?;
        let mut recovery = json!({
            "view": view,
            "modelId": model_id,
            "context": context,
            "sourceStatus": if current { "current" } else { "conflict" },
            "attemptCount": attempt_count,
        });
        // 基线 :961 条件展开 ...(artifact ? {artifact} : {}) / ...(proposal ? {proposal} : {})
        if let Some(artifact) = artifact {
            recovery["artifact"] = artifact;
        }
        if let Some(proposal) = proposal {
            recovery["proposal"] = proposal;
        }
        Ok(recovery)
    }

    /// 基线 `readLegacyRosterSource`（main-generation-owner.ts:963）：
    /// `assertCurrent` → `readLegacyRosterSource(deps.database)`。
    pub fn read_legacy_roster_source_cmd(&self, conn: &Connection) -> Result<Value, String> {
        self.assert_current()?;
        let source = read_legacy_roster_source(conn)?;
        serde_json::to_value(source)
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))
    }

    /// 基线 `adoptLegacyCards`（main-generation-owner.ts:964）：
    /// `assertCurrent` → `adoptLegacyCards(deps.database, request)`。
    pub fn adopt_legacy_cards_cmd(
        &self,
        conn: &Connection,
        request: &AdoptLegacyCardsRequest,
    ) -> Result<Value, String> {
        self.assert_current()?;
        let receipt = adopt_legacy_cards(conn, request)?;
        serde_json::to_value(receipt)
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))
    }

    /// 基线 `beginLegacyRosterGeneration`（main-generation-owner.ts:965-973）：
    /// 请求校验（modelId / uiActionNonce 键集 + 非空 nonce ≤256）→
    /// 幂等查询（projectId + uiActionNonce + legacyRosterContext）→
    /// `GENERATION_NONCE_CONFLICT` → 命中即 `legacyRosterRecovery` 重放；
    /// 否则内部 `begin(..., legacyInitialization=true)` 创建运行。
    ///
    /// ⚠️ 诚实化占位：基线 :972-973 的运行创建核心（主生成策略
    /// 规划器 `newMainGenerationPolicy` + 生成运行服务
    /// `generation-run-service`）未迁移（B3）——Tauri `owner.begin`
    /// 对 `legacy-character-roster-repair` 操作硬拒绝
    /// `GENERATION_LEGACY_ADMISSION_REQUIRED`（对齐基线 IPC 边界
    /// 行为，begin 方法头注）。本命令 fail-closed 于创建入口
    /// （`GENERATION_LEGACY_ROSTER_BEGIN_UNMIGRATED` 为迁移占位码、
    /// 非基线码），待创建核心迁移波次活化。
    pub fn begin_legacy_roster_generation(
        &self,
        conn: &Connection,
        request: &Value,
    ) -> Result<Value, String> {
        self.assert_current()?;
        // 基线 :967-970 请求校验（modelId / uiActionNonce 键集 + nonce 形态）
        if !keys_subset(request, &["modelId", "uiActionNonce"]) {
            return Err(mutating_error("GENERATION_LEGACY_REQUEST_INVALID"));
        }
        let ui_action_nonce = request
            .get("uiActionNonce")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if ui_action_nonce.trim().is_empty() || ui_action_nonce.chars().count() > 256 {
            return Err(mutating_error("GENERATION_LEGACY_REQUEST_INVALID"));
        }
        // 基线 :971-972 幂等查询（projectId + uiActionNonce + legacyRosterContext）。
        // ⚠️ JSON1 扩展（`json_extract`）在 rusqlite bundled 下可用性未验证，
        // 改用 Rust 端解析 action_json / binding_json 过滤（语义逐字对齐，
        // legacy roster 运行数量少，全量扫描无性能顾虑）。
        let candidates: Vec<(String, String, String)> = conn
            .prepare(
                "SELECT r.run_id, g.action_json, r.binding_json FROM generation_runs r \
                 JOIN generation_roots g ON g.root_action_id = r.root_action_id",
            )
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?
            .collect::<Result<Vec<(String, String, String)>, _>>()
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
        let mut matched: Vec<String> = Vec::new();
        for (run_id, action_json, binding_json) in candidates {
            let action = serde_json::from_str::<Value>(&action_json).unwrap_or(Value::Null);
            let binding = serde_json::from_str::<Value>(&binding_json).unwrap_or(Value::Null);
            if action.get("projectId").and_then(Value::as_str)
                == Some(self.session.project_id.as_str())
                && action.get("uiActionNonce").and_then(Value::as_str) == Some(ui_action_nonce)
                && binding
                    .get("sourceManifest")
                    .and_then(|manifest| manifest.get("legacyRosterContext"))
                    .is_some()
            {
                matched.push(run_id);
            }
        }
        if matched.len() > 1 {
            return Err(mutating_error("GENERATION_NONCE_CONFLICT"));
        }
        if let Some(run_id) = matched.first() {
            // 基线 :972 幂等重放：legacyRosterRecovery(repository.get(run_id))
            // ——只读路径，不依赖运行创建核心。
            let proposals = self.character_proposal_service(conn);
            let repository = self.repository.clone();
            let project_id = self.session.project_id.clone();
            let roster = LegacyRosterGeneration::new(conn, repository, &project_id, &proposals);
            let run = self
                .repository
                .get(conn, run_id)
                .map_err(|error| mutating_error(map_generation_error(&error)))?;
            let proposal = roster.proposal(&run)?;
            let artifact = roster.artifact_for(&run)?;
            return self.legacy_roster_recovery(conn, &run, proposal, artifact);
        }
        // 基线 :972-973 运行创建核心（主生成策略规划器 `newMainGenerationPolicy`
        // + 生成运行服务 `generation-run-service`）未迁移（B3）——fail-closed
        // 于创建入口（占位码，见方法头注）。
        Err(mutating_error("GENERATION_LEGACY_ROSTER_BEGIN_UNMIGRATED"))
    }

    /// 基线 `readLegacyRosterGeneration`（main-generation-owner.ts:976-980）：
    /// 请求校验（handle 键集）→ `legacyRosters.require(handle)` 身份
    /// 校验 → `legacyRosterRecovery` 恢复视图。
    pub fn read_legacy_roster_generation(
        &self,
        conn: &Connection,
        request: &Value,
    ) -> Result<Value, String> {
        self.assert_current()?;
        // 基线 :978-979 请求校验（handle 键集）
        if !keys_subset(request, &["handle"]) {
            return Err(mutating_error("GENERATION_LEGACY_REQUEST_INVALID"));
        }
        let handle = request.get("handle").cloned().unwrap_or(Value::Null);
        // 基线 :880-896 characters + legacyRosters 依赖构造
        let proposals = self.character_proposal_service(conn);
        let repository = self.repository.clone();
        let project_id = self.session.project_id.clone();
        let roster = LegacyRosterGeneration::new(conn, repository, &project_id, &proposals);
        // 基线 :980 legacyRosters.require(request.handle) → legacyRosterRecovery
        let run = roster.require(&handle)?;
        let proposal = roster.proposal(&run)?;
        let artifact = roster.artifact_for(&run)?;
        self.legacy_roster_recovery(conn, &run, proposal, artifact)
    }

    /// 基线 `executeLegacyRosterGeneration`（main-generation-owner.ts:981-1021）：
    /// 请求校验（handle 键集）→ `legacyRosters.require(handle)` →
    /// 双阶段 LLM 执行循环（initial / repair 各至多 3 次尝试，
    /// `buildLegacyRosterJsonRepairTask` 修复重试）→ 产物解析 →
    /// `generation_attempts.usage_receipt_json` 回执写入。
    ///
    /// ⚠️ 诚实化占位：基线 :988-1019 的双阶段 LLM 执行循环依赖
    /// 主生成分发核心（`service.execute` 的 LLM 流式分发 +
    /// `generation:snapshot` / `generation:reasoning` 事件发射，
    /// B3 未迁移）——本命令 fail-closed 于执行入口
    /// （`GENERATION_LEGACY_ROSTER_EXECUTE_UNMIGRATED` 为迁移占位码、
    /// 非基线码），待 LLM 执行循环迁移波次活化。
    pub fn execute_legacy_roster_generation(
        &self,
        conn: &Connection,
        request: &Value,
    ) -> Result<Value, String> {
        self.assert_current()?;
        // 基线 :983-984 请求校验（handle 键集）
        if !keys_subset(request, &["handle"]) {
            return Err(mutating_error("GENERATION_LEGACY_REQUEST_INVALID"));
        }
        let handle = request.get("handle").cloned().unwrap_or(Value::Null);
        // 基线 :880-896 characters + legacyRosters 依赖构造
        let proposals = self.character_proposal_service(conn);
        let repository = self.repository.clone();
        let project_id = self.session.project_id.clone();
        let roster = LegacyRosterGeneration::new(conn, repository, &project_id, &proposals);
        // 基线 :985 legacyRosters.require(request.handle) 身份校验（就位）
        roster.require(&handle)?;
        // 基线 :988-1019 双阶段 LLM 执行循环依赖主生成分发核心
        // （service.execute 的 LLM 流式分发 + 事件发射，B3 未迁移）
        // —— fail-closed 于执行入口（占位码，见方法头注）。
        Err(mutating_error(
            "GENERATION_LEGACY_ROSTER_EXECUTE_UNMIGRATED",
        ))
    }

    /// 基线 `stageLegacyRosterGeneration`（main-generation-owner.ts:1023-1027）：
    /// 请求校验（handle + artifact 键集）→ `legacyRosters.stage(handle,
    /// artifact, assertFinalizationSources)` 阶段提交（幂等重放 /
    /// artifact 校验 → proof → assertSources → 提案阶段提交 →
    /// 效应写回 usage_receipt_json）。
    pub fn stage_legacy_roster_generation(
        &self,
        conn: &Connection,
        request: &Value,
    ) -> Result<Value, String> {
        self.assert_current()?;
        // 基线 :1025-1026 请求校验（handle + artifact 键集）
        if !keys_subset(request, &["handle", "artifact"]) {
            return Err(mutating_error("GENERATION_LEGACY_REQUEST_INVALID"));
        }
        let handle = request.get("handle").cloned().unwrap_or(Value::Null);
        let artifact = request.get("artifact").cloned().unwrap_or(Value::Null);
        // 基线 :880-896 characters + legacyRosters 依赖构造
        let proposals = self.character_proposal_service(conn);
        let repository = self.repository.clone();
        let project_id = self.session.project_id.clone();
        let roster = LegacyRosterGeneration::new(conn, repository, &project_id, &proposals);
        // 基线 :1027 legacyRosters.stage(handle, artifact, assertFinalizationSources)
        // —— assertFinalizationSources 等价物：assert_sources_current 无
        // blueprintRange 形态（roster 提交钩子，跳过范围检查）。
        roster.stage(&handle, &artifact, &|run| {
            let handle = handle_of(run);
            self.assert_sources_current(conn, &handle, None)
        })
    }

    /// 基线 `cancelLegacyRosterGeneration`（main-generation-owner.ts:1028-1032）：
    /// 请求校验（handle 键集）→ `legacyRosters.require(handle)` 身份
    /// 校验 → `service.cancel(run.rootActionId)`（`repository.pause(cancel=true)`
    /// 等价物）→ `viewOf` 运行视图。
    pub fn cancel_legacy_roster_generation(
        &self,
        conn: &Connection,
        request: &Value,
    ) -> Result<Value, String> {
        self.assert_current()?;
        // 基线 :1030-1031 请求校验（handle 键集）
        if !keys_subset(request, &["handle"]) {
            return Err(mutating_error("GENERATION_LEGACY_REQUEST_INVALID"));
        }
        let handle = request.get("handle").cloned().unwrap_or(Value::Null);
        // 基线 :880-896 characters + legacyRosters 依赖构造（require 需要）
        let proposals = self.character_proposal_service(conn);
        let repository = self.repository.clone();
        let project_id = self.session.project_id.clone();
        let roster = LegacyRosterGeneration::new(conn, repository, &project_id, &proposals);
        // 基线 :1032 legacyRosters.require(request.handle); service.cancel(run.rootActionId); return viewOf(...)
        let run = roster.require(&handle)?;
        self.repository
            .pause(conn, &run.root_action_id, true)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        let run = self
            .repository
            .get(conn, &run.run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        self.view_of(conn, &run)
    }

    /// 基线 `withGenerationAgentChildEffect`（generation-controller.ts:66-71
    /// → main-generation-owner.ts:1301 → agents.withChildEffect）的接线。
    ///
    /// 本波次活化（G4-10d，占位翻转 fail-closed → 真实执行）：
    /// 基线 :1301 首行 `requireRun(handle)`（非执行语义——
    /// 默认 `execution = false`，main-generation-owner.ts:138）
    /// → 方法级构造 `AgentGeneration`（对齐 [`Self::admit_workflow`]
    /// 先例）→ 委托服务层 `with_child_effect`（agent_generation.rs:1795，
    /// 已逐字移植基线 agent-generation.ts:343-367：运行定位 +
    /// 句柄 deep-equal 身份校验 / 无 agentWorkflowRegistrationId
    /// 透传 / registration childHandles·rootActionId·sourcesCurrent
    /// 三重校验 / 会话轮次 capture-before → 效应 → capture-after
    /// 的 agentSourceTransition 回写）。服务层错误码与效应闭包
    /// 错误经前缀感知折叠为 MUTATING 信封（对齐 `commands::db`
    /// `blueprint_commit_range_result` :818 先例：已带 "Error: "
    /// 前缀的错误——如 `require_run` 透传与效应闭包内已包装的
    /// 仓储错误——原样透传，避免双重前缀）。
    ///
    /// 基线首行事务门禁（`if (!this.db.inTransaction) throw
    /// GENERATION_AGENT_TRANSACTION_REQUIRED`，agent-generation.ts:345）
    /// 的 Tauri 等价物为**调用方结构保证**：`db:project-core-commit-generated`
    /// 路径在本波实现中于外层事务内调用；`db:blueprint-commit-range`
    /// 路径的效应（`blueprints::commit_range`）自开事务、提交原子性
    /// 自足。rusqlite 0.40.2 无 `is_in_transaction()` API
    /// （`is_autocommit()` 对「效应自开事务」路径为误报），故方法内
    /// 不重复检查。
    ///
    /// AgentHost current/capture 使用真实模型收据与源重建；子代效应前后
    /// 的源过渡由 AgentGeneration 在调用方事务内验证并冻结，不以旧 binding 冒充当前源。
    pub fn with_agent_child_effect<T>(
        &self,
        conn: &Connection,
        handle: &Value,
        effect: impl FnOnce() -> T,
    ) -> Result<T, String> {
        // 基线 :1301 首行 `requireRun(handle)`（非执行语义——
        // 默认 `execution = false`，main-generation-owner.ts:138；
        // 错误已是 MUTATING 信封形态，经 `?` 原样透传）。
        self.require_run(conn, handle, false)?;
        // 基线 :1301 `agents.withChildEffect(handle, effect)`——
        // 方法级借用视图共用 owner 的恢复标记，不重分类当前活跃认领。
        let assert_current = || self.assert_current();
        let agent = self
            .agent_service(conn, &assert_current)
            .map_err(|error| mutating_error(&error))?;
        // 效应委托服务层。错误经前缀感知折叠为 MUTATING 信封
        // （对齐 `commands::db::blueprint_commit_range_result`
        // :818 先例：已带 "Error: " 前缀的错误——如效应闭包内
        // 已包装的仓储错误——原样透传，避免双重前缀）。
        agent
            .with_child_effect(handle, |_conn| Ok(effect()))
            .map_err(|error| {
                if error.starts_with("Error: ") {
                    error
                } else {
                    mutating_error(&error)
                }
            })
    }

    // =====================================================================
    // 批次 G4-9a：角色提案 owner 方法（平移自
    // electron/services/main-generation-owner.ts 的 owner 接线
    // + electron/controllers/generation-controller.ts:181-190 的
    // register 项；服务层见 services/character_proposal.rs）
    // =====================================================================

    /// 服务层结果 → MUTATING 信封（**前缀感知**，对齐
    /// `commands::db::blueprint_commit_range_result` 的 G4-7 先例）：
    /// 服务层裸码（如 `CHARACTER_APPROVAL_INVALID`）按基线
    /// generation-controller.ts:160-162 的 catch 映射
    /// （[`map_generation_error`]，`CHARACTER` 前缀在透传正则内）
    /// 后补 `Error: ` 前缀；已带前缀的错误（prove 闭包 /
    /// `assert_current` / `with_agent_child_effect` / 仓储访问点的
    /// MUTATING 信封）原样透传，避免双重前缀。
    fn character_proposal_result(result: Result<Value, String>) -> Result<Value, String> {
        result.map_err(|error| {
            if error.starts_with("Error: ") {
                error
            } else {
                mutating_error(map_generation_error(&error))
            }
        })
    }

    /// 基线 `owner.characterProposals.stage(source)`
    /// （generation-controller.ts:183 → main-generation-owner.ts:880-896
    /// 的 `characters` 依赖构造）。
    ///
    /// 基线 admission 门禁（controller:182
    /// `source.kind === 'legacy-roster-generation'` →
    /// `GENERATION_LEGACY_ADMISSION_REQUIRED`）由命令层承载
    /// （对齐基线 register 顺序：门禁在 owner 方法之前）。
    pub fn stage_character_proposal(
        &self,
        conn: &Connection,
        source: &Value,
    ) -> Result<Value, String> {
        self.assert_current()?;
        let proposals = self.character_proposal_service(conn);
        Self::character_proposal_result(proposals.stage(source))
    }

    /// 基线 `owner.characterProposals.read(proposalBatchId)`
    /// （generation-controller.ts:185）。
    pub fn read_character_proposal(
        &self,
        conn: &Connection,
        proposal_batch_id: &str,
    ) -> Result<Value, String> {
        self.assert_current()?;
        let proposals = self.character_proposal_service(conn);
        Self::character_proposal_result(proposals.read(proposal_batch_id))
    }

    /// 基线 `owner.characterProposals.listPendingFinalized()`
    /// （generation-controller.ts:186）。
    pub fn list_pending_finalized_character_proposals(
        &self,
        conn: &Connection,
    ) -> Result<Value, String> {
        self.assert_current()?;
        let proposals = self.character_proposal_service(conn);
        let summaries = proposals.list_pending_finalized().map_err(|error| {
            if error.starts_with("Error: ") {
                error
            } else {
                mutating_error(map_generation_error(&error))
            }
        })?;
        Ok(Value::Array(summaries))
    }

    /// 基线 `owner.characterProposals.readPendingFinalized(proposalBatchId)`
    /// （generation-controller.ts:187）。
    pub fn read_pending_finalized_character_proposal(
        &self,
        conn: &Connection,
        proposal_batch_id: &str,
    ) -> Result<Value, String> {
        self.assert_current()?;
        let proposals = self.character_proposal_service(conn);
        Self::character_proposal_result(proposals.read_pending_finalized(proposal_batch_id))
    }

    /// 基线 `owner.approveCharacterProposal(request)`
    /// （generation-controller.ts:188 → main-generation-owner.ts:1255-1266）：
    /// `assertCurrent` → 事务内 `characters.read` → 幂等重放
    /// （已批准 → 直接 `approve`）→ handle 四分支解析 →
    /// `agents.withChildEffect(handle, () => approve)`（handle 存在时）。
    ///
    /// # 刻意偏离（均有上游语义理由）
    /// - **外层事务**：基线以 `db.transaction(...).immediate()` 包裹
    ///   `read` + `approve`（better-sqlite3 嵌套事务退化为 savepoint）；
    ///   Tauri 侧服务层 `read` / `approve` 自包 `unchecked_transaction`
    ///   （rusqlite 不支持嵌套事务，服务层仅 `stage` 有 `is_autocommit`
    ///   适配），故本方法不包外层事务、顺序调用。差异面：`read` 的
    ///   envelope 刷新（pending 时 revision / items 同步）在 `approve`
    ///   前提交——该刷新为幂等收敛操作（下次 `read` 同样执行），
    ///   `approve` 失败时仅影响写入时序、不影响正确性。
    /// - **finalized-generation 分支**：基线 `proveFinalizedCharacterGeneration`
    ///   未迁移（B1 待移植叶函数，见 `services/character_proposal.rs`
    ///   头注）——服务层 `read_envelope` 按基线 `readEnvelope` 的
    ///   catch 语义把未移植证明的失败折叠为
    ///   `CHARACTER_PROPOSAL_SOURCE_CHANGED`，故 finalized-generation
    ///   提案在 `read` 步即 fail-closed；本方法的
    ///   `CHARACTER_PROPOSAL_PROOF_PORT_PENDING` 分支为证明函数
    ///   移植后的结构占位（迁移占位码、非基线码），待证明函数
    ///   移植后活化 handle 解析。
    /// - **withChildEffect**：基线事务门禁（agent-generation.ts:345）
    ///   由调用方结构保证（见 [`MainGenerationOwner::with_agent_child_effect`]
    ///   头注）；G4-10d 活化——运行身份（`require_run`）/ 服务层
    ///   registration / 源一致性门禁链先行校验，效应经服务层
    ///   `with_child_effect` 执行（host `current` / `capture` 保守
    ///   占位的诚实化项见该方法头注）。
    pub fn approve_character_proposal(
        &self,
        conn: &Connection,
        request: &Value,
    ) -> Result<Value, String> {
        self.assert_current()?;
        let proposals = self.character_proposal_service(conn);
        // 基线 :1257 `characters.read(request.proposalBatchId)`
        let batch = Self::character_proposal_result(
            proposals.read(request["proposalBatchId"].as_str().unwrap_or_default()),
        )?;
        // 基线 :1258 幂等重放：已批准 → 直接 approve（重放决策校验）
        if batch["status"].as_str() == Some("approved") {
            return Self::character_proposal_result(proposals.approve(request));
        }
        // 基线 :1260-1263 handle 解析（finalized-generation / legacy-roster-generation
        // / generation / directory 四分支；import / 其他 → undefined）
        let source = batch["source"].clone();
        let handle = match source["kind"].as_str() {
            // 基线 :1261 proveFinalizedCharacterGeneration(...).currentHandle ——
            // 证明函数未迁移（B1），fail-closed（见方法头注）。
            Some("finalized-generation") => {
                return Err(mutating_error("CHARACTER_PROPOSAL_PROOF_PORT_PENDING"));
            }
            // 基线 :1262 readLegacyRosterGenerationProof(...).currentHandle
            Some("legacy-roster-generation") => {
                let runs = GenerationRunRepository::default();
                let proof = Self::character_proposal_result(read_legacy_roster_generation_proof(
                    conn,
                    &runs,
                    &self.session.project_id,
                    &source,
                ))?;
                Some(proof["currentHandle"].clone())
            }
            // 基线 :1263 `source.kind === 'generation' ? source.handle`
            Some("generation") => Some(source["handle"].clone()),
            // 基线 :1264 directory 分支：listDirectoryProgress 查找
            // operationId 匹配项的 sourceHandle（未命中 → undefined）
            Some("directory") => {
                let progress = self.list_directory_progress(conn).map_err(|error| {
                    if error.starts_with("Error: ") {
                        error
                    } else {
                        mutating_error(map_generation_error(&error))
                    }
                })?;
                let operation_id = source["operationId"].as_str().unwrap_or_default();
                progress
                    .as_array()
                    .and_then(|items| {
                        items
                            .iter()
                            .find(|item| item["operationId"].as_str() == Some(operation_id))
                            .cloned()
                    })
                    .map(|item| item["sourceHandle"].clone())
            }
            _ => None,
        };
        // 基线 :1265 `handle ? agents.withChildEffect(handle, () =>
        // characters.approve(request)) : characters.approve(request)`。
        // withChildEffect 返回 Result<Value>，拍平双层 Result。
        match handle {
            Some(handle) if !handle.is_null() => self
                .with_agent_child_effect(conn, &handle, || proposals.approve(request))
                .and_then(|result| result)
                .map_err(|error| {
                    if error.starts_with("Error: ") {
                        error
                    } else {
                        mutating_error(map_generation_error(&error))
                    }
                }),
            _ => Self::character_proposal_result(proposals.approve(request)),
        }
    }

    /// 基线 `owner.characterProposals.cancel(request)`
    /// （generation-controller.ts:189）。
    pub fn cancel_character_proposal(
        &self,
        conn: &Connection,
        request: &Value,
    ) -> Result<Value, String> {
        self.assert_current()?;
        let proposals = self.character_proposal_service(conn);
        Self::character_proposal_result(proposals.cancel(request))
    }

    /// 基线 `owner.characterProposals.identitySnapshot()`
    /// （generation-controller.ts:190，`character-identity:read` 频道）。
    pub fn character_identity_snapshot(&self, conn: &Connection) -> Result<Value, String> {
        self.assert_current()?;
        let proposals = self.character_proposal_service(conn);
        Self::character_proposal_result(proposals.identity_snapshot())
    }
}

// ── AgentHost 注入（基线 main-generation-owner.ts:897-902）──

/// 基线 owner 闭包 :897-902 的 `AgentHost` 注入等价物——
/// owner 自身作为 agent 生成状态机的宿主：
/// - `begin` → `begin(selection, false, true)`（agent
///   初始化路径）；
/// - `read` → `viewOf(requireRun(handle))`（非执行语义）；
/// - `resume` → `resume(handle)`；
/// - `execute` → `execute({ handle, invocationNonce,
///   task }, true)`（主进程内部执行路径，见
///   [`MainGenerationOwner::execute_agent_round`]）；
/// - `current` / `capture` → 以实时模型收据重建源；读取失败按基线返回 false / 错误。
impl AgentHost for MainGenerationOwner {
    fn begin(&self, conn: &Connection, selection: &Value) -> Result<Value, String> {
        self.begin(conn, selection, false, true)
    }

    fn read(&self, conn: &Connection, handle: &MainGenerationRunHandle) -> Result<Value, String> {
        // 基线 `read: handle => viewOf(requireRun(handle))`——
        // `requireRun` 非执行语义（默认 execution = false）。
        let handle_value = serde_json::to_value(handle).map_err(|error| error.to_string())?;
        let run = self.require_run(conn, &handle_value, false)?;
        self.view_of(conn, &run)
    }

    fn resume(&self, conn: &Connection, handle: &MainGenerationRunHandle) -> Result<Value, String> {
        // 基线 `resume` 闭包捕获：owner.resume(handle)。
        let handle_value = serde_json::to_value(handle).map_err(|error| error.to_string())?;
        self.resume_with_agent_admission(conn, &handle_value, true)
    }

    fn execute(
        &self,
        conn: &Connection,
        handle: &MainGenerationRunHandle,
        nonce: &str,
        task: &Value,
    ) -> Result<Value, String> {
        self.execute_agent_round(conn, handle, nonce, task)
    }

    fn current(
        &self,
        conn: &Connection,
        run: &DurableGenerationRun,
        expected: Option<&RunBinding>,
    ) -> bool {
        self.rebuild_binding(conn, &run.binding)
            .is_ok_and(|current| {
                compare_generation_source_bindings(expected.unwrap_or(&run.binding), &current)
            })
    }

    fn capture(&self, conn: &Connection, run: &DurableGenerationRun) -> Result<RunBinding, String> {
        self.rebuild_binding(conn, &run.binding)
    }
}
// ============================================================================
// 草稿上下文准备辅助（基线 main-generation-owner.ts:295-308）
// ============================================================================

/// 基线 `without(row, excluded)` 的等价物
/// （`generation_source_binding::without` 为模块私有，
/// 本模块就地实现）。
fn without_keys(row: &Map<String, Value>, excluded: &[&str]) -> Map<String, Value> {
    row.iter()
        .filter(|(key, _)| !excluded.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// 基线 generationOutputContract：只允许可见文本/结构化数据和唯一用途覆盖。
fn generation_output_contract(selection: &Value) -> Result<Value, String> {
    let output = selection
        .get("output")
        .and_then(Value::as_str)
        .filter(|output| matches!(*output, "visible-text" | "structured-data"))
        .ok_or_else(|| "GENERATION_OUTPUT_CONTRACT_INVALID".to_string())?;
    let Some(overrides) = selection.get("outputOverrides") else {
        return Ok(json!(output));
    };
    let overrides = overrides
        .as_array()
        .filter(|items| !items.is_empty() && items.len() <= 8)
        .ok_or_else(|| "GENERATION_OUTPUT_CONTRACT_INVALID".to_string())?;
    let mut purposes = HashSet::new();
    for item in overrides {
        let purpose = item
            .get("purpose")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let valid_purpose = !purpose.is_empty()
            && purpose.len() <= 128
            && purpose
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && purpose.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, ':' | '_' | '-')
            });
        if !valid_purpose
            || !purposes.insert(purpose)
            || !item.as_object().is_some_and(|object| {
                object
                    .keys()
                    .all(|key| matches!(key.as_str(), "purpose" | "output"))
            })
            || !matches!(
                item.get("output").and_then(Value::as_str),
                Some("visible-text" | "structured-data")
            )
        {
            return Err("GENERATION_OUTPUT_CONTRACT_INVALID".to_string());
        }
    }
    Ok(json!({"primary": output, "overrides": overrides}))
}

/// 基线 `textHash`（`createHash('sha256').update(text)
/// .digest('hex')`；形态对齐 `generation_source_binding::hash`，
/// G4-6a——未修改 UTF-8 字节的 SHA-256 hex）。
fn text_hash(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 基线 `preparationProjection(binding, excludeKnowledge = false)`
/// （main-generation-owner.ts:295-299）：准备期投影——
/// `manifest` 剔除 `selectedDraftIds` /
/// `selectedFinalizedDraftIds` / `materialDecision` /
/// `materialDecisionHash`（+ `excludeKnowledge` 时
/// `knowledgeSnapshot`；S10B 注释：材料决策键只在
/// 取得准入之后才可能出现在真正运行的 manifest 上，
/// 因此与 `selectedFinalizedDraftIds` 同类，不参与
/// 「准备期冻结源未变」的比对）、`fingerprint` 剔除
/// `contextSnapshotHash` / `dependencyHash`（指纹侧
/// `contextSnapshotHash` 本就被排除）、`sources` 剔除
/// `draft:` / `finalized:` 前缀（+ `excludeKnowledge`
/// 时 `kb:` 前缀与 `knowledge-selection`）。
fn preparation_projection(binding: &RunBinding, exclude_knowledge: bool) -> Value {
    let manifest_excluded: &[&str] = if exclude_knowledge {
        &PREPARATION_MANIFEST_EXCLUDED_WITH_KNOWLEDGE
    } else {
        &PREPARATION_MANIFEST_EXCLUDED
    };
    let manifest = without_keys(&binding.source_manifest, manifest_excluded);
    let fingerprint = serde_json::to_value(&binding.fingerprint)
        .ok()
        .and_then(|value| {
            value
                .as_object()
                .map(|record| without_keys(record, &PREPARATION_FINGERPRINT_EXCLUDED))
        })
        .unwrap_or_default();
    let sources: Vec<Value> = binding
        .source_refs
        .iter()
        .filter(|reference| {
            !reference.source_id.starts_with("draft:")
                && !reference.source_id.starts_with("finalized:")
                && (!exclude_knowledge
                    || (!reference.source_id.starts_with("kb:")
                        && reference.source_id != "knowledge-selection"))
        })
        .map(|reference| serde_json::to_value(reference).unwrap_or(Value::Null))
        .collect();
    json!({
        "manifest": Value::Object(manifest),
        "fingerprint": Value::Object(fingerprint),
        "sources": Value::Array(sources),
    })
}

/// 基线 `preparationMatches(previous, current,
/// excludeKnowledge = false)`（main-generation-owner.ts:301-308）：
/// 准备期投影深度相等（基线 `isDeepStrictEqual` 的
/// `Value` 相等等价物）+ `draft:` / `finalized:`
/// sourceRefs 逐引用深度相等（按 `sourceId` 匹配后
/// 全保真比较）。
fn preparation_matches(
    previous: &RunBinding,
    current: &RunBinding,
    exclude_knowledge: bool,
) -> bool {
    preparation_projection(previous, exclude_knowledge)
        == preparation_projection(current, exclude_knowledge)
        && previous
            .source_refs
            .iter()
            .filter(|reference| {
                reference.source_id.starts_with("draft:")
                    || reference.source_id.starts_with("finalized:")
            })
            .all(|reference| {
                current
                    .source_refs
                    .iter()
                    .find(|candidate| candidate.source_id == reference.source_id)
                    .is_some_and(|candidate| {
                        serde_json::to_value(reference).unwrap_or(Value::Null)
                            == serde_json::to_value(candidate).unwrap_or(Value::Null)
                    })
            })
}

// per-database owner 缓存
// ============================================================================

/// 缓存条目（对齐基线 `{ owner, session, subscribers }`，
/// generation-controller.ts:37）。
struct OwnerCacheEntry {
    owner: MainGenerationOwner,
    /// 基线 `subscribers: Set<WebContents>`。Tauri 侧事件经
    /// 基线 `subscribers: Set<WebContents>`。D-2 既定决策（**广播 +
    /// 客户端过滤**）下事件经 `AppHandle::emit` 全窗口广播、客户端按
    /// 载荷 `projectId` / `epoch` 过滤，per-window 订阅登记不再需要；
    /// 本字段保留为缓存条目形状锚点（基线 `{ owner, session, subscribers }`），
    /// 随执行波次清理移除。
    #[allow(dead_code)]
    subscribers: HashSet<String>,
}

/// 临时执行租约守卫：只有运行创建/恢复成功后才交给 owner。
/// 任一校验或账本错误均自动关闭新租约，原运行租约不受影响。
struct OwnerModelLease {
    store: Arc<Mutex<crate::llm::lease::LlmLeaseStore>>,
    receipt: crate::llm::lease::ModelExecutionLeaseReceipt,
    attached: bool,
}
impl OwnerModelLease {
    fn attach(mut self, owner: &MainGenerationOwner, run_id: &str) {
        owner.record_run_lease(run_id.to_string(), self.receipt.lease_id.clone());
        self.attached = true;
    }
}
impl Drop for OwnerModelLease {
    fn drop(&mut self) {
        if !self.attached {
            if let Ok(mut store) = self.store.lock() {
                store.close(&self.receipt.lease_id, owner_now_ms());
            }
        }
    }
}

/// 借用不可变连接的嵌套事务守卫；失败或异常退出时撤销本次草稿效应。
struct DraftCommitSavepoint<'a> {
    conn: &'a Connection,
    committed: bool,
}
impl<'a> DraftCommitSavepoint<'a> {
    fn new(conn: &'a Connection) -> Result<Self, String> {
        conn.execute_batch("SAVEPOINT lorekeeper_draft_commit")
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
        Ok(Self {
            conn,
            committed: false,
        })
    }
    fn commit(mut self) -> Result<(), String> {
        self.conn
            .execute_batch("RELEASE lorekeeper_draft_commit")
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
        self.committed = true;
        Ok(())
    }
}
impl Drop for DraftCommitSavepoint<'_> {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.conn.execute_batch(
                "ROLLBACK TO lorekeeper_draft_commit; RELEASE lorekeeper_draft_commit",
            );
        }
    }
}

/// per-database owner 缓存（对齐基线 `owners` Map，
/// generation-controller.ts:37）。
///
/// - 键：归一化项目根路径（`security::normalized_project_path`），等价
///   基线以 `Database` 对象身份为键（Tauri 侧一项目根 ⇔ 唯一项目库
///   连接，`db/mod.rs` 头注「一个进程同一时刻只持有一个项目库连接」）；
/// - 线程安全包装：`Mutex`（单写入者模型，与 `state.rs` 各注册表一致）。
#[derive(Default)]
pub struct MainGenerationOwnerCache {
    entries: Mutex<HashMap<String, OwnerCacheEntry>>,
    /// 独立短锁，网络等待时仍可访问；只保存身份和中止服务，不保存正文/模型配置。
    execution_controls: Mutex<HashMap<String, ExecutionControlEntry>>,
    /// 暂停/取消等待执行结算期间阻止新登记，不阻止原执行的短数据库回调。
    stopping_projects: Mutex<HashMap<String, usize>>,
    /// 关闭登记的引用计数：并发关闭请求全部退出后才重新开放执行登记。
    closing_projects: Mutex<HashMap<String, usize>>,
    lease_store: Arc<Mutex<crate::llm::lease::LlmLeaseStore>>,
    event_sink: SharedGenerationEventSink,
}

/// 已经由 owner 和项目数据库门禁验证的在途执行身份。
struct ExecutionControlEntry {
    session: ProjectSessionContext,
    handle: MainGenerationRunHandle,
    service: Arc<crate::services::generation_run_service::GenerationRunService>,
    users: usize,
    completion: Arc<ExecutionCompletion>,
}

/// 同句柄控制登记引用计数；最后一个准备/投影退出后才撤销中止意图并通知等待者。
pub(crate) struct ExecutionControlGuard<'a> {
    cache: &'a MainGenerationOwnerCache,
    key: String,
    completion: Arc<ExecutionCompletion>,
}

impl Drop for ExecutionControlGuard<'_> {
    fn drop(&mut self) {
        let finished = if let Ok(mut controls) = self.cache.execution_controls.lock() {
            let (remove, finished) = match controls.get_mut(&self.key) {
                Some(entry) if Arc::ptr_eq(&entry.completion, &self.completion) => {
                    entry.users -= 1;
                    (entry.users == 0, entry.users == 0)
                }
                _ => (false, true),
            };
            if remove {
                if let Some(entry) = controls.remove(&self.key) {
                    entry
                        .service
                        .clear_abort_request(&entry.handle.root_action_id);
                }
            }
            finished
        } else {
            true
        };
        if finished {
            self.completion.finish();
        }
    }
}

#[derive(Default)]
struct ExecutionCompletion {
    done: Mutex<bool>,
    changed: Condvar,
}

impl ExecutionCompletion {
    fn wait(&self) -> Result<(), String> {
        let mut done = self
            .done
            .lock()
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
        while !*done {
            done = self
                .changed
                .wait(done)
                .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
        }
        Ok(())
    }

    fn finish(&self) {
        if let Ok(mut done) = self.done.lock() {
            *done = true;
            self.changed.notify_all();
        }
    }
}

/// 用户停止操作的短准入围栏，等待的是捕获的执行实例，而非后续新执行。
pub(crate) struct ExecutionStopGuard<'a> {
    cache: &'a MainGenerationOwnerCache,
    key: String,
    completion: Option<Arc<ExecutionCompletion>>,
}

impl ExecutionStopGuard<'_> {
    pub(crate) fn wait(&self) -> Result<(), String> {
        match &self.completion {
            Some(completion) => completion.wait(),
            None => Ok(()),
        }
    }
}

impl Drop for ExecutionStopGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut stopping) = self.cache.stopping_projects.lock() {
            if let Some(count) = stopping.get_mut(&self.key) {
                *count -= 1;
                if *count == 0 {
                    stopping.remove(&self.key);
                }
            }
        }
    }
}

/// 关闭事务的易失准入围栏，失败/提前返回时也必须撤销。
struct ProjectCloseGuard<'a> {
    cache: &'a MainGenerationOwnerCache,
    key: String,
}

impl Drop for ProjectCloseGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut closing) = self.cache.closing_projects.lock() {
            if let Some(count) = closing.get_mut(&self.key) {
                *count -= 1;
                if *count == 0 {
                    closing.remove(&self.key);
                }
            }
        }
    }
}

impl std::fmt::Debug for MainGenerationOwnerCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 不展示准备快照、源绑定、模型配置或正文。
        formatter
            .debug_struct("MainGenerationOwnerCache")
            .finish_non_exhaustive()
    }
}

/// [`MainGenerationOwnerCache::authorized_owner`] 返回的 owner 守卫：
/// 持有缓存锁直至 drop，保证「门禁 → owner 方法」临界区内项目库 /
/// 会话态不被并发切换（基线单线程事件循环的等价物）。
///
/// ⚠️ 持守卫期间**不得**调用 [`MainGenerationOwnerCache::suspend_for_project_close`]
/// （同一把锁，会自死锁）；基线的挂起时机是项目关闭，发生在任何
/// 命令临界区之外。
pub struct OwnerGuard<'a> {
    entries: MutexGuard<'a, HashMap<String, OwnerCacheEntry>>,
    key: String,
}

impl<'a> OwnerGuard<'a> {
    /// 当前 owner（可变）。owner 方法应在持守卫时调用。
    pub fn owner(&mut self) -> &mut MainGenerationOwner {
        let entry = self
            .entries
            .get_mut(&self.key)
            .expect("owner entry is held by the guard");
        &mut entry.owner
    }

    /// 捕获的会话身份（基线 `ownerSessions` WeakMap 的等价读取点，
    /// generation-controller.ts:38 / :126）。
    pub fn session(&self) -> &ProjectSessionContext {
        let entry = self
            .entries
            .get(&self.key)
            .expect("owner entry is held by the guard");
        &entry.owner.session
    }
}

impl MainGenerationOwnerCache {
    /// 新建空缓存（等价基线模块级 `const owners = new Map()` 的初始态）。
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with_lease_store(
        lease_store: Arc<Mutex<crate::llm::lease::LlmLeaseStore>>,
    ) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            execution_controls: Mutex::new(HashMap::new()),
            stopping_projects: Mutex::new(HashMap::new()),
            closing_projects: Mutex::new(HashMap::new()),
            lease_store,
            event_sink: Arc::new(Mutex::new(None)),
        }
    }
    pub(crate) fn set_event_sink(&self, sink: MainGenerationEventSink) {
        if let Ok(mut current) = self.event_sink.lock() {
            *current = Some(sink);
        }
    }

    /// 必须在 owner → 数据库临界区内调用；先校验完整运行身份，再公开中止句柄。
    pub(crate) fn register_execution_control<'a>(
        &'a self,
        owner: &MainGenerationOwner,
        conn: &Connection,
        handle: &Value,
    ) -> Result<ExecutionControlGuard<'a>, String> {
        owner.require_run(conn, handle, true)?;
        let handle: MainGenerationRunHandle = serde_json::from_value(handle.clone())
            .map_err(|_| mutating_error("GENERATION_RUN_IDENTITY_MISMATCH"))?;
        let key = security::normalized_project_path(&owner.session.project_path);
        // 此短锁保留到控制登记完成，与关闭围栏的发布形成单一准入顺序。
        let closing = self
            .closing_projects
            .lock()
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
        if closing.contains_key(&key) {
            return Err(mutating_error("GENERATION_PROJECT_CLOSED"));
        }
        let stopping = self
            .stopping_projects
            .lock()
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
        if stopping.contains_key(&key) {
            return Err(mutating_error("GENERATION_INVOCATION_CONFLICT"));
        }
        let mut controls = self
            .execution_controls
            .lock()
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
        let completion = if let Some(entry) = controls.get_mut(&key) {
            if entry.handle != handle
                || entry.session.project_id != owner.session.project_id
                || entry.session.lease_id != owner.session.lease_id
                || !Arc::ptr_eq(&entry.service, &owner.run_service)
            {
                return Err(mutating_error("GENERATION_INVOCATION_CONFLICT"));
            }
            entry.users += 1;
            entry.completion.clone()
        } else {
            let completion = Arc::new(ExecutionCompletion::default());
            owner
                .run_service
                .clear_abort_request(&handle.root_action_id);
            controls.insert(
                key.clone(),
                ExecutionControlEntry {
                    session: owner.session.clone(),
                    handle,
                    service: Arc::clone(&owner.run_service),
                    users: 1,
                    completion: completion.clone(),
                },
            );
            completion
        };
        Ok(ExecutionControlGuard {
            cache: self,
            key,
            completion,
        })
    }

    /// 在等待 owner/数据库长锁前中止已认证的在途运行。
    /// 此处不替代持久化门禁：命令仍须重新取得 owner 并完成暂停/取消。
    pub(crate) fn request_execution_abort(
        &self,
        session: &ProjectSessionContext,
        active: Option<&ActiveProject>,
        handle: &MainGenerationRunHandle,
    ) -> Result<(), String> {
        security::assert_current_project_context(session, active)
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
        let key = security::normalized_project_path(&session.project_path);
        let controls = self
            .execution_controls
            .lock()
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
        if let Some(entry) = controls.get(&key) {
            if entry.session.project_id != session.project_id
                || entry.session.lease_id != session.lease_id
            {
                return Err(mutating_error(GENERATION_REQUEST_FAILED));
            }
            // 只有逐字段一致的完整句柄才允许提前中止；其他句柄走原持久化校验。
            if entry.handle == *handle {
                entry.service.request_abort_root(&handle.root_action_id);
            }
        }
        Ok(())
    }

    /// 先发布停止围栏、认证完整句柄并发信号，再由调用方无业务锁等待。
    pub(crate) fn begin_execution_stop(
        &self,
        session: &ProjectSessionContext,
        active: Option<&ActiveProject>,
        handle: &MainGenerationRunHandle,
    ) -> Result<ExecutionStopGuard<'_>, String> {
        security::assert_current_project_context(session, active)
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
        let key = security::normalized_project_path(&session.project_path);
        {
            let mut stopping = self
                .stopping_projects
                .lock()
                .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
            *stopping.entry(key.clone()).or_default() += 1;
        }
        let mut guard = ExecutionStopGuard {
            cache: self,
            key: key.clone(),
            completion: None,
        };
        self.request_execution_abort(session, active, handle)?;
        {
            let controls = self
                .execution_controls
                .lock()
                .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
            if let Some(entry) = controls.get(&key) {
                if entry.session.project_id != session.project_id
                    || entry.session.lease_id != session.lease_id
                {
                    return Err(mutating_error(GENERATION_REQUEST_FAILED));
                }
                if entry.handle == *handle {
                    // 信号已由认证中止入口发布；这里只捕获该实例的完成通知。
                    guard.completion = Some(entry.completion.clone());
                }
            }
        }
        Ok(guard)
    }

    fn wait_project_execution(&self, project_root: &str) -> Result<(), String> {
        let key = security::normalized_project_path(project_root);
        let completion = self
            .execution_controls
            .lock()
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?
            .get(&key)
            .map(|entry| entry.completion.clone());
        match completion {
            Some(completion) => completion.wait(),
            None => Ok(()),
        }
    }

    /// 只用于内部数据库关闭钩子。先阻止新执行登记，再唤醒已登记执行；
    /// 不持 owner/数据库锁，不提前关闭会话或连接。
    fn register_project_close(&self, project_root: &str) -> Result<ProjectCloseGuard<'_>, String> {
        let key = security::normalized_project_path(project_root);
        {
            let mut closing = self
                .closing_projects
                .lock()
                .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
            *closing.entry(key.clone()).or_default() += 1;
        }
        let guard = ProjectCloseGuard {
            cache: self,
            key: key.clone(),
        };
        let controls = self
            .execution_controls
            .lock()
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
        if let Some(entry) = controls.get(&key) {
            entry
                .service
                .request_project_close_root(&entry.handle.root_action_id);
        }
        Ok(guard)
    }

    /// 同时持有 owner 缓存锁与关闭临界区，锁序始终为 owner → 项目库。
    /// 回调必须在关闭连接前挂起 owner；失败时条目仍留在缓存，不能丢失可重试状态。
    pub(crate) fn before_database_close<T>(
        &self,
        project_root: Option<&str>,
        close: impl FnOnce(Option<&mut MainGenerationOwner>) -> Result<T, String>,
    ) -> Result<T, String> {
        let _closing = project_root
            .map(|root| self.register_project_close(root))
            .transpose()?;
        if let Some(root) = project_root {
            self.wait_project_execution(root)?;
        }
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
        let key = project_root.map(security::normalized_project_path);
        let owner = key
            .as_ref()
            .and_then(|key| entries.get_mut(key))
            .map(|entry| &mut entry.owner);
        let result = close(owner)?;
        if let Some(key) = key {
            entries.remove(&key);
        }
        Ok(result)
    }

    /// 缓存条目数（诊断 / 测试用；基线无等价物）。
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .ok()
            .map(|entries| entries.len())
            .unwrap_or(0)
    }

    /// 缓存是否为空（诊断 / 测试用；基线无等价物）。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 对齐基线 `authorizedOwner` 门控（generation-controller.ts:86-130）。
    ///
    /// 语义（逐条对齐）：
    /// 1. 会话再认证：`security::assert_current_project_context`（基线
    ///    `projectAccess.assertCurrentProjectContext`）；失败按基线
    ///    register catch 映射为 [`GENERATION_REQUEST_FAILED`]（基线中文
    ///    守卫文案不匹配透传正则，generation-controller.ts:160-162）；
    /// 2. 当前项目库必须已打开，否则 [`GENERATION_DATABASE_NOT_READY`]
    ///    （基线 `getProjectDb()` 为 null，generation-controller.ts:88-90）；
    /// 3. 缓存命中但 `projectId` / `leaseId` 与缓存会话不一致 → 先挂起
    ///    旧 owner 再逐出（基线 generation-controller.ts:91-95）；
    /// 4. 未命中 → 以**冻结的会话副本**创建 owner 并入缓存（基线
    ///    `Object.freeze({ ...session })`，generation-controller.ts:97）；
    /// 5. 基线末步 `entry.subscribers.add(event.sender)`
    ///    （generation-controller.ts:129）的 Tauri 等价物为 **TODO(B3)**：
    ///    命令层把当前窗口标签登记进 [`OwnerCacheEntry::subscribers`]。
    ///
    /// 返回 [`OwnerGuard`]（持锁）；失败返回带 `Error: ` 前缀的基线错误码
    /// （MUTATING 信封，见 [`mutating_error`]）。
    pub fn authorized_owner(
        &self,
        session: &ProjectSessionContext,
        active: Option<&ActiveProject>,
    ) -> Result<OwnerGuard<'_>, String> {
        // 1) 会话再认证（基线 projectAccess.assertCurrentProjectContext）
        security::assert_current_project_context(session, active)
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
        // 2) 当前项目库（基线 getProjectDb()；Tauri 侧活跃会话即库已打开）
        let Some(active) = active else {
            return Err(mutating_error(GENERATION_DATABASE_NOT_READY));
        };
        // 缓存键由持锁认证后的活跃项目确定。
        // 调用方传入的快照仍受既有身份校验；生产使用下方实时入口。
        let entries = self
            .entries
            .lock()
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
        self.authorized_owner_locked(entries, session, Some(active))
    }

    pub(crate) fn authorized_owner_current(
        &self,
        session: &ProjectSessionContext,
        read_active: impl FnOnce() -> Option<ActiveProject>,
    ) -> Result<OwnerGuard<'_>, String> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
        let active = read_active();
        self.authorized_owner_locked(entries, session, active.as_ref())
    }

    fn authorized_owner_locked<'a>(
        &'a self,
        mut entries: MutexGuard<'a, HashMap<String, OwnerCacheEntry>>,
        session: &ProjectSessionContext,
        active: Option<&ActiveProject>,
    ) -> Result<OwnerGuard<'a>, String> {
        security::assert_current_project_context(session, active)
            .map_err(|_| mutating_error(GENERATION_REQUEST_FAILED))?;
        let active = active.ok_or_else(|| mutating_error(GENERATION_DATABASE_NOT_READY))?;
        let key = security::normalized_project_path(&active.root_path);
        // 3) 会话身份不一致 → 挂起并逐出（基线 generation-controller.ts:91-95）
        let stale_session = entries.get(&key).is_some_and(|entry| {
            entry.owner.session.project_id != session.project_id
                || entry.owner.session.lease_id != session.lease_id
        });
        if stale_session {
            let mut stale = entries
                .remove(&key)
                .expect("stale entry was just found above");
            stale.owner.suspend_for_project_close();
        }
        // 4) 未命中 → 以冻结会话副本创建（基线 Object.freeze({ ...session })）
        entries
            .entry(key.clone())
            .or_insert_with(|| OwnerCacheEntry {
                owner: MainGenerationOwner::with_event_sink(
                    session.clone(),
                    Arc::clone(&self.lease_store),
                    Arc::clone(&self.event_sink),
                ),
                subscribers: HashSet::new(),
            });
        // 5) D-2 既定广播/客户端身份过滤；不额外登记窗口订阅。
        Ok(OwnerGuard { entries, key })
    }

    /// 基线 `onProjectDatabaseBeforeClose` 钩子（generation-controller.ts:82-85）：
    /// 项目库关闭前挂起其 owner 并逐出缓存。
    ///
    /// 命令层在关闭项目库（`AppState::project_db` 置空）**前**调用。
    pub fn suspend_for_project_close(&self, project_root: &str) {
        let key = security::normalized_project_path(project_root);
        if let Some(mut entries) = self.entries.lock().ok() {
            if let Some(mut entry) = entries.remove(&key) {
                entry.owner.suspend_for_project_close();
                // TODO(B3)：基线同时随条目丢弃 `subscribers`
                // （WebContents 集合随窗口销毁）。
            }
        }
    }
}

// ============================================================================
// 事件经 OwnerEventDelivery 注入命令层出口；services 保持无 Tauri 依赖。
// 快照取自当前连接的持久收据，推理仅在内存出口交付。
// ============================================================================
// 事件频道与载荷（契约 `GenerationOwnerEvents`，
// src/shared/generation-owner-contract.ts）
// ============================================================================

/// `generation:snapshot` 事件频道名。
pub const GENERATION_SNAPSHOT_EVENT: &str = "generation:snapshot";

/// `generation:reasoning` 事件频道名。
pub const GENERATION_REASONING_EVENT: &str = "generation:reasoning";

/// `MainGenerationReasoningEvent`（基线 `MainGenerationOwnerDependencies`
/// 的 `onReasoning` 载荷，generation-controller.ts:121-124 /
/// main-generation-owner.ts:110-116）：流式推理的易失文本增量
/// `{ projectId, epoch, rootActionId, runId, attemptId, text }`——
/// 绝不进入运行视图或回执（契约 `GenerationOwnerEvents` 注释）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MainGenerationReasoningEventProjection {
    pub project_id: String,
    pub epoch: String,
    pub root_action_id: String,
    pub run_id: String,
    pub attempt_id: String,
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::generation_run_repository::OpenGenerationRunRequest;

    fn draft_commit_fixture(
        text: &str,
        finish: &str,
        target: i64,
    ) -> (
        crate::services::main_generation_sources::tests::TempRoot,
        Connection,
        MainGenerationOwner,
        Value,
    ) {
        draft_commit_source_fixture(text, finish, target, false)
    }

    fn draft_commit_source_fixture(
        text: &str,
        finish: &str,
        target: i64,
        with_source: bool,
    ) -> (
        crate::services::main_generation_sources::tests::TempRoot,
        Connection,
        MainGenerationOwner,
        Value,
    ) {
        let (temp, conn, mut owner, _) = source_owner_fixture();
        let chapter = if with_source { 2 } else { 1 };
        let sources = if with_source {
            let id = crate::repositories::draft_repository::create(
                &conn,
                &crate::repositories::draft_repository::DraftCreateParams {
                    chapter_number: 1,
                    version: None,
                    source: "write".to_string(),
                    content: "合成前章".to_string(),
                    word_count: 4,
                    source_dependencies: None,
                },
            )
            .unwrap();
            vec![id]
        } else {
            vec![]
        };
        conn.execute(
            "INSERT INTO blueprints (chapter_number,title) VALUES (?,'合成蓝图')",
            [chapter],
        )
        .unwrap();
        let selection = json!({"operation":"chapter-draft","uiActionNonce":"draft-fixture",
            "modelId":"fixture","chapterNumber":chapter,"selectedDraftIds":sources,
            "selectedFinalizedDraftIds":[],"selectedBlueprintChapterNumbers":[],
            "promptKeys":["edit_selected_text"],"skillStages":[],"output":"visible-text",
            "authorInputs":[{"id":"draft:target-units","text":target.to_string()}]});
        let policy = crate::services::main_generation_plan::new_main_generation_policy(
            &selection,
            &crate::services::main_generation_sources::tests::model(),
        )
        .unwrap();
        let model_receipt = crate::services::main_generation_sources::read_only_model_receipt(
            &owner.global_data_root,
            "fixture",
        )
        .unwrap();
        let binding = owner
            .build_binding(
                &conn,
                &selection,
                &model_receipt,
                &policy.to_value(),
                &json!("visible-text"),
            )
            .unwrap();
        let run = owner
            .repository
            .open(
                &conn,
                &OpenGenerationRunRequest {
                    project_id: binding.project_id,
                    epoch: binding.epoch,
                    fingerprint: binding.fingerprint,
                    context_snapshot_id: binding.context_snapshot_id,
                    source_manifest: binding.source_manifest,
                    source_refs: binding.source_refs,
                    operation: "chapter-draft".to_string(),
                    ui_action_nonce: "draft-fixture".to_string(),
                    frozen_input_hash: fake_content_hash(),
                    budget: policy.budget,
                    parent_root_action_id: None,
                },
            )
            .unwrap();
        let receipt = owner
            .repository
            .reserve(
                &conn,
                &run.run_id,
                "draft-invocation",
                &fake_content_hash(),
                100,
                10,
                None,
                None,
                None,
                None,
                false,
            )
            .unwrap();
        owner
            .repository
            .mark_dispatched(&conn, &receipt.attempt.attempt_id)
            .unwrap();
        let prior = receipt.artifact.unwrap();
        let artifact = owner
            .repository
            .snapshot(&conn, &receipt.attempt.attempt_id, prior.revision, text)
            .unwrap();
        owner
            .repository
            .settle(
                &conn,
                &receipt.attempt.attempt_id,
                None,
                Some(finish.to_string()),
                None,
                None,
            )
            .unwrap();
        let composition = owner
            .repository
            .compose_visible(
                &conn,
                &run.run_id,
                &[artifact.artifact_id],
                None,
                "draft-visible-v2",
            )
            .unwrap();
        let request = json!({"handle":handle_of(&run),"source":"write","chapterNumber":chapter,
            "expectedCompositionHash":composition.text_hash});
        // 此测试只走本地账本，不进行网络调用或真实模型租约分发。
        owner.preparations.clear();
        (temp, conn, owner, request)
    }

    fn draft_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM drafts", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn draft_commit_creates_receipt_and_replays_after_source_change_test() {
        let (_temp, conn, owner, request) =
            draft_commit_fixture("甲乙丙丁戊己庚辛壬癸", "stop", 10);
        let receipt = owner.commit_draft(&conn, &request).unwrap();
        assert_eq!(receipt["success"], true);
        assert_eq!(receipt["version"], 1);
        assert_eq!(draft_count(&conn), 1);
        assert_eq!(
            GenerationDraftEffects
                .read_recovery(
                    &conn,
                    &owner.repository,
                    request["handle"]["runId"].as_str().unwrap()
                )
                .unwrap()["kind"],
            "current"
        );
        let usage: String = conn
            .query_row(
                "SELECT usage_receipt_json FROM generation_attempts WHERE run_id=?",
                [request["handle"]["runId"].as_str().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        let usage: Value = serde_json::from_str(&usage).unwrap();
        assert_eq!(usage["draftCommit"]["contentHash"], receipt["contentHash"]);
        assert!(usage["draftCommit"].get("content_hash").is_none());
        assert!(usage["draftCommit"].get("batchId").is_none());
        conn.execute(
            "UPDATE blueprints SET title='合成变化' WHERE chapter_number=1",
            [],
        )
        .unwrap();
        assert_eq!(owner.commit_draft(&conn, &request).unwrap(), receipt);
        let mut conflict = request.clone();
        conflict["expectedCompositionHash"] = json!("0".repeat(64));
        assert_eq!(
            owner.commit_draft(&conn, &conflict).unwrap_err(),
            "Error: GENERATION_DRAFT_COMMIT_CONFLICT"
        );
        assert_eq!(draft_count(&conn), 1);
    }

    #[test]
    fn draft_commit_rejects_target_and_composition_mismatch_test() {
        let (_temp, conn, owner, request) =
            draft_commit_fixture("甲乙丙丁戊己庚辛壬癸", "stop", 10);
        for key in ["source", "chapterNumber", "batchId"] {
            let mut invalid = request.clone();
            invalid[key] = if key == "chapterNumber" {
                json!(2)
            } else {
                json!("invalid")
            };
            assert_eq!(
                owner.commit_draft(&conn, &invalid).unwrap_err(),
                "Error: GENERATION_DRAFT_TARGET_INVALID"
            );
        }
        let mut invalid = request;
        invalid["expectedCompositionHash"] = json!("0".repeat(64));
        assert_eq!(
            owner.commit_draft(&conn, &invalid).unwrap_err(),
            "Error: GENERATION_DRAFT_COMPOSITION_REQUIRED"
        );
        assert_eq!(draft_count(&conn), 0);
    }

    #[test]
    fn draft_commit_rejects_truncated_and_insufficient_text_test() {
        for (text, finish, target) in [("甲乙丙丁戊己庚辛壬癸", "length", 10), ("甲乙", "stop", 10)]
        {
            let (_temp, conn, owner, request) = draft_commit_fixture(text, finish, target);
            assert_eq!(
                owner.commit_draft(&conn, &request).unwrap_err(),
                "Error: GENERATION_DRAFT_INCOMPLETE"
            );
            assert_eq!(draft_count(&conn), 0);
        }
    }

    #[test]
    fn draft_commit_rejects_source_drift_before_writing_test() {
        let (_temp, conn, owner, request) =
            draft_commit_fixture("甲乙丙丁戊己庚辛壬癸", "stop", 10);
        conn.execute(
            "UPDATE blueprints SET title='合成变化' WHERE chapter_number=1",
            [],
        )
        .unwrap();
        assert_eq!(
            owner.commit_draft(&conn, &request).unwrap_err(),
            "Error: GENERATION_SOURCE_CHANGED"
        );
        assert_eq!(draft_count(&conn), 0);
    }

    #[test]
    fn draft_commit_receipt_write_failure_rolls_back_body_test() {
        let (_temp, conn, owner, request) =
            draft_commit_fixture("甲乙丙丁戊己庚辛壬癸", "stop", 10);
        let contents_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM contents", [], |row| row.get(0))
            .unwrap();
        conn.execute_batch("CREATE TEMP TRIGGER reject_draft_receipt BEFORE UPDATE OF usage_receipt_json ON generation_attempts
            WHEN json_extract(NEW.usage_receipt_json,'$.draftCommit') IS NOT NULL
            BEGIN SELECT RAISE(ABORT,'合成收据写入失败'); END;").unwrap();
        assert_eq!(
            owner.commit_draft(&conn, &request).unwrap_err(),
            "Error: GENERATION_REQUEST_FAILED"
        );
        assert!(conn.is_autocommit());
        assert_eq!(draft_count(&conn), 0);
        let contents_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM contents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(contents_after, contents_before);
        conn.execute_batch("DROP TRIGGER reject_draft_receipt")
            .unwrap();
        assert_eq!(
            owner.commit_draft(&conn, &request).unwrap()["success"],
            true
        );
    }

    #[test]
    fn draft_commit_shares_caller_transaction_and_edited_body_invalidates_replay_test() {
        let (_temp, conn, owner, request) =
            draft_commit_fixture("甲乙丙丁戊己庚辛壬癸", "stop", 10);
        {
            let tx = rusqlite::Transaction::new_unchecked(
                &conn,
                rusqlite::TransactionBehavior::Immediate,
            )
            .unwrap();
            owner.commit_draft(&tx, &request).unwrap();
            assert!(!tx.is_autocommit());
            assert_eq!(draft_count(&tx), 1);
            // 守卫释放后整笔效应回滚，不能提前提交。
        }
        assert_eq!(draft_count(&conn), 0);
        let receipt = owner.commit_draft(&conn, &request).unwrap();
        conn.execute("UPDATE contents SET body='人工合成修改' WHERE id=(SELECT content_id FROM drafts WHERE id=?)",
            [receipt["id"].as_i64().unwrap()]).unwrap();
        assert_eq!(
            owner.commit_draft(&conn, &request).unwrap_err(),
            "Error: GENERATION_DRAFT_RECEIPT_INVALID"
        );
    }

    #[test]
    fn draft_commit_nested_failure_rolls_back_even_when_caller_commits_test() {
        let (_temp, conn, owner, request) =
            draft_commit_fixture("甲乙丙丁戊己庚辛壬癸", "stop", 10);
        conn.execute_batch("CREATE TEMP TRIGGER reject_nested_receipt BEFORE UPDATE OF usage_receipt_json ON generation_attempts
            WHEN json_extract(NEW.usage_receipt_json,'$.draftCommit') IS NOT NULL
            BEGIN SELECT RAISE(ABORT,'合成失败'); END;").unwrap();
        let tx =
            rusqlite::Transaction::new_unchecked(&conn, rusqlite::TransactionBehavior::Immediate)
                .unwrap();
        assert!(owner.commit_draft(&tx, &request).is_err());
        assert_eq!(draft_count(&tx), 0);
        tx.commit().unwrap();
        assert_eq!(draft_count(&conn), 0);
    }

    #[test]
    fn draft_commit_rejects_cancelled_action_and_wrong_epoch_test() {
        let (_temp, conn, mut owner, request) =
            draft_commit_fixture("甲乙丙丁戊己庚辛壬癸", "stop", 10);
        owner.session.lease_id = "new-epoch".to_string();
        assert_eq!(
            owner.commit_draft(&conn, &request).unwrap_err(),
            "Error: GENERATION_EPOCH_STALE"
        );
        owner.session.lease_id = "lease-1".to_string();
        owner
            .repository
            .pause(
                &conn,
                request["handle"]["rootActionId"].as_str().unwrap(),
                true,
            )
            .unwrap();
        assert_eq!(
            owner.commit_draft(&conn, &request).unwrap_err(),
            "Error: GENERATION_ACTION_CANCELLED"
        );
        assert_eq!(draft_count(&conn), 0);
    }

    #[test]
    fn draft_commit_preserves_candidate_dependencies_and_rejects_drift_test() {
        let (_temp, conn, owner, request) =
            draft_commit_source_fixture("甲乙丙丁戊己庚辛壬癸", "stop", 10, true);
        let receipt = owner.commit_draft(&conn, &request).unwrap();
        let dependencies: String = conn
            .query_row(
                "SELECT source_dependencies FROM drafts WHERE id=?",
                [receipt["id"].as_i64().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        let dependencies: Value = serde_json::from_str(&dependencies).unwrap();
        assert_eq!(dependencies.as_array().unwrap().len(), 1);
        assert_eq!(dependencies[0]["kind"], "candidate");
        assert_eq!(dependencies[0]["draftId"], 1);
        assert_eq!(dependencies[0]["contentHash"], text_hash("合成前章"));
        let (_temp, conn, owner, request) =
            draft_commit_source_fixture("甲乙丙丁戊己庚辛壬癸", "stop", 10, true);
        conn.execute("UPDATE contents SET body='合成前章变化' WHERE id=(SELECT content_id FROM drafts WHERE id=1)", []).unwrap();
        assert_eq!(
            owner.commit_draft(&conn, &request).unwrap_err(),
            "Error: GENERATION_SOURCE_CHANGED"
        );
        assert_eq!(draft_count(&conn), 1);
    }

    fn begin_root_selection() -> Value {
        json!({"operation":"brainstorm","uiActionNonce":"begin-root","modelId":"fixture",
            "selectedDraftIds":[],"selectedFinalizedDraftIds":[],"selectedBlueprintChapterNumbers":[],
            "promptKeys":["edit_selected_text"],"skillStages":[],"output":"visible-text"})
    }

    fn begin_run_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM generation_runs", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn begin_root_creates_real_binding_safe_receipt_and_preserves_lease_on_replay_test() {
        let (_temp, conn, owner, _) = source_owner_fixture();
        let selection = begin_root_selection();
        let view = owner.begin(&conn, &selection, false, false).unwrap();
        let handle = &view["handle"];
        let run = owner
            .repository
            .get(&conn, handle["runId"].as_str().unwrap())
            .unwrap();
        assert_eq!(run.binding.project_id, "project-1");
        assert_eq!(run.binding.epoch, "lease-1");
        assert_eq!(run.binding.source_manifest["operation"], "brainstorm");
        assert_eq!(
            run.binding.source_manifest["outputContract"],
            "visible-text"
        );
        assert_eq!(
            owner.rebuild_binding(&conn, &run.binding).unwrap(),
            run.binding
        );
        for private in [
            "synthetic-only",
            "apiKey",
            "leaseId",
            "baseUrl",
            "createdAt",
            "expiresAt",
        ] {
            assert!(!view.to_string().contains(private), "不得泄漏 {private}");
        }
        let lease = owner.run_leases.lock().unwrap()[&run.run_id].clone();
        let repeated = owner.begin(&conn, &selection, false, false).unwrap();
        assert_eq!(repeated["handle"], view["handle"]);
        assert_eq!(owner.run_leases.lock().unwrap()[&run.run_id], lease);
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 1);
        assert_eq!(begin_run_count(&conn), 2);
        assert!(conn.is_autocommit());
    }

    #[test]
    fn begin_root_output_contract_validates_and_freezes_overrides_test() {
        let (_temp, conn, owner, _) = source_owner_fixture();
        let mut selection = begin_root_selection();
        for invalid in [
            json!(null),
            json!([]),
            json!([{"purpose":"bad purpose","output":"visible-text"}]),
            json!([{"purpose":"review","output":"structured-data","extra":true}]),
            json!([{"purpose":"review","output":"visible-text"},{"purpose":"review","output":"structured-data"}]),
            json!([{"purpose":"review","output":"unknown"}]),
        ] {
            selection["outputOverrides"] = invalid;
            assert_eq!(
                owner.begin(&conn, &selection, false, false).unwrap_err(),
                mutating_error("GENERATION_OUTPUT_CONTRACT_INVALID")
            );
        }
        let overrides = json!([{"purpose":"review","output":"structured-data"}]);
        selection["outputOverrides"] = overrides.clone();
        let view = owner.begin(&conn, &selection, false, false).unwrap();
        let run = owner
            .repository
            .get(&conn, view["handle"]["runId"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            run.binding.source_manifest["outputContract"],
            json!({"primary":"visible-text","overrides":overrides})
        );
        assert_eq!(begin_run_count(&conn), 2);
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 1);
    }

    #[test]
    fn begin_root_write_failure_rolls_back_root_and_releases_lease_test() {
        let (_temp, conn, owner, _) = source_owner_fixture();
        conn.execute_batch(
            "CREATE TEMP TRIGGER reject_begin_run BEFORE INSERT ON generation_runs
            BEGIN SELECT RAISE(ABORT,'合成运行写入失败'); END;",
        )
        .unwrap();
        assert_eq!(
            owner
                .begin(&conn, &begin_root_selection(), false, false)
                .unwrap_err(),
            mutating_error(GENERATION_REQUEST_FAILED)
        );
        assert_eq!(begin_run_count(&conn), 1);
        let roots: i64 = conn
            .query_row("SELECT COUNT(*) FROM generation_roots", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(roots, 1);
        assert!(owner.run_leases.lock().unwrap().is_empty());
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 0);
        assert!(conn.is_autocommit());
    }

    #[test]
    fn begin_root_source_and_preparation_failures_release_new_lease_test() {
        let (_temp, conn, owner, _) = source_owner_fixture();
        let mut selection = begin_root_selection();
        selection["promptKeys"] = json!(["synthetic-missing-prompt"]);
        assert!(owner.begin(&conn, &selection, false, false).is_err());
        selection = begin_root_selection();
        selection["preparationId"] = json!("missing");
        assert_eq!(
            owner.begin(&conn, &selection, false, false).unwrap_err(),
            mutating_error("GENERATION_DRAFT_PREPARATION_REQUIRED")
        );
        assert_eq!(begin_run_count(&conn), 1);
        assert!(owner.run_leases.lock().unwrap().is_empty());
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 0);
        assert!(conn.is_autocommit());
    }

    #[test]
    fn begin_root_rejects_unmigrated_admission_without_creating_budget_test() {
        let (_temp, conn, owner, _) = source_owner_fixture();
        for (field, value) in [
            ("batchId", json!("batch")),
            ("continueDirectoryOperationId", json!("directory")),
            ("reviewRevisionContextId", json!("review")),
            ("finalizedCharacterContextId", json!("finalized")),
            ("importSlot", json!({})),
            ("importExecution", json!({})),
        ] {
            let mut selection = begin_root_selection();
            selection[field] = value;
            assert!(
                owner.begin(&conn, &selection, false, false).is_err(),
                "{field}"
            );
        }
        for operation in [
            "review-chapter",
            "refine-draft",
            "refine-from-review",
            "import-global-facts",
            "import-blueprints",
            "editor-inline",
            "finalized-character-state",
            "plot-tree-snapshot",
            "legacy-character-roster-repair",
            "agent-round",
        ] {
            let mut selection = begin_root_selection();
            selection["operation"] = json!(operation);
            assert!(
                owner.begin(&conn, &selection, false, false).is_err(),
                "{operation}"
            );
        }
        let tx =
            rusqlite::Transaction::new_unchecked(&conn, rusqlite::TransactionBehavior::Immediate)
                .unwrap();
        assert!(owner
            .begin(&tx, &begin_root_selection(), false, false)
            .is_err());
        tx.commit().unwrap();
        assert_eq!(begin_run_count(&conn), 1);
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 0);
    }

    #[test]
    fn begin_root_nonce_conflict_releases_temporary_lease_and_retains_original_test() {
        let (_temp, conn, owner, _) = source_owner_fixture();
        let selection = begin_root_selection();
        let view = owner.begin(&conn, &selection, false, false).unwrap();
        let id = view["handle"]["runId"].as_str().unwrap();
        let original = owner.run_leases.lock().unwrap()[id].clone();
        conn.execute("UPDATE generation_runs SET binding_json=json_set(binding_json,'$.sourceManifest.syntheticConflict',1) WHERE run_id=?", [id]).unwrap();
        assert_eq!(
            owner.begin(&conn, &selection, false, false).unwrap_err(),
            mutating_error("GENERATION_NONCE_CONFLICT")
        );
        assert_eq!(begin_run_count(&conn), 2);
        assert_eq!(owner.run_leases.lock().unwrap()[id], original);
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 1);
    }

    #[test]
    fn begin_root_planning_normalizes_target_and_rejects_invalid_target_test() {
        let (_temp, conn, owner, _) = source_owner_fixture();
        let mut selection = begin_root_selection();
        selection["operation"] = json!("generate-core-seed");
        selection["output"] = json!("structured-data");
        selection["authorInputs"] = json!([{"id":"architecture:author-config","text":"{\"totalChapters\":100}"},
            {"id":"planning:target-units","text":"0x258"}]);
        let view = owner.begin(&conn, &selection, false, false).unwrap();
        let run = owner
            .repository
            .get(&conn, view["handle"]["runId"].as_str().unwrap())
            .unwrap();
        let inputs = run.binding.source_manifest["authorInputs"]
            .as_array()
            .unwrap();
        assert_eq!(
            inputs
                .iter()
                .find(|input| input["id"] == "planning:target-units")
                .unwrap()["text"],
            "600"
        );
        for text in ["invalid", "0x+258", "0", "1001", "1.5"] {
            selection["uiActionNonce"] = json!(format!("invalid-{text}"));
            selection["authorInputs"][1]["text"] = json!(text);
            assert_eq!(
                owner.begin(&conn, &selection, false, false).unwrap_err(),
                mutating_error("GENERATION_PLANNING_TARGET_INVALID")
            );
        }
        selection["authorInputs"][1]["text"] = json!("600");
        selection["authorInputs"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"planning:target-units","text":"600"}));
        assert_eq!(
            owner.begin(&conn, &selection, false, false).unwrap_err(),
            mutating_error("GENERATION_PLANNING_TARGET_INVALID")
        );
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 1);
        assert_eq!(begin_run_count(&conn), 2);
    }

    #[test]
    fn begin_root_prepared_draft_checks_material_and_source_drift_test() {
        let (_temp, conn, mut owner, _) = source_owner_fixture();
        for chapter in 1..=6 {
            conn.execute(
                "INSERT INTO blueprints(chapter_number,title) VALUES (?,'合成蓝图')",
                [chapter],
            )
            .unwrap();
        }
        let request = json!({"chapterNumber":1,"modelId":"fixture","promptKeys":["edit_selected_text"],
            "skillStages":[],"selectedDraftIds":[],"query":"合成查询"});
        let snapshot = json!({"version":1,"state":"empty","storageState":"absent","query":"合成查询",
            "topK":3,"canonicalRevision":null,"documentsRevision":null,"items":[]});
        let prepared = owner
            .prepare_draft_context(&conn, &request, &snapshot, None)
            .unwrap();
        let mut selection = begin_root_selection();
        selection["operation"] = json!("chapter-draft");
        selection["chapterNumber"] = json!(1);
        selection["selectedBlueprintChapterNumbers"] = json!([1, 2, 3, 4, 5, 6]);
        selection["preparationId"] = prepared["preparationId"].clone();
        assert_eq!(
            owner.begin(&conn, &selection, false, false).unwrap_err(),
            mutating_error("GENERATION_MATERIAL_DECISION_REQUIRED")
        );
        selection["materialDecision"] = json!({"version":1,"verdict":"admitted","promptHash":text_hash("合成提示"),
            "capacity":{"maxInputUnits":1000,"methodVersion":"utf8-bytes-v1","admittedUnits":12},
            "coverage":{"required":1,"included":1,"complete":true},
            "included":[{"sourceId":"author:required","revision":1,"contentHash":text_hash("合成作者"),
                "category":"author","required":true,"units":12}],"omitted":[]});
        let view = owner.begin(&conn, &selection, false, false).unwrap();
        let run = owner
            .repository
            .get(&conn, view["handle"]["runId"].as_str().unwrap())
            .unwrap();
        assert_eq!(run.binding.source_manifest["knowledgeSnapshot"], snapshot);
        conn.execute(
            "UPDATE blueprints SET title='漂移合成蓝图' WHERE chapter_number=1",
            [],
        )
        .unwrap();
        selection["uiActionNonce"] = json!("prepared-drift");
        assert_eq!(
            owner.begin(&conn, &selection, false, false).unwrap_err(),
            mutating_error("GENERATION_DRAFT_PREPARATION_CHANGED")
        );
        assert_eq!(begin_run_count(&conn), 2);
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 1);
    }

    #[test]
    fn begin_root_planning_defaults_and_authoritative_range_guard_test() {
        let (_temp, conn, owner, _) = source_owner_fixture();
        let mut selection = begin_root_selection();
        selection["operation"] = json!("generate-core-seed");
        selection["output"] = json!("structured-data");
        let view = owner.begin(&conn, &selection, false, false).unwrap();
        let run = owner
            .repository
            .get(&conn, view["handle"]["runId"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            run.binding.source_manifest["authorInputs"],
            json!([{"id":"planning:target-units","text":"600"}])
        );
        conn.execute("UPDATE project_core SET total_chapters=2", [])
            .unwrap();
        selection["operation"] = json!("generate-plot-outline");
        selection["authorInputs"] = json!([
            {"id":"architecture:author-config","text":"{\"totalChapters\":100}"},
            {"id":"architecture:planning-intent","text":"{\"version\":\"architecture-action-v1\",\"priorSteps\":[],\"synopsisRange\":{\"from\":1,\"to\":3}}"}]);
        assert_eq!(
            owner.begin(&conn, &selection, false, false).unwrap_err(),
            mutating_error("GENERATION_PLANNING_RANGE_INVALID")
        );
        assert_eq!(begin_run_count(&conn), 2);
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 1);
    }

    #[test]
    fn begin_root_transaction_entry_preserves_caller_rollback_test() {
        let (_temp, conn, owner, prior) = source_owner_fixture();
        let request = OpenGenerationRunRequest {
            project_id: prior.binding.project_id,
            epoch: prior.binding.epoch,
            fingerprint: prior.binding.fingerprint,
            context_snapshot_id: prior.binding.context_snapshot_id,
            source_manifest: prior.binding.source_manifest,
            source_refs: prior.binding.source_refs,
            operation: "brainstorm".to_string(),
            ui_action_nonce: "outer-open".to_string(),
            frozen_input_hash: fake_content_hash(),
            budget: crate::services::main_generation_plan::main_generation_policy().budget,
            parent_root_action_id: None,
        };
        assert!(owner
            .run_service
            .open_in_transaction(&conn, &request)
            .is_err());
        let tx =
            rusqlite::Transaction::new_unchecked(&conn, rusqlite::TransactionBehavior::Immediate)
                .unwrap();
        owner
            .run_service
            .open_in_transaction(&tx, &request)
            .unwrap();
        assert_eq!(begin_run_count(&tx), 2);
        tx.rollback().unwrap();
        assert_eq!(begin_run_count(&conn), 1);
    }

    #[test]
    fn begin_root_reopen_replays_old_epoch_without_allocating_another_root_or_lease_test() {
        let (_temp, conn, mut owner, _) = source_owner_fixture();
        let selection = begin_root_selection();
        let view = owner.begin(&conn, &selection, false, false).unwrap();
        owner.pause(&conn, &view["handle"]).unwrap();
        owner.suspend_with_database(&conn).unwrap();
        let mut reopened = MainGenerationOwner::new(ProjectSessionContext {
            project_id: "project-1".to_string(),
            lease_id: "lease-2".to_string(),
            project_path: owner.session.project_path.clone(),
        });
        reopened.global_data_root = owner.global_data_root.clone();
        let repeated = reopened.begin(&conn, &selection, false, false).unwrap();
        assert_eq!(repeated["handle"], view["handle"]);
        assert_eq!(begin_run_count(&conn), 2);
        assert!(reopened.run_leases.lock().unwrap().is_empty());
        assert_eq!(reopened.lease_store.lock().unwrap().live_lease_count(), 0);
        let resumed = reopened.resume(&conn, &repeated["handle"]).unwrap();
        assert_eq!(resumed["handle"]["epoch"], "lease-2");
        assert_eq!(reopened.lease_store.lock().unwrap().live_lease_count(), 1);
        assert_eq!(begin_run_count(&conn), 2);
    }

    fn source_owner_fixture() -> (
        crate::services::main_generation_sources::tests::TempRoot,
        Connection,
        MainGenerationOwner,
        DurableGenerationRun,
    ) {
        let (temp, conn, binding, global, project) =
            crate::services::main_generation_sources::tests::fixture("zh-CN");
        let mut session = session("project-1", "lease-1");
        session.project_path = project.to_string_lossy().into_owned();
        let mut owner = MainGenerationOwner::new(session);
        owner.global_data_root = global;
        let policy = crate::services::main_generation_plan::read_main_generation_policy(
            binding.source_manifest.get("policy").unwrap(),
        )
        .unwrap();
        let run = owner
            .repository
            .open(
                &conn,
                &OpenGenerationRunRequest {
                    project_id: binding.project_id,
                    epoch: binding.epoch,
                    fingerprint: binding.fingerprint,
                    context_snapshot_id: binding.context_snapshot_id,
                    source_manifest: binding.source_manifest,
                    source_refs: binding.source_refs,
                    operation: "brainstorm".to_string(),
                    ui_action_nonce: "source-fixture".to_string(),
                    frozen_input_hash: fake_content_hash(),
                    budget: policy.budget,
                    parent_root_action_id: None,
                },
            )
            .unwrap();
        (temp, conn, owner, run)
    }

    #[test]
    fn owner_resume_rebuilds_epoch_and_replaces_execution_lease_test() {
        let (_temp, conn, mut owner, run) = source_owner_fixture();
        owner
            .repository
            .pause(&conn, &run.root_action_id, false)
            .unwrap();
        owner.session.lease_id = "lease-2".to_string();
        owner.resume(&conn, &handle_of(&run)).unwrap();
        let resumed = owner.repository.get(&conn, &run.run_id).unwrap();
        assert_eq!(resumed.binding.epoch, "lease-2");
        let first = owner.run_leases.lock().unwrap()[&run.run_id].clone();
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 1);
        // 已开放的根不能重复恢复；拒绝时新租约回收，原租约仍有效。
        assert_eq!(
            owner.resume(&conn, &handle_of(&resumed)),
            Err(mutating_error("GENERATION_RESUME_REFUSED"))
        );
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 1);
        assert_eq!(owner.run_leases.lock().unwrap()[&run.run_id], first);
        owner
            .repository
            .pause(&conn, &run.root_action_id, false)
            .unwrap();
        owner.resume(&conn, &handle_of(&resumed)).unwrap();
        let replacement = owner.run_leases.lock().unwrap()[&run.run_id].clone();
        assert_ne!(first, replacement);
        let mut store = owner.lease_store.lock().unwrap();
        assert_eq!(store.live_lease_count(), 1);
        assert_eq!(
            store.resolve_model_at(&first, owner_now_ms()),
            Err("模型执行租约无效".to_string())
        );
        drop(store);
        owner.suspend_with_database(&conn).unwrap();
        assert!(owner.closed);
        assert!(owner.run_leases.lock().unwrap().is_empty());
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 0);
    }

    #[test]
    fn owner_resume_author_drift_cleans_temporary_lease_and_preserves_binding_test() {
        let (_temp, conn, owner, run) = source_owner_fixture();
        owner
            .repository
            .pause(&conn, &run.root_action_id, false)
            .unwrap();
        crate::repositories::project_core_repository::update(
            &conn,
            json!({"globalGuidance":"变化的合成事实"})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            owner.resume(&conn, &handle_of(&run)),
            Err(mutating_error("GENERATION_RECOVERY_UNAUTHORIZED"))
        );
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 0);
        assert!(owner.run_leases.lock().unwrap().is_empty());
        assert_eq!(
            owner.repository.get(&conn, &run.run_id).unwrap().binding,
            run.binding
        );
    }

    #[test]
    fn owner_resume_missing_model_does_not_issue_execution_lease_test() {
        let (_temp, conn, owner, run) = source_owner_fixture();
        std::fs::write(
            crate::app_paths::models_config_path_at(&owner.global_data_root),
            "[]",
        )
        .unwrap();
        assert_eq!(
            owner.resume(&conn, &handle_of(&run)),
            Err(mutating_error("GENERATION_MODEL_MISSING"))
        );
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 0);
    }

    #[test]
    fn temporary_owner_lease_closes_when_not_attached_test() {
        let (_temp, _conn, owner, _run) = source_owner_fixture();
        let lease = owner.acquire_model_lease("fixture").unwrap();
        let id = lease.receipt.lease_id.clone();
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 1);
        drop(lease);
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 0);
        assert_eq!(
            owner
                .lease_store
                .lock()
                .unwrap()
                .resolve_model_at(&id, owner_now_ms()),
            Err("模型执行租约无效".to_string())
        );
    }

    #[test]
    fn failed_owner_suspend_keeps_lease_until_successful_retry_test() {
        let (_temp, conn, mut owner, run) = source_owner_fixture();
        owner
            .acquire_model_lease("fixture")
            .unwrap()
            .attach(&owner, &run.run_id);
        let unavailable = Connection::open_in_memory().unwrap();
        assert!(owner.suspend_with_database(&unavailable).is_err());
        assert!(!owner.closed);
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 1);
        owner.suspend_with_database(&conn).unwrap();
        assert!(owner.closed);
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 0);
    }

    #[test]
    fn before_close_failure_keeps_cached_owner_for_retry_test() {
        let cache = MainGenerationOwnerCache::new();
        let active = active();
        cache
            .authorized_owner(&session("project-1", "lease-1"), Some(&active))
            .unwrap();
        assert_eq!(
            cache.before_database_close(Some(&active.root_path), |_| Err::<(), _>(
                "合成失败".to_string()
            )),
            Err("合成失败".to_string())
        );
        assert_eq!(cache.len(), 1);
        cache
            .before_database_close(Some(&active.root_path), |owner| {
                owner.unwrap().suspend_for_project_close();
                Ok(())
            })
            .unwrap();
        assert!(cache.is_empty());
    }

    #[test]
    fn real_host_current_capture_and_source_assertion_reject_author_drift_test() {
        let (_temp, conn, owner, run) = source_owner_fixture();
        assert!(AgentHost::current(&owner, &conn, &run, None));
        assert_eq!(
            AgentHost::capture(&owner, &conn, &run).unwrap(),
            run.binding
        );
        owner
            .assert_sources_current(&conn, &handle_of(&run), None)
            .unwrap();
        crate::repositories::project_core_repository::update(
            &conn,
            json!({"globalGuidance":"新的合成约束"})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        assert!(!AgentHost::current(&owner, &conn, &run, None));
        let captured = AgentHost::capture(&owner, &conn, &run).unwrap();
        assert_ne!(captured.fingerprint, run.binding.fingerprint);
        assert!(AgentHost::current(&owner, &conn, &run, Some(&captured)));
        assert_eq!(
            owner
                .assert_sources_current(&conn, &handle_of(&run), None)
                .unwrap_err(),
            mutating_error("GENERATION_SOURCE_CHANGED")
        );
    }

    #[test]
    fn real_host_model_missing_and_owner_closed_fail_closed_test() {
        let (_temp, conn, mut owner, run) = source_owner_fixture();
        std::fs::write(
            crate::app_paths::models_config_path_at(&owner.global_data_root),
            "[]",
        )
        .unwrap();
        assert!(!AgentHost::current(&owner, &conn, &run, None));
        assert_eq!(
            AgentHost::capture(&owner, &conn, &run).unwrap_err(),
            mutating_error("GENERATION_MODEL_MISSING")
        );
        owner.suspend_for_project_close();
        assert!(!AgentHost::current(&owner, &conn, &run, None));
        assert_eq!(
            AgentHost::capture(&owner, &conn, &run).unwrap_err(),
            mutating_error(GENERATION_OWNER_CLOSED)
        );
    }

    #[test]
    fn real_host_capture_shares_outer_child_effect_transaction_test() {
        let (_temp, conn, owner, run) = source_owner_fixture();
        let transaction = conn.unchecked_transaction().unwrap();
        let before = AgentHost::capture(&owner, &transaction, &run).unwrap();
        crate::repositories::project_core_repository::update(
            &transaction,
            json!({"globalGuidance":"事务内合成效应"})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        let after = AgentHost::capture(&owner, &transaction, &run).unwrap();
        assert_ne!(before.fingerprint, after.fingerprint);
        assert!(!conn.is_autocommit(), "源读取不得提交调用方事务");
        transaction.rollback().unwrap();
        assert_eq!(AgentHost::capture(&owner, &conn, &run).unwrap(), before);
    }

    #[test]
    fn draft_preparation_binding_freezes_real_six_chapter_sources_without_opening_run_test() {
        let (_temp, conn, owner, _) = source_owner_fixture();
        for chapter in 1..=6 {
            let blueprint = serde_json::from_value(
                json!({"chapterNumber":chapter, "title":format!("合成蓝图{chapter}")}),
            )
            .unwrap();
            crate::repositories::blueprint_repository::upsert(&conn, &blueprint).unwrap();
        }
        let request = json!({"chapterNumber":1, "modelId":"fixture", "promptKeys":["edit_selected_text"],
            "skillStages":[], "selectedDraftIds":[], "query":"合成查询"});
        let binding = owner
            .draft_preparation_binding(&conn, &request, None)
            .unwrap();
        assert_eq!(
            binding
                .source_manifest
                .get("selectedBlueprintChapterNumbers")
                .unwrap(),
            &json!([1, 2, 3, 4, 5, 6])
        );
        assert_eq!(
            binding.source_manifest.get("outputContract").unwrap(),
            &json!("visible-text")
        );
        assert_eq!(owner.rebuild_binding(&conn, &binding).unwrap(), binding);
        let count: i64 = conn
            .query_row("SELECT count(*) FROM generation_runs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "准备不得新建运行");
        let changed =
            serde_json::from_value(json!({"chapterNumber":1, "title":"更新后的合成蓝图"})).unwrap();
        crate::repositories::blueprint_repository::upsert(&conn, &changed).unwrap();
        let next = owner
            .draft_preparation_binding(&conn, &request, None)
            .unwrap();
        assert_ne!(binding.fingerprint, next.fingerprint);
        assert!(!preparation_matches(&binding, &next, true));
    }

    #[test]
    fn draft_preparation_binding_rejects_query_shape_and_missing_model_test() {
        let (_temp, conn, owner, _) = source_owner_fixture();
        let request = json!({"chapterNumber":1, "modelId":"missing", "promptKeys":["edit_selected_text"],
            "skillStages":[], "selectedDraftIds":[], "query":"合成查询"});
        assert_eq!(
            owner
                .draft_preparation_binding(&conn, &request, Some(&json!({"query":"不同查询"})))
                .unwrap_err(),
            mutating_error("GENERATION_DRAFT_PREPARATION_INVALID")
        );
        assert_eq!(
            owner
                .draft_preparation_binding(&conn, &request, None)
                .unwrap_err(),
            mutating_error("GENERATION_MODEL_MISSING")
        );
    }

    fn session(project_id: &str, lease_id: &str) -> ProjectSessionContext {
        ProjectSessionContext {
            project_id: project_id.to_string(),
            lease_id: lease_id.to_string(),
            project_path: r"F:\novel\demo".to_string(),
        }
    }

    fn active() -> ActiveProject {
        ActiveProject {
            project_id: "project-1".to_string(),
            lease_id: "lease-1".to_string(),
            root_path: r"F:\novel\demo".to_string(),
        }
    }

    fn active_with_lease(lease_id: &str) -> ActiveProject {
        ActiveProject {
            project_id: "project-1".to_string(),
            lease_id: lease_id.to_string(),
            root_path: r"F:\novel\demo".to_string(),
        }
    }

    #[test]
    fn execution_abort_channel_does_not_wait_for_owner_lock_and_cleans_up_test() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        let run = open_run(&conn, "project-1", "lease-1", "chapter-draft").unwrap();
        let cache = MainGenerationOwnerCache::new();
        let session = session("project-1", "lease-1");
        let active = active();
        let mut owner_guard = cache.authorized_owner(&session, Some(&active)).unwrap();
        let owner = owner_guard.owner();
        let control = cache
            .register_execution_control(owner, &conn, &handle_of(&run))
            .unwrap();
        let handle: MainGenerationRunHandle = serde_json::from_value(handle_of(&run)).unwrap();
        // owner 锁被当前线程持有，中止必须只取独立短锁，不能自死锁。
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    cache
                        .request_execution_abort(&session, Some(&active), &handle)
                        .unwrap()
                })
                .join()
                .unwrap();
        });
        assert!(owner.run_service.has_abort_request(&run.root_action_id));
        drop(control);
        assert!(cache.execution_controls.lock().unwrap().is_empty());
        assert!(!owner.run_service.has_abort_request(&run.root_action_id));
    }

    #[test]
    fn execution_abort_channel_rejects_stale_session_and_ignores_tampered_handle_test() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        let run = open_run(&conn, "project-1", "lease-1", "chapter-draft").unwrap();
        let cache = MainGenerationOwnerCache::new();
        let session = session("project-1", "lease-1");
        let active = active();
        let mut owner_guard = cache.authorized_owner(&session, Some(&active)).unwrap();
        let owner = owner_guard.owner();
        let _control = cache
            .register_execution_control(owner, &conn, &handle_of(&run))
            .unwrap();
        let handle: MainGenerationRunHandle = serde_json::from_value(handle_of(&run)).unwrap();
        let mut stale = session.clone();
        stale.lease_id = "old-lease".to_string();
        assert!(cache
            .request_execution_abort(&stale, Some(&active), &handle)
            .is_err());
        let mut foreign = session.clone();
        foreign.project_path = r"F:\novel\other".to_string();
        assert!(cache
            .request_execution_abort(&foreign, Some(&active), &handle)
            .is_err());
        for field in ["projectId", "epoch", "rootActionId", "runId"] {
            let mut changed = handle_of(&run);
            changed[field] = json!("tampered");
            let changed = serde_json::from_value(changed).unwrap();
            cache
                .request_execution_abort(&session, Some(&active), &changed)
                .unwrap();
            assert!(!owner.run_service.has_abort_request(&run.root_action_id));
        }
        assert!(!owner.run_service.has_abort_request(&run.root_action_id));
    }

    #[test]
    fn project_close_fence_counts_overlapping_requests_and_reopens_on_exit_test() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        let run = open_run(&conn, "project-1", "lease-1", "chapter-draft").unwrap();
        let owner = owner_with(&conn);
        let cache = MainGenerationOwnerCache::new();
        let first = cache
            .register_project_close(&owner.session.project_path)
            .unwrap();
        let second = cache
            .register_project_close(&owner.session.project_path)
            .unwrap();
        assert_eq!(
            cache
                .register_execution_control(&owner, &conn, &handle_of(&run))
                .err(),
            Some(mutating_error("GENERATION_PROJECT_CLOSED"))
        );
        drop(first);
        assert!(cache
            .register_execution_control(&owner, &conn, &handle_of(&run))
            .is_err());
        drop(second);
        let execution = cache
            .register_execution_control(&owner, &conn, &handle_of(&run))
            .unwrap();
        drop(execution);
        assert!(cache.closing_projects.lock().unwrap().is_empty());
    }

    #[test]
    fn project_close_fence_does_not_block_other_project_test() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        let run = open_run(&conn, "project-1", "lease-1", "chapter-draft").unwrap();
        let owner = owner_with(&conn);
        let cache = MainGenerationOwnerCache::new();
        let _closing = cache.register_project_close(r"F:\novel\other").unwrap();
        let _execution = cache
            .register_execution_control(&owner, &conn, &handle_of(&run))
            .unwrap();
        assert!(!owner.run_service.has_abort_request(&run.root_action_id));
    }

    #[test]
    fn project_close_callback_failure_keeps_owner_and_releases_fence_test() {
        let cache = MainGenerationOwnerCache::new();
        let active = active();
        cache
            .authorized_owner(&session("project-1", "lease-1"), Some(&active))
            .unwrap();
        let result = cache.before_database_close(Some(&active.root_path), |_| {
            Err::<(), _>("合成关闭失败".to_string())
        });
        assert_eq!(result, Err("合成关闭失败".to_string()));
        assert_eq!(cache.len(), 1);
        assert!(cache.closing_projects.lock().unwrap().is_empty());
        let mut guard = cache
            .authorized_owner(&session("project-1", "lease-1"), Some(&active))
            .unwrap();
        assert!(!guard.owner().closed);
    }

    #[test]
    fn execution_control_registration_rejects_invalid_identity_without_entry_test() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        let run = open_run(&conn, "project-1", "lease-1", "chapter-draft").unwrap();
        let cache = MainGenerationOwnerCache::new();
        let owner = owner_with(&conn);
        let mut handle = handle_of(&run);
        handle["rootActionId"] = json!("tampered");
        assert!(cache
            .register_execution_control(&owner, &conn, &handle)
            .is_err());
        assert!(cache.execution_controls.lock().unwrap().is_empty());
    }

    #[test]
    fn execution_abort_unblocks_durable_cancel_with_owner_and_database_locked_test() {
        use crate::services::generation_run_service::{
            create_generation_run_service, DispatchOptions, ExecuteGenerationRequest,
            GenerationDispatchResult, GenerationDispatcher, GenerationRunServiceDependencies,
            VisibleEventKind, VisibleGenerationEvent,
        };
        struct WaitingProvider(std::sync::mpsc::Sender<()>);
        impl GenerationDispatcher for WaitingProvider {
            fn dispatch<'a>(
                &'a self,
                _request: &'a Value,
                options: DispatchOptions<'a>,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = Result<GenerationDispatchResult, String>>
                        + Send
                        + 'a,
                >,
            > {
                Box::pin(async move {
                    (options.on_visible)(VisibleGenerationEvent {
                        kind: VisibleEventKind::Delta,
                        text: "已收到正文".to_string(),
                        event_id: None,
                    });
                    self.0.send(()).unwrap();
                    std::future::pending().await
                })
            }
        }
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        let run = open_run(&conn, "project-1", "lease-1", "chapter-draft").unwrap();
        let database = Arc::new(Mutex::new(conn));
        let cache = Arc::new(MainGenerationOwnerCache::new());
        let session = session("project-1", "lease-1");
        let active = active();
        let (started, wait_started) = std::sync::mpsc::channel();
        {
            let mut guard = cache.authorized_owner(&session, Some(&active)).unwrap();
            guard.owner().run_service = Arc::new(create_generation_run_service(
                GenerationRunServiceDependencies {
                    repository: GenerationRunRepository::default(),
                    dispatch: Arc::new(WaitingProvider(started)),
                    on_snapshot: None,
                    on_durable_snapshot: None,
                    on_reasoning: None,
                    validate_recovery: None,
                },
            ));
        }
        let execution_cache = Arc::clone(&cache);
        let execution_database = Arc::clone(&database);
        let execution_session = session.clone();
        let execution_active = active.clone();
        let execution_run = run.clone();
        let (done, wait_done) = std::sync::mpsc::channel();
        let execution =
            std::thread::spawn(move || {
                let mut guard = execution_cache
                    .authorized_owner(&execution_session, Some(&execution_active))
                    .unwrap();
                let owner = guard.owner();
                let conn = execution_database.lock().unwrap();
                let _control = execution_cache
                    .register_execution_control(owner, &conn, &handle_of(&execution_run))
                    .unwrap();
                let receipt = owner.run_service.execute_blocking(&conn, ExecuteGenerationRequest {
                run_id: execution_run.run_id,
                invocation_nonce: "cancel-lock-test".to_string(),
                request_hash: "a".repeat(64),
                provider_request: Value::Null,
                reserved_tokens: 1024,
                requested_output_tokens: 256,
                input_upper_bound_tokens: 128,
                reasoning_upper_bound_tokens: 0,
                usage_policy: crate::repositories::generation_run_repository::ProviderUsagePolicy {
                    estimator_version: "fixture".to_string(),
                    safety_margin_tokens: 512,
                    reasoning: "included-in-completion".to_string(),
                    can_bound_total_liability: true,
                },
                purpose: None,
                budget_decision: None,
                agent_tool_names: None,
                replay_task: None,
                reconciliation_injected: false,
            }).unwrap();
                done.send(receipt).unwrap();
            });
        wait_started
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("分发开始");
        assert!(cache.entries.try_lock().is_err());
        assert!(database.try_lock().is_err());
        let handle = serde_json::from_value(handle_of(&run)).unwrap();
        cache
            .request_execution_abort(&session, Some(&active), &handle)
            .unwrap();
        let receipt = wait_done
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("中止必须释放网络等待，不依赖业务锁");
        execution.join().unwrap();
        assert_eq!(
            receipt.failure_code.as_deref(),
            Some("GENERATION_CANCELLED")
        );
        assert_eq!(receipt.artifact.as_ref().unwrap().text, "已收到正文");
        let mut guard = cache.authorized_owner(&session, Some(&active)).unwrap();
        let conn = database.lock().unwrap();
        let view = guard.owner().cancel(&conn, &handle_of(&run)).unwrap();
        assert_eq!(view["status"], json!("cancelled"));
        assert!(cache.execution_controls.lock().unwrap().is_empty());
    }

    #[test]
    fn project_close_unblocks_locked_execution_and_reopens_durable_receipt_test() {
        assert_locked_project_close(false);
    }

    #[test]
    fn project_close_failure_preserves_session_database_and_retryable_owner_test() {
        assert_locked_project_close(true);
    }

    /// 真实 AppState/磁盘项目库关闭链路；provider 是本地合成挂起 Future，不访问网络。
    fn assert_locked_project_close(fail_close: bool) {
        use crate::services::generation_run_service::{
            create_generation_run_service, DispatchOptions, ExecuteGenerationRequest,
            GenerationDispatchResult, GenerationDispatcher, GenerationRunServiceDependencies,
            VisibleEventKind, VisibleGenerationEvent,
        };
        struct WaitingProvider(std::sync::mpsc::Sender<()>);
        impl GenerationDispatcher for WaitingProvider {
            fn dispatch<'a>(
                &'a self,
                _request: &'a Value,
                options: DispatchOptions<'a>,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = Result<GenerationDispatchResult, String>>
                        + Send
                        + 'a,
                >,
            > {
                Box::pin(async move {
                    (options.on_visible)(VisibleGenerationEvent {
                        kind: VisibleEventKind::Delta,
                        text: "关闭前正文".to_string(),
                        event_id: None,
                    });
                    self.0.send(()).unwrap();
                    std::future::pending().await
                })
            }
        }
        let (_temp, _fixture_conn, fixture_owner, _) = source_owner_fixture();
        let session = fixture_owner.session.clone();
        let active = ActiveProject {
            project_id: session.project_id.clone(),
            lease_id: session.lease_id.clone(),
            root_path: session.project_path.clone(),
        };
        let state = Arc::new(crate::state::AppState::new());
        state.activate_project(active.clone()).unwrap();
        let run = state.with_project_db(|conn| {
            let run = open_run(conn, &session.project_id, &session.lease_id, "chapter-draft")?;
            if fail_close {
                // 仅阻止关闭阶段的持久暂停，不阻止网络中止的 flush/settle。
                conn.execute_batch(
                    "CREATE TRIGGER reject_project_close BEFORE UPDATE OF status ON generation_runs
                     WHEN NEW.status='paused' BEGIN SELECT RAISE(ABORT, 'synthetic-close-failure'); END;"
                ).unwrap();
            }
            Ok(run)
        }).unwrap();
        let (started, wait_started) = std::sync::mpsc::channel();
        {
            let mut guard = state
                .generation_owners
                .authorized_owner(&session, Some(&active))
                .unwrap();
            guard.owner().run_service = Arc::new(create_generation_run_service(
                GenerationRunServiceDependencies {
                    repository: GenerationRunRepository::default(),
                    dispatch: Arc::new(WaitingProvider(started)),
                    on_snapshot: None,
                    on_durable_snapshot: None,
                    on_reasoning: None,
                    validate_recovery: None,
                },
            ));
        }
        let execution_state = Arc::clone(&state);
        let execution_session = session.clone();
        let execution_active = active.clone();
        let execution_run = run.clone();
        let (done, wait_done) = std::sync::mpsc::channel();
        let execution =
            std::thread::spawn(move || {
                let mut owner_guard = execution_state
                    .generation_owners
                    .authorized_owner(&execution_session, Some(&execution_active))
                    .unwrap();
                let owner = owner_guard.owner();
                let database = execution_state.project_db.lock().unwrap();
                let conn = database.as_ref().unwrap().connection();
                let _control = execution_state
                    .generation_owners
                    .register_execution_control(owner, conn, &handle_of(&execution_run))
                    .unwrap();
                let receipt = owner.run_service.execute_blocking(conn, ExecuteGenerationRequest {
                run_id: execution_run.run_id,
                invocation_nonce: "project-close-lock-test".to_string(),
                request_hash: "f".repeat(64),
                provider_request: Value::Null,
                reserved_tokens: 1024,
                requested_output_tokens: 256,
                input_upper_bound_tokens: 128,
                reasoning_upper_bound_tokens: 0,
                usage_policy: crate::repositories::generation_run_repository::ProviderUsagePolicy {
                    estimator_version: "fixture".to_string(),
                    safety_margin_tokens: 512,
                    reasoning: "included-in-completion".to_string(),
                    can_bound_total_liability: true,
                },
                purpose: None,
                budget_decision: None,
                agent_tool_names: None,
                replay_task: None,
                reconciliation_injected: false,
            }).unwrap();
                done.send(receipt).unwrap();
            });
        wait_started
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("分发开始");
        assert!(state.generation_owners.entries.try_lock().is_err());
        assert!(state.project_db.try_lock().is_err());
        let close_state = Arc::clone(&state);
        let (closed, wait_closed) = std::sync::mpsc::channel();
        let closing = std::thread::spawn(move || {
            closed.send(close_state.close_project_database()).unwrap();
        });
        let receipt = wait_done
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("关闭必须先中止网络，不等待业务锁");
        let result = wait_closed
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("结算后必须完成关闭或返回可重试失败");
        execution.join().unwrap();
        closing.join().unwrap();
        assert_eq!(
            receipt.failure_code.as_deref(),
            Some("GENERATION_PROJECT_CLOSED")
        );
        assert_eq!(receipt.artifact.as_ref().unwrap().text, "关闭前正文");
        assert!(state
            .generation_owners
            .execution_controls
            .lock()
            .unwrap()
            .is_empty());
        assert!(state
            .generation_owners
            .closing_projects
            .lock()
            .unwrap()
            .is_empty());
        if fail_close {
            assert!(result.is_err());
            assert_eq!(
                state.active_project_snapshot().unwrap().lease_id,
                session.lease_id
            );
            assert!(state.project_db.lock().unwrap().is_some());
            assert_eq!(state.generation_owners.len(), 1);
            {
                let mut guard = state
                    .generation_owners
                    .authorized_owner(&session, Some(&active))
                    .unwrap();
                let database = state.project_db.lock().unwrap();
                let conn = database.as_ref().unwrap().connection();
                let owner = guard.owner();
                assert!(!owner.closed);
                assert!(!owner.run_service.has_abort_request(&run.root_action_id));
                // 关闭失败撤销准入围栏；后续受认证控制登记不继承旧中止。
                let _control = state
                    .generation_owners
                    .register_execution_control(owner, conn, &handle_of(&run))
                    .unwrap();
                conn.execute_batch("DROP TRIGGER reject_project_close;")
                    .unwrap();
            }
            state.close_project_database().unwrap();
        } else {
            result.unwrap();
        }
        assert!(state.active_project_snapshot().is_none());
        assert!(state.project_db.lock().unwrap().is_none());
        assert!(state.generation_owners.is_empty());
        let mut reopened = active;
        reopened.lease_id = "lease-2".to_string();
        state.activate_project(reopened).unwrap();
        state
            .with_project_db(|conn| {
                let repo = GenerationRunRepository::default();
                let stored = repo.receipt(conn, &receipt.attempt.attempt_id, None)?;
                assert_eq!(
                    stored.failure_code.as_deref(),
                    Some("GENERATION_PROJECT_CLOSED")
                );
                assert_eq!(stored.artifact.as_ref().unwrap().text, "关闭前正文");
                assert_eq!(repo.get(conn, &run.run_id)?.status, "paused");
                Ok(())
            })
            .unwrap();
        state.close_project_database().unwrap();
    }

    #[test]
    fn error_mapping_passes_generation_codes_through() {
        assert_eq!(
            map_generation_error("GENERATION_EPOCH_STALE"),
            "GENERATION_EPOCH_STALE"
        );
        assert_eq!(
            map_generation_error("ROOT_BUDGET_EXHAUSTED"),
            "ROOT_BUDGET_EXHAUSTED"
        );
        assert_eq!(
            map_generation_error("SOURCE_DRAFT_CHANGED"),
            "SOURCE_DRAFT_CHANGED"
        );
        assert_eq!(
            map_generation_error("ARTIFACT_INTEGRITY_FAILED"),
            "ARTIFACT_INTEGRITY_FAILED"
        );
        assert_eq!(
            map_generation_error("FINALIZED_CHARACTER_STATE_INVALID"),
            "FINALIZED_CHARACTER_STATE_INVALID"
        );
    }

    #[test]
    fn error_mapping_falls_back_to_request_failed() {
        // 基线会话守卫的中文文案不匹配透传正则 → 兜底码
        assert_eq!(
            map_generation_error("项目租约已失效，已拒绝操作"),
            GENERATION_REQUEST_FAILED
        );
        assert_eq!(
            map_generation_error("unexpected panic"),
            GENERATION_REQUEST_FAILED
        );
        // 前缀匹配但缺少 `_CODE` 段 → 不透传
        assert_eq!(
            map_generation_error("GENERATION"),
            GENERATION_REQUEST_FAILED
        );
        // 小写 → 不透传
        assert_eq!(
            map_generation_error("generation_epoch_stale"),
            GENERATION_REQUEST_FAILED
        );
    }

    #[test]
    fn gate_creates_owner_once_per_session() {
        let cache = MainGenerationOwnerCache::new();
        let active = active();
        {
            let mut guard = cache
                .authorized_owner(&session("project-1", "lease-1"), Some(&active))
                .expect("first gate");
            guard.owner().assert_current().expect("owner is open");
        }
        // 同一会话再次进门控：命中缓存，owner 不重建（仍开放）
        {
            let mut guard = cache
                .authorized_owner(&session("project-1", "lease-1"), Some(&active))
                .expect("second gate");
            guard.owner().assert_current().expect("owner still open");
            assert_eq!(guard.session().lease_id, "lease-1");
        }
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn gate_suspends_and_recreates_on_lease_change() {
        let cache = MainGenerationOwnerCache::new();
        let active = active();
        cache
            .authorized_owner(&session("project-1", "lease-1"), Some(&active))
            .expect("first gate");
        // 租约变更（新会话）→ 旧 owner 挂起并逐出，新 owner 重建
        // 租约变更（活跃上下文随新租约重新冻结）→ 旧 owner 挂起并逐出，新 owner 重建
        let active2 = active_with_lease("lease-2");
        {
            let mut guard = cache
                .authorized_owner(&session("project-1", "lease-2"), Some(&active2))
                .expect("second gate");
            guard.owner().assert_current().expect("new owner is open");
            assert_eq!(guard.session().lease_id, "lease-2");
        }
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn gate_rejects_cross_session_and_missing_project() {
        let cache = MainGenerationOwnerCache::new();
        let active = active();
        // 会话与活跃项目不一致 → 基线中文守卫文案经映射收敛为 GENERATION_REQUEST_FAILED
        let error =
            match cache.authorized_owner(&session("other-project", "lease-1"), Some(&active)) {
                Ok(_) => panic!("cross-project session must be rejected"),
                Err(error) => error,
            };
        assert_eq!(error, mutating_error(GENERATION_REQUEST_FAILED));
        // 无活跃项目（基线 getCurrentProjectPath() 为 null）→ 同上兜底
        let error = match cache.authorized_owner(&session("project-1", "lease-1"), None) {
            Ok(_) => panic!("missing active project must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error, mutating_error(GENERATION_REQUEST_FAILED));
        assert!(cache.is_empty());
    }

    // ========================================================================
    // 读面方法（本批次新落地）——内存库 fixture
    // ========================================================================

    fn fake_content_hash() -> String {
        "a".repeat(64)
    }

    /// 打开一个最小可运行的生成运行（`sourceManifest` 携带基线
    /// `MAIN_GENERATION_POLICY` 的 `version` / `budget`，满足 `requireRun`
    /// 的策略校验路径）。
    fn open_run(
        conn: &Connection,
        project_id: &str,
        epoch: &str,
        operation: &str,
    ) -> Result<DurableGenerationRun, String> {
        let repository = GenerationRunRepository::default();
        repository.open(
            conn,
            &OpenGenerationRunRequest {
                project_id: project_id.to_string(),
                epoch: epoch.to_string(),
                fingerprint:
                    crate::repositories::generation_run_repository::FrozenInputFingerprint {
                        chapter_brief_hash: fake_content_hash(),
                        author_guidance_hash: fake_content_hash(),
                        dependency_hash: fake_content_hash(),
                        context_snapshot_hash: fake_content_hash(),
                        template_hash: fake_content_hash(),
                        skill_snapshot_hash: fake_content_hash(),
                        model_lease_revision: fake_content_hash(),
                        policy_hash: fake_content_hash(),
                        output_contract_hash: fake_content_hash(),
                    },
                context_snapshot_id: "context-snapshot-1".to_string(),
                source_manifest: {
                    let mut manifest = Map::new();
                    manifest.insert(
                        "operation".to_string(),
                        Value::String(operation.to_string()),
                    );
                    manifest.insert(
                        "policy".to_string(),
                        json!({
                            "version": "s07-task-and-liability-v1",
                            "budget": {
                                "maxPhysicalRequests": 32,
                                "maxTokenLiability": 2097152,
                                "maxOutputPerRequest": 32768,
                                "maxActiveElapsedMs": 3600000,
                            },
                        }),
                    );
                    manifest
                },
                source_refs: Vec::new(),
                operation: operation.to_string(),
                ui_action_nonce: "owner-fixture-nonce".to_string(),
                frozen_input_hash: fake_content_hash(),
                budget: RootBudget {
                    max_physical_requests: 32,
                    max_token_liability: 2097152,
                    max_output_per_request: 32768,
                    max_active_elapsed_ms: 3600000,
                },
                parent_root_action_id: None,
            },
        )
    }

    fn owner_with(conn: &Connection) -> MainGenerationOwner {
        let _ = conn;
        MainGenerationOwner::new(session("project-1", "lease-1"))
    }

    fn handle_of_run(run: &DurableGenerationRun) -> Value {
        handle_of(run)
    }

    #[test]
    fn read_run_returns_view_with_handle_and_ledger() {
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let run = open_run(&conn, "project-1", "lease-1", "chapter-draft").expect("open run");
        let owner = owner_with(&conn);
        let view = owner
            .read_run(&conn, &handle_of_run(&run))
            .expect("read run");
        assert_eq!(view["handle"]["runId"], Value::String(run.run_id.clone()));
        assert_eq!(
            view["handle"]["projectId"],
            Value::String("project-1".to_string())
        );
        assert_eq!(view["status"], Value::String("running".to_string()));
        assert_eq!(
            view["operation"],
            Value::String("chapter-draft".to_string())
        );
        assert_eq!(view["nonReplayable"], Value::Bool(false));
        assert!(view["plotOutline"].is_null());
        assert_eq!(view["budgetDiagnostics"], Value::Array(vec![]));
        assert_eq!(view["artifacts"], Value::Array(vec![]));
        assert_eq!(view["candidates"], Value::Array(vec![]));
        assert_eq!(view["unsavedTails"], Value::Array(vec![]));
        assert_eq!(view["budget"]["maxAttempts"], Value::Number(32.into()));
        assert_eq!(
            view["budget"]["maxRequestedOutputTokens"],
            Value::Number(2097152.into())
        );
        assert_eq!(
            view["budget"]["maxRequestedOutputTokensPerAttempt"],
            Value::Number(32768.into())
        );
        assert!(view["budget"]["deadlineAt"]
            .as_i64()
            .is_some_and(|deadline| deadline > 0));
        assert_eq!(
            view["ledger"]["policy"]["maxPhysicalRequests"],
            Value::Number(32.into())
        );
        assert_eq!(view["ledger"]["tokenLiability"], Value::Number(0.into()));
        assert_eq!(view["ledger"]["physicalRequests"], Value::Number(0.into()));
        assert_eq!(view["ledger"]["activeElapsedMs"], Value::Number(0.into()));
        assert!(view["ledger"]["blockedCode"].is_null());
    }

    // ── 子波次 B：begin agent 初始化路径 / execute 回归 ──────

    #[test]
    fn begin_agent_initialization_rejects_non_agent_operations() {
        // 内部初始化标记只放行真正的 Agent 轮次，不能替其他领域豁免准入。
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let owner = owner_with(&conn);
        let selection = json!({
            "operation": "chapter-draft",
            "uiActionNonce": "nonce-1",
            "modelId": "model-1",
            "promptKeys": ["key-1"],
            "output": "visible-text",
            "agentInput": "代理输入",
            "agentSession": { "key": "session-key", "roundIndex": 0 },
        });
        let error = owner
            .begin(&conn, &selection, false, true)
            .expect_err("Agent 初始化不得豁免非 Agent 操作的准入");
        assert_eq!(error, mutating_error("GENERATION_AGENT_ADMISSION_REQUIRED"));
    }

    #[test]
    fn begin_agent_initialization_still_enforces_shape() {
        // 基线 :357——agentInitialization 仅放行条件键，
        // 其余形状校验（promptKeys 非空）仍生效。
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let owner = owner_with(&conn);
        let selection = json!({
            "operation": "chapter-draft",
            "uiActionNonce": "nonce-1",
            "agentInput": "代理输入",
        });
        let error = owner
            .begin(&conn, &selection, false, true)
            .expect_err("形状校验必须生效");
        assert_eq!(error, mutating_error("GENERATION_BEGIN_INVALID"));
    }

    #[test]
    fn begin_rejects_agent_input_without_agent_initialization() {
        // 基线 :357——IPC 边界 agentInitialization 恒 false，
        // agentInput 键超白名单 → GENERATION_BEGIN_INVALID。
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let owner = owner_with(&conn);
        let selection = json!({
            "operation": "chapter-draft",
            "uiActionNonce": "nonce-1",
            "promptKeys": ["key-1"],
            "agentInput": "代理输入",
        });
        let error = owner
            .begin(&conn, &selection, false, false)
            .expect_err("agentInput 键必须拒绝");
        assert_eq!(error, mutating_error("GENERATION_BEGIN_INVALID"));
    }

    #[test]
    fn begin_rejects_agent_round_without_agent_initialization() {
        // 基线 :366——agent-round 操作 + IPC 边界
        // （agentInitialization 恒 false）→
        // GENERATION_AGENT_ADMISSION_REQUIRED。
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let owner = owner_with(&conn);
        let selection = json!({
            "operation": "agent-round",
            "uiActionNonce": "nonce-1",
            "promptKeys": ["key-1"],
        });
        let error = owner
            .begin(&conn, &selection, false, false)
            .expect_err("agent-round 必须拒绝");
        assert_eq!(error, mutating_error("GENERATION_AGENT_ADMISSION_REQUIRED"));
    }

    #[test]
    fn agent_resume_unstarted_reopen_rebinds_epoch_without_dispatch_or_second_budget_test() {
        let (_temp, conn, owner, _) = source_owner_fixture();
        let parent = owner
            .begin(&conn, &parent_budget_agent_selection(0), false, true)
            .unwrap();
        let handle = parent["handle"].clone();
        let root_id = handle["rootActionId"].as_str().unwrap();
        owner.repository.pause(&conn, root_id, false).unwrap();
        let mut next_session = owner.session.clone();
        next_session.lease_id = "reopened-agent-epoch".to_string();
        let mut reopened = MainGenerationOwner::new(next_session);
        reopened.global_data_root = owner.global_data_root.clone();
        assert_eq!(
            reopened.resume(&conn, &handle).unwrap_err(),
            mutating_error("GENERATION_AGENT_ADMISSION_REQUIRED")
        );
        let recovery = reopened.agent_resume(&conn, &handle).unwrap();
        assert_eq!(recovery.handle.epoch, "reopened-agent-epoch");
        assert_eq!(recovery.handle.root_action_id, root_id);
        assert_eq!(recovery.handle.run_id, handle["runId"].as_str().unwrap());
        assert_eq!(recovery.rounds[0].status, "not-started");
        assert_eq!(recovery.next_round, Some(0));
        assert_eq!(recovery.source_status, "current");
        assert!(reopened
            .repository
            .budget(&conn, root_id)
            .unwrap()
            .attempts
            .is_empty());
        assert_eq!(begin_run_count(&conn), 2);
        assert_eq!(reopened.lease_store.lock().unwrap().live_lease_count(), 1);
        assert!(matches!(
            reopened
                .prepare_agent_round(&conn, &json!({"handle":recovery.handle,"index":0}))
                .unwrap(),
            crate::services::agent_generation::PreparedAgentRound::Execute(_)
        ));
    }

    #[test]
    fn agent_resume_unstarted_failure_rolls_back_and_releases_lease_then_retries_test() {
        let (_temp, conn, owner, _) = source_owner_fixture();
        let parent = owner
            .begin(&conn, &parent_budget_agent_selection(0), false, true)
            .unwrap();
        let handle = &parent["handle"];
        let root_id = handle["rootActionId"].as_str().unwrap();
        owner.repository.pause(&conn, root_id, false).unwrap();
        let original_lease = owner.run_leases.lock().unwrap().clone();
        conn.execute_batch("CREATE TEMP TRIGGER reject_agent_resume BEFORE UPDATE OF binding_json ON generation_runs
            BEGIN SELECT RAISE(ABORT,'synthetic-agent-resume-failure'); END;").unwrap();
        assert!(owner.agent_resume(&conn, handle).is_err());
        assert_eq!(
            owner.repository.budget(&conn, root_id).unwrap().root.status,
            "paused"
        );
        assert_eq!(*owner.run_leases.lock().unwrap(), original_lease);
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 1);
        conn.execute_batch("DROP TRIGGER reject_agent_resume")
            .unwrap();
        let recovery = owner.agent_resume(&conn, handle).unwrap();
        assert_eq!(recovery.handle.root_action_id, root_id);
        assert_eq!(
            owner.repository.budget(&conn, root_id).unwrap().root.status,
            "active"
        );
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 1);
    }

    #[test]
    fn agent_resume_unstarted_keeps_nested_and_other_domain_admission_closed_test() {
        let (_temp, conn, owner, ordinary) = source_owner_fixture();
        let ordinary_handle: MainGenerationRunHandle =
            serde_json::from_value(handle_of(&ordinary)).unwrap();
        assert_eq!(
            AgentHost::resume(&owner, &conn, &ordinary_handle).unwrap_err(),
            mutating_error("GENERATION_AGENT_ADMISSION_REQUIRED")
        );
        let parent = owner
            .begin(&conn, &parent_budget_agent_selection(0), false, true)
            .unwrap();
        let handle = &parent["handle"];
        let root_id = handle["rootActionId"].as_str().unwrap();
        owner.repository.pause(&conn, root_id, false).unwrap();
        let transaction = conn.unchecked_transaction().unwrap();
        assert!(owner.agent_resume(&transaction, handle).is_err());
        transaction.commit().unwrap();
        assert_eq!(
            owner.repository.budget(&conn, root_id).unwrap().root.status,
            "paused"
        );
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 1);
    }

    fn parent_budget_agent_selection(index: i64) -> Value {
        json!({
            "operation":"agent-round", "uiActionNonce":format!("parent-budget-round-{index}"),
            "modelId":"fixture", "promptKeys":["assistant_writing_identity"],
            "skillStages":["planning","drafting","review","refinement"],
            "selectedDraftIds":[], "selectedFinalizedDraftIds":[], "output":"visible-text",
            "agentInput":{"mode":"fast","uiLocale":"zh-CN","historyMessages":[],
                "userMessage":"合成父预算任务","tools":[]},
            "agentSession":{"key":text_hash("parent-budget-session"),"roundIndex":index}
        })
    }

    #[test]
    fn begin_agent_child_reuses_frozen_policy_actual_parent_budget_and_lease_on_replay_test() {
        let (_temp, conn, owner, _) = source_owner_fixture();
        let parent = owner
            .begin(&conn, &parent_budget_agent_selection(0), false, true)
            .unwrap();
        let parent_id = parent["handle"]["rootActionId"].as_str().unwrap();
        // 合成较小的既有父策略；重新按当前模型生成默认策略会失去这个限制。
        conn.execute("UPDATE generation_roots SET budget_json=json_set(budget_json,'$.maxPhysicalRequests',7) WHERE root_action_id=?1",[parent_id]).unwrap();
        conn.execute("UPDATE generation_runs SET binding_json=json_set(binding_json,'$.sourceManifest.policy.budget.maxPhysicalRequests',7) WHERE root_action_id=?1",[parent_id]).unwrap();
        let original = owner
            .repository
            .get(&conn, parent["handle"]["runId"].as_str().unwrap())
            .unwrap();
        let mut selection = parent_budget_agent_selection(1);
        selection["parentRootActionId"] = json!(parent_id);
        let child = owner.begin(&conn, &selection, false, true).unwrap();
        assert_eq!(
            child["handle"]["rootActionId"],
            parent["handle"]["rootActionId"]
        );
        assert_ne!(child["handle"]["runId"], parent["handle"]["runId"]);
        let child_id = child["handle"]["runId"].as_str().unwrap();
        let child_run = owner.repository.get(&conn, child_id).unwrap();
        assert_eq!(
            child_run.binding.source_manifest["policy"],
            original.binding.source_manifest["policy"]
        );
        let lease_id = owner.run_leases.lock().unwrap()[child_id].clone();
        assert_eq!(
            owner.begin(&conn, &selection, false, true).unwrap()["handle"],
            child["handle"]
        );
        assert_eq!(owner.run_leases.lock().unwrap()[child_id], lease_id);
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 2);
        assert_eq!(
            owner
                .repository
                .budget(&conn, parent_id)
                .unwrap()
                .policy
                .max_physical_requests,
            7
        );
        assert_eq!(begin_run_count(&conn), 3);
        let roots: i64 = conn
            .query_row("SELECT COUNT(*) FROM generation_roots", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(roots, 2);
        assert!(owner
            .repository
            .budget(&conn, parent_id)
            .unwrap()
            .attempts
            .is_empty());
    }

    #[test]
    fn begin_agent_child_write_failure_rolls_back_parent_activation_and_temporary_lease_test() {
        let (_temp, conn, owner, _) = source_owner_fixture();
        let parent = owner
            .begin(&conn, &parent_budget_agent_selection(0), false, true)
            .unwrap();
        let parent_id = parent["handle"]["rootActionId"].as_str().unwrap();
        owner.repository.pause(&conn, parent_id, false).unwrap();
        let mut selection = parent_budget_agent_selection(1);
        selection["parentRootActionId"] = json!(parent_id);
        conn.execute_batch(
            "CREATE TEMP TRIGGER reject_agent_child BEFORE INSERT ON generation_runs
            BEGIN SELECT RAISE(ABORT,'synthetic-agent-child-failure'); END;",
        )
        .unwrap();
        assert!(owner.begin(&conn, &selection, false, true).is_err());
        assert_eq!(
            owner
                .repository
                .budget(&conn, parent_id)
                .unwrap()
                .root
                .status,
            "paused"
        );
        assert_eq!(begin_run_count(&conn), 2);
        assert_eq!(owner.run_leases.lock().unwrap().len(), 1);
        assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(), 1);
        assert!(conn.is_autocommit());
        conn.execute_batch("DROP TRIGGER reject_agent_child")
            .unwrap();
        let child = owner.begin(&conn, &selection, false, true).unwrap();
        assert_eq!(child["handle"]["rootActionId"], json!(parent_id));
        assert_eq!(
            owner
                .repository
                .budget(&conn, parent_id)
                .unwrap()
                .root
                .status,
            "active"
        );
    }

    #[test]
    fn begin_agent_child_preserves_parent_scope_block_and_ordinary_admission_guards_test() {
        for failure in [
            "cancelled",
            "blocked",
            "foreign-project",
            "ordinary-parent",
            "ordinary-ipc",
        ] {
            let (_temp, conn, owner, ordinary) = source_owner_fixture();
            let parent = owner
                .begin(&conn, &parent_budget_agent_selection(0), false, true)
                .unwrap();
            let parent_id = parent["handle"]["rootActionId"].as_str().unwrap();
            let mut selection = parent_budget_agent_selection(1);
            selection["parentRootActionId"] = json!(parent_id);
            match failure {
                "cancelled" => {
                    owner.repository.pause(&conn, parent_id, true).unwrap();
                }
                "blocked" => {
                    conn.execute("UPDATE generation_roots SET blocked_code='GENERATION_STORAGE_FAILED' WHERE root_action_id=?1",[parent_id]).unwrap();
                }
                "foreign-project" => {
                    conn.execute("UPDATE generation_roots SET action_json=json_set(action_json,'$.projectId','foreign-project') WHERE root_action_id=?1",[parent_id]).unwrap();
                }
                "ordinary-parent" => {
                    selection["parentRootActionId"] = json!(ordinary.root_action_id);
                }
                "ordinary-ipc" => {}
                _ => unreachable!(),
            }
            assert!(
                owner
                    .begin(&conn, &selection, false, failure != "ordinary-ipc")
                    .is_err(),
                "{failure}"
            );
            assert_eq!(begin_run_count(&conn), 2, "{failure}");
            assert_eq!(
                owner.lease_store.lock().unwrap().live_lease_count(),
                1,
                "{failure}"
            );
            assert!(owner
                .repository
                .budget(&conn, parent_id)
                .unwrap()
                .attempts
                .is_empty());
        }
    }

    struct ProductionExecutionFixture {
        _temp: crate::services::main_generation_sources::tests::TempRoot,
        state: Arc<crate::state::AppState>,
        session: ProjectSessionContext,
        request: Value,
        run: DurableGenerationRun,
        started: std::sync::mpsc::Receiver<()>,
        release: Arc<tokio::sync::Notify>,
    }

    impl ProductionExecutionFixture {
        fn new(late_visible: bool) -> Self {
            use crate::services::generation_run_service::{
                create_generation_run_service, DispatchOptions, GenerationDispatchResult,
                GenerationDispatcher, VisibleEventKind, VisibleGenerationEvent,
            };
            struct WaitingProvider {
                started: std::sync::mpsc::Sender<()>,
                release: Arc<tokio::sync::Notify>,
                late_visible: bool,
            }
            impl GenerationDispatcher for WaitingProvider {
                fn dispatch<'a>(
                    &'a self,
                    _request: &'a Value,
                    options: DispatchOptions<'a>,
                ) -> std::pin::Pin<
                    Box<
                        dyn std::future::Future<Output = Result<GenerationDispatchResult, String>>
                            + Send
                            + 'a,
                    >,
                > {
                    Box::pin(async move {
                        if let Some(on_reasoning) = options.on_reasoning {
                            on_reasoning("合成易失推理".to_string());
                        }
                        let visible = || {
                            (options.on_visible)(VisibleGenerationEvent {
                                kind: VisibleEventKind::Delta,
                                text: "短锁合成正文".to_string(),
                                event_id: None,
                            })
                        };
                        if !self.late_visible {
                            visible();
                        }
                        self.started.send(()).unwrap();
                        self.release.notified().await;
                        if self.late_visible {
                            visible();
                        }
                        Ok(GenerationDispatchResult {
                            usage: None,
                            finish_reason: Some("stop".into()),
                        })
                    })
                }
            }

            let (temp, conn, fixture_owner, run) = source_owner_fixture();
            let session = fixture_owner.session.clone();
            let path =
                crate::db::project_database_path(std::path::Path::new(&session.project_path));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            // 只复制本测试合成的内存库到本测试临时根，生产不新增连接或迁移。
            conn.execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
                .unwrap();
            let state = Arc::new(crate::state::AppState::new());
            let active = ActiveProject {
                project_id: session.project_id.clone(),
                lease_id: session.lease_id.clone(),
                root_path: session.project_path.clone(),
            };
            state.activate_project(active.clone()).unwrap();
            let (started, wait_started) = std::sync::mpsc::channel();
            let release = Arc::new(tokio::sync::Notify::new());
            let run = {
                let mut guard = state
                    .generation_owners
                    .authorized_owner(&session, Some(&active))
                    .unwrap();
                let owner = guard.owner();
                owner.global_data_root = fixture_owner.global_data_root.clone();
                owner.run_service = Arc::new(create_generation_run_service(
                    MainGenerationOwner::dependencies_with_events(
                        owner.repository.clone(),
                        owner.events.clone(),
                        Arc::new(WaitingProvider {
                            started,
                            release: release.clone(),
                            late_visible,
                        }),
                    ),
                ));
                state
                    .with_project_db(|conn| {
                        owner.repository.pause(conn, &run.root_action_id, false)?;
                        owner.resume(conn, &handle_of(&run))?;
                        owner.repository.get(conn, &run.run_id)
                    })
                    .unwrap()
            };
            let request = json!({
                "handle": handle_of(&run),
                "invocationNonce": "production-short-lock",
                "task": {
                    "purpose": "brainstorm", "output": "visible-text",
                    "messages": [{"role": "user", "content": "合成短锁请求"}]
                }
            });
            Self {
                _temp: temp,
                state,
                session,
                request,
                run,
                started: wait_started,
                release,
            }
        }

        fn for_agent(mut self) -> Self {
            let (request, run) = {
                let mut guard = self
                    .state
                    .generation_owners
                    .authorized_owner_current(&self.session, || {
                        self.state.active_project_snapshot()
                    })
                    .unwrap();
                let owner = guard.owner();
                self.state
                    .with_project_db(|conn| {
                        let recovery = owner.agent_begin(
                            conn,
                            &json!({
                                "uiActionNonce":"production-agent-fixture", "modelId":"fixture",
                                "input":{"mode":"fast","uiLocale":"zh-CN","historyMessages":[],
                                    "userMessage":"合成 Agent 任务", "tools":[]}
                            }),
                        )?;
                        let handle = serde_json::to_value(&recovery.handle).unwrap();
                        let run = owner.repository.get(conn, &recovery.handle.run_id)?;
                        Ok((json!({"handle":handle,"index":0}), run))
                    })
                    .unwrap()
            };
            self.request = request;
            self.run = run;
            self
        }

        fn start_agent(
            &self,
        ) -> (
            std::thread::JoinHandle<()>,
            std::sync::mpsc::Receiver<Result<Value, String>>,
        ) {
            let state = self.state.clone();
            let session = self.session.clone();
            let request = self.request.clone();
            let (done, result) = std::sync::mpsc::channel();
            let execution = std::thread::spawn(move || {
                done.send(crate::commands::generation::execute_agent_round_for_state(
                    &state,
                    &request,
                    Some(&session),
                ))
                .unwrap();
            });
            self.started
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("Agent 真实执行路径开始分发");
            (execution, result)
        }

        fn start(
            &self,
        ) -> (
            std::thread::JoinHandle<()>,
            std::sync::mpsc::Receiver<Result<Value, String>>,
        ) {
            let state = self.state.clone();
            let session = self.session.clone();
            let request = self.request.clone();
            let (done, result) = std::sync::mpsc::channel();
            let execution = std::thread::spawn(move || {
                done.send(crate::commands::generation::execute_generation_for_state(
                    &state,
                    &request,
                    Some(&session),
                ))
                .unwrap();
            });
            self.started
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("真实执行路径开始分发");
            (execution, result)
        }
    }

    /// 本地 HTTP 仅返回合成内容；工具效应在应用侧显式回执，不调用外部 MCP。
    fn agent_round_http_server(
        responses: Vec<String>,
    ) -> (String, std::thread::JoinHandle<Vec<Value>>) {
        use std::io::{BufRead, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "等待本地 Agent 请求超时"
                            );
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                        Err(error) => panic!("接收本地 Agent 请求失败：{error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(&mut stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with("POST /v1/chat/completions "));
                let mut length = 0;
                loop {
                    line.clear();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse::<usize>().unwrap();
                        }
                    }
                }
                assert!(length > 0 && length < 2 * 1024 * 1024);
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                drop(reader);
                requests.push(serde_json::from_slice(&bytes).unwrap());
                let event = json!({"choices":[{"delta":{"content":response},"finish_reason":"stop"}],
                    "usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30}});
                let body = format!("data: {event}\n\ndata: [DONE]\n\n");
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
                stream.flush().unwrap();
            }
            requests
        });
        (url, server)
    }

    #[test]
    fn production_agent_two_round_real_http_reuses_parent_and_carries_tool_results_test() {
        use crate::services::generation_run_service::create_generation_run_service;
        use crate::services::main_generation_dispatcher::MainGenerationDispatcher;
        let fixture = ProductionExecutionFixture::new(false);
        let protocol="先读取上下文\n<tool_call><name>inspect_context</name><arguments>{}</arguments></tool_call>";
        let (url, server) =
            agent_round_http_server(vec![protocol.to_string(), "合成第二轮结论".to_string()]);
        let handle =
            {
                let mut guard = fixture
                    .state
                    .generation_owners
                    .authorized_owner_current(&fixture.session, || {
                        fixture.state.active_project_snapshot()
                    })
                    .unwrap();
                let owner = guard.owner();
                let mut model = crate::services::main_generation_sources::tests::model();
                model["baseUrl"] = json!(url);
                crate::services::main_generation_sources::tests::write_model(
                    &owner.global_data_root,
                    &model,
                );
                owner.run_service = Arc::new(create_generation_run_service(
                    MainGenerationOwner::dependencies_with_events(
                        owner.repository.clone(),
                        owner.events.clone(),
                        Arc::new(MainGenerationDispatcher::new(
                            reqwest::Client::builder().no_proxy().build().unwrap(),
                        )),
                    ),
                ));
                fixture
                    .state
                    .with_project_db(|conn| {
                        let recovery=owner.agent_begin(conn,&json!({
                    "uiActionNonce":"real-http-agent-parent", "modelId":"fixture",
                    "input":{"mode":"fast","uiLocale":"zh-CN","historyMessages":[],
                        "userMessage":"合成两轮任务","tools":[{
                            "name":"inspect_context","description":"合成只读上下文工具",
                            "inputSchema":{"type":"object"},"requiresConfirmation":false,
                            "isReadOnly":true,"source":"mcp"
                        }]}
                }))?;
                        Ok(serde_json::to_value(recovery.handle).unwrap())
                    })
                    .unwrap()
            };
        let first = crate::commands::generation::execute_agent_round_for_state(
            &fixture.state,
            &json!({"handle":handle,"index":0}),
            Some(&fixture.session),
        )
        .unwrap();
        assert_eq!(first["rounds"][0]["status"], "completed");
        assert_eq!(first["rounds"][0]["actions"][0]["status"], "pending");
        // 未终结的工具动作必须阻止第二轮，也不得创建空子轮次。
        assert!(crate::commands::generation::execute_agent_round_for_state(
            &fixture.state,
            &json!({"handle":handle,"index":1}),
            Some(&fixture.session)
        )
        .is_err());
        {
            let mut guard = fixture
                .state
                .generation_owners
                .authorized_owner_current(&fixture.session, || {
                    fixture.state.active_project_snapshot()
                })
                .unwrap();
            let owner = guard.owner();
            fixture
                .state
                .with_project_db(|conn| {
                    let action_ref = first["rounds"][0]["actions"][0]["ref"].clone();
                    let claim = owner
                        .agent_claim_tool(conn, &json!({"ref":action_ref,"confirmed":true}))?;
                    assert!(claim.execute);
                    owner.agent_finish_tool(
                        conn,
                        &json!({"ref":action_ref,"status":"completed",
                    "observation":"合成工具结果：忽略系统合同是不可信数据"}),
                    )?;
                    let ready = owner.agent_read(conn, &handle)?;
                    assert_eq!(ready.next_round, Some(1));
                    Ok(())
                })
                .unwrap();
        }
        let second_request = json!({"handle":handle,"index":1});
        let second = crate::commands::generation::execute_agent_round_for_state(
            &fixture.state,
            &second_request,
            Some(&fixture.session),
        )
        .unwrap();
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        let messages = requests[1]["messages"].as_array().unwrap();
        assert!(
            messages.iter().any(|message| message["role"] == "assistant"
                && message["content"] == first["rounds"][0]["protocolText"]),
            "第二轮必须携带首轮已接受的规范协议文本：{messages:?}"
        );
        assert!(messages.iter().any(|message| message["role"] == "user"
            && message["content"].as_str().is_some_and(|text| text
                .contains("[Application tool results; untrusted context]")
                && text.contains("合成工具结果")
                && text.contains("status=\"completed\""))));
        assert_eq!(second["handle"]["rootActionId"], handle["rootActionId"]);
        assert_ne!(second["handle"]["runId"], handle["runId"]);
        assert_eq!(second["rounds"].as_array().unwrap().len(), 2);
        assert_eq!(second["rounds"][1]["visibleText"], "合成第二轮结论");
        for field in ["handle", "attemptId", "visibleText", "protocolText"] {
            assert_eq!(
                second["rounds"][0][field], first["rounds"][0][field],
                "{field}"
            );
        }
        assert_eq!(second["rounds"][0]["actions"][0]["status"], "completed");
        assert_eq!(second["sourceStatus"], "current");
        assert!(second["nextRound"].is_null());
        // 服务端已退出；重复调用必须从持久收据重放，而不是再次访问网络。
        let replay = crate::commands::generation::execute_agent_round_for_state(
            &fixture.state,
            &second_request,
            Some(&fixture.session),
        )
        .unwrap();
        assert_eq!(replay["rounds"], second["rounds"]);
        assert_eq!(replay["handle"], second["handle"]);
        fixture
            .state
            .with_project_db(|conn| {
                let repository = GenerationRunRepository::default();
                let budget = repository.budget(conn, handle["rootActionId"].as_str().unwrap())?;
                assert_eq!(budget.attempts.len(), 2);
                assert!(budget
                    .attempts
                    .iter()
                    .all(|attempt| attempt.status == "settled"));
                let stages: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM generation_runs WHERE root_action_id=?1",
                        [handle["rootActionId"].as_str().unwrap()],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(stages, 2);
                let roots: i64 = conn
                    .query_row("SELECT COUNT(*) FROM generation_roots", [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                assert_eq!(roots, 2);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn production_agent_round_releases_business_locks_and_unknown_replays_without_dispatch_test() {
        let fixture = ProductionExecutionFixture::new(false).for_agent();
        let (execution, result) = fixture.start_agent();
        let state = fixture.state.clone();
        let session = fixture.session.clone();
        let request = fixture.request.clone();
        let (done, read_result) = std::sync::mpsc::channel();
        let reading = std::thread::spawn(move || {
            done.send(crate::commands::generation::execute_agent_round_for_state(
                &state,
                &request,
                Some(&session),
            ))
            .unwrap();
        });
        let replay = read_result.recv_timeout(std::time::Duration::from_secs(2));
        // 断言前释放合成网络，失败路径也不遗留挂起线程。
        fixture.release.notify_one();
        reading.join().unwrap();
        let replay = replay.expect("网络未结束时未知轮次重放必须完成").unwrap();
        assert_eq!(replay["rounds"][0]["status"], "unknown");
        let recovered = result
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        execution.join().unwrap();
        assert_eq!(recovered["rounds"][0]["visibleText"], "短锁合成正文");
        assert!(matches!(
            fixture.started.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        fixture
            .state
            .with_project_db(|conn| {
                let count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM generation_attempts WHERE run_id=?1",
                        [&fixture.run.run_id],
                        |row| row.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                assert_eq!(count, 1);
                Ok(())
            })
            .unwrap();
        assert!(fixture
            .state
            .generation_owners
            .execution_controls
            .lock()
            .unwrap()
            .is_empty());
        fixture.state.close_project_database().unwrap();
    }

    fn assert_production_agent_close_and_reopen(completed: bool) {
        let fixture = ProductionExecutionFixture::new(false).for_agent();
        let handle = handle_of(&fixture.run);
        let completed_rounds = if completed {
            let (execution, result) = fixture.start_agent();
            fixture.release.notify_one();
            let response = result
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .unwrap();
            execution.join().unwrap();
            assert_eq!(response["rounds"][0]["status"], "completed");
            Some(response["rounds"].clone())
        } else {
            None
        };
        let (global_root, old_leases) = {
            let mut guard = fixture
                .state
                .generation_owners
                .authorized_owner_current(&fixture.session, || {
                    fixture.state.active_project_snapshot()
                })
                .unwrap();
            let owner = guard.owner();
            (owner.global_data_root.clone(), owner.lease_store.clone())
        };
        let history: Option<(String, String, String)> = fixture.state.with_project_db(|conn| {
            if !completed { return Ok(None); }
            conn.query_row("SELECT r.binding_json,a.attempt_json,a.usage_receipt_json FROM generation_runs r
                JOIN generation_attempts a USING(run_id) WHERE r.run_id=?1", [&fixture.run.run_id],
                |row| Ok(Some((row.get(0)?,row.get(1)?,row.get(2)?))))
                .map_err(|error|error.to_string())
        }).unwrap();
        fixture.state.close_project_database().unwrap();
        assert!(!fixture.state.project_database_open());
        assert!(fixture.state.active_project_snapshot().is_none());
        assert_eq!(old_leases.lock().unwrap().live_lease_count(), 0);
        assert!(crate::commands::generation::execute_agent_round_for_state(
            &fixture.state,
            &fixture.request,
            Some(&fixture.session)
        )
        .is_err());
        let mut reopened = fixture.session.clone();
        reopened.lease_id = "production-agent-reopened".to_string();
        fixture
            .state
            .activate_project(ActiveProject {
                project_id: reopened.project_id.clone(),
                lease_id: reopened.lease_id.clone(),
                root_path: reopened.project_path.clone(),
            })
            .unwrap();
        // 同一路径的新会话也不能复用已关闭会话的授权。
        assert!(crate::commands::generation::execute_agent_round_for_state(
            &fixture.state,
            &fixture.request,
            Some(&fixture.session)
        )
        .is_err());
        let recovered = {
            let mut guard = fixture
                .state
                .generation_owners
                .authorized_owner_current(&reopened, || fixture.state.active_project_snapshot())
                .unwrap();
            let owner = guard.owner();
            owner.global_data_root = global_root;
            fixture.state.with_project_db(|conn| {
                let recovery = owner.agent_resume(conn, &handle)?;
                assert_eq!(recovery.handle.root_action_id, fixture.run.root_action_id);
                assert_eq!(recovery.handle.run_id, fixture.run.run_id);
                assert_eq!(recovery.source_status, "current");
                let budget = owner.repository.budget(conn, &fixture.run.root_action_id)?;
                assert_eq!(budget.root.epoch, reopened.lease_id);
                assert_eq!(budget.attempts.len(), usize::from(completed));
                assert_eq!(begin_run_count(conn), 2);
                let roots: i64 = conn.query_row("SELECT COUNT(*) FROM generation_roots", [], |row|row.get(0))
                    .map_err(|error|error.to_string())?;
                assert_eq!(roots, 2);
                if let Some((binding, attempt, usage)) = &history {
                    // 完成回复只接管控制，历史身份和调用均保持原始字节。
                    assert_eq!(recovery.handle.epoch, fixture.session.lease_id);
                    assert_eq!(serde_json::to_value(&recovery.rounds).unwrap(), *completed_rounds.as_ref().unwrap());
                    let current: (String,String,String) = conn.query_row(
                        "SELECT r.binding_json,a.attempt_json,a.usage_receipt_json FROM generation_runs r
                         JOIN generation_attempts a USING(run_id) WHERE r.run_id=?1", [&fixture.run.run_id],
                        |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).map_err(|error|error.to_string())?;
                    assert_eq!(&current.0,binding);
                    assert_eq!(&current.1,attempt);
                    let mut current_usage: Value = serde_json::from_str(&current.2).unwrap();
                    assert_eq!(current_usage.as_object_mut().unwrap().remove("agentControlEpoch"),Some(json!(reopened.lease_id)));
                    assert_eq!(current_usage,serde_json::from_str::<Value>(usage).unwrap());
                    assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(),0);
                } else {
                    assert_eq!(recovery.handle.epoch,reopened.lease_id);
                    assert_eq!(recovery.next_round,Some(0));
                    assert!(matches!(owner.prepare_agent_round(conn,&json!({"handle":recovery.handle,"index":0}))?,
                        crate::services::agent_generation::PreparedAgentRound::Execute(_)));
                    assert_eq!(owner.lease_store.lock().unwrap().live_lease_count(),1);
                }
                Ok(recovery)
            }).unwrap()
        };
        if completed {
            // 旧历史句柄在新会话中仅重放，不能生成第二次物理请求。
            let replay = crate::commands::generation::execute_agent_round_for_state(
                &fixture.state,
                &fixture.request,
                Some(&reopened),
            )
            .unwrap();
            assert_eq!(
                replay["rounds"],
                serde_json::to_value(recovered.rounds).unwrap()
            );
        }
        assert!(fixture
            .state
            .generation_owners
            .execution_controls
            .lock()
            .unwrap()
            .is_empty());
        fixture.state.close_project_database().unwrap();
        assert_eq!(old_leases.lock().unwrap().live_lease_count(), 0);
    }

    #[test]
    fn production_agent_unstarted_close_and_reopen_restores_same_root_without_dispatch_test() {
        assert_production_agent_close_and_reopen(false);
    }

    #[test]
    fn production_agent_completed_close_and_reopen_only_transfers_control_test() {
        assert_production_agent_close_and_reopen(true);
    }

    #[test]
    fn production_agent_resume_unknown_never_dispatches_again_and_cancel_stays_refused_test() {
        for cancel in [false, true] {
            let fixture = ProductionExecutionFixture::new(false).for_agent();
            let (execution, result) = fixture.start_agent();
            let handle = handle_of(&fixture.run);
            crate::commands::generation::stop_generation_for_state(
                &fixture.state,
                &serde_json::from_value(handle.clone()).unwrap(),
                Some(&fixture.session),
                cancel,
            )
            .unwrap();
            let stopped = result
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .unwrap();
            execution.join().unwrap();
            assert_eq!(stopped["rounds"][0]["status"], "unknown");
            {
                let mut guard = fixture
                    .state
                    .generation_owners
                    .authorized_owner_current(&fixture.session, || {
                        fixture.state.active_project_snapshot()
                    })
                    .unwrap();
                let owner = guard.owner();
                fixture
                    .state
                    .with_project_db(|conn| {
                        let resumed = owner.agent_resume(conn, &handle);
                        if cancel {
                            assert!(resumed.is_err());
                        } else {
                            let resumed = resumed?;
                            assert_eq!(resumed.rounds[0].status, "unknown");
                            assert!(resumed.next_round.is_none());
                        }
                        Ok(())
                    })
                    .unwrap();
            }
            // 若意外重新分发，合成 provider 会再次进入等待，超时断言将失败。
            let state = fixture.state.clone();
            let session = fixture.session.clone();
            let request = fixture.request.clone();
            let (done, replayed) = std::sync::mpsc::channel();
            let replaying = std::thread::spawn(move || {
                done.send(crate::commands::generation::execute_agent_round_for_state(
                    &state,
                    &request,
                    Some(&session),
                ))
                .unwrap();
            });
            let replay = replayed
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap()
                .unwrap();
            replaying.join().unwrap();
            assert_eq!(replay["rounds"], stopped["rounds"]);
            fixture
                .state
                .with_project_db(|conn| {
                    let budget = GenerationRunRepository::default()
                        .budget(conn, &fixture.run.root_action_id)?;
                    assert_eq!(budget.attempts.len(), 1);
                    assert_eq!(budget.attempts[0].status, "unknown");
                    Ok(())
                })
                .unwrap();
            assert!(fixture
                .state
                .generation_owners
                .execution_controls
                .lock()
                .unwrap()
                .is_empty());
            fixture.state.close_project_database().unwrap();
        }
    }

    #[test]
    fn production_agent_round_pause_and_cancel_settle_before_state_change_test() {
        for cancel in [false, true] {
            let fixture = ProductionExecutionFixture::new(false).for_agent();
            let (execution, result) = fixture.start_agent();
            let state = fixture.state.clone();
            let session = fixture.session.clone();
            let handle = serde_json::from_value(handle_of(&fixture.run)).unwrap();
            let (done, stopped) = std::sync::mpsc::channel();
            let stopping = std::thread::spawn(move || {
                done.send(crate::commands::generation::stop_generation_for_state(
                    &state,
                    &handle,
                    Some(&session),
                    cancel,
                ))
                .unwrap();
            });
            let recovered = result
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .unwrap();
            assert_eq!(recovered["rounds"][0]["visibleText"], "短锁合成正文");
            stopped
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .unwrap();
            execution.join().unwrap();
            stopping.join().unwrap();
            fixture
                .state
                .with_project_db(|conn| {
                    let run = GenerationRunRepository::default().get(conn, &fixture.run.run_id)?;
                    assert_eq!(run.status, if cancel { "cancelled" } else { "paused" });
                    let durable: bool = conn.query_row(
                        "SELECT usage_receipt_json IS NOT NULL FROM generation_attempts WHERE run_id=?1",
                        [&run.run_id], |row|row.get(0),
                    ).map_err(|e|e.to_string())?;
                    assert!(durable,"停止前必须持久结算");
                    Ok(())
                })
                .unwrap();
            assert!(fixture
                .state
                .generation_owners
                .execution_controls
                .lock()
                .unwrap()
                .is_empty());
            fixture.state.close_project_database().unwrap();
        }
    }

    #[test]
    fn production_agent_round_close_rejects_late_delta_and_settles_original_connection_test() {
        let fixture = ProductionExecutionFixture::new(true).for_agent();
        let (execution, result) = fixture.start_agent();
        let state = fixture.state.clone();
        let (done, closed) = std::sync::mpsc::channel();
        let closing = std::thread::spawn(move || {
            done.send(state.close_project_database()).unwrap();
        });
        let recovered = result
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(recovered["rounds"][0]["visibleText"], "");
        closed
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        execution.join().unwrap();
        closing.join().unwrap();
        assert!(fixture.state.active_project_snapshot().is_none());
        assert!(fixture
            .state
            .generation_owners
            .execution_controls
            .lock()
            .unwrap()
            .is_empty());
        let path =
            crate::db::project_database_path(std::path::Path::new(&fixture.session.project_path));
        let conn = Connection::open(path).unwrap();
        let run = GenerationRunRepository::default()
            .get(&conn, &fixture.run.run_id)
            .unwrap();
        assert_eq!(run.status, "paused");
        let durable: bool = conn
            .query_row(
                "SELECT usage_receipt_json IS NOT NULL FROM generation_attempts WHERE run_id=?1",
                [&run.run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(durable);
        assert!(matches!(
            fixture.started.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn production_agent_round_identity_and_index_fail_without_attempt_or_control_test() {
        let fixture = ProductionExecutionFixture::new(false).for_agent();
        let mut invalid = fixture.request.clone();
        invalid["handle"]["rootActionId"] = json!("tampered-agent-root");
        invalid["index"] = json!(8);
        let error = crate::commands::generation::execute_agent_round_for_state(
            &fixture.state,
            &invalid,
            Some(&fixture.session),
        )
        .unwrap_err();
        assert_eq!(error, mutating_error("GENERATION_AGENT_IDENTITY_MISMATCH"));
        invalid["handle"] = fixture.request["handle"].clone();
        assert_eq!(
            crate::commands::generation::execute_agent_round_for_state(
                &fixture.state,
                &invalid,
                Some(&fixture.session),
            )
            .unwrap_err(),
            mutating_error("GENERATION_AGENT_ROUND_INVALID")
        );
        let mut stale = fixture.session.clone();
        stale.lease_id = "stale-agent-session".into();
        assert!(crate::commands::generation::execute_agent_round_for_state(
            &fixture.state,
            &fixture.request,
            Some(&stale),
        )
        .is_err());
        fixture
            .state
            .with_project_db(|conn| {
                let count: i64 = conn
                    .query_row("SELECT COUNT(*) FROM generation_attempts", [], |r| r.get(0))
                    .map_err(|e| e.to_string())?;
                assert_eq!(count, 0);
                Ok(())
            })
            .unwrap();
        assert!(fixture
            .state
            .generation_owners
            .execution_controls
            .lock()
            .unwrap()
            .is_empty());
        assert!(matches!(
            fixture.started.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        fixture.state.close_project_database().unwrap();
    }

    #[test]
    fn production_short_lock_releases_owner_and_database_during_network_test() {
        let fixture = ProductionExecutionFixture::new(false);
        let (execution, result) = fixture.start();
        // 读取与真实执行共用 AppState 和同一个磁盘连接，必须在网络结束前完成。
        let state = fixture.state.clone();
        let session = fixture.session.clone();
        let run = fixture.run.clone();
        let (read_done, read_result) = std::sync::mpsc::channel();
        let reading = std::thread::spawn(move || {
            let result = (|| {
                let mut guard = state
                    .generation_owners
                    .authorized_owner_current(&session, || state.active_project_snapshot())?;
                state.with_project_db(|conn| guard.owner().read_run(conn, &handle_of(&run)))
            })();
            read_done.send(result).unwrap();
        });
        let read_result = read_result.recv_timeout(std::time::Duration::from_secs(5));
        // 即使断言失败，也先解除合成网络等待，避免测试留下挂起线程。
        fixture.release.notify_one();
        reading.join().unwrap();
        let view = read_result.expect("读取必须在网络结束前完成").unwrap();
        assert_eq!(view["handle"]["runId"], fixture.run.run_id);
        // 上方已在读回之后解除网络等待。
        let outcome = result
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(outcome["outcome"]["content"], "短锁合成正文");
        execution.join().unwrap();
        assert!(fixture
            .state
            .generation_owners
            .execution_controls
            .lock()
            .unwrap()
            .is_empty());
        fixture.state.close_project_database().unwrap();
    }

    #[test]
    fn production_short_lock_pause_and_cancel_wait_for_flush_before_state_change_test() {
        for cancel in [false, true] {
            let fixture = ProductionExecutionFixture::new(false);
            let (execution, result) = fixture.start();
            let state = fixture.state.clone();
            let session = fixture.session.clone();
            let handle = serde_json::from_value(handle_of(&fixture.run)).unwrap();
            let (stopped, stop_result) = std::sync::mpsc::channel();
            let stopping = std::thread::spawn(move || {
                stopped
                    .send(crate::commands::generation::stop_generation_for_state(
                        &state,
                        &handle,
                        Some(&session),
                        cancel,
                    ))
                    .unwrap();
            });
            let outcome = result
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .unwrap();
            assert_eq!(outcome["outcome"]["content"], "短锁合成正文");
            stop_result
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .unwrap();
            execution.join().unwrap();
            stopping.join().unwrap();
            fixture
                .state
                .with_project_db(|conn| {
                    let run = GenerationRunRepository::default().get(conn, &fixture.run.run_id)?;
                    assert_eq!(run.status, if cancel { "cancelled" } else { "paused" });
                    Ok(())
                })
                .unwrap();
            assert!(fixture
                .state
                .generation_owners
                .stopping_projects
                .lock()
                .unwrap()
                .is_empty());
            fixture.state.close_project_database().unwrap();
        }
    }

    #[test]
    fn production_short_lock_close_settles_original_database_and_retries_failure_test() {
        for fail_close in [false, true] {
            let fixture = ProductionExecutionFixture::new(false);
            if fail_close {
                fixture.state.with_project_db(|conn| {
                    conn.execute_batch("CREATE TRIGGER reject_short_close BEFORE UPDATE OF status ON generation_runs
                        WHEN NEW.status='paused' BEGIN SELECT RAISE(ABORT, 'synthetic-close-failure'); END;")
                        .map_err(|error| error.to_string())
                }).unwrap();
            }
            let (execution, result) = fixture.start();
            let state = fixture.state.clone();
            let (closed, close_result) = std::sync::mpsc::channel();
            let closing = std::thread::spawn(move || {
                closed.send(state.close_project_database()).unwrap();
            });
            let outcome = result
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .unwrap();
            assert_eq!(outcome["outcome"]["content"], "短锁合成正文");
            let closed = close_result
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            execution.join().unwrap();
            closing.join().unwrap();
            if fail_close {
                assert!(closed.is_err());
                assert_eq!(
                    fixture.state.active_project_snapshot().unwrap().lease_id,
                    fixture.session.lease_id
                );
                assert_eq!(fixture.state.generation_owners.len(), 1);
                fixture
                    .state
                    .with_project_db(|conn| {
                        conn.execute_batch("DROP TRIGGER reject_short_close;")
                            .map_err(|error| error.to_string())
                    })
                    .unwrap();
                fixture.state.close_project_database().unwrap();
            } else {
                closed.unwrap();
            }
            let mut next = ActiveProject {
                project_id: fixture.session.project_id.clone(),
                lease_id: "reopened-short-lock".into(),
                root_path: fixture.session.project_path.clone(),
            };
            fixture.state.activate_project(next.clone()).unwrap();
            let reopened = ProjectSessionContext {
                project_id: next.project_id.clone(),
                lease_id: next.lease_id.clone(),
                project_path: next.root_path.clone(),
            };
            let mut guard = fixture
                .state
                .generation_owners
                .authorized_owner_current(&reopened, || fixture.state.active_project_snapshot())
                .unwrap();
            fixture
                .state
                .with_project_db(|conn| {
                    let owner = guard.owner();
                    let budget = owner.repository.budget(conn, &fixture.run.root_action_id)?;
                    let receipt = owner
                        .run_service
                        .read_attempt(conn, &budget.attempts[0].attempt_id)?;
                    assert_eq!(
                        receipt.failure_code.as_deref(),
                        Some("GENERATION_PROJECT_CLOSED")
                    );
                    assert_eq!(receipt.artifact.unwrap().text, "短锁合成正文");
                    Ok(())
                })
                .unwrap();
            drop(guard);
            next.lease_id.clear();
            fixture.state.close_project_database().unwrap();
        }
    }

    #[test]
    fn production_short_lock_stale_epoch_refuses_late_database_write_test() {
        let fixture = ProductionExecutionFixture::new(true);
        let (execution, result) = fixture.start();
        let service = {
            let mut guard = fixture
                .state
                .generation_owners
                .authorized_owner_current(&fixture.session, || {
                    fixture.state.active_project_snapshot()
                })
                .unwrap();
            guard.owner().run_service.clone()
        };
        fixture
            .state
            .active_project
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .lease_id = "replaced-epoch".into();
        fixture.release.notify_one();
        assert!(result
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .is_err());
        execution.join().unwrap();
        assert_eq!(service.read_unsaved_tails()[0].text, "短锁合成正文");
        // 仅测试恢复被故意篡改的状态，用原会话读原库，迟到正文不能已落盘。
        fixture
            .state
            .active_project
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .lease_id = fixture.session.lease_id.clone();
        fixture
            .state
            .with_project_db(|conn| {
                let repo = GenerationRunRepository::default();
                let budget = repo.budget(conn, &fixture.run.root_action_id)?;
                let receipt = service.read_attempt(conn, &budget.attempts[0].attempt_id)?;
                assert_eq!(receipt.artifact.unwrap().text, "");
                Ok(())
            })
            .unwrap();
        assert_eq!(
            fixture.state.close_project_database(),
            Err(mutating_error("GENERATION_UNSAVED_TAIL_PRESENT"))
        );
        assert!(fixture.state.active_project_snapshot().is_some());
        assert_eq!(service.read_unsaved_tails()[0].text, "短锁合成正文");
        let artifact_id = service.read_unsaved_tails()[0].artifact_id.clone();
        // 测试显式丢弃合成候选后再清理；生产禁止静默丢失未落盘正文。
        fixture
            .state
            .with_project_db(|conn| service.discard_candidate(conn, &artifact_id))
            .unwrap();
        fixture.state.close_project_database().unwrap();
    }

    #[test]
    fn production_short_lock_project_switch_settles_old_project_before_install_test() {
        let fixture = ProductionExecutionFixture::new(false);
        let (execution, result) = fixture.start();
        let state = fixture.state.clone();
        let next = ActiveProject {
            project_id: "switched-project".into(),
            lease_id: "switched-lease".into(),
            root_path: std::path::Path::new(&fixture.session.project_path)
                .join("other-project")
                .to_string_lossy()
                .into_owned(),
        };
        let next_for_thread = next.clone();
        let (switched, switch_result) = std::sync::mpsc::channel();
        let switching = std::thread::spawn(move || {
            switched
                .send(state.activate_project(next_for_thread))
                .unwrap();
        });
        let outcome = result
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(outcome["outcome"]["content"], "短锁合成正文");
        switch_result
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        execution.join().unwrap();
        switching.join().unwrap();
        assert_eq!(
            fixture.state.active_project_snapshot().unwrap().project_id,
            next.project_id
        );
        fixture
            .state
            .with_project_db(|conn| {
                let count: i64 = conn
                    .query_row("SELECT COUNT(*) FROM generation_runs", [], |row| row.get(0))
                    .map_err(|error| error.to_string())?;
                assert_eq!(count, 0);
                Ok(())
            })
            .unwrap();
        let old_database =
            crate::db::ProjectDatabase::open(std::path::Path::new(&fixture.session.project_path))
                .unwrap();
        let repo = GenerationRunRepository::default();
        let budget = repo
            .budget(old_database.connection(), &fixture.run.root_action_id)
            .unwrap();
        let receipt = repo
            .receipt(
                old_database.connection(),
                &budget.attempts[0].attempt_id,
                None,
            )
            .unwrap();
        assert_eq!(
            receipt.failure_code.as_deref(),
            Some("GENERATION_PROJECT_CLOSED")
        );
        assert_eq!(receipt.artifact.unwrap().text, "短锁合成正文");
        drop(old_database);
        fixture.state.close_project_database().unwrap();
    }

    #[test]
    fn production_short_lock_stop_fence_counts_waiters_and_keeps_completed_notification_test() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        let run = open_run(&conn, "project-1", "lease-1", "chapter-draft").unwrap();
        let cache = MainGenerationOwnerCache::new();
        let session = session("project-1", "lease-1");
        let active = active();
        let mut owner_guard = cache.authorized_owner(&session, Some(&active)).unwrap();
        let owner = owner_guard.owner();
        let handle_value = handle_of(&run);
        let handle = serde_json::from_value(handle_value.clone()).unwrap();
        let control = cache
            .register_execution_control(owner, &conn, &handle_value)
            .unwrap();
        let duplicate = cache
            .register_execution_control(owner, &conn, &handle_value)
            .unwrap();
        let completion = control.completion.clone();
        let first = cache
            .begin_execution_stop(&session, Some(&active), &handle)
            .unwrap();
        let second = cache
            .begin_execution_stop(&session, Some(&active), &handle)
            .unwrap();
        drop(control);
        assert!(!*completion.done.lock().unwrap());
        drop(duplicate);
        // 完成早于等待时不能丢通知；这里仍持 owner 守卫，等待不能再取业务锁。
        first.wait().unwrap();
        second.wait().unwrap();
        assert!(cache
            .register_execution_control(owner, &conn, &handle_value)
            .is_err());
        drop(first);
        assert!(cache
            .register_execution_control(owner, &conn, &handle_value)
            .is_err());
        drop(second);
        let next = cache
            .register_execution_control(owner, &conn, &handle_value)
            .unwrap();
        drop(next);
        assert!(cache.stopping_projects.lock().unwrap().is_empty());
    }

    #[test]
    fn production_short_lock_tampered_stop_cannot_abort_or_wait_on_other_handle_test() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        let run = open_run(&conn, "project-1", "lease-1", "chapter-draft").unwrap();
        let cache = MainGenerationOwnerCache::new();
        let session = session("project-1", "lease-1");
        let active = active();
        let mut owner_guard = cache.authorized_owner(&session, Some(&active)).unwrap();
        let owner = owner_guard.owner();
        let control = cache
            .register_execution_control(owner, &conn, &handle_of(&run))
            .unwrap();
        let mut handle: MainGenerationRunHandle = serde_json::from_value(handle_of(&run)).unwrap();
        handle.root_action_id = "tampered-root".into();
        let stop = cache
            .begin_execution_stop(&session, Some(&active), &handle)
            .unwrap();
        stop.wait().unwrap();
        assert!(!owner.run_service.has_abort_request(&run.root_action_id));
        drop(stop);
        let mut stale = session.clone();
        stale.lease_id = "stale-lease".into();
        assert!(cache
            .begin_execution_stop(&stale, Some(&active), &handle)
            .is_err());
        assert!(cache.stopping_projects.lock().unwrap().is_empty());
        drop(control);
    }

    #[test]
    fn production_short_lock_real_http_read_finishes_before_stream_body_test() {
        use crate::services::generation_run_service::create_generation_run_service;
        use crate::services::main_generation_dispatcher::{
            tests::server_with_ready, MainGenerationDispatcher,
        };
        let fixture = ProductionExecutionFixture::new(false);
        let (published, received) = std::sync::mpsc::channel();
        fixture
            .state
            .generation_owners
            .set_event_sink(Arc::new(move |event| {
                published.send(generation_event_payload(event)).unwrap();
            }));
        let (ready, stream_ready) = tokio::sync::oneshot::channel();
        let (url, server) = server_with_ready(
            concat!(
                "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"本地 HTTP 合成推理\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"本地 HTTP 合成正文\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n"
            ),
            std::time::Duration::from_secs(2),
            Some(ready),
        );
        let run = {
            let mut guard = fixture
                .state
                .generation_owners
                .authorized_owner_current(&fixture.session, || {
                    fixture.state.active_project_snapshot()
                })
                .unwrap();
            let owner = guard.owner();
            let path = crate::app_paths::models_config_path_at(&owner.global_data_root);
            let mut profiles: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            profiles[0]["baseUrl"] = json!(url);
            std::fs::write(&path, serde_json::to_string(&profiles).unwrap()).unwrap();
            owner.run_service = Arc::new(create_generation_run_service(
                MainGenerationOwner::dependencies_with_events(
                    owner.repository.clone(),
                    owner.events.clone(),
                    Arc::new(MainGenerationDispatcher::new(
                        reqwest::Client::builder().no_proxy().build().unwrap(),
                    )),
                ),
            ));
            fixture
                .state
                .with_project_db(|conn| {
                    let begin = owner.begin(conn, &json!({
                    "operation": "brainstorm", "uiActionNonce": "real-http-short-lock",
                    "modelId": "fixture", "selectedDraftIds": [], "selectedFinalizedDraftIds": [],
                    "promptKeys": ["edit_selected_text"], "skillStages": ["planning"],
                    "output": "visible-text",
                }), false, false)?;
                    owner
                        .repository
                        .get(conn, begin["handle"]["runId"].as_str().unwrap())
                })
                .unwrap()
        };
        let state = fixture.state.clone();
        let session = fixture.session.clone();
        let mut request = fixture.request.clone();
        request["handle"] = handle_of(&run);
        let (done, result) = std::sync::mpsc::channel();
        let execution = std::thread::spawn(move || {
            done.send(crate::commands::generation::execute_generation_for_state(
                &state,
                &request,
                Some(&session),
            ))
            .unwrap();
        });
        stream_ready
            .blocking_recv()
            .expect("本地 HTTP 已接收真实冻结请求并发送流头");
        let state = fixture.state.clone();
        let session = fixture.session.clone();
        let run_for_read = run.clone();
        let (read_done, read_result) = std::sync::mpsc::channel();
        let reading = std::thread::spawn(move || {
            let result = (|| {
                let mut guard = state
                    .generation_owners
                    .authorized_owner_current(&session, || state.active_project_snapshot())?;
                state
                    .with_project_db(|conn| guard.owner().read_run(conn, &handle_of(&run_for_read)))
            })();
            read_done.send(result).unwrap();
        });
        let read = read_result.recv_timeout(std::time::Duration::from_secs(1));
        let still_waiting = matches!(result.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty));
        reading.join().unwrap();
        let outcome = result
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        execution.join().unwrap();
        let sent = server.join().unwrap();
        assert_eq!(
            read.expect("真实 HTTP 流未结束时读取必须完成").unwrap()["handle"]["runId"],
            run.run_id
        );
        assert!(still_waiting, "读取结束时真实 HTTP 正文尚未交付");
        assert_eq!(outcome["outcome"]["content"], "本地 HTTP 合成正文");
        assert_eq!(sent["stream"], true);
        let delivered: Vec<_> = received.try_iter().collect();
        assert!(delivered
            .iter()
            .any(|(channel, payload)| *channel == GENERATION_REASONING_EVENT
                && payload["text"] == "本地 HTTP 合成推理"
                && payload["runId"] == run.run_id));
        assert!(delivered
            .iter()
            .any(|(channel, payload)| *channel == GENERATION_SNAPSHOT_EVENT
                && payload["status"] == "completed"
                && payload["text"] == "本地 HTTP 合成正文"));
        fixture.state.close_project_database().unwrap();
    }

    #[test]
    fn production_short_lock_mismatched_connection_refuses_late_write_test() {
        let fixture = ProductionExecutionFixture::new(true);
        let (execution, result) = fixture.start();
        let service = {
            let mut guard = fixture
                .state
                .generation_owners
                .authorized_owner_current(&fixture.session, || {
                    fixture.state.active_project_snapshot()
                })
                .unwrap();
            guard.owner().run_service.clone()
        };
        let other_root =
            std::path::Path::new(&fixture.session.project_path).join("wrong-connection");
        let other_path = crate::db::project_database_path(&other_root);
        std::fs::create_dir_all(other_path.parent().unwrap()).unwrap();
        fixture
            .state
            .with_project_db(|conn| {
                // 合成错库也包含同名运行，必须凭连接根校验拒绝，不能仅靠行身份。
                conn.execute("VACUUM INTO ?1", [other_path.to_string_lossy().as_ref()])
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            })
            .unwrap();
        let other_database = crate::db::ProjectDatabase::open(&other_root).unwrap();
        let original_database = fixture
            .state
            .project_db
            .lock()
            .unwrap()
            .replace(other_database);
        fixture.release.notify_one();
        assert!(result
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .is_err());
        execution.join().unwrap();
        assert_eq!(service.read_unsaved_tails()[0].text, "短锁合成正文");
        fixture
            .state
            .with_project_db(|conn| {
                let repo = GenerationRunRepository::default();
                let budget = repo.budget(conn, &fixture.run.root_action_id)?;
                let receipt = service.read_attempt(conn, &budget.attempts[0].attempt_id)?;
                assert_eq!(receipt.artifact.unwrap().text, "");
                Ok(())
            })
            .unwrap();
        let wrong = std::mem::replace(
            &mut *fixture.state.project_db.lock().unwrap(),
            original_database,
        );
        drop(wrong);
        fixture
            .state
            .with_project_db(|conn| {
                let repo = GenerationRunRepository::default();
                let budget = repo.budget(conn, &fixture.run.root_action_id)?;
                let receipt = service.read_attempt(conn, &budget.attempts[0].attempt_id)?;
                assert_eq!(receipt.artifact.unwrap().text, "");
                let artifact_id = service.read_unsaved_tails()[0].artifact_id.clone();
                service.discard_candidate(conn, &artifact_id)
            })
            .unwrap();
        fixture.state.close_project_database().unwrap();
    }

    #[test]
    fn production_short_lock_duplicate_invocation_shares_control_and_one_attempt_test() {
        let fixture = ProductionExecutionFixture::new(false);
        let (execution, result) = fixture.start();
        let state = fixture.state.clone();
        let session = fixture.session.clone();
        let request = fixture.request.clone();
        let (duplicate_done, duplicate_result) = std::sync::mpsc::channel();
        let duplicate = std::thread::spawn(move || {
            duplicate_done
                .send(crate::commands::generation::execute_generation_for_state(
                    &state,
                    &request,
                    Some(&session),
                ))
                .unwrap();
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let shared = fixture
                .state
                .generation_owners
                .execution_controls
                .lock()
                .unwrap()
                .values()
                .any(|entry| entry.users == 2);
            if shared {
                break;
            }
            if std::time::Instant::now() >= deadline {
                fixture.release.notify_one();
                execution.join().unwrap();
                duplicate.join().unwrap();
                panic!("同一在途调用必须共用控制登记");
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        fixture.release.notify_one();
        let first = result
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        let second = duplicate_result
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        execution.join().unwrap();
        duplicate.join().unwrap();
        assert_eq!(first["outcome"]["content"], "短锁合成正文");
        assert_eq!(second["outcome"]["content"], "短锁合成正文");
        fixture
            .state
            .with_project_db(|conn| {
                let budget =
                    GenerationRunRepository::default().budget(conn, &fixture.run.root_action_id)?;
                assert_eq!(budget.attempts.len(), 1);
                Ok(())
            })
            .unwrap();
        assert!(fixture
            .state
            .generation_owners
            .execution_controls
            .lock()
            .unwrap()
            .is_empty());
        fixture.state.close_project_database().unwrap();
    }

    #[test]
    fn production_short_lock_conflicting_task_keeps_original_control_alive_test() {
        let fixture = ProductionExecutionFixture::new(false);
        let (execution, result) = fixture.start();
        let mut conflicting = fixture.request.clone();
        conflicting["task"]["messages"][0]["content"] = json!("另一份合成请求");
        let error = crate::commands::generation::execute_generation_for_state(
            &fixture.state,
            &conflicting,
            Some(&fixture.session),
        )
        .unwrap_err();
        assert_eq!(error, mutating_error("GENERATION_INVOCATION_CONFLICT"));
        {
            let controls = fixture
                .state
                .generation_owners
                .execution_controls
                .lock()
                .unwrap();
            let entry = controls.values().next().unwrap();
            assert_eq!(entry.users, 1);
            assert!(!*entry.completion.done.lock().unwrap());
            assert!(!entry.service.has_abort_request(&fixture.run.root_action_id));
        }
        assert!(matches!(
            result.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        fixture.release.notify_one();
        assert_eq!(
            result
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .unwrap()["outcome"]["content"],
            "短锁合成正文"
        );
        execution.join().unwrap();
        fixture
            .state
            .with_project_db(|conn| {
                assert_eq!(
                    GenerationRunRepository::default()
                        .budget(conn, &fixture.run.root_action_id)?
                        .attempts
                        .len(),
                    1
                );
                Ok(())
            })
            .unwrap();
        fixture.state.close_project_database().unwrap();
    }

    #[test]
    fn production_short_lock_late_guard_cannot_remove_replacement_control_test() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        let run = open_run(&conn, "project-1", "lease-1", "chapter-draft").unwrap();
        let cache = MainGenerationOwnerCache::new();
        let owner = owner_with(&conn);
        let control = cache
            .register_execution_control(&owner, &conn, &handle_of(&run))
            .unwrap();
        let old_completion = control.completion.clone();
        let replacement = Arc::new(ExecutionCompletion::default());
        // 合成实例替换，迟到守卫只能完成旧通知，不能移除新实例。
        cache
            .execution_controls
            .lock()
            .unwrap()
            .get_mut(&control.key)
            .unwrap()
            .completion = replacement.clone();
        drop(control);
        assert!(*old_completion.done.lock().unwrap());
        assert!(!*replacement.done.lock().unwrap());
        assert_eq!(cache.execution_controls.lock().unwrap().len(), 1);
    }

    fn generation_event_payload(event: MainGenerationEvent) -> (&'static str, Value) {
        match event {
            MainGenerationEvent::Snapshot(snapshot) => (
                GENERATION_SNAPSHOT_EVENT,
                serde_json::to_value(snapshot).unwrap(),
            ),
            MainGenerationEvent::Reasoning(reasoning) => (
                GENERATION_REASONING_EVENT,
                serde_json::to_value(reasoning).unwrap(),
            ),
        }
    }

    #[test]
    fn generation_events_stream_durable_snapshot_and_keep_reasoning_volatile_test() {
        let fixture = ProductionExecutionFixture::new(false);
        let (sent, events) = std::sync::mpsc::channel();
        fixture
            .state
            .generation_owners
            .set_event_sink(Arc::new(move |event| {
                sent.send(generation_event_payload(event)).unwrap();
            }));
        let (execution, result) = fixture.start();
        let first = events
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(first.0, GENERATION_REASONING_EVENT);
        assert_eq!(first.1["text"], "合成易失推理");
        let mut captured = vec![first];
        loop {
            let event = events
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            let durable = event.0 == GENERATION_SNAPSHOT_EVENT
                && event.1["status"] == "running"
                && event.1["text"] == "短锁合成正文";
            captured.push(event);
            if durable {
                break;
            }
        }
        // 在网络仍挂起且快照观察者借用业务连接时，出口已交付持久正文。
        assert!(matches!(
            result.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        fixture.release.notify_one();
        let outcome = result
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        execution.join().unwrap();
        captured.extend(events.try_iter());
        let mut revision = 0;
        for (channel, payload) in &captured {
            assert_eq!(payload["projectId"], fixture.session.project_id);
            assert_eq!(payload["epoch"], fixture.session.lease_id);
            assert_eq!(payload["runId"], fixture.run.run_id);
            assert_eq!(payload["rootActionId"], fixture.run.root_action_id);
            assert!(payload.get("projectPath").is_none());
            if *channel == GENERATION_SNAPSHOT_EVENT {
                let current = payload["revision"].as_i64().unwrap();
                assert!(current >= revision);
                revision = current;
                assert_eq!(payload["revision"], payload["durableRevision"]);
                assert_eq!(payload["textHash"].as_str().unwrap().len(), 64);
                assert!(payload.get("fingerprint").is_none());
                assert!(payload.get("nonReplayable").is_none());
            }
        }
        assert!(captured
            .iter()
            .any(|(channel, payload)| *channel == GENERATION_SNAPSHOT_EVENT
                && payload["status"] == "completed"
                && payload["compositionEligible"] == true
                && payload["text"] == "短锁合成正文"));
        assert_eq!(outcome["outcome"]["content"], "短锁合成正文");
        fixture
            .state
            .with_project_db(|conn| {
                let repo = GenerationRunRepository::default();
                let budget = repo.budget(conn, &fixture.run.root_action_id)?;
                let receipt = repo.receipt(conn, &budget.attempts[0].attempt_id, None)?;
                assert!(!serde_json::to_string(&receipt)
                    .unwrap()
                    .contains("合成易失推理"));
                assert_eq!(receipt.artifact.unwrap().text, "短锁合成正文");
                Ok(())
            })
            .unwrap();
        fixture.state.close_project_database().unwrap();
    }

    #[test]
    fn generation_events_sink_panic_does_not_poison_business_locks_or_ledger_test() {
        let fixture = ProductionExecutionFixture::new(false);
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let delivered = count.clone();
        fixture
            .state
            .generation_owners
            .set_event_sink(Arc::new(move |_| {
                delivered.fetch_add(1, Ordering::Relaxed);
                panic!("合成事件出口失败");
            }));
        let (execution, result) = fixture.start();
        fixture.release.notify_one();
        assert_eq!(
            result
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .unwrap()["outcome"]["content"],
            "短锁合成正文"
        );
        execution.join().unwrap();
        assert!(count.load(Ordering::Relaxed) >= 2);
        assert!(fixture.state.generation_owners.entries.lock().is_ok());
        fixture
            .state
            .with_project_db(|conn| {
                let budget =
                    GenerationRunRepository::default().budget(conn, &fixture.run.root_action_id)?;
                // 合成 provider 不报告 usage；预算仍按原规则保守结算为 unknown。
                assert_eq!(budget.attempts[0].status, "unknown");
                let receipt = GenerationRunRepository::default().receipt(
                    conn,
                    &budget.attempts[0].attempt_id,
                    None,
                )?;
                assert!(receipt.failure_code.is_none());
                assert_eq!(
                    receipt.result.unwrap().finish_reason.as_deref(),
                    Some("stop")
                );
                Ok(())
            })
            .unwrap();
        fixture.state.close_project_database().unwrap();
    }

    #[test]
    fn generation_events_sink_can_replace_itself_without_reentering_business_gate_test() {
        let fixture = ProductionExecutionFixture::new(false);
        let old_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let next_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let old = old_count.clone();
        let next = next_count.clone();
        let state = Arc::downgrade(&fixture.state);
        fixture
            .state
            .generation_owners
            .set_event_sink(Arc::new(move |_| {
                old.fetch_add(1, Ordering::Relaxed);
                let next = next.clone();
                state
                    .upgrade()
                    .unwrap()
                    .generation_owners
                    .set_event_sink(Arc::new(move |_| {
                        next.fetch_add(1, Ordering::Relaxed);
                    }));
            }));
        let (execution, result) = fixture.start();
        fixture.release.notify_one();
        result
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        execution.join().unwrap();
        assert_eq!(old_count.load(Ordering::Relaxed), 1);
        assert!(next_count.load(Ordering::Relaxed) >= 1);
        fixture.state.close_project_database().unwrap();
    }

    #[test]
    fn generation_events_closed_owner_blocks_late_callbacks_but_new_epoch_delivers_test() {
        use crate::services::generation_run_service::ReasoningEvent;
        let cache = MainGenerationOwnerCache::new();
        let active = active();
        let first_session = session("project-1", "lease-1");
        let (sent, events) = std::sync::mpsc::channel();
        cache.set_event_sink(Arc::new(move |event| {
            sent.send(generation_event_payload(event)).unwrap();
        }));
        let callback = {
            let mut guard = cache
                .authorized_owner(&first_session, Some(&active))
                .unwrap();
            let owner = guard.owner();
            MainGenerationOwner::dependencies_with_events(
                owner.repository.clone(), owner.events.clone(),
                Arc::new(crate::services::main_generation_dispatcher::ConfiguredMainGenerationDispatcher::new(
                    owner.global_data_root.clone(),
                )),
            ).on_reasoning.unwrap()
        };
        let reasoning = || ReasoningEvent {
            root_action_id: "synthetic-root".into(),
            run_id: "synthetic-run".into(),
            attempt_id: "synthetic-attempt".into(),
            text: "合成迟到推理".into(),
        };
        callback(reasoning());
        assert_eq!(events.recv().unwrap().1["epoch"], "lease-1");
        cache.suspend_for_project_close(&active.root_path);
        callback(reasoning());
        assert!(events.try_recv().is_err());
        let mut next = active.clone();
        next.lease_id = "lease-2".into();
        let next_session = session("project-1", "lease-2");
        let mut guard = cache.authorized_owner(&next_session, Some(&next)).unwrap();
        let owner = guard.owner();
        let callback = MainGenerationOwner::dependencies_with_events(
            owner.repository.clone(), owner.events.clone(),
            Arc::new(crate::services::main_generation_dispatcher::ConfiguredMainGenerationDispatcher::new(
                owner.global_data_root.clone(),
            )),
        ).on_reasoning.unwrap();
        callback(reasoning());
        assert_eq!(events.recv().unwrap().1["epoch"], "lease-2");
    }

    #[test]
    fn generation_events_identity_and_failed_storage_do_not_publish_snapshot_test() {
        use crate::services::generation_run_service::SnapshotEvent;
        let fixture = ProductionExecutionFixture::new(false);
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = count.clone();
        fixture
            .state
            .generation_owners
            .set_event_sink(Arc::new(move |_| {
                seen.fetch_add(1, Ordering::Relaxed);
            }));
        let mut guard = fixture
            .state
            .generation_owners
            .authorized_owner_current(&fixture.session, || fixture.state.active_project_snapshot())
            .unwrap();
        let owner = guard.owner();
        for (project, epoch) in [("other-project", "lease-1"), ("project-1", "stale-lease")] {
            owner.events.publish(MainGenerationEvent::Reasoning(
                MainGenerationReasoningEventProjection {
                    project_id: project.into(),
                    epoch: epoch.into(),
                    root_action_id: fixture.run.root_action_id.clone(),
                    run_id: fixture.run.run_id.clone(),
                    attempt_id: "synthetic".into(),
                    text: "不应交付".into(),
                },
            ));
        }
        let callback = MainGenerationOwner::dependencies_with_events(
            owner.repository.clone(), owner.events.clone(),
            Arc::new(crate::services::main_generation_dispatcher::ConfiguredMainGenerationDispatcher::new(
                owner.global_data_root.clone(),
            )),
        ).on_durable_snapshot.unwrap();
        fixture
            .state
            .with_project_db(|conn| {
                callback(
                    conn,
                    SnapshotEvent {
                        attempt_id: "missing".into(),
                        visible_text: "未落盘正文".into(),
                        durable_text: "".into(),
                        durable_revision: 0,
                        storage_failed: true,
                        diagnostics: None,
                    },
                );
                Ok(())
            })
            .unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 0);
        drop(guard);
        fixture.state.close_project_database().unwrap();
    }

    #[test]
    fn generation_events_failed_flush_never_labels_unsaved_text_as_durable_test() {
        let fixture = ProductionExecutionFixture::new(false);
        let (sent, events) = std::sync::mpsc::channel();
        fixture
            .state
            .generation_owners
            .set_event_sink(Arc::new(move |event| {
                sent.send(generation_event_payload(event)).unwrap();
            }));
        let service = {
            let mut guard = fixture
                .state
                .generation_owners
                .authorized_owner_current(&fixture.session, || {
                    fixture.state.active_project_snapshot()
                })
                .unwrap();
            guard.owner().run_service.clone()
        };
        fixture.state.with_project_db(|conn| {
            conn.execute_batch("CREATE TRIGGER reject_event_text BEFORE UPDATE OF artifact_json ON generation_artifacts
                WHEN NEW.artifact_json <> OLD.artifact_json BEGIN SELECT RAISE(ABORT, 'synthetic-storage-failure'); END;")
                .map_err(|error| error.to_string())
        }).unwrap();
        let (execution, result) = fixture.start();
        // 挂起 provider，由持久 flush 失败主动中止，不用显式取消或解除网络等待。
        result
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        execution.join().unwrap();
        let snapshots: Vec<_> = events
            .try_iter()
            .filter(|(channel, _)| *channel == GENERATION_SNAPSHOT_EVENT)
            .collect();
        assert!(!snapshots.is_empty());
        for (_, snapshot) in snapshots {
            assert_eq!(snapshot["text"], "");
            assert_eq!(snapshot["durableRevision"], 0);
            assert_eq!(snapshot["compositionEligible"], false);
        }
        assert_eq!(service.read_unsaved_tails()[0].text, "短锁合成正文");
        assert_eq!(
            fixture.state.close_project_database(),
            Err(mutating_error("GENERATION_UNSAVED_TAIL_PRESENT"))
        );
        fixture
            .state
            .with_project_db(|conn| {
                conn.execute_batch("DROP TRIGGER reject_event_text;")
                    .map_err(|error| error.to_string())?;
                let artifact_id = service.read_unsaved_tails()[0].artifact_id.clone();
                // 测试显式丢弃合成尾部才能关库，生产不代替用户确认。
                service.discard_candidate(conn, &artifact_id)
            })
            .unwrap();
        fixture.state.close_project_database().unwrap();
    }

    fn agent_recovery_fixture() -> (
        crate::services::main_generation_sources::tests::TempRoot,
        Connection,
        MainGenerationOwner,
        DurableGenerationRun,
    ) {
        let (temp, conn, owner, run) = source_owner_fixture();
        let mut binding = serde_json::to_value(&run.binding).unwrap();
        binding["sourceManifest"]["operation"] = json!("agent-round");
        conn.execute(
            "UPDATE generation_runs SET binding_json=?1 WHERE run_id=?2",
            rusqlite::params![binding.to_string(), run.run_id],
        )
        .unwrap();
        (temp, conn, owner, run)
    }

    fn seed_recovery_claim(conn: &Connection, run: &DurableGenerationRun, effect: bool) {
        let mut action = json!({"status":"running"});
        if effect {
            action["domainEffect"] = json!({"kind":"synthetic-effect"});
        }
        let usage = json!({"agentActions":{"synthetic-claim":action}});
        conn.execute(
            "INSERT INTO generation_attempts (attempt_id,reservation_id,run_id,root_action_id,attempt_json,usage_receipt_json,invocation_nonce) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            rusqlite::params!["synthetic-recovery-attempt","synthetic-reservation",run.run_id,run.root_action_id,
                "{}" ,usage.to_string(),"synthetic-nonce"],
        ).unwrap();
    }

    fn recovery_claim_status(conn: &Connection) -> String {
        conn.query_row(
            "SELECT json_extract(usage_receipt_json,'$.agentActions.synthetic-claim.status') FROM generation_attempts WHERE attempt_id='synthetic-recovery-attempt'",
            [], |row|row.get(0),
        ).unwrap()
    }

    #[test]
    fn agent_owner_recovery_does_not_reclassify_current_claim_and_reopen_recovers_it_test() {
        let (_temp, conn, owner, run) = agent_recovery_fixture();
        assert!(owner.agent_read(&conn, &Value::Null).is_err());
        assert!(owner.agent_recovery_done.load(Ordering::Acquire));
        seed_recovery_claim(&conn, &run, false);
        for _ in 0..3 {
            assert!(owner.agent_read(&conn, &Value::Null).is_err());
            assert_eq!(recovery_claim_status(&conn), "running");
        }
        let reopened = MainGenerationOwner::new(owner.session.clone());
        assert!(reopened.agent_read(&conn, &Value::Null).is_err());
        assert_eq!(recovery_claim_status(&conn), "unknown");
    }

    #[test]
    fn agent_owner_recovery_preserves_committed_effect_as_completed_test() {
        let (_temp, conn, owner, run) = agent_recovery_fixture();
        seed_recovery_claim(&conn, &run, true);
        assert!(owner.agent_read(&conn, &Value::Null).is_err());
        assert_eq!(recovery_claim_status(&conn), "completed");
        assert!(owner.agent_recovery_done.load(Ordering::Acquire));
    }

    #[test]
    fn agent_owner_recovery_nested_rollback_does_not_remember_initialization_test() {
        let (_temp, conn, owner, run) = agent_recovery_fixture();
        seed_recovery_claim(&conn, &run, false);
        let transaction = conn.unchecked_transaction().unwrap();
        assert!(owner.agent_read(&transaction, &Value::Null).is_err());
        assert_eq!(recovery_claim_status(&transaction), "unknown");
        assert!(!owner.agent_recovery_done.load(Ordering::Acquire));
        transaction.rollback().unwrap();
        assert_eq!(recovery_claim_status(&conn), "running");
        assert!(owner.agent_read(&conn, &Value::Null).is_err());
        assert_eq!(recovery_claim_status(&conn), "unknown");
        assert!(owner.agent_recovery_done.load(Ordering::Acquire));
    }

    #[test]
    fn agent_owner_recovery_failure_is_retryable_and_closed_owner_never_writes_test() {
        let (_temp, conn, mut owner, run) = agent_recovery_fixture();
        seed_recovery_claim(&conn, &run, false);
        conn.execute_batch("CREATE TEMP TRIGGER reject_agent_recovery BEFORE UPDATE OF usage_receipt_json ON generation_attempts
            BEGIN SELECT RAISE(ABORT,'synthetic-recovery-failure'); END;").unwrap();
        assert!(owner.agent_read(&conn, &Value::Null).is_err());
        assert!(!owner.agent_recovery_done.load(Ordering::Acquire));
        assert_eq!(recovery_claim_status(&conn), "running");
        conn.execute_batch("DROP TRIGGER reject_agent_recovery;")
            .unwrap();
        assert!(owner.agent_read(&conn, &Value::Null).is_err());
        assert_eq!(recovery_claim_status(&conn), "unknown");
        conn.execute("UPDATE generation_attempts SET usage_receipt_json=?1 WHERE attempt_id='synthetic-recovery-attempt'",
            [json!({"agentActions":{"synthetic-claim":{"status":"running"}}}).to_string()]).unwrap();
        owner.suspend_for_project_close();
        assert!(owner.agent_read(&conn, &Value::Null).is_err());
        assert_eq!(recovery_claim_status(&conn), "running");
    }

    fn prepared_owner_fixture() -> (
        crate::services::main_generation_sources::tests::TempRoot,
        Connection,
        MainGenerationOwner,
        Value,
    ) {
        let (temp, conn, owner, run) = source_owner_fixture();
        owner
            .repository
            .pause(&conn, &run.root_action_id, false)
            .unwrap();
        owner.resume(&conn, &handle_of(&run)).unwrap();
        let run = owner.repository.get(&conn, &run.run_id).unwrap();
        let request = json!({
            "handle": handle_of(&run),
            "invocationNonce": "prepared-test",
            "task": {
                "purpose": "brainstorm",
                "output": "visible-text",
                "messages": [{"role": "user", "content": "合成构思请求"}]
            }
        });
        (temp, conn, owner, request)
    }

    #[test]
    fn prepared_execution_owns_frozen_inputs_without_owner_or_database_borrow() {
        let (_temp, conn, owner, mut request) = prepared_owner_fixture();
        let prepared = owner.prepare_execution(&conn, &request, false).unwrap();
        let original_task = request["task"].clone();
        request["task"]["messages"][0]["content"] = json!("准备后修改原请求");
        let pending = owner.pending_invocations.clone();
        assert_eq!(pending.lock().unwrap().len(), 1);
        let run_id = request["handle"]["runId"].as_str().unwrap();
        let run = owner.repository.get(&conn, run_id).unwrap();
        assert!(owner
            .repository
            .budget(&conn, &run.root_action_id)
            .unwrap()
            .attempts
            .is_empty());
        assert!(Arc::ptr_eq(&prepared.service, &owner.run_service));
        drop(owner);
        drop(conn);
        // 编译和跨线程移动共同验证准备值没有 owner/连接生命周期借用。
        std::thread::spawn(move || {
            assert_eq!(prepared.task, original_task);
            match &prepared.plan {
                PreparedExecutionPlan::Dispatch(execution) => {
                    assert_eq!(execution.invocation_nonce, "prepared-test");
                    assert!(execution.reserved_tokens > 0);
                    assert!(!execution.provider_request.is_null());
                }
                PreparedExecutionPlan::Replay(_) => panic!("首次准备不能重放"),
            }
            drop(prepared);
        })
        .join()
        .unwrap();
        assert!(pending.lock().unwrap().is_empty());
    }

    #[test]
    fn prepared_execution_plan_failure_cleans_pending_without_reserving_attempt() {
        let (_temp, conn, owner, request) = prepared_owner_fixture();
        // 保留其他证据，只撤销运行租约，使登记后的计划准备失败。
        owner.run_leases.lock().unwrap().clear();
        let error = owner
            .prepare_execution(&conn, &request, false)
            .err()
            .unwrap();
        assert_eq!(error, mutating_error("GENERATION_RESUME_REQUIRED"));
        assert!(owner.pending_invocations.lock().unwrap().is_empty());
        let run = owner
            .repository
            .get(&conn, request["handle"]["runId"].as_str().unwrap())
            .unwrap();
        assert!(owner
            .repository
            .budget(&conn, &run.root_action_id)
            .unwrap()
            .attempts
            .is_empty());
    }

    #[test]
    fn prepared_execution_duplicate_guard_preserves_conflict_until_last_drop() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        let owner = owner_with(&conn);
        let first = owner.register_pending_invocation("key", "hash-1").unwrap();
        let second = owner.register_pending_invocation("key", "hash-1").unwrap();
        drop(first);
        assert_eq!(owner.pending_invocations.lock().unwrap()["key"].holders, 1);
        assert_eq!(
            owner
                .register_pending_invocation("key", "hash-2")
                .err()
                .unwrap(),
            mutating_error("GENERATION_INVOCATION_CONFLICT")
        );
        drop(second);
        assert!(owner.pending_invocations.lock().unwrap().is_empty());
        let replacement = owner.register_pending_invocation("key", "hash-2").unwrap();
        drop(replacement);
        assert!(owner.pending_invocations.lock().unwrap().is_empty());
    }

    #[test]
    fn prepared_execution_unwind_reclaims_pending_guard() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        let owner = owner_with(&conn);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = owner.register_pending_invocation("key", "hash").unwrap();
            panic!("合成准备中断");
        }));
        assert!(result.is_err());
        assert!(owner.pending_invocations.lock().unwrap().is_empty());
    }

    #[test]
    fn prepared_execution_compatibility_and_terminal_replay_keep_single_attempt() {
        use crate::services::generation_run_service::{
            create_generation_run_service, DispatchOptions, GenerationDispatchResult,
            GenerationDispatcher, GenerationRunServiceDependencies,
        };
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct PreparedDispatcher {
            calls: Arc<AtomicUsize>,
            pending: Arc<Mutex<HashMap<String, PendingOwnerInvocation>>>,
        }
        impl GenerationDispatcher for PreparedDispatcher {
            fn dispatch<'a>(
                &'a self,
                _request: &'a Value,
                _options: DispatchOptions<'a>,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = Result<GenerationDispatchResult, String>>
                        + Send
                        + 'a,
                >,
            > {
                Box::pin(async move {
                    assert_eq!(self.pending.lock().unwrap().len(), 1);
                    self.calls.fetch_add(1, Ordering::SeqCst);
                    Ok(GenerationDispatchResult {
                        usage: None,
                        finish_reason: Some("stop".into()),
                    })
                })
            }
        }

        let (_temp, conn, mut owner, request) = prepared_owner_fixture();
        let calls = Arc::new(AtomicUsize::new(0));
        owner.run_service = Arc::new(create_generation_run_service(
            GenerationRunServiceDependencies {
                repository: GenerationRunRepository::default(),
                dispatch: Arc::new(PreparedDispatcher {
                    calls: calls.clone(),
                    pending: owner.pending_invocations.clone(),
                }),
                on_reasoning: None,
                on_snapshot: None,
                on_durable_snapshot: None,
                validate_recovery: None,
            },
        ));
        let first = owner.execute(&conn, &request).unwrap();
        assert!(owner.pending_invocations.lock().unwrap().is_empty());
        // 已结算重放不能再次分发，也不需要重新获得运行模型租约。
        owner.run_leases.lock().unwrap().clear();
        let replay = owner.execute(&conn, &request).unwrap();
        let mut first_outcome = first["outcome"].clone();
        let mut replay_outcome = replay["outcome"].clone();
        // deadlineAt 是预算读回时按剩余活跃时间投影的时钟值，不是重放身份。
        for outcome in [&mut first_outcome, &mut replay_outcome] {
            let deadline = outcome["receipt"]["budget"]
                .as_object_mut()
                .unwrap()
                .remove("deadlineAt")
                .unwrap();
            assert!(deadline.as_i64().is_some_and(|value| value > 0));
        }
        assert_eq!(first_outcome, replay_outcome);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(owner.pending_invocations.lock().unwrap().is_empty());
        let run = owner
            .repository
            .get(&conn, request["handle"]["runId"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            owner
                .repository
                .budget(&conn, &run.root_action_id)
                .unwrap()
                .attempts
                .len(),
            1
        );
    }

    #[test]
    fn execute_rejects_invalid_semantic_task_before_dispatch() {
        // 非法语义任务必须在预算预留、模型租约及网络分发前被拒绝。
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let run = open_run(&conn, "project-1", "lease-1", "chapter-draft").expect("open run");
        let owner = owner_with(&conn);
        let before = owner
            .repository()
            .budget(&conn, &run.root_action_id)
            .expect("budget");
        let request = json!({ "handle": handle_of_run(&run), "task": null });
        let expected =
            crate::services::main_generation_plan::assert_semantic_generation_task(&Value::Null)
                .expect_err("非法任务");
        assert_eq!(
            owner.execute(&conn, &request).expect_err("拒绝执行"),
            mutating_error(map_generation_error(&expected))
        );
        let after = owner
            .repository()
            .budget(&conn, &run.root_action_id)
            .expect("budget");
        assert_eq!(before.attempts.len(), after.attempts.len());
        assert!(owner.pending_invocations.lock().unwrap().is_empty());
    }

    #[test]
    fn execute_rejects_agent_round_run_at_ipc_boundary() {
        // 基线 :504——agent-round 运行的 IPC execute
        // （executionInitialization 恒 false）→
        // GENERATION_AGENT_ADMISSION_REQUIRED（回归保护：
        // execute → execute_inner(agent_execution=false)）。
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let run =
            open_run(&conn, "project-1", "lease-1", "agent-round").expect("open agent-round run");
        let owner = owner_with(&conn);
        let request = json!({
            "handle": handle_of_run(&run),
            "task": { "purpose": "agent-round", "output": "visible-text", "messages": [] },
        });
        let error = owner
            .execute(&conn, &request)
            .expect_err("agent-round IPC execute 必须拒绝");
        assert_eq!(error, mutating_error("GENERATION_AGENT_ADMISSION_REQUIRED"));
    }

    #[test]
    fn read_run_rejects_identity_mismatch() {
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let run = open_run(&conn, "project-1", "lease-1", "chapter-draft").expect("open run");
        let owner = owner_with(&conn);
        // 篡改 rootActionId → 句柄与持久运行不一致
        let mut tampered = handle_of_run(&run);
        tampered["rootActionId"] = Value::String("other-root".to_string());
        let error = owner
            .read_run(&conn, &tampered)
            .expect_err("identity mismatch");
        assert_eq!(error, mutating_error("GENERATION_RUN_IDENTITY_MISMATCH"));
        // 他项目句柄 → 项目归属不匹配
        let foreign =
            open_run(&conn, "project-2", "lease-2", "chapter-draft").expect("foreign run");
        let error = owner
            .read_run(&conn, &handle_of_run(&foreign))
            .expect_err("cross-project");
        assert_eq!(error, mutating_error("GENERATION_RUN_IDENTITY_MISMATCH"));
    }

    #[test]
    fn read_run_short_circuits_private_planning_history() {
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let run = open_run(&conn, "project-1", "lease-1", "planning").expect("open run");
        // 私有规划史：policy.planning.outlineProtocol='chapter-parts-v2' 且版本
        // 非法。公开 API 无法改 manifest，测试直接改 binding_json 构造。
        conn.execute(
            "UPDATE generation_runs SET binding_json=json_set(binding_json,'$.sourceManifest.policy',json(?1)) WHERE run_id=?2",
            [
                json!({"version": "bogus-v9", "planning": {"outlineProtocol": "chapter-parts-v2"}}).to_string(),
                run.run_id.clone(),
            ],
        )
        .expect("patch policy");
        let owner = owner_with(&conn);
        // 基线在 isPrivatePlanningHistory 命中时直接返回 run，跳过策略校验
        //（否则非法版本会报 GENERATION_POLICY_*）。
        let view = owner
            .read_run(&conn, &handle_of_run(&run))
            .expect("private planning history short-circuits policy validation");
        assert_eq!(view["status"], Value::String("running".to_string()));
    }

    #[test]
    fn list_returns_current_project_views_newest_first() {
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let local = open_run(&conn, "project-1", "lease-1", "chapter-draft").expect("local run");
        let _foreign =
            open_run(&conn, "project-2", "lease-2", "chapter-draft").expect("foreign run");
        let owner = owner_with(&conn);
        let views = owner.list(&conn).expect("list");
        let array = views.as_array().expect("list returns an array");
        assert_eq!(array.len(), 1);
        assert_eq!(
            array[0]["handle"]["runId"],
            Value::String(local.run_id.clone())
        );
        assert_eq!(
            array[0]["operation"],
            Value::String("chapter-draft".to_string())
        );
    }

    #[test]
    fn list_directory_progress_attaches_author_inputs() {
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let run = open_run(&conn, "project-1", "lease-1", "directory").expect("open run");
        let owner = owner_with(&conn);
        // 无目录进度记录 → 空数组
        let progress = owner
            .list_directory_progress(&conn)
            .expect("list directory progress");
        assert_eq!(progress, Value::Array(vec![]));
        let _ = run;
    }

    #[test]
    fn read_visible_composition_returns_null_without_record() {
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let run = open_run(&conn, "project-1", "lease-1", "chapter-draft").expect("open run");
        let owner = owner_with(&conn);
        let composition = owner
            .read_visible_composition(&conn, &handle_of_run(&run))
            .expect("read visible composition");
        assert!(composition.is_null());
    }

    #[test]
    fn compose_visible_rejects_agent_round_and_algorithm_mismatch() {
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let owner = owner_with(&conn);
        // agent-round 运行 → GENERATION_AGENT_CONTROL_IMMUTABLE
        let agent_run = open_run(&conn, "project-1", "lease-1", "agent-round").expect("agent run");
        let error = owner
            .compose_visible(
                &conn,
                &handle_of_run(&agent_run),
                &[],
                None,
                Some("visible-continuation-v1"),
            )
            .expect_err("agent round is immutable");
        assert_eq!(error, mutating_error("GENERATION_AGENT_CONTROL_IMMUTABLE"));
        // 非 chapter-draft 运行 + draft-visible 算法 → GENERATION_COMPOSITION_ALGORITHM_INVALID
        let draft_run = open_run(&conn, "project-1", "lease-1", "directory").expect("draft run");
        let error = owner
            .compose_visible(
                &conn,
                &handle_of_run(&draft_run),
                &[],
                None,
                Some(DRAFT_VISIBLE_TEXT_VERSION),
            )
            .expect_err("draft-visible algorithm on non chapter-draft");
        assert_eq!(
            error,
            mutating_error("GENERATION_COMPOSITION_ALGORITHM_INVALID")
        );
    }

    #[test]
    fn owner_close_is_gated() {
        let cache = MainGenerationOwnerCache::new();
        let active = active();
        {
            let mut guard = cache
                .authorized_owner(&session("project-1", "lease-1"), Some(&active))
                .expect("gate");
            guard.owner().suspend_for_project_close();
            let error = guard.owner().assert_current().expect_err("owner suspended");
            assert_eq!(error, mutating_error(GENERATION_OWNER_CLOSED));
            // 幂等：二次挂起不 panic
            guard.owner().suspend_for_project_close();
        }
    }

    #[test]
    fn suspend_hook_evicts_entry() {
        let cache = MainGenerationOwnerCache::new();
        let active = active();
        cache
            .authorized_owner(&session("project-1", "lease-1"), Some(&active))
            .expect("gate");
        cache.suspend_for_project_close(r"F:\novel\demo");
        assert!(cache.is_empty());
        // 逐出后重新进门控 → 重建开放的新 owner
        let mut guard = cache
            .authorized_owner(&session("project-1", "lease-1"), Some(&active))
            .expect("re-gate");
        guard
            .owner()
            .assert_current()
            .expect("rebuilt owner is open");
    }
    #[test]
    fn read_character_proposal_rejects_unknown_batch_test() {
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let owner = owner_with(&conn);
        let error = owner
            .read_character_proposal(&conn, "cpb:missing")
            .expect_err("未知提案批次必须拒绝");
        assert_eq!(error, mutating_error("CHARACTER_PROPOSAL_RECEIPT_INVALID"));
    }

    #[test]
    fn approve_character_proposal_rejects_unknown_batch_test() {
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let owner = owner_with(&conn);
        let request = json!({
            "proposalBatchId": "cpb:missing",
            "expectedRevision": 1,
            "operationId": "op-1",
            "selections": []
        });
        // 幂等重放前置的 read 步先失败（未知批次）
        let error = owner
            .approve_character_proposal(&conn, &request)
            .expect_err("未知提案批次必须拒绝");
        assert_eq!(error, mutating_error("CHARACTER_PROPOSAL_RECEIPT_INVALID"));
    }

    #[test]
    fn cancel_character_proposal_rejects_invalid_request_test() {
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let owner = owner_with(&conn);
        // 键集超集（含非法键 illegalKey）→ 请求校验拒绝
        let request = json!({
            "proposalBatchId": "cpb:missing",
            "expectedRevision": 1,
            "illegalKey": true
        });
        let error = owner
            .cancel_character_proposal(&conn, &request)
            .expect_err("非法请求必须拒绝");
        assert_eq!(error, mutating_error("CHARACTER_APPROVAL_INVALID"));
    }

    #[test]
    fn character_identity_snapshot_returns_empty_snapshot_test() {
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let owner = owner_with(&conn);
        let snapshot = owner
            .character_identity_snapshot(&conn)
            .expect("身份快照读取失败");
        // create_tables 种子化 character_identity_meta(id='main', revision=0)
        assert_eq!(snapshot["revision"], Value::from(0));
        assert_eq!(snapshot["characters"], Value::Array(vec![]));
        assert_eq!(snapshot["aliases"], Value::Array(vec![]));
        assert_eq!(snapshot["relationships"], Value::Array(vec![]));
    }
}
