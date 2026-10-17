//! 便携恢复项目的连接级写入门禁。
//! 守卫函数与 TEMP 触发器只活到当前连接关闭，不修改持久 schema。
//! 保护冻结行的 UPDATE/DELETE/同键 INSERT（含 REPLACE/UPSERT），并拒绝借冻结父账本续写。
//! 新身份允许创建；冻结侧车与转移权威未通过准入验证时，连接不得暴露给仓储。

use std::path::Path;

use rusqlite::{functions::FunctionFlags, Connection};

use crate::services::portable_runtime_freeze::PortableRuntimeFreezeGuard;

fn invalid() -> String {
    "PORTABLE_RUNTIME_FREEZE_INVALID".into()
}
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// 仅读取连接上已安装的冻结事实，供历史读面统一过滤，不授予写入权威。
/// 普通项目没有该函数，查询失败时不视为冻结，后续解析仍按原规则拒绝损坏数据。
pub(crate) fn frozen_row(conn: &Connection, table: &str, id: &str) -> bool {
    conn.query_row(
        "SELECT _lorekeeper_history_frozen(?1, ?2)",
        rusqlite::params![table, id],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value != 0)
    .unwrap_or(false)
}

/// 由项目连接打开入口调用。普通项目不装触发器；恢复项目先验证目标身份及转移历史。
pub fn admit_connection(conn: &Connection, project_root: &Path) -> Result<(), String> {
    let root = project_root.to_string_lossy();
    let guard =
        crate::services::portable_runtime_freeze::read_portable_runtime_freeze(Some(&root), None)
            .map_err(|_| invalid())?;
    if !guard.active() {
        return Ok(());
    }
    let storage = project_root.join(crate::db::PROJECT_DIR_NAME);
    let bytes = crate::services::portable_runtime_freeze::regular_file(
        &storage.join("project.json").to_string_lossy(),
        4 * 1024 * 1024,
        None,
    )
    .map_err(|_| invalid())?
    .ok_or_else(invalid)?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if !crate::project_access::is_project_manifest(&manifest) {
        return Err(invalid());
    }
    let project_id = manifest["projectId"].as_str().ok_or_else(invalid)?;
    crate::services::portable_current_authority::read_portable_current_authority(
        &crate::services::portable_current_authority::ReadPortableCurrentAuthorityInput {
            database: conn,
            project_storage_root: &storage.to_string_lossy(),
            project_id,
            test_hooks: None,
        },
    )
    .map_err(|_| invalid())?
    .ok_or_else(invalid)?;
    install_connection_guard(conn, guard)
}

/// 表名与主键均从已初始化的本地 schema 取出，所有标识符逐段引用。
fn install_connection_guard(
    conn: &Connection,
    guard: PortableRuntimeFreezeGuard,
) -> Result<(), String> {
    // REPLACE 可能先按另一唯一键删除旧行；必须让隐式删除也触发冻结检查。
    conn.execute_batch("PRAGMA recursive_triggers = ON")
        .map_err(|_| invalid())?;
    conn.create_scalar_function(
        "_lorekeeper_history_frozen",
        2,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        move |ctx| {
            let table: String = ctx.get(0)?;
            let id: String = ctx.get(1)?;
            Ok(guard.is_frozen_record(&table, &id))
        },
    )
    .map_err(|_| invalid())?;
    let tables: Vec<String> = conn.prepare(
        "SELECT name FROM pragma_table_list WHERE schema='main' AND type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
    ).map_err(|_| invalid())?.query_map([], |r| r.get(0)).map_err(|_| invalid())?
        .collect::<Result<_,_>>().map_err(|_| invalid())?;
    // 每个连接单次安装，失败则回滚临时 DDL，调用者丢弃未准入连接。
    let tx = conn.unchecked_transaction().map_err(|_| invalid())?;
    for table in tables {
        let columns: Vec<(String, i64)> = tx
            .prepare(&format!("PRAGMA main.table_info({})", quote(&table)))
            .map_err(|_| invalid())?
            .query_map([], |r| Ok((r.get(1)?, r.get(5)?)))
            .map_err(|_| invalid())?
            .collect::<Result<_, _>>()
            .map_err(|_| invalid())?;
        let mut keys: Vec<_> = columns.iter().filter(|(_, pk)| *pk > 0).collect();
        keys.sort_by_key(|(_, pk)| *pk);
        let identity = |prefix: &str| {
            if keys.is_empty() {
                format!("CAST({prefix}.rowid AS TEXT)")
            } else {
                keys.iter()
                    .map(|(key, _)| format!("COALESCE(CAST({prefix}.{} AS TEXT),'')", quote(key)))
                    .collect::<Vec<_>>()
                    .join(" || ':' || ")
            }
        };
        let frozen = |prefix: &str| {
            format!(
                "_lorekeeper_history_frozen({}, {})",
                literal(&table),
                identity(prefix)
            )
        };
        let mut parents = Vec::new();
        // 这些列代表执行所有权，不是可重新使用的定稿事实引用。
        let run_table = if table.starts_with("import_") {
            "import_runs"
        } else {
            "generation_runs"
        };
        for (column, parent) in [
            ("root_action_id", "generation_roots"),
            ("run_id", run_table),
            ("attempt_id", "generation_attempts"),
        ] {
            if columns.iter().any(|(name, _)| name == column) {
                parents.push(format!(
                    "_lorekeeper_history_frozen({}, COALESCE(CAST(NEW.{} AS TEXT),''))",
                    literal(parent),
                    quote(column)
                ));
            }
        }
        for (operation, predicate) in [
            (
                "INSERT",
                std::iter::once(frozen("NEW"))
                    .chain(parents.iter().cloned())
                    .collect::<Vec<_>>()
                    .join(" OR "),
            ),
            (
                "UPDATE",
                std::iter::once(format!("{} OR {}", frozen("OLD"), frozen("NEW")))
                    .chain(parents.iter().cloned())
                    .collect::<Vec<_>>()
                    .join(" OR "),
            ),
            ("DELETE", frozen("OLD")),
        ] {
            let trigger = quote(&format!("lorekeeper_freeze_{table}_{operation}"));
            tx.execute_batch(&format!(
                "CREATE TEMP TRIGGER {trigger} BEFORE {operation} ON main.{} WHEN {predicate}
                 BEGIN SELECT RAISE(ABORT, 'PORTABLE_RUNTIME_FROZEN'); END;",
                quote(&table)
            ))
            .map_err(|_| invalid())?;
        }
    }
    tx.commit().map_err(|_| invalid())?;
    Ok(())
}

