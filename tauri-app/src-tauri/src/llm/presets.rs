//! 服务商内置预设目录 —— 迁移自 `src/shared/provider-presets.ts`。
//!
//! 该模块是**能力证据的唯一事实源**：只有内置目录里被验证过的端点 + 精确模型 slug
//! 才能产生「已验证能力 / 已验证推理映射」。用户填写的 `capabilities` 与输出上限属于
//! 操作策略，永远不能升级为协议证据（`resolve_model_profile_capabilities` 的守卫）。

use serde::Serialize;

use crate::llm::reasoning::{ReasoningAdapter, ReasoningEffort};

/// 单模型的已验证能力元数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCapabilities {
    /// `None` 表示端点未声明上下文窗口（对齐基线的显式 `null`）。
    pub context_window_tokens: Option<u64>,
    pub max_output_tokens: u64,
    pub reasoning: bool,
    pub structured_output: bool,
    pub usage: bool,
}

/// 已验证推理映射中的 provider 取值（可为字符串或数字）。
#[derive(Debug, Clone, PartialEq)]
pub enum PresetValue {
    Text(String),
    Number(i64),
}

impl PresetValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            PresetValue::Text(text) => Some(text.as_str()),
            PresetValue::Number(_) => None,
        }
    }

    pub fn as_number(&self) -> Option<i64> {
        match self {
            PresetValue::Number(value) => Some(*value),
            PresetValue::Text(_) => None,
        }
    }
}

/// 经官方文档验证的 provider 请求映射。
#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedReasoningMapping {
    pub adapter: ReasoningAdapter,
    pub supported_efforts: Vec<ReasoningEffort>,
    pub provider_values: Vec<(ReasoningEffort, PresetValue)>,
    /// 文档化的兼容别名：请求强度 → provider 实际强度。
    pub request_aliases: Vec<(ReasoningEffort, ReasoningEffort)>,
}

impl VerifiedReasoningMapping {
    pub fn provider_value(&self, effort: ReasoningEffort) -> Option<&PresetValue> {
        self.provider_values
            .iter()
            .find(|(key, _)| *key == effort)
            .map(|(_, value)| value)
    }

    pub fn request_alias(&self, requested: ReasoningEffort) -> Option<ReasoningEffort> {
        self.request_aliases
            .iter()
            .find(|(key, _)| *key == requested)
            .map(|(_, alias)| *alias)
    }
}

/// 单个生成模型预设。
#[derive(Debug, Clone, PartialEq)]
pub struct ModelPreset {
    pub name: String,
    pub capabilities: Option<ModelCapabilities>,
    pub reasoning_mapping: Option<VerifiedReasoningMapping>,
    pub max_tokens: u64,
}

/// 基线 `REASONING_EFFORTS` 的长度上限
/// （`src/shared/reasoning-types.ts`：`off` / `low` / `medium` /
/// `high` / `xhigh` / `max`）。
const REASONING_EFFORTS_LIMIT: usize = 6;

/// 基线 `Number.isSafeInteger`（±2^53-1）。
fn is_safe_json_integer(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && value.abs() <= MAX_SAFE_INTEGER as f64
}

