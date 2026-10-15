# Tauri 迁移进度快照（2026-10-15）

## 已验证状态（第 56 次）

本轮代码：`580225ad`，master。普通迁移由主会话直接完成，子代理派发继续停用。

| 指标 | 结果 | 验证版本/范围 | verifiedAt | 证据路径 |
|---|---|---|---|---|
| invoke/事件 | 契约290、事件7；注册251 → **覆盖250**，剩余40 | `580225ad`，check:channels | 2026-10-15 | 终端输出 / check:channels |
| Rust全量（基线） | **1415 passed / 0 failed（26.63s）** | `4c9364c4`，cargo test | 2026-10-15 | `.pi/tmp/b92-full.log`（全量后累计1个切片） |
| Rust定向 | **14 passed / 0 failed** | `580225ad`，cargo test project_overview & project_peek | 2026-10-15 | 终端输出 |
| 全目标检查 | 退出 0，无警告 | `580225ad`，cargo check --all-targets | 2026-10-15 | 终端输出 |
| 前端定向 | 概览/会话传输12项通过 | `580225ad`，vitest | 2026-10-15 | 终端输出 |
| TS/lint/格式 | typecheck、lint、定向rustfmt、diff检查退出0 | `580225ad` | 2026-10-15 | 终端输出 |
| 最近前端非浏览器全量（沿用，非本轮） | 172文件/2053项：1929通过、123失败、1跳过 | B91，启动门禁代码`74ea2200`；本轮未重跑 | 2026-10-15 | `.pi/tmp/b91-results.json`、`b91-full.log` |
| GUI/真实模型/MCP | 未验收 | 本地测试不能替代真实窗口验收 | — | — |

## 本次更新：B93 项目概览只读投影与离线预览能力凭证

- 接通 `project:overview-current` 与 `project:peek-overview`，命令名、TS 参数映射、IPC 客户端注入与迁移清单齐备；覆盖从 248 增加至 250，未迁移频道降至 40。
- `project_overview_repository::read` 遵循纯只读投影契约：不建表、不运行迁移、不可反向写入；若缺少 7 个核心表之一或缺少 `project_core` main 行则立即返回相应不可用状态。
- 定稿章节统计严格与 `finalization_outbox` 校验：只有草稿处于 finalized 状态且与 outbox 收据（receipt 存在、章节号匹配、正文快照一致、sha256 校验一致）完全吻合时才计入 finalizedChapters。
- 摘要提取（`excerpt`）仅在当前打开项目（current）且参数开启时提供最新草稿正文尾部 240 字符；离线预览默认不暴露正文摘要。
- 离线预览能力（`ProjectPeek`）：
  - 仅由主进程在生成最近项目列表时签发不透明 UUID 凭证，禁止前端传路径任意签发或直接探测文件系统；
  - 预览前通过只读打开建立沙箱临时目录（Scratch）并做快照，开启 `PRAGMA query_only=ON` 与 `PRAGMA trusted_schema=OFF`；
  - 严密防篡改：签发后源 `project.json` 被改动、源数据库文件被篡改、或源目录存在 hot rollback journal（`lorekeeper.db-journal`）均立即拒绝并返回 `{"state":"unavailable"}`；
  - 进程退回或从最近列表中移除时，即刻撤销对应路径的能力凭证；`clear` 清空全部凭证。
- 测试覆盖：
  - Rust `project_overview_repository`：空 ID 拒绝、缺少 schema 拒绝、缺少 core 拒绝、空阶段统计、全流程阶段进展、定稿凭证强校验、尾部截断 240 字符共 7 项测试；
  - Rust `project_peek`：签发与生命周期、无效 manifest 拒绝、未知凭证拒绝、数据库损坏拦截、清单篡改拦截、hot rollback journal 拦截、清空凭证共 7 项测试；
  - 前端：`project:overview-current` 会话上下文注入与 `project:peek-overview` 纯凭证无会话传输 2 项契约测试。

## 后续工作与未解除边界

1. 下一候选切片：Agent/draft 跨会话在途收据×关闭重开×多会话导航、unknown result×父根生命周期。
2. 启动门禁已持久化于 `~/.lorekeeper/appearance.json`，真实 `tauri dev/build` 窗口尚未首验；`skin:import-custom` 仍返回 `CUSTOM_SKIN_UNAVAILABLE`，前端 `SKIN_SNAPSHOT_NOT_READY` 分类待补。
3. 剩余 40 频道：cloud-backup 8、finalization-generation 5、finalized-character 5、graph-generation 5、character-avatar 4、dialog 4、editor-inline 4、project 3、import-generation 2。归档 zip、头像 schema、云备份/凭据及新权限仍须先确认；不通过加映射绕过未迁移拒绝。
4. 前端 B91 全量的 7 个失败文件与 `f15a1b71` 完全一致，本轮仅跑相关合同，不宣告这些遗留失败已解除。

工作区、实际 HEAD 及下一活动队列以 Git 与 `.pi/tmp/pipeline-state.md` 为准。根 `src/`、`electron/`、`docs/` 及根依赖配置/lockfile 未改。最近全量基线为 1415/0，全量后 Rust 代码切片累计 1；本轮秘密扫描无发现。
