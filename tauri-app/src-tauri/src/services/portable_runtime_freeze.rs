//! 批次 G4-1：便携运行时冻结（自 `electron/services/portable-runtime-freeze.ts` 平移）。
//!
//! 上游事实源：`electron/services/portable-runtime-freeze.ts`（160 行）。
//! 语义铁律：错误码逐字一致（`PORTABLE_RUNTIME_FREEZE_INVALID` /
//! `PORTABLE_RUNTIME_FROZEN`）；路径拼接对齐 Node `path.join`/`path.resolve`
//! 语义；JSON 字段名 camelCase。
//!
//! # 与上游的实现层偏差（行为等价）
//! - Node `fs.openSync`/`fstatSync`/`lstatSync` 的 fd 竞态防护舞蹈，
//!   用 `std::fs::File` + `symlink_metadata` + Win32 `GetFileInformationByHandle`
//!   （Windows）/ `MetadataExt`（Unix）实现同一组校验
//!   （nlink===1、dev/ino 一致、非符号链接、realpath 稳定、读后复检）。
//! - `toLocaleLowerCase('en-US')` → `str::to_lowercase()`（仅土耳其语等
//!   特殊 locale 规则有差异，与 `import::parsing::normalized_location` 同款偏差）。
//! - `Number.isSafeInteger` 对 JSON `1.0` 的宽松接受由 `is_exact_number` 保持。
//!
//! 纯逻辑、无 Tauri 依赖；数据库无关。

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use serde_json::{Map, Value};

/// 权威侧车字节上限（上游 `AUTHORITY_MAX_BYTES = 4 * 1024 * 1024`，模块私有）。
const AUTHORITY_MAX_BYTES: u64 = 4 * 1024 * 1024;
/// 冻结历史侧车字节上限（上游 `PORTABLE_RUNTIME_HISTORY_MAX_BYTES`，导出常量）。
pub const PORTABLE_RUNTIME_HISTORY_MAX_BYTES: usize = 16 * 1024 * 1024;
/// 记录数上限（上游 `MAX_RECORDS = 100_000`，模块私有）。
const MAX_RECORDS: usize = 100_000;
pub(crate) const FREEZE_FILE: &str = "portable-runtime-freeze.json";
pub(crate) const AUTHORITY_FILE: &str = "portable-transfer-authority.json";

static ID: LazyLock<regex::Regex> = LazyLock::new(|| {
    // 上游模式逐字不变；`\p{L}\p{N}` 的 Unicode 类编译产物超 regex crate 默认
    // 10MB size_limit（JavaScript 无此限制），仅放宽编译上限。
    regex::RegexBuilder::new(r"^[\p{L}\p{N}._:@+-]{1,512}$")
        .size_limit(1 << 30)
        .build()
        .expect("portable id regex is valid")
});
static TABLE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^[a-z][a-z0-9_]{0,127}$").expect("portable table regex is valid")
});
static HASH: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^[a-f0-9]{64}$").expect("portable hash regex is valid"));
static UUID: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
    )
    .expect("portable uuid regex is valid")
});

/// 冻结表名联合（上游 `PortableRuntimeFreezeTable`）。
///
/// 注意：上游运行时只用 `TABLE` 正则校验 `table` 字段（不限制为下述
/// 八个值之一），故解析时按正则接受任意合法表名并存入冻结键集；
/// 本枚举为类型化查询面（对齐上游 TS 类型系统对 `isFrozen` 参数的约束）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortableRuntimeFreezeTable {
    Drafts,
    ChapterDeletionOperations,
    FinalizationOutbox,
    ImportRuns,
    RecoveryCandidates,
    GenerationRoots,
    GenerationRuns,
    GenerationAttempts,
}

impl PortableRuntimeFreezeTable {
    /// 横线命名字符串（对齐上游联合类型的字面量值）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Drafts => "drafts",
            Self::ChapterDeletionOperations => "chapter_deletion_operations",
            Self::FinalizationOutbox => "finalization_outbox",
            Self::ImportRuns => "import_runs",
            Self::RecoveryCandidates => "recovery_candidates",
            Self::GenerationRoots => "generation_roots",
            Self::GenerationRuns => "generation_runs",
            Self::GenerationAttempts => "generation_attempts",
        }
    }
}

