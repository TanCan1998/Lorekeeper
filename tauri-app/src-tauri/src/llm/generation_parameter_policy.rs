//! 基线原版生成参数策略 —— 移植自
//! `electron/llm/generation-parameter-policy.ts`。
//!
//! 单一实现（G4-B3-M 裁决，B3-M1 双实现合并）：D2 IPC 调整版
//! `crate::llm::params`（`max_tokens` 为请求→档案**回退**语义）
//! 已整体删除，本模块是**唯一**的生成参数策略——
//! `resolve_generation_parameters` 的 `max_tokens` 为能力约束钳制
//! （基线 `Math.min(request ?? ∞, userMax ?? ∞, modelMax ?? ∞)`），
//! 同时服务计划构造路径（`main_generation_plan` 的
//! `buildMainGenerationPlan` / `resolvePhysicalLiability`）与
//! `llm:generate` / `llm:generate-stream` / 模型管理探测命令。
//!
//! 与基线的偏差：基线 `validateKimiTemperature` 抛出异常，
//! Rust 侧按 crate 惯例以 `Result<_, String>` 建模
//! （错误消息逐字保留）。
//!
//! 依赖方向 `llm → services` 与基线
//! `electron/llm → src/services/generation` 一致：返回类型
//! 复用 B3-1a 移植的
//! [`crate::services::task_budget_planner::TaskBudgetCapabilityConstraints`]
//! （字段 1:1，基线同源）。

use crate::llm::lease::LeaseModelProfile;
use crate::llm::presets::{ModelCapabilityProfile, MAX_SAFE_INTEGER};
use crate::llm::provider_presets::{
    resolve_model_profile_budget_capabilities, ModelBudgetCapabilityProfile,
};
use crate::llm::reasoning::{resolve_reasoning_policy, ProviderReasoningDirective};
use crate::services::task_budget_planner::{
    TaskBudgetCapabilityConstraints, TaskBudgetCapabilitySource,
};
use serde_json::Value;

/// 从模型档案解析出的有效请求参数（基线
/// `ResolvedGenerationParameters`）。
///
/// `None` 表示 provider 必须省略该字段（基线 `undefined`）。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedGenerationParameters {
    /// `None` = provider 必须省略该字段。
    pub temperature: Option<f64>,
    /// 能力约束钳制后的输出预算；`None` = 无有限上界
    /// （基线 `Infinity`，由 provider 自行决定）。
    pub max_tokens: Option<u64>,
    /// 结构化输出要求，原样透传给 provider 载荷。
    pub response_format: Option<Value>,
    pub reasoning: Option<ProviderReasoningDirective>,
}

/// 调用方给出的参数意图（基线 `GenerationParameterRequest`：
/// `Pick<LLMRequest, 'maxTokens' | 'responseFormat'>` +
/// 创作策略 / 受控阶段）。
#[derive(Debug, Clone, Default)]
pub struct GenerationParameterRequest<'a> {
    pub max_tokens: Option<u64>,
    pub response_format: Option<Value>,
    pub creative_strategy: Option<&'a str>,
    pub reasoning_stage: Option<&'a str>,
}

/// 官方 Kimi 端点（仅这两个主机名被认可为官方）。
const OFFICIAL_KIMI_HOSTS: [&str; 2] = ["api.moonshot.cn", "api.moonshot.ai"];

/// 采样温度由端点固定的模型家族前缀。
const KIMI_FIXED_TEMPERATURE_MODEL_PREFIXES: [&str; 4] =
    ["kimi-k3", "kimi-k2.7", "kimi-k2.6", "kimi-k2.5"];

/// 采样温度非法时的用户可见文案（命令层会再加 `Error: ` 前缀）。
pub const KIMI_TEMPERATURE_MESSAGE: &str =
    "Kimi API 的 temperature 必须在 0 到 1 之间。请在模型设置中调整后重试。";