/// 基线 `isReasoningMapping`（`src/shared/reasoning-types.ts:55-99`）——
/// 手动 `reasoningMapping` 的完整验证器（含 5 值 adapter 白名单 /
/// `supportedEfforts` 非空无重复且 ≤ 基线努力枚举长度 / `providerValues`
/// 键 ⊆ `supportedEfforts` / 各 adapter wire 格式 / `requestAliases`
/// 约束 / 仅允许 4 个键）。验证失败返回 `None`。
fn is_reasoning_mapping(value: &serde_json::Value) -> Option<VerifiedReasoningMapping> {
    let mapping = value.as_object()?;
    // 基线：仅允许 4 个键。
    if mapping.keys().any(|key| {
        !matches!(
            key.as_str(),
            "adapter" | "supportedEfforts" | "providerValues" | "requestAliases"
        )
    }) {
        return None;
    }
    let adapter = match mapping.get("adapter")?.as_str()? {
        "openai-reasoning-effort" => ReasoningAdapter::OpenAiReasoningEffort,
        "openai-thinking-budget" => ReasoningAdapter::OpenAiThinkingBudget,
        "deepseek-v4-thinking" => ReasoningAdapter::DeepSeekV4Thinking,
        "siliconflow-v4-thinking" => ReasoningAdapter::SiliconFlowV4Thinking,
        "gemini-thinking-budget" => ReasoningAdapter::GeminiThinkingBudget,
        _ => return None,
    };
    // supportedEfforts：非空、无重复、长度不超过基线努力枚举。
    let supported_efforts: Vec<ReasoningEffort> = mapping
        .get("supportedEfforts")?
        .as_array()?
        .iter()
        .map(|value| value.as_str().and_then(ReasoningEffort::parse))
        .collect::<Option<Vec<_>>>()?;
    if supported_efforts.is_empty() || supported_efforts.len() > REASONING_EFFORTS_LIMIT {
        return None;
    }
    if supported_efforts
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len()
        != supported_efforts.len()
    {
        return None;
    }
    // providerValues：必须是对象（基线拒绝数组）。
    let provider_values = mapping.get("providerValues")?.as_object()?;
    // 基线：providerValues 的键 ⊆ supportedEfforts。
    if provider_values.keys().any(|key| {
        !supported_efforts
            .iter()
            .any(|effort| effort.as_str() == key.as_str())
    }) {
        return None;
    }
    let mut wire_values = Vec::with_capacity(supported_efforts.len());
    for effort in &supported_efforts {
        // 基线：每个 supportedEfforts 条目都必须有 provider 取值
        // （缺失 → `undefined` → wire 格式检查失败）。
        let wire = provider_values.get(effort.as_str())?;
        match adapter {
            ReasoningAdapter::OpenAiReasoningEffort => {
                let text = wire.as_str()?;
                // 基线 `/^[A-Za-z][A-Za-z0-9_-]{0,63}$/`。
                let mut characters = text.chars();
                if !characters
                    .next()
                    .is_some_and(|first| first.is_ascii_alphabetic())
                    || !characters.all(|character| {
                        character.is_ascii_alphanumeric() || character == '_' || character == '-'
                    })
                    || text.len() > 64
                {
                    return None;
                }
                // 基线：`off` 的取值必须是文档化的停用别名。
                if *effort == ReasoningEffort::Off && !matches!(text, "off" | "none" | "disabled") {
                    return None;
                }
                wire_values.push((*effort, PresetValue::Text(text.to_string())));
            }
            ReasoningAdapter::GeminiThinkingBudget | ReasoningAdapter::OpenAiThinkingBudget => {
                let number = wire.as_f64()?;
                // 基线：安全整数且 ≥ 0；`off` 必须为 0。
                if !is_safe_json_integer(number)
                    || number < 0.0
                    || (*effort == ReasoningEffort::Off && number != 0.0)
                {
                    return None;
                }
                // 基线：openai-thinking-budget 的非 `off` 取值不得为 0
                // （0 是「禁用」哨兵，不是合法预算）。
                if adapter == ReasoningAdapter::OpenAiThinkingBudget
                    && *effort != ReasoningEffort::Off
                    && number == 0.0
                {
                    return None;
                }
                wire_values.push((*effort, PresetValue::Number(number as i64)));
            }
            ReasoningAdapter::DeepSeekV4Thinking => {
                let text = wire.as_str()?;
                // 基线：`off` 必须为 `disabled`；其余为文档化取值。
                if (*effort == ReasoningEffort::Off && text != "disabled")
                    || (*effort != ReasoningEffort::Off
                        && !matches!(text, "enabled" | "low" | "high" | "max"))
                {
                    return None;
                }
                wire_values.push((*effort, PresetValue::Text(text.to_string())));
            }
            // 基线 siliconflow-v4-thinking：仅 `high` / `max`，且
            // provider 取值与强度同名。
            ReasoningAdapter::SiliconFlowV4Thinking => {
                if !matches!(*effort, ReasoningEffort::High | ReasoningEffort::Max)
                    || wire.as_str() != Some(effort.as_str())
                {
                    return None;
                }
                wire_values.push((*effort, PresetValue::Text(effort.as_str().to_string())));
            }
        }
    }
    // requestAliases：基线 `undefined` 缺省；出现时必须为对象
    // （`null` / 数组都拒绝）。
    let request_aliases = match mapping.get("requestAliases") {
        None => Vec::new(),
        Some(aliases) => {
            let aliases = aliases.as_object()?;
            let mut pairs = Vec::with_capacity(aliases.len());
            for (request, effective) in aliases {
                let request = ReasoningEffort::parse(request)?;
                let effective = supported_efforts
                    .iter()
                    .copied()
                    .find(|effort| Some(effort.as_str()) == effective.as_str())?;
                // 基线：`off` 的别名只能是 `off`。
                if request == ReasoningEffort::Off && effective != ReasoningEffort::Off {
                    return None;
                }
                pairs.push((request, effective));
            }
            pairs
        }
    };
    Some(VerifiedReasoningMapping {
        adapter,
        supported_efforts,
        provider_values: wire_values,
        request_aliases,
    })
}

/// 单个服务商预设。
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderPreset {
    pub provider: String,
    pub display_name: Option<String>,
    pub base_url: String,
    pub protocol: String,
    pub models: Vec<ModelPreset>,
    pub embedding_models: Vec<String>,
    pub embedding_model_capabilities: Vec<(String, ModelCapabilities)>,
}

/// 解析已验证能力所需的最小档案字段（缺失或类型不符一律视为「无证据」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCapabilityProfile<'a> {
    pub provider: Option<&'a str>,
    pub protocol: Option<&'a str>,
    pub base_url: Option<&'a str>,
    pub model_name: Option<&'a str>,
    /// 原始 JSON 手动推理映射（基线 `ModelProfile.reasoningMapping`）。
    ///
    /// 手动映射是指令机器的**一等输入**（基线
    /// `resolveModelProfileReasoningMapping` 的手动优先分支）：
    /// [`is_reasoning_mapping`] 验证通过即被消费，**不再回落**
    /// 内置目录。`None` = 档案未携带手动映射。
    pub reasoning_mapping: Option<&'a serde_json::Value>,
}

