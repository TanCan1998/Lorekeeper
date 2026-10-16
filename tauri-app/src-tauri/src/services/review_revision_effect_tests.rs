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
