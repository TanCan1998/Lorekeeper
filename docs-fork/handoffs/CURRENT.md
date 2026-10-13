# 当前交接入口

先读本页和 [.pi/tmp/pipeline-state.md](../../.pi/tmp/pipeline-state.md)；仅按任务读取详细材料。

- 最近已验证快照：[2026-10-14-tauri-migration-status.md](./2026-10-14-tauri-migration-status.md)（第 38 次：G4-10d 收口——agent-generation 子波次 D，占位活化翻转 fail-closed；`cargo test --lib` 1033 passed / 0 failed、`check:channels` 243 命令 / 242 覆盖 / 48 未迁移，编排者独立复验）。
- 最近 Git HEAD：`b3d9eb78`（G4-10d 代码提交）；快照文档随本次 docs 提交更新。
- 编排授权与流程：[orchestration-policy.md](../agents/orchestration-policy.md)。
- 派发与结果格式：[dispatch-template.md](../agents/dispatch-template.md)。
- 频道事实：[频道清单](../plans/tauri-migration-channel-inventory.md)；数值见快照验证记录（243 命令 / 242 覆盖 / 48 未迁移——G4-10c 注册 agent-generation 8 频道）。
- 活动队列、runId、未解除阻塞和下一步：只维护在状态文件。

## 下一波入口

1. **B3 创建核心与 LLM 双阶段执行循环迁移**：begin 主体 / execute 分发核心 / resume 主体 / execute_round 主体活化（agent-generation 域剩余 fail-closed 面 + commit_draft 新写入路径 + agent-round 完整 LLM 循环均依赖此波次）；研究入口 `.pi/tmp/research/agent-generation-domain.md` §4 依赖矩阵。
2. **currentModelReceipt 迁移**（host capture 真实化）：基线 main-generation-owner.ts:251-256——迁移后 `current`/`capture` 保守占位可替换为 `rebuild_generation_source_binding` 真实重建。
3. 后续候选：legacy-roster begin/execute 活化、上游合并独立波次、import-novel-resume 单项修复评估。

恢复先核对当前 HEAD、工作区和登记 run 状态；仅有不一致或未知运行时才扫描孤儿目录。不得因历史文字"在飞"就停止、重派或清理。
