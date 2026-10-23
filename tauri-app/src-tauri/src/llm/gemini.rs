//! 批次 D2-b：Gemini 协议 provider。
//!
//! 迁移自 `electron/llm/gemini-provider.ts`。
//!
//! 刻意与 OpenAI provider 保持**行为差异**（两者在基线上本就不同，不得被「顺手统一」）：
//!
//! | 维度 | OpenAI | Gemini |
//! |---|---|---|
//! | `<think>` 剥离 | 有（三条规则） | **无** |
//! | SSE 解析 | 严格（类型错误即致命） | 宽松（损坏 keepalive 直接忽略） |
//! | 失败时回传可见候选 | 有（`content` + `usage`） | **无**（仅错误文案） |
//! | usage / finishReason 归一来源 | `choices[0].finish_reason` | `candidates[0].finishReason` |

use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde_json::{json, Map, Value};

use crate::llm::chat::{
    deliver, ChatMessage, InBandReasoning, LlmFinishReason, LlmGenerateOptions, LlmResponse,
    LlmStreamOptions, ProviderUsageEvidence, SseDelivery, StreamFailure, TokenUsage,
    UsageAccounting, UsageProtocol,
};
use crate::llm::reasoning::ProviderReasoningDirective;

/// 对齐基线 `normalizeFinishReason()`。
///
/// `None` 与 `null` 都落到 `unknown` —— 注意基线判据是 `!== undefined`，
/// 因此 `finishReason: null` **也会**被归一为 `unknown`（不是跳过）。
pub fn normalize_finish_reason(reason: Option<&str>) -> LlmFinishReason {
    match reason {
        Some("STOP") => LlmFinishReason::Stop,
        Some("MAX_TOKENS") => LlmFinishReason::Length,
        Some("SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT") => {
            LlmFinishReason::ContentFilter
        }
        _ => LlmFinishReason::Unknown,
    }
}

/// 对齐基线 `toGeminiContents()`：`system` 提升为 `systemInstruction`（后写覆盖），
/// 其余消息 `assistant` → `model`，其它 → `user`。
fn to_gemini_contents(messages: &[ChatMessage]) -> (Vec<Value>, Option<String>) {
    let mut system_instruction: Option<String> = None;
    let mut contents: Vec<Value> = Vec::new();
    for message in messages {
        if message.role == "system" {
            system_instruction = Some(message.content.clone());
            continue;
        }
        let role = if message.role == "assistant" {
            "model"
        } else {
            "user"
        };
        contents.push(json!({
            "role": role,
            "parts": [{ "text": message.content }],
        }));
    }
    (contents, system_instruction)
}

/// 对齐基线 `applyReasoning()`：只有 `gemini-thinking-budget` 会进入 `generationConfig`。
fn apply_reasoning(generation_config: &mut Map<String, Value>, opts: &LlmGenerateOptions) {
    if let Some(ProviderReasoningDirective::GeminiThinkingBudget { thinking_budget }) =
        opts.reasoning.as_ref()
    {
        generation_config.insert(
            "thinkingConfig".to_string(),
            json!({ "thinkingBudget": thinking_budget }),
        );
    }
}

/// 请求端点（`baseUrl` 去掉**一个**尾部 `/` 后拼接，不做 URL 规范化 ——
/// 对齐基线 `baseUrl.replace(/\/$/, '')` 的单次替换语义）。
pub fn generate_content_url(base_url: &str, model_name: &str, stream: bool) -> String {
    let base = base_url.strip_suffix('/').unwrap_or(base_url);
    if stream {
        format!("{base}/v1beta/models/{model_name}:streamGenerateContent?alt=sse")
    } else {
        format!("{base}/v1beta/models/{model_name}:generateContent")
    }
}

