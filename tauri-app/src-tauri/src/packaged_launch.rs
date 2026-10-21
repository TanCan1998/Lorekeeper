//! Windows 启动门禁：主程序必须由应用身份启动。
//!
//! WebView2 的进程归组依赖宿主进程的包身份；直接运行安装目录里的 exe 会拆组。
//! 这里在进入 Tauri 之前检查 GetCurrentPackageFullName：
//! - 已有包身份：正常启动；
//! - 没有包身份：先尝试同目录的身份启动器（它会注册并以包身份拉起），失败则提示退出。
//!
//! 豁免：debug 构建（开发）、LOREKEEPER_ALLOW_UNPACKAGED=1（本机调试）、
//! --cleanup-project（清理入口在调用前已单独放行）。
//! LOREKEEPER_PACKAGED_LAUNCH_PROBE=1 只检测不拉起、不弹窗，退出码 42，供自动化验证。

/// 无包身份且不允许拉起时，自动化探针使用的退出码。
pub const PROBE_EXIT_CODE: i32 = 42;

/// 是否拦截无包身份启动。纯函数，便于单测。
pub fn should_block(has_package: bool, debug_build: bool, allowed_unpackaged: bool) -> bool {
    !has_package && !debug_build && !allowed_unpackaged
}

#[cfg(windows)]
mod win {
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::ptr::null_mut;

    const APPMODEL_ERROR_NO_PACKAGE: i32 = 15700;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentPackageFullName(length: *mut u32, name: *mut u16) -> i32;
    }

    /// 当前进程是否带包身份（MSIX / sparse package）。
    pub fn has_package_identity() -> bool {
        let mut length = 0u32;
        // SAFETY: 首次调用只取长度；返回非“无包”时按文档再查缓冲。
        let status = unsafe { GetCurrentPackageFullName(&mut length, null_mut()) };
        status != APPMODEL_ERROR_NO_PACKAGE && length > 0
    }

    fn launcher_path() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let candidate = exe.parent()?.join("lorekeeper-activate.exe");
        candidate.is_file().then_some(candidate)
    }

    /// 拉起同目录身份启动器；成功返回 true。启动器会注册身份并以包身份拉起主程序。
    pub fn relaunch_through_launcher() -> bool {
        let Some(launcher) = launcher_path() else {
            return false;
        };
        std::process::Command::new(launcher)
            .arg("--quiet")
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .is_ok()
    }

    pub fn notice() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND,
        };
        let text: Vec<u16> = "LoreKeeper 需要通过桌面或开始菜单快捷方式启动。\n\n\
            Windows 会先注册应用身份再打开主程序；直接运行安装目录里的 exe 会失去这个身份。\n\n\
            Start LoreKeeper from its shortcut so Windows can register the app identity first."
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let title: Vec<u16> = "LoreKeeper"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: 两个字符串在调用期间有效；父窗口为空，属应用自身提示。
        unsafe {
            MessageBoxW(
                null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND,
            );
        }
    }
}

/// 启动门禁入口。返回 false 表示调用方应退出。
#[cfg(windows)]
pub fn ensure_packaged_start() -> bool {
    let probe = std::env::var_os("LOREKEEPER_PACKAGED_LAUNCH_PROBE").is_some();
    let allowed = std::env::var_os("LOREKEEPER_ALLOW_UNPACKAGED").is_some();
    let has_package = win::has_package_identity();
    // 探针优先于 debug 豁免：让开发构建也能验证真实包身份检测，且不弹窗、不拉起。
    if probe {
        std::process::exit(if has_package { 0 } else { PROBE_EXIT_CODE });
    }
    if !should_block(has_package, cfg!(debug_assertions), allowed) {
        return true;
    }
    if win::relaunch_through_launcher() {
        return false;
    }
    win::notice();
    false
}

/// 非 Windows 平台不做包身份检查。
#[cfg(not(windows))]
pub fn ensure_packaged_start() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_only_unpackaged_release_launches() {
        assert!(should_block(false, false, false));
        assert!(!should_block(true, false, false));
        assert!(!should_block(false, true, false), "调试构建必须放行");
        assert!(!should_block(false, false, true), "显式豁免必须放行");
    }
}
