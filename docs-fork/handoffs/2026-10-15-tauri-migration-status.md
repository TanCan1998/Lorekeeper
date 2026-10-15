# Tauri 迁移进度快照（2026-10-15）

## 已验证状态（第 57 次）

本轮代码：`504d1dd6`，master。普通迁移由主会话直接完成，子代理派发继续停用。

| 指标 | 结果 | 验证版本/范围 | verifiedAt | 证据路径 |
|---|---|---|---|---|
| invoke/事件 | 契约290、事件7；注册255 → **覆盖254**，剩余36 | `504d1dd6`，check:channels | 2026-10-15 | 终端输出 / check:channels |
| Rust全量（基线） | **1415 passed / 0 failed（26.63s）** | `4c9364c4`，cargo test | 2026-10-15 | `.pi/tmp/b92-full.log`（全量后累计2个切片） |
| Rust定向 | **27 passed / 0 failed** | `504d1dd6`，cargo test editor_inline | 2026-10-15 | 终端输出 |
| 全目标检查 | 退出 0，无警告 | `504d1dd6`，cargo check --all-targets | 2026-10-15 | 终端输出 |
| 前端定向 | editor-inline 传输契约4项通过 | `504d1dd6`，vitest | 2026-10-15 | 终端输出 |
| TS/lint/格式 | typecheck、lint、定向rustfmt、diff检查退出0 | `504d1dd6` | 2026-10-15 | 终端输出 |
| 最近前端非浏览器全量（沿用，非本轮） | 172文件/2053项：1929通过、123失败、1跳过 | B91，启动门禁代码`74ea2200`；本轮未重跑 | 2026-10-15 | `.pi/tmp/b91-results.json`、`b91-full.log` |
| GUI/真实模型/MCP | 未验收 | 本地测试不能替代真实窗口验收 | — | — |

## 本次更新：B94 编辑器行内生成四频道与专属准入控制

- 接通 `editor-inline:begin`、`editor-inline:read-recovery`、`editor-inline:execute`、`editor-inline:cancel`；
- 命令名映射、TS 参数命名（`request`）、项目会话尾参自动注入（`isProjectScopedChannel`）与迁移清单齐备；覆盖从 250 增加至 254，未迁移频道降至 36。
- 专属准入控制（`editor_inline_admission`）：
  - 普通 `generation:begin` / `generation:execute` 传入 `operation == "editor-inline"` 一律拦截并返回 `GENERATION_EDITOR_ADMISSION_REQUIRED`；
  - 仅由主进程内部专属命令入口（`editor_inline_begin` / `editor_inline_execute`）显式置准入标志为 `true`，严防渲染层直调通用入口绕过。
- 任务构造完全对齐基线：
  - 提示词模板固定为 `edit_selected_text`，按动作分流（refine / expand / continue / dialogue）；
  - 中英文双语指令与系统角色不可变合同（不可覆盖）；
  - 任务属性：`purpose: editor-ai-${action}`，`refine` 时为 `review` 阶段，其余动作为 `drafting` 阶段，输出格式为 `visible-text`。
- 状态机与安全防护：
  - 严格输入校验：非对象、未知键、空/超长 `uiActionNonce`、越界或负数 from/to、切片正文不匹配、超过 8MB 文本等均报 `GENERATION_EDITOR_INPUT_INVALID`；
  - 幂等性：基于 `projectId` 与 `uiActionNonce` 查询 `generation_roots`，输入与模型一致时幂等返回已有 recovery，冲突时返回 `GENERATION_NONCE_CONFLICT`；
  - 状态恢复：读取 recovery 时根据 `rebuild_binding` 动态判定 `sourceStatus`（`current` 或 `conflict`）；
  - 单次尝试约束：`prepare_editor_inline_execution` 强制校验 `generation_attempts` 尝试记录数必须为 0，防止非重放重复执行（报 `GENERATION_EDITOR_ATTEMPT_CONFLICT`）；
  - `cancel_editor_inline` 标记根操作取消，再次读取时自动标记为 conflict。
- 测试覆盖：
  - Rust `editor_inline_generation`：23 项测试覆盖输入校验、上下文构造、任务构建、语言分流、上下文读取与哈希反欺诈校验；
  - Rust `main_generation_owner`：4 项测试覆盖非法入参拒绝、通用 begin 准入拦截、非 editor-inline handle 拒绝、既有尝试冲突拦截；
  - 前端：4 项传输契约测试覆盖 begin/read-recovery/execute/cancel 的项目会话自动注入与参数对象打包。

## 后续工作与未解除边界

1. 下一候选切片：`import-generation:read` / `import-generation:execute` 2 频道（派生 generation 导入切片）。
2. Agent/draft 在途收据×关闭重开×多会话导航、unknown result×父根生命周期。
3. 启动门禁已持久化于 `~/.lorekeeper/appearance.json`，真实 `tauri dev/build` 窗口尚未首验；`skin:import-custom` 仍返回 `CUSTOM_SKIN_UNAVAILABLE`，前端 `SKIN_SNAPSHOT_NOT_READY` 分类待补。
4. 剩余 36 频道：cloud-backup 8、finalization-generation 5、finalized-character 5、graph-generation 5、character-avatar 4、dialog 4、project 3、import-generation 2。归档 zip、头像 schema、云备份/凭据及新权限仍须先确认；不通过加映射绕过未迁移拒绝。
5. 前端 B91 全量的 7 个失败文件与 `f15a1b71` 完全一致，本轮仅跑相关合同，不宣告这些遗留失败已解除。

工作区、实际 HEAD 及下一活动队列以 Git 与 `.pi/tmp/pipeline-state.md` 为准。根 `src/`、`electron/`、`docs/` 及根依赖配置/lockfile 未改。最近全量基线为 1415/0，全量后 Rust 代码切片累计 2；本轮秘密扫描无发现。
