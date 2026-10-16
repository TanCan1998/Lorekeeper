//! 便携项目导出服务 —— 平移自 `electron/services/project-archive-service.ts` 的 `exportPortableProject`，
//! 按**本项目口径**落地（2026-10-16 用户确认「不与基线互通、按本项目方案存储恢复」）。
//!
//! # 流程
//!
//! 1. 路径与安全校验：源根与暂存父目录必须是安全目录，归档目标与暂存目录**不得**落在源项目内；
//! 2. `VACUUM INTO` 脱敏副本（[`sanitize_portable_database`]）：中和 R/X 列、删除派生索引表；
//! 3. 便携转移权威（[`create_portable_transfer_authority`] + [`serialize_portable_transfer_authority`]）：
//!    固化「本次导出元组」的定稿/摘要来源现状，写入 `portable-transfer-authority.json`；
//! 4. 资产装配：头像资产（`character_avatar_assets` 与 `character_avatar_unresolved` 中保留路径的行）、
//!    原始资产白名单（[`enumerate_raw_files`]）逐文件校验大小与 sha256；
//! 5. 清单装配：条目（`lorekeeper.db` / 转移权威 / 头像 / 原始资产）、逐表行数、省略项、转移收据 id；
//! 6. 两阶段发布：先在暂存目录内写出归档（容器层负责原子性与身份复核），再以**无覆盖硬链接**发布到目标。
//!
//! # 与基线的差异（本项目口径）
//!
//! - 项目存储根是 `<项目根>/.lore`（`db::PROJECT_DIR_NAME`），不是上游的 `.ai-novel`：导出从该根\n//!   收集库、头像与原始资产，归档条目一律相对该根、不带任何前缀；\n//! - 不产出基线式逐行历史投影文件（`historyProjectionIds` 只承载头像引用占位，见 `omittedItems` 说明）；
//! - 派生索引（FTS5 `kb_fts`、HNSW 向量快照）一律按 `rebuild-stale` 省略，知识**事实**在库副本内；
//! - 基线依赖 Electron 备份 API 二次快照做「源未变」复核；本项目在归档写出前复核一次
//!   （容器层已对每个源文件做「读取前后四次身份复核 + sha256 校验」，等价强度，少一次整库快照）。

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::db::PROJECT_DIR_NAME;
use crate::services::portable_project_archive::{
    safe_directory, write_portable_project_archive, PortableArchiveReceipt, PortableArchiveSource,
    WritePortableProjectArchiveInput, MANIFEST_PATH,
};
use crate::services::portable_project_assets::{enumerate_raw_files, PortableProvidedFile};
use crate::services::portable_project_format::{
    PortableArchiveLimits, PortableEntryDisposition, PortableOmissionItem, PortableOmissionReason,
    PortableProjectManifest, PortableProjectManifestEntry, PORTABLE_PROJECT_FORMAT_VERSION,
    PORTABLE_SOURCE_SCHEMA_VERSION,
};
use crate::services::portable_project_sanitize::{
    sanitize_portable_database, SanitizePortableDatabaseInput,
};
use crate::services::portable_transfer_authority::{
    create_portable_transfer_authority, serialize_portable_transfer_authority,
    CreatePortableTransferAuthorityInput,
};

/// 便携库在归档内的条目名（本项目库文件名）
pub const PORTABLE_DATABASE_ENTRY: &str = "lorekeeper.db";
/// 转移权威在归档内的条目名
pub const PORTABLE_TRANSFER_AUTHORITY_ENTRY: &str = "portable-transfer-authority.json";
/// 暂存目录内的便携库文件名
const STAGED_DATABASE_FILE: &str = "portable.db";
/// 暂存目录内的归档文件名
const STAGED_ARCHIVE_FILE: &str = "portable-project.archive";

fn source_changed() -> String {
    "PORTABLE_SOURCE_CHANGED".to_string()
}

