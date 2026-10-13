//! 预算失败诊断格式化（1:1 移植
//! `src/services/generation/prompt-budget-failure.ts`）。
//!
//! 仅格式化安全会计字段；提示词文本、端点详情与密钥
//! 按类型永不越界。
//!
//! 移植说明：
//! - `PromptBudgetReport` / `PromptBudgetSectionReport` 为
//!   `generation-harness.ts:53-73` 的最小忠实投影
//!   （先例 `legacy_roster_generation_pure.rs`）；
//!   generation-harness 整体移植波次统一时替换；
//! - `promptBudgetFailureFromError` 基线以 `error instanceof
//!   PromptBudgetExceededError` 分流；Rust 无异常类，签名
//!   调整为接收 `Option<&PromptBudgetReport>`（调用方负责
//!   捕获，`None` 对应基线非该类错误时返回 `undefined`）；
//! - `Locale` / `WritingLanguage` 基线为字面量联合，Rust
//!   侧无对应枚举，以 `&str`（`"zh-CN"` / `"en-US"`）传递，
//!   与既有 `agent_generation_context` 的 `ui_locale` 惯例
//!   一致；
//! - 基线 `new Intl.NumberFormat(locale).format(value)`：
//!   `zh-CN` 与 `en-US` 的十进制分组分隔符均为半角逗号，
//!   故两种 locale 共享千分位实现（语义等价，非偏差）。

use crate::services::task_budget_planner::{
    GenerationBudgetActualState, GenerationBudgetDiagnostic, TaskBudgetDecision,
    TaskBudgetReasonCode,
};

/// 提示词预算耗尽失败码（基线 `PROMPT_BUDGET_FAILURE_CODE`）。
pub const PROMPT_BUDGET_FAILURE_CODE: &str = "prompt_budget_exhausted";

/// 基线 `PromptBudgetFailureCode = typeof PROMPT_BUDGET_FAILURE_CODE`
/// ——Rust 侧即 `&'static str` 取值 `PROMPT_BUDGET_FAILURE_CODE`。

/// 安全提示词诊断区段（基线 `PromptBudgetSectionReport`，
/// generation-harness.ts:53-60 的最小忠实投影）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptBudgetSectionReport {
    pub section_name: String,
    pub utf8_bytes: i64,
    /// 安全的人类可读标识；提示词文本永不复制到此处。
    pub display_name: Option<String>,
}

/// 安全提示词诊断报告（基线 `PromptBudgetReport`，
/// generation-harness.ts:62-73 的最小忠实投影）。
/// 提示词文本与提供方端点详情按类型不存在。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptBudgetReport {
    pub total_utf8_bytes: i64,
    pub limit_utf8_bytes: i64,
    /// 当前报告存在；可选以保持旧版本持久化报告可读。
    pub context_window_tokens: Option<i64>,
    /// 当前报告存在；可选以保持旧版本持久化报告可读。
    pub estimated_input_tokens: Option<i64>,
    pub reserved_output_tokens: i64,
    pub sections: Vec<PromptBudgetSectionReport>,
    pub model_id: String,
    pub error_code: String,
}

/// 提示词预算失败载荷（基线 `promptBudgetFailureFromError`
/// 的返回类型 `{ failureCode; message; report }`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptBudgetFailure {
    pub failure_code: String,
    pub message: String,
    pub report: PromptBudgetReport,
}

/// 区段双语标签（基线 `SECTION_LABELS`）。`None` 表示基线
/// 无表项（回退到「其他结构化上下文」）。
fn section_labels(section_name: &str) -> Option<(&'static str, &'static str)> {
    match section_name {
        "global-guidance" => Some(("全局指导", "Global guidance")),
        "step-guidance" => Some(("步骤指导", "Step guidance")),
        "reference-works" => Some(("参考作品", "Reference works")),
        "knowledge-base" => Some(("知识库", "Knowledge base")),
        "story-premise" => Some(("故事前提", "Story premise")),
        "genre" => Some(("作品类型", "Genre")),
        "protagonist-profile" => Some(("主角设定", "Protagonist profile")),
        "identity-manifest" => Some(("角色身份清单", "Character identity manifest")),
        "validated-prefix" => Some(("已验证角色详情", "Validated character details")),
        "batch-slot-ids" => Some(("本批角色标识", "Batch character identifiers")),
        "architecture" => Some(("故事架构", "Story architecture")),
        "previous-blueprints" => Some(("已有章节蓝图", "Previous chapter blueprints")),
        "target-chapter" => Some(("目标章节", "Target chapter")),
        "project-chapter-count" => Some(("项目章节数", "Project chapter count")),
        "repair-contract" => Some(("结构化修复合同", "Structured repair contract")),
        "repair-candidate" => Some(("待修复候选", "Repair candidate")),
        "system-instructions" => Some(("系统指令", "System instructions")),
        "continuation-request" => Some(("续写请求", "Continuation request")),
        "prompt-overhead" => Some(("模板与结构开销", "Template and structure overhead")),
        _ => None,
    }
}

