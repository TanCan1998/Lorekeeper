# 当前交接入口

先读本页和 [.pi/tmp/pipeline-state.md](../../.pi/tmp/pipeline-state.md)；仅按任务读取详细材料。

- 最近已验证快照：[2026-10-14-tauri-migration-status.md](./2026-10-14-tauri-migration-status.md)（第 39 次：B3-2b 收口——生成运行服务本体 generation_run_service + agent_response_protocol 1:1 平移；`cargo test --lib` 1184 passed / 0 failed，编排者独立复验）。
- 最近 Git HEAD：`fe33fcd6`（G4-B3-2b 代码提交）；快照文档随本次 docs 提交更新。
- 编排授权与流程：[orchestration-policy.md](../agents/orchestration-policy.md)。
- 派发与结果格式：[dispatch-template.md](../agents/dispatch-template.md)。
- 频道事实：[频道清单](../plans/tauri-migration-channel-inventory.md)；数值见快照验证记录（243 命令 / 242 覆盖 / 48 未迁移——G4-10c 注册 agent-generation 8 频道）。
- 活动队列、runId、未解除阻塞和下一步：只维护在状态文件。

## 下一波入口

1. **推理机器对齐专波**（B3-M1 + B3-M2 + B3-M3 合并，均涉及既有文件/在役命令行为，一次复验）→ **B3-3**（owner 主体接线：begin/execute_inner/resume 主体 + currentModelReceipt + rebuildBinding 真实化 + B3-2 剩余②llm_generation.rs 流式核心抽取为 pub 服务函数 + ③事件发射活化，dispatch 经 GenerationDispatcher trait 由 llm::openai/llm::gemini 实现）→ **B3-4**（commit_draft 写入路径）；研究入口 `.pi/tmp/research/agent-generation-b3.md`。
2. **currentModelReceipt 迁移**（host capture 真实化，随 B3-3）：基线 main-generation-owner.ts:251-256——迁移后 `current`/`capture` 保守占位可替换为 `rebuild_generation_source_binding` 真实重建。
3. 后续候选：legacy-roster begin/execute 活化、上游合并独立波次、import-novel-resume 单项修复评估。

恢复先核对当前 HEAD、工作区和登记 run 状态；仅有不一致或未知运行时才扫描孤儿目录。不得因历史文字"在飞"就停止、重派或清理。
