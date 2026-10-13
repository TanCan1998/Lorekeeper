# Tauri 迁移进度快照（2026-10-13）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档（日期化交接快照）。
> **规则**：命名 `YYYY-MM-DD-tauri-migration-status.md`，一日一新文件，**旧文件冻结后不允许修改**（见 [`docs-fork/agents/pi-development.md`](../agents/pi-development.md) §9）。
> **模板**：[`_TEMPLATE-tauri-migration-status.md`](./_TEMPLATE-tauri-migration-status.md)。
>
> 全部历史快照见 `docs/handoffs/` 目录（按日期命名）。
>
> - channel 级盘点：[`docs-fork/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。

---

## 快照（最后更新：2026-10-13 · 第 32 次）

> **本表只填最新一次自检的实测值。**

| 项 | 值 |
|---|---|
| 仓库 / 分支 | `F:\workplace\pi\ai-novel-writer` / `master`（与 origin/master 同步） |
| 产品身份 | Lorekeeper（设定司）/ `com.tancan1998.lorekeeper` |
| 已注册命令 | 220 |
| 覆盖 invoke 频道 | 219 |
| 未迁移 invoke 频道 | 71 [agent-generation=8 cloud-backup=8 legacy-roster=7 character-proposal=6 finalization-generation=5 finalized-character=5 graph-generation=5 project=5 character-avatar=4 dialog=4 editor-inline=4 kb=3 startup=3 import-generation=2 character-identity=1 db=1] |
| orphan | 无 |
| `cargo test --lib` | 990 passed / 0 failed（与第 31 次一致——G4-8a 服务层零新增测试，诚实记录见下） |
| `cargo check --all-targets` | 0 错误 0 告警 |
| `pnpm typecheck` / `lint` | exit 0 / exit 0 |
| 定向 `vitest` | 未跑（本波无前端测试改动） |
| 已完成批次 | G4-1 / G4-2 / G4-3a / G4-3b / G4-5 / G4-6a / G4-6b-1 / G4-6b-2 / G4-6b-3 / G4-6b-4 / G4-6c-1 / G4-6c-2 / G4-7 / **G4-8a**（2026-10-13 新增） |
| 当前阶段 | G4-8a legacy-roster **服务层收口**（d752b414：validate_legacy_roster_candidate + readLegacyRosterGenerationProof 真实化 + LegacyRosterGeneration 服务类）；legacy-roster 命令层（7 频道）待迁移（G4-8b） |
| 依赖 | 无新增（rusqlite 0.40.2 [bundled] / reqwest 0.13 / hnsw_rs / jieba 等不变） |
| GUI 冒烟 | 未做 |
| 双栈隔离 | 维持（Tauri 用 `AI_NOVEL_LOREKEEPER_HOME` / `~/.lorekeeper`；项目库 `<root>/.lore/lorekeeper.db`） |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc（`D:\Environment\rust`） |

---

## 本次更新（第 31 次：G4-7 —— with_agent_child_effect 结构接线 + db:project-core-commit-generated 频道迁移）

> 第 26 次（G4-6a）至第 31 次（G4-7）详见 Git 历史（f4e06276 / 143679f0 等）与提交消息；本段为第 32 次（同日改写）。

### 1. 新增（本轮）

| 项 | 内容 |
|---|---|
| 仓储 | character_roster_repository.rs（+22——`validate_legacy_roster_candidate`，对齐基线 `validateLegacyRosterCandidate`，不重启用已退役的按名写路径） |
| 服务 | character_proposal.rs（+188/−12——`readLegacyRosterGenerationProof` 真实化）；legacy_roster_generation.rs（+310 新文件——LegacyRosterGeneration 服务类 + 共享类型） |
| 命令 | 无（G4-8a 纯服务层，已注册命令数不变） |
| Schema | 无变更 |

**实现要点**：
- **服务层逐行平移**：LegacyRosterGeneration 服务类（legacy_roster_generation.rs）平移基线 legacy-roster-generation.ts 全量方法——`require`（句柄键集 + projectId / rootActionId / epoch（originEpoch 或 binding epoch）身份校验 → `GENERATION_RUN_IDENTITY_MISMATCH`）/ `task` / `artifact_for`（最近 attempt 产物引用 + proof 校验失败回退 None）/ `saved`（`generation_attempts.usage_receipt_json` 的 `legacyRosterProposal` 读回 + 完整性校验 → `GENERATION_LEGACY_EFFECT_INVALID`）/ `proposal` / `assert_mutable`（`GENERATION_LEGACY_EFFECT_SEALED`）/ `stage`（`unchecked_transaction` 事务：幂等重放（artifact 一致直接读回批次）→ `GENERATION_LEGACY_ARTIFACT_INVALID` 校验 → proof → assertSources → `proposals.stage` → 效应写回 usage_receipt_json）
- **proof 真实化**：`readLegacyRosterGenerationProof`（character_proposal.rs）从待移植桩转为真实实现——依赖本波落地的 `validate_legacy_roster_candidate` 与既有 `read_legacy_roster_generation_context`；键集约束先行校验，保持上游失败语义
- **共享类型**：`LegacyRosterGenerationContext` / `LegacyRosterGenerationArtifact` / `LegacyRosterSourceStatus`（`'current' | 'conflict'`）/ `LegacyRosterGenerationRecovery`——其中 `view` / `proposal` 两项的上游类型（`MainGenerationRunView` / `CharacterProposalBatch`）尚未移植为 Rust 类型，以 `serde_json::Value` 占位（模块头注文档化，待后续波次接入）
- **就地辅助**：`text_hash`（SHA-256 hex，对齐基线 `textHash`；G4-6c 就地实现先例——generation_run_repository 的同名函数为模块私有）/ `keys_subset`（对齐基线 `Object.keys(source).some(key => !allowed.includes(key))` 失败语义）

**⚠️ 诚实化占位 / 降级项（如有，必须显式列出）**：

| 项 | 阻塞于 | 表现形式 |
|---|---|---|
| G4-8a 零新增测试 | 服务类依赖 `CharacterProposalService`（Tauri 侧从未构造——prove 闭包需在命令层接线），`LegacyRosterGeneration` 从未被调用 | `cargo test --lib` 990 passed 与第 31 次一致；服务类与 `validate_legacy_roster_candidate` 的测试随 G4-8b 命令层迁移补（命令层是首个真实调用点） |
| legacy-roster 命令层（7 频道） | G4-8a 仅服务层 | 7 频道未迁移（legacy-roster=7）；read-source / adopt-existing / read / stage / cancel 依赖已就位（G4-8b 可迁移）；begin 依赖创建核心（B3：newMainGenerationPolicy / generation-run-service 未迁移——Tauri `owner.begin` 对 `legacy-character-roster-repair` 硬拒绝 `GENERATION_LEGACY_ADMISSION_REQUIRED`，对齐基线 IPC 边界行为）；execute 依赖 LLM 双阶段执行循环 |

### 2. 前端登记

- `src/services/ipc-client.ts`：无变化（G4-8a 纯服务层，无新频道）
- `src/shared/migrated-channels.ts`：无变化
- `test/`：无变化

### 3. 行为对照（Electron ↔ Tauri，如做了才写）

- 服务类逐行平移基线 legacy-roster-generation.ts（方法集 / 错误码 / 事务结构 / 效应完整性校验链）
- proof 真实化后错误码与基线 validateLegacyRosterCandidate 失败语义一致
- ⚠️ 执行语义未对照（服务层无调用点——命令层 G4-8b 迁移后生效）

### 4. 提交清单

| Commit | 说明 |
|---|---|
| `d752b414` | feat(tauri): G4-8a legacy-roster 服务层——proof 真实化 + LegacyRosterGeneration 服务类（3 files +508/−12） |

---

## 交接给下次会话（**从这里接**）

### 1. 当前工作区状态

- `git status --porcelain`：仅本快照文档在途更新（docs 提交前）
- HEAD：`d752b414 feat(tauri): G4-8a legacy-roster 服务层——proof 真实化 + LegacyRosterGeneration 服务类`
- **G4-8a 收口**：legacy-roster 服务层完整落地（validate_legacy_roster_candidate + readLegacyRosterGenerationProof 真实化 + LegacyRosterGeneration 服务类）；频道计数 220 命令 / 219 覆盖 / 71 未迁移（legacy-roster=7）；legacy-roster 命令层（7 频道）待迁移

### 2. 下一步（1-2-3）

1. **G4-8b legacy-roster 命令层迁移**（优先——服务层已就位）：read-source / adopt-existing / read / stage / cancel 5 频道真实化（依赖：read_legacy_roster_source / adopt_legacy_cards / LegacyRosterGeneration::require+stage / repository.pause(cancel=true) / view_of 均已就位）；begin / execute 2 频道 fail-closed 占位（依赖创建核心 B3 / LLM 双阶段循环，未迁移）；命令层是 LegacyRosterGeneration 首个调用点，随波补服务类测试
2. agent-generation 域迁移波次：活化 with_agent_child_effect 主体（基线 agent-generation.ts:344-376——runs registry / workflowRegistration / sessionRuns navigation / host.capture / generation_attempts usage_receipt_json 回执写入；agent-generation 服务层整体）——G4-7 结构就位，主体 fail-closed 占位待活化
3. 上游合并波次（上游未合并 109 提交：electron 基线同步 main-generation-owner +133 / generation-run-repository +178；src/ 重设计 162 文件 +9139；docs/ 镜像重同步 169 触次；G4-5 已迁移命令契约对齐 generation:list-directory-progress return → DirectoryGenerationCatalog）；其余 71 个未迁移 invoke 频道（分前缀明细见顶部表格）

### 3. 阻塞项与待授权项（**不得删除，须逐条确认后更新**）

| # | 项 | 状态 |
|---|---|---|
| B14 | 真 Windows 自动更新（tauri-plugin-updater） | 待批次 H 后段 |
| B22 | 两段式关窗 dirty 分支 | 待 GUI 实测 |
| B24 | 整屏瞬黑（暂缓） | 暂缓 |
| N1 | import-novel-resume 1/32 浏览器模式既存应用行为（2026-10-12 登记） | 单独立项评估中（修复方向：预览 effect 加 inspectionSessionRef 校验） |
| N2 | 71 个未迁移 invoke 频道迁移批次 | G4-8a 收口（2026-10-13，220 命令 / 219 覆盖 / 71 未迁移——legacy-roster=7）；generation 域频道清零（generation=0） |

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
| `pnpm run check:channels` | `tauri-app/` | 契约 invoke 频道 290（事件频道 7）；已注册命令 220 → 覆盖 invoke 频道 219；未迁移 invoke 频道 71 [agent-generation=8 cloud-backup=8 legacy-roster=7 character-proposal=6 finalization-generation=5 finalized-character=5 graph-generation=5 project=5 character-avatar=4 dialog=4 editor-inline=4 kb=3 startup=3 import-generation=2 character-identity=1 db=1]；「命令名与契约频道一一对应 ✅」（exit 1 属预期——未迁移 > 0） |
| `cargo test --lib` | `tauri-app/src-tauri/` | test result: ok. 990 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 56.55s（与第 31 次一致——G4-8a 服务层零新增测试，见诚实化占位表） |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | Finished `dev` profile [unoptimized + debuginfo] target(s) in 34.38s（0 错误 0 告警） |
| `pnpm typecheck` / `pnpm run lint` | `tauri-app/` | exit 0 / exit 0 |
| `git status --porcelain` | 仓库根 | 仅 docs-fork/handoffs/2026-10-13-tauri-migration-status.md 在途（本快照） |
| `git log -1` | 仓库根 | d752b414 feat(tauri): G4-8a legacy-roster 服务层——proof 真实化 + LegacyRosterGeneration 服务类 |

---
