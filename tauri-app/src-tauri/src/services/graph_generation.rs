//! Graph generation run owner: task/result derivation, stored-effect
//! verification, and confirmations against the SQLite ledger.
//!
//! Port of upstream `electron/services/graph-generation.ts` (plus the
//! `src/shared/graph-generation.ts` types, folded into this module to
//! avoid a name clash with the repository-side
//! [`crate::services::graph_generation_source`] module).
//!
//! Semantics preserved from upstream:
//! - every run re-reads its binding manifest and re-proves the stored
//!   context / task / effect hashes before any write;
//! - `confirm` is a single immediate transaction with two-phase
//!   conflict detection (`GENERATION_GRAPH_EFFECT_CONFLICT` for an
//!   already-stored index, `GENERATION_GRAPH_TARGET_CHANGED` for a
//!   moved plot-tree baseline);
//! - `proven_graph_own_event_ids` re-proves the finalized-row identity
//!   behind every event effect before exposing it to callers.

use crate::plot_tree::PlotTreeSnapshot;
use crate::repositories::generation_run_repository::{GenerationRunRepository, VisibleArtifact};
use crate::repositories::narrative_thread_repository::{
    confirm_event, create_plan, NarrativeThreadEvent, NarrativeThreadEventInput,
    NarrativeThreadPlanInput, NarrativeThreadPlanRecord,
};
use crate::repositories::plot_tree_repository::save;
use crate::services::graph_generation_source::{
    read_graph_generation_context, GraphGenerationContext, GraphGenerationKind,
};
use crate::services::narrative_thread_generation_pure::{
    parse_narrative_thread_event_candidates, parse_narrative_thread_plan_candidates,
    NarrativeThreadEventCandidate,
};
use crate::services::plot_tree_generation_pure::{derive_plot_tree_snapshot, PlotTreeDerivation};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// `GENERATION_RUN_IDENTITY_MISMATCH` — handle/run identity mismatch.
pub const GENERATION_RUN_IDENTITY_MISMATCH: &str = "GENERATION_RUN_IDENTITY_MISMATCH";
/// `GENERATION_GRAPH_TASK_INVALID` — stored task hash / deep-equal check failed.
pub const GENERATION_GRAPH_TASK_INVALID: &str = "GENERATION_GRAPH_TASK_INVALID";
/// `GENERATION_GRAPH_ARTIFACT_INVALID` — artifact lookup / receipt proof failed.
pub const GENERATION_GRAPH_ARTIFACT_INVALID: &str = "GENERATION_GRAPH_ARTIFACT_INVALID";
/// `GENERATION_GRAPH_EFFECT_INVALID` — effect payload proof failed.
pub const GENERATION_GRAPH_EFFECT_INVALID: &str = "GENERATION_GRAPH_EFFECT_INVALID";
/// `GENERATION_GRAPH_EFFECT_SEALED` — run already has confirmed effects.
pub const GENERATION_GRAPH_EFFECT_SEALED: &str = "GENERATION_GRAPH_EFFECT_SEALED";
/// `GENERATION_GRAPH_SELECTION_INVALID` — bad effect index selection.
pub const GENERATION_GRAPH_SELECTION_INVALID: &str = "GENERATION_GRAPH_SELECTION_INVALID";
/// `GENERATION_GRAPH_EFFECT_CONFLICT` — index already stored with a different artifact.
pub const GENERATION_GRAPH_EFFECT_CONFLICT: &str = "GENERATION_GRAPH_EFFECT_CONFLICT";
/// `GENERATION_GRAPH_TARGET_CHANGED` — plot-tree baseline moved under the run.
pub const GENERATION_GRAPH_TARGET_CHANGED: &str = "GENERATION_GRAPH_TARGET_CHANGED";
/// `GENERATION_GRAPH_OWN_EFFECT_CHANGED` — proven own event row changed.
pub const GENERATION_GRAPH_OWN_EFFECT_CHANGED: &str = "GENERATION_GRAPH_OWN_EFFECT_CHANGED";
/// `GENERATION_GRAPH_CONTEXT_AMBIGUOUS` — more than one run shares the context key.
pub const GENERATION_GRAPH_CONTEXT_AMBIGUOUS: &str = "GENERATION_GRAPH_CONTEXT_AMBIGUOUS";
/// `GENERATION_GRAPH_CONTEXT_INVALID` — stored context deep-equal check failed.
pub const GENERATION_GRAPH_CONTEXT_INVALID: &str = "GENERATION_GRAPH_CONTEXT_INVALID";

/// Per-generation task (upstream `GraphGenerationTask`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphGenerationTask {
    pub purpose: String,
    pub reasoning_stage: String,
    pub output: String,
    pub messages: Vec<GraphGenerationMessage>,
}

/// Task message (upstream `GraphGenerationMessage`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphGenerationMessage {
    pub role: String,
    pub content: String,
}

/// Generation result (upstream `GraphGenerationResult`).
///
/// `derivation` on the plot arm is the upstream
/// `PlotTreeDerivation['kind']` string (`'model' | 'normalized' |
/// 'deterministic'`), matching `deriveGraphGenerationResult`.
#[derive(Debug, Clone, PartialEq)]
pub enum GraphGenerationResult {
    Plot {
        snapshot: PlotTreeSnapshot,
        derivation: String,
    },
    Plan {
        candidates: Vec<NarrativeThreadPlanInput>,
    },
    Event {
        candidates: Vec<NarrativeThreadEventCandidate>,
    },
}

impl GraphGenerationResult {
    /// Upstream `result.kind`.
    pub fn kind(&self) -> GraphGenerationKind {
        match self {
            Self::Plot { .. } => GraphGenerationKind::Plot,
            Self::Plan { .. } => GraphGenerationKind::Plan,
            Self::Event { .. } => GraphGenerationKind::Event,
        }
    }
}

