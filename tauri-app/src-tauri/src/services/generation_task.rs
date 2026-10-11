//! `GenerationTask` 语义契约（1:1 移植 `src/services/generation/generation-harness.ts:17-33`）。
//!
//! 「语义生成契约」——物理 provider 参数刻意缺席：TS 中 `maxTokens` /
//! `maxOutputTokens` / `responseFormat` / `thinking` / `plan` 均为
//! `?: never` 编译期守卫，Rust 侧以不声明对应字段等价。
//!
//! # 刻意偏离（对齐 `generation_run_repository.rs` 头注的「全保真」约定）
//! - `budgetDemand`（`TaskBudgetDemand` 判别联合）与 `promptBudget`
//!   （`PromptBudgetPolicy`）对本模块不可解释，以 `serde_json::Value`
//!   原样存取，不做结构校验；
//! - `messages: readonly GenerationMessage[]` → `Vec<GenerationMessage>`。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 产物形态（`GenerationOutput = 'visible-text' | 'structured-data'`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GenerationOutput {
    #[serde(rename = "visible-text")]
    VisibleText,
    #[serde(rename = "structured-data")]
    StructuredData,
}

/// 消息角色（`GenerationMessage.role`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GenerationMessageRole {
    #[serde(rename = "system")]
    System,
    #[serde(rename = "user")]
    User,
    #[serde(rename = "assistant")]
    Assistant,
}

/// 单条对话消息（`GenerationMessage`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationMessage {
    pub role: GenerationMessageRole,
    pub content: String,
}

/// 推理阶段（`GenerationReasoningStage`，源自 `src/shared/reasoning-types.ts`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GenerationReasoningStage {
    #[serde(rename = "drafting")]
    Drafting,
    #[serde(rename = "planning")]
    Planning,
    #[serde(rename = "review")]
    Review,
    #[serde(rename = "general")]
    General,
}

/// 语义生成契约。物理 provider 参数 deliberately absent（见文件头注）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationTask {
    pub purpose: String,
    /// 语义用量需求（`TaskBudgetDemand`）——全保真 `Value`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_demand: Option<Value>,
    /// 显式产物语义；省略时仅由 output 形状派生。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_stage: Option<GenerationReasoningStage>,
    pub output: GenerationOutput,
    pub messages: Vec<GenerationMessage>,
    /// 受保护结构化 prompt 的字节精确安全策略（`PromptBudgetPolicy`）——
    /// 全保真 `Value`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_budget: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_task_round_trips_camel_case_contract() {
        let task = GenerationTask {
            purpose: "editor-ai-refine".to_string(),
            budget_demand: None,
            reasoning_stage: Some(GenerationReasoningStage::Review),
            output: GenerationOutput::VisibleText,
            messages: vec![
                GenerationMessage {
                    role: GenerationMessageRole::System,
                    content: "系统角色".to_string(),
                },
                GenerationMessage {
                    role: GenerationMessageRole::User,
                    content: "用户指令".to_string(),
                },
            ],
            prompt_budget: None,
        };
        let json = serde_json::to_value(&task).unwrap();
        // camelCase 键名逐字对齐 TS 契约
        assert_eq!(json["purpose"], "editor-ai-refine");
        assert_eq!(json["reasoningStage"], "review");
        assert_eq!(json["output"], "visible-text");
        assert_eq!(json["messages"][0]["role"], "system");
        assert_eq!(json["messages"][1]["role"], "user");
        // 可选字段省略（TS `undefined` 键消失语义）
        assert!(json.get("budgetDemand").is_none());
        assert!(json.get("promptBudget").is_none());
        // 往返等价
        let decoded: GenerationTask = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, task);
    }

    #[test]
    fn generation_task_preserves_opaque_duck_typed_fields() {
        let json = serde_json::json!({
            "purpose": "p",
            "budgetDemand": { "kind": "draft-units", "requestedUnits": 3 },
            "output": "structured-data",
            "messages": [],
            "promptBudget": { "limitUtf8Bytes": 8192, "sections": [] }
        });
        let task: GenerationTask = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(task.budget_demand, Some(json["budgetDemand"].clone()));
        assert_eq!(task.prompt_budget, Some(json["promptBudget"].clone()));
        assert_eq!(task.output, GenerationOutput::StructuredData);
        assert_eq!(task.reasoning_stage, None);
    }
}
