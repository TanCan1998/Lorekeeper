//! 独立的本机项目清理入口：不注册 IPC，不接收命令行路径，不读取最近项目列表。
//! 卸载默认保留项目；只有原生目录选择和两次明确确认后才执行清理。

use crate::project_access::{is_project_manifest, manifest_path};
use crate::services::portable_project_archive::{path_identity, same_object, FileIdentity};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tauri_plugin_dialog::{
    DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};

struct CleanupPlan {
    root: PathBuf,
    identity: FileIdentity,
    manifest_digest: Vec<u8>,
}

fn failure(message: impl std::fmt::Display) -> String {
    format!("Error: {message}")
}

fn is_link(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // 除符号链接外，也拒绝 junction、挂载点和其他 reparse point。
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn check_path(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(failure)?;
    if is_link(&metadata) || (!metadata.is_dir() && !metadata.is_file()) {
        return Err(failure("目录含链接或特殊文件，已保留项目"));
    }
    Ok(())
}

fn check_tree(root: &Path) -> Result<(), String> {
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut count = 0;
    while let Some((path, depth)) = pending.pop() {
        count += 1;
        if count > 100_000 || depth > 64 {
            return Err(failure("目录规模超出安全清理范围，请手动处理"));
        }
        check_path(&path)?;
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.eq_ignore_ascii_case(".vela") || name.eq_ignore_ascii_case(".lorekeeper") {
            return Err(failure("目录包含其他数据根，已拒绝清理"));
        }
        if path.is_dir() {
            if path != root && path.join(".lore/project.json").exists() {
                return Err(failure("目录中包含嵌套项目，请分别选择清理"));
            }
            for entry in std::fs::read_dir(&path).map_err(failure)? {
                pending.push((entry.map_err(failure)?.path(), depth + 1));
            }
        }
    }
    Ok(())
}

fn protected_paths() -> Vec<PathBuf> {
    let mut paths = vec![crate::app_paths::lorekeeper_home()];
    if let Ok(binary) = std::env::current_exe() {
        if let Some(parent) = binary.parent() {
            paths.push(parent.to_path_buf());
        }
    }
    for key in [
        "WINDIR",
        "SystemRoot",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
        "APPDATA",
        "LOCALAPPDATA",
        "AI_NOVEL_VELA_HOME",
    ] {
        if let Some(value) = std::env::var_os(key) {
            paths.push(PathBuf::from(value));
        }
    }
    #[cfg(target_os = "macos")]
    paths.extend(
        [
            "/System",
            "/Library",
            "/Applications",
            "/usr",
            "/bin",
            "/sbin",
            "/private",
        ]
        .into_iter()
        .map(PathBuf::from),
    );
    if let Some(home) = crate::project_access::home_directory() {
        paths.push(Path::new(&home).join(".vela"));
    }
    paths
}

fn is_within(root: &Path, target: &Path) -> bool {
    #[cfg(windows)]
    {
        crate::project_access::is_contained_path(&root.to_string_lossy(), &target.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        target.starts_with(root)
    }
}

fn prepare(root: &Path, home: Option<&Path>, protected: &[PathBuf]) -> Result<CleanupPlan, String> {
    if !root.is_absolute() {
        return Err(failure("必须通过目录选择器指定绝对路径"));
    }
    // 先检查原始路径，不能先 realpath 后误把链接指向的真实目录当成用户授权目标。
    for ancestor in root.ancestors() {
        check_path(ancestor)?;
        let name = ancestor.file_name().unwrap_or_default().to_string_lossy();
        if name.eq_ignore_ascii_case(".vela") || name.eq_ignore_ascii_case(".lorekeeper") {
            return Err(failure("拒绝清理全局数据根中的目录"));
        }
    }
    let root = std::fs::canonicalize(root).map_err(failure)?;
    if root.parent().is_none() || !root.is_dir() {
        return Err(failure("拒绝清理磁盘根目录或非目录目标"));
    }
    if let Some(home) = home.and_then(|p| std::fs::canonicalize(p).ok()) {
        if is_within(&root, &home) {
            return Err(failure("拒绝清理用户主目录及其上级目录"));
        }
    }
    for protected in protected
        .iter()
        .filter_map(|p| std::fs::canonicalize(p).ok())
    {
        if is_within(&protected, &root) || is_within(&root, &protected) {
            return Err(failure("拒绝清理程序、系统或全局数据目录"));
        }
    }
    check_tree(&root)?;
    let manifest = manifest_path(&root.to_string_lossy());
    let body = std::fs::read(manifest).map_err(failure)?;
    let value = serde_json::from_slice(&body).map_err(failure)?;
    if !is_project_manifest(&value) {
        return Err(failure(
            "只允许清理含有效 .lore/project.json 的 LoreKeeper 项目",
        ));
    }
    Ok(CleanupPlan {
        identity: path_identity(&root).map_err(failure)?,
        root,
        manifest_digest: Sha256::digest(body).to_vec(),
    })
}

impl CleanupPlan {
    fn revalidate(&self, home: Option<&Path>, protected: &[PathBuf]) -> Result<(), String> {
        let latest = prepare(&self.root, home, protected)?;
        if latest.root != self.root
            || !same_object(&self.identity, &latest.identity)
            || latest.manifest_digest != self.manifest_digest
        {
            return Err(failure("确认期间目录或项目身份发生变化，已取消清理"));
        }
        Ok(())
    }

    fn delete(self, home: Option<&Path>, protected: &[PathBuf]) -> Result<(), String> {
        self.revalidate(home, protected)?;
        // 不跟随目录内符号链接；预检额外拒绝所有链接及嵌套项目。
        // 不重试、不扫盘，失败如实报告，避免部分删除被伪装成成功。
        std::fs::remove_dir_all(&self.root).map_err(|error| {
            failure(format!(
                "清理未完成，可能有文件被占用；请检查所选目录：{error}"
            ))
        })
    }
}

fn confirmed(result: MessageDialogResult) -> bool {
    // 取消、关闭窗口、默认的“保留”以及未知返回值均不能授权删除。
    result == MessageDialogResult::Custom("删除所选项目".to_string())
}

fn confirm(app: &tauri::AppHandle, message: String) -> bool {
    confirmed(
        app.dialog()
            .message(message)
            .title("LoreKeeper 项目清理")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::YesNoCancelCustom(
                "保留项目".into(),
                "删除所选项目".into(),
                "取消".into(),
            ))
            .blocking_show_with_result(),
    )
}

fn cleanup_dialog(app: &tauri::AppHandle) -> Result<(), String> {
    let Some(selection) = app
        .dialog()
        .file()
        .set_title("选择要清理的 LoreKeeper 项目目录（取消则保留）")
        .blocking_pick_folder()
    else {
        return Ok(());
    };
    let root = selection.into_path().map_err(failure)?;
    let home = Some(PathBuf::from(
        crate::project_access::home_directory()
            .ok_or_else(|| failure("无法确认用户主目录，已保留项目"))?,
    ));
    let protected = protected_paths();
    let plan = prepare(&root, home.as_deref(), &protected)?;
    if !confirm(app, format!(
        "默认保留项目。请先关闭所有 LoreKeeper 窗口并备份。\n\n所选目录：\n{}\n\n删除范围是该目录及其全部文件（包括正文、数据库和备份），不可撤销。",
        plan.root.display(),
    )) { return Ok(()) }
    if !confirm(app, format!(
        "二次确认：永久删除以下项目目录及全部文件？\n\n{}\n\n其他项目、全局配置和共享 WebView2 不会清理。",
        plan.root.display(),
    )) { return Ok(()) }
    plan.delete(home.as_deref(), &protected)?;
    app.dialog()
        .message("所选项目已清理。其他项目和全局配置保持不变。")
        .title("LoreKeeper 项目清理")
        .buttons(MessageDialogButtons::OkCustom("确定".into()))
        .blocking_show();
    Ok(())
}

/// 仅由本机入口调用：清空窗口配置，不启动业务服务，也不对渲染层开放权限。
pub(crate) fn run() {
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                if let Err(error) = cleanup_dialog(&handle) {
                    handle
                        .dialog()
                        .message(error)
                        .title("LoreKeeper 项目清理失败")
                        .kind(MessageDialogKind::Error)
                        .buttons(MessageDialogButtons::OkCustom("保留剩余文件并退出".into()))
                        .blocking_show();
                }
                handle.exit(0);
            });
            Ok(())
        })
        .run(context)
        .expect("LoreKeeper 项目清理入口启动失败");
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "lorekeeper-cleanup-{}",
            crate::project_access::random_uuid_v4()
        ));
        std::fs::create_dir_all(root.join(".lore")).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        std::fs::write(root.join(".lore/project.json"), serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1, "kind": "ai-novel-project",
            "projectId": crate::project_access::random_uuid_v4(), "createdAt": "2026-10-19T00:00:00Z",
        })).unwrap()).unwrap();
        root
    }

    #[test]
    fn cleanup_accepts_only_explicit_delete_button() {
        for result in [
            MessageDialogResult::Yes,
            MessageDialogResult::No,
            MessageDialogResult::Ok,
            MessageDialogResult::Cancel,
            MessageDialogResult::Custom("保留项目".into()),
            MessageDialogResult::Custom("取消".into()),
        ] {
            assert!(!confirmed(result));
        }
        assert!(confirmed(MessageDialogResult::Custom(
            "删除所选项目".into()
        )));
    }

    #[test]
    fn cleanup_preflight_preserves_valid_project() {
        let root = fixture();
        let plan = prepare(&root, None, &[]).unwrap();
        plan.revalidate(None, &[]).unwrap();
        assert!(root.join(".lore/project.json").is_file());
    }

    #[test]
    fn cleanup_rejects_home_ancestors_and_protected_directories() {
        let root = fixture();
        assert!(prepare(&root, Some(&root), &[]).is_err());
        assert!(prepare(&root, Some(&root.join(".lore")), &[]).is_err());
        assert!(prepare(&root, None, &[root.clone()]).is_err());
        assert!(prepare(&root, None, &[root.join(".lore")]).is_err());
        assert!(prepare(&root, None, &[root.parent().unwrap().to_path_buf()]).is_err());
        assert!(prepare(Path::new("relative"), None, &[]).is_err());
    }

    #[test]
    fn cleanup_rejects_missing_invalid_or_foreign_manifest() {
        let root = fixture();
        for body in ["{}", "not-json", r#"{"schemaVersion":1,"kind":"other"}"#] {
            std::fs::write(root.join(".lore/project.json"), body).unwrap();
            assert!(prepare(&root, None, &[]).is_err());
        }
        assert!(prepare(&root.join(".lore"), None, &[]).is_err());
    }

    #[test]
    fn cleanup_rejects_nested_projects_and_foreign_data() {
        for child in [".vela", ".lorekeeper", "nested/.lore"] {
            let root = fixture();
            std::fs::create_dir_all(root.join(child)).unwrap();
            if child == "nested/.lore" {
                std::fs::write(root.join(child).join("project.json"), "{}").unwrap();
            }
            assert!(prepare(&root, None, &[]).is_err());
        }
    }

    #[test]
    fn cleanup_rejects_projects_inside_foreign_global_data() {
        let root = fixture();
        let nested = root.join(".vela/project");
        std::fs::create_dir_all(nested.join(".lore")).unwrap();
        std::fs::copy(
            root.join(".lore/project.json"),
            nested.join(".lore/project.json"),
        )
        .unwrap();
        assert!(prepare(&nested, None, &[]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn cleanup_windows_protected_paths_are_case_insensitive() {
        assert!(is_within(
            Path::new("C:/Users/Author"),
            Path::new("c:/users/author/project")
        ));
        assert!(!is_within(
            Path::new("C:/Users/Author"),
            Path::new("C:/Users/Author2")
        ));
    }

    #[test]
    fn cleanup_revalidates_manifest_and_directory_identity() {
        let root = fixture();
        let plan = prepare(&root, None, &[]).unwrap();
        std::fs::write(root.join(".lore/project.json"), "{}").unwrap();
        assert!(plan.revalidate(None, &[]).is_err());
        let root = fixture();
        let plan = prepare(&root, None, &[]).unwrap();
        std::fs::rename(&root, root.with_extension("original")).unwrap();
        std::fs::create_dir_all(root.join(".lore")).unwrap();
        std::fs::copy(
            root.with_extension("original").join(".lore/project.json"),
            root.join(".lore/project.json"),
        )
        .unwrap();
        assert!(plan.revalidate(None, &[]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn cleanup_rejects_junction_without_reading_target() {
        let root = fixture();
        // 开发者模式不是测试前提；junction 使用 cmd 内建命令创建，仅作用于合成目录。
        let target = fixture();
        let link = root.join("linked");
        let status = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(prepare(&root, None, &[]).is_err());
        assert!(target.join(".lore/project.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_rejects_symlink_without_reading_target() {
        let root = fixture();
        let target = fixture();
        std::os::unix::fs::symlink(&target, root.join("linked")).unwrap();
        assert!(prepare(&root, None, &[]).is_err());
        assert!(prepare(&root.join("linked"), None, &[]).is_err());
        assert!(target.join(".lore/project.json").exists());
    }
}
