//! 基线 main-generation-owner.ts 的 dispatchProvider 进程内适配器。
//! 仅消费主进程冻结的模型、语义任务和计划；不读取渲染层模型/预算。
//! 传输诊断发射仍属 B3-M4，本适配器不伪造 diagnostics 证据。

use std::future::Future;
use std::pin::Pin;
use std::sync::{LazyLock, Mutex};

use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::llm::chat::{
    ChatMessage, LlmFinishReason, LlmGenerateOptions, LlmStreamOptions, ProviderUsageEvidence,
    StreamCallbacks, TokenUsage,
};
use crate::llm::lease::LeaseModelProfile;
use crate::llm::reasoning::ProviderReasoningDirective;
use crate::services::generation_run_service::{
    DispatchOptions, GenerationDispatchResult, GenerationDispatcher, GenerationProviderUsage,
    VisibleEventKind, VisibleGenerationEvent,
};
use crate::services::generation_task::GenerationTask;
use crate::services::main_generation_plan::MainGenerationPlan;

/// 由 owner 生成的内部载荷；绝不作为 IPC 入参开放。
pub fn provider_request(
    model: &Value,
    task: &GenerationTask,
    plan: &MainGenerationPlan,
    conversation_id: &str,
) -> Value {
    json!({
        "model": model, "task": task, "conversationId": conversation_id,
        "plan": {
            "trustedUsage": plan.trusted_usage,
            "options": {
                "temperature": plan.options.temperature,
                "maxTokens": plan.options.max_tokens,
                "outputTokenParameter": plan.options.output_token_parameter,
                "responseFormat": plan.options.response_format,
                "reasoning": plan.options.reasoning,
            },
        },
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderRequest {
    model: Value,
    task: ProviderTask,
    plan: ProviderPlan,
    conversation_id: String,
}

#[derive(Deserialize)]
struct ProviderTask {
    messages: Vec<ChatMessage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderPlan {
    options: ProviderOptions,
    trusted_usage: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderOptions {
    temperature: Option<f64>,
    max_tokens: Option<u64>,
    output_token_parameter: Option<OutputTokenParameter>,
    response_format: Option<Value>,
    reasoning: Option<ProviderReasoningDirective>,
}

#[derive(Deserialize)]
enum OutputTokenParameter {
    #[serde(rename = "max_tokens")]
    Tokens,
    #[serde(rename = "max_completion_tokens")]
    CompletionTokens,
}

impl OutputTokenParameter {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Tokens => "max_tokens",
            Self::CompletionTokens => "max_completion_tokens",
        }
    }
}

static NETWORK_FAILURE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)fetch failed|failed to fetch|network|ECONNRESET|ETIMEDOUT|无法读取响应流|完成标记前结束")
        .expect("基线网络错误分类正则")
});

fn failure_code(error: &str) -> &'static str {
    if NETWORK_FAILURE.is_match(error) {
        "NETWORK_ERROR"
    } else {
        "GENERATION_PROVIDER_FAILED"
    }
}

fn usage_projection(
    evidence: Option<ProviderUsageEvidence>,
    trusted_usage: bool,
    protocol: &str,
) -> Option<GenerationProviderUsage> {
    evidence.map(|value| GenerationProviderUsage {
        prompt_tokens: value.usage.prompt_tokens,
        completion_tokens: value.usage.completion_tokens,
        total_tokens: value.usage.total_tokens,
        reasoning_tokens: value.reasoning_tokens,
        accounting: value.accounting.as_str().to_string(),
        total_includes_reasoning: value.total_includes_reasoning,
        trusted: trusted_usage && value.protocol.as_str() == protocol,
    })
}

/// owner 生命周期内保留分发入口，但每次分发前读取当前 Lorekeeper 代理配置。
/// HTTP 客户端不跨调用缓存，避免设置变更仅在重开项目后生效。
pub(crate) struct ConfiguredMainGenerationDispatcher {
    global_data_root: std::path::PathBuf,
}
impl ConfiguredMainGenerationDispatcher {
    pub(crate) fn new(global_data_root: std::path::PathBuf) -> Self {
        Self { global_data_root }
    }
}
impl GenerationDispatcher for ConfiguredMainGenerationDispatcher {
    fn dispatch<'a>(
        &'a self,
        request: &'a Value,
        options: DispatchOptions<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<GenerationDispatchResult, String>> + Send + 'a>> {
        Box::pin(async move {
            if options.signal.is_cancelled() {
                return Err("GENERATION_CANCELLED".to_string());
            }
            let config = crate::commands::read_global_config_at(
                &crate::app_paths::global_config_path_at(&self.global_data_root),
            );
            let proxy = crate::llm::chat::proxy_from_config(&config);
            let client = crate::llm::chat::build_client(proxy.as_ref())
                .map_err(|error| failure_code(&error))?;
            MainGenerationDispatcher::new(client)
                .dispatch(request, options)
                .await
        })
    }
}

