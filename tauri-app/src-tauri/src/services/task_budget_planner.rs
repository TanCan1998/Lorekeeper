//! S07 任务预算规划器（1:1 移植 `src/services/generation/task-budget-planner.ts`）。
//!
//! 纯 S07 规划缝：提出单一物理预留；持久化 main owner 独自负责
//! 预留、派发与结算（本模块不接 owner 调用点——为 B3-1c 计划
//! 构造器供给依赖）。非法输入统一返回 `TASK_BUDGET_INPUT_INVALID`
//! （基线 `fail()` 抛出 `Error('TASK_BUDGET_INPUT_INVALID')`）。
//!
//! 移植说明：
//! - 基线 TS 判别联合（`TaskBudgetDemand` / `TaskBudgetLiabilityBound`
//!   / `decision` / `outputAllocation` / 原因码）以 Rust 枚举建模；
//! - 基线 `number` 在本域均为整数 token 计数，Rust 以 `i64` 建模
//!   （计数远低于 JS 安全整数上界，语义等价）；基线 `?? Infinity`
//!   语义以 `Option<i64>` 建模（`None` 即 Infinity）；
//! - `stage` 复用 `crate::llm::reasoning::GenerationReasoningStage`
//!   （基线 `GenerationReasoningStage` 的既有 Rust 等价物）；
//! - `writingLanguage` 基线为 `'zh-CN' | 'en-US'` 字面量联合，Rust
//!   侧无对应枚举，按既有惯例以字符串传递并在规划器入口运行时
//!   校验（与基线运行时校验一致）；
//! - Rust 侧无 `TaskBudgetDecision` 既有类型（generation_run_repository
//!   以 `serde_json::Value` 透传 TS 鸭子类型，main_generation_owner
//!   为私有 IPC 投影），本模块以基线形状 1:1 定义完整类型；
//! - 类型当前无 IPC 消费面（grep 核实），derive `Debug/Clone/PartialEq`
//!   供测试断言，暂不 serde。

use crate::llm::reasoning::GenerationReasoningStage;

/// 单单位 token 费率（基线 `{ numerator, denominator }`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenRate {
    pub numerator: i64,
    pub denominator: i64,
}

/// 草稿单位策略（基线 `S07_TASK_BUDGET_POLICY.draftUnits`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskBudgetDraftUnitsPolicy {
    pub base_tokens: i64,
    pub tokens_per_unit_zh_cn: TokenRate,
    pub tokens_per_unit_en_us: TokenRate,
}

impl TaskBudgetDraftUnitsPolicy {
    /// 基线 `tokensPerUnit[writingLanguage]`（语言已在规划器入口校验）。
    fn tokens_per_unit(&self, writing_language: &str) -> TokenRate {
        match writing_language {
            "zh-CN" => self.tokens_per_unit_zh_cn,
            "en-US" => self.tokens_per_unit_en_us,
            _ => unreachable!("writing language validated at planner entry"),
        }
    }
}

/// 结构化项分阶段 token 表（基线 `tokensPerItem[language]` 的单语言值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StructuredItemStageTokens {
    pub drafting: i64,
    pub planning: i64,
    pub review: i64,
    pub general: i64,
}

/// 结构化项策略（基线 `S07_TASK_BUDGET_POLICY.structuredItems`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskBudgetStructuredItemsPolicy {
    pub base_tokens: i64,
    pub tokens_per_item_zh_cn: StructuredItemStageTokens,
    pub tokens_per_item_en_us: StructuredItemStageTokens,
}

impl TaskBudgetStructuredItemsPolicy {
    /// 基线 `tokensPerItem[writingLanguage][stage]`（语言已在入口校验）。
    fn tokens_per_item(&self, writing_language: &str, stage: GenerationReasoningStage) -> i64 {
        let table = match writing_language {
            "zh-CN" => &self.tokens_per_item_zh_cn,
            "en-US" => &self.tokens_per_item_en_us,
            _ => unreachable!("writing language validated at planner entry"),
        };
        match stage {
            GenerationReasoningStage::Drafting => table.drafting,
            GenerationReasoningStage::Planning => table.planning,
            GenerationReasoningStage::Review => table.review,
            GenerationReasoningStage::General => table.general,
        }
    }
}

/// S07 任务预算策略（基线 `S07_TASK_BUDGET_POLICY`，`Object.freeze`）。
///
/// 语义化尺寸支持范围规划，并非每个提供方分词器的数学上界；
/// 物理责任保持硬上限。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskBudgetPolicy {
    pub version: &'static str,
    pub output_estimator_version: &'static str,
    pub draft_units: TaskBudgetDraftUnitsPolicy,
    pub structured_items: TaskBudgetStructuredItemsPolicy,
}

pub const S07_TASK_BUDGET_POLICY: TaskBudgetPolicy = TaskBudgetPolicy {
    version: "s07-task-budget-v1",
    output_estimator_version: "semantic-units-sizing-v1",
    draft_units: TaskBudgetDraftUnitsPolicy {
        base_tokens: 512,
        tokens_per_unit_zh_cn: TokenRate {
            numerator: 12,
            denominator: 5,
        },
        tokens_per_unit_en_us: TokenRate {
            numerator: 3,
            denominator: 2,
        },
    },
    structured_items: TaskBudgetStructuredItemsPolicy {
        base_tokens: 512,
        tokens_per_item_zh_cn: StructuredItemStageTokens {
            drafting: 3_072,
            planning: 3_072,
            review: 512,
            general: 512,
        },
        tokens_per_item_en_us: StructuredItemStageTokens {
            drafting: 2_048,
            planning: 2_048,
            review: 384,
            general: 384,
        },
    },
};

/// 需求种类（基线 `TaskBudgetDemand['kind']`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskBudgetDemandKind {
    DraftUnits,
    StructuredItems,
}

/// 任务预算需求（基线 `TaskBudgetDemand` 判别联合）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskBudgetDemand {
    DraftUnits {
        writing_language: String,
        requested_units: i64,
        /// 仅当产品能把每一段保留为显式候选时为 true。
        segmentable: bool,
    },
    StructuredItems {
        writing_language: String,
        requested_items: i64,
    },
}

impl TaskBudgetDemand {
    fn kind(&self) -> TaskBudgetDemandKind {
        match self {
            TaskBudgetDemand::DraftUnits { .. } => TaskBudgetDemandKind::DraftUnits,
            TaskBudgetDemand::StructuredItems { .. } => TaskBudgetDemandKind::StructuredItems,
        }
    }

    fn writing_language(&self) -> &str {
        match self {
            TaskBudgetDemand::DraftUnits {
                writing_language, ..
            } => writing_language,
            TaskBudgetDemand::StructuredItems {
                writing_language, ..
            } => writing_language,
        }
    }
}

/// 能力来源（基线 `TaskBudgetCapabilitySource`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskBudgetCapabilitySource {
    VerifiedProviderPreset,
    Unknown,
}

/// 能力约束（基线 `TaskBudgetCapabilityConstraints`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskBudgetCapabilityConstraints {
    /// 由精确端点/模型预设证明的提供方能力；None 表示未知。
    pub model_context_window_tokens: Option<i64>,
    /// 由精确端点/模型预设证明的提供方能力；None 表示未知。
    pub model_max_output_tokens: Option<i64>,
    pub model_context_source: TaskBudgetCapabilitySource,
    pub model_output_source: TaskBudgetCapabilitySource,
    /// 用户运营限制约束已知提供方能力，但永不证明它。
    pub user_context_window_tokens: Option<i64>,
    pub user_max_output_tokens: Option<i64>,
}

