//! 蓝图批次成本规划 —— 1:1 移植
//! `src/services/workflows/blueprint-batch-policy.ts`。
//!
//! 仅从逻辑工作量推导任务成本包络：刻意不检查提供方名称、
//! 模型 ID 或模型特定的 token 注册表。
//!
//! 移植说明：
//! - 基线 `chapterCount: number` 以 `f64` 建模
//!   （`Number.isFinite` 检查仅在浮点建模下有意义）；
//! - 基线 `Math.ceil(a / b)` 以整数上取整实现；
//! - `getBlueprintBatchAdvice` 不在本波写面（未移植）。

/// 单语义批次的产品上限（基线 `MAX_BLUEPRINT_ITEMS_PER_BATCH`）。
pub const MAX_BLUEPRINT_ITEMS_PER_BATCH: i64 = 5;
/// 默认蓝图生成数（基线 `DEFAULT_BLUEPRINT_GENERATION_COUNT`）。
pub const DEFAULT_BLUEPRINT_GENERATION_COUNT: i64 = MAX_BLUEPRINT_ITEMS_PER_BATCH;
/// 运行时硬上限（基线 `DIRECTORY_MAX_ATTEMPTS_HARD_LIMIT`）。
const DIRECTORY_MAX_ATTEMPTS_HARD_LIMIT: i64 = 32;
/// 运行时总请求 token 上限（基线 `DIRECTORY_MAX_REQUESTED_TOKENS`）。
const DIRECTORY_MAX_REQUESTED_TOKENS: i64 = 131_072;
/// 语法修复预留调用（基线 `DIRECTORY_SYNTAX_REPAIR_RESERVE_CALLS`）。
const DIRECTORY_SYNTAX_REPAIR_RESERVE_CALLS: i64 = 1;
/// 单任务蓝图章数上限（基线 `MAX_BLUEPRINT_CHAPTERS_PER_TASK`）。
pub const MAX_BLUEPRINT_CHAPTERS_PER_TASK: i64 = 50;
/// 单次尝试请求 token 上限
/// （基线 `DIRECTORY_MAX_REQUESTED_TOKENS_PER_ATTEMPT`）。
const DIRECTORY_MAX_REQUESTED_TOKENS_PER_ATTEMPT: i64 = 4_096;
/// 运行时截止时间下界（基线 `DIRECTORY_MIN_DEADLINE_MS`）。
const DIRECTORY_MIN_DEADLINE_MS: i64 = 10 * 60_000;
/// 运行时截止时间上界（基线 `DIRECTORY_MAX_DEADLINE_MS`）。
const DIRECTORY_MAX_DEADLINE_MS: i64 = 30 * 60_000;
/// 每次尝试的截止时间（基线 `DIRECTORY_DEADLINE_PER_ATTEMPT_MS`）。
const DIRECTORY_DEADLINE_PER_ATTEMPT_MS: i64 = 2 * 60_000;

/// 蓝图生成成本计划（基线 `BlueprintGenerationCostPlan`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlueprintGenerationCostPlan {
    pub chapter_count: i64,
    pub semantic_batch_count: i64,
    /// 所有语义批次无分裂完成时的基线调用数。
    pub expected_calls: i64,
    /// 旧运行时允许值；新的主进程自有根使用 `recovery_call_bound`。
    pub max_calls: i64,
    pub recovery_call_bound: i64,
    pub max_compact_single_fallbacks: i64,
    /// 旧成本元数据，不是主进程自有根的准入限制。
    pub exceeds_hard_limit: bool,
    pub runtime_budget: BlueprintRuntimeBudget,
}

/// 运行时预算（基线 `BlueprintGenerationCostPlan.runtimeBudget`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlueprintRuntimeBudget {
    pub max_attempts: i64,
    pub max_requested_output_tokens: i64,
    pub max_requested_output_tokens_per_attempt: i64,
    pub deadline_ms: i64,
}

/// 基线 `Math.ceil(a / b)` 的整数上取整。
fn div_ceil(value: i64, divisor: i64) -> i64 {
    if value <= 0 {
        0
    } else {
        (value + divisor - 1) / divisor
    }
}