/// 基线 `sectionLabel`。
fn section_label(section: &PromptBudgetSectionReport, locale: &str) -> String {
    if section.section_name == "writing-skill" {
        let generic = if locale == "zh-CN" {
            "写作 Skill"
        } else {
            "Writing Skill"
        };
        match &section.display_name {
            None => generic.to_string(),
            Some(display_name) => {
                if locale == "zh-CN" {
                    format!("{generic}：{display_name}")
                } else {
                    format!("{generic}: {display_name}")
                }
            }
        }
    } else {
        match section_labels(&section.section_name) {
            None => {
                if locale == "zh-CN" {
                    "其他结构化上下文".to_string()
                } else {
                    "Other structured context".to_string()
                }
            }
            Some((zh_cn, en_us)) => {
                if locale == "zh-CN" {
                    zh_cn.to_string()
                } else {
                    en_us.to_string()
                }
            }
        }
    }
}

/// 基线 `formatInteger`（`new Intl.NumberFormat(locale)`：
/// 两种支持 locale 的十进制分组分隔符均为半角逗号）。
fn format_integer(value: i64, _locale: &str) -> String {
    let negative = value < 0;
    let digits = value.unsigned_abs().to_string();
    let length = digits.len();
    let mut grouped = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (length - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(character);
    }
    if negative {
        grouped.insert(0, '-');
    }
    grouped
}

/// 生成失败分类（基线 `GenerationFailureCategory`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationFailureCategory {
    CapacityPreflight,
    BudgetExhausted,
    ProviderLength,
    Network,
    Storage,
    Cancelled,
    Unknown,
}

const CAPACITY_FAILURES: [&str; 3] = [
    "GENERATION_INPUT_CAPACITY_EXCEEDED",
    "TASK_BUDGET_CAPACITY_CONFLICT",
    "PROMPT_BUDGET_EXHAUSTED",
];

const BUDGET_FAILURES: [&str; 4] = [
    "ROOT_BUDGET_EXHAUSTED",
    "ATTEMPT_BUDGET_EXHAUSTED",
    "REQUESTED_TOKEN_BUDGET_EXHAUSTED",
    "DEADLINE_EXHAUSTED",
];

const NETWORK_FAILURES: [&str; 4] = ["NETWORK_ERROR", "ECONNRESET", "ETIMEDOUT", "FETCH_FAILED"];

const STORAGE_FAILURES: [&str; 6] = [
    "GENERATION_STORAGE_FAILED",
    "PERSISTENCE_FAILED",
    "DB_WRITE_FAILED",
    "SQLITE_FULL",
    "ENOSPC",
    "EACCES",
];

const CANCELLED_FAILURES: [&str; 2] = ["GENERATION_CANCELLED", "CANCELLED"];

/// 基线 `classifyGenerationFailure`。
pub fn classify_generation_failure(
    failure_code: Option<&str>,
    finish_reason: Option<&str>,
) -> GenerationFailureCategory {
    let code = failure_code.unwrap_or("").trim().to_uppercase();
    let finish = finish_reason.unwrap_or("").trim().to_lowercase();
    if finish == "length" || finish == "max_tokens" || finish == "max_output_tokens" {
        return GenerationFailureCategory::ProviderLength;
    }
    if finish == "cancelled" || CANCELLED_FAILURES.contains(&code.as_str()) {
        return GenerationFailureCategory::Cancelled;
    }
    // IPC 可能把稳定码包在调用前缀中，预算 owner 在冒号后
    // 追加非敏感原因。两者仍是预检事实。主预检抛出
    // `TASK_BUDGET_SCOPE_SPLIT_REQUIRED`；批执行器消费该
    // 精确拼写，故此处两者都接受。
    if code.contains("TASK_BUDGET_CAPACITY_CONFLICT")
        || code.contains("TASK_BUDGET_SPLIT_REQUIRED")
        || code.contains("TASK_BUDGET_SCOPE_SPLIT_REQUIRED")
    {
        return GenerationFailureCategory::CapacityPreflight;
    }
    if CAPACITY_FAILURES.contains(&code.as_str()) {
        return GenerationFailureCategory::CapacityPreflight;
    }
    if BUDGET_FAILURES.contains(&code.as_str()) {
        return GenerationFailureCategory::BudgetExhausted;
    }
    if NETWORK_FAILURES.contains(&code.as_str()) {
        return GenerationFailureCategory::Network;
    }
    if STORAGE_FAILURES.contains(&code.as_str()) {
        return GenerationFailureCategory::Storage;
    }
    GenerationFailureCategory::Unknown
}

