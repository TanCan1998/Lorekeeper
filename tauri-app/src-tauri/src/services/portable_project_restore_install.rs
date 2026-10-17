//! 便携项目恢复安装段 —— 平移自 `electron/services/project-restore-service.ts` 的
//! **安装执行段**，按**本项目口径**落地（2026-10-16 用户确认「不与基线互通、按本项目方案存储恢复」）。
//!
//! # 职责边界
//!
//! 计划校验（manifest → 「归档条目 → 目标相对路径」映射）在
//! [`crate::services::portable_project_restore`]；本模块把计划**落到磁盘**：
//! 暂存目录所有权清单、安装日志与预留标记、原子安装（硬链接发布 + 身份复核）、
//! 幂等重跑识别（基线 `installedReceipt`）、转移权威改写与新 `project.json` 生成、
//! 以及本项目口径下的派生索引重建（FTS5 `kb_fts`）。
//! 命令层（消费 archive + restore-target 两个授权、会话门禁）由 B113c 落地。
//!
//! # 本项目口径与基线差异
//!
//! | 面 | 基线（Electron） | 本项目 |
//! |---|---|---|
//! | 项目存储根 | `<targetRoot>/.ai-novel` | `<targetRoot>/.lore`（[`crate::db::PROJECT_DIR_NAME`]） |
//! | 运行时冻结侧车 | 历史投影与冻结键 | 本项目空投影侧车，安装前逐行重建比对，安装后必须激活守卫 |
//! | 知识面 | 还原 LanceDB 快照（`portable-knowledge-source.json`） | **无**：知识事实随库入档，恢复后只重建派生索引 FTS5；HNSW 快照由用户在应用内重建 |
//! | 库完整性 | `verifyProjectSqlite`（桌面迁移注册表 + schema 指纹） | [`assert_portable_source_schema`]（便携源 schema 对账） |
//! | 项目清单 | `createCanonicalProjectManifest` | 本项目 `project.json`（`schemaVersion` / `kind` / `projectId` / `createdAt`） |
//!
//! # 安全语义（与基线逐条对齐）
//!
//! - 暂存目录用**所有权清单**（路径 + 身份）管理：身份不符即拒绝操作，回收只删本次创建且身份未变的对象；
//! - 目标预留用「同父目录安装日志 + 目标根内硬链接标记」双份表达，重跑靠同一份日志识别；
//! - 安装阶段不覆盖任何既有文件：临时文件 `wx` 创建 → 哈希复核 → 硬链接发布（`AlreadyExists` 即拒绝）；
//! - 任何路径都要「字面路径 == realpath」且非符号链接，失败一律
//!   `PORTABLE_RESTORE_UNSAFE_TARGET`。

use std::collections::{BTreeMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::services::portable_project_archive::{
    absolute as archive_absolute, contained, extract_portable_project_archive, literal_path_text,
    real_path_text, same_object, ExtractPortableProjectArchiveInput, FileIdentity,
};

/// 归档层的身份/物理性助手把失败一律折叠为 `PORTABLE_ARCHIVE_SOURCE_UNSAFE`；
/// 恢复面统一要求 `PORTABLE_RESTORE_UNSAFE_TARGET`，故在此逐层包装。
fn owned_identity(path: &Path) -> Result<FileIdentity, String> {
    crate::services::portable_project_archive::path_identity(path).map_err(|_| unsafe_target())
}

fn owned_handle_identity(file: &File) -> Result<FileIdentity, String> {
    crate::services::portable_project_archive::handle_identity(file).map_err(|_| unsafe_target())
}

fn owned_safe_directory(directory: &Path) -> Result<(), String> {
    crate::services::portable_project_archive::safe_directory(directory)
        .map_err(|_| unsafe_target())
}

fn owned_link_count(path: &Path) -> Result<u64, String> {
    crate::services::portable_project_archive::link_count(path).map_err(|_| unsafe_target())
}
use crate::services::portable_project_field_policy::assert_portable_source_schema;
use crate::services::portable_project_format::{
    default_portable_archive_limits, portable_path_key, PortableArchiveLimits,
    PortableProjectManifest, PortableProjectManifestEntry,
};
use crate::services::portable_project_restore::{
    plan_restore, RestorePlan, RESTORE_DATABASE_ENTRY, RESTORE_PROJECT_MANIFEST_ENTRY,
    RESTORE_TRANSFER_AUTHORITY_ENTRY,
};
use crate::services::portable_transfer_authority::{
    map_portable_transfer_authority, parse_portable_transfer_authority,
    serialize_portable_transfer_authority, verify_portable_transfer_authority,
    PortableTransferAuthority,
};

/// 目标根内的预留标记名（与安装日志硬链接共享同一 inode）
pub const RESTORE_INSTALL_MARKER_NAME: &str = ".portable-restore-reservation.json";
/// 安装日志版本
const INSTALL_JOURNAL_VERSION: i64 = 1;
/// 安装日志与标记的字节上限（基线 `readPhysicalFile` 默认 4 MiB）
const INSTALL_JOURNAL_MAX_BYTES: u64 = 4 * 1024 * 1024;
/// 库副文件后缀（基线 `['-wal','-shm','-journal']`）
const DATABASE_SIDECAR_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];
/// 暂存安装临时文件名中的固定中缀
const INSTALL_TEMPORARY_SUFFIX_PREFIX: &str = ".portable-restore-";
/// 正在进行的安装日志键（进程内互斥；同一目标的并发恢复直接 `TARGET_EXISTS`）
static ACTIVE_INSTALL_JOURNALS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// 在完整恢复过程（含重建派生索引）持有目标锁，避免成功日志释放后的幂等重跑并发写库。
static ACTIVE_RESTORE_TARGETS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
struct TargetHold(String);
impl Drop for TargetHold {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE_RESTORE_TARGETS.lock() {
            active.remove(&self.0);
        }
    }
}

fn cancelled() -> String {
    "PORTABLE_RESTORE_CANCELLED".to_string()
}

fn invalid() -> String {
    "PORTABLE_RESTORE_INVALID".to_string()
}

fn target_exists() -> String {
    "PORTABLE_RESTORE_TARGET_EXISTS".to_string()
}

fn unsafe_target() -> String {
    "PORTABLE_RESTORE_UNSAFE_TARGET".to_string()
}

/// 恢复安装段的公开输入（命令层 B113c 按契约构造）
pub struct RestorePortableProjectInput<'a> {
    /// 已授权读取的便携归档路径（渲染进程从不携带路径）
    pub archive_path: &'a Path,
    /// 已授权创建的目标项目根（存储根为其下的 `.lore`）
    pub target_project_root: &'a Path,
    /// 新项目清单的 `createdAt`（ISO8601；由命令层生成以便测试确定性）
    pub created_at: &'a str,
    /// 注入的新项目 id（缺省随机 UUID v4；必须与 origin 不同）
    pub target_project_id: Option<&'a str>,
    /// 取消信号（基线 `AbortSignal`；本项目为命令层可置位的原子标志）
    pub cancel: Option<&'a std::sync::atomic::AtomicBool>,
    /// 归档限额（缺省 = 便携默认限额）
    pub limits: Option<PortableArchiveLimits>,
}

/// 恢复收据（对齐基线 `RestorePortableProjectReceipt`，追加本项目派生面记录）
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePortableProjectReceipt {
    pub origin_project_id: String,
    pub target_project_id: String,
    pub target_project_root: String,
    pub snapshot_generation: String,
    pub portable_database_sha256: String,
    /// 本项目口径：恢复后重建的派生索引表（派生面按 `rebuild-stale` 省略）
    #[serde(skip_serializing)]
    pub rebuilt_derived_tables: Vec<String>,
    pub requires_runtime_freeze_guard: bool,
}

/// 所有权清单中的对象类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnedKind {
    File,
    Directory,
}

/// 所有权清单中的单个对象（路径 + 身份）
#[derive(Debug, Clone)]
struct OwnedPath {
    path: PathBuf,
    kind: OwnedKind,
    identity: FileIdentity,
}

/// 暂存尝试的所有权清单（基线 `OwnedAttempt`）
#[derive(Debug)]
struct OwnedAttempt {
    root: PathBuf,
    paths: BTreeMap<PathBuf, OwnedPath>,
}

/// 安装日志（基线 `InstallJournal`，字段逐字对齐）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InstallJournal {
    version: i64,
    target_root: String,
    archive_sha256: String,
    origin_project_id: String,
    snapshot_generation: String,
    transfer_receipt_id: String,
    portable_database_sha256: String,
    target_project_id: String,
    reservation_token: String,
}

/// 已读入的安装日志（文档 + 路径 + 身份）
#[derive(Debug, Clone)]
struct ReadInstallJournal {
    document: InstallJournal,
    path: PathBuf,
    identity: FileIdentity,
}

/// 日志持有凭据：离开作用域时把日志键从进程内互斥集合移除（基线 `finally` 清理）
struct JournalHold {
    key: String,
}

impl Drop for JournalHold {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE_INSTALL_JOURNALS.lock() {
            active.remove(&self.key);
        }
    }
}

fn check_cancel(cancel: Option<&std::sync::atomic::AtomicBool>) -> Result<(), String> {
    match cancel {
        Some(flag) if flag.load(std::sync::atomic::Ordering::SeqCst) => Err(cancelled()),
        _ => Ok(()),
    }
}

/// 绝对化（`path.resolve` 语义：相对路径按进程工作目录解析）
fn absolute(path: &Path) -> Result<PathBuf, String> {
    archive_absolute(path)
}

/// 卷根或 Unix 根没有父目录，不能作为新副本落点。
fn is_filesystem_root(path: &Path) -> bool {
    path.parent().is_none()
}

