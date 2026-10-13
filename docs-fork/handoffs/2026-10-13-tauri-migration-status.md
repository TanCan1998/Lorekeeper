# Tauri 迁移进度快照（2026-10-13）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档（日期化交接快照）。
> **规则**：命名 `YYYY-MM-DD-tauri-migration-status.md`，一日一新文件，**旧文件冻结后不允许修改**（见 [`docs-fork/agents/pi-development.md`](../agents/pi-development.md) §9）。
> **模板**：[`_TEMPLATE-tauri-migration-status.md`](./_TEMPLATE-tauri-migration-status.md)。
>
> 全部历史快照见 `docs/handoffs/` 目录（按日期命名）。
>
> - channel 级盘点：[`docs-fork/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。

---

## 快照（最后更新：2026-10-13 · 第 31 次）

> **本表只填最新一次自检的实测值。**

| 项 | 值 |
|---|---|
| 仓库 / 分支 | `F:\workplace\pi\ai-novel-writer` / `master`（与 origin/master 同步） |
| 产品身份 | Lorekeeper（设定司）/ `com.tancan1998.lorekeeper` |
| 已注册命令 | 220 |
| 覆盖 invoke 频道 | 219 |
| 未迁移 invoke 频道 | 71 [agent-generation=8 cloud-backup=8 legacy-roster=7 character-proposal=6 finalization-generation=5 finalized-character=5 graph-generation=5 project=5 character-avatar=4 dialog=4 editor-inline=4 kb=3 startup=3 import-generation=2 character-identity=1 db=1] |
| orphan | 无 |
| `cargo test --lib` | 990 passed / 0 failed（989 既有 + 1 新增平移测试） |
| `cargo check --all-targets` | 0 错误 0 告警 |
| `pnpm typecheck` / `lint` | exit 0 / exit 0 |
| 定向 `vitest` | 未跑（本波无前端测试改动） |
| 已完成批次 | G4-1 / G4-2 / G4-3a / G4-3b / G4-5 / G4-6a / G4-6b-1 / G4-6b-2 / G4-6b-3 / G4-6b-4 / G4-6c-1 / G4-6c-2 / **G4-7**（2026-10-13 新增） |
| 当前阶段 | G4-7 owner 效应面——**with_agent_child_effect 结构接线收口**（143679f0：主体 fail-closed 占位 + db:project-core-commit-generated 频道迁移 + blueprint 新提交路径包装）；agent-generation 域（主体活化 + 执行核心）待迁移 |
| 依赖 | 无新增（rusqlite 0.40.2 [bundled] / reqwest 0.13 / hnsw_rs / jieba 等不变） |
| GUI 冒烟 | 未做 |
| 双栈隔离 | 维持（Tauri 用 `AI_NOVEL_LOREKEEPER_HOME` / `~/.lorekeeper`；项目库 `<root>/.lore/lorekeeper.db`） |
| Rust 工具链 | rustc/cargo 1.99.0 stable-msvc（`D:\Environment\rust`） |

---

## 本次更新（第 31 次：G4-7 —— with_agent_child_effect 结构接线 + db:project-core-commit-generated 频道迁移）

> 第 26 次（G4-6a）至第 30 次（G4-6c-2）详见 Git 历史（f4e06276 等）与提交消息；本段为第 31 次（同日改写）。

### 1. 新增（本轮）

| 项 | 内容 |
|---|---|
| 仓储 / 服务 | main_generation_owner.rs（+29——with_agent_child_effect 方法） |
| 命令 | db_project_core_commit_generated（commands/db.rs +183；ProjectCoreCommitGeneratedRequest 结构 + 命令 + inner + 新测试） |
| Schema | 无变更 |

**实现要点**：
- **占位设计（范围收窄）**：基线 `agents.withChildEffect`（agent-generation.ts:344-376）主体依赖 agent-generation 状态机（runs registry / workflowRegistration / sessionRuns navigation / host.capture / generation_attempts usage_receipt_json 回执写入），属未迁移域——本波 `with_agent_child_effect` 为结构接线：主体 fail-closed（`GENERATION_AGENT_CHILD_EFFECT_UNMIGRATED`，迁移占位码、非基线码），基线 `requireRun(handle)` 身份校验随主体一并待活化；结构就位后随 agent-generation 域波次活化
- **事务门禁的调用方结构保证**：基线 withChildEffect 首行 `if (!this.db.inTransaction) throw GENERATION_AGENT_TRANSACTION_REQUIRED`（agent-generation.ts:345）的 Tauri 等价物——rusqlite 0.40.2 无 `is_in_transaction()` API（`is_autocommit()` 对「效应自开事务」路径为误报）；db:project-core-commit-generated 路径在外层事务内调用（`unchecked_transaction` + commit / 事务 Drop 即回滚）、blueprint-commit-range 路径的效应 `commit_range` 自开事务、提交原子性自足；方法内不重复检查（方法头注文档化）
- db:project-core-commit-generated 逐行平移基线 db-controller.ts:178-193：guard_read 门禁 → generationRunHandle 缺失（Rust 以 `Option` 承载基线运行时 `!request.generationRunHandle` 检查）→ `GENERATION_OWNER_REQUIRED` → authorized_owner 门禁（锁序：owner 缓存锁→项目库锁，与 blueprint 路径同序）→ with_project_db 内 unchecked_transaction 事务中 with_agent_child_effect 包装 assert_sources_current + project_core::update
- 错误信封前缀约定：authorized_owner / with_agent_child_effect 透传错误已是 MUTATING 信封形态（带 `"Error: "` 前缀）直接透传；project_core::update 等仓储裸错误经 mutating_error 包装——与 db.rs 既有命令一致
- blueprint-commit-range 新提交路径（基线 :572-573）：`None` 分支由直提拒绝改为 `owner.with_agent_child_effect(conn, &handle_value, || commit_range(...)).and_then(|outcome| outcome)` 包装（拍平双层 Result）——错误码不变、禁止静默走直提路径；幂等 `Some(_)` 分支不动
- **blueprint_commit_range_result 前缀感知修复**：已带 `"Error: "` 前缀的错误（with_agent_child_effect 透传）原样透传，避免 `"Error: Error: "` 双重前缀（复验实测捕获：blueprint_commit_range_generation_handle_branches_test op-fresh 断言 left=`Some("Error: Error: …")`）
- 新增 project_core_commit_generated_validation_test（无 handle → `GENERATION_OWNER_REQUIRED`；有 handle → `GENERATION_AGENT_CHILD_EFFECT_UNMIGRATED`）；cargo test --lib 989→990

**⚠️ 诚实化占位 / 降级项（如有，必须显式列出）**：

| 项 | 阻塞于 | 表现形式 |
|---|---|---|
| with_agent_child_effect 主体 | agent-generation 状态机（runs registry / workflowRegistration / sessionRuns / host.capture / usage 回执写入）未迁移 | db:project-core-commit-generated 有 handle 的请求收 `GENERATION_AGENT_CHILD_EFFECT_UNMIGRATED`（占位码）；blueprint-commit-range 新提交路径同；随 agent-generation 域波次活化 |
| requireRun 身份校验 | 随 with_agent_child_effect 主体待活化 | 占位主体在入口直接 fail-closed，句柄身份校验不执行（活化后恢复） |

### 2. 前端登记

- `src/services/ipc-client.ts`：CHANNEL_ARG_NAMES 追加 1 频道（db:project-core-commit-generated，+2 行含注释计数 4→5 频道）
- `src/shared/migrated-channels.ts`：`check:channels --emit` 再生成（+1 行）
- `test/`：无变化

### 3. 行为对照（Electron ↔ Tauri，如做了才写）

- 命令结构逐行平移基线 db-controller.ts:178-193（门禁顺序 / 错误码 / 事务结构 / owner 锁序）
- blueprint 新提交路径包装后错误码与基线占位断言一致（`GENERATION_AGENT_CHILD_EFFECT_UNMIGRATED` 单前缀形态，op-fresh 断言不回归）
- ⚠️ 执行语义未对照（fail-closed 占位——依赖未迁移，见上表）

### 4. 提交清单

| Commit | 说明 |
|---|---|
| `143679f0` | feat(tauri): G4-7 with_agent_child_effect 结构接线 + db:project-core-commit-generated 频道迁移（5 files +213/−4，含 ipc-client 登记；由 778c56f7 amend 补登记） |

---

## 交接给下次会话（**从这里接**）

### 1. 当前工作区状态

- `git status --porcelain`：仅本快照文档在途更新（docs 提交前）
- HEAD：`143679f0 feat(tauri): G4-7 with_agent_child_effect 结构接线 + db:project-core-commit-generated 频道迁移`
- **G4-7 收口**：with_agent_child_effect 结构接线（主体 fail-closed 占位）+ db:project-core-commit-generated 频道迁移 + blueprint 新提交路径包装 + 前缀感知修复；频道计数 220 命令 / 219 覆盖 / 71 未迁移（db=1）；agent-generation 域（with_agent_child_effect 主体活化 + 执行核心）待迁移

### 2. 下一步（1-2-3）

1. **agent-generation 域迁移波次**：活化 with_agent_child_effect 主体（基线 agent-generation.ts:344-376——runs registry / workflowRegistration / sessionRuns navigation / host.capture / generation_attempts usage_receipt_json 回执写入；agent-generation 服务层整体）——本波结构就位，主体 fail-closed 占位待活化
2. 上游合并波次（上游未合并 109 提交：electron 基线同步 main-generation-owner +133 / generation-run-repository +178；src/ 重设计 162 文件 +9139；docs/ 镜像重同步 169 触次；G4-5 已迁移命令契约对齐 generation:list-directory-progress return → DirectoryGenerationCatalog）
3. 其余 71 个未迁移 invoke 频道（分前缀明细见顶部表格；db 域仅剩 db=1 即本波已清零外的残余——实际 db=1 为本波迁移后余额，见 check:channels 输出）

### 3. 阻塞项与待授权项（**不得删除，须逐条确认后更新**）

| # | 项 | 状态 |
|---|---|---|
| B14 | 真 Windows 自动更新（tauri-plugin-updater） | 待批次 H 后段 |
| B22 | 两段式关窗 dirty 分支 | 待 GUI 实测 |
| B24 | 整屏瞬黑（暂缓） | 暂缓 |
| N1 | import-novel-resume 1/32 浏览器模式既存应用行为（2026-10-12 登记） | 单独立项评估中（修复方向：预览 effect 加 inspectionSessionRef 校验） |
| N2 | 71 个未迁移 invoke 频道迁移批次 | G4-7 收口（2026-10-13，220 命令 / 219 覆盖 / 71 未迁移——db=1）；generation 域频道清零（generation=0） |

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
| `cargo test --lib` | `tauri-app/src-tauri/` | test result: ok. 990 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 57.04s（989 既有 + 新增 project_core_commit_generated_validation_test） |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | Finished `dev` profile [unoptimized + debuginfo] target(s) in 34.38s（0 错误 0 告警） |
| `pnpm typecheck` / `pnpm run lint` | `tauri-app/` | exit 0 / exit 0 |
| `git status --porcelain` | 仓库根 | 仅 docs-fork/handoffs/2026-10-13-tauri-migration-status.md 在途（本快照） |
| `git log -1` | 仓库根 | 143679f0 feat(tauri): G4-7 with_agent_child_effect 结构接线 + db:project-core-commit-generated 频道迁移 |

---
