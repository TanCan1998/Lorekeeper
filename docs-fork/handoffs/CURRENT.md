# 当前交接入口

- 最新：[第117次：身份包产物、真实安装/卸载与提权限制](./2026-10-19-tauri-migration-status.md)。**归组仍未在产品上验收，发布继续暂停。**
- 已真实执行：独立启动器 release 构建（262,144 B）→ 真实 MSIX（134,256 B，仅启动器+图标，`AppListEntry="none"`）；MSI/NSIS 均重建成功；**静默安装到用户自选目录**退出 0，落盘主程序、激活器与 `identity\LoreKeeper.msix`，`HKCU` 记录 schema=1/scope=user/路径正确；**静默卸载**后目录、注册表键、身份包全部清零。
- **决定性限制**：未签名 MSIX 注册在普通用户下失败（`0x80073D2B`「未签名的程序包不能包含可执行的激活」，加 `AppListEntry=none` 仍失败），提权后同一 MSIX 注册成功并可就位包内启动器（测试后已移除）。因此 Tauri NSIS `currentUser` 安装无法自动获得身份；安装器已改为失败时明确告警，不再假成功。
- 因未签名包不允许可执行激活，`shell:AppsFolder` 方案作废；包外启动器改为读取安装器登记的 `IdentityLauncherPath`（限定 WindowsApps 内本产品启动器）再转交，绝不无身份启动主程序。
- **仍未完成**：MSI 侧注册/置备自定义动作（提权上下文、安装用户 vs SYSTEM、`Add-AppxProvisionedPackage` 多用户语义）未实现未实测；真实 GUI 启动后的任务管理器归组未验证（必须先有提权注册路径）；升级/修复/回滚、多用户、最低系统、外部工具 breakaway、macOS 未验证。
- CI 修复：`4c66847c` 的 `Tauri Rust CI`（39184958870）因已声明的安装资源在 CI 未预先生成而失败（`resource path target\identity\LoreKeeper.msix doesn't exist`）；已加入“构建身份小包”步骤、MakeAppx 版本探测与 `build.rs` 缺包提示，待新提交验证。
- 本轮合同：Node 38 项通过（安装 6 + 身份 10 + 发布 19 + 其余），启动器单测 10/10、clippy 无告警；`git diff --check` 通过。主 crate 与前端未改，Rust 云端全量基线仍 44259fa3/39148045359。
- 产品运行时代码基线仍 44259fa3（LoreKeeper / 1.0.0-beta.1、中文 NSIS/MSI、安全清理、identifier/数据根/UpgradeCode 不变）。无新 tag/Release/Latest；公开仍[旧 1.0.0-beta](https://github.com/TanCan1998/Lorekeeper/releases/tag/lorekeeper-v1.0.0-beta)。
- 禁止子代理、不抢焦点、不改 Electron/根构建及锁文件/上游 docs；机制与限制见[研究§11](../research/webview-process-attribution.md#11-第117次身份包产物资源安装与真实安装卸载回归)。
