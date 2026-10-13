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
| 已注册命令 | 205 |
| 覆盖 invoke 频道 | 204 |
| 未迁移 invoke 频道 | 86 [generation=14 agent-generation=8 cloud-backup=8 legacy-roster=7 character-proposal=6 finalization-generation=5 finalized-character=5 graph-generation=5 project=5 character-avatar=4 dialog=4 editor-inline=4 kb=3 startup=3 db=2 import-generation=2 character-identity=1] |
| orphan | 无 |
| `cargo test --lib` | 988 passed / 0 failed（984 既有 + 4 新增平移测试） |
| `cargo check --all-targets` | 0 错误 0 告警 |
| `pnpm typecheck` / `lint` | exit 0 / exit 0（本波纯 Rust 服务层，无 TS 改动） |
| 定向 `vitest` | 未跑（本波无前端改动） |
| 已完成批次 | G4-1 / G4-2 / G4-3a / G4-3b / G4-5 / **G4-6a**（2026-10-13 新增） |
| 当前阶段 | G4-6 owner 写面——G4-6a 逻辑层收口；G4-6b 写面 9 命令 + 2 事件待派 |
| 依赖 | 无新增（rusqlite 0.40.2 [bundled] / reqwest 0.13 / hnsw_rs / jieba 等不变） |
| GUI 冒烟 | 未做 |
| 双栈隔离 | 维持（Tauri 用 `AI_NOVEL_LOREKEEPER_HOME` / `~/.lorekeeper`；项目库 `<root>/.lore/lorekeeper.db`） |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc（`D:\Environment\rust`） |

---

## 本次更新（第 26 次：G4-6a —— generation 源绑定逻辑层移植）

### 1. 新增（本轮）

| 项 | 内容 |
|---|---|
| 仓储 / 服务 | 新建 `services/generation_source_binding.rs`（3630 行，平移自 electron/services/generation-source-binding.ts 615 行）；`services/mod.rs` +1 行注册 |
| 命令 | 无（纯服务层——写面命令在 G4-6b） |
| Schema | 无变更 |

**实现要点**：
- 公开函数 build/rebuild/compare_generation_source_binding(s) + 4 类型（SafeGenerationModelReceipt / GenerationSourceBindingInput / GenerationSourceBindingDependencies / GenerationSourceBindingResult）+ GenerationKnowledgeSnapshot 补齐（electron src/shared/generation-knowledge.ts 平移）
- 哈希对齐：createHash（node:crypto）→ Tauri 侧既有哈希形态；`MATERIAL_DECISION_UNIT_METHOD_VERSION='utf8-bytes-v1'` 口径逐字对齐
- 深度比较：isDeepStrictEqual（node:util）→ serde_json::Value 比较
- hard_links：std 无 stable API（`number_of_links` 属 unstable `windows_by_handle`，probe 实测 E0658）→ 按 portable_runtime_freeze.rs / import/parsing.rs 先例：Unix `MetadataExt::nlink()` / Windows 直调 Win32 `GetFileInformationByHandle` 取 `nNumberOfLinks`；无法取计数时 fail-closed 按非 1 处置
- rusqlite 行读取：按 current_character_projection.rs 先例改为 `statement.query(params)` + `rows.next()` 循环（闭包 `Result<_, String>` 与 rusqlite `Result<_, rusqlite::Error>` 不兼容）
- PlotOutlinePolicy：generation_run_repository.rs 中为真私有 struct——按同型私有 fn `plot_outline_policy` 既有移植先例在本地定义同型 struct
- let-chain（edition 2021）→ `is_some_and`（文件内既有先例）

**⚠️ 诚实化占位 / 降级项（如有，必须显式列出）**：

| 项 | 阻塞于 | 表现形式 |
|---|---|---|
| hard_link_count 降级 | Windows `number_of_links` 属 unstable（E0658） | Win32 `GetFileInformationByHandle` 直调（unsafe FFI）；句柄打开失败时 fail-closed 按非 1 处置（基线语义：hard link 数 ≠ 1 视为符号链接/异常文件）——偏离已文件头注释标明 |

