//! 推理策略缝 —— 迁移自 `src/shared/reasoning-policy.ts` + `src/shared/reasoning-types.ts`。
//!
//! 该模块是**唯一的推理决策点**：调用方只提供产品意图（创作策略 + 受控语义阶段），
//! 模型档案优先级、已验证能力查表、provider 级封顶与用户可见的解析回执全部由这里拥有。
//! 任何调用方都不得自行推断推理强度（`docs/product-domain.md` 提示词合同）。

use serde::{Deserialize, Serialize};

use crate::llm::presets::{ModelCapabilityProfile, PresetValue, VerifiedReasoningMapping};
/// 创作策略（产品意图）。`auto` 表示由阶段表决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CreativeStrategy {
    Auto,
    FluentDrafting,
    ConsistencyFirst,
    DeepPlanning,
}

impl CreativeStrategy {
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("fluent-drafting") => CreativeStrategy::FluentDrafting,
            Some("consistency-first") => CreativeStrategy::ConsistencyFirst,
            Some("deep-planning") => CreativeStrategy::DeepPlanning,
            _ => CreativeStrategy::Auto,
        }
    }
}

/// 受控语义阶段 —— 由产品调用方给出，**永不**从诊断用 `purpose` 文案推断。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GenerationReasoningStage {
    Drafting,
    Planning,
    Review,
    General,
}

impl GenerationReasoningStage {
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("drafting") => GenerationReasoningStage::Drafting,
            Some("planning") => GenerationReasoningStage::Planning,
            Some("review") => GenerationReasoningStage::Review,
            _ => GenerationReasoningStage::General,
        }
    }
}

/// 推理强度（产品侧命名空间）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Off,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl ReasoningEffort {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "off" => Some(ReasoningEffort::Off),
            "low" => Some(ReasoningEffort::Low),
            "medium" => Some(ReasoningEffort::Medium),
            "high" => Some(ReasoningEffort::High),
            "xhigh" => Some(ReasoningEffort::XHigh),
            "max" => Some(ReasoningEffort::Max),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ReasoningEffort::Off => "off",
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
            ReasoningEffort::XHigh => "xhigh",
            ReasoningEffort::Max => "max",
        }
    }

    /// 强度排序（对齐基线 `EFFORT_RANK`）。
    fn rank(self) -> u8 {
        match self {
            ReasoningEffort::Off => 0,
            ReasoningEffort::Low => 1,
            ReasoningEffort::Medium => 2,
            ReasoningEffort::High => 3,
            ReasoningEffort::XHigh => 4,
            ReasoningEffort::Max => 5,
        }
    }
}

/// provider 请求适配器家族。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReasoningAdapter {
    #[serde(rename = "openai-reasoning-effort")]
    OpenAiReasoningEffort,
    #[serde(rename = "gemini-thinking-budget")]
    GeminiThinkingBudget,
    #[serde(rename = "deepseek-v4-thinking")]
    DeepSeekV4Thinking,
    #[serde(rename = "siliconflow-v4-thinking")]
    SiliconFlowV4Thinking,
    #[serde(rename = "openai-thinking-budget")]
    OpenAiThinkingBudget,
}
/// 深寻思考开关状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingState {
    Disabled,
    Enabled,
}

/// provider 级推理指令 —— 唯一允许进入请求载荷的推理表达形式。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "adapter")]
pub enum ProviderReasoningDirective {
    #[serde(rename = "openai-reasoning-effort")]
    OpenAiReasoningEffort {
        #[serde(rename = "reasoningEffort")]
        reasoning_effort: String,
    },
    #[serde(rename = "gemini-thinking-budget")]
    GeminiThinkingBudget {
        #[serde(rename = "thinkingBudget")]
        thinking_budget: i64,
    },
    #[serde(rename = "deepseek-v4-thinking")]
    DeepSeekV4Thinking {
        thinking: ThinkingState,
        #[serde(rename = "reasoningEffort", skip_serializing_if = "Option::is_none")]
        reasoning_effort: Option<ReasoningEffort>,
    },
    /// 基线 `siliconflow-v4-thinking` 形：仅显式 `high` / `max`
    /// 可达（由 [`provider_directive`] 守卫）。
    #[serde(rename = "siliconflow-v4-thinking")]
    SiliconFlowV4Thinking {
        #[serde(rename = "reasoningEffort")]
        reasoning_effort: ReasoningEffort,
    },
    /// 基线 `openai-thinking-budget` 形：`0` = 禁用思考，正数 =
    /// 与正文分开计费的独立思考预算。
    #[serde(rename = "openai-thinking-budget")]
    OpenAiThinkingBudget {
        #[serde(rename = "thinkingBudget")]
        thinking_budget: i64,
    },
}

