//! plot_tree_generation_pure.rs —— 完整平移自 `src/shared/plot-tree-generation-pure.ts`
//!
//! 剧情树快照生成的**纯函数层**（无 DB / 无 IO）：
//! - `build_plot_tree_task`：从 `PlotTreeSourceBundle` 构造只读生成任务
//!   （prompt 合同逐字对齐上游，ZH/EN 双语按 `writingLanguage` 选择）；
//! - `parse_plot_tree_snapshot` / `derive_plot_tree_snapshot`：模型输出 →
//!   已校验 `PlotTreeSnapshot`（model → normalized → deterministic 三级降级）；
//! - `deterministic_plot_tree_snapshot`：无可用模型输出时的只读兜底快照。
//!
//! 上游以具名 `Error` 子类抛出失败；本模块按项目惯例返回 `Result<T, String>`，
//! 错误串为上游错误码（`NO_EVENT_SOURCES` / `SOURCE_LIMIT_EXCEEDED` /
//! `SOURCE_INPUT_TOO_LARGE` / `invalid_json` 等）。上游错误类仍保留为
//! 公开结构体（`code` 字段逐字对齐），供效果层按码分类。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::plot_tree::{
    assert_plot_tree_snapshot, has_usable_plot_tree_event_source, PlotTreeFinalizedChapterFact,
    PlotTreeNarrativeThreadEventFact, PlotTreeSnapshot,
};
use crate::repositories::plot_tree_repository::PlotTreeSourceBundle;

#[cfg(test)]
use crate::plot_tree::PlotTreeTrack;
use crate::services::graph_generation::{GraphGenerationMessage, GraphGenerationTask};

/// 生成预算（对齐基线 `PLOT_TREE_GENERATION_BUDGET`，`Object.freeze`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlotTreeGenerationBudget {
    pub max_attempts: i64,
    pub max_requested_output_tokens: i64,
    pub max_requested_output_tokens_per_attempt: i64,
    pub deadline_ms: i64,
}

/// 基线 `PLOT_TREE_GENERATION_BUDGET`。
pub const PLOT_TREE_GENERATION_BUDGET: PlotTreeGenerationBudget = PlotTreeGenerationBudget {
    max_attempts: 1,
    max_requested_output_tokens: 8192,
    max_requested_output_tokens_per_attempt: 8192,
    deadline_ms: 600_000,
};

/// 基线 `PLOT_TREE_INPUT_MAX_CHARACTERS`。
pub const PLOT_TREE_INPUT_MAX_CHARACTERS: usize = 220_000;

/// 基线 `PLOT_TREE_SOURCE_LIMITS`（大项目阈值为 120 个蓝图 / 定稿章节）。
#[derive(Debug, Clone, Copy)]
struct PlotTreeSourceLimits {
    synopsis_characters: usize,
    label_characters: usize,
    detail_characters: usize,
    large_project_threshold: usize,
    large_project_synopsis_characters: usize,
    large_project_label_characters: usize,
    large_project_detail_characters: usize,
    large_project_event_detail_characters: usize,
    blueprints: usize,
    finalized_chapters: usize,
}

const PLOT_TREE_SOURCE_LIMITS: PlotTreeSourceLimits = PlotTreeSourceLimits {
    synopsis_characters: 6_000,
    label_characters: 160,
    detail_characters: 320,
    large_project_threshold: 120,
    large_project_synopsis_characters: 3_000,
    large_project_label_characters: 48,
    large_project_detail_characters: 64,
    large_project_event_detail_characters: 24,
    blueprints: 200,
    finalized_chapters: 200,
};

/// 基线 `PlotTreeResponseErrorCode`（`'invalid_json' | 'invalid_contract'`）。
pub type PlotTreeResponseErrorCode = String;

/// 基线 `PlotTreeGenerationErrorCode`（`'DEADLINE_EXHAUSTED' | 'PROVIDER_REQUEST_FAILED'`）。
pub type PlotTreeGenerationErrorCode = String;

/// 基线 `PlotTreeGenerationError`（保留 `code` 字段供效果层分类）。
#[derive(Debug, Clone, PartialEq)]
pub struct PlotTreeGenerationError {
    pub code: PlotTreeGenerationErrorCode,
}

impl std::fmt::Display for PlotTreeGenerationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.code)
    }
}

impl std::error::Error for PlotTreeGenerationError {}

/// 基线 `PlotTreeResponseError`（保留 `code` + `message`）。
#[derive(Debug, Clone, PartialEq)]
pub struct PlotTreeResponseError {
    pub code: PlotTreeResponseErrorCode,
    pub message: String,
}

impl std::fmt::Display for PlotTreeResponseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PlotTreeResponseError {}

/// 基线 `PlotTreeSourceError`（`code = 'NO_EVENT_SOURCES'`）。
#[derive(Debug, Clone, PartialEq)]
pub struct PlotTreeSourceError {
    pub code: &'static str,
}

impl PlotTreeSourceError {
    /// 基线无参构造器。
    pub fn new() -> Self {
        Self {
            code: "NO_EVENT_SOURCES",
        }
    }
}

