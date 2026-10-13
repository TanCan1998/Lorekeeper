# Tauri 迁移进度快照（2026-10-13）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档（日期化交接快照）。
> **规则**：命名 `YYYY-MM-DD-tauri-migration-status.md`，一日一新文件，**旧文件冻结后不允许修改**（见 [`docs-fork/agents/pi-development.md`](../agents/pi-development.md) §9）。
> **模板**：[`_TEMPLATE-tauri-migration-status.md`](./_TEMPLATE-tauri-migration-status.md)。
>
> 全部历史快照见 `docs/handoffs/` 目录（按日期命名）。
>
> - channel 级盘点：[`docs-fork/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。

---

## 快照（最后更新：2026-10-13 · 第 33 次）

> **本表只填最新一次自检的实测值。**

| 项 | 值 |
|---|---|
| 仓库 / 分支 | `F:\workplace\pi\ai-novel-writer` / `master`（与 origin/master 同步） |
| 产品身份 | Lorekeeper（设定司）/ `com.tancan1998.lorekeeper` |
| 已注册命令 | 227 |
| 覆盖 invoke 频道 | 226 |
| 未迁移 invoke 频道 | 64 [agent-generation=8 cloud-backup=8 character-proposal=6 finalization-generation=5 finalized-character=5 graph-generation=5 project=5 character-avatar=4 dialog=4 editor-inline=4 kb=3 startup=3 import-generation=2 character-identity=1 db=1] |
| orphan | 无 |
| `cargo test --lib` | 993 passed / 0 failed（990 既有 + 3 新增 validate_legacy_roster_candidate 测试） |
| `cargo check --all-targets` | 0 错误 0 告警 |
| `pnpm typecheck` / `lint` | exit 0 / exit 0 |
| 定向 `vitest` | 未跑（本波无前端测试改动） |
| 已完成批次 | G4-1 / G4-2 / G4-3a / G4-3b / G4-5 / G4-6a / G4-6b-1 / G4-6b-2 / G4-6b-3 / G4-6b-4 / G4-6c-1 / G4-6c-2 / G4-7 / G4-8a / **G4-8b**（2026-10-13 新增） |
| 当前阶段 | G4-8b legacy-roster **命令层收口**（79b68517：7 频道迁移——read-source / adopt-existing / read / stage / cancel 5 真实化 + begin / execute 2 fail-closed 占位；legacy-roster=7 清零）；agent-generation 域待迁移 |
| 依赖 | 无新增（rusqlite 0.40.2 [bundled] / reqwest 0.13 / hnsw_rs / jieba 等不变） |
| GUI 冒烟 | 未做 |
| 双栈隔离 | 维持（Tauri 用 `AI_NOVEL_LOREKEEPER_HOME` / `~/.lorekeeper`；项目库 `<root>/.lore/lorekeeper.db`） |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc（`D:\Environment\rust`） |

---

## 本次更新（第 31 次：G4-7 —— with_agent_child_effect 结构接线 + db:project-core-commit-generated 频道迁移）

> 第 26 次（G4-6a）至第 32 次（G4-8a）详见 Git 历史（f4e06276 / 143679f0 / d752b414 等）与提交消息；本段为第 33 次（同日改写）。

### 1. 新增（本轮）

| 项 | 内容 |
|---|---|
| owner 方法 | main_generation_owner.rs（+352——9 方法：`character_proposal_service` 依赖构造 / `legacy_roster_recovery` 恢复视图 / `read_legacy_roster_source_cmd` / `adopt_legacy_cards_cmd` / `begin_legacy_roster_generation` / `read_legacy_roster_generation` / `execute_legacy_roster_generation` / `stage_legacy_roster_generation` / `cancel_legacy_roster_generation`） |
| 命令 | commands/generation.rs（+126——7 `#[tauri::command]`）；lib.rs（+8——`generate_handler!` 注册 7 命令） |
| 测试 | character_roster_repository.rs（+32——3 个 `validate_legacy_roster_candidate` 测试：非对象拒绝 / 不支持 schema 版本拒绝 / 合法候选条目归一化） |
| 前端 | ipc-client.ts（+8——CHANNEL_ARG_NAMES 登记 6 带参频道）；migrated-channels.ts（+7——`check:channels --emit` 再生成 226 频道） |
| Schema | 无变更 |

**实现要点**：
- **5 频道真实化**（逐行平移基线 generation-controller.ts:206-212 + main-generation-owner.ts:963-1032）：
  - `legacy-roster:read-source` → `read_legacy_roster_source(conn)`（纯仓储读，assertCurrent 门禁）
  - `legacy-roster:adopt-existing` → `adopt_legacy_cards(conn, request)`（强类型 `AdoptLegacyCardsRequest`，纯仓储写）
  - `legacy-roster:read` → 请求校验（handle 键集）→ `LegacyRosterGeneration::require` 身份校验 → `legacy_roster_recovery` 恢复视图构造（`view` / `modelId`（`sourceManifest.modelReceipt.modelId`）/ `context` / `sourceStatus`（`assert_sources_current` 无 blueprintRange 形态等价 `assertFinalizationSources`）/ `attemptCount`（`SELECT COUNT(*)`）+ 条件展开 `artifact` / `proposal`）
  - `legacy-roster:stage` → 请求校验（handle + artifact 键集）→ `LegacyRosterGeneration::stage` 事务（幂等重放 → `GENERATION_LEGACY_ARTIFACT_INVALID` 校验 → proof → assertSources（`assert_sources_current` 无 range 形态）→ `proposals.stage` → 效应写回 `usage_receipt_json`）
  - `legacy-roster:cancel` → 请求校验（handle 键集）→ `require` 身份校验 → `repository.pause(cancel=true)`（基线 `service.cancel` 等价物）→ `view_of` 运行视图
- **依赖构造（对齐基线 :880-896）**：`character_proposal_service` 方法构造 `CharacterProposalService`——prove 闭包内 `assert_current` + `prove_character_proposal`，其 `assertSources` 闭包做 `require_run` + budget 取消态检查 → `CHARACTER_PROPOSAL_SOURCE_CHANGED`（基线 :676-677 rebuildBinding 深比较为 G4-6c-1 既定 fail-closed 占位）
- **begin 幂等重放路径可用**：基线 :971-972 幂等查询（projectId + uiActionNonce + legacyRosterContext）——⚠️ JSON1 扩展（`json_extract`）在 rusqlite bundled 下可用性未验证（无使用先例），改用 Rust 端解析 `action_json` / `binding_json` 过滤（语义逐字对齐，legacy roster 运行数量少，全量扫描无性能顾虑）；命中 1 条即 `legacy_roster_recovery` 重放（只读路径，不依赖创建核心），命中 >1 条 → `GENERATION_NONCE_CONFLICT`
- **2 频道 fail-closed 占位**（迁移占位码、非基线码）：
  - `legacy-roster:begin`：请求校验 + 幂等重放路径就位；创建路径 fail-closed（`GENERATION_LEGACY_ROSTER_BEGIN_UNMIGRATED`）——依赖创建核心（B3：newMainGenerationPolicy / generation-run-service 未迁移；Tauri `owner.begin` 对 `legacy-character-roster-repair` 硬拒绝 `GENERATION_LEGACY_ADMISSION_REQUIRED`，对齐基线 IPC 边界行为）
  - `legacy-roster:execute`：请求校验 + `require` 身份校验就位；执行入口 fail-closed（`GENERATION_LEGACY_ROSTER_EXECUTE_UNMIGRATED`）——依赖 LLM 双阶段执行循环（B3：service.execute 流式分发 + 事件发射未迁移）
- **`keys_subset` 复用**：legacy_roster_generation.rs 的键集子集校验由模块私有提升为 `pub(crate)`，供 owner 方法请求校验复用（对齐基线 `Object.keys(request).some(key => !allowed.includes(key))` 失败语义）

**⚠️ 诚实化占位 / 降级项（如有，必须显式列出）**：

| 项 | 阻塞于 | 表现形式 |
|---|---|---|
| begin 创建路径 | 创建核心（B3：newMainGenerationPolicy / generation-run-service）未迁移 | 创建请求收 `GENERATION_LEGACY_ROSTER_BEGIN_UNMIGRATED`（占位码）；幂等重放路径（命中既有 run）可用；待创建核心迁移波次活化 |
| execute 执行路径 | LLM 双阶段执行循环（B3：service.execute 流式分发 + `generation:snapshot` / `generation:reasoning` 事件发射）未迁移 | 执行请求收 `GENERATION_LEGACY_ROSTER_EXECUTE_UNMIGRATED`（占位码）；身份校验就位；待 LLM 执行循环迁移波次活化 |
| LegacyRosterGeneration 服务类测试 | 需要 `CharacterProposalService` + run 状态 fixture 工程（内存库 + generation_runs / generation_roots 记录 + legacyRosterContext sourceManifest） | 本波仅补 `validate_legacy_roster_candidate` 3 测试（纯函数）；服务类（require / stage / recovery 构造）测试待后续波次 |
| assertSources rebuildBinding 深比较 | `currentModelReceipt → loadModel` 依赖未迁移（G4-6c-1 既定 fail-closed 占位） | `sourceStatus` 在源已变更时可能误报 `current`（保守占位）；待 generation-source-binding 迁移波次补齐 |
| begin 幂等查询 JSON1 | rusqlite bundled 下 `json_extract` 可用性未验证（无使用先例） | 改用 Rust 端解析 `action_json` / `binding_json` 过滤（语义对齐，全量扫描）；性能无顾虑（legacy roster 运行数量少） |

### 2. 前端登记

- `src/services/ipc-client.ts`：CHANNEL_ARG_NAMES 追加 6 带参频道（legacy-roster:adopt-existing / begin / read / execute / stage / cancel，参数名 `request`；read-source 无参不登记）
- `src/shared/migrated-channels.ts`：`check:channels --emit` 再生成（+6 行，219→226 频道）
- `test/`：无变化

### 3. 行为对照（Electron ↔ Tauri，如做了才写）

- 5 真实化频道逐行平移基线（门禁顺序 / 错误码 / 事务结构 / 身份校验链）
- read 的 `legacy_roster_recovery` 构造与基线 `legacyRosterRecovery`（:953-961）字段一一对应（view / modelId / context / sourceStatus / attemptCount + 条件展开 artifact / proposal）
- stage 事务结构与基线 `LegacyRosterGeneration::stage`（legacy-roster-generation.ts:111-136）逐行对齐（幂等重放 → artifact 校验 → proof → assertSources → proposals.stage → 效应写回）
- cancel 的 `repository.pause(cancel=true)` 与基线 `service.cancel(run.rootActionId)` 语义等价（run 取消 + 视图读回）
- ⚠️ begin 创建路径 / execute 执行路径未对照（fail-closed 占位——依赖未迁移，见上表）

### 4. 提交清单

| Commit | 说明 |
|---|---|
| `79b68517` | feat(tauri): G4-8b legacy-roster 命令层——7 频道迁移（5 真实化 + 2 fail-closed 占位）（7 files +533/−2） |

---

## 交接给下次会话（**从这里接**）

### 1. 当前工作区状态

- `git status --porcelain`：仅本快照文档在途更新（docs 提交前）
- HEAD：`79b68517 feat(tauri): G4-8b legacy-roster 命令层——7 频道迁移（5 真实化 + 2 fail-closed 占位）`
- **G4-8b 收口**：legacy-roster 命令层完整落地（7 频道——5 真实化 + 2 fail-closed 占位）；频道计数 227 命令 / 226 覆盖 / 64 未迁移（legacy-roster=0 清零）；agent-generation 域（with_agent_child_effect 主体活化 + 执行核心）待迁移

### 2. 下一步（1-2-3）

1. **begin / execute 活化**：依赖 B3 创建核心（newMainGenerationPolicy / generation-run-service）与 LLM 双阶段执行循环——G4-8b 已就位请求校验 / 幂等重放 / 身份校验，创建与执行入口待依赖迁移后活化（占位码 `GENERATION_LEGACY_ROSTER_BEGIN_UNMIGRATED` / `GENERATION_LEGACY_ROSTER_EXECUTE_UNMIGRATED`）
2. **LegacyRosterGeneration 服务类测试**：构造 `CharacterProposalService` + run 状态 fixture（内存库 + generation_runs / generation_roots + legacyRosterContext sourceManifest），补 require / stage / recovery 构造测试
3. agent-generation 域迁移波次：活化 with_agent_child_effect 主体（基线 agent-generation.ts:344-376——runs registry / workflowRegistration / sessionRuns navigation / host.capture / generation_attempts usage_receipt_json 回执写入；agent-generation 服务层整体）——G4-7 结构就位，主体 fail-closed 占位待活化
4. 上游合并波次（上游未合并 109 提交：electron 基线同步 main-generation-owner +133 / generation-run-repository +178；src/ 重设计 162 文件 +9139；docs/ 镜像重同步 169 触次；G4-5 已迁移命令契约对齐 generation:list-directory-progress return → DirectoryGenerationCatalog）；其余 64 个未迁移 invoke 频道（分前缀明细见顶部表格）

### 3. 阻塞项与待授权项（**不得删除，须逐条确认后更新**）

| # | 项 | 状态 |
|---|---|---|
| B14 | 真 Windows 自动更新（tauri-plugin-updater） | 待批次 H 后段 |
| B22 | 两段式关窗 dirty 分支 | 待 GUI 实测 |
| B24 | 整屏瞬黑（暂缓） | 暂缓 |
| N1 | import-novel-resume 1/32 浏览器模式既存应用行为（2026-10-12 登记） | 单独立项评估中（修复方向：预览 effect 加 inspectionSessionRef 校验） |
| N2 | 64 个未迁移 invoke 频道迁移批次 | G4-8b 收口（2026-10-13，227 命令 / 226 覆盖 / 64 未迁移——legacy-roster=0 清零）；generation 域频道清零（generation=0） |

### 4. 红线提醒（每次接手都要过一遍）

- 🚫 `docs/` 上游镜像永不修改、永不新增；fork 专属文档只写 `docs-fork/`
- 🚫 子代理禁 `git add`/`git commit`（编排者收口）；编排者独立复验后方可提交
- 🚫 双栈隔离不可回退：Tauri 侧禁读 `AI_NOVEL_VELA_HOME`、禁回退 `~/.vela`、禁使用 `.vela/` 项目目录
- ⚠️ 提交消息首行显示宽度 ≤120 列（CJK 计 2）——CI 已强制（本机 `.git/hooks/commit-msg` 钩子亦装）；rebase 改写已推送历史须 `force-with-lease` + 用户确认
- ⚠️ 未迁移频道测试失败属既存登记不修（对应频道迁移批次后转绿）
- ⚠️ 子代理命名新规（2026-10-12）：派发前 `subagent({action:"create", ...})` 动态注册批次 agent（name=批次名-角色），收口后 `action:"delete"` 清理
- ⚠️ 推送授权（2026-10-13 用户新规）：普通 git push 无需逐波确认（复验通过 + 提交后自主推送）；force-push 改写已推送历史仍须用户确认

---

### 5. 自检记录（2026-10-13 实测）

> **生成本表快照时在本机实跑。下次更新快照表必须先重跑这些命令。**

| 命令 | 工作目录 | 实测输出 |
|---|---|---|
| `pnpm run check:channels` | `tauri-app/` | 契约 invoke 频道 290（事件频道 7）；已注册命令 227 → 覆盖 invoke 频道 226；未迁移 invoke 频道 64 [agent-generation=8 cloud-backup=8 character-proposal=6 finalization-generation=5 finalized-character=5 graph-generation=5 project=5 character-avatar=4 dialog=4 editor-inline=4 kb=3 startup=3 import-generation=2 character-identity=1 db=1]；「命令名与契约频道一一对应 ✅」（exit 1 属预期——未迁移 > 0） |
| `cargo test --lib` | `tauri-app/src-tauri/` | test result: ok. 993 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 54.84s（990 既有 + 3 新增 validate_legacy_roster_candidate 测试） |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | Finished `dev` profile [unoptimized + debuginfo] target(s) in 34.38s（0 错误 0 告警） |
| `pnpm typecheck` / `pnpm run lint` | `tauri-app/` | exit 0 / exit 0 |
| `git status --porcelain` | 仓库根 | 仅 docs-fork/handoffs/2026-10-13-tauri-migration-status.md 在途（本快照） |
| `git log -1` | 仓库根 | 79b68517 feat(tauri): G4-8b legacy-roster 命令层——7 频道迁移（5 真实化 + 2 fail-closed 占位） |

---
