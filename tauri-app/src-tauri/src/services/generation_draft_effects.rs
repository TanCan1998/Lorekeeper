//! generation_draft_effects —— 批量草稿效应读面（批次 G4-5）。
//!
//! 事实源（上游 Electron 基线）：
//! `electron/services/generation-draft-effects.ts`（176 行）——
//! `GenerationDraftEffects` 类的读面方法：
//! - `readBatch(batchId, projectId)`（:67-70）：批量进度快照，
//!   源一致性冲突时抛出冲突码；
//! - `readBatchHistory(batchId, projectId)`（:72-75）：同快照附
//!   `sourceCurrent` 标记（历史查询面，不抛冲突码）；
//! - `readBatchSnapshot`（私有，:77-145）：快照构造核心——
//!   遍历 `generation_attempts` 全表，按 `usage_receipt_json`
//!   里的 `draftCommit.batchId` 归集本批次的已提交章，逐章校验
//!   草稿本体 / outbox / 后处理链；
//! - `readRecovery(runId)`（:59-66）：草稿保存态（absent /
//!   current / changed），`generation:read-context` 的依赖项；
//! - 私有助手 `attempts` / `verifiedCommit` / `readSaveState`
//!   与模块级 `assertGenerationBatchIntent` / `handleOf`。
//!
//! 写面方法（`commit`）与 outbox / postProcess 的写面派生随
//! G4-6/G4-7 迁移，本文件**刻意不平移**（基线 `commit` 是唯一
//! 写面入口，依赖 `assertSources` 闭包与 DraftRepository）。
//!
//! # 刻意偏离（均有上游语义理由）
//! - **构造器捕获**：基线 `new GenerationDraftEffects(db, runs)`
//!   捕获 DB 句柄与仓储；Tauri 侧仓储约定按调用传入
//!   `&Connection`（见 `repositories/generation_run_repository.rs`
//!   头注），故方法按调用接收 `conn` 与 `runs`。
//! - **JSON 解析失败**：基线 `JSON.parse` 抛 `SyntaxError`，
//!   经 controller register catch 收敛为 `GENERATION_REQUEST_FAILED`
//!   （generation-controller.ts:160-162）；本实现按
//!   `map_generation_error` 同一正则透传 / 兜底映射。
//! - **`verifiedCommit` 的不可达断言**：基线 `readBatchSnapshot`
//!   对 `verifiedCommit(row)!` 作非空断言（`draftCommit.batchId`
//!   已命中时 `draftCommit` 必定义）；Rust 侧以 `expect` 表达同一
//!   不可达不变量。

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::repositories::generation_run_repository::{
    DurableGenerationRun, GenerationRunRepository, MainGenerationRunHandle,
};
use crate::services::main_generation_owner::{map_generation_error, mutating_error};

/// 基线 `MAX_BATCH_CHAPTERS`（src/shared/generation-owner-contract.ts）。
const MAX_BATCH_CHAPTERS: i64 = 10;

/// 基线 `DRAFT_VISIBLE_TEXT_VERSION`（src/shared/draft-visible-text.ts）：
/// 仅 `draft-visible-v1` / `draft-visible-v2` 命中。
const DRAFT_VISIBLE_TEXT_VERSION: &str = "draft-visible-v2";

/// 基线 register catch 兜底码（generation-controller.ts:161），
/// 用于本文件内结构性缺失（如 `modelReceipt.modelId` 缺键，
/// 基线会抛 `TypeError` 经同一 catch 收敛）的等价映射。
const GENERATION_REQUEST_FAILED: &str = "GENERATION_REQUEST_FAILED";

/// 基线 `AttemptRow`（`generation_attempts` 行投影）。
struct AttemptRow {
    /// 基线 SELECT 含 `attempt_id`，但读面（`verifiedCommit` /
    /// `readBatchSnapshot`）不消费该列（与基线一致），暂抑制死代码警告。
    #[allow(dead_code)]
    attempt_id: String,
    run_id: String,
    usage_receipt_json: String,
}

