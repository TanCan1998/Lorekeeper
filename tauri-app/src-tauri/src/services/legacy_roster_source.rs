//! LegacyRoster 来源服务（服务层移植自 `electron/services/legacy-roster-source.ts`）。
//!
//! 旧架构（identity 升级前的 `characters` / `character_aliases` /
//! `character_relationships` 三表）角色名册的来源读取、模型上下文捕获与
//! 「旧卡片收养」幂等审批。
//!
//! 注意：本文件是**服务层形态**的移植（强类型请求/回执），与仓库层
//! `repositories::character_roster_repository` 中基于 `serde_json::Value`
//! 的动态端口（`read_legacy_roster_source` / `adopt_legacy_cards`）并存；
//! 两者行为均对齐上游同一 TS 源。服务层独有 `capture_legacy_roster_context`
//! （仓库层无对应物）。
//!
//! 语义对齐铁律：
//! - 业务错误码与上游逐字一致（如 `LEGACY_ROSTER_ID_SCHEMA_REQUIRED`）；
//! - JSON 字段名 camelCase（`#[serde(rename_all = "camelCase")]`），与上游
//!   TS 接口逐字段对齐；
//! - 基础设施层错误（事务、SQL 失败）用中文 `format!` 文案，与仓库层移植一致。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::repositories::character_repository::has_character_identity_schema;
use crate::repositories::character_roster_repository::{
    self as character_roster, hash_text, CharacterRosterMigrationState, CharacterRosterSnapshot,
    CharacterRosterStatus,
};
use crate::repositories::project_core_repository as project_core;

/// `Number.isSafeInteger` 上界（上游 `revision()` 守卫；仓库层 `MAX_SAFE_INTEGER`
/// 为私有，本模块按先例私有重定义）。
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
/// 审批操作 ID 前缀（上游 `const prefix = 'legacy-cards-adoption:'`）。
const ADOPTION_OPERATION_PREFIX: &str = "legacy-cards-adoption:";

/// 旧架构名册来源快照（对齐上游 `LegacyRosterSource`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LegacyRosterSource {
    pub snapshot: CharacterRosterSnapshot,
    pub raw_legacy: String,
    pub legacy_hash: String,
    pub identity_revision: i64,
    pub active_ids: Vec<String>,
    /// Actual rows, including retired identities and author state, not stored meta.fact_hash.
    pub facts_hash: String,
    pub genre: String,
    pub writing_language: String,
}

/// 旧卡片收养请求（对齐上游 `AdoptLegacyCardsRequest`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdoptLegacyCardsRequest {
    pub operation_id: String,
    pub expected_revision: i64,
    pub expected_legacy_hash: String,
    pub expected_identity_revision: i64,
    pub expected_facts_hash: String,
}

/// 旧卡片收养回执（对齐上游 `AdoptLegacyCardsReceipt`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdoptLegacyCardsReceipt {
    pub success: bool,
    pub operation_id: String,
    pub snapshot: CharacterRosterSnapshot,
    pub active_ids: Vec<String>,
}

/// 审批信封（上游私有 `AdoptionEnvelope`，持久化于
/// `character_identity_approvals.receipt_json`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct AdoptionEnvelope {
    version: i64,
    kind: String,
    request: AdoptLegacyCardsRequest,
    request_hash: String,
    receipt: AdoptLegacyCardsReceipt,
    receipt_hash: String,
}

/// 读取旧架构名册来源（对齐上游 `readLegacyRosterSource`）。
///
/// 上游包在 `db.transaction()`（deferred）内，这里用 `unchecked_transaction`
/// 保证一致读快照。
pub fn read_legacy_roster_source(conn: &Connection) -> Result<LegacyRosterSource, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败: {error}"))?;
    let source = read_source(&tx)?;
    tx.commit()
        .map_err(|error| format!("提交事务失败: {error}"))?;
    Ok(source)
}

/// 捕获旧名册模型来源上下文（对齐上游 `captureLegacyRosterContext`）。
pub fn capture_legacy_roster_context(conn: &Connection) -> Result<LegacyRosterSource, String> {
    let source = read_legacy_roster_source(conn)?;
    if source.snapshot.migration_state != CharacterRosterMigrationState::LegacyMarkdownPending
        || source.snapshot.status != CharacterRosterStatus::LegacyRepairRequired
        || !source.active_ids.is_empty()
        || !source.snapshot.entries.is_empty()
        || source.raw_legacy.trim().is_empty()
    {
        return Err("LEGACY_ROSTER_MODEL_SOURCE_REQUIRED".to_string());
    }
    Ok(source)
}

