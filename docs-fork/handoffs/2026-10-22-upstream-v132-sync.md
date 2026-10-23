# 上游 1.3.2 同步（2026-10-22）

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

## 合并后遗留（另开切片，不阻塞本次合并）

1. `ipc-channels.ts` 的 `httpFailure?`：Tauri 连接测试是否消费该字段，涉及 Rust 侧`llm_connection_test`返回值，需单独设计。
2. Tauri 5 个双实现 shared 文件的**纯增量上游改动**尚未同步：`structured-contract-diagnostic.ts`（+30 行 `structuredContractDiagnosticText` 双语说明）、`plot-tree.ts`（+8）、`character-proposal-parser.ts`（+3）、`import-run.ts`（+2）、`legacy-roster-generation-pure.ts`（+1）。需先出行为差异清单再决定是否对齐。
3. 上游新增的 `test(verify)` / `*.browser.tsx` 场景中可迁移给 Tauri 的用例（英文界面导入、模型连接、项目页报错等）。
4. `check:i18n` 棘轮不搬入 `tauri-app/`（扫描对象是 Electron `src/` 结构）；Tauri 侧继续用自有双语检查。

## 明确未做

- 未改 `docs/`；未改根 `.npmrc`、`vite.config.ts`、上游 CI 配置。
- 未跑 Rust 测试、`cargo check --all-targets`（本轮无 Rust 语义变更）；最近 Rust 全量仍为发布源码 `058ec936` / CI 39555957392（1750 通过），**不移记为当前 SHA**。
- 未重打包、未发布、未改 beta.2 的 Release/标签/附件；未执行 GUI、安装、注册、提权。
- 未派发子代理；`tmp-check/` 保持不动。