/// SHA-256 十六进制（小写）
fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// 流式文件哈希（基线 `hashFile`）
fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|_| unsafe_target())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|_| unsafe_target())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// 基线 `hashPhysicalFile`：读取前后复核「常规文件、非符号链接、身份未变、realpath 稳定」
fn hash_physical_file(path: &Path) -> Result<String, String> {
    let before = owned_identity(path)?;
    if !before.is_file || before.is_symlink || real_path_text(path) != literal_path_text(path) {
        return Err(unsafe_target());
    }
    let mut file = File::open(path).map_err(|_| unsafe_target())?;
    if before != owned_handle_identity(&file)? {
        return Err(unsafe_target());
    }
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|_| unsafe_target())?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    let digest = format!("{:x}", hasher.finalize());
    let after = owned_identity(path)?;
    if !after.is_file
        || after.is_symlink
        || before != after
        || before != owned_handle_identity(&file)?
        || real_path_text(path) != literal_path_text(path)
    {
        return Err(unsafe_target());
    }
    Ok(digest)
}

/// 基线 `readPhysicalFile`：句柄与路径身份一致的常规文件整读
fn read_physical_file(path: &Path, max_bytes: u64) -> Result<(Vec<u8>, FileIdentity), String> {
    let mut file = File::open(path).map_err(|_| unsafe_target())?;
    let opened = owned_handle_identity(&file)?;
    let current = owned_identity(path)?;
    if !opened.is_file
        || opened.size > max_bytes
        || !current.is_file
        || current.is_symlink
        || !same_object(&opened, &current)
        || real_path_text(path) != literal_path_text(path)
    {
        return Err(unsafe_target());
    }
    let mut bytes: Vec<u8> = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unsafe_target())?;
    let final_opened = owned_handle_identity(&file)?;
    let final_path = owned_identity(path)?;
    if bytes.len() as u64 != opened.size
        || bytes.len() as u64 > max_bytes
        || opened != final_opened
        || opened != final_path
        || final_path.is_symlink
        || real_path_text(path) != literal_path_text(path)
    {
        return Err(unsafe_target());
    }
    Ok((bytes, opened))
}

/// 基线 `unlinkIdentity`：身份相符才删除
fn unlink_identity(path: &Path, identity: &FileIdentity) -> Result<(), String> {
    let actual = owned_identity(path)?;
    if !actual.is_file
        || actual.is_symlink
        || !same_object(identity, &actual)
        || real_path_text(path) != literal_path_text(path)
    {
        return Err(unsafe_target());
    }
    std::fs::remove_file(path).map_err(|_| unsafe_target())
}

/// 基线 `json(bytes)`：严格 UTF-8 + JSON
fn json_value(bytes: &[u8]) -> Result<Value, String> {
    let decoded = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    serde_json::from_str(decoded).map_err(|_| invalid())
}

/// 创建私有目录（`0o700`，Unix 生效）
fn create_private_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir(path).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// 基线 `exactOwned`：清单对象仍须是「同一对象 + 同类型 + 非符号链接 + realpath 稳定」
fn exact_owned(owned: &OwnedPath) -> Result<FileIdentity, String> {
    let actual = owned_identity(&owned.path)?;
    let right_kind = match owned.kind {
        OwnedKind::File => actual.is_file,
        OwnedKind::Directory => actual.is_dir,
    };
    if !right_kind
        || actual.is_symlink
        || !same_object(&owned.identity, &actual)
        || real_path_text(&owned.path) != literal_path_text(&owned.path)
    {
        return Err(unsafe_target());
    }
    Ok(actual)
}

/// 基线 `captureAttempt`：把暂存根与每个归档条目路径按身份登记进所有权清单
fn capture_attempt(root: &Path, plan: &RestorePlan) -> Result<OwnedAttempt, String> {
    let root = absolute(root)?;
    let mut attempt = OwnedAttempt {
        root: root.clone(),
        paths: BTreeMap::new(),
    };
    let outcome = (|| -> Result<(), String> {
        capture_owned(&mut attempt, &root, OwnedKind::Directory)?;
        for entry in &plan.entries {
            let segments: Vec<&str> = entry.archive_path.split('/').collect();
            let parents = segments.len().saturating_sub(1);
            let mut cursor = root.clone();
            for segment in segments.iter().take(parents) {
                cursor.push(segment);
                capture_owned(&mut attempt, &cursor, OwnedKind::Directory)?;
            }
            let name = segments.last().ok_or_else(invalid)?;
            cursor.push(name);
            capture_owned(&mut attempt, &cursor, OwnedKind::File)?;
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        cleanup_attempt(&attempt);
        return Err(error);
    }
    Ok(attempt)
}

/// 基线 `capture`：登记单个对象（重复路径须同类型且身份仍相符）
fn capture_owned(
    attempt: &mut OwnedAttempt,
    candidate: &Path,
    expected: OwnedKind,
) -> Result<(), String> {
    let resolved = absolute(candidate)?;
    if let Some(existing) = attempt.paths.get(&resolved).cloned() {
        if existing.kind != expected {
            return Err(unsafe_target());
        }
        exact_owned(&existing)?;
        return Ok(());
    }
    let identity = owned_identity(&resolved)?;
    if identity.is_symlink
        || !contained(&attempt.root, &resolved)
        || real_path_text(&resolved) != literal_path_text(&resolved)
    {
        return Err(unsafe_target());
    }
    let kind = if identity.is_file {
        OwnedKind::File
    } else if identity.is_dir {
        OwnedKind::Directory
    } else {
        return Err(unsafe_target());
    };
    if kind != expected {
        return Err(unsafe_target());
    }
    if kind == OwnedKind::File && owned_link_count(&resolved)? != 1 {
        return Err(unsafe_target());
    }
    attempt.paths.insert(
        resolved.clone(),
        OwnedPath {
            path: resolved,
            kind,
            identity,
        },
    );
    Ok(())
}

/// 基线 `cleanupAttempt`：逆序回收仍属本次尝试且身份未变的文件与目录
fn cleanup_attempt(attempt: &OwnedAttempt) {
    let mut ordered: Vec<OwnedPath> = attempt.paths.values().cloned().collect();
    ordered.sort_by(|left, right| {
        right
            .path
            .as_os_str()
            .len()
            .cmp(&left.path.as_os_str().len())
    });
    for owned in ordered {
        let Ok(actual) = owned_identity(&owned.path) else {
            continue;
        };
        if actual.is_symlink
            || !same_object(&owned.identity, &actual)
            || real_path_text(&owned.path) != literal_path_text(&owned.path)
        {
            continue;
        }
        match owned.kind {
            OwnedKind::File if actual.is_file => {
                let _ = std::fs::remove_file(&owned.path);
            }
            OwnedKind::Directory if actual.is_dir => {
                let _ = std::fs::remove_dir(&owned.path);
            }
            _ => {}
        }
    }
}

/// 基线 `mkdirOwned`：在清单根内建目录并登记（已登记则复核身份）
fn mkdir_owned(attempt: &mut OwnedAttempt, directory: &Path) -> Result<(), String> {
    let directory = absolute(directory)?;
    if !contained(&attempt.root, &directory) {
        return Err(unsafe_target());
    }
    if let Some(existing) = attempt.paths.get(&directory).cloned() {
        exact_owned(&existing)?;
        return Ok(());
    }
    let parent = directory.parent().ok_or_else(unsafe_target)?.to_path_buf();
    mkdir_owned(attempt, &parent)?;
    create_private_dir(&directory)?;
    let identity = owned_identity(&directory)?;
    attempt.paths.insert(
        directory.clone(),
        OwnedPath {
            path: directory.clone(),
            kind: OwnedKind::Directory,
            identity,
        },
    );
    let owned = attempt
        .paths
        .get(&directory)
        .cloned()
        .ok_or_else(unsafe_target)?;
    exact_owned(&owned)?;
    Ok(())
}

/// 清单内移动采用硬链接再删除旧名，保证跨平台无覆盖；仅限同一暂存根。
fn move_owned_file(attempt: &mut OwnedAttempt, source: &Path, target: &Path) -> Result<(), String> {
    let from = absolute(source)?;
    let to = absolute(target)?;
    let owned = attempt.paths.get(&from).cloned().ok_or_else(invalid)?;
    if owned.kind != OwnedKind::File {
        return Err(invalid());
    }
    exact_owned(&owned)?;
    if std::fs::symlink_metadata(&to).is_ok() {
        return Err(invalid());
    }
    let parent = to.parent().ok_or_else(invalid)?.to_path_buf();
    mkdir_owned(attempt, &parent)?;
    std::fs::hard_link(&from, &to).map_err(|_| invalid())?;
    let linked = owned_identity(&to)?;
    if !same_object(&owned.identity, &linked) {
        return Err(unsafe_target());
    }
    unlink_identity(&from, &owned.identity)?;
    attempt.paths.remove(&from);
    let identity = owned_identity(&to)?;
    attempt.paths.insert(
        to.clone(),
        OwnedPath {
            path: to.clone(),
            kind: OwnedKind::File,
            identity,
        },
    );
    let owned = attempt.paths.get(&to).cloned().ok_or_else(unsafe_target)?;
    exact_owned(&owned)?;
    Ok(())
}

/// 先登记独占创建的文件身份，再写入；写入失败也能安全回收，Unix 权限固定为 0600。
fn write_new_owned(attempt: &mut OwnedAttempt, file: PathBuf, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut handle = options.open(&file).map_err(|_| unsafe_target())?;
    let identity = owned_handle_identity(&handle)?;
    if !same_object(&identity, &owned_identity(&file)?) {
        return Err(unsafe_target());
    }
    attempt.paths.insert(
        file.clone(),
        OwnedPath {
            path: file.clone(),
            kind: OwnedKind::File,
            identity,
        },
    );
    handle.write_all(bytes).map_err(|_| unsafe_target())?;
    handle.sync_all().map_err(|_| unsafe_target())?;
    if !same_object(&identity, &owned_identity(&file)?) {
        return Err(unsafe_target());
    }
    Ok(())
}

/// 删除前复核旧文件；新文件用独占创建，绝不覆盖竞态插入的陌生对象。
fn replace_owned_file(attempt: &mut OwnedAttempt, file: &Path, bytes: &[u8]) -> Result<(), String> {
    let file = absolute(file)?;
    let owned = attempt.paths.get(&file).cloned().ok_or_else(invalid)?;
    if owned.kind != OwnedKind::File {
        return Err(invalid());
    }
    exact_owned(&owned)?;
    unlink_identity(&file, &owned.identity)?;
    attempt.paths.remove(&file);
    write_new_owned(attempt, file, bytes)
}

/// 目标不得存在，父目录必须仍由本次尝试持有。
fn write_owned_file(attempt: &mut OwnedAttempt, file: &Path, bytes: &[u8]) -> Result<(), String> {
    let file = absolute(file)?;
    if std::fs::symlink_metadata(&file).is_ok() {
        return Err(invalid());
    }
    mkdir_owned(attempt, file.parent().ok_or_else(invalid)?)?;
    write_new_owned(attempt, file, bytes)
}

/// 基线 `captureGeneratedFileIfPresent`：把「本次过程新生成的文件」纳入清理范围
fn capture_generated_file_if_present(
    attempt: &mut OwnedAttempt,
    file: &Path,
) -> Result<(), String> {
    let file = absolute(file)?;
    if std::fs::symlink_metadata(&file).is_err() {
        return Ok(());
    }
    if attempt.paths.contains_key(&file) {
        return Err(invalid());
    }
    let identity = owned_identity(&file)?;
    if !identity.is_file
        || identity.is_symlink
        || owned_link_count(&file)? != 1
        || !contained(&attempt.root, &file)
        || real_path_text(&file) != literal_path_text(&file)
    {
        return Err(unsafe_target());
    }
    attempt.paths.insert(
        file.clone(),
        OwnedPath {
            path: file,
            kind: OwnedKind::File,
            identity,
        },
    );
    Ok(())
}

/// 基线 `removeOwnedFile`：不在清单即无事可做
fn remove_owned_file(attempt: &mut OwnedAttempt, file: &Path) -> Result<(), String> {
    let file = absolute(file)?;
    let Some(owned) = attempt.paths.get(&file).cloned() else {
        return Ok(());
    };
    if owned.kind != OwnedKind::File {
        return Err(unsafe_target());
    }
    exact_owned(&owned)?;
    std::fs::remove_file(&file).map_err(|_| unsafe_target())?;
    attempt.paths.remove(&file);
    Ok(())
}

/// 基线 `removeEmptyOwnedDirectories`：清掉除白名单外的空目录（逆序）
fn remove_empty_owned_directories(
    attempt: &mut OwnedAttempt,
    except: &[PathBuf],
) -> Result<(), String> {
    let mut directories: Vec<OwnedPath> = attempt
        .paths
        .values()
        .filter(|owned| owned.kind == OwnedKind::Directory)
        .cloned()
        .collect();
    directories.sort_by(|left, right| {
        right
            .path
            .as_os_str()
            .len()
            .cmp(&left.path.as_os_str().len())
    });
    for owned in directories {
        if except.iter().any(|kept| *kept == owned.path) {
            continue;
        }
        exact_owned(&owned)?;
        let empty = std::fs::read_dir(&owned.path)
            .map_err(|_| unsafe_target())?
            .next()
            .is_none();
        if empty {
            std::fs::remove_dir(&owned.path).map_err(|_| unsafe_target())?;
            attempt.paths.remove(&owned.path);
        }
    }
    Ok(())
}

/// 目录下的名字清单（排序后返回）
fn directory_names(directory: &Path) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .map_err(|_| unsafe_target())?
        .map(|item| {
            item.map(|value| value.file_name().to_string_lossy().to_string())
                .map_err(|_| unsafe_target())
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    Ok(names)
}

/// 基线 `installJournalPath`：同父目录下按目标根规范化文本哈希命名
fn install_journal_path(target_root: &Path) -> Result<PathBuf, String> {
    let digest = sha256_hex(literal_path_text(target_root).as_bytes());
    let parent = target_root.parent().ok_or_else(unsafe_target)?;
    Ok(parent.join(format!(".portable-project-restore-{}.json", &digest[..32])))
}

fn hold_journal(key: &str) -> Result<JournalHold, String> {
    if !ACTIVE_INSTALL_JOURNALS
        .lock()
        .map_err(|_| unsafe_target())?
        .insert(key.to_string())
    {
        return Err(target_exists());
    }
    Ok(JournalHold {
        key: key.to_string(),
    })
}

fn journal_key(path: &Path) -> String {
    literal_path_text(path)
}

fn is_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_source_uuid(value: &str) -> bool {
    static UUID: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
        )
        .expect("UUID 模式应有效")
    });
    UUID.is_match(value)
}

