//! graph_generation_source.rs —— 平移自 `electron/services/graph-generation-source.ts`
//!
//! 图生成（剧情树快照 / 叙事线索计划 / 叙事线索事件）的**上下文捕获与校验**：
//! - `validate_graph_generation_input`：输入判别（kind 白名单 + 每种 kind 的键集 + 正整数约束）；
//! - `capture_graph_generation_context`：从作者事实源（project_core / blueprints /
//!   narrative_thread_plans / drafts / finalization_outbox / continuity_projection_meta）
//!   捕获不可变生成上下文，并计算 `targetBaselineHash`（剧情树快照列的 SHA-256）；
//! - `graph_generation_task`：按 kind 派生只读生成任务（prompt 合同）；
//! - `read_graph_generation_context`：从 `DurableGenerationRun.binding.sourceManifest`
//!   重读并**逐项重证**上下文与任务（含哈希与深比较），保证绑定不可篡改。
//!
//! 错误码与上游逐字一致（`GENERATION_GRAPH_*` 系列）。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::repositories::blueprint_repository::get_all as get_all_blueprints;
use crate::repositories::narrative_thread_repository as narrative_threads;
use crate::repositories::plot_tree_repository;
use crate::repositories::project_core_repository::resolve_writing_language;
use crate::services::graph_generation::GraphGenerationTask;
use crate::services::narrative_thread_generation_pure::{
    build_narrative_thread_event_task, build_narrative_thread_plan_task,
    NarrativeThreadEventTaskInput, NarrativeThreadPlanTaskInput,
};
use crate::services::plot_tree_generation_pure::build_plot_tree_task;

/// JS `Number.isSafeInteger` 上界（2^53 - 1）
const JS_MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

const GENERATION_GRAPH_INPUT_INVALID: &str = "GENERATION_GRAPH_INPUT_INVALID";
const GENERATION_GRAPH_CONTEXT_INVALID: &str = "GENERATION_GRAPH_CONTEXT_INVALID";
const GENERATION_GRAPH_PROJECT_REQUIRED: &str = "GENERATION_GRAPH_PROJECT_REQUIRED";
const GENERATION_GRAPH_BLUEPRINT_REQUIRED: &str = "GENERATION_GRAPH_BLUEPRINT_REQUIRED";
const GENERATION_GRAPH_PLAN_REQUIRED: &str = "GENERATION_GRAPH_PLAN_REQUIRED";
const GENERATION_GRAPH_FINALIZED_SOURCE_INVALID: &str = "GENERATION_GRAPH_FINALIZED_SOURCE_INVALID";
const GENERATION_GRAPH_TASK_INVALID: &str = "GENERATION_GRAPH_TASK_INVALID";

/// `captureGraphGenerationContext` 的作用域（对齐基线内联类型 `{ projectId; epoch }`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphGenerationScope {
    pub project_id: String,
    pub epoch: String,
}

/// 每种 kind 对应的 `operation`（对齐基线 `operations` 常量表）。
fn operations_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "plot" => Some("plot-tree-snapshot"),
        "plan" => Some("narrative-thread-plan-candidate"),
        "event" => Some("narrative-thread-event-candidate"),
        _ => None,
    }
}

/// 基线 `positive`：`Number.isSafeInteger(value) && value > 0`。
fn positive(value: &Value) -> bool {
    is_safe_integer(value).is_some_and(|number| number > 0)
}

/// 基线 `positive`（已为 `i64` 的形式）。
fn positive_i64(value: i64) -> bool {
    is_safe_i64(value) && value > 0
}

/// 基线 `nonempty`：非空白字符串。
fn nonempty(value: &str) -> bool {
    !value.trim().is_empty()
}

/// JS `Number.isSafeInteger`（`Value` 形式）。
fn is_safe_integer(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .filter(|number| *number >= -JS_MAX_SAFE_INTEGER && *number <= JS_MAX_SAFE_INTEGER)
}

/// JS `Number.isSafeInteger`（`i64` 形式）。
fn is_safe_i64(value: i64) -> bool {
    value >= -JS_MAX_SAFE_INTEGER && value <= JS_MAX_SAFE_INTEGER
}

