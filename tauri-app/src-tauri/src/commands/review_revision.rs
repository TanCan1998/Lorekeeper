//! 批次 G4-3b：审稿修稿服务层命令 —— `review-revision:*` 4 频道。
//!
//! 平移自 `electron/controllers/generation-controller.ts:213-216` 的
//! 4 个注册项，owner 接线见
//! `electron/services/main-generation-owner.ts:1272-1275`：
//! - `review-revision:prepare` → `prepareReviewRevision(request)`
//! - `review-revision:commit-review` → `commitReview(request)`
//! - `review-revision:commit-revision` → `commitRevision(request)`
//! - `review-revision:read-recovery` → `readReviewRevisionRecovery(request.handle)`
//!
//! 命令名对齐前端 `ipc-client.ts` 的 `toCommandName`
//! （`channel.replace(/[:-]/g, '_')`），入参按频道契约
//! `src/shared/review-revision-generation.ts` 末节
//! `ReviewRevisionGenerationInvokeChannels` 登记。
//!
//! 会话门禁（基线 `assertCurrent`）由命令层在 `with_project_db` 之外
//! 显式断言活跃项目会话（对齐 finalization 批次范式）；服务层内部
//! 以同一断言闭包作二级门禁，失败收敛为 `GENERATION_OWNER_CLOSED`。
//!
//! `assertSourcesCurrent` / `compareGenerationSourceBindings` / 候选快照
//! 投影由 `MainGenerationOwner` 的真实实现注入：`assert_sources_current`、
//! `rebuild_binding` + `current_model_receipt` 与 `view_of(run).candidates`。

use serde::Deserialize;
use tauri::State;

use crate::repositories::generation_run_repository::{
    DurableGenerationRun, GenerationRunRepository, MainGenerationRunHandle,
};
use crate::security::{
    assert_current_project_context, guard_message, GuardKind, ProjectSessionContext,
};
use crate::services::review_revision_context::{
    PrepareReviewRevisionRequest, PreparedReviewRevisionContext,
};
use crate::services::review_revision_generation::{
    ReviewGenerationCommitRequest, ReviewRevisionCommitReceipt, ReviewRevisionGeneration,
    ReviewRevisionRecovery, ReviewRevisionScope, RevisionGenerationCommitRequest,
};
use crate::state::AppState;

/// 基线 `readRecovery(handle, ...)` 的定位参数包装
/// （频道契约 `args: [request: { handle: MainGenerationRunHandle }]`）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewRevisionRecoveryRequest {
    pub handle: MainGenerationRunHandle,
}

/// 基线 `assertCurrent` 的命令层等价物：项目会话三字段与活跃租约一致。
fn assert_current_session(
    state: &AppState,
    session: Option<&ProjectSessionContext>,
) -> Result<(), String> {
    let Some(context) = session else {
        return Err("缺少项目会话上下文，已拒绝操作".to_string());
    };
    let active = state.active_project_snapshot();
    assert_current_project_context(context, active.as_ref()).map_err(guard_message)
}

/// 服务作用域：基线 `{ projectId, epoch }`。Tauri 侧 `epoch` 即会话租约
/// （对齐 `services/main_generation_owner.rs` 的
/// `run.binding.epoch != self.session.lease_id` 判别）。
fn scope_of(state: &AppState) -> Result<ReviewRevisionScope, String> {
    let Some(active) = state.active_project_snapshot() else {
        return Err(guard_message(GuardKind::LeaseInvalid));
    };
    Ok(ReviewRevisionScope {
        project_id: active.project_id,
        epoch: active.lease_id,
    })
}

/// `review-revision:prepare` —— 捕获审稿 / 修稿上下文并缓存为
/// `contextId`（基线 `prepare`）。
#[tauri::command]
pub async fn review_revision_prepare(
    state: State<'_, AppState>,
    request: PrepareReviewRevisionRequest,
    project_session: Option<ProjectSessionContext>,
) -> Result<PreparedReviewRevisionContext, String> {
    let state = state.inner();
    assert_current_session(state, project_session.as_ref())?;
    let scope = scope_of(state)?;
    let service = ReviewRevisionGeneration::new(GenerationRunRepository::default(), scope);
    state.with_project_db(|conn| {
        service.prepare(conn, &request, || {
            assert_current_session(state, project_session.as_ref())
        })
    })
}

