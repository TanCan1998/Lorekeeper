# 当前交接入口

- 最新快照：[2026-10-15-tauri-migration-status.md](./2026-10-15-tauri-migration-status.md)，第 **53** 次。
- 最新验证代码：`d5a3fc97` / `master`，已推送 `origin/master`；本次提交仅更新文档，最终 HEAD 以 Git 为准。
- 本轮阶段复核 Rust 全量 **1383 passed / 0 failed（58.54s）**；前端组合套件 42文件 **417通过/1跳过**，助手客户端 40项与 Agent store 20项定向通过，typecheck、lint 退出0。
- 助手客户端在 `close()` 或已请求 `cancel()` 后不再发起新的 `begin()` / `resume()`（`GENERATION_AGENT_DETACHED`），在途迟到收据仍由取消路径回收；在途取消后的恢复只重放主进程已保存的轮次，不重新请求模型或认领工具。
- 最近前端全量扫描仍有 7文件 / 123项失败（`f15a1b71`）：本轮已复核其中两项分类，分别为 Electron 运行时导入与未开放特殊域按合同拒绝；未补映射绕过，未删除测试。
- 生产短锁、关闭围栏、快照/推理事件、Agent 轮次/恢复、材料/短细纲/对账、普通重启，以及已登记 `generate_draft → chapter-draft` 仍在原范围；其他工作流、图谱/审稿/定稿派生特殊域未开放。
- 频道计数沿用 `a1884ac9` 历史实测：243 注册 / 242 invoke 覆盖 / 48 未迁移；契约 invoke 290、事件 7。约83%仅为历史频道覆盖率，不是产品完成度。
- 真实模型、GUI、MCP、Windows 更新等未验收。活动/待续状态见 [.pi/tmp/pipeline-state.md](../../.pi/tmp/pipeline-state.md)（本机，不入库）；开发子代理派发仍禁止。

## 下次接续与未解除边界

1. 按“先复现失败再修复”继续核对助手与草稿链路的未覆盖组合（在途收据 × 关闭重开 × 多会话导航、工具结果 unknown × 父根生命周期），并按修改范围运行定向/typecheck/lint。
2. 前端剩余失败先分类，再决定是测试适配、真实迁移缺口还是尚未开放域；不得为全绿放开 `finalized-character:read-context`、`review-revision:prepare`、`graph-generation:*`、`finalization-generation:*` 等拒绝面。
3. 新准入、schema、依赖、定稿/文件权限、CI/发布与 ADR 仍先确认；真实模型/GUI/MCP、Windows 更新、dirty 关窗、整屏瞬黑尚未验收。

详情：[最新快照](./2026-10-15-tauri-migration-status.md)；频道事实：[频道清单](../plans/tauri-migration-channel-inventory.md)；验证/交接规则：[开发指南](../agents/pi-development.md)。