/// HTTP 客户端由 owner 在 dispatch 前按当前全局代理配置构造。
/// 不访问 AppState 或 llm_streams；watch 取消会主动丢弃在途网络 future。
pub struct MainGenerationDispatcher {
    client: reqwest::Client,
}

impl MainGenerationDispatcher {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

impl GenerationDispatcher for MainGenerationDispatcher {
    fn dispatch<'a>(
        &'a self,
        request: &'a Value,
        options: DispatchOptions<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<GenerationDispatchResult, String>> + Send + 'a>> {
        Box::pin(async move {
            if options.signal.is_cancelled() {
                return Err("GENERATION_CANCELLED".to_string());
            }
            let model = request.get("model").ok_or("GENERATION_MODEL_MISSING")?;
            let profile = LeaseModelProfile::from_value(model);
            if !model.is_object()
                || profile.id.is_empty()
                || profile.provider.is_empty()
                || profile.base_url.is_empty()
                || profile.model_name.is_empty()
                || !["openai", "gemini"].contains(&profile.protocol)
            {
                return Err("GENERATION_MODEL_MISSING".to_string());
            }
            let request: ProviderRequest = serde_json::from_value(request.clone())
                .map_err(|_| "GENERATION_PROVIDER_REQUEST_INVALID".to_string())?;
            let evidence = Mutex::new(None);
            let finish = Mutex::new(None);
            let failure = Mutex::new(None);

            let on_chunk = |text: &str| {
                (options.on_visible)(VisibleGenerationEvent {
                    kind: VisibleEventKind::Delta,
                    text: text.to_string(),
                    event_id: None,
                })
            };
            let on_done = |text: String, _: Option<TokenUsage>, reason: LlmFinishReason| {
                (options.on_visible)(VisibleGenerationEvent {
                    kind: VisibleEventKind::Cumulative,
                    text,
                    event_id: None,
                });
                *finish.lock().expect("终态证据锁") = Some(reason);
            };
            let on_error = |error: String, _: Option<String>, _: Option<TokenUsage>| {
                *failure.lock().expect("失败证据锁") = Some(failure_code(&error));
            };
            let on_reasoning = |text: &str| {
                if let Some(callback) = options.on_reasoning {
                    callback(text.to_string());
                }
            };
            let on_usage = |value: &ProviderUsageEvidence| {
                *evidence.lock().expect("用量证据锁") = Some(value.clone());
            };
            let is_cancelled = || options.signal.is_cancelled();
            let stream_options = LlmStreamOptions {
                visible_only: true,
                generate: LlmGenerateOptions {
                    temperature: request.plan.options.temperature,
                    max_tokens: request.plan.options.max_tokens,
                    output_token_parameter: request
                        .plan
                        .options
                        .output_token_parameter
                        .as_ref()
                        .map(OutputTokenParameter::as_str),
                    response_format: request.plan.options.response_format,
                    reasoning: request.plan.options.reasoning,
                    conversation_id: Some(request.conversation_id),
                },
                callbacks: StreamCallbacks {
                    on_chunk: &on_chunk,
                    on_done: &on_done,
                    on_error: &on_error,
                    on_reasoning: Some(&on_reasoning),
                    on_usage_evidence: Some(&on_usage),
                },
                is_cancelled: &is_cancelled,
            };
            tokio::select! {
                biased;
                _ = options.signal.cancelled() => return Err("GENERATION_CANCELLED".to_string()),
                _ = crate::commands::generate_stream_core(
                    &self.client, &request.model, &request.task.messages, &stream_options,
                ) => {}
            }
            if let Some(code) = failure.into_inner().expect("失败证据锁") {
                return Err(code.to_string());
            }
            let reason = finish
                .into_inner()
                .expect("终态证据锁")
                .ok_or("GENERATION_PROVIDER_FAILED")?;
            Ok(GenerationDispatchResult {
                finish_reason: Some(reason.as_str().to_string()),
                usage: usage_projection(
                    evidence.into_inner().expect("用量证据锁"),
                    request.plan.trusted_usage,
                    request
                        .model
                        .get("protocol")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                ),
            })
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::llm::chat::{UsageAccounting, UsageProtocol};

    use crate::services::generation_run_service::GenerationCancelToken;
    use std::time::Duration;

    pub(crate) fn fixture(protocol: &str, url: &str) -> Value {
        json!({
            "conversationId": "frozen-lease",
            "model": {
                "id": "fixture", "provider": "custom", "protocol": protocol,
                "modelName": "fixture", "baseUrl": url, "apiKey": "synthetic-only",
            },
            "task": {"messages": [{"role": "user", "content": "测试请求"}]},
            "plan": {"trustedUsage": true, "options": {
                "maxTokens": 32, "outputTokenParameter": "max_completion_tokens",
                "reasoning": {"adapter": "openai-reasoning-effort", "reasoningEffort": "high"},
            }},
        })
    }

    pub(crate) fn server(
        body: &'static str,
        pause: Duration,
    ) -> (String, std::thread::JoinHandle<Value>) {
        server_with_ready(body, pause, None)
    }

    pub(crate) fn server_with_ready(
        body: &'static str,
        pause: Duration,
        ready: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> (String, std::thread::JoinHandle<Value>) {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < deadline, "分发器没有发起请求");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            // Windows 接受连接可能继承监听器的非阻塞标志；读取请求前显式恢复阻塞模式。
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                headers.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let length = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            let request = serde_json::from_slice(&bytes).unwrap();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            if let Some(ready) = ready {
                let _ = ready.send(());
            }

            std::thread::sleep(pause);
            // 取消测试会在服务器交付正文前关闭客户端连接。
            let _ = socket.write_all(body.as_bytes());
            request
        });
        (format!("http://{address}"), thread)
    }

