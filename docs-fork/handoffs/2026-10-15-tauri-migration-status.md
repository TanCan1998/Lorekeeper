# Tauri 迁移进度快照（2026-10-15）

> 第 53 次；按用户“继续迁移、不派发、一批接一批”的指令由主会话直接实施，未派发子代理。
> 同日替换当前摘要，第52次内容保留于 Git 历史（文档提交 `f7135bd1` 之前）；旧日期快照冻结。
> 当前入口：[CURRENT.md](CURRENT.md)；维护规则：[开发指南 §9](../agents/pi-development.md#9-交接状态与快照维护)。

## 已验证状态（第53次）

最新代码：`d5a3fc97` / `master`，已推送 `origin/master`；文档更新前工作区干净。以下结果均核对自本机日志，日期为 2026-10-15。

| 指标 | 结果 | 验证版本 / 范围 / 时间 | 证据（本机，不入库） |
|---|---|---|---|
| 本轮 Rust 全量 | **1383 passed / 0 failed，58.54s**；全目标 0 失败 | `d5a3fc97`，`cargo test`，18:00 +08:00 阶段复核 | `.pi/tmp/b86-rust-full.log` |
| 本轮前端定向（助手客户端） | **40项通过**；新增4项关闭/取消守卫回归，去掉守卫时3项先复现失败 | `8175e4d0`；17:51 +08:00 | `.pi/tmp/b84-client-detached-{before,target}.log` |
| 本轮前端定向（Agent store） | **20项通过**；新增2项在途取消组合回归 | `d5a3fc97`；17:58 +08:00 | `.pi/tmp/b85-inflight-replay-target.log` |
| 本轮前端组合套件 | 42文件 **417通过 / 1跳过**；typecheck、lint 退出0 | `d5a3fc97`；`src/services/agent` + `src/stores` + `src/components/panels/agent` | `.pi/tmp/b85-inflight-replay-{suite,typecheck,lint}.log` |
| 最近前端全量扫描（沿用） | 170文件中7文件失败；2019测试：1895通过、123失败、1待定 | `f15a1b71`；本轮未重跑全量 | `.pi/tmp/b78-results.json` |
| 失败分类复核（本轮抽查） | `finalize-chapter.blueprint-entities` 单点失败已定位为前端按合同拒绝未登记命名参数的 `finalization-generation:read`；`export-service-integration` 为运行时导入 `@baseline/controllers\|database` | `d5a3fc97` 本机复跑 | `.pi/tmp/b87-finalize-chapter-target.log`、`.pi/tmp/b87-export-integration.log` |
| 频道 / 事件计数（沿用） | 243注册、242 invoke覆盖、48未迁移；契约invoke290、事件7 | `a1884ac9` 的2026-10-14历史实测；本轮未改变注册或契约数，未重新计数 | [频道盘点](../plans/tauri-migration-channel-inventory.md) |
| GUI / 真实模型 / MCP | 未验收 | 合成Tauri传输、合成fixture与本地HTTP不能替代真实模型、主进程SQLite或完整产品端到端证据 | 本轮无新增验收结论 |

Rust验证目录为 `tauri-app/src-tauri/`，显式设置 `RUSTUP_HOME=D:/Environment/rust/rustup`、`CARGO_HOME=D:/Environment/rust/cargo`；前端在 `tauri-app/` 执行 `pnpm exec vitest run <文件>`、`pnpm typecheck`、`pnpm run lint`。差异、链接、版本、提交消息与泄漏扫描另行核验。

## 本轮交付与关键决策（第52次 → 第53次）

1. **停止或关闭的助手客户端不再发起新运行**（`8175e4d0`）：`AgentGenerationClient.begin()` / `resume()` 现在在首次 await 前执行 `assertNotDetached()`；`close()` 或已请求 `cancel()` 的客户端以 `GENERATION_AGENT_DETACHED` 直接拒绝，不再向主进程创建新 run、更新纪元，或在不通知的情况下替换恢复上下文。在途 begin/resume 返回后仍由取消路径回收迟到收据（`GENERATION_AGENT_CANCELLED` + `generation:cancel`），不把必要结算误封闭。新增4项定向回归；去掉守卫时其中3项复现失败（`.pi/tmp/b84-client-detached-before.log`）。
2. **在途取消后的恢复只重放持久轮次**（`d5a3fc97`）：新增2项 store 组合回归。(a) 取消在途模型轮次后不再认领/完成工具、不再请求新一轮，句柄与导航保留；恢复只重放主进程已保存的第0轮，模型响应仍只被调用一次，`begin/round/resume/claim/finish` 全部零传输。(b) 取消后主进程仍把在途工具记为 `running`、并保留已登记草稿子代时，恢复只补 `workflow_started` 导航并标记 `result_unknown`，不重新认领也不重派。
3. **失败分类复核，不放宽边界**：`finalize-chapter.blueprint-entities.test.ts` 的单点失败已定位为前端按合同拒绝“命名参数未登记”的频道 `finalization-generation:read`（定稿派生生成为未开放特殊域），不是可顺手补映射的 mock 笔误；`export-service-integration.test.ts` 在运行时导入 `@baseline/controllers`、`@baseline/database`（Electron 主进程 + 基线 DB），无法在 Tauri 栈成立。两者保持失败与拒绝：未删除测试、未放开权限、未补映射绕过。

第52次交付（草稿真实传输接线、unknown 只补导航、恢复身份与工具回执校验、取消与事件生命周期、休眠契约修复、守住未开放边界）结论不变；更早的生产短锁、关闭围栏、快照/推理事件、Agent 首/后轮与恢复、材料/短细纲/对账、普通重启，以及已登记 `generate_draft → chapter-draft` 有限准入与草稿保存到父 Agent 续轮仍在原范围。真实 SQLite 与本地 HTTP 证据以 `90792744` 的阶段验证为准；前端测试桩不扩大这些结论。

## 尚未解除的失败与阻塞

最近全量扫描仍有以下 **7文件 / 123失败**，未删除、跳过或放开生产边界来制造全绿：

| 测试文件（相对 `tauri-app/src/`） | 失败数 | 主要断点 |
|---|---|---|
| `services/__tests__/export-service-integration.test.ts` | 1 | 运行时导入 Electron 主进程与基线 DB（`@baseline/controllers\|database`），Tauri 栈无对应运行时；本轮已复核 |
| `services/__tests__/narrative-thread-candidate-generator.test.ts` | 2 | `graph-generation:begin` 前端未迁移，不擅自开放图谱 |
| `services/__tests__/plot-tree-generator.test.ts` | 1 | 同上，不擅自开放图谱 |
| `services/workflows/commands/__tests__/finalize-chapter.blueprint-entities.test.ts` | 1 | 后处理失败分支触达 `finalization-generation:read`，命名参数未登记即按合同拒绝；本轮已复核为未开放特殊域 |
| `services/workflows/commands/__tests__/finalize-postprocess-character.integration.test.ts` | 15 | 定稿角色后处理特殊域及回执测试缺口，未声称真实持久化通过 |
| `services/workflows/commands/__tests__/mutation-failure-boundary.test.ts` | 63 | 全部受 `finalized-character:read-context` 或 `review-revision:prepare` 未开放阻断 |
| `services/workflows/commands/__tests__/refine-draft.command.test.ts` | 40 | `review-revision:prepare` 前端拒绝；Rust 已注册不等于完整迁移 |

计数来自 `b78-results.json`，不是最新 HEAD 重跑。约83%仅为历史频道覆盖率，不是产品完成度；B3 仍未完成完整端到端验收。

## 交接给下次会话

- 分支 `master`；最新代码 `d5a3fc97` 已推送。活动/待续入口：`.pi/tmp/pipeline-state.md`（本机，不入库）。未创建代理 run；开发子代理派发仍禁止。
- **首要待续**：继续按“先复现失败再修复”核对助手与草稿链路尚未覆盖的组合（在途收据 × 关闭重开 × 多会话导航、工具结果 unknown × 父根生命周期），并按修改范围跑定向/typecheck/lint。前端全量剩余失败要先判断是测试适配、真实迁移缺口还是尚未开放域，不能直接补映射绕过拒绝。
- **不得触碰的拒绝面**：`finalized-character:read-context`、`review-revision:prepare`、`graph-generation:*`、`finalization-generation:*` 等未开放域，以及 `generate_blueprint`、`generate_architecture`、其他普通父根工作流、规划续作、知识采集；新准入、schema、依赖、定稿/文件权限、CI/发布与 ADR 仍须先确认。
- 真实模型、完整 GUI、MCP、Windows 更新、dirty 关窗、整屏瞬黑等未验收。Electron、根 `src/`、上游 `docs/`、根配置及锁文件保持隔离。
