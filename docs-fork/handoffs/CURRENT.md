# 当前交接入口

- 最新快照：[2026-10-15-tauri-migration-status.md](./2026-10-15-tauri-migration-status.md)，第 **56** 次。
- 最新验证代码：`580225ad` / master；文档提交后的最终 HEAD 以 Git 为准。
- **B93 项目概览只读投影与离线预览能力凭证已接通**：
  - `project:overview-current`：当前活跃项目库只读快照聚合，严苛校验定稿收据与正文哈希；
  - `project:peek-overview`：进程内不透明能力凭证驱动，隔离临时只读快照，排斥热回滚日志与篡改。
- 频道：invoke 290、事件 7；注册 251 → **覆盖 250**，剩余 40（约 86.2%，不等于产品完成度）。
- 本轮 Rust 验证：概览与离线预览定向 14 项通过（7 在 `project_overview_repository`，7 在 `project_peek`）；全目标 `cargo check --all-targets` 退出 0 无警告；定向 rustfmt 退出 0。
- 全量 Rust 基线：1415 通过 / 0 失败（基线代码 `4c9364c4`，全量后 Rust 代码切片累计 1：B93）。
- 前端验证：传输与会话 12 项（含 2 项 project-overview 契约测试）通过；typecheck、lint、check:channels 退出 0。
- 最近前端非浏览器全量仍为 B91 的 1929 通过 / 123 失败 / 1 跳过；浏览器/真实 GUI/真实模型/MCP 尚未验收。
- 下一切片候选：
  1. Agent/draft 跨会话在途收据、关闭重开与未知结果父根生命周期。
  2. 真实 tauri dev/build 窗口首验与启动门禁（SKIN_SNAPSHOT_NOT_READY 分类）。
  3. 特殊域（cloud-backup 8、character-avatar 4、project 归档 3 等）继续按需保持确认边界。
- 本机活动状态：[pipeline-state](../../.pi/tmp/pipeline-state.md)；频道事实：[频道清单](../plans/tauri-migration-channel-inventory.md)；完整证据范围见[最新快照](./2026-10-15-tauri-migration-status.md)。
