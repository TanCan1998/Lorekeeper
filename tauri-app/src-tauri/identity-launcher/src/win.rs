//! Windows 侧实现：注册表安装记录、包身份检查、带身份启动与包激活。
//!
//! 只用官方 `windows-sys` 与应用模型公开 API；不修改令牌、不关闭沙箱、
//! 不注入浏览器诊断参数、不改共享 WebView2。

use crate::record::{
    application_user_model_id, package_family_from_full_name, quote_command_line, validate,
    InstallRecord, InstallScope, PACKAGE_NAME, RECORD_SCHEMA_VERSION,
};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_SUCCESS};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileAttributesW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    INVALID_FILE_ATTRIBUTES,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, UpdateProcThreadAttribute, WaitForSingleObject,
    EXTENDED_STARTUPINFO_PRESENT, INFINITE, PROCESS_INFORMATION, STARTUPINFOEXW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

/// 安装记录所在的注册表子键（与 `tauri.conf.json` 的厂商/产品键一致）。
pub const RECORD_SUBKEY: &str = r"Software\tancan1998\LoreKeeper";
/// 读取候选：64 位native视图 + WOW6432Node + 当前用户两种视图。
/// 安装器可能是 32 位（NSIS）或 64 位 MSI，写入视图不同；这里按明确清单读取，
/// 不猜测、不搜索任意键。
pub const RECORD_CANDIDATES: [(HKEY, &str, InstallScope); 4] = [
    (HKEY_LOCAL_MACHINE, RECORD_SUBKEY, InstallScope::Machine),
    (
        HKEY_LOCAL_MACHINE,
        r"Software\WOW6432Node\tancan1998\LoreKeeper",
        InstallScope::Machine,
    ),
    (HKEY_CURRENT_USER, RECORD_SUBKEY, InstallScope::User),
    (
        HKEY_CURRENT_USER,
        r"Software\WOW6432Node\tancan1998\LoreKeeper",
        InstallScope::User,
    ),
];
/// 已安装应用模型包的仓库键；只读，用于包外模式定位包族名。
const PACKAGE_REPOSITORY: &str =
    r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";

// `PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY` = ProcThreadAttributeValue(18, FALSE, TRUE, FALSE)。
// 该常量由 Windows SDK 定义；windows-sys 未导出，此处按 SDK 取值声明。
const PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY: usize = 0x0002_0012;
/// `PROCESS_CREATION_DESKTOP_APP_BREAKAWAY_DISABLE_PROCESS_TREE`。
const POLICY_DISABLE_PROCESS_TREE: u32 = 2;
/// `APPMODEL_ERROR_NO_PACKAGE`。
const APPMODEL_ERROR_NO_PACKAGE: i32 = 15700;