fn asset_missing() -> String {
    "PORTABLE_ASSET_MISSING".to_string()
}

fn asset_unsafe() -> String {
    "PORTABLE_ASSET_UNSAFE".to_string()
}

fn target_exists() -> String {
    "PORTABLE_ARCHIVE_TARGET_EXISTS".to_string()
}

/// 上游 `ExportPortableProjectInput` 的收据侧字段
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSourceEvidence {
    pub schema_version: u32,
    pub schema_fingerprint: String,
    pub table_count: usize,
    pub field_count: usize,
}

/// 上游 `ExportPortableProjectReceipt`
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPortableProjectReceipt {
    pub origin_project_id: String,
    pub snapshot_generation: String,
    pub target_sha256: String,
    pub target_byte_size: u64,
    pub source_evidence: ExportSourceEvidence,
    pub entry_count: usize,
    pub requires_runtime_freeze_guard: bool,
}

/// 上游 `ExportPortableProjectInput`（会话校验由命令层完成后传入已确认的项目身份）
#[derive(Debug, Clone)]
pub struct ExportPortableProjectInput<'a> {
    pub source_project_root: &'a Path,
    /// 已由命令层校验的源项目 id（迁移权威与清单的来源身份）
    pub origin_project_id: &'a str,
    pub target_archive_path: &'a Path,
    pub attempt_parent_path: &'a Path,
    pub snapshot_generation: Option<&'a str>,
    /// 严格 ISO 8601 UTC 毫秒串（`2026-10-16T05:00:00.000Z`）
    pub created_at: &'a str,
    pub limits: Option<PortableArchiveLimits>,
}

fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|root| root.join(path))
        .map_err(|_| asset_unsafe())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|_| asset_missing())?;
    Ok(sha256_hex(&bytes))
}

/// 便携 schema 指纹：表/字段规范化文本的 SHA-256（本项目口径，供收据比对）
fn schema_fingerprint(conn: &Connection) -> Result<String, String> {
    let mut statement = conn
        .prepare(
            "SELECT name FROM pragma_table_list WHERE schema='main' AND type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .map_err(|_| asset_unsafe())?;
    let tables = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| asset_unsafe())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| asset_unsafe())?;
    drop(statement);
    let mut canonical = String::new();
    for table in tables {
        canonical.push_str(&table);
        canonical.push('(');
        let quoted = format!("'{}'", table.replace('\'', "''"));
        let mut columns = conn
            .prepare(&format!("PRAGMA table_info({quoted})"))
            .map_err(|_| asset_unsafe())?;
        let names = columns
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })
            .map_err(|_| asset_unsafe())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| asset_unsafe())?;
        for (index, (name, type_name)) in names.iter().enumerate() {
            if index > 0 {
                canonical.push(',');
            }
            canonical.push_str(name);
            canonical.push(':');
            canonical.push_str(type_name);
        }
        canonical.push_str(");");
    }
    Ok(sha256_hex(canonical.as_bytes()))
}

/// 头像资产行（`character_avatar_assets` 与 `character_avatar_unresolved` 中保留路径的行）
#[derive(Debug, Clone, PartialEq, Eq)]
struct AvatarAssetRow {
    relative_path: String,
    content_hash: String,
    byte_size: i64,
}

