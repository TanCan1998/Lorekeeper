//! 提供方预设预算能力 —— 移植自 `src/shared/provider-presets.ts` 的
//! **预算能力解析子集**。
//!
//! 与既有 [`crate::llm::presets`]（请求特征 / 推理映射证据源）分工：
//! 本模块只拥有**预算规划用**的容量证据（`budgetCapabilities` 与
//! `capabilities` 的数值字段），不对推理、结构化输出、用量报告或
//! 其他请求特征做任何声明——提供方文档化的容量事实不构成端点
//! 请求特征支持（基线注释：`Provider-documented capacity facts that
//! do not imply request-feature support.`）。
//!
//! 移植偏差（基线字段不移植）：`ProviderPreset` 的 `provider` /
//! `displayName` / `embeddingModels` / `embeddingModelCapabilities` /
//! `budgetProviderAliases`，`ModelPreset` 的 `maxTokens` /
//! `structuredOutput` / `reasoningMapping`，以及 `ModelCapabilities`
//! 的 `reasoning` / `structuredOutput` / `usage` 布尔标记——均不参与
//! 预算能力解析（基线 `resolveModelProfileBudgetCapabilities` 只读
//! protocol / baseUrl / modelName 及上述数值字段）。其中布尔标记在
//! 基线 `validatedCapabilities` 中另有类型校验，本模块预设数据为
//! 类型化常量，该检查对常量数据恒真（不可观测）。

// 本波不接调用点（B3-1c 计划构造器消费）——模块级抑制
// dead_code 告警（对齐 `commands/generation.rs` 既有
// `#[allow(dead_code)]` 惯例与 lib.rs:47-49 的公开 API
// 约定）；接线波次移除本属性，届时未消费项将重新告警。
#![allow(dead_code)]

use crate::llm::presets::MAX_SAFE_INTEGER;

/// 预算能力证据（基线 `ModelBudgetCapabilities['evidence']`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelBudgetCapabilitiesEvidence {
    pub source_url: &'static str,
    /// 基线字面量联合 `'conservative-provider-documentation'`。
    pub calibration: &'static str,
}

/// 提供方文档化的容量事实（基线 `ModelBudgetCapabilities`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelBudgetCapabilities {
    pub context_window_tokens: u64,
    pub max_output_tokens: u64,
    pub evidence: ModelBudgetCapabilitiesEvidence,
}

/// 预算规划解析结果（基线 `ResolvedModelBudgetCapabilities`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedModelBudgetCapabilities {
    pub context_window_tokens: u64,
    pub max_output_tokens: u64,
    /// 仅专用预算证据路径携带；`capabilities` 回退路径为 `None`
    /// （基线回退返回值不含 `evidence` 字段）。
    pub evidence: Option<ModelBudgetCapabilitiesEvidence>,
}

/// 模型能力数值子集（基线 `ModelCapabilities` 中预算解析所需字段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCapabilities {
    /// `None` 表示端点未声明上下文窗口（对齐基线的显式 `null`）。
    pub context_window_tokens: Option<u64>,
    pub max_output_tokens: u64,
}

/// 单个模型预设的预算相关字段（基线 `ModelPreset` 子集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelPreset {
    pub name: &'static str,
    pub budget_capabilities: Option<ModelBudgetCapabilities>,
    pub capabilities: Option<ModelCapabilities>,
}

/// 单个服务商预设的预算相关字段（基线 `ProviderPreset` 子集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderPreset {
    pub protocol: &'static str,
    pub base_url: &'static str,
    /// 基线 `budgetCapabilityBaseUrls`：额外被认可供应预算能力证据的
    /// 精确官方端点。
    pub budget_capability_base_urls: &'static [&'static str],
    pub models: &'static [ModelPreset],
}

/// 预算能力解析所需的最小档案字段（基线 `ModelCapabilityProfile` 的
/// 预算子集；`provider` / `maxTokens` / `capabilities` /
/// `reasoningMapping` 不参与预算能力解析，不移植）。
///
/// 字段为 `Option` 对齐基线的 `unknown` 运行时校验：`None` 等价于
/// 基线的非字符串值（`typeof x !== 'string'` 守卫拒绝）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelBudgetCapabilityProfile<'a> {
    pub protocol: Option<&'a str>,
    pub base_url: Option<&'a str>,
    pub model_name: Option<&'a str>,
}

