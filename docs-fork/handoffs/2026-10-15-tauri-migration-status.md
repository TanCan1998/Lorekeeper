# Tauri 迁移进度快照（2026-10-15）

## 已验证状态（第 58 次）

本轮代码：`8f6a98cb`，master。普通迁移由主会话直接完成，子代理派发继续停用。

| 指标 | 结果 | 验证版本/范围 | verifiedAt | 证据路径 |
|---|---|---|---|---|
| invoke/事件 | 契约290、事件7；注册257 → **覆盖256**，剩余34 | `8f6a98cb`，check:channels | 2026-10-15 | 终端输出 / check:channels |
| Rust全量（基线） | **1415 passed / 0 failed（26.63s）** | `4c9364c4`，cargo test | 2026-10-15 | `.pi/tmp/b92-full.log`（全量后累计3个切片） |
| Rust定向 | **20 passed / 0 failed** | `8f6a98cb`，cargo test import_generation | 2026-10-15 | 终端输出 |
| 全目标检查 | 退出 0，无警告 | `8f6a98cb`，cargo check --all-targets | 2026-10-15 | 终端输出 |
| 前端定向 | import-generation 传输契约2项通过 | `8f6a98cb`，vitest | 2026-10-15 | 终端输出 |
| TS/lint/格式 | typecheck、lint、定向rustfmt、diff检查退出0 | `8f6a98cb` | 2026-10-15 | 终端输出 |
| 最近前端非浏览器全量（沿用，非本轮） | 172文件/2053项：1929通过、123失败、1跳过 | B91，启动门禁代码`74ea2200`；本轮未重跑 | 2026-10-15 | `.pi/tmp/b91-results.json`、`b91-full.log` |
| GUI/真实模型/MCP | 未验收 | 本地测试不能替代真实窗口验收 | — | — |

## 本次更新：B95 导入生成双频道与导入槽位恢复准入

- 接通 `import-generation:read` 与 `import-generation:execute`；
- 命令名映射、TS 参数命名（`request`）、项目会话尾参自动注入（`isProjectScopedChannel`）与迁移清单齐备；覆盖从 254 增加至 256，未迁移频道降至 34。
- 专属导入准入控制（`import_admission`）：
  - 普通 `generation:execute` 传入包含 `importSlot` 的运行一律拦截并返回 `GENERATION_IMPORT_ADMISSION_REQUIRED`；
  - 仅由主进程内部专属命令入口（`import_generation_execute`）显式置准入标志为 `true`，严防渲染层直调通用入口绕过。
- 槽位查找与恢复（`read_import_generation`）：
  - 格式校验（`validate_import_generation_slot`），非法槽位报 `GENERATION_IMPORT_SLOT_INVALID`；
  - 根据 `importSlotKey` 与 `projectId` 过滤唯一定位运行；若多于1条抛出 `GENERATION_IMPORT_SLOT_AMBIGUOUS`，未找到返回 `null`，找到返回 `{ view, modelId, frozenContext }`。
- 执行与连续性约束（`execute_import_generation`）：
  - 序数与连续性：校验 `ordinal` 合法性与 `generation_attempts` 计数严格相等（不相等报 `GENERATION_IMPORT_ORDINAL_GAP`），且所有前序尝试必须为 `stop`（否则报 `GENERATION_IMPORT_PREVIOUS_ATTEMPT_INCOMPLETE`）；
  - 租约与执行权限：强制要求 `execution` 权威证明（缺失报 `GENERATION_IMPORT_EXECUTION_REQUIRED`），校验 `import_runs` 状态为 running 且无 cancel 请求，以及 owner 与 epoch 严格匹配（不符报 `GENERATION_IMPORT_EXECUTION_STALE`）；
  - 效应不可变性：若该槽位已有提交的 `effect_receipt`，拒绝重复执行并报 `GENERATION_IMPORT_EFFECT_SEALED`。
- 测试覆盖：
  - Rust `import_generation`：20 项测试覆盖槽位阶段序列化、上下文序列化、槽位校验、检查点解析/构造、槽位查找失败返回 null、非法槽位拒绝、缺少 execution 权限拦截、通用 execute 准入拦截；
  - 前端：2 项传输契约测试覆盖 read/execute 的项目会话自动注入与参数对象打包。

## 后续工作与未解除边界

1. 下一候选切片：Agent/draft 跨会话在途收据×关闭重开×多会话导航、unknown result×父根生命周期。
2. 启动门禁已持久化于 `~/.lorekeeper/appearance.json`，真实 `tauri dev/build` 窗口尚未首验；`skin:import-custom` 仍返回 `CUSTOM_SKIN_UNAVAILABLE`，前端 `SKIN_SNAPSHOT_NOT_READY` 分类待补。
3. 剩余 34 频道：cloud-backup 8、finalization-generation 5、finalized-character 5、graph-generation 5、character-avatar 4、dialog 4、project 3。归档 zip、头像 schema、云备份/凭据及新权限仍须先确认；不通过加映射绕过未迁移拒绝。
4. 前端 B91 全量的 7 个失败文件与 `f15a1b71` 完全一致，本轮仅跑相关合同，不宣告这些遗留失败已解除。

工作区、实际 HEAD 及下一活动队列以 Git 与 `.pi/tmp/pipeline-state.md` 为准。根 `src/`、`electron/`、`docs/` 及根依赖配置/lockfile 未改。最近全量基线为 1415/0，全量后 Rust 代码切片累计 3；本轮秘密扫描无发现。
