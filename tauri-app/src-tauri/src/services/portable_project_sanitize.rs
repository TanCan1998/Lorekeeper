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
//! - 参与表级 `UNIQUE(...)` 的 R/X 文本列（如 `import_run_receipts` 的
//!   `effect_namespace`/`effect_key`）若一律中和为常量，多行时会撞唯一约束；
//!   本项目改用「`portable-neutral:` + 行身份」的确定性中和值：不含原值、同库可重复生成、逐行唯一。
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

/// 表的唯一索引列组（表级 `UNIQUE(...)`；`PRIMARY KEY` 索引跳过，主键列不中和）。
///
/// 表达式索引的列名为 `NULL`，无法安全派生，直接跳过（本项目 schema 未使用）。
fn unique_index_columns(conn: &Connection, table: &str) -> Result<Vec<Vec<String>>, String> {
    let quoted = format!("'{}'", table.replace('\'', "''"));
    let mut statement = conn
        .prepare(&format!("PRAGMA index_list({quoted})"))
        .map_err(|_| unsafe_projection())?;
    let indexes = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|_| unsafe_projection())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| unsafe_projection())?;
    let mut groups = Vec::new();
    for (name, unique, origin) in indexes {
        if unique == 0 || origin == "pk" {
            continue;
        }
        let quoted_name = format!("'{}'", name.replace('\'', "''"));
        let mut info = conn
            .prepare(&format!("PRAGMA index_info({quoted_name})"))
            .map_err(|_| unsafe_projection())?;
        let columns: Vec<Option<String>> = info
            .query_map([], |row| row.get::<_, Option<String>>(2))
            .map_err(|_| unsafe_projection())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| unsafe_projection())?;
        if columns.iter().any(Option::is_none) {
            continue;
        }
        groups.push(columns.into_iter().flatten().collect());
    }
    Ok(groups)
}

