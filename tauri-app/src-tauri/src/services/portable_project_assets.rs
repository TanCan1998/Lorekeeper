//! 便携资产枚举 —— 平移自 `electron/services/portable-project-assets.ts` 的**原始资产**部分。
//!
//! 覆盖：项目存储根（本项目为 `.lore/`，上游基线为 `.ai-novel/`）下允许进入归档的原始文件白名单
//! （`prompts` / `skills` / `writing-skills.json` / `partial_arch` / `partial_arch.json` /
//! `chapter_creation_log.json`），以及其余顶层条目的**显式省略清单**校验：
//! 顶层出现清单外的名字一律 `PORTABLE_ASSET_UNSAFE`（fail closed，避免未知文件被静默漏掉）。
//!
//! # 与基线的差异（需在 B112b 收据中沿用）
//!
//! - 基线同一模块还用 **LanceDB**（`lancedb` 目录 + `embedding-spaces.json`）读取向量库快照并投影为
//!   `portable-knowledge-source.json`。Tauri 侧已否决 LanceDB，知识库落在 SQLite
//!   （`kb_documents` / `kb_chunks` / `kb_fts`），**该投影尚未决定口径**，故本切片只交付原始资产面；
//!   `portable-knowledge-source.json` 的生成与恢复接线待确认后另行落地；
//! - 基线的 `nlink !== 1n` 硬链接防护在 Rust 侧按平台取（Unix `nlink`；Windows
//!   `GetFileInformationByHandle().nNumberOfLinks`），与 `portable_runtime_freeze` 同款。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::services::portable_project_format::{
    default_portable_archive_limits, portable_path_key, PortableEntryDisposition,
};

/// 上游 `PORTABLE_KNOWLEDGE_SOURCE_PATH`（本切片只保留常量，投影待确认）
pub const PORTABLE_KNOWLEDGE_SOURCE_PATH: &str = "portable-knowledge-source.json";

/// 上游 `CANONICAL_RAW_PROJECT_ASSETS`：允许进入归档的原始资产顶层名
pub const CANONICAL_RAW_PROJECT_ASSETS: &[&'static str] = &[
    "prompts",
    "skills",
    "writing-skills.json",
    "partial_arch",
    "partial_arch.json",
    "chapter_creation_log.json",
];

/// 已知会被省略的顶层条目（出现即合法，但不入档）。
///
/// **本项目口径**（2026-10-16 用户确认「不与基线互通、按本项目方案存储恢复」）：
/// 清单按 Tauri 实际存储根（`<项目根>/.lore/`）登记 —— 库文件 `lorekeeper.db`（及其 WAL/SHM/journal）、
/// 项目清单 `project.json`、派生向量索引目录 `kb/`（HNSW 快照，`rebuild-stale`）、
/// 头像目录 `avatars/`（作为 `avatar-asset` 条目单独入档，此处只声明其顶层合法性）、
/// 知识副本 `knowledge-copies/`、迁移期文件 `portable-transfer-authority.json` 与
/// `portable-runtime-freeze.json`，以及运行期目录 `cache` / `logs` / `temp` / `trash`。
///
/// 与基线的差异：基线清单以 `project.db` / `lancedb` / `vectors.json` 为名（Electron 存储形态），
/// 本项目不再列举这些名称（Tauri 项目不会出现），新增 `lorekeeper.db*` 与 `kb`。
pub const KNOWN_OMITTED_TOP_LEVEL: &[&'static str] = &[
    "avatars",
    "lorekeeper.db",
    "lorekeeper.db-wal",
    "lorekeeper.db-shm",
    "lorekeeper.db-journal",
    "project.json",
    "kb",
    "portable-transfer-authority.json",
    "portable-runtime-freeze.json",
    PORTABLE_KNOWLEDGE_SOURCE_PATH,
    "knowledge-copies",
    "cache",
    "logs",
    "temp",
    "trash",
];

/// 上游 `PortableProvidedFile`（`bytes` 仅用于 provider 生成的不可变内容）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableProvidedFile {
    pub id: String,
    pub archive_path: String,
    /// 为空表示内容由 provider 生成（不可变），不读源文件
    pub source_path: String,
    pub byte_size: u64,
    pub sha256: String,
    pub disposition: PortableEntryDisposition,
}

