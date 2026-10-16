//! 便携项目归档容器读写层 —— 平移自 `electron/services/portable-project-archive.ts`。
//!
//! 本模块分两步落地：**本切片（B111b-1）交付写入面**（[`write_portable_project_archive`]）
//! 与共用底座（文件身份复核 / 安全目录 / 暂存目录所有权模型）；读取面
//! （`extract_portable_project_archive`）在 B111b-2 落地。
//!
//! # 与基线的实现口径（用户确认 2026-10-16）
//!
//! - ZIP 容器由 `zip` crate 承担：写入恒用 `Stored` + `unix_permissions(0o600)` + 自动 Zip64，
//!   条目顺序与 manifest 一致，`manifest.json` 恒为首条目；
//! - 写出归档的**字节序列不与基线自研 writer 逐字节相同**（基线恒置 UTF-8 标志位与 data
//!   descriptor；`zip` 仅在名称含非 ASCII 时置 UTF-8 标志、可 seek 时回填头字段）。Tauri 侧
//!   读取将兼容两种来源；Electron 侧读取器要求 UTF-8 标志位，故对纯 ASCII 条目名的 Tauri 归档
//!   会拒绝 —— 该差异在 B111b-2 读取面一并记录到快照；
//! - 安全约束逐条保留：目标**只**以「临时文件 + 无覆盖硬链接」发布；源文件在读取前后共四次
//!   身份复核（`dev`/`ino` + `size` + `mtime`/`ctime`）；暂存路径所有权记录；逐级符号链接拒绝；
//!   路径键冲突与限额校验；
//! - Windows 上目录身份经 `FILE_FLAG_BACKUP_SEMANTICS` 打开目录句柄后取（复用
//!   `GetFileInformationByHandle`），与 `portable_runtime_freeze` / `import::parsing` 同款做法。

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::services::portable_project_format::{
    parse_portable_project_manifest, portable_path_key, PortableArchiveLimits,
    PortableProjectManifest, PortableProjectManifestEntry,
};

/// manifest 条目名（对齐基线 `MANIFEST_PATH`）
pub const MANIFEST_PATH: &str = "manifest.json";
/// manifest 内容上限（对齐基线 `PORTABLE_MANIFEST_MAX_BYTES` = 16 MiB）
pub const PORTABLE_MANIFEST_MAX_BYTES: u64 = 16 * 1024 * 1024;
/// 复制块大小（对齐基线 `COPY_CHUNK_BYTES` = 1 MiB）
const COPY_CHUNK_BYTES: usize = 1024 * 1024;
/// 暂存目录前缀（对齐基线 `.portable-project-attempt-`）
const STAGING_PREFIX: &str = ".portable-project-attempt-";
/// 归档临时文件前缀（对齐基线 `.portable-project-archive-`）
const TEMPORARY_PREFIX: &str = ".portable-project-archive-";

fn archive_invalid() -> String {
    "PORTABLE_ARCHIVE_INVALID".to_string()
}

fn source_changed() -> String {
    "PORTABLE_ARCHIVE_SOURCE_CHANGED".to_string()
}

fn source_unsafe() -> String {
    "PORTABLE_ARCHIVE_SOURCE_UNSAFE".to_string()
}

fn target_exists() -> String {
    "PORTABLE_ARCHIVE_TARGET_EXISTS".to_string()
}

fn publish_unsupported() -> String {
    "PORTABLE_ARCHIVE_PUBLISH_UNSUPPORTED".to_string()
}

fn limit_exceeded() -> String {
    "PORTABLE_ARCHIVE_LIMIT_EXCEEDED".to_string()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// 上游 `PortableArchiveSource`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableArchiveSource {
    pub entry: PortableProjectManifestEntry,
    pub source_path: PathBuf,
}

/// 上游 `PortableArchiveReceipt`
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableArchiveReceipt {
    pub entry_count: usize,
    pub payload_bytes: u64,
    pub archive_bytes: u64,
    pub manifest_sha256: String,
}

