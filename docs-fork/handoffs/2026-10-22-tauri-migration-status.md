# Tauri 迁移进度快照（2026-10-22）

## 已验证状态（第126次：历史临时产物与闲置依赖瘦身）

验证范围：`master` 基于 `51a9da6a`，仅修改 Tauri 前端依赖声明与自有锁文件；验证完成于 **2026-10-22 04:03 +08**。当前轮不改 Rust、频道、数据库、权限或发布配置。

| 指标 | 结果 | 验证范围与证据 |
|---|---|---|
| 历史临时产物 | 402 个目标、1610 个文件、680756953 字节（约649.2 MiB逻辑大小）已删除 | 用户明确同意后，仅旧探针 WebView2 缓存、历史 PDB、MSI 解包 bin；本机 `.pi/tmp/slimming-cleanup-result-1792612352604.json` |
| 前端依赖 | 移除确认的7项直接声明；应用锁图522→403，少119个快照 | pnpm 12.14.0正常移除；无新增快照、保留快照内容不变，工具链锁文档及settings不变 |
| 类型检查 / lint | 全部 exit 0，lint零告警 | `pnpm typecheck`、`pnpm run lint`；本机 `.pi/tmp/slimming-typecheck.log`、`slimming-lint.log` |
| 前端全量 | 225文件，2827通过、1跳过、0失败 | `pnpm test --maxWorkers=2`；本机 `.pi/tmp/slimming-test.log` |
| 构建 / 产物体检 | 全部 exit 0；183文件、41119203字节、8个WOFF2，无TTF与禁止标识 | `pnpm build`、`node scripts/assert-package-contents.mjs --dist dist`；本机 `.pi/tmp/slimming-build.log`、`slimming-contents.log` |
| Electron / 根隔离 | 根 package.json、pnpm-lock.yaml、vite.config.ts、.npmrc逐字节未变 | 源码和根依赖未动；工作区仅本轮Tauri依赖与交接文档变更，既有未跟踪 `tmp-check/` 保持不动 |
| Rust / invoke / GUI | 本轮未重跑，未改对应实现或契约，未启动GUI | 最近Rust全量仍引用发布源码 `058ec936` / CI `39555957392` 的1750通过+all-targets通过；不归给本轮新提交 |

依赖验证输入摘要：

- `tauri-app/package.json` SHA256：`6aa9fd60216cdc74948475319bcbcac6a8204fa96b37fc150a45967237fc3be8`。
- `tauri-app/pnpm-lock.yaml` SHA256：`795e9a6f0e250bc3ec335e89d4f9b14bdededb3d5b52655f6565473f1e5152d2`。
- 验证汇总：本机 `.pi/tmp/slimming-validation-result.json`、`.pi/tmp/slimming-final-validation.json`。本机日志与原始清理清单不入库。

## 本次更新

用户要求先清理不影响下次构建的历史临时产物，再调查依赖供确认；随后明确同意移除建议的7项并验证。

仅移除：

- 生产依赖：`@monaco-editor/react`、`monaco-editor`、`react-markdown`、`react-resizable-panels`、`rehype-raw`、`remark-gfm`。
- 开发依赖：`@tailwindcss/typography`。

保留：

- `@lezer/highlight`：CodeMirror仍间接使用，删除直接声明也不能消除该包。
- `electron`、`@types/better-sqlite3`：基线类型闭包和tsconfig路径所需，不是Tauri渲染运行时依赖。
- 两栈源码、根依赖树、字体与Rust依赖；恢复备份、登录状态、验证收据及已发布安装包。

清理阶段未动正式Rust target、两套node_modules及dist；第二阶段仅按确认更新Tauri依赖树，并重建dist完成验证。旧探针缓存、调试符号及解包副本将来复现实验时需重新生成；不影响正常应用构建输入。

## 收益与边界

- 临时产物为约649.2 MiB逻辑文件，未测删除前后卷空闲量，不把它当作精确物理释放量。
- 移除依赖对应本机约76.98 MiB独占版本逻辑文件；pnpm硬链接/全局store可能保留，不宣称物理磁盘等量释放。
- 前端新构建约39.2 MiB；本机旧dist未绑定同一源码，不能用新旧差异归因依赖移除或宣称安装包缩小。字体未改。
- 未重打包、移动标签、修改Release或覆盖beta.2附件；公开安装包仍是 `058ec936` 构建。
- 本轮无Rust语义切片，累计数不增加，历史全量基线不刷新。

## 交接给下次会话

- 当前任务入口：`.pi/tmp/pipeline-state.md`；提交后精确HEAD在该文件记录。
- 本轮功能范围已验证；后续若要减少安装包体积，字体保留策略及构建配置须另行确认，不延伸本次授权。
- 未解除验收边界：beta.2安装后自动转交激活器的真实交互、多用户UAC（用户暂缓）、旧版升级、macOS真机、真实模型/MCP/业务云。
- 不删除Electron基线、不改上游docs、不恢复子代理、不推送AtomGit；后续代码由AtomGit平台拉取，未核验前不宣称新提交已同步。
