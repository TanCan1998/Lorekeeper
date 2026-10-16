# 当前交接入口

- 最新快照：[2026-10-16-tauri-migration-status.md](./2026-10-16-tauri-migration-status.md)，第 **64** 次。
- 最新已验证代码：`29c63e11` / master；本轮另有存储根修复 `0b746ce2`。
- 本轮完成便携冻结证据与恢复安装**服务层**：空投影冻结侧车、恢复校验、所有权清理、
  安装日志与标记、断点续装、转移权威改写、新项目清单及 FTS 重建后的业务证据幂等验证。
- **Rust 最新全量：1530 passed / 0 failed**；`cargo check --all-targets` 无告警。
  日志：`.pi/tmp/b113b-full.log`、`.pi/tmp/b113b-all-targets.log`。
  本轮便携定向 71 项通过；全量后累计 Rust 代码切片数 **0**。
- **频道覆盖沿用** `df756fed` 的 280/290（本轮没有新增频道，也未重跑频道脚本）；
  尚余 cloud-backup 8、project:archive-restore、project:import-legacy-copy。
- **下一阻塞**：开放恢复频道前，必须接好运行时冻结历史写入门禁。
  现有生成仓储默认冻结回调恒为 false，仅有侧车不足以防止历史重放。
  若采用 SQLite 连接级临时触发器与守卫函数，涉及启用现有 rusqlite 的 functions feature
  及连接级 TEMP schema，需用户先确认；未新增依赖、未改 Cargo 配置、未安装触发器。
- 后续：恢复频道接线与前端契约测试 → 旧版副本导入边界 → 云凭据安全存储边界 → 真实窗口验收。
- 前端最近非浏览器全量仍沿用 B91 历史结果，真实 GUI/模型/MCP 尚未验收。
- 活动状态：[pipeline-state](../../.pi/tmp/pipeline-state.md)；频道事实：
  [频道清单](../plans/tauri-migration-channel-inventory.md)。不派发子代理。