/// 行身份的确定性 SQL 表达式：主键列拼接，无声明主键时退回 `rowid`。
fn row_identity_expression(columns: &[ColumnInfo]) -> String {
    let mut keys: Vec<&ColumnInfo> = columns.iter().filter(|column| column.primary_key).collect();
    keys.sort_by(|left, right| left.name.cmp(&right.name));
    if keys.is_empty() {
        return "CAST(rowid AS TEXT)".to_string();
    }
    keys.iter()
        .map(|column| {
            format!(
                "COALESCE(CAST({} AS TEXT),'')",
                quote_identifier(&column.name)
            )
        })
        .collect::<Vec<_>>()
        .join(" || ':' || ")
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

        // R / X 列：常量中和值只由列决定（与行无关），故每列一条 UPDATE；
        // 但参与 `UNIQUE(...)` 的文本列统一置常量会撞唯一约束（如
        // `import_run_receipts` 的 `effect_namespace`/`effect_key`，多行时必然冲突）。
        // 这些列改用「常量前缀 + 行身份」的确定性中和值：不含原值、同库可重复生成，且逐行唯一。
        let unique_groups = unique_index_columns(&conn, &table)?;
        let identity = row_identity_expression(&columns);
        for (column, _) in &updates {
            let quoted_column = quote_identifier(&column.name);
            // 作者草稿恢复后仍可编辑；依赖中和为合法空集合，避免读面按整库身份
            // 豁免空串而误放过恢复后新增/损坏的当前行。不保留旧依赖或其摘要。
            let neutral = if table == "drafts" && column.name == "source_dependencies" {
                Value::Text("[]".to_string())
            } else {
                neutral_value(column)
            };
            let unique_text_column = matches!(neutral, Value::Text(_))
                && unique_groups
                    .iter()
                    .any(|group| group.iter().any(|name| name == &column.name));
            if unique_text_column {
                let sql = format!(
                    "UPDATE {quoted_table} SET {quoted_column}='portable-neutral:' || {identity}"
                );
                conn.execute_batch(&sql).map_err(|_| unsafe_projection())?;
                continue;
            }
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
    fn sanitized_generation_candidates_skip_frozen_run_before_decoding_budget_test() {
        use crate::repositories::generation_run_repository::GenerationRunRepository;
        use crate::services::{
            portable_project_guard, portable_project_history, portable_runtime_freeze,
        };

        let (base, source, target) = temp_paths("candidate-read");
        seed(&source);
        {
            let conn = Connection::open(&source).unwrap();
            conn.execute_batch(
                "INSERT INTO generation_roots (root_action_id,idempotency_key,action_json,budget_json)
                 VALUES ('root-1','key-1','{}','{}');
                 INSERT INTO generation_runs (run_id,root_action_id,binding_json,status,created_at_ms,open_key)
                 VALUES ('run-1','root-1','{}','completed',0,'open-1');
                 INSERT INTO generation_attempts VALUES
                 ('attempt-1','reservation-1','run-1','root-1','{}','{}','nonce-1');
                 INSERT INTO generation_artifacts (artifact_id,attempt_id,run_id,artifact_json,revision,status)
                 VALUES ('artifact-1','attempt-1','run-1','{}',1,'complete');",
            )
            .unwrap();
        }
        sanitize_portable_database(&SanitizePortableDatabaseInput {
            source_database_path: &source,
            target_database_path: &target,
            limits: None,
        })
        .unwrap();
        let conn = Connection::open(&target).unwrap();
        let repository = GenerationRunRepository::default();
        // 未安装便携守卫时，空绑定仍须作为损坏数据报错。
        assert!(repository.list_candidates(&conn, Some("run-1")).is_err());
        let document = portable_project_history::freeze_document(
            &conn,
            "123e4567-e89b-12d3-a456-426614174000",
            "gen-candidate-read",
        )
        .unwrap();
        let guard = portable_runtime_freeze::parse_portable_runtime_freeze(
            &serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        portable_project_guard::install_guard_for_test(&conn, guard).unwrap();
        // 全局与按运行读面一致：冻结候选不再解析中和后的绑定、预算和收据。
        assert!(repository.list_candidates(&conn, None).unwrap().is_empty());
        assert!(repository
            .list_candidates(&conn, Some("run-1"))
            .unwrap()
            .is_empty());
        assert!(repository
            .list_candidates(&conn, Some("missing-run"))
            .is_err());
        assert_eq!(
            read_column_value(&conn, "generation_runs", "binding_json", 1).unwrap(),
            Some(Value::Text(String::new()))
        );
        // 候选读面不得解冻历史，亦不得影响源库。
        assert!(conn
            .execute("UPDATE generation_artifacts SET status='discarded'", [])
            .unwrap_err()
            .to_string()
            .contains("PORTABLE_RUNTIME_FROZEN"));
        let source_conn = Connection::open(&source).unwrap();
        assert_eq!(
            read_column_value(&source_conn, "generation_runs", "binding_json", 1).unwrap(),
            Some(Value::Text("{}".into()))
        );
        drop(source_conn);
        drop(conn);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn sanitized_import_history_is_readable_but_not_listed_as_resumable_test() {
        use crate::repositories::import_run_repository as imports;
        use crate::services::{
            portable_project_guard, portable_project_history, portable_runtime_freeze,
        };

        fn insert_import(conn: &Connection, id: &str, purpose: &str, status: &str) {
            conn.execute(
                "INSERT INTO import_runs (id,purpose,root_run_id,effect_namespace,
                 source_fingerprint,manifest_fingerprint,locale,stage,status,
                 total_chapters,manifest_chapter_count)
                 VALUES (?1,?2,?1,?1,?3,?3,'zh-CN','prepared',?4,1,1)",
                rusqlite::params![id, purpose, "a".repeat(64), status],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO import_run_chapters (run_id,chapter_number,source_id,source_chapter_number,
                 title,content_fingerprint,content_size,content_snapshot)
                 VALUES (?1,1,'src-1',1,'第1章',?2,9,'正文甲')",
                rusqlite::params![id, crate::import::parsing::sha256_hex("正文甲")],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO import_run_receipts (run_id,schema_version,effect_namespace,effect_key,
                 stage,batch_id,kind,payload_json,payload_hash,state)
                 VALUES (?1,1,?2,?3,'global','done','project-global-facts','{}',?4,'prepared')",
                rusqlite::params![
                    id,
                    format!("import:{purpose}:{id}"),
                    format!("global-facts:{id}"),
                    "c".repeat(64)
                ],
            )
            .unwrap();
        }

        let (base, source, target) = temp_paths("import-read");
        seed(&source);
        let ids: Vec<_> = ["reference", "author-manuscript"]
            .into_iter()
            .flat_map(|purpose| {
                ["ready", "running", "failed", "cancelled"]
                    .into_iter()
                    .map(move |status| (format!("old-{purpose}-{status}"), purpose, status))
            })
            .collect();
        {
            let conn = Connection::open(&source).unwrap();
            for (id, purpose, status) in &ids {
                insert_import(&conn, id, purpose, status);
            }
            assert_eq!(imports::list_resumable(&conn).unwrap().len(), ids.len());
        }
        sanitize_portable_database(&SanitizePortableDatabaseInput {
            source_database_path: &source,
            target_database_path: &target,
            limits: None,
        })
        .unwrap();
        let conn = Connection::open(&target).unwrap();
        let document = portable_project_history::freeze_document(
            &conn,
            "123e4567-e89b-12d3-a456-426614174000",
            "gen-import-read",
        )
        .unwrap();
        let guard = portable_runtime_freeze::parse_portable_runtime_freeze(
            &serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        portable_project_guard::install_guard_for_test(&conn, guard).unwrap();
        // 两种用途、四种候选状态的冻结行均不能再显示为可继续任务。
        assert!(imports::list_resumable(&conn).unwrap().is_empty());
        for (id, _, status) in &ids {
            let history = imports::get(&conn, id).unwrap().unwrap();
            assert_eq!(&history.status, status);
            assert!(history.resumable, "历史字段保留，不改写成当前执行权威");
            assert!(history.effect_namespace.is_empty());
            assert!(history.source_display.is_empty());
            assert!(
                imports::start_or_resume(&conn, id, "test-owner", 100, 30_000)
                    .unwrap_err()
                    .contains("PORTABLE_RUNTIME_FROZEN")
            );
        }
        // 收据的唯一索引列（`UNIQUE(effect_namespace, effect_key)`）改用确定性中和值：
        // 不含原值、同库可重复生成，多行收据不再撞唯一约束。
        let neutral_namespace: String = conn
            .query_row(
                "SELECT effect_namespace FROM import_run_receipts
                 WHERE run_id = 'old-reference-ready' AND stage = 'global'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(neutral_namespace.starts_with("portable-neutral:"));
        assert!(!neutral_namespace.contains("import:"));

        // 冻结历史的章节快照（D 策略）保留原值，历史只读仍可查。
        let chapters = imports::list_chapter_batch(&conn, "old-reference-ready", 0, 10).unwrap();
        assert_eq!(chapters.len(), 1);
        assert_eq!(chapters[0].content, "正文甲");
        // 冻结历史的执行收据不发布投影：不报泛化损坏，也不声称可重放。
        for (id, _, _) in &ids {
            assert!(
                imports::get_effect_receipt(&conn, id, "global", "done")
                    .expect("冻结收据读面不应报损坏")
                    .is_none(),
                "冻结收据不应发布执行投影：{id}"
            );
        }

        // 同一连接上的新身份仍正常列举与执行，不能把整个恢复项目禁用。
        insert_import(&conn, "new-import", "reference", "ready");
        let live = imports::list_resumable(&conn).unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, "new-import");
        let started =
            imports::start_or_resume(&conn, "new-import", "test-owner", 100, 30_000).unwrap();
        assert_eq!(started.run.status, "running");
        assert_eq!(started.run.stage, "knowledge");
        // 未冻结运行的损坏数据继续拒绝，不能用跳过历史掩盖当前数据错误。
        conn.execute(
            "UPDATE import_runs SET completed_batches_json='invalid' WHERE id='new-import'",
            [],
        )
        .unwrap();
        assert!(imports::list_resumable(&conn).is_err());
        // 未冻结运行上的损坏收据仍按原规则拒绝，不用跳过历史掩盖当前数据错误。
        // `new-import` 的收据在种子里就是「载荷与哈希不匹配」的损坏行。
        let error = imports::get_effect_receipt(&conn, "new-import", "global", "done")
            .expect_err("未冻结损坏收据应拒绝");
        assert!(error.contains("receipt 损坏"), "实际：{error}");
        let source_conn = Connection::open(&source).unwrap();
        assert_eq!(
            imports::list_resumable(&source_conn).unwrap().len(),
            ids.len()
        );
        assert!(!imports::get(&source_conn, &ids[0].0)
            .unwrap()
            .unwrap()
            .effect_namespace
            .is_empty());
        drop(source_conn);
        drop(conn);
        std::fs::remove_dir_all(base).unwrap();
    }

    /// 便携副本按 R 策略中和 JSON 列后，读面不能再走 SQL §json_extract§：
    /// 空串会抛 malformed JSON，把恢复副本上的写作读面整条打死。
    #[test]
    fn sanitized_json_columns_are_read_row_wise_and_fail_closed_when_unfrozen_test() {
        use crate::services::{
            portable_project_guard, portable_project_history, portable_runtime_freeze,
        };

        let (base, source, target) = temp_paths("json-read");
        seed(&source);
        {
            let conn = Connection::open(&source).unwrap();
            conn.execute_batch(
                "INSERT INTO generation_roots (root_action_id,idempotency_key,action_json,budget_json)
                 VALUES ('root-1','key-1','{}','{}');
                 INSERT INTO generation_runs (run_id,root_action_id,binding_json,status,created_at_ms,open_key)
                 VALUES ('run-1','root-1','{}','completed',0,'open-1');
                 INSERT INTO generation_attempts VALUES
                 ('attempt-1','reservation-1','run-1','root-1','{}','{}','nonce-1');",
            )
            .unwrap();
        }
        sanitize_portable_database(&SanitizePortableDatabaseInput {
            source_database_path: &source,
            target_database_path: &target,
            limits: None,
        })
        .unwrap();
        let conn = Connection::open(&target).unwrap();
        // 修复前形态：SQL 直接 json_extract 中和列会失败，正是恢复副本读面被打死的原因。
        assert!(conn
            .query_row(
                "SELECT json_extract(binding_json,'$.status') FROM generation_runs",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .is_err());

        let document = portable_project_history::freeze_document(
            &conn,
            "123e4567-e89b-12d3-a456-426614174000",
            "gen-json-read",
        )
        .unwrap();
        // 对照：没有认证冻结键时，中和列仍按损坏拒绝（fail closed）。
        let empty = serde_json::json!({
            "version": 1,
            "originProjectId": "123e4567-e89b-12d3-a456-426614174000",
            "snapshotGeneration": "gen-json-empty",
            "nonReplayable": true,
            "requiresRuntimeFreezeGuard": true,
            "records": [],
            "avatarReferenceProjections": [],
        });
        let unguarded = portable_runtime_freeze::parse_portable_runtime_freeze(
            &serde_json::to_vec(&empty).unwrap(),
        )
        .unwrap();
        // 守卫按连接安装（TEMP 触发器），对照在独立连接上做。
        let unguarded_conn = Connection::open(&target).unwrap();
        portable_project_guard::install_guard_for_test(&unguarded_conn, unguarded).unwrap();
        assert!(portable_project_guard::frozen_aware_json_pointer_rows(
            &unguarded_conn,
            "generation_attempts",
            "attempt_id",
            "attempt_json",
            "/status",
            &["reserved", "dispatch-marked"],
        )
        .is_err());
        drop(unguarded_conn);

        // 认证冻结后：历史行被跳过，读面不再报错，也不把历史当在途/当前归属。
        let guard = portable_runtime_freeze::parse_portable_runtime_freeze(
            &serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        assert!(guard.is_frozen_record("generation_attempts", "attempt-1"));
        assert!(guard.is_frozen_record("generation_runs", "run-1"));
        portable_project_guard::install_guard_for_test(&conn, guard).unwrap();
        assert_eq!(
            portable_project_guard::frozen_aware_json_pointer_rows(
                &conn,
                "generation_attempts",
                "attempt_id",
                "attempt_json",
                "/status",
                &["reserved", "dispatch-marked"],
            )
            .unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            portable_project_guard::frozen_aware_json_pointer_rows(
                &conn,
                "generation_runs",
                "run_id",
                "binding_json",
                "/sourceManifest/agentSession/key",
                &["session-1"],
            )
            .unwrap(),
            Vec::<String>::new()
        );

        // 同一连接上的新身份仍可被读到，过滤只作用于认证冻结行。
        // 新 attempt 必须挂在新根/新运行上：借冻结父账本续写由守卫拒绝。
        conn.execute_batch(
            "INSERT INTO generation_roots (root_action_id,idempotency_key,action_json,budget_json)
             VALUES ('root-2','key-2','{}','{}');
             INSERT INTO generation_runs (run_id,root_action_id,binding_json,status,created_at_ms,open_key)
             VALUES ('run-2','root-2','{}','running',0,'open-2');
             INSERT INTO generation_attempts VALUES
             ('attempt-2','reservation-2','run-2','root-2','{\"status\":\"reserved\"}','{}','nonce-2');",
        )
        .unwrap();
        assert_eq!(
            portable_project_guard::frozen_aware_json_pointer_rows(
                &conn,
                "generation_attempts",
                "attempt_id",
                "attempt_json",
                "/status",
                &["reserved", "dispatch-marked"],
            )
            .unwrap(),
            vec!["attempt-2".to_string()]
        );
        // 非冻结行的坏 JSON 仍报错，不用跳过历史掩盖当前数据问题。
        conn.execute(
            "UPDATE generation_attempts SET attempt_json='' WHERE attempt_id='attempt-2'",
            [],
        )
        .unwrap();
        assert!(portable_project_guard::frozen_aware_json_pointer_rows(
            &conn,
            "generation_attempts",
            "attempt_id",
            "attempt_json",
            "/status",
            &["reserved", "dispatch-marked"],
        )
        .is_err());

        drop(conn);
        let source_conn = Connection::open(&source).unwrap();
        assert_eq!(
            read_column_value(&source_conn, "generation_runs", "binding_json", 1).unwrap(),
            Some(Value::Text("{}".into()))
        );
        drop(source_conn);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn sanitized_recovery_candidate_keeps_visible_integrity_without_replaying_source_test() {
        use crate::repositories::{
            blueprint_repository as blueprints, recovery_candidate_repository as recovery,
        };
        use crate::services::{
            portable_project_guard, portable_project_history, portable_runtime_freeze,
        };

        for corrupt_visible_text in [false, true] {
            let (base, source, target) = temp_paths("recovery-read");
            seed(&source);
            let id;
            {
                let conn = Connection::open(&source).unwrap();
                let blueprint = serde_json::from_value(serde_json::json!({
                    "chapterNumber": 1, "title": "合成章"
                }))
                .unwrap();
                blueprints::upsert(&conn, &blueprint).unwrap();
                let request = serde_json::from_value(serde_json::json!({
                    "runId": "recovery-run", "stepId": "recovery-step", "chapterNumber": 1,
                    "source": {"chapterNumber": 1, "title": "合成章"},
                    "visibleText": "合成可见片段", "failureCode": "TIMEOUT"
                }))
                .unwrap();
                let candidate = recovery::record(&conn, "source-project", &request).unwrap();
                assert!(candidate.source_current);
                id = candidate.candidate_id;
            }
            sanitize_portable_database(&SanitizePortableDatabaseInput {
                source_database_path: &source,
                target_database_path: &target,
                limits: None,
            })
            .unwrap();
            let conn = Connection::open(&target).unwrap();
            // 中和的源快照只有在已验证的便携冻结上下文中才能只读展示。
            assert!(recovery::list_pending(&conn).is_err());
            if corrupt_visible_text {
                conn.execute(
                    "UPDATE recovery_candidates SET visible_text='已篡改片段'",
                    [],
                )
                .unwrap();
            }
            let document = portable_project_history::freeze_document(
                &conn,
                "123e4567-e89b-12d3-a456-426614174000",
                "gen-recovery-read",
            )
            .unwrap();
            let guard = portable_runtime_freeze::parse_portable_runtime_freeze(
                &serde_json::to_vec(&document).unwrap(),
            )
            .unwrap();
            portable_project_guard::install_guard_for_test(&conn, guard).unwrap();
            if corrupt_visible_text {
                assert_eq!(
                    recovery::list_pending(&conn).unwrap_err(),
                    "恢复候选完整性校验失败"
                );
            } else {
                let history = recovery::list_pending(&conn).unwrap();
                assert_eq!(history.len(), 1);
                assert_eq!(history[0].candidate_id, id);
                assert_eq!(history[0].visible_text, "合成可见片段");
                assert_eq!(
                    history[0].content_hash,
                    recovery::sha256_hex("合成可见片段")
                );
                assert!(history[0].source_hash.is_empty());
                assert!(!history[0].source_current);
                assert!(conn
                    .execute("UPDATE recovery_candidates SET status='continued'", [])
                    .unwrap_err()
                    .to_string()
                    .contains("PORTABLE_RUNTIME_FROZEN"));
            }
            if !corrupt_visible_text {
                let request = serde_json::from_value(serde_json::json!({
                    "runId": "fresh-recovery-run", "stepId": "fresh-step", "chapterNumber": 1,
                    "source": {"chapterNumber": 1, "title": "合成章"},
                    "visibleText": "合成新候选片段"
                }))
                .unwrap();
                let fresh = recovery::record(&conn, "restored-project", &request).unwrap();
                assert!(fresh.source_current);
                let mixed = recovery::list_pending(&conn).unwrap();
                assert_eq!(mixed.len(), 2);
                assert!(!mixed[0].source_current);
                assert!(
                    mixed[1].source_current,
                    "不能因同项目存在冻结历史而禁用新候选"
                );
                recovery::update_pending(&conn, &fresh.candidate_id, "合成更新片段").unwrap();
                recovery::resolve(&conn, &fresh.candidate_id, recovery::STATUS_CONTINUED).unwrap();
                let remaining = recovery::list_pending(&conn).unwrap();
                assert_eq!(remaining.len(), 1);
                assert_eq!(remaining[0].candidate_id, id);
            }
            let source_conn = Connection::open(&source).unwrap();
            let current = recovery::list_pending(&source_conn).unwrap();
            assert!(current[0].source_current);
            assert!(!current[0].source_hash.is_empty());
            assert_eq!(current[0].visible_text, "合成可见片段");
            drop(source_conn);
            drop(conn);
            std::fs::remove_dir_all(base).unwrap();
        }
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