/// 基线 `planBlueprintGenerationCost`。
///
/// 仅从逻辑工作量推导任务成本包络。刻意不检查提供方名称、
/// 模型 ID 或模型特定的 token 注册表。
pub fn plan_blueprint_generation_cost(chapter_count: f64) -> BlueprintGenerationCostPlan {
    let normalized_chapter_count = if chapter_count.is_finite() {
        chapter_count.max(0.0).floor() as i64
    } else {
        0
    };
    let semantic_batch_count = div_ceil(normalized_chapter_count, MAX_BLUEPRINT_ITEMS_PER_BATCH);
    let max_compact_single_fallbacks = normalized_chapter_count;
    // 对 n 个语义项，递归分裂每个非单长的结果会产生一棵满二叉树，
    // 共 2n-1 次物理调用。跨 B 个初始批次即 2N-B。执行器另外
    // 允许每项键一次紧凑回退与一次全局语法修复。
    let uncapped_max_calls = if normalized_chapter_count == 0 {
        0
    } else {
        2 * normalized_chapter_count - semantic_batch_count
            + max_compact_single_fallbacks
            + DIRECTORY_SYNTAX_REPAIR_RESERVE_CALLS
    };
    let max_calls = DIRECTORY_MAX_ATTEMPTS_HARD_LIMIT.min(uncapped_max_calls);

    BlueprintGenerationCostPlan {
        chapter_count: normalized_chapter_count,
        semantic_batch_count,
        expected_calls: semantic_batch_count,
        max_calls,
        recovery_call_bound: uncapped_max_calls,
        max_compact_single_fallbacks,
        exceeds_hard_limit: normalized_chapter_count > MAX_BLUEPRINT_CHAPTERS_PER_TASK,
        runtime_budget: BlueprintRuntimeBudget {
            max_attempts: max_calls,
            max_requested_output_tokens: DIRECTORY_MAX_REQUESTED_TOKENS
                .min(max_calls * DIRECTORY_MAX_REQUESTED_TOKENS_PER_ATTEMPT),
            max_requested_output_tokens_per_attempt: DIRECTORY_MAX_REQUESTED_TOKENS_PER_ATTEMPT,
            deadline_ms: DIRECTORY_MAX_DEADLINE_MS
                .min(DIRECTORY_MIN_DEADLINE_MS.max(max_calls * DIRECTORY_DEADLINE_PER_ATTEMPT_MS)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_the_requested_generation_scope_to_five_test() {
        assert_eq!(DEFAULT_BLUEPRINT_GENERATION_COUNT, 5);
        assert_eq!(MAX_BLUEPRINT_ITEMS_PER_BATCH, 5);
    }

    #[test]
    fn derives_task_cost_from_logical_scope_without_consulting_a_model_registry_test() {
        let plan_four = plan_blueprint_generation_cost(4.0);
        assert_eq!(plan_four.max_calls, 12);
        assert_eq!(plan_four.max_compact_single_fallbacks, 4);
        assert_eq!(plan_four.runtime_budget.max_attempts, 12);
        assert_eq!(plan_four.runtime_budget.max_requested_output_tokens, 49_152);
        assert_eq!(plan_four.runtime_budget.deadline_ms, 1_440_000);

        assert_eq!(
            plan_blueprint_generation_cost(5.0),
            BlueprintGenerationCostPlan {
                chapter_count: 5,
                semantic_batch_count: 1,
                expected_calls: 1,
                max_calls: 15,
                recovery_call_bound: 15,
                max_compact_single_fallbacks: 5,
                exceeds_hard_limit: false,
                runtime_budget: BlueprintRuntimeBudget {
                    max_attempts: 15,
                    max_requested_output_tokens: 61_440,
                    max_requested_output_tokens_per_attempt: 4_096,
                    deadline_ms: 1_800_000,
                },
            }
        );

        let plan_eleven = plan_blueprint_generation_cost(11.0);
        assert_eq!(plan_eleven.semantic_batch_count, 3);
        assert_eq!(plan_eleven.expected_calls, 3);
        assert_eq!(plan_eleven.max_calls, 31);
        assert_eq!(plan_eleven.max_compact_single_fallbacks, 11);
        assert_eq!(plan_eleven.exceeds_hard_limit, false);
        assert_eq!(plan_eleven.runtime_budget.max_attempts, 31);
        assert_eq!(
            plan_eleven.runtime_budget.max_requested_output_tokens,
            126_976
        );
        assert_eq!(
            plan_eleven
                .runtime_budget
                .max_requested_output_tokens_per_attempt,
            4_096
        );

        let plan_fifty = plan_blueprint_generation_cost(50.0);
        assert_eq!(plan_fifty.semantic_batch_count, 10);
        assert_eq!(plan_fifty.expected_calls, 10);
        assert_eq!(plan_fifty.max_calls, 32);
        assert_eq!(plan_fifty.recovery_call_bound, 141);
        assert_eq!(plan_fifty.max_compact_single_fallbacks, 50);
        assert_eq!(plan_fifty.exceeds_hard_limit, false);
        assert_eq!(plan_fifty.runtime_budget.max_attempts, 32);
        assert_eq!(
            plan_fifty.runtime_budget.max_requested_output_tokens,
            131_072
        );
        assert_eq!(plan_fifty.runtime_budget.deadline_ms, 1_800_000);

        let plan_fifty_one = plan_blueprint_generation_cost(51.0);
        assert_eq!(plan_fifty_one.semantic_batch_count, 11);
        assert_eq!(plan_fifty_one.expected_calls, 11);
        assert_eq!(plan_fifty_one.max_calls, 32);
        assert_eq!(plan_fifty_one.max_compact_single_fallbacks, 51);
        assert_eq!(plan_fifty_one.exceeds_hard_limit, true);
        assert_eq!(
            plan_fifty_one.runtime_budget.max_requested_output_tokens,
            131_072
        );
    }

    #[test]
    fn non_finite_and_fractional_scopes_normalize_like_the_baseline_test() {
        // 基线 `Number.isFinite` 拒绝 NaN / ±Infinity → 0。
        for non_finite in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let plan = plan_blueprint_generation_cost(non_finite);
            assert_eq!(plan.chapter_count, 0);
            assert_eq!(plan.semantic_batch_count, 0);
            assert_eq!(plan.max_calls, 0);
            assert_eq!(plan.recovery_call_bound, 0);
        }
        // 负数 → Math.max(0, …) → 0；小数 → Math.floor。
        assert_eq!(plan_blueprint_generation_cost(-3.0).chapter_count, 0);
        assert_eq!(plan_blueprint_generation_cost(4.9).chapter_count, 4);
    }
}
