//! 便携脱敏副本 —— 平移自 `electron/services/project-archive-service.ts` 的 `sanitizePortableDatabase`，
//! 按**本项目口径**（2026-10-16 用户确认「不与基线互通、按本项目方案存储恢复」）落地。
//!
//! # 流程
//!
//! 1. `VACUUM INTO` 把活跃库复制成便携副本（不复制 WAL/日志，且是 SQLite 官方的一致性快照）；
//! 2. 在副本上删除**派生索引表**（[`crate::services::portable_project_field_policy::PORTABLE_REBUILDABLE_TABLES`]，
//!    即 FTS5 虚拟表 `kb_fts` 与其影子表）；
//! 3. 逐表逐列按策略处置：
//!    - `redacted-projection`（R）：值必须中和，不入档（收据/投影类字段）；
//!    - `exclude-machine-authority`（X）：值中和，且**不留摘要**（策略原文即 `exclude-original-and-digest`）；
//!    - `validated-relative`（P）：值必须能通过便携路径校验（空串视为未赋值，放行）；
//!    - `historical-nonreplayable`（H）/`preserve-domain`（D）/`avatar-asset`（A）/`rebuild-stale`（B）：
//!      原值保留（历史证据与项目事实随库入档）；
//! 4. 记录逐表行数（`semanticCounts` 的 `table.<name>`）与被中和的列清单（供 `omittedItems` 记账）。
//!
//! # 与基线的差异（本项目口径，无互通需求）
//!
//! - 基线对 R 列有逐列定制的「投影器」（把 JSON 收据裁剪成安全子集）并把逐行历史投影写成独立归档文件；
//!   本项目 R 列一律中和、列名进 `omittedItems`（`historical-nonreplayable`），
//!   H 列仍在库中保留；导出另由 `portable_project_history` 生成空投影冻结侧车，
//!   标记历史不可重放，不携带原始收据或机器字段摘要。
//! - 基线的 P 列校验对空串也报错；本项目把空串视为未赋值直接放行（`target_file_name` 等列允许空默认值），
//!   非空值仍严格校验；
//! - 基线用 `better-sqlite3` 备份 API，本项目用 `VACUUM INTO`（同一语义、零新依赖）。

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::types::Value;
use rusqlite::{params, Connection, OptionalExtension};

use crate::services::portable_project_field_policy::{
    assert_portable_source_schema, drop_rebuildable_tables, get_portable_field_policy,
    PortableFieldDisposition, PortableSchemaCounts,
};
use crate::services::portable_project_format::{portable_path_key, PortableArchiveLimits};

/// 上游 `sanitizePortableDatabase` 的输入
#[derive(Debug, Clone)]
pub struct SanitizePortableDatabaseInput<'a> {
    /// 活跃库（`.lore/lorekeeper.db`）
    pub source_database_path: &'a Path,
    /// 便携副本目标路径（不得已存在）
    pub target_database_path: &'a Path,
    pub limits: Option<PortableArchiveLimits>,
}

/// 被中和的列（供 `omittedItems` 记账）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeutralizedColumn {
    pub table: String,
    pub column: String,
    pub disposition: PortableFieldDisposition,
}

/// 上游 `sanitizePortableDatabase` 的返回
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizePortableDatabaseResult {
    /// `semanticCounts` 的 `table.<name>` 逐表行数
    pub semantic_counts: BTreeMap<String, u64>,
    pub neutralized_columns: Vec<NeutralizedColumn>,
    /// 被删除的派生索引表（`rebuild-stale`）
    pub dropped_tables: Vec<String>,
    pub schema: PortableSchemaCounts,
}

fn unsafe_projection() -> String {
    "PORTABLE_UNSAFE_PROJECTION".to_string()
}

/// `VACUUM INTO` 复制：源必须存在，目标不得已存在
pub fn copy_to_portable_database(source: &Path, target: &Path) -> Result<(), String> {
    if std::fs::symlink_metadata(source).is_err() {
        return Err("PORTABLE_ASSET_MISSING".to_string());
    }
    if std::fs::symlink_metadata(target).is_ok() {
        return Err("PORTABLE_RESTORE_TARGET_EXISTS".to_string());
    }
    let connection = Connection::open_with_flags(
        source,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| "PORTABLE_ASSET_MISSING".to_string())?;
    let target_text = target.to_string_lossy().to_string();
    connection
        .execute("VACUUM INTO ?1", params![target_text])
        .map_err(|_| unsafe_projection())?;
    Ok(())
}

