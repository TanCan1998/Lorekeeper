# 项目开发参考（按任务查阅）

本文件收纳原项目说明中的环境、命令、目录、迁移映射和 Git 细则。只读取当前任务相关章节；常驻约束见根 AGENTS.md。版本号、迁移状态和已知问题是整理时的历史记录，执行前以源码、频道清单和最新交接快照为准。

## 本机环境与网络规则（Agent 必读）

1. **GitHub 下载加速**：任何涉及 github.com 的下载操作，在原链接前加 `https://gh-proxy.com/` 前缀（如 `https://gh-proxy.com/https://github.com/...`）；链接已有此前缀则不重复添加。

2. **npm/pnpm 会话级镜像**（不落盘、不改项目 `.npmrc`，它是上游同步文件）：
   - **一键脚本**：`. D:\Environment\npm-mirrors.ps1`（含 registry + Electron 镜像 + 跳过 Electron 二进制 + better-sqlite3 备用镜像；已挂到本机 `$PROFILE`，新终端自动生效；**不改 `.npmrc`**）。`.npmrc` 经确认**未被忽略且被 git 跟踪**（上游同步文件，合并上游会冲突），故一律走会话级变量。
   - registry：`$env:npm_config_registry = 'https://registry.npmmirror.com'`
   - Electron 二进制镜像（下载时提速）：`$env:ELECTRON_MIRROR = 'https://npmmirror.com/mirrors/electron/'`
   - **跳过 Electron 二进制下载**（迁移期间不跑 Electron，install 提速 ~100MB+）：`$env:ELECTRON_SKIP_BINARY_DOWNLOAD = '1'`；将来要运行 Electron 基线时，去掉该变量执行 `pnpm rebuild electron` 恢复二进制即可，零文件改动。
   - better-sqlite3 prebuild 镜像（重装遇 prebuild 时备用）：`$env:npm_config_better_sqlite3_binary_host_mirror = 'https://registry.npmmirror.com/-/binary/better-sqlite3'`

3. **开发环境安装位置**：安装新的开发环境工具链（Rust/rustup、cargo、JDK、Go 等）时，统一安装到 `D:\Environment\` 下新建专用目录（如 `D:\Environment\rust`），不装到默认用户目录，保持工具链集中管理。示例（rustup）：先设 `RUSTUP_HOME=D:\Environment\rust\rustup`、`CARGO_HOME=D:\Environment\rust\cargo`（并加入 PATH）再运行 rustup-init。

4. **环境事实**：
   - pnpm 已升到 **11.11.0**（与 packageManager 一致；2026-10-06 升级）；node v26.10.0（较新，原生模块如 better-sqlite3 若重装遇 prebuild 缺失会回退 node-gyp，本机无 VS 构建链，届时需切 node LTS 或装构建工具）。
   - 本机 Rust 工具链已装（2026-10-06）：**rustc/cargo 1.99.0 stable-msvc**，装于 `D:\Environment\rust\{rustup,cargo}`（遵循第 3 条规则）；crates.io 走 rsproxy 镜像（`D:\Environment\rust\cargo\config.toml`）。新终端自动生效；工具调用脚本内需显式设 `RUSTUP_HOME` / `CARGO_HOME`。

5. **根 `pnpm-lock.yaml` 必须保持 git 干净状态，禁止删除。**

## 技术栈（当前基线 Electron → 目标 Tauri）

| 层 | 现值（Electron） | 目标（Tauri 2） |
|----|------------------|------------------|
| 运行/壳 | Electron 41 + Node ≥20 | Tauri 2 + Rust（crate `lorekeeper` / lib `lorekeeper_lib`；`tauri`、`tauri-build`） |
| 前端渲染 | React 19 + TS 6 + Vite 8 + Tailwind 4 | **复用**（不变），Vite 仍是前端构建 |
| 状态/编辑器 | Zustand + Monaco/Codemirror | **复用**（不变，纯前端） |
| 持久化 | better-sqlite3（Node 原生模块） | `rusqlite 0.40.2`（[bundled]，已定案，**非 sqlx**） |
| 向量库 | @lancedb/lancedb（Node 原生，退化为 LIKE） | **方案 B：自研混合检索**（FTS5 + HNSW `hnsw_rs` + `jieba` 分词 + RRF 融合）。**否决 LanceDB Rust**（+1680 依赖 / datafusion 增内存，与「减内存」冲突）。向量层已落地（`db/vector.rs` LocalVectorIndex），kb 命令层待 F2 收口 |
| LLM HTTP | Node `fetch`/`undici` + 手写 SSE | `reqwest 0.13`（`default-features=false` + `native-tls` + `socks`；**刻意不用 rustls**，避免 aws-lc-rs 构建链） |
| IPC | Electron IPC（ipcMain/ipcRenderer） | Tauri `#[command]` + `@tauri-apps/api/core` 的 `invoke` |
| 壳能力 | BrowserWindow/menu/notify/updater/dialog | Tauri window/menu/notification + `tauri-plugin-dialog 2`（已接入）；updater 待批次 H |