### 2. 前端登记

- `src/services/ipc-client.ts`：无变化（纯服务层）
- `src/shared/migrated-channels.ts`：无变化（无新命令注册——`pnpm run check:channels` 复测 205/204/86 与 G4-5 收口一致）
- `test/`：无变化（cargo test --lib +4 平移测试在 Rust 侧）

### 3. 行为对照（Electron ↔ Tauri，如做了才写）

- 纯逻辑层平移：cargo test --lib 988 passed（含 4 个平移测试覆盖核心路径）；逐字对齐错误文案/边界/错误码/哈希口径
- ⚠️ 未做：浏览器模式行为对照（本波无前端改动）

### 4. 提交清单

| Commit | 说明 |
|---|---|
| `3581a0a8` | feat(tauri): G4-6a 移植 generation 源绑定逻辑层（2 files +3797） |

---

## 交接给下次会话（**从这里接**）

### 1. 当前工作区状态

- `git status --porcelain`：空（干净）
- HEAD：`3581a0a8 feat(tauri): G4-6a 移植 generation 源绑定逻辑层（generation-source-binding.ts 615 行基线）`
- ⚠️ 上一份快照中已过期的描述：「G4-6 owner 写面 + commands/db.rs:622-658 三分支接线」已拆分——G4-6a 逻辑层已收口（本快照）；G4-6b 写面 9 命令 + 2 事件为下一派；db.rs:622-658 三分支接线随写面波次收口

### 2. 下一步（1-2-3）

1. **G4-6b owner 写面 9 命令 + 2 事件**（下一派）：commit-draft / begin-batch / begin / execute / cancel / pause / resume / restart / discard-candidate + generation:snapshot / generation:reasoning 事件；create 批次 agent「G4-6b-worker」→ 派发 → 收口后 delete（G4-6a 逻辑层已就位）
2. G4-7 with_agent_child_effect 后置
3. B26 剩余旧桩现代化（workflow-launch-dialogs / AIOutputPanel.failed / AIOutputPanel.recovery——后两者含事件插件 transformCallback 范式）

### 3. 阻塞项与待授权项（**不得删除，须逐条确认后更新**）

| # | 项 | 状态 |
|---|---|---|
| B14 | 真 Windows 自动更新（tauri-plugin-updater） | 待批次 H 后段 |
| B22 | 两段式关窗 dirty 分支 | 待 GUI 实测 |
| B24 | 整屏瞬黑（暂缓） | 暂缓 |
| N1 | import-novel-resume 1/32 浏览器模式既存应用行为（2026-10-12 登记） | 单独立项评估中（修复方向：预览 effect 加 inspectionSessionRef 校验） |
| N2 | 86 个未迁移 invoke 频道迁移批次 | G4-6a 逻辑层已就位（2026-10-13）；G4-6b 写面 9 命令 + 2 事件下一派 |

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
| `pnpm run check:channels` | `tauri-app/` | 契约 invoke 频道 290（事件频道 7）；已注册命令 205 → 覆盖 invoke 频道 204；未迁移 invoke 频道 86 [generation=14 agent-generation=8 cloud-backup=8 legacy-roster=7 character-proposal=6 finalization-generation=5 finalized-character=5 graph-generation=5 project=5 character-avatar=4 dialog=4 editor-inline=4 kb=3 startup=3 db=2 import-generation=2 character-identity=1]；「命令名与契约频道一一对应 ✅」（exit 1 属预期——未迁移 > 0） |
| `cargo test --lib` | `tauri-app/src-tauri/` | test result: ok. 988 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 55.42s |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.12s（0 错误 0 告警） |
| `pnpm typecheck` / `pnpm run lint` | `tauri-app/` | exit 0 / exit 0 |
| `git status --porcelain` | 仓库根 | 空 |
| `git log -1` | 仓库根 | 3581a0a8 feat(tauri): G4-6a 移植 generation 源绑定逻辑层（generation-source-binding.ts 615 行基线） |

---