/// 用户可见的解析结果状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningResolutionStatus {
    Mapped,
    Capped,
    Forced,
    Unsupported,
}

/// 决策来源：项目策略 or 模型档案覆盖。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReasoningSource {
    ProjectStrategy,
    ModelOverride,
}

/// 推理策略解析回执。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningPolicyResolution {
    pub requested: ReasoningEffort,
    pub effective: Option<ReasoningEffort>,
    pub status: ReasoningResolutionStatus,
    pub source: ReasoningSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_directive: Option<ProviderReasoningDirective>,
}

/// 阶段请求表（对齐基线 `STAGE_REQUESTS`）。
fn stage_request(strategy: CreativeStrategy, stage: GenerationReasoningStage) -> ReasoningEffort {
    use CreativeStrategy as S;
    use GenerationReasoningStage as G;
    use ReasoningEffort as E;
    match (strategy, stage) {
        (S::Auto, G::Drafting) => E::Low,
        (S::Auto, G::Planning) => E::Medium,
        (S::Auto, G::Review) => E::High,
        (S::Auto, G::General) => E::Low,
        (S::FluentDrafting, G::Drafting) => E::Off,
        (S::FluentDrafting, G::Planning) => E::Low,
        (S::FluentDrafting, G::Review) => E::Low,
        (S::FluentDrafting, G::General) => E::Low,
        (S::ConsistencyFirst, G::Drafting) => E::Low,
        (S::ConsistencyFirst, G::Planning) => E::High,
        (S::ConsistencyFirst, G::Review) => E::High,
        (S::ConsistencyFirst, G::General) => E::Medium,
        (S::DeepPlanning, G::Drafting) => E::Low,
        (S::DeepPlanning, G::Planning) => E::Max,
        (S::DeepPlanning, G::Review) => E::High,
        (S::DeepPlanning, G::General) => E::Medium,
    }
}

/// 把「解析后的有效强度」翻译为 provider 指令；无法表达即返回 `None`（= unsupported）。
fn provider_directive(
    mapping: &VerifiedReasoningMapping,
    effective: ReasoningEffort,
) -> Option<ProviderReasoningDirective> {
    let value = mapping.provider_value(effective);
    match mapping.adapter {
        ReasoningAdapter::OpenAiReasoningEffort => {
            let text = value?.as_str()?;
            if text.trim().is_empty() {
                return None;
            }
            Some(ProviderReasoningDirective::OpenAiReasoningEffort {
                reasoning_effort: text.to_string(),
            })
        }
        ReasoningAdapter::DeepSeekV4Thinking => {
            let text = value?.as_str()?;
            if effective == ReasoningEffort::Off && text == "disabled" {
                return Some(ProviderReasoningDirective::DeepSeekV4Thinking {
                    thinking: ThinkingState::Disabled,
                    reasoning_effort: None,
                });
            }
            if effective == ReasoningEffort::Off {
                return None;
            }
            // 非 off 的 enabled 不携带强度；其余强度来自 wire，而非产品命名。
            let reasoning_effort = match text {
                "enabled" => None,
                "low" | "high" | "max" => ReasoningEffort::parse(text),
                _ => return None,
            };
            Some(ProviderReasoningDirective::DeepSeekV4Thinking {
                thinking: ThinkingState::Enabled,
                reasoning_effort,
            })
        }
        // 基线 `providerDirective` 的 siliconflow 分支：仅显式
        // `high` / `max` 且 provider 取值与强度同名可达——绝不把
        // off/low/medium 强行提升到 high。
        ReasoningAdapter::SiliconFlowV4Thinking => {
            let matches_effort = matches!(effective, ReasoningEffort::High | ReasoningEffort::Max)
                && value.and_then(PresetValue::as_str) == Some(effective.as_str());
            matches_effort.then(|| ProviderReasoningDirective::SiliconFlowV4Thinking {
                reasoning_effort: effective,
            })
        }
        ReasoningAdapter::GeminiThinkingBudget => {
            let budget = value?.as_number()?;
            Some(ProviderReasoningDirective::GeminiThinkingBudget {
                thinking_budget: budget,
            })
        }
        // 基线 fallthrough（gemini-thinking-budget 与
        // openai-thinking-budget 共用）：数值取值即构造指令；
        // `0` 表示禁用思考，正数表示独立预算。
        ReasoningAdapter::OpenAiThinkingBudget => {
            let budget = value?.as_number()?;
            Some(ProviderReasoningDirective::OpenAiThinkingBudget {
                thinking_budget: budget,
            })
        }
    }
}

