# Tauri 迁移进度快照（2026-10-15）

> 第 54 次；用户确认「继续迁移、不派发、一批接一批」并批准开工 **B7 启动态子集**（A 方案 + 皮肤/生成号落 Lorekeeper 全局根）。主会话直接实施，未派发子代理。
> 同日替换当前摘要，第53次内容保留于 Git 历史；旧日期快照冻结。
> 当前入口：[CURRENT.md](CURRENT.md)；维护规则：[开发指南 §9](../agents/pi-development.md#9-交接状态与快照维护)。

## 已验证状态（第54次）

最新代码：`74ea2200` / `master`，已推送 `origin/master`；文档更新前工作区干净（另有浏览器测试遗留 artifact 未跟踪，见文末）。日期 2026-10-15。

| 指标 | 结果 | 验证版本 / 范围 | 证据（本机，不入库） |
|---|---|---|---|
| 本轮 Rust 全量 | **1397 passed / 0 failed，57.56s** | `74ea2200`，`cargo test` | `.pi/tmp/b89-rust-full.log` |
| 编译检查 | `cargo check --all-targets` 零告警；定向 rustfmt `--check` 通过 | `74ea2200` | `.pi/tmp/b89-check.log`、`.pi/tmp/b89-startup-gate-target.log` |
| 启动门禁定向 | `startup_gate` **9项**、`commands::startup` 1项、`commands::skin` **8项** 全通过 | `74ea2200` | `.pi/tmp/b89-{startup-gate-target,startup-commands,skin-target}.log` |
| 前端启动传输 | 新 `startup-transport` **4项**、会话网关/主题 **28项** 通过；typecheck、lint 退出0 | `74ea2200` | `.pi/tmp/b89-{startup-transport,startup-frontend-target,typecheck,lint}.log` |
| 频道计数（本轮实测） | 契约 invoke 290、事件 7；**已注册 246 → 覆盖 245**；**未迁移 45**（原 48） | `74ea2200`，`pnpm run check:channels` | `.pi/tmp/b89-channels.log`、`.pi/tmp/b89-channels-emit.log` |
| 浏览器套件（逐项失败） | 48文件：39通过 / 9失败；357测试：319通过 / 38失败 | `74ea2200` 的整仓浏览器配置运行；失败集中在图谱/编辑器/批量工作流等未迁移域 | `.pi/tmp/b89-startup-browser.log` |
| GUI / 真实模型 / MCP | 未验收 | 启动门禁已可回 ready，但未在真实窗口跑通；合成传输不等于端到端 | 本轮无 GUI 结论 |

Rust验证目录 `tauri-app/src-tauri/`（显式 `RUSTUP_HOME`/`CARGO_HOME`）；前端在 `tauri-app/` 用 `pnpm exec vitest run <文件>`、`pnpm typecheck`、`pnpm run lint`；浏览器配置为 `pnpm run test:browser`。

## 本轮交付与关键决策（第53次 → 第54次）

1. **启动门禁三频道接通**（`74ea2200`）：新增 `services/startup_gate.rs`（状态机 + 全局根持久化）与 `commands/startup.rs`（`startup_get_state` / `startup_skin_snapshot` / `startup_appearance_ack`）。渲染层启动页此前因三频道未登记而**恒停在「启动尚未完成」**，工作台无法加载；现在三频道的语义与基线 `electron/controllers/startup-controller.ts` 对齐：未就绪回 `pending`、阻塞回 `blocked`（可带码）、快照漂移以错误拒绝、非法读回回 `false`。
2. **持久化位置**：`AI_NOVEL_LOREKEEPER_HOME` / `~/.lorekeeper/appearance.json` 保存 `{version, skinRevision, backgroundSkin}`；`globalGeneration` 取该文件的内容修订（FNV-1a 64，含版本与皮肤），皮肤修订为独立单调计数。首次装载写入初值；文件不可读 / JSON 非法 / 版本不认识 → `GLOBAL_DATA_BLOCKED` 且**不覆盖**作者文件；皮肤取值非法 → `SKIN_NOT_READY`。
3. **皮肤状态单一权威**：activeSkin 改由门禁状态持有，`skin:get-state` 按需组合，`activate` 现在**持久化**并前移皮肤修订（重启后仍生效）。`import-custom` 的占位实现不再报告假成功，改为 `CUSTOM_SKIN_UNAVAILABLE`；`remove-custom` 行为不变。
4. **发送方等价物**：只允许标签为 `main` 的窗口、且其当前 URL 属于本应用自身 origin（`tauri://localhost`、`http://tauri.localhost`、开发服务器 `localhost|127.0.0.1`）的调用；不信任发送方的 `get-state` 只回 `{state:'blocked'}`（不泄漏快照、不回码）。
5. **刻意偏离（已记录）**：Tauri 两栈隔离，不读基线目录，故 `GLOBAL_MODEL_CREDENTIALS_DIFFER` 永不产生、`migrationNotice` 恒为 `None`；外观档案仍由渲染层 `localStorage` 持有，读回修订按窗口在内存中记录（不落盘，避免作者存储被清空/替换后锁死启动）。
6. **守护测试**：`startup_gate` 9项（内容修订前移/恢复、坏文件不覆盖、皮肤非法阻塞、快照漂移、读回形状与单调性、未装载 pending）、`commands::startup` 信任发送方 URL 黑白名单、`commands::skin` 8项（持久化、非法皮肤、未装载、占位不假成功）、前端 `startup-transport` 4项（已迁移登记、命令名与命名参数、无项目会话注入、其它未迁移频道仍被拦截）。
7. **修复一处真实死锁**：`match` 直接以 `Mutex::lock()` 为 scrutinee 时临时守卫活到整个 match，分支内二次上锁即自死锁；改为先绑定结果再匹配（`commands/skin.rs`）。

第52/53次交付（草稿真实传输接线、unknown只补导航、恢复身份与工具回执校验、在途取消后只重放、助手客户端停止/关闭后不再发起新运行、失败分类复核）结论不变。

## 尚未解除的失败与阻塞

- **B7 其余部分仍未迁移**：`project:overview-current` / `project:peek-overview` / `project:archive-*` / `project:import-legacy-copy` / `kb:*-document-copy` / `dialog:select-{legacy-project,project-archive,project-archive-export,project-restore-target}` / `character-avatar:4` / `cloud-backup:8` / `import-generation:2`，以及特殊域 `finalization-generation:5` / `finalized-character:5` / `graph-generation:5` / `editor-inline:4`。归档类需要新 crate 与 schema，仍须先确认。
- **前端全量**：最近一次非浏览器全量扫描仍为 `f15a1b71` 的 170文件 / 7文件失败 / 123项失败；浏览器套件 9文件 / 38项失败（本轮实测，未修复）。剩余失败应先分类为测试适配、真实迁移缺口或未开放域，**不得补映射绕过**。
- 前端浏览器测试遗留 `tauri-app/.vitest-attachments/`（38个失败附件）与若干空 `__screenshots__` 目录为未跟踪 artifact，尚未清理（删除属破坏性操作，待用户确认）。
- 真实模型、完整 GUI 窗口、MCP、Windows 更新、dirty 关窗、整屏瞬黑未验收；约83%仅为历史频道覆盖率，不是产品完成度。

## 交接给下次会话

- 分支 `master`；最新代码 `74ea2200` 已推送。活动/待续入口：`.pi/tmp/pipeline-state.md`（本机，不入库）。开发子代理派发仍禁止。
- **启动门禁首验未做**：请先在真实 `tauri dev`/`build` 窗口确认 `~/.lorekeeper/appearance.json` 初值与「能进入工作台」；若皮肤或全局根异常，应看到阻塞页而不是默认值覆盖。
- 次优先：B7 的 `project:*` 概览/归档、`kb` 文档副本、`dialog:select-*` 授权、`cloud-backup`、`character-avatar`；其中新 crate / schema 仍先确认。特殊域与 `generate_blueprint` / `generate_architecture` 等继续关闭。
- 真实模型、完整 GUI、MCP、Windows 更新、dirty 关窗、整屏瞬黑未验收；Electron、根 `src/`、上游 `docs/`、根配置与锁文件保持隔离。
