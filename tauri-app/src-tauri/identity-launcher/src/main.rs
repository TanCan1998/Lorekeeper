//! LoreKeeper 身份启动器（发布构建无控制台窗口）。
//!
//! 两个模式，均不接受命令行参数：
//! - 包内（有包身份）：校验安装记录与文件后，用公开桌面应用进程策略启动真实主程序，
//!   让主程序与全部 WebView2 进程留在同一应用组；等待并透传退出码。
//! - 包外（无包身份）：只激活已注册的身份包（`shell:AppsFolder\<AUMID>`），
//!   绝不无身份启动主程序，避免“看起来启动了但不归组”。
//!
//! 不做的事：不改令牌、不关沙箱、不加浏览器诊断参数、不复制/改口径运行库、
//! 不按命令行传入的任意路径启动。

#![cfg_attr(windows, windows_subsystem = "windows")]

mod record;
#[cfg(windows)]
mod win;

fn main() {
    std::process::exit(run());
}

#[cfg(windows)]
fn run() -> i32 {
    if std::env::args_os().len() > 1 {
        win::report("身份启动器不接受命令行参数");
        return 2;
    }
    match win::current_package_full_name() {
        Some(full_name) => {
            if !full_name.starts_with(concat!("com.tancan1998.lorekeeper", "_")) {
                win::report("当前包身份不属于 LoreKeeper，已拒绝启动");
                return 3;
            }
            let record = match win::read_install_record() {
                Ok(record) => record,
                Err(error) => {
                    win::report(&error.to_string());
                    return 4;
                }
            };
            if let Err(error) = win::verify_files(&record) {
                win::report(&error.to_string());
                return 5;
            }
            match win::launch_with_identity(&record) {
                // 退出码按无符号读取，转成进程退出码范围。
                Ok(code) => code.min(i32::MAX as u32) as i32,
                Err(error) => {
                    win::report(&error.to_string());
                    6
                }
            }
        }
        None => match win::activate_identity_package() {
            Ok(_) => 0,
            Err(error) => {
                win::report(&error.to_string());
                7
            }
        },
    }
}

#[cfg(not(windows))]
fn run() -> i32 {
    eprintln!("Error: 身份启动器仅支持 Windows");
    1
}
