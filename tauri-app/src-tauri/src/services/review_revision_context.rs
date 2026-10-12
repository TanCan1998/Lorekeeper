//! 审稿修稿上下文捕获（批次 G4-3b）——行为等价平移自
//! `electron/services/review-revision-context.ts`（154 行）。
//!
//! 基线捕获的 `ReviewRevisionContext` 是**会话无关**的纯函数产物：
//! 仅项目 id 进入来源身份，会话租约（`epoch`）由使用方在构造
//! `SourceRef` 时按当前会话补上（见 `source_ref` / `generation_source_binding`
//! 的跨会话比较规则）。捕获结果同时存入运行账本
//! `sourceManifest.reviewRevisionContext` 并返回渲染层做准入。
//!
//! # 共享纯函数随调用图平移（私有项，待后续共享逻辑移植波次抽公共模块）
//!
//! 基线依赖的共享纯函数在 Rust 侧尚未建立公共模块，本波随调用图一并
//! 平移为本文件私有项（对齐 G4-3a `review_cycle_repository.rs` 先例）：
//!
//! - `human-confirmed-review.ts`：`serializeHumanConfirmedReviewSnapshot`
//!   （`parseHumanConfirmedReviewSnapshot` 已在 G4-3a 平移为
//!   `review_cycle_repository` 私有项；本文件需要**写**回快照，故在此
//!   另译 `serialize` 段；`HumanConfirmedReviewSnapshot` 等类型在
//!   `review_cycle_repository` 中为私有，故本文件另译同形类型，
//!   序列化字节与契约逐字段一致）。
//! - `chapter-goal-review.ts`：`freezeChapterGoals`。
//! - `consistency-preflight.ts`：`findBlueprintContinuityRisks`
//!   （含 `continuityStableFactKey` / `isExplicitTerminalSubject` /
//!   `normalizedKeyPart` / `escapeRegExp`）。
//! - `character-roster.ts`：`CHARACTER_STATE_TEXT_FIELDS` 常量。
//! - `generation-run-repository.ts`：`textHash`（G4-3a 已在
//!   `review_cycle_repository` 另译 `hash`；此处为独立副本）。
//!
//! # 刻意偏离（均有上游语义理由）
//!
//! - **请求键集校验**：基线在 `captureReviewRevisionContext` 入口用
//!   `Object.keys(request).some(...)` 拒绝未知键（`fail()` 抛
//!   `GENERATION_REVIEW_CONTEXT_INVALID`）。Rust 侧以
//!   `#[serde(deny_unknown_fields)]` 在反序列化期等价拒绝；拒绝时
//!   错误消息为 serde 反序列化错误（而非基线错误码），但「拒绝未知键」
//!   的准入语义逐字一致。其余运行时校验（操作域 / draftId / locale /
//!   authorInputs / 互斥 / 重查）仍在函数体内逐字执行并抛基线错误码。
//! - **事务**：基线 `captureReviewRevisionContext` 以
//!   `db.transaction(fn)()` 包裹（调用方 `prepare` 已在外层开事务时即
//!   保存点）。本文件捕获全程只读，方法体直接执行，由调用方
//!   （`prepare`）按基线语义开事务；只读捕获在自动提交模式下逐字段等价。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::draft_source_guard::ExpectedDraftSource;
use crate::repositories::blueprint_repository::{get_all as get_all_blueprints, BlueprintData};
use crate::repositories::character_repository::{
    get_all as get_all_characters, CharacterStateFieldProvenance, FinalizedSourceIdentity,
};
use crate::repositories::consistency_exemption_repository::list as list_exemptions;
use crate::repositories::finalized_continuity_repository::{
    list_finalized_continuity_before, FinalizedContinuityProjection,
};
use crate::repositories::project_core_repository::get as get_project_core;
use crate::repositories::review_cycle_repository::{plan_recheck, ReviewCycleRecheckContext};
use crate::repositories::review_repository::get_full as get_review_full;

// ===== 基线常量 =====

/// 基线 `REVIEW_REVISION_OPERATIONS = ['review-chapter', 'refine-draft', 'refine-from-review']`
pub const REVIEW_REVISION_OPERATIONS: [&str; 3] =
    ["review-chapter", "refine-draft", "refine-from-review"];

/// 基线 `CHARACTER_STATE_TEXT_FIELDS`（`src/shared/character-roster.ts`）
const CHARACTER_STATE_TEXT_FIELDS: [&str; 6] = [
    "location",
    "powerLevel",
    "physicalState",
    "mentalState",
    "keyItems",
    "recentEvents",
];

/// 基线 `fail = () => { throw new Error('GENERATION_REVIEW_CONTEXT_INVALID') }`
fn invalid<T>() -> Result<T, String> {
    Err("GENERATION_REVIEW_CONTEXT_INVALID".to_string())
}

/// SHA-256 十六进制摘要（对齐基线 `textHash` / `createHash('sha256')`）
fn text_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

// ===== 请求 / 上下文契约类型（字段顺序对齐基线对象字面量构造序）=====

/// 基线 `ReviewRevisionOperation`（`'review-chapter' | 'refine-draft' | 'refine-from-review'`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewRevisionOperation {
    ReviewChapter,
    RefineDraft,
    RefineFromReview,
}

impl ReviewRevisionOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReviewChapter => "review-chapter",
            Self::RefineDraft => "refine-draft",
            Self::RefineFromReview => "refine-from-review",
        }
    }
}

/// 基线 `PrepareReviewRevisionRequest.expectedDraft`
/// （`{ chapterNumber, version, status, contentHash }`——不含 `id` / `content`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpectedDraftRef {
    pub chapter_number: i64,
    pub version: i64,
    pub status: String,
    pub content_hash: String,
}

/// 基线 `PrepareReviewRevisionRequest`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareReviewRevisionRequest {
    pub operation: ReviewRevisionOperation,
    pub draft_id: i64,
    pub expected_draft: ExpectedDraftRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_source_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_review_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_cycle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_merged_hash: Option<String>,
    pub author_inputs: Vec<crate::repositories::generation_run_repository::GenerationAuthorInput>,
    pub ui_locale: String,
}

/// 基线 `ReviewMaterialIdentity`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewMaterialIdentity {
    pub project_id: String,
    pub source_id: String,
    pub revision: i64,
    /// 基线注释：`SHA-256 of the unmodified UTF-8 bytes of the material source.`
    pub content_hash: String,
    pub provenance: String,
}

