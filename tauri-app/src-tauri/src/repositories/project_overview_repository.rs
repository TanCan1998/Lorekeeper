//! 项目概览只读投影；不创建表、不迁移、不把草稿状态直接视为有效定稿。
use std::collections::BTreeSet;

use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};

use super::finalized_continuity_repository::sha256_hex;

const MAX_SAFE: i64 = 9_007_199_254_740_991;

fn positive(value: i64) -> bool {
    (1..=MAX_SAFE).contains(&value)
}

fn chapters(conn: &Connection, sql: &str) -> rusqlite::Result<BTreeSet<i64>> {
    let mut statement = conn.prepare(sql)?;
    let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
    Ok(rows
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|n| positive(*n))
        .collect())
}

fn stage(id: &str, count: usize, total: usize) -> Value {
    json!({"id": id, "count": count, "status": if count == total { "completed" }
        else if count == 0 { "not-started" } else { "in-progress" }})
}

fn planned(id: &str, chapters: &BTreeSet<i64>, total: i64) -> Value {
    let status = if !positive(total) {
        if chapters.is_empty() {
            "unknown"
        } else {
            "in-progress"
        }
    } else if chapters.is_empty() {
        "not-started"
    } else if chapters.iter().filter(|n| **n <= total).count() as i64 == total {
        "completed"
    } else {
        "in-progress"
    };
    json!({"id": id, "count": chapters.len(), "status": status})
}