/// 基线 `isDeepStrictEqual` 的等价物：两侧序列化为 `serde_json::Value` 后比较
/// （对象键序无关，与 JS 深比较语义一致）。
fn values_equal<T: Serialize + ?Sized, U: Serialize + ?Sized>(a: &T, b: &U) -> bool {
    match (serde_json::to_value(a), serde_json::to_value(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// 文本哈希：UTF-8 字节 SHA-256 小写 hex（对齐基线 `textHash`）。
fn text_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 基线 `new Date().toISOString()` 的等价物（UTC，毫秒精度）。
fn now_iso_string() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs();
    let days = (seconds / 86_400) as i64;
    let time = (seconds % 86_400) as i64;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        time / 3600,
        (time % 3600) / 60,
        time % 60,
        now.subsec_millis(),
    )
}

/// Howard Hinnant 的「天数 → 公历日期」算法。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// 基线 `validateGraphGenerationInput`：kind 白名单 + 每种 kind 的键集 + 正整数约束。
///
/// 输入以 `serde_json::Value` 接收（与命令层从 JSON 反序列化的形态一致），
/// 未知键在值层面逐键拒绝（对齐基线 `Object.keys(input).some(...)`）。
pub fn validate_graph_generation_input(input: &Value) -> Result<GraphGenerationInput, String> {
    let Some(record) = input.as_object() else {
        return Err(GENERATION_GRAPH_INPUT_INVALID.to_string());
    };
    let Some(kind) = record.get("kind").and_then(Value::as_str) else {
        return Err(GENERATION_GRAPH_INPUT_INVALID.to_string());
    };
    let allowed: &[&str] = match kind {
        "plot" => &["kind"],
        "plan" => &["kind", "chapterNumber"],
        "event" => &["kind", "planId", "draftId"],
        _ => return Err(GENERATION_GRAPH_INPUT_INVALID.to_string()),
    };
    if record.keys().any(|key| !allowed.contains(&key.as_str()))
        || kind == "plan" && !positive(record.get("chapterNumber").unwrap_or(&Value::Null))
        || kind == "event"
            && (!positive(record.get("planId").unwrap_or(&Value::Null))
                || !positive(record.get("draftId").unwrap_or(&Value::Null)))
    {
        return Err(GENERATION_GRAPH_INPUT_INVALID.to_string());
    }
    serde_json::from_value(input.clone()).map_err(|_| GENERATION_GRAPH_INPUT_INVALID.to_string())
}

/// 基线 `captureGraphGenerationContext`：从作者事实源捕获不可变生成上下文。
///
/// `ignored_own_event_ids` 仅由主效果所有者在校验其 ACK 后提供；对 event kind，
/// 被忽略的自身事件会从计划视图中剔除并重算 `status` / `dormantChapters` / `overdue`。
pub fn capture_graph_generation_context(
    conn: &Connection,
    input: &Value,
    scope: &GraphGenerationScope,
    key: &str,
    previous: Option<&GraphGenerationContext>,
    ignored_own_event_ids: &[i64],
) -> Result<GraphGenerationContext, String> {
    let valid = validate_graph_generation_input(input)?;
    let valid_input = valid.clone();
    if !nonempty(&scope.project_id)
        || !nonempty(&scope.epoch)
        || !nonempty(key)
        || ignored_own_event_ids.iter().any(|id| *id <= 0)
        || has_duplicates(ignored_own_event_ids)
        || previous.is_some_and(|previous| {
            previous.project_id() != scope.project_id
                || previous.key() != key
                || !nonempty(previous.origin_epoch())
                || !nonempty(previous.created_at())
                || previous.kind() != valid.kind()
                || !values_equal(previous.input(), &valid_input)
        })
    {
        return Err(GENERATION_GRAPH_CONTEXT_INVALID.to_string());
    }

    let Some(core) = conn
        .query_row(
            "SELECT writing_language, total_chapters, plot_tree_snapshot
             FROM project_core WHERE id = 'main'",
            [],
            |row| {
                Ok(ProjectCoreRow {
                    writing_language: row
                        .get::<_, Option<String>>("writing_language")?
                        .unwrap_or_default(),
                    total_chapters: row
                        .get::<_, Option<i64>>("total_chapters")?
                        .unwrap_or_default(),
                    plot_tree_snapshot: row
                        .get::<_, Option<String>>("plot_tree_snapshot")?
                        .unwrap_or_default(),
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
    else {
        return Err(GENERATION_GRAPH_PROJECT_REQUIRED.to_string());
    };

    let base = GraphGenerationBasePayload {
        project_id: scope.project_id.clone(),
        origin_epoch: previous
            .map(|previous| previous.origin_epoch().to_string())
            .unwrap_or_else(|| scope.epoch.clone()),
        key: key.to_string(),
        created_at: previous
            .map(|previous| previous.created_at().to_string())
            .unwrap_or_else(now_iso_string),
        input: valid_input,
        writing_language: resolve_writing_language(&core.writing_language),
    };

    match valid {
        GraphGenerationInput::Plot => {
            let mut sources = plot_tree_repository::read(conn)?;
            // 基线：`sources.snapshot = null; delete sources.storedSnapshotInvalid`
            // —— 捕获上下文永远不含存量快照（生成期间只读来源事实）。
            sources.snapshot = None;
            sources.stored_snapshot_invalid = None;
            Ok(GraphGenerationContext::Plot(GraphGenerationPlotContext {
                base,
                kind: GraphGenerationKind::Plot,
                sources,
                target_baseline_hash: text_hash(&core.plot_tree_snapshot),
            }))
        }
        GraphGenerationInput::Plan { chapter_number } => {
            let blueprint = get_all_blueprints(conn)?
                .into_iter()
                .find(|row| row.chapter_number == chapter_number);
            if blueprint.is_none()
                || !positive_i64(core.total_chapters)
                || chapter_number > core.total_chapters
            {
                return Err(GENERATION_GRAPH_BLUEPRINT_REQUIRED.to_string());
            }
            Ok(GraphGenerationContext::Plan(GraphGenerationPlanContext {
                base,
                kind: GraphGenerationKind::Plan,
                blueprint: blueprint.expect("已校验存在"),
                total_chapters: core.total_chapters,
            }))
        }
        GraphGenerationInput::Event { plan_id, draft_id } => {
            let mut plan = narrative_threads::list(conn)?
                .into_iter()
                .find(|row| row.id == plan_id)
                .ok_or_else(|| GENERATION_GRAPH_PLAN_REQUIRED.to_string())?;
            let Some(row) = conn
                .query_row(
                    FINALIZED_SOURCE_SQL,
                    [draft_id],
                    FinalizedSourceRow::from_row,
                )
                .optional()
                .map_err(|error| error.to_string())?
            else {
                return Err(GENERATION_GRAPH_FINALIZED_SOURCE_INVALID.to_string());
            };
            // 同章最新定稿草稿（版本更高优先；版本相同取更大 id）。
            let latest = conn
                .query_row(
                    "SELECT id FROM drafts
                     WHERE chapter_number = ?1 AND status = 'finalized'
                     ORDER BY version DESC, id DESC LIMIT 1",
                    [row.chapter_number],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if row.status != "finalized"
                || latest != Some(draft_id)
                || !nonempty(&row.finalization_id)
                || row.finalized_chapter_number != row.chapter_number
                || !is_safe_i64(row.content_revision)
                || row.content_revision < 0
                || row.content != row.snapshot
                || text_hash(&row.snapshot) != row.content_hash
                || !is_safe_i64(row.projection_generation)
                || row.projection_generation < 0
            {
                return Err(GENERATION_GRAPH_FINALIZED_SOURCE_INVALID.to_string());
            }
            if !ignored_own_event_ids.is_empty() {
                plan.events
                    .retain(|event| !ignored_own_event_ids.contains(&event.id));
                plan.status = plan
                    .events
                    .last()
                    .map(|event| event.event_type.clone())
                    .unwrap_or_else(|| "planned".to_string());
                let current_chapter: i64 = conn
                    .query_row(
                        "SELECT COALESCE(MAX(chapter_number), 0) FROM drafts WHERE status = 'finalized'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(|error| error.to_string())?;
                let terminal = plan.status == "resolved" || plan.status == "abandoned";
                plan.dormant_chapters = if terminal {
                    0
                } else {
                    let last_chapter = plan
                        .events
                        .last()
                        .map(|event| event.chapter_number)
                        .unwrap_or(plan.target_start_chapter);
                    std::cmp::max(0, current_chapter - last_chapter)
                };
                plan.overdue = !terminal && current_chapter > plan.target_end_chapter;
            }
            Ok(GraphGenerationContext::Event(GraphGenerationEventContext {
                base,
                kind: GraphGenerationKind::Event,
                plan,
                source: FinalizedSourceIdentityPayload {
                    draft_id,
                    chapter_number: row.chapter_number,
                    finalization_id: row.finalization_id,
                    content_hash: row.content_hash,
                },
                content: row.content,
                content_revision: row.content_revision,
                projection_generation: row.projection_generation,
            }))
        }
    }
}

/// 基线 `graphGenerationTask`：按上下文 kind 派生只读生成任务。
pub fn graph_generation_task(
    context: &GraphGenerationContext,
) -> Result<GraphGenerationTask, String> {
    match context {
        GraphGenerationContext::Plot(plot) => build_plot_tree_task(&plot.sources),
        GraphGenerationContext::Plan(plan) => Ok(build_narrative_thread_plan_task(
            NarrativeThreadPlanTaskInput {
                writing_language: plan.base.writing_language.clone(),
                total_chapters: plan.total_chapters,
                blueprint: plan.blueprint.clone(),
            },
        )),
        GraphGenerationContext::Event(event) => Ok(build_narrative_thread_event_task(
            NarrativeThreadEventTaskInput {
                writing_language: event.base.writing_language.clone(),
                plan: event.plan.clone(),
                draft_id: event.source.draft_id,
                chapter_number: event.source.chapter_number,
                finalized_content: event.content.clone(),
            },
        )),
    }
}

/// 基线 `readGraphGenerationContext`：从持久化运行绑定中重读并逐项重证上下文。
///
/// 校验失败分别抛 `GENERATION_GRAPH_CONTEXT_INVALID`（上下文本身）或
/// `GENERATION_GRAPH_TASK_INVALID`（任务哈希 / 深比较不一致）。
pub fn read_graph_generation_context(
    run: &DurableGenerationRun,
) -> Result<GraphGenerationContext, String> {
    let manifest = &run.binding.source_manifest;
    let Some(context_value) = manifest.get("graphGenerationContext") else {
        return Err(GENERATION_GRAPH_CONTEXT_INVALID.to_string());
    };
    let context: GraphGenerationContext = serde_json::from_value(context_value.clone())
        .map_err(|_| GENERATION_GRAPH_CONTEXT_INVALID.to_string())?;
    let input = context.input().clone();
    if input.kind() != context.kind()
        || manifest_str(manifest, "operation") != operations_kind(context.kind().as_str())
        || text_hash(
            &serde_json::to_string(context_value)
                .map_err(|_| GENERATION_GRAPH_CONTEXT_INVALID.to_string())?,
        ) != manifest_str(manifest, "graphGenerationContextHash").unwrap_or_default()
        || context.project_id() != run.binding.project_id
        || !nonempty(context.origin_epoch())
        || manifest_str(manifest, "graphGenerationOriginEpoch") != Some(context.origin_epoch())
        || !nonempty(context.key())
        || !nonempty(context.created_at())
        || manifest_str(manifest, "graphGenerationKey") != Some(context.key())
        || !manifest
            .get("graphGenerationInput")
            .is_some_and(|stored_input| values_equal(stored_input, &input))
    {
        return Err(GENERATION_GRAPH_CONTEXT_INVALID.to_string());
    }
    validate_graph_generation_input(
        &serde_json::to_value(&input).map_err(|_| GENERATION_GRAPH_CONTEXT_INVALID.to_string())?,
    )?;

    let invalid = match &context {
        GraphGenerationContext::Plan(plan) => {
            !matches!(input, GraphGenerationInput::Plan { chapter_number } if chapter_number == plan.blueprint.chapter_number)
                || !positive_i64(plan.total_chapters)
        }
        GraphGenerationContext::Plot(plot) => {
            plot.sources.snapshot.is_some()
                || plot.sources.facts.writing_language != plot.base.writing_language
                || !crate::plot_tree::is_plot_tree_source_revision(&plot.target_baseline_hash)
        }
        GraphGenerationContext::Event(event) => {
            !matches!(input, GraphGenerationInput::Event { plan_id, .. } if plan_id == event.plan.id)
                || event.source.draft_id != input_event_draft_id(&input)
                || !positive_i64(event.source.chapter_number)
                || !nonempty(&event.source.finalization_id)
                || !is_safe_i64(event.content_revision)
                || event.content_revision < 0
                || text_hash(&event.content) != event.source.content_hash
                || !is_safe_i64(event.projection_generation)
                || event.projection_generation < 0
        }
    };
    if invalid {
        return Err(GENERATION_GRAPH_CONTEXT_INVALID.to_string());
    }

    let task = graph_generation_task(&context)?;
    let Some(stored_task) = manifest
        .get("graphGenerationTask")
        .filter(|task| !task.is_null())
    else {
        return Err(GENERATION_GRAPH_TASK_INVALID.to_string());
    };
    let task_serialized =
        serde_json::to_value(&task).map_err(|_| GENERATION_GRAPH_TASK_INVALID.to_string())?;
    if text_hash(
        &serde_json::to_string(stored_task)
            .map_err(|_| GENERATION_GRAPH_TASK_INVALID.to_string())?,
    ) != manifest_str(manifest, "graphGenerationTaskHash").unwrap_or_default()
        || !values_equal(stored_task, &task_serialized)
    {
        return Err(GENERATION_GRAPH_TASK_INVALID.to_string());
    }
    Ok(context)
}

/// event kind 校验用的 `input.draftId` 提取（非 event kind 返回 0）。
fn input_event_draft_id(input: &GraphGenerationInput) -> i64 {
    match input {
        GraphGenerationInput::Event { draft_id, .. } => *draft_id,
        _ => 0,
    }
}

/// 基线 `Set` 去重判定（`ignoredOwnEventIds` 唯一性约束）。
fn has_duplicates(values: &[i64]) -> bool {
    for (index, value) in values.iter().enumerate() {
        if values[..index].contains(value) {
            return true;
        }
    }
    false
}

/// manifest 字符串字段读取（缺失或非字符串均视为不匹配）。
fn manifest_str<'a>(manifest: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    manifest.get(key).and_then(Value::as_str)
}

/// `project_core` 主行（捕获上下文所需三列）。
struct ProjectCoreRow {
    writing_language: String,
    total_chapters: i64,
    plot_tree_snapshot: String,
}

/// event kind 的定稿来源行（对齐基线 `captureGraphGenerationContext` 的 JOIN 投影）。
struct FinalizedSourceRow {
    chapter_number: i64,
    status: String,
    content: String,
    finalization_id: String,
    content_hash: String,
    snapshot: String,
    finalized_chapter_number: i64,
    content_revision: i64,
    projection_generation: i64,
}

impl FinalizedSourceRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            chapter_number: row.get("chapter_number")?,
            status: row.get("status")?,
            content: row.get("content")?,
            finalization_id: row.get("finalization_id")?,
            content_hash: row.get("content_hash")?,
            snapshot: row.get("snapshot")?,
            finalized_chapter_number: row.get("finalized_chapter_number")?,
            content_revision: row.get("content_revision")?,
            projection_generation: row.get("projection_generation")?,
        })
    }
}

/// event kind 的定稿来源 SQL（语义对齐基线；列别名用 snake_case 以适配 rusqlite 列名查找）。
const FINALIZED_SOURCE_SQL: &str = "SELECT d.chapter_number AS chapter_number, d.status,
    c.body AS content, f.finalization_id AS finalization_id, f.content_hash AS content_hash,
    f.content_snapshot AS snapshot, f.chapter_number AS finalized_chapter_number,
    f.content_revision AS content_revision, m.generation AS projection_generation
    FROM drafts d JOIN contents c ON c.id = d.content_id
    JOIN finalization_outbox f ON f.draft_id = d.id
    JOIN continuity_projection_meta m ON m.id = 'main' WHERE d.id = ?1";

/// 上下文公共字段（对齐基线 `{...base}` 展开；序列化键序与基线一致）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphGenerationBasePayload {
    pub project_id: String,
    pub origin_epoch: String,
    pub key: String,
    pub created_at: String,
    pub input: GraphGenerationInput,
    pub writing_language: String,
}

/// plot kind 上下文（对齐基线 `GraphGenerationContext` 的 plot 分支）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphGenerationPlotContext {
    #[serde(flatten)]
    pub base: GraphGenerationBasePayload,
    pub kind: GraphGenerationKind,
    pub sources: crate::repositories::plot_tree_repository::PlotTreeSourceBundle,
    pub target_baseline_hash: String,
}