const OPENAI_GPT_4_1_BUDGET: ModelBudgetCapabilities = ModelBudgetCapabilities {
    context_window_tokens: 1_047_576,
    max_output_tokens: 32_768,
    evidence: ModelBudgetCapabilitiesEvidence {
        source_url: "https://developers.openai.com/api/docs/models/gpt-4.1",
        calibration: "conservative-provider-documentation",
    },
};

const OPENAI_O3_BUDGET: ModelBudgetCapabilities = ModelBudgetCapabilities {
    context_window_tokens: 200_000,
    max_output_tokens: 100_000,
    evidence: ModelBudgetCapabilitiesEvidence {
        source_url: "https://developers.openai.com/api/docs/models/o3",
        calibration: "conservative-provider-documentation",
    },
};

const OPENAI_GPT_4O_BUDGET: ModelBudgetCapabilities = ModelBudgetCapabilities {
    context_window_tokens: 128_000,
    max_output_tokens: 16_384,
    evidence: ModelBudgetCapabilitiesEvidence {
        source_url: "https://developers.openai.com/api/docs/models/gpt-4o",
        calibration: "conservative-provider-documentation",
    },
};

const OPENAI_GPT_4O_MINI_BUDGET: ModelBudgetCapabilities = ModelBudgetCapabilities {
    context_window_tokens: 128_000,
    max_output_tokens: 16_384,
    evidence: ModelBudgetCapabilitiesEvidence {
        source_url: "https://developers.openai.com/api/docs/models/gpt-4o-mini",
        calibration: "conservative-provider-documentation",
    },
};

const OPENAI_GPT_4_TURBO_BUDGET: ModelBudgetCapabilities = ModelBudgetCapabilities {
    context_window_tokens: 128_000,
    max_output_tokens: 4_096,
    evidence: ModelBudgetCapabilitiesEvidence {
        source_url: "https://developers.openai.com/api/docs/models/gpt-4-turbo",
        calibration: "conservative-provider-documentation",
    },
};

const OPENAI_GPT_3_5_TURBO_BUDGET: ModelBudgetCapabilities = ModelBudgetCapabilities {
    context_window_tokens: 16_385,
    max_output_tokens: 4_096,
    evidence: ModelBudgetCapabilitiesEvidence {
        source_url: "https://developers.openai.com/api/docs/models/gpt-3.5-turbo",
        calibration: "conservative-provider-documentation",
    },
};

const SILICONFLOW_DEEPSEEK_V4_FLASH_BUDGET: ModelBudgetCapabilities = ModelBudgetCapabilities {
    // 提供方页面发布四舍五入的 1049K / 393K 值。使用保守的
    // 十进制界限而非虚构二进制界限。
    context_window_tokens: 1_000_000,
    max_output_tokens: 393_000,
    evidence: ModelBudgetCapabilitiesEvidence {
        source_url: "https://www.siliconflow.com/models/deepseek-v4-flash",
        calibration: "conservative-provider-documentation",
    },
};

const SILICONFLOW_DEEPSEEK_V4_PRO_BUDGET: ModelBudgetCapabilities = ModelBudgetCapabilities {
    context_window_tokens: 1_000_000,
    max_output_tokens: 393_000,
    evidence: ModelBudgetCapabilitiesEvidence {
        source_url: "https://www.siliconflow.com/models/deepseek-v4-pro",
        calibration: "conservative-provider-documentation",
    },
};

const SILICONFLOW_QWEN_3_8_27B_BUDGET: ModelBudgetCapabilities = ModelBudgetCapabilities {
    context_window_tokens: 262_144,
    // 保守的支持输出，而非模型最大值的声明。
    max_output_tokens: 16_384,
    evidence: ModelBudgetCapabilitiesEvidence {
        source_url: "https://www.siliconflow.cn/models",
        calibration: "conservative-provider-documentation",
    },
};

