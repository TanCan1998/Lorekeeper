//! 启动门禁命令（RD-10 启动态子集：`startup:*` 两频道）。
//!
//! 对齐 `electron/controllers/startup-controller.ts`：
//! - 信任发送方：只有主窗口的原始渲染文档可以读回/确认自己的存储；
//! - `startup:get-state` 恒以状态对象返回（阻塞态也返回对象，不 reject）；
//! - `startup:appearance-ack` 对非法外观 v3 读回返回 `false`，不恢复旧皮肤快照。
//!
//! **发送方等价物（刻意偏离）**：基线比较 `event.sender` 与主窗口 webContents 及其
//! mainFrame URL；Tauri 侧校验 (1) 调用来自标签为 `main` 的窗口，(2) 该窗口当前 URL
//! 属于本应用自身 origin（`tauri://localhost`、Windows 生产的 `http://tauri.localhost`、
//! 或开发服务器 `http://localhost|127.0.0.1:<port>`）。导航防护插件已拒绝渲染层导航
//! 替换主框架，故该检查是纵深防御而非唯一屏障。

use serde_json::Value;
use tauri::{State, WebviewWindow};
use url::Url;

use crate::state::AppState;

/// 主窗口标签（与 `commands/window.rs` 的 `MAIN_WINDOW_LABEL` 一致）。
const MAIN_WINDOW_LABEL: &str = "main";

/// 启动期装载门禁（`lib.rs` 的 `setup` 调用）。
///
/// 失败**不 panic 也不阻断进程**：门禁自身进入阻塞态，`startup:get-state` 会把阻塞码
/// 交给渲染层，由渲染层的启动页决定是否进入工作台。
pub fn install_startup_gate(app: &tauri::AppHandle) {
    use tauri::Manager;
    let state = app.state::<AppState>();
    let mut gate = state.startup_gate.lock().unwrap();
    gate.prepare_at(&crate::app_paths::lorekeeper_home());
    match gate.blocked_code() {
        Some(code) => eprintln!("[Lorekeeper] 启动门禁阻塞：{}", code.as_str()),
        None => eprintln!("[Lorekeeper] 启动门禁就绪：等待外观v3读回确认"),
    }
}

/// URL 是否属于本应用自身（信任发送方的 URL 条件）。
pub(crate) fn is_trusted_startup_url(url: &Url) -> bool {
    match url.scheme() {
        // 生产：Tauri 自定义协议（Windows 上表现为 http://tauri.localhost）。
        "tauri" => url.host_str() == Some("localhost"),
        // 开发服务器（vite devUrl）与 Windows 生产 origin。
        "http" | "https" => matches!(
            url.host_str(),
            Some("localhost") | Some("127.0.0.1") | Some("tauri.localhost")
        ),
        _ => false,
    }
}

/// 只允许主窗口的自身文档读回启动状态。
pub(crate) fn is_trusted_startup_sender(webview: &WebviewWindow) -> bool {
    if webview.label() != MAIN_WINDOW_LABEL {
        return false;
    }
    let Ok(url) = webview.url() else {
        return false;
    };
    is_trusted_startup_url(&url)
}

/// `startup:get-state`
#[tauri::command]
pub fn startup_get_state(webview: WebviewWindow, state: State<'_, AppState>) -> Value {
    let gate = state.startup_gate.lock().unwrap();
    if !is_trusted_startup_sender(&webview) {
        // 对齐基线：不信任发送方只得到 blocked，不回任何码、不泄漏快照。
        return gate.blocked_for_untrusted_sender().to_json();
    }
    gate.startup_state().to_json()
}

/// `startup:appearance-ack`
#[tauri::command]
pub fn startup_appearance_ack(
    webview: WebviewWindow,
    ack: Value,
    state: State<'_, AppState>,
) -> bool {
    if !is_trusted_startup_sender(&webview) {
        return false;
    }
    state
        .startup_gate
        .lock()
        .unwrap()
        .acknowledge(webview.label(), &ack)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> Url {
        Url::parse(raw).unwrap()
    }

    #[test]
    fn trusted_startup_url_accepts_own_origins_only_test() {
        for accepted in [
            "tauri://localhost/index.html",
            "tauri://localhost/",
            "http://tauri.localhost/index.html",
            "http://localhost:5190/",
            "http://127.0.0.1:5190/index.html?x=1",
        ] {
            assert!(
                is_trusted_startup_url(&url(accepted)),
                "应被信任：{accepted}"
            );
        }
        for rejected in [
            "https://evil.example/index.html",
            "http://localhost.evil.example/",
            "file:///C:/novels/index.html",
            "tauri://other-host/index.html",
            "data:text/html,hi",
            "https://tauri.localhost.evil.example/",
        ] {
            assert!(
                !is_trusted_startup_url(&url(rejected)),
                "不得信任：{rejected}"
            );
        }
    }
}