/// 基线 `parseInstallJournal`：严格键集 + 字段形状校验（一律 `UNSAFE_TARGET`）
fn parse_install_journal(bytes: &[u8], target_root: &Path) -> Result<InstallJournal, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| unsafe_target())?;
    let document = value.as_object().ok_or_else(unsafe_target)?;
    let mut actual: Vec<&str> = document.keys().map(String::as_str).collect();
    actual.sort_unstable();
    let mut expected = [
        "version",
        "targetRoot",
        "archiveSha256",
        "originProjectId",
        "snapshotGeneration",
        "transferReceiptId",
        "portableDatabaseSha256",
        "targetProjectId",
        "reservationToken",
    ];
    expected.sort_unstable();
    if actual != expected {
        return Err(unsafe_target());
    }
    let text = |key: &str| document.get(key).and_then(Value::as_str);
    if document.get("version").and_then(Value::as_i64) != Some(INSTALL_JOURNAL_VERSION)
        || text("targetRoot") != Some(literal_path_text(target_root).as_str())
        || !text("archiveSha256").is_some_and(is_hash)
        || !text("originProjectId").is_some_and(is_source_uuid)
        || !text("snapshotGeneration")
            .is_some_and(|value| !value.is_empty() && value.chars().count() <= 512)
        || !text("transferReceiptId")
            .is_some_and(|value| !value.is_empty() && value.chars().count() <= 512)
        || !text("portableDatabaseSha256").is_some_and(is_hash)
        || !text("targetProjectId").is_some_and(crate::project_access::is_uuid_v4)
        || !text("reservationToken").is_some_and(crate::project_access::is_uuid_v4)
    {
        return Err(unsafe_target());
    }
    serde_json::from_value(value).map_err(|_| unsafe_target())
}

/// 基线 `readInstallJournal`
fn read_install_journal(target_root: &Path) -> Result<Option<ReadInstallJournal>, String> {
    let journal = install_journal_path(target_root)?;
    match std::fs::symlink_metadata(&journal) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(unsafe_target()),
    }
    let (bytes, identity) = read_physical_file(&journal, INSTALL_JOURNAL_MAX_BYTES)?;
    Ok(Some(ReadInstallJournal {
        document: parse_install_journal(&bytes, target_root)?,
        path: journal,
        identity,
    }))
}

/// 基线 `sameInstallSource`：日志描述的来源元组与本次归档/权威一致
fn same_install_source(
    journal: &InstallJournal,
    archive_sha256: &str,
    authority: &PortableTransferAuthority,
) -> bool {
    journal.archive_sha256 == archive_sha256
        && journal.origin_project_id == authority.origin_project_id
        && journal.snapshot_generation == authority.snapshot_generation
        && journal.transfer_receipt_id == authority.receipt_id
        && journal.portable_database_sha256 == authority.portable_database_sha256
}

/// 基线 `createInstallJournal`：`wx` 创建同父目录日志并纳入进程内互斥集合
fn create_install_journal(
    target_root: &Path,
    archive_sha256: &str,
    authority: &PortableTransferAuthority,
    target_project_id: &str,
) -> Result<(ReadInstallJournal, JournalHold), String> {
    let journal = install_journal_path(target_root)?;
    let key = journal_key(&journal);
    let hold = hold_journal(&key)?;
    let document = InstallJournal {
        version: INSTALL_JOURNAL_VERSION,
        target_root: literal_path_text(target_root),
        archive_sha256: archive_sha256.to_string(),
        origin_project_id: authority.origin_project_id.clone(),
        snapshot_generation: authority.snapshot_generation.clone(),
        transfer_receipt_id: authority.receipt_id.clone(),
        portable_database_sha256: authority.portable_database_sha256.clone(),
        target_project_id: target_project_id.to_string(),
        reservation_token: crate::project_access::random_uuid_v4(),
    };
    let body = format!(
        "{}\n",
        serde_json::to_string(&document).map_err(|_| unsafe_target())?
    );
    let mut handle = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&journal)
    {
        Ok(handle) => handle,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(target_exists())
        }
        Err(error) => return Err(error.to_string()),
    };
    handle
        .write_all(body.as_bytes())
        .map_err(|_| unsafe_target())?;
    handle.sync_all().map_err(|_| unsafe_target())?;
    drop(handle);
    let (_, identity) = read_physical_file(&journal, INSTALL_JOURNAL_MAX_BYTES)?;
    Ok((
        ReadInstallJournal {
            document,
            path: journal,
            identity,
        },
        hold,
    ))
}

/// 基线 `releaseInstallJournal`：按需删标记，再删日志，并释放进程内互斥
fn release_install_journal(
    journal: &ReadInstallJournal,
    target_root: &Path,
    remove_marker: bool,
) -> Result<(), String> {
    if remove_marker {
        unlink_identity(
            &target_root.join(RESTORE_INSTALL_MARKER_NAME),
            &journal.identity,
        )?;
    }
    unlink_identity(&journal.path, &journal.identity)?;
    if let Ok(mut active) = ACTIVE_INSTALL_JOURNALS.lock() {
        active.remove(&journal_key(&journal.path));
    }
    Ok(())
}

