//! Narrative-thread generation candidate parsing / task construction.
//!
//! Byte-for-byte port of upstream `src/shared/narrative-thread-generation-pure.ts`
//! (pure helpers; no DB, no model I/O). Consumed by
//! [`crate::services::graph_generation`] when deriving
//! `GraphGenerationResult::Event` / `GraphGenerationResult::Plan` and by the
//! graph-generation source binder when building per-run tasks.

use crate::repositories::blueprint_repository::BlueprintData;
use crate::repositories::narrative_thread_repository::{
    NarrativeThreadPlanInput, NarrativeThreadView,
};
use crate::services::graph_generation::{GraphGenerationMessage, GraphGenerationTask};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Mirrors upstream `NARRATIVE_THREAD_CANDIDATE_BUDGET`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NarrativeThreadCandidateBudget {
    pub max_attempts: i64,
    pub max_requested_output_tokens: i64,
    pub max_requested_output_tokens_per_attempt: i64,
    pub deadline_ms: i64,
}

pub const NARRATIVE_THREAD_CANDIDATE_BUDGET: NarrativeThreadCandidateBudget =
    NarrativeThreadCandidateBudget {
        max_attempts: 1,
        max_requested_output_tokens: 4096,
        max_requested_output_tokens_per_attempt: 4096,
        deadline_ms: 120_000,
    };

/// Mirrors upstream `MAX_PLAN_CANDIDATES` / `MAX_EVENT_CANDIDATES`.
pub const MAX_PLAN_CANDIDATES: usize = 8;
pub const MAX_EVENT_CANDIDATES: usize = 5;

pub const NARRATIVE_THREAD_CANDIDATES_INVALID: &str = "NARRATIVE_THREAD_CANDIDATES_INVALID";

/// Plan candidate is the repository plan input verbatim (upstream type alias).
pub type NarrativeThreadPlanCandidate = NarrativeThreadPlanInput;

/// Event candidate extracted from a finalized chapter (upstream
/// `NarrativeThreadEventCandidate`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NarrativeThreadEventCandidate {
    #[serde(rename = "type")]
    pub event_type: String,
    pub evidence: String,
    pub reason: String,
}

/// Input for [`build_narrative_thread_plan_task`] — upstream
/// `Omit<GenerateNarrativeThreadPlanCandidateInput, 'modelId' | 'signal'>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NarrativeThreadPlanTaskInput {
    pub writing_language: String,
    pub total_chapters: i64,
    pub blueprint: BlueprintData,
}

/// Input for [`build_narrative_thread_event_task`] — upstream
/// `Omit<GenerateNarrativeThreadEventCandidateInput, 'modelId' | 'signal'>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NarrativeThreadEventTaskInput {
    pub writing_language: String,
    pub plan: NarrativeThreadView,
    pub draft_id: i64,
    pub chapter_number: i64,
    pub finalized_content: String,
}

/// Parse plan candidates (upstream `parseNarrativeThreadPlanCandidates`).
///
/// `legacy_stored_effect` mirrors the stored-effect recovery path: invalid
/// candidates are skipped instead of failing the whole parse.
pub fn parse_narrative_thread_plan_candidates(
    content: &str,
    total_chapters: i64,
    legacy_stored_effect: bool,
) -> Result<Vec<NarrativeThreadPlanCandidate>, String> {
    let raw = candidates_from_json(content, MAX_PLAN_CANDIDATES, legacy_stored_effect)?;
    let mut candidates = Vec::new();
    for candidate in raw {
        let Some(value) = record(&candidate) else {
            return invalid_candidates(legacy_stored_effect);
        };
        let title = bounded_text(value.get("title").unwrap_or(&Value::Null), 120);
        let event_type = bounded_text(value.get("type").unwrap_or(&Value::Null), 60);
        let author_intent = bounded_text(value.get("authorIntent").unwrap_or(&Value::Null), 1000);
        let target_start_chapter = value.get("targetStartChapter").and_then(Value::as_i64);
        let target_end_chapter = value.get("targetEndChapter").and_then(Value::as_i64);
        let valid = title.is_some()
            && event_type.is_some()
            && author_intent.is_some()
            && target_start_chapter
                .map(|chapter| {
                    is_safe_integer(chapter) && chapter >= 1 && chapter <= total_chapters
                })
                .unwrap_or(false)
            && target_end_chapter
                .map(|chapter| {
                    is_safe_integer(chapter)
                        && chapter >= target_start_chapter.unwrap_or_default()
                        && chapter <= total_chapters
                })
                .unwrap_or(false);
        if !valid {
            return invalid_candidates(legacy_stored_effect);
        }
        candidates.push(NarrativeThreadPlanInput {
            title: title.unwrap_or_default(),
            thread_type: event_type.unwrap_or_default(),
            target_start_chapter: target_start_chapter.unwrap_or_default(),
            target_end_chapter: target_end_chapter.unwrap_or_default(),
            author_intent: author_intent.unwrap_or_default(),
        });
    }
    Ok(candidates)
}