/// plan kind 上下文（对齐基线 `GraphGenerationContext` 的 plan 分支）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphGenerationPlanContext {
    #[serde(flatten)]
    pub base: GraphGenerationBasePayload,
    pub kind: GraphGenerationKind,
    pub blueprint: crate::repositories::blueprint_repository::BlueprintData,
    pub total_chapters: i64,
}

/// event kind 上下文（对齐基线 `GraphGenerationContext` 的 event 分支）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphGenerationEventContext {
    #[serde(flatten)]
    pub base: GraphGenerationBasePayload,
    pub kind: GraphGenerationKind,
    pub plan: crate::repositories::narrative_thread_repository::NarrativeThreadView,
    pub source: FinalizedSourceIdentityPayload,
    pub content: String,
    pub content_revision: i64,
    pub projection_generation: i64,
}

/// 定稿来源身份（对齐 `src/shared/finalized-continuity.ts` 的 `FinalizedSourceIdentity`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedSourceIdentityPayload {
    pub draft_id: i64,
    pub finalization_id: String,
    pub chapter_number: i64,
    pub content_hash: String,
}

/// 上下文 kind 判别（对齐基线 `input.kind` / `context.kind` 的三值联合）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GraphGenerationKind {
    Plot,
    Plan,
    Event,
}

