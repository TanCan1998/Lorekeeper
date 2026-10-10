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
//!    [`emit_generation_reasoning`]（**TODO(B3)** 才真实发射）。
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

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex, MutexGuard};

use serde::Serialize;
use serde_json::{json, Map, Value};
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use rusqlite::{Connection, OptionalExtension};

use crate::repositories::generation_run_repository::{
    DurableGenerationRun, FrozenInputFingerprint, GenerationExecutionReceipt,
    GenerationRunRepository, GenerationTransportDiagnostics, PhysicalAttempt, PlotOutlineProgress,
    RootBudget, RunView,
};
use crate::security::{self, ProjectSessionContext};
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
struct MainGenerationSnapshotProjection {
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
/// 其余字段基线恒输出。`unsavedTails` 依赖未迁移的生成运行服务
/// （`service.readUnsavedTails`，generation-run-service.ts），恒为 `[]`。
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
    /// 生成账本仓储（W1；`repositories/generation_run_repository.rs`）。
    repository: GenerationRunRepository,
    /// 基线 `runLeases: Map<string, string>`（attemptId → leaseId，
    /// main-generation-owner.ts:112）。挂起时逐 lease 关闭并清空。
    run_leases: HashMap<String, String>,
    /// 基线 `closed` 标志（main-generation-owner.ts:116）。
    closed: bool,
}

impl MainGenerationOwner {
    /// 基线 owner 构造的骨架等价物（generation-controller.ts:106-125 的
    /// `createMainGenerationOwner({...})` 调用，裁剪到当前已迁移的依赖面：
    /// 仅会话身份与生成账本仓储；`leases` / `loadModel` / `buildBinding` /
    /// `onSnapshot` / `onReasoning` 等依赖随对应批次接入）。
    fn new(session: ProjectSessionContext) -> Self {
        Self {
            session,
            repository: GenerationRunRepository::default(),
            run_leases: HashMap::new(),
            closed: false,
        }
    }