/// 列定义（`PRAGMA table_info` 的投影）
#[derive(Debug, Clone, PartialEq, Eq)]
struct ColumnInfo {
    name: String,
    type_name: String,
    not_null: bool,
    /// `PRAGMA table_info` 的 `pk` 序号 > 0（复合主键时为出现次序）
    primary_key: bool,
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('\"', "\"\""))
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<ColumnInfo>, String> {
    let quoted = format!("'{}'", table.replace('\'', "''"));
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info({quoted})"))
        .map_err(|_| unsafe_projection())?;
    let rows = statement
        .query_map([], |row| {
            Ok(ColumnInfo {
                name: row.get::<_, String>(1)?,
                type_name: row.get::<_, String>(2)?,
                not_null: row.get::<_, i64>(3)? != 0,
                primary_key: row.get::<_, i64>(5)? != 0,
            })
        })
        .map_err(|_| unsafe_projection())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| unsafe_projection())
}

fn table_names(conn: &Connection) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare(
            "SELECT name FROM pragma_table_list WHERE schema='main' AND type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .map_err(|_| unsafe_projection())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| unsafe_projection())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| unsafe_projection())
}

/// 中和值（对齐基线 `neutralValue` 的兜底分支）：可空 → `NULL`；数值型 → `0`；其余 → `''`
fn neutral_value(column: &ColumnInfo) -> Value {
    if !column.not_null {
        return Value::Null;
    }
    let upper = column.type_name.to_uppercase();
    if ["INT", "REAL", "NUM", "DEC", "BOOL"]
        .iter()
        .any(|needle| upper.contains(needle))
    {
        Value::Integer(0)
    } else {
        Value::Text(String::new())
    }
}

/// 便携相对路径校验（上游 `portablePathKey`）：空串视为未赋值放行
fn validate_relative_value(value: &str, max_path_bytes: usize) -> Result<(), String> {
    if value.is_empty() {
        return Ok(());
    }
    portable_path_key(value, max_path_bytes)
        .map(|_| ())
        .map_err(|_| unsafe_projection())
}

/// 上游 `sanitizePortableDatabase`（本项目口径）
pub fn sanitize_portable_database(
    input: &SanitizePortableDatabaseInput<'_>,
) -> Result<SanitizePortableDatabaseResult, String> {
    let limits = input.limits.unwrap_or_default();
    copy_to_portable_database(input.source_database_path, input.target_database_path)?;
    let conn = Connection::open(input.target_database_path).map_err(|_| unsafe_projection())?;
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(|_| unsafe_projection())?;

    // 派生索引表先删（虚拟表/影子表在结构对账中会被拒绝）
    let dropped_tables = drop_rebuildable_tables(&conn)?;
    assert_portable_source_schema(&conn)?;

    let mut semantic_counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut neutralized_columns: Vec<NeutralizedColumn> = Vec::new();

    for table in table_names(&conn)? {
        let columns = table_columns(&conn, &table)?;
        let mut updates: Vec<(&ColumnInfo, PortableFieldDisposition)> = Vec::new();
        let mut validations: Vec<&ColumnInfo> = Vec::new();
        for column in &columns {
            let policy = get_portable_field_policy(&table, &column.name)?;
            // 主键列**不中和**：`INTEGER PRIMARY KEY` 是 rowid 别名，置 NULL 会被 SQLite 拒绝
            // （`datatype mismatch`），且主键是行身份与外键引用目标，中和会让副本自相矛盾。
            // 主键不携带收据/机器权威语义，保留即可（本项目口径，见模块头注）。
            if column.primary_key {
                continue;
            }
            match policy.disposition {
                PortableFieldDisposition::RedactedProjection
                | PortableFieldDisposition::ExcludeMachineAuthority => {
                    updates.push((column, policy.disposition));
                    neutralized_columns.push(NeutralizedColumn {
                        table: table.clone(),
                        column: column.name.clone(),
                        disposition: policy.disposition,
                    });
                }
                PortableFieldDisposition::ValidatedRelative => validations.push(column),
                _ => {}
            }
        }

        let quoted_table = quote_identifier(&table);
        let row_count: u64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {quoted_table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|_| unsafe_projection())? as u64;
        semantic_counts.insert(format!("table.{table}"), row_count);

        // P 列：逐行校验（空串放行）。
        if !validations.is_empty() && row_count > 0 {
            let select_list = validations
                .iter()
                .map(|column| quote_identifier(&column.name))
                .collect::<Vec<_>>()
                .join(",");
            let mut statement = conn
                .prepare(&format!("SELECT {select_list} FROM {quoted_table}"))
                .map_err(|_| unsafe_projection())?;
            let rows = statement
                .query_map([], |row| {
                    let mut values: Vec<Value> = Vec::with_capacity(validations.len());
                    for index in 0..validations.len() {
                        values.push(row.get(index)?);
                    }
                    Ok(values)
                })
                .map_err(|_| unsafe_projection())?;
            for row in rows {
                let values = row.map_err(|_| unsafe_projection())?;
                for value in values {
                    let text = match value {
                        Value::Text(text) => text,
                        Value::Null => String::new(),
                        _ => return Err(unsafe_projection()),
                    };
                    validate_relative_value(&text, limits.max_path_bytes)?;
                }
            }
        }

        // R / X 列：中和值只由列决定（与行无关），故每列一条 UPDATE。
        for (column, _) in &updates {
            let quoted_column = quote_identifier(&column.name);
            let neutral = neutral_value(column);
            let sql = match &neutral {
                Value::Null => format!("UPDATE {quoted_table} SET {quoted_column}=NULL"),
                other => {
                    let literal = match other {
                        Value::Integer(number) => number.to_string(),
                        Value::Text(text) => format!("'{}'", text.replace('\'', "''")),
                        _ => return Err(unsafe_projection()),
                    };
                    format!("UPDATE {quoted_table} SET {quoted_column}={literal}")
                }
            };
            conn.execute_batch(&sql).map_err(|_| unsafe_projection())?;
        }
    }

    let schema = assert_portable_source_schema(&conn)?;
    Ok(SanitizePortableDatabaseResult {
        semantic_counts,
        neutralized_columns,
        dropped_tables,
        schema,
    })
}