impl Serialize for GraphGenerationResult {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        match self {
            Self::Plot {
                snapshot,
                derivation,
            } => {
                map.serialize_entry("kind", "plot")?;
                map.serialize_entry("snapshot", snapshot)?;
                map.serialize_entry("derivation", derivation)?;
            }
            Self::Plan { candidates } => {
                map.serialize_entry("kind", "plan")?;
                map.serialize_entry("candidates", candidates)?;
            }
            Self::Event { candidates } => {
                map.serialize_entry("kind", "event")?;
                map.serialize_entry("candidates", candidates)?;
            }
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for GraphGenerationResult {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let Some(kind) = value.get("kind").and_then(Value::as_str) else {
            return Err(serde::de::Error::custom(GENERATION_GRAPH_EFFECT_INVALID));
        };
        match kind {
            "plot" => {
                let snapshot: PlotTreeSnapshot = value
                    .get("snapshot")
                    .cloned()
                    .and_then(|snapshot| serde_json::from_value(snapshot).ok())
                    .ok_or_else(|| serde::de::Error::custom(GENERATION_GRAPH_EFFECT_INVALID))?;
                let derivation = value
                    .get("derivation")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| serde::de::Error::custom(GENERATION_GRAPH_EFFECT_INVALID))?;
                Ok(Self::Plot {
                    snapshot,
                    derivation,
                })
            }
            "plan" => {
                let candidates: Vec<NarrativeThreadPlanInput> = value
                    .get("candidates")
                    .cloned()
                    .and_then(|candidates| serde_json::from_value(candidates).ok())
                    .ok_or_else(|| serde::de::Error::custom(GENERATION_GRAPH_EFFECT_INVALID))?;
                Ok(Self::Plan { candidates })
            }
            "event" => {
                let candidates: Vec<NarrativeThreadEventCandidate> = value
                    .get("candidates")
                    .cloned()
                    .and_then(|candidates| serde_json::from_value(candidates).ok())
                    .ok_or_else(|| serde::de::Error::custom(GENERATION_GRAPH_EFFECT_INVALID))?;
                Ok(Self::Event { candidates })
            }
            _ => Err(serde::de::Error::custom(GENERATION_GRAPH_EFFECT_INVALID)),
        }
    }
}

/// Stored artifact reference (upstream `GraphGenerationArtifact`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphGenerationArtifact {
    pub artifact_id: String,
    pub revision: i64,
    pub text_hash: String,
}

/// Generation effect receipt (upstream `GraphGenerationEffect`).
#[derive(Debug, Clone, PartialEq)]
pub enum GraphGenerationEffect {
    Plot {
        success: bool,
        index: i64,
        snapshot: PlotTreeSnapshot,
        derivation: String,
    },
    Plan {
        success: bool,
        index: i64,
        plan: NarrativeThreadPlanRecord,
    },
    Event {
        success: bool,
        index: i64,
        event: NarrativeThreadEvent,
    },
}

impl GraphGenerationEffect {
    /// Upstream `effect.success`.
    pub fn success(&self) -> bool {
        match self {
            Self::Plot { success, .. }
            | Self::Plan { success, .. }
            | Self::Event { success, .. } => *success,
        }
    }

    /// Upstream `effect.index`.
    pub fn index(&self) -> i64 {
        match self {
            Self::Plot { index, .. } | Self::Plan { index, .. } | Self::Event { index, .. } => {
                *index
            }
        }
    }

    /// Upstream `effect.kind`.
    pub fn kind(&self) -> GraphGenerationKind {
        match self {
            Self::Plot { .. } => GraphGenerationKind::Plot,
            Self::Plan { .. } => GraphGenerationKind::Plan,
            Self::Event { .. } => GraphGenerationKind::Event,
        }
    }
}

impl Serialize for GraphGenerationEffect {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        // Key order mirrors the upstream object literal:
        // `{ success, index, ...kind, ...payload }`.
        let mut map = serializer.serialize_map(None)?;
        match self {
            Self::Plot {
                success,
                index,
                snapshot,
                derivation,
            } => {
                map.serialize_entry("success", success)?;
                map.serialize_entry("index", index)?;
                map.serialize_entry("kind", "plot")?;
                map.serialize_entry("snapshot", snapshot)?;
                map.serialize_entry("derivation", derivation)?;
            }
            Self::Plan {
                success,
                index,
                plan,
            } => {
                map.serialize_entry("success", success)?;
                map.serialize_entry("index", index)?;
                map.serialize_entry("kind", "plan")?;
                map.serialize_entry("plan", plan)?;
            }
            Self::Event {
                success,
                index,
                event,
            } => {
                map.serialize_entry("success", success)?;
                map.serialize_entry("index", index)?;
                map.serialize_entry("kind", "event")?;
                map.serialize_entry("event", event)?;
            }
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for GraphGenerationEffect {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let Some(kind) = value.get("kind").and_then(Value::as_str) else {
            return Err(serde::de::Error::custom(GENERATION_GRAPH_EFFECT_INVALID));
        };
        let success = value
            .get("success")
            .and_then(Value::as_bool)
            .ok_or_else(|| serde::de::Error::custom(GENERATION_GRAPH_EFFECT_INVALID))?;
        let index = value
            .get("index")
            .and_then(Value::as_i64)
            .ok_or_else(|| serde::de::Error::custom(GENERATION_GRAPH_EFFECT_INVALID))?;
        match kind {
            "plot" => {
                let snapshot: PlotTreeSnapshot = value
                    .get("snapshot")
                    .cloned()
                    .and_then(|snapshot| serde_json::from_value(snapshot).ok())
                    .ok_or_else(|| serde::de::Error::custom(GENERATION_GRAPH_EFFECT_INVALID))?;
                let derivation = value
                    .get("derivation")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| serde::de::Error::custom(GENERATION_GRAPH_EFFECT_INVALID))?;
                Ok(Self::Plot {
                    success,
                    index,
                    snapshot,
                    derivation,
                })
            }
            "plan" => {
                let plan: NarrativeThreadPlanRecord = value
                    .get("plan")
                    .cloned()
                    .and_then(|plan| serde_json::from_value(plan).ok())
                    .ok_or_else(|| serde::de::Error::custom(GENERATION_GRAPH_EFFECT_INVALID))?;
                Ok(Self::Plan {
                    success,
                    index,
                    plan,
                })
            }
            "event" => {
                let event: NarrativeThreadEvent = value
                    .get("event")
                    .cloned()
                    .and_then(|event| serde_json::from_value(event).ok())
                    .ok_or_else(|| serde::de::Error::custom(GENERATION_GRAPH_EFFECT_INVALID))?;
                Ok(Self::Event {
                    success,
                    index,
                    event,
                })
            }
            _ => Err(serde::de::Error::custom(GENERATION_GRAPH_EFFECT_INVALID)),
        }
    }
}

/// Durable stored-effect row (upstream `StoredGraphEffect`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredGraphEffect {
    version: i64,
    artifact: GraphGenerationArtifact,
    index: i64,
    context_hash: String,
    receipt: GraphGenerationEffect,
    receipt_hash: String,
}

/// Outcome of the artifact / receipt proof (upstream `proof` return).
struct ProofOutcome {
    attempt_id: String,
    artifact: VisibleArtifact,
    context: GraphGenerationContext,
    result: GraphGenerationResult,
}