/// 上游 `WritePortableProjectArchiveInput`
#[derive(Debug, Clone)]
pub struct WritePortableProjectArchiveInput<'a> {
    pub manifest: &'a PortableProjectManifest,
    pub sources: &'a [PortableArchiveSource],
    pub target_path: &'a Path,
    pub limits: Option<PortableArchiveLimits>,
}

/// 文件身份（上游 `fs.BigIntStats` 的可比较投影：`dev` / `ino` / `size` / `mtimeNs` / `ctimeNs`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    pub dev: u64,
    pub ino: u64,
    pub size: u64,
    pub mtime_ns: i64,
    pub ctime_ns: i64,
    pub is_file: bool,
    pub is_dir: bool,
    pub is_symlink: bool,
}

/// 上游 `sameFile`：`dev` / `ino` / `size` / `mtime` / `ctime` 全等
fn same_file(left: &FileIdentity, right: &FileIdentity) -> bool {
    left.dev == right.dev
        && left.ino == right.ino
        && left.size == right.size
        && left.mtime_ns == right.mtime_ns
        && left.ctime_ns == right.ctime_ns
}

/// 上游 `sameObject`：仅 `dev` 与 `ino` 相等
fn same_object(left: &FileIdentity, right: &FileIdentity) -> bool {
    left.dev == right.dev && left.ino == right.ino
}

/// 上游 `normalizeRealPath`：`realpath` 后的比较文本（去 Windows `\\?\` 前缀；Windows 小写化）
pub fn real_path_text(path: &Path) -> String {
    let text = std::fs::canonicalize(path)
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string());
    comparable_path_text(&text)
}

/// 未做 `realpath` 的字面路径比较文本
pub fn literal_path_text(path: &Path) -> String {
    comparable_path_text(&path.to_string_lossy())
}

fn comparable_path_text(text: &str) -> String {
    let stripped = text.strip_prefix(r"\\?\").unwrap_or(text).to_string();
    if cfg!(windows) {
        stripped.to_lowercase()
    } else {
        stripped
    }
}

/// `SystemTime` → 纳秒（不可得时为 0）
fn time_ns(time: std::io::Result<std::time::SystemTime>) -> i64 {
    time.ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_nanos() as i64)
        .unwrap_or(0)
}

#[cfg(unix)]
pub fn path_identity(path: &Path) -> Result<FileIdentity, String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| source_unsafe())?;
    Ok(FileIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
        size: metadata.len(),
        mtime_ns: metadata.mtime_nsec(),
        ctime_ns: metadata.ctime_nsec(),
        is_file: metadata.is_file(),
        is_dir: metadata.is_dir(),
        is_symlink: metadata.file_type().is_symlink(),
    })
}

#[cfg(unix)]
pub fn handle_identity(file: &File) -> Result<FileIdentity, String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().map_err(|_| source_unsafe())?;
    Ok(FileIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
        size: metadata.len(),
        mtime_ns: metadata.mtime_nsec(),
        ctime_ns: metadata.ctime_nsec(),
        is_file: metadata.is_file(),
        is_dir: metadata.is_dir(),
        is_symlink: false,
    })
}

#[cfg(windows)]
fn windows_handle_identity(file: &File) -> Result<(u64, u64), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `file` 持有有效句柄，`information` 为栈上可写结构；
    // 调用失败时 Win32 返回 0（不产生写越界或悬垂访问）。
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) };
    if succeeded == 0 {
        return Err(source_unsafe());
    }
    Ok((
        u64::from(information.dwVolumeSerialNumber),
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    ))
}

