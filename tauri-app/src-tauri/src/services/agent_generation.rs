//! AgentGeneration —— Agent 生成状态机服务层（agent-generation 域子波次 A）。
//!
//! 平移自 `electron/services/agent-generation.ts`（405 行，`AgentGeneration` 类）
//! 的核心状态机方法。类型镜像 `src/shared/agent-generation.ts`。
//!
//! 架构要点（与基线逐字对应）：
//! - 构造签名 `(db, runs, scope: {projectId, epoch}, assertCurrent, host)`；
//!   一个 owner 实例持有一个 `agents` 实例（基线 main-generation-owner.ts:899）。
//! - **状态存储无新表**：导航 / 控制状态全部落在既有
//!   `generation_runs.binding_json.sourceManifest`（`agentContext` /
//!   `agentContextHash` / `agentSession` / `agentWorkflowRegistrationId` /
//!   `modelReceipt` / `operation`）与 `generation_attempts.usage_receipt_json`
//!   （`agentActions` / `agentAuthorSelections` / `agentSourceTransition` /
//!   `agentControlEpoch`）之上（基线注释："Agent navigation/control lives on
//!   existing runs and attempt receipts. No second budget or writer."）。
//! - 身份门禁链：`require_run`（`GENERATION_AGENT_IDENTITY_MISMATCH`）→
//!   `context`（`GENERATION_AGENT_CONTEXT_INVALID`）→ `navigation`
//!   （`GENERATION_AGENT_NAVIGATION_INVALID`）。
//! - `AgentHost` 在本波仅定义 trait + 方法签名；host 实现由子波次 B 在
//!   owner 侧接线（基线 main-generation-owner.ts:899-906）。
//!
//! 与基线的一致性约定（对齐 `generation_run_repository` 头注）：
//! - 基线 `db.transaction(fn).immediate()` → `conn.unchecked_transaction()`
//!   （DEFERRED；单写入者模型下行为等价）；基线 better-sqlite3 的嵌套事务
//!   自动降级为 SAVEPOINT，rusqlite 无嵌套事务——`is_autocommit()` 检测到
//!   外层环境事务时直接复用（savepoint 等价语义）。
//! - 基线 `if (!db.inTransaction)` 事务门禁（`GENERATION_AGENT_TRANSACTION_REQUIRED`）
//!   的 Tauri 等价物为**调用方结构保证**（对齐 `main_generation_owner`
//!   `with_agent_child_effect` 头注约定：rusqlite 0.40.2 无 `is_in_transaction()`
//!   API，方法内不重复检查）。
//! - `isDeepStrictEqual` → `serde_json::to_value` 后的 `Value` 相等比较；
//!   `structuredClone` → `Value` / 派生 `Clone` 的深拷贝。
//! - 基线 `JSON.parse` 松散读取（保留全部既有键回写）→ 以 `serde_json::Value`
//!   原样存取，不做结构校验（全保真）。
//!
//! ⚠️ 偏差（相对上游运行期行为，均为损坏态 / 边界态防御）：
//! - 基线对畸形松散对象的访问抛 `TypeError`（经 controller 折叠为
//!   `GENERATION_REQUEST_FAILED`）；Rust 侧对不可达的损坏态以描述性
//!   非契约字符串错误返回（同样折叠为 `GENERATION_REQUEST_FAILED`），
//! - 基线 `String.length`（UTF-16 码元）→ Rust `chars().count()`；
//!   小说正文为 BMP 文本时二者一致。
//! - `Number.isSafeInteger(x)` 对 `x.0` 形式浮点判真；`serde_json` 把
//!   `1.0` 解析为 f64，导航 `roundIndex` 等处按非整数拒绝（运行期
//!   `roundIndex` 恒为整数写出，不可达）。
//!
//! 分发边界：生产首轮已接统一短锁生命周期；直连 execute_round 仍保守关闭。
//! resume 对已完成回复仅移交控制纪元，未完成阶段仍由宿主准入决定。
//! `admit_workflow` 已随子波次 B 逐字移植（owner.begin 子代准入）；
//! `commit_domain_tool` 已随子波次 C 逐字移植（基线 :377-404，
//! 域工具效应提交，依赖 `agent_domain_tool_effect`）。

use crate::repositories::generation_run_repository::{
    DurableGenerationRun, GenerationRunRepository, MainGenerationRunHandle, RunBinding,
};
use crate::services::agent_domain_tool_effect;
use crate::services::agent_generation_context::{
    validate_agent_generation_input, AgentGenerationContext, AgentGenerationInput, AgentToolSource,
};
use crate::services::prompt_system_role::WritingLanguage;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

// ── 基线错误码（逐字对齐 electron/services/agent-generation.ts）──
// 除 `GENERATION_EPOCH_STALE`（基线 finishTool:263 逐字如此，无 AGENT
// 前缀）与迁移占位码外，其余均匹配 controller 透传正则 `^GENERATION_[A-Z_]+$`。

/// `GENERATION_AGENT_BEGIN_INVALID` — begin 请求形态非法。
pub const GENERATION_AGENT_BEGIN_INVALID: &str = "GENERATION_AGENT_BEGIN_INVALID";
/// `GENERATION_AGENT_IDENTITY_MISMATCH` — handle/run 身份不匹配。
pub const GENERATION_AGENT_IDENTITY_MISMATCH: &str = "GENERATION_AGENT_IDENTITY_MISMATCH";
/// `GENERATION_AGENT_CONTEXT_INVALID` — agentContext 缺失 / 版本非 1 / 哈希不匹配。
pub const GENERATION_AGENT_CONTEXT_INVALID: &str = "GENERATION_AGENT_CONTEXT_INVALID";
/// `GENERATION_AGENT_NAVIGATION_INVALID` — agentSession.key 非 64 位 hex 或 roundIndex 越界。
pub const GENERATION_AGENT_NAVIGATION_INVALID: &str = "GENERATION_AGENT_NAVIGATION_INVALID";
/// `GENERATION_AGENT_EFFECT_BINDING_INVALID` — agentSourceTransition 的 afterHash 不匹配。
pub const GENERATION_AGENT_EFFECT_BINDING_INVALID: &str = "GENERATION_AGENT_EFFECT_BINDING_INVALID";
/// `GENERATION_AGENT_ROOT_MISMATCH` — 会话轮次 rootActionId 不一致。
pub const GENERATION_AGENT_ROOT_MISMATCH: &str = "GENERATION_AGENT_ROOT_MISMATCH";
/// `GENERATION_AGENT_ROUND_GAP` — 会话轮次 index 不连续。
pub const GENERATION_AGENT_ROUND_GAP: &str = "GENERATION_AGENT_ROUND_GAP";
/// `GENERATION_AGENT_ROUND_AMBIGUOUS` — 单轮次存在多个 attempt。
pub const GENERATION_AGENT_ROUND_AMBIGUOUS: &str = "GENERATION_AGENT_ROUND_AMBIGUOUS";
/// `GENERATION_AGENT_NONCE_CONFLICT` — 既有 session 的 input / modelId 与请求冲突。
pub const GENERATION_AGENT_NONCE_CONFLICT: &str = "GENERATION_AGENT_NONCE_CONFLICT";
/// `GENERATION_AGENT_TOOL_IDENTITY_MISMATCH` — 工具动作 ref 精确定位失败。
pub const GENERATION_AGENT_TOOL_IDENTITY_MISMATCH: &str = "GENERATION_AGENT_TOOL_IDENTITY_MISMATCH";
/// `GENERATION_AGENT_CONFIRMATION_REQUIRED` — confirmed 非 boolean。
pub const GENERATION_AGENT_CONFIRMATION_REQUIRED: &str = "GENERATION_AGENT_CONFIRMATION_REQUIRED";
/// `GENERATION_AGENT_AUTHOR_SELECTION_INVALID` — 作者蓝图提案形态非法。
pub const GENERATION_AGENT_AUTHOR_SELECTION_INVALID: &str =
    "GENERATION_AGENT_AUTHOR_SELECTION_INVALID";
/// `GENERATION_AGENT_AUTHOR_SELECTION_CONFLICT` — 既有 selections 与请求冲突。
pub const GENERATION_AGENT_AUTHOR_SELECTION_CONFLICT: &str =
    "GENERATION_AGENT_AUTHOR_SELECTION_CONFLICT";
/// `GENERATION_AGENT_TOOL_STALE` — root epoch / 状态 / 源指纹过期。
pub const GENERATION_AGENT_TOOL_STALE: &str = "GENERATION_AGENT_TOOL_STALE";
/// `GENERATION_AGENT_TOOL_RESULT_INVALID` — finish-tool 请求形态非法。
pub const GENERATION_AGENT_TOOL_RESULT_INVALID: &str = "GENERATION_AGENT_TOOL_RESULT_INVALID";
/// `GENERATION_AGENT_TOOL_RESULT_CONFLICT` — 工具结果幂等重放冲突。
pub const GENERATION_AGENT_TOOL_RESULT_CONFLICT: &str = "GENERATION_AGENT_TOOL_RESULT_CONFLICT";
/// `GENERATION_AGENT_WORKFLOW_NOT_AUTHORIZED` — start_workflow 注册未授权。
pub const GENERATION_AGENT_WORKFLOW_NOT_AUTHORIZED: &str =
    "GENERATION_AGENT_WORKFLOW_NOT_AUTHORIZED";
/// `GENERATION_AGENT_WORKFLOW_INVALID` — 工作流参数形态非法。
pub const GENERATION_AGENT_WORKFLOW_INVALID: &str = "GENERATION_AGENT_WORKFLOW_INVALID";
/// `GENERATION_AGENT_REGISTRATION_INVALID` — registrationId 格式非法。
pub const GENERATION_AGENT_REGISTRATION_INVALID: &str = "GENERATION_AGENT_REGISTRATION_INVALID";
/// `GENERATION_AGENT_REGISTRATION_MISSING` — registrationId 定位不唯一。
pub const GENERATION_AGENT_REGISTRATION_MISSING: &str = "GENERATION_AGENT_REGISTRATION_MISSING";
/// `GENERATION_AGENT_REGISTRATION_REQUIRED` — agent-round 子代缺少
/// registrationId（基线 admitWorkflow :310）。
pub const GENERATION_AGENT_REGISTRATION_REQUIRED: &str = "GENERATION_AGENT_REGISTRATION_REQUIRED";
/// `GENERATION_AGENT_CHILD_AMBIGUOUS` — 幂等定位命中多个已登记子代
/// （基线 admitWorkflow :323）。
pub const GENERATION_AGENT_CHILD_AMBIGUOUS: &str = "GENERATION_AGENT_CHILD_AMBIGUOUS";
/// `GENERATION_AGENT_CHILD_NOT_AUTHORIZED` — 子代 handle 未授权。
pub const GENERATION_AGENT_CHILD_NOT_AUTHORIZED: &str = "GENERATION_AGENT_CHILD_NOT_AUTHORIZED";
/// `GENERATION_AGENT_SOURCE_CHANGED` — 源指纹在效应前后发生变化。
pub const GENERATION_AGENT_SOURCE_CHANGED: &str = "GENERATION_AGENT_SOURCE_CHANGED";
/// `GENERATION_AGENT_CHILD_EFFECT_ROUND_REQUIRED` — 会话轮次缺少 attempt。
pub const GENERATION_AGENT_CHILD_EFFECT_ROUND_REQUIRED: &str =
    "GENERATION_AGENT_CHILD_EFFECT_ROUND_REQUIRED";
/// `GENERATION_AGENT_DOMAIN_EFFECT_REQUIRED` — 域工具 completed 但无域效应。
pub const GENERATION_AGENT_DOMAIN_EFFECT_REQUIRED: &str = "GENERATION_AGENT_DOMAIN_EFFECT_REQUIRED";
/// `GENERATION_AGENT_DOMAIN_TOOL_INVALID` — 域工具提交参数非法
/// （agent-domain-tool-effect 字段白名单 / 类型 / 枚举校验）。
pub const GENERATION_AGENT_DOMAIN_TOOL_INVALID: &str = "GENERATION_AGENT_DOMAIN_TOOL_INVALID";
/// `GENERATION_AGENT_DOMAIN_TOOL_NOT_AUTHORIZED` — 域工具提交未授权
/// （基线 commitDomainTool:385-390 门禁）。
pub const GENERATION_AGENT_DOMAIN_TOOL_NOT_AUTHORIZED: &str =
    "GENERATION_AGENT_DOMAIN_TOOL_NOT_AUTHORIZED";
/// 基线 finishTool:263 逐字为 `'GENERATION_EPOCH_STALE'`（**无 AGENT 前缀**，
/// 与同文件其余码不同；逐字保留）。
pub const GENERATION_EPOCH_STALE: &str = "GENERATION_EPOCH_STALE";
/// `GENERATION_PLANNING_RANGE_INVALID` — 规划章节区间非法（plot-outline-contract）。
pub const GENERATION_PLANNING_RANGE_INVALID: &str = "GENERATION_PLANNING_RANGE_INVALID";
/// `GENERATION_PLANNING_TARGET_INVALID` — 规划目标字数非法（plot-outline-contract）。
pub const GENERATION_PLANNING_TARGET_INVALID: &str = "GENERATION_PLANNING_TARGET_INVALID";

// ── 规划归一化（平移 src/shared/agent-generation.ts:6-19 + plot-outline-contract）──

/// 基线 `DEFAULT_PLANNING_ACTION_CHAPTERS`（src/shared/plot-outline-contract.ts:9）。
pub const DEFAULT_PLANNING_ACTION_CHAPTERS: i64 = 5;
/// 基线 `PLANNING_ACTION_CHAPTER_LIMIT`（src/shared/plot-outline-contract.ts:10）。
const PLANNING_ACTION_CHAPTER_LIMIT: i64 = 10;
/// 基线 `DEFAULT_PLANNING_TARGET_UNITS`（src/shared/plot-outline-contract.ts:12 区段）。
const DEFAULT_PLANNING_TARGET_UNITS: i64 = 600;
/// 基线 `PLANNING_TARGET_UNITS_LIMIT`（src/shared/plot-outline-contract.ts）。
const PLANNING_TARGET_UNITS_LIMIT: i64 = 1000;

/// 基线 `parsePlanningTargetUnits`（src/shared/plot-outline-contract.ts:12）。
fn parse_planning_target_units(value: Option<&Value>) -> Result<i64, String> {
    let Some(value) = value else {
        return Ok(DEFAULT_PLANNING_TARGET_UNITS);
    };
    // 基线 `typeof value !== 'number' || !Number.isSafeInteger(value)`。
    let units = value
        .as_i64()
        .ok_or_else(|| GENERATION_PLANNING_TARGET_INVALID.to_string())?;
    if units < 1 || units > PLANNING_TARGET_UNITS_LIMIT {
        return Err(GENERATION_PLANNING_TARGET_INVALID.to_string());
    }
    Ok(units)
}

/// 基线 `normalizeAgentPlanningArguments`（src/shared/agent-generation.ts:6-19）：
/// 规划参数归一化 + `GENERATION_PLANNING_RANGE_INVALID` /
/// `GENERATION_PLANNING_TARGET_INVALID`。
pub fn normalize_agent_planning_arguments(
    args: &Map<String, Value>,
    total_chapters: i64,
) -> Result<Map<String, Value>, String> {
    let workflow = args
        .get("workflow")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if workflow != "generate_architecture" && workflow != "generate_blueprint" {
        return Ok(args.clone());
    }
    // 基线 `args.start_chapter === undefined ? 1 : args.start_chapter`：
    // 键缺失 → 默认 1；键存在但非安全整数 → 范围非法。
    let from = match args.get("start_chapter") {
        None => 1,
        Some(value) => value
            .as_i64()
            .ok_or_else(|| GENERATION_PLANNING_RANGE_INVALID.to_string())?,
    };
    // 基线 `args.chapter_count === undefined
    //   ? Math.min(DEFAULT_PLANNING_ACTION_CHAPTERS, totalChapters - Number(from) + 1)
    //   : args.chapter_count`。
    let count = match args.get("chapter_count") {
        None => DEFAULT_PLANNING_ACTION_CHAPTERS.min(total_chapters - from + 1),
        Some(value) => value
            .as_i64()
            .ok_or_else(|| GENERATION_PLANNING_RANGE_INVALID.to_string())?,
    };
    // 基线 `typeof from !== 'number' || typeof count !== 'number'
    //   || !Number.isSafeInteger(count) || count < 1`。
    if count < 1 {
        return Err(GENERATION_PLANNING_RANGE_INVALID.to_string());
    }
    let to = from + count - 1;
    // 基线 `assertPlanningActionRange({ from, to })`：from/to 均安全整数、
    // from ≥ 1、to ≥ from、跨度 ≤ PLANNING_ACTION_CHAPTER_LIMIT。
    if from < 1 || to < from || to - from + 1 > PLANNING_ACTION_CHAPTER_LIMIT {
        return Err(GENERATION_PLANNING_RANGE_INVALID.to_string());
    }
    // 基线 `if (to > totalChapters) throw GENERATION_PLANNING_RANGE_INVALID`。
    if to > total_chapters {
        return Err(GENERATION_PLANNING_RANGE_INVALID.to_string());
    }
    let target_units = parse_planning_target_units(args.get("target_units"))?;
    let mut normalized = args.clone();
    normalized.insert("start_chapter".to_string(), Value::from(from));
    normalized.insert("chapter_count".to_string(), Value::from(count));
    normalized.insert("target_units".to_string(), Value::from(target_units));
    Ok(normalized)
}

// ── 共享契约类型镜像（src/shared/agent-generation.ts）──

/// `AgentToolActionRef`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolActionRef {
    pub handle: MainGenerationRunHandle,
    pub attempt_id: String,
    pub tool_call_id: String,
}

/// `AgentToolAction`（`ref` 为 Rust 关键字，序列化名逐字对齐基线 `ref`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolAction {
    #[serde(rename = "ref")]
    pub ref_: AgentToolActionRef,
    pub name: String,
    pub arguments: Map<String, Value>,
    /// 基线六值联合 `'pending' | 'running' | 'completed' | 'declined' | 'failed' | 'unknown'`；
    /// 以 `String` 鸭子类型保真。
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<AgentWorkflowRegistration>,
}

/// `AgentAuthorBlueprintProposal`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentAuthorBlueprintProposal {
    pub name: String,
    pub arguments: Map<String, Value>,
}

/// `AgentToolClaim`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolClaim {
    pub action: AgentToolAction,
    pub execute: bool,
}

/// `AgentDomainToolReceipt`（子波次 C 的 `commit_domain_tool` 返回值；本波仅类型镜像）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDomainToolReceipt {
    pub kind: String,
    pub tool_call_id: String,
    pub changes: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_number: Option<i64>,
    pub before_hash: String,
    pub after_hash: String,
    /// 新鲜读投影；历史 ACK 不得覆盖较新的渲染层编辑。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<bool>,
}

/// `AgentWorkflowRegistration.planning`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorkflowPlanning {
    pub from: i64,
    pub to: i64,
    pub target_units: i64,
}

/// `AgentWorkflowRegistration`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorkflowRegistration {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planning: Option<AgentWorkflowPlanning>,
    pub registration_id: String,
    /// 基线三值联合 `'generate_draft' | 'generate_blueprint' | 'generate_architecture'`。
    pub workflow: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_number: Option<i64>,
    pub parent_handle: MainGenerationRunHandle,
    pub model_id: String,
    /// 基线二值联合 `'registered' | 'started'`。
    pub state: String,
    pub child_handles: Vec<MainGenerationRunHandle>,
}

/// 基线 `admitWorkflow(selection)` 的返回值
/// （`{ existing?: MainGenerationRunHandle; parentRootActionId: string }`，
/// agent-generation.ts:305）——`existing` 命中已登记子代时
/// owner.begin 幂等早退（返回其运行视图），否则携带
/// `parentRootActionId` 供调用方继续准入链。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmittedWorkflow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing: Option<MainGenerationRunHandle>,
    pub parent_root_action_id: String,
}

/// `AgentGenerationRound`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentGenerationRound {
    pub index: i64,
    pub handle: MainGenerationRunHandle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    /// 基线四值联合 `'not-started' | 'completed' | 'incomplete' | 'unknown'`。
    pub status: String,
    pub visible_text: String,
    /// 主进程仅重建已接受的协议；隐藏推理永不包含。
    pub protocol_text: String,
    pub actions: Vec<AgentToolAction>,
}

/// `AgentGenerationRecovery`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentGenerationRecovery {
    pub handle: MainGenerationRunHandle,
    pub model_id: String,
    pub context: AgentGenerationContext,
    pub rounds: Vec<AgentGenerationRound>,
    /// 基线二值联合 `'current' | 'conflict'`。
    pub source_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_round: Option<i64>,
    /// 基线 `run: host.read(...)`（`MainGenerationRunView`）。owner 侧视图
    /// 投影在 Rust 侧为 `main_generation_owner` 的私有类型，本波以松散
    /// `serde_json::Value` 鸭子类型保真（经 Tauri IPC 序列化后形态一致）；
    /// 子波次 B 接线 host 实现时按 `viewOf` 输出构造。
    pub run: Value,
}

/// 已冻结的内部 Agent 轮次请求；不借用数据库、宿主或导航状态。
#[derive(Debug, Clone, PartialEq)]
pub struct AgentRoundExecutionPlan {
    pub handle: MainGenerationRunHandle,
    pub invocation_nonce: String,
    pub task: Value,
}

/// 准备阶段只决定重放或执行；生产派发须另接短数据库回调与关闭围栏。
#[derive(Debug, Clone, PartialEq)]
pub enum PreparedAgentRound {
    Replay(Box<AgentGenerationRecovery>),
    Execute(AgentRoundExecutionPlan),
}

/// 基线 `ActionState`（`updateAction` 更新闭包返回值）：
/// `Pick<AgentToolAction, 'status' | 'observation' | 'workflow'>`
/// 与 `{ domainEffect?, planningArguments? }` 的交集。
#[derive(Debug, Clone, Default)]
pub struct ActionState {
    pub status: String,
    #[allow(dead_code)]
    pub observation: Option<String>,
    #[allow(dead_code)]
    pub workflow: Option<AgentWorkflowRegistration>,
    #[allow(dead_code)]
    pub domain_effect: Option<AgentDomainToolReceipt>,
    #[allow(dead_code)]
    pub planning_arguments: Option<Map<String, Value>>,
}

impl ActionState {
    /// 基线 `return action`（非 pending 幂等分支）：仅透传
    /// `status` / `observation` / `workflow` 三键。
    fn of_action(action: &AgentToolAction) -> Self {
        Self {
            status: action.status.clone(),
            observation: action.observation.clone(),
            workflow: action.workflow.clone(),
            ..Default::default()
        }
    }
}

// ── AgentHost（基线接口，本波仅定义 trait）──

/// 基线 `AgentHost` 接口（electron/services/agent-generation.ts:11-22）——
/// owner 注入的宿主能力。子波次 B 由 `MainGenerationOwner` 注入实现
/// （基线 main-generation-owner.ts:899-906）：
/// - `begin` → owner `begin(selection, false, true)`（agent 初始化路径）
/// - `read` → owner `viewOf(requireRun(handle))`
/// - `resume` → owner `resume(handle)`（LLM 续跑）
/// - `execute` → owner `execute({handle, invocationNonce, task}, true)`
///   （internal=true 绕过 IPC 准入）
/// - `current` → `compareGenerationSourceBindings(expected ?? run.binding, deps.rebuildBinding(...))`
/// - `capture` → `deps.rebuildBinding(run.binding, currentModelReceipt(run.binding))`
///
/// 视图类型说明：基线 `MainGenerationRunView` / `MainGenerationExecuteReceipt`
/// 在 Rust 侧为 owner 私有投影，本波以松散 `serde_json::Value` 鸭子类型保真。
/// `task` 为基线 `GenerationTask`（`{ purpose, output, reasoningStage, messages }`），
/// 同样以 `Value` 保真（子波次 B 接线时可按 `generation_task` 模型强类型化）。
pub trait AgentHost {
    /// 基线 `begin(selection): MainGenerationRunView`。
    fn begin(&self, conn: &Connection, selection: &Value) -> Result<Value, String>;
    /// 基线 `read(handle): MainGenerationRunView`。
    fn read(&self, conn: &Connection, handle: &MainGenerationRunHandle) -> Result<Value, String>;
    /// 基线 `resume(handle): Promise<MainGenerationRunView>`。
    fn resume(&self, conn: &Connection, handle: &MainGenerationRunHandle) -> Result<Value, String>;
    /// 基线 `execute(handle, nonce, task): Promise<MainGenerationExecuteReceipt>`。
    fn execute(
        &self,
        conn: &Connection,
        handle: &MainGenerationRunHandle,
        nonce: &str,
        task: &Value,
    ) -> Result<Value, String>;
    /// 基线 `current(run, expected?): boolean`。
    fn current(
        &self,
        conn: &Connection,
        run: &DurableGenerationRun,
        expected: Option<&RunBinding>,
    ) -> bool;
    /// 基线 `capture(run): RunBinding`。
    fn capture(&self, conn: &Connection, run: &DurableGenerationRun) -> Result<RunBinding, String>;
}