fn avatar_asset_rows(conn: &Connection) -> Result<Vec<AvatarAssetRow>, String> {
    let mut rows: Vec<AvatarAssetRow> = Vec::new();
    {
        let mut statement = conn
            .prepare(
                "SELECT relative_path, content_hash, byte_size FROM character_avatar_assets ORDER BY relative_path",
            )
            .map_err(|_| asset_unsafe())?;
        let mapped = statement
            .query_map([], |row| {
                Ok(AvatarAssetRow {
                    relative_path: row.get(0)?,
                    content_hash: row.get(1)?,
                    byte_size: row.get(2)?,
                })
            })
            .map_err(|_| asset_unsafe())?;
        for row in mapped {
            rows.push(row.map_err(|_| asset_unsafe())?);
        }
    }
    {
        let mut statement = conn
            .prepare(
                "SELECT preserved_relative_path, content_hash, byte_size FROM character_avatar_unresolved \
                 WHERE preserved_relative_path IS NOT NULL ORDER BY preserved_relative_path",
            )
            .map_err(|_| asset_unsafe())?;
        let mapped = statement
            .query_map([], |row| {
                Ok(AvatarAssetRow {
                    relative_path: row.get(0)?,
                    content_hash: row.get(1)?,
                    byte_size: row.get(2)?,
                })
            })
            .map_err(|_| asset_unsafe())?;
        for row in mapped {
            rows.push(row.map_err(|_| asset_unsafe())?);
        }
    }
    Ok(rows)
}

fn omitted_items(
    neutralized: &[crate::services::portable_project_sanitize::NeutralizedColumn],
    dropped_tables: &[String],
) -> Vec<PortableOmissionItem> {
    let mut items: Vec<PortableOmissionItem> = vec![
        PortableOmissionItem {
            id: "vectors".to_string(),
            reason: PortableOmissionReason::RebuildStale,
        },
        PortableOmissionItem {
            id: "cache".to_string(),
            reason: PortableOmissionReason::RebuildStale,
        },
        PortableOmissionItem {
            id: "logs".to_string(),
            reason: PortableOmissionReason::ExcludeMachineAuthority,
        },
        PortableOmissionItem {
            id: "temp".to_string(),
            reason: PortableOmissionReason::ExcludeMachineAuthority,
        },
        PortableOmissionItem {
            id: "machine-authority-fields".to_string(),
            reason: PortableOmissionReason::ExcludeMachineAuthority,
        },
    ];
    for table in dropped_tables {
        items.push(PortableOmissionItem {
            id: format!("derived-index:{table}"),
            reason: PortableOmissionReason::RebuildStale,
        });
    }
    for column in neutralized {
        items.push(PortableOmissionItem {
            id: format!("neutralized:{}.{}", column.table, column.column),
            reason: PortableOmissionReason::HistoricalNonreplayable,
        });
    }
    items
}