const BIGMODEL_GLM_5_3_BUDGET: ModelBudgetCapabilities = ModelBudgetCapabilities {
    context_window_tokens: 1_000_000,
    max_output_tokens: 131_072,
    evidence: ModelBudgetCapabilitiesEvidence {
        source_url: "https://docs.bigmodel.cn/cn/guide/models/text/glm-5.3",
        calibration: "conservative-provider-documentation",
    },
};

/// 无预算 / 能力数据的模型条目（基线仅有 `name` + 旧 `maxTokens`）。
const fn plain(name: &'static str) -> ModelPreset {
    ModelPreset {
        name,
        budget_capabilities: None,
        capabilities: None,
    }
}

/// 基线 `BUILTIN_PRESETS`（`createProviderCatalog()`）的预算能力解析
/// 子集——仅保留解析所需字段，条目顺序与基线目录一致。
///
/// 注：基线 openai 预设的 `gpt-4.1` 与 `gpt-4.1-2025-04-14` 携带
/// 相同的预算证据（同一模型文档源），此处同样共享常量。
pub const BUILTIN_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        protocol: "openai",
        base_url: "https://api.openai.com",
        // 持久化档案记录带版本的端点；两种拼写是同一官方主机，
        // 因此仅预算证据可通过它匹配。
        budget_capability_base_urls: &["https://api.openai.com/v1"],
        models: &[
            ModelPreset {
                name: "gpt-4.1",
                // 保留旧的运营默认值；预算证据独立存在，从不作为
                // 请求特征支持公示。
                budget_capabilities: Some(OPENAI_GPT_4_1_BUDGET),
                capabilities: None,
            },
            ModelPreset {
                name: "gpt-4.1-2025-04-14",
                budget_capabilities: Some(OPENAI_GPT_4_1_BUDGET),
                capabilities: None,
            },
            ModelPreset {
                name: "o3",
                budget_capabilities: Some(OPENAI_O3_BUDGET),
                capabilities: None,
            },
            ModelPreset {
                name: "gpt-4o",
                budget_capabilities: Some(OPENAI_GPT_4O_BUDGET),
                capabilities: None,
            },
            ModelPreset {
                name: "gpt-4o-mini",
                budget_capabilities: Some(OPENAI_GPT_4O_MINI_BUDGET),
                capabilities: None,
            },
            ModelPreset {
                name: "gpt-4-turbo",
                budget_capabilities: Some(OPENAI_GPT_4_TURBO_BUDGET),
                capabilities: None,
            },
            ModelPreset {
                name: "gpt-3.5-turbo",
                budget_capabilities: Some(OPENAI_GPT_3_5_TURBO_BUDGET),
                capabilities: None,
            },
        ],
    },
    ProviderPreset {
        protocol: "openai",
        base_url: "https://api.x.ai/v1",
        budget_capability_base_urls: &[],
        models: &[ModelPreset {
            name: "grok-4.5",
            // 保留 `maxTokens` 的运营默认值；新设置表单使用精确的
            // 能力字段名。此处仅移植数值字段。
            budget_capabilities: None,
            capabilities: Some(ModelCapabilities {
                context_window_tokens: Some(500_000),
                max_output_tokens: 8_192,
            }),
        }],
    },
    ProviderPreset {
        protocol: "openai",
        base_url: "https://api.siliconflow.cn/v1",
        budget_capability_base_urls: &["https://api.siliconflow.com/v1"],
        models: &[
            ModelPreset {
                name: "deepseek-ai/DeepSeek-V4-Flash",
                budget_capabilities: Some(SILICONFLOW_DEEPSEEK_V4_FLASH_BUDGET),
                capabilities: None,
            },
            ModelPreset {
                name: "deepseek-ai/DeepSeek-V4-Pro",
                budget_capabilities: Some(SILICONFLOW_DEEPSEEK_V4_PRO_BUDGET),
                capabilities: None,
            },
            ModelPreset {
                name: "Qwen/Qwen3.8-27B",
                budget_capabilities: Some(SILICONFLOW_QWEN_3_8_27B_BUDGET),
                capabilities: None,
            },
        ],
    },
    ProviderPreset {
        protocol: "openai",
        base_url: "https://text.novelai.net/oa",
        budget_capability_base_urls: &[],
        models: &[],
    },
    ProviderPreset {
        protocol: "openai",
        base_url: "https://api.deepseek.com",
        budget_capability_base_urls: &[],
        models: &[
            ModelPreset {
                name: "deepseek-v4-flash",
                budget_capabilities: None,
                capabilities: Some(ModelCapabilities {
                    context_window_tokens: Some(1_000_000),
                    max_output_tokens: 384_000,
                }),
            },
            ModelPreset {
                name: "deepseek-v4-pro",
                budget_capabilities: None,
                capabilities: Some(ModelCapabilities {
                    context_window_tokens: Some(1_000_000),
                    max_output_tokens: 384_000,
                }),
            },
        ],
    },
    ProviderPreset {
        protocol: "openai",
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        budget_capability_base_urls: &[],
        models: &[
            plain("glm-4.5"),
            plain("glm-4.5-air"),
            plain("glm-4.6"),
            plain("glm-4.7"),
            plain("glm-4.7-flashx"),
            plain("glm-5-turbo"),
            plain("glm-5"),
            ModelPreset {
                name: "glm-5.3",
                budget_capabilities: Some(BIGMODEL_GLM_5_3_BUDGET),
                capabilities: None,
            },
        ],
    },
    ProviderPreset {
        protocol: "gemini",
        base_url: "https://generativelanguage.googleapis.com",
        budget_capability_base_urls: &[],
        models: &[
            ModelPreset {
                name: "gemini-2.5-flash-lite",
                budget_capabilities: None,
                capabilities: Some(ModelCapabilities {
                    context_window_tokens: Some(1_048_576),
                    max_output_tokens: 65_536,
                }),
            },
            plain("gemini-3.1-pro-preview"),
            plain("gemini-3-flash-preview"),
        ],
    },
    ProviderPreset {
        protocol: "openai",
        base_url: "http://localhost:11434/v1",
        budget_capability_base_urls: &[],
        models: &[
            plain("qwen3-14b-abliterated-novel-q4"),
            plain("llama3.3"),
            plain("llama3.2"),
            plain("qwen2.5"),
            plain("qwen2.5-coder"),
            plain("mistral"),
            plain("phi4"),
            plain("gemma3"),
        ],
    },
    ProviderPreset {
        protocol: "openai",
        base_url: "",
        budget_capability_base_urls: &[],
        models: &[],
    },
];