/// 原因码双语标签（基线 `REASON_LABELS`）。
fn reason_labels(code: TaskBudgetReasonCode) -> (&'static str, &'static str) {
    match code {
        TaskBudgetReasonCode::TaskDemand => ("任务完整范围", "Complete task scope"),
        TaskBudgetReasonCode::OutputAllocationCeiling => (
            "物理输出使用可用上限",
            "Physical output uses the available ceiling",
        ),
        TaskBudgetReasonCode::InputUpperBound => ("输入估计上界", "Input estimate upper bound"),
        TaskBudgetReasonCode::SafetyMargin => ("上下文安全余量", "Context safety margin"),
        TaskBudgetReasonCode::ModelContextCap => ("模型上下文剩余空间", "Remaining model context"),
        TaskBudgetReasonCode::ModelOutputCap => ("模型输出上限", "Model output limit"),
        TaskBudgetReasonCode::UserContextCap => ("用户上下文设置", "User context setting"),
        TaskBudgetReasonCode::UserOutputCap => ("用户输出设置", "User output setting"),
        TaskBudgetReasonCode::RootOutputCap => ("单次请求安全上限", "Per-request safety limit"),
        TaskBudgetReasonCode::RootRemainingCap => ("父任务剩余额度", "Parent task remainder"),
        TaskBudgetReasonCode::ProtocolReasoningReserve => {
            ("独立推理预留", "Separate reasoning reserve")
        }
        TaskBudgetReasonCode::ProtocolTotalBound => {
            ("协议总开销上限", "Protocol total-liability limit")
        }
        TaskBudgetReasonCode::ModelCapabilityUnknown => {
            ("模型能力未经验证", "Unverified model capability")
        }
        TaskBudgetReasonCode::LiabilityBoundUnknown => {
            ("协议总开销无法约束", "Unbounded protocol liability")
        }
        TaskBudgetReasonCode::RequiredInputCapacityConflict => (
            "必需输入与输出空间冲突",
            "Required input conflicts with output capacity",
        ),
        TaskBudgetReasonCode::SingleItemCapacityConflict => {
            ("单个结构项无法容纳", "One structured item does not fit")
        }
        TaskBudgetReasonCode::DraftSegmentationDisabled => (
            "此正文任务不能安全分段",
            "This draft task cannot be segmented safely",
        ),
        TaskBudgetReasonCode::ScopeSplit => ("已缩小本次范围", "This request uses a smaller scope"),
    }
}

/// 基线 `reasonLabel`。
fn reason_label(code: TaskBudgetReasonCode, locale: &str) -> &'static str {
    let (zh_cn, en_us) = reason_labels(code);
    if locale == "zh-CN" {
        zh_cn
    } else {
        en_us
    }
}

/// 基线 `failureCategoryLabel`。
fn failure_category_label(category: GenerationFailureCategory, locale: &str) -> &'static str {
    let labels = match category {
        GenerationFailureCategory::CapacityPreflight => ("容量预检", "Capacity preflight"),
        GenerationFailureCategory::BudgetExhausted => ("父任务预算", "Parent task budget"),
        GenerationFailureCategory::ProviderLength => ("服务商长度终止", "Provider length stop"),
        GenerationFailureCategory::Network => ("网络", "Network"),
        GenerationFailureCategory::Storage => ("保存", "Storage"),
        GenerationFailureCategory::Cancelled => ("已取消", "Cancelled"),
        GenerationFailureCategory::Unknown => ("其他", "Other"),
    };
    if locale == "zh-CN" {
        labels.0
    } else {
        labels.1
    }
}

/// 基线 `formatTaskBudgetDecisionFailure`。
pub fn format_task_budget_decision_failure(decision: &TaskBudgetDecision, locale: &str) -> String {
    let decisive: Vec<&str> = decision
        .reasons
        .iter()
        .filter(|reason| reason.selected)
        .map(|reason| reason_label(reason.code, locale))
        .collect();
    if locale == "zh-CN" {
        format!(
            "任务需要 {} 个输出 tokens；本次未发送。裁决依据：{}。",
            format_integer(decision.requested_output_tokens, locale),
            decisive.join("、"),
        )
    } else {
        format!(
            "The task needs {} output tokens; no request was sent. Decision factors: {}.",
            format_integer(decision.requested_output_tokens, locale),
            decisive.join(", "),
        )
    }
}

