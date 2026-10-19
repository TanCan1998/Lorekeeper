# 当前交接入口

- 最新：[第118次：真实产品归组与提权注册闭环](./2026-10-19-tauri-migration-status.md)。**真实产品已实测归组**；MSI 注册与多用户仍是缺口，发布继续暂停。
- 已实测（真实产品，不是探针）：提权静默安装到自选目录退出 0 → 身份包注册成功（`com.tancan1998.lorekeeper_1.0.1.0_x64__e8hwe6njnc7fy`，Status=0）、`IdentityLauncherPath` 指向 WindowsApps 内启动器；经激活器启动后主程序位于所选目录，**主程序与 6 个 WebView2 读回同一包身份**；提权静默卸载后目录、HKCU 记录、当前用户包、WindowsApps 包目录全部清零。
- 提权注册助手：`--register-identity`（非提权明确报错）→ `Add-AppxProvisionedPackage -SkipLicense`（全机置备）+ `Add-AppxPackage -AllowUnsigned`（当前用户）+ 回写 `IdentityLauncherPath`。NSIS `POSTINSTALL` 先直接尝试，失败且非静默时 `ExecShell "runas"` 调起助手；静默安装不弹 UAC 也不写假路径。
- 云 CI：`4c66847c` 的 Tauri Rust CI 因已声明的安装资源未预生成而失败（`resource path target\identity\LoreKeeper.msix doesn't exist`）；`76fec45` 已修复并 **success**（39186538173），同 SHA 的 Gitleaks 与提交信息检查亦 success。
- **仍未完成**：MSI 侧调用同一助手的自定义动作（安装记录已写 HKLM，注册未接）；第二个 Windows 用户下“置备后首次使用”未验证；升级/修复/回滚、最低系统、外部工具 breakaway、macOS 未验证；普通用户双击安装的 UAC 交互未人工确认。
- 合同：Node 38 项通过（含提权助手断言）、启动器单测 9/9、clippy 无告警。主 crate 与前端未改，Rust 云端全量基线仍 44259fa3/39148045359。
- 产品运行时代码基线仍 44259fa3（LoreKeeper / 1.0.0-beta.1、中文 NSIS/MSI、安全清理、identifier/数据根/UpgradeCode 不变）。无新 tag/Release/Latest；公开仍[旧 1.0.0-beta](https://github.com/TanCan1998/Lorekeeper/releases/tag/lorekeeper-v1.0.0-beta)。
- 禁止子代理、不抢焦点、不改 Electron/根构建及锁文件/上游 docs；证据见[研究§12](../research/webview-process-attribution.md#12-第118次真实产品归组与提权注册闭环)。
