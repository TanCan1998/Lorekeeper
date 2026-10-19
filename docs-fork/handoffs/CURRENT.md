# 当前交接入口

- 最新：[第115次：未签名小启动器、外部自选目录归组与构建契约](./2026-10-19-tauri-migration-status.md)。**真实LoreKeeper启动器/安装器尚未接入，不是产品修复或发布完成报告。**
- 用户已确认：不签名、真实归组、主程序自选目录、预留签名位置；使用小型完整MSIX身份启动器+外部主程序逻辑映射，MSI全机/多用户优先。签名稀疏包不再是默认主线。
- 本轮最简宿主4次/48份采样：公开进程策略0x02使两个外部目录均8/8自有PID同组，普通创建/直接启动仍拆组；同字节EXE、无浏览器flag。有效UIA只读不排序/展开，任务管理器/探针前台采样均0；一轮其他句柄变化如实保留。见[研究§9](../research/webview-process-attribution.md#9-第115次未签名小启动器与外部自选目录)。
- 新增 `tauri-app/scripts/windows-identity.mjs`、同名Node测试、`src-tauri/installer/windows-identity.json`。仅生成构建契约/清单；默认unsigned，证书字段null，signed尚未实现则报错。没有接入build.rs、安装器或发布流程。
- 本轮身份9+安装6+发布19共34测试通过，typecheck/lint退出0，MakeAppx候选清单语法通过（实验占位二进制，未注册）。Rust/前端功能代码未变；最近云端全量仍44259fa3 / 39148045359：1689通过、all-targets成功，Rust语义切片累计0，不冒充新全量。
- 临时包finally全用户剩余0，独立复核当前用户包0、自有探针进程0。先行自开Taskmgr33880已结束；有效轮复用的23564不是本轮所有，不关闭。无新证书或信任变更；第114次临时证书/私钥清理结果仍是历史基线。
- [接入计划](../plans/windows-sparse-identity-integration.md)：下一切片原生启动器/安装记录与合成工具子进程策略，随后真实LoreKeeper后台验证、MSI全机stage/provision/每用户注册、升级/修复/回滚/卸载。候选最小能力、文件/注册表虚拟化、直接EXE入口、多用户和macOS均待验收。
- 既有运行时代码44259fa3：LoreKeeper / 1.0.0-beta.1、中文NSIS/MSI、安全清理与显式Latest已实现；identifier/数据根/UpgradeCode不变。默认保留，删除须显式目录+两次确认，只清本应用注册键。
- 产品归组和生命周期未通过前继续暂停新版；无新tag、Release或Latest变化，公开仍[旧1.0.0-beta](https://github.com/TanCan1998/Lorekeeper/releases/tag/lorekeeper-v1.0.0-beta)。真实产品安装卸载、项目删除/注册表清理、模型/MCP/云业务仍未执行。
- 禁止子代理、不抢焦点、不改Electron/根构建及锁文件/上游docs；授权和在飞状态见 `.pi/tmp/pipeline-state.md`。