impl GraphGenerationKind {
    /// 基线 `String` 形式（`as_str` 语义，用于 manifest 比较）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plot => "plot",
            Self::Plan => "plan",
            Self::Event => "event",
        }
    }
}

/// 图生成输入判别（对齐基线 `GraphGenerationInput` 三分支联合）。
///
/// 序列化形状与基线对象字面量逐键一致：`{"kind":"plot"}` /
/// `{"kind":"plan","chapterNumber":n}` / `{"kind":"event","planId":n,"draftId":n}`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphGenerationInput {
    Plot,
    Plan { chapter_number: i64 },
    Event { plan_id: i64, draft_id: i64 },
}

impl GraphGenerationInput {
    /// 基线 `input.kind`。
    pub fn kind(&self) -> GraphGenerationKind {
        match self {
            Self::Plot => GraphGenerationKind::Plot,
            Self::Plan { .. } => GraphGenerationKind::Plan,
            Self::Event { .. } => GraphGenerationKind::Event,
        }
    }
}

impl Serialize for GraphGenerationInput {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        match self {
            Self::Plot => {
                map.serialize_entry("kind", "plot")?;
            }
            Self::Plan { chapter_number } => {
                map.serialize_entry("kind", "plan")?;
                map.serialize_entry("chapterNumber", chapter_number)?;
            }
            Self::Event { plan_id, draft_id } => {
                map.serialize_entry("kind", "event")?;
                map.serialize_entry("planId", plan_id)?;
                map.serialize_entry("draftId", draft_id)?;
            }
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for GraphGenerationInput {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let Some(record) = value.as_object() else {
            return Err(serde::de::Error::custom(GENERATION_GRAPH_INPUT_INVALID));
        };
        match record.get("kind").and_then(Value::as_str) {
            Some("plot") => Ok(Self::Plot),
            Some("plan") => {
                let chapter_number = record
                    .get("chapterNumber")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| serde::de::Error::custom(GENERATION_GRAPH_INPUT_INVALID))?;
                Ok(Self::Plan { chapter_number })
            }
            Some("event") => {
                let plan_id = record
                    .get("planId")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| serde::de::Error::custom(GENERATION_GRAPH_INPUT_INVALID))?;
                let draft_id = record
                    .get("draftId")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| serde::de::Error::custom(GENERATION_GRAPH_INPUT_INVALID))?;
                Ok(Self::Event { plan_id, draft_id })
            }
            _ => Err(serde::de::Error::custom(GENERATION_GRAPH_INPUT_INVALID)),
        }
    }
}

