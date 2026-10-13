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

## 快照（最后更新：2026-10-14 · 第 37 次）

> **本表只填最新一次自检的实测值。**

| 项 | 值 |
|---|---|
| 仓库 / 分支 | `F:\workplace\pi\ai-novel-writer` / `master`（与 origin/master 同步） |
| 产品身份 | Lorekeeper（设定司）/ `com.tancan1998.lorekeeper` |
| 已注册命令 | 243（+8：agent-generation:* 8 频道命令注册——G4-10c） |
| 覆盖 invoke 频道 | 242（+8） |
| 未迁移 invoke 频道 | 48 [cloud-backup=8 finalization-generation=5 finalized-character=5 graph-generation=5 project=5 character-avatar=4 dialog=4 editor-inline=4 kb=3 startup=3 import-generation=2]（agent-generation 域移出——8 频道命令层注册） |
| orphan | 无 |
| `cargo test --lib` | 1033 passed / 0 failed（计数不变：commit_domain_tool 占位断言改语义断言，无新增测试——见本次更新诚实化项） |
| `cargo check --all-targets` | 0 错误 0 告警（exit 0） |
| `pnpm typecheck` / `lint` | 未跑（本波纯 Rust 改动，无 TS 改动面） |
| 定向 `vitest` | 未跑（本波无前端逻辑改动） |
| 已完成批次 | G4-1 / G4-2 / G4-3a / G4-3b / G4-5 / G4-6a / G4-6b-1~4 / G4-6c-1~2 / G4-7 / G4-8a / G4-8b / G4-9a / G4-9b / G4-10a / G4-10b / **G4-10c**（2026-10-14 新增） |
| 当前阶段 | G4-10c agent-generation 子波次 C 收口（域工具效应移植 + 8 频道命令注册 + commit_domain_tool 活化）；下一波 = **子波次 D**（`with_agent_child_effect` 主体活化 + `commands/db.rs` 两处调用点 + `character_proposal.rs` approve 接线） |
| 依赖 | 无新增（rusqlite 0.40.2 [bundled] / reqwest 0.13 / hnsw_rs / jieba 等不变） |
| GUI 冒烟 | 未做 |
| 双栈隔离 | 维持（Tauri 用 `AI_NOVEL_LOREKEEPER_HOME` / `~/.lorekeeper`；项目库 `<root>/.lore/lorekeeper.db`） |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc（`D:\Environment\rust`） |

---
---

## 本次更新（第 37 次：G4-10c —— agent-generation 子波次 C，域工具效应移植与 8 频道命令注册）

### 1. 新增（本轮，6 写面 + 注册 2 文件，7 文件 +984/−10）

