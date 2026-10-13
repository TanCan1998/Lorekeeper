# 当前交接入口

先读本页和 [.pi/tmp/pipeline-state.md](../../.pi/tmp/pipeline-state.md)；仅按任务读取详细材料。

- 最近已验证快照：[2026-10-14-tauri-migration-status.md](./2026-10-14-tauri-migration-status.md)（第 36 次：G4-10b 收口——agent-generation 子波次 B，owner 内部执行路径与子代准入闭合；`cargo test --lib` 1033 passed / 0 failed，编排者独立复验）。
- 最近 Git HEAD：`2c0365a5`（G4-10b 代码提交）；快照文档随本次 docs 提交更新。
- 编排授权与流程：[orchestration-policy.md](../agents/orchestration-policy.md)。
- 派发与结果格式：[dispatch-template.md](../agents/dispatch-template.md)。
- 频道事实：[频道清单](../plans/tauri-migration-channel-inventory.md)；数值见快照验证记录（235 命令 / 234 覆盖 / 56 未迁移——本波无命令注册，计数不变）。
- 活动队列、runId、未解除阻塞和下一步：只维护在状态文件。

## 下一波入口

1. **子波次 C**：域效应 `agent_domain_tool_effect` 移植 + 8 频道 agent-generation 命令注册（新建 `commands/agent_generation.rs` + `lib.rs` 登记；`agent_generation.rs` 的 `commit_domain_tool` 占位活化）。研究：`.pi/tmp/research/agent-generation-domain.md`（§2.2/§2.3）、`.pi/tmp/research/agent-generation-wave-b.md` §0.4。
2. **子波次 D**：`with_agent_child_effect` 主体活化 + `commands/db.rs` 两处调用点 + `character_proposal.rs` approve 接线。
3. 前置依赖：begin 主体 / execute 分发核心 / resume / execute_round 主体仍 fail-closed（依赖 B3 创建核心与 LLM 双阶段执行循环迁移）。

恢复先核对当前 HEAD、工作区和登记 run 状态；仅有不一致或未知运行时才扫描孤儿目录。不得因历史文字"在飞"就停止、重派或清理。