/// 旧卡片收养（对齐上游 `adoptLegacyCards`）。
///
/// 幂等：同一 `operationId` 已有审批信封且请求一致时直接重放持久化回执。
///
/// 事务形态对齐仓库层 `character_roster_repository::adopt_legacy_cards`
/// 先例：幂等读回与来源校验在事务外执行，写段自开 `unchecked_transaction`
/// （rusqlite 0.40 无 `&self` + Immediate 事务 API；单连接桌面应用内
/// 连接级写入串行已覆盖上游 `.immediate()` 的串行化意图）。
pub fn adopt_legacy_cards(
    conn: &Connection,
    request: &AdoptLegacyCardsRequest,
) -> Result<AdoptLegacyCardsReceipt, String> {
    // 上游对象形状守卫（多余键 / 非字符串 / 空 operationId / 超长 / 非安全整数 /
    // 非 64-hex）在类型层由 `AdoptLegacyCardsRequest` 强制，这里保留值域校验。
    if request.operation_id.trim().is_empty()
        || request.operation_id.len() > 200
        || !is_safe_non_negative(request.expected_revision)
        || !is_safe_non_negative(request.expected_identity_revision)
        || !is_sha256_hex(&request.expected_legacy_hash)
        || !is_sha256_hex(&request.expected_facts_hash)
    {
        return Err("LEGACY_ROSTER_ADOPTION_INVALID".to_string());
    }
    if !has_character_identity_schema(conn) {
        return Err("LEGACY_ROSTER_ID_SCHEMA_REQUIRED".to_string());
    }
    let request_hash = hash_text(&canonical_json(request));
    let operation_id = format!("{ADOPTION_OPERATION_PREFIX}{}", request.operation_id);
    // 幂等读回：同一 operationId 已有审批信封时校验信封完整性后直接重放。
    // （对齐仓库层 character_roster_repository::adopt_legacy_cards 先例：
    // 读回与来源校验在事务外执行，写段自开 unchecked_transaction。）
    if let Some((payload_hash, receipt_json)) = conn
        .query_row(
            "SELECT payload_hash, receipt_json FROM character_identity_approvals WHERE operation_id = ?1",
            [&operation_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| format!("读取审批记录失败: {error}"))?
    {
        // 上游 `JSON.parse` 失败会抛出原始 SyntaxError；类型化端口统一归入
        // `LEGACY_ROSTER_ADOPTION_RECEIPT_INVALID`（信封不可读即无效）。
        let saved: AdoptionEnvelope = serde_json::from_str(&receipt_json)
            .map_err(|_| "LEGACY_ROSTER_ADOPTION_RECEIPT_INVALID".to_string())?;
        // 上游 `!Array.isArray(saved.receipt.activeIds)` 在类型层恒为真（Vec），
        // 其余守卫逐条对齐。
        if saved.version != 1
            || saved.kind != "legacy-cards-adoption"
            || saved.request_hash != payload_hash
            || hash_text(&canonical_json(&saved.request)) != saved.request_hash
            || saved.receipt_hash != hash_text(&canonical_json(&saved.receipt))
            || !saved.receipt.success
            || saved.receipt.operation_id != request.operation_id
            || saved.receipt.snapshot.migration_state != CharacterRosterMigrationState::Ready
            || saved.receipt.snapshot.status != CharacterRosterStatus::Ready
        {
            return Err("LEGACY_ROSTER_ADOPTION_RECEIPT_INVALID".to_string());
        }
        if request_hash != saved.request_hash || !values_equal(request, &saved.request) {
            return Err("LEGACY_ROSTER_ADOPTION_NONCE_CONFLICT".to_string());
        }
        return Ok(saved.receipt);
    }
    let source = read_source(conn)?;
    if source.snapshot.migration_state != CharacterRosterMigrationState::LegacyCardsPreserved
        || source.snapshot.status != CharacterRosterStatus::Inconsistent
        || source.active_ids.is_empty()
    {
        return Err("LEGACY_ROSTER_CARDS_REQUIRED".to_string());
    }
    if source.snapshot.revision != request.expected_revision
        || source.identity_revision != request.expected_identity_revision
        || source.legacy_hash != request.expected_legacy_hash
        || source.facts_hash != request.expected_facts_hash
    {
        return Err("LEGACY_ROSTER_ADOPTION_SOURCE_CHANGED".to_string());
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("开启事务失败: {error}"))?;
    character_roster::refresh_character_identity_projection(&tx)?;
    let snapshot = character_roster::read(&tx)?;
    if snapshot.status != CharacterRosterStatus::Ready
        || snapshot.migration_state != CharacterRosterMigrationState::Ready
        || snapshot.legacy_markdown
            != (if source.raw_legacy.is_empty() {
                None
            } else {
                Some(source.raw_legacy.clone())
            })
    {
        return Err("LEGACY_ROSTER_ADOPTION_READBACK_INVALID".to_string());
    }
    // 首次 ACK 与每次持久化重放返回同一 JSON 表示（上游注释）。
    let receipt = AdoptLegacyCardsReceipt {
        success: true,
        operation_id: request.operation_id.clone(),
        snapshot,
        active_ids: source.active_ids,
    };
    let envelope = AdoptionEnvelope {
        version: 1,
        kind: "legacy-cards-adoption".to_string(),
        request: request.clone(),
        request_hash: request_hash.clone(),
        receipt_hash: hash_text(&canonical_json(&receipt)),
        receipt: receipt.clone(),
    };
    tx.execute(
        "INSERT INTO character_identity_approvals (operation_id, payload_hash, receipt_json) VALUES (?1, ?2, ?3)",
        rusqlite::params![operation_id, request_hash, canonical_json(&envelope)],
    )
    .map_err(|error| format!("写入审批记录失败: {error}"))?;
    tx.commit()
        .map_err(|error| format!("提交事务失败: {error}"))?;
    Ok(receipt)
}

/// 上游私有 `readSource`：不包事务的实际读取逻辑。
fn read_source(conn: &Connection) -> Result<LegacyRosterSource, String> {
    if !has_character_identity_schema(conn) {
        return Err("LEGACY_ROSTER_ID_SCHEMA_REQUIRED".to_string());
    }
    let core: Option<(Option<String>, String)> = conn
        .query_row(
            "SELECT genre, writing_language FROM project_core WHERE id = 'main'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("读取项目核心配置失败: {error}"))?;
    let identity_revision: Option<i64> = conn
        .query_row(
            "SELECT revision FROM character_identity_meta WHERE id = 'main'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("读取身份元数据失败: {error}"))?;
    let raw_legacy: Option<String> = conn
        .query_row(
            "SELECT legacy_markdown FROM character_roster_meta WHERE id = 'main'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("读取角色名单元数据失败: {error}"))?;
    let (genre, core_writing_language) = core.ok_or("LEGACY_ROSTER_SOURCE_INVALID")?;
    let identity_revision = identity_revision.ok_or("LEGACY_ROSTER_SOURCE_INVALID")?;
    let raw_legacy = raw_legacy.ok_or("LEGACY_ROSTER_SOURCE_INVALID")?;
    if !is_safe_non_negative(identity_revision) {
        return Err("LEGACY_ROSTER_SOURCE_INVALID".to_string());
    }
    let mut characters: Vec<(String, i64)> = Vec::new();
    {
        let mut statement = conn
            .prepare("SELECT character_id, retired FROM characters ORDER BY character_id")
            .map_err(|error| format!("读取角色失败: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|error| format!("读取角色失败: {error}"))?;
        for row in rows {
            let (character_id, retired): (String, i64) =
                row.map_err(|error| format!("读取角色失败: {error}"))?;
            if character_id.trim().is_empty() {
                return Err("LEGACY_ROSTER_ID_INVALID".to_string());
            }
            characters.push((character_id, retired));
        }
    }
    let active_ids: Vec<String> = characters
        .iter()
        .filter(|(_, retired)| *retired == 0)
        .map(|(character_id, _)| character_id.clone())
        .collect();
    let map_rows = |table: &str,
                    order: &str|
     -> Result<Vec<serde_json::Map<String, serde_json::Value>>, String> {
        let mut statement = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
            .map_err(|error| format!("读取 {table} 失败: {error}"))?;
        let columns: Vec<String> = statement
            .column_names()
            .iter()
            .map(|name| name.to_string())
            .collect();
        let rows = statement
            .query_map([], |row| {
                let mut map = serde_json::Map::new();
                for (index, name) in columns.iter().enumerate() {
                    let value: rusqlite::types::Value = row.get(index)?;
                    map.insert(
                        name.clone(),
                        match value {
                            rusqlite::types::Value::Null => serde_json::Value::Null,
                            rusqlite::types::Value::Integer(number) => {
                                serde_json::json!(number)
                            }
                            rusqlite::types::Value::Real(number) => serde_json::json!(number),
                            rusqlite::types::Value::Text(text) => serde_json::json!(text),
                            rusqlite::types::Value::Blob(blob) => serde_json::json!(blob),
                        },
                    );
                }
                Ok(map)
            })
            .map_err(|error| format!("读取 {table} 失败: {error}"))?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row.map_err(|error| format!("读取 {table} 失败: {error}"))?);
        }
        Ok(items)
    };
    let facts = serde_json::json!({
        "characters": map_rows("characters", "character_id")?,
        "aliases": map_rows("character_aliases", "character_id, name, source_key, valid_from")?,
        "relationships": map_rows("character_relationships", "relationship_id")?,
    });
    let legacy_hash = hash_text(&raw_legacy);
    Ok(LegacyRosterSource {
        snapshot: character_roster::read(conn)?,
        raw_legacy,
        legacy_hash,
        identity_revision,
        active_ids,
        facts_hash: hash_text(&facts.to_string()),
        // 上游 `core.genre` 直接透传（NULL 时为 null）；类型化端口按代码库
        // 惯例（`row_to_data`）将 NULL 归一化为空串，下游 `genre || '（待确认）'`
        // 语义不变。
        genre: genre.unwrap_or_default(),
        writing_language: project_core::resolve_writing_language(&core_writing_language),
    })
}

/// 上游 `revision()`：安全非负整数。
fn is_safe_non_negative(value: i64) -> bool {
    (0..=MAX_SAFE_INTEGER).contains(&value)
}

/// 上游 `hash()`：64 位小写十六进制（SHA-256 摘要形状）。
fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// 确定性 JSON 序列化（结构字段按声明顺序，与上游 `JSON.stringify` 插入顺序一致）。
fn canonical_json<T: Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string(value).expect("LegacyRoster 信封序列化失败")
}

/// `isDeepStrictEqual` 的 JSON 近似（对齐 `generation_run_repository` 先例）。
fn values_equal<T: Serialize + ?Sized, U: Serialize + ?Sized>(left: &T, right: &U) -> bool {
    match (serde_json::to_value(left), serde_json::to_value(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;
    use crate::repositories::project_core_repository as project_core;

    /// 无身份 schema 的内存库（六张身份表已拆除，`has_character_identity_schema` 为 false）。
    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        create_tables(&conn).expect("建表失败");
        project_core::init(&conn, "测试项目", project_core::DEFAULT_WRITING_LANGUAGE)
            .expect("主台账初始化失败");
        conn.execute_batch(
            "DROP TABLE IF EXISTS character_identity_origins;
             DROP TABLE IF EXISTS character_aliases;
             DROP TABLE IF EXISTS character_identity_proposals;
             DROP TABLE IF EXISTS character_identity_approvals;
             DROP TABLE IF EXISTS character_relationships;
             DROP TABLE IF EXISTS character_identity_meta;",
        )
        .expect("身份表拆除失败");
        conn
    }

    /// 保留身份 schema 的内存库（`has_character_identity_schema` 为 true）。
    fn identity_schema_db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        create_tables(&conn).expect("建表失败");
        project_core::init(&conn, "测试项目", project_core::DEFAULT_WRITING_LANGUAGE)
            .expect("主台账初始化失败");
        // 触发 migrate_character_roster_schema，保证 character_roster_meta 行存在。
        character_roster::read(&conn).expect("roster 元数据初始化失败");
        conn
    }

    /// 置为「旧 Markdown 待迁移」旧态（上游修复流程的入口状态）。
    fn set_pending_state(conn: &Connection) {
        conn.execute(
            "UPDATE character_roster_meta SET migration_state = 'legacy_markdown_pending', legacy_markdown = '旧图谱原文' WHERE id = 'main'",
            [],
        )
        .expect("旧态写入失败");
    }

    fn adoption_request(operation_id: &str) -> AdoptLegacyCardsRequest {
        AdoptLegacyCardsRequest {
            operation_id: operation_id.to_string(),
            expected_revision: 0,
            expected_legacy_hash: hash_text("旧图谱原文"),
            expected_identity_revision: 0,
            expected_facts_hash: hash_text("{}"),
        }
    }

    #[test]
    fn read_legacy_roster_source_returns_pending_source() {
        let conn = identity_schema_db();
        set_pending_state(&conn);
        let source = read_legacy_roster_source(&conn).expect("来源读取失败");
        assert_eq!(source.raw_legacy, "旧图谱原文");
        assert_eq!(source.legacy_hash, hash_text("旧图谱原文"));
        assert_eq!(source.identity_revision, 0);
        assert!(source.active_ids.is_empty());
        assert_eq!(source.writing_language, "zh-CN");
        assert_eq!(
            source.snapshot.migration_state,
            CharacterRosterMigrationState::LegacyMarkdownPending
        );
        assert_eq!(
            source.snapshot.status,
            CharacterRosterStatus::LegacyRepairRequired
        );
        assert_eq!(source.facts_hash.len(), 64);
    }

    #[test]
    fn capture_legacy_roster_context_accepts_pending_source() {
        let conn = identity_schema_db();
        set_pending_state(&conn);
        let source = capture_legacy_roster_context(&conn).expect("上下文捕获失败");
        assert_eq!(source.raw_legacy, "旧图谱原文");
        assert_eq!(
            source.snapshot.migration_state,
            CharacterRosterMigrationState::LegacyMarkdownPending
        );
    }

    #[test]
    fn capture_legacy_roster_context_requires_pending_markdown() {
        let conn = identity_schema_db();
        let error = capture_legacy_roster_context(&conn).expect_err("空旧态应拒绝");
        assert_eq!(error, "LEGACY_ROSTER_MODEL_SOURCE_REQUIRED");
    }

    #[test]
    fn read_legacy_roster_source_requires_identity_schema() {
        let conn = memory_db();
        let error = read_legacy_roster_source(&conn).expect_err("无身份 schema 应拒绝");
        assert_eq!(error, "LEGACY_ROSTER_ID_SCHEMA_REQUIRED");
    }

    #[test]
    fn capture_legacy_roster_context_requires_identity_schema() {
        let conn = memory_db();
        let error = capture_legacy_roster_context(&conn).expect_err("无身份 schema 应拒绝");
        assert_eq!(error, "LEGACY_ROSTER_ID_SCHEMA_REQUIRED");
    }

    #[test]
    fn adopt_legacy_cards_requires_identity_schema() {
        let conn = memory_db();
        let error =
            adopt_legacy_cards(&conn, &adoption_request("op-1")).expect_err("无身份 schema 应拒绝");
        assert_eq!(error, "LEGACY_ROSTER_ID_SCHEMA_REQUIRED");
    }

    #[test]
    fn adopt_legacy_cards_rejects_invalid_request() {
        let conn = identity_schema_db();
        let mut request = adoption_request("op-1");
        request.operation_id = " ".to_string();
        assert_eq!(
            adopt_legacy_cards(&conn, &request).expect_err("空 operationId 应拒绝"),
            "LEGACY_ROSTER_ADOPTION_INVALID"
        );
        let mut request = adoption_request("op-1");
        request.operation_id = "x".repeat(201);
        assert_eq!(
            adopt_legacy_cards(&conn, &request).expect_err("超长 operationId 应拒绝"),
            "LEGACY_ROSTER_ADOPTION_INVALID"
        );
        let mut request = adoption_request("op-1");
        request.expected_revision = -1;
        assert_eq!(
            adopt_legacy_cards(&conn, &request).expect_err("负 revision 应拒绝"),
            "LEGACY_ROSTER_ADOPTION_INVALID"
        );
        let mut request = adoption_request("op-1");
        request.expected_legacy_hash = "ZZ".to_string();
        assert_eq!(
            adopt_legacy_cards(&conn, &request).expect_err("非 hex hash 应拒绝"),
            "LEGACY_ROSTER_ADOPTION_INVALID"
        );
    }

    #[test]
    fn adopt_legacy_cards_requires_preserved_cards() {
        let conn = identity_schema_db();
        set_pending_state(&conn);
        // pending 态不是 legacy_cards_preserved + inconsistent，收养应拒绝。
        let error =
            adopt_legacy_cards(&conn, &adoption_request("op-1")).expect_err("pending 态应拒绝收养");
        assert_eq!(error, "LEGACY_ROSTER_CARDS_REQUIRED");
    }
}
