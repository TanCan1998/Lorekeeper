//! 批次 G4-1：当前便携权威（自 `electron/services/portable-current-authority.ts` 平移）。
//!
//! 上游事实源：`electron/services/portable-current-authority.ts`（82 行）。
//! 语义铁律：错误码逐字一致（`PORTABLE_CURRENT_AUTHORITY_INVALID`）。
//!
//! 读取流程（与上游逐行对应）：
//! 1. `storageRoot = resolve(projectStorageRoot)`；
//! 2. `freeze = readPortableRuntimeFreeze(dirname(storageRoot), __testHooks)`，
//!    抛错 → `INVALID`；
//! 3. `!freeze.active` → `null`（普通项目，无当前权威）；
//! 4. `readRegular(join(storageRoot, AUTHORITY_FILE))`——与 `regularFile`
//!    同款 TOCTOU 防护，但 `ENOENT` 同样视为 `INVALID`（权威文件必须存在）；
//! 5. `JSON.parse` → `parsePortableTransferAuthority`；
//! 6. `targetProjectId !== projectId` → `INVALID`；
//! 7. `verifyPortableTransferAuthorityHistory(database, authority, freeze)`；
//! 8. 返回「冻结的」投影 `{receiptId, originProjectId, snapshotGeneration}`；
//!    任何非 `PortableCurrentAuthorityError` 异常 → `INVALID`。
//!
//! 纯逻辑、无 Tauri 依赖；仅依赖 `rusqlite::Connection` 只读查询面。

use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::services::portable_runtime_freeze::{
    read_portable_runtime_freeze, regular_file, resolve_lexical, FreezeTestHooks, AUTHORITY_FILE,
};
use crate::services::portable_transfer_authority::{
    parse_portable_transfer_authority, verify_portable_transfer_authority_history,
};

/// 权威文件字节上限（上游 `MAX_BYTES = 4 * 1024 * 1024`，模块私有）。
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// 当前权威投影（上游返回值 `{receiptId, originProjectId, snapshotGeneration}`）。
///
/// 上游语义为「冻结的」只读投影；Rust 侧以值类型交付，
/// 调用方无法借此修改权威文件或库内事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableCurrentAuthority {
    pub receipt_id: String,
    pub origin_project_id: String,
    pub snapshot_generation: String,
}

/// 当前权威错误（上游 `PortableCurrentAuthorityError`，`code` 逐字一致）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableCurrentAuthorityError;

impl PortableCurrentAuthorityError {
    /// 上游错误码字符串（逐字一致）。
    pub fn code(&self) -> &'static str {
        "PORTABLE_CURRENT_AUTHORITY_INVALID"
    }
}

impl std::fmt::Display for PortableCurrentAuthorityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for PortableCurrentAuthorityError {}

/// `readPortableCurrentAuthority` 输入（上游对象参数）。
pub struct ReadPortableCurrentAuthorityInput<'a> {
    pub database: &'a Connection,
    /// 项目存储根（`<projectRoot>/.ai-novel`）。
    pub project_storage_root: &'a str,
    /// 期望的目标项目 ID（`authority.targetProjectId` 必须等于它）。
    pub project_id: &'a str,
    /// 确定性替换竞态测试缝（上游 `__testHooks`，`@internal`）。
    pub test_hooks: Option<&'a FreezeTestHooks<'a>>,
}

fn invalid() -> PortableCurrentAuthorityError {
    PortableCurrentAuthorityError
}

/// Node `path.dirname(value)`：根路径的父路径是自身。
fn path_parent(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => path,
    }
}

