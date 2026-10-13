# Tauri 迁移进度快照（2026-10-14）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档（日期化交接快照）。
> **规则**：命名 `YYYY-MM-DD-tauri-migration-status.md`，一日一新文件，**旧文件冻结后不允许修改**
> （见 [`docs-fork/agents/pi-development.md`](../agents/pi-development.md) §9）。
> **模板**：[`_TEMPLATE-tauri-migration-status.md`](./_TEMPLATE-tauri-migration-status.md)。
>
> 全部历史快照见 `docs/handoffs/` 目录（按日期命名）。
>
> - channel 级盘点：[`docs-fork/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。

---

## 快照（最后更新：2026-10-14 · 第 35 次）

> **本表只填最新一次自检的实测值。**

| 项 | 值 |
|---|---|
| 仓库 / 分支 | `F:\workplace\pi\ai-novel-writer` / `master`（与 origin/master 同步） |
| 产品身份 | Lorekeeper（设定司）/ `com.tancan1998.lorekeeper` |
| 已注册命令 | 235 |
| 覆盖 invoke 频道 | 234 |
| 未迁移 invoke 频道 | 56 [agent-generation=8 cloud-backup=8 finalization-generation=5 finalized-character=5 graph-generation=5 project=5 character-avatar=4 dialog=4 editor-inline=4 kb=3 startup=3 import-generation=2] |
| orphan | 无 |
| `cargo test --lib` | 998 passed / 0 failed（997 既有 + 1 新增 review_cycle_get_channels_test） |
| `cargo check --all-targets` | 0 错误 0 告警 |
| `pnpm typecheck` / `lint` | exit 0 / exit 0 |
| 定向 `vitest` | 未跑（本波无前端逻辑改动——仅 CHANNEL_ARG_NAMES 登记 1 行 + migrated-channels 再生 1 行） |
| 已完成批次 | G4-1 / G4-2 / G4-3a / G4-3b / G4-5 / G4-6a / G4-6b-1~4 / G4-6c-1~2 / G4-7 / G4-8a / G4-8b / G4-9a / **G4-9b**（2026-10-14 新增） |
| 当前阶段 | G4-9b db:review-cycle-get 命令层收口（**db 域清零**）；下一波 = **agent-generation 域**（研究就绪，4 子波次 A→B→C→D） |
| 依赖 | 无新增（rusqlite 0.40.2 [bundled] / reqwest 0.13 / hnsw_rs / jieba 等不变） |
| GUI 冒烟 | 未做 |
| 双栈隔离 | 维持（Tauri 用 `AI_NOVEL_LOREKEEPER_HOME` / `~/.lorekeeper`；项目库 `<root>/.lore/lorekeeper.db`） |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc（`D:\Environment\rust`） |

---

## 本次更新（第 35 次：G4-9b —— db:review-cycle-get 频道迁移，db 域清零）

### 1. 新增（本轮）

| 项 | 内容 |
|---|---|
| 仓储 | `review_cycle_repository.rs`（+267——`ReviewCycleProjection` / `ReviewCycleFindingProjection` struct + `RevisionStatus` 枚举 + `get_by_review_id`） |
| 命令 | `commands/db.rs`（+86——`review_cycle_get_inner` + `db_review_cycle_get`，模板 `review_get_full_inner` :1868 / `db_review_get_full` :1936） |
| 测试 | `review_cycle_get_channels_test`（读门禁 / 无 cycle→None / 无效 reviewId(0)→`REVIEW_CYCLE_INPUT_INVALID`） |
| 前端 | `ipc-client.ts`（CHANNEL_ARG_NAMES 登记 `'db:review-cycle-get': ['reviewId', 'expectedProjectPath']`，对齐 `db:review-get-full` 形式）；`migrated-channels.ts`（`check:channels:emit` 再生 +1 行） |
| Schema | 无变更 |

**实现要点**：
- `get_by_review_id` **逐字平移基线 getByReviewId**（electron/repositories/review-cycle-repository.ts:504-596）查询链：`integer(reviewId,1)` 门禁 → `review_cycles` 查询（无行→`Ok(None)`）→ `verify_m03_review_cycle` → `reviews⋈contents`（`source_content` 为 null→conflict）→ `parse_saved_report`（非法→`REVIEW_CYCLE_INPUT_INVALID`）→ findings sourceRow（`LIMIT 1`）+ persisted（`ORDER BY finding_id`）→ 仅 persisted 无 sourceRow→conflict → 复用既有 `finding()` 构建器计算 itemIndex 首现下标映射 → 尾部 merge-committed + `is_current_merged_cycle` 时追加 `plan_recheck().disposition`
- serde 条件展开对齐基线：`mergedHash` / `recheckDisposition` / `targetId` 用 `skip_serializing_if`——缺席时键不出现（基线 `...(unshown.length ? { sourceContent } : {})` 同构）
- **复用既有构件**（不重造）：`finding()` 构建器 / `verify_m03_review_cycle` / `is_current_merged_cycle` / `plan_recheck` 均 G4-8 前波已就位
- 错误码 `REVIEW_CYCLE_INPUT_INVALID` 对齐基线 `invalid()` 抛错语义；`RevisionStatus` 枚举 kebab-case，CHECK 约束保证解析安全

**⚠️ 诚实化占位 / 降级项**：无——本波 1 频道全真实化，无 fail-closed 占位。

### 2. 前端登记

- `src/services/ipc-client.ts`：+1 行（位置参数频道登记，对齐 `db:review-get-full`）
- `src/shared/migrated-channels.ts`：`check:channels:emit` 再生（+1 行，233→234 频道）
- `test/`：无变化

### 3. 行为对照（Electron ↔ Tauri）

- 查询链 / 错误码 / 序列化字段名 / 条件展开逐行对齐基线；对照覆盖到仓储层完整查询链（reread 基线 :504-596 逐字平移），命令层门禁（assertRequiredExpectedProjectPath + 无项目库→null）对齐基线 handler。

### 4. 提交清单

| Commit | 说明 |
|---|---|
| `9278944c` | ✨ feat(tauri): G4-9b db:review-cycle-get 频道迁移（db 域清零）（5 files +355/−1） |

---

## 交接给下次会话（**从这里接**）

### 1. 当前工作区状态

- `git status --porcelain`：空（快照文档提交前）
- HEAD：`9278944c ✨ feat(tauri): G4-9b db:review-cycle-get 频道迁移（db 域清零）`
- ⚠️ 上一份快照中已过期的描述：「下一波 = db:review-cycle-get（scout 已研究就绪）」**已收口**；「57 未迁移」已更新为 56（db 域清零）

### 2. 下一步（1-2-3）

1. **agent-generation 域迁移**（研究就绪：`.pi/tmp/research/agent-generation-domain.md`，228 行）——建议 4 子波次 A→B→C→D 依赖序：A 核心状态机（新建 `services/agent_generation.rs`）→ B owner 内部执行路径（`main_generation_owner.rs` begin 内部 agent-round 准入 + admitWorkflow + recordWorkflowChild + execute internal 入口）→ C 域效应 + 8 频道命令注册（新建 `agent_domain_tool_effect.rs` + `commands/agent_generation.rs` + `lib.rs`）→ D 占位活化（`with_agent_child_effect` 主体 + `commands/db.rs` 两处调用点 + `character_proposal.rs` approve）。**安全项优先**：Rust `begin` 的 `agentWorkflowRegistrationId` 键无 admitWorkflow 等价校验（基线有 `GENERATION_AGENT_CHILD_NOT_AUTHORIZED` 门禁——子波次 B 随 admit_workflow 闭合，或先行 fail-closed 该键）
2. **待验证项**（实现波次开工前）：rusqlite 捆绑 SQLite 的 JSON1 / `json_extract` 可用性（`generation_run_repository` 现有 SQL 是否已用可作先例核查点）；`plot-outline-contract` 等价物（`assertPlanningActionRange` / `parsePlanningTargetUnits`）是否已有 Rust 移植
3. **begin / execute 活化**（legacy-roster 域）：依赖 B3 创建核心（newMainGenerationPolicy / generation-run-service）与 LLM 双阶段执行循环——G4-8b 已就位请求校验 / 幂等重放 / 身份校验，创建与执行入口待依赖迁移后活化
4. **上游合并波次**（上游未合并 109 提交：electron 基线同步 main-generation-owner +133 / generation-run-repository +178；src/ 重设计 162 文件 +9139；docs/ 镜像重同步 169 触次；G4-5 已迁移命令契约对齐 `generation:list-directory-progress` return → `DirectoryGenerationCatalog`）

### 3. 阻塞项与待授权项（**不得删除，须逐条确认后更新**）

| # | 项 | 状态 |
|---|---|---|
| B14 | 真 Windows 自动更新（tauri-plugin-updater） | 待批次 H 后段 |
| B22 | 两段式关窗 dirty 分支 | 待 GUI 实测 |
| B24 | 整屏瞬黑（暂缓） | 暂缓 |
| N1 | import-novel-resume 1/32 浏览器模式既存应用行为（2026-10-12 登记） | 单独立项评估中（修复方向：预览 effect 加 inspectionSessionRef 校验） |
| N2 | 未迁移 invoke 频道迁移批次 | G4-9b 收口（2026-10-14，235 命令 / 234 覆盖 / 56 未迁移——db 域清零）；最大未迁移域 = agent-generation（8 频道，研究就绪） |

### 4. 红线提醒（每次接手都要过一遍）

- 🚫 `docs/` 是上游镜像：永不修改、永不新增；fork 专属文档只写 `docs-fork/`
- 🚫 禁止新增依赖（npm / Rust crate）；禁止改 `db/schema.rs` 既有表定义（Ask first）
- 🚫 禁止提交 API Key / Token / 用户数据 / 小说正文（Gitleaks CI 防线）
- ⚠️ 双栈隔离不可回退：Tauri 侧禁读 `AI_NOVEL_VELA_HOME` / `~/.vela` / `.vela/`
- ⚠️ 定稿不可逆：定稿绑定草稿身份 + 收据 + 章节号 + 正文哈希
- ⚠️ 一次只迁一个 controller；子代理自述不算证据，编排者独立复验

---

### 5. 自检记录（2026-10-14 实测）

> **生成本表快照时在本机实跑。下次更新快照表必须先重跑这些命令。**

| 命令 | 工作目录 | 实测输出 |
|---|---|---|
| `pnpm run check:channels` | `tauri-app/` | 契约 invoke 频道 290（事件频道 7）；已注册命令 235 → 覆盖 invoke 频道 234；未迁移 invoke 频道 56；命令名与契约频道一一对应 ✅ |
| `cargo test --lib` | `tauri-app/src-tauri/` | test result: ok. 998 passed; 0 failed; 0 ignored; finished in 54.44s |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | Finished `dev` profile [unoptimized + debuginfo] target(s)（0 错误 0 告警） |
| `git status --porcelain` | 仓库根 | 空 |
| `git log -1` | 仓库根 | `927894c ✨ feat(tauri): G4-9b db:review-cycle-get 频道迁移（db 域清零）` |