/// 冻结错误（上游 `PortableRuntimeFreezeError`，`code` 逐字一致）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortableRuntimeFreezeError {
    /// `PORTABLE_RUNTIME_FREEZE_INVALID`
    Invalid,
    /// `PORTABLE_RUNTIME_FROZEN`
    Frozen,
}

impl PortableRuntimeFreezeError {
    /// 上游错误码字符串（逐字一致）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid => "PORTABLE_RUNTIME_FREEZE_INVALID",
            Self::Frozen => "PORTABLE_RUNTIME_FROZEN",
        }
    }
}

impl std::fmt::Display for PortableRuntimeFreezeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for PortableRuntimeFreezeError {}

/// 确定性替换竞态测试缝（上游 `__testHooks?: { afterOpen?(file) }`，`@internal`）。
pub struct FreezeTestHooks<'a> {
    /// 打开文件后、首次 fstat 前调用（用于模拟打开后替换）。
    pub after_open: Option<&'a dyn Fn(&str)>,
}

/// 运行时冻结守卫（上游 `PortableRuntimeFreezeGuard`）。
///
/// `active === false` 表示普通项目（无标记），一切可变。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableRuntimeFreezeGuard {
    active: bool,
    /// 冻结键集：`(table, recordId)`，对齐上游 `` `${table}\0${recordId}` ``。
    frozen: HashSet<(String, String)>,
}

impl PortableRuntimeFreezeGuard {
    /// 上游 `active` 只读属性。
    pub fn active(&self) -> bool {
        self.active
    }

    /// 上游 `isFrozen(table, recordId)`。
    pub fn is_frozen(&self, table: PortableRuntimeFreezeTable, record_id: &str) -> bool {
        self.frozen
            .contains(&(table.as_str().to_owned(), record_id.to_owned()))
    }

    /// 上游 `assertMutable(table, recordId)`；冻结时抛 `PORTABLE_RUNTIME_FROZEN`。
    pub fn assert_mutable(
        &self,
        table: PortableRuntimeFreezeTable,
        record_id: &str,
    ) -> Result<(), PortableRuntimeFreezeError> {
        if self.is_frozen(table, record_id) {
            Err(PortableRuntimeFreezeError::Frozen)
        } else {
            Ok(())
        }
    }
}

fn inactive() -> PortableRuntimeFreezeGuard {
    PortableRuntimeFreezeGuard {
        active: false,
        frozen: HashSet::new(),
    }
}

/// Node `path.resolve(value)` 的词法等价（不访问文件系统）。
pub(crate) fn resolve_lexical(value: &str) -> PathBuf {
    let mut resolved = if Path::new(value).is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir().unwrap_or_default()
    };
    for component in Path::new(value).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other.as_os_str()),
        }
    }
    resolved
}

/// 上游 `normalized(value)`：resolve + 去 `\\?\` 前缀 + Windows 小写化。
fn normalized(value: &str) -> String {
    let resolved = resolve_lexical(value);
    let mut text = resolved.to_string_lossy().into_owned();
    if let Some(stripped) = text.strip_prefix(r"\\?\") {
        text = stripped.to_owned();
    }
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

/// 上游 `fs.realpathSync.native`。
fn realpath_native(file: &str) -> Result<String, PortableRuntimeFreezeError> {
    fs::canonicalize(file)
        .map(|resolved| resolved.to_string_lossy().into_owned())
        .map_err(|_| PortableRuntimeFreezeError::Invalid)
}

/// 上游 `physicalDirectory(directory)`：逐段 lstat，拒绝符号链接与非物理路径。
fn physical_directory(directory: &str) -> Result<(), PortableRuntimeFreezeError> {
    let absolute = resolve_lexical(directory);
    let mut cursor = PathBuf::new();
    let mut seen_root = false;
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => {
                cursor.push(prefix.as_os_str());
                seen_root = true;
            }
            Component::RootDir => {
                cursor.push("/");
                seen_root = true;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                cursor.pop();
            }
            Component::Normal(segment) => {
                if !seen_root {
                    return Err(PortableRuntimeFreezeError::Invalid);
                }
                cursor.push(segment);
                let info = fs::symlink_metadata(&cursor)
                    .map_err(|_| PortableRuntimeFreezeError::Invalid)?;
                let canonical =
                    fs::canonicalize(&cursor).map_err(|_| PortableRuntimeFreezeError::Invalid)?;
                if !info.is_dir()
                    || info.file_type().is_symlink()
                    || normalized(&canonical.to_string_lossy())
                        != normalized(&cursor.to_string_lossy())
                {
                    return Err(PortableRuntimeFreezeError::Invalid);
                }
            }
        }
    }
    Ok(())
}

