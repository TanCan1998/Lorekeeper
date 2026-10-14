//! 批次 D2-b：OpenAI 兼容协议 provider。
//!
//! 迁移自 `electron/llm/openai-provider.ts`（生成 / 流式 / 终态归一 / `<think>` 剥离）
//! 与 `electron/llm/openai-compatible-endpoint.ts`（端点规范化）。
//!
//! ## 流式协议解码器
//!
//! 基线**不使用** SSE 库，而是在 provider 内手写「字节流 → 行 → `data:` 字段 → 事件」
//! 的分层解析，并对每个结构位置做了**严格**类型校验（损坏的 JSON、非对象载荷、
//! `choices` / `delta` / `content` 类型错误都直接判为致命错误，而不是跳过）。
//! [`OpenAiSseDecoder`] 逐条复刻该状态机，并抽成纯函数式单元以便离线测试 ——
//! 网络层只负责把 chunk 喂进来、把回调发出去。

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Map, Value};

use crate::llm::chat::{
    deliver, strip_thinking, ChatMessage, InBandReasoning, LlmFinishReason, LlmGenerateOptions,
    LlmResponse, LlmStreamOptions, ProviderUsageEvidence, SseDelivery, StreamFailure, TokenUsage,
    UsageAccounting, UsageProtocol,
};
use crate::llm::reasoning::ProviderReasoningDirective;

/// opencode Go 的 `User-Agent` 前缀（基线为 `ai-novel-writer/<package.json version>`）。
const OPENCODE_GO_USER_AGENT_PREFIX: &str = "ai-novel-writer/";

/// 与基线 `package.json` 版本一致的 UA（crate 版本与其同步维护）。
pub fn user_agent() -> String {
    format!(
        "{OPENCODE_GO_USER_AGENT_PREFIX}{}",
        env!("CARGO_PKG_VERSION")
    )
}

/// 是否为 opencode Go 端点（需要会话粘性头）。
///
/// 对齐基线 `isOpencodeGoBaseUrl()`：必须 `https` + 主机 `opencode.ai` +
/// 去除尾部 `/` 后的路径恰为 `/zen/go` 或以 `/zen/go/` 开头。
pub fn is_opencode_go_base_url(base_url: &str) -> bool {
    let Ok(endpoint) = url::Url::parse(base_url.trim()) else {
        return false;
    };
    let configured_path = endpoint.path().trim_end_matches('/');
    endpoint.scheme() == "https"
        && endpoint.host_str() == Some("opencode.ai")
        && (configured_path == "/zen/go" || configured_path.starts_with("/zen/go/"))
}

/// 对齐基线 `resolveOpenAIChatCompletionsUrl()`。
///
/// `baseUrl` 已是完整端点时原样保留；以 `/chat` 结尾时补 `/completions`；
/// 路径为空或是 NovelAI 时补 `/v1/chat/completions`；其余补 `/chat/completions`。
pub fn resolve_chat_completions_url(base_url: &str, provider: &str) -> Result<String, String> {
    let mut endpoint = url::Url::parse(base_url.trim())
        .map_err(|error| format!("无效的 baseUrl（{base_url}）：{error}"))?;
    let configured_path = endpoint.path().trim_end_matches('/').to_string();
    let next_path = if configured_path.ends_with("/chat/completions") {
        configured_path
    } else if configured_path.ends_with("/chat") {
        format!("{configured_path}/completions")
    } else if configured_path.is_empty() || provider == "novelai" {
        format!("{configured_path}/v1/chat/completions")
    } else {
        format!("{configured_path}/chat/completions")
    };
    endpoint.set_path(&next_path);
    Ok(endpoint.to_string())
}

/// 对齐基线 `normalizeFinishReason()`。缺失/未知供应商值一律归一为 `unknown`。
pub fn normalize_finish_reason(reason: Option<&str>) -> LlmFinishReason {
    match reason {
        Some("stop") => LlmFinishReason::Stop,
        Some("length") => LlmFinishReason::Length,
        Some("model_context_window_exceeded") => LlmFinishReason::Length,
        Some("content_filter") => LlmFinishReason::ContentFilter,
        Some("sensitive") => LlmFinishReason::ContentFilter,
        Some("network_error") => LlmFinishReason::Error,
        _ => LlmFinishReason::Unknown,
    }
}

