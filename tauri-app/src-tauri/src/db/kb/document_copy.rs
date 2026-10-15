//! document_copy —— 知识库文档「完整原文项目副本」的旁路存储。
//!
//! 对齐基线 `electron/vector-store.ts` 的 `KNOWLEDGE_COPY_MARKER` /
//! `readKnowledgeCopy` / `writeKnowledgeCopy`（副本文件
//! `<项目存储根>/knowledge-copies/<sha256(docId)>.json`，字段 camelCase），
//! 以及 `electron/knowledge-base.ts` 的 `readDocumentCopy` /
//! `saveDocumentCopy` / `reindexDocumentCopy` 依赖的副本语义。
//!
//! 基线把副本写在 `getProjectDataRoot(projectPath)` 下；Tauri 的项目存储根是
//! `<project>/.lore`（见 [`crate::db::PROJECT_DIR_NAME`]），**绝不触碰**基线的
//! `.vela/`。
//!
//! 安全性对齐基线：副本目录若为符号链接、副本文件若为符号链接或硬链接
//! （`nlink != 1`）、或副本 JSON 形状不符，一律拒绝而不是降级读取。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 文档行 `filePath` 的副本标记（对齐基线 `KNOWLEDGE_COPY_MARKER`）
pub const KNOWLEDGE_COPY_MARKER: &str = "knowledge-copy:";
/// 副本目录名（项目存储根下）
const COPY_DIRECTORY: &str = "knowledge-copies";
/// 副本目录/文件路径不安全（符号链接、硬链接、目录被替换）
pub const UNSAFE_PATH: &str = "KNOWLEDGE_COPY_UNSAFE_PATH";
/// 副本文件形状非法
pub const INVALID_COPY: &str = "KNOWLEDGE_COPY_INVALID";

/// 项目存储根：`<project>/.lore`
pub fn project_storage_root(project_root: &str) -> PathBuf {
    Path::new(project_root).join(crate::db::PROJECT_DIR_NAME)
}

/// 文档行的副本 `filePath`：`knowledge-copy:<docId>`
pub fn marker_file_path(doc_id: &str) -> String {
    format!("{KNOWLEDGE_COPY_MARKER}{doc_id}")
}

/// 该 `filePath` 是否指向项目副本
pub fn is_copy_file_path(file_path: &str) -> bool {
    file_path.starts_with(KNOWLEDGE_COPY_MARKER)
}

/// 内容哈希（SHA-256 小写 hex，对齐基线 `contentHash`）
pub fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 副本记录（对齐基线 `KnowledgeCopy`；JSON 字段为 camelCase）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeCopy {
    pub content: String,
    pub indexed_hash: String,
    pub edited: bool,
    pub index_dirty: bool,
}

impl KnowledgeCopy {
    /// 新建未编辑副本（导入时写入，对齐基线 `importDocument`/`importText`）
    pub fn fresh(content: &str) -> Self {
        Self {
            content: content.to_string(),
            indexed_hash: hash_content(content),
            edited: false,
            index_dirty: false,
        }
    }

    /// 索引状态：`current` 当且仅当未标记脏且索引哈希等于当前内容哈希
    pub fn index_status(&self) -> &'static str {
        if !self.index_dirty && hash_content(&self.content) == self.indexed_hash {
            "current"
        } else {
            "stale"
        }
    }
}