- 包管理器 `pnpm@11.11.0`；迁移期两棵独立依赖树：仓库根（Electron 基线）与 `tauri-app/`（Tauri 前端），各自 install，**两处都只用 pnpm**，勿混用 npm。
- Rust 侧代码在 `tauri-app/src-tauri/`（Tauri 约定目录）。

## 命令

```bash
# —— 当前 Electron 基线 ——
pnpm install                 # 安装依赖
pnpm dev                     # Vite 开发（渲染进程）
pnpm typecheck               # tsc --noEmit（改 TS 必过）
pnpm test                    # vitest run
pnpm run lint                # eslint 零告警
pnpm run check:i18n          # i18n 覆盖校验

# —— Tauri 迁移目标（在 tauri-app/ 下执行）——
pnpm typecheck               # tsc --noEmit（改 tauri-app TS 必过）
pnpm test                    # vitest run（⚠️ 全量暂超时，见进度文档遗留项）
pnpm run lint                # eslint 零告警
pnpm build                   # tsc && vite build → tauri-app/dist
cargo build                  # 在 tauri-app/src-tauri/ 下（Rust 工具链已装：见环境规则第 3/4 条，脚本内须显式设 RUSTUP_HOME/CARGO_HOME）
cargo test <模块或过滤词>      # 日常 Rust 相关回归；全量 cargo test 仅阶段/累计/风险触发，见开发指南 §7
pnpm tauri dev               # 本地开发（Vite 5190 + Rust 联动）
pnpm tauri build             # 打包
```