/// 上下文枚举（三分支联合；序列化直接输出各分支结构，无外层标签）。
#[derive(Debug, Clone, PartialEq)]
pub enum GraphGenerationContext {
    Plot(GraphGenerationPlotContext),
    Plan(GraphGenerationPlanContext),
    Event(GraphGenerationEventContext),
}

impl GraphGenerationContext {
    /// 公共字段（对齐基线 `{...base}` 展开）。
    pub fn base(&self) -> &GraphGenerationBasePayload {
        match self {
            Self::Plot(context) => &context.base,
            Self::Plan(context) => &context.base,
            Self::Event(context) => &context.base,
        }
    }

    /// 基线 `context.projectId`。
    pub fn project_id(&self) -> &str {
        &self.base().project_id
    }

    /// 基线 `context.originEpoch`。
    pub fn origin_epoch(&self) -> &str {
        &self.base().origin_epoch
    }

    /// 基线 `context.key`。
    pub fn key(&self) -> &str {
        &self.base().key
    }

    /// 基线 `context.createdAt`。
    pub fn created_at(&self) -> &str {
        &self.base().created_at
    }

    /// 基线 `context.input`。
    pub fn input(&self) -> &GraphGenerationInput {
        &self.base().input
    }

    /// 基线 `context.writingLanguage`。
    pub fn writing_language(&self) -> &str {
        &self.base().writing_language
    }