fn has_model_family_prefix(model_name: &str, prefixes: &[&str]) -> bool {
    let normalized = model_name.trim().to_lowercase();
    prefixes
        .iter()
        .any(|prefix| normalized == *prefix || normalized.starts_with(&format!("{prefix}-")))
}

/// 是否官方 Kimi 端点（要求 https 且主机名精确匹配，路径不限）。
fn is_official_kimi_host(base_url: &str) -> bool {
    let Ok(endpoint) = url::Url::parse(base_url.trim()) else {
        return false;
    };
    if endpoint.scheme() != "https" {
        return false;
    }
    match endpoint.host_str() {
        Some(host) => OFFICIAL_KIMI_HOSTS.contains(&host.to_lowercase().as_str()),
        None => false,
    }
}

/// 基线 `validateKimiTemperature`（中文错误消息逐字保留）。
fn validate_kimi_temperature(temperature: Option<f64>) -> Result<(), String> {
    if let Some(value) = temperature {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            return Ok(());
        }
    }
    Err(KIMI_TEMPERATURE_MESSAGE.to_string())
}

/// 基线 `positiveInteger`：正的安全整数，否则 `None`。
fn positive_integer(value: Option<&Value>) -> Option<i64> {
    let number = value?.as_i64()?;
    (number > 0 && number <= MAX_SAFE_INTEGER as i64).then_some(number)
}

/// 基线 `positiveInteger` 的类型化值版本（预算证据字段）。
fn positive_integer_u64(value: u64) -> Option<i64> {
    (value > 0 && value <= MAX_SAFE_INTEGER).then_some(value as i64)
}