impl Default for PlotTreeSourceError {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for PlotTreeSourceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for PlotTreeSourceError {}

/// 基线 `PlotTreeSourceLimitError`（`code = 'SOURCE_LIMIT_EXCEEDED'`）。
#[derive(Debug, Clone, PartialEq)]
pub struct PlotTreeSourceLimitError {
    pub code: &'static str,
    pub maximum: i64,
}

impl PlotTreeSourceLimitError {
    /// 基线构造器（`maximum` 为超限的源数量上限）。
    pub fn new(maximum: i64) -> Self {
        Self {
            code: "SOURCE_LIMIT_EXCEEDED",
            maximum,
        }
    }
}

impl std::fmt::Display for PlotTreeSourceLimitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for PlotTreeSourceLimitError {}

/// 基线 `PlotTreeInputLimitError`（`code = 'SOURCE_INPUT_TOO_LARGE'`）。
#[derive(Debug, Clone, PartialEq)]
pub struct PlotTreeInputLimitError {
    pub code: &'static str,
    pub maximum_characters: i64,
}

impl PlotTreeInputLimitError {
    /// 基线构造器（`maximumCharacters` 为输入字符上限）。
    pub fn new(maximum_characters: i64) -> Self {
        Self {
            code: "SOURCE_INPUT_TOO_LARGE",
            maximum_characters,
        }
    }
}

impl std::fmt::Display for PlotTreeInputLimitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for PlotTreeInputLimitError {}

/// 基线 `PlotTreeIncompleteError`：模型未以 `stop` 结束时的本地化消息。
#[derive(Debug, Clone, PartialEq)]
pub struct PlotTreeIncompleteError {
    pub finish_reason: String,
    pub message: String,
}

impl PlotTreeIncompleteError {
    /// 基线构造器：按 `writingLanguage` 与 `finishReason` 生成本地化消息。
    pub fn new(writing_language: &str, finish_reason: &str) -> Self {
        let message = if finish_reason == "length" {
            prompt_language_text(
                writing_language,
                "剧情树输出达到模型最大长度，结果未保存，请提高最大输出 Tokens 或缩短项目资料。",
                "Plot-tree output reached the model maximum output length and was not saved. Increase maximum output tokens or shorten the project sources.",
            )
            .to_string()
        } else if finish_reason == "content_filter" {
            prompt_language_text(
                writing_language,
                "剧情树输出因内容限制未完成，结果未保存。",
                "Plot-tree output was stopped by the content policy and was not saved.",
            )
            .to_string()
        } else if finish_reason == "cancelled" {
            prompt_language_text(
                writing_language,
                "剧情树生成已取消，结果未保存。",
                "Plot-tree generation was cancelled and the result was not saved.",
            )
            .to_string()
        } else {
            prompt_language_text(
                writing_language,
                "剧情树生成未正常完成，结果未保存。",
                "Plot-tree generation did not complete normally and the result was not saved.",
            )
            .to_string()
        };
        Self {
            finish_reason: finish_reason.to_string(),
            message,
        }
    }
}

impl std::fmt::Display for PlotTreeIncompleteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PlotTreeIncompleteError {}

/// 基线 `parsePlotTreeSnapshot`：解析并严格校验模型输出的剧情树快照。
///
/// `generated_at` 为 `None` 时使用当前 UTC 时间戳（对齐基线默认参数）。
pub fn parse_plot_tree_snapshot(
    content: &str,
    sources: &PlotTreeSourceBundle,
    generated_at: Option<&str>,
) -> Result<PlotTreeSnapshot, String> {
    let generated_at = match generated_at {
        Some(timestamp) => timestamp.to_string(),
        None => current_iso_timestamp(),
    };
    let parsed = parse_plot_tree_json(content)?;
    strict_snapshot(
        parsed.get("tracks").unwrap_or(&Value::Null),
        sources,
        &generated_at,
    )
}

/// 基线 `strictSnapshot`：用来源事实校验快照载荷。
fn strict_snapshot(
    tracks: &Value,
    sources: &PlotTreeSourceBundle,
    generated_at: &str,
) -> Result<PlotTreeSnapshot, String> {
    let value = serde_json::json!({
        "version": 1,
        "generatedAt": generated_at,
        "writingLanguage": sources.facts.writing_language,
        "sourceRevision": sources.source_revision,
        "tracks": tracks,
    });
    assert_plot_tree_snapshot(&value, &sources.facts, &sources.source_revision)
}

/// 基线 `parsePlotTreeJSON`：解析 JSON（支持 ```json 围栏；大小写不敏感）。
fn parse_plot_tree_json(content: &str) -> Result<Value, String> {
    let trimmed = content.trim();
    let payload = parse_fenced_json(trimmed)
        .map(|fenced| fenced.trim())
        .unwrap_or(trimmed);
    serde_json::from_str(payload).map_err(|error| format!("invalid_json: {error}"))
}

/// JS `/^```json\s*([\s\S]*?)\s*```$/iu` 的等价物：整体匹配时返回捕获组。
fn parse_fenced_json(trimmed: &str) -> Option<&str> {
    let lower = trimmed.to_lowercase();
    if !lower.starts_with("```json") {
        return None;
    }
    let after_prefix = trimmed["```json".len()..].trim_start();
    if !after_prefix.to_lowercase().ends_with("```") {
        return None;
    }
    let body = after_prefix[..after_prefix.len() - "```".len()].trim_end();
    let start = trimmed.len() - after_prefix.len();
    Some(&trimmed[start..start + body.len()])
}

/// 基线 `object`：非 null 的非数组对象。
fn object(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    value.as_object()
}

/// 基线 `exactEventSource`：用合成「source-check」轨道验证单个事件来源，
/// 返回校验通过后的**规范化**来源；任何校验失败返回 `None`。
fn exact_event_source(
    value: &Value,
    status: &str,
    chapter_number: i64,
    sources: &PlotTreeSourceBundle,
    generated_at: &str,
) -> Option<Value> {
    let synthetic = serde_json::json!([{
        "id": "source-check",
        "title": "source-check",
        "role": "main",
        "startChapter": chapter_number,
        "endChapter": chapter_number,
        "summary": "source-check",
        "events": [{
            "status": status,
            "chapterNumber": chapter_number,
            "summary": "source-check",
            "sources": [value],
        }],
    }]);
    let snapshot = strict_snapshot(&synthetic, sources, generated_at).ok()?;
    let source = snapshot
        .tracks
        .into_iter()
        .next()?
        .events
        .into_iter()
        .next()?
        .sources
        .into_iter()
        .next()?;
    serde_json::to_value(&source).ok()
}

/// 基线 `normalizePlotTreeSnapshot`：逐事件校验并规范化模型输出，
/// 无法规范化的事件 / 轨道被丢弃；全部失败时抛错（由调用方降级）。
fn normalize_plot_tree_snapshot(
    parsed: &Value,
    sources: &PlotTreeSourceBundle,
    generated_at: &str,
) -> Result<PlotTreeSnapshot, String> {
    let raw_tracks: Vec<&Value> = parsed
        .get("tracks")
        .and_then(Value::as_array)
        .map(|tracks| tracks.iter().collect())
        .unwrap_or_default();
    let mut tracks: Vec<Value> = Vec::new();
    for candidate in raw_tracks {
        let Some(track) = object(candidate) else {
            continue;
        };
        let Some(raw_events) = track.get("events").and_then(Value::as_array) else {
            continue;
        };
        let mut events: Vec<Value> = Vec::new();
        for candidate_event in raw_events {
            let Some(event) = object(candidate_event) else {
                continue;
            };
            let status = event
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if status != "planned" && status != "occurred" {
                continue;
            }
            let Some(chapter_number) = event.get("chapterNumber").and_then(Value::as_i64) else {
                continue;
            };
            if !is_safe_integer(chapter_number) {
                continue;
            }
            let Some(raw_sources) = event.get("sources").and_then(Value::as_array) else {
                continue;
            };
            let mut event_sources: Vec<Value> = Vec::new();
            let mut seen: Vec<String> = Vec::new();
            for source_value in raw_sources {
                let Some(source) =
                    exact_event_source(source_value, status, chapter_number, sources, generated_at)
                else {
                    continue;
                };
                let key = serde_json::to_string(&source).unwrap_or_default();
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                event_sources.push(source);
            }
            if event_sources.is_empty() {
                continue;
            }
            let mut normalized = event.clone();
            normalized.insert("status".to_string(), Value::from(status));
            normalized.insert("chapterNumber".to_string(), Value::from(chapter_number));
            normalized.insert("sources".to_string(), Value::Array(event_sources));
            events.push(Value::Object(normalized));
        }
        if events.is_empty() {
            continue;
        }
        let chapters: Vec<i64> = events
            .iter()
            .map(|event| {
                event
                    .get("chapterNumber")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
            })
            .collect();
        let mut normalized_track = track.clone();
        normalized_track.insert(
            "startChapter".to_string(),
            Value::from(chapters.iter().min().copied().unwrap_or(0)),
        );
        normalized_track.insert(
            "endChapter".to_string(),
            Value::from(chapters.iter().max().copied().unwrap_or(0)),
        );
        normalized_track.insert("events".to_string(), Value::Array(events));
        tracks.push(Value::Object(normalized_track));
    }
    strict_snapshot(&Value::Array(tracks), sources, generated_at)
}

/// 基线 `deterministicPlotTreeSnapshot`：从来源事实合成只读剧情树快照。
///
/// 无任何可用事件时抛 `NO_EVENT_SOURCES`（对齐基线 `PlotTreeSourceError`）。
fn deterministic_plot_tree_snapshot(
    sources: &PlotTreeSourceBundle,
    generated_at: &str,
) -> Result<PlotTreeSnapshot, String> {
    let finalized_chapters: Vec<&PlotTreeFinalizedChapterFact> = sources
        .facts
        .finalized_chapters
        .iter()
        .filter(|chapter| {
            (!chapter.summary.trim().is_empty() || !chapter.title.trim().is_empty())
                && exact_event_source(
                    &serde_json::json!({
                        "type": "finalized-chapter",
                        "draftId": chapter.draft_id,
                        "chapterNumber": chapter.chapter_number,
                    }),
                    "occurred",
                    chapter.chapter_number,
                    sources,
                    generated_at,
                )
                .is_some()
        })
        .collect();
    let finalized_chapter_numbers: std::collections::HashSet<i64> = finalized_chapters
        .iter()
        .map(|chapter| chapter.chapter_number)
        .collect();
    let mut candidates: Vec<Value> = Vec::new();
    for chapter in &finalized_chapters {
        candidates.push(serde_json::json!({
            "status": "occurred",
            "chapterNumber": chapter.chapter_number,
            "summary": first_nonempty(&[
                chapter.summary.trim(),
                chapter.title.trim(),
            ]),
            "sources": [{
                "type": "finalized-chapter",
                "draftId": chapter.draft_id,
                "chapterNumber": chapter.chapter_number,
            }],
        }));
    }
    for blueprint in sources
        .facts
        .blueprints
        .iter()
        .filter(|blueprint| !finalized_chapter_numbers.contains(&blueprint.chapter_number))
    {
        candidates.push(serde_json::json!({
            "status": "planned",
            "chapterNumber": blueprint.chapter_number,
            "summary": first_nonempty(&[
                blueprint.key_events.trim(),
                blueprint.purpose.trim(),
                blueprint.title.trim(),
            ]),
            "sources": [{
                "type": "blueprint",
                "chapterNumber": blueprint.chapter_number,
            }],
        }));
    }
    for thread in &sources.facts.narrative_threads {
        candidates.push(serde_json::json!({
            "status": "planned",
            "chapterNumber": thread.target_start_chapter,
            "summary": first_nonempty(&[
                thread.author_intent.trim(),
                thread.title.trim(),
            ]),
            "sources": [{
                "type": "narrative-thread",
                "planId": thread.id,
            }],
        }));
        for event in &thread.events {
            candidates.push(serde_json::json!({
                "status": "occurred",
                "chapterNumber": event.chapter_number,
                "summary": first_nonempty(&[
                    event.evidence.trim(),
                    event.reason.trim(),
                ]),
                "sources": [{
                    "type": "narrative-thread",
                    "planId": thread.id,
                    "eventId": event.id,
                    "chapterNumber": event.chapter_number,
                }],
            }));
        }
    }

    let mut seen: Vec<String> = Vec::new();
    let mut sorted_events: Vec<Value> = Vec::new();
    for mut event in candidates {
        let source = event
            .get("sources")
            .and_then(Value::as_array)
            .and_then(|sources| sources.first())
            .cloned()
            .unwrap_or(Value::Null);
        let Some(normalized) = exact_event_source(
            &source,
            event
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            event
                .get("chapterNumber")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            sources,
            generated_at,
        ) else {
            continue;
        };
        let key = serde_json::to_string(&normalized).unwrap_or_default();
        let empty_summary = event
            .get("summary")
            .and_then(Value::as_str)
            .map(|summary| summary.is_empty())
            .unwrap_or(true);
        if empty_summary || seen.contains(&key) {
            continue;
        }
        if let Some(object) = event.as_object_mut() {
            object.insert("sources".to_string(), Value::Array(vec![normalized]));
        }
        seen.push(key);
        sorted_events.push(event);
    }
    sorted_events.sort_by(|left, right| {
        left.get("chapterNumber")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .cmp(
                &right
                    .get("chapterNumber")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
            )
            .then_with(|| {
                left.get("status")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .cmp(
                        right
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    )
            })
            .then_with(|| {
                let left_key = serialized_first_source(left);
                let right_key = serialized_first_source(right);
                left_key.cmp(&right_key)
            })
    });
    if sorted_events.is_empty() {
        return Err(PlotTreeSourceError::new().to_string());
    }
    let start_chapter = sorted_events
        .first()
        .and_then(|event| event.get("chapterNumber"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let end_chapter = sorted_events
        .last()
        .and_then(|event| event.get("chapterNumber"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let track = serde_json::json!({
        "id": "source-backed-progress",
        "title": prompt_language_text(
            &sources.facts.writing_language,
            "剧情进展",
            "Plot progress",
        ),
        "role": "main",
        "startChapter": start_chapter,
        "endChapter": end_chapter,
        "summary": prompt_language_text(
            &sources.facts.writing_language,
            "基于当前章节与叙事线索的只读进展。",
            "Read-only progress from current chapters and narrative threads.",
        ),
        "events": sorted_events,
    });
    strict_snapshot(&Value::Array(vec![track]), sources, generated_at)
}

/// 排序键：事件首个来源的序列化字符串（对齐基线 `JSON.stringify(sources[0])`）。
fn serialized_first_source(event: &Value) -> String {
    event
        .get("sources")
        .and_then(Value::as_array)
        .and_then(|sources| sources.first())
        .map(|source| serde_json::to_string(source).unwrap_or_default())
        .unwrap_or_default()
}

/// 基线 `a.trim() || b.trim() || ...` 的等价物。
fn first_nonempty<'a>(values: &[&'a str]) -> &'a str {
    values
        .iter()
        .copied()
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

/// 基线 `boundedText`：JS 字符串长度（UTF-16 码元）超限时保留首尾并插入省略号。
fn bounded_text(value: &str, maximum: usize) -> String {
    if js_length(value) <= maximum {
        return value.to_string();
    }
    let head = (maximum - 1 + 1) / 2;
    let tail = maximum - head - 1;
    let head_end = utf16_char_boundary(value, head);
    let tail_start = utf16_char_boundary(value, js_length(value) - tail);
    format!("{}…{}", &value[..head_end], &value[tail_start..])
}

/// JS `String.prototype.length`（UTF-16 码元数）。
fn js_length(value: &str) -> usize {
    value.encode_utf16().count()
}

/// 第 `index` 个 UTF-16 码元对应的字节边界（`index` 不得超过码元总数）。
fn utf16_char_boundary(value: &str, index: usize) -> usize {
    let mut count = 0;
    for (offset, _) in value.char_indices() {
        if count == index {
            return offset;
        }
        count += 1;
    }
    value.len()
}

/// 基线 `generationFacts`：注入 prompt 的只读来源摘要（按项目规模收紧上限）。
fn generation_facts(sources: &PlotTreeSourceBundle) -> Value {
    let limits = &PLOT_TREE_SOURCE_LIMITS;
    let large_project = sources.facts.blueprints.len() > limits.large_project_threshold
        || sources.facts.finalized_chapters.len() > limits.large_project_threshold;
    let label_characters = if large_project {
        limits.large_project_label_characters
    } else {
        limits.label_characters
    };
    let detail_characters = if large_project {
        limits.large_project_detail_characters
    } else {
        limits.detail_characters
    };
    let event_detail_characters = if large_project {
        limits.large_project_event_detail_characters
    } else {
        limits.detail_characters
    };
    let synopsis_characters = if large_project {
        limits.large_project_synopsis_characters
    } else {
        limits.synopsis_characters
    };
    serde_json::json!({
        "synopsis": bounded_text(&sources.facts.synopsis.content, synopsis_characters),
        "blueprints": sources.facts.blueprints.iter().map(|blueprint| {
            serde_json::json!({
                "chapterNumber": blueprint.chapter_number,
                "title": bounded_text(&blueprint.title, label_characters),
                "purpose": bounded_text(&blueprint.purpose, detail_characters),
                "keyEvents": bounded_text(&blueprint.key_events, detail_characters),
            })
        }).collect::<Vec<_>>(),
        "finalizedChapters": sources.facts.finalized_chapters.iter().map(|chapter| {
            serde_json::json!({
                "draftId": chapter.draft_id,
                "chapterNumber": chapter.chapter_number,
                "title": bounded_text(&chapter.title, label_characters),
                "summary": bounded_text(&chapter.summary, detail_characters),
            })
        }).collect::<Vec<_>>(),
        "narrativeThreads": sources.facts.narrative_threads.iter().map(|thread| {
            serde_json::json!({
                "id": thread.id,
                "title": bounded_text(&thread.title, label_characters),
                "type": bounded_text(&thread.thread_type, label_characters),
                "targetStartChapter": thread.target_start_chapter,
                "targetEndChapter": thread.target_end_chapter,
                "authorIntent": bounded_text(&thread.author_intent, detail_characters),
                "status": thread.status,
                "events": thread.events.iter().map(|event: &PlotTreeNarrativeThreadEventFact| {
                    serde_json::json!({
                        "id": event.id,
                        "chapterNumber": event.chapter_number,
                        "type": event.event_type,
                        "evidence": bounded_text(&event.evidence, event_detail_characters),
                        "reason": bounded_text(&event.reason, event_detail_characters),
                    })
                }).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
    })
}

/// 基线 `buildPlotTreeTask`：构造剧情树生成任务。
///
/// 无可用事件源 → `NO_EVENT_SOURCES`；蓝图 / 定稿章节超限 →
/// `SOURCE_LIMIT_EXCEEDED`；注入事实超过输入上限 → `SOURCE_INPUT_TOO_LARGE`。
pub fn build_plot_tree_task(sources: &PlotTreeSourceBundle) -> Result<GraphGenerationTask, String> {
    if !has_usable_plot_tree_event_source(&sources.facts) {
        return Err(PlotTreeSourceError::new().to_string());
    }
    if sources.facts.blueprints.len() > PLOT_TREE_SOURCE_LIMITS.blueprints
        || sources.facts.finalized_chapters.len() > PLOT_TREE_SOURCE_LIMITS.finalized_chapters
    {
        return Err(
            PlotTreeSourceLimitError::new(PLOT_TREE_SOURCE_LIMITS.blueprints as i64).to_string(),
        );
    }
    let facts = serde_json::to_string(&generation_facts(sources))
        .map_err(|error| format!("invalid_json: {error}"))?;
    if js_length(&facts) > PLOT_TREE_INPUT_MAX_CHARACTERS {
        return Err(
            PlotTreeInputLimitError::new(PLOT_TREE_INPUT_MAX_CHARACTERS as i64).to_string(),
        );
    }
    let zh_cn_prompt = [
        "你是小说剧情结构编辑。把给定的情节总大纲、章节蓝图、已定稿章节摘要和作者确认的叙事线索归纳为只读剧情树。",
        "区分 main 主线与 subplot 支线；每条支线必须用 parentTrackId 关联一条主线，主线不能有 parentTrackId。planned 只能来自章节蓝图或人工叙事计划，occurred 只能来自已定稿章节或已确认叙事事件。",
        "情节总大纲只用于归纳轨道和摘要，不是可引用来源；绝不能在事件 sources 中引用它，也绝不能输出 source.type=\"synopsis\"。每个事件必须至少引用一个同章节的真实来源，且只能使用以下格式：{\"type\":\"blueprint\",\"chapterNumber\":1}、{\"type\":\"finalized-chapter\",\"draftId\":1,\"chapterNumber\":1}、{\"type\":\"narrative-thread\",\"planId\":1}、{\"type\":\"narrative-thread\",\"planId\":1,\"eventId\":1,\"chapterNumber\":1}。不得编造 ID 或章节。",
        "只输出 JSON 对象：{\"tracks\":[{\"id\":\"stable-id\",\"title\":\"\",\"role\":\"main\",\"startChapter\":1,\"endChapter\":1,\"summary\":\"\",\"events\":[{\"status\":\"planned|occurred\",\"chapterNumber\":1,\"summary\":\"\",\"sources\":[]}]}]}。仅 subplot 轨道增加 parentTrackId。不要输出解释或 Markdown。",
    ]
    .concat();
    let en_us_prompt = [
        "You are a fiction plot-structure editor. Derive a read-only plot tree from the supplied synopsis, chapter blueprints, finalized chapter summaries, and author-confirmed narrative threads.",
        "Separate main tracks from subplot tracks. Every subplot must reference one main track with parentTrackId; main tracks must not have parentTrackId. planned must be supported by a chapter blueprint or human narrative plan; occurred must be supported by a finalized chapter or confirmed narrative event.",
        "The synopsis is context for synthesizing tracks and summaries, not a citable source. Never cite it in event sources and never emit source.type=\"synopsis\". Every event must cite at least one real source for the same chapter using only these forms: {\"type\":\"blueprint\",\"chapterNumber\":1}, {\"type\":\"finalized-chapter\",\"draftId\":1,\"chapterNumber\":1}, {\"type\":\"narrative-thread\",\"planId\":1}, or {\"type\":\"narrative-thread\",\"planId\":1,\"eventId\":1,\"chapterNumber\":1}. Never invent an ID or chapter.",
        "Return only one JSON object: {\"tracks\":[{\"id\":\"stable-id\",\"title\":\"\",\"role\":\"main\",\"startChapter\":1,\"endChapter\":1,\"summary\":\"\",\"events\":[{\"status\":\"planned|occurred\",\"chapterNumber\":1,\"summary\":\"\",\"sources\":[]}]}]}. Add parentTrackId only to subplot tracks. Do not return explanations or Markdown.",
    ]
    .concat();
    Ok(GraphGenerationTask {
        purpose: "plot-tree-snapshot".to_string(),
        reasoning_stage: "planning".to_string(),
        output: "structured-data".to_string(),
        messages: vec![
            GraphGenerationMessage {
                role: "system".to_string(),
                content: prompt_language_text(
                    &sources.facts.writing_language,
                    &zh_cn_prompt,
                    &en_us_prompt,
                )
                .to_string(),
            },
            GraphGenerationMessage {
                role: "user".to_string(),
                content: facts,
            },
        ],
    })
}

/// 基线 `PlotTreeDerivation`：快照及其推导路径（`model` / `normalized` / `deterministic`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlotTreeDerivation {
    pub snapshot: PlotTreeSnapshot,
    pub kind: String,
}

/// 基线 `derivePlotTreeSnapshot`：模型输出 → 快照的三级降级推导。
///
/// JSON 解析失败或严格校验失败时依次尝试规范化与确定性兜底；
/// 确定性兜底也失败（无任何可用事件源）时抛 `NO_EVENT_SOURCES`。
pub fn derive_plot_tree_snapshot(
    content: &str,
    sources: &PlotTreeSourceBundle,
    generated_at: &str,
) -> Result<PlotTreeDerivation, String> {
    let parsed = match parse_plot_tree_json(content) {
        Ok(parsed) => parsed,
        Err(_) => {
            return Ok(PlotTreeDerivation {
                snapshot: deterministic_plot_tree_snapshot(sources, generated_at)?,
                kind: "deterministic".to_string(),
            })
        }
    };
    match strict_snapshot(
        parsed.get("tracks").unwrap_or(&Value::Null),
        sources,
        generated_at,
    ) {
        Ok(snapshot) => Ok(PlotTreeDerivation {
            snapshot,
            kind: "model".to_string(),
        }),
        Err(_) => match normalize_plot_tree_snapshot(&parsed, sources, generated_at) {
            Ok(snapshot) => Ok(PlotTreeDerivation {
                snapshot,
                kind: "normalized".to_string(),
            }),
            Err(_) => Ok(PlotTreeDerivation {
                snapshot: deterministic_plot_tree_snapshot(sources, generated_at)?,
                kind: "deterministic".to_string(),
            }),
        },
    }
}

/// 基线 `writingLanguageText`：`language === 'en-US'` 时取英文，否则取中文。
fn prompt_language_text<'a>(language: &str, zh_cn_text: &'a str, en_us_text: &'a str) -> &'a str {
    if language == "en-US" {
        en_us_text
    } else {
        zh_cn_text
    }
}

/// 基线 `new Date().toISOString()` 的等价物（UTC，毫秒精度）。
pub fn current_iso_timestamp() -> String {
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

/// JS `Number.isSafeInteger`（`i64` 形式）。
fn is_safe_integer(value: i64) -> bool {
    value >= -9_007_199_254_740_991 && value <= 9_007_199_254_740_991
}

/// 构造带一个蓝图来源的最小 `PlotTreeSourceBundle`（测试夹具）。
#[cfg(test)]
fn test_sources(writing_language: &str) -> PlotTreeSourceBundle {
    use crate::plot_tree::{PlotTreeBlueprintFact, PlotTreeSourceFacts, PlotTreeSynopsisFact};
    PlotTreeSourceBundle {
        facts: PlotTreeSourceFacts {
            writing_language: writing_language.to_string(),
            synopsis: PlotTreeSynopsisFact {
                content: "总纲".to_string(),
            },
            blueprints: vec![PlotTreeBlueprintFact {
                chapter_number: 1,
                title: "第一章".to_string(),
                purpose: "开篇".to_string(),
                key_events: "登场".to_string(),
            }],
            finalized_chapters: vec![PlotTreeFinalizedChapterFact {
                draft_id: 1,
                chapter_number: 1,
                title: "第一章".to_string(),
                summary: "正文摘要".to_string(),
            }],
            narrative_threads: Vec::new(),
        },
        source_revision: revision_hex("seed"),
        snapshot: None,
        stored_snapshot_invalid: None,
    }
}

/// 64 位小写 hex 修订号（测试夹具）。
#[cfg(test)]
fn revision_hex(seed: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
fn test_track() -> PlotTreeTrack {
    PlotTreeTrack {
        id: "t1".to_string(),
        title: "主线".to_string(),
        role: "main".to_string(),
        parent_track_id: None,
        start_chapter: 1,
        end_chapter: 1,
        summary: "摘要".to_string(),
        events: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_plot_tree_task_without_sources_test() {
        let sources = PlotTreeSourceBundle {
            facts: crate::plot_tree::PlotTreeSourceFacts {
                writing_language: "zh-CN".to_string(),
                synopsis: crate::plot_tree::PlotTreeSynopsisFact {
                    content: String::new(),
                },
                blueprints: Vec::new(),
                finalized_chapters: Vec::new(),
                narrative_threads: Vec::new(),
            },
            source_revision: revision_hex("seed"),
            snapshot: None,
            stored_snapshot_invalid: None,
        };
        assert_eq!(
            build_plot_tree_task(&sources).unwrap_err(),
            "NO_EVENT_SOURCES"
        );
    }

    #[test]
    fn build_plot_tree_task_contract_test() {
        let sources = test_sources("zh-CN");
        let task = build_plot_tree_task(&sources).unwrap();
        assert_eq!(task.purpose, "plot-tree-snapshot");
        assert_eq!(task.reasoning_stage, "planning");
        assert_eq!(task.output, "structured-data");
        assert_eq!(task.messages.len(), 2);
        assert_eq!(task.messages[0].role, "system");
        assert_eq!(task.messages[1].role, "user");
        // 中文系统提示逐字对齐上游第一行
        assert!(task.messages[0]
            .content
            .starts_with("你是小说剧情结构编辑。把给定的情节总大纲"));
        assert!(task.messages[0]
            .content
            .contains("绝不能在事件 sources 中引用它"));
        // 用户消息为注入事实的 JSON 串
        let facts: Value = serde_json::from_str(&task.messages[1].content).unwrap();
        assert_eq!(facts.get("synopsis").and_then(Value::as_str), Some("总纲"));
        assert_eq!(
            facts
                .get("blueprints")
                .and_then(Value::as_array)
                .and_then(|blueprints| blueprints.first())
                .and_then(|blueprint| blueprint.get("chapterNumber"))
                .and_then(Value::as_i64),
            Some(1)
        );
    }

    #[test]
    fn build_plot_tree_task_english_prompt_test() {
        let sources = test_sources("en-US");
        let task = build_plot_tree_task(&sources).unwrap();
        assert!(task.messages[0]
            .content
            .starts_with("You are a fiction plot-structure editor."));
    }

    #[test]
    fn bounded_text_shortens_long_text_test() {
        // 长度 5，上限 4：head = ceil((4-1)/2) = 2，tail = 4-2-1 = 1
        assert_eq!(bounded_text("abcde", 4), "ab…e");
        assert_eq!(bounded_text("abc", 10), "abc");
    }

    #[test]
    fn parse_plot_tree_snapshot_rejects_invalid_json_test() {
        let sources = test_sources("zh-CN");
        let error =
            parse_plot_tree_snapshot("not json", &sources, Some("2026-01-01T00:00:00.000Z"))
                .unwrap_err();
        assert!(error.starts_with("invalid_json:"));
    }

    #[test]
    fn derive_plot_tree_snapshot_model_path_test() {
        let sources = test_sources("zh-CN");
        let generated_at = "2026-01-01T00:00:00.000Z";
        // 模型输出：引用同章蓝图的合法轨道
        let content = r#"{"tracks":[{"id":"t","title":"主线","role":"main","startChapter":1,"endChapter":1,"summary":"s","events":[{"status":"planned","chapterNumber":1,"summary":"e","sources":[{"type":"blueprint","chapterNumber":1}]}]}]}"#;
        let derivation = derive_plot_tree_snapshot(content, &sources, generated_at).unwrap();
        assert_eq!(derivation.kind, "model");
        assert_eq!(derivation.snapshot.version, 1);
        assert_eq!(derivation.snapshot.generated_at, generated_at);
        assert_eq!(derivation.snapshot.writing_language, "zh-CN");
        assert_eq!(
            derivation.snapshot.source_revision.as_deref(),
            Some(sources.source_revision.as_str())
        );
        assert_eq!(derivation.snapshot.tracks.len(), 1);
    }

    #[test]
    fn derive_plot_tree_snapshot_deterministic_fallback_test() {
        let sources = test_sources("zh-CN");
        let generated_at = "2026-01-01T00:00:00.000Z";
        // 完全无效的模型输出 → 确定性兜底（finalized 章节 1 成为 occurred 事件）
        let derivation = derive_plot_tree_snapshot("{}", &sources, generated_at).unwrap();
        assert_eq!(derivation.kind, "deterministic");
        assert_eq!(derivation.snapshot.tracks.len(), 1);
        let track = &derivation.snapshot.tracks[0];
        assert_eq!(track.id, "source-backed-progress");
        assert_eq!(track.title, "剧情进展");
        assert_eq!(track.role, "main");
        assert!(track.parent_track_id.is_none());
        assert_eq!(track.events.len(), 1);
        assert_eq!(track.events[0].status, "occurred");
        assert_eq!(track.events[0].chapter_number, 1);
        assert_eq!(track.events[0].summary, "正文摘要");
        assert_eq!(track.events[0].sources.len(), 1);
        assert!(matches!(
            track.events[0].sources[0],
            crate::plot_tree::PlotTreeSourceReference::FinalizedChapter {
                draft_id: 1,
                chapter_number: 1
            }
        ));
    }

    #[test]
    fn derive_plot_tree_snapshot_deterministic_english_track_test() {
        let sources = test_sources("en-US");
        let derivation =
            derive_plot_tree_snapshot("{}", &sources, "2026-01-01T00:00:00.000Z").unwrap();
        let track = &derivation.snapshot.tracks[0];
        assert_eq!(track.title, "Plot progress");
        assert_eq!(
            track.summary,
            "Read-only progress from current chapters and narrative threads."
        );
    }

    #[test]
    fn derive_plot_tree_snapshot_normalized_path_test() {
        let sources = test_sources("zh-CN");
        let generated_at = "2026-01-01T00:00:00.000Z";
        // 顶层缺少 startChapter/endChapter → 严格校验失败 → 规范化补齐
        let content = r#"{"tracks":[{"id":"t","title":"主线","role":"main","summary":"s","events":[{"status":"planned","chapterNumber":1,"summary":"e","sources":[{"type":"blueprint","chapterNumber":1}]}]}]}"#;
        let derivation = derive_plot_tree_snapshot(content, &sources, generated_at).unwrap();
        assert_eq!(derivation.kind, "normalized");
        let track = &derivation.snapshot.tracks[0];
        assert_eq!(track.start_chapter, 1);
        assert_eq!(track.end_chapter, 1);
    }

    #[test]
    fn derive_plot_tree_snapshot_rejects_unusable_sources_test() {
        let sources = PlotTreeSourceBundle {
            facts: crate::plot_tree::PlotTreeSourceFacts {
                writing_language: "zh-CN".to_string(),
                synopsis: crate::plot_tree::PlotTreeSynopsisFact {
                    content: String::new(),
                },
                blueprints: Vec::new(),
                finalized_chapters: Vec::new(),
                narrative_threads: Vec::new(),
            },
            source_revision: revision_hex("seed"),
            snapshot: None,
            stored_snapshot_invalid: None,
        };
        assert_eq!(
            derive_plot_tree_snapshot("{}", &sources, "2026-01-01T00:00:00.000Z").unwrap_err(),
            "NO_EVENT_SOURCES"
        );
    }

    #[test]
    fn parse_fenced_json_handles_fence_and_case_test() {
        assert_eq!(
            parse_plot_tree_json("```json\n{\"tracks\": []}\n```").unwrap(),
            serde_json::json!({"tracks": []})
        );
        assert_eq!(
            parse_plot_tree_json("```JSON\n{\"a\":1}\n```").unwrap(),
            serde_json::json!({"a": 1})
        );
        assert_eq!(
            parse_plot_tree_json("{\"a\":1}").unwrap(),
            serde_json::json!({"a": 1})
        );
    }

    #[test]
    fn plot_tree_source_limit_error_display_test() {
        assert_eq!(
            PlotTreeSourceLimitError::new(200).to_string(),
            "SOURCE_LIMIT_EXCEEDED"
        );
        assert_eq!(
            PlotTreeInputLimitError::new(220_000).to_string(),
            "SOURCE_INPUT_TOO_LARGE"
        );
        assert_eq!(PlotTreeSourceError::new().to_string(), "NO_EVENT_SOURCES");
        assert_eq!(
            PlotTreeIncompleteError::new("zh-CN", "length").to_string(),
            "剧情树输出达到模型最大长度，结果未保存，请提高最大输出 Tokens 或缩短项目资料。"
        );
        assert_eq!(
            PlotTreeIncompleteError::new("en-US", "length").to_string(),
            "Plot-tree output reached the model maximum output length and was not saved. Increase maximum output tokens or shorten the project sources."
        );
        assert_eq!(
            PlotTreeIncompleteError::new("zh-CN", "other").finish_reason,
            "other"
        );
    }

    #[test]
    fn budget_constants_test() {
        assert_eq!(PLOT_TREE_GENERATION_BUDGET.max_attempts, 1);
        assert_eq!(
            PLOT_TREE_GENERATION_BUDGET.max_requested_output_tokens,
            8192
        );
        assert_eq!(
            PLOT_TREE_GENERATION_BUDGET.max_requested_output_tokens_per_attempt,
            8192
        );
        assert_eq!(PLOT_TREE_GENERATION_BUDGET.deadline_ms, 600_000);
        assert_eq!(PLOT_TREE_INPUT_MAX_CHARACTERS, 220_000);
    }

    #[test]
    fn deterministic_snapshot_sorts_and_dedupes_test() {
        let mut sources = test_sources("zh-CN");
        sources
            .facts
            .blueprints
            .push(crate::plot_tree::PlotTreeBlueprintFact {
                chapter_number: 2,
                title: "第二章".to_string(),
                purpose: "承接".to_string(),
                key_events: String::new(),
            });
        let derivation =
            derive_plot_tree_snapshot("{}", &sources, "2026-01-01T00:00:00.000Z").unwrap();
        let track = &derivation.snapshot.tracks[0];
        // 第一章 occurred(1) 排在第二章 planned(2) 之前
        assert_eq!(track.events.len(), 2);
        assert_eq!(track.events[0].status, "occurred");
        assert_eq!(track.events[0].chapter_number, 1);
        assert_eq!(track.events[1].status, "planned");
        assert_eq!(track.events[1].chapter_number, 2);
        assert_eq!(track.events[1].summary, "承接");
        assert_eq!(track.start_chapter, 1);
        assert_eq!(track.end_chapter, 2);
    }

    #[test]
    fn deterministic_snapshot_skips_blank_finalized_chapter_test() {
        let mut sources = test_sources("zh-CN");
        sources.facts.finalized_chapters[0].summary = "   ".to_string();
        sources.facts.finalized_chapters[0].title = " ".to_string();
        let derivation =
            derive_plot_tree_snapshot("{}", &sources, "2026-01-01T00:00:00.000Z").unwrap();
        let track = &derivation.snapshot.tracks[0];
        // 定稿章节空白摘要被跳过；蓝图 planned 事件保留
        assert_eq!(track.events.len(), 1);
        assert_eq!(track.events[0].status, "planned");
        assert_eq!(track.events[0].chapter_number, 1);
        assert_eq!(track.events[0].summary, "登场");
    }

    #[test]
    fn exact_event_source_rejects_wrong_chapter_test() {
        let sources = test_sources("zh-CN");
        // 蓝图在章节 1：章节 2 的引用不合法
        assert!(exact_event_source(
            &serde_json::json!({"type": "blueprint", "chapterNumber": 2}),
            "planned",
            2,
            &sources,
            "2026-01-01T00:00:00.000Z",
        )
        .is_none());
        // 章节 1 的蓝图引用合法
        assert!(exact_event_source(
            &serde_json::json!({"type": "blueprint", "chapterNumber": 1}),
            "planned",
            1,
            &sources,
            "2026-01-01T00:00:00.000Z",
        )
        .is_some());
        // synopsis 引用永远非法
        assert!(exact_event_source(
            &serde_json::json!({"type": "synopsis"}),
            "planned",
            1,
            &sources,
            "2026-01-01T00:00:00.000Z",
        )
        .is_none());
    }

    #[test]
    fn current_iso_timestamp_format_test() {
        let timestamp = current_iso_timestamp();
        assert_eq!(timestamp.len(), 24);
        assert!(timestamp.ends_with('Z'));
        assert_eq!(&timestamp[19..20], ".");
    }

    #[test]
    fn unused_track_helper_test() {
        // test_track 夹具的显式使用点（防止未使用告警）
        let track = test_track();
        assert_eq!(track.id, "t1");
        assert_eq!(track.role, "main");
        assert!(track.events.is_empty());
    }
}