/// 基线 `resolveModelProfileReasoningMapping(model)?.adapter
/// === adapter` 门禁（`openai-provider.ts` 各推理载荷分支
/// 共用）：档案 JSON 经 [`ModelCapabilityProfile::from_value`]
/// 构造后解析**已验证**推理映射（手动映射优先 + 内置目录），
/// 比对 adapter 家族。
fn reasoning_mapping_adapter_is(
    model: &Value,
    adapter: crate::llm::reasoning::ReasoningAdapter,
) -> bool {
    crate::llm::presets::resolve_model_profile_reasoning_mapping(
        &crate::llm::presets::ModelCapabilityProfile::from_value(model),
    )
    .map_or(false, |mapping| mapping.adapter == adapter)
}
/// 对齐基线 `buildRequestBody()`。
///
/// 注意 `JSON.stringify` 会**丢弃**值为 `undefined` 的键，因此 `max_tokens` 在两侧
/// 都缺失时**不得**出现 `null` 占位。
pub fn build_request_body(
    model: &Value,
    messages: &[ChatMessage],
    opts: &LlmGenerateOptions,
    stream: bool,
) -> Value {
    let is_novel_ai = model.get("provider").and_then(Value::as_str) == Some("novelai");
    let provider = model
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut body = Map::new();

    body.insert(
        "model".to_string(),
        json!(model
            .get("modelName")
            .and_then(Value::as_str)
            .unwrap_or_default()),
    );
    body.insert(
        "messages".to_string(),
        serde_json::to_value(messages).unwrap_or_else(|_| Value::Array(Vec::new())),
    );
    // 温度由生成参数策略决议：`None` 是「必须省略」的指令，不得回退到 model.temperature。
    if let Some(max_tokens) = opts
        .max_tokens
        .or_else(|| model.get("maxTokens").and_then(Value::as_u64))
    {
        body.insert(
            opts.output_token_parameter
                .unwrap_or("max_tokens")
                .to_string(),
            json!(max_tokens),
        );
    }
    body.insert("stream".to_string(), json!(stream));

    if let Some(temperature) = opts.temperature {
        body.insert("temperature".to_string(), json!(temperature));
    }

    match opts.reasoning.as_ref() {
        Some(ProviderReasoningDirective::OpenAiReasoningEffort { reasoning_effort }) => {
            if !is_novel_ai {
                body.insert(
                    "reasoning_effort".to_string(),
                    serde_json::to_value(reasoning_effort).unwrap_or(Value::Null),
                );
            }
        }
        Some(ProviderReasoningDirective::DeepSeekV4Thinking {
            thinking,
            reasoning_effort,
        }) => {
            if provider == "deepseek" {
                let thinking_value = serde_json::to_value(thinking).unwrap_or(Value::Null);
                body.insert(
                    "thinking".to_string(),
                    json!({ "type": thinking_value.clone() }),
                );
                if thinking_value == json!("enabled") {
                    if let Some(effort) = reasoning_effort {
                        body.insert(
                            "reasoning_effort".to_string(),
                            serde_json::to_value(effort).unwrap_or(Value::Null),
                        );
                    }
                }
            }
        }
        // Gemini 专属指令绝不进入 OpenAI 载荷。
        Some(ProviderReasoningDirective::GeminiThinkingBudget { .. }) => {}
        // 基线 openai-provider.ts:72-77：openai-thinking-budget
        // 指令以映射 adapter 一致性为门禁（基线另有
        // !isNovelAI 条件）；`enable_thinking` 恒发送，正预算
        // 才发送数值（禁用的 0 无需是合法 provider 预算）。
        Some(ProviderReasoningDirective::OpenAiThinkingBudget { thinking_budget }) => {
            if !is_novel_ai
                && reasoning_mapping_adapter_is(
                    model,
                    crate::llm::reasoning::ReasoningAdapter::OpenAiThinkingBudget,
                )
            {
                body.insert("enable_thinking".to_string(), json!(*thinking_budget > 0));
                if *thinking_budget > 0 {
                    body.insert("thinking_budget".to_string(), json!(*thinking_budget));
                }
            }
        }
        // 基线 openai-provider.ts:79-83：siliconflow-v4-thinking
        // 指令以映射 adapter 一致性为门禁（基线无 !isNovelAI
        // 条件，与 openai-reasoning-effort 分支区别）。
        Some(ProviderReasoningDirective::SiliconFlowV4Thinking { reasoning_effort }) => {
            if reasoning_mapping_adapter_is(
                model,
                crate::llm::reasoning::ReasoningAdapter::SiliconFlowV4Thinking,
            ) {
                body.insert("enable_thinking".to_string(), json!(true));
                body.insert(
                    "reasoning_effort".to_string(),
                    serde_json::to_value(reasoning_effort).unwrap_or(Value::Null),
                );
            }
        }
        None => {}
    }

    if let Some(response_format) = opts.response_format.as_ref() {
        if !is_novel_ai {
            body.insert("response_format".to_string(), response_format.clone());
        }
    }

    // OpenAI 流式接口只有显式请求时才发送最终 usage chunk；NovelAI 保持更窄的兼容载荷。
    if stream && !is_novel_ai {
        body.insert(
            "stream_options".to_string(),
            json!({ "include_usage": true }),
        );
    }

    Value::Object(body)
}

/// 对齐基线 `buildRequestHeaders()`。
fn build_request_headers(
    model: &Value,
    conversation_id: Option<&str>,
) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let api_key = model
        .get("apiKey")
        .and_then(Value::as_str)
        .unwrap_or_default();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|error| format!("API Key 含非法字符：{error}"))?,
    );
    let base_url = model
        .get("baseUrl")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if is_opencode_go_base_url(base_url) {
        // 缺失会话作用域时退化为单请求 UUID，绝不与创作运行共享会话粘性。
        let session_id = conversation_id
            .map(str::to_string)
            .unwrap_or_else(crate::project_access::random_uuid_v4);
        headers.insert(
            "x-opencode-session",
            HeaderValue::from_str(&session_id)
                .map_err(|error| format!("会话标识含非法字符：{error}"))?,
        );
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(&user_agent())
                .map_err(|error| format!("User-Agent 非法：{error}"))?,
        );
    }
    Ok(headers)
}

/// 一次非流式生成。
///
/// 返回**总是**成功（信封而非 `Err`），与基线 provider 的 try/catch 语义一致：
/// 只有 `resolve/params` 决议失败才走 `commands` 层的表达。
pub async fn generate(
    client: &reqwest::Client,
    model: &Value,
    messages: &[ChatMessage],
    opts: &LlmGenerateOptions,
) -> LlmResponse {
    match generate_inner(client, model, messages, opts).await {
        Ok(response) => response,
        Err(error) => LlmResponse::failure(crate::llm::chat::provider_error_text(error)),
    }
}

