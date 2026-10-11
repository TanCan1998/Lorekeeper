//! 批次 G4-1：便携迁移权威（自 `electron/services/portable-transfer-authority.ts` 平移）。
//!
//! 上游事实源：`electron/services/portable-transfer-authority.ts`（317 行）。
//! 语义铁律：错误码逐字一致（`PORTABLE_TRANSFER_AUTHORITY_INVALID`）；
//! `receiptId` = `portable-transfer:` + SHA-256(规范 JSON).slice(0,32)，
//! 其哈希输入为上游对象字面量构造顺序
//! （originProjectId / snapshotGeneration / portableDatabaseSha256 /
//! finalizations / summarySources）——Rust 侧以声明顺序相同的
//! `ReceiptInput` 结构序列化保证逐字节一致。
//!
//! # 与上游的实现层偏差（行为等价）
//! - 上游 `better-sqlite3` `Database` → `rusqlite::Connection`。
//! - 上游 SQL 语句逐字平移；SQLite 层错误（缺表等）统一映射为
//!   `PORTABLE_TRANSFER_AUTHORITY_INVALID`（TS 中原始 sqlite 错误会
//!   直接透出；调用面仅区分成功/失败，文案经命令层 MUTATING 规则包裹）。
//! - `Number.isSafeInteger` 的 ±(2^53-1) 边界由 `integer()` 保持
//!   （含对 JSON 无小数浮点表示的接受）。
//!
//! 纯逻辑、无 Tauri 依赖；仅依赖 `rusqlite::Connection` 只读查询面。

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::services::portable_runtime_freeze::PortableRuntimeFreezeGuard;
use crate::services::portable_runtime_freeze::PortableRuntimeFreezeTable;

/// 记录数上限（上游 `MAX_RECORDS = 100_000`，模块私有）。
const MAX_RECORDS: usize = 100_000;
/// 序列化字节上限（上游 `MAX_BYTES = 4 * 1024 * 1024`，模块私有）。
const MAX_BYTES: usize = 4 * 1024 * 1024;
/// `Number.isSafeInteger` 上界（上游 `integer()` 的隐含边界）。
const SAFE_INTEGER_MAX: i64 = (1i64 << 53) - 1;

/// 便携源 schema 版本（上游 `PORTABLE_SOURCE_SCHEMA_VERSION`，
/// 自 `electron/services/portable-project-format.ts`，值 7）。
pub const PORTABLE_SOURCE_SCHEMA_VERSION: i64 = 7;

/// 权威文件相对路径（上游 `PORTABLE_TRANSFER_AUTHORITY_PATH`）。
pub const PORTABLE_TRANSFER_AUTHORITY_PATH: &str = ".ai-novel/portable-transfer-authority.json";

static ID: LazyLock<regex::Regex> = LazyLock::new(|| {
    // 上游模式逐字不变；`\p{L}\p{N}` 的 Unicode 类编译产物超 regex crate 默认
    // 10MB size_limit（JavaScript 无此限制），仅放宽编译上限。
    regex::RegexBuilder::new(r"^[\p{L}\p{N}._:@+-]{1,512}$")
        .size_limit(1 << 30)
        .build()
        .expect("portable id regex is valid")
});
static UUID: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
    )
    .expect("portable uuid regex is valid")
});
static HASH: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^[a-f0-9]{64}$").expect("portable hash regex is valid"));

/// 权威错误（上游 `throw new Error('PORTABLE_TRANSFER_AUTHORITY_INVALID')`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableTransferAuthorityError;

impl PortableTransferAuthorityError {
    /// 上游错误码字符串（逐字一致）。
    pub fn code(&self) -> &'static str {
        "PORTABLE_TRANSFER_AUTHORITY_INVALID"
    }
}

impl std::fmt::Display for PortableTransferAuthorityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for PortableTransferAuthorityError {}

impl From<rusqlite::Error> for PortableTransferAuthorityError {
    fn from(_: rusqlite::Error) -> Self {
        Self
    }
}

/// 定稿权威（上游 `PortableFinalizationAuthority`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableFinalizationAuthority {
    pub finalization_id: String,
    pub draft_id: i64,
    pub chapter_number: i64,
    pub content_hash: String,
}

/// 摘要来源权威（上游 `PortableSummarySourceAuthority`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableSummarySourceAuthority {
    pub summary_id: i64,
    pub draft_id: i64,
    pub chapter_number: i64,
    pub source_finalization_id: String,
    pub source_content_hash: String,
    pub projection_generation: i64,
}

/// 便携迁移权威（上游 `PortableTransferAuthority`）。
///
/// 字段声明顺序对齐上游对象字面量构造顺序，
/// 以保证 `JSON.stringify` 序列化（收据哈希输入）逐字节一致。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableTransferAuthority {
    pub version: i64,
    pub receipt_id: String,
    pub origin_project_id: String,
    pub target_project_id: Option<String>,
    pub snapshot_generation: String,
    pub source_schema_version: i64,
    pub portable_database_sha256: String,
    pub requires_runtime_freeze_guard: bool,
    pub finalizations: Vec<PortableFinalizationAuthority>,
    pub summary_sources: Vec<PortableSummarySourceAuthority>,
}

/// 收据哈希输入（上游 `receiptId()` 的 `Pick<…>` 参数，
/// 字段顺序 = 上游 `JSON.stringify` 对象字面量顺序）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReceiptInput<'a> {
    origin_project_id: &'a str,
    snapshot_generation: &'a str,
    portable_database_sha256: &'a str,
    finalizations: &'a [PortableFinalizationAuthority],
    summary_sources: &'a [PortableSummarySourceAuthority],
}

fn invalid() -> PortableTransferAuthorityError {
    PortableTransferAuthorityError
}