    /// 基线 `context.kind`。
    pub fn kind(&self) -> GraphGenerationKind {
        match self {
            Self::Plot(context) => context.kind,
            Self::Plan(context) => context.kind,
            Self::Event(context) => context.kind,
        }
    }
}

impl Serialize for GraphGenerationContext {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Plot(context) => context.serialize(serializer),
            Self::Plan(context) => context.serialize(serializer),
            Self::Event(context) => context.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for GraphGenerationContext {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let kind = value
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| serde::de::Error::custom(GENERATION_GRAPH_CONTEXT_INVALID))?;
        match kind {
            "plot" => {
                let context: GraphGenerationPlotContext = serde_json::from_value(value)
                    .map_err(|error| serde::de::Error::custom(error.to_string()))?;
                Ok(Self::Plot(context))
            }
            "plan" => {
                let context: GraphGenerationPlanContext = serde_json::from_value(value)
                    .map_err(|error| serde::de::Error::custom(error.to_string()))?;
                Ok(Self::Plan(context))
            }
            "event" => {
                let context: GraphGenerationEventContext = serde_json::from_value(value)
                    .map_err(|error| serde::de::Error::custom(error.to_string()))?;
                Ok(Self::Event(context))
            }
            _ => Err(serde::de::Error::custom(GENERATION_GRAPH_CONTEXT_INVALID)),
        }
    }
}

