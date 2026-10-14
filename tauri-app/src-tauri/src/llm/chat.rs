//! 批次 D2-b：LLM 生成链共享类型、`<think>` 剥离与 HTTP 客户端装配。
//!
//! 迁移自 `electron/llm/openai-provider.ts` / `gemini-provider.ts` 的**共用部分**
//! 以及 `electron/controllers/llm-controller.ts` 的 `applyProxyConfig()`：
//!
//! | 本模块项 | 基线来源 |
//! |---|---|
//! | [`LlmFinishReason`] / [`TokenUsage`] / [`LlmResponse`] | `src/shared/ipc-channels.ts` |
//! | [`LlmGenerateOptions`] / [`LlmStreamOptions`] | `electron/llm/provider.interface.ts` |
//! | [`strip_thinking`] | `OpenAIProvider.stripThinking()`（三条替换规则） |
//! | [`build_client`] / [`proxy_from_config`] | `llm-controller.ts::applyProxyConfig()` |
//!
//! ## 与基线的刻意差异（均为环境差异，不是语义放宽）
//!
//! 1. **代理装配方式**：基线写 `process.env.HTTP_PROXY/HTTPS_PROXY` 让 `fetch` 读取；
//!    Rust 侧改为在 `reqwest::Client` 上显式装配 [`reqwest::Proxy`]。语义等价，且不会
//!    让「多窗口各自的项目设置」通过进程环境互相污染。
//! 2. **取消机制**：基线的 `AbortSignal` 立即中断在途请求；Rust 侧用原子探针
//!    （见 [`LlmStreamOptions::is_cancelled`]），在下一个 chunk 到达时终止。LLM 流式
//!    场景下 chunk 间隔极短，取消延迟可忽略；终止后 `Response` 被 drop 即关闭连接。
//! 3. **错误文案前缀**：基线的 `String(error)` 对 `Error` 实例会带上 `"Error: "`
//!    前缀；Rust 侧经 [`provider_error_text`] 复刻同一格式，但异常对象名（如
//!    `TypeError`）无法逐字复刻 —— 只保留 `"Error: "` 一档。
//! 4. **传输诊断发射**（B3-2a 偏差登记，待裁决项 B3-M4）：基线
//!    `openai-provider.ts` 经 `notify()`（250ms 节流）发射
//!    `onDiagnostics`（request→response→stream→complete 相位迁移、
//!    HTTP_ERROR/RESPONSE_BODY_MISSING/PROVIDER_STREAM_ERROR/
//!    STREAM_INVALID/STREAM_INCOMPLETE 错误码、visibleEvents/
//!    reasoningEvents 计数、`safeTransportError` 的 Node undici 码提取）；
//!    `gemini-provider.ts` 从不发射。Rust 侧传输诊断发射机器尚未移植
//!    ——`GenerationTransportDiagnostics` 类型已就位于
//!    `repositories/generation_run_repository.rs`（运行服务侧），但
//!    provider 侧发射点（含 reqwest `ErrorKind` → 基线 undici 码的
//!    映射裁决）属专波事项；基线测试 `openai-provider.test.ts` 的
//!    `causeCode === 'UND_ERR_*'` 断言在 Rust 侧不可移植（排除并注释）。
//!    本波及当前 IPC 路径均不发射传输诊断。

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::llm::reasoning::ProviderReasoningDirective;

/// 生成终态（provider-neutral）。
///
/// `stop` 是唯一被下游工作流与 Agent 视为「完整回应」的终态；缺失供应商值时
/// 归一为 `unknown`，它保留文本可检视性，但**不是**可提交的证据。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmFinishReason {
    Stop,
    Length,
    ContentFilter,
    Cancelled,
    Error,
    Unknown,
}

impl LlmFinishReason {
    /// 序列化形式（与 `ipc-channels.ts` 的 `LLMFinishReason` 字面量一致）。
    pub fn as_str(self) -> &'static str {
        match self {
            LlmFinishReason::Stop => "stop",
            LlmFinishReason::Length => "length",
            LlmFinishReason::ContentFilter => "content_filter",
            LlmFinishReason::Cancelled => "cancelled",
            LlmFinishReason::Error => "error",
            LlmFinishReason::Unknown => "unknown",
        }
    }
}

