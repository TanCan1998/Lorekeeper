# Tauri手动打包与预发布

> **2026-10-21 当前授权**：用户最终确认保留 beta.1，新发 `1.0.0-beta.2`：`publish=true`、`prerelease=false`、`make_latest=true`，全部三平台四包，同步 GitHub 与 AtomGit；不删除、不移动旧标签，不覆盖旧 Release 或附件。此具体授权取代历史暂停记录，不授权本次产物的原生安装/卸载/注册、前台窗口或真实业务。证据与待验限制见[本轮快照](../handoffs/2026-10-21-upstream-merge-and-release.md)及[发布说明](../releases/1.0.0-beta.2.md)；新 SHA 云端全量成功前不得发布。

## 边界

首版 **1.0.0-beta** 使用预发布且不设最新版；beta.1 与 beta.2 采用普通 Release + Latest，但正文必须明确预览/非正式版，不能冒充完整原生验收。只修改 Tauri 版本，不改 Electron 根版本。

包身份路线已有正向证据；beta.2 新增主程序包身份检查，无包身份时转交激活器。不能把探针、包身份读回或编译成功等同于新安装包真实任务管理器逐 PID 验收；所有入口、升级与多用户仍按[研究矩阵](../research/webview-process-attribution.md)区分证据。

工作流 `.github/workflows/tauri-release-package.yml` 只接受 `workflow_dispatch`。默认 `publish=false` 仍仅打包；明确 `publish=true` 才允许独立发布作业写Release/标签。三平台为Windows x64（NSIS/MSI）、macOS ARM64/Intel（DMG），不提供Linux安装包。

## 平台构建命令

主配置的 `build.beforeBuildCommand` 为 Windows 生成身份小包；macOS 由 `src-tauri/tauri.macos.conf.json` 覆盖为 `pnpm build`，不执行 Windows 专用启动器、MakeAppx 或资源映射。发布合同测试同时锁定这两点。2026-10-21 首次发布尝试即因两平台共用一个 `beforeBuildCommand` 而中断，修复后需以新 SHA 重跑云端全量与四包打包。

## 流程

1. prepare锁定完整源码SHA，核对package/Cargo/config/Cargo.lock项目版本、新标签命名与发布说明；已存在同名标签或Release立即失败。
2. 三个平台以同一SHA构建，冻结依赖，前端体检并生成真实安装包；打包流不执行测试。
3. 发布作业仅拥有 `contents:write`、`actions:read`，其他作业保持只读。下载当前run/attempt且同一源码SHA的三份产物。
4. 要求同一源码SHA已有独立Tauri Rust CI全量成功记录；不以旧SHA测试或打包成功替代。
5. 对精确四包及三清单校验版本、平台、run/attempt、大小与SHA256，拒绝额外文件、坏摘要及错世代产物。
6. 原子创建新标签，创建草稿、上传9个资产（四安装包、三平台原清单、SHA256SUMS、release-manifest），核对远端大小/摘要/集合后公开。
7. `make_latest` 是独立显式选项，默认 false。拒绝 `prerelease=true` + `make_latest=true`；beta名称不强制GitHub预发布属性，但发布说明必须明确“预览”和“非正式版”，Release标题带预览标签。草稿始终 `make_latest=false`，公开时才应用所选值，选择Latest后须读回 `/releases/latest` 核对Release ID。
8. 再读回Release与标签SHA。发生错误不覆盖、不自动删除标签或草稿，须核查后处理；不能盲目重跑发布作业覆盖现有版本。

临时Actions安装包与清单仅保留 **1天**，Release assets不受影响。同run发布须在过期前完成，过期应重新完整打包；详见[Actions存储策略](actions-storage.md)。

所有Actions固定commit SHA；本次download-artifact v8.0.2为 `9000827ccba6bdab643e8b6fd33ac0654aef8333`。

## AtomGit 镜像与双源说明

