//! 最近项目的能力凭证与离线只读快照。源数据库绝不交给 SQLite 打开。
//! 只读取 Lorekeeper 清单和 db/WAL/SHM；清单或物理文件改变即拒绝本次快照。
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::project_access::{is_project_manifest, project_path_key, random_uuid_v4};
use crate::repositories::project_overview_repository;

const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
const SUFFIXES: [&str; 3] = ["", "-wal", "-shm"];

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Clone, PartialEq)]
struct Manifest {
    root: PathBuf,
    project_id: String,
    hash: String,
}

#[derive(Debug, Clone, PartialEq)]
struct Image {
    suffix: &'static str,
    bytes: Vec<u8>,
    identity: String,
    modified: std::time::SystemTime,
}

fn safe_directory(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_dir() || is_link(&meta) {
        return Err("PROJECT_PEEK_UNSAFE_DIRECTORY".into());
    }
    Ok(())
}

fn is_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

/// 用打开的文件句柄核对硬链接数/身份；不支持的平台拒绝而非跳过检查。
fn identity(file: &File) -> Result<String, String> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // 安全：句柄由 file 持有，输出结构有效；失败或重解析点均拒绝。
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0
            || info.nNumberOfLinks != 1
            || info.dwFileAttributes & 0x400 != 0
        {
            return Err("PROJECT_PEEK_UNSAFE_FILE".into());
        }
        Ok(format!(
            "{}:{}:{}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        if meta.nlink() != 1 {
            return Err("PROJECT_PEEK_UNSAFE_FILE".into());
        }
        Ok(format!("{}:{}", meta.dev(), meta.ino()))
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = file;
        Err("PROJECT_PEEK_UNSUPPORTED_PLATFORM".into())
    }
}

fn read_file(path: &Path, suffix: &'static str) -> Result<Image, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || is_link(&meta) || meta.len() > MAX_FILE_BYTES {
        return Err("PROJECT_PEEK_UNSAFE_FILE".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // 打开重解析点自身而非目标，再由句柄属性检查拒绝。
        options.custom_flags(0x00200000);
    }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    let id = identity(&file)?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    if !before.is_file() || before.len() > MAX_FILE_BYTES {
        return Err("PROJECT_PEEK_UNSAFE_FILE".into());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let after = file.metadata().map_err(|e| e.to_string())?;
    let path_meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if is_link(&path_meta)
        || bytes.len() as u64 != before.len()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return Err("PROJECT_PEEK_SOURCE_CHANGED".into());
    }
    Ok(Image {
        suffix,
        bytes,
        identity: id,
        modified: after.modified().map_err(|e| e.to_string())?,
    })
}

fn manifest(root: &Path) -> Result<Manifest, String> {
    safe_directory(root)?;
    let canonical = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let storage = canonical.join(crate::db::PROJECT_DIR_NAME);
    safe_directory(&storage)?;
    if fs::canonicalize(&storage).map_err(|e| e.to_string())? != storage {
        return Err("PROJECT_PEEK_UNSAFE_DIRECTORY".into());
    }
    let bytes = read_file(&storage.join("project.json"), "")?.bytes;
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if !is_project_manifest(&value) {
        return Err("PROJECT_PEEK_MANIFEST_INVALID".into());
    }
    Ok(Manifest {
        root: canonical,
        project_id: value["projectId"].as_str().unwrap().into(),
        hash: digest(&bytes),
    })
}

fn images(root: &Path) -> Result<Vec<Image>, String> {
    safe_directory(&root.join(crate::db::PROJECT_DIR_NAME))?;
    let db = crate::db::project_database_path(root);
    let with_suffix =
        |suffix| db.with_file_name(format!("{}{suffix}", crate::db::PROJECT_DB_FILE_NAME));
    // 不尝试恢复源库的热回滚日志。
    match fs::symlink_metadata(with_suffix("-journal")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err("PROJECT_PEEK_ROLLBACK_JOURNAL_PRESENT".into()),
    }
    let mut result = Vec::new();
    for suffix in SUFFIXES {
        let path = with_suffix(suffix);
        match fs::symlink_metadata(&path) {
            Ok(_) => result.push(read_file(&path, suffix)?),
            Err(e) if suffix != "" && e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(result)
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self, String> {
        let root =
            std::env::temp_dir().join(format!("lorekeeper-project-peek-{}", random_uuid_v4()));
        #[allow(unused_mut)]
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&root).map_err(|e| e.to_string())?;
        Ok(Self(root))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        // 只删自己创建的固定文件名，不递归遍历目录或跟随符号链接。
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let _ = fs::remove_file(self.0.join(format!("snapshot.db{suffix}")));
        }
        let _ = fs::remove_dir(&self.0);
    }
}

