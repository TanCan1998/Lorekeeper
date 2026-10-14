//! 计划构造器 —— 1:1 移植 `electron/services/main-generation-plan.ts`（328 行）。
//!
//! 纯函数面：把冻结的根预算策略、规划 scope、语义任务契约与
//! 物理预留计划缝合成一个 `MainGenerationPlan`。本波**不接 owner
//! 调用点**（begin 主体在 B3-3），不激活任何 fail-closed 占位。
//!
//! # 移植说明（基线语义逐字，错误码 / 数值常量 / 加权值不改写）
//! - 基线 `throw new Error('CODE')` 统一按 crate 惯例以
//!   `Result<_, String>` 建模，错误码字符串逐字保留；
//! - 基线 `number` 在本域为整数 token 计数；`scaledRootBudget` /
//!   `batchRootBudget` 的基线浮点算术 + 出口安全整数检查以 `f64`
//!   1:1 建模（对齐 `blueprint_batch_policy` 的 `chapterCount: f64`
//!   先例），其余计数以 `i64` 建模；
//! - `isDeepStrictEqual` 以 `serde_json::Value` 的 `PartialEq`
//!   等价（对象键序无关，与 JS 深比较语义一致）。
//!
//! # 刻意偏离（登记）
//! 1. **选择子与模型档案以松散 `serde_json::Value` 传递**：基线
//!    `BeginGenerationRequest` / `ModelProfile` 在 Rust 侧无 1:1
//!    等价物（`agent_generation.rs` 的 owner 投影同样以 `Value`
//!    保真）；且 `ModelProfile.reasoningMapping` 字段不在
//!    `LeaseModelProfile` 投影中，`canBound` 的
//!    `!model.reasoningMapping` 检查需原始 JSON 访问。内部按需
//!    以 `LeaseModelProfile::from_value` 构造类型化投影。
//! 2. **`PlanningGenerationScope` / `ArchitecturePlanningIntent` /
//!    `DirectoryRequestedRange` / `PlanningStep` 在本模块 1:1 建模**
//!    （基线来自 `src/shared/generation-owner-contract`，Rust 侧
//!    无等价类型）。
//! 3. **`MainGenerationOptions` 保留计划的最小序列化投影**：共享
//!    `LlmGenerateOptions` 已具备 `output_token_parameter`，B3-3
//!    接线时透传；`conversationId` 在计划构造路径从不设置。
//! 4. **`task_budget_policy` 以全保真 `Value` 携带**：
//!    `TaskBudgetPolicy`（B3-1a）无 serde derive（该波裁决），
//!    `readMainGenerationPolicy` 的 deep-equal 门禁需要其 JSON
//!    形状；数值逐字取自 `S07_TASK_BUDGET_POLICY` 常量字段。
//! 5. **`MAIN_GENERATION_POLICY` 冻结常量以构造函数
//!    `main_generation_policy()` 等价提供**：`Value` 字段不可常量
//!    构造；每次调用返回等价副本，语义与冻结常量一致。
//! 6. **共享依赖已统一**：`token_liability` / 规划单位校验来自
//!    `commands/generation.rs`，章节默认值与协议常量来自既有服务。
//!    f64 输入仅做字符串薄适配；`PLOT_OUTLINE_CONTENT` 因 Value
//!    不可常量构造而保留构造函数。
//! 7. **B3-M2 推理机器已对齐**：五种指令、SiliconFlow 目录与
//!    手动映射均由共享解析层消费；历史未移植断言仍保留说明，
//!    不将其计入本波已测覆盖。
//! 8. **`TaskBudgetPreflightError` 携带 `TaskBudgetDecision`**（基线
//!    语义），但按 crate 惯例以 `String`（`Display`，前缀逐字）
//!    传播；决策载荷由公开类型携带，B3-3 接线时可按需提升为
//!    typed error。
//! 9. 基线 `Number(text)` / `Number(config?.totalChapters)` 强制
//!    转换以 `js_number_text` / `js_number_value` 建模；非数值域
//!    （NaN / 缺失）在基线与 Rust 侧均拒绝（错误码等价）。
//! 10. 基线 `receipt: Pick<ModelExecutionLeaseReceipt,
//!     'capabilityEvidence'>` 以直接 `&ModelExecutionCapabilityEvidence`
//!     投影等价；`creativeStrategy` / `policy` 基线默认参数
//!     （`'auto'` / `MAIN_GENERATION_POLICY`）以显式传参等价。
//! 11. 基线 `new URL(model.baseUrl)` 对非法 URL 抛 `TypeError`；
//!     Rust 侧以 `Result` 携带 url crate 错误文案（合法档案不受
//!     影响）。

#![allow(dead_code)] // 本波不接调用点（B3-3 接线时消费；对齐 generation_parameter_policy.rs 惯例）

use crate::commands::generation::{
    assert_planning_action_range, parse_planning_target_units, token_liability,
};
use crate::llm::generation_parameter_policy::{
    resolve_generation_capability_constraints, resolve_generation_parameters,
    GenerationParameterRequest, ResolvedGenerationParameters,
};
use crate::llm::lease::{LeaseModelProfile, ModelExecutionCapabilityEvidence};
use crate::llm::presets::MAX_SAFE_INTEGER;
use crate::llm::reasoning::{
    CreativeStrategy, GenerationReasoningStage, ProviderReasoningDirective, ThinkingState,
};
#[cfg(test)]
use crate::repositories::generation_run_repository::PhysicalAttempt;
use crate::repositories::generation_run_repository::{
    GenerationBudgetReceipt, PlotOutlineRange, ProviderUsagePolicy, RootBudget,
};
use crate::repositories::project_core_repository::CREATIVE_STRATEGIES;
use crate::services::agent_generation::DEFAULT_PLANNING_ACTION_CHAPTERS;
use crate::services::blueprint_batch_policy::plan_blueprint_generation_cost;
use crate::services::generation_source_binding::PLOT_OUTLINE_PROTOCOL;
use crate::services::generation_task::{
    GenerationMessageRole, GenerationOutput, GenerationReasoningStage as TaskReasoningStage,
    GenerationTask,
};
use crate::services::prompt_budget_failure::format_task_budget_decision_failure;
use crate::services::task_budget_planner::{
    plan_task_budget, TaskBudgetCapabilityConstraints, TaskBudgetDecision, TaskBudgetDecisionKind,
    TaskBudgetDemand, TaskBudgetInputEstimate, TaskBudgetLiabilityBound,
    TaskBudgetOutputAllocation, TaskBudgetPlannerInput, TaskBudgetRootLimits,
    S07_TASK_BUDGET_POLICY,
};
use regex::Regex;
use serde_json::{json, Value};
use std::sync::OnceLock;

// ── 错误码（基线 `new Error('CODE')` 字面量，逐字）──

const GENERATION_POLICY_UNSUPPORTED: &str = "GENERATION_POLICY_UNSUPPORTED";
const GENERATION_PLANNING_BUDGET_INVALID: &str = "GENERATION_PLANNING_BUDGET_INVALID";
const GENERATION_PLANNING_INTENT_INVALID: &str = "GENERATION_PLANNING_INTENT_INVALID";
const GENERATION_PLANNING_OPERATION_NOT_SELECTED: &str =
    "GENERATION_PLANNING_OPERATION_NOT_SELECTED";
const GENERATION_PLANNING_TARGET_INVALID: &str = "GENERATION_PLANNING_TARGET_INVALID";
const GENERATION_BATCH_INTENT_INVALID: &str = "GENERATION_BATCH_INTENT_INVALID";
const GENERATION_SEMANTIC_TASK_INVALID: &str = "GENERATION_SEMANTIC_TASK_INVALID";
const GENERATION_INPUT_CAPACITY_EXCEEDED: &str = "GENERATION_INPUT_CAPACITY_EXCEEDED";
const ROOT_BUDGET_EXHAUSTED: &str = "ROOT_BUDGET_EXHAUSTED";

/// 基线 `PLOT_OUTLINE_CONTENT`（`{ maxAttemptsPerChapter: 2 }`）。
fn plot_outline_content_value() -> Value {
    json!({ "maxAttemptsPerChapter": 2 })
}

/// 基线 `Number.isSafeInteger`（±2^53-1 内的整数）。
fn is_safe_integer(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && value.abs() <= MAX_SAFE_INTEGER as f64
}

/// 基线 `Number.isSafeInteger` 的 `Value` 形式。
fn safe_integer_value(value: &Value) -> Option<i64> {
    let number = value.as_i64()?;
    (number.abs() <= MAX_SAFE_INTEGER as i64).then_some(number)
}

/// 基线 `Number(text)` 的字符串形式（非数值 → `None`，下游按
/// NaN 域拒绝）。
fn js_number_text(text: &str) -> Option<f64> {
    let text = text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    for (lower, upper, radix) in [("0x", "0X", 16), ("0b", "0B", 2), ("0o", "0O", 8)] {
        if let Some(digits) = text
            .strip_prefix(lower)
            .or_else(|| text.strip_prefix(upper))
        {
            if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
                return None;
            }
            return u64::from_str_radix(digits, radix)
                .ok()
                .map(|number| number as f64);
        }
    }
    if text.is_empty() {
        Some(0.0)
    } else {
        text.parse::<f64>().ok()
    }
}

/// 基线 `Number(value)` 的 `Value` 形式（数字原样、字符串解析、
/// 其余域在基线为 NaN/0，下游检查均拒绝）。
fn js_number_value(value: &Value) -> Option<f64> {
    if let Some(number) = value.as_f64() {
        Some(number)
    } else {
        value.as_str().and_then(js_number_text)
    }
}

// ── 策略常量与类型 ──────────────────────────────────────────

/// 基线 `MainGenerationPolicy` 类型。
#[derive(Debug, Clone, PartialEq)]
pub struct MainGenerationPolicy {
    pub version: String,
    /// 基线 `taskBudgetPolicy`（`S07_TASK_BUDGET_POLICY` 冻结常量）——
    /// 全保真 `Value`（见模块头注第 4 条）。
    pub task_budget_policy: Value,
    pub capability_policy_version: String,
    pub budget: RootBudget,
    pub estimator_version: String,
    pub safety_margin_tokens: i64,
    pub planning: Option<PlanningGenerationScope>,
}

impl MainGenerationPolicy {
    /// 保持规范 JSON 合同，复用既有规划联合与预算的投影，不复制序列化分支。
    pub fn to_value(&self) -> Value {
        let budget = serde_json::to_value(&self.budget).expect("根预算可序列化");
        let mut value = policy_to_value_with(&self.version, &budget, self.planning.as_ref());
        let object = value.as_object_mut().expect("策略投影是对象");
        object.insert(
            "taskBudgetPolicy".to_string(),
            self.task_budget_policy.clone(),
        );
        object.insert(
            "capabilityPolicyVersion".to_string(),
            json!(self.capability_policy_version),
        );
        object.insert(
            "estimatorVersion".to_string(),
            json!(self.estimator_version),
        );
        object.insert(
            "safetyMarginTokens".to_string(),
            json!(self.safety_margin_tokens),
        );
        value
    }
}

/// 基线 `MAIN_GENERATION_POLICY` 冻结常量
/// （`version: 's07-task-and-liability-v1'`）——见模块头注第 5 条。
pub fn main_generation_policy() -> MainGenerationPolicy {
    MainGenerationPolicy {
        version: "s07-task-and-liability-v1".to_string(),
        task_budget_policy: s07_task_budget_policy_value(),
        capability_policy_version: "verified-endpoint-model-budget-v1".to_string(),
        budget: RootBudget {
            max_physical_requests: 32,
            max_token_liability: 2_097_152,
            max_output_per_request: 32_768,
            max_active_elapsed_ms: 3_600_000,
        },
        estimator_version: "utf8-bytes-plus-chat-framing-v1".to_string(),
        safety_margin_tokens: 512,
        planning: None,
    }
}

/// 基线 `S07_TASK_BUDGET_POLICY` 的 JSON 形状（全保真 `Value`；
/// 数值逐字取自常量字段，键名按基线 camelCase 契约）。
fn s07_task_budget_policy_value() -> Value {
    let policy = &S07_TASK_BUDGET_POLICY;
    let draft_units = &policy.draft_units;
    let structured_items = &policy.structured_items;
    json!({
        "version": policy.version,
        "outputEstimatorVersion": policy.output_estimator_version,
        "draftUnits": {
            "baseTokens": draft_units.base_tokens,
            "tokensPerUnit": {
                "zh-CN": {
                    "numerator": draft_units.tokens_per_unit_zh_cn.numerator,
                    "denominator": draft_units.tokens_per_unit_zh_cn.denominator,
                },
                "en-US": {
                    "numerator": draft_units.tokens_per_unit_en_us.numerator,
                    "denominator": draft_units.tokens_per_unit_en_us.denominator,
                },
            },
        },
        "structuredItems": {
            "baseTokens": structured_items.base_tokens,
            "tokensPerItem": {
                "zh-CN": {
                    "drafting": structured_items.tokens_per_item_zh_cn.drafting,
                    "planning": structured_items.tokens_per_item_zh_cn.planning,
                    "review": structured_items.tokens_per_item_zh_cn.review,
                    "general": structured_items.tokens_per_item_zh_cn.general,
                },
                "en-US": {
                    "drafting": structured_items.tokens_per_item_en_us.drafting,
                    "planning": structured_items.tokens_per_item_en_us.planning,
                    "review": structured_items.tokens_per_item_en_us.review,
                    "general": structured_items.tokens_per_item_en_us.general,
                },
            },
        },
    })
}

/// 基线 `MAIN_GENERATION_POLICY.budget` 的 JSON 形状。
fn main_generation_policy_budget_value() -> Value {
    json!({
        "maxPhysicalRequests": 32,
        "maxTokenLiability": 2_097_152,
        "maxOutputPerRequest": 32_768,
        "maxActiveElapsedMs": 3_600_000,
    })
}

/// 基线 `scaledRootBudget`（私有）。
///
/// 基线以 `number` 实参做浮点算术并在出口做安全整数检查；
/// Rust 侧以 `f64` 1:1 建模该检查。
fn scaled_root_budget(
    requests: f64,
    max_output_per_request: f64,
    per_request_liability: f64,
) -> Result<RootBudget, String> {
    let policy = main_generation_policy();
    let base = &policy.budget;
    let max_physical_requests = (base.max_physical_requests as f64).max(requests);
    let max_token_liability = per_request_liability * max_physical_requests;
    let max_active_elapsed_ms = base.max_active_elapsed_ms as f64
        / base.max_physical_requests as f64
        * max_physical_requests;
    for value in [
        max_physical_requests,
        max_token_liability,
        max_output_per_request,
        max_active_elapsed_ms,
    ] {
        if !is_safe_integer(value) || value <= 0.0 {
            return Err(GENERATION_PLANNING_BUDGET_INVALID.to_string());
        }
    }
    Ok(RootBudget {
        max_physical_requests: max_physical_requests as i64,
        max_token_liability: max_token_liability as i64,
        max_output_per_request: max_output_per_request as i64,
        max_active_elapsed_ms: max_active_elapsed_ms as i64,
    })
}

// ── 规划 scope（基线 generation-owner-contract 形状的 1:1 建模）──

/// 基线 `ArchitecturePlanningIntent`。
#[derive(Debug, Clone, PartialEq)]
pub struct ArchitecturePlanningIntent {
    pub prior_steps: Vec<PlanningStep>,
    pub synopsis_range: Option<PlotOutlineRange>,
}

/// 基线 `priorSteps` 元素联合（`'premise' | 'characters' | 'worldbuilding'`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningStep {
    Premise,
    Characters,
    Worldbuilding,
}

impl PlanningStep {
    fn as_str(self) -> &'static str {
        match self {
            PlanningStep::Premise => "premise",
            PlanningStep::Characters => "characters",
            PlanningStep::Worldbuilding => "worldbuilding",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "premise" => Some(PlanningStep::Premise),
            "characters" => Some(PlanningStep::Characters),
            "worldbuilding" => Some(PlanningStep::Worldbuilding),
            _ => None,
        }
    }
}

/// 基线 `DirectoryGenerationProgress.requestedRange` 的
/// `mode` 联合（`'full' | 'append'`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryMode {
    Full,
    Append,
}

impl DirectoryMode {
    fn as_str(self) -> &'static str {
        match self {
            DirectoryMode::Full => "full",
            DirectoryMode::Append => "append",
        }
    }
}

/// 基线目录 `requestedRange`（`{ mode, startChapter, endChapter }`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectoryRequestedRange {
    pub mode: DirectoryMode,
    pub start_chapter: i64,
    pub end_chapter: i64,
}

/// 基线 `PlanningGenerationScope` 三态联合。
#[derive(Debug, Clone, PartialEq)]
pub enum PlanningGenerationScope {
    /// 基线 `{ kind: 'architecture', intent, outlineProtocol: 'legacy-range-v1' }`。
    ArchitectureLegacyRange { intent: ArchitecturePlanningIntent },
    /// 基线 `{ kind: 'architecture', intent, outlineProtocol: PLOT_OUTLINE_PROTOCOL, outlineContent: PLOT_OUTLINE_CONTENT }`。
    ArchitectureOutline { intent: ArchitecturePlanningIntent },
    /// 基线 `{ kind: 'directory', requestedRange }`。
    Directory {
        requested_range: DirectoryRequestedRange,
    },
}

