//! 批次 B：项目文件系统命令 —— 迁移自 `electron/controllers/fs-controller.ts`（基础 7 频道）。
//!
//! 行为对齐要点：
//! - 统一守卫：`project_session` 尾参必填（渲染层自动注入）→ 会话租约校验 →
//!   `expectedProjectPath` 匹配 → 词法包含 + realpath 边界（security 模块）；
//! - 文件互斥：骨架阶段全局串行锁（基线为 per-path mutex 队列；安全语义不变，
//!   仅并发度收窄，per-path 优化批次 C 后评估）；
//! - 原子写：同目录临时文件 + rename 提交，`commitState` 两态
//!   （基线第三态 `unknown` 保留结构，等 atomicWriteFailureCommitState 细化）；
//! - 错误不跨 IPC 泄露内部路径/原始 IO 错误（`fs_error_message` 统一映射）；
//! - `fs:list-dir` 失败走 invoke reject（不伪装空目录），其余命令结构化失败。

use serde::Serialize;
use tauri::State;

use crate::commands::SimpleResult;
use crate::security::{
    assert_current_project_context, assert_project_file_path, guard_message, GuardKind,
    PathCheckMode, ProjectSessionContext,
};
use crate::state::AppState;

/// 迁移自契约 `FileWriteCommitState`。
/// 骨架阶段不产生 `Unknown`（无 fsync 后段），批次 C 细化
/// atomicWriteFailureCommitState 后启用，暂抑制死代码警告。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileWriteCommitState {
    NotCommitted,
    Committed,
    Unknown,
}

/// 迁移自契约 `FileNode`。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<FileNode>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextReadResponse {
    pub success: bool,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteFileResponse {
    pub success: bool,
    pub commit_state: FileWriteCommitState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonReadResponse {
    pub success: bool,
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 内部错误 —— 守卫类错误映射为统一文案，IO 类错误按频道映射默认文案。
/// `Io` 携带的原始消息仅用于后续日志/诊断（当前不外泄到前端），
/// 骨架阶段无读取点，暂抑制死代码警告。
#[allow(dead_code)]
#[derive(Debug, Clone)]
enum FsError {
    Guard(GuardKind),
    /// 目标不存在（对齐 ENOENT / SECURE_FS_NOT_FOUND）。
    MissingFile,
    Io(String),
}

impl From<GuardKind> for FsError {
    fn from(kind: GuardKind) -> Self {
        FsError::Guard(kind)
    }
}

type FsOutcome<T> = Result<T, FsError>;

/// 对齐 `projectFilesystemFailure`：只本地化文案，不暴露路径或原始错误。
/// locale 来自已保存的界面偏好，不来自当前项目的写作语言。
fn fs_error_message(channel: &str, error: &FsError) -> String {
    let (zh_cn, en_us) = fs_error_copy(channel, error);
    crate::commands::config::main_text(&zh_cn, en_us)
}

/// 错误分类与文案分离，便于不触碰全局配置地检查所有守卫/频道组合。
fn fs_error_copy(channel: &str, error: &FsError) -> (String, &'static str) {
    if let FsError::Guard(kind) = error {
        let en_us = match kind {
            GuardKind::MissingSessionContext => {
                "Project session context is missing; the operation was rejected."
            }
            GuardKind::OutsideProject => {
                "The target is outside the current project; the operation was rejected."
            }
            GuardKind::CrossProject => "Cross-project file access was rejected.",
            GuardKind::LeaseInvalid => "The project lease has expired; the operation was rejected.",
        };
        return (guard_message(*kind), en_us);
    }
    let (zh_cn, en_us) = match channel {
        "fs:read-file" if matches!(error, FsError::MissingFile) => (
            "项目文件不存在。请检查文件路径；故事架构保存在项目数据中，请使用 read_architecture 工具读取。",
            "The project file does not exist. Check the file path; story architecture is stored in project data and must be read with the read_architecture tool.",
        ),
        "fs:read-file" => ("无法读取项目文件。", "Could not read the project file."),
        "fs:read-json" => ("无法读取项目数据。", "Could not read the project data."),
        "fs:write-file" | "fs:mkdir" | "fs:write-json" => (
            "无法写入项目文件。",
            "Could not write the project file.",
        ),
        _ => ("项目文件操作失败。", "The project file operation failed."),
    };
    (zh_cn.to_string(), en_us)
}

/// 统一守卫链 —— 对齐 `assertProjectFileOperation`：
/// 会话租约 → expectedProjectPath → 路径边界（词法 + realpath）。
fn assert_project_file_operation(
    state: &AppState,
    context: &ProjectSessionContext,
    target_path: &str,
    expected_project_path: &str,
    mode: PathCheckMode,
) -> FsOutcome<()> {
    let active = state.active_project_snapshot();
    let active_root = active.as_ref().map(|project| project.root_path.as_str());
    assert_current_project_context(context, active.as_ref())?;
    crate::security::assert_required_expected_project_path(
        active_root,
        Some(expected_project_path),
    )
    .map_err(FsError::Guard)?;
    assert_project_file_path(target_path, active_root.unwrap_or_default(), mode)
        .map_err(FsError::Guard)?;
    Ok(())
}

/// 原子写：先建父目录，再写同目录临时文件，rename 提交。
/// 返回提交前失败（NotCommitted）；rename 自身失败同样视为 NotCommitted
///（骨架无 fsync 后阶段，`Unknown` 暂不产生）。
fn write_text_atomically(target: &std::path::Path, content: &str) -> std::io::Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "无父目录"))?;
    std::fs::create_dir_all(parent)?;
    let file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let tmp = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let write_result = std::fs::write(&tmp, content);
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    match std::fs::rename(&tmp, target) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}