/// 基线 `formatGenerationBudgetDiagnostic`：仅格式化安全
/// 会计字段。
pub fn format_generation_budget_diagnostic(
    diagnostic: &GenerationBudgetDiagnostic,
    locale: &str,
) -> String {
    let category = classify_generation_failure(
        diagnostic.failure_code.as_deref(),
        diagnostic.finish_reason.as_deref(),
    );
    let category_text = if category == GenerationFailureCategory::Unknown {
        match diagnostic.actual_state {
            GenerationBudgetActualState::Settled => {
                if locale == "zh-CN" {
                    "预算结算"
                } else {
                    "Budget settled"
                }
            }
            GenerationBudgetActualState::Reserved => {
                if locale == "zh-CN" {
                    "预算预留"
                } else {
                    "Budget reserved"
                }
            }
            GenerationBudgetActualState::NotDispatched => {
                if locale == "zh-CN" {
                    "未发送"
                } else {
                    "Not dispatched"
                }
            }
            GenerationBudgetActualState::Unknown => {
                if locale == "zh-CN" {
                    "用量待确认"
                } else {
                    "Usage pending confirmation"
                }
            }
        }
    } else {
        failure_category_label(category, locale)
    };
    let requested = format_integer(diagnostic.requested_output_tokens, locale);
    let semantic_tokens = diagnostic
        .reasons
        .iter()
        .find(|reason| reason.code == TaskBudgetReasonCode::TaskDemand)
        .and_then(|reason| reason.value_tokens);
    let output = match semantic_tokens {
        Some(semantic_tokens) if semantic_tokens != diagnostic.requested_output_tokens => {
            if locale == "zh-CN" {
                format!(
                    "语义输出估算 {} tokens，物理输出上限 {requested} tokens",
                    format_integer(semantic_tokens, locale),
                )
            } else {
                format!(
                    "semantic output estimate {} tokens, physical output limit {requested} tokens",
                    format_integer(semantic_tokens, locale),
                )
            }
        }
        _ => {
            if locale == "zh-CN" {
                format!("需求输出 {requested} tokens")
            } else {
                format!("requested output {requested} tokens")
            }
        }
    };
    let reserved = format_integer(diagnostic.reserved_tokens, locale);
    let selected_reasons: Vec<&str> = diagnostic
        .reasons
        .iter()
        .filter(|reason| reason.selected)
        .map(|reason| reason_label(reason.code, locale))
        .collect();
    let reason_suffix = if selected_reasons.is_empty() {
        String::new()
    } else if locale == "zh-CN" {
        format!("裁决依据：{}。", selected_reasons.join("、"))
    } else {
        format!("Decision factors: {}.", selected_reasons.join(", "))
    };
    if diagnostic.actual_state == GenerationBudgetActualState::NotDispatched {
        if locale == "zh-CN" {
            format!(
                "{category_text}；{output}，原总预留 {reserved} tokens；请求未发送，预留已释放。{reason_suffix}"
            )
        } else {
            format!(
                "{category_text}; {output}, original total reservation {reserved} tokens; no request was sent and the reservation was released. {reason_suffix}"
            )
            .trim()
            .to_string()
        }
    } else if diagnostic.actual_state == GenerationBudgetActualState::Unknown {
        if locale == "zh-CN" {
            format!(
                "{category_text}；{output}，总预留 {reserved} tokens；实际用量未知，父任务仍按预留额度保守记账。{reason_suffix}"
            )
        } else {
            format!(
                "{category_text}; {output}, total reserved {reserved} tokens; actual usage is unknown, so the parent task conservatively keeps the reservation. {reason_suffix}"
            )
            .trim()
            .to_string()
        }
    } else if diagnostic.actual_state == GenerationBudgetActualState::Reserved {
        if locale == "zh-CN" {
            format!(
                "{category_text}；{output}，总预留 {reserved} tokens；请求尚未结算。{reason_suffix}"
            )
        } else {
            format!(
                "{category_text}; {output}, total reserved {reserved} tokens; the request has not settled yet. {reason_suffix}"
            )
            .trim()
            .to_string()
        }
    } else {
        let actual = diagnostic.actual;
        let total = match actual.and_then(|actual| actual.total) {
            None => {
                if locale == "zh-CN" {
                    "未知".to_string()
                } else {
                    "unknown".to_string()
                }
            }
            Some(total) => format!("{} tokens", format_integer(total, locale)),
        };
        let parts = match actual {
            Some(actual) => {
                let entries = [
                    (
                        if locale == "zh-CN" { "输入" } else { "input" },
                        actual.input,
                    ),
                    (
                        if locale == "zh-CN" {
                            "输出"
                        } else {
                            "completion"
                        },
                        actual.completion,
                    ),
                    (
                        if locale == "zh-CN" {
                            "推理"
                        } else {
                            "reasoning"
                        },
                        actual.reasoning,
                    ),
                ];
                entries
                    .iter()
                    .map(|(label, value)| match value {
                        None => format!(
                            "{label} {}",
                            if locale == "zh-CN" {
                                "未知"
                            } else {
                                "unknown"
                            }
                        ),
                        Some(value) => format!("{label} {}", format_integer(*value, locale)),
                    })
                    .collect::<Vec<_>>()
                    .join(if locale == "zh-CN" { "、" } else { ", " })
            }
            None => {
                if locale == "zh-CN" {
                    "分项未知".to_string()
                } else {
                    "breakdown unknown".to_string()
                }
            }
        };
        if locale == "zh-CN" {
            format!(
                "{category_text}；{output}，总预留 {reserved} tokens；实际总量 {total}（{parts}）。{reason_suffix}"
            )
        } else {
            format!(
                "{category_text}; {output}, total reserved {reserved} tokens; actual total {total} ({parts}). {reason_suffix}"
            )
            .trim()
            .to_string()
        }
    }
}