#[cfg(windows)]
pub fn handle_identity(file: &File) -> Result<FileIdentity, String> {
    let metadata = file.metadata().map_err(|_| source_unsafe())?;
    let (dev, ino) = windows_handle_identity(file)?;
    Ok(FileIdentity {
        dev,
        ino,
        size: metadata.len(),
        mtime_ns: time_ns(metadata.modified()),
        ctime_ns: time_ns(metadata.created()),
        is_file: metadata.is_file(),
        is_dir: metadata.is_dir(),
        is_symlink: false,
    })
}

#[cfg(windows)]
pub fn path_identity(path: &Path) -> Result<FileIdentity, String> {
    use std::os::windows::fs::OpenOptionsExt;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| source_unsafe())?;
    if metadata.file_type().is_symlink() {
        return Ok(FileIdentity {
            dev: 0,
            ino: 0,
            size: metadata.len(),
            mtime_ns: time_ns(metadata.modified()),
            ctime_ns: time_ns(metadata.created()),
            is_file: false,
            is_dir: false,
            is_symlink: true,
        });
    }
    // 目录需要 `FILE_FLAG_BACKUP_SEMANTICS` 才能取得句柄（与基线 `fs.lstatSync` 的目录身份同源）。
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS);
    let file = options.open(path).map_err(|_| source_unsafe())?;
    let mut identity = handle_identity(&file)?;
    // 路径侧元数据是权威的（符号链接、目录位）；句柄只提供 `dev` / `ino`。
    identity.size = metadata.len();
    identity.mtime_ns = time_ns(metadata.modified());
    identity.ctime_ns = time_ns(metadata.created());
    identity.is_file = metadata.is_file();
    identity.is_dir = metadata.is_dir();
    identity.is_symlink = false;
    Ok(identity)
}

#[cfg(not(any(unix, windows)))]
pub fn path_identity(path: &Path) -> Result<FileIdentity, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| source_unsafe())?;
    Ok(FileIdentity {
        dev: 0,
        ino: 0,
        size: metadata.len(),
        mtime_ns: time_ns(metadata.modified()),
        ctime_ns: time_ns(metadata.created()),
        is_file: metadata.is_file(),
        is_dir: metadata.is_dir(),
        is_symlink: metadata.file_type().is_symlink(),
    })
}

#[cfg(not(any(unix, windows)))]
pub fn handle_identity(file: &File) -> Result<FileIdentity, String> {
    let metadata = file.metadata().map_err(|_| source_unsafe())?;
    Ok(FileIdentity {
        dev: 0,
        ino: 0,
        size: metadata.len(),
        mtime_ns: time_ns(metadata.modified()),
        ctime_ns: time_ns(metadata.created()),
        is_file: metadata.is_file(),
        is_dir: metadata.is_dir(),
        is_symlink: false,
    })
}

/// 绝对化（对齐 `path.resolve`）
fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|root| root.join(path))
        .map_err(|error| error.to_string())
}

/// 上游 `contained`：候选落在根内（含相等）
pub fn contained(root: &Path, candidate: &Path) -> bool {
    candidate.starts_with(root)
}

/// 上游 `assertRegularSource`：常规文件、非符号链接、`realpath` 与字面路径一致
fn assert_regular_source(source_path: &Path) -> Result<FileIdentity, String> {
    let identity = path_identity(source_path).map_err(|_| source_unsafe())?;
    if !identity.is_file
        || identity.is_symlink
        || real_path_text(source_path) != literal_path_text(source_path)
    {
        return Err(source_unsafe());
    }
    Ok(identity)
}