impl<'a> ModelCapabilityProfile<'a> {
    /// 从模型 JSON 条目读取（非字符串字段按缺失处理，对齐基线的 `typeof x !== 'string'`）。
    ///
    /// 生产与测试共用：`llm/openai.rs` 的推理载荷门禁经此
    /// 构造档案并解析已验证推理映射（基线
    /// `resolveModelProfileReasoningMapping` 同样是「手动映射
    /// 优先 + 内置目录」）；租约路径由
    /// [`crate::llm::lease::LeaseModelProfile::capability_profile`]
    /// 直接从冻结快照构造，避免再解析一次 JSON。
    pub fn from_value(model: &'a serde_json::Value) -> Self {
        ModelCapabilityProfile {
            provider: model.get("provider").and_then(|value| value.as_str()),
            protocol: model.get("protocol").and_then(|value| value.as_str()),
            base_url: model.get("baseUrl").and_then(|value| value.as_str()),
            model_name: model.get("modelName").and_then(|value| value.as_str()),
            reasoning_mapping: model.get("reasoningMapping"),
        }
    }
}

fn caps(
    context_window_tokens: Option<u64>,
    max_output_tokens: u64,
    reasoning: bool,
    structured_output: bool,
    usage: bool,
) -> ModelCapabilities {
    ModelCapabilities {
        context_window_tokens,
        max_output_tokens,
        reasoning,
        structured_output,
        usage,
    }
}

/// 无能力元数据的模型条目（仅旧的 `maxTokens`）。
fn plain(name: &str, max_tokens: u64) -> ModelPreset {
    ModelPreset {
        name: name.to_string(),
        capabilities: None,
        reasoning_mapping: None,
        max_tokens,
    }
}

fn detailed(
    name: &str,
    max_tokens: u64,
    capabilities: ModelCapabilities,
    reasoning_mapping: VerifiedReasoningMapping,
) -> ModelPreset {
    ModelPreset {
        name: name.to_string(),
        capabilities: Some(capabilities),
        reasoning_mapping: Some(reasoning_mapping),
        max_tokens,
    }
}

fn values(pairs: &[(ReasoningEffort, PresetValue)]) -> Vec<(ReasoningEffort, PresetValue)> {
    pairs.to_vec()
}

fn text(value: &str) -> PresetValue {
    PresetValue::Text(value.to_string())
}