fn architecture_intent_value(intent: &ArchitecturePlanningIntent) -> Value {
    json!({
        "version": "architecture-action-v1",
        "priorSteps": intent.prior_steps.iter().map(|step| step.as_str()).collect::<Vec<_>>(),
        "synopsisRange": intent.synopsis_range.map_or(Value::Null, |range| json!({
            "from": range.from,
            "to": range.to,
        })),
    })
}

fn planning_scope_to_value(scope: &PlanningGenerationScope) -> Value {
    match scope {
        PlanningGenerationScope::ArchitectureLegacyRange { intent } => json!({
            "kind": "architecture",
            "intent": architecture_intent_value(intent),
            "outlineProtocol": "legacy-range-v1",
        }),
        PlanningGenerationScope::ArchitectureOutline { intent } => json!({
            "kind": "architecture",
            "intent": architecture_intent_value(intent),
            "outlineProtocol": PLOT_OUTLINE_PROTOCOL,
            "outlineContent": plot_outline_content_value(),
        }),
        PlanningGenerationScope::Directory { requested_range } => json!({
            "kind": "directory",
            "requestedRange": {
                "mode": requested_range.mode.as_str(),
                "startChapter": requested_range.start_chapter,
                "endChapter": requested_range.end_chapter,
            },
        }),
    }
}

// ── 私有解析辅助（基线 main-generation-plan.ts 私有函数 1:1）──

/// 基线 `planningRecord`（私有）。
fn planning_record(value: &Value) -> Result<&Value, String> {
    if !value.is_object() {
        return Err(GENERATION_PLANNING_INTENT_INVALID.to_string());
    }
    Ok(value)
}

/// 基线 `planningInput`（私有）：按 id 过滤 `authorInputs`，
/// `JSON.parse` 唯一匹配项的文本为对象。
fn planning_input(selection: &Value, id: &str) -> Result<Option<Value>, String> {
    let inputs: Vec<&Value> = selection
        .get("authorInputs")
        .and_then(Value::as_array)
        .map(|array| {
            array
                .iter()
                .filter(|input| input.get("id").and_then(Value::as_str) == Some(id))
                .collect()
        })
        .unwrap_or_default();
    if inputs.is_empty() {
        return Ok(None);
    }
    if inputs.len() != 1 {
        return Err(GENERATION_PLANNING_INTENT_INVALID.to_string());
    }
    let text = inputs[0]
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match serde_json::from_str::<Value>(text) {
        Ok(value) if value.is_object() => Ok(Some(value)),
        _ => Err(GENERATION_PLANNING_INTENT_INVALID.to_string()),
    }
}

/// 基线 `planningRange`（私有）：严格数字类型的安全整数，
/// `from ≥ 1` 且 `to ≥ from`。
fn planning_range(from: &Value, to: &Value) -> Result<PlotOutlineRange, String> {
    let from = safe_integer_value(from).ok_or(GENERATION_PLANNING_INTENT_INVALID.to_string())?;
    let to = safe_integer_value(to).ok_or(GENERATION_PLANNING_INTENT_INVALID.to_string())?;
    if from < 1 || to < from {
        return Err(GENERATION_PLANNING_INTENT_INVALID.to_string());
    }
    Ok(PlotOutlineRange { from, to })
}

/// 基线 `architectureIntent`（私有）：意图形状与精确键集校验。
fn architecture_intent(value: &Value) -> Result<ArchitecturePlanningIntent, String> {
    planning_record(value)?;
    const ORDER: [PlanningStep; 3] = [
        PlanningStep::Premise,
        PlanningStep::Characters,
        PlanningStep::Worldbuilding,
    ];
    if value.get("version").and_then(Value::as_str) != Some("architecture-action-v1") {
        return Err(GENERATION_PLANNING_INTENT_INVALID.to_string());
    }
    let steps = match value.get("priorSteps") {
        Some(steps) if steps.is_array() => steps.clone(),
        _ => return Err(GENERATION_PLANNING_INTENT_INVALID.to_string()),
    };
    let steps: Vec<String> = steps
        .as_array()
        .unwrap()
        .iter()
        .map(|step| {
            step.as_str()
                .map(|step| step.to_string())
                .ok_or_else(|| GENERATION_PLANNING_INTENT_INVALID.to_string())
        })
        .collect::<Result<_, _>>()?;
    // 基线 `isDeepStrictEqual(steps, order.filter(step => steps.includes(step)))`：
    // 严格子集序（无重复、无未知值、顺序与目录一致）。
    let filtered: Vec<PlanningStep> = ORDER
        .iter()
        .filter(|step| steps.iter().any(|candidate| candidate == step.as_str()))
        .copied()
        .collect();
    if steps.iter().map(String::as_str).collect::<Vec<_>>()
        != filtered
            .iter()
            .map(|step| step.as_str())
            .collect::<Vec<_>>()
    {
        return Err(GENERATION_PLANNING_INTENT_INVALID.to_string());
    }
    let synopsis_range = match value.get("synopsisRange") {
        Some(Value::Null) => None,
        Some(range) if range.is_object() => Some(planning_range(
            range.get("from").unwrap_or(&Value::Null),
            range.get("to").unwrap_or(&Value::Null),
        )?),
        _ => return Err(GENERATION_PLANNING_INTENT_INVALID.to_string()),
    };
    // 基线：`!intent.priorSteps.length && !intent.synopsisRange` → 拒绝。
    if filtered.is_empty() && synopsis_range.is_none() {
        return Err(GENERATION_PLANNING_INTENT_INVALID.to_string());
    }
    // 基线 `isDeepStrictEqual(value, intent)`：精确键集
    // `{ version, priorSteps, synopsisRange }`。
    let expected = json!({
        "version": "architecture-action-v1",
        "priorSteps": filtered.iter().map(|step| step.as_str()).collect::<Vec<_>>(),
        "synopsisRange": synopsis_range.map_or(Value::Null, |range| json!({
            "from": range.from,
            "to": range.to,
        })),
    });
    if value != &expected {
        return Err(GENERATION_PLANNING_INTENT_INVALID.to_string());
    }
    Ok(ArchitecturePlanningIntent {
        prior_steps: filtered,
        synopsis_range,
    })
}

/// 已解析数值的薄适配，单位范围统一由共享校验器判断。
fn parse_planning_target_units_f64(value: Option<f64>) -> Result<i64, String> {
    let text = value.map(|number| number.to_string());
    parse_planning_target_units(text.as_deref())
}

/// begin 与策略构造共用的目标单位校验，缺省值只用于未提供目标时。
pub(crate) fn planning_target_units(selection: &Value) -> Result<i64, String> {
    let targets: Vec<&Value> = author_inputs(selection)
        .filter(|input| input.get("id").and_then(Value::as_str) == Some("planning:target-units"))
        .collect();
    if targets.len() > 1 {
        return Err(GENERATION_PLANNING_TARGET_INVALID.to_string());
    }
    let Some(target) = targets.first() else {
        return parse_planning_target_units_f64(None);
    };
    let number = js_number_value(target.get("text").unwrap_or(&Value::Null))
        .ok_or_else(|| GENERATION_PLANNING_TARGET_INVALID.to_string())?;
    parse_planning_target_units_f64(Some(number))
}

/// 基线 `authorInputs` 迭代（缺失时为空）。
fn author_inputs(selection: &Value) -> impl Iterator<Item = &Value> {
    selection
        .get("authorInputs")
        .and_then(Value::as_array)
        .map(|array| array.iter())
        .into_iter()
        .flatten()
}

// ── 规划 scope 构造（基线 planningGenerationScope）──

/// 基线 `planningGenerationScope`：
/// `BeginGenerationRequest{operation, authorInputs}` →
/// `PlanningGenerationScope | null`。
pub fn planning_generation_scope(
    selection: &Value,
) -> Result<Option<PlanningGenerationScope>, String> {
    if selection.get("operation").and_then(Value::as_str) == Some("chapter-blueprint-directory") {
        let range = planning_input(selection, "directory:requested-range")?;
        let config = planning_input(selection, "directory:author-config")?;
        let (Some(range), Some(config)) = (range, config) else {
            return Err(GENERATION_PLANNING_INTENT_INVALID.to_string());
        };
        let mode = range.get("mode").and_then(Value::as_str);
        if !matches!(mode, Some("full") | Some("append")) {
            return Err(GENERATION_PLANNING_INTENT_INVALID.to_string());
        }
        let parsed = planning_range(
            range.get("startChapter").unwrap_or(&Value::Null),
            range.get("endChapter").unwrap_or(&Value::Null),
        )?;
        assert_planning_action_range(parsed)?;
        planning_range(
            &json!(parsed.to),
            config.get("totalChapters").unwrap_or(&Value::Null),
        )?;
        // 基线 `isDeepStrictEqual(range, requestedRange)`：精确键集
        // `{ mode, startChapter, endChapter }`（`mode` 必须为字面量字符串）。
        let expected = json!({
            "mode": mode,
            "startChapter": parsed.from,
            "endChapter": parsed.to,
        });
        if range != expected {
            return Err(GENERATION_PLANNING_INTENT_INVALID.to_string());
        }
        return Ok(Some(PlanningGenerationScope::Directory {
            requested_range: DirectoryRequestedRange {
                mode: if mode == Some("full") {
                    DirectoryMode::Full
                } else {
                    DirectoryMode::Append
                },
                start_chapter: parsed.from,
                end_chapter: parsed.to,
            },
        }));
    }
    let step = match selection.get("operation").and_then(Value::as_str) {
        Some("generate-core-seed") => "premise",
        Some("character-architecture") => "characters",
        Some("generate-world-building") => "worldbuilding",
        Some("generate-plot-outline") => "synopsis",
        _ => return Ok(None),
    };
    let provided = planning_input(selection, "architecture:planning-intent")?;
    let config = planning_input(selection, "architecture:author-config")?;
    let effective = match provided {
        Some(provided) => provided,
        None => {
            // 基线默认意图（未提供 planning-intent 时）：
            // `Math.min(Number(config?.totalChapters), DEFAULT_PLANNING_ACTION_CHAPTERS)`。
            let total_chapters = config.as_ref().and_then(|config| {
                js_number_value(config.get("totalChapters").unwrap_or(&Value::Null))
            });
            let synopsis_to = match total_chapters {
                Some(number) => {
                    let clamped = number.min(DEFAULT_PLANNING_ACTION_CHAPTERS as f64);
                    if is_safe_integer(clamped) {
                        json!(clamped as i64)
                    } else {
                        // 非整数域（基线 NaN/小数）：planningRange 拒绝。
                        Value::Null
                    }
                }
                None => Value::Null,
            };
            json!({
                "version": "architecture-action-v1",
                "priorSteps": if step == "synopsis" {
                    Value::Array(vec![])
                } else {
                    json!([step])
                },
                "synopsisRange": if step == "synopsis" {
                    json!({ "from": 1, "to": synopsis_to })
                } else {
                    Value::Null
                },
            })
        }
    };
    let intent = architecture_intent(&effective)?;
    if step == "synopsis" {
        if intent.synopsis_range.is_none() {
            return Err(GENERATION_PLANNING_OPERATION_NOT_SELECTED.to_string());
        }
    } else if !intent
        .prior_steps
        .iter()
        .any(|candidate| candidate.as_str() == step)
    {
        return Err(GENERATION_PLANNING_OPERATION_NOT_SELECTED.to_string());
    }
    if let Some(range) = intent.synopsis_range {
        // 基线 `planningRange(intent.synopsisRange.to, config?.totalChapters)`。
        let total_chapters = config.as_ref().map_or(Value::Null, |config| {
            config.get("totalChapters").cloned().unwrap_or(Value::Null)
        });
        planning_range(&json!(range.to), &total_chapters)?;
        assert_planning_action_range(range)?;
    }
    Ok(Some(PlanningGenerationScope::ArchitectureOutline {
        intent,
    }))
}

// ── 物理责任解析（基线 resolvePhysicalLiability）──

/// 基线 `resolvePhysicalLiability` 的返回对象（私有）。
#[derive(Debug, Clone)]
struct PhysicalLiability {
    capability: TaskBudgetCapabilityConstraints,
    parameters: ResolvedGenerationParameters,
    usage_policy: ProviderUsagePolicy,
    reservation_ceiling: i64,
    effective_context: Option<i64>,
    model_output: Option<i64>,
    openai: bool,
    gemini: bool,
    total_bounded: bool,
    can_bound: bool,
    separate_reasoning: i64,
    gemini_reasoning: Option<i64>,
    compatible_reasoning: Option<i64>,
}

fn creative_strategy_name(strategy: CreativeStrategy) -> &'static str {
    match strategy {
        CreativeStrategy::Auto => "auto",
        CreativeStrategy::FluentDrafting => "fluent-drafting",
        CreativeStrategy::ConsistencyFirst => "consistency-first",
        CreativeStrategy::DeepPlanning => "deep-planning",
    }
}

fn reasoning_stage_name(stage: GenerationReasoningStage) -> &'static str {
    match stage {
        GenerationReasoningStage::Drafting => "drafting",
        GenerationReasoningStage::Planning => "planning",
        GenerationReasoningStage::Review => "review",
        GenerationReasoningStage::General => "general",
    }
}

/// 任务侧阶段枚举 → 策略缝阶段枚举（两个 Rust 枚举分别 1:1
/// 对应基线同一 TS 联合的两个消费面）。
fn policy_stage(stage: TaskReasoningStage) -> GenerationReasoningStage {
    match stage {
        TaskReasoningStage::Drafting => GenerationReasoningStage::Drafting,
        TaskReasoningStage::Planning => GenerationReasoningStage::Planning,
        TaskReasoningStage::Review => GenerationReasoningStage::Review,
        TaskReasoningStage::General => GenerationReasoningStage::General,
    }
}

/// 基线 `model.reasoningMapping` 真值检查（手动映射存在且非 null）。
fn has_reasoning_mapping(model: &Value) -> bool {
    model
        .get("reasoningMapping")
        .map_or(false, |mapping| !mapping.is_null())
}

/// 基线 SiliconFlow DeepSeek-V4 模型名联合
/// （`/^(?:Pro\/)?deepseek-ai\/DeepSeek-V4(?:-Flash|-Pro)?(?:-\d{4})?$/iu`）。
fn siliconflow_v4_model_name() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?i)^(?:Pro/)?deepseek-ai/DeepSeek-V4(?:-Flash|-Pro)?(?:-\d{4})?$")
            .expect("siliconflow V4 模型名正则编译失败")
    })
}