/// 上游 `readPortableCurrentAuthority(input)`：
/// 读取冻结目标项目当前绑定的转移权威。
///
/// - 无激活冻结守卫（普通项目）→ `Ok(None)`；
/// - 权威文件缺失/损坏/目标不匹配/历史校验失败 → `INVALID`。
pub fn read_portable_current_authority(
    input: &ReadPortableCurrentAuthorityInput<'_>,
) -> Result<Option<PortableCurrentAuthority>, PortableCurrentAuthorityError> {
    let storage_root = resolve_lexical(input.project_storage_root);
    let freeze = read_portable_runtime_freeze(
        Some(&path_parent(&storage_root).to_string_lossy()),
        input.test_hooks,
    )
    .map_err(|_| invalid())?;
    if !freeze.active() {
        return Ok(None);
    }
    let after_open = input.test_hooks.and_then(|hooks| hooks.after_open);
    let file = storage_root.join(AUTHORITY_FILE);
    // 上游 `readRegular` 与 `regularFile` 同款 TOCTOU 防护，
    // 但 ENOENT 同样视为 invalid（权威文件必须存在）。
    let bytes = regular_file(&file.to_string_lossy(), MAX_BYTES, after_open)
        .map_err(|_| invalid())?
        .ok_or_else(invalid)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let authority = parse_portable_transfer_authority(&value).map_err(|_| invalid())?;
    if authority.target_project_id.as_deref() != Some(input.project_id) {
        return Err(invalid());
    }
    verify_portable_transfer_authority_history(input.database, &authority, Some(&freeze))
        .map_err(|_| invalid())?;
    Ok(Some(PortableCurrentAuthority {
        receipt_id: authority.receipt_id,
        origin_project_id: authority.origin_project_id,
        snapshot_generation: authority.snapshot_generation,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use rusqlite::params;
    use sha2::{Digest, Sha256};

    use crate::services::portable_runtime_freeze::FREEZE_FILE;
    use crate::services::portable_transfer_authority::{
        create_portable_transfer_authority, map_portable_transfer_authority,
        serialize_portable_transfer_authority, CreatePortableTransferAuthorityInput,
    };

    const ORIGIN_PROJECT_ID: &str = "123e4567-e89b-12d3-a456-426614174000";
    const TARGET_PROJECT_ID: &str = "987e6543-e21b-42d3-a456-426614174000";
    const SNAPSHOT_GENERATION: &str = "gen.1";

    /// 测试用 SHA-256 hex（与生产 `sha256_hex` 同算法）。
    fn sha256_hex(value: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(value.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TempProject {
        base: PathBuf,
    }

    impl TempProject {
        fn new(label: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "lorekeeper-g4-1-current-{}-{}-{}",
                label,
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst),
            ));
            let _ = fs::remove_dir_all(&base);
            fs::create_dir_all(&base).expect("temp project root created");
            Self { base }
        }

        fn storage_root(&self) -> PathBuf {
            self.base.join(".ai-novel")
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

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

    /// 无冻结记录的激活冻结文档（`records: []`）。
    fn write_freeze_document(project: &TempProject) {
        fs::create_dir_all(project.storage_root()).expect("storage root created");
        fs::write(
            project.storage_root().join(FREEZE_FILE),
            format!(
                r#"{{"version":1,"originProjectId":"{ORIGIN_PROJECT_ID}","snapshotGeneration":"{SNAPSHOT_GENERATION}","nonReplayable":true,"requiresRuntimeFreezeGuard":true,"records":[],"avatarReferenceProjections":[]}}"#
            ),
        )
        .expect("freeze document written");
    }

    /// 构造并落盘绑定到 `project_id` 的权威文件，返回序列化字节。
    fn write_authority_file(
        database: &Connection,
        project: &TempProject,
        project_id: &str,
    ) -> Vec<u8> {
        insert_finalized(database, 1, 1, "正文", "fin.1");
        let origin = create_portable_transfer_authority(CreatePortableTransferAuthorityInput {
            freeze: None,
            database,
            origin_project_id: ORIGIN_PROJECT_ID,
            snapshot_generation: SNAPSHOT_GENERATION,
            portable_database_sha256: &"b".repeat(64),
        })
        .expect("authority created");
        let mapped = map_portable_transfer_authority(origin, project_id).expect("mapped");
        let bytes = serialize_portable_transfer_authority(&mapped).expect("serialized");
        fs::write(project.storage_root().join(AUTHORITY_FILE), &bytes)
            .expect("authority file written");
        bytes
    }

    fn read(
        database: &Connection,
        project: &TempProject,
        project_id: &str,
    ) -> Result<Option<PortableCurrentAuthority>, PortableCurrentAuthorityError> {
        let storage_root = project.storage_root().to_string_lossy().into_owned();
        read_portable_current_authority(&ReadPortableCurrentAuthorityInput {
            database,
            project_storage_root: &storage_root,
            project_id,
            test_hooks: None,
        })
    }

    #[test]
    fn error_code_matches_upstream() {
        assert_eq!(
            PortableCurrentAuthorityError.code(),
            "PORTABLE_CURRENT_AUTHORITY_INVALID"
        );
        assert_eq!(
            PortableCurrentAuthorityError.to_string(),
            "PORTABLE_CURRENT_AUTHORITY_INVALID"
        );
    }

    #[test]
    fn read_returns_none_for_normal_project() {
        let database = setup_database();
        let project = TempProject::new("normal");
        // 无 .ai-novel 存储根 = 普通项目，无当前权威。
        assert_eq!(read(&database, &project, TARGET_PROJECT_ID), Ok(None));
    }

    #[test]
    fn read_returns_current_authority() {
        let database = setup_database();
        let project = TempProject::new("current");
        write_freeze_document(&project);
        let bytes = write_authority_file(&database, &project, TARGET_PROJECT_ID);
        let current = read(&database, &project, TARGET_PROJECT_ID)
            .expect("current authority")
            .expect("active freeze yields authority");
        // 收据以权威文件内 receiptId 为准。
        let value: Value = serde_json::from_slice(&bytes).expect("json value");
        let expected_receipt_id = value
            .get("receiptId")
            .and_then(Value::as_str)
            .expect("receiptId present");
        assert_eq!(current.receipt_id, expected_receipt_id);
        assert_eq!(current.origin_project_id, ORIGIN_PROJECT_ID);
        assert_eq!(current.snapshot_generation, SNAPSHOT_GENERATION);
    }

    #[test]
    fn read_rejects_target_project_mismatch() {
        let database = setup_database();
        let project = TempProject::new("mismatch");
        write_freeze_document(&project);
        write_authority_file(&database, &project, TARGET_PROJECT_ID);
        // 权威绑定 TARGET_PROJECT_ID，以 ORIGIN_PROJECT_ID 读取 → invalid。
        assert!(read(&database, &project, ORIGIN_PROJECT_ID).is_err());
    }

    #[test]
    fn read_rejects_missing_authority_file() {
        let database = setup_database();
        let project = TempProject::new("no-authority");
        write_freeze_document(&project);
        // 冻结激活但权威文件缺失：ENOENT 同样 → invalid。
        assert!(read(&database, &project, TARGET_PROJECT_ID).is_err());
    }

    #[test]
    fn read_maps_freeze_errors_to_invalid() {
        let database = setup_database();
        let project = TempProject::new("bad-freeze");
        fs::create_dir_all(project.storage_root()).expect("storage root created");
        fs::write(project.storage_root().join(FREEZE_FILE), b"not json")
            .expect("malformed freeze document written");
        assert!(read(&database, &project, TARGET_PROJECT_ID).is_err());
    }

    #[test]
    fn read_rejects_history_divergence() {
        let database = setup_database();
        let project = TempProject::new("divergent");
        write_freeze_document(&project);
        write_authority_file(&database, &project, TARGET_PROJECT_ID);
        // 权威文件有效，但库内事实已变（收据被删）→ 历史校验失败。
        database
            .execute(
                "DELETE FROM finalization_outbox WHERE draft_id = 1",
                params![],
            )
            .expect("outbox deleted");
        assert!(read(&database, &project, TARGET_PROJECT_ID).is_err());
    }
}