/// `review-revision:commit-review` —— 提交已生成的审稿正文
/// （基线 `commitReview`）。
#[tauri::command]
pub async fn review_revision_commit_review(
    state: State<'_, AppState>,
    request: ReviewGenerationCommitRequest,
    project_session: Option<ProjectSessionContext>,
) -> Result<ReviewRevisionCommitReceipt, String> {
    let state = state.inner();
    assert_current_session(state, project_session.as_ref())?;
    let cache = &state.generation_owners;
    let mut guard =
        crate::commands::generation::authorized_gate(&cache, state, project_session.as_ref())?;
    let owner: &crate::services::main_generation_owner::MainGenerationOwner = guard.owner();
    let scope = scope_of(state)?;
    let service = ReviewRevisionGeneration::new(GenerationRunRepository::default(), scope);
    // 基线 `assertSourcesCurrent`：提交前复核源 / 模型 / 模板仍为当前。
    state.with_project_db(|conn| {
        let assert_sources =
            |handle: &MainGenerationRunHandle| owner.assert_review_sources_current(conn, handle);
        service.commit_review(conn, &request, assert_sources)
    })
}

/// `review-revision:commit-revision` —— 提交已生成的修稿正文
/// （基线 `commitRevision`）。
#[tauri::command]
pub async fn review_revision_commit_revision(
    state: State<'_, AppState>,
    request: RevisionGenerationCommitRequest,
    project_session: Option<ProjectSessionContext>,
) -> Result<ReviewRevisionCommitReceipt, String> {
    let state = state.inner();
    assert_current_session(state, project_session.as_ref())?;
    let cache = &state.generation_owners;
    let mut guard =
        crate::commands::generation::authorized_gate(&cache, state, project_session.as_ref())?;
    let owner: &crate::services::main_generation_owner::MainGenerationOwner = guard.owner();
    let scope = scope_of(state)?;
    let service = ReviewRevisionGeneration::new(GenerationRunRepository::default(), scope);
    // 基线 `assertSourcesCurrent`：占位理由同 `review_revision_commit_review`。
    state.with_project_db(|conn| {
        let assert_sources =
            |handle: &MainGenerationRunHandle| owner.assert_review_sources_current(conn, handle);
        service.commit_revision(conn, &request, assert_sources)
    })
}