/// Parse event candidates (upstream `parseNarrativeThreadEventCandidates`).
///
/// Evidence must be a verbatim excerpt of the finalized content (compared with
/// all ECMAScript whitespace stripped from both sides).
pub fn parse_narrative_thread_event_candidates(
    content: &str,
    finalized_content: &str,
    legacy_stored_effect: bool,
) -> Result<Vec<NarrativeThreadEventCandidate>, String> {
    let normalized_source = strip_js_whitespace(finalized_content);
    let raw = candidates_from_json(content, MAX_EVENT_CANDIDATES, legacy_stored_effect)?;
    let mut candidates = Vec::new();
    for candidate in raw {
        let Some(value) = record(&candidate) else {
            return invalid_candidates(legacy_stored_effect);
        };
        let type_value = value.get("type").unwrap_or(&Value::Null);
        let type_string = js_string(type_value);
        if (!legacy_stored_effect && !type_value.is_string())
            || !["planted", "progressing", "resolved", "abandoned"].contains(&type_string.as_str())
        {
            return invalid_candidates(legacy_stored_effect);
        }
        let evidence = bounded_text(value.get("evidence").unwrap_or(&Value::Null), 240);
        let reason = bounded_text(value.get("reason").unwrap_or(&Value::Null), 500);
        let evidence_normalized = evidence
            .as_deref()
            .map(strip_js_whitespace)
            .unwrap_or_default();
        if evidence.is_none()
            || reason.is_none()
            || !normalized_source.contains(&evidence_normalized)
        {
            return invalid_candidates(legacy_stored_effect);
        }
        candidates.push(NarrativeThreadEventCandidate {
            event_type: type_string,
            evidence: evidence.unwrap_or_default(),
            reason: reason.unwrap_or_default(),
        });
    }
    Ok(candidates)
}

/// Build the plan-candidate generation task (upstream
/// `buildNarrativeThreadPlanTask`).
pub fn build_narrative_thread_plan_task(
    input: NarrativeThreadPlanTaskInput,
) -> GraphGenerationTask {
    let total_chapters = input.total_chapters;
    let zh_cn_prompt = format!(
        "你是小说结构编辑。只从章节蓝图提出可供作者确认的伏笔与叙事线索计划，不得声称正文事件已经发生。所有章节号必须是 1..{total_chapters} 范围内的整数。优先提出 3–8 条真正有用的候选；不足 3 条时不要凑数。只输出 JSON 对象：{{\"candidates\":[{{\"title\":\"\",\"type\":\"\",\"targetStartChapter\":1,\"targetEndChapter\":1,\"authorIntent\":\"\"}}]}}。最多 8 项。",
    );
    let en_us_prompt = format!(
        "You are a fiction structure editor. Propose foreshadowing and narrative-thread plans from the chapter blueprint for author confirmation. Never claim that a manuscript event has occurred. Every chapter number must be an integer within 1..{total_chapters}. Prefer 3–8 genuinely useful candidates; do not pad the list when fewer than three are justified. Return only one JSON object: {{\"candidates\":[{{\"title\":\"\",\"type\":\"\",\"targetStartChapter\":1,\"targetEndChapter\":1,\"authorIntent\":\"\"}}]}}. Maximum 8 items.",
    );
    let user_content = serde_json::json!({
        "totalChapters": input.total_chapters,
        "blueprint": input.blueprint,
    });
    GraphGenerationTask {
        purpose: "narrative-thread-plan-candidate".to_string(),
        reasoning_stage: "planning".to_string(),
        output: "structured-data".to_string(),
        messages: vec![
            GraphGenerationMessage {
                role: "system".to_string(),
                content: prompt_language_text(
                    &input.writing_language,
                    &zh_cn_prompt,
                    &en_us_prompt,
                )
                .to_string(),
            },
            GraphGenerationMessage {
                role: "user".to_string(),
                content: user_content.to_string(),
            },
        ],
    }
}