/// token 用量（对齐基线 `TokenUsage`：三项均可为 `null`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
}

/// token 用量的分账口径（基线 `ProviderUsageEvidence.accounting` 字面量联合）。
///
/// 基线语义：**证据不是权限**——`included-in-completion` 表示推理
/// token 已计入 `completion_tokens`（OpenAI 口径），`separately-billed`
/// 表示单独计费（Gemini 口径）；可信记账结算由消费侧（生成运行服务的
/// `settleProviderUsage`）据此做出，本类型只承载协议事实。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsageAccounting {
    IncludedInCompletion,
    SeparatelyBilled,
    Unknown,
}

impl UsageAccounting {
    /// 序列化形式（与基线字面量一致）。
    ///
    /// B3-2b 事件载荷序列化将消费本方法；当前仅契约测试使用。
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            UsageAccounting::IncludedInCompletion => "included-in-completion",
            UsageAccounting::SeparatelyBilled => "separately-billed",
            UsageAccounting::Unknown => "unknown",
        }
    }
}

/// 证据来源协议（基线 `ProviderUsageEvidence.protocol` 字面量联合）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageProtocol {
    OpenAi,
    Gemini,
}

impl UsageProtocol {
    /// 序列化形式（与基线字面量一致）。
    ///
    /// B3-2b 事件载荷序列化将消费本方法；当前仅契约测试使用。
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            UsageProtocol::OpenAi => "openai",
            UsageProtocol::Gemini => "gemini",
        }
    }
}

/// provider 侧 usage 证据（对齐基线 `ProviderUsageEvidence`，
/// `electron/llm/provider.interface.ts`）。
///
/// **逐字段核对基线**：基线只有以下五个字段，**没有** `trusted` 字段
/// （「可信」是消费侧依据 [`UsageAccounting`] 做出的判定，不是协议
/// 事实）。`reasoning_tokens` 为协议报告的推理 token 分项
/// （OpenAI：`usage.completion_tokens_details.reasoning_tokens`；
/// Gemini：`usageMetadata.thoughtsTokenCount`），未报告时为 `None`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsageEvidence {
    pub usage: TokenUsage,
    pub reasoning_tokens: Option<i64>,
    pub accounting: UsageAccounting,
    pub total_includes_reasoning: bool,
    pub protocol: UsageProtocol,
}

/// 非流式生成响应（对齐 `LLMResponse` 的 `{ success, content, finishReason, usage?, error? }`）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmResponse {
    pub success: bool,
    pub content: String,
    pub finish_reason: LlmFinishReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl LlmResponse {
    /// 供失败路径构造信封（`content` 恒为空 —— 对齐基线 provider 的 catch 分支）。
    pub fn failure(error: String) -> Self {
        LlmResponse {
            success: false,
            content: String::new(),
            finish_reason: LlmFinishReason::Error,
            usage: None,
            error: Some(error),
        }
    }
}

/// 一次生成请求的有效参数。
///
/// `temperature` 已由 [`crate::llm::generation_parameter_policy::resolve_generation_parameters`] 决议：
#[derive(Debug, Clone)]
pub struct LlmGenerateOptions {
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    /// OpenAI 官方端点使用独立的输出上限参数名；缺失时沿用兼容默认值。
    pub output_token_parameter: Option<&'static str>,
    pub response_format: Option<Value>,
    pub reasoning: Option<ProviderReasoningDirective>,
    /// 调用方作用域的逻辑会话身份，逐字透传给要求会话粘性的网关（opencode Go）。
    /// 缺失表示单请求作用域，**绝不**退化为进程级共享默认值。
    pub conversation_id: Option<String>,
}