/// 对齐基线 Gemini 请求载荷构造（非流式与流式共用）。
pub fn build_request_body(
    model: &Value,
    messages: &[ChatMessage],
    opts: &LlmGenerateOptions,
) -> Value {
    let (contents, system_instruction) = to_gemini_contents(messages);
    let mut generation_config = Map::new();
    // `undefined` 的键在 `JSON.stringify` 中被丢弃，故 `None` 时必须整键省略。
    if let Some(max_tokens) = opts
        .max_tokens
        .or_else(|| model.get("maxTokens").and_then(Value::as_u64))
    {
        generation_config.insert("maxOutputTokens".to_string(), json!(max_tokens));
    }
    if let Some(temperature) = opts.temperature {
        generation_config.insert("temperature".to_string(), json!(temperature));
    }
    if opts
        .response_format
        .as_ref()
        .and_then(|format| format.get("type"))
        .and_then(Value::as_str)
        == Some("json_object")
    {
        generation_config.insert("responseMimeType".to_string(), json!("application/json"));
    }
    apply_reasoning(&mut generation_config, opts);

    let mut body = Map::new();
    body.insert("contents".to_string(), Value::Array(contents));
    body.insert(
        "generationConfig".to_string(),
        Value::Object(generation_config),
    );
    if let Some(system_instruction) = system_instruction {
        body.insert(
            "systemInstruction".to_string(),
            json!({ "parts": [{ "text": system_instruction }] }),
        );
    }
    Value::Object(body)
}

fn build_request_headers(model: &Value) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let api_key = model
        .get("apiKey")
        .and_then(Value::as_str)
        .unwrap_or_default();
    headers.insert(
        "x-goog-api-key",
        HeaderValue::from_str(api_key).map_err(|error| format!("API Key 含非法字符：{error}"))?,
    );
    Ok(headers)
}

fn usage_from(metadata: Option<&Value>) -> Option<TokenUsage> {
    let Value::Object(raw) = metadata? else {
        return None;
    };
    Some(TokenUsage {
        prompt_tokens: raw.get("promptTokenCount").and_then(Value::as_i64),
        completion_tokens: raw.get("candidatesTokenCount").and_then(Value::as_i64),
        total_tokens: raw.get("totalTokenCount").and_then(Value::as_i64),
    })
}

/// 基线 `if (parsed.usageMetadata)` 的 JS 真值判据：对象 / 数组恒真，
/// `null` / `false` / `0` / 空串为假。
fn usage_metadata_truthy(metadata: &Value) -> bool {
    match metadata {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|value| value != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// 基线 `onUsageEvidence` 发射点：`usageMetadata` 为真值即发射
/// （`accounting: 'separately-billed'`；`reasoning_tokens` 取自
/// `thoughtsTokenCount`）。非对象 metadata 同样发射（usage 全
/// `null`，对齐基线 `?? null` 的弱类型读取）。
fn usage_evidence(metadata: Option<&Value>) -> Option<ProviderUsageEvidence> {
    let metadata = metadata?;
    if !usage_metadata_truthy(metadata) {
        return None;
    }
    let usage = usage_from(Some(metadata)).unwrap_or(TokenUsage {
        prompt_tokens: None,
        completion_tokens: None,
        total_tokens: None,
    });
    Some(ProviderUsageEvidence {
        usage,
        reasoning_tokens: metadata.get("thoughtsTokenCount").and_then(Value::as_i64),
        accounting: UsageAccounting::SeparatelyBilled,
        total_includes_reasoning: true,
        protocol: UsageProtocol::Gemini,
    })
}

fn response_text(data: &Value) -> String {
    data.pointer("/candidates/0/content/parts/0/text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// 一次非流式生成（`Err` 仅用于非常规失败，信封语义与 OpenAI provider 一致）。
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
    let model_name = model
        .get("modelName")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let url = generate_content_url(base_url, model_name, false);
    let body = build_request_body(model, messages, opts);

    let response = client
        .post(&url)
        .headers(build_request_headers(model)?)
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
            error: Some(format!("Gemini API 调用失败 ({status}): {text}")),
            http_failure: Some(crate::llm::chat::HttpFailure { status, body: text }),
        });
    }

    let data: Value = response.json().await.map_err(|error| error.to_string())?;
    let text = response_text(&data);
    let finish_reason = normalize_finish_reason(
        data.pointer("/candidates/0/finishReason")
            .and_then(Value::as_str),
    );
    let usage = usage_from(data.get("usageMetadata"));

    if finish_reason == LlmFinishReason::Stop {
        return Ok(LlmResponse {
            success: true,
            content: text,
            finish_reason,
            usage,
            error: None,
            http_failure: None,
        });
    }
    Ok(LlmResponse {
        success: false,
        content: text,
        finish_reason,
        usage,
        error: Some("Gemini API 返回的文本未正常完成".to_string()),
        http_failure: None,
    })
}