/// Build the event-candidate generation task (upstream
/// `buildNarrativeThreadEventTask`).
pub fn build_narrative_thread_event_task(
    input: NarrativeThreadEventTaskInput,
) -> GraphGenerationTask {
    let zh_cn_prompt = "你是小说定稿事实审查员。只判断给定已定稿章节是否推进了给定叙事线索。证据必须是正文中逐字出现、最多 240 字的短摘录。只输出 JSON 对象：{\"candidates\":[{\"type\":\"planted|progressing|resolved|abandoned\",\"evidence\":\"\",\"reason\":\"\"}]}。最多 5 项，不得输出计划 ID、草稿 ID 或章节号。";
    let en_us_prompt = "You review finalized fiction facts. Decide only whether the supplied finalized chapter advances the supplied narrative thread. Evidence must be a verbatim excerpt of at most 240 characters from the manuscript. Return only one JSON object: {\"candidates\":[{\"type\":\"planted|progressing|resolved|abandoned\",\"evidence\":\"\",\"reason\":\"\"}]}. Maximum 5 items. Do not output plan IDs, draft IDs, or chapter numbers.";
    let user_content = serde_json::json!({
        "chapterNumber": input.chapter_number,
        "plan": {
            "title": input.plan.title,
            "type": input.plan.thread_type,
            "targetStartChapter": input.plan.target_start_chapter,
            "targetEndChapter": input.plan.target_end_chapter,
            "authorIntent": input.plan.author_intent,
            "currentStatus": input.plan.status,
        },
        "finalizedContent": input.finalized_content,
    });
    GraphGenerationTask {
        purpose: "narrative-thread-event-candidate".to_string(),
        reasoning_stage: "review".to_string(),
        output: "structured-data".to_string(),
        messages: vec![
            GraphGenerationMessage {
                role: "system".to_string(),
                content: prompt_language_text(&input.writing_language, zh_cn_prompt, en_us_prompt)
                    .to_string(),
            },
            GraphGenerationMessage {
                role: "user".to_string(),
                content: user_content.to_string(),
            },
        ],
    }
}

/// Upstream `promptLanguageText` — `en-US` selects the English prompt.
fn prompt_language_text<'a>(language: &str, zh_cn: &'a str, en_us: &'a str) -> &'a str {
    if language == "en-US" {
        en_us
    } else {
        zh_cn
    }
}

/// Upstream `record` — a JSON object, or `None`.
fn record(value: &Value) -> Option<&Value> {
    if value.is_object() {
        Some(value)
    } else {
        None
    }
}

/// Upstream `candidatesFromJson` — parse, validate the wrapper shape, cap at
/// `limit`. JSON syntax errors propagate as an invalid-candidates error
/// regardless of `legacy_stored_effect` (upstream lets `JSON.parse` throw).
fn candidates_from_json(
    content: &str,
    limit: usize,
    legacy_stored_effect: bool,
) -> Result<Vec<Value>, String> {
    let parsed: Value =
        serde_json::from_str(content.trim()).map_err(|_| invalid_candidates_error())?;
    let Some(parsed) = record(&parsed) else {
        return invalid_candidates(legacy_stored_effect);
    };
    let Some(raw_candidates) = parsed.get("candidates").and_then(Value::as_array) else {
        return invalid_candidates(legacy_stored_effect);
    };
    if !legacy_stored_effect && raw_candidates.len() > limit {
        return invalid_candidates(false);
    }
    Ok(raw_candidates.iter().take(limit).cloned().collect())
}

/// Upstream `invalidCandidates` — throws for current-effect parsing, yields an
/// empty list for the legacy stored-effect recovery path.
fn invalid_candidates<T>(legacy_stored_effect: bool) -> Result<Vec<T>, String> {
    if !legacy_stored_effect {
        return Err(invalid_candidates_error());
    }
    Ok(Vec::new())
}

fn invalid_candidates_error() -> String {
    NARRATIVE_THREAD_CANDIDATES_INVALID.to_string()
}

