# LoreKeeper迁移进度快照（2026-10-19）

> [当前入口](./CURRENT.md) · 第 **109** 次：上游同步与定稿图标接入。不是迁移或新版发布完成报告。验证截至2026-10-19 00:13 +08:00。

## 已验证状态

- 分支 `master`；上游合并提交 `460fcd77` 已推送，Commit message CI **39143162149**、Gitleaks **39143162141** 均成功。
- 图标代码提交 **e31c42d11a506e828758db1c33f57d6f17d2ae40**；后续交接文档提交与最终推送/CI状态由本机 `.pi/tmp/pipeline-state.md` 记录。
- 上游 `upstream/master` 为 **d21207c856b6d70b1f54215d39f8ae423bb17273**，19个新增提交已合入。用户随后要求同步最新中英文README，已再次 `fetch --no-tags` 确认远端无新增；`README_en.md` 与上游逐字节一致，`README.md` 仅额外保留顶部 fork 目的说明。
- 本轮根 `package.json`、`pnpm-lock.yaml`、`vite.config.ts`、`.npmrc` 未改；`docs/` 与上游一致；Tauri依赖、数据库、权限、版本及安装配置未改。

| 验证范围 | 本轮结果 | 版本与本机证据 |
|---|---|---|
| 根 typecheck / lint | 均exit 0 | 合并树460fcd77；`.pi/tmp/upstream-followup/` |
| 上游相关契约 | 4文件38通过，涵盖README本地化、发布版本、打包契约、更新E2E脚本 | 同上 `contracts.log` |
| 新增NSIS识别回归 | 1通过、73未选中 | 同上 `nsis.log`，仅模拟进程识别，不运行真实安装 |
| 真实8.3路径辅助进程链 | 32项全部跳过，没有有效通过项 | 同上 `nsis-chain.log`；不算本轮验收 |
| Tauri typecheck / lint / build | 均exit 0 | 图标代码e31c42d1；`.pi/tmp/branding-adoption/` |
| 图标及品牌单元测试 | 4文件17通过 | 同上 `tests.log` |
| 图标无头浏览器 | 3通过，实际SVG加载、22/52像素尺寸、实时亮暗切换、拖动命中保护 | 同上 `browser.log` |
| 外壳无头浏览器 | 2文件17通过，stage-root与stage-shell-settings | 同上 `shell-browser.log`；日志含act警告与IPC异常输出，退出码0，不等于真实桌面验收 |
| 原生打包 / GUI / 安装卸载 | 本轮未执行 | 不把前端构建与无头浏览器当成原生包认证 |
| Rust全量 / all-targets | 本轮未运行，无Rust源码变化 | 最近全量仍a5418614/run39125354221：1680通过0失败及all-targets成功，非本轮结果；累计Rust语义切片0 |

前端构建有大chunk提示，不改打包配置。所有测试均未启动前台应用/安装器，未调用真实模型、MCP或业务云。

## 本次更新

### 上游同步

用户明确授权这批上游改动及其一处CI配置变更：向Electron更新E2E脚本传入 `github.token`，同时纳入子进程环境凭据隔离。只读试合并发现README和更新测试两处冲突，正式合并时保留fork说明、1天制品保留策略及上游新增权限/令牌断言；未改其他工作流。上游新品牌“书童 Pagekin”只留在上游镜像文档/资源，不替换Tauri LoreKeeper身份。

### 定稿图标

- 采用已确认的单颗彩色四芒星“司”字，默认亮色、保留暗色；正式源图在 `tauri-app/public/branding/`，原始摘要验证通过。
- Tauri CLI从亮色源图生成七份桌面PNG/ICO/ICNS，沿用既有资源路径；启动页、顶栏、关于页复用SVG，应用内跟随亮暗主题切换。
- 只提交选中资源，不上传旧候选、用户参考图、第三方官方图、字体包或临时移动端图标。生成与摘要说明见[图标登记](../design/branding.md)。
- 当前productName仍 `Lorekeeper`、版本仍 `1.0.0-beta`，未擅自修改大小写或升级身份。已成功查询旧WiX UpgradeCode：`ed3e3199-4ddc-501a-a745-179a185e8dfb`；后续改名须固定此值。

## 尚未完成与下一步

1. **子进程归组**：Windows WebView2与macOS WebKit正常归组尚无已验证修复。继续核查宿主创建机制和已登记上游问题；改图标不算归组修复。研究材料 `.pi/tmp/process-group-research/`。
2. **中文安装卸载/安全清理**：方案已有用户确认，但未实现。默认保留项目，删除必须显式选目录并二次确认，只清本应用注册键；不碰Vela/系统/共享WebView2。不运行真实数据删除或抢焦点原生测试。
3. **1.0.0-beta.1**：未改版本、未打新包、未发布。保留beta名，以普通GitHub Release（prerelease=false）+Latest发布，正文明确预览；发布脚本旧门禁仍待按已确认方案调整。不得覆盖旧标签/资产。
4. 原生包图标、升级身份与中文安装生命周期仍须后续本机后台验证，macOS须对应平台验证；无法后台完成的对话框继续标待验收。

无在飞子代理、GUI、Vite或下载任务。禁止子代理、不抢焦点、其他先确认边界不变。上轮设计与具体授权历史仍见 `.pi/tmp/branding-install-state.md` 和[第108次](./2026-10-18-tauri-migration-status.md)。公开版本仍为[旧1.0.0-beta预发布](https://github.com/TanCan1998/Lorekeeper/releases/tag/lorekeeper-v1.0.0-beta)，旧九资产不变。
