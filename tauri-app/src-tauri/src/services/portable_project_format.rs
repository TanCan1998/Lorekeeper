//! 便携项目格式：manifest 契约 + 路径规范键 + 归档限额。
//!
//! 上游事实源：`electron/services/portable-project-format.ts`（manifest 部分）。
//! 本模块同时保留上游目录常量 [`CANONICAL_PROJECT_DIRECTORY`]（`.ai-novel`）
//! 作上游对齐：本项目存储根是 `.lore`（[`crate::db::PROJECT_DIR_NAME`]），G4-1 波次模块
//! 已陆续切到该口径，禁止再用来拼存储根路径。
//!
//! 迁移口径（1:1）：
//! - 错误一律以基线错误码字符串上浮（`PORTABLE_*`），由命令层折叠进契约判别联合；
//! - `fieldPolicy` / `assertPortableSourceSchema`（字段策略表与 schema 对账）属于便携策略面，
//!   依赖 `portable-project-field-policy.json` 与桌面迁移注册表，随 B112 导出切片一并落地；
//! - 基线 `containsSurrogateCodeUnit` 在 Rust 中恒为假（`&str` 保证合法 UTF-8、无孤立代理码元），
//!   已删除该分支并在路径注释中说明；`String.prototype.normalize('NFKC')` 由
//!   `unicode-normalization` 等价实现。
//!
//! 纯逻辑、无 Tauri 依赖。

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use unicode_normalization::UnicodeNormalization;

/// 上游 `CANONICAL_PROJECT_DIRECTORY`（值 `.ai-novel`，常量值保持逐字一致）。
///
/// 本项目项目存储根是 `<项目根>/.lore`（[`crate::db::PROJECT_DIR_NAME`]，L3 双栈隔离）；
/// 便携归档条目一律相对存储根、不带前缀，故本常量**不再参与任何路径拼接**。
pub const CANONICAL_PROJECT_DIRECTORY: &str = ".ai-novel";

/// 便携归档格式版本（上游 `PORTABLE_PROJECT_FORMAT_VERSION`）
pub const PORTABLE_PROJECT_FORMAT_VERSION: u32 = 1;
/// 便携归档的源 schema 版本（上游 `PORTABLE_SOURCE_SCHEMA_VERSION`；
/// 与 `portable_transfer_authority::PORTABLE_SOURCE_SCHEMA_VERSION` 同源同值）
pub const PORTABLE_SOURCE_SCHEMA_VERSION: u32 = 7;

/// 归档条目处置（上游 `PORTABLE_ENTRY_DISPOSITIONS`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PortableEntryDisposition {
    #[serde(rename = "portable-database")]
    PortableDatabase,
    #[serde(rename = "author-content")]
    AuthorContent,
    #[serde(rename = "knowledge-source")]
    KnowledgeSource,
    #[serde(rename = "prompt")]
    Prompt,
    #[serde(rename = "skill")]
    Skill,
    #[serde(rename = "avatar-asset")]
    AvatarAsset,
    #[serde(rename = "transfer-receipt")]
    TransferReceipt,
    #[serde(rename = "history-projection")]
    HistoryProjection,
}

impl PortableEntryDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PortableDatabase => "portable-database",
            Self::AuthorContent => "author-content",
            Self::KnowledgeSource => "knowledge-source",
            Self::Prompt => "prompt",
            Self::Skill => "skill",
            Self::AvatarAsset => "avatar-asset",
            Self::TransferReceipt => "transfer-receipt",
            Self::HistoryProjection => "history-projection",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "portable-database" => Self::PortableDatabase,
            "author-content" => Self::AuthorContent,
            "knowledge-source" => Self::KnowledgeSource,
            "prompt" => Self::Prompt,
            "skill" => Self::Skill,
            "avatar-asset" => Self::AvatarAsset,
            "transfer-receipt" => Self::TransferReceipt,
            "history-projection" => Self::HistoryProjection,
            _ => return None,
        })
    }
}

/// 省略原因（上游 `PORTABLE_OMISSION_REASONS`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PortableOmissionReason {
    #[serde(rename = "exclude-machine-authority")]
    ExcludeMachineAuthority,
    #[serde(rename = "rebuild-stale")]
    RebuildStale,
    #[serde(rename = "historical-nonreplayable")]
    HistoricalNonreplayable,
    #[serde(rename = "unsupported-safe-projection")]
    UnsupportedSafeProjection,
}

