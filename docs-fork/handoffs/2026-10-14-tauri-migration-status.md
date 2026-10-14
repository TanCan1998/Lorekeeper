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

## 快照（最后更新：2026-10-14 · 第 49 次）

> 已验证代码提交：`a1884ac96d1b1c7c7342b6d07a71b47096fdf815`；核验截至 2026-10-14 21:55 +08:00。主会话直接实施，没有派发代理。快照文档随后独立提交；最终 HEAD 以 Git 和本机 pipeline 为准。

| 项 | 值 |
|---|---|
| 仓库 / 分支 | `F:\workplace\pi\ai-novel-writer` / `master`；关闭切片 `cf23c0ba` 已推送；短数据库访问代码 `a1884ac9` 已提交，与本快照一起推送 origin/master |
| 产品身份 | Lorekeeper（设定司）/ `com.tancan1998.lorekeeper` |
| 命令 / invoke 覆盖 / 未迁移 | check:channels exit 0：243 / 242 / 48；契约 invoke 290、事件 7；本轮无命令注册 |
| `cargo test` | 1267 passed / 0 failed（55.32s）；bin/doc-test 无失败；本次新增 5 项短数据库访问与重放测试 |
| `cargo check --all-targets` | exit 0，零告警 |
| `pnpm typecheck` / `pnpm run lint` | 均 exit 0 |
| 前端测试 | 本次定向 generation runtime/renderer/harness：3 文件 / 43 项通过；既存扩展前端与 Agent store 失败未复验，不能宣称全绿 |
| 格式 / 差异检查 | 修改 Rust 的定向 rustfmt --check、git diff --check 均 exit 0 |
| 当前范围 | 运行服务支持按阶段的同步数据库访问回调；旧直接连接接口仍兼容。生产 owner/IPC 尚未切换，网络长锁完整拆解与事件接线未完成 |
| 隔离 / 依赖 | Electron、根 src、docs/、根配置和锁文件无改动；无新依赖/schema/权限/CI；本机 AGENTS 的提交授权已按用户确认更新，不入库 |
| GUI 冒烟 | 未做 |

## 本次更新（第 49 次：关闭切片推送与短数据库访问基础）

- 用户确认当前关闭切片已人工核验，并明确授权后续普通迁移切片在自动检查通过后直接提交、推送，免逐次人工审核。授权不恢复代理派发，不放宽新依赖/schema/权限/CI/ADR/旧 Electron 清理等先确认边界。关闭切片 `cf23c0ba` 已推送 origin/master。
- generation_run_service 增加 GenerationDatabaseAccess 同步短回调入口：幂等读取、预算预留、标记分发、预算时钟、周期/增量 flush、终态成功/失败结算和终态读取均经该入口。网络 Future 不保存回调中取得的连接/守卫；原 execute/execute_blocking 通过直接连接适配共用逻辑，兼容现有 owner/内部调用。
- 回调因连接/身份不可用而拒绝时，不回写旧库；根级内存围栏、取消信号和 unsaved tail 保留未落盘正文。新增测试在分发后撤销回调授权，验证原库正文和结算字段未被修改。
- 并发在途登记改成短锁原子 get-or-insert，同键不同哈希先拒绝，防止两线程都读到空状态后重复启动；等待在途操作之前释放 pending 锁。watch 完成通知改为保留完成值，即使先完成、后订阅也不会丢失结果。
- 新增五项回归：跨线程网络等待时可读取同一连接、取消正文持久化；在途重放不持 pending/连接锁且只产生一条 attempt；分发后撤销授权不写旧库；迟到订阅立即取得完成值；预留前同键哈希冲突拒绝且不分配 attempt。均为本地合成 provider，不是 HTTP 假服务器或真实模型验收。
- 完整 Rust 1267 passed / 0 failed；check 零告警；定向 rustfmt、typecheck、lint、check:channels 和定向前端三文件 43 项通过。证据 `.pi/tmp/b33-scoped-{target,test,check,rustfmt,typecheck,lint,channels,vitest}.log` 与 `b33-scoped-reviewed.diff`，均不入库。Rust diff SHA256：`6336c02a9b0c9de57059797251b653d7a983e821e851324ddd094cfcaf234a59`。
- **关键边界**：当前生产 IPC 仍由 owner 持库调用直接连接适配，故普通读取仍可能等待。本切片完成服务层短锁基础与测试，不代表生产网络长锁已消除；无新增数据库连接、schema 或依赖。
- **下一步**：把 owner 准备/分发/回执投影拆开，冻结 provider 请求与在途守卫；生产回调每次按 owner → 数据库锁序重新认证项目路径、epoch、会话和连接。关闭钩子还需在不持业务锁时等待执行控制退出，避免短锁化后过早关闭库；补真实 AppState 并发读取/暂停/关闭/切换及本地 HTTP 假服务器测试后自动提交推送。
- 快照/推理事件、材料提示哈希、短细纲重试、知识采集、特殊域准入、内部 Agent/子代生命周期、B3-M4/M5、既存扩展前端失败和 GUI 专项仍待续。当前任务/占用只维护 `.pi/tmp/pipeline-state.md`。

