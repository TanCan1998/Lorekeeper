//! 多段修稿沿真实账本、合成与周期证明验证，不引入模型或用户资料。
use super::*;

const FIRST: &str = "🔑开场。甲与乙共同完成了合成任务，所有合成材料均已核对清楚。两人按照合成清单逐项记录调整结果，等待作者检查。";
const SECOND: &str = "问题已经调整。e\u{301}尾声。";

fn multipart_fixture() -> (Fixture, String) {
    merged_fixture_with_chunks(false, &[FIRST, SECOND])
}

#[test]
fn multipart_revision_preserves_both_chunks_and_recovers_without_new_attempts() {
    let (f, _) = multipart_fixture();
    let (body, handle): (String, MainGenerationRunHandle) = {
        let row = revision_repo::get_full(&f.conn, 1).unwrap().unwrap();
        let run_id: String = f
            .conn
            .query_row(
                "SELECT run_id FROM generation_attempts
             WHERE json_extract(usage_receipt_json,'$.reviewRevisionEffect.kind')='revision'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let run = f.service.runs.get(&f.conn, &run_id).unwrap();
        (row.content, ReviewRevisionGeneration::handle_of(&run))
    };
    assert_eq!(body, format!("{FIRST}\n\n{SECOND}"));
    assert_eq!(f.count("generation_attempts"), 3);
    let recovery = f
        .service
        .read_recovery(&f.conn, &handle, |_| false, |_| None)
        .unwrap();
    assert_eq!(recovery.saved.unwrap().content, body);
    assert!(!recovery.can_resume);
    assert!(recovery.context_id.is_none());
    assert_eq!(f.count("generation_attempts"), 3);
}

#[test]
fn multipart_revision_removes_overlap_before_effect_proof() {
    let overlap = "甲与乙共同完成了合成任务，所有合成材料均已核对清楚。两人按照合成清单逐项记录调整结果，等待作者检查。";
    let addition = format!("{overlap}{SECOND}");
    let (f, _) = merged_fixture_with_chunks(false, &[FIRST, &addition]);
    let row = revision_repo::get_full(&f.conn, 1).unwrap().unwrap();
    assert_eq!(row.content, format!("{FIRST}\n\n{SECOND}"));
    assert_eq!(row.content.matches(overlap).count(), 1);
}

#[test]
fn multipart_revision_rejects_attempt_owner_moved_to_another_run() {
    let (f, _) = multipart_fixture();
    let (attempt_id, run_id): (String, String) = f.conn.query_row(
        "SELECT attempt_id,run_id FROM generation_attempts
         WHERE json_extract(usage_receipt_json,'$.purpose')='refine-from-review' ORDER BY rowid LIMIT 1",
        [], |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    f.conn
        .execute(
            "UPDATE generation_attempts SET run_id=?1 WHERE attempt_id=?2",
            rusqlite::params![f.run.run_id, attempt_id],
        )
        .unwrap();
    assert!(
        !review_cycle::verify_m03_review_cycle(&f.conn),
        "产物run与物理attempt的run归属必须同时验证"
    );
    f.conn
        .execute(
            "UPDATE generation_attempts SET run_id=?1 WHERE attempt_id=?2",
            rusqlite::params![run_id, attempt_id],
        )
        .unwrap();
    assert!(review_cycle::verify_m03_review_cycle(&f.conn));
}

#[test]
fn multipart_revision_rejects_no_growth_without_business_writes() {
    for chunk in [FIRST.to_string(), "！！！\n\n".to_string()] {
        let f = Fixture::new(ReviewRevisionOperation::RefineDraft);
        let duplicate = if chunk == FIRST {
            f.artifact.text.clone()
        } else {
            chunk
        };
        let next = append_artifact(&f, &f.run, &duplicate, "stop");
        let error = f
            .service
            .runs
            .compose_visible(
                &f.conn,
                &f.run.run_id,
                &[f.artifact.artifact_id.clone(), next.artifact_id],
                None,
                "visible-append-v1",
            )
            .unwrap_err();
        assert_eq!(error, "GENERATION_COMPOSITION_NO_PROGRESS");
        assert_eq!(f.count("revisions"), 0);
        assert_eq!(f.count("review_cycles"), 0);
        assert_eq!(f.count("generation_attempts"), 2);
        f.revision().unwrap();
        assert_eq!(f.count("revisions"), 1);
    }
}

#[test]
fn multipart_revision_rejects_first_chunk_identity_receipt_and_terminal_tampering() {
    let (f, _) = multipart_fixture();
    let (artifact_id, attempt_id): (String, String) = f
        .conn
        .query_row(
            "SELECT a.artifact_id,a.attempt_id FROM generation_artifacts a
         JOIN generation_attempts t ON t.attempt_id=a.attempt_id
         WHERE json_extract(t.usage_receipt_json,'$.purpose')='refine-from-review'
         ORDER BY t.rowid LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let cases = [
        ("generation_artifacts", "artifact_json", "/epoch", json!("")),
        (
            "generation_artifacts",
            "artifact_json",
            "/attemptId",
            json!("wrong-attempt"),
        ),
        (
            "generation_artifacts",
            "artifact_json",
            "/rootActionId",
            json!("wrong-root"),
        ),
        (
            "generation_artifacts",
            "artifact_json",
            "/projectId",
            json!("wrong-project"),
        ),
        (
            "generation_artifacts",
            "artifact_json",
            "/fingerprint/templateHash",
            json!("0".repeat(64)),
        ),
        (
            "generation_artifacts",
            "artifact_json",
            "/textHash",
            json!("0".repeat(64)),
        ),
        (
            "generation_artifacts",
            "artifact_json",
            "/revision",
            json!(99),
        ),
        (
            "generation_attempts",
            "attempt_json",
            "/attemptId",
            json!("wrong-attempt"),
        ),
        (
            "generation_attempts",
            "attempt_json",
            "/rootActionId",
            json!("wrong-root"),
        ),
        (
            "generation_attempts",
            "attempt_json",
            "/status",
            json!("dispatched"),
        ),
        (
            "generation_attempts",
            "usage_receipt_json",
            "/artifactIdentity/epoch",
            json!("wrong-epoch"),
        ),
        (
            "generation_attempts",
            "usage_receipt_json",
            "/artifactIdentity/artifactId",
            json!("wrong-artifact"),
        ),
        (
            "generation_attempts",
            "usage_receipt_json",
            "/result/finishReason",
            json!("error"),
        ),
        (
            "generation_attempts",
            "usage_receipt_json",
            "/result/failureCode",
            json!("synthetic-failure"),
        ),
    ];
    for (table, column, pointer, changed) in cases {
        let (key, id) = if table == "generation_artifacts" {
            ("artifact_id", &artifact_id)
        } else {
            ("attempt_id", &attempt_id)
        };
        let raw: String = f
            .conn
            .query_row(
                &format!("SELECT {column} FROM {table} WHERE {key}=?1"),
                [id],
                |r| r.get(0),
            )
            .unwrap();
        let mut value: Value = serde_json::from_str(&raw).unwrap();
        if pointer == "/result/failureCode" {
            value["result"]["failureCode"] = changed;
        } else {
            *value.pointer_mut(pointer).unwrap() = changed;
        }
        let update = format!("UPDATE {table} SET {column}=?1 WHERE {key}=?2");
        f.conn
            .execute(&update, rusqlite::params![value.to_string(), id])
            .unwrap();
        assert!(
            !review_cycle::verify_m03_review_cycle(&f.conn),
            "{table}.{column}{pointer}"
        );
        f.conn.execute(&update, rusqlite::params![raw, id]).unwrap();
        assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    }
    assert_eq!(f.count("generation_attempts"), 3);
}

#[test]
fn multipart_revision_rejects_reordered_duplicate_or_incomplete_composition_sources() {
    let (f, _) = multipart_fixture();
    let (attempt_id, raw): (String, String) = f
        .conn
        .query_row(
            "SELECT attempt_id,usage_receipt_json FROM generation_attempts
         WHERE json_extract(usage_receipt_json,'$.reviewRevisionEffect.kind')='revision'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let original: Value = serde_json::from_str(&raw).unwrap();
    let first = original["visibleComposition"]["artifactIds"][0].clone();
    let last = original["visibleComposition"]["artifactIds"][1].clone();
    let sources = original["visibleComposition"]["sources"]
        .as_array()
        .unwrap();
    for (ids, refs) in [
        (
            json!([last.clone(), first.clone()]),
            json!([sources[1], sources[0]]),
        ),
        (
            json!([first.clone(), first, last.clone()]),
            json!([sources[0], sources[0], sources[1]]),
        ),
        (json!([last]), json!([sources[1]])),
    ] {
        let mut value = original.clone();
        value["visibleComposition"]["artifactIds"] = ids;
        value["visibleComposition"]["sources"] = refs;
        f.conn
            .execute(
                "UPDATE generation_attempts SET usage_receipt_json=?1 WHERE attempt_id=?2",
                rusqlite::params![value.to_string(), attempt_id],
            )
            .unwrap();
        assert!(!review_cycle::verify_m03_review_cycle(&f.conn));
        f.conn
            .execute(
                "UPDATE generation_attempts SET usage_receipt_json=?1 WHERE attempt_id=?2",
                rusqlite::params![raw, attempt_id],
            )
            .unwrap();
        assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    }
}

#[test]
fn multipart_revision_rejects_attempt_root_column_moved_to_another_root() {
    let (f, _) = multipart_fixture();
    let (attempt_id, original_root): (String, String) = f
        .conn
        .query_row(
            "SELECT attempt_id,root_action_id FROM generation_attempts
         WHERE json_extract(usage_receipt_json,'$.purpose')='refine-from-review'
         ORDER BY rowid LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let (foreign, _) = generate(&f, &f.prepared, "另一个合成运行，不提交审稿。");
    assert_ne!(foreign.root_action_id, original_root);
    assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    f.conn
        .execute(
            "UPDATE generation_attempts SET root_action_id=?1 WHERE attempt_id=?2",
            rusqlite::params![foreign.root_action_id, attempt_id],
        )
        .unwrap();
    assert!(!review_cycle::verify_m03_review_cycle(&f.conn));
    f.conn
        .execute(
            "UPDATE generation_attempts SET root_action_id=?1 WHERE attempt_id=?2",
            rusqlite::params![original_root, attempt_id],
        )
        .unwrap();
    assert!(review_cycle::verify_m03_review_cycle(&f.conn));
}