详见[镜像协议](atomgit-mirror.md)。GitHub 仍为主仓库，原打包平台与验收门禁不变。2026-10-21 起代码与标签由 AtomGit 平台拉取，GitHub 不推送同步；安装包只按原样单独上传，不重新编译或压缩。

- 发布作业不再访问 AtomGit：代码与标签由 AtomGit 平台镜像拉取，GitHub 侧没有推送工作流或推送脚本。
- 代码拉取完成后，用 `pnpm mirror:release` 手动上传原样附件；`ATOMGIT_ACCESS_TOKEN` 只供该工具使用。上传后匿名流式读回大小和 SHA256，再更新并读回两端相同下载表格。
- 附件工具只补缺失文件，不覆盖已有附件；两端公开操作不是事务，失败保留现状。预发布属性保留，AtomGit Latest 语义不冒充 GitHub 独立 `make_latest`。
- beta.2 已获明确打包与发布授权；附件工具不重建安装包，不代替独立 Rust 全量门禁。

## 离线自检

在仓库根执行：

```powershell
node --test tauri-app/scripts/release-helpers.node-test.mjs tauri-app/scripts/installer-contract.node-test.mjs
node --check tauri-app/scripts/publish-release.mjs
```

再做Tauri的typecheck、相关测试、lint与build/diff检查。Node内置测试使用独立命名，避免被Vitest当作自己的测试套件扫描。版本/配置变化按项目混合验证政策触发对应SHA的Rust云端验证。

原始打包清单 `published:false` 是打包时状态，不在发布后重写它；Release实时状态以GitHub API为准。清单摘要不是代码签名或安装验收。

## 安装与版本限制

Windows无证书签名；macOS仅ad-hoc、未开发者签名/公证。应用版本为1.0.0-beta.2，WiX数字版本由1.0.1.0递增至1.0.2.0（不能仅递增MSI忽略的第四段）；UpgradeCode固定为 `ed3e3199-4ddc-501a-a745-179a185e8dfb`。发布门禁 `releaseIdentity` 会调用 `assertWixVersion` 校验该字段：必须是 `major.minor.patch[.build]` 纯数字（主/次≤255，修订/构建≤65535），主次版本必须与应用版本相同、修订号不得低于应用版本修订号，缺失或带预发布后缀直接失败。显示名称采用LoreKeeper（本 fork 自有品牌，不跟随上游改名，见 `project-reference.md` 的「品牌与命名边界」），标识和数据目录不变。不承诺从内部1.1.0版本降级的安装路径。

NSIS固定当前用户与SimpChinese；MSI为zh-CN。EXE卸载勾选框默认不选，保留项目；勾选后调用独立本机清理模式，不使用模板的AppData递归删除。手动选择目录及两次明确确认后才清理一个有效LoreKeeper项目；拒绝系统/程序/全局目录、链接、嵌套项目与Vela数据。静默、被动、升级和安装器维护调用一律不启动清理。安装位置/语言注册键无论是否保留项目都清，厂商键仅在空时移除，共享运行库不动。

MSI静默卸载/macOS拖拽卸载默认保留，卸载前可运行 `lorekeeper.exe --cleanup-project`，macOS为 `open -n -W -a LoreKeeper --args --cleanup-project`。清理模式不接收路径参数、不注册IPC，不加载最近项目或业务服务。macOS系统拖拽流程本身无法变成自定义中文卸载向导，包内提供中文说明。

beta.1 已获独立授权并通过本机安装、修复、卸载、身份注册/注销与隔离项目清理验收；这不代表 beta.2 安装包验收。beta.2 目前仅有自动测试、探针与构建证据，macOS 归属显示待真机核验；不能把名称/图标修改冒充归组修复。

发布不授权前台窗口、真实模型/MCP/业务云或安装实测，也不把历史495f5632安装包当作本次产物。具体限制必须写入版本发布说明。
