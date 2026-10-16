# Tauri 迁移进度快照（2026-10-16）

> 入口 [CURRENT.md](./CURRENT.md)；规则见 [开发指南 §9](../agents/pi-development.md)。

## 已验证状态（第 73 次 —— 真实审稿复核链与合并事务接线）

- 提交前核对时间：2026-10-16；master，基线 HEAD 为 `49b91ee0`，本地 origin/master 为 `34662f72`。此前检查点已提交，旧交接指针已纠正。
- 本轮修改五个 Rust 文件（含新增链路测试文件）；用户本次明确要求提交并推送，按进行中检查点保存。完整 proveEffect 的跨身份与多段续写专项审计仍未结束，最终提交/推送结果见本机 pipeline。
- 主会话直接执行，无子代理、无登记在飞工作；生产审稿四频道继续关闭。

| 检查 | 本轮结果 | 范围与证据（本机，不入库） |
|---|---|---|
| Rust 全量 | **1581 通过，0 失败**；库 62.27 秒 | HEAD + 本轮五文件工作区；.pi/tmp/recovery/chain-full.log |
| 扩大编译 | cargo check --all-targets 通过，无告警 | 同一最终代码；.pi/tmp/recovery/chain-check.log |
| 审稿域定向 | 中间版本 42 通过 | 后续新增来源篡改、冻结历史两项，均由最终全量覆盖 |
| 修稿仓储定向 | 10 通过 | .pi/tmp/recovery/chain-revision.log；后续仅增加冻结测试辅助 |
| 格式与差异 | 五文件 rustfmt --check、git diff --check 通过 | 最终 Rust 工作区，工具输出 |
| 前端与 GUI | 未重测 | 无 TS/频道开放改动；不沿用为本轮验收 |

## 本次更新

- 以真实 SQLite、生成账本和服务构建初审 → 人工确认 → 修稿 → 合并 → 复核合法链。测试材料全部为合成数据。
- 发现并修复合并接线缺口：将基线 commitMergedRevision 补入周期仓储，由修稿合并事务同步推进周期、保存不可变合并快照；重复合并也验证周期一致性。
- 新增快照写入失败回滚测试：作者正文、草稿状态、修稿 pending 状态和周期同时恢复，重试可成功。
- 从持久 finding 和合并快照重建复核准入/context，逐项比较冻结集合、报告身份与完整回执映射，校验最终 finding 状态、evidenceHash 和 confirmation_item_index。
- 复核历史来源版本改为读取审稿冻结字段，不再依赖后来可能改变的当前草稿版本。
- 强化修稿完整来源字段、确认 snapshot 与正文、authorInputs/冻结上下文一致性及 attempt/root 身份；补产物 revision、epoch、各段来源收据及正文有效增长检查。
- 新增六项链路回归：合法复核、模型声称解决/无效证据保持 unknown、缺失/重复/伪造映射与持久状态篡改、合并回滚、自洽重算哈希后的来源伪造、脱敏冻结历史不可重放。
- 脱敏专项使用真实 sanitize、freeze_document 与正式连接触发器实现；仅增加 cfg(test) 安装辅助，不改变生产权限/准入规则。保留作者正文与三份审稿可读事实，originalReceiptVerified 始终 false。

### 尚未解除的边界

1. **尚不宣布完整 proveEffect 等价**：继续补多段修稿续写的独立正反回归、跨 root/run 组合、历史策略版本及必现/豁免混合场景。
2. 已新增脱敏链路专项，但不等同于 GUI 导入恢复全链验收或真实模型验收。
3. 审稿四个生产前端频道继续关闭；本次明确授权仅保存并推送进行中检查点，不代表完整切片验收。未改 schema、依赖、生产权限、ADR、根配置/锁文件、Electron 或上游 docs。

### 沿用范围（非本轮新验证）