use crate::repositories::generation_run_repository::DurableGenerationRun;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn memory_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库应可打开");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("应启用外键");
        schema::create_tables(&conn).expect("建表应成功");
        conn
    }

    fn seed_project(conn: &Connection) {
        conn.execute(
            "INSERT INTO project_core (id, project_name, writing_language, synopsis, total_chapters)
             VALUES ('main', '项目', 'zh-CN', '梗概', 10)",
            [],
        )
        .unwrap();
    }

    fn input_json(kind: &str, extra: &str) -> Value {
        serde_json::from_str(&format!("{{\"kind\":\"{kind}\"{extra}}}")).unwrap()
    }

    #[test]
    fn validate_input_accepts_each_kind_test() {
        assert!(matches!(
            validate_graph_generation_input(&input_json("plot", "")).unwrap(),
            GraphGenerationInput::Plot
        ));
        assert!(matches!(
            validate_graph_generation_input(&input_json("plan", ",\"chapterNumber\":3")).unwrap(),
            GraphGenerationInput::Plan { chapter_number: 3 }
        ));
        assert!(matches!(
            validate_graph_generation_input(&input_json("event", ",\"planId\":1,\"draftId\":7"))
                .unwrap(),
            GraphGenerationInput::Event {
                plan_id: 1,
                draft_id: 7
            }
        ));
    }

    #[test]
    fn validate_input_rejects_bad_kinds_and_keys_test() {
        for bad in [
            input_json("plot", ",\"extra\":1"),
            input_json("plan", ",\"chapterNumber\":0"),
            input_json("plan", ",\"chapterNumber\":1,\"extra\":1"),
            input_json("event", ",\"planId\":1"),
            input_json("event", ",\"planId\":1,\"draftId\":-2"),
            serde_json::from_str::<Value>("[]").unwrap(),
            serde_json::from_str::<Value>("\"plot\"").unwrap(),
            input_json("unknown", ""),
        ] {
            assert_eq!(
                validate_graph_generation_input(&bad).unwrap_err(),
                "GENERATION_GRAPH_INPUT_INVALID"
            );
        }
    }

    #[test]
    fn capture_plan_context_test() {
        let conn = memory_conn();
        seed_project(&conn);
        conn.execute(
            "INSERT INTO blueprints (chapter_number, title, purpose, key_events)
             VALUES (3, '第三章', '目的', '事件')",
            [],
        )
        .unwrap();
        let scope = GraphGenerationScope {
            project_id: "p1".to_string(),
            epoch: "e1".to_string(),
        };
        let context = capture_graph_generation_context(
            &conn,
            &input_json("plan", ",\"chapterNumber\":3"),
            &scope,
            "Main-plot-1",
            None,
            &[],
        )
        .unwrap();
        assert_eq!(context.kind(), GraphGenerationKind::Plan);
        assert_eq!(context.project_id(), "p1");
        assert_eq!(context.origin_epoch(), "e1");
        assert_eq!(context.key(), "Main-plot-1");
        assert_eq!(context.writing_language(), "zh-CN");
        let GraphGenerationContext::Plan(plan) = &context else {
            panic!("应为 plan 上下文");
        };
        assert_eq!(plan.blueprint.chapter_number, 3);
        assert_eq!(plan.total_chapters, 10);
        // 序列化键序：公共字段在前，kind / 蓝图 / totalChapters 在后
        let json = serde_json::to_string(&context).unwrap();
        assert!(json.starts_with(
            r#"{"projectId":"p1","originEpoch":"e1","key":"Main-plot-1","createdAt":"#,
        ));
        assert!(json.contains(r#","kind":"plan","blueprint":{""#));
        assert!(json.ends_with(r#","totalChapters":10}"#));
    }

    #[test]
    fn capture_plan_context_missing_blueprint_test() {
        let conn = memory_conn();
        seed_project(&conn);
        let scope = GraphGenerationScope {
            project_id: "p1".to_string(),
            epoch: "e1".to_string(),
        };
        let error = capture_graph_generation_context(
            &conn,
            &input_json("plan", ",\"chapterNumber\":5"),
            &scope,
            "k",
            None,
            &[],
        )
        .unwrap_err();
        assert_eq!(error, "GENERATION_GRAPH_BLUEPRINT_REQUIRED");
    }

    #[test]
    fn capture_context_without_project_test() {
        let conn = memory_conn();
        let scope = GraphGenerationScope {
            project_id: "p1".to_string(),
            epoch: "e1".to_string(),
        };
        let error = capture_graph_generation_context(
            &conn,
            &input_json("plot", ""),
            &scope,
            "k",
            None,
            &[],
        )
        .unwrap_err();
        assert_eq!(error, "GENERATION_GRAPH_PROJECT_REQUIRED");
    }

    #[test]
    fn capture_context_rejects_bad_scope_or_ignored_ids_test() {
        let conn = memory_conn();
        seed_project(&conn);
        let scope = GraphGenerationScope {
            project_id: "p1".to_string(),
            epoch: "e1".to_string(),
        };
        let empty_scope = GraphGenerationScope {
            project_id: " ".to_string(),
            epoch: "e1".to_string(),
        };
        assert_eq!(
            capture_graph_generation_context(
                &conn,
                &input_json("plot", ""),
                &empty_scope,
                "k",
                None,
                &[]
            )
            .unwrap_err(),
            "GENERATION_GRAPH_CONTEXT_INVALID"
        );
        assert_eq!(
            capture_graph_generation_context(
                &conn,
                &input_json("plot", ""),
                &scope,
                "k",
                None,
                &[0]
            )
            .unwrap_err(),
            "GENERATION_GRAPH_CONTEXT_INVALID"
        );
        assert_eq!(
            capture_graph_generation_context(
                &conn,
                &input_json("plot", ""),
                &scope,
                "k",
                None,
                &[1, 1]
            )
            .unwrap_err(),
            "GENERATION_GRAPH_CONTEXT_INVALID"
        );
    }

    #[test]
    fn graph_generation_task_dispatch_test() {
        let conn = memory_conn();
        seed_project(&conn);
        conn.execute(
            "INSERT INTO blueprints (chapter_number, title, purpose, key_events)
             VALUES (1, '第一章', '目的', '事件')",
            [],
        )
        .unwrap();
        let scope = GraphGenerationScope {
            project_id: "p1".to_string(),
            epoch: "e1".to_string(),
        };
        let plan_context = capture_graph_generation_context(
            &conn,
            &input_json("plan", ",\"chapterNumber\":1"),
            &scope,
            "k",
            None,
            &[],
        )
        .unwrap();
        let task = graph_generation_task(&plan_context).unwrap();
        assert_eq!(task.purpose, "narrative-thread-plan-candidate");
        assert_eq!(task.reasoning_stage, "planning");
        assert_eq!(task.output, "structured-data");
        assert_eq!(task.messages.len(), 2);
        assert_eq!(task.messages[0].role, "system");
        assert_eq!(task.messages[1].role, "user");
        // 用户消息内容：{totalChapters, blueprint}
        let user: Value = serde_json::from_str(&task.messages[1].content).unwrap();
        assert_eq!(user.get("totalChapters").and_then(Value::as_i64), Some(10));
        assert_eq!(
            user.get("blueprint")
                .and_then(|b| b.get("chapterNumber"))
                .and_then(Value::as_i64),
            Some(1)
        );

        let plot_context = capture_graph_generation_context(
            &conn,
            &input_json("plot", ""),
            &scope,
            "k2",
            None,
            &[],
        )
        .unwrap();
        let task = graph_generation_task(&plot_context).unwrap();
        assert_eq!(task.purpose, "plot-tree-snapshot");
    }

    #[test]
    fn read_context_round_trip_test() {
        let conn = memory_conn();
        seed_project(&conn);
        conn.execute(
            "INSERT INTO blueprints (chapter_number, title, purpose, key_events)
             VALUES (1, '第一章', '目的', '事件')",
            [],
        )
        .unwrap();
        let scope = GraphGenerationScope {
            project_id: "p1".to_string(),
            epoch: "e1".to_string(),
        };
        let context = capture_graph_generation_context(
            &conn,
            &input_json("plan", ",\"chapterNumber\":1"),
            &scope,
            "Main-key-1",
            None,
            &[],
        )
        .unwrap();
        let task = graph_generation_task(&context).unwrap();
        let context_json = serde_json::to_value(&context).unwrap();
        let task_json = serde_json::to_value(&task).unwrap();
        let mut manifest = Map::new();
        manifest.insert(
            "operation".to_string(),
            Value::from("narrative-thread-plan-candidate"),
        );
        manifest.insert("graphGenerationContext".to_string(), context_json.clone());
        manifest.insert(
            "graphGenerationContextHash".to_string(),
            Value::from(text_hash(&serde_json::to_string(&context_json).unwrap())),
        );
        manifest.insert("graphGenerationOriginEpoch".to_string(), Value::from("e1"));
        manifest.insert("graphGenerationKey".to_string(), Value::from("Main-key-1"));
        manifest.insert(
            "graphGenerationInput".to_string(),
            serde_json::to_value(context.base().input.clone()).unwrap(),
        );
        manifest.insert("graphGenerationTask".to_string(), task_json.clone());
        manifest.insert(
            "graphGenerationTaskHash".to_string(),
            Value::from(text_hash(&serde_json::to_string(&task_json).unwrap())),
        );
        let run = DurableGenerationRun {
            run_id: "run-1".to_string(),
            root_action_id: "root-1".to_string(),
            binding: crate::repositories::generation_run_repository::RunBinding {
                project_id: "p1".to_string(),
                epoch: "e1".to_string(),
                fingerprint: test_fingerprint(),
                context_snapshot_id: "cs-1".to_string(),
                source_manifest: manifest,
                source_refs: Vec::new(),
                extra: Map::new(),
            },
            status: "active".to_string(),
        };
        let read = read_graph_generation_context(&run).unwrap();
        assert_eq!(read, context);

        // 任务哈希被篡改 → GENERATION_GRAPH_TASK_INVALID
        let mut tampered = run;
        tampered.binding.source_manifest.insert(
            "graphGenerationTaskHash".to_string(),
            Value::from("0".repeat(64)),
        );
        assert_eq!(
            read_graph_generation_context(&tampered).unwrap_err(),
            "GENERATION_GRAPH_TASK_INVALID"
        );
    }

    fn test_fingerprint() -> crate::repositories::generation_run_repository::FrozenInputFingerprint
    {
        crate::repositories::generation_run_repository::FrozenInputFingerprint {
            chapter_brief_hash: "h".to_string(),
            author_guidance_hash: "h".to_string(),
            dependency_hash: "h".to_string(),
            context_snapshot_hash: "h".to_string(),
            template_hash: "h".to_string(),
            skill_snapshot_hash: "h".to_string(),
            model_lease_revision: "h".to_string(),
            policy_hash: "h".to_string(),
            output_contract_hash: "h".to_string(),
        }
    }
}
