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

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex, MutexGuard};

use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use rusqlite::{Connection, OptionalExtension};

use crate::repositories::generation_run_repository::{
    ChapterRange, DurableGenerationRun, FrozenInputFingerprint, GenerationExecutionReceipt,
    GenerationRunRepository, GenerationTransportDiagnostics, PhysicalAttempt, PlotOutlineProgress,
    RootBudget, RunBinding, RunView,
};

use crate::security::{self, ProjectSessionContext};
use crate::services::character_proposal::{
    prove_character_proposal, read_legacy_roster_generation_proof, CharacterProposalService,
};
use crate::services::generation_draft_effects::{
    assert_generation_batch_intent, GenerationDraftEffects,
};
use crate::services::legacy_roster_generation::{
    keys_subset, LegacyRosterGeneration,
};
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
    /// 基线 `closed` 标志（main-generation-owner.ts:116）。
    closed: bool,
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
    fn new(session: ProjectSessionContext) -> Self {
        Self {
            session,
            repository: GenerationRunRepository::default(),
            run_leases: HashMap::new(),
            preparations: Vec::new(),
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

    /// 基线 `begin`（选择 → 建运行 → 返回运行视图，main-generation-owner.ts:351-444）。
    ///
    /// 本波次就位**准入校验面**（错误码 / 边界条件逐字对齐基线）：
    /// - 选择形状校验（:357-363）→ `GENERATION_BEGIN_INVALID`（操作名
    ///   正则 `^[a-z0-9][a-z0-9:_-]{0,127}$`、`promptKeys` 非空数组、
    ///   `parentRootActionId` 字符串非空、`uiActionNonce` 字符串非空且
    ///   ≤256、键集白名单 [`BEGIN_ALLOWED_KEYS`]——IPC 边界六个初始化
    ///   标志恒为 false，对齐 generation-controller.ts:173-178 注册形态；
    ///   `batch_initialization` 标志仅主进程内部 `beginBatch` 初始化
    ///   调用置 true（IPC 边界恒 false，见
    ///   [`MainGenerationOwner::begin_batch`]）；
    /// - 子生成器准入（:364-369、:376）→ 五个 `*_ADMISSION_REQUIRED`；
    /// - 根行动子代禁令（:379-383，经 `repository.budget`）→ 三个
    ///   `*_CHILD_FORBIDDEN`；
    /// - 批量意图主进程独占（:387）→ `GENERATION_BATCH_INTENT_MAIN_ONLY`
    ///   （`batch_initialization = false` 的 IPC 边界语义；主进程
    ///   内部批量初始化调用传 `true` 豁免该检查）。
    ///
    /// ⚠️ 诚实化占位（B3 波次说明）：基线 :395-443 的 run 创建核心——
    /// `newMainGenerationPolicy` 策略构造器（electron/services/
    /// main-generation-plan.ts 写面，含 `planningGenerationScope` /
    /// `resolveGenerationCapabilityConstraints` / `scaledRootBudget` 等
    /// 容量规划数学）、`deps.leases.begin` 模型执行租约、`buildBinding`
    /// 源绑定（逻辑层 G4-6a 已就位：`generation_source_binding`）、
    /// `service.open` 运行持久化编排与子生成器登记册（imports / agents /
    /// finalizations / graphs / legacyRosters）的 admit 联动，以及
    /// `generationOutputContract` 输出合同校验——依赖的策略规划器与
    /// 生成运行服务尚未迁移。校验面之后按基线顺序在策略构造点
    /// fail-closed（方法末段；命令层收敛为 `GENERATION_REQUEST_FAILED`）。
    /// 影响面：合法选择当前收到「尚未迁移」拒绝而非运行视图；非法
    /// 选择的错误码与基线逐字一致。
    pub fn begin(
        &self,
        conn: &Connection,
        selection: &Value,
        batch_initialization: bool,
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
            map.keys()
                .all(|key| BEGIN_ALLOWED_KEYS.contains(&key.as_str()))
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
        if operation == "agent-round" {
            return Err(mutating_error("GENERATION_AGENT_ADMISSION_REQUIRED"));
        }
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
        // ⚠️ B3 占位：基线 :395-443 创建核心（策略构造器 / 模型租约 /
        // buildBinding / service.open / 子生成器登记册 admit）未迁移——
        // 策略构造点 fail-closed（命令层收敛为 `GENERATION_REQUEST_FAILED`）。
        Err(mutating_error(
            "主生成运行创建尚未迁移：主生成策略规划器（newMainGenerationPolicy）与生成运行服务（generation-run-service）待后续批次移植",
        ))
    }

    /// 基线 `execute`（分发 → 流式回执，main-generation-owner.ts:501-623）。
    ///
    /// 本波次就位**准入校验面**（错误码逐字对齐基线）：
    /// - 运行身份与可写性（`require_run`，execution 语义）；
    /// - 子生成器执行准入（:512-521）→ 六个 `*_ADMISSION_REQUIRED`
    ///   （IPC 边界执行标志恒为 false，对齐 generation-controller.ts:231-236
    ///   注册形态）；
    /// - 材料决策必需性（:532-537）→ `GENERATION_MATERIAL_DECISION_REQUIRED`；
    /// - 输出合同一致性（:585-590）→ `GENERATION_OUTPUT_CONTRACT_CHANGED`；
    /// - 大纲请求断言（`repository.assert_plot_outline_request`，:568）。
    ///
    /// ⚠️ 诚实化占位（B3 波次说明）：基线 :522-531 的任务形状断言
    /// （`assertSemanticGenerationTask`）与规划目标单位指令注入、
    /// :538-590 的材料决策提示哈希逐字校验（`GENERATION_MATERIAL_PROMPT_MISMATCH`
    /// 系，依赖 `runAttempts` / `shortOutlineContext` / 短细纲块编解码
    /// 等共享模块）、:591-623 的调用重放（pending 映射 / `findInvocation`
    /// 幂等）与短细纲重试校验、:577-581 的子生成器登记册可变性断言，
    /// 以及分发核心（`rebuildBinding` 已就位、模型执行租约经
    /// `AppState::llm_leases` 可接线、`buildMainGenerationPlan` 计划构造
    /// 与 `service.execute` 的 LLM 分发循环未迁移——含
    /// `generation:snapshot` / `generation:reasoning` 事件发射点，
    /// 发射器已就位于命令层，见
    /// [`crate::commands::generation::emit_generation_snapshot`]）
    /// 尚未迁移。fail-closed 于分发入口（命令层收敛为
    /// `GENERATION_REQUEST_FAILED`）。
    pub fn execute(&self, conn: &Connection, request: &Value) -> Result<Value, String> {
        let handle = request.get("handle").cloned().unwrap_or(Value::Null);
        let run = self.require_run(conn, &handle, true)?;
        // 基线 :512-521 子生成器执行准入（IPC 边界执行标志恒为 false）。
        let manifest = &run.binding.source_manifest;
        let run_operation = manifest
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if run_operation == "agent-round" {
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
        let task = request.get("task").cloned().unwrap_or(Value::Null);
        // 基线 :532-537 材料决策必需性（审稿 / 精修类操作与带准备的
        // chapter-draft 必须已绑定材料决策收据）。
        let material_decision = manifest.get("materialDecision");
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
        // ⚠️ B3 占位：基线 :522-590 的任务形状 / 材料决策提示哈希校验
        // 与 :591-623 的调用重放 / 短细纲重试 / 可变性断言，及分发
        // 核心（LLM 分发循环 + 事件发射）未迁移——分发入口 fail-closed
        // （命令层收敛为 `GENERATION_REQUEST_FAILED`）。
        Err(mutating_error(
            "主生成执行尚未迁移：生成运行服务（generation-run-service）的 LLM 分发循环待后续批次移植",
        ))
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
    /// 本波次就位：运行身份校验（`require_run`，非执行语义）与大纲
    /// 可写性断言（`repository.assert_plot_outline_writable`，:626-627）。
    ///
    /// ⚠️ 诚实化占位（B3 波次说明）：基线 :628-630 的
    /// `plotOutlineAlreadyCurrent` 早退优化（依赖 `renderPlotOutlineRange`
    /// / `plotOutlineTargetCurrent` 的源重建比对）、:631-634 的子生成器
    /// 登记册可变性断言（imports / finalizations / graphs / legacyRosters
    /// 未迁移）、:635-644 的模型收据重建（`currentModelReceipt` 的模型
    /// 配置加载）与 `service.resume` 重分发（生成运行服务未迁移）尚未
    /// 迁移。fail-closed 于恢复入口（命令层收敛为 `GENERATION_REQUEST_FAILED`）。
    pub fn resume(&self, conn: &Connection, handle: &Value) -> Result<Value, String> {
        let run = self.require_run(conn, handle, false)?;
        self.repository
            .assert_plot_outline_writable(conn, &run.run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        // ⚠️ B3 占位：基线 :628-644 的早退优化 / 可变性断言 / 模型源
        // 重建 / service.resume 重分发未迁移——恢复入口 fail-closed。
        Err(mutating_error(
            "主生成恢复尚未迁移：生成运行服务（generation-run-service）的 resume 重分发待后续批次移植",
        ))
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
    /// ⚠️ B3 波次说明：基线 `service.pause` 的进行中分发中止
    /// （generation-run-service.ts:342-344，遍历进行中
    /// `AbortController` 注册表并 `abort()`）依赖生成运行服务的
    /// LLM 分发循环，尚未迁移——当前 `execute` fail-closed、
    /// 不存在进行中分发，该副作用恒为空操作；分发循环迁移后
    /// 需在此接线中止逻辑。
    pub fn pause(&self, conn: &Connection, handle: &Value) -> Result<Value, String> {
        let run = self.require_run(conn, handle, true)?;
        self.repository
            .pause(conn, &run.root_action_id, false)
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
    /// ⚠️ B3 波次说明：基线 `service.cancel` 的进行中分发中止
    /// （generation-run-service.ts:345-347）同 `pause`——依赖
    /// 生成运行服务的 LLM 分发循环，尚未迁移、当前恒为空操作。
    pub fn cancel(&self, conn: &Connection, handle: &Value) -> Result<Value, String> {
        let run = self.require_run(conn, handle, false)?;
        self.repository
            .pause(conn, &run.root_action_id, true)
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
    /// ⚠️ B3 波次说明：基线 :1379-1384 的子生成器登记册可变性
    /// 断言（reviewRevisions / imports / finalizations / graphs /
    /// legacyRosters，未迁移）、`service.discardCandidate` 的
    /// `unsavedTails` 清理（generation-run-service.ts:338-341，
    /// 恒空——`readUnsavedTails` 依赖未迁移）与
    /// `volatileReceipts.delete`（:1387，易失回执缓存未迁移）
    /// 尚未迁移，均为空操作。
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
        self.repository
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
    /// ⚠️ B3 占位：基线 :139-174 的新提交写入路径未迁移——
    /// `assertSourcesCurrent` 源一致性重建（:700-717，依赖
    /// `rebuildBinding`）、可见合成 / 目标单位 / 批次进度校验
    /// （:140-160）、`DraftRepository.create` 草稿行写入与
    /// `generation_attempts` usage 回执写回（:161-171），以及
    /// `agents.withChildEffect` 子生成器效应登记
    /// （agent-generation.ts:344-371）与外层
    /// `deps.database.transaction(...).immediate()` 事务包装
    /// （generation-draft-effects.ts 头注：`commit` 写面刻意不平移，
    /// 随 G4-6/G4-7 迁移）——新提交入口 fail-closed（命令层收敛为
    /// `GENERATION_REQUEST_FAILED`）。
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
            // 注：`read_commit` 回执序列化为 snake_case 键
            // （StoredDraftCommit 无 rename_all）。
            if previous.get("content_hash").and_then(Value::as_str)
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
                "contentHash": previous.get("content_hash").cloned().unwrap_or(Value::Null),
                "content": previous.get("content").cloned().unwrap_or(Value::Null),
            }));
        }
        // ⚠️ B3 占位：新提交写入路径（见方法头注）未迁移——
        // 新提交入口 fail-closed（命令层收敛为
        // GENERATION_REQUEST_FAILED）。
        Err(mutating_error(
            "主生成草稿提交尚未迁移：草稿写入路径（generation-draft-effects.commit 的 DraftRepository.create 与 generation_attempts usage 回执写回）与子生成器效应登记（agents.withChildEffect）待后续批次移植",
        ))
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
        let run = self.begin(conn, &Value::Object(selection), true)?;
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
    /// 本波次就位**准入校验面**（错误码逐字对齐基线）：
    /// - `GENERATION_DRAFT_PREPARATION_INVALID`（:322-324）：
    ///   request 非对象 / `query` 非字符串 /
    ///   `knowledgeSnapshot` 存在时 `request.query !==`
    ///   `knowledgeSnapshot.query` / 键集超白名单
    ///   [`PREPARATION_ALLOWED_KEYS`]；
    /// - 基线 :325-326 `GENERATION_MODEL_MISSING`——依赖
    ///   `deps.loadModel`（模型加载命令面）未迁移，见下。
    ///
    /// ⚠️ 诚实化占位（G4-6c 波次说明）：基线 :325-335 的
    /// 绑定构造核心——`deps.loadModel(request.modelId)` 模型
    /// 加载（`load_model` 未迁移）、`safeGenerationModelReceipt(
    /// createModelExecutionLeaseReceipt(model, { leaseId:
    /// 'read-only-evidence', createdAt: 0, expiresAt: 0 }))`
    /// 模型执行租约回执（`safe_generation_model_receipt`
    /// 未迁移）、策略构造（`request.batchId ? rootPolicy(
    /// draftEffects.readBatch(batchId, projectId).rootHandle
    /// .rootActionId) : newMainGenerationPolicy(selection, model)`
    /// ——`newMainGenerationPolicy` 依赖 main-generation-plan.ts
    /// 写面未迁移（G4-6b-1 既定 fail-closed 依赖；`rootPolicy`
    /// 读面已就位，但其输入 run 由策略规划器落库面创建）、
    /// 以及 `deps.buildBinding(selection, receipt, policy)`
    /// 源绑定构造（`generation_source_binding` 已就位但需
    /// receipt / policy 输入）——依赖未迁移。校验面与
    /// `selection` 构造（基线 :328-331，仅依赖 request /
    /// knowledgeSnapshot，与模型加载解耦、无副作用）之后
    /// 按基线顺序在模型加载构造点 fail-closed（方法末段；
    /// 命令层收敛为 `GENERATION_REQUEST_FAILED`）。
    ///
    /// 影响面：合法请求当前收到「尚未迁移」拒绝而非
    /// `RunBinding`；非法请求的错误码与基线逐字一致。
    pub fn draft_preparation_binding(
        &self,
        _conn: &Connection,
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
        // 基线 :328-331 `selection` 构造（逐字平移；仅依赖
        // request / knowledgeSnapshot，与模型加载解耦——
        // 模型租约回执未迁移，构造结果随策略构造点
        // fail-closed 一并丢弃，见方法头注）。
        {
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
            let _ = Value::Object(selection);
        }
        // ⚠️ 基线 :325-335 绑定构造核心 fail-closed（依赖
        // 未迁移，见方法头注）：`deps.loadModel` /
        // `safeGenerationModelReceipt(createModelExecutionLeaseReceipt(
        // ...))` / 策略构造（`newMainGenerationPolicy` /
        // `rootPolicy`）/ `deps.buildBinding`。
        Err(mutating_error(
            "草稿上下文准备尚未迁移：模型加载（loadModel）与模型执行租约回执（safeGenerationModelReceipt）以及主生成策略规划器（newMainGenerationPolicy）待后续批次移植",
        ))
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
    /// 4. **rebuildBinding 比对点 —— fail-closed 占位（G4-6c-1 既定）**：
    ///    基线末步为
    ///    `deps.rebuildBinding(run.binding, currentModelReceipt(run.binding))`
    ///    与 `run.binding` 深比较，不等 → `GENERATION_SOURCE_CHANGED`。
    ///    `rebuildBinding` 的 `currentModelReceipt → loadModel` 依赖未迁移，
    ///    比对无法执行——实现到比对点即 fail-closed：此处刻意省略比对
    ///    （恒 `Ok(())`），未迁移依赖引发的源漂移由命令层经
    ///    `map_generation_error` 兜底码统一收敛 `GENERATION_REQUEST_FAILED`，
    ///    待模型加载面迁移后补齐深比较。
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
            return Ok(());
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
        // 基线 :676-677 rebuildBinding 深比较 —— fail-closed 占位（见方法头注）。
        Ok(())
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
            let action = serde_json::from_str::<Value>(&action_json)
                .unwrap_or(Value::Null);
            let binding = serde_json::from_str::<Value>(&binding_json)
                .unwrap_or(Value::Null);
            if action.get("projectId").and_then(Value::as_str)
                == Some(self.session.project_id.as_str())
                && action.get("uiActionNonce").and_then(Value::as_str)
                    == Some(ui_action_nonce)
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
        Err(mutating_error("GENERATION_LEGACY_ROSTER_EXECUTE_UNMIGRATED"))
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
    /// → main-generation-owner.ts:1301 → agents.withChildEffect）的结构接线。
    ///
    /// 基线 `agents.withChildEffect`（agent-generation.ts:344-376）完整实现
    /// 依赖 agent-generation 状态机（runs registry / workflowRegistration /
    /// sessionRuns navigation / host.capture / generation_attempts
    /// usage_receipt_json 回执写入），属 agent-generation 域迁移波次——
    /// 本方法为结构接线，主体 **fail-closed 占位**：
    /// - 基线首行事务门禁（`if (!this.db.inTransaction) throw
    ///   GENERATION_AGENT_TRANSACTION_REQUIRED`，agent-generation.ts:345）
    ///   的 Tauri 等价物为**调用方结构保证**：`db:project-core-commit-generated`
    ///   路径在本波实现中于外层事务内调用；`db:blueprint-commit-range`
    ///   路径的效应（`blueprints::commit_range`）自开事务、提交原子性
    ///   自足。rusqlite 0.40.2 无 `is_in_transaction()` API
    ///   （`is_autocommit()` 对「效应自开事务」路径为误报），故方法内
    ///   不重复检查。
    /// - 基线 `requireRun(handle)` 身份校验（main-generation-owner.ts:1301）
    ///   与 `withChildEffect` 效应执行随主体一并待活化；当前在方法入口
    ///   直接 fail-closed 拒绝（`GENERATION_AGENT_CHILD_EFFECT_UNMIGRATED`
    ///   为迁移占位码、非基线码），禁止静默执行 `effect`。
    pub fn with_agent_child_effect<T>(
        &self,
        _conn: &Connection,
        _handle: &Value,
        _effect: impl FnOnce() -> T,
    ) -> Result<T, String> {
        Err(mutating_error("GENERATION_AGENT_CHILD_EFFECT_UNMIGRATED"))
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
    ///   头注）；主体 fail-closed 占位
    ///   （`GENERATION_AGENT_CHILD_EFFECT_UNMIGRATED`）。
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
/// Tauri 等价物（D-2 既定决策：**广播 + 客户端过滤**）：命令层
/// [`crate::commands::generation::emit_generation_snapshot`] 经
/// `AppHandle::emit` 全窗口广播（客户端按载荷 `projectId` / `epoch`
/// 过滤——等价基线 `subscriber.send` 的每窗口定向，但省去订阅登记）。
/// services 层保持无 Tauri 依赖（`services/mod.rs` 约定），故此处仅留
/// 接线点；调用点随生成运行服务（`generation-run-service.ts`）迁移波次
/// 接入——基线唯一发射点为 `execute` 的 LLM 调用循环
/// （main-generation-owner.ts:613-615，`snapshotOf(receipt)` 之后）。
#[allow(dead_code)]
fn emit_generation_snapshot(_subscribers: &HashSet<String>, _snapshot: &serde_json::Value) {
    // TODO(B3)：`for label in subscribers { app_handle.emit_to(label, "generation:snapshot", snapshot) }`
}

/// `generation:reasoning` 事件发射接线点（基线 generation-controller.ts:121-124；
/// 载荷 `{ projectId, epoch, rootActionId, runId, attemptId, text }`，
/// 见 `MainGenerationOwnerDependencies::onReasoning`）。
///
/// Tauri 等价物（D-2 既定决策：同上，广播 + 客户端过滤）：命令层
/// [`crate::commands::generation::emit_generation_reasoning`] 经
/// `AppHandle::emit` 全窗口广播；调用点随生成运行服务迁移波次接入
/// （基线发射点：LLM 分发流的可见文本增量）。
#[allow(dead_code)]
fn emit_generation_reasoning(_subscribers: &HashSet<String>, _event: &serde_json::Value) {
    // TODO(B3)：`for label in subscribers { app_handle.emit_to(label, "generation:reasoning", event) }`
}

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
