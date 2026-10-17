//! 审修上下文只授予当前项目会话中的有界阶段准入，不接受渲染层自造快照。
use super::*;
use crate::services::review_revision_context::review_revision_request;
use serde_json::json;

impl ReviewRevisionGeneration {
    pub(super) fn prepared_context(&self, id: &str) -> Option<PreparedReviewRevisionContext> {
        PREPARED_CONTEXTS
            .lock()
            .expect("审稿修稿上下文缓存不得被污染")
            .get(id)
            .filter(|(scope, _)| {
                scope.project_id == self.scope.project_id && scope.epoch == self.scope.epoch
            })
            .map(|(_, prepared)| prepared.clone())
    }

    pub(crate) fn admit(
        &self,
        conn: &Connection,
        selection: &Value,
    ) -> Result<Option<PreparedReviewRevisionContext>, String> {
        let operation = selection
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !REVIEW_REVISION_OPERATIONS.contains(&operation) {
            if selection
                .get("reviewRevisionContextId")
                .is_some_and(|id| !id.is_null())
            {
                return Err(mutating_error("GENERATION_REVIEW_CONTEXT_INVALID"));
            }
            return Ok(None);
        }
        let prepared = selection
            .get("reviewRevisionContextId")
            .and_then(Value::as_str)
            .and_then(|id| self.prepared_context(id))
            .ok_or_else(|| mutating_error("GENERATION_REVIEW_CONTEXT_REQUIRED"))?;
        let context = &prepared.context;
        let finalized = context.source.status == "finalized";
        let review = context.operation == ReviewRevisionOperation::ReviewChapter;
        let prompt = match context.operation {
            ReviewRevisionOperation::ReviewChapter => "consistency_check",
            ReviewRevisionOperation::RefineDraft => "refine_chapter",
            ReviewRevisionOperation::RefineFromReview => "refine_from_review",
        };
        // IPC 的对象键序不授予新事实；仅忽略序列化键序，字段集合、类型和值仍完整匹配。
        let inputs_match = selection
            .get("authorInputs")
            .and_then(Value::as_array)
            .is_some_and(|inputs| {
                inputs.len() == 1
                    && inputs[0].as_object().is_some_and(|input| input.len() == 2)
                    && inputs[0].get("id") == Some(&json!("review-revision-context"))
                    && inputs[0]
                        .get("text")
                        .and_then(Value::as_str)
                        .and_then(|text| serde_json::from_str::<Value>(text).ok())
                        == Some(serde_json::to_value(context).expect("审修上下文可序列化"))
            });
        let expected_parent = prepared.parent_root_action_id.as_ref().map(|id| json!(id));
        if operation != context.operation.as_str()
            || selection.get("chapterNumber") != Some(&json!(context.source.chapter_number))
            || selection.get("promptKeys") != Some(&json!([prompt]))
            || selection.get("skillStages")
                != Some(&json!([if review { "review" } else { "refinement" }]))
            || selection.get("output")
                != Some(&json!(if review {
                    "structured-data"
                } else {
                    "visible-text"
                }))
            || selection.get("selectedDraftIds")
                != Some(&if finalized {
                    json!([])
                } else {
                    json!([context.source.id])
                })
            || selection.get("selectedFinalizedDraftIds")
                != Some(&if finalized {
                    json!([context.source.id])
                } else {
                    json!([])
                })
            || selection.get("selectedBlueprintChapterNumbers")
                != Some(&json!(context
                    .blueprints
                    .iter()
                    .map(|blueprint| blueprint.chapter_number)
                    .collect::<Vec<_>>()))
            || !inputs_match
            || selection.get("parentRootActionId") != expected_parent.as_ref()
            || [
                "outputOverrides",
                "batchId",
                "batchIntent",
                "continueDirectoryOperationId",
                "preparationId",
                "agentWorkflowRegistrationId",
                "materialDecision",
            ]
            .iter()
            .any(|key| selection.get(*key).is_some())
            || (context.confirmation.is_none()
                && prepared.model_id.as_ref().is_some_and(|id| {
                    selection.get("modelId").and_then(Value::as_str) != Some(id.as_str())
                }))
        {
            return Err(mutating_error("GENERATION_REVIEW_CONTEXT_MISMATCH"));
        }
        let current = capture_review_revision_context(
            conn,
            &review_revision_request(context),
            &self.scope.project_id,
            context.recheck.as_ref().map(|recheck| recheck.version),
        )
        .map_err(|_| mutating_error("GENERATION_REVIEW_CONTEXT_CHANGED"))?;
        let parent = self.parent(conn, context)?;
        if &current != context
            || parent.parent_root_action_id != prepared.parent_root_action_id
            || parent.model_id != prepared.model_id
        {
            return Err(mutating_error("GENERATION_REVIEW_CONTEXT_CHANGED"));
        }
        Ok(Some(prepared))
    }

    /// 正式效果封存其候选与收据；重放已保存回执不经过此可变操作门禁。
    pub(crate) fn assert_mutable(&self, conn: &Connection, run_id: &str) -> Result<(), String> {
        for attempt in self.attempts(conn, Some(run_id))? {
            let usage: Value = serde_json::from_str(&attempt.usage_receipt_json)
                .map_err(|_| mutating_error("GENERATION_REVIEW_RECEIPT_INVALID"))?;
            if usage
                .get("reviewRevisionEffect")
                .is_some_and(|effect| !effect.is_null())
            {
                return Err(mutating_error("GENERATION_REVIEW_ALREADY_SAVED"));
            }
        }
        Ok(())
    }
}