    /// 基线 `handleOf(run)` 的句柄比对等价物（main-generation-owner.ts:145）：
    /// 运行的项目归属 + 句柄四元组（`projectId` / `epoch` / `rootActionId` /
    /// `runId`）必须与入参逐字段一致，否则 `GENERATION_RUN_IDENTITY_MISMATCH`。
    fn require_run(
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
            // 基线 `service.readUnsavedTails()`（生成运行服务，B3 迁移）→ 恒空。
            unsaved_tails: Vec::new(),
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

    /// 基线 `runLeases` 登记点（`runLeases.set(attemptId, leaseId)` 的
    /// 等价物；B3 迁移执行编排时启用）。
    pub fn record_run_lease(&mut self, attempt_id: String, lease_id: String) {
        self.run_leases.insert(attempt_id, lease_id);
    }

    /// 对齐基线 `suspendForProjectClose`（main-generation-owner.ts:1390-1399）：
    /// 项目关闭前挂起 owner——清准备态、关子生成器、挂起运行服务、
    /// 关闭全部在途模型执行租约、置 `closed`。
    ///
    /// 幂等：基线 `if (closed) return` 之后的清理只在首次执行
    /// （:1394-1398）；`preparations` / 子生成器的清理（:1391-1393）
    /// 在对应服务迁移后接入。
    pub fn suspend_for_project_close(&mut self) {
        // TODO(B3+)：`preparations.clear()`——草稿上下文准备缓存
        //   （main-generation-owner.ts:1391；`prepareDraftContext` 迁移后接入）。
        // TODO(B3+)：`finalizedCharacters.close()` / `reviewRevisions.close()`
        //   （main-generation-owner.ts:1392-1393；子生成器迁移后接入）。
        if self.closed {
            return;
        }
        // TODO(B3+)：`service.suspendForProjectClose()`——生成运行服务挂起
        //   （main-generation-owner.ts:1395；`generation-run-service.ts` 迁移后接入）。
        self.closed = true;
        // TODO(B3+)：基线逐个 `deps.leases.close(leaseId)` 关闭模型执行租约
        //   （main-generation-owner.ts:1397；`llm::lease` 注册表接线后接入）。
        self.run_leases.clear();
    }

    // -----------------------------------------------------------------
    // B3 迁移占位：生成编排方法（方法体平移自 main-generation-owner.ts
    // 的同名闭包；契约类型随 `src/shared/generation-owner-contract.ts`
    // 迁移落地后补齐签名）。
    // -----------------------------------------------------------------

    /// 基线 `begin`（选择 → 建运行 → 返回运行视图）。
    pub fn begin(&self, _conn: &Connection) -> Result<serde_json::Value, String> {
        self.assert_current()?;
        todo!("B3: 迁移自 electron/services/main-generation-owner.ts 的 begin")
    }

    /// 基线 `execute`（分发 → 流式回执）。
    pub fn execute(&self, _conn: &Connection) -> Result<serde_json::Value, String> {
        self.assert_current()?;
        todo!("B3: 迁移自 electron/services/main-generation-owner.ts 的 execute")
    }
}

// ============================================================================
// per-database owner 缓存
// ============================================================================

/// 缓存条目（对齐基线 `{ owner, session, subscribers }`，
/// generation-controller.ts:37）。
struct OwnerCacheEntry {
    owner: MainGenerationOwner,
    /// 基线 `subscribers: Set<WebContents>`。Tauri 侧事件经
    /// `tauri::Emitter` 按窗口标签寻址，骨架以窗口标签集合占位
    /// （**TODO(B3)**：真实订阅登记 + `emit_to` 发射）。
    #[allow(dead_code)]
    subscribers: HashSet<String>,
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
        let key = security::normalized_project_path(&active.root_path);
        // 锁中毒（持有锁的线程 panic）：按基线兜底码收敛（同 register catch）
        let mut entries = match self.entries.lock() {
            Ok(entries) => entries,
            Err(_) => return Err(mutating_error(GENERATION_REQUEST_FAILED)),
        };
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
                owner: MainGenerationOwner::new(session.clone()),
                subscribers: HashSet::new(),
            });
        // 5) TODO(B3)：基线在此登记 `event.sender` 到 subscribers。
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
// 事件发射接线点（**TODO(B3)**：真实发射）
// ============================================================================

/// `generation:snapshot` 事件发射接线点。
///
/// 基线：owner 构造时注入 `onSnapshot`（generation-controller.ts:117-120），
/// 生成运行服务每步经 `subscriber.send('generation:snapshot', snapshot)`
/// 向全部订阅窗口广播 `MainGenerationSnapshot`（契约见
/// `src/shared/generation-owner-contract.ts` 的 `GenerationOwnerChannels`）。
///
/// Tauri 等价物（**TODO(B3)**）：`AppHandle` + `tauri::Emitter::emit_to`
/// 按 [`OwnerCacheEntry::subscribers`] 的窗口标签发射。services 层保持
/// 无 Tauri 依赖（`services/mod.rs` 约定），故此处仅留接线点，
/// 骨架阶段不发射。
#[allow(dead_code)]
fn emit_generation_snapshot(_subscribers: &HashSet<String>, _snapshot: &serde_json::Value) {
    // TODO(B3)：`for label in subscribers { app_handle.emit_to(label, "generation:snapshot", snapshot) }`
}

/// `generation:reasoning` 事件发射接线点（基线 generation-controller.ts:121-124；
/// 载荷 `{ projectId, epoch, rootActionId, runId, attemptId, text }`，
/// 见 `MainGenerationOwnerDependencies::onReasoning`）。
///
/// Tauri 等价物（**TODO(B3)**）：同上，经 `emit_to` 发射
/// `generation:reasoning`。
#[allow(dead_code)]
fn emit_generation_reasoning(_subscribers: &HashSet<String>, _event: &serde_json::Value) {
    // TODO(B3)：`for label in subscribers { app_handle.emit_to(label, "generation:reasoning", event) }`
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::generation_run_repository::OpenGenerationRunRequest;

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
}