---


## 本次更新（第 46 次：B3-3 普通 begin 策略、租约与运行创建）

- 主会话直接推进，没有派发、恢复或重派代理。新增写面为 generation_run_repository、generation_run_service、main_generation_plan，继续修改 main_generation_owner；累计六个 Rust 文件及三份 fork 文档未提交/推送，人工逐项核验待完成。
- 普通根运行 begin 移除占位：校验输出合同及最多八个唯一用途覆盖，获取模型租约，冻结安全模型收据，在 immediate 事务内规划、构造真实绑定、创建根/运行和投影视图；成功移交 owner，失败由守卫回收临时租约及事务。
- 仓储/服务增加调用方事务入口，不嵌套 BEGIN、不提前提交。重复 begin 复用既有运行及租约；重新打开项目仍按不含 epoch 的根行动键读回旧运行，不分配新预算或可执行租约，普通 resume 再绑定新 epoch。
- 规划范围对照真实 project_core 总章数；显式目标必须唯一且合法，仅缺省时使用 600，并冻结规范十进制文本。数值解析补 BOM/空白和无符号进制前缀。草稿 preparation 缓存冻结知识快照，要求材料决策并复核准备来源；命令层知识采集仍待接入。
- 新增 11 项 begin 测试：真实写入与安全投影、幂等租约、输出合同、写入失败根/运行回滚、来源/准备失败租约回收、特殊域/外层事务拒绝、nonce 冲突、规划目标/默认值/范围、准备材料与来源漂移、调用方事务回滚、跨纪元重放与 resume。
- 保持特殊域关闭：Agent 初始化/新子代、导入、审阅修订、批量、目录续接及内部子代根预算准入未接入；仅保留既有 Agent 子代幂等读回。普通 begin 拒绝调用方外层事务；restart 及 execute_inner 未激活。
- 验证：完整 cargo test **1247 passed / 0 failed**（88.96s），cargo check --all-targets 零告警，typecheck、lint、check:channels、定向 pnpm test（3 文件 / 43 项）、git diff --check 均 exit 0；本次未运行扩展前端失败套件或 GUI，没有真实外部模型调用。
- 证据：`.pi/tmp/b33-begin-{target,test,check,typecheck,lint,channels,vitest}.log`、`b33-begin-reviewed.diff`，均不入库。累计 Rust diff SHA256：`e91a29de09db12a4bada70071eea02b934e2507913834a1b5cd30d6134f80f49`；无新依赖/schema/权限/CI、Electron、根 src、docs/、根配置或锁文件改动。
- 下一步 execute_inner 的网络分发、流式 flush/settle、终态/取消与事件接线，再接知识采集及特殊域准入；补批次/真实 Agent 子效应及前端既存失败专项复验。不能将普通 begin 与草稿提交当作主生成端到端完成。

---


## 本次更新（第 45 次：B3-4 草稿提交事务与幂等收据）

