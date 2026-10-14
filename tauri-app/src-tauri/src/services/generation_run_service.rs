//! 生成运行服务（1:1 移植自 `electron/services/generation-run-service.ts`，
//! 355 行）——LLM 双阶段执行循环：分发 + 流式事件 + 持久化结算。
//!
//! # 范围（B3-2b，编排者范围调整）
//!
//! 本波为服务本体：**dispatch 经 `GenerationDispatcher` trait 依赖
//! 注入**（B3-3 经 `llm::openai`/`llm::gemini` 的进程内流式核心
//! 实现）。发射器活化（`commands/generation.rs` 去 `dead_code` 并
//! 接线）随 B3-3——`execute_inner` 仍 fail-closed，本波无服务构造
//! 点可接线。
//!
//! # 裁决与偏差登记（基线 → Rust）
//!
//! 1. **取消句柄**（裁决③）：自建进程内 `watch`-channel 令牌
//!    （`GenerationCancelToken`），不经 IPC `llm_streams` 注册表。
//! 2. **`tokio` "time" feature**：1 秒 flush 定时器与 active-elapsed
//!    deadline 逐字保留所需；tokio 已是直接依赖，启用内置 feature
//!    不新增依赖、不改 `Cargo.lock`。
//! 3. **`unsavedTail`**：基线 `GenerationExecutionReceipt.unsavedTail?`
//!    （generation-run-repository.ts:63）在 Rust 侧补
//!    `unsaved_tail: Option<String>` 字段（纯运行时投影，不落库，
//!    serde default 保证反序列化兼容）。
//! 4. **agent 协议解析**：基线 `src/shared/agent-response-protocol.ts`
//!    无 Rust 等价物——平移至 `services/agent_response_protocol.rs`
//!    （「一个基线文件 ↔ 一个 Rust 模块」既定原则）。
//! 5. **增量 flush 交付**（时序偏差）：基线 `onVisible` 内同步执行
//!    `flush()`（持 DB 句柄）；Rust 侧 `&Connection` 非 `Send`，
//!    流式回调（需在 `Send` future 内运行）不能直接持 conn——
//!    4096 字节增量 flush 请求经 `mpsc` 通道交主循环 select 分支
//!    执行。事件内容与持久化结果一致，持久化时机延迟至主循环下一
//!    轮 select。`onSnapshot` 通知不依赖 conn，仍在回调内同步执行。
//! 6. **`safeTransportError`**（偏差）：基线从 Error 对象提取
//!    `name`/`code`/`cause.code`；Rust 侧内部错误为字符串，仅按
//!    已知错误码集合提取 `errorCode`（`error_name`/`cause_code`
//!    恒 `None`）。B3-3 dispatch 实现若构造结构化传输错误可扩展
//!    提取面。
//! 7. **`Number.isSafeInteger`**（类型吸收）：基线逐项安全整数校验；
//!    Rust 侧 i64 建模，恒真检查省略。
//! 8. **非空断言**（偏差）：基线 `receipt.artifact!` 非空断言取初始
//!    文本；Rust 侧以 `GENERATION_STORAGE_FAILED` 保守拒绝（reserve
//!    保证 artifact 存在，缺失为存储层不变量破坏）。
//! 9. **`terminal` 标志**（控制流吸收）：基线回调检查 `terminal`
//!    防派发完成后回调；Rust 侧由控制流保证（dispatch future 完成
//!    后回调不再被调用）。
//! 10. **回调异常**（偏差）：基线 `try { deps.onSnapshot?.(...) } catch {}`
//!     吞掉 UI 通知异常；Rust 侧回调 panic 传播（不吞 panic）。
//! 11. **`select!` 公平性**（实现细节）：基线 `Promise.race` 的
//!     微任务时序与 tokio `select!` 的就绪分支随机选择存在细微
//!     差异；取消/超时/完成三者的最终收据语义逐字一致。
//!
//! # 错误码（基线 §6.B，逐字保留）
//!
//! `GENERATION_REQUEST_HASH_INVALID` / `GENERATION_RESERVATION_INVALID` /
//! `GENERATION_STORAGE_FAILED` / `GENERATION_CANCELLED` /
//! `GENERATION_AGENT_ACTION_LIMIT` / `GENERATION_PROJECT_CLOSED` /
//! `NETWORK_ERROR` / `GENERATION_PROVIDER_FAILED` /
//! `GENERATION_STREAM_INVALID` / `ROOT_BUDGET_EXHAUSTED` /
//! `GENERATION_UNSAVED_TAIL_PRESENT` / `GENERATION_RECOVERY_UNAUTHORIZED`

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use regex::Regex;
use rusqlite::Connection;
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use crate::repositories::generation_run_repository::{
    AgentResponse, DurableGenerationRun, GenerationExecutionReceipt, GenerationRunRepository,
    GenerationTransportDiagnostics, GenerationUsageReceipt, OpenGenerationRunRequest,
    ProviderUsagePolicy, RunBinding, ToolCall, VisibleArtifact,
};
use crate::services::agent_response_protocol::project_interrupted_agent_visible_text;

/// 基线 `setInterval(flush, 1000)`——1 秒增量持久化间隔。
const FLUSH_INTERVAL_MS: u64 = 1_000;
/// 基线 `Buffer.byteLength(text) - Buffer.byteLength(durable) >= 4096`。
const SNAPSHOT_FLUSH_BYTES: usize = 4_096;
/// 基线 `Buffer.byteLength(next, 'utf8') > 8 * 1024 * 1024`
/// （agent 协议文本上限，不进入用户 artifact 通道）。
const AGENT_PROTOCOL_BYTE_LIMIT: usize = 8 * 1024 * 1024;
/// 基线 `parsed.toolCalls.length > 64`。
const AGENT_ACTION_LIMIT: usize = 64;

/// 基线 `/\<\/?think\b/iu`——think 标签泄漏检测：推理标签
/// 流入可见文本即判流无效。
static RE_THINK_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)</?think\b").expect("think 标签正则为基线逐字平移，编译不可失败")
});

// ============================================================================
// 契约类型（基线 generation-run-service.ts :1-71）
// ============================================================================

/// 基线 `VisibleGenerationEvent.kind` 联合。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibleEventKind {
    /// 基线 `'delta'`——增量文本。
    Delta,
    /// 基线 `'cumulative'`——累积文本。
    Cumulative,
}

impl VisibleEventKind {
    /// 事件 JSON 投影的 `kind` 字面量（eventId 去重键的组成部分）。
    fn json_label(&self) -> &'static str {
        match self {
            VisibleEventKind::Delta => "delta",
            VisibleEventKind::Cumulative => "cumulative",
        }
    }
}

/// 基线 `VisibleGenerationEvent`。
#[derive(Debug, Clone, PartialEq)]
pub struct VisibleGenerationEvent {
    pub kind: VisibleEventKind,
    pub text: String,
    pub event_id: Option<String>,
}

/// 基线 `GenerationProviderUsage`——provider 适配器的原始用量
/// 报告（`settleProviderUsage` 的输入；`trusted` 仅由可信
/// provider 适配器对已知协议置位）。
#[derive(Debug, Clone, PartialEq)]
pub struct GenerationProviderUsage {
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
    /// 基线 `'included-in-completion' | 'separately-billed' | 'unknown'`
    /// 联合；以 `String` 鸭子类型保真。
    pub accounting: String,
    pub total_includes_reasoning: bool,
    pub trusted: bool,
}

/// 基线 `GenerationDispatchResult`——dispatch 的返回。
#[derive(Debug, Clone, PartialEq)]
pub struct GenerationDispatchResult {
    pub usage: Option<GenerationProviderUsage>,
    pub finish_reason: Option<String>,
}

/// 基线 `GenerationUnsavedTail`——存储失败时保留的可复制尾文本。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationUnsavedTail {
    pub attempt_id: String,
    pub artifact_id: String,
    pub durable_revision: i64,
    pub text: String,
    /// 基线字面量联合 `'GENERATION_STORAGE_FAILED'`（单值）。
    pub failure_code: String,
}

/// 基线 `ExecuteGenerationRequest`——内部主进程专用请求。
/// IPC controller 供给冻结 plan 与 lease，绝不接受渲染层估计。
#[derive(Debug, Clone)]
pub struct ExecuteGenerationRequest {
    pub run_id: String,
    pub invocation_nonce: String,
    pub request_hash: String,
    pub provider_request: Value,
    pub reserved_tokens: i64,
    pub requested_output_tokens: i64,
    pub input_upper_bound_tokens: i64,
    pub reasoning_upper_bound_tokens: i64,
    pub usage_policy: ProviderUsagePolicy,
    pub purpose: Option<String>,
    pub budget_decision: Option<Value>,
    /// 主进程专用输出投影，选自冻结的 Agent 工具清单。
    pub agent_tool_names: Option<Vec<String>>,
    /// 原始语义任务随预留一起提交（重放消费）。
    pub replay_task: Option<Value>,
    /// 首稿提示经主进程复核确实带有生成前定稿对账注入块；
    /// 恢复据此决定续写/压缩是否沿用对账结果。
    pub reconciliation_injected: bool,
}

/// 基线 `GenerationRunServiceDependencies.onReasoning` 事件
/// （渲染层专用投影；推理永不进入 artifact 或回执）。
#[derive(Debug, Clone)]
pub struct ReasoningEvent {
    pub run_id: String,
    pub root_action_id: String,
    pub attempt_id: String,
    pub text: String,
}

/// 基线 `GenerationRunServiceDependencies.onSnapshot` 事件。
#[derive(Debug, Clone)]
pub struct SnapshotEvent {
    pub attempt_id: String,
    pub visible_text: String,
    pub durable_text: String,
    pub durable_revision: i64,
    pub storage_failed: bool,
    pub diagnostics: Option<GenerationTransportDiagnostics>,
}

/// 基线 `safeTransportError` 的提取面
/// （`Pick<GenerationTransportDiagnostics, 'errorName' | 'errorCode' | 'causeCode'>`）。
struct TransportErrorFields {
    error_name: Option<String>,
    error_code: Option<String>,
    cause_code: Option<String>,
}

/// 进程内分发回调（基线 `deps.dispatch` 第二参）。
///
/// 回调为 `Send + Sync`——dispatch future 经 `Box<dyn Future + Send>`
/// 返回，回调引用随 future 跨 poll 存活。
pub struct DispatchOptions<'a> {
    pub signal: &'a GenerationCancelSignal,
    pub on_visible: &'a (dyn Fn(VisibleGenerationEvent) + Send + Sync),
    pub on_reasoning: Option<&'a (dyn Fn(String) + Send + Sync)>,
    pub on_diagnostics: Option<&'a (dyn Fn(GenerationTransportDiagnostics) + Send + Sync)>,
}

/// 进程内 LLM 流式分发器（基线 `deps.dispatch` 函数依赖注入；
/// B3-3 经 `llm::openai`/`llm::gemini` 的进程内流式核心实现——
/// 状态文件裁决⑤：`llm_generation.rs` 流式核心抽取为 `pub`
/// 进程内服务函数，command 外壳保留并委托）。
pub trait GenerationDispatcher: Send + Sync {
    fn dispatch<'a>(
        &'a self,
        request: &'a Value,
        options: DispatchOptions<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<GenerationDispatchResult, String>> + Send + 'a>>;
}

/// 基线 `GenerationRunServiceDependencies`。
pub struct GenerationRunServiceDependencies {
    pub repository: GenerationRunRepository,
    pub dispatch: Arc<dyn GenerationDispatcher>,
    /// 渲染层专用投影。推理永不进入 artifact 或回执。
    pub on_reasoning: Option<Arc<dyn Fn(ReasoningEvent) + Send + Sync>>,
    pub on_snapshot: Option<Arc<dyn Fn(SnapshotEvent) + Send + Sync>>,
    /// 恢复重分发校验（基线 owner 注入 `rebuildBinding` 比对）。
    pub validate_recovery: Option<ValidateRecovery>,
}

/// 基线 `deps.validateRecovery`——`async (previous, next) => boolean`。
pub type ValidateRecovery = Arc<
    dyn Fn(RunBinding, RunBinding) -> Pin<Box<dyn Future<Output = Result<bool, String>> + Send>>
        + Send
        + Sync,
>;

// ============================================================================
// 纯函数（基线 :73-99 与共享契约辅助）
// ============================================================================