/// 在已验证映射内寻找最接近的可用强度（`mapped` / `capped` / `forced`）。
fn closest_effective_effort(
    requested: ReasoningEffort,
    mapping: &VerifiedReasoningMapping,
) -> Option<(ReasoningEffort, ReasoningResolutionStatus)> {
    if let Some(alias) = mapping.request_alias(requested) {
        if mapping.supported_efforts.contains(&alias) {
            return Some((alias, ReasoningResolutionStatus::Mapped));
        }
    }
    if mapping.supported_efforts.contains(&requested) {
        return Some((requested, ReasoningResolutionStatus::Mapped));
    }

    let mut supported = mapping.supported_efforts.clone();
    supported.sort_by_key(|effort| effort.rank());
    if supported.is_empty() {
        return None;
    }
    if requested == ReasoningEffort::Off {
        return None;
    }

    let lower_or_equal: Vec<ReasoningEffort> = supported
        .iter()
        .copied()
        .filter(|effort| effort.rank() <= requested.rank())
        .collect();
    let effective = lower_or_equal.last().copied().unwrap_or(supported[0]);
    Some((effective, ReasoningResolutionStatus::Capped))
}

/// 解析模型档案上的覆盖值：`auto` / 合法强度 → 原样，其余（含缺失）→ `auto`。
fn resolve_override(value: Option<&str>) -> Option<ReasoningEffort> {
    match value {
        Some(text) if text != "auto" => ReasoningEffort::parse(text),
        _ => None,
    }
}