/// 基线 `resolvePhysicalLiability`（私有）：provider 能力分析、
/// usagePolicy 构造与预留上限计算。
fn resolve_physical_liability(
    model: &Value,
    creative_strategy: CreativeStrategy,
    stage: GenerationReasoningStage,
) -> Result<PhysicalLiability, String> {
    // 基线 `new URL(model.baseUrl)`；非法 URL 在基线抛 TypeError。
    let endpoint = url::Url::parse(
        model
            .get("baseUrl")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )
    .map_err(|error| format!("GENERATION_MODEL_ENDPOINT_INVALID: {error}"))?;
    let official = endpoint.scheme() == "https"
        && endpoint.username().is_empty()
        && endpoint.password().is_none()
        && endpoint.port().is_none();
    let host = if official {
        endpoint.host_str().unwrap_or_default()
    } else {
        ""
    };
    let openai =
        host == "api.openai.com" && model.get("protocol").and_then(Value::as_str) == Some("openai");
    let deepseek = host == "api.deepseek.com"
        && model.get("protocol").and_then(Value::as_str) == Some("openai");
    let gemini = host == "generativelanguage.googleapis.com"
        && model.get("protocol").and_then(Value::as_str) == Some("gemini");
    let silicon_v4 = ["api.siliconflow.cn", "api.siliconflow.com"].contains(&host)
        && model.get("protocol").and_then(Value::as_str) == Some("openai")
        && siliconflow_v4_model_name().is_match(
            model
                .get("modelName")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
    let silicon_qwen = host == "api.siliconflow.cn"
        && model.get("protocol").and_then(Value::as_str) == Some("openai")
        && endpoint.path().trim_end_matches('/') == "/v1"
        && endpoint.query().map_or(true, |query| query.is_empty())
        && endpoint
            .fragment()
            .map_or(true, |fragment| fragment.is_empty())
        && model.get("modelName").and_then(Value::as_str) == Some("Qwen/Qwen3.8-27B");
    let capability =
        resolve_generation_capability_constraints(&LeaseModelProfile::from_value(model));
    let model_context = capability.model_context_window_tokens;
    let model_output = capability.model_max_output_tokens;
    let known_capacity = model_context.is_some() && model_output.is_some();
    let safety = main_generation_policy().safety_margin_tokens;
    let parameters = resolve_generation_parameters(
        &LeaseModelProfile::from_value(model),
        &GenerationParameterRequest {
            max_tokens: None,
            response_format: None,
            creative_strategy: Some(creative_strategy_name(creative_strategy)),
            reasoning_stage: Some(reasoning_stage_name(stage)),
        },
    )?;
    let gemini_reasoning = match parameters.reasoning {
        Some(ProviderReasoningDirective::GeminiThinkingBudget { thinking_budget }) => {
            Some(thinking_budget)
        }
        _ => None,
    };
    // 基线 `openai-thinking-budget` adapter 的 `thinkingBudget`
    // （G4-B3-M 裁决，B3-M2 推理机器对齐：adapter 变体已移植，
    // 手动映射与 Qwen 目录映射均经指令机器解析）。
    let compatible_reasoning = match parameters.reasoning {
        Some(ProviderReasoningDirective::OpenAiThinkingBudget { thinking_budget }) => {
            Some(thinking_budget)
        }
        _ => None,
    };
    // Qwen 的可用容量保持 256K。在 provider 截断策略下保守预留模型
    // 卡声明的扩容 1M 包络：
    // https://docs.siliconflow.cn/docs/userguide/capabilities/reasoning
    // https://huggingface.co/Qwen/Qwen3.8-27B
    // 这是工程余量，不是经验证计的计费上限。
    let total_bounded =
        compatible_reasoning.is_none() && known_capacity && (silicon_v4 || silicon_qwen);
    // 目录提供上界，而非使用兼容模型的许可。缺失上界产生运营估计
    // 并保持未知。
    let can_bound = compatible_reasoning.is_none()
        && !has_reasoning_mapping(model)
        && known_capacity
        && (openai
            || deepseek
            || silicon_v4
            || (gemini && gemini_reasoning.map_or(false, |value| value >= 0)));
    let separate_reasoning = compatible_reasoning.or(gemini_reasoning).unwrap_or(0);
    let usage_policy = ProviderUsagePolicy {
        estimator_version: main_generation_policy().estimator_version.to_string(),
        safety_margin_tokens: safety,
        reasoning: if !can_bound {
            "unknown".to_string()
        } else if model.get("protocol").and_then(Value::as_str) == Some("gemini") {
            "separately-billed".to_string()
        } else {
            "included-in-completion".to_string()
        },
        can_bound_total_liability: can_bound,
    };
    // 基线 `Math.min(modelContext ?? Infinity, userContextWindowTokens ?? Infinity)`。
    let effective_context = match (model_context, capability.user_context_window_tokens) {
        (Some(model_context), Some(user_context)) => Some(model_context.min(user_context)),
        (Some(model_context), None) => Some(model_context),
        (None, Some(user_context)) => Some(user_context),
        (None, None) => None,
    };
    let reservation_ceiling = if total_bounded {
        1_048_576
    } else {
        effective_context.unwrap_or(main_generation_policy().budget.max_token_liability)
    };
    Ok(PhysicalLiability {
        capability,
        parameters,
        usage_policy,
        reservation_ceiling,
        effective_context,
        model_output,
        openai,
        gemini,
        total_bounded,
        can_bound,
        separate_reasoning,
        gemini_reasoning,
        compatible_reasoning,
    })
}

// ── 策略构造（基线 newMainGenerationPolicy）──

/// 基线 `newMainGenerationPolicy`：选择子 + 模型档案 →
/// `s07-capacity-v2` 策略（物理请求数按批量 / 蓝图 / 架构步骤
/// 加权，token 责任按 4 策略 × 4 阶段的最大预留上限缩放）。
pub fn new_main_generation_policy(
    selection: &Value,
    model: &Value,
) -> Result<MainGenerationPolicy, String> {
    let planning = planning_generation_scope(selection)?;
    if planning.is_some() {
        planning_target_units(selection)?;
    }
    let limits = resolve_generation_capability_constraints(&LeaseModelProfile::from_value(model));
    let user_max_output_tokens = limits
        .user_max_output_tokens
        .ok_or(GENERATION_PLANNING_BUDGET_INVALID.to_string())?;
    let model_max_output_tokens = limits
        .model_max_output_tokens
        .unwrap_or(user_max_output_tokens);
    let cap = user_max_output_tokens.min(model_max_output_tokens);
    // 基线：批量意图按章数；目录蓝图按成本包络；架构按步骤加权
    // （premise=1、worldbuilding=2、characters=3+(2*8-ceil(8/3))+1=17）
    // 加细纲每章 2；其余默认 32。
    let requests = if selection
        .get("batchIntent")
        .map_or(false, |intent| !intent.is_null())
    {
        let range = selection
            .get("batchIntent")
            .and_then(|intent| intent.get("range"))
            .unwrap_or(&Value::Null);
        let end =
            js_number_value(range.get("endChapter").unwrap_or(&Value::Null)).unwrap_or(f64::NAN);
        let start =
            js_number_value(range.get("startChapter").unwrap_or(&Value::Null)).unwrap_or(f64::NAN);
        end - start + 1.0
    } else if let Some(PlanningGenerationScope::Directory { requested_range }) = &planning {
        let chapter_count =
            (requested_range.end_chapter - requested_range.start_chapter + 1) as f64;
        plan_blueprint_generation_cost(chapter_count).recovery_call_bound as f64
    } else if let Some(PlanningGenerationScope::ArchitectureOutline { intent }) = &planning {
        let mut requests: f64 = intent
            .prior_steps
            .iter()
            .map(|step| match step {
                PlanningStep::Premise => 1.0,
                PlanningStep::Worldbuilding => 2.0,
                PlanningStep::Characters => 3.0 + (2.0 * 8.0 - (8.0 / 3.0f64).ceil()) + 1.0,
            })
            .sum();
        if let Some(range) = intent.synopsis_range {
            requests += 2.0 * (range.to - range.from + 1) as f64;
        }
        requests
    } else {
        main_generation_policy().budget.max_physical_requests as f64
    };
    // 基线：4 策略 × 4 阶段 `resolvePhysicalLiability` 的
    // `reservationCeiling` 取最大。
    let mut liability = i64::MIN;
    for stage in [
        GenerationReasoningStage::Drafting,
        GenerationReasoningStage::Planning,
        GenerationReasoningStage::Review,
        GenerationReasoningStage::General,
    ] {
        for strategy_name in CREATIVE_STRATEGIES {
            let ceiling = resolve_physical_liability(
                model,
                CreativeStrategy::parse(Some(strategy_name)),
                stage,
            )?
            .reservation_ceiling;
            liability = liability.max(ceiling);
        }
    }
    let scaled_requests = if selection
        .get("batchIntent")
        .map_or(false, |intent| !intent.is_null())
    {
        requests * BATCH_ROOT_PHYSICAL_REQUESTS_PER_CHAPTER as f64
    } else {
        requests
    };
    let budget = scaled_root_budget(scaled_requests, cap as f64, liability as f64)?;
    Ok(MainGenerationPolicy {
        version: "s07-capacity-v2".to_string(),
        task_budget_policy: s07_task_budget_policy_value(),
        capability_policy_version: main_generation_policy().capability_policy_version.clone(),
        budget,
        estimator_version: main_generation_policy().estimator_version.clone(),
        safety_margin_tokens: main_generation_policy().safety_margin_tokens,
        planning,
    })
}

// ── 策略反序列化门禁（基线 readMainGenerationPolicy）──

/// 基线 `readMainGenerationPolicy`：版本联合 + budget 键集/值域 +
/// planning 三态重构 + 全结构 deep-equal 门禁。
pub fn read_main_generation_policy(value: &Value) -> Result<MainGenerationPolicy, String> {
    let policy = main_generation_policy();
    let record = planning_record(value)?;
    if record.get("version").and_then(Value::as_str) == Some(policy.version.as_str())
        && value
            == &policy_to_value_with(
                &policy.version,
                &main_generation_policy_budget_value(),
                None,
            )
    {
        return Ok(policy);
    }
    let version = record.get("version").and_then(Value::as_str);
    if !matches!(
        version,
        Some("s07-planning-v1") | Some("s07-capacity-v2") | Some("s07-task-and-liability-v1")
    ) {
        return Err(GENERATION_POLICY_UNSUPPORTED.to_string());
    }
    let version = version.unwrap();
    let budget = planning_record(record.get("budget").unwrap_or(&Value::Null))?.clone();
    // 基线：`record.planning === undefined ? null : planningRecord(...)`
    // ——键缺失为 null；显式 null 被 planningRecord 拒绝。
    let scope = match record.get("planning") {
        None => None,
        Some(scope) => Some(planning_record(scope)?.clone()),
    };
    {
        let budget_object = budget.as_object().expect("planning_record 已保证为对象");
        let mut budget_keys: Vec<&str> = budget_object.keys().map(String::as_str).collect();
        budget_keys.sort_unstable();
        let mut expected_keys = vec![
            "maxActiveElapsedMs",
            "maxOutputPerRequest",
            "maxPhysicalRequests",
            "maxTokenLiability",
        ];
        expected_keys.sort_unstable();
        if budget_keys != expected_keys {
            return Err(GENERATION_PLANNING_BUDGET_INVALID.to_string());
        }
        if budget_object
            .values()
            .any(|number| !matches!(safe_integer_value(number), Some(number) if number > 0))
        {
            return Err(GENERATION_PLANNING_BUDGET_INVALID.to_string());
        }
    }
    let planning = if let Some(scope) = &scope {
        if scope.get("kind").and_then(Value::as_str) == Some("architecture")
            && scope.get("outlineProtocol").and_then(Value::as_str) == Some("legacy-range-v1")
        {
            Some(PlanningGenerationScope::ArchitectureLegacyRange {
                intent: architecture_intent(scope.get("intent").unwrap_or(&Value::Null))?,
            })
        } else if scope.get("kind").and_then(Value::as_str) == Some("architecture")
            && scope.get("outlineProtocol").and_then(Value::as_str) == Some(PLOT_OUTLINE_PROTOCOL)
            && scope
                .get("outlineContent")
                .map_or(false, |content| content == &plot_outline_content_value())
        {
            Some(PlanningGenerationScope::ArchitectureOutline {
                intent: architecture_intent(scope.get("intent").unwrap_or(&Value::Null))?,
            })
        } else if scope.get("kind").and_then(Value::as_str) == Some("directory") {
            let requested_range =
                planning_record(scope.get("requestedRange").unwrap_or(&Value::Null))?;
            let range = planning_range(
                requested_range.get("startChapter").unwrap_or(&Value::Null),
                requested_range.get("endChapter").unwrap_or(&Value::Null),
            )?;
            let mode = requested_range.get("mode").and_then(Value::as_str);
            if !matches!(mode, Some("full") | Some("append")) {
                return Err(GENERATION_PLANNING_INTENT_INVALID.to_string());
            }
            Some(PlanningGenerationScope::Directory {
                requested_range: DirectoryRequestedRange {
                    mode: if mode == Some("full") {
                        DirectoryMode::Full
                    } else {
                        DirectoryMode::Append
                    },
                    start_chapter: range.from,
                    end_chapter: range.to,
                },
            })
        } else {
            return Err(GENERATION_POLICY_UNSUPPORTED.to_string());
        }
    } else if version == "s07-planning-v1" {
        return Err(GENERATION_POLICY_UNSUPPORTED.to_string());
    } else {
        None
    };
    // 基线 `isDeepStrictEqual(value, { ...MAIN_GENERATION_POLICY, version, budget, ...(planning ? { planning } : {}) })`。
    let expected = policy_to_value_with(version, &budget, planning.as_ref());
    if value != &expected {
        return Err(GENERATION_POLICY_UNSUPPORTED.to_string());
    }
    Ok(MainGenerationPolicy {
        version: version.to_string(),
        task_budget_policy: s07_task_budget_policy_value(),
        capability_policy_version: policy.capability_policy_version.clone(),
        budget: RootBudget {
            max_physical_requests: safe_integer_value(
                budget.get("maxPhysicalRequests").unwrap_or(&Value::Null),
            )
            .unwrap_or_default(),
            max_token_liability: safe_integer_value(
                budget.get("maxTokenLiability").unwrap_or(&Value::Null),
            )
            .unwrap_or_default(),
            max_output_per_request: safe_integer_value(
                budget.get("maxOutputPerRequest").unwrap_or(&Value::Null),
            )
            .unwrap_or_default(),
            max_active_elapsed_ms: safe_integer_value(
                budget.get("maxActiveElapsedMs").unwrap_or(&Value::Null),
            )
            .unwrap_or_default(),
        },
        estimator_version: policy.estimator_version.clone(),
        safety_margin_tokens: policy.safety_margin_tokens,
        planning,
    })
}

/// 基线 deep-equal 重构：
/// `{ ...MAIN_GENERATION_POLICY, version, budget, ...(planning ? { planning } : {}) }`。
fn policy_to_value_with(
    version: &str,
    budget: &Value,
    planning: Option<&PlanningGenerationScope>,
) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("version".to_string(), json!(version));
    object.insert(
        "taskBudgetPolicy".to_string(),
        s07_task_budget_policy_value(),
    );
    object.insert(
        "capabilityPolicyVersion".to_string(),
        json!("verified-endpoint-model-budget-v1"),
    );
    object.insert("budget".to_string(), budget.clone());
    object.insert(
        "estimatorVersion".to_string(),
        json!("utf8-bytes-plus-chat-framing-v1"),
    );
    object.insert(
        "safetyMarginTokens".to_string(),
        json!(main_generation_policy().safety_margin_tokens),
    );
    if let Some(planning) = planning {
        object.insert("planning".to_string(), planning_scope_to_value(planning));
    }
    Value::Object(object)
}

// ── 批量根预算（基线 batchRootBudget）──

/// 基线 `BATCH_ROOT_PHYSICAL_REQUESTS_PER_CHAPTER`。
pub const BATCH_ROOT_PHYSICAL_REQUESTS_PER_CHAPTER: i64 = 8;

/// 基线 `batchRootBudget`：批量根预算按章数缩放（线程 10 复用
/// 原有写前调用余量）。
///
/// 批量创作的所有子运行（每章首稿、auto_finalize 的 notes/cards）
/// 共用批次根的一份预算。短细纲让 auto_finalize 每章最低物理请求
/// 从 3 次（首稿、notes、cards）变为 4 次，10 章最低 40 次，超过
/// 单根的 32 次上限，第 9 章起会确定性 ROOT_BUDGET_EXHAUSTED。
///
/// 每章预算 8 次 = 最低 4 次（细纲、首稿、notes、cards）+ 4 次余量，
/// 足够覆盖典型的一次压缩或续写加两次角色卡修复；余量在整批内共享。
/// 单章理论上限（细纲 1 + 首稿 1 + 续写/无进展恢复 7 + 压缩 1 +
/// notes 1 + cards 3 = 14 次）不按章预留：超出仍 fail-closed。
/// token 责任上限与活动时长按每次请求的同一比例（65 536 tokens、
/// 112.5 秒）缩放，使批量每次请求可用的额度与单根一致；单章非
/// 批量根不变。
pub fn batch_root_budget(chapter_count: f64) -> Result<RootBudget, String> {
    if !is_safe_integer(chapter_count) || chapter_count < 1.0 {
        return Err(GENERATION_BATCH_INTENT_INVALID.to_string());
    }
    scaled_root_budget(
        chapter_count * BATCH_ROOT_PHYSICAL_REQUESTS_PER_CHAPTER as f64,
        main_generation_policy().budget.max_output_per_request as f64,
        // 基线默认 perRequestLiability = maxTokenLiability / maxPhysicalRequests。
        main_generation_policy().budget.max_token_liability as f64
            / main_generation_policy().budget.max_physical_requests as f64,
    )
}

// ── 语义任务断言（基线 assertSemanticGenerationTask）──

