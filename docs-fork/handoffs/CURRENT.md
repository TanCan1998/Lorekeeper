# 当前交接入口

- 最新：[第119次：MSI 全机注册/注销闭环与置备缺口](./2026-10-19-tauri-migration-status.md)。NSIS 与 MSI 两条安装链都已在真实产品上验证归组与清理；**多用户置备未生效**，发布继续暂停。
- 已实测（真实产品，不是探针）：
  - **NSIS**：提权静默安装到自选目录 → 注册成功、`IdentityLauncherPath` 就位；经激活器启动后主程序与 6 个 WebView2 读回同一包身份；静默卸载后目录、HKCU 记录、包、WindowsApps 目录全部清零。
  - **MSI**：`msiexec /i ... /qn INSTALLDIR=自选目录` 退出 0，`HKLM` 记录 `scope=machine` 且含启动器路径，安装后 `Get-AppxPackage` 计数 1；`msiexec /x ... /qn` 退出 0，目录/HKLM 记录/包全部清零。WiX 自定义动作在 `InstallFinalize` 后注册、卸载前注销。
- **多用户仍不成立**：`Add-AppxProvisionedPackage -SkipLicense` 未生效（安装后 `provisioned=0`）。不能把“全机安装”写成“所有用户已获得身份”；第二个用户需各自提权注册，或改用可签名身份包。
- 升级路径：自定义动作以 `Installed AND NOT UPGRADINGPRODUCTCODE` 保护，升级不注销身份；但升级后包版本变化会让 `IdentityLauncherPath` 指向旧版本目录，**升级后重新注册/路径刷新尚未实现**。
- 提权助手：`--register-identity`（非提权明确报错）→ 置备（尽力）+ 当前用户注册 + 回写记录；`--unregister-identity` → 移除当前用户注册、尽力移除置备、删除记录值。写回位置按记录实际命中的蜂巢（HKLM/HKCU，含 WOW6432Node）。
- 云 CI：`4c66847c` 因已声明资源未预生成失败；`76fec45` 修复后 success；`7048c8f` 三运行 success。
- 合同：Node 39 项通过（含 MSI 注册/注销与提权助手断言）、启动器单测 9/9、clippy 无告警。主 crate 与前端未改，Rust 云端全量基线仍 44259fa3/39148045359。
- **仍未完成**：多用户置备（当前失败）、升级后重新注册、修复/回滚、最低系统版本、外部工具 breakaway、macOS、双击安装的 UAC 交互人工确认。
- 产品运行时代码基线仍 44259fa3（LoreKeeper / 1.0.0-beta.1、中文 NSIS/MSI、安全清理、identifier/数据根/UpgradeCode 不变）。无新 tag/Release/Latest；公开仍[旧 1.0.0-beta](https://github.com/TanCan1998/Lorekeeper/releases/tag/lorekeeper-v1.0.0-beta)。
- 禁止子代理、不抢焦点、不改 Electron/根构建及锁文件/上游 docs；证据见[研究§13](../research/webview-process-attribution.md#13-第119次msi-全机注册注销闭环与置备缺口)。
