//! 审稿修稿生成服务（批次 G4-3b）——行为等价平移自
//! `electron/services/review-revision-generation.ts`（301 行）的
//! 4 个公开方法 `prepare` / `commitReview` / `commitRevision` /
//! `readRecovery` 及其调用图内的全部私有辅助。
//!
//! # 状态模型（刻意偏离，对齐 Tauri 无状态命令模型）
//!
//! 基线 `ReviewRevisionGeneration` 由主进程 owner 持有（per-session
//! 实例），`contexts` 是实例内存缓存。Tauri 命令是无状态的，故
//! `contexts` 缓存平移为**进程级静态 `Mutex<HashMap>`**：
//! `contextId` 为 UUID v4（全局唯一），按 `contextId` 查找与
//! 项目归属无关；项目归属由 `requireRun` 的
//! `run.binding.projectId === scope.projectId` 门禁等价保证。
//! 容量上限（64）与 FIFO 淘汰逐字对齐基线 `remember`。
//!
//! # 依赖注入（基线闭包回调的 Rust 等价物）
//!
//! 基线构造器注入 `assertCurrent`，方法注入 `assertSources` /
//! `sourcesMatch` / `snapshot`。Rust 侧以 `impl FnOnce` /
//! `impl Fn` 参数逐字保留注入点，由命令层按会话状态提供：
//!
//! - `assert_current`：会话有效性门禁（等价于基线 `assertCurrent`，
//!   命令层在 `with_project_db` 之外显式断言活跃项目会话）；
//! - `assert_sources`：提交前源/模板/模型仍为当前（基线
//!   `assertSourcesCurrent`）；
//! - `sources_match` / `snapshot`：恢复读面的源绑定比对与候选
//!   快照投影（基线 `compareGenerationSourceBindings` /
//!   `viewOf(run).candidates?.at(-1)`）——二者依赖
//!   `MainGenerationOwner` 的 `rebuildBinding` / `currentModelReceipt` /
//!   `viewOf(run).candidates`，已由命令层按真实实现注入
//!   （`assert_review_sources_current` / `review_revision_sources_match` /
//!   `review_revision_candidate_snapshot`）。
//!
//! # 共享纯函数随调用图平移（私有项）
//!
//! 基线依赖的共享纯函数在 Rust 侧尚未建立公共模块，本波随调用图
//! 一并平移为本文件私有项（对齐 G4-3a / G4-3b `review_revision_context`
//! 先例）：
//!
//! - `review-generation-report.ts`：`buildReviewGenerationReport` /
//!   `parseReviewGenerationResult`（含 `isReviewShape` / `isReviewResult` /
//!   `reportText`）。
//! - `chapter-goal-review.ts`：`normalizeChapterGoalReview` /
//!   `chapterGoalReviewItems` / `locateGoalEvidence`。
//! - `consistency-preflight.ts`：`mergeConsistencyFindingsIntoReview`。
//! - `review-cycle.ts`：`buildReviewCycleRecheckReport` /
//!   `isNoopRevision`（`countOccurrences` / `objectiveText` /
//!   `parseRecheckItems` 为其私有辅助）。
//! - `character-proposal-parser.ts`：`extractSingleCompleteJsonObject` /
//!   `findCompleteJsonObjectEnd` / `isRecord`。
//! - `draft-visible-text.ts`：`stripDraftThinkingTags`。
//! - `refinement-completeness.ts`：`assertMateriallyCompleteRevision` /
//!   `countVisibleProseUnits`。
//! - `visible-text-integrity.ts`：`assertMechanicallyCompleteVisibleText` /
//!   `visibleProseUnitCount` / `mechanicalCompletionError`。
//! - `draft-units.ts`：`countDraftUnits` 复用 `crate::draft_units`（已移植）。
//!
//! # 刻意偏离（均有上游语义理由）
//!
//! - **事务**：审稿/修稿提交由服务持有IMMEDIATE事务，仓储显式复用同一事务；
//!   业务正文、旧pending状态、生成效果收据及审稿周期一并提交或回滚，
//!   不嵌套BEGIN，不因业务仓储先提交而遗留不可恢复的半成品。
//!
//! - **错误信封**：基线 `throw new Error(code)` 经 IPC 序列化为
//!   `Error: <code>`；Tauri 命令返回 `Result<T, String>`，命令层
//!   以 `commands::main_generation_owner::mutating_error` 复刻
//!   「Error: 」前缀（对齐 `commands::db::mutating_error` 既有惯例）。

use rusqlite::{Connection, OptionalExtension};
use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use crate::draft_units::count_draft_units;
use crate::repositories::generation_run_repository::{
    DurableGenerationRun, GenerationAuthorInput, GenerationRunRepository, MainGenerationRunHandle,
    VisibleArtifact, VisibleCompositionReceipt,
};
use crate::repositories::review_cycle_repository::{
    self as review_cycle, attach_generated_revision as attach_generated_revision_repo,
    commit_recheck as commit_recheck_repo, create as create_review_cycle_repo,
    plan_recheck as plan_recheck_repo, AttachGeneratedRevisionInput, CreateReviewCycleInput,
    ReviewCycleRecheckContext, ReviewCycleSource,
};
use crate::repositories::review_repository::{
    self as review_repo, create_in_transaction as create_review_repo,
    get_full as get_review_full_repo, ReviewCreateParams,
};
use crate::repositories::revision_repository::{
    self as revision_repo, get_full as get_revision_full_repo,
    replace_pending_in_transaction as replace_pending_repo, RevisionCreateParams,
};

use crate::draft_source_guard::ExpectedDraftSource;
use crate::services::review_revision_context::{
    capture_review_revision_context, PreparedReviewRevisionContext, ReviewRevisionContext,
    ReviewRevisionOperation, REVIEW_REVISION_OPERATIONS,
};
// ===== 契约类型（字段序对齐基线接口 / 对象字面量构造序）=====

/// 基线 `ReviewGenerationCommitRequest.artifact`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewArtifactRef {
    pub artifact_id: String,
    pub revision: i64,
    pub text_hash: String,
}

/// 基线 `ReviewGenerationCommitRequest`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewGenerationCommitRequest {
    pub context_id: String,
    pub handle: MainGenerationRunHandle,
    pub artifact: ReviewArtifactRef,
}

/// 基线 `RevisionGenerationCommitRequest`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionGenerationCommitRequest {
    pub context_id: String,
    pub handle: MainGenerationRunHandle,
    pub expected_composition_hash: String,
}

/// 基线 `ReviewRevisionCommitReceipt.reviewCycle`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRevisionCommitCycle {
    pub cycle_id: String,
    pub revision_status: String,
    pub recheck_count: i64,
    pub disposition: String,
}

/// 基线 `ReviewRevisionCommitReceipt`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRevisionCommitReceipt {
    pub success: bool,
    pub kind: String,
    pub id: i64,
    pub index: i64,
    pub content: String,
    pub content_hash: String,
    pub source: ExpectedDraftSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_cycle: Option<ReviewRevisionCommitCycle>,
}

/// 基线 `ReviewRevisionRecovery`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRevisionRecovery {
    pub handle: MainGenerationRunHandle,
    pub context: ReviewRevisionContext,
    pub model_id: String,
    pub source_status: String,
    pub can_resume: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved: Option<ReviewRevisionCommitReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition: Option<VisibleCompositionReceipt>,
    pub last_composition_finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_artifact: Option<Value>,
    pub latest_artifact_finish_reason: Option<String>,
    pub attempted_purposes: Vec<String>,
}

// ===== 服务构造 =====

/// 基线 `{ projectId, epoch }` 作用域（会话身份）。
#[derive(Debug, Clone)]
pub struct ReviewRevisionScope {
    pub project_id: String,
    pub epoch: String,
}

/// 基线 `ReviewRevisionGeneration`。
///
/// `contexts` 缓存为进程级静态项（见文件头注「状态模型」）。
pub struct ReviewRevisionGeneration {
    runs: GenerationRunRepository,
    scope: ReviewRevisionScope,
}

/// 进程级 `contexts` 缓存（基线 `private readonly contexts = new Map()`，
/// 容量 64 + FIFO 淘汰，见 `remember`）。
static PREPARED_CONTEXTS: LazyLock<Mutex<HashMap<String, PreparedReviewRevisionContext>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 基线 `contexts` 容量上限。
const CONTEXT_CACHE_CAPACITY: usize = 64;

impl ReviewRevisionGeneration {
    /// 基线 `new ReviewRevisionGeneration(deps.database, repository, scope, assertCurrent)`
    /// 的构造等价物（`db` 由方法参数 `&Connection` 注入，
    /// `assertCurrent` 由方法参数注入）。
    pub fn new(runs: GenerationRunRepository, scope: ReviewRevisionScope) -> Self {
        Self { runs, scope }
    }

    // ===== 私有辅助 =====

    /// 基线 `handleOf(run)`
    fn handle_of(run: &DurableGenerationRun) -> MainGenerationRunHandle {
        MainGenerationRunHandle {
            project_id: run.binding.project_id.clone(),
            epoch: run.binding.epoch.clone(),
            root_action_id: run.root_action_id.clone(),
            run_id: run.run_id.clone(),
        }
    }

    /// 基线 `contextHash(context) = textHash(JSON.stringify(context))`
    fn context_hash(context: &ReviewRevisionContext) -> String {
        let json = serde_json::to_string(context).expect("上下文 JSON 序列化不得失败");
        text_hash(&json)
    }

    /// 基线 `remember(context, lineage)`：缓存 prepared 上下文（容量 64 + FIFO）。
    fn remember(
        context: &ReviewRevisionContext,
        lineage: Lineage,
    ) -> PreparedReviewRevisionContext {
        let prepared = PreparedReviewRevisionContext {
            context_id: random_uuid_v4(),
            context: context.clone(),
            parent_root_action_id: lineage.parent_root_action_id,
            model_id: lineage.model_id,
        };
        let mut contexts = PREPARED_CONTEXTS
            .lock()
            .expect("审稿修稿上下文缓存不得被污染");
        if contexts.len() >= CONTEXT_CACHE_CAPACITY {
            let oldest = contexts.keys().next().cloned();
            if let Some(oldest) = oldest {
                contexts.remove(&oldest);
            }
        }
        contexts.insert(prepared.context_id.clone(), prepared.clone());
        prepared
    }

    /// 基线 `context(run)`：从运行账本 `sourceManifest` 读取并校验上下文。
    fn context_of(&self, run: &DurableGenerationRun) -> Result<ReviewRevisionContext, String> {
        let context_value = run
            .binding
            .source_manifest
            .get("reviewRevisionContext")
            .cloned();
        let Some(context_value) = context_value else {
            return Err(mutating_error("GENERATION_REVIEW_CONTEXT_UNPROVEN"));
        };
        let context: ReviewRevisionContext = serde_json::from_value(context_value)
            .map_err(|_| mutating_error("GENERATION_REVIEW_CONTEXT_UNPROVEN"))?;
        let manifest_operation = run
            .binding
            .source_manifest
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if context.version != 1
            || context.operation.as_str() != manifest_operation
            || !REVIEW_REVISION_OPERATIONS.contains(&context.operation.as_str())
            || run
                .binding
                .source_manifest
                .get("reviewRevisionContextHash")
                .and_then(Value::as_str)
                != Some(Self::context_hash(&context).as_str())
            || context.source_hash != text_hash(&context.source.content)
            || !is_deep_strict_equal(
                &run.binding
                    .source_manifest
                    .get("authorInputs")
                    .cloned()
                    .unwrap_or(Value::Null),
                &serde_json::to_value(&[GenerationAuthorInput {
                    id: "review-revision-context".to_string(),
                    text: serde_json::to_string(&context).expect("上下文 JSON 序列化不得失败"),
                }])
                .expect("authorInputs JSON 序列化不得失败"),
            )
        {
            return Err(mutating_error("GENERATION_REVIEW_CONTEXT_UNPROVEN"));
        }
        Ok(context)
    }

