# 当前交接入口

先读本页和 [.pi/tmp/pipeline-state.md](../../.pi/tmp/pipeline-state.md)；仅按任务读取详细材料。

- 最近已验证快照：[2026-10-14-tauri-migration-status.md](./2026-10-14-tauri-migration-status.md)（第 35 次；按原记录保留，未重跑其测试）。
- 最近 Git HEAD：`206c97a61c318c222e5b6012e045026de8d358c3`（整理时只读核对）。HEAD 已有 G4-10a 提交，快照尚未覆盖；以状态文件中的提交/待核对字段为准，不能当作最新快照实测值。
- 编排授权与流程：[orchestration-policy.md](../agents/orchestration-policy.md)。
- 派发与结果格式：[dispatch-template.md](../agents/dispatch-template.md)。
- 频道事实：[频道清单](../plans/tauri-migration-channel-inventory.md)；数值见快照验证记录。
- 活动队列、runId、未解除阻塞和下一步：只维护在状态文件。

恢复先核对当前 HEAD、工作区和登记 run 状态；仅有不一致或未知运行时才扫描孤儿目录。不得因历史文字“在飞”就停止、重派或清理。

新交接只更新本页的快照/版本指针和短提示；详细证据在快照，活动状态在状态文件。本页不复制指标表、长任务历史和授权全文。
