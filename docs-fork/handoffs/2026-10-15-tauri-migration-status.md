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
| 前端全量（非浏览器） | **172文件/2053项：1929通过 / 123失败 / 1跳过**（7文件失败） | `74ea2200`，`vitest run`（JSON 报告） | `.pi/tmp/b91-results.json`、`.pi/tmp/b91-full.log` |
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

## 剩余 45 个未迁移频道的工作量评估（本轮调研结论）

本轮把 45 个未迁移频道逐个看过实现，结论是**没有可当天闭合的小切片**，全部属于「大件」或「先确认」。下次会话可直接按此表选型，不必重新调研：

| 频道组 | 数量 | 基线实现规模与前置 | 结论 |
|---|---|---|---|
| `kb:read/save/reindex-document-copy` | 3 | 基线在 `electron/knowledge-base.ts` 只有 3 个小函数，但依赖 `knowledge-copy:` 副本旁路文件（`vector-store.ts` 的 `readKnowledgeCopy`/`writeKnowledgeCopy` 与 `indexDirty`/`indexedHash`）、`withKnowledgeSourceGate`（按项目存储根串行化的短临界区），以及**已迁移导入路径**写入的副本标记 | 中等偏大：会动到已迁移的 kb 导入行为并新增旁路文件与门禁，需独立切片 |
| `project:overview-current` / `peek-overview` | 2 | 基线为 `electron/services/project-peek.ts`（**17.9KB**）：把目标库复制到 scratch 根只读打开、物理文件快照（size/hash/device/inode/mtime）、manifest 与迁移日志校验、schema 校验、阶段状态推导；`peek-overview` 另需「最近项目预览能力」注册表 | 大件且安全敏感：必须整块移植，不做简化版（避免派生投影失真） |
| `dialog:select-{legacy-project,project-archive,project-archive-export,project-restore-target}` | 4 | 选择器本身小（可复用已有授权机制），但唯一消费者是归档导出/恢复与旧库导入，需要 **zip/crc32 类新 crate** 与 `VACUUM INTO` 替代 | 先确认（新依赖 / schema） |
| `character-avatar:choose/commit/read-batch/remove` | 4 | 需要头像表 **schema 迁移** 与文件落盘位置约定 | 先确认（schema） |
| `cloud-backup:8` | 8 | WebDAV/HTTP 远程备份、凭据存储与恢复编排 | 先确认（新依赖 + 凭据） |
| `import-generation:read/execute` | 2 | Rust 侧已有契约类型（`services/import_generation.rs`）与导入槽位冻结（`generation_source_binding`），但生成 owner 未接导入槽位执行路径 | 中等偏大：需在 owner 内接通导入轮次 |
| `finalization-generation:5` / `finalized-character:5` / `graph-generation:5` / `editor-inline:4` | 19 | 特殊域 | 维持关闭，不因持续迁移自动开放 |


- **B7 其余部分仍未迁移**（已逐个调研，见上节「剩余频道工作量评估」）：`project:*`、`kb:*-document-copy`、`dialog:select-*`、`character-avatar:4`、`cloud-backup:8`、`import-generation:2`，以及特殊域 `finalization-generation:5` / `finalized-character:5` / `graph-generation:5` / `editor-inline:4`。
- **前端全量（本轮实测）**：非浏览器全量在 `74ea2200` 下为 172文件 / 7文件失败 / 2053项（1929通过 / 123失败 / 1跳过），失败文件与其逐项数量与 `f15a1b71` **完全一致**，本轮改动未新增失败；浏览器套件 48文件 / 9文件失败 / 38项失败（未修复）。剩余失败应先分类为测试适配、真实迁移缺口或未开放域，**不得补映射绕过**。
- 真实模型、完整 GUI 窗口、MCP、Windows 更新、dirty 关窗、整屏瞬黑未验收；约84%仅为历史频道覆盖率，不是产品完成度。

## 交接给下次会话

- 分支 `master`；最新代码 `74ea2200` 已推送。活动/待续入口：`.pi/tmp/pipeline-state.md`（本机，不入库）。开发子代理派发仍禁止。
- **启动门禁首验未做**：请先在真实 `tauri dev`/`build` 窗口确认 `~/.lorekeeper/appearance.json` 初值与「能进入工作台」；若皮肤或全局根异常，应看到阻塞页而不是默认值覆盖。
- 次优先：按上节评估表整体推进一件「大件」（建议顺序：`kb:*-document-copy` → `project:overview-current`/`peek-overview` → `import-generation`），或先做 `dialog:select-*` + 归档（**需先确认 zip 依赖与存储 schema**）。特殊域与 `generate_blueprint` / `generate_architecture` 等继续关闭。
- 真实模型、完整 GUI、MCP、Windows 更新、dirty 关窗、整屏瞬黑未验收；Electron、根 `src/`、上游 `docs/`、根配置与锁文件保持隔离。