/// 一条对话消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// 流式回调集合（对齐基线 `LLMStreamOptions` 的回调）。
///
/// `on_reasoning` / `on_usage_evidence` 为 B3-2a 新增（基线
/// `onReasoning` / `onUsageEvidence` 的 1:1 对应），**Option 化**：
/// 现有 IPC 路径（`llm:generate` / `llm:generate-stream`）传 `None`，
/// 可观测行为逐字不变——新增回调只在 B3-2b 生成运行服务消费。
///
/// 基线另有 `onDiagnostics`：openai provider 发射而 gemini 不发射；
/// Rust 侧传输诊断发射机器待 B3-M4 专波移植，本波不建该回调
/// （见模块头注偏差登记）。
pub struct StreamCallbacks<'a> {
    pub on_chunk: &'a (dyn Fn(&str) + Send + Sync),
    /// `finish_reason` 恒为归一后的终态证据。
    pub on_done: &'a (dyn Fn(String, Option<TokenUsage>, LlmFinishReason) + Send + Sync),
    /// `content` 是**已交付的可见候选文本**，绝不是隐藏推理内容。
    pub on_error: &'a (dyn Fn(String, Option<String>, Option<TokenUsage>) + Send + Sync),
    /// 隐藏推理文本（基线 `onReasoning`）：协议 reasoning 字段原文，
    /// 或可见文本中带内 `</think>…</think>` 块的抽取结果。
    /// **绝不**进入可见候选（基线注释：ephemeral provider text only）。
    pub on_reasoning: Option<&'a (dyn Fn(&str) + Send + Sync)>,
    /// provider 侧 usage 证据（基线 `onUsageEvidence`）：协议 metadata
    /// 是证据而非释放预留的权限（基线注释原意）。
    pub on_usage_evidence: Option<&'a (dyn Fn(&ProviderUsageEvidence) + Send + Sync)>,
}

/// 解码器单次 `push` 产生的交付物，顺序即基线回调触发顺序
/// （单事件内：usage 证据 → 推理文本 → 可见 chunk）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseDelivery {
    /// 可见文本 chunk（基线 `onChunk`）。
    Chunk(String),
    /// 隐藏推理文本（基线 `onReasoning`）。
    Reasoning(String),
    /// provider 侧 usage 证据（基线 `onUsageEvidence`）。
    UsageEvidence(ProviderUsageEvidence),
}

/// 把一枚解码交付物分发给回调；新增回调缺失时跳过
/// （对齐基线 `opts.onReasoning?.()` 的可选调用语义）。
pub fn deliver(delivery: SseDelivery, callbacks: &StreamCallbacks<'_>) {
    match delivery {
        SseDelivery::Chunk(chunk) => (callbacks.on_chunk)(&chunk),
        SseDelivery::Reasoning(reasoning) => {
            if let Some(on_reasoning) = callbacks.on_reasoning {
                on_reasoning(&reasoning);
            }
        }
        SseDelivery::UsageEvidence(evidence) => {
            if let Some(on_usage_evidence) = callbacks.on_usage_evidence {
                on_usage_evidence(&evidence);
            }
        }
    }
}

/// 基线思考标签对（`InBandReasoningStream` / `VisibleStreamFilter` /
/// `stripThinking` 共用同一对标签）。
const THINK_OPEN_TAG: &str = "\x3cthink\x3e";
const THINK_CLOSE_TAG: &str = "</think>";

/// 复刻基线 `InBandReasoningStream`（`electron/llm/provider.interface.ts`）：
/// 从**可见文本**中抽取 `</think>…</think>` 块内的推理文本。
///
/// 与 [`strip_thinking`] 的差异：本类型**保留**块内原文（含嵌套与块内
/// 标记），只剥离标签本身；以深度计数处理嵌套；跨 `push` 的半枚标签
/// 暂存于 `pending`，待后续文本补足后再分类——拆分到两个 chunk 的
/// 开标签绝不会泄漏推理前缀（基线 `owner-visible-stream.test.ts` 的
/// 拆分思维用例即验证此边界）。
///
/// 基线在 `onReasoning` 缺失时直接短路返回；Rust 侧由调用方决定
/// 返回值是否投递，故本类型不持有回调。
#[derive(Debug, Default)]
pub struct InBandReasoning {
    pending: String,
    depth: u32,
}

