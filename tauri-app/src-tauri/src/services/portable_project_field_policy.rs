//! 便携字段策略表与源 schema 对账 —— 平移自 `electron/services/portable-project-format.ts` 的
//! `fieldPolicy` / `getPortableFieldPolicy` / `listPortableFieldPolicyKeys` /
//! `assertPortableSourceSchema` 四段（与 [`crate::services::portable_project_format`] 同源同文，
//! 拆分为独立模块只因 Rust 侧需要 `rusqlite` 依赖）。
//!
//! 策略表本体 1:1 取自基线 `portable-project-field-policy.json`（51 张表；逐字段给出
//! 处置、校验器、消费者与来源），以保证 Tauri 侧导出的迁移口径与基线**同表同字段同处置**。
//!
//! # 与基线的差异
//!
//! - 基线在 `assertPortableSourceSchema` 里先跑桌面迁移注册表校验
//!   （`verifySchema(SqliteSchemaAdapter, getDesktopMigrationRegistry(), CURRENT_DESKTOP_SCHEMA_VERSION)`）；
//!   Tauri 侧是全新库、无迁移运行时（`db::schema::create_tables` 直接建最终列集，
//!   Tauri 以 `.lore/` 与基线 `.ai-novel/` 分栈），故该步等价物 = 下面的「表/字段双向精确对账」：
//!   实际库中不得出现策略表之外的表或字段，策略表字段（除 `optionalFields`）也必须在库中存在；
//! - 基线的 `sqlite_*` 实现对象被显式排除（本实现同样按 `name NOT LIKE 'sqlite_%'` 过滤）。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::OnceLock;

use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;

/// 策略表本体（基线同名 JSON 文件原样内嵌）
const FIELD_POLICY_JSON: &str = include_str!("portable_project_field_policy.json");

/// 上游 `PortableFieldDisposition`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortableFieldDisposition {
    PreserveDomain,
    HistoricalNonreplayable,
    RedactedProjection,
    ExcludeMachineAuthority,
    RebuildStale,
    ValidatedRelative,
    AvatarAsset,
}

impl PortableFieldDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreserveDomain => "preserve-domain",
            Self::HistoricalNonreplayable => "historical-nonreplayable",
            Self::RedactedProjection => "redacted-projection",
            Self::ExcludeMachineAuthority => "exclude-machine-authority",
            Self::RebuildStale => "rebuild-stale",
            Self::ValidatedRelative => "validated-relative",
            Self::AvatarAsset => "avatar-asset",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "preserve-domain" => Self::PreserveDomain,
            "historical-nonreplayable" => Self::HistoricalNonreplayable,
            "redacted-projection" => Self::RedactedProjection,
            "exclude-machine-authority" => Self::ExcludeMachineAuthority,
            "rebuild-stale" => Self::RebuildStale,
            "validated-relative" => Self::ValidatedRelative,
            "avatar-asset" => Self::AvatarAsset,
            _ => return None,
        })
    }
}

/// 上游 `PortableFieldPolicy.origin`（联合类型四态）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortableFieldOrigin {
    SignedS01,
    M01M05Delta,
    M06Delta,
    RegisteredDonor,
}

impl PortableFieldOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SignedS01 => "signed-s01",
            Self::M01M05Delta => "m01-m05-delta",
            Self::M06Delta => "m06-delta",
            Self::RegisteredDonor => "registered-donor",
        }
    }
}

/// 上游 `PortableFieldPolicy`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableFieldPolicy {
    pub disposition: PortableFieldDisposition,
    pub validator: String,
    pub consumer: String,
    pub origin: PortableFieldOrigin,
    /// 基线签名处置文案；delta 引入的字段为 `None`。
    pub signed_disposition: Option<String>,
}

fn policy_invalid() -> String {
    "PORTABLE_POLICY_INVALID".to_string()
}

