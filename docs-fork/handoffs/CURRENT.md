# 当前交接入口

- 最新快照：[2026-10-15-tauri-migration-status.md](./2026-10-15-tauri-migration-status.md)，第 **61** 次（阶段收口快照）。
- 最新验证代码：`bc078492` / master；文档提交后的最终 HEAD 以 Git 为准。
- **阶段收口（累计 5 个 Rust 切片全量回归通过）**：
  - B93: 项目概览只读投影与离线预览凭证（`project:overview-current` / `project:peek-overview`，2 频道）
  - B94: 编辑器行内生成四频道与专属准入控制（`editor-inline:begin` / `read-recovery` / `execute` / `cancel`，4 频道）
  - B95: 导入生成双频道与导入槽位恢复准入（`import-generation:read` / `import-generation:execute`，2 频道）
  - B96: 定稿后续生成五频道与专属准入控制（`finalization-generation:read` / `begin` / `execute` / `commit` / `cancel`，5 频道）
  - B97: 剧情图生成五频道与专属准入控制（`graph-generation:read` / `begin` / `execute` / `confirm` / `cancel`，5 频道）
- **频道指标**：invoke 290、事件 7；注册 267 → **覆盖 266**，剩余 24（覆盖率约 **91.7%**，不等于产品完成度）。
- **Rust 全量验证基线**：**1448 passed / 0 failed（58.63s）**，基线代码 `bc078492`，日志归档于 `.pi/tmp/b97-full.log`（全量后累计 Rust 代码切片数：**0**）。
- **静态质量与格式**：全目标 `cargo check --all-targets` 退出 0 无警告；定向 rustfmt 退出 0；`pnpm typecheck`、`pnpm run lint`、`pnpm run check:channels` 退出 0。
- **前端契约验证**：本阶段新增 5 个 vitest 契约测试套件全部 100% 通过（共 18 项单测）。
- 最近前端非浏览器全量仍为 B91 的 1929 通过 / 123 失败 / 1 跳过；浏览器/真实 GUI/真实模型/MCP 尚未验收。
- **下阶段活动队列候选**：
  1. Agent/draft 跨会话在途收据×关闭重开×多会话导航、unknown result×父根生命周期。
  2. 真实 tauri dev/build 窗口首验与启动门禁（SKIN_SNAPSHOT_NOT_READY 分类）。
  3. 特殊域（cloud-backup 8、finalized-character 5、character-avatar 4、dialog 4、project 归档 3）继续按需保持确认边界。
- 本机活动状态：[pipeline-state](../../.pi/tmp/pipeline-state.md)；频道事实：[频道清单](../plans/tauri-migration-channel-inventory.md)；完整证据范围见[最新快照](./2026-10-15-tauri-migration-status.md)。