    /// 基线 `attempts(runId?)`
    fn attempts(&self, conn: &Connection, run_id: Option<&str>) -> Result<Vec<AttemptRow>, String> {
        let (sql, params): (&str, Vec<String>) = match run_id {
            Some(run_id) => (
                "SELECT attempt_id,run_id,usage_receipt_json FROM generation_attempts WHERE run_id=? ORDER BY rowid",
                vec![run_id.to_string()],
            ),
            None => (
                "SELECT attempt_id,run_id,usage_receipt_json FROM generation_attempts ORDER BY rowid",
                vec![],
            ),
        };
        let mut stmt = conn
            .prepare(sql)
            .map_err(|error| mutating_error(&error.to_string()))?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(params.iter().map(|value| value.as_str())),
                |row| {
                    Ok(AttemptRow {
                        attempt_id: row.get("attempt_id")?,
                        run_id: row.get("run_id")?,
                        usage_receipt_json: row.get("usage_receipt_json")?,
                    })
                },
            )
            .map_err(|error| mutating_error(&error.to_string()))?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|error| mutating_error(&error.to_string()))?);
        }
        Ok(result)
    }

    /// 基线 `requireRun(handle)`
    fn require_run(
        &self,
        conn: &Connection,
        handle: &MainGenerationRunHandle,
        assert_current: impl FnOnce() -> Result<(), String>,
    ) -> Result<DurableGenerationRun, String> {
        assert_current().map_err(|_| mutating_error("GENERATION_OWNER_CLOSED"))?;
        let run = self
            .runs
            .get(conn, &handle.run_id)
            .map_err(|error| mutating_error(&error))?;
        if run.binding.project_id != self.scope.project_id
            || !is_deep_strict_equal(
                &serde_json::to_value(handle).unwrap_or(Value::Null),
                &serde_json::to_value(Self::handle_of(&run)).unwrap_or(Value::Null),
            )
        {
            return Err(mutating_error("GENERATION_RUN_IDENTITY_MISMATCH"));
        }
        self.context_of(&run)?;
        Ok(run)
    }

    /// 基线 `proveArtifact(run, reference)`
    fn prove_artifact(
        &self,
        conn: &Connection,
        run: &DurableGenerationRun,
        reference: &ReviewArtifactRef,
    ) -> Result<ProvedArtifact, String> {
        if reference.artifact_id.is_empty()
            && reference.revision == 0
            && reference.text_hash.is_empty()
        {
            // 基线 `!reference`（空对象）守卫；`artifact` 为必需字段，
            // 此处以「全空」近似基线空对象守卫（契约层 `deny_unknown_fields`
            // 已保证三键齐备，空值组合即基线空对象语义）。
            return Err(mutating_error("GENERATION_REVIEW_ARTIFACT_INVALID"));
        }
        let row: Option<ArtifactRow> = conn
            .query_row(
                "SELECT attempt_id,run_id,status FROM generation_artifacts WHERE artifact_id=?",
                [&reference.artifact_id],
                |row| {
                    Ok(ArtifactRow {
                        attempt_id: row.get("attempt_id")?,
                        run_id: row.get("run_id")?,
                        status: row.get("status")?,
                    })
                },
            )
            .optional()
            .map_err(|error| mutating_error(&error.to_string()))?;
        let Some(row) = row else {
            return Err(mutating_error("GENERATION_REVIEW_ARTIFACT_INVALID"));
        };
        if row.run_id != run.run_id || row.status == "discarded" {
            return Err(mutating_error("GENERATION_REVIEW_ARTIFACT_INVALID"));
        }
        let receipt = self
            .runs
            .receipt(conn, &row.attempt_id, None)
            .map_err(|error| mutating_error(&error))?;
        let Some(artifact) = receipt.artifact.clone() else {
            return Err(mutating_error("GENERATION_REVIEW_ARTIFACT_INVALID"));
        };
        if artifact.revision != reference.revision
            || artifact.text_hash != reference.text_hash
            || text_hash(&artifact.text) != reference.text_hash
            || receipt.failure_code.is_some()
            || receipt
                .result
                .as_ref()
                .and_then(|result| result.finish_reason.as_deref())
                != Some("stop")
            || !["settled", "unknown"].contains(&receipt.attempt.status.as_str())
        {
            return Err(mutating_error("GENERATION_REVIEW_ARTIFACT_INVALID"));
        }
        Ok(ProvedArtifact { row, artifact })
    }

    /// 基线 `reviewBody(run, reference, reportVersion = 2)`
    fn review_body(
        &self,
        conn: &Connection,
        run: &DurableGenerationRun,
        reference: &ReviewArtifactRef,
        report_version: i64,
    ) -> Result<String, String> {
        let context = self.context_of(run)?;
        let proved = self.prove_artifact(conn, run, reference)?;
        if let Some(recheck) = &context.recheck {
            let attempt = self
                .attempts(conn, Some(&run.run_id))?
                .into_iter()
                .find(|candidate| candidate.attempt_id == proved.row.attempt_id);
            let mut persisted_version: Option<i64> = None;
            if let Some(attempt) = &attempt {
                if let Ok(usage) = serde_json::from_str::<Value>(&attempt.usage_receipt_json) {
                    if let Some(receipt) = usage.get("reviewCycleRecheck") {
                        let version = receipt.get("version").and_then(Value::as_i64);
                        if version == Some(1) || version == Some(2) {
                            persisted_version = version;
                        }
                    }
                }
            }
            // 基线：无效的持久化回执在后续证明检查中失败，不降级当前策略。
            let policy = RecheckPolicy {
                context: recheck_policy_context(recheck),
                version: persisted_version.unwrap_or(2),
            };
            let built = build_review_cycle_recheck_report(
                &proved.artifact.text,
                &context.source.content,
                &policy,
                &context.ui_locale,
                report_version,
            );
            let body = ReviewReportBody {
                summary: built.summary,
                items: built.items,
            };
            return Ok(serde_json::to_string_pretty(&body).expect("复核报告 JSON 序列化不得失败"));
        }
        let built = build_review_generation_report(&BuildReviewGenerationReportInput {
            content: proved.artifact.text.clone(),
            source_content: context.source.content.clone(),
            frozen_goals: context.frozen_goals.clone(),
            writing_language: context.writing_language.clone(),
            ui_locale: context.ui_locale.clone(),
            preflight_findings: context.preflight_findings.clone(),
            report_version,
        });
        Ok(serde_json::to_string_pretty(&built).expect("审稿报告 JSON 序列化不得失败"))
    }

    /// 基线 `saved(run)`
    fn saved(
        &self,
        conn: &Connection,
        run: &DurableGenerationRun,
    ) -> Result<Option<SavedEffect>, String> {
        let rows = self
            .attempts(conn, Some(&run.run_id))?
            .into_iter()
            .filter(|row| {
                serde_json::from_str::<Value>(&row.usage_receipt_json)
                    .ok()
                    .and_then(|usage| usage.get("reviewRevisionEffect").cloned())
                    .is_some()
            })
            .collect::<Vec<_>>();
        if rows.len() > 1 {
            return Err(mutating_error("GENERATION_REVIEW_RECEIPT_INVALID"));
        }
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let effect: Effect = serde_json::from_str::<Value>(&row.usage_receipt_json)
            .ok()
            .and_then(|usage| usage.get("reviewRevisionEffect").cloned())
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| mutating_error("GENERATION_REVIEW_RECEIPT_INVALID"))?;
        let context = self.context_of(run)?;
        let proof = self.prove_artifact(conn, run, &effect.artifact)?;
        if effect.report_version.is_some()
            && (effect.report_version != Some(2) || effect.kind != "review")
        {
            return Err(mutating_error("GENERATION_REVIEW_RECEIPT_INVALID"));
        }
        if proof.row.attempt_id != row.attempt_id
            || effect.context_hash != Self::context_hash(&context)
            || effect.kind
                != if context.operation == ReviewRevisionOperation::ReviewChapter {
                    "review"
                } else {
                    "revision"
                }
        {
            return Err(mutating_error("GENERATION_REVIEW_RECEIPT_INVALID"));
        }
        let row_full = if effect.kind == "review" {
            get_review_full_repo(conn, effect.id)
                .map_err(|error| mutating_error(&error))?
                .map(ReviewRowFull::Review)
        } else {
            get_revision_full_repo(conn, effect.id)
                .map_err(|error| mutating_error(&error))?
                .map(ReviewRowFull::Revision)
        };
        let index = match &row_full {
            Some(ReviewRowFull::Review(row)) => row.meta.review_index,
            Some(ReviewRowFull::Revision(row)) => row.meta.revision_index,
            None => 0,
        };
        let composition = if effect.kind == "revision" {
            self.runs
                .read_visible_composition(conn, &run.run_id)
                .map_err(|error| mutating_error(&error))?
        } else {
            None
        };
        let body = if effect.kind == "review" {
            Some(self.review_body(
                conn,
                run,
                &effect.artifact,
                effect.report_version.unwrap_or(1),
            )?)
        } else {
            composition
                .as_ref()
                .map(|composition| composition.text.trim().to_string())
        };
        let row_full = match row_full {
            Some(row_full) => row_full,
            None => {
                return Err(mutating_error("GENERATION_REVIEW_RECEIPT_INVALID"));
            }
        };
        let (base_draft_id, source_draft, content, revision_status) = match &row_full {
            ReviewRowFull::Review(row) => (
                row.meta.base_draft_id,
                row.source_draft.clone(),
                row.content.clone(),
                None,
            ),
            ReviewRowFull::Revision(row) => (
                row.meta.base_draft_id,
                row.source_draft.clone(),
                row.content.clone(),
                Some(row.meta.status.clone()),
            ),
        };
        if base_draft_id != context.source.id
            || index != effect.index
            || !is_deep_strict_equal(
                &serde_json::to_value(&source_draft).unwrap_or(Value::Null),
                &serde_json::to_value(&context.source).unwrap_or(Value::Null),
            )
            || effect.content_hash != text_hash(&content)
            || content != body.unwrap_or_default()
            || (effect.kind == "revision"
                && (composition
                    .as_ref()
                    .map(|composition| composition.algorithm.as_str())
                    != Some("visible-append-v1")
                    || composition
                        .as_ref()
                        .map(|composition| composition.text_hash.as_str())
                        != Some(effect.composition_hash.as_deref().unwrap_or_default())
                    || composition
                        .and_then(|composition| composition.artifact_ids.last().cloned())
                        != Some(effect.artifact.artifact_id.clone())))
        {
            return Err(mutating_error("GENERATION_REVIEW_RECEIPT_INVALID"));
        }
        if let Some(revision_status) = &revision_status {
            if !["pending", "merged", "discarded"].contains(&revision_status.as_str()) {
                return Err(mutating_error("GENERATION_REVIEW_RECEIPT_INVALID"));
            }
        }
        let cycle = match &context.recheck {
            Some(recheck) => Some(
                plan_recheck_repo(&recheck.cycle_id, conn)
                    .map_err(|error| mutating_error(&error))?,
            ),
            None => None,
        };
        let effect_content_hash = effect.content_hash.clone();
        Ok(Some(SavedEffect {
            effect,
            receipt: ReviewRevisionCommitReceipt {
                success: true,
                kind: if context.operation == ReviewRevisionOperation::ReviewChapter {
                    "review"
                } else {
                    "revision"
                }
                .to_string(),
                id: match &row_full {
                    ReviewRowFull::Review(row) => row.meta.id,
                    ReviewRowFull::Revision(row) => row.meta.id,
                },
                index,
                content: match &row_full {
                    ReviewRowFull::Review(row) => row.content.clone(),
                    ReviewRowFull::Revision(row) => row.content.clone(),
                },
                content_hash: effect_content_hash,
                source: context.source.clone(),
                revision_status: revision_status.clone(),
                review_cycle: context
                    .recheck
                    .as_ref()
                    .map(|recheck| ReviewRevisionCommitCycle {
                        cycle_id: recheck.cycle_id.clone(),
                        revision_status: "merge-committed".to_string(),
                        recheck_count: if cycle.as_ref().map(|cycle| cycle.disposition)
                            == Some(review_cycle::RecheckDisposition::Completed)
                        {
                            1
                        } else {
                            0
                        },
                        disposition: cycle
                            .as_ref()
                            .map(|cycle| match cycle.disposition {
                                review_cycle::RecheckDisposition::Required => "required",
                                review_cycle::RecheckDisposition::NotRequired => "not-required",
                                review_cycle::RecheckDisposition::Completed => "completed",
                            })
                            .unwrap_or("required")
                            .to_string(),
                    }),
            },
        }))
    }

    /// 基线 `parent(context)`
    fn parent(
        &self,
        conn: &Connection,
        context: &ReviewRevisionContext,
    ) -> Result<Lineage, String> {
        if let Some(recheck) = &context.recheck {
            let plan = plan_recheck_repo(&recheck.cycle_id, conn)
                .map_err(|error| mutating_error(&error))?;
            let frozen_plan = plan
                .context
                .as_ref()
                .map(|plan_context| RecheckContextFrozen {
                    context: plan_context.clone(),
                    version: context
                        .recheck
                        .as_ref()
                        .map(|recheck| recheck.version)
                        .unwrap_or(2),
                });
            let Some(frozen_plan) = frozen_plan else {
                return Err(mutating_error("GENERATION_REVIEW_LINEAGE_UNPROVEN"));
            };
            if plan.disposition != review_cycle::RecheckDisposition::Required
                || !is_deep_strict_equal(
                    &serde_json::to_value(&RecheckPolicyView {
                        version: frozen_plan.version,
                        cycle_id: frozen_plan.context.cycle_id.clone(),
                        comparison_version: frozen_plan.context.comparison_version,
                        merged_hash: frozen_plan.context.merged_hash.clone(),
                        finding_set_hash: frozen_plan.context.finding_set_hash.clone(),
                        source_content: frozen_plan.context.source_content.clone(),
                        findings: frozen_plan.context.findings.clone(),
                    })
                    .unwrap_or(Value::Null),
                    &serde_json::to_value(&RecheckPolicyView {
                        version: recheck.version,
                        cycle_id: recheck.cycle_id.clone(),
                        comparison_version: recheck.comparison_version,
                        merged_hash: recheck.merged_hash.clone(),
                        finding_set_hash: recheck.finding_set_hash.clone(),
                        source_content: recheck.source_content.clone(),
                        findings: recheck
                            .findings
                            .iter()
                            .map(recheck_policy_finding)
                            .collect(),
                    })
                    .unwrap_or(Value::Null),
                )
            {
                return Err(mutating_error("GENERATION_REVIEW_LINEAGE_UNPROVEN"));
            }
            let matching = self
                .attempts(conn, None)?
                .into_iter()
                .filter(|row| {
                    serde_json::from_str::<Value>(&row.usage_receipt_json)
                        .ok()
                        .and_then(|usage| usage.get("reviewRevisionEffect").cloned())
                        .and_then(|value| serde_json::from_value::<Effect>(value).ok())
                        .map(|effect| effect.kind == "review" && effect.id == plan.review_id)
                        .unwrap_or(false)
                })
                .collect::<Vec<_>>();
            if matching.len() != 1 {
                return Err(mutating_error("GENERATION_REVIEW_LINEAGE_UNPROVEN"));
            }
            let parent_run = self
                .runs
                .get(conn, &matching[0].run_id)
                .map_err(|error| mutating_error(&error))?;
            if parent_run.root_action_id != plan.root_action_id
                || parent_run.binding.project_id != self.scope.project_id
            {
                return Err(mutating_error("GENERATION_REVIEW_LINEAGE_UNPROVEN"));
            }
            let model_id = parent_run
                .binding
                .source_manifest
                .get("modelReceipt")
                .and_then(|receipt| receipt.get("modelId"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_default();
            return Ok(Lineage {
                parent_root_action_id: Some(plan.root_action_id.clone()),
                model_id: Some(model_id),
            });
        }
        let Some(confirmation) = &context.confirmation else {
            return Ok(Lineage::default());
        };
        let matching = self
            .attempts(conn, None)?
            .into_iter()
            .filter(|row| {
                serde_json::from_str::<Value>(&row.usage_receipt_json)
                    .ok()
                    .and_then(|usage| usage.get("reviewRevisionEffect").cloned())
                    .and_then(|value| serde_json::from_value::<Effect>(value).ok())
                    .map(|effect| {
                        effect.kind == "review"
                            && effect.id == confirmation.snapshot.source_review_id
                    })
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            return Err(mutating_error("GENERATION_REVIEW_LINEAGE_UNPROVEN"));
        }
        let run = self
            .runs
            .get(conn, &matching[0].run_id)
            .map_err(|error| mutating_error(&error))?;
        let saved = self.saved(conn, &run)?;
        if run.binding.project_id != self.scope.project_id
            || saved.as_ref().map(|saved| saved.receipt.kind.as_str()) != Some("review")
            || saved
                .as_ref()
                .map(|saved| saved.receipt.content_hash.as_str())
                != Some(confirmation.original_review_content_hash.as_str())
            || !is_deep_strict_equal(
                &serde_json::to_value(&saved.map(|saved| saved.receipt.source))
                    .unwrap_or(Value::Null),
                &serde_json::to_value(&context.source).unwrap_or(Value::Null),
            )
        {
            return Err(mutating_error("GENERATION_REVIEW_LINEAGE_UNPROVEN"));
        }
        let model_id = run
            .binding
            .source_manifest
            .get("modelReceipt")
            .and_then(|receipt| receipt.get("modelId"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_default();
        Ok(Lineage {
            parent_root_action_id: Some(run.root_action_id.clone()),
            model_id: Some(model_id),
        })
    }

    /// 基线 `assertPrepared(contextId, run)`
    fn assert_prepared(&self, context_id: &str, run: &DurableGenerationRun) -> Result<(), String> {
        let contexts = PREPARED_CONTEXTS
            .lock()
            .expect("审稿修稿上下文缓存不得被污染");
        let cached = contexts
            .get(context_id)
            .map(|prepared| prepared.context.clone());
        drop(contexts);
        let cached = cached.ok_or_else(|| mutating_error("GENERATION_REVIEW_CONTEXT_REQUIRED"))?;
        if !is_deep_strict_equal(
            &serde_json::to_value(&cached).unwrap_or(Value::Null),
            &serde_json::to_value(&self.context_of(run)?).unwrap_or(Value::Null),
        ) {
            return Err(mutating_error("GENERATION_REVIEW_CONTEXT_REQUIRED"));
        }
        Ok(())
    }

    /// 基线 `record(run, artifact, effect)`
    fn record(
        &self,
        conn: &Connection,
        run: &DurableGenerationRun,
        artifact: &ReviewArtifactRef,
        effect: EffectInput,
    ) -> Result<ReviewRevisionCommitReceipt, String> {
        let proof = self.prove_artifact(conn, run, artifact)?;
        let row = self
            .attempts(conn, Some(&run.run_id))?
            .into_iter()
            .find(|row| row.attempt_id == proof.row.attempt_id)
            .ok_or_else(|| mutating_error("GENERATION_REVIEW_RECEIPT_INVALID"))?;
        let mut usage: Value = serde_json::from_str(&row.usage_receipt_json)
            .map_err(|_| mutating_error("GENERATION_REVIEW_RECEIPT_INVALID"))?;
        let context = self.context_of(run)?;
        let effect_value = serde_json::to_value(&Effect {
            kind: effect.kind.clone(),
            id: effect.id,
            index: effect.index,
            content_hash: effect.content_hash.clone(),
            context_hash: Self::context_hash(&context),
            artifact: artifact.clone(),
            composition_hash: effect.composition_hash.clone(),
            report_version: effect.report_version,
        })
        .expect("效果 JSON 序列化不得失败");
        if let Some(object) = usage.as_object_mut() {
            object.insert("reviewRevisionEffect".to_string(), effect_value);
        }
        conn.execute(
            "UPDATE generation_attempts SET usage_receipt_json=? WHERE attempt_id=?",
            [
                serde_json::to_string(&usage).expect("usage JSON 序列化不得失败"),
                row.attempt_id.clone(),
            ],
        )
        .map_err(|error| mutating_error(&error.to_string()))?;
        let saved = self
            .saved(conn, run)?
            .ok_or_else(|| mutating_error("GENERATION_REVIEW_RECEIPT_INVALID"))?;
        Ok(saved.receipt)
    }

    // ===== 基线 4 个公开方法 =====

    /// 基线 `prepare(request)`
    pub fn prepare(
        &self,
        conn: &Connection,
        request: &crate::services::review_revision_context::PrepareReviewRevisionRequest,
        assert_current: impl FnOnce() -> Result<(), String>,
    ) -> Result<PreparedReviewRevisionContext, String> {
        assert_current().map_err(|_| mutating_error("GENERATION_OWNER_CLOSED"))?;
        let scope = self.scope.clone();
        let runs = self.runs.clone();
        let result = in_transaction(conn, |tx| {
            let context = capture_review_revision_context(tx, request, &scope.project_id, None)
                .map_err(|error| mutating_error(&error))?;
            let lineage = Self::parent_in(tx, &runs, &scope, &context)?;
            Ok(Self::remember(&context, lineage))
        })?;
        Ok(result)
    }

    /// `parent` 的事务内调用辅助（`prepare` 的事务包裹等价物）。
    fn parent_in(
        conn: &Connection,
        runs: &GenerationRunRepository,
        scope: &ReviewRevisionScope,
        context: &ReviewRevisionContext,
    ) -> Result<Lineage, String> {
        let probe = ReviewRevisionGeneration {
            runs: runs.clone(),
            scope: scope.clone(),
        };
        probe.parent(conn, context)
    }

    /// 基线 `commitReview(request, assertSources)`
    pub fn commit_review(
        &self,
        conn: &Connection,
        request: &ReviewGenerationCommitRequest,
        assert_sources: impl FnOnce(&MainGenerationRunHandle) -> Result<(), String>,
    ) -> Result<ReviewRevisionCommitReceipt, String> {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .map_err(|error| mutating_error(&error.to_string()))?;
        let receipt = self.commit_review_in_transaction(&tx, request, assert_sources)?;
        tx.commit()
            .map_err(|error| mutating_error(&error.to_string()))?;
        Ok(receipt)
    }

    fn commit_review_in_transaction(
        &self,
        conn: &rusqlite::Transaction<'_>,
        request: &ReviewGenerationCommitRequest,
        assert_sources: impl FnOnce(&MainGenerationRunHandle) -> Result<(), String>,
    ) -> Result<ReviewRevisionCommitReceipt, String> {
        if request.context_id.is_empty() && request.artifact.artifact_id.is_empty() {
            // 基线 `!request` 空对象守卫；契约层 `deny_unknown_fields`
            // 已保证三键齐备，此处以「全空」近似（与 `prove_artifact` 同例）。
            return Err(mutating_error("GENERATION_REVIEW_COMMIT_INVALID"));
        }
        let run = self.require_run(conn, &request.handle, || Ok(()))?;
        let context = self.context_of(&run)?;
        let saved = self.saved(conn, &run)?;
        if context.operation != ReviewRevisionOperation::ReviewChapter {
            return Err(mutating_error("GENERATION_REVIEW_COMMIT_INVALID"));
        }
        if let Some(saved) = saved {
            if !is_deep_strict_equal(
                &serde_json::to_value(&saved.effect.artifact).unwrap_or(Value::Null),
                &serde_json::to_value(&request.artifact).unwrap_or(Value::Null),
            ) {
                return Err(mutating_error("GENERATION_REVIEW_COMMIT_CONFLICT"));
            }
            return Ok(saved.receipt);
        }
        self.assert_prepared(&request.context_id, &run)?;
        assert_sources(&request.handle).map_err(|_| mutating_error("GENERATION_SOURCE_CHANGED"))?;
        let proof = self.prove_artifact(conn, &run, &request.artifact)?;
        let content = self.review_body(conn, &run, &request.artifact, 2)?;
        let result = create_review_repo(
            conn,
            &ReviewCreateParams {
                base_draft_id: context.source.id,
                review_index: None,
                content: content.clone(),
                expected_source: Some(context.source.clone()),
            },
        )
        .map_err(|error| mutating_error(&error))?;
        let content_hash = text_hash(&content);
        let receipt = self.record(
            conn,
            &run,
            &request.artifact,
            EffectInput {
                kind: "review".to_string(),
                id: result.id,
                index: result.review_index,
                content_hash: content_hash.clone(),
                composition_hash: None,
                report_version: Some(2),
            },
        )?;
        if let Some(recheck) = &context.recheck {
            let policy = RecheckPolicy {
                context: recheck_policy_context(recheck),
                version: 2,
            };
            let built = build_review_cycle_recheck_report(
                &proof.artifact.text,
                &context.source.content,
                &policy,
                &context.ui_locale,
                2,
            );
            let findings = built
                .decisions
                .into_iter()
                .filter(|decision| decision.status != "unknown")
                .map(|decision| {
                    let item = built
                        .items
                        .get(decision.review_item_index as usize)
                        .cloned()
                        .unwrap_or_default();
                    let resolved = decision.status == "resolved";
                    RecheckFinding {
                        finding_id: decision.finding_id.clone(),
                        target_id: decision.target_id.clone(),
                        review_item_index: decision.review_item_index,
                        evidence_hash: text_hash(
                            &serde_json::to_string(&RecheckEvidencePayload {
                                finding_id: decision.finding_id.clone(),
                                target_id: decision.target_id.clone(),
                                review_item_index: decision.review_item_index,
                                item,
                                resolved,
                            })
                            .expect("证据负载 JSON 序列化不得失败"),
                        ),
                        resolved,
                    }
                })
                .collect::<Vec<_>>();
            let attempt = self
                .attempts(conn, Some(&run.run_id))?
                .into_iter()
                .find(|row| row.attempt_id == proof.row.attempt_id)
                .ok_or_else(|| mutating_error("GENERATION_REVIEW_RECEIPT_INVALID"))?;
            let mut usage: Value = serde_json::from_str(&attempt.usage_receipt_json)
                .map_err(|_| mutating_error("GENERATION_REVIEW_RECEIPT_INVALID"))?;
            let recheck_value = serde_json::to_value(&PersistedRecheckReceipt {
                version: policy.version,
                cycle_id: recheck.cycle_id.clone(),
                comparison_version: recheck.comparison_version,
                merged_hash: recheck.merged_hash.clone(),
                finding_set_hash: recheck.finding_set_hash.clone(),
                findings,
            })
            .expect("复核回执 JSON 序列化不得失败");
            if let Some(object) = usage.as_object_mut() {
                object.insert("reviewCycleRecheck".to_string(), recheck_value);
            }
            conn.execute(
                "UPDATE generation_attempts SET usage_receipt_json=? WHERE attempt_id=?",
                [
                    serde_json::to_string(&usage).expect("usage JSON 序列化不得失败"),
                    attempt.attempt_id.clone(),
                ],
            )
            .map_err(|error| mutating_error(&error.to_string()))?;
            commit_recheck_repo(&recheck.cycle_id, &attempt.attempt_id, conn)
                .map_err(|error| mutating_error(&error))?;
            return self
                .saved(conn, &run)?
                .map(|saved| saved.receipt)
                .ok_or_else(|| mutating_error("GENERATION_REVIEW_RECEIPT_INVALID"));
        }
        create_review_cycle_repo(
            conn,
            &CreateReviewCycleInput {
                root_action_id: run.root_action_id.clone(),
                review_id: result.id,
                review_content_hash: content_hash.clone(),
                source: ReviewCycleSource {
                    project_id: run.binding.project_id.clone(),
                    epoch: run.binding.epoch.clone(),
                    source_id: format!("draft:{}", context.source.id),
                    revision: context.source.version,
                    content_hash: text_hash(&context.source.content),
                },
            },
        )
        .map_err(|error| mutating_error(&error))?;
        Ok(receipt)
    }

    /// 基线 `commitRevision(request, assertSources)`
    pub fn commit_revision(
        &self,
        conn: &Connection,
        request: &RevisionGenerationCommitRequest,
        assert_sources: impl FnOnce(&MainGenerationRunHandle) -> Result<(), String>,
    ) -> Result<ReviewRevisionCommitReceipt, String> {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .map_err(|error| mutating_error(&error.to_string()))?;
        let receipt = self.commit_revision_in_transaction(&tx, request, assert_sources)?;
        tx.commit()
            .map_err(|error| mutating_error(&error.to_string()))?;
        Ok(receipt)
    }

    fn commit_revision_in_transaction(
        &self,
        conn: &rusqlite::Transaction<'_>,
        request: &RevisionGenerationCommitRequest,
        assert_sources: impl FnOnce(&MainGenerationRunHandle) -> Result<(), String>,
    ) -> Result<ReviewRevisionCommitReceipt, String> {
        if request.context_id.is_empty() && request.expected_composition_hash.is_empty() {
            return Err(mutating_error("GENERATION_REVIEW_COMMIT_INVALID"));
        }
        let run = self.require_run(conn, &request.handle, || Ok(()))?;
        let context = self.context_of(&run)?;
        let saved = self.saved(conn, &run)?;
        if context.operation == ReviewRevisionOperation::ReviewChapter {
            return Err(mutating_error("GENERATION_REVIEW_COMMIT_INVALID"));
        }
        if let Some(saved) = saved {
            if saved.effect.composition_hash.as_deref()
                != Some(request.expected_composition_hash.as_str())
            {
                return Err(mutating_error("GENERATION_REVIEW_COMMIT_CONFLICT"));
            }
            return Ok(saved.receipt);
        }
        self.assert_prepared(&request.context_id, &run)?;
        assert_sources(&request.handle).map_err(|_| mutating_error("GENERATION_SOURCE_CHANGED"))?;
        let composition = self
            .runs
            .read_visible_composition(conn, &run.run_id)
            .map_err(|error| mutating_error(&error))?;
        let Some(composition) = composition else {
            return Err(mutating_error("GENERATION_REVIEW_COMPOSITION_REQUIRED"));
        };
        if composition.algorithm != "visible-append-v1"
            || composition.text_hash != request.expected_composition_hash
        {
            return Err(mutating_error("GENERATION_REVIEW_COMPOSITION_REQUIRED"));
        }
        let Some(artifact) = composition.sources.last() else {
            return Err(mutating_error("GENERATION_REVIEW_COMPOSITION_REQUIRED"));
        };
        let artifact_ref = ReviewArtifactRef {
            artifact_id: artifact.artifact_id.clone(),
            revision: artifact.revision,
            text_hash: artifact.text_hash.clone(),
        };
        self.prove_artifact(conn, &run, &artifact_ref)?;
        let content = composition.text.trim().to_string();
        if content.is_empty() {
            return Err(mutating_error("GENERATION_REVIEW_REVISION_EMPTY"));
        }
        assert_materially_complete_revision(
            &context.source.content,
            &content,
            context.config.words_per_chapter,
            &context.ui_locale,
        )
        .map_err(|error| mutating_error(&error))?;
        if is_noop_revision(&context.source.content, &content) {
            return Err(mutating_error("GENERATION_REVIEW_REVISION_NOOP"));
        }
        let result = replace_pending_repo(
            conn,
            &RevisionCreateParams {
                base_draft_id: context.source.id,
                revision_type: if context.operation == ReviewRevisionOperation::RefineFromReview {
                    "review-fix".to_string()
                } else {
                    "refine".to_string()
                },
                user_prompt: Some(
                    context
                        .confirmation
                        .as_ref()
                        .map(|confirmation| confirmation.snapshot.author_guidance.clone())
                        .or_else(|| {
                            context
                                .author_inputs
                                .iter()
                                .find(|input| input.id == "user-prompt")
                                .map(|input| input.text.clone())
                        })
                        .unwrap_or_default(),
                ),
                content: content.clone(),
                word_count: count_draft_units(&content),
                expected_source: Some(context.source.clone()),
                review_source_id: context
                    .confirmation
                    .as_ref()
                    .map(|confirmation| confirmation.review_source_id),
            },
        )
        .map_err(|error| mutating_error(&error))?;
        let receipt = self.record(
            conn,
            &run,
            &artifact_ref,
            EffectInput {
                kind: "revision".to_string(),
                id: result.id,
                index: result.revision_index,
                content_hash: text_hash(&content),
                composition_hash: Some(composition.text_hash.clone()),
                report_version: None,
            },
        )?;
        if let Some(confirmation) = &context.confirmation {
            let cycle: Option<CycleRow> = conn
                .query_row(
                    "SELECT cycle_id,root_action_id FROM review_cycles WHERE review_id=?",
                    [confirmation.snapshot.source_review_id],
                    |row| {
                        Ok(CycleRow {
                            cycle_id: row.get("cycle_id")?,
                            root_action_id: row.get("root_action_id")?,
                        })
                    },
                )
                .optional()
                .map_err(|error| mutating_error(&error.to_string()))?;
            let Some(cycle) = cycle else {
                return Err(mutating_error("REVIEW_CYCLE_INPUT_INVALID"));
            };
            attach_generated_revision_repo(
                conn,
                &AttachGeneratedRevisionInput {
                    cycle_id: cycle.cycle_id.clone(),
                    root_action_id: cycle.root_action_id.clone(),
                    confirmation_review_id: confirmation.review_source_id,
                    confirmation_content_hash: text_hash(&confirmation.content),
                    revision_id: result.id,
                    revision_content_hash: text_hash(&content),
                },
            )
            .map_err(|error| mutating_error(&error))?;
        }
        Ok(receipt)
    }

    /// 基线 `readRecovery(handle, sourcesMatch, snapshot)`
    pub fn read_recovery(
        &self,
        conn: &Connection,
        handle: &MainGenerationRunHandle,
        sources_match: impl Fn(&DurableGenerationRun) -> bool,
        snapshot: impl Fn(&DurableGenerationRun) -> Option<Value>,
    ) -> Result<ReviewRevisionRecovery, String> {
        let run = self.require_run(conn, handle, || Ok(()))?;
        let context = self.context_of(&run)?;
        let saved = self.saved(conn, &run)?.map(|saved| saved.receipt);
        // 基线：源/模型/模板变更时原始候选仍可用（保守保留）。
        let current =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sources_match(&run)))
                .unwrap_or(false);
        let composition = self
            .runs
            .read_visible_composition(conn, &run.run_id)
            .map_err(|error| mutating_error(&error))?;
        let latest_artifact = snapshot(&run);
        let last = composition
            .as_ref()
            .and_then(|composition| composition.artifact_ids.last().cloned());
        let finish = |conn: &Connection, id: &Option<String>| -> Option<String> {
            let Some(id) = id else {
                return None;
            };
            let attempt: String = conn
                .query_row(
                    "SELECT attempt_id FROM generation_artifacts WHERE artifact_id=?",
                    [id],
                    |row| row.get(0),
                )
                .optional()
                .ok()
                .flatten()?;
            let receipt = self.runs.receipt(conn, &attempt, None).ok()?;
            receipt.result.and_then(|result| result.finish_reason)
        };
        let last_composition_finish_reason = finish(conn, &last);
        let latest_artifact_finish_reason = latest_artifact
            .as_ref()
            .and_then(|artifact| {
                artifact
                    .get("artifactId")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .and_then(|artifact_id| finish(conn, &Some(artifact_id)));
        let attempted_purposes = self
            .attempts(conn, Some(&run.run_id))?
            .into_iter()
            .map(|row| {
                serde_json::from_str::<Value>(&row.usage_receipt_json)
                    .ok()
                    .and_then(|usage| {
                        usage
                            .get("purpose")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "unknown".to_string())
            })
            .collect::<Vec<_>>();
        let mut can_resume = current;
        if saved.is_none()
            && context.operation == ReviewRevisionOperation::ReviewChapter
            && latest_artifact_finish_reason.as_deref() == Some("length")
            && attempted_purposes
                .iter()
                .filter(|purpose| purpose.as_str() == "review-chapter")
                .count()
                >= 2
            && !attempted_purposes
                .iter()
                .any(|purpose| purpose.as_str() == "review-chapter-rebuild")
        {
            can_resume = false;
        }
        if saved.is_none()
            && context.operation != ReviewRevisionOperation::ReviewChapter
            && composition.is_some()
            && last_composition_finish_reason.as_deref() == Some("stop")
        {
            let composition_text = composition
                .as_ref()
                .map(|composition| composition.text.clone())
                .unwrap_or_default();
            let complete = assert_materially_complete_revision(
                &context.source.content,
                &composition_text,
                context.config.words_per_chapter,
                &context.ui_locale,
            )
            .is_ok()
                && !is_noop_revision(&context.source.content, &composition_text.trim());
            if !complete {
                can_resume = false;
            }
        }
        let context_id = if can_resume && saved.is_none() {
            Some(Self::remember(&context, self.parent(conn, &context)?).context_id)
        } else {
            None
        };
        Ok(ReviewRevisionRecovery {
            handle: Self::handle_of(&run),
            context,
            model_id: run
                .binding
                .source_manifest
                .get("modelReceipt")
                .and_then(|receipt| receipt.get("modelId"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_default(),
            source_status: if current { "current" } else { "conflict" }.to_string(),
            can_resume,
            context_id,
            saved,
            composition,
            last_composition_finish_reason,
            latest_artifact,
            latest_artifact_finish_reason,
            attempted_purposes,
        })
    }
}

// ===== 内部类型 =====

/// 基线 `Effect`（`generation_attempts.usage_receipt_json.reviewRevisionEffect`）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Effect {
    kind: String,
    id: i64,
    index: i64,
    content_hash: String,
    context_hash: String,
    artifact: ReviewArtifactRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    composition_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    report_version: Option<i64>,
}

/// 基线 `record` 的入参效果（`Omit<Effect, 'artifact' | 'contextHash'>`）
#[derive(Debug, Clone)]
struct EffectInput {
    kind: String,
    id: i64,
    index: i64,
    content_hash: String,
    composition_hash: Option<String>,
    report_version: Option<i64>,
}

/// 基线 `parent` 的返回（`{ parentRootActionId?, modelId? }`）
#[derive(Debug, Clone, Default)]
struct Lineage {
    parent_root_action_id: Option<String>,
    model_id: Option<String>,
}

/// 基线 `AttemptRow`
#[derive(Debug, Clone)]
struct AttemptRow {
    attempt_id: String,
    run_id: String,
    usage_receipt_json: String,
}

/// 基线 `proveArtifact` 的 `generation_artifacts` 行
#[derive(Debug, Clone)]
struct ArtifactRow {
    attempt_id: String,
    run_id: String,
    status: String,
}

/// 基线 `proveArtifact` 的返回
#[derive(Debug, Clone)]
struct ProvedArtifact {
    row: ArtifactRow,
    artifact: VisibleArtifact,
}
/// 基线 `saved` 的返回
#[derive(Debug, Clone)]
struct SavedEffect {
    effect: Effect,
    receipt: ReviewRevisionCommitReceipt,
}

/// 审稿 / 修稿完整行的联合（基线 `ReviewRepository.getFull` / `RevisionRepository.getFull`）
#[derive(Debug, Clone)]
enum ReviewRowFull {
    Review(review_repo::ReviewFull),
    Revision(revision_repo::RevisionFull),
}

/// 基线 `review_cycles` 的 `cycle_id, root_action_id` 行
#[derive(Debug, Clone)]
struct CycleRow {
    cycle_id: String,
    root_action_id: String,
}

/// 基线 `buildReviewCycleRecheckReport` 的策略输入
/// （`{ ...context.recheck, version: persistedVersion ?? 2 }`）
#[derive(Debug, Clone)]
struct RecheckPolicy {
    context: ReviewCycleRecheckContext,
    version: i64,
}

/// 基线 `parent` 中 `frozenPlan` 的中间视图（`{ ...plan.context, version }`）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecheckPolicyView {
    version: i64,
    cycle_id: String,
    comparison_version: i64,
    merged_hash: String,
    finding_set_hash: String,
    source_content: Option<String>,
    findings: Vec<review_cycle::ReviewCycleRecheckFinding>,
}

/// `RecheckFindingView` → `ReviewCycleRecheckFinding` 的逐字段转换
/// （视图与仓储类型字段逐字同形，仅 `Deserialize` 派生差异）。
fn recheck_policy_finding(
    view: &crate::services::review_revision_context::RecheckFindingView,
) -> review_cycle::ReviewCycleRecheckFinding {
    review_cycle::ReviewCycleRecheckFinding {
        finding_id: view.finding_id.clone(),
        target_id: view.target_id.clone(),
        category: view.category.clone(),
        kind: view.kind,
        problem: view.problem.clone(),
        expected: view.expected.clone(),
        source_span: view
            .source_span
            .as_ref()
            .map(|span| review_cycle::FindingSpan {
                start: span.start,
                end: span.end,
                unit: span.unit.clone(),
            }),
        occurrence: view.occurrence,
        source_excerpt: view.source_excerpt.clone(),
        must_show: view.must_show,
    }
}

/// `RecheckContextView` → `ReviewCycleRecheckContext` 的逐字段转换
/// （`RecheckPolicy` 的策略输入为仓储类型；视图由捕获层反序列化而来）。
fn recheck_policy_context(
    view: &crate::services::review_revision_context::RecheckContextView,
) -> ReviewCycleRecheckContext {
    ReviewCycleRecheckContext {
        version: view.version,
        cycle_id: view.cycle_id.clone(),
        comparison_version: view.comparison_version,
        merged_hash: view.merged_hash.clone(),
        finding_set_hash: view.finding_set_hash.clone(),
        source_content: view.source_content.clone(),
        findings: view.findings.iter().map(recheck_policy_finding).collect(),
    }
}

/// 基线 `frozenPlan` 的中间结构（`{ ...plan.context, version: context.recheck.version }`）
#[derive(Debug, Clone)]
struct RecheckContextFrozen {
    context: ReviewCycleRecheckContext,
    version: i64,
}

/// 基线 `buildReviewCycleRecheckReport` 的返回（`summary` / `items` 段）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewReportBody {
    summary: String,
    items: Vec<Value>,
}

/// 基线 `buildReviewCycleRecheckReport` 的完整返回（含 `decisions`）
#[derive(Debug, Clone)]
struct RecheckReport {
    summary: String,
    items: Vec<Value>,
    decisions: Vec<RecheckDecision>,
}

/// 基线 `ReviewCycleRecheckDecision`
#[derive(Debug, Clone)]
struct RecheckDecision {
    finding_id: String,
    target_id: String,
    status: String,
    review_item_index: i64,
}

/// 基线 `commitReview` 中 `findings` 的单条
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecheckFinding {
    finding_id: String,
    target_id: String,
    review_item_index: i64,
    evidence_hash: String,
    resolved: bool,
}

/// 基线 `evidenceHash` 的负载（`{ findingId, targetId, reviewItemIndex, item, resolved }`）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecheckEvidencePayload {
    finding_id: String,
    target_id: String,
    review_item_index: i64,
    item: Value,
    resolved: bool,
}

/// 基线 `commitReview` 中持久化的 `reviewCycleRecheck` 回执
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedRecheckReceipt {
    version: i64,
    cycle_id: String,
    comparison_version: i64,
    merged_hash: String,
    finding_set_hash: String,
    findings: Vec<RecheckFinding>,
}

/// 基线 `buildReviewGenerationReport` 的输入
#[derive(Debug, Clone)]
struct BuildReviewGenerationReportInput {
    content: String,
    source_content: String,
    frozen_goals: crate::services::review_revision_context::FrozenChapterGoals,
    writing_language: String,
    ui_locale: String,
    preflight_findings: Vec<crate::services::review_revision_context::ConsistencyFinding>,
    report_version: i64,
}

// ===== 基础设施 =====

/// 基线 `textHash`（SHA-256 十六进制小写）
fn text_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 基线 `isDeepStrictEqual` 的 JSON 等价物（`serde_json::Value` 深度相等；
/// 对象键序无关，对齐 JS `JSON.parse` 后的深度比较语义）
fn is_deep_strict_equal(left: &Value, right: &Value) -> bool {
    left == right
}

/// 基线 `db.transaction(fn).immediate()` 的事务包裹等价物
/// （失败时 `Transaction` drop 即回滚，与基线语义一致；
/// 对齐 `generation_run_repository::in_transaction` 既有惯例）
fn in_transaction<T>(
    conn: &Connection,
    operation: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, String>,
) -> Result<T, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| mutating_error(&error.to_string()))?;
    let result = operation(&tx)?;
    tx.commit()
        .map_err(|error| mutating_error(&error.to_string()))?;
    Ok(result)
}

/// 基线 `mutating_error`（「Error: 」前缀信封，对齐
/// `commands::main_generation_owner::mutating_error`）
fn mutating_error(code: &str) -> String {
    format!("Error: {code}")
}

/// 基线 `randomUUID`（UUID v4）
fn random_uuid_v4() -> String {
    crate::project_access::random_uuid_v4()
}

// ===== 共享纯函数（私有，随调用图平移）=====

/// 基线 `isNoopRevision`（忽略空白与格式控制符后正文相同）
fn is_noop_revision(before: &str, after: &str) -> bool {
    let visible = |value: &str| {
        value
            .chars()
            .filter(|ch| !ch.is_whitespace() && !ch.is_control())
            .collect::<String>()
    };
    visible(before) == visible(after)
}

/// 基线 `assertMateriallyCompleteRevision`
fn assert_materially_complete_revision(
    source: &str,
    revision: &str,
    target_units: i64,
    ui_locale: &str,
) -> Result<(), String> {
    assert_mechanically_complete_visible_text(revision, ui_locale, source)?;
    let source_units = count_visible_prose_units(source);
    let revision_units = count_visible_prose_units(revision);
    let bounded_target = if target_units > 0 {
        source_units.min(target_units)
    } else {
        source_units
    };
    let minimum_units = source_units.min(std::cmp::max(
        MIN_REFINEMENT_UNITS,
        (bounded_target as f64 * MIN_REFINEMENT_COMPLETION_RATIO).floor() as i64,
    ));
    if revision_units < minimum_units {
        return Err(mechanical_completion_message(
            ui_locale,
            "修稿结果明显短于原稿，可能仍不完整，结果未被保存。请重试或缩短本次修稿范围。",
            "The revision is materially shorter than the source and may be incomplete. Nothing was saved; retry or narrow the revision scope.",
        ));
    }
    Ok(())
}

/// 基线 `MIN_REFINEMENT_COMPLETION_RATIO` / `MIN_REFINEMENT_UNITS`
const MIN_REFINEMENT_COMPLETION_RATIO: f64 = 0.6;
const MIN_REFINEMENT_UNITS: i64 = 200;

/// 基线 `countVisibleProseUnits`（英文单词 + 中日韩字符 + 其他可见字符）
fn count_visible_prose_units(text: &str) -> i64 {
    let english_words = regex::Regex::new(r"[A-Za-z]+(?:['’][A-Za-z]+)*")
        .expect("英文单词正则")
        .find_iter(text)
        .count() as i64;
    let without_english_words = regex::Regex::new(r"[A-Za-z]+(?:['’][A-Za-z]+)*")
        .expect("英文单词正则")
        .replace_all(text, "")
        .to_string();
    let chinese_characters =
        regex::Regex::new(r"[\u{3400}-\u{4DBF}\u{4E00}-\u{9FFF}\u{F900}-\u{FAFF}]")
            .expect("中日韩字符正则")
            .find_iter(&without_english_words)
            .count() as i64;
    let other_characters =
        regex::Regex::new(r"[\u{3400}-\u{4DBF}\u{4E00}-\u{9FFF}\u{F900}-\u{FAFF}]")
            .expect("中日韩字符正则")
            .replace_all(&without_english_words, "")
            .to_string();
    let other_count = regex::Regex::new(r"[\s\p{P}\p{S}]")
        .expect("空白与标点正则")
        .replace_all(&other_characters, "")
        .chars()
        .count() as i64;
    chinese_characters + english_words + other_count
}

/// 基线 `visibleProseUnitCount`（字母与数字字符数）
fn visible_prose_unit_count(text: &str) -> i64 {
    regex::Regex::new(r"[\p{L}\p{N}]")
        .expect("可见字符正则")
        .find_iter(text)
        .count() as i64
}

/// 基线 `mechanicalCompletionError` 的消息（`localize` 等价物）
fn mechanical_completion_message(
    ui_locale: &str,
    zh_cn_reason: &str,
    en_us_reason: &str,
) -> String {
    if ui_locale == "zh-CN" {
        format!("AI 输出包含{zh_cn_reason}，可能仍不完整，结果未被保存。")
    } else {
        format!(
            "AI output contains {en_us_reason} and may still be incomplete, so it was not saved."
        )
    }
}

/// 基线 `assertMechanicallyCompleteVisibleText`
fn assert_mechanically_complete_visible_text(
    content: &str,
    ui_locale: &str,
    source: &str,
) -> Result<(), String> {
    let trimmed = content.trim();
    if visible_prose_unit_count(trimmed) == 0 {
        return Err(mechanical_completion_message(
            ui_locale,
            "空白或无可见正文",
            "blank or no visible prose",
        ));
    }
    let code_fence = regex::Regex::new(r"(?:^|\n)\s*```").expect("代码围栏正则");
    if code_fence.is_match(trimmed) {
        return Err(mechanical_completion_message(
            ui_locale,
            "代码围栏",
            "a code fence",
        ));
    }
    let think_tag = regex::Regex::new(r"(?i)</?\s*think(?:\s|>|$)").expect("think 标签正则");
    if think_tag.is_match(trimmed) {
        return Err(mechanical_completion_message(
            ui_locale,
            "think 标签残片",
            "a leftover think tag",
        ));
    }
    let paragraphs: Vec<String> = regex::Regex::new(r"\n\s*\n+")
        .expect("段落分割正则")
        .split(trimmed)
        .map(|paragraph| paragraph.trim())
        .filter(|paragraph| !paragraph.is_empty())
        .map(str::to_string)
        .collect();
    let opening = trimmed
        .split("\r\n")
        .map(|line| line.trim())
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_string();
    let opening_units = visible_prose_unit_count(&opening);
    if opening_units <= MAX_META_OPENING_VISIBLE_UNITS {
        let patterns: [(bool, &str); 4] = [
            (
                true,
                r"^(?:以下|下面)(?:是|为)(?:(?:根据|按照)(?:您|用户)(?:的)?(?:要求|指示))?(?:(?:修订|修改|重写|生成|完成|提供)(?:后)?的?)?(?:完整的?)?(?:正文|章节(?:正文)?|内容)\s*(?:[：:]|[。.!！]?$)",
            ),
            (
                false,
                r"(?i)^(?:(?:certainly|as\s+requested)[,!:]?\s+)?(?:here|below)\s+is\s+(?:(?:the|your)\s+)?(?:(?:revised|rewritten|generated|complete|full|updated|requested)\s+)*(?:text|chapter|content|revision|version|draft|prose)\s*(?:[:：]|[.!]?$)",
            ),
            (
                true,
                r"^(?:(?:根据|按照)(?:您|用户).{0,40}(?:要求|指示)|这是(?:我为您|根据您的要求)).{0,30}(?:修订|修改|重写|生成|提供).{0,10}(?:正文|章节|内容)[。！：:]?$",
            ),
            (
                false,
                r"(?i)^(?:(?:as\s+requested|certainly)[,!:]?\s+)?i(?:'ve|\s+have|\s+will)\s+(?:revised|rewritten|generated|provide|write|revise)\s+(?:(?:the|your|this|complete|full|revised)\s+)*(?:text|chapter|content|revision|version|draft|prose)[.!:：]?$",
            ),
        ];
        for (is_zh, pattern) in patterns {
            let compiled = if is_zh {
                regex::Regex::new(pattern).expect("首段元话术正则")
            } else {
                regex::RegexBuilder::new(pattern)
                    .case_insensitive(true)
                    .build()
                    .expect("首段元话术正则")
            };
            if compiled.is_match(&opening) {
                return Err(mechanical_completion_message(
                    ui_locale,
                    "首段元话术",
                    "opening meta commentary",
                ));
            }
        }
    }
    let zh_truncation = "\n…[内容已按上下文预算截断]…\n".trim();
    let en_truncation = "\n…[content truncated to fit the context budget]…\n".trim();
    let truncation_paragraph = regex::Regex::new(r"(?i)^(?:…\s*)?(?:\[(?:内容已按上下文预算截断|内容截断|输出被截断|content truncated to fit the context budget|truncated)\]|[（(]?(?:未完待续|未完)[）)]?)(?:\s*…)?$")
        .expect("截断段落正则");
    if trimmed.contains(zh_truncation)
        || trimmed.contains(en_truncation)
        || paragraphs
            .iter()
            .any(|paragraph| truncation_paragraph.is_match(paragraph))
    {
        return Err(mechanical_completion_message(
            ui_locale,
            "截断标记",
            "a truncation marker",
        ));
    }
    let groups: [fn(&str) -> Vec<String>; 2] = [
        |text: &str| {
            regex::Regex::new(r"\n\s*\n+")
                .expect("段落分割正则")
                .split(text)
                .map(str::to_string)
                .collect()
        },
        |text: &str| text.split("\r\n").map(str::to_string).collect(),
    ];
    for split in groups {
        let mut source_counts: HashMap<String, i64> = HashMap::new();
        for paragraph in split(source) {
            let normalized = regex::Regex::new(r"\s+")
                .expect("空白归一正则")
                .replace_all(&paragraph, " ")
                .trim()
                .to_string();
            *source_counts.entry(normalized).or_insert(0) += 1;
        }
        let mut seen_paragraphs: HashMap<String, i64> = HashMap::new();
        for paragraph in split(trimmed) {
            let normalized = regex::Regex::new(r"\s+")
                .expect("空白归一正则")
                .replace_all(&paragraph, " ")
                .trim()
                .to_string();
            if visible_prose_unit_count(&normalized) < MIN_OBVIOUS_DUPLICATE_PARAGRAPH_VISIBLE_UNITS
            {
                continue;
            }
            let count = seen_paragraphs.get(&normalized).copied().unwrap_or(0) + 1;
            if count > std::cmp::max(1, source_counts.get(&normalized).copied().unwrap_or(0)) {
                return Err(mechanical_completion_message(
                    ui_locale,
                    "明显重复段落",
                    "an obviously duplicated paragraph",
                ));
            }
            seen_paragraphs.insert(normalized, count);
        }
    }
    Ok(())
}

/// 基线 `MAX_META_OPENING_VISIBLE_UNITS` / `MIN_OBVIOUS_DUPLICATE_PARAGRAPH_VISIBLE_UNITS`
const MAX_META_OPENING_VISIBLE_UNITS: i64 = 200;
const MIN_OBVIOUS_DUPLICATE_PARAGRAPH_VISIBLE_UNITS: i64 = 120;

/// 基线 `stripDraftThinkingTags`
fn strip_draft_thinking_tags(text: &str) -> String {
    if text.is_empty() {
        return text.to_string();
    }
    let paired = regex::Regex::new(r"(?is)</think>[\s\S]*?(?:</think>|$)").expect("思考标签正则");
    let without_paired_thinking = paired.replace_all(text, "").to_string();
    let orphan_closing = regex::Regex::new(r"(?i)</think>").expect("孤儿闭合标签正则");
    let Some(orphan) = orphan_closing.find(&without_paired_thinking) else {
        return regex::Regex::new(r"(?i)</?think>")
            .expect("思考标签正则")
            .replace_all(&without_paired_thinking, "")
            .trim()
            .to_string();
    };
    let hidden_prefix = &without_paired_thinking[..orphan.start()];
    let visible_suffix = &without_paired_thinking[orphan.end()..];
    let looks_like_reasoning = regex::Regex::new(r"(?i)^\s*(?:思考|推理|分析|reasoning|analysis)")
        .expect("推理前缀正则")
        .is_match(hidden_prefix);
    let has_structured_visible_suffix = regex::Regex::new(r"(?i)^\s*(?:```(?:json)?\s*)?[{[]")
        .expect("结构化后缀正则")
        .is_match(visible_suffix);
    let cleaned = if looks_like_reasoning || has_structured_visible_suffix {
        visible_suffix.to_string()
    } else {
        format!("{hidden_prefix}{visible_suffix}")
    };
    regex::Regex::new(r"(?i)</?think>")
        .expect("思考标签正则")
        .replace_all(&cleaned, "")
        .trim()
        .to_string()
}

/// 基线 `isRecord`
fn is_record(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    if !value.is_null() && value.is_object() {
        value.as_object()
    } else {
        None
    }
}

/// 基线 `findCompleteJsonObjectEnd`
fn find_complete_json_object_end(source: &str, start: usize) -> Option<usize> {
    let chars: Vec<char> = source.chars().collect();
    let mut closers: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for index in start..chars.len() {
        let ch = chars[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
        } else if ch == '{' || ch == '[' {
            closers.push(if ch == '{' { '}' } else { ']' });
        } else if ch == '}' || ch == ']' {
            if closers.pop() != Some(ch) {
                return None;
            }
            if closers.is_empty() {
                return Some(index);
            }
        }
    }
    None
}

/// 基线 `extractSingleCompleteJsonObject(content, excludeArrayRoots = false)`
fn extract_single_complete_json_object(
    content: &str,
    exclude_array_roots: bool,
) -> Result<String, String> {
    let source = strip_draft_thinking_tags(content).trim().to_string();
    let mut candidates: Vec<String> = Vec::new();
    let mut search_from = 0;
    let chars: Vec<char> = source.chars().collect();
    while search_from < chars.len() {
        let object_start = chars[search_from..]
            .iter()
            .position(|ch| *ch == '{')
            .map(|index| search_from + index);
        let array_start = if exclude_array_roots {
            chars[search_from..]
                .iter()
                .position(|ch| *ch == '[')
                .map(|index| search_from + index)
        } else {
            None
        };
        let start = match (object_start, array_start) {
            (None, None) => break,
            (Some(object_start), None) => object_start,
            (None, Some(array_start)) => array_start,
            (Some(object_start), Some(array_start)) => object_start.min(array_start),
        };
        let Some(end) = find_complete_json_object_end(&source, start) else {
            return Err("AI 返回包含截断 JSON 对象片段".to_string());
        };
        let candidate: String = chars[start..=end].iter().collect();
        if let Ok(parsed) = serde_json::from_str::<Value>(&candidate) {
            if is_record(&parsed).is_some() {
                candidates.push(candidate);
            }
        }
        search_from = end + 1;
    }
    if candidates.len() == 1 {
        return Ok(candidates[0].clone());
    }
    if candidates.len() > 1 {
        return Err("AI 返回包含多个完整 JSON 对象，无法确定唯一结构化结果".to_string());
    }
    Err("AI 返回未包含一个完整 JSON 对象".to_string())
}

/// 基线 `parseReviewGenerationResult(content, reportVersion = 2)`
fn parse_review_generation_result(content: &str, report_version: i64) -> Result<Value, String> {
    let trimmed = strip_draft_thinking_tags(content);
    let fenced =
        regex::Regex::new(r"(?is)^```(?:json)?[ \t]*\r?\n([\s\S]*?)\r?\n```$").expect("围栏正则");
    let parsed: Value = if let Some(captures) = fenced.captures(&trimmed) {
        let inner = captures
            .get(1)
            .map(|match_| match_.as_str())
            .unwrap_or_default()
            .trim();
        serde_json::from_str(inner)
            .or_else(|_| {
                let extracted = extract_single_complete_json_object(&trimmed, true)
                    .map_err(|error| serde_json::Error::custom(error))?;
                serde_json::from_str(&extracted)
            })
            .map_err(|_| "invalid review contract".to_string())?
    } else {
        serde_json::from_str(&trimmed)
            .or_else(|_| {
                let extracted = extract_single_complete_json_object(&trimmed, true)
                    .map_err(|error| serde_json::Error::custom(error))?;
                serde_json::from_str(&extracted)
            })
            .map_err(|_| "invalid review contract".to_string())?
    };
    if !is_review_shape(&parsed) {
        return Err("invalid review contract".to_string());
    }
    let mut bounded = serde_json::Map::new();
    if let Some(goal_reviews) = parsed.get("goalReviews") {
        bounded.insert("goalReviews".to_string(), goal_reviews.clone());
    }
    bounded.insert(
        "summary".to_string(),
        Value::String(report_text(
            parsed
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            REVIEW_SUMMARY_MAX_CHARACTERS,
            report_version,
        )),
    );
    let items: Vec<Value> = parsed
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|item| {
            let record = item.as_object().cloned().unwrap_or_default();
            let category = record
                .get("category")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let severity = record
                .get("severity")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let description = report_text(
                record
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                REVIEW_DESCRIPTION_MAX_CHARACTERS,
                report_version,
            );
            let quote = record.get("quote");
            let mut bounded_item = serde_json::Map::new();
            bounded_item.insert("category".to_string(), Value::String(category));
            bounded_item.insert("severity".to_string(), Value::String(severity));
            bounded_item.insert("description".to_string(), Value::String(description));
            let quote_empty = quote
                .and_then(Value::as_str)
                .map(|text| text.trim().is_empty())
                .unwrap_or(true);
            let severity_pass = record.get("severity").and_then(Value::as_str) == Some("pass");
            if let Some(quote) = quote {
                if !(quote_empty && severity_pass) {
                    bounded_item.insert(
                        "quote".to_string(),
                        Value::String(report_text(
                            quote.as_str().unwrap_or_default(),
                            REVIEW_QUOTE_MAX_CHARACTERS,
                            report_version,
                        )),
                    );
                }
            }
            Value::Object(bounded_item)
        })
        .collect();
    bounded.insert("items".to_string(), Value::Array(items));
    let bounded_value = Value::Object(bounded);
    if !is_review_result(&bounded_value) {
        return Err("invalid review contract".to_string());
    }
    Ok(bounded_value)
}