/// 递归列目录：过滤 `.` 开头、目录优先、名称排序（TODO locale 对齐 zh-CN）。
fn read_dir_recursive(
    root_display: &std::path::Path,
    dir: &std::path::Path,
) -> FsOutcome<Vec<FileNode>> {
    let entries = std::fs::read_dir(dir).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            FsError::MissingFile
        } else {
            FsError::Io(err.to_string())
        }
    })?;
    let mut visible: Vec<(String, bool, std::path::PathBuf)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| FsError::Io(err.to_string()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let is_dir = entry
            .file_type()
            .map_err(|err| FsError::Io(err.to_string()))?
            .is_dir();
        visible.push((name, is_dir, entry.path()));
    }
    visible.sort_by(|a, b| match (a.1, b.1) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.0.cmp(&b.0),
    });
    let mut nodes = Vec::with_capacity(visible.len());
    for (name, is_dir, path) in visible {
        let display_path = root_display.join(path.strip_prefix(dir).unwrap_or(&path));
        let node = if is_dir {
            FileNode {
                name,
                path: display_path.to_string_lossy().to_string(),
                is_dir: true,
                children: Some(read_dir_recursive(root_display, &path)?),
            }
        } else {
            FileNode {
                name,
                path: display_path.to_string_lossy().to_string(),
                is_dir: false,
                children: None,
            }
        };
        nodes.push(node);
    }
    Ok(nodes)
}

// ===== fs:read-file =====

#[tauri::command]
pub async fn fs_read_file(
    state: State<'_, AppState>,
    project_session: ProjectSessionContext,
    file_path: String,
    expected_project_path: String,
) -> Result<TextReadResponse, String> {
    let _file_lock = state.fs_lock.lock().expect("fs_lock 锁中毒");
    Ok(
        match read_file_inner(&state, &project_session, &file_path, &expected_project_path) {
            Ok(content) => TextReadResponse {
                success: true,
                content,
                error: None,
            },
            Err(error) => TextReadResponse {
                success: false,
                content: String::new(),
                error: Some(fs_error_message("fs:read-file", &error)),
            },
        },
    )
}