/// 基线 `ReviewFinalizedMaterial`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFinalizedMaterial {
    pub draft_id: i64,
    pub chapter_number: i64,
    pub chapter_title: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<FinalizedSourceIdentity>,
    /// 基线注释：`Only current projections may supply derived facts; other rows supply original prose.`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection: Option<ContinuityProjectionView>,
    /// 基线注释：`主进程捕获的来源身份；渲染层据此做准入，不据此编造事实。`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<ReviewMaterialIdentity>,
}

/// 基线 `NovelConfig`（字段顺序对齐基线构造序：
/// `genre, subGenre, targetAudience, totalChapters, wordsPerChapter, writingLanguage,
/// creativeStrategy, narrativeThreadDormantChapterThreshold, plotStructure, narrativePOV,
/// writingStyle, referenceWorks, globalGuidance, goldenFinger, coreOutline, worldSetting, protagonistProfile`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NovelConfig {
    pub genre: String,
    pub sub_genre: String,
    pub target_audience: String,
    pub total_chapters: i64,
    pub words_per_chapter: i64,
    pub writing_language: String,
    pub creative_strategy: String,
    pub narrative_thread_dormant_chapter_threshold: i64,
    pub plot_structure: String,
    pub narrative_pov: String,
    pub writing_style: String,
    pub reference_works: String,
    pub global_guidance: String,
    pub golden_finger: String,
    pub core_outline: String,
    pub world_setting: String,
    pub protagonist_profile: String,
}

/// 基线 `BlueprintForPreflight`（`consistency-preflight.ts`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintForPreflight {
    pub chapter_number: i64,
    pub title: String,
    pub role: String,
    pub purpose: String,
    pub key_events: String,
    pub characters: Vec<String>,
    pub suspense_hook: String,
    pub user_guidance: String,
    pub notes: String,
}

/// 基线 `ChapterGoalEvidence`（`chapter-goal-review.ts`）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterGoalEvidence {
    pub quote: String,
    pub start: i64,
    pub end: i64,
}

/// 基线 `ChapterGoalReviewItem`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterGoalReviewItem {
    pub id: String,
    pub text: String,
    pub status: String,
    pub description: String,
    pub evidence: Vec<ChapterGoalEvidence>,
}

/// 基线 `ChapterGoalReview`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterGoalReview {
    pub version: i64,
    pub chapter_number: i64,
    pub coverage: String,
    pub items: Vec<ChapterGoalReviewItem>,
}

/// 基线 `FrozenChapterGoals` 中不可变的 `{ id, text }` 项
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenChapterGoalItem {
    pub id: String,
    pub text: String,
}

/// 基线 `FrozenChapterGoals`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenChapterGoals {
    pub chapter_number: i64,
    pub coverage: String,
    pub items: Vec<FrozenChapterGoalItem>,
}

/// 基线 `ConsistencyFinding`（`consistency-preflight.ts`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsistencyFinding {
    pub stable_fact_key: String,
    pub severity: String,
    pub source_chapter: i64,
    pub evidence: String,
    pub issue: LocaleText,
    pub suggestion: LocaleText,
}

/// 基线 `{ zhCN, enUS }` 双语文本
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocaleText {
    pub zh_cn: String,
    pub en_us: String,
}

/// `FindingSpan` 的同形视图（`review_cycle_repository` 的同名类型为
/// 私有 `Serialize` 项；本文件另译带 `Deserialize` 的同形类型，
/// 字段序与序列化字节逐字一致）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingSpanView {
    pub start: i64,
    pub end: i64,
    pub unit: String,
}

/// `ReviewCycleRecheckFinding` 的同形视图（理由同上；
/// `kind` 复用 `review_cycle_repository` 的公开 `FindingKind`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecheckFindingView {
    pub finding_id: String,
    pub target_id: String,
    pub category: String,
    pub kind: crate::repositories::review_cycle_repository::FindingKind,
    pub problem: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<FindingSpanView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub must_show: Option<bool>,
}

/// `ReviewCycleRecheckContext` 的同形视图（理由同上）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecheckContextView {
    pub version: i64,
    pub cycle_id: String,
    pub comparison_version: i64,
    pub merged_hash: String,
    pub finding_set_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_content: Option<String>,
    pub findings: Vec<RecheckFindingView>,
}

/// `FinalizedContinuityProjection` 的同形视图（`finalized_continuity_repository`
/// 的同名类型仅派生 `Serialize`；本文件另译带 `Deserialize` 的同形类型。
/// 字段类型均已派生 `Deserialize`，字段序与原类型逐字一致）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinuityProjectionView {
    pub draft_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_finalized_draft_id: Option<i64>,
    pub chapter_number: i64,
    pub chapter_title: String,
    pub chapter_notes: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facts:
        Option<Vec<crate::repositories::finalized_continuity_repository::FinalizedContinuityFact>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_state_candidates: Option<
        Vec<crate::repositories::finalized_continuity_repository::FinalizedCharacterStateCandidate>,
    >,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<FinalizedSourceIdentity>,
    pub source_status: String,
}

impl PartialEq for ContinuityProjectionView {
    fn eq(&self, other: &Self) -> bool {
        self.draft_id == other.draft_id
            && self.current_finalized_draft_id == other.current_finalized_draft_id
            && self.chapter_number == other.chapter_number
            && self.chapter_title == other.chapter_title
            && self.chapter_notes == other.chapter_notes
            && serde_json::to_value(&self.facts).unwrap_or_default()
                == serde_json::to_value(&other.facts).unwrap_or_default()
            && self.character_state_candidates == other.character_state_candidates
            && self.source == other.source
            && self.source_status == other.source_status
    }
}

/// `FinalizedContinuityProjection` → `ContinuityProjectionView` 的逐字段转换。
fn continuity_projection_view(
    projection: &FinalizedContinuityProjection,
) -> ContinuityProjectionView {
    ContinuityProjectionView {
        draft_id: projection.draft_id,
        current_finalized_draft_id: projection.current_finalized_draft_id,
        chapter_number: projection.chapter_number,
        chapter_title: projection.chapter_title.clone(),
        chapter_notes: projection.chapter_notes.clone(),
        facts: projection.facts.clone(),
        character_state_candidates: projection.character_state_candidates.clone(),
        source: projection.source.clone(),
        source_status: projection.source_status.clone(),
    }
}

/// `ReviewCycleRecheckContext` → `RecheckContextView` 的逐字段转换。
fn recheck_context_view(context: &ReviewCycleRecheckContext) -> RecheckContextView {
    RecheckContextView {
        version: context.version,
        cycle_id: context.cycle_id.clone(),
        comparison_version: context.comparison_version,
        merged_hash: context.merged_hash.clone(),
        finding_set_hash: context.finding_set_hash.clone(),
        source_content: context.source_content.clone(),
        findings: context
            .findings
            .iter()
            .map(|finding| RecheckFindingView {
                finding_id: finding.finding_id.clone(),
                target_id: finding.target_id.clone(),
                category: finding.category.clone(),
                kind: finding.kind,
                problem: finding.problem.clone(),
                expected: finding.expected.clone(),
                source_span: finding.source_span.as_ref().map(|span| FindingSpanView {
                    start: span.start,
                    end: span.end,
                    unit: span.unit.clone(),
                }),
                occurrence: finding.occurrence,
                source_excerpt: finding.source_excerpt.clone(),
                must_show: finding.must_show,
            })
            .collect(),
    }
}

