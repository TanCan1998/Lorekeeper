# Tauri 迁移进度快照（2026-10-12）

> **用途**：AI/开发者接续 Tauri 迁移工作的入口文档（日期化交接快照）。
> **规则**：命名 `YYYY-MM-DD-tauri-migration-status.md`，一日一新文件，**旧文件冻结后不允许修改**（见 [`docs-fork/agents/pi-development.md`](../agents/pi-development.md) §9）。
> **模板**：[`_TEMPLATE-tauri-migration-status.md`](./_TEMPLATE-tauri-migration-status.md)。
>
> 全部历史快照见 `docs-fork/handoffs/` 目录（按日期命名）。
>
> - channel 级盘点：[`docs-fork/plans/tauri-migration-channel-inventory.md`](../plans/tauri-migration-channel-inventory.md)。

---

## 快照（最后更新：2026-10-12 · 第二十五次）

> **本表只填最新一次自检的实测值。**

| 项 | 值 |
|---|---|
| 仓库 / 分支 | **`TanCan1998/Lorekeeper`**（`EthanYoQ/AI-Novel-Writer` 的 PUBLIC fork）/ `master` |
| 产品身份 | **Lorekeeper（设定司）**；`identifier = com.tancan1998.lorekeeper`；npm `lorekeeper-tauri`；Rust crate `lorekeeper` / lib `lorekeeper_lib` |
| 已注册命令 | **198** |
| 覆盖 invoke 频道 | **197**（契约 invoke 总数 **290**，事件频道 **7**） |
| 未迁移 invoke 频道 | **93**（上游新增频道；`generation=21 agent-generation=8 cloud-backup=8 legacy-roster=7 character-proposal=6 finalization-generation=5 finalized-character=5 graph-generation=5 project=5 character-avatar=4 dialog=4 editor-inline=4 kb=3 startup=3 db=2 import-generation=2 character-identity=1`） |
| orphan | **空** ✅ |
| `cargo test --lib` | **984/984** ✅（981 + G4-3b review-revision 3） |
| `cargo fmt --check` | **干净（0 差异）** ✅ |
| `cargo check --all-targets` | **0 告警** ✅ |
| `pnpm typecheck` / `lint` | **exit 0（tsc 0 错误，含 refine-draft 既存 4 错误修复）** / **exit 0** ✅ |
| 定向 `vitest` | **6/6**（`test/channel-migration-coverage.test.ts`，经 `pnpm run check:channels`）✅ |
| 已完成批次 | A ✅ / B ✅ / C ✅ / D1 ✅ / D2-a ✅ / D2-b ✅ / D2-c ✅ / **E ✅** / **F1 ✅** / **L3 ✅** / **F2 ✅** / **批次 E G1 ✅** / **H1 ✅** / **H2 ✅** / **H3 ✅** / **B12 ✅** / **批次 G 的 G1 ✅** / **批次 G2a ✅** / **批次 G2b ✅（1→2 全部完成）** / **批次 G3a ✅** / **批次 G3b ✅** / **批次 G4 ✅（代码门）** / **B13 ✅** / **H4 ✅** / **上游 master 合并 ✅** / **B26 测试桩现代化批次全收口 ✅**（批 1/2a/2b/2c/3/4/5/6 + 第 4 派 + import-novel-resume + refine-draft 修复） |
| 当前阶段 | **G4-3b 收口 + B26 批次收口 + CI 修复完成（2026-10-12）**：review-revision:* 4 频道命令注册（已迁移 193→197、未迁移 97→93）；B26 全部旧桩现代化收口；rebase reword 5 个超宽提交修复 CI commit-message 红（force-with-lease 推送 `da46ce0c`）。待办：G4-5/6/7、B26 剩余旧桩（workflow-launch-dialogs / AIOutputPanel×2 等）、import-novel-resume 1/32 既存应用行为单独立项 |
| 依赖 | 同上一轮（本轮无新增：G4-3b 无 Cargo.toml 改动；B26 为纯测试文件改写） |
| GUI 冒烟 | ✅ 十三轮（2026-10-10）⏸ 第十四轮（G4 参考链路）待暂停 |
| 双栈隔离 | **L0/L1/L2/L3 全部隔离**：安装标识 / `~/.lorekeeper` / `<root>/.lore/`（库 `.lore/lorekeeper.db`、向量 `.lore/kb/`）。基线为 `~/.vela` / `<root>/.vela/`。两栈项目目录刻意不互通（`ee40aaab`） |
| Rust 工具链 | rustc/cargo **1.99.0 stable-msvc** @ `D:\Environment\rust\`（本机内嵌设置 `RUSTUP_HOME` / `CARGO_HOME`） |

<sub>*命令与频道差额：198 命令中 1 个为门户 0 耦合（`app_health_check`，不对应任何迁移频道），其余 197 个 invoke 频道一一对应。/sub>

---

## 本次更新（第 25 次：G4-3b + B26 批次收口 + CI 修复）

### 1. 新增（本轮）

| 项 | 内容 |
|---|---|
| 仓储 / 服务 | 服务层 `review_revision_context.rs`（审稿修稿上下文装载）+ `review_revision_generation.rs`（prepare / commitReview / commitRevision / readRecovery 四步生成编排） |
| 命令 | `commands/review_revision.rs` 新建：`review-revision:prepare` / `commit-review` / `commit-revision` / `read-recovery` 4 频道注册（`lib.rs` invoke_handler + `services/mod.rs` + `commands/mod.rs` 接线） |
| Schema | 无 Schema 变更（4 频道复用既有表） |

**实现要点**：
- 服务层平移自 Electron `electron/services/`，逐字对齐基线错误文案 / 边界条件 / 错误码
- 审稿修稿定稿不可逆约束保持：定稿绑定草稿身份 + 收据 + 章节号 + 正文哈希
- B26 测试桩现代化统一走 `installTauriInternals({ commands })`（`tauri-app/test/helpers/tauri-internals.ts`）：频道名→命令名（`:`/`-`→`_`）、位置参数→命名参数（按 `ipc-client.ts` 的 `CHANNEL_ARG_NAMES`）、事件桩经 `plugin:event|listen` / `transformCallback` 接管

**⚠️ 诚实化占位 / 降级项（如有，必须显式列出）**：

| 项 | 阻塞于 | 表现形式 |
|---|---|---|
| 未迁移频道测试失败 | 对应频道迁移批次 | generation:* / editor-inline:* / finalization-generation:* 等前置拒绝，登记既存不修（迁移后转绿） |
| import-novel-resume 1/32 | 浏览器模式暴露的既存应用行为 | ImportNovelDialog 预览 effect 跨项目重放（非桩现代化引入，代码/断言自 ad7a77ea 逐字未变）——单独立项 |
| tauri-globals.d.ts | 删除评估不成立 | electron/controllers/__tests__ 经 `@baseline` 链入编译，依赖其 Window augmentation——保留 |

### 2. 前端登记

- `src/services/ipc-client.ts`：无新增（4 频道契约已在上游合并时登记）
- `src/shared/migrated-channels.ts`：生成物随 `lib.rs` 同提交（G4-3b 提交内）
- `test/`：batch-chapter-completion-mode.browser.tsx 现代化（installTauriInternals 桩 + 49 命令键 + 命名参数）

### 3. 行为对照（Electron ↔ Tauri，如做了才写）

⚠️ 未做（本轮为服务层平移 + 测试桩现代化，无 GUI 行为变化；G4 GUI 冒烟待第十四轮）

### 4. 提交清单

| Commit | 说明 |
|---|---|
| `afa5181` | feat(tauri): G4-3b 审稿修稿服务层与 review-revision 4 频道注册（原 `6b164bc9`，rebase 后新 hash） |
| `d625180` | test(tauri): B26 收尾——现代化 import-novel-resume browser 测试桩（原 `adb9753e`） |
| `f8f4e34` | test(tauri): B26 第 4 派现代化 batch-chapter-completion-mode 浏览器测试桩（原 `4bb4d3b9`） |
| `7ec590b` | fix(tauri): 修复 refine-draft 测试桩 invoke mock 签名遗留的 4 个 typecheck 错误 |
| `da46ce0c` | docs(fork): §10.2 commit-msg 钩子从可选改为推荐（本机已装 2026-10-12） |
| `0389e9c` / `a5ed9e3` / `ddd68bb` / `e7e4d31` / `6d81c0e` | rebase reword 5 个超宽提交（首行 224/200/176/165/179 列 → 105/105/116/97/99 列），CI commit-message 修复 |

---

## 交接给下次会话（**从这里接**）

### 1. 当前工作区状态

- `git status --porcelain`：空（干净）
- HEAD：`da46ce0c` docs(fork): §10.2 commit-msg 钩子从可选改为推荐（本机已装 2026-10-12）
- ⚠️ 上一份快照（2026-10-10）中已过期的描述：「G4-3 review-cycle 仓储层在飞」已收口（`41940a16` + `afa5181`）；「未迁移 invoke 97」已为 **93**；「测试桩现代化批次约 70 文件」已全收口（B26 批次完成）

### 2. 下一步（1-2-3）

1. **G4-5 owner 读算面**（下波派发应用命名新机制：`subagent({action:"create", ...})` 建批次 agent「G4-5-worker」→ 派发 → 收口后 delete；详见 `.pi/tmp/pipeline-state.md` 命名条）
2. **B26 剩余旧桩现代化**：workflow-launch-dialogs / AIOutputPanel.failed / AIOutputPanel.recovery（后两者含事件插件 transformCallback 范式，参照 `src/components/layout/__tests__/TitleBar-exit.browser.tsx`）
3. **import-novel-resume 1/32 失败修复单独立项评估**（预览 effect 加 inspectionSessionRef 校验；含是否回馈上游）

### 3. 阻塞项与待授权项（**不得删除，须逐条确认后更新**）

| # | 项 | 状态 |
|---|---|---|
| B14 | 真 Windows 自动更新（tauri-plugin-updater） | 待批次 H 后段 |
| B22 | 两段式关窗 dirty 分支 | 待 GUI 实测 |
| B24 | 整屏瞬黑（暂缓） | 暂缓 |
| N1 | import-novel-resume 1/32 浏览器模式既存应用行为（2026-10-12 新登记） | 单独立项评估中（修复方向：预览 effect 加 inspectionSessionRef 校验） |
| N2 | 93 个未迁移 invoke 频道迁移批次 | 待 G4-5/6/7 后的下一波编排 |

### 4. 红线提醒（每次接手都要过一遍）

- 🚫 `docs/` 上游镜像永不修改、永不新增；fork 专属文档只写 `docs-fork/`
- 🚫 子代理禁 `git add`/`git commit`（编排者收口）；编排者独立复验后方可提交
- 🚫 双栈隔离不可回退：Tauri 侧禁读 `AI_NOVEL_VELA_HOME`、禁回退 `~/.vela`、禁使用 `.vela/` 项目目录
- ⚠️ 提交消息首行显示宽度 ≤120 列（CJK 计 2）——CI 已强制（本机 `.git/hooks/commit-msg` 钩子亦装）；rebase 改写已推送历史须 `force-with-lease` + 用户确认
- ⚠️ 未迁移频道测试失败属既存登记不修（对应频道迁移批次后转绿）
- ⚠️ 子代理命名新规（2026-10-12）：派发前 `subagent({action:"create", ...})` 动态注册批次 agent（name=批次名-角色），收口后 `action:"delete"` 清理

---

### 5. 自检记录（2026-10-12 实测）

> **生成本表快照时在本机实跑。下次更新快照表必须先重跑这些命令。**

| 命令 | 工作目录 | 实测输出 |
|---|---|---|
| `pnpm run check:channels` | `tauri-app/` | exit 0；契约 invoke 290（事件 7）· 已注册命令 198 → 覆盖 197 · 未迁移 93 `[generation=21 agent-generation=8 cloud-backup=8 legacy-roster=7 character-proposal=6 finalization-generation=5 finalized-character=5 graph-generation=5 project=5 character-avatar=4 dialog=4 editor-inline=4 kb=3 startup=3 db=2 import-generation=2 character-identity=1]`；命令名与契约频道一一对应 ✅ |
| `cargo test --lib` | `tauri-app/src-tauri/` | test result: ok. **984 passed; 0 failed**; 0 ignored（56.25s） |
| `cargo check --all-targets` | `tauri-app/src-tauri/` | exit 0（0 告警，11.67s 增量） |
| `pnpm typecheck` / `pnpm run lint` | `tauri-app/` | exit 0 / exit 0（eslint 0 告警） |
| `git status --porcelain` | 仓库根 | 空 |
| `git log -1` | 仓库根 | `da46ce0c` docs(fork): §10.2 commit-msg 钩子从可选改为推荐（本机已装 2026-10-12） |