- 写面仅三个 Rust 文件：draft_repository 提取 `create_in_transaction`；generation_draft_effects 新增真实提交；main_generation_owner 移除草稿提交占位并接入真实来源复核及 Agent child effect。
- 独立提交使用 immediate 事务。复用调用方事务时增加保存点守卫：只释放本次保存点，不提前提交外层；来源、草稿正文/版本/依赖、attempt 的 draftCommit 收据及代理效应同一原子边界，失败/异常退出回滚本次效应。
- 校验 write 来源、章节/操作/batch 身份、visible composition 算法和哈希、最后 attempt 的 stop 终态、目标单位正安全整数及 70% 正文下限。目标字符串兼容 Number 的十进制、指数及无符号进制前缀。
- 草稿依赖由真实 candidate/finalized source refs 构建，复用既有仓储验证；批次写前后复核进度。提交收据写入既有 attempt usage JSON，不新增账本或数据库结构。收据使用 camelCase，保留旧 Rust snake_case 读取 alias；既有提交在正文/收据完整且哈希一致时幂等返回，不重新要求来源未变化。
- 新增 11 项测试：实际创建与重放、哈希/目标/合成冲突、截断与短文、蓝图/候选来源漂移、依赖保留、收据失败回滚/重试、外层回滚、外层捕获失败后提交仍无残留、纪元/取消拒绝、camelCase/旧字段读取及数值解析。
- 前端扩展检查暴露既存缺口：MIGRATED_CHANNELS 与注册表/旧 97 断言不一致（集合口径 56，而注册口径 48），review-revision 四频道缺参数登记，两处既有 review Rust 入参缺 camelCase；Agent store 九个模拟测试失败。本轮未改这些文件，HEAD 原文确认 review 两结构缺标注。本轮 StoredDraftCommit 扫描问题已修正，不修改测试以掩盖其他失败。
- 证据：`.pi/tmp/b34-{test,check,typecheck,lint,channels,vitest,vitest-target,target}.log`、`b34-reviewed.diff`，均不入库。Rust diff SHA256：`348491d1c23a4a8696b03cb455ab624e69738c4c0652f03357fb35ceb7a2c6a1`。第 44 次 owner 缓存/普通恢复/关闭切片已提交为 `efb96e0d`，此前授权不代表本轮已人工核验。
- 下一步：继续 B3-3 普通 begin 的策略/租约/运行创建，再接 execute_inner 的分发/终态/事件；补批次与真实 Agent 子效应专项及前端既存失败复验。知识采集、特殊域恢复准入、B3-M4/M5 与 GUI 仍未完成；无 schema/依赖/权限等新增授权项。活动任务和运行事实只维护在 pipeline 状态。
---


## 本次更新（第 43 次：B3-3 模型只读证据、真实源绑定与草稿准备绑定）