fn read_file_inner(
    state: &State<'_, AppState>,
    context: &ProjectSessionContext,
    file_path: &str,
    expected_project_path: &str,
) -> FsOutcome<String> {
    assert_project_file_operation(
        state,
        context,
        file_path,
        expected_project_path,
        PathCheckMode::Existing,
    )?;
    std::fs::read_to_string(file_path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            FsError::MissingFile
        } else {
            FsError::Io(err.to_string())
        }
    })
}

// ===== fs:write-file =====

#[tauri::command]
pub async fn fs_write_file(
    state: State<'_, AppState>,
    project_session: ProjectSessionContext,
    file_path: String,
    content: String,
    expected_project_path: String,
) -> Result<WriteFileResponse, String> {
    let _file_lock = state.fs_lock.lock().expect("fs_lock 锁中毒");
    Ok(
        match write_file_inner(
            &state,
            &project_session,
            &file_path,
            &content,
            &expected_project_path,
        ) {
            Ok(()) => WriteFileResponse {
                success: true,
                commit_state: FileWriteCommitState::Committed,
                error: None,
            },
            Err(error) => WriteFileResponse {
                success: false,
                commit_state: FileWriteCommitState::NotCommitted,
                error: Some(fs_error_message("fs:write-file", &error)),
            },
        },
    )
}

fn write_file_inner(
    state: &State<'_, AppState>,
    context: &ProjectSessionContext,
    file_path: &str,
    content: &str,
    expected_project_path: &str,
) -> FsOutcome<()> {
    assert_project_file_operation(
        state,
        context,
        file_path,
        expected_project_path,
        PathCheckMode::Writable,
    )?;
    write_text_atomically(std::path::Path::new(file_path), content)
        .map_err(|err| FsError::Io(err.to_string()))
}

// ===== fs:list-dir =====

#[tauri::command]
pub async fn fs_list_dir(
    state: State<'_, AppState>,
    project_session: ProjectSessionContext,
    dir_path: String,
    expected_project_path: String,
) -> Result<Vec<FileNode>, String> {
    let _file_lock = state.fs_lock.lock().expect("fs_lock 锁中毒");
    assert_project_file_operation(
        &state,
        &project_session,
        &dir_path,
        &expected_project_path,
        PathCheckMode::Existing,
    )
    .map_err(|error| fs_error_message("fs:list-dir", &error))?;
    let root_display = crate::security::lexically_normalize(&dir_path);
    read_dir_recursive(&root_display, std::path::Path::new(&dir_path))
        .map_err(|error| fs_error_message("fs:list-dir", &error))
}

// ===== fs:mkdir =====

#[tauri::command]
pub async fn fs_mkdir(
    state: State<'_, AppState>,
    project_session: ProjectSessionContext,
    dir_path: String,
    expected_project_path: String,
) -> Result<SimpleResult, String> {
    let _file_lock = state.fs_lock.lock().expect("fs_lock 锁中毒");
    Ok(
        match mkdir_inner(&state, &project_session, &dir_path, &expected_project_path) {
            Ok(()) => SimpleResult {
                success: true,
                error: None,
            },
            Err(error) => SimpleResult {
                success: false,
                error: Some(fs_error_message("fs:mkdir", &error)),
            },
        },
    )
}

fn mkdir_inner(
    state: &State<'_, AppState>,
    context: &ProjectSessionContext,
    dir_path: &str,
    expected_project_path: &str,
) -> FsOutcome<()> {
    assert_project_file_operation(
        state,
        context,
        dir_path,
        expected_project_path,
        PathCheckMode::Writable,
    )?;
    std::fs::create_dir_all(dir_path).map_err(|err| FsError::Io(err.to_string()))
}

// ===== fs:check-exists =====