/// 基线 `normalizedOfficialBaseUrl`：仅接受 http/https、无凭据、
/// 无查询、无片段；尾斜杠归一。
///
/// 含基线特例：`https://api.deepseek.com/v1` 与裸端点
/// `https://api.deepseek.com` 归一为同一字符串（预设 baseUrl
/// 不带 `/v1`）。
pub fn normalized_official_base_url(value: Option<&str>) -> Option<String> {
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
    // 基线特例：DeepSeek 的版本化端点与裸端点是同一官方主机
    // （`origin === 'https://api.deepseek.com'`，默认端口省略）。
    if endpoint.scheme() == "https"
        && endpoint.host_str() == Some("api.deepseek.com")
        && endpoint.port_or_known_default() == Some(443)
        && endpoint.path() == "/v1"
    {
        endpoint.set_path("/");
    }
    let text = endpoint.to_string();
    Some(text.strip_suffix('/').unwrap_or(&text).to_string())
}

/// 基线 `validatedCapabilities`（预算子集：仅校验数值字段）。
///
/// 偏差：基线另校验 `reasoning` / `structuredOutput` / `usage`
/// 三个布尔标记的类型——本模块预设数据为类型化常量，布尔标记
/// 不移植，该检查对常量数据恒真（不可观测）。
pub fn validated_capabilities(value: Option<&ModelCapabilities>) -> Option<ModelCapabilities> {
    let candidate = value?;
    let valid_context = match candidate.context_window_tokens {
        None => true,
        Some(tokens) => tokens > 0 && tokens <= MAX_SAFE_INTEGER,
    };
    if !valid_context
        || candidate.max_output_tokens == 0
        || candidate.max_output_tokens > MAX_SAFE_INTEGER
    {
        return None;
    }
    Some(*candidate)
}