/// 测试专用：复用正式触发器实现；不绕过生产连接的权威准入。
#[cfg(test)]
pub(crate) fn install_guard_for_test(
    conn: &Connection,
    guard: PortableRuntimeFreezeGuard,
) -> Result<(), String> {
    install_connection_guard(conn, guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn guarded() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_tables(&conn).unwrap();
        conn.execute("INSERT INTO generation_roots (root_action_id,idempotency_key,action_json,budget_json) VALUES ('old-root','old-key','{}','{}')",[]).unwrap();
        conn.execute("INSERT INTO generation_runs (run_id,root_action_id,binding_json,status,created_at_ms,open_key) VALUES ('old-run','old-root','{}','running',0,'old-open')",[]).unwrap();
        let doc = crate::services::portable_project_history::freeze_document(
            &conn,
            "123e4567-e89b-12d3-a456-426614174000",
            "gen-1",
        )
        .unwrap();
        let guard = crate::services::portable_runtime_freeze::parse_portable_runtime_freeze(
            &serde_json::to_vec(&doc).unwrap(),
        )
        .unwrap();
        install_connection_guard(&conn, guard).unwrap();
        conn
    }
    fn blocked(conn: &Connection, sql: &str) {
        let err = conn.execute_batch(sql).unwrap_err().to_string();
        assert!(err.contains("PORTABLE_RUNTIME_FROZEN"), "{err}");
    }
    #[test]
    fn protects_old_rows_replacement_and_parent_replay_without_blocking_new_work() {
        let conn = guarded();
        blocked(
            &conn,
            "UPDATE generation_roots SET revision=1 WHERE root_action_id='old-root'",
        );
        blocked(&conn, "DELETE FROM generation_runs WHERE run_id='old-run'");
        blocked(&conn,"INSERT OR REPLACE INTO generation_roots (root_action_id,idempotency_key,action_json,budget_json) VALUES ('old-root','old-key','{}','{}')");
        // 新主键撞上旧幂等键，SQLite 的隐式 DELETE 也不得绕过守卫。
        blocked(&conn,"INSERT OR REPLACE INTO generation_roots (root_action_id,idempotency_key,action_json,budget_json) VALUES ('other-root','old-key','{}','{}')");
        blocked(&conn,"INSERT INTO generation_runs (run_id,root_action_id,binding_json,status,created_at_ms,open_key) VALUES ('new-run','old-root','{}','running',0,'new-open')");
        conn.execute("INSERT INTO generation_roots (root_action_id,idempotency_key,action_json,budget_json) VALUES ('new-root','new-key','{}','{}')",[]).unwrap();
        conn.execute("INSERT INTO generation_runs (run_id,root_action_id,binding_json,status,created_at_ms,open_key) VALUES ('new-run','new-root','{}','running',0,'new-open')",[]).unwrap();
        conn.execute(
            "UPDATE generation_roots SET revision=1 WHERE root_action_id='new-root'",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM generation_runs WHERE run_id='new-run'", [])
            .unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM main.sqlite_schema WHERE type='trigger' AND name LIKE 'lorekeeper_freeze_%'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert!(conn.query_row("SELECT COUNT(*) FROM temp.sqlite_schema WHERE type='trigger' AND name LIKE 'lorekeeper_freeze_%'",[],|r|r.get::<_,i64>(0)).unwrap()>0);
    }
    #[test]
    fn transaction_rollback_and_upsert_do_not_bypass_guard() {
        let conn = guarded();
        blocked(&conn,"INSERT INTO generation_roots (root_action_id,idempotency_key,action_json,budget_json) VALUES ('old-root','old-key','{}','{}') ON CONFLICT(root_action_id) DO UPDATE SET revision=9");
        conn.execute_batch("BEGIN; INSERT INTO generation_roots (root_action_id,idempotency_key,action_json,budget_json) VALUES ('fresh','fresh','{}','{}')").unwrap();
        blocked(
            &conn,
            "DELETE FROM generation_roots WHERE root_action_id='old-root'",
        );
        conn.execute_batch("ROLLBACK").unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM generation_roots WHERE root_action_id='fresh'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}