// ── AgentGeneration 状态机 ──────────────────────────────────────────────

/// 基线 `AgentBeginSelection`（`BeginGenerationRequest & { agentInput, agentSession }`）
/// 的松散 `Value` 形态；`selection()` 按基线 :172-179 逐键构造。
/// 会话导航键（`{ key, roundIndex }`）。
#[derive(Debug, Clone)]
struct AgentNavigation {
    key: String,
    round_index: i64,
}

/// 控制纪元写入使用独立保存点，防止外层事务捕获错误后留下半次移交。
struct AgentControlSavepoint<'a> {
    conn: &'a Connection,
    active: bool,
}

impl<'a> AgentControlSavepoint<'a> {
    fn new(conn: &'a Connection) -> Result<Self, String> {
        conn.execute_batch("SAVEPOINT lorekeeper_agent_control_resume")
            .map_err(|error| error.to_string())?;
        Ok(Self { conn, active: true })
    }

    fn release(&mut self) -> Result<(), String> {
        self.conn
            .execute_batch("RELEASE SAVEPOINT lorekeeper_agent_control_resume")
            .map_err(|error| error.to_string())?;
        self.active = false;
        Ok(())
    }
}

impl Drop for AgentControlSavepoint<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self.conn.execute_batch(
                "ROLLBACK TO SAVEPOINT lorekeeper_agent_control_resume; RELEASE SAVEPOINT lorekeeper_agent_control_resume",
            );
        }
    }
}

/// Agent 生成状态机（基线 `AgentGeneration` 类）。
///
/// 导航 / 控制状态全部落在既有 runs 与 attempt 回执之上——无第二预算表、
/// 无第二写入器。一个实例绑定一个 `{projectId, epoch}` 作用域。
pub struct AgentGeneration<'a> {
    conn: &'a Connection,
    runs: GenerationRunRepository,
    project_id: &'a str,
    epoch: &'a str,
    assert_current: &'a dyn Fn() -> Result<(), String>,
    host: &'a dyn AgentHost,
}

impl<'a> AgentGeneration<'a> {
    /// 基线构造器（electron/services/agent-generation.ts:25-43），含构造期
    /// 恢复扫描（`.immediate()` 事务）。
    ///
    /// 恢复扫描语义（基线注释逐字）：渲染层持有的工具可能在旧 owner 关闭前
    /// 越过其提交点；其未完成的认领是**未知结果**，绝不是重跑的许可。
    /// 扫描 `generation_attempts JOIN generation_runs` 中
    /// `binding_json.sourceManifest.operation='agent-round'` 且 projectId
    /// 匹配的行，把 `usage_receipt_json.agentActions` 中 `status='running'`
    /// 的悬空认领修复为 `'completed'`（有 domainEffect）或 `'unknown'`（无）。
    pub fn new(
        conn: &'a Connection,
        runs: GenerationRunRepository,
        project_id: &'a str,
        epoch: &'a str,
        assert_current: &'a dyn Fn() -> Result<(), String>,
        host: &'a dyn AgentHost,
    ) -> Result<Self, String> {
        let agent = Self::without_recovery(conn, runs, project_id, epoch, assert_current, host);
        agent.recover_dangling_claims()?;
        Ok(agent)
    }

    /// owner 已完成启动恢复时只重建借用视图，不触碰当前仍在运行的工具认领。
    pub(crate) fn without_recovery(
        conn: &'a Connection,
        runs: GenerationRunRepository,
        project_id: &'a str,
        epoch: &'a str,
        assert_current: &'a dyn Fn() -> Result<(), String>,
        host: &'a dyn AgentHost,
    ) -> Self {
        Self {
            conn,
            runs,
            project_id,
            epoch,
            assert_current,
            host,
        }
    }

    /// 构造期恢复扫描（基线 :31-43）。
    fn recover_dangling_claims(&self) -> Result<(), String> {
        if self.conn.is_autocommit() {
            // 无环境事务：自开事务（基线 `.immediate()`）。
            let tx = self
                .conn
                .unchecked_transaction()
                .map_err(|error| error.to_string())?;
            let outcome = self.recover_dangling_claims_in_transaction();
            match outcome {
                Ok(()) => tx.commit().map(|_| ()).map_err(|error| error.to_string()),
                // 错误时 tx drop 即回滚，与基线 `db.transaction()` 语义一致。
                Err(error) => Err(error),
            }
        } else {
            // 基线 better-sqlite3 嵌套事务自动降级为 SAVEPOINT；
            // rusqlite 无嵌套事务——直接复用外层环境事务。
            self.recover_dangling_claims_in_transaction()
        }
    }