/// 基线 `HumanConfirmedReviewItem`（字段顺序对齐基线构造序）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanConfirmedReviewItem {
    pub category: String,
    pub severity: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stable_fact_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_chapter: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finding_id: Option<String>,
    pub decision: String,
    pub origin: String,
}

/// 基线 `HumanConfirmedReviewSnapshot`（字段顺序对齐基线构造序：
/// `kind, schemaVersion, sourceReviewId, [cycleId], [sourceDraft], [goalReview], summary, authorGuidance, items`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanConfirmedReviewSnapshot {
    pub kind: String,
    pub schema_version: i64,
    pub source_review_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_draft: Option<ExpectedDraftSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_review: Option<ChapterGoalReview>,
    pub summary: String,
    pub author_guidance: String,
    pub items: Vec<HumanConfirmedReviewItem>,
}

/// 基线 `ReviewRevisionContext.confirmation`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewConfirmation {
    pub review_source_id: i64,
    pub content: String,
    pub original_review_content_hash: String,
    pub snapshot: HumanConfirmedReviewSnapshot,
}

/// 基线 `ReviewRevisionContext`（字段顺序对齐基线构造序）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRevisionContext {
    pub version: i64,
    pub operation: ReviewRevisionOperation,
    pub source: ExpectedDraftSource,
    pub source_hash: String,
    pub config: NovelConfig,
    pub writing_language: String,
    pub ui_locale: String,
    pub author_inputs: Vec<crate::repositories::generation_run_repository::GenerationAuthorInput>,
    pub character_states: String,
    pub worldbuilding: String,
    pub history: Vec<ReviewFinalizedMaterial>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predecessor: Option<ReviewFinalizedMaterial>,
    pub blueprints: Vec<BlueprintForPreflight>,
    pub frozen_goals: FrozenChapterGoals,
    pub preflight_findings: Vec<ConsistencyFinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<ReviewConfirmation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recheck: Option<RecheckContextView>,
}

/// 基线 `PreparedReviewRevisionContext`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedReviewRevisionContext {
    pub context_id: String,
    pub context: ReviewRevisionContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_root_action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

// ===== 基线 `reviewRevisionRequest` =====

/// 基线 `reviewRevisionRequest(context)`：仅重建原始作者选择，绝不重建更新的手稿或确认。
pub fn review_revision_request(context: &ReviewRevisionContext) -> PrepareReviewRevisionRequest {
    let mut request = PrepareReviewRevisionRequest {
        operation: context.operation,
        draft_id: context.source.id,
        expected_draft: ExpectedDraftRef {
            chapter_number: context.source.chapter_number,
            version: context.source.version,
            status: context.source.status.clone(),
            content_hash: context.source_hash.clone(),
        },
        review_source_id: None,
        confirmed_review_content: None,
        review_cycle_id: None,
        expected_merged_hash: None,
        author_inputs: context.author_inputs.clone(),
        ui_locale: context.ui_locale.clone(),
    };
    if let Some(confirmation) = &context.confirmation {
        request.review_source_id = Some(confirmation.review_source_id);
        request.confirmed_review_content = Some(confirmation.content.clone());
    }
    if let Some(recheck) = &context.recheck {
        request.review_cycle_id = Some(recheck.cycle_id.clone());
        request.expected_merged_hash = Some(recheck.merged_hash.clone());
    }
    request
}

// ===== 共享纯函数（私有，随调用图平移）=====

/// 基线 `serializeHumanConfirmedReviewSnapshot`（`JSON.stringify(snapshot, null, 2)`）
fn serialize_human_confirmed_review_snapshot(snapshot: &HumanConfirmedReviewSnapshot) -> String {
    serde_json::to_string_pretty(snapshot).expect("快照 JSON 序列化不得失败")
}

/// 基线 `freezeChapterGoals(chapterNumber, keyEvents, authorSources)`
fn freeze_chapter_goals(
    chapter_number: i64,
    key_events: Option<&str>,
    author_sources: &[String],
) -> FrozenChapterGoals {
    // 基线 `split(/\r?\n|[；;]/u).map(trim).filter(Boolean)`
    let mut items: Vec<FrozenChapterGoalItem> = key_events
        .unwrap_or_default()
        .split(|c| c == '\n' || c == '；' || c == ';')
        .filter_map(|text| {
            let text = text.trim();
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        })
        .enumerate()
        .map(|(index, text)| FrozenChapterGoalItem {
            id: format!("ch{chapter_number}:keyEvents:{}", index + 1),
            text: text.to_string(),
        })
        .collect();
    // 基线 `marker = /^\s*【第(\d+)章必现】\s*(\S(?:.*\S)?)\s*$/u`
    let marker =
        regex::Regex::new(r"^\s*【第(\d+)章必现】\s*(\S(?:.*\S)?)\s*$").expect("必现标记正则");
    let mut must_show: Vec<FrozenChapterGoalItem> = author_sources
        .iter()
        .flat_map(|source| source.split('\n'))
        .filter_map(|line| {
            marker.captures(line).and_then(|captures| {
                let chapter = captures.get(1)?.as_str().parse::<i64>().ok()?;
                if chapter == chapter_number {
                    Some(captures.get(2)?.as_str().to_string())
                } else {
                    None
                }
            })
        })
        .enumerate()
        .map(|(index, text)| FrozenChapterGoalItem {
            id: format!("ch{chapter_number}:mustShow:{}", index + 1),
            text,
        })
        .collect();
    items.append(&mut must_show);
    // 基线 `coverage: items.length ? 'complete' : keyEvents === undefined ? 'unknown' : 'not_configured'`
    let coverage = if !items.is_empty() {
        "complete"
    } else if key_events.is_none() {
        "unknown"
    } else {
        "not_configured"
    };
    FrozenChapterGoals {
        chapter_number,
        coverage: coverage.to_string(),
        items,
    }
}