/// Gemini 流式解码器：旧 IPC 宽松解析；持久化可见流严格校验载荷。
///
/// 行缓冲是**字节**级（`Vec<u8>`）：reqwest 的 `chunk()` 可能在任意字节处切分，
/// 若按 `&str` 严格解码，一个多字节字符被拆到两个 chunk 就会误判为流错误。
/// 基线用 `TextDecoder({ stream: true })` 处理同一问题；此处按字节缓冲 + 逐行
/// [`String::from_utf8_lossy`]（非法字节替换为 U+FFFD，与 TextDecoder 一致）。
#[derive(Debug, Default)]
pub struct GeminiSseDecoder {
    buffer: Vec<u8>,
    full_text: String,
    usage: Option<TokenUsage>,
    finish_reason: Option<LlmFinishReason>,
    in_band_reasoning: InBandReasoning,

    visible: crate::llm::visible_stream::VisibleStreamFilter,
    visible_only: bool,
    fatal_error: Option<String>,
}

/// 持久化可见流启用基线的严格载荷门禁；旧 IPC 保留宽松解析。
fn assert_visible_stream_payload(value: &Value) -> Result<(), &'static str> {
    let invalid = "Gemini 响应流格式无效";
    let record = value.as_object().ok_or(invalid)?;
    if record.contains_key("error") {
        return Err("Gemini 响应流报告错误");
    }
    let Some(candidates) = record.get("candidates") else {
        return Ok(());
    };
    for candidate in candidates.as_array().ok_or(invalid)? {
        let candidate = candidate.as_object().ok_or(invalid)?;
        if candidate
            .get("finishReason")
            .is_some_and(|value| !value.is_null() && !value.is_string())
        {
            return Err(invalid);
        }
        let Some(content) = candidate.get("content") else {
            continue;
        };
        let content = content.as_object().ok_or(invalid)?;
        let Some(parts) = content.get("parts") else {
            continue;
        };
        for part in parts.as_array().ok_or(invalid)? {
            let part = part.as_object().ok_or(invalid)?;
            if part.get("text").is_some_and(|value| !value.is_string())
                || part.get("thought").is_some_and(|value| !value.is_boolean())
            {
                return Err(invalid);
            }
        }
    }
    Ok(())
}