impl InBandReasoning {
    /// 喂入一段可见文本；返回本次抽取到的推理文本（`None` 表示无发射）。
    pub fn push(&mut self, text: &str) -> Option<String> {
        // 基线以 UTF-16 码元索引；标签与 `<` 均为 ASCII，逐 `char` 索引
        // 在「标签分类」与「切片边界」上逐字等价（多字节字符只出现在
        // 被收集的正文中，其边界恒由 ASCII 分隔符确定）。
        let input: Vec<char> = self.pending.chars().chain(text.chars()).collect();
        let mut parts: Vec<String> = Vec::new();
        self.pending = String::new();
        let mut cursor = 0usize;
        while cursor < input.len() {
            let opening = input[cursor..]
                .iter()
                .position(|&character| character == '<')
                .map(|offset| cursor + offset);
            let end = opening.unwrap_or(input.len());
            if self.depth > 0 && end > cursor {
                parts.push(input[cursor..end].iter().collect());
            }
            let Some(opening) = opening else {
                break;
            };
            cursor = opening;
            let tail: String = input[cursor..(cursor + 8).min(input.len())]
                .iter()
                .collect::<String>()
                .to_lowercase();
            let tag = if tail.starts_with(THINK_OPEN_TAG) {
                Some(THINK_OPEN_TAG)
            } else if tail.starts_with(THINK_CLOSE_TAG) {
                Some(THINK_CLOSE_TAG)
            } else {
                None
            };
            if let Some(tag) = tag {
                self.depth = if tag == THINK_OPEN_TAG {
                    self.depth + 1
                } else {
                    self.depth.saturating_sub(1)
                };
                cursor += tag.chars().count();
                continue;
            }
            // 半枚标签（输入在标签中途截断）：暂存待补，不做任何收集。
            if THINK_OPEN_TAG.starts_with(&tail) || THINK_CLOSE_TAG.starts_with(&tail) {
                self.pending = input[cursor..].iter().collect();
                break;
            }
            if self.depth > 0 {
                parts.push("<".to_string());
            }
            cursor += 1;
        }
        let reasoning = parts.concat();
        if reasoning.is_empty() {
            None
        } else {
            Some(reasoning)
        }
    }
}

/// 流式生成选项（生成参数 + 回调 + 取消探针）。
pub struct LlmStreamOptions<'a> {
    /// 持久化生成只交付正文；旧 IPC 必须显式保持 false。
    pub visible_only: bool,

    pub generate: LlmGenerateOptions,
    pub callbacks: StreamCallbacks<'a>,
    /// 取消探针：返回 `true` 时流式循环在下个 chunk 处终止（对齐基线 `AbortSignal`）。
    pub is_cancelled: &'a (dyn Fn() -> bool + Send + Sync),
}

/// 流式失败原因：取消与一般错误必须分开（对齐基线对 `AbortError` 的特判）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamFailure {
    Cancelled,
    Message(String),
}

impl StreamFailure {
    /// 归一为对用户可见的文案（取消文案对应用户主动中止）。
    pub fn into_message(self) -> String {
        match self {
            StreamFailure::Cancelled => "已取消生成".to_string(),
            StreamFailure::Message(message) => message,
        }
    }
}

/// 复刻基线 `String(error)` 对 `Error` 实例的 `"Error: "` 前缀。
///
/// 复用 [`crate::commands::db::mutating_error`] 以免出现第二份格式定义。
pub fn provider_error_text(error: String) -> String {
    crate::commands::mutating_error(error)
}

/// 规则 1：剥离 `<think>…</think>` 块；未闭合时延伸到输入末尾（`\z` 等价 JS 无 `m` 标志的 `$`）。
static THINK_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<think>[\s\S]*?(?:</think>|\z)").unwrap());
/// 规则 2：剥离**首个** `</think>` 之前的全部内容及其后空白（非全局替换，故用 `replace`）。
static THINK_HEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[\s\S]*?</think>\s*").unwrap());
/// 规则 3：剥离**首个**残留的 `<think>` / `</think>` 标签（非全局替换）。
static THINK_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</?think>").unwrap());

/// 对齐基线 `OpenAIProvider.stripThinking()`。
///
/// 三条规则的**顺序**与**全局/首个匹配**语义都必须一致：
/// 规则 1 是全局替换，规则 2/3 只替换首个匹配；最后 `trim()`。
pub fn strip_thinking(content: &str) -> String {
    let step1 = THINK_BLOCK.replace_all(content, "");
    let step2 = THINK_HEAD.replace(&step1, "");
    let step3 = THINK_TAG.replace(&step2, "");
    step3.trim().to_string()
}

/// 代理规格（对齐基线 `GlobalConfig.proxy` 的判定结果）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxySpec {
    /// `http` 或 `socks5`（基线仅支持这两档）。
    pub scheme: String,
    pub host: String,
    pub port: u64,
}

