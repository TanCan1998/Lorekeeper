//! import_generation_source —— 导入素材的生成上下文冻结（对齐
//! `electron/services/import-generation-source.ts`）。
//!
//! 从持久化数据库读取不可变导入素材（`import_runs` /
//! `import_run_chapters` / `project_core`），绝不采样渲染层数据。
//!
//! 语义对齐要点：
//! - 错误码与基线逐字一致（`GENERATION_IMPORT_SLOT_INVALID` 等五值；
//!   批次检查点构造失败文案为中文，与 `src/shared/import-run.ts` 逐字一致）；
//! - `slot.runId.length` / `slot.batchId.length` 为 UTF-16 码元数
//!   （`encode_utf16().count()`）；
//! - `countDraftUnits` 直接复用 `crate::draft_units::count_draft_units`；
//! - `createImportRunChapterBatchCheckpointId` /
//!   `parseImportRunChapterBatchCheckpointId` 仅移植这两个函数及其
//!   内部依赖（两个正则），来源 `src/shared/import-run.ts:99-130`。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Map;
use sha2::{Digest, Sha256};

use crate::draft_units::count_draft_units;
use crate::repositories::project_core_repository;
use crate::services::import_generation::{
    ImportGenerationChapter, ImportGenerationContext, ImportGenerationSlot,
    ImportGenerationSlotStage,
};
use crate::services::source_ref::is_content_hash;

const GENERATION_IMPORT_SLOT_INVALID: &str = "GENERATION_IMPORT_SLOT_INVALID";
const GENERATION_IMPORT_MANIFEST_REQUIRED: &str = "GENERATION_IMPORT_MANIFEST_REQUIRED";
const GENERATION_IMPORT_CONTENT_CHANGED: &str = "GENERATION_IMPORT_CONTENT_CHANGED";
const GENERATION_IMPORT_CHECKPOINT_CHANGED: &str = "GENERATION_IMPORT_CHECKPOINT_CHANGED";
const GENERATION_IMPORT_CONTEXT_LIMIT: &str = "GENERATION_IMPORT_CONTEXT_LIMIT";

/// 上下文体积上限（基线 `8 * 1024 * 1024` UTF-8 字节）。
const IMPORT_CONTEXT_BYTE_LIMIT: usize = 8 * 1024 * 1024;

/// 槽位键长度上限（基线 `slot.runId.length > 256`，UTF-16 码元）。
const SLOT_KEY_MAX_UTF16_UNITS: usize = 256;

/// `ImportRunChapterBatchCheckpoint`（对齐 `src/shared/import-run.ts`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRunChapterBatchCheckpoint {
    pub start_chapter: i64,
    pub end_chapter: i64,
    pub content_fingerprint_prefixes: Vec<String>,
}

/// `createImportRunChapterBatchCheckpointId` 的输入
/// （基线 `Pick<ImportRunChapterInput, 'number' | 'contentFingerprint'>`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRunChapterCheckpointEntry {
    pub number: i64,
    pub content_fingerprint: String,
}

/// `validateImportGenerationSlot`：槽位形状与阶段语义校验。
pub fn validate_import_generation_slot(slot: &ImportGenerationSlot) -> Result<(), String> {
    let valid = !slot.run_id.trim().is_empty()
        && slot.run_id.encode_utf16().count() <= SLOT_KEY_MAX_UTF16_UNITS
        && slot.batch_id.encode_utf16().count() <= SLOT_KEY_MAX_UTF16_UNITS
        && match slot.stage {
            // 非 blueprints 阶段批次标识恒为 'done'；
            // blueprints 阶段必须可解析为章批次检查点。
            ImportGenerationSlotStage::Blueprints => {
                parse_import_run_chapter_batch_checkpoint_id(&slot.batch_id).is_some()
            }
            ImportGenerationSlotStage::Global | ImportGenerationSlotStage::Style => {
                slot.batch_id == "done"
            }
        };
    if valid {
        Ok(())
    } else {
        Err(GENERATION_IMPORT_SLOT_INVALID.to_string())
    }
}

