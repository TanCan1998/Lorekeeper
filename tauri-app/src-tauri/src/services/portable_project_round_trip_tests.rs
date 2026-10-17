//! 复杂合成项目的便携往返回归：只验证存储/读面，不等同 GUI 或审修效果证明。

use super::*;
use crate::repositories::{
    blueprint_repository as blueprints, draft_repository as drafts,
    generation_run_repository::GenerationRunRepository, import_run_repository as imports,
    recovery_candidate_repository as recovery, review_repository as reviews,
};
use crate::services::portable_project_export::{
    export_portable_project, ExportPortableProjectInput,
};
use serde_json::json;

const ORIGIN: &str = "123e4567-e89b-12d3-a456-426614174000";
const FIRST: &str = "987e6543-e21b-42d3-a456-426614174000";
const SECOND: &str = "456e7890-e21b-42d3-a456-426614174000";
const CREATED: &str = "2026-10-17T07:00:00.000Z";

struct Fixture {
    draft_id: i64,
    review_id: i64,
    candidate_id: String,
    avatar: Vec<u8>,
}

fn seed(source: &Path) -> Fixture {
    let storage = source.join(crate::db::PROJECT_DIR_NAME);
    std::fs::create_dir_all(storage.join("avatars")).unwrap();
    std::fs::create_dir_all(storage.join("prompts")).unwrap();
    std::fs::write(storage.join("prompts/round-trip.md"), b"synthetic-prompt").unwrap();
    let avatar_path = storage.join("avatars/character.png");
    image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 255]))
        .save(&avatar_path)
        .unwrap();
    let avatar = std::fs::read(&avatar_path).unwrap();
    std::fs::write(storage.join("avatars/orphan.png"), &avatar).unwrap();
    let conn = Connection::open(storage.join(RESTORE_DATABASE_ENTRY)).unwrap();
    crate::db::schema::create_tables(&conn).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    blueprints::upsert(
        &conn,
        &serde_json::from_value(json!({"chapterNumber": 1, "title": "合成章"})).unwrap(),
    )
    .unwrap();
    let draft_id = drafts::create(
        &conn,
        &serde_json::from_value(json!({
            "chapterNumber": 1, "source": "write", "content": "合成草稿片段", "wordCount": 6
        }))
        .unwrap(),
    )
    .unwrap();
    let draft = drafts::get_full(&conn, draft_id).unwrap().unwrap();
    let review_id = reviews::create(
        &conn,
        &serde_json::from_value(json!({
            "baseDraftId": draft_id, "content": "合成审稿材料",
            "expectedSource": {
                "id": draft_id, "chapterNumber": 1, "version": draft.meta.version,
                "status": draft.meta.status, "content": draft.content
            }
        }))
        .unwrap(),
    )
    .unwrap()
    .id;
    let candidate = recovery::record(
        &conn,
        ORIGIN,
        &serde_json::from_value(json!({
            "runId": "recovery-run", "stepId": "recovery-step", "chapterNumber": 1,
            "source": {"chapterNumber": 1, "title": "合成章"},
            "sourceDraft": {"id": draft_id, "version": draft.meta.version},
            "visibleText": "合成恢复片段", "failureCode": "TIMEOUT"
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(candidate.source_current);
    conn.execute_batch(
        "INSERT INTO generation_roots (root_action_id,idempotency_key,action_json,budget_json)
         VALUES ('old-root','old-key','{}','{}');
         INSERT INTO generation_runs (run_id,root_action_id,binding_json,status,created_at_ms,open_key)
         VALUES ('old-run','old-root','{}','completed',0,'old-open');
         INSERT INTO generation_attempts VALUES
         ('old-attempt','old-reservation','old-run','old-root','{}','{}','old-nonce');
         INSERT INTO generation_artifacts (artifact_id,attempt_id,run_id,artifact_json,revision,status)
         VALUES ('old-artifact','old-attempt','old-run','{}',1,'complete');
         INSERT INTO import_runs (id,root_run_id,effect_namespace,source_fingerprint,
         manifest_fingerprint,locale,stage,total_chapters,manifest_chapter_count)
         VALUES ('old-import','old-import','old-import','fingerprint','manifest','zh-CN','prepared',1,1);
         INSERT INTO characters (character_id,name) VALUES ('character-1','合成角色');",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO character_avatar_assets
         (character_id,asset_revision,relative_path,content_hash,mime,byte_size,source_reference)
         VALUES ('character-1',1,'avatars/character.png',?1,'image/png',?2,'synthetic-reference')",
        rusqlite::params![sha256_hex(&avatar), avatar.len() as i64],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO character_avatar_unresolved
         (record_id,disposition,source_reference,source_relative_path,preserved_relative_path,
         content_hash,mime,byte_size,candidate_character_ids_json)
         VALUES ('orphan-1','orphan','synthetic-orphan','avatars/orphan.png','avatars/orphan.png',
         ?1,'image/png',?2,'[]')",
        rusqlite::params![sha256_hex(&avatar), avatar.len() as i64],
    )
    .unwrap();
    Fixture {
        draft_id,
        review_id,
        candidate_id: candidate.candidate_id,
        avatar,
    }
}

fn export(source: &Path, archive: &Path, staging: &Path, origin: &str, generation: &str) {
    export_portable_project(&ExportPortableProjectInput {
        source_project_root: source,
        origin_project_id: origin,
        target_archive_path: archive,
        attempt_parent_path: staging,
        snapshot_generation: Some(generation),
        created_at: CREATED,
        limits: None,
    })
    .unwrap();
}

fn assert_restored(root: &Path, fixture: &Fixture, project_id: &str) {
    let project = crate::db::ProjectDatabase::open(root).unwrap();
    let conn = project.connection();
    let draft = drafts::get_full(conn, fixture.draft_id).unwrap().unwrap();
    assert_eq!(draft.content, "合成草稿片段");
    assert_eq!(draft.meta.chapter_number, 1);
    assert_eq!(draft.meta.version, 1);
    let review = reviews::get_full(conn, fixture.review_id).unwrap().unwrap();
    assert_eq!(review.content, "合成审稿材料");
    assert_eq!(review.source_draft.unwrap().content, draft.content);
    let candidates = recovery::list_pending(conn).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].candidate_id, fixture.candidate_id);
    assert_eq!(candidates[0].visible_text, "合成恢复片段");
    assert!(!candidates[0].source_current);
    assert!(candidates[0].source_hash.is_empty());
    assert_eq!(
        candidates[0].project_id, ORIGIN,
        "历史来源身份不改写为新执行身份"
    );
    assert!(imports::get(conn, "old-import").unwrap().is_some());
    assert!(imports::list_resumable(conn).unwrap().is_empty());
    let generation = GenerationRunRepository::default();
    assert!(generation
        .read_listable_run(conn, "old-run", project_id)
        .unwrap()
        .is_none());
    assert!(generation.list_candidates(conn, None).unwrap().is_empty());
    assert!(generation
        .list_candidates(conn, Some("old-run"))
        .unwrap()
        .is_empty());
    generation.recover_interrupted(conn).unwrap();
    for sql in [
        "UPDATE generation_roots SET revision=1 WHERE root_action_id='old-root'",
        "UPDATE recovery_candidates SET status='continued'",
        "UPDATE import_runs SET status='running' WHERE id='old-import'",
    ] {
        assert!(conn
            .execute(sql, [])
            .unwrap_err()
            .to_string()
            .contains("PORTABLE_RUNTIME_FROZEN"));
    }
    let storage = root.join(crate::db::PROJECT_DIR_NAME);
    assert_eq!(
        std::fs::read(storage.join("avatars/character.png")).unwrap(),
        fixture.avatar
    );
    assert_eq!(
        std::fs::read(storage.join("avatars/orphan.png")).unwrap(),
        fixture.avatar
    );
    assert_eq!(
        std::fs::read(storage.join("prompts/round-trip.md")).unwrap(),
        b"synthetic-prompt"
    );
    // 复用头像命令实际调用的服务读链，既验证资产内容，也验证惰性孤儿核对不会误改历史。
    let assets = crate::services::character_asset_service::read_many(
        conn,
        &storage,
        &["character-1".to_string()],
    )
    .unwrap();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].asset_revision, 1);
    assert_eq!(assets[0].mime, "image/png");
    assert_eq!(
        crate::services::character_asset_service::base64_decode(&assets[0].base64).unwrap(),
        fixture.avatar
    );
    assert_eq!(
        crate::services::character_asset_service::record_reclaimable_orphans(conn, &storage)
            .unwrap(),
        0
    );
    let avatar_rows: i64 = conn.query_row(
        "SELECT (SELECT count(*) FROM character_avatar_assets)+(SELECT count(*) FROM character_avatar_unresolved)",
        [], |row| row.get(0)
    ).unwrap();
    assert_eq!(avatar_rows, 2);
}

