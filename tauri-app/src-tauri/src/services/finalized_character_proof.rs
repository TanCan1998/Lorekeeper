//! finalized_character_proof —— 定稿角色生成的产物来源证明（批次 B99）。
//!
//! 1:1 移植 `electron/services/finalized-character-generation-proof.ts`：
//! `proveFinalizedCharacterGeneration`（run 身份 / manifest 专属字段 / 冻结上下文
//! 序列化 / 产物行与 receipt / 模型来源的逐层校验）与
//! `normalizeFinalizedCharacterProposalSource`（稳定定稿来源归一化）。

use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::repositories::finalized_continuity_repository::is_lowercase_hex64;
use crate::repositories::generation_run_repository::{
    DurableGenerationRun, GenerationRunRepository,
};
use crate::services::finalization_generation::text_hash;
use crate::services::finalization_generation_source::{
    FinalizationGenerationContext, FinalizationGenerationStep, FinalizedCharacterContext,
};
use crate::services::finalized_character_service::{
    parse_finalized_character_state_response, ParsedStateResponse,
};

pub const CHARACTER_PROPOSAL_SOURCE_INVALID: &str = "CHARACTER_PROPOSAL_SOURCE_INVALID";
pub const CHARACTER_PROPOSAL_CONTEXT_UNPROVEN: &str = "CHARACTER_PROPOSAL_CONTEXT_UNPROVEN";
pub const CHARACTER_PROPOSAL_ARTIFACT_INVALID: &str = "CHARACTER_PROPOSAL_ARTIFACT_INVALID";
pub const CHARACTER_PROPOSAL_PROVENANCE_REQUIRED: &str = "CHARACTER_PROPOSAL_PROVENANCE_REQUIRED";

const HANDLE_KEYS: [&str; 4] = ["projectId", "epoch", "rootActionId", "runId"];
const ARTIFACT_KEYS: [&str; 3] = ["artifactId", "revision", "textHash"];
const DEDICATED_MANIFEST_KEYS: [&str; 3] = [
    "finalizationGenerationContext",
    "finalizationGenerationContextHash",
    "finalizationGenerationOriginEpoch",
];

/// `proveFinalizedCharacterGeneration` 的证明结果。
#[derive(Debug)]
pub struct FinalizedCharacterProof {
    pub context: FinalizedCharacterContext,
    /// 类型化响应（`response_value` 的解析形态）；crate 内可见。
    pub(crate) response: ParsedStateResponse,
    /// `{updates, unresolved}` 的 JSON 形态（领域提交与提案暂存复用）。
    pub response_value: Value,
    pub model: Value,
    pub artifact: Value,
    pub current_handle: Value,
    pub source_handle: Value,
    pub stable_finalization_source: bool,
}

fn has_only_keys(value: &Value, allowed: &[&str]) -> bool {
    value
        .as_object()
        .is_none_or(|map| map.keys().all(|key| allowed.contains(&key.as_str())))
}

