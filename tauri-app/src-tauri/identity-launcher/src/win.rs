//! Windows 侧实现：注册表安装记录、包身份检查、带身份启动与包激活。
//!
//! 只用官方 `windows-sys` 与应用模型公开 API；不修改令牌、不关闭沙箱、
//! 不注入浏览器诊断参数、不改共享 WebView2。

use crate::record::{
    quote_command_line, validate, InstallRecord, InstallScope, PACKAGE_NAME, RECORD_SCHEMA_VERSION,
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
    RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
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
/// 候选同时带 PowerShell 可用的蜂巢前缀，注册助手据此写回同一位置。
pub const RECORD_CANDIDATES: [(HKEY, &str, InstallScope, &str); 4] = [
    (HKEY_LOCAL_MACHINE, RECORD_SUBKEY, InstallScope::Machine, "HKLM"),
    (
        HKEY_LOCAL_MACHINE,
        r"Software\WOW6432Node\tancan1998\LoreKeeper",
        InstallScope::Machine,
        "HKLM",
    ),
    (HKEY_CURRENT_USER, RECORD_SUBKEY, InstallScope::User, "HKCU"),
    (
        HKEY_CURRENT_USER,
        r"Software\WOW6432Node\tancan1998\LoreKeeper",
        InstallScope::User,
        "HKCU",
    ),
];

/// 命中记录的位置：注册表蜂巢前缀 + 子键，用于把结果写回同一处。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordLocation {
    pub hive: &'static str,
    pub subkey: &'static str,
}
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
    let identity_launcher = read_string(hive, subkey_path, "IdentityLauncherPath");
    let record = InstallRecord {
        schema_version,
        scope: declared,
        identity_launcher,
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
    locate_install_record().map(|located| located.0)
}

