//! 使用真实内存SQLite、账本仓储及审稿服务，不模拟领域写入或使用用户项目。
use super::*;
use crate::db::schema::create_tables;
use crate::repositories::generation_run_repository::{OpenGenerationRunRequest, VisibleArtifact};
use crate::services::review_revision_context::{
    ExpectedDraftRef, PrepareReviewRevisionRequest, ReviewRevisionOperation,
};
use serde_json::json;

struct Fixture {
    conn: Connection,
    service: ReviewRevisionGeneration,
    prepared: PreparedReviewRevisionContext,
    run: DurableGenerationRun,
    artifact: VisibleArtifact,
}
impl Fixture {
    fn new(operation: ReviewRevisionOperation) -> Self {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn.execute(
            "INSERT INTO project_core (id,project_name) VALUES ('main','合成测试')",
            [],
        )
        .unwrap();
        let source = "作者原始正文。".repeat(20);
        conn.execute("INSERT INTO contents(body) VALUES (?1)", [&source])
            .unwrap();
        let content_id = conn.last_insert_rowid();
        conn.execute("INSERT INTO drafts(chapter_number,version,source,content_id,word_count,source_dependencies) VALUES (1,1,'write',?1,0,'[]')",[content_id]).unwrap();
        let draft_id = conn.last_insert_rowid();
        let service = ReviewRevisionGeneration::new(
            GenerationRunRepository::default(),
            ReviewRevisionScope {
                project_id: "test-project".into(),
                epoch: "test-lease".into(),
            },
        );
        let prepared = service
            .prepare(
                &conn,
                &PrepareReviewRevisionRequest {
                    operation,
                    draft_id,
                    expected_draft: ExpectedDraftRef {
                        chapter_number: 1,
                        version: 1,
                        status: "draft".into(),
                        content_hash: text_hash(&source),
                    },
                    review_source_id: None,
                    confirmed_review_content: None,
                    review_cycle_id: None,
                    expected_merged_hash: None,
                    author_inputs: vec![],
                    ui_locale: "zh-CN".into(),
                },
                || Ok(()),
            )
            .unwrap();
        let context = serde_json::to_value(&prepared.context).unwrap();
        let request:OpenGenerationRunRequest=serde_json::from_value(json!({
            "projectId":"test-project","epoch":"test-lease",
            "fingerprint":{"chapterBriefHash":"a".repeat(64),"authorGuidanceHash":"b".repeat(64),"dependencyHash":"c".repeat(64),
              "contextSnapshotHash":"d".repeat(64),"templateHash":"e".repeat(64),"skillSnapshotHash":"f".repeat(64),
              "modelLeaseRevision":"0".repeat(64),"policyHash":"1".repeat(64),"outputContractHash":"2".repeat(64)},
            "contextSnapshotId":"test-snapshot",
            "sourceManifest":{"operation":prepared.context.operation.as_str(),"outputContract":"visible-text","reviewRevisionContext":context,
              "reviewRevisionContextHash":ReviewRevisionGeneration::context_hash(&prepared.context),
              "authorInputs":[{"id":"review-revision-context","text":serde_json::to_string(&prepared.context).unwrap()}],
              "modelReceipt":{"modelId":"synthetic-model"}},
            "sourceRefs":[],"operation":prepared.context.operation.as_str(),"uiActionNonce":"test-action",
            "frozenInputHash":"a".repeat(64),
            "budget":{"maxPhysicalRequests":4,"maxTokenLiability":10000,"maxOutputPerRequest":4096,"maxActiveElapsedMs":60000}
        })).unwrap();
        let run = service.runs.open(&conn, &request).unwrap();
        let reserved = service
            .runs
            .reserve(
                &conn,
                &run.run_id,
                "test-attempt",
                &"b".repeat(64),
                100,
                100,
                None,
                Some(prepared.context.operation.as_str()),
                None,
                None,
                false,
            )
            .unwrap();
        let attempt = &reserved.attempt.attempt_id;
        service.runs.mark_dispatched(&conn, attempt).unwrap();
        let text = if prepared.context.operation == ReviewRevisionOperation::ReviewChapter {
            json!({"summary":"合成审稿","items":[{"category":"剧情连贯性","severity":"pass","description":"未发现矛盾"}]}).to_string()
        } else {
            "修订后的测试正文。".repeat(20)
        };
        let artifact = service.runs.snapshot(&conn, attempt, 0, &text).unwrap();
        service
            .runs
            .settle(&conn, attempt, None, Some("stop".into()), None, None)
            .unwrap();
        Self {
            conn,
            service,
            prepared,
            run,
            artifact,
        }
    }
    fn review(&self) -> Result<ReviewRevisionCommitReceipt, String> {
        self.service.commit_review(
            &self.conn,
            &ReviewGenerationCommitRequest {
                context_id: self.prepared.context_id.clone(),
                handle: ReviewRevisionGeneration::handle_of(&self.run),
                artifact: ReviewArtifactRef {
                    artifact_id: self.artifact.artifact_id.clone(),
                    revision: self.artifact.revision,
                    text_hash: self.artifact.text_hash.clone(),
                },
            },
            |_| Ok(()),
        )
    }
    fn revision(&self) -> Result<ReviewRevisionCommitReceipt, String> {
        let composition = self.service.runs.compose_visible(
            &self.conn,
            &self.run.run_id,
            &[self.artifact.artifact_id.clone()],
            Some(&self.artifact.text_hash),
            "visible-append-v1",
        )?;
        self.service.commit_revision(
            &self.conn,
            &RevisionGenerationCommitRequest {
                context_id: self.prepared.context_id.clone(),
                handle: ReviewRevisionGeneration::handle_of(&self.run),
                expected_composition_hash: composition.text_hash,
            },
            |_| Ok(()),
        )
    }
    fn count(&self, table: &str) -> i64 {
        self.conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }
}
#[test]
fn review_commit_has_cycle_and_replays_same_receipt() {
    let f = Fixture::new(ReviewRevisionOperation::ReviewChapter);
    let receipt = f.review().expect("正常审稿必须连同审稿周期一起成功");
    assert_eq!(f.count("reviews"), 1);
    assert_eq!(f.count("review_cycles"), 1);
    assert_eq!(
        serde_json::to_value(f.review().unwrap()).unwrap(),
        serde_json::to_value(receipt).unwrap()
    );
    assert_eq!(f.count("reviews"), 1);
}
#[test]
fn revision_commit_uses_persisted_composition_source_identity() {
    let f = Fixture::new(ReviewRevisionOperation::RefineDraft);
    let receipt = f
        .revision()
        .expect("有效修稿不能使用伪造的零修订号或空哈希");
    assert_eq!(receipt.content, f.artifact.text);
    assert_eq!(f.count("revisions"), 1);
    assert_eq!(f.revision().unwrap(), receipt);
    assert_eq!(f.count("revisions"), 1);
}
#[test]
fn review_receipt_write_failure_rolls_back_business_rows() {
    let f = Fixture::new(ReviewRevisionOperation::ReviewChapter);
    let before = f.count("contents");
    f.conn.execute_batch("CREATE TEMP TRIGGER reject_review_receipt BEFORE UPDATE OF usage_receipt_json ON generation_attempts BEGIN SELECT RAISE(ABORT,'synthetic-receipt-failure'); END;").unwrap();
    let error = f.review().unwrap_err();
    assert!(error.contains("synthetic-receipt-failure"), "{error}");
    assert_eq!(f.count("reviews"), 0, "收据失败不能留下已提交的审稿");
    assert_eq!(f.count("contents"), before, "收据失败不能遗留内容行");
    f.conn
        .execute_batch("DROP TRIGGER reject_review_receipt;")
        .unwrap();
    f.review().expect("排除写入故障后可重试");
    assert_eq!(f.count("reviews"), 1);
    assert_eq!(f.count("review_cycles"), 1);
}

