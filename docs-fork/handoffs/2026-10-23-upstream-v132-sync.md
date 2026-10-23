# 上游 1.3.2 同步（2026-10-23）

## 授权与范围

- 用户授权**合并上游并更新根依赖与锁文件**（`package.json`、`pnpm-lock.yaml`、`pnpm-workspace.yaml`）。
- 本轮只做上游合入与本机定向验证；不重打包、不动 beta.2、不发布、不改 `docs/`、不恢复子代理派发。
- 合并前分支 `chore/merge-upstream-20261022`，基线 `785511ed`（origin/master 同步，工作区仅未跟踪 `tmp-check/`）。

## 合并结果

- 上游区间：`7e6b827f` → `305ee755`（tag `v1.3.2`），**124 个提交、27 个 PR 合并**。
- 合并提交 `c4583267`：`🔧 chore(sync): 合并上游 1.3.2 并保留 fork 增强`，`--no-ff`，**零冲突**（`ort` 策略）。
- 变更规模：274 文件、6285 插入、3148 删除（13 新增 / 259 修改 / 2 删除）。
- 冲突面判定依据：本地自 `7e6b827f` 改动 1190 文件，上游改动 274 文件，**两者交集为 0**。比对在合并前完成，非事后推断。
- `docs/` 与上游一致性：合并后 `git diff upstream/master -- docs` 为空。

## 上游内容概览

- 1.3.2 发布收尾：版本号、`.release/notes/v1.3.2.md`、说明措辞按实际行为收窄。
- 主进程报错体系改造（本批主体）：`electron/repositories` 43、`electron/controllers` 23、`electron/services` 19、`electron/llm` 5、`electron/mcp` 4 的中文报错改为登记错误码，渲染层按界面语言说明；`src/` 侧 62 服务 + 60 组件 + 16 store 同步改走统一显示入口。
- 新增棘轮：`scripts/check-i18n-coverage.mjs` 扩到渲染层 throw 与主进程边界，基线 `scripts/i18n-main-error-baseline.json`。
- 依赖安全修复：`vite ^8.0.16 → ^8.2.4`、Storybook 全家 `10.5.10 → 10.6.2`、`js-yaml 4.3.1 → 4.3.2`、新增 `source-map-js 1.2.1 → 1.2.2` override；`plugins/dsh-ai-novel-writer` 同步升级 `js-yaml`、`vite`、`source-map-js`、`undici`。
- 清理：删除无生产调用方的 `src/services/agent/agent-completion.ts`（含测试）；删除无调用方的旧向量迁移与文件导入路径。
- 无 Rust、无 `tauri-app/`、无 `.github/` 改动。

## 对 Tauri 栈的影响判定

- `tauri-app/` 与 `src-tauri/` 上游零改动 → 不触发 Rust 测试预算，累计切片数不变。
- `@baseline/*` → `../electron/*` 仅类型引用；上游对 `electron/repositories` 的 43 文件改动**无 `export` 行增删**，签名未变，`tauri-app` typecheck 实测通过。
- `src/shared/ipc-channels.ts` 仅新增 `LLMResponseBase.httpFailure?`（可选响应字段），不是频道；`check:channels` 统计不变（288 invoke / 7 事件 / 缺失 0）。
- `electron/migrations/baseline-schema.ts` 仅 3 处 `throw` 文案改错误码，**无 schema 结构变更**。

## 本机验证

根目录（`. D:\Environment\npm-mirrors.ps1` 后用镜像，改项目 `.npmrc` 未动）：

- `pnpm install`：+31 / -21 包（storybook 10.6.2、vite 8.2.4、eslint-plugin-storybook 10.6.2）；供应链策略检查通过；**`package.json`、`pnpm-lock.yaml`、`pnpm-workspace.yaml` 均未被二次改写**（`git status --porcelain` 为空）。
- `pnpm typecheck` exit 0（`NODE_OPTIONS=--max-old-space-size=4096`）。
- `pnpm run lint` exit 0，`check-color-literals` 通过。
- `pnpm check:i18n` exit 0：`i18n coverage check passed`。
- `pnpm exec vitest run`（显式排除 `tauri-app/**`、`.pi/**` 及配置既有排除项）：**418 文件通过 / 15 失败，6173 通过 / 17 失败 / 12 跳过**，退出码 1。失败全部归因为本机环境或 fork 既存差异，逐项如下：