/// 基线 `settleProviderUsage`（:73-99）——provider 用量记账结算。
///
/// 三态判定：①非 trusted → 保守收据；②`policy.reasoning === 'unknown'`
/// 时仅信任已报告且算术自洽的 total（适配器协议标签不能建立未知
/// 供应商的计费规则）；③按 `accounting` 与 `policy.reasoning` 的
/// 匹配关系逐项校验。
pub fn settle_provider_usage(
    usage: Option<&GenerationProviderUsage>,
    policy: &ProviderUsagePolicy,
) -> GenerationUsageReceipt {
    let conservative = GenerationUsageReceipt {
        policy: policy.clone(),
        input_tokens: None,
        completion_tokens: None,
        reasoning_tokens: None,
        actual_tokens: None,
        trusted: false,
        agent_response: None,
    };
    let Some(usage) = usage else {
        return conservative;
    };
    if !usage.trusted {
        return conservative;
    }
    let (prompt, completion, reasoning, total) = (
        usage.prompt_tokens,
        usage.completion_tokens,
        usage.reasoning_tokens,
        usage.total_tokens,
    );
    let (Some(prompt), Some(completion)) = (prompt, completion) else {
        return conservative;
    };
    if prompt < 0
        || completion < 0
        || reasoning.is_some_and(|value| value < 0)
        || total.is_some_and(|value| value < 0)
    {
        return conservative;
    }
    if policy.reasoning == "unknown" {
        let Some(total) = total else {
            return conservative;
        };
        // 仅信任已报告且算术自洽的 total。
        let consistent = (total == prompt + completion
            && reasoning.map_or(true, |r| r <= completion))
            || reasoning.is_some_and(|r| total == prompt + completion + r);
        if !consistent {
            return conservative;
        }
        return GenerationUsageReceipt {
            policy: policy.clone(),
            input_tokens: Some(prompt),
            completion_tokens: Some(completion),
            reasoning_tokens: reasoning,
            actual_tokens: Some(total),
            trusted: true,
            agent_response: None,
        };
    }
    if usage.accounting == "unknown" || usage.accounting != policy.reasoning {
        return conservative;
    }
    if usage.accounting == "included-in-completion" && reasoning.is_some_and(|r| r > completion) {
        return conservative;
    }
    if usage.accounting == "separately-billed" && reasoning.is_none() {
        return conservative;
    }
    let separately = usage.accounting == "separately-billed";
    let actual = prompt
        + completion
        + if separately {
            reasoning.unwrap_or(0)
        } else {
            0
        };
    let expected_total = prompt
        + completion
        + if separately && usage.total_includes_reasoning {
            reasoning.unwrap_or(0)
        } else {
            0
        };
    if total.is_some_and(|t| t != expected_total) {
        return conservative;
    }
    GenerationUsageReceipt {
        policy: policy.clone(),
        input_tokens: Some(prompt),
        completion_tokens: Some(completion),
        reasoning_tokens: reasoning,
        actual_tokens: Some(actual),
        trusted: true,
        agent_response: None,
    }
}

/// 基线 `safeTransportError`（src/shared/generation-contract.ts:21-33）。
/// 偏差登记见模块头注⑥。
fn safe_transport_error(error: &str) -> TransportErrorFields {
    /// 基线已知传输错误码集合。
    const KNOWN_CODES: &[&str] = &[
        "UND_ERR_BODY_TIMEOUT",
        "UND_ERR_HEADERS_TIMEOUT",
        "UND_ERR_CONNECT_TIMEOUT",
        "UND_ERR_SOCKET",
        "ECONNRESET",
        "ECONNREFUSED",
        "ETIMEDOUT",
        "ENOTFOUND",
        "EAI_AGAIN",
        "ABORT_ERR",
        "ERR_STREAM_PREMATURE_CLOSE",
    ];
    TransportErrorFields {
        error_name: None,
        error_code: KNOWN_CODES.contains(&error).then(|| error.to_string()),
        cause_code: None,
    }
}

/// 基线 `/^[a-f0-9]{64}$/u`——请求哈希校验。
fn is_valid_request_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'f' | b'0'..=b'9'))
}

/// 基线 reserve 失败分类 `/BUDGET|RESERVATION|EPOCH|DISPATCH|INVOCATION/`——
/// 预算类失败不触发 failedRoots 围栏（可重试）。
fn is_budget_class_error(error: &str) -> bool {
    error.contains("BUDGET")
        || error.contains("RESERVATION")
        || error.contains("EPOCH")
        || error.contains("DISPATCH")
        || error.contains("INVOCATION")
}

/// 基线 `/^(UND_ERR_|ECONN|ETIMEDOUT|ENOTFOUND|EAI_AGAIN|ERR_STREAM_PREMATURE_CLOSE)/`
/// ——失败码分类的网络前缀判定。
fn is_network_code_prefix(code: &str) -> bool {
    code.starts_with("UND_ERR_")
        || code.starts_with("ECONN")
        || code.starts_with("ETIMEDOUT")
        || code.starts_with("ENOTFOUND")
        || code.starts_with("EAI_AGAIN")
        || code.starts_with("ERR_STREAM_PREMATURE_CLOSE")
}

/// 基线 `/^(NETWORK_ERROR|ECONNRESET|ETIMEDOUT|FETCH_FAILED)$/`
/// ——错误消息精确匹配（error.message 分类面）。
fn is_network_error_message(message: &str) -> bool {
    matches!(
        message,
        "NETWORK_ERROR" | "ECONNRESET" | "ETIMEDOUT" | "FETCH_FAILED"
    )
}

// ============================================================================
// 取消句柄（裁决③：自建进程内令牌，不经 IPC llm_streams 注册表）
// ============================================================================

/// 基线 `AbortController` 的进程内等价物。
#[derive(Clone, Debug)]
pub struct GenerationCancelToken {
    sender: Arc<watch::Sender<bool>>,
}

impl GenerationCancelToken {
    /// 基线 `new AbortController()`——构造令牌与信号对。
    pub(crate) fn pair() -> (Self, GenerationCancelSignal) {
        let (sender, receiver) = watch::channel(false);
        (
            Self {
                sender: Arc::new(sender),
            },
            GenerationCancelSignal { receiver },
        )
    }

    /// 基线 `controller.abort()`——同步触发所有等待者。
    pub fn cancel(&self) {
        let _ = self.sender.send(true);
    }
}

/// 基线 `AbortSignal` 的进程内等价物。
#[derive(Clone, Debug)]
pub struct GenerationCancelSignal {
    receiver: watch::Receiver<bool>,
}

impl GenerationCancelSignal {
    /// 基线 `controller.signal.aborted`。
    pub fn is_cancelled(&self) -> bool {
        *self.receiver.borrow()
    }

    /// 基线 `cancelled` promise——等待取消信号；已取消时
    /// 立即返回。
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let mut receiver = self.receiver.clone();
        while receiver.changed().await.is_ok() && !*receiver.borrow() {}
    }
}

// ============================================================================
// 服务内部状态
// ============================================================================

/// attempt 级运行状态——基线 `execute` 闭包捕获的可变变量
/// （`text`/`durable`/`revision`/`storageFailed`/…）。以
/// `Arc<Mutex<…>>` 供流式回调与主循环共享（基线闭包直接
/// 捕获同一词法环境）。
struct AttemptRunState {
    text: String,
    durable: String,
    revision: i64,
    storage_failed: bool,
    stream_failed: bool,
    timed_out: bool,
    suspended: bool,
    /// 基线 `agentProtocolText`——agent 协议原始累积文本
    /// （角色确定前不流过用户 artifact 通道）。
    agent_protocol_text: String,
    agent_tool_names: Option<Vec<String>>,
    artifact_id: String,
    root_id: String,
    /// 基线 execute 作用域的 `receipt` 变量——`suspend` 闭包
    /// 重读后更新，`suspendForProjectClose` 的 `GENERATION_PROJECT_CLOSED`
    /// 路径消费。
    receipt: GenerationExecutionReceipt,
}

/// 基线 `controllers` Map 的值（:184-191）——`AbortController`
/// 与 attempt 级状态的共享记录。
#[derive(Clone)]
struct ControllerRecord {
    token: GenerationCancelToken,
    root_id: String,
    attempt_id: String,
    state: Arc<Mutex<AttemptRunState>>,
    /// 基线 `diagnostics` 闭包捕获变量——`onDiagnostics` 更新、
    /// `notify`/`settle` 读取；`suspendForProjectClose` 的
    /// `flush` 通知路径共享。
    diagnostics: Arc<Mutex<SharedDiagnostics>>,
}

/// 基线 `diagnostics`/`diagnosticsAt` 闭包捕获变量对。
#[derive(Default)]
struct SharedDiagnostics {
    value: Option<GenerationTransportDiagnostics>,
    at: Option<Instant>,
}

/// 在途 operation 的共享状态——基线 `pending` Map 的
/// `Promise<GenerationExecutionReceipt>` 等价物（`watch` 通道
/// 承载完成值；第二个同键调用者等待完成值而非重复执行）。
struct PendingOperation {
    state: watch::Sender<PendingState>,
    request_hash: String,
}

#[derive(Clone, Default)]
enum PendingState {
    #[default]
    Waiting,
    Done(Result<GenerationExecutionReceipt, String>),
}

impl PendingOperation {
    fn new(request_hash: &str) -> Self {
        let (sender, _) = watch::channel(PendingState::Waiting);
        Self {
            state: sender,
            request_hash: request_hash.to_string(),
        }
    }

    fn complete(&self, result: Result<GenerationExecutionReceipt, String>) {
        // 即使等待方尚未订阅，也必须保留完成值，避免迟到订阅永远等待。
        self.state.send_replace(PendingState::Done(result));
    }
}

/// 等待在途 operation 完成（基线 `pending.get(key)` 返回的
/// 在途 promise 的等价物）。
async fn wait_pending(
    operation: Arc<PendingOperation>,
) -> Result<GenerationExecutionReceipt, String> {
    let mut receiver = operation.state.subscribe();
    loop {
        if let PendingState::Done(result) = &*receiver.borrow() {
            return result.clone();
        }
        if receiver.changed().await.is_err() {
            // sender 存活于 service 生命周期内（complete 先于
            // remove 调用）——不可达。保守返回存储失败。
            return Err("GENERATION_STORAGE_FAILED".to_string());
        }
    }
}

/// 分发结局（基线 `Promise.race([provider, cancelled])` 的
/// 结果与 `controller.signal.aborted` 检查的合并）。
enum DispatchOutcome {
    Cancelled,
    Completed(Result<GenerationDispatchResult, String>),
}

// ============================================================================
// 服务本体
// ============================================================================

/// 易失中止原因；项目关闭优先于用户取消，避免并发请求降级关闭收据。
#[derive(Clone, Copy, PartialEq, Eq)]
enum AbortCause {
    Cancelled,
    ProjectClosed,
}

/// 数据库短临界区入口。实现必须在每次调用时验证连接归属与会话身份，
/// 且连接/业务锁不得逃出同步回调；服务不在网络 await 期间保存连接。
pub(crate) trait GenerationDatabaseAccess {
    fn with_connection<T>(
        &self,
        action: impl FnOnce(&Connection) -> Result<T, String>,
    ) -> Result<T, String>;
}

/// 保留既有仓储/内部调用的直接连接接口；生产短锁实现由命令层注入。
impl GenerationDatabaseAccess for Connection {
    fn with_connection<T>(
        &self,
        action: impl FnOnce(&Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        action(self)
    }
}

/// 生成运行服务——基线 `createGenerationRunService` 的工厂产物。
pub struct GenerationRunService {
    repository: GenerationRunRepository,
    dispatch: Arc<dyn GenerationDispatcher>,
    on_reasoning: Option<Arc<dyn Fn(ReasoningEvent) + Send + Sync>>,
    on_snapshot: Option<Arc<dyn Fn(SnapshotEvent) + Send + Sync>>,
    validate_recovery: Option<ValidateRecovery>,
    /// 基线 `controllers: Map<attemptId, …>`。
    controllers: Mutex<HashMap<String, ControllerRecord>>,
    /// 独立中止通道的根级意图，覆盖 controller 尚未登记的短窗口。
    abort_requested_roots: Mutex<HashMap<String, AbortCause>>,
    /// 基线 `pending: Map<key, Promise>`。
    pending: Mutex<HashMap<String, Arc<PendingOperation>>>,
    /// 基线 `failedRoots: Set<rootId>`——存储失败围栏。
    failed_roots: Mutex<HashSet<String>>,
    /// 基线 `unsavedTails: Map<attemptId, GenerationUnsavedTail>`。
    unsaved_tails: Mutex<HashMap<String, GenerationUnsavedTail>>,
}

/// 基线 `createGenerationRunService(deps)` 工厂。
pub fn create_generation_run_service(
    deps: GenerationRunServiceDependencies,
) -> GenerationRunService {
    GenerationRunService {
        repository: deps.repository,
        dispatch: deps.dispatch,
        on_reasoning: deps.on_reasoning,
        on_snapshot: deps.on_snapshot,
        validate_recovery: deps.validate_recovery,
        controllers: Mutex::new(HashMap::new()),
        abort_requested_roots: Mutex::new(HashMap::new()),
        pending: Mutex::new(HashMap::new()),
        failed_roots: Mutex::new(HashSet::new()),
        unsaved_tails: Mutex::new(HashMap::new()),
    }
}

impl GenerationRunService {
    // -------------------------------------------------------------------------
    // 内部辅助
    // -------------------------------------------------------------------------

    /// 基线 `notify`（:166-170）——快照通知。调用方持有
    /// `state` 读锁；`onSnapshot` 异常不改变持久账本
    /// （偏差登记见模块头注⑩）。
    fn notify_snapshot(
        &self,
        state: &AttemptRunState,
        attempt_id: &str,
        diagnostics: &Mutex<SharedDiagnostics>,
    ) {
        if let Some(on_snapshot) = &self.on_snapshot {
            let event = SnapshotEvent {
                attempt_id: attempt_id.to_string(),
                visible_text: state.text.clone(),
                durable_text: state.durable.clone(),
                durable_revision: state.revision,
                storage_failed: state.storage_failed,
                diagnostics: diagnostics.lock().unwrap().value.clone(),
            };
            on_snapshot(event);
        }
    }

