# 当前交接入口

- 最新快照：[2026-10-15-tauri-migration-status.md](./2026-10-15-tauri-migration-status.md)，第 **60** 次。
- 最新验证代码：`bc078492` / master；文档提交后的最终 HEAD 以 Git 为准。
- **B97 剧情图生成五频道已接通**：
  - `graph-generation:read` / `begin` / `execute` / `confirm` / `cancel`；
  - 专属准入控制（`graph_admission`），通用 `generation:begin` 与 `generation:execute` 调入 `GRAPH_OPERATIONS`（`plot-tree-snapshot`、`narrative-thread-plan-candidate`、`narrative-thread-event-candidate`）一律拦截并返回 `GENERATION_GRAPH_ADMISSION_REQUIRED`；
  - 任务与上下文冻结：在 `sourceManifest` 中固化 `graphGenerationContext`、`graphGenerationKey`、`graphGenerationTask` 及其 SHA-256 哈希；
  - 幂等防重：按 `uiActionNonce` 查重，相同输入幂等返回已有 recovery，不同输入抛出 `GENERATION_NONCE_CONFLICT`；
  - 单次尝试约束：执行阶段校验 `attemptCount == 0`（违背则抛 `GENERATION_GRAPH_ATTEMPT_CONFLICT`）并断言可变性；
  - 确认与效应落盘：委托 `GraphGeneration::confirm` 在立即事务中完成双阶段冲突检测与效应写入（校验 `targetBaselineHash` 或插入计划/事件记录）。
- 频道：invoke 290、事件 7；注册 267 → **覆盖 266**，剩余 24（覆盖率约 **91.7%**，不等于产品完成度）。
- 本轮 Rust 验证：定向 `graph_generation` 22 项全部通过；全目标 `cargo check --all-targets` 退出 0 无警告；定向 rustfmt 退出 0。
- 全量 Rust 基线：1415 通过 / 0 失败（基线代码 `4c9364c4`，全量后 Rust 代码切片累计 5：B93 + B94 + B95 + B96 + B97）。
- 前端验证：`ipc-client-graph-generation.test.ts` 5 项契约测试全部通过；typecheck、lint、check:channels 退出 0。
- 最近前端非浏览器全量仍为 B91 的 1929 通过 / 123 失败 / 1 跳过；浏览器/真实 GUI/真实模型/MCP 尚未验收。
- 下一切片候选：
  1. Agent/draft 跨会话在途收据×关闭重开×多会话导航、unknown result×父根生命周期。
  2. 真实 tauri dev/build 窗口首验与启动门禁（SKIN_SNAPSHOT_NOT_READY 分类）。
  3. 特殊域（cloud-backup 8、finalized-character 5、character-avatar 4、dialog 4、project 归档 3）继续按需保持确认边界。
- 本机活动状态：[pipeline-state](../../.pi/tmp/pipeline-state.md)；频道事实：[频道清单](../plans/tauri-migration-channel-inventory.md)；完整证据范围见[最新快照](./2026-10-15-tauri-migration-status.md)。