/// 基线 `assertSemanticGenerationTask`。
///
/// 基线接收类型化 `GenerationTask` 并在运行时做对象形状守卫；
/// Rust 侧输入为松散 `Value`（对齐 `agent_generation` 的 owner
/// 投影），校验后反序列化为类型化 `GenerationTask` 返回——键集
/// 白名单因此保留为运行时校验（基线的 `?: never` 编译期守卫与
/// 运行时键集守卫在 Rust 侧合并于此）。
pub fn assert_semantic_generation_task(task: &Value) -> Result<GenerationTask, String> {
    let invalid = || GENERATION_SEMANTIC_TASK_INVALID.to_string();
    let purpose = task
        .get("purpose")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let messages_valid = task
        .get("messages")
        .and_then(Value::as_array)
        .map(|messages| {
            !messages.is_empty()
                && messages.len() <= 1024
                && messages.iter().all(|message| {
                    matches!(
                        message.get("role").and_then(Value::as_str),
                        Some("system") | Some("user") | Some("assistant")
                    ) && message.get("content").map_or(false, Value::is_string)
                })
        })
        .unwrap_or(false);
    if !task.is_object()
        || !semantic_purpose_pattern().is_match(purpose)
        || !matches!(
            task.get("output").and_then(Value::as_str),
            Some("visible-text") | Some("structured-data")
        )
        || !messages_valid
        || task
            .as_object()
            .expect("is_object 已保证")
            .keys()
            .any(|key| {
                !matches!(
                    key.as_str(),
                    "purpose"
                        | "output"
                        | "messages"
                        | "reasoningStage"
                        | "promptBudget"
                        | "budgetDemand"
                )
            })
        || task.get("reasoningStage").map_or(false, |stage| {
            !matches!(
                stage.as_str(),
                Some("drafting") | Some("planning") | Some("review") | Some("general")
            )
        })
    {
        return Err(invalid());
    }
    if let Some(demand) = task.get("budgetDemand") {
        // 基线第二段校验（判别联合）。
        let kind = demand.get("kind").and_then(Value::as_str);
        let allowed_keys: Vec<&str> = if kind == Some("draft-units") {
            vec!["kind", "writingLanguage", "requestedUnits", "segmentable"]
        } else {
            vec!["kind", "writingLanguage", "requestedItems"]
        };
        let demand_valid = demand.is_object()
            && matches!(kind, Some("draft-units") | Some("structured-items"))
            && matches!(
                demand.get("writingLanguage").and_then(Value::as_str),
                Some("zh-CN") | Some("en-US")
            )
            && demand
                .as_object()
                .expect("is_object 已保证")
                .keys()
                .all(|key| allowed_keys.contains(&key.as_str()))
            && match kind {
                Some("draft-units") => {
                    matches!(
                        safe_integer_value(demand.get("requestedUnits").unwrap_or(&Value::Null)),
                        Some(units) if units > 0
                    ) && demand.get("segmentable").map_or(false, Value::is_boolean)
                }
                Some("structured-items") => matches!(
                    safe_integer_value(demand.get("requestedItems").unwrap_or(&Value::Null)),
                    Some(items) if items > 0
                ),
                _ => false,
            };
        if !demand_valid {
            return Err(invalid());
        }
    }
    serde_json::from_value::<GenerationTask>(task.clone()).map_err(|_| invalid())
}

/// 基线 purpose 正则 `^[a-z0-9][a-z0-9:_-]{0,127}$`。
fn semantic_purpose_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"^[a-z0-9][a-z0-9:_-]{0,127}$").expect("语义任务 purpose 正则编译失败")
    })
}

/// 基线 `task.budgetDemand` 判别联合 → 类型化 `TaskBudgetDemand`
/// （`assert_semantic_generation_task` 已先行校验形状）。
fn parse_task_budget_demand(demand: &Value) -> Result<TaskBudgetDemand, String> {
    match demand.get("kind").and_then(Value::as_str) {
        Some("draft-units") => Ok(TaskBudgetDemand::DraftUnits {
            writing_language: demand
                .get("writingLanguage")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            requested_units: safe_integer_value(
                demand.get("requestedUnits").unwrap_or(&Value::Null),
            )
            .unwrap_or_default(),
            segmentable: demand
                .get("segmentable")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
        }),
        Some("structured-items") => Ok(TaskBudgetDemand::StructuredItems {
            writing_language: demand
                .get("writingLanguage")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            requested_items: safe_integer_value(
                demand.get("requestedItems").unwrap_or(&Value::Null),
            )
            .unwrap_or_default(),
        }),
        _ => Err(GENERATION_SEMANTIC_TASK_INVALID.to_string()),
    }
}

// ── 计划类型与预检错误 ──────────────────────────────────────

/// 基线 `LLMGenerateOptions`（`electron/llm/provider.interface.ts`）
/// 的最小投影——见模块头注第 3 条。
#[derive(Debug, Clone, PartialEq)]
pub struct MainGenerationOptions {
    /// `None` 表示 provider 必须省略该字段（基线 `undefined`）。
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    pub output_token_parameter: Option<&'static str>,
    pub response_format: Option<Value>,
    pub reasoning: Option<ProviderReasoningDirective>,
    pub conversation_id: Option<String>,
}

/// 基线 `MainGenerationPlan` 接口。
#[derive(Debug, Clone, PartialEq)]
pub struct MainGenerationPlan {
    pub budget_decision: Option<TaskBudgetDecision>,
    pub options: MainGenerationOptions,
    pub usage_policy: ProviderUsagePolicy,
    pub trusted_usage: bool,
    pub reserved_tokens: i64,
    pub input_upper_bound_tokens: i64,
    pub reasoning_upper_bound_tokens: i64,
    pub requested_output_tokens: i64,
}

/// 基线 `TaskBudgetPreflightError`：仅 main 创建的语义决策可穿过
/// IPC 错误过滤器。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBudgetPreflightError {
    pub decision: TaskBudgetDecision,
    message: String,
}

impl TaskBudgetPreflightError {
    /// 基线构造器：`split-required` →
    /// `TASK_BUDGET_SCOPE_SPLIT_REQUIRED:{selectedQuantity}`（无空格）；
    /// `capacity-conflict` → `TASK_BUDGET_CAPACITY_CONFLICT: {failure}`
    /// （含空格）。
    pub fn new(decision: TaskBudgetDecision) -> Self {
        let message = if decision.decision == TaskBudgetDecisionKind::SplitRequired {
            format!(
                "TASK_BUDGET_SCOPE_SPLIT_REQUIRED:{}",
                decision.selected_quantity
            )
        } else {
            format!(
                "TASK_BUDGET_CAPACITY_CONFLICT: {}",
                format_task_budget_decision_failure(&decision, &decision.writing_language)
            )
        };
        TaskBudgetPreflightError { decision, message }
    }
}

impl std::fmt::Display for TaskBudgetPreflightError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TaskBudgetPreflightError {}

// ── 物理预留计划（基线 buildMainGenerationPlan）──

fn message_role_name(role: GenerationMessageRole) -> &'static str {
    match role {
        GenerationMessageRole::System => "system",
        GenerationMessageRole::User => "user",
        GenerationMessageRole::Assistant => "assistant",
    }
}

/// 基线 `options` 构造（两个分支共享）：
/// `{ ...parameters, maxTokens, outputTokenParameter?, responseFormat? }`。
fn plan_options(
    parameters: &ResolvedGenerationParameters,
    max_tokens: u64,
    openai: bool,
    structured_output_evidence: bool,
    output: GenerationOutput,
) -> MainGenerationOptions {
    MainGenerationOptions {
        temperature: parameters.temperature,
        max_tokens: Some(max_tokens),
        output_token_parameter: if openai {
            Some("max_completion_tokens")
        } else {
            None
        },
        response_format: if output == GenerationOutput::StructuredData && structured_output_evidence
        {
            Some(json!({ "type": "json_object" }))
        } else {
            parameters.response_format.clone()
        },
        reasoning: parameters.reasoning.clone(),
        conversation_id: None,
    }
}

/// 基线 `buildMainGenerationPlan`：语义任务 → 输入上界 → 物理
/// 责任 → 根余量 → S07 预算决策（或容量合并输出上限）→ 计划。
///
/// 基线默认参数（`creativeStrategy = 'auto'` / `policy =
/// MAIN_GENERATION_POLICY`）以显式传参等价（见模块头注第 10 条）。
pub fn build_main_generation_plan(
    model: &Value,
    receipt: &ModelExecutionCapabilityEvidence,
    task: &Value,
    budget: &GenerationBudgetReceipt,
    creative_strategy: CreativeStrategy,
    policy: &MainGenerationPolicy,
) -> Result<MainGenerationPlan, String> {
    let task = assert_semantic_generation_task(task)?;
    // 基线：每消息 utf8 字节 + role 字节 + 32，基数 32。
    let input_upper_bound_tokens: i64 = 32
        + task
            .messages
            .iter()
            .map(|message| {
                message.content.len() as i64 + message_role_name(message.role).len() as i64 + 32
            })
            .sum::<i64>();
    let stage = policy_stage(task.reasoning_stage.unwrap_or(
        if task.output == GenerationOutput::VisibleText {
            TaskReasoningStage::Drafting
        } else {
            TaskReasoningStage::Planning
        },
    ));
    let liability = resolve_physical_liability(model, creative_strategy, stage)?;
    let PhysicalLiability {
        capability,
        parameters,
        usage_policy,
        reservation_ceiling,
        effective_context,
        model_output,
        openai,
        gemini,
        total_bounded,
        can_bound,
        separate_reasoning,
        gemini_reasoning,
        compatible_reasoning,
    } = liability;
    let evidence = receipt;
    let safety = main_generation_policy().safety_margin_tokens;
    let root_remaining = budget.policy.max_token_liability
        - budget.attempts.iter().map(token_liability).sum::<i64>();
    let remaining = if policy.version == "s07-capacity-v2" {
        root_remaining.min(reservation_ceiling)
    } else {
        root_remaining
    };
    if let Some(demand_value) = &task.budget_demand {
        // 基线：total-bounded 尝试需要完整预留包络空间；先报告
        // 根耗尽再推导可能误导的容量。
        if remaining <= 0 || (total_bounded && remaining < 1_048_576) {
            return Err(ROOT_BUDGET_EXHAUSTED.to_string());
        }
        let separate_output = total_bounded
            || model.get("protocol").and_then(Value::as_str) == Some("gemini")
            || gemini_reasoning.is_some()
            || compatible_reasoning.is_some()
        // 基线 `parameters.reasoning?.adapter ===
        // 'siliconflow-v4-thinking'`（G4-B3-M 裁决，B3-M2 推理
        // 机器对齐：adapter 变体已移植）。
        || matches!(
            parameters.reasoning,
            Some(ProviderReasoningDirective::SiliconFlowV4Thinking { .. })
        );
        let thinking_disabled = match parameters.reasoning.as_ref() {
            Some(ProviderReasoningDirective::DeepSeekV4Thinking { thinking, .. }) => {
                *thinking == ThinkingState::Disabled
            }
            Some(ProviderReasoningDirective::OpenAiReasoningEffort { reasoning_effort }) => {
                matches!(reasoning_effort.as_str(), "off" | "none" | "disabled")
            }
            _ => false,
        };
        let demand = parse_task_budget_demand(demand_value)?;
        let decision = plan_task_budget(TaskBudgetPlannerInput {
            stage,
            demand,
            input_estimate: TaskBudgetInputEstimate {
                upper_bound_tokens: input_upper_bound_tokens,
                estimator_version: main_generation_policy().estimator_version.to_string(),
            },
            capability,
            root: TaskBudgetRootLimits {
                remaining_token_liability: remaining,
                max_output_per_request: budget.policy.max_output_per_request,
            },
            liability: if total_bounded {
                TaskBudgetLiabilityBound::TotalBounded {
                    total_liability_upper_bound_tokens: 1_048_576,
                }
            } else if !can_bound {
                TaskBudgetLiabilityBound::Unknown {
                    reasoning_upper_bound_tokens: Some(separate_reasoning),
                }
            } else if gemini {
                TaskBudgetLiabilityBound::SeparateBounded {
                    reasoning_upper_bound_tokens: separate_reasoning,
                }
            } else {
                TaskBudgetLiabilityBound::IncludedInOutput
            },
            output_allocation: Some(if policy.version == "s07-capacity-v2" {
                TaskBudgetOutputAllocation::PhysicalCapacity
            } else if policy.version != "s07-planning-v1" && (separate_output || thinking_disabled)
            {
                TaskBudgetOutputAllocation::SemanticEstimate
            } else {
                TaskBudgetOutputAllocation::AvailableCeiling
            }),
            safety_margin_tokens: safety,
        })?;
        // 更大的语义范围不产生预留或派发：调用方重建精确的较小
        // 范围，保留每一字节必需输入。
        if decision.decision != TaskBudgetDecisionKind::Ready {
            return Err(TaskBudgetPreflightError::new(decision).to_string());
        }
        return Ok(MainGenerationPlan {
            budget_decision: Some(decision.clone()),
            options: plan_options(
                &parameters,
                decision.reserved_output_tokens as u64,
                openai,
                evidence.structured_output == Some(true),
                task.output,
            ),
            usage_policy,
            trusted_usage: true,
            reserved_tokens: decision.reservation_liability_tokens,
            input_upper_bound_tokens,
            reasoning_upper_bound_tokens: decision.reasoning_upper_bound_tokens,
            requested_output_tokens: decision.reserved_output_tokens,
        });
    }
    // 基线：total-bounded 尝试需要完整预留包络空间；在推导可能
    // 误导的非正每请求输入容量之前先报告根耗尽。
    if remaining <= 0 || (total_bounded && remaining < 1_048_576) {
        return Err(ROOT_BUDGET_EXHAUSTED.to_string());
    }
    // 与 planTaskBudget 相同的能力合并：已验证模型上限只会被用户
    // ModelSettings 限额收窄，根上限 / 根余量与输入占用。租约的
    // 冻结证据数值此处不是上界：预设无租约级能力时取用户值
    // （OpenAI、Silicon），已验证上下文可隐藏更小的用户设置。
    // 基线 `Math.min(modelOutput ?? ∞, userMaxOutputTokens ?? ∞, maxOutputPerRequest,
    //   remaining - input - separate - safety, effectiveContext - input - separate - safety)`。
    let mut requested_output_tokens = budget
        .policy
        .max_output_per_request
        .min(remaining - input_upper_bound_tokens - separate_reasoning - safety);
    if let Some(model_output) = model_output {
        requested_output_tokens = requested_output_tokens.min(model_output);
    }
    if let Some(user_max_output_tokens) = capability.user_max_output_tokens {
        requested_output_tokens = requested_output_tokens.min(user_max_output_tokens);
    }
    if let Some(effective_context) = effective_context {
        requested_output_tokens = requested_output_tokens
            .min(effective_context - input_upper_bound_tokens - separate_reasoning - safety);
    }
    if requested_output_tokens <= 0 {
        return Err(GENERATION_INPUT_CAPACITY_EXCEEDED.to_string());
    }
    // SiliconFlow 的 max_tokens 不含推理，thinking_budget 也并非
    // 每个模型的硬停止。改为预留独立选择的包络。这是预留，绝非
    // 声称模型消耗了那么多 token。
    let reasoning_upper_bound_tokens = if total_bounded {
        1_048_576 - input_upper_bound_tokens - requested_output_tokens - safety
    } else {
        separate_reasoning
    };
    let reserved_tokens =
        input_upper_bound_tokens + requested_output_tokens + reasoning_upper_bound_tokens + safety;
    if reserved_tokens > remaining {
        return Err(ROOT_BUDGET_EXHAUSTED.to_string());
    }
    Ok(MainGenerationPlan {
        budget_decision: None,
        options: plan_options(
            &parameters,
            requested_output_tokens as u64,
            openai,
            evidence.structured_output == Some(true),
            task.output,
        ),
        usage_policy,
        trusted_usage: true,
        reserved_tokens,
        input_upper_bound_tokens,
        reasoning_upper_bound_tokens,
        requested_output_tokens,
    })
}

