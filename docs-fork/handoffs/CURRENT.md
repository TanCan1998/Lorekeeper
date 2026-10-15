# 当前交接入口

- 最新快照：[2026-10-15-tauri-migration-status.md](./2026-10-15-tauri-migration-status.md)，第 **57** 次。
- 最新验证代码：`504d1dd6` / master；文档提交后的最终 HEAD 以 Git 为准。
- **B94 编辑器行内生成四频道已接通**：
  - `editor-inline:begin` / `editor-inline:read-recovery` / `editor-inline:execute` / `editor-inline:cancel`；
  - 专属准入控制（`editor_inline_admission`），普通 `generation:begin` / `generation:execute` 调用一律拦截并返回 `GENERATION_EDITOR_ADMISSION_REQUIRED`；
  - 任务构造完全对齐基线：模板 `edit_selected_text`，按动作分流（refine/expand/continue/dialogue），中英文双语指令与系统角色不可变合同；
  - 幂等性与单次尝试冲突防护（`GENERATION_NONCE_CONFLICT` / `GENERATION_EDITOR_ATTEMPT_CONFLICT`）均有回归覆盖。
- 频道：invoke 290、事件 7；注册 255 → **覆盖 254**，剩余 36（约 87.6%，不等于产品完成度）。
- 本轮 Rust 验证：定向 `editor_inline` 27 项全部通过（23 项在 `editor_inline_generation`，4 项在 `main_generation_owner`）；全目标 `cargo check --all-targets` 退出 0 无警告；定向 rustfmt 退出 0。
- 全量 Rust 基线：1415 通过 / 0 失败（基线代码 `4c9364c4`，全量后 Rust 代码切片累计 2：B93 + B94）。
- 前端验证：`ipc-client-editor-inline.test.ts` 4 项契约测试全部通过；typecheck、lint、check:channels 退出 0。
- 最近前端非浏览器全量仍为 B91 的 1929 通过 / 123 失败 / 1 跳过；浏览器/真实 GUI/真实模型/MCP 尚未验收。
- 下一切片候选：
  1. `import-generation:read` / `import-generation:execute` 2 频道（派生 generation 导入切片）。
  2. Agent/draft 跨会话在途收据×关闭重开×多会话导航、unknown result×父根生命周期。
  3. 真实 tauri dev/build 窗口首验与启动门禁（SKIN_SNAPSHOT_NOT_READY 分类）。
  4. 特殊域（cloud-backup 8、finalization-generation 5、finalized-character 5、graph-generation 5、character-avatar 4、dialog 4、project 归档 3）继续按需保持确认边界。
- 本机活动状态：[pipeline-state](../../.pi/tmp/pipeline-state.md)；频道事实：[频道清单](../plans/tauri-migration-channel-inventory.md)；完整证据范围见[最新快照](./2026-10-15-tauri-migration-status.md)。
