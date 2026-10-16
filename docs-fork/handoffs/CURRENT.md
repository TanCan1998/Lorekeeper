# 当前交接入口

- 最新快照：[2026-10-16-tauri-migration-status.md](./2026-10-16-tauri-migration-status.md)，第 **65** 次。
- 最新已验证代码：`f42ae6f1` / master；冻结连接门禁 `f4ef5e00`，恢复服务 `29c63e11`。
- 已完成便携导出/恢复频道：双授权、冻结证据、连接级历史写入门禁、无覆盖安装、断点续装、
  FTS 重建后的业务证据幂等校验；恢复不切换活动项目，最近项目登记失败返回成功及警告。
- **Rust 最新全量：1535 passed / 0 failed**（库测试59.69s），全目标检查无告警。
  证据：`.pi/tmp/b113c-full.log`、`.pi/tmp/b113c-check.log`；全量后累计 Rust 切片 **0**。
- 本轮命令定向9项、前端3文件8项；typecheck、lint、check:channels通过。
- **频道覆盖281/290**，注册282、事件7，剩余9：cloud-backup八频道、project:import-legacy-copy。
- **下一确认边界**：
  1. 旧版副本导入的输入范围：既定“不与 Electron 互通、无需解码旧库”仍有效；
     上游此频道明确导入 .vela/vela.db 与 LanceDB，不能直接平移或冒充已完成。
  2. 云备份凭据安全存储：keyring 或 Windows DPAPI，须先选定方案与平台边界。
- GUI/真实模型/MCP尚未验收，前端最近非浏览器全量仍沿用B91历史结果。
- 活动状态：[pipeline-state](../../.pi/tmp/pipeline-state.md)；
  [频道清单](../plans/tauri-migration-channel-inventory.md)。不派发子代理。