改 TS 代码最小自检顺序：`pnpm typecheck` → `pnpm test`（相关）→ `pnpm run lint`。改 Rust 默认相关 `cargo test <模块或过滤词>` + 定向 rustfmt + diff 检查；需要时扩大到 `cargo check --all-targets`。全量在领域阶段收口、累计 5 个 Rust 代码切片、最终验收、异常跨域回归或关键安全规则变更时执行，详见 [开发指南 §7](pi-development.md#7-测试约定)。纯文档不重跑代码测试。

> **⚠️ 本地已知既存失败（2026-10-08 决定：维持现状，不改上游测试）**
> 根目录 `pnpm test` 全量跑时，`scripts/__tests__/public-repository-hygiene.test.ts` 必失败——它断言 `AGENTS.md` **不存在**，而本机 `AGENTS.md` 是 pi 指引文件、被 `.gitignore:62` 忽略、**不入库**；CI 的 checkout 不含它，故 **CI 不受影响**。判断回归时把这一条排除，勿当成自己引入的失败。
> 另注：`tauri-app/` 的 `pnpm test` 全量暂超时，见进度快照遗留项。

## 目录结构

```text
electron/              当前主进程（Node/Electron）——迁移源，保留不动，用于同步上游
  main.ts, preload.ts  入口
  ipc-handlers.ts      所有 Electron IPC 通道注册点（多 controller 路由）
  controllers/         IPC controller（按模块） → 迁移为 Rust commands
  repositories/        SQLite 访问（better-sqlite3） → 迁移为 rusqlite
  services/ llm/ security/ mcp/ utils/   领域服务 / LLM 适配 / 原生安全 / MCP / 工具
tauri-app/            迁移项目根（独立 pnpm 项目，与基线依赖物理隔离）
  src/                渲染进程（React）——复制自基线 src/（排除 stories），随迁移适配
    services/ipc-client.ts、finalization-client.ts ← 唯二需换底层的调用面
  src-tauri/          Tauri Rust 后端（crate= lorekeeper；productName= Lorekeeper；identifier= com.tancan1998.lorekeeper）
    src/commands/     Rust 命令（config/db/project/llm/llm_execution/llm_generation/llm_management/fs/skin/window/chapter_lifecycle/…）
    src/db/           mod.rs（项目库= <root>/.lore/lorekeeper.db）、schema.rs、vector.rs（F2 向量层）
    src/repositories/ 平移自 electron/repositories/
    src/llm/          chat/discovery/embedding/gemini/lease/openai/params/…
    tauri.conf.json   devUrl 127.0.0.1:5190；decorations:false；窗口 1440x900
    capabilities/default.json   core:default（按迁移批次追加最小权限）
src/                   Electron 基线渲染层（上游同源，不改动）
docs/                 **上游镜像**（永不修改/新增）
  product-domain.md / adr/ (0001-0017) / agents/ / research/ / handoffs/ / plans/ / experiments/ / readme-visuals/ / assets/readme/
docs-fork/            **fork 专属文档**入口（README.md）
  agents/pi-development.md   迁移开发规范
  handoffs/           进度快照（_TEMPLATE + 各日期）——最新 2026-10-10
  plans/              tauri-migration-channel-inventory.md（invoke 频道盘点；契约总数 193，事件频道 4）+ archive/
  research/           迁移调研（LanceDB 否决 / 方案 B 定案 / FTS5 发现等）+ archive/
  adr/                fork 维护的 ADR 索引
  notes-on-upstream-changes.md   上游改动登记
plugins/dsh-ai-novel-writer/   DeepSeek Harness 独立插件（与桌面版无关）
```

## IPC 契约中枢（双份同源结构）

- 基线：`src/shared/ipc-channels.ts`（上游同源）声明每个频道的 `args` / `return`。
- 迁移版：`tauri-app/src/shared/ipc-channels.ts` 为副本；其 electron 侧类型经 `@baseline` 别名引用 `electron/` 单源（仅 `import type`，运行时零依赖），**禁止复制基线类型**。
- 迁移以频道清单为唯一事实源：**每个 channel 对应一个 Rust `#[command]`**（盘点见 `docs-fork/plans/tauri-migration-channel-inventory.md`）。
- 前端调用改为 `@tauri-apps/api/core` 的 `invoke`。
- `tauri-app` 渲染层不得依赖任何 Electron 运行时 API。

## 迁移映射（Electron → Tauri）

| Electron | Tauri 2 |
|----------|---------|
| `ipcMain.handle(ch, handler)` | `#[tauri::command]`，`invoke_handler` 注册 |
| `contextBridge` / preload | `@tauri-apps/api`（前端直接 `invoke`）；白名单 `tauri.conf.json` 权限 |
| `app.getPath('userData')` | `lorekeeper_home()` = `AI_NOVEL_LOREKEEPER_HOME` 或 `~/.lorekeeper`（全局根）；项目库 = `<root>/.lore/lorekeeper.db` |
| browserWindow / menu / notify | Tauri window/menu/notification（`tauri-plugin-notification` 待批次 H） |
| `electron-updater` | `tauri-plugin-updater`（**待批次 H**，当前未接入） |
| `dialog.showOpenDialog` 等 | `tauri-plugin-dialog 2`（已接入；`dialog:select-folder` 已真实化） |
| better-sqlite3 | `rusqlite 0.40.2`（[bundled]，已定案） |
| @lancedb 向量检索（退化为 LIKE） | **自研混合检索**：FTS5 + HNSW `hnsw_rs` + `jieba` 分词 + RRF（**否决 LanceDB Rust**）；向量层已落地 `db/vector.rs`，`kb:*` 命令层待 F2 收口 |
| Node `fetch`/`undici` + 手写 SSE | `reqwest 0.13`（`native-tls` + `socks`，**不用 rustls**） |
| 子进程 / 原生 shell / fs | `tauri-plugin-shell` / `tauri-plugin-fs`（需授权；`tauri-plugin-fs` 已随 dialog 传递引入，**批次 H 勿重复引入**） |

## 代码风格

- 前端沿用：函数组件 + Hooks；严格 TS；状态走 Zustand store。
- Rust 侧：`snake_case` 函数/变量，`PascalCase` 类型；命令名用 kebab-case（如 `chapter_finalize`）；错误用 `Result<T, String>` 或自定义 `Error`，需可序列化回前端。
- 共享类型放 `tauri-app/src/shared/`；基线数据类型经 `@baseline` 引用单源，不得复制；Rust 命令签名与 `ipc-channels.ts` 中的 args/return 一一对应，保持类型一致。

```rust
#[tauri::command]
fn chapter_finalize(chapter_id: String, body: String) -> Result<FinalizedReceipt, String> {
    // TODO: 迁移自 electron/services/finalization-service.ts 逻辑
    ...
}
```

## 关键约束 / 边界

- **领域事实源**：作者事实优先，派生投影只读、不得反向覆盖。见 `docs/product-domain.md`。
- **定稿不可逆**：定稿绑定草稿身份 + 收据 + 章节号 + 正文哈希；有专属生命周期。迁移 Rust 时保持此不可变约束。
- **文件系统安全**：外部文件须显式授权（ADR 0002）；Tauri 侧用 `tauri-plugin-fs` 的能力白名单 + 自研授权校验，勿绕过。
- **提示词合同**：语言 / 结构化输出 / 工具协议不可被覆盖（ADR 0008/0015）。
- **一次只迁一个 controller**：IPC 层按 `ipc-channels.ts` 逐个 channel 平移并保持行为一致，避免大爆炸式重构。
- **双栈隔离不可回退**：Tauri 侧禁止读 `AI_NOVEL_VELA_HOME`、禁止回退 `~/.vela`、禁止使用 `.vela/` 项目目录（Tauri 专用 `<root>/.lore/`，库文件 `.lore/lorekeeper.db`）；Tauri 侧命令失败文案按基线 MUTATING 规则带 `"Error: "` 前缀（`commands/db.rs::mutating_error`）。
- **Shell 输出纪律（本机 pi 的 `powershell` 工具真实后端是 PS 5.1，已实测）**：默认**不写 `2>&1`**（写了会把 stderr 渲染成 7–8 行装饰）；确需合并流时用 `2>&1 | ForEach-Object { "$_" }`（实测 293 B/8 行 → 38 B/2 行）；**禁用** `-notmatch 'CategoryInfo…'` 过滤装饰（实测无效，装饰是渲染期产物）；退出码紧跟命令取，**不要**在需要退出码的命令上用 `| Select-Object -First N`（早关管道会毁掉退出码）。详见 `docs-fork/research/2026-10-10-pi-powershell-backend-discovery.md`。

### 三层边界

- ✅ **Always**：改 `tauri-app` TS 过 typecheck + lint 及相关测试（在 `tauri-app/` 下跑）；改 Rust 跑相关定向测试，全量按开发指南 §7 分层触发，非逐切片必跑；共享类型放 `tauri-app/src/shared/`；遵循领域术语。
- ⚠️ **Ask first**：改数据库 schema/迁移、新增依赖（尤其原生模块或 Rust crate）、改 CI/发布配置、动定稿与外部文件权限、更新 ADR、删除/清理任何 Electron 旧代码（默认保留，它是 `upstream` 同步锚点）。
  - 「改 CI/发布配置」依然适用：Gitleaks CI（`.github/workflows/gitleaks.yml`）已于本次配置（用户授权）并作为例外记录；后续任何改动（调扫描策略、升级 Gitleaks 版本等）仍需 Ask first。
- 🚫 **Never**：提交 API Key/Token/模型凭据/用户数据/小说正文（Gitleaks CI 会扫描泄露，命中即 CI 失败）；编辑 `node_modules/`、`dist*/`、`.release/`、`target/` 产物；提交构建产物。

## Git 工作流

- Issue-first：PR 链接已有 Issue，一个 PR 一个主题（迁移建议按 controller 拆分，如 `✨ feat(tauri): migrate config controller`）。
- 本 fork 通过 `origin` = `TanCan1998/Lorekeeper`、`upstream` = `EthanYoQ/AI-Novel-Writer` 关联；定期 `git fetch upstream` 合并原仓库演进。
- 语义化提交：`<gitmoji> type(scope): 描述`，Gitmoji 与 `type` 一一配对（✨ `feat` 新功能 / 🐛 `fix` Bug 修复 / 📚 `docs` 文档更新 / 💎 `style` 代码格式 / ♻️ `refactor` 重构 / 🧪 `test` 测试 / 🔧 `chore` 构建·工具 / ⚡ `perf` 性能优化 / 📦 `build` 构建产物 / 👷 `ci` CI 配置 / ⏪ `revert` 回退；白名单与 `scripts/check-commit-msg.mjs` 的 `COMMIT_TYPES` / `GITMOJI`、`docs-fork/agents/pi-development.md` §10 同源，改一处必须同步其余两处）；**提交说明用中文**；分支 `type/short-description`。
- **CI / 工作流类改动统一写 `🔧 chore(ci): 描述`**（`ci` 只作 scope，不写裸 `ci:` 类型前缀），与仓库 Gitleaks 系列提交（`chore(ci): 新增 Gitleaks 秘密扫描工作流` 等）保持一致；上游既存的裸 `ci:` 提交不追溯改写。
- **提交消息必须无 BOM、无 CRLF、无行尾空白**（2026-10-08 出过 BOM 事故，详见 `docs-fork/agents/pi-development.md` §10）：用 `git commit -m`，或用 Node `fs.writeFileSync(p, msg, 'utf8')` 写消息文件后 `git commit -F`；**禁止** PowerShell 5.1 的 `Set-Content -Encoding UTF8` / `Out-File`（默认带 BOM）。提交前自检：`node scripts/check-commit-msg.mjs --range <base>..HEAD`（CI 已由 `.github/workflows/commit-message-ci.yml` 强制）。
- 正式源码 / 工作流走 Git；安装包、更新元数据等大产物不进 Git，仅作 CI Artifact。
- **秘密扫描（Gitleaks CI）**：仓库已配置 `.github/workflows/gitleaks.yml`（固定 Gitleaks **v8.30.1**）——PR 只扫本次变更（`origin/<base_ref>..HEAD`），push 扫全历史；SARIF 报告同时上传 GitHub Code Scanning 与 workflow artifact，发现泄露即 CI 失败。
  - 本地自查（可选）：`gitleaks git --report-format json --report-path gitleaks-report.json --redact=50`；加 `--verbose` 可在终端直接查看命中详情。
  - 本地既存 pre-commit 钩子（`.git/hooks/pre-commit`，写入 `gitleaks protect --staged --redact --no-banner`）**只是快速反馈**，可被 `--no-verify` 绕过，不能作为唯一保障；**CI 才是强制防线**（钩子位于 `.git/`、不入库，新克隆需自行安装）。
  - 不要提交未脱敏的 Gitleaks 报告；报告文件名（`gitleaks.sarif`、`gitleaks-report.json`）建议加入 `.gitignore`（当前尚未忽略）。
- AI/Agent 辅助产出必须经人工逐项核验后提交。

详细迁移开发指南见 `docs-fork/agents/pi-development.md`。
## 品牌与命名边界（fork 自有，2026-10-20 固化）

本 fork 使用**自有名称**，不跟随上游改名：

- 中文名 `设定司`、英文名 `LoreKeeper`，唯一来源是 `tauri-app/src/shared/brand.ts` 的 `APP_BRAND`；产品标识 `com.tancan1998.lorekeeper`、全局根 `AI_NOVEL_LOREKEEPER_HOME` 或 `~/.lorekeeper`、项目目录 `.lore/` 同样属于本 fork 命名，不受上游影响。
- 上游自 2026-10-18（`d21207c8` / PR #434）起把项目改名为「书童 Pagekin」。那只是上游历史，**不采纳**。
- 合并上游时的口径仍然是「fork 增强保留 + 上游改动并入」，但**品牌类冲突一律保留本 fork 命名**：产品名、显示名、图标文案、README 标题、发布说明标题都不因为上游改名而跟随。
- 上游若重构品牌表达方式（新增品牌常量、集中命名、改名脚本），按「机制采纳、命名保留」处理：拿它的结构，换回 `设定司` / `LoreKeeper`。
- 不要为了减少冲突而把本 fork 改回上游名称；同名会与本 fork 的发布物、身份包和数据目录混淆。

## 上游同步时核查的 fork 增强

保留 ui/Dialog.tsx 的弹窗 ESC 栈处理、SettingsModal.tsx 的 ESC 处理、TitleBar.tsx 的 data-tauri-drag-region；弹窗动画修复见提交 61ea32ad。commands/window.rs 已实现四个窗口命令及两段式关窗协议。同步上游时结合当前源码核查这些行为。

## 上游同步流程（2026-10-18 固化）

本 fork 是**增量式**分支：自 fork 点起，上游目录（`src/`、`electron/`、`docs/`、`scripts/`、`plugins/` 等）几乎未被改动过。2026-10-18 实测：合并 upstream/master（当时落后 **787** 个提交）只产生 **1 个冲突**（`.gitignore`，两边各加几行），`tauri-app/` 与 `docs-fork/` **零变更**，根 `pnpm-lock.yaml`、`vite.config.ts`、`.npmrc` 未变，根 `package.json` 只随上游升版本与脚本。

标准步骤：

```powershell
git fetch upstream --tags                       # 先拉取，落后数会远大于本地旧状态
git checkout -b chore/merge-upstream-<YYYYMMDD>
git merge upstream/master --no-edit            # 试合并，不改 master
git diff --name-only --diff-filter=U           # 列出冲突面
# 冲突处理：本项目口径是「fork 增强保留 + 上游改动并入」两者都留
pnpm typecheck; pnpm run lint                  # 根 TS 自检（借上游测试面验证合并结果）
git checkout master; git merge --ff-only chore/merge-upstream-<YYYYMMDD>
git push origin master
git push origin --delete chore/merge-upstream-<YYYYMMDD>
git branch -d chore/merge-upstream-<YYYYMMDD>
```

**两个必须注意的坑（本轮都踩过）**：

1. **提交消息豁免名单**：`.github/workflows/commit-message-ci.yml` 是 fork 独有文件，其设计是「上游导入的历史在任何事件下都不审计」，靠 `UPSTREAM_IGNORE_AUTHORS` 保证「合并 upstream 不会让 push 变红」。上游一旦出现**新贡献者邮箱**，push 就会变红（本轮：`cheng274771204@gmail.com`）。修法就是把它追加到该变量（逗号分隔），本地可用 `node scripts/check-commit-msg.mjs --range <before>..<after> --ignore-author <email>` 先复现与验证。
2. **YAML 缩进**：`UPSTREAM_IGNORE_AUTHORS` 位于 `run: |` 块内，注释与赋值都必须保持 **10 空格**缩进；写到缩进之外会让 GitHub 判为无效工作流，直接生成**日志为空**的失败运行（运行名会显示成文件路径）。改完用 `node -e` 加载 `js-yaml` 解析一遍再推。

## 工作区稀疏检出（sparse on/off）

**2026-10-18 决策**：不把上游文件从 master 剥离。理由：① 剥离后每次合并 upstream 都会对**每个上游改动文件**产生 `modify/delete` 冲突；② 会丢掉逐频道平移所需的 `electron/`、`src/` 行为参照；③ 与 `双栈隔离` 约束、`docs/` 上游镜像不可改、根 `pnpm-lock.yaml` 保持干净且禁删相冲突；④ 剥离不减 `.git`（历史 blob 仍在），只影响工作区。

需要「只看产品树」时用**稀疏检出**而非分支剥离（只影响工作区里被跟踪的文件，不改历史、不删提交）：

```powershell
. .pi\tmp\sparse.ps1 on      # 只留 tauri-app / docs-fork / .github（根文件始终保留）
. .pi\tmp\sparse.ps1 off     # 恢复全树（平移 Electron 参照时用）
. .pi\tmp\sparse.ps1 status  # 查看状态与可见文件数
```

- 开关位于 `.pi\tmp\sparse.ps1`（会话工具，不入库）；切换前会检查未提交改动，有则拒绝执行。
- `node_modules/`、`tauri-app/src-tauri/target/`、`.pi/tmp/` 等被忽略或未跟踪的内容不受影响。
- 上游合并分支**不需要常驻**：每次同步临时开 `chore/merge-upstream-<YYYYMMDD>` 即可（先例：`chore/merge-upstream-20261010`、`20261018`）。