/// `importGenerationSlotKey`：槽位的稳定哈希键。
pub fn import_generation_slot_key(slot: &ImportGenerationSlot) -> Result<String, String> {
    validate_import_generation_slot(slot)?;
    let canonical = serde_json::to_string(&[
        slot.run_id.as_str(),
        slot_stage_name(slot.stage),
        slot.batch_id.as_str(),
    ])
    .map_err(|error| format!("GENERATION_IMPORT_SLOT_KEY_ENCODE_ERROR: {error}"))?;
    Ok(text_hash(&canonical))
}

/// 阶段名的规范字符串（对齐 `ImportGenerationSlotStage` 的
/// serde `rename_all = "lowercase"` 序列化结果；上游
/// `importGenerationSlotKey` 直接 `JSON.stringify` 该字符串名）。
fn slot_stage_name(stage: ImportGenerationSlotStage) -> &'static str {
    match stage {
        ImportGenerationSlotStage::Global => "global",
        ImportGenerationSlotStage::Style => "style",
        ImportGenerationSlotStage::Blueprints => "blueprints",
    }
}

/// `captureImportGenerationContext`：从持久化数据库读取不可变导入素材。
pub fn capture_import_generation_context(
    conn: &Connection,
    slot: &ImportGenerationSlot,
) -> Result<ImportGenerationContext, String> {
    validate_import_generation_slot(slot)?;
    let run = conn
        .query_row(
            "SELECT purpose, manifest_fingerprint, manifest_chapter_count, manifest_word_count \
             FROM import_runs WHERE id = ?1",
            [&slot.run_id],
            |row| {
                Ok(ImportRunRow {
                    purpose: row.get("purpose")?,
                    manifest_fingerprint: row.get("manifest_fingerprint")?,
                    manifest_chapter_count: row.get("manifest_chapter_count")?,
                    manifest_word_count: row.get("manifest_word_count")?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("读取导入运行台账失败：{error}"))?;
    let Some(run) = run else {
        return Err(GENERATION_IMPORT_MANIFEST_REQUIRED.to_string());
    };
    if run.purpose != "reference" || !is_content_hash(&run.manifest_fingerprint) {
        return Err(GENERATION_IMPORT_MANIFEST_REQUIRED.to_string());
    }

    let range = match slot.stage {
        ImportGenerationSlotStage::Blueprints => {
            parse_import_run_chapter_batch_checkpoint_id(&slot.batch_id)
        }
        _ => None,
    };
    let rows = load_chapter_rows(conn, &slot.run_id)?;
    let selected: Vec<ImportRunChapterRow> = if let Some(range) = &range {
        rows.into_iter()
            .filter(|row| {
                row.chapter_number >= range.start_chapter && row.chapter_number <= range.end_chapter
            })
            .collect()
    } else if slot.stage == ImportGenerationSlotStage::Global {
        // global：首章 + 末章
        let last = rows.len().saturating_sub(1);
        rows.into_iter()
            .enumerate()
            .filter(|(index, _)| *index == 0 || *index == last)
            .map(|(_, row)| row)
            .collect()
    } else {
        // style：前 3 章 + 末 2 章（≤5 章时全取）
        let total = rows.len();
        rows.into_iter()
            .enumerate()
            .filter(|(index, _)| total <= 5 || *index < 3 || *index >= total - 2)
            .map(|(_, row)| row)
            .collect()
    };
    if selected.is_empty()
        || selected
            .iter()
            .any(|row| text_hash(&row.content_snapshot) != row.content_fingerprint)
    {
        return Err(GENERATION_IMPORT_CONTENT_CHANGED.to_string());
    }

    let chapters: Vec<ImportGenerationChapter> = selected
        .iter()
        .map(|row| ImportGenerationChapter {
            number: row.chapter_number,
            title: row.title.clone(),
            content: row.content_snapshot.clone(),
            content_fingerprint: row.content_fingerprint.clone(),
            word_count: count_draft_units(&row.content_snapshot),
        })
        .collect();
    if range.is_some() {
        let entries: Vec<ImportRunChapterCheckpointEntry> = chapters
            .iter()
            .map(|chapter| ImportRunChapterCheckpointEntry {
                number: chapter.number,
                content_fingerprint: chapter.content_fingerprint.clone(),
            })
            .collect();
        let checkpoint = create_import_run_chapter_batch_checkpoint_id(&entries)?;
        if checkpoint != slot.batch_id {
            return Err(GENERATION_IMPORT_CHECKPOINT_CHANGED.to_string());
        }
    }

    let core = project_core_repository::get(conn)?;
    let Some(core) = core else {
        return Err(GENERATION_IMPORT_CONTEXT_LIMIT.to_string());
    };
    let serialized_bytes = serde_json::to_vec(&chapters)
        .map_err(|error| format!("GENERATION_IMPORT_CONTEXT_ENCODE_ERROR: {error}"))?;
    if serialized_bytes.len() > IMPORT_CONTEXT_BYTE_LIMIT {
        return Err(GENERATION_IMPORT_CONTEXT_LIMIT.to_string());
    }

    Ok(ImportGenerationContext {
        slot: slot.clone(),
        manifest_fingerprint: run.manifest_fingerprint,
        total_chapters: run.manifest_chapter_count,
        total_words: run.manifest_word_count,
        chapters,
        core,
        prompts: Map::new(),
    })
}

/// `createImportRunChapterBatchCheckpointId`：批次检查点的规范身份
/// （蓝图分批、持久收据与仓储检查点共享；来源 `src/shared/import-run.ts:99-118`）。
pub fn create_import_run_chapter_batch_checkpoint_id(
    chapters: &[ImportRunChapterCheckpointEntry],
) -> Result<String, String> {
    if chapters.is_empty() {
        return Err("导入章节批次不能为空".to_string());
    }
    for (index, chapter) in chapters.iter().enumerate() {
        if !is_safe_integer(chapter.number) || chapter.number < 1 {
            return Err("导入章节批次章号无效".to_string());
        }
        if !is_content_hash(&chapter.content_fingerprint) {
            return Err("导入章节批次内容指纹无效".to_string());
        }
        if index > 0 && chapter.number != chapters[index - 1].number + 1 {
            return Err("导入章节批次必须连续".to_string());
        }
    }
    let prefixes: Vec<String> = chapters
        .iter()
        .map(|chapter| chapter.content_fingerprint[..8].to_string())
        .collect();
    let first = chapters.first().map(|chapter| chapter.number).unwrap_or(0);
    let last = chapters.last().map(|chapter| chapter.number).unwrap_or(0);
    Ok(format!("{first}-{last}-{}", prefixes.join(".")))
}

/// `parseImportRunChapterBatchCheckpointId`：批次检查点解析
/// （来源 `src/shared/import-run.ts:120-130`；非法输入返回 `None`）。
pub fn parse_import_run_chapter_batch_checkpoint_id(
    checkpoint_id: &str,
) -> Option<ImportRunChapterBatchCheckpoint> {
    let pattern =
        regex::Regex::new(r"^([1-9]\d*)-([1-9]\d*)-([a-f0-9]{8}(?:\.[a-f0-9]{8})*)$").ok()?;
    let captures = pattern.captures(checkpoint_id)?;
    let start_chapter = captures.get(1)?.as_str().parse::<i64>().ok()?;
    let end_chapter = captures.get(2)?.as_str().parse::<i64>().ok()?;
    let content_fingerprint_prefixes: Vec<String> = captures
        .get(3)?
        .as_str()
        .split('.')
        .map(str::to_string)
        .collect();
    if !is_safe_integer(start_chapter)
        || !is_safe_integer(end_chapter)
        || end_chapter < start_chapter
        || content_fingerprint_prefixes.len() as i64 != end_chapter - start_chapter + 1
    {
        return None;
    }
    Some(ImportRunChapterBatchCheckpoint {
        start_chapter,
        end_chapter,
        content_fingerprint_prefixes,
    })
}

#[derive(Debug)]
struct ImportRunRow {
    purpose: String,
    manifest_fingerprint: String,
    manifest_chapter_count: i64,
    manifest_word_count: i64,
}

#[derive(Debug)]
struct ImportRunChapterRow {
    chapter_number: i64,
    title: String,
    content_snapshot: String,
    content_fingerprint: String,
}

fn load_chapter_rows(conn: &Connection, run_id: &str) -> Result<Vec<ImportRunChapterRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT chapter_number, title, content_snapshot, content_fingerprint \
             FROM import_run_chapters WHERE run_id = ?1 ORDER BY chapter_number",
        )
        .map_err(|error| format!("读取导入章节快照失败：{error}"))?;
    let mut rows = stmt
        .query([run_id])
        .map_err(|error| format!("读取导入章节快照失败：{error}"))?;
    let mut result = Vec::new();
    while let Ok(Some(row)) = rows.next() {
        result.push(ImportRunChapterRow {
            chapter_number: row
                .get("chapter_number")
                .map_err(|error| error.to_string())?,
            title: row.get("title").map_err(|error| error.to_string())?,
            content_snapshot: row
                .get("content_snapshot")
                .map_err(|error| error.to_string())?,
            content_fingerprint: row
                .get("content_fingerprint")
                .map_err(|error| error.to_string())?,
        });
    }
    Ok(result)
}

/// UTF-8 字节 SHA-256 小写 hex（基线 `textHash`；各文件内私有，先例
/// `generation_run_repository.rs` 的 `text_hash`）。
fn text_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `Number.isSafeInteger` 等价物（JS 安全整数为 ±2^53-1；Rust i64 可能超出）。
fn is_safe_integer(value: i64) -> bool {
    (-9007199254740991..=9007199254740991).contains(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;

    fn seeded_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        conn
    }

    fn slot(stage: ImportGenerationSlotStage, batch_id: &str) -> ImportGenerationSlot {
        ImportGenerationSlot {
            run_id: "run-1".to_string(),
            stage,
            batch_id: batch_id.to_string(),
        }
    }

    fn valid_fingerprint(text: &str) -> String {
        text_hash(text)
    }

    #[test]
    fn validate_slot_accepts_done_batches_test() {
        assert!(
            validate_import_generation_slot(&slot(ImportGenerationSlotStage::Global, "done"))
                .is_ok()
        );
        assert!(
            validate_import_generation_slot(&slot(ImportGenerationSlotStage::Style, "done"))
                .is_ok()
        );
        assert!(validate_import_generation_slot(&slot(
            ImportGenerationSlotStage::Blueprints,
            "1-1-aaaaaaaa"
        ))
        .is_ok());
    }

    #[test]
    fn validate_slot_rejects_invalid_shapes_test() {
        assert_eq!(
            validate_import_generation_slot(&slot(
                ImportGenerationSlotStage::Global,
                "1-1-aaaaaaaa"
            ))
            .unwrap_err(),
            "GENERATION_IMPORT_SLOT_INVALID",
            "global 阶段批次标识必须为 done"
        );
        assert_eq!(
            validate_import_generation_slot(&slot(ImportGenerationSlotStage::Blueprints, "done"))
                .unwrap_err(),
            "GENERATION_IMPORT_SLOT_INVALID",
            "blueprints 阶段批次标识必须为检查点"
        );
        assert!(
            validate_import_generation_slot(&ImportGenerationSlot {
                run_id: " ".to_string(),
                ..slot(ImportGenerationSlotStage::Global, "done")
            })
            .is_err(),
            "空白 runId 应拒绝"
        );
        assert!(
            validate_import_generation_slot(&ImportGenerationSlot {
                run_id: "x".repeat(257),
                ..slot(ImportGenerationSlotStage::Global, "done")
            })
            .is_err(),
            "runId 超 256 UTF-16 码元应拒绝"
        );
        assert!(
            validate_import_generation_slot(&ImportGenerationSlot {
                batch_id: "y".repeat(257),
                ..slot(ImportGenerationSlotStage::Global, "done")
            })
            .is_err(),
            "batchId 超 256 UTF-16 码元应拒绝"
        );
        // 代理对按 UTF-16 码元计：128 个代理对 = 256 码元（合法），129 个 = 258（拒绝）
        assert!(validate_import_generation_slot(&ImportGenerationSlot {
            run_id: "\u{1D400}".repeat(128),
            ..slot(ImportGenerationSlotStage::Global, "done")
        })
        .is_ok());
    }

    #[test]
    fn slot_key_is_stable_and_distinct_test() {
        let global = slot(ImportGenerationSlotStage::Global, "done");
        let key = import_generation_slot_key(&global).unwrap();
        assert_eq!(key, import_generation_slot_key(&global).unwrap());
        assert_eq!(key.len(), 64);
        assert_ne!(
            key,
            import_generation_slot_key(&slot(ImportGenerationSlotStage::Style, "done")).unwrap()
        );
        assert!(
            import_generation_slot_key(&slot(ImportGenerationSlotStage::Blueprints, "done"))
                .is_err(),
            "非法槽位不应生成键"
        );
    }

    #[test]
    fn checkpoint_round_trip_test() {
        let chapters = vec![
            ImportRunChapterCheckpointEntry {
                number: 1,
                content_fingerprint: "a".repeat(64),
            },
            ImportRunChapterCheckpointEntry {
                number: 2,
                content_fingerprint: "b".repeat(64),
            },
        ];
        let checkpoint_id = create_import_run_chapter_batch_checkpoint_id(&chapters).unwrap();
        assert_eq!(
            checkpoint_id,
            format!("1-2-{}.{}", "a".repeat(8), "b".repeat(8))
        );
        let parsed = parse_import_run_chapter_batch_checkpoint_id(&checkpoint_id).unwrap();
        assert_eq!(parsed.start_chapter, 1);
        assert_eq!(parsed.end_chapter, 2);
        assert_eq!(
            parsed.content_fingerprint_prefixes,
            vec!["aaaaaaaa".to_string(), "bbbbbbbb".to_string()]
        );
    }

    #[test]
    fn checkpoint_creation_rejects_bad_input_test() {
        assert_eq!(
            create_import_run_chapter_batch_checkpoint_id(&[]).unwrap_err(),
            "导入章节批次不能为空"
        );
        assert_eq!(
            create_import_run_chapter_batch_checkpoint_id(&[ImportRunChapterCheckpointEntry {
                number: 0,
                content_fingerprint: "a".repeat(64),
            }])
            .unwrap_err(),
            "导入章节批次章号无效"
        );
        assert_eq!(
            create_import_run_chapter_batch_checkpoint_id(&[ImportRunChapterCheckpointEntry {
                number: 1,
                content_fingerprint: "xyz".to_string(),
            }])
            .unwrap_err(),
            "导入章节批次内容指纹无效"
        );
        assert_eq!(
            create_import_run_chapter_batch_checkpoint_id(&[
                ImportRunChapterCheckpointEntry {
                    number: 1,
                    content_fingerprint: "a".repeat(64),
                },
                ImportRunChapterCheckpointEntry {
                    number: 3,
                    content_fingerprint: "b".repeat(64),
                },
            ])
            .unwrap_err(),
            "导入章节批次必须连续"
        );
    }

    #[test]
    fn checkpoint_parse_rejects_bad_format_test() {
        assert!(parse_import_run_chapter_batch_checkpoint_id("nope").is_none());
        assert!(
            parse_import_run_chapter_batch_checkpoint_id("0-1-aaaaaaaa").is_none(),
            "章号须从 1 起"
        );
        assert!(
            parse_import_run_chapter_batch_checkpoint_id("2-1-aaaaaaaa").is_none(),
            "end < start 应拒绝"
        );
        assert!(
            parse_import_run_chapter_batch_checkpoint_id("1-2-aaaaaaaa").is_none(),
            "前缀数量不符"
        );
        assert!(
            parse_import_run_chapter_batch_checkpoint_id("1-2-aaaaaaaa.bbbbbbbb.cccccccccc")
                .is_none(),
            "前缀数量超过章号跨度应拒绝"
        );
    }

    fn seed_import_run(
        conn: &Connection,
        manifest_fingerprint: &str,
        total_chapters: i64,
        manifest_chapter_count: i64,
        manifest_word_count: i64,
    ) {
        conn.execute(
            "INSERT INTO import_runs (id, root_run_id, effect_namespace, source_fingerprint, \
             manifest_fingerprint, locale, total_chapters, manifest_chapter_count, manifest_word_count) \
             VALUES ('run-1', 'run-1', 'import:reference:run-1', 'src-fp', ?1, 'zh-CN', ?2, ?3, ?4)",
            rusqlite::params![
                manifest_fingerprint,
                total_chapters,
                manifest_chapter_count,
                manifest_word_count
            ],
        )
        .unwrap();
    }

    fn seed_import_chapters(conn: &Connection, snapshots: &[(&str, &str)]) {
        for (index, (content, fingerprint)) in snapshots.iter().enumerate() {
            conn.execute(
                "INSERT INTO import_run_chapters (run_id, chapter_number, title, content_snapshot, \
                 content_fingerprint, content_size) VALUES ('run-1', ?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    index as i64 + 1,
                    format!("第{}章", index + 1),
                    content,
                    fingerprint,
                    content.chars().count() as i64
                ],
            )
            .unwrap();
        }
    }

    #[test]
    fn capture_global_context_selects_first_and_last_test() {
        let conn = seeded_db();
        seed_import_run(&conn, &valid_fingerprint("manifest"), 3, 3, 9);
        seed_import_chapters(
            &conn,
            &[
                ("第一章正文", &valid_fingerprint("第一章正文")),
                ("第二章正文", &valid_fingerprint("第二章正文")),
                ("第三章正文", &valid_fingerprint("第三章正文")),
            ],
        );
        project_core_repository::init(&conn, "项目", "zh-CN").unwrap();

        let context = capture_import_generation_context(
            &conn,
            &slot(ImportGenerationSlotStage::Global, "done"),
        )
        .unwrap();
        assert_eq!(context.slot.run_id, "run-1");
        assert_eq!(context.manifest_fingerprint.len(), 64);
        assert_eq!(context.total_chapters, 3);
        assert_eq!(context.total_words, 9);
        assert_eq!(
            context
                .chapters
                .iter()
                .map(|chapter| chapter.number)
                .collect::<Vec<_>>(),
            vec![1, 3],
            "global 阶段取首末章"
        );
        assert_eq!(context.chapters[0].content, "第一章正文");
        assert_eq!(
            context.chapters[0].word_count,
            count_draft_units("第一章正文")
        );
        assert!(context.prompts.is_empty());
        assert_eq!(context.core.project_name, "项目");
    }

    #[test]
    fn capture_style_stage_samples_boundaries_test() {
        let conn = seeded_db();
        seed_import_run(&conn, &valid_fingerprint("manifest"), 6, 6, 6);
        let snapshots: Vec<(String, String)> = (1..=6)
            .map(|index| {
                let content = format!("第{index}章正文");
                let fingerprint = valid_fingerprint(&content);
                (content, fingerprint)
            })
            .collect();
        let pairs: Vec<(&str, &str)> = snapshots
            .iter()
            .map(|(content, fingerprint)| (content.as_str(), fingerprint.as_str()))
            .collect();
        seed_import_chapters(&conn, &pairs);
        project_core_repository::init(&conn, "项目", "zh-CN").unwrap();

        let context = capture_import_generation_context(
            &conn,
            &slot(ImportGenerationSlotStage::Style, "done"),
        )
        .unwrap();
        assert_eq!(
            context
                .chapters
                .iter()
                .map(|chapter| chapter.number)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 5, 6],
            "style 阶段取前 3 章 + 末 2 章"
        );
    }

    #[test]
    fn capture_rejects_changed_content_test() {
        let conn = seeded_db();
        seed_import_run(&conn, &valid_fingerprint("manifest"), 1, 1, 1);
        seed_import_chapters(&conn, &[("正文", &"0".repeat(64))]);
        project_core_repository::init(&conn, "项目", "zh-CN").unwrap();
        let error = capture_import_generation_context(
            &conn,
            &slot(ImportGenerationSlotStage::Global, "done"),
        )
        .unwrap_err();
        assert_eq!(error, "GENERATION_IMPORT_CONTENT_CHANGED");
    }

    #[test]
    fn capture_rejects_missing_run_stale_purpose_and_missing_core_test() {
        let conn = seeded_db();
        // 无 import_runs 行
        let error = capture_import_generation_context(
            &conn,
            &slot(ImportGenerationSlotStage::Global, "done"),
        )
        .unwrap_err();
        assert_eq!(error, "GENERATION_IMPORT_MANIFEST_REQUIRED");
        // 有 run 但 project_core 未初始化
        seed_import_run(&conn, &valid_fingerprint("manifest"), 1, 1, 1);
        seed_import_chapters(&conn, &[("正文", &valid_fingerprint("正文"))]);
        let error = capture_import_generation_context(
            &conn,
            &slot(ImportGenerationSlotStage::Global, "done"),
        )
        .unwrap_err();
        assert_eq!(error, "GENERATION_IMPORT_CONTEXT_LIMIT");
        // purpose 非 reference
        project_core_repository::init(&conn, "项目", "zh-CN").unwrap();
        conn.execute(
            "UPDATE import_runs SET purpose = 'author-manuscript' WHERE id = 'run-1'",
            [],
        )
        .unwrap();
        let error = capture_import_generation_context(
            &conn,
            &slot(ImportGenerationSlotStage::Global, "done"),
        )
        .unwrap_err();
        assert_eq!(error, "GENERATION_IMPORT_MANIFEST_REQUIRED");
    }

    #[test]
    fn capture_blueprints_verifies_checkpoint_id_test() {
        let conn = seeded_db();
        let first_fingerprint = valid_fingerprint("第一章正文");
        let second_fingerprint = valid_fingerprint("第二章正文");
        seed_import_run(&conn, &valid_fingerprint("manifest"), 2, 2, 2);
        seed_import_chapters(
            &conn,
            &[
                ("第一章正文", &first_fingerprint),
                ("第二章正文", &second_fingerprint),
            ],
        );
        project_core_repository::init(&conn, "项目", "zh-CN").unwrap();

        let batch_id = create_import_run_chapter_batch_checkpoint_id(&[
            ImportRunChapterCheckpointEntry {
                number: 1,
                content_fingerprint: first_fingerprint,
            },
            ImportRunChapterCheckpointEntry {
                number: 2,
                content_fingerprint: second_fingerprint.clone(),
            },
        ])
        .unwrap();
        let context = capture_import_generation_context(
            &conn,
            &slot(ImportGenerationSlotStage::Blueprints, &batch_id),
        )
        .unwrap();
        assert_eq!(context.chapters.len(), 2);
        assert_eq!(context.slot.batch_id, batch_id);

        // 批次 ID 与库内内容指纹前缀不一致 → 检查点已变化
        let stale_batch_id = create_import_run_chapter_batch_checkpoint_id(&[
            ImportRunChapterCheckpointEntry {
                number: 1,
                content_fingerprint: "b".repeat(64),
            },
            ImportRunChapterCheckpointEntry {
                number: 2,
                content_fingerprint: second_fingerprint,
            },
        ])
        .unwrap();
        let error = capture_import_generation_context(
            &conn,
            &slot(ImportGenerationSlotStage::Blueprints, &stale_batch_id),
        )
        .unwrap_err();
        assert_eq!(error, "GENERATION_IMPORT_CHECKPOINT_CHANGED");
    }

    #[test]
    fn capture_blueprints_range_filters_chapters_test() {
        let conn = seeded_db();
        let fingerprints: Vec<String> = (1..=4)
            .map(|index| valid_fingerprint(&format!("第{index}章正文")))
            .collect();
        seed_import_run(&conn, &valid_fingerprint("manifest"), 4, 4, 4);
        let snapshots: Vec<(String, String)> = (1..=4)
            .map(|index| {
                (
                    format!("第{index}章正文"),
                    fingerprints[(index - 1) as usize].clone(),
                )
            })
            .collect();
        let pairs: Vec<(&str, &str)> = snapshots
            .iter()
            .map(|(content, fingerprint)| (content.as_str(), fingerprint.as_str()))
            .collect();
        seed_import_chapters(&conn, &pairs);
        project_core_repository::init(&conn, "项目", "zh-CN").unwrap();

        let batch_id = create_import_run_chapter_batch_checkpoint_id(&[
            ImportRunChapterCheckpointEntry {
                number: 2,
                content_fingerprint: fingerprints[1].clone(),
            },
            ImportRunChapterCheckpointEntry {
                number: 3,
                content_fingerprint: fingerprints[2].clone(),
            },
        ])
        .unwrap();
        let context = capture_import_generation_context(
            &conn,
            &slot(ImportGenerationSlotStage::Blueprints, &batch_id),
        )
        .unwrap();
        assert_eq!(
            context
                .chapters
                .iter()
                .map(|chapter| chapter.number)
                .collect::<Vec<_>>(),
            vec![2, 3],
            "blueprints 阶段按章号区间过滤"
        );
    }
}