/// 文件身份（上游 `fs.BigIntStats` 的 `dev` / `ino` / `nlink` / `size`
/// 与 `isFile` / `isSymbolicLink` 的可比较投影）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
    nlink: u64,
    size: u64,
    is_file: bool,
    is_symlink: bool,
}

/// 上游 `sameFile(left, right)`：`dev` 与 `ino` 均相等。
#[cfg(unix)]
fn file_identity_from_file(file: &File) -> Result<FileIdentity, PortableRuntimeFreezeError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file
        .metadata()
        .map_err(|_| PortableRuntimeFreezeError::Invalid)?;
    Ok(FileIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
        nlink: metadata.nlink(),
        size: metadata.len(),
        is_file: metadata.is_file(),
        is_symlink: metadata.file_type().is_symlink(),
    })
}

#[cfg(windows)]
fn file_identity_from_file(file: &File) -> Result<FileIdentity, PortableRuntimeFreezeError> {
    let metadata = file
        .metadata()
        .map_err(|_| PortableRuntimeFreezeError::Invalid)?;
    // std 的 `MetadataExt::file_index/volume_serial_number` 仍属 unstable
    // （`windows_by_handle`，rustc 1.99 实测 E0658），与 G1 批次
    // `import::parsing::platform_file_identity` 同款：直调 Win32。
    let (dev, ino, nlink) =
        windows_handle_identity(file).ok_or(PortableRuntimeFreezeError::Invalid)?;
    Ok(FileIdentity {
        dev,
        ino,
        nlink,
        size: metadata.len(),
        is_file: metadata.is_file(),
        is_symlink: metadata.file_type().is_symlink(),
    })
}

#[cfg(windows)]
fn windows_handle_identity(file: &File) -> Option<(u64, u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `file` 持有有效句柄，`information` 为栈上可写结构；
    // 调用失败时 Win32 返回 0（不做任何写越界/悬垂访问）。
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) };
    if succeeded == 0 {
        return None;
    }
    let index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Some((
        u64::from(information.dwVolumeSerialNumber),
        index,
        u64::from(information.nNumberOfLinks),
    ))
}

#[cfg(unix)]
fn file_identity_from_path(path: &Path) -> Result<FileIdentity, PortableRuntimeFreezeError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path).map_err(|_| PortableRuntimeFreezeError::Invalid)?;
    Ok(FileIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
        nlink: metadata.nlink(),
        size: metadata.len(),
        is_file: metadata.is_file(),
        is_symlink: metadata.file_type().is_symlink(),
    })
}

#[cfg(windows)]
fn file_identity_from_path(path: &Path) -> Result<FileIdentity, PortableRuntimeFreezeError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| PortableRuntimeFreezeError::Invalid)?;
    // 已确认非符号链接后经打开句柄取卷序列号/文件索引/硬链接数，
    // 与 lstat 的 dev/ino/nlink 语义等价（std 稳定 API 不提供后者）。
    let file = File::open(path).map_err(|_| PortableRuntimeFreezeError::Invalid)?;
    let (dev, ino, nlink) =
        windows_handle_identity(&file).ok_or(PortableRuntimeFreezeError::Invalid)?;
    Ok(FileIdentity {
        dev,
        ino,
        nlink,
        size: metadata.len(),
        is_file: metadata.is_file(),
        is_symlink: metadata.file_type().is_symlink(),
    })
}