| 项 | 内容 |
|---|---|
| `services/agent_domain_tool_effect.rs`（新建，339 行） | 逐字移植基线 `electron/services/agent-domain-tool-effect.ts`（50 行）：`is_agent_domain_tool_current`（config→`project_core_repository::get` 哈希比对；blueprint→`blueprint_repository::get_all` 按 chapterNumber 定位比对，行缺失/收据缺 chapterNumber 按基线 `!!current` 语义返回 false）；`commit_agent_domain_tool`（name 白名单、`args.changes` 非空对象校验、字段归一化 config `narrativePov`→`narrativePOV` / blueprint `作者微操指导`·`用户指引`→`userGuidance`、`writingLanguage` 简体中文/English→zh-CN/en-US、configStrings(10)/configEnums(3)/blueprintStrings(7) 白名单 + totalChapters/wordsPerChapter 正整数 + characters 字符串数组校验、重复字段拒绝、config→`project_core_update`（写回 narrativePOV→narrativePov）、blueprint→`blueprint_upsert`（`{...before, ...changes}`，`args.chapter_number` 键名逐字保留）、返回 `{kind, toolCallId, changes, chapterNumber?, beforeHash, afterHash}`、`GENERATION_AGENT_DOMAIN_TOOL_INVALID` 逐字保留） |
| `services/agent_generation.rs` | `commit_domain_tool` 占位活化（基线 :377-404 逐字）：`.immediate()` 等价事务包装（`is_autocommit` 分流，复用 with_child_effect 先例）→ `action(ref)` 定位 → 幂等（已有 domainEffect 返回 `{...domainEffect, current: is_agent_domain_tool_current(...)}`）→ 门禁（running + root.epoch==scope.epoch + root.active + sources_current + 工具在 context 输入表且 builtin/requiresConfirmation/!isReadOnly，否则 `GENERATION_AGENT_DOMAIN_TOOL_NOT_AUTHORIZED`）→ 会话全轮次投影 capture before（缺 attempt→`GENERATION_AGENT_CHILD_EFFECT_ROUND_REQUIRED`）→ `commit_agent_domain_tool` → 全轮次写 `agentSourceTransition` + 本 attempt action 补 `domainEffect` → 返回 `{...effect, current: true}`；新增错误码常量 `GENERATION_AGENT_DOMAIN_TOOL_INVALID` / `GENERATION_AGENT_DOMAIN_TOOL_NOT_AUTHORIZED`；测试断言由 UNMIGRATED 改语义断言（畸形 ref→`GENERATION_AGENT_IDENTITY_MISMATCH`，基线 :379 定位先行） |
| `services/main_generation_owner.rs` | 新增 8 个公共 `agent_*` 委托方法（agent_begin/read/resume/round/claim_tool/finish_tool/register_workflow/commit_domain_tool），对齐 `admit_workflow` 先例（方法级构造 `AgentGeneration::new(conn, repository, project_id, lease_id, assert_current, self)` 后委托） |
| `commands/agent_generation.rs`（新建，225 行） | 8 个 `#[tauri::command]`（agent_generation_begin/read/resume/round/claim_tool/finish_tool/register_workflow/commit_domain_tool），范式照 character_proposal.rs：`authorized_gate`（MainGenerationOwnerCache 会话再认证 + 活跃项目库校验）+ `state.with_project_db` + `domain_result` 辅助（typed→Value + `mutating_error(map_generation_error(&e))` 折叠，承载基线 controller:217-230 register 门禁与错误映射正则）；read/resume 取 `request.handle`，register-workflow/commit-domain-tool 取 `request.ref`（对齐基线 handler） |
| `commands/mod.rs` / `services/mod.rs` / `lib.rs` | `pub mod agent_generation;` + `pub use agent_generation::*;`（字母序）/ `pub mod agent_domain_tool_effect;` / `generate_handler!` 注册 8 频道（批次 G4-10c 注释，对齐 G4-8b/G4-9a 形态） |
| 测试 | 无新增（1033 计数不变）：commit_domain_tool 既有占位测试改语义断言；新模块无内嵌测试（见诚实化项） |
| Schema / 依赖 | 无变更 / 无新增（状态落既有 `generation_attempts.usage_receipt_json` / `generation_runs.binding_json` 列） |

**行为对照（Electron ↔ Tauri）**：
- `commit_agent_domain_tool` 与基线 50 行逐行对照：白名单集合、枚举值域、字段名归一化、`Object.hasOwn` 重复拒绝、`Number.isSafeInteger` 正整数、`args.chapter_number` snake_case 键名（易错点，逐字保留）、config 写回时 narrativePOV→narrativePov 键名回转、blueprint `{...before, ...changes}` 合并——全部一致
- `commit_domain_tool` 九步链与基线 :377-404 对照：事务门禁（基线 `db.inTransaction` 检查按既定约定为调用方结构保证——rusqlite 0.40.2 无 `is_in_transaction()`，方法内不重复检查）、幂等短路、门禁五条件、投影捕获、transition 回写（actionId=toolCallId、before/after hash）、domainEffect 合并（state 缺失时基线 spread undefined 等价于仅含 domainEffect 的新对象）——全部一致

**⚠️ 诚实化占位 / 降级项**：
- 新模块无内嵌单元测试：`commit_agent_domain_tool` 字段白名单/枚举/映射逻辑经编排者逐行对照基线核验（逐字移植），但无 Rust 侧直接测试覆盖；`commit_domain_tool` 仅身份门禁路径有测试断言。域效应路径的端到端覆盖随 agent-round 完整循环迁移波次补齐
- `is_agent_domain_tool_current` 基线返回 `bool`，Rust 为 `Result<bool, String>`——为保留基线仓储读取错误原样传播语义（调用方 `?` 上抛经事务回滚折叠）；行缺失/收据缺 chapterNumber 仍按基线 `!!current` 语义返回 `false`
- 哈希口径：`beforeHash`/`afterHash` 为 typed 序列化（结构体声明序）SHA-256 hex——写入侧、读取侧、当前性判定同模块内同一序列化路径，进程内逐字自洽（基线 TS 侧同理）
- `resume` / `execute_round` 占位保持 fail-closed（`GENERATION_AGENT_RESUME_UNMIGRATED` / `GENERATION_AGENT_ROUND_UNMIGRATED`，迁移占位码、非基线码，匹配透传正则原样透传）——依赖 B3 创建核心与 LLM 双阶段执行循环迁移（后续波次）
- 方法级构造 `AgentGeneration`（恢复扫描按方法执行，幂等）——同 G4-10b 已记偏差

### 2. 前端登记

- 无（前端 `ipc-client.ts` 经 `toCommandName` 自动映射频道→命令名，共享契约 `src/shared/agent-generation.ts` 不变；无 TS 改动面）

