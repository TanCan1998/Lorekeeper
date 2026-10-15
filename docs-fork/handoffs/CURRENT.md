# 当前交接入口

- 最新快照：[2026-10-15-tauri-migration-status.md](./2026-10-15-tauri-migration-status.md)，第 **58** 次。
- 最新验证代码：`8f6a98cb` / master；文档提交后的最终 HEAD 以 Git 为准。
- **B95 导入生成双频道已接通**：
  - `import-generation:read` / `import-generation:execute`；
  - 专属准入控制（`import_admission`），普通 `generation:execute` 调用一律拦截并返回 `GENERATION_IMPORT_ADMISSION_REQUIRED`；
  - 槽位查找与恢复：按 `importSlotKey` 与 `projectId` 唯一定位运行，多于 1 条报 `GENERATION_IMPORT_SLOT_AMBIGUOUS`，不存在返回 `null`；
  - 序数连续性与前序完成断言：强制要求 `ordinal == count`（否则报 `GENERATION_IMPORT_ORDINAL_GAP`），且所有前序尝试必须为 `stop`（否则报 `GENERATION_IMPORT_PREVIOUS_ATTEMPT_INCOMPLETE`）；
  - 租约与执行权限：校验 `ImportRunExecutionAuthority` 与 `import_runs` 状态（running 且无 cancel 请求，owner/epoch 匹配），效应不可变性（已提交 effect receipt 抛 `GENERATION_IMPORT_EFFECT_SEALED`）。
- 频道：invoke 290、事件 7；注册 257 → **覆盖 256**，剩余 34（约 88.3%，不等于产品完成度）。
- 本轮 Rust 验证：定向 `import_generation` 20 项全部通过；全目标 `cargo check --all-targets` 退出 0 无警告；定向 rustfmt 退出 0。
- 全量 Rust 基线：1415 通过 / 0 失败（基线代码 `4c9364c4`，全量后 Rust 代码切片累计 3：B93 + B94 + B95）。
- 前端验证：`ipc-client-import-generation.test.ts` 2 项契约测试全部通过；typecheck、lint、check:channels 退出 0。
- 最近前端非浏览器全量仍为 B91 的 1929 通过 / 123 失败 / 1 跳过；浏览器/真实 GUI/真实模型/MCP 尚未验收。
- 下一切片候选：
  1. Agent/draft 跨会话在途收据×关闭重开×多会话导航、unknown result×父根生命周期。
  2. 真实 tauri dev/build 窗口首验与启动门禁（SKIN_SNAPSHOT_NOT_READY 分类）。
  3. 特殊域（cloud-backup 8、finalization-generation 5、finalized-character 5、graph-generation 5、character-avatar 4、dialog 4、project 归档 3）继续按需保持确认边界。
- 本机活动状态：[pipeline-state](../../.pi/tmp/pipeline-state.md)；频道事实：[频道清单](../plans/tauri-migration-channel-inventory.md)；完整证据范围见[最新快照](./2026-10-15-tauri-migration-status.md)。