#[test]
fn complex_archive_restores_reopens_and_reexports_history_materials_and_assets_test() {
    let base = std::env::temp_dir().join(format!(
        "lorekeeper-complex-round-trip-{}",
        crate::project_access::random_uuid_v4()
    ));
    let source = base.join("source");
    let staging = base.join("staging");
    std::fs::create_dir_all(&staging).unwrap();
    let fixture = seed(&source);
    let source = std::fs::canonicalize(source).unwrap();
    let staging = std::fs::canonicalize(staging).unwrap();
    let source_database = source
        .join(crate::db::PROJECT_DIR_NAME)
        .join(RESTORE_DATABASE_ENTRY);
    let source_hash = sha256_file(&source_database).unwrap();
    let archive = base.join("first.ainovel");
    export(&source, &archive, &staging, ORIGIN, "complex-first");
    let first_root = base.join("first");
    let first = restore_portable_project(&RestorePortableProjectInput {
        archive_path: &archive,
        target_project_root: &first_root,
        created_at: CREATED,
        target_project_id: Some(FIRST),
        cancel: None,
        limits: None,
    })
    .unwrap();
    assert_eq!(first.origin_project_id, ORIGIN);
    assert_eq!(first.target_project_id, FIRST);
    assert_restored(&first_root, &fixture, FIRST);
    assert_restored(&first_root, &fixture, FIRST);
    // 再导出恢复副本，再恢复为第三身份；不把二次转移等同重新授权历史。
    let archive = base.join("second.ainovel");
    export(&first_root, &archive, &staging, FIRST, "complex-second");
    let second_root = base.join("second");
    let second = restore_portable_project(&RestorePortableProjectInput {
        archive_path: &archive,
        target_project_root: &second_root,
        created_at: CREATED,
        target_project_id: Some(SECOND),
        cancel: None,
        limits: None,
    })
    .unwrap();
    assert_eq!(second.origin_project_id, FIRST);
    assert_eq!(second.target_project_id, SECOND);
    assert_restored(&second_root, &fixture, SECOND);
    assert_restored(&second_root, &fixture, SECOND);
    assert_eq!(
        sha256_file(&source_database).unwrap(),
        source_hash,
        "原始源库必须逐字节不变"
    );
    let original = Connection::open(&source_database).unwrap();
    assert!(recovery::list_pending(&original).unwrap()[0].source_current);
    assert_eq!(imports::list_resumable(&original).unwrap().len(), 1);
    drop(original);
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn restored_asset_drift_is_not_silently_repaired_or_reexported_test() {
    let base = std::env::temp_dir().join(format!(
        "lorekeeper-asset-drift-{}",
        crate::project_access::random_uuid_v4()
    ));
    let source = base.join("source");
    let staging = base.join("staging");
    std::fs::create_dir_all(&staging).unwrap();
    let fixture = seed(&source);
    let source = std::fs::canonicalize(source).unwrap();
    let staging = std::fs::canonicalize(staging).unwrap();
    let archive = base.join("first.ainovel");
    export(&source, &archive, &staging, ORIGIN, "asset-drift");
    let target = base.join("restored");
    let input = RestorePortableProjectInput {
        archive_path: &archive,
        target_project_root: &target,
        created_at: CREATED,
        target_project_id: Some(FIRST),
        cancel: None,
        limits: None,
    };
    restore_portable_project(&input).unwrap();
    let storage = target.join(crate::db::PROJECT_DIR_NAME);
    let changed = storage.join("avatars/character.png");
    let changed_bytes = b"synthetic-corrupted-image";
    std::fs::write(&changed, changed_bytes).unwrap();
    {
        let project = crate::db::ProjectDatabase::open(&target).unwrap();
        let avatars = crate::services::character_asset_service::read_many(
            project.connection(),
            &storage,
            &["character-1".to_string()],
        )
        .unwrap();
        assert!(avatars.is_empty(), "破损头像不能作为有效数据返回给渲染层");
        assert_eq!(
            recovery::list_pending(project.connection()).unwrap()[0].visible_text,
            "合成恢复片段"
        );
    }
    // 同档重试不覆盖用户已修改的目标；二次导出也必须拒绝哈希漂移。
    assert_eq!(
        restore_portable_project(&input).unwrap_err(),
        "PORTABLE_RESTORE_TARGET_EXISTS"
    );
    assert_eq!(std::fs::read(&changed).unwrap(), changed_bytes);
    let second_archive = base.join("must-not-publish.ainovel");
    let error = export_portable_project(&ExportPortableProjectInput {
        source_project_root: &target,
        origin_project_id: FIRST,
        target_archive_path: &second_archive,
        attempt_parent_path: &staging,
        snapshot_generation: Some("drift-reexport"),
        created_at: CREATED,
        limits: None,
    })
    .unwrap_err();
    assert_eq!(error, "PORTABLE_SOURCE_CHANGED");
    assert!(!second_archive.exists());
    assert_eq!(std::fs::read(&changed).unwrap(), changed_bytes);
    assert_eq!(
        std::fs::read(storage.join("avatars/orphan.png")).unwrap(),
        fixture.avatar
    );
    assert!(
        std::fs::read_dir(&staging).unwrap().next().is_none(),
        "失败暂存目录必须回收"
    );
    let original_storage = source.join(crate::db::PROJECT_DIR_NAME);
    assert_eq!(
        std::fs::read(original_storage.join("avatars/character.png")).unwrap(),
        fixture.avatar
    );
    std::fs::remove_dir_all(base).unwrap();
}