/// 基线 `validatedBudgetCapabilities`。
pub fn validated_budget_capabilities(
    value: Option<&ModelBudgetCapabilities>,
) -> Option<ModelBudgetCapabilities> {
    let candidate = value?;
    if candidate.context_window_tokens == 0
        || candidate.context_window_tokens > MAX_SAFE_INTEGER
        || candidate.max_output_tokens == 0
        || candidate.max_output_tokens > MAX_SAFE_INTEGER
        || candidate.evidence.source_url.is_empty()
        || candidate.evidence.calibration != "conservative-provider-documentation"
    {
        return None;
    }
    Some(*candidate)
}

/// 基线 `matchesBudgetCapabilityEndpoint`：档案端点与预设的
/// 官方端点（baseUrl 或 `budgetCapabilityBaseUrls`）之一精确匹配。
pub fn matches_budget_capability_endpoint(preset: &ProviderPreset, base_url: Option<&str>) -> bool {
    let Some(endpoint) = normalized_official_base_url(base_url) else {
        return false;
    };
    preset
        .budget_capability_base_urls
        .iter()
        .copied()
        .chain(core::iter::once(preset.base_url))
        .any(|candidate| {
            normalized_official_base_url(Some(candidate)).as_deref() == Some(endpoint.as_str())
        })
}

