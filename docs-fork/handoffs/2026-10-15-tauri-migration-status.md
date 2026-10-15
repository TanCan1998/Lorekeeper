# Tauri 迁移进度快照（2026-10-15）

## 已验证状态（第 60 次）

本轮代码：`bc078492`，master。普通迁移由主会话直接完成，子代理派发继续停用。

| 指标 | 结果 | 验证版本/范围 | verifiedAt | 证据路径 |
|---|---|---|---|---|
| invoke/事件 | 契约290、事件7；注册267 → **覆盖266**，剩余24 | `bc078492`，check:channels | 2026-10-15 | 终端输出 / check:channels |
| Rust全量（基线） | **1415 passed / 0 failed（26.63s）** | `4c9364c4`，cargo test | 2026-10-15 | `.pi/tmp/b92-full.log`（全量后累计5个切片） |
| Rust定向 | **22 passed / 0 failed** | `bc078492`，cargo test graph_generation | 2026-10-15 | 终端输出 |
| 全目标检查 | 退出 0，无警告 | `bc078492`，cargo check --all-targets | 2026-10-15 | 终端输出 |
| 前端定向 | graph-generation 传输契约5项通过 | `bc078492`，vitest | 2026-10-15 | 终端输出 |
| TS/lint/格式 | typecheck、lint、定向rustfmt、diff检查退出0 | `bc078492` | 2026-10-15 | 终端输出 |
| 最近前端非浏览器全量（沿用，非本轮） | 172文件/2053项：1929通过、123失败、1跳过 | B91，启动门禁代码`74ea2200`；本轮未重跑 | 2026-10-15 | `.pi/tmp/b91-results.json`、`b91-full.log` |
| GUI/真实模型/MCP | 未验收 | 本地测试不能替代真实窗口验收 | — | — |

## 本次更新：B97 剧情图生成五频道与专属准入控制

- 接通 `graph-generation:read` / `begin` / `execute` / `confirm` / `cancel` 全部 5 个频道；
- 注册命令数达到 267，覆盖 invoke 频道提升至 266 / 290（覆盖率达 91.7%），未迁移频道降至 24 个；
- 专属图生成准入控制（`graph_admission`）：
  - 通用 `generation:begin` 与 `generation:execute` 调入 `GRAPH_OPERATIONS`（`plot-tree-snapshot`、`narrative-thread-plan-candidate`、`narrative-thread-event-candidate`）时一律拦截并返回 `GENERATION_GRAPH_ADMISSION_REQUIRED`；
  - 仅由专属命令入口显式置准入标志为 `true`，严防渲染层直调通用入口绕过。
- 幂等查重与上下文固化（`begin_graph_generation`）：
  - 校验 `input` 形状与 `uiActionNonce`（非空且 <= 256 字符），按 `uiActionNonce` 查重；
  - 同一 nonce 重复调用若 input 一致幂等返回已有 recovery，不一致报 `GENERATION_NONCE_CONFLICT`；
  - 全新运行以 `graph_admission = true` 调入内部生成状态机，并在 `sourceManifest` 中固化 `graphGenerationContext`、`graphGenerationKey`、`graphGenerationTask` 及其 SHA-256 哈希。
- 单次尝试与执行约束（`prepare_graph_generation_execution`）：
  - 校验 handle 格式与可变性（`assert_mutable`）；
  - 严格断言单次 attempt（已存在 attempt 则抛出 `GENERATION_GRAPH_ATTEMPT_CONFLICT`）；
  - 单次执行 nonce 恒为 `graph:0`，调入底层引擎。
- 确认与效应落盘（`confirm_graph_generation`）：
  - 校验产物引用与 index，委托 `GraphGeneration::confirm` 在立即事务中完成双阶段冲突检测与效应写入；
  - `plot`：校验 `targetBaselineHash` 未漂移，保存 plot-tree 快照；
  - `plan`：插入叙事线索计划记录；
  - `event`：校验自有事件行未变并确认事件。
- 取消与重置（`cancel_graph_generation`）：
  - 校验 handle，调用 `cancel(rootActionId)` 取消关联根动作，返回最新 run 视图。
- 测试覆盖：
  - Rust `graph_generation`：22 项测试覆盖输入校验、上下文捕获与回读、结果派生、序列化键顺序、规范 JSON 比较、通用 begin 准入拦截、通用 execute 准入拦截、空 nonce 拒绝、非法请求拦截；
  - 前端：5 项传输契约测试覆盖 read/begin/execute/confirm/cancel 全部 5 频道的项目会话自动注入与参数打包。

## 后续工作与未解除边界

1. 下一候选切片：Agent/draft 跨会话在途收据×关闭重开×多会话导航、unknown result×父根生命周期。
2. 启动门禁已持久化于 `~/.lorekeeper/appearance.json`，真实 `tauri dev/build` 窗口尚未首验；`skin:import-custom` 仍返回 `CUSTOM_SKIN_UNAVAILABLE`，前端 `SKIN_SNAPSHOT_NOT_READY` 分类待补。
3. 剩余 24 频道：cloud-backup 8、finalized-character 5、character-avatar 4、dialog 4、project 3。归档 zip、头像 schema、云备份/凭据及新权限仍须先确认；不通过加映射绕过未迁移拒绝。
4. 前端 B91 全量的 7 个失败文件与 `f15a1b71` 完全一致，本轮仅跑相关合同，不宣告这些遗留失败已解除。

工作区、实际 HEAD 及下一活动队列以 Git 与 `.pi/tmp/pipeline-state.md` 为准。根 `src/`、`electron/`、`docs/` 及根依赖配置/lockfile 未改。最近全量基线为 1415/0，全量后 Rust 代码切片累计 5；本轮秘密扫描无发现。