/// 基线 `reservationOwned`：目标根与标记仍归本日志所有
fn reservation_owned(
    journal: &ReadInstallJournal,
    target_root: &Path,
) -> Result<FileIdentity, String> {
    let root = owned_identity(target_root)?;
    if !root.is_dir
        || root.is_symlink
        || real_path_text(target_root) != literal_path_text(target_root)
    {
        return Err(unsafe_target());
    }
    let marker = target_root.join(RESTORE_INSTALL_MARKER_NAME);
    let (bytes, identity) = read_physical_file(&marker, INSTALL_JOURNAL_MAX_BYTES)?;
    if !same_object(&identity, &journal.identity)
        || parse_install_journal(&bytes, target_root)? != journal.document
    {
        return Err(unsafe_target());
    }
    Ok(root)
}

/// 基线 `cleanupInstallTemporaries`：清掉上次中断留下的、本次预留令牌对应的临时文件
fn cleanup_install_temporaries(
    journal: &ReadInstallJournal,
    target_root: &Path,
) -> Result<(), String> {
    let storage = target_root.join(crate::db::PROJECT_DIR_NAME);
    if std::fs::symlink_metadata(&storage).is_err() {
        return Ok(());
    }
    owned_safe_directory(&storage)?;
    let suffix = format!(
        "{}{}.tmp",
        INSTALL_TEMPORARY_SUFFIX_PREFIX, journal.document.reservation_token
    );
    fn visit(directory: &Path, suffix: &str) -> Result<(), String> {
        let mut names: Vec<PathBuf> = std::fs::read_dir(directory)
            .map_err(|_| unsafe_target())?
            .map(|item| item.map(|value| value.path()).map_err(|_| unsafe_target()))
            .collect::<Result<Vec<_>, _>>()?;
        names.sort();
        for entry in names {
            let info = owned_identity(&entry)?;
            if info.is_symlink || real_path_text(&entry) != literal_path_text(&entry) {
                return Err(unsafe_target());
            }
            if info.is_dir {
                visit(&entry, suffix)?;
                continue;
            }
            let Some(name) = entry
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
            else {
                return Err(unsafe_target());
            };
            if !info.is_file || !name.starts_with('.') || !name.ends_with(suffix) {
                continue;
            }
            let final_name = &name[1..name.len() - suffix.len()];
            if final_name.is_empty() {
                return Err(unsafe_target());
            }
            let final_path = entry.with_file_name(final_name);
            if std::fs::symlink_metadata(&final_path).is_ok() {
                let final_info = owned_identity(&final_path)?;
                if !final_info.is_file || final_info.is_symlink || !same_object(&info, &final_info)
                {
                    return Err(unsafe_target());
                }
            }
            std::fs::remove_file(&entry).map_err(|_| unsafe_target())?;
        }
        Ok(())
    }
    visit(&storage, &suffix)
}

/// 基线 `relativeKey`：`/` 分隔 + [`portable_path_key`] 归一
fn relative_key(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| invalid())?
        .to_string_lossy()
        .replace('\\', "/");
    portable_path_key(&relative, default_portable_archive_limits().max_path_bytes)
}

/// 基线 `listPhysicalTree`：物理树（相对键集合）
fn list_physical_tree(root: &Path) -> Result<(HashSet<String>, HashSet<String>), String> {
    fn visit(
        root: &Path,
        directory: &Path,
        files: &mut HashSet<String>,
        directories: &mut HashSet<String>,
    ) -> Result<(), String> {
        let mut names: Vec<PathBuf> = std::fs::read_dir(directory)
            .map_err(|_| unsafe_target())?
            .map(|item| item.map(|value| value.path()).map_err(|_| unsafe_target()))
            .collect::<Result<Vec<_>, _>>()?;
        names.sort();
        for entry in names {
            let info = owned_identity(&entry)?;
            if info.is_symlink || real_path_text(&entry) != literal_path_text(&entry) {
                return Err(unsafe_target());
            }
            let key = relative_key(root, &entry)?;
            if info.is_dir {
                directories.insert(key);
                visit(root, &entry, files, directories)?;
            } else if info.is_file {
                files.insert(key);
            } else {
                return Err(unsafe_target());
            }
        }
        Ok(())
    }
    let mut files: HashSet<String> = HashSet::new();
    let mut directories: HashSet<String> = HashSet::new();
    visit(root, root, &mut files, &mut directories)?;
    Ok((files, directories))
}

/// 基线 `temporaryPath`：安装期临时文件路径
fn temporary_path(target_storage: &Path, relative: &Path, token: &str) -> PathBuf {
    let target = target_storage.join(relative);
    let name = target
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_default();
    let parent = target
        .parent()
        .map(|value| value.to_path_buf())
        .unwrap_or_else(|| target_storage.to_path_buf());
    parent.join(format!(
        ".{}{}{}.tmp",
        name, INSTALL_TEMPORARY_SUFFIX_PREFIX, token
    ))
}

/// 目标存储根下的条目路径（归档条目路径 → 本地路径）
fn storage_path(storage_root: &Path, archive_path: &str) -> PathBuf {
    storage_root.join(archive_path.replace('/', std::path::MAIN_SEPARATOR_STR))
}

/// 库副文件路径
fn sidecar_path(database_path: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{}", database_path.to_string_lossy(), suffix))
}

/// 从 manifest 取出指定便携路径键的条目
fn manifest_entry<'a>(
    manifest: &'a PortableProjectManifest,
    key: &str,
) -> Result<&'a PortableProjectManifestEntry, String> {
    let max_path_bytes = default_portable_archive_limits().max_path_bytes;
    manifest
        .entries
        .iter()
        .find(|entry| {
            portable_path_key(&entry.path, max_path_bytes)
                .map(|candidate| candidate == key)
                .unwrap_or(false)
        })
        .ok_or_else(invalid)
}

/// 单段物理目录校验（目录、非符号链接、realpath 稳定）
fn assert_physical_directory(directory: &Path) -> Result<(), String> {
    let identity = owned_identity(directory)?;
    if !identity.is_dir
        || identity.is_symlink
        || real_path_text(directory) != literal_path_text(directory)
    {
        return Err(unsafe_target());
    }
    Ok(())
}

/// 新项目清单字节（本项目 `project.json`：基线 `createCanonicalProjectManifest` 的对位物）
fn project_manifest_bytes(project_id: &str, created_at: &str) -> Vec<u8> {
    let manifest = serde_json::json!({
        "schemaVersion": 1,
        "kind": "ai-novel-project",
        "projectId": project_id,
        "createdAt": created_at,
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&manifest).unwrap_or_default()
    )
    .into_bytes()
}

/// 基线 `restoreReceipt`
fn restore_receipt(
    target_root: &Path,
    authority: &PortableTransferAuthority,
    target_project_id: &str,
) -> RestorePortableProjectReceipt {
    RestorePortableProjectReceipt {
        origin_project_id: authority.origin_project_id.clone(),
        target_project_id: target_project_id.to_string(),
        target_project_root: target_root.to_string_lossy().into_owned(),
        snapshot_generation: authority.snapshot_generation.clone(),
        portable_database_sha256: authority.portable_database_sha256.clone(),
        rebuilt_derived_tables: Vec::new(),
        requires_runtime_freeze_guard: true,
    }
}

/// 填上本项目派生索引重建记录
fn with_rebuilt_tables(
    mut receipt: RestorePortableProjectReceipt,
    rebuilt: Vec<String>,
) -> RestorePortableProjectReceipt {
    receipt.rebuilt_derived_tables = rebuilt;
    receipt
}

/// 基线 `installedReceipt`：识别「本归档已安装完成」的幂等重跑。
///
/// 本项目口径：冻结侧车必须存在且守卫激活；业务库逐行/结构证据必须匹配归档库，
/// 仅排除与编译期 DDL 一致的 FTS 派生对象，避免重建后物理字节变化破坏幂等性。
fn installed_receipt(
    target_root: &Path,
    authority: &PortableTransferAuthority,
    manifest: &PortableProjectManifest,
    expected_database: &str,
) -> Result<Option<RestorePortableProjectReceipt>, String> {
    if std::fs::symlink_metadata(target_root).is_err() {
        return Ok(None);
    }
    assert_physical_directory(target_root)?;
    let names = directory_names(target_root)?;
    if names
        .iter()
        .any(|name| name != crate::db::PROJECT_DIR_NAME && name != RESTORE_INSTALL_MARKER_NAME)
    {
        return Ok(None);
    }
    let storage = target_root.join(crate::db::PROJECT_DIR_NAME);
    let manifest_path = storage.join(RESTORE_PROJECT_MANIFEST_ENTRY);
    if std::fs::symlink_metadata(&manifest_path).is_err() {
        return Ok(None);
    }
    assert_physical_directory(&storage)?;
    let Ok((manifest_bytes, _)) = read_physical_file(&manifest_path, INSTALL_JOURNAL_MAX_BYTES)
    else {
        return Ok(None);
    };
    let Ok(manifest_value) = json_value(&manifest_bytes) else {
        return Ok(None);
    };
    if !crate::project_access::is_project_manifest(&manifest_value) {
        return Ok(None);
    }
    let Some(target_project_id) = manifest_value
        .get("projectId")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Ok(None);
    };
    let authority_path = storage.join(RESTORE_TRANSFER_AUTHORITY_ENTRY);
    let Ok((authority_bytes, _)) = read_physical_file(&authority_path, INSTALL_JOURNAL_MAX_BYTES)
    else {
        return Ok(None);
    };
    let Ok(authority_value) = json_value(&authority_bytes) else {
        return Ok(None);
    };
    let Ok(installed_authority) = parse_portable_transfer_authority(&authority_value) else {
        return Ok(None);
    };
    if installed_authority.origin_project_id != authority.origin_project_id
        || installed_authority.snapshot_generation != authority.snapshot_generation
        || installed_authority.receipt_id != authority.receipt_id
        || installed_authority.portable_database_sha256 != authority.portable_database_sha256
        || installed_authority.target_project_id.as_deref() != Some(target_project_id.as_str())
        || !crate::project_access::is_uuid_v4(&target_project_id)
        || target_project_id == authority.origin_project_id
    {
        return Ok(None);
    }
    let max_path_bytes = default_portable_archive_limits().max_path_bytes;
    let authority_key = portable_path_key(RESTORE_TRANSFER_AUTHORITY_ENTRY, max_path_bytes)?;
    for entry in &manifest.entries {
        let key = portable_path_key(&entry.path, max_path_bytes)?;
        if key == authority_key || key == RESTORE_DATABASE_ENTRY {
            continue;
        }
        let file = storage_path(&storage, &entry.path);
        if std::fs::symlink_metadata(&file).is_err() {
            return Ok(None);
        }
        if hash_physical_file(&file)? != entry.sha256 {
            return Ok(None);
        }
    }
    let database_path = storage.join(RESTORE_DATABASE_ENTRY);
    let Ok(database) =
        Connection::open_with_flags(&database_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return Ok(None);
    };
    let inspected = (|| -> Result<(), String> {
        database
            .execute_batch("PRAGMA foreign_keys = ON")
            .map_err(|_| invalid())?;
        if database_evidence(&database)? != expected_database {
            return Err(invalid());
        }
        let freeze = crate::services::portable_runtime_freeze::read_portable_runtime_freeze(
            Some(&target_root.to_string_lossy()),
            None,
        )
        .map_err(|_| invalid())?;
        if !freeze.active() {
            return Err(invalid());
        }
        verify_portable_transfer_authority(&database, &installed_authority, Some(&freeze))
            .map_err(|_| invalid())?;
        Ok(())
    })();
    drop(database);
    if inspected.is_err() {
        return Ok(None);
    }
    Ok(Some(restore_receipt(
        target_root,
        authority,
        &target_project_id,
    )))
}