- 结构覆盖 289/290、前端开放 285/290 沿用历史口径，不是产品完成率。
- 前端全量沿用 `4c03d515`：2101 通过/0 失败/1 跳过；GUI/build 沿用 `a6655558`，均非当前 Rust 工作区的新验收。

## 下一步

1. 核对最新 HEAD、工作区和 pipeline 指纹，本轮全量覆盖的 Rust 文件指纹不变。
2. 优先补双段/重叠/无增长续写与首段来源篡改测试，再覆盖历史策略、必现及作者豁免混合场景。
3. 继续完成效果证明审计并按既有授权自检、提交及推送；完整验收前不开放频道。

活动状态与文件指纹见 `.pi/tmp/pipeline-state.md`。最新全量为本轮最终代码，之后仅交接文档变化；全量后 Rust 语义切片累计 0。

---

以下为前次收口历史。第65次“云凭据方案未定”现已由用户DPAPI选择与本切片替代。

## 历史状态（第 65 次 —— 冻结连接门禁与归档恢复频道）

- 代码提交：`f4ef5e00`（连接级冻结门禁）、`f42ae6f1`（恢复频道），均已推送。
- 用户已确认启用 rusqlite 的 functions feature 与连接级 TEMP schema；
  实际仅修改 Tauri Cargo feature，**没有新增 crate、没有修改锁文件或持久 schema**。
- 每次打开恢复项目，先验证清单、转移权威与历史，再注册连接级守卫与 TEMP 触发器。
  冻结行的更新、删除、同键插入、UPSERT、REPLACE受保护；借冻结执行父身份续写也被拒绝，
  新身份可正常创建/更新。recursive_triggers确保REPLACE的隐式唯一键删除也受检查。
  门禁随连接关闭消失，重新打开重新准入；普通项目保持原行为。
- `project:archive-restore` 已注册：消费archive read与target create授权，不注入活动项目会话，
  不切换当前项目；阻塞线程完成恢复，最近项目登记失败保留成功收据及警告。
  同时修复既有导出失败信封的errorCode camelCase序列化。
- **本轮最新验证**（`f42ae6f1` 对应工作区）：
  - Rust全量 **1535 passed / 0 failed**，库测试59.69s，main/doc tests无测试；
    `.pi/tmp/b113c-full.log`。全量后累计Rust切片 **0**。
  - `cargo check --all-targets` 无告警：`.pi/tmp/b113c-check.log`。
  - 归档命令定向9项：`.pi/tmp/b113c-command.log`。
  - 前端3文件8项：`.pi/tmp/b113c-vitest.log`；
    typecheck与lint退出0：`.pi/tmp/b113c-typecheck.log`、`.pi/tmp/b113c-lint.log`。
  - 频道实测 **290 invoke / 7事件 / 282注册 / 281覆盖 / 9未迁移**：
    `.pi/tmp/b113c-channels.log`。剩余cloud-backup八频道与project:import-legacy-copy。
  - 定向rustfmt、diff检查及根src/Electron/docs/配置/lockfile隔离检查通过。
- **下一确认边界**：
  - 旧版副本导入：上游读取 .vela/vela.db 及 LanceDB；当前本项目决定是不与基线互通、
    无需解码旧库，因此不能默认为跨栈一次性导入，也不能注册不可用桩冒充覆盖。
  - 云备份：凭据安全存储方案与平台范围（keyring或Windows DPAPI）未定。
- GUI/真实模型/MCP仍未验收，不能以本地单元测试替代；前端非浏览器全量沿用B91历史结果。

---

以下为第64次收口时的历史描述，其中“门禁未接线、恢复频道未开放”已被本次实现替代。

## 历史状态（第 64 次 —— 便携冻结证据与恢复安装服务）

- 最新代码 `29c63e11`；存储根修复 `0b746ce2` 已推送，统一从 Tauri 项目的 `.lore` 读取。
- B109–B112 与 B113a 已在前一接续区间完成：文件对话框、头像、ZIP 容器、字段策略、脱敏副本、
  原始资产与导出频道、恢复计划。该区间详情以对应提交与本机 pipeline 记录为准；
  不把前端/频道旧实测归给本轮代码。