/// 上游 `exportPortableProject`（本项目口径）
pub fn export_portable_project(
    input: &ExportPortableProjectInput<'_>,
) -> Result<ExportPortableProjectReceipt, String> {
    let limits = input.limits.unwrap_or_default();
    let source_root = absolute(input.source_project_root)?;
    let target = absolute(input.target_archive_path)?;
    let attempt_parent = absolute(input.attempt_parent_path)?;
    if target.starts_with(&source_root) || attempt_parent.starts_with(&source_root) {
        return Err("PORTABLE_TARGET_INSIDE_SOURCE".to_string());
    }
    safe_directory(&source_root)?;
    safe_directory(&attempt_parent)?;
    let storage_root = source_root.join(PROJECT_DIR_NAME);
    safe_directory(&storage_root)?;
    let source_database = storage_root.join(PORTABLE_DATABASE_ENTRY);
    if std::fs::symlink_metadata(&source_database).is_err() {
        return Err(asset_missing());
    }
    if std::fs::symlink_metadata(&target).is_ok() {
        return Err(target_exists());
    }

    let snapshot_generation = match input.snapshot_generation {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => crate::project_access::random_uuid_v4(),
    };
    let attempt = crate::services::portable_project_archive::prepare_staging(&attempt_parent)?;
    let staged_database = attempt.root.join(STAGED_DATABASE_FILE);
    let staged_archive = attempt.root.join(STAGED_ARCHIVE_FILE);

    let outcome = (|| -> Result<ExportPortableProjectReceipt, String> {
        // 脱敏副本
        let sanitized = sanitize_portable_database(&SanitizePortableDatabaseInput {
            source_database_path: &source_database,
            target_database_path: &staged_database,
            limits: Some(limits),
        })?;
        let database_bytes = std::fs::metadata(&staged_database)
            .map_err(|_| asset_missing())?
            .len();
        let database_sha256 = sha256_file(&staged_database)?;

        // 转移权威
        let transfer_bytes = {
            let conn = Connection::open(&staged_database).map_err(|_| asset_unsafe())?;
            let authority =
                create_portable_transfer_authority(CreatePortableTransferAuthorityInput {
                    database: &conn,
                    freeze: None,
                    origin_project_id: input.origin_project_id,
                    snapshot_generation: &snapshot_generation,
                    portable_database_sha256: &database_sha256,
                })
                .map_err(|_| asset_unsafe())?;
            let receipt_id = authority.receipt_id.clone();
            let bytes =
                serialize_portable_transfer_authority(&authority).map_err(|_| asset_unsafe())?;
            let evidence = ExportSourceEvidence {
                schema_version: PORTABLE_SOURCE_SCHEMA_VERSION,
                schema_fingerprint: schema_fingerprint(&conn)?,
                table_count: sanitized.schema.tables,
                field_count: sanitized.schema.fields,
            };
            let avatars = avatar_asset_rows(&conn)?;
            (bytes, receipt_id, evidence, avatars)
        };
        let (authority_bytes, receipt_id, evidence, avatars) = transfer_bytes;
        let authority_path = attempt.root.join(PORTABLE_TRANSFER_AUTHORITY_ENTRY);
        std::fs::write(&authority_path, &authority_bytes).map_err(|_| asset_unsafe())?;

        // 资产装配
        let mut sources: Vec<PortableArchiveSource> = Vec::new();
        let mut entries: Vec<PortableProjectManifestEntry> = Vec::new();
        let mut claimed_path: std::collections::HashSet<String> = std::collections::HashSet::new();
        let add_source = |path: String,
                          byte_size: u64,
                          sha256: String,
                          disposition: PortableEntryDisposition,
                          source_path: PathBuf,
                          sources: &mut Vec<PortableArchiveSource>,
                          entries: &mut Vec<PortableProjectManifestEntry>,
                          claimed: &mut std::collections::HashSet<String>|
         -> Result<(), String> {
            let key = crate::services::portable_project_format::portable_path_key(
                &path,
                limits.max_path_bytes,
            )
            .map_err(|_| asset_unsafe())?;
            if key == MANIFEST_PATH || !claimed.insert(key) {
                return Err(asset_unsafe());
            }
            let entry = PortableProjectManifestEntry {
                path,
                byte_size,
                sha256,
                disposition,
            };
            sources.push(PortableArchiveSource {
                entry: entry.clone(),
                source_path,
            });
            entries.push(entry);
            Ok(())
        };

        add_source(
            PORTABLE_DATABASE_ENTRY.to_string(),
            database_bytes,
            database_sha256,
            PortableEntryDisposition::PortableDatabase,
            staged_database.clone(),
            &mut sources,
            &mut entries,
            &mut claimed_path,
        )?;
        add_source(
            PORTABLE_TRANSFER_AUTHORITY_ENTRY.to_string(),
            authority_bytes.len() as u64,
            sha256_hex(&authority_bytes),
            PortableEntryDisposition::TransferReceipt,
            authority_path.clone(),
            &mut sources,
            &mut entries,
            &mut claimed_path,
        )?;

        // 头像资产：逐文件比对大小与内容哈希
        for row in &avatars {
            let source_path = storage_root.join(
                row.relative_path
                    .replace('/', std::path::MAIN_SEPARATOR_STR),
            );
            if !source_path.starts_with(&storage_root) {
                return Err(asset_unsafe());
            }
            let size = std::fs::metadata(&source_path)
                .map_err(|_| asset_missing())?
                .len();
            if size as i64 != row.byte_size || sha256_file(&source_path)? != row.content_hash {
                return Err(source_changed());
            }
            add_source(
                row.relative_path.clone(),
                size,
                row.content_hash.clone(),
                PortableEntryDisposition::AvatarAsset,
                source_path,
                &mut sources,
                &mut entries,
                &mut claimed_path,
            )?;
        }

        // 原始资产白名单
        let provided: Vec<PortableProvidedFile> = enumerate_raw_files(&storage_root)?;
        for file in &provided {
            if file.source_path.is_empty() {
                continue;
            }
            let source_path = PathBuf::from(&file.source_path);
            let size = std::fs::metadata(&source_path)
                .map_err(|_| asset_missing())?
                .len();
            if size != file.byte_size || sha256_file(&source_path)? != file.sha256 {
                return Err(source_changed());
            }
            add_source(
                file.archive_path.clone(),
                size,
                file.sha256.clone(),
                file.disposition,
                source_path,
                &mut sources,
                &mut entries,
                &mut claimed_path,
            )?;
        }

        let mut semantic_counts = sanitized.semantic_counts.clone();
        semantic_counts.insert("raw-assets".to_string(), provided.len() as u64);
        semantic_counts.insert("avatars".to_string(), avatars.len() as u64);
        let payload_bytes: u64 = entries.iter().map(|entry| entry.byte_size).sum();
        let manifest = PortableProjectManifest {
            format_version: PORTABLE_PROJECT_FORMAT_VERSION,
            source_schema_version: PORTABLE_SOURCE_SCHEMA_VERSION,
            origin_project_id: input.origin_project_id.to_string(),
            snapshot_generation: snapshot_generation.clone(),
            created_at: input.created_at.to_string(),
            declared_uncompressed_bytes: payload_bytes,
            declared_compressed_bytes: payload_bytes,
            entries,
            semantic_counts,
            omitted_items: omitted_items(&sanitized.neutralized_columns, &sanitized.dropped_tables),
            transfer_receipt_ids: vec![receipt_id],
            history_projection_ids: Vec::new(),
        };
        let archive_receipt: PortableArchiveReceipt =
            write_portable_project_archive(&WritePortableProjectArchiveInput {
                manifest: &manifest,
                sources: &sources,
                target_path: &staged_archive,
                limits: Some(limits),
            })?;

        // 两阶段发布：无覆盖硬链接到目标
        match std::fs::hard_link(&staged_archive, &target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(target_exists())
            }
            Err(_) => return Err("PORTABLE_ARCHIVE_PUBLISH_UNSUPPORTED".to_string()),
        }
        let target_bytes = std::fs::metadata(&target)
            .map_err(|_| asset_missing())?
            .len();
        Ok(ExportPortableProjectReceipt {
            origin_project_id: input.origin_project_id.to_string(),
            snapshot_generation,
            target_sha256: sha256_file(&target)?,
            target_byte_size: target_bytes,
            source_evidence: evidence,
            entry_count: archive_receipt.entry_count,
            requires_runtime_freeze_guard: true,
        })
    })();

    discard_attempt(&attempt);
    outcome
}