/// 上游 `sha256(value)`：`createHash('sha256').update(value).digest('hex')`。
fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 上游 `receiptId(input)`：`portable-transfer:` + SHA-256 前 32 个 hex 字符。
fn receipt_id(input: &ReceiptInput<'_>) -> String {
    let serialized = serde_json::to_string(input).expect("receipt input serializes");
    format!("portable-transfer:{}", &sha256_hex(&serialized)[..32])
}

/// 上游 `object(value, keys)`：精确键集校验（排序后逐项比较）。
fn exact_object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, PortableTransferAuthorityError> {
    let map = value.as_object().ok_or_else(invalid)?;
    let mut actual: Vec<&str> = map.keys().map(String::as_str).collect();
    actual.sort_unstable();
    let mut expected = keys.to_vec();
    expected.sort_unstable();
    if actual != expected {
        return Err(invalid());
    }
    Ok(map)
}

/// 上游 `identifier(value)`。
fn identifier(value: Option<&Value>) -> Result<&str, PortableTransferAuthorityError> {
    value
        .and_then(Value::as_str)
        .filter(|text| ID.is_match(text))
        .ok_or_else(invalid)
}

/// 上游 `uuid(value)`。
fn uuid(value: Option<&Value>) -> Result<&str, PortableTransferAuthorityError> {
    value
        .and_then(Value::as_str)
        .filter(|text| UUID.is_match(text))
        .ok_or_else(invalid)
}

/// 上游 `hash(value)`。
fn hash(value: Option<&Value>) -> Result<&str, PortableTransferAuthorityError> {
    value
        .and_then(Value::as_str)
        .filter(|text| HASH.is_match(text))
        .ok_or_else(invalid)
}

/// 上游 `integer(value, minimum = 0)`：`Number.isSafeInteger` 语义。
fn integer(value: Option<&Value>, minimum: i64) -> Result<i64, PortableTransferAuthorityError> {
    match value {
        Some(Value::Number(number)) => {
            if let Some(candidate) = number.as_i64() {
                if candidate >= minimum
                    && candidate >= -SAFE_INTEGER_MAX
                    && candidate <= SAFE_INTEGER_MAX
                {
                    return Ok(candidate);
                }
            } else if let Some(float) = number.as_f64() {
                // JS 中 `1.0 === 1`，无小数部分的浮点仍是安全整数。
                let candidate = float as i64;
                if float.fract() == 0.0
                    && float >= minimum as f64
                    && float >= -(SAFE_INTEGER_MAX as f64)
                    && float <= SAFE_INTEGER_MAX as f64
                {
                    return Ok(candidate);
                }
            }
            Err(invalid())
        }
        _ => Err(invalid()),
    }
}

/// 上游 `finalization(value)`。
fn validate_finalization(
    value: &Value,
) -> Result<PortableFinalizationAuthority, PortableTransferAuthorityError> {
    let row = exact_object(
        value,
        &["finalizationId", "draftId", "chapterNumber", "contentHash"],
    )?;
    Ok(PortableFinalizationAuthority {
        finalization_id: identifier(row.get("finalizationId"))?.to_owned(),
        draft_id: integer(row.get("draftId"), 1)?,
        chapter_number: integer(row.get("chapterNumber"), 1)?,
        content_hash: hash(row.get("contentHash"))?.to_owned(),
    })
}

/// 上游 `summarySource(value)`。
fn validate_summary_source(
    value: &Value,
) -> Result<PortableSummarySourceAuthority, PortableTransferAuthorityError> {
    let row = exact_object(
        value,
        &[
            "summaryId",
            "draftId",
            "chapterNumber",
            "sourceFinalizationId",
            "sourceContentHash",
            "projectionGeneration",
        ],
    )?;
    Ok(PortableSummarySourceAuthority {
        summary_id: integer(row.get("summaryId"), 1)?,
        draft_id: integer(row.get("draftId"), 1)?,
        chapter_number: integer(row.get("chapterNumber"), 1)?,
        source_finalization_id: identifier(row.get("sourceFinalizationId"))?.to_owned(),
        source_content_hash: hash(row.get("sourceContentHash"))?.to_owned(),
        projection_generation: integer(row.get("projectionGeneration"), 0)?,
    })
}

/// 上游 `assertFacts(finalizations, summarySources)`。
fn assert_facts(
    finalizations: &[PortableFinalizationAuthority],
    summary_sources: &[PortableSummarySourceAuthority],
) -> Result<(), PortableTransferAuthorityError> {
    let mut finalization_ids = HashSet::new();
    let mut draft_ids = HashSet::new();
    let mut chapter_numbers = HashSet::new();
    for item in finalizations {
        if !finalization_ids.insert(item.finalization_id.clone())
            || !draft_ids.insert(item.draft_id)
            || !chapter_numbers.insert(item.chapter_number)
        {
            return Err(invalid());
        }
    }
    let mut summary_ids = HashSet::new();
    for item in summary_sources {
        if !summary_ids.insert(item.summary_id) {
            return Err(invalid());
        }
    }
    let finalization_by_id: HashMap<&str, &PortableFinalizationAuthority> = finalizations
        .iter()
        .map(|item| (item.finalization_id.as_str(), item))
        .collect();
    for item in summary_sources {
        match finalization_by_id.get(item.source_finalization_id.as_str()) {
            Some(current)
                if current.content_hash == item.source_content_hash
                    && current.draft_id == item.draft_id
                    && current.chapter_number == item.chapter_number => {}
            _ => return Err(invalid()),
        }
    }
    Ok(())
}