impl GeminiSseDecoder {
    /// 喂入一段字节；返回本次新产生的交付物（顺序即交付顺序）。
    pub fn push(&mut self, bytes: &[u8], is_final: bool) -> Vec<SseDelivery> {
        let mut deliveries = Vec::new();
        self.buffer.extend_from_slice(bytes);
        if is_final {
            // 对齐基线：流结束后若仍有残留内容（且非纯空白）则处理一次。
            if !String::from_utf8_lossy(&self.buffer).trim().is_empty() {
                let tail = std::mem::take(&mut self.buffer);
                let line = String::from_utf8_lossy(&tail)
                    .trim_end_matches('\r')
                    .to_string();
                self.process_line(&line, &mut deliveries);
            }
            self.buffer.clear();
            return deliveries;
        }
        while let Some(index) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let consumed: Vec<u8> = self.buffer.drain(..=index).collect();
            // 去掉行尾的 `\n`；行内残留的 `\r` 交给 `process_line` 的 `trim` 处理。
            let line = String::from_utf8_lossy(&consumed[..consumed.len() - 1]).to_string();
            self.process_line(&line, &mut deliveries);
        }
        deliveries
    }

    /// 终态正文；visible_only 模式保留原始空白并隔离带内思考。
    pub fn full_text(&self) -> String {
        self.full_text.clone()
    }

    /// 已解析到的 token 用量。
    pub fn usage(&self) -> Option<TokenUsage> {
        self.usage.clone()
    }

    /// 归一后的终态证据（缺失即 `unknown`）。
    pub fn finish_reason(&self) -> LlmFinishReason {
        self.finish_reason.unwrap_or(LlmFinishReason::Unknown)
    }

    fn process_line(&mut self, line: &str, deliveries: &mut Vec<SseDelivery>) {
        if self.fatal_error.is_some() {
            return;
        }

        let Some(rest) = line.strip_prefix("data: ") else {
            return;
        };
        let json_text = rest.trim();
        if json_text.is_empty() {
            return;
        }
        // 宽松解析：非 `data:` 行与损坏的 keepalive 一并忽略。
        let Ok(parsed) = serde_json::from_str::<Value>(json_text) else {
            if self.visible_only {
                self.fatal_error = Some("Gemini 响应流格式无效".to_string());
            }

            return;
        };
        if self.visible_only {
            if let Err(error) = assert_visible_stream_payload(&parsed) {
                self.fatal_error = Some(error.to_string());
                return;
            }
        }

        if let Some(candidate) = parsed.pointer("/candidates/0") {
            // 基线判据是 `!== undefined`：`null` 与非字符串值同样会被归一为 `unknown`
            // （`as_str()` 仅对字符串返回 `Some`，其余落到 `normalize(None)`）。
            if let Some(finish) = candidate.get("finishReason") {
                self.finish_reason = Some(normalize_finish_reason(finish.as_str()));
            }
            // 基线逐 part 处理：`thought: true` 的 part 原文直发推理；
            // 其余 part 的文本先入带内抽取器（可能发射推理），再汇入
            // 可见 chunk（无 `text` 的 part——如 functionCall——贡献
            // 空串，与基线 `map(...).join('')` 一致）。
            let mut visible = String::new();
            if let Some(parts) = candidate
                .pointer("/content/parts")
                .and_then(Value::as_array)
            {
                for part in parts {
                    let is_thought = part.get("thought").and_then(Value::as_bool) == Some(true);
                    let text = part.get("text").and_then(Value::as_str);
                    if is_thought {
                        // 基线：`if (part.text) opts.onReasoning?.(part.text)`
                        if let Some(text) = text.filter(|text| !text.is_empty()) {
                            deliveries.push(SseDelivery::Reasoning(text.to_string()));
                        }
                    } else if let Some(text) = text {
                        if let Some(extracted) = self.in_band_reasoning.push(text) {
                            deliveries.push(SseDelivery::Reasoning(extracted));
                        }
                        visible.push_str(text);
                    }
                }
            }
            if self.visible_only {
                visible = self.visible.push(&visible);
            }

            if !visible.is_empty() {
                self.full_text.push_str(&visible);
                deliveries.push(SseDelivery::Chunk(visible));
            }
        }
        // 基线：`usageMetadata` 为真值即发射证据（缺失字段记 `null`，
        // 不跳过）；同时作为 `onDone` 的用量来源。
        if let Some(evidence) = usage_evidence(parsed.get("usageMetadata")) {
            self.usage = Some(evidence.usage.clone());
            deliveries.push(SseDelivery::UsageEvidence(evidence));
        }
    }
}

