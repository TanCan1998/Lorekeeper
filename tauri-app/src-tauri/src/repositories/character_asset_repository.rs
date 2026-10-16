//! 角色头像资产仓储 —— 平移自 `electron/repositories/character-asset-repository.ts`。
//!
//! 表结构见 [`crate::db::schema::CREATE_CHARACTER_AVATAR_ASSETS`]（m05 两表：
//! 已绑定资产 + 未决（孤儿/歧义/未知分叉/仅引用）资产）。
//! 约定：仓储只接收 `&Connection`，不持有全局状态；会话门禁与错误形状由命令层负责。

use std::collections::HashSet;

use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};

/// 基线 `CharacterAssetRecord`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterAssetRecord {
    pub character_id: String,
    pub asset_revision: i64,
    pub relative_path: String,
    pub content_hash: String,
    pub mime: String,
    pub byte_size: i64,
    pub source_reference: String,
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<CharacterAssetRecord> {
    Ok(CharacterAssetRecord {
        character_id: row.get("character_id")?,
        asset_revision: row.get("asset_revision")?,
        relative_path: row.get("relative_path")?,
        content_hash: row.get("content_hash")?,
        mime: row.get("mime")?,
        byte_size: row.get("byte_size")?,
        source_reference: row.get("source_reference")?,
    })
}

/// 基线 `characterExists`：角色存在且未退役
pub fn character_exists(conn: &Connection, character_id: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT 1 FROM characters WHERE character_id=?1 AND retired=0",
        [character_id],
        |_| Ok(()),
    )
    .optional()
    .map_err(|error| error.to_string())
    .map(|row| row.is_some())
}

/// 基线 `read`
pub fn read(conn: &Connection, character_id: &str) -> Result<Option<CharacterAssetRecord>, String> {
    conn.query_row(
        "SELECT * FROM character_avatar_assets WHERE character_id=?1",
        [character_id],
        from_row,
    )
    .optional()
    .map_err(|error| error.to_string())
}

/// 基线 `readMany`：结果按 `character_id` 升序（与基线 SQL 的 ORDER BY 一致）
pub fn read_many(
    conn: &Connection,
    character_ids: &[String],
) -> Result<Vec<CharacterAssetRecord>, String> {
    if character_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", character_ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT * FROM character_avatar_assets WHERE character_id IN ({placeholders}) ORDER BY character_id"
    );
    let mut statement = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params_from_iter(character_ids.iter()), from_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

/// 基线 `nextRevision`
pub fn next_revision(conn: &Connection, character_id: &str) -> Result<i64, String> {
    Ok(read(conn, character_id)?
        .map(|record| record.asset_revision)
        .unwrap_or(0)
        + 1)
}

/// 基线 `replace`：事务内校验角色存在与版本递增，返回被替换的旧记录
pub fn replace(
    conn: &Connection,
    record: &CharacterAssetRecord,
) -> Result<Option<CharacterAssetRecord>, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    if !character_exists(&tx, &record.character_id)? {
        return Err("CHARACTER_NOT_FOUND".to_string());
    }
    let previous = read(&tx, &record.character_id)?;
    let expected = previous
        .as_ref()
        .map(|record| record.asset_revision)
        .unwrap_or(0)
        + 1;
    if record.asset_revision != expected {
        return Err("AVATAR_REVISION_CONFLICT".to_string());
    }
    tx.execute(
        "INSERT INTO character_avatar_assets(
            character_id,asset_revision,relative_path,content_hash,mime,byte_size,source_reference
         ) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(character_id) DO UPDATE SET
            asset_revision=excluded.asset_revision,relative_path=excluded.relative_path,
            content_hash=excluded.content_hash,mime=excluded.mime,byte_size=excluded.byte_size,
            source_reference=excluded.source_reference",
        params![
            record.character_id,
            record.asset_revision,
            record.relative_path,
            record.content_hash,
            record.mime,
            record.byte_size,
            record.source_reference
        ],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(previous)
}

/// 基线 `remove`：事务内删除绑定行并返回旧记录
pub fn remove(
    conn: &Connection,
    character_id: &str,
) -> Result<Option<CharacterAssetRecord>, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    if !character_exists(&tx, character_id)? {
        return Err("CHARACTER_NOT_FOUND".to_string());
    }
    let previous = read(&tx, character_id)?;
    tx.execute(
        "DELETE FROM character_avatar_assets WHERE character_id=?1",
        [character_id],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(previous)
}