#[derive(Debug, Default)]
pub struct ProjectPeek {
    capabilities: HashMap<String, Manifest>,
}

impl ProjectPeek {
    /// 每次刷新最近列表撤销前一批凭证；能力永不持久化。
    pub fn clear(&mut self) {
        self.capabilities.clear();
    }

    pub fn revoke_path(&mut self, path: &str) {
        let key = project_path_key(path);
        self.capabilities
            .retain(|_, value| project_path_key(&value.root.to_string_lossy()) != key);
    }

    /// 仅由主进程的最近列表调用；不暴露“传任意路径签发”的 IPC。
    pub fn issue(&mut self, path: &str) -> Option<(String, String)> {
        let value = manifest(Path::new(path)).ok()?;
        let db = crate::db::project_database_path(&value.root);
        // 签发不打开数据库、不修复库；完整物理检查在使用时执行。
        let meta = fs::symlink_metadata(db).ok()?;
        if !meta.is_file() || is_link(&meta) {
            return None;
        }
        let id = random_uuid_v4();
        let project_id = value.project_id.clone();
        self.capabilities.insert(id.clone(), value);
        Some((id, project_id))
    }

    pub fn peek(&self, capability: &str) -> Value {
        self.snapshot(capability, || {})
            .unwrap_or_else(|_| json!({"state":"unavailable"}))
    }