/// 基线 `REVIEW_SUMMARY_MAX_CHARACTERS` / `REVIEW_DESCRIPTION_MAX_CHARACTERS` / `REVIEW_QUOTE_MAX_CHARACTERS`
const REVIEW_SUMMARY_MAX_CHARACTERS: usize = 120;
const REVIEW_DESCRIPTION_MAX_CHARACTERS: usize = 200;
const REVIEW_QUOTE_MAX_CHARACTERS: usize = 160;

/// 基线 `reportText(value, maxCharacters, version)`（V1 按字符截断；V2 仅 trim）
fn report_text(value: &str, max_characters: usize, version: i64) -> String {
    if version == 1 {
        value.trim().chars().take(max_characters).collect()
    } else {
        value.trim().to_string()
    }
}

/// 基线 `isReviewShape(value)`
fn is_review_shape(value: &Value) -> bool {
    let Some(record) = is_record(value) else {
        return false;
    };
    if record
        .keys()
        .any(|key| key != "summary" && key != "items" && key != "goalReviews")
        || record.get("summary").and_then(Value::as_str).is_none()
        || record
            .get("items")
            .and_then(Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0)
            < 1
        || record
            .get("items")
            .and_then(Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0)
            > 10
    {
        return false;
    }
    record
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items.iter().all(|item| {
                let Some(item_record) = is_record(item) else {
                    return false;
                };
                let severity = item_record.get("severity").and_then(Value::as_str);
                !item_record.keys().any(|key| {
                    key != "category" && key != "severity" && key != "description" && key != "quote"
                }) && item_record
                    .get("category")
                    .and_then(Value::as_str)
                    .is_some()
                    && matches!(severity, Some("error") | Some("warning") | Some("pass"))
                    && item_record
                        .get("description")
                        .and_then(Value::as_str)
                        .is_some()
                    && match item_record.get("quote") {
                        None => severity == Some("pass"),
                        Some(quote) => quote.as_str().is_some(),
                    }
            })
        })
        .unwrap_or(false)
}

