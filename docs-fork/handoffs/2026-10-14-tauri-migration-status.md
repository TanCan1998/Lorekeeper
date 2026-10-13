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

## 快照（最后更新：2026-10-14 · 第 36 次）

> **本表只填最新一次自检的实测值。**

| 项 | 值 |
|---|---|
| 仓库 / 分支 | `F:\workplace\pi\ai-novel-writer` / `master`（与 origin/master 同步） |
| 产品身份 | Lorekeeper（设定司）/ `com.tancan1998.lorekeeper` |
| 已注册命令 | 235（不变——本波无命令注册，agent-generation 8 频道命令属子波次 C） |
| 覆盖 invoke 频道 | 234（不变） |
| 未迁移 invoke 频道 | 56 [agent-generation=8 cloud-backup=8 finalization-generation=5 finalized-character=5 graph-generation=5 project=5 character-avatar=4 dialog=4 editor-inline=4 kb=3 startup=3 import-generation=2]（不变） |
| orphan | 无 |
| `cargo test --lib` | 1033 passed / 0 failed（1019 既有 + 14 新增：admit_workflow 链 9 + begin agent 初始化路径 4 + execute IPC 边界回归 1） |
| `cargo check --all-targets` | 0 错误 0 告警（exit 0） |
| `pnpm typecheck` / `lint` | 未跑（本波纯 Rust 改动，无 TS 改动面） |
| 定向 `vitest` | 未跑（本波无前端逻辑改动） |
| 已完成批次 | G4-1 / G4-2 / G4-3a / G4-3b / G4-5 / G4-6a / G4-6b-1~4 / G4-6c-1~2 / G4-7 / G4-8a / G4-8b / G4-9a / G4-9b / G4-10a / **G4-10b**（2026-10-14 新增） |
| 当前阶段 | G4-10b agent-generation 子波次 B 收口（owner 内部执行路径 + agentWorkflowRegistrationId 安全缺口闭合）；下一波 = **子波次 C**（域效应 `agent_domain_tool_effect` 移植 + 8 频道命令注册） |
| 依赖 | 无新增（rusqlite 0.40.2 [bundled] / reqwest 0.13 / hnsw_rs / jieba 等不变） |
| GUI 冒烟 | 未做 |
| 双栈隔离 | 维持（Tauri 用 `AI_NOVEL_LOREKEEPER_HOME` / `~/.lorekeeper`；项目库 `<root>/.lore/lorekeeper.db`） |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc（`D:\Environment\rust`） |

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
- HEAD：`2c0365a5 ✨ feat(tauri): G4-10b agent-generation 子波次 B——内部执行路径与子代准入`
- ⚠️ 上一份快照（第 35 次）中「下一波 = agent-generation 域（4 子波次 A→B→C→D）」：A（G4-10a）与 B（G4-10b）**已收口**；「安全项优先：Rust begin 的 agentWorkflowRegistrationId 键无 admitWorkflow 等价校验」**已闭合**（本波）

### 2. 下一步（1-2-3）

1. **子波次 C**：域效应 `agent_domain_tool_effect` 移植 + 8 频道 agent-generation 命令注册（新建 `commands/agent_generation.rs` + `lib.rs` 登记）——研究 `.pi/tmp/research/agent-generation-domain.md`（§2.2/§2.3）与 `.pi/tmp/research/agent-generation-wave-b.md` §0.4；`commit_domain_tool` 占位（agent_generation.rs）在本波活化
2. **子波次 D**：`with_agent_child_effect` 主体活化 + `commands/db.rs` 两处调用点 + `character_proposal.rs` approve 接线
3. **前置依赖提醒**：begin 主体 / execute 分发核心 / resume / execute_round 主体仍 fail-closed——完整 agent-round LLM 循环依赖 B3 创建核心（newMainGenerationPolicy / generation-run-service）与 LLM 双阶段执行循环迁移

### 3. 阻塞项与待授权项（**不得删除，须逐条确认后更新**）

| # | 项 | 状态 |
|---|---|---|
| B14 | 真 Windows 自动更新（tauri-plugin-updater） | 待批次 H 后段 |
| B22 | 两段式关窗 dirty 分支 | 待 GUI 实测 |
| B24 | 整屏瞬黑（暂缓） | 暂缓 |
| N1 | import-novel-resume 1/32 浏览器模式既存应用行为（2026-10-12 登记） | 单独立项评估中 |
| N2 | 未迁移 invoke 频道迁移批次 | G4-10b 收口（2026-10-14，235 命令 / 234 覆盖 / 56 未迁移——计数不变，命令注册属子波次 C）；最大未迁移域 = agent-generation（8 频道） |
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
| `pnpm run check:channels` | `tauri-app/` | 未跑（本波无命令注册——计数不变 235/234/56，引用第 35 次实测 2026-10-14） |
| `cargo test --lib` | `tauri-app/src-tauri/` | test result: ok. 1033 passed; 0 failed; 0 ignored; finished in 51.05s（exit 0；编排者独立复验，HEAD 2c0365a5） |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | Finished `dev` profile [unoptimized + debuginfo] target(s) in 18.22s（exit 0；编排者独立复验） |
| `git status --porcelain` | 仓库根 | 空 |
| `git log -1` | 仓库根 | `2c0365a5 ✨ feat(tauri): G4-10b agent-generation 子波次 B——内部执行路径与子代准入` |
