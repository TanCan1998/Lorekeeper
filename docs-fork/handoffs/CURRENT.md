# 当前交接入口

先读本页和 [.pi/tmp/pipeline-state.md](../../.pi/tmp/pipeline-state.md)；仅按任务读取详细材料。

- 最近已验证快照：[2026-10-14-tauri-migration-status.md](./2026-10-14-tauri-migration-status.md)（第 44 次：B3-3 owner 常驻缓存、普通恢复与关闭生命周期已验证，完整 cargo test 1225 passed / 0 failed，定向前端 43 passed）。
- 复验基线 Git HEAD：`2fbddb47`；第 44 次 B3-3 切片与 fork 文档随本次提交入库。用户已确认人工逐项核验并授权推送 origin/master；具体提交号以 Git 和本机 pipeline 状态为准。
- 编排授权与流程：[orchestration-policy.md](../agents/orchestration-policy.md)。
- 派发与结果格式：[dispatch-template.md](../agents/dispatch-template.md)。
- 频道事实：[频道清单](../plans/tauri-migration-channel-inventory.md)；数值见快照验证记录（243 命令 / 242 覆盖 / 48 未迁移——G4-10c 注册 agent-generation 8 频道）。
- 活动队列、runId、未解除阻塞和下一步：只维护在状态文件。

## 下一波入口

1. **B3-3** 继续（owner begin/execute_inner + 网络执行/事件接线 + 特殊域恢复准入及共享符号去重）→ **B3-4**（commit_draft）。owner 已保留运行服务及配置分发入口，但尚未实际执行网络分发。研究入口 `.pi/tmp/research/agent-generation-b3.md`。
2. **已验证接线**：真实源绑定、草稿准备绑定、跨 IPC owner 缓存、普通 resume 账本恢复及模型租约生命周期；关闭失败保留库/会话/owner 供重试。知识采集命令、执行主体和事件仍待续；特殊域恢复保持关闭。
3. 后续候选：legacy-roster begin/execute 活化、上游合并独立波次、import-novel-resume 单项修复评估。

恢复先核对当前 HEAD、工作区和登记 run 状态；仅有不一致或未知运行时才扫描孤儿目录。不得因历史文字"在飞"就停止、重派或清理。
