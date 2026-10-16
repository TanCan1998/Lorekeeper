//! 角色头像资产服务 —— 平移自 `electron/services/character-asset-service.ts`。
//!
//! 契约：`tauri-app/src/shared/character-avatar.ts`（错误码、视图、输入上限 4 MB）。
//! 存储布局：`<项目数据根>/avatars/<sha256(characterId)>/<assetRevision>-<contentHash>.<suffix>`，
//! 与基线逐字段一致；安装走「临时文件 + fsync + 硬链接」，**永不覆盖**既有文件。
//!
//! 安全边界（ADR 0002 同一套思路）：所有读写路径先逐级 `lstat` 拒绝符号链接，
//! 再确认目标落在数据根内；相对路径只来自本模块生成（`avatars/...`）或库中已有记录。
//!
//! 与基线的差异：基线的 `resolveService` 每次请求都会跑一次 `recordReclaimableOrphans`
//! （惰性清理）；本实现的同名函数由命令层在解析作用域时调用，语义与调用频率一致。

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::repositories::character_asset_repository as repo;
use crate::services::avatar_image::{
    avatar_mime, compress_avatar_image, detect_avatar_extension, AvatarImageExtension,
};

/// 基线 `MAX_CHARACTER_AVATAR_INPUT_BYTES`
pub const MAX_CHARACTER_AVATAR_INPUT_BYTES: usize = 4 * 1024 * 1024;
/// 基线 `CHARACTER_ID = /^[a-zA-Z0-9_-]{1,128}$/u`
const CHARACTER_ID_MAX_UNITS: usize = 128;
/// 基线 `read-batch` 的 id 数量上限
const AVATAR_BATCH_LIMIT: usize = 256;
/// 基线的「路径不安全」内部码（不进入契约错误码，命令层折叠为 `AVATAR_SAVE_FAILED`）
const UNSAFE_PATH: &str = "CHARACTER_ASSET_UNSAFE_PATH";
/// 基线 `fs.linkSync` 冲突码（同路径不同内容）
const TARGET_CONFLICT: &str = "CHARACTER_ASSET_TARGET_CONFLICT";

/// 契约 `CharacterAvatarView`
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterAvatarView {
    pub character_id: String,
    pub asset_revision: i64,
    pub mime: String,
    pub base64: String,
}

/// 契约 `CharacterAvatarChooseResponse` 的 `image`（无 `assetRevision`）
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterAvatarPreview {
    pub character_id: String,
    pub mime: String,
    pub base64: String,
}

/// 基线 `CHARACTER_ID` 校验（字符集是 ASCII，故按字符计数与按 UTF-16 码元计数等价）
pub fn is_valid_character_id(character_id: &str) -> bool {
    let count = character_id.chars().count();
    count >= 1
        && count <= CHARACTER_ID_MAX_UNITS
        && character_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
}