/// 基线 `signedDefaults`（A 无默认值 —— 头像字段必须由 `signedOverrides` 提供）
fn signed_default(code: &str) -> Option<&'static str> {
    Some(match code {
        "D" => "preserve-domain-value",
        "H" => "historical-read-only-projection",
        "R" => "allowlist-rebuild-or-block",
        "X" => "exclude-machine-authority",
        "B" => "rebuildable-metric-version",
        "P" => "validate-relative-projection",
        _ => return None,
    })
}

fn string_list(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// 基线 `loadFieldPolicy`：逐字段校验策略表自洽并归一化为查找表
fn load_field_policy() -> Result<BTreeMap<String, PortableFieldPolicy>, String> {
    let document: Value = serde_json::from_str(FIELD_POLICY_JSON).map_err(|_| policy_invalid())?;
    let object = document.as_object().ok_or_else(policy_invalid)?;
    if object.get("schemaVersion").and_then(Value::as_u64)
        != Some(crate::services::portable_project_format::PORTABLE_SOURCE_SCHEMA_VERSION as u64)
    {
        return Err(policy_invalid());
    }
    let rules_document = object.get("rules").ok_or_else(policy_invalid)?;
    let rules_object = rules_document.as_object().ok_or_else(policy_invalid)?;
    let tables_object = object
        .get("tables")
        .and_then(Value::as_object)
        .ok_or_else(policy_invalid)?;
    let delta_tables = string_list(object.get("deltaTables").unwrap_or(&Value::Null));
    let m06_tables = string_list(object.get("m06Tables").unwrap_or(&Value::Null));
    let delta_character_fields =
        string_list(object.get("deltaCharacterFields").unwrap_or(&Value::Null));
    let optional_fields = string_list(object.get("optionalFields").unwrap_or(&Value::Null));
    let signed_overrides: HashMap<String, String> = object
        .get("signedOverrides")
        .and_then(Value::as_object)
        .ok_or_else(policy_invalid)?
        .iter()
        .map(|(key, value)| (key.clone(), value.as_str().unwrap_or_default().to_string()))
        .collect();

    let mut policy: BTreeMap<String, PortableFieldPolicy> = BTreeMap::new();
    for (table, fields) in tables_object {
        let Some(fields) = fields.as_object() else {
            return Err(policy_invalid());
        };
        if table.is_empty() {
            return Err(policy_invalid());
        }
        for (field, code) in fields {
            let code = code.as_str().ok_or_else(policy_invalid)?;
            let Some(rule) = rules_object.get(code) else {
                return Err(policy_invalid());
            };
            let Some(parts) = rule.as_array() else {
                return Err(policy_invalid());
            };
            if parts.len() != 3 {
                return Err(policy_invalid());
            }
            let disposition_text = parts[0].as_str().unwrap_or_default();
            let Some(disposition) = PortableFieldDisposition::parse(disposition_text) else {
                return Err(policy_invalid());
            };
            let validator = parts[1].as_str().unwrap_or_default();
            let consumer = parts[2].as_str().unwrap_or_default();
            let key = format!("{table}.{field}");
            let delta = optional_fields.contains(&key)
                || delta_tables.contains(table)
                || m06_tables.contains(table)
                || (table == "characters" && delta_character_fields.contains(field));
            let signed_disposition = if delta {
                None
            } else {
                signed_overrides
                    .get(&key)
                    .cloned()
                    .or_else(|| signed_default(code).map(str::to_string))
            };
            if field.is_empty()
                || validator.is_empty()
                || consumer.is_empty()
                || (!delta && signed_disposition.is_none())
                || (delta && signed_overrides.contains_key(&key))
                || policy.contains_key(&key)
            {
                return Err(policy_invalid());
            }
            let origin = if optional_fields.contains(&key) {
                PortableFieldOrigin::RegisteredDonor
            } else if m06_tables.contains(table) {
                PortableFieldOrigin::M06Delta
            } else if delta {
                PortableFieldOrigin::M01M05Delta
            } else {
                PortableFieldOrigin::SignedS01
            };
            policy.insert(
                key,
                PortableFieldPolicy {
                    disposition,
                    validator: validator.to_string(),
                    consumer: consumer.to_string(),
                    origin,
                    signed_disposition,
                },
            );
        }
    }
    // 基线：signedOverrides 的每个键都必须命中一个 signed-s01 字段
    for key in signed_overrides.keys() {
        if policy.get(key).map(|entry| entry.origin) != Some(PortableFieldOrigin::SignedS01) {
            return Err(policy_invalid());
        }
    }
    Ok(policy)
}

fn field_policy() -> Result<&'static BTreeMap<String, PortableFieldPolicy>, String> {
    static POLICY: OnceLock<Result<BTreeMap<String, PortableFieldPolicy>, String>> =
        OnceLock::new();
    match POLICY.get_or_init(load_field_policy) {
        Ok(policy) => Ok(policy),
        Err(error) => Err(error.clone()),
    }
}