/// 基线 `formatPromptBudgetFailure`：仅格式化安全字节报告；
/// 提示词片段永不越过此边界。
pub fn format_prompt_budget_failure(report: &PromptBudgetReport, locale: &str) -> String {
    let mut sections = report.sections.clone();
    // JS `Array.prototype.sort` 为稳定排序，Rust `sort_by`
    // 同为稳定排序，相等区段相对顺序一致。
    sections.sort_by(|left, right| right.utf8_bytes.cmp(&left.utf8_bytes));
    let contributors: Vec<String> = sections
        .iter()
        .take(3)
        .map(|section| {
            format!(
                "{} {}",
                section_label(section, locale),
                format_integer(section.utf8_bytes, locale)
            )
        })
        .collect();
    let contributors = contributors.join(if locale == "zh-CN" { "、" } else { ", " });

    if locale == "zh-CN" {
        let context_window = match report.context_window_tokens {
            None => "未知".to_string(),
            Some(context_window_tokens) => {
                format!("{} tokens", format_integer(context_window_tokens, locale))
            }
        };
        let estimated_input = match report.estimated_input_tokens {
            None => "未知".to_string(),
            Some(estimated_input_tokens) => {
                format!("{} tokens", format_integer(estimated_input_tokens, locale))
            }
        };
        format!(
            "提示词共 {} UTF-8 字节，超过上限 {} 字节；输出保留空间为 {} tokens。模型上下文：{context_window}；估算输入：{estimated_input}。主要占用：{contributors}。模型：{}；结果码：{}。",
            format_integer(report.total_utf8_bytes, locale),
            format_integer(report.limit_utf8_bytes, locale),
            format_integer(report.reserved_output_tokens, locale),
            report.model_id,
            report.error_code,
        )
    } else {
        let context_window = match report.context_window_tokens {
            None => "unknown".to_string(),
            Some(context_window_tokens) => {
                format!("{} tokens", format_integer(context_window_tokens, locale))
            }
        };
        let estimated_input = match report.estimated_input_tokens {
            None => "unknown".to_string(),
            Some(estimated_input_tokens) => {
                format!("{} tokens", format_integer(estimated_input_tokens, locale))
            }
        };
        format!(
            "The prompt uses {} UTF-8 bytes, exceeding the {}-byte limit; {} tokens are reserved for output. Model context: {context_window}; estimated input: {estimated_input}. Top contributors: {contributors}. Model: {}; result code: {}.",
            format_integer(report.total_utf8_bytes, locale),
            format_integer(report.limit_utf8_bytes, locale),
            format_integer(report.reserved_output_tokens, locale),
            report.model_id,
            report.error_code,
        )
    }
}