fn asset_unsafe() -> String {
    "PORTABLE_ASSET_UNSAFE".to_string()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// 上游 `normalized`：`realpath` 后的比较文本
fn normalized(path: &Path) -> String {
    crate::services::portable_project_archive::real_path_text(path)
}

fn literal(path: &Path) -> String {
    crate::services::portable_project_archive::literal_path_text(path)
}

/// 上游 `contained`（严格：自身不算包含，与 `portable_project_archive::contained` 的等值语义不同）
fn strictly_contained(root: &Path, candidate: &Path) -> bool {
    candidate != root && candidate.starts_with(root)
}

/// 上游 `physicalDirectory`：目录、非符号链接、`realpath` 与字面路径一致、且在根内
fn physical_directory(directory: &Path, root: &Path) -> Result<(), String> {
    let identity = crate::services::portable_project_archive::path_identity(directory)
        .map_err(|_| asset_unsafe())?;
    if !identity.is_dir
        || identity.is_symlink
        || normalized(directory) != literal(directory)
        || (directory != root && !strictly_contained(root, directory))
    {
        return Err(asset_unsafe());
    }
    Ok(())
}

/// 上游 `physicalFile`：常规文件、非符号链接、**硬链接数为 1**、在根内且 `realpath` 在根内
fn physical_file(file: &Path, root: &Path) -> Result<u64, String> {
    let identity = crate::services::portable_project_archive::path_identity(file)
        .map_err(|_| asset_unsafe())?;
    if !identity.is_file || identity.is_symlink || !strictly_contained(root, file) {
        return Err(asset_unsafe());
    }
    let root_real = std::fs::canonicalize(root).map_err(|_| asset_unsafe())?;
    let file_real = std::fs::canonicalize(file).map_err(|_| asset_unsafe())?;
    if !strictly_contained(&root_real, &file_real) {
        return Err(asset_unsafe());
    }
    if crate::services::portable_project_archive::link_count(file).map_err(|_| asset_unsafe())? != 1
    {
        return Err(asset_unsafe());
    }
    Ok(identity.size)
}

/// 上游 `disposition(topLevel)`
fn disposition(top_level: &str) -> PortableEntryDisposition {
    match top_level {
        "prompts" => PortableEntryDisposition::Prompt,
        "skills" | "writing-skills.json" => PortableEntryDisposition::Skill,
        _ => PortableEntryDisposition::AuthorContent,
    }
}

fn sorted_entries(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(directory)
        .map_err(|_| asset_unsafe())?
        .map(|entry| entry.map(|value| value.path()).map_err(|_| asset_unsafe()))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by(|left, right| {
        left.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
            .cmp(
                &right
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default(),
            )
    });
    Ok(entries)
}

/// 上游 `enumerateRawFiles`：白名单内的原始资产逐文件枚举（排序、去重、安全校验）
pub fn enumerate_raw_files(storage_root: &Path) -> Result<Vec<PortableProvidedFile>, String> {
    let root = if storage_root.is_absolute() {
        storage_root.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| asset_unsafe())?
            .join(storage_root)
    };
    physical_directory(&root, &root)?;

    let mut top_keys: HashSet<String> = HashSet::new();
    for entry in sorted_entries(&root)? {
        let name = entry
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .ok_or_else(asset_unsafe)?;
        let identity = crate::services::portable_project_archive::path_identity(&entry)
            .map_err(|_| asset_unsafe())?;
        if identity.is_symlink {
            return Err(asset_unsafe());
        }
        if name == "knowledge-copies" && !identity.is_dir {
            return Err(asset_unsafe());
        }
        let key = portable_path_key(&name, default_portable_archive_limits().max_path_bytes)?;
        if !top_keys.insert(key) {
            return Err(asset_unsafe());
        }
        if !CANONICAL_RAW_PROJECT_ASSETS.contains(&name.as_str())
            && !KNOWN_OMITTED_TOP_LEVEL.contains(&name.as_str())
        {
            return Err(asset_unsafe());
        }
    }

    let mut files: Vec<PortableProvidedFile> = Vec::new();
    let mut keys: HashSet<String> = HashSet::new();

    for name in CANONICAL_RAW_PROJECT_ASSETS {
        let entry = root.join(name);
        if std::fs::symlink_metadata(&entry).is_err() {
            continue;
        }
        let identity = crate::services::portable_project_archive::path_identity(&entry)
            .map_err(|_| asset_unsafe())?;
        let directory_expected = matches!(*name, "prompts" | "skills" | "partial_arch");
        if identity.is_symlink || directory_expected != identity.is_dir {
            return Err(asset_unsafe());
        }
        walk_asset(&root, name, &entry, &mut files, &mut keys)?;
    }
    files.sort_by(|left, right| left.archive_path.cmp(&right.archive_path));
    Ok(files)
}

/// 递归遍历单个白名单条目（目录下钻 / 文件登记），逐层做物理性校验。
fn walk_asset(
    root: &Path,
    top_level: &str,
    entry: &Path,
    files: &mut Vec<PortableProvidedFile>,
    keys: &mut HashSet<String>,
) -> Result<(), String> {
    let identity = crate::services::portable_project_archive::path_identity(entry)
        .map_err(|_| asset_unsafe())?;
    if identity.is_symlink {
        return Err(asset_unsafe());
    }
    if identity.is_dir {
        physical_directory(entry, root)?;
        for child in sorted_entries(entry)? {
            walk_asset(root, top_level, &child, files, keys)?;
        }
        return Ok(());
    }
    let size = physical_file(entry, root)?;
    let archive_path = entry
        .strip_prefix(root)
        .map_err(|_| asset_unsafe())?
        .to_string_lossy()
        .replace('\\', "/");
    let key = portable_path_key(
        &archive_path,
        default_portable_archive_limits().max_path_bytes,
    )?;
    if !keys.insert(key) {
        return Err(asset_unsafe());
    }
    let bytes = std::fs::read(entry).map_err(|_| asset_unsafe())?;
    if bytes.len() as u64 != size {
        return Err(asset_unsafe());
    }
    files.push(PortableProvidedFile {
        id: format!("raw:{archive_path}"),
        archive_path,
        source_path: entry.to_string_lossy().to_string(),
        byte_size: bytes.len() as u64,
        sha256: sha256_hex(&bytes),
        disposition: disposition(top_level),
    });
    Ok(())
}

