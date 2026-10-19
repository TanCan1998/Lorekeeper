//! 安装记录与包标识的纯逻辑：解析、校验、命令行引号。
//!
//! 这里刻意不碰 Win32：所有条目校验都能在任一平台单测，
//! 注册表/文件系统读取与进程创建在 `win` 模块完成。

/// 安装记录结构版本；升级记录格式必须同步提升并写迁移，不能默默复用旧值。
pub const RECORD_SCHEMA_VERSION: u32 = 1;
/// 记录中的包名必须与 tauri.conf.json 的 identifier 一致。
pub const PACKAGE_NAME: &str = "com.tancan1998.lorekeeper";
/// 安装目录中真正的主程序文件名。
pub const MAIN_EXECUTABLE_NAME: &str = "lorekeeper.exe";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallScope {
    Machine,
    User,
}

impl InstallScope {
    pub fn parse(value: &str) -> Result<Self, RecordError> {
        match value {
            "machine" => Ok(Self::Machine),
            "user" => Ok(Self::User),
            _ => Err(RecordError::UnknownScope),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Machine => "machine",
            Self::User => "user",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallRecord {
    pub schema_version: u32,
    pub scope: InstallScope,
    pub package_name: String,
    pub install_location: String,
    pub main_executable: String,
    /// 安装器登记的包内启动器路径（WindowsApps 内）；包外模式据此转交身份。
    pub identity_launcher: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordError {
    MissingValue(&'static str),
    UnsupportedSchema,
    UnknownScope,
    PackageMismatch,
    NotAbsolute,
    UnsafeCharacter,
    ParentTraversal,
    UnexpectedExecutable,
    OutsideInstallLocation,
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingValue(name) => write!(f, "安装记录缺少 {name}"),
            Self::UnsupportedSchema => f.write_str("安装记录版本不受支持，请重新安装 LoreKeeper"),
            Self::UnknownScope => f.write_str("安装记录的安装范围无效"),
            Self::PackageMismatch => f.write_str("安装记录属于其他应用，拒绝启动"),
            Self::NotAbsolute => f.write_str("安装记录中的路径必须是绝对路径"),
            Self::UnsafeCharacter => f.write_str("安装记录中的路径含不允许的字符"),
            Self::ParentTraversal => f.write_str("安装记录中的路径含上级目录片段，拒绝启动"),
            Self::UnexpectedExecutable => {
                write!(f, "主程序必须是 {MAIN_EXECUTABLE_NAME}")
            }
            Self::OutsideInstallLocation => f.write_str("主程序不在记录声明的安装目录内"),
        }
    }
}

impl std::error::Error for RecordError {}

/// Windows 绝对路径判定：盘符路径、UNC 与 `\\?\` 前缀形式。
fn is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    if path.starts_with(r"\\?\") || path.starts_with(r"\\.\") {
        // 设备路径必须紧跟盘符或 UNC，不能只留前缀。
        let rest = &path[4..];
        return rest.len() > 2 && (rest.as_bytes()[1] == b':' || rest.starts_with(r"\"));
    }
    if let Some(rest) = path.strip_prefix(r"\\") {
        return rest.len() > 1 && !rest.starts_with('\\');
    }
    bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/')
}

fn has_unsafe_character(path: &str) -> bool {
    path.chars()
        .any(|ch| ch == '"' || ch == '\0' || (ch.is_control() && ch != '\t'))
}

fn has_parent_traversal(path: &str) -> bool {
    path.split(['\\', '/']).any(|segment| segment == "..")
}

/// 去掉末尾分隔符但保留根，便于比较目录。
fn normalize_dir(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['\\', '/']);
    if trimmed.is_empty() || trimmed.ends_with(':') {
        path
    } else {
        trimmed
    }
}

fn file_name_of(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['\\', '/']);
    match trimmed.rfind(['\\', '/']) {
        Some(index) => &trimmed[index + 1..],
        None => trimmed,
    }
}

fn parent_of(path: &str) -> Option<&str> {
    let trimmed = path.trim_end_matches(['\\', '/']);
    let index = trimmed.rfind(['\\', '/'])?;
    let parent = &trimmed[..index];
    if parent.is_empty() || parent.ends_with(':') {
        None
    } else {
        Some(parent)
    }
}

/// 校验记录自洽且指向本应用的主程序；不做文件系统访问。
pub fn validate(record: &InstallRecord) -> Result<(), RecordError> {
    if record.schema_version != RECORD_SCHEMA_VERSION {
        return Err(RecordError::UnsupportedSchema);
    }
    if record.package_name != PACKAGE_NAME {
        return Err(RecordError::PackageMismatch);
    }
    for (name, value) in [
        ("InstallLocation", record.install_location.as_str()),
        ("MainExecutable", record.main_executable.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(RecordError::MissingValue(name));
        }
        if has_unsafe_character(value) {
            return Err(RecordError::UnsafeCharacter);
        }
        if has_parent_traversal(value) {
            return Err(RecordError::ParentTraversal);
        }
        if !is_absolute(value) {
            return Err(RecordError::NotAbsolute);
        }
    }
    if !file_name_of(&record.main_executable).eq_ignore_ascii_case(MAIN_EXECUTABLE_NAME) {
        return Err(RecordError::UnexpectedExecutable);
    }
    let install_location = normalize_dir(&record.install_location);
    match parent_of(&record.main_executable) {
        Some(parent) if normalize_dir(parent).eq_ignore_ascii_case(install_location) => Ok(()),
        _ => Err(RecordError::OutsideInstallLocation),
    }
}

/// 与清单生成器一致：只接受四段、无前导零的 UInt16 数字版本。
pub fn valid_package_version(value: &str) -> bool {
    let mut count = 0;
    for part in value.split('.') {
        if part.is_empty()
            || !part.bytes().all(|byte| byte.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
            || part.parse::<u16>().is_err()
        {
            return false;
        }
        count += 1;
    }
    count == 4
}

/// 只解析本产品的完整主包名；不从包族名或不完整字段猜版本。
fn version_from_package_full_name(full_name: &str) -> Option<&str> {
    let mut fields = full_name.split('_');
    let name = fields.next()?;
    let version = fields.next()?;
    let architecture = fields.next()?;
    let resource = fields.next()?;
    let publisher_id = fields.next()?;
    if fields.next().is_some()
        || name != PACKAGE_NAME
        || !valid_package_version(version)
        || !matches!(architecture, "x64" | "arm64")
        || !resource.is_empty()
        || publisher_id.len() != 13
        || !publisher_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(version)
}

/// 版本必须精确匹配本次构建；旧包存在/已注册不代表升级已完成。
pub fn package_matches_version(full_name: &str, expected_version: &str) -> bool {
    valid_package_version(expected_version)
        && version_from_package_full_name(full_name) == Some(expected_version)
}

/// 从 WindowsApps 内的固定启动器路径取完整包名（父目录名）。
///
/// 仅接受当前部署使用的系统目录，不接受名称相似的目录、额外子目录或任意 EXE。
pub fn package_full_name_from_launcher(path: &str) -> Option<String> {
    let (parent, executable) = path.rsplit_once('\\')?;
    let (root, full_name) = parent.rsplit_once('\\')?;
    if !root.eq_ignore_ascii_case(r"C:\Program Files\WindowsApps")
        || !executable.eq_ignore_ascii_case("lorekeeper-identity-launcher.exe")
    {
        return None;
    }
    version_from_package_full_name(full_name)?;
    Some(full_name.to_string())
}

/// PowerShell 单引号字面量：自选目录中的单引号必须加倍，不拼接可执行脚本。
pub fn quote_powershell_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// 命令行必须是可写缓冲：Windows 允许 CreateProcess 就地修改该字符串。
pub fn quote_command_line(executable: &str) -> Result<String, RecordError> {
    if executable.contains('"') || executable.contains('\0') {
        return Err(RecordError::UnsafeCharacter);
    }
    Ok(format!("\"{executable}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> InstallRecord {
        InstallRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            scope: InstallScope::Machine,
            package_name: PACKAGE_NAME.to_string(),
            install_location: r"C:\Program Files\LoreKeeper".to_string(),
            main_executable: r"C:\Program Files\LoreKeeper\lorekeeper.exe".to_string(),
            identity_launcher: None,
        }
    }

    #[test]
    fn accepts_machine_and_user_records() {
        assert_eq!(validate(&record()), Ok(()));
        let mut user = record();
        user.scope = InstallScope::User;
        assert_eq!(validate(&user), Ok(()));
        assert_eq!(InstallScope::parse("machine"), Ok(InstallScope::Machine));
        assert_eq!(InstallScope::parse("user"), Ok(InstallScope::User));
        assert_eq!(InstallScope::parse("all"), Err(RecordError::UnknownScope));
        assert_eq!(InstallScope::Machine.as_str(), "machine");
    }

    #[test]
    fn accepts_trailing_separator_and_case_insensitive_directory() {
        let mut value = record();
        value.install_location = r"C:\PROGRAM FILES\lorekeeper\\".to_string();
        value.main_executable = r"c:\program files\LoreKeeper\LOREKEEPER.EXE".to_string();
        assert_eq!(validate(&value), Ok(()));
    }

    #[test]
    fn accepts_long_path_and_unc_installs() {
        let mut long = record();
        long.install_location = r"\\?\D:\小说\LoreKeeper".to_string();
        long.main_executable = r"\\?\D:\小说\LoreKeeper\lorekeeper.exe".to_string();
        assert_eq!(validate(&long), Ok(()));

        let mut unc = record();
        unc.install_location = r"\\server\share\LoreKeeper".to_string();
        unc.main_executable = r"\\server\share\LoreKeeper\lorekeeper.exe".to_string();
        assert_eq!(validate(&unc), Ok(()));
    }

    #[test]
    fn rejects_schema_drift_and_other_packages() {
        let mut version = record();
        version.schema_version = 2;
        assert_eq!(validate(&version), Err(RecordError::UnsupportedSchema));
        let mut package = record();
        package.package_name = "com.other.app".to_string();
        assert_eq!(validate(&package), Err(RecordError::PackageMismatch));
    }

    #[test]
    fn rejects_missing_relative_and_traversing_paths() {
        let mut empty = record();
        empty.main_executable = "   ".to_string();
        assert_eq!(validate(&empty), Err(RecordError::MissingValue("MainExecutable")));

        let mut relative = record();
        relative.install_location = r"LoreKeeper".to_string();
        relative.main_executable = r"LoreKeeper\lorekeeper.exe".to_string();
        assert_eq!(validate(&relative), Err(RecordError::NotAbsolute));

        let mut traversal = record();
        traversal.main_executable = r"C:\Program Files\LoreKeeper\..\evil\lorekeeper.exe".to_string();
        assert_eq!(validate(&traversal), Err(RecordError::ParentTraversal));

        // 盘符根目录没有可用的安装目录父级，不能把主程序直接放在卷根。
        let mut drive_only = record();
        drive_only.main_executable = r"C:\lorekeeper.exe".to_string();
        drive_only.install_location = r"C:\".to_string();
        assert_eq!(validate(&drive_only), Err(RecordError::OutsideInstallLocation));
    }

    #[test]
    fn rejects_quoting_and_control_characters() {
        let mut quoted = record();
        quoted.main_executable = "C:\\Program Files\\LoreKeeper\\\"evil.exe".to_string();
        assert_eq!(validate(&quoted), Err(RecordError::UnsafeCharacter));

        let mut control = record();
        control.install_location = "C:\\Program Files\\Lore\u{7f}Keeper".to_string();
        assert_eq!(validate(&control), Err(RecordError::UnsafeCharacter));
    }

    #[test]
    fn rejects_executable_not_named_lorekeeper_or_outside_directory() {
        let mut other = record();
        other.main_executable = r"C:\Program Files\LoreKeeper\other.exe".to_string();
        assert_eq!(validate(&other), Err(RecordError::UnexpectedExecutable));

        let mut elsewhere = record();
        elsewhere.main_executable = r"C:\Program Files\Other\lorekeeper.exe".to_string();
        assert_eq!(validate(&elsewhere), Err(RecordError::OutsideInstallLocation));
    }

    #[test]
    fn derives_package_full_name_only_from_windowsapps_launcher() {
        let path = r"C:\Program Files\WindowsApps\com.tancan1998.lorekeeper_1.0.1.0_x64__e8hwe6njnc7fy\lorekeeper-identity-launcher.exe";
        assert_eq!(
            package_full_name_from_launcher(path),
            Some("com.tancan1998.lorekeeper_1.0.1.0_x64__e8hwe6njnc7fy".to_string())
        );
        assert_eq!(package_full_name_from_launcher(r"D:\Temp\LoreKeeper\lorekeeper.exe"), None);
        assert_eq!(
            package_full_name_from_launcher(r"C:\Program Files\WindowsApps\other_1.0.0.0_x64__abc\x.exe"),
            None
        );
    }

    #[test]
    fn validates_numeric_package_versions_without_semver_fallback() {
        for version in ["0.0.0.0", "1.0.1.0", "65535.65535.65535.65535"] {
            assert!(valid_package_version(version), "{version}");
        }
        for version in [
            "",
            "1.0.0",
            "1.0.0.0.0",
            "1.0.0-beta.1",
            "01.0.0.0",
            "1.0.0.65536",
            "1.0.-1.0",
            "1.0.0.+1",
            "1.0.0. 1",
            "1.0.0.１",
        ] {
            assert!(!valid_package_version(version), "{version}");
        }
    }

    #[test]
    fn requires_exact_version_even_when_an_old_or_newer_package_is_present() {
        let full_name = "com.tancan1998.lorekeeper_1.0.1.0_x64__e8hwe6njnc7fy";
        assert!(package_matches_version(full_name, "1.0.1.0"));
        assert!(!package_matches_version(full_name, "1.0.2.0"));
        assert!(!package_matches_version(full_name, "1.0.0.0"));
        assert!(!package_matches_version(full_name, "1.0.1"));
        assert!(!package_matches_version(
            "com.other.app_1.0.1.0_x64__e8hwe6njnc7fy",
            "1.0.1.0"
        ));
        assert!(!package_matches_version(
            "com.tancan1998.lorekeeper_1.0.1.0_x64__e8hwe6njnc7fy_extra",
            "1.0.1.0"
        ));
    }

    #[test]
    fn rejects_similar_windowsapps_directories_and_malformed_package_paths() {
        for path in [
            r"C:\Program Files\WindowsAppsBackup\com.tancan1998.lorekeeper_1.0.1.0_x64__e8hwe6njnc7fy\lorekeeper-identity-launcher.exe",
            r"C:\Program Files\WindowsApps\com.tancan1998.lorekeeper_1.0.1.0_x64__e8hwe6njnc7fy\other.exe",
            r"C:\Program Files\WindowsApps\com.tancan1998.lorekeeper_1.0.1.0_x64__e8hwe6njnc7fy\extra\lorekeeper-identity-launcher.exe",
            r"C:\Program Files\WindowsApps\com.tancan1998.lorekeeper_bad_x64__e8hwe6njnc7fy\lorekeeper-identity-launcher.exe",
            r"C:\Program Files\WindowsApps\com.tancan1998.lorekeeper_1.0.1.0_x86__e8hwe6njnc7fy\lorekeeper-identity-launcher.exe",
            r"C:\Program Files\WindowsApps\com.tancan1998.lorekeeper_1.0.1.0_x64_resource_e8hwe6njnc7fy\lorekeeper-identity-launcher.exe",
        ] {
            assert_eq!(package_full_name_from_launcher(path), None, "{path}");
        }
    }

    #[test]
    fn quotes_powershell_literals_without_expanding_user_paths() {
        assert_eq!(
            quote_powershell_literal(r"D:\作者's [草稿]\LoreKeeper"),
            r"'D:\作者''s [草稿]\LoreKeeper'"
        );
        assert_eq!(
            quote_powershell_literal("$env:TEMP; exit 1"),
            "'$env:TEMP; exit 1'"
        );
    }

    #[test]
    fn quotes_absolute_path_with_spaces_and_rejects_quotes() {
        assert_eq!(
            quote_command_line(r"C:\Program Files\LoreKeeper\lorekeeper.exe"),
            Ok(r#""C:\Program Files\LoreKeeper\lorekeeper.exe""#.to_string())
        );
        assert_eq!(quote_command_line("a\"b"), Err(RecordError::UnsafeCharacter));
    }

    #[test]
    fn errors_are_human_readable_chinese() {
        for error in [
            RecordError::MissingValue("MainExecutable"),
            RecordError::UnsupportedSchema,
            RecordError::UnknownScope,
            RecordError::PackageMismatch,
            RecordError::NotAbsolute,
            RecordError::UnsafeCharacter,
            RecordError::ParentTraversal,
            RecordError::UnexpectedExecutable,
            RecordError::OutsideInstallLocation,
        ] {
            let text = error.to_string();
            assert!(!text.is_empty());
            assert!(!text.is_ascii(), "错误文案应为中文：{text}");
        }
    }
}