/// 基线 `installStagedProject`：把暂存存储根原子发布到目标存储根
fn install_staged_project(
    attempt: &mut OwnedAttempt,
    storage: &Path,
    target_root: &Path,
    journal: &ReadInstallJournal,
) -> Result<(), String> {
    let root_identity = reservation_owned(journal, target_root)?;
    let names = directory_names(target_root)?;
    if names
        .iter()
        .any(|name| name != RESTORE_INSTALL_MARKER_NAME && name != crate::db::PROJECT_DIR_NAME)
    {
        return Err(unsafe_target());
    }
    let target_storage = target_root.join(crate::db::PROJECT_DIR_NAME);
    if std::fs::symlink_metadata(&target_storage).is_err() {
        create_private_dir(&target_storage)?;
    }
    assert_physical_directory(&target_storage)?;

    let sources: Vec<OwnedPath> = attempt
        .paths
        .values()
        .filter(|owned| owned.kind == OwnedKind::File && contained(storage, &owned.path))
        .cloned()
        .collect();
    let mut by_key: BTreeMap<String, OwnedPath> = BTreeMap::new();
    for source in &sources {
        by_key.insert(relative_key(storage, &source.path)?, source.clone());
    }
    let expected_files: HashSet<String> = by_key.keys().cloned().collect();
    let mut expected_directories: HashSet<String> = HashSet::new();
    for source in &sources {
        let relative = source
            .path
            .strip_prefix(storage)
            .map_err(|_| unsafe_target())?;
        let mut cursor = relative.parent();
        while let Some(parent) = cursor {
            if parent.as_os_str().is_empty() {
                break;
            }
            expected_directories.insert(relative_key(storage, &storage.join(parent))?);
            cursor = parent.parent();
        }
    }

    let token = journal.document.reservation_token.clone();
    for source in &sources {
        let relative = source
            .path
            .strip_prefix(storage)
            .map_err(|_| unsafe_target())?;
        let temporary = temporary_path(&target_storage, relative, &token);
        if std::fs::symlink_metadata(&temporary).is_err() {
            continue;
        }
        let info = owned_identity(&temporary)?;
        if !info.is_file
            || info.is_symlink
            || real_path_text(&temporary) != literal_path_text(&temporary)
        {
            return Err(unsafe_target());
        }
        let target = target_storage.join(relative);
        if std::fs::symlink_metadata(&target).is_ok() {
            let target_info = owned_identity(&target)?;
            if !target_info.is_file || target_info.is_symlink || !same_object(&info, &target_info) {
                return Err(unsafe_target());
            }
        }
        std::fs::remove_file(&temporary).map_err(|_| unsafe_target())?;
    }

    let (existing_files, existing_directories) = list_physical_tree(&target_storage)?;
    if existing_files
        .iter()
        .any(|key| !expected_files.contains(key))
        || existing_directories
            .iter()
            .any(|key| !expected_directories.contains(key))
    {
        return Err(unsafe_target());
    }
    let manifest_key = portable_path_key(
        RESTORE_PROJECT_MANIFEST_ENTRY,
        default_portable_archive_limits().max_path_bytes,
    )?;
    if existing_files.contains(&manifest_key) {
        return Err(unsafe_target());
    }

    let mut ordered: Vec<String> = expected_directories.iter().cloned().collect();
    ordered.sort_by(|left, right| left.split('/').count().cmp(&right.split('/').count()));
    for key in ordered {
        let directory = target_storage.join(key.replace('/', std::path::MAIN_SEPARATOR_STR));
        if std::fs::symlink_metadata(&directory).is_err() {
            create_private_dir(&directory)?;
        }
        assert_physical_directory(&directory)?;
    }

    for (key, source) in &by_key {
        if key == &manifest_key {
            continue;
        }
        install_staged_file(attempt, storage, &target_storage, &token, source)?;
    }
    reservation_owned(journal, target_root)?;
    let root_now = owned_identity(target_root)?;
    if !same_object(&root_identity, &root_now) {
        return Err(unsafe_target());
    }
    let names = directory_names(target_root)?;
    if names
        .iter()
        .any(|name| name != RESTORE_INSTALL_MARKER_NAME && name != crate::db::PROJECT_DIR_NAME)
    {
        return Err(unsafe_target());
    }
    let (final_files, final_directories) = list_physical_tree(&target_storage)?;
    if final_files
        .iter()
        .any(|key| key != &manifest_key && !expected_files.contains(key))
        || final_directories
            .iter()
            .any(|key| !expected_directories.contains(key))
    {
        return Err(unsafe_target());
    }
    let manifest_source = by_key.get(&manifest_key).ok_or_else(unsafe_target)?;
    install_staged_file(attempt, storage, &target_storage, &token, manifest_source)?;
    Ok(())
}

/// 基线 `installFile`：临时文件发布或与既有目标逐哈希比对
fn install_staged_file(
    attempt: &mut OwnedAttempt,
    storage: &Path,
    target_storage: &Path,
    token: &str,
    source: &OwnedPath,
) -> Result<(), String> {
    exact_owned(source)?;
    let relative = source
        .path
        .strip_prefix(storage)
        .map_err(|_| unsafe_target())?
        .to_path_buf();
    let target = target_storage.join(&relative);
    owned_safe_directory(target.parent().ok_or_else(unsafe_target)?)?;
    if std::fs::symlink_metadata(&target).is_ok() {
        if hash_physical_file(&target)? != hash_physical_file(&source.path)? {
            return Err(unsafe_target());
        }
    } else {
        let temporary = temporary_path(target_storage, &relative, token);
        copy_new_file(&source.path, &temporary)?;
        if hash_physical_file(&temporary)? != hash_physical_file(&source.path)? {
            return Err(unsafe_target());
        }
        match std::fs::hard_link(&temporary, &target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(unsafe_target())
            }
            Err(_) => return Err(unsafe_target()),
        }
        let temporary_info = owned_identity(&temporary)?;
        let linked = owned_identity(&target)?;
        if !linked.is_file || linked.is_symlink || !same_object(&temporary_info, &linked) {
            return Err(unsafe_target());
        }
        std::fs::remove_file(&temporary).map_err(|_| unsafe_target())?;
    }
    remove_owned_file(attempt, &source.path)?;
    Ok(())
}

/// 独占创建目标并复制全部字节（基线 `copyFileSync(..., COPYFILE_EXCL)`）
fn copy_new_file(source: &Path, target: &Path) -> Result<(), String> {
    let mut reader = File::open(source).map_err(|_| unsafe_target())?;
    let mut writer = match OpenOptions::new().write(true).create_new(true).open(target) {
        Ok(writer) => writer,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(unsafe_target())
        }
        Err(error) => return Err(error.to_string()),
    };
    std::io::copy(&mut reader, &mut writer).map_err(|_| unsafe_target())?;
    writer.sync_all().map_err(|_| unsafe_target())?;
    Ok(())
}

/// 本项目口径：恢复后的派生索引重建（归档按 `rebuild-stale` 省略 `kb_fts`）。
///
/// 知识**事实**（`kb_documents` / `kb_chunks` / `kb_embedding_spaces`）随库入档，
/// 只有 FTS5 全文索引需要按 `kb_chunks` 重建；HNSW 向量快照不在归档内，
/// 由用户在应用内按需重建（`kb:get-vector-rebuild-status`）。
fn rebuild_derived_tables(storage_root: &Path) -> Result<Vec<String>, String> {
    let database_path = storage_root.join(RESTORE_DATABASE_ENTRY);
    let database = Connection::open(&database_path).map_err(|_| invalid())?;
    database
        .execute_batch("PRAGMA foreign_keys = ON")
        .map_err(|_| invalid())?;
    database
        .execute_batch(crate::db::schema::CREATE_KB_FTS)
        .map_err(|_| invalid())?;
    crate::db::kb::fts::clear(&database).map_err(|_| invalid())?;
    let chunks: Vec<(String, String, String, String, String)> = {
        let mut statement = database
            .prepare("SELECT id, doc_id, file_name, corpus_kind, text FROM kb_chunks")
            .map_err(|_| invalid())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .map_err(|_| invalid())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|_| invalid())?
    };
    for (chunk_id, doc_id, file_name, corpus_kind, text) in &chunks {
        crate::db::kb::fts::replace_chunk(
            &database,
            &crate::db::kb::fts::FtsChunk {
                chunk_id,
                doc_id,
                file_name,
                corpus_kind,
                text,
            },
        )
        .map_err(|_| invalid())?;
    }
    Ok(vec!["kb_fts".to_string()])
}

