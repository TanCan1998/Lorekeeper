//! 生成专用知识快照：一次检索，随后只按冻结 ID 复核，不按同名材料替换。
//! 规范数据来自当前项目 SQLite；不创建索引、不迁移旧目录、不请求模型。
use std::collections::HashSet;
use std::path::Path;

use crate::db::kb::{document_copy, fts, hybrid, revisions, store};
use crate::services::generation_source_binding::{
    GenerationKnowledgeItem, GenerationKnowledgeSnapshot, KnowledgeCorpusKind, KnowledgeState,
    KnowledgeStorageState,
};
use rusqlite::{Connection, OptionalExtension};

fn fail(code: &str) -> String {
    format!("GENERATION_KNOWLEDGE_{code}")
}
fn read_error(_: rusqlite::Error) -> String {
    fail("READ_FAILED")
}

fn storage_state(conn: &Connection) -> Result<KnowledgeStorageState, String> {
    let (documents, chunks): (i64, i64) = conn
        .query_row(
            "SELECT (SELECT COUNT(*) FROM kb_documents), (SELECT COUNT(*) FROM kb_chunks)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(read_error)?;
    if (documents == 0) != (chunks == 0) {
        return Err(fail("INCOMPLETE_TABLES"));
    }
    Ok(if documents == 0 {
        KnowledgeStorageState::Empty
    } else {
        KnowledgeStorageState::Present
    })
}

fn source_item(
    conn: &Connection,
    root: &Path,
    id: &str,
    score: f64,
    revision: i64,
) -> Result<GenerationKnowledgeItem, String> {
    if id.is_empty() || !score.is_finite() {
        return Err(fail("INVALID_ROW"));
    }
    let row: Option<(String, i64, String, String, String)> = conn
        .query_row(
            "SELECT doc_id, chunk_index, text, file_name, corpus_kind FROM kb_chunks WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(read_error)?;
    let (doc, index, text, name, kind) = row.ok_or_else(|| fail("SOURCE_STALE"))?;
    let corpus_kind = match kind.as_str() {
        "project-knowledge" => KnowledgeCorpusKind::ProjectKnowledge,
        "unknown" => KnowledgeCorpusKind::Unknown,
        "reference" => return Err(fail("INELIGIBLE_SOURCE")),
        _ => return Err(fail("INVALID_CORPUS")),
    };
    if doc.is_empty() || index < 0 || text.is_empty() || name.is_empty() {
        return Err(fail("INVALID_ROW"));
    }
    let document: Option<(String, String, String)> = conn
        .query_row(
            "SELECT file_name, corpus_kind, file_path FROM kb_documents WHERE id=?1",
            [&doc],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(read_error)?;
    let (doc_name, doc_kind, path) = document.ok_or_else(|| fail("DOCUMENT_INCOMPLETE"))?;
    if doc_name != name || doc_kind != kind {
        return Err(fail("DOCUMENT_INCOMPLETE"));
    }
    if document_copy::is_copy_file_path(&path)
        && !matches!(document_copy::read(root, &doc), Ok(Some(copy)) if copy.index_status() == "current")
    {
        return Err(fail("SOURCE_STALE"));
    }
    let integrity = store::document_integrity(conn, &doc)
        .map_err(|_| fail("READ_FAILED"))?
        .filter(|value| value.complete)
        .ok_or_else(|| fail("DOCUMENT_INCOMPLETE"))?;
    let mismatched: i64 = conn.query_row(
        "SELECT COUNT(*) FROM kb_chunks WHERE doc_id=?1 AND (file_name != ?2 OR typeof(text) != 'text')", [&doc, &name], |r| r.get(0)
    ).map_err(read_error)?;
    if mismatched != 0 || index >= integrity.chunk_count {
        return Err(fail("DOCUMENT_INCOMPLETE"));
    }
    Ok(GenerationKnowledgeItem {
        revision,
        chunk_id: id.to_string(),
        document_id: doc,
        chunk_index: index,
        content_hash: document_copy::hash_content(&text),
        text,
        file_name: name,
        score,
        corpus_kind,
        document_chunk_count: integrity.chunk_count,
        document_chunk_set_hash: integrity.chunk_set_hash,
    })
}

/// 向量支路传入的是索引 ID/距离投影，而非去身份化的通用搜索结果。
/// 无可用向量命中时使用既定的 FTS5/jieba 支路；不扫描 reference 或脏副本。
pub(crate) fn capture(
    conn: &Connection,
    root: &Path,
    query: &str,
    top_k: i64,
    vector_hits: Option<&[(String, f64)]>,
) -> Result<GenerationKnowledgeSnapshot, String> {
    if !(1..=100).contains(&top_k) {
        return Err(fail("INVALID_REQUEST"));
    }
    let before = revisions::read(conn).map_err(read_error)?;
    let storage_state = storage_state(conn)?;
    let stale = document_copy::stale_document_ids(conn, root).map_err(|_| fail("READ_FAILED"))?;
    let excluded = vec!["reference".to_string()];
    let filter = store::SearchFilter {
        excluded_doc_ids: &stale,
        excluded_corpus_kinds: &excluded,
        ..Default::default()
    };
    let mut hits = Vec::new();
    if let Some(vector_hits) = vector_hits {
        let ids = vector_hits
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        let rows = store::chunk_rows(conn, &ids).map_err(|_| fail("READ_FAILED"))?;
        for (id, score) in vector_hits {
            let row = rows
                .iter()
                .find(|row| row.id == *id)
                .ok_or_else(|| fail("VECTOR_CANONICAL_MISMATCH"))?;
            if store::matches_filter(row, &filter) {
                hits.push((id.clone(), *score));
                if hits.len() == top_k as usize {
                    break;
                }
            }
        }
    }
    if hits.is_empty() && storage_state == KnowledgeStorageState::Present {
        // 排除脏文档及 reference 后才截断 topK，避免无效候选耗尽窗口。
        let limit: i64 = conn
            .query_row("SELECT COUNT(*) FROM kb_chunks", [], |r| r.get(0))
            .map_err(read_error)?;
        let recalled = fts::search(conn, query, limit as usize).map_err(read_error)?;
        let ids = recalled
            .iter()
            .map(|hit| hit.chunk_id.clone())
            .collect::<Vec<_>>();
        let rows = store::chunk_rows(conn, &ids).map_err(|_| fail("READ_FAILED"))?;
        let terms = hybrid::extract_query_terms(query);
        for hit in recalled {
            let row = rows
                .iter()
                .find(|row| row.id == hit.chunk_id)
                .ok_or_else(|| fail("SOURCE_MISMATCH"))?;
            if store::matches_filter(row, &filter) {
                hits.push((row.id.clone(), hybrid::text_relevance(&row.text, &terms)));
            }
        }
        hits.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        hits.truncate(top_k as usize);
    }
    let mut seen = HashSet::new();
    let items = hits
        .iter()
        .map(|(id, score)| {
            if !seen.insert(id) {
                return Err(fail("DUPLICATE_CHUNK"));
            }
            source_item(conn, root, id, *score, before.canonical)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let snapshot = GenerationKnowledgeSnapshot {
        version: 1,
        state: if items.is_empty() {
            KnowledgeState::Empty
        } else {
            KnowledgeState::Stored
        },
        storage_state,
        query: query.to_string(),
        top_k,
        canonical_revision: Some(before.canonical),
        documents_revision: Some(before.documents),
        items,
    };
    if revisions::read(conn).map_err(read_error)? != before {
        return Err(fail("CHANGED_DURING_READ"));
    }
    verify(conn, root, &snapshot)?;
    Ok(snapshot)
}

/// 不发嵌入请求、不重搜；包括空结果也绑定规范库修订，新增材料后必须重新准备。
pub(crate) fn verify(
    conn: &Connection,
    root: &Path,
    snapshot: &GenerationKnowledgeSnapshot,
) -> Result<(), String> {
    if snapshot.version != 1
        || !(1..=100).contains(&snapshot.top_k)
        || snapshot.items.len() > snapshot.top_k as usize
        || snapshot.state
            != if snapshot.items.is_empty() {
                KnowledgeState::Empty
            } else {
                KnowledgeState::Stored
            }
    {
        return Err(fail("INVALID_SNAPSHOT"));
    }
    let before = revisions::read(conn).map_err(read_error)?;
    if snapshot.canonical_revision != Some(before.canonical)
        || snapshot.documents_revision != Some(before.documents)
        || snapshot.storage_state != storage_state(conn)?
    {
        return Err(fail("SOURCE_STALE"));
    }
    let mut seen = HashSet::new();
    for item in &snapshot.items {
        if !seen.insert(&item.chunk_id) {
            return Err(fail("INVALID_SNAPSHOT"));
        }
        if source_item(conn, root, &item.chunk_id, item.score, before.canonical)? != *item {
            return Err(fail("SOURCE_STALE"));
        }
    }
    if revisions::read(conn).map_err(read_error)? != before {
        return Err(fail("CHANGED_DURING_READ"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        schema::create_tables(&conn).unwrap();
        conn
    }
    fn add(conn: &Connection, id: &str, kind: &str) -> Vec<String> {
        store::insert_document(
            conn,
            id,
            "合成材料",
            &["测试角色核对合成卡片".into(), "隔离知识原文".into()],
            &store::DocumentMeta {
                corpus_kind: kind,
                ..Default::default()
            },
        )
        .unwrap()
    }
    fn capture_text(conn: &Connection) -> GenerationKnowledgeSnapshot {
        capture(conn, Path::new("."), "测试角色", 5, None).unwrap()
    }

    #[test]
    fn stale_copy_is_excluded_before_top_k_and_invalidates_frozen_source() {
        let conn = conn();
        let root = std::env::temp_dir().join(format!(
            "lorekeeper-knowledge-copy-proof-{}",
            crate::project_access::random_uuid_v4()
        ));
        std::fs::create_dir(&root).unwrap();
        let id = "copy-doc";
        let content = "测试角色读取合成卡片";
        document_copy::write(&root, id, &document_copy::KnowledgeCopy::fresh(content)).unwrap();
        store::insert_document(
            &conn,
            id,
            "合成副本",
            &[content.into()],
            &store::DocumentMeta {
                file_path: &document_copy::marker_file_path(id),
                corpus_kind: "project-knowledge",
                ..Default::default()
            },
        )
        .unwrap();
        let frozen = capture(&conn, &root, "测试角色", 1, None).unwrap();
        assert_eq!(frozen.items[0].document_id, id);
        document_copy::save(
            &conn,
            &root,
            id,
            "编辑后的测试角色材料".into(),
            &document_copy::hash_content(content),
        )
        .unwrap();
        assert!(verify(&conn, &root, &frozen).is_err());
        add(&conn, "eligible", "project-knowledge");
        add(&conn, "reference", "reference");
        let current = capture(&conn, &root, "测试角色", 1, None).unwrap();
        assert_eq!(current.items.len(), 1);
        assert_eq!(current.items[0].document_id, "eligible");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_snapshot_is_real_and_becomes_stale_when_material_is_added() {
        let conn = conn();
        let snapshot = capture_text(&conn);
        assert_eq!(snapshot.storage_state, KnowledgeStorageState::Empty);
        assert_eq!(snapshot.canonical_revision, Some(0));
        verify(&conn, Path::new("."), &snapshot).unwrap();
        add(&conn, "new", "project-knowledge");
        assert!(verify(&conn, Path::new("."), &snapshot).is_err());
    }

    #[test]
    fn text_capture_preserves_identity_and_excludes_reference() {
        let conn = conn();
        add(&conn, "reference", "reference");
        let ids = add(&conn, "project", "project-knowledge");
        let snapshot = capture_text(&conn);
        assert_eq!(snapshot.items.len(), 1);
        let item = &snapshot.items[0];
        assert_eq!(item.chunk_id, ids[0]);
        assert_eq!(item.document_id, "project");
        assert_eq!(item.document_chunk_count, 2);
        assert_eq!(item.content_hash, document_copy::hash_content(&item.text));
        verify(&conn, Path::new("."), &snapshot).unwrap();
    }

    #[test]
    fn exact_ids_and_complete_document_are_required_after_rehashed_tampering() {
        let conn = conn();
        add(&conn, "doc", "project-knowledge");
        let snapshot = capture_text(&conn);
        for field in [
            "chunkId",
            "documentId",
            "chunkIndex",
            "text",
            "fileName",
            "contentHash",
            "documentChunkCount",
            "documentChunkSetHash",
            "revision",
        ] {
            let mut value = serde_json::to_value(&snapshot).unwrap();
            value["items"][0][field] = if value["items"][0][field].is_number() {
                serde_json::json!(999)
            } else {
                serde_json::json!("合成篡改")
            };
            let changed = serde_json::from_value(value).unwrap();
            assert!(verify(&conn, Path::new("."), &changed).is_err(), "{field}");
        }
        // 即使攻击者自行重算正文哈希，物理原文仍不一致。
        let mut changed = snapshot.clone();
        changed.items[0].text = "合成伪造原文".into();
        changed.items[0].content_hash = document_copy::hash_content(&changed.items[0].text);
        assert!(verify(&conn, Path::new("."), &changed).is_err());
        // 未选中的另一块变化也使冻结的整文档证明失效。
        conn.execute(
            "UPDATE kb_chunks SET text='变更未选块' WHERE chunk_index=1",
            [],
        )
        .unwrap();
        assert!(verify(&conn, Path::new("."), &snapshot).is_err());
    }

    #[test]
    fn incomplete_documents_duplicate_ids_and_invalid_snapshots_are_rejected() {
        let conn = conn();
        add(&conn, "doc", "project-knowledge");
        let snapshot = capture_text(&conn);
        let mut duplicate = snapshot.clone();
        duplicate.items.push(duplicate.items[0].clone());
        assert!(verify(&conn, Path::new("."), &duplicate).is_err());
        for (version, top_k) in [(0, 5), (1, 0), (1, 101)] {
            let mut invalid = snapshot.clone();
            invalid.version = version;
            invalid.top_k = top_k;
            assert!(verify(&conn, Path::new("."), &invalid).is_err());
        }
        conn.execute("DELETE FROM kb_chunks WHERE chunk_index=1", [])
            .unwrap();
        assert!(capture(&conn, Path::new("."), "测试角色", 5, None).is_err());
    }

    #[test]
    fn vector_ids_bind_canonical_sources_and_do_not_research_on_verify() {
        let conn = conn();
        let ids = add(&conn, "doc", "project-knowledge");
        let snapshot = capture(
            &conn,
            Path::new("."),
            "无关键词命中",
            5,
            Some(&[(ids[1].clone(), 0.8)]),
        )
        .unwrap();
        assert_eq!(snapshot.items[0].chunk_id, ids[1]);
        fts::clear(&conn).unwrap();
        verify(&conn, Path::new("."), &snapshot).unwrap();
        assert!(capture(
            &conn,
            Path::new("."),
            "查询",
            5,
            Some(&[("missing".into(), 0.8)])
        )
        .is_err());
        assert!(capture(
            &conn,
            Path::new("."),
            "查询",
            5,
            Some(&[(ids[0].clone(), f64::NAN)])
        )
        .is_err());
    }
}
