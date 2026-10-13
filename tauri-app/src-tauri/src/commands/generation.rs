//! 批次 G4-5：主生成 owner 读面命令 —— `generation:*` 7 频道。
//!
//! 平移自 `electron/controllers/generation-controller.ts` 的
//! 注册项（owner 接线见 `electron/services/main-generation-owner.ts`
//! 与 `electron/services/generation-draft-effects.ts`）：
//! - `generation:read` → `owner.read(handle)`（:734 既有
//!   `MainGenerationOwner::read_run`）
//! - `generation:list` → `owner.list()`（:742 既有）
//! - `generation:list-directory-progress` →
//!   `owner.listDirectoryProgress()`（:767 既有）
//! - `generation:read-visible-composition` →
//!   `owner.readVisibleComposition(handle)`（:812 既有）
//! - `generation:read-batch` →
//!   `draftEffects.readBatch(batchId)`（controller :250；
//!   owner 闭包 :880 经 `deps.projectId` 补项目身份）
//! - `generation:list-batches` → `owner.listBatches()`
//!   （controller :251；owner :1286——`list()` 过滤
//!   `sourceManifest.batchIntent` 后逐条 `readBatchHistory`）
//! - `generation:read-context` → `readContext(handle)`
//!   （owner :834——`GenerationRecoveryContext` 构造，
//!   依赖 `draftEffects.readRecovery` 与运行视图）
//!
//! 命令名对齐前端 `ipc-client.ts` 的 `toCommandName`
//! （`channel.replace(/[:-]/g, '_')`），入参按频道契约
//! `src/shared/generation-owner-contract.ts` 的
//! `GenerationOwnerChannels` 登记。
//!
//! # 门禁（基线 `authorizedOwner`，generation-controller.ts:86-130）
//! 所有命令先经 `MainGenerationOwnerCache::authorized_owner`
//! 会话再认证 + 活跃项目库校验，再进入 owner 方法
//! （owner 方法内部以 `assert_current` 作二级门禁，
//! 失败收敛为 `GENERATION_OWNER_CLOSED`）。
//!
//! # 刻意偏离（均有上游语义理由）
//! - **owner 缓存按调用构造**：基线 per-database 缓存挂在
//!   controller 闭包状态上；Tauri 侧 `AppState` 的
//!   owner 缓存字段随写面波次（G4-6/G4-7）接入，本批
//!   命令层按调用构造本地缓存——读面 owner 无跨调用可变
//!   状态（`runLeases` 仅在执行面写入、订阅者集合为
//!   B3 TODO），语义等价基线缓存（门禁语义逐条不变）。
//! - **`readContext` 的源现状探针**：基线
//!   `planningContinuation` / `plotOutlineRecovery` /
//!   `readFailedShortOutlineTask` 的末段以
//!   `deps.rebuildBinding(run.binding, currentModelReceipt(...))`
//!   + `compareGenerationSourceBindings` 判定冻结绑定与
//!   当前项目源是否一致；`rebuildBinding` / `loadModel`
//!   依赖（`generation-source-binding.ts`）尚未迁移
//!   （见 `services/main_generation_owner.rs` 头注与
//!   `commands/review_revision.rs` 同类占位说明），本批
//!   按该既定范式以**保守占位**（探针判定为一致）注入，
//!   待 generation-source-binding 迁移波次补齐。影响面：
//!   源已变更时 `planningContinuation.state` /
//!   `plotOutlineRecovery.writeState` 可能报告
//!   `ready` 而非 `source-changed`（其余状态分支
//!   `continued` / `in-flight` / `complete` /
//!   `save-prefix` 均由持久化数据精确计算）。

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter, State};

use crate::repositories::generation_run_repository::{
    MainGenerationRunHandle, PhysicalAttempt, PlotOutlineCursor, PlotOutlineProgress,
    PlotOutlineRange, ProjectCoreSynopsisExpected,
};
use crate::repositories::project_core_repository as project_core;
use crate::security::ProjectSessionContext;
use crate::services::generation_draft_effects::GenerationDraftEffects;
use crate::services::main_generation_owner::{
    map_generation_error, mutating_error, MainGenerationOwnerCache,
};
use crate::state::AppState;

/// 基线 `DRAFT_RECONCILE_PURPOSE`（src/shared/draft-reconciliation.ts）。
const DRAFT_RECONCILE_PURPOSE: &str = "chapter-draft-reconcile";
/// 基线 `DRAFT_SHORT_OUTLINE_PURPOSE`
/// （src/shared/draft-short-outline.ts）。
const DRAFT_SHORT_OUTLINE_PURPOSE: &str = "chapter-draft-short-outline";
/// 基线 `DEFAULT_PLANNING_TARGET_UNITS`
/// （src/shared/plot-outline-contract.ts:9）。
const DEFAULT_PLANNING_TARGET_UNITS: i64 = 600;
/// 基线 `PLANNING_TARGET_UNITS_LIMIT`
/// （src/shared/plot-outline-contract.ts:10）。
const PLANNING_TARGET_UNITS_LIMIT: i64 = 1000;
/// 基线 `GENERATION_PLOT_OUTLINE_PREFIX_INVALID`
/// （src/shared/plot-outline-contract.ts 抛错码）。
const GENERATION_PLOT_OUTLINE_PREFIX_INVALID: &str = "GENERATION_PLOT_OUTLINE_PREFIX_INVALID";
/// 基线 `GENERATION_PLANNING_TARGET_INVALID`
/// （src/shared/plot-outline-contract.ts 抛错码，
/// electron/services/main-generation-plan.ts:112）。
const GENERATION_PLANNING_TARGET_INVALID: &str = "GENERATION_PLANNING_TARGET_INVALID";

/// 基线 `readBatch` 请求包装（频道契约
/// `args: [{ batchId: string }]`）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerationReadBatchRequest {
    pub batch_id: String,
}

/// 基线 `authorizedOwner` 门禁的命令层入口：
/// 会话再认证 + 活跃项目库校验，返回持锁的 owner 守卫。
/// 会话缺失时按基线 register catch 前的守卫文案拒绝
/// （对齐 `commands/review_revision.rs` 的同型门禁）。
fn authorized_gate<'a>(
    cache: &'a MainGenerationOwnerCache,
    state: &'a AppState,
    project_session: Option<&'a ProjectSessionContext>,
) -> Result<crate::services::main_generation_owner::OwnerGuard<'a>, String> {
    let Some(session) = project_session else {
        return Err("缺少项目会话上下文，已拒绝操作".to_string());
    };
    let active = state.active_project_snapshot();
    cache.authorized_owner(session, active.as_ref())
}

/// `generation:read` —— 句柄 → 运行视图
/// （基线 `read(handle)`，main-generation-owner.ts:1315）。
#[tauri::command]
pub async fn generation_read(
    state: State<'_, AppState>,
    handle: MainGenerationRunHandle,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let handle = serde_json::to_value(&handle)
        .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.read_run(conn, &handle))
}

/// `generation:list` —— 当前项目全部运行视图
/// （基线 `list()`，main-generation-owner.ts:292-296）。
#[tauri::command]
pub async fn generation_list(
    state: State<'_, AppState>,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.list(conn))
}

/// `generation:list-directory-progress` —— 目录生成进度清单
/// （基线 `listDirectoryProgress()`，main-generation-owner.ts:1328）。
#[tauri::command]
pub async fn generation_list_directory_progress(
    state: State<'_, AppState>,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.list_directory_progress(conn))
}

/// `generation:read-visible-composition` —— 可见合成回执
/// （基线 `readVisibleComposition(handle)`，
/// main-generation-owner.ts:1332-1336；无合成记录 → `null`）。
#[tauri::command]
pub async fn generation_read_visible_composition(
    state: State<'_, AppState>,
    handle: MainGenerationRunHandle,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let handle = serde_json::to_value(&handle)
        .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.read_visible_composition(conn, &handle))
}

/// `generation:read-batch` —— 批量创作进度快照
/// （基线 owner 闭包 `readBatch`，main-generation-owner.ts:880：
/// `assertCurrent()` 后 `draftEffects.readBatch(batchId, deps.projectId)`）。
#[tauri::command]
pub async fn generation_read_batch(
    state: State<'_, AppState>,
    request: GenerationReadBatchRequest,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let project_id = guard.session().project_id.clone();
    let owner = guard.owner();
    let effects = GenerationDraftEffects;
    state.with_project_db(|conn| {
        effects.read_batch(conn, owner.repository(), &request.batch_id, &project_id)
    })
}