/// 基线 `restorePortableProject`（本项目口径）
pub fn restore_portable_project(
    input: &RestorePortableProjectInput<'_>,
) -> Result<RestorePortableProjectReceipt, String> {
    check_cancel(input.cancel)?;
    let target_root = absolute(input.target_project_root)?;
    let target_key = literal_path_text(&target_root);
    if !ACTIVE_RESTORE_TARGETS
        .lock()
        .map_err(|_| unsafe_target())?
        .insert(target_key.clone())
    {
        return Err(target_exists());
    }
    let _target_hold = TargetHold(target_key);
    let target_parent = target_root
        .parent()
        .ok_or_else(unsafe_target)?
        .to_path_buf();
    if is_filesystem_root(&target_root) || std::fs::symlink_metadata(&target_parent).is_err() {
        return Err(unsafe_target());
    }
    owned_safe_directory(&target_parent)?;
    let archive_sha256 = hash_physical_file(input.archive_path)?;
    let extracted = extract_portable_project_archive(&ExtractPortableProjectArchiveInput {
        archive_path: input.archive_path,
        staging_parent_path: &target_parent,
        limits: input.limits,
    })?;
    let outcome = (|| {
        if hash_physical_file(input.archive_path)? != archive_sha256 {
            return Err(invalid());
        }
        crate::services::portable_project_archive::assert_attempt_directory(
            &extracted.staging_attempt,
            &extracted.staging_path,
        )
        .map_err(|_| unsafe_target())?;
        let plan = plan_restore(&extracted.manifest)?;
        let mut attempt = capture_attempt(&extracted.staging_path, &plan)?;
        let outcome = run_restore(
            &mut attempt,
            &plan,
            &extracted.manifest,
            input,
            &target_root,
            &archive_sha256,
        );
        cleanup_attempt(&attempt);
        outcome
    })();
    crate::services::portable_project_archive::cleanup_attempt(&extracted.staging_attempt);
    outcome
}

/// 基线 `restorePortableProject` 主体（暂存已捕获之后）
#[allow(clippy::too_many_arguments)]
fn run_restore(
    attempt: &mut OwnedAttempt,
    plan: &RestorePlan,
    manifest: &PortableProjectManifest,
    input: &RestorePortableProjectInput<'_>,
    target_root: &Path,
    archive_sha256: &str,
) -> Result<RestorePortableProjectReceipt, String> {
    let staging = attempt.root.clone();
    let authority_path = staging.join(RESTORE_TRANSFER_AUTHORITY_ENTRY);
    let (authority_bytes, _) = read_physical_file(&authority_path, INSTALL_JOURNAL_MAX_BYTES)?;
    let authority =
        parse_portable_transfer_authority(&json_value(&authority_bytes)?).map_err(|_| invalid())?;
    let database_entry = manifest_entry(manifest, RESTORE_DATABASE_ENTRY)?;
    let database_path = staging.join(RESTORE_DATABASE_ENTRY);
    if authority.target_project_id.is_some()
        || authority.origin_project_id != manifest.origin_project_id
        || authority.snapshot_generation != manifest.snapshot_generation
        || authority.portable_database_sha256 != database_entry.sha256
        || !manifest
            .transfer_receipt_ids
            .iter()
            .any(|id| id == &authority.receipt_id)
        || sha256_file(&database_path)? != authority.portable_database_sha256
    {
        return Err(invalid());
    }

    let expected_database;
    {
        let database =
            Connection::open_with_flags(&database_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|_| invalid())?;
        database
            .execute_batch("PRAGMA foreign_keys = ON")
            .map_err(|_| invalid())?;
        assert_portable_source_schema(&database).map_err(|_| invalid())?;
        let freeze_path = staging.join(crate::services::portable_project_history::FREEZE_ENTRY);
        let (freeze_bytes, _) = read_physical_file(
            &freeze_path,
            crate::services::portable_runtime_freeze::PORTABLE_RUNTIME_HISTORY_MAX_BYTES as u64,
        )?;
        let freeze_document = json_value(&freeze_bytes)?;
        crate::services::portable_project_history::verify_freeze_document(
            &database,
            &freeze_document,
            &manifest.origin_project_id,
            &manifest.snapshot_generation,
        )
        .map_err(|_| invalid())?;
        let ids: Vec<String> = freeze_document["records"]
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .map(|record| {
                record["projectionId"]
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(invalid)
            })
            .collect::<Result<_, _>>()?;
        if ids != manifest.history_projection_ids {
            return Err(invalid());
        }
        let freeze =
            crate::services::portable_runtime_freeze::parse_portable_runtime_freeze(&freeze_bytes)
                .map_err(|_| invalid())?;
        verify_portable_transfer_authority(&database, &authority, Some(&freeze))
            .map_err(|_| invalid())?;
        expected_database = database_evidence(&database)?;
    }
    for suffix in DATABASE_SIDECAR_SUFFIXES {
        capture_generated_file_if_present(attempt, &sidecar_path(&database_path, suffix))?;
    }

    let mut journal = read_install_journal(target_root)?;
    let mut _held = if let Some(current) = &journal {
        Some(hold_journal(&journal_key(&current.path))?)
    } else {
        None
    };
    if let Some(current) = &journal {
        if !same_install_source(&current.document, archive_sha256, &authority) {
            return Err(target_exists());
        }
    }
    if std::fs::symlink_metadata(target_root).is_ok() {
        if let Some(current) = &journal {
            reservation_owned(current, target_root)?;
            cleanup_install_temporaries(current, target_root)?;
        }
        if let Some(receipt) =
            installed_receipt(target_root, &authority, manifest, &expected_database)?
        {
            if let Some(current) = journal.take() {
                if current.document.target_project_id != receipt.target_project_id {
                    return Err(unsafe_target());
                }
                ACTIVE_INSTALL_JOURNALS
                    .lock()
                    .map_err(|_| unsafe_target())?
                    .insert(journal_key(&current.path));
                release_install_journal(&current, target_root, true)?;
            } else if std::fs::symlink_metadata(target_root.join(RESTORE_INSTALL_MARKER_NAME))
                .is_ok()
            {
                return Err(unsafe_target());
            }
            let rebuilt = rebuild_derived_tables(&target_root.join(crate::db::PROJECT_DIR_NAME))?;
            return Ok(with_rebuilt_tables(receipt, rebuilt));
        }
        let Some(current) = journal.as_ref() else {
            return Err(target_exists());
        };
        reservation_owned(current, target_root)?;
        // 已在读取日志后原子取得进程内持有凭据。
    } else if let Some(current) = journal.take() {
        unlink_identity(&current.path, &current.identity)?;
    }

    let target_project_id = journal
        .as_ref()
        .map(|current| current.document.target_project_id.clone())
        .or_else(|| input.target_project_id.map(str::to_string))
        .unwrap_or_else(crate::project_access::random_uuid_v4);
    if !crate::project_access::is_uuid_v4(&target_project_id)
        || target_project_id == authority.origin_project_id
    {
        return Err(invalid());
    }

    let storage = staging.join(crate::db::PROJECT_DIR_NAME);
    mkdir_owned(attempt, &storage)?;
    for entry in &plan.entries {
        let source = storage_path(&staging, &entry.archive_path);
        if contained(&storage, &source) {
            continue;
        }
        let target = storage_path(&storage, &entry.archive_path);
        move_owned_file(attempt, &source, &target)?;
    }
    let mapped = map_portable_transfer_authority(authority.clone(), &target_project_id)
        .map_err(|_| invalid())?;
    let mapped_bytes = serialize_portable_transfer_authority(&mapped).map_err(|_| invalid())?;
    replace_owned_file(
        attempt,
        &storage.join(RESTORE_TRANSFER_AUTHORITY_ENTRY),
        &mapped_bytes,
    )?;
    write_owned_file(
        attempt,
        &storage.join(RESTORE_PROJECT_MANIFEST_ENTRY),
        &project_manifest_bytes(&target_project_id, input.created_at),
    )?;
    for suffix in DATABASE_SIDECAR_SUFFIXES {
        remove_owned_file(attempt, &sidecar_path(&database_path, suffix))?;
    }
    remove_empty_owned_directories(attempt, &[staging.clone(), storage.clone()])?;
    let names = directory_names(&staging)?;
    if names.len() != 1 || names[0] != crate::db::PROJECT_DIR_NAME {
        return Err(unsafe_target());
    }
    check_cancel(input.cancel)?;

    if journal.is_none() {
        let (created, hold) =
            create_install_journal(target_root, archive_sha256, &authority, &target_project_id)?;
        _held = Some(hold);
        match std::fs::create_dir(target_root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                release_install_journal(&created, target_root, false)?;
                return Err(target_exists());
            }
            Err(error) => return Err(error.to_string()),
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(target_root, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| error.to_string())?;
        }
        if std::fs::hard_link(&created.path, target_root.join(RESTORE_INSTALL_MARKER_NAME)).is_err()
        {
            return Err(unsafe_target());
        }
        reservation_owned(&created, target_root)?;
        journal = Some(created);
    }
    check_cancel(input.cancel)?;
    let current = journal.as_ref().ok_or_else(unsafe_target)?.clone();
    install_staged_project(attempt, &storage, target_root, &current)?;
    let installed = installed_receipt(target_root, &authority, manifest, &expected_database)?
        .ok_or_else(unsafe_target)?;
    if installed.target_project_id != target_project_id {
        return Err(unsafe_target());
    }
    release_install_journal(&current, target_root, true)?;
    _held = None;
    let rebuilt = rebuild_derived_tables(&target_root.join(crate::db::PROJECT_DIR_NAME))?;
    Ok(with_rebuilt_tables(installed, rebuilt))
}