### 3. 提交清单

| Commit | 说明 |
|---|---|
| `28daa346` | ✨ feat(tauri): G4-10c agent-generation 子波次 C——域工具效应移植与 8 频道命令注册（7 files +984/−10） |
| `2c0365a5` | ✨ feat(tauri): G4-10b agent-generation 子波次 B——内部执行路径与子代准入（前置依赖） |

---

## 本次更新（第 36 次：G4-10b —— agent-generation 子波次 B，owner 内部执行路径与子代准入闭合）

### 1. 新增（本轮，3 写面 +1424/−252）

| 项 | 内容 |
|---|---|
| `services/main_generation_owner.rs` | `begin` 加 `agent_initialization` 第三参（条件键集放行 `agentInput`/`agentSession`、agent-round 操作条件化拒绝——IPC 边界拒绝逐字不变）；基线 :367 位置插入 `admit_workflow` 准入调用（**agentWorkflowRegistrationId 安全缺口闭合**）；基线 :371 位置 existing 幂等早退（子代禁令后、batchIntent 前）；新增私有 `admit_workflow` 辅助（方法级构造 `AgentGeneration`，对齐基线 :897 owner 闭包内 `new`）；`execute` 主体提取为私有 `execute_inner(conn, request, agent_execution)`（公有 IPC 签名逐字不变）；新增私有 `execute_agent_round`（基线 :899 host.execute 注入等价物，分发占位处预置 agentToolNames 提取）；新增 `impl AgentHost for MainGenerationOwner`（begin/read/resume/execute 接 owner 内部入口；current/capture 保守占位） |
| `services/agent_generation.rs` | `admit_workflow` 逐字移植基线 agent-generation.ts:305-342（九步校验链：registration 定位 / expectedOperations 表 / 五重门禁 / existing 幂等 / planning 区间+targetUnits / 根纪元+cancelled / 源一致性）；新增 `AdmittedWorkflow` 结构体；新增 `GENERATION_AGENT_REGISTRATION_REQUIRED` / `GENERATION_AGENT_CHILD_AMBIGUOUS` 错误码；resume/execute_round 占位注释改指「分发核心波次」（占位码不变）；admit_workflow 测试 9 个 |
| `commands/generation.rs` | :1981 IPC 调用点传第四参 false（1 行） |
| 测试 | 共 14 个（cargo test --lib 1019→1033）：admit_workflow 链 9（passthrough / registration missing×2 / 五重门禁 / planning 区间+单位+JSON 失败 / tool stale×2 / source changed / admits new child / existing 幂等 / ambiguous）+ begin agent 初始化路径 4 + execute IPC 边界回归 1 |
| Schema / 依赖 | 无变更 / 无新增 |

**实现要点**：
- **安全缺口闭合**：Rust `begin` 原对带 `agentWorkflowRegistrationId` 的 selection 无任何校验（BEGIN_ALLOWED_KEYS 含该键但无 admitWorkflow 等价物）——现经完整校验链：`GENERATION_AGENT_REGISTRATION_MISSING`（定位失败/重复）→ `GENERATION_AGENT_CHILD_NOT_AUTHORIZED`（五重门禁：action 态 / modelId / parentRootActionId / operation 表 / generate_draft 章节号 / 批量键）→ `GENERATION_AGENT_CHILD_AMBIGUOUS`（幂等定位多命中）→ planning 区间与 targetUnits 逐字校验 → `GENERATION_AGENT_TOOL_STALE`（根纪元/cancelled）→ `GENERATION_AGENT_SOURCE_CHANGED`（源指纹）
- `execute` 重构为 `execute_inner` 私有分流：基线 `agentExecution` 语义（准入豁免 + agentToolNames 注入）保留在私有层，IPC 边界 `execute` 签名与行为逐字不变（回归测试保护）
- `impl AgentHost for MainGenerationOwner`：基线 :897-902 闭包注入的 Rust 等价物——`begin`→`begin(sel, false, true)`、`read`→`require_run(false)+view_of`、`resume`→`resume`、`execute`→`execute_agent_round`

**⚠️ 诚实化占位 / 降级项**（均已记 doc 注释）：
- AgentHost `current`/`capture` 保守占位：基线经 `deps.rebuildBinding(run.binding, currentModelReceipt(...))` 重建后比对，rebuildBinding/currentModelReceipt 未迁移（先例 review_revision.rs:109-110、generation.rs:841）——`SOURCE_CHANGED` 检测随 rebuildBinding 迁移波次补齐
- 方法级构造 `AgentGeneration`：基线 owner 创建时构造一次（恢复扫描一次），本波每次 `begin` 执行一次恢复扫描——幂等（仅修复悬空 `running` 认领），行为等价，偏差已记
- begin 主体（策略构造器/service.open）、execute 分发核心（LLM 循环）、resume/execute_round 主体保持 B3 fail-closed 占位不动

