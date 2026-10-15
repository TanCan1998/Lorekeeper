//! finalization_generation
//!
//! 1:1 port of electron/services/finalization-generation.ts
//! Handles state machine, slot recovery, and effect validation for finalization-generation.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::repositories::generation_run_repository::{
    DurableGenerationRun, GenerationRunRepository,
};
use crate::services::finalization_generation_source::{
    finalization_notes_baseline, finalization_slot_key, read_finalization_generation_context,
    FinalizationGenerationSlot, FinalizationGenerationStep,
};

pub const GENERATION_FINALIZATION_SLOT_INVALID: &str = "GENERATION_FINALIZATION_SLOT_INVALID";
pub const GENERATION_FINALIZATION_SLOT_CONFLICT: &str = "GENERATION_FINALIZATION_SLOT_CONFLICT";
pub const GENERATION_FINALIZATION_REQUEST_INVALID: &str = "GENERATION_FINALIZATION_REQUEST_INVALID";
pub const GENERATION_FINALIZATION_PARENT_INVALID: &str = "GENERATION_FINALIZATION_PARENT_INVALID";
pub const GENERATION_FINALIZATION_PREDECESSOR_REQUIRED: &str =
    "GENERATION_FINALIZATION_PREDECESSOR_REQUIRED";
pub const GENERATION_FINALIZATION_SOURCE_CHANGED: &str = "GENERATION_FINALIZATION_SOURCE_CHANGED";
pub const GENERATION_FINALIZATION_TASK_INVALID: &str = "GENERATION_FINALIZATION_TASK_INVALID";
pub const GENERATION_FINALIZATION_ARTIFACT_INVALID: &str =
    "GENERATION_FINALIZATION_ARTIFACT_INVALID";
pub const GENERATION_FINALIZATION_EFFECT_INVALID: &str = "GENERATION_FINALIZATION_EFFECT_INVALID";
pub const GENERATION_FINALIZATION_EFFECT_CONFLICT: &str = "GENERATION_FINALIZATION_EFFECT_CONFLICT";
pub const GENERATION_FINALIZATION_EFFECT_SEALED: &str = "GENERATION_FINALIZATION_EFFECT_SEALED";
pub const GENERATION_FINALIZATION_SUPERSEDED: &str = "GENERATION_FINALIZATION_SUPERSEDED";
pub const GENERATION_FINALIZATION_NOTES_CHANGED: &str = "GENERATION_FINALIZATION_NOTES_CHANGED";
pub const GENERATION_FINALIZATION_ATTEMPT_CONFLICT: &str =
    "GENERATION_FINALIZATION_ATTEMPT_CONFLICT";
pub const GENERATION_RUN_IDENTITY_MISMATCH: &str = "GENERATION_RUN_IDENTITY_MISMATCH";