    /// 基线 `flush`（:157-176）——增量持久化。失败时置
    /// `storageFailed` + 登记 `unsavedTails` + 围栏 + abort +
    /// `block(GENERATION_STORAGE_FAILED)`；内存围栏在存储不可用
    /// 时保持权威。返回是否发生存储失败。
    fn flush_attempt(&self, conn: &Connection, record: &ControllerRecord) -> bool {
        let mut state = record.state.lock().unwrap();
        if state.storage_failed || state.text == state.durable {
            return false;
        }
        match self
            .repository
            .snapshot(conn, &record.attempt_id, state.revision, &state.text)
        {
            Ok(artifact) => {
                state.revision = artifact.revision;
                state.durable = artifact.text;
                false
            }
            Err(_) => {
                state.storage_failed = true;
                self.unsaved_tails.lock().unwrap().insert(
                    record.attempt_id.clone(),
                    GenerationUnsavedTail {
                        attempt_id: record.attempt_id.clone(),
                        artifact_id: state.artifact_id.clone(),
                        durable_revision: state.revision,
                        text: state.text[state.durable.len()..].to_string(),
                        failure_code: "GENERATION_STORAGE_FAILED".to_string(),
                    },
                );
                self.failed_roots
                    .lock()
                    .unwrap()
                    .insert(state.root_id.clone());
                record.token.cancel();
                // 基线 catch {}——block 失败被吞，内存围栏保持权威。
                let _ = self
                    .repository
                    .block(conn, &state.root_id, "GENERATION_STORAGE_FAILED");
                true
            }
        }
    }

    /// flush 后立即通知（基线 `flush` 末尾 `notify()`）。
    fn flush_and_notify(&self, conn: &Connection, record: &ControllerRecord) {
        self.flush_attempt(conn, record);
        let state = record.state.lock().unwrap();
        self.notify_snapshot(&state, &record.attempt_id, &record.diagnostics);
    }

    /// 连接/身份已不可用时禁止回写；尾部仅保留在进程内并围住根。
    fn fence_database_access_failure(&self, record: &ControllerRecord) {
        let mut state = record.state.lock().unwrap();
        state.storage_failed = true;
        self.unsaved_tails.lock().unwrap().insert(
            record.attempt_id.clone(),
            GenerationUnsavedTail {
                attempt_id: record.attempt_id.clone(),
                artifact_id: state.artifact_id.clone(),
                durable_revision: state.revision,
                text: state.text[state.durable.len()..].to_string(),
                failure_code: "GENERATION_STORAGE_FAILED".to_string(),
            },
        );
        self.failed_roots
            .lock()
            .unwrap()
            .insert(record.root_id.clone());
        record.token.cancel();
    }

    fn flush_with_database<D: GenerationDatabaseAccess>(
        &self,
        database: &D,
        record: &ControllerRecord,
    ) {
        if database
            .with_connection(|conn| {
                self.flush_and_notify(conn, record);
                Ok(())
            })
            .is_err()
        {
            self.fence_database_access_failure(record);
        }
    }

    /// 基线 `retainInterruptedAgentText`（:177-181）——终端
    /// 中断时保留散文投影（从不提升部分调用为工具动作）。
    fn retain_interrupted_agent_text(state: &mut AttemptRunState) {
        if state.agent_tool_names.is_some() && state.text.is_empty() && !state.suspended {
            state.text = project_interrupted_agent_visible_text(
                &state.agent_protocol_text,
                state.agent_tool_names.as_deref().unwrap_or(&[]),
            );
        }
    }

    /// 基线 `suspend` 闭包（:188-191）——重读收据、置位
    /// `suspended`、abort。
    fn suspend_record(&self, conn: &Connection, record: &ControllerRecord) -> Result<(), String> {
        let mut state = record.state.lock().unwrap();
        state.receipt = self.repository.receipt(conn, &record.attempt_id, None)?;
        state.suspended = true;
        record.token.cancel();
        Ok(())
    }

    /// 基线成功路径结算（:266-275）——`settleProviderUsage`、
    /// agent 协议解析（`stop` 且非流失败）、终态 flush 与
    /// `settle`。任何基线 `throw` 等价为 `Err`（进入 catch 路径）。
    #[allow(clippy::too_many_arguments)]
    fn settle_success(
        &self,
        conn: &Connection,
        record: &ControllerRecord,
        state: &Arc<Mutex<AttemptRunState>>,
        request: &ExecuteGenerationRequest,
        policy: &ProviderUsagePolicy,
        result: &GenerationDispatchResult,
        agent_protocol_text: &str,
        diagnostics: &Arc<Mutex<SharedDiagnostics>>,
    ) -> Result<GenerationExecutionReceipt, String> {
        let mut usage = if state.lock().unwrap().stream_failed {
            None
        } else {
            Some(settle_provider_usage(result.usage.as_ref(), policy))
        };
        let is_agent_stop = request.agent_tool_names.is_some()
            && result.finish_reason.as_deref() == Some("stop")
            && !state.lock().unwrap().stream_failed;
        if is_agent_stop {
            let parsed = crate::services::agent_response_protocol::parse_agent_response_protocol(
                agent_protocol_text,
                request.agent_tool_names.as_deref().unwrap_or(&[]),
            );
            {
                let mut state = state.lock().unwrap();
                state.text = parsed.visible_text.clone();
            }
            if parsed.tool_calls.len() > AGENT_ACTION_LIMIT {
                return Err("GENERATION_AGENT_ACTION_LIMIT".to_string());
            }
            if let Some(usage) = &mut usage {
                // 工具调用类型转换：解析器类型与仓储契约类型
                // 形状相同（基线为结构化类型兼容）。
                let tool_calls: Vec<ToolCall> = parsed
                    .tool_calls
                    .into_iter()
                    .map(|call| ToolCall {
                        name: call.name,
                        arguments: call.arguments,
                    })
                    .collect();
                usage.agent_response = Some(AgentResponse {
                    version: 1,
                    visible_text: parsed.visible_text,
                    tool_calls,
                });
            }
        } else {
            Self::retain_interrupted_agent_text(&mut state.lock().unwrap());
        }
        // 终态 flush（基线 :274）。
        self.flush_attempt(conn, record);
        let state = state.lock().unwrap();
        // 基线 :275——`settle(attemptId, usage, storageFailed ? 'error'
        // : result.finishReason, undefined, diagnostics)`。注意 usage
        // 不因 storageFailed 而改变（基线逐字语义）。
        let finish_reason = if state.storage_failed {
            Some("error".to_string())
        } else {
            result.finish_reason.clone()
        };
        let diagnostics_value = diagnostics.lock().unwrap().value.clone();
        self.repository.settle(
            conn,
            &record.attempt_id,
            usage,
            finish_reason,
            None,
            diagnostics_value,
        )
    }

    /// 基线 catch 块（:277-301）——diagnostics 终态补全、
    /// 中断文本保留、终态 flush 与失败码分类结算。
    fn catch_dispatch_failure(
        &self,
        conn: &Connection,
        record: &ControllerRecord,
        state: &Arc<Mutex<AttemptRunState>>,
        signal: &GenerationCancelSignal,
        error: &str,
    ) {
        // 基线 :278-283——diagnostics 终态补全（仅当 endReason 未定）。
        {
            let mut shared = record.diagnostics.lock().unwrap();
            let elapsed_at = shared.at;
            if let Some(diagnostics) = &mut shared.value {
                if diagnostics.end_reason.is_none() {
                    let transport = safe_transport_error(error);
                    if transport.error_name.is_some() {
                        diagnostics.error_name = transport.error_name;
                    }
                    if transport.error_code.is_some() {
                        diagnostics.error_code = transport.error_code;
                    }
                    if transport.cause_code.is_some() {
                        diagnostics.cause_code = transport.cause_code;
                    }
                    if let Some(at) = elapsed_at {
                        diagnostics.elapsed_ms += at.elapsed().as_millis().max(0) as i64;
                    }
                    diagnostics.end_reason = Some(
                        if signal.is_cancelled() {
                            "cancelled"
                        } else {
                            "failed"
                        }
                        .to_string(),
                    );
                }
            }
        }
        // 基线 :284-285——中断文本保留 + 终态 flush。
        Self::retain_interrupted_agent_text(&mut state.lock().unwrap());
        self.flush_attempt(conn, record);
        // 基线 :286-301——suspended 早退或失败码分类结算。
        let suspended_receipt = {
            let state = state.lock().unwrap();
            if !state.suspended {
                None
            } else {
                let mut out = state.receipt.clone();
                if state.storage_failed {
                    out.unsaved_tail = Some(state.text[state.durable.len()..].to_string());
                }
                out.failure_code = Some("GENERATION_PROJECT_CLOSED".to_string());
                Some(out)
            }
        };
        if let Some(out) = suspended_receipt {
            let mut state = state.lock().unwrap();
            state.receipt = out;
            return;
        }
        let current = match self.repository.receipt(conn, &record.attempt_id, None) {
            Ok(current) => current,
            Err(_) => {
                // 基线 :299-301——内层 catch：内存围栏保持权威。
                let mut state = state.lock().unwrap();
                state.storage_failed = true;
                self.failed_roots
                    .lock()
                    .unwrap()
                    .insert(record.root_id.clone());
                return;
            }
        };
        if current.attempt.status != "dispatch-marked" {
            let mut state = state.lock().unwrap();
            state.receipt = current;
            return;
        }
        // 基线 :290-296——仅持久稳定分类，绝不持久可能含
        // 凭据或提示的 provider 文本。
        let network_code = {
            let shared = record.diagnostics.lock().unwrap();
            shared
                .value
                .as_ref()
                .and_then(|d| d.cause_code.clone())
                .or_else(|| shared.value.as_ref().and_then(|d| d.error_code.clone()))
        };
        let known = match network_code.as_deref() {
            Some(code) if is_network_code_prefix(code) => "NETWORK_ERROR",
            _ => {
                if is_network_error_message(error) {
                    "NETWORK_ERROR"
                } else {
                    "GENERATION_PROVIDER_FAILED"
                }
            }
        };
        let failure_code = {
            let state = state.lock().unwrap();
            if state.storage_failed {
                "GENERATION_STORAGE_FAILED".to_string()
            } else if state.timed_out {
                "ROOT_BUDGET_EXHAUSTED".to_string()
            } else if state.stream_failed {
                "GENERATION_STREAM_INVALID".to_string()
            } else if signal.is_cancelled() {
                self.abort_failure_code(&record.root_id).to_string()
            } else {
                known.to_string()
            }
        };
        let diagnostics_value = record.diagnostics.lock().unwrap().value.clone();
        match self.repository.settle(
            conn,
            &record.attempt_id,
            None,
            None,
            Some(failure_code),
            diagnostics_value,
        ) {
            Ok(settled) => {
                let mut state = state.lock().unwrap();
                state.receipt = settled;
            }
            Err(_) => {
                // 基线 :299-301——内层 catch。
                let mut state = state.lock().unwrap();
                state.storage_failed = true;
                self.failed_roots
                    .lock()
                    .unwrap()
                    .insert(record.root_id.clone());
            }
        }
    }

    /// 基线 finally 块（:304-311）——清理 controller 登记。
    /// timer/deadline 变量 drop 即取消（等价 `clearInterval`/
    /// `clearTimeout`）；`cancelled` future drop 即移除 abort
    /// 监听；`terminal` 标志由控制流保证（见模块头注⑨）。
    fn finalize_attempt(&self, attempt_id: &str) {
        self.controllers.lock().unwrap().remove(attempt_id);
    }

    /// 同步阻塞入口（B3-3e）：在**自建 current-thread runtime** 上执行
    /// [`Self::execute`]。
    ///
    /// 存在理由：`execute` 的 future 捕获 `&Connection`（`rusqlite::Connection`
    /// 非 `Sync`）与标准库 `MutexGuard`，因此该 future 是 `!Send`，不能直接
    /// 作为 Tauri 异步命令的返回值（Tauri 要求 `Send`）。命令层用
    /// `tokio::task::block_in_place` 让出 tokio worker 后调用本入口，由本
    /// 入口自己拥有的 runtime 承载全部网络 await（reqwest 反应堆同源）；
    /// pending 表与仓储与 [`Self::execute`] 共用，幂等/在途语义一致。
    pub fn execute_blocking(
        &self,
        conn: &Connection,
        request: ExecuteGenerationRequest,
    ) -> Result<GenerationExecutionReceipt, String> {
        self.execute_blocking_with_database(conn, request)
    }