impl ProxySpec {
    /// 代理 URL（`reqwest::Proxy::all` 的入参形式）。
    pub fn url(&self) -> String {
        format!("{}://{}:{}", self.scheme, self.host, self.port)
    }
}

/// 从全局配置的 `proxy` 段解析代理（对齐基线 `applyProxyConfig()` 的判定顺序）。
///
/// 基线要求 `enabled === true` **且** `host` 非空才启用；`type === 'socks5'` 走
/// socks5，其余一律 http。配置读取本身失败时基线静默忽略 —— 此处同样返回 `None`。
pub fn proxy_from_config(config: &Value) -> Option<ProxySpec> {
    let proxy = config.get("proxy")?;
    if !proxy
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let host = proxy.get("host").and_then(Value::as_str)?.trim();
    if host.is_empty() {
        return None;
    }
    let port = proxy.get("port").and_then(Value::as_u64)?;
    let scheme = match proxy.get("type").and_then(Value::as_str) {
        Some("socks5") => "socks5",
        _ => "http",
    };
    Some(ProxySpec {
        scheme: scheme.to_string(),
        host: host.to_string(),
        port,
    })
}

/// 构建一次生成调用所用的 HTTP 客户端。
///
/// 刻意不设总超时：基线 `fetch` 无超时，长文生成可能持续数分钟。
pub fn build_client(proxy: Option<&ProxySpec>) -> Result<reqwest::Client, String> {
    build_client_with_timeout(proxy, None)
}

/// 同上，但可选总超时（含 body 读取）—— 供模型发现等有界调用使用。
///
/// `None` 表示不设超时（等价于基线 `fetch` 的默认行为）；`Some` 对应基线
/// `AbortController` + `setTimeout(abort, ms)` 的总时长语义。
pub fn build_client_with_timeout(
    proxy: Option<&ProxySpec>,
    timeout: Option<std::time::Duration>,
) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder();
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    if let Some(spec) = proxy {
        builder = builder.proxy(
            reqwest::Proxy::all(spec.url()).map_err(|error| format!("代理配置无效：{error}"))?,
        );
    }
    builder
        .build()
        .map_err(|error| format!("HTTP 客户端初始化失败：{error}"))
}

