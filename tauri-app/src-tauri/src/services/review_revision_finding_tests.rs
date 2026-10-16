//! 集合摘要自洽不等于初审来源可信；测试主动重算摘要以避免只覆盖坏哈希。
use super::*;
use rusqlite::types::{Value as SqlValue, ValueRef};

fn reviewed_fixture() -> (Fixture, String) {
    let f = Fixture::with_report(
        ReviewRevisionOperation::ReviewChapter,
        Some(("调整合成目标", "甲\r\n乙")),
        Some(json!({"goalStatus":"unmet","items":[
            {"category":"剧情连贯性","severity":"pass","description":"合成检查"}
        ]})),
    );
    f.review().unwrap();
    let cycle = f
        .conn
        .query_row("SELECT cycle_id FROM review_cycles", [], |r| r.get(0))
        .unwrap();
    (f, cycle)
}

/// 测试端独立按既有规范字段顺序计算摘要，先断言与未篡改库一致。
fn current_set_hash(f: &Fixture, cycle: &str) -> String {
    let names = [
        "findingId",
        "sourceProjectId",
        "sourceEpoch",
        "sourceId",
        "sourceRevision",
        "sourceContentHash",
        "spanStart",
        "spanEnd",
        "spanUnit",
        "excerptHash",
        "occurrence",
        "category",
        "targetId",
        "kind",
        "problemText",
        "expectedText",
    ];
    let mut stmt = f
        .conn
        .prepare(
            "SELECT finding_id,source_project_id,source_epoch,source_id,source_revision,
                source_content_hash,span_start,span_end,span_unit,excerpt_hash,occurrence,
                category,target_id,kind,problem_text,expected_text
         FROM review_findings WHERE cycle_id=?1 ORDER BY finding_id",
        )
        .unwrap();
    let objects: Vec<String> = stmt
        .query_map([cycle], |row| {
            let mut pairs = Vec::new();
            for (index, name) in names.iter().enumerate() {
                let value = match row.get_ref(index)? {
                    ValueRef::Null => "null".into(),
                    ValueRef::Integer(value) => value.to_string(),
                    ValueRef::Text(value) => {
                        serde_json::to_string(std::str::from_utf8(value).unwrap()).unwrap()
                    }
                    _ => panic!("合成finding只应包含整数、文本或NULL"),
                };
                pairs.push(format!("{}:{value}", serde_json::to_string(name).unwrap()));
            }
            Ok(format!("{{{}}}", pairs.join(",")))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    text_hash(&format!("[{}]", objects.join(",")))
}

fn verify_rehashed_tamper(f: &Fixture, cycle: &str, mutation: impl FnOnce()) {
    let original: String = f
        .conn
        .query_row(
            "SELECT finding_set_hash FROM review_cycles WHERE cycle_id=?1",
            [cycle],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(current_set_hash(f, cycle), original);
    f.conn.execute_batch("SAVEPOINT synthetic_forgery").unwrap();
    mutation();
    f.conn
        .execute(
            "UPDATE review_cycles SET finding_set_hash=?1 WHERE cycle_id=?2",
            rusqlite::params![current_set_hash(f, cycle), cycle],
        )
        .unwrap();
    assert!(
        !review_cycle::verify_m03_review_cycle(&f.conn),
        "重新计算摘要不能使伪造finding集合获得原始报告来源"
    );
    f.conn
        .execute_batch("ROLLBACK TO synthetic_forgery; RELEASE synthetic_forgery")
        .unwrap();
    assert!(review_cycle::verify_m03_review_cycle(&f.conn));
}

#[test]
fn finding_origin_rejects_rehashed_immutable_identity_and_problem_changes() {
    let (f, cycle) = reviewed_fixture();
    for (column, value) in [
        ("source_project_id", SqlValue::Text("forged-project".into())),
        ("source_epoch", SqlValue::Text("forged-epoch".into())),
        ("source_id", SqlValue::Text("draft:999".into())),
        ("source_revision", SqlValue::Integer(99)),
        ("source_content_hash", SqlValue::Text("0".repeat(64))),
        ("finding_id", SqlValue::Text("finding:forged".into())),
        ("problem_text", SqlValue::Text("伪造的合成问题".into())),
        ("category", SqlValue::Text("伪造分类".into())),
        ("expected_text", SqlValue::Text("伪造合成目标".into())),
        ("kind", SqlValue::Text("literary".into())),
    ] {
        verify_rehashed_tamper(&f, &cycle, || {
            f.conn
                .execute(&format!("UPDATE review_findings SET {column}=?1"), [value])
                .unwrap();
        });
    }
}

#[test]
fn finding_origin_rejects_missing_report_finding_even_with_matching_empty_hash() {
    let (f, cycle) = reviewed_fixture();
    verify_rehashed_tamper(&f, &cycle, || {
        f.conn
            .execute("DELETE FROM review_findings WHERE cycle_id=?1", [&cycle])
            .unwrap();
    });
}

#[test]
fn finding_origin_rejects_extra_unanchored_finding_with_recomputed_hash() {
    let (f, cycle) = reviewed_fixture();
    verify_rehashed_tamper(&f, &cycle, || {
        f.conn.execute(
            "INSERT INTO review_findings SELECT cycle_id,finding_id||'-extra',source_project_id,
                source_epoch,source_id,source_revision,source_content_hash,NULL,NULL,NULL,NULL,NULL,
                category,NULL,kind,problem_text,expected_text,'unverified',NULL,NULL
             FROM review_findings WHERE cycle_id=?1", [&cycle],
        ).unwrap();
    });
}

#[test]
fn finding_origin_rejects_state_or_evidence_before_any_recheck() {
    let (f, _) = reviewed_fixture();
    for (status, evidence) in [
        ("unknown", None),
        ("resolved", Some("a".repeat(64))),
        ("unresolved", Some("b".repeat(64))),
    ] {
        f.conn.execute_batch("SAVEPOINT synthetic_status").unwrap();
        f.conn
            .execute(
                "UPDATE review_findings SET status=?1,evidence_hash=?2",
                rusqlite::params![status, evidence],
            )
            .unwrap();
        assert!(
            !review_cycle::verify_m03_review_cycle(&f.conn),
            "没有复核收据不能改变初审状态或添加证据"
        );
        f.conn
            .execute_batch("ROLLBACK TO synthetic_status; RELEASE synthetic_status")
            .unwrap();
        assert!(review_cycle::verify_m03_review_cycle(&f.conn));
    }
}

#[test]
fn finding_origin_keeps_unchanged_goal_state_before_and_after_other_goal_recheck() {
    for complete_recheck in [false, true] {
        let f = Fixture::with_report(
            ReviewRevisionOperation::ReviewChapter,
            Some(("调整合成目标；保留合成尾声", "甲\r\n乙")),
            Some(
                json!({"items":[{"category":"剧情连贯性","severity":"pass","description":"合成检查"}],
                "goalReviews":[
                    {"id":"ch1:keyEvents:1","status":"unmet","description":"第一项待调整","evidence":[{"quote":"甲\r\n乙"}]},
                    {"id":"ch1:keyEvents:2","status":"unmet","description":"第二项保持原样","evidence":[{"quote":"e\u{301}"}]}
                ]}),
            ),
        );
        let revised = "🔑开场。甲与乙共同完成了合成任务，问题已经调整。e\u{301}尾声。";
        let (f, cycle) = merge_reviewed_fixture(f, false, &[revised]);
        let plan = review_cycle::plan_recheck(&cycle, &f.conn).unwrap();
        let context = plan.context.unwrap();
        assert_eq!(context.findings.len(), 1, "正文未变化的第二项不得进入复核");
        assert_eq!(context.findings[0].target_id, "ch1:keyEvents:1");
        if complete_recheck {
            let row = crate::repositories::draft_repository::get_full(
                &f.conn,
                f.prepared.context.source.id,
            )
            .unwrap()
            .unwrap();
            let source = ExpectedDraftSource {
                id: row.meta.id,
                chapter_number: row.meta.chapter_number,
                version: row.meta.version,
                status: row.meta.status,
                content: row.content,
            };
            let prepared = prepare(
                &f,
                &source,
                ReviewRevisionOperation::ReviewChapter,
                None,
                Some(&cycle),
            );
            let finding = &context.findings[0];
            let text = json!({"summary":"只复核已变化的合成目标","items":[{
                "findingId":finding.finding_id,"targetId":finding.target_id,"resolved":false,
                "evidenceQuote":"甲与乙共同完成了合成任务","reason":"仍需作者核实"
            }]})
            .to_string();
            let (run, artifact) = generate(&f, &prepared, &text);
            f.service
                .commit_review(
                    &f.conn,
                    &ReviewGenerationCommitRequest {
                        context_id: prepared.context_id,
                        handle: ReviewRevisionGeneration::handle_of(&run),
                        artifact: ReviewArtifactRef {
                            artifact_id: artifact.artifact_id,
                            revision: artifact.revision,
                            text_hash: artifact.text_hash,
                        },
                    },
                    |_| Ok(()),
                )
                .unwrap();
        }
        assert!(review_cycle::verify_m03_review_cycle(&f.conn));
        for (status, evidence) in [
            ("unknown", None),
            ("resolved", Some("a".repeat(64))),
            ("unresolved", Some("b".repeat(64))),
        ] {
            f.conn
                .execute_batch("SAVEPOINT synthetic_unchanged")
                .unwrap();
            f.conn.execute("UPDATE review_findings SET status=?1,evidence_hash=?2 WHERE target_id='ch1:keyEvents:2'",
                rusqlite::params![status,evidence]).unwrap();
            assert!(
                !review_cycle::verify_m03_review_cycle(&f.conn),
                "不参与复核的finding不能借另一项的收据改变状态或获得证据"
            );
            f.conn
                .execute_batch("ROLLBACK TO synthetic_unchanged; RELEASE synthetic_unchanged")
                .unwrap();
            assert!(review_cycle::verify_m03_review_cycle(&f.conn));
        }
        assert_eq!(
            f.count("generation_attempts"),
            if complete_recheck { 3 } else { 2 }
        );
    }
}

#[test]
fn finding_origin_preserves_initial_unknown_when_report_has_valid_anchor() {
    let f = Fixture::with_report(
        ReviewRevisionOperation::ReviewChapter,
        Some(("待核实合成目标", "甲\r\n乙")),
        Some(json!({"goalStatus":"unknown","items":[
            {"category":"剧情连贯性","severity":"pass","description":"合成检查"}
        ]})),
    );
    f.review().unwrap();
    let (status, evidence): (String, Option<String>) = f
        .conn
        .query_row(
            "SELECT status,evidence_hash FROM review_findings WHERE target_id='ch1:keyEvents:1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "unknown");
    assert!(evidence.is_none());
    assert!(review_cycle::verify_m03_review_cycle(&f.conn));
}