/// 基线 `referencedPaths`：已绑定路径 + 未决记录中已保留的路径
pub fn referenced_paths(conn: &Connection) -> Result<HashSet<String>, String> {
    let mut paths = HashSet::new();
    {
        let mut statement = conn
            .prepare("SELECT relative_path FROM character_avatar_assets")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        for row in rows {
            paths.insert(row.map_err(|error| error.to_string())?);
        }
    }
    {
        let mut statement = conn
            .prepare(
                "SELECT preserved_relative_path FROM character_avatar_unresolved \
                 WHERE preserved_relative_path IS NOT NULL",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        for row in rows {
            paths.insert(row.map_err(|error| error.to_string())?);
        }
    }
    Ok(paths)
}

/// 基线两处 `INSERT OR IGNORE INTO character_avatar_unresolved`（`orphan` 处置）
#[allow(clippy::too_many_arguments)]
pub fn insert_unresolved_orphan(
    conn: &Connection,
    record_id: &str,
    source_reference: &str,
    relative_path: &str,
    content_hash: &str,
    mime: &str,
    byte_size: i64,
    candidate_character_ids_json: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO character_avatar_unresolved(
            record_id,disposition,source_reference,source_relative_path,preserved_relative_path,
            content_hash,mime,byte_size,candidate_character_ids_json
         ) VALUES(?1,'orphan',?2,?3,?4,?5,?6,?7,?8)",
        params![
            record_id,
            source_reference,
            relative_path,
            relative_path,
            content_hash,
            mime,
            byte_size,
            candidate_character_ids_json
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        create_tables(&conn).expect("建表失败");
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        conn.execute(
            "INSERT INTO characters(character_id, name) VALUES('char-1', '角色一')",
            [],
        )
        .unwrap();
        conn
    }

    fn record(character_id: &str, revision: i64, path: &str) -> CharacterAssetRecord {
        CharacterAssetRecord {
            character_id: character_id.to_string(),
            asset_revision: revision,
            relative_path: path.to_string(),
            content_hash: "a".repeat(64),
            mime: "image/png".to_string(),
            byte_size: 10,
            source_reference: String::new(),
        }
    }

    #[test]
    fn replace_requires_increasing_revision_and_returns_previous_test() {
        let conn = memory_db();
        assert!(replace(&conn, &record("char-1", 1, "avatars/x/1-a.png"))
            .unwrap()
            .is_none());
        let previous = replace(&conn, &record("char-1", 2, "avatars/x/2-b.png")).unwrap();
        assert_eq!(previous.unwrap().relative_path, "avatars/x/1-a.png");
        assert_eq!(
            replace(&conn, &record("char-1", 2, "avatars/x/2-b.png")).unwrap_err(),
            "AVATAR_REVISION_CONFLICT"
        );
        assert_eq!(
            replace(&conn, &record("char-missing", 1, "avatars/y/1-c.png")).unwrap_err(),
            "CHARACTER_NOT_FOUND"
        );
        assert_eq!(next_revision(&conn, "char-1").unwrap(), 3);
    }

    #[test]
    fn remove_and_referenced_paths_cover_unresolved_rows_test() {
        let conn = memory_db();
        replace(&conn, &record("char-1", 1, "avatars/x/1-a.png")).unwrap();
        insert_unresolved_orphan(
            &conn,
            "record-1",
            "runtime-crash",
            "avatars/z/9-c.png",
            &"b".repeat(64),
            "image/png",
            7,
            "[]",
        )
        .unwrap();
        let paths = referenced_paths(&conn).unwrap();
        assert!(paths.contains("avatars/x/1-a.png"));
        assert!(paths.contains("avatars/z/9-c.png"));
        let removed = remove(&conn, "char-1").unwrap();
        assert_eq!(removed.unwrap().relative_path, "avatars/x/1-a.png");
        assert!(read(&conn, "char-1").unwrap().is_none());
        assert_eq!(
            remove(&conn, "char-missing").unwrap_err(),
            "CHARACTER_NOT_FOUND"
        );
    }

    #[test]
    fn deleting_character_retains_avatar_as_unresolved_orphan_test() {
        let conn = memory_db();
        replace(&conn, &record("char-1", 1, "avatars/x/1-a.png")).unwrap();
        conn.execute("DELETE FROM characters WHERE character_id='char-1'", [])
            .unwrap();
        let retained: (String, String, String, i64) = conn
            .query_row(
                "SELECT disposition, preserved_relative_path, content_hash, byte_size \
                 FROM character_avatar_unresolved",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(retained.0, "orphan");
        assert_eq!(retained.1, "avatars/x/1-a.png");
        assert_eq!(retained.2, "a".repeat(64));
        assert_eq!(retained.3, 10);
        let candidates: String = conn
            .query_row(
                "SELECT candidate_character_ids_json FROM character_avatar_unresolved",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(candidates, "['char-1']".replace('\'', "\""));
    }

    #[test]
    fn read_many_orders_by_character_id_test() {
        let conn = memory_db();
        conn.execute(
            "INSERT INTO characters(character_id, name) VALUES('char-2', '角色二')",
            [],
        )
        .unwrap();
        replace(&conn, &record("char-2", 1, "avatars/y/1-b.png")).unwrap();
        replace(&conn, &record("char-1", 1, "avatars/x/1-a.png")).unwrap();
        let rows = read_many(&conn, &["char-2".to_string(), "char-1".to_string()]).unwrap();
        assert_eq!(
            rows.iter()
                .map(|record| record.character_id.as_str())
                .collect::<Vec<_>>(),
            vec!["char-1", "char-2"]
        );
        assert!(read_many(&conn, &[]).unwrap().is_empty());
    }
}