async fn generate_inner(
    client: &reqwest::Client,
    model: &Value,
    messages: &[ChatMessage],
    opts: &LlmGenerateOptions,
) -> Result<LlmResponse, String> {
    let base_url = model
        .get("baseUrl")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let provider = model
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("openai");
    let url = resolve_chat_completions_url(base_url, provider)?;
    let body = build_request_body(model, messages, opts, false);
    let headers = build_request_headers(model, opts.conversation_id.as_deref())?;

    let response = client
        .post(&url)
        .headers(headers)
        .body(serde_json::to_vec(&body).map_err(|error| error.to_string())?)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        return Ok(LlmResponse {
            success: false,
            content: String::new(),
            finish_reason: LlmFinishReason::Error,
            usage: None,
            error: Some(format!("API 调用失败 ({status}): {text}")),
        });
    }

    let data: Value = response.json().await.map_err(|error| error.to_string())?;
    let choice = data.pointer("/choices/0");
    let raw_content = choice
        .and_then(|choice| choice.pointer("/message/content"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let content = strip_thinking(raw_content);
    let finish_reason = normalize_finish_reason(
        choice
            .and_then(|choice| choice.get("finish_reason"))
            .and_then(Value::as_str),
    );
    let usage = match data.get("usage") {
        Some(Value::Object(raw)) => Some(TokenUsage {
            prompt_tokens: raw.get("prompt_tokens").and_then(Value::as_i64),
            completion_tokens: raw.get("completion_tokens").and_then(Value::as_i64),
            total_tokens: raw.get("total_tokens").and_then(Value::as_i64),
        }),
        _ => None,
    };

    if finish_reason == LlmFinishReason::Stop {
        return Ok(LlmResponse {
            success: true,
            content,
            finish_reason,
            usage,
            error: None,
        });
    }
    Ok(LlmResponse {
        success: false,
        content,
        finish_reason,
        usage,
        error: Some("API 返回的文本未正常完成".to_string()),
    })
}

/// OpenAI 流式事件解码器 —— 无网络依赖的纯状态机。
///
/// 字段与 [`generate_stream`] 内部状态一一对应；`push()` 返回本次新产生的 chunk。
#[derive(Debug, Default)]
pub struct OpenAiSseDecoder {
    is_thinking: bool,
    line_buffer: Vec<u8>,
    data_lines: Vec<String>,
    saw_done: bool,
    finish_reason: Option<LlmFinishReason>,
    fatal_error: Option<String>,
    usage: Option<TokenUsage>,
    full_text: String,
    in_band_reasoning: InBandReasoning,

    visible: crate::llm::visible_stream::VisibleStreamFilter,
    visible_only: bool,
}

impl OpenAiSseDecoder {
    /// 喂入一段字节（`is_final` 表示不会再有余量，需冲刷行缓冲与尾事件）。
    pub fn push(&mut self, bytes: &[u8], is_final: bool) -> Vec<SseDelivery> {
        self.line_buffer.extend_from_slice(bytes);
        let mut deliveries = Vec::new();
        let mut consumed = 0usize;
        let mut index = 0usize;
        while index < self.line_buffer.len() {
            let byte = self.line_buffer[index];
            if byte != b'\r' && byte != b'\n' {
                index += 1;
                continue;
            }
            // 行尾可能是被切开的 CRLF：非最终轮次先等下一个 chunk。
            if byte == b'\r' && index + 1 == self.line_buffer.len() && !is_final {
                break;
            }
            let line = self.line_buffer[consumed..index].to_vec();
            self.process_line(&line, &mut deliveries);
            index += if byte == b'\r' && self.line_buffer.get(index + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
            consumed = index;
            if self.fatal_error.is_some() || self.saw_done {
                break;
            }
        }
        self.line_buffer.drain(..consumed);
        if is_final && self.fatal_error.is_none() && !self.saw_done {
            if !self.line_buffer.is_empty() {
                let tail = std::mem::take(&mut self.line_buffer);
                self.process_line(&tail, &mut deliveries);
            }
            // 末尾的空行触发最后一个事件的派发（对齐基线 `processLine('')`）。
            self.process_line(b"", &mut deliveries);
            self.line_buffer.clear();
        }
        deliveries
    }

    /// 致命错误（损坏载荷 / 类型错误），非 `None` 时调用方必须中止。
    pub fn fatal_error(&self) -> Option<&str> {
        self.fatal_error.as_deref()
    }

    /// 是否已收到 `[DONE]` 完成标记。
    pub fn saw_done(&self) -> bool {
        self.saw_done
    }

    /// 归一后的终态证据（缺失即 `unknown`）。
    pub fn finish_reason(&self) -> LlmFinishReason {
        self.finish_reason.unwrap_or(LlmFinishReason::Unknown)
    }

    /// 已解析到的 token 用量。
    pub fn usage(&self) -> Option<TokenUsage> {
        self.usage.clone()
    }

    /// 交付给 `onDone` 的最终文本（已剥离推理块）。
    pub fn visible_full_text(&self) -> String {
        if self.visible_only {
            self.visible.text().to_string()
        } else {
            strip_thinking(&self.full_text)
        }
    }

    /// 交付给 `onError` 的可见候选（为空时返回 `None`，对齐基线 `|| undefined`）。
    pub fn visible_candidate(&self) -> Option<String> {
        let visible = self.visible_full_text();
        if visible.is_empty() {
            None
        } else {
            Some(visible)
        }
    }

    /// 流结束时若推理块仍未闭合，补发闭合标签（对齐基线在 `onDone` 前补 `\n</think>\n\n`）。
    pub fn close_thinking(&mut self) -> Option<String> {
        if self.visible_only || !self.is_thinking {
            return None;
        }
        self.is_thinking = false;
        let close_tag = "\n</think>\n\n".to_string();
        self.full_text.push_str(&close_tag);
        Some(close_tag)
    }

    fn process_line(&mut self, line: &[u8], deliveries: &mut Vec<SseDelivery>) {
        if line.is_empty() {
            if !self.data_lines.is_empty() {
                let data = self.data_lines.join("\n");
                self.data_lines.clear();
                self.process_event(&data, deliveries);
            }
            return;
        }
        if line[0] == b':' {
            return;
        }
        let text = String::from_utf8_lossy(line);
        let colon = text.find(':');
        let field = match colon {
            Some(index) => &text[..index],
            None => &text[..],
        };
        if field != "data" {
            return;
        }
        let mut value = match colon {
            Some(index) => &text[index + 1..],
            None => "",
        };
        if let Some(stripped) = value.strip_prefix(' ') {
            value = stripped;
        }
        self.data_lines.push(value.to_string());
    }

    fn process_event(&mut self, data: &str, deliveries: &mut Vec<SseDelivery>) {
        if self.fatal_error.is_some() || self.saw_done {
            return;
        }
        let payload_text = data.trim();
        if payload_text.is_empty() {
            return;
        }
        if payload_text == "[DONE]" {
            self.saw_done = true;
            return;
        }

        let parsed: Value = match serde_json::from_str(payload_text) {
            Ok(value) => value,
            Err(_) => {
                self.fatal_error = Some("响应流包含损坏的 JSON 数据".to_string());
                return;
            }
        };
        let Value::Object(payload) = parsed else {
            self.fatal_error = Some("响应流包含无效的 OpenAI 数据对象".to_string());
            return;
        };

        if payload.contains_key("error") {
            let message = payload
                .get("error")
                .and_then(Value::as_object)
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("供应商返回流式错误");
            self.fatal_error = Some(format!("供应商返回流式错误：{message}"));
            return;
        }

        if let Some(Value::Object(raw_usage)) = payload.get("usage") {
            // 基线：`usage` 为非空非数组对象即发射证据——数值字段
            // 缺失记 `null`（不跳过整条证据）；`reasoning_tokens`
            // 取自 `completion_tokens_details.reasoning_tokens`。
            let reasoning_tokens = raw_usage
                .get("completion_tokens_details")
                .and_then(|details| details.get("reasoning_tokens"))
                .and_then(Value::as_i64);
            deliveries.push(SseDelivery::UsageEvidence(ProviderUsageEvidence {
                usage: TokenUsage {
                    prompt_tokens: raw_usage.get("prompt_tokens").and_then(Value::as_i64),
                    completion_tokens: raw_usage.get("completion_tokens").and_then(Value::as_i64),
                    total_tokens: raw_usage.get("total_tokens").and_then(Value::as_i64),
                },
                reasoning_tokens,
                accounting: UsageAccounting::IncludedInCompletion,
                total_includes_reasoning: true,
                protocol: UsageProtocol::OpenAi,
            }));
            let prompt = raw_usage.get("prompt_tokens").and_then(Value::as_i64);
            let completion = raw_usage.get("completion_tokens").and_then(Value::as_i64);
            let total = raw_usage.get("total_tokens").and_then(Value::as_i64);
            if let (Some(prompt), Some(completion), Some(total)) = (prompt, completion, total) {
                self.usage = Some(TokenUsage {
                    prompt_tokens: Some(prompt),
                    completion_tokens: Some(completion),
                    total_tokens: Some(total),
                });
            }
        }

        let Some(choices_value) = payload.get("choices") else {
            return;
        };
        let Value::Array(choices) = choices_value else {
            self.fatal_error = Some("响应流的 choices 类型无效".to_string());
            return;
        };
        let Some(raw_choice) = choices.first() else {
            return;
        };
        let Value::Object(choice) = raw_choice else {
            self.fatal_error = Some("响应流的 choice 类型无效".to_string());
            return;
        };

        match choice.get("finish_reason") {
            None | Some(Value::Null) => {}
            Some(Value::String(reason)) => {
                self.finish_reason = Some(normalize_finish_reason(Some(reason)));
            }
            Some(_) => {
                self.fatal_error = Some("响应流的 finish_reason 类型无效".to_string());
                return;
            }
        }

        let Some(delta_value) = choice.get("delta") else {
            return;
        };
        let Value::Object(delta) = delta_value else {
            self.fatal_error = Some("响应流的 delta 类型无效".to_string());
            return;
        };

        let reasoning = match delta.get("reasoning_content") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(text.as_str()),
            Some(_) => {
                self.fatal_error = Some("响应流的 reasoning_content 类型无效".to_string());
                return;
            }
        };
        let content = match delta.get("content") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(text.as_str()),
            Some(_) => {
                self.fatal_error = Some("响应流的 content 类型无效".to_string());
                return;
            }
        };

        // 基线发射顺序：`reasoning_content` 原文直发推理 →
        // `content` 进带内抽取器（可能发射推理）→ 可见 chunk。
        // 本段只增发射，下方的可见 chunk 构造逻辑逐字不变。
        if let Some(text) = reasoning.filter(|text| !text.is_empty()) {
            deliveries.push(SseDelivery::Reasoning((*text).to_string()));
        }
        if let Some(text) = content {
            if let Some(extracted) = self.in_band_reasoning.push(text) {
                deliveries.push(SseDelivery::Reasoning(extracted));
            }
        }

        let mut emit = String::new();
        if let Some(text) = reasoning.filter(|text| !text.is_empty()) {
            if !self.is_thinking {
                self.is_thinking = true;
                emit.push_str("\x3cthink\x3e\n");
            }
            emit.push_str(text);
        }
        if let Some(text) = content {
            if self.is_thinking {
                self.is_thinking = false;
                emit.push_str("\n\n</think>\n\n");
            }
            emit.push_str(text);
        }
        if self.visible_only {
            emit = self.visible.push(content.unwrap_or_default());
        }

        if !emit.is_empty() {
            self.full_text.push_str(&emit);
            deliveries.push(SseDelivery::Chunk(emit));
        }
    }
}