/// 基线 `proveFinalizedCharacterGeneration`：所有生成值只从不可变产物读取，
/// 请求只携带引用。
pub fn prove_finalized_character_generation(
    conn: &Connection,
    runs: &GenerationRunRepository,
    project_id: &str,
    handle: &Value,
    reference: &Value,
) -> Result<FinalizedCharacterProof, String> {
    if !handle.is_object() || !has_only_keys(handle, &HANDLE_KEYS) {
        return Err(CHARACTER_PROPOSAL_SOURCE_INVALID.to_string());
    }
    let handle_project_id = handle
        .get("projectId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let handle_epoch = handle
        .get("epoch")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let handle_root_action_id = handle
        .get("rootActionId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let handle_run_id = handle
        .get("runId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let run: DurableGenerationRun = runs
        .get(conn, handle_run_id)
        .map_err(|error| error.to_string())?;
    let current_handle = json!({
        "projectId": &run.binding.project_id,
        "epoch": &run.binding.epoch,
        "rootActionId": &run.root_action_id,
        "runId": &run.run_id,
    });
    let manifest = &run.binding.source_manifest;
    let dedicated = DEDICATED_MANIFEST_KEYS
        .iter()
        .any(|key| manifest.contains_key(*key));
    if handle_project_id != project_id
        || handle_project_id != run.binding.project_id
        || handle_root_action_id != run.root_action_id
        || manifest.get("operation").and_then(Value::as_str) != Some("finalized-character-state")
        || (!dedicated && handle != &current_handle)
    {
        return Err(CHARACTER_PROPOSAL_SOURCE_INVALID.to_string());
    }
    let serialized = manifest
        .get("authorInputs")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find(|input| {
                input.get("id").and_then(Value::as_str) == Some("finalized-character-context")
            })
        })
        .and_then(|input| input.get("text"))
        .and_then(Value::as_str)
        .ok_or_else(|| CHARACTER_PROPOSAL_CONTEXT_UNPROVEN.to_string())?;
    if manifest
        .get("finalizedCharacterContextHash")
        .and_then(Value::as_str)
        != Some(text_hash(serialized).as_str())
    {
        return Err(CHARACTER_PROPOSAL_CONTEXT_UNPROVEN.to_string());
    }
    let context: FinalizedCharacterContext = serde_json::from_str(serialized)
        .map_err(|_| CHARACTER_PROPOSAL_CONTEXT_UNPROVEN.to_string())?;
    if dedicated {
        let finalization_value = manifest
            .get("finalizationGenerationContext")
            .cloned()
            .unwrap_or(Value::Null);
        let Some(origin_epoch) = manifest
            .get("finalizationGenerationOriginEpoch")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            return Err(CHARACTER_PROPOSAL_CONTEXT_UNPROVEN.to_string());
        };
        let finalization: FinalizationGenerationContext =
            serde_json::from_value(finalization_value.clone())
                .map_err(|_| CHARACTER_PROPOSAL_CONTEXT_UNPROVEN.to_string())?;
        let identity_matches = serde_json::to_value(&finalization.identity).ok()
            == serde_json::to_value(&context).ok();
        if manifest
            .get("finalizationGenerationContextHash")
            .and_then(Value::as_str)
            != Some(text_hash(&finalization_value.to_string()).as_str())
            || origin_epoch != context.epoch
            || !identity_matches
            || finalization.slot.step_key != FinalizationGenerationStep::CharacterCards
            || finalization.slot.source != context.source
            || context.characters.iter().any(|character| {
                character.fields.iter().any(|field| {
                    field.project_id != context.project_id
                        || field.epoch != context.epoch
                        || field.character_id != character.character_id
                })
            })
        {
            return Err(CHARACTER_PROPOSAL_CONTEXT_UNPROVEN.to_string());
        }
        if handle_epoch != run.binding.epoch && handle_epoch != origin_epoch {
            return Err(CHARACTER_PROPOSAL_SOURCE_INVALID.to_string());
        }
    }
    let selected_finalized = manifest
        .get("selectedFinalizedDraftIds")
        .cloned()
        .unwrap_or(Value::Null);
    if context.project_id != project_id
        || (!dedicated && context.epoch != handle_epoch)
        || text_hash(&context.content) != context.source.content_hash
        || selected_finalized != json!([context.source.draft_id])
        || !run.binding.source_refs.iter().any(|item| {
            item.source_id
                == format!(
                    "finalized:{}:{}",
                    context.source.draft_id, context.source.finalization_id
                )
                && item.content_hash == context.source.content_hash
        })
    {
        return Err(CHARACTER_PROPOSAL_CONTEXT_UNPROVEN.to_string());
    }
    if !reference.is_object() || !has_only_keys(reference, &ARTIFACT_KEYS) {
        return Err(CHARACTER_PROPOSAL_ARTIFACT_INVALID.to_string());
    }
    let artifact_id = reference
        .get("artifactId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let reference_revision = reference.get("revision").and_then(Value::as_i64);
    let reference_text_hash = reference.get("textHash").and_then(Value::as_str);
    let row: Option<(String, String, String)> = conn
        .query_row(
            "SELECT run_id, attempt_id, status FROM generation_artifacts WHERE artifact_id=?1",
            [artifact_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((artifact_run_id, attempt_id, status)) = row else {
        return Err(CHARACTER_PROPOSAL_ARTIFACT_INVALID.to_string());
    };
    if artifact_run_id != run.run_id || status == "discarded" {
        return Err(CHARACTER_PROPOSAL_ARTIFACT_INVALID.to_string());
    }
    let receipt = runs
        .receipt(conn, &attempt_id, None)
        .map_err(|error| error.to_string())?;
    let Some(artifact) = receipt.artifact.clone() else {
        return Err(CHARACTER_PROPOSAL_ARTIFACT_INVALID.to_string());
    };
    if reference_revision != Some(artifact.revision)
        || reference_text_hash != Some(artifact.text_hash.as_str())
        || text_hash(&artifact.text) != artifact.text_hash
        || receipt.failure_code.is_some()
        || (receipt.attempt.status != "settled" && receipt.attempt.status != "unknown")
        || receipt
            .result
            .as_ref()
            .and_then(|result| result.finish_reason.as_deref())
            != Some("stop")
    {
        return Err(CHARACTER_PROPOSAL_ARTIFACT_INVALID.to_string());
    }
    let response = parse_finalized_character_state_response(&artifact.text, &context)?;
    let model = manifest.get("modelReceipt").cloned().unwrap_or(Value::Null);
    let model_revision = model.get("modelRevision").and_then(Value::as_str);
    if model_revision.is_none_or(|value| !is_lowercase_hex64(value)) {
        return Err(CHARACTER_PROPOSAL_PROVENANCE_REQUIRED.to_string());
    }
    let source_handle = if dedicated {
        json!({
            "projectId": &run.binding.project_id,
            "epoch": &context.epoch,
            "rootActionId": &run.root_action_id,
            "runId": &run.run_id,
        })
    } else {
        current_handle.clone()
    };
    let updates: Vec<Value> = response
        .updates
        .iter()
        .map(|update| update.raw.clone())
        .collect();
    let response_value = json!({
        "updates": updates,
        "unresolved": &response.unresolved,
    });
    Ok(FinalizedCharacterProof {
        context,
        response,
        response_value,
        model,
        artifact: json!({
            "artifactId": artifact_id,
            "revision": artifact.revision,
            "textHash": artifact.text_hash,
        }),
        current_handle,
        source_handle,
        stable_finalization_source: dedicated,
    })
}

/// 基线 `normalizeFinalizedCharacterProposalSource`：稳定定稿来源归一化为
/// 产物来源句柄；不稳定的旧来源原样返回（保留写入门禁）。
pub fn normalize_finalized_character_proposal_source(
    conn: &Connection,
    runs: &GenerationRunRepository,
    project_id: &str,
    source: &Value,
) -> Result<Value, String> {
    if !source.is_object()
        || source.get("kind").and_then(Value::as_str) != Some("finalized-generation")
        || !has_only_keys(source, &["kind", "handle", "artifact"])
    {
        return Err(CHARACTER_PROPOSAL_SOURCE_INVALID.to_string());
    }
    let handle = source.get("handle").cloned().unwrap_or(Value::Null);
    let artifact = source.get("artifact").cloned().unwrap_or(Value::Null);
    let proof = prove_finalized_character_generation(conn, runs, project_id, &handle, &artifact)?;
    let normalized = if proof.stable_finalization_source {
        json!({
            "kind": "finalized-generation",
            "handle": &proof.source_handle,
            "artifact": {
                "artifactId": artifact.get("artifactId"),
                "revision": artifact.get("revision"),
                "textHash": artifact.get("textHash"),
            },
        })
    } else {
        source.clone()
    };
    Ok(json!({
        "source": normalized,
        "currentHandle": &proof.current_handle,
        "stableFinalizationSource": proof.stable_finalization_source,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn prove_rejects_invalid_handle_shape() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        let runs = GenerationRunRepository::default();

        // 非对象句柄。
        assert_eq!(
            prove_finalized_character_generation(
                &conn,
                &runs,
                "project-1",
                &json!("handle"),
                &json!({})
            )
            .unwrap_err(),
            CHARACTER_PROPOSAL_SOURCE_INVALID.to_string()
        );

        // 缺键句柄按基线流入 run 查询（句柄本身形状合法）。
        assert_eq!(
            prove_finalized_character_generation(
                &conn,
                &runs,
                "project-1",
                &json!({ "projectId": "project-1" }),
                &json!({})
            )
            .unwrap_err(),
            "GENERATION_RUN_MISSING".to_string()
        );

        // 多余键 → SOURCE_INVALID。
        assert_eq!(
            prove_finalized_character_generation(
                &conn,
                &runs,
                "project-1",
                &json!({
                    "projectId": "project-1",
                    "epoch": "lease-1",
                    "rootActionId": "root-1",
                    "runId": "run-1",
                    "extra": 1
                }),
                &json!({})
            )
            .unwrap_err(),
            CHARACTER_PROPOSAL_SOURCE_INVALID.to_string()
        );
    }
}