#[link(name = "kernel32")]
extern "system" {
    /// 调用方无包身份时返回 `APPMODEL_ERROR_NO_PACKAGE`。
    fn GetCurrentPackageFullName(length: *mut u32, name: *mut u16) -> i32;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LauncherError {
    RecordMissing,
    RecordInvalid(String),
    UnsafeFile(String),
    PackageMissing,
    Win32 { step: &'static str, code: u32 },
}

impl std::fmt::Display for LauncherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RecordMissing => {
                f.write_str("找不到 LoreKeeper 安装记录，请重新安装或修复 LoreKeeper")
            }
            Self::RecordInvalid(detail) => write!(f, "安装记录无效：{detail}"),
            Self::UnsafeFile(detail) => write!(f, "主程序位置不安全：{detail}"),
            Self::PackageMissing => f.write_str("尚未注册 LoreKeeper 身份包，无法启动"),
            Self::Win32 { step, code } => write!(f, "{step} 失败（Win32 错误 {code}）"),
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub fn current_package_full_name() -> Option<String> {
    let mut length = 0u32;
    // SAFETY: 首次调用只取所需长度；返回非 APPMODEL_ERROR_NO_PACKAGE 时按文档查询缓冲。
    let status = unsafe { GetCurrentPackageFullName(&mut length, null_mut()) };
    if status == APPMODEL_ERROR_NO_PACKAGE || length == 0 {
        return None;
    }
    let mut buffer = vec![0u16; length as usize];
    // SAFETY: buffer 长度来自上一次调用返回的字符数。
    let status = unsafe { GetCurrentPackageFullName(&mut length, buffer.as_mut_ptr()) };
    if status != 0 {
        return None;
    }
    let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

fn read_string(hive: HKEY, subkey_path: &str, name: &str) -> Option<String> {
    let subkey = wide(subkey_path);
    let value = wide(name);
    let mut buffer = vec![0u16; 32_768];
    let mut size = (buffer.len() * std::mem::size_of::<u16>()) as u32;
    // SAFETY: 子键/值名为 NUL 结尾宽字符串，缓冲大小以字节给出。
    let status = unsafe {
        RegGetValueW(
            hive,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS || size == 0 {
        return None;
    }
    let units = (size as usize / std::mem::size_of::<u16>()).min(buffer.len());
    let end = buffer[..units]
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units);
    Some(String::from_utf16_lossy(&buffer[..end]))
}

fn read_dword(hive: HKEY, subkey_path: &str, name: &str) -> Option<u32> {
    let subkey = wide(subkey_path);
    let value = wide(name);
    let mut data = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: DWORD 输出缓冲指向栈上 u32，size 与实际类型一致。
    let status = unsafe {
        RegGetValueW(
            hive,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut data as *mut u32).cast(),
            &mut size,
        )
    };
    (status == ERROR_SUCCESS).then_some(data)
}

fn record_from(
    hive: HKEY,
    subkey_path: &str,
    scope: InstallScope,
) -> Result<InstallRecord, LauncherError> {
    let schema_version = read_dword(hive, subkey_path, "IdentityRecordSchema")
        .ok_or(LauncherError::RecordMissing)?;
    let install_location =
        read_string(hive, subkey_path, "InstallLocation").ok_or(LauncherError::RecordMissing)?;
    // 主程序名固定；安装器可省略该值，避免把目录属性拼接口径写进两条安装链。
    let main_executable = read_string(hive, subkey_path, "MainExecutable").unwrap_or_else(|| {
        format!(
            "{}\\{}",
            install_location.trim_end_matches(['\\', '/']),
            crate::record::MAIN_EXECUTABLE_NAME
        )
    });
    let package_name =
        read_string(hive, subkey_path, "IdentityPackageName").ok_or(LauncherError::RecordMissing)?;
    // 声明范围必须与实际命中的注册表位置一致，避免把用户级记录当成全机安装。
    let declared = InstallScope::parse(
        &read_string(hive, subkey_path, "InstallScope").ok_or(LauncherError::RecordMissing)?,
    )
    .map_err(|error| LauncherError::RecordInvalid(error.to_string()))?;
    if declared != scope {
        return Err(LauncherError::RecordInvalid(format!(
            "安装范围与注册表位置不一致（记录声明为 {}）",
            declared.as_str()
        )));
    }
    let record = InstallRecord {
        schema_version,
        scope: declared,
        package_name,
        install_location,
        main_executable,
    };
    validate(&record).map_err(|error| LauncherError::RecordInvalid(error.to_string()))?;
    if schema_version != RECORD_SCHEMA_VERSION {
        return Err(LauncherError::RecordInvalid(
            "安装记录版本不受支持".to_string(),
        ));
    }
    Ok(record)
}

/// 全机安装优先于当前用户安装；两者都缺失时明确失败，不回退猜测路径。
pub fn read_install_record() -> Result<InstallRecord, LauncherError> {
    let mut first_error = None;
    for (hive, subkey, scope) in RECORD_CANDIDATES {
        match record_from(hive, subkey, scope) {
            Ok(record) => return Ok(record),
            Err(LauncherError::RecordMissing) => continue,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    Err(first_error.unwrap_or(LauncherError::RecordMissing))
}

fn attributes(path: &str) -> Option<u32> {
    let wide_path = wide(path);
    // SAFETY: 入参为 NUL 结尾宽字符串。
    let value = unsafe { GetFileAttributesW(wide_path.as_ptr()) };
    (value != INVALID_FILE_ATTRIBUTES).then_some(value)
}

/// 拒绝链接/联接目录与缺失文件；安装目录与主程序都不能被重定向。
pub fn verify_files(record: &InstallRecord) -> Result<(), LauncherError> {
    let directory = attributes(&record.install_location)
        .ok_or_else(|| LauncherError::UnsafeFile("安装目录不存在".to_string()))?;
    if directory & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(LauncherError::UnsafeFile(
            "安装目录是链接或联接点".to_string(),
        ));
    }
    if directory & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(LauncherError::UnsafeFile("安装位置不是目录".to_string()));
    }
    let executable = attributes(&record.main_executable)
        .ok_or_else(|| LauncherError::UnsafeFile("主程序不存在".to_string()))?;
    if executable & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(LauncherError::UnsafeFile(
            "主程序是链接或联接点".to_string(),
        ));
    }
    if executable & FILE_ATTRIBUTE_DIRECTORY != 0 {
        return Err(LauncherError::UnsafeFile("主程序不是文件".to_string()));
    }
    Ok(())
}

/// 带包身份启动主程序：显式保留桌面应用运行环境，随后等待并透传退出码。
pub fn launch_with_identity(record: &InstallRecord) -> Result<u32, LauncherError> {
    let mut command_line = wide(
        &quote_command_line(&record.main_executable)
            .map_err(|error| LauncherError::RecordInvalid(error.to_string()))?,
    );
    let application = wide(&record.main_executable);
    let working_directory = wide(&record.install_location);
    let mut policy = POLICY_DISABLE_PROCESS_TREE;

    let mut attribute_size = 0usize;
    // SAFETY: 首次调用用于查询所需字节数，返回 FALSE 且设置 ERROR_INSUFFICIENT_BUFFER 属预期。
    unsafe {
        InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut attribute_size);
    }
    let slots = attribute_size.div_ceil(std::mem::size_of::<usize>());
    let mut attribute_buffer = vec![0usize; slots.max(1)];
    // SAFETY: 缓冲区按 usize 对齐且足以容纳 attribute_size 字节。
    let initialized = unsafe {
        InitializeProcThreadAttributeList(
            attribute_buffer.as_mut_ptr().cast(),
            1,
            0,
            &mut attribute_size,
        )
    };
    if initialized == 0 {
        // SAFETY: 仅读取当前线程的最后错误码。
        return Err(LauncherError::Win32 {
            step: "初始化进程属性列表",
            code: unsafe { GetLastError() },
        });
    }

    struct Attributes(*mut std::ffi::c_void);
    impl Drop for Attributes {
        fn drop(&mut self) {
            // SAFETY: 指针来自成功初始化的属性列表，且只在此处释放一次。
            unsafe { DeleteProcThreadAttributeList(self.0.cast()) };
        }
    }
    let attributes = Attributes(attribute_buffer.as_mut_ptr().cast());

    // SAFETY: 策略值指向栈上 u32，生命周期覆盖 CreateProcessW 调用。
    let updated = unsafe {
        UpdateProcThreadAttribute(
            attributes.0,
            0,
            PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY,
            (&mut policy as *mut u32).cast(),
            std::mem::size_of::<u32>(),
            null_mut(),
            null_mut(),
        )
    };
    if updated == 0 {
        // SAFETY: 仅读取当前线程的最后错误码。
        return Err(LauncherError::Win32 {
            step: "设置桌面应用进程策略",
            code: unsafe { GetLastError() },
        });
    }

    // SAFETY: 结构体为零初始化后按文档填写 cb 与属性列表指针。
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.lpAttributeList = attributes.0;
    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    // SAFETY: 所有指针在调用期间有效；不继承句柄、不传自定义环境块。
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT,
            null(),
            working_directory.as_ptr(),
            &startup.StartupInfo,
            &mut process,
        )
    };
    if created == 0 {
        // SAFETY: 仅读取当前线程的最后错误码。
        return Err(LauncherError::Win32 {
            step: "启动主程序",
            code: unsafe { GetLastError() },
        });
    }

    // SAFETY: 两个句柄由 CreateProcessW 输出，且在此后不再使用。
    unsafe {
        CloseHandle(process.hThread);
        WaitForSingleObject(process.hProcess, INFINITE);
    }
    let mut exit_code = 0u32;
    // SAFETY: 进程句柄在关闭前有效。
    let read = unsafe { GetExitCodeProcess(process.hProcess, &mut exit_code) };
    // SAFETY: 同上，主进程句柄在此关闭。
    unsafe { CloseHandle(process.hProcess) };
    if read == 0 {
        // SAFETY: 仅读取当前线程的最后错误码。
        return Err(LauncherError::Win32 {
            step: "读取主程序退出码",
            code: unsafe { GetLastError() },
        });
    }
    Ok(exit_code)
}

