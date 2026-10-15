# Tauri 迁移进度快照（2026-10-15）

## 已验证状态（第 59 次）

本轮代码：`3209a8e2`，master。普通迁移由主会话直接完成，子代理派发继续停用。

| 指标 | 结果 | 验证版本/范围 | verifiedAt | 证据路径 |
|---|---|---|---|---|
| invoke/事件 | 契约290、事件7；注册262 → **覆盖261**，剩余29 | `3209a8e2`，check:channels | 2026-10-15 | 终端输出 / check:channels |
| Rust全量（基线） | **1415 passed / 0 failed（26.63s）** | `4c9364c4`，cargo test | 2026-10-15 | `.pi/tmp/b92-full.log`（全量后累计4个切片） |
| Rust定向 | **11 passed / 0 failed** | `3209a8e2`，cargo test finalization_generation | 2026-10-15 | 终端输出 |
| 全目标检查 | 退出 0，无警告 | `3209a8e2`，cargo check --all-targets | 2026-10-15 | 终端输出 |
| 前端定向 | finalization-generation 传输契约5项通过 | `3209a8e2`，vitest | 2026-10-15 | 终端输出 |
| TS/lint/格式 | typecheck、lint、定向rustfmt、diff检查退出0 | `3209a8e2` | 2026-10-15 | 终端输出 |
| 最近前端非浏览器全量（沿用，非本轮） | 172文件/2053项：1929通过、123失败、1跳过 | B91，启动门禁代码`74ea2200`；本轮未重跑 | 2026-10-15 | `.pi/tmp/b91-results.json`、`b91-full.log` |
| GUI/真实模型/MCP | 未验收 | 本地测试不能替代真实窗口验收 | — | — |

## 本次更新：B96 定稿后续生成五频道与专属准入控制

- 接通 `finalization-generation:read` / `begin` / `execute` / `commit` / `cancel` 全部 5 个频道；
- 注册命令数达到 262，覆盖 invoke 频道提升至 261 / 290（突破 90% 覆盖率，约 90.0%），未迁移频道降至 29 个；
- 专属定稿准入控制（`finalization_admission`）：
  - 通用 `generation:begin` 与 `generation:execute` 传入 `finalized-chapter-notes` 或 `finalized-character-state` 时一律拦截并返回 `GENERATION_FINALIZATION_ADMISSION_REQUIRED`；
  - 仅由专属命令入口显式置准入标志为 `true`，严防渲染层直调通用入口绕过。
- 槽位查找与轮次状态机（`finalization_generation.rs`）：
  - `find_finalization_run`：校验槽位格式，依据 `finalizationGenerationSlotKey` 遍历历史运行，严格断言重试命名与未提交效应；
  - `is_finalization_run_retryable`：检查已落盘效应、执行中预算与基线漂移；
  - 前序约束：校验 sibling 步骤，若 sibling 存在且尚未提交效应，抛出 `GENERATION_FINALIZATION_PREDECESSOR_REQUIRED`；
  - 任务与上下文冻结：在 `sourceManifest` 中固化 `finalizationGenerationContext`、`finalizationGenerationTask` 与 SHA-256 哈希。
- 效应落盘与连续性（`commit_finalization_generation`）：
  - 校验产物引用与状态（非 discarded、settled/unknown、stop）；
  - `chapter_notes`：校验 `notes_baseline` 未漂移，保存 finalized continuity facts 并同步更新 blueprint notes；
  - 生成不可变 `finalizationEffect: { version: 1, artifact, contextHash, receipt, receiptHash }` 写入 `generation_attempts.usage_receipt_json`；
  - 重复提交同一产物幂等返回，冲突产物报错 `GENERATION_FINALIZATION_EFFECT_CONFLICT`。
- 取消与重置（`cancel_finalization_generation`）：
  - 校验 handle，调用 `cancel(rootActionId)` 取消关联根动作，返回最新 run 视图。
- 测试覆盖：
  - Rust `finalization_generation`：11 项测试覆盖槽位查找、未找到返回 null、非法槽位拒绝、通用 begin 准入拦截、通用 execute 准入拦截、执行请求校验、取消身份校验；
  - 前端：5 项传输契约测试覆盖 read/begin/execute/commit/cancel 全部 5 频道的项目会话自动注入与参数打包。

## 后续工作与未解除边界

1. 下一候选切片：Agent/draft 跨会话在途收据×关闭重开×多会话导航、unknown result×父根生命周期。
2. 启动门禁已持久化于 `~/.lorekeeper/appearance.json`，真实 `tauri dev/build` 窗口尚未首验；`skin:import-custom` 仍返回 `CUSTOM_SKIN_UNAVAILABLE`，前端 `SKIN_SNAPSHOT_NOT_READY` 分类待补。
3. 剩余 29 频道：cloud-backup 8、finalized-character 5、graph-generation 5、character-avatar 4、dialog 4、project 3。归档 zip、头像 schema、云备份/凭据及新权限仍须先确认；不通过加映射绕过未迁移拒绝。
4. 前端 B91 全量的 7 个失败文件与 `f15a1b71` 完全一致，本轮仅跑相关合同，不宣告这些遗留失败已解除。

工作区、实际 HEAD 及下一活动队列以 Git 与 `.pi/tmp/pipeline-state.md` 为准。根 `src/`、`electron/`、`docs/` 及根依赖配置/lockfile 未改。最近全量基线为 1415/0，全量后 Rust 代码切片累计 4；本轮秘密扫描无发现。
