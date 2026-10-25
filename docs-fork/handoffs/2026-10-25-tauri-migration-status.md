# Tauri 迁移进度快照（2026-10-25）

> 当前入口 CURRENT.md；规则见 ../agents/pi-development.md §9 与 ../agents/orchestration-policy.md。

## 已验证状态（第 136 次）

| 指标 | 结果 | 验证版本/范围 | verifiedAt | 证据路径 |
|---|---|---|---|---|
| invoke 契约/注册/覆盖/未迁移 | 288 / 289 / 287 / **0**，命令名与契约一一对应 | HEAD 工作区（本轮 shared 改动未提交时） | 2026-10-25 11:33 +08 | `pnpm run check:channels` 输出（exit 0） |
| 事件覆盖 | 7 | 同上 | 2026-10-25 11:33 +08 | 同上 |
| 前端 typecheck / lint | 0 / 0 | `tauri-app` 全量 | 2026-10-25 11:33 +08 | `pnpm run typecheck`、`pnpm run lint`（exit 0） |
| 定向 vitest | 32 文件 / **540 通过** / 0 失败 | `pnpm test src/shared` | 2026-10-25 11:33 +08 | 同上 |
| Rust 全量 | 未重跑（本轮零 Rust 改动） | 基线 `6ba2bc8d` / CI 39982608773：1752 通过 + all-targets | 2026-10-23 | 引用原验证，不记为本轮结果 |
| GUI/双栈隔离 | 未做 | — | — | 本轮不涉界面与运行时 |

## 本次更新

- taskId 136：**对齐上游 1.3.2 遗留的 shared 增量，并补齐 LLM 响应契约**（上轮 CURRENT「遗留」明列的最后一项源码级事项）。
- 契约：`tauri-app/src/shared/ipc-channels.ts` 的 `LLMResponseBase` 补 `httpFailure?: { status: number; body: string }`。Rust `llm::chat::LlmResponse.http_failure` 早已序列化该字段（`#[serde(skip_serializing_if)]`），Tauri 契约副本缺声明属既有漂移；生成链仍按 `error` 判定，`httpFailure` 只作连接测试证据，故为可选字段，不改变成功判定。
- 注释增量：`character-proposal-parser`（3）、`import-run`（2）、`legacy-roster-generation-pure`（1）、`plot-tree`（8）共 14 处 `// i18n-exempt:` 与上游逐字同步，说明这些中文 `throw` 只在调用处被改写或仅进日志、作者看不到。仅注释，无行为变化。
- 判定依据：逐文件比对 `src/`（466 文件）与 `tauri-app/src/`（461 文件）——相对路径缺口只有 8 个 Storybook 故事与 2 个已按死代码审计删除的服务；175 个同名文件差异抽样核对为「类型走 `@baseline` 复用」「覆盖说明/恢复信号按 Tauri 语义重写」等刻意差异化，非未迁移。
- 未改：Rust 代码、schema、依赖、锁文件、权限；未重打包、未发布、未改 `docs/`、未派发子代理。

## 交接给下次会话

- 分支 `master`，HEAD 为本快照所在提交；工作区仅既存未跟踪 `tmp-check/`。
- 当前状态入口：.pi/tmp/pipeline-state.md。
- 下一步候选：①上游新的 `src/`、`electron/` 增量的下一次同步（本轮已把 1.3.2 的 shared 尾巴收完）；②低层 provider 文案仍为中文句子（`API 调用失败 (status)` 等），上游 a71bbcb3 将其码化为 `LLM_REQUEST_FAILED` / `LLM_STREAM_*`，若要对齐需同时改 Rust 分类、契约与展示面，属独立切片，需先评估消费面；③beta.2 打包后才可见的桌面图标与安装入口验收。
- 未解除阻塞（来源 CURRENT.md）：beta.2 多用户 UAC、旧版升级、卸载/重装、NSIS 原生安装、macOS 真机、真实模型/MCP/云业务；B 档 17 个契约常量与 C 档 66 个仅测试引用按审计结论不动。
- 验证命令与范围见上表；本轮全部为 `tauri-app` 内定向验证，无 Rust 与根工程改动。
