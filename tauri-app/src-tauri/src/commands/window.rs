//! Window 频道的 Tauri 命令实现
//!
//! 对应 `src/shared/ipc-channels.ts` 的 `WindowChannels`：
//! - `window:minimize` → [`window_minimize`]
//! - `window:toggle-maximize` → [`window_toggle_maximize`]
//! - `window:close` → [`window_close`]
//! - `window:resolve-close` → [`window_resolve_close`]
//! - 事件 `window:close-requested` → 由 [`WindowCloseGuard`] 在 `CloseRequested` 时发出
//!
//! # 批次 A 收口（2026-10-10）
//!
//! 此前四个命令都是**假成功骨架**（`// TODO` + 直接 `Ok(success: true)`），
//! 导致标题栏的最小化 / 最大化 / 关闭按钮「点了没反应」（GUI 冒烟实测）。
//! 本文件按 `electron/controllers/window-controller.ts` 逐条落实，并复刻其
//! **两段式关窗协议**：
//!
//! | 基线 | 本实现 |
//! |---|---|
//! | `win.minimize()` | [`window_minimize`] |
//! | `win.isMaximized() ? unmaximize() : maximize()` | [`window_toggle_maximize`] |
//! | `win.close()` | [`window_close`]（触发 `CloseRequested` → 守卫拦截并广播确认请求） |
//! | `installWindowCloseGuard` 的 `close` 监听 | [`WindowCloseGuard`] + `lib.rs` 的 `on_window_event` |
//! | `window:resolve-close` 的 requestId/decision 校验 | [`WindowCloseGuard::resolve`] |
//!
//! **刻意偏离**：基线守卫在 `webContents.isDestroyed() || isLoadingMainFrame()` 时放行；
//! Tauri 无对应概念（`CloseRequested` 只对存活窗口触发），故仅保留 `approved` 一条放行条件。

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// 主窗口标签（与 `tauri.conf.json` / 其它命令一致）
const MAIN_WINDOW_LABEL: &str = "main";

/// 两段式关窗守卫（对齐基线 `WindowCloseGuardState` + `closeGuardStates`）
#[derive(Debug, Default)]
pub struct WindowCloseGuard {
    approved: bool,
    pending_request_id: Option<String>,
    request_sequence: u64,
}

impl WindowCloseGuard {
    /// 窗口收到 `CloseRequested` 时调用。
    ///
    /// 返回 `(allow_close, emit_request_id)`：
    /// - `allow_close == true` → 放行（用户已在确认流程中选择 proceed）；
    /// - `allow_close == false` → 调用方须 `prevent_close()`；若 `emit_request_id` 为
    ///   `Some` 则需向渲染层广播 `window:close-requested`（已有 pending 时不重复广播）。
    pub fn on_close_requested(&mut self, label: &str) -> (bool, Option<String>) {
        if self.approved {
            return (true, None);
        }
        if self.pending_request_id.is_some() {
            return (false, None);
        }
        self.request_sequence += 1;
        let request_id = format!("{label}:{}", self.request_sequence);
        self.pending_request_id = Some(request_id.clone());
        (false, Some(request_id))
    }

    /// 仅匹配的退出确认可执行挂起；失败重置本次确认，绝不预先授予关闭许可。
    pub(crate) fn resolve_after_suspend(
        &mut self,
        request_id: &str,
        proceed: bool,
        suspend: impl FnOnce() -> Result<(), String>,
    ) -> Result<bool, String> {
        if self.pending_request_id.as_deref() != Some(request_id) {
            return Ok(false);
        }
        if proceed {
            if let Err(error) = suspend() {
                self.resolve(request_id, false);
                return Err(error);
            }
        }
        Ok(self.resolve(request_id, proceed))
    }