/// `generation_attempts` 行 → `AttemptRow`（`attempts` 的
/// 两个查询分支共用，使 `MappedRows` 的行映射类型统一）。
fn attempt_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AttemptRow> {
    Ok(AttemptRow {
        attempt_id: row.get(0)?,
        run_id: row.get(1)?,
        usage_receipt_json: row.get(2)?,
    })
}

/// 基线 `StoredDraftCommit`（attempt usage 载荷里持久化的草稿提交，
/// generation-draft-effects.ts:9-13）。
#[derive(Deserialize, Serialize)]
struct StoredDraftCommit {
    success: bool,
    id: i64,
    version: i64,
    content_hash: String,
    content: String,
    chapter_number: i64,
    handle: MainGenerationRunHandle,
    batch_id: Option<String>,
}

/// 基线 `verifiedCommit` 的草稿行投影
/// （`drafts JOIN contents`，:108）。
struct DraftRow {
    /// 基线 SELECT 含 `d.id`，但读面比对不消费该列（与基线一致），
    /// 暂抑制死代码警告。
    #[allow(dead_code)]
    id: i64,
    version: i64,
    chapter_number: i64,
    status: String,
    body: String,
}

/// 基线 `verifiedCommit` 的返回值（:125）。
struct VerifiedCommit {
    receipt: StoredDraftCommit,
    matches_body: bool,
    draft: Option<DraftRow>,
}

/// 基线 `readBatchSnapshot` 的 outbox 行投影（:118）。
struct OutboxRow {
    finalization_id: String,
    content_hash: String,
    content_snapshot: String,
    chapter_number: i64,
    publication_status: String,
}

/// 基线 `readBatchSnapshot` 的后处理行投影（:127-129）。
struct PostProcessRow {
    id: String,
    all_critical_passed: i64,
}

/// 基线 `readBatchSnapshot` 的后处理步骤行投影（:128）。
struct PostProcessStepRow {
    step_key: String,
    critical: i64,
    ok: i64,
}

/// 基线 `textHash`（repositories/generation-run-repository.ts）——
/// SHA-256 十六进制小写（与 `generation_run_repository.rs` 的
/// 私有 `text_hash` 同算法同输出）。
fn text_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 基线 `isDraftVisibleTextVersion`（src/shared/draft-visible-text.ts）。
fn is_draft_visible_text_version(value: &str) -> bool {
    value == "draft-visible-v1" || value == DRAFT_VISIBLE_TEXT_VERSION
}

/// 基线 `handleOf(run)`（main-generation-owner.ts:125-126）。
fn handle_of(run: &DurableGenerationRun) -> Value {
    json!({
        "projectId": run.binding.project_id,
        "epoch": run.binding.epoch,
        "rootActionId": run.root_action_id,
        "runId": run.run_id,
    })
}

/// 基线模块级 `assertGenerationBatchIntent`（generation-draft-effects.ts:18-27）。
pub fn assert_generation_batch_intent(intent: &Value) -> Result<(), String> {
    let mode = intent.get("mode").and_then(Value::as_str);
    let range = intent.get("range");
    let start = range
        .and_then(|item| item.get("startChapter"))
        .and_then(Value::as_i64);
    let end = range
        .and_then(|item| item.get("endChapter"))
        .and_then(Value::as_i64);
    let target_units = intent.get("targetUnits").and_then(Value::as_i64);
    if !matches!(mode, Some("draft_review") | Some("auto_finalize"))
        || range.is_none()
        || start.is_none()
        || end.is_none()
        || start.unwrap_or_default() < 1
        || end.unwrap_or(i64::MAX) < start.unwrap_or_default()
        || end.unwrap_or(i64::MAX) - start.unwrap_or_default() >= MAX_BATCH_CHAPTERS
        || target_units.is_none()
        || target_units.unwrap_or_default() < 1
        || target_units.unwrap_or_default() > 1_000_000
    {
        return Err(mutating_error("GENERATION_BATCH_INTENT_INVALID"));
    }
    Ok(())
}

/// 基线 `GenerationDraftEffects` 的读面（`commit` 写面随 G4-6/G4-7
/// 迁移，不在本结构体内）。
pub struct GenerationDraftEffects;