/// Derive the structured result from artifact text (upstream
/// `deriveGraphGenerationResult`).
///
/// `legacy_stored_effect` is threaded through to the candidate parsers
/// so stored-effect recovery degrades instead of failing.
pub fn derive_graph_generation_result(
    context: &GraphGenerationContext,
    text: &str,
    legacy_stored_effect: bool,
) -> Result<GraphGenerationResult, String> {
    match context {
        GraphGenerationContext::Plot(plot) => {
            let derivation: PlotTreeDerivation =
                derive_plot_tree_snapshot(text, &plot.sources, &plot.base.created_at)?;
            Ok(GraphGenerationResult::Plot {
                snapshot: derivation.snapshot,
                derivation: derivation.kind,
            })
        }
        GraphGenerationContext::Plan(plan) => Ok(GraphGenerationResult::Plan {
            candidates: parse_narrative_thread_plan_candidates(
                text,
                plan.total_chapters,
                legacy_stored_effect,
            )?,
        }),
        GraphGenerationContext::Event(event) => Ok(GraphGenerationResult::Event {
            candidates: parse_narrative_thread_event_candidates(
                text,
                &event.content,
                legacy_stored_effect,
            )?,
        }),
    }
}

/// Graph generation run owner (upstream `GraphGeneration` class).
pub struct GraphGeneration<'a> {
    conn: &'a Connection,
    runs: GenerationRunRepository,
    project_id: &'a str,
}

impl<'a> GraphGeneration<'a> {
    /// Upstream constructor.
    pub fn new(conn: &'a Connection, runs: GenerationRunRepository, project_id: &'a str) -> Self {
        Self {
            conn,
            runs,
            project_id,
        }
    }