/// 上游 `parsePortableTransferAuthority(value)`。
pub fn parse_portable_transfer_authority(
    value: &Value,
) -> Result<PortableTransferAuthority, PortableTransferAuthorityError> {
    let source = exact_object(
        value,
        &[
            "version",
            "receiptId",
            "originProjectId",
            "targetProjectId",
            "snapshotGeneration",
            "sourceSchemaVersion",
            "portableDatabaseSha256",
            "requiresRuntimeFreezeGuard",
            "finalizations",
            "summarySources",
        ],
    )?;
    if !is_exact_number(source.get("version"), 1)
        || !is_exact_number(
            source.get("sourceSchemaVersion"),
            PORTABLE_SOURCE_SCHEMA_VERSION,
        )
        || source
            .get("requiresRuntimeFreezeGuard")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err(invalid());
    }
    let finalizations_value = source
        .get("finalizations")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let summary_sources_value = source
        .get("summarySources")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if finalizations_value.len() > MAX_RECORDS || summary_sources_value.len() > MAX_RECORDS {
        return Err(invalid());
    }
    let finalizations: Result<Vec<_>, _> = finalizations_value
        .iter()
        .map(validate_finalization)
        .collect();
    let finalizations = finalizations?;
    let summary_sources: Result<Vec<_>, _> = summary_sources_value
        .iter()
        .map(validate_summary_source)
        .collect();
    let summary_sources = summary_sources?;
    assert_facts(&finalizations, &summary_sources)?;
    let result = PortableTransferAuthority {
        version: 1,
        receipt_id: identifier(source.get("receiptId"))?.to_owned(),
        origin_project_id: uuid(source.get("originProjectId"))?.to_owned(),
        target_project_id: match source.get("targetProjectId") {
            Some(Value::Null) => None,
            Some(value) => Some(uuid(Some(value))?.to_owned()),
            None => return Err(invalid()),
        },
        snapshot_generation: identifier(source.get("snapshotGeneration"))?.to_owned(),
        source_schema_version: PORTABLE_SOURCE_SCHEMA_VERSION,
        portable_database_sha256: hash(source.get("portableDatabaseSha256"))?.to_owned(),
        requires_runtime_freeze_guard: true,
        finalizations,
        summary_sources,
    };
    let expected_receipt_id = receipt_id(&ReceiptInput {
        origin_project_id: &result.origin_project_id,
        snapshot_generation: &result.snapshot_generation,
        portable_database_sha256: &result.portable_database_sha256,
        finalizations: &result.finalizations,
        summary_sources: &result.summary_sources,
    });
    if result.receipt_id != expected_receipt_id {
        return Err(invalid());
    }
    Ok(result)
}

/// 上游严格相等 `source.version !== 1` 的数值判定
/// （JS 中 `1.0 === 1` 为真，故同时接受无小数部分的浮点表示）。
fn is_exact_number(value: Option<&Value>, expected: i64) -> bool {
    match value {
        Some(Value::Number(number)) => {
            number.as_i64() == Some(expected)
                || number
                    .as_f64()
                    .is_some_and(|float| float == expected as f64 && float.fract() == 0.0)
        }
        _ => false,
    }
}

/// `currentness` 的定稿行（上游 `currentness()` 首查的投影行）。
#[derive(Debug)]
struct FinalizationRow {
    finalization_id: Option<String>,
    draft_id: i64,
    chapter_number: Option<i64>,
    content_hash: Option<String>,
    content_snapshot: Option<String>,
    draft_chapter_number: i64,
    draft_status: String,
    body: String,
}

/// 上游 `currentness(database, legacyCopy, freeze?)`：
/// 从项目库提取「当前」定稿与摘要来源事实。
fn currentness(
    database: &Connection,
    legacy_copy: bool,
    freeze: Option<&PortableRuntimeFreezeGuard>,
) -> Result<
    (
        Vec<PortableFinalizationAuthority>,
        Vec<PortableSummarySourceAuthority>,
    ),
    PortableTransferAuthorityError,
