# 当前交接入口

先读本页和 [.pi/tmp/pipeline-state.md](../../.pi/tmp/pipeline-state.md)；仅按任务读取详细材料。

- 最近已验证快照：[2026-10-14-tauri-migration-status.md](./2026-10-14-tauri-migration-status.md)（第 37 次：G4-10c 收口——agent-generation 子波次 C，域工具效应移植 + 8 频道命令注册；`cargo test --lib` 1033 passed / 0 failed、`check:channels` 243 命令 / 242 覆盖 / 48 未迁移，编排者独立复验）。
- 最近 Git HEAD：`28daa346`（G4-10c 代码提交）；快照文档随本次 docs 提交更新。
- 编排授权与流程：[orchestration-policy.md](../agents/orchestration-policy.md)。
- 派发与结果格式：[dispatch-template.md](../agents/dispatch-template.md)。
- 频道事实：[频道清单](../plans/tauri-migration-channel-inventory.md)；数值见快照验证记录（243 命令 / 242 覆盖 / 48 未迁移——G4-10c 注册 agent-generation 8 频道）。
- 活动队列、runId、未解除阻塞和下一步：只维护在状态文件。

## 下一波入口

1. **子波次 D**：`with_agent_child_effect` 主体活化（main_generation_owner.rs 占位）+ `commands/db.rs` 两处调用点 + `character_proposal.rs` approve 接线；活化后 db.rs 两处 UNMIGRATED 测试断言（:4718-4729、:4761-4780）改语义断言。研究：`.pi/tmp/research/agent-generation-domain.md` §3.1/§3.4。
2. 前置依赖：begin 主体 / execute 分发核心 / resume / execute_round 主体仍 fail-closed（依赖 B3 创建核心与 LLM 双阶段执行循环迁移）。
3. 后续候选：legacy-roster begin/execute 活化、上游合并独立波次、import-novel-resume 单项修复评估。

恢复先核对当前 HEAD、工作区和登记 run 状态；仅有不一致或未知运行时才扫描孤儿目录。不得因历史文字"在飞"就停止、重派或清理。