/// 读取单列单行（测试与诊断用）
pub fn read_column_value(
    conn: &Connection,
    table: &str,
    column: &str,
    rowid: i64,
) -> Result<Option<Value>, String> {
    conn.query_row(
        &format!(
            "SELECT {} FROM {} WHERE rowid=?1",
            quote_identifier(column),
            quote_identifier(table)
        ),
        [rowid],
        |row| row.get::<_, Value>(0),
    )
    .optional()
    .map_err(|_| unsafe_projection())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;

    fn temp_paths(name: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "lorekeeper-sanitize-{name}-{}",
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let source = base.join("lorekeeper.db");
        let target = base.join("portable.db");
        (base, source, target)
    }

    fn seed(path: &Path) {
        let conn = Connection::open(path).unwrap();
        create_tables(&conn).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        // R 列（llm_calls 全部为 R）：中和
        conn.execute(
            "INSERT INTO llm_calls (model_id, model_name, purpose, prompt_tokens, error_message)
             VALUES ('model-1', '名称', 'draft', 11, 'boom')",
            [],
        )
        .unwrap();
        // X 列（旧库身份桥）：中和且不留摘要
        conn.execute(
            "INSERT INTO import_legacy_identity_bridge (id, ciphertext_hex, iv_hex, auth_tag_hex)
             VALUES ('bridge-1', 'deadbeef', 'cafe', 'babe')",
            [],
        )
        .unwrap();
        // P 列（target_file_name）合法值放行
        conn.execute(
            "INSERT INTO chapter_deletion_operations (operation_id, draft_id, chapter_number, finalization_id, target_file_name)
             VALUES ('op-1', 7, 3, 'fin-1', 'chapter-0003.md')",
            [],
        )
        .unwrap();
        // D 列（知识事实）：原样保留
        conn.execute(
            "INSERT INTO kb_documents (id, file_name, file_path, corpus_kind, imported_at, chunk_count, content_hash)
             VALUES ('doc-1', '参考.md', '', 'reference', '2026-10-16T00:00:00.000Z', 1, 'hash')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO kb_chunks (id, doc_id, file_name, text, chunk_index, total_chunks, imported_at, corpus_kind)
             VALUES ('chunk-1', 'doc-1', '参考.md', '正文', 0, 1, '2026-10-16T00:00:00.000Z', 'reference')",
            [],
        )
        .unwrap();
        // 派生索引：FTS5 虚拟表（应被删除）
        conn.execute_batch(
            "INSERT INTO kb_fts (chunk_id, doc_id, file_name, corpus_kind, tokens)
             VALUES ('chunk-1', 'doc-1', '参考.md', 'reference', '正文')",
        )
        .unwrap();
    }

    fn has_table(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT 1 FROM pragma_table_list WHERE schema='main' AND name=?1",
            [name],
            |_| Ok(()),
        )
        .optional()
        .unwrap()
        .is_some()
    }

    #[test]
    fn sanitize_copies_neutralizes_and_drops_index_test() {
        let (base, source, target) = temp_paths("basic");
        seed(&source);
        let result = sanitize_portable_database(&SanitizePortableDatabaseInput {
            source_database_path: &source,
            target_database_path: &target,
            limits: None,
        })
        .unwrap();

        assert_eq!(result.dropped_tables, vec!["kb_fts".to_string()]);
        assert_eq!(result.semantic_counts.get("table.kb_documents"), Some(&1));
        assert_eq!(result.semantic_counts.get("table.kb_chunks"), Some(&1));
        assert!(result
            .neutralized_columns
            .iter()
            .any(|entry| entry.table == "llm_calls" && entry.column == "model_id"));
        assert!(result.neutralized_columns.iter().any(|entry| {
            entry.table == "import_legacy_identity_bridge"
                && entry.column == "ciphertext_hex"
                && entry.disposition == PortableFieldDisposition::ExcludeMachineAuthority
        }));

        let conn = Connection::open(&target).unwrap();
        assert!(!has_table(&conn, "kb_fts"), "派生 FTS 索引不入档");
        assert!(has_table(&conn, "kb_documents"), "知识事实随库入档");
        // R 列中和：可空列 → NULL，NOT NULL 列 → ''
        assert_eq!(
            read_column_value(&conn, "llm_calls", "model_name", 1).unwrap(),
            Some(Value::Null)
        );
        assert_eq!(
            read_column_value(&conn, "llm_calls", "model_id", 1).unwrap(),
            Some(Value::Text(String::new()))
        );
        assert_eq!(
            read_column_value(&conn, "llm_calls", "prompt_tokens", 1).unwrap(),
            Some(Value::Null)
        );
        // X 列中和
        assert_eq!(
            read_column_value(&conn, "import_legacy_identity_bridge", "ciphertext_hex", 1).unwrap(),
            Some(Value::Text(String::new()))
        );
        // P 列原值保留
        assert_eq!(
            read_column_value(&conn, "chapter_deletion_operations", "target_file_name", 1).unwrap(),
            Some(Value::Text("chapter-0003.md".to_string()))
        );
        // D 列原值保留
        assert_eq!(
            read_column_value(&conn, "kb_chunks", "text", 1).unwrap(),
            Some(Value::Text("正文".to_string()))
        );
        // 源库未被修改
        let source_conn = Connection::open(&source).unwrap();
        assert_eq!(
            read_column_value(&source_conn, "llm_calls", "model_id", 1).unwrap(),
            Some(Value::Text("model-1".to_string()))
        );
        assert!(has_table(&source_conn, "kb_fts"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn sanitize_rejects_unsafe_relative_value_and_existing_target_test() {
        let (base, source, target) = temp_paths("reject");
        seed(&source);
        {
            let conn = Connection::open(&source).unwrap();
            conn.execute(
                "INSERT INTO chapter_deletion_operations (operation_id, draft_id, chapter_number, finalization_id, target_file_name)
                 VALUES ('op-2', 8, 4, 'fin-2', 'a/../escape.md')",
                [],
            )
            .unwrap();
        }
        assert_eq!(
            sanitize_portable_database(&SanitizePortableDatabaseInput {
                source_database_path: &source,
                target_database_path: &target,
                limits: None,
            })
            .unwrap_err(),
            "PORTABLE_UNSAFE_PROJECTION"
        );
        // 目标已存在 → 拒绝
        std::fs::write(&target, b"occupied").unwrap();
        assert_eq!(
            sanitize_portable_database(&SanitizePortableDatabaseInput {
                source_database_path: &source,
                target_database_path: &target,
                limits: None,
            })
            .unwrap_err(),
            "PORTABLE_RESTORE_TARGET_EXISTS"
        );
        // 源缺失 → ASSET_MISSING
        assert_eq!(
            sanitize_portable_database(&SanitizePortableDatabaseInput {
                source_database_path: &base.join("missing.db"),
                target_database_path: &base.join("out.db"),
                limits: None,
            })
            .unwrap_err(),
            "PORTABLE_ASSET_MISSING"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn relative_validation_allows_empty_and_rejects_unsafe_test() {
        assert!(validate_relative_value("", 1024).is_ok());
        assert!(validate_relative_value("chapter-0001.md", 1024).is_ok());
        assert!(validate_relative_value("a/../b.md", 1024).is_err());
        assert!(validate_relative_value("/abs.md", 1024).is_err());
    }
}
