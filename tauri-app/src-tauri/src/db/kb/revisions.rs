//! 生成知识快照的持久修订号（2026-10-17 用户授权的增量迁移）。
//! 修订号随规范写入在同一事务递增；回滚不消耗修订，FTS/向量重建不递增。
use rusqlite::{Connection, OptionalExtension};

const CREATE_META: &str = r#"
CREATE TABLE kb_source_revisions (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  canonical_revision INTEGER NOT NULL CHECK (typeof(canonical_revision) = 'integer' AND canonical_revision >= 0),
  documents_revision INTEGER NOT NULL CHECK (typeof(documents_revision) = 'integer' AND documents_revision >= 0)
);
INSERT INTO kb_source_revisions VALUES (1, 0, 0);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRevisions {
    pub canonical: i64,
    pub documents: i64,
}

struct RevisionSavepoint<'a> {
    conn: &'a Connection,
    committed: bool,
}

impl<'a> RevisionSavepoint<'a> {
    fn new(conn: &'a Connection) -> rusqlite::Result<Self> {
        conn.execute_batch("SAVEPOINT lorekeeper_kb_revision_migration")?;
        Ok(Self {
            conn,
            committed: false,
        })
    }

    fn commit(mut self) -> rusqlite::Result<()> {
        self.conn
            .execute_batch("RELEASE lorekeeper_kb_revision_migration")?;
        self.committed = true;
        Ok(())
    }
}

impl std::ops::Deref for RevisionSavepoint<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn
    }
}

impl Drop for RevisionSavepoint<'_> {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.conn.execute_batch("ROLLBACK TO lorekeeper_kb_revision_migration; RELEASE lorekeeper_kb_revision_migration");
        }
    }
}

/// 使用保存点兼容外层事务；既有库不重置计数，损坏元数据不得静默补零。
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let savepoint = RevisionSavepoint::new(conn)?;
    let exists = savepoint
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE type='table' AND name='kb_source_revisions'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        savepoint.execute_batch(CREATE_META)?;
    }
    read(&savepoint)?;
    for (table, column) in [
        ("kb_chunks", "canonical_revision"),
        ("kb_documents", "documents_revision"),
    ] {
        for event in ["INSERT", "UPDATE", "DELETE"] {
            let name = format!("kb_source_revision_{table}_{}", event.to_lowercase());
            // 对缺行和整数溢出 fail closed；不使用 REPLACE 或写前内存缓存。
            savepoint.execute_batch(&format!(
                r#"
CREATE TRIGGER IF NOT EXISTS {name} AFTER {event} ON {table} BEGIN
  SELECT CASE WHEN (SELECT COUNT(*) FROM kb_source_revisions WHERE id=1) != 1
    THEN RAISE(ABORT, 'GENERATION_KNOWLEDGE_INVALID_REVISION') END;
  UPDATE kb_source_revisions SET {column} = {column} + 1 WHERE id=1;
END;"#
            ))?;
        }
    }
    savepoint.commit()
}

