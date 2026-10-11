//! 旧名册生成上下文服务（服务层移植自
//! `electron/services/legacy-roster-generation-context.ts`）。
//!
//! 负责：捕获「旧名册修复」生成上下文（绑定项目 / 源纪元 / 上下文键 /
//! 创建时间 / 来源快照）、构造生成任务、以及从持久化运行
//! （`DurableGenerationRun.binding.sourceManifest`）中读回并校验上下文。
//!
//! 语义对齐铁律：业务错误码与上游逐字一致
//! （`GENERATION_LEGACY_CONTEXT_INVALID` / `GENERATION_LEGACY_TASK_INVALID`）；
//! 模块间互调同波新文件用 `crate::services::xxx`（`mod.rs` 由编排者统一接）。

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::commands::project::{epoch_millis_now, iso8601_utc_from_millis};
use crate::repositories::character_roster_repository::{
    hash_text, CharacterRosterMigrationState, CharacterRosterStatus,
};
use crate::repositories::generation_run_repository::DurableGenerationRun;
use crate::services::legacy_roster_generation::LegacyRosterGenerationContext;
use crate::services::legacy_roster_generation_pure::{
    build_legacy_roster_task, GenerationTask, LegacyRosterTaskInput,
};
use crate::services::legacy_roster_source::capture_legacy_roster_context;

/// 捕获范围（对齐上游 `scope: { projectId: string; epoch: string }`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LegacyRosterGenerationScope {
    pub project_id: String,
    pub epoch: String,
}

/// 捕获旧名册生成上下文（对齐上游 `captureLegacyRosterGenerationContext`）。
///
/// `previous` 给出时沿用其 `originEpoch` / `createdAt`（上游 `??` 兜底），
/// 否则取当前 scope 纪元与当前 UTC 时间戳。
pub fn capture_legacy_roster_generation_context(
    conn: &Connection,
    scope: &LegacyRosterGenerationScope,
    key: &str,
    previous: Option<&LegacyRosterGenerationContext>,
) -> Result<LegacyRosterGenerationContext, String> {
    if key.is_empty()
        || scope.project_id.is_empty()
        || scope.epoch.is_empty()
        || previous.is_some_and(|previous| {
            previous.project_id != scope.project_id
                || previous.key != key
                || previous.origin_epoch.is_empty()
                || previous.created_at.is_empty()
        })
    {
        return Err("GENERATION_LEGACY_CONTEXT_INVALID".to_string());
    }
    Ok(LegacyRosterGenerationContext {
        project_id: scope.project_id.clone(),
        origin_epoch: previous
            .map(|previous| previous.origin_epoch.clone())
            .unwrap_or_else(|| scope.epoch.clone()),
        key: key.to_string(),
        created_at: previous
            .map(|previous| previous.created_at.clone())
            .unwrap_or_else(now_iso8601),
        source: capture_legacy_roster_context(conn)?,
    })
}

/// 构造旧名册生成任务（对齐上游 `legacyRosterGenerationTask`）。
pub fn legacy_roster_generation_task(context: &LegacyRosterGenerationContext) -> GenerationTask {
    build_legacy_roster_task(&LegacyRosterTaskInput {
        legacy_markdown: context.source.raw_legacy.clone(),
        genre: context.source.genre.clone(),
    })
}