/// 上游 `regularFile(file, maxBytes, afterOpen?)`：带 TOCTOU 防护的常规文件读取。
/// `ENOENT` → `Ok(None)`（上游返回 `null`）；其余任何异常 → `INVALID`。
pub(crate) fn regular_file(
    file: &str,
    max_bytes: u64,
    after_open: Option<&dyn Fn(&str)>,
) -> Result<Option<Vec<u8>>, PortableRuntimeFreezeError> {
    let descriptor = match File::open(file) {
        Ok(descriptor) => descriptor,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(PortableRuntimeFreezeError::Invalid),
    };
    let opened = file_identity_from_file(&descriptor)?;
    if let Some(hook) = after_open {
        hook(file);
    }
    let current = file_identity_from_path(Path::new(file))?;
    if !opened.is_file
        || opened.nlink != 1
        || opened.size > max_bytes
        || !current.is_file
        || current.is_symlink
        || current.nlink != 1
        || opened.dev != current.dev
        || opened.ino != current.ino
        || normalized(&realpath_native(file)?) != normalized(file)
    {
        return Err(PortableRuntimeFreezeError::Invalid);
    }
    let mut bytes = Vec::new();
    (&descriptor)
        .read_to_end(&mut bytes)
        .map_err(|_| PortableRuntimeFreezeError::Invalid)?;
    let final_stats = file_identity_from_path(Path::new(file))?;
    let final_opened = file_identity_from_file(&descriptor)?;
    if bytes.len() as u64 != opened.size
        || opened != final_opened
        || opened != final_stats
        || final_stats.is_symlink
        || normalized(&realpath_native(file)?) != normalized(file)
    {
        return Err(PortableRuntimeFreezeError::Invalid);
    }
    Ok(Some(bytes))
}

/// 上游 `record(value)`：单条冻结记录的精确键集校验。
fn validate_record(value: &Value) -> Result<(String, String), PortableRuntimeFreezeError> {
    const KEYS: [&str; 9] = [
        "projectionId",
        "table",
        "recordId",
        "terminalState",
        "projection",
        "projectionHash",
        "excludedFields",
        "nonReplayable",
        "originalReceiptVerified",
    ];
    let item = exact_keys(value, &KEYS)?;
    let is_identifier = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .is_some_and(|text| ID.is_match(text))
    };
    if !is_identifier(item.get("projectionId"))
        || !item
            .get("table")
            .and_then(Value::as_str)
            .is_some_and(|table| TABLE.is_match(table))
        || !is_identifier(item.get("recordId"))
        || !is_identifier(item.get("terminalState"))
        || !matches!(item.get("projection"), Some(Value::Object(_)))
        || !item
            .get("projectionHash")
            .and_then(Value::as_str)
            .is_some_and(|hash| HASH.is_match(hash))
    {
        return Err(PortableRuntimeFreezeError::Invalid);
    }
    match item.get("excludedFields") {
        Some(Value::Array(fields)) => {
            if !fields
                .iter()
                .all(|field| field.as_str().is_some_and(|text| ID.is_match(text)))
            {
                return Err(PortableRuntimeFreezeError::Invalid);
            }
        }
        _ => return Err(PortableRuntimeFreezeError::Invalid),
    }
    if item.get("nonReplayable").and_then(Value::as_bool) != Some(true)
        || item.get("originalReceiptVerified").and_then(Value::as_bool) != Some(false)
    {
        return Err(PortableRuntimeFreezeError::Invalid);
    }
    let table = item
        .get("table")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let record_id = item
        .get("recordId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Ok((table.to_owned(), record_id.to_owned()))
}

/// 上游 `readPortableRuntimeFreeze(projectRoot, __testHooks?)`：
/// 读取项目信任边界的运行时历史侧车。无标记 = 普通项目（非激活守卫）。
pub fn read_portable_runtime_freeze(
    project_root: Option<&str>,
    test_hooks: Option<&FreezeTestHooks<'_>>,
) -> Result<PortableRuntimeFreezeGuard, PortableRuntimeFreezeError> {
    let Some(project_root) = project_root else {
        return Ok(inactive());
    };
    // 本项目口径（L3 双栈隔离）：存储根是 .lore（db::PROJECT_DIR_NAME），不是上游的
    // .ai-novel（CANONICAL_PROJECT_DIRECTORY 只作便携归档内的目录约定保留）。
    let storage_root = resolve_lexical(project_root).join(crate::db::PROJECT_DIR_NAME);
    match fs::symlink_metadata(&storage_root) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(inactive()),
        Err(_) => return Err(PortableRuntimeFreezeError::Invalid),
    }
    physical_directory(&storage_root.to_string_lossy())?;
    let after_open = test_hooks.and_then(|hooks| hooks.after_open);
    let authority = regular_file(
        &storage_root.join(AUTHORITY_FILE).to_string_lossy(),
        AUTHORITY_MAX_BYTES,
        after_open,
    )?;
    let bytes = regular_file(
        &storage_root.join(FREEZE_FILE).to_string_lossy(),
        PORTABLE_RUNTIME_HISTORY_MAX_BYTES as u64,
        after_open,
    )?;
    if authority.is_none() && bytes.is_none() {
        return Ok(inactive());
    }
    let bytes = bytes.ok_or(PortableRuntimeFreezeError::Invalid)?;
    parse_portable_runtime_freeze(&bytes)
}

