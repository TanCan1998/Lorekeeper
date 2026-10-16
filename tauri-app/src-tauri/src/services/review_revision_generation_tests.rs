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
        Self::with_goal(operation, None)
    }
    fn with_goal(operation: ReviewRevisionOperation, goal: Option<(&str, &str)>) -> Self {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn.execute(
            "INSERT INTO project_core (id,project_name) VALUES ('main','合成测试')",
            [],
        )
        .unwrap();
        let source = if goal.is_some() {
            "🔑开场。甲\r\n乙。e\u{301}尾声。".into()
        } else {
            "作者原始正文。".repeat(20)
        };
        if let Some((text, _)) = goal {
            conn.execute(
                "INSERT INTO blueprints(chapter_number,key_events) VALUES (1,?1)",
                [text],
            )
            .unwrap();
        }
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
            let mut value = json!({"summary":"合成审稿","items":[{"category":"剧情连贯性","severity":"pass","description":"未发现矛盾"}]});
            if let Some((_, quote)) = goal {
                let id = &prepared
                    .context
                    .frozen_goals
                    .items
                    .first()
                    .expect("准备阶段必须读取作者蓝图目标")
                    .id;
                value["goalReviews"] = json!([{"id":id,"status":"completed","description":"已展示","evidence":[{"quote":quote}]}]);
            }
            value.to_string()
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
fn review_cycle_verification_requires_exactly_one_report_effect() {
    let f = Fixture::new(ReviewRevisionOperation::ReviewChapter);
    f.review().unwrap();
    assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    let usage: String = f
        .conn
        .query_row(
            "SELECT usage_receipt_json FROM generation_attempts",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut value: Value = serde_json::from_str(&usage).unwrap();
    value["reviewRevisionEffect"]["contentHash"] = Value::String("0".repeat(64));
    f.conn
        .execute(
            "UPDATE generation_attempts SET usage_receipt_json=?1",
            [value.to_string()],
        )
        .unwrap();
    assert!(!review_cycle::verify_m03_review_cycle(&f.conn));
}
#[test]
fn report_proof_rebuilds_frozen_review_and_recheck_versions() {
    let review = Fixture::new(ReviewRevisionOperation::ReviewChapter);
    let proof =
        rebuild_review_report_hash(&review.prepared.context, &review.artifact.text, 2, None)
            .unwrap();
    let receipt = review.review().unwrap();
    assert_eq!(proof, text_hash(&receipt.content));
    assert_eq!(
        rebuild_review_report_hash(&review.prepared.context, &review.artifact.text, 0, None),
        None
    );
    assert_eq!(
        rebuild_review_report_hash(&review.prepared.context, &review.artifact.text, 2, Some(1)),
        None
    );
}
#[test]
fn review_cycle_rejects_receipt_and_artifact_tampering_in_sqlite() {
    let mutations = [
        (
            "generation_attempts",
            "usage_receipt_json",
            "/reviewRevisionEffect/unexpected",
            json!(true),
        ),
        (
            "generation_attempts",
            "usage_receipt_json",
            "/reviewRevisionEffect/reportVersion",
            json!(1),
        ),
        (
            "generation_attempts",
            "usage_receipt_json",
            "/reviewRevisionEffect/contextHash",
            json!("0".repeat(64)),
        ),
        (
            "generation_artifacts",
            "artifact_json",
            "/revision",
            json!(99),
        ),
        (
            "generation_artifacts",
            "artifact_json",
            "/text",
            json!("被替换的合成产物"),
        ),
        (
            "generation_artifacts",
            "artifact_json",
            "/attemptId",
            json!("other-attempt"),
        ),
        (
            "generation_artifacts",
            "artifact_json",
            "/rootActionId",
            json!("other-root"),
        ),
    ];
    for (table, column, pointer, replacement) in mutations {
        let f = Fixture::new(ReviewRevisionOperation::ReviewChapter);
        f.review().unwrap();
        assert!(review_cycle::verify_m03_review_cycle(&f.conn));
        let original: String = f
            .conn
            .query_row(&format!("SELECT {column} FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        let mut value: Value = serde_json::from_str(&original).unwrap();
        if pointer.ends_with("/unexpected") {
            value["reviewRevisionEffect"]["unexpected"] = replacement;
        } else {
            *value.pointer_mut(pointer).unwrap() = replacement;
        }
        let sql = format!("UPDATE {table} SET {column}=?1");
        f.conn.execute(&sql, [value.to_string()]).unwrap();
        assert!(
            !review_cycle::verify_m03_review_cycle(&f.conn),
            "未拒绝 {pointer}"
        );
        f.conn.execute(&sql, [&original]).unwrap();
        assert!(
            review_cycle::verify_m03_review_cycle(&f.conn),
            "恢复后仍拒绝 {pointer}"
        );
    }
}

#[test]
fn recheck_evidence_hash_matches_ordered_payload_and_covers_every_field() {
    let item = json!({"description":"合成复核证据", "resolved":false});
    // 负载顺序与基线 JSON.stringify 的字段插入顺序一致，条目沿用持久化值。
    let expected = text_hash(&format!(
        "{{\"findingId\":\"finding-1\",\"targetId\":\"target-1\",\"reviewItemIndex\":0,\"item\":{},\"resolved\":false}}",
        serde_json::to_string(&item).unwrap(),
    ));
    let actual = recheck_evidence_hash("finding-1", "target-1", 0, &item, false);
    assert_eq!(actual, expected);
    for changed in [
        recheck_evidence_hash("finding-2", "target-1", 0, &item, false),
        recheck_evidence_hash("finding-1", "target-2", 0, &item, false),
        recheck_evidence_hash("finding-1", "target-1", 1, &item, false),
        recheck_evidence_hash(
            "finding-1",
            "target-1",
            0,
            &json!({"resolved":false}),
            false,
        ),
        recheck_evidence_hash("finding-1", "target-1", 0, &item, true),
    ] {
        assert_ne!(actual, changed);
    }
}

#[test]
fn review_cycle_rejects_two_individually_bound_effect_attempts() {
    let f = Fixture::new(ReviewRevisionOperation::ReviewChapter);
    f.review().unwrap();
    assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    let usage_raw: String = f
        .conn
        .query_row(
            "SELECT usage_receipt_json FROM generation_attempts",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let artifact_raw: String = f
        .conn
        .query_row(
            "SELECT artifact_json FROM generation_artifacts",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut usage: Value = serde_json::from_str(&usage_raw).unwrap();
    usage["artifactIdentity"]["artifactId"] = json!("duplicate-artifact");
    usage["reviewRevisionEffect"]["artifact"]["artifactId"] = json!("duplicate-artifact");
    let mut artifact: Value = serde_json::from_str(&artifact_raw).unwrap();
    artifact["artifactId"] = json!("duplicate-artifact");
    artifact["attemptId"] = json!("duplicate-attempt");
    // 克隆完整有效效果而不是仅复制引用，确保实际覆盖匹配数必须为一的约束。
    f.conn.execute(
        "INSERT INTO generation_attempts(attempt_id,reservation_id,run_id,root_action_id,attempt_json,usage_receipt_json,invocation_nonce)
         SELECT 'duplicate-attempt','duplicate-reservation',run_id,root_action_id,attempt_json,?1,'duplicate-nonce' FROM generation_attempts",
        [usage.to_string()],
    ).unwrap();
    f.conn.execute(
        "INSERT INTO generation_artifacts(artifact_id,attempt_id,run_id,artifact_json,revision,status)
         SELECT 'duplicate-artifact','duplicate-attempt',run_id,?1,revision,status FROM generation_artifacts",
        [artifact.to_string()],
    ).unwrap();
    assert!(!review_cycle::verify_m03_review_cycle(&f.conn));
}

#[test]
fn goal_locator_uses_utf16_offsets_without_normalizing_source() {
    let source = "🔑开场。甲\r\n乙。e\u{301}尾声。";
    for (quote, start, end) in [("甲\r\n乙", 5, 9), ("e\u{301}", 10, 12)] {
        let proof = locate_goal_evidence(Some(quote), source).unwrap();
        assert_eq!(proof.quote, quote);
        assert_eq!((proof.start, proof.end), (start, end));
    }
}
#[test]
fn goal_locator_restores_quoted_fallback_but_not_ambiguous_fallback() {
    for quoted in ["“甲乙”", "\"甲乙\""] {
        let proof = locate_goal_evidence(Some(quoted), "🔑甲乙。尾声").unwrap();
        assert_eq!(proof.quote, "甲乙");
        assert_eq!((proof.start, proof.end), (2, 4));
        assert!(locate_goal_evidence(Some(quoted), "甲乙甲乙").is_none());
    }
    assert!(locate_goal_evidence(Some("“哈哈”"), "哈哈哈").is_none());
    // 基线正常命中取首处；只有剥离引号的回退分支要求唯一，不能擅改产品合同。
    assert!(locate_goal_evidence(Some("哈哈"), "哈哈哈").is_some());
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

#[test]
fn utf16_goal_survives_real_sqlite_commit_cycle_and_recovery() {
    let f = Fixture::with_goal(
        ReviewRevisionOperation::ReviewChapter,
        Some(("展示合成证据", "e\u{301}")),
    );
    let receipt = f.review().expect("Unicode证据必须能贯穿报告、周期和回执");
    let body: Value = serde_json::from_str(&receipt.content).unwrap();
    let proof = &body["goalReview"]["items"][0]["evidence"][0];
    assert_eq!(proof["quote"], "e\u{301}");
    assert_eq!(proof["start"], 10);
    assert_eq!(proof["end"], 12);
    assert_eq!(f.count("review_cycles"), 1);
    assert_eq!(receipt.source.content, "🔑开场。甲\r\n乙。e\u{301}尾声。");
    let recovery = f
        .service
        .read_recovery(
            &f.conn,
            &ReviewRevisionGeneration::handle_of(&f.run),
            |_| true,
            |_| Some(serde_json::to_value(&f.artifact).unwrap()),
        )
        .unwrap();
    assert_eq!(recovery.saved, Some(receipt));
    assert!(
        recovery.context_id.is_none(),
        "已提交收据不能再签发可提交上下文"
    );
}
#[test]
fn recovery_rebuilds_prepared_context_without_dispatching_again() {
    let f = Fixture::new(ReviewRevisionOperation::ReviewChapter);
    PREPARED_CONTEXTS
        .lock()
        .unwrap()
        .remove(&f.prepared.context_id);
    let handle = ReviewRevisionGeneration::handle_of(&f.run);
    let recovery = f
        .service
        .read_recovery(
            &f.conn,
            &handle,
            |_| true,
            |_| Some(serde_json::to_value(&f.artifact).unwrap()),
        )
        .unwrap();
    assert!(recovery.can_resume);
    assert!(recovery.saved.is_none());
    let context_id = recovery.context_id.unwrap();
    assert_ne!(context_id, f.prepared.context_id);
    assert_eq!(
        recovery.latest_artifact_finish_reason.as_deref(),
        Some("stop")
    );
    f.service
        .commit_review(
            &f.conn,
            &ReviewGenerationCommitRequest {
                context_id,
                handle,
                artifact: ReviewArtifactRef {
                    artifact_id: f.artifact.artifact_id.clone(),
                    revision: f.artifact.revision,
                    text_hash: f.artifact.text_hash.clone(),
                },
            },
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(f.count("generation_attempts"), 1);
    assert_eq!(f.count("reviews"), 1);
}
#[test]
fn recovery_with_source_drift_exposes_copy_only_without_prepared_context() {
    let f = Fixture::new(ReviewRevisionOperation::ReviewChapter);
    let recovery = f
        .service
        .read_recovery(
            &f.conn,
            &ReviewRevisionGeneration::handle_of(&f.run),
            |_| false,
            |_| Some(serde_json::to_value(&f.artifact).unwrap()),
        )
        .unwrap();
    assert_eq!(recovery.source_status, "conflict");
    assert!(!recovery.can_resume);
    assert!(recovery.context_id.is_none());
    assert!(recovery.saved.is_none());
    assert!(
        recovery.latest_artifact.is_some(),
        "冲突候选仍允许复制，不删除或重放"
    );
    assert_eq!(f.count("generation_attempts"), 1);
    assert_eq!(f.count("reviews"), 0);
}
#[test]
fn recovery_of_saved_receipt_does_not_authorize_a_new_execution_on_source_drift() {
    let f = Fixture::new(ReviewRevisionOperation::ReviewChapter);
    let saved = f.review().unwrap();
    let recovery = f
        .service
        .read_recovery(
            &f.conn,
            &ReviewRevisionGeneration::handle_of(&f.run),
            |_| false,
            |_| Some(serde_json::to_value(&f.artifact).unwrap()),
        )
        .unwrap();
    assert_eq!(recovery.saved, Some(saved));
    assert!(!recovery.can_resume);
    assert!(recovery.context_id.is_none());
    assert_eq!(f.count("generation_attempts"), 1);
    assert_eq!(f.count("reviews"), 1);
}