/// 同一次读取同时返回记录与位置，避免写回时猜错蜂巢。
pub fn locate_install_record() -> Result<(InstallRecord, RecordLocation), LauncherError> {
    let mut first_error = None;
    for (hive, subkey, scope, hive_name) in RECORD_CANDIDATES {
        match record_from(hive, subkey, scope) {
            Ok(record) => {
                return Ok((
                    record,
                    RecordLocation {
                        hive: hive_name,
                        subkey,
                    },
                ))
            }
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

/// 登记在记录里的包内启动器路径：必须是 WindowsApps 下本产品的启动器。
fn packaged_launcher_path() -> Result<String, LauncherError> {
    let record = read_install_record()?;
    let launcher = record
        .identity_launcher
        .ok_or(LauncherError::PackageMissing)?;
    let lower = launcher.to_ascii_lowercase();
    // 未签名包不出现在应用列表，只能直接运行包内 EXE；路径必须落在 WindowsApps 内。
    if !lower.starts_with(r"c:\program files\windowsapps\com.tancan1998.lorekeeper_")
        && !lower.starts_with(r"c:\program files\windowsapps\com.tancan1998.lorekeeper_1")
    {
        return Err(LauncherError::RecordInvalid(
            "登记的身份启动器路径不在 WindowsApps 内".to_string(),
        ));
    }
    if !launcher.to_ascii_lowercase().ends_with("\\lorekeeper-identity-launcher.exe") {
        return Err(LauncherError::RecordInvalid(
            "登记的身份启动器文件名不符".to_string(),
        ));
    }
    attributes(&launcher).ok_or_else(|| LauncherError::UnsafeFile("身份启动器不存在".to_string()))?;
    Ok(launcher)
}

/// 包外模式：只转交给包内启动器以取得真实包身份，绝不无身份启动主程序。
pub fn activate_identity_package() -> Result<String, LauncherError> {
    let launcher = packaged_launcher_path()?;
    let mut child = std::process::Command::new(&launcher)
        .spawn()
        .map_err(|error| LauncherError::RecordInvalid(format!("无法启动身份启动器：{error}")))?;
    let code = child
        .wait()
        .map_err(|error| LauncherError::RecordInvalid(format!("等待身份启动器失败：{error}")))?;
    Ok(format!("{launcher} (exit {})", code.code().unwrap_or(-1)))
}

/// 是否已提权（用于判断是否需要自我提升）。
pub fn is_elevated() -> bool {
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let mut token = std::ptr::null_mut();
    // SAFETY: 只查询当前进程令牌的提权状态，句柄在此关闭。
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut size = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

/// 提权注册助手：置备到全机（多用户）+ 为当前用户注册，并登记包内启动器路径。
///
/// 未签名身份包必须提权才能注册（本机实测 0x80073D2B），因此这里只在已提权时执行；
/// 未提权由调用方负责显式提升，绝不静默伪装成功。
pub fn register_identity() -> Result<String, LauncherError> {
    // 记录可能在全机（MSI）或当前用户（NSIS）蜂巢，写回必须落在同一处。
    let (record, location) = locate_install_record()?;
    let msix = format!(
        "{}\\identity\\LoreKeeper.msix",
        record.install_location.trim_end_matches(['\\', '/'])
    );
    if attributes(&msix).is_none() {
        return Err(LauncherError::UnsafeFile("安装包内缺少身份组件".to_string()));
    }
    if !is_elevated() {
        return Err(LauncherError::RecordInvalid(
            "注册身份组件需要管理员权限".to_string(),
        ));
    }
    let key_path = format!("{}:\\{}", location.hive, location.subkey);
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         try {{ Add-AppxProvisionedPackage -Online -PackagePath '{msix}' -SkipLicense | Out-Null }} catch {{ }}; \
         Add-AppxPackage -Path '{msix}' -AllowUnsigned -ErrorAction Stop; \
         $p=Get-AppxPackage -Name {PACKAGE_NAME}; \
         if(-not $p){{ throw '注册后仍找不到身份包' }}; \
         $launcher=Join-Path $p.InstallLocation 'lorekeeper-identity-launcher.exe'; \
         if(-not (Test-Path $launcher)){{ throw '身份包内缺少启动器' }}; \
         if(-not (Test-Path '{key_path}')){{ New-Item -Path '{key_path}' -Force | Out-Null }}; \
         Set-ItemProperty -Path '{key_path}' -Name 'IdentityLauncherPath' -Value $launcher; \
         [Console]::Out.Write($launcher)"
    );
    let windir = std::env::var_os("WINDIR")
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| r"C:\Windows".to_string());
    let output = std::process::Command::new(format!(r"{windir}\System32\WindowsPowerShell\v1.0\powershell.exe"))
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .output()
        .map_err(|error| LauncherError::RecordInvalid(format!("无法调用 PowerShell：{error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf16_lossy(
            &String::from_utf8_lossy(&output.stderr)
                .encode_utf16()
                .collect::<Vec<u16>>(),
        );
        return Err(LauncherError::RecordInvalid(format!(
            "注册身份组件失败：{}",
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// 卸载助手：移除当前用户注册与全机置备，并清除记录中的启动器路径。
pub fn unregister_identity() -> Result<(), LauncherError> {
    let (_, location) = locate_install_record()?;
    if !is_elevated() {
        return Err(LauncherError::RecordInvalid(
            "注销身份组件需要管理员权限".to_string(),
        ));
    }
    let key_path = format!("{}:\\{}", location.hive, location.subkey);
    let script = format!(
        "$ErrorActionPreference='Continue'; \
         Get-AppxPackage -Name {PACKAGE_NAME} | Remove-AppxPackage -ErrorAction SilentlyContinue; \
         try {{ Get-AppxProvisionedPackage -Online | Where-Object {{ $_.DisplayName -eq '{PACKAGE_NAME}' }} | Remove-AppxProvisionedPackage -Online -ErrorAction SilentlyContinue | Out-Null }} catch {{ }}; \
         if(Test-Path '{key_path}') {{ Remove-ItemProperty -Path '{key_path}' -Name 'IdentityLauncherPath' -ErrorAction SilentlyContinue }}; \
         [Console]::Out.Write('unregistered')"
    );
    let windir = std::env::var_os("WINDIR")
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| r"C:\Windows".to_string());
    let output = std::process::Command::new(format!(r"{windir}\System32\WindowsPowerShell\v1.0\powershell.exe"))
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .output()
        .map_err(|error| LauncherError::RecordInvalid(format!("无法调用 PowerShell：{error}")))?;
    if !output.status.success() {
        return Err(LauncherError::RecordInvalid(
            "注销身份组件失败".to_string(),
        ));
    }
    Ok(())
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