/// 上游 `parsePortableRuntimeFreeze(bytes)`：解析冻结历史并构建激活守卫。
pub fn parse_portable_runtime_freeze(
    bytes: &[u8],
) -> Result<PortableRuntimeFreezeGuard, PortableRuntimeFreezeError> {
    if bytes.len() > PORTABLE_RUNTIME_HISTORY_MAX_BYTES {
        return Err(PortableRuntimeFreezeError::Invalid);
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| PortableRuntimeFreezeError::Invalid)?;
    const KEYS: [&str; 7] = [
        "version",
        "originProjectId",
        "snapshotGeneration",
        "nonReplayable",
        "requiresRuntimeFreezeGuard",
        "records",
        "avatarReferenceProjections",
    ];
    let document = exact_keys(&value, &KEYS)?;
    if !is_exact_number(document.get("version"), 1)
        || !document
            .get("originProjectId")
            .and_then(Value::as_str)
            .is_some_and(|id| UUID.is_match(id))
        || !document
            .get("snapshotGeneration")
            .and_then(Value::as_str)
            .is_some_and(|id| ID.is_match(id))
        || document.get("nonReplayable").and_then(Value::as_bool) != Some(true)
        || document
            .get("requiresRuntimeFreezeGuard")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err(PortableRuntimeFreezeError::Invalid);
    }
    let records = document
        .get("records")
        .and_then(Value::as_array)
        .ok_or(PortableRuntimeFreezeError::Invalid)?;
    let avatar_reference_projections = document
        .get("avatarReferenceProjections")
        .and_then(Value::as_array)
        .ok_or(PortableRuntimeFreezeError::Invalid)?;
    if records.len() > MAX_RECORDS || avatar_reference_projections.len() > MAX_RECORDS {
        return Err(PortableRuntimeFreezeError::Invalid);
    }
    if !avatar_reference_projections
        .iter()
        .all(|item| matches!(item, Value::Object(_)))
    {
        return Err(PortableRuntimeFreezeError::Invalid);
    }
    let mut frozen = HashSet::new();
    for record in records {
        let key = validate_record(record)?;
        if !frozen.insert(key) {
            return Err(PortableRuntimeFreezeError::Invalid);
        }
    }
    Ok(PortableRuntimeFreezeGuard {
        active: true,
        frozen,
    })
}

/// 精确键集校验（上游 `Object.keys(item).length === keys.length && every(hasOwn)`）。
fn exact_keys<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, PortableRuntimeFreezeError> {
    let map = value
        .as_object()
        .ok_or(PortableRuntimeFreezeError::Invalid)?;
    let mut actual: Vec<&str> = map.keys().map(String::as_str).collect();
    actual.sort_unstable();
    let mut expected = keys.to_vec();
    expected.sort_unstable();
    if actual != expected {
        return Err(PortableRuntimeFreezeError::Invalid);
    }
    Ok(map)
}

