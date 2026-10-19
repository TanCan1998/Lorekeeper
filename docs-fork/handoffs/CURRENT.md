# 当前交接入口

- 最新：[第121次：首启补注册闭环（自愈）与静默可观测](./2026-10-19-tauri-migration-status.md)。**多用户按方案 A 收口**：MSI 全机安装 + 每用户首次一次提权注册，不引入签名；该流程已实测通过。
- 决策依据（本机源码）：数据根按用户隔离（`AI_NOVEL_LOREKEEPER_HOME` 或 `~/.lorekeeper`），云凭据用当前用户 DPAPI，全项目无 `ProgramData` 机器级共享状态。因此“多用户”只影响安装位置与身份注册，不影响数据模型；不签名的前提下，“全机装完零操作全覆盖”不可实现。
- 已实测：
  - NSIS 与 MSI 两条链均完成真实安装 → 注册 → 归组（主程序 + 6 个 WebView2 同包身份）→ 卸载清零。
  - MSI：`msiexec /i /qn INSTALLDIR=自选目录` 退出 0（`scope=machine`、启动器路径就位、包计数 1）；`/x /qn` 后目录、HKLM 记录、包、置备、WindowsApps 目录全部 0/false，无残留进程。
- 首启注册（方案 A 的关键）**已实测通过**：删除本用户注册与包文件后，非提权静默首启 → 自动一次 UAC 补注册（从安装目录 MSIX 重新 `Add-AppxPackage`）→ 重读记录 → 主程序 PID 32620 与 WebView2 同为 `com.tancan1998.lorekeeper_1.0.1.0_x64__e8hwe6njnc7fy`；包计数 0 → 1，错误日志为空。
- 上一轮失败原因已定位：`Remove-AppxPackage` 连带删除 WindowsApps 内包目录，加上错误走 MessageBox 阻塞非交互环境；现已改为“文件缺失/未注册即自愈”+ `--quiet` 写日志不弹框，并区分“用户取消授权”与“系统策略阻止”。
- 升级导致的路径漂移（包版本变化后 `IdentityLauncherPath` 指向旧目录）**仍未处理**，自愈只覆盖未注册/文件缺失。
- 云 CI：`dcb88e77` 三运行 success；本轮改动待提交后核验。
- 合同：Node 39 项、启动器单测 10 项（新增包全名推导）、clippy 无告警。主 crate 与前端未改，云端全量基线仍 44259fa3/39148045359。
- **仍未完成**：首启注册验证、升级后重新注册/路径刷新、修复/回滚、最低系统版本、外部工具 breakaway、macOS。发布继续暂停。
- 禁止子代理、不抢焦点、不改 Electron/根构建及锁文件/上游 docs；证据见[研究§14](../research/webview-process-attribution.md#14-第120次多用户范围决策与首启注册流程)。