#[test]
fn review_cycle_failure_rolls_back_business_and_effect_receipt() {
    let f = Fixture::new(ReviewRevisionOperation::ReviewChapter);
    let before = f.count("contents");
    let usage_before: String = f
        .conn
        .query_row(
            "SELECT usage_receipt_json FROM generation_attempts",
            [],
            |r| r.get(0),
        )
        .unwrap();
    f.conn.execute_batch("CREATE TEMP TRIGGER reject_review_cycle BEFORE INSERT ON review_cycles BEGIN SELECT RAISE(ABORT,'synthetic-cycle-failure'); END;").unwrap();
    let error = f.review().unwrap_err();
    assert!(error.contains("synthetic-cycle-failure"), "{error}");
    assert_eq!(f.count("reviews"), 0);
    assert_eq!(f.count("review_cycles"), 0);
    assert_eq!(f.count("contents"), before);
    let usage_after: String = f
        .conn
        .query_row(
            "SELECT usage_receipt_json FROM generation_attempts",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(usage_after, usage_before, "周期失败不能提前提交效果收据");
    f.conn
        .execute_batch("DROP TRIGGER reject_review_cycle;")
        .unwrap();
    f.review().unwrap();
    assert_eq!(f.count("reviews"), 1);
}
#[test]
fn revision_receipt_failure_preserves_old_pending_and_allows_retry() {
    let f = Fixture::new(ReviewRevisionOperation::RefineDraft);
    let old = revision_repo::create(
        &f.conn,
        &RevisionCreateParams {
            base_draft_id: f.prepared.context.source.id,
            revision_type: "refine".into(),
            user_prompt: None,
            review_source_id: None,
            content: "旧待合并修稿".into(),
            word_count: 7,
            expected_source: Some(f.prepared.context.source.clone()),
        },
    )
    .unwrap();
    let before = f.count("contents");
    f.conn.execute_batch("CREATE TEMP TRIGGER reject_revision_receipt BEFORE UPDATE OF usage_receipt_json ON generation_attempts WHEN json_extract(NEW.usage_receipt_json,'$.reviewRevisionEffect') IS NOT NULL BEGIN SELECT RAISE(ABORT,'synthetic-revision-receipt-failure'); END;").unwrap();
    let error = f.revision().unwrap_err();
    assert!(
        error.contains("synthetic-revision-receipt-failure"),
        "{error}"
    );
    assert_eq!(f.count("contents"), before);
    assert_eq!(f.count("revisions"), 1);
    let prior = revision_repo::get_full(&f.conn, old.id).unwrap().unwrap();
    assert_eq!(prior.meta.status, "pending");
    assert_eq!(prior.content, "旧待合并修稿");
    f.conn
        .execute_batch("DROP TRIGGER reject_revision_receipt;")
        .unwrap();
    let saved = f.revision().unwrap();
    assert_ne!(saved.id, old.id);
    assert_eq!(
        revision_repo::get_full(&f.conn, old.id)
            .unwrap()
            .unwrap()
            .meta
            .status,
        "discarded"
    );
    assert_eq!(f.count("revisions"), 2);
}
#[test]
fn stale_handle_and_source_probe_fail_before_any_business_write() {
    let f = Fixture::new(ReviewRevisionOperation::ReviewChapter);
    let mut request = ReviewGenerationCommitRequest {
        context_id: f.prepared.context_id.clone(),
        handle: ReviewRevisionGeneration::handle_of(&f.run),
        artifact: ReviewArtifactRef {
            artifact_id: f.artifact.artifact_id.clone(),
            revision: f.artifact.revision,
            text_hash: f.artifact.text_hash.clone(),
        },
    };
    request.handle.epoch = "stale-lease".into();
    assert_eq!(
        f.service
            .commit_review(&f.conn, &request, |_| Ok(()))
            .unwrap_err(),
        "Error: GENERATION_RUN_IDENTITY_MISMATCH"
    );
    request.handle = ReviewRevisionGeneration::handle_of(&f.run);
    assert_eq!(
        f.service
            .commit_review(&f.conn, &request, |_| Err("synthetic-source-drift".into()))
            .unwrap_err(),
        "Error: GENERATION_SOURCE_CHANGED"
    );
    assert_eq!(f.count("reviews"), 0);
    assert_eq!(f.count("review_cycles"), 0);
    f.review().unwrap();
}
#[test]
fn discarded_artifact_cannot_commit_business_rows() {
    let f = Fixture::new(ReviewRevisionOperation::ReviewChapter);
    f.service
        .runs
        .discard_candidate(&f.conn, &f.artifact.artifact_id)
        .unwrap();
    assert_eq!(
        f.review().unwrap_err(),
        "Error: GENERATION_REVIEW_ARTIFACT_INVALID"
    );
    assert_eq!(f.count("reviews"), 0);
}