/// 单一推理策略缝：产品意图 → provider 指令 + 用户可见回执。
///
/// 返回 `unsupported` 表示该端点没有被验证过的推理映射，调用方必须**省略**推理字段，
/// 而不是猜测一个默认值。
pub fn resolve_reasoning_policy(
    profile: &ModelCapabilityProfile<'_>,
    reasoning_override: Option<&str>,
    creative_strategy: Option<&str>,
    stage: Option<&str>,
) -> ReasoningPolicyResolution {
    let strategy = CreativeStrategy::parse(creative_strategy);
    let override_effort = resolve_override(reasoning_override);
    let source = if override_effort.is_some() {
        ReasoningSource::ModelOverride
    } else {
        ReasoningSource::ProjectStrategy
    };
    let requested = override_effort
        .unwrap_or_else(|| stage_request(strategy, GenerationReasoningStage::parse(stage)));

    let unsupported = |requested: ReasoningEffort| ReasoningPolicyResolution {
        requested,
        effective: None,
        status: ReasoningResolutionStatus::Unsupported,
        source,
        provider_directive: None,
    };

    let Some(mapping) = crate::llm::presets::resolve_model_profile_reasoning_mapping(profile)
    else {
        return unsupported(requested);
    };
    // 基线特例（reasoning-policy.ts:110-120）：本 Qwen 集成仅定义
    // 显式选择的应用预算——无手动映射时，除 model-override 显式
    // `medium` / `off` 外一律 unsupported（绝不臆造自动预算）。
    if profile.reasoning_mapping.is_none()
        && profile.model_name == Some("Qwen/Qwen3.8-27B")
        && (source != ReasoningSource::ModelOverride
            || (requested != ReasoningEffort::Medium && requested != ReasoningEffort::Off))
    {
        return unsupported(requested);
    }
    // 基线特例：仅显式、文档化的 SiliconFlow 选择受支持——保留
    // 自动请求，绝不强行提升 off/low/medium 到 high。
    if profile.reasoning_mapping.is_none()
        && mapping.adapter == ReasoningAdapter::SiliconFlowV4Thinking
        && (source != ReasoningSource::ModelOverride
            || (requested != ReasoningEffort::High && requested != ReasoningEffort::Max))
    {
        return unsupported(requested);
    }

    let Some((effective, status)) = closest_effective_effort(requested, &mapping) else {
        return unsupported(requested);
    };
    let Some(directive) = provider_directive(&mapping, effective) else {
        return unsupported(requested);
    };
    ReasoningPolicyResolution {
        requested,
        effective: Some(effective),
        status,
        source,
        provider_directive: Some(directive),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn deepseek_profile<'a>() -> ModelCapabilityProfile<'a> {
        profile(
            "deepseek",
            "openai",
            "https://api.deepseek.com",
            "deepseek-v4-flash",
        )
    }

    #[test]
    fn manual_mapping_preserves_all_efforts_and_provider_wire_values_test() {
        let openai = serde_json::json!({
            "protocol": "openai",
            "reasoningMapping": {
                "adapter": "openai-reasoning-effort",
                "supportedEfforts": ["off", "low", "medium", "high", "xhigh", "max"],
                "providerValues": {
                    "off": "none", "low": "low", "medium": "medium", "high": "high",
                    "xhigh": "xhigh", "max": "provider_max",
                },
            },
        });
        for (effort, wire) in [("off", "none"), ("xhigh", "xhigh"), ("max", "provider_max")] {
            let result = resolve_reasoning_policy(
                &ModelCapabilityProfile::from_value(&openai),
                Some(effort),
                None,
                None,
            );
            assert_eq!(result.requested.as_str(), effort);
            assert_eq!(result.status, ReasoningResolutionStatus::Mapped);
            assert_eq!(
                result.provider_directive,
                Some(ProviderReasoningDirective::OpenAiReasoningEffort {
                    reasoning_effort: wire.to_string(),
                })
            );
        }
        for (wire, expected) in [("enabled", None), ("low", Some(ReasoningEffort::Low))] {
            let deepseek = serde_json::json!({
                "protocol": "openai",
                "reasoningMapping": {
                    "adapter": "deepseek-v4-thinking", "supportedEfforts": ["high"],
                    "providerValues": { "high": wire },
                },
            });
            let result = resolve_reasoning_policy(
                &ModelCapabilityProfile::from_value(&deepseek),
                Some("high"),
                None,
                None,
            );
            assert_eq!(
                result.provider_directive,
                Some(ProviderReasoningDirective::DeepSeekV4Thinking {
                    thinking: ThinkingState::Enabled,
                    reasoning_effort: expected,
                })
            );
        }
    }

    #[test]
    fn project_strategy_without_verified_mapping_is_unsupported_test() {
        // 未在内置目录中的端点 → 没有协议证据 → 必须 unsupported
        let result = resolve_reasoning_policy(
            &profile("custom", "openai", "https://example.com/v1", "any-model"),
            None,
            Some("deep-planning"),
            Some("planning"),
        );
        assert_eq!(
            result.requested,
            ReasoningEffort::Max,
            "阶段表仍须给出请求强度"
        );
        assert_eq!(result.effective, None);
        assert_eq!(result.status, ReasoningResolutionStatus::Unsupported);
        assert_eq!(result.source, ReasoningSource::ProjectStrategy);
        assert!(result.provider_directive.is_none());
    }

    #[test]
    fn verified_deepseek_mapping_caps_and_aliases_test() {
        // medium 通过 requestAliases → high（mapped）
        let mapped =
            resolve_reasoning_policy(&deepseek_profile(), None, Some("auto"), Some("planning"));
        assert_eq!(mapped.requested, ReasoningEffort::Medium);
        assert_eq!(mapped.effective, Some(ReasoningEffort::High));
        assert_eq!(mapped.status, ReasoningResolutionStatus::Mapped);
        assert_eq!(
            mapped.provider_directive,
            Some(ProviderReasoningDirective::DeepSeekV4Thinking {
                thinking: ThinkingState::Enabled,
                reasoning_effort: Some(ReasoningEffort::High),
            })
        );

        // auto/review → high（直接支持）
        let direct =
            resolve_reasoning_policy(&deepseek_profile(), None, Some("auto"), Some("review"));
        assert_eq!(direct.effective, Some(ReasoningEffort::High));
        assert_eq!(direct.status, ReasoningResolutionStatus::Mapped);

        // auto/drafting → low（直接支持）
        let low =
            resolve_reasoning_policy(&deepseek_profile(), None, Some("auto"), Some("drafting"));
        assert_eq!(low.effective, Some(ReasoningEffort::Low));
        assert_eq!(low.status, ReasoningResolutionStatus::Mapped);
    }

    #[test]
    fn model_override_off_maps_to_disabled_thinking_test() {
        let result = resolve_reasoning_policy(
            &deepseek_profile(),
            Some("off"),
            Some("auto"),
            Some("review"),
        );
        assert_eq!(result.source, ReasoningSource::ModelOverride);
        assert_eq!(result.requested, ReasoningEffort::Off);
        assert_eq!(result.effective, Some(ReasoningEffort::Off));
        assert_eq!(
            result.provider_directive,
            Some(ProviderReasoningDirective::DeepSeekV4Thinking {
                thinking: ThinkingState::Disabled,
                reasoning_effort: None,
            })
        );
    }

    #[test]
    fn invalid_override_falls_back_to_project_strategy_test() {
        let result = resolve_reasoning_policy(
            &deepseek_profile(),
            Some("ultra"),
            Some("auto"),
            Some("review"),
        );
        assert_eq!(
            result.source,
            ReasoningSource::ProjectStrategy,
            "非法覆盖值不得生效"
        );
        assert_eq!(result.requested, ReasoningEffort::High);
    }

    #[test]
    fn openai_reasoning_effort_only_emits_documented_values_test() {
        let xai = profile("xai", "openai", "https://api.x.ai/v1", "grok-4.5");
        // deep-planning/planning 请求 max，但 grok 只支持 low/medium/high → capped 到 high
        let capped = resolve_reasoning_policy(&xai, None, Some("deep-planning"), Some("planning"));
        assert_eq!(capped.requested, ReasoningEffort::Max);
        assert_eq!(capped.effective, Some(ReasoningEffort::High));
        assert_eq!(capped.status, ReasoningResolutionStatus::Capped);
        assert_eq!(
            capped.provider_directive,
            Some(ProviderReasoningDirective::OpenAiReasoningEffort {
                reasoning_effort: "high".to_string(),
            })
        );

        // 基线 off 无映射时必须 unsupported，不得擅自开启思考。
        let disabled =
            resolve_reasoning_policy(&xai, None, Some("fluent-drafting"), Some("drafting"));
        assert_eq!(disabled.requested, ReasoningEffort::Off);
        assert_eq!(disabled.effective, None);
        assert_eq!(disabled.status, ReasoningResolutionStatus::Unsupported);
        assert!(disabled.provider_directive.is_none());
    }

    #[test]
    fn gemini_thinking_budget_is_passed_through_test() {
        let gemini = profile(
            "gemini",
            "gemini",
            "https://generativelanguage.googleapis.com",
            "gemini-2.5-flash-lite",
        );
        let result = resolve_reasoning_policy(&gemini, None, Some("auto"), Some("planning"));
        assert_eq!(result.requested, ReasoningEffort::Medium);
        assert_eq!(
            result.provider_directive,
            Some(ProviderReasoningDirective::GeminiThinkingBudget {
                thinking_budget: 8192,
            })
        );

        let off = resolve_reasoning_policy(&gemini, Some("off"), None, None);
        assert_eq!(
            off.provider_directive,
            Some(ProviderReasoningDirective::GeminiThinkingBudget { thinking_budget: 0 })
        );
    }

    #[test]
    fn stage_defaults_to_general_when_absent_test() {
        let result = resolve_reasoning_policy(&deepseek_profile(), None, None, None);
        // auto + general → low
        assert_eq!(result.requested, ReasoningEffort::Low);
    }

    #[test]
    fn preset_lookup_requires_matching_endpoint_and_protocol_test() {
        // 协议不符（openai vs gemini）→ 无映射
        let wrong_protocol = profile(
            "gemini",
            "openai",
            "https://generativelanguage.googleapis.com",
            "gemini-2.5-flash-lite",
        );
        assert_eq!(
            resolve_reasoning_policy(&wrong_protocol, None, None, None).status,
            ReasoningResolutionStatus::Unsupported
        );

        // 端点不符（自建代理）→ 无映射
        let wrong_endpoint = profile(
            "deepseek",
            "openai",
            "https://my-proxy.example.com/v1",
            "deepseek-v4-flash",
        );
        assert_eq!(
            resolve_reasoning_policy(&wrong_endpoint, None, None, None).status,
            ReasoningResolutionStatus::Unsupported
        );

        // 端点带尾斜杠 / 大小写差异仍算同一端点
        let equivalent = profile(
            "deepseek",
            "openai",
            "HTTPS://API.DEEPSEEK.COM/",
            "deepseek-v4-flash",
        );
        assert_eq!(
            resolve_reasoning_policy(&equivalent, None, None, None).status,
            ReasoningResolutionStatus::Mapped
        );
    }

    #[test]
    fn qwen_native_admission_requires_explicit_budget_selection_test() {
        // 基线 'fixed Qwen native admission'：本 Qwen 集成仅定义
        // 显式选择的应用预算。
        let qwen = profile(
            "siliconflow",
            "openai",
            "https://api.siliconflow.cn/v1",
            "Qwen/Qwen3.8-27B",
        );

        // 自动请求（项目策略）→ unsupported（绝不臆造预算）。
        let automatic = resolve_reasoning_policy(&qwen, None, Some("auto"), Some("planning"));
        assert_eq!(automatic.requested, ReasoningEffort::Medium);
        assert_eq!(automatic.status, ReasoningResolutionStatus::Unsupported);
        assert_eq!(automatic.effective, None);
        assert!(automatic.provider_directive.is_none());

        // model-override 显式 medium → 命中目录映射（off/medium，
        // medium → 16_384 独立预算）。
        let medium = resolve_reasoning_policy(&qwen, Some("medium"), None, None);
        assert_eq!(medium.status, ReasoningResolutionStatus::Mapped);
        assert_eq!(medium.effective, Some(ReasoningEffort::Medium));
        assert_eq!(
            medium.provider_directive,
            Some(ProviderReasoningDirective::OpenAiThinkingBudget {
                thinking_budget: 16_384,
            })
        );

        // model-override 显式 off → 0 = 禁用思考。
        let off = resolve_reasoning_policy(&qwen, Some("off"), None, None);
        assert_eq!(off.status, ReasoningResolutionStatus::Mapped);
        assert_eq!(
            off.provider_directive,
            Some(ProviderReasoningDirective::OpenAiThinkingBudget { thinking_budget: 0 })
        );

        // 其余强度（high/low/max）→ unsupported。
        for override_value in ["high", "low", "max"] {
            let result = resolve_reasoning_policy(&qwen, Some(override_value), None, None);
            assert_eq!(
                result.status,
                ReasoningResolutionStatus::Unsupported,
                "override {override_value} 必须 unsupported"
            );
        }
    }

    #[test]
    fn siliconflow_v4_thinking_requires_explicit_high_or_max_test() {
        // 基线 'keeps SiliconFlow output and liability unchanged for
        // explicit high/max'：仅显式 high/max 受支持。
        let flash = profile(
            "siliconflow",
            "openai",
            "https://api.siliconflow.cn/v1",
            "deepseek-ai/DeepSeek-V4-Flash",
        );

        // 自动请求（planning → medium）→ unsupported。
        let automatic = resolve_reasoning_policy(&flash, None, Some("auto"), Some("planning"));
        assert_eq!(automatic.requested, ReasoningEffort::Medium);
        assert_eq!(automatic.status, ReasoningResolutionStatus::Unsupported);
        assert!(automatic.provider_directive.is_none());

        // 显式 high / max → siliconflow-v4-thinking 指令。
        for (override_value, effort) in [
            ("high", ReasoningEffort::High),
            ("max", ReasoningEffort::Max),
        ] {
            let result = resolve_reasoning_policy(&flash, Some(override_value), None, None);
            assert_eq!(result.status, ReasoningResolutionStatus::Mapped);
            assert_eq!(result.effective, Some(effort));
            assert_eq!(
                result.provider_directive,
                Some(ProviderReasoningDirective::SiliconFlowV4Thinking {
                    reasoning_effort: effort,
                })
            );
        }

        // 显式 low / medium / off → unsupported（绝不强行提升）。
        for override_value in ["low", "medium", "off"] {
            let result = resolve_reasoning_policy(&flash, Some(override_value), None, None);
            assert_eq!(
                result.status,
                ReasoningResolutionStatus::Unsupported,
                "override {override_value} 必须 unsupported"
            );
        }
    }

    #[test]
    fn manual_reasoning_mapping_is_consumed_without_catalog_fallback_test() {
        // 基线：手动 reasoningMapping 优先——验证通过即返回，
        // 不回落内置目录（自定义端点也能解析）。
        let manual = ModelCapabilityProfile {
            provider: Some("custom"),
            protocol: Some("openai"),
            base_url: Some("https://relay.example.com/v1"),
            model_name: Some("hand-entered"),
            reasoning_mapping: Some(&serde_json::json!({
                "adapter": "openai-thinking-budget",
                "supportedEfforts": ["off", "medium"],
                "providerValues": { "off": 0, "medium": 4096 },
            })),
        };
        let result = resolve_reasoning_policy(&manual, Some("medium"), None, None);
        assert_eq!(result.status, ReasoningResolutionStatus::Mapped);
        assert_eq!(
            result.provider_directive,
            Some(ProviderReasoningDirective::OpenAiThinkingBudget {
                thinking_budget: 4096,
            })
        );

        // 非法手动映射（providerValues 键不在 supportedEfforts 内）
        // → 验证失败 → unsupported（不回落目录）。
        let invalid = ModelCapabilityProfile {
            reasoning_mapping: Some(&serde_json::json!({
                "adapter": "openai-thinking-budget",
                "supportedEfforts": ["off"],
                "providerValues": { "off": 0, "medium": 4096 },
            })),
            ..manual
        };
        assert_eq!(
            resolve_reasoning_policy(&invalid, Some("medium"), None, None).status,
            ReasoningResolutionStatus::Unsupported
        );
    }

    #[test]
    fn manual_mapping_protocol_consistency_is_enforced_test() {
        // 基线：(protocol === 'gemini') === (adapter ===
        // 'gemini-thinking-budget') 必须成立。
        let gemini_mapping = || {
            serde_json::json!({
                "adapter": "gemini-thinking-budget",
                "supportedEfforts": ["low", "medium"],
                "providerValues": { "low": 1024, "medium": 8192 },
            })
        };

        // gemini 协议 + gemini 映射 → 通过。
        let consistent = profile(
            "gemini",
            "gemini",
            "https://generativelanguage.googleapis.com",
            "gemini-2.5-flash-lite",
        );
        let consistent = ModelCapabilityProfile {
            reasoning_mapping: Some(&gemini_mapping()),
            ..consistent
        };
        let result = resolve_reasoning_policy(&consistent, Some("medium"), None, None);
        assert_eq!(result.status, ReasoningResolutionStatus::Mapped);
        assert_eq!(
            result.provider_directive,
            Some(ProviderReasoningDirective::GeminiThinkingBudget {
                thinking_budget: 8192,
            })
        );

        // gemini 协议 + openai 映射 → 拒绝（不回落目录）。
        let mismatched = ModelCapabilityProfile {
            reasoning_mapping: Some(&serde_json::json!({
                "adapter": "openai-thinking-budget",
                "supportedEfforts": ["off", "medium"],
                "providerValues": { "off": 0, "medium": 4096 },
            })),
            ..consistent
        };
        assert_eq!(
            resolve_reasoning_policy(&mismatched, Some("medium"), None, None).status,
            ReasoningResolutionStatus::Unsupported
        );

        // openai 协议 + gemini 映射 → 拒绝。
        let openai_with_gemini_mapping = ModelCapabilityProfile {
            provider: Some("custom"),
            protocol: Some("openai"),
            base_url: Some("https://relay.example.com/v1"),
            model_name: Some("hand-entered"),
            reasoning_mapping: Some(&gemini_mapping()),
        };
        assert_eq!(
            resolve_reasoning_policy(&openai_with_gemini_mapping, Some("medium"), None, None)
                .status,
            ReasoningResolutionStatus::Unsupported
        );
    }
}