- 本轮 B113b：恢复**服务层**已完成，频道未开放：
  - 导出新增 `portable-runtime-freeze.json`，只含脱敏历史行身份、空投影、排除字段名；
    不携带原始收据及机器字段摘要，不宣称原始收据已验证。
  - 转移权威与冻结证据来自同一脱敏副本；恢复要求唯一侧车，逐行重建比对冻结集合、
    来源元组与 manifest 投影引用，缺失或漂移即拒绝。
  - 所有权清单贯穿解包、校验、移动、写出与清理；安装日志和目标标记以硬链接共享身份。
  - 新项目 UUID v4、转移权威目标映射、新 `project.json`、断点续装、无覆盖发布。
  - FTS5 重建后，幂等识别比较业务结构与行证据，而不是仍要求归档库原始物理哈希。
    只有与编译期 DDL 一致的 FTS 对象可排除；业务篡改不得冒充同一安装。
- **本轮验证**（`29c63e11` 对应工作区）：
  - Rust 全量 **1530 passed / 0 failed**（库测试 62.41s，main/doc tests 无测试）；
    `.pi/tmp/b113b-full.log`。全量后累计 Rust 代码切片数 **0**。
  - 便携定向 **71 passed / 0 failed**，`.pi/tmp/b113b-portable.log`；
    全量前进一步补了既有恢复计划测试的缺失侧车断言，已由最终全量覆盖。
  - `cargo check --all-targets` 无告警，`.pi/tmp/b113b-all-targets.log`；
    定向 rustfmt 与 `git diff --check` 通过。
  - 本轮未改 TS，未重跑前端与频道脚本。频道覆盖沿用 `df756fed` 的 **280/290**，
    剩余 10（cloud-backup 8、project:archive-restore、project:import-legacy-copy）。
- **未解除阻塞**：运行时仓储默认冻结回调仍未接线，不能开放恢复频道或宣称恢复项目已可安全写入。
  拟在项目数据库连接上注册冻结守卫函数及 TEMP 触发器，拒绝已有历史行的更新、删除和同键替换，
  并拦截以冻结生成父身份续接的新账本写入；新身份的正常创作不受影响。
  此方案涉及 rusqlite functions feature 与连接级 TEMP schema，须先确认；尚未实施。
- 云备份凭据安全存储、旧版导入边界及真实窗口验收仍在后续队列。
- 本轮未修改 Electron、根 src、上游 docs、根配置与根 pnpm-lock.yaml，未派发子代理。

---

以下保留第 63 次快照的历史明细与验证表，**不是本轮最新状态**。

## 历史状态（第 63 次 —— B99–B108 十切片收口：定稿角色提交与生成路径活化）

本轮为**交接收口切片**：B99–B108 的代码已在 2026-10-16 02:02–09:51 之间逐个提交并推送，
但交接文档仍停在 B98（第 62 次）。本轮不新增代码，只补齐快照、入口与本机实测证据；
下表"本轮实测"行均为本次收集的真实运行结果。

