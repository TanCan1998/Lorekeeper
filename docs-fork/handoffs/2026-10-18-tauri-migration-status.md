# Tauri迁移交接快照（2026-10-18）

> [当前入口](./CURRENT.md) · [混合验证](../agents/tauri-rust-ci.md) · [证明审计](../plans/2026-10-17-tauri-proof-audit.md) · [频道清单](../plans/tauri-migration-channel-inventory.md)。第 **102** 次交接；同日第101次见 f387a5b0，上一日为[第94次](./2026-10-17-tauri-migration-status.md)。

## 当前代码与已验证状态

代码 **0f0880d27b29d5ef0546fa78a664afe0317833d6**，master，已推送。本文件不改变Rust代码指纹；本轮为一次Rust架构规范重构、一次原生归档整链验收与首版范围清点，没有真实模型/MCP/业务云调用或发布。

| 验证 | 结果 | 版本/证据 |
|---|---|---|
| 最新云端全量 | **1692通过/0失败**，库223.16秒；all-targets36.80秒无告警 | 0f0880d2 / [39105179211](https://github.com/TanCan1998/Lorekeeper/actions/runs/39105179211)，2026-10-18 06:52 UTC完成 |
| 前片云端全量 | 1690/0，库196.58秒；all-targets35.54秒无告警 | b5850fbe / [39102768655](https://github.com/TanCan1998/Lorekeeper/actions/runs/39102768655) |
| 本机定向（本轮） | update30/0、mcp35/0、克隆句柄回归3/0；all-targets52.50秒 | 0f0880d2 提交前；定向rustfmt与diff检查通过 |
| 原生归档整链 | 真实导出/原生保存选择/原生文件夹选择/正式恢复/副本编辑保存/项目与应用重开全部通过 | native-final-1792304552720/acceptance-evidence.json |
| 前端 | 本轮未改，未重跑；沿用190文件2121通过/1跳过、typecheck/lint/build通过 | 第101次原生验收内记录 |
| 代码规范/秘密扫描 | commit message与gitleaks均success | 0f0880d2：[39105179120](https://github.com/TanCan1998/Lorekeeper/actions/runs/39105179120)/[39105179174](https://github.com/TanCan1998/Lorekeeper/actions/runs/39105179174) |
| 双栈隔离 | electron、根src/docs、根配置与lock无差异 | 改动仅5个 tauri-app/src-tauri 文件 |

云端产物已下载核对：expectedSha与testedSha同为0f0880d2、runAttempt=1、runnerOs=Windows、toolchain1.99.0、testExitCode与checkExitCode均0、cargoLockSha256=7b6c5b1c…、state=passed，两日志无 error/warning。测试数由1690升至1692，增量正是本轮两条克隆句柄回归。Rust语义切片累计 **0**。

## 本轮：Rust UX 规范审查与 A1/A2 落地

用户要求在 #31 首版范围最终检查前，先按 `rust-lib-ux`（Microsoft Libraries/UX 14 条）审查本项目Rust代码。审查只读完成后再执行获准的两项改动。

**审查口径**：本 crate 是 Tauri 应用（`lorekeeper_lib`，单一消费者），不是发布库。按指南「应用内部私有可适度放宽，跨模块/跨 crate 暴露才按规则执行」，只判定会被命名、注入、替换的边界，不把 IPC DTO 的 `pub` 字段计入违规。

**盘点**：198 个 `.rs`、1315 个 `pub fn`、609 个 `pub struct`、124 个 `pub enum`、6 个 `pub trait`。

实测合规：`pub fn` 参数含 `Arc/Rc/RefCell/Mutex/Box` 为 **0**；无 `prelude`；返回 `impl Future` 为 0（唯一 1 处是 `webdav_backup.rs` 的 `future` 参数，供 `tokio::select!`）；无对外自定义集合类型且 3 组 `len/is_empty` 全部配对；`Pin<Box<dyn Future>>` 9 处全在 trait 方法内（指南允许）；`CloudCredentialStore<P = DpapiProtector>` 为「泛型+默认具体实现」；`GithubReleaseBackend` 有 inherent 方法再转发 trait；`UpdateServiceOptions`/`GenerationRunServiceDependencies` 已是「必需参数打包 deps」形态。

**A 类（已改，0f0880d2）**：

- `UpdateService` 与 `McpManager` 原本无 `Clone`，由 `AppState` 与命令层用 `Arc` 二次包装（`Mutex<Option<Arc<UpdateService>>>`、`Mutex<Arc<McpManager>>`、`pub service: Arc<UpdateService>`），同时命中 M-SERVICES-CLONE 与 M-AVOID-WRAPPERS。
- 改为内部持 `Arc` 内核并 `#[derive(Clone)]`：`Clone` 只复制句柄，状态仍单份。`AppState` 按值保存，命令层签名不再出现 `Arc`。
- 新增两条回归，断言克隆体的写入必须对原句柄可见（`update::service::tests::cloned_service_shares_state_test`、`mcp::tests::cloned_manager_shares_tables_test`）——若误写成深拷贝即失败。
- IPC 契约、数据库 schema、文件权限、定稿生命周期、会话准入均未改动。

**B 类（真实但本轮不改，记为技术债）**：`GenerationRunService` 仍无共享 `Clone`（3495 行、字段被直接访问遍布，改造回归风险高于收益）；`GenerationRunServiceDependencies` 的 4 个 `pub Arc<dyn …>` 字段与 `ValidateRecovery` 别名（仅构造期注入、横切扩散大）；`AgentHost` 以 `Value` 传视图（待强类型化，属领域模型）；`AgentGeneration::new` 与 `build_main_generation_plan` 各 6 参（借用视图/基线纯函数平移）。

**C 类（规则命中但项目契约明确，明确不改）**：`Result<_, String>` 1585 处与 2340 处 `.map_err` 服从 Tauri 命令错误契约与 `Error: ` 前缀，领域错误以稳定 code 表达，canonical struct 的前提不成立；`lib.rs` 全 `pub mod` 是应用 crate 避免 `dead_code` 的既有做法；43 处 `pub fn get_*` 为基线仓储方法 1:1 平移，改名会破坏与 `electron/` 的逐条对照。

## 原生归档整链验收（#30 关闭依据）

用户本次明确解除前台交互限制（仅限 #30/#31 验收），仍只用隔离合成项目：

1. 点击真实「导出本地存档」按钮 → 原生**保存**对话框（标题 `导出项目存档`）→ 本进程 HWND 定向选择 `Edit` id 1001 与 `Button` id 1，未注入全局键鼠、未切换焦点。
2. 导出 2231534 字节，SHA256 `2a4aa6b974e1f89369d2cc6175f129487c1592c1f806e3eb44b36cb773426c6b`，界面收据哈希匹配；源项目 `0a3b457b-93f1-473e-a194-6dab64c2b01b` 及其 lease 未变。
3. 「从本地存档恢复副本」→ 原生**文件夹**选择（标题 `选择恢复副本所在文件夹`，`Edit` id 1152、`Button` id 1「选择文件夹」）→ 正式恢复命令生成新副本 `4aa04e1d-9adf-49c4-aa1c-92ef3a15bae6`（`合成迁移验收-恢复副本`），未替换当前项目；便携图片 1 个哈希一致。
4. 副本经真实 `useProjectStore.openProject` 打开，在真实 CodeMirror 中改第二章作者稿并点击真实保存按钮（bodyLength 2900）；旧 lease 拒绝、第一章定稿不变、`generation:list` 零候选。
5. 项目关闭/重开、应用正常退出/重启后重验：持久化一致、新 lease、旧 lease 拒绝、编辑器内容匹配；角色页实际渲染恢复头像（blob、`complete`、`naturalWidth > 0`、alt 含「头像」）。
6. harness `--compare-source`：源作者稿与源定稿均未变；`--verify`：33/33 历史 run 冻结、1 个可变稿、`frozen_write_rejected=true` 且 `total_changes` 不变。
7. GUI 正常关闭；独占 Vite 按 PID/命令行核对后停止；默认构建恢复。

证据汇总 `.pi/tmp/recovery/native-final-1792304552720/acceptance-evidence.json`：head f387a5b0、backendSha b5850fbe、binarySha256 95d471d5…、nativeRestore=true、portableAssetCount=1、postRestoreAppRestart=true、sourceUnchanged=true、guardFreezeRuns=33、realExternalServicesTested=false、releaseReady=false；`git diff --name-only b5850fbe -- tauri-app` 为空。

**据此 #30 可登记关闭**：导出、原生选档、恢复新身份、副本编辑保存、重开与重启、冻结守卫均有可复核证据。仍不包含真实模型/MCP/业务云调用，也不等于 macOS/Linux 原生归档已认证。

## #31 首版范围最终检查

以 0f0880d2 源码重算，不引用历史章节数字：

| 项 | 结果 |
|---|---|
| 版本号 | Cargo.toml、package.json、tauri.conf.json 三处一致 **1.1.0** |
| 契约 invoke 频道 | **290** |
| 已注册命令 | **291** → 注册契约频道 **290**（多出的 1 个是 `app_health_check`，非业务频道） |
| 功能迁移 | **289/290** |
| 首版明确不支持 | **1**：`project:import-legacy-copy` → `LEGACY_IMPORT_UNSUPPORTED`（不读旧库、不建副本、不消耗授权） |
| 未迁移 invoke 频道 | **0** |
| 事件频道 | **7**：`update:state`、`window:close-requested`、`llm:stream-chunk/done/error`、`generation:snapshot`、`generation:reasoning` |
| 命令名与契约对应 | 一一对应，`pnpm run check:channels` 通过，生成物未过期 |
| 前端路由 | 285/290 为历史口径；`review-revision:*` 四频道已于第91次接通，`DEFERRED_RENDERER_CHANNELS` 现为空 |

**#31 可声明**：首版范围内每个 invoke 频道都有明确处置——289 项功能迁移完成，1 项按既定决策明确不支持，0 项悬空；7 个事件频道均已接线。

**#31 不能推出**：真实模型/外部MCP/业务云/发布就绪；macOS/Linux 认证；所有 JSON 消费者与交错场景穷尽；生产质量或安全认证。`update:download`/`update:quit-and-install` 仍诚实返回 `DOWNLOAD_NOT_READY`/`INSTALL_NOT_READY`，属既定架构选择而非缺口。

## 边界与未解除项

- 子代理派发继续停用，全部由主会话直接执行。
- 原生对话框的前台授权仅限本次 #30/#31 验收，已随验收结束失效；后续恢复默认「不置顶、不激活、不抢焦点」。
- 未改数据库 schema/迁移、未加依赖、未改 CI 与发布配置、未改定稿/文件权限语义、未改 Electron 与根配置。
- 不把测试数量或命令注册数当作发布认证；不追溯把旧确认卡取消当同意。

## 证据与接续

- 本机：`.pi/tmp/recovery/native-final-1792304552720/`（source/home/archive/target/evidence JSON/截图/日志）；`.pi/tmp/native-final-session.json` 已 closed，PID 仅历史。
- 云端：`.pi/tmp/recovery/ci-0f0880d2/`（validation.json、cargo-test.log 176826字节、cargo-check.log）。
- 审查脚本（不入库）：`.pi/tmp/rust-ux-scan{,2,3}.cjs`、`.pi/tmp/rust-ux-refactor.cjs`。
- 当前活动事实源 `.pi/tmp/pipeline-state.md`；上述辅助源码、构建、归档、合成正文和日志均不入库。