/// 基线 `normalizedKeyPart`（`value.trim().replace(/\s+/gu, ' ')`）
fn normalized_key_part(value: &str) -> String {
    value
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// 基线 `escapeRegExp`（`value.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&')`）
fn escape_reg_exp(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(
            ch,
            '.' | '*' | '+' | '?' | '^' | '$' | '{' | '}' | '(' | ')' | '|' | '[' | ']' | '\\'
        ) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// 基线 `isExplicitTerminalSubject(statement, entity)`
fn is_explicit_terminal_subject(statement: &str, entity: &str) -> bool {
    let subject = escape_reg_exp(entity);
    let divider = r"[\s|:：,，、—-]*";
    let chinese = format!(
        "{subject}{divider}(?:(?:已经|已|被确认(?:为)?|被证实(?:为)?){divider})?(?:死亡|身亡|牺牲|去世)"
    );
    let english =
        format!(r"{subject}[\s|:,-]+(?:(?:is|was|has(?: been)?)\s+)?(?:dead|died|deceased)\b");
    let chinese_re = regex::Regex::new(&chinese).expect("中文终态正则");
    let english_re = regex::RegexBuilder::new(&english)
        .case_insensitive(true)
        .build()
        .expect("英文终态正则");
    chinese_re.is_match(statement) || english_re.is_match(statement)
}

/// 基线 `continuityStableFactKey`（FNV-1a 64 位，十六进制 16 位补零）
fn continuity_stable_fact_key(
    category: &str,
    source_chapter: i64,
    entities: &[String],
    statement: &str,
) -> String {
    let identity = [
        category.to_string(),
        source_chapter.to_string(),
        {
            let mut normalized: Vec<String> =
                entities.iter().map(|e| normalized_key_part(e)).collect();
            normalized.sort();
            normalized.join(",")
        },
        normalized_key_part(statement),
    ]
    .join(":");
    let mut hash: u64 = 0xcbf29ce484222325;
    for character in identity.chars() {
        hash ^= u64::from(character as u32);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fact:{hash:016x}")
}

/// 基线 `findBlueprintContinuityRisks(projections, blueprint, exemptions)`
fn find_blueprint_continuity_risks(
    projections: &[ContinuityProjectionView],
    blueprint: &BlueprintForPreflight,
    exemptions: &[crate::repositories::consistency_exemption_repository::ConsistencyExemption],
) -> Vec<ConsistencyFinding> {
    let active_exemptions: std::collections::HashSet<&str> = exemptions
        .iter()
        .filter(|exemption| !exemption.revoked)
        .map(|exemption| exemption.stable_fact_key.as_str())
        .collect();
    let characters: std::collections::HashSet<String> = blueprint
        .characters
        .iter()
        .map(|character| normalized_key_part(character))
        .collect();
    let mut findings = Vec::new();
    for projection in projections {
        if projection.source_status != "current" {
            continue;
        }
        let Some(facts) = &projection.facts else {
            continue;
        };
        for fact in facts {
            if fact.category != "character-state" {
                continue;
            }
            let stable_fact_key = continuity_stable_fact_key(
                &fact.category,
                fact.source_chapter,
                &fact.entities,
                &fact.statement,
            );
            if active_exemptions.contains(stable_fact_key.as_str()) {
                continue;
            }
            let subject = fact
                .entities
                .iter()
                .map(|entity| normalized_key_part(entity))
                .find(|entity| {
                    characters.contains(entity)
                        && is_explicit_terminal_subject(&fact.statement, entity)
                });
            let Some(subject) = subject else {
                continue;
            };
            findings.push(ConsistencyFinding {
                stable_fact_key,
                severity: "warning".to_string(),
                source_chapter: fact.source_chapter,
                evidence: fact.evidence.clone(),
                issue: LocaleText {
                    zh_cn: format!(
                        "定稿正文绑定的派生索引提示“{subject}”处于死亡终态，但当前蓝图仍将其列为出场角色。请对照原文确认语义。"
                    ),
                    en_us: format!(
                        "A derived index bound to finalized prose marks “{subject}” as dead, but the current blueprint still schedules the character to appear. Confirm the meaning against the prose."
                    ),
                },
                suggestion: LocaleText {
                    zh_cn: "调整蓝图，或说明这是回忆、幻象等刻意安排。".to_string(),
                    en_us: "Adjust the blueprint, or record an intentional device such as a flashback or vision.".to_string(),
                },
            });
        }
    }
    findings
}

// ===== 基线 `captureReviewRevisionContext` =====

/// 基线 `captureReviewRevisionContext(db, request, projectId, frozenRecheckVersion?)`。
///
/// 捕获的数据库是权威；返回的材料在 dispatch / resume / save 时也会重读。
/// 仅项目 id 进入捕获身份，会话租约（`epoch`）刻意不作为输入——
/// 结果是 `(db, request)` 的纯函数，因此重开后逐字段不变。
pub fn capture_review_revision_context(
    conn: &Connection,
    request: &PrepareReviewRevisionRequest,
    project_id: &str,
    frozen_recheck_version: Option<i64>,
) -> Result<ReviewRevisionContext, String> {
    // 基线入口运行时校验（键集校验已由 `deny_unknown_fields` 在反序列化期完成）
    if !REVIEW_REVISION_OPERATIONS.contains(&request.operation.as_str())
        || !is_safe_integer(request.draft_id)
        || request.draft_id < 1
        || (request.ui_locale != "zh-CN" && request.ui_locale != "en-US")
        || request.author_inputs.len() > 2
    {
        return invalid();
    }
    let allowed_inputs: &[&str] = match request.operation {
        ReviewRevisionOperation::RefineDraft => &["user-prompt", "merged-guidance"],
        ReviewRevisionOperation::ReviewChapter => &["review-focus"],
        ReviewRevisionOperation::RefineFromReview => &[],
    };
    let ids: std::collections::HashSet<&str> = request
        .author_inputs
        .iter()
        .map(|item| item.id.as_str())
        .collect();
    if ids.len() != request.author_inputs.len()
        || request.author_inputs.iter().any(|item| {
            !allowed_inputs.contains(&item.id.as_str()) || item.text.len() > 8 * 1024 * 1024
        })
    {
        return invalid();
    }
    if request.operation != ReviewRevisionOperation::RefineFromReview
        && (request.review_source_id.is_some() || request.confirmed_review_content.is_some())
    {
        return invalid();
    }
    let recheck_requested =
        request.review_cycle_id.is_some() || request.expected_merged_hash.is_some();
    if recheck_requested
        && (request.operation != ReviewRevisionOperation::ReviewChapter
            || request
                .review_cycle_id
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty()
            || request.expected_merged_hash.is_none())
    {
        return invalid();
    }

    let source = query_source(conn, request.draft_id)?;
    let Some(source) = source else {
        return Err("GENERATION_REVIEW_SOURCE_CHANGED".to_string());
    };
    let latest: Option<i64> = conn
        .query_row(
            "SELECT id FROM drafts WHERE chapter_number=? AND status IN ('draft','revised','reviewed','finalized') ORDER BY version DESC,id DESC LIMIT 1",
            [source.chapter_number],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let expected = ExpectedDraftRef {
        chapter_number: source.chapter_number,
        version: source.version,
        status: source.status.clone(),
        content_hash: text_hash(&source.content),
    };
    if latest != Some(source.id) || request.expected_draft != expected {
        return Err("GENERATION_REVIEW_SOURCE_CHANGED".to_string());
    }

    let mut recheck: Option<RecheckContextView> = None;
    if recheck_requested {
        let plan = plan_recheck(
            request
                .review_cycle_id
                .as_deref()
                .ok_or_else(|| "GENERATION_REVIEW_RECHECK_NOT_REQUIRED".to_string())?,
            conn,
        )
        .map_err(|error| {
            if error == "REVIEW_CYCLE_INPUT_INVALID" {
                "GENERATION_REVIEW_RECHECK_NOT_REQUIRED".to_string()
            } else {
                error
            }
        })?;
        let Some(plan_context) = &plan.context else {
            return Err("GENERATION_REVIEW_RECHECK_NOT_REQUIRED".to_string());
        };
        if plan.disposition
            != crate::repositories::review_cycle_repository::RecheckDisposition::Required
            || plan_context.merged_hash
                != request.expected_merged_hash.as_deref().unwrap_or_default()
            || text_hash(&source.content) != plan_context.merged_hash
        {
            return Err("GENERATION_REVIEW_RECHECK_NOT_REQUIRED".to_string());
        }
        recheck = if frozen_recheck_version == Some(1) {
            let mut frozen = recheck_context_view(plan_context);
            frozen.version = 1;
            Some(frozen)
        } else {
            Some(recheck_context_view(plan_context))
        };
    }

    let core = get_project_core(conn)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "GENERATION_REVIEW_CONTEXT_INVALID".to_string())?;
    let config = NovelConfig {
        genre: core.genre.clone(),
        sub_genre: core.sub_genre.clone(),
        target_audience: core.target_audience.clone(),
        total_chapters: core.total_chapters,
        words_per_chapter: core.words_per_chapter,
        writing_language: core.writing_language.clone(),
        creative_strategy: core.creative_strategy.clone(),
        narrative_thread_dormant_chapter_threshold: core.narrative_thread_dormant_chapter_threshold,
        plot_structure: core.plot_structure.clone(),
        narrative_pov: core.narrative_pov.clone(),
        writing_style: core.writing_style.clone(),
        reference_works: core.reference_works.clone(),
        global_guidance: core.global_guidance.clone(),
        golden_finger: core.golden_finger.clone(),
        core_outline: core.core_outline.clone(),
        world_setting: core.world_setting.clone(),
        protagonist_profile: core.protagonist_profile.clone(),
    };

    let mut confirmation: Option<ReviewConfirmation> = None;
    if request.operation == ReviewRevisionOperation::RefineFromReview {
        let Some(review_source_id) = request.review_source_id else {
            return invalid();
        };
        if review_source_id < 1 {
            return invalid();
        }
        let Some(confirmed_review_content) = &request.confirmed_review_content else {
            return invalid();
        };
        let Some(stored) =
            get_review_full(conn, review_source_id).map_err(|error| error.to_string())?
        else {
            return Err("GENERATION_REVIEW_CONFIRMATION_CHANGED".to_string());
        };
        let Some(snapshot) = parse_human_confirmed_review_snapshot(&stored.content) else {
            return Err("GENERATION_REVIEW_CONFIRMATION_CHANGED".to_string());
        };
        let Some(requested) = parse_human_confirmed_review_snapshot(confirmed_review_content)
        else {
            return Err("GENERATION_REVIEW_CONFIRMATION_CHANGED".to_string());
        };
        let Some(original) =
            get_review_full(conn, snapshot.source_review_id).map_err(|error| error.to_string())?
        else {
            return Err("GENERATION_REVIEW_CONFIRMATION_CHANGED".to_string());
        };
        let Some(original_source_draft) = &original.source_draft else {
            return Err("GENERATION_REVIEW_CONFIRMATION_CHANGED".to_string());
        };
        let Some(stored_source_draft) = &stored.source_draft else {
            return Err("GENERATION_REVIEW_CONFIRMATION_CHANGED".to_string());
        };
        let Some(snapshot_source_draft) = &snapshot.source_draft else {
            return Err("GENERATION_REVIEW_CONFIRMATION_CHANGED".to_string());
        };
        if snapshot.source_review_id == stored.meta.id
            || *stored_source_draft != source
            || *snapshot_source_draft != source
            || *original_source_draft != source
            || stored.meta.base_draft_id != source.id
            || original.meta.base_draft_id != source.id
            || !snapshot.items.iter().any(|item| item.decision == "apply")
            || serialize_human_confirmed_review_snapshot(&snapshot)
                != serialize_human_confirmed_review_snapshot(&requested)
        {
            return Err("GENERATION_REVIEW_CONFIRMATION_CHANGED".to_string());
        }
        confirmation = Some(ReviewConfirmation {
            review_source_id: stored.meta.id,
            content: serialize_human_confirmed_review_snapshot(&snapshot),
            original_review_content_hash: text_hash(&original.content),
            snapshot,
        });
    }

    let chapter_floor = source.chapter_number;
    let chapter_ceiling = source.chapter_number + 5;
    let blueprints: Vec<BlueprintForPreflight> = get_all_blueprints(conn)
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|blueprint| {
            blueprint.chapter_number >= chapter_floor && blueprint.chapter_number <= chapter_ceiling
        })
        .map(blueprint_for_preflight)
        .collect();
    let current_blueprint = blueprints
        .iter()
        .find(|blueprint| blueprint.chapter_number == source.chapter_number)
        .cloned();
    let projections = list_finalized_continuity_before(conn, source.chapter_number)
        .map_err(|error| error.to_string())?;
    let rows =
        query_finalized_history(conn, source.chapter_number).map_err(|error| error.to_string())?;
    let mut history = Vec::new();
    for row in &rows {
        if row.finalization_id.is_some()
            && (row.content_hash.as_deref() != Some(text_hash(&row.body).as_str())
                || row.content_snapshot.as_deref() != Some(row.body.as_str()))
        {
            return Err("GENERATION_REVIEW_HISTORY_CHANGED".to_string());
        }
        let content_hash = text_hash(&row.body);
        let source_identity =
            row.finalization_id
                .as_deref()
                .map(|finalization_id| FinalizedSourceIdentity {
                    draft_id: row.id,
                    finalization_id: finalization_id.to_string(),
                    chapter_number: row.chapter_number,
                    content_hash: content_hash.clone(),
                });
        let projection = projections
            .iter()
            .find(|item| {
                item.draft_id == row.id
                    && item.source_status == "current"
                    && projection_source_matches(item, &source_identity)
            })
            .cloned();
        let material_identity = ReviewMaterialIdentity {
            project_id: project_id.to_string(),
            source_id: format!("finalized:{}", row.id),
            revision: row.id,
            content_hash: content_hash.clone(),
            provenance: if row.finalization_id.is_some() {
                "finalized"
            } else {
                "legacy"
            }
            .to_string(),
        };
        let mut material = ReviewFinalizedMaterial {
            draft_id: row.id,
            chapter_number: row.chapter_number,
            chapter_title: row.chapter_title.clone().unwrap_or_default(),
            content: row.body.clone(),
            source: None,
            projection: None,
            identity: Some(material_identity),
        };
        if let Some(source_identity) = &source_identity {
            material.source = Some(source_identity.clone());
        }
        if let Some(projection) = &projection {
            material.projection = Some(continuity_projection_view(projection));
        }
        history.push(material);
    }

    // 基线：草稿已记录作者选定的前驱，绝不以更新版本替代。
    let dependencies_json: String = conn
        .query_row(
            "SELECT source_dependencies FROM drafts WHERE id=?",
            [source.id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let dependencies: Value = serde_json::from_str(&dependencies_json)
        .map_err(|_| "GENERATION_REVIEW_HISTORY_CHANGED".to_string())?;
    let Some(dependencies) = dependencies.as_array() else {
        return Err("GENERATION_REVIEW_HISTORY_CHANGED".to_string());
    };
    let mut predecessor: Option<ReviewFinalizedMaterial> = None;
    for dependency in dependencies {
        let Some(dependency) = dependency.as_object() else {
            continue;
        };
        if dependency
            .get("kind")
            .is_some_and(|kind| kind.as_str() != Some("candidate"))
        {
            continue;
        }
        let (Some(draft_id), Some(content_hash)) = (
            dependency.get("draftId").and_then(Value::as_i64),
            dependency.get("contentHash").and_then(Value::as_str),
        ) else {
            return Err("GENERATION_REVIEW_HISTORY_CHANGED".to_string());
        };
        let row = query_source(conn, draft_id)?;
        let Some(row) = row else {
            return Err("GENERATION_REVIEW_HISTORY_CHANGED".to_string());
        };
        if text_hash(&row.content) != content_hash
            || !["draft", "revised", "reviewed", "finalized", "archived"]
                .contains(&row.status.as_str())
        {
            return Err("GENERATION_REVIEW_HISTORY_CHANGED".to_string());
        }
        if row.chapter_number != source.chapter_number - 1
            || history.iter().any(|item| item.draft_id == row.id)
        {
            continue;
        }
        if predecessor.is_some() {
            return Err("GENERATION_REVIEW_HISTORY_CHANGED".to_string());
        }
        predecessor = Some(ReviewFinalizedMaterial {
            draft_id: row.id,
            chapter_number: row.chapter_number,
            chapter_title: String::new(),
            content: row.content.clone(),
            source: None,
            projection: None,
            identity: Some(ReviewMaterialIdentity {
                project_id: project_id.to_string(),
                source_id: format!("candidate:{}", row.id),
                revision: row.version,
                content_hash: content_hash.to_string(),
                provenance: "generated".to_string(),
            }),
        });
    }

    let active_ids: std::collections::HashSet<String> = conn
        .query_row(
            "SELECT json_group_array(character_id) FROM characters WHERE retired=0",
            [],
            |row| row.get::<_, String>(0),
        )
        .map(|json: String| {
            serde_json::from_str::<Value>(&json)
                .ok()
                .and_then(|value| value.as_array().cloned())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let author_cards = get_all_characters(conn)
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|card| {
            card.character_id
                .as_deref()
                .is_some_and(|character_id| active_ids.contains(character_id))
        })
        .collect::<Vec<_>>();
    let state_lines = author_cards
        .iter()
        .flat_map(|card| {
            let Some(state) = &card.current_state else {
                return Vec::new();
            };
            let provenance = state.provenance.as_ref();
            let fields: Vec<(String, String)> = CHARACTER_STATE_TEXT_FIELDS
                .iter()
                .filter_map(|field| {
                    let Some(provenance) = provenance.and_then(|provenance| provenance.get(*field)) else {
                        return None;
                    };
                    let CharacterStateFieldProvenance::Author { chapter_number } = provenance else {
                        return None;
                    };
                    let value = state_field_value(state, field);
                    if value.is_empty() {
                        return None;
                    }
                    Some((
                        (*field).to_string(),
                        format!("{value} @ch{chapter_number}"),
                    ))
                })
                .collect();
            if fields.is_empty() {
                return Vec::new();
            }
            let fields_json = serde_json::to_string(&fields).unwrap_or_default();
            let line = if core.writing_language == "en-US" {
                format!(
                    "{} ({}) author state (time-bound to the annotated chapter, not permanent): {fields_json}",
                    card.name,
                    card.role.as_str()
                )
            } else {
                format!(
                    "{}（{}）作者状态（按标注章节理解，非永久约束）: {fields_json}",
                    card.name,
                    card.role.as_str()
                )
            };
            vec![line]
        })
        .collect::<Vec<_>>();
    let character_states = if state_lines.is_empty() {
        if core.writing_language == "en-US" {
            "(none)".to_string()
        } else {
            "（暂无）".to_string()
        }
    } else {
        state_lines.join("\n")
    };

    let frozen_goals = freeze_chapter_goals(
        source.chapter_number,
        current_blueprint
            .as_ref()
            .map(|blueprint| blueprint.key_events.as_str()),
        &[core.world_setting.clone()]
            .into_iter()
            .chain(author_cards.iter().map(|card| card.notes.clone()))
            .collect::<Vec<_>>(),
    );
    let preflight_findings = match &current_blueprint {
        Some(current_blueprint) => {
            let exemptions = list_exemptions(conn).map_err(|error| error.to_string())?;
            let projections_with_projection: Vec<ContinuityProjectionView> = history
                .iter()
                .filter_map(|item| item.projection.clone())
                .collect();
            find_blueprint_continuity_risks(
                &projections_with_projection,
                current_blueprint,
                &exemptions,
            )
        }
        None => Vec::new(),
    };

    let source_hash = text_hash(&source.content);
    Ok(RevisionContextBuilder {
        version: 1,
        operation: request.operation,
        source,
        source_hash,
        config,
        writing_language: core.writing_language,
        ui_locale: request.ui_locale.clone(),
        author_inputs: request.author_inputs.clone(),
        character_states,
        worldbuilding: core.worldbuilding,
        history,
        predecessor,
        blueprints,
        frozen_goals,
        preflight_findings,
        confirmation,
        recheck,
    }
    .build())
}

// ===== 内部辅助 =====

/// 基线 `Number.isSafeInteger(value)`（i64 全域安全，仅作显式守卫）
fn is_safe_integer(value: i64) -> bool {
    value.abs() <= 9007199254740991
}

fn query_source(conn: &Connection, draft_id: i64) -> Result<Option<ExpectedDraftSource>, String> {
    conn.query_row(
        "SELECT d.id,d.chapter_number,d.version,d.status,c.body FROM drafts d JOIN contents c ON c.id=d.content_id WHERE d.id=?",
        [draft_id],
        |row| {
            Ok(ExpectedDraftSource {
                id: row.get("id")?,
                chapter_number: row.get("chapter_number")?,
                version: row.get("version")?,
                status: row.get("status")?,
                content: row.get("body")?,
            })
        },
    )
    .optional()
    .map_err(|error| error.to_string())
}

/// 基线定稿历史查询（含终稿 outbox 凭据的左连接）
#[derive(Debug)]
struct FinalizedHistoryRow {
    id: i64,
    chapter_number: i64,
    body: String,
    finalization_id: Option<String>,
    chapter_title: Option<String>,
    content_hash: Option<String>,
    content_snapshot: Option<String>,
}

fn query_finalized_history(
    conn: &Connection,
    chapter_number: i64,
) -> Result<Vec<FinalizedHistoryRow>, String> {
    let mut stmt = conn.prepare(
        "SELECT d.id,d.chapter_number,d.version,c.body,o.finalization_id,o.chapter_title,o.content_hash,o.content_snapshot FROM drafts d JOIN contents c ON c.id=d.content_id LEFT JOIN finalization_outbox o ON o.draft_id=d.id WHERE d.status='finalized' AND d.chapter_number < ? AND NOT EXISTS (SELECT 1 FROM drafts n WHERE n.chapter_number=d.chapter_number AND n.status='finalized' AND (n.version>d.version OR n.version=d.version AND n.id>d.id)) ORDER BY d.chapter_number,d.id",
    )
    .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map([chapter_number], |row| {
            Ok(FinalizedHistoryRow {
                id: row.get("id")?,
                chapter_number: row.get("chapter_number")?,
                body: row.get("body")?,
                finalization_id: row.get("finalization_id")?,
                chapter_title: row.get("chapter_title")?,
                content_hash: row.get("content_hash")?,
                content_snapshot: row.get("content_snapshot")?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|error| error.to_string())?);
    }
    Ok(result)
}

/// 基线 `projections.find(item => item.draftId === row.id && item.sourceStatus === 'current' && isDeepStrictEqual(item.source, sourceIdentity))`
fn projection_source_matches(
    projection: &FinalizedContinuityProjection,
    source_identity: &Option<FinalizedSourceIdentity>,
) -> bool {
    match (&projection.source, source_identity) {
        (Some(actual), Some(expected)) => actual == expected,
        (None, None) => true,
        _ => false,
    }
}

fn blueprint_for_preflight(blueprint: BlueprintData) -> BlueprintForPreflight {
    BlueprintForPreflight {
        chapter_number: blueprint.chapter_number,
        title: blueprint.title,
        role: blueprint.role,
        purpose: blueprint.purpose,
        key_events: blueprint.key_events,
        characters: blueprint.characters,
        suspense_hook: blueprint.suspense_hook,
        user_guidance: blueprint.user_guidance,
        notes: blueprint.notes,
    }
}

/// `CharacterStateData` 的文本字段读取（`CHARACTER_STATE_TEXT_FIELDS` 逐字段映射）
fn state_field_value<'a>(
    state: &'a crate::repositories::character_repository::CharacterStateData,
    field: &str,
) -> &'a str {
    match field {
        "location" => &state.location,
        "powerLevel" => &state.power_level,
        "physicalState" => &state.physical_state,
        "mentalState" => &state.mental_state,
        "keyItems" => &state.key_items,
        "recentEvents" => &state.recent_events,
        _ => "",
    }
}

/// 基线 `parseHumanConfirmedReviewSnapshot(content)` 的本文件同形实现
/// （`review_cycle_repository` 的同名私有项类型不互通，此处另译；
/// 校验规则逐字对齐 `src/shared/human-confirmed-review.ts`）。
fn parse_human_confirmed_review_snapshot(content: &str) -> Option<HumanConfirmedReviewSnapshot> {
    let parsed: Value = serde_json::from_str(content).ok()?;
    validate_human_confirmed_review_snapshot(&parsed)
}

/// 基线 `validateHumanConfirmedReviewSnapshot(value)`
fn validate_human_confirmed_review_snapshot(value: &Value) -> Option<HumanConfirmedReviewSnapshot> {
    let record = value.as_object()?;
    let schema_version = record.get("schemaVersion")?.as_i64()?;
    if record.get("kind").and_then(Value::as_str) != Some("human-confirmed-review")
        || (schema_version != 1 && schema_version != 2)
        || record
            .get("sourceReviewId")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            < 1
        || !record.get("items")?.is_array()
    {
        return None;
    }
    let cycle_id = match record.get("cycleId") {
        None => None,
        Some(Value::Null) => None,
        Some(value) => {
            let text = value.as_str()?;
            if text.trim().is_empty() {
                return None;
            }
            Some(text.trim().to_string())
        }
    };
    if (schema_version == 1 && cycle_id.is_some()) || (schema_version == 2 && cycle_id.is_none()) {
        return None;
    }
    let summary = record.get("summary")?.as_str()?;
    let author_guidance = record.get("authorGuidance")?.as_str()?;
    let source_draft = match record.get("sourceDraft") {
        None | Some(Value::Null) => None,
        Some(value) => Some(parse_source_draft(value)?),
    };
    let goal_review = match record.get("goalReview") {
        None | Some(Value::Null) => None,
        Some(value) => Some(parse_chapter_goal_review(value)?),
    };
    let items_value = record.get("items")?.as_array()?;
    let mut items = Vec::new();
    for item in items_value {
        items.push(parse_human_confirmed_item(item, schema_version)?);
    }
    Some(HumanConfirmedReviewSnapshot {
        kind: "human-confirmed-review".to_string(),
        schema_version,
        source_review_id: record.get("sourceReviewId")?.as_i64()?,
        cycle_id,
        source_draft,
        goal_review,
        summary: summary.to_string(),
        author_guidance: author_guidance.to_string(),
        items,
    })
}

/// 基线 `parseSourceDraft(value)`
fn parse_source_draft(value: &Value) -> Option<ExpectedDraftSource> {
    let source = value.as_object()?;
    let id = source.get("id")?.as_i64()?;
    let chapter_number = source.get("chapterNumber")?.as_i64()?;
    let version = source.get("version")?.as_i64()?;
    if id < 1 || chapter_number < 1 || version < 1 {
        return None;
    }
    let status = source.get("status")?.as_str()?;
    if !["draft", "revised", "reviewed", "finalized", "archived"].contains(&status) {
        return None;
    }
    let content = source.get("content")?.as_str()?;
    if crate::draft_units::count_draft_units(content) < 1 {
        return None;
    }
    Some(ExpectedDraftSource {
        id,
        chapter_number,
        version,
        status: status.to_string(),
        content: content.to_string(),
    })
}

/// 基线 `parseItem(value, schemaVersion)`
fn parse_human_confirmed_item(
    value: &Value,
    schema_version: i64,
) -> Option<HumanConfirmedReviewItem> {
    let record = value.as_object()?;
    let category = nonempty_string(record.get("category"))?;
    let severity = nonempty_string(record.get("severity"))?;
    let description = nonempty_string(record.get("description"))?;
    let quote = optional_string(record.get("quote"))?;
    let stable_fact_key = optional_nonempty_string(record.get("stableFactKey"))?;
    let source_chapter = record.get("sourceChapter").and_then(Value::as_i64);
    let goal_id = optional_nonempty_string(record.get("goalId"))?;
    let finding_id = optional_nonempty_string(record.get("findingId"))?;
    let decision = record.get("decision")?.as_str()?;
    let origin = record.get("origin")?.as_str()?;
    if !["apply", "ignore", "waive"].contains(&decision) || !["ai", "author"].contains(&origin) {
        return None;
    }
    if source_chapter.is_some_and(|chapter| chapter < 1) {
        return None;
    }
    if schema_version == 1 && (finding_id.is_some() || decision == "waive") {
        return None;
    }
    if decision == "waive" && (schema_version != 2 || origin != "ai" || finding_id.is_none()) {
        return None;
    }
    Some(HumanConfirmedReviewItem {
        category,
        severity,
        description,
        quote,
        stable_fact_key,
        source_chapter,
        goal_id,
        finding_id,
        decision: decision.to_string(),
        origin: origin.to_string(),
    })
}

/// 基线 `parseChapterGoalReview(value)`（`chapter-goal-review.ts`）
fn parse_chapter_goal_review(value: &Value) -> Option<ChapterGoalReview> {
    let parsed = value.as_object()?;
    let version = parsed.get("version")?.as_i64()?;
    let chapter_number = parsed.get("chapterNumber")?.as_i64()?;
    if version != 1 || chapter_number < 1 {
        return None;
    }
    let coverage = parsed.get("coverage")?.as_str()?;
    if !["complete", "unknown", "not_configured"].contains(&coverage) {
        return None;
    }
    let items_value = parsed.get("items")?.as_array()?;
    let mut ids = std::collections::HashSet::new();
    let mut items = Vec::new();
    for item_value in items_value {
        let item = item_value.as_object()?;
        let id = nonempty_string(item.get("id"))?;
        if ids.contains(&id) {
            return None;
        }
        ids.insert(id.clone());
        let text = nonempty_string(item.get("text"))?;
        let description = nonempty_string(item.get("description"))?;
        let status = item.get("status")?.as_str()?;
        if !["completed", "unmet", "unknown"].contains(&status) {
            return None;
        }
        let evidence_value = item.get("evidence")?.as_array()?;
        if status != "unknown" && evidence_value.is_empty() {
            return None;
        }
        let mut evidence = Vec::new();
        for evidence_entry in evidence_value {
            let evidence_record = evidence_entry.as_object()?;
            let quote = nonempty_string(evidence_record.get("quote"))?;
            let start = evidence_record.get("start")?.as_i64()?;
            let end = evidence_record.get("end")?.as_i64()?;
            if start < 0 || end != start + quote.chars().count() as i64 {
                return None;
            }
            evidence.push(ChapterGoalEvidence { quote, start, end });
        }
        items.push(ChapterGoalReviewItem {
            id,
            text,
            status: status.to_string(),
            description,
            evidence,
        });
    }
    Some(ChapterGoalReview {
        version,
        chapter_number,
        coverage: coverage.to_string(),
        items,
    })
}

/// 基线 `nonEmptyString(value)`：`trim` 后非空
fn nonempty_string(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 基线 `stringValue(value)`：仅字符串可满足
fn optional_string(value: Option<&Value>) -> Option<Option<String>> {
    match value {
        None => Some(None),
        Some(Value::Null) => Some(None),
        Some(value) => Some(Some(value.as_str()?.to_string())),
    }
}

/// 基线 `nonEmptyString` 的可选键版本（`undefined` → `None`）
fn optional_nonempty_string(value: Option<&Value>) -> Option<Option<String>> {
    match value {
        None => Some(None),
        Some(Value::Null) => Some(None),
        Some(value) => {
            let text = value.as_str()?;
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(Some(trimmed.to_string()))
            }
        }
    }
}

/// 构造 `ReviewRevisionContext` 的中间 builder（保证字段序与基线构造序一致）
struct RevisionContextBuilder {
    version: i64,
    operation: ReviewRevisionOperation,
    source: ExpectedDraftSource,
    source_hash: String,
    config: NovelConfig,
    writing_language: String,
    ui_locale: String,
    author_inputs: Vec<crate::repositories::generation_run_repository::GenerationAuthorInput>,
    character_states: String,
    worldbuilding: String,
    history: Vec<ReviewFinalizedMaterial>,
    predecessor: Option<ReviewFinalizedMaterial>,
    blueprints: Vec<BlueprintForPreflight>,
    frozen_goals: FrozenChapterGoals,
    preflight_findings: Vec<ConsistencyFinding>,
    confirmation: Option<ReviewConfirmation>,
    recheck: Option<RecheckContextView>,
}

impl RevisionContextBuilder {
    fn build(self) -> ReviewRevisionContext {
        ReviewRevisionContext {
            version: self.version,
            operation: self.operation,
            source: self.source,
            source_hash: self.source_hash,
            config: self.config,
            writing_language: self.writing_language,
            ui_locale: self.ui_locale,
            author_inputs: self.author_inputs,
            character_states: self.character_states,
            worldbuilding: self.worldbuilding,
            history: self.history,
            predecessor: self.predecessor,
            blueprints: self.blueprints,
            frozen_goals: self.frozen_goals,
            preflight_findings: self.preflight_findings,
            confirmation: self.confirmation,
            recheck: self.recheck,
        }
    }
}