fn installed_package_full_names() -> Vec<String> {
    let subkey = wide(PACKAGE_REPOSITORY);
    let mut key: HKEY = null_mut();
    // SAFETY: 只读打开仓库键；失败时返回空集合，由调用方报明确错误。
    let opened = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut key) };
    if opened != ERROR_SUCCESS {
        return Vec::new();
    }
    let mut names = Vec::new();
    let mut index = 0u32;
    loop {
        let mut buffer = vec![0u16; 512];
        let mut length = buffer.len() as u32;
        // SAFETY: 缓冲与长度按文档传入；不再有子键时返回 ERROR_NO_MORE_ITEMS。
        let status = unsafe {
            RegEnumKeyExW(
                key,
                index,
                buffer.as_mut_ptr(),
                &mut length,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            break;
        }
        names.push(String::from_utf16_lossy(&buffer[..length as usize]));
        index += 1;
    }
    // SAFETY: 句柄由 RegOpenKeyExW 成功返回。
    unsafe { RegCloseKey(key) };
    names
}

/// 包外模式：只激活已注册的身份包，绝不无身份启动主程序。
pub fn activate_identity_package() -> Result<String, LauncherError> {
    let family = installed_package_full_names()
        .iter()
        .filter(|name| name.starts_with(&format!("{PACKAGE_NAME}_")))
        .filter_map(|name| package_family_from_full_name(name))
        .next()
        .ok_or(LauncherError::PackageMissing)?;
    let aumid = application_user_model_id(&family);
    let windir = std::env::var_os("WINDIR")
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| r"C:\Windows".to_string());
    let explorer = format!(r"{windir}\explorer.exe");
    let target = format!("shell:AppsFolder\\{aumid}");
    std::process::Command::new(explorer)
        .arg(target)
        .spawn()
        .map_err(|error| LauncherError::RecordInvalid(format!("无法激活身份包：{error}")))?;
    Ok(aumid)
}

/// 启动失败时给出可见提示；正常启动不弹窗、不抢焦点。
pub fn report(message: &str) {
    eprintln!("Error: {message}");
    let text = wide(message);
    let title = wide("LoreKeeper");
    // SAFETY: 两个字符串均在调用期间有效；父窗口为空，属应用自身错误提示。
    unsafe {
        MessageBoxW(null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
    }
}