impl PortableOmissionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExcludeMachineAuthority => "exclude-machine-authority",
            Self::RebuildStale => "rebuild-stale",
            Self::HistoricalNonreplayable => "historical-nonreplayable",
            Self::UnsupportedSafeProjection => "unsupported-safe-projection",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "exclude-machine-authority" => Self::ExcludeMachineAuthority,
            "rebuild-stale" => Self::RebuildStale,
            "historical-nonreplayable" => Self::HistoricalNonreplayable,
            "unsupported-safe-projection" => Self::UnsupportedSafeProjection,
            _ => return None,
        })
    }
}

/// 上游 `PortableProjectManifestEntry`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableProjectManifestEntry {
    pub path: String,
    pub byte_size: u64,
    pub sha256: String,
    pub disposition: PortableEntryDisposition,
}

/// 上游 `PortableProjectManifest`（字段顺序与 upstream 一致，便于 stableJson 写入）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableProjectManifest {
    pub format_version: u32,
    pub source_schema_version: u32,
    pub origin_project_id: String,
    pub snapshot_generation: String,
    pub created_at: String,
    pub declared_uncompressed_bytes: u64,
    pub declared_compressed_bytes: u64,
    pub entries: Vec<PortableProjectManifestEntry>,
    pub semantic_counts: BTreeMap<String, u64>,
    pub omitted_items: Vec<PortableOmissionItem>,
    pub transfer_receipt_ids: Vec<String>,
    pub history_projection_ids: Vec<String>,
}

/// 上游 `omittedItems` 元素
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableOmissionItem {
    pub id: String,
    pub reason: PortableOmissionReason,
}

/// 上游 `PortableArchiveLimits`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortableArchiveLimits {
    pub max_entries: usize,
    pub max_entry_bytes: u64,
    pub max_total_bytes: u64,
    pub max_declared_compressed_bytes: u64,
    pub max_compression_ratio: u64,
    pub max_path_bytes: usize,
    pub max_semantic_counts: usize,
    pub max_semantic_count_value: u64,
    pub max_omitted_items: usize,
    pub max_reference_ids: usize,
}

const GIB: u64 = 1024 * 1024 * 1024;

impl Default for PortableArchiveLimits {
    /// 上游 `DEFAULT_PORTABLE_ARCHIVE_LIMITS`
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_entry_bytes: 2 * GIB,
            max_total_bytes: 16 * GIB,
            max_declared_compressed_bytes: 16 * GIB,
            max_compression_ratio: 200,
            max_path_bytes: 1024,
            max_semantic_counts: 256,
            max_semantic_count_value: 1_000_000_000,
            max_omitted_items: 100_000,
            max_reference_ids: 100_000,
        }
    }
}

/// 上游 `DEFAULT_PORTABLE_ARCHIVE_LIMITS`（供调用方引用）
pub fn default_portable_archive_limits() -> PortableArchiveLimits {
    PortableArchiveLimits::default()
}

fn manifest_invalid() -> String {
    "PORTABLE_MANIFEST_INVALID".to_string()
}

fn path_unsafe() -> String {
    "PORTABLE_PATH_UNSAFE".to_string()
}

fn limit_exceeded() -> String {
    "PORTABLE_ARCHIVE_LIMIT_EXCEEDED".to_string()
}

/// 上游 `record`：非对象（含 null / 数组）一律无效
fn record(value: &Value) -> Result<&Map<String, Value>, String> {
    value.as_object().ok_or_else(manifest_invalid)
}

/// 上游 `exactKeys`：键集合必须完全一致（键名均为 ASCII，排序语义与 JS 一致）
fn exact_keys(value: &Map<String, Value>, expected: &[&str]) -> Result<(), String> {
    let mut actual: Vec<&str> = value.keys().map(|key| key.as_str()).collect();
    actual.sort_unstable();
    let mut sorted_expected: Vec<&str> = expected.to_vec();
    sorted_expected.sort_unstable();
    if actual != sorted_expected {
        return Err(manifest_invalid());
    }
    Ok(())
}

/// 上游 `boundedInteger`：安全整数且落在 `0..=maximum`
fn bounded_integer(value: &Value, maximum: u64) -> Result<u64, String> {
    let number = value.as_u64().ok_or_else(limit_exceeded)?;
    if number > maximum {
        return Err(limit_exceeded());
    }
    Ok(number)
}