    /// 渲染层回应确认请求（对齐 `window:resolve-close`）。
    ///
    /// 返回 `false` 表示请求不匹配（须回 `{ success: false }`）。
    pub fn resolve(&mut self, request_id: &str, proceed: bool) -> bool {
        if self.pending_request_id.as_deref() != Some(request_id) {
            return false;
        }
        self.pending_request_id = None;
        if proceed {
            self.approved = true;
        }
        true
    }
}

/// 窗口操作响应
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowOperationResponse {
    pub success: bool,
}

/// 关闭决策（契约字面量：'proceed' | 'cancel'）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResolveCloseDecision {
    Proceed,
    Cancel,
}

/// 最大化/取消最大化响应（`maximized` 与基线一致：找不到窗口时缺省）
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleMaximizeResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximized: Option<bool>,
}

/// `window:minimize`（对齐基线：找不到窗口即 `{ success: false }`）
#[tauri::command]
pub fn window_minimize(app: AppHandle) -> Result<WindowOperationResponse, String> {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return Ok(WindowOperationResponse { success: false });
    };
    Ok(WindowOperationResponse {
        success: window.minimize().is_ok(),
    })
}

/// `window:toggle-maximize`（切换后回读 `is_maximized()`，与基线一致）
#[tauri::command]
pub fn window_toggle_maximize(app: AppHandle) -> Result<ToggleMaximizeResponse, String> {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return Ok(ToggleMaximizeResponse {
            success: false,
            maximized: None,
        });
    };
    let Ok(was_maximized) = window.is_maximized() else {
        return Ok(ToggleMaximizeResponse {
            success: false,
            maximized: None,
        });
    };
    let outcome = if was_maximized {
        window.unmaximize()
    } else {
        window.maximize()
    };
    Ok(ToggleMaximizeResponse {
        success: outcome.is_ok(),
        maximized: Some(window.is_maximized().unwrap_or(was_maximized)),
    })
}

/// `window:close`（真正 `close()`；随后由 `CloseRequested` 守卫决定是否广播确认请求）
#[tauri::command]
pub fn window_close(app: AppHandle) -> Result<WindowOperationResponse, String> {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return Ok(WindowOperationResponse { success: false });
    };
    Ok(WindowOperationResponse {
        success: window.close().is_ok(),
    })
}

