//! Lorekeeper 冻结历史证据：只在脱敏副本上枚举 R/H 行，不携带原始收据或机器字段。
//! 空投影表示不迁出原始收据，也不声称其已验证；行身份与只读历史冻结键仍保留。
//! 文档按已脱敏库确定性生成，恢复时重新枚举比对，拒绝漏冻、额外键与来源元组漂移。

use rusqlite::{types::ValueRef, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::services::portable_project_field_policy::{
    get_portable_field_policy, PortableFieldDisposition,
};
use crate::services::portable_runtime_freeze::{
    parse_portable_runtime_freeze, PORTABLE_RUNTIME_HISTORY_MAX_BYTES,
};

pub const FREEZE_ENTRY: &str = "portable-runtime-freeze.json";

/// 作者内容表：这些表整行**不进入**冻结键集，恢复副本里作者可继续编辑。
///
/// 它们含 R 列（`drafts.source_dependencies` / `character_relationships.provenance_json`），
/// 列值照旧中和（隐私不变），但行级冻结会把「作者正文/角色关系」也变成只读，
/// 恢复归档后无法继续创作——而冻结的用意是「历史执行收据不可重放」，不是「作者内容只读」。
/// 上游基线同样只冻结执行类行（生成/导入/恢复候选/定稿出箱/章节删除等）。
const AUTHOR_CONTENT_TABLES: [&str; 2] = ["drafts", "character_relationships"];

fn invalid() -> String {
    "PORTABLE_UNSAFE_PROJECTION".into()
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// 所有字段策略来自已对账的脱敏库；R/X 内容及其摘要均不进入侧车。
pub fn freeze_document(
    conn: &Connection,
    origin_project_id: &str,
    snapshot_generation: &str,
) -> Result<Value, String> {
    let tables: Vec<String> = conn.prepare(
        "SELECT name FROM pragma_table_list WHERE schema='main' AND type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
    ).map_err(|_| invalid())?.query_map([], |r| r.get(0)).map_err(|_| invalid())?
        .collect::<Result<_, _>>().map_err(|_| invalid())?;
    let mut records = Vec::new();
    for table in tables {
        let columns: Vec<(String, i64)> = conn
            .prepare(&format!("PRAGMA table_info({})", quote(&table)))
            .map_err(|_| invalid())?
            .query_map([], |r| Ok((r.get(1)?, r.get(5)?)))
            .map_err(|_| invalid())?
            .collect::<Result<_, _>>()
            .map_err(|_| invalid())?;
        let mut excluded = Vec::new();
        let mut historical = false;
        for (name, _) in &columns {
            match get_portable_field_policy(&table, name)?.disposition {
                PortableFieldDisposition::RedactedProjection => excluded.push(name.clone()),
                PortableFieldDisposition::HistoricalNonreplayable => historical = true,
                _ => {}
            }
        }
        if excluded.is_empty() && !historical {
            continue;
        }
        if AUTHOR_CONTENT_TABLES.contains(&table.as_str()) {
            continue;
        }
        let mut keys: Vec<_> = columns.iter().filter(|(_, pk)| *pk > 0).collect();
        keys.sort_by_key(|(_, pk)| *pk);
        // 与基线一致：优先稳定行身份，无声明主键时才用 rowid。
        let key_columns = if keys.is_empty() {
            "rowid".into()
        } else {
            keys.iter()
                .map(|(name, _)| quote(name))
                .collect::<Vec<_>>()
                .join(",")
        };
        let state = [
            "status",
            "state",
            "publication_status",
            "manuscript_status",
            "knowledge_status",
            "revision_status",
        ]
        .iter()
        .find(|candidate| columns.iter().any(|(name, _)| name == **candidate))
        .map(|name| quote(name))
        .unwrap_or_else(|| "'historical'".into());
        let sql = format!(
            "SELECT {key_columns}, {state} FROM {} ORDER BY {key_columns}",
            quote(&table)
        );
        let mut stmt = conn.prepare(&sql).map_err(|_| invalid())?;
        let key_count = keys.len().max(1);
        let mut rows = stmt.query([]).map_err(|_| invalid())?;
        while let Some(row) = rows.next().map_err(|_| invalid())? {
            let mut identity = Vec::new();
            for index in 0..key_count {
                identity.push(match row.get_ref(index).map_err(|_| invalid())? {
                    ValueRef::Integer(id) => id.to_string(),
                    ValueRef::Text(id) => {
                        std::str::from_utf8(id).map_err(|_| invalid())?.to_string()
                    }
                    _ => return Err(invalid()),
                });
            }
            let id = identity.join(":");
            let terminal = match row.get_ref(key_count).map_err(|_| invalid())? {
                ValueRef::Text(value) if !value.is_empty() => std::str::from_utf8(value)
                    .map_err(|_| invalid())?
                    .to_string(),
                _ => "historical".into(),
            };
            records.push(json!({
                "projectionId": format!("history:{table}:{id}"),
                "table": table, "recordId": id, "terminalState": terminal,
                "projection": {}, "projectionHash": format!("{:x}", Sha256::digest(b"{}")),
                "excludedFields": excluded, "nonReplayable": true, "originalReceiptVerified": false,
            }));
            if records.len() > 100_000 {
                return Err(invalid());
            }
        }
    }
    let document = json!({
        "version": 1, "originProjectId": origin_project_id,
        "snapshotGeneration": snapshot_generation,
        "nonReplayable": true, "requiresRuntimeFreezeGuard": true,
        "records": records, "avatarReferenceProjections": [],
    });
    let bytes = serde_json::to_vec(&document).map_err(|_| invalid())?;
    if bytes.len() > PORTABLE_RUNTIME_HISTORY_MAX_BYTES {
        return Err(invalid());
    }
    parse_portable_runtime_freeze(&bytes).map_err(|_| invalid())?;
    Ok(document)
}

/// 恢复端必须在发布项目前验证全量冻结键与脱敏历史一致，不能只检验 JSON 形状。
pub fn verify_freeze_document(
    conn: &Connection,
    document: &Value,
    origin: &str,
    generation: &str,
) -> Result<(), String> {
    if &freeze_document(conn, origin, generation)? != document {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_freezes_history_without_exporting_original_receipts() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        crate::services::portable_project_field_policy::drop_rebuildable_tables(&conn).unwrap();
        conn.execute("INSERT INTO chapter_deletion_operations (operation_id, draft_id, chapter_number, finalization_id, target_file_name) VALUES ('op-1', 7, 3, 'fin-1', 'chapter-0003.md')", []).unwrap();
        let origin = "123e4567-e89b-12d3-a456-426614174000";
        let doc = freeze_document(&conn, origin, "gen-1").unwrap();
        let guard = parse_portable_runtime_freeze(&serde_json::to_vec(&doc).unwrap()).unwrap();
        assert!(guard.active());
        assert!(guard.is_frozen(
            crate::services::portable_runtime_freeze::PortableRuntimeFreezeTable::ChapterDeletionOperations, "op-1"
        ));
        assert_eq!(doc["records"][0]["projection"], json!({}));
        assert_eq!(doc["records"][0]["originalReceiptVerified"], false);
        verify_freeze_document(&conn, &doc, origin, "gen-1").unwrap();
        let mut bad = doc.clone();
        bad["records"] = json!([]);
        assert!(verify_freeze_document(&conn, &bad, origin, "gen-1").is_err());
        assert!(verify_freeze_document(&conn, &doc, origin, "other").is_err());
    }
}