/// 上游 `identifier`：非空、UTF-16 码元长度 ≤ 512、不含 NUL
fn identifier(value: &Value) -> Result<String, String> {
    let text = value.as_str().ok_or_else(manifest_invalid)?;
    let units = text.encode_utf16().count();
    if units == 0 || units > 512 || text.contains('\0') {
        return Err(manifest_invalid());
    }
    Ok(text.to_string())
}

/// JS `Number` 相等（`1.0 === 1`）：`formatVersion` / `sourceSchemaVersion` 用
fn equals_number(value: &Value, expected: f64) -> bool {
    value
        .as_f64()
        .map(|number| number == expected)
        .unwrap_or(false)
}

/// 严格 ISO 8601 UTC 毫秒串（等价上游 `new Date(v).toISOString() === v` 的往返判定）
pub(crate) fn is_exact_iso8601_millis(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 24 {
        return false;
    }
    let digits_at = |start: usize, count: usize| -> Option<u32> {
        // 用字节切片取数：长度与分隔符检查后仍可能含非 ASCII 字节，
        // 直接按字节下标切 `&str` 会 panic。
        let slice = bytes.get(start..start + count)?;
        if !slice.iter().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        std::str::from_utf8(slice).ok()?.parse::<u32>().ok()
    };
    if !(bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'.'
        && bytes[23] == b'Z')
    {
        return false;
    }
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second), Some(millis)) = (
        digits_at(0, 4),
        digits_at(5, 2),
        digits_at(8, 2),
        digits_at(11, 2),
        digits_at(14, 2),
        digits_at(17, 2),
        digits_at(20, 3),
    ) else {
        return false;
    };
    if month == 0 || month > 12 || hour > 23 || minute > 59 || second > 59 || millis > 999 {
        return false;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    day >= 1 && day <= days
}

/// Windows 保留设备名（与 `external_grant::is_windows_reserved_name` 同源实现，
/// 此处按上游 `WINDOWS_RESERVED` 正则语义复用同一函数）
fn is_windows_reserved_name(segment: &str) -> bool {
    crate::external_grant::is_windows_reserved_name(segment)
}

/// 上游 `portablePathKey`：NFKC 归一化 → 安全校验 → 小写化分段键
pub fn portable_path_key(value: &str, max_path_bytes: usize) -> Result<String, String> {
    // 基线 `Buffer.byteLength(value, 'utf8')`；`&str` 恒为合法 UTF-8，故"孤立代理码元"分支不存在
    if value.is_empty() || value.len() > max_path_bytes {
        return Err(path_unsafe());
    }
    let normalized: String = value.nfkc().collect();
    if normalized.starts_with('/') || normalized.contains('\\') {
        return Err(path_unsafe());
    }
    let bytes = normalized.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(path_unsafe());
    }
    let mut keys: Vec<String> = Vec::new();
    for segment in normalized.split('/') {
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.ends_with('.')
            || segment.ends_with(' ')
            || segment
                .chars()
                .any(|character| "<>:\"|?*".contains(character))
            || segment.chars().any(|character| (character as u32) <= 31)
            || is_windows_reserved_name(segment)
        {
            return Err(path_unsafe());
        }
        keys.push(segment.to_lowercase());
    }
    Ok(keys.join("/"))
}

fn resolved_limits(
    configured: Option<&PortableArchiveLimits>,
) -> Result<PortableArchiveLimits, String> {
    let Some(limits) = configured else {
        return Ok(PortableArchiveLimits::default());
    };
    let all: [u64; 10] = [
        limits.max_entries as u64,
        limits.max_entry_bytes,
        limits.max_total_bytes,
        limits.max_declared_compressed_bytes,
        limits.max_compression_ratio,
        limits.max_path_bytes as u64,
        limits.max_semantic_counts as u64,
        limits.max_semantic_count_value,
        limits.max_omitted_items as u64,
        limits.max_reference_ids as u64,
    ];
    if all.iter().any(|value| *value == 0) {
        return Err("PORTABLE_ARCHIVE_LIMIT_INVALID".to_string());
    }
    Ok(*limits)
}

const COUNT_KEY_MAX_UNITS: usize = 64;
const HASH_LENGTH: usize = 64;