- 主会话直接推进，无子代理或新 runId；HEAD 仍为 `2fbddb47`，本轮代码与文档未提交/推送。沿用第 42 次已验证的流式核心、分发器与可见流；这些实现仍未接入 owner 执行主体。
- 新增 `main_generation_sources.rs`：Lorekeeper 模型读取与必需字段/协议校验；用既有租约工厂生成 `read-only-evidence`、时间 0/0 的安全模型收据，不登记可执行租约，不走过期判定，不泄漏 API key 或地址。owner 捕获全局根，模型、提示词、技能均无 Electron 目录回退。
- 新增 `builtin_writing_skills.rs/json`：七项双语内置技能的只读目录与元数据。内置提示词先投影规范 JSON，避免变量 HashMap 遍历顺序造成伪漂移。沿用既有 source binding 构造/重建与资产权限门禁。
- owner `current` / `capture` / `assert_sources_current` 接入真实绑定；检测模型、作者事实、技能或蓝图事实变化。主生成策略补 `to_value`，复用既有序列化器，并验证严格读回。
- `draft_preparation_binding` 移除旧构造占位：同一模型快照生成证据与策略，批次读取根策略，冻结六章蓝图窗口、知识快照及 visible-text 合同。不创建运行；命令层知识采集仍待接线。
- source binding 在已存在事务时共享调用方连接，不嵌套 BEGIN、不提交调用方事务；独立读取仍创建读事务。真实 host 测试证明事务内作者效应可见，回滚后恢复原绑定。
- 新增 13 项测试：内置技能 2、模型/源绑定 6、真实 host 3、草稿准备绑定 2；覆盖双语稳定性、证据脱敏、模型移除/错误协议、跨纪元重建、非法技能身份、真实漂移及事务回滚。仅临时合成数据，无真实模型配置或外部网络。
- 早期夹具误用了不存在的 tempfile 依赖，改为既有 std 临时目录模式，未加依赖。蓝图漂移夹具由非源事实 notes 改为真实 title 事实，未放宽生产校验。最终全量通过。
- 独立验证（2026-10-14 14:19–14:22 +08:00）：完整 cargo test **1217 passed / 0 failed**（49.30s，bin/doc-test 无失败），cargo check --all-targets 零告警，check:channels（243/242/48）、typecheck、lint、定向 vitest（3 文件/43 项）、git diff --check 均 exit 0。证据 `.pi/tmp/b33-sources-{test,check,channels,typecheck,lint,vitest,target,host,preparation}.log`，前切片日志保留，均不入库。
- 跟踪 Rust diff SHA256 `69ee20372af98c1d5ca52bebb6d6b6dd6482479e465a99c241e412372d0e00c9`。新文件不含于该哈希：sources `403884067d8256bcb1693179645cf737499ea32df6a447f5c447cc85ee6ff9fb`；builtin skills rs `b6c9d4635450d295b76bf881eb813195b478cd4bf746a4c4c56ce0158d8fce9a`；skills json `259c8326ed18f2bfbe8c889149e8110c910d875226bc87d739b36e78093617a8`。可见流/分发器新文件哈希沿用第 42 次，记录在 pipeline。
- 下一步 owner begin/execute_inner/resume、service 生命周期与事件发射、共享符号去重，再 B3-4。B3-M4/M5 专项与 GUI 未完成；主会话复核不替代人工逐项核验。无新依赖/schema/权限/CI、Electron、根 src、docs/、根配置或锁文件变更。

---


## 本次更新（第 41 次：B3-M 提交与 B3-3 流式核心抽取切片）

- B3-M Rust diff 复核 SHA256 与第 40 次一致，提交 `2fbddb47`（15 文件）；未推送。主会话代码复核不替代人工逐项核验，人工审核记录仍未取得。
- `commands/llm_generation.rs` 抽取公开进程内 `generate_stream_core`，IPC 后台任务委托；不持有 AppState、不访问 llm_streams、不发射 UI 事件，取消探针及回调原样传给 provider。启动前已取消时不发起网络调用，取消终态由调用方结算。
- 新增 3 项测试：启动前取消（两协议均无回调）、本机 OpenAI SSE、本机 Gemini SSE；校验协议路由、终态正文、推理回调、usage 证据及输出键名透传。无真实外部 API 调用。
- 初次 OpenAI 测试误将原始 chunk 等同可见正文而失败；复核后修正断言，保留 IPC 带内思考标签合同。后续 GenerationDispatcher 必须过滤这些 chunk，不能直接写入 artifact。
- 验证版本：HEAD `2fbddb47` + Rust diff SHA256 `4d926f090a54e97b638512d972d01ed9843960b1eda678dc15f674d3bb02f4d6`；2026-10-14 13:23–13:27 +08:00。完整 cargo test 1189 passed / 0 failed（55.59s，bin/doc-test 无失败）；cargo check --all-targets 零告警；check:channels（243/242/48）、typecheck、lint、git diff --check 均 exit 0。
- 证据 `.pi/tmp/b33-core-{test,check,channels,typecheck,lint}.log`，不入库。仅 Rust 单文件；无新依赖/schema/权限/CI、Electron、根 src、docs/、根配置或锁文件改动。
- 边界：owner begin/execute_inner/resume、current/capture 真实重建、GenerationDispatcher、事件发射仍未完成；B3-4、M4/M5 与 GUI 待续。本切片与文档未提交，待人工核验；状态只在收口后统一更新。

---


