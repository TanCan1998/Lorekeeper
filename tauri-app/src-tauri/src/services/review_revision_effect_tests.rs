//! 重复效果使用各自真实run/attempt/artifact，异常输入仅作用于合成内存库。
use super::*;

fn duplicate_claim(f: &Fixture, kind: &str, id: i64, foreign_root: bool) {
    let (binding_raw, usage_raw, artifact_raw, root): (String, String, String, String) = f
        .conn
        .query_row(
            "SELECT r.binding_json,a.usage_receipt_json,v.artifact_json,a.root_action_id
         FROM generation_attempts a JOIN generation_runs r ON r.run_id=a.run_id
         JOIN generation_artifacts v ON v.attempt_id=a.attempt_id
         WHERE json_extract(a.usage_receipt_json,'$.reviewRevisionEffect.kind')=?1
           AND json_extract(a.usage_receipt_json,'$.reviewRevisionEffect.id')=?2",
            rusqlite::params![kind, id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    let binding: Value = serde_json::from_str(&binding_raw).unwrap();
    let mut usage: Value = serde_json::from_str(&usage_raw).unwrap();
    let original: VisibleArtifact = serde_json::from_str(&artifact_raw).unwrap();
    let prepared = PreparedReviewRevisionContext {
        context_id: random_uuid_v4(),
        context: serde_json::from_value(binding["sourceManifest"]["reviewRevisionContext"].clone())
            .unwrap(),
        parent_root_action_id: if foreign_root {
            None
        } else {
            Some(root.clone())
        },
        model_id: Some("synthetic-model".into()),
    };
    let (run, artifact) = generate(f, &prepared, &original.text);
    assert_eq!(run.root_action_id == root, !foreign_root);
    assert!(
        review_cycle::verify_m03_review_cycle(&f.conn),
        "尚未注入重复效果时必须合法"
    );
    usage["artifactIdentity"]["artifactId"] = json!(artifact.artifact_id);
    usage["artifactIdentity"]["epoch"] = json!(artifact.epoch);
    usage["artifactIdentity"]["fingerprint"] = serde_json::to_value(&artifact.fingerprint).unwrap();
    usage["reviewRevisionEffect"]["artifact"] = json!({
        "artifactId":artifact.artifact_id,"revision":artifact.revision,"textHash":artifact.text_hash
    });
    if kind == "revision" {
        usage["visibleComposition"]["artifactIds"] = json!([artifact.artifact_id]);
        usage["visibleComposition"]["sources"] = json!([{
            "artifactId":artifact.artifact_id,"revision":artifact.revision,"textHash":artifact.text_hash
        }]);
    }
    f.conn
        .execute(
            "UPDATE generation_attempts SET usage_receipt_json=?1 WHERE attempt_id=?2",
            rusqlite::params![usage.to_string(), artifact.attempt_id],
        )
        .unwrap();
    assert!(
        !review_cycle::verify_m03_review_cycle(&f.conn),
        "同一业务效果不能由另一真实run/attempt重复声明，包括跨root情形"
    );
}

#[test]
fn effect_proof_rejects_duplicate_initial_review_in_same_and_foreign_roots() {
    for foreign in [false, true] {
        let f = Fixture::new(ReviewRevisionOperation::ReviewChapter);
        let saved = f.review().unwrap();
        duplicate_claim(&f, "review", saved.id, foreign);
    }
}

#[test]
fn effect_proof_rejects_duplicate_revision_in_same_and_foreign_roots() {
    for foreign in [false, true] {
        let (f, _) = merged_fixture();
        let id: i64 = f
            .conn
            .query_row("SELECT revision_id FROM review_cycles", [], |r| r.get(0))
            .unwrap();
        duplicate_claim(&f, "revision", id, foreign);
    }
}

#[test]
fn effect_proof_rejects_duplicate_recheck_in_same_and_foreign_roots() {
    for foreign in [false, true] {
        let (f, _, handle) = rechecked_fixture(Some(false));
        let id: i64 = f
            .conn
            .query_row(
                "SELECT json_extract(usage_receipt_json,'$.reviewRevisionEffect.id')
             FROM generation_attempts WHERE run_id=?1",
                [&handle.run_id],
                |r| r.get(0),
            )
            .unwrap();
        duplicate_claim(&f, "review", id, foreign);
    }
}

#[test]
fn effect_proof_rejects_malformed_rehashed_review_artifact_without_panicking() {
    let f = Fixture::new(ReviewRevisionOperation::ReviewChapter);
    f.review().unwrap();
    let (artifact_raw, usage_raw): (String, String) = f
        .conn
        .query_row(
            "SELECT v.artifact_json,a.usage_receipt_json FROM generation_artifacts v
         JOIN generation_attempts a ON a.attempt_id=v.attempt_id",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    for text in [
        "不是JSON",
        "[]",
        "{}",
        r#"{"summary":"合成报告","items":[{"category":"合成","severity":"warning","description":"缺失证据"}]}"#,
    ] {
        f.conn
            .execute_batch("SAVEPOINT synthetic_invalid_report")
            .unwrap();
        let mut artifact: Value = serde_json::from_str(&artifact_raw).unwrap();
        let mut usage: Value = serde_json::from_str(&usage_raw).unwrap();
        artifact["text"] = json!(text);
        artifact["textHash"] = json!(text_hash(text));
        usage["reviewRevisionEffect"]["artifact"]["textHash"] = json!(text_hash(text));
        f.conn
            .execute(
                "UPDATE generation_artifacts SET artifact_json=?1",
                [artifact.to_string()],
            )
            .unwrap();
        f.conn
            .execute(
                "UPDATE generation_attempts SET usage_receipt_json=?1",
                [usage.to_string()],
            )
            .unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            review_cycle::verify_m03_review_cycle(&f.conn)
        }));
        assert!(
            matches!(result, Ok(false)),
            "不可信报告必须失败关闭，不能panic"
        );
        let saved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.service.saved(&f.conn, &f.run)
        }));
        assert!(saved.is_ok(), "保存结果恢复也不能因畸形历史报告panic");
        assert!(saved.unwrap().is_err());
        f.conn
            .execute_batch("ROLLBACK TO synthetic_invalid_report; RELEASE synthetic_invalid_report")
            .unwrap();
        assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    }
}