/// 一次流式生成：chunk / done / error 三态回调，与基线 `generateStream()` 一一对应。
pub async fn generate_stream(
    client: &reqwest::Client,
    model: &Value,
    messages: &[ChatMessage],
    opts: &LlmStreamOptions<'_>,
) {
    let mut decoder = OpenAiSseDecoder {
        visible_only: opts.visible_only,
        ..Default::default()
    };
    match stream_inner(client, model, messages, opts, &mut decoder).await {
        Ok(()) => {}
        Err(failure) => {
            // 基线 `fail()`：把已交付的可见候选一并交回调用方。
            let content = decoder.visible_candidate();
            let usage = decoder.usage();
            (opts.callbacks.on_error)(failure.into_message(), content, usage);
        }
    }
}

async fn stream_inner(
    client: &reqwest::Client,
    model: &Value,
    messages: &[ChatMessage],
    opts: &LlmStreamOptions<'_>,
    decoder: &mut OpenAiSseDecoder,
) -> Result<(), StreamFailure> {
    let base_url = model
        .get("baseUrl")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let provider = model
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("openai");
    let url = resolve_chat_completions_url(base_url, provider).map_err(StreamFailure::Message)?;
    let body = build_request_body(model, messages, &opts.generate, true);
    let headers = build_request_headers(model, opts.generate.conversation_id.as_deref())
        .map_err(StreamFailure::Message)?;

    let mut response = client
        .post(&url)
        .headers(headers)
        .body(serde_json::to_vec(&body).map_err(|error| StreamFailure::Message(error.to_string()))?)
        .send()
        .await
        .map_err(|error| StreamFailure::Message(error.to_string()))?;

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        return Err(StreamFailure::Message(format!(
            "API 调用失败 ({status}): {text}"
        )));
    }

    loop {
        if (opts.is_cancelled)() {
            return Err(StreamFailure::Cancelled);
        }
        if decoder.fatal_error().is_some() || decoder.saw_done() {
            break;
        }
        let chunk = response
            .chunk()
            .await
            .map_err(|error| StreamFailure::Message(error.to_string()))?;
        let Some(bytes) = chunk else {
            break;
        };
        for delivery in decoder.push(&bytes, false) {
            deliver(delivery, &opts.callbacks);
        }
    }

    if decoder.fatal_error().is_none() && !decoder.saw_done() {
        for delivery in decoder.push(&[], true) {
            deliver(delivery, &opts.callbacks);
        }
    }

    if let Some(fatal) = decoder.fatal_error() {
        return Err(StreamFailure::Message(fatal.to_string()));
    }
    if !decoder.saw_done() {
        return Err(StreamFailure::Message(
            "响应流在完成标记前结束，生成结果不完整".to_string(),
        ));
    }
    if let Some(close_tag) = decoder.close_thinking() {
        (opts.callbacks.on_chunk)(&close_tag);
    }
    (opts.callbacks.on_done)(
        decoder.visible_full_text(),
        decoder.usage(),
        decoder.finish_reason(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::chat::{build_client, ProxySpec};

    fn model(value: Value) -> Value {
        value
    }

    /// 只取解码交付物中的可见 chunk（既有断言口径）。
    fn visible_chunks(deliveries: Vec<SseDelivery>) -> Vec<String> {
        deliveries
            .into_iter()
            .filter_map(|delivery| match delivery {
                SseDelivery::Chunk(chunk) => Some(chunk),
                _ => None,
            })
            .collect()
    }
    #[test]
    fn resolve_url_matches_baseline_branches_test() {
        let cases = [
            (
                "https://api.deepseek.com",
                "deepseek",
                "https://api.deepseek.com/v1/chat/completions",
            ),
            (
                "https://api.deepseek.com/",
                "openai",
                "https://api.deepseek.com/v1/chat/completions",
            ),
            (
                "https://host/v1",
                "openai",
                "https://host/v1/chat/completions",
            ),
            (
                "https://host/v1/chat/completions",
                "openai",
                "https://host/v1/chat/completions",
            ),
            (
                "https://host/v1/chat",
                "openai",
                "https://host/v1/chat/completions",
            ),
            (
                "https://host/custom",
                "novelai",
                "https://host/custom/v1/chat/completions",
            ),
        ];
        for (base_url, provider, expected) in cases {
            assert_eq!(
                resolve_chat_completions_url(base_url, provider).unwrap(),
                expected,
                "base_url={base_url} provider={provider}"
            );
        }
        assert!(resolve_chat_completions_url("不是 URL", "openai").is_err());
    }

    #[test]
    fn opencode_go_detection_matches_baseline_test() {
        assert!(is_opencode_go_base_url("https://opencode.ai/zen/go"));
        assert!(is_opencode_go_base_url("https://opencode.ai/zen/go/"));
        assert!(is_opencode_go_base_url("https://opencode.ai/zen/go/v1"));
        assert!(is_opencode_go_base_url("  https://opencode.ai/zen/go  "));
        assert!(!is_opencode_go_base_url("https://opencode.ai/zen"));
        assert!(!is_opencode_go_base_url("https://opencode.ai/zen/gopher"));
        assert!(!is_opencode_go_base_url("http://opencode.ai/zen/go"));
        assert!(!is_opencode_go_base_url("https://other.ai/zen/go"));
    }

    #[test]
    fn normalize_finish_reason_maps_provider_vocabulary_test() {
        assert_eq!(normalize_finish_reason(Some("stop")), LlmFinishReason::Stop);
        assert_eq!(
            normalize_finish_reason(Some("length")),
            LlmFinishReason::Length
        );
        assert_eq!(
            normalize_finish_reason(Some("model_context_window_exceeded")),
            LlmFinishReason::Length
        );
        assert_eq!(
            normalize_finish_reason(Some("content_filter")),
            LlmFinishReason::ContentFilter
        );
        assert_eq!(
            normalize_finish_reason(Some("sensitive")),
            LlmFinishReason::ContentFilter
        );
        assert_eq!(
            normalize_finish_reason(Some("network_error")),
            LlmFinishReason::Error
        );
        assert_eq!(normalize_finish_reason(None), LlmFinishReason::Unknown);
        assert_eq!(
            normalize_finish_reason(Some("tool_calls")),
            LlmFinishReason::Unknown
        );
    }

    #[test]
    fn request_body_omits_temperature_when_undefined_test() {
        let profile = model(json!({
            "provider": "deepseek",
            "modelName": "deepseek-v4-flash",
            "temperature": 0.7,
            "maxTokens": 4096,
        }));
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "你好".to_string(),
        }];
        let omitted = build_request_body(
            &profile,
            &messages,
            &LlmGenerateOptions {
                temperature: None,
                max_tokens: None,
                output_token_parameter: None,
                response_format: None,
                reasoning: None,
                conversation_id: None,
            },
            false,
        );
        assert!(
            omitted.get("temperature").is_none(),
            "temperature 必须整键省略"
        );
        assert_eq!(omitted.get("max_tokens"), Some(&json!(4096)));
        assert_eq!(omitted.get("stream"), Some(&json!(false)));
        assert!(omitted.get("stream_options").is_none());

        let explicit = build_request_body(
            &profile,
            &messages,
            &LlmGenerateOptions {
                temperature: Some(0.2),
                max_tokens: Some(64),
                output_token_parameter: None,
                response_format: None,
                reasoning: None,
                conversation_id: None,
            },
            true,
        );
        assert_eq!(explicit.get("temperature"), Some(&json!(0.2)));
        assert_eq!(explicit.get("max_tokens"), Some(&json!(64)));
        assert_eq!(
            explicit.get("stream_options"),
            Some(&json!({"include_usage": true}))
        );
    }

    #[test]
    fn request_body_uses_selected_output_token_parameter_test() {
        let profile = model(json!({ "maxTokens": 4096 }));
        for stream in [false, true] {
            for max_tokens in [None, Some(8192)] {
                let body = build_request_body(
                    &profile,
                    &[],
                    &LlmGenerateOptions {
                        temperature: None,
                        max_tokens,
                        output_token_parameter: Some("max_completion_tokens"),
                        response_format: None,
                        reasoning: None,
                        conversation_id: None,
                    },
                    stream,
                );
                assert_eq!(
                    body["max_completion_tokens"],
                    json!(max_tokens.unwrap_or(4096))
                );
                assert!(body.get("max_tokens").is_none());
            }
        }
    }

    #[test]
    fn request_body_novelai_keeps_narrow_compatibility_payload_test() {
        let profile = model(json!({
            "provider": "novelai",
            "modelName": "kayra",
            "maxTokens": 256,
        }));
        let body = build_request_body(
            &profile,
            &[],
            &LlmGenerateOptions {
                temperature: Some(0.5),
                max_tokens: None,
                output_token_parameter: None,
                response_format: Some(json!({"type": "json_object"})),
                reasoning: Some(ProviderReasoningDirective::OpenAiReasoningEffort {
                    reasoning_effort: "high".to_string(),
                }),
                conversation_id: None,
            },
            true,
        );
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("response_format").is_none());
        assert!(body.get("stream_options").is_none());
        assert_eq!(body.get("temperature"), Some(&json!(0.5)));
    }

    #[test]
    fn request_body_deepseek_thinking_only_for_deepseek_test() {
        let directive = Some(ProviderReasoningDirective::DeepSeekV4Thinking {
            thinking: crate::llm::reasoning::ThinkingState::Enabled,
            reasoning_effort: Some(crate::llm::reasoning::ReasoningEffort::High),
        });
        let deepseek = build_request_body(
            &model(json!({"provider": "deepseek", "modelName": "m", "maxTokens": 1})),
            &[],
            &LlmGenerateOptions {
                temperature: None,
                max_tokens: None,
                output_token_parameter: None,
                response_format: None,
                reasoning: directive.clone(),
                conversation_id: None,
            },
            false,
        );
        assert_eq!(deepseek.get("thinking"), Some(&json!({"type": "enabled"})));
        assert_eq!(deepseek.get("reasoning_effort"), Some(&json!("high")));

        let other = build_request_body(
            &model(json!({"provider": "openai", "modelName": "m", "maxTokens": 1})),
            &[],
            &LlmGenerateOptions {
                temperature: None,
                max_tokens: None,
                output_token_parameter: None,
                response_format: None,
                reasoning: directive,
                conversation_id: None,
            },
            false,
        );
        assert!(other.get("thinking").is_none());
        assert!(other.get("reasoning_effort").is_none());
    }
    #[test]
    fn request_body_translates_openai_thinking_budget_for_qwen_test() {
        // 基线 'fixed Qwen native admission'：Qwen/Qwen3.8-27B
        // 的 openai-thinking-budget 映射（medium → 16_384）。
        let qwen = model(json!({
            "provider": "siliconflow",
            "protocol": "openai",
            "baseUrl": "https://api.siliconflow.cn/v1",
            "modelName": "Qwen/Qwen3.8-27B",
            "maxTokens": 16384,
        }));
        let messages = vec![];
        let body = build_request_body(
            &qwen,
            &messages,
            &LlmGenerateOptions {
                temperature: None,
                max_tokens: Some(16_384),
                output_token_parameter: None,
                response_format: None,
                reasoning: Some(ProviderReasoningDirective::OpenAiThinkingBudget {
                    thinking_budget: 16_384,
                }),
                conversation_id: None,
            },
            false,
        );
        assert_eq!(body.get("enable_thinking"), Some(&json!(true)));
        assert_eq!(body.get("thinking_budget"), Some(&json!(16_384)));
        assert!(body.get("reasoning_effort").is_none());

        // 禁用预算（0）→ enable_thinking=false 且无数值字段
        // （基线：禁用的 0 无需是合法 provider 预算）。
        let body = build_request_body(
            &qwen,
            &messages,
            &LlmGenerateOptions {
                temperature: None,
                max_tokens: Some(16_384),
                output_token_parameter: None,
                response_format: None,
                reasoning: Some(ProviderReasoningDirective::OpenAiThinkingBudget {
                    thinking_budget: 0,
                }),
                conversation_id: None,
            },
            false,
        );
        assert_eq!(body.get("enable_thinking"), Some(&json!(false)));
        assert!(body.get("thinking_budget").is_none());
    }

    #[test]
    fn request_body_translates_siliconflow_v4_thinking_for_deepseek_v4_test() {
        // 基线 'keeps SiliconFlow output and liability unchanged
        // for explicit high/max'：DeepSeek-V4-Flash 的
        // siliconflow-v4-thinking 映射（high/max）。
        let flash = model(json!({
            "provider": "siliconflow",
            "protocol": "openai",
            "baseUrl": "https://api.siliconflow.cn/v1",
            "modelName": "deepseek-ai/DeepSeek-V4-Flash",
            "maxTokens": 16384,
        }));
        let body = build_request_body(
            &flash,
            &[],
            &LlmGenerateOptions {
                temperature: None,
                max_tokens: Some(16_384),
                output_token_parameter: None,
                response_format: None,
                reasoning: Some(ProviderReasoningDirective::SiliconFlowV4Thinking {
                    reasoning_effort: crate::llm::reasoning::ReasoningEffort::High,
                }),
                conversation_id: None,
            },
            false,
        );
        assert_eq!(body.get("enable_thinking"), Some(&json!(true)));
        assert_eq!(body.get("reasoning_effort"), Some(&json!("high")));
        assert!(body.get("thinking_budget").is_none());
    }

    #[test]
    fn request_body_reasoning_gate_requires_matching_verified_mapping_test() {
        // 映射门禁不匹配（自建端点无已验证映射）→
        // 推理字段不进载荷。
        let relay = model(json!({
            "provider": "siliconflow",
            "protocol": "openai",
            "baseUrl": "https://relay.example.com/v1",
            "modelName": "deepseek-ai/DeepSeek-V4-Flash",
            "maxTokens": 16384,
        }));
        let directives = [
            ProviderReasoningDirective::SiliconFlowV4Thinking {
                reasoning_effort: crate::llm::reasoning::ReasoningEffort::High,
            },
            ProviderReasoningDirective::OpenAiThinkingBudget {
                thinking_budget: 16_384,
            },
        ];
        for directive in directives {
            let body = build_request_body(
                &relay,
                &[],
                &LlmGenerateOptions {
                    temperature: None,
                    max_tokens: Some(16_384),
                    output_token_parameter: None,
                    response_format: None,
                    reasoning: Some(directive),
                    conversation_id: None,
                },
                false,
            );
            assert!(body.get("enable_thinking").is_none());
            assert!(body.get("thinking_budget").is_none());
            assert!(body.get("reasoning_effort").is_none());
        }
    }

    /// 用一段合成的 OpenAI SSE 流驱动解码器：验证分片（含 CRLF 切断）、
    /// reasoning→`<think>` 包裹、usage 收集与终态归一。
    #[test]
    fn sse_decoder_emits_think_wrapped_chunks_and_final_evidence_test() {
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"想\"}}]}\r\n\r\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"一下\"}}]}\r\n\r\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"正文\"}}]}\r\n\r\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],",
            "\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":5,\"total_tokens\":8}}\r\n\r\n",
            ": 心跳注释\n",
            "data: [DONE]\r\n\r\n",
        );
        let bytes = stream.as_bytes();
        let mut decoder = OpenAiSseDecoder::default();
        let mut chunks: Vec<String> = Vec::new();
        // 故意按 3 字节切分，制造行尾/多字节被切断的情形。
        for piece in bytes.chunks(3) {
            chunks.extend(visible_chunks(decoder.push(piece, false)));
        }
        assert!(
            decoder.fatal_error().is_none(),
            "{:?}",
            decoder.fatal_error()
        );
        assert!(decoder.saw_done(), "[DONE] 必须被识别");
        assert_eq!(decoder.finish_reason(), LlmFinishReason::Stop);
        assert_eq!(
            decoder.usage(),
            Some(TokenUsage {
                prompt_tokens: Some(3),
                completion_tokens: Some(5),
                total_tokens: Some(8)
            })
        );
        let emitted = chunks.concat();
        assert!(emitted.starts_with("<think>\n想一下"));
        assert!(emitted.contains("</think>"));
        assert!(emitted.ends_with("正文"));
        assert_eq!(decoder.visible_full_text(), "正文");
    }

    /// 基线 `owner-visible-stream.test.ts`：`usage` 对象存在即发射
    /// 证据（数值字段缺失记 `null`，不跳过整条证据）；
    /// `reasoning_tokens` 取自 `completion_tokens_details.reasoning_tokens`。
    #[test]
    fn sse_decoder_emits_usage_evidence_with_reasoning_split_test() {
        let mut decoder = OpenAiSseDecoder::default();
        let deliveries = decoder.push(
            b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":20,\"total_tokens\":30,\"completion_tokens_details\":{\"reasoning_tokens\":7}}}\n\n",
            false,
        );
        assert_eq!(
            deliveries,
            vec![SseDelivery::UsageEvidence(ProviderUsageEvidence {
                usage: TokenUsage {
                    prompt_tokens: Some(10),
                    completion_tokens: Some(20),
                    total_tokens: Some(30),
                },
                reasoning_tokens: Some(7),
                accounting: UsageAccounting::IncludedInCompletion,
                total_includes_reasoning: true,
                protocol: UsageProtocol::OpenAi,
            })]
        );
        // 分账明细缺失时 `reasoning_tokens` 记 `None`（基线 `?? null`）。
        let mut decoder = OpenAiSseDecoder::default();
        let deliveries = decoder.push(
            b"data: {\"choices\":[],\"usage\":{\"completion_tokens_details\":{}}}\n\n",
            false,
        );
        assert_eq!(
            deliveries,
            vec![SseDelivery::UsageEvidence(ProviderUsageEvidence {
                usage: TokenUsage {
                    prompt_tokens: None,
                    completion_tokens: None,
                    total_tokens: None,
                },
                reasoning_tokens: None,
                accounting: UsageAccounting::IncludedInCompletion,
                total_includes_reasoning: true,
                protocol: UsageProtocol::OpenAi,
            })]
        );
    }

    /// 基线「never emits protocol reasoning fields」：
    /// `reasoning_content` 原文直发推理；可见文本中的带内
    /// `</think>` 块经抽取器发射（半枚标签跨 chunk 暂存，
    /// 不泄漏推理前缀）。
    #[test]
    fn sse_decoder_emits_reasoning_raw_and_in_band_test() {
        let mut decoder = OpenAiSseDecoder::default();
        let mut reasoning: Vec<String> = Vec::new();
        let mut chunks: Vec<String> = Vec::new();
        for piece in [
            b"data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"protocol secret\"}}]}\n\n".as_slice(),
            "data: {\"choices\":[{\"delta\":{\"content\":\" 前<th\"}}]}\n\n".as_bytes(),
            "data: {\"choices\":[{\"delta\":{\"content\":\"ink>inband secret</think>乙 \\n\"}}]}\n\n".as_bytes(),
        ] {
            for delivery in decoder.push(piece, false) {
                match delivery {
                    SseDelivery::Chunk(chunk) => chunks.push(chunk),
                    SseDelivery::Reasoning(text) => reasoning.push(text),
                    SseDelivery::UsageEvidence(_) => {}
                }
            }
        }
        assert_eq!(reasoning.concat(), "protocol secretinband secret");
        assert_eq!(chunks.len(), 3);
        // 非 visibleOnly 路径：`onDone` 文本经 `strip_thinking` 归一
        // （`<th` 与 `ink>` 拼成完整开标签，块内文本被剥离，与基线
        // 非 visibleOnly 路径的 `stripThinking(fullText)` 一致）。
        assert_eq!(decoder.visible_full_text(), "前乙");
    }

    /// 基线「delivers split in-band thoughts separately」：拆分到
    /// 三个 chunk 的思维块只发射一次抽取结果，且块内标记（`<em>`）
    /// 保留；闭合标签跨 chunk 拼接后才触发剥离。
    #[test]
    fn sse_decoder_holds_split_in_band_thoughts_test() {
        let mut decoder = OpenAiSseDecoder::default();
        let mut reasoning: Vec<String> = Vec::new();
        let mut chunks: Vec<String> = Vec::new();
        for piece in [
            "data: {\"choices\":[{\"delta\":{\"content\":\"前<th\"}}]}\n\n".as_bytes(),
            "data: {\"choices\":[{\"delta\":{\"content\":\"ink>秘密<em>里</em></thi\"}}]}\n\n".as_bytes(),
            "data: {\"choices\":[{\"delta\":{\"content\":\"nk>后\",\"finish_reason\":\"stop\"}}]}\n\n".as_bytes(),
        ] {
            for delivery in decoder.push(piece, false) {
                match delivery {
                    SseDelivery::Chunk(chunk) => chunks.push(chunk),
                    SseDelivery::Reasoning(text) => reasoning.push(text),
                    SseDelivery::UsageEvidence(_) => {}
                }
            }
        }
        assert_eq!(reasoning.concat(), "秘密<em>里</em>");
        assert_eq!(
            chunks.concat(),
            "前\x3cthink\x3e秘密<em>里</em>\x3c/think\x3e后"
        );
        assert_eq!(decoder.visible_full_text(), "前后");
    }
    #[test]
    fn sse_decoder_reports_typed_fatal_errors_test() {
        // 损坏 JSON
        let mut decoder = OpenAiSseDecoder::default();
        decoder.push(b"data: {oops}\n\n", false);
        assert_eq!(decoder.fatal_error(), Some("响应流包含损坏的 JSON 数据"));

        // 非对象载荷
        let mut decoder = OpenAiSseDecoder::default();
        decoder.push(b"data: [1,2]\n\n", false);
        assert_eq!(
            decoder.fatal_error(),
            Some("响应流包含无效的 OpenAI 数据对象")
        );

        // choices 类型错误
        let mut decoder = OpenAiSseDecoder::default();
        decoder.push(b"data: {\"choices\":\"x\"}\n\n", false);
        assert_eq!(decoder.fatal_error(), Some("响应流的 choices 类型无效"));

        // delta 类型错误
        let mut decoder = OpenAiSseDecoder::default();
        decoder.push(b"data: {\"choices\":[{\"delta\":7}]}\n\n", false);
        assert_eq!(decoder.fatal_error(), Some("响应流的 delta 类型无效"));

        // content 类型错误
        let mut decoder = OpenAiSseDecoder::default();
        decoder.push(
            b"data: {\"choices\":[{\"delta\":{\"content\":1}}]}\n\n",
            false,
        );
        assert_eq!(decoder.fatal_error(), Some("响应流的 content 类型无效"));

        // finish_reason 类型错误
        let mut decoder = OpenAiSseDecoder::default();
        decoder.push(b"data: {\"choices\":[{\"finish_reason\":1}]}\n\n", false);
        assert_eq!(
            decoder.fatal_error(),
            Some("响应流的 finish_reason 类型无效")
        );
    }

    #[test]
    fn sse_decoder_reports_provider_error_payload_test() {
        let mut decoder = OpenAiSseDecoder::default();
        decoder.push(
            "data: {\"error\":{\"message\":\"限流\"}}\n\n".as_bytes(),
            false,
        );
        assert_eq!(decoder.fatal_error(), Some("供应商返回流式错误：限流"));

        // `error` 为非对象时仍进入错误分支（基线的 `Object.hasOwn` 不看值类型），
        // 回退文案又被拼进模板 —— 基线的这种重复措辞是刻意复刻的。
        let mut decoder = OpenAiSseDecoder::default();
        decoder.push(b"data: {\"error\":null}\n\n", false);
        assert_eq!(
            decoder.fatal_error(),
            Some("供应商返回流式错误：供应商返回流式错误")
        );
    }

    #[test]
    fn sse_decoder_joins_multiline_data_and_ignores_other_fields_test() {
        let mut decoder = OpenAiSseDecoder::default();
        // 多行 data: 由空行统一派发；`event:` 字段被忽略。
        decoder.push(b"event: message\n", false);
        decoder.push(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\n",
            false,
        );
        decoder.push(b"\n", false);
        assert!(
            decoder.fatal_error().is_none(),
            "`event:` 字段不得影响 data 解析：{:?}",
            decoder.fatal_error()
        );
        assert_eq!(decoder.visible_full_text(), "A");
    }

    #[test]
    fn sse_decoder_closes_unterminated_thinking_test() {
        let mut decoder = OpenAiSseDecoder::default();
        decoder.push(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"想\"}}]}\n\ndata: [DONE]\n\n"
                .as_bytes(),
            false,
        );
        let close = decoder.close_thinking();
        assert!(close.is_some());
        assert_eq!(decoder.close_thinking(), None, "闭合标签只补发一次");
        assert_eq!(decoder.visible_full_text(), "");
    }

    #[test]
    fn client_builds_with_http_and_socks_proxy_test() {
        // 仅验证代理 URL 可被 reqwest 接受（不发起任何请求）。
        for scheme in ["http", "socks5"] {
            let spec = ProxySpec {
                scheme: scheme.to_string(),
                host: "127.0.0.1".to_string(),
                port: 7890,
            };
            assert!(build_client(Some(&spec)).is_ok(), "{scheme} 代理应可装配");
        }
        assert!(build_client(None).is_ok());
    }
}
