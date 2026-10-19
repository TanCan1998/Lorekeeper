//! LoreKeeper 身份启动器（发布构建无控制台窗口）。
//!
//! 两个模式；`--quiet` 仅抑制错误弹窗（写日志），不改变启动语义：
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

/// 唯一允许的命令行开关：提权注册身份组件（安装器或用户显式触发）。
const REGISTER_IDENTITY: &str = "--register-identity";
/// 卸载时移除当前用户注册与全机置备（需要提权）。
const UNREGISTER_IDENTITY: &str = "--unregister-identity";
/// 静默模式：错误只写日志，不弹模态框（自动化与无人值守场景）。
const QUIET: &str = "--quiet";

fn main() {
    std::process::exit(run());
}

#[cfg(windows)]
fn run() -> i32 {
    let mut arguments: Vec<String> = Vec::new();
    let mut quiet = false;
    for argument in std::env::args_os().skip(1) {
        let argument = argument.to_string_lossy().into_owned();
        if argument == QUIET {
            quiet = true;
        } else {
            arguments.push(argument);
        }
    }
    win::set_quiet(quiet);
    if arguments.len() == 1 && arguments[0] == REGISTER_IDENTITY {
        return match win::register_identity() {
            Ok(_) => 0,
            Err(error) => {
                win::report(&error.to_string());
                8
            }
        };
    }
    if arguments.len() == 1 && arguments[0] == UNREGISTER_IDENTITY {
        return match win::unregister_identity() {
            Ok(()) => 0,
            Err(error) => {
                win::report(&error.to_string());
                9
            }
        };
    }
    if !arguments.is_empty() {
        win::report("身份启动器只接受 --register-identity、--unregister-identity 或 --quiet");
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