/// 基线 `isReviewResult(value)`
fn is_review_result(value: &Value) -> bool {
    is_review_shape(value)
        && !value
            .get("summary")
            .and_then(Value::as_str)
            .map(|summary| summary.trim().is_empty())
            .unwrap_or(true)
        && value
            .get("items")
            .and_then(Value::as_array)
            .map(|items| {
                items.iter().all(|item| {
                    let record = item.as_object().cloned().unwrap_or_default();
                    !record
                        .get("category")
                        .and_then(Value::as_str)
                        .map(|category| category.trim().is_empty())
                        .unwrap_or(true)
                        && !record
                            .get("description")
                            .and_then(Value::as_str)
                            .map(|description| description.trim().is_empty())
                            .unwrap_or(true)
                        && match record.get("quote") {
                            None => true,
                            Some(quote) => !quote
                                .as_str()
                                .map(|text| text.trim().is_empty())
                                .unwrap_or(true),
                        }
                })
            })
            .unwrap_or(false)
}

/// 基线 `buildReviewGenerationReport(input)`
fn build_review_generation_report(input: &BuildReviewGenerationReportInput) -> Value {
    let mut parsed_result = parse_review_generation_result(&input.content, input.report_version)
        .expect("审稿产物解析失败");
    let goal_reviews = parsed_result.get("goalReviews").cloned();
    let goal_review = normalize_chapter_goal_review(
        goal_reviews.as_ref().unwrap_or(&Value::Null),
        &input.frozen_goals,
        &input.source_content,
        &input.writing_language,
    );
    if let Some(object) = parsed_result.as_object_mut() {
        object.remove("goalReviews");
        object.insert(
            "goalReview".to_string(),
            serde_json::to_value(&goal_review).expect("目标核对 JSON 序列化不得失败"),
        );
        let mut items = object
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        items.extend(chapter_goal_review_items(
            &goal_review,
            &input.writing_language,
        ));
        object.insert("items".to_string(), Value::Array(items));
        let has_unknown = object
            .get("items")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .any(|item| item.get("severity").and_then(Value::as_str) == Some("unknown"))
            })
            .unwrap_or(false);
        let goal_unmet = goal_review.items.iter().any(|item| item.status == "unmet");
        if has_unknown {
            object.insert(
                "summary".to_string(),
                Value::String(if input.ui_locale == "en-US" {
                    "The review contains unresolved items and is not an overall pass.".to_string()
                } else {
                    "审稿包含待核实项目，不能视为全部通过。".to_string()
                }),
            );
        } else if goal_unmet {
            object.insert(
                "summary".to_string(),
                Value::String(if input.ui_locale == "en-US" {
                    "Some chapter goals are unmet; check their evidence.".to_string()
                } else {
                    "本章存在尚未完成的目标，请核对逐项证据。".to_string()
                }),
            );
        }
    }
    merge_consistency_findings_into_review(
        &parsed_result,
        &input.preflight_findings,
        &input.ui_locale,
    )
}