    fn snapshot(&self, capability: &str, after_copy: impl FnOnce()) -> Result<Value, String> {
        let expected = self
            .capabilities
            .get(capability)
            .ok_or("PROJECT_PEEK_CAPABILITY_INVALID")?;
        if manifest(&expected.root)? != *expected {
            return Err("PROJECT_PEEK_SOURCE_CHANGED".into());
        }
        let before = images(&expected.root)?;
        let scratch = Scratch::new()?;
        for image in &before {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(scratch.0.join(format!("snapshot.db{}", image.suffix)))
                .map_err(|e| e.to_string())?;
            file.write_all(&image.bytes).map_err(|e| e.to_string())?;
        }
        after_copy();
        if images(&expected.root)? != before || manifest(&expected.root)? != *expected {
            return Err("PROJECT_PEEK_SOURCE_CHANGED".into());
        }
        let revision = digest(&serde_json::to_vec(&json!({
            "manifest":expected.hash,
            "files":before.iter().map(|i| json!([i.suffix,i.bytes.len(),digest(&i.bytes)])).collect::<Vec<_>>()
        })).map_err(|e| e.to_string())?);
        let db = Connection::open_with_flags(
            scratch.0.join("snapshot.db"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")
            .map_err(|e| e.to_string())?;
        let overview =
            project_overview_repository::read(&db, &expected.project_id, false, &revision)?;
        drop(db);
        if manifest(&expected.root)? != *expected {
            return Err("PROJECT_PEEK_SOURCE_CHANGED".into());
        }
        Ok(overview)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;

    struct TempProject {
        root: PathBuf,
    }

    impl TempProject {
        fn new(name: &str) -> (Self, String) {
            let id = random_uuid_v4();
            let root =
                std::env::temp_dir().join(format!("lorekeeper-peek-test-{}", random_uuid_v4()));
            let lore_dir = root.join(crate::db::PROJECT_DIR_NAME);
            fs::create_dir_all(&lore_dir).unwrap();

            let manifest = json!({
                "schemaVersion": 1,
                "kind": "ai-novel-project",
                "projectId": id,
                "projectName": name,
                "createdAt": "2026-10-15T00:00:00.000Z",
                "updatedAt": "2026-10-15T00:00:00.000Z"
            });
            fs::write(
                lore_dir.join("project.json"),
                serde_json::to_vec_pretty(&manifest).unwrap(),
            )
            .unwrap();

            let db_path = lore_dir.join(crate::db::PROJECT_DB_FILE_NAME);
            let conn = Connection::open(&db_path).unwrap();
            create_tables(&conn).unwrap();
            conn.execute(
                "INSERT INTO project_core (id, project_name, total_chapters) VALUES ('main', ?1, 5)",
                [name],
            ).unwrap();
            drop(conn);

            (Self { root }, id)
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn test_issue_and_peek_lifecycle() {
        let (temp, proj_id) = TempProject::new("我的预览小说");
        let mut peek = ProjectPeek::default();
        let path_str = temp.root.to_string_lossy().to_string();

        let issued = peek.issue(&path_str).expect("应当成功签发");
        assert_eq!(issued.1, proj_id);
        let cap_id = issued.0;

        let overview = peek.peek(&cap_id);
        assert_eq!(overview["state"], "ready");
        assert_eq!(overview["projectId"], proj_id);
        assert_eq!(overview["name"], "我的预览小说");

        // 撤销该路径
        peek.revoke_path(&path_str);
        let overview_after_revoke = peek.peek(&cap_id);
        assert_eq!(overview_after_revoke["state"], "unavailable");
    }

    #[test]
    fn test_issue_rejects_missing_or_corrupt_manifest() {
        let mut peek = ProjectPeek::default();
        // 不存在的目录
        assert!(peek.issue("C:/definitely/not/a/real/path").is_none());

        // 目录存在但缺少 project.json
        let empty_dir = std::env::temp_dir().join(format!("lorekeeper-empty-{}", random_uuid_v4()));
        fs::create_dir_all(&empty_dir).unwrap();
        assert!(peek.issue(&empty_dir.to_string_lossy()).is_none());
        let _ = fs::remove_dir_all(&empty_dir);
    }

    #[test]
    fn test_peek_rejects_unknown_capability() {
        let peek = ProjectPeek::default();
        let overview = peek.peek("invalid-uuid-404");
        assert_eq!(overview["state"], "unavailable");
    }

    #[test]
    fn test_peek_rejects_when_database_corrupted() {
        let (temp, _) = TempProject::new("损坏测试");
        let mut peek = ProjectPeek::default();
        let path_str = temp.root.to_string_lossy().to_string();

        let (cap_id, _) = peek.issue(&path_str).unwrap();

        // 破坏数据库文件头
        let db_path = temp
            .root
            .join(crate::db::PROJECT_DIR_NAME)
            .join(crate::db::PROJECT_DB_FILE_NAME);
        fs::write(&db_path, b"not a valid sqlite database header at all").unwrap();

        // 再次 peek 必须失败并返回 unavailable
        let overview = peek.peek(&cap_id);
        assert_eq!(overview["state"], "unavailable");
    }

    #[test]
    fn test_peek_rejects_when_manifest_modified() {
        let (temp, _) = TempProject::new("清单修改测试");
        let mut peek = ProjectPeek::default();
        let path_str = temp.root.to_string_lossy().to_string();

        let (cap_id, _) = peek.issue(&path_str).unwrap();

        // 修改 project.json 中的内容
        let manifest_path = temp
            .root
            .join(crate::db::PROJECT_DIR_NAME)
            .join("project.json");
        let mut manifest_val: Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest_val["projectName"] = json!("篡改后的项目名");
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest_val).unwrap(),
        )
        .unwrap();

        let overview = peek.peek(&cap_id);
        assert_eq!(overview["state"], "unavailable");
    }

    #[test]
    fn test_peek_rejects_when_rollback_journal_present() {
        let (temp, _) = TempProject::new("日志测试");
        let mut peek = ProjectPeek::default();
        let path_str = temp.root.to_string_lossy().to_string();

        let (cap_id, _) = peek.issue(&path_str).unwrap();

        // 创建 hot rollback journal
        let journal_path = temp
            .root
            .join(crate::db::PROJECT_DIR_NAME)
            .join(format!("{}-journal", crate::db::PROJECT_DB_FILE_NAME));
        fs::write(&journal_path, b"dummy journal content").unwrap();

        let overview = peek.peek(&cap_id);
        assert_eq!(overview["state"], "unavailable");

        let _ = fs::remove_file(&journal_path);
    }

    #[test]
    fn test_clear_invalidates_all_capabilities() {
        let (temp1, _) = TempProject::new("项目1");
        let (temp2, _) = TempProject::new("项目2");
        let mut peek = ProjectPeek::default();

        let (cap1, _) = peek.issue(&temp1.root.to_string_lossy()).unwrap();
        let (cap2, _) = peek.issue(&temp2.root.to_string_lossy()).unwrap();

        assert_eq!(peek.peek(&cap1)["state"], "ready");
        assert_eq!(peek.peek(&cap2)["state"], "ready");

        peek.clear();

        assert_eq!(peek.peek(&cap1)["state"], "unavailable");
        assert_eq!(peek.peek(&cap2)["state"], "unavailable");
    }
}
