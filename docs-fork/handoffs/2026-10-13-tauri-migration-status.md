# Tauri 迁移进度快照（2026-10-13）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档（日期化交接快照）。
> **规则**：命名 `YYYY-MM-DD-tauri-migration-status.md`，一日一新文件，**旧文件冻结后不允许修改**（见 [`docs-fork/agents/pi-development.md`](../agents/pi-development.md) §9）。
> **模板**：[`_TEMPLATE-tauri-migration-status.md`](./_TEMPLATE-tauri-migration-status.md)。
>
> 全部历史快照见 `docs/handoffs/` 目录（按日期命名）。
>
> - channel 级盘点：[`docs-fork/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。

---

## 快照（最后更新：2026-10-13 · 第 34 次）

> **本表只填最新一次自检的实测值。**

| 项 | 值 |
|---|---|
| 仓库 / 分支 | `F:\workplace\pi\ai-novel-writer` / `master`（与 origin/master 同步） |
| 产品身份 | Lorekeeper（设定司）/ `com.tancan1998.lorekeeper` |
| 已注册命令 | 234 |
| 覆盖 invoke 频道 | 233 |
| 未迁移 invoke 频道 | 57 [agent-generation=8 cloud-backup=8 finalization-generation=5 finalized-character=5 graph-generation=5 project=5 character-avatar=4 dialog=4 editor-inline=4 kb=3 startup=3 import-generation=2 db=1] |
| orphan | 无 |
| `cargo test --lib` | 997 passed / 0 failed（993 既有 + 4 新增 owner 方法测试） |
| `cargo check --all-targets` | 0 错误 0 告警 |
| `pnpm typecheck` / `lint` | exit 0 / exit 0 |
| 定向 `vitest` | blueprint-character-sync.test.ts 6 passed（基线既存测试转绿——本波修复 stage 参数登记） |
| 已完成批次 | G4-1 / G4-2 / G4-3a / G4-3b / G4-5 / G4-6a / G4-6b-1 / G4-6b-2 / G4-6b-3 / G4-6b-4 / G4-6c-1 / G4-6c-2 / G4-7 / G4-8a / G4-8b / **G4-9a**（2026-10-13 新增） |
| 当前阶段 | G4-9a character-proposal **命令层收口**（b014a409：7 频道迁移——stage / read / approve / cancel / read-pending-finalized 5 真实化 + list-pending-finalized / character-identity:read 2 无参真实化；character-proposal=6、character-identity=1 清零）；agent-generation 域待迁移 |
| 依赖 | 无新增（rusqlite 0.40.2 [bundled] / reqwest 0.13 / hnsw_rs / jieba 等不变） |
| GUI 冒烟 | 未做 |
| 双栈隔离 | 维持（Tauri 用 `AI_NOVEL_LOREKEEPER_HOME` / `~/.lorekeeper`；项目库 `<root>/.lore/lorekeeper.db`） |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc（`D:\Environment\rust`） |

---

## 本次更新（第 34 次：G4-9a —— character-proposal 域 7 频道命令层迁移）

> 第 26 次（G4-6a）至第 33 次（G4-8b）详见 Git 历史（f4e06276 / 143679f0 / d752b414 / 79b68517 / b014a409 等）与提交消息；本段为第 34 次（同日改写）。

### 1. 新增（本轮）

| 项 | 内容 |
|---|---|
| 命令 | commands/character_proposal.rs（新建 +182——7 `#[tauri::command]`：stage / read / list-pending-finalized / read-pending-finalized / approve / cancel + character-identity:read） |
| owner 方法 | main_generation_owner.rs（+210——7 方法：`character_proposal_service` 依赖构造 / `stage_character_proposal` / `read_character_proposal` / `list_pending_finalized_character_proposals` / `read_pending_finalized_character_proposal` / `approve_character_proposal` / `cancel_character_proposal` / `character_identity_snapshot` + `character_proposal_result` 前缀感知错误信封辅助） |
| 测试 | main_generation_owner.rs（+4 单元测试：read / approve / cancel / identity_snapshot owner 方法） |
| 前端 | ipc-client.ts（CHANNEL_ARG_NAMES 登记 5 带参频道——单对象参数频道统一登记 `request`）；migrated-channels.ts（`check:channels --emit` 再生成 +7 频道） |
| Schema | 无变更 |

**实现要点**：
- **7 频道全部真实化**（服务层 character_proposal.rs 2554 行 G4-8a 已就位，本波纯命令接线）：
  - `character-proposal:stage` → admission 门禁（`request.source.kind === 'legacy-roster-generation'` → `GENERATION_LEGACY_ADMISSION_REQUIRED`）→ `owner.stage_character_proposal(conn, request.source)`
  - `character-proposal:read` → `owner.read_character_proposal(conn, request.proposalBatchId)`
  - `character-proposal:list-pending-finalized` → `owner.list_pending_finalized_character_proposals(conn)`（无参）
  - `character-proposal:read-pending-finalized` → `owner.read_pending_finalized_character_proposal(conn, request.proposalBatchId)`
  - `character-proposal:approve` → `owner.approve_character_proposal(conn, request)`（幂等重放 → handle 四分支解析 → `with_agent_child_effect` fail-closed 占位）
  - `character-proposal:cancel` → `owner.cancel_character_proposal(conn, request)`
  - `character-identity:read` → `owner.character_identity_snapshot(conn)`（无参）
- **单对象参数频道统一登记 `request`**（本波关键约定）：位置参数 `{source}` / `{proposalBatchId}` / `ApproveCharacterProposalRequest` 整体映射为命名参数 `request`（Rust 命令参数名 `request: Value`，内部解构 `request["source"]` / `request["proposalBatchId"]`）——对齐基线 handler 接收整个 request 对象的语义（`blueprint-character-sync.test.ts` 断言 `named = {request: {source}, projectSession}`）。**子代理首派误用位置参数键名登记（stage:['source'] / read:['proposalBatchId']），致基线既存测试 3 failed——编排者复验发现并修复为统一 ['request'] 登记，测试转绿**
- **门禁链对齐基线 generation-controller.ts:86-190**：`authorized_owner` 会话再认证（命令层）→ owner 方法 `assert_current` 二级门禁 → 服务层
- **错误信封前缀感知**（G4-7 `blueprint_commit_range_result` 先例）：`character_proposal_result`——服务层裸码经 `map_generation_error`（`CHARACTER_` 前缀在透传正则内）+ `mutating_error` 补 `Error: ` 前缀；已带前缀的 MUTATING 信封（prove 闭包 / `assert_current` / `with_agent_child_effect`）原样透传，避免双重前缀
- **approve 平移基线 main-generation-owner.ts:1255-1266**：幂等重放（已批准 → 直接重放决策校验）→ handle 四分支解析（finalized / legacy-roster / generation / directory——directory 分支经 `list_directory_progress` 查找 `operationId` 匹配项的 `sourceHandle`）→ `with_agent_child_effect`（fail-closed 占位 `GENERATION_AGENT_CHILD_EFFECT_UNMIGRATED`，agent-generation 域迁移波次活化）；import 来源（无 handle）直接执行

**⚠️ 诚实化占位 / 降级项（如有，必须显式列出）**：

| 项 | 阻塞于 | 表现形式 |
|---|---|---|
| approve 外层事务 | Rust 服务层自包 `unchecked_transaction` 无 savepoint 适配（基线 `db.transaction().immediate()` 包裹 read+approve，better-sqlite3 嵌套为 savepoint） | read 的 envelope 刷新（幂等收敛操作）提前提交；approve 失败仅影响写入时序、不影响正确性（方法头注文档化） |
| finalized-generation 来源 | `proveFinalizedCharacterGeneration` 未迁移 | finalized-generation 提案在 read 步 fail-closed（`CHARACTER_PROPOSAL_SOURCE_CHANGED`——read_envelope 按基线 catch 语义折叠）；owner 方法的 `CHARACTER_PROPOSAL_PROOF_PORT_PENDING` 分支为结构占位，待证明函数移植后活化 |
| with_agent_child_effect 主体 | agent-generation 状态机未迁移 | 带 handle 的 approve 路径 fail-closed（`GENERATION_AGENT_CHILD_EFFECT_UNMIGRATED`）；import 来源（无 handle）与已批准幂等重放路径不受影响 |

### 2. 前端登记

- `src/services/ipc-client.ts`：CHANNEL_ARG_NAMES 追加 5 带参频道（stage / read / read-pending-finalized / approve / cancel，参数名统一 `request`；list-pending-finalized / character-identity:read 无参不登记）
- `src/shared/migrated-channels.ts`：`check:channels --emit` 再生成（+7 行，226→233 频道）
- `test/`：无变化

### 3. 行为对照（Electron ↔ Tauri，如做了才写）

- 7 频道逐行平移基线（门禁顺序 / 错误码 / 身份校验链）
- stage 的 admission 门禁在命令层承载（对齐基线 register 顺序：门禁先于 owner 方法）
- approve 的 handle 四分支解析与基线 :1255-1266 逐行对齐（幂等重放 → 四分支 → withChildEffect）
- 错误信封前缀感知与基线 catch 映射（generation-controller.ts:160-162 `mapGenerationError`）等价

### 4. 提交清单

| Commit | 说明 |
|---|---|
| `b014a409` | feat(tauri): G4-9a character-proposal 域 7 频道命令层迁移（6 files +499/−2） |
---

## 交接给下次会话（**从这里接**）

### 1. 当前工作区状态

- `git status --porcelain`：仅本快照文档在途更新（docs 提交前）
- HEAD：`b014a409 ✨ feat(tauri): G4-9a character-proposal 域 7 频道命令层迁移`
- **G4-9a 收口**：character-proposal 命令层完整落地（7 频道全部真实化——服务层 G4-8a 已就位，纯命令接线）；频道计数 234 命令 / 233 覆盖 / 57 未迁移（character-proposal=6、character-identity=1 清零）；下一波 = **db:review-cycle-get**（scout 已研究就绪）；agent-generation 域（with_agent_child_effect 主体活化 + 执行核心）待迁移

### 2. 下一步（1-2-3）

1. **db:review-cycle-get**（下一波，scout 已研究就绪）：read-only 单频道——review_cycle_repository.rs 3319 行 + review_cycles/review_findings/review_cycle_merges 三表已建 + commands/db.rs 审稿子域模式成熟，仅缺 get_by_review_id 读函数 + db_review_cycle_get 命令接线，无 schema 变更；写面 commands/db.rs + review_cycle_repository.rs 追加 + 公共装配（lib.rs/ipc-client.ts/migrated-channels.ts）
2. **begin / execute 活化**：依赖 B3 创建核心（newMainGenerationPolicy / generation-run-service）与 LLM 双阶段执行循环——G4-8b 已就位请求校验 / 幂等重放 / 身份校验，创建与执行入口待依赖迁移后活化（占位码 `GENERATION_LEGACY_ROSTER_BEGIN_UNMIGRATED` / `GENERATION_LEGACY_ROSTER_EXECUTE_UNMIGRATED`）
3. **LegacyRosterGeneration 服务类测试**：构造 `CharacterProposalService` + run 状态 fixture（内存库 + generation_runs / generation_roots + legacyRosterContext sourceManifest），补 require / stage / recovery 构造测试
4. agent-generation 域迁移波次：活化 with_agent_child_effect 主体（基线 agent-generation.ts:344-376——runs registry / workflowRegistration / sessionRuns navigation / host.capture / generation_attempts usage_receipt_json 回执写入；agent-generation 服务层整体）——G4-7 结构就位，主体 fail-closed 占位待活化
5. 上游合并波次（上游未合并 109 提交：electron 基线同步 main-generation-owner +133 / generation-run-repository +178；src/ 重设计 162 文件 +9139；docs/ 镜像重同步 169 触次；G4-5 已迁移命令契约对齐 generation:list-directory-progress return → DirectoryGenerationCatalog）；其余 57 个未迁移 invoke 频道（分前缀明细见顶部表格）

### 3. 阻塞项与待授权项（**不得删除，须逐条确认后更新**）

| # | 项 | 状态 |
|---|---|---|
| B14 | 真 Windows 自动更新（tauri-plugin-updater） | 待批次 H 后段 |
| B22 | 两段式关窗 dirty 分支 | 待 GUI 实测 |
| B24 | 整屏瞬黑（暂缓） | 暂缓 |
| N1 | import-novel-resume 1/32 浏览器模式既存应用行为（2026-10-12 登记） | 单独立项评估中（修复方向：预览 effect 加 inspectionSessionRef 校验） |
| N2 | 57 个未迁移 invoke 频道迁移批次 | G4-9a 收口（2026-10-13，234 命令 / 233 覆盖 / 57 未迁移——character-proposal=6、character-identity=1 清零）；generation 域频道清零（generation=0） |

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
| `pnpm run check:channels` | `tauri-app/` | 契约 invoke 频道 290（事件频道 7）；已注册命令 234 → 覆盖 invoke 频道 233；未迁移 invoke 频道 57 [agent-generation=8 cloud-backup=8 finalization-generation=5 finalized-character=5 graph-generation=5 project=5 character-avatar=4 dialog=4 editor-inline=4 kb=3 startup=3 import-generation=2 db=1]；「命令名与契约频道一一对应 ✅」（exit 1 属预期——未迁移 > 0） |
| `cargo test --lib` | `tauri-app/src-tauri/` | test result: ok. 997 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 56.31s（993 既有 + 4 新增 owner 方法测试） |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | Finished `dev` profile [unoptimized + debuginfo] target(s) in 11.62s（0 错误 0 告警） |
| `pnpm typecheck` / `pnpm run lint` | `tauri-app/` | exit 0 / exit 0 |
| `git status --porcelain` | 仓库根 | 仅 docs-fork/handoffs/2026-10-13-tauri-migration-status.md 在途（本快照） |
| `git log -1` | 仓库根 | b014a409 ✨ feat(tauri): G4-9a character-proposal 域 7 频道命令层迁移（6 files +499/−2） |

---