/// 一次流式生成。
///
/// 旧 IPC 失败回调合同不变；visible_only 模式回传可见候选与 usage，
/// 供持久化运行服务保留已交付文本，不把错误流升级为成功终态。
pub async fn generate_stream(
    client: &reqwest::Client,
    model: &Value,
    messages: &[ChatMessage],
    opts: &LlmStreamOptions<'_>,
) {
    let mut decoder = GeminiSseDecoder {
        visible_only: opts.visible_only,
        ..Default::default()
    };
    match stream_inner(client, model, messages, opts, &mut decoder).await {
        Ok(()) => {}
        Err(failure) => {
            let content = opts.visible_only.then(|| decoder.full_text());
            let usage = if opts.visible_only {
                decoder.usage()
            } else {
                None
            };
            (opts.callbacks.on_error)(failure.into_message(), content, usage);
        }
    }
}

async fn stream_inner(
    client: &reqwest::Client,
    model: &Value,
    messages: &[ChatMessage],
    opts: &LlmStreamOptions<'_>,
    decoder: &mut GeminiSseDecoder,
) -> Result<(), StreamFailure> {
    let base_url = model
        .get("baseUrl")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let model_name = model
        .get("modelName")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let url = generate_content_url(base_url, model_name, true);
    let body = build_request_body(model, messages, &opts.generate);

    let mut response = client
        .post(&url)
        .headers(build_request_headers(model).map_err(StreamFailure::Message)?)
        .body(serde_json::to_vec(&body).map_err(|error| StreamFailure::Message(error.to_string()))?)
        .send()
        .await
        .map_err(|error| StreamFailure::Message(error.to_string()))?;

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        return Err(StreamFailure::Message(format!(
            "Gemini API 调用失败 ({status}): {text}"
        )));
    }

    loop {
        if (opts.is_cancelled)() {
            return Err(StreamFailure::Cancelled);
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

        if let Some(error) = &decoder.fatal_error {
            return Err(StreamFailure::Message(error.clone()));
        }
    }

    for delivery in decoder.push(&[], true) {
        deliver(delivery, &opts.callbacks);
    }
    if let Some(error) = &decoder.fatal_error {
        return Err(StreamFailure::Message(error.clone()));
    }

    (opts.callbacks.on_done)(
        decoder.full_text(),
        decoder.usage(),
        decoder.finish_reason(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_only_rejects_all_baseline_malformed_payloads_after_stop() {
        for payload in [
            serde_json::json!({"error": {"message": "fixture"}}),
            serde_json::json!({"candidates": [{"content": {"parts": [{"text": 42}]}}]}),
            serde_json::json!({"candidates": [{"content": {"parts": [{"text": "隐藏", "thought": "true"}]}}]}),
            serde_json::json!({"candidates": "invalid"}),
            Value::Null,
        ] {
            let mut decoder = GeminiSseDecoder {
                visible_only: true,
                ..Default::default()
            };
            decoder.push(b"data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"prefix \"}]},\"finishReason\":\"STOP\"}]}\n", false);
            decoder.push(format!("data: {payload}\n").as_bytes(), false);
            assert!(decoder.fatal_error.is_some());
            assert_eq!(decoder.full_text(), "prefix ");
            assert!(decoder
                .push(
                    b"data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"late\"}]}}]}\n",
                    false
                )
                .is_empty());
        }
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
    fn finish_reason_maps_gemini_vocabulary_test() {
        assert_eq!(normalize_finish_reason(Some("STOP")), LlmFinishReason::Stop);
        assert_eq!(
            normalize_finish_reason(Some("MAX_TOKENS")),
            LlmFinishReason::Length
        );
        for blocked in ["SAFETY", "RECITATION", "BLOCKLIST", "PROHIBITED_CONTENT"] {
            assert_eq!(
                normalize_finish_reason(Some(blocked)),
                LlmFinishReason::ContentFilter,
                "{blocked}"
            );
        }
        assert_eq!(
            normalize_finish_reason(Some("OTHER")),
            LlmFinishReason::Unknown
        );
        assert_eq!(normalize_finish_reason(None), LlmFinishReason::Unknown);
    }

    #[test]
    fn contents_hoist_system_instruction_and_rename_assistant_test() {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: "第一".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: "问".to_string(),
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: "答".to_string(),
            },
            ChatMessage {
                role: "system".to_string(),
                content: "第二".to_string(),
            },
        ];
        let (contents, system) = to_gemini_contents(&messages);
        assert_eq!(system.as_deref(), Some("第二"), "后写的 system 覆盖前一个");
        assert_eq!(contents.len(), 2, "system 消息不得进入 contents");
        assert_eq!(contents[0]["role"], json!("user"));
        assert_eq!(contents[1]["role"], json!("model"));
    }

    #[test]
    fn request_body_shape_and_optional_fields_test() {
        let profile = json!({"modelName": "gemini-2.5-pro", "maxTokens": 2048});
        let body = build_request_body(
            &profile,
            &[ChatMessage {
                role: "user".to_string(),
                content: "你好".to_string(),
            }],
            &LlmGenerateOptions {
                temperature: None,
                max_tokens: None,
                output_token_parameter: None,
                response_format: Some(json!({"type": "json_object"})),
                reasoning: Some(ProviderReasoningDirective::GeminiThinkingBudget {
                    thinking_budget: 4096,
                }),
                conversation_id: None,
            },
        );
        assert_eq!(body["generationConfig"]["maxOutputTokens"], json!(2048));
        assert!(body["generationConfig"].get("temperature").is_none());
        assert_eq!(
            body["generationConfig"]["responseMimeType"],
            json!("application/json")
        );
        assert_eq!(
            body["generationConfig"]["thinkingConfig"],
            json!({"thinkingBudget": 4096})
        );
        assert!(body.get("systemInstruction").is_none());
    }

    #[test]
    fn openai_only_directives_never_leak_into_gemini_payload_test() {
        let profile = json!({"modelName": "gemini-2.5-pro", "maxTokens": 16});
        let body = build_request_body(
            &profile,
            &[],
            &LlmGenerateOptions {
                temperature: None,
                max_tokens: None,
                output_token_parameter: None,
                response_format: None,
                reasoning: Some(ProviderReasoningDirective::OpenAiReasoningEffort {
                    reasoning_effort: "low".to_string(),
                }),
                conversation_id: None,
            },
        );
        assert!(body["generationConfig"].get("thinkingConfig").is_none());
        assert!(body["generationConfig"].get("reasoning_effort").is_none());
    }

    #[test]
    fn urls_match_baseline_string_concatenation_test() {
        assert_eq!(
            generate_content_url("https://g.example/", "m", false),
            "https://g.example/v1beta/models/m:generateContent"
        );
        assert_eq!(
            generate_content_url("https://g.example", "m", true),
            "https://g.example/v1beta/models/m:streamGenerateContent?alt=sse"
        );
        // 基线只去掉**一个**尾部斜杠（`/\/$/` 非全局），双斜杠必须保留一条
        assert_eq!(
            generate_content_url("https://g.example//", "m", false),
            "https://g.example//v1beta/models/m:generateContent"
        );
    }

    #[test]
    fn sse_decoder_is_lenient_and_accumulates_evidence_test() {
        let mut decoder = GeminiSseDecoder::default();
        let mut chunks = Vec::new();
        chunks.extend(visible_chunks(decoder.push(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"甲\"}]}}]}\n".as_bytes(),
            false,
        )));
        // 损坏的 keepalive 与注释行一律忽略
        chunks.extend(visible_chunks(
            decoder.push(":keepalive\ndata: {broken\n".as_bytes(), false),
        ));
        chunks.extend(visible_chunks(decoder.push(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"乙\"}]},\"finishReason\":\"STOP\"}],".as_bytes(),
            false,
        )));
        chunks.extend(visible_chunks(decoder.push(
            "\"usageMetadata\":{\"promptTokenCount\":1,\"candidatesTokenCount\":2,\"totalTokenCount\":3}}\n".as_bytes(),
            false,
        )));
        assert_eq!(chunks.concat(), "甲乙");
        assert_eq!(decoder.full_text(), "甲乙");
        assert_eq!(decoder.finish_reason(), LlmFinishReason::Stop);
        assert_eq!(
            decoder.usage(),
            Some(TokenUsage {
                prompt_tokens: Some(1),
                completion_tokens: Some(2),
                total_tokens: Some(3)
            })
        );
    }

    #[test]
    fn sse_decoder_handles_crlf_and_trailing_buffer_test() {
        let mut decoder = GeminiSseDecoder::default();
        let mut chunks = visible_chunks(decoder.push(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"尾\"}]}}]}\r".as_bytes(),
            false,
        ));
        assert!(chunks.is_empty(), "无换行时不得提前派发");
        chunks = visible_chunks(decoder.push(&[], true));
        assert_eq!(chunks.concat(), "尾");
        assert_eq!(decoder.full_text(), "尾");
    }

    #[test]
    fn sse_decoder_null_finish_reason_becomes_unknown_test() {
        let mut decoder = GeminiSseDecoder::default();
        decoder.push(
            "data: {\"candidates\":[{\"finishReason\":null}]}\n".as_bytes(),
            false,
        );
        assert_eq!(decoder.finish_reason(), LlmFinishReason::Unknown);
        // 非字符串、非 null 的值同样归一为 unknown（对齐基线 `!== undefined` 判据）
        decoder.push(
            "data: {\"candidates\":[{\"finishReason\":7}]}\n".as_bytes(),
            false,
        );
        assert_eq!(decoder.finish_reason(), LlmFinishReason::Unknown);
    }

    /// 基线 `owner-visible-stream.test.ts`：`thought: true` 的
    /// part 原文直发推理；多 part 可见文本按序拼接（functionCall
    /// 等无 text part 贡献空串）；`usageMetadata` 真值即发射证据
    /// （`thoughtsTokenCount` → `reasoning_tokens`，
    /// `accounting: separately-billed`）。
    #[test]
    fn gemini_decoder_emits_thought_parts_and_usage_evidence_test() {
        let mut decoder = GeminiSseDecoder::default();
        let mut reasoning: Vec<String> = Vec::new();
        let mut chunks: Vec<String> = Vec::new();
        let mut evidence: Vec<ProviderUsageEvidence> = Vec::new();
        for delivery in decoder.push(
            concat!(
                "data: {\"candidates\":[{\"content\":{\"parts\":[",
                "{\"text\":\"hidden\",\"thought\":true},",
                "{\"text\":\" 甲\"},",
                "{\"functionCall\":{\"secret\":\"tool\"}},",
                "{\"text\":\"乙 \",\"thought\":false}]},\"finishReason\":\"STOP\"}],",
                "\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":20,",
                "\"thoughtsTokenCount\":7,\"totalTokenCount\":37}}\n"
            )
            .as_bytes(),
            false,
        ) {
            match delivery {
                SseDelivery::Chunk(chunk) => chunks.push(chunk),
                SseDelivery::Reasoning(text) => reasoning.push(text),
                SseDelivery::UsageEvidence(item) => evidence.push(item),
            }
        }
        assert_eq!(chunks.concat(), " 甲乙 ");
        assert_eq!(reasoning.concat(), "hidden");
        assert_eq!(
            evidence,
            vec![ProviderUsageEvidence {
                usage: TokenUsage {
                    prompt_tokens: Some(10),
                    completion_tokens: Some(20),
                    total_tokens: Some(37),
                },
                reasoning_tokens: Some(7),
                accounting: UsageAccounting::SeparatelyBilled,
                total_includes_reasoning: true,
                protocol: UsageProtocol::Gemini,
            }]
        );
        assert_eq!(decoder.full_text(), " 甲乙 ");
        assert_eq!(decoder.finish_reason(), LlmFinishReason::Stop);
    }

    /// 基线「keeps Gemini thought parts and in-band thoughts out of
    /// visible output」：带内 `</think>` 块从非 thought part 抽取，
    /// 与 thought part 原文合并为推理序列；半枚标签跨事件暂存。
    #[test]
    fn gemini_decoder_extracts_in_band_thoughts_test() {
        let mut decoder = GeminiSseDecoder::default();
        let mut reasoning: Vec<String> = Vec::new();
        let mut chunks: Vec<String> = Vec::new();
        for piece in [
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"推理\",\"thought\":true},{\"text\":\"前<th\"}]}}]}\n\n".as_bytes(),
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"ink>内</think>后\"}]},\"finishReason\":\"STOP\"}]}\n\n".as_bytes(),
        ] {
            for delivery in decoder.push(piece, false) {
                match delivery {
                    SseDelivery::Chunk(chunk) => chunks.push(chunk),
                    SseDelivery::Reasoning(text) => reasoning.push(text),
                    SseDelivery::UsageEvidence(_) => {}
                }
            }
        }
        assert_eq!(reasoning.concat(), "推理内");
        assert_eq!(chunks.concat(), "前\x3cthink\x3e内\x3c/think\x3e后");
        assert_eq!(decoder.full_text(), "前\x3cthink\x3e内\x3c/think\x3e后");
    }

    /// 基线 `if (parsed.usageMetadata)` 真值判据：`0` / `null`
    /// 为假不发射；空对象为真——发射全 `null` 用量的证据。
    #[test]
    fn gemini_decoder_usage_evidence_requires_truthy_metadata_test() {
        let mut decoder = GeminiSseDecoder::default();
        assert!(decoder
            .push(b"data: {\"usageMetadata\":0}\n", false)
            .is_empty());
        assert!(decoder
            .push(b"data: {\"usageMetadata\":null}\n", false)
            .is_empty());
        assert_eq!(decoder.usage(), None);
        let deliveries = decoder.push(b"data: {\"usageMetadata\":{}}\n", false);
        assert_eq!(
            deliveries,
            vec![SseDelivery::UsageEvidence(ProviderUsageEvidence {
                usage: TokenUsage {
                    prompt_tokens: None,
                    completion_tokens: None,
                    total_tokens: None,
                },
                reasoning_tokens: None,
                accounting: UsageAccounting::SeparatelyBilled,
                total_includes_reasoning: true,
                protocol: UsageProtocol::Gemini,
            })]
        );
        assert_eq!(
            decoder.usage(),
            Some(TokenUsage {
                prompt_tokens: None,
                completion_tokens: None,
                total_tokens: None,
            })
        );
    }

    #[test]
    fn sse_decoder_tolerates_multibyte_split_across_chunks_test() {
        // reqwest 的 chunk 边界可能把「甲」的 UTF-8 三字节拆开；
        // 解码器必须按字节缓冲，不得因此把流判为错误。
        let prefix = b"data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"";
        let suffix = b"\"}]}}]}\n";
        let jia = "甲".as_bytes();
        assert_eq!(jia.len(), 3, "UTF-8 三字节字符");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(prefix);
        bytes.extend_from_slice(jia);
        bytes.extend_from_slice(suffix);
        let split = prefix.len() + 1;

        let mut decoder = GeminiSseDecoder::default();
        assert!(
            decoder.push(&bytes[..split], false).is_empty(),
            "行未结束时不得派发"
        );
        let chunks = visible_chunks(decoder.push(&bytes[split..], false));
        assert_eq!(chunks.concat(), "甲");
        assert_eq!(decoder.full_text(), "甲");
    }
}