/// `window:resolve-close`（两段式关窗协议的第二段）
#[tauri::command]
pub async fn window_resolve_close(
    app: AppHandle,
    request_id: String,
    decision: ResolveCloseDecision,
) -> Result<WindowOperationResponse, String> {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return Ok(WindowOperationResponse { success: false });
    };
    let proceed = matches!(decision, ResolveCloseDecision::Proceed);
    let state = app.state::<AppState>();
    // 退出挂起可能等待在途任务结算，不占用窗口消息线程或Tokio调度工作线程。
    let resolved = tokio::task::block_in_place(|| state.resolve_window_close(&request_id, proceed))
        .map_err(|error| {
            if error.starts_with("Error: ") {
                error
            } else {
                format!("Error: {error}")
            }
        })?;
    if !resolved {
        return Ok(WindowOperationResponse { success: false });
    }
    if proceed {
        // 已获批：再次 close() 会被守卫放行（与基线 `state.approved = true; win.close()` 等价）
        let _ = window.close();
    }
    Ok(WindowOperationResponse { success: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_close_requires_matching_confirmation_and_successful_suspend_test() {
        let mut guard = WindowCloseGuard::default();
        let (_, request) = guard.on_close_requested("main");
        let request = request.unwrap();
        let calls = std::cell::Cell::new(0);
        assert!(!guard
            .resolve_after_suspend("wrong", true, || {
                calls.set(calls.get() + 1);
                Ok(())
            })
            .unwrap());
        assert_eq!(calls.get(), 0);
        assert!(guard
            .resolve_after_suspend(&request, false, || {
                calls.set(calls.get() + 1);
                Ok(())
            })
            .unwrap());
        assert_eq!(calls.get(), 0);
        let (_, request) = guard.on_close_requested("main");
        let request = request.unwrap();
        assert_eq!(
            guard
                .resolve_after_suspend(&request, true, || Err("挂起失败".into()))
                .unwrap_err(),
            "挂起失败"
        );
        assert!(!guard.approved);
        assert!(guard.pending_request_id.is_none());
        let (allow, next) = guard.on_close_requested("main");
        assert!(!allow);
        let next = next.unwrap();
        assert_ne!(next, request);
        assert!(guard
            .resolve_after_suspend(&next, true, || {
                calls.set(calls.get() + 1);
                Ok(())
            })
            .unwrap());
        assert_eq!(calls.get(), 1);
        assert_eq!(guard.on_close_requested("main"), (true, None));
        assert!(!guard
            .resolve_after_suspend(&next, true, || panic!("重复确认不可再次挂起"))
            .unwrap());
    }

    #[test]
    fn concurrent_window_close_decisions_cannot_approve_before_suspend_or_repeat_it_test() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, Arc, Mutex,
        };
        let guard = Arc::new(Mutex::new(WindowCloseGuard::default()));
        let request = guard.lock().unwrap().on_close_requested("main").1.unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first = {
            let guard = guard.clone();
            let request = request.clone();
            let calls = calls.clone();
            std::thread::spawn(move || {
                guard
                    .lock()
                    .unwrap()
                    .resolve_after_suspend(&request, true, || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        started_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        Ok(())
                    })
            })
        };
        started_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(guard.try_lock().is_err());
        let second = {
            let guard = guard.clone();
            let calls = calls.clone();
            std::thread::spawn(move || {
                guard
                    .lock()
                    .unwrap()
                    .resolve_after_suspend(&request, true, || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
            })
        };
        release_tx.send(()).unwrap();
        assert!(first.join().unwrap().unwrap());
        assert!(!second.join().unwrap().unwrap());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(guard.lock().unwrap().approved);
    }

    #[test]
    fn window_operation_response_serializes() {
        let resp = WindowOperationResponse { success: true };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("success"));
    }

    #[test]
    fn toggle_maximize_response_omits_absent_maximized() {
        let with_state = ToggleMaximizeResponse {
            success: true,
            maximized: Some(true),
        };
        assert!(serde_json::to_string(&with_state)
            .unwrap()
            .contains("\"maximized\":true"));
        // 对齐基线：找不到窗口时只回 `{ success: false }`
        let without_state = ToggleMaximizeResponse {
            success: false,
            maximized: None,
        };
        assert_eq!(
            serde_json::to_string(&without_state).unwrap(),
            "{\"success\":false}"
        );
    }

    #[test]
    fn close_guard_follows_two_phase_protocol_test() {
        let mut guard = WindowCloseGuard::default();
        // 第一次关闭请求：拦截 + 广播 requestId
        let (allow, request) = guard.on_close_requested("main");
        assert!(!allow);
        assert_eq!(request.as_deref(), Some("main:1"));

        // 重复请求不重复广播（对齐基线 `if (state.pendingRequestId) return`）
        let (allow, request) = guard.on_close_requested("main");
        assert!(!allow);
        assert!(request.is_none());

        // requestId 不匹配 → 不认（回 success:false），且 pending 保持不变
        assert!(!guard.resolve("main:999", true));
        let (allow, request) = guard.on_close_requested("main");
        assert!(!allow, "不匹配的 resolve 不得放行");
        assert!(request.is_none(), "pending 仍在，不应重复广播");

        // cancel → 认，但不放行；之后可再次发起
        assert!(guard.resolve("main:1", false));
        let (allow, request) = guard.on_close_requested("main");
        assert!(!allow, "cancel 不得放行");
        assert_eq!(request.as_deref(), Some("main:2"));

        // proceed → 认且放行，此后 close 直接通过
        assert!(guard.resolve("main:2", true));
        let (allow, request) = guard.on_close_requested("main");
        assert!(allow);
        assert!(request.is_none());
    }
}