/// 基线 `resolveModelProfileBudgetCapabilities`。
///
/// 解析预算规划用的容量证据，**不**对推理、结构化输出、用量报告
/// 或其他请求特征做任何声明。用户存储的能力与输出上限属于运行
/// 策略，永远不能覆盖该结果。
pub fn resolve_model_profile_budget_capabilities(
    profile: &ModelBudgetCapabilityProfile<'_>,
) -> Option<ResolvedModelBudgetCapabilities> {
    let protocol = profile.protocol?;
    let model_name = profile.model_name?.trim();
    let preset = BUILTIN_PRESETS.iter().find(|candidate| {
        candidate.protocol == protocol
            && matches_budget_capability_endpoint(candidate, profile.base_url)
    })?;

    let model = preset
        .models
        .iter()
        .find(|candidate| candidate.name == model_name);
    // Qwen 证据限定于已发布的 CN 端点，而非 V4 别名。
    if model_name == "Qwen/Qwen3.8-27B"
        && normalized_official_base_url(profile.base_url).as_deref()
            != Some("https://api.siliconflow.cn/v1")
    {
        return None;
    }
    if let Some(dedicated) = validated_budget_capabilities(
        model.and_then(|candidate| candidate.budget_capabilities.as_ref()),
    ) {
        return Some(ResolvedModelBudgetCapabilities {
            context_window_tokens: dedicated.context_window_tokens,
            max_output_tokens: dedicated.max_output_tokens,
            evidence: Some(dedicated.evidence),
        });
    }

    let capabilities =
        validated_capabilities(model.and_then(|candidate| candidate.capabilities.as_ref()))?;
    let context_window_tokens = capabilities.context_window_tokens?;
    Some(ResolvedModelBudgetCapabilities {
        context_window_tokens,
        max_output_tokens: capabilities.max_output_tokens,
        evidence: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile<'a>(
        protocol: &'a str,
        base_url: &'a str,
        model_name: &'a str,
    ) -> ModelBudgetCapabilityProfile<'a> {
        ModelBudgetCapabilityProfile {
            protocol: Some(protocol),
            base_url: Some(base_url),
            model_name: Some(model_name),
        }
    }

    #[test]
    fn dedicated_budget_evidence_resolves_for_builtin_endpoints_test() {
        // openai 预设：持久化档案记录带版本的端点，仅预算证据可匹配。
        let resolved = resolve_model_profile_budget_capabilities(&profile(
            "openai",
            "https://api.openai.com/v1",
            "gpt-4.1",
        ))
        .expect("gpt-4.1 必须通过 /v1 端点解析预算证据");
        assert_eq!(resolved.context_window_tokens, 1_047_576);
        assert_eq!(resolved.max_output_tokens, 32_768);
        assert_eq!(
            resolved.evidence.expect("专用证据必须携带来源").source_url,
            "https://developers.openai.com/api/docs/models/gpt-4.1"
        );

        // 裸端点同样匹配预设 baseUrl。
        assert!(resolve_model_profile_budget_capabilities(&profile(
            "openai",
            "https://api.openai.com",
            "o3",
        ))
        .is_some());

        // bigmodel：glm-5.3 携带专用预算证据。
        let glm = resolve_model_profile_budget_capabilities(&profile(
            "openai",
            "https://open.bigmodel.cn/api/paas/v4",
            "glm-5.3",
        ))
        .expect("glm-5.3 必须解析预算证据");
        assert_eq!(glm.context_window_tokens, 1_000_000);
        assert_eq!(glm.max_output_tokens, 131_072);
    }

    #[test]
    fn siliconflow_budget_evidence_matches_both_official_endpoints_test() {
        // 预设 baseUrl（CN 端点）。
        let cn = resolve_model_profile_budget_capabilities(&profile(
            "openai",
            "https://api.siliconflow.cn/v1",
            "deepseek-ai/DeepSeek-V4-Flash",
        ))
        .expect("CN 端点必须匹配");
        assert_eq!(cn.context_window_tokens, 1_000_000);
        assert_eq!(cn.max_output_tokens, 393_000);

        // budgetCapabilityBaseUrls（国际端点）。
        let com = resolve_model_profile_budget_capabilities(&profile(
            "openai",
            "https://api.siliconflow.com/v1",
            "deepseek-ai/DeepSeek-V4-Flash",
        ))
        .expect("国际端点必须通过 budgetCapabilityBaseUrls 匹配");
        assert_eq!(com.context_window_tokens, 1_000_000);
        assert_eq!(com.max_output_tokens, 393_000);

        // 端点匹配与档案 provider 字段无关（基线只比对 protocol）。
        let via_openai_provider = resolve_model_profile_budget_capabilities(&profile(
            "openai",
            "https://api.siliconflow.cn/v1",
            "deepseek-ai/DeepSeek-V4-Flash",
        ))
        .expect("protocol 匹配即可");
        assert_eq!(via_openai_provider.max_output_tokens, 393_000);
    }

    #[test]
    fn capability_fallback_resolves_without_evidence_test() {
        // xai grok-4.5：仅 capabilities 数值字段（回退路径，无证据）。
        let grok = resolve_model_profile_budget_capabilities(&profile(
            "openai",
            "https://api.x.ai/v1",
            "grok-4.5",
        ))
        .expect("grok-4.5 必须经 capabilities 回退解析");
        assert_eq!(grok.context_window_tokens, 500_000);
        assert_eq!(grok.max_output_tokens, 8_192);
        assert_eq!(grok.evidence, None, "回退路径不得携带证据");

        // deepseek：裸端点 + /v1 别名（基线特例归一）均匹配。
        for base_url in [
            "https://api.deepseek.com",
            "https://api.deepseek.com/v1",
            "https://api.deepseek.com/v1/",
        ] {
            let resolved = resolve_model_profile_budget_capabilities(&profile(
                "openai",
                base_url,
                "deepseek-v4-flash",
            ))
            .unwrap_or_else(|| panic!("{base_url} 必须与裸端点归一匹配"));
            assert_eq!(resolved.context_window_tokens, 1_000_000);
            assert_eq!(resolved.max_output_tokens, 384_000);
        }

        // gemini 原生协议端点。
        let gemini = resolve_model_profile_budget_capabilities(&profile(
            "gemini",
            "https://generativelanguage.googleapis.com",
            "gemini-2.5-flash-lite",
        ))
        .expect("gemini 端点必须匹配");
        assert_eq!(gemini.context_window_tokens, 1_048_576);
        assert_eq!(gemini.max_output_tokens, 65_536);
    }

    #[test]
    fn qwen_evidence_is_scoped_to_the_cn_endpoint_test() {
        // 基线特例：modelName === 'Qwen/Qwen3.8-27B' 且归一化端点不是
        // 'https://api.siliconflow.cn/v1' 时返回 undefined。
        let cn = resolve_model_profile_budget_capabilities(&profile(
            "openai",
            "https://api.siliconflow.cn/v1",
            "Qwen/Qwen3.8-27B",
        ))
        .expect("CN 端点必须解析 Qwen 证据");
        assert_eq!(cn.context_window_tokens, 262_144);
        assert_eq!(cn.max_output_tokens, 16_384);

        // 国际端点（V4 别名）→ 无证据。
        assert!(resolve_model_profile_budget_capabilities(&profile(
            "openai",
            "https://api.siliconflow.com/v1",
            "Qwen/Qwen3.8-27B",
        ))
        .is_none());

        // 任何其他 openai 端点 → 无证据。
        assert!(resolve_model_profile_budget_capabilities(&profile(
            "openai",
            "https://api.openai.com/v1",
            "Qwen/Qwen3.8-27B",
        ))
        .is_none());
    }

    #[test]
    fn unverified_endpoints_and_unknown_models_resolve_no_evidence_test() {
        // 自建代理 → 无证据（即使模型名与内置 slug 相同）。
        assert!(resolve_model_profile_budget_capabilities(&profile(
            "openai",
            "https://proxy.example.test/v1",
            "grok-4.5",
        ))
        .is_none());

        // 协议不符 → 无证据。
        assert!(resolve_model_profile_budget_capabilities(&profile(
            "gemini",
            "https://api.deepseek.com",
            "deepseek-v4-flash",
        ))
        .is_none());

        // 已验证端点上的未知 slug → 无证据。
        assert!(resolve_model_profile_budget_capabilities(&profile(
            "openai",
            "https://api.openai.com/v1",
            "gpt-test",
        ))
        .is_none());

        // 非法端点（凭据 / 查询 / 片段 / 非 http）→ 无证据。
        for base_url in [
            "https://user:secret@api.openai.com/v1",
            "https://api.openai.com/v1?key=1",
            "https://api.openai.com/v1#frag",
            "ftp://api.openai.com/v1",
            "not-a-url",
        ] {
            assert!(
                resolve_model_profile_budget_capabilities(&profile("openai", base_url, "gpt-4.1",))
                    .is_none(),
                "{base_url} 必须被端点规范化拒绝"
            );
        }
    }

    #[test]
    fn endpoint_normalization_accepts_case_and_trailing_slash_test() {
        // （端点，模型 slug）——slug 必须与端点所属预设一致。
        for (base_url, model_name) in [
            ("https://API.DEEPSEEK.COM", "deepseek-v4-flash"),
            ("  https://api.deepseek.com  ", "deepseek-v4-flash"),
            ("https://api.deepseek.com/v1/", "deepseek-v4-flash"),
            (
                "https://api.siliconflow.cn/v1/",
                "deepseek-ai/DeepSeek-V4-Flash",
            ),
        ] {
            let resolved =
                resolve_model_profile_budget_capabilities(&profile("openai", base_url, model_name))
                    .unwrap_or_else(|| panic!("{base_url} 必须与内置端点等价"));
            assert_eq!(resolved.context_window_tokens, 1_000_000);
        }
    }

    #[test]
    fn missing_profile_fields_resolve_no_evidence_test() {
        // protocol / modelName 非字符串（基线 typeof 守卫）。
        assert!(
            resolve_model_profile_budget_capabilities(&ModelBudgetCapabilityProfile {
                protocol: None,
                base_url: Some("https://api.openai.com/v1"),
                model_name: Some("gpt-4.1"),
            })
            .is_none()
        );
        assert!(
            resolve_model_profile_budget_capabilities(&ModelBudgetCapabilityProfile {
                protocol: Some("openai"),
                base_url: Some("https://api.openai.com/v1"),
                model_name: None,
            })
            .is_none()
        );
    }
}