/// 内置服务商目录（对齐基线 `createProviderCatalog()`）。
///
/// 每次调用返回新对象，调用方可安全派生 UI 状态而不污染全局预设。
pub fn builtin_presets() -> Vec<ProviderPreset> {
    vec![
        ProviderPreset {
            provider: "openai".to_string(),
            display_name: Some("OpenAI".to_string()),
            base_url: "https://api.openai.com".to_string(),
            protocol: "openai".to_string(),
            models: vec![
                plain("gpt-4o", 16_384),
                plain("gpt-4o-mini", 16_384),
                plain("gpt-4-turbo", 4_096),
                plain("gpt-3.5-turbo", 4_096),
            ],
            embedding_models: vec![
                "text-embedding-3-small".to_string(),
                "text-embedding-3-large".to_string(),
                "text-embedding-ada-002".to_string(),
            ],
            embedding_model_capabilities: vec![],
        },
        ProviderPreset {
            provider: "xai".to_string(),
            display_name: Some("xAI(Grok)".to_string()),
            base_url: "https://api.x.ai/v1".to_string(),
            protocol: "openai".to_string(),
            models: vec![detailed(
                "grok-4.5",
                8_192,
                caps(Some(500_000), 8_192, true, true, true),
                // https://docs.x.ai/developers/model-capabilities/text/reasoning
                VerifiedReasoningMapping {
                    adapter: ReasoningAdapter::OpenAiReasoningEffort,
                    supported_efforts: vec![
                        ReasoningEffort::Low,
                        ReasoningEffort::Medium,
                        ReasoningEffort::High,
                    ],
                    provider_values: values(&[
                        (ReasoningEffort::Low, text("low")),
                        (ReasoningEffort::Medium, text("medium")),
                        (ReasoningEffort::High, text("high")),
                    ]),
                    request_aliases: vec![],
                },
            )],
            embedding_models: vec![],
            embedding_model_capabilities: vec![],
        },
        ProviderPreset {
            provider: "siliconflow".to_string(),
            display_name: Some("SiliconFlow".to_string()),
            base_url: "https://api.siliconflow.cn/v1".to_string(),
            protocol: "openai".to_string(),
            models: vec![
                // JSON mode: https://docs.siliconflow.cn/docs/api/chat-completions-post
                ModelPreset {
                    name: "deepseek-ai/DeepSeek-V4-Flash".to_string(),
                    // 基线无 `capabilities` 字段（预算能力证据由
                    // `llm/provider_presets.rs` 的 budgetCapabilities 承载）。
                    capabilities: None,
                    reasoning_mapping: Some(siliconflow_v4_thinking_mapping()),
                    // 保留既有运营默认值。单独验证的容量是规划
                    // 证据，不扩大用户设置。
                    max_tokens: 16_384,
                },
                ModelPreset {
                    name: "deepseek-ai/DeepSeek-V4-Pro".to_string(),
                    capabilities: None,
                    reasoning_mapping: Some(siliconflow_v4_thinking_mapping()),
                    max_tokens: 16_384,
                },
                // Application-selected budget, not a provider-defined
                // medium effort.
                // https://docs.siliconflow.cn/docs/userguide/capabilities/reasoning
                ModelPreset {
                    name: "Qwen/Qwen3.8-27B".to_string(),
                    capabilities: None,
                    reasoning_mapping: Some(openai_thinking_budget_mapping()),
                    max_tokens: 16_384,
                },
            ],
            embedding_models: vec!["BAAI/bge-m3".to_string()],
            embedding_model_capabilities: vec![(
                "BAAI/bge-m3".to_string(),
                caps(Some(8_192), 0, false, false, true),
            )],
        },
        ProviderPreset {
            provider: "novelai".to_string(),
            display_name: Some("NovelAI".to_string()),
            base_url: "https://text.novelai.net/oa".to_string(),
            protocol: "openai".to_string(),
            models: vec![],
            embedding_models: vec![],
            embedding_model_capabilities: vec![],
        },
        ProviderPreset {
            provider: "deepseek".to_string(),
            display_name: Some("DeepSeek".to_string()),
            base_url: "https://api.deepseek.com".to_string(),
            protocol: "openai".to_string(),
            models: vec![
                detailed(
                    "deepseek-v4-flash",
                    384_000,
                    caps(Some(1_000_000), 384_000, true, true, true),
                    deepseek_thinking_mapping(),
                ),
                detailed(
                    "deepseek-v4-pro",
                    384_000,
                    caps(Some(1_000_000), 384_000, true, true, true),
                    deepseek_thinking_mapping(),
                ),
            ],
            embedding_models: vec![],
            embedding_model_capabilities: vec![],
        },
        ProviderPreset {
            // 智谱 BigModel —— OpenAI 兼容协议，API 路径为 /v4
            provider: "bigmodel".to_string(),
            display_name: Some("BigModel（智谱）".to_string()),
            base_url: "https://open.bigmodel.cn/api/paas/v4".to_string(),
            protocol: "openai".to_string(),
            models: vec![
                plain("glm-4.5", 65_536),
                plain("glm-4.5-air", 65_536),
                plain("glm-4.6", 65_536),
                plain("glm-4.7", 65_536),
                plain("glm-4.7-flashx", 65_536),
                plain("glm-5-turbo", 65_536),
                plain("glm-5", 65_536),
            ],
            embedding_models: vec!["embedding-3".to_string()],
            embedding_model_capabilities: vec![],
        },
        ProviderPreset {
            provider: "gemini".to_string(),
            display_name: Some("Google Gemini".to_string()),
            base_url: "https://generativelanguage.googleapis.com".to_string(),
            protocol: "gemini".to_string(),
            models: vec![
                detailed(
                    "gemini-2.5-flash-lite",
                    65_536,
                    caps(Some(1_048_576), 65_536, true, true, true),
                    // https://ai.google.dev/gemini-api/docs/generate-content/thinking
                    VerifiedReasoningMapping {
                        adapter: ReasoningAdapter::GeminiThinkingBudget,
                        supported_efforts: vec![
                            ReasoningEffort::Off,
                            ReasoningEffort::Low,
                            ReasoningEffort::Medium,
                            ReasoningEffort::High,
                        ],
                        provider_values: values(&[
                            (ReasoningEffort::Off, PresetValue::Number(0)),
                            (ReasoningEffort::Low, PresetValue::Number(1_024)),
                            (ReasoningEffort::Medium, PresetValue::Number(8_192)),
                            (ReasoningEffort::High, PresetValue::Number(24_576)),
                        ]),
                        request_aliases: vec![],
                    },
                ),
                plain("gemini-3.1-pro-preview", 65_536),
                plain("gemini-3-flash-preview", 65_536),
            ],
            embedding_models: vec!["text-embedding-004".to_string()],
            embedding_model_capabilities: vec![],
        },
        ProviderPreset {
            provider: "ollama".to_string(),
            display_name: Some("Ollama（本地）".to_string()),
            base_url: "http://localhost:11434/v1".to_string(),
            protocol: "openai".to_string(),
            models: vec![
                plain("qwen3-14b-abliterated-novel-q4", 8_192),
                plain("llama3.3", 4_096),
                plain("llama3.2", 4_096),
                plain("qwen2.5", 8_192),
                plain("qwen2.5-coder", 8_192),
                plain("mistral", 4_096),
                plain("phi4", 4_096),
                plain("gemma3", 8_192),
            ],
            embedding_models: vec![
                "nomic-embed-text".to_string(),
                "mxbai-embed-large".to_string(),
                "bge-m3".to_string(),
            ],
            embedding_model_capabilities: vec![],
        },
        ProviderPreset {
            provider: "custom".to_string(),
            display_name: Some("自定义".to_string()),
            base_url: String::new(),
            protocol: "openai".to_string(),
            models: vec![],
            embedding_models: vec![],
            embedding_model_capabilities: vec![],
        },
    ]
}