impl GenerationDraftEffects {
    /// 基线私有 `attempts(runId?)`（:21-24）：`runId` 缺省时扫描
    /// 全表（`readBatchSnapshot` 的批次归集依赖全表扫描）。
    fn attempts(conn: &Connection, run_id: Option<&str>) -> Result<Vec<AttemptRow>, String> {
        let mut statement = conn
            .prepare(match run_id {
                Some(_) => "SELECT attempt_id,run_id,usage_receipt_json FROM generation_attempts WHERE run_id=? ORDER BY rowid",
                None => "SELECT attempt_id,run_id,usage_receipt_json FROM generation_attempts ORDER BY rowid",
            })
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
        let rows = match run_id {
            Some(run_id) => statement
                .query_map([run_id], attempt_row)
                .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?,
            None => statement
                .query_map([], attempt_row)
                .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?,
        };
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))
    }

    /// 基线私有 `verifiedCommit(row)`（:27-57）：校验 attempt usage
    /// 载荷里的草稿提交回执——可见合成版本 / 章节号 / 版本 / 内容哈希 /
    /// 批次身份 / 句柄（含 `artifactIdentity.epoch` 覆盖）任一不符即
    /// `GENERATION_DRAFT_RECEIPT_INVALID`；`draftCommit` 缺省 →
    /// `None`（非草稿 attempt）。
    fn verified_commit(
        &self,
        conn: &Connection,
        runs: &GenerationRunRepository,
        row: &AttemptRow,
    ) -> Result<Option<VerifiedCommit>, String> {
        let usage: Value = serde_json::from_str(&row.usage_receipt_json)
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
        let Some(stored_value) = usage.get("draftCommit") else {
            return Ok(None);
        };
        // 基线把 `draftCommit` 断言为 `StoredDraftCommit` 后逐字段
        // 比对——任一字段缺失 / 类型不符在基线同样落入
        // `GENERATION_DRAFT_RECEIPT_INVALID` 的字段检查分支。
        let stored: StoredDraftCommit = serde_json::from_value(stored_value.clone())
            .map_err(|_| mutating_error("GENERATION_DRAFT_RECEIPT_INVALID"))?;
        let run = runs
            .get(conn, &row.run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        let composition = runs
            .read_visible_composition(conn, &run.run_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        let draft = conn
            .query_row(
                "SELECT d.id,d.version,d.chapter_number,d.status,c.body FROM drafts d JOIN contents c ON c.id=d.content_id WHERE d.id=?",
                [stored.id],
                |row| {
                    Ok(DraftRow {
                        id: row.get(0)?,
                        version: row.get(1)?,
                        chapter_number: row.get(2)?,
                        status: row.get(3)?,
                        body: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
        // 基线 `artifactIdentity?.epoch`：缺省时 JS 对象以 `undefined`
        // 参与 `isDeepStrictEqual`，与任何字符串句柄均不等价；
        // 此处以 JSON `null` 表达同一判别（句柄 `epoch` 恒为字符串）。
        let expected_epoch = usage
            .get("artifactIdentity")
            .and_then(|identity| identity.get("epoch"))
            .cloned()
            .unwrap_or(Value::Null);
        let expected_handle = json!({
            "projectId": run.binding.project_id,
            "epoch": expected_epoch,
            "rootActionId": run.root_action_id,
            "runId": run.run_id,
        });
        let stored_handle = json!({
            "projectId": stored.handle.project_id,
            "epoch": stored.handle.epoch,
            "rootActionId": stored.handle.root_action_id,
            "runId": stored.handle.run_id,
        });
        let manifest_chapter_number = run
            .binding
            .source_manifest
            .get("chapterNumber")
            .and_then(Value::as_i64);
        let manifest_batch_id = run
            .binding
            .source_manifest
            .get("batchId")
            .and_then(Value::as_str);
        if composition.is_none()
            || !is_draft_visible_text_version(&composition.as_ref().unwrap().algorithm)
            || !stored.success
            || Some(stored.chapter_number) != manifest_chapter_number
            || draft.is_some()
                && (Some(stored.chapter_number) != Some(draft.as_ref().unwrap().chapter_number)
                    || Some(stored.version) != Some(draft.as_ref().unwrap().version))
            || stored.content_hash != text_hash(&stored.content)
            || composition.as_ref().map(|item| item.text_hash.as_str())
                != Some(stored.content_hash.as_str())
            || composition.as_ref().map(|item| item.text.as_str()) != Some(stored.content.as_str())
            || stored.batch_id.as_deref() != manifest_batch_id
            || stored_handle != expected_handle
        {
            return Err(mutating_error("GENERATION_DRAFT_RECEIPT_INVALID"));
        }
        Ok(Some(VerifiedCommit {
            receipt: stored,
            matches_body: draft.is_some()
                && composition.as_ref().unwrap().text == draft.as_ref().unwrap().body,
            draft,
        }))
    }

    /// 基线私有 `readSaveState(runId)`（:59-64）：单运行至多一个
    /// 有效草稿提交（多提交 → `GENERATION_DRAFT_RECEIPT_INVALID`）。
    fn read_save_state(
        &self,
        conn: &Connection,
        runs: &GenerationRunRepository,
        run_id: &str,
    ) -> Result<Option<VerifiedCommit>, String> {
        let mut commits = Vec::new();
        for row in Self::attempts(conn, Some(run_id))? {
            if let Some(commit) = self.verified_commit(conn, runs, &row)? {
                commits.push(commit);
            }
        }
        if commits.len() > 1 {
            return Err(mutating_error("GENERATION_DRAFT_RECEIPT_INVALID"));
        }
        Ok(commits.into_iter().next())
    }

    /// 基线 `readRecovery(runId)`（:59-66 的 readRecovery，:59-66
    /// 区段）——草稿保存态：`absent` / `changed`（正文已被其他
    /// 途径改写）/ `current`（回执 + 正文）。
    pub fn read_recovery(
        &self,
        conn: &Connection,
        runs: &GenerationRunRepository,
        run_id: &str,
    ) -> Result<Value, String> {
        let Some(saved) = self.read_save_state(conn, runs, run_id)? else {
            return Ok(json!({ "kind": "absent" }));
        };
        if !saved.matches_body {
            return Ok(json!({ "kind": "changed" }));
        }
        let receipt = saved.receipt;
        Ok(json!({
            "kind": "current",
            "receipt": {
                "success": true,
                "id": receipt.id,
                "version": receipt.version,
                "content": receipt.content,
                "contentHash": receipt.content_hash,
            },
        }))
    }

    /// 基线 `readCommit(runId)`（:47-52）：草稿提交回执
    /// （正文已被改写时抛 `GENERATION_DRAFT_RECEIPT_INVALID`）。
    /// 短大纲重试准入（`assertShortOutlineRetryAvailable`）
    /// 的依赖项。
    pub fn read_commit(
        &self,
        conn: &Connection,
        runs: &GenerationRunRepository,
        run_id: &str,
    ) -> Result<Option<Value>, String> {
        let saved = self.read_save_state(conn, runs, run_id)?;
        if let Some(saved) = saved {
            if !saved.matches_body {
                return Err(mutating_error("GENERATION_DRAFT_RECEIPT_INVALID"));
            }
            return Ok(Some(serde_json::to_value(saved.receipt).map_err(
                |error| mutating_error(map_generation_error(&error.to_string())),
            )?));
        }
        Ok(None)
    }
    /// 基线 `readBatch(batchId, projectId)`（:67-70）：批量进度快照；
    /// 源一致性冲突（草稿回执失效 / 草稿被替换 / 定稿冲突）时按
    /// 基线抛出对应冲突码。
    pub fn read_batch(
        &self,
        conn: &Connection,
        runs: &GenerationRunRepository,
        batch_id: &str,
        project_id: &str,
    ) -> Result<Value, String> {
        let (progress, source_conflict) =
            self.read_batch_snapshot(conn, runs, batch_id, project_id)?;
        if let Some(code) = source_conflict {
            return Err(mutating_error(code));
        }
        Ok(progress)
    }

    /// 基线 `readBatchHistory(batchId, projectId)`（:72-75）：历史
    /// 查询面——同快照附 `sourceCurrent` 标记，不因源冲突抛错。
    pub fn read_batch_history(
        &self,
        conn: &Connection,
        runs: &GenerationRunRepository,
        batch_id: &str,
        project_id: &str,
    ) -> Result<Value, String> {
        let (progress, source_conflict) =
            self.read_batch_snapshot(conn, runs, batch_id, project_id)?;
        let mut object = progress.as_object().cloned().unwrap_or_default();
        object.insert(
            "sourceCurrent".to_string(),
            Value::Bool(source_conflict.is_none()),
        );
        Ok(Value::Object(object))
    }

    /// 基线私有 `readBatchSnapshot(batchId, projectId)`（:77-145）：
    /// 全表扫描 `generation_attempts`，按 usage 载荷的
    /// `draftCommit.batchId` 归集本批次已提交章，构造
    /// `GenerationBatchProgress` 与源冲突标记。返回
    /// `(progress, sourceConflict)`——冲突码由调用方决定抛出
    /// （`readBatch`）或降级为标记（`readBatchHistory`）。
    fn read_batch_snapshot(
        &self,
        conn: &Connection,
        runs: &GenerationRunRepository,
        batch_id: &str,
        project_id: &str,
    ) -> Result<(Value, Option<&'static str>), String> {
        // 基线以 `batchId` 即根运行 ID 取根运行（`runs.get(batchId)`）。
        let root = runs
            .get(conn, batch_id)
            .map_err(|error| mutating_error(map_generation_error(&error)))?;
        let intent = root
            .binding
            .source_manifest
            .get("batchIntent")
            .cloned()
            .unwrap_or(Value::Null);
        assert_generation_batch_intent(&intent)?;
        if root.binding.project_id != project_id
            || root
                .binding
                .source_manifest
                .get("operation")
                .and_then(Value::as_str)
                != Some("batch-chapters")
        {
            return Err(mutating_error("GENERATION_BATCH_IDENTITY_INVALID"));
        }
        let mut completed_chapters: Vec<Value> = Vec::new();
        let mut source_conflict: Option<&'static str> = None;
        for row in Self::attempts(conn, None)? {
            let usage: Value = serde_json::from_str(&row.usage_receipt_json)
                .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
            if usage
                .get("draftCommit")
                .and_then(|commit| commit.get("batchId"))
                .and_then(Value::as_str)
                != Some(batch_id)
            {
                continue;
            }
            // 基线 `this.verifiedCommit(row)!`：`draftCommit.batchId`
            // 已命中 ⇒ `draftCommit` 必定义 ⇒ 非空断言不可达。
            let saved = self
                .verified_commit(conn, runs, &row)?
                .expect("draftCommit is defined when its batchId matched");
            if !saved.matches_body && source_conflict.is_none() {
                source_conflict = Some("GENERATION_DRAFT_RECEIPT_INVALID");
            }
            let committed = saved.receipt;
            if committed.handle.root_action_id != root.root_action_id {
                return Err(mutating_error("GENERATION_BATCH_LINEAGE_INVALID"));
            }
            let latest: Option<i64> = conn
                .query_row(
                    "SELECT id FROM drafts WHERE chapter_number=? ORDER BY version DESC,id DESC LIMIT 1",
                    [committed.chapter_number],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
            if latest != Some(committed.id) && source_conflict.is_none() {
                source_conflict = Some("GENERATION_BATCH_DRAFT_REPLACED");
            }
            let mut outbox: Option<OutboxRow> = conn
                .query_row(
                    "SELECT finalization_id,content_hash,content_snapshot,chapter_number,publication_status FROM finalization_outbox WHERE draft_id=?",
                    [committed.id],
                    |row| {
                        Ok(OutboxRow {
                            finalization_id: row.get(0)?,
                            content_hash: row.get(1)?,
                            content_snapshot: row.get(2)?,
                            chapter_number: row.get(3)?,
                            publication_status: row.get(4)?,
                        })
                    },
                )
                .optional()
                .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
            if let Some(outbox_row) = &outbox {
                if outbox_row.content_hash != text_hash(&outbox_row.content_snapshot)
                    || Some(outbox_row.chapter_number) != Some(committed.chapter_number)
                {
                    return Err(mutating_error("GENERATION_BATCH_FINALIZATION_CONFLICT"));
                }
                if outbox_row.content_snapshot != committed.content {
                    // 基线 `saved.draft?.status !== 'finalized' ||
                    // saved.draft.body !== outbox.content_snapshot`：
                    // 草稿行缺失（`?.` 短路为 undefined）同样抛错。
                    let mismatch = saved.draft.as_ref().map_or(true, |draft| {
                        draft.status != "finalized" || draft.body != outbox_row.content_snapshot
                    });
                    if mismatch {
                        return Err(mutating_error("GENERATION_BATCH_FINALIZATION_CONFLICT"));
                    }
                    if source_conflict.is_none() {
                        source_conflict = Some("GENERATION_BATCH_FINALIZATION_CONFLICT");
                    }
                    outbox = None;
                }
            }
            let post_process: Option<PostProcessRow> = match &outbox {
                Some(outbox) => conn
                    .query_row(
                        "SELECT id,all_critical_passed FROM post_process_runs WHERE trigger_source_type='chapter_finalize' AND trigger_source_id=? ORDER BY created_at DESC,rowid DESC LIMIT 1",
                        [format!("finalization:{}", outbox.finalization_id)],
                        |row| {
                            Ok(PostProcessRow {
                                id: row.get(0)?,
                                all_critical_passed: row.get(1)?,
                            })
                        },
                    )
                    .optional()
                    .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?,
                None => None,
            };
            let steps: Vec<PostProcessStepRow> = match &post_process {
                Some(post_process) => {
                    let mut statement = conn
                        .prepare(
                            "SELECT step_key,critical,ok FROM post_process_steps WHERE run_id=?",
                        )
                        .map_err(|error| {
                            mutating_error(map_generation_error(&error.to_string()))
                        })?;
                    let steps = statement
                        .query_map([post_process.id.clone()], |row| {
                            Ok(PostProcessStepRow {
                                step_key: row.get(0)?,
                                critical: row.get(1)?,
                                ok: row.get(2)?,
                            })
                        })
                        .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|error| {
                            mutating_error(map_generation_error(&error.to_string()))
                        })?;
                    steps
                }
                None => Vec::new(),
            };
            // 基线 `postProcessComplete`：全部关键步骤通过且
            // `kb_import` / `chapter_notes` 两个必需步骤均关键且通过。
            let post_process_complete = post_process
                .as_ref()
                .is_some_and(|item| item.all_critical_passed == 1)
                && ["kb_import", "chapter_notes"].iter().all(|key| {
                    steps
                        .iter()
                        .any(|step| step.step_key == *key && step.critical == 1 && step.ok == 1)
                })
                && steps
                    .iter()
                    .filter(|step| step.critical == 1)
                    .all(|step| step.ok == 1);
            let source_run = runs
                .get(conn, &row.run_id)
                .map_err(|error| mutating_error(map_generation_error(&error)))?;
            let mut chapter = Map::new();
            chapter.insert("chapterNumber".to_string(), json!(committed.chapter_number));
            chapter.insert("draftId".to_string(), json!(committed.id));
            chapter.insert("version".to_string(), json!(committed.version));
            chapter.insert("contentHash".to_string(), json!(committed.content_hash));
            chapter.insert("sourceRunHandle".to_string(), handle_of(&source_run));
            chapter.insert(
                "postProcessComplete".to_string(),
                json!(post_process_complete),
            );
            match &outbox {
                Some(outbox) if outbox.publication_status == "published" => {
                    chapter.insert(
                        "finalizationId".to_string(),
                        json!(outbox.finalization_id.clone()),
                    );
                }
                Some(outbox) => {
                    chapter.insert(
                        "pendingFinalizationId".to_string(),
                        json!(outbox.finalization_id.clone()),
                    );
                }
                None => {}
            }
            completed_chapters.push(Value::Object(chapter));
        }
        completed_chapters.sort_by(|left, right| {
            let left_number = left
                .get("chapterNumber")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            let right_number = right
                .get("chapterNumber")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            left_number.cmp(&right_number)
        });
        let intent_start = intent
            .get("range")
            .and_then(|item| item.get("startChapter"))
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let intent_end = intent
            .get("range")
            .and_then(|item| item.get("endChapter"))
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let intent_mode = intent
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut next_chapter_number: Option<i64> = Some(intent_start);
        for (index, chapter) in completed_chapters.iter().enumerate() {
            let chapter_number = chapter
                .get("chapterNumber")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            let has_finalization_id = chapter.get("finalizationId").is_some();
            let post_process_complete = chapter
                .get("postProcessComplete")
                .and_then(Value::as_bool)
                .unwrap_or_default();
            if chapter_number != intent_start + index as i64
                || chapter_number > intent_end
                || next_chapter_number != Some(chapter_number)
            {
                return Err(mutating_error("GENERATION_BATCH_PROGRESS_INVALID"));
            }
            if intent_mode == "auto_finalize" && (!has_finalization_id || !post_process_complete) {
                break;
            }
            next_chapter_number = if chapter_number == intent_end {
                None
            } else {
                Some(chapter_number + 1)
            };
        }
        let pending_root_run_ids: Vec<String> = {
            let mut statement = conn
                .prepare("SELECT run_id FROM generation_runs WHERE root_action_id=?")
                .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
            let run_ids = statement
                .query_map([root.root_action_id.clone()], |row| row.get(0))
                .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| mutating_error(map_generation_error(&error.to_string())))?;
            run_ids
        };
        let mut pending: Vec<DurableGenerationRun> = Vec::new();
        for run_id in pending_root_run_ids {
            let run = runs
                .get(conn, &run_id)
                .map_err(|error| mutating_error(map_generation_error(&error)))?;
            let is_batch_chapter = run
                .binding
                .source_manifest
                .get("batchId")
                .and_then(Value::as_str)
                == Some(batch_id);
            let is_next_chapter = run
                .binding
                .source_manifest
                .get("chapterNumber")
                .and_then(Value::as_i64)
                == next_chapter_number;
            if is_batch_chapter
                && is_next_chapter
                && self.read_save_state(conn, runs, &run.run_id)?.is_none()
            {
                pending.push(run);
            }
        }
        if pending.len() > 1 {
            return Err(mutating_error("GENERATION_BATCH_PROGRESS_INVALID"));
        }
        // 基线 `{ ...structuredClone(intent), modelId, batchId,
        // rootHandle, completedChapters, nextChapterNumber,
        // currentChapterRunHandle?, authorInputs }`——以 manifest
        // 的 `batchIntent` 对象为基底逐键注入（等价基线展开）。
        let mut progress = intent.as_object().cloned().unwrap_or_default();
        let model_id = root
            .binding
            .source_manifest
            .get("modelReceipt")
            .and_then(|receipt| receipt.get("modelId"))
            .and_then(Value::as_str)
            .ok_or_else(|| mutating_error(GENERATION_REQUEST_FAILED))?;
        progress.insert("modelId".to_string(), json!(model_id));
        progress.insert("batchId".to_string(), json!(batch_id));
        progress.insert("rootHandle".to_string(), handle_of(&root));
        progress.insert(
            "completedChapters".to_string(),
            Value::Array(completed_chapters),
        );
        progress.insert("nextChapterNumber".to_string(), json!(next_chapter_number));
        if let Some(pending_run) = pending.first() {
            progress.insert(
                "currentChapterRunHandle".to_string(),
                handle_of(pending_run),
            );
        }
        let author_inputs = root
            .binding
            .source_manifest
            .get("authorInputs")
            .cloned()
            .filter(|value| !value.is_null())
            .unwrap_or_else(|| json!([]));
        progress.insert("authorInputs".to_string(), author_inputs);
        Ok((Value::Object(progress), source_conflict))
    }
}