| 指标 | 结果 | 验证版本/范围 | verifiedAt | 证据路径 |
|---|---|---|---|---|
| invoke/事件 | 契约290、事件7；注册272 → **覆盖271**，剩余19 | `712ae602`，check:channels | 2026-10-16 | `.pi/tmp/b63-channels.log` |
| **Rust全量（最新基线）** | **1480 passed / 0 failed（60.02s）** | `712ae602`，cargo test | 2026-10-16（B108 自检） | `.pi/tmp/b108-full.log`（全量后累计代码切片数：**0**） |
| Rust全量（区间内过程记录） | B99 1462、B103 1478、B106 1479、B107 1480、B108 1480，均 0 失败 | 各切片提交前 | 2026-10-16 | `.pi/tmp/b99-full.log`、`b103-full.log`、`b106-full.log`、`b107-full.log`、`b108-full.log` |
| Rust定向（本轮实测） | agent_generation 56、main_generation_owner 180，0 失败 | `712ae602`，cargo test --lib | 2026-10-16 | `.pi/tmp/b108-test2.log`、`b108-test-main_generation_owner.log` |
| 全目标检查（本轮实测） | 退出 0，无 error/warning | `712ae602`，cargo check --all-targets | 2026-10-16 | `.pi/tmp/b63-at.log` |
| 前端定向（本轮实测） | 2 文件 9 项通过 | `712ae602`，vitest | 2026-10-16 | `.pi/tmp/b63-vitest.log` |
| TS/lint/频道自检（本轮实测） | typecheck、lint、check:channels 退出 0 | `712ae602` | 2026-10-16 | `.pi/tmp/b63-typecheck.log`、`b63-lint.log`、`b63-channels.log` |
| 最近前端非浏览器全量（沿用，非本轮） | 172文件/2053项：1929通过、123失败、1跳过 | B91，启动门禁代码 `74ea2200`；本轮未重跑 | 2026-10-15 | `.pi/tmp/b91-results.json`、`b91-full.log` |
| GUI/真实模型/MCP | 未验收 | 本地测试不能替代真实窗口验收 | — | — |

## 本次更新：B99–B108 十切片明细（均已提交并推送）

1. **B99 `b9a5ffd7` — 定稿角色状态提交与来源证明（`finalized-character:commit`，本域最后 1 个频道）**：
   平移 `parseFinalizedCharacterStateResponse`（UTF-16 证据定位/唯一性/身份判定）与
   `SummaryRepository.commitFinalizedCharacterStates`（派生补丁判定、作者保护候选、候选落库与状态投影推进）；
   新增 `finalized_character_proof` 模块（`proveFinalizedCharacterGeneration` /
   `normalizeFinalizedCharacterProposalSource`）并接线角色提案 finalized 分支与来源时效断言；
   owner 新增 `admit_finalized_character` 生成准入（`finalizedCharacterContextHash` 进入源绑定）；
   领域提交与效应落盘统一在调用方 IMMEDIATE 事务内完成。新增 14 项 Rust 单测 + 前端契约测试。
2. **B100 `e0c2a39e` — 角色提案解析器移植**：1:1 平移 `character-proposal-parser.ts`
   （`extractSingleCompleteJsonObject`、身份清单/槽位/详情解码、素材抽取诊断、
   12000 UTF-16 码元分块、架构与规划两条解析链），解除三处 PENDING 桩。新增 9 项 Rust 单测。
3. **B101 `d62f0682` — 导入推演解码与语法修复证据保持**：平移 `import-inference-contract.ts` 解码链与
   `structured-syntax-repair.ts` 的证据保持判定，接线 character_proposal 的 import 分支；
   五条来源分支（legacy/finalized/import/directory/generation）至此全部由已移植叶函数服务，
   `CHARACTER_PROPOSAL_PROOF_PORT_PENDING` 占位码删除。新增 5 项 Rust 单测。
4. **B102 `76bae7eb` — review-revision 恢复投影改用 owner 真实实现**：`assert_review_sources_current` /
   `review_revision_sources_match` / `review_revision_candidate_snapshot` 三处保守占位换成真实接线
   （源漂移与候选投影此前恒通过/恒空）。新增 1 项 owner 单测。
5. **B103 `0278b233` — 角色提案源现状探针真实化**：`assertSources` 深比较占位改由
   `assert_binding_current` 承担（`GENERATION_SOURCE_CHANGED` / `GENERATION_MODEL_MISSING`）。新增 1 项 owner 单测。
6. **B104 `2fc03caa` — 规划续作与大纲恢复源现状探针真实化**：统一由 owner `binding_is_current` 判定，
   `planning_continuation` 与 `plot_outline_recovery` 源漂移时返回/保持 `source-changed`。新增断言覆盖。
