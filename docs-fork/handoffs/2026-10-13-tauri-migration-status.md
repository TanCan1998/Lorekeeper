# Tauri 迁移进度快照（2026-10-13）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档（日期化交接快照）。
> **规则**：命名 `YYYY-MM-DD-tauri-migration-status.md`，一日一新文件，**旧文件冻结后不允许修改**（见 [`docs-fork/agents/pi-development.md`](../agents/pi-development.md) §9）。
> **模板**：[`_TEMPLATE-tauri-migration-status.md`](./_TEMPLATE-tauri-migration-status.md)。
>
> 全部历史快照见 `docs/handoffs/` 目录（按日期命名）。
>
> - channel 级盘点：[`docs-fork/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。

---

## 快照（最后更新：2026-10-13 · 第 26 次）

> **本表只填最新一次自检的实测值。**

| 项 | 值 |
|---|---|
| 仓库 / 分支 | `F:\workplace\pi\ai-novel-writer` / `master`（与 origin/master 同步） |
| 产品身份 | Lorekeeper（设定司）/ `com.tancan1998.lorekeeper` |
| 已注册命令 | 219 |
| 覆盖 invoke 频道 | 218 |
| 未迁移 invoke 频道 | 72 [agent-generation=8 cloud-backup=8 legacy-roster=7 character-proposal=6 finalization-generation=5 finalized-character=5 graph-generation=5 project=5 character-avatar=4 dialog=4 editor-inline=4 kb=3 startup=3 db=2 import-generation=2 character-identity=1] |
| orphan | 无 |
| `cargo test --lib` | 988 passed / 0 failed（984 既有 + 4 新增平移测试） |
| `cargo check --all-targets` | 0 错误 0 告警 |
| `pnpm typecheck` / `lint` | exit 0 / exit 0（本波纯 Rust 服务层，无 TS 改动） |
| 定向 `vitest` | 未跑（本波无前端改动） |
| 已完成批次 | G4-1 / G4-2 / G4-3a / G4-3b / G4-5 / G4-6a / G4-6b-1 / G4-6b-2 / G4-6b-3 / G4-6b-4 / G4-6c-1 / **G4-6c-2**（2026-10-13 新增） |
| 当前阶段 | G4-6 owner 写面——**G4-6c 全波收口**（f4e06276：prepare-draft-context 迁移 + db.rs 三分支接线）；G4-7 with_agent_child_effect 待派 |
| 依赖 | 无新增（rusqlite 0.40.2 [bundled] / reqwest 0.13 / hnsw_rs / jieba 等不变） |
| GUI 冒烟 | 未做 |
| 双栈隔离 | 维持（Tauri 用 `AI_NOVEL_LOREKEEPER_HOME` / `~/.lorekeeper`；项目库 `<root>/.lore/lorekeeper.db`） |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc（`D:\Environment\rust`） |

---

## 本次更新（第 30 次：G4-6c 阶段 2 —— db.rs generationRunHandle 三分支接线，G4-6c 全波收口）

> 第 26 次（G4-6a）/ 第 27 次（G4-6b-1/2/3）/ 第 28 次（G4-6b-4）/ 第 29 次（G4-6c-1 prepare-draft-context 迁移）详见提交消息与 Git 历史；本段为第 30 次（同日改写，含 G4-6c-2 阶段 2 收口）。

### 1. 新增（本轮）

| 项 | 内容 |
|---|---|
| 仓储 / 服务 | main_generation_owner.rs（+411——begin/execute/bind_material_decision/resume 四方法真实实现，替换 W3 骨架；GRAPH_OPERATIONS / BEGIN_ALLOWED_KEYS 常量；GENERATION_SNAPSHOT_EVENT / GENERATION_REASONING_EVENT 频道常量；MainGenerationReasoningEventProjection 载荷；D-2 决策文档） |
| 命令 | generation_begin / generation_bind_material_decision / generation_execute / generation_resume（commands/generation.rs）+ emit_generation_snapshot / emit_generation_reasoning 广播发射器（AppHandle + Emitter） |
| Schema | 无变更 |

**实现要点**：
- 标志参数对齐：电子基线 begin/execute 的 6 个初始化/执行标志为位置参数、仅主进程内部调用方置 true；IPC 边界（generation-controller.ts:173-178/231-236）恒为 false——Tauri 命令按契约单一对象入参，校验面与基线 IPC 行为逐字一致
- 事件发射（D-2 既定决策）：命令层 AppHandle::emit 全窗口广播（等价基线 subscriber.send 每窗口定向，省去订阅登记），客户端按 projectId/epoch 过滤；调用点（execute 的 LLM 循环）随 generation-run-service 迁移波次接入，发射器以 #[allow(dead_code)] 就位
- 准入校验面完整平移：GENERATION_BEGIN_INVALID 键集白名单、五个 *_ADMISSION_REQUIRED、三个 *_CHILD_FORBIDDEN、BATCH_INTENT_MAIN_ONLY、MATERIAL_DECISION_REQUIRED/INVALID/CONFLICT、OUTPUT_CONTRACT_CHANGED 等错误码逐字对齐
- bind_material_decision 幂等重放/冲突检测分支为完整真实实现
- 前端登记：ipc-client.ts CHANNEL_ARG_NAMES 追加 4 频道；migrated-channels.ts 经 `pnpm run check:channels --emit` 再生成
- 前端登记：ipc-client.ts CHANNEL_ARG_NAMES 追加 4 频道；migrated-channels.ts 经 `pnpm run check:channels --emit` 再生成
- G4-6b-2 控制面：pause/cancel/discardCandidate 状态标记型完整实现（repository.pause/discard_candidate 仓储层已逐字移植）；restart 执行依赖型 fail-closed 占位（校验面完整：GENERATION_LEGACY_ADMISSION_REQUIRED / GENERATION_GRAPH_ADMISSION_REQUIRED / GENERATION_RESTART_NEW_NONCE_REQUIRED；核心 begin 收敛 GENERATION_REQUEST_FAILED）；pause 进行中 AbortController 中止依赖未迁移 LLM 分发循环、恒为空操作（文档化）
- G4-6b-3 草稿写面：commit-draft / retry-draft-short-outline 均执行依赖型 fail-closed 占位——commit-draft 幂等重放面逐字对齐基线完整实现（require_run → read_commit → 目标校验 GENERATION_DRAFT_TARGET_INVALID → 已提交重放[expectedCompositionHash 一致返回原回执子集；不一致 GENERATION_DRAFT_COMMIT_CONFLICT] → 新提交入口 fail-closed；核心写入路径 GenerationDraftEffects::commit 刻意未迁移[generation_draft_effects.rs 头注]）；retry-draft-short-outline 仅运行身份前缀就位（require_run execution=true），重试入口 fail-closed
- G4-6b-4 批次写面 3 命令（8355ba7d，5 files +241/−6）：
  - begin-batch —— 执行依赖型：验证面完整平移（assert_generation_batch_intent 复用 generation_draft_effects.rs 已迁移 pub 函数，GENERATION_BATCH_INTENT_INVALID）→ 按基线 intent 展开语义构造 begin 选择（剔除 mode/range/targetUnits，注入 operation='batch-chapters' / 空选择集 / output='visible-text' / batchIntent，未知键随展开携带由 BEGIN_ALLOWED_KEYS 白名单拦截）→ begin(batch_initialization=true)（主进程内部初始化调用，豁免 BATCH_INTENT_MAIN_ONLY）→ readBatch(run.handle.runId)；因 G4-6b-1 begin 创建核心 fail-closed，合法请求当前收「尚未迁移」拒绝（命令层收敛 GENERATION_REQUEST_FAILED），错误码与基线逐字一致。begin 签名扩展 batch_initialization 参数（对齐基线位置参数语义），generation_begin 命令调用点传 false，IPC 边界行为不变
  - confirm-batch-finalization —— 完整实现（验证型）：assert_current → GenerationDraftEffects.read_batch（基线 :1288）→ completedChapters 匹配 chapterNumber + finalizationId + postProcessComplete=true 的已提交章校验，否则 GENERATION_BATCH_FINALIZATION_REQUIRED；通过则原样返回批次进度（基线无写入）；{ batchId, chapterNumber, finalizationId } 命名参数对象入参（GenerationConfirmBatchFinalizationRequest，camelCase + deny_unknown_fields）
  - compose-visible —— 服务层 B1-W3 读面波次已就位（真实实现：agent-round 禁令 / 取消运行检查 / 算法有效性检查 → repository.compose_visible），本波未重复添加，仅补 generation_compose_visible 命令（4 位置参数 handle + string[] + string? + algorithm?，Tauri 自动 camelCase 对齐契约）与注册；assertMutable 守卫中 reviewRevisions/imports/finalizations/legacyRosters 对应服务未迁移，维持 B1-W3 既定刻意省略（graphs.assert_mutable 虽已迁移但未接入，保持既有方法形态）
- G4-6c 阶段 1 批次（4ebaa7ab，5 files +532/−5）：generation:prepare-draft-context 迁移（**generation 域最后 1 频道，收口后 generation=0**）。性质判定：**执行依赖型 fail-closed（双重依赖未迁移）+ 验证面完整平移**：
  - 验证面完整实现：draft_preparation_binding（GENERATION_DRAFT_PREPARATION_INVALID 校验面逐字平移——request 非对象 / query 非字符串 / knowledgeSnapshot 存在时 query 不一致 / 键集超 PREPARATION_ALLOWED_KEYS 白名单 8 键；+ selection 构造逐字平移——operation='chapter-draft' / uiActionNonce='prepare-only' / selectedFinalizedDraftIds=[] / **selectedBlueprintChapterNumbers 6 章窗口（chapterNumber..+5）** / promptKeys / skillStages / authorInputs / output='visible-text' / batchId? / knowledgeSnapshot?）
  - prepare_draft_context：事务内 binding 构造 + beforeKnowledgeRead 校验（preparationMatches(before, binding, true) 不匹配 → GENERATION_DRAFT_PREPARATION_CHANGED）+ selectedDrafts 读取（SELECT d.chapter_number, d.version, c.body FROM drafts d JOIN contents c ON c.id = d.content_id WHERE d.id = ? → contentHash 为正文 SHA-256 hex）+ preparationId（project_access::random_uuid_v4，基线 randomUUID 等价物）+ preparations LRU 容量 64 超限逐出最旧（Vec<(String, PreparationEntry)> 保序等价基线 JS Map 插入序）
  - prepared_knowledge：未命中 → GENERATION_DRAFT_PREPARATION_REQUIRED；preparation_projection / preparation_matches 逐字平移（manifest 剔除 selectedDraftIds/selectedFinalizedDraftIds/materialDecision/materialDecisionHash[+knowledgeSnapshot]；fingerprint 剔除 contextSnapshotHash/dependencyHash；sources 剔除 draft:/finalized: 前缀[+kb:/knowledge-selection]；draft:/finalized: sourceRefs 逐引用等价比对）；suspend_for_project_close 挂接 preparations.clear()（基线 :1391）
  - fail-closed 占位（未迁移依赖）：load_model（模型加载命令面）/ safe_generation_model_receipt / newMainGenerationPolicy（main-generation-plan.ts 写面，G4-6b-1 既定）/ owner 侧 buildBinding 调用面——draft_preparation_binding 在模型加载构造点 fail-closed（命令层收敛 GENERATION_REQUEST_FAILED；create_model_execution_lease_receipt 已迁移[llm/lease.rs:371]但依赖 loadModel 的 LeaseModelProfile 输入）；命令层 KB 捕获 fail-closed 返回 KNOWLEDGE_BASE_NATIVE_UNAVAILABLE（KnowledgeBaseUnavailableError.code 对齐基线 register catch generation-controller.ts:159-160；KB 命令层待 F2 收口——db/vector.rs LocalVectorIndex 已落地未接命令层）
  - 与基线差异（6 项）：preparations Vec 保序 / knowledge_snapshot 全保真 Value 存储（IPC 边界 Value，Value::clone 等价 structuredClone）/ without_keys+text_hash 就地实现（generation_source_binding.rs 的为模块私有且不在 Target；textHash 形态逐字对齐 G4-6a sha256 hex）/ selection 构造前置于 fail-closed 返回（仅依赖 request/knowledgeSnapshot 无副作用，执行语义等价）/ read_preparation_drafts 缺失草稿与非数字 draftId → GENERATION_REQUEST_FAILED（基线 TypeError 经 register catch 收敛等价）/ PreparationEntry.binding 标 #[allow(dead_code)]（基线同样只写不读）
  - 阶段 2 已于本波收口（`f4e06276`，见下「G4-6c 阶段 2 批次」段）
- G4-6c 阶段 2 批次（f4e06276，4 files +401/−47，含 disk_e2e.rs 机械编译修复 +2 行）：db.rs generationRunHandle 三分支接线收口——
  - **assert_sources_current 方法**（main_generation_owner.rs +102）：验证面逐字平移基线 main-generation-owner.ts:662-678——require_run(handle, true) → 根预算 cancelled → GENERATION_ACTION_CANCELLED → blueprint_range 存在时：续接运行 remainingRange 不一致 → GENERATION_DIRECTORY_CONTINUATION_RANGE_CHANGED；selectedBlueprintChapterNumbers 缺失 / 非安全整数 / start<1 / end<start / 跨度 ≥10000 / 范围内未选中 → GENERATION_FORMAL_RANGE_NOT_SELECTED。**rebuildBinding 比对点 fail-closed 占位**（currentModelReceipt→loadModel 未迁移，G4-6c-1 既定）：方法头注文档化「实现到比对点即 fail-closed，恒 Ok(())，源漂移由命令层经 map_generation_error 收敛 GENERATION_REQUEST_FAILED」
  - **BlueprintCommitRangeRequest 字段承载**（blueprint_repository.rs +17）：补 generation_run_handle: Option<MainGenerationRunHandle> / generation_requested_range: Option<ChapterRange>——**共享契约判定**：electron/repositories/blueprint-repository.ts:58-70 的 BlueprintRangeCommitRequest 已含 generationRunHandle?/generationRequestedRange?，tauri-app/src/shared/ipc-channels.ts:844 复用该类型——契约已有，Rust 补字段即对齐契约，无需改 TS
  - **blueprint_commit_range_inner 三分支**（commands/db.rs +327）：① authorRecovery 分支——commitGenerationBlueprintAuthorEdit 未迁移（grep 核实 Rust 侧无此路径），维持现状头注说明；② 无 handle 直提保持；③ 有 handle：requested 归约（generation_requested_range ?? {start,end}）→ GENERATION_DIRECTORY_RANGE_INVALID 校验（逐条对齐 :566-569）→ 幂等分支（get_committed_range_operation 已有等价物 :1052）→ commitRange 闭包接线 assert_sources_current(handle, requested)。**withGenerationAgentChildEffect（G4-7）fail-closed**：新提交路径显式拒绝 GENERATION_AGENT_CHILD_EFFECT_UNMIGRATED（迁移占位码，非基线码——基线此处恒成功调用 G4-7 依赖），禁止静默走直提路径；recordGenerationDirectoryCommit 占位省略（Rust 回执无 generation_progress 字段，头注文档化）
  - **roster 路径闭包完整接线**：request.generationRunHandle 非 null 时 owner 门禁 + roster::commit(conn, request, Some(闭包))，闭包调 assert_sources_current(conn, handle, None)（无蓝图范围参数，对齐基线 :686）；畸形句柄由 require_run 以 GENERATION_RUN_IDENTITY_MISMATCH 拒绝（与基线对 truthy 非对象句柄行为一致）
  - **并发安全**：owner 门禁在项目库锁之前取缓存锁（锁序：缓存→库，与 generation 命令同序；在 with_project_db 内反向取锁会与 generation 命令构成 ABBA 死锁）；M8 约定：仓储钩子闭包 &dyn Fn() 无返回通道，失败以 panic 中断提交（同 roster 作者路径 assert_current 先例），事务 Drop 即回滚
  - **焦点测试**：blueprint_commit_range_generation_handle_branches_test（覆盖 RANGE_INVALID / 新提交 fail-closed / 幂等重放三路径；cargo test --lib 988→989）
  - **与基线差异**：GENERATION_AGENT_CHILD_EFFECT_UNMIGRATED 为迁移占位码 / is_safe_integer_value 与 is_safe_integer 就地重复实现（generation_run_repository 同名函数私有且按裁决只读）/ owner 门禁位置（基线事务内解析 owner；Tauri 侧提交前执行——authorized_owner 在已认证会话上为必过路径，行为等价）/ 源断言闭包「接线但休眠」（新提交路径 fail-closed 拦截在前，幂等重放仓储层早退在后——与基线语义一致，G4-7 落地后闭包即活化）
  - **disk_e2e.rs（Target 外 +2 行）**：BlueprintCommitRangeRequest 结构体字面量补两 None 字段的机械编译修复（不加则 cargo check --all-targets 无法通过）——编排者核实为纯机械修复后接受
**⚠️ 诚实化占位 / 降级项（如有，必须显式列出）**：

| 项 | 阻塞于 | 表现形式 |
|---|---|---|
| 执行核心 fail-closed 占位 | newMainGenerationPolicy 策略规划器（main-generation-plan.ts 写面）/ generation-run-service LLM 分发循环 / builtin-prompt-templates.ts 1506 行模板库 / 子生成器登记册（imports/agents/finalizations/graphs/legacyRosters）未迁移 | begin/execute/resume 命令存在、准入校验完整，但执行核心 fail-closed 占位（调用不真正执行生成）；调用点随 generation-run-service 迁移波次接入 |
| restart 核心 begin 占位 | 主生成策略规划器（main-generation-plan.ts 写面）未迁移 | restart 命令存在、校验面完整（legacy/graph 拒绝 + nonce 新鲜性），核心 begin fail-closed（命令层收敛 GENERATION_REQUEST_FAILED）；pause 进行中 AbortController 中止恒空操作（G4-6b-2） |
| commit-draft 核心写入占位 | GenerationDraftEffects::commit（DraftRepository.create 草稿行写入 + generation_attempts usage 回执写回）刻意未迁移（随 G4-6/G4-7 迁移）/ agents.withChildEffect / assertSourcesCurrent（依赖 rebuildBinding）未迁移 | commit-draft 命令存在、幂等重放面完整实现（已提交草稿重放可用），新提交入口 fail-closed（G4-6b-3） |
| retry-draft-short-outline 占位 | volatileReceipts 易失回执缓存 / repository.receipt 投影 / compareGenerationSourceBindings 绑定比对 / execute 重试分发（G4-6b-1 已 fail-closed）未迁移 | retry 命令存在、运行身份前缀就位，重试入口 fail-closed（G4-6b-3） |
| blueprint 三分支 child-effect 占位 | withGenerationAgentChildEffect（G4-7）/ recordGenerationDirectoryCommit（generation-run-service）未迁移 | blueprint-commit-range 有 handle 的请求：新提交路径显式拒绝 GENERATION_AGENT_CHILD_EFFECT_UNMIGRATED（禁止静默走直提）；幂等重放路径仓储层早退（源断言闭包接线但休眠，G4-7 落地后活化） |

### 2. 前端登记

- `src/services/ipc-client.ts`：CHANNEL_ARG_NAMES 追加 4 频道（G4-6b-1，+5 行）+ 3 频道（G4-6b-4，+4 行）+ 1 频道（G4-6c，+2 行）
- `src/shared/migrated-channels.ts`：`check:channels --emit` 再生成（G4-6b-1 +4 行 / G4-6b-4 +3 行 / G4-6c +1 行）
- `test/`：无变化

### 3. 行为对照（Electron ↔ Tauri，如做了才写）

- 准入校验面逐字对齐基线（错误码/键集白名单/冲突检测）；bind_material_decision 完整真实实现
- G4-6b-4：confirm-batch-finalization 验证面逐字对齐基线（readBatch 快照 + 三键匹配校验）；compose-visible 命令接线与 B1-W3 已迁移服务层一致
- G4-6c：prepare-draft-context 验证面逐字对齐基线（键集白名单 / 6 章窗口 selection / selectedDrafts 读取 / LRU 64 / preparationMatches 比对）；执行路径 fail-closed（GENERATION_REQUEST_FAILED / KNOWLEDGE_BASE_NATIVE_UNAVAILABLE）
- G4-6c-2：assert_sources_current 验证面逐字对齐基线（GENERATION_ACTION_CANCELLED / GENERATION_DIRECTORY_CONTINUATION_RANGE_CHANGED / GENERATION_FORMAL_RANGE_NOT_SELECTED）；blueprint 三分支范围校验逐条对齐 :566-569；roster 闭包对齐基线 :686
- ⚠️ 执行核心未对照（fail-closed 占位——依赖未迁移，见上表）

### 4. 提交清单

| Commit | 说明 |
|---|---|
| `3581a0a8` | feat(tauri): G4-6a 移植 generation 源绑定逻辑层（第 26 次，详见 Git 历史） |
| `f3d5c763` | feat(tauri): G4-6b-1 核心写面 4 命令 + 事件 2（5 files +563/−17） |
| `9bea8dc0` | feat(tauri): G4-6b-2 控制面 4 命令（5 files +289） |
| `e121d14d` | feat(tauri): G4-6b-3 草稿写面 2 命令（5 files +158） |
| `8355ba7d` | feat(tauri): G4-6b-4 批次写面 3 命令（5 files +241/−6） |
| `4ebaa7ab` | feat(tauri): G4-6c prepare-draft-context 迁移（5 files +532/−5；generation 域收口） |
| `f4e06276` | feat(tauri): G4-6c 阶段 2 db.rs generationRunHandle 三分支接线（4 files +401/−47） |
---

## 交接给下次会话（**从这里接**）

### 1. 当前工作区状态

- `git status --porcelain`：仅本快照文档在途更新（docs 提交前）
- HEAD：`f4e06276 feat(tauri): G4-6c 阶段 2 db.rs generationRunHandle 三分支接线`
- **G4-6c 全波收口**：阶段 1（4ebaa7ab，prepare-draft-context 迁移，generation 域频道清零）+ 阶段 2（f4e06276，db.rs 三分支接线 + assert_sources_current 方法 + BlueprintCommitRangeRequest 字段承载）；频道计数 219 命令 / 218 覆盖 / 72 未迁移（本波纯接线无新频道注册）；G4-7 with_agent_child_effect 待派
### 2. 下一步（1-2-3）

1. **G4-7：with_agent_child_effect 移植**（前置依赖已就位：assert_sources_current 方法 + 源断言闭包接线；基线 generation-controller.ts withGenerationAgentChildEffect + db-controller.ts:182-193 db:project-core-commit-generated 频道迁移 + blueprint-commit-range 新提交路径活化）
2. 上游合并波次（electron 基线同步 main-generation-owner +133 / generation-run-repository +178；src/ 重设计 162 文件 +9139；docs/ 镜像重同步 169 触次；G4-5 已迁移命令契约对齐 generation:list-directory-progress return → DirectoryGenerationCatalog）
3. B26 全线已收口（2026-10-13 全量核实 48 个 browser 测试文件：27 已迁移 __TAURI_INTERNALS__ + 21 纯组件测试 + 0 旧桩）

### 3. 阻塞项与待授权项（**不得删除，须逐条确认后更新**）

| # | 项 | 状态 |
|---|---|---|
| B14 | 真 Windows 自动更新（tauri-plugin-updater） | 待批次 H 后段 |
| B22 | 两段式关窗 dirty 分支 | 待 GUI 实测 |
| B24 | 整屏瞬黑（暂缓） | 暂缓 |
| N1 | import-novel-resume 1/32 浏览器模式既存应用行为（2026-10-12 登记） | 单独立项评估中（修复方向：预览 effect 加 inspectionSessionRef 校验） |
| N2 | 72 个未迁移 invoke 频道迁移批次 | G4-6c 全波收口（2026-10-13，219 命令 / 218 覆盖 / 72 未迁移——阶段 2 纯接线无新频道注册）；generation 域频道清零（generation=0） |

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
| `pnpm run check:channels` | `tauri-app/` | 契约 invoke 频道 290（事件频道 7）；已注册命令 219 → 覆盖 invoke 频道 218；未迁移 invoke 频道 72 [agent-generation=8 cloud-backup=8 legacy-roster=7 character-proposal=6 finalization-generation=5 finalized-character=5 graph-generation=5 project=5 character-avatar=4 dialog=4 editor-inline=4 kb=3 startup=3 db=2 import-generation=2 character-identity=1]；「命令名与契约频道一一对应 ✅」（exit 1 属预期——未迁移 > 0） |
| `cargo test --lib` | `tauri-app/src-tauri/` | test result: ok. 989 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 56.59s（988 既有 + 新增 blueprint_commit_range_generation_handle_branches_test） |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | Finished `dev` profile [unoptimized + debuginfo] target(s)（0 错误 0 告警） |
| `pnpm typecheck` / `pnpm run lint` | `tauri-app/` | exit 0 / exit 0 |
| `git status --porcelain` | 仓库根 | 仅 docs-fork/handoffs/2026-10-13-tauri-migration-status.md 在途（本快照） |
| `git log -1` | 仓库根 | f4e06276 feat(tauri): G4-6c 阶段 2 db.rs generationRunHandle 三分支接线 |
---