## 本次更新（第 40 次：G4-B3-M 推理机器对齐专波，主会话直接收尾）

- 原 attempt 1 / runId `b390feab-b2ee-435b-8553-d1d357ae4edb` 已 complete，但任务未完成（2 个失败、M3 未实施）；本次未派发或恢复子代理，由主会话修复并独立验收。
- M1：移除旧 `llm/params.rs` 双实现，生成与连接探测统一用 `generation_parameter_policy`；补钳制差例 `8192 / 4096 / 32768 → 4096`。
- M2：补齐五类 adapter、SiliconFlow 三模型、手动映射优先校验与协议门禁、租约字段透传、OpenAI 载荷翻译。复核补齐基线六档强度（含 xhigh）、OpenAI 自定义 wire 原样透传、DeepSeek enabled / 强度重映射，以及 off 无映射时 unsupported（不强制开启推理）。
- 修复原失败夹具：DeepSeek high→enabled 是基线合法映射，非法值改为 invalid 并补合法断言；numeric() 恢复继承 unknown() 的端点、模型名与 65536 上限，不改容量算法以迎合错误夹具。
- M3：共享 token_liability / 规划单位校验 / 章节默认值 / 协议常量；f64 入参保留字符串薄适配，Value 内容构造器保留；LlmGenerateOptions 新增 output_token_parameter，OpenAI 消费选择的请求键名，所有构造点补齐（Gemini 仅测试构造面）。
- 实际 Rust 写面 15 文件（含旧 params 删除）；无 Electron、根 src、docs/ 或根锁文件改动。仅定向 rustfmt（skip_children），未清理运行产物。
- 验证版本：HEAD `1e45cc1a` + Rust diff SHA256 `f5fee6436181dd78e66c76b4f7b332731dbc479d99b4e48c667433c1482def7e`；验证时间 2026-10-14 13:08–13:15 +08:00。`cargo check --all-targets` exit 0 / 零告警；完整 `cargo test` exit 0 / 1186 passed；频道检查 exit 0 / 243 命令、242 覆盖、48 未迁移；typecheck / lint / 定向 vitest 均 exit 0；`git diff --check` exit 0。
- 本机证据：`.pi/tmp/b3m-check.log`、`b3m-test.log`、`b3m-run-check-channels.log`、`b3m-typecheck.log`、`b3m-run-lint.log`、`b3m-vitest.log`（不入库）。
- 未完成边界：B3-3 / B3-4 未实施；owner 的本地副本与 current/capture 保守占位仍待接线；B3-M4 传输诊断与 B3-M5 Gemini 错误部分内容仍待后续专项；历史未移植规划断言不计为本波覆盖；GUI 未测。提交等待人工逐项核验，未提交或推送。

---

## 本次更新（第 39 次：B3-2b —— 生成运行服务本体，generation_run_service + agent_response_protocol 1:1 平移）

### 1. 新增（本轮，5 写面 +2851/−2）

| 项 | 内容 |
|---|---|
| `services/agent_response_protocol.rs`（新建） | 基线 `src/shared/agent-response-protocol.ts` 1:1 平移：`parseAgentResponseProtocol` 三形态（原始调用 `名称\n{...}` / 信封 `{"name","arguments"}` / 标记块 `<tool_call>...</tool_call>` 内 `<name>…</name><arguments>…</arguments>` + 空工具 `<name></name>` 特例）；隐藏标签栈式配对扫描（think/thinking/analysis/reasoning）；中断投影（未闭合 tool_call 的 name 前缀扣留）；多空行压缩；12 单测 |
| `services/generation_run_service.rs`（新建） | 基线 `electron/services/generation-run-service.ts`（355 行）1:1 平移：`execute` 幂等重放（findInvocation）+ 在途去重（pending 映射）+ failedRoots 围栏 + `select!` 分发循环（1s flush 定时器 / deadline / 取消 watch 令牌）+ `settle_provider_usage` 三态记账（trusted / unknown 仅算术自洽 / untrusted 保守）+ catch 失败码分类（NETWORK_ERROR / GENERATION_PROVIDER_FAILED / GENERATION_AGENT_ACTION_LIMIT / GENERATION_CANCELLED）+ 增量 flush 经 mpsc 通道（4096 字节阈值 / 8MB agent 协议上限 / 64 工具调用上限）+ 公共 API（open/resume/pause/cancel/restart/suspend_for_project_close）+ 20 单测（含 MockDispatcher） |
| `repositories/generation_run_repository.rs` | `GenerationExecutionReceipt` 补 `unsaved_tail: Option<String>`（serde default——对齐基线 unsavedTails 记录） |
| `Cargo.toml` | 启用 tokio `"time"` feature（定时器 select! 需要；tokio 依赖已有，Cargo.lock 不变） |
| `services/mod.rs` | 登记 2 模块（agent_response_protocol / generation_run_service） |