/// 基线 `hash`：SHA-256 小写 hex
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// 基线 `hash(JSON.stringify([...]))`：用于稳定生成未决记录 id
fn json_hash(parts: &[&str]) -> String {
    let payload = serde_json::to_string(parts).unwrap_or_else(|_| "[]".to_string());
    sha256_hex(payload.as_bytes())
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 标准 base64 编码（对齐 Node `Buffer.toString('base64')`）
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(BASE64_ALPHABET[(first >> 2) as usize] as char);
        output.push(BASE64_ALPHABET[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            output.push(BASE64_ALPHABET[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(BASE64_ALPHABET[(third & 0x3f) as usize] as char);
        } else {
            output.push('=');
        }
    }
    output
}

/// 基线 `validBase64`：非空、长度 4 的倍数、字符集合法、最多两处尾随 `=`
pub fn is_valid_base64(value: &str) -> bool {
    if value.is_empty() || value.len() % 4 != 0 {
        return false;
    }
    let bytes = value.as_bytes();
    let padding = bytes.iter().rev().take_while(|byte| **byte == b'=').count();
    if padding > 2 {
        return false;
    }
    bytes[..bytes.len() - padding]
        .iter()
        .all(|byte| BASE64_ALPHABET.contains(byte))
}

/// 标准 base64 解码（输入必须已通过 [`is_valid_base64`]）
pub fn base64_decode(value: &str) -> Result<Vec<u8>, String> {
    if !is_valid_base64(value) {
        return Err("IMAGE_FORMAT_INVALID".to_string());
    }
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len() / 4 * 3);
    let mut index = 0usize;
    while index < bytes.len() {
        let mut block = [0u32; 4];
        let mut filled = 0usize;
        for offset in 0..4 {
            let byte = bytes[index + offset];
            if byte == b'=' {
                break;
            }
            let Some(value) = BASE64_ALPHABET.iter().position(|entry| *entry == byte) else {
                return Err("IMAGE_FORMAT_INVALID".to_string());
            };
            block[offset] = value as u32;
            filled = offset + 1;
        }
        let combined = (block[0] << 18) | (block[1] << 12) | (block[2] << 6) | block[3];
        output.push((combined >> 16) as u8);
        if filled > 2 {
            output.push((combined >> 8) as u8);
        }
        if filled > 3 {
            output.push(combined as u8);
        }
        index += 4;
    }

    Ok(output)
}

/// 目标类型期望（基线 `assertSafePath` 的 `expected`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpectedKind {
    File,
    Directory,
}