/// 调用方持有数据库锁；事务确保跨表聚合来自同一读快照。
/// Tauri 没有 Electron 的迁移版本表，按实际读面验表/列，失败即 unavailable，不运行迁移。
pub fn read(
    conn: &Connection,
    project_id: &str,
    excerpt: bool,
    physical_revision: &str,
) -> Result<Value, String> {
    if project_id.is_empty() {
        return Err("PROJECT_OVERVIEW_ID_MISSING".into());
    }
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    for table in [
        "project_core",
        "blueprints",
        "drafts",
        "contents",
        "reviews",
        "characters",
        "finalization_outbox",
    ] {
        let found: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?1 AND type='table')",
                [table],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !found {
            return Err("PROJECT_OVERVIEW_SCHEMA_UNAVAILABLE".into());
        }
    }
    let core = tx.query_row(
        "SELECT project_name, genre, sub_genre, target_audience, total_chapters,
        core_outline, world_setting, golden_finger, protagonist_profile, global_guidance,
        writing_style, premise, worldbuilding, characters_arch, synopsis FROM project_core WHERE id='main'",
        [], |r| {
            let mut fields = Vec::new();
            for i in 0..15 {
                fields.push(match r.get_ref(i)? {
                    rusqlite::types::ValueRef::Text(v) => Value::String(String::from_utf8_lossy(v).into_owned()),
                    rusqlite::types::ValueRef::Integer(v) => json!(v),
                    _ => Value::Null,
                });
            }
            Ok(fields)
        }).optional().map_err(|e| e.to_string())?.ok_or("PROJECT_OVERVIEW_CORE_MISSING")?;
    let filled = |i: usize| core[i].as_str().is_some_and(|s| !s.trim().is_empty());
    let configuration = [1, 2, 3, 5, 6, 7, 8, 9, 10]
        .into_iter()
        .filter(|i| filled(*i))
        .count();
    let architecture = (11..15).filter(|i| filled(*i)).count();
    let blueprints = chapters(&tx, "SELECT DISTINCT chapter_number FROM blueprints")
        .map_err(|e| e.to_string())?;
    let mut statement = tx
        .prepare(
            "SELECT d.chapter_number, d.word_count, d.content_id
        FROM drafts d WHERE d.status IN ('draft','revised','finalized')
        AND NOT EXISTS (SELECT 1 FROM drafts n WHERE n.chapter_number=d.chapter_number
            AND n.status IN ('draft','revised','finalized')
            AND (n.version>d.version OR (n.version=d.version AND n.id>d.id)))
        ORDER BY d.chapter_number DESC, d.version DESC, d.id DESC",
        )
        .map_err(|e| e.to_string())?;
    let drafts = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1).unwrap_or(None),
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    drop(statement);
    let drafting = drafts
        .iter()
        .map(|r| r.0)
        .filter(|n| positive(*n))
        .collect();
    let total_words = drafts
        .iter()
        .filter_map(|r| r.1)
        .filter(|n| (0..=MAX_SAFE).contains(n))
        .try_fold(0i64, |sum, n| sum.checked_add(n).filter(|n| *n <= MAX_SAFE))
        .ok_or("PROJECT_OVERVIEW_COUNT_OVERFLOW")?;
    let review = chapters(
        &tx,
        "SELECT DISTINCT d.chapter_number FROM reviews r JOIN drafts d ON d.id=r.base_draft_id",
    )
    .map_err(|e| e.to_string())?;
    let mut statement = tx
        .prepare(
            "SELECT d.id,d.chapter_number,d.version,c.body,
        f.finalization_id,f.chapter_number,f.content_hash,f.content_snapshot
        FROM drafts d JOIN contents c ON c.id=d.content_id
        LEFT JOIN finalization_outbox f ON f.draft_id=d.id
        WHERE d.status='finalized' AND NOT EXISTS (
            SELECT 1 FROM drafts n WHERE n.chapter_number=d.chapter_number AND n.status='finalized'
            AND (n.version>d.version OR (n.version=d.version AND n.id>d.id)))
        ORDER BY d.chapter_number,d.version DESC,d.id DESC",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    let mut finalized = Vec::new();
    let mut finalized_chapters = BTreeSet::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let id: i64 = row.get(0).map_err(|e| e.to_string())?;
        let chapter: i64 = row.get(1).map_err(|e| e.to_string())?;
        let version: i64 = row.get(2).map_err(|e| e.to_string())?;
        let body: String = row.get(3).map_err(|e| e.to_string())?;
        let receipt: Option<String> = row.get(4).map_err(|e| e.to_string())?;
        let receipt_chapter: Option<i64> = row.get(5).map_err(|e| e.to_string())?;
        let hash: Option<String> = row.get(6).map_err(|e| e.to_string())?;
        let snapshot: Option<String> = row.get(7).map_err(|e| e.to_string())?;
        if positive(id)
            && positive(chapter)
            && positive(version)
            && !body.trim().is_empty()
            && receipt.as_ref().is_some_and(|r| !r.trim().is_empty())
            && receipt_chapter == Some(chapter)
            && snapshot.as_deref() == Some(body.as_str())
            && hash.as_deref() == Some(sha256_hex(&body).as_str())
        {
            finalized_chapters.insert(chapter);
            finalized.push(json!([id, chapter, version, receipt, hash]));
        }
    }
    drop(rows);
    drop(statement);
    let characters: i64 = tx
        .query_row("SELECT COUNT(*) FROM characters WHERE retired=0", [], |r| {
            r.get(0)
        })
        .map_err(|e| e.to_string())?;
    let total = core[4].as_i64().unwrap_or(0);
    let stages = json!([
        stage("configuration", configuration, 9),
        stage("architecture", architecture, 4),
        planned("blueprint", &blueprints, total),
        planned("drafting", &drafting, total),
        planned("review", &review, total),
        planned("finalization", &finalized_chapters, total)
    ]);
    let revision = sha256_hex(&json!({"projectId":project_id, "physicalRevision":physical_revision,
        "core":core, "totalWords":total_words, "characters":characters, "stages":stages, "finalized":finalized}).to_string());
    let mut result = json!({"state":"ready","projectId":project_id,"revision":revision,
        "name":core[0].as_str().unwrap_or("").trim(),"totalWords":total_words,"characters":characters,
        "finalizedChapters":finalized_chapters.len(),"stages":stages});
    if excerpt {
        if let Some((_, _, content_id)) = drafts.first() {
            let body: Option<String> = tx
                .query_row("SELECT body FROM contents WHERE id=?1", [content_id], |r| {
                    r.get(0)
                })
                .optional()
                .map_err(|e| e.to_string())?;
            if let Some(body) = body.filter(|b| !b.trim().is_empty()) {
                let tail: String = body
                    .trim()
                    .chars()
                    .rev()
                    .take(240)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                result["excerpt"] = json!(tail);
            }
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        conn
    }

    #[test]
    fn test_read_rejects_empty_project_id() {
        let conn = memory_db();
        let err = read(&conn, "", false, "rev-0").unwrap_err();
        assert_eq!(err, "PROJECT_OVERVIEW_ID_MISSING");
    }

    #[test]
    fn test_read_rejects_missing_schema() {
        let conn = Connection::open_in_memory().unwrap();
        // 表完全没有初始化
        let err = read(&conn, "proj-1", false, "rev-0").unwrap_err();
        assert_eq!(err, "PROJECT_OVERVIEW_SCHEMA_UNAVAILABLE");
    }

    #[test]
    fn test_read_rejects_missing_core() {
        let conn = memory_db();
        // project_core 表存在但没有 main 行
        let err = read(&conn, "proj-1", false, "rev-0").unwrap_err();
        assert_eq!(err, "PROJECT_OVERVIEW_CORE_MISSING");
    }

    #[test]
    fn test_read_empty_project_stages() {
        let conn = memory_db();
        conn.execute(
            "INSERT INTO project_core (id, project_name, total_chapters) VALUES ('main', '我的测试小说', 10)",
            [],
        )
        .unwrap();

        let overview = read(&conn, "proj-1", false, "rev-0").unwrap();
        assert_eq!(overview["state"], "ready");
        assert_eq!(overview["projectId"], "proj-1");
        assert_eq!(overview["name"], "我的测试小说");
        assert_eq!(overview["totalWords"], 0);
        assert_eq!(overview["characters"], 0);
        assert_eq!(overview["finalizedChapters"], 0);
        assert!(overview.get("excerpt").is_none());

        let stages = overview["stages"].as_array().unwrap();
        assert_eq!(stages.len(), 6);
        assert_eq!(stages[0]["id"], "configuration");
        assert_eq!(stages[0]["status"], "not-started");
        assert_eq!(stages[1]["id"], "architecture");
        assert_eq!(stages[1]["status"], "not-started");
        assert_eq!(stages[2]["id"], "blueprint");
        assert_eq!(stages[2]["status"], "not-started");
        assert_eq!(stages[3]["id"], "drafting");
        assert_eq!(stages[3]["status"], "not-started");
        assert_eq!(stages[4]["id"], "review");
        assert_eq!(stages[4]["status"], "not-started");
        assert_eq!(stages[5]["id"], "finalization");
        assert_eq!(stages[5]["status"], "not-started");
    }

    #[test]
    fn test_read_full_pipeline_progress() {
        let conn = memory_db();
        conn.execute(
            "INSERT INTO project_core (
                id, project_name, genre, sub_genre, target_audience, total_chapters,
                core_outline, world_setting, golden_finger, protagonist_profile, global_guidance,
                writing_style, premise, worldbuilding, characters_arch, synopsis
            ) VALUES (
                'main', '修仙传', '玄幻', '修真', '大众', 2,
                '主线大纲', '宏大世界', '金手指系统', '少年主角', '正能量',
                '爽文', '前提设定', '世界观设定', '角色成长路线', '故事简介'
            )",
            [],
        )
        .unwrap();

        // 蓝图 2 章
        conn.execute(
            "INSERT INTO blueprints (chapter_number, title) VALUES (1, '第一章'), (2, '第二章')",
            [],
        )
        .unwrap();

        // 草稿第 1、2 章
        conn.execute("INSERT INTO contents (body) VALUES ('第一章正文内容')", [])
            .unwrap();
        let content_id1 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, content_id, word_count, status) VALUES (1, 1, ?1, 500, 'draft')",
            [content_id1],
        )
        .unwrap();

        conn.execute("INSERT INTO contents (body) VALUES ('第二章正文内容')", [])
            .unwrap();
        let content_id2 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (chapter_number, version, content_id, word_count, status) VALUES (2, 1, ?1, 600, 'draft')",
            [content_id2],
        )
        .unwrap();

        // 审稿 2 章
        conn.execute(
            "INSERT INTO reviews (base_draft_id, review_index, content_id) VALUES (1, 1, ?1), (2, 1, ?2)",
            [content_id1, content_id2],
        )
        .unwrap();

        // 角色：活跃角色 2 个，退休角色 1 个
        conn.execute(
            "INSERT INTO characters (character_id, name, retired) VALUES ('c1', '主角', 0), ('c2', '配角', 0), ('c3', '亡魂', 1)",
            [],
        )
        .unwrap();

        let overview = read(&conn, "proj-1", false, "rev-1").unwrap();
        assert_eq!(overview["totalWords"], 1100);
        assert_eq!(overview["characters"], 2);

        let stages = overview["stages"].as_array().unwrap();
        // 9 个全填 -> completed
        assert_eq!(stages[0]["status"], "completed");
        assert_eq!(stages[0]["count"], 9);
        // 4 个架构全填 -> completed
        assert_eq!(stages[1]["status"], "completed");
        assert_eq!(stages[1]["count"], 4);
        // 蓝图 2 章，总章节 2 -> completed
        assert_eq!(stages[2]["status"], "completed");
        assert_eq!(stages[2]["count"], 2);
        // 草稿 2 章，总章节 2 -> completed
        assert_eq!(stages[3]["status"], "completed");
        assert_eq!(stages[3]["count"], 2);
        // 审稿 2 章，总章节 2 -> completed
        assert_eq!(stages[4]["status"], "completed");
        assert_eq!(stages[4]["count"], 2);
        // 定稿 0 章 -> not-started
        assert_eq!(stages[5]["status"], "not-started");
        assert_eq!(stages[5]["count"], 0);
    }

    #[test]
    fn test_read_finalization_validation() {
        let conn = memory_db();
        conn.execute(
            "INSERT INTO project_core (id, project_name, total_chapters) VALUES ('main', '定稿测试', 2)",
            [],
        )
        .unwrap();

        let body1 = "第1章完整定稿正文内容";
        let hash1 = sha256_hex(body1);
        conn.execute("INSERT INTO contents (body) VALUES (?1)", [body1])
            .unwrap();
        let c1 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (id, chapter_number, version, content_id, word_count, status) VALUES (10, 1, 1, ?1, 100, 'finalized')",
            [c1],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO finalization_outbox (finalization_id, draft_id, chapter_number, content_hash, content_revision, content_snapshot, target_file_name)
             VALUES ('receipt-1', 10, 1, ?1, 1, ?2, 'ch1.txt')",
            [&hash1, body1],
        )
        .unwrap();

        // 第2章：草稿标记 finalized，但 finalization_outbox 中 hash 篡改不匹配
        let body2 = "第2章正文";
        conn.execute("INSERT INTO contents (body) VALUES (?1)", [body2])
            .unwrap();
        let c2 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (id, chapter_number, version, content_id, word_count, status) VALUES (20, 2, 1, ?1, 200, 'finalized')",
            [c2],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO finalization_outbox (finalization_id, draft_id, chapter_number, content_hash, content_revision, content_snapshot, target_file_name)
             VALUES ('receipt-2', 20, 2, 'bad_hash', 1, ?1, 'ch2.txt')",
            [body2],
        )
        .unwrap();

        let overview = read(&conn, "proj-1", false, "rev-0").unwrap();
        // 只有第1章有效定稿
        assert_eq!(overview["finalizedChapters"], 1);
        let stages = overview["stages"].as_array().unwrap();
        let finalization_stage = &stages[5];
        assert_eq!(finalization_stage["status"], "in-progress");
        assert_eq!(finalization_stage["count"], 1);
    }

    #[test]
    fn test_read_excerpt_truncation() {
        let conn = memory_db();
        conn.execute(
            "INSERT INTO project_core (id, project_name, total_chapters) VALUES ('main', '摘要测试', 1)",
            [],
        )
        .unwrap();

        let long_body = "头".to_string() + &"中".repeat(300) + "尾";
        conn.execute("INSERT INTO contents (body) VALUES (?1)", [&long_body])
            .unwrap();
        let cid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO drafts (id, chapter_number, version, content_id, word_count, status) VALUES (1, 1, 1, ?1, 302, 'draft')",
            [cid],
        )
        .unwrap();

        // excerpt = false: 不包含 excerpt
        let no_excerpt = read(&conn, "proj-1", false, "rev-0").unwrap();
        assert!(no_excerpt.get("excerpt").is_none());

        // excerpt = true: 包含尾部 240 字符
        let with_excerpt = read(&conn, "proj-1", true, "rev-0").unwrap();
        let excerpt = with_excerpt["excerpt"].as_str().unwrap();
        assert_eq!(excerpt.chars().count(), 240);
        assert!(excerpt.ends_with('尾'));
    }
}