**行为对照（Electron ↔ Tauri）**：
- `execute` 与基线 :545-613 逐行对照：requestHash 校验（`GENERATION_REQUEST_HASH_INVALID`）→ pending 幂等 → failedRoots 围栏（`GENERATION_RESERVATION_INVALID`）→ reserve → markDispatched → 分发循环（flush 1s / deadline / cancel）→ settle（usage 三态 + 收据写回）→ catch 分类——顺序与错误码逐字一致
- `settle_provider_usage` 与基线 :73-96 对照：`Number.isSafeInteger` 校验、unknown 策略仅信任算术自洽 total、accounting 须等于 policy.reasoning、included-in-completion 拒绝 r>c、separately-billed 要求 r!==null 且 actual=p+c+r——全部一致
- 取消语义：基线 abort → `GENERATION_CANCELLED`；Rust 侧 watch-channel 令牌 `token.cancel()` → select! 分支 → `GENERATION_CANCELLED`——时序对齐（失败分支立即 abort，不设延迟标志位）

**⚠️ 偏差登记**（均经规则批准）：
- 正则 look-ahead `(?=[\s>])` 以捕获组 `([^<>]*)` + `marker_tag_boundary_valid` 函数等价（regex crate 不支持 look-around——官方文档确认「lacks look-around and backreferences」）；tagged-empty-tool 基线反向引用 `\1` 以双捕获组比较替代
- 取消句柄为进程内 watch-channel 令牌（`GenerationCancelToken`），不经 IPC `llm_streams` 注册表（B3 裁决项③）
- 增量 flush 经 mpsc 通道交付（`&Connection` 非 `Send`，回调不能直接持 conn）
- trait `GenerationDispatcher::dispatch` 显式统一生命周期 `'a`（`'_` 隐式绑定 `&self` 生命周期，但 future 实际借用 `options`）
- 发射器活化（去 dead_code + 接线）随 B3-3——execute_inner 仍 fail-closed，B3-2b 期间无服务构造点可接线（B3-2b 范围调整，编排者裁定）

### 2. 前端登记

- 无（本波无命令注册、无 TS 改动面）

### 3. 提交清单

| Commit | 说明 |
|---|---|
| `fe33fcd6` | ✨ feat(tauri): G4-B3-2b 生成运行服务本体——generation_run_service + agent_response_protocol 1:1 平移（5 files +2851/−2） |
| `b1d72e92` | ✨ feat(tauri): G4-B3-2a 流式传输层扩展——reasoning/usage evidence 回调 + 带内推理抽取（IPC 路径行为不变）（前置依赖） |

---

## 本次更新（第 38 次：G4-10d —— agent-generation 子波次 D，占位活化翻转 fail-closed）

### 1. 新增（本轮，3 写面 +149/−69）