/// `generation:list-batches` —— 全部批量创作历史
/// （基线 `listBatches`，main-generation-owner.ts:1286：
/// `assertCurrent()` 后 `list()` 过滤 `sourceManifest.batchIntent`
/// 命中的运行，逐条 `draftEffects.readBatchHistory(runId, deps.projectId)`）。
#[tauri::command]
pub async fn generation_list_batches(
    state: State<'_, AppState>,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let project_id = guard.session().project_id.clone();
    let owner = guard.owner();
    let effects = GenerationDraftEffects;
    state.with_project_db(|conn| {
        let views = owner.list(conn)?;
        let repository = owner.repository();
        let mut batches = Vec::new();
        for view in views.as_array().cloned().unwrap_or_default() {
            let Some(run_id) = view
                .get("handle")
                .and_then(|handle| handle.get("runId"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            let run = repository
                .get(conn, run_id)
                .map_err(|error| mutating_error(map_generation_error(&error)))?;
            if run
                .binding
                .source_manifest
                .get("batchIntent")
                .filter(|intent| !intent.is_null())
                .is_none()
            {
                continue;
            }
            batches.push(effects.read_batch_history(conn, repository, run_id, &project_id)?);
        }
        Ok(Value::Array(batches))
    })
}

/// `generation:read-context` —— 运行恢复上下文
/// （基线 `readContext(handle)`，main-generation-owner.ts:834-876）。
#[tauri::command]
pub async fn generation_read_context(
    state: State<'_, AppState>,
    handle: MainGenerationRunHandle,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let epoch = guard.session().lease_id.clone();
    let handle = serde_json::to_value(&handle)
        .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
    let owner = guard.owner();
    state.with_project_db(|conn| read_recovery_context(conn, owner, &epoch, &handle))
}

// ============================================================================
// readContext（main-generation-owner.ts:834-876）的平移
// ============================================================================

/// 基线 `planningContinuation(run)` 的返回投影
/// （`PlanningContinuation`，src/shared/generation-owner-contract.ts）。
struct PlanningContinuationProjection {
    source_handle: Value,
    remaining_range: Option<PlotOutlineRange>,
    target_units: i64,
    state: String,
    next_handle: Option<Value>,
}

/// 基线 `plotOutlineRecovery(run, outline)` 的返回投影
/// （`PlotOutlineRecovery`，src/shared/plot-outline-contract.ts）。
#[derive(serde::Serialize)]
struct PlotOutlineRecoveryProjection {
    source_handle: Value,
    lease_epoch: String,
    draft: String,
    complete_chapters: Vec<i64>,
    write_state: Value,
    saved: Value,
}

/// 基线 `shortOutlineContext(run)` 的返回投影
/// （`GenerationRecoveryContext['draftShortOutline']`）。
#[derive(serde::Serialize)]
struct ShortOutlineContextProjection {
    artifact_ids: Vec<String>,
    completed_output: Option<String>,
    prompt_hash: String,
    retry: Value,
    initial_draft_task: Option<Value>,
}

/// 基线 `readContext(handle)`（main-generation-owner.ts:834-876）：
/// 运行恢复上下文——模型身份 / 句柄 / 操作 / 章节号 / 大纲与
/// 大纲恢复 / 规划续作 / 蓝图恢复 / 作者输入 / 选中草稿 /
/// 可见合成 / 末次合成结束原因 / 尝试用途 / 草稿保存态 /
/// 知识快照 / 批次身份 / 定稿对账 / 短大纲上下文 / 选中草稿
/// 证据。
fn read_recovery_context(
    conn: &Connection,
    owner: &crate::services::main_generation_owner::MainGenerationOwner,
    epoch: &str,
    handle: &Value,
) -> Result<Value, String> {
    // 基线 `requireRun(handle)` + `viewOf(run)`（`plotOutlineRecovery`
    // 与 `blueprintRecovery` 消费候选）：`read_run` 内联二者。
    let view = owner.read_run(conn, handle)?;
    let run_id = handle
        .get("runId")
        .and_then(Value::as_str)
        .ok_or_else(|| mutating_error("GENERATION_RUN_IDENTITY_MISMATCH"))?;
    let repository = owner.repository();
    let run = repository
        .get(conn, run_id)
        .map_err(|error| mutating_error(map_generation_error(&error)))?;
    let manifest = &run.binding.source_manifest;
    // 基线 `composition = isPrivatePlanningHistory ? null :
    // repository.readVisibleComposition(runId)`——直接调仓储
    // 原始方法（不经 owner 的 `authorInputs` 富化）。
    let composition = if repository
        .is_private_planning_history(conn, &run.run_id)
        .map_err(|error| mutating_error(map_generation_error(&error)))?
    {
        None
    } else {
        repository
            .read_visible_composition(conn, &run.run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?
    };
    let plot_outline = repository
        .read_plot_outline(conn, &run.run_id, None)
        .map_err(|error| mutating_error(map_generation_error(&error)))?;
    let continuation = planning_continuation(conn, repository, &run)?;
    let blueprint_saved = if manifest.get("operation").and_then(Value::as_str)
        == Some("chapter-blueprint-directory")
    {
        repository
            .read_blueprint_author_edit(conn, &run.run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?
    } else {
        None
    };
    let effects = GenerationDraftEffects;
    let draft_save = effects.read_recovery(conn, repository, &run.run_id)?;
    let material_decision = manifest.get("materialDecision").cloned();
    let required_candidate_draft_ids: HashSet<i64> = material_decision
        .as_ref()
        .and_then(|decision| decision.get("included"))
        .and_then(Value::as_array)
        .map(|included| {
            included
                .iter()
                .filter(|item| {
                    item.get("required")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                        && item
                            .get("sourceId")
                            .and_then(Value::as_str)
                            .is_some_and(|source_id| candidate_draft_id(source_id).is_some())
                })
                .filter_map(|item| {
                    item.get("sourceId")
                        .and_then(Value::as_str)
                        .and_then(candidate_draft_id)
                })
                .collect()
        })
        .unwrap_or_default();
    let selected_drafts: Vec<Value> = manifest
        .get("selectedDraftIds")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|draft_id| {
            let draft_id = draft_id.as_i64().unwrap_or_default();
            let row = conn
                .query_row(
                    "SELECT d.chapter_number,d.version,c.body FROM drafts d JOIN contents c ON c.id=d.content_id WHERE d.id=?",
                    [draft_id],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
            let reference = run
                .binding
                .source_refs
                .iter()
                .find(|source| source.source_id == format!("draft:{draft_id}"));
            Ok(match (row, reference) {
                (
                    Some((chapter_number, version, body)),
                    Some(reference),
                ) if version == reference.revision
                    && text_hash(&body) == reference.content_hash =>
                {
                    let mut entry = Map::new();
                    entry.insert("draftId".to_string(), json!(draft_id));
                    entry.insert("chapterNumber".to_string(), json!(chapter_number));
                    entry.insert("version".to_string(), json!(version));
                    entry.insert(
                        "contentHash".to_string(),
                        json!(reference.content_hash.clone()),
                    );
                    entry.insert("content".to_string(), json!(body));
                    if required_candidate_draft_ids.contains(&draft_id) {
                        entry.insert("required".to_string(), json!(true));
                    }
                    Value::Object(entry)
                }
                _ => Value::Null,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let attempt_rows = run_attempts(conn, &run.run_id)?;
    let last_composition_finish_reason = match composition
        .as_ref()
        .and_then(|item| item.artifact_ids.last().cloned())
    {
        Some(artifact_id) => {
            let receipt = repository
                .receipt(conn, &artifact_id, None)
                .map_err(|error| mutating_error(map_generation_error(&error)))?;
            receipt
                .result
                .as_ref()
                .and_then(|result| result.finish_reason.clone())
        }
        None => None,
    };
    let draft_short_outline = if material_decision
        .as_ref()
        .and_then(|decision| decision.get("shortOutlinePromptHash"))
        .and_then(Value::as_str)
        .is_some()
    {
        Some(short_outline_context(
            conn,
            repository,
            &run,
            &attempt_rows,
        )?)
    } else {
        None
    };
    let draft_reconciliation = if attempt_rows
        .iter()
        .any(|(_, _, usage)| attempt_purpose(usage) == DRAFT_RECONCILE_PURPOSE)
    {
        let artifact_ids: Vec<String> = attempt_rows
            .iter()
            .filter(|(_, _, usage)| attempt_purpose(usage) == DRAFT_RECONCILE_PURPOSE)
            .filter_map(|(attempt_id, _, _)| {
                repository
                    .receipt(conn, attempt_id, None)
                    .ok()
                    .and_then(|receipt| {
                        receipt
                            .artifact
                            .as_ref()
                            .map(|artifact| artifact.artifact_id.clone())
                    })
            })
            .collect();
        // 基线：首稿未发出，或首稿 usage 载 `reconciliationInjected`
        // 为 true 时，沿用最近一次完整结束的对账输出；否则 null。
        let first_draft_usage = attempt_rows
            .iter()
            .find(|(_, _, usage)| attempt_purpose(usage) == "chapter-draft")
            .map(|(_, _, usage)| usage.clone());
        let injected = first_draft_usage
            .as_ref()
            .and_then(|usage| {
                serde_json::from_str::<Value>(usage)
                    .ok()
                    .and_then(|usage| usage.get("reconciliationInjected").and_then(Value::as_bool))
            })
            .unwrap_or(false);
        let completed_output = match first_draft_usage {
            None => completed_reconciliation_output(conn, repository, &attempt_rows)?,
            Some(_) if injected => {
                completed_reconciliation_output(conn, repository, &attempt_rows)?
            }
            Some(_) => None,
        };
        Some(json!({
            "artifactIds": artifact_ids,
            "completedOutput": completed_output,
        }))
    } else {
        None
    };
    let plot_outline_recovery = match &plot_outline {
        Some(outline) => Some(plot_outline_recovery(
            conn, repository, &run, &view, outline, epoch,
        )?),
        None => None,
    };
    let blueprint_recovery = if manifest.get("operation").and_then(Value::as_str)
        == Some("chapter-blueprint-directory")
    {
        continuation.as_ref().map(|continuation| {
            // 基线 `viewOf(run).candidates?.at(-1)?.text ?? ''`。
            let draft = view
                .get("candidates")
                .and_then(Value::as_array)
                .and_then(|candidates| candidates.last())
                .and_then(|candidate| candidate.get("text"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let write_state = if blueprint_saved.is_some() {
                "author-saved".to_string()
            } else if continuation.state == "save-prefix" {
                "ready".to_string()
            } else {
                continuation.state.clone()
            };
            json!({
                "sourceHandle": handle_of(&run),
                "leaseEpoch": epoch,
                "draft": draft,
                "editRange": continuation.remaining_range,
                "saved": blueprint_saved,
                "writeState": write_state,
            })
        })
    } else {
        None
    };
    // 基线 `modelReceipt(binding).modelId`（:121-127）：
    // `modelReceipt` 缺键 / `modelId` 非字符串 / `modelRevision`
    // 非 64 位小写十六进制 → `GENERATION_MODEL_BINDING_INVALID`。
    let model_receipt = manifest
        .get("modelReceipt")
        .ok_or_else(|| mutating_error("GENERATION_MODEL_BINDING_INVALID"))?;
    let model_id = model_receipt
        .get("modelId")
        .and_then(Value::as_str)
        .ok_or_else(|| mutating_error("GENERATION_MODEL_BINDING_INVALID"))?;
    let model_revision = model_receipt
        .get("modelRevision")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if model_revision.len() != 64
        || !model_revision
            .chars()
            .all(|char| char.is_ascii_hexdigit() && !char.is_ascii_uppercase())
    {
        return Err(mutating_error("GENERATION_MODEL_BINDING_INVALID"));
    }
    let mut context = Map::new();
    context.insert("modelId".to_string(), json!(model_id));
    context.insert("handle".to_string(), handle_of(&run));
    context.insert(
        "operation".to_string(),
        json!(manifest
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or_default()),
    );
    context.insert(
        "chapterNumber".to_string(),
        manifest
            .get("chapterNumber")
            .and_then(Value::as_i64)
            .map(Value::from)
            .unwrap_or(Value::Null),
    );
    if let Some(outline) = &plot_outline {
        context.insert(
            "plotOutline".to_string(),
            serde_json::to_value(outline)
                .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?,
        );
        context.insert(
            "plotOutlineRecovery".to_string(),
            serde_json::to_value(
                plot_outline_recovery.expect("plot outline present ⇒ recovery computed"),
            )
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?,
        );
    }
    if let Some(continuation) = &continuation {
        context.insert(
            "planningContinuation".to_string(),
            json!({
                "sourceHandle": continuation.source_handle,
                "remainingRange": continuation.remaining_range,
                "targetUnits": continuation.target_units,
                "state": continuation.state,
                "nextHandle": continuation.next_handle,
            }),
        );
    }
    if let Some(blueprint_recovery) = &blueprint_recovery {
        context.insert("blueprintRecovery".to_string(), blueprint_recovery.clone());
    }
    let author_inputs = manifest
        .get("authorInputs")
        .cloned()
        .filter(|value| !value.is_null())
        .unwrap_or_else(|| json!([]));
    context.insert("authorInputs".to_string(), author_inputs);
    let selected_finalized_draft_ids = manifest
        .get("selectedFinalizedDraftIds")
        .cloned()
        .filter(|value| !value.is_null())
        .unwrap_or_else(|| json!([]));
    context.insert(
        "selectedFinalizedDraftIds".to_string(),
        selected_finalized_draft_ids,
    );
    let selected_blueprint_chapter_numbers = manifest
        .get("selectedBlueprintChapterNumbers")
        .cloned()
        .filter(|value| !value.is_null())
        .unwrap_or_else(|| json!([]));
    context.insert(
        "selectedBlueprintChapterNumbers".to_string(),
        selected_blueprint_chapter_numbers,
    );
    context.insert(
        "composition".to_string(),
        composition
            .as_ref()
            .map(|item| serde_json::to_value(item))
            .transpose()
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?
            .unwrap_or(Value::Null),
    );
    context.insert(
        "lastCompositionFinishReason".to_string(),
        json!(last_composition_finish_reason),
    );
    context.insert(
        "attemptedPurposes".to_string(),
        json!(attempt_rows
            .iter()
            .map(|(_, _, usage)| attempt_purpose(usage))
            .collect::<Vec<_>>()),
    );
    context.insert("draftSave".to_string(), draft_save);
    if let Some(batch_id) = manifest.get("batchId").and_then(Value::as_str) {
        context.insert("batchId".to_string(), json!(batch_id));
    }
    if let Some(knowledge_snapshot) = manifest.get("knowledgeSnapshot") {
        context.insert("knowledgeSnapshot".to_string(), knowledge_snapshot.clone());
    }
    if let Some(draft_reconciliation) = &draft_reconciliation {
        context.insert(
            "draftReconciliation".to_string(),
            draft_reconciliation.clone(),
        );
    }
    if let Some(draft_short_outline) = &draft_short_outline {
        context.insert(
            "draftShortOutline".to_string(),
            serde_json::to_value(draft_short_outline)
                .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?,
        );
    }
    if selected_drafts.iter().all(|item| !item.is_null()) {
        context.insert("selectedDrafts".to_string(), json!(selected_drafts));
    }
    Ok(Value::Object(context))
}

/// 基线 `planningContinuation(run)`（main-generation-owner.ts:700-740）：
/// 规划续作状态机——`continued`（已登记续作）→ `in-flight`
/// （生成在途）→ 源现状探针（`source-changed` / 后续分支）
/// → `complete`（无剩余范围）→ `save-prefix`（大纲已含可续
/// 前缀）→ `ready`。
fn planning_continuation(
    conn: &Connection,
    repository: &crate::repositories::generation_run_repository::GenerationRunRepository,
    run: &crate::repositories::generation_run_repository::DurableGenerationRun,
) -> Result<Option<PlanningContinuationProjection>, String> {
    if repository
        .is_private_planning_history(conn, &run.run_id)
        .map_err(|error| mutating_error(map_generation_error(&error)))?
    {
        return Ok(None);
    }
    let outline = repository
        .read_plot_outline(conn, &run.run_id, None)
        .map_err(|error| mutating_error(map_generation_error(&error)))?;
    let inputs: Vec<Value> = run
        .binding
        .source_manifest
        .get("authorInputs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let directory_range = if run
        .binding
        .source_manifest
        .get("operation")
        .and_then(Value::as_str)
        == Some("chapter-blueprint-directory")
    {
        inputs
            .iter()
            .find(|input| {
                input.get("id").and_then(Value::as_str) == Some("directory:requested-range")
            })
            .cloned()
    } else {
        None
    };
    if outline.is_none() && directory_range.is_none() {
        return Ok(None);
    }
    let scope = directory_range
        .as_ref()
        .map(|input| {
            // 基线 `JSON.parse(directoryRange.text)`。
            serde_json::from_str::<Value>(
                input
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
            .map_err(|_| mutating_error("GENERATION_REQUEST_FAILED"))
        })
        .transpose()?;
    let saved = if outline.is_some() {
        repository
            .read_plot_outline_author_edit(conn, &run.run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?
    } else {
        None
    };
    let blueprint_saved = if scope.is_some() {
        repository
            .read_blueprint_author_edit(conn, &run.run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?
    } else {
        None
    };
    let progress = if scope.is_some() {
        repository
            .list_directory_progress(conn)
            .map_err(|error| mutating_error(map_generation_error(&error)))?
            .into_iter()
            .find(|item| item.source_handle.run_id == run.run_id)
    } else {
        None
    };
    let remaining_range = if let Some(outline) = &outline {
        saved
            .as_ref()
            .and_then(|item| item.remaining_range)
            .or(Some(outline.range))
    } else if let Some(blueprint_saved) = &blueprint_saved {
        blueprint_saved.remaining_range
    } else if let Some(progress) = &progress {
        progress
            .remaining_range
            .as_ref()
            .map(|range| PlotOutlineRange {
                from: range.start_chapter,
                to: range.end_chapter,
            })
    } else {
        let scope = scope.expect("scope is defined in this branch");
        Some(PlotOutlineRange {
            from: scope
                .get("startChapter")
                .and_then(Value::as_i64)
                .ok_or_else(|| mutating_error("GENERATION_REQUEST_FAILED"))?,
            to: scope
                .get("endChapter")
                .and_then(Value::as_i64)
                .ok_or_else(|| mutating_error("GENERATION_REQUEST_FAILED"))?,
        })
    };
    let target = inputs
        .iter()
        .find(|input| input.get("id").and_then(Value::as_str) == Some("planning:target-units"));
    let target_units = parse_planning_target_units(
        target
            .and_then(|input| input.get("text"))
            .and_then(Value::as_str),
    )
    .map_err(|code| mutating_error(&code))?;
    let source_handle = handle_of(run);
    let continued = repository
        .read_planning_continuation(conn, &run.run_id)
        .map_err(|error| mutating_error(map_generation_error(&error)))?;
    if let Some(continued) = continued {
        return Ok(Some(PlanningContinuationProjection {
            source_handle,
            remaining_range,
            target_units,
            state: "continued".to_string(),
            next_handle: Some(json!({
                "projectId": continued.next_handle.project_id,
                "epoch": continued.next_handle.epoch,
                "rootActionId": continued.next_handle.root_action_id,
                "runId": continued.next_handle.run_id,
            })),
        }));
    }
    if project_generation_in_flight(conn)
        .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?
    {
        return Ok(Some(PlanningContinuationProjection {
            source_handle,
            remaining_range,
            target_units,
            state: "in-flight".to_string(),
            next_handle: None,
        }));
    }
    // 基线源现状探针（:722-735）：以 `rebuildBinding` +
    // `compareGenerationSourceBindings` 重构绑定并逐字段
    // 比对。`rebuildBinding` / `loadModel` 依赖尚未迁移
    // （见本文件头注「刻意偏离」），按 review_revision 批次
    // 既定范式以保守占位（判定为一致）注入——可计算的
    // 分支（`continued` / `in-flight` / `complete` /
    // `save-prefix`）不受影响。
    let current = true;
    if !current {
        return Ok(Some(PlanningContinuationProjection {
            source_handle,
            remaining_range,
            target_units,
            state: "source-changed".to_string(),
            next_handle: None,
        }));
    }
    let Some(remaining_range) = remaining_range else {
        return Ok(Some(PlanningContinuationProjection {
            source_handle,
            remaining_range: None,
            target_units,
            state: "complete".to_string(),
            next_handle: None,
        }));
    };
    if outline.is_some()
        && saved.is_none()
        && (outline
            .as_ref()
            .and_then(|item| item.composition.as_ref())
            .and_then(|composition| composition.chapters.as_ref())
            .is_some_and(|chapters| !chapters.is_empty())
            || matches!(
                outline.as_ref().unwrap().cursor,
                PlotOutlineCursor::Accept { .. }
            ))
    {
        return Ok(Some(PlanningContinuationProjection {
            source_handle,
            remaining_range: Some(remaining_range),
            target_units,
            state: "save-prefix".to_string(),
            next_handle: None,
        }));
    }
    Ok(Some(PlanningContinuationProjection {
        source_handle,
        remaining_range: Some(remaining_range),
        target_units,
        state: "ready".to_string(),
        next_handle: None,
    }))
}

/// 基线 `plotOutlineRecovery(run, outline)`
/// （main-generation-owner.ts:741-766）：大纲恢复提案——
/// 失败候选文本 / 作者已存大纲 / 源现状判别。
fn plot_outline_recovery(
    conn: &Connection,
    repository: &crate::repositories::generation_run_repository::GenerationRunRepository,
    run: &crate::repositories::generation_run_repository::DurableGenerationRun,
    view: &Value,
    outline: &PlotOutlineProgress,
    epoch: &str,
) -> Result<PlotOutlineRecoveryProjection, String> {
    let saved = repository
        .read_plot_outline_author_edit(conn, &run.run_id)
        .map_err(|error| mutating_error(map_generation_error(&error)))?;
    // 基线 `currentAttempts`：`request` / `stopped` 光标下，
    // 本章大纲尝试的 attemptId 集合。
    let current_attempts: HashSet<String> = match outline.cursor {
        PlotOutlineCursor::Request { chapter_number, .. }
        | PlotOutlineCursor::Stopped { chapter_number, .. } => run_attempts(conn, &run.run_id)?
            .into_iter()
            .filter(|(_, _, usage)| {
                attempt_purpose(usage)
                    .starts_with(&format!("plot-outline:chapter:{chapter_number}:"))
            })
            .map(|(attempt_id, _, _)| attempt_id)
            .collect(),
        _ => HashSet::new(),
    };
    // 基线 `[...(viewOf(run).candidates ?? [])].reverse()
    // .find(candidate => currentAttempts.has(candidate.attemptId))`。
    let latest = view
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| {
            candidates.iter().rev().find(|candidate| {
                candidate
                    .get("attemptId")
                    .and_then(Value::as_str)
                    .is_some_and(|attempt_id| current_attempts.contains(attempt_id))
            })
        })
        .and_then(|candidate| candidate.get("text"))
        .and_then(Value::as_str);
    let mut draft =
        plot_outline_recovery_draft(outline, latest).map_err(|code| mutating_error(&code))?;
    // 基线 `writeState` 初值 `{ kind: 'blocked', reason:
    // 'source-changed' }`；基线在 try 内探测，作者已存大纲
    // 命中 → `author-saved`，源一致且无续作登记 →
    // `in-flight` / `ready`。基线 catch 吞掉探针异常
    // （「Frozen proposals remain readable」）。
    let mut write_state = json!({ "kind": "blocked", "reason": "source-changed" });
    if let Some(saved) = &saved {
        // 基线可计算前缀：当前摘要存在且哈希与作者存档一致
        // （`plotOutlineTargetCurrent` 的剩余比对依赖
        // `rebuildBinding`，见本文件头注「刻意偏离」，按
        // 保守占位判定为一致）。
        let synopsis = project_core::get(conn)
            .map_err(|error| mutating_error(map_generation_error(&error)))?
            .map(|core| core.synopsis);
        if let Some(synopsis) = synopsis {
            if text_hash(&synopsis) == saved.synopsis_hash {
                draft = synopsis;
                write_state = json!({ "kind": "blocked", "reason": "author-saved" });
            }
        }
    } else if repository
        .read_planning_continuation(conn, &run.run_id)
        .map_err(|error| mutating_error(map_generation_error(&error)))?
        .is_none()
    {
        // 基线 `compareGenerationSourceBindings` 探针同上以
        // 保守占位（判定为一致）注入。
        write_state = if project_generation_in_flight(conn)
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?
        {
            json!({ "kind": "blocked", "reason": "in-flight" })
        } else {
            json!({ "kind": "ready" })
        };
    }
    let mut complete_chapters: Vec<i64> = outline
        .composition
        .as_ref()
        .and_then(|composition| composition.chapters.as_ref())
        .map(|chapters| {
            chapters
                .iter()
                .map(|chapter| chapter.chapter_number)
                .collect()
        })
        .unwrap_or_default();
    if let PlotOutlineCursor::Accept { chapter_number, .. } = outline.cursor {
        complete_chapters.push(chapter_number);
    }
    Ok(PlotOutlineRecoveryProjection {
        source_handle: handle_of(run),
        lease_epoch: epoch.to_string(),
        draft,
        complete_chapters,
        write_state,
        saved: serde_json::to_value(&saved)
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?,
    })
}

/// 基线 `shortOutlineContext(run)`（main-generation-owner.ts:208-227）：
/// 短大纲（自动大纲）恢复上下文——制品 ID / 已完成输出 /
/// 提示哈希 / 重试可用性 / 初始草稿任务。
fn short_outline_context(
    conn: &Connection,
    repository: &crate::repositories::generation_run_repository::GenerationRunRepository,
    run: &crate::repositories::generation_run_repository::DurableGenerationRun,
    attempt_rows: &[(String, String, String)],
) -> Result<ShortOutlineContextProjection, String> {
    let prompt_hash = run
        .binding
        .source_manifest
        .get("materialDecision")
        .and_then(|decision| decision.get("shortOutlinePromptHash"))
        .and_then(Value::as_str)
        .ok_or_else(|| mutating_error("GENERATION_REQUEST_FAILED"))?
        .to_string();
    // 基线 `attempts`（过滤取消前派发）。
    let attempts: Vec<&(String, String, String)> = attempt_rows
        .iter()
        .filter(|(_, attempt_json, _)| {
            attempt_status(attempt_json).as_deref() != Some("cancelled-before-dispatch")
        })
        .collect();
    let outline_attempts: Vec<&&(String, String, String)> = attempts
        .iter()
        .filter(|(_, _, usage)| attempt_purpose(usage) == DRAFT_SHORT_OUTLINE_PURPOSE)
        .collect();
    let receipts: Vec<_> = outline_attempts
        .iter()
        .map(|(attempt_id, _, _)| {
            repository
                .receipt(conn, attempt_id, None)
                .map_err(|error| mutating_error(map_generation_error(&error)))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let completed = receipts.iter().find(|receipt| {
        receipt.failure_code.is_none()
            && receipt
                .result
                .as_ref()
                .and_then(|result| result.finish_reason.as_deref())
                == Some("stop")
            && receipt
                .artifact
                .as_ref()
                .is_some_and(|artifact| !artifact.text.trim().is_empty())
    });
    let first_draft = attempts
        .iter()
        .find(|(_, _, usage)| attempt_purpose(usage) == "chapter-draft");
    let initial_draft_task = match first_draft {
        Some((attempt_id, _, _)) => {
            let usage = conn
                .query_row(
                    "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id=?",
                    [attempt_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
            usage.and_then(|usage| {
                serde_json::from_str::<Value>(&usage)
                    .ok()
                    .and_then(|usage| usage.get("replayTask").cloned())
            })
        }
        None => None,
    };
    let artifact_ids: Vec<String> = receipts
        .iter()
        .filter_map(|receipt| {
            receipt
                .artifact
                .as_ref()
                .map(|artifact| artifact.artifact_id.clone())
        })
        .collect();
    let completed_output = completed.and_then(|receipt| {
        receipt
            .artifact
            .as_ref()
            .map(|artifact| artifact.text.clone())
    });
    Ok(ShortOutlineContextProjection {
        artifact_ids,
        completed_output,
        prompt_hash,
        retry: short_outline_retry(conn, repository, run, attempt_rows)?,
        initial_draft_task,
    })
}

/// 基线 `shortOutlineRetry(run)`（main-generation-owner.ts:201-207）。
fn short_outline_retry(
    conn: &Connection,
    repository: &crate::repositories::generation_run_repository::GenerationRunRepository,
    run: &crate::repositories::generation_run_repository::DurableGenerationRun,
    attempt_rows: &[(String, String, String)],
) -> Result<Value, String> {
    let failed_attempt_id = attempt_rows
        .first()
        .map(|(attempt_id, _, _)| attempt_id.clone());
    if let Some(failed_attempt_id) = failed_attempt_id {
        return Ok(
            match assert_short_outline_retry_available(
                conn,
                repository,
                run,
                attempt_rows,
                &failed_attempt_id,
            ) {
                Ok(()) => json!({
                    "kind": "available",
                    "failedAttemptId": failed_attempt_id,
                }),
                Err(_) => json!({ "kind": "unavailable" }),
            },
        );
    }
    Ok(json!({ "kind": "unavailable" }))
}

/// 基线 `assertShortOutlineRetryAvailable(run, failedAttemptId)`
/// （main-generation-owner.ts:187-199）：重试准入——任务可
/// 复原 + 运行 / 账本状态允许再次物理请求。
fn assert_short_outline_retry_available(
    conn: &Connection,
    repository: &crate::repositories::generation_run_repository::GenerationRunRepository,
    run: &crate::repositories::generation_run_repository::DurableGenerationRun,
    attempt_rows: &[(String, String, String)],
    failed_attempt_id: &str,
) -> Result<(), String> {
    read_failed_short_outline_task(conn, repository, run, attempt_rows, failed_attempt_id)?;
    let budget = repository
        .budget(conn, &run.root_action_id)
        .map_err(|error| mutating_error(map_generation_error(&error)))?;
    if attempt_rows.len() != 1
        || repository
            .read_visible_composition(conn, &run.run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?
            .is_some()
        || GenerationDraftEffects
            .read_commit(conn, repository, &run.run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?
            .is_some()
        || !matches!(run.status.as_str(), "running" | "paused" | "failed")
        || !matches!(budget.root.status.as_str(), "active" | "paused")
        || budget.blocked_code.is_some()
        || budget
            .attempts
            .iter()
            .any(|attempt| matches!(attempt.status.as_str(), "reserved" | "dispatch-marked"))
        || budget
            .attempts
            .iter()
            .filter(|attempt| attempt.status != "cancelled-before-dispatch")
            .count() as i64
            >= budget.policy.max_physical_requests
        || budget.attempts.iter().map(token_liability).sum::<i64>()
            >= budget.policy.max_token_liability
        || budget.active_elapsed_ms >= budget.policy.max_active_elapsed_ms
    {
        return Err(mutating_error(
            "GENERATION_DRAFT_SHORT_OUTLINE_RETRY_UNAVAILABLE",
        ));
    }
    Ok(())
}

/// 基线 `readFailedShortOutlineTask(run, failedAttemptId)`
/// （main-generation-owner.ts:167-185）：复原失败的短大纲
/// 任务——校验任务身份 / 结算状态 / 提示哈希绑定。
fn read_failed_short_outline_task(
    conn: &Connection,
    repository: &crate::repositories::generation_run_repository::GenerationRunRepository,
    run: &crate::repositories::generation_run_repository::DurableGenerationRun,
    attempt_rows: &[(String, String, String)],
    failed_attempt_id: &str,
) -> Result<Value, String> {
    let decision = run
        .binding
        .source_manifest
        .get("materialDecision")
        .cloned()
        .unwrap_or(Value::Null);
    let prompt_hash = decision
        .get("shortOutlinePromptHash")
        .and_then(Value::as_str);
    let first = attempt_rows.first();
    if run
        .binding
        .source_manifest
        .get("operation")
        .and_then(Value::as_str)
        != Some("chapter-draft")
        || prompt_hash.is_none()
        || first.is_none()
        || first.map(|(attempt_id, _, _)| attempt_id.as_str()) != Some(failed_attempt_id)
        || first.map(|(_, _, usage)| attempt_purpose(usage)).as_deref()
            != Some(DRAFT_SHORT_OUTLINE_PURPOSE)
    {
        return Err(mutating_error(
            "GENERATION_DRAFT_SHORT_OUTLINE_RETRY_UNAVAILABLE",
        ));
    }
    let prompt_hash = prompt_hash.expect("prompt hash is defined");
    let receipt = repository
        .receipt(conn, failed_attempt_id, None)
        .map_err(|error| mutating_error(map_generation_error(&error)))?;
    if receipt.attempt.status != "settled"
        || receipt.failure_code.is_some()
        || !receipt.artifact.is_some()
        || !(receipt
            .result
            .as_ref()
            .and_then(|result| result.finish_reason.as_deref())
            == Some("length")
            || receipt
                .result
                .as_ref()
                .and_then(|result| result.finish_reason.as_deref())
                == Some("stop")
                && receipt
                    .artifact
                    .as_ref()
                    .map(|artifact| artifact.text.trim().is_empty())
                    .unwrap_or(true))
    {
        return Err(mutating_error(
            "GENERATION_DRAFT_SHORT_OUTLINE_RETRY_UNAVAILABLE",
        ));
    }
    let usage: Value = serde_json::from_str(
        &conn
            .query_row(
                "SELECT usage_receipt_json FROM generation_attempts WHERE attempt_id=?",
                [failed_attempt_id],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?,
    )
    .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
    let Some(task) = usage.get("replayTask").cloned() else {
        return Err(mutating_error(
            "GENERATION_DRAFT_SHORT_OUTLINE_TASK_INVALID",
        ));
    };
    // 基线 `assertSemanticGenerationTask(task)`：任务结构校验
    // ——用途 / 输出 / 消息角色与正文字段齐备。
    let task_purpose = task
        .get("purpose")
        .and_then(Value::as_str)
        .ok_or_else(|| mutating_error("GENERATION_DRAFT_SHORT_OUTLINE_TASK_INVALID"))?;
    let task_output = task
        .get("output")
        .and_then(Value::as_str)
        .ok_or_else(|| mutating_error("GENERATION_DRAFT_SHORT_OUTLINE_TASK_INVALID"))?;
    let prompts: Vec<&Value> = task
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| mutating_error("GENERATION_DRAFT_SHORT_OUTLINE_TASK_INVALID"))?
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .collect();
    if task_purpose != DRAFT_SHORT_OUTLINE_PURPOSE
        || task_output != "visible-text"
        || prompts.len() != 1
        || prompts[0]
            .get("content")
            .and_then(Value::as_str)
            .map(|content| text_hash(content))
            .as_deref()
            != Some(prompt_hash)
    {
        return Err(mutating_error(
            "GENERATION_DRAFT_SHORT_OUTLINE_TASK_INVALID",
        ));
    }
    // 基线末段 `requestHash` 重验与
    // `compareGenerationSourceBindings` 探针：前者以
    // JS 对象插入序序列化任务（`serde_json` 默认按键排序，
    // 无法逐字节复现），后者依赖尚未迁移的
    // `rebuildBinding`（见本文件头注「刻意偏离」）——
    // 均按保守占位（判定为一致）注入；任务的用途 / 输出 /
    // 提示哈希绑定已在上方精确校验。
    Ok(task)
}

/// 基线 `completedReconciliationOutput(runId)`
/// （main-generation-owner.ts:201-207）：最近一次完整结束
/// （stop、无失败码）的对账原始输出。
fn completed_reconciliation_output(
    conn: &Connection,
    repository: &crate::repositories::generation_run_repository::GenerationRunRepository,
    attempt_rows: &[(String, String, String)],
) -> Result<Option<String>, String> {
    for (attempt_id, _, usage) in attempt_rows.iter().rev() {
        if attempt_purpose(usage) != DRAFT_RECONCILE_PURPOSE {
            continue;
        }
        let receipt = repository
            .receipt(conn, attempt_id, None)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        if receipt.failure_code.is_none()
            && receipt
                .result
                .as_ref()
                .and_then(|result| result.finish_reason.as_deref())
                == Some("stop")
            && receipt
                .artifact
                .as_ref()
                .is_some_and(|artifact| !artifact.text.trim().is_empty())
        {
            return Ok(receipt.artifact.map(|artifact| artifact.text));
        }
    }
    Ok(None)
}

/// 基线 `runAttempts(runId)`（main-generation-owner.ts:145）：
/// 运行的 attempt 行（`attemptId` / 派发状态 / 用途）。
fn run_attempts(conn: &Connection, run_id: &str) -> Result<Vec<(String, String, String)>, String> {
    let mut statement = conn
        .prepare(
            "SELECT attempt_id,attempt_json,usage_receipt_json FROM generation_attempts WHERE run_id=? ORDER BY rowid",
        )
        .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
    let attempts = statement
        .query_map([run_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| mutating_error(map_generation_error(&error.to_string())));
    attempts
}

/// attempt usage 载荷的 `purpose`（基线
/// `String(JSON.parse(row.usage_receipt_json).purpose ?? 'unknown')`）。
fn attempt_purpose(usage: &str) -> String {
    serde_json::from_str::<Value>(usage)
        .ok()
        .as_ref()
        .and_then(|usage| usage.get("purpose"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

/// attempt 载荷（`attempt_json`）的 `status`（基线
/// `runAttempts` 的 `status` 投影）。
fn attempt_status(usage: &str) -> Option<String> {
    serde_json::from_str::<Value>(usage)
        .ok()
        .as_ref()
        .and_then(|usage| usage.get("status"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// 基线 `projectGenerationInFlight()`（main-generation-owner.ts:698）：
/// 派发在途（`reserved` / `dispatch-marked`）。基线另有
/// `pending.size > 0`（执行面在途映射）——执行面尚未迁移，
/// 恒空，此处以 SQL 判别等价。
fn project_generation_in_flight(conn: &Connection) -> Result<bool, String> {
    conn.query_row(
        "SELECT 1 FROM generation_attempts WHERE json_extract(attempt_json,'$.status') IN ('reserved','dispatch-marked') LIMIT 1",
        [],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map(|row| row.is_some())
    .map_err(|error| mutating_error(map_generation_error(&error.to_string())))
}

/// 基线 `tokenLiability`（src/shared/generation-contract.ts:66-68，
/// Rust 侧另有 `services/main_generation_owner.rs` 的同型私有 fn）。
fn token_liability(attempt: &PhysicalAttempt) -> i64 {
    if attempt.status == "cancelled-before-dispatch" {
        0
    } else if attempt.status == "settled" && attempt.actual_tokens.is_some() {
        attempt.actual_tokens.unwrap_or(0)
    } else {
        attempt.reserved_tokens
    }
}

/// 基线 `candidate:\d+` 候选源 ID 提取
/// （`materialDecision.included[].sourceId`）。
fn candidate_draft_id(source_id: &str) -> Option<i64> {
    let digits = source_id.strip_prefix("candidate:")?;
    if digits.is_empty() || !digits.chars().all(|char| char.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i64>().ok()
}

/// 基线 `parsePlanningTargetUnits`（src/shared/plot-outline-contract.ts:12）。
fn parse_planning_target_units(value: Option<&str>) -> Result<i64, String> {
    let Some(text) = value else {
        return Ok(DEFAULT_PLANNING_TARGET_UNITS);
    };
    // 基线 `Number(text)`：非数值 / 非安全整数 / 越界 → 抛错。
    let integer = text
        .trim()
        .parse::<i64>()
        .map_err(|_| GENERATION_PLANNING_TARGET_INVALID.to_string())?;
    if integer < 1 || integer > PLANNING_TARGET_UNITS_LIMIT {
        return Err(GENERATION_PLANNING_TARGET_INVALID.to_string());
    }
    Ok(integer)
}

/// 基线 `handleOf(run)`（main-generation-owner.ts:125-126）。
fn handle_of(run: &crate::repositories::generation_run_repository::DurableGenerationRun) -> Value {
    json!({
        "projectId": run.binding.project_id,
        "epoch": run.binding.epoch,
        "rootActionId": run.root_action_id,
        "runId": run.run_id,
    })
}

/// 基线 `textHash`（SHA-256 十六进制小写）。
fn text_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

// ============================================================================
// plot-outline 纯函数平移（src/shared/plot-outline-contract.ts；
// `repositories/generation_run_repository.rs` 的同型私有 fn
// 未公开，本文件按基线逐字移植）
// ============================================================================

const CHINESE_DIGITS: &str = "零〇一二两三四五六七八九";

#[derive(Debug, Clone, PartialEq)]
struct HeadingMatch {
    from: i64,
    to: i64,
    index: usize,
    end: usize,
    grouped: bool,
}

fn chinese_digit(char: char) -> Option<i64> {
    match char {
        '零' | '〇' => Some(0),
        '一' => Some(1),
        '二' | '两' => Some(2),
        '三' => Some(3),
        '四' => Some(4),
        '五' => Some(5),
        '六' => Some(6),
        '七' => Some(7),
        '八' => Some(8),
        '九' => Some(9),
        _ => None,
    }
}

fn chinese_unit(char: char) -> Option<i64> {
    match char {
        '十' => Some(10),
        '百' => Some(100),
        '千' => Some(1000),
        _ => None,
    }
}

fn is_chinese_number_char(char: char) -> bool {
    char.is_ascii_digit() || CHINESE_DIGITS.contains(char) || matches!(char, '十' | '百' | '千')
}

fn chapter_number(token: &str) -> i64 {
    if !token.is_empty() && token.chars().all(|char| char.is_ascii_digit()) {
        return token.parse::<i64>().unwrap_or_default();
    }
    let mut result = 0;
    let mut digit = 0;
    for char in token.chars() {
        if let Some(value) = chinese_digit(char) {
            digit = value;
        } else if let Some(unit) = chinese_unit(char) {
            result += digit.max(1) * unit;
            digit = 0;
        }
    }
    result + digit
}

fn match_chinese_number(chars: &[char], start: usize) -> Option<(i64, usize)> {
    if start >= chars.len() || !is_chinese_number_char(chars[start]) {
        return None;
    }
    let mut end = start;
    while end < chars.len() && is_chinese_number_char(chars[end]) {
        end += 1;
    }
    let token: String = chars[start..end].iter().collect();
    Some((chapter_number(&token), end))
}

fn match_ascii_digits(chars: &[char], start: usize) -> Option<(i64, usize)> {
    if start >= chars.len() || !chars[start].is_ascii_digit() {
        return None;
    }
    let mut end = start;
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
    }
    let token: String = chars[start..end].iter().collect();
    token.parse::<i64>().ok().map(|value| (value, end))
}

/// 中文支 `第 NUMBER([–—-]NUMBER)?章`；返回 `(from, to, 是否有第二数字)`。
fn match_chinese_chapter(chars: &[char], start: usize) -> Option<(i64, i64, bool)> {
    let (from, mut position) = match_chinese_number(chars, start + 1)?;
    let mut to = from;
    let mut grouped = false;
    if position < chars.len()
        && (chars[position] == '–' || chars[position] == '—' || chars[position] == '-')
    {
        if let Some((second, end)) = match_chinese_number(chars, position + 1) {
            to = second;
            grouped = true;
            position = end;
        }
    }
    if position >= chars.len() || chars[position] != '章' {
        return None;
    }
    position += 1;
    let mut lookahead = position;
    while lookahead < chars.len() && (chars[lookahead] == '\t' || chars[lookahead] == ' ') {
        lookahead += 1;
    }
    if !(lookahead == chars.len()
        || matches!(chars[lookahead], ':' | '：' | '.' | '．' | '—' | '-'))
    {
        return None;
    }
    Some((from, to, grouped))
}

/// 阿拉伯支 `Chapters?[\t ]+(\d+)([\t ]*[–—-][\t ]*(\d+))?`。
fn match_english_chapter(chars: &[char], start: usize) -> Option<(i64, i64, bool)> {
    if start + 7 > chars.len()
        || !chars[start..start + 7]
            .iter()
            .collect::<String>()
            .eq_ignore_ascii_case("chapter")
    {
        return None;
    }
    let mut position = start + 7;
    if position < chars.len() && (chars[position] == 's' || chars[position] == 'S') {
        position += 1;
    }
    let whitespace_start = position;
    while position < chars.len() && (chars[position] == '\t' || chars[position] == ' ') {
        position += 1;
    }
    if position == whitespace_start {
        return None;
    }
    let (from, mut end) = match_ascii_digits(chars, position)?;
    let mut to = from;
    let mut grouped = false;
    let mut range = end;
    while range < chars.len() && (chars[range] == '\t' || chars[range] == ' ') {
        range += 1;
    }
    if range < chars.len() && (chars[range] == '–' || chars[range] == '—' || chars[range] == '-')
    {
        let mut dash_end = range + 1;
        while dash_end < chars.len() && (chars[dash_end] == '\t' || chars[dash_end] == ' ') {
            dash_end += 1;
        }
        if let Some((second, second_end)) = match_ascii_digits(chars, dash_end) {
            to = second;
            grouped = true;
            end = second_end;
        }
    }
    let mut lookahead = end;
    while lookahead < chars.len() && (chars[lookahead] == '\t' || chars[lookahead] == ' ') {
        lookahead += 1;
    }
    if !(lookahead == chars.len()
        || matches!(chars[lookahead], ':' | '：' | '.' | '．' | '—' | '-'))
    {
        return None;
    }
    Some((from, to, grouped))
}

/// `/Chapters\s/iu`：匹配文本含大小写不敏感 "chapters" 紧接空白字符。
fn contains_chapters_followed_by_whitespace(text: &str) -> bool {
    let lower: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let needle: Vec<char> = "chapters".chars().collect();
    if lower.len() < needle.len() {
        return false;
    }
    for start in 0..=(lower.len() - needle.len()) {
        if lower[start..start + needle.len()] == needle[..] {
            let after = start + needle.len();
            if after < lower.len() && lower[after].is_whitespace() {
                return true;
            }
        }
    }
    false
}

/// 逐行匹配 JS 正则
/// `^[\t ]*(?:#{1,6}[\t ]+|[-*+][\t ]+)?(?:第(NUMBER)(?:[–—-](NUMBER))?章|Chapters?[\t ]+(\d+)(?:[\t ]*[–—-][\t ]*(\d+))?)(?=[\t ]*(?:[:：.．—-]|$))[^\r\n]*`
/// （`regex` crate 不支持前瞻断言，手工实现等价算法，含可选组回溯语义）。
fn match_heading_line(line: &str) -> Option<(i64, i64, bool)> {
    let chars: Vec<char> = line.chars().collect();
    let mut position = 0;
    while position < chars.len() && (chars[position] == '\t' || chars[position] == ' ') {
        position += 1;
    }
    // 可选 markdown 列表前缀 #{1,6}[\t ]+ 或 [-*+][\t ]+；不构成时整组回退为空。
    if position < chars.len() && chars[position] == '#' {
        let mut hashes = position;
        while hashes < chars.len() && chars[hashes] == '#' && hashes - position < 6 {
            hashes += 1;
        }
        if hashes - position >= 1
            && hashes < chars.len()
            && (chars[hashes] == '\t' || chars[hashes] == ' ')
        {
            while hashes < chars.len() && (chars[hashes] == '\t' || chars[hashes] == ' ') {
                hashes += 1;
            }
            position = hashes;
        }
    } else if position < chars.len()
        && (chars[position] == '-' || chars[position] == '*' || chars[position] == '+')
    {
        let marker = position + 1;
        if marker < chars.len() && (chars[marker] == '\t' || chars[marker] == ' ') {
            let mut marker = marker;
            while marker < chars.len() && (chars[marker] == '\t' || chars[marker] == ' ') {
                marker += 1;
            }
            position = marker;
        }
    }
    if position < chars.len() && chars[position] == '第' {
        if let Some(result) = match_chinese_chapter(&chars, position) {
            return Some(result);
        }
        return None;
    }
    match_english_chapter(&chars, position)
}

fn headings(text: &str) -> Vec<HeadingMatch> {
    let mut result = Vec::new();
    let mut byte_offset = 0;
    for line in text.split('\n') {
        let line_len = line.len();
        let match_len = if line.ends_with('\r') {
            line_len - 1
        } else {
            line_len
        };
        if let Some((from, to, grouped)) = match_heading_line(line) {
            let match_text = &line[..match_len];
            result.push(HeadingMatch {
                from,
                to,
                index: byte_offset,
                end: byte_offset + match_len,
                grouped: grouped || contains_chapters_followed_by_whitespace(match_text),
            });
        }
        byte_offset += line_len + 1;
    }
    result
}

/// 基线 `joinPlotOutlineEntries`（plot-outline-contract.ts:177）。
fn join_plot_outline_entries(prefix: &str, next: &str) -> String {
    [prefix.trim(), next.trim()]
        .iter()
        .filter(|entry| !entry.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 基线 `renderPlotOutlineSynopsis`（plot-outline-contract.ts:187）。
fn render_plot_outline_synopsis(
    body: &str,
    covered_to: i64,
    expected: &ProjectCoreSynopsisExpected,
) -> String {
    let english = expected.writing_language == "en-US";
    let tail = if covered_to < expected.total_chapters {
        if english {
            format!(
                "> This outline covers chapters 1-{covered_to} of {}; the remaining chapters will be generated in later batches.",
                expected.total_chapters
            )
        } else {
            format!(
                "> 本大纲已覆盖至第 {} 章（全书 {} 章），其余章节将在后续批次继续生成。",
                covered_to, expected.total_chapters
            )
        }
    } else {
        String::new()
    };
    let title = if english {
        "Plot Outline"
    } else {
        "情节大纲"
    };
    format!("# {title}\n\n{}", join_plot_outline_entries(body, &tail))
        .trim()
        .to_string()
}

/// 基线 `plotOutlineConfirmedPrefix`（plot-outline-contract.ts）——
/// 已确认前缀的一致性校验与提取。
fn plot_outline_confirmed_prefix(
    expected: &ProjectCoreSynopsisExpected,
    range: &PlotOutlineRange,
) -> Result<String, String> {
    let entries = headings(&expected.synopsis);
    if entries.iter().any(|entry| {
        (entry.from < range.from && entry.to >= range.from)
            || (entry.from <= range.to && entry.to > range.to)
    }) {
        return Err(GENERATION_PLOT_OUTLINE_PREFIX_INVALID.to_string());
    }
    if range.from == 1 {
        return Ok(String::new());
    }
    let before: Vec<&HeadingMatch> = entries
        .iter()
        .filter(|entry| entry.to < range.from)
        .collect();
    let mut next = 1;
    for entry in &before {
        if entry.from != next || entry.to < entry.from {
            return Err(GENERATION_PLOT_OUTLINE_PREFIX_INVALID.to_string());
        }
        next = entry.to + 1;
    }
    if next != range.from || before.is_empty() {
        return Err(GENERATION_PLOT_OUTLINE_PREFIX_INVALID.to_string());
    }
    if let Some(following) = entries.iter().find(|entry| entry.from >= range.from) {
        return Ok(expected.synopsis[before[0].index..following.index]
            .trim()
            .to_string());
    }
    let title = format!(
        "# {}\n\n",
        if expected.writing_language == "en-US" {
            "Plot Outline"
        } else {
            "情节大纲"
        }
    );
    let suffix =
        render_plot_outline_synopsis("", range.from - 1, expected)[title.len()..].to_string();
    if !expected.synopsis.starts_with(&title)
        || suffix.is_empty()
        || !expected.synopsis.ends_with(&suffix)
    {
        return Err(GENERATION_PLOT_OUTLINE_PREFIX_INVALID.to_string());
    }
    let prefix = expected.synopsis[title.len()..expected.synopsis.len() - suffix.len()]
        .trim()
        .to_string();
    if prefix.is_empty()
        || render_plot_outline_synopsis(&prefix, range.from - 1, expected) != expected.synopsis
    {
        return Err(GENERATION_PLOT_OUTLINE_PREFIX_INVALID.to_string());
    }
    Ok(prefix)
}

/// 基线 `renderPlotOutlineRange`（plot-outline-contract.ts:212）。
fn render_plot_outline_range(
    body: &str,
    range: &PlotOutlineRange,
    expected: &ProjectCoreSynopsisExpected,
) -> Result<String, String> {
    // 基线 `plotOutlineConfirmedPrefix(expected, range)`：仅作
    // 一致性校验（抛错），结果在末行才使用。
    plot_outline_confirmed_prefix(expected, range)?;
    let entries = headings(&expected.synopsis);
    if entries.is_empty() {
        return Ok(render_plot_outline_synopsis(body, range.to, expected));
    }
    let start = entries.iter().find(|entry| entry.from >= range.from);
    let after = entries.iter().find(|entry| entry.from > range.to);
    if let Some(start) = start {
        let before = &expected.synopsis[..start.index];
        if let Some(after) = after {
            return Ok(format!(
                "{}{}\n\n{}",
                before,
                body.trim(),
                &expected.synopsis[after.index..]
            ));
        }
        let title = format!(
            "# {}\n\n",
            if expected.writing_language == "en-US" {
                "Plot Outline"
            } else {
                "情节大纲"
            }
        );
        let suffix =
            render_plot_outline_synopsis("", range.to, expected)[title.len()..].to_string();
        return Ok(format!(
            "{}{}{}",
            before,
            body.trim(),
            if suffix.is_empty() {
                String::new()
            } else {
                format!("\n\n{suffix}")
            }
        ));
    }
    let prefix = plot_outline_confirmed_prefix(expected, range)?;
    Ok(render_plot_outline_synopsis(
        &join_plot_outline_entries(&prefix, body),
        range.to,
        expected,
    ))
}

/// 基线 `plotOutlineRecoveryDraft`（plot-outline-contract.ts:243）：
/// 大纲恢复提案文本——已完成章数 → 下一章占位标题（或失败
/// 候选）→ 范围渲染。
fn plot_outline_recovery_draft(
    progress: &PlotOutlineProgress,
    failed_candidate: Option<&str>,
) -> Result<String, String> {
    let completed = progress
        .composition
        .as_ref()
        .and_then(|composition| composition.chapters.as_ref())
        .map_or(0, |chapters| chapters.len());
    let chapter = progress.range.from + completed as i64;
    let mut body = progress
        .composition
        .as_ref()
        .map(|composition| composition.text.clone())
        .unwrap_or_default();
    match &progress.cursor {
        PlotOutlineCursor::Accept { text, .. } => {
            body = join_plot_outline_entries(&body, text);
        }
        PlotOutlineCursor::Complete => {}
        _ => {
            let heading = if progress.source_expected.writing_language == "en-US" {
                format!("## Chapter {chapter}: ")
            } else {
                format!("## 第{chapter}章：")
            };
            let fallback = failed_candidate
                .map(|candidate| candidate.trim())
                .filter(|candidate| !candidate.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("{heading}\n"));
            body = join_plot_outline_entries(&body, &fallback);
        }
    }
    render_plot_outline_range(
        &body,
        &PlotOutlineRange {
            from: progress.range.from,
            to: chapter.min(progress.range.to),
        },
        &progress.source_expected,
    )
}

// ============================================================================
// 批次 G4-6b：主生成 owner 写面命令 —— `generation:*` 4 频道
// （begin / bind-material-decision / execute / resume）
// ============================================================================

/// 广播 `generation:snapshot` 事件。
///
/// D-2 既定决策：**广播 + 客户端过滤**——经 `AppHandle::emit`
/// 全窗口广播（等价基线 `subscriber.send('generation:snapshot',
/// snapshot)` 的每窗口定向，但省去 `OwnerCacheEntry::subscribers`
/// 订阅登记），客户端按载荷 `projectId` / `epoch` 过滤。
/// 频道名单一事实源：`services::main_generation_owner`。
///
/// ⚠️ 调用点随生成运行服务（`generation-run-service.ts`）迁移波次
/// 接入——基线唯一发射点为 `execute` 的 LLM 调用循环
/// （main-generation-owner.ts:613-615，`snapshotOf(receipt)` 之后）。
#[allow(dead_code)]
pub fn emit_generation_snapshot(app: &AppHandle, payload: &Value) {
    let _ = app.emit(
        crate::services::main_generation_owner::GENERATION_SNAPSHOT_EVENT,
        payload,
    );
}

/// 广播 `generation:reasoning` 事件（D-2 既定决策：同上，广播 +
/// 客户端过滤）；载荷 `MainGenerationReasoningEvent`
/// （`{ projectId, epoch, rootActionId, runId, attemptId, text }`，
/// 绝不进入运行视图或回执）。
///
/// ⚠️ 调用点随生成运行服务迁移波次接入（基线发射点：LLM 分发流的
/// 可见文本增量，generation-controller.ts:121-124 的 `onReasoning`）。
#[allow(dead_code)]
pub fn emit_generation_reasoning(app: &AppHandle, payload: &Value) {
    let _ = app.emit(
        crate::services::main_generation_owner::GENERATION_REASONING_EVENT,
        payload,
    );
}

/// `generation:begin` —— 选择 → 建运行 → 运行视图
/// （基线 `begin(selection)`，main-generation-owner.ts:351）。
///
/// IPC 边界前置校验（基线 generation-controller.ts:174-177，先于
/// owner 门禁）：chapter-draft 必须携带 `preparationId`
/// （`GENERATION_DRAFT_PREPARATION_REQUIRED`）与 `materialDecision`
/// （`GENERATION_MATERIAL_DECISION_REQUIRED`）；非 chapter-draft
/// 不得携带 `materialDecision`（`GENERATION_MATERIAL_DECISION_INVALID`）。
#[tauri::command]
pub async fn generation_begin(
    state: State<'_, AppState>,
    selection: Value,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let operation = selection
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if operation == "chapter-draft"
        && selection
            .get("preparationId")
            .is_none_or(|value| value.is_null())
    {
        return Err(mutating_error(map_generation_error(
            "GENERATION_DRAFT_PREPARATION_REQUIRED",
        )));
    }
    if operation == "chapter-draft"
        && selection
            .get("materialDecision")
            .is_none_or(|value| value.is_null())
    {
        return Err(mutating_error(map_generation_error(
            "GENERATION_MATERIAL_DECISION_REQUIRED",
        )));
    }
    if operation != "chapter-draft"
        && selection
            .get("materialDecision")
            .is_some_and(|value| !value.is_null())
    {
        return Err(mutating_error(map_generation_error(
            "GENERATION_MATERIAL_DECISION_INVALID",
        )));
    }
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.begin(conn, &selection))
}

/// `generation:bind-material-decision` 请求包装
/// （频道契约 `args: [{ handle; materialDecision }]`）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerationBindMaterialDecisionRequest {
    pub handle: MainGenerationRunHandle,
    pub material_decision: Value,
}

/// `generation:bind-material-decision` —— 材料决策收据绑定
/// （基线 `bindMaterialDecision(handle, materialDecision)`，
/// main-generation-owner.ts:646）。
#[tauri::command]
pub async fn generation_bind_material_decision(
    state: State<'_, AppState>,
    request: GenerationBindMaterialDecisionRequest,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let handle = serde_json::to_value(&request.handle)
        .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
    let material_decision = request.material_decision;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.bind_material_decision(conn, &handle, &material_decision))
}

/// `generation:execute` —— 分发 → 执行回执
/// （基线 `execute(request)`，main-generation-owner.ts:501；
/// IPC 边界执行标志恒为 false，对齐 generation-controller.ts:231-236
/// 注册形态）。
#[tauri::command]
pub async fn generation_execute(
    state: State<'_, AppState>,
    request: Value,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.execute(conn, &request))
}

/// `generation:resume` —— 运行恢复重分发
/// （基线 `resume(handle)`，main-generation-owner.ts:625）。
#[tauri::command]
pub async fn generation_resume(
    state: State<'_, AppState>,
    handle: MainGenerationRunHandle,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let handle = serde_json::to_value(&handle)
        .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.resume(conn, &handle))
}

/// `generation:pause` —— 运行暂停
/// （基线 `pause(handle)`，main-generation-owner.ts:1336）。
#[tauri::command]
pub async fn generation_pause(
    state: State<'_, AppState>,
    handle: MainGenerationRunHandle,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let handle = serde_json::to_value(&handle)
        .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.pause(conn, &handle))
}

/// `generation:cancel` —— 运行取消
/// （基线 `cancel(handle)`，main-generation-owner.ts:1337）。
#[tauri::command]
pub async fn generation_cancel(
    state: State<'_, AppState>,
    handle: MainGenerationRunHandle,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let handle = serde_json::to_value(&handle)
        .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.cancel(conn, &handle))
}

/// `generation:restart` —— 运行重启（新选择重开 + 旧 run 处置）
/// （基线 `restart(handle, selection)`，main-generation-owner.ts:1338）。
///
/// IPC 边界前置校验（基线 generation-controller.ts:259-262，先于
/// owner 门禁）：chapter-draft 必须携带 `preparationId`
/// （`GENERATION_DRAFT_PREPARATION_REQUIRED`）与 `materialDecision`
/// （`GENERATION_MATERIAL_DECISION_REQUIRED`）；非 chapter-draft
/// 不得携带 `materialDecision`（`GENERATION_MATERIAL_DECISION_INVALID`）。
#[tauri::command]
pub async fn generation_restart(
    state: State<'_, AppState>,
    handle: MainGenerationRunHandle,
    request: Value,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let operation = request
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if operation == "chapter-draft"
        && request
            .get("preparationId")
            .is_none_or(|value| value.is_null())
    {
        return Err(mutating_error(map_generation_error(
            "GENERATION_DRAFT_PREPARATION_REQUIRED",
        )));
    }
    if operation == "chapter-draft"
        && request
            .get("materialDecision")
            .is_none_or(|value| value.is_null())
    {
        return Err(mutating_error(map_generation_error(
            "GENERATION_MATERIAL_DECISION_REQUIRED",
        )));
    }
    if operation != "chapter-draft"
        && request
            .get("materialDecision")
            .is_some_and(|value| !value.is_null())
    {
        return Err(mutating_error(map_generation_error(
            "GENERATION_MATERIAL_DECISION_INVALID",
        )));
    }
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let handle = serde_json::to_value(&handle)
        .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.restart(conn, &handle, &request))
}

/// `generation:discard-candidate` —— 候选制品丢弃
/// （基线 `discardCandidate(handle, artifactId)`，
/// main-generation-owner.ts:1376）。
#[tauri::command]
pub async fn generation_discard_candidate(
    state: State<'_, AppState>,
    handle: MainGenerationRunHandle,
    artifact_id: String,
    project_session: Option<ProjectSessionContext>,
) -> Result<Value, String> {
    let state = state.inner();
    let cache = MainGenerationOwnerCache::new();
    let mut guard = authorized_gate(&cache, state, project_session.as_ref())?;
    let handle = serde_json::to_value(&handle)
        .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
    let owner = guard.owner();
    state.with_project_db(|conn| owner.discard_candidate(conn, &handle, &artifact_id))
}