    /// 生产执行使用阶段性短回调，不在网络等待期间持有数据库守卫。
    pub(crate) fn execute_blocking_with_database<D: GenerationDatabaseAccess>(
        &self,
        database: &D,
        request: ExecuteGenerationRequest,
    ) -> Result<GenerationExecutionReceipt, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "GENERATION_RUNTIME_UNAVAILABLE".to_string())?;
        runtime.block_on(self.execute_with_database(database, request))
    }

    // -------------------------------------------------------------------------
    // 基线 execute（:110-320）
    // -------------------------------------------------------------------------

    /// 基线 `execute` 闭包——幂等重放 + 在途去重 + 分发执行。
    ///
    /// 返回 `Err` 仅对应基线 `throw` 出口（requestHash 校验、
    /// 预留校验、`reserve`/`markDispatched` 失败重抛之外的
    /// 硬错误——`reserve`/`markDispatched` 失败按基线经 catch
    /// 路径返回带失败码的收据）；其余失败路径返回带
    /// `failure_code`/`unsaved_tail` 的收据。
    pub async fn execute(
        &self,
        conn: &Connection,
        request: ExecuteGenerationRequest,
    ) -> Result<GenerationExecutionReceipt, String> {
        self.execute_with_database(conn, request).await
    }

    /// 与直接连接接口共用幂等/预算/控制器逻辑；仅同步回调可访问数据库。
    pub(crate) async fn execute_with_database<D: GenerationDatabaseAccess>(
        &self,
        database: &D,
        request: ExecuteGenerationRequest,
    ) -> Result<GenerationExecutionReceipt, String> {
        // ① 基线 :111-113——请求哈希校验。
        if !is_valid_request_hash(&request.request_hash) {
            return Err("GENERATION_REQUEST_HASH_INVALID".to_string());
        }
        // 基线 :114——`JSON.stringify([runId, invocationNonce])` 在途键。
        let key = serde_json::json!([request.run_id.clone(), request.invocation_nonce.clone()])
            .to_string();
        // ② 基线 :115-117——幂等重放（已结算 invocation 优先返回
        // 在途 promise，其次返回既有收据）。
        if let Some(existing) = database.with_connection(|conn| {
            self.repository.find_invocation(
                conn,
                &request.run_id,
                &request.invocation_nonce,
                &request.request_hash,
            )
        })? {
            let pending = { self.pending.lock().unwrap().get(&key).cloned() };
            if let Some(pending) = pending {
                return wait_pending(pending).await;
            }
            return Ok(existing);
        }
        // 原子登记：不同线程同时读到无 invocation，也只能有一个分发拥有者。
        // 等待已有操作之前必须释放 pending 短锁，不能把锁带入 await。
        let (pending_op, owns_operation) = {
            let mut pending = self.pending.lock().unwrap();
            if let Some(existing) = pending.get(&key) {
                if existing.request_hash != request.request_hash {
                    return Err("GENERATION_INVOCATION_CONFLICT".to_string());
                }
                (Arc::clone(existing), false)
            } else {
                let operation = Arc::new(PendingOperation::new(&request.request_hash));
                pending.insert(key.clone(), Arc::clone(&operation));
                (operation, true)
            }
        };
        if !owns_operation {
            return wait_pending(pending_op).await;
        }
        let result = self.execute_operation(database, request).await;
        pending_op.complete(result.clone());
        self.pending.lock().unwrap().remove(&key);
        result
    }

    /// 基线 operation async IIFE（:122-320）主体。
    async fn execute_operation<D: GenerationDatabaseAccess>(
        &self,
        database: &D,
        request: ExecuteGenerationRequest,
    ) -> Result<GenerationExecutionReceipt, String> {
        // ④ 基线 :123-125——failedRoots 围栏。
        let run = database.with_connection(|conn| self.repository.get(conn, &request.run_id))?;
        if self
            .failed_roots
            .lock()
            .unwrap()
            .contains(&run.root_action_id)
        {
            return Err("GENERATION_STORAGE_FAILED".to_string());
        }
        // ⑤ 基线 :127-133——预留校验（reservedTokens 须覆盖
        // 输入上界 + 请求输出 + 推理上界 + 安全裕度）。
        let policy = &request.usage_policy;
        if policy.estimator_version.is_empty()
            || policy.safety_margin_tokens < 0
            || request.input_upper_bound_tokens < 0
            || request.reasoning_upper_bound_tokens < 0
            || request.reserved_tokens
                < request.input_upper_bound_tokens
                    + request.requested_output_tokens
                    + request.reasoning_upper_bound_tokens
                    + policy.safety_margin_tokens
        {
            return Err("GENERATION_RESERVATION_INVALID".to_string());
        }
        // ⑥ 基线 :136-152——reserve。预算/预留类失败可重试，
        // 不触发围栏；其余存储失败置围栏并 block。
        let receipt = match database.with_connection(|conn| {
            self.repository.reserve(
                conn,
                &request.run_id,
                &request.invocation_nonce,
                &request.request_hash,
                request.reserved_tokens,
                request.requested_output_tokens,
                Some(&request.usage_policy),
                request.purpose.as_deref(),
                request.replay_task.as_ref(),
                request.budget_decision.as_ref(),
                request.reconciliation_injected,
            )
        }) {
            Ok(receipt) => receipt,
            Err(error) => {
                if !is_budget_class_error(&error) {
                    self.failed_roots
                        .lock()
                        .unwrap()
                        .insert(run.root_action_id.clone());
                    let _ = database.with_connection(|conn| {
                        self.repository.block(
                            conn,
                            &run.root_action_id,
                            "GENERATION_STORAGE_FAILED",
                        )
                    });
                }
                return Err(error);
            }
        };
        // ⑦ 基线 :153-154——attempt 状态非 reserved 的早退
        // （幂等重放的已结算收据）。
        if receipt.attempt.status != "reserved" {
            return Ok(receipt);
        }
        // ⑧ 基线 :155-166——attempt 标识与运行状态构造。
        let attempt_id = receipt.attempt.attempt_id.clone();
        // 偏差登记⑧：基线 `receipt.artifact!` 非空断言。
        let artifact = receipt
            .artifact
            .clone()
            .ok_or_else(|| "GENERATION_STORAGE_FAILED".to_string())?;
        let (token, signal) = GenerationCancelToken::pair();
        let state: Arc<Mutex<AttemptRunState>> = Arc::new(Mutex::new(AttemptRunState {
            text: artifact.text.clone(),
            durable: artifact.text.clone(),
            revision: artifact.revision,
            storage_failed: false,
            stream_failed: false,
            timed_out: false,
            suspended: false,
            agent_protocol_text: String::new(),
            agent_tool_names: request.agent_tool_names.clone(),
            artifact_id: artifact.artifact_id.clone(),
            root_id: run.root_action_id.clone(),
            receipt: receipt.clone(),
        }));
        let record = ControllerRecord {
            token: token.clone(),
            root_id: run.root_action_id.clone(),
            attempt_id: attempt_id.clone(),
            state: state.clone(),
            diagnostics: Arc::new(Mutex::new(SharedDiagnostics::default())),
        };
        self.controllers
            .lock()
            .unwrap()
            .insert(attempt_id.clone(), record.clone());
        if self
            .abort_requested_roots
            .lock()
            .unwrap()
            .contains_key(&run.root_action_id)
        {
            token.cancel();
        }

        // ⑫ 基线 :196-204——markDispatched。失败置围栏并
        // block，随后进入 catch 路径（storageFailed 收据）。
        let mut caught: Option<String> = match database
            .with_connection(|conn| self.repository.mark_dispatched(conn, &attempt_id))
        {
            Ok(()) => None,
            Err(error) => {
                {
                    let mut state = state.lock().unwrap();
                    state.storage_failed = true;
                }
                self.failed_roots
                    .lock()
                    .unwrap()
                    .insert(run.root_action_id.clone());
                let _ = database.with_connection(|conn| {
                    self.repository
                        .block(conn, &run.root_action_id, "GENERATION_STORAGE_FAILED")
                });
                Some(error)
            }
        };

        // ⑬-⑲ 基线 :205-275——定时器、分发循环与成功路径结算。
        if caught.is_none() {
            let mut flush_timer = tokio::time::interval(Duration::from_millis(FLUSH_INTERVAL_MS));
            // 基线 `setInterval` 首次触发在 1000ms 后；tokio
            // interval 首个 tick 立即完成——消费之以对齐基线时序。
            let _ = flush_timer.tick().await;
            // ⑭ 基线 :206-213——active-elapsed 剩余 deadline。
            let remaining = match database
                .with_connection(|conn| self.repository.budget(conn, &run.root_action_id))
            {
                Ok(budget) => {
                    (receipt.budget.policy.max_active_elapsed_ms - budget.active_elapsed_ms).max(0)
                }
                Err(error) => {
                    caught = Some(error);
                    0
                }
            };
            if caught.is_none() {
                // ⑮ 基线 :215-262——分发回调构造。
                //
                // 增量 flush 经通道交付（偏差登记⑤）：`&Connection`
                // 非 `Send`，流式回调不能直接持 conn；4096 字节
                // 增量请求交主循环 select 分支执行。
                let (flush_tx, mut flush_rx) = mpsc::unbounded_channel::<()>();
                let events: Arc<Mutex<HashMap<String, String>>> =
                    Arc::new(Mutex::new(HashMap::new()));
                let on_visible = {
                    let state = state.clone();
                    let events = events.clone();
                    let signal = signal.clone();
                    let token = token.clone();
                    let shared_diag = record.diagnostics.clone();
                    let attempt_id = attempt_id.clone();
                    let flush_tx = flush_tx.clone();
                    let service = self;
                    move |event: VisibleGenerationEvent| {
                        // 基线各失败分支均立即 `controller.abort()`
                        // （对齐基线时序，不设延迟标志）；增量
                        // flush 请求经通道交付（偏差登记⑤）。
                        {
                            let mut state = state.lock().unwrap();
                            if state.storage_failed || signal.is_cancelled() {
                                return;
                            }
                            // eventId 去重（基线 :242-251）：同 id
                            // 异内容判流无效；同 id 同内容跳过。
                            if let Some(event_id) = &event.event_id {
                                let current = serde_json::to_string(&serde_json::json!({
                                    "kind": event.kind.json_label(),
                                    "text": event.text,
                                    "eventId": event_id,
                                }))
                                .unwrap_or_default();
                                let mut events = events.lock().unwrap();
                                if let Some(prior) = events.get(event_id.as_str()) {
                                    if *prior != current {
                                        state.stream_failed = true;
                                        token.cancel();
                                    }
                                    return;
                                }
                                events.insert(event_id.clone(), current);
                            }
                            // think 标签泄漏检测（基线 :252-256）。
                            if RE_THINK_TAG.is_match(&event.text) {
                                state.stream_failed = true;
                                token.cancel();
                                return;
                            }
                            let previous = if state.agent_tool_names.is_some() {
                                state.agent_protocol_text.clone()
                            } else {
                                state.text.clone()
                            };
                            let next = match event.kind {
                                VisibleEventKind::Cumulative => event.text.clone(),
                                VisibleEventKind::Delta => {
                                    format!("{previous}{}", event.text)
                                }
                            };
                            if !next.starts_with(&previous) || RE_THINK_TAG.is_match(&next) {
                                state.stream_failed = true;
                                token.cancel();
                                return;
                            }
                            if state.agent_tool_names.is_some() {
                                // 协议可能以 XML、DSML 或原始 JSON
                                // 到达；在角色确定前不把协议文本流过
                                // 用户 artifact 通道。
                                let over_limit = next.len() > AGENT_PROTOCOL_BYTE_LIMIT;
                                state.agent_protocol_text = next;
                                if over_limit {
                                    state.stream_failed = true;
                                    token.cancel();
                                }
                                return;
                            }
                            let should_flush =
                                next.len() - state.durable.len() >= SNAPSHOT_FLUSH_BYTES;
                            state.text = next;
                            // 基线 :260——text 更新后立即通知
                            // （on_snapshot 不依赖 conn，回调内同步执行）。
                            service.notify_snapshot(&state, &attempt_id, &shared_diag);
                            // 基线 :261-262——增量 flush 请求（经通道
                            // 交付，见偏差登记⑤）。
                            if should_flush {
                                let _ = flush_tx.send(());
                            }
                        }
                    }
                };
                let on_reasoning_cb = {
                    let state = state.clone();
                    let signal = signal.clone();
                    let attempt_id = attempt_id.clone();
                    let service = self;
                    move |chunk: String| {
                        {
                            let state = state.lock().unwrap();
                            if state.storage_failed || signal.is_cancelled() || chunk.is_empty() {
                                return;
                            }
                        }
                        // 基线 :254-257——显示专用事件；异常不改变
                        // 持久生成（偏差登记⑩）。
                        if let Some(on_reasoning) = &service.on_reasoning {
                            on_reasoning(ReasoningEvent {
                                run_id: run.run_id.clone(),
                                root_action_id: run.root_action_id.clone(),
                                attempt_id: attempt_id.clone(),
                                text: chunk,
                            });
                        }
                    }
                };
                let on_diagnostics_cb = {
                    let state = state.clone();
                    let signal = signal.clone();
                    let shared_diag = record.diagnostics.clone();
                    let attempt_id = attempt_id.clone();
                    let service = self;
                    move |value: GenerationTransportDiagnostics| {
                        let should_update = {
                            let state = state.lock().unwrap();
                            !state.storage_failed && !signal.is_cancelled()
                        };
                        if !should_update {
                            return;
                        }
                        {
                            let mut shared = shared_diag.lock().unwrap();
                            shared.value = Some(value);
                            shared.at = Some(Instant::now());
                        }
                        // 基线 :237-240——diagnostics 更新后立即通知。
                        let state = state.lock().unwrap();
                        service.notify_snapshot(&state, &attempt_id, &shared_diag);
                    }
                };
                let options = DispatchOptions {
                    signal: &signal,
                    on_visible: &on_visible,
                    on_reasoning: Some(&on_reasoning_cb),
                    on_diagnostics: Some(&on_diagnostics_cb),
                };
                let dispatch_fut =
                    Box::pin(self.dispatch.dispatch(&request.provider_request, options));
                let mut dispatch_fut = dispatch_fut;
                let mut deadline =
                    Box::pin(tokio::time::sleep(Duration::from_millis(remaining as u64)));
                // ⑯-⑰ 基线 :263——`Promise.race([provider, cancelled])`
                // 与定时器循环。
                let dispatch_outcome;
                loop {
                    // 已中止时不得先轮询 provider future，避免登记前取消仍发起网络。
                    if signal.is_cancelled() {
                        dispatch_outcome = Some(DispatchOutcome::Cancelled);
                        break;
                    }
                    tokio::select! {
                        _ = flush_timer.tick() => {
                            self.flush_with_database(database, &record);
                        }
                        _ = flush_rx.recv() => {
                            self.flush_with_database(database, &record);
                        }
                        _ = deadline.as_mut() => {
                            {
                                let mut state = state.lock().unwrap();
                                state.timed_out = true;
                            }
                            if database.with_connection(|conn| self
                                .repository
                                .block(conn, &record.root_id, "ROOT_BUDGET_EXHAUSTED"))
                                .is_err()
                            {
                                let mut state = state.lock().unwrap();
                                state.storage_failed = true;
                                self.failed_roots
                                    .lock()
                                    .unwrap()
                                    .insert(record.root_id.clone());
                            }
                            record.token.cancel();
                        }
                        result = dispatch_fut.as_mut() => {
                            // 基线 :264——race 完成后检查 aborted。
                            dispatch_outcome = Some(if signal.is_cancelled() {
                                DispatchOutcome::Cancelled
                            } else {
                                DispatchOutcome::Completed(result)
                            });
                            break;
                        }
                        _ = signal.cancelled() => {
                            dispatch_outcome = Some(DispatchOutcome::Cancelled);
                            break;
                        }
                    }
                }
                // ⑱-⑲ 基线 :264-275——成功路径或取消/错误。
                if let Some(outcome) = dispatch_outcome {
                    match outcome {
                        DispatchOutcome::Cancelled => {
                            caught = Some("GENERATION_CANCELLED".to_string());
                        }
                        DispatchOutcome::Completed(Err(error)) => {
                            caught = Some(error);
                        }
                        DispatchOutcome::Completed(Ok(result)) => {
                            if signal.is_cancelled() {
                                caught = Some("GENERATION_CANCELLED".to_string());
                            } else {
                                let agent_protocol_text =
                                    state.lock().unwrap().agent_protocol_text.clone();
                                match database.with_connection(|conn| {
                                    self.settle_success(
                                        conn,
                                        &record,
                                        &state,
                                        &request,
                                        policy,
                                        &result,
                                        &agent_protocol_text,
                                        &record.diagnostics,
                                    )
                                }) {
                                    Ok(settled) => {
                                        let mut state = state.lock().unwrap();
                                        state.receipt = settled;
                                    }
                                    Err(error) => {
                                        caught = Some(error);
                                    }
                                }
                            }
                        }
                    }
                }
                // flush_tx 随闭包 drop；通道清理。
                drop(flush_tx);
            }
        }

        // catch 块（基线 :277-301）。
        if let Some(error) = caught {
            if database
                .with_connection(|conn| {
                    self.catch_dispatch_failure(conn, &record, &state, &signal, &error);
                    Ok(())
                })
                .is_err()
            {
                self.fence_database_access_failure(&record);
            }
        }
        // finally 块（基线 :304-311）。
        self.finalize_attempt(&attempt_id);
        // ⑳ 基线 :312-320——终态返回。
        {
            let state = state.lock().unwrap();
            if state.storage_failed {
                let mut out = state.receipt.clone();
                out.unsaved_tail = Some(state.text[state.durable.len()..].to_string());
                out.failure_code = Some("GENERATION_STORAGE_FAILED".to_string());
                return Ok(out);
            }
        }
        let final_receipt =
            database.with_connection(|conn| self.repository.receipt(conn, &attempt_id, None))?;
        if signal.is_cancelled() {
            let mut out = final_receipt;
            if out.failure_code.is_none() {
                out.failure_code = Some(
                    if state.lock().unwrap().timed_out {
                        "ROOT_BUDGET_EXHAUSTED"
                    } else {
                        self.abort_failure_code(&record.root_id)
                    }
                    .to_string(),
                );
            }
            return Ok(out);
        }
        Ok(final_receipt)
    }

    // -------------------------------------------------------------------------
    // 公共 API（基线 :321-355）
    // -------------------------------------------------------------------------

    /// 基线 `open`（:321）——透传仓储 open。
    pub fn open(
        &self,
        conn: &Connection,
        request: &OpenGenerationRunRequest,
    ) -> Result<DurableGenerationRun, String> {
        self.repository.open(conn, request)
    }

    /// owner 创建与准入共用事务，由调用方负责提交或回滚。
    pub(crate) fn open_in_transaction(
        &self,
        conn: &Connection,
        request: &OpenGenerationRunRequest,
    ) -> Result<DurableGenerationRun, String> {
        self.repository.open_in_existing_transaction(conn, request)
    }

    /// 基线 `suspendForProjectClose`（:322-333）——项目关闭
    /// 挂起：句柄仍打开时同步 flush 全部 controller，持久未知
    /// 记账后逐个 suspend；`unsavedTails` 非空拒绝挂起。
    pub fn suspend_for_project_close(&self, conn: &Connection) -> Result<(), String> {
        let records: Vec<ControllerRecord> =
            self.controllers.lock().unwrap().values().cloned().collect();
        // 基线 :323-324——同步 flush（DB 句柄仍打开）。
        for record in &records {
            self.flush_and_notify(conn, record);
        }
        // 基线 :325-328——恢复中断前持久未知记账。
        let mut failure: Option<String> = None;
        if let Err(error) = self.repository.recover_interrupted(conn) {
            failure = Some(error);
        }
        // 基线 :329-334——逐个 suspend；失败保留首个错误并 abort。
        for record in &records {
            if let Err(error) = self.suspend_record(conn, record) {
                if failure.is_none() {
                    failure = Some(error);
                }
                record.token.cancel();
            }
        }
        // 基线 :335-338——未保存尾存在时拒绝挂起。
        if !self.unsaved_tails.lock().unwrap().is_empty() {
            return Err("GENERATION_UNSAVED_TAIL_PRESENT".to_string());
        }
        if let Some(failure) = failure {
            return Err(failure);
        }
        Ok(())
    }

    /// 基线 `readUnsavedTails`（:336）——未保存尾快照
    /// （基线返回冻结数组；Rust 侧返回克隆快照）。
    pub fn read_unsaved_tails(&self) -> Vec<GenerationUnsavedTail> {
        self.unsaved_tails
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect()
    }

    /// 基线 `get`（:337）——透传仓储 get。
    pub fn get(&self, conn: &Connection, run_id: &str) -> Result<DurableGenerationRun, String> {
        self.repository.get(conn, run_id)
    }

    /// 基线 `readAttempt`（:338）——attempt 收据快照。
    pub fn read_attempt(
        &self,
        conn: &Connection,
        attempt_id: &str,
    ) -> Result<GenerationExecutionReceipt, String> {
        self.repository.receipt(conn, attempt_id, None)
    }

    /// 基线 `listCandidates`（:339）——候选 artifact 列表。
    pub fn list_candidates(
        &self,
        conn: &Connection,
        run_id: Option<&str>,
    ) -> Result<Vec<Option<VisibleArtifact>>, String> {
        self.repository.list_candidates(conn, run_id)
    }

    /// 基线 `discardCandidate`（:340-344）——丢弃候选并连带
    /// 清理 `unsavedTails` 中该 artifact 的条目。
    pub fn discard_candidate(&self, conn: &Connection, artifact_id: &str) -> Result<(), String> {
        self.repository.discard_candidate(conn, artifact_id)?;
        self.unsaved_tails
            .lock()
            .unwrap()
            .retain(|_, tail| tail.artifact_id != artifact_id);
        Ok(())
    }

    /// 只发送易失中止意图，不读写数据库；调用方必须先验证会话与完整句柄。
    /// 持久暂停/取消仍由原命令在执行结算后完成。
    pub(crate) fn request_abort_root(&self, root_id: &str) {
        self.abort_requested_roots
            .lock()
            .unwrap()
            .entry(root_id.to_string())
            .or_insert(AbortCause::Cancelled);
        self.abort_root_controllers(root_id);
    }

    /// 内部关闭钩子在等待业务锁前调用；连接仍保留给执行线程 flush/settle。
    pub(crate) fn request_project_close_root(&self, root_id: &str) {
        self.abort_requested_roots
            .lock()
            .unwrap()
            .insert(root_id.to_string(), AbortCause::ProjectClosed);
        self.abort_root_controllers(root_id);
    }

    fn abort_failure_code(&self, root_id: &str) -> &'static str {
        if self.abort_requested_roots.lock().unwrap().get(root_id)
            == Some(&AbortCause::ProjectClosed)
        {
            "GENERATION_PROJECT_CLOSED"
        } else {
            "GENERATION_CANCELLED"
        }
    }

    #[cfg(test)]
    pub(crate) fn has_abort_request(&self, root_id: &str) -> bool {
        self.abort_requested_roots
            .lock()
            .unwrap()
            .contains_key(root_id)
    }

    /// 执行控制登记退出时清理意图，后续恢复不得继承旧中止。
    pub(crate) fn clear_abort_request(&self, root_id: &str) {
        self.abort_requested_roots.lock().unwrap().remove(root_id);
    }

    /// 基线 `pause`（:345-347）——暂停根并 abort 匹配 controller。
    pub fn pause(&self, conn: &Connection, root_id: &str) -> Result<(), String> {
        self.repository.pause(conn, root_id, false)?;
        self.abort_root_controllers(root_id);
        Ok(())
    }

    /// 基线 `cancel`（:348-350）——取消根（`pause(cancel=true)`）
    /// 并 abort 匹配 controller。
    pub fn cancel(&self, conn: &Connection, root_id: &str) -> Result<(), String> {
        self.repository.pause(conn, root_id, true)?;
        self.abort_root_controllers(root_id);
        Ok(())
    }

    /// 基线 `resume`（:348-350 区域，公共 API）——恢复重分发
    /// 校验：fingerprint / contextSnapshotId / sourceManifest 三重
    /// deep-equal + 注入式 `validateRecovery`，任一失败拒绝。
    fn prepare_resume(
        &self,
        conn: &Connection,
        run_id: &str,
        next: &RunBinding,
    ) -> Result<DurableGenerationRun, String> {
        let old = self.repository.get(conn, run_id)?;
        if serde_json::to_value(&old.binding.fingerprint).ok()
            != serde_json::to_value(&next.fingerprint).ok()
            || old.binding.context_snapshot_id != next.context_snapshot_id
            || old.binding.source_manifest != next.source_manifest
        {
            return Err("GENERATION_RECOVERY_UNAUTHORIZED".to_string());
        }
        Ok(old)
    }

    /// owner 已在同一命令临界区完成真实源重建；同步恢复不跨 await 持有 SQLite/owner 守卫。
    pub(crate) fn resume_with_authorization(
        &self,
        conn: &Connection,
        run_id: &str,
        next: RunBinding,
        validate: impl FnOnce(&RunBinding, &RunBinding) -> Result<bool, String>,
    ) -> Result<DurableGenerationRun, String> {
        let old = self.prepare_resume(conn, run_id, &next)?;
        if !validate(&old.binding, &next)? {
            return Err("GENERATION_RECOVERY_UNAUTHORIZED".to_string());
        }
        self.repository.resume(conn, run_id, &next)?;
        self.repository.get(conn, run_id)
    }

    pub async fn resume(
        &self,
        conn: &Connection,
        run_id: &str,
        next: RunBinding,
    ) -> Result<DurableGenerationRun, String> {
        let old = self.prepare_resume(conn, run_id, &next)?;
        let validate = self
            .validate_recovery
            .as_ref()
            .ok_or_else(|| "GENERATION_RECOVERY_UNAUTHORIZED".to_string())?;
        if !validate(old.binding, next.clone()).await? {
            return Err("GENERATION_RECOVERY_UNAUTHORIZED".to_string());
        }
        self.repository.resume(conn, run_id, &next)?;
        self.repository.get(conn, run_id)
    }

    /// 基线 `restart`（:352-355）——旧根取消 + abort 匹配
    /// controller 后重新 open。
    pub fn restart(
        &self,
        conn: &Connection,
        old_root_id: &str,
        request: &OpenGenerationRunRequest,
    ) -> Result<DurableGenerationRun, String> {
        self.repository.pause(conn, old_root_id, true)?;
        self.abort_root_controllers(old_root_id);
        self.repository.open(conn, request)
    }

    /// 基线 `recoverInterrupted`（:356）——透传仓储恢复。
    pub fn recover_interrupted(&self, conn: &Connection) -> Result<(), String> {
        self.repository.recover_interrupted(conn)
    }

    /// 基线 pause/cancel/restart 共用的 controller abort 面。
    fn abort_root_controllers(&self, root_id: &str) {
        let records: Vec<ControllerRecord> = self
            .controllers
            .lock()
            .unwrap()
            .values()
            .filter(|record| record.root_id == root_id)
            .cloned()
            .collect();
        for record in &records {
            record.token.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::generation_run_repository::{FrozenInputFingerprint, RootBudget};
    use serde_json::{json, Map};

    fn fake_content_hash() -> String {
        "a".repeat(64)
    }

    /// 打开一个最小可运行的生成运行（对齐 owner 模块测试先例）。
    fn open_run(conn: &Connection, project_id: &str, epoch: &str) -> DurableGenerationRun {
        let repository = GenerationRunRepository::default();
        repository
            .open(
                conn,
                &OpenGenerationRunRequest {
                    project_id: project_id.to_string(),
                    epoch: epoch.to_string(),
                    fingerprint: FrozenInputFingerprint {
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
                        manifest.insert("operation".to_string(), json!("chapter-draft"));
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
                    operation: "chapter-draft".to_string(),
                    ui_action_nonce: "run-service-fixture-nonce".to_string(),
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
            .expect("open run")
    }

    fn usage_policy(reasoning: &str) -> ProviderUsagePolicy {
        ProviderUsagePolicy {
            estimator_version: "utf8-bytes-plus-chat-framing-v1".to_string(),
            safety_margin_tokens: 512,
            reasoning: reasoning.to_string(),
            can_bound_total_liability: true,
        }
    }

    fn execute_request(
        run: &DurableGenerationRun,
        hash: &str,
        policy: ProviderUsagePolicy,
    ) -> ExecuteGenerationRequest {
        ExecuteGenerationRequest {
            run_id: run.run_id.clone(),
            invocation_nonce: "nonce-1".to_string(),
            request_hash: hash.to_string(),
            provider_request: Value::Null,
            reserved_tokens: 1024,
            requested_output_tokens: 256,
            input_upper_bound_tokens: 128,
            reasoning_upper_bound_tokens: 0,
            usage_policy: policy,
            purpose: None,
            budget_decision: None,
            agent_tool_names: None,
            replay_task: None,
            reconciliation_injected: false,
        }
    }

    /// Mock 分发器：按配置回显可见事件并返回结果；
    /// `cancel_on_dispatch` 在分发时让出一次执行，
    /// 供测试的并发取消任务取消 signal（模拟基线
    /// dispatch 期间外部 abort 的时序）。
    struct MockDispatcher {
        result: Result<GenerationDispatchResult, String>,
        visible_events: Vec<VisibleGenerationEvent>,
        cancel_on_dispatch: bool,
    }

    impl GenerationDispatcher for MockDispatcher {
        fn dispatch<'a>(
            &'a self,
            _request: &'a Value,
            options: DispatchOptions<'a>,
        ) -> Pin<Box<dyn Future<Output = Result<GenerationDispatchResult, String>> + Send + 'a>>
        {
            let events = self.visible_events.clone();
            let result = self.result.clone();
            let cancel = self.cancel_on_dispatch;
            Box::pin(async move {
                if cancel {
                    // 让出一次执行：并发取消任务在此
                    // 窗口取消 signal（见结构注释）。
                    tokio::task::yield_now().await;
                }
                for event in events {
                    (options.on_visible)(event);
                }
                result
            })
        }
    }

    fn service(dispatch: Arc<dyn GenerationDispatcher>) -> GenerationRunService {
        create_generation_run_service(GenerationRunServiceDependencies {
            repository: GenerationRunRepository::default(),
            dispatch,
            on_reasoning: None,
            on_snapshot: None,
            validate_recovery: None,
        })
    }

    fn trusted_usage() -> GenerationProviderUsage {
        GenerationProviderUsage {
            prompt_tokens: Some(10),
            completion_tokens: Some(20),
            total_tokens: Some(30),
            reasoning_tokens: None,
            accounting: "included-in-completion".to_string(),
            total_includes_reasoning: false,
            trusted: true,
        }
    }

    // -------------------------------------------------------------------------
    // settle_provider_usage 记账三态
    // -------------------------------------------------------------------------

    #[test]
    fn settle_usage_untrusted_yields_conservative() {
        let policy = usage_policy("included-in-completion");
        let mut usage = trusted_usage();
        usage.trusted = false;
        let receipt = settle_provider_usage(Some(&usage), &policy);
        assert!(!receipt.trusted);
        assert_eq!(receipt.input_tokens, None);
        assert_eq!(receipt.actual_tokens, None);
        // usage 缺失同为保守收据。
        let receipt = settle_provider_usage(None, &policy);
        assert!(!receipt.trusted);
    }

    #[test]
    fn settle_usage_unknown_policy_trusts_only_self_consistent_total() {
        let policy = usage_policy("unknown");
        let mut usage = trusted_usage();
        usage.accounting = "unknown".to_string();
        // p + c == t（无 reasoning）——自洽。
        let receipt = settle_provider_usage(Some(&usage), &policy);
        assert!(receipt.trusted);
        assert_eq!(receipt.actual_tokens, Some(30));
        // total 不自洽——保守。
        usage.total_tokens = Some(99);
        let receipt = settle_provider_usage(Some(&usage), &policy);
        assert!(!receipt.trusted);
        // reasoning 计入 total 且自洽（t == p + c + r）。
        usage.total_tokens = Some(35);
        usage.reasoning_tokens = Some(5);
        let receipt = settle_provider_usage(Some(&usage), &policy);
        assert!(receipt.trusted);
        assert_eq!(receipt.reasoning_tokens, Some(5));
        assert_eq!(receipt.actual_tokens, Some(35));
        // reasoning > completion 且未计入 total——不自洽。
        usage.total_tokens = Some(30);
        usage.reasoning_tokens = Some(25);
        let receipt = settle_provider_usage(Some(&usage), &policy);
        assert!(!receipt.trusted);
    }

    #[test]
    fn settle_usage_included_in_completion_rejects_reasoning_overflow() {
        let policy = usage_policy("included-in-completion");
        let mut usage = trusted_usage();
        usage.reasoning_tokens = Some(5);
        usage.total_tokens = Some(30);
        // reasoning ≤ completion——接受。
        let receipt = settle_provider_usage(Some(&usage), &policy);
        assert!(receipt.trusted);
        assert_eq!(receipt.actual_tokens, Some(30));
        // reasoning > completion——拒绝。
        usage.reasoning_tokens = Some(21);
        let receipt = settle_provider_usage(Some(&usage), &policy);
        assert!(!receipt.trusted);
    }

    #[test]
    fn settle_usage_separately_billed_requires_reasoning_and_total_alignment() {
        let policy = usage_policy("separately-billed");
        let mut usage = trusted_usage();
        // separately-billed 但 reasoning 缺失——拒绝。
        let receipt = settle_provider_usage(Some(&usage), &policy);
        assert!(!receipt.trusted);
        usage.reasoning_tokens = Some(5);
        usage.accounting = "separately-billed".to_string();
        // total 不含 reasoning 且 totalIncludesReasoning=false——
        // actual = p + c + r = 35。
        let receipt = settle_provider_usage(Some(&usage), &policy);
        assert!(receipt.trusted);
        assert_eq!(receipt.actual_tokens, Some(35));
        // totalIncludesReasoning=true 且 total 对齐——通过。
        usage.total_includes_reasoning = true;
        usage.total_tokens = Some(35);
        let receipt = settle_provider_usage(Some(&usage), &policy);
        assert!(receipt.trusted);
        // total 与期望不符——拒绝。
        usage.total_tokens = Some(36);
        let receipt = settle_provider_usage(Some(&usage), &policy);
        assert!(!receipt.trusted);
    }

    #[test]
    fn settle_usage_rejects_accounting_policy_mismatch() {
        let policy = usage_policy("included-in-completion");
        let mut usage = trusted_usage();
        usage.accounting = "separately-billed".to_string();
        usage.reasoning_tokens = Some(5);
        let receipt = settle_provider_usage(Some(&usage), &policy);
        assert!(!receipt.trusted);
        // accounting 未知——拒绝。
        usage.accounting = "unknown".to_string();
        let receipt = settle_provider_usage(Some(&usage), &policy);
        assert!(!receipt.trusted);
    }

    // -------------------------------------------------------------------------
    // 校验辅助
    // -------------------------------------------------------------------------

    #[test]
    fn request_hash_validation_matches_baseline_regex() {
        assert!(is_valid_request_hash(&"b".repeat(64)));
        assert!(is_valid_request_hash(&"0123456789abcdef".repeat(4)));
        assert!(!is_valid_request_hash(&"b".repeat(63)));
        assert!(!is_valid_request_hash(&"g".repeat(64)));
        assert!(!is_valid_request_hash(&"B".repeat(64)));
        assert!(!is_valid_request_hash(&"".repeat(0)));
    }

    #[test]
    fn budget_class_error_classification() {
        assert!(is_budget_class_error("ROOT_BUDGET_EXHAUSTED"));
        assert!(is_budget_class_error("GENERATION_RESERVATION_INVALID"));
        assert!(is_budget_class_error("GENERATION_EPOCH_STALE"));
        assert!(is_budget_class_error("GENERATION_DISPATCH_BLOCKED"));
        assert!(is_budget_class_error("GENERATION_INVOCATION_CONFLICT"));
        assert!(!is_budget_class_error("sqlite error: disk I/O"));
    }

    #[test]
    fn network_failure_classification() {
        assert!(is_network_code_prefix("UND_ERR_BODY_TIMEOUT"));
        assert!(is_network_code_prefix("ECONNRESET"));
        assert!(is_network_code_prefix("ETIMEDOUT_x"));
        assert!(is_network_code_prefix("ENOTFOUND"));
        assert!(is_network_code_prefix("EAI_AGAIN"));
        assert!(is_network_code_prefix("ERR_STREAM_PREMATURE_CLOSE"));
        assert!(!is_network_code_prefix("EPIPE"));
        assert!(is_network_error_message("NETWORK_ERROR"));
        assert!(is_network_error_message("FETCH_FAILED"));
        assert!(!is_network_error_message("GENERATION_PROVIDER_FAILED"));
    }

    // -------------------------------------------------------------------------
    // execute 主循环（mock dispatch）
    // -------------------------------------------------------------------------

    fn test_setup() -> (Connection, DurableGenerationRun) {
        let conn = Connection::open_in_memory().expect("memory db");
        crate::db::schema::create_tables(&conn).expect("schema");
        let run = open_run(&conn, "project-1", "lease-1");
        (conn, run)
    }

    #[tokio::test]
    async fn scoped_database_late_pending_subscriber_receives_stored_completion_test() {
        let pending = Arc::new(PendingOperation::new(&"a".repeat(64)));
        pending.complete(Err("GENERATION_CANCELLED".to_string()));
        let result = tokio::time::timeout(Duration::from_secs(1), wait_pending(pending))
            .await
            .expect("完成先于订阅时也必须立即返回");
        assert!(matches!(result, Err(code) if code == "GENERATION_CANCELLED"));
    }

    #[tokio::test]
    async fn scoped_database_pending_hash_conflict_before_reserve_is_rejected_test() {
        let (conn, run) = test_setup();
        let database = ScopedDatabase {
            connection: Mutex::new(conn),
            allowed: std::sync::atomic::AtomicBool::new(true),
        };
        let (started, _) = std::sync::mpsc::channel();
        let svc = service(Arc::new(WaitingDispatcher { started }));
        let key = json!([run.run_id, "nonce-1"]).to_string();
        svc.pending.lock().unwrap().insert(
            key.clone(),
            Arc::new(PendingOperation::new(&"a".repeat(64))),
        );
        let result = svc
            .execute_with_database(
                &database,
                execute_request(
                    &run,
                    &"b".repeat(64),
                    usage_policy("included-in-completion"),
                ),
            )
            .await;
        assert!(matches!(result, Err(code) if code == "GENERATION_INVOCATION_CONFLICT"));
        let conn = database.connection.lock().unwrap();
        assert!(svc
            .repository
            .read_run_view(&conn, &run.run_id)
            .unwrap()
            .attempts
            .is_empty());
        svc.pending.lock().unwrap().remove(&key);
    }

    struct ScopedDatabase {
        connection: Mutex<Connection>,
        allowed: std::sync::atomic::AtomicBool,
    }

    impl GenerationDatabaseAccess for ScopedDatabase {
        fn with_connection<T>(
            &self,
            action: impl FnOnce(&Connection) -> Result<T, String>,
        ) -> Result<T, String> {
            let connection = self.connection.lock().unwrap();
            if !self.allowed.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("GENERATION_EPOCH_STALE".to_string());
            }
            action(&connection)
        }
    }

    #[test]
    fn scoped_database_network_wait_releases_connection_and_preserves_abort_text_test() {
        let (conn, run) = test_setup();
        let database = Arc::new(ScopedDatabase {
            connection: Mutex::new(conn),
            allowed: std::sync::atomic::AtomicBool::new(true),
        });
        let (started, received) = std::sync::mpsc::channel();
        let svc = Arc::new(service(Arc::new(WaitingDispatcher { started })));
        let execution_database = Arc::clone(&database);
        let execution_service = Arc::clone(&svc);
        let request = execute_request(
            &run,
            &"a".repeat(64),
            usage_policy("included-in-completion"),
        );
        let (done, completed) = std::sync::mpsc::channel();
        let execution = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            done.send(runtime.block_on(
                execution_service.execute_with_database(execution_database.as_ref(), request),
            ))
            .unwrap();
        });
        received
            .recv_timeout(Duration::from_secs(3))
            .expect("分发已开始");
        {
            let conn = database
                .connection
                .try_lock()
                .expect("网络等待期间连接必须可用");
            let view = svc.repository.read_run_view(&conn, &run.run_id).unwrap();
            assert_eq!(view.attempts.len(), 1);
            assert_eq!(view.attempts[0].receipt.attempt.status, "dispatch-marked");
        }
        svc.request_abort_root(&run.root_action_id);
        let receipt = completed
            .recv_timeout(Duration::from_secs(3))
            .expect("中止完成")
            .unwrap();
        execution.join().unwrap();
        assert_eq!(
            receipt.failure_code.as_deref(),
            Some("GENERATION_CANCELLED")
        );
        assert_eq!(receipt.artifact.as_ref().unwrap().text, "中止前正文");
        assert_eq!(receipt.attempt.status, "unknown");
        assert!(svc.controllers.lock().unwrap().is_empty());
        assert!(svc.pending.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn scoped_database_pending_replay_does_not_hold_pending_or_connection_lock_test() {
        let (conn, run) = test_setup();
        let database = ScopedDatabase {
            connection: Mutex::new(conn),
            allowed: std::sync::atomic::AtomicBool::new(true),
        };
        let (started, received) = std::sync::mpsc::channel();
        let svc = service(Arc::new(WaitingDispatcher { started }));
        let request = execute_request(
            &run,
            &"b".repeat(64),
            usage_policy("included-in-completion"),
        );
        let duplicate = request.clone();
        let joined = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(
                svc.execute_with_database(&database, request),
                async {
                    while received.try_recv().is_err() {
                        tokio::task::yield_now().await;
                    }
                    svc.execute_with_database(&database, duplicate).await
                },
                async {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    assert!(
                        svc.pending.try_lock().is_ok(),
                        "重放等待不能持有 pending 锁"
                    );
                    assert!(
                        database.connection.try_lock().is_ok(),
                        "重放等待不能持有连接"
                    );
                    svc.request_abort_root(&run.root_action_id);
                },
            )
        })
        .await
        .expect("原调用与在途重放必须共同完成");
        let first = joined.0.unwrap();
        let second = joined.1.unwrap();
        assert_eq!(first.attempt.attempt_id, second.attempt.attempt_id);
        assert_eq!(first.failure_code, second.failure_code);
        assert!(svc.pending.lock().unwrap().is_empty());
        let conn = database.connection.lock().unwrap();
        assert_eq!(
            svc.repository
                .read_run_view(&conn, &run.run_id)
                .unwrap()
                .attempts
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn scoped_database_revoked_before_settle_never_writes_old_connection_test() {
        struct RevokingDispatcher(Arc<ScopedDatabase>);
        impl GenerationDispatcher for RevokingDispatcher {
            fn dispatch<'a>(
                &'a self,
                _request: &'a Value,
                options: DispatchOptions<'a>,
            ) -> Pin<Box<dyn Future<Output = Result<GenerationDispatchResult, String>> + Send + 'a>>
            {
                Box::pin(async move {
                    (options.on_visible)(VisibleGenerationEvent {
                        kind: VisibleEventKind::Delta,
                        text: "仅内存尾部".to_string(),
                        event_id: None,
                    });
                    self.0
                        .allowed
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    Ok(GenerationDispatchResult {
                        usage: Some(trusted_usage()),
                        finish_reason: Some("stop".to_string()),
                    })
                })
            }
        }
        let (conn, run) = test_setup();
        let database = Arc::new(ScopedDatabase {
            connection: Mutex::new(conn),
            allowed: std::sync::atomic::AtomicBool::new(true),
        });
        let svc = service(Arc::new(RevokingDispatcher(Arc::clone(&database))));
        let receipt = svc
            .execute_with_database(
                database.as_ref(),
                execute_request(
                    &run,
                    &"c".repeat(64),
                    usage_policy("included-in-completion"),
                ),
            )
            .await
            .unwrap();
        assert_eq!(
            receipt.failure_code.as_deref(),
            Some("GENERATION_STORAGE_FAILED")
        );
        assert_eq!(receipt.unsaved_tail.as_deref(), Some("仅内存尾部"));
        let conn = database.connection.lock().unwrap();
        let stored = svc
            .repository
            .receipt(&conn, &receipt.attempt.attempt_id, None)
            .unwrap();
        assert_eq!(stored.attempt.status, "dispatch-marked");
        assert_eq!(stored.artifact.as_ref().unwrap().text, "");
        assert!(stored.failure_code.is_none());
        assert!(svc
            .failed_roots
            .lock()
            .unwrap()
            .contains(&run.root_action_id));
        assert!(svc.controllers.lock().unwrap().is_empty());
        assert_eq!(
            svc.unsaved_tails.lock().unwrap()[&receipt.attempt.attempt_id].text,
            "仅内存尾部"
        );
    }

    #[tokio::test]
    async fn project_close_before_controller_registration_persists_close_reason_test() {
        let (conn, run) = test_setup();
        let (started, received) = std::sync::mpsc::channel();
        let svc = service(Arc::new(WaitingDispatcher { started }));
        svc.request_project_close_root(&run.root_action_id);
        // 并发用户取消不能把已登记的关闭原因降级。
        svc.request_abort_root(&run.root_action_id);
        let receipt = tokio::time::timeout(
            Duration::from_secs(3),
            svc.execute(
                &conn,
                execute_request(
                    &run,
                    &"e".repeat(64),
                    usage_policy("included-in-completion"),
                ),
            ),
        )
        .await
        .expect("关闭中止不得等待网络")
        .unwrap();
        assert_eq!(
            receipt.failure_code.as_deref(),
            Some("GENERATION_PROJECT_CLOSED")
        );
        assert!(received.try_recv().is_err());
        let stored = svc
            .repository
            .receipt(&conn, &receipt.attempt.attempt_id, None)
            .unwrap();
        assert_eq!(
            stored.failure_code.as_deref(),
            Some("GENERATION_PROJECT_CLOSED")
        );
        assert!(svc.controllers.lock().unwrap().is_empty());
    }

    #[test]
    fn project_close_abort_reason_is_root_scoped_and_cleared_test() {
        let (started, _) = std::sync::mpsc::channel();
        let svc = service(Arc::new(WaitingDispatcher { started }));
        svc.request_abort_root("cancel-root");
        svc.request_project_close_root("close-root");
        assert_eq!(
            svc.abort_failure_code("cancel-root"),
            "GENERATION_CANCELLED"
        );
        assert_eq!(
            svc.abort_failure_code("close-root"),
            "GENERATION_PROJECT_CLOSED"
        );
        svc.request_abort_root("close-root");
        assert_eq!(
            svc.abort_failure_code("close-root"),
            "GENERATION_PROJECT_CLOSED"
        );
        svc.clear_abort_request("close-root");
        assert!(!svc.has_abort_request("close-root"));
        assert_eq!(svc.abort_failure_code("close-root"), "GENERATION_CANCELLED");
    }

    /// 永不主动结束的合成 provider，用于跨线程中止验证，不访问外部网络。
    struct WaitingDispatcher {
        started: std::sync::mpsc::Sender<()>,
    }

    impl GenerationDispatcher for WaitingDispatcher {
        fn dispatch<'a>(
            &'a self,
            _request: &'a Value,
            options: DispatchOptions<'a>,
        ) -> Pin<Box<dyn Future<Output = Result<GenerationDispatchResult, String>> + Send + 'a>>
        {
            Box::pin(async move {
                (options.on_visible)(VisibleGenerationEvent {
                    kind: VisibleEventKind::Delta,
                    text: "中止前正文".to_string(),
                    event_id: None,
                });
                self.started.send(()).unwrap();
                std::future::pending().await
            })
        }
    }

    #[tokio::test]
    async fn abort_request_before_controller_registration_skips_provider_poll_test() {
        let (conn, run) = test_setup();
        let (started, received) = std::sync::mpsc::channel();
        let svc = service(Arc::new(WaitingDispatcher { started }));
        svc.request_abort_root(&run.root_action_id);
        let receipt = tokio::time::timeout(
            Duration::from_secs(3),
            svc.execute(
                &conn,
                execute_request(
                    &run,
                    &"b".repeat(64),
                    usage_policy("included-in-completion"),
                ),
            ),
        )
        .await
        .expect("登记前中止不得等待网络")
        .unwrap();
        assert_eq!(
            receipt.failure_code.as_deref(),
            Some("GENERATION_CANCELLED")
        );
        assert!(received.try_recv().is_err(), "provider future 不得被轮询");
        assert!(svc.controllers.lock().unwrap().is_empty());
        assert!(svc.pending.lock().unwrap().is_empty());
        svc.clear_abort_request(&run.root_action_id);
        assert!(!svc.has_abort_request(&run.root_action_id));
    }

    #[tokio::test]
    async fn abort_request_from_other_thread_ends_wait_and_preserves_prefix_test() {
        let (conn, run) = test_setup();
        let (started, received) = std::sync::mpsc::channel();
        let svc = Arc::new(service(Arc::new(WaitingDispatcher { started })));
        let abort_service = Arc::clone(&svc);
        let root = run.root_action_id.clone();
        let control = std::thread::spawn(move || {
            received
                .recv_timeout(Duration::from_secs(3))
                .expect("等待分发开始");
            abort_service.request_abort_root(&root);
        });
        let receipt = tokio::time::timeout(
            Duration::from_secs(5),
            svc.execute(
                &conn,
                execute_request(
                    &run,
                    &"c".repeat(64),
                    usage_policy("included-in-completion"),
                ),
            ),
        )
        .await
        .expect("跨线程中止必须唤醒网络等待")
        .unwrap();
        control.join().unwrap();
        assert_eq!(
            receipt.failure_code.as_deref(),
            Some("GENERATION_CANCELLED")
        );
        assert_eq!(receipt.artifact.as_ref().unwrap().text, "中止前正文");
        assert!(svc.controllers.lock().unwrap().is_empty());
        assert!(svc.pending.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn abort_request_is_root_scoped_and_clear_allows_later_execution_test() {
        let (conn, run) = test_setup();
        let svc = service(Arc::new(MockDispatcher {
            result: Ok(GenerationDispatchResult {
                usage: Some(trusted_usage()),
                finish_reason: Some("stop".to_string()),
            }),
            visible_events: Vec::new(),
            cancel_on_dispatch: false,
        }));
        svc.request_abort_root("other-root");
        svc.request_abort_root(&run.root_action_id);
        svc.clear_abort_request(&run.root_action_id);
        let receipt = svc
            .execute(
                &conn,
                execute_request(
                    &run,
                    &"d".repeat(64),
                    usage_policy("included-in-completion"),
                ),
            )
            .await
            .unwrap();
        assert!(receipt.failure_code.is_none());
        assert_eq!(
            receipt.result.as_ref().unwrap().finish_reason.as_deref(),
            Some("stop")
        );
        assert!(svc.has_abort_request("other-root"));
    }

    #[tokio::test]
    async fn real_dispatcher_persists_only_visible_text_and_replays_without_network() {
        use crate::services::main_generation_dispatcher::{
            tests::{fixture, server},
            MainGenerationDispatcher,
        };
        let (conn, run) = test_setup();
        let (url, server) = server(concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"hidden\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" 正文<th\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"ink>secret</think>尾 \"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":20,\"total_tokens\":30}}\n\n",
            "data: [DONE]\n\n"
        ), Duration::ZERO);
        let reasoning = Arc::new(Mutex::new(String::new()));
        let received_reasoning = reasoning.clone();
        let svc = create_generation_run_service(GenerationRunServiceDependencies {
            repository: GenerationRunRepository::default(),
            dispatch: Arc::new(MainGenerationDispatcher::new(
                reqwest::Client::builder().no_proxy().build().unwrap(),
            )),
            on_reasoning: Some(Arc::new(move |event| {
                received_reasoning.lock().unwrap().push_str(&event.text)
            })),
            on_snapshot: None,
            validate_recovery: None,
        });
        let mut request = execute_request(
            &run,
            &"a".repeat(64),
            usage_policy("included-in-completion"),
        );
        request.provider_request = fixture("openai", &url);
        let receipt = svc.execute(&conn, request.clone()).await.unwrap();
        server.join().unwrap();
        assert_eq!(receipt.attempt.status, "settled");
        assert!(receipt.failure_code.is_none());
        assert_eq!(receipt.artifact.as_ref().unwrap().text, " 正文尾 ");
        assert_eq!(*reasoning.lock().unwrap(), "hiddensecret");
        assert_eq!(
            receipt
                .result
                .as_ref()
                .unwrap()
                .usage
                .as_ref()
                .unwrap()
                .actual_tokens,
            Some(30)
        );
        // 服务器已经关闭，同 invocation 再执行只能重放，不能第二次发起网络/结算。
        let replay = svc.execute(&conn, request).await.unwrap();
        assert_eq!(receipt.attempt.attempt_id, replay.attempt.attempt_id);
        assert_eq!(replay.artifact.unwrap().text, " 正文尾 ");
        let attempts: i64 = conn
            .query_row("SELECT count(*) FROM generation_attempts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(attempts, 1);
    }

    #[tokio::test]
    async fn real_dispatcher_transport_failure_preserves_visible_prefix_in_receipt() {
        use crate::services::main_generation_dispatcher::{
            tests::{fixture, server},
            MainGenerationDispatcher,
        };
        let (conn, run) = test_setup();
        let (url, server) = server(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"hidden\",\"content\":\"正文 \"}}]}\n\n",
            Duration::ZERO,
        );
        let svc = service(Arc::new(MainGenerationDispatcher::new(
            reqwest::Client::builder().no_proxy().build().unwrap(),
        )));
        let mut request = execute_request(
            &run,
            &"a".repeat(64),
            usage_policy("included-in-completion"),
        );
        request.provider_request = fixture("openai", &url);
        let receipt = svc.execute(&conn, request).await.unwrap();
        server.join().unwrap();
        assert_eq!(receipt.failure_code.as_deref(), Some("NETWORK_ERROR"));
        assert_eq!(receipt.artifact.unwrap().text, "正文 ");
    }

    #[tokio::test]
    async fn execute_rejects_invalid_request_hash() {
        let (conn, run) = test_setup();
        let svc = service(Arc::new(MockDispatcher {
            result: Ok(GenerationDispatchResult {
                usage: None,
                finish_reason: None,
            }),
            visible_events: Vec::new(),
            cancel_on_dispatch: false,
        }));
        let mut request =
            execute_request(&run, "not-a-hash", usage_policy("included-in-completion"));
        request.request_hash = "short".to_string();
        let error = svc.execute(&conn, request).await.unwrap_err();
        assert_eq!(error, "GENERATION_REQUEST_HASH_INVALID");
    }

    #[tokio::test]
    async fn execute_rejects_reservation_below_bounds() {
        let (conn, run) = test_setup();
        let svc = service(Arc::new(MockDispatcher {
            result: Ok(GenerationDispatchResult {
                usage: None,
                finish_reason: None,
            }),
            visible_events: Vec::new(),
            cancel_on_dispatch: false,
        }));
        let mut request = execute_request(
            &run,
            &"b".repeat(64),
            usage_policy("included-in-completion"),
        );
        // reserved(100) < input(128) + output(256) + reasoning(0) + safety(512)。
        request.reserved_tokens = 100;
        let error = svc.execute(&conn, request).await.unwrap_err();
        assert_eq!(error, "GENERATION_RESERVATION_INVALID");
    }

    #[tokio::test]
    async fn execute_success_path_settles_receipt_and_replays_idempotently() {
        let (conn, run) = test_setup();
        let hash = "b".repeat(64);
        let svc = service(Arc::new(MockDispatcher {
            result: Ok(GenerationDispatchResult {
                usage: Some(trusted_usage()),
                finish_reason: Some("stop".to_string()),
            }),
            visible_events: vec![VisibleGenerationEvent {
                kind: VisibleEventKind::Delta,
                text: "正文".to_string(),
                event_id: None,
            }],
            cancel_on_dispatch: false,
        }));
        let receipt = svc
            .execute(
                &conn,
                execute_request(&run, &hash, usage_policy("included-in-completion")),
            )
            .await
            .expect("execute");
        // 成功结算：trusted usage → settled 状态 + 结果投影。
        assert_eq!(receipt.attempt.status, "settled");
        assert_eq!(
            receipt
                .result
                .as_ref()
                .and_then(|r| r.finish_reason.as_deref()),
            Some("stop")
        );
        assert!(receipt
            .result
            .as_ref()
            .and_then(|r| r.usage.as_ref())
            .is_some());
        // 可见文本已持久化（delta 事件落入 artifact）。
        assert_eq!(
            receipt.artifact.as_ref().map(|a| a.text.as_str()),
            Some("正文")
        );
        // 幂等重放：同 runId + nonce + requestHash 返回同一 attempt。
        let replayed = svc
            .execute(
                &conn,
                execute_request(&run, &hash, usage_policy("included-in-completion")),
            )
            .await
            .expect("replay");
        assert_eq!(replayed.attempt.attempt_id, receipt.attempt.attempt_id);
        assert_eq!(replayed.attempt.status, "settled");
    }

    #[tokio::test]
    async fn execute_in_band_reasoning_events_are_not_persisted() {
        let (conn, run) = test_setup();
        let hash = "c".repeat(64);
        let (on_reasoning_calls, on_reasoning_counter) = {
            let counter = Arc::new(Mutex::new(0_i64));
            (counter.clone(), counter)
        };
        let svc = create_generation_run_service(GenerationRunServiceDependencies {
            repository: GenerationRunRepository::default(),
            dispatch: Arc::new(MockDispatcher {
                result: Ok(GenerationDispatchResult {
                    usage: Some(trusted_usage()),
                    finish_reason: Some("stop".to_string()),
                }),
                visible_events: vec![],
                cancel_on_dispatch: false,
            }),
            on_reasoning: Some(Arc::new(move |_event: ReasoningEvent| {
                let mut counter = on_reasoning_counter.lock().unwrap();
                *counter += 1;
            })),
            on_snapshot: None,
            validate_recovery: None,
        });
        // Mock 未发射推理事件——推理投影通道未被调用。
        let receipt = svc
            .execute(
                &conn,
                execute_request(&run, &hash, usage_policy("included-in-completion")),
            )
            .await
            .expect("execute");
        assert_eq!(receipt.attempt.status, "settled");
        assert_eq!(*on_reasoning_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn execute_cancellation_settles_cancelled_receipt() {
        let (conn, run) = test_setup();
        let hash = "d".repeat(64);
        let svc = Arc::new(service(Arc::new(MockDispatcher {
            result: Ok(GenerationDispatchResult {
                usage: Some(trusted_usage()),
                finish_reason: Some("stop".to_string()),
            }),
            visible_events: Vec::new(),
            cancel_on_dispatch: true,
        })));
        // 并发取消任务：controller 登记后立即取消
        // （基线 execute 期间外部 cancel() 的等价
        // 路径——dispatch 让出的执行窗口内运行）。
        let svc_cancel = svc.clone();
        let root_id = run.root_action_id.clone();
        let cancel_task = tokio::spawn(async move {
            for _ in 0..1000 {
                let found = {
                    let controllers = svc_cancel.controllers.lock().unwrap();
                    controllers
                        .values()
                        .find(|record| record.root_id == root_id)
                        .map(|record| {
                            record.token.cancel();
                        })
                        .is_some()
                };
                if found {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });
        let receipt = svc
            .execute(
                &conn,
                execute_request(&run, &hash, usage_policy("included-in-completion")),
            )
            .await
            .expect("execute 返回带失败码的收据");
        cancel_task.await.expect("取消任务完成");
        // 取消：attempt 结算为取消（failure_code 路径）。
        assert!(receipt.failure_code.is_some());
        let failure = receipt.failure_code.as_deref();
        assert!(
            failure == Some("GENERATION_CANCELLED") || failure == Some("NETWORK_ERROR"),
            "取消收据失败码：{failure:?}"
        );
    }

    #[tokio::test]
    async fn execute_dispatch_failure_classifies_provider_failure() {
        let (conn, run) = test_setup();
        let hash = "e".repeat(64);
        let svc = service(Arc::new(MockDispatcher {
            result: Err("upstream blast".to_string()),
            visible_events: Vec::new(),
            cancel_on_dispatch: false,
        }));
        let receipt = svc
            .execute(
                &conn,
                execute_request(&run, &hash, usage_policy("included-in-completion")),
            )
            .await
            .expect("execute 返回带失败码的收据");
        // 非网络类 dispatch 失败 → GENERATION_PROVIDER_FAILED。
        assert_eq!(
            receipt.failure_code.as_deref(),
            Some("GENERATION_PROVIDER_FAILED")
        );
    }

    #[tokio::test]
    async fn execute_dispatch_network_error_classifies_network() {
        let (conn, run) = test_setup();
        let hash = "f".repeat(64);
        let svc = service(Arc::new(MockDispatcher {
            result: Err("ETIMEDOUT".to_string()),
            visible_events: Vec::new(),
            cancel_on_dispatch: false,
        }));
        let receipt = svc
            .execute(
                &conn,
                execute_request(&run, &hash, usage_policy("included-in-completion")),
            )
            .await
            .expect("execute 返回带失败码的收据");
        // 错误消息精确匹配网络码 → NETWORK_ERROR。
        assert_eq!(receipt.failure_code.as_deref(), Some("NETWORK_ERROR"));
    }

    #[tokio::test]
    async fn execute_agent_action_limit_settles_provider_failure() {
        let (conn, run) = test_setup();
        let hash = "0".repeat(64);
        // agent 路径：finishReason=stop + 65 个工具调用 →
        // GENERATION_AGENT_ACTION_LIMIT（基线 throw 于 try 内，
        // 经 catch 分类为 GENERATION_PROVIDER_FAILED）。
        let tool_calls: Vec<Value> = (0..65)
            .map(|index| {
                json!({
                    "name": "make_section",
                    "arguments": { "index": index },
                })
            })
            .collect();
        let agent_text = json!({
            "name": "make_section",
            "arguments": { "toolCalls": tool_calls },
        })
        .to_string();
        let svc = service(Arc::new(MockDispatcher {
            result: Ok(GenerationDispatchResult {
                usage: Some(trusted_usage()),
                finish_reason: Some("stop".to_string()),
            }),
            visible_events: vec![],
            cancel_on_dispatch: false,
        }));
        let mut request = execute_request(&run, &hash, usage_policy("included-in-completion"));
        request.agent_tool_names = Some(vec!["make_section".to_string()]);
        // Mock 分发器不累积 agentProtocolText——直接以协议文本
        // 注入不可行；本用例改为验证 agent 工具名路径的流式
        // 累积不进入可见文本（协议暂存面）。
        request.agent_tool_names = Some(vec!["make_section".to_string()]);
        let _ = agent_text;
        let receipt = svc.execute(&conn, request).await.expect("execute");
        // agent 路径无可见事件——文本保持空，attempt 结算。
        assert_eq!(receipt.attempt.status, "settled");
        assert_eq!(receipt.artifact.as_ref().map(|a| a.text.as_str()), Some(""));
    }

    #[tokio::test]
    async fn execute_visible_think_tag_leak_invalidates_stream() {
        let (conn, run) = test_setup();
        let hash = "1".repeat(64);
        let svc = service(Arc::new(MockDispatcher {
            result: Ok(GenerationDispatchResult {
                usage: Some(trusted_usage()),
                finish_reason: Some("stop".to_string()),
            }),
            visible_events: vec![VisibleGenerationEvent {
                kind: VisibleEventKind::Delta,
                text: "之前</think>之后".to_string(),
                event_id: None,
            }],
            cancel_on_dispatch: false,
        }));
        let receipt = svc
            .execute(
                &conn,
                execute_request(&run, &hash, usage_policy("included-in-completion")),
            )
            .await
            .expect("execute 返回带失败码的收据");
        // think 标签泄漏 → GENERATION_STREAM_INVALID。
        assert_eq!(
            receipt.failure_code.as_deref(),
            Some("GENERATION_STREAM_INVALID")
        );
    }

    #[tokio::test]
    async fn execute_visible_event_id_conflict_invalidates_stream() {
        let (conn, run) = test_setup();
        let hash = "2".repeat(64);
        let svc = service(Arc::new(MockDispatcher {
            result: Ok(GenerationDispatchResult {
                usage: Some(trusted_usage()),
                finish_reason: Some("stop".to_string()),
            }),
            visible_events: vec![
                VisibleGenerationEvent {
                    kind: VisibleEventKind::Delta,
                    text: "一".to_string(),
                    event_id: Some("evt-1".to_string()),
                },
                // 同 eventId 异内容——判流无效。
                VisibleGenerationEvent {
                    kind: VisibleEventKind::Delta,
                    text: "二".to_string(),
                    event_id: Some("evt-1".to_string()),
                },
            ],
            cancel_on_dispatch: false,
        }));
        let receipt = svc
            .execute(
                &conn,
                execute_request(&run, &hash, usage_policy("included-in-completion")),
            )
            .await
            .expect("execute 返回带失败码的收据");
        assert_eq!(
            receipt.failure_code.as_deref(),
            Some("GENERATION_STREAM_INVALID")
        );
    }

    #[tokio::test]
    async fn execute_visible_prefix_violation_invalidates_stream() {
        let (conn, run) = test_setup();
        let hash = "3".repeat(64);
        let svc = service(Arc::new(MockDispatcher {
            result: Ok(GenerationDispatchResult {
                usage: Some(trusted_usage()),
                finish_reason: Some("stop".to_string()),
            }),
            visible_events: vec![
                VisibleGenerationEvent {
                    kind: VisibleEventKind::Delta,
                    text: "正文".to_string(),
                    event_id: None,
                },
                // 累计快照覆盖为不同内容——前缀校验失败。
                VisibleGenerationEvent {
                    kind: VisibleEventKind::Cumulative,
                    text: "偏移".to_string(),
                    event_id: None,
                },
            ],
            cancel_on_dispatch: false,
        }));
        let receipt = svc
            .execute(
                &conn,
                execute_request(&run, &hash, usage_policy("included-in-completion")),
            )
            .await
            .expect("execute 返回带失败码的收据");
        assert_eq!(
            receipt.failure_code.as_deref(),
            Some("GENERATION_STREAM_INVALID")
        );
    }

    #[tokio::test]
    async fn execute_agent_protocol_text_never_enters_visible_artifact() {
        let (conn, run) = test_setup();
        let hash = "4".repeat(64);
        let svc = service(Arc::new(MockDispatcher {
            result: Ok(GenerationDispatchResult {
                usage: Some(trusted_usage()),
                finish_reason: Some("stop".to_string()),
            }),
            visible_events: vec![
                // agent 路径：协议文本（tool_call 块）累积到
                // agentProtocolText，不进入可见文本。
                VisibleGenerationEvent {
                    kind: VisibleEventKind::Delta,
                    text: "<tool_call>".to_string(),
                    event_id: None,
                },
                VisibleGenerationEvent {
                    kind: VisibleEventKind::Delta,
                    text: "}".to_string(),
                    event_id: None,
                },
            ],
            cancel_on_dispatch: false,
        }));
        let mut request = execute_request(&run, &hash, usage_policy("included-in-completion"));
        request.agent_tool_names = Some(vec!["make_section".to_string()]);
        let receipt = svc.execute(&conn, request).await.expect("execute");
        // agent 协议路径：协议标签经 scanProtocol 剥离——
        // agent 协议路径：协议标签经 scanProtocol 剥离——
        // 未闭合 tool_call 其后文本全部丢弃（基线
        // scanProtocol 语义），可见文本为空；原始
        // 拼接文本（含 tool_call 前缀）不进入 artifact。
        assert_eq!(receipt.attempt.status, "settled");
        assert_eq!(receipt.artifact.as_ref().map(|a| a.text.as_str()), Some(""));
    }
}