| 归因 | 数量 | 证据 |
|---|---|---|
| Windows 无符号链接权限（`EPERM: symlink`） | 5 | `quality-modernization-r3-native`、`quality-modernization-receipt`、`project-format-migration`、`portable-project-archive`、`project-restore-service` |
| 并发下用例超时（5s/15s/120s） | 5 | `check-commit-msg`、`quality-modernization-run`、`release-evidence-v2`、`windows-cloud-build-workflow`、`main-generation-owner` |
| 缺 macOS 官方旧版夹具 | 4 | `f05-a11-official-windows-driver` ×2、`smoke-macos-dmg-acceptance-contract` ×2（`node:internal/errors:999`） |
| fork 品牌/README 断言 | 1 | `release-version`：期望 README 含上游下载链接，fork README 已改 |
| 本机 `AGENTS.md` 存在的卫生测试 | 1 | `public-repository-hygiene`：`AGENTS.md: expected true to be false`（已知既存） |
| 连带统计 | — | 首次全量跑为 19 文件 / 27 测试失败，二次为 15 / 17，**失败集合浮动**，符合超时/缓存特征 |

定向复跑（串行，无并发压力）：`main-generation-owner`、`project-access` 两个上游改动用例**通过**；`project-restore-service` 仅剩 symlink `EPERM` 一条，与该文件上游改动无关。

`tauri-app/`：

- `pnpm install`：`Already up to date`（pnpm 12.14，锁图未变）。
- `pnpm typecheck` exit 0。
- `pnpm run lint` exit 0。
- `pnpm check:channels` exit 0：契约 invoke 288（事件 7）、已注册 289、功能迁移 287/288、明确不支持 1、**缺失 0**、一一对应。
- `pnpm test` exit 0：**225 文件、2827 通过、1 跳过、0 失败**，与第 126 次记录一致。
- 未跑 `test:browser`（上游新增 4 个 `*.browser.tsx`，普通 `vitest run` 不匹配其文件名）；本机缺 Playwright 自带浏览器，如需执行沿用本机 Edge 无头，不安装浏览器。

## 远端 CI（推送 `d0c9b32b` 后）