/// 副本文件路径：`<storage_root>/knowledge-copies/<sha256(docId)>.json`
fn copy_path(storage_root: &Path, doc_id: &str) -> Result<PathBuf, String> {
    match fs::symlink_metadata(storage_root) {
        Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => {
            return Err(UNSAFE_PATH.to_string())
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let directory = storage_root.join(COPY_DIRECTORY);
    match fs::symlink_metadata(&directory) {
        Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => {
            return Err(UNSAFE_PATH.to_string())
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    Ok(directory.join(format!("{}.json", hash_content(doc_id))))
}

/// 硬链接数（非 1 视为被替换/共享，拒绝读取）
fn link_count(file: &fs::File) -> Result<u64, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        file.metadata()
            .map(|meta| meta.nlink())
            .map_err(|error| error.to_string())
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // 安全：句柄由 file 持有，输出结构有效；查询失败时拒绝访问。
        let succeeded =
            unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) };
        if succeeded == 0 {
            Err(UNSAFE_PATH.to_string())
        } else {
            Ok(u64::from(information.nNumberOfLinks))
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err(UNSAFE_PATH.to_string())
    }
}

/// 读取副本；文件不存在返回 `Ok(None)`（对齐基线 `readKnowledgeCopy`）
pub fn read(storage_root: &Path, doc_id: &str) -> Result<Option<KnowledgeCopy>, String> {
    let file = copy_path(storage_root, doc_id)?;
    let meta = match fs::symlink_metadata(&file) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(UNSAFE_PATH.to_string());
    }
    let mut handle = fs::File::open(&file).map_err(|error| error.to_string())?;
    if link_count(&handle)? != 1 {
        return Err(UNSAFE_PATH.to_string());
    }
    // 从完成链接数检查的同一文件句柄读取，避免再次按路径打开。
    let mut raw = String::new();
    std::io::Read::read_to_string(&mut handle, &mut raw).map_err(|error| error.to_string())?;
    let copy: KnowledgeCopy = serde_json::from_str(&raw).map_err(|_| INVALID_COPY.to_string())?;
    Ok(Some(copy))
}

