# Tauri手动打包与预发布

## 边界

首版 **1.0.0-beta**、标签 **lorekeeper-v1.0.0-beta** 使用预发布且不设最新版。后续已获准准备 **1.0.0-beta.1**：保留 beta 名称，允许普通 Release + Latest，正文明确预览/非正式版，不覆盖旧版。只修改Tauri版本，不改Electron根版本。

进程归组仍未修复；用户最新明确**不接受带此缺陷先公开**，要求继续调研，因此新版发布暂停。配置就绪、提交或打包不代表发布成功，不再以旧方案授权请求缺陷豁免。后续优先按[研究矩阵](../research/webview-process-attribution.md)核验。

工作流 `.github/workflows/tauri-release-package.yml` 只接受 `workflow_dispatch`。默认 `publish=false` 仍仅打包；明确 `publish=true` 才允许独立发布作业写Release/标签。三平台为Windows x64（NSIS/MSI）、macOS ARM64/Intel（DMG），不提供Linux安装包。

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

详见[镜像协议](atomgit-mirror.md)。GitHub 仍为主仓库，原打包平台与验收门禁不变；AtomGit 只同步已提交源码和同一批已验证安装包，不重复编译或压缩。

- 发布作业必须有仓库 Secret `ATOMGIT_ACCESS_TOKEN`。在 GitHub 公开并核验后，显式推送代码与标签、同步原始九附件；不能依赖 `GITHUB_TOKEN` 创建的 Release 事件自动触发另一个工作流。
- 镜像完成前不登记双端发布成功；AtomGit 上传后匿名流式读回大小和 SHA256，更新并读回两端相同下载表格，推荐中国大陆用户使用 AtomGit。
- `.github/workflows/atomgit-mirror.yml` 自动同步分支/标签，以及用户手动公开/编辑的 Release；手动填 `release_tag` 可仅重试既有发布同步。
- 两端公开操作不是事务；失败保留现状，不删除、强推或覆盖原包。预发布属性保留，AtomGit Latest 语义不冒充 GitHub 独立 `make_latest`。
- 本轮不触发打包、Rust 全量或 beta.1 发布，既有新版本暂停边界不变。

## 离线自检

在仓库根执行：

```powershell
node --test tauri-app/scripts/release-helpers.node-test.mjs tauri-app/scripts/installer-contract.node-test.mjs
node --check tauri-app/scripts/publish-release.mjs
```

再做Tauri的typecheck、相关测试、lint与build/diff检查。Node内置测试使用独立命名，避免被Vitest当作自己的测试套件扫描。版本/配置变化按项目混合验证政策触发对应SHA的Rust云端验证。

原始打包清单 `published:false` 是打包时状态，不在发布后重写它；Release实时状态以GitHub API为准。清单摘要不是代码签名或安装验收。

## 安装与版本限制

Windows无证书签名；macOS仅ad-hoc、未开发者签名/公证。应用版本为1.0.0-beta.1，WiX数字版本由1.0.0.0递增至1.0.1.0（不能仅递增MSI忽略的第四段）；UpgradeCode固定为 `ed3e3199-4ddc-501a-a745-179a185e8dfb`。发布门禁 `releaseIdentity` 会调用 `assertWixVersion` 校验该字段：必须是 `major.minor.patch[.build]` 纯数字（主/次≤255，修订/构建≤65535），主次版本必须与应用版本相同、修订号不得低于应用版本修订号，缺失或带预发布后缀直接失败。显示名称采用LoreKeeper（本 fork 自有品牌，不跟随上游改名，见 `project-reference.md` 的「品牌与命名边界」），标识和数据目录不变。不承诺从内部1.1.0版本降级的安装路径。

NSIS固定当前用户与SimpChinese；MSI为zh-CN。EXE卸载勾选框默认不选，保留项目；勾选后调用独立本机清理模式，不使用模板的AppData递归删除。手动选择目录及两次明确确认后才清理一个有效LoreKeeper项目；拒绝系统/程序/全局目录、链接、嵌套项目与Vela数据。静默、被动、升级和安装器维护调用一律不启动清理。安装位置/语言注册键无论是否保留项目都清，厂商键仅在空时移除，共享运行库不动。

MSI静默卸载/macOS拖拽卸载默认保留，卸载前可运行 `lorekeeper.exe --cleanup-project`，macOS为 `open -n -W -a LoreKeeper --args --cleanup-project`。清理模式不接收路径参数、不注册IPC，不加载最近项目或业务服务。macOS系统拖拽流程本身无法变成自定义中文卸载向导，包内提供中文说明。

本机测试只验证清理守卫、配置/发布合同和NSIS编译语法，不执行真实删除、注册表清理或安装卸载。Windows任务管理器归组仍受[Tauri #15567](https://github.com/tauri-apps/tauri/issues/15567)与[WebView2 #5628](https://github.com/MicrosoftEdge/WebView2Feedback/issues/5628)阻塞，macOS归属显示待真机核验；不能把名称/图标修改冒充归组修复。

发布不授权前台窗口、真实模型/MCP/业务云或安装实测，也不把历史495f5632安装包当作本次产物。具体限制必须写入版本发布说明。