#[test]
fn effect_proof_binds_root_action_json_identity_and_operation() {
    let f = Fixture::new(ReviewRevisionOperation::ReviewChapter);
    f.review().unwrap();
    let raw: String = f
        .conn
        .query_row("SELECT action_json FROM generation_roots", [], |r| r.get(0))
        .unwrap();
    for (key, value) in [
        ("rootActionId", json!("wrong-root")),
        ("operation", json!("refine-draft")),
    ] {
        let mut action: Value = serde_json::from_str(&raw).unwrap();
        action[key] = value;
        f.conn
            .execute(
                "UPDATE generation_roots SET action_json=?1",
                [action.to_string()],
            )
            .unwrap();
        assert!(!review_cycle::verify_m03_review_cycle(&f.conn));
        f.conn
            .execute("UPDATE generation_roots SET action_json=?1", [&raw])
            .unwrap();
        assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    }
}

#[test]
fn effect_proof_rejects_project_epoch_and_artifact_identity_mismatches() {
    // 三条链分别创建真实初审、修稿和复核；单一身份字段篡改必须失败关闭。
    for stage in ["review", "revision", "recheck"] {
        let f = match stage {
            "review" => {
                let f = Fixture::new(ReviewRevisionOperation::ReviewChapter);
                f.review().unwrap();
                f
            }
            "revision" => merged_fixture().0,
            _ => rechecked_fixture(Some(false)).0,
        };
        let kind = if stage == "revision" {
            "revision"
        } else {
            "review"
        };
        let (attempt_id, run_id, artifact_id): (String, String, String) = f.conn.query_row(
            "SELECT a.attempt_id,a.run_id,json_extract(a.usage_receipt_json,'$.reviewRevisionEffect.artifact.artifactId')
             FROM generation_attempts a WHERE json_extract(a.usage_receipt_json,'$.reviewRevisionEffect.kind')=?1
             ORDER BY a.rowid DESC LIMIT 1",
            [kind], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
        for (table, column, key, id, pointer, replacement) in [
            (
                "generation_runs",
                "binding_json",
                "run_id",
                run_id.as_str(),
                "/projectId",
                json!("foreign-project"),
            ),
            (
                "generation_runs",
                "binding_json",
                "run_id",
                run_id.as_str(),
                "/epoch",
                json!(""),
            ),
            (
                "generation_artifacts",
                "artifact_json",
                "artifact_id",
                artifact_id.as_str(),
                "/projectId",
                json!("foreign-project"),
            ),
            (
                "generation_artifacts",
                "artifact_json",
                "artifact_id",
                artifact_id.as_str(),
                "/epoch",
                json!(""),
            ),
            (
                "generation_artifacts",
                "artifact_json",
                "artifact_id",
                artifact_id.as_str(),
                "/artifactId",
                json!("foreign-artifact"),
            ),
            (
                "generation_artifacts",
                "artifact_json",
                "artifact_id",
                artifact_id.as_str(),
                "/revision",
                json!(999),
            ),
            (
                "generation_artifacts",
                "artifact_json",
                "artifact_id",
                artifact_id.as_str(),
                "/fingerprint/templateHash",
                json!("9".repeat(64)),
            ),
            (
                "generation_attempts",
                "usage_receipt_json",
                "attempt_id",
                attempt_id.as_str(),
                "/artifactIdentity/artifactId",
                json!("foreign-artifact"),
            ),
            (
                "generation_attempts",
                "usage_receipt_json",
                "attempt_id",
                attempt_id.as_str(),
                "/artifactIdentity/epoch",
                json!("foreign-epoch"),
            ),
            (
                "generation_attempts",
                "usage_receipt_json",
                "attempt_id",
                attempt_id.as_str(),
                "/artifactIdentity/fingerprint/templateHash",
                json!("9".repeat(64)),
            ),
            (
                "generation_attempts",
                "usage_receipt_json",
                "attempt_id",
                attempt_id.as_str(),
                "/reviewRevisionEffect/artifact/revision",
                json!(999),
            ),
            (
                "generation_attempts",
                "usage_receipt_json",
                "attempt_id",
                attempt_id.as_str(),
                "/reviewRevisionEffect/artifact/textHash",
                json!("9".repeat(64)),
            ),
        ] {
            let raw: String = f
                .conn
                .query_row(
                    &format!("SELECT {column} FROM {table} WHERE {key}=?1"),
                    [id],
                    |r| r.get(0),
                )
                .unwrap();
            let mut value: Value = serde_json::from_str(&raw).unwrap();
            *value
                .pointer_mut(pointer)
                .expect("测试路径必须属于真实收据合同") = replacement;
            let update = format!("UPDATE {table} SET {column}=?1 WHERE {key}=?2");
            f.conn
                .execute(&update, rusqlite::params![value.to_string(), id])
                .unwrap();
            assert!(
                !review_cycle::verify_m03_review_cycle(&f.conn),
                "{stage} 未拒绝 {table}{pointer}"
            );
            f.conn.execute(&update, rusqlite::params![raw, id]).unwrap();
            assert!(
                review_cycle::verify_m03_review_cycle(&f.conn),
                "{stage} 恢复后仍拒绝 {table}{pointer}"
            );
        }
    }
}

#[test]
fn effect_proof_preserves_historical_artifact_epoch_after_legitimate_resume() {
    let f = Fixture::new(ReviewRevisionOperation::ReviewChapter);
    // 在提交前暂停恢复；已提交效果会封存root，不能假设它仍可恢复执行。
    f.service
        .runs
        .pause(&f.conn, &f.run.root_action_id, false)
        .unwrap();
    let mut next = f.run.binding.clone();
    next.epoch = "resumed-synthetic-epoch".into();
    f.service
        .runs
        .resume(&f.conn, &f.run.run_id, &next)
        .unwrap();
    let resumed = f.service.runs.get(&f.conn, &f.run.run_id).unwrap();
    assert_eq!(resumed.binding.epoch, next.epoch);
    assert_ne!(f.artifact.epoch, next.epoch);
    f.service
        .commit_review(
            &f.conn,
            &ReviewGenerationCommitRequest {
                context_id: f.prepared.context_id.clone(),
                handle: ReviewRevisionGeneration::handle_of(&resumed),
                artifact: ReviewArtifactRef {
                    artifact_id: f.artifact.artifact_id.clone(),
                    revision: f.artifact.revision,
                    text_hash: f.artifact.text_hash.clone(),
                },
            },
            |_| Ok(()),
        )
        .unwrap();
    assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    assert!(f.service.saved(&f.conn, &resumed).unwrap().is_some());
}