    async fn run_fixture(
        protocol: &str,
        body: &'static str,
    ) -> (
        Result<GenerationDispatchResult, String>,
        Vec<VisibleGenerationEvent>,
        String,
        Value,
    ) {
        let (url, server) = server(body, Duration::ZERO);
        let dispatcher =
            MainGenerationDispatcher::new(reqwest::Client::builder().no_proxy().build().unwrap());
        let (_token, signal) = GenerationCancelToken::pair();
        let visible = Mutex::new(Vec::new());
        let reasoning = Mutex::new(String::new());
        let on_visible = |event| visible.lock().unwrap().push(event);
        let on_reasoning = |text: String| reasoning.lock().unwrap().push_str(&text);
        let request = fixture(protocol, &url);
        let result = dispatcher
            .dispatch(
                &request,
                DispatchOptions {
                    signal: &signal,
                    on_visible: &on_visible,
                    on_reasoning: Some(&on_reasoning),
                    on_diagnostics: None,
                },
            )
            .await;
        (
            result,
            visible.into_inner().unwrap(),
            reasoning.into_inner().unwrap(),
            server.join().unwrap(),
        )
    }

    #[tokio::test]
    async fn openai_dispatch_keeps_reasoning_private_and_projects_terminal_cumulative() {
        let (result, visible, reasoning, request) = run_fixture("openai", concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"协议秘密\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" 甲<th\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"ink>带内秘密</think>乙 \\n\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":20,\"total_tokens\":30,\"completion_tokens_details\":{\"reasoning_tokens\":7}}}\n\n",
            "data: [DONE]\n\n"
        )).await;
        let result = result.unwrap();
        assert_eq!(result.finish_reason.as_deref(), Some("stop"));
        assert!(result.usage.as_ref().unwrap().trusted);
        assert_eq!(result.usage.unwrap().reasoning_tokens, Some(7));
        assert_eq!(reasoning, "协议秘密带内秘密");
        assert_eq!(
            visible
                .iter()
                .filter(|event| event.kind == VisibleEventKind::Delta)
                .map(|event| event.text.as_str())
                .collect::<String>(),
            " 甲乙 \n"
        );
        assert_eq!(visible.last().unwrap().kind, VisibleEventKind::Cumulative);
        assert_eq!(visible.last().unwrap().text, " 甲乙 \n");
        assert_eq!(request["max_completion_tokens"], 32);
        assert_eq!(request["reasoning_effort"], "high");
        assert!(request.get("max_tokens").is_none());
    }