fn is_count_key(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() || value.encode_utf16().count() > COUNT_KEY_MAX_UNITS {
        return false;
    }
    characters.all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '_'
            || character == '.'
            || character == '-'
    })
}

fn is_sha256(value: &str) -> bool {
    value.len() == HASH_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn references(value: &Value, maximum: usize) -> Result<Vec<String>, String> {
    let array = value.as_array().ok_or_else(limit_exceeded)?;
    if array.len() > maximum {
        return Err(limit_exceeded());
    }
    let mut result = Vec::with_capacity(array.len());
    let mut seen: HashSet<String> = HashSet::new();
    for item in array {
        let id = identifier(item)?;
        if !seen.insert(id.clone()) {
            return Err(manifest_invalid());
        }
        result.push(id);
    }
    Ok(result)
}

/// 上游 `parsePortableProjectManifest`：逐字段校验并归一化（未知字段一律拒绝）
pub fn parse_portable_project_manifest(
    value: &Value,
    configured_limits: Option<&PortableArchiveLimits>,
) -> Result<PortableProjectManifest, String> {
    let limits = resolved_limits(configured_limits)?;
    let source = record(value)?;
    exact_keys(
        source,
        &[
            "formatVersion",
            "sourceSchemaVersion",
            "originProjectId",
            "snapshotGeneration",
            "createdAt",
            "declaredUncompressedBytes",
            "declaredCompressedBytes",
            "entries",
            "semanticCounts",
            "omittedItems",
            "transferReceiptIds",
            "historyProjectionIds",
        ],
    )?;
    let format_version = source.get("formatVersion").ok_or_else(manifest_invalid)?;
    let source_schema_version = source
        .get("sourceSchemaVersion")
        .ok_or_else(manifest_invalid)?;
    if !equals_number(format_version, PORTABLE_PROJECT_FORMAT_VERSION as f64)
        || !equals_number(source_schema_version, PORTABLE_SOURCE_SCHEMA_VERSION as f64)
    {
        return Err("PORTABLE_SCHEMA_UNSUPPORTED".to_string());
    }
    let origin_project_id =
        identifier(source.get("originProjectId").ok_or_else(manifest_invalid)?)?;
    let snapshot_generation = identifier(
        source
            .get("snapshotGeneration")
            .ok_or_else(manifest_invalid)?,
    )?;
    let created_at = source
        .get("createdAt")
        .and_then(|value| value.as_str())
        .ok_or_else(manifest_invalid)?
        .to_string();
    if !is_exact_iso8601_millis(&created_at) {
        return Err(manifest_invalid());
    }
    let declared_uncompressed_bytes = bounded_integer(
        source
            .get("declaredUncompressedBytes")
            .ok_or_else(manifest_invalid)?,
        limits.max_total_bytes,
    )?;
    let declared_compressed_bytes = bounded_integer(
        source
            .get("declaredCompressedBytes")
            .ok_or_else(manifest_invalid)?,
        limits.max_declared_compressed_bytes,
    )?;
    if (declared_uncompressed_bytes > 0 && declared_compressed_bytes == 0)
        || declared_uncompressed_bytes
            > declared_compressed_bytes
                .max(1)
                .saturating_mul(limits.max_compression_ratio)
    {
        return Err(limit_exceeded());
    }
    let entries_value = source.get("entries").ok_or_else(manifest_invalid)?;
    let entries_array = entries_value.as_array().ok_or_else(limit_exceeded)?;
    if entries_array.len() > limits.max_entries {
        return Err(limit_exceeded());
    }
    let mut paths: HashSet<String> = HashSet::new();
    let mut directories: HashSet<String> = HashSet::new();
    let mut entries: Vec<PortableProjectManifestEntry> = Vec::with_capacity(entries_array.len());
    for item in entries_array {
        let entry = record(item)?;
        exact_keys(entry, &["path", "byteSize", "sha256", "disposition"])?;
        let path = entry
            .get("path")
            .and_then(|value| value.as_str())
            .ok_or_else(path_unsafe)?;
        let key = portable_path_key(path, limits.max_path_bytes)?;
        if paths.contains(&key) || directories.contains(&key) {
            return Err("PORTABLE_PATH_CONFLICT".to_string());
        }
        let parts: Vec<&str> = key.split('/').collect();
        for index in 1..parts.len() {
            let directory = parts[..index].join("/");
            if paths.contains(&directory) {
                return Err("PORTABLE_PATH_CONFLICT".to_string());
            }
            directories.insert(directory);
        }
        paths.insert(key);
        let byte_size = bounded_integer(
            entry.get("byteSize").ok_or_else(manifest_invalid)?,
            limits.max_entry_bytes,
        )?;
        let sha256 = entry
            .get("sha256")
            .and_then(|value| value.as_str())
            .ok_or_else(manifest_invalid)?;
        if !is_sha256(sha256) {
            return Err(manifest_invalid());
        }
        let disposition = entry
            .get("disposition")
            .and_then(|value| value.as_str())
            .and_then(PortableEntryDisposition::parse)
            .ok_or_else(manifest_invalid)?;
        entries.push(PortableProjectManifestEntry {
            path: path.to_string(),
            byte_size,
            sha256: sha256.to_string(),
            disposition,
        });
    }
    let mut total: u64 = 0;
    for entry in &entries {
        total = total
            .checked_add(entry.byte_size)
            .ok_or_else(limit_exceeded)?;
    }
    if total > limits.max_total_bytes || total != declared_uncompressed_bytes {
        return Err(limit_exceeded());
    }
    let counts = record(source.get("semanticCounts").ok_or_else(manifest_invalid)?)?;
    if counts.len() > limits.max_semantic_counts {
        return Err(limit_exceeded());
    }
    let mut semantic_counts: BTreeMap<String, u64> = BTreeMap::new();
    for (key, count) in counts {
        if !is_count_key(key) {
            return Err(manifest_invalid());
        }
        semantic_counts.insert(
            key.clone(),
            bounded_integer(count, limits.max_semantic_count_value)?,
        );
    }
    let omitted_value = source.get("omittedItems").ok_or_else(manifest_invalid)?;
    let omitted_array = omitted_value.as_array().ok_or_else(limit_exceeded)?;
    if omitted_array.len() > limits.max_omitted_items {
        return Err(limit_exceeded());
    }
    let mut omitted_items: Vec<PortableOmissionItem> = Vec::with_capacity(omitted_array.len());
    let mut omitted_ids: HashSet<String> = HashSet::new();
    for item in omitted_array {
        let omitted = record(item)?;
        exact_keys(omitted, &["id", "reason"])?;
        let reason = omitted
            .get("reason")
            .and_then(|value| value.as_str())
            .and_then(PortableOmissionReason::parse)
            .ok_or_else(manifest_invalid)?;
        let id = identifier(omitted.get("id").ok_or_else(manifest_invalid)?)?;
        if !omitted_ids.insert(id.clone()) {
            return Err(manifest_invalid());
        }
        omitted_items.push(PortableOmissionItem { id, reason });
    }
    Ok(PortableProjectManifest {
        format_version: PORTABLE_PROJECT_FORMAT_VERSION,
        source_schema_version: PORTABLE_SOURCE_SCHEMA_VERSION,
        origin_project_id,
        snapshot_generation,
        created_at,
        declared_uncompressed_bytes,
        declared_compressed_bytes,
        entries,
        semantic_counts,
        omitted_items,
        transfer_receipt_ids: references(
            source
                .get("transferReceiptIds")
                .ok_or_else(manifest_invalid)?,
            limits.max_reference_ids,
        )?,
        history_projection_ids: references(
            source
                .get("historyProjectionIds")
                .ok_or_else(manifest_invalid)?,
            limits.max_reference_ids,
        )?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(path: &str, bytes: u64) -> Value {
        json!({ "path": path, "byteSize": bytes, "sha256": "a".repeat(64), "disposition": "author-content" })
    }

    fn manifest(entries: Vec<Value>, total: u64) -> Value {
        json!({
            "formatVersion": 1,
            "sourceSchemaVersion": 7,
            "originProjectId": "project-1",
            "snapshotGeneration": "gen-1",
            "createdAt": "2026-10-16T05:00:00.000Z",
            "declaredUncompressedBytes": total,
            "declaredCompressedBytes": total.max(1),
            "entries": entries,
            "semanticCounts": { "chapters": 3 },
            "omittedItems": [{ "id": "omitted-1", "reason": "rebuild-stale" }],
            "transferReceiptIds": ["receipt-1"],
            "historyProjectionIds": []
        })
    }

    #[test]
    fn canonical_project_directory_matches_upstream() {
        assert_eq!(CANONICAL_PROJECT_DIRECTORY, ".ai-novel");
    }

    #[test]
    fn parse_accepts_valid_manifest_test() {
        let parsed = parse_portable_project_manifest(
            &manifest(
                vec![entry("novel/book.json", 10), entry("avatars/a.png", 5)],
                15,
            ),
            None,
        )
        .unwrap();
        assert_eq!(parsed.format_version, 1);
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.semantic_counts.get("chapters"), Some(&3));
        assert_eq!(
            parsed.omitted_items[0].reason,
            PortableOmissionReason::RebuildStale
        );
        assert_eq!(parsed.transfer_receipt_ids, vec!["receipt-1".to_string()]);
        assert!(parsed.history_projection_ids.is_empty());
    }

    #[test]
    fn parse_rejects_version_and_shape_drift_test() {
        let mut value = manifest(vec![], 0);
        value["formatVersion"] = json!(2);
        assert_eq!(
            parse_portable_project_manifest(&value, None).unwrap_err(),
            "PORTABLE_SCHEMA_UNSUPPORTED"
        );

        let mut value = manifest(vec![], 0);
        value["sourceSchemaVersion"] = json!(6);
        assert_eq!(
            parse_portable_project_manifest(&value, None).unwrap_err(),
            "PORTABLE_SCHEMA_UNSUPPORTED"
        );

        // 未知键 / 缺键一律拒绝
        let mut value = manifest(vec![], 0);
        value["extra"] = json!(true);
        assert_eq!(
            parse_portable_project_manifest(&value, None).unwrap_err(),
            "PORTABLE_MANIFEST_INVALID"
        );

        let mut value = manifest(vec![], 0);
        value.as_object_mut().unwrap().remove("createdAt");
        assert_eq!(
            parse_portable_project_manifest(&value, None).unwrap_err(),
            "PORTABLE_MANIFEST_INVALID"
        );

        // createdAt 必须是严格 ISO 8601 UTC 毫秒（基线 toISOString 往返判定）
        for created in [
            "2026-10-16T05:00:00Z",
            "2026-10-16T05:00:00.000+00:00",
            "2026-02-30T00:00:00.000Z",
            "2026-10-16",
        ] {
            let mut value = manifest(vec![], 0);
            value["createdAt"] = json!(created);
            assert_eq!(
                parse_portable_project_manifest(&value, None).unwrap_err(),
                "PORTABLE_MANIFEST_INVALID",
                "{created}"
            );
        }
    }

    #[test]
    fn parse_enforces_entry_limits_and_conflicts_test() {
        // 求和必须等于 declaredUncompressedBytes
        let mut value = manifest(vec![entry("a/b.json", 10)], 11);
        assert_eq!(
            parse_portable_project_manifest(&value, None).unwrap_err(),
            "PORTABLE_ARCHIVE_LIMIT_EXCEEDED"
        );

        // 路径冲突：文件与目录同名 / 重复键（大小写不同视为同一键）
        value = manifest(vec![entry("a/b.json", 1), entry("A/b.json", 1)], 2);
        assert_eq!(
            parse_portable_project_manifest(&value, None).unwrap_err(),
            "PORTABLE_PATH_CONFLICT"
        );
        value = manifest(vec![entry("a", 1), entry("a/b.json", 1)], 2);
        assert_eq!(
            parse_portable_project_manifest(&value, None).unwrap_err(),
            "PORTABLE_PATH_CONFLICT"
        );

        // 非法 sha256 / disposition / byteSize
        value = manifest(
            vec![
                json!({ "path": "a", "byteSize": 1, "sha256": "A".repeat(64), "disposition": "author-content" }),
            ],
            1,
        );
        assert_eq!(
            parse_portable_project_manifest(&value, None).unwrap_err(),
            "PORTABLE_MANIFEST_INVALID"
        );
        value = manifest(
            vec![
                json!({ "path": "a", "byteSize": 1, "sha256": "a".repeat(64), "disposition": "nope" }),
            ],
            1,
        );
        assert_eq!(
            parse_portable_project_manifest(&value, None).unwrap_err(),
            "PORTABLE_MANIFEST_INVALID"
        );
        value = manifest(
            vec![
                json!({ "path": "a", "byteSize": -1, "sha256": "a".repeat(64), "disposition": "skill" }),
            ],
            1,
        );
        assert_eq!(
            parse_portable_project_manifest(&value, None).unwrap_err(),
            "PORTABLE_ARCHIVE_LIMIT_EXCEEDED"
        );

        // 压缩比超限
        let mut value = manifest(vec![entry("a", 1000)], 1000);
        value["declaredCompressedBytes"] = json!(1);
        assert_eq!(
            parse_portable_project_manifest(&value, None).unwrap_err(),
            "PORTABLE_ARCHIVE_LIMIT_EXCEEDED"
        );

        // 自定义限额（maxEntries = 1）
        let limits = PortableArchiveLimits {
            max_entries: 1,
            ..PortableArchiveLimits::default()
        };
        let value = manifest(vec![entry("a", 1), entry("b", 1)], 2);
        assert_eq!(
            parse_portable_project_manifest(&value, Some(&limits)).unwrap_err(),
            "PORTABLE_ARCHIVE_LIMIT_EXCEEDED"
        );
        let zero = PortableArchiveLimits {
            max_entries: 0,
            ..PortableArchiveLimits::default()
        };
        assert_eq!(
            parse_portable_project_manifest(&manifest(vec![], 0), Some(&zero)).unwrap_err(),
            "PORTABLE_ARCHIVE_LIMIT_INVALID"
        );
    }

    #[test]
    fn parse_validates_counts_omissions_and_references_test() {
        let mut value = manifest(vec![], 0);
        value["semanticCounts"] = json!({ "Chapters": 1 });
        assert_eq!(
            parse_portable_project_manifest(&value, None).unwrap_err(),
            "PORTABLE_MANIFEST_INVALID"
        );

        let mut value = manifest(vec![], 0);
        value["semanticCounts"] = json!({ "chapters": 1_000_000_001 });
        assert_eq!(
            parse_portable_project_manifest(&value, None).unwrap_err(),
            "PORTABLE_ARCHIVE_LIMIT_EXCEEDED"
        );

        let mut value = manifest(vec![], 0);
        value["omittedItems"] = json!([{ "id": "a", "reason": "nope" }]);
        assert_eq!(
            parse_portable_project_manifest(&value, None).unwrap_err(),
            "PORTABLE_MANIFEST_INVALID"
        );

        let mut value = manifest(vec![], 0);
        value["omittedItems"] = json!([{ "id": "a", "reason": "rebuild-stale" }, { "id": "a", "reason": "rebuild-stale" }]);
        assert_eq!(
            parse_portable_project_manifest(&value, None).unwrap_err(),
            "PORTABLE_MANIFEST_INVALID"
        );

        let mut value = manifest(vec![], 0);
        value["transferReceiptIds"] = json!(["a", "a"]);
        assert_eq!(
            parse_portable_project_manifest(&value, None).unwrap_err(),
            "PORTABLE_MANIFEST_INVALID"
        );

        let mut value = manifest(vec![], 0);
        value["historyProjectionIds"] = json!([""]);
        assert_eq!(
            parse_portable_project_manifest(&value, None).unwrap_err(),
            "PORTABLE_MANIFEST_INVALID"
        );
    }

    #[test]
    fn portable_path_key_normalizes_and_rejects_unsafe_paths_test() {
        assert_eq!(
            portable_path_key("Novel/Book.json", 1024).unwrap(),
            "novel/book.json"
        );
        // NFKC：全角字符归一化后再判定
        assert_eq!(portable_path_key("ａｂｃ", 1024).unwrap(), "abc");
        for unsafe_path in [
            "",
            "/abs",
            "a\\b",
            "C:/windows",
            "a/../b",
            "a/./b",
            "a//b",
            "trailing.",
            "trailing ",
            "bad:name",
            "bad*name",
            "con",
            "CON.txt",
            "lpt1.log",
            "control\u{1}",
        ] {
            assert_eq!(
                portable_path_key(unsafe_path, 1024).unwrap_err(),
                "PORTABLE_PATH_UNSAFE",
                "{unsafe_path:?}"
            );
        }
        // UTF-8 字节长度上限（与基线 Buffer.byteLength 一致）
        assert_eq!(
            portable_path_key("汉字", 4).unwrap_err(),
            "PORTABLE_PATH_UNSAFE"
        );
        assert_eq!(portable_path_key("汉字", 6).unwrap(), "汉字");
    }
}