#[tauri::command]
pub async fn fs_check_exists(
    state: State<'_, AppState>,
    project_session: ProjectSessionContext,
    file_path: String,
    expected_project_path: String,
) -> Result<bool, String> {
    let _file_lock = state.fs_lock.lock().expect("fs_lock 锁中毒");
    assert_project_file_operation(
        &state,
        &project_session,
        &file_path,
        &expected_project_path,
        PathCheckMode::Writable,
    )
    .map_err(|error| fs_error_message("fs:check-exists", &error))?;
    Ok(std::path::Path::new(&file_path).exists())
}

// ===== fs:read-json =====

#[tauri::command]
pub async fn fs_read_json(
    state: State<'_, AppState>,
    project_session: ProjectSessionContext,
    file_path: String,
    expected_project_path: String,
) -> Result<JsonReadResponse, String> {
    let _file_lock = state.fs_lock.lock().expect("fs_lock 锁中毒");
    let outcome = (|| -> FsOutcome<serde_json::Value> {
        assert_project_file_operation(
            &state,
            &project_session,
            &file_path,
            &expected_project_path,
            PathCheckMode::Existing,
        )?;
        let content = std::fs::read_to_string(file_path).map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                FsError::MissingFile
            } else {
                FsError::Io(err.to_string())
            }
        })?;
        serde_json::from_str(&content).map_err(|err| FsError::Io(err.to_string()))
    })();
    match outcome {
        Ok(data) => Ok(JsonReadResponse {
            success: true,
            data: Some(data),
            error: None,
        }),
        Err(error) => Ok(JsonReadResponse {
            success: false,
            data: None,
            error: Some(fs_error_message("fs:read-json", &error)),
        }),
    }
}

// ===== fs:write-json =====