/// 从持久化运行读回并校验旧名册生成上下文
/// （对齐上游 `readLegacyRosterGenerationContext`）。
///
/// 校验 manifest 中的 `operation` / `legacyRosterKey` /
/// `legacyRosterOriginEpoch` / `legacyRosterContextHash` /
/// `legacyRosterTask` / `legacyRosterTaskHash` 与上下文自身一致性。
pub fn read_legacy_roster_generation_context(
    run: &DurableGenerationRun,
) -> Result<LegacyRosterGenerationContext, String> {
    let manifest = &run.binding.source_manifest;
    let context: LegacyRosterGenerationContext = serde_json::from_value(
        manifest
            .get("legacyRosterContext")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .map_err(|_| "GENERATION_LEGACY_CONTEXT_INVALID".to_string())?;
    if context.project_id != run.binding.project_id
        || context.key.is_empty()
        || context.created_at.is_empty()
        || context.origin_epoch.is_empty()
        || manifest
            .get("operation")
            .and_then(serde_json::Value::as_str)
            != Some("legacy-character-roster-repair")
        || manifest
            .get("legacyRosterKey")
            .and_then(serde_json::Value::as_str)
            != Some(context.key.as_str())
        || manifest
            .get("legacyRosterOriginEpoch")
            .and_then(serde_json::Value::as_str)
            != Some(context.origin_epoch.as_str())
        || manifest
            .get("legacyRosterContextHash")
            .and_then(serde_json::Value::as_str)
            != Some(hash_text(&canonical_json(&context)).as_str())
        || context.source.legacy_hash != hash_text(&context.source.raw_legacy)
        || context.source.snapshot.migration_state
            != CharacterRosterMigrationState::LegacyMarkdownPending
        || context.source.snapshot.status != CharacterRosterStatus::LegacyRepairRequired
        || !context.source.active_ids.is_empty()
        || !context.source.snapshot.entries.is_empty()
        || context.source.snapshot.legacy_markdown.as_deref()
            != Some(context.source.raw_legacy.as_str())
    {
        return Err("GENERATION_LEGACY_CONTEXT_INVALID".to_string());
    }
    let task = legacy_roster_generation_task(&context);
    if !values_equal(
        &task,
        &manifest
            .get("legacyRosterTask")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    ) || manifest
        .get("legacyRosterTaskHash")
        .and_then(serde_json::Value::as_str)
        != Some(hash_text(&canonical_json(&task)).as_str())
    {
        return Err("GENERATION_LEGACY_TASK_INVALID".to_string());
    }
    Ok(context)
}

/// `new Date().toISOString()`（`YYYY-MM-DDTHH:MM:SS.mmmZ`）。
fn now_iso8601() -> String {
    iso8601_utc_from_millis(epoch_millis_now())
}

/// 确定性 JSON 序列化（结构字段按声明顺序，与上游 `JSON.stringify`
/// 插入顺序一致）。
fn canonical_json<T: Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string(value).expect("LegacyRoster 上下文序列化失败")
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
    use crate::repositories::character_roster_repository as character_roster;
    use crate::repositories::generation_run_repository::{FrozenInputFingerprint, RunBinding};
    use crate::repositories::project_core_repository as project_core;

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

    fn test_scope() -> LegacyRosterGenerationScope {
        LegacyRosterGenerationScope {
            project_id: "proj-1".to_string(),
            epoch: "epoch-1".to_string(),
        }
    }

    /// 构造带 manifest 的持久化运行（结构字面量，字段对齐
    /// `generation_run_repository` 的 `DurableGenerationRun`）。
    fn test_run(manifest: serde_json::Map<String, serde_json::Value>) -> DurableGenerationRun {
        DurableGenerationRun {
            run_id: "run-1".to_string(),
            root_action_id: "action-1".to_string(),
            status: "running".to_string(),
            binding: RunBinding {
                project_id: "proj-1".to_string(),
                epoch: "epoch-1".to_string(),
                fingerprint: FrozenInputFingerprint {
                    chapter_brief_hash: "a".to_string(),
                    author_guidance_hash: "a".to_string(),
                    dependency_hash: "a".to_string(),
                    context_snapshot_hash: "a".to_string(),
                    template_hash: "a".to_string(),
                    skill_snapshot_hash: "a".to_string(),
                    model_lease_revision: "a".to_string(),
                    policy_hash: "a".to_string(),
                    output_contract_hash: "a".to_string(),
                },
                context_snapshot_id: "snap-1".to_string(),
                source_manifest: manifest,
                source_refs: Vec::new(),
                extra: serde_json::Map::new(),
            },
        }
    }

    #[test]
    fn capture_legacy_roster_generation_context_rejects_invalid_scope() {
        let conn = identity_schema_db();
        set_pending_state(&conn);
        let scope = test_scope();
        // 空 key
        assert_eq!(
            capture_legacy_roster_generation_context(&conn, &scope, "", None)
                .expect_err("空 key 应拒绝"),
            "GENERATION_LEGACY_CONTEXT_INVALID"
        );
        // 空 projectId
        let mut empty_project = scope.clone();
        empty_project.project_id = String::new();
        assert_eq!(
            capture_legacy_roster_generation_context(&conn, &empty_project, "k", None)
                .expect_err("空 projectId 应拒绝"),
            "GENERATION_LEGACY_CONTEXT_INVALID"
        );
        // 空 epoch
        let mut empty_epoch = scope.clone();
        empty_epoch.epoch = String::new();
        assert_eq!(
            capture_legacy_roster_generation_context(&conn, &empty_epoch, "k", None)
                .expect_err("空 epoch 应拒绝"),
            "GENERATION_LEGACY_CONTEXT_INVALID"
        );
        // previous 与 scope 项目不一致
        let mismatched = LegacyRosterGenerationContext {
            project_id: "other-project".to_string(),
            origin_epoch: "epoch-1".to_string(),
            key: "k".to_string(),
            created_at: "2026-01-01T00:00:00.000Z".to_string(),
            source: capture_legacy_roster_context(&conn).unwrap_or_else(|_| {
                unreachable!("pending 态未设置时 capture 必然失败，测试不应到达此处")
            }),
        };
        assert_eq!(
            capture_legacy_roster_generation_context(&conn, &scope, "k", Some(&mismatched))
                .expect_err("previous 项目不一致应拒绝"),
            "GENERATION_LEGACY_CONTEXT_INVALID"
        );
        // previous 键不一致
        let mut wrong_key = mismatched.clone();
        wrong_key.project_id = "proj-1".to_string();
        wrong_key.key = "other-key".to_string();
        assert_eq!(
            capture_legacy_roster_generation_context(&conn, &scope, "k", Some(&wrong_key))
                .expect_err("previous key 不一致应拒绝"),
            "GENERATION_LEGACY_CONTEXT_INVALID"
        );
        // previous originEpoch 为空
        let mut empty_origin = wrong_key;
        empty_origin.key = "k".to_string();
        empty_origin.origin_epoch = String::new();
        assert_eq!(
            capture_legacy_roster_generation_context(&conn, &scope, "k", Some(&empty_origin))
                .expect_err("previous originEpoch 为空应拒绝"),
            "GENERATION_LEGACY_CONTEXT_INVALID"
        );
        // previous createdAt 为空
        let mut empty_created = empty_origin;
        empty_created.origin_epoch = "epoch-1".to_string();
        empty_created.created_at = String::new();
        assert_eq!(
            capture_legacy_roster_generation_context(&conn, &scope, "k", Some(&empty_created))
                .expect_err("previous createdAt 为空应拒绝"),
            "GENERATION_LEGACY_CONTEXT_INVALID"
        );
    }

    #[test]
    fn capture_legacy_roster_generation_context_requires_identity_schema() {
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
        // 校验通过后进入 captureLegacyRosterContext，无身份 schema 时报其错误码。
        let error =
            capture_legacy_roster_generation_context(&conn, &test_scope(), "roster-1", None)
                .expect_err("无身份 schema 应拒绝");
        assert_eq!(error, "LEGACY_ROSTER_ID_SCHEMA_REQUIRED");
    }

    #[test]
    fn capture_legacy_roster_generation_context_captures_pending_source() {
        let conn = identity_schema_db();
        set_pending_state(&conn);
        let context =
            capture_legacy_roster_generation_context(&conn, &test_scope(), "roster-1", None)
                .expect("上下文捕获失败");
        assert_eq!(context.project_id, "proj-1");
        assert_eq!(context.origin_epoch, "epoch-1");
        assert_eq!(context.key, "roster-1");
        assert!(!context.created_at.is_empty());
        assert_eq!(context.source.raw_legacy, "旧图谱原文");

        // 二次捕获沿用 previous 的 originEpoch / createdAt。
        let renewed = capture_legacy_roster_generation_context(
            &conn,
            &test_scope(),
            "roster-1",
            Some(&context),
        )
        .expect("二次捕获失败");
        assert_eq!(renewed.origin_epoch, context.origin_epoch);
        assert_eq!(renewed.created_at, context.created_at);
    }

    #[test]
    fn legacy_roster_generation_task_builds_contract_task() {
        let conn = identity_schema_db();
        set_pending_state(&conn);
        let context =
            capture_legacy_roster_generation_context(&conn, &test_scope(), "roster-1", None)
                .expect("上下文捕获失败");
        let task = legacy_roster_generation_task(&context);
        assert_eq!(task.purpose, "legacy-character-roster-repair");
        assert_eq!(task.reasoning_stage.as_deref(), Some("planning"));
        assert_eq!(task.output, "structured-data");
        assert_eq!(task.messages.len(), 2);
        assert_eq!(task.messages[0].role, "system");
        assert_eq!(task.messages[1].role, "user");
        assert!(task.messages[1].content.contains("旧图谱原文"));
    }

    #[test]
    fn read_legacy_roster_generation_context_round_trips() {
        let conn = identity_schema_db();
        set_pending_state(&conn);
        let context =
            capture_legacy_roster_generation_context(&conn, &test_scope(), "roster-1", None)
                .expect("上下文捕获失败");
        let task = legacy_roster_generation_task(&context);
        let mut manifest = serde_json::Map::new();
        manifest.insert(
            "operation".to_string(),
            serde_json::json!("legacy-character-roster-repair"),
        );
        manifest.insert(
            "legacyRosterKey".to_string(),
            serde_json::json!(context.key),
        );
        manifest.insert(
            "legacyRosterOriginEpoch".to_string(),
            serde_json::json!(context.origin_epoch),
        );
        manifest.insert(
            "legacyRosterContext".to_string(),
            serde_json::to_value(&context).expect("上下文序列化失败"),
        );
        manifest.insert(
            "legacyRosterContextHash".to_string(),
            serde_json::json!(hash_text(&canonical_json(&context))),
        );
        manifest.insert(
            "legacyRosterTask".to_string(),
            serde_json::to_value(&task).expect("任务序列化失败"),
        );
        manifest.insert(
            "legacyRosterTaskHash".to_string(),
            serde_json::json!(hash_text(&canonical_json(&task))),
        );
        let run = test_run(manifest);
        let restored = read_legacy_roster_generation_context(&run).expect("上下文读回失败");
        assert_eq!(restored, context);
    }

    #[test]
    fn read_legacy_roster_generation_context_rejects_tampered_manifest() {
        let conn = identity_schema_db();
        set_pending_state(&conn);
        let context =
            capture_legacy_roster_generation_context(&conn, &test_scope(), "roster-1", None)
                .expect("上下文捕获失败");
        let task = legacy_roster_generation_task(&context);

        // contextHash 被篡改
        let mut manifest = serde_json::Map::new();
        manifest.insert(
            "operation".to_string(),
            serde_json::json!("legacy-character-roster-repair"),
        );
        manifest.insert(
            "legacyRosterKey".to_string(),
            serde_json::json!(context.key),
        );
        manifest.insert(
            "legacyRosterOriginEpoch".to_string(),
            serde_json::json!(context.origin_epoch),
        );
        manifest.insert(
            "legacyRosterContext".to_string(),
            serde_json::to_value(&context).expect("上下文序列化失败"),
        );
        manifest.insert(
            "legacyRosterContextHash".to_string(),
            serde_json::json!(hash_text("tampered")),
        );
        manifest.insert(
            "legacyRosterTask".to_string(),
            serde_json::to_value(&task).expect("任务序列化失败"),
        );
        manifest.insert(
            "legacyRosterTaskHash".to_string(),
            serde_json::json!(hash_text(&canonical_json(&task))),
        );
        let error = read_legacy_roster_generation_context(&test_run(manifest))
            .expect_err("contextHash 篡改应拒绝");
        assert_eq!(error, "GENERATION_LEGACY_CONTEXT_INVALID");

        // taskHash 被篡改
        let mut manifest = serde_json::Map::new();
        manifest.insert(
            "operation".to_string(),
            serde_json::json!("legacy-character-roster-repair"),
        );
        manifest.insert(
            "legacyRosterKey".to_string(),
            serde_json::json!(context.key),
        );
        manifest.insert(
            "legacyRosterOriginEpoch".to_string(),
            serde_json::json!(context.origin_epoch),
        );
        manifest.insert(
            "legacyRosterContext".to_string(),
            serde_json::to_value(&context).expect("上下文序列化失败"),
        );
        manifest.insert(
            "legacyRosterContextHash".to_string(),
            serde_json::json!(hash_text(&canonical_json(&context))),
        );
        manifest.insert(
            "legacyRosterTask".to_string(),
            serde_json::to_value(&task).expect("任务序列化失败"),
        );
        manifest.insert(
            "legacyRosterTaskHash".to_string(),
            serde_json::json!(hash_text("tampered")),
        );
        let error = read_legacy_roster_generation_context(&test_run(manifest))
            .expect_err("taskHash 篡改应拒绝");
        assert_eq!(error, "GENERATION_LEGACY_TASK_INVALID");
    }
}