/// 写入副本（临时文件 + `rename` 原子替换，对齐基线 `writeKnowledgeCopy`）
pub fn write(storage_root: &Path, doc_id: &str, copy: &KnowledgeCopy) -> Result<(), String> {
    let file = copy_path(storage_root, doc_id)?;
    let directory = file
        .parent()
        .ok_or_else(|| UNSAFE_PATH.to_string())?
        .to_path_buf();
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let stem = file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("copy.json");
    let temporary = directory.join(format!(
        "{stem}.{}.tmp",
        crate::project_access::random_uuid_v4()
    ));
    let raw = serde_json::to_string(copy).map_err(|error| error.to_string())?;
    let result = (|| -> Result<(), String> {
        let mut handle = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        handle
            .write_all(raw.as_bytes())
            .map_err(|error| error.to_string())?;
        handle.sync_all().map_err(|error| error.to_string())?;
        drop(handle);
        fs::rename(&temporary, &file).map_err(|error| error.to_string())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

/// 完整原文不可恢复时的统一文案。
pub const UNAVAILABLE: &str = "完整原文不可用，请重新导入";

/// 必须持有 source_gate；只认副本标记，不从旧块拼接正文。
pub fn read_view(
    conn: &rusqlite::Connection,
    root: &Path,
    doc_id: &str,
) -> Result<serde_json::Value, String> {
    let unavailable = serde_json::json!({"available": false, "indexStatus": "unavailable"});
    let Some(row) = super::store::document_row(conn, doc_id)? else {
        return Ok(unavailable);
    };
    if !is_copy_file_path(&row.file_path) {
        return Ok(unavailable);
    }
    let Some(copy) = read(root, doc_id)? else {
        return Ok(unavailable);
    };
    Ok(serde_json::json!({
        "available": true, "content": copy.content, "contentHash": hash_content(&copy.content),
        "edited": copy.edited, "indexStatus": copy.index_status(),
    }))
}

/// 必须持有 source_gate；比较与写回在同一临界区，禁止最后写入者静默覆盖。
pub fn save(
    conn: &rusqlite::Connection,
    root: &Path,
    doc_id: &str,
    content: String,
    expected_hash: &str,
) -> Result<(), String> {
    let row = super::store::document_row(conn, doc_id)?.ok_or(UNAVAILABLE)?;
    if !is_copy_file_path(&row.file_path) {
        return Err(UNAVAILABLE.to_string());
    }
    let mut copy = read(root, doc_id)?.ok_or(UNAVAILABLE)?;
    if hash_content(&copy.content) != expected_hash {
        return Err("项目副本已变化，请重新打开后编辑".to_string());
    }
    copy.content = content;
    copy.edited = true;
    copy.index_dirty = true;
    write(root, doc_id, &copy)
}

pub struct ReindexedCopy {
    pub chunk_count: usize,
    pub previous_ids: Vec<String>,
    pub spaces: Vec<super::hybrid::EmbeddingSpace>,
}

/// 必须持有 source_gate。SQLite 事务覆盖旧块/FTS/代际元数据的更新；
/// 副本写失败回滚数据库，提交失败补偿副本；不调用模型、不生成向量。
pub fn reindex(
    conn: &rusqlite::Connection,
    root: &Path,
    doc_id: &str,
) -> Result<ReindexedCopy, String> {
    reindex_with_writer(conn, root, doc_id, write)
}

fn reindex_with_writer(
    conn: &rusqlite::Connection,
    root: &Path,
    doc_id: &str,
    mut persist: impl FnMut(&Path, &str, &KnowledgeCopy) -> Result<(), String>,
) -> Result<ReindexedCopy, String> {
    let row = super::store::document_row(conn, doc_id)?.ok_or(UNAVAILABLE)?;
    if !is_copy_file_path(&row.file_path) {
        return Err(UNAVAILABLE.to_string());
    }
    let copy = read(root, doc_id)?
        .filter(|copy| !copy.content.trim().is_empty())
        .ok_or("项目副本为空，无法重建索引")?;
    let chunks = super::chunks::chunk_text(&copy.content, 500, 50);
    let previous_ids = super::store::document_chunk_ids(conn, doc_id)?;
    if previous_ids.is_empty() {
        return Err("KNOWLEDGE_DOCUMENT_CHUNKS_MISSING".to_string());
    }
    let spaces = super::hybrid::list_spaces(conn).map_err(|error| error.to_string())?;
    let imported_at: String = conn
        .query_row(
            "SELECT imported_at FROM kb_documents WHERE id = ?1",
            [doc_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let transaction = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    super::fts::delete_document(&transaction, doc_id).map_err(|error| error.to_string())?;
    transaction
        .execute("DELETE FROM kb_chunks WHERE doc_id = ?1", [doc_id])
        .map_err(|error| error.to_string())?;
    transaction
        .execute("DELETE FROM kb_documents WHERE id = ?1", [doc_id])
        .map_err(|error| error.to_string())?;
    let meta = super::store::DocumentMeta {
        file_path: &row.file_path,
        chapter_number: row.chapter_number,
        chapter_title: row.chapter_title.as_deref(),
        corpus_kind: &row.corpus_kind,
    };
    super::store::insert_document(&transaction, doc_id, &row.file_name, &chunks, &meta)?;
    transaction
        .execute(
            "UPDATE kb_documents SET imported_at = ?1 WHERE id = ?2",
            rusqlite::params![imported_at, doc_id],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE kb_chunks SET imported_at = ?1 WHERE doc_id = ?2",
            rusqlite::params![imported_at, doc_id],
        )
        .map_err(|error| error.to_string())?;
    // 重建后新块没有向量，必须撤销激活，避免旧代际短路遮蔽新正文。
    transaction
        .execute(
            "UPDATE kb_embedding_spaces SET status = 'inactive' WHERE status = 'active'",
            [],
        )
        .map_err(|error| error.to_string())?;
    let next = KnowledgeCopy {
        indexed_hash: hash_content(&copy.content),
        index_dirty: false,
        ..copy.clone()
    };
    // 提交前只能持久化脏标记；崩溃/提交失败均不会让旧索引冒充新正文。
    persist(
        root,
        doc_id,
        &KnowledgeCopy {
            index_dirty: true,
            ..next.clone()
        },
    )?;
    if let Err(error) = transaction.commit() {
        // 数据库提交失败已回滚；恢复副本，不得把未提交索引宣告为 current。
        write(root, doc_id, &copy)
            .map_err(|restore| format!("{error}；恢复副本失败：{restore}"))?;
        return Err(error.to_string());
    }
    // 提交后的落盘失败保留 stale，允许重试；不向检索暴露未确认的副本。
    persist(root, doc_id, &next)?;
    Ok(ReindexedCopy {
        chunk_count: chunks.len(),
        previous_ids,
        spaces,
    })
}

/// 删除单个副本；不递归删除目录，不跟随符号链接。
pub fn remove(root: &Path, doc_id: &str) -> Result<(), String> {
    let file = copy_path(root, doc_id)?;
    match fs::remove_file(file) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// 损坏、缺失、编辑后未重建的副本均不可作为当前检索来源。
pub fn stale_document_ids(conn: &rusqlite::Connection, root: &Path) -> Result<Vec<String>, String> {
    Ok(super::store::list_documents(conn)?.into_iter().filter(|document| {
        is_copy_file_path(&document.file_path) &&
            !matches!(read(root, &document.id), Ok(Some(copy)) if copy.index_status() == "current")
    }).map(|document| document.id).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "lorekeeper-document-copy-{tag}-{}",
            crate::project_access::random_uuid_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn post_commit_copy_failure_stays_stale_and_retry_recovers() {
        let (conn, root) = fixture();
        save(
            &conn,
            &root,
            "doc",
            "新正文".into(),
            &hash_content("旧正文"),
        )
        .unwrap();
        let mut writes = 0;
        let result = reindex_with_writer(&conn, &root, "doc", |root, id, copy| {
            writes += 1;
            if writes == 2 {
                return Err("注入提交后落盘失败".into());
            }
            assert!(copy.index_dirty);
            write(root, id, copy)
        });
        assert!(result.is_err());
        assert_eq!(writes, 2);
        assert_eq!(stale_document_ids(&conn, &root).unwrap(), vec!["doc"]);
        let text: String = conn
            .query_row(
                "SELECT text FROM kb_chunks WHERE doc_id = 'doc'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(text, "新正文");
        reindex(&conn, &root, "doc").unwrap();
        assert!(stale_document_ids(&conn, &root).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    fn fixture() -> (rusqlite::Connection, PathBuf) {
        use crate::db::schema::*;
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        for schema in [
            CREATE_KB_DOCUMENTS,
            CREATE_KB_CHUNKS,
            CREATE_KB_EMBEDDING_SPACES,
            CREATE_KB_FTS,
        ] {
            conn.execute_batch(schema).unwrap();
        }
        let root = temp_root("domain");
        let marker = marker_file_path("doc");
        super::super::store::insert_document(
            &conn,
            "doc",
            "测试.md",
            &["旧正文".to_string()],
            &super::super::store::DocumentMeta {
                file_path: &marker,
                chapter_number: Some(3),
                chapter_title: Some("测试章节"),
                corpus_kind: "project-knowledge",
            },
        )
        .unwrap();
        write(&root, "doc", &KnowledgeCopy::fresh("旧正文")).unwrap();
        (conn, root)
    }

    #[test]
    fn save_keeps_old_index_and_reindex_preserves_metadata() {
        let (conn, root) = fixture();
        let old_ids = super::super::store::document_chunk_ids(&conn, "doc").unwrap();
        let before = super::super::store::document_row(&conn, "doc").unwrap();
        conn.execute_batch("UPDATE kb_documents SET imported_at = 'fixed-time'")
            .unwrap();
        save(
            &conn,
            &root,
            "doc",
            "新正文".into(),
            &hash_content("旧正文"),
        )
        .unwrap();
        assert_eq!(
            read_view(&conn, &root, "doc").unwrap()["indexStatus"],
            "stale"
        );
        assert_eq!(stale_document_ids(&conn, &root).unwrap(), vec!["doc"]);
        assert_eq!(
            super::super::store::document_chunk_ids(&conn, "doc").unwrap(),
            old_ids
        );
        assert!(save(&conn, &root, "doc", "覆盖".into(), &hash_content("旧正文")).is_err());
        let rebuilt = reindex(&conn, &root, "doc").unwrap();
        assert_eq!(rebuilt.previous_ids, old_ids);
        assert_eq!(rebuilt.chunk_count, 1);
        assert_eq!(
            super::super::store::document_row(&conn, "doc").unwrap(),
            before
        );
        assert_eq!(
            read_view(&conn, &root, "doc").unwrap()["indexStatus"],
            "current"
        );
        assert!(stale_document_ids(&conn, &root).unwrap().is_empty());
        let date: String = conn
            .query_row(
                "SELECT imported_at FROM kb_documents WHERE id = 'doc'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(date, "fixed-time");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reindex_sidecar_failure_rolls_back_chunks_fts_and_active_space() {
        let (conn, root) = fixture();
        save(
            &conn,
            &root,
            "doc",
            "新正文".into(),
            &hash_content("旧正文"),
        )
        .unwrap();
        let old_ids = super::super::store::document_chunk_ids(&conn, "doc").unwrap();
        super::super::hybrid::upsert_space(
            &conn,
            &super::super::hybrid::EmbeddingSpace {
                generation: 1,
                dimension: 2,
                model_fingerprint: "test".into(),
                distance_metric: "cosine".into(),
                status: "active".into(),
            },
        )
        .unwrap();
        let result =
            reindex_with_writer(
                &conn,
                &root,
                "doc",
                |_, _, _| Err("注入副本落盘失败".into()),
            );
        assert!(result.is_err());
        assert_eq!(
            super::super::store::document_chunk_ids(&conn, "doc").unwrap(),
            old_ids
        );
        assert!(super::super::hybrid::active_space(&conn).unwrap().is_some());
        let text: String = conn
            .query_row(
                "SELECT text FROM kb_chunks WHERE doc_id = 'doc'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(text, "旧正文");
        let fts_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM kb_fts WHERE doc_id = 'doc'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_count, 1);
        assert_eq!(read(&root, "doc").unwrap().unwrap().index_status(), "stale");
        assert!(conn.is_autocommit());
        reindex(&conn, &root, "doc").unwrap();
        assert!(super::super::hybrid::active_space(&conn).unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_commit_restores_previous_copy() {
        let (conn, root) = fixture();
        save(
            &conn,
            &root,
            "doc",
            "新正文".into(),
            &hash_content("旧正文"),
        )
        .unwrap();
        let before = read(&root, "doc").unwrap();
        // 延迟外键约束令事务在副本写入之后才提交失败。
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
            CREATE TABLE parent(id INTEGER PRIMARY KEY);
            CREATE TABLE child(id INTEGER REFERENCES parent(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TEMP TRIGGER fail_commit AFTER INSERT ON kb_chunks
            BEGIN INSERT INTO child VALUES (7); END;",
        )
        .unwrap();
        assert!(reindex(&conn, &root, "doc").is_err());
        assert_eq!(read(&root, "doc").unwrap(), before);
        assert!(conn.is_autocommit());
        let text: String = conn
            .query_row(
                "SELECT text FROM kb_chunks WHERE doc_id = 'doc'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(text, "旧正文");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_legacy_and_empty_copies_are_not_reconstructed() {
        let (conn, root) = fixture();
        assert_eq!(
            read_view(&conn, &root, "absent").unwrap()["available"],
            false
        );
        conn.execute_batch("UPDATE kb_documents SET file_path = 'legacy.txt'")
            .unwrap();
        assert_eq!(read_view(&conn, &root, "doc").unwrap()["available"], false);
        assert!(save(&conn, &root, "doc", "新".into(), &hash_content("旧正文")).is_err());
        conn.execute_batch("UPDATE kb_documents SET file_path = 'knowledge-copy:doc'")
            .unwrap();
        save(&conn, &root, "doc", " ".into(), &hash_content("旧正文")).unwrap();
        assert!(reindex(&conn, &root, "doc").is_err());
        remove(&root, "doc").unwrap();
        assert_eq!(stale_document_ids(&conn, &root).unwrap(), vec!["doc"]);
        assert!(reindex(&conn, &root, "doc").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn marker_helpers_test() {
        assert_eq!(marker_file_path("doc-1"), "knowledge-copy:doc-1");
        assert!(is_copy_file_path("knowledge-copy:doc-1"));
        assert!(!is_copy_file_path("D:/novels/正文/第一章.txt"));
        assert_eq!(hash_content("甲乙"), hash_content("甲乙"));
        assert_ne!(hash_content("甲乙"), hash_content("甲乙 "));
    }

    #[test]
    fn write_then_read_roundtrip_uses_camel_case_json_test() {
        let root = temp_root("roundtrip");
        let copy = KnowledgeCopy::fresh("第一章正文");
        write(&root, "doc-1", &copy).unwrap();

        let file = root
            .join("knowledge-copies")
            .join(format!("{}.json", hash_content("doc-1")));
        let raw = fs::read_to_string(&file).unwrap();
        assert!(raw.contains("\"indexedHash\""));
        assert!(raw.contains("\"indexDirty\""));

        let read_back = read(&root, "doc-1").unwrap().unwrap();
        assert_eq!(read_back, copy);
        assert_eq!(read_back.index_status(), "current");

        // 只应有副本文件本身，无临时文件残留
        let entries: Vec<_> = fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries.len(), 1);

        // 覆盖写：同一文档第二次写入得到新内容
        let edited = KnowledgeCopy {
            content: "第一章正文（改）".to_string(),
            indexed_hash: hash_content("第一章正文"),
            edited: true,
            index_dirty: true,
        };
        write(&root, "doc-1", &edited).unwrap();
        let read_back = read(&root, "doc-1").unwrap().unwrap();
        assert_eq!(read_back.index_status(), "stale");
        assert!(read_back.edited);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn read_missing_file_is_none_test() {
        let root = temp_root("missing");
        assert!(read(&root, "absent").unwrap().is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn read_rejects_wrong_shape_test() {
        let root = temp_root("shape");
        let directory = root.join("knowledge-copies");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join(format!("{}.json", hash_content("doc-1"))),
            "{\"content\":\"甲\",\"edited\":false}",
        )
        .unwrap();
        assert_eq!(read(&root, "doc-1"), Err(INVALID_COPY.to_string()));
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn read_rejects_symlinked_directory_and_hardlinked_file_test() {
        use std::os::unix::fs::symlink;
        let root = temp_root("unsafe");
        let outside = temp_root("unsafe-outside");
        symlink(&outside, root.join("knowledge-copies")).unwrap();
        assert_eq!(read(&root, "doc-1"), Err(UNSAFE_PATH.to_string()));
        assert_eq!(
            write(&root, "doc-1", &KnowledgeCopy::fresh("甲")),
            Err(UNSAFE_PATH.to_string())
        );
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn read_rejects_hardlinked_file_test() {
        // Windows 和 Unix 都必须拒绝硬链接副本。
        let root = temp_root("hardlink");
        let directory = root.join("knowledge-copies");
        fs::create_dir_all(&directory).unwrap();
        let file = directory.join(format!("{}.json", hash_content("doc-1")));
        fs::write(
            &file,
            "{\"content\":\"甲\",\"indexedHash\":\"x\",\"edited\":false,\"indexDirty\":false}",
        )
        .unwrap();
        fs::hard_link(&file, directory.join("alias.json")).unwrap();
        assert_eq!(read(&root, "doc-1"), Err(UNSAFE_PATH.to_string()));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn project_storage_root_is_lore_directory_test() {
        let root = project_storage_root("D:/novels/甲");
        assert!(root.ends_with("甲/.lore") || root.ends_with("甲\\.lore"));
    }
}