    fn recover_dangling_claims_in_transaction(&self) -> Result<(), String> {
        let rows: Vec<(String, Option<String>)> = self
            .conn
            .prepare(
                "SELECT a.attempt_id,a.usage_receipt_json FROM generation_attempts a JOIN generation_runs r ON r.run_id=a.run_id WHERE json_extract(r.binding_json,'$.projectId')=? AND json_extract(r.binding_json,'$.sourceManifest.operation')='agent-round'",
            )
            .map_err(|error| error.to_string())?
            .query_map([self.project_id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|error| error.to_string())?;
        for (attempt_id, usage_json) in rows {
            // 基线 `JSON.parse(row.usage_receipt_json ?? '{}')`。
            let mut usage: Value = serde_json::from_str(usage_json.as_deref().unwrap_or("{}"))
                .map_err(|error| error.to_string())?;
            let Some(agent_actions) = usage.get_mut("agentActions") else {
                continue;
            };
            let Some(agent_actions) = agent_actions.as_object_mut() else {
                continue;
            };
            let mut changed = false;
            for state in agent_actions.values_mut() {
                let Some(state) = state.as_object_mut() else {
                    continue;
                };
                if state.get("status").and_then(Value::as_str) == Some("running") {
                    let has_effect = state.contains_key("domainEffect");
                    state.insert(
                        "status".to_string(),
                        Value::String(if has_effect { "completed" } else { "unknown" }.to_string()),
                    );
                    if has_effect {
                        state.insert(
                            "observation".to_string(),
                            Value::String(
                                "The confirmed application change was committed.".to_string(),
                            ),
                        );
                    }
                    changed = true;
                }
            }
            if changed {
                let usage_string =
                    serde_json::to_string(&usage).map_err(|error| error.to_string())?;
                self.conn
                    .execute(
                        "UPDATE generation_attempts SET usage_receipt_json=? WHERE attempt_id=?",
                        [usage_string, attempt_id],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    // ── 身份门禁链（基线 :62-86）───────────────────────────────────────

    /// 基线 `handle(run)`。
    fn handle(&self, run: &DurableGenerationRun) -> MainGenerationRunHandle {
        MainGenerationRunHandle {
            project_id: run.binding.project_id.clone(),
            epoch: run.binding.epoch.clone(),
            root_action_id: run.root_action_id.clone(),
            run_id: run.run_id.clone(),
        }
    }

    /// 基线 `requireRun(handle)`（:62-70）：handle 四字段 deep-equal +
    /// projectId 匹配 + `operation='agent-round'`，否则
    /// `GENERATION_AGENT_IDENTITY_MISMATCH`；随后校验 agentContext。
    pub fn require_run(&self, handle: &Value) -> Result<DurableGenerationRun, String> {
        (self.assert_current)()?;
        // 基线 `this.runs.get(handle.runId)` 先于 deep-equal：畸形 handle
        // （非对象 / 键集不精确）在 deep-equal 阶段即失败。
        let valid_handle = handle.is_object()
            && handle
                .as_object()
                .map(|record| {
                    record.keys().all(|key| {
                        matches!(
                            key.as_str(),
                            "projectId" | "epoch" | "rootActionId" | "runId"
                        )
                    })
                })
                .unwrap_or(false);
        if !valid_handle {
            return Err(GENERATION_AGENT_IDENTITY_MISMATCH.to_string());
        }
        let handle_value: MainGenerationRunHandle = serde_json::from_value(handle.clone())
            .map_err(|_| GENERATION_AGENT_IDENTITY_MISMATCH.to_string())?;
        let run = self.runs.get(self.conn, &handle_value.run_id)?;
        if !is_deep_strict_equal(&self.handle(&run), &handle_value)
            || run.binding.project_id != self.project_id
            || run
                .binding
                .source_manifest
                .get("operation")
                .and_then(Value::as_str)
                != Some("agent-round")
        {
            return Err(GENERATION_AGENT_IDENTITY_MISMATCH.to_string());
        }
        self.context(&run)?;
        Ok(run)
    }

    /// 基线 `context(run)`（:72-77）：agentContext 存在、`version === 1`
    /// 且 `textHash(JSON.stringify(context)) === agentContextHash`，
    /// 否则 `GENERATION_AGENT_CONTEXT_INVALID`。
    ///
    /// 哈希口径说明：`agentContextHash` 由 `generation_source_binding` 以
    /// **typed 序列化**（结构体声明序）计算；此处先反序列化为
    /// `AgentGenerationContext` 再序列化求哈希，与写入侧口径逐字一致
    /// （`serde_json::Value` 的 BTreeMap 键序与声明序不同，不能直接哈希）。
    fn context(&self, run: &DurableGenerationRun) -> Result<AgentGenerationContext, String> {
        let context_value = run
            .binding
            .source_manifest
            .get("agentContext")
            .ok_or_else(|| GENERATION_AGENT_CONTEXT_INVALID.to_string())?;
        let context: AgentGenerationContext = serde_json::from_value(context_value.clone())
            .map_err(|_| GENERATION_AGENT_CONTEXT_INVALID.to_string())?;
        let context_string = serde_json::to_string(&context)
            .map_err(|_| GENERATION_AGENT_CONTEXT_INVALID.to_string())?;
        if context.version != 1
            || text_hash(&context_string)
                != run
                    .binding
                    .source_manifest
                    .get("agentContextHash")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
        {
            return Err(GENERATION_AGENT_CONTEXT_INVALID.to_string());
        }
        Ok(context)
    }

    /// 基线 `navigation(run)`（:79-86）：agentSession.key 为 64 位小写 hex、
    /// roundIndex 为 0..7 安全整数，否则 `GENERATION_AGENT_NAVIGATION_INVALID`。
    fn navigation(&self, run: &DurableGenerationRun) -> Result<AgentNavigation, String> {
        let value = run
            .binding
            .source_manifest
            .get("agentSession")
            .ok_or_else(|| GENERATION_AGENT_NAVIGATION_INVALID.to_string())?;
        let key = value
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| GENERATION_AGENT_NAVIGATION_INVALID.to_string())?;
        // 基线 `Number.isSafeInteger(value.roundIndex)`：键缺失或非整数 → 非法。
        let round_index = match value.get("roundIndex") {
            Some(number) if number.is_i64() => number.as_i64().unwrap_or_default(),
            _ => return Err(GENERATION_AGENT_NAVIGATION_INVALID.to_string()),
        };
        if !is_session_key(key) || round_index < 0 || round_index >= 8 {
            return Err(GENERATION_AGENT_NAVIGATION_INVALID.to_string());
        }
        Ok(AgentNavigation {
            key: key.to_string(),
            round_index,
        })
    }

    /// 基线 `sessionRuns(key)`（:88-95）——「runs registry」等价物：
    /// `json_extract(binding_json,'$.sourceManifest.agentSession.key')=?`
    /// 全表扫 generation_runs，按 projectId 过滤、按 roundIndex 排序。
    fn session_runs(&self, key: &str) -> Result<Vec<DurableGenerationRun>, String> {
        let rows: Vec<String> = self
            .conn
            .prepare(
                "SELECT run_id FROM generation_runs WHERE json_extract(binding_json,'$.sourceManifest.agentSession.key')=? ORDER BY rowid",
            )
            .map_err(|error| error.to_string())?
            .query_map([key], |row| row.get(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|error| error.to_string())?;
        // 基线 `.map(row => this.runs.get(row.run_id))`——缺失 run 抛
        // `GENERATION_RUN_MISSING`（原样传播）；navigation 在排序键提取时
        // 同样传播（基线在比较器内抛错）。
        let mut records: Vec<(i64, DurableGenerationRun)> = rows
            .iter()
            .map(|run_id| self.runs.get(self.conn, run_id))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|run| run.binding.project_id == self.project_id)
            .map(|run| {
                self.navigation(&run)
                    .map(|navigation| (navigation.round_index, run))
            })
            .collect::<Result<Vec<_>, _>>()?;
        records.sort_by_key(|(round_index, _)| *round_index);
        Ok(records.into_iter().map(|(_, run)| run).collect())
    }

    /// 基线 `modelId(run)`（:97）：`sourceManifest.modelReceipt.modelId`。
    fn model_id(&self, run: &DurableGenerationRun) -> Result<String, String> {
        run.binding
            .source_manifest
            .get("modelReceipt")
            .and_then(|receipt| receipt.get("modelId"))
            .and_then(Value::as_str)
            .map(str::to_string)
            // 基线对缺失 modelReceipt 抛 TypeError（折叠为
            // GENERATION_REQUEST_FAILED）；此处以描述性字符串等价折叠。
            .ok_or_else(|| "agent model receipt missing".to_string())
    }

    /// 基线 `actionId(attemptId, index)`（:98）：`agent-tool:${attemptId}:${index}`。
    fn action_id(attempt_id: &str, index: usize) -> String {
        format!("agent-tool:{attempt_id}:{index}")
    }

    /// 基线 `sourcesCurrent(run)`（:101-106）：取该 run 最新 attempt 的
    /// `agentSourceTransition`（按 rowid 倒序首个非空），校验
    /// `textHash(JSON.stringify(binding)) === afterHash`（否则
    /// `GENERATION_AGENT_EFFECT_BINDING_INVALID`），再委托
    /// `host.current(run, transition?.binding)`。
    fn sources_current(&self, run: &DurableGenerationRun) -> Result<bool, String> {
        let rows: Vec<Option<String>> = self
            .conn
            .prepare(
                "SELECT usage_receipt_json FROM generation_attempts WHERE run_id=? ORDER BY rowid DESC",
            )
            .map_err(|error| error.to_string())?
            .query_map([&run.run_id], |row| row.get(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|error| error.to_string())?;
        // 基线 `.find(Boolean)`——倒序扫描的首个非空 transition。
        let mut transition: Option<Value> = None;
        for row in rows {
            let usage: Value = serde_json::from_str(row.as_deref().unwrap_or("{}"))
                .map_err(|error| error.to_string())?;
            if let Some(candidate) = usage.get("agentSourceTransition") {
                if !candidate.is_null() {
                    transition = Some(candidate.clone());
                    break;
                }
            }
        }
        let expected = match &transition {
            Some(transition) => {
                let binding_string =
                    serde_json::to_string(transition.get("binding").unwrap_or(&Value::Null))
                        .map_err(|error| error.to_string())?;
                if text_hash(&binding_string)
                    != transition
                        .get("afterHash")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                {
                    return Err(GENERATION_AGENT_EFFECT_BINDING_INVALID.to_string());
                }
                serde_json::from_value(transition.get("binding").cloned().unwrap_or(Value::Null))
                    .map_err(|_| GENERATION_AGENT_EFFECT_BINDING_INVALID.to_string())?
            }
            None => None,
        };
        Ok(self.host.current(self.conn, run, expected.as_ref()))
    }

    // ── 轮次重建（基线 :83-129）────────────────────────────────────────

    /// 基线 `round(run)`（:83-129）：单 attempt 约束 → 无 attempt 空轮 →
    /// attempt 回执解析（complete 判定）→ toolCalls → actions
    /// （`toolCallId = 'agent-tool:${attemptId}:${callIndex}'`）。
    fn round(&self, run: &DurableGenerationRun) -> Result<AgentGenerationRound, String> {
        let index = self.navigation(run)?.round_index;
        let rows: Vec<(String, Option<String>)> = self
            .conn
            .prepare(
                "SELECT attempt_id,usage_receipt_json FROM generation_attempts WHERE run_id=? ORDER BY rowid",
            )
            .map_err(|error| error.to_string())?
            .query_map([&run.run_id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|error| error.to_string())?;
        if rows.len() > 1 {
            return Err(GENERATION_AGENT_ROUND_AMBIGUOUS.to_string());
        }
        let Some((attempt_id, usage_json)) = rows.into_iter().next() else {
            // 基线 :86——无 attempt 的 not-started 空轮。
            return Ok(AgentGenerationRound {
                index,
                handle: self.handle(run),
                attempt_id: None,
                status: "not-started".to_string(),
                visible_text: String::new(),
                protocol_text: String::new(),
                actions: Vec::new(),
            });
        };
        let receipt = self.runs.receipt(self.conn, &attempt_id, None)?;
        // 基线 `JSON.parse(row.usage_receipt_json ?? '{}')`——松散读取，
        // 保留全部既有键（purpose / result 等）回写。
        let mut usage: Value = serde_json::from_str(usage_json.as_deref().unwrap_or("{}"))
            .map_err(|error| error.to_string())?;
        let response = receipt
            .result
            .as_ref()
            .and_then(|result| result.usage.as_ref())
            .and_then(|result_usage| result_usage.agent_response.as_ref());
        // 基线 :99-102——complete = agentResponse v1 + finishReason==='stop'
        // + attempt.status ∈ {settled, unknown} + artifact.text === visibleText。
        let complete = match response {
            Some(response) => {
                response.version == 1
                    && receipt
                        .result
                        .as_ref()
                        .and_then(|result| result.finish_reason.as_deref())
                        == Some("stop")
                    && matches!(receipt.attempt.status.as_str(), "settled" | "unknown")
                    && receipt
                        .artifact
                        .as_ref()
                        .is_some_and(|artifact| artifact.text == response.visible_text)
            }
            None => false,
        };
        if !complete {
            // 基线 :103-106——未 complete：attempt 状态为 unknown /
            // dispatch-marked / reserved → 'unknown' 轮，否则 'incomplete'。
            return Ok(AgentGenerationRound {
                index,
                handle: self.handle(run),
                attempt_id: Some(attempt_id),
                status: if matches!(
                    receipt.attempt.status.as_str(),
                    "unknown" | "dispatch-marked" | "reserved"
                ) {
                    "unknown".to_string()
                } else {
                    "incomplete".to_string()
                },
                visible_text: receipt
                    .artifact
                    .map(|artifact| artifact.text)
                    .unwrap_or_default(),
                protocol_text: String::new(),
                actions: Vec::new(),
            });
        }
        let response = response.expect("complete ⇒ agentResponse 存在");
        // toolCalls → actions；start_workflow 规划参数按需归一化
        // （基线 :107-121）。
        let mut normalized = false;
        let mut actions: Vec<AgentToolAction> = Vec::new();
        for (call_index, call) in response.tool_calls.iter().enumerate() {
            let tool_call_id = Self::action_id(&attempt_id, call_index);
            let mut state: Option<Value> = usage
                .get("agentActions")
                .and_then(|agent_actions| agent_actions.get(&tool_call_id))
                .cloned();
            if call.name == "start_workflow"
                && matches!(
                    call.arguments.get("workflow").and_then(Value::as_str),
                    Some("generate_architecture") | Some("generate_blueprint")
                )
                && state
                    .as_ref()
                    .and_then(|state| state.get("planningArguments"))
                    .is_none()
                && state
                    .as_ref()
                    .and_then(|state| state.get("workflow"))
                    .is_none()
                && (state.is_none()
                    || state
                        .as_ref()
                        .and_then(|state| state.get("status"))
                        .and_then(Value::as_str)
                        == Some("pending"))
            {
                // 基线 :111——`SELECT total_chapters FROM project_core WHERE id='main'`。
                let total_chapters: i64 = self
                    .conn
                    .query_row(
                        "SELECT total_chapters FROM project_core WHERE id='main'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(|error| error.to_string())?;
                match normalize_agent_planning_arguments(&call.arguments, total_chapters) {
                    Ok(planning_arguments) => {
                        // 基线 :113——`{ ...state, status: state?.status ?? 'pending', planningArguments }`。
                        let mut next: Map<String, Value> = state
                            .as_ref()
                            .and_then(Value::as_object)
                            .cloned()
                            .unwrap_or_default();
                        next.insert(
                            "status".to_string(),
                            state
                                .as_ref()
                                .and_then(|state| state.get("status"))
                                .cloned()
                                .unwrap_or_else(|| Value::String("pending".to_string())),
                        );
                        next.insert(
                            "planningArguments".to_string(),
                            Value::Object(planning_arguments),
                        );
                        state = Some(Value::Object(next));
                    }
                    Err(error)
                        if error == GENERATION_PLANNING_RANGE_INVALID
                            || error == GENERATION_PLANNING_TARGET_INVALID =>
                    {
                        // 基线 :115-117——规划参数非法 → failed + 错误码观察。
                        state = Some(json!({
                            "status": "failed",
                            "observation": error,
                        }));
                    }
                    Err(error) => return Err(error),
                }
                // 基线 :119——`usage.agentActions = { ...usage.agentActions, [toolCallId]: state }`。
                let mut agent_actions: Map<String, Value> = usage
                    .get("agentActions")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                agent_actions.insert(tool_call_id.clone(), state.clone().unwrap_or(Value::Null));
                if let Some(map) = usage.as_object_mut() {
                    map.insert("agentActions".to_string(), Value::Object(agent_actions));
                }
                normalized = true;
            }
            // 基线 :123-127——`arguments: structuredClone(state?.planningArguments ?? call.arguments)`。
            let arguments = state
                .as_ref()
                .and_then(|state| state.get("planningArguments"))
                .cloned()
                .unwrap_or_else(|| Value::Object(call.arguments.clone()));
            actions.push(AgentToolAction {
                ref_: AgentToolActionRef {
                    handle: self.handle(run),
                    attempt_id: attempt_id.clone(),
                    tool_call_id: tool_call_id.clone(),
                },
                name: call.name.clone(),
                arguments: arguments.as_object().cloned().unwrap_or_default(),
                status: state
                    .as_ref()
                    .and_then(|state| state.get("status"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| "pending".to_string()),
                observation: state
                    .as_ref()
                    .and_then(|state| state.get("observation"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                workflow: state
                    .as_ref()
                    .and_then(|state| state.get("workflow"))
                    .cloned()
                    .and_then(|workflow| serde_json::from_value(workflow).ok()),
            });
        }
        if normalized {
            let usage_string = serde_json::to_string(&usage).map_err(|error| error.to_string())?;
            self.conn
                .execute(
                    "UPDATE generation_attempts SET usage_receipt_json=? WHERE attempt_id=?",
                    [usage_string, attempt_id.clone()],
                )
                .map_err(|error| error.to_string())?;
        }
        // 基线 :124-129——propose_novel_config completed 且 domainEffect.kind==='config'
        // 时，按 agentAuthorSelections 插入作者跟进动作
        // （toolCallId 后缀 `:author:${selectionIndex}`）。
        let mut action_index = 0usize;
        while action_index < actions.len() {
            let parent = actions[action_index].clone();
            let parent_has_config_effect = usage
                .get("agentActions")
                .and_then(|agent_actions| agent_actions.get(parent.ref_.tool_call_id.as_str()))
                .and_then(|state| state.get("domainEffect"))
                .and_then(|effect| effect.get("kind"))
                .and_then(Value::as_str)
                == Some("config");
            if parent.name != "propose_novel_config"
                || parent.status != "completed"
                || !parent_has_config_effect
            {
                action_index += 1;
                continue;
            }
            // 基线 `usage.agentAuthorSelections?.[toolCallId] ?? []`——
            // 损坏态（非数组）按空处理（写入侧由 claimTool 保证数组形态）。
            let selected: Vec<Value> = usage
                .get("agentAuthorSelections")
                .and_then(|selections| selections.get(parent.ref_.tool_call_id.as_str()))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let followups: Vec<AgentToolAction> = selected
                .iter()
                .enumerate()
                .filter_map(|(selection_index, proposal)| {
                    let tool_call_id =
                        format!("{}:author:{}", parent.ref_.tool_call_id, selection_index);
                    let state = usage
                        .get("agentActions")
                        .and_then(|agent_actions| agent_actions.get(tool_call_id.as_str()));
                    Some(AgentToolAction {
                        ref_: AgentToolActionRef {
                            handle: parent.ref_.handle.clone(),
                            attempt_id: parent.ref_.attempt_id.clone(),
                            tool_call_id,
                        },
                        name: proposal
                            .get("name")
                            .and_then(Value::as_str)
                            .map(str::to_string)?,
                        arguments: proposal
                            .get("arguments")
                            .and_then(Value::as_object)
                            .cloned()
                            .unwrap_or_default(),
                        status: state
                            .and_then(|state| state.get("status"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_else(|| "pending".to_string()),
                        observation: state
                            .and_then(|state| state.get("observation"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        workflow: None,
                    })
                })
                .collect();
            // 基线 `actions.splice(index + 1, 0, ...followups); index += followups.length;`
            // （for 循环再 +1）。
            let followup_count = followups.len();
            let mut tail = actions.split_off(action_index + 1);
            actions.extend(followups);
            actions.append(&mut tail);
            action_index += 1 + followup_count;
        }
        // 基线 :128-129——`protocolText = [visibleText, ...toolCalls.map(JSON.stringify)].filter(Boolean).join('\n')`。
        let mut protocol_parts: Vec<String> = vec![response.visible_text.clone()];
        protocol_parts.extend(
            response
                .tool_calls
                .iter()
                .map(|call| serde_json::to_string(call).unwrap_or_default()),
        );
        let protocol_text = protocol_parts
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(AgentGenerationRound {
            index,
            handle: self.handle(run),
            attempt_id: Some(attempt_id),
            status: "completed".to_string(),
            visible_text: response.visible_text.clone(),
            protocol_text,
            actions,
        })
    }

    // ── 读面（基线 :131-139）───────────────────────────────────────────

    /// 基线 `read(handle)`（:131-139）：requireRun → sessionRuns →
    /// rootActionId 全一致（`GENERATION_AGENT_ROOT_MISMATCH`）→ 轮次 index
    /// 连续（`GENERATION_AGENT_ROUND_GAP`）→ 每轮 round() 重建 → nextRound
    /// 计算 → 返回 recovery（含 `sourceStatus` 与 `host.read(...)` 视图）。
    pub fn read(&self, handle: &Value) -> Result<AgentGenerationRecovery, String> {
        let selected = self.require_run(handle)?;
        let key = self.navigation(&selected)?.key;
        let records = self.session_runs(&key)?;
        if records.is_empty()
            || records
                .first()
                .is_some_and(|run| run.root_action_id != selected.root_action_id)
            || records
                .iter()
                .any(|run| run.root_action_id != selected.root_action_id)
        {
            return Err(GENERATION_AGENT_ROOT_MISMATCH.to_string());
        }
        let rounds: Vec<AgentGenerationRound> = records
            .iter()
            .map(|run| self.round(run))
            .collect::<Result<_, _>>()?;
        let latest = records.last().expect("session runs 非空");
        let last = rounds.last().expect("轮次非空");
        if rounds
            .iter()
            .enumerate()
            .any(|(index, round)| round.index != index as i64)
        {
            return Err(GENERATION_AGENT_ROUND_GAP.to_string());
        }
        // 基线 :136-137——nextRound：末轮 not-started → 其 index；末轮
        // completed 且全部动作终态且 index<7 → index+1；否则 null。
        let all_done = !last.actions.is_empty()
            && last.actions.iter().all(|action| {
                matches!(action.status.as_str(), "completed" | "failed" | "declined")
            });
        let next_round = if last.status == "not-started" {
            Some(last.index)
        } else if last.status == "completed" && all_done && last.index < 7 {
            Some(last.index + 1)
        } else {
            None
        };
        let recovery_handle = self.handle(latest);
        Ok(AgentGenerationRecovery {
            handle: recovery_handle.clone(),
            model_id: self.model_id(&records[0])?,
            context: self.context(&records[0])?,
            rounds,
            source_status: if self.sources_current(latest)? {
                "current".to_string()
            } else {
                "conflict".to_string()
            },
            next_round,
            run: self.host.read(self.conn, &recovery_handle)?,
        })
    }

    // ── begin（基线 :141-154）──────────────────────────────────────────

    /// 基线 `begin(request)`（:141-154）：幂等恢复入口——按 nonce 派生
    /// session key，命中且 context.input / modelId 一致则返回既有
    /// recovery，否则开第 0 轮 run。
    pub fn begin(&self, request: &Value) -> Result<AgentGenerationRecovery, String> {
        (self.assert_current)()?;
        // 基线 :143-147——请求键集精确 {uiActionNonce, modelId, input}、
        // nonce 非空（trim 后）≤256 字符、modelId 非空。
        let valid_request = request.is_object()
            && request
                .as_object()
                .map(|record| {
                    record
                        .keys()
                        .all(|key| matches!(key.as_str(), "uiActionNonce" | "modelId" | "input"))
                })
                .unwrap_or(false);
        let ui_action_nonce = request
            .get("uiActionNonce")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let model_id = request
            .get("modelId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !valid_request
            || ui_action_nonce.trim().is_empty()
            || ui_action_nonce.chars().count() > 256
            || model_id.trim().is_empty()
        {
            return Err(GENERATION_AGENT_BEGIN_INVALID.to_string());
        }
        // 基线 :149——`validateAgentGenerationInput`（子波次 A 前置已移植，
        // 错误码 `GENERATION_AGENT_INPUT_INVALID`）。
        let input = validate_agent_generation_input(request.get("input").unwrap_or(&Value::Null))?;
        // 基线 :150——`key = textHash(JSON.stringify([projectId, uiActionNonce]))`。
        let key = text_hash(
            &serde_json::to_string(&Value::Array(vec![
                Value::String(self.project_id.to_string()),
                Value::String(ui_action_nonce.to_string()),
            ]))
            .map_err(|error| error.to_string())?,
        );
        let existing = self.session_runs(&key)?;
        if let Some(first) = existing.first() {
            // 基线 :151-153——既有 session：context.input 必须 deep-equal
            // 且 modelId 一致，否则 `GENERATION_AGENT_NONCE_CONFLICT`；
            // 命中即 read 返回。
            if !is_deep_strict_equal(&self.context(first)?.input, &input)
                || self.model_id(first)? != model_id
            {
                return Err(GENERATION_AGENT_NONCE_CONFLICT.to_string());
            }
            return self.read(
                &serde_json::to_value(self.handle(first)).map_err(|error| error.to_string())?,
            );
        }
        // 基线 :154-155——否则经 host.begin 开第 0 轮 run 并 read。
        let view = self
            .host
            .begin(self.conn, &self.selection(&input, &key, 0, model_id, None))?;
        let view_handle = view.get("handle").cloned().unwrap_or(Value::Null);
        self.read(&view_handle)
    }

    /// 基线 `selection(input, key, roundIndex, modelId, parentRootActionId?)`
    /// （:172-179）——host.begin 的 agent 初始化 selection 形状。
    fn selection(
        &self,
        input: &AgentGenerationInput,
        key: &str,
        round_index: i64,
        model_id: &str,
        parent_root_action_id: Option<&str>,
    ) -> Value {
        let mut selection = json!({
            "operation": "agent-round",
            "uiActionNonce": format!("agent:{key}:{round_index}"),
            "modelId": model_id,
            "promptKeys": ["assistant_writing_identity"],
            "skillStages": ["planning", "drafting", "review", "refinement"],
            "selectedDraftIds": [],
            "selectedFinalizedDraftIds": [],
            "output": "visible-text",
            "agentInput": serde_json::to_value(input).unwrap_or_default(),
            "agentSession": { "key": key, "roundIndex": round_index },
        });
        if let Some(parent_root_action_id) = parent_root_action_id {
            selection["parentRootActionId"] = Value::String(parent_root_action_id.to_string());
        }
        selection
    }

    // ── 工具动作定位与更新（基线 :205-229）─────────────────────────────

    /// 基线 `action(ref)`（:205-211）：requireRun + round().actions 精确定位
    /// ref（attemptId + toolCallId），否则 `GENERATION_AGENT_TOOL_IDENTITY_MISMATCH`。
    fn action(&self, ref_: &Value) -> Result<(DurableGenerationRun, AgentToolAction), String> {
        // 基线直接访问 `ref.handle` / `ref.attemptId` / `ref.toolCallId`
        // （畸形 ref 在 TS 侧抛 TypeError）；此处按基线访问顺序：
        // 先 requireRun(ref.handle)（身份门禁），再精确定位。
        let handle = ref_.get("handle").cloned().unwrap_or(Value::Null);
        let run = self.require_run(&handle)?;
        let attempt_id = ref_
            .get("attemptId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let tool_call_id = ref_
            .get("toolCallId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let action = self
            .round(&run)?
            .actions
            .into_iter()
            .find(|action| {
                action.ref_.attempt_id == attempt_id && action.ref_.tool_call_id == tool_call_id
            })
            .ok_or_else(|| GENERATION_AGENT_TOOL_IDENTITY_MISMATCH.to_string())?;
        Ok((run, action))
    }

    /// 基线 `updateAction(ref, update)`（:224-229）：`.immediate()` 事务内
    /// 定位 action → 读 attempt `usage_receipt_json` → 合并 next 状态
    /// （status 恒写；observation / workflow 仅当闭包给出时合并）→
    /// deep-equal 短路 → 写回 → 重新定位返回。
    fn update_action<F>(&self, ref_: &Value, update: F) -> Result<AgentToolAction, String>
    where
        F: FnOnce(
            &AgentToolAction,
            &DurableGenerationRun,
            &Connection,
        ) -> Result<ActionState, String>,
    {
        if self.conn.is_autocommit() {
            // 无环境事务：自开事务（基线 `.immediate()`）。
            let tx = self
                .conn
                .unchecked_transaction()
                .map_err(|error| error.to_string())?;
            let outcome = self.update_action_in_transaction(ref_, update);
            match outcome {
                Ok(action) => tx
                    .commit()
                    .map(|_| action)
                    .map_err(|error| error.to_string()),
                // 错误时 tx drop 即回滚，与基线 `db.transaction()` 语义一致。
                Err(error) => Err(error),
            }
        } else {
            // 基线 better-sqlite3 嵌套事务自动降级为 SAVEPOINT；
            // rusqlite 无嵌套事务——直接复用外层环境事务
            // （对齐 generation_run_repository 头注约定）。
            self.update_action_in_transaction(ref_, update)
        }
    }

    fn update_action_in_transaction<F>(
        &self,
        ref_: &Value,
        update: F,
    ) -> Result<AgentToolAction, String>
    where
        F: FnOnce(
            &AgentToolAction,
            &DurableGenerationRun,
            &Connection,
        ) -> Result<ActionState, String>,
    {
        let (run, action) = self.action(ref_)?;
        let state = update(&action, &run, self.conn)?;
        let usage_json: Option<String> = self
            .conn
            .query_row(
                "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id=? AND run_id=?",
                [ref_.get("attemptId").and_then(Value::as_str).unwrap_or_default(), run.run_id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        // 基线 `JSON.parse(row.usage_receipt_json)`——行缺失或回执为
        // null / 非对象时抛 TypeError（折叠为 GENERATION_REQUEST_FAILED）。
        let Some(usage_json) = usage_json else {
            return Err("agent tool action attempt missing".to_string());
        };
        let mut usage: Value =
            serde_json::from_str(&usage_json).map_err(|error| error.to_string())?;
        let usage_object = usage
            .as_object_mut()
            .ok_or_else(|| "agent usage receipt is not an object".to_string())?;
        let tool_call_id = ref_
            .get("toolCallId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let prior = usage_object
            .get("agentActions")
            .and_then(|agent_actions| agent_actions.get(&tool_call_id))
            .cloned();
        // 基线 :226-228——`next = { ...prior, status, ...(observation), ...(workflow) }`。
        let mut next: Map<String, Value> = prior
            .as_ref()
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        next.insert("status".to_string(), Value::String(state.status.clone()));
        if let Some(observation) = &state.observation {
            next.insert(
                "observation".to_string(),
                Value::String(observation.clone()),
            );
        }
        if let Some(workflow) = &state.workflow {
            next.insert(
                "workflow".to_string(),
                serde_json::to_value(workflow).map_err(|error| error.to_string())?,
            );
        }
        let next_value = Value::Object(next);
        // 基线 :228——deep-equal 短路：状态未变化时不写回。
        if !is_deep_strict_equal(&prior.unwrap_or(Value::Null), &next_value) {
            let mut agent_actions: Map<String, Value> = usage_object
                .get("agentActions")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            agent_actions.insert(tool_call_id.clone(), next_value);
            usage_object.insert("agentActions".to_string(), Value::Object(agent_actions));
            let usage_string = serde_json::to_string(&usage).map_err(|error| error.to_string())?;
            self.conn
                .execute(
                    "UPDATE generation_attempts SET usage_receipt_json=? WHERE attempt_id=?",
                    [
                        usage_string,
                        ref_.get("attemptId")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    ],
                )
                .map_err(|error| error.to_string())?;
        }
        // 基线 :229——重新定位返回（事务内重建）。
        self.action(ref_).map(|(_, action)| action)
    }

    // ── 认领面（基线 :230-274）─────────────────────────────────────────

    /// 基线 `claimTool(request)`（:230-257）：作者认领工具调用——
    /// pending→running（confirmed）或 declined；返回 `{ action, execute }`
    /// （renderer 据 `execute` 决定是否真执行工具）。
    pub fn claim_tool(&self, request: &Value) -> Result<AgentToolClaim, String> {
        // 基线 :231——`typeof request.confirmed !== 'boolean'` 运行时校验。
        let Some(confirmed) = request.get("confirmed").and_then(Value::as_bool) else {
            return Err(GENERATION_AGENT_CONFIRMATION_REQUIRED.to_string());
        };
        let mut execute = false;
        let action = self.update_action(
            request.get("ref").unwrap_or(&Value::Null),
            |action, run, conn| {
                if request.get("authorBlueprintProposals").is_some() {
                    // 基线 :234-243——作者蓝图提案分支：仅 propose_novel_config
                    // + confirmed + 数组 ≤100 + 每项键集精确
                    // {name:'propose_chapter_blueprint', arguments} +
                    // JSON ≤256KiB + 上下文输入工具名单含
                    // builtin / requiresConfirmation / !isReadOnly 的
                    // propose_chapter_blueprint。
                    let selections = request.get("authorBlueprintProposals").unwrap();
                    let Some(selections) = selections.as_array() else {
                        return Err(GENERATION_AGENT_AUTHOR_SELECTION_INVALID.to_string());
                    };
                    let selections_valid = action.name == "propose_novel_config"
                        && confirmed
                        && selections.len() <= 100
                        && selections.iter().all(|item| {
                            item.get("name").and_then(Value::as_str)
                                == Some("propose_chapter_blueprint")
                                && item
                                    .as_object()
                                    .map(|record| {
                                        record
                                            .keys()
                                            .all(|key| matches!(key.as_str(), "name" | "arguments"))
                                    })
                                    .unwrap_or(false)
                                && item
                                    .get("arguments")
                                    .is_some_and(|arguments| arguments.is_object())
                        })
                        && serde_json::to_string(selections)
                            .map(|serialized| serialized.len() <= 256 * 1024)
                            .unwrap_or(false)
                        && self.context(run)?.input.tools.iter().any(|tool| {
                            tool.name == "propose_chapter_blueprint"
                                && matches!(tool.source, AgentToolSource::Builtin)
                                && tool.requires_confirmation
                                && !tool.is_read_only
                        });
                    if !selections_valid {
                        return Err(GENERATION_AGENT_AUTHOR_SELECTION_INVALID.to_string());
                    }
                    // 基线 :244-250——持久化 selections（事务内写回
                    // usage_receipt_json.agentAuthorSelections）。
                    let usage_json: String = conn
                        .query_row(
                            "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id=?",
                            [ref_attempt_id(request)],
                            |row| row.get(0),
                        )
                        .map_err(|error| error.to_string())?;
                    let mut usage: Value =
                        serde_json::from_str(&usage_json).map_err(|error| error.to_string())?;
                    let tool_call_id = tool_call_id(request);
                    let prior = usage
                        .get("agentAuthorSelections")
                        .and_then(|selections| selections.get(tool_call_id.as_str()))
                        .cloned();
                    // 基线 :247-248——既有 selections 必须 deep-equal；
                    // 无既有时 action 必须为 pending。
                    let prior_present = prior.as_ref().is_some_and(|prior| !prior.is_null());
                    if (prior_present && !is_deep_strict_equal(&prior.unwrap(), selections))
                        || (!prior_present && action.status != "pending")
                    {
                        return Err(GENERATION_AGENT_AUTHOR_SELECTION_CONFLICT.to_string());
                    }
                    let mut author_selections: Map<String, Value> = usage
                        .get("agentAuthorSelections")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    author_selections.insert(tool_call_id, Value::Array(selections.clone()));
                    if let Some(usage_object) = usage.as_object_mut() {
                        usage_object.insert(
                            "agentAuthorSelections".to_string(),
                            Value::Object(author_selections),
                        );
                    }
                    let usage_string =
                        serde_json::to_string(&usage).map_err(|error| error.to_string())?;
                    conn.execute(
                        "UPDATE generation_attempts SET usage_receipt_json=? WHERE attempt_id=?",
                        [usage_string, ref_attempt_id(request)],
                    )
                    .map_err(|error| error.to_string())?;
                }
                // 基线 :252-253——非 pending 幂等返回。
                if action.status != "pending" {
                    return Ok(ActionState::of_action(action));
                }
                // 基线 :254-256——root epoch / 源指纹 / root 状态门禁。
                let root = self.runs.budget(conn, &run.root_action_id)?.root;
                if root.epoch != self.epoch
                    || !self.sources_current(run)?
                    || root.status != "active"
                {
                    return Err(GENERATION_AGENT_TOOL_STALE.to_string());
                }
                // 基线 :257-261——工具名不在上下文输入工具表 → failed +
                // 本地化观察（en-US / zh）。
                let context = self.context(run)?;
                if !context
                    .input
                    .tools
                    .iter()
                    .any(|tool| tool.name == action.name)
                {
                    let observation = if context.writing_language == WritingLanguage::EnUs {
                        format!("Unknown tool: {}", action.name)
                    } else {
                        format!("未知工具：{}", action.name)
                    };
                    return Ok(ActionState {
                        status: "failed".to_string(),
                        observation: Some(observation),
                        ..Default::default()
                    });
                }
                // 基线 :262-266——confirmed → running；否则 declined。
                execute = confirmed;
                if confirmed {
                    Ok(ActionState {
                        status: "running".to_string(),
                        ..Default::default()
                    })
                } else {
                    Ok(ActionState {
                        status: "declined".to_string(),
                        observation: Some("The author declined this action.".to_string()),
                        ..Default::default()
                    })
                }
            },
        )?;
        Ok(AgentToolClaim { action, execute })
    }

    /// 基线 `finishTool(request)`（:259-274）：工具结果回执写入
    /// （幂等重放 / 冲突检测）。
    pub fn finish_tool(&self, request: &Value) -> Result<AgentToolAction, String> {
        // 基线 :260-262——status ∈ {completed, failed, unknown}、
        // observation 为字符串且 ≤256KiB（UTF-8 字节）。
        let status = request
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let observation = request
            .get("observation")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !matches!(status, "completed" | "failed" | "unknown")
            || !request.get("observation").is_some_and(Value::is_string)
            || observation.len() > 256 * 1024
        {
            return Err(GENERATION_AGENT_TOOL_RESULT_INVALID.to_string());
        }
        self.update_action(
            request.get("ref").unwrap_or(&Value::Null),
            |action, run, conn| {
                // 基线 :264——root epoch 门禁（基线逐字为
                // 'GENERATION_EPOCH_STALE'，无 AGENT 前缀）。
                if self.runs.budget(conn, &run.root_action_id)?.root.epoch != self.epoch {
                    return Err(GENERATION_EPOCH_STALE.to_string());
                }
                // 基线 :265-267——读取存储态（含 domainEffect）。
                let usage_json: String = conn
                    .query_row(
                        "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id=?",
                        [ref_attempt_id(request)],
                        |row| row.get(0),
                    )
                    .map_err(|error| error.to_string())?;
                let stored: Value =
                    serde_json::from_str(&usage_json).map_err(|error| error.to_string())?;
                let effect = stored
                    .get("agentActions")
                    .and_then(|agent_actions| agent_actions.get(tool_call_id(request).as_str()))
                    .and_then(|state| state.get("domainEffect"))
                    .cloned();
                // 基线 :268-270——域工具 completed 必须有域效应。
                if status == "completed"
                    && matches!(
                        action.name.as_str(),
                        "propose_novel_config" | "propose_chapter_blueprint"
                    )
                    && effect.is_none()
                {
                    return Err(GENERATION_AGENT_DOMAIN_EFFECT_REQUIRED.to_string());
                }
                // 基线 :271-275——非 running：幂等重放（completed +
                // completed + 有 domainEffect；或同 status 同 observation），
                // 否则 `GENERATION_AGENT_TOOL_RESULT_CONFLICT`。
                if action.status != "running" {
                    if effect.is_some() && action.status == "completed" && status == "completed" {
                        return Ok(ActionState::of_action(action));
                    }
                    if action.status == status && action.observation.as_deref() == Some(observation)
                    {
                        return Ok(ActionState::of_action(action));
                    }
                    return Err(GENERATION_AGENT_TOOL_RESULT_CONFLICT.to_string());
                }
                // 基线 :276-278——写回结果（保留 workflow）。
                Ok(ActionState {
                    status: status.to_string(),
                    observation: Some(observation.to_string()),
                    workflow: action.workflow.clone(),
                    ..Default::default()
                })
            },
        )
    }

    // ── 工作流注册（基线 :276-292 / :305-342）─────────────────────────

    /// 基线 `registerWorkflow(ref)`（:276-292）：`start_workflow` 工具确认后
    /// 注册子生成工作流（registrationId = `agent-workflow:${textHash(toolCallId)}`）。
    pub fn register_workflow(&self, ref_: &Value) -> Result<AgentWorkflowRegistration, String> {
        let updated = self.update_action(ref_, |action, run, conn| {
            // 基线 :278-279——已有 workflow 幂等返回。
            if action.workflow.is_some() {
                return Ok(ActionState::of_action(action));
            }
            // 基线 :280-284——running + start_workflow + root epoch/status +
            // 源指纹门禁。
            let root = self.runs.budget(conn, &run.root_action_id)?.root;
            if action.status != "running"
                || action.name != "start_workflow"
                || root.epoch != self.epoch
                || root.status != "active"
                || !self.sources_current(run)?
            {
                return Err(GENERATION_AGENT_WORKFLOW_NOT_AUTHORIZED.to_string());
            }
            // 基线 :285-291——workflow ∈ {generate_draft, generate_blueprint,
            // generate_architecture} 且参数键集 ⊆
            // {workflow, chapter_number, start_chapter, chapter_count, target_units}；
            // generate_draft 另要求 chapter_number 为 ≥1 安全整数。
            let args = &action.arguments;
            let workflow = args
                .get("workflow")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !matches!(
                workflow,
                "generate_draft" | "generate_blueprint" | "generate_architecture"
            ) || args.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "workflow"
                        | "chapter_number"
                        | "start_chapter"
                        | "chapter_count"
                        | "target_units"
                )
            }) {
                return Err(GENERATION_AGENT_WORKFLOW_INVALID.to_string());
            }
            let chapter_number = args.get("chapter_number");
            if workflow == "generate_draft"
                && !chapter_number
                    .and_then(Value::as_i64)
                    .is_some_and(|chapter_number| chapter_number >= 1)
            {
                return Err(GENERATION_AGENT_WORKFLOW_INVALID.to_string());
            }
            // 基线 :292-298——registration 构造。
            let registration = AgentWorkflowRegistration {
                registration_id: format!("agent-workflow:{}", text_hash(&tool_call_id(ref_))),
                workflow: workflow.to_string(),
                chapter_number: if workflow == "generate_draft" {
                    chapter_number.and_then(Value::as_i64)
                } else {
                    None
                },
                planning: if workflow == "generate_draft" {
                    None
                } else {
                    // 基线 `Number(args.start_chapter)` 等——损坏态（非整数）
                    // 按 0 兜底（运行期参数经 round() 归一化恒为整数）。
                    Some(AgentWorkflowPlanning {
                        from: args
                            .get("start_chapter")
                            .and_then(Value::as_i64)
                            .unwrap_or_default(),
                        to: args
                            .get("start_chapter")
                            .and_then(Value::as_i64)
                            .unwrap_or_default()
                            + args
                                .get("chapter_count")
                                .and_then(Value::as_i64)
                                .unwrap_or_default()
                            - 1,
                        target_units: args
                            .get("target_units")
                            .and_then(Value::as_i64)
                            .unwrap_or_default(),
                    })
                },
                parent_handle: self.handle(run),
                model_id: self.model_id(run)?,
                state: "registered".to_string(),
                child_handles: Vec::new(),
            };
            // 基线 :299-300——`{ status: action.status, workflow: registration }`。
            Ok(ActionState {
                status: action.status.clone(),
                observation: action.observation.clone(),
                workflow: Some(registration),
                ..Default::default()
            })
        })?;
        // 基线 :302——`updated.workflow!`（updateAction 重建后必含 workflow）。
        updated
            .workflow
            .ok_or_else(|| "agent workflow registration missing after update".to_string())
    }

    /// 基线 `workflowRegistration(id)`（:305-342 区段）：全表扫 agent-round
    /// runs，定位 `registrationId` 唯一的 action（`GENERATION_AGENT_REGISTRATION_MISSING`
    /// 当找不到或重复）。
    fn workflow_registration(
        &self,
        id: &str,
    ) -> Result<
        (
            AgentToolAction,
            DurableGenerationRun,
            AgentWorkflowRegistration,
        ),
        String,
    > {
        // 基线 :306——`/^agent-workflow:[a-f0-9]{64}$/u`。
        let Some(hex) = id.strip_prefix("agent-workflow:") else {
            return Err(GENERATION_AGENT_REGISTRATION_INVALID.to_string());
        };
        if !is_session_key(hex) {
            return Err(GENERATION_AGENT_REGISTRATION_INVALID.to_string());
        }
        let rows: Vec<String> = self
            .conn
            .prepare(
                "SELECT run_id FROM generation_runs WHERE json_extract(binding_json,'$.sourceManifest.operation')='agent-round'",
            )
            .map_err(|error| error.to_string())?
            .query_map([], |row| row.get(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|error| error.to_string())?;
        let mut found: Vec<(
            AgentToolAction,
            DurableGenerationRun,
            AgentWorkflowRegistration,
        )> = Vec::new();
        for run_id in rows {
            let run = self.runs.get(self.conn, &run_id)?;
            if run.binding.project_id != self.project_id {
                continue;
            }
            // 基线 :310-314——逐轮重建 actions，过滤 registrationId 匹配项。
            for action in self.round(&run)?.actions {
                if let Some(registration) = &action.workflow {
                    if registration.registration_id == id {
                        let registration = registration.clone();
                        found.push((action, run.clone(), registration));
                    }
                }
            }
        }
        if found.len() != 1 {
            return Err(GENERATION_AGENT_REGISTRATION_MISSING.to_string());
        }
        Ok(found.pop().expect("registration 唯一"))
    }

    // ── 子代效应捕获（基线 :343-380）───────────────────────────────────

    /// 基线 `withChildEffect(handle, effect)`（:343-367）——仅 main 的既有
    /// 正式写入器在自身域事务内调用：捕获会话所有轮次的源指纹变迁
    /// （before/after hash）并写回各轮次 attempt 的
    /// `usage_receipt_json.agentSourceTransition`。
    ///
    /// 基线 `effect: () => T`；Rust 侧显式传入 `&Connection`（调用方域写入
    /// 需要）。事务门禁（`GENERATION_AGENT_TRANSACTION_REQUIRED`）的 Tauri
    /// 等价物为**调用方结构保证**（对齐 `main_generation_owner`
    /// `with_agent_child_effect` 头注约定）。
    pub fn with_child_effect<T>(
        &self,
        handle: &Value,
        effect: impl FnOnce(&Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        // 基线 :346-348——`runs.get(handle.runId)` + handle deep-equal
        // （注意：基线此处不经 requireRun / assertCurrent）。
        let run_id = handle
            .get("runId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let child = self.runs.get(self.conn, run_id)?;
        if !is_deep_strict_equal(
            &serde_json::to_value(self.handle(&child)).map_err(|e| e.to_string())?,
            handle,
        ) {
            return Err(GENERATION_AGENT_CHILD_NOT_AUTHORIZED.to_string());
        }
        // 基线 :349-351——无 agentWorkflowRegistrationId：非 agent 子代，
        // 直接透传效应。
        let Some(registration_id) = child
            .binding
            .source_manifest
            .get("agentWorkflowRegistrationId")
            .and_then(Value::as_str)
        else {
            return effect(self.conn);
        };
        // 基线 :352-356——registration 定位 + childHandles / rootActionId /
        // sourcesCurrent 三重校验。
        let (action, run, registration) = self.workflow_registration(registration_id)?;
        if !registration
            .child_handles
            .iter()
            .any(|item| item.run_id == child.run_id)
            || child.root_action_id != run.root_action_id
            || !self.sources_current(&run)?
        {
            return Err(GENERATION_AGENT_SOURCE_CHANGED.to_string());
        }
        // 基线 :357-363——**效应前**对会话全部轮次 capture before-binding。
        let navigation_key = self.navigation(&run)?.key;
        let projections: Vec<(DurableGenerationRun, String, RunBinding)> = self
            .session_runs(&navigation_key)?
            .iter()
            .map(|target| {
                if !self.sources_current(target)? {
                    return Err(GENERATION_AGENT_SOURCE_CHANGED.to_string());
                }
                let attempt_id: Option<String> = self
                    .conn
                    .query_row(
                        "SELECT attempt_id FROM generation_attempts WHERE run_id=?",
                        [&target.run_id],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|error| error.to_string())?;
                let Some(attempt_id) = attempt_id else {
                    return Err(GENERATION_AGENT_CHILD_EFFECT_ROUND_REQUIRED.to_string());
                };
                Ok((
                    target.clone(),
                    attempt_id,
                    self.host.capture(self.conn, target)?,
                ))
            })
            .collect::<Result<_, _>>()?;
        let result = effect(self.conn)?;
        // 基线 :364-366——**效应后** capture after-binding 并把
        // agentSourceTransition（含 before/after hash）写回每个轮次 attempt。
        for (target, attempt_id, before) in projections {
            let binding = self.host.capture(self.conn, &target)?;
            let usage_json: String = self
                .conn
                .query_row(
                    "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id=?",
                    [&attempt_id],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            let mut usage: Value =
                serde_json::from_str(&usage_json).map_err(|error| error.to_string())?;
            let before_string =
                serde_json::to_string(&before).map_err(|error| error.to_string())?;
            let binding_string =
                serde_json::to_string(&binding).map_err(|error| error.to_string())?;
            let transition = json!({
                "actionId": format!("{}:{}", action.ref_.tool_call_id, child.run_id),
                "beforeHash": text_hash(&before_string),
                "afterHash": text_hash(&binding_string),
                "binding": serde_json::to_value(&binding).map_err(|error| error.to_string())?,
            });
            if let Some(usage_object) = usage.as_object_mut() {
                usage_object.insert("agentSourceTransition".to_string(), transition);
            }
            let usage_string = serde_json::to_string(&usage).map_err(|error| error.to_string())?;
            self.conn
                .execute(
                    "UPDATE generation_attempts SET usage_receipt_json=? WHERE attempt_id=?",
                    [usage_string, attempt_id],
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(result)
    }

    /// 基线 `recordWorkflowChild(id, child)`（:369-380）：run 创建后登记子代
    /// handle，并把 registration 态推进到 `'started'`（childHandles 幂等追加）。
    pub fn record_workflow_child(&self, id: &str, child: &Value) -> Result<(), String> {
        // 基线 :371-373——registration 定位 + child 身份校验
        // （rootActionId 与 projectId 匹配）。
        let (action, run) = {
            let (action, run, _registration) = self.workflow_registration(id)?;
            (action, run)
        };
        let child_handle: MainGenerationRunHandle = serde_json::from_value(child.clone())
            .map_err(|_| GENERATION_AGENT_CHILD_NOT_AUTHORIZED.to_string())?;
        if child_handle.root_action_id != run.root_action_id
            || child_handle.project_id != run.binding.project_id
        {
            return Err(GENERATION_AGENT_CHILD_NOT_AUTHORIZED.to_string());
        }
        // 基线 :374-378——updateAction 内推进 workflow 态。
        let child_handle_value =
            serde_json::to_value(child_handle.clone()).map_err(|error| error.to_string())?;
        self.update_action(
            &serde_json::to_value(&action.ref_).map_err(|error| error.to_string())?,
            |current, _run, _conn| {
                let Some(workflow) = current.workflow.clone() else {
                    // 基线 `current.workflow!` 非空断言——损坏态防御。
                    return Err("agent workflow registration missing".to_string());
                };
                let mut child_handles = workflow.child_handles.clone();
                let already_registered = child_handles
                    .iter()
                    .any(|item| item.run_id == child_handle.run_id);
                if !already_registered {
                    child_handles.push(child_handle.clone());
                }
                Ok(ActionState {
                    status: current.status.clone(),
                    observation: current.observation.clone(),
                    workflow: Some(AgentWorkflowRegistration {
                        planning: workflow.planning.clone(),
                        registration_id: workflow.registration_id.clone(),
                        workflow: workflow.workflow.clone(),
                        chapter_number: workflow.chapter_number.clone(),
                        parent_handle: workflow.parent_handle.clone(),
                        model_id: workflow.model_id.clone(),
                        state: "started".to_string(),
                        child_handles,
                    }),
                    ..Default::default()
                })
            },
        )?;
        // 基线返回 void；child_handle_value 仅用于借用检查平息（实际未使用）。
        let _ = child_handle_value;
        Ok(())
    }

    // ── 控制恢复与轮次准备 ─────────────────────────────────────────

    /// 已完成回复只移交控制纪元，不重写历史绑定，也不派发第二个模型请求。
    /// 未完成阶段仍由宿主恢复，其特殊域准入和不可重放保护不得绕过。
    pub fn resume(&self, handle: &Value) -> Result<AgentGenerationRecovery, String> {
        let recovery = self.read(handle)?;
        if recovery.source_status != "current" {
            return Err("GENERATION_SOURCE_CHANGED".to_string());
        }
        let last = recovery
            .rounds
            .last()
            .ok_or_else(|| "GENERATION_AGENT_ROUND_INVALID".to_string())?;
        if last.status != "completed" {
            let view = self.host.resume(self.conn, &recovery.handle)?;
            return self.read(view.get("handle").unwrap_or(&Value::Null));
        }
        let attempt_id = last
            .attempt_id
            .as_deref()
            .ok_or_else(|| "GENERATION_AGENT_ROUND_INVALID".to_string())?;
        let transaction = if self.conn.is_autocommit() {
            Some(
                rusqlite::Transaction::new_unchecked(
                    self.conn,
                    rusqlite::TransactionBehavior::Immediate,
                )
                .map_err(|error| error.to_string())?,
            )
        } else {
            None
        };
        // 即使调用方捕获错误后提交外层事务，根纪元和控制收据也必须一起回滚。
        let mut savepoint = AgentControlSavepoint::new(self.conn)?;
        self.runs.activate_root_for_committed_stage(
            self.conn,
            &recovery.handle.root_action_id,
            &crate::repositories::generation_run_repository::ProjectEpoch {
                project_id: self.project_id.to_string(),
                epoch: self.epoch.to_string(),
            },
        )?;
        let raw: String = self.conn.query_row(
            "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id=?1 AND run_id=?2",
            [attempt_id, last.handle.run_id.as_str()], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        let mut usage: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
        usage
            .as_object_mut()
            .ok_or_else(|| "GENERATION_AGENT_ROUND_INVALID".to_string())?
            .insert("agentControlEpoch".to_string(), json!(self.epoch));
        self.conn.execute(
            "UPDATE generation_attempts SET usage_receipt_json=?1 WHERE attempt_id=?2 AND run_id=?3",
            rusqlite::params![usage.to_string(), attempt_id, last.handle.run_id],
        ).map_err(|error| error.to_string())?;
        savepoint.release()?;
        if let Some(transaction) = transaction {
            transaction.commit().map_err(|error| error.to_string())?;
        }
        self.read(&serde_json::to_value(&recovery.handle).map_err(|error| error.to_string())?)
    }

    /// 基线 `executeRound(request)`（:180-219）——随分发核心
    /// （generation-run-service）波次实现

    pub fn execute_round(&self, _request: &Value) -> Result<AgentGenerationRecovery, String> {
        Err("GENERATION_AGENT_ROUND_UNMIGRATED".to_string())
    }

    /// 基线 executeRound 的同步准备段：先认证，再校验轮次与准入；
    /// 已返回或未知调用只重放，绝不因再次点击而静默派发。
    ///
    /// 此方法不调用 host.execute；普通 IPC 仍在 execute_round 中 fail-closed，
    /// 待 owner 将拥有的计划接入统一执行控制后再开放。
    pub fn prepare_execute_round(&self, request: &Value) -> Result<PreparedAgentRound, String> {
        let requested_handle = request.get("handle").unwrap_or(&Value::Null);
        let selected = self.require_run(requested_handle)?;
        let recovery = self.read(requested_handle)?;
        // 0.0 与 0 在 JS 中均为安全整数；有限的 0..7 直接限定安全整数范围。
        let index = request
            .get("index")
            .and_then(Value::as_f64)
            .filter(|value| value.fract() == 0.0 && (0.0..8.0).contains(value))
            .ok_or_else(|| "GENERATION_AGENT_ROUND_INVALID".to_string())?
            as i64;
        let existing = recovery.rounds.iter().find(|round| round.index == index);
        if existing.is_some_and(|round| round.status != "not-started") {
            return Ok(PreparedAgentRound::Replay(Box::new(recovery)));
        }
        if recovery.next_round != Some(index) {
            return Err("GENERATION_AGENT_ROUND_ORDER".to_string());
        }
        let blocked = recovery
            .run
            .get("ledger")
            .and_then(|ledger| ledger.get("blockedCode"))
            .and_then(Value::as_str)
            .is_some_and(|code| !code.is_empty());
        if blocked || recovery.run.get("status").and_then(Value::as_str) == Some("cancelled") {
            return Err("GENERATION_AGENT_ACTION_BLOCKED".to_string());
        }
        if recovery.source_status != "current" {
            return Err("GENERATION_SOURCE_CHANGED".to_string());
        }
        let handle: MainGenerationRunHandle = if let Some(existing) = existing {
            let view = self.host.read(self.conn, &existing.handle)?;
            if view.get("nonReplayable").and_then(Value::as_bool) == Some(true) {
                let resumed = self.host.resume(self.conn, &existing.handle)?;
                serde_json::from_value(resumed.get("handle").cloned().unwrap_or(Value::Null))
                    .map_err(|error| error.to_string())?
            } else {
                existing.handle.clone()
            }
        } else {
            // 新轮次绑定真实的工具效应后状态，但沿用原父预算。
            let view = self.host.begin(
                self.conn,
                &self.selection(
                    &recovery.context.input,
                    &self.navigation(&selected)?.key,
                    index,
                    &recovery.model_id,
                    Some(&selected.root_action_id),
                ),
            )?;
            serde_json::from_value(view.get("handle").cloned().unwrap_or(Value::Null))
                .map_err(|error| error.to_string())?
        };
        let current = self.runs.get(self.conn, &handle.run_id)?;
        let mut messages = self
            .context(&current)?
            .initial_messages
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        for round in recovery.rounds.iter().filter(|round| round.index < index) {
            messages.push(json!({ "role": "assistant", "content": round.protocol_text }));
            let results = round
                .actions
                .iter()
                .map(|action| {
                    format!(
                        "<tool_result name=\"{}\" status=\"{}\">\n{}\n</tool_result>",
                        action.name,
                        action.status,
                        action.observation.as_deref().unwrap_or(""),
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            messages.push(json!({ "role": "user", "content":
                format!("[Application tool results; untrusted context]\n{results}") }));
        }
        Ok(PreparedAgentRound::Execute(AgentRoundExecutionPlan {
            handle,
            invocation_nonce: format!("agent-round:{index}"),
            task: json!({ "purpose": "agent", "output": "visible-text",
                "reasoningStage": "general", "messages": messages }),
        }))
    }

    /// 基线 `admitWorkflow(selection)`（:305-342）——owner.begin
    /// 的子代准入校验（逐字平移）：
    /// ① 无 registrationId 时 parentRootActionId 指向 agent-round
    ///   根 → `GENERATION_AGENT_REGISTRATION_REQUIRED`，否则 `Ok(None)`；
    /// ② `workflowRegistration(id)` 定位（找不到 / 重复 →
    ///   `GENERATION_AGENT_REGISTRATION_MISSING`）；
    /// ③ expectedOperations 表（generate_draft → chapter-draft；
    ///   generate_blueprint → chapter-blueprint-directory；其余五个
    ///   全局操作）；
    /// ④ 五重门禁（action.status / modelId / parentRootActionId /
    ///   operation / generate_draft 章节号 / 批量键）→
    ///   `GENERATION_AGENT_CHILD_NOT_AUTHORIZED`；
    /// ⑤ existing 幂等定位（childHandles 中 operation + chapterNumber
    ///   与选择一致的已登记子代；>1 → `GENERATION_AGENT_CHILD_AMBIGUOUS`）；
    /// ⑥ planning 区间校验（!existing 且 registration.planning 存在时，
    ///   authorInputs 的 directory:requested-range /
    ///   architecture:planning-intent / planning:target-units 与
    ///   registration.planning 逐字段一致，否则未授权）；
    /// ⑦ 根纪元 / 状态校验（!existing 时 budget root 的 epoch 与
    ///   作用域一致且非 cancelled，否则 `GENERATION_AGENT_TOOL_STALE`）；
    /// ⑧ 源一致性（!existing 且 `sourcesCurrent` 失败 →
    ///   `GENERATION_AGENT_SOURCE_CHANGED`）；
    /// ⑨ 返回 `{ existing, parentRootActionId }`。
    pub fn admit_workflow(&self, selection: &Value) -> Result<Option<AdmittedWorkflow>, String> {
        // ① 基线 :306-312——无 registrationId：parentRootActionId
        // 指向 agent-round 根 → 缺少注册；否则非 agent 子代，透传。
        let Some(registration_id) = selection
            .get("agentWorkflowRegistrationId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            if let Some(parent_root_action_id) =
                selection.get("parentRootActionId").and_then(Value::as_str)
            {
                let root = self.runs.budget(self.conn, parent_root_action_id)?.root;
                if root.operation == "agent-round" {
                    return Err(GENERATION_AGENT_REGISTRATION_REQUIRED.to_string());
                }
            }
            return Ok(None);
        };
        // ② 基线 :313——registrationId 唯一定位。
        let (action, run, registration) = self.workflow_registration(registration_id)?;
        // ③ 基线 :314-318——expectedOperations 表。
        let expected_operations: Vec<&str> = if registration.workflow == "generate_draft" {
            vec!["chapter-draft"]
        } else if registration.workflow == "generate_blueprint" {
            vec!["chapter-blueprint-directory"]
        } else {
            vec![
                "generate-global-config",
                "generate-core-seed",
                "character-architecture",
                "generate-world-building",
                "generate-plot-outline",
            ]
        };
        // ④ 基线 :319-322——五重门禁（任一不满足 → 未授权）。
        let selection_operation = selection
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let selection_chapter_number = selection.get("chapterNumber").and_then(Value::as_i64);
        let not_authorized = !["running", "completed"].contains(&action.status.as_str())
            || selection.get("modelId").and_then(Value::as_str)
                != Some(registration.model_id.as_str())
            || selection.get("parentRootActionId").and_then(Value::as_str)
                != Some(run.root_action_id.as_str())
            || !expected_operations.contains(&selection_operation)
            || (registration.workflow == "generate_draft"
                && selection_chapter_number != registration.chapter_number)
            || selection
                .get("batchId")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty())
            || selection
                .get("batchIntent")
                .is_some_and(|intent| !intent.is_null())
            || selection
                .get("continueDirectoryOperationId")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty());
        if not_authorized {
            return Err(GENERATION_AGENT_CHILD_NOT_AUTHORIZED.to_string());
        }
        // ⑤ 基线 :323-327——existing 幂等定位：childHandles 中
        // operation + chapterNumber 与选择一致的已登记子代。
        let matches: Vec<DurableGenerationRun> = registration
            .child_handles
            .iter()
            .map(|handle| self.runs.get(self.conn, &handle.run_id))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|child| {
                child
                    .binding
                    .source_manifest
                    .get("operation")
                    .and_then(Value::as_str)
                    == Some(selection_operation)
                    && child
                        .binding
                        .source_manifest
                        .get("chapterNumber")
                        .and_then(Value::as_i64)
                        == selection_chapter_number
            })
            .collect();
        if matches.len() > 1 {
            return Err(GENERATION_AGENT_CHILD_AMBIGUOUS.to_string());
        }
        let existing = matches.into_iter().next();
        // ⑥ 基线 :328-337——planning 区间校验（!existing 且
        // registration.planning 存在时）。
        if existing.is_none() && registration.planning.is_some() {
            let planning = registration.planning.as_ref().expect("planning 存在");
            // 基线 :329 `const input = (id) => selection.authorInputs?.find(item => item.id === id)?.text`。
            let input = |id: &str| -> Option<&Value> {
                selection
                    .get("authorInputs")
                    .and_then(Value::as_array)
                    .and_then(|inputs| {
                        inputs
                            .iter()
                            .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
                    })
                    .and_then(|item| item.get("text"))
            };
            // 基线 :330-334——JSON 解析失败 → 未授权（catch 分支）。
            let range: Option<Value> = if registration.workflow == "generate_blueprint" {
                let text = input("directory:requested-range")
                    .and_then(Value::as_str)
                    .unwrap_or("null");
                let value: Value = serde_json::from_str(text)
                    .map_err(|_| GENERATION_AGENT_CHILD_NOT_AUTHORIZED.to_string())?;
                // 基线 `value && { from: value.startChapter, to: value.endChapter }`——
                // value 为 null 时区间为 undefined（Rust `None`）。
                if value.is_null() {
                    None
                } else {
                    Some(json!({
                        "from": value.get("startChapter").cloned().unwrap_or(Value::Null),
                        "to": value.get("endChapter").cloned().unwrap_or(Value::Null),
                    }))
                }
            } else {
                let text = input("architecture:planning-intent")
                    .and_then(Value::as_str)
                    .unwrap_or("null");
                let value: Value = serde_json::from_str(text)
                    .map_err(|_| GENERATION_AGENT_CHILD_NOT_AUTHORIZED.to_string())?;
                // 基线 `JSON.parse(...)?.synopsisRange`——解析结果为
                // null 时为 undefined（Rust `None`）。
                value.get("synopsisRange").cloned()
            };
            // 基线 :335——`!isDeepStrictEqual(range, { from, to }) ||
            // Number(input('planning:target-units')) !== targetUnits`。
            let expected_range = json!({ "from": planning.from, "to": planning.to });
            let target_units_match = match input("planning:target-units") {
                Some(Value::Number(number)) => number.as_i64() == Some(planning.target_units),
                Some(Value::String(text)) => {
                    text.parse::<i64>().ok() == Some(planning.target_units)
                }
                _ => false,
            };
            if !is_deep_strict_equal(range.as_ref().unwrap_or(&Value::Null), &expected_range)
                || !target_units_match
            {
                return Err(GENERATION_AGENT_CHILD_NOT_AUTHORIZED.to_string());
            }
        }
        // ⑦ 基线 :338-339——根纪元 / 取消状态校验（!existing 时；
        // 基线两次 budget 调用合并为一次，语义等价——无副作用）。
        if existing.is_none() {
            let root = self.runs.budget(self.conn, &run.root_action_id)?.root;
            if root.epoch != self.epoch || root.status == "cancelled" {
                return Err(GENERATION_AGENT_TOOL_STALE.to_string());
            }
        }
        // ⑧ 基线 :340——源一致性（!existing 时）。
        if existing.is_none() && !self.sources_current(&run)? {
            return Err(GENERATION_AGENT_SOURCE_CHANGED.to_string());
        }
        // ⑨ 基线 :341-342——返回准入结果。
        Ok(Some(AdmittedWorkflow {
            existing: existing.map(|child| self.handle(&child)),
            parent_root_action_id: run.root_action_id.clone(),
        }))
    }

    /// 基线 `commitDomainTool(ref)`（:377-404）——域工具效应
    /// 提交（`.immediate()` 事务）：`action(ref)` 定位 →
    /// 幂等（已有 domainEffect 直接返回
    /// `{...domainEffect, current}` 新鲜读投影）→ 门禁
    /// （running + root epoch + root active + sourcesCurrent
    /// + 工具在上下文输入表且 builtin / requiresConfirmation
    /// / !isReadOnly）→ 会话全轮次投影 capture before →
    /// `commit_agent_domain_tool` 提交真实域写入 → 全轮次
    /// 写 `agentSourceTransition` + 本 attempt action 补
    /// `domainEffect` → 返回 `{...effect, current: true}`。
    pub fn commit_domain_tool(&self, ref_: &Value) -> Result<AgentDomainToolReceipt, String> {
        if self.conn.is_autocommit() {
            // 无环境事务：自开事务（基线 `.immediate()`）。
            let tx = self
                .conn
                .unchecked_transaction()
                .map_err(|error| error.to_string())?;
            let outcome = self.commit_domain_tool_in_transaction(ref_);
            match outcome {
                Ok(receipt) => tx
                    .commit()
                    .map(|_| receipt)
                    .map_err(|error| error.to_string()),
                // 错误时 tx drop 即回滚，与基线 `db.transaction()` 语义一致。
                Err(error) => Err(error),
            }
        } else {
            // 基线 better-sqlite3 嵌套事务自动降级为 SAVEPOINT；
            // rusqlite 无嵌套事务——直接复用外层环境事务
            // （对齐 generation_run_repository 头注约定）。
            self.commit_domain_tool_in_transaction(ref_)
        }
    }

    /// `commitDomainTool` 事务主体（基线 :378-403）。
    fn commit_domain_tool_in_transaction(
        &self,
        ref_: &Value,
    ) -> Result<AgentDomainToolReceipt, String> {
        // 基线 :379——`action(ref)` 定位（requireRun +
        // round().actions 精确定位，否则
        // `GENERATION_AGENT_TOOL_IDENTITY_MISMATCH`）。
        let (run, action) = self.action(ref_)?;
        let tool_call_id = ref_
            .get("toolCallId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        // 基线 :380-381——读本 attempt 的 usage_receipt_json。
        let attempt_id = ref_
            .get("attemptId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let usage_json: String = self
            .conn
            .query_row(
                "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id=?",
                [attempt_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let usage: Value = serde_json::from_str(&usage_json).map_err(|error| error.to_string())?;
        let state = usage
            .get("agentActions")
            .and_then(|agent_actions| agent_actions.get(tool_call_id.as_str()))
            .cloned();
        // 基线 :382-384——幂等：已有 domainEffect 直接返回
        // （`current` 为新鲜读投影比对）。
        if let Some(mut effect) = state
            .as_ref()
            .and_then(|state| state.get("domainEffect"))
            .cloned()
        {
            let receipt: AgentDomainToolReceipt =
                serde_json::from_value(effect.clone()).map_err(|error| error.to_string())?;
            let current =
                agent_domain_tool_effect::is_agent_domain_tool_current(self.conn, &receipt)?;
            if let Some(object) = effect.as_object_mut() {
                object.insert("current".to_string(), Value::Bool(current));
            }
            return serde_json::from_value(effect).map_err(|error| error.to_string());
        }
        // 基线 :385-390——门禁。
        let root = self.runs.budget(self.conn, &run.root_action_id)?.root;
        if action.status != "running"
            || root.epoch != self.epoch
            || root.status != "active"
            || !self.sources_current(&run)?
            || !self.context(&run)?.input.tools.iter().any(|tool| {
                tool.name == action.name
                    && matches!(tool.source, AgentToolSource::Builtin)
                    && tool.requires_confirmation
                    && !tool.is_read_only
            })
        {
            return Err(GENERATION_AGENT_DOMAIN_TOOL_NOT_AUTHORIZED.to_string());
        }
        // 基线 :391-397——会话全轮次投影 capture before
        // （同 withChildEffect；缺 attempt →
        // `GENERATION_AGENT_CHILD_EFFECT_ROUND_REQUIRED`）。
        let navigation_key = self.navigation(&run)?.key;
        let projections: Vec<(DurableGenerationRun, String, RunBinding)> = self
            .session_runs(&navigation_key)?
            .iter()
            .map(|target| {
                if !self.sources_current(target)? {
                    return Err(GENERATION_AGENT_SOURCE_CHANGED.to_string());
                }
                let target_attempt_id: Option<String> = self
                    .conn
                    .query_row(
                        "SELECT attempt_id FROM generation_attempts WHERE run_id=?",
                        [&target.run_id],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|error| error.to_string())?;
                let Some(target_attempt_id) = target_attempt_id else {
                    return Err(GENERATION_AGENT_CHILD_EFFECT_ROUND_REQUIRED.to_string());
                };
                Ok((
                    target.clone(),
                    target_attempt_id,
                    self.host.capture(self.conn, target)?,
                ))
            })
            .collect::<Result<_, _>>()?;
        // 基线 :398——提交域效应（事务内真实写入
        // project_core / blueprints）。
        let effect = agent_domain_tool_effect::commit_agent_domain_tool(
            self.conn,
            &tool_call_id,
            &action.name,
            &action.arguments,
        )?;
        // 基线 :399-403——全轮次写 agentSourceTransition；
        // 本 attempt 的 action 状态补 domainEffect。
        for (target, target_attempt_id, before) in projections {
            let binding = self.host.capture(self.conn, &target)?;
            let mut target_usage: Value = if target_attempt_id == attempt_id {
                usage.clone()
            } else {
                let target_json: String = self
                    .conn
                    .query_row(
                        "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id=?",
                        [&target_attempt_id],
                        |row| row.get(0),
                    )
                    .map_err(|error| error.to_string())?;
                serde_json::from_str(&target_json).map_err(|error| error.to_string())?
            };
            let before_string =
                serde_json::to_string(&before).map_err(|error| error.to_string())?;
            let binding_string =
                serde_json::to_string(&binding).map_err(|error| error.to_string())?;
            let transition = json!({
                "actionId": tool_call_id,
                "beforeHash": text_hash(&before_string),
                "afterHash": text_hash(&binding_string),
                "binding": serde_json::to_value(&binding).map_err(|error| error.to_string())?,
            });
            if let Some(object) = target_usage.as_object_mut() {
                object.insert("agentSourceTransition".to_string(), transition);
            }
            if target_attempt_id == attempt_id {
                // 基线 :402——`{ ...state!, domainEffect: effect }`
                // （state 缺失时基线 spread undefined 等价于
                // 仅含 domainEffect 的新对象）。
                let mut merged_state = state.clone().unwrap_or_else(|| json!({}));
                if let Some(object) = merged_state.as_object_mut() {
                    object.insert(
                        "domainEffect".to_string(),
                        serde_json::to_value(&effect).map_err(|error| error.to_string())?,
                    );
                }
                let mut agent_actions: Map<String, Value> = target_usage
                    .get("agentActions")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                agent_actions.insert(tool_call_id.clone(), merged_state);
                if let Some(object) = target_usage.as_object_mut() {
                    object.insert("agentActions".to_string(), Value::Object(agent_actions));
                }
            }
            let usage_string =
                serde_json::to_string(&target_usage).map_err(|error| error.to_string())?;
            self.conn
                .execute(
                    "UPDATE generation_attempts SET usage_receipt_json=? WHERE attempt_id=?",
                    [usage_string, target_attempt_id],
                )
                .map_err(|error| error.to_string())?;
        }
        // 基线 :404——`{ ...effect, current: true }`。
        Ok(AgentDomainToolReceipt {
            current: Some(true),
            ..effect
        })
    }
}

// ── 辅助函数 ────────────────────────────────────────────────────────────

/// 基线 `textHash`（SHA-256 十六进制小写）——文件内私有副本
/// （对齐 `commands/generation.rs` / `graph_generation.rs` 先例）。
fn text_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 基线 `isDeepStrictEqual` 的 JSON 等价物（对齐
/// `generation_source_binding::is_deep_strict_equal` 先例）。
fn is_deep_strict_equal<T: Serialize + ?Sized, U: Serialize + ?Sized>(left: &T, right: &U) -> bool {
    match (serde_json::to_value(left), serde_json::to_value(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// 基线 `/^[a-f0-9]{64}$/u`——64 位小写 hex（session key / registrationId 后缀）。
fn is_session_key(key: &str) -> bool {
    key.len() == 64 && key.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

/// 基线 `request.ref.attemptId` 访问（`action` / `updateAction` 内部 SQL 参数）。
fn ref_attempt_id(request: &Value) -> String {
    request
        .get("ref")
        .and_then(|ref_| ref_.get("attemptId"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// 基线 `request.ref.toolCallId` 访问。
fn tool_call_id(request: &Value) -> String {
    request
        .get("ref")
        .and_then(|ref_| ref_.get("toolCallId"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

// ── 测试（参照 services/graph_generation.rs 测试内嵌形态）────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::agent_generation_context::{AgentFrozenTool, AgentGenerationMode};
    use serde_json::json;

    /// In-memory DB + 全量 schema（对齐 graph_generation.rs 测试先例）。
    fn test_connection() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory DB");
        crate::db::schema::create_tables(&conn).expect("create tables");
        conn
    }

    fn assert_current() -> Result<(), String> {
        Ok(())
    }

    /// 测试宿主：记录调用；`begin` 按 selection 直接开一个 agent-round run
    /// （模拟 owner.begin 的 agent 初始化路径）；`read` 返回桩视图；
    /// `current` 恒真（无 agentSourceTransition 时基线 compare 语义）。
    struct TestHost;

    impl AgentHost for TestHost {
        fn begin(&self, _conn: &Connection, selection: &Value) -> Result<Value, String> {
            // 测试宿主语义：按 selection 的 agentSession / agentInput 构造
            // 一个新的 agent-round run（context 用测试上下文，哈希口径与
            // generation_source_binding 一致）。
            let context = test_context();
            let session = selection.get("agentSession").cloned().unwrap_or(json!({}));
            let key = session
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let round_index = session
                .get("roundIndex")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            let binding = agent_binding(&context, key, round_index, None);
            seed_agent_run(
                _conn,
                "host-begin-root",
                &format!("host-begin-run-{key}-{round_index}"),
                &binding,
                None,
            );
            Ok(json!({
                "handle": {
                    "projectId": "project-1",
                    "epoch": "epoch-1",
                    "rootActionId": "host-begin-root",
                    "runId": format!("host-begin-run-{key}-{round_index}"),
                },
                "status": "active",
            }))
        }

        fn read(
            &self,
            _conn: &Connection,
            handle: &MainGenerationRunHandle,
        ) -> Result<Value, String> {
            Ok(json!({
                "handle": serde_json::to_value(handle).unwrap_or_default(),
                "status": "active",
            }))
        }

        fn resume(
            &self,
            _conn: &Connection,
            _handle: &MainGenerationRunHandle,
        ) -> Result<Value, String> {
            Err("test host resume not implemented".to_string())
        }

        fn execute(
            &self,
            _conn: &Connection,
            _handle: &MainGenerationRunHandle,
            _nonce: &str,
            _task: &Value,
        ) -> Result<Value, String> {
            Err("test host execute not implemented".to_string())
        }

        fn current(
            &self,
            _conn: &Connection,
            _run: &DurableGenerationRun,
            _expected: Option<&RunBinding>,
        ) -> bool {
            true
        }

        fn capture(
            &self,
            _conn: &Connection,
            run: &DurableGenerationRun,
        ) -> Result<RunBinding, String> {
            Ok(run.binding.clone())
        }
    }

    fn test_fingerprint() -> Value {
        json!({
            "chapterBriefHash": "h1",
            "authorGuidanceHash": "h2",
            "dependencyHash": "h3",
            "contextSnapshotHash": "h4",
            "templateHash": "h5",
            "skillSnapshotHash": "h6",
            "modelLeaseRevision": "r1",
            "policyHash": "h7",
            "outputContractHash": "h8",
        })
    }

    fn test_context() -> AgentGenerationContext {
        AgentGenerationContext {
            version: 1,
            input: AgentGenerationInput {
                mode: AgentGenerationMode::Fast,
                ui_locale: "zh-CN".to_string(),
                history_messages: Vec::new(),
                user_message: "继续".to_string(),
                editor_context: None,
                tools: vec![
                    AgentFrozenTool {
                        name: "propose_novel_config".to_string(),
                        description: "d1".to_string(),
                        input_schema: json!({}),
                        requires_confirmation: true,
                        is_read_only: false,
                        source: AgentToolSource::Builtin,
                    },
                    AgentFrozenTool {
                        name: "propose_chapter_blueprint".to_string(),
                        description: "d2".to_string(),
                        input_schema: json!({}),
                        requires_confirmation: true,
                        is_read_only: false,
                        source: AgentToolSource::Builtin,
                    },
                ],
            },
            writing_language: WritingLanguage::ZhCn,
            initial_messages: Vec::new(),
        }
    }

    /// 基线 `generation_source_binding` 的 agent binding 构造口径：
    /// agentContext / agentContextHash（typed 序列化哈希）/ agentSession /
    /// modelReceipt / operation。
    fn agent_binding(
        context: &AgentGenerationContext,
        session_key: &str,
        round_index: i64,
        extra_manifest: Option<Map<String, Value>>,
    ) -> Value {
        let mut manifest = json!({
            "operation": "agent-round",
            "modelReceipt": { "modelId": "model-1" },
            "agentContext": serde_json::to_value(context).unwrap_or_default(),
            "agentContextHash": text_hash(&serde_json::to_string(context).unwrap()),
            "agentSession": { "key": session_key, "roundIndex": round_index },
        });
        if let Some(extra) = extra_manifest {
            if let Some(object) = manifest.as_object_mut() {
                for (key, value) in extra {
                    object.insert(key, value);
                }
            }
        }
        json!({
            "projectId": "project-1",
            "epoch": "epoch-1",
            "fingerprint": test_fingerprint(),
            "contextSnapshotId": "snapshot-1",
            "sourceManifest": manifest,
            "sourceRefs": [],
        })
    }

    /// 构造 agent-round run 的完整 fixture（root + run + 可选 attempt）。
    fn seed_agent_run(
        conn: &Connection,
        root_action_id: &str,
        run_id: &str,
        binding: &Value,
        attempt: Option<AttemptFixture>,
    ) {
        let action_json = json!({
            "projectId": "project-1",
            "epoch": "epoch-1",
            "rootActionId": root_action_id,
            "operation": "agent-round",
            "uiActionNonce": "nonce-1",
            "frozenInputHash": "input-hash",
            "status": "active",
        });
        let budget_json = json!({
            "maxPhysicalRequests": 4,
            "maxTokenLiability": 1000,
            "maxOutputPerRequest": 512,
            "maxActiveElapsedMs": 60000,
        });
        conn.execute(
            "INSERT OR REPLACE INTO generation_roots (root_action_id,idempotency_key,action_json,budget_json,active_elapsed_ms,revision) VALUES (?,?,?,?,?,?)",
            [root_action_id, &format!("idem-{root_action_id}-{run_id}"), &action_json.to_string(), &budget_json.to_string(), &"0".to_string(), &"0".to_string()],
        )
        .expect("insert root");
        conn.execute(
            "INSERT INTO generation_runs (run_id,root_action_id,binding_json,status,created_at_ms,open_key) VALUES (?,?,?,?,?,?)",
            [run_id, root_action_id, &binding.to_string(), &"active".to_string(), &"0".to_string(), &format!("open-{run_id}")],
        )
        .expect("insert run");
        let Some(attempt) = attempt else {
            return;
        };
        let attempt_json = json!({
            "attemptId": attempt.attempt_id,
            "reservationId": format!("reservation-{}", attempt.attempt_id),
            "rootActionId": root_action_id,
            "status": attempt.attempt_status,
            "reservedTokens": 0,
            "requestedOutputTokens": 0,
        });
        conn.execute(
            "INSERT INTO generation_attempts (attempt_id,reservation_id,run_id,root_action_id,attempt_json,usage_receipt_json,invocation_nonce) VALUES (?,?,?,?,?,?,?)",
            [
                &attempt.attempt_id,
                &format!("reservation-{}", attempt.attempt_id),
                run_id,
                root_action_id,
                &attempt_json.to_string(),
                &{
                    // fixture 补全：artifactIdentity 与 artifact 行同口径
                    // （基线由执行时写入；缺失或 id 不匹配时创建/修正，
                    // epoch/fingerprint 缺失时补默认值）。
                    let mut usage = attempt.usage_receipt_json.clone();
                    if attempt.artifact.is_some() {
                        let expected_id = format!("artifact-{}", attempt.attempt_id);
                        let needs_identity = usage
                            .get("artifactIdentity")
                            .and_then(|identity| identity.get("artifactId"))
                            .and_then(Value::as_str)
                            != Some(expected_id.as_str());
                        if needs_identity {
                            let mut identity = usage
                                .get("artifactIdentity")
                                .cloned()
                                .unwrap_or_else(|| json!({}));
                            if let Some(map) = identity.as_object_mut() {
                                map.insert("artifactId".to_string(), json!(expected_id));
                                if !map.contains_key("epoch") {
                                    map.insert("epoch".to_string(), json!("epoch-1"));
                                }
                                if !map.contains_key("fingerprint") {
                                    map.insert(
                                        "fingerprint".to_string(),
                                        test_fingerprint(),
                                    );
                                }
                            }
                            if let Some(object) = usage.as_object_mut() {
                                object.insert("artifactIdentity".to_string(), identity);
                            }
                        }
                    }
                    usage.to_string()
                },
                &format!("invocation-{}", attempt.attempt_id),
            ],
        )
        .expect("insert attempt");
        if let Some((artifact_text, revision)) = attempt.artifact {
            let artifact_json = json!({
                "projectId": "project-1",
                "epoch": "epoch-1",
                "artifactId": format!("artifact-{}", attempt.attempt_id),
                "attemptId": attempt.attempt_id,
                "rootActionId": root_action_id,
                "revision": revision,
                "text": artifact_text,
                "textHash": text_hash(&artifact_text),
                "fingerprint": test_fingerprint(),
            });
            conn.execute(
                "INSERT INTO generation_artifacts (artifact_id,attempt_id,run_id,artifact_json,revision,status) VALUES (?,?,?,?,?,?)",
                [
                    format!("artifact-{}", attempt.attempt_id),
                    attempt.attempt_id.to_string(),
                    run_id.to_string(),
                    artifact_json.to_string(),
                    revision.to_string(),
                    "settled".to_string(),
                ],
            )
            .expect("insert artifact");
        }
    }

    struct AttemptFixture {
        attempt_id: String,
        attempt_status: &'static str,
        usage_receipt_json: Value,
        /// `(text, revision)`——存在时同步插入 generation_artifacts 行
        /// （回执 artifact 完整性校验需要 artifactIdentity 同口径）。
        artifact: Option<(String, i64)>,
    }

    /// completed 轮次 attempt 的 usage_receipt_json：
    /// result.usage.agentResponse + artifactIdentity（与 artifact 行同口径）。
    fn completed_usage(visible_text: &str, tool_calls: Value) -> Value {
        let mut usage = json!({
            "purpose": "agent-round",
            "artifactIdentity": {
                "artifactId": "artifact-fixed",
                "epoch": "epoch-1",
                "fingerprint": test_fingerprint(),
            },
            "result": {
                "finishReason": "stop",
                "usage": {
                    "policy": {
                        "estimatorVersion": "v1",
                        "safetyMarginTokens": 10,
                        "reasoning": "conservative",
                        "canBoundTotalLiability": true,
                    },
                    "trusted": true,
                    "agentResponse": {
                        "version": 1,
                        "visibleText": visible_text,
                        "toolCalls": tool_calls,
                    },
                },
            },
        });
        if let Some(object) = usage.as_object_mut() {
            if let Some(result) = object.get_mut("result") {
                if let Some(result_object) = result.as_object_mut() {
                    if let Some(inner) = result_object.get_mut("usage") {
                        if let Some(inner_object) = inner.as_object_mut() {
                            inner_object.insert("inputTokens".to_string(), Value::from(1));
                        }
                    }
                }
            }
        }
        usage
    }

    fn agent_generation(conn: &Connection) -> AgentGeneration<'_> {
        AgentGeneration::new(
            conn,
            GenerationRunRepository::default(),
            "project-1",
            "epoch-1",
            &assert_current,
            &TestHost,
        )
        .expect("construct AgentGeneration")
    }

    fn handle_of(run_id: &str) -> Value {
        json!({
            "projectId": "project-1",
            "epoch": "epoch-1",
            "rootActionId": "root-1",
            "runId": run_id,
        })
    }

    // ── 构造期恢复扫描 ─────────────────────────────────────────────────

    #[test]
    fn constructor_recovery_scan_repairs_dangling_claims() {
        let conn = test_connection();
        let session_key = text_hash("recovery-session");
        // run A：running + domainEffect → completed + 固定观察。
        let usage_a = json!({
            "agentActions": {
                "agent-tool:attempt-a:0": {
                    "status": "running",
                    "domainEffect": { "kind": "config", "toolCallId": "t" },
                },
                "agent-tool:attempt-a:1": { "status": "running" },
                "agent-tool:attempt-a:2": { "status": "completed" },
            },
        });
        seed_agent_run(
            &conn,
            "root-a",
            "run-a",
            &agent_binding(&test_context(), &session_key, 0, None),
            Some(AttemptFixture {
                attempt_id: "attempt-a".to_string(),
                attempt_status: "settled",
                usage_receipt_json: usage_a,
                artifact: None,
            }),
        );
        // 非 agent-round run：running 认领不得被修复。
        let binding_non_agent = json!({
            "projectId": "project-1",
            "epoch": "epoch-1",
            "fingerprint": test_fingerprint(),
            "contextSnapshotId": "snapshot-1",
            "sourceManifest": { "operation": "chapter-draft" },
            "sourceRefs": [],
        });
        seed_agent_run(
            &conn,
            "root-b",
            "run-b",
            &binding_non_agent,
            Some(AttemptFixture {
                attempt_id: "attempt-b".to_string(),
                attempt_status: "settled",
                usage_receipt_json: json!({
                    "agentActions": { "agent-tool:attempt-b:0": { "status": "running" } },
                }),
                artifact: None,
            }),
        );

        // 构造期扫描执行（AgentGeneration::new 内）。
        let agent = agent_generation(&conn);
        let _ = agent;

        let stored: Value = conn
            .query_row(
                "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id='attempt-a'",
                [],
                |row| row.get(0),
            )
            .map(|json: String| serde_json::from_str(&json).unwrap())
            .unwrap();
        let actions = stored.get("agentActions").unwrap();
        // running + domainEffect → completed + 固定观察。
        assert_eq!(
            actions
                .get("agent-tool:attempt-a:0")
                .and_then(|s| s.get("status"))
                .and_then(Value::as_str),
            Some("completed")
        );
        assert_eq!(
            actions
                .get("agent-tool:attempt-a:0")
                .and_then(|s| s.get("observation"))
                .and_then(Value::as_str),
            Some("The confirmed application change was committed.")
        );
        // running 无 domainEffect → unknown。
        assert_eq!(
            actions
                .get("agent-tool:attempt-a:1")
                .and_then(|s| s.get("status"))
                .and_then(Value::as_str),
            Some("unknown")
        );
        // 既有 completed 不受影响。
        assert_eq!(
            actions
                .get("agent-tool:attempt-a:2")
                .and_then(|s| s.get("status"))
                .and_then(Value::as_str),
            Some("completed")
        );
        // 非 agent-round run 的 running 认领保持原状。
        let stored_b: Value = conn
            .query_row(
                "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id='attempt-b'",
                [],
                |row| row.get(0),
            )
            .map(|json: String| serde_json::from_str(&json).unwrap())
            .unwrap();
        assert_eq!(
            stored_b
                .get("agentActions")
                .and_then(|a| a.get("agent-tool:attempt-b:0"))
                .and_then(|s| s.get("status"))
                .and_then(Value::as_str),
            Some("running")
        );
    }

    // ── require_run 身份门禁 ───────────────────────────────────────────

    #[test]
    fn require_run_rejects_identity_mismatch() {
        let conn = test_connection();
        let session_key = text_hash("identity-session");
        seed_agent_run(
            &conn,
            "root-1",
            "run-1",
            &agent_binding(&test_context(), &session_key, 0, None),
            None,
        );
        let agent = agent_generation(&conn);

        // 合法 handle 通过。
        agent
            .require_run(&handle_of("run-1"))
            .expect("valid handle");
        // epoch 不匹配。
        let mismatched_epoch = json!({
            "projectId": "project-1",
            "epoch": "epoch-other",
            "rootActionId": "root-1",
            "runId": "run-1",
        });
        assert_eq!(
            agent.require_run(&mismatched_epoch).unwrap_err(),
            GENERATION_AGENT_IDENTITY_MISMATCH
        );
        // 额外键 → deep-equal 失败。
        let extra_key = json!({
            "projectId": "project-1",
            "epoch": "epoch-1",
            "rootActionId": "root-1",
            "runId": "run-1",
            "extra": 1,
        });
        assert_eq!(
            agent.require_run(&extra_key).unwrap_err(),
            GENERATION_AGENT_IDENTITY_MISMATCH
        );
        // 非 agent-round operation。
        let non_agent_binding = json!({
            "projectId": "project-1",
            "epoch": "epoch-1",
            "fingerprint": test_fingerprint(),
            "contextSnapshotId": "snapshot-1",
            "sourceManifest": { "operation": "chapter-draft" },
            "sourceRefs": [],
        });
        seed_agent_run(&conn, "root-2", "run-2", &non_agent_binding, None);
        assert_eq!(
            agent.require_run(&handle_of("run-2")).unwrap_err(),
            GENERATION_AGENT_IDENTITY_MISMATCH
        );
        // 畸形 handle（非对象）。
        assert_eq!(
            agent.require_run(&Value::Null).unwrap_err(),
            GENERATION_AGENT_IDENTITY_MISMATCH
        );
    }

    // ── navigation 校验 ──────────────────────────────────────────────────

    #[test]
    fn navigation_rejects_invalid_session() {
        let conn = test_connection();
        // key 非 64 位 hex。
        seed_agent_run(
            &conn,
            "root-1",
            "run-1",
            &agent_binding(&test_context(), "short-key", 0, None),
            None,
        );
        // roundIndex 越界（≥8）。
        seed_agent_run(
            &conn,
            "root-1",
            "run-2",
            &agent_binding(&test_context(), &text_hash("out-of-range"), 8, None),
            None,
        );
        // roundIndex 为负。
        seed_agent_run(
            &conn,
            "root-1",
            "run-3",
            &agent_binding(&test_context(), &text_hash("negative"), -1, None),
            None,
        );
        let agent = agent_generation(&conn);
        assert_eq!(
            agent.read(&handle_of("run-1")).unwrap_err(),
            GENERATION_AGENT_NAVIGATION_INVALID
        );
        assert_eq!(
            agent.read(&handle_of("run-2")).unwrap_err(),
            GENERATION_AGENT_NAVIGATION_INVALID
        );
        assert_eq!(
            agent.read(&handle_of("run-3")).unwrap_err(),
            GENERATION_AGENT_NAVIGATION_INVALID
        );
    }

    // ── read 轮次重建 + nextRound 计算 ──────────────────────────────────

    /// 构造一个会话：round 0 completed（含一个终态动作）+ round 1 not-started。
    fn seed_completed_session(conn: &Connection, session_key: &str) {
        let tool_call_id = "agent-tool:attempt-0:0";
        let usage = completed_usage(
            "visible-0",
            json!([{ "name": "propose_novel_config", "arguments": {} }]),
        )
        .as_object()
        .cloned()
        .map(|mut object| {
            object.insert(
                "agentActions".to_string(),
                json!({ tool_call_id: { "status": "completed" } }),
            );
            Value::Object(object)
        })
        .unwrap();
        seed_agent_run(
            conn,
            "root-1",
            "run-0",
            &agent_binding(&test_context(), session_key, 0, None),
            Some(AttemptFixture {
                attempt_id: "attempt-0".to_string(),
                attempt_status: "settled",
                usage_receipt_json: usage,
                artifact: Some(("visible-0".to_string(), 0)),
            }),
        );
        seed_agent_run(
            conn,
            "root-1",
            "run-1",
            &agent_binding(&test_context(), session_key, 1, None),
            None,
        );
    }

    #[test]
    fn read_reconstructs_rounds_and_next_round() {
        let conn = test_connection();
        let session_key = text_hash("read-session");
        seed_completed_session(&conn, &session_key);
        let agent = agent_generation(&conn);

        let recovery = agent.read(&handle_of("run-1")).expect("read recovery");
        // 轮次重建：round 0 completed / round 1 not-started。
        assert_eq!(recovery.rounds.len(), 2);
        assert_eq!(recovery.rounds[0].index, 0);
        assert_eq!(recovery.rounds[0].status, "completed");
        assert_eq!(recovery.rounds[0].visible_text, "visible-0");
        assert_eq!(recovery.rounds[0].actions.len(), 1);
        assert_eq!(recovery.rounds[0].actions[0].status, "completed");
        assert_eq!(recovery.rounds[1].index, 1);
        assert_eq!(recovery.rounds[1].status, "not-started");
        // nextRound：末轮 not-started → 其 index。
        assert_eq!(recovery.next_round, Some(1));
        // 读面元数据：modelId / context / sourceStatus / run 视图。
        assert_eq!(recovery.model_id, "model-1");
        assert_eq!(recovery.context.version, 1);
        assert_eq!(recovery.source_status, "current");
        assert_eq!(recovery.handle.run_id, "run-1");
        assert!(recovery.run.get("status").and_then(Value::as_str) == Some("active"));
        // protocolText = [visibleText, ...toolCalls.map(JSON.stringify)]。
        assert!(recovery.rounds[0]
            .protocol_text
            .starts_with("visible-0\n{\"name\":\"propose_novel_config\""));
    }

    #[test]
    fn read_next_round_null_when_actions_pending() {
        let conn = test_connection();
        let session_key = text_hash("pending-session");
        // round 0 completed 但动作仍 running → allDone=false → nextRound=null。
        let tool_call_id = "agent-tool:attempt-0:0";
        let usage = completed_usage(
            "visible-0",
            json!([{ "name": "propose_novel_config", "arguments": {} }]),
        )
        .as_object()
        .cloned()
        .map(|mut object| {
            object.insert(
                "agentActions".to_string(),
                json!({ tool_call_id: { "status": "pending" } }),
            );
            Value::Object(object)
        })
        .unwrap();
        seed_agent_run(
            &conn,
            "root-1",
            "run-0",
            &agent_binding(&test_context(), &session_key, 0, None),
            Some(AttemptFixture {
                attempt_id: "attempt-0".to_string(),
                attempt_status: "settled",
                usage_receipt_json: usage,
                artifact: Some(("visible-0".to_string(), 0)),
            }),
        );
        let agent = agent_generation(&conn);
        let recovery = agent.read(&handle_of("run-0")).expect("read recovery");
        assert_eq!(recovery.next_round, None);
        assert_eq!(recovery.rounds[0].actions[0].status, "pending");
    }

    #[test]
    fn read_next_round_increments_completed_session() {
        let conn = test_connection();
        let session_key = text_hash("increment-session");
        // 单轮会话：round 0 completed 且全部动作终态、index<7 → nextRound=1。
        let tool_call_id = "agent-tool:attempt-0:0";
        let usage = completed_usage(
            "visible-0",
            json!([{ "name": "propose_novel_config", "arguments": {} }]),
        )
        .as_object()
        .cloned()
        .map(|mut object| {
            object.insert(
                "agentActions".to_string(),
                json!({ tool_call_id: { "status": "completed" } }),
            );
            Value::Object(object)
        })
        .unwrap();
        seed_agent_run(
            &conn,
            "root-1",
            "run-0",
            &agent_binding(&test_context(), &session_key, 0, None),
            Some(AttemptFixture {
                attempt_id: "attempt-0".to_string(),
                attempt_status: "settled",
                usage_receipt_json: usage,
                artifact: Some(("visible-0".to_string(), 0)),
            }),
        );
        let agent = agent_generation(&conn);
        let recovery = agent.read(&handle_of("run-0")).expect("read recovery");
        assert_eq!(recovery.next_round, Some(1));
    }

    #[test]
    fn read_rejects_root_mismatch_and_round_gap() {
        let conn = test_connection();
        let session_key = text_hash("conflict-session");
        // 同会话键、不同 rootActionId → ROOT_MISMATCH。
        seed_agent_run(
            &conn,
            "root-1",
            "run-0",
            &agent_binding(&test_context(), &session_key, 0, None),
            None,
        );
        seed_agent_run(
            &conn,
            "root-2",
            "run-1",
            &agent_binding(&test_context(), &session_key, 1, None),
            None,
        );
        let agent = agent_generation(&conn);
        assert_eq!(
            agent
                .read(&json!({
                    "projectId": "project-1",
                    "epoch": "epoch-1",
                    "rootActionId": "root-2",
                    "runId": "run-1",
                }))
                .unwrap_err(),
            GENERATION_AGENT_ROOT_MISMATCH
        );

        // 轮次 index 不连续（0 与 2）→ ROUND_GAP。
        let conn = test_connection();
        let session_key = text_hash("gap-session");
        seed_agent_run(
            &conn,
            "root-1",
            "run-0",
            &agent_binding(&test_context(), &session_key, 0, None),
            None,
        );
        seed_agent_run(
            &conn,
            "root-1",
            "run-2",
            &agent_binding(&test_context(), &session_key, 2, None),
            None,
        );
        let agent = agent_generation(&conn);
        assert_eq!(
            agent.read(&handle_of("run-2")).unwrap_err(),
            GENERATION_AGENT_ROUND_GAP
        );
    }

    // ── round() 重建 ───────────────────────────────────────────────────

    #[test]
    fn round_rebuilds_not_started_and_completed() {
        let conn = test_connection();
        let session_key = text_hash("round-session");
        // not-started：无 attempt。
        seed_agent_run(
            &conn,
            "root-1",
            "run-0",
            &agent_binding(&test_context(), &session_key, 0, None),
            None,
        );
        let agent = agent_generation(&conn);
        let run = agent.require_run(&handle_of("run-0")).unwrap();
        let round = agent.round(&run).expect("round not-started");
        assert_eq!(round.index, 0);
        assert_eq!(round.status, "not-started");
        assert!(round.attempt_id.is_none());
        assert!(round.actions.is_empty());
        assert_eq!(round.protocol_text, "");

        // completed：toolCalls → actions，toolCallId 形态为
        // `agent-tool:${attemptId}:${callIndex}`。
        let tool_calls = json!([
            { "name": "propose_novel_config", "arguments": { "changes": {} } },
            { "name": "start_workflow", "arguments": { "workflow": "generate_draft", "chapter_number": 3 } },
        ]);
        let usage = completed_usage("visible-1", tool_calls);
        seed_agent_run(
            &conn,
            "root-1",
            "run-1",
            &agent_binding(&test_context(), &session_key, 1, None),
            Some(AttemptFixture {
                attempt_id: "attempt-1".to_string(),
                attempt_status: "settled",
                usage_receipt_json: usage,
                artifact: Some(("visible-1".to_string(), 0)),
            }),
        );
        let run = agent.require_run(&handle_of("run-1")).unwrap();
        let round = agent.round(&run).expect("round completed");
        assert_eq!(round.status, "completed");
        assert_eq!(round.visible_text, "visible-1");
        assert_eq!(round.attempt_id, Some("attempt-1".to_string()));
        assert_eq!(round.actions.len(), 2);
        assert_eq!(round.actions[0].ref_.tool_call_id, "agent-tool:attempt-1:0");
        assert_eq!(round.actions[0].name, "propose_novel_config");
        assert_eq!(round.actions[0].status, "pending");
        assert_eq!(round.actions[0].ref_.attempt_id, "attempt-1");
        assert_eq!(round.actions[1].ref_.tool_call_id, "agent-tool:attempt-1:1");
        assert_eq!(round.actions[1].name, "start_workflow");
        // protocolText：visibleText 与各 toolCall 的 JSON.stringify 以换行连接。
        assert_eq!(
            round.protocol_text,
            format!(
                "visible-1\n{}\n{}",
                r#"{"name":"propose_novel_config","arguments":{"changes":{}}}"#,
                r#"{"name":"start_workflow","arguments":{"chapter_number":3,"workflow":"generate_draft"}}"#
            )
        );
    }

    #[test]
    fn round_rebuilds_incomplete_and_unknown() {
        let conn = test_connection();
        let session_key = text_hash("incomplete-session");
        // finishReason 非 stop → incomplete。
        let usage = json!({
            "result": {
                "finishReason": "length",
                "usage": {
                    "policy": {
                        "estimatorVersion": "v1",
                        "safetyMarginTokens": 10,
                        "reasoning": "conservative",
                        "canBoundTotalLiability": true,
                    },
                    "trusted": true,
                    "agentResponse": {
                        "version": 1,
                        "visibleText": "partial",
                        "toolCalls": [],
                    },
                },
            },
        });
        seed_agent_run(
            &conn,
            "root-1",
            "run-0",
            &agent_binding(&test_context(), &session_key, 0, None),
            Some(AttemptFixture {
                attempt_id: "attempt-0".to_string(),
                attempt_status: "settled",
                usage_receipt_json: usage,
                artifact: Some(("partial".to_string(), 0)),
            }),
        );
        let agent = agent_generation(&conn);
        let run = agent.require_run(&handle_of("run-0")).unwrap();
        let round = agent.round(&run).expect("round incomplete");
        assert_eq!(round.status, "incomplete");
        assert_eq!(round.visible_text, "partial");
        assert!(round.actions.is_empty());

        // attempt 状态 reserved → unknown 轮。
        let conn = test_connection();
        let session_key = text_hash("reserved-session");
        seed_agent_run(
            &conn,
            "root-1",
            "run-0",
            &agent_binding(&test_context(), &session_key, 0, None),
            Some(AttemptFixture {
                attempt_id: "attempt-0".to_string(),
                attempt_status: "reserved",
                usage_receipt_json: json!({}),
                artifact: None,
            }),
        );
        let agent = agent_generation(&conn);
        let run = agent.require_run(&handle_of("run-0")).unwrap();
        let round = agent.round(&run).expect("round unknown");
        assert_eq!(round.status, "unknown");
    }

    #[test]
    fn round_rejects_ambiguous_attempts() {
        let conn = test_connection();
        let session_key = text_hash("ambiguous-session");
        let binding = agent_binding(&test_context(), &session_key, 0, None);
        seed_agent_run(
            &conn,
            "root-1",
            "run-0",
            &binding,
            Some(AttemptFixture {
                attempt_id: "attempt-0".to_string(),
                attempt_status: "settled",
                usage_receipt_json: completed_usage("visible-0", json!([])),
                artifact: None,
            }),
        );
        let attempt_json = json!({
            "attemptId": "attempt-1",
            "reservationId": "reservation-1",
            "rootActionId": "root-1",
            "status": "settled",
            "reservedTokens": 0,
            "requestedOutputTokens": 0,
        });
        conn.execute(
            "INSERT INTO generation_attempts (attempt_id,reservation_id,run_id,root_action_id,attempt_json,usage_receipt_json,invocation_nonce) VALUES (?,?,?,?,?,?,?)",
            ["attempt-1", "reservation-1", "run-0", "root-1", &attempt_json.to_string(), "{}", "invocation-1"],
        )
        .expect("insert second attempt");
        let agent = agent_generation(&conn);
        let run = agent.require_run(&handle_of("run-0")).unwrap();
        assert_eq!(
            agent.round(&run).unwrap_err(),
            GENERATION_AGENT_ROUND_AMBIGUOUS
        );
    }

    #[test]
    fn round_normalizes_start_workflow_planning_arguments() {
        let conn = test_connection();
        // project_core 主行：total_chapters=10。
        conn.execute(
            "INSERT INTO project_core (id,total_chapters) VALUES ('main',10)",
            [],
        )
        .expect("insert project_core");
        let session_key = text_hash("planning-session");
        let tool_calls = json!([
            {
                "name": "start_workflow",
                "arguments": { "workflow": "generate_blueprint", "start_chapter": 2, "chapter_count": 3, "target_units": 700 },
            },
        ]);
        let usage = completed_usage("visible-0", tool_calls);
        seed_agent_run(
            &conn,
            "root-1",
            "run-0",
            &agent_binding(&test_context(), &session_key, 0, None),
            Some(AttemptFixture {
                attempt_id: "attempt-0".to_string(),
                attempt_status: "settled",
                usage_receipt_json: usage,
                artifact: Some(("visible-0".to_string(), 0)),
            }),
        );
        let agent = agent_generation(&conn);
        let run = agent.require_run(&handle_of("run-0")).unwrap();
        let round = agent.round(&run).expect("round completed");
        assert_eq!(round.actions.len(), 1);
        // 归一化参数进入 action.arguments（planningArguments 优先于原始参数）。
        let arguments = &round.actions[0].arguments;
        assert_eq!(
            arguments.get("start_chapter").and_then(Value::as_i64),
            Some(2)
        );
        assert_eq!(
            arguments.get("chapter_count").and_then(Value::as_i64),
            Some(3)
        );
        assert_eq!(
            arguments.get("target_units").and_then(Value::as_i64),
            Some(700)
        );
        // 归一化结果持久化到 usage_receipt_json.agentActions。
        let stored: Value = conn
            .query_row(
                "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id='attempt-0'",
                [],
                |row| row.get(0),
            )
            .map(|json: String| serde_json::from_str(&json).unwrap())
            .unwrap();
        let state = stored
            .get("agentActions")
            .and_then(|a| a.get("agent-tool:attempt-0:0"))
            .unwrap();
        assert_eq!(
            state
                .get("planningArguments")
                .and_then(|p| p.get("target_units"))
                .and_then(Value::as_i64),
            Some(700)
        );
    }

    // ── normalizeAgentPlanningArguments 纯函数 ────────────────────────

    #[test]
    fn normalize_agent_planning_arguments_port() {
        // 非规划 workflow：原样返回。
        let args = json!({ "workflow": "generate_draft" })
            .as_object()
            .cloned()
            .unwrap();
        assert_eq!(normalize_agent_planning_arguments(&args, 10).unwrap(), args);
        // 显式区间 + target_units。
        let args = json!({
            "workflow": "generate_blueprint",
            "start_chapter": 2,
            "chapter_count": 3,
            "target_units": 700,
        })
        .as_object()
        .cloned()
        .unwrap();
        let normalized = normalize_agent_planning_arguments(&args, 10).unwrap();
        assert_eq!(
            normalized.get("start_chapter").and_then(Value::as_i64),
            Some(2)
        );
        assert_eq!(
            normalized.get("chapter_count").and_then(Value::as_i64),
            Some(3)
        );
        assert_eq!(
            normalized.get("target_units").and_then(Value::as_i64),
            Some(700)
        );
        // 缺省：start_chapter=1、chapter_count=min(5, total-from+1)、target_units=600。
        let args = json!({ "workflow": "generate_architecture" })
            .as_object()
            .cloned()
            .unwrap();
        let normalized = normalize_agent_planning_arguments(&args, 10).unwrap();
        assert_eq!(
            normalized.get("start_chapter").and_then(Value::as_i64),
            Some(1)
        );
        assert_eq!(
            normalized.get("chapter_count").and_then(Value::as_i64),
            Some(5)
        );
        assert_eq!(
            normalized.get("target_units").and_then(Value::as_i64),
            Some(600)
        );
        // 跨度超限（>10 章）→ RANGE_INVALID。
        let args = json!({
            "workflow": "generate_blueprint",
            "start_chapter": 1,
            "chapter_count": 11,
        })
        .as_object()
        .cloned()
        .unwrap();
        assert_eq!(
            normalize_agent_planning_arguments(&args, 100).unwrap_err(),
            GENERATION_PLANNING_RANGE_INVALID
        );
        // to > totalChapters → RANGE_INVALID。
        let args = json!({
            "workflow": "generate_blueprint",
            "start_chapter": 8,
            "chapter_count": 5,
        })
        .as_object()
        .cloned()
        .unwrap();
        assert_eq!(
            normalize_agent_planning_arguments(&args, 10).unwrap_err(),
            GENERATION_PLANNING_RANGE_INVALID
        );
        // count < 1 → RANGE_INVALID。
        let args = json!({
            "workflow": "generate_blueprint",
            "chapter_count": 0,
        })
        .as_object()
        .cloned()
        .unwrap();
        assert_eq!(
            normalize_agent_planning_arguments(&args, 10).unwrap_err(),
            GENERATION_PLANNING_RANGE_INVALID
        );
        // target_units 越界 → TARGET_INVALID。
        let args = json!({
            "workflow": "generate_blueprint",
            "target_units": 1001,
        })
        .as_object()
        .cloned()
        .unwrap();
        assert_eq!(
            normalize_agent_planning_arguments(&args, 10).unwrap_err(),
            GENERATION_PLANNING_TARGET_INVALID
        );
    }

    // ── begin 幂等恢复 ───────────────────────────────────────────────────

    #[test]
    fn begin_validates_request_and_recovers_by_nonce() {
        let conn = test_connection();
        let agent = agent_generation(&conn);
        // 键集不精确 → BEGIN_INVALID。
        let request =
            json!({ "uiActionNonce": "n1", "modelId": "model-1", "input": {}, "extra": 1 });
        assert_eq!(
            agent.begin(&request).unwrap_err(),
            GENERATION_AGENT_BEGIN_INVALID
        );
        // nonce 空 / 超长 → BEGIN_INVALID。
        let request = json!({ "uiActionNonce": "  ", "modelId": "model-1", "input": {} });
        assert_eq!(
            agent.begin(&request).unwrap_err(),
            GENERATION_AGENT_BEGIN_INVALID
        );
        let request = json!({
            "uiActionNonce": "n".repeat(257),
            "modelId": "model-1",
            "input": {},
        });
        assert_eq!(
            agent.begin(&request).unwrap_err(),
            GENERATION_AGENT_BEGIN_INVALID
        );
        // modelId 空 → BEGIN_INVALID。
        let request = json!({ "uiActionNonce": "n1", "modelId": "", "input": {} });
        assert_eq!(
            agent.begin(&request).unwrap_err(),
            GENERATION_AGENT_BEGIN_INVALID
        );
        // input 校验失败 → INPUT_INVALID（agent_generation_context 移植面）。
        let request =
            json!({ "uiActionNonce": "n1", "modelId": "model-1", "input": { "mode": "fast" } });
        assert_eq!(
            agent.begin(&request).unwrap_err(),
            "GENERATION_AGENT_INPUT_INVALID"
        );
    }

    #[test]
    fn begin_opens_round_zero_and_recovers_idempotently() {
        let conn = test_connection();
        let agent = agent_generation(&conn);
        let input = serde_json::to_value(test_context().input).unwrap();
        let request = json!({
            "uiActionNonce": "begin-nonce-1",
            "modelId": "model-1",
            "input": input,
        });
        // 首次 begin：host.begin 开第 0 轮 run → read 重建。
        let recovery = agent.begin(&request).expect("first begin");
        assert_eq!(recovery.rounds.len(), 1);
        assert_eq!(recovery.rounds[0].index, 0);
        assert_eq!(recovery.rounds[0].status, "not-started");
        assert_eq!(recovery.next_round, Some(0));
        assert_eq!(recovery.model_id, "model-1");
        // 派生 session key = textHash(JSON.stringify([projectId, nonce]))。
        let session_key =
            text_hash(&serde_json::to_string(&json!(["project-1", "begin-nonce-1"])).unwrap());
        assert_eq!(
            recovery.rounds[0].handle.run_id,
            format!("host-begin-run-{session_key}-0")
        );
        // 二次 begin：同一 nonce + 同一 input + 同一 modelId → 幂等命中既有 session。
        let recovery_again = agent.begin(&request).expect("second begin");
        assert_eq!(recovery_again.rounds.len(), 1);
        assert_eq!(
            recovery_again.rounds[0].handle.run_id,
            recovery.rounds[0].handle.run_id
        );
        // 冲突：同一 nonce 但 modelId 不同 → NONCE_CONFLICT。
        let request = json!({
            "uiActionNonce": "begin-nonce-1",
            "modelId": "model-2",
            "input": input,
        });
        assert_eq!(
            agent.begin(&request).unwrap_err(),
            GENERATION_AGENT_NONCE_CONFLICT
        );
    }

    // ── claim / finish / register-workflow 状态机 ──────────────────────

    /// 构造一个 pending 工具动作的 completed 轮次（含 propose_novel_config 调用）。
    fn seed_pending_tool_session(conn: &Connection, session_key: &str) -> Value {
        let tool_calls =
            json!([{ "name": "propose_novel_config", "arguments": { "changes": {} } }]);
        let usage = completed_usage("visible-0", tool_calls);
        seed_agent_run(
            conn,
            "root-1",
            "run-0",
            &agent_binding(&test_context(), session_key, 0, None),
            Some(AttemptFixture {
                attempt_id: "attempt-0".to_string(),
                attempt_status: "settled",
                usage_receipt_json: usage,
                artifact: Some(("visible-0".to_string(), 0)),
            }),
        );
        handle_of("run-0")
    }

    fn tool_ref(handle: &Value) -> Value {
        json!({
            "handle": handle,
            "attemptId": "attempt-0",
            "toolCallId": "agent-tool:attempt-0:0",
        })
    }

    #[test]
    fn claim_tool_requires_boolean_confirmation() {
        let conn = test_connection();
        let session_key = text_hash("claim-session");
        let handle = seed_pending_tool_session(&conn, &session_key);
        let agent = agent_generation(&conn);
        let request = json!({ "ref": tool_ref(&handle), "confirmed": "yes" });
        assert_eq!(
            agent.claim_tool(&request).unwrap_err(),
            GENERATION_AGENT_CONFIRMATION_REQUIRED
        );
    }

    #[test]
    fn claim_tool_transitions_pending_to_running_or_declined() {
        let conn = test_connection();
        let session_key = text_hash("claim-session-2");
        let handle = seed_pending_tool_session(&conn, &session_key);
        let agent = agent_generation(&conn);
        // confirmed=true → running + execute=true。
        let request = json!({ "ref": tool_ref(&handle), "confirmed": true });
        let claim = agent.claim_tool(&request).expect("claim running");
        assert_eq!(claim.action.status, "running");
        assert!(claim.execute);
        // 幂等：非 pending 直接返回当前态。
        let claim_again = agent.claim_tool(&request).expect("claim idempotent");
        assert_eq!(claim_again.action.status, "running");
        assert!(!claim_again.execute);

        // declined 路径（独立会话）。
        let conn = test_connection();
        let session_key = text_hash("claim-session-3");
        let handle = seed_pending_tool_session(&conn, &session_key);
        let agent = agent_generation(&conn);
        let request = json!({ "ref": tool_ref(&handle), "confirmed": false });
        let claim = agent.claim_tool(&request).expect("claim declined");
        assert_eq!(claim.action.status, "declined");
        assert_eq!(
            claim.action.observation.as_deref(),
            Some("The author declined this action.")
        );
        assert!(!claim.execute);
    }

    #[test]
    fn finish_tool_writes_result_and_detects_conflicts() {
        let conn = test_connection();
        let session_key = text_hash("finish-session");
        let handle = seed_pending_tool_session(&conn, &session_key);
        let agent = agent_generation(&conn);
        // 先认领为 running。
        let request = json!({ "ref": tool_ref(&handle), "confirmed": true });
        agent.claim_tool(&request).expect("claim");
        // 形态非法 → TOOL_RESULT_INVALID。
        let request = json!({ "ref": tool_ref(&handle), "status": "done", "observation": "o" });
        assert_eq!(
            agent.finish_tool(&request).unwrap_err(),
            GENERATION_AGENT_TOOL_RESULT_INVALID
        );
        // 合法完成：propose_novel_config completed 但无 domainEffect →
        // DOMAIN_EFFECT_REQUIRED。
        let request =
            json!({ "ref": tool_ref(&handle), "status": "completed", "observation": "o" });
        assert_eq!(
            agent.finish_tool(&request).unwrap_err(),
            GENERATION_AGENT_DOMAIN_EFFECT_REQUIRED
        );
        // failed 无域效应要求 → 写回成功。
        let request =
            json!({ "ref": tool_ref(&handle), "status": "failed", "observation": "失败原因" });
        let action = agent.finish_tool(&request).expect("finish failed");
        assert_eq!(action.status, "failed");
        assert_eq!(action.observation.as_deref(), Some("失败原因"));
        // 幂等重放：同 status 同 observation → 直接返回。
        let action_again = agent.finish_tool(&request).expect("finish replay");
        assert_eq!(action_again.status, "failed");
        // 冲突：同 status 不同 observation → TOOL_RESULT_CONFLICT。
        let request =
            json!({ "ref": tool_ref(&handle), "status": "failed", "observation": "其他" });
        assert_eq!(
            agent.finish_tool(&request).unwrap_err(),
            GENERATION_AGENT_TOOL_RESULT_CONFLICT
        );
    }

    #[test]
    fn register_workflow_registers_start_workflow() {
        let conn = test_connection();
        let session_key = text_hash("workflow-session");
        // 先构造 AgentGeneration（构造期恢复扫描时 DB 为空）——
        // 基线恢复扫描只修复构造时已存在的 running 认领，fixture
        // 随后插入的 running 态保留（门禁要求 action.status='running'）。
        let agent = agent_generation(&conn);
        conn.execute(
            "INSERT INTO project_core (id,total_chapters) VALUES ('main',10)",
            [],
        )
        .expect("insert project_core");
        let tool_calls = json!([
            {
                "name": "start_workflow",
                "arguments": {
                    "workflow": "generate_blueprint",
                    "start_chapter": 2,
                    "chapter_count": 3,
                    "target_units": 700,
                },
            },
        ]);
        let usage = completed_usage("visible-0", tool_calls)
            .as_object()
            .cloned()
            .map(|mut object| {
                object.insert(
                    "agentActions".to_string(),
                    json!({
                        "agent-tool:attempt-0:0": { "status": "running" },
                    }),
                );
                Value::Object(object)
            })
            .unwrap();
        seed_agent_run(
            &conn,
            "root-1",
            "run-0",
            &agent_binding(&test_context(), &session_key, 0, None),
            Some(AttemptFixture {
                attempt_id: "attempt-0".to_string(),
                attempt_status: "settled",
                usage_receipt_json: usage,
                artifact: Some(("visible-0".to_string(), 0)),
            }),
        );
        let handle = handle_of("run-0");
        let ref_ = tool_ref(&handle);
        let registration = agent.register_workflow(&ref_).expect("register workflow");
        assert_eq!(registration.workflow, "generate_blueprint");
        assert_eq!(registration.state, "registered");
        assert!(registration.registration_id.starts_with("agent-workflow:"));
        assert_eq!(
            registration.registration_id.len(),
            "agent-workflow:".len() + 64
        );
        assert_eq!(
            registration
                .planning
                .as_ref()
                .map(|p| (p.from, p.to, p.target_units)),
            Some((2, 4, 700))
        );
        assert_eq!(registration.model_id, "model-1");
        assert!(registration.child_handles.is_empty());
        // 重建后 action.workflow 已持久化。
        let run = agent.require_run(&handle).unwrap();
        let round = agent.round(&run).unwrap();
        assert_eq!(round.actions[0].status, "running");
        assert!(round.actions[0].workflow.is_some());
        // workflowRegistration 全表扫描定位。
        let (action, run, found) = agent
            .workflow_registration(&registration.registration_id)
            .expect("workflow registration lookup");
        assert_eq!(action.ref_.tool_call_id, "agent-tool:attempt-0:0");
        assert_eq!(run.run_id, "run-0");
        assert_eq!(found.registration_id, registration.registration_id);
        // 非法 registrationId 格式。
        assert_eq!(
            agent
                .workflow_registration("agent-workflow:short")
                .unwrap_err(),
            GENERATION_AGENT_REGISTRATION_INVALID
        );
        // 找不到 → REGISTRATION_MISSING。
        let missing = format!("agent-workflow:{}", text_hash("nonexistent"));
        assert_eq!(
            agent.workflow_registration(&missing).unwrap_err(),
            GENERATION_AGENT_REGISTRATION_MISSING
        );
    }

    // ── with_child_effect / record_workflow_child ──────────────────────

    /// 构造带 workflow registration 的会话：round 0 的 start_workflow 已
    /// 注册（state=registered），并登记子代 handle。
    fn seed_registered_session(conn: &Connection) -> (Value, String, MainGenerationRunHandle) {
        let session_key = text_hash("child-effect-session");
        let registration_id = format!("agent-workflow:{}", text_hash("agent-tool:attempt-0:0"));
        let registration = AgentWorkflowRegistration {
            planning: Some(AgentWorkflowPlanning {
                from: 2,
                to: 4,
                target_units: 700,
            }),
            registration_id: registration_id.clone(),
            workflow: "generate_blueprint".to_string(),
            chapter_number: None,
            parent_handle: MainGenerationRunHandle {
                project_id: "project-1".to_string(),
                epoch: "epoch-1".to_string(),
                root_action_id: "root-1".to_string(),
                run_id: "run-0".to_string(),
            },
            model_id: "model-1".to_string(),
            state: "registered".to_string(),
            child_handles: vec![MainGenerationRunHandle {
                project_id: "project-1".to_string(),
                epoch: "epoch-1".to_string(),
                root_action_id: "root-1".to_string(),
                run_id: "child-0".to_string(),
            }],
        };
        let tool_calls = json!([
            {
                "name": "start_workflow",
                "arguments": {
                    "workflow": "generate_blueprint",
                    "start_chapter": 2,
                    "chapter_count": 3,
                    "target_units": 700,
                },
            },
        ]);
        let usage = completed_usage("visible-0", tool_calls)
            .as_object()
            .cloned()
            .map(|mut object| {
                object.insert(
                    "agentActions".to_string(),
                    json!({
                        "agent-tool:attempt-0:0": {
                            "status": "completed",
                            "workflow": serde_json::to_value(&registration).unwrap(),
                        },
                    }),
                );
                Value::Object(object)
            })
            .unwrap();
        seed_agent_run(
            conn,
            "root-1",
            "run-0",
            &agent_binding(&test_context(), &session_key, 0, None),
            Some(AttemptFixture {
                attempt_id: "attempt-0".to_string(),
                attempt_status: "settled",
                usage_receipt_json: usage,
                artifact: Some(("visible-0".to_string(), 0)),
            }),
        );
        // 子代 run（agent-round，登记 agentWorkflowRegistrationId）。
        let child_binding = agent_binding(
            &test_context(),
            &text_hash("child-effect-session-child"),
            0,
            Some(
                vec![(
                    "agentWorkflowRegistrationId".to_string(),
                    Value::String(registration_id.clone()),
                )]
                .into_iter()
                .collect(),
            ),
        );
        seed_agent_run(conn, "root-1", "child-0", &child_binding, None);
        (
            handle_of("run-0"),
            registration_id,
            registration.child_handles[0].clone(),
        )
    }

    #[test]
    fn with_child_effect_captures_source_transitions() {
        let conn = test_connection();
        let (_handle, _registration_id, child) = seed_registered_session(&conn);
        let agent = agent_generation(&conn);
        // 非 agent 子代（无 registrationId）→ 效应透传。
        let plain = handle_of("run-0");
        let result = agent
            .with_child_effect(&plain, |_conn| Ok(42))
            .expect("plain effect passthrough");
        assert_eq!(result, 42);
        // agent 子代：效应前后捕获并写回 agentSourceTransition。
        let result = agent
            .with_child_effect(&serde_json::to_value(&child).unwrap(), |conn| {
                // 效应内可读到外层环境事务（本测试无外层事务，autocommit）。
                let _ = conn;
                Ok("effect-result")
            })
            .expect("child effect");
        assert_eq!(result, "effect-result");
        // 会话全部轮次（本会话仅 run-0）attempt 写回 agentSourceTransition。
        let stored: Value = conn
            .query_row(
                "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id='attempt-0'",
                [],
                |row| row.get(0),
            )
            .map(|json: String| serde_json::from_str(&json).unwrap())
            .unwrap();
        let transition = stored
            .get("agentSourceTransition")
            .expect("transition written");
        assert_eq!(
            transition.get("actionId").and_then(Value::as_str),
            Some("agent-tool:attempt-0:0:child-0")
        );
        let before_hash = transition
            .get("beforeHash")
            .and_then(Value::as_str)
            .unwrap();
        let after_hash = transition.get("afterHash").and_then(Value::as_str).unwrap();
        assert_eq!(before_hash.len(), 64);
        assert_eq!(after_hash.len(), 64);
        assert!(transition.get("binding").is_some());
        // registrationId 校验：childHandles 不含该子代时 SOURCE_CHANGED。
        let conn = test_connection();
        let (handle, registration_id, _child) = seed_registered_session(&conn);
        let agent = agent_generation(&conn);
        // 篡改：把 registration 的 childHandles 清空（直接改 run-0 的 usage）。
        let stored: String = conn
            .query_row(
                "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id='attempt-0'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let mut usage: Value = serde_json::from_str(&stored).unwrap();
        if let Some(state) = usage
            .get_mut("agentActions")
            .and_then(|a| a.get_mut("agent-tool:attempt-0:0"))
            .and_then(|s| s.get_mut("workflow"))
        {
            state
                .as_object_mut()
                .unwrap()
                .insert("childHandles".to_string(), json!([]));
        }
        conn.execute(
            "UPDATE generation_attempts SET usage_receipt_json=? WHERE attempt_id='attempt-0'",
            [serde_json::to_string(&usage).unwrap()],
        )
        .unwrap();
        let orphan = json!({
            "projectId": "project-1",
            "epoch": "epoch-1",
            "rootActionId": "root-1",
            "runId": "child-0",
        });
        assert_eq!(
            agent
                .with_child_effect(&orphan, |_conn| Ok(()))
                .unwrap_err(),
            GENERATION_AGENT_SOURCE_CHANGED
        );
        let _ = registration_id;
        let _ = handle;
    }

    #[test]
    fn record_workflow_child_starts_registration() {
        let conn = test_connection();
        let (handle, registration_id, child) = seed_registered_session(&conn);
        let agent = agent_generation(&conn);
        agent
            .record_workflow_child(&registration_id, &serde_json::to_value(&child).unwrap())
            .expect("record workflow child");
        // registration 态推进到 started，子代 handle 已登记（幂等）。
        let stored: Value = conn
            .query_row(
                "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id='attempt-0'",
                [],
                |row| row.get(0),
            )
            .map(|json: String| serde_json::from_str(&json).unwrap())
            .unwrap();
        let workflow = stored
            .get("agentActions")
            .and_then(|a| a.get("agent-tool:attempt-0:0"))
            .and_then(|s| s.get("workflow"))
            .unwrap();
        assert_eq!(
            workflow.get("state").and_then(Value::as_str),
            Some("started")
        );
        let child_handles = workflow
            .get("childHandles")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(child_handles.len(), 1);
        assert_eq!(
            child_handles[0].get("runId").and_then(Value::as_str),
            Some("child-0")
        );
        // 幂等：重复登记不重复追加。
        agent
            .record_workflow_child(&registration_id, &serde_json::to_value(&child).unwrap())
            .expect("record workflow child idempotent");
        let stored: Value = conn
            .query_row(
                "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id='attempt-0'",
                [],
                |row| row.get(0),
            )
            .map(|json: String| serde_json::from_str(&json).unwrap())
            .unwrap();
        let child_handles = stored
            .get("agentActions")
            .and_then(|a| a.get("agent-tool:attempt-0:0"))
            .and_then(|s| s.get("workflow"))
            .and_then(|w| w.get("childHandles"))
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(child_handles.len(), 1);
        let _ = handle;
    }

    // ── admit_workflow（子波次 B：owner.begin 子代准入）──────────

    /// admit_workflow fixture：registration_id 采用基线
    /// `action_id` 口径（`agent-tool:attempt-0:0`）；`seed`
    /// 参数使各用例的 registrationId 互不冲突（全表扫描
    /// 定位要求唯一）。
    fn admit_registration(
        seed: &str,
        child_run_ids: &[&str],
    ) -> (String, AgentWorkflowRegistration) {
        let registration_id = format!("agent-workflow:{}", text_hash(seed));
        let registration = AgentWorkflowRegistration {
            planning: Some(AgentWorkflowPlanning {
                from: 2,
                to: 4,
                target_units: 700,
            }),
            registration_id: registration_id.clone(),
            workflow: "generate_blueprint".to_string(),
            chapter_number: None,
            parent_handle: MainGenerationRunHandle {
                project_id: "project-1".to_string(),
                epoch: "epoch-1".to_string(),
                root_action_id: "root-1".to_string(),
                run_id: "run-0".to_string(),
            },
            model_id: "model-1".to_string(),
            state: "registered".to_string(),
            child_handles: child_run_ids
                .iter()
                .map(|run_id| MainGenerationRunHandle {
                    project_id: "project-1".to_string(),
                    epoch: "epoch-1".to_string(),
                    root_action_id: "root-1".to_string(),
                    run_id: run_id.to_string(),
                })
                .collect(),
        };
        (registration_id, registration)
    }

    /// 构造 run-0：attempt 的 agentActions 携带 registration
    /// （action status 可控——门禁①的 fixture；`attempt_id`
    /// 参数使同库多 run 播种的 reservation_id 不冲突）。
    fn seed_workflow_run(
        conn: &Connection,
        registration: &AgentWorkflowRegistration,
        action_status: &str,
        attempt_id: &str,
    ) {
        let session_key = text_hash("admit-session");
        let tool_calls = json!([
            {
                "name": "start_workflow",
                "arguments": {
                    "workflow": registration.workflow,
                    "start_chapter": 2,
                    "chapter_count": 3,
                    "target_units": 700,
                },
            },
        ]);
        let usage = completed_usage("visible-0", tool_calls)
            .as_object()
            .cloned()
            .map(|mut object| {
                object.insert(
                    "agentActions".to_string(),
                    json!({
                        &format!("agent-tool:{attempt_id}:0"): {
                            "status": action_status,
                            "workflow": serde_json::to_value(registration).unwrap(),
                        },
                    }),
                );
                Value::Object(object)
            })
            .unwrap();
        seed_agent_run(
            conn,
            &registration.parent_handle.root_action_id,
            &registration.parent_handle.run_id,
            &agent_binding(&test_context(), &session_key, 0, None),
            Some(AttemptFixture {
                attempt_id: attempt_id.to_string(),
                attempt_status: "settled",
                usage_receipt_json: usage,
                artifact: Some(("visible-0".to_string(), 0)),
            }),
        );
    }

    /// 合法选择（generate_blueprint → chapter-blueprint-directory；
    /// planning 输入与 registration.planning {from:2, to:4,
    /// targetUnits:700} 逐字段一致）。
    fn admit_selection(registration_id: &str) -> Value {
        json!({
            "operation": "chapter-blueprint-directory",
            "modelId": "model-1",
            "parentRootActionId": "root-1",
            "agentWorkflowRegistrationId": registration_id,
            "authorInputs": [
                { "id": "directory:requested-range", "text": "{\"startChapter\":2,\"endChapter\":4}" },
                { "id": "planning:target-units", "text": "700" },
            ],
        })
    }

    /// 子代 run fixture：manifest 覆盖 operation / chapterNumber
    /// （existing 幂等定位的过滤键）并登记 registrationId。
    fn seed_admit_child(
        conn: &Connection,
        run_id: &str,
        operation: &str,
        chapter_number: Option<i64>,
        registration_id: &str,
    ) {
        let mut extra = Map::new();
        extra.insert(
            "operation".to_string(),
            Value::String(operation.to_string()),
        );
        if let Some(chapter_number) = chapter_number {
            extra.insert(
                "chapterNumber".to_string(),
                Value::Number(chapter_number.into()),
            );
        }
        extra.insert(
            "agentWorkflowRegistrationId".to_string(),
            Value::String(registration_id.to_string()),
        );
        seed_agent_run(
            conn,
            "root-1",
            run_id,
            &agent_binding(&test_context(), &text_hash(run_id), 0, Some(extra)),
            None,
        );
    }

    #[test]
    fn admit_workflow_without_registration_id_passthrough() {
        let conn = test_connection();
        let agent = agent_generation(&conn);
        // 无 registrationId、无父根 → 透传 Ok(None)。
        assert_eq!(agent.admit_workflow(&json!({})).expect("passthrough"), None);
        // 父根为 agent-round → GENERATION_AGENT_REGISTRATION_REQUIRED。
        let session_key = text_hash("admit-no-registration");
        seed_agent_run(
            &conn,
            "root-agent",
            "run-agent",
            &agent_binding(&test_context(), &session_key, 0, None),
            None,
        );
        assert_eq!(
            agent
                .admit_workflow(&json!({ "parentRootActionId": "root-agent" }))
                .unwrap_err(),
            GENERATION_AGENT_REGISTRATION_REQUIRED
        );
        // 父根为非 agent 操作 → 透传 Ok(None)。
        conn.execute(
            "UPDATE generation_roots SET action_json = json_set(action_json,'$.operation','chapter-draft') WHERE root_action_id='root-agent'",
            [],
        )
        .expect("reparent root operation");
        assert_eq!(
            agent
                .admit_workflow(&json!({ "parentRootActionId": "root-agent" }))
                .expect("non-agent parent"),
            None
        );
    }

    #[test]
    fn admit_workflow_registration_missing() {
        let conn = test_connection();
        let agent = agent_generation(&conn);
        // registrationId 不存在。
        let missing = format!("agent-workflow:{}", text_hash("nonexistent"));
        assert_eq!(
            agent
                .admit_workflow(&admit_selection(&missing))
                .unwrap_err(),
            GENERATION_AGENT_REGISTRATION_MISSING
        );
        // registrationId 重复（两个 agent-round run 携带同一 id）。
        let (registration_id, registration) = admit_registration("duplicate", &[]);
        let mut duplicate = registration.clone();
        duplicate.parent_handle = MainGenerationRunHandle {
            project_id: "project-1".to_string(),
            epoch: "epoch-1".to_string(),
            root_action_id: "root-2".to_string(),
            run_id: "run-1".to_string(),
        };
        seed_workflow_run(&conn, &registration, "completed", "attempt-0");
        seed_workflow_run(&conn, &duplicate, "completed", "attempt-1");
        assert_eq!(
            agent
                .admit_workflow(&admit_selection(&registration_id))
                .unwrap_err(),
            GENERATION_AGENT_REGISTRATION_MISSING
        );
    }

    #[test]
    fn admit_workflow_gates_reject_unauthorized_child() {
        // 门禁①——action.status 非 running/completed。
        {
            let conn = test_connection();
            let agent = agent_generation(&conn);
            let (registration_id, registration) = admit_registration("gate-status", &[]);
            seed_workflow_run(&conn, &registration, "pending", "attempt-0");
            assert_eq!(
                agent
                    .admit_workflow(&admit_selection(&registration_id))
                    .unwrap_err(),
                GENERATION_AGENT_CHILD_NOT_AUTHORIZED
            );
        }
        // 门禁②——modelId 与 registration 不一致。
        {
            let conn = test_connection();
            let agent = agent_generation(&conn);
            let (registration_id, registration) = admit_registration("gate-model", &[]);
            seed_workflow_run(&conn, &registration, "completed", "attempt-0");
            let mut selection = admit_selection(&registration_id);
            selection["modelId"] = Value::String("model-2".to_string());
            assert_eq!(
                agent.admit_workflow(&selection).unwrap_err(),
                GENERATION_AGENT_CHILD_NOT_AUTHORIZED
            );
        }
        // 门禁③——parentRootActionId 与 registration 根不一致。
        {
            let conn = test_connection();
            let agent = agent_generation(&conn);
            let (registration_id, registration) = admit_registration("gate-parent", &[]);
            seed_workflow_run(&conn, &registration, "completed", "attempt-0");
            let mut selection = admit_selection(&registration_id);
            selection["parentRootActionId"] = Value::String("root-2".to_string());
            assert_eq!(
                agent.admit_workflow(&selection).unwrap_err(),
                GENERATION_AGENT_CHILD_NOT_AUTHORIZED
            );
        }
        // 门禁④——operation 不在 expectedOperations。
        {
            let conn = test_connection();
            let agent = agent_generation(&conn);
            let (registration_id, registration) = admit_registration("gate-operation", &[]);
            seed_workflow_run(&conn, &registration, "completed", "attempt-0");
            let mut selection = admit_selection(&registration_id);
            selection["operation"] = Value::String("chapter-draft".to_string());
            assert_eq!(
                agent.admit_workflow(&selection).unwrap_err(),
                GENERATION_AGENT_CHILD_NOT_AUTHORIZED
            );
        }
        // 门禁⑤——generate_draft 章节号与 registration 不一致。
        {
            let conn = test_connection();
            let agent = agent_generation(&conn);
            let (registration_id, mut registration) = admit_registration("gate-chapter", &[]);
            registration.workflow = "generate_draft".to_string();
            registration.chapter_number = Some(3);
            seed_workflow_run(&conn, &registration, "completed", "attempt-0");
            let mut selection = admit_selection(&registration_id);
            selection["operation"] = Value::String("chapter-draft".to_string());
            selection["chapterNumber"] = Value::Number(5.into());
            assert_eq!(
                agent.admit_workflow(&selection).unwrap_err(),
                GENERATION_AGENT_CHILD_NOT_AUTHORIZED
            );
        }
        // 门禁⑥——选择携带批量键（batchId / batchIntent /
        // continueDirectoryOperationId 任一）。
        for key in ["batchId", "batchIntent", "continueDirectoryOperationId"] {
            let conn = test_connection();
            let agent = agent_generation(&conn);
            let (registration_id, registration) = admit_registration("gate-batch", &[]);
            seed_workflow_run(&conn, &registration, "completed", "attempt-0");
            let mut selection = admit_selection(&registration_id);
            selection[key] = json!("batch-1");
            assert_eq!(
                agent.admit_workflow(&selection).unwrap_err(),
                GENERATION_AGENT_CHILD_NOT_AUTHORIZED
            );
        }
    }

    #[test]
    fn admit_workflow_planning_mismatch_rejected() {
        // 区间不匹配（endChapter 5 ≠ planning.to 4）。
        {
            let conn = test_connection();
            let agent = agent_generation(&conn);
            let (registration_id, registration) = admit_registration("planning-range", &[]);
            seed_workflow_run(&conn, &registration, "completed", "attempt-0");
            let mut selection = admit_selection(&registration_id);
            if let Some(inputs) = selection
                .get_mut("authorInputs")
                .and_then(Value::as_array_mut)
            {
                inputs[0] = json!({ "id": "directory:requested-range", "text": "{\"startChapter\":2,\"endChapter\":5}" });
            }
            assert_eq!(
                agent.admit_workflow(&selection).unwrap_err(),
                GENERATION_AGENT_CHILD_NOT_AUTHORIZED
            );
        }
        // 目标单位不匹配（600 ≠ planning.targetUnits 700）。
        {
            let conn = test_connection();
            let agent = agent_generation(&conn);
            let (registration_id, registration) = admit_registration("planning-units", &[]);
            seed_workflow_run(&conn, &registration, "completed", "attempt-0");
            let mut selection = admit_selection(&registration_id);
            if let Some(inputs) = selection
                .get_mut("authorInputs")
                .and_then(Value::as_array_mut)
            {
                inputs[1] = json!({ "id": "planning:target-units", "text": "600" });
            }
            assert_eq!(
                agent.admit_workflow(&selection).unwrap_err(),
                GENERATION_AGENT_CHILD_NOT_AUTHORIZED
            );
        }
        // planning 输入 JSON 解析失败。
        {
            let conn = test_connection();
            let agent = agent_generation(&conn);
            let (registration_id, registration) = admit_registration("planning-json", &[]);
            seed_workflow_run(&conn, &registration, "completed", "attempt-0");
            let mut selection = admit_selection(&registration_id);
            if let Some(inputs) = selection
                .get_mut("authorInputs")
                .and_then(Value::as_array_mut)
            {
                inputs[0] = json!({ "id": "directory:requested-range", "text": "not-json" });
            }
            assert_eq!(
                agent.admit_workflow(&selection).unwrap_err(),
                GENERATION_AGENT_CHILD_NOT_AUTHORIZED
            );
        }
    }

    #[test]
    fn admit_workflow_tool_stale_rejected() {
        let conn = test_connection();
        let agent = agent_generation(&conn);
        let (registration_id, registration) = admit_registration("tool-stale", &[]);
        seed_workflow_run(&conn, &registration, "completed", "attempt-0");
        // 根纪元与作用域不一致。
        conn.execute(
            "UPDATE generation_roots SET action_json = json_set(action_json,'$.epoch','epoch-2') WHERE root_action_id='root-1'",
            [],
        )
        .expect("update root epoch");
        assert_eq!(
            agent
                .admit_workflow(&admit_selection(&registration_id))
                .unwrap_err(),
            GENERATION_AGENT_TOOL_STALE
        );
        // 根已取消。
        conn.execute(
            "UPDATE generation_roots SET action_json = json_set(action_json,'$.epoch','epoch-1','$.status','cancelled') WHERE root_action_id='root-1'",
            [],
        )
        .expect("cancel root");
        assert_eq!(
            agent
                .admit_workflow(&admit_selection(&registration_id))
                .unwrap_err(),
            GENERATION_AGENT_TOOL_STALE
        );
    }

    /// 源一致性失败宿主（`current` 恒 false——触发
    /// `GENERATION_AGENT_SOURCE_CHANGED`）。
    struct StaleSourceHost;

    impl AgentHost for StaleSourceHost {
        fn begin(&self, _conn: &Connection, _selection: &Value) -> Result<Value, String> {
            Err("stale source host begin not implemented".to_string())
        }
        fn read(
            &self,
            _conn: &Connection,
            _handle: &MainGenerationRunHandle,
        ) -> Result<Value, String> {
            Err("stale source host read not implemented".to_string())
        }
        fn resume(
            &self,
            _conn: &Connection,
            _handle: &MainGenerationRunHandle,
        ) -> Result<Value, String> {
            Err("stale source host resume not implemented".to_string())
        }
        fn execute(
            &self,
            _conn: &Connection,
            _handle: &MainGenerationRunHandle,
            _nonce: &str,
            _task: &Value,
        ) -> Result<Value, String> {
            Err("stale source host execute not implemented".to_string())
        }
        fn current(
            &self,
            _conn: &Connection,
            _run: &DurableGenerationRun,
            _expected: Option<&RunBinding>,
        ) -> bool {
            false
        }
        fn capture(
            &self,
            _conn: &Connection,
            _run: &DurableGenerationRun,
        ) -> Result<RunBinding, String> {
            Err("stale source host capture not implemented".to_string())
        }
    }

    #[test]
    fn admit_workflow_source_changed_rejected() {
        let conn = test_connection();
        let agent = AgentGeneration::new(
            &conn,
            GenerationRunRepository::default(),
            "project-1",
            "epoch-1",
            &assert_current,
            &StaleSourceHost,
        )
        .expect("construct AgentGeneration with stale source host");
        let (registration_id, registration) = admit_registration("source-changed", &[]);
        seed_workflow_run(&conn, &registration, "completed", "attempt-0");
        assert_eq!(
            agent
                .admit_workflow(&admit_selection(&registration_id))
                .unwrap_err(),
            GENERATION_AGENT_SOURCE_CHANGED
        );
    }

    #[test]
    fn admit_workflow_admits_new_child() {
        let conn = test_connection();
        let agent = agent_generation(&conn);
        let (registration_id, registration) = admit_registration("new-child", &[]);
        seed_workflow_run(&conn, &registration, "completed", "attempt-0");
        // 无已登记子代 → 五重门禁 / planning / 根纪元 / 源一致性
        // 全部通过，返回 { existing: None, parentRootActionId }。
        let admitted = agent
            .admit_workflow(&admit_selection(&registration_id))
            .expect("admit workflow")
            .expect("no existing child");
        assert!(admitted.existing.is_none());
        assert_eq!(admitted.parent_root_action_id, "root-1");
    }

    #[test]
    fn admit_workflow_existing_child_idempotent() {
        let conn = test_connection();
        let agent = agent_generation(&conn);
        let (registration_id, registration) = admit_registration("idempotent", &["child-0"]);
        seed_workflow_run(&conn, &registration, "completed", "attempt-0");
        // 已登记子代：operation + chapterNumber 与选择一致 →
        // 幂等返回 existing（跳过 planning / 根纪元 / 源一致性）。
        seed_admit_child(
            &conn,
            "child-0",
            "chapter-blueprint-directory",
            Some(5),
            &registration_id,
        );
        let mut selection = admit_selection(&registration_id);
        selection["chapterNumber"] = Value::Number(5.into());
        let admitted = agent
            .admit_workflow(&selection)
            .expect("admit workflow")
            .expect("existing child");
        assert_eq!(admitted.existing.expect("existing").run_id, "child-0");
        assert_eq!(admitted.parent_root_action_id, "root-1");
    }

    #[test]
    fn admit_workflow_ambiguous_children_rejected() {
        let conn = test_connection();
        let agent = agent_generation(&conn);
        let (registration_id, registration) =
            admit_registration("ambiguous", &["child-0", "child-1"]);
        seed_workflow_run(&conn, &registration, "completed", "attempt-0");
        // 两个已登记子代均匹配 operation + chapterNumber →
        // GENERATION_AGENT_CHILD_AMBIGUOUS。
        seed_admit_child(
            &conn,
            "child-0",
            "chapter-blueprint-directory",
            Some(5),
            &registration_id,
        );
        seed_admit_child(
            &conn,
            "child-1",
            "chapter-blueprint-directory",
            Some(5),
            &registration_id,
        );
        let mut selection = admit_selection(&registration_id);
        selection["chapterNumber"] = Value::Number(5.into());
        assert_eq!(
            agent.admit_workflow(&selection).unwrap_err(),
            GENERATION_AGENT_CHILD_AMBIGUOUS
        );
    }

    struct PreparationHost {
        current: bool,
        blocked: bool,
        cancelled: bool,
        non_replayable: bool,
        begins: std::cell::Cell<usize>,
        resumes: std::cell::Cell<usize>,
    }

    impl Default for PreparationHost {
        fn default() -> Self {
            Self {
                current: true,
                blocked: false,
                cancelled: false,
                non_replayable: false,
                begins: std::cell::Cell::new(0),
                resumes: std::cell::Cell::new(0),
            }
        }
    }

    impl AgentHost for PreparationHost {
        fn begin(&self, conn: &Connection, selection: &Value) -> Result<Value, String> {
            self.begins.set(self.begins.get() + 1);
            let parent = selection["parentRootActionId"].as_str().unwrap();
            let key = selection["agentSession"]["key"].as_str().unwrap();
            let index = selection["agentSession"]["roundIndex"].as_i64().unwrap();
            let run_id = format!("prepared-child-{index}");
            let context = preparation_context();
            let binding = agent_binding(&context, key, index, None);
            // 只添加同预算的阶段行，不重新创建或覆盖父预算。
            conn.execute(
                "INSERT INTO generation_runs (run_id,root_action_id,binding_json,status,created_at_ms,open_key) VALUES (?1,?2,?3,'active',0,?4)",
                rusqlite::params![run_id, parent, binding.to_string(), format!("open-{run_id}")],
            ).map_err(|error| error.to_string())?;
            Ok(json!({"handle": {"projectId":"project-1","epoch":"epoch-1",
                "rootActionId":parent,"runId":run_id}}))
        }

        fn read(
            &self,
            conn: &Connection,
            handle: &MainGenerationRunHandle,
        ) -> Result<Value, String> {
            let mut view = TestHost.read(conn, handle)?;
            view["nonReplayable"] = json!(self.non_replayable);
            view["ledger"] = json!({"blockedCode": if self.blocked { Some("GENERATION_STORAGE_FAILED") } else { None }});
            if self.cancelled {
                view["status"] = json!("cancelled");
            }
            Ok(view)
        }

        fn resume(
            &self,
            conn: &Connection,
            handle: &MainGenerationRunHandle,
        ) -> Result<Value, String> {
            self.resumes.set(self.resumes.get() + 1);
            TestHost.read(conn, handle)
        }

        fn execute(
            &self,
            _: &Connection,
            _: &MainGenerationRunHandle,
            _: &str,
            _: &Value,
        ) -> Result<Value, String> {
            panic!("准备阶段不得派发模型请求")
        }

        fn current(
            &self,
            _: &Connection,
            _: &DurableGenerationRun,
            _: Option<&RunBinding>,
        ) -> bool {
            self.current
        }

        fn capture(
            &self,
            _: &Connection,
            run: &DurableGenerationRun,
        ) -> Result<RunBinding, String> {
            Ok(run.binding.clone())
        }
    }

    fn preparation_context() -> AgentGenerationContext {
        let mut context = test_context();
        context.initial_messages = serde_json::from_value(json!([
            {"role":"system","content":"固定系统合同"},
            {"role":"user","content":"合成任务"}
        ]))
        .unwrap();
        context
    }

    fn preparation_run(conn: &Connection, attempt: Option<AttemptFixture>) {
        seed_agent_run(
            conn,
            "root-1",
            "run-0",
            &agent_binding(
                &preparation_context(),
                &text_hash("prepared-session"),
                0,
                None,
            ),
            attempt,
        );
    }

    fn preparation_agent<'a>(
        conn: &'a Connection,
        host: &'a PreparationHost,
    ) -> AgentGeneration<'a> {
        AgentGeneration::new(
            conn,
            GenerationRunRepository::default(),
            "project-1",
            "epoch-1",
            &assert_current,
            host,
        )
        .unwrap()
    }

    fn resume_completed_fixture(conn: &Connection) {
        preparation_run(
            conn,
            Some(AttemptFixture {
                attempt_id: "resume-attempt".to_string(),
                attempt_status: "settled",
                usage_receipt_json: completed_usage("合成已完成回复", json!([])),
                artifact: Some(("合成已完成回复".to_string(), 1)),
            }),
        );
    }

    fn resume_epoch_agent<'a>(
        conn: &'a Connection,
        host: &'a PreparationHost,
    ) -> AgentGeneration<'a> {
        AgentGeneration::new(
            conn,
            GenerationRunRepository::default(),
            "project-1",
            "epoch-2",
            &assert_current,
            host,
        )
        .unwrap()
    }

    fn resume_epoch(conn: &Connection) -> String {
        GenerationRunRepository::default()
            .budget(conn, "root-1")
            .unwrap()
            .root
            .epoch
    }

    #[test]
    fn resume_completed_transfers_control_without_rewriting_history_or_dispatch_test() {
        let conn = test_connection();
        resume_completed_fixture(&conn);
        let host = PreparationHost::default();
        let agent = resume_epoch_agent(&conn, &host);
        let before = agent.read(&handle_of("run-0")).unwrap();
        let history: (String, String, String) = conn
            .query_row(
                "SELECT r.binding_json,a.attempt_json,v.artifact_json FROM generation_runs r
             JOIN generation_attempts a USING(run_id) JOIN generation_artifacts v USING(attempt_id)
             WHERE r.run_id='run-0'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        let after = agent.resume(&handle_of("run-0")).unwrap();
        assert_eq!(after.handle, before.handle);
        assert_eq!(after.rounds, before.rounds);
        assert_eq!(after.handle.epoch, "epoch-1");
        assert_eq!(resume_epoch(&conn), "epoch-2");
        let preserved: (String, String, String) = conn
            .query_row(
                "SELECT r.binding_json,a.attempt_json,v.artifact_json FROM generation_runs r
             JOIN generation_attempts a USING(run_id) JOIN generation_artifacts v USING(attempt_id)
             WHERE r.run_id='run-0'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(preserved, history);
        let epoch:String = conn.query_row(
            "SELECT json_extract(usage_receipt_json,'$.agentControlEpoch') FROM generation_attempts WHERE attempt_id='resume-attempt'",
            [],|row|row.get(0),
        ).unwrap();
        assert_eq!(epoch, "epoch-2");
        assert_eq!(host.resumes.get(), 0);
        assert_eq!(host.begins.get(), 0);
        assert_eq!(
            agent.runs.budget(&conn, "root-1").unwrap().attempts.len(),
            1
        );
    }

    #[test]
    fn resume_completed_conflict_and_cancelled_root_never_transfer_control_test() {
        for conflict in [true, false] {
            let conn = test_connection();
            resume_completed_fixture(&conn);
            let host = PreparationHost {
                current: !conflict,
                ..Default::default()
            };
            if !conflict {
                conn.execute("UPDATE generation_roots SET action_json=json_set(action_json,'$.status','cancelled')",[]).unwrap();
            }
            let agent = resume_epoch_agent(&conn, &host);
            assert!(agent.resume(&handle_of("run-0")).is_err());
            assert_eq!(resume_epoch(&conn), "epoch-1");
            assert_eq!(host.resumes.get(), 0);
            let control:Option<String> = conn.query_row(
                "SELECT json_extract(usage_receipt_json,'$.agentControlEpoch') FROM generation_attempts WHERE attempt_id='resume-attempt'",
                [],|row|row.get(0),
            ).unwrap();
            assert!(control.is_none());
        }
    }

    #[test]
    fn resume_completed_receipt_failure_rolls_back_even_if_outer_transaction_commits_test() {
        let conn = test_connection();
        resume_completed_fixture(&conn);
        conn.execute_batch("CREATE TEMP TRIGGER reject_agent_control BEFORE UPDATE OF usage_receipt_json ON generation_attempts
            WHEN json_extract(NEW.usage_receipt_json,'$.agentControlEpoch')='epoch-2'
            BEGIN SELECT RAISE(ABORT,'synthetic-control-failure'); END;").unwrap();
        let host = PreparationHost::default();
        let transaction = conn.unchecked_transaction().unwrap();
        {
            let agent = resume_epoch_agent(&transaction, &host);
            assert!(agent.resume(&handle_of("run-0")).is_err());
        }
        transaction.commit().unwrap();
        assert_eq!(resume_epoch(&conn), "epoch-1");
        conn.execute_batch("DROP TRIGGER reject_agent_control")
            .unwrap();
        resume_epoch_agent(&conn, &host)
            .resume(&handle_of("run-0"))
            .unwrap();
        assert_eq!(resume_epoch(&conn), "epoch-2");
    }

    #[test]
    fn resume_completed_success_borrows_and_obeys_outer_rollback_test() {
        let conn = test_connection();
        resume_completed_fixture(&conn);
        let host = PreparationHost::default();
        let transaction = conn.unchecked_transaction().unwrap();
        resume_epoch_agent(&transaction, &host)
            .resume(&handle_of("run-0"))
            .unwrap();
        assert_eq!(resume_epoch(&transaction), "epoch-2");
        assert!(!conn.is_autocommit());
        transaction.rollback().unwrap();
        assert_eq!(resume_epoch(&conn), "epoch-1");
    }

    #[test]
    fn resume_unstarted_delegates_to_host_without_new_attempt_or_control_epoch_test() {
        let conn = test_connection();
        preparation_run(&conn, None);
        let host = PreparationHost::default();
        let agent = preparation_agent(&conn, &host);
        let recovery = agent.resume(&handle_of("run-0")).unwrap();
        assert_eq!(recovery.rounds[0].status, "not-started");
        assert_eq!(host.resumes.get(), 1);
        assert_eq!(host.begins.get(), 0);
        assert!(agent
            .runs
            .budget(&conn, "root-1")
            .unwrap()
            .attempts
            .is_empty());
    }

    #[test]

    fn prepare_round_freezes_request_without_dispatch_or_connection_borrow() {
        let conn = test_connection();
        preparation_run(&conn, None);
        let host = PreparationHost::default();
        let agent = preparation_agent(&conn, &host);
        let PreparedAgentRound::Execute(plan) = agent
            .prepare_execute_round(&json!({"handle":handle_of("run-0"),"index":0}))
            .unwrap()
        else {
            panic!("首轮应准备执行");
        };
        assert_eq!(plan.handle.run_id, "run-0");
        assert_eq!(plan.invocation_nonce, "agent-round:0");
        assert_eq!(plan.task["purpose"], "agent");
        assert_eq!(plan.task["output"], "visible-text");
        assert_eq!(plan.task["reasoningStage"], "general");
        assert_eq!(host.begins.get(), 0);
        assert_eq!(host.resumes.get(), 0);
        assert!(GenerationRunRepository::default()
            .budget(&conn, "root-1")
            .unwrap()
            .attempts
            .is_empty());
        drop(agent);
        drop(conn);
        assert_eq!(
            plan.task["messages"],
            serde_json::to_value(preparation_context().initial_messages).unwrap()
        );
    }

    #[test]
    fn prepare_round_validates_integer_range_after_handle_identity() {
        let conn = test_connection();
        preparation_run(&conn, None);
        let host = PreparationHost::default();
        let agent = preparation_agent(&conn, &host);
        for index in [
            Value::Null,
            json!(-1),
            json!(8),
            json!(0.5),
            json!("0"),
            json!(false),
        ] {
            assert_eq!(
                agent
                    .prepare_execute_round(&json!({"handle":handle_of("run-0"),"index":index}))
                    .unwrap_err(),
                "GENERATION_AGENT_ROUND_INVALID"
            );
        }
        let mut wrong = handle_of("run-0");
        wrong["projectId"] = json!("other-project");
        assert_eq!(
            agent
                .prepare_execute_round(&json!({"handle":wrong,"index":-1}))
                .unwrap_err(),
            GENERATION_AGENT_IDENTITY_MISMATCH
        );
        assert!(matches!(
            agent
                .prepare_execute_round(&json!({"handle":handle_of("run-0"),"index":0.0}))
                .unwrap(),
            PreparedAgentRound::Execute(_)
        ));
    }

    #[test]
    fn prepare_round_returned_or_unknown_invocation_replays_before_new_admission() {
        for (status, usage, artifact) in [
            (
                "settled",
                completed_usage("合成返回", json!([])),
                Some(("合成返回".to_string(), 0)),
            ),
            ("unknown", json!({}), None),
        ] {
            let conn = test_connection();
            preparation_run(
                &conn,
                Some(AttemptFixture {
                    attempt_id: "attempt-0".into(),
                    attempt_status: status,
                    usage_receipt_json: usage,
                    artifact,
                }),
            );
            let host = PreparationHost {
                current: false,
                blocked: true,
                cancelled: true,
                non_replayable: true,
                ..Default::default()
            };
            let agent = preparation_agent(&conn, &host);
            let before = agent.read(&handle_of("run-0")).unwrap();
            let PreparedAgentRound::Replay(replayed) = agent
                .prepare_execute_round(&json!({"handle":handle_of("run-0"),"index":0}))
                .unwrap()
            else {
                panic!("既有调用不得静默重跑");
            };
            assert_eq!(*replayed, before);
            assert_eq!(host.begins.get(), 0);
            assert_eq!(host.resumes.get(), 0);
            assert_eq!(
                GenerationRunRepository::default()
                    .budget(&conn, "root-1")
                    .unwrap()
                    .attempts
                    .len(),
                1
            );
        }
    }

    #[test]
    fn prepare_round_rejects_pending_tools_and_out_of_order_index() {
        let conn = test_connection();
        let usage = completed_usage(
            "合成返回",
            json!([{"name":"propose_novel_config","arguments":{}}]),
        );
        preparation_run(
            &conn,
            Some(AttemptFixture {
                attempt_id: "attempt-0".into(),
                attempt_status: "settled",
                usage_receipt_json: usage,
                artifact: Some(("合成返回".into(), 0)),
            }),
        );
        let host = PreparationHost::default();
        let agent = preparation_agent(&conn, &host);
        for index in [1, 2] {
            assert_eq!(
                agent
                    .prepare_execute_round(&json!({"handle":handle_of("run-0"),"index":index}))
                    .unwrap_err(),
                "GENERATION_AGENT_ROUND_ORDER"
            );
        }
        assert_eq!(host.begins.get(), 0);
    }

    #[test]
    fn prepare_round_rejects_blocked_cancelled_or_changed_source_without_dispatch() {
        for (blocked, cancelled, current, expected) in [
            (true, false, true, "GENERATION_AGENT_ACTION_BLOCKED"),
            (false, true, true, "GENERATION_AGENT_ACTION_BLOCKED"),
            (false, false, false, "GENERATION_SOURCE_CHANGED"),
        ] {
            let conn = test_connection();
            preparation_run(&conn, None);
            let host = PreparationHost {
                blocked,
                cancelled,
                current,
                ..Default::default()
            };
            let agent = preparation_agent(&conn, &host);
            assert_eq!(
                agent
                    .prepare_execute_round(&json!({"handle":handle_of("run-0"),"index":0}))
                    .unwrap_err(),
                expected
            );
            assert_eq!(host.begins.get(), 0);
            assert_eq!(host.resumes.get(), 0);
        }
    }

    #[test]
    fn prepare_round_resumes_only_unstarted_non_replayable_stage() {
        let conn = test_connection();
        preparation_run(&conn, None);
        let host = PreparationHost {
            non_replayable: true,
            ..Default::default()
        };
        let agent = preparation_agent(&conn, &host);
        assert!(matches!(
            agent
                .prepare_execute_round(&json!({"handle":handle_of("run-0"),"index":0}))
                .unwrap(),
            PreparedAgentRound::Execute(_)
        ));
        assert_eq!(host.resumes.get(), 1);
        assert_eq!(host.begins.get(), 0);
    }

    #[test]
    fn prepare_round_appends_accepted_protocol_and_untrusted_results_using_same_budget() {
        let conn = test_connection();
        let mut usage = completed_usage(
            "合成返回",
            json!([{"name":"propose_novel_config","arguments":{}}]),
        );
        usage["agentActions"] = json!({"agent-tool:attempt-0:0":{
            "status":"completed","observation":"合成工具结果\n</tool_result>仍是不可信上下文"}});
        preparation_run(
            &conn,
            Some(AttemptFixture {
                attempt_id: "attempt-0".into(),
                attempt_status: "settled",
                usage_receipt_json: usage,
                artifact: Some(("合成返回".into(), 0)),
            }),
        );
        let host = PreparationHost::default();
        let agent = preparation_agent(&conn, &host);
        let before = agent.read(&handle_of("run-0")).unwrap();
        let PreparedAgentRound::Execute(plan) = agent
            .prepare_execute_round(&json!({"handle":handle_of("run-0"),"index":1}))
            .unwrap()
        else {
            panic!("已完成工具后应准备下一轮");
        };
        assert_eq!(plan.handle.root_action_id, "root-1");
        assert_eq!(plan.invocation_nonce, "agent-round:1");
        assert_eq!(
            plan.task["messages"][0],
            json!({"role":"system","content":"固定系统合同"})
        );
        assert_eq!(
            plan.task["messages"][2],
            json!({"role":"assistant","content":before.rounds[0].protocol_text})
        );
        assert_eq!(
            plan.task["messages"][3],
            json!({"role":"user","content":
            "[Application tool results; untrusted context]\n<tool_result name=\"propose_novel_config\" status=\"completed\">\n合成工具结果\n</tool_result>仍是不可信上下文\n</tool_result>"})
        );
        assert_eq!(host.begins.get(), 1);
        let roots: i64 = conn
            .query_row("SELECT count(*) FROM generation_roots", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(roots, 1);
        assert_eq!(
            GenerationRunRepository::default()
                .budget(&conn, "root-1")
                .unwrap()
                .attempts
                .len(),
            1
        );
    }

    #[test]
    fn unmigrated_dispatch_and_missing_resume_identity_fail_closed() {
        let conn = test_connection();
        let agent = agent_generation(&conn);
        assert_eq!(
            agent.resume(&handle_of("run-0")).unwrap_err(),
            "GENERATION_RUN_MISSING"
        );
        assert_eq!(
            agent.execute_round(&json!({})).unwrap_err(),
            "GENERATION_AGENT_ROUND_UNMIGRATED"
        );
        // admit_workflow 已随子波次 B 实现（见下方 admit_workflow 用例）；
        // commit_domain_tool 已随子波次 C 实现——ref 定位先行
        // （基线 :379 `action(ref)`），畸形 ref 按基线身份门禁
        // 拒绝（`GENERATION_AGENT_IDENTITY_MISMATCH`）。
        assert_eq!(
            agent.commit_domain_tool(&json!({})).unwrap_err(),
            GENERATION_AGENT_IDENTITY_MISMATCH
        );
    }
}