/// 上游 `sameFiles`：两次枚举结果逐字段相等（用于快照后的「源未变」复核）
pub fn same_files(left: &[PortableProvidedFile], right: &[PortableProvidedFile]) -> bool {
    left == right
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::PROJECT_DIR_NAME;

    fn temp_root(name: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "lorekeeper-assets-{name}-{}",
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let storage = base.join("project").join(PROJECT_DIR_NAME);
        std::fs::create_dir_all(&storage).unwrap();
        (
            std::fs::canonicalize(&base).unwrap(),
            std::fs::canonicalize(&storage).unwrap(),
        )
    }

    #[test]
    fn enumerate_returns_sorted_allowlisted_files_test() {
        let (base, storage) = temp_root("enumerate");
        std::fs::create_dir_all(storage.join("prompts")).unwrap();
        std::fs::create_dir_all(storage.join("skills").join("nested")).unwrap();
        std::fs::write(storage.join("prompts").join("a.md"), b"prompt-a").unwrap();
        std::fs::write(storage.join("prompts").join("b.md"), b"prompt-b").unwrap();
        std::fs::write(
            storage.join("skills").join("nested").join("c.md"),
            b"skill-c",
        )
        .unwrap();
        std::fs::write(storage.join("writing-skills.json"), b"{}").unwrap();
        std::fs::write(storage.join("chapter_creation_log.json"), b"[]").unwrap();
        // 已知省略项：合法但不入档
        std::fs::create_dir_all(storage.join("cache")).unwrap();
        std::fs::write(storage.join("cache").join("junk.bin"), b"x").unwrap();
        std::fs::write(storage.join("lorekeeper.db"), b"db").unwrap();

        let files = enumerate_raw_files(&storage).unwrap();
        let paths: Vec<&str> = files
            .iter()
            .map(|file| file.archive_path.as_str())
            .collect();
        assert_eq!(
            paths,
            vec![
                "chapter_creation_log.json",
                "prompts/a.md",
                "prompts/b.md",
                "skills/nested/c.md",
                "writing-skills.json",
            ]
        );
        assert_eq!(files[1].disposition, PortableEntryDisposition::Prompt);
        assert_eq!(files[3].disposition, PortableEntryDisposition::Skill);
        assert_eq!(
            files[0].disposition,
            PortableEntryDisposition::AuthorContent
        );
        assert_eq!(files[1].sha256, sha256_hex(b"prompt-a"));
        assert_eq!(files[1].byte_size, 8);
        assert!(files[1].id.starts_with("raw:"));
        assert!(same_files(&files, &enumerate_raw_files(&storage).unwrap()));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn enumerate_rejects_unknown_top_level_and_wrong_kind_test() {
        let (base, storage) = temp_root("reject");
        std::fs::write(storage.join("mystery.bin"), b"x").unwrap();
        assert_eq!(
            enumerate_raw_files(&storage).unwrap_err(),
            "PORTABLE_ASSET_UNSAFE"
        );
        std::fs::remove_file(storage.join("mystery.bin")).unwrap();

        // prompts 必须是目录
        std::fs::write(storage.join("prompts"), b"not-a-dir").unwrap();
        assert_eq!(
            enumerate_raw_files(&storage).unwrap_err(),
            "PORTABLE_ASSET_UNSAFE"
        );
        std::fs::remove_file(storage.join("prompts")).unwrap();

        // knowledge-copies 必须是目录
        std::fs::write(storage.join("knowledge-copies"), b"not-a-dir").unwrap();
        assert_eq!(
            enumerate_raw_files(&storage).unwrap_err(),
            "PORTABLE_ASSET_UNSAFE"
        );
        std::fs::remove_file(storage.join("knowledge-copies")).unwrap();
        assert!(enumerate_raw_files(&storage).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn enumerate_rejects_symlink_and_hardlink_test() {
        use std::os::unix::fs::{hard_link, symlink};
        let (base, storage) = temp_root("links");
        std::fs::create_dir_all(storage.join("prompts")).unwrap();
        let outside = base.join("outside.md");
        std::fs::write(&outside, b"outside").unwrap();
        symlink(&outside, storage.join("prompts").join("link.md")).unwrap();
        assert_eq!(
            enumerate_raw_files(&storage).unwrap_err(),
            "PORTABLE_ASSET_UNSAFE"
        );
        std::fs::remove_file(storage.join("prompts").join("link.md")).unwrap();

        // 硬链接（nlink > 1）同样拒绝
        let original = storage.join("prompts").join("a.md");
        std::fs::write(&original, b"content").unwrap();
        hard_link(&original, storage.join("prompts").join("b.md")).unwrap();
        assert_eq!(
            enumerate_raw_files(&storage).unwrap_err(),
            "PORTABLE_ASSET_UNSAFE"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