/// 回收暂存目录：先复核暂存根仍是本次尝试创建的那个目录，再整体删除。
///
/// 暂存根是本次操作私有的随机命名目录（`<父目录>/.portable-project-attempt-<uuid>`），
/// 内部还有脱敏副本、转移权威与暂存归档等**未进入所有权清单**的文件，故不能用逐项回收；
/// 身份复核不通过时（目录被替换/劫持）退回逐项回收，绝不删除来历不明的目录。
fn discard_attempt(attempt: &crate::services::portable_project_archive::StagingAttempt) {
    if crate::services::portable_project_archive::assert_attempt_directory(attempt, &attempt.root)
        .is_ok()
    {
        let _ = std::fs::remove_dir_all(&attempt.root);
        return;
    }
    crate::services::portable_project_archive::cleanup_attempt(attempt);
}

/// 读取源库中「是否存在指针」等只读判定（诊断用）
pub fn source_database_exists(source_project_root: &Path) -> bool {
    source_project_root
        .join(PROJECT_DIR_NAME)
        .join(PORTABLE_DATABASE_ENTRY)
        .exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;

    /// 迁移权威要求 `originProjectId` 是 UUID（与真实项目 id 一致）
    const ORIGIN_PROJECT_ID: &str = "123e4567-e89b-12d3-a456-426614174000";

    fn temp_project(name: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "lorekeeper-export-{name}-{}",
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let project = base.join("project");
        let storage = project.join(PROJECT_DIR_NAME);
        std::fs::create_dir_all(storage.join("prompts")).unwrap();
        let canonical_base = std::fs::canonicalize(&base).unwrap();
        let canonical_project = std::fs::canonicalize(&project).unwrap();
        let canonical_storage = std::fs::canonicalize(&storage).unwrap();
        (
            canonical_base,
            canonical_project,
            canonical_storage,
            project,
        )
    }

    fn seed_project(storage: &Path) {
        let db = storage.join(PORTABLE_DATABASE_ENTRY);
        let conn = Connection::open(&db).unwrap();
        create_tables(&conn).unwrap();
        conn.execute(
            "INSERT INTO kb_documents (id, file_name, file_path, corpus_kind, imported_at, chunk_count, content_hash)
             VALUES ('doc-1', '参考.md', '', 'reference', '2026-10-16T00:00:00.000Z', 1, 'hash')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO llm_calls (model_id, model_name) VALUES ('model-1', '名称')",
            [],
        )
        .unwrap();
        drop(conn);
        std::fs::write(storage.join("prompts").join("a.md"), b"prompt-a").unwrap();
    }

    #[test]
    fn export_publishes_archive_with_manifest_and_receipt_test() {
        let (base, project, storage, _raw_project) = temp_project("basic");
        seed_project(&storage);
        let target = base.join("book.ainovel");
        let attempt_parent = base.join("staging");
        std::fs::create_dir_all(&attempt_parent).unwrap();
        let receipt = export_portable_project(&ExportPortableProjectInput {
            source_project_root: &project,
            origin_project_id: ORIGIN_PROJECT_ID,
            target_archive_path: &target,
            attempt_parent_path: &attempt_parent,
            snapshot_generation: Some("gen-1"),
            created_at: "2026-10-16T05:00:00.000Z",
            limits: None,
        })
        .unwrap();

        assert_eq!(receipt.origin_project_id, ORIGIN_PROJECT_ID);
        assert_eq!(receipt.snapshot_generation, "gen-1");
        assert_eq!(
            receipt.target_byte_size,
            std::fs::metadata(&target).unwrap().len()
        );
        assert_eq!(receipt.target_sha256, sha256_file(&target).unwrap());
        assert_eq!(
            receipt.source_evidence.schema_version,
            PORTABLE_SOURCE_SCHEMA_VERSION
        );
        assert_eq!(receipt.source_evidence.schema_fingerprint.len(), 64);
        assert!(receipt.source_evidence.table_count >= 40);
        assert!(receipt.requires_runtime_freeze_guard);
        // 条目：库 + 转移权威 + 原始资产
        assert_eq!(receipt.entry_count, 3);

        // 暂存目录回收
        let leftovers: Vec<String> = std::fs::read_dir(&attempt_parent)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(leftovers.is_empty(), "暂存目录必须回收：{leftovers:?}");

        // 归档可被本项目读取面解出，且口径正确：知识事实在库内、派生索引不在
        let extract_parent = base.join("extract");
        std::fs::create_dir_all(&extract_parent).unwrap();
        let extracted =
            crate::services::portable_project_archive::extract_portable_project_archive(
                &crate::services::portable_project_archive::ExtractPortableProjectArchiveInput {
                    archive_path: &target,
                    staging_parent_path: &extract_parent,
                    limits: None,
                },
            )
            .unwrap();
        assert_eq!(extracted.manifest.origin_project_id, ORIGIN_PROJECT_ID);
        assert_eq!(extracted.manifest.snapshot_generation, "gen-1");
        assert_eq!(extracted.manifest.transfer_receipt_ids.len(), 1);
        assert!(extracted.manifest.omitted_items.iter().any(
            |item| item.id == "vectors" && item.reason == PortableOmissionReason::RebuildStale
        ));
        assert!(extracted
            .manifest
            .omitted_items
            .iter()
            .any(|item| item.id == "derived-index:kb_fts"));
        assert!(extracted
            .manifest
            .omitted_items
            .iter()
            .any(|item| item.id == "neutralized:llm_calls.model_id"
                && item.reason == PortableOmissionReason::HistoricalNonreplayable));
        assert_eq!(
            extracted.manifest.semantic_counts.get("table.kb_documents"),
            Some(&1)
        );
        assert_eq!(
            extracted.manifest.semantic_counts.get("raw-assets"),
            Some(&1)
        );
        let portable =
            Connection::open(extracted.staging_path.join(PORTABLE_DATABASE_ENTRY)).unwrap();
        let preserved: String = portable
            .query_row(
                "SELECT file_name FROM kb_documents WHERE id='doc-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(preserved, "参考.md", "知识事实随库入档");
        let neutralized: String = portable
            .query_row("SELECT model_id FROM llm_calls", [], |row| row.get(0))
            .unwrap();
        assert_eq!(neutralized, "", "R 列已中和");
        assert!(std::fs::read(extracted.staging_path.join("prompts/a.md")).is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn export_rejects_target_inside_source_and_existing_target_test() {
        let (base, project, storage, _raw_project) = temp_project("reject");
        seed_project(&storage);
        let attempt_parent = base.join("staging");
        std::fs::create_dir_all(&attempt_parent).unwrap();

        // 目标落在源项目内 → 拒绝
        assert_eq!(
            export_portable_project(&ExportPortableProjectInput {
                source_project_root: &project,
                origin_project_id: "project-1",
                target_archive_path: &storage.join("book.ainovel"),
                attempt_parent_path: &attempt_parent,
                snapshot_generation: None,
                created_at: "2026-10-16T05:00:00.000Z",
                limits: None,
            })
            .unwrap_err(),
            "PORTABLE_TARGET_INSIDE_SOURCE"
        );

        // 目标已存在 → 拒绝（且不留下暂存目录）
        let target = base.join("book.ainovel");
        std::fs::write(&target, b"occupied").unwrap();
        assert_eq!(
            export_portable_project(&ExportPortableProjectInput {
                source_project_root: &project,
                origin_project_id: "project-1",
                target_archive_path: &target,
                attempt_parent_path: &attempt_parent,
                snapshot_generation: None,
                created_at: "2026-10-16T05:00:00.000Z",
                limits: None,
            })
            .unwrap_err(),
            "PORTABLE_ARCHIVE_TARGET_EXISTS"
        );
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"occupied",
            "既有文件不得被覆盖"
        );
        let leftovers: Vec<String> = std::fs::read_dir(&attempt_parent)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(leftovers.is_empty(), "失败必须回收暂存目录：{leftovers:?}");

        // 源库缺失 → ASSET_MISSING
        let empty_project = base.join("empty");
        std::fs::create_dir_all(empty_project.join(PROJECT_DIR_NAME)).unwrap();
        assert_eq!(
            export_portable_project(&ExportPortableProjectInput {
                source_project_root: &std::fs::canonicalize(&empty_project).unwrap(),
                origin_project_id: "project-1",
                target_archive_path: &base.join("empty.ainovel"),
                attempt_parent_path: &attempt_parent,
                snapshot_generation: None,
                created_at: "2026-10-16T05:00:00.000Z",
                limits: None,
            })
            .unwrap_err(),
            "PORTABLE_ASSET_MISSING"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
