//! 本机 Skill 命令：只有原生选择结果可成为候选，渲染层不能提交路径。
use crate::{state::AppState, writing_skills::local as package};
use serde_json::{json, Value};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;

fn public_error() -> Value {
    json!({"success":false,"error":super::super::config::main_text_with_system_locale(
        "无法读取所选 Skill，请重新选择安全的 UTF-8 文本文件或文件夹。",
        "Cannot read the selected skill. Choose a safe UTF-8 text file or folder again.")})
}
#[tauri::command]
pub async fn skills_pick_local(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    kind: String,
) -> Result<Value, String> {
    if window.label() != "main" || !matches!(kind.as_str(), "file" | "folder") {
        return Ok(public_error());
    }
    let owner = window.label().to_string();
    let request = state
        .local_skill_candidates
        .lock()
        .map_err(|_| "Skill candidate lock poisoned")?
        .begin(&owner);
    let title = if kind == "folder" {
        super::super::config::main_text_with_system_locale(
            "选择 Skill 文件夹",
            "Choose a skill folder",
        )
    } else {
        super::super::config::main_text_with_system_locale(
            "选择 SKILL.md 文件",
            "Choose a SKILL.md file",
        )
    };
    let mut dialog = app.dialog().file().set_parent(&window).set_title(title);
    let (sender, receiver) = std::sync::mpsc::channel();
    if kind == "folder" {
        dialog.pick_folder(move |selected| {
            let _ = sender.send(selected);
        });
    } else {
        dialog = dialog.add_filter("Markdown", &["md", "markdown"]);
        dialog.pick_file(move |selected| {
            let _ = sender.send(selected);
        });
    }
    let selected = tauri::async_runtime::spawn_blocking(move || {
        receiver.recv_timeout(std::time::Duration::from_secs(600))
    })
    .await;
    let selected = match selected {
        Ok(Ok(Some(selected))) => selected,
        Ok(Ok(None)) => return Ok(json!({"success":true,"cancelled":true})),
        _ => return Ok(public_error()),
    };
    let path = match selected {
        tauri_plugin_dialog::FilePath::Path(path) => path,
        _ => return Ok(public_error()),
    };
    let load_path = path.clone();
    let load_kind = kind.clone();
    let loaded = match tauri::async_runtime::spawn_blocking(move || {
        package::load_local(&load_kind, &load_path)
    })
    .await
    {
        Ok(Ok(loaded)) => loaded,
        _ => return Ok(public_error()),
    };
    // 回调之后再复核窗口仍存活；新选择或窗口销毁会使请求令牌失效。
    if app.get_webview_window(&owner).is_none() {
        return Ok(public_error());
    }
    let result = state
        .local_skill_candidates
        .lock()
        .map_err(|_| "Skill candidate lock poisoned")?
        .complete(&owner, &request, &kind, path, &loaded);
    match result {
        Ok(inspection) => Ok(json!({"success":true,"cancelled":false,"inspection":inspection})),
        Err(_) => Ok(public_error()),
    }
}
#[tauri::command]
pub async fn skills_install_local(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    candidate_id: String,
) -> Result<Value, String> {
    if window.label() != "main" {
        return Ok(public_error());
    }
    let owner = window.label().to_string();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        let state = app.state::<AppState>();
        // 与同名 GitHub 安装及卸载串行；候选先消费，任何失败都必须重新检查。
        let _operation = state.fs_lock.lock().map_err(|_| "Skill operation lock poisoned")?;
        let loaded = state.local_skill_candidates.lock().map_err(|_| "Skill candidate lock poisoned")?.take(&owner, &candidate_id)?;
        if !loaded.inspection.skill.compatible { return Err("Skill incompatible".into()); }
        let home = crate::app_paths::lorekeeper_home();
        super::ensure_owned_skills_root_at(&home)?;
        let directory = super::writing_skill_directory_at(&home, &loaded.inspection.skill.metadata.name)?;
        package::install_at(&directory, &loaded)?;
        let inspected = &loaded.inspection.skill;
        Ok(json!({"success":true,"skill":{"name":inspected.metadata.name,"source":"user","version":inspected.metadata.version,
            "language":inspected.metadata.language,"compatible":true,"utf8Bytes":inspected.utf8_bytes}}))
    }).await;
    match result {
        Ok(Ok(result)) => Ok(result),
        _ => Ok(
            json!({"success":false,"error":format!("Error: {}",super::super::config::main_text_with_system_locale(
            "Skill 无法安装：候选已过期、内容变化、目标已存在或内容不兼容。请重新选择并检查。",
            "Cannot install the skill: the candidate expired, changed, already exists, or is incompatible. Choose and inspect it again."))}),
        ),
    }
}