/// 基线 `promptBudgetFailureFromError`。Rust 无异常类，
/// 调用方负责把捕获的 `PromptBudgetExceededError` 载荷
/// 传入；`None` 对应基线非该类错误时返回 `undefined`。
pub fn prompt_budget_failure_from_error(
    report: Option<&PromptBudgetReport>,
    locale: &str,
) -> Option<PromptBudgetFailure> {
    let report = report?;
    Some(PromptBudgetFailure {
        failure_code: PROMPT_BUDGET_FAILURE_CODE.to_string(),
        message: format_prompt_budget_failure(report, locale),
        report: report.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::reasoning::GenerationReasoningStage;
    use crate::services::task_budget_planner::{
        plan_task_budget, TaskBudgetCapabilityConstraints, TaskBudgetCapabilitySource,
        TaskBudgetDemand, TaskBudgetInputEstimate, TaskBudgetLiabilityBound,
        TaskBudgetPlannerInput, TaskBudgetReason, TaskBudgetReasonCode, TaskBudgetRootLimits,
    };

    /// 基线测试 fixture（`__tests__/prompt-budget-failure.test.ts`）。
    fn writing_skill_report() -> PromptBudgetReport {
        PromptBudgetReport {
            total_utf8_bytes: 12_500,
            limit_utf8_bytes: 12_000,
            context_window_tokens: Some(16_384),
            estimated_input_tokens: Some(3_200),
            reserved_output_tokens: 4_096,
            sections: vec![PromptBudgetSectionReport {
                section_name: "writing-skill".to_string(),
                display_name: Some("Scene Craft".to_string()),
                utf8_bytes: 2_400,
            }],
            model_id: "model-1".to_string(),
            error_code: "PROMPT_BUDGET_EXHAUSTED".to_string(),
        }
    }

    #[test]
    fn shows_the_safe_skill_display_name_without_exposing_its_prompt_content() {
        let report = writing_skill_report();
        // 基线另断言 `JSON.stringify(report)` 不含提示词正文——
        // 本投影类型按构造不存在提示词文本字段（类型级保证）。
        assert!(format_prompt_budget_failure(&report, "en-US")
            .contains("Writing Skill: Scene Craft 2,400"));
        assert!(format_prompt_budget_failure(&report, "zh-CN")
            .contains("写作 Skill：Scene Craft 2,400"));
    }

    #[test]
    fn separates_requested_total_reserved_and_trusted_actual_usage() {
        let diagnostic = GenerationBudgetDiagnostic {
            attempt_id: "attempt-1".to_string(),
            planner_version: Some("s07-task-budget-v1".to_string()),
            requested_output_tokens: 7_712,
            reserved_tokens: 9_224,
            actual: Some(
                crate::services::task_budget_planner::GenerationActualUsage {
                    input: Some(900),
                    completion: Some(3_050),
                    reasoning: Some(210),
                    total: Some(4_160),
                },
            ),
            actual_state: GenerationBudgetActualState::Settled,
            finish_reason: Some("stop".to_string()),
            failure_code: None,
            reasons: vec![TaskBudgetReason {
                code: TaskBudgetReasonCode::ModelOutputCap,
                value_tokens: Some(8_192),
                selected: true,
            }],
        };

        let message = format_generation_budget_diagnostic(&diagnostic, "zh-CN");
        assert!(message.contains("需求输出 7,712 tokens"));
        assert!(message.contains("总预留 9,224 tokens"));
        assert!(message.contains("实际总量 4,160 tokens"));
        assert!(message.contains("推理 210"));
        assert!(message.contains("裁决依据：模型输出上限"));
    }

    #[test]
    fn keeps_an_unknown_sent_attempt_charged_at_its_reservation() {
        let message = format_generation_budget_diagnostic(
            &GenerationBudgetDiagnostic {
                attempt_id: "attempt-unknown".to_string(),
                planner_version: None,
                requested_output_tokens: 4_096,
                reserved_tokens: 8_192,
                actual: None,
                actual_state: GenerationBudgetActualState::Unknown,
                finish_reason: None,
                failure_code: Some("NETWORK_ERROR".to_string()),
                reasons: vec![],
            },
            "zh-CN",
        );

        assert!(message.contains("网络"));
        assert!(message.contains("实际用量未知"));
        assert!(message.contains("仍按预留额度保守记账"));
    }

    #[test]
    fn explains_semantic_sizing_and_physical_allocation_without_upgrading_unknown_usage() {
        for locale in ["zh-CN", "en-US"] {
            let message = format_generation_budget_diagnostic(
                &GenerationBudgetDiagnostic {
                    attempt_id: "shared-completion".to_string(),
                    planner_version: None,
                    requested_output_tokens: 16_384,
                    reserved_tokens: 50_410,
                    actual: None,
                    actual_state: GenerationBudgetActualState::Unknown,
                    finish_reason: None,
                    failure_code: None,
                    reasons: vec![
                        TaskBudgetReason {
                            code: TaskBudgetReasonCode::TaskDemand,
                            value_tokens: Some(1_712),
                            selected: true,
                        },
                        TaskBudgetReason {
                            code: TaskBudgetReasonCode::OutputAllocationCeiling,
                            value_tokens: Some(16_384),
                            selected: true,
                        },
                        TaskBudgetReason {
                            code: TaskBudgetReasonCode::LiabilityBoundUnknown,
                            value_tokens: None,
                            selected: true,
                        },
                    ],
                },
                locale,
            );
            assert!(message.contains(if locale == "zh-CN" {
                "语义输出估算 1,712 tokens"
            } else {
                "semantic output estimate 1,712 tokens"
            }));
            assert!(message.contains(if locale == "zh-CN" {
                "物理输出上限 16,384 tokens"
            } else {
                "physical output limit 16,384 tokens"
            }));
            assert!(message.contains(if locale == "zh-CN" {
                "实际用量未知"
            } else {
                "actual usage is unknown"
            }));
        }
    }

    #[test]
    fn classifies_failure_codes_and_finish_reasons_separately() {
        for (failure_code, finish_reason, expected) in [
            (
                Some("TASK_BUDGET_CAPACITY_CONFLICT"),
                None,
                GenerationFailureCategory::CapacityPreflight,
            ),
            (
                Some("TASK_BUDGET_CAPACITY_CONFLICT:single-item-capacity-conflict"),
                None,
                GenerationFailureCategory::CapacityPreflight,
            ),
            (
                Some("Error invoking remote method 'generation:run': Error: TASK_BUDGET_CAPACITY_CONFLICT:single-item-capacity-conflict"),
                None,
                GenerationFailureCategory::CapacityPreflight,
            ),
            (
                Some("GENERATION_RESERVATION_INVALID"),
                None,
                GenerationFailureCategory::Unknown,
            ),
            (
                Some("TASK_BUDGET_SPLIT_REQUIRED:scope-split"),
                None,
                GenerationFailureCategory::CapacityPreflight,
            ),
            // 主进程预检实际抛出的是带 SCOPE_ 的拼写，
            // 批执行器也按它解析。
            (
                Some("TASK_BUDGET_SCOPE_SPLIT_REQUIRED:7"),
                None,
                GenerationFailureCategory::CapacityPreflight,
            ),
            (
                Some("Error invoking remote method 'generation:execute': Error: TASK_BUDGET_SCOPE_SPLIT_REQUIRED:7"),
                None,
                GenerationFailureCategory::CapacityPreflight,
            ),
            (
                Some("ROOT_BUDGET_EXHAUSTED"),
                None,
                GenerationFailureCategory::BudgetExhausted,
            ),
            (None, Some("length"), GenerationFailureCategory::ProviderLength),
            (
                Some("NETWORK_ERROR"),
                None,
                GenerationFailureCategory::Network,
            ),
            (
                Some("GENERATION_STORAGE_FAILED"),
                None,
                GenerationFailureCategory::Storage,
            ),
            (Some("SQLITE_FULL"), None, GenerationFailureCategory::Storage),
            (
                Some("GENERATION_CANCELLED"),
                None,
                GenerationFailureCategory::Cancelled,
            ),
            (None, Some("cancelled"), GenerationFailureCategory::Cancelled),
        ] {
            assert_eq!(
                classify_generation_failure(failure_code, finish_reason),
                expected,
                "failureCode={failure_code:?}, finishReason={finish_reason:?}"
            );
        }
    }

    #[test]
    fn reports_a_released_pre_dispatch_reservation_without_inventing_zero_actual_usage() {
        let message = format_generation_budget_diagnostic(
            &GenerationBudgetDiagnostic {
                attempt_id: "cancelled-before-dispatch".to_string(),
                planner_version: None,
                requested_output_tokens: 4_096,
                reserved_tokens: 8_192,
                actual: None,
                actual_state: GenerationBudgetActualState::NotDispatched,
                finish_reason: None,
                failure_code: Some("GENERATION_CANCELLED".to_string()),
                reasons: vec![],
            },
            "zh-CN",
        );

        assert!(message.contains("已取消"));
        assert!(message.contains("原总预留 8,192 tokens"));
        assert!(message.contains("请求未发送，预留已释放"));
        assert!(!message.contains("实际用量"));
        assert!(!message.contains("实际总量 0"));
    }

    #[test]
    fn explains_a_preflight_conflict_without_exposing_prompt_content() {
        let decision = plan_task_budget(TaskBudgetPlannerInput {
            stage: GenerationReasoningStage::Drafting,
            demand: TaskBudgetDemand::DraftUnits {
                writing_language: "zh-CN".to_string(),
                requested_units: 3_000,
                segmentable: false,
            },
            input_estimate: TaskBudgetInputEstimate {
                upper_bound_tokens: 1_000,
                estimator_version: "fixture-v1".to_string(),
            },
            capability: TaskBudgetCapabilityConstraints {
                model_context_window_tokens: Some(8_192),
                model_max_output_tokens: Some(4_096),
                model_context_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                model_output_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                user_context_window_tokens: None,
                user_max_output_tokens: None,
            },
            root: TaskBudgetRootLimits {
                remaining_token_liability: 100_000,
                max_output_per_request: 32_768,
            },
            liability: TaskBudgetLiabilityBound::IncludedInOutput,
            output_allocation: None,
            safety_margin_tokens: 512,
        })
        .unwrap();

        let message = format_task_budget_decision_failure(&decision, "zh-CN");
        assert!(message.contains("本次未发送"));
        assert!(message.contains("此正文任务不能安全分段"));
    }

    #[test]
    fn formats_prompt_budget_failure_with_localized_contributors() {
        // 补充用例（基线测试仅覆盖 writing-skill 显示名；
        // 此处验证区段排序取前三与双语模板）。
        let report = PromptBudgetReport {
            total_utf8_bytes: 20_000,
            limit_utf8_bytes: 12_000,
            context_window_tokens: None,
            estimated_input_tokens: None,
            reserved_output_tokens: 2_048,
            sections: vec![
                PromptBudgetSectionReport {
                    section_name: "architecture".to_string(),
                    display_name: None,
                    utf8_bytes: 9_000,
                },
                PromptBudgetSectionReport {
                    section_name: "knowledge-base".to_string(),
                    display_name: None,
                    utf8_bytes: 5_000,
                },
                PromptBudgetSectionReport {
                    section_name: "genre".to_string(),
                    display_name: None,
                    utf8_bytes: 3_000,
                },
                PromptBudgetSectionReport {
                    section_name: "unknown-section".to_string(),
                    display_name: None,
                    utf8_bytes: 2_000,
                },
            ],
            model_id: "model-x".to_string(),
            error_code: "PROMPT_BUDGET_EXHAUSTED".to_string(),
        };
        let zh = format_prompt_budget_failure(&report, "zh-CN");
        assert!(zh.contains(
            "提示词共 20,000 UTF-8 字节，超过上限 12,000 字节；输出保留空间为 2,048 tokens。"
        ));
        assert!(zh.contains("模型上下文：未知；估算输入：未知。"));
        assert!(zh.contains("主要占用：故事架构 9,000、知识库 5,000、作品类型 3,000。"));
        assert!(zh.contains("模型：model-x；结果码：PROMPT_BUDGET_EXHAUSTED。"));
        let en = format_prompt_budget_failure(&report, "en-US");
        assert!(en.contains(
            "The prompt uses 20,000 UTF-8 bytes, exceeding the 12,000-byte limit; 2,048 tokens are reserved for output. "
        ));
        assert!(en.contains("Model context: unknown; estimated input: unknown. "));
        assert!(en.contains(
            "Top contributors: Story architecture 9,000, Knowledge base 5,000, Genre 3,000. "
        ));
        assert!(en.contains("Model: model-x; result code: PROMPT_BUDGET_EXHAUSTED."));
    }

    #[test]
    fn prompt_budget_failure_from_error_maps_the_exceeded_report() {
        let report = writing_skill_report();
        let failure = prompt_budget_failure_from_error(Some(&report), "zh-CN").unwrap();
        assert_eq!(failure.failure_code, "prompt_budget_exhausted");
        assert_eq!(
            failure.message,
            format_prompt_budget_failure(&report, "zh-CN")
        );
        assert_eq!(failure.report, report);
        // 基线非该类错误时返回 undefined。
        assert!(prompt_budget_failure_from_error(None, "zh-CN").is_none());
    }

    #[test]
    fn format_integer_groups_thousands_for_both_locales() {
        // 补充用例：验证 Intl.NumberFormat 等价实现。
        for locale in ["zh-CN", "en-US"] {
            assert_eq!(format_integer(0, locale), "0");
            assert_eq!(format_integer(999, locale), "999");
            assert_eq!(format_integer(1_000, locale), "1,000");
            assert_eq!(format_integer(1_234_567, locale), "1,234,567");
            assert_eq!(format_integer(-1_234, locale), "-1,234");
        }
    }
}