    /// Resolve and verify a run handle (upstream `require`).
    pub fn require(&self, handle: &Value) -> Result<DurableGenerationRunAlias, String> {
        let valid_handle = handle.is_object()
            && handle
                .as_object()
                .map(|record| {
                    record.keys().all(|key| {
                        matches!(
                            key.as_str(),
                            "projectId" | "epoch" | "rootActionId" | "runId"
                        )
                    })
                })
                .unwrap_or(false);
        if !valid_handle {
            return Err(GENERATION_RUN_IDENTITY_MISMATCH.to_string());
        }
        let run_id = handle
            .get("runId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let run = self.runs.get(self.conn, run_id)?;
        let context = read_graph_generation_context(&run)?;
        let handle_project_id = handle
            .get("projectId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let handle_root_action_id = handle
            .get("rootActionId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let handle_epoch = handle.get("epoch").and_then(Value::as_str);
        if handle_project_id != self.project_id
            || run.binding.project_id != self.project_id
            || run.root_action_id != handle_root_action_id
            || !handle_epoch
                .is_some_and(|epoch| epoch == run.binding.epoch || epoch == context.origin_epoch())
        {
            return Err(GENERATION_RUN_IDENTITY_MISMATCH.to_string());
        }
        Ok(run)
    }

    /// Re-read and verify the stored task (upstream `task`).
    pub fn task(&self, run: &DurableGenerationRunAlias) -> Result<GraphGenerationTask, String> {
        read_graph_generation_context(run)?;
        let manifest = &run.binding.source_manifest;
        let Some(task) = manifest
            .get("graphGenerationTask")
            .filter(|task| !task.is_null())
        else {
            return Err(GENERATION_GRAPH_TASK_INVALID.to_string());
        };
        let task_string =
            serde_json::to_string(task).map_err(|_| GENERATION_GRAPH_TASK_INVALID.to_string())?;
        if text_hash(&task_string)
            != manifest
                .get("graphGenerationTaskHash")
                .and_then(Value::as_str)
                .unwrap_or_default()
        {
            return Err(GENERATION_GRAPH_TASK_INVALID.to_string());
        }
        serde_json::from_value(task.clone()).map_err(|_| GENERATION_GRAPH_TASK_INVALID.to_string())
    }

    /// Prove an artifact reference and derive its result (upstream
    /// private `proof`).
    fn proof(
        &self,
        run: &DurableGenerationRunAlias,
        reference: &Value,
        legacy_stored_effect: bool,
    ) -> Result<ProofOutcome, String> {
        let valid_reference = reference.is_object()
            && reference
                .as_object()
                .map(|record| {
                    record
                        .keys()
                        .all(|key| matches!(key.as_str(), "artifactId" | "revision" | "textHash"))
                })
                .unwrap_or(false);
        if !valid_reference {
            return Err(GENERATION_GRAPH_ARTIFACT_INVALID.to_string());
        }
        let artifact_id = reference
            .get("artifactId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let reference_revision = reference
            .get("revision")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let reference_text_hash = reference
            .get("textHash")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let row = self
            .conn
            .query_row(
                "SELECT run_id, attempt_id, status FROM generation_artifacts WHERE artifact_id = ?1",
                [artifact_id],
                |row| {
                    Ok((
                        row.get::<_, String>("run_id")?,
                        row.get::<_, String>("attempt_id")?,
                        row.get::<_, String>("status")?,
                    ))
                },
            )
            .optional()
            .map_err(|_| GENERATION_GRAPH_ARTIFACT_INVALID.to_string())?;
        let Some((run_id, attempt_id, status)) = row else {
            return Err(GENERATION_GRAPH_ARTIFACT_INVALID.to_string());
        };
        if run_id != run.run_id || status == "discarded" {
            return Err(GENERATION_GRAPH_ARTIFACT_INVALID.to_string());
        }
        let receipt = self.runs.receipt(self.conn, &attempt_id, None)?;
        let Some(artifact) = receipt.artifact else {
            return Err(GENERATION_GRAPH_ARTIFACT_INVALID.to_string());
        };
        if artifact.revision != reference_revision
            || artifact.text_hash != reference_text_hash
            || text_hash(&artifact.text) != reference_text_hash
            || artifact.text.trim().is_empty()
            || receipt.failure_code.is_some()
            || !matches!(receipt.attempt.status.as_str(), "settled" | "unknown")
            || receipt
                .result
                .as_ref()
                .and_then(|result| result.finish_reason.as_deref())
                != Some("stop")
        {
            return Err(GENERATION_GRAPH_ARTIFACT_INVALID.to_string());
        }
        let context = read_graph_generation_context(run)?;
        let result =
            derive_graph_generation_result(&context, &artifact.text, legacy_stored_effect)?;
        Ok(ProofOutcome {
            attempt_id,
            artifact,
            context,
            result,
        })
    }

    /// Effect payload proof (upstream private `assertEffectPayload`).
    fn assert_effect_payload(
        &self,
        context: &GraphGenerationContext,
        result: &GraphGenerationResult,
        effect: &GraphGenerationEffect,
    ) -> Result<(), String> {
        if !effect.success()
            || effect.kind() != result.kind()
            || !is_safe_integer(effect.index())
            || effect.index() < 0
        {
            return Err(GENERATION_GRAPH_EFFECT_INVALID.to_string());
        }
        match (context, result, effect) {
            (
                GraphGenerationContext::Plot(_),
                GraphGenerationResult::Plot {
                    snapshot,
                    derivation,
                },
                GraphGenerationEffect::Plot {
                    index,
                    snapshot: effect_snapshot,
                    derivation: effect_derivation,
                    ..
                },
            ) => {
                if *index != 0
                    || !values_equal(snapshot, effect_snapshot)
                    || derivation != effect_derivation
                {
                    return Err(GENERATION_GRAPH_EFFECT_INVALID.to_string());
                }
            }
            (
                GraphGenerationContext::Plan(_),
                GraphGenerationResult::Plan { candidates },
                GraphGenerationEffect::Plan { index, plan, .. },
            ) => {
                let Some(candidate) = candidates.get(*index as usize) else {
                    return Err(GENERATION_GRAPH_EFFECT_INVALID.to_string());
                };
                if !is_safe_integer(plan.id)
                    || plan.id < 1
                    || !values_equal(
                        &candidate_payload(candidate),
                        &candidate_payload_from_record(plan),
                    )
                {
                    return Err(GENERATION_GRAPH_EFFECT_INVALID.to_string());
                }
            }
            (
                GraphGenerationContext::Event(event),
                GraphGenerationResult::Event { candidates },
                GraphGenerationEffect::Event {
                    index,
                    event: stored,
                    ..
                },
            ) => {
                let Some(candidate) = candidates.get(*index as usize) else {
                    return Err(GENERATION_GRAPH_EFFECT_INVALID.to_string());
                };
                if !is_safe_integer(stored.id)
                    || stored.id < 1
                    || stored.plan_id != event.plan.id
                    || stored.draft_id != event.source.draft_id
                    || stored.chapter_number != event.source.chapter_number
                    || !values_equal(
                        &serde_json::to_value(candidate)
                            .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?,
                        &serde_json::json!({
                            "type": stored.event_type,
                            "evidence": stored.evidence,
                            "reason": stored.reason,
                        }),
                    )
                {
                    return Err(GENERATION_GRAPH_EFFECT_INVALID.to_string());
                }
            }
            _ => return Err(GENERATION_GRAPH_EFFECT_INVALID.to_string()),
        }
        Ok(())
    }

    /// Verified stored effects (upstream private `storedEffects`).
    fn stored_effects(
        &self,
        run: &DurableGenerationRunAlias,
    ) -> Result<Vec<StoredGraphEffect>, String> {
        let context = read_graph_generation_context(run)?;
        let context_hash = text_hash(
            &serde_json::to_string(&context)
                .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?,
        );
        let mut statement = self
            .conn
            .prepare(
                "SELECT attempt_id, usage_receipt_json FROM generation_attempts WHERE run_id = ?1 ORDER BY rowid",
            )
            .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?;
        let rows = statement
            .query_map([run.run_id.as_str()], |row| {
                Ok((
                    row.get::<_, String>("attempt_id")?,
                    row.get::<_, String>("usage_receipt_json")?,
                ))
            })
            .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?;
        let mut effects = Vec::new();
        for row in rows {
            let (attempt_id, usage_receipt_json) =
                row.map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?;
            let Some(saved_value) = serde_json::from_str::<Value>(&usage_receipt_json)
                .ok()
                .and_then(|receipt| {
                    receipt
                        .get("graphEffects")
                        .filter(|saved| !saved.is_null())
                        .cloned()
                })
            else {
                continue;
            };
            let Some(saved) = saved_value.as_array() else {
                return Err(GENERATION_GRAPH_EFFECT_INVALID.to_string());
            };
            if saved.len() > 8 {
                return Err(GENERATION_GRAPH_EFFECT_INVALID.to_string());
            }
            for effect in saved {
                let effect: StoredGraphEffect = serde_json::from_value(effect.clone())
                    .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?;
                let proof = self.proof(
                    run,
                    &serde_json::to_value(&effect.artifact)
                        .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?,
                    true,
                )?;
                if effect.version != 1
                    || proof.attempt_id != attempt_id
                    || effect.context_hash != context_hash
                    || effect.index != effect.receipt.index()
                    || text_hash(
                        &serde_json::to_string(&effect.receipt)
                            .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?,
                    ) != effect.receipt_hash
                    || effects
                        .iter()
                        .any(|existing: &StoredGraphEffect| existing.index == effect.index)
                {
                    return Err(GENERATION_GRAPH_EFFECT_INVALID.to_string());
                }
                self.assert_effect_payload(&context, &proof.result, &effect.receipt)?;
                effects.push(effect);
            }
        }
        Ok(effects)
    }

    /// Verified effect receipts (upstream `effects`).
    pub fn effects(
        &self,
        run: &DurableGenerationRunAlias,
    ) -> Result<Vec<GraphGenerationEffect>, String> {
        Ok(self
            .stored_effects(run)?
            .into_iter()
            .map(|effect| effect.receipt)
            .collect())
    }

    /// Reject writes onto a run that already sealed effects (upstream
    /// `assertMutable`).
    pub fn assert_mutable(&self, run: &DurableGenerationRunAlias) -> Result<(), String> {
        if run
            .binding
            .source_manifest
            .contains_key("graphGenerationContext")
            && !self.effects(run)?.is_empty()
        {
            return Err(GENERATION_GRAPH_EFFECT_SEALED.to_string());
        }
        Ok(())
    }

    /// Derive the result for an artifact reference (upstream `result`).
    pub fn result(
        &self,
        run: &DurableGenerationRunAlias,
        reference: &Value,
    ) -> Result<GraphGenerationResult, String> {
        Ok(self.proof(run, reference, false)?.result)
    }

    /// Proven event IDs owned by this run (upstream `ownEventIds`).
    pub fn own_event_ids(&self, run: &DurableGenerationRunAlias) -> Result<Vec<i64>, String> {
        let mut ids = Vec::new();
        for effect in self.effects(run)? {
            let GraphGenerationEffect::Event { event, .. } = effect else {
                continue;
            };
            let row = self
                .conn
                .query_row(
                    "SELECT e.id AS id, e.plan_id AS planId, e.draft_id AS draftId, \
                     e.event_type AS type, e.evidence AS evidence, e.reason AS reason, \
                     e.created_at AS createdAt, d.chapter_number AS chapterNumber, \
                     o.chapter_title AS chapterTitle \
                     FROM narrative_thread_confirmations e \
                     JOIN drafts d ON d.id = e.draft_id AND d.status = 'finalized' \
                     JOIN finalization_outbox o ON o.draft_id = d.id \
                     WHERE e.id = ?1",
                    [event.id],
                    |row| {
                        Ok(serde_json::json!({
                            "id": row.get::<_, i64>("id")?,
                            "planId": row.get::<_, i64>("planId")?,
                            "draftId": row.get::<_, i64>("draftId")?,
                            "type": row.get::<_, String>("type")?,
                            "evidence": row.get::<_, String>("evidence")?,
                            "reason": row.get::<_, String>("reason")?,
                            "createdAt": row.get::<_, String>("createdAt")?,
                            "chapterNumber": row.get::<_, i64>("chapterNumber")?,
                            "chapterTitle": row.get::<_, String>("chapterTitle")?,
                        }))
                    },
                )
                .optional()
                .map_err(|_| GENERATION_GRAPH_OWN_EFFECT_CHANGED.to_string())?;
            let Some(row) = row else {
                return Err(GENERATION_GRAPH_OWN_EFFECT_CHANGED.to_string());
            };
            let expected = serde_json::json!({
                "id": event.id,
                "planId": event.plan_id,
                "draftId": event.draft_id,
                "type": event.event_type,
                "evidence": event.evidence,
                "reason": event.reason,
                "createdAt": event.created_at,
                "chapterNumber": event.chapter_number,
                "chapterTitle": event.chapter_title,
            });
            if !values_equal(&row, &expected) {
                return Err(GENERATION_GRAPH_OWN_EFFECT_CHANGED.to_string());
            }
            ids.push(event.id);
        }
        Ok(ids)
    }

    /// Confirm one effect index (upstream `confirm`).
    ///
    /// `assert_sources` is the caller's pre-commit validation hook
    /// (upstream passes `assertSources`); it runs inside the
    /// transaction before any write.
    pub fn confirm<F>(
        &self,
        handle: &Value,
        reference: &Value,
        index: i64,
        assert_sources: F,
    ) -> Result<GraphGenerationEffect, String>
    where
        F: FnOnce(&DurableGenerationRunAlias) -> Result<(), String>,
    {
        let transaction = self
            .conn
            .unchecked_transaction()
            .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?;
        let outcome = (|| -> Result<GraphGenerationEffect, String> {
            let run = self.require_in(&transaction, handle)?;
            if !is_safe_integer(index) || index < 0 {
                return Err(GENERATION_GRAPH_SELECTION_INVALID.to_string());
            }
            let stored = self
                .stored_effects_in(&transaction, &run)?
                .into_iter()
                .find(|effect| effect.index == index);
            if let Some(stored) = stored {
                if !values_equal(
                    &serde_json::to_value(&stored.artifact)
                        .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?,
                    reference,
                ) {
                    return Err(GENERATION_GRAPH_EFFECT_CONFLICT.to_string());
                }
                return Ok(stored.receipt);
            }
            let proof = self.proof_in(&transaction, &run, reference, false)?;
            assert_sources(&run)?;
            let receipt = match &proof.context {
                GraphGenerationContext::Plot(plot)
                    if proof.result.kind() == GraphGenerationKind::Plot =>
                {
                    if index != 0 {
                        return Err(GENERATION_GRAPH_SELECTION_INVALID.to_string());
                    }
                    let GraphGenerationResult::Plot { snapshot, .. } = &proof.result else {
                        return Err(GENERATION_GRAPH_SELECTION_INVALID.to_string());
                    };
                    let current_snapshot: Option<String> = transaction
                        .query_row(
                            "SELECT plot_tree_snapshot FROM project_core WHERE id = 'main'",
                            [],
                            |row| row.get::<_, String>("plot_tree_snapshot"),
                        )
                        .optional()
                        .map_err(|_| GENERATION_GRAPH_TARGET_CHANGED.to_string())?;
                    if text_hash(current_snapshot.as_deref().unwrap_or_default())
                        != plot.target_baseline_hash
                    {
                        return Err(GENERATION_GRAPH_TARGET_CHANGED.to_string());
                    }
                    let snapshot = save(
                        &transaction,
                        &serde_json::to_value(snapshot)
                            .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?,
                        &plot.sources.source_revision,
                    )?;
                    GraphGenerationEffect::Plot {
                        success: true,
                        index,
                        snapshot,
                        derivation: match &proof.result {
                            GraphGenerationResult::Plot { derivation, .. } => derivation.clone(),
                            _ => unreachable!(),
                        },
                    }
                }
                GraphGenerationContext::Plan(_)
                    if proof.result.kind() == GraphGenerationKind::Plan =>
                {
                    let GraphGenerationResult::Plan { candidates } = &proof.result else {
                        return Err(GENERATION_GRAPH_SELECTION_INVALID.to_string());
                    };
                    let Some(candidate) = candidates.get(index as usize) else {
                        return Err(GENERATION_GRAPH_SELECTION_INVALID.to_string());
                    };
                    let plan = create_plan(&transaction, candidate)?;
                    GraphGenerationEffect::Plan {
                        success: true,
                        index,
                        plan,
                    }
                }
                GraphGenerationContext::Event(event)
                    if proof.result.kind() == GraphGenerationKind::Event =>
                {
                    let GraphGenerationResult::Event { candidates } = &proof.result else {
                        return Err(GENERATION_GRAPH_SELECTION_INVALID.to_string());
                    };
                    let Some(candidate) = candidates.get(index as usize) else {
                        return Err(GENERATION_GRAPH_SELECTION_INVALID.to_string());
                    };
                    let confirmed = confirm_event(
                        &transaction,
                        &NarrativeThreadEventInput {
                            plan_id: event.plan.id,
                            draft_id: event.source.draft_id,
                            event_type: candidate.event_type.clone(),
                            evidence: candidate.evidence.clone(),
                            reason: candidate.reason.clone(),
                        },
                    )?;
                    GraphGenerationEffect::Event {
                        success: true,
                        index,
                        event: confirmed,
                    }
                }
                _ => return Err(GENERATION_GRAPH_SELECTION_INVALID.to_string()),
            };
            self.assert_effect_payload(&proof.context, &proof.result, &receipt)?;
            let usage: Value = transaction
                .query_row(
                    "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id = ?1",
                    [proof.attempt_id.as_str()],
                    |row| row.get::<_, String>("usage_receipt_json"),
                )
                .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())
                .and_then(|json| {
                    serde_json::from_str(&json)
                        .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())
                })?;
            let effect = StoredGraphEffect {
                version: 1,
                artifact: GraphGenerationArtifact {
                    artifact_id: proof.artifact.artifact_id.clone(),
                    revision: proof.artifact.revision,
                    text_hash: proof.artifact.text_hash.clone(),
                },
                index,
                context_hash: text_hash(
                    &serde_json::to_string(&proof.context)
                        .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?,
                ),
                receipt_hash: text_hash(
                    &serde_json::to_string(&receipt)
                        .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?,
                ),
                receipt,
            };
            let mut graph_effects: Vec<Value> = usage
                .get("graphEffects")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            graph_effects.push(
                serde_json::to_value(&effect)
                    .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?,
            );
            let mut merged = usage.as_object().cloned().unwrap_or_default();
            merged.insert(
                "graphEffects".to_string(),
                serde_json::to_value(&graph_effects)
                    .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?,
            );
            transaction
                .execute(
                    "UPDATE generation_attempts SET usage_receipt_json = ?1 WHERE attempt_id = ?2",
                    [
                        serde_json::to_string(&merged)
                            .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?,
                        proof.attempt_id.clone(),
                    ],
                )
                .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?;
            Ok(effect.receipt)
        })();
        match outcome {
            Ok(receipt) => {
                transaction
                    .commit()
                    .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?;
                Ok(receipt)
            }
            Err(error) => {
                let _ = transaction.rollback();
                Err(error)
            }
        }
    }

    /// `require` against an explicit connection (transaction alias).
    fn require_in(
        &self,
        conn: &Connection,
        handle: &Value,
    ) -> Result<DurableGenerationRunAlias, String> {
        let valid_handle = handle.is_object()
            && handle
                .as_object()
                .map(|record| {
                    record.keys().all(|key| {
                        matches!(
                            key.as_str(),
                            "projectId" | "epoch" | "rootActionId" | "runId"
                        )
                    })
                })
                .unwrap_or(false);
        if !valid_handle {
            return Err(GENERATION_RUN_IDENTITY_MISMATCH.to_string());
        }
        let run_id = handle
            .get("runId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let run = GenerationRunRepository::default().get(conn, run_id)?;
        let context = read_graph_generation_context(&run)?;
        let handle_project_id = handle
            .get("projectId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let handle_root_action_id = handle
            .get("rootActionId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let handle_epoch = handle.get("epoch").and_then(Value::as_str);
        if handle_project_id != self.project_id
            || run.binding.project_id != self.project_id
            || run.root_action_id != handle_root_action_id
            || !handle_epoch
                .is_some_and(|epoch| epoch == run.binding.epoch || epoch == context.origin_epoch())
        {
            return Err(GENERATION_RUN_IDENTITY_MISMATCH.to_string());
        }
        Ok(run)
    }

    /// `proof` against an explicit connection (transaction alias).
    fn proof_in(
        &self,
        conn: &Connection,
        run: &DurableGenerationRunAlias,
        reference: &Value,
        legacy_stored_effect: bool,
    ) -> Result<ProofOutcome, String> {
        let valid_reference = reference.is_object()
            && reference
                .as_object()
                .map(|record| {
                    record
                        .keys()
                        .all(|key| matches!(key.as_str(), "artifactId" | "revision" | "textHash"))
                })
                .unwrap_or(false);
        if !valid_reference {
            return Err(GENERATION_GRAPH_ARTIFACT_INVALID.to_string());
        }
        let artifact_id = reference
            .get("artifactId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let reference_revision = reference
            .get("revision")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let reference_text_hash = reference
            .get("textHash")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let row = conn
            .query_row(
                "SELECT run_id, attempt_id, status FROM generation_artifacts WHERE artifact_id = ?1",
                [artifact_id],
                |row| {
                    Ok((
                        row.get::<_, String>("run_id")?,
                        row.get::<_, String>("attempt_id")?,
                        row.get::<_, String>("status")?,
                    ))
                },
            )
            .optional()
            .map_err(|_| GENERATION_GRAPH_ARTIFACT_INVALID.to_string())?;
        let Some((run_id, attempt_id, status)) = row else {
            return Err(GENERATION_GRAPH_ARTIFACT_INVALID.to_string());
        };
        if run_id != run.run_id || status == "discarded" {
            return Err(GENERATION_GRAPH_ARTIFACT_INVALID.to_string());
        }
        let receipt = GenerationRunRepository::default().receipt(conn, &attempt_id, None)?;
        let Some(artifact) = receipt.artifact else {
            return Err(GENERATION_GRAPH_ARTIFACT_INVALID.to_string());
        };
        if artifact.revision != reference_revision
            || artifact.text_hash != reference_text_hash
            || text_hash(&artifact.text) != reference_text_hash
            || artifact.text.trim().is_empty()
            || receipt.failure_code.is_some()
            || !matches!(receipt.attempt.status.as_str(), "settled" | "unknown")
            || receipt
                .result
                .as_ref()
                .and_then(|result| result.finish_reason.as_deref())
                != Some("stop")
        {
            return Err(GENERATION_GRAPH_ARTIFACT_INVALID.to_string());
        }
        let context = read_graph_generation_context(run)?;
        let result =
            derive_graph_generation_result(&context, &artifact.text, legacy_stored_effect)?;
        Ok(ProofOutcome {
            attempt_id,
            artifact,
            context,
            result,
        })
    }

    /// `storedEffects` against an explicit connection (transaction alias).
    fn stored_effects_in(
        &self,
        conn: &Connection,
        run: &DurableGenerationRunAlias,
    ) -> Result<Vec<StoredGraphEffect>, String> {
        let context = read_graph_generation_context(run)?;
        let context_hash = text_hash(
            &serde_json::to_string(&context)
                .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?,
        );
        let mut statement = conn
            .prepare(
                "SELECT attempt_id, usage_receipt_json FROM generation_attempts WHERE run_id = ?1 ORDER BY rowid",
            )
            .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?;
        let rows = statement
            .query_map([run.run_id.as_str()], |row| {
                Ok((
                    row.get::<_, String>("attempt_id")?,
                    row.get::<_, String>("usage_receipt_json")?,
                ))
            })
            .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?;
        let mut effects = Vec::new();
        for row in rows {
            let (attempt_id, usage_receipt_json) =
                row.map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?;
            let Some(saved_value) = serde_json::from_str::<Value>(&usage_receipt_json)
                .ok()
                .and_then(|receipt| {
                    receipt
                        .get("graphEffects")
                        .filter(|saved| !saved.is_null())
                        .cloned()
                })
            else {
                continue;
            };
            let Some(saved) = saved_value.as_array() else {
                return Err(GENERATION_GRAPH_EFFECT_INVALID.to_string());
            };
            if saved.len() > 8 {
                return Err(GENERATION_GRAPH_EFFECT_INVALID.to_string());
            }
            for effect in saved {
                let effect: StoredGraphEffect = serde_json::from_value(effect.clone())
                    .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?;
                let proof = self.proof_in(
                    conn,
                    run,
                    &serde_json::to_value(&effect.artifact)
                        .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?,
                    true,
                )?;
                if effect.version != 1
                    || proof.attempt_id != attempt_id
                    || effect.context_hash != context_hash
                    || effect.index != effect.receipt.index()
                    || text_hash(
                        &serde_json::to_string(&effect.receipt)
                            .map_err(|_| GENERATION_GRAPH_EFFECT_INVALID.to_string())?,
                    ) != effect.receipt_hash
                    || effects
                        .iter()
                        .any(|existing: &StoredGraphEffect| existing.index == effect.index)
                {
                    return Err(GENERATION_GRAPH_EFFECT_INVALID.to_string());
                }
                self.assert_effect_payload(&context, &proof.result, &effect.receipt)?;
                effects.push(effect);
            }
        }
        Ok(effects)
    }
}

/// Free function form of `ownEventIds` (upstream
/// `provenGraphOwnEventIds`).
///
/// Non-event contexts own nothing; otherwise the unique run sharing
/// this context key is re-read and its proven event IDs returned.
pub fn proven_graph_own_event_ids(
    conn: &Connection,
    context: &GraphGenerationContext,
) -> Result<Vec<i64>, String> {
    if context.kind() != GraphGenerationKind::Event {
        return Ok(Vec::new());
    }
    let key = context.key();
    // 便携副本按 R 策略中和 binding_json，SQL 的 json_extract 会抛 malformed JSON。
    // 共用读面逐行判定：认证冻结的历史 run 不参与当前上下文归属，
    // 非冻结行仍严格解析（坏绑定照原样报错）。
    let run_ids = crate::services::portable_project_guard::frozen_aware_json_pointer_rows(
        conn,
        "generation_runs",
        "run_id",
        "binding_json",
        "/sourceManifest/graphGenerationContext/key",
        &[key],
    )
    .map_err(|_| GENERATION_GRAPH_CONTEXT_INVALID.to_string())?;
    if run_ids.len() > 1 {
        return Err(GENERATION_GRAPH_CONTEXT_AMBIGUOUS.to_string());
    }
    let Some(run_id) = run_ids.first() else {
        return Ok(Vec::new());
    };
    let runs = GenerationRunRepository::default();
    let run = runs.get(conn, run_id)?;
    if run.binding.project_id != context.project_id()
        || !values_equal(
            &serde_json::to_value(read_graph_generation_context(&run)?)
                .map_err(|_| GENERATION_GRAPH_CONTEXT_INVALID.to_string())?,
            &serde_json::to_value(context)
                .map_err(|_| GENERATION_GRAPH_CONTEXT_INVALID.to_string())?,
        )
    {
        return Err(GENERATION_GRAPH_CONTEXT_INVALID.to_string());
    }
    GraphGeneration::new(conn, runs, context.project_id()).own_event_ids(&run)
}

/// `DurableGenerationRun` re-export alias (keeps signatures short).
pub type DurableGenerationRunAlias =
    crate::repositories::generation_run_repository::DurableGenerationRun;

/// Text hash — UTF-8 bytes, SHA-256, lowercase hex (private helper;
/// mirrors the private `text_hash` in `generation_run_repository`).
fn text_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    let mut hash = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hash.push_str(&format!("{byte:02x}"));
    }
    hash
}

/// `isDeepStrictEqual` via canonical `serde_json::Value` comparison
/// (private helper; mirrors the private `values_equal` in
/// `generation_run_repository`).
fn values_equal<T: Serialize + ?Sized, U: Serialize + ?Sized>(a: &T, b: &U) -> bool {
    serde_json::to_value(a)
        .ok()
        .and_then(|left| serde_json::to_value(b).ok().map(|right| (left, right)))
        .map(|(left, right)| left == right)
        .unwrap_or(false)
}

/// JS `Number.isSafeInteger`.
fn is_safe_integer(value: i64) -> bool {
    value.abs() <= 9_007_199_254_740_991
}

/// Plan candidate payload (upstream destructured
/// `{ title, type, targetStartChapter, targetEndChapter, authorIntent }`).
fn candidate_payload(candidate: &NarrativeThreadPlanInput) -> Value {
    serde_json::json!({
        "title": candidate.title,
        "type": candidate.thread_type,
        "targetStartChapter": candidate.target_start_chapter,
        "targetEndChapter": candidate.target_end_chapter,
        "authorIntent": candidate.author_intent,
    })
}

/// Stored plan record payload for the deep-equal check (same field
/// subset as [`candidate_payload`]).
fn candidate_payload_from_record(plan: &NarrativeThreadPlanRecord) -> Value {
    serde_json::json!({
        "title": plan.title,
        "type": plan.thread_type,
        "targetStartChapter": plan.target_start_chapter,
        "targetEndChapter": plan.target_end_chapter,
        "authorIntent": plan.author_intent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::blueprint_repository::BlueprintData;
    use crate::repositories::narrative_thread_repository::NarrativeThreadPlanInput;
    use crate::services::graph_generation_source::{
        GraphGenerationBasePayload, GraphGenerationInput, GraphGenerationPlanContext,
    };
    use crate::services::narrative_thread_generation_pure::{
        build_narrative_thread_plan_task, NarrativeThreadPlanTaskInput,
        NARRATIVE_THREAD_CANDIDATES_INVALID,
    };
    use serde_json::json;

    /// Plan context fixture — built directly so derivation tests
    /// do not depend on persisted author facts (the capture path
    /// requires a blueprint row).
    fn plan_context_fixture() -> GraphGenerationPlanContext {
        GraphGenerationPlanContext {
            base: GraphGenerationBasePayload {
                project_id: "project-1".to_string(),
                origin_epoch: "epoch-1".to_string(),
                key: "key-1".to_string(),
                created_at: "2026-01-01T00:00:00.000Z".to_string(),
                input: GraphGenerationInput::Plan { chapter_number: 1 },
                writing_language: "zh-CN".to_string(),
            },
            kind: GraphGenerationKind::Plan,
            blueprint: BlueprintData {
                chapter_number: 1,
                title: "t".to_string(),
                role: String::new(),
                purpose: String::new(),
                key_events: String::new(),
                characters: Vec::new(),
                new_character_candidates: None,
                relationship_hints: None,
                suspense_hook: String::new(),
                user_guidance: String::new(),
                notes: String::new(),
                notes_updated_at: String::new(),
            },
            total_chapters: 2,
        }
    }

    #[test]
    fn result_and_effect_serialization_key_order() {
        let result = GraphGenerationResult::Plan {
            candidates: vec![NarrativeThreadPlanInput {
                title: "t".to_string(),
                thread_type: "foreshadowing".to_string(),
                target_start_chapter: 1,
                target_end_chapter: 2,
                author_intent: "i".to_string(),
            }],
        };
        assert!(serde_json::to_string(&result)
            .unwrap()
            .starts_with(r#"{"kind":"plan","candidates":"#));
        let effect = GraphGenerationEffect::Plan {
            success: true,
            index: 0,
            plan: NarrativeThreadPlanRecord {
                id: 1,
                title: "t".to_string(),
                thread_type: "foreshadowing".to_string(),
                target_start_chapter: 1,
                target_end_chapter: 2,
                author_intent: "i".to_string(),
                created_at: "c".to_string(),
                updated_at: "u".to_string(),
            },
        };
        assert!(serde_json::to_string(&effect)
            .unwrap()
            .starts_with(r#"{"success":true,"index":0,"kind":"plan","plan":"#));
    }

    #[test]
    fn derive_result_round_trips_through_value() {
        let candidate = NarrativeThreadPlanInput {
            title: "t".to_string(),
            thread_type: "foreshadowing".to_string(),
            target_start_chapter: 1,
            target_end_chapter: 2,
            author_intent: "i".to_string(),
        };
        let result = GraphGenerationResult::Plan {
            candidates: vec![candidate.clone()],
        };
        let value = serde_json::to_value(&result).unwrap();
        let parsed: GraphGenerationResult = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, result);
        let effect = GraphGenerationEffect::Plan {
            success: true,
            index: 0,
            plan: NarrativeThreadPlanRecord {
                id: 5,
                title: "t".to_string(),
                thread_type: "foreshadowing".to_string(),
                target_start_chapter: 1,
                target_end_chapter: 2,
                author_intent: "i".to_string(),
                created_at: "c".to_string(),
                updated_at: "u".to_string(),
            },
        };
        let effect_value = serde_json::to_value(&effect).unwrap();
        let parsed_effect: GraphGenerationEffect = serde_json::from_value(effect_value).unwrap();
        assert_eq!(parsed_effect, effect);
        assert!(parsed_effect.success());
        assert_eq!(parsed_effect.index(), 0);
        assert_eq!(parsed_effect.kind(), GraphGenerationKind::Plan);
    }

    #[test]
    fn derive_result_rejects_unknown_kind() {
        let error = serde_json::from_value::<GraphGenerationResult>(json!({"kind": "nope"}))
            .unwrap_err()
            .to_string();
        assert!(error.contains(GENERATION_GRAPH_EFFECT_INVALID));
        let error = serde_json::from_value::<GraphGenerationEffect>(json!({"kind": "nope"}))
            .unwrap_err()
            .to_string();
        assert!(error.contains(GENERATION_GRAPH_EFFECT_INVALID));
    }

    #[test]
    fn derive_result_recovers_legacy_invalid_candidates() {
        let context = GraphGenerationContext::Plan(plan_context_fixture());
        // Legacy stored-effect recovery skips invalid candidates; malformed
        // JSON still fails (upstream `JSON.parse` semantics).
        let text = r#"{"candidates":[{"title":""}]}"#;
        let result = derive_graph_generation_result(&context, text, true).unwrap();
        let GraphGenerationResult::Plan { candidates } = result else {
            panic!("expected plan result");
        };
        assert!(candidates.is_empty());
        let error = derive_graph_generation_result(&context, "not json", true).unwrap_err();
        assert_eq!(error, NARRATIVE_THREAD_CANDIDATES_INVALID);
    }

    #[test]
    fn derive_result_for_plan_context() {
        let context = GraphGenerationContext::Plan(plan_context_fixture());
        let text = serde_json::json!({
            "candidates": [{
                "title": "t",
                "type": "foreshadowing",
                "targetStartChapter": 1,
                "targetEndChapter": 2,
                "authorIntent": "i",
            }],
        })
        .to_string();
        let result = derive_graph_generation_result(&context, &text, false).unwrap();
        assert_eq!(result.kind(), GraphGenerationKind::Plan);
        let GraphGenerationResult::Plan { candidates } = result else {
            panic!("expected plan result");
        };
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].title, "t");
    }

    #[test]
    fn build_task_matches_source_binder_task() {
        // The same plan context must yield the same task from both modules.
        let plan = plan_context_fixture();
        let context = GraphGenerationContext::Plan(plan.clone());
        let task =
            crate::services::graph_generation_source::graph_generation_task(&context).unwrap();
        assert_eq!(task.purpose, "narrative-thread-plan-candidate");
        assert_eq!(task.reasoning_stage, "planning");
        assert_eq!(task.output, "structured-data");
        assert_eq!(task.messages.len(), 2);
        let expected = build_narrative_thread_plan_task(NarrativeThreadPlanTaskInput {
            writing_language: plan.base.writing_language.clone(),
            total_chapters: plan.total_chapters,
            blueprint: plan.blueprint.clone(),
        });
        assert_eq!(task, expected);
    }
    #[test]
    fn require_rejects_foreign_keys_in_handle() {
        let conn = test_connection();
        let runs = GenerationRunRepository::default();
        let owner = GraphGeneration::new(&conn, runs, "project-1");
        let error = owner
            .require(&json!({"projectId": "p", "epoch": "e", "rootActionId": "r", "runId": "x", "extra": 1}))
            .unwrap_err();
        assert_eq!(error, GENERATION_RUN_IDENTITY_MISMATCH);
        let error = owner.require(&json!("not-an-object")).unwrap_err();
        assert_eq!(error, GENERATION_RUN_IDENTITY_MISMATCH);
    }

    #[test]
    fn text_hash_is_lowercase_sha256_hex() {
        let hash = text_hash("");
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(hash.len(), 64);
        assert!(hash
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }

    #[test]
    fn values_equal_compares_canonical_json() {
        assert!(values_equal(&json!({"a": 1}), &json!({"a": 1})));
        assert!(!values_equal(&json!({"a": 1}), &json!({"a": 2})));
        // serde_json Map is ordered; key order is significant — mirrors
        // the intra-stack consistency contract (AGENTS.md: Node byte
        // equality is not required across stacks).
        assert!(!values_equal(&json!([1, 2]), &json!([2, 1])));
    }

    /// In-memory DB with the full schema (mirrors repository tests).
    fn test_connection() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory DB");
        conn.pragma_update(None, "foreign_keys", true)
            .expect("enable foreign keys");
        crate::db::schema::create_tables(&conn).expect("create tables");
        conn
    }
}