/// 深寻思考映射（flash / pro 共用）。
fn deepseek_thinking_mapping() -> VerifiedReasoningMapping {
    // https://api-docs.deepseek.com/guides/thinking_mode/
    VerifiedReasoningMapping {
        adapter: ReasoningAdapter::DeepSeekV4Thinking,
        supported_efforts: vec![
            ReasoningEffort::Off,
            ReasoningEffort::Low,
            ReasoningEffort::High,
            ReasoningEffort::Max,
        ],
        provider_values: values(&[
            (ReasoningEffort::Off, text("disabled")),
            (ReasoningEffort::Low, text("low")),
            (ReasoningEffort::High, text("high")),
            (ReasoningEffort::Max, text("max")),
        ]),
        request_aliases: vec![(ReasoningEffort::Medium, ReasoningEffort::High)],
    }
}

/// SiliconFlow DeepSeek-V4 思考映射（Flash / Pro 共用）。
fn siliconflow_v4_thinking_mapping() -> VerifiedReasoningMapping {
    // https://docs.siliconflow.cn/docs/api/chat-completions-post
    VerifiedReasoningMapping {
        adapter: ReasoningAdapter::SiliconFlowV4Thinking,
        supported_efforts: vec![ReasoningEffort::High, ReasoningEffort::Max],
        provider_values: values(&[
            (ReasoningEffort::High, text("high")),
            (ReasoningEffort::Max, text("max")),
        ]),
        request_aliases: vec![],
    }
}

/// Qwen/Qwen3.8-27B 应用预算映射（显式 off / medium）。
fn openai_thinking_budget_mapping() -> VerifiedReasoningMapping {
    // https://docs.siliconflow.cn/docs/userguide/capabilities/reasoning
    VerifiedReasoningMapping {
        adapter: ReasoningAdapter::OpenAiThinkingBudget,
        supported_efforts: vec![ReasoningEffort::Off, ReasoningEffort::Medium],
        provider_values: values(&[
            (ReasoningEffort::Off, PresetValue::Number(0)),
            (ReasoningEffort::Medium, PresetValue::Number(16_384)),
        ]),
        request_aliases: vec![],
    }
}

/// 端点规范化（对齐基线 `normalizedOfficialBaseUrl`）：
/// 仅接受 http/https、无凭据、无查询、无片段；尾斜杠归一。
fn normalized_official_base_url(value: Option<&str>) -> Option<String> {
    let value = value?;
    let mut endpoint = url::Url::parse(value.trim()).ok()?;
    let scheme = endpoint.scheme();
    if scheme != "https" && scheme != "http" {
        return None;
    }
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        return None;
    }
    if endpoint.query().is_some() || endpoint.fragment().is_some() {
        return None;
    }
    let trimmed_path = endpoint.path().trim_end_matches('/').to_string();
    let path = if trimmed_path.is_empty() {
        "/".to_string()
    } else {
        trimmed_path
    };
    endpoint.set_path(&path);
    let text = endpoint.to_string();
    Some(text.strip_suffix('/').unwrap_or(&text).to_string())
}

/// 能力校验（对齐基线 `validatedCapabilities`）。
fn validated_capabilities(value: Option<&ModelCapabilities>) -> Option<ModelCapabilities> {
    let value = value?;
    let valid_context = match value.context_window_tokens {
        None => true,
        Some(tokens) => tokens > 0 && tokens <= MAX_SAFE_INTEGER,
    };
    if !valid_context || value.max_output_tokens == 0 || value.max_output_tokens > MAX_SAFE_INTEGER
    {
        return None;
    }
    Some(value.clone())
}

/// JS `Number.MAX_SAFE_INTEGER` —— 与 `Number.isSafeInteger` 语义对齐。
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// 查找内置预设：要求 provider 命中、协议一致、**规范化端点一致**。
fn matching_preset<'a>(
    presets: &'a [ProviderPreset],
    profile: &ModelCapabilityProfile<'_>,
) -> Option<&'a ProviderPreset> {
    let provider = profile.provider?;
    let protocol = profile.protocol?;
    profile.model_name?;
    let preset = presets
        .iter()
        .find(|candidate| candidate.provider == provider)?;
    if preset.protocol != protocol {
        return None;
    }
    if normalized_official_base_url(profile.base_url)
        != normalized_official_base_url(Some(preset.base_url.as_str()))
    {
        return None;
    }
    Some(preset)
}

/// 解析模型档案的**已验证**能力。
///
/// 返回 `None` 表示「无协议证据」：用户存储的能力与输出上限属于运行策略，
/// 不得覆盖该结论。
pub fn resolve_model_profile_capabilities(
    profile: &ModelCapabilityProfile<'_>,
) -> Option<ModelCapabilities> {
    let presets = builtin_presets();
    let preset = matching_preset(&presets, profile)?;
    let model_name = profile.model_name?.trim();
    let model = preset
        .models
        .iter()
        .find(|candidate| candidate.name == model_name)?;
    validated_capabilities(model.capabilities.as_ref())
}