/// 基线 `resolveGenerationCapabilityConstraints`。
///
/// 分离已验证的提供方能力与用户运营限制：S07 规划器可以用
/// 后者收窄已验证上限，但绝不把用户输入的大数当作端点能力
/// 的证明。
pub fn resolve_generation_capability_constraints(
    model: &LeaseModelProfile<'_>,
) -> TaskBudgetCapabilityConstraints {
    let verified = resolve_model_profile_budget_capabilities(&ModelBudgetCapabilityProfile {
        protocol: Some(model.protocol),
        base_url: Some(model.base_url),
        model_name: Some(model.model_name),
    });
    let model_context_window_tokens = verified
        .as_ref()
        .and_then(|evidence| positive_integer_u64(evidence.context_window_tokens));
    let model_max_output_tokens = verified
        .as_ref()
        .and_then(|evidence| positive_integer_u64(evidence.max_output_tokens));
    let user_output_limits = [
        positive_integer(
            model
                .capabilities
                .and_then(|value| value.get("maxOutputTokens")),
        ),
        positive_integer(model.max_tokens),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<i64>>();
    TaskBudgetCapabilityConstraints {
        model_context_window_tokens,
        model_max_output_tokens,
        model_context_source: if model_context_window_tokens.is_some() {
            TaskBudgetCapabilitySource::VerifiedProviderPreset
        } else {
            TaskBudgetCapabilitySource::Unknown
        },
        model_output_source: if model_max_output_tokens.is_some() {
            TaskBudgetCapabilitySource::VerifiedProviderPreset
        } else {
            TaskBudgetCapabilitySource::Unknown
        },
        user_context_window_tokens: positive_integer(
            model
                .capabilities
                .and_then(|value| value.get("contextWindowTokens")),
        ),
        user_max_output_tokens: if user_output_limits.is_empty() {
            None
        } else {
            user_output_limits.into_iter().min()
        },
    }
}

/// 基线 `resolveGenerationParameters`。
///
/// 从单一模型档案解析有效请求参数。模型档案温度是唯一用户
/// 可见的采样输入；调用方可以选择工作流特征与输出预算，但
/// 不能覆盖温度。
///
/// `Err` 仅在模型档案自身非法时发生（官方 Kimi 端点的温度
/// 越界）——这类失败必须**阻断**调用，而不是静默回退到
/// 默认温度。
pub fn resolve_generation_parameters(
    model: &LeaseModelProfile<'_>,
    request: &GenerationParameterRequest<'_>,
) -> Result<ResolvedGenerationParameters, String> {
    let is_official_kimi = is_official_kimi_host(model.base_url);
    let uses_fixed_kimi_temperature = is_official_kimi
        && has_model_family_prefix(model.model_name, &KIMI_FIXED_TEMPERATURE_MODEL_PREFIXES);

    if is_official_kimi && !uses_fixed_kimi_temperature {
        validate_kimi_temperature(model.temperature.and_then(|value| value.as_f64()))?;
    }

    let reasoning_resolution = resolve_reasoning_policy(
        &ModelCapabilityProfile {
            provider: Some(model.provider),
            protocol: Some(model.protocol),
            base_url: Some(model.base_url),
            model_name: Some(model.model_name),
            reasoning_mapping: model.reasoning_mapping,
        },
        model.reasoning_override,
        request.creative_strategy,
        request.reasoning_stage,
    );
    let limits = resolve_generation_capability_constraints(model);

    Ok(ResolvedGenerationParameters {
        temperature: if uses_fixed_kimi_temperature {
            None
        } else {
            model.temperature.and_then(|value| value.as_f64())
        },
        // 基线 `Math.min(request.maxTokens ?? ∞, userMax ?? ∞, modelMax ?? ∞)`：
        // 任一有限上界都参与取小，全缺省时为 `None`（∞）。
        max_tokens: [
            request.max_tokens,
            limits.user_max_output_tokens.map(|value| value as u64),
            limits.model_max_output_tokens.map(|value| value as u64),
        ]
        .into_iter()
        .flatten()
        .min(),
        response_format: request
            .response_format
            .as_ref()
            .filter(|value| !value.is_null())
            .cloned(),
        reasoning: reasoning_resolution.provider_directive,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 基线测试夹具 `openAIModel`（gpt-test 不在内置目录中，
    /// 因此无验证证据、无推理指令）。
    fn open_ai_model() -> Value {
        json!({
            "id": "openai-test",
            "name": "OpenAI test",
            "provider": "openai",
            "protocol": "openai",
            "modelName": "gpt-test",
            "apiKey": "test-key",
            "baseUrl": "https://api.openai.com/v1",
            "temperature": 1,
            "maxTokens": 4096,
            "purposes": ["generation"],
        })
    }

    fn request<'a>() -> GenerationParameterRequest<'a> {
        GenerationParameterRequest {
            max_tokens: None,
            response_format: None,
            creative_strategy: Some("auto"),
            reasoning_stage: Some("drafting"),
        }
    }

    #[test]
    fn request_output_limit_is_clamped_by_author_and_model_caps_test() {
        let value = json!({
            "provider": "custom", "protocol": "openai", "modelName": "synthetic-model",
            "baseUrl": "https://compatible.example/v1", "maxTokens": 4096,
            "capabilities": {
                "contextWindowTokens": 131072, "maxOutputTokens": 32768,
                "reasoning": false, "structuredOutput": false, "usage": false,
            },
        });
        let resolved = resolve_generation_parameters(
            &LeaseModelProfile::from_value(&value),
            &GenerationParameterRequest {
                max_tokens: Some(8192),
                ..request()
            },
        )
        .unwrap();
        assert_eq!(resolved.max_tokens, Some(4096));
    }

    #[test]
    fn keeps_a_lower_author_output_limit_when_capability_metadata_declares_more_test() {
        let value = json!({
            "id": "openai-test",
            "provider": "openai",
            "protocol": "openai",
            "modelName": "gpt-test",
            "baseUrl": "https://api.openai.com/v1",
            "temperature": 1,
            "maxTokens": 1024,
            "capabilities": {
                "contextWindowTokens": 131072,
                "maxOutputTokens": 32768,
                "reasoning": false,
                "structuredOutput": false,
                "usage": false,
            },
        });
        let constraints =
            resolve_generation_capability_constraints(&LeaseModelProfile::from_value(&value));
        assert_eq!(constraints.user_max_output_tokens, Some(1024));
    }

    #[test]
    fn separates_a_verified_provider_limit_from_a_larger_user_operational_cap_test() {
        let value = json!({
            "id": "grok-4.5",
            "provider": "xai",
            "protocol": "openai",
            "modelName": "grok-4.5",
            "baseUrl": "https://api.x.ai/v1",
            "temperature": 1,
            "maxTokens": 131_072,
            "capabilities": {
                "contextWindowTokens": 131_072,
                "maxOutputTokens": 131_072,
                "reasoning": true,
                "structuredOutput": true,
                "usage": true,
            },
        });
        let constraints =
            resolve_generation_capability_constraints(&LeaseModelProfile::from_value(&value));
        assert_eq!(
            constraints,
            TaskBudgetCapabilityConstraints {
                model_context_window_tokens: Some(500_000),
                model_max_output_tokens: Some(8_192),
                model_context_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                model_output_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                user_context_window_tokens: Some(131_072),
                user_max_output_tokens: Some(131_072),
            }
        );
    }

    #[test]
    fn does_not_infer_capability_from_a_model_name_on_an_unverified_endpoint_test() {
        let value = json!({
            "id": "openai-test",
            "provider": "custom",
            "protocol": "openai",
            "modelName": "grok-4.5",
            "baseUrl": "https://proxy.example.test/v1",
            "temperature": 1,
            "maxTokens": 131_072,
            "capabilities": {
                "contextWindowTokens": 131_072,
                "maxOutputTokens": 131_072,
                "reasoning": true,
                "structuredOutput": true,
                "usage": true,
            },
        });
        let constraints =
            resolve_generation_capability_constraints(&LeaseModelProfile::from_value(&value));
        assert_eq!(
            constraints,
            TaskBudgetCapabilityConstraints {
                model_context_window_tokens: None,
                model_max_output_tokens: None,
                model_context_source: TaskBudgetCapabilitySource::Unknown,
                model_output_source: TaskBudgetCapabilitySource::Unknown,
                user_context_window_tokens: Some(131_072),
                user_max_output_tokens: Some(131_072),
            }
        );
    }

    #[test]
    fn keeps_siliconflow_provider_evidence_separate_from_the_smaller_user_cap_test() {
        let value = json!({
            "id": "openai-test",
            "provider": "siliconflow",
            "protocol": "openai",
            "modelName": "deepseek-ai/DeepSeek-V4-Flash",
            "baseUrl": "https://api.siliconflow.com/v1",
            "temperature": 1,
            "maxTokens": 16_384,
            "capabilities": {
                "contextWindowTokens": 65_536,
                "maxOutputTokens": 16_384,
                "reasoning": false,
                "structuredOutput": false,
                "usage": false,
            },
        });
        let constraints =
            resolve_generation_capability_constraints(&LeaseModelProfile::from_value(&value));
        assert_eq!(
            constraints,
            TaskBudgetCapabilityConstraints {
                model_context_window_tokens: Some(1_000_000),
                model_max_output_tokens: Some(393_000),
                model_context_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                model_output_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                user_context_window_tokens: Some(65_536),
                user_max_output_tokens: Some(16_384),
            }
        );
    }

    #[test]
    fn recognizes_the_approved_openai_compatible_profile_by_exact_siliconflow_endpoint_and_slug_test(
    ) {
        let value = json!({
            "id": "openai-test",
            "provider": "openai",
            "protocol": "openai",
            "modelName": "deepseek-ai/DeepSeek-V4-Flash",
            "baseUrl": "https://api.siliconflow.cn/v1",
            "temperature": 1,
            "maxTokens": 16_384,
        });
        let constraints =
            resolve_generation_capability_constraints(&LeaseModelProfile::from_value(&value));
        assert_eq!(constraints.model_context_window_tokens, Some(1_000_000));
        assert_eq!(constraints.model_max_output_tokens, Some(393_000));
        assert_eq!(
            constraints.model_context_source,
            TaskBudgetCapabilitySource::VerifiedProviderPreset
        );
        assert_eq!(
            constraints.model_output_source,
            TaskBudgetCapabilitySource::VerifiedProviderPreset
        );
        assert_eq!(constraints.user_max_output_tokens, Some(16_384));
    }

    #[test]
    fn forwards_generic_model_settings_without_inventing_a_reasoning_field_test() {
        let value = open_ai_model();
        let model = LeaseModelProfile::from_value(&value);
        let resolved = resolve_generation_parameters(
            &model,
            &GenerationParameterRequest {
                max_tokens: Some(512),
                response_format: Some(json!({ "type": "json_object" })),
                creative_strategy: Some("deep-planning"),
                reasoning_stage: Some("drafting"),
            },
        )
        .expect("合法档案必须解析成功");
        assert_eq!(resolved.temperature, Some(1.0));
        assert_eq!(resolved.max_tokens, Some(512));
        assert_eq!(
            resolved.response_format,
            Some(json!({ "type": "json_object" }))
        );
        assert_eq!(
            resolved.reasoning, None,
            "gpt-test 无验证证据，不得臆造推理字段"
        );
    }

    #[test]
    fn keeps_the_requested_output_budget_when_the_model_output_limit_is_cleared_test() {
        let value = serde_json::json!({
            "id": "openai-test",
            "provider": "openai",
            "protocol": "openai",
            "modelName": "gpt-test",
            "baseUrl": "https://api.openai.com/v1",
            "temperature": 1,
            "maxTokens": 0,
        });
        let model = LeaseModelProfile::from_value(&value);
        let resolved = resolve_generation_parameters(
            &model,
            &GenerationParameterRequest {
                max_tokens: Some(1024),
                ..request()
            },
        )
        .expect("maxTokens 为 0 必须按缺省处理");
        assert_eq!(resolved.max_tokens, Some(1024));
    }

    #[test]
    fn omits_fixed_temperature_for_kimi_model_families_on_official_hosts_test() {
        let official_kimi_hosts = ["https://api.moonshot.cn/v1", "https://api.moonshot.ai/v1"];
        let fixed_kimi_models = ["kimi-k3", "kimi-k2.7", "kimi-k2.6", "kimi-k2.5"];
        for base_url in official_kimi_hosts {
            for model_name in fixed_kimi_models {
                let value = json!({
                    "id": "openai-test",
                    "provider": "custom",
                    "protocol": "openai",
                    "modelName": model_name,
                    "baseUrl": base_url,
                    "temperature": 0.7,
                    "maxTokens": 4096,
                });
                let model = LeaseModelProfile::from_value(&value);
                let resolved = resolve_generation_parameters(
                    &model,
                    &GenerationParameterRequest {
                        max_tokens: Some(512),
                        ..request()
                    },
                )
                .unwrap_or_else(|error| panic!("{model_name} 属于固定采样家族，不应报错：{error}"));
                assert_eq!(
                    resolved.temperature, None,
                    "{model_name}@{base_url} 属于固定采样家族，必须省略 temperature"
                );
                assert_eq!(resolved.max_tokens, Some(512));
            }
        }
    }

    #[test]
    fn enforces_the_documented_range_for_an_official_kimi_model_without_a_fixed_temperature_rule_test(
    ) {
        let base_value = json!({
            "id": "openai-test",
            "provider": "custom",
            "protocol": "openai",
            "modelName": "kimi-future-preview",
            "baseUrl": "https://api.moonshot.cn/v1",
            "maxTokens": 4096,
        });
        let mut in_range = base_value.clone();
        in_range["temperature"] = json!(0.6);
        let resolved = resolve_generation_parameters(
            &LeaseModelProfile::from_value(&in_range),
            &GenerationParameterRequest {
                max_tokens: Some(512),
                ..request()
            },
        )
        .expect("0.6 在文档化范围内");
        assert_eq!(resolved.temperature, Some(0.6));

        let mut out_of_range = base_value.clone();
        out_of_range["temperature"] = json!(1.1);
        let error = resolve_generation_parameters(
            &LeaseModelProfile::from_value(&out_of_range),
            &GenerationParameterRequest {
                max_tokens: Some(512),
                ..request()
            },
        )
        .unwrap_err();
        assert!(
            error.contains("0 到 1"),
            "错误消息必须逐字保留基线中文文案：{error}"
        );
        assert_eq!(error, KIMI_TEMPERATURE_MESSAGE);
    }

    #[test]
    fn does_not_apply_official_kimi_rules_or_reasoning_fields_to_a_proxy_endpoint_test() {
        let value = json!({
            "id": "openai-test",
            "provider": "custom",
            "protocol": "openai",
            "modelName": "kimi-k3",
            "baseUrl": "https://kimi-proxy.example.test/v1",
            "temperature": 0.3,
            "reasoningOverride": "max",
            "maxTokens": 4096,
        });
        let model = LeaseModelProfile::from_value(&value);
        let resolved = resolve_generation_parameters(
            &model,
            &GenerationParameterRequest {
                max_tokens: Some(512),
                response_format: None,
                creative_strategy: Some("deep-planning"),
                reasoning_stage: Some("planning"),
            },
        )
        .expect("代理端点必须正常解析");
        assert_eq!(resolved.temperature, Some(0.3));
        assert_eq!(resolved.max_tokens, Some(512));
        assert_eq!(
            resolved.reasoning, None,
            "代理端点无验证证据，不得带出推理字段"
        );
    }

    #[test]
    fn does_not_apply_official_kimi_rules_to_an_invalid_or_non_https_endpoint_test() {
        for base_url in [
            "api.moonshot.cn/v1",
            "http://api.moonshot.cn/v1",
            "ftp://api.moonshot.ai/v1",
        ] {
            let value = json!({
                "id": "openai-test",
                "provider": "custom",
                "protocol": "openai",
                "modelName": "kimi-k3",
                "baseUrl": base_url,
                "temperature": 0.3,
                "maxTokens": 4096,
            });
            let model = LeaseModelProfile::from_value(&value);
            let resolved = resolve_generation_parameters(
                &model,
                &GenerationParameterRequest {
                    max_tokens: Some(512),
                    ..request()
                },
            )
            .unwrap_or_else(|error| panic!("{base_url} 必须正常解析：{error}"));
            assert_eq!(
                resolved.temperature,
                Some(0.3),
                "{base_url} 不是官方 https 端点，不得触发固定温度规则"
            );
            assert_eq!(resolved.max_tokens, Some(512));
        }
    }

    #[test]
    fn maps_the_profile_override_through_an_exact_verified_model_preset_test() {
        let value = json!({
            "id": "grok-4.5",
            "provider": "xai",
            "protocol": "openai",
            "modelName": "grok-4.5",
            "baseUrl": "https://api.x.ai/v1",
            "temperature": 1,
            "reasoningOverride": "max",
            "maxTokens": 4096,
        });
        let model = LeaseModelProfile::from_value(&value);
        let resolved = resolve_generation_parameters(
            &model,
            &GenerationParameterRequest {
                max_tokens: Some(512),
                reasoning_stage: Some("drafting"),
                ..request()
            },
        )
        .expect("已验证预设必须解析成功");
        assert_eq!(resolved.temperature, Some(1.0));
        assert_eq!(resolved.max_tokens, Some(512));
        assert_eq!(
            resolved.reasoning,
            Some(ProviderReasoningDirective::OpenAiReasoningEffort {
                reasoning_effort: "high".to_string(),
            }),
            "override 'max' 必须经精确模型预设映射并封顶到 'high'"
        );
    }
}