/// 基线 `assertSafePath`：逐级 `lstat`，拒绝符号链接与类型不符；缺失视为可继续
fn assert_safe_path(location: &Path, expected: ExpectedKind) -> Result<(), String> {
    let components: Vec<Component> = location.components().collect();
    let root_len = components
        .iter()
        .take_while(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
        .count();
    let mut cursor = PathBuf::from_iter(
        components[..root_len]
            .iter()
            .map(|component| component.as_os_str()),
    );
    for (index, component) in components[root_len..].iter().enumerate() {
        cursor.push(component.as_os_str());
        let last = root_len + index + 1 == components.len();
        match std::fs::symlink_metadata(&cursor) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(UNSAFE_PATH.to_string());
                }
                if !last && !metadata.is_dir() {
                    return Err(UNSAFE_PATH.to_string());
                }
                if last {
                    let wrong = match expected {
                        ExpectedKind::File => !metadata.is_file(),
                        ExpectedKind::Directory => !metadata.is_dir(),
                    };
                    if wrong {
                        return Err(UNSAFE_PATH.to_string());
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

/// 基线 `contained`：候选必须落在根内（分量级比较，不做大小写折叠）
fn contained(root: &Path, candidate: &Path) -> bool {
    candidate.starts_with(root)
}

/// 基线 `targetPath`：拼相对路径 → 范围校验 → 父目录安全校验
fn target_path(root: &Path, relative_path: &str) -> Result<PathBuf, String> {
    let mut target = root.to_path_buf();
    for part in relative_path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(UNSAFE_PATH.to_string());
        }
        target.push(part);
    }
    if !contained(root, &target) {
        return Err(UNSAFE_PATH.to_string());
    }
    if let Some(parent) = target.parent() {
        assert_safe_path(parent, ExpectedKind::Directory)?;
    }
    Ok(target)
}

/// 基线 `installFile`：已存在且同内容视为幂等成功；否则临时文件 + 硬链接原子安装
fn install_file(root: &Path, relative_path: &str, bytes: &[u8]) -> Result<bool, String> {
    let target = target_path(root, relative_path)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        assert_safe_path(parent, ExpectedKind::Directory)?;
    }
    if target.exists() {
        assert_safe_path(&target, ExpectedKind::File)?;
        let existing = std::fs::read(&target).map_err(|error| error.to_string())?;
        if sha256_hex(&existing) != sha256_hex(bytes) {
            return Err(TARGET_CONFLICT.to_string());
        }
        return Ok(false);
    }
    let file_name = target
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| UNSAFE_PATH.to_string())?;
    let temporary = target.with_file_name(format!(
        "{file_name}.{}.tmp",
        crate::project_access::random_uuid_v4()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    let written = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| error.to_string());
    drop(file);
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    let linked = std::fs::hard_link(&temporary, &target);
    let _ = std::fs::remove_file(&temporary);
    linked.map_err(|error| error.to_string())?;
    Ok(true)
}

/// 基线 `service.hasCharacter`
pub fn has_character(conn: &Connection, character_id: &str) -> Result<bool, String> {
    if !is_valid_character_id(character_id) {
        return Ok(false);
    }
    repo::character_exists(conn, character_id)
}

/// 基线 `reclaim`：未被引用且删除成功即结束；否则登记为 `orphan` 未决记录
fn reclaim(
    conn: &Connection,
    data_root: &Path,
    record: &repo::CharacterAssetRecord,
    source_reference: &str,
) -> Result<(), String> {
    if repo::referenced_paths(conn)?.contains(&record.relative_path) {
        return Ok(());
    }
    if let Ok(target) = target_path(data_root, &record.relative_path) {
        if std::fs::remove_file(target).is_ok() {
            return Ok(());
        }
    }
    let record_id = json_hash(&[
        source_reference,
        &record.relative_path,
        &record.content_hash,
    ]);
    let candidate_ids =
        serde_json::to_string(&[record.character_id.as_str()]).unwrap_or_else(|_| "[]".to_string());
    repo::insert_unresolved_orphan(
        conn,
        &record_id,
        source_reference,
        &record.relative_path,
        &record.content_hash,
        &record.mime,
        record.byte_size,
        &candidate_ids,
    )
}

/// 基线 `previewFile`：只读预览所选文件，不落库
pub fn preview_file(
    conn: &Connection,
    character_id: &str,
    file_path: &Path,
) -> Result<CharacterAvatarPreview, String> {
    if !has_character(conn, character_id)? {
        return Err("CHARACTER_NOT_FOUND".to_string());
    }
    assert_safe_path(file_path, ExpectedKind::File)?;
    let before = std::fs::symlink_metadata(file_path).map_err(|error| error.to_string())?;
    if !before.is_file() || before.file_type().is_symlink() || before.len() == 0 {
        return Err("IMAGE_READ_FAILED".to_string());
    }
    if before.len() > MAX_CHARACTER_AVATAR_INPUT_BYTES as u64 {
        return Err("IMAGE_TOO_LARGE".to_string());
    }
    let mut file = std::fs::File::open(file_path).map_err(|error| error.to_string())?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    if !opened.is_file() || opened.len() != before.len() {
        return Err("IMAGE_READ_FAILED".to_string());
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|_| "IMAGE_READ_FAILED".to_string())?;
    if bytes.len() as u64 != before.len() {
        return Err("IMAGE_READ_FAILED".to_string());
    }
    let extension =
        detect_avatar_extension(&bytes).ok_or_else(|| "IMAGE_FORMAT_INVALID".to_string())?;
    Ok(CharacterAvatarPreview {
        character_id: character_id.to_string(),
        mime: avatar_mime(extension).to_string(),
        base64: base64_encode(&bytes),
    })
}

/// 基线 `commit`：压缩 → 安装文件 → 事务内推进绑定行 → 回收旧资产
pub fn commit(
    conn: &Connection,
    data_root: &Path,
    character_id: &str,
    base64: &str,
) -> Result<CharacterAvatarView, String> {
    if !has_character(conn, character_id)? {
        return Err("CHARACTER_NOT_FOUND".to_string());
    }
    if !is_valid_base64(base64) {
        return Err("IMAGE_FORMAT_INVALID".to_string());
    }
    if base64.len() > (MAX_CHARACTER_AVATAR_INPUT_BYTES * 4).div_ceil(3) + 4 {
        return Err("IMAGE_TOO_LARGE".to_string());
    }
    let original = base64_decode(base64)?;
    if original.is_empty() {
        return Err("IMAGE_FORMAT_INVALID".to_string());
    }
    if original.len() > MAX_CHARACTER_AVATAR_INPUT_BYTES {
        return Err("IMAGE_TOO_LARGE".to_string());
    }
    let extension =
        detect_avatar_extension(&original).ok_or_else(|| "IMAGE_FORMAT_INVALID".to_string())?;
    let encoded = compress_avatar_image(&original, extension);
    let encoded_extension = detect_avatar_extension(&encoded.bytes)
        .ok_or_else(|| "IMAGE_FORMAT_INVALID".to_string())?;
    let content_hash = sha256_hex(&encoded.bytes);
    let asset_revision = repo::next_revision(conn, character_id)?;
    let suffix = match encoded_extension {
        AvatarImageExtension::Jpeg => "jpg",
        other => other.as_str(),
    };
    let relative_path = format!(
        "avatars/{}/{}-{}.{}",
        sha256_hex(character_id.as_bytes()),
        asset_revision,
        content_hash,
        suffix
    );
    let installed = install_file(data_root, &relative_path, &encoded.bytes)?;
    let record = repo::CharacterAssetRecord {
        character_id: character_id.to_string(),
        asset_revision,
        relative_path: relative_path.clone(),
        content_hash,
        mime: avatar_mime(encoded_extension).to_string(),
        byte_size: encoded.bytes.len() as i64,
        source_reference: String::new(),
    };
    let previous = match repo::replace(conn, &record) {
        Ok(previous) => previous,
        Err(error) => {
            if installed {
                if let Ok(target) = target_path(data_root, &relative_path) {
                    let _ = std::fs::remove_file(target);
                }
            }
            return Err(error);
        }
    };
    if let Some(previous) = &previous {
        if previous.relative_path != relative_path {
            reclaim(
                conn,
                data_root,
                previous,
                &format!("runtime-replace:{character_id}"),
            )?;
        }
    }
    Ok(CharacterAvatarView {
        character_id: character_id.to_string(),
        asset_revision,
        mime: avatar_mime(encoded_extension).to_string(),
        base64: base64_encode(&encoded.bytes),
    })
}

/// 基线 `readMany`：id 校验与去重后逐条读取；缺失 / 不安全 / 哈希不符一律跳过
pub fn read_many(
    conn: &Connection,
    data_root: &Path,
    character_ids: &[String],
) -> Result<Vec<CharacterAvatarView>, String> {
    let mut ids: Vec<String> = Vec::new();
    for character_id in character_ids {
        if !is_valid_character_id(character_id) {
            return Err("INVALID_CHARACTER_ID".to_string());
        }
        if !ids.contains(character_id) {
            ids.push(character_id.clone());
        }
    }
    if ids.len() > AVATAR_BATCH_LIMIT {
        return Err("INVALID_CHARACTER_ID".to_string());
    }
    let mut views = Vec::new();
    for record in repo::read_many(conn, &ids)? {
        let Ok(file) = target_path(data_root, &record.relative_path) else {
            continue;
        };
        if assert_safe_path(&file, ExpectedKind::File).is_err() {
            continue;
        }
        let Ok(bytes) = std::fs::read(&file) else {
            continue;
        };
        let Some(extension) = detect_avatar_extension(&bytes) else {
            continue;
        };
        if bytes.len() as i64 != record.byte_size || sha256_hex(&bytes) != record.content_hash {
            continue;
        }
        views.push(CharacterAvatarView {
            character_id: record.character_id,
            asset_revision: record.asset_revision,
            mime: avatar_mime(extension).to_string(),
            base64: base64_encode(&bytes),
        });
    }
    Ok(views)
}

/// 基线 `remove`
pub fn remove(conn: &Connection, data_root: &Path, character_id: &str) -> Result<(), String> {
    if !is_valid_character_id(character_id) {
        return Err("INVALID_CHARACTER_ID".to_string());
    }
    let previous = repo::remove(conn, character_id)?;
    if let Some(previous) = &previous {
        reclaim(
            conn,
            data_root,
            previous,
            &format!("runtime-remove:{character_id}"),
        )?;
    }
    Ok(())
}

/// 基线 `recordReclaimableOrphans`：把崩溃遗留的未引用文件登记为可恢复孤儿（绝不删除）
pub fn record_reclaimable_orphans(conn: &Connection, data_root: &Path) -> Result<usize, String> {
    let avatar_root = data_root.join("avatars");
    if !avatar_root.exists() {
        return Ok(0);
    }
    assert_safe_path(&avatar_root, ExpectedKind::Directory)?;
    let referenced = repo::referenced_paths(conn)?;
    let mut files = Vec::new();
    collect_files(&avatar_root, &mut files)?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let mut recorded = 0usize;
    for file in files {
        let relative_path = file
            .strip_prefix(data_root)
            .map_err(|_| UNSAFE_PATH.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        if referenced.contains(&relative_path) {
            continue;
        }
        let bytes = std::fs::read(&file).map_err(|error| error.to_string())?;
        let content_hash = sha256_hex(&bytes);
        let extension = detect_avatar_extension(&bytes);
        let record_id = json_hash(&["runtime-crash", &relative_path, &content_hash]);
        repo::insert_unresolved_orphan(
            &tx,
            &record_id,
            "runtime-crash",
            &relative_path,
            &content_hash,
            extension
                .map(avatar_mime)
                .unwrap_or("application/octet-stream"),
            bytes.len() as i64,
            "[]",
        )?;
        recorded += 1;
    }
    tx.commit().map_err(|error| error.to_string())?;
    Ok(recorded)
}

/// 递归收集 `avatars/` 下的文件；任一符号链接即视为不安全
fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in std::fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err(UNSAFE_PATH.to_string());
        }
        if metadata.is_dir() {
            collect_files(&path, files)?;
        } else if metadata.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_tables;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().expect("内存库打开失败");
        create_tables(&conn).expect("建表失败");
        conn.execute(
            "INSERT INTO characters(character_id, name) VALUES('char-1', '角色一')",
            [],
        )
        .unwrap();
        conn
    }

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "lorekeeper-avatar-{name}-{}",
            crate::project_access::random_uuid_v4()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// 最小 PNG：魔数即可通过格式检测（解码失败时压缩函数原样返回）
    fn png_bytes(marker: u8) -> Vec<u8> {
        let mut bytes = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        bytes.extend_from_slice(&[marker; 32]);
        bytes
    }

    #[test]
    fn base64_round_trip_and_validation_test() {
        for sample in [vec![], vec![0u8], vec![1, 2], vec![1, 2, 3], vec![255; 7]] {
            let encoded = base64_encode(&sample);
            if sample.is_empty() {
                assert_eq!(encoded, "");
                assert!(
                    !is_valid_base64(&encoded),
                    "空串不是合法 base64（对齐基线 validBase64）"
                );
                continue;
            }
            assert_eq!(base64_decode(&encoded).unwrap(), sample);
            assert!(is_valid_base64(&encoded));
        }
        assert_eq!(base64_encode(b"a"), "YQ==");
        assert_eq!(base64_encode(b"ab"), "YWI=");
        assert_eq!(base64_encode(b"abc"), "YWJj");
        assert!(!is_valid_base64(""));
        assert!(!is_valid_base64("YWJ"));
        assert!(!is_valid_base64("YWJj="));
        assert!(!is_valid_base64("YW?j"));
        assert!(!is_valid_base64("=YWJ"));
    }

    #[test]
    fn commit_installs_file_and_reclaims_previous_revision_test() {
        let conn = memory_db();
        let root = temp_root("commit");
        let first = commit(&conn, &root, "char-1", &base64_encode(&png_bytes(1))).unwrap();
        assert_eq!(first.asset_revision, 1);
        let expected_dir = root.join("avatars").join(sha256_hex(b"char-1"));
        let entries: Vec<String> = std::fs::read_dir(&expected_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].starts_with("1-"));
        assert!(entries[0].ends_with(".png"));

        let second = commit(&conn, &root, "char-1", &base64_encode(&png_bytes(2))).unwrap();
        assert_eq!(second.asset_revision, 2);
        let entries: Vec<String> = std::fs::read_dir(&expected_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries.len(), 1, "旧版本未被引用时即回收");
        assert!(entries[0].starts_with("2-"));

        let removed = read_many(&conn, &root, &["char-1".to_string()]).unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].base64, second.base64);

        assert_eq!(
            commit(&conn, &root, "char-1", "not-base64").unwrap_err(),
            "IMAGE_FORMAT_INVALID"
        );
        assert_eq!(
            commit(&conn, &root, "char-missing", &base64_encode(&png_bytes(3))).unwrap_err(),
            "CHARACTER_NOT_FOUND"
        );
        assert_eq!(
            commit(&conn, &root, "char-1", &base64_encode(b"plain text")).unwrap_err(),
            "IMAGE_FORMAT_INVALID"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_many_skips_tampered_and_remove_reclaims_test() {
        let conn = memory_db();
        let root = temp_root("read");
        commit(&conn, &root, "char-1", &base64_encode(&png_bytes(4))).unwrap();
        let avatar_dir = root.join("avatars").join(sha256_hex(b"char-1"));
        let file = std::fs::read_dir(&avatar_dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        std::fs::write(&file, png_bytes(9)).unwrap();
        assert!(
            read_many(&conn, &root, &["char-1".to_string()])
                .unwrap()
                .is_empty(),
            "内容哈希不符的头像必须回落初始头像"
        );
        assert_eq!(
            read_many(&conn, &root, &["bad id!".to_string()]).unwrap_err(),
            "INVALID_CHARACTER_ID"
        );

        std::fs::write(&file, png_bytes(5)).unwrap();
        let record = repo::read(&conn, "char-1").unwrap().unwrap();
        let digest = sha256_hex(&png_bytes(5));
        commit(&conn, &root, "char-1", &base64_encode(&png_bytes(5))).unwrap();
        assert_ne!(record.content_hash, digest, "第二次提交生成独立资产文件");

        remove(&conn, &root, "char-1").unwrap();
        assert!(repo::read(&conn, "char-1").unwrap().is_none());
        assert_eq!(
            remove(&conn, &root, "bad id!").unwrap_err(),
            "INVALID_CHARACTER_ID"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn preview_and_reclaimable_orphans_test() {
        let conn = memory_db();
        let root = temp_root("preview");
        let source = root.join("source.png");
        std::fs::write(&source, png_bytes(6)).unwrap();
        let preview = preview_file(&conn, "char-1", &source).unwrap();
        assert_eq!(preview.mime, "image/png");
        assert_eq!(base64_decode(&preview.base64).unwrap(), png_bytes(6));
        assert_eq!(
            preview_file(&conn, "char-1", &root.join("missing.png"))
                .unwrap_err()
                .is_empty(),
            false
        );
        let text = root.join("note.txt");
        std::fs::write(&text, b"plain").unwrap();
        assert_eq!(
            preview_file(&conn, "char-1", &text).unwrap_err(),
            "IMAGE_FORMAT_INVALID"
        );
        assert_eq!(
            preview_file(&conn, "char-missing", &source).unwrap_err(),
            "CHARACTER_NOT_FOUND"
        );
        assert_eq!(
            record_reclaimable_orphans(&conn, &root).unwrap(),
            0,
            "尚未创建 avatars 目录时无事可做"
        );

        // 崩溃遗留：avatars 下的未引用文件被登记为孤儿，且不删除
        let orphan_dir = root.join("avatars").join("crash");
        std::fs::create_dir_all(&orphan_dir).unwrap();
        let orphan = orphan_dir.join("9-dead.png");
        std::fs::write(&orphan, png_bytes(7)).unwrap();
        assert_eq!(record_reclaimable_orphans(&conn, &root).unwrap(), 1);
        assert!(orphan.exists(), "登记孤儿不得删除文件");
        assert_eq!(
            record_reclaimable_orphans(&conn, &root).unwrap(),
            0,
            "已登记为未决记录的文件不再重复登记（对齐基线 referencedPaths 跳过）"
        );
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM character_avatar_unresolved",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
