# 当前交接入口

先读本页和 [.pi/tmp/pipeline-state.md](../../.pi/tmp/pipeline-state.md)；仅按任务读取详细材料。

- 最近已验证快照：[2026-10-14-tauri-migration-status.md](./2026-10-14-tauri-migration-status.md)（第 46 次：B3-3 普通 begin 策略/租约/运行创建，完整 cargo test 1247 passed / 0 failed，定向前端 43 项通过；扩展前端仍有历史失败，见快照）。
- 复验基线 Git HEAD：`efb96e0d`；累计六个 Rust 文件及三份 fork 交接文档尚未提交/推送，人工逐项核验待完成。子代理派发临时禁用，须用户明确恢复。
- 编排授权与流程：[orchestration-policy.md](../agents/orchestration-policy.md)。
- 派发与结果格式：[dispatch-template.md](../agents/dispatch-template.md)。
- 频道事实：[频道清单](../plans/tauri-migration-channel-inventory.md)；数值见快照验证记录（243 命令 / 242 覆盖 / 48 未迁移——G4-10c 注册 agent-generation 8 频道）。
- 活动队列、runId、未解除阻塞和下一步：只维护在状态文件。

## 下一波入口

1. **B3-3** 普通根运行 begin 已接通，继续 execute_inner + 网络执行/事件接线 + 特殊域恢复准入及共享符号去重。**B3-4** commit_draft 事务/收据切片已实现，待执行链路接通及批次/真实 Agent 子效应专项。研究入口 `.pi/tmp/research/agent-generation-b3.md`。
2. **已验证接线**：真实源绑定、草稿准备绑定、跨 IPC owner 缓存、普通 begin/resume、模型租约生命周期及草稿提交；关闭失败保留库/会话/owner 供重试。知识采集、执行主体、事件及前端既存失败复验仍待续；特殊域创建/恢复保持关闭。
3. 后续候选：legacy-roster begin/execute 活化、上游合并独立波次、import-novel-resume 单项修复评估。

恢复先核对当前 HEAD、工作区和登记 run 状态；仅有不一致或未知运行时才扫描孤儿目录。不得因历史文字"在飞"就停止、重派或清理。