/// 责任上界（基线 `TaskBudgetLiabilityBound` 判别联合）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskBudgetLiabilityBound {
    IncludedInOutput,
    SeparateBounded {
        reasoning_upper_bound_tokens: i64,
    },
    TotalBounded {
        /// 物理上界：输入 + 输出 + 推理 + 安全余量。
        total_liability_upper_bound_tokens: i64,
    },
    Unknown {
        reasoning_upper_bound_tokens: Option<i64>,
    },
}

/// 输入估计（基线 `TaskBudgetPlannerInput.inputEstimate`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBudgetInputEstimate {
    pub upper_bound_tokens: i64,
    pub estimator_version: String,
}

/// 根账本限制（基线 `TaskBudgetPlannerInput.root`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskBudgetRootLimits {
    pub remaining_token_liability: i64,
    pub max_output_per_request: i64,
}

/// 输出分配策略（基线 `outputAllocation?: 'semantic-estimate' | 'available-ceiling' | 'physical-capacity'`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskBudgetOutputAllocation {
    SemanticEstimate,
    AvailableCeiling,
    PhysicalCapacity,
}

/// 规划器输入（基线 `TaskBudgetPlannerInput`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBudgetPlannerInput {
    pub stage: GenerationReasoningStage,
    pub demand: TaskBudgetDemand,
    pub input_estimate: TaskBudgetInputEstimate,
    pub capability: TaskBudgetCapabilityConstraints,
    pub root: TaskBudgetRootLimits,
    pub liability: TaskBudgetLiabilityBound,
    pub output_allocation: Option<TaskBudgetOutputAllocation>,
    pub safety_margin_tokens: i64,
}

/// 决策结果（基线 `decision: 'ready' | 'split-required' | 'capacity-conflict'`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskBudgetDecisionKind {
    Ready,
    SplitRequired,
    CapacityConflict,
}

/// 原因码（基线 `TaskBudgetReasonCode`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskBudgetReasonCode {
    TaskDemand,
    OutputAllocationCeiling,
    InputUpperBound,
    SafetyMargin,
    ModelContextCap,
    ModelOutputCap,
    UserContextCap,
    UserOutputCap,
    RootOutputCap,
    RootRemainingCap,
    ProtocolReasoningReserve,
    ProtocolTotalBound,
    ModelCapabilityUnknown,
    LiabilityBoundUnknown,
    RequiredInputCapacityConflict,
    SingleItemCapacityConflict,
    DraftSegmentationDisabled,
    ScopeSplit,
}

/// 决策原因（基线 `TaskBudgetReason`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBudgetReason {
    pub code: TaskBudgetReasonCode,
    /// 本决策背后的非敏感数值证据（如适用）。
    pub value_tokens: Option<i64>,
    pub selected: bool,
}

/// 决策（基线 `TaskBudgetDecision`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBudgetDecision {
    pub policy_version: &'static str,
    pub output_estimator_version: &'static str,
    pub input_estimator_version: String,
    pub decision: TaskBudgetDecisionKind,
    pub stage: GenerationReasoningStage,
    pub writing_language: String,
    pub demand_kind: TaskBudgetDemandKind,
    pub requested_quantity: i64,
    pub selected_quantity: i64,
    pub remaining_quantity: i64,
    /// 任何容量决策前的完整语义范围输出需求。
    pub requested_output_tokens: i64,
    /// 本次所选物理请求的提供方最大输出；零表示不派发。
    pub reserved_output_tokens: i64,
    pub reasoning_upper_bound_tokens: i64,
    /// 派发前预留的总根账本责任；零表示不派发。
    pub reservation_liability_tokens: i64,
    pub input_upper_bound_tokens: i64,
    pub reasons: Vec<TaskBudgetReason>,
}

/// 受信实际用量（基线 `GenerationBudgetDiagnostic.actual`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationActualUsage {
    pub input: Option<i64>,
    pub completion: Option<i64>,
    pub reasoning: Option<i64>,
    pub total: Option<i64>,
}

/// 实际用量状态（基线 `actualState: 'settled' | 'unknown' | 'reserved' | 'not-dispatched'`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationBudgetActualState {
    Settled,
    Unknown,
    Reserved,
    NotDispatched,
}

/// 生成预算诊断（基线 `GenerationBudgetDiagnostic`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationBudgetDiagnostic {
    pub attempt_id: String,
    pub planner_version: Option<String>,
    pub requested_output_tokens: i64,
    /// 完整根账本预留（含输入/输出/推理/安全）。
    pub reserved_tokens: i64,
    pub actual: Option<GenerationActualUsage>,
    pub actual_state: GenerationBudgetActualState,
    pub finish_reason: Option<String>,
    pub failure_code: Option<String>,
    pub reasons: Vec<TaskBudgetReason>,
}

/// 基线 `fail()`：非法输入统一以 `TASK_BUDGET_INPUT_INVALID` 拒绝。
fn fail() -> String {
    "TASK_BUDGET_INPUT_INVALID".to_string()
}

/// 基线 `positiveInteger`（正安全整数；本域计数远低于 JS 安全整数上界）。
fn positive_integer(value: i64) -> Result<(), String> {
    if value <= 0 {
        Err(fail())
    } else {
        Ok(())
    }
}

/// 基线 `nonNegativeInteger`。
fn non_negative_integer(value: i64) -> Result<(), String> {
    if value < 0 {
        Err(fail())
    } else {
        Ok(())
    }
}

/// 基线 `optionalPositiveInteger`。
fn optional_positive_integer(value: Option<i64>) -> Result<(), String> {
    match value {
        Some(value) => positive_integer(value),
        None => Ok(()),
    }
}

/// 基线 `demandQuantity`。
fn demand_quantity(demand: &TaskBudgetDemand) -> i64 {
    match demand {
        TaskBudgetDemand::DraftUnits {
            requested_units, ..
        } => *requested_units,
        TaskBudgetDemand::StructuredItems {
            requested_items, ..
        } => *requested_items,
    }
}

/// 基线 `outputTokensFor`：语义范围输出需求（`Math.ceil` 以整数
/// 上取整等价实现，入参均为正整数）。
fn output_tokens_for(
    stage: GenerationReasoningStage,
    demand: &TaskBudgetDemand,
    quantity: i64,
) -> i64 {
    if quantity == 0 {
        return 0;
    }
    match demand {
        TaskBudgetDemand::DraftUnits {
            writing_language, ..
        } => {
            let rate = S07_TASK_BUDGET_POLICY
                .draft_units
                .tokens_per_unit(writing_language);
            let product = quantity * rate.numerator;
            S07_TASK_BUDGET_POLICY.draft_units.base_tokens
                + (product + rate.denominator - 1) / rate.denominator
        }
        TaskBudgetDemand::StructuredItems {
            writing_language, ..
        } => {
            S07_TASK_BUDGET_POLICY.structured_items.base_tokens
                + quantity
                    * S07_TASK_BUDGET_POLICY
                        .structured_items
                        .tokens_per_item(writing_language, stage)
        }
    }
}

/// 基线 `maximumQuantityFor`（`Math.floor` 以整数除法等价实现）。
fn maximum_quantity_for(
    stage: GenerationReasoningStage,
    demand: &TaskBudgetDemand,
    available_output_tokens: i64,
) -> i64 {
    match demand {
        TaskBudgetDemand::DraftUnits {
            writing_language, ..
        } => {
            let usable = available_output_tokens - S07_TASK_BUDGET_POLICY.draft_units.base_tokens;
            if usable <= 0 {
                return 0;
            }
            let rate = S07_TASK_BUDGET_POLICY
                .draft_units
                .tokens_per_unit(writing_language);
            usable * rate.denominator / rate.numerator
        }
        TaskBudgetDemand::StructuredItems {
            writing_language, ..
        } => {
            let usable =
                available_output_tokens - S07_TASK_BUDGET_POLICY.structured_items.base_tokens;
            if usable <= 0 {
                return 0;
            }
            usable
                / S07_TASK_BUDGET_POLICY
                    .structured_items
                    .tokens_per_item(writing_language, stage)
        }
    }
}