/// 基线 `writingLanguageText(language, zhCN, enUS)`
fn writing_language_text(language: &str, zh_cn: &str, en_us: &str) -> String {
    if language == "en-US" {
        en_us.to_string()
    } else {
        zh_cn.to_string()
    }
}

/// 基线 `normalizeChapterGoalReview(raw, goals, draft, language)`
fn normalize_chapter_goal_review(
    raw: &Value,
    goals: &crate::services::review_revision_context::FrozenChapterGoals,
    draft: &str,
    language: &str,
) -> crate::services::review_revision_context::ChapterGoalReview {
    let candidates: Vec<Value> = raw.as_array().cloned().unwrap_or_default();
    let mut invalid_coverage = !raw.is_array() && !goals.items.is_empty();
    if candidates.iter().any(|value| {
        is_record(value).is_none()
            || !goals
                .items
                .iter()
                .any(|goal| goal.id == value.get("id").and_then(Value::as_str).unwrap_or_default())
    }) {
        invalid_coverage = true;
    }
    let items: Vec<crate::services::review_revision_context::ChapterGoalReviewItem> = goals
        .items
        .iter()
        .map(|goal| {
            let matches: Vec<&Value> = candidates
                .iter()
                .filter(|value| value.get("id").and_then(Value::as_str) == Some(goal.id.as_str()))
                .collect();
            let candidate = if matches.len() == 1 {
                matches[0].as_object().cloned()
            } else {
                None
            };
            let mut evidence: Vec<crate::services::review_revision_context::ChapterGoalEvidence> =
                Vec::new();
            let mut valid = candidate.as_ref().is_some_and(|record| {
                record.keys().all(|key| {
                    key == "id" || key == "status" || key == "description" || key == "evidence"
                }) && matches!(
                    record.get("status").and_then(Value::as_str),
                    Some("completed") | Some("unmet") | Some("unknown")
                ) && !record
                    .get("description")
                    .and_then(Value::as_str)
                    .map(|text| text.trim().is_empty())
                    .unwrap_or(true)
                    && record.get("evidence").and_then(Value::as_array).is_some()
            });
            if let Some(record) = &candidate {
                if valid {
                    for value in record
                        .get("evidence")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                    {
                        let entry = value.as_object().cloned();
                        let located = entry.as_ref().and_then(|entry| {
                            locate_goal_evidence(entry.get("quote").and_then(Value::as_str), draft)
                        });
                        let entry_keys_valid = entry
                            .as_ref()
                            .is_none_or(|entry| entry.keys().all(|key| key == "quote"));
                        match (entry, located, entry_keys_valid) {
                            (Some(entry), Some(located), true)
                                if {
                                    let _ = &entry;
                                    true
                                } =>
                            {
                                evidence.push(located)
                            }
                            _ => valid = false,
                        }
                    }
                    if record.get("status").and_then(Value::as_str) != Some("unknown")
                        && evidence.is_empty()
                    {
                        valid = false;
                    }
                }
            }
            if !valid {
                invalid_coverage = true;
            }
            crate::services::review_revision_context::ChapterGoalReviewItem {
                id: goal.id.clone(),
                text: goal.text.clone(),
                status: if valid {
                    candidate
                        .as_ref()
                        .and_then(|record| {
                            record
                                .get("status")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        .unwrap_or_default()
                } else {
                    "unknown".to_string()
                },
                description: if valid {
                    candidate
                        .as_ref()
                        .and_then(|record| {
                            record
                                .get("description")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        .unwrap_or_default()
                } else {
                    writing_language_text(
                        language,
                        "该目标缺少有效的逐项判断或正文证据，请人工核实。",
                        "This goal lacks a valid judgment or draft evidence; verify it manually.",
                    )
                },
                evidence: if valid { evidence } else { Vec::new() },
            }
        })
        .collect();
    crate::services::review_revision_context::ChapterGoalReview {
        version: 1,
        chapter_number: goals.chapter_number,
        coverage: if invalid_coverage {
            "unknown".to_string()
        } else {
            goals.coverage.clone()
        },
        items,
    }
}

/// 基线 `locateGoalEvidence(quote, draft)`
fn locate_goal_evidence(
    quote: Option<&str>,
    draft: &str,
) -> Option<crate::services::review_revision_context::ChapterGoalEvidence> {
    let quote = quote?;
    if quote.trim().is_empty() {
        return None;
    }
    // 正常命中保留基线的首处定位；仅剥除模型添加引号的回退要求唯一。
    let (candidate, start) = if let Some(start) = draft.find(quote) {
        (quote, start)
    } else {
        let candidate = quote
            .strip_prefix('“')
            .and_then(|value| value.strip_suffix('”'))
            .or_else(|| {
                quote
                    .strip_prefix('"')
                    .and_then(|value| value.strip_suffix('"'))
            })?;
        if candidate.trim().is_empty() {
            return None;
        }
        let start = draft.find(candidate)?;
        // 跳过首个完整标量以保持UTF-8切片合法，同时检测重叠的第二次命中。
        let next = start + candidate.chars().next()?.len_utf8();
        if draft[next..].contains(candidate) {
            return None;
        }
        (candidate, start)
    };
    let utf16_start = draft[..start].encode_utf16().count() as i64;
    Some(
        crate::services::review_revision_context::ChapterGoalEvidence {
            quote: candidate.to_string(),
            start: utf16_start,
            end: utf16_start + candidate.encode_utf16().count() as i64,
        },
    )
}

/// 基线 `chapterGoalReviewItems(review, language)`
fn chapter_goal_review_items(
    review: &crate::services::review_revision_context::ChapterGoalReview,
    language: &str,
) -> Vec<Value> {
    let category = writing_language_text(language, "本章目标", "Chapter goal");
    let mut items: Vec<Value> = review
        .items
        .iter()
        .map(|item| {
            let severity = match item.status.as_str() {
                "completed" => "pass",
                "unmet" => "error",
                _ => "unknown",
            };
            let mut record = serde_json::Map::new();
            record.insert("category".to_string(), Value::String(category.clone()));
            record.insert("goalId".to_string(), Value::String(item.id.clone()));
            record.insert("severity".to_string(), Value::String(severity.to_string()));
            record.insert(
                "description".to_string(),
                Value::String(format!("{}\n{}", item.text, item.description)),
            );
            if !item.evidence.is_empty() {
                record.insert(
                    "quote".to_string(),
                    Value::String(
                        item.evidence
                            .iter()
                            .map(|evidence| evidence.quote.clone())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ),
                );
            }
            Value::Object(record)
        })
        .collect();
    if review.coverage != "complete" {
        let description = if review.coverage == "not_configured" {
            writing_language_text(
                language,
                "本章未配置可核对的关键事件，未完成目标验收。",
                "No chapter key events are configured; goal acceptance was not performed.",
            )
        } else {
            writing_language_text(
                language,
                "本章目标来源或逐项核对不完整，请人工核实。",
                "Chapter goal sources or checklist coverage are incomplete; verify manually.",
            )
        };
        let mut record = serde_json::Map::new();
        record.insert("category".to_string(), Value::String(category));
        record.insert("severity".to_string(), Value::String("unknown".to_string()));
        record.insert("description".to_string(), Value::String(description));
        items.push(Value::Object(record));
    }
    items
}

/// 基线 `mergeConsistencyFindingsIntoReview(review, findings, locale)`
fn merge_consistency_findings_into_review(
    review: &Value,
    findings: &[crate::services::review_revision_context::ConsistencyFinding],
    locale: &str,
) -> Value {
    let mapped: Vec<Value> = findings
        .iter()
        .map(|finding| {
            let mut record = serde_json::Map::new();
            record.insert(
                "category".to_string(),
                Value::String(if locale == "en-US" {
                    "Deterministic continuity preflight".to_string()
                } else {
                    "确定性一致性预检".to_string()
                }),
            );
            record.insert(
                "severity".to_string(),
                Value::String(finding.severity.clone()),
            );
            record.insert(
                "description".to_string(),
                Value::String(if locale == "en-US" {
                    finding.issue.en_us.clone()
                } else {
                    finding.issue.zh_cn.clone()
                }),
            );
            record.insert("quote".to_string(), Value::String(finding.evidence.clone()));
            record.insert(
                "stableFactKey".to_string(),
                Value::String(finding.stable_fact_key.clone()),
            );
            record.insert(
                "sourceChapter".to_string(),
                Value::from(finding.source_chapter),
            );
            Value::Object(record)
        })
        .collect();
    let mut merged = review.clone();
    if let Some(object) = merged.as_object_mut() {
        let mut items = object
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        items.extend(mapped);
        object.insert("items".to_string(), Value::Array(items));
    }
    merged
}

/// 基线 `countOccurrences(text, excerpt)`
fn count_occurrences(text: &str, excerpt: &str) -> i64 {
    if excerpt.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut offset = 0;
    while offset <= text.len() {
        let Some(found) = text[offset..].find(excerpt) else {
            break;
        };
        let found = offset + found;
        count += 1;
        offset = found + excerpt.len().max(1);
    }
    count
}

/// 基线 `objectiveText(value)`（去除非字母数字字符）
fn objective_text(value: &str) -> String {
    value.chars().filter(|ch| ch.is_alphanumeric()).collect()
}

/// 基线 `parseRecheckItems(content, reportVersion)`
fn parse_recheck_items(content: &str, report_version: i64) -> Option<RecheckParsedItems> {
    let trimmed = content.trim();
    let fenced =
        regex::Regex::new(r"(?is)^```json[ \t]*\r?\n([\s\S]*?)\r?\n```$").expect("围栏正则");
    let parsed: Value = if report_version == 1 {
        let inner = fenced
            .captures(&trimmed)
            .and_then(|captures| captures.get(1))
            .map(|match_| match_.as_str())
            .unwrap_or(trimmed)
            .trim();
        serde_json::from_str(inner).ok()?
    } else {
        let extracted = extract_single_complete_json_object(content, true).ok()?;
        serde_json::from_str(&extracted).ok()?
    };
    let record = parsed.as_object()?;
    if parsed.is_array()
        || record.keys().any(|key| key != "summary" && key != "items")
        || record
            .get("summary")
            .and_then(Value::as_str)
            .map(|summary| summary.trim().is_empty())
            .unwrap_or(true)
        || record.get("items").and_then(Value::as_array).is_none()
    {
        return None;
    }
    let mut items = Vec::new();
    for item in record.get("items")?.as_array()? {
        let row = item.as_object()?;
        if row.keys().any(|key| {
            key != "findingId"
                && key != "targetId"
                && key != "resolved"
                && key != "evidenceQuote"
                && key != "reason"
        }) || row
            .get("findingId")
            .and_then(Value::as_str)
            .map(|text| text.trim().is_empty())
            .unwrap_or(true)
            || row
                .get("targetId")
                .and_then(Value::as_str)
                .map(|text| text.trim().is_empty())
                .unwrap_or(true)
            || row.get("resolved").and_then(Value::as_bool).is_none()
            || row
                .get("evidenceQuote")
                .and_then(Value::as_str)
                .map(|text| text.trim().is_empty())
                .unwrap_or(true)
            || row
                .get("reason")
                .and_then(Value::as_str)
                .map(|text| text.trim().is_empty())
                .unwrap_or(true)
        {
            return None;
        }
        items.push(RecheckModelItem {
            finding_id: row.get("findingId")?.as_str()?.to_string(),
            target_id: row.get("targetId")?.as_str()?.to_string(),
            resolved: row.get("resolved")?.as_bool()?,
            evidence_quote: row.get("evidenceQuote")?.as_str()?.to_string(),
            reason: row.get("reason")?.as_str()?.to_string(),
        });
    }
    Some(RecheckParsedItems {
        summary: record.get("summary")?.as_str()?.trim().to_string(),
        items,
    })
}

/// 基线 `parseRecheckItems` 的返回
#[derive(Debug, Clone)]
struct RecheckParsedItems {
    summary: String,
    items: Vec<RecheckModelItem>,
}

/// 基线 `ReviewCycleRecheckModelItem`
#[derive(Debug, Clone)]
struct RecheckModelItem {
    finding_id: String,
    target_id: String,
    resolved: bool,
    evidence_quote: String,
    reason: String,
}

/// 基线 `buildReviewCycleRecheckReport(content, merged, context, uiLocale, reportVersion = 2)`
fn build_review_cycle_recheck_report(
    content: &str,
    merged: &str,
    context: &RecheckPolicy,
    ui_locale: &str,
    report_version: i64,
) -> RecheckReport {
    let parsed = parse_recheck_items(content, report_version);
    let mut counts: HashMap<String, i64> = HashMap::new();
    if let Some(parsed) = &parsed {
        for item in &parsed.items {
            *counts.entry(item.finding_id.clone()).or_insert(0) += 1;
        }
    }
    let mut items: Vec<Value> = Vec::new();
    let mut decisions: Vec<RecheckDecision> = Vec::new();
    for finding in &context.context.findings {
        let candidate = if counts.get(&finding.finding_id).copied() == Some(1) {
            parsed.as_ref().and_then(|parsed| {
                parsed
                    .items
                    .iter()
                    .find(|item| item.finding_id == finding.finding_id)
            })
        } else {
            None
        };
        let evidence_count = candidate
            .map(|candidate| count_occurrences(merged, &candidate.evidence_quote))
            .unwrap_or(0);
        let valid = candidate.is_some_and(|candidate| {
            candidate.target_id == finding.target_id
                && evidence_count == 1
                && (!finding.must_show.unwrap_or(false)
                    || !context
                        .context
                        .source_content
                        .as_deref()
                        .map(|source_content| {
                            objective_text(source_content)
                                .contains(&objective_text(&candidate.evidence_quote))
                        })
                        .unwrap_or(false))
        });
        let review_item_index = items.len() as i64;
        let Some(candidate) = candidate else {
            items.push(recheck_item_value(
                finding,
                "unknown",
                &if ui_locale == "en-US" {
                    "The recheck did not provide unique, verifiable evidence.".to_string()
                } else {
                    "复核未提供唯一且可验证的新证据。".to_string()
                },
                None,
                None,
            ));
            decisions.push(RecheckDecision {
                finding_id: finding.finding_id.clone(),
                target_id: finding.target_id.clone(),
                status: "unknown".to_string(),
                review_item_index,
            });
            continue;
        };
        if !valid {
            items.push(recheck_item_value(
                finding,
                "unknown",
                &if ui_locale == "en-US" {
                    "The recheck did not provide unique, verifiable evidence.".to_string()
                } else {
                    "复核未提供唯一且可验证的新证据。".to_string()
                },
                None,
                None,
            ));
            decisions.push(RecheckDecision {
                finding_id: finding.finding_id.clone(),
                target_id: finding.target_id.clone(),
                status: "unknown".to_string(),
                review_item_index,
            });
            continue;
        }
        if context.version == 2 && candidate.resolved {
            items.push(recheck_item_value(
                finding,
                "unknown",
                &if ui_locale == "en-US" {
                    format!("The model supplied anchored evidence, but semantic resolution still requires author verification. {}", candidate.reason)
                } else {
                    format!("模型提供了可定位的新证据，但是否实质解决仍需作者核实。{}", candidate.reason)
                },
                Some(candidate.evidence_quote.clone()),
                Some(false),
            ));
            decisions.push(RecheckDecision {
                finding_id: finding.finding_id.clone(),
                target_id: finding.target_id.clone(),
                status: "unknown".to_string(),
                review_item_index,
            });
            continue;
        }
        items.push(recheck_item_value(
            finding,
            if candidate.resolved {
                "pass"
            } else {
                "warning"
            },
            &candidate.reason,
            Some(candidate.evidence_quote.clone()),
            Some(candidate.resolved),
        ));
        decisions.push(RecheckDecision {
            finding_id: finding.finding_id.clone(),
            target_id: finding.target_id.clone(),
            status: if candidate.resolved {
                "resolved"
            } else {
                "unresolved"
            }
            .to_string(),
            review_item_index,
        });
    }
    RecheckReport {
        summary: parsed
            .as_ref()
            .map(|parsed| parsed.summary.trim().to_string())
            .filter(|summary| !summary.is_empty())
            .unwrap_or_else(|| {
                if ui_locale == "en-US" {
                    "The recheck output was invalid; affected findings remain unknown.".to_string()
                } else {
                    "复核输出无效；受影响项目保持待核实。".to_string()
                }
            }),
        items,
        decisions,
    }
}

/// 基线 `buildReviewCycleRecheckReport` 的单条 items 元素构造辅助
fn recheck_item_value(
    finding: &review_cycle::ReviewCycleRecheckFinding,
    severity: &str,
    description: &str,
    quote: Option<String>,
    resolved: Option<bool>,
) -> Value {
    let mut record = serde_json::Map::new();
    record.insert(
        "category".to_string(),
        Value::String(finding.category.clone()),
    );
    record.insert("severity".to_string(), Value::String(severity.to_string()));
    record.insert(
        "findingId".to_string(),
        Value::String(finding.finding_id.clone()),
    );
    record.insert(
        "targetId".to_string(),
        Value::String(finding.target_id.clone()),
    );
    record.insert(
        "description".to_string(),
        Value::String(description.to_string()),
    );
    if let Some(quote) = quote {
        record.insert("quote".to_string(), Value::String(quote));
    }
    if let Some(resolved) = resolved {
        record.insert("resolved".to_string(), Value::Bool(resolved));
    }
    Value::Object(record)
}

#[cfg(test)]
#[path = "review_revision_generation_tests.rs"]
mod commit_tests;

#[cfg(test)]
mod serialization_contract_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn revision_commit_request_accepts_renderer_camel_case() {
        let handle = json!({ "projectId": "synthetic-project", "epoch": "synthetic-epoch",
            "rootActionId": "synthetic-root", "runId": "synthetic-run" });
        let value = json!({ "contextId": "synthetic-context", "handle": handle.clone(),
            "expectedCompositionHash": "a".repeat(64) });
        let request: RevisionGenerationCommitRequest =
            serde_json::from_value(value.clone()).unwrap();
        assert_eq!(request.context_id, "synthetic-context");
        assert_eq!(request.expected_composition_hash, "a".repeat(64));
        assert_eq!(serde_json::to_value(request).unwrap(), value);
        assert!(serde_json::from_value::<RevisionGenerationCommitRequest>(json!({
            "context_id": "synthetic-context", "handle": handle, "expected_composition_hash": "a".repeat(64)
        })).is_err());
    }
}