### 2. 前端登记

- 无（本波无命令注册；agent-generation 8 频道命令注册属子波次 C）

### 3. 行为对照（Electron ↔ Tauri）

- begin 准入链顺序逐字对齐基线 :366-371（agent-round 拒绝 → admitWorkflow → 子代禁令 → existing 早退）；admit_workflow 九步校验链与基线 :305-342 逐行对照（错误码、判定顺序、JSON 解析失败语义、`Number()` 数值比较等价物）；execute 的 agentExecution 两处使用（:545 准入豁免 / :617 agentToolNames）对齐基线。

### 4. 提交清单

| Commit | 说明 |
|---|---|
| `2c0365a5` | ✨ feat(tauri): G4-10b agent-generation 子波次 B——内部执行路径与子代准入（3 files +1424/−252） |
| `4e9e8ebd` | 📚 docs(fork): 编排规范与派发模板——交接文档体系整理（本日早前提交） |
| `206c97a6` | ✨ feat(tauri): G4-10a agent-generation 域子波次 A——核心状态机服务层（前置依赖） |

---

## 交接给下次会话（**从这里接**）

### 1. 当前工作区状态

- `git status --porcelain`：空（快照提交前）
- HEAD：`28daa346 ✨ feat(tauri): G4-10c agent-generation 子波次 C——域工具效应移植与 8 频道命令注册`
- ⚠️ 上一份快照（第 35 次）中「下一波 = agent-generation 域（4 子波次 A→B→C→D）」：A（G4-10a）、B（G4-10b）与 C（G4-10c）**已收口**；「安全项优先：Rust begin 的 agentWorkflowRegistrationId 键无 admitWorkflow 等价校验」**已闭合**（G4-10b）

### 2. 下一步（1-2-3）

1. **子波次 D**：`with_agent_child_effect` 主体活化（main_generation_owner.rs 占位）+ `commands/db.rs` 两处调用点 + `character_proposal.rs` approve 接线——研究 `.pi/tmp/research/agent-generation-domain.md` §3.1/§3.4；活化后 db.rs 两处 UNMIGRATED 测试断言（:4718-4729、:4761-4780）改语义断言
2. **前置依赖提醒**：begin 主体 / execute 分发核心 / resume / execute_round 主体仍 fail-closed——完整 agent-round LLM 循环依赖 B3 创建核心（newMainGenerationPolicy / generation-run-service）与 LLM 双阶段执行循环迁移；agent-generation 域效应端到端测试覆盖（本波诚实化项）随该波次补齐
3. **后续候选**：legacy-roster begin/execute 活化、上游合并独立波次、import-novel-resume 单项修复评估（不作为已授权新增依赖/schema/CI 工作）

### 3. 阻塞项与待授权项（**不得删除，须逐条确认后更新**）

| # | 项 | 状态 |
|---|---|---|
| B14 | 真 Windows 自动更新（tauri-plugin-updater） | 待批次 H 后段 |
| B22 | 两段式关窗 dirty 分支 | 待 GUI 实测 |
| B24 | 整屏瞬黑（暂缓） | 暂缓 |
| N1 | import-novel-resume 1/32 浏览器模式既存应用行为（2026-10-12 登记） | 单独立项评估中 |
| N2 | 未迁移 invoke 频道迁移批次 | G4-10c 收口（2026-10-14，243 命令 / 242 覆盖 / 48 未迁移——agent-generation 域 8 频道命令层注册移出）；最大未迁移域 = cloud-backup（8 频道） |
| ~~R1~~ | ~~G4-10a 已提交而旧状态仍称在飞~~ | **已解除**（2026-10-14：run 3d05930b 核实 failed/终止，工作已提交 206c97a，批次 agent 已清理） |

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
| `pnpm run check:channels` | `tauri-app/` | 契约 invoke 频道 290（事件频道 7）；已注册命令 243 → 覆盖 invoke 频道 242；未迁移 invoke 频道 48；命令名与契约频道一一对应 ✅（exit 0；编排者独立复验，HEAD 28daa346 工作区） |
| `cargo test --lib` | `tauri-app/src-tauri/` | test result: ok. 1033 passed; 0 failed; 0 ignored; finished in 52.90s（exit 0；编排者独立复验，HEAD 28daa346 工作区） |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | Finished `dev` profile [unoptimized + debuginfo] target(s) in 8.83s（exit 0；编排者独立复验） |
| `git status --porcelain` | 仓库根 | 空 |
| `git log -1` | 仓库根 | `28daa346 ✨ feat(tauri): G4-10c agent-generation 子波次 C——域工具效应移植与 8 频道命令注册` |