/// 上游严格相等 `document.version !== 1` 的数值判定
/// （JS 中 `1.0 === 1` 为真，故同时接受无小数部分的浮点表示）。
fn is_exact_number(value: Option<&Value>, expected: i64) -> bool {
    match value {
        Some(Value::Number(number)) => {
            number.as_i64() == Some(expected)
                || number
                    .as_f64()
                    .is_some_and(|float| float == expected as f64 && float.fract() == 0.0)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TempProject {
        base: PathBuf,
    }

    impl TempProject {
        fn new(label: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "lorekeeper-g4-1-freeze-{}-{}-{}",
                label,
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst),
            ));
            let _ = fs::remove_dir_all(&base);
            fs::create_dir_all(&base).expect("temp project root created");
            Self { base }
        }

        fn storage_root(&self) -> PathBuf {
            self.base.join(crate::db::PROJECT_DIR_NAME)
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    fn hash64(character: char) -> String {
        character.to_string().repeat(64)
    }

    fn freeze_record(table: &str, record_id: &str) -> String {
        format!(
            r#"{{"projectionId":"proj.1","table":"{table}","recordId":"{record_id}","terminalState":"term.1","projection":{{}},"projectionHash":"{}","excludedFields":[],"nonReplayable":true,"originalReceiptVerified":false}}"#,
            hash64('a'),
        )
    }

    fn freeze_document(records: &str) -> String {
        format!(
            r#"{{"version":1,"originProjectId":"123e4567-e89b-12d3-a456-426614174000","snapshotGeneration":"gen.1","nonReplayable":true,"requiresRuntimeFreezeGuard":true,"records":{records},"avatarReferenceProjections":[]}}"#,
        )
    }

    #[test]
    fn error_codes_match_upstream() {
        assert_eq!(
            PortableRuntimeFreezeError::Invalid.code(),
            "PORTABLE_RUNTIME_FREEZE_INVALID"
        );
        assert_eq!(
            PortableRuntimeFreezeError::Frozen.code(),
            "PORTABLE_RUNTIME_FROZEN"
        );
        assert_eq!(
            PortableRuntimeFreezeError::Frozen.to_string(),
            "PORTABLE_RUNTIME_FROZEN"
        );
    }

    #[test]
    fn parse_accepts_valid_document_and_builds_guard() {
        let document = freeze_document(&format!(
            "[{},{}]",
            freeze_record("drafts", "draft.1"),
            freeze_record("generation_runs", "run.α"),
        ));
        let guard = parse_portable_runtime_freeze(document.as_bytes()).expect("valid document");
        assert!(guard.active());
        assert!(guard.is_frozen(PortableRuntimeFreezeTable::Drafts, "draft.1"));
        assert!(guard.is_frozen(PortableRuntimeFreezeTable::GenerationRuns, "run.α"));
        assert!(!guard.is_frozen(PortableRuntimeFreezeTable::Drafts, "draft.2"));
        assert_eq!(
            guard.assert_mutable(PortableRuntimeFreezeTable::Drafts, "draft.1"),
            Err(PortableRuntimeFreezeError::Frozen)
        );
        assert_eq!(
            guard.assert_mutable(PortableRuntimeFreezeTable::Drafts, "draft.2"),
            Ok(())
        );
    }

    #[test]
    fn parse_rejects_duplicate_records() {
        let record = freeze_record("drafts", "draft.1");
        let document = freeze_document(&format!("[{record},{record}]"));
        assert_eq!(
            parse_portable_runtime_freeze(document.as_bytes()),
            Err(PortableRuntimeFreezeError::Invalid)
        );
    }

    #[test]
    fn parse_rejects_unknown_keys_and_wrong_version() {
        let extra = format!(
            r#"{{"version":1,"originProjectId":"123e4567-e89b-12d3-a456-426614174000","snapshotGeneration":"gen.1","nonReplayable":true,"requiresRuntimeFreezeGuard":true,"records":[],"avatarReferenceProjections":[],"extra":1}}"#
        );
        assert_eq!(
            parse_portable_runtime_freeze(extra.as_bytes()),
            Err(PortableRuntimeFreezeError::Invalid)
        );
        let wrong_version = freeze_document("[]").replace("\"version\":1", "\"version\":2");
        assert_eq!(
            parse_portable_runtime_freeze(wrong_version.as_bytes()),
            Err(PortableRuntimeFreezeError::Invalid)
        );
        assert_eq!(
            parse_portable_runtime_freeze(b"not json"),
            Err(PortableRuntimeFreezeError::Invalid)
        );
        assert_eq!(
            parse_portable_runtime_freeze(b"[1,2,3]"),
            Err(PortableRuntimeFreezeError::Invalid)
        );
    }

    #[test]
    fn parse_rejects_malformed_record_fields() {
        let bad_table = freeze_record("Drafts", "draft.1");
        assert_eq!(
            parse_portable_runtime_freeze(freeze_document(&format!("[{bad_table}]")).as_bytes()),
            Err(PortableRuntimeFreezeError::Invalid)
        );
        let bad_hash = freeze_record("drafts", "draft.1").replace(&hash64('a'), &hash64('b')[..63]);
        assert_eq!(
            parse_portable_runtime_freeze(freeze_document(&format!("[{bad_hash}]")).as_bytes()),
            Err(PortableRuntimeFreezeError::Invalid)
        );
        let mutable_record = freeze_record("drafts", "draft.1")
            .replace("\"nonReplayable\":true", "\"nonReplayable\":false");
        assert_eq!(
            parse_portable_runtime_freeze(
                freeze_document(&format!("[{mutable_record}]")).as_bytes()
            ),
            Err(PortableRuntimeFreezeError::Invalid)
        );
    }

    #[test]
    fn read_returns_inactive_without_storage_root() {
        let guard = read_portable_runtime_freeze(None, None).expect("null root");
        assert!(!guard.active());
        assert!(!guard.is_frozen(PortableRuntimeFreezeTable::Drafts, "draft.1"));
        assert_eq!(
            guard.assert_mutable(PortableRuntimeFreezeTable::Drafts, "draft.1"),
            Ok(())
        );

        let project = TempProject::new("no-storage");
        let guard = read_portable_runtime_freeze(Some(&project.base.to_string_lossy()), None)
            .expect("missing .lore is a normal project");
        assert!(!guard.active());
    }

    #[test]
    fn read_activates_guard_from_freeze_file() {
        let project = TempProject::new("active");
        fs::create_dir_all(project.storage_root()).expect("storage root created");
        fs::write(
            project.storage_root().join(FREEZE_FILE),
            freeze_document(&format!("[{}]", freeze_record("drafts", "draft.1"))),
        )
        .expect("freeze file written");
        let guard = read_portable_runtime_freeze(Some(&project.base.to_string_lossy()), None)
            .expect("valid freeze sidecar");
        assert!(guard.active());
        assert!(guard.is_frozen(PortableRuntimeFreezeTable::Drafts, "draft.1"));
    }

    #[test]
    fn read_is_inactive_when_both_sidecars_absent() {
        let project = TempProject::new("empty-storage");
        fs::create_dir_all(project.storage_root()).expect("storage root created");
        let guard = read_portable_runtime_freeze(Some(&project.base.to_string_lossy()), None)
            .expect("empty storage root is a normal project");
        assert!(!guard.active());
    }

    #[test]
    fn read_rejects_missing_freeze_file_when_authority_present() {
        let project = TempProject::new("authority-only");
        fs::create_dir_all(project.storage_root()).expect("storage root created");
        fs::write(
            project.storage_root().join(AUTHORITY_FILE),
            b"{\"version\":1}",
        )
        .expect("authority file written");
        assert_eq!(
            read_portable_runtime_freeze(Some(&project.base.to_string_lossy()), None),
            Err(PortableRuntimeFreezeError::Invalid)
        );
    }

    #[test]
    fn read_rejects_non_directory_storage_root() {
        let project = TempProject::new("file-storage");
        fs::write(project.storage_root(), b"not a directory").expect("file written");
        assert_eq!(
            read_portable_runtime_freeze(Some(&project.base.to_string_lossy()), None),
            Err(PortableRuntimeFreezeError::Invalid)
        );
    }

    #[test]
    fn table_enum_round_trips_upstream_literals() {
        assert_eq!(PortableRuntimeFreezeTable::Drafts.as_str(), "drafts");
        assert_eq!(
            PortableRuntimeFreezeTable::ChapterDeletionOperations.as_str(),
            "chapter_deletion_operations"
        );
        assert_eq!(
            PortableRuntimeFreezeTable::FinalizationOutbox.as_str(),
            "finalization_outbox"
        );
        assert_eq!(
            PortableRuntimeFreezeTable::ImportRuns.as_str(),
            "import_runs"
        );
        assert_eq!(
            PortableRuntimeFreezeTable::RecoveryCandidates.as_str(),
            "recovery_candidates"
        );
        assert_eq!(
            PortableRuntimeFreezeTable::GenerationRoots.as_str(),
            "generation_roots"
        );
        assert_eq!(
            PortableRuntimeFreezeTable::GenerationRuns.as_str(),
            "generation_runs"
        );
        assert_eq!(
            PortableRuntimeFreezeTable::GenerationAttempts.as_str(),
            "generation_attempts"
        );
        assert_eq!(
            serde_json::to_value(PortableRuntimeFreezeTable::ChapterDeletionOperations)
                .expect("serializes"),
            serde_json::json!("chapter_deletion_operations")
        );
    }

    #[test]
    fn history_max_bytes_constant_matches_upstream() {
        assert_eq!(PORTABLE_RUNTIME_HISTORY_MAX_BYTES, 16 * 1024 * 1024);
    }
}