/// 上游 `safeDirectory`：逐级 `lstat`，全部为目录、非符号链接且 `realpath` 与字面路径一致
pub fn safe_directory(directory: &Path) -> Result<(), String> {
    let absolute = absolute(directory)?;
    // 从卷根**之后**开始逐级检查（对齐基线 path.parse(absolute).root 之后的段）：
    // Windows 的 \\?\C: / C: 本身不是可打开的目标，不能作为一级去取身份。
    let components: Vec<Component> = absolute.components().collect();
    let root_len = components
        .iter()
        .take_while(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
        .count();
    let mut cursor = PathBuf::from_iter(
        components[..root_len]
            .iter()
            .map(|component| component.as_os_str()),
    );
    for component in &components[root_len..] {
        cursor.push(component.as_os_str());
        let identity = path_identity(&cursor).map_err(|_| source_unsafe())?;
        if !identity.is_dir
            || identity.is_symlink
            || real_path_text(&cursor) != literal_path_text(&cursor)
        {
            return Err(source_unsafe());
        }
    }
    Ok(())
}

/// 上游 `StagingAttempt`
#[derive(Debug)]
pub struct StagingAttempt {
    pub root: PathBuf,
    directories: HashMap<PathBuf, FileIdentity>,
    created: Vec<(PathBuf, FileIdentity, bool)>,
}

/// 上游 `assertAttemptDirectory`：暂存目录链必须仍是本次尝试创建的那批目录
pub fn assert_attempt_directory(attempt: &StagingAttempt, directory: &Path) -> Result<(), String> {
    if !contained(&attempt.root, directory) {
        return Err(source_unsafe());
    }
    let relative = directory
        .strip_prefix(&attempt.root)
        .map_err(|_| source_unsafe())?;
    let mut cursor = attempt.root.clone();
    for segment in relative.components() {
        cursor.push(segment.as_os_str());
        let expected = attempt.directories.get(&cursor).ok_or_else(source_unsafe)?;
        let actual = path_identity(&cursor).map_err(|_| source_unsafe())?;
        if !actual.is_dir || actual.is_symlink || !same_object(expected, &actual) {
            return Err(source_unsafe());
        }
    }
    let root_expected = attempt
        .directories
        .get(&attempt.root)
        .ok_or_else(source_unsafe)?;
    let root_actual = path_identity(&attempt.root).map_err(|_| source_unsafe())?;
    if !root_actual.is_dir
        || root_actual.is_symlink
        || !same_object(root_expected, &root_actual)
        || real_path_text(&attempt.root) != literal_path_text(&attempt.root)
        || !contained(&attempt.root, directory)
    {
        return Err(source_unsafe());
    }
    Ok(())
}

fn create_private_directory(path: &Path) -> Result<(), String> {
    std::fs::create_dir(path).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// 上游 `prepareStaging`：在（安全的）父目录下随机命名建私有暂存根
pub fn prepare_staging(staging_parent_path: &Path) -> Result<StagingAttempt, String> {
    let parent = absolute(staging_parent_path)?;
    safe_directory(&parent)?;
    let root = parent.join(format!(
        "{STAGING_PREFIX}{}",
        crate::project_access::random_uuid_v4()
    ));
    create_private_directory(&root)?;
    let identity = path_identity(&root)?;
    let attempt = StagingAttempt {
        root: root.clone(),
        directories: HashMap::from([(root.clone(), identity)]),
        created: vec![(root.clone(), identity, true)],
    };
    assert_attempt_directory(&attempt, &root)?;
    Ok(attempt)
}

/// 上游 `targetFor`：按条目路径建父目录并返回目标文件路径（已存在即 `TARGET_EXISTS`）
pub fn target_for(attempt: &mut StagingAttempt, archive_path: &str) -> Result<PathBuf, String> {
    let mut directory = attempt.root.clone();
    let segments: Vec<&str> = archive_path.split('/').collect();
    let parents = segments.len().saturating_sub(1);
    for segment in segments.iter().take(parents) {
        assert_attempt_directory(attempt, &directory)?;
        directory.push(segment);
        if !attempt.directories.contains_key(&directory) {
            match create_private_directory(&directory) {
                Ok(()) => {}
                Err(error) if error.contains("exists") => return Err(target_exists()),
                Err(error) => return Err(error),
            }
            let identity = path_identity(&directory)?;
            attempt.directories.insert(directory.clone(), identity);
            attempt.created.push((directory.clone(), identity, true));
        }
        assert_attempt_directory(attempt, &directory)?;
    }
    let name = segments.last().copied().ok_or_else(archive_invalid)?;
    let mut target = directory;
    target.push(name);
    Ok(target)
}

/// 上游 `cleanupAttempt`：逆序删除仍属本次尝试的文件与目录（身份不符即保留）
pub fn cleanup_attempt(attempt: &StagingAttempt) {
    for (path, identity, is_directory) in attempt.created.iter().rev() {
        let Ok(actual) = path_identity(path) else {
            continue;
        };
        if actual.is_symlink
            || !same_object(identity, &actual)
            || real_path_text(path) != literal_path_text(path)
        {
            continue;
        }
        if *is_directory {
            let _ = std::fs::remove_dir(path);
        } else {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// 上游 `assertOpenedTarget`：句柄与路径指向同一个常规文件且 `realpath` 一致
fn assert_opened_target(target: &Path, file: &File) -> Result<FileIdentity, String> {
    let opened = handle_identity(file)?;
    let actual = path_identity(target)?;
    if !actual.is_file
        || actual.is_symlink
        || !same_file(&opened, &actual)
        || real_path_text(target) != literal_path_text(target)
    {
        return Err(source_unsafe());
    }
    Ok(actual)
}

/// 上游 `writePortableProjectArchive`（B111b-1 写入面）
pub fn write_portable_project_archive(
    input: &WritePortableProjectArchiveInput<'_>,
) -> Result<PortableArchiveReceipt, String> {
    let limits = input.limits.unwrap_or_default();
    let manifest = parse_portable_project_manifest(
        &serde_json::to_value(input.manifest).map_err(|_| archive_invalid())?,
        Some(&limits),
    )?;
    if manifest.declared_compressed_bytes != manifest.declared_uncompressed_bytes
        || input.sources.len() != manifest.entries.len()
    {
        return Err(archive_invalid());
    }
    for entry in &manifest.entries {
        if portable_path_key(&entry.path, limits.max_path_bytes)? == MANIFEST_PATH {
            return Err(archive_invalid());
        }
    }
    let mut source_by_key: HashMap<String, &PortableArchiveSource> = HashMap::new();
    for source in input.sources {
        let key = portable_path_key(&source.entry.path, limits.max_path_bytes)?;
        if source_by_key.insert(key, source).is_some() {
            return Err(archive_invalid());
        }
    }
    let mut ordered: Vec<&PortableArchiveSource> = Vec::with_capacity(manifest.entries.len());
    for entry in &manifest.entries {
        let key = portable_path_key(&entry.path, limits.max_path_bytes)?;
        let Some(source) = source_by_key.get(&key) else {
            return Err(archive_invalid());
        };
        if source.entry != *entry {
            return Err(archive_invalid());
        }
        ordered.push(source);
    }

    let target = absolute(input.target_path)?;
    let target_parent = target.parent().ok_or_else(archive_invalid)?.to_path_buf();
    safe_directory(&target_parent)?;
    if std::fs::symlink_metadata(&target).is_ok() {
        return Err(target_exists());
    }

    let temporary = target_parent.join(format!(
        "{TEMPORARY_PREFIX}{}.tmp",
        crate::project_access::random_uuid_v4()
    ));
    let mut options = OpenOptions::new();
    options.write(true).read(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = match options.open(&temporary) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(target_exists())
        }
        Err(error) => return Err(error.to_string()),
    };

    let outcome = write_archive_body(&file, &manifest, &ordered, &limits, &temporary, &target);
    let cleanup_identity = handle_identity(&file).ok();
    drop(file);
    if let Some(identity) = cleanup_identity {
        if let Ok(actual) = path_identity(&temporary) {
            if actual.is_file
                && !actual.is_symlink
                && same_file(&identity, &actual)
                && real_path_text(&temporary) == literal_path_text(&temporary)
            {
                let _ = std::fs::remove_file(&temporary);
            }
        }
    }
    outcome
}

fn write_archive_body(
    file: &File,
    manifest: &PortableProjectManifest,
    ordered: &[&PortableArchiveSource],
    limits: &PortableArchiveLimits,
    temporary: &Path,
    target: &Path,
) -> Result<PortableArchiveReceipt, String> {
    assert_opened_target(temporary, file)?;
    let manifest_bytes = serde_json::to_vec(manifest).map_err(|_| archive_invalid())?;
    if manifest_bytes.len() as u64 > limits.max_entry_bytes.min(PORTABLE_MANIFEST_MAX_BYTES) {
        return Err(limit_exceeded());
    }
    let manifest_sha256 = sha256_hex(&manifest_bytes);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o600);
    // 写入用独立句柄（`try_clone` 共享文件偏移，串行写入安全）；原始句柄保留给发布前后复核。
    let writer_handle = file.try_clone().map_err(|error| error.to_string())?;
    let mut writer = ZipWriter::new(writer_handle).set_auto_large_file();
    writer
        .start_file(MANIFEST_PATH, options)
        .map_err(|_| archive_invalid())?;
    writer
        .write_all(&manifest_bytes)
        .map_err(|error| error.to_string())?;

    let mut buffer = vec![0u8; COPY_CHUNK_BYTES];
    for source in ordered {
        let before = assert_regular_source(&source.source_path)?;
        if before.size != source.entry.byte_size {
            return Err(source_changed());
        }
        let mut source_file = File::open(&source.source_path).map_err(|_| source_changed())?;
        if !same_file(&before, &handle_identity(&source_file)?) {
            return Err(source_changed());
        }
        writer
            .start_file(source.entry.path.as_str(), options)
            .map_err(|_| archive_invalid())?;
        let mut hasher = Sha256::new();
        let mut copied: u64 = 0;
        while copied < source.entry.byte_size {
            let length = (source.entry.byte_size - copied).min(buffer.len() as u64) as usize;
            let read = source_file
                .read(&mut buffer[..length])
                .map_err(|_| source_changed())?;
            if read == 0 {
                return Err(source_changed());
            }
            hasher.update(&buffer[..read]);
            writer
                .write_all(&buffer[..read])
                .map_err(|error| error.to_string())?;
            copied += read as u64;
        }
        let final_handle = handle_identity(&source_file)?;
        let final_path = assert_regular_source(&source.source_path)?;
        if !same_file(&before, &final_handle)
            || !same_file(&before, &final_path)
            || format!("{:x}", hasher.finalize()) != source.entry.sha256
        {
            return Err(source_changed());
        }
    }
    writer.finish().map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    let publish_identity = assert_opened_target(temporary, file)?;
    match std::fs::hard_link(temporary, target) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(target_exists())
        }
        Err(_) => return Err(publish_unsupported()),
    }
    let published = path_identity(target).map_err(|_| archive_invalid())?;
    if !published.is_file
        || published.is_symlink
        || !same_object(&publish_identity, &published)
        || real_path_text(target) != literal_path_text(target)
    {
        return Err(archive_invalid());
    }
    let archive_bytes = file.metadata().map_err(|error| error.to_string())?.len();
    Ok(PortableArchiveReceipt {
        entry_count: manifest.entries.len(),
        payload_bytes: manifest.declared_uncompressed_bytes,
        archive_bytes,
        manifest_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::portable_project_format::PortableEntryDisposition;
    use std::collections::BTreeMap;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lorekeeper-archive-{name}-{}",
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // safe_directory 要求 realpath 与字面路径一致（基线同一约束），
        // 故测试根目录先规范化（Windows 上 %TEMP% 可能带 8.3 短名）。
        std::fs::canonicalize(&dir).unwrap()
    }

    fn make_manifest(entries: &[(&str, &[u8])]) -> PortableProjectManifest {
        let entries: Vec<PortableProjectManifestEntry> = entries
            .iter()
            .map(|(path, bytes)| PortableProjectManifestEntry {
                path: (*path).to_string(),
                byte_size: bytes.len() as u64,
                sha256: sha256_hex(bytes),
                disposition: PortableEntryDisposition::AuthorContent,
            })
            .collect();
        let total: u64 = entries.iter().map(|entry| entry.byte_size).sum();
        PortableProjectManifest {
            format_version: 1,
            source_schema_version: 7,
            origin_project_id: "project-1".to_string(),
            snapshot_generation: "gen-1".to_string(),
            created_at: "2026-10-16T05:00:00.000Z".to_string(),
            declared_uncompressed_bytes: total,
            declared_compressed_bytes: total,
            entries,
            semantic_counts: BTreeMap::new(),
            omitted_items: Vec::new(),
            transfer_receipt_ids: Vec::new(),
            history_projection_ids: Vec::new(),
        }
    }

    fn case(root: &Path) -> (PortableProjectManifest, Vec<PortableArchiveSource>) {
        let first = b"first payload".to_vec();
        let second = b"second payload".to_vec();
        std::fs::create_dir_all(root.join("novel")).unwrap();
        std::fs::write(root.join("a.txt"), &first).unwrap();
        std::fs::write(root.join("novel/b.json"), &second).unwrap();
        let manifest = make_manifest(&[("a.txt", &first), ("novel/b.json", &second)]);
        let sources = vec![
            PortableArchiveSource {
                entry: manifest.entries[0].clone(),
                source_path: root.join("a.txt"),
            },
            PortableArchiveSource {
                entry: manifest.entries[1].clone(),
                source_path: root.join("novel/b.json"),
            },
        ];
        (manifest, sources)
    }

    #[test]
    fn write_produces_readable_stored_archive_test() {
        let root = temp_dir("write");
        let (manifest, sources) = case(&root);
        let target = root.join("book.ainovel");
        let receipt = write_portable_project_archive(&WritePortableProjectArchiveInput {
            manifest: &manifest,
            sources: &sources,
            target_path: &target,
            limits: None,
        })
        .unwrap();
        assert_eq!(receipt.entry_count, 2);
        assert_eq!(receipt.payload_bytes, 27);
        assert!(receipt.archive_bytes > 0);
        assert_eq!(receipt.manifest_sha256.len(), 64);
        assert!(target.exists(), "发布为真目标文件");
        let temporaries: Vec<String> = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with(".portable-project-archive-"))
            .collect();
        assert!(temporaries.is_empty(), "临时文件必须被清理");

        let mut archive = zip::ZipArchive::new(File::open(&target).unwrap()).unwrap();
        assert_eq!(archive.len(), 3);
        let mut names = Vec::new();
        let mut contents = Vec::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).unwrap();
            assert_eq!(entry.compression(), CompressionMethod::Stored);
            assert!(!entry.encrypted());
            names.push(entry.name().unwrap().to_string());
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            contents.push(bytes);
        }
        assert_eq!(names, vec!["manifest.json", "a.txt", "novel/b.json"]);
        assert_eq!(contents[1], b"first payload");
        assert_eq!(contents[2], b"second payload");
        let parsed: PortableProjectManifest = serde_json::from_slice(&contents[0]).unwrap();
        assert_eq!(parsed, manifest, "manifest 内容往返一致");
        assert_eq!(sha256_hex(&contents[0]), receipt.manifest_sha256);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_rejects_existing_target_and_bad_inputs_test() {
        let root = temp_dir("reject");
        let (manifest, sources) = case(&root);
        let target = root.join("book.ainovel");
        std::fs::write(&target, b"occupied").unwrap();
        assert_eq!(
            write_portable_project_archive(&WritePortableProjectArchiveInput {
                manifest: &manifest,
                sources: &sources,
                target_path: &target,
                limits: None,
            })
            .unwrap_err(),
            "PORTABLE_ARCHIVE_TARGET_EXISTS"
        );
        std::fs::remove_file(&target).unwrap();

        // entries 含 manifest.json → 无效
        let mut collides = manifest.clone();
        collides.entries[0].path = "MANIFEST.JSON".to_string();
        collides.declared_uncompressed_bytes -= collides.entries[0].byte_size;
        collides.entries[0].byte_size = 0;
        assert_eq!(
            write_portable_project_archive(&WritePortableProjectArchiveInput {
                manifest: &collides,
                sources: &sources,
                target_path: &target,
                limits: None,
            })
            .unwrap_err(),
            "PORTABLE_ARCHIVE_INVALID"
        );

        // sources 数量与 manifest 不一致 → 无效
        assert_eq!(
            write_portable_project_archive(&WritePortableProjectArchiveInput {
                manifest: &manifest,
                sources: &sources[..1],
                target_path: &target,
                limits: None,
            })
            .unwrap_err(),
            "PORTABLE_ARCHIVE_INVALID"
        );

        // 声明压缩字节与未压缩字节不一致 → 无效
        let mut drift = manifest.clone();
        drift.declared_compressed_bytes += 1;
        assert_eq!(
            write_portable_project_archive(&WritePortableProjectArchiveInput {
                manifest: &drift,
                sources: &sources,
                target_path: &target,
                limits: None,
            })
            .unwrap_err(),
            "PORTABLE_ARCHIVE_INVALID"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_rejects_source_drift_test() {
        let root = temp_dir("drift");
        let (manifest, sources) = case(&root);
        let target = root.join("book.ainovel");
        // 源文件内容被替换（大小与 sha256 均与 manifest 不符）→ SOURCE_CHANGED
        std::fs::write(root.join("a.txt"), b"tampered payload+").unwrap();
        assert_eq!(
            write_portable_project_archive(&WritePortableProjectArchiveInput {
                manifest: &manifest,
                sources: &sources,
                target_path: &target,
                limits: None,
            })
            .unwrap_err(),
            "PORTABLE_ARCHIVE_SOURCE_CHANGED"
        );
        assert!(!target.exists(), "失败不得留下目标文件");
        // 大小一致但内容不同（同长度）→ sha256 不符 → SOURCE_CHANGED
        std::fs::write(root.join("a.txt"), b"FIRST PAYLOAD").unwrap();
        assert_eq!(
            write_portable_project_archive(&WritePortableProjectArchiveInput {
                manifest: &manifest,
                sources: &sources,
                target_path: &target,
                limits: None,
            })
            .unwrap_err(),
            "PORTABLE_ARCHIVE_SOURCE_CHANGED"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn staging_attempt_owns_created_directories_test() {
        let root = temp_dir("staging");
        let staging_parent = root.join("staging");
        std::fs::create_dir_all(&staging_parent).unwrap();
        let mut attempt = prepare_staging(&staging_parent).unwrap();
        let target = target_for(&mut attempt, "novel/deep/b.json").unwrap();
        assert!(target.starts_with(&attempt.root));
        assert!(attempt.root.join("novel/deep").is_dir());
        assert_attempt_directory(&attempt, &attempt.root.join("novel/deep")).unwrap();
        let staging_root = attempt.root.clone();
        cleanup_attempt(&attempt);
        assert!(!staging_root.exists(), "清理删除本次尝试创建的目录树");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn safe_directory_rejects_symlink_test() {
        use std::os::unix::fs::symlink;
        let root = temp_dir("symlink");
        let real = root.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = root.join("link");
        symlink(&real, &link).unwrap();
        assert!(safe_directory(&real).is_ok());
        assert_eq!(
            safe_directory(&link).unwrap_err(),
            "PORTABLE_ARCHIVE_SOURCE_UNSAFE"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