/// 协议判定（对齐 `LLMFactory.getProvider()`）：`protocol === 'gemini'` 走 Gemini，
/// 其余一律按 OpenAI 兼容协议处理。
pub fn is_gemini(model: &Value) -> bool {
    model.get("protocol").and_then(Value::as_str) == Some("gemini")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strip_thinking_matches_baseline_rules_test() {
        // 规则 1：完整 think 块被剥离（全局）
        assert_eq!(strip_thinking("<think>推理中</think>正文"), "正文");
        // 规则 1：未闭合时延伸到输入末尾
        assert_eq!(strip_thinking("前文<think>推理中"), "前文");
        // 规则 2：无起始标签时剥离首个 `</think>` **之前的全部内容**
        assert_eq!(strip_thinking("推理中</think>\n\n正文"), "正文");
        assert_eq!(strip_thinking("正文</think>后半"), "后半");
        // 大小写不敏感
        assert_eq!(strip_thinking("<THINK>a</Think>正文"), "正文");
        // 多行（`[\s\S]` 语义）
        assert_eq!(strip_thinking("<think>a\nb</think>\n正文"), "正文");
    }

    #[test]
    fn strip_thinking_rules_2_and_3_replace_only_first_test() {
        // 规则 2/3 非全局（对齐基线 `.replace(re, '')` 无 `g` 标志）：
        // 规则 2 吃掉首个 `</think>` 及其前缀，规则 3 只消掉下一个残留标签，
        // 因此第三个 `</think>` 会被保留。
        assert_eq!(strip_thinking("a</think>b</think>c"), "bc");
        assert_eq!(
            strip_thinking("a</think>b</think>c</think>d"),
            "bc</think>d"
        );
        // 规则 1 是全局替换：每个 `<think>` 都会吞到 `</think>` 或输入末尾
        assert_eq!(strip_thinking("x<think>y<think>z"), "x");
    }

    #[test]
    fn proxy_from_config_follows_baseline_guards_test() {
        assert_eq!(proxy_from_config(&json!({})), None);
        assert_eq!(
            proxy_from_config(&json!({"proxy": {"enabled": false, "host": "h", "port": 1}})),
            None
        );
        assert_eq!(
            proxy_from_config(&json!({"proxy": {"enabled": true, "host": "  ", "port": 1}})),
            None
        );
        assert_eq!(
            proxy_from_config(&json!({"proxy": {"enabled": true, "host": "h"}})),
            None
        );
        assert_eq!(
            proxy_from_config(
                &json!({"proxy": {"enabled": true, "host": "127.0.0.1", "port": 7890}})
            ),
            Some(ProxySpec {
                scheme: "http".to_string(),
                host: "127.0.0.1".to_string(),
                port: 7890
            })
        );
        assert_eq!(
            proxy_from_config(
                &json!({"proxy": {"enabled": true, "host": "127.0.0.1", "port": 7891, "type": "socks5"}})
            )
            .map(|spec| spec.url()),
            Some("socks5://127.0.0.1:7891".to_string())
        );
    }

    #[test]
    fn provider_error_text_prefixes_js_error_shape_test() {
        assert_eq!(provider_error_text("失败".to_string()), "Error: 失败");
    }

    #[test]
    fn finish_reason_serializes_to_contract_literals_test() {
        for (reason, literal) in [
            (LlmFinishReason::Stop, "stop"),
            (LlmFinishReason::Length, "length"),
            (LlmFinishReason::ContentFilter, "content_filter"),
            (LlmFinishReason::Cancelled, "cancelled"),
            (LlmFinishReason::Error, "error"),
            (LlmFinishReason::Unknown, "unknown"),
        ] {
            assert_eq!(reason.as_str(), literal);
            assert_eq!(serde_json::to_value(reason).unwrap(), json!(literal));
        }
    }

    #[test]
    fn protocol_detection_matches_provider_factory_test() {
        // 与 `LLMFactory.getProvider()` 同一依据：只有 `protocol === 'gemini'` 走 Gemini。
        assert!(is_gemini(&json!({"protocol": "gemini"})));
        assert!(!is_gemini(&json!({"protocol": "openai"})));
        assert!(!is_gemini(&json!({"protocol": "openai-compatible"})));
        assert!(!is_gemini(&json!({})), "缺 protocol 按 OpenAI 兼容处理");
        assert!(!is_gemini(&json!({"protocol": null})));
    }

    #[test]
    fn usage_evidence_literals_match_baseline_contract_test() {
        // 与基线 `ProviderUsageEvidence` 的字面量联合逐字一致
        // （B3-2b 结算与事件载荷消费这些字面量）。
        for (accounting, literal) in [
            (
                UsageAccounting::IncludedInCompletion,
                "included-in-completion",
            ),
            (UsageAccounting::SeparatelyBilled, "separately-billed"),
            (UsageAccounting::Unknown, "unknown"),
        ] {
            assert_eq!(accounting.as_str(), literal);
            assert_eq!(serde_json::to_value(accounting).unwrap(), json!(literal));
        }
        for (protocol, literal) in [
            (UsageProtocol::OpenAi, "openai"),
            (UsageProtocol::Gemini, "gemini"),
        ] {
            assert_eq!(protocol.as_str(), literal);
            assert_eq!(serde_json::to_value(protocol).unwrap(), json!(literal));
        }
        // `ProviderUsageEvidence` 的 camelCase 载荷形态（基线接口字段名）。
        let evidence = ProviderUsageEvidence {
            usage: TokenUsage {
                prompt_tokens: Some(1),
                completion_tokens: Some(2),
                total_tokens: Some(3),
            },
            reasoning_tokens: Some(7),
            accounting: UsageAccounting::IncludedInCompletion,
            total_includes_reasoning: true,
            protocol: UsageProtocol::OpenAi,
        };
        assert_eq!(
            serde_json::to_value(&evidence).unwrap(),
            json!({
                "usage": {"promptTokens": 1, "completionTokens": 2, "totalTokens": 3},
                "reasoningTokens": 7,
                "accounting": "included-in-completion",
                "totalIncludesReasoning": true,
                "protocol": "openai",
            })
        );
    }
}