/// `review-revision:read-recovery` —— 读取候选恢复读面
/// （基线 `readRecovery`）。
#[tauri::command]
pub async fn review_revision_read_recovery(
    state: State<'_, AppState>,
    request: ReviewRevisionRecoveryRequest,
    project_session: Option<ProjectSessionContext>,
) -> Result<ReviewRevisionRecovery, String> {
    let state = state.inner();
    assert_current_session(state, project_session.as_ref())?;
    let cache = &state.generation_owners;
    let mut guard =
        crate::commands::generation::authorized_gate(&cache, state, project_session.as_ref())?;
    let owner: &crate::services::main_generation_owner::MainGenerationOwner = guard.owner();
    let scope = scope_of(state)?;
    let service = ReviewRevisionGeneration::new(GenerationRunRepository::default(), scope);
    // 基线 `sourcesMatch` / `snapshot`：源绑定比对与最新候选快照。
    state.with_project_db(|conn| {
        let sources_match =
            |run: &DurableGenerationRun| owner.review_revision_sources_match(conn, run);
        let snapshot =
            |run: &DurableGenerationRun| owner.review_revision_candidate_snapshot(conn, run);
        service.read_recovery(conn, &request.handle, sources_match, snapshot)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;
    use crate::repositories::generation_run_repository::GenerationAuthorInput;
    use crate::services::review_revision_context::{ExpectedDraftRef, ReviewRevisionOperation};
    use rusqlite::Connection;
    use sha2::{Digest, Sha256};

    /// 与 `repositories::generation_run_repository::text_hash` 同算法
    /// （该函数为仓储私有项，测试侧平移）。
    fn text_hash(text: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(text.as_bytes());
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        create_tables(&conn).expect("建表失败");
        conn
    }

    /// 种 `project_core` 单行（`capture` 的 `get_project_core` 必需）
    /// 与一章草稿，返回 `(draft_id, 正文)`。
    fn seed_project_and_draft(conn: &Connection) -> (i64, String) {
        conn.execute(
            "INSERT INTO project_core (id, project_name) VALUES ('main', '测试项目')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO contents (body) VALUES ('正文')", [])
            .unwrap();
        let content_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, source, content_id, word_count, source_dependencies)
             VALUES (1, 2, 'write', ?1, 2, '[]')",
            [content_id],
        )
        .unwrap();
        (conn.last_insert_rowid(), "正文".to_string())
    }

    fn refine_draft_request(draft_id: i64, content: &str) -> PrepareReviewRevisionRequest {
        PrepareReviewRevisionRequest {
            operation: ReviewRevisionOperation::RefineDraft,
            draft_id,
            expected_draft: ExpectedDraftRef {
                chapter_number: 1,
                version: 2,
                status: "draft".to_string(),
                content_hash: text_hash(content),
            },
            review_source_id: None,
            confirmed_review_content: None,
            review_cycle_id: None,
            expected_merged_hash: None,
            author_inputs: vec![GenerationAuthorInput {
                id: "user-prompt".to_string(),
                text: "润色首段".to_string(),
            }],
            ui_locale: "zh-CN".to_string(),
        }
    }

    fn service() -> ReviewRevisionGeneration {
        ReviewRevisionGeneration::new(
            GenerationRunRepository::default(),
            ReviewRevisionScope {
                project_id: "p1".to_string(),
                epoch: "lease-1".to_string(),
            },
        )
    }

    #[test]
    fn prepare_captures_refine_draft_context_test() {
        let conn = memory_db();
        let (draft_id, content) = seed_project_and_draft(&conn);
        let request = refine_draft_request(draft_id, &content);

        let prepared = service()
            .prepare(&conn, &request, || Ok(()))
            .expect("prepare 失败");

        // 缓存身份：UUID v4 形态（8-4-4-4-12），非空
        assert_eq!(prepared.context_id.len(), 36);
        assert_eq!(prepared.context_id.as_bytes()[8], b'-');
        // 捕获的上下文逐字段对齐请求与草稿现状
        assert_eq!(prepared.context.source.id, draft_id);
        assert_eq!(prepared.context.source.version, 2);
        assert_eq!(prepared.context.source.status, "draft");
        assert_eq!(prepared.context.source.content, "正文");
        assert_eq!(prepared.context.source_hash, text_hash("正文"));
        assert_eq!(prepared.context.operation.as_str(), "refine-draft");
        assert_eq!(prepared.context.ui_locale, "zh-CN");
        assert_eq!(prepared.context.author_inputs.len(), 1);
        assert_eq!(prepared.context.author_inputs[0].id, "user-prompt");
        // 审稿配置来自 project_core 单行
        assert_eq!(prepared.context.config.words_per_chapter, 3000);
        // refine-draft 无复核 / 确认面
        assert!(prepared.context.recheck.is_none());
        assert!(prepared.context.confirmation.is_none());
        // 父系血缘：首次准备无父根动作
        assert!(prepared.parent_root_action_id.is_none());
    }

    #[test]
    fn prepare_rejects_stale_draft_source_test() {
        let conn = memory_db();
        let (draft_id, content) = seed_project_and_draft(&conn);
        // 期望引用指向过期版本（草稿实际为 v2）
        let mut request = refine_draft_request(draft_id, &content);
        request.expected_draft.version = 1;

        let error = service()
            .prepare(&conn, &request, || Ok(()))
            .expect_err("过期源引用必须拒绝");
        assert_eq!(error, "Error: GENERATION_REVIEW_SOURCE_CHANGED");
    }

    #[test]
    fn prepare_rejects_invalid_locale_test() {
        let conn = memory_db();
        let (draft_id, content) = seed_project_and_draft(&conn);
        let mut request = refine_draft_request(draft_id, &content);
        request.ui_locale = "fr-FR".to_string();

        let error = service()
            .prepare(&conn, &request, || Ok(()))
            .expect_err("非法 locale 必须拒绝");
        // 入口运行时校验失败 → invalid() 的 GENERATION_REVIEW_CONTEXT_INVALID
        assert_eq!(error, "Error: GENERATION_REVIEW_CONTEXT_INVALID");
    }
}