/// 上游 `getPortableFieldPolicy`：未知字段一律 fail closed
pub fn get_portable_field_policy(table: &str, field: &str) -> Result<PortableFieldPolicy, String> {
    let policy = field_policy()?;
    policy
        .get(&format!("{table}.{field}"))
        .cloned()
        .ok_or_else(|| "PORTABLE_SCHEMA_UNSUPPORTED".to_string())
}

/// 上游 `listPortableFieldPolicyKeys`：排序后的 `table.field` 键集合
pub fn list_portable_field_policy_keys() -> Result<Vec<String>, String> {
    Ok(field_policy()?.keys().cloned().collect())
}

/// 诊断用：实际库与策略表的差异清单（空 = 对账通过）。
///
/// 与 [`assert_portable_source_schema`] 同源；后者把它折叠为 `PORTABLE_SCHEMA_UNSUPPORTED`。
pub fn portable_source_schema_diff(conn: &Connection) -> Result<Vec<String>, String> {
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(|_| "PORTABLE_SCHEMA_UNSUPPORTED".to_string())?;
    let policy = field_policy()?;
    let mut diffs: Vec<String> = Vec::new();
    let mut diffs_objects: Vec<String> = Vec::new();
    let mut actual: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    {
        let mut statement = conn
            .prepare(
                "SELECT name, type FROM pragma_table_list WHERE schema='main' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .map_err(|_| "PORTABLE_SCHEMA_UNSUPPORTED".to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|_| "PORTABLE_SCHEMA_UNSUPPORTED".to_string())?;
        for row in rows {
            let (name, kind) = row.map_err(|_| "PORTABLE_SCHEMA_UNSUPPORTED".to_string())?;
            if kind != "table" {
                diffs_objects.push(format!("{name}#{kind}"));
                continue;
            }
            actual.insert(name, BTreeSet::new());
        }
    }
    let table_names: Vec<String> = actual.keys().cloned().collect();
    for table in table_names {
        let quoted = format!("'{}'", table.replace('\'', "''"));
        let mut statement = conn
            .prepare(&format!("PRAGMA table_info({quoted})"))
            .map_err(|_| "PORTABLE_SCHEMA_UNSUPPORTED".to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|_| "PORTABLE_SCHEMA_UNSUPPORTED".to_string())?;
        let mut fields: BTreeSet<String> = BTreeSet::new();
        for row in rows {
            fields.insert(row.map_err(|_| "PORTABLE_SCHEMA_UNSUPPORTED".to_string())?);
        }
        actual.insert(table, fields);
    }

    for object in &diffs_objects {
        diffs.push(format!("object-not-table:{object}"));
    }
    for (table, fields) in &actual {
        if !policy
            .keys()
            .any(|key| key.starts_with(&format!("{table}.")))
        {
            diffs.push(format!("table-not-in-policy:{table}"));
            continue;
        }
        for field in fields {
            if !policy.contains_key(&format!("{table}.{field}")) {
                diffs.push(format!("field-not-in-policy:{table}.{field}"));
            }
        }
    }
    for (key, entry) in policy.iter() {
        if entry.origin == PortableFieldOrigin::RegisteredDonor {
            continue;
        }
        let (table, field) = key.split_once('.').unwrap_or((key.as_str(), ""));
        let present = actual
            .get(table)
            .map(|fields| fields.contains(field))
            .unwrap_or(false);
        if !present {
            diffs.push(format!("missing-field:{key}"));
        }
    }
    Ok(diffs)
}

/// 上游 `assertPortableSourceSchema`：表/字段双向精确对账（只读）
/// 上游 `assertPortableSourceSchema` 返回的计数
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableSchemaCounts {
    pub tables: usize,
    pub fields: usize,
}

/// 上游 `assertPortableSourceSchema`：表/字段双向精确对账（只读）
pub fn assert_portable_source_schema(conn: &Connection) -> Result<PortableSchemaCounts, String> {
    conn.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(|_| "PORTABLE_SCHEMA_UNSUPPORTED".to_string())?;
    let policy = field_policy().map_err(|_| "PORTABLE_SCHEMA_UNSUPPORTED".to_string())?;
    let mut tables: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    {
        let mut statement = conn
            .prepare(
                "SELECT name, type FROM pragma_table_list WHERE schema='main' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .map_err(|_| "PORTABLE_SCHEMA_UNSUPPORTED".to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|_| "PORTABLE_SCHEMA_UNSUPPORTED".to_string())?;
        for row in rows {
            let (name, kind) = row.map_err(|_| "PORTABLE_SCHEMA_UNSUPPORTED".to_string())?;
            if kind != "table"
                || !policy
                    .keys()
                    .any(|key| key.starts_with(&format!("{name}.")))
            {
                return Err("PORTABLE_SCHEMA_UNSUPPORTED".to_string());
            }
            tables.insert(name, BTreeSet::new());
        }
    }
    let table_names: Vec<String> = tables.keys().cloned().collect();
    for table in table_names {
        let quoted = format!("'{}'", table.replace('\'', "''"));
        let mut statement = conn
            .prepare(&format!("PRAGMA table_info({quoted})"))
            .map_err(|_| "PORTABLE_SCHEMA_UNSUPPORTED".to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|_| "PORTABLE_SCHEMA_UNSUPPORTED".to_string())?;
        let mut fields: BTreeSet<String> = BTreeSet::new();
        for row in rows {
            let field = row.map_err(|_| "PORTABLE_SCHEMA_UNSUPPORTED".to_string())?;
            if !policy.contains_key(&format!("{table}.{field}")) {
                return Err("PORTABLE_SCHEMA_UNSUPPORTED".to_string());
            }
            fields.insert(field);
        }
        tables.insert(table, fields);
    }
    let optional: BTreeSet<String> = policy
        .iter()
        .filter(|(_, entry)| entry.origin == PortableFieldOrigin::RegisteredDonor)
        .map(|(key, _)| key.clone())
        .collect();
    let actual: BTreeSet<String> = tables
        .iter()
        .flat_map(|(table, fields)| {
            fields
                .iter()
                .map(|field| format!("{table}.{field}"))
                .collect::<Vec<_>>()
        })
        .collect();
    if policy
        .keys()
        .any(|key| !actual.contains(key) && !optional.contains(key))
    {
        return Err("PORTABLE_SCHEMA_UNSUPPORTED".to_string());
    }
    Ok(PortableSchemaCounts {
        tables: tables.len(),
        fields: actual.len(),
    })
}

/// 便携导出必须**整表排除**的可重建索引表（Tauri 特有，基线无同名对象）。
///
/// 本项目口径（2026-10-16 用户确认「不与基线互通、按本项目方案存储恢复」）：
/// 知识**事实**（`kb_documents` / `kb_chunks` / `kb_embedding_spaces`）属项目数据，随库副本入档；
/// 只有**派生索引**不入档 —— FTS5 全文索引（`kb_fts`）在归档副本上整表删除，恢复端重建索引；
/// 向量侧同理：`.lore/kb/*.hnsw.*` HNSW 快照是文件级派生面，由资产层按 `rebuild-stale` 省略
/// （不进便携库，故不在此列）。
///
/// `kb_fts` 是 FTS5 虚拟表，`DROP TABLE` 会连带删除其全部影子表（`kb_fts_data` 等），
/// 因此无需逐个列举影子表；虚拟表与影子表在 `pragma_table_list` 中也不是 `table` 类型，
/// 正是 [`assert_portable_source_schema`] 会拒绝的形态 —— 所以排除必须在**对账之前**完成。
pub const PORTABLE_REBUILDABLE_TABLES: &[&'static str] = &["kb_fts"];

/// 在（脱敏副本的）连接上删除可重建索引表；返回实际删除的表名（供 `omittedItems` 记账）。
///
/// 表不存在视为幂等成功（对齐基线「派生面缺失即无事可做」的语义）。
pub fn drop_rebuildable_tables(conn: &Connection) -> Result<Vec<String>, String> {
    let mut dropped: Vec<String> = Vec::new();
    for table in PORTABLE_REBUILDABLE_TABLES {
        let exists = conn
            .query_row(
                "SELECT 1 FROM pragma_table_list WHERE schema='main' AND name=?1",
                [table],
                |_| Ok(()),
            )
            .optional()
            .map_err(|_| "PORTABLE_UNSAFE_PROJECTION".to_string())?;
        if exists.is_none() {
            continue;
        }
        // 表名来自编译期常量，不接受外部输入，故无注入面。
        conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{table}\""))
            .map_err(|_| "PORTABLE_UNSAFE_PROJECTION".to_string())?;
        dropped.push((*table).to_string());
    }
    Ok(dropped)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        create_tables(&conn).expect("建表失败");
        conn
    }

    #[test]
    fn field_policy_loads_with_expected_shape_test() {
        let keys = list_portable_field_policy_keys().unwrap();
        assert!(keys.len() >= 300, "策略表键数异常：{}", keys.len());
        assert!(
            keys.windows(2).all(|pair| pair[0] < pair[1]),
            "键必须有序且唯一"
        );

        // 签名来源字段：处置来自签名默认值或 overrides
        let project_name = get_portable_field_policy("project_core", "project_name").unwrap();
        assert_eq!(project_name.origin, PortableFieldOrigin::SignedS01);
        assert_eq!(
            project_name.disposition,
            PortableFieldDisposition::PreserveDomain
        );
        assert!(project_name.signed_disposition.is_some());

        // 历史不可重放字段：signed overrides 指定
        let manuscript =
            get_portable_field_policy("chapter_deletion_operations", "manuscript_status").unwrap();
        assert_eq!(manuscript.origin, PortableFieldOrigin::SignedS01);
        assert_eq!(
            manuscript.signed_disposition.as_deref(),
            Some("non-executable-history")
        );
        assert_eq!(
            manuscript.disposition,
            PortableFieldDisposition::HistoricalNonreplayable
        );

        // delta 表字段（m01-m05）：无签名处置
        let run_id = get_portable_field_policy("generation_runs", "run_id").unwrap();
        assert_eq!(run_id.origin, PortableFieldOrigin::M01M05Delta);
        assert!(run_id.signed_disposition.is_none());

        // m06 表字段
        let merge = get_portable_field_policy("review_cycle_merges", "cycle_id").unwrap();
        assert_eq!(merge.origin, PortableFieldOrigin::M06Delta);

        // 注册捐赠字段（唯一 optionalField）
        let avatar = get_portable_field_policy("characters", "avatar").unwrap();
        assert_eq!(avatar.origin, PortableFieldOrigin::RegisteredDonor);

        // 若某 delta 字段带 signedOverride，基线判定策略表不自洽（构造校验见下一条测试）
        assert_eq!(
            get_portable_field_policy("characters", "no_such_field").unwrap_err(),
            "PORTABLE_SCHEMA_UNSUPPORTED"
        );
    }

    #[test]
    fn schema_reconcile_diagnostic_test() {
        let conn = memory_db();
        drop_rebuildable_tables(&conn).unwrap();
        let diffs = portable_source_schema_diff(&conn).unwrap();
        for diff in &diffs {
            println!("DIFF {diff}");
        }
        println!("DIFF_COUNT {}", diffs.len());
    }

    #[test]
    fn assert_schema_accepts_tauri_project_schema_test() {
        let conn = memory_db();
        // 活跃库含 Tauri 特有的人工索引表与 FTS 虚拟表/影子表，按基线语义这些必须先在
        // 脱敏副本上排除；对账只针对排除后的便携 schema。
        assert_eq!(
            assert_portable_source_schema(&conn).unwrap_err(),
            "PORTABLE_SCHEMA_UNSUPPORTED",
            "活跃库不得直接通过便携对账"
        );
        let dropped = drop_rebuildable_tables(&conn).unwrap();
        assert_eq!(dropped, vec!["kb_fts".to_string()]);
        assert!(
            drop_rebuildable_tables(&conn).unwrap().is_empty(),
            "重复排除幂等"
        );
        let counts = assert_portable_source_schema(&conn).unwrap();
        assert!(counts.tables >= 40, "表数异常：{}", counts.tables);
        assert!(counts.fields >= 300, "字段数异常：{}", counts.fields);
    }

    #[test]
    fn older_archives_without_source_revisions_still_match_and_upgrade() {
        let conn = memory_db();
        drop_rebuildable_tables(&conn).unwrap();
        for table in ["kb_chunks", "kb_documents"] {
            for event in ["insert", "update", "delete"] {
                conn.execute_batch(&format!("DROP TRIGGER kb_source_revision_{table}_{event}"))
                    .unwrap();
            }
        }
        conn.execute_batch("DROP TABLE kb_source_revisions")
            .unwrap();
        assert!(assert_portable_source_schema(&conn).is_ok());
        create_tables(&conn).unwrap();
        assert_eq!(crate::db::kb::revisions::read(&conn).unwrap().canonical, 0);
        drop_rebuildable_tables(&conn).unwrap();
        assert!(assert_portable_source_schema(&conn).is_ok());
        assert_eq!(
            get_portable_field_policy("kb_source_revisions", "canonical_revision")
                .unwrap()
                .disposition,
            PortableFieldDisposition::RebuildStale
        );
    }

    #[test]
    fn assert_schema_rejects_unknown_table_field_and_missing_column_test() {
        let conn = memory_db();
        drop_rebuildable_tables(&conn).unwrap();
        assert!(assert_portable_source_schema(&conn).is_ok());

        // 未知表
        conn.execute_batch("CREATE TABLE extra_table (id TEXT)")
            .unwrap();
        assert_eq!(
            assert_portable_source_schema(&conn).unwrap_err(),
            "PORTABLE_SCHEMA_UNSUPPORTED"
        );
        conn.execute_batch("DROP TABLE extra_table").unwrap();
        assert!(assert_portable_source_schema(&conn).is_ok());

        // 策略表字段缺失（建一张列不完整的影子表并替换）
        conn.execute_batch(
            "ALTER TABLE consistency_exemptions RENAME TO consistency_exemptions_original;
             CREATE TABLE consistency_exemptions (id TEXT);",
        )
        .unwrap();
        assert_eq!(
            assert_portable_source_schema(&conn).unwrap_err(),
            "PORTABLE_SCHEMA_UNSUPPORTED"
        );
    }
}