| 项 | 内容 |
|---|---|
| `services/main_generation_owner.rs` | ① `with_agent_child_effect` 活化（基线 main-generation-owner.ts:1301 逐字）：`require_run(conn, handle, false)`（非执行语义，错误已是 MUTATING 信封经 `?` 透传）→ 方法级构造 `AgentGeneration`（对齐 `admit_workflow` 先例）→ 委托服务层 `with_child_effect`（agent_generation.rs:1795，本波未改），错误经前缀感知折叠为 MUTATING 信封（对齐 `commands::db::blueprint_commit_range_result` :818 先例，避免 `Error: Error: ` 双重前缀）；方法签名不变（占位参数恢复为实参）。② `commit_draft` 接线（基线 :1292-1297）：重放分支保持现状不动（基线重放不走 withChildEffect）；新提交写入路径包进 `agent.with_child_effect(&handle, \|_conn| <B3 占位 Err>)`——门禁链（运行身份 / 服务层 registration / 源一致性）先行校验，效应写入路径保持 B3 fail-closed（基线 draftEffects.commit 写入面与 assertSourcesCurrent 写入时源断言回调属未迁移写入路径，本波不实现）；B3 占位文案更新（去掉已不成立的「agents.withChildEffect 待移植」表述）。③ `approve_character_proposal` 头注更新（活化事实 + 诚实化项） |
| `commands/db.rs` | 两处测试断言由 `GENERATION_AGENT_CHILD_EFFECT_UNMIGRATED` 改为 `GENERATION_RUN_MISSING`（活化后实测行为：夹具 `activated_state` 仅初始化 project_core 台账、无 generation_runs 行，handle 指向的 run-1 不存在 → 门禁链首环 `require_run` 的运行读取失败 → `GENERATION_RUN_MISSING` 经 `map_generation_error` 透传）；注释记录实际命中路径与依据。调用点代码（:263、:945）已是 G4-7 结构接线，未改动 |
| `commands/character_proposal.rs` | approve 已经 `owner.approve_character_proposal` → `with_agent_child_effect`（:3046）接线，本波随 owner 方法活化生效；无占位码断言（grep `CHILD_EFFECT_UNMIGRATED` commands/ 清零），仅命令头注更新活化事实表述 |
| 测试 | 无新增（1033 计数不变）：db.rs 两处断言改语义断言；目标测试单独复跑 2 passed |
| Schema / 依赖 | 无变更 / 无新增 |

**行为对照（Electron ↔ Tauri）**：
- `with_agent_child_effect` 与基线 :1301 逐行对照：`requireRun(handle)`（非执行语义）→ `agents.withChildEffect(handle, effect)`——Rust 侧 require_run(false) → 方法级 AgentGeneration 构造 → 服务层 with_child_effect 委托，顺序与语义逐字一致
- `commit_draft` 与基线 :1292-1297 对照：`requireRun(request.handle)` → 事务内重放分支（readCommit 命中 → 不走 withChildEffect）→ 新提交路径 `agents.withChildEffect(request.handle, () => draftEffects.commit(...))`——Rust 侧结构一致（重放分支已内联实现，新写入路径 B3 占位）
- 错误信封：服务层裸码（`GENERATION_AGENT_CHILD_NOT_AUTHORIZED` 等）经前缀感知折叠为 `Error: <CODE>` MUTATING 信封，与基线 controller catch 映射（透传正则 `^GENERATION_[A-Z_]+$`）最终形态一致

**⚠️ 诚实化占位 / 降级项**（均已记 doc 注释）：
- host `current`/`capture` 保持保守占位：基线 capture 经 `deps.rebuildBinding(run.binding, currentModelReceipt(run.binding))` 重建；`rebuild_generation_source_binding` 已迁移（generation_source_binding.rs:3636），但 `currentModelReceipt`（基线 :251-256：模型注册表 loadModel + 只读租据收据构造）未迁移——with_child_effect 写入的 agentSourceTransition 为退化形态（before/after 均取自占位 capture，hash 相等；binding 为效应前快照）。源变迁真实检测随 currentModelReceipt 迁移波次补齐；激活后 sourcesCurrent 对「expected 与 run.binding 一致」场景与基线等价，源已变场景检测缺口按 G4-10b 已记录偏差延续
- commit_draft 新写入路径仍 B3 fail-closed（门禁链先行校验后效应报「主生成草稿提交尚未迁移」）——完整草稿写入（DraftRepository.create + usage 回执写回 + assertSourcesCurrent 写入时源断言回调）属 B3 创建核心波次
- begin 主体 / execute 分发核心 / resume / execute_round 主体仍 fail-closed（依赖 B3 创建核心与 LLM 双阶段执行循环迁移）
- 编排者复验修正：worker 交付中 `approve_character_proposal` 头注有一行重复文档行（`- **withChildEffect**：基线事务门禁` 相邻重复）——编排者独立复验时去重（纯注释，无语义变化）