pub fn text_hash(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedCharacterArtifact {
    pub artifact_id: String,
    pub revision: i64,
    pub text_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedEffect {
    pub version: i64,
    pub artifact: FinalizedCharacterArtifact,
    pub context_hash: String,
    pub receipt: Value,
    pub receipt_hash: String,
}

pub fn find_finalization_run(
    conn: &Connection,
    repository: &GenerationRunRepository,
    project_id: &str,
    slot: &FinalizationGenerationSlot,
) -> Result<Option<DurableGenerationRun>, String> {
    let key = finalization_slot_key(slot)?;
    let mut stmt = conn
        .prepare(
            "SELECT run_id, json_extract(json_extract(open_key,'$[0]'),'$[2]') AS ui_action_nonce              FROM generation_runs              WHERE json_extract(binding_json,'$.sourceManifest.finalizationGenerationSlotKey')=?              ORDER BY rowid",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([&key], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut previous: Option<DurableGenerationRun> = None;
    for (run_id, ui_action_nonce) in rows {
        let run = repository.get(conn, &run_id).map_err(|e| e.to_string())?;
        let context = read_finalization_generation_context(&run)?;
        if run.binding.project_id != project_id || context.slot != *slot {
            return Err(GENERATION_FINALIZATION_SLOT_CONFLICT.to_string());
        }
        if let Some(prev) = &previous {
            let prev_effect = read_finalization_effect(conn, repository, project_id, prev)?;
            let expected_nonce = format!("finalization:{}:retry:{}", key, prev.run_id);
            if prev_effect.is_some() || ui_action_nonce.as_deref() != Some(&expected_nonce) {
                return Err(GENERATION_FINALIZATION_SLOT_CONFLICT.to_string());
            }
        }
        previous = Some(run);
    }
    Ok(previous)
}

pub fn is_finalization_run_retryable(
    conn: &Connection,
    repository: &GenerationRunRepository,
    project_id: &str,
    run: &DurableGenerationRun,
) -> Result<bool, String> {
    if read_finalization_effect(conn, repository, project_id, run)?.is_some() {
        return Ok(false);
    }
    let context = read_finalization_generation_context(run)?;
    let budget = repository
        .budget(conn, &run.root_action_id)
        .map_err(|e| e.to_string())?;
    if budget
        .attempts
        .iter()
        .any(|a| a.status == "reserved" || a.status == "dispatch-marked")
    {
        return Ok(false);
    }
    if budget.root.status == "cancelled" {
        return Ok(true);
    }
    if context.slot.step_key == FinalizationGenerationStep::ChapterNotes {
        let current_baseline = finalization_notes_baseline(conn, &context.slot);
        if current_baseline != context.notes_baseline {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn require_finalization_run(
    conn: &Connection,
    repository: &GenerationRunRepository,
    project_id: &str,
    handle: &Value,
) -> Result<DurableGenerationRun, String> {
    let obj = handle
        .as_object()
        .ok_or_else(|| GENERATION_RUN_IDENTITY_MISMATCH.to_string())?;
    for k in obj.keys() {
        if !["projectId", "epoch", "rootActionId", "runId"].contains(&k.as_str()) {
            return Err(GENERATION_RUN_IDENTITY_MISMATCH.to_string());
        }
    }
    let handle_project_id = obj
        .get("projectId")
        .and_then(Value::as_str)
        .ok_or_else(|| GENERATION_RUN_IDENTITY_MISMATCH.to_string())?;
    let handle_epoch = obj
        .get("epoch")
        .and_then(Value::as_str)
        .ok_or_else(|| GENERATION_RUN_IDENTITY_MISMATCH.to_string())?;
    let handle_root_action_id = obj
        .get("rootActionId")
        .and_then(Value::as_str)
        .ok_or_else(|| GENERATION_RUN_IDENTITY_MISMATCH.to_string())?;
    let handle_run_id = obj
        .get("runId")
        .and_then(Value::as_str)
        .ok_or_else(|| GENERATION_RUN_IDENTITY_MISMATCH.to_string())?;

    let run = repository
        .get(conn, handle_run_id)
        .map_err(|_| GENERATION_RUN_IDENTITY_MISMATCH.to_string())?;
    let context = read_finalization_generation_context(&run)
        .map_err(|_| GENERATION_RUN_IDENTITY_MISMATCH.to_string())?;
    if handle_project_id != project_id
        || run.binding.project_id != project_id
        || handle_root_action_id != run.root_action_id
    {
        return Err(GENERATION_RUN_IDENTITY_MISMATCH.to_string());
    }
    if handle_epoch != run.binding.epoch && handle_epoch != context.identity.epoch {
        return Err(GENERATION_RUN_IDENTITY_MISMATCH.to_string());
    }
    Ok(run)
}

pub fn validate_finalization_artifact(
    conn: &Connection,
    repository: &GenerationRunRepository,
    run: &DurableGenerationRun,
    reference: &FinalizedCharacterArtifact,
) -> Result<(String, String), String> {
    let mut stmt = conn
        .prepare("SELECT run_id, attempt_id, status FROM generation_artifacts WHERE artifact_id=?")
        .map_err(|e| e.to_string())?;
    let row: Option<(String, String, String)> = stmt
        .query_row([&reference.artifact_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())
        .ok();

    let Some((art_run_id, attempt_id, status)) = row else {
        return Err(GENERATION_FINALIZATION_ARTIFACT_INVALID.to_string());
    };
    if art_run_id != run.run_id || status == "discarded" {
        return Err(GENERATION_FINALIZATION_ARTIFACT_INVALID.to_string());
    }
    let receipt = repository
        .receipt(conn, &attempt_id, None)
        .map_err(|_| GENERATION_FINALIZATION_ARTIFACT_INVALID.to_string())?;
    let Some(artifact) = &receipt.artifact else {
        return Err(GENERATION_FINALIZATION_ARTIFACT_INVALID.to_string());
    };
    if artifact.revision != reference.revision
        || artifact.text_hash != reference.text_hash
        || text_hash(&artifact.text) != reference.text_hash
        || artifact.text.trim().is_empty()
        || receipt.failure_code.is_some()
        || (receipt.attempt.status != "settled" && receipt.attempt.status != "unknown")
        || receipt
            .result
            .as_ref()
            .and_then(|r| r.finish_reason.as_deref())
            != Some("stop")
    {
        return Err(GENERATION_FINALIZATION_ARTIFACT_INVALID.to_string());
    }
    Ok((artifact.text.clone(), attempt_id))
}

pub fn read_finalization_effect(
    conn: &Connection,
    repository: &GenerationRunRepository,
    _project_id: &str,
    run: &DurableGenerationRun,
) -> Result<Option<Value>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT attempt_id, usage_receipt_json              FROM generation_attempts              WHERE run_id=?              ORDER BY rowid",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([&run.run_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut effects = Vec::new();
    for (attempt_id, usage_json) in rows {
        if let Some(usage_str) = usage_json {
            if let Ok(val) = serde_json::from_str::<Value>(&usage_str) {
                if let Some(effect_val) = val.get("finalizationEffect") {
                    if !effect_val.is_null() {
                        if let Ok(saved) = serde_json::from_value::<SavedEffect>(effect_val.clone())
                        {
                            effects.push((attempt_id, saved));
                        }
                    }
                }
            }
        }
    }

    if effects.len() > 1 {
        return Err(GENERATION_FINALIZATION_EFFECT_INVALID.to_string());
    }
    let Some((saved_attempt_id, saved)) = effects.into_iter().next() else {
        return Ok(None);
    };

    let (_, validated_attempt_id) =
        validate_finalization_artifact(conn, repository, run, &saved.artifact)?;
    if validated_attempt_id != saved_attempt_id {
        return Err(GENERATION_FINALIZATION_EFFECT_INVALID.to_string());
    }
    if saved.version != 1 {
        return Err(GENERATION_FINALIZATION_EFFECT_INVALID.to_string());
    }
    let context = read_finalization_generation_context(run)?;
    let expected_context_hash = text_hash(&serde_json::to_string(&context).unwrap_or_default());
    if saved.context_hash != expected_context_hash {
        return Err(GENERATION_FINALIZATION_EFFECT_INVALID.to_string());
    }
    let step_key_str = match context.slot.step_key {
        FinalizationGenerationStep::ChapterNotes => "chapter_notes",
        FinalizationGenerationStep::CharacterCards => "character_cards",
    };
    if saved.receipt.get("stepKey").and_then(Value::as_str) != Some(step_key_str)
        || saved.receipt.get("success").and_then(Value::as_bool) != Some(true)
    {
        return Err(GENERATION_FINALIZATION_EFFECT_INVALID.to_string());
    }
    let expected_receipt_hash =
        text_hash(&serde_json::to_string(&saved.receipt).unwrap_or_default());
    if saved.receipt_hash != expected_receipt_hash {
        return Err(GENERATION_FINALIZATION_EFFECT_INVALID.to_string());
    }

    Ok(Some(saved.receipt))
}

pub fn assert_finalization_mutable(
    conn: &Connection,
    repository: &GenerationRunRepository,
    project_id: &str,
    run: &DurableGenerationRun,
) -> Result<(), String> {
    if run
        .binding
        .source_manifest
        .get("finalizationGenerationContext")
        .is_none()
    {
        return Ok(());
    }
    if read_finalization_effect(conn, repository, project_id, run)?.is_some() {
        return Err(GENERATION_FINALIZATION_EFFECT_SEALED.to_string());
    }
    let context = read_finalization_generation_context(run)?;
    let latest = find_finalization_run(conn, repository, project_id, &context.slot)?;
    if latest.as_ref().map(|r| r.run_id.as_str()) != Some(run.run_id.as_str()) {
        return Err(GENERATION_FINALIZATION_SUPERSEDED.to_string());
    }
    Ok(())
}