> {
    let core: Vec<String> = database
        .prepare("SELECT id FROM project_core")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if core.len() > 1 || core.iter().any(|row| row != "main") {
        return Err(invalid());
    }
    let legacy_clause = if legacy_copy {
        "AND o.finalization_id IS NOT NULL"
    } else {
        ""
    };
    let query = format!(
        "SELECT o.finalization_id AS finalizationId, d.id AS draftId,
      o.chapter_number AS chapterNumber, o.content_hash AS contentHash,
      o.content_snapshot AS contentSnapshot, d.chapter_number AS draftChapterNumber,
      d.status AS draftStatus, c.body AS body
    FROM drafts d
    JOIN contents c ON c.id = d.content_id
    LEFT JOIN finalization_outbox o ON o.draft_id = d.id
    WHERE d.status = 'finalized'
      {legacy_clause}
      AND NOT EXISTS (
        SELECT 1 FROM drafts newer
        WHERE newer.chapter_number = d.chapter_number
          AND newer.status = 'finalized'
          AND (newer.version > d.version OR (newer.version = d.version AND newer.id > d.id))
      )
    ORDER BY o.finalization_id"
    );
    let rows: Vec<FinalizationRow> = database
        .prepare(&query)?
        .query_map([], |row| {
            Ok(FinalizationRow {
                finalization_id: row.get(0)?,
                draft_id: row.get(1)?,
                chapter_number: row.get(2)?,
                content_hash: row.get(3)?,
                content_snapshot: row.get(4)?,
                draft_chapter_number: row.get(5)?,
                draft_status: row.get(6)?,
                body: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let finalizations: Result<Vec<_>, _> = rows
        .into_iter()
        .filter(|row| {
            !(row.finalization_id.is_none()
                && freeze.is_some_and(|guard| {
                    guard.is_frozen(
                        PortableRuntimeFreezeTable::Drafts,
                        &row.draft_id.to_string(),
                    )
                }))
        })
        .map(|row| {
            let projected = validate_finalization(&serde_json::json!({
                "finalizationId": row.finalization_id,
                "draftId": row.draft_id,
                "chapterNumber": row.chapter_number,
                "contentHash": row.content_hash,
            }))?;
            if row.draft_status != "finalized"
                || row.chapter_number != Some(row.draft_chapter_number)
                || row.content_snapshot.as_deref() != Some(row.body.as_str())
                || row.content_snapshot.as_deref().map(sha256_hex) != row.content_hash
            {
                return Err(invalid());
            }
            Ok(projected)
        })
        .collect();
    let finalizations = finalizations?;
    let summary_sources: Vec<PortableSummarySourceAuthority> = database
        .prepare(
            "SELECT s.id AS summaryId, s.draft_id AS draftId, s.chapter_number AS chapterNumber,
      s.source_finalization_id AS sourceFinalizationId, s.source_content_hash AS sourceContentHash,
      s.projection_generation AS projectionGeneration
    FROM summary_snapshots s
    JOIN drafts d ON d.id = s.draft_id
    WHERE (s.source_finalization_id <> '' OR s.source_content_hash <> '')
      AND d.status = 'finalized'
      AND NOT EXISTS (
        SELECT 1 FROM drafts newer
        WHERE newer.chapter_number = d.chapter_number
          AND newer.status = 'finalized'
          AND (newer.version > d.version OR (newer.version = d.version AND newer.id > d.id))
      )
    ORDER BY s.id",
        )?
        .query_map([], |row| {
            Ok(PortableSummarySourceAuthority {
                summary_id: row.get(0)?,
                draft_id: row.get(1)?,
                chapter_number: row.get(2)?,
                source_finalization_id: row.get(3)?,
                source_content_hash: row.get(4)?,
                projection_generation: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    assert_facts(&finalizations, &summary_sources)?;
    Ok((finalizations, summary_sources))
}

/// 上游 `createAuthority(input, facts)`。
fn create_authority(
    origin_project_id: &str,
    snapshot_generation: &str,
    portable_database_sha256: &str,
    finalizations: Vec<PortableFinalizationAuthority>,
    summary_sources: Vec<PortableSummarySourceAuthority>,
) -> Result<PortableTransferAuthority, PortableTransferAuthorityError> {
    let expected_receipt_id = receipt_id(&ReceiptInput {
        origin_project_id,
        snapshot_generation,
        portable_database_sha256,
        finalizations: &finalizations,
        summary_sources: &summary_sources,
    });
    parse_portable_transfer_authority(&serde_json::json!({
        "version": 1,
        "originProjectId": origin_project_id,
        "targetProjectId": null,
        "snapshotGeneration": snapshot_generation,
        "sourceSchemaVersion": PORTABLE_SOURCE_SCHEMA_VERSION,
        "portableDatabaseSha256": portable_database_sha256,
        "requiresRuntimeFreezeGuard": true,
        "finalizations": finalizations,
        "summarySources": summary_sources,
        "receiptId": expected_receipt_id,
    }))
}

/// `createPortableTransferAuthority` 输入（上游对象参数）。
pub struct CreatePortableTransferAuthorityInput<'a> {
    /// 运行时冻结守卫（可选；冻结的已删除草稿不参与当前事实）。
    pub freeze: Option<&'a PortableRuntimeFreezeGuard>,
    pub database: &'a Connection,
    pub origin_project_id: &'a str,
    pub snapshot_generation: &'a str,
    pub portable_database_sha256: &'a str,
}

/// 上游 `createPortableTransferAuthority(input)`。
pub fn create_portable_transfer_authority(
    input: CreatePortableTransferAuthorityInput<'_>,
) -> Result<PortableTransferAuthority, PortableTransferAuthorityError> {
    let (finalizations, summary_sources) = currentness(input.database, false, input.freeze)?;
    create_authority(
        input.origin_project_id,
        input.snapshot_generation,
        input.portable_database_sha256,
        finalizations,
        summary_sources,
    )
}

/// `createLegacyCopyTransferAuthority` 输入（上游对象参数，无冻结守卫）。
pub struct LegacyCopyTransferAuthorityInput<'a> {
    pub database: &'a Connection,
    pub origin_project_id: &'a str,
    pub snapshot_generation: &'a str,
    pub portable_database_sha256: &'a str,
}

/// 上游 `createLegacyCopyTransferAuthority(input)`：
/// 「旧定稿正文无 outbox 收据仍为可读 legacy 内容；
/// 仅源绑定收据成为转移权威。」
pub fn create_legacy_copy_transfer_authority(
    input: LegacyCopyTransferAuthorityInput<'_>,
) -> Result<PortableTransferAuthority, PortableTransferAuthorityError> {
    let (finalizations, summary_sources) = currentness(input.database, true, None)?;
    create_authority(
        input.origin_project_id,
        input.snapshot_generation,
        input.portable_database_sha256,
        finalizations,
        summary_sources,
    )
}

/// 上游 `mapPortableTransferAuthority(authority, targetProjectId)`。
pub fn map_portable_transfer_authority(
    authority: PortableTransferAuthority,
    target_project_id: &str,
) -> Result<PortableTransferAuthority, PortableTransferAuthorityError> {
    let mut value = serde_json::to_value(&authority).map_err(|_| invalid())?;
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "targetProjectId".to_owned(),
            serde_json::json!(target_project_id),
        );
    }
    parse_portable_transfer_authority(&value)
}

/// 上游 `verifyPortableTransferAuthority(database, authority, freeze?)`。
pub fn verify_portable_transfer_authority(
    database: &Connection,
    authority: &PortableTransferAuthority,
    freeze: Option<&PortableRuntimeFreezeGuard>,
) -> Result<(), PortableTransferAuthorityError> {
    let (finalizations, summary_sources) = currentness(database, false, freeze)?;
    if serde_json::to_string(&finalizations).map_err(|_| invalid())?
        != serde_json::to_string(&authority.finalizations).map_err(|_| invalid())?
        || serde_json::to_string(&summary_sources).map_err(|_| invalid())?
            != serde_json::to_string(&authority.summary_sources).map_err(|_| invalid())?
    {
        return Err(invalid());
    }
    Ok(())
}

/// `verifyPortableTransferAuthorityHistory` 的定稿行（上游逐列投影）。
#[derive(Debug)]
struct HistoryFinalizationRow {
    status: String,
    chapter_number: i64,
    body: String,
    finalization_id: Option<String>,
    content_hash: Option<String>,
    content_snapshot: Option<String>,
}

/// `verifyPortableTransferAuthorityHistory` 的摘要来源行（上游逐列投影）。
#[derive(Debug)]
struct HistorySummarySourceRow {
    summary_id: i64,
    draft_id: i64,
    chapter_number: i64,
    source_finalization_id: String,
    source_content_hash: String,
    projection_generation: i64,
}

/// 上游 `verifyPortableTransferAuthorityHistory(database, authority, freeze?)`：
/// 「原始导出元组是不可变历史；允许更新的定稿版本。」
pub fn verify_portable_transfer_authority_history(
    database: &Connection,
    authority: &PortableTransferAuthority,
    freeze: Option<&PortableRuntimeFreezeGuard>,
) -> Result<(), PortableTransferAuthorityError> {
    let mut deleted: HashSet<i64> = HashSet::new();
    for item in &authority.finalizations {
        let row = database
            .prepare(
                "SELECT d.status AS status,d.chapter_number AS chapterNumber,c.body AS body,
      o.finalization_id AS finalizationId,o.content_hash AS contentHash,o.content_snapshot AS contentSnapshot
    FROM drafts d JOIN contents c ON c.id=d.content_id
    LEFT JOIN finalization_outbox o ON o.draft_id=d.id WHERE d.id=?",
            )?
            .query_row(rusqlite::params![item.draft_id], |row| {
                Ok(HistoryFinalizationRow {
                    status: row.get(0)?,
                    chapter_number: row.get(1)?,
                    body: row.get(2)?,
                    finalization_id: row.get(3)?,
                    content_hash: row.get(4)?,
                    content_snapshot: row.get(5)?,
                })
            })
            .optional()?;
        if row.is_none()
            && freeze.is_some_and(|guard| guard.active())
            && database
                .prepare("SELECT 1 FROM drafts WHERE id=?")?
                .query_row([item.draft_id], |_| Ok(()))
                .optional()?
                .is_none()
        {
            let deletion = database
                .prepare(
                    "SELECT operation_id AS operationId FROM chapter_deletion_operations
      WHERE draft_id=? AND chapter_number=? AND finalization_id=?
        AND status IN ('pending','failed','completed')
        AND (legacy_knowledge_authorization='not_required'
          OR (legacy_knowledge_authorization='consumed' AND legacy_knowledge_authorized_at <> ''))",
                )?
                .query_row(
                    rusqlite::params![item.draft_id, item.chapter_number, item.finalization_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            if let Some(operation_id) = deletion {
                if !freeze.expect("freeze active").is_frozen(
                    PortableRuntimeFreezeTable::ChapterDeletionOperations,
                    &operation_id,
                ) {
                    deleted.insert(item.draft_id);
                    continue;
                }
            }
        }
        let row = row.ok_or_else(invalid)?;
        if row.status != "finalized"
            || row.chapter_number != item.chapter_number
            || row.finalization_id.as_deref() != Some(item.finalization_id.as_str())
            || row.content_hash.as_deref() != Some(item.content_hash.as_str())
            || row.content_snapshot.as_deref() != Some(row.body.as_str())
            || sha256_hex(&row.body) != item.content_hash
        {
            return Err(invalid());
        }
    }
    for item in &authority.summary_sources {
        let row = database
            .prepare(
                "SELECT id AS summaryId,draft_id AS draftId,chapter_number AS chapterNumber,
      source_finalization_id AS sourceFinalizationId,source_content_hash AS sourceContentHash,
      projection_generation AS projectionGeneration
    FROM summary_snapshots WHERE id=?",
            )?
            .query_row([item.summary_id], |row| {
                Ok(HistorySummarySourceRow {
                    summary_id: row.get(0)?,
                    draft_id: row.get(1)?,
                    chapter_number: row.get(2)?,
                    source_finalization_id: row.get(3)?,
                    source_content_hash: row.get(4)?,
                    projection_generation: row.get(5)?,
                })
            })
            .optional()?;
        if row.is_none() && deleted.contains(&item.draft_id) {
            continue;
        }
        let row = row.ok_or_else(invalid)?;
        if row.summary_id != item.summary_id
            || row.draft_id != item.draft_id
            || row.chapter_number != item.chapter_number
            || row.source_finalization_id != item.source_finalization_id
            || row.source_content_hash != item.source_content_hash
            || row.projection_generation != item.projection_generation
        {
            return Err(invalid());
        }
    }
    Ok(())
}

/// 上游 `serializePortableTransferAuthority(authority)`：
/// 规范 JSON 字节（≤ `MAX_BYTES`）。
pub fn serialize_portable_transfer_authority(
    authority: &PortableTransferAuthority,
) -> Result<Vec<u8>, PortableTransferAuthorityError> {
    let reparsed = parse_portable_transfer_authority(
        &serde_json::to_value(authority).map_err(|_| invalid())?,
    )?;
    let bytes = serde_json::to_vec(&reparsed).map_err(|_| invalid())?;
    if bytes.len() > MAX_BYTES {
        return Err(invalid());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use serde_json::json;

    const ORIGIN_PROJECT_ID: &str = "123e4567-e89b-12d3-a456-426614174000";
    const TARGET_PROJECT_ID: &str = "987e6543-e21b-42d3-a456-426614174000";
    const SNAPSHOT_GENERATION: &str = "gen.1";

    fn setup_database() -> Connection {
        let database = Connection::open_in_memory().expect("in-memory database");
        database
            .execute_batch(
                r#"
                CREATE TABLE project_core (id TEXT PRIMARY KEY);
                CREATE TABLE contents (id INTEGER PRIMARY KEY, body TEXT NOT NULL);
                CREATE TABLE drafts (
                    id INTEGER PRIMARY KEY,
                    content_id INTEGER NOT NULL,
                    chapter_number INTEGER NOT NULL,
                    version INTEGER NOT NULL,
                    status TEXT NOT NULL
                );
                CREATE TABLE finalization_outbox (
                    draft_id INTEGER NOT NULL,
                    finalization_id TEXT NOT NULL,
                    chapter_number INTEGER NOT NULL,
                    content_hash TEXT NOT NULL,
                    content_snapshot TEXT NOT NULL
                );
                CREATE TABLE summary_snapshots (
                    id INTEGER PRIMARY KEY,
                    draft_id INTEGER NOT NULL,
                    chapter_number INTEGER NOT NULL,
                    source_finalization_id TEXT NOT NULL,
                    source_content_hash TEXT NOT NULL,
                    projection_generation INTEGER NOT NULL
                );
                CREATE TABLE chapter_deletion_operations (
                    operation_id TEXT PRIMARY KEY,
                    draft_id INTEGER NOT NULL,
                    chapter_number INTEGER NOT NULL,
                    finalization_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    legacy_knowledge_authorization TEXT NOT NULL,
                    legacy_knowledge_authorized_at TEXT NOT NULL
                );
                INSERT INTO project_core (id) VALUES ('main');
                "#,
            )
            .expect("schema created");
        database
    }

    fn insert_finalized(
        database: &Connection,
        draft_id: i64,
        chapter_number: i64,
        body: &str,
        finalization_id: &str,
    ) {
        let content_hash = sha256_hex(body);
        database
            .execute(
                "INSERT INTO contents (id, body) VALUES (?1, ?2)",
                params![draft_id, body],
            )
            .expect("contents row inserted");
        database
            .execute(
                "INSERT INTO drafts (id, content_id, chapter_number, version, status) VALUES (?1, ?2, ?3, 1, 'finalized')",
                params![draft_id, draft_id, chapter_number],
            )
            .expect("drafts row inserted");
        database
            .execute(
                "INSERT INTO finalization_outbox (draft_id, finalization_id, chapter_number, content_hash, content_snapshot) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![draft_id, finalization_id, chapter_number, content_hash, body],
            )
            .expect("outbox row inserted");
    }

    fn sample_authority(database: &Connection) -> PortableTransferAuthority {
        insert_finalized(database, 1, 1, "正文", "fin.1");
        create_portable_transfer_authority(CreatePortableTransferAuthorityInput {
            freeze: None,
            database,
            origin_project_id: ORIGIN_PROJECT_ID,
            snapshot_generation: SNAPSHOT_GENERATION,
            portable_database_sha256: &"b".repeat(64),
        })
        .expect("authority created")
    }

    /// 含一条冻结记录的激活守卫（上游 `readPortableRuntimeFreeze` 产物）。
    fn freeze_guard_with(table: &str, record_id: &str) -> PortableRuntimeFreezeGuard {
        let document = format!(
            r#"{{"version":1,"originProjectId":"{ORIGIN_PROJECT_ID}","snapshotGeneration":"{SNAPSHOT_GENERATION}","nonReplayable":true,"requiresRuntimeFreezeGuard":true,"records":[{{"projectionId":"proj.1","table":"{table}","recordId":"{record_id}","terminalState":"term.1","projection":{{}},"projectionHash":"{}","excludedFields":[],"nonReplayable":true,"originalReceiptVerified":false}}],"avatarReferenceProjections":[]}}"#,
            "a".repeat(64),
        );
        crate::services::portable_runtime_freeze::parse_portable_runtime_freeze(document.as_bytes())
            .expect("freeze guard")
    }

    /// 无冻结记录的激活守卫（`records: []`）。
    fn empty_freeze_guard() -> PortableRuntimeFreezeGuard {
        let document = format!(
            r#"{{"version":1,"originProjectId":"{ORIGIN_PROJECT_ID}","snapshotGeneration":"{SNAPSHOT_GENERATION}","nonReplayable":true,"requiresRuntimeFreezeGuard":true,"records":[],"avatarReferenceProjections":[]}}"#
        );
        crate::services::portable_runtime_freeze::parse_portable_runtime_freeze(document.as_bytes())
            .expect("empty freeze guard")
    }

    #[test]
    fn constants_match_upstream() {
        assert_eq!(PORTABLE_SOURCE_SCHEMA_VERSION, 7);
        assert_eq!(
            PORTABLE_TRANSFER_AUTHORITY_PATH,
            ".ai-novel/portable-transfer-authority.json"
        );
        assert_eq!(
            PortableTransferAuthorityError.code(),
            "PORTABLE_TRANSFER_AUTHORITY_INVALID"
        );
        assert_eq!(
            PortableTransferAuthorityError.to_string(),
            "PORTABLE_TRANSFER_AUTHORITY_INVALID"
        );
    }

    #[test]
    fn parse_round_trips_created_authority() {
        let database = setup_database();
        let authority = sample_authority(&database);
        let value = serde_json::to_value(&authority).expect("to value");
        assert_eq!(parse_portable_transfer_authority(&value), Ok(authority));
    }

    #[test]
    fn parse_rejects_tampered_receipt_and_unknown_keys() {
        let database = setup_database();
        let authority = sample_authority(&database);

        let mut value = serde_json::to_value(&authority).expect("to value");
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "receiptId".to_owned(),
                json!("portable-transfer:00000000000000000000000000000000"),
            );
        }
        assert_eq!(parse_portable_transfer_authority(&value), Err(invalid()));

        let mut value = serde_json::to_value(&authority).expect("to value");
        if let Some(object) = value.as_object_mut() {
            object.insert("extra".to_owned(), json!(1));
        }
        assert_eq!(parse_portable_transfer_authority(&value), Err(invalid()));

        let mut value = serde_json::to_value(&authority).expect("to value");
        if let Some(object) = value.as_object_mut() {
            object.insert("version".to_owned(), json!(2));
        }
        assert_eq!(parse_portable_transfer_authority(&value), Err(invalid()));

        let mut value = serde_json::to_value(&authority).expect("to value");
        if let Some(object) = value.as_object_mut() {
            object.insert("sourceSchemaVersion".to_owned(), json!(6));
        }
        assert_eq!(parse_portable_transfer_authority(&value), Err(invalid()));
    }

    #[test]
    fn parse_rejects_duplicate_finalization_identities() {
        let finalizations = vec![
            PortableFinalizationAuthority {
                finalization_id: "fin.1".to_owned(),
                draft_id: 1,
                chapter_number: 1,
                content_hash: "a".repeat(64),
            },
            PortableFinalizationAuthority {
                finalization_id: "fin.2".to_owned(),
                draft_id: 1,
                chapter_number: 2,
                content_hash: "b".repeat(64),
            },
        ];
        let expected_receipt_id = receipt_id(&ReceiptInput {
            origin_project_id: ORIGIN_PROJECT_ID,
            snapshot_generation: SNAPSHOT_GENERATION,
            portable_database_sha256: &"c".repeat(64),
            finalizations: &finalizations,
            summary_sources: &[],
        });
        let value = json!({
            "version": 1,
            "receiptId": expected_receipt_id,
            "originProjectId": ORIGIN_PROJECT_ID,
            "targetProjectId": null,
            "snapshotGeneration": SNAPSHOT_GENERATION,
            "sourceSchemaVersion": PORTABLE_SOURCE_SCHEMA_VERSION,
            "portableDatabaseSha256": "c".repeat(64),
            "requiresRuntimeFreezeGuard": true,
            "finalizations": finalizations,
            "summarySources": []
        });
        assert_eq!(parse_portable_transfer_authority(&value), Err(invalid()));
    }

    #[test]
    fn parse_rejects_summary_source_without_matching_finalization() {
        let finalizations = vec![PortableFinalizationAuthority {
            finalization_id: "fin.1".to_owned(),
            draft_id: 1,
            chapter_number: 1,
            content_hash: "a".repeat(64),
        }];
        let summary_sources = vec![PortableSummarySourceAuthority {
            summary_id: 1,
            draft_id: 1,
            chapter_number: 1,
            source_finalization_id: "fin.missing".to_owned(),
            source_content_hash: "a".repeat(64),
            projection_generation: 1,
        }];
        let expected_receipt_id = receipt_id(&ReceiptInput {
            origin_project_id: ORIGIN_PROJECT_ID,
            snapshot_generation: SNAPSHOT_GENERATION,
            portable_database_sha256: &"c".repeat(64),
            finalizations: &finalizations,
            summary_sources: &summary_sources,
        });
        let value = json!({
            "version": 1,
            "receiptId": expected_receipt_id,
            "originProjectId": ORIGIN_PROJECT_ID,
            "targetProjectId": null,
            "snapshotGeneration": SNAPSHOT_GENERATION,
            "sourceSchemaVersion": PORTABLE_SOURCE_SCHEMA_VERSION,
            "portableDatabaseSha256": "c".repeat(64),
            "requiresRuntimeFreezeGuard": true,
            "finalizations": finalizations,
            "summarySources": summary_sources
        });
        assert_eq!(parse_portable_transfer_authority(&value), Err(invalid()));
    }

    #[test]
    fn create_and_verify_accept_unchanged_database() {
        let database = setup_database();
        let authority = sample_authority(&database);
        assert_eq!(
            verify_portable_transfer_authority(&database, &authority, None),
            Ok(())
        );
        assert_eq!(
            verify_portable_transfer_authority_history(&database, &authority, None),
            Ok(())
        );
    }

    #[test]
    fn verify_rejects_divergent_database() {
        let database = setup_database();
        let authority = sample_authority(&database);
        database
            .execute(
                "UPDATE finalization_outbox SET content_hash = ?1 WHERE draft_id = 1",
                params!["x".repeat(64)],
            )
            .expect("outbox tampered");
        assert!(verify_portable_transfer_authority(&database, &authority, None).is_err());
        assert!(verify_portable_transfer_authority_history(&database, &authority, None).is_err());
    }

    #[test]
    fn create_includes_summary_sources() {
        let database = setup_database();
        insert_finalized(&database, 1, 1, "正文", "fin.1");
        database
            .execute(
                "INSERT INTO summary_snapshots (id, draft_id, chapter_number, source_finalization_id, source_content_hash, projection_generation) VALUES (1, 1, 1, 'fin.1', ?1, 1)",
                params![sha256_hex("正文")],
            )
            .expect("summary row inserted");
        let authority = create_portable_transfer_authority(CreatePortableTransferAuthorityInput {
            freeze: None,
            database: &database,
            origin_project_id: ORIGIN_PROJECT_ID,
            snapshot_generation: SNAPSHOT_GENERATION,
            portable_database_sha256: &"b".repeat(64),
        })
        .expect("authority with summary sources");
        assert_eq!(authority.summary_sources.len(), 1);
        assert_eq!(authority.summary_sources[0].summary_id, 1);
        assert_eq!(authority.summary_sources[0].source_finalization_id, "fin.1");
        assert_eq!(
            verify_portable_transfer_authority(&database, &authority, None),
            Ok(())
        );
        assert_eq!(
            verify_portable_transfer_authority_history(&database, &authority, None),
            Ok(())
        );
    }

    #[test]
    fn create_excludes_frozen_deleted_drafts() {
        let database = setup_database();
        // 定稿草稿无 outbox 收据：非冻结时当前事实非法。
        database
            .execute(
                "INSERT INTO contents (id, body) VALUES (?, ?)",
                params![1, "正文"],
            )
            .expect("contents row inserted");
        database
            .execute(
                "INSERT INTO drafts (id, content_id, chapter_number, version, status) VALUES (?, ?, ?, ?, ?)",
                params![1, 1, 1, 1, "finalized"],
            )
            .expect("drafts row inserted");
        let create = |freeze: Option<&PortableRuntimeFreezeGuard>| {
            create_portable_transfer_authority(CreatePortableTransferAuthorityInput {
                freeze,
                database: &database,
                origin_project_id: ORIGIN_PROJECT_ID,
                snapshot_generation: SNAPSHOT_GENERATION,
                portable_database_sha256: &"b".repeat(64),
            })
        };
        assert!(create(None).is_err());
        let frozen = freeze_guard_with("drafts", "1");
        let authority = create(Some(&frozen)).expect("frozen draft excluded");
        assert!(authority.finalizations.is_empty());
        assert_eq!(
            verify_portable_transfer_authority(&database, &authority, Some(&frozen)),
            Ok(())
        );
    }

    #[test]
    fn legacy_copy_only_includes_receipted_drafts() {
        let database = setup_database();
        insert_finalized(&database, 1, 1, "正文", "fin.1");
        // 无收据的 legacy 定稿草稿。
        database
            .execute(
                "INSERT INTO contents (id, body) VALUES (?, ?)",
                params![2, "旧正文"],
            )
            .expect("contents row inserted");
        database
            .execute(
                "INSERT INTO drafts (id, content_id, chapter_number, version, status) VALUES (?, ?, ?, ?, ?)",
                params![2, 2, 2, 1, "finalized"],
            )
            .expect("drafts row inserted");
        let legacy = create_legacy_copy_transfer_authority(LegacyCopyTransferAuthorityInput {
            database: &database,
            origin_project_id: ORIGIN_PROJECT_ID,
            snapshot_generation: SNAPSHOT_GENERATION,
            portable_database_sha256: &"b".repeat(64),
        })
        .expect("legacy copy");
        assert_eq!(legacy.finalizations.len(), 1);
        assert_eq!(legacy.finalizations[0].finalization_id, "fin.1");
    }

    #[test]
    fn map_updates_target_project_id_only() {
        let database = setup_database();
        let authority = sample_authority(&database);
        let mapped =
            map_portable_transfer_authority(authority.clone(), TARGET_PROJECT_ID).expect("mapped");
        assert_eq!(mapped.target_project_id, Some(TARGET_PROJECT_ID.to_owned()));
        // 收据哈希输入不含 targetProjectId，故收据不变。
        assert_eq!(mapped.receipt_id, authority.receipt_id);
        assert_eq!(mapped.origin_project_id, authority.origin_project_id);
        assert_eq!(mapped.finalizations, authority.finalizations);
    }

    #[test]
    fn serialize_round_trips_within_byte_limit() {
        let database = setup_database();
        let authority = sample_authority(&database);
        let bytes = serialize_portable_transfer_authority(&authority).expect("serialized");
        assert!(bytes.len() <= MAX_BYTES);
        let reparsed =
            parse_portable_transfer_authority(&serde_json::from_slice(&bytes).expect("json value"))
                .expect("reparsed");
        assert_eq!(reparsed, authority);
    }

    #[test]
    fn history_allows_unfrozen_deletion_operations() {
        let database = setup_database();
        let authority = sample_authority(&database);
        // 删除草稿及其收据（上游场景：定稿已删除但权威元组仍在导出历史中）。
        database
            .execute(
                "DELETE FROM finalization_outbox WHERE draft_id = 1",
                params![],
            )
            .expect("outbox deleted");
        database
            .execute("DELETE FROM drafts WHERE id = 1", params![])
            .expect("drafts deleted");
        database
            .execute("DELETE FROM contents WHERE id = 1", params![])
            .expect("contents deleted");
        // 无冻结守卫：草稿消失即历史不一致。
        assert!(verify_portable_transfer_authority_history(&database, &authority, None).is_err());
        // 删除操作已登记但未被冻结：视为合法删除，跳过该元组。
        database
            .execute(
                "INSERT INTO chapter_deletion_operations (operation_id, draft_id, chapter_number, finalization_id, status, legacy_knowledge_authorization, legacy_knowledge_authorized_at) VALUES ('op.1', 1, 1, 'fin.1', 'completed', 'not_required', '')",
                params![],
            )
            .expect("deletion operation inserted");
        let freeze = empty_freeze_guard();
        assert_eq!(
            verify_portable_transfer_authority_history(&database, &authority, Some(&freeze)),
            Ok(())
        );
        // 删除操作被冻结：不得视为已删除，历史校验失败。
        let frozen = freeze_guard_with("chapter_deletion_operations", "op.1");
        assert!(
            verify_portable_transfer_authority_history(&database, &authority, Some(&frozen))
                .is_err()
        );
    }

    #[test]
    fn history_rejects_unknown_summary_source_tampering() {
        let database = setup_database();
        insert_finalized(&database, 1, 1, "正文", "fin.1");
        database
            .execute(
                "INSERT INTO summary_snapshots (id, draft_id, chapter_number, source_finalization_id, source_content_hash, projection_generation) VALUES (1, 1, 1, 'fin.1', ?1, 1)",
                params![sha256_hex("正文")],
            )
            .expect("summary row inserted");
        let authority = create_portable_transfer_authority(CreatePortableTransferAuthorityInput {
            freeze: None,
            database: &database,
            origin_project_id: ORIGIN_PROJECT_ID,
            snapshot_generation: SNAPSHOT_GENERATION,
            portable_database_sha256: &"b".repeat(64),
        })
        .expect("authority");
        assert_eq!(
            verify_portable_transfer_authority_history(&database, &authority, None),
            Ok(())
        );
        database
            .execute(
                "UPDATE summary_snapshots SET projection_generation = 2 WHERE id = 1",
                params![],
            )
            .expect("summary tampered");
        assert!(verify_portable_transfer_authority_history(&database, &authority, None).is_err());
    }
}