/// 安装后的派生 FTS 会改变库文件字节；幂等校验比较归档库的结构与行证据，
/// 仅允许与编译期 FTS DDL 完全一致的派生对象，不忽略任何业务表、触发器或索引。
fn database_evidence(conn: &Connection) -> Result<String, String> {
    use rusqlite::types::ValueRef;
    let mut checks = conn
        .prepare("PRAGMA integrity_check")
        .map_err(|_| invalid())?;
    let results = checks
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|_| invalid())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid())?;
    if results != ["ok"] {
        return Err(invalid());
    }
    if conn
        .prepare("PRAGMA foreign_key_check")
        .map_err(|_| invalid())?
        .query([])
        .map_err(|_| invalid())?
        .next()
        .map_err(|_| invalid())?
        .is_some()
    {
        return Err(invalid());
    }
    let derived = Connection::open_in_memory().map_err(|_| invalid())?;
    derived
        .execute_batch(crate::db::schema::CREATE_KB_FTS)
        .map_err(|_| invalid())?;
    fn schema(conn: &Connection) -> Result<Vec<(String, String, String, Option<String>)>, String> {
        conn.prepare("SELECT type, name, tbl_name, sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name")
            .map_err(|_| invalid())?.query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
            .map_err(|_| invalid())?.collect::<Result<_, _>>().map_err(|_| invalid())
    }
    let allowed = schema(&derived)?;
    let mut digest = Sha256::new();
    let actual = schema(conn)?;
    let mut tables = Vec::new();
    for object in actual {
        if let Some(expected) = allowed.iter().find(|entry| entry.1 == object.1) {
            if expected != &object {
                return Err(invalid());
            }
            continue;
        }
        if object.0 == "table" {
            tables.push(object.1.clone());
        }
        let bytes = serde_json::to_vec(&object).map_err(|_| invalid())?;
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    tables.sort();
    for table in tables {
        let quoted = format!("\"{}\"", table.replace('"', "\"\""));
        let mut stmt = conn
            .prepare(&format!("SELECT rowid,* FROM {quoted} ORDER BY rowid"))
            .map_err(|_| invalid())?;
        let width = stmt.column_count();
        let mut rows = stmt.query([]).map_err(|_| invalid())?;
        while let Some(row) = rows.next().map_err(|_| invalid())? {
            digest.update((width as u64).to_le_bytes());
            for column in 0..width {
                match row.get_ref(column).map_err(|_| invalid())? {
                    ValueRef::Null => digest.update([0]),
                    ValueRef::Integer(value) => {
                        digest.update([1]);
                        digest.update(value.to_le_bytes());
                    }
                    ValueRef::Real(value) => {
                        digest.update([2]);
                        digest.update(value.to_bits().to_le_bytes());
                    }
                    ValueRef::Text(value) | ValueRef::Blob(value) => {
                        digest.update([
                            if matches!(
                                row.get_ref(column).map_err(|_| invalid())?,
                                ValueRef::Text(_)
                            ) {
                                3
                            } else {
                                4
                            },
                        ]);
                        digest.update((value.len() as u64).to_le_bytes());
                        digest.update(value);
                    }
                }
            }
        }
        digest.update(b"end-table");
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
#[path = "portable_project_round_trip_tests.rs"]
mod round_trip_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;
    use crate::services::portable_project_export::{
        export_portable_project, ExportPortableProjectInput,
    };

    const ORIGIN_PROJECT_ID: &str = "123e4567-e89b-12d3-a456-426614174000";
    const TARGET_PROJECT_ID: &str = "987e6543-e21b-42d3-a456-426614174000";
    const CREATED_AT: &str = "2026-10-16T05:00:00.000Z";
    const RESTORE_CREATED_AT: &str = "2026-10-16T06:00:00.000Z";

    fn temp_base(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "lorekeeper-restore-{name}-{}",
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::canonicalize(&base).unwrap()
    }

    /// 建源项目（`.lore` 存储根 + 库 + 原始资产）并导出成便携归档
    fn exported_archive(base: &Path) -> PathBuf {
        let source = base.join("source");
        let storage = source.join(crate::db::PROJECT_DIR_NAME);
        std::fs::create_dir_all(storage.join("prompts")).unwrap();
        let database = storage.join(RESTORE_DATABASE_ENTRY);
        let conn = Connection::open(&database).unwrap();
        create_tables(&conn).unwrap();
        conn.execute("INSERT INTO generation_roots (root_action_id,idempotency_key,action_json,budget_json) VALUES ('old-root','old-key','{}','{}')", []).unwrap();
        conn.execute(
            "INSERT INTO kb_documents (id, file_name, file_path, corpus_kind, imported_at, chunk_count, content_hash)
             VALUES ('doc-1', '参考.md', '', 'reference', '2026-10-16T00:00:00.000Z', 1, 'hash')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO kb_chunks (id, doc_id, file_name, text, chunk_index, total_chunks, imported_at, corpus_kind)
             VALUES ('chunk-1', 'doc-1', '参考.md', 'alpha beta gamma', 0, 1, '2026-10-16T00:00:00.000Z', 'reference')",
            [],
        )
        .unwrap();
        drop(conn);
        std::fs::write(storage.join("prompts").join("a.md"), b"prompt-a").unwrap();

        let source = std::fs::canonicalize(&source).unwrap();
        let staging_parent = base.join("staging");
        std::fs::create_dir_all(&staging_parent).unwrap();
        let staging_parent = std::fs::canonicalize(&staging_parent).unwrap();
        let archive = base.join("book.ainovel");
        export_portable_project(&ExportPortableProjectInput {
            source_project_root: &source,
            origin_project_id: ORIGIN_PROJECT_ID,
            target_archive_path: &archive,
            attempt_parent_path: &staging_parent,
            snapshot_generation: Some("gen-1"),
            created_at: CREATED_AT,
            limits: None,
        })
        .unwrap();
        archive
    }

    fn restore(
        archive: &Path,
        target: &Path,
        target_project_id: Option<&str>,
    ) -> Result<RestorePortableProjectReceipt, String> {
        restore_portable_project(&RestorePortableProjectInput {
            archive_path: archive,
            target_project_root: target,
            created_at: RESTORE_CREATED_AT,
            target_project_id,
            cancel: None,
            limits: None,
        })
    }

    #[test]
    fn restore_installs_archive_and_rebuilds_derived_index_test() {
        let base = temp_base("install");
        let archive = exported_archive(&base);
        let target = base.join("restored");
        let receipt = restore(&archive, &target, Some(TARGET_PROJECT_ID)).unwrap();
        assert_eq!(receipt.origin_project_id, ORIGIN_PROJECT_ID);
        assert_eq!(receipt.target_project_id, TARGET_PROJECT_ID);
        assert_eq!(receipt.snapshot_generation, "gen-1");
        assert_eq!(receipt.rebuilt_derived_tables, vec!["kb_fts".to_string()]);
        assert!(receipt.requires_runtime_freeze_guard);
        assert_eq!(
            receipt.portable_database_sha256.len(),
            64,
            "收据必须携带便携库哈希"
        );

        let storage = target.join(crate::db::PROJECT_DIR_NAME);
        let manifest_bytes = std::fs::read(storage.join(RESTORE_PROJECT_MANIFEST_ENTRY)).unwrap();
        let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
        assert!(
            crate::project_access::is_project_manifest(&manifest),
            "恢复必须重建有效项目清单"
        );
        assert_eq!(
            manifest.get("projectId").and_then(Value::as_str),
            Some(TARGET_PROJECT_ID)
        );
        assert_eq!(
            manifest.get("createdAt").and_then(Value::as_str),
            Some(RESTORE_CREATED_AT)
        );

        let authority_bytes =
            std::fs::read(storage.join(RESTORE_TRANSFER_AUTHORITY_ENTRY)).unwrap();
        let authority =
            parse_portable_transfer_authority(&serde_json::from_slice(&authority_bytes).unwrap())
                .unwrap();
        assert_eq!(authority.origin_project_id, ORIGIN_PROJECT_ID);
        assert_eq!(
            authority.target_project_id.as_deref(),
            Some(TARGET_PROJECT_ID),
            "转移权威必须改写为新项目 id"
        );

        assert_eq!(
            std::fs::read(storage.join("prompts").join("a.md")).unwrap(),
            b"prompt-a"
        );

        let conn = Connection::open(storage.join(RESTORE_DATABASE_ENTRY)).unwrap();
        let text: String = conn
            .query_row("SELECT text FROM kb_chunks LIMIT 1", [], |row| row.get(0))
            .unwrap();
        assert!(text.contains("alpha"), "知识事实必须随库入档：{text}");
        let indexed: i64 = conn
            .query_row("SELECT COUNT(*) FROM kb_fts", [], |row| row.get(0))
            .unwrap();
        assert!(indexed >= 1, "kb_fts 必须按 kb_chunks 重建");
        let hits = crate::db::kb::fts::search(&conn, "alpha", 5).unwrap();
        assert!(!hits.is_empty(), "重建后的 FTS 必须可检索");
        drop(conn);

        // 真正的项目连接必须自动安装冻结门禁；直接仓储 SQL 也不能绕过。
        let project = crate::db::ProjectDatabase::open(&target).unwrap();
        let conn = project.connection();
        let error = conn
            .execute(
                "UPDATE generation_roots SET revision=1 WHERE root_action_id='old-root'",
                [],
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("PORTABLE_RUNTIME_FROZEN"), "{error}");
        conn.execute("INSERT INTO generation_roots (root_action_id,idempotency_key,action_json,budget_json) VALUES ('fresh-root','fresh-key','{}','{}')", []).unwrap();
        drop(project);
        let reopened = crate::db::ProjectDatabase::open(&target).unwrap();
        let error = reopened
            .connection()
            .execute(
                "DELETE FROM generation_roots WHERE root_action_id='old-root'",
                [],
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("PORTABLE_RUNTIME_FROZEN"), "{error}");
        reopened
            .connection()
            .execute(
                "UPDATE generation_roots SET revision=1 WHERE root_action_id='fresh-root'",
                [],
            )
            .unwrap();
        drop(reopened);

        // 目标根只有存储根；安装日志与标记已释放；暂存目录已回收
        assert_eq!(
            directory_names(&target).unwrap(),
            vec![crate::db::PROJECT_DIR_NAME.to_string()]
        );
        let base_names = directory_names(&base).unwrap();
        assert!(
            !base_names
                .iter()
                .any(|name| name.starts_with(".portable-project-attempt-")),
            "暂存目录必须回收：{base_names:?}"
        );
        assert!(
            !base_names
                .iter()
                .any(|name| name.starts_with(".portable-project-restore-")),
            "成功安装后安装日志必须释放：{base_names:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn restore_is_idempotent_for_same_archive_test() {
        let base = temp_base("idempotent");
        let archive = exported_archive(&base);
        let target = base.join("restored");
        let first = restore(&archive, &target, Some(TARGET_PROJECT_ID)).unwrap();
        let second = restore(&archive, &target, Some(TARGET_PROJECT_ID)).unwrap();
        assert_eq!(first.target_project_id, second.target_project_id);
        assert_eq!(second.rebuilt_derived_tables, vec!["kb_fts".to_string()]);
        assert_eq!(
            directory_names(&target).unwrap(),
            vec![crate::db::PROJECT_DIR_NAME.to_string()]
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn restore_rejects_occupied_target_test() {
        let base = temp_base("occupied");
        let archive = exported_archive(&base);

        // 空目录（无日志、无标记）→ TARGET_EXISTS
        let empty = base.join("empty-target");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(
            restore(&archive, &empty, Some(TARGET_PROJECT_ID)).unwrap_err(),
            "PORTABLE_RESTORE_TARGET_EXISTS"
        );

        // 目录内已有陌生内容 → TARGET_EXISTS
        let stray = base.join("stray-target");
        std::fs::create_dir_all(stray.join(crate::db::PROJECT_DIR_NAME)).unwrap();
        std::fs::write(stray.join("stray.txt"), b"x").unwrap();
        assert_eq!(
            restore(&archive, &stray, Some(TARGET_PROJECT_ID)).unwrap_err(),
            "PORTABLE_RESTORE_TARGET_EXISTS"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn restore_rejects_invalid_inputs_test() {
        let base = temp_base("invalid");
        let archive = exported_archive(&base);
        let target = base.join("restored");

        // 目标 id 不是 UUID v4
        assert_eq!(
            restore(&archive, &target, Some("not-a-uuid")).unwrap_err(),
            "PORTABLE_RESTORE_INVALID"
        );
        // 目标 id 等于 origin
        assert_eq!(
            restore(&archive, &target, Some(ORIGIN_PROJECT_ID)).unwrap_err(),
            "PORTABLE_RESTORE_INVALID"
        );
        // 父目录不存在
        let orphan = base.join("missing-parent").join("child");
        assert_eq!(
            restore(&archive, &orphan, Some(TARGET_PROJECT_ID)).unwrap_err(),
            "PORTABLE_RESTORE_UNSAFE_TARGET"
        );
        // 取消信号
        let cancel = std::sync::atomic::AtomicBool::new(true);
        assert_eq!(
            restore_portable_project(&RestorePortableProjectInput {
                archive_path: &archive,
                target_project_root: &target,
                created_at: RESTORE_CREATED_AT,
                target_project_id: Some(TARGET_PROJECT_ID),
                cancel: Some(&cancel),
                limits: None,
            })
            .unwrap_err(),
            "PORTABLE_RESTORE_CANCELLED"
        );
        // 失败后目标不应存在
        assert!(std::fs::symlink_metadata(&target).is_err());
        let base_names = directory_names(&base).unwrap();
        assert!(
            !base_names
                .iter()
                .any(|name| name.starts_with(".portable-project-attempt-")),
            "失败路径同样必须回收暂存目录：{base_names:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn restore_resumes_owned_reservation_and_rejects_marker_substitution() {
        let base = temp_base("resume");
        let archive = exported_archive(&base);
        let target = base.join("restored");
        let extracted = extract_portable_project_archive(&ExtractPortableProjectArchiveInput {
            archive_path: &archive,
            staging_parent_path: &base,
            limits: None,
        })
        .unwrap();
        let authority = parse_portable_transfer_authority(
            &json_value(
                &std::fs::read(
                    extracted
                        .staging_path
                        .join(RESTORE_TRANSFER_AUTHORITY_ENTRY),
                )
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let (journal, hold) = create_install_journal(
            &target,
            &hash_physical_file(&archive).unwrap(),
            &authority,
            TARGET_PROJECT_ID,
        )
        .unwrap();
        std::fs::create_dir(&target).unwrap();
        std::fs::hard_link(&journal.path, target.join(RESTORE_INSTALL_MARKER_NAME)).unwrap();
        drop(hold);
        crate::services::portable_project_archive::cleanup_attempt(&extracted.staging_attempt);
        assert_eq!(
            restore(&archive, &target, None).unwrap().target_project_id,
            TARGET_PROJECT_ID
        );

        let target2 = base.join("replaced-marker");
        let (journal, hold) = create_install_journal(
            &target2,
            &hash_physical_file(&archive).unwrap(),
            &authority,
            TARGET_PROJECT_ID,
        )
        .unwrap();
        std::fs::create_dir(&target2).unwrap();
        // 内容相同但 inode 不同，不得冒充本安装日志的硬链接标记。
        std::fs::copy(&journal.path, target2.join(RESTORE_INSTALL_MARKER_NAME)).unwrap();
        drop(hold);
        assert_eq!(
            restore(&archive, &target2, None).unwrap_err(),
            "PORTABLE_RESTORE_UNSAFE_TARGET"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn restore_rejects_business_tampering_after_derived_index_rebuild() {
        let base = temp_base("tamper");
        let archive = exported_archive(&base);
        let target = base.join("restored");
        restore(&archive, &target, Some(TARGET_PROJECT_ID)).unwrap();
        let conn = Connection::open(
            target
                .join(crate::db::PROJECT_DIR_NAME)
                .join(RESTORE_DATABASE_ENTRY),
        )
        .unwrap();
        conn.execute("UPDATE kb_chunks SET text='changed'", [])
            .unwrap();
        drop(conn);
        assert_eq!(
            restore(&archive, &target, None).unwrap_err(),
            "PORTABLE_RESTORE_TARGET_EXISTS"
        );
        assert_eq!(
            std::fs::read_to_string(
                target
                    .join(crate::db::PROJECT_DIR_NAME)
                    .join("prompts/a.md")
            )
            .unwrap(),
            "prompt-a"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn cleanup_preserves_unowned_files_and_capture_rejects_hard_links() {
        let base = temp_base("ownership");
        let root = base.join("attempt");
        std::fs::create_dir(&root).unwrap();
        let mut attempt = OwnedAttempt {
            root: root.clone(),
            paths: BTreeMap::new(),
        };
        capture_owned(&mut attempt, &root, OwnedKind::Directory).unwrap();
        let file = root.join("owned");
        write_owned_file(&mut attempt, &file, b"owned").unwrap();
        std::fs::write(root.join("stranger"), b"preserve").unwrap();
        let link = root.join("hard-link");
        std::fs::hard_link(&file, &link).unwrap();
        assert_eq!(
            capture_owned(&mut attempt, &link, OwnedKind::File).unwrap_err(),
            "PORTABLE_RESTORE_UNSAFE_TARGET"
        );
        cleanup_attempt(&attempt);
        assert_eq!(std::fs::read(root.join("stranger")).unwrap(), b"preserve");
        assert!(root.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn install_journal_shape_is_enforced_test() {
        let base = temp_base("journal");
        let target = base.join("restored");
        let document = InstallJournal {
            version: INSTALL_JOURNAL_VERSION,
            target_root: literal_path_text(&target),
            archive_sha256: "a".repeat(64),
            origin_project_id: ORIGIN_PROJECT_ID.to_string(),
            snapshot_generation: "gen-1".to_string(),
            transfer_receipt_id: "portable-transfer:abc".to_string(),
            portable_database_sha256: "b".repeat(64),
            target_project_id: TARGET_PROJECT_ID.to_string(),
            reservation_token: "11111111-1111-4111-8111-111111111111".to_string(),
        };
        let bytes = serde_json::to_vec(&document).unwrap();
        assert_eq!(parse_install_journal(&bytes, &target).unwrap(), document);

        // 未知键
        let mut value: Value = serde_json::from_slice(&bytes).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("extra".to_string(), Value::Bool(true));
        assert_eq!(
            parse_install_journal(&serde_json::to_vec(&value).unwrap(), &target).unwrap_err(),
            "PORTABLE_RESTORE_UNSAFE_TARGET"
        );
        // 目标根不符
        let other = base.join("other");
        assert_eq!(
            parse_install_journal(&bytes, &other).unwrap_err(),
            "PORTABLE_RESTORE_UNSAFE_TARGET"
        );
        // 坏哈希与坏 uuid
        let mut broken = document.clone();
        broken.archive_sha256 = "zz".repeat(32);
        assert_eq!(
            parse_install_journal(&serde_json::to_vec(&broken).unwrap(), &target).unwrap_err(),
            "PORTABLE_RESTORE_UNSAFE_TARGET"
        );
        let mut broken = document.clone();
        broken.reservation_token = "not-a-uuid".to_string();
        assert_eq!(
            parse_install_journal(&serde_json::to_vec(&broken).unwrap(), &target).unwrap_err(),
            "PORTABLE_RESTORE_UNSAFE_TARGET"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