    #[tokio::test]
    async fn gemini_dispatch_filters_inband_thoughts_and_preserves_whitespace() {
        let (result, visible, reasoning, _) = run_fixture("gemini", concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"协议秘密\",\"thought\":true},{\"text\":\" 前<th\"}]}}]}\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"ink>带内秘密</think>后 \"}]},\"finishReason\":\"STOP\"}],",
            "\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":20,\"thoughtsTokenCount\":7,\"totalTokenCount\":37}}\n"
        )).await;
        assert_eq!(reasoning, "协议秘密带内秘密");
        assert_eq!(visible.last().unwrap().text, " 前后 ");
        let usage = result.unwrap().usage.unwrap();
        assert!(usage.trusted);
        assert_eq!(usage.accounting, "separately-billed");
        assert_eq!(usage.reasoning_tokens, Some(7));
    }

    #[tokio::test]
    async fn malformed_gemini_after_stop_is_failure_not_success() {
        let (result, visible, _, _) = run_fixture("gemini", concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"正文 \"}]},\"finishReason\":\"STOP\"}]}\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":42}]}}]}\n"
        )).await;
        assert_eq!(result.unwrap_err(), "GENERATION_PROVIDER_FAILED");
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].text, "正文 ");
        assert_eq!(visible[0].kind, VisibleEventKind::Delta);
    }

    #[tokio::test]
    async fn incomplete_openai_is_network_failure_with_only_visible_prefix() {
        let (result, visible, reasoning, _) = run_fixture("openai",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"秘密\",\"content\":\"正文 \"}}]}\n\n"
        ).await;
        assert_eq!(result.unwrap_err(), "NETWORK_ERROR");
        assert_eq!(reasoning, "秘密");
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].text, "正文 ");
    }

    #[tokio::test]
    async fn watch_cancellation_interrupts_waiting_response_without_ipc_registry() {
        let (ready, accepted) = tokio::sync::oneshot::channel();
        let (url, server) =
            server_with_ready("data: [DONE]\n\n", Duration::from_millis(400), Some(ready));
        let dispatcher =
            MainGenerationDispatcher::new(reqwest::Client::builder().no_proxy().build().unwrap());
        let (token, signal) = GenerationCancelToken::pair();
        let cancel = async {
            accepted.await.expect("服务器已收到冻结请求");
            let cancelled_at = std::time::Instant::now();
            token.cancel();

            cancelled_at
        };
        let visible = |_| panic!("取消后不可交付正文");
        let request = fixture("openai", &url);
        let (result, cancelled_at) = tokio::join!(
            dispatcher.dispatch(
                &request,
                DispatchOptions {
                    signal: &signal,
                    on_visible: &visible,
                    on_reasoning: None,
                    on_diagnostics: None,
                }
            ),
            cancel
        );
        assert_eq!(result.unwrap_err(), "GENERATION_CANCELLED");
        assert!(
            cancelled_at.elapsed() < Duration::from_millis(300),
            "取消不得等待服务器下一块"
        );
        server.join().unwrap();
    }

    #[tokio::test]
    async fn invalid_model_and_pre_cancelled_requests_are_fail_closed() {
        let dispatcher = MainGenerationDispatcher::new(reqwest::Client::new());
        let (token, signal) = GenerationCancelToken::pair();
        let visible = |_| panic!("无效请求不得交付正文");
        let options = || DispatchOptions {
            signal: &signal,
            on_visible: &visible,
            on_reasoning: None,
            on_diagnostics: None,
        };
        assert_eq!(
            dispatcher
                .dispatch(&json!({"model": {}}), options())
                .await
                .unwrap_err(),
            "GENERATION_MODEL_MISSING"
        );
        token.cancel();
        assert_eq!(
            dispatcher
                .dispatch(&json!({}), options())
                .await
                .unwrap_err(),
            "GENERATION_CANCELLED"
        );
    }

    #[test]
    fn usage_requires_both_frozen_plan_trust_and_matching_protocol() {
        let evidence = ProviderUsageEvidence {
            usage: TokenUsage {
                prompt_tokens: Some(10),
                completion_tokens: Some(20),
                total_tokens: Some(30),
            },
            reasoning_tokens: Some(7),
            accounting: UsageAccounting::IncludedInCompletion,
            total_includes_reasoning: true,
            protocol: UsageProtocol::OpenAi,
        };
        assert!(
            usage_projection(Some(evidence.clone()), true, "openai")
                .unwrap()
                .trusted
        );
        assert!(
            !usage_projection(Some(evidence.clone()), false, "openai")
                .unwrap()
                .trusted
        );
        assert!(
            !usage_projection(Some(evidence), true, "gemini")
                .unwrap()
                .trusted
        );
        assert!(usage_projection(None, true, "openai").is_none());
    }

    #[test]
    fn error_classification_and_output_key_validation_follow_contract() {
        for error in [
            "fetch failed",
            "无法读取响应流",
            "响应流在完成标记前结束",
            "ECONNRESET",
        ] {
            assert_eq!(failure_code(error), "NETWORK_ERROR");
        }
        assert_eq!(
            failure_code("API 调用失败 (403)"),
            "GENERATION_PROVIDER_FAILED"
        );
        assert!(
            serde_json::from_value::<OutputTokenParameter>(json!("max_completion_tokens")).is_ok()
        );
        assert!(serde_json::from_value::<OutputTokenParameter>(json!("arbitrary-key")).is_err());
    }
}