/// Upstream `boundedText` — trimmed non-empty strings within the JS
/// (UTF-16) length budget; anything else is `None`.
fn bounded_text(value: &Value, max_length: usize) -> Option<String> {
    let text = value.as_str()?.trim();
    if !text.is_empty() && js_length(text) <= max_length {
        Some(text.to_string())
    } else {
        None
    }
}

/// JS `String.length` — UTF-16 code units.
fn js_length(value: &str) -> usize {
    value.encode_utf16().count()
}

/// JS `String(value)` coercion used for the event `type` membership check.
fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        _ => "[object Object]".to_string(),
    }
}

/// Upstream `finalizedContent.replace(/\s+/gu, '')` — ECMAScript whitespace set
/// (with the `u` flag: includes `\uFEFF` and the Unicode space separators).
fn strip_js_whitespace(value: &str) -> String {
    value.chars().filter(|c| !is_js_whitespace(*c)).collect()
}

fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\u{000B}' | '\u{000C}' | '\r' | '\n' | ' ' | '\u{00A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// JS `Number.isSafeInteger`.
fn is_safe_integer(value: i64) -> bool {
    value.abs() <= 9_007_199_254_740_991
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blueprint_fixture() -> BlueprintData {
        BlueprintData {
            chapter_number: 3,
            title: "第三章".to_string(),
            role: "主角".to_string(),
            purpose: "铺设伏笔".to_string(),
            key_events: "关键事件".to_string(),
            characters: vec!["林默".to_string()],
            new_character_candidates: None,
            relationship_hints: None,
            suspense_hook: "钩子".to_string(),
            user_guidance: "指导".to_string(),
            notes: "备注".to_string(),
            notes_updated_at: "2026-01-01T00:00:00.000Z".to_string(),
        }
    }

    fn thread_view_fixture() -> NarrativeThreadView {
        NarrativeThreadView {
            id: 7,
            title: "线索".to_string(),
            thread_type: "foreshadowing".to_string(),
            target_start_chapter: 2,
            target_end_chapter: 6,
            author_intent: "意图".to_string(),
            status: "active".to_string(),
            dormant_chapters: 0,
            overdue: false,
            events: Vec::new(),
            created_at: "2026-01-01T00:00:00.000Z".to_string(),
            updated_at: "2026-01-01T00:00:00.000Z".to_string(),
        }
    }

    #[test]
    fn budget_constants_match_upstream() {
        assert_eq!(NARRATIVE_THREAD_CANDIDATE_BUDGET.max_attempts, 1);
        assert_eq!(
            NARRATIVE_THREAD_CANDIDATE_BUDGET.max_requested_output_tokens,
            4096
        );
        assert_eq!(
            NARRATIVE_THREAD_CANDIDATE_BUDGET.max_requested_output_tokens_per_attempt,
            4096
        );
        assert_eq!(NARRATIVE_THREAD_CANDIDATE_BUDGET.deadline_ms, 120_000);
        assert_eq!(MAX_PLAN_CANDIDATES, 8);
        assert_eq!(MAX_EVENT_CANDIDATES, 5);
    }

    #[test]
    fn parse_plan_candidates_accepts_valid_payload() {
        let content = r#"{"candidates":[{"title":"伏笔","type":"foreshadowing","targetStartChapter":2,"targetEndChapter":4,"authorIntent":"意图"}]}"#;
        let candidates = parse_narrative_thread_plan_candidates(content, 10, false).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].title, "伏笔");
        assert_eq!(candidates[0].thread_type, "foreshadowing");
        assert_eq!(candidates[0].target_start_chapter, 2);
        assert_eq!(candidates[0].target_end_chapter, 4);
        assert_eq!(candidates[0].author_intent, "意图");
    }

    #[test]
    fn parse_plan_candidates_rejects_out_of_range_chapters() {
        let content = r#"{"candidates":[{"title":"伏笔","type":"foreshadowing","targetStartChapter":0,"targetEndChapter":4,"authorIntent":"意图"}]}"#;
        assert_eq!(
            parse_narrative_thread_plan_candidates(content, 10, false).unwrap_err(),
            NARRATIVE_THREAD_CANDIDATES_INVALID
        );
        // Legacy recovery path skips the invalid candidate instead.
        assert!(parse_narrative_thread_plan_candidates(content, 10, true)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn parse_plan_candidates_enforces_end_after_start() {
        let content = r#"{"candidates":[{"title":"伏笔","type":"foreshadowing","targetStartChapter":5,"targetEndChapter":3,"authorIntent":"意图"}]}"#;
        assert_eq!(
            parse_narrative_thread_plan_candidates(content, 10, false).unwrap_err(),
            NARRATIVE_THREAD_CANDIDATES_INVALID
        );
    }

    #[test]
    fn parse_plan_candidates_enforces_total_cap() {
        let mut candidates = Vec::new();
        for index in 0..9 {
            candidates.push(serde_json::json!({
                "title": format!("伏笔{index}"),
                "type": "foreshadowing",
                "targetStartChapter": 1,
                "targetEndChapter": 2,
                "authorIntent": "意图",
            }));
        }
        let content = serde_json::json!({ "candidates": candidates }).to_string();
        assert_eq!(
            parse_narrative_thread_plan_candidates(&content, 10, false).unwrap_err(),
            NARRATIVE_THREAD_CANDIDATES_INVALID
        );
        // Legacy path silently truncates at the limit.
        assert_eq!(
            parse_narrative_thread_plan_candidates(&content, 10, true)
                .unwrap()
                .len(),
            MAX_PLAN_CANDIDATES
        );
    }

    #[test]
    fn parse_plan_candidates_rejects_non_array_wrapper() {
        assert_eq!(
            parse_narrative_thread_plan_candidates(r#"{"candidates":{}}"#, 10, false).unwrap_err(),
            NARRATIVE_THREAD_CANDIDATES_INVALID
        );
        assert_eq!(
            parse_narrative_thread_plan_candidates(r#"[]"#, 10, false).unwrap_err(),
            NARRATIVE_THREAD_CANDIDATES_INVALID
        );
        assert_eq!(
            parse_narrative_thread_plan_candidates("not json", 10, false).unwrap_err(),
            NARRATIVE_THREAD_CANDIDATES_INVALID
        );
    }

    #[test]
    fn parse_event_candidates_accepts_verbatim_evidence() {
        let content =
            r#"{"candidates":[{"type":"progressing","evidence":"短摘录","reason":"理由"}]}"#;
        let candidates =
            parse_narrative_thread_event_candidates(content, "正文 短摘录 正文", false).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].event_type, "progressing");
        assert_eq!(candidates[0].evidence, "短摘录");
        assert_eq!(candidates[0].reason, "理由");
    }

    #[test]
    fn parse_event_candidates_compares_whitespace_stripped() {
        let content = r#"{"candidates":[{"type":"planted","evidence":"a b","reason":"r"}]}"#;
        // Source contains "a\nb" — equal after stripping all JS whitespace.
        let candidates =
            parse_narrative_thread_event_candidates(content, "x a\nb x", false).unwrap();
        assert_eq!(candidates.len(), 1);
        // And a source without the excerpt still fails.
        assert_eq!(
            parse_narrative_thread_event_candidates(content, "no match here", false).unwrap_err(),
            NARRATIVE_THREAD_CANDIDATES_INVALID
        );
    }

    #[test]
    fn parse_event_candidates_rejects_unknown_type() {
        let content = r#"{"candidates":[{"type":"unknown","evidence":"a","reason":"r"}]}"#;
        assert_eq!(
            parse_narrative_thread_event_candidates(content, "a", false).unwrap_err(),
            NARRATIVE_THREAD_CANDIDATES_INVALID
        );
        assert!(parse_narrative_thread_event_candidates(content, "a", true)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn parse_event_candidates_rejects_non_string_type_when_not_legacy() {
        let content = r#"{"candidates":[{"type":1,"evidence":"a","reason":"r"}]}"#;
        assert_eq!(
            parse_narrative_thread_event_candidates(content, "a", false).unwrap_err(),
            NARRATIVE_THREAD_CANDIDATES_INVALID
        );
    }

    #[test]
    fn parse_event_candidates_rejects_empty_evidence_or_reason() {
        let content = r#"{"candidates":[{"type":"planted","evidence":"","reason":"r"}]}"#;
        assert_eq!(
            parse_narrative_thread_event_candidates(content, "a", false).unwrap_err(),
            NARRATIVE_THREAD_CANDIDATES_INVALID
        );
        let content = r#"{"candidates":[{"type":"planted","evidence":"a","reason":""}]}"#;
        assert_eq!(
            parse_narrative_thread_event_candidates(content, "a", false).unwrap_err(),
            NARRATIVE_THREAD_CANDIDATES_INVALID
        );
    }

    #[test]
    fn build_plan_task_interpolates_total_chapters_and_blueprint() {
        let task = build_narrative_thread_plan_task(NarrativeThreadPlanTaskInput {
            writing_language: "zh-CN".to_string(),
            total_chapters: 10,
            blueprint: blueprint_fixture(),
        });
        assert_eq!(task.purpose, "narrative-thread-plan-candidate");
        assert_eq!(task.reasoning_stage, "planning");
        assert_eq!(task.output, "structured-data");
        assert_eq!(task.messages.len(), 2);
        assert_eq!(task.messages[0].role, "system");
        assert_eq!(task.messages[1].role, "user");
        assert!(task.messages[0].content.contains("1..10 范围内的整数"));
        assert!(task.messages[0]
            .content
            .contains(r#""targetStartChapter":1,"targetEndChapter":1"#));
        let user: Value = serde_json::from_str(&task.messages[1].content).unwrap();
        assert_eq!(user["totalChapters"], 10);
        assert_eq!(user["blueprint"]["title"], "第三章");
        assert_eq!(user["blueprint"]["chapterNumber"], 3);
    }

    #[test]
    fn build_plan_task_selects_english_prompt_for_en_us() {
        let task = build_narrative_thread_plan_task(NarrativeThreadPlanTaskInput {
            writing_language: "en-US".to_string(),
            total_chapters: 5,
            blueprint: blueprint_fixture(),
        });
        assert!(task.messages[0].content.contains("within 1..5"));
        assert!(task.messages[0].content.contains("Maximum 8 items"));
    }

    #[test]
    fn build_event_task_serializes_plan_and_finalized_content() {
        let task = build_narrative_thread_event_task(NarrativeThreadEventTaskInput {
            writing_language: "zh-CN".to_string(),
            plan: thread_view_fixture(),
            draft_id: 42,
            chapter_number: 5,
            finalized_content: "定稿正文".to_string(),
        });
        assert_eq!(task.purpose, "narrative-thread-event-candidate");
        assert_eq!(task.reasoning_stage, "review");
        assert_eq!(task.output, "structured-data");
        assert_eq!(task.messages.len(), 2);
        assert!(task.messages[0].content.contains("最多 240 字的短摘录"));
        assert!(task.messages[0]
            .content
            .contains("不得输出计划 ID、草稿 ID 或章节号"));
        let user: Value = serde_json::from_str(&task.messages[1].content).unwrap();
        assert_eq!(user["chapterNumber"], 5);
        assert_eq!(user["plan"]["title"], "线索");
        assert_eq!(user["plan"]["type"], "foreshadowing");
        assert_eq!(user["plan"]["targetStartChapter"], 2);
        assert_eq!(user["plan"]["targetEndChapter"], 6);
        assert_eq!(user["plan"]["authorIntent"], "意图");
        assert_eq!(user["plan"]["currentStatus"], "active");
        assert_eq!(user["finalizedContent"], "定稿正文");
    }

    #[test]
    fn build_event_task_selects_english_prompt_for_en_us() {
        let task = build_narrative_thread_event_task(NarrativeThreadEventTaskInput {
            writing_language: "en-US".to_string(),
            plan: thread_view_fixture(),
            draft_id: 42,
            chapter_number: 5,
            finalized_content: "finalized".to_string(),
        });
        assert!(task.messages[0]
            .content
            .contains("verbatim excerpt of at most 240 characters"));
        assert!(task.messages[0].content.contains("Maximum 5 items"));
    }

    #[test]
    fn js_string_coercion_matches_javascript_semantics() {
        assert_eq!(js_string(&Value::Null), "null");
        assert_eq!(js_string(&Value::Bool(true)), "true");
        assert_eq!(js_string(&serde_json::json!(1.5)), "1.5");
        assert_eq!(js_string(&serde_json::json!("text")), "text");
        assert_eq!(js_string(&serde_json::json!({})), "[object Object]");
    }
}