### 2. 前端登记

- 无（本波无命令注册、无 TS 改动面）

### 3. 提交清单

| Commit | 说明 |
|---|---|
| `b3d9eb78` | ✨ feat(tauri): G4-10d agent-generation 子波次 D——占位活化（with_agent_child_effect 与调用点接线）（3 files +149/−69） |
| `28daa346` | ✨ feat(tauri): G4-10c agent-generation 子波次 C——域工具效应移植与 8 频道命令注册（前置依赖） |

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
- HEAD：`fe33fcd6 ✨ feat(tauri): G4-B3-2b 生成运行服务本体——generation_run_service + agent_response_protocol 1:1 平移`
- ⚠️ 上一份快照（第 35 次）中「下一波 = agent-generation 域（4 子波次 A→B→C→D）」：A（G4-10a）、B（G4-10b）、C（G4-10c）与 D（G4-10d）**已收口**；「安全项优先：Rust begin 的 agentWorkflowRegistrationId 键无 admitWorkflow 等价校验」**已闭合**（G4-10b）

### 2. 下一步（1-2-3）

1. **推理机器对齐专波**（B3-M1 + B3-M2 + B3-M3 合并，均涉及既有文件/在役命令行为，一次复验）：B3-M1 resolve_generation_parameters 双实现合并（IPC 命令 maxTokens 钳制语义）、B3-M2 ReasoningAdapter 扩 siliconflow-v4-thinking / openai-thinking-budget 2 变体 + SiliconFlow 目录补全、B3-M3 私有依赖合并（main_generation_plan.rs 本地平移副本提为 pub）→ **B3-3**（owner 主体接线：begin/execute_inner/resume 主体 + currentModelReceipt + rebuildBinding 真实化 + B3-2 剩余②llm_generation.rs 流式核心抽取为 pub 服务函数 + ③事件发射活化，dispatch 经 GenerationDispatcher trait 由 llm::openai/llm::gemini 实现）→ **B3-4**（commit_draft 写入路径）；研究入口 `.pi/tmp/research/agent-generation-b3.md`
2. **currentModelReceipt 迁移**（host capture 真实化）：基线 main-generation-owner.ts:251-256（模型注册表 loadModel + 只读租据收据构造）——迁移后 `impl AgentHost for MainGenerationOwner` 的 `current`/`capture` 保守占位可替换为 `rebuild_generation_source_binding`（已迁移，generation_source_binding.rs:3636）真实重建，agentSourceTransition 退化形态与源已变场景检测缺口随之补齐
3. **后续候选**：legacy-roster begin/execute 活化、上游合并独立波次、import-novel-resume 单项修复评估、agent-generation 域效应端到端测试覆盖（随 B3 波次补齐）（不作为已授权新增依赖/schema/CI 工作）

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
| `pnpm run check:channels` | `tauri-app/` | 契约 invoke 频道 290（事件频道 7）；已注册命令 243 → 覆盖 invoke 频道 242；未迁移 invoke 频道 48；命令名与契约频道一一对应 ✅（exit 0；编排者独立复验，HEAD b3d9eb78 工作区——本波无命令注册，计数较第 37 次不变） |
| `cargo test --lib` | `tauri-app/src-tauri/` | test result: ok. 1184 passed; 0 failed; 0 ignored; finished in 53.75s（exit 0；编排者独立复验，HEAD fe33fcd6 工作区） |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | Finished `dev` profile [unoptimized + debuginfo] target(s) in 10.12s（exit 0；编排者独立复验，0 错误 0 警告） |
| `git status --porcelain` | 仓库根 | 空 |
| `git log -1` | 仓库根 | `fe33fcd6 ✨ feat(tauri): G4-B3-2b 生成运行服务本体——generation_run_service + agent_response_protocol 1:1 平移` |
