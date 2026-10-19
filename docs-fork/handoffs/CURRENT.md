# 当前交接入口

- 最新：[第116次：产品身份启动器与全机安装记录（构建验证）](./2026-10-19-tauri-migration-status.md)。**只做构建与单测，没有安装、没有注册身份包；产品未验收，发布继续暂停。**
- 用户要求“继续”落地真实启动器与 MSI 全机/多用户。本轮已把机制转成产品代码并真实生成 MSI 与 NSIS。
- 新增独立 crate `tauri-app/src-tauri/identity-launcher/`（仅官方 `windows-sys`，不进主 crate 依赖图）：包内模式校验证安装记录与文件后，用公开 `PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY`(2) 带身份启动真实主程序并透传退出码；包外模式绝不无身份启动，只用 `shell:AppsFolder\<AUMID>` 激活身份包。单测 10/10。
- 安装记录（schema 1）已双向接线：NSIS `POSTINSTALL` 写当前用户；MSI 新 WiX 片段写 HKLM（`DirectoryRef INSTALLDIR`、`Action=createAndRemoveOnUninstall`、`Win64=yes`）。键名 `Software\tancan1998\LoreKeeper` 与模板 `Manufacturer` 一致。
- 本机验证：`pnpm tauri build --bundles msi --bundles nsis` EXIT=0，产出 MSI 53,186,560 B 与 NSIS 49,738,678 B；生成的 `main.wxs` 含 `ComponentRef LoreKeeperInstallRecord`；MSI `Registry` 表只读查询到 4 条 HKLM 记录；生成的 `installer.nsi` 在第 706 行插入 `NSIS_HOOK_POSTINSTALL`。Node 合同 18 项（安装 7 + 身份 9，另有发布 19）通过，含真实 `makensis` 语法编译。
- **仍未完成**：启动器未打包进 MSIX、也未随安装包安装；两个安装器都未注册/置备 MSIX 身份；快捷方式仍指向安装目录 exe（包外启动器未被入口使用）；真实安装/升级/修复/回滚/卸载、多用户、最低系统版本、外部工具 breakaway、macOS 均未验证。
- 因此“Windows 真实归组”仍停在最简宿主与机制层；运行时代码基线仍 44259fa3（LoreKeeper / 1.0.0-beta.1、中文 NSIS/MSI、安全清理、identifier/数据根/UpgradeCode 不变），最近云端全量 39148045359：1689 通过、all-targets 成功，Rust 语义切片累计 0（本轮未改主 crate）。
- 发布继续暂停：无新 tag/Release/Latest，公开仍[旧 1.0.0-beta](https://github.com/TanCan1998/Lorekeeper/releases/tag/lorekeeper-v1.0.0-beta)。真实产品安装卸载、项目删除/注册表清理、模型/MCP/云业务仍未执行。
- 禁止子代理、不抢焦点、不改 Electron/根构建及锁文件/上游 docs；授权与在飞状态见 `.pi/tmp/pipeline-state.md`，机制证据见[研究§10](../research/webview-process-attribution.md#10-第116次产品启动器与全机安装记录接线构建验证)。