- Commit message CI [39979222625](https://github.com/TanCan1998/Lorekeeper/actions/runs/39979222625)：success。
- Gitleaks [39979222600](https://github.com/TanCan1998/Lorekeeper/actions/runs/39979222600)：success。
- Tauri Rust CI 未触发（本轮 `tauri-app/`、`src-tauri/` 无改动），符合预期，不据此登记 Rust 全量。
- AtomGit 走平台 Pull，本机未推送镜像；未核验平台是否已拉取 `d0c9b32b`。

## 合并后遗留（另开切片，不阻塞本次合并）

1. ~~`httpFailure` 契约~~ **已落地**（`e44adbde`）。Rust 新增 `HttpFailure { status, body }` 并挂到 `LlmResponse.http_failure`，`openai`/`gemini` 非 2xx 时填充；`LlmTestConnectionResult` 透传为 `httpFailure`（`serde(rename)` + 缺省时省略）；渲染层新增 `modelTestFailureMessage` 按界面语言写「模型请求失败（HTTP 401）…服务返回：…」，正文为空时不留悬空「服务返回」。
   - **刻意差异**：基线把 `error` 也改成 `LLM_REQUEST_FAILED`；Tauri 保留原中文 `error` 文案，只**追加**结构化字段，避免影响共用的 `llm:generate` 失败提示（那部分仍与现状一致，不回归）。
   - 验证：`cargo check --all-targets` exit 0；定向 `cargo test --lib llm` **164 通过 0 失败**（含 2 个新增序列化用例）；5 个改动文件 `rustfmt --check` 通过；`tauri-app` typecheck/lint 通过；全量 vitest **227 文件 / 2833 通过 / 1 跳过 / 0 失败**；settings 四个 browser 用例 35 通过（Edge 无头）。
   - **云端全量已通过**：同一 SHA `6ba2bc8d` 的 Tauri Rust CI [39982608773](https://github.com/TanCan1998/Lorekeeper/actions/runs/39982608773) —— **1752 通过 0 失败**（166.31s），「运行全量测试与所有目标检查」步骤 success，Commit message CI 与 Gitleaks 亦 success。Rust 累计切片**清零**，最新全量基线更新为本 SHA。
2. ~~`structured-contract-diagnostic.ts` 对齐~~ **已完成**（`54486fc7`）。Tauri 侧此前 3 处 catch 把诊断的 `error.message`（形如「结构化合同诊断 code=… path=… field=…」的技术串）直接显示给作者；现新增 `structuredContractDiagnosticText`（12 个码 × 双语、根路径 `$` 说成「整体内容」）并接入蓝图恢复稿的刷新、保存与续接三条失败路径。非诊断错误仍显示原始说明，行为不变。验证：`tauri-app` typecheck/lint 通过；新增 3 个单测通过；`planning-recovery-editor.browser.tsx`（本机 Edge 无头）66 通过；全量 vitest 226 文件 / 2830 通过 / 1 跳过 / 0 失败。
3. 其余 4 个双实现 shared 文件的纯增量仍待评估：`plot-tree.ts`（+8）、`character-proposal-parser.ts`（+3）、`import-run.ts`（+2）、`legacy-roster-generation-pure.ts`（+1）。需先出行为差异清单。
4. 诊断文案的**显示面已审计完毕**，清单见 [plans/2026-10-23-structured-diagnostic-display-audit.md](../plans/2026-10-23-structured-diagnostic-display-audit.md)。结论：抛出点集中在 3 个 shared/parse 文件（63 处）；作者手稿路径只有蓝图恢复稿一条（已修）；**模型输出路径有 3 处会把技术串给作者**（`DirectoryBlueprintContractError`、`import-novel.command` 中文分支、`architecture.command` 中文分支）。审计明确建议**不要**把 `structuredContractDiagnosticText` 套到模型输出（语气错位），应改为「生成失败 + 可读摘要」、技术码只进诊断日志；改造需同步 `directory.command.test.ts:1309` 断言。
5. 上游新增的 `test(verify)` / `*.browser.tsx` 场景中可迁移给 Tauri 的用例（英文界面导入、模型连接、项目页报错等）。
6. `check:i18n` 棘轮不搬入 `tauri-app/`（扫描对象是 Electron `src/` 结构）；Tauri 侧继续用自有双语检查。

## 明确未做

- 未改 `docs/`；未改根 `.npmrc`、`vite.config.ts`、上游 CI 配置。
- 未跑 Rust 测试、`cargo check --all-targets`（本轮无 Rust 语义变更）；最近 Rust 全量仍为发布源码 `058ec936` / CI 39555957392（1750 通过），**不移记为当前 SHA**。
- 未重打包、未发布、未改 beta.2 的 Release/标签/附件；未执行 GUI、安装、注册、提权。
- 未派发子代理；`tmp-check/` 保持不动。

## 2026-10-23 后续：beta.2 原生拉起与关闭验收

用户授权后按推荐执行。验收前系统已处于 beta.2 **MSI 安装态**：`D:\Program Files\LoreKeeper` 内 `lorekeeper.exe` 的 SHA256 `AA785FEB9F91E6F33D8C81B8E429E22F48B00AE9E935BA97FC6B29CCDBF35871` 与 beta.2 MSI 产物一致；Appx `com.tancan1998.lorekeeper 1.0.2.0` 已注册；公共桌面快捷方式指向 `lorekeeper-activate.exe` 且无参数；`identity\LoreKeeper.msix` 载荷存在。

- **探针（不建窗口）**：`LOREKEEPER_PACKAGED_LAUNCH_PROBE=1` 直接运行安装目录 `lorekeeper.exe` → 退出码 **42**，2 秒后无残留进程。
- **真实拉起**：直接运行 `lorekeeper.exe`（无包身份）。1 秒内出现 `lorekeeper` / `lorekeeper-activate` / `lorekeeper-identity-launcher` 三个进程，最初直接运行的那个 PID 已退出；WebView2 CDP 读到 `type=page title="Lorekeeper 设定司" url=http://tauri.localhost/`；Appx 版本前后一致。**门禁 → 激活器 → 以包身份重启主程序**这条链路成立。
- **干净关闭复验**（不设 CDP 端口，只向主窗口句柄发 `WM_CLOSE`）：三个进程在 **1 秒内全部自行退出**，无残留。
- 首轮验证时我先用 CDP `/json/close/{target}` 关闭页面，随后进程 35 秒未退出；上面的对照复验说明这是**验证手段干扰**（关闭 WebView2 page target 破坏了正常退出流程），不是产品缺陷。强制结束只用于清理那一次实例，不作为关闭行为证据。
- 数据根无新写入：两次启动前后 `~/.lorekeeper` 最新仍为 `recent-projects.json @ 2026-10-23 02:17`，`~/.vela` 最新仍为 `config.json @ 2026-10-16 21:38`。
- 身份启动器日志写在 `%TEMP%\LoreKeeper\identity-launcher.log`（`1241a1d6` 修复生效，未再落到安装目录）；日志尾部三条为 beta.1 时期历史记录（缺少管理员权限注册/注销）。
- 验证期间未置顶、未发送键鼠、未切前台；窗口由应用自身启动，关闭只用 `WM_CLOSE`。
- 仍未验收：多用户 UAC（需第二登录账号）、旧版本升级路径、卸载/重装、NSIS 包的原生安装与拉起、macOS 真机、真实模型/MCP/业务云。