7. **B105 `2a9973d4` — 活化角色提案批准的 finalized-generation 分支**：
   改调用 `finalized_character_proof::prove_finalized_character_generation` 取 `currentHandle`
   （基线 :1261），占位码与过时头注移除。
8. **B106 `99e16fd4` — 活化 `legacy-roster:begin`**：`begin_internal` 增加 `legacy_initialization` 参数，
   仅专用入口允许 `legacyRosterKey` 选择键与 `legacy-character-roster-repair` 操作；
   `legacyRosterKey` 由源绑定展开为 `legacyRosterContext`。新增 1 项 owner 单测。
9. **B107 `413d5c28` — 活化 `legacy-roster:execute` 双阶段执行与执行面准入**：
   解除 `finalized-*` / `legacy-character-roster-repair` 的无条件执行拒绝守卫（改由
   `finalization_admission` / `legacy_admission` 控制）；移植 `initial` → `repair` 双阶段循环
   （每阶段至多 3 次尝试、长度截断续写提示、`GENERATION_LEGACY_ATTEMPT_CONFLICT` 校验、
   产物 `visibleArtifact` 匹配与正文哈希校验）。新增 1 项 owner 单测。
10. **B108 `712ae602` — 清理 agent 轮次执行的遗留 fail-closed 死路径**：删除服务层
    `GENERATION_AGENT_ROUND_UNMIGRATED` 死路径与 owner `agent_round` 包装及过时注释，
    测试改为断言真实准备段身份门禁（`GENERATION_AGENT_IDENTITY_MISMATCH`）。

**接入面**：`finalized-character` 域 5 频道全部完成；本区间只新增 1 个频道覆盖（270 → 271），
B100–B108 均为既有域的实现补齐与占位桩清理，不新增频道。频道覆盖 271/290（93.4%）；
已注册命令 272、事件 7、未迁移 19。

## 未解除边界与待确认项

1. 剩余 19 个频道全部落在**需先确认边界**的特殊域：
   `cloud-backup` 8（安全存储、网络与凭据策略）、`character-avatar` 4（图片存储与裁剪方案）、
   `dialog` 4（项目归档等文件弹窗）、`project` 3（ZIP 归档导出/恢复/旧版导入方案）。
2. 浏览器/真实 GUI/真实模型/MCP 未验收；真实 `tauri dev`/`build` 窗口首验与启动门禁
   （`SKIN_SNAPSHOT_NOT_READY` 分类）仍未开始。
3. Agent/draft 跨会话在途收据×关闭重开×多会话导航、unknown result×父根生命周期仍待处理。

## 交接给下次会话

- 分支 `master` @ `712ae602`，工作区干净，与 `origin/master` 同步（0/0 未推送差异）。
- 当前状态入口：`.pi/tmp/pipeline-state.md`（本机活动队列，不入库）。
- 下一步（前置条件）：
  1. 先向用户确认特殊域边界（安全存储/网络、图片存储、ZIP 归档、文件弹窗），再按域切片推进 19 频道；
  2. 未确认前沿线可做真实 `tauri dev` 窗口首验与启动门禁分类；
  3. 普通切片自检后可直接提交推送（2026-10-14 授权），schema/新依赖/权限/CI/ADR/归档/头像/云备份仍须先确认。
- 过时描述的当前处理：CURRENT.md 与 `docs-fork/README.md` 此前写"266/290、第 61 次快照"、
  `.pi/tmp/pipeline-state.md` 停在 B98 —— 已在当前状态统一纠正为 271/290 与第 63 次；
  冻结快照正文不改，历史以来源链接追溯。
- 验证命令与目录：均为 `tauri-app/` 与 `tauri-app/src-tauri/` 下执行；
  代码版本 `712ae602`；证据见上表 `.pi/tmp/b63-*.log`、`b99..b108-*.log`。
