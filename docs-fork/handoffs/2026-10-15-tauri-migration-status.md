# Tauri 迁移进度快照（2026-10-15）

## 已验证状态（第 61 次 —— 阶段收口快照）

本轮代码：`bc078492`，master。普通迁移由主会话直接完成，子代理派发继续停用。

| 指标 | 结果 | 验证版本/范围 | verifiedAt | 证据路径 |
|---|---|---|---|---|
| invoke/事件 | 契约290、事件7；注册267 → **覆盖266**，剩余24 | `bc078492`，check:channels | 2026-10-15 | 终端输出 / check:channels |
| **Rust全量（最新基线）** | **1448 passed / 0 failed（58.63s）** | `bc078492`，cargo test | 2026-10-15 | `.pi/tmp/b97-full.log`（全量后累计0个切片） |
| 全目标检查 | 退出 0，无警告 | `bc078492`，cargo check --all-targets | 2026-10-15 | 终端输出 |
| 前端定向 | 新增5套契约测试18项全部通过 | `bc078492`，vitest | 2026-10-15 | 终端输出 |
| TS/lint/格式 | typecheck、lint、定向rustfmt、diff检查退出0 | `bc078492` | 2026-10-15 | 终端输出 |
| 最近前端非浏览器全量（沿用，非本轮） | 172文件/2053项：1929通过、123失败、1跳过 | B91，启动门禁代码`74ea2200`；本轮未重跑 | 2026-10-15 | `.pi/tmp/b91-results.json`、`b91-full.log` |
| GUI/真实模型/MCP | 未验收 | 本地测试不能替代真实窗口验收 | — | — |

## 本次更新：五切片阶段收口与全量回归验收

本波次由主会话直接连续完成了 5 个关键切片的迁移、接入与全量验收，无缝贯通了主生成 Owner 下的派生子生成器体系与离线概览能力：

1. **B93：项目概览只读投影与离线预览凭证（2 频道）**
   - 接入 `project:overview-current` 与 `project:peek-overview`；
   - 实现了纯只读投影，严格核验 `finalization_outbox` 定稿章节哈希与快照一致性；
   - 沙箱临时库防篡改与离线能力凭证。
2. **B94：编辑器行内生成四频道与专属准入控制（4 频道）**
   - 接入 `editor-inline:begin` / `read-recovery` / `execute` / `cancel`；
   - 引入 `editor_inline_admission` 专属准入，拦截渲染层通过通用 `generation:begin` / `execute` 绕过；
   - 严格单次 attempt 约束与幂等防重。
3. **B95：导入生成双频道与导入槽位恢复准入（2 频道）**
   - 接入 `import-generation:read` / `import-generation:execute`；
   - 引入 `import_admission` 专属准入，保障槽位恢复与序数严格连续性（`GENERATION_IMPORT_ORDINAL_GAP`）；
   - 执行租约权威与已提交效应不可变性（`GENERATION_IMPORT_EFFECT_SEALED`）。
4. **B96：定稿后续生成五频道与专属准入控制（5 频道）**
   - 接入 `finalization-generation:read` / `begin` / `execute` / `commit` / `cancel`；
   - 引入 `finalization_admission` 专属准入，前序 sibling 效应断言与槽位轮次状态机；
   - `chapter_notes` 基线校验、连续性事实落盘与 `blueprint notes` 级联更新。
5. **B97：剧情图生成五频道与专属准入控制（5 频道）**
   - 接入 `graph-generation:read` / `begin` / `execute` / `confirm` / `cancel`；
   - 引入 `graph_admission` 专属准入，幂等查重与单次 attempt 强约束；
   - 双阶段冲突检测与大纲/计划/事件三分支确认落盘。

**全量回归证据**：
- 阶段累计 5 个 Rust 切片，触发全量 `cargo test` 回归；
- 测试总数从 1415 增加到 **1448 项，100% 全部通过，0 项失败**，耗时 58.63 秒；
- 完整日志归档于 `.pi/tmp/b97-full.log`；
- 全量后切片累计数重新归零（0）。

## 后续工作与未解除边界

1. 下一阶段活动候选：
   - Agent/draft 跨会话在途收据×关闭重开×多会话导航、unknown result×父根生命周期；
   - 真实 tauri dev/build 窗口首验与启动门禁（SKIN_SNAPSHOT_NOT_READY 分类）。
2. 剩余 24 个未迁移频道全景：
   - `cloud-backup`（8 频道）：云备份连接、备份、恢复、凭据管理（需安全存储与网络策略确认）；
   - `finalized-character`（5 频道）：定稿角色状态候选裁决与提交；
   - `character-avatar`（4 频道）：角色头像选取、裁剪、批量读取与删除（需图片存储方案确认）；
   - `dialog`（4 频道）：项目归档选择等文件弹窗；
   - `project`（3 频道）：归档导出/恢复/旧版导入（需 ZIP 归档方案确认）。
3. 根 `src/`、`electron/`、`docs/` 及根依赖配置/lockfile 保持完全干净；本轮秘密扫描无发现。