/// 基线 `conflict`：容量冲突决策（不留任何预留）。
fn conflict(
    input: &TaskBudgetPlannerInput,
    requested_quantity: i64,
    requested_output_tokens: i64,
    reasons: Vec<TaskBudgetReason>,
) -> TaskBudgetDecision {
    TaskBudgetDecision {
        policy_version: S07_TASK_BUDGET_POLICY.version,
        output_estimator_version: S07_TASK_BUDGET_POLICY.output_estimator_version,
        input_estimator_version: input.input_estimate.estimator_version.clone(),
        decision: TaskBudgetDecisionKind::CapacityConflict,
        stage: input.stage,
        writing_language: input.demand.writing_language().to_string(),
        demand_kind: input.demand.kind(),
        requested_quantity,
        selected_quantity: 0,
        remaining_quantity: requested_quantity,
        requested_output_tokens,
        reserved_output_tokens: 0,
        reasoning_upper_bound_tokens: 0,
        reservation_liability_tokens: 0,
        input_upper_bound_tokens: input.input_estimate.upper_bound_tokens,
        reasons,
    }
}

/// 纯 S07 规划缝（基线 `planTaskBudget`）。它提出一个物理预留；
/// 持久化 main owner 独自负责预留、派发与结算。
///
/// 入口校验顺序与基线逐行对齐：语言/估计器版本 → 需求量 →
/// 输入上界 → 根剩余/单请求上限 → 安全余量 → 能力取值 →
/// 来源标记一致性 → 责任模式分支校验。`stage` 与 `demand.kind`
/// 的基线运行时校验在 Rust 由枚举类型编译期保证（类型化收窄，
/// 与 repositories 既有做法一致）。
pub fn plan_task_budget(input: TaskBudgetPlannerInput) -> Result<TaskBudgetDecision, String> {
    if !matches!(input.demand.writing_language(), "zh-CN" | "en-US")
        || input.input_estimate.estimator_version.trim().is_empty()
    {
        return Err(fail());
    }
    let requested_quantity = demand_quantity(&input.demand);
    positive_integer(requested_quantity)?;
    non_negative_integer(input.input_estimate.upper_bound_tokens)?;
    non_negative_integer(input.root.remaining_token_liability)?;
    positive_integer(input.root.max_output_per_request)?;
    non_negative_integer(input.safety_margin_tokens)?;
    optional_positive_integer(input.capability.model_context_window_tokens)?;
    optional_positive_integer(input.capability.model_max_output_tokens)?;
    optional_positive_integer(input.capability.user_context_window_tokens)?;
    optional_positive_integer(input.capability.user_max_output_tokens)?;
    // 来源标记与取值必须一致：取值为 None（未知）当且仅当来源为 unknown。
    if (input.capability.model_context_window_tokens.is_none())
        != (input.capability.model_context_source == TaskBudgetCapabilitySource::Unknown)
        || (input.capability.model_max_output_tokens.is_none())
            != (input.capability.model_output_source == TaskBudgetCapabilitySource::Unknown)
    {
        return Err(fail());
    }
    match input.liability {
        TaskBudgetLiabilityBound::SeparateBounded {
            reasoning_upper_bound_tokens,
        } => non_negative_integer(reasoning_upper_bound_tokens)?,
        TaskBudgetLiabilityBound::Unknown {
            reasoning_upper_bound_tokens,
        } => non_negative_integer(reasoning_upper_bound_tokens.unwrap_or(0))?,
        TaskBudgetLiabilityBound::TotalBounded {
            total_liability_upper_bound_tokens,
        } => positive_integer(total_liability_upper_bound_tokens)?,
        TaskBudgetLiabilityBound::IncludedInOutput => {}
    }

    let requested_output_tokens = output_tokens_for(input.stage, &input.demand, requested_quantity);
    let mut reasons: Vec<TaskBudgetReason> = vec![
        TaskBudgetReason {
            code: TaskBudgetReasonCode::TaskDemand,
            value_tokens: Some(requested_output_tokens),
            selected: true,
        },
        TaskBudgetReason {
            code: TaskBudgetReasonCode::InputUpperBound,
            value_tokens: Some(input.input_estimate.upper_bound_tokens),
            selected: true,
        },
        TaskBudgetReason {
            code: TaskBudgetReasonCode::SafetyMargin,
            value_tokens: Some(input.safety_margin_tokens),
            selected: true,
        },
    ];
    if input.capability.model_context_window_tokens.is_none()
        || input.capability.model_max_output_tokens.is_none()
    {
        reasons.push(TaskBudgetReason {
            code: TaskBudgetReasonCode::ModelCapabilityUnknown,
            value_tokens: None,
            selected: true,
        });
    }
    if matches!(input.liability, TaskBudgetLiabilityBound::Unknown { .. }) {
        reasons.push(TaskBudgetReason {
            code: TaskBudgetReasonCode::LiabilityBoundUnknown,
            value_tokens: None,
            selected: true,
        });
    }

    let model_context = input.capability.model_context_window_tokens;
    // 基线 `Math.min(modelContext ?? Infinity, userContextWindowTokens ?? Infinity)`：
    // 任一未知（Infinity）时取另一个，两者皆未知时为 None（Infinity）。
    let effective_context: Option<i64> =
        match (model_context, input.capability.user_context_window_tokens) {
            (Some(model), Some(user)) => Some(model.min(user)),
            (Some(model), None) => Some(model),
            (None, Some(user)) => Some(user),
            (None, None) => None,
        };
    let fixed_reasoning_tokens = match input.liability {
        TaskBudgetLiabilityBound::SeparateBounded {
            reasoning_upper_bound_tokens,
        } => reasoning_upper_bound_tokens,
        TaskBudgetLiabilityBound::Unknown {
            reasoning_upper_bound_tokens,
        } => reasoning_upper_bound_tokens.unwrap_or(0),
        _ => 0,
    };
    let mut output_limits: Vec<(TaskBudgetReasonCode, i64)> = Vec::new();
    if let Some(model_max_output_tokens) = input.capability.model_max_output_tokens {
        output_limits.push((
            TaskBudgetReasonCode::ModelOutputCap,
            model_max_output_tokens,
        ));
    }
    output_limits.push((
        TaskBudgetReasonCode::RootOutputCap,
        input.root.max_output_per_request,
    ));
    if let Some(effective_context) = effective_context {
        let code = if model_context.is_none() {
            TaskBudgetReasonCode::UserContextCap
        } else {
            TaskBudgetReasonCode::ModelContextCap
        };
        output_limits.push((
            code,
            effective_context
                - input.input_estimate.upper_bound_tokens
                - fixed_reasoning_tokens
                - input.safety_margin_tokens,
        ));
    }
    if let Some(user_max_output_tokens) = input.capability.user_max_output_tokens {
        output_limits.push((TaskBudgetReasonCode::UserOutputCap, user_max_output_tokens));
    }
    if let Some(user_context_window_tokens) = input.capability.user_context_window_tokens {
        reasons.push(TaskBudgetReason {
            code: TaskBudgetReasonCode::UserContextCap,
            value_tokens: Some(user_context_window_tokens),
            // 基线 `userContextWindowTokens <= (modelContext ?? Infinity)`：
            // 模型能力未知时任何有限用户设置都不超过 Infinity。
            selected: match model_context {
                Some(model_context) => user_context_window_tokens <= model_context,
                None => true,
            },
        });
    }

    let is_separate_bounded = matches!(
        input.liability,
        TaskBudgetLiabilityBound::SeparateBounded { .. }
    );
    let is_included_in_output = matches!(
        input.liability,
        TaskBudgetLiabilityBound::IncludedInOutput { .. }
    );
    let is_unknown = matches!(input.liability, TaskBudgetLiabilityBound::Unknown { .. });
    let mut total_envelope_tokens: Option<i64> = None;
    if is_separate_bounded || (is_unknown && fixed_reasoning_tokens > 0) {
        output_limits.push((
            TaskBudgetReasonCode::RootRemainingCap,
            input.root.remaining_token_liability
                - input.input_estimate.upper_bound_tokens
                - fixed_reasoning_tokens
                - input.safety_margin_tokens,
        ));
        reasons.push(TaskBudgetReason {
            code: TaskBudgetReasonCode::ProtocolReasoningReserve,
            value_tokens: Some(fixed_reasoning_tokens),
            selected: true,
        });
    } else if is_included_in_output || is_unknown {
        // 未知协议获得一个估计值，而非声称零推理。
        // 持久化使用策略在结算时保留区分。
        output_limits.push((
            TaskBudgetReasonCode::RootRemainingCap,
            input.root.remaining_token_liability
                - input.input_estimate.upper_bound_tokens
                - input.safety_margin_tokens,
        ));
    } else {
        // 已验证的上下文值是受支持的容量下限，而非协议物理责任
        // 上限的证明。保留独立上界。
        let total_liability_upper_bound_tokens = match input.liability {
            TaskBudgetLiabilityBound::TotalBounded {
                total_liability_upper_bound_tokens,
            } => total_liability_upper_bound_tokens,
            _ => unreachable!("preceding branches narrow the liability mode to total-bounded"),
        };
        total_envelope_tokens = Some(total_liability_upper_bound_tokens);
        reasons.push(TaskBudgetReason {
            code: TaskBudgetReasonCode::ProtocolTotalBound,
            value_tokens: Some(total_liability_upper_bound_tokens),
            selected: true,
        });
        if input.root.remaining_token_liability < total_liability_upper_bound_tokens {
            reasons.push(TaskBudgetReason {
                code: TaskBudgetReasonCode::RootRemainingCap,
                value_tokens: Some(input.root.remaining_token_liability),
                selected: true,
            });
            return Ok(conflict(
                &input,
                requested_quantity,
                requested_output_tokens,
                reasons,
            ));
        }
        output_limits.push((
            TaskBudgetReasonCode::RootRemainingCap,
            total_liability_upper_bound_tokens
                - input.input_estimate.upper_bound_tokens
                - input.safety_margin_tokens,
        ));
    }

    // outputLimits 至少含 root-output-cap（基线同前提，永不为空）；
    // 防御性回退以 i64::MAX 近似基线空数组时的 Infinity。
    let available_output_tokens = output_limits
        .iter()
        .map(|(_, value)| *value)
        .min()
        .unwrap_or(i64::MAX);
    for (code, value) in &output_limits {
        reasons.push(TaskBudgetReason {
            code: *code,
            value_tokens: Some((*value).max(0)),
            selected: *value == available_output_tokens,
        });
    }
    let physical_capacity =
        input.output_allocation == Some(TaskBudgetOutputAllocation::PhysicalCapacity);
    let minimum_output_tokens = if physical_capacity {
        1
    } else {
        output_tokens_for(input.stage, &input.demand, 1)
    };
    if available_output_tokens < minimum_output_tokens {
        reasons.push(TaskBudgetReason {
            code: TaskBudgetReasonCode::RequiredInputCapacityConflict,
            value_tokens: Some(available_output_tokens.max(0)),
            selected: true,
        });
        if matches!(input.demand, TaskBudgetDemand::StructuredItems { .. })
            && requested_quantity == 1
        {
            reasons.push(TaskBudgetReason {
                code: TaskBudgetReasonCode::SingleItemCapacityConflict,
                value_tokens: None,
                selected: true,
            });
        }
        return Ok(conflict(
            &input,
            requested_quantity,
            requested_output_tokens,
            reasons,
        ));
    }

    let selected_quantity =
        if physical_capacity && matches!(input.demand, TaskBudgetDemand::DraftUnits { .. }) {
            requested_quantity
        } else {
            requested_quantity.min((if physical_capacity { 1 } else { 0 }).max(
                maximum_quantity_for(input.stage, &input.demand, available_output_tokens),
            ))
        };
    if selected_quantity < requested_quantity
        && matches!(
            input.demand,
            TaskBudgetDemand::DraftUnits {
                segmentable: false,
                ..
            }
        )
    {
        reasons.push(TaskBudgetReason {
            code: TaskBudgetReasonCode::DraftSegmentationDisabled,
            value_tokens: None,
            selected: true,
        });
        return Ok(conflict(
            &input,
            requested_quantity,
            requested_output_tokens,
            reasons,
        ));
    }
    if selected_quantity <= 0 {
        reasons.push(TaskBudgetReason {
            code: TaskBudgetReasonCode::RequiredInputCapacityConflict,
            value_tokens: Some(available_output_tokens.max(0)),
            selected: true,
        });
        return Ok(conflict(
            &input,
            requested_quantity,
            requested_output_tokens,
            reasons,
        ));
    }

    let decision_kind = if selected_quantity < requested_quantity {
        TaskBudgetDecisionKind::SplitRequired
    } else {
        TaskBudgetDecisionKind::Ready
    };
    let use_available_ceiling = decision_kind == TaskBudgetDecisionKind::Ready
        && (physical_capacity
            || input.output_allocation == Some(TaskBudgetOutputAllocation::AvailableCeiling));
    let reserved_output_tokens = if use_available_ceiling {
        available_output_tokens
    } else {
        output_tokens_for(input.stage, &input.demand, selected_quantity)
    };
    if use_available_ceiling {
        reasons.push(TaskBudgetReason {
            code: TaskBudgetReasonCode::OutputAllocationCeiling,
            value_tokens: Some(reserved_output_tokens),
            selected: true,
        });
    }
    let reasoning_upper_bound_tokens = match total_envelope_tokens {
        None => fixed_reasoning_tokens,
        Some(total_envelope_tokens) => (total_envelope_tokens
            - input.input_estimate.upper_bound_tokens
            - reserved_output_tokens
            - input.safety_margin_tokens)
            .max(0),
    };
    let reservation_liability_tokens = match total_envelope_tokens {
        Some(total_envelope_tokens) => total_envelope_tokens,
        None => {
            input.input_estimate.upper_bound_tokens
                + reserved_output_tokens
                + reasoning_upper_bound_tokens
                + input.safety_margin_tokens
        }
    };
    if decision_kind == TaskBudgetDecisionKind::SplitRequired {
        reasons.push(TaskBudgetReason {
            code: TaskBudgetReasonCode::ScopeSplit,
            value_tokens: Some(reserved_output_tokens),
            selected: true,
        });
    }
    Ok(TaskBudgetDecision {
        policy_version: S07_TASK_BUDGET_POLICY.version,
        output_estimator_version: S07_TASK_BUDGET_POLICY.output_estimator_version,
        input_estimator_version: input.input_estimate.estimator_version.clone(),
        decision: decision_kind,
        stage: input.stage,
        writing_language: input.demand.writing_language().to_string(),
        demand_kind: input.demand.kind(),
        requested_quantity,
        selected_quantity,
        remaining_quantity: requested_quantity - selected_quantity,
        requested_output_tokens,
        reserved_output_tokens,
        reasoning_upper_bound_tokens,
        reservation_liability_tokens,
        input_upper_bound_tokens: input.input_estimate.upper_bound_tokens,
        reasons,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 基线测试 fixture（`__tests__/task-budget-planner.test.ts`）。
    fn fixture(demand: TaskBudgetDemand) -> TaskBudgetPlannerInput {
        let stage = match demand {
            TaskBudgetDemand::DraftUnits { .. } => GenerationReasoningStage::Drafting,
            TaskBudgetDemand::StructuredItems { .. } => GenerationReasoningStage::Planning,
        };
        TaskBudgetPlannerInput {
            stage,
            demand,
            input_estimate: TaskBudgetInputEstimate {
                upper_bound_tokens: 1_000,
                estimator_version: "fixture-input-v1".to_string(),
            },
            capability: TaskBudgetCapabilityConstraints {
                model_context_window_tokens: Some(131_072),
                model_max_output_tokens: Some(32_768),
                model_context_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                model_output_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                user_context_window_tokens: None,
                user_max_output_tokens: None,
            },
            root: TaskBudgetRootLimits {
                remaining_token_liability: 2_097_152,
                max_output_per_request: 32_768,
            },
            liability: TaskBudgetLiabilityBound::IncludedInOutput,
            output_allocation: None,
            safety_margin_tokens: 512,
        }
    }

    fn chinese_draft(requested_units: i64, segmentable: bool) -> TaskBudgetDemand {
        TaskBudgetDemand::DraftUnits {
            writing_language: "zh-CN".to_string(),
            requested_units,
            segmentable,
        }
    }

    fn chinese_items(requested_items: i64) -> TaskBudgetDemand {
        TaskBudgetDemand::StructuredItems {
            writing_language: "zh-CN".to_string(),
            requested_items,
        }
    }

    #[test]
    fn keeps_semantic_sizing_separate_from_available_physical_allocation() {
        let decision = plan_task_budget(TaskBudgetPlannerInput {
            output_allocation: Some(TaskBudgetOutputAllocation::AvailableCeiling),
            liability: TaskBudgetLiabilityBound::Unknown {
                reasoning_upper_bound_tokens: None,
            },
            root: TaskBudgetRootLimits {
                remaining_token_liability: 100_000,
                max_output_per_request: 16_384,
            },
            ..fixture(chinese_draft(500, false))
        })
        .unwrap();
        assert_eq!(decision.decision, TaskBudgetDecisionKind::Ready);
        assert_eq!(decision.requested_quantity, 500);
        assert_eq!(decision.selected_quantity, 500);
        assert_eq!(decision.requested_output_tokens, 1_712);
        assert_eq!(decision.reserved_output_tokens, 16_384);
        assert_eq!(decision.reservation_liability_tokens, 17_896);
        assert!(decision.reasons.contains(&TaskBudgetReason {
            code: TaskBudgetReasonCode::LiabilityBoundUnknown,
            value_tokens: None,
            selected: true,
        }));
    }

    #[test]
    fn preserves_scope_splits_before_allocating_a_physical_request() {
        let input = TaskBudgetPlannerInput {
            root: TaskBudgetRootLimits {
                remaining_token_liability: 100_000,
                max_output_per_request: 16_384,
            },
            ..fixture(chinese_items(200))
        };
        let with_allocation = TaskBudgetPlannerInput {
            output_allocation: Some(TaskBudgetOutputAllocation::AvailableCeiling),
            ..input.clone()
        };
        assert_eq!(
            plan_task_budget(with_allocation).unwrap(),
            plan_task_budget(input).unwrap()
        );
    }

    #[test]
    fn plans_the_complete_chinese_target_without_a_universal_4k_cap() {
        for (units, tokens) in [(900, 2_672), (2_000, 5_312), (3_000, 7_712)] {
            let decision = plan_task_budget(TaskBudgetPlannerInput {
                root: TaskBudgetRootLimits {
                    remaining_token_liability: 100_000,
                    max_output_per_request: 8_192,
                },
                ..fixture(chinese_draft(units, true))
            })
            .unwrap();
            assert_eq!(decision.policy_version, "s07-task-budget-v1");
            assert_eq!(decision.decision, TaskBudgetDecisionKind::Ready);
            assert_eq!(decision.requested_quantity, units);
            assert_eq!(decision.selected_quantity, units);
            assert_eq!(decision.requested_output_tokens, tokens);
            assert_eq!(decision.reserved_output_tokens, tokens);
        }
    }

    #[test]
    fn splits_a_segmentable_draft_before_a_historical_4k_request_ceiling() {
        let decision = plan_task_budget(TaskBudgetPlannerInput {
            root: TaskBudgetRootLimits {
                remaining_token_liability: 100_000,
                max_output_per_request: 4_096,
            },
            ..fixture(chinese_draft(3_000, true))
        })
        .unwrap();
        assert_eq!(decision.decision, TaskBudgetDecisionKind::SplitRequired);
        assert_eq!(decision.requested_quantity, 3_000);
        assert_eq!(decision.selected_quantity, 1_493);
        assert_eq!(decision.remaining_quantity, 1_507);
        assert_eq!(decision.reserved_output_tokens, 4_096);
        assert!(decision.reasons.contains(&TaskBudgetReason {
            code: TaskBudgetReasonCode::ScopeSplit,
            value_tokens: Some(4_096),
            selected: true,
        }));
    }

    #[test]
    fn does_not_silently_segment_a_task_that_cannot_preserve_segment_candidates() {
        let decision = plan_task_budget(TaskBudgetPlannerInput {
            root: TaskBudgetRootLimits {
                remaining_token_liability: 100_000,
                max_output_per_request: 4_096,
            },
            ..fixture(chinese_draft(3_000, false))
        })
        .unwrap();
        assert_eq!(decision.decision, TaskBudgetDecisionKind::CapacityConflict);
        assert_eq!(decision.reserved_output_tokens, 0);
        assert!(decision.reasons.contains(&TaskBudgetReason {
            code: TaskBudgetReasonCode::DraftSegmentationDisabled,
            value_tokens: None,
            selected: true,
        }));
    }

    #[test]
    fn uses_a_user_operational_limit_without_upgrading_unknown_model_capability() {
        let decision = plan_task_budget(TaskBudgetPlannerInput {
            capability: TaskBudgetCapabilityConstraints {
                model_context_window_tokens: None,
                model_max_output_tokens: None,
                model_context_source: TaskBudgetCapabilitySource::Unknown,
                model_output_source: TaskBudgetCapabilitySource::Unknown,
                user_context_window_tokens: Some(131_072),
                user_max_output_tokens: Some(131_072),
            },
            ..fixture(chinese_draft(900, true))
        })
        .unwrap();
        assert_eq!(decision.decision, TaskBudgetDecisionKind::Ready);
        assert_eq!(decision.selected_quantity, 900);
        assert_eq!(decision.reservation_liability_tokens, 4_184);
        assert!(decision.reasons.contains(&TaskBudgetReason {
            code: TaskBudgetReasonCode::ModelCapabilityUnknown,
            value_tokens: None,
            selected: true,
        }));
    }

    #[test]
    fn uses_the_verified_model_limit_even_when_the_user_setting_is_128k() {
        let decision = plan_task_budget(TaskBudgetPlannerInput {
            capability: TaskBudgetCapabilityConstraints {
                model_context_window_tokens: Some(500_000),
                model_max_output_tokens: Some(8_192),
                model_context_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                model_output_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                user_context_window_tokens: Some(131_072),
                user_max_output_tokens: Some(131_072),
            },
            ..fixture(chinese_draft(5_000, true))
        })
        .unwrap();
        assert_eq!(decision.decision, TaskBudgetDecisionKind::SplitRequired);
        assert_eq!(decision.selected_quantity, 3_200);
        assert_eq!(decision.reserved_output_tokens, 8_192);
        assert!(decision.reasons.contains(&TaskBudgetReason {
            code: TaskBudgetReasonCode::ModelOutputCap,
            value_tokens: Some(8_192),
            selected: true,
        }));
    }

    #[test]
    fn splits_200_structured_items_into_complete_non_overlapping_ranges() {
        let mut batches: Vec<i64> = Vec::new();
        let mut remaining = 200;
        while remaining > 0 {
            let decision = plan_task_budget(fixture(chinese_items(remaining))).unwrap();
            assert!(decision.selected_quantity > 0);
            batches.push(decision.selected_quantity);
            remaining = decision.remaining_quantity;
        }
        assert_eq!(batches, vec![10; 20]);
        assert_eq!(batches.iter().sum::<i64>(), 200);
    }

    #[test]
    fn sizes_a_16k_operational_cap_to_five_dense_blueprint_items_per_request() {
        let decision = plan_task_budget(TaskBudgetPlannerInput {
            capability: TaskBudgetCapabilityConstraints {
                model_context_window_tokens: Some(1_000_000),
                model_max_output_tokens: Some(393_000),
                model_context_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                model_output_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                user_context_window_tokens: Some(65_536),
                user_max_output_tokens: Some(16_384),
            },
            ..fixture(chinese_items(200))
        })
        .unwrap();
        assert_eq!(decision.decision, TaskBudgetDecisionKind::SplitRequired);
        assert_eq!(decision.selected_quantity, 5);
        assert_eq!(decision.remaining_quantity, 195);
        assert_eq!(decision.reserved_output_tokens, 15_872);
    }

    #[test]
    fn freezes_the_dense_chinese_blueprint_fixture_behind_structured_planning_sizing() {
        // 基线另断言 fixture 序列化的 utf16/utf8/非 ASCII 计数——那是
        // 测试装置自证（与规划器纯函数无关），此处仅移植规划器断言。
        let decision = plan_task_budget(fixture(chinese_items(1))).unwrap();
        assert_eq!(decision.requested_output_tokens, 3_584);
    }

    #[test]
    fn explains_when_one_structured_item_cannot_fit_beside_required_input() {
        let decision = plan_task_budget(TaskBudgetPlannerInput {
            capability: TaskBudgetCapabilityConstraints {
                model_context_window_tokens: Some(4_096),
                model_max_output_tokens: Some(4_096),
                model_context_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                model_output_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                user_context_window_tokens: None,
                user_max_output_tokens: None,
            },
            input_estimate: TaskBudgetInputEstimate {
                upper_bound_tokens: 2_500,
                estimator_version: "fixture-input-v1".to_string(),
            },
            ..fixture(chinese_items(1))
        })
        .unwrap();
        assert_eq!(decision.decision, TaskBudgetDecisionKind::CapacityConflict);
        assert!(decision.reasons.contains(&TaskBudgetReason {
            code: TaskBudgetReasonCode::SingleItemCapacityConflict,
            value_tokens: None,
            selected: true,
        }));
        assert!(decision
            .reasons
            .iter()
            .any(|reason| reason.code == TaskBudgetReasonCode::RequiredInputCapacityConflict));
    }

    #[test]
    fn keeps_independent_reasoning_overhead_in_the_root_reservation() {
        let decision = plan_task_budget(TaskBudgetPlannerInput {
            root: TaskBudgetRootLimits {
                remaining_token_liability: 10_000,
                max_output_per_request: 32_768,
            },
            liability: TaskBudgetLiabilityBound::SeparateBounded {
                reasoning_upper_bound_tokens: 4_096,
            },
            ..fixture(chinese_items(1))
        })
        .unwrap();
        assert_eq!(decision.decision, TaskBudgetDecisionKind::Ready);
        assert_eq!(decision.requested_output_tokens, 3_584);
        assert_eq!(decision.reserved_output_tokens, 3_584);
        assert_eq!(decision.reasoning_upper_bound_tokens, 4_096);
        assert_eq!(decision.reservation_liability_tokens, 9_192);
    }

    #[test]
    fn does_not_double_count_reasoning_included_in_completion_usage() {
        let decision = plan_task_budget(TaskBudgetPlannerInput {
            liability: TaskBudgetLiabilityBound::IncludedInOutput,
            ..fixture(chinese_items(5))
        })
        .unwrap();
        assert_eq!(decision.reasoning_upper_bound_tokens, 0);
        assert_eq!(
            decision.reservation_liability_tokens,
            decision.input_upper_bound_tokens + decision.reserved_output_tokens + 512
        );
    }

    #[test]
    fn preserves_a_protocol_total_liability_envelope_instead_of_inventing_a_reasoning_cap() {
        let decision = plan_task_budget(TaskBudgetPlannerInput {
            liability: TaskBudgetLiabilityBound::TotalBounded {
                total_liability_upper_bound_tokens: 32_768,
            },
            ..fixture(chinese_draft(900, true))
        })
        .unwrap();
        assert_eq!(decision.decision, TaskBudgetDecisionKind::Ready);
        assert_eq!(decision.reservation_liability_tokens, 32_768);
        assert_eq!(
            decision.reasoning_upper_bound_tokens,
            32_768 - decision.input_upper_bound_tokens - decision.reserved_output_tokens - 512
        );
    }

    #[test]
    fn does_not_shrink_an_exact_protocol_liability_bound_to_a_conservative_model_capacity() {
        let decision = plan_task_budget(TaskBudgetPlannerInput {
            capability: TaskBudgetCapabilityConstraints {
                model_context_window_tokens: Some(1_000_000),
                model_max_output_tokens: Some(393_000),
                model_context_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                model_output_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                user_context_window_tokens: Some(65_536),
                user_max_output_tokens: Some(16_384),
            },
            root: TaskBudgetRootLimits {
                remaining_token_liability: 2_000_000,
                max_output_per_request: 32_768,
            },
            liability: TaskBudgetLiabilityBound::TotalBounded {
                total_liability_upper_bound_tokens: 1_048_576,
            },
            ..fixture(chinese_draft(900, true))
        })
        .unwrap();
        assert_eq!(decision.decision, TaskBudgetDecisionKind::Ready);
        assert_eq!(decision.reservation_liability_tokens, 1_048_576);
        assert!(decision.reasons.contains(&TaskBudgetReason {
            code: TaskBudgetReasonCode::ProtocolTotalBound,
            value_tokens: Some(1_048_576),
            selected: true,
        }));
    }

    #[test]
    fn subtracts_separately_bounded_reasoning_from_the_model_context_capacity() {
        let decision = plan_task_budget(TaskBudgetPlannerInput {
            capability: TaskBudgetCapabilityConstraints {
                model_context_window_tokens: Some(12_000),
                model_max_output_tokens: Some(8_192),
                model_context_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                model_output_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                user_context_window_tokens: None,
                user_max_output_tokens: None,
            },
            input_estimate: TaskBudgetInputEstimate {
                upper_bound_tokens: 3_000,
                estimator_version: "fixture-input-v1".to_string(),
            },
            root: TaskBudgetRootLimits {
                remaining_token_liability: 100_000,
                max_output_per_request: 32_768,
            },
            liability: TaskBudgetLiabilityBound::SeparateBounded {
                reasoning_upper_bound_tokens: 8_192,
            },
            ..fixture(chinese_draft(900, true))
        })
        .unwrap();
        assert_eq!(decision.decision, TaskBudgetDecisionKind::CapacityConflict);
        assert!(decision.reasons.contains(&TaskBudgetReason {
            code: TaskBudgetReasonCode::ModelContextCap,
            value_tokens: Some(296),
            selected: true,
        }));
    }

    #[test]
    fn records_unknown_liability_and_refuses_an_unaffordable_known_envelope() {
        let unknown = plan_task_budget(TaskBudgetPlannerInput {
            liability: TaskBudgetLiabilityBound::Unknown {
                reasoning_upper_bound_tokens: None,
            },
            ..fixture(chinese_draft(900, true))
        })
        .unwrap();
        assert!(unknown.reasons.contains(&TaskBudgetReason {
            code: TaskBudgetReasonCode::LiabilityBoundUnknown,
            value_tokens: None,
            selected: true,
        }));

        let exhausted = plan_task_budget(TaskBudgetPlannerInput {
            root: TaskBudgetRootLimits {
                remaining_token_liability: 8_000,
                max_output_per_request: 32_768,
            },
            liability: TaskBudgetLiabilityBound::TotalBounded {
                total_liability_upper_bound_tokens: 32_768,
            },
            ..fixture(chinese_draft(900, true))
        })
        .unwrap();
        assert_eq!(exhausted.decision, TaskBudgetDecisionKind::CapacityConflict);
        assert!(exhausted.reasons.contains(&TaskBudgetReason {
            code: TaskBudgetReasonCode::RootRemainingCap,
            value_tokens: Some(8_000),
            selected: true,
        }));
    }

    #[test]
    fn reports_a_fully_exhausted_parent_budget_instead_of_treating_zero_as_malformed_input() {
        let decision = plan_task_budget(TaskBudgetPlannerInput {
            root: TaskBudgetRootLimits {
                remaining_token_liability: 0,
                max_output_per_request: 32_768,
            },
            ..fixture(chinese_draft(900, true))
        })
        .unwrap();
        assert_eq!(decision.decision, TaskBudgetDecisionKind::CapacityConflict);
        assert!(decision.reasons.contains(&TaskBudgetReason {
            code: TaskBudgetReasonCode::RootRemainingCap,
            value_tokens: Some(0),
            selected: true,
        }));
    }

    #[test]
    fn uses_the_explicit_writing_language_for_structured_item_estimates() {
        let chinese = plan_task_budget(fixture(chinese_items(5))).unwrap();
        let english = plan_task_budget(fixture(TaskBudgetDemand::StructuredItems {
            writing_language: "en-US".to_string(),
            requested_items: 5,
        }))
        .unwrap();
        assert_eq!(chinese.requested_output_tokens, 15_872);
        assert_eq!(english.requested_output_tokens, 10_752);
    }

    // ── 属性不变量（基线 task-budget-planner.property.test.ts）──
    // fast-check 属性测试不移植（禁新依赖红线）；以固定样本集循环
    // 断言同等安全不变量。

    /// 固定样本集：语言 × 阶段 × 需求 × 责任模式 × 能力配置。
    fn planner_sample_inputs() -> Vec<TaskBudgetPlannerInput> {
        let mut inputs = Vec::new();
        for &language in &["zh-CN", "en-US"] {
            for &stage in &[
                GenerationReasoningStage::Drafting,
                GenerationReasoningStage::Planning,
                GenerationReasoningStage::Review,
                GenerationReasoningStage::General,
            ] {
                for &(is_draft, quantity) in &[
                    (true, 1_i64),
                    (true, 7),
                    (true, 500),
                    (true, 3_000),
                    (false, 1),
                    (false, 5),
                    (false, 200),
                ] {
                    let demand = if is_draft {
                        TaskBudgetDemand::DraftUnits {
                            writing_language: language.to_string(),
                            requested_units: quantity,
                            segmentable: true,
                        }
                    } else {
                        TaskBudgetDemand::StructuredItems {
                            writing_language: language.to_string(),
                            requested_items: quantity,
                        }
                    };
                    for &liability in &[
                        TaskBudgetLiabilityBound::IncludedInOutput,
                        TaskBudgetLiabilityBound::SeparateBounded {
                            reasoning_upper_bound_tokens: 4_096,
                        },
                        TaskBudgetLiabilityBound::TotalBounded {
                            total_liability_upper_bound_tokens: 1_048_576,
                        },
                        TaskBudgetLiabilityBound::Unknown {
                            reasoning_upper_bound_tokens: None,
                        },
                        TaskBudgetLiabilityBound::Unknown {
                            reasoning_upper_bound_tokens: Some(1_024),
                        },
                    ] {
                        for &(model_context, model_output) in &[
                            (Some(131_072_i64), Some(32_768_i64)),
                            (Some(12_000), Some(8_192)),
                            (None, None),
                        ] {
                            // 规划器要求两个来源标记与取值一致。
                            let model_context_source = if model_context.is_none() {
                                TaskBudgetCapabilitySource::Unknown
                            } else {
                                TaskBudgetCapabilitySource::VerifiedProviderPreset
                            };
                            let model_output_source = if model_output.is_none() {
                                TaskBudgetCapabilitySource::Unknown
                            } else {
                                TaskBudgetCapabilitySource::VerifiedProviderPreset
                            };
                            inputs.push(TaskBudgetPlannerInput {
                                stage,
                                demand: demand.clone(),
                                input_estimate: TaskBudgetInputEstimate {
                                    upper_bound_tokens: 1_000,
                                    estimator_version: "property-v1".to_string(),
                                },
                                capability: TaskBudgetCapabilityConstraints {
                                    model_context_window_tokens: model_context,
                                    model_max_output_tokens: model_output,
                                    model_context_source,
                                    model_output_source,
                                    user_context_window_tokens: Some(131_072),
                                    user_max_output_tokens: Some(131_072),
                                },
                                root: TaskBudgetRootLimits {
                                    remaining_token_liability: 2_097_152,
                                    max_output_per_request: 32_768,
                                },
                                liability,
                                output_allocation: None,
                                safety_margin_tokens: 512,
                            });
                        }
                    }
                }
            }
        }
        inputs
    }

    #[test]
    fn never_reserves_more_than_the_remaining_root_liability() {
        for input in planner_sample_inputs() {
            let root_remaining = input.root.remaining_token_liability;
            let root_max_output = input.root.max_output_per_request;
            let decision = plan_task_budget(input).unwrap();
            if decision.decision != TaskBudgetDecisionKind::Ready {
                continue;
            }
            assert!(decision.reservation_liability_tokens <= root_remaining);
            assert!(decision.reserved_output_tokens <= root_max_output);
            assert!(decision.reserved_output_tokens > 0);
        }
    }

    #[test]
    fn holds_both_safety_invariants_on_dispatchable_requests() {
        // 基线另断言 ready 样本数 > 500（生成器退化守卫）；本样本集
        // 全量统计等价验证。
        let mut ready = 0;
        for input in planner_sample_inputs() {
            let root_remaining = input.root.remaining_token_liability;
            let root_max_output = input.root.max_output_per_request;
            let decision = plan_task_budget(input).unwrap();
            if decision.decision != TaskBudgetDecisionKind::Ready {
                continue;
            }
            ready += 1;
            assert!(decision.reservation_liability_tokens <= root_remaining);
            assert!(decision.reserved_output_tokens <= root_max_output);
            assert!(decision.reserved_output_tokens > 0);
        }
        assert!(ready > 500, "ready samples: {ready}");
    }

    #[test]
    fn keeps_quantities_and_coverage_arithmetically_consistent() {
        for input in planner_sample_inputs() {
            let decision = plan_task_budget(input).unwrap();
            assert!(decision.selected_quantity >= 0);
            assert!(decision.selected_quantity <= decision.requested_quantity);
            assert_eq!(
                decision.remaining_quantity,
                decision.requested_quantity - decision.selected_quantity
            );
            if decision.decision == TaskBudgetDecisionKind::CapacityConflict {
                // 失败的决策绝不允许留下任何预留。
                assert_eq!(decision.reserved_output_tokens, 0);
                assert_eq!(decision.reservation_liability_tokens, 0);
            }
        }
    }

    #[test]
    fn preserves_unknown_capability_without_inventing_a_model_bound() {
        for input in planner_sample_inputs() {
            let model_max_output_none = input.capability.model_max_output_tokens.is_none();
            let model_context_none = input.capability.model_context_window_tokens.is_none();
            if !model_max_output_none && !model_context_none {
                continue;
            }
            let decision = plan_task_budget(input).unwrap();
            assert!(decision
                .reasons
                .iter()
                .any(|reason| reason.code == TaskBudgetReasonCode::ModelCapabilityUnknown));
            if model_max_output_none {
                assert!(!decision
                    .reasons
                    .iter()
                    .any(|reason| reason.code == TaskBudgetReasonCode::ModelOutputCap));
            }
            if model_context_none {
                assert!(!decision
                    .reasons
                    .iter()
                    .any(|reason| reason.code == TaskBudgetReasonCode::ModelContextCap));
            }
        }
    }

    #[test]
    fn keeps_unknown_liability_visible_while_respecting_the_root_estimate() {
        for mut input in planner_sample_inputs() {
            // 能力未知会先命中它自己的冲突，这里隔离协议责任这一条。
            if input.capability.model_context_window_tokens.is_none()
                || input.capability.model_max_output_tokens.is_none()
            {
                continue;
            }
            input.liability = TaskBudgetLiabilityBound::Unknown {
                reasoning_upper_bound_tokens: None,
            };
            let root_remaining = input.root.remaining_token_liability;
            let decision = plan_task_budget(input).unwrap();
            assert!(decision
                .reasons
                .iter()
                .any(|reason| reason.code == TaskBudgetReasonCode::LiabilityBoundUnknown));
            assert!(decision.reservation_liability_tokens <= root_remaining);
        }
    }

    #[test]
    fn is_a_pure_function_of_its_input() {
        for input in planner_sample_inputs() {
            let first = plan_task_budget(input.clone()).unwrap();
            let second = plan_task_budget(input).unwrap();
            assert_eq!(first, second);
        }
    }

    // ── 补充用例（基线测试文件未覆盖非法输入；fail 路径为 1:1 移植
    // 语义，验证入口校验与错误码逐字）──

    #[test]
    fn rejects_malformed_inputs_with_task_budget_input_invalid() {
        // 非法写作语言。
        assert_eq!(
            plan_task_budget(TaskBudgetPlannerInput {
                demand: TaskBudgetDemand::StructuredItems {
                    writing_language: "ja-JP".to_string(),
                    requested_items: 1,
                },
                ..fixture(chinese_items(1))
            })
            .unwrap_err(),
            "TASK_BUDGET_INPUT_INVALID"
        );
        // 空白估计器版本。
        assert_eq!(
            plan_task_budget(TaskBudgetPlannerInput {
                input_estimate: TaskBudgetInputEstimate {
                    upper_bound_tokens: 1_000,
                    estimator_version: "  ".to_string(),
                },
                ..fixture(chinese_items(1))
            })
            .unwrap_err(),
            "TASK_BUDGET_INPUT_INVALID"
        );
        // 非正需求量。
        assert_eq!(
            plan_task_budget(TaskBudgetPlannerInput {
                demand: TaskBudgetDemand::StructuredItems {
                    writing_language: "zh-CN".to_string(),
                    requested_items: 0,
                },
                ..fixture(chinese_items(1))
            })
            .unwrap_err(),
            "TASK_BUDGET_INPUT_INVALID"
        );
        // 负的输入上界。
        assert_eq!(
            plan_task_budget(TaskBudgetPlannerInput {
                input_estimate: TaskBudgetInputEstimate {
                    upper_bound_tokens: -1,
                    estimator_version: "fixture-input-v1".to_string(),
                },
                ..fixture(chinese_items(1))
            })
            .unwrap_err(),
            "TASK_BUDGET_INPUT_INVALID"
        );
        // 负的根剩余额度。
        assert_eq!(
            plan_task_budget(TaskBudgetPlannerInput {
                root: TaskBudgetRootLimits {
                    remaining_token_liability: -1,
                    max_output_per_request: 32_768,
                },
                ..fixture(chinese_items(1))
            })
            .unwrap_err(),
            "TASK_BUDGET_INPUT_INVALID"
        );
        // 非正的单请求上限。
        assert_eq!(
            plan_task_budget(TaskBudgetPlannerInput {
                root: TaskBudgetRootLimits {
                    remaining_token_liability: 100_000,
                    max_output_per_request: 0,
                },
                ..fixture(chinese_items(1))
            })
            .unwrap_err(),
            "TASK_BUDGET_INPUT_INVALID"
        );
        // 负的安全余量。
        assert_eq!(
            plan_task_budget(TaskBudgetPlannerInput {
                safety_margin_tokens: -1,
                ..fixture(chinese_items(1))
            })
            .unwrap_err(),
            "TASK_BUDGET_INPUT_INVALID"
        );
        // 能力取值非正。
        assert_eq!(
            plan_task_budget(TaskBudgetPlannerInput {
                capability: TaskBudgetCapabilityConstraints {
                    model_context_window_tokens: Some(0),
                    model_max_output_tokens: Some(32_768),
                    model_context_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                    model_output_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                    user_context_window_tokens: None,
                    user_max_output_tokens: None,
                },
                ..fixture(chinese_items(1))
            })
            .unwrap_err(),
            "TASK_BUDGET_INPUT_INVALID"
        );
        // 来源标记与取值不一致（取值已知但来源为 unknown）。
        assert_eq!(
            plan_task_budget(TaskBudgetPlannerInput {
                capability: TaskBudgetCapabilityConstraints {
                    model_context_window_tokens: Some(131_072),
                    model_max_output_tokens: Some(32_768),
                    model_context_source: TaskBudgetCapabilitySource::Unknown,
                    model_output_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                    user_context_window_tokens: None,
                    user_max_output_tokens: None,
                },
                ..fixture(chinese_items(1))
            })
            .unwrap_err(),
            "TASK_BUDGET_INPUT_INVALID"
        );
        // 来源标记与取值不一致（取值未知但来源为已验证预设）。
        assert_eq!(
            plan_task_budget(TaskBudgetPlannerInput {
                capability: TaskBudgetCapabilityConstraints {
                    model_context_window_tokens: None,
                    model_max_output_tokens: None,
                    model_context_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                    model_output_source: TaskBudgetCapabilitySource::VerifiedProviderPreset,
                    user_context_window_tokens: None,
                    user_max_output_tokens: None,
                },
                ..fixture(chinese_items(1))
            })
            .unwrap_err(),
            "TASK_BUDGET_INPUT_INVALID"
        );
        // separate-bounded 负推理上界。
        assert_eq!(
            plan_task_budget(TaskBudgetPlannerInput {
                liability: TaskBudgetLiabilityBound::SeparateBounded {
                    reasoning_upper_bound_tokens: -1,
                },
                ..fixture(chinese_items(1))
            })
            .unwrap_err(),
            "TASK_BUDGET_INPUT_INVALID"
        );
        // total-bounded 非正总上界。
        assert_eq!(
            plan_task_budget(TaskBudgetPlannerInput {
                liability: TaskBudgetLiabilityBound::TotalBounded {
                    total_liability_upper_bound_tokens: 0,
                },
                ..fixture(chinese_items(1))
            })
            .unwrap_err(),
            "TASK_BUDGET_INPUT_INVALID"
        );
    }
}