// ── 单元测试（基线 electron/services/__tests__/main-generation-plan.test.ts
// 关键用例移植：策略构造、预算缩放、语义任务断言、规划 scope、
// readMainGenerationPolicy 门禁、buildMainGenerationPlan 容量分支）──
//
// 未移植断言（逐条登记，依赖见模块头注第 7 条 / 待裁决项 B3-M2）：
// - 基线用例 'reserves separate Qwen budgets without claiming a hard
//   bound'（'fixed Qwen native admission' describe）未移植：依赖
//   ReasoningAdapter 变体 openai-thinking-budget（Rust 侧缺失）。
// - 基线用例 'keeps SiliconFlow output and liability unchanged for
//   explicit high/max' 未移植：依赖 ReasoningAdapter 变体
//   siliconflow-v4-thinking（Rust 侧缺失）。
// - 基线用例 'does not treat an absent directive or an unsupported off
//   request as disabled thinking' 未移植：依赖手动 reasoningMapping
//   被指令机器消费（Rust 侧缺失）。
// - 基线用例 'uses the actual OpenAI directive %s' 未移植：依赖手动
//   reasoningMapping 被指令机器消费（Rust 侧缺失）。
// - 基线用例 'reserves the physical ceiling...' 的 options.reasoning
//   断言与 'preserves disabled, numeric and separate-output allocations'
//   的前两个条目未移植：同上（手动 reasoningMapping 消费）。
// - settleProviderUsage 断言未移植：依赖未移植面
//   （electron/services/generation-run-service.ts 属 B3-2）。
#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::lease::{
        create_model_execution_lease_receipt, CapabilityEvidenceSource, ModelExecutionLeaseReceipt,
    };
    use crate::repositories::generation_run_repository::RootAction;
    use crate::services::task_budget_planner::TaskBudgetReasonCode;

    /// 基线夹具 `task`。
    fn task_value() -> Value {
        json!({
            "purpose": "chapter:draft",
            "output": "visible-text",
            "messages": [{ "role": "user", "content": "雨夜，铜钥匙落在门前。" }],
        })
    }

    /// 基线 `draft(units)`。
    fn draft_task(requested_units: i64) -> Value {
        json!({
            "purpose": "chapter:draft",
            "output": "visible-text",
            "budgetDemand": {
                "kind": "draft-units",
                "writingLanguage": "zh-CN",
                "requestedUnits": requested_units,
                "segmentable": false,
            },
            "messages": [{ "role": "user", "content": "雨夜，铜钥匙落在门前。" }],
        })
    }

    /// 基线 `plot(content)`（SPEC-002 结构化产物任务）。
    fn plot_task(content: &str) -> Value {
        json!({
            "purpose": "plot-tree-snapshot",
            "output": "structured-data",
            "reasoningStage": "planning",
            "messages": [{ "role": "user", "content": content }],
        })
    }

    /// 基线 `outline`（共享推理补全 describe 的 500 单位细纲任务）。
    fn outline_task(content: &str) -> Value {
        json!({
            "purpose": "chapter-draft-short-outline",
            "output": "visible-text",
            "reasoningStage": "planning",
            "budgetDemand": {
                "kind": "draft-units",
                "writingLanguage": "zh-CN",
                "requestedUnits": 500,
                "segmentable": false,
            },
            "messages": [{ "role": "user", "content": content }],
        })
    }

    /// 基线夹具 `model(overrides)`。
    fn model(overrides: Value) -> Value {
        let mut value = json!({
            "id": "合成模型",
            "name": "合成预算模型",
            "provider": "openai",
            "protocol": "openai",
            "modelName": "gpt-4.1",
            "apiKey": "",
            "baseUrl": "https://api.openai.com/v1",
            "temperature": 0.7,
            "maxTokens": 8192,
            "purposes": ["generation"],
        });
        if let (Some(target), Some(source)) = (value.as_object_mut(), overrides.as_object()) {
            for (key, item) in source {
                target.insert(key.clone(), item.clone());
            }
        }
        value
    }

    /// 基线夹具 `ledger()`。
    fn ledger() -> GenerationBudgetReceipt {
        GenerationBudgetReceipt {
            root: RootAction {
                project_id: "项目".to_string(),
                epoch: "会话".to_string(),
                root_action_id: "根".to_string(),
                operation: "正文".to_string(),
                ui_action_nonce: "点击".to_string(),
                frozen_input_hash: "a".repeat(64),
                status: "active".to_string(),
            },
            policy: main_generation_policy().budget,
            active_elapsed_ms: 0,
            attempts: vec![],
            blocked_code: None,
        }
    }

    /// 基线 `{ ...ledger(), policy }`。
    fn ledger_with_policy(policy: &MainGenerationPolicy) -> GenerationBudgetReceipt {
        GenerationBudgetReceipt {
            policy: policy.budget.clone(),
            ..ledger()
        }
    }

    /// 基线 `createModelExecutionLeaseReceipt(profile, ...)`。
    fn lease_receipt(profile: &Value) -> ModelExecutionLeaseReceipt {
        create_model_execution_lease_receipt(
            &LeaseModelProfile::from_value(profile),
            "合成租约".to_string(),
            1,
            1000,
        )
        .expect("合法模型档案必须签发租约回执")
    }

    /// 基线 `plan(profile, input, budget)`（默认 `creativeStrategy = 'auto'`
    /// / `policy = MAIN_GENERATION_POLICY`）。
    fn plan(
        profile: &Value,
        input: &Value,
        budget: &GenerationBudgetReceipt,
    ) -> Result<MainGenerationPlan, String> {
        let receipt = lease_receipt(profile);
        build_main_generation_plan(
            profile,
            &receipt.capability_evidence,
            input,
            budget,
            CreativeStrategy::Auto,
            &main_generation_policy(),
        )
    }

    /// 基线 `plan` 的显式策略重载。
    fn plan_with_policy(
        profile: &Value,
        input: &Value,
        budget: &GenerationBudgetReceipt,
        policy: &MainGenerationPolicy,
    ) -> Result<MainGenerationPlan, String> {
        let receipt = lease_receipt(profile);
        build_main_generation_plan(
            profile,
            &receipt.capability_evidence,
            input,
            budget,
            CreativeStrategy::Auto,
            policy,
        )
    }

    /// 基线 `selection`（'new physical root liability' describe）。
    fn selection_value() -> Value {
        json!({
            "operation": "chapter-draft",
            "uiActionNonce": "new",
            "modelId": "合成模型",
            "selectedDraftIDs": [],
            "selectedFinalizedDraftIDs": [],
            "promptKeys": ["first_chapter_draft"],
            "skillStages": [],
            "output": "visible-text",
        })
    }

    /// 基线 `newMainGenerationPolicy(selection, profile)`。
    fn new_policy(profile: &Value) -> MainGenerationPolicy {
        new_main_generation_policy(&selection_value(), profile).expect("策略构造必须成功")
    }

    /// 基线夹具 `silicon()`。
    fn silicon_profile() -> Value {
        model(json!({
            "baseUrl": "https://api.siliconflow.cn/v1",
            "modelName": "deepseek-ai/DeepSeek-V4-Flash",
            "capabilities": {
                "contextWindowTokens": 65536,
                "maxOutputTokens": 8192,
                "reasoning": false,
                "structuredOutput": false,
                "usage": false,
            },
        }))
    }

    /// 基线夹具 `gemini()`。
    fn gemini_profile() -> Value {
        model(json!({
            "provider": "gemini",
            "protocol": "gemini",
            "baseUrl": "https://generativelanguage.googleapis.com",
            "modelName": "gemini-2.5-flash-lite",
            "reasoningOverride": "medium",
        }))
    }

    /// 基线夹具 `unknown()`。
    fn unknown_profile() -> Value {
        model(json!({
            "baseUrl": "https://compatible.example/v1",
            "modelName": "custom-model",
            "maxTokens": 65536,
        }))
    }

    /// 基线夹具 `numeric(contextWindowTokens?)`（手动
    /// `openai-thinking-budget` 映射）。
    fn numeric_profile(context_window_tokens: Option<u64>) -> Value {
        let high = if context_window_tokens.is_some() {
            3_000_000
        } else {
            1_500_000
        };
        let mut overrides = json!({
            // 基线 numeric() 继承 unknown()，不能误用已验证的 gpt-4.1 容量。
            "baseUrl": "https://compatible.example/v1",
            "modelName": "custom-model",
            "maxTokens": 65536,
            "reasoningOverride": "high",
            "reasoningMapping": {
                "adapter": "openai-thinking-budget",
                "supportedEfforts": ["off", "low", "medium", "high"],
                "providerValues": {
                    "off": 0,
                    "low": 8192,
                    "medium": 300000,
                    "high": high,
                },
            },
        });
        if let Some(tokens) = context_window_tokens {
            overrides["capabilities"] = json!({
                "contextWindowTokens": tokens,
                "maxOutputTokens": 65536,
                "reasoning": true,
                "structuredOutput": true,
                "usage": false,
            });
        }
        model(overrides)
    }

    /// 基线 'semantic demand with shared reasoning completion' describe
    /// 的共享档案（`deepseek-flash` + 手动 `deepseek-v4-thinking` 映射）。
    fn shared_reasoning_profile() -> Value {
        model(json!({
            "provider": "deepseek",
            "baseUrl": "https://api.deepseek.com",
            "modelName": "deepseek-flash",
            "temperature": 0,
            "maxTokens": 16384,
            "reasoningOverride": "high",
            "capabilities": {
                "contextWindowTokens": 1048576,
                "maxOutputTokens": 393216,
                "reasoning": true,
                "structuredOutput": true,
                "usage": true,
            },
            "reasoningMapping": {
                "adapter": "deepseek-v4-thinking",
                "supportedEfforts": ["off", "high"],
                "providerValues": { "off": "disabled", "high": "high" },
            },
        }))
    }

    /// 基线 'S07 semantic pre-dispatch integration' describe 的 `known()`。
    fn known_profile() -> Value {
        model(json!({
            "provider": "deepseek",
            "baseUrl": "https://api.deepseek.com",
            "modelName": "deepseek-v4-flash",
            "maxTokens": 16384,
        }))
    }

    #[test]
    fn covers_every_stage_and_strategy_reservation_for_every_profile() {
        let profiles = vec![
            model(json!({})),
            model(json!({ "modelName": "o3", "maxTokens": 65536 })),
            model(json!({
                "provider": "deepseek",
                "baseUrl": "https://api.deepseek.com",
                "modelName": "deepseek-v4-flash",
                "maxTokens": 65536,
            })),
            gemini_profile(),
            silicon_profile(),
            unknown_profile(),
            numeric_profile(None),
            numeric_profile(Some(4_000_000)),
        ];
        for profile in &profiles {
            let policy = new_policy(profile);
            let bound = policy.budget.max_token_liability / policy.budget.max_physical_requests;
            let receipt = lease_receipt(profile);
            let budget = ledger_with_policy(&policy);
            for stage in ["drafting", "planning", "review", "general"] {
                for strategy in CREATIVE_STRATEGIES {
                    for demand in [
                        None,
                        Some(json!({
                            "kind": "draft-units",
                            "writingLanguage": "zh-CN",
                            "requestedUnits": 1000,
                            "segmentable": false,
                        })),
                    ] {
                        let mut input = json!({
                            "purpose": "chapter:draft",
                            "output": "visible-text",
                            "reasoningStage": stage,
                            "messages": [
                                { "role": "user", "content": "x".repeat(43699) },
                            ],
                        });
                        if let Some(demand) = demand {
                            input["budgetDemand"] = demand;
                        }
                        let result = build_main_generation_plan(
                            profile,
                            &receipt.capability_evidence,
                            &input,
                            &budget,
                            CreativeStrategy::parse(Some(strategy)),
                            &policy,
                        )
                        .unwrap_or_else(|error| {
                            panic!(
                                "{}@{} 必须构造成功：{error}",
                                profile["modelName"], profile["baseUrl"]
                            )
                        });
                        assert!(result.reserved_tokens <= bound);
                        assert!(result.requested_output_tokens > 0);
                        assert_eq!(
                            result.reserved_tokens,
                            result.input_upper_bound_tokens
                                + result.requested_output_tokens
                                + result.reasoning_upper_bound_tokens
                                + 512
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn keeps_the_unknown_first_request_compatibility_envelope_and_unknown_settlement_semantics() {
        let profile = unknown_profile();
        let policy = new_policy(&profile);
        assert_eq!(policy.budget.max_token_liability, 32 * 2_097_152);
        for bytes in [1usize, 43_699, 2_000_000] {
            let input = json!({
                "purpose": "chapter:draft",
                "output": "visible-text",
                "messages": [{ "role": "user", "content": "x".repeat(bytes) }],
            });
            let old = plan(&profile, &input, &ledger()).unwrap();
            let next =
                plan_with_policy(&profile, &input, &ledger_with_policy(&policy), &policy).unwrap();
            assert!(next.requested_output_tokens >= old.requested_output_tokens);
            assert!(!next.usage_policy.can_bound_total_liability);
            assert_eq!(next.usage_policy.reasoning, "unknown");
            assert!(next.reserved_tokens <= 2_097_152);
        }
        let too_large = json!({
            "purpose": "chapter:draft",
            "output": "visible-text",
            "messages": [{ "role": "user", "content": "x".repeat(2_200_000) }],
        });
        let error = plan(&profile, &too_large, &ledger()).unwrap_err();
        assert_eq!(error, "GENERATION_INPUT_CAPACITY_EXCEEDED");
        let error = plan_with_policy(&profile, &too_large, &ledger_with_policy(&policy), &policy)
            .unwrap_err();
        assert_eq!(error, "GENERATION_INPUT_CAPACITY_EXCEEDED");
    }

    #[test]
    fn removes_the_old_cumulative_early_stop_without_changing_total_bounded_reservation_or_unknown_liability(
    ) {
        let profile = silicon_profile();
        let policy = new_policy(&profile);
        assert_eq!(policy.budget.max_token_liability, 32 * 1_048_576);
        let mut budget = ledger_with_policy(&policy);
        for index in 0..32i64 {
            let next = plan(&profile, &task_value(), &budget).unwrap();
            assert_eq!(next.reserved_tokens, 1_048_576);
            budget.attempts.push(PhysicalAttempt {
                attempt_id: index.to_string(),
                reservation_id: index.to_string(),
                root_action_id: "根".to_string(),
                status: "unknown".to_string(),
                reserved_tokens: next.reserved_tokens,
                requested_output_tokens: next.requested_output_tokens,
                actual_tokens: None,
            });
            if index == 1 {
                let error = plan(
                    &profile,
                    &task_value(),
                    &GenerationBudgetReceipt {
                        policy: ledger().policy.clone(),
                        ..budget.clone()
                    },
                )
                .unwrap_err();
                assert_eq!(error, "ROOT_BUDGET_EXHAUSTED");
            }
        }
        let error = plan(&profile, &task_value(), &budget).unwrap_err();
        assert_eq!(error, "ROOT_BUDGET_EXHAUSTED");
        budget.attempts[0].status = "settled".to_string();
        budget.attempts[0].actual_tokens = Some(100);
        budget.attempts[1].status = "settled".to_string();
        budget.attempts[1].actual_tokens = Some(100);
        let result = plan(&profile, &task_value(), &budget).unwrap();
        assert_eq!(result.reserved_tokens, 1_048_576);
    }

    #[test]
    fn lets_actual_physical_capacity_decide_a_43_699_byte_input_with_a_soft_target() {
        let profile = model(json!({
            "baseUrl": "https://compatible.example/v1",
            "modelName": "custom",
            "maxTokens": 65536,
            "capabilities": {
                "contextWindowTokens": 65536,
                "maxOutputTokens": 65536,
                "reasoning": false,
                "structuredOutput": true,
                "usage": false,
            },
        }));
        let policy = new_policy(&profile);
        let input = json!({
            "purpose": "chapter:draft",
            "output": "visible-text",
            "messages": [{ "role": "user", "content": "x".repeat(43699) }],
            "budgetDemand": {
                "kind": "structured-items",
                "requestedItems": 1,
                "writingLanguage": "zh-CN",
            },
        });
        let result =
            plan_with_policy(&profile, &input, &ledger_with_policy(&policy), &policy).unwrap();
        assert_eq!(result.requested_output_tokens, 21257);
        let smaller = model(json!({
            "baseUrl": "https://compatible.example/v1",
            "modelName": "custom",
            "maxTokens": 65536,
            "capabilities": {
                "contextWindowTokens": 32768,
                "maxOutputTokens": 65536,
                "reasoning": false,
                "structuredOutput": true,
                "usage": false,
            },
        }));
        let smaller_policy = new_policy(&smaller);
        let error = plan_with_policy(
            &smaller,
            &input,
            &ledger_with_policy(&smaller_policy),
            &smaller_policy,
        )
        .unwrap_err();
        assert!(!error.is_empty());
    }

    #[test]
    fn plans_unregistered_compatible_models_as_estimates_without_inventing_capability() {
        for demand in [
            None,
            Some(json!({
                "kind": "draft-units",
                "writingLanguage": "zh-CN",
                "requestedUnits": 1000,
                "segmentable": false,
            })),
        ] {
            let mut input = task_value();
            if let Some(demand) = demand {
                input["budgetDemand"] = demand;
            }
            let profile = model(json!({
                "provider": "custom",
                "baseUrl": "http://localhost:8000/v1",
                "modelName": "hand-entered",
            }));
            let result = plan(&profile, &input, &ledger()).unwrap();
            assert!(!result.usage_policy.can_bound_total_liability);
            assert_eq!(result.usage_policy.reasoning, "unknown");
            assert_eq!(result.requested_output_tokens, 8192);
            assert_eq!(
                result.reserved_tokens,
                result.input_upper_bound_tokens + result.requested_output_tokens + 512
            );
            assert!(result.options.reasoning.is_none());
            if input.get("budgetDemand").is_some() {
                let decision = result.budget_decision.as_ref().unwrap();
                assert!(decision.reasons.iter().any(|reason| matches!(
                    reason.code,
                    TaskBudgetReasonCode::ModelCapabilityUnknown
                ) && reason.selected));
            }
            let qwen = model(json!({
                "provider": "siliconflow",
                "baseUrl": "https://api.siliconflow.cn/v1",
                "modelName": "Qwen/Qwen3.8-27B",
            }));
            let qwen_result = plan(&qwen, &input, &ledger()).unwrap();
            assert!(qwen_result.options.reasoning.is_none());
            assert!(!qwen_result.usage_policy.can_bound_total_liability);
            assert_eq!(qwen_result.usage_policy.reasoning, "unknown");
            assert!(qwen_result.requested_output_tokens > 0);
        }
    }

    #[test]
    fn does_not_invent_qwen_mappings_for_unmatched_settings() {
        let qwen = || {
            model(json!({
                "provider": "siliconflow",
                "baseUrl": "https://api.siliconflow.cn/v1",
                "modelName": "Qwen/Qwen3.8-27B",
                "maxTokens": 16384,
                "reasoningOverride": "medium",
            }))
        };
        let inputs = [task_value(), draft_task(1000)];
        for overrides in [
            json!({ "baseUrl": "https://api.siliconflow.com/v1" }),
            json!({ "baseUrl": "http://api.siliconflow.cn/v1" }),
            json!({ "baseUrl": "https://api.siliconflow.cn/v2" }),
            json!({ "baseUrl": "https://api.siliconflow.cn/v1?proxy=1" }),
            json!({ "baseUrl": "https://api.siliconflow.cn.evil.test/v1" }),
            json!({ "modelName": "Qwen/Qwen3.8-27B-other" }),
            json!({ "protocol": "gemini" }),
            json!({ "reasoningOverride": "auto" }),
            json!({ "reasoningOverride": "high" }),
            json!({ "reasoningOverride": "low" }),
        ] {
            let mut profile = qwen();
            if let (Some(target), Some(source)) = (profile.as_object_mut(), overrides.as_object()) {
                for (key, item) in source {
                    target.insert(key.clone(), item.clone());
                }
            }
            for input in &inputs {
                let result = plan(&profile, input, &ledger()).unwrap();
                assert!(result.options.reasoning.is_none());
                assert!(!result.usage_policy.can_bound_total_liability);
            }
        }
    }

    #[test]
    fn keeps_the_temporary_s05_policy_finite_and_explicit() {
        assert_eq!(
            main_generation_policy().budget,
            RootBudget {
                max_physical_requests: 32,
                max_token_liability: 2_097_152,
                max_output_per_request: 32_768,
                max_active_elapsed_ms: 3_600_000,
            }
        );
    }

    #[test]
    fn scales_batch_roots_by_chapter_count_without_shrinking_below_the_single_root_policy() {
        assert_eq!(
            batch_root_budget(1.0).unwrap(),
            main_generation_policy().budget
        );
        assert_eq!(
            batch_root_budget(4.0).unwrap(),
            main_generation_policy().budget
        );
        assert_eq!(
            batch_root_budget(5.0).unwrap(),
            RootBudget {
                max_physical_requests: 40,
                max_token_liability: 2_621_440,
                max_output_per_request: 32_768,
                max_active_elapsed_ms: 4_500_000,
            }
        );
        assert_eq!(batch_root_budget(10.0).unwrap().max_physical_requests, 80);
        for invalid in [0.0, -1.0, 1.5, f64::NAN] {
            let error = batch_root_budget(invalid).unwrap_err();
            assert_eq!(error, "GENERATION_BATCH_INTENT_INVALID");
        }
    }

    #[test]
    fn uses_openai_combined_completion_cap_without_counting_reasoning_twice() {
        let profile = model(json!({ "modelName": "o3", "reasoningOverride": "high" }));
        let result = plan(&profile, &task_value(), &ledger()).unwrap();
        assert_eq!(
            result.options.output_token_parameter,
            Some("max_completion_tokens")
        );
        assert_eq!(result.reasoning_upper_bound_tokens, 0);
        assert_eq!(
            result.reserved_tokens,
            result.input_upper_bound_tokens + result.requested_output_tokens + 512
        );
        assert_eq!(result.usage_policy.reasoning, "included-in-completion");
    }

    #[test]
    fn keeps_deepseek_max_tokens_with_its_included_reasoning_cap() {
        let profile = model(json!({
            "provider": "deepseek",
            "baseUrl": "https://api.deepseek.com",
            "modelName": "deepseek-v4-flash",
        }));
        let result = plan(&profile, &task_value(), &ledger()).unwrap();
        assert!(result.options.output_token_parameter.is_none());
        assert_eq!(
            result.options.max_tokens,
            Some(result.requested_output_tokens as u64)
        );
        assert_eq!(result.reasoning_upper_bound_tokens, 0);
    }

    #[test]
    fn counts_gemini_explicit_thinking_exactly_once_in_remaining_and_total() {
        let profile = gemini_profile();
        let mut budget = ledger();
        budget.policy.max_token_liability = 10000;
        let result = plan(&profile, &task_value(), &budget).unwrap();
        assert_eq!(
            result.options.reasoning,
            Some(ProviderReasoningDirective::GeminiThinkingBudget {
                thinking_budget: 8192,
            })
        );
        assert_eq!(result.reasoning_upper_bound_tokens, 8192);
        assert_eq!(
            result.requested_output_tokens,
            10000 - result.input_upper_bound_tokens - 8192 - 512
        );
        assert_eq!(result.reserved_tokens, 10000);
        assert_eq!(result.usage_policy.reasoning, "separately-billed");
    }

    #[test]
    fn distinguishes_known_gemini_zero_from_an_unknown_thinking_estimate() {
        let off = model(json!({
            "provider": "gemini",
            "protocol": "gemini",
            "baseUrl": "https://generativelanguage.googleapis.com",
            "modelName": "gemini-2.5-flash-lite",
            "reasoningOverride": "off",
        }));
        assert_eq!(
            plan(&off, &task_value(), &ledger())
                .unwrap()
                .reasoning_upper_bound_tokens,
            0
        );
        let unknown = model(json!({
            "provider": "gemini",
            "protocol": "gemini",
            "baseUrl": "https://generativelanguage.googleapis.com",
            "modelName": "gemini-unknown",
        }));
        let result = plan(&unknown, &task_value(), &ledger()).unwrap();
        assert!(!result.usage_policy.can_bound_total_liability);
        assert_eq!(result.usage_policy.reasoning, "unknown");
    }

    #[test]
    fn reserves_the_full_silicon_v4_1m_despite_smaller_user_context_and_feature_claims() {
        let result = plan(&silicon_profile(), &task_value(), &ledger()).unwrap();
        assert_eq!(result.reserved_tokens, 1_048_576);
        assert_eq!(
            result.reasoning_upper_bound_tokens,
            1_048_576 - result.input_upper_bound_tokens - result.requested_output_tokens - 512
        );
        assert!(result.reasoning_upper_bound_tokens > 65_536);
    }

    #[test]
    fn refuses_silicon_v4_when_the_remaining_root_cannot_cover_the_full_ceiling() {
        let mut budget = ledger();
        budget.policy.max_token_liability = 1_048_575;
        let error = plan(&silicon_profile(), &task_value(), &budget).unwrap_err();
        assert_eq!(error, "ROOT_BUDGET_EXHAUSTED");
    }

    #[test]
    fn classifies_a_third_full_context_silicon_reservation_as_exhausted_root_budget_not_input_capacity(
    ) {
        let mut budget = ledger();
        for index in 0..2 {
            budget.attempts.push(PhysicalAttempt {
                attempt_id: format!("attempt-{index}"),
                reservation_id: format!("reservation-{index}"),
                root_action_id: "根".to_string(),
                status: "unknown".to_string(),
                reserved_tokens: 1_048_576,
                requested_output_tokens: 8_192,
                actual_tokens: None,
            });
        }
        let error = plan(&silicon_profile(), &task_value(), &budget).unwrap_err();
        assert_eq!(error, "ROOT_BUDGET_EXHAUSTED");
    }

    #[test]
    fn reduces_output_for_high_input_occupancy_and_refuses_no_positive_capacity() {
        let mut budget = ledger();
        budget.policy.max_token_liability = 4096;
        let input = json!({
            "purpose": "chapter:draft",
            "output": "visible-text",
            "messages": [{ "role": "user", "content": "雨".repeat(1000) }],
        });
        let result = plan(&model(json!({})), &input, &budget).unwrap();
        assert_eq!(result.input_upper_bound_tokens, 3068);
        assert_eq!(result.requested_output_tokens, 516);
        let larger = json!({
            "purpose": "chapter:draft",
            "output": "visible-text",
            "messages": [{ "role": "user", "content": "雨".repeat(1200) }],
        });
        let error = plan(&model(json!({})), &larger, &budget).unwrap_err();
        assert_eq!(error, "GENERATION_INPUT_CAPACITY_EXCEEDED");
    }

    #[test]
    fn keeps_unknown_reservations_liable_while_settled_actual_and_pre_dispatch_cancellation_differ()
    {
        let mut budget = ledger();
        budget.policy.max_token_liability = 10000;
        budget.attempts.push(PhysicalAttempt {
            attempt_id: "旧请求".to_string(),
            reservation_id: "旧预留".to_string(),
            root_action_id: "根".to_string(),
            status: "unknown".to_string(),
            reserved_tokens: 8000,
            requested_output_tokens: 7000,
            actual_tokens: Some(1),
        });
        let result = plan(&model(json!({})), &task_value(), &budget).unwrap();
        assert_eq!(result.reserved_tokens, 2000);
        budget.attempts[0].status = "settled".to_string();
        let result = plan(&model(json!({})), &task_value(), &budget).unwrap();
        assert_eq!(result.requested_output_tokens, 8192);
        budget.attempts[0].status = "cancelled-before-dispatch".to_string();
        let result = plan(&model(json!({})), &task_value(), &budget).unwrap();
        assert_eq!(result.requested_output_tokens, 8192);
    }

    #[test]
    fn rejects_renderer_physical_control_keys() {
        for key in [
            "maxTokens",
            "maxOutputTokens",
            "thinking",
            "thinking_budget",
            "plan",
            "reservedTokens",
            "responseFormat",
        ] {
            let mut input = task_value();
            input[key] = json!(1);
            let error = plan(&model(json!({})), &input, &ledger()).unwrap_err();
            assert_eq!(error, "GENERATION_SEMANTIC_TASK_INVALID");
        }
    }

    #[test]
    fn does_not_trust_endpoint_claims() {
        for base_url in [
            "https://proxy.example/v1",
            "http://api.openai.com/v1",
            "https://api.openai.com.evil.example/v1",
            "https://api.openai.com:444/v1",
            "https://user@api.openai.com/v1",
        ] {
            let profile = model(json!({ "baseUrl": base_url }));
            let result = plan(&profile, &task_value(), &ledger()).unwrap();
            assert!(!result.usage_policy.can_bound_total_liability);
        }
    }

    #[test]
    fn does_not_trust_an_arbitrary_silicon_model_or_protocol_claim() {
        let silicon_unknown = model(json!({
            "baseUrl": "https://api.siliconflow.cn/v1",
            "modelName": "unknown-model",
            "capabilities": {
                "contextWindowTokens": 65536,
                "maxOutputTokens": 8192,
                "reasoning": false,
                "structuredOutput": false,
                "usage": false,
            },
        }));
        let result = plan(&silicon_unknown, &task_value(), &ledger()).unwrap();
        assert!(!result.usage_policy.can_bound_total_liability);
        let gemini_protocol = model(json!({ "protocol": "gemini" }));
        let result = plan(&gemini_protocol, &task_value(), &ledger()).unwrap();
        assert!(!result.usage_policy.can_bound_total_liability);
    }

    #[test]
    fn official_protocol_bound_does_not_upgrade_unknown_model_evidence() {
        for base_url in ["https://api.openai.com/v1", "https://api.deepseek.com"] {
            let profile = model(json!({
                "baseUrl": base_url,
                "modelName": "unknown-future-model",
            }));
            let receipt = lease_receipt(&profile);
            assert_eq!(
                receipt.capability_evidence.source.feature_flags,
                CapabilityEvidenceSource::Unknown
            );
            assert_eq!(receipt.capability_evidence.reasoning, None);
            assert_eq!(receipt.capability_evidence.usage, None);
            let result = plan(&profile, &task_value(), &ledger()).unwrap();
            assert!(!result.usage_policy.can_bound_total_liability);
        }
    }

    #[test]
    fn caps_output_at_the_main_per_request_policy_even_when_the_profile_permits_more() {
        let profile = model(json!({ "modelName": "o3", "maxTokens": 100000 }));
        assert_eq!(
            plan(&profile, &task_value(), &ledger())
                .unwrap()
                .requested_output_tokens,
            32768
        );
    }

    #[test]
    fn admits_every_shipped_openai_catalog_model() {
        for model_name in [
            "gpt-4.1",
            "gpt-4.1-2025-04-14",
            "o3",
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-4-turbo",
            "gpt-3.5-turbo",
        ] {
            let profile = model(json!({ "modelName": model_name, "maxTokens": 2048 }));
            let result = plan(&profile, &task_value(), &ledger()).unwrap();
            assert!(result.requested_output_tokens > 0, "{model_name}");
            assert!(
                result.usage_policy.can_bound_total_liability,
                "{model_name}"
            );
        }
    }

    #[test]
    fn never_requests_more_than_the_verified_model_output_limit_when_the_user_limit_is_larger() {
        let profile = model(json!({
            "modelName": "gpt-4-turbo",
            "maxTokens": 32768,
            "reasoningOverride": "off",
            "capabilities": {
                "contextWindowTokens": 128000,
                "maxOutputTokens": 32768,
                "reasoning": false,
                "structuredOutput": true,
                "usage": true,
            },
        }));
        let result = plan(&profile, &plot_task("合成情节数据"), &ledger()).unwrap();
        assert_eq!(result.requested_output_tokens, 4096);
        assert_eq!(result.options.max_tokens, Some(4096));
        assert_eq!(
            result.reserved_tokens,
            result.input_upper_bound_tokens + 4096 + 512
        );
    }

    #[test]
    fn keeps_the_smaller_user_output_limit_below_the_verified_model_limit() {
        let profile = model(json!({
            "modelName": "gpt-4-turbo",
            "maxTokens": 2048,
            "reasoningOverride": "off",
        }));
        assert_eq!(
            plan(&profile, &plot_task("合成情节数据"), &ledger())
                .unwrap()
                .requested_output_tokens,
            2048
        );
    }

    #[test]
    fn refuses_input_that_exceeds_a_smaller_user_context_limit_instead_of_trusting_the_1m_model_window(
    ) {
        let profile = model(json!({
            "provider": "deepseek",
            "baseUrl": "https://api.deepseek.com",
            "modelName": "deepseek-v4-flash",
            "reasoningOverride": "off",
            "maxTokens": 2048,
            "capabilities": {
                "contextWindowTokens": 4096,
                "maxOutputTokens": 2048,
                "reasoning": false,
                "structuredOutput": true,
                "usage": true,
            },
        }));
        let error = plan(&profile, &plot_task(&"A".repeat(10000)), &ledger()).unwrap_err();
        assert_eq!(error, "GENERATION_INPUT_CAPACITY_EXCEEDED");
    }

    #[test]
    fn lets_the_user_context_limit_reduce_the_output_cap_by_the_input_occupancy() {
        let profile = model(json!({
            "provider": "deepseek",
            "baseUrl": "https://api.deepseek.com",
            "modelName": "deepseek-v4-flash",
            "reasoningOverride": "off",
            "maxTokens": 4000,
            "capabilities": {
                "contextWindowTokens": 4096,
                "maxOutputTokens": 4000,
                "reasoning": false,
                "structuredOutput": true,
                "usage": true,
            },
        }));
        let result = plan(&profile, &plot_task("合成情节数据"), &ledger()).unwrap();
        assert_eq!(
            result.requested_output_tokens,
            4096 - result.input_upper_bound_tokens - 512
        );
        assert!(result.requested_output_tokens < 4000);
    }

    #[test]
    fn refuses_input_that_exceeds_the_verified_model_context_even_without_a_user_context_limit() {
        let profile = model(json!({
            "modelName": "gpt-3.5-turbo",
            "maxTokens": 4096,
            "reasoningOverride": "off",
        }));
        let error = plan(&profile, &plot_task(&"A".repeat(20000)), &ledger()).unwrap_err();
        assert_eq!(error, "GENERATION_INPUT_CAPACITY_EXCEEDED");
    }

    #[test]
    fn keeps_the_silicon_v4_full_context_reservation_while_honoring_the_user_context_limit() {
        let profile = model(json!({
            "baseUrl": "https://api.siliconflow.cn/v1",
            "modelName": "deepseek-ai/DeepSeek-V4-Flash",
            "reasoningOverride": "off",
            "capabilities": {
                "contextWindowTokens": 4096,
                "maxOutputTokens": 3000,
                "reasoning": false,
                "structuredOutput": true,
                "usage": true,
            },
        }));
        let result = plan(&profile, &plot_task("合成情节数据"), &ledger()).unwrap();
        assert_eq!(result.requested_output_tokens, 3000);
        assert_eq!(result.reserved_tokens, 1_048_576);
        let error = plan(&profile, &plot_task(&"A".repeat(10000)), &ledger()).unwrap_err();
        assert_eq!(error, "GENERATION_INPUT_CAPACITY_EXCEEDED");
    }

    #[test]
    fn uses_the_verified_silicon_context_1m_not_the_documented_reservation_as_the_input_capacity() {
        let profile = model(json!({
            "baseUrl": "https://api.siliconflow.cn/v1",
            "modelName": "deepseek-ai/DeepSeek-V4-Flash",
            "reasoningOverride": "off",
            "maxTokens": 16384,
        }));
        let result = plan(&profile, &plot_task(&"A".repeat(900_000)), &ledger()).unwrap();
        assert_eq!(result.requested_output_tokens, 16384);
        let error = plan(&profile, &plot_task(&"A".repeat(1_000_000)), &ledger()).unwrap_err();
        assert_eq!(error, "GENERATION_INPUT_CAPACITY_EXCEEDED");
    }

    #[test]
    fn treats_the_root_output_ceiling_and_remaining_liability_exactly_as_before() {
        let deepseek = |max_tokens: i64| {
            model(json!({
                "provider": "deepseek",
                "baseUrl": "https://api.deepseek.com",
                "modelName": "deepseek-v4-flash",
                "reasoningOverride": "off",
                "maxTokens": max_tokens,
            }))
        };
        assert_eq!(
            plan(&deepseek(100000), &plot_task("合成情节数据"), &ledger())
                .unwrap()
                .requested_output_tokens,
            32768
        );
        let mut budget = ledger();
        budget.policy.max_token_liability = 4096;
        let baseline_input = plan(&deepseek(16384), &plot_task("合成情节数据"), &ledger())
            .unwrap()
            .input_upper_bound_tokens;
        let result = plan(&deepseek(16384), &plot_task("合成情节数据"), &budget).unwrap();
        assert_eq!(result.requested_output_tokens, 4096 - baseline_input - 512);
    }

    #[test]
    fn sizes_5000_chinese_units_above_the_former_8k_cap_with_an_explicit_decision() {
        let result = plan(&known_profile(), &draft_task(5000), &ledger()).unwrap();
        assert!(result.requested_output_tokens > 8192);
        let decision = result.budget_decision.as_ref().unwrap();
        assert_eq!(
            result.options.max_tokens,
            Some(decision.reserved_output_tokens as u64)
        );
        assert_eq!(decision.decision, TaskBudgetDecisionKind::Ready);
        assert_eq!(decision.requested_quantity, 5000);
        assert_eq!(decision.selected_quantity, 5000);
        assert_eq!(
            result.reserved_tokens,
            result.input_upper_bound_tokens + result.requested_output_tokens + 512
        );
    }

    #[test]
    fn refuses_an_oversized_indivisible_draft_instead_of_reducing_its_output_cap() {
        let profile = model(json!({
            "provider": "deepseek",
            "baseUrl": "https://api.deepseek.com",
            "modelName": "deepseek-v4-flash",
            "maxTokens": 1024,
        }));
        let error = plan(&profile, &draft_task(5000), &ledger()).unwrap_err();
        assert!(error.starts_with("TASK_BUDGET_CAPACITY_CONFLICT: "));
    }

    #[test]
    fn returns_a_pre_dispatch_split_for_200_structured_items_without_editing_author_inputs() {
        let input = json!({
            "purpose": "chapter:draft",
            "output": "structured-data",
            "budgetDemand": {
                "kind": "structured-items",
                "writingLanguage": "zh-CN",
                "requestedItems": 200,
            },
            "messages": [{ "role": "user", "content": "雨夜，铜钥匙落在门前。" }],
        });
        let error = plan(&known_profile(), &input, &ledger()).unwrap_err();
        assert!(error.starts_with("TASK_BUDGET_SCOPE_SPLIT_REQUIRED:"));
    }

    #[test]
    fn cannot_treat_an_unknown_model_plus_a_large_user_limit_as_capability_proof() {
        let profile = model(json!({
            "modelName": "unknown-future-model",
            "maxTokens": 128000,
            "capabilities": {
                "contextWindowTokens": 128000,
                "maxOutputTokens": 128000,
                "reasoning": false,
                "usage": true,
                "structuredOutput": true,
            },
        }));
        let result = plan(&profile, &draft_task(900), &ledger()).unwrap();
        assert!(!result.usage_policy.can_bound_total_liability);
        assert!(result
            .budget_decision
            .as_ref()
            .unwrap()
            .reasons
            .iter()
            .any(
                |reason| matches!(reason.code, TaskBudgetReasonCode::ModelCapabilityUnknown)
                    && reason.selected
            ));
    }

    #[test]
    fn refuses_an_indivisible_draft_when_only_the_user_output_cap_is_too_small() {
        let profile = model(json!({
            "capabilities": {
                "contextWindowTokens": 1_047_576,
                "maxOutputTokens": 1024,
                "reasoning": false,
                "usage": true,
                "structuredOutput": true,
            },
        }));
        let error = plan(&profile, &draft_task(900), &ledger()).unwrap_err();
        assert!(error.starts_with("TASK_BUDGET_CAPACITY_CONFLICT: "));
    }

    #[test]
    fn keeps_dense_required_evidence_intact_when_it_cannot_fit() {
        let profile = model(json!({
            "provider": "deepseek",
            "baseUrl": "https://api.deepseek.com",
            "modelName": "deepseek-v4-flash",
            "maxTokens": 16384,
            "capabilities": {
                "contextWindowTokens": 1024,
                "maxOutputTokens": 16384,
                "reasoning": false,
                "structuredOutput": false,
                "usage": false,
            },
        }));
        let input = json!({
            "purpose": "chapter:draft",
            "output": "visible-text",
            "budgetDemand": {
                "kind": "draft-units",
                "writingLanguage": "zh-CN",
                "requestedUnits": 900,
                "segmentable": false,
            },
            "messages": [{ "role": "user", "content": "作者明确事实\r\n".repeat(1000) }],
        });
        let error = plan(&profile, &input, &ledger()).unwrap_err();
        assert!(error.starts_with("TASK_BUDGET_CAPACITY_CONFLICT: "));
    }

    #[test]
    fn rejects_physical_fields_hidden_inside_semantic_demand() {
        let mut input = draft_task(900);
        input["budgetDemand"]["maxTokens"] = json!(9);
        let error = plan(&known_profile(), &input, &ledger()).unwrap_err();
        assert_eq!(error, "GENERATION_SEMANTIC_TASK_INVALID");
    }

    #[test]
    fn reserves_the_physical_ceiling_while_keeping_the_500_unit_semantic_demand_and_unknown_liability(
    ) {
        let profile = shared_reasoning_profile();
        let outline = outline_task(&"x".repeat(33446));
        let result = plan(&profile, &outline, &ledger()).unwrap();
        let decision = result.budget_decision.as_ref().unwrap();
        assert_eq!(decision.decision, TaskBudgetDecisionKind::Ready);
        assert_eq!(decision.requested_quantity, 500);
        assert_eq!(decision.selected_quantity, 500);
        assert_eq!(decision.requested_output_tokens, 1712);
        assert_eq!(decision.reserved_output_tokens, 16384);
        assert_eq!(decision.reservation_liability_tokens, 50410);
        assert_eq!(result.input_upper_bound_tokens, 33514);
        assert_eq!(result.requested_output_tokens, 16384);
        assert_eq!(result.reserved_tokens, 50410);
        assert_eq!(result.reasoning_upper_bound_tokens, 0);
        assert_eq!(result.usage_policy.reasoning, "unknown");
        assert!(!result.usage_policy.can_bound_total_liability);
        assert!(decision.reasons.iter().any(|reason| matches!(
            reason.code,
            TaskBudgetReasonCode::LiabilityBoundUnknown
        ) && reason.selected));
        assert!(decision.reasons.iter().any(|reason| matches!(
            reason.code,
            TaskBudgetReasonCode::OutputAllocationCeiling
        ) && reason.value_tokens == Some(16384)
            && reason.selected));
        assert_eq!(result.options.max_tokens, Some(16384));
        assert_eq!(result.options.temperature, Some(0.0));
        // 基线用例 'reserves the physical ceiling...' 的 options.reasoning 断言
        // （{ adapter: 'deepseek-v4-thinking', thinking: 'enabled',
        // reasoningEffort: 'high' }）未移植：依赖手动 reasoningMapping 被指令
        // 机器消费（Rust 侧缺失，见待裁决项 B3-M2）。
    }

    #[test]
    fn uses_the_same_allocation_for_equivalent_draft_purposes() {
        let profile = shared_reasoning_profile();
        for (purpose, stage, requested_units) in [
            ("chapter:draft", "drafting", 1000),
            ("chapter:draft-continuation", "drafting", 150),
            ("chapter:draft-no-progress-recovery", "drafting", 1),
            ("chapter:draft-condense", "drafting", 1000),
        ] {
            let input = json!({
                "purpose": purpose,
                "output": "visible-text",
                "reasoningStage": stage,
                "budgetDemand": {
                    "kind": "draft-units",
                    "writingLanguage": "zh-CN",
                    "requestedUnits": requested_units,
                    "segmentable": false,
                },
                "messages": [{ "role": "user", "content": "x".repeat(33446) }],
            });
            let result = plan(&profile, &input, &ledger()).unwrap();
            assert_eq!(result.options.max_tokens, Some(16384));
            assert_eq!(
                result
                    .budget_decision
                    .as_ref()
                    .unwrap()
                    .requested_output_tokens,
                512 + (requested_units * 12 + 4) / 5
            );
        }
    }

    #[test]
    fn allocates_shared_completion_for_structured_purposes() {
        let profile = shared_reasoning_profile();
        for purpose in ["chapter-blueprint-directory", "structured-syntax-repair"] {
            let input = json!({
                "purpose": purpose,
                "output": "structured-data",
                "reasoningStage": "planning",
                "budgetDemand": {
                    "kind": "structured-items",
                    "writingLanguage": "zh-CN",
                    "requestedItems": 1,
                },
                "messages": [{ "role": "user", "content": "x".repeat(33446) }],
            });
            let result = plan(&profile, &input, &ledger()).unwrap();
            let decision = result.budget_decision.as_ref().unwrap();
            assert_eq!(decision.requested_output_tokens, 3584);
            assert_eq!(decision.reserved_output_tokens, 16384);
        }
    }

    #[test]
    fn honors_a_lower_ceiling_and_rejects_semantic_demand_that_no_longer_fits() {
        let overrides = json!({
            "provider": "deepseek",
            "baseUrl": "https://api.deepseek.com",
            "modelName": "deepseek-flash",
            "temperature": 0,
            "maxTokens": 16384,
            "reasoningOverride": "high",
            "capabilities": {
                "contextWindowTokens": 1048576,
                "maxOutputTokens": 393216,
                "reasoning": true,
                "structuredOutput": true,
                "usage": true,
            },
            "reasoningMapping": {
                "adapter": "deepseek-v4-thinking",
                "supportedEfforts": ["off", "high"],
                "providerValues": { "off": "disabled", "high": "high" },
            },
        });
        let outline = outline_task(&"x".repeat(33446));
        for limit in [
            "user-output",
            "user-context",
            "root-output",
            "root-remaining",
        ] {
            let mut bounded_overrides = overrides.clone();
            let mut budget = ledger();
            match limit {
                "user-output" => {
                    bounded_overrides["maxTokens"] = json!(2000);
                }
                "user-context" => {
                    bounded_overrides["capabilities"]["contextWindowTokens"] = json!(36026);
                }
                "root-output" => {
                    budget.policy.max_output_per_request = 2000;
                }
                "root-remaining" => {
                    budget.policy.max_token_liability = 36026;
                }
                _ => unreachable!(),
            }
            let bounded = model(bounded_overrides);
            let result = plan(&bounded, &outline, &budget).unwrap();
            assert_eq!(result.options.max_tokens, Some(2000), "{limit}");
            let larger = json!({
                "purpose": "chapter-draft-short-outline",
                "output": "visible-text",
                "reasoningStage": "planning",
                "budgetDemand": {
                    "kind": "draft-units",
                    "writingLanguage": "zh-CN",
                    "requestedUnits": 1000,
                    "segmentable": false,
                },
                "messages": [{ "role": "user", "content": "x".repeat(33446) }],
            });
            let error = plan(&bounded, &larger, &budget).unwrap_err();
            assert!(
                error.starts_with("TASK_BUDGET_CAPACITY_CONFLICT: "),
                "{limit}: {error}"
            );
        }
    }

    #[test]
    fn honors_verified_model_output_and_context_below_user_limits() {
        let bounded = model(json!({
            "modelName": "gpt-3.5-turbo",
            "maxTokens": 100000,
        }));
        let small = json!({
            "purpose": "chapter-draft-short-outline",
            "output": "visible-text",
            "reasoningStage": "planning",
            "budgetDemand": {
                "kind": "draft-units",
                "writingLanguage": "zh-CN",
                "requestedUnits": 500,
                "segmentable": false,
            },
            "messages": [{ "role": "user", "content": "outline" }],
        });
        assert_eq!(
            plan(&bounded, &small, &ledger())
                .unwrap()
                .options
                .max_tokens,
            Some(4096)
        );
        let occupied = json!({
            "purpose": "chapter-draft-short-outline",
            "output": "visible-text",
            "reasoningStage": "planning",
            "budgetDemand": {
                "kind": "draft-units",
                "writingLanguage": "zh-CN",
                "requestedUnits": 500,
                "segmentable": false,
            },
            "messages": [{ "role": "user", "content": "x".repeat(13805) }],
        });
        assert_eq!(
            plan(&bounded, &occupied, &ledger())
                .unwrap()
                .options
                .max_tokens,
            Some(2000)
        );
        let error = plan(&bounded, &outline_task(&"x".repeat(33446)), &ledger()).unwrap_err();
        assert!(error.starts_with("TASK_BUDGET_CAPACITY_CONFLICT: "));
    }

    #[test]
    fn preserves_separate_output_allocations_for_portable_profiles() {
        // 基线用例 'preserves disabled, numeric and separate-output
        // allocations'：前两个条目（{ ...profile(), reasoningOverride: 'off' }
        // 与 { ...profile(), reasoningMapping: { adapter:
        // 'openai-thinking-budget', supportedEfforts: ['high'],
        // providerValues: { high: 4096 } } }）未移植：依赖手动
        // reasoningMapping 被指令机器消费（thinkingDisabled /
        // separateOutput 分流，Rust 侧缺失，见待裁决项 B3-M2）。
        let outline = outline_task(&"x".repeat(33446));
        let gemini_bounded = model(json!({
            "provider": "gemini",
            "protocol": "gemini",
            "baseUrl": "https://generativelanguage.googleapis.com",
            "modelName": "gemini-2.5-flash-lite",
            "reasoningOverride": "medium",
            "maxTokens": 16384,
        }));
        let gemini_unknown = model(json!({
            "provider": "gemini",
            "protocol": "gemini",
            "baseUrl": "https://generativelanguage.googleapis.com",
            "modelName": "gemini-unknown",
            "maxTokens": 16384,
        }));
        let silicon_high = model(json!({
            "baseUrl": "https://api.siliconflow.cn/v1",
            "modelName": "deepseek-ai/DeepSeek-V4-Flash",
            "reasoningOverride": "high",
            "capabilities": {
                "contextWindowTokens": 65536,
                "maxOutputTokens": 8192,
                "reasoning": false,
                "structuredOutput": false,
                "usage": false,
            },
        }));
        for bounded in [&gemini_bounded, &gemini_unknown, &silicon_high] {
            assert_eq!(
                plan(bounded, &outline, &ledger())
                    .unwrap()
                    .options
                    .max_tokens,
                Some(1712)
            );
        }
        let envelope = plan(&silicon_high, &outline, &ledger()).unwrap();
        assert_eq!(envelope.reserved_tokens, 1_048_576);
        assert_eq!(
            envelope.reasoning_upper_bound_tokens,
            1_048_576 - 33514 - 1712 - 512
        );
    }

    // ── 规划 scope / 策略构造 / 反序列化门禁 / 语义任务契约的直接单元
    // 用例（基线测试文件未直接覆盖，按任务简报「关键用例」清单补充）──

    #[test]
    fn planning_generation_scope_parses_directory_and_architecture_operations() {
        // 未知操作 → null
        assert_eq!(planning_generation_scope(&selection_value()).unwrap(), None);
        // 目录操作：full 模式
        let directory = json!({
            "operation": "chapter-blueprint-directory",
            "authorInputs": [
                {
                    "id": "directory:requested-range",
                    "text": "{\"mode\":\"full\",\"startChapter\":2,\"endChapter\":5}",
                },
                { "id": "directory:author-config", "text": "{\"totalChapters\":10}" },
            ],
        });
        let scope = planning_generation_scope(&directory).unwrap().unwrap();
        assert_eq!(
            scope,
            PlanningGenerationScope::Directory {
                requested_range: DirectoryRequestedRange {
                    mode: DirectoryMode::Full,
                    start_chapter: 2,
                    end_chapter: 5,
                },
            }
        );
        // 目录操作：append 模式
        let directory = json!({
            "operation": "chapter-blueprint-directory",
            "authorInputs": [
                {
                    "id": "directory:requested-range",
                    "text": "{\"mode\":\"append\",\"startChapter\":1,\"endChapter\":3}",
                },
                { "id": "directory:author-config", "text": "{\"totalChapters\":10}" },
            ],
        });
        let scope = planning_generation_scope(&directory).unwrap().unwrap();
        assert_eq!(
            scope,
            PlanningGenerationScope::Directory {
                requested_range: DirectoryRequestedRange {
                    mode: DirectoryMode::Append,
                    start_chapter: 1,
                    end_chapter: 3,
                },
            }
        );
        // 架构操作：generate-plot-outline（synopsis，默认意图）
        let outline = json!({
            "operation": "generate-plot-outline",
            "authorInputs": [
                { "id": "architecture:author-config", "text": "{\"totalChapters\":10}" },
            ],
        });
        match planning_generation_scope(&outline).unwrap().unwrap() {
            PlanningGenerationScope::ArchitectureOutline { intent } => {
                assert!(intent.prior_steps.is_empty());
                assert_eq!(
                    intent.synopsis_range,
                    Some(PlotOutlineRange { from: 1, to: 5 })
                );
            }
            _ => panic!("generate-plot-outline 必须解析为架构意图"),
        }
        // 架构操作：generate-core-seed（premise，默认意图）
        let seed = json!({ "operation": "generate-core-seed", "authorInputs": [] });
        match planning_generation_scope(&seed).unwrap().unwrap() {
            PlanningGenerationScope::ArchitectureOutline { intent } => {
                assert_eq!(intent.prior_steps, vec![PlanningStep::Premise]);
                assert_eq!(intent.synopsis_range, None);
            }
            _ => panic!("generate-core-seed 必须解析为架构意图"),
        }
        // 架构操作：显式 planning-intent
        let explicit = json!({
            "operation": "generate-plot-outline",
            "authorInputs": [
                {
                    "id": "architecture:planning-intent",
                    "text": "{\"version\":\"architecture-action-v1\",\"priorSteps\":[\"premise\"],\"synopsisRange\":{\"from\":2,\"to\":4}}",
                },
                { "id": "architecture:author-config", "text": "{\"totalChapters\":10}" },
            ],
        });
        match planning_generation_scope(&explicit).unwrap().unwrap() {
            PlanningGenerationScope::ArchitectureOutline { intent } => {
                assert_eq!(intent.prior_steps, vec![PlanningStep::Premise]);
                assert_eq!(
                    intent.synopsis_range,
                    Some(PlotOutlineRange { from: 2, to: 4 })
                );
            }
            _ => panic!("显式意图必须原样解析"),
        }
        // 非法意图：priorSteps 含未知值
        let invalid = json!({
            "operation": "generate-core-seed",
            "authorInputs": [
                {
                    "id": "architecture:planning-intent",
                    "text": "{\"version\":\"architecture-action-v1\",\"priorSteps\":[\"premise\",\"unknown-step\"],\"synopsisRange\":null}",
                },
            ],
        });
        assert_eq!(
            planning_generation_scope(&invalid).unwrap_err(),
            "GENERATION_PLANNING_INTENT_INVALID"
        );
        // 非法意图：目录模式缺失
        let invalid = json!({
            "operation": "chapter-blueprint-directory",
            "authorInputs": [
                {
                    "id": "directory:requested-range",
                    "text": "{\"startChapter\":1,\"endChapter\":3}",
                },
                { "id": "directory:author-config", "text": "{\"totalChapters\":10}" },
            ],
        });
        assert_eq!(
            planning_generation_scope(&invalid).unwrap_err(),
            "GENERATION_PLANNING_INTENT_INVALID"
        );
    }

    #[test]
    fn new_main_generation_policy_validates_planning_target_units() {
        // planning:target-units 唯一性
        let selection = json!({
            "operation": "generate-core-seed",
            "authorInputs": [
                { "id": "planning:target-units", "text": "600" },
                { "id": "planning:target-units", "text": "700" },
            ],
        });
        assert_eq!(
            new_main_generation_policy(&selection, &model(json!({}))).unwrap_err(),
            "GENERATION_PLANNING_TARGET_INVALID"
        );
        // 合法目标单位
        let selection = json!({
            "operation": "generate-core-seed",
            "authorInputs": [
                { "id": "planning:target-units", "text": "600" },
            ],
        });
        assert!(new_main_generation_policy(&selection, &model(json!({}))).is_ok());
        // 越界目标单位
        let selection = json!({
            "operation": "generate-core-seed",
            "authorInputs": [
                { "id": "planning:target-units", "text": "1001" },
            ],
        });
        assert_eq!(
            new_main_generation_policy(&selection, &model(json!({}))).unwrap_err(),
            "GENERATION_PLANNING_TARGET_INVALID"
        );
    }

    #[test]
    fn read_main_generation_policy_round_trips_the_frozen_default() {
        let value = json!({
            "version": "s07-task-and-liability-v1",
            "taskBudgetPolicy": s07_task_budget_policy_value(),
            "capabilityPolicyVersion": "verified-endpoint-model-budget-v1",
            "budget": main_generation_policy_budget_value(),
            "estimatorVersion": "utf8-bytes-plus-chat-framing-v1",
            "safetyMarginTokens": 512,
        });
        assert_eq!(
            read_main_generation_policy(&value).unwrap(),
            main_generation_policy()
        );
    }

    #[test]
    fn read_main_generation_policy_reconstructs_capacity_policies_with_planning_scopes() {
        let base = json!({
            "version": "s07-capacity-v2",
            "taskBudgetPolicy": s07_task_budget_policy_value(),
            "capabilityPolicyVersion": "verified-endpoint-model-budget-v1",
            "budget": main_generation_policy_budget_value(),
            "estimatorVersion": "utf8-bytes-plus-chat-framing-v1",
            "safetyMarginTokens": 512,
        });
        // 无 planning scope 的 capacity 策略
        let policy = read_main_generation_policy(&base).unwrap();
        assert_eq!(policy.version, "s07-capacity-v2");
        assert_eq!(policy.planning, None);
        // 目录 planning scope 重构
        let mut with_directory = base.clone();
        with_directory["planning"] = json!({
            "kind": "directory",
            "requestedRange": { "mode": "append", "startChapter": 3, "endChapter": 7 },
        });
        let policy = read_main_generation_policy(&with_directory).unwrap();
        assert_eq!(
            policy.planning,
            Some(PlanningGenerationScope::Directory {
                requested_range: DirectoryRequestedRange {
                    mode: DirectoryMode::Append,
                    start_chapter: 3,
                    end_chapter: 7,
                },
            })
        );
        // 架构 planning scope（现行 outline 协议 + content）重构
        let mut with_architecture = base.clone();
        with_architecture["planning"] = json!({
            "kind": "architecture",
            "intent": {
                "version": "architecture-action-v1",
                "priorSteps": ["premise"],
                "synopsisRange": null,
            },
            "outlineProtocol": "chapter-outline-v3",
            "outlineContent": { "maxAttemptsPerChapter": 2 },
        });
        let policy = read_main_generation_policy(&with_architecture).unwrap();
        assert_eq!(
            policy.planning,
            Some(PlanningGenerationScope::ArchitectureOutline {
                intent: ArchitecturePlanningIntent {
                    prior_steps: vec![PlanningStep::Premise],
                    synopsis_range: None,
                },
            })
        );
        // 旧版 legacy-range-v1 协议重构（无 outlineContent）
        let mut with_legacy = with_architecture.clone();
        if let Some(planning) = with_legacy.get_mut("planning") {
            let object = planning.as_object_mut().unwrap();
            object.insert("outlineProtocol".to_string(), json!("legacy-range-v1"));
            object.remove("outlineContent");
        }
        let policy = read_main_generation_policy(&with_legacy).unwrap();
        assert!(matches!(
            policy.planning,
            Some(PlanningGenerationScope::ArchitectureLegacyRange { .. })
        ));
        // s07-planning-v1 携带合法 scope 时通过门禁
        let mut planning_v1 = with_directory.clone();
        planning_v1["version"] = json!("s07-planning-v1");
        let policy = read_main_generation_policy(&planning_v1).unwrap();
        assert_eq!(policy.version, "s07-planning-v1");
        assert_eq!(
            policy.planning,
            Some(PlanningGenerationScope::Directory {
                requested_range: DirectoryRequestedRange {
                    mode: DirectoryMode::Append,
                    start_chapter: 3,
                    end_chapter: 7,
                },
            })
        );
    }

    #[test]
    fn read_main_generation_policy_rejects_unsupported_versions_and_malformed_budgets() {
        let base = json!({
            "version": "s07-capacity-v2",
            "taskBudgetPolicy": s07_task_budget_policy_value(),
            "capabilityPolicyVersion": "verified-endpoint-model-budget-v1",
            "budget": main_generation_policy_budget_value(),
            "estimatorVersion": "utf8-bytes-plus-chat-framing-v1",
            "safetyMarginTokens": 512,
        });
        // 不支持的版本
        let mut unsupported = base.clone();
        unsupported["version"] = json!("s07-planning-v0");
        assert_eq!(
            read_main_generation_policy(&unsupported).unwrap_err(),
            "GENERATION_POLICY_UNSUPPORTED"
        );
        // s07-planning-v1 无 planning scope
        let mut planning_v1 = base.clone();
        planning_v1["version"] = json!("s07-planning-v1");
        assert_eq!(
            read_main_generation_policy(&planning_v1).unwrap_err(),
            "GENERATION_POLICY_UNSUPPORTED"
        );
        // budget 键集缺失
        let mut missing_key = base.clone();
        if let Some(budget) = missing_key.get_mut("budget") {
            budget
                .as_object_mut()
                .unwrap()
                .remove("maxOutputPerRequest");
        }
        assert_eq!(
            read_main_generation_policy(&missing_key).unwrap_err(),
            "GENERATION_PLANNING_BUDGET_INVALID"
        );
        // budget 值非正
        let mut zero_value = base.clone();
        zero_value["budget"]["maxPhysicalRequests"] = json!(0);
        assert_eq!(
            read_main_generation_policy(&zero_value).unwrap_err(),
            "GENERATION_PLANNING_BUDGET_INVALID"
        );
        // budget 值非安全整数
        let mut fractional = base.clone();
        fractional["budget"]["maxTokenLiability"] = json!(2_097_152.5);
        assert_eq!(
            read_main_generation_policy(&fractional).unwrap_err(),
            "GENERATION_PLANNING_BUDGET_INVALID"
        );
        // 未知 planning scope 类型
        let mut unknown_scope = base.clone();
        unknown_scope["planning"] = json!({ "kind": "unknown-kind" });
        assert_eq!(
            read_main_generation_policy(&unknown_scope).unwrap_err(),
            "GENERATION_POLICY_UNSUPPORTED"
        );
        // 目录 scope 的非法 mode
        let mut invalid_mode = base.clone();
        invalid_mode["planning"] = json!({
            "kind": "directory",
            "requestedRange": { "mode": "incremental", "startChapter": 1, "endChapter": 3 },
        });
        assert_eq!(
            read_main_generation_policy(&invalid_mode).unwrap_err(),
            "GENERATION_PLANNING_INTENT_INVALID"
        );
        // 顶层非对象
        assert_eq!(
            read_main_generation_policy(&json!(null)).unwrap_err(),
            "GENERATION_PLANNING_INTENT_INVALID"
        );
    }

    #[test]
    fn assert_semantic_generation_task_validates_the_semantic_contract() {
        // 合法任务反序列化
        let task = assert_semantic_generation_task(&task_value()).unwrap();
        assert_eq!(task.purpose, "chapter:draft");
        assert_eq!(task.output, GenerationOutput::VisibleText);
        assert_eq!(task.messages.len(), 1);
        assert_eq!(task.messages[0].content, "雨夜，铜钥匙落在门前。");
        // purpose 正则 `^[a-z0-9][a-z0-9:_-]{0,127}$`（含 128 字符上界）
        assert!(assert_semantic_generation_task(&json!({
            "purpose": &"a".repeat(128),
            "output": "visible-text",
            "messages": [{ "role": "user", "content": "x" }],
        }))
        .is_ok());
        for purpose in ["Chapter:draft", "", "-leading", &"a".repeat(129)] {
            let mut invalid = task_value();
            invalid["purpose"] = json!(purpose);
            assert_eq!(
                assert_semantic_generation_task(&invalid).unwrap_err(),
                "GENERATION_SEMANTIC_TASK_INVALID"
            );
        }
        // output 二态
        let mut invalid = task_value();
        invalid["output"] = json!("visible");
        assert_eq!(
            assert_semantic_generation_task(&invalid).unwrap_err(),
            "GENERATION_SEMANTIC_TASK_INVALID"
        );
        // messages 1..1024 且角色 / 内容类型
        let mut invalid = task_value();
        invalid["messages"] = json!([]);
        assert_eq!(
            assert_semantic_generation_task(&invalid).unwrap_err(),
            "GENERATION_SEMANTIC_TASK_INVALID"
        );
        let mut invalid = task_value();
        invalid["messages"] = json!(vec![json!({ "role": "user", "content": "x" }); 1025]);
        assert_eq!(
            assert_semantic_generation_task(&invalid).unwrap_err(),
            "GENERATION_SEMANTIC_TASK_INVALID"
        );
        let mut invalid = task_value();
        invalid["messages"] = json!([{ "role": "tool", "content": "x" }]);
        assert_eq!(
            assert_semantic_generation_task(&invalid).unwrap_err(),
            "GENERATION_SEMANTIC_TASK_INVALID"
        );
        let mut invalid = task_value();
        invalid["messages"] = json!([{ "role": "user", "content": 1 }]);
        assert_eq!(
            assert_semantic_generation_task(&invalid).unwrap_err(),
            "GENERATION_SEMANTIC_TASK_INVALID"
        );
        // 键集白名单
        let mut invalid = task_value();
        invalid["maxTokens"] = json!(1);
        assert_eq!(
            assert_semantic_generation_task(&invalid).unwrap_err(),
            "GENERATION_SEMANTIC_TASK_INVALID"
        );
        // reasoningStage 四态
        let mut invalid = task_value();
        invalid["reasoningStage"] = json!("draft");
        assert_eq!(
            assert_semantic_generation_task(&invalid).unwrap_err(),
            "GENERATION_SEMANTIC_TASK_INVALID"
        );
        // budgetDemand 判别联合：kind / language / 键集 / 值域
        let mut invalid = draft_task(10);
        invalid["budgetDemand"]["kind"] = json!("draft-units-x");
        assert_eq!(
            assert_semantic_generation_task(&invalid).unwrap_err(),
            "GENERATION_SEMANTIC_TASK_INVALID"
        );
        let mut invalid = draft_task(10);
        invalid["budgetDemand"]["writingLanguage"] = json!("zh-CN-Traditional");
        assert_eq!(
            assert_semantic_generation_task(&invalid).unwrap_err(),
            "GENERATION_SEMANTIC_TASK_INVALID"
        );
        let mut invalid = draft_task(10);
        invalid["budgetDemand"]["requestedUnits"] = json!(0);
        assert_eq!(
            assert_semantic_generation_task(&invalid).unwrap_err(),
            "GENERATION_SEMANTIC_TASK_INVALID"
        );
        let mut invalid = draft_task(10);
        invalid["budgetDemand"]["segmentable"] = json!("yes");
        assert_eq!(
            assert_semantic_generation_task(&invalid).unwrap_err(),
            "GENERATION_SEMANTIC_TASK_INVALID"
        );
        let mut invalid = draft_task(10);
        invalid["budgetDemand"]["extra"] = json!(1);
        assert_eq!(
            assert_semantic_generation_task(&invalid).unwrap_err(),
            "GENERATION_SEMANTIC_TASK_INVALID"
        );
        let mut invalid = draft_task(10);
        invalid["budgetDemand"]
            .as_object_mut()
            .unwrap()
            .remove("segmentable");
        assert_eq!(
            assert_semantic_generation_task(&invalid).unwrap_err(),
            "GENERATION_SEMANTIC_TASK_INVALID"
        );
        // 合法 structured-items 需求
        let structured = json!({
            "purpose": "plot-tree-snapshot",
            "output": "structured-data",
            "budgetDemand": {
                "kind": "structured-items",
                "writingLanguage": "en-US",
                "requestedItems": 200,
            },
            "messages": [{ "role": "user", "content": "合成" }],
        });
        assert!(assert_semantic_generation_task(&structured).is_ok());
        assert!(assert_semantic_generation_task(&draft_task(10)).is_ok());
    }

    #[test]
    fn task_budget_preflight_error_carries_the_decision_with_verbatim_prefixes() {
        let decision = plan(&known_profile(), &draft_task(5000), &ledger())
            .unwrap()
            .budget_decision
            .clone()
            .unwrap();
        let error = TaskBudgetPreflightError::new(decision.clone());
        assert_eq!(
            error.to_string(),
            format!(
                "TASK_BUDGET_CAPACITY_CONFLICT: {}",
                format_task_budget_decision_failure(&decision, &decision.writing_language)
            )
        );
        // split-required 决策的前缀（无空格，逐字）
        let split = plan(
            &known_profile(),
            &json!({
                "purpose": "chapter:draft",
                "output": "structured-data",
                "budgetDemand": {
                    "kind": "structured-items",
                    "writingLanguage": "zh-CN",
                    "requestedItems": 200,
                },
                "messages": [{ "role": "user", "content": "雨夜，铜钥匙落在门前。" }],
            }),
            &ledger(),
        )
        .unwrap_err();
        assert!(split.starts_with("TASK_BUDGET_SCOPE_SPLIT_REQUIRED:"));
    }
}