/// 解析模型档案的**已验证**推理映射。
///
/// 手动 `reasoningMapping`（档案字段）优先：验证通过
/// 即返回，不回落内置目录；否则查内置目录（要求端点
/// 与精确模型 slug 命中）。
pub fn resolve_model_profile_reasoning_mapping(
    profile: &ModelCapabilityProfile<'_>,
) -> Option<VerifiedReasoningMapping> {
    // 基线手动优先分支（provider-presets.ts:594-601）：档案携带
    // 手动 `reasoningMapping` 时，验证通过即返回，**不回落**内置
    // 目录；验证失败或协议一致性检查不通过则整体无证据。
    if let Some(raw_mapping) = profile.reasoning_mapping {
        let mapping = is_reasoning_mapping(raw_mapping)?;
        let gemini_consistent = (profile.protocol == Some("gemini"))
            == (mapping.adapter == ReasoningAdapter::GeminiThinkingBudget);
        return gemini_consistent.then_some(mapping);
    }
    let presets = builtin_presets();
    let preset = matching_preset(&presets, profile)?;
    let model_name = profile.model_name?.trim();
    preset
        .models
        .iter()
        .find(|candidate| candidate.name == model_name)
        .and_then(|model| model.reasoning_mapping.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::reasoning::resolve_reasoning_policy;

    fn profile<'a>(
        provider: &'a str,
        protocol: &'a str,
        base_url: &'a str,
        model_name: &'a str,
    ) -> ModelCapabilityProfile<'a> {
        ModelCapabilityProfile {
            provider: Some(provider),
            protocol: Some(protocol),
            base_url: Some(base_url),
            model_name: Some(model_name),
            reasoning_mapping: None,
        }
    }

    #[test]
    fn catalog_covers_every_baseline_provider_test() {
        let providers: Vec<String> = builtin_presets()
            .into_iter()
            .map(|preset| preset.provider)
            .collect();
        assert_eq!(
            providers,
            vec![
                "openai",
                "xai",
                "siliconflow",
                "novelai",
                "deepseek",
                "bigmodel",
                "gemini",
                "ollama",
                "custom",
            ]
        );
    }

    #[test]
    fn verify_verified_capabilities_requires_exact_endpoint_test() {
        let official = profile(
            "deepseek",
            "openai",
            "https://api.deepseek.com",
            "deepseek-v4-flash",
        );
        let capabilities = resolve_model_profile_capabilities(&official).unwrap();
        assert_eq!(capabilities.context_window_tokens, Some(1_000_000));
        assert_eq!(capabilities.max_output_tokens, 384_000);
        assert!(capabilities.reasoning);

        // 自建代理 → 无协议证据
        let proxy = profile(
            "deepseek",
            "openai",
            "https://my-proxy.example.com/v1",
            "deepseek-v4-flash",
        );
        assert!(resolve_model_profile_capabilities(&proxy).is_none());

        // 协议不符 → 无证据
        let wrong_protocol = profile(
            "deepseek",
            "gemini",
            "https://api.deepseek.com",
            "deepseek-v4-flash",
        );
        assert!(resolve_model_profile_capabilities(&wrong_protocol).is_none());

        // 精确 slug 不匹配 → 无证据
        let wrong_slug = profile(
            "deepseek",
            "openai",
            "https://api.deepseek.com",
            "deepseek-v4-flash-preview",
        );
        assert!(resolve_model_profile_capabilities(&wrong_slug).is_none());
    }

    #[test]
    fn plain_models_have_no_capability_evidence_test() {
        // glm-4.5 只有 legacy maxTokens，没有能力元数据
        let glm = profile(
            "bigmodel",
            "openai",
            "https://open.bigmodel.cn/api/paas/v4",
            "glm-4.5",
        );
        assert!(resolve_model_profile_capabilities(&glm).is_none());

        // OpenAI 内置模型同样只有 legacy 上限
        let gpt = profile("openai", "openai", "https://api.openai.com", "gpt-4o");
        assert!(resolve_model_profile_capabilities(&gpt).is_none());
    }

    #[test]
    fn endpoint_normalization_accepts_case_and_trailing_slash_test() {
        let variants = [
            "https://api.deepseek.com/",
            "https://API.DEEPSEEK.COM",
            "  https://api.deepseek.com  ",
        ];
        for base_url in variants {
            let candidate = profile("deepseek", "openai", base_url, "deepseek-v4-flash");
            assert!(
                resolve_model_profile_capabilities(&candidate).is_some(),
                "{base_url} 必须与内置端点等价"
            );
        }
    }

    #[test]
    fn endpoint_normalization_rejects_credentials_and_query_test() {
        let with_credentials = profile(
            "deepseek",
            "openai",
            "https://user:secret@api.deepseek.com",
            "deepseek-v4-flash",
        );
        assert!(resolve_model_profile_capabilities(&with_credentials).is_none());

        let with_query = profile(
            "deepseek",
            "openai",
            "https://api.deepseek.com?key=1",
            "deepseek-v4-flash",
        );
        assert!(resolve_model_profile_capabilities(&with_query).is_none());

        let with_fragment = profile(
            "deepseek",
            "openai",
            "https://api.deepseek.com#frag",
            "deepseek-v4-flash",
        );
        assert!(resolve_model_profile_capabilities(&with_fragment).is_none());
    }

    #[test]
    fn path_is_preserved_when_comparing_endpoints_test() {
        // baseUrl 带路径（bigmodel 的 /api/paas/v4）必须整体匹配，不能只看 host
        let with_path = profile(
            "bigmodel",
            "openai",
            "https://open.bigmodel.cn/api/paas/v4",
            "glm-5",
        );
        assert!(resolve_model_profile_reasoning_mapping(&with_path).is_none());

        let without_path = profile("bigmodel", "openai", "https://open.bigmodel.cn", "glm-5");
        assert!(
            resolve_model_profile_reasoning_mapping(&without_path).is_none(),
            "路径不一致时不得视为同一端点（此处两者都无映射，需靠能力解析区分）"
        );
        // 用有大写 host 的等价写法确认路径比较仍然有效
        let same_path_other_case = profile(
            "bigmodel",
            "openai",
            "https://OPEN.BIGMODEL.CN/api/paas/v4/",
            "glm-4.7",
        );
        assert!(resolve_model_profile_reasoning_mapping(&same_path_other_case).is_none());
    }

    #[test]
    fn reasoning_mapping_is_cloned_not_shared_test() {
        let deepseek = profile(
            "deepseek",
            "openai",
            "https://api.deepseek.com",
            "deepseek-v4-pro",
        );
        let mut first = resolve_model_profile_reasoning_mapping(&deepseek).unwrap();
        first.supported_efforts.clear();
        let second = resolve_model_profile_reasoning_mapping(&deepseek).unwrap();
        assert_eq!(
            second.supported_efforts.len(),
            4,
            "返回的映射不得被调用方污染全局预设"
        );
    }

    #[test]
    fn capability_profile_from_value_requires_string_fields_test() {
        let value = serde_json::json!({
            "provider": "deepseek",
            "protocol": "openai",
            "baseUrl": "https://api.deepseek.com",
            "modelName": "deepseek-v4-flash",
        });
        let profile = ModelCapabilityProfile::from_value(&value);
        assert!(resolve_model_profile_capabilities(&profile).is_some());

        // 数字类型的 provider → 视为缺失（对齐基线 typeof 守卫）
        let typed_wrong = serde_json::json!({
            "provider": 42,
            "protocol": "openai",
            "baseUrl": "https://api.deepseek.com",
            "modelName": "deepseek-v4-flash",
        });
        assert!(
            resolve_model_profile_capabilities(&ModelCapabilityProfile::from_value(&typed_wrong))
                .is_none()
        );
    }

    #[test]
    fn policy_and_presets_agree_on_unsupported_endpoints_test() {
        let proxy = profile("custom", "openai", "https://relay.example.com/v1", "gpt-4o");
        let resolution = resolve_reasoning_policy(&proxy, None, Some("auto"), Some("review"));
        assert!(resolution.provider_directive.is_none());
    }

    #[test]
    fn siliconflow_catalog_carries_verified_reasoning_mappings_test() {
        // 基线 SiliconFlow 目录：DeepSeek-V4 Flash/Pro 与
        // Qwen/Qwen3.8-27B 的已验证推理映射。
        let flash = profile(
            "siliconflow",
            "openai",
            "https://api.siliconflow.cn/v1",
            "deepseek-ai/DeepSeek-V4-Flash",
        );
        let mapping = resolve_model_profile_reasoning_mapping(&flash).unwrap();
        assert_eq!(mapping.adapter, ReasoningAdapter::SiliconFlowV4Thinking);
        assert_eq!(
            mapping.supported_efforts,
            vec![ReasoningEffort::High, ReasoningEffort::Max]
        );
        assert_eq!(
            mapping.provider_value(ReasoningEffort::High),
            Some(&PresetValue::Text("high".to_string()))
        );
        assert_eq!(
            mapping.provider_value(ReasoningEffort::Max),
            Some(&PresetValue::Text("max".to_string()))
        );

        let qwen = profile(
            "siliconflow",
            "openai",
            "https://api.siliconflow.cn/v1",
            "Qwen/Qwen3.8-27B",
        );
        let mapping = resolve_model_profile_reasoning_mapping(&qwen).unwrap();
        assert_eq!(mapping.adapter, ReasoningAdapter::OpenAiThinkingBudget);
        assert_eq!(
            mapping.provider_value(ReasoningEffort::Off),
            Some(&PresetValue::Number(0))
        );
        assert_eq!(
            mapping.provider_value(ReasoningEffort::Medium),
            Some(&PresetValue::Number(16_384))
        );

        // SiliconFlow 模型无能力元数据（基线无 capabilities 字段）。
        assert!(resolve_model_profile_capabilities(&flash).is_none());
        assert!(resolve_model_profile_capabilities(&qwen).is_none());

        // 端点不符（自建代理）→ 无映射。
        let proxy = profile(
            "siliconflow",
            "openai",
            "https://my-proxy.example.com/v1",
            "deepseek-ai/DeepSeek-V4-Flash",
        );
        assert!(resolve_model_profile_reasoning_mapping(&proxy).is_none());
    }

    #[test]
    fn is_reasoning_mapping_validates_the_full_baseline_contract_test() {
        // 合法手动映射（含 requestAliases）。
        let valid = serde_json::json!({
            "adapter": "deepseek-v4-thinking",
            "supportedEfforts": ["off", "high"],
            "providerValues": { "off": "disabled", "high": "high" },
            "requestAliases": { "medium": "high" },
        });
        let mapping = is_reasoning_mapping(&valid).unwrap();
        assert_eq!(mapping.adapter, ReasoningAdapter::DeepSeekV4Thinking);
        assert_eq!(
            mapping.request_aliases,
            vec![(ReasoningEffort::Medium, ReasoningEffort::High)]
        );

        // 基线允许非 off 强度的 enabled wire；不能因错误夹具缩窄合同。
        let mut enabled = valid.clone();
        enabled["providerValues"]["high"] = serde_json::json!("enabled");
        assert!(is_reasoning_mapping(&enabled).is_some());

        // 基线拒绝面（逐条对齐 isReasoningMapping）。
        let rejections = [
            // 非对象 / 数组
            serde_json::json!(null),
            serde_json::json!([]),
            // 未知 adapter
            serde_json::json!({
                "adapter": "unknown-adapter",
                "supportedEfforts": ["off"],
                "providerValues": { "off": "disabled" },
            }),
            // supportedEfforts 为空
            serde_json::json!({
                "adapter": "openai-thinking-budget",
                "supportedEfforts": [],
                "providerValues": {},
            }),
            // supportedEfforts 重复
            serde_json::json!({
                "adapter": "openai-thinking-budget",
                "supportedEfforts": ["off", "off"],
                "providerValues": { "off": 0 },
            }),
            // 多余键
            serde_json::json!({
                "adapter": "openai-thinking-budget",
                "supportedEfforts": ["off"],
                "providerValues": { "off": 0 },
                "extra": 1,
            }),
            // providerValues 键不在 supportedEfforts 内
            serde_json::json!({
                "adapter": "openai-thinking-budget",
                "supportedEfforts": ["off"],
                "providerValues": { "off": 0, "medium": 4096 },
            }),
            // supportedEfforts 条目缺少 provider 取值
            serde_json::json!({
                "adapter": "openai-thinking-budget",
                "supportedEfforts": ["off", "medium"],
                "providerValues": { "off": 0 },
            }),
            // openai-reasoning-effort：wire 非字符串
            serde_json::json!({
                "adapter": "openai-reasoning-effort",
                "supportedEfforts": ["high"],
                "providerValues": { "high": 1 },
            }),
            // openai-reasoning-effort：wire 不匹配基线正则
            serde_json::json!({
                "adapter": "openai-reasoning-effort",
                "supportedEfforts": ["high"],
                "providerValues": { "high": "1high" },
            }),
            // openai-reasoning-effort：off 的非法别名
            serde_json::json!({
                "adapter": "openai-reasoning-effort",
                "supportedEfforts": ["off"],
                "providerValues": { "off": "maybe" },
            }),
            // gemini-thinking-budget：off 必须为 0
            serde_json::json!({
                "adapter": "gemini-thinking-budget",
                "supportedEfforts": ["off"],
                "providerValues": { "off": 8 },
            }),
            // gemini-thinking-budget：负数预算
            serde_json::json!({
                "adapter": "gemini-thinking-budget",
                "supportedEfforts": ["low"],
                "providerValues": { "low": -1 },
            }),
            // gemini-thinking-budget：非整数
            serde_json::json!({
                "adapter": "gemini-thinking-budget",
                "supportedEfforts": ["low"],
                "providerValues": { "low": 1.5 },
            }),
            // openai-thinking-budget：非 off 取值为 0
            serde_json::json!({
                "adapter": "openai-thinking-budget",
                "supportedEfforts": ["off", "medium"],
                "providerValues": { "off": 0, "medium": 0 },
            }),
            // deepseek-v4-thinking：off 必须为 disabled
            serde_json::json!({
                "adapter": "deepseek-v4-thinking",
                "supportedEfforts": ["off"],
                "providerValues": { "off": "enabled" },
            }),
            // deepseek-v4-thinking：非法取值
            serde_json::json!({
                "adapter": "deepseek-v4-thinking",
                "supportedEfforts": ["high"],
                "providerValues": { "high": "invalid" },
            }),
            // siliconflow-v4-thinking：仅 high/max
            serde_json::json!({
                "adapter": "siliconflow-v4-thinking",
                "supportedEfforts": ["low"],
                "providerValues": { "low": "low" },
            }),
            // siliconflow-v4-thinking：取值必须与强度同名
            serde_json::json!({
                "adapter": "siliconflow-v4-thinking",
                "supportedEfforts": ["high"],
                "providerValues": { "high": "max" },
            }),
            // requestAliases：请求强度非法
            serde_json::json!({
                "adapter": "deepseek-v4-thinking",
                "supportedEfforts": ["off", "high"],
                "providerValues": { "off": "disabled", "high": "high" },
                "requestAliases": { "ultra": "high" },
            }),
            // requestAliases：目标不在 supportedEfforts 内
            serde_json::json!({
                "adapter": "deepseek-v4-thinking",
                "supportedEfforts": ["off", "high"],
                "providerValues": { "off": "disabled", "high": "high" },
                "requestAliases": { "medium": "low" },
            }),
            // requestAliases：off 的别名必须是 off
            serde_json::json!({
                "adapter": "deepseek-v4-thinking",
                "supportedEfforts": ["off", "high"],
                "providerValues": { "off": "disabled", "high": "high" },
                "requestAliases": { "off": "high" },
            }),
            // requestAliases：null 拒绝
            serde_json::json!({
                "adapter": "deepseek-v4-thinking",
                "supportedEfforts": ["off", "high"],
                "providerValues": { "off": "disabled", "high": "high" },
                "requestAliases": null,
            }),
        ];
        for value in rejections {
            assert!(is_reasoning_mapping(&value).is_none(), "必须拒绝：{value}");
        }
    }
}
