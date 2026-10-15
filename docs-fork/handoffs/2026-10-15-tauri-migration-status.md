# Tauri 迁移进度快照（2026-10-15）

## 已验证状态（第 62 次 —— B98 切片：定稿角色状态候选读面与决策）

本轮代码：`3feadeb5`，master。普通迁移由主会话直接完成，子代理派发继续停用。

| 指标 | 结果 | 验证版本/范围 | verifiedAt | 证据路径 |
|---|---|---|---|---|
| invoke/事件 | 契约290、事件7；注册271 → **覆盖270**，剩余20 | `3feadeb5`，check:channels | 2026-10-16 | 终端输出 / check:channels |
| **Rust全量（最新基线，沿用）** | **1448 passed / 0 failed（58.63s）** | `bc078492`，cargo test | 2026-10-15 | `.pi/tmp/b97-full.log`（本轮后累计1个切片） |
| Rust定向 | finalized_character 8、character_roster 19、finalized_continuity 10、finalization_generation 11、main_generation_owner 175，全部0失败 | `3feadeb5`，cargo test --lib | 2026-10-16 | `.pi/tmp/b98-test3.log`、`b98-test-*.log` |
| 全目标检查 | 退出 0，无警告 | `3feadeb5`，cargo check --all-targets | 2026-10-16 | `.pi/tmp/b98-all-targets.log` |
| 前端定向 | 新增契约测试4项 + 特殊域边界用例更新后 2 文件 8 项全部通过 | `3feadeb5`，vitest | 2026-10-16 | `.pi/tmp/b98-vitest3.log` |
| TS/lint/格式 | typecheck、lint、check:channels、定向rustfmt退出0 | `3feadeb5` | 2026-10-16 | 终端输出 |
| 最近前端非浏览器全量（沿用，非本轮） | 172文件/2053项：1929通过、123失败、1跳过 | B91，启动门禁代码`74ea2200`；本轮未重跑 | 2026-10-15 | `.pi/tmp/b91-results.json`、`b91-full.log` |
| GUI/真实模型/MCP | 未验收 | 本地测试不能替代真实窗口验收 | — | — |

## 本次更新：B98 定稿角色状态候选读面与决策（finalized-character 4 频道）

本切片把 `finalized-character` 域的**读面与候选裁决**从上游 1:1 平移进 Tauri 侧，
承接 B96/B97 的定稿生成链路（同一 `FinalizedCharacterContext` 与冻结身份快照）：

1. **来源与哈希口径逐字段对齐**：
   - `characterStateCandidateKey` 复原为 `fcs-state:sha256(JSON.stringify([draftId, finalizationId,
     contentHash, characterId, field, sha256(value)]))`（此前半成品用冒号拼接，与基线不符）；
   - `stateDecisionPayloadHash` 复原为八元组数组 JSON 的 SHA-256（含 `operationId`/`decision`）。
2. **候选行的来源失效判定**：`readCurrentCharacterStateCandidateRow` 要求候选行来源与冻结定稿源同一、
   投影代不领先于当前水位、落后时该章未进入 `stale_from_chapter` 区间、且草稿仍是本章最新定稿稿，
   否则 `FINALIZED_CHARACTER_STATE_SOURCE_CHANGED`；列表频道对失效行静默跳过，不污染其它章候选。
3. **待决候选投影与历史行升级**：未绑定元数据的旧候选在 `read-state-candidate` 读取时按当前角色行补齐
   `candidateKey` / `source` / `expectedFieldRevision` / `expectedFieldValueHash` 并写回 store；
   冻结快照缺失、身份不在冻结名单内或字段版本非法时不作为待决候选暴露。
4. **决策与幂等**：`accept` 走 CAS（revision + 值哈希 + provenance 全等）后写 `cs_<field>` 与
   `author` provenance（章节号 + 新版本号），并按基线推进角色状态投影；`decline` 只落收据。
   同一 `operationId` 重放同载荷返回 `idempotent:true`，异载荷报 `FINALIZED_CHARACTER_STATE_OPERATION_CONFLICT`。
5. **`read-context`**：`draftId` 校验后签发 `contextId`（`randomUUID`）、上下文进入 64 条插入序 LRU，
   `originProjectId` 取自已验证的便携转移权威（读取失败向上传播）；项目关闭时清空缓存。

**接入面**：`commands/generation.rs` 新增 4 个命令并在 `lib.rs` 注册；owner 新增 4 个处理器与
上下文缓存字段；前端 `ipc-client` 登记 3 个带参频道（`list-state-candidates` 无参不登记），
`migrated-channels.ts` 重新生成（270 个频道）。

**复用面调整**：`refresh_character_state_projection` 补齐；`finalized_continuity_repository` 的候选
store / 收据 / 决策枚举与解析函数、`finalization_generation_source` 的冻结快照与 `stateSnapshot`
提升为 `pub(crate)`/`pub` 供本服务复用（无重复实现）。

**测试**：Rust 新增 8 项（候选键与载荷哈希稳定性、列表/读取/决策全链路、CAS 冲突、来源失效与
参数校验、owner 请求形状与关闭清理）；前端新增 4 项契约测试，并把「未开放特殊域边界」回归从已迁移
频道切换到仍未迁移的 `finalized-character:commit` 与 `character-avatar`（该文件此前已因 B96/B97 过时失败）。

## 后续工作与未解除边界

1. 下一阶段活动候选：
   - B99：`finalized-character:commit`（产物来源证明 `proveFinalizedCharacterGeneration` +
     `finalized-character-state` 生成准入 + 角色提案暂存与 `unresolved` 回传）；
   - Agent/draft 跨会话在途收据×关闭重开×多会话导航、unknown result×父根生命周期；
   - 真实 `tauri dev`/`build` 窗口首验与启动门禁（`SKIN_SNAPSHOT_NOT_READY` 分类）。
2. 剩余 20 个未迁移频道全景：
   - `cloud-backup`（8 频道）：云备份连接、备份、恢复、凭据管理（需安全存储与网络策略确认）；
   - `character-avatar`（4 频道）：角色头像选取、裁剪、批量读取与删除（需图片存储方案确认）；
   - `dialog`（4 频道）：项目归档选择等文件弹窗；
   - `project`（3 频道）：归档导出/恢复/旧版导入（需 ZIP 归档方案确认）；
   - `finalized-character:commit`（1 频道）：随 B99 一并完成。
3. 根 `src/`、`electron/`、`docs/` 及根依赖配置/lockfile 保持完全干净；本地 gitleaks 扫描无发现。