#[tauri::command]
pub async fn fs_write_json(
    state: State<'_, AppState>,
    project_session: ProjectSessionContext,
    file_path: String,
    data: serde_json::Value,
    expected_project_path: String,
) -> Result<SimpleResult, String> {
    let _file_lock = state.fs_lock.lock().expect("fs_lock 锁中毒");
    let outcome = (|| -> FsOutcome<()> {
        assert_project_file_operation(
            &state,
            &project_session,
            &file_path,
            &expected_project_path,
            PathCheckMode::Writable,
        )?;
        let pretty =
            serde_json::to_string_pretty(&data).map_err(|err| FsError::Io(err.to_string()))?;
        write_text_atomically(std::path::Path::new(&file_path), &pretty)
            .map_err(|err| FsError::Io(err.to_string()))
    })();
    match outcome {
        Ok(()) => Ok(SimpleResult {
            success: true,
            error: None,
        }),
        Err(error) => Ok(SimpleResult {
            success: false,
            error: Some(fs_error_message("fs:write-json", &error)),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_project_root(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vela-fs-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn filesystem_guard_copy_remains_specific_in_both_languages_test() {
        for channel in [
            "fs:read-file",
            "fs:write-file",
            "fs:list-dir",
            "fs:mkdir",
            "fs:check-exists",
            "fs:read-json",
            "fs:write-json",
        ] {
            for (kind, expected_english) in [
                (
                    GuardKind::MissingSessionContext,
                    "Project session context is missing; the operation was rejected.",
                ),
                (
                    GuardKind::OutsideProject,
                    "The target is outside the current project; the operation was rejected.",
                ),
                (
                    GuardKind::CrossProject,
                    "Cross-project file access was rejected.",
                ),
                (
                    GuardKind::LeaseInvalid,
                    "The project lease has expired; the operation was rejected.",
                ),
            ] {
                let (zh_cn, en_us) = fs_error_copy(channel, &FsError::Guard(kind));
                assert_eq!(zh_cn, guard_message(kind));
                assert_eq!(en_us, expected_english);
            }
        }
    }

    #[test]
    fn filesystem_io_copy_does_not_disclose_paths_or_raw_errors_test() {
        let raw = "Access denied: D:\\private-synthetic\\source.txt";
        let error = FsError::Io(raw.to_string());
        for (channel, zh, en) in [
            (
                "fs:read-file",
                "无法读取项目文件。",
                "Could not read the project file.",
            ),
            (
                "fs:read-json",
                "无法读取项目数据。",
                "Could not read the project data.",
            ),
            (
                "fs:write-file",
                "无法写入项目文件。",
                "Could not write the project file.",
            ),
            (
                "fs:mkdir",
                "无法写入项目文件。",
                "Could not write the project file.",
            ),
            (
                "fs:write-json",
                "无法写入项目文件。",
                "Could not write the project file.",
            ),
            (
                "fs:list-dir",
                "项目文件操作失败。",
                "The project file operation failed.",
            ),
            (
                "fs:check-exists",
                "项目文件操作失败。",
                "The project file operation failed.",
            ),
        ] {
            let copy = fs_error_copy(channel, &error);
            assert_eq!(copy.0, zh);
            assert_eq!(copy.1, en);
            assert!(!copy.0.contains(raw));
            assert!(!copy.1.contains(raw));
        }
    }

    #[test]
    fn missing_file_copy_only_guides_plain_text_readers_test() {
        let (zh_cn, en_us) = fs_error_copy("fs:read-file", &FsError::MissingFile);
        assert_eq!(zh_cn, "项目文件不存在。请检查文件路径；故事架构保存在项目数据中，请使用 read_architecture 工具读取。");
        assert_eq!(en_us, "The project file does not exist. Check the file path; story architecture is stored in project data and must be read with the read_architecture tool.");
        for channel in ["fs:read-json", "fs:write-file", "fs:mkdir", "fs:write-json"] {
            let (zh_cn, en_us) = fs_error_copy(channel, &FsError::MissingFile);
            assert!(!zh_cn.contains("read_architecture"));
            assert!(!en_us.contains("read_architecture"));
        }
    }

    #[test]
    fn write_failure_envelopes_keep_commit_state_and_json_shape_test() {
        let (_, message) = fs_error_copy("fs:write-file", &FsError::Guard(GuardKind::LeaseInvalid));
        let response = WriteFileResponse {
            success: false,
            commit_state: FileWriteCommitState::NotCommitted,
            error: Some(message.to_string()),
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["success"], false);
        assert_eq!(value["commitState"], "not_committed");
        assert_eq!(value["error"], message);
        let response = JsonReadResponse {
            success: false,
            data: None,
            error: Some(message.to_string()),
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["data"], serde_json::Value::Null);
        assert_eq!(value["error"], message);
    }

    #[test]
    fn atomic_write_creates_parent_and_content_test() {
        let dir = temp_project_root("atomic");
        let target = dir.join("sub").join("chapter.md");
        write_text_atomically(&target, "正文").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "正文");
        // 覆盖写
        write_text_atomically(&target, "正文v2").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "正文v2");
        // 临时文件不留残留
        let leftovers: Vec<_> = std::fs::read_dir(target.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "临时文件必须清理");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_dir_recursive_filters_hidden_and_sorts_dirs_first_test() {
        let dir = temp_project_root("list");
        std::fs::create_dir_all(dir.join("chapters")).unwrap();
        std::fs::write(dir.join("chapters").join("1.md"), "a").unwrap();
        std::fs::write(dir.join(".hidden"), "x").unwrap();
        std::fs::write(dir.join("readme.md"), "r").unwrap();
        let tree = read_dir_recursive(&dir, &dir).unwrap();
        assert_eq!(tree.len(), 2, "隐藏文件必须被过滤，目录优先排序");
        let chapters = &tree[0];
        assert!(chapters.is_dir, "目录优先于文件");
        assert_eq!(tree[1].name, "readme.md");
        let children = chapters.children.as_ref().unwrap();
        assert_eq!(children.len(), 1);
    }
}