pub fn read(conn: &Connection) -> rusqlite::Result<SourceRevisions> {
    let (id, canonical, documents, count): (i64, i64, i64, i64) = conn.query_row(
        "SELECT id, canonical_revision, documents_revision, (SELECT COUNT(*) FROM kb_source_revisions) FROM kb_source_revisions WHERE id=1",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    )?;
    if id != 1 || count != 1 || canonical < 0 || documents < 0 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(SourceRevisions {
        canonical,
        documents,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{
        kb::{fts, store},
        schema,
    };

    fn legacy() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(schema::CREATE_KB_DOCUMENTS).unwrap();
        conn.execute_batch(schema::CREATE_KB_CHUNKS).unwrap();
        conn.execute_batch(schema::CREATE_KB_FTS).unwrap();
        store::insert_document(
            &conn,
            "doc",
            "合成知识",
            &["合成规范材料".into()],
            &store::DocumentMeta {
                corpus_kind: "project-knowledge",
                ..Default::default()
            },
        )
        .unwrap();
        conn
    }

    #[test]
    fn migration_preserves_legacy_data_is_idempotent_and_counts_all_events() {
        let conn = legacy();
        migrate(&conn).unwrap();
        assert_eq!(
            read(&conn).unwrap(),
            SourceRevisions {
                canonical: 0,
                documents: 0
            }
        );
        assert_eq!(store::list_documents(&conn).unwrap().len(), 1);
        conn.execute("UPDATE kb_chunks SET text='合成变更'", [])
            .unwrap();
        conn.execute("UPDATE kb_documents SET file_name='新合成知识'", [])
            .unwrap();
        let before = read(&conn).unwrap();
        assert_eq!(
            before,
            SourceRevisions {
                canonical: 1,
                documents: 1
            }
        );
        migrate(&conn).unwrap();
        assert_eq!(read(&conn).unwrap(), before);
        conn.execute("DELETE FROM kb_chunks", []).unwrap();
        conn.execute("DELETE FROM kb_documents", []).unwrap();
        store::insert_document(
            &conn,
            "next",
            "合成知识",
            &["后续材料".into()],
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            read(&conn).unwrap(),
            SourceRevisions {
                canonical: 3,
                documents: 3
            }
        );
    }

    #[test]
    fn rollback_and_rebuilt_indexes_do_not_invent_source_revisions() {
        let conn = legacy();
        migrate(&conn).unwrap();
        let before = read(&conn).unwrap();
        {
            let tx = conn.unchecked_transaction().unwrap();
            tx.execute("UPDATE kb_chunks SET text='待回滚'", [])
                .unwrap();
            tx.execute("UPDATE kb_documents SET file_name='待回滚'", [])
                .unwrap();
            assert_ne!(read(&tx).unwrap(), before);
        }
        assert_eq!(read(&conn).unwrap(), before);
        fts::clear(&conn).unwrap();
        assert_eq!(read(&conn).unwrap(), before);
        // 同一外层事务撤销迁移，不留下表或部分触发器。
        let old = legacy();
        let tx = old.unchecked_transaction().unwrap();
        migrate(&tx).unwrap();
        tx.rollback().unwrap();
        assert!(read(&old).is_err());
        assert_eq!(
            old.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type='trigger'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn missing_metadata_and_overflow_refuse_writes_without_resetting() {
        let conn = legacy();
        migrate(&conn).unwrap();
        conn.execute("DELETE FROM kb_source_revisions", []).unwrap();
        assert!(migrate(&conn).is_err());
        assert!(conn
            .execute("UPDATE kb_chunks SET text='拒绝写入'", [])
            .is_err());
        conn.execute(
            "INSERT INTO kb_source_revisions VALUES (1, 9223372036854775807, 0)",
            [],
        )
        .unwrap();
        assert!(conn
            .execute("UPDATE kb_chunks SET text='溢出写入'", [])
            .is_err());
        assert_eq!(read(&conn).unwrap().canonical, i64::MAX);
    }

    #[test]
    fn second_connection_observes_only_committed_revisions() {
        let root = std::env::temp_dir().join(format!(
            "lorekeeper-kb-revisions-{}",
            crate::project_access::random_uuid_v4()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("fixture.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
        schema::create_tables(&conn).unwrap();
        let other = Connection::open(&path).unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        store::insert_document(
            &tx,
            "doc",
            "合成知识",
            &["隔离材料".into()],
            &Default::default(),
        )
        .unwrap();
        assert_eq!(read(&other).unwrap().canonical, 0);
        tx.commit().unwrap();
        assert_eq!(
            read(&other).unwrap(),
            SourceRevisions {
                canonical: 1,
                documents: 1
            }
        );
        drop(other);
        drop(conn);
        std::fs::remove_dir_all(root).unwrap();
    }
}
