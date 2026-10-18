# Tauri迁移交接快照（2026-10-18）

> [当前入口](./CURRENT.md) · [混合验证](../agents/tauri-rust-ci.md) · [证明审计](../plans/2026-10-17-tauri-proof-audit.md) · [频道清单](../plans/tauri-migration-channel-inventory.md)。本次为第 **95** 次交接。上一日快照见 [2026-10-17](./2026-10-17-tauri-migration-status.md)（第94次）。

## 代码与验证基线

当前代码：**e6d03172**（含上游合并 051c8c83 与两笔 CI 修复），master，已推送origin。本次后续仅文档提交。

**Rust 全量基线仍绑定 72b91745**：本轮上游合并把上游 787 个提交并入 master，但 `tauri-app/` 与 `docs-fork/` **零变更**，Rust 代码指纹未变，故沿用既有全量证据，不重复跑、也不把合并后的文档SHA冒充Rust测试SHA。

| 验证 | 结果 | 绑定证据 |
|---|---|---|
| Rust全量（tauri-app未变） | 1679通过/0失败，库267.99秒 | [39088431086](https://github.com/TanCan1998/Lorekeeper/actions/runs/39088431086)，72b91745 |
| all-targets | 38.37秒，成功无告警 | 同上artifact：expectedSha/testedSha为完整72b91745，test/check退出0、state=passed，两日志已下载核对 |
| 合并后提交规范 | success | [39090445181](https://github.com/TanCan1998/Lorekeeper/actions/runs/39090445181)，e6d03172 |
| 合并后秘密扫描 | success | 39090445076，e6d03172 |
| 本机根TS自检（合并验证） | typecheck 30.3秒、lint 64.5秒，均通过 | 合并提交 051c8c83 上执行 |
| 前端 | 无源码变化（本轮未改前端） | 上次全量：190文件2121通过/1跳过，typecheck/lint成功（8ecfc0fb） |

本轮Rust语义切片累计 **0**（合并与CI修复不计入语义切片）；累计基线仍为 **4**（3607e6c2、f7f5658e、c1547e83、72b91745），全量基线上一次为93b3b27d。

## 上游同步（2026-10-18，第95次主要工作）

- **落后 787 个提交**（含 v1.2.3-Preview）。试合并到临时分支 `chore/merge-upstream-20261018`：**仅 1 个冲突**（`.gitignore`，双方各加几行，两边都留）。
- `tauri-app/`、`docs-fork/` **零变更**；我们的 `tauri-rust-ci.yml`、`commit-message-ci.yml`、`gitleaks.yml`、`scripts/check-commit-msg.mjs` 全部原样保留；根 `pnpm-lock.yaml`、`vite.config.ts`、`.npmrc` 未变；根 `package.json` 仅随上游升版本（1.2.1→1.2.3-Preview）与脚本（新增 `verify:drive`、lint 追加色值检查）。
- 合并内容分布：`src/`411、`docs/`227、`scripts/`53、`electron/`26、`public/`4、`test/`2、`.release/`2、`.github/workflows/pr-ci.yml` 1。这 26 个 `electron/` 与 411 个 `src/` 改动是后续平移的行为参照更新。
- 合并前在临时分支跑根 TS 自检（typecheck/lint）通过，再 `--ff-only` 合入 master；临时分支已删除（本地 + origin）。
- **两处 CI 连带问题（均已修复，提交 4ac2dd4e、e6d03172）**：① 上游新增贡献者 `cheng274771204@gmail.com` 不在提交消息豁免名单，导致 master 的 Commit message CI 变红 → 追加到 `UPSTREAM_IGNORE_AUTHORS`；② 该改动第一次把注释写到 `run: |` 块的 10 空格缩进之外，GitHub 判为**无效工作流**并生成日志为空的失败运行 → 恢复缩进并用本地 `js-yaml` 解析校验后重推，master 现已绿。
- 流程与坑已写入 [项目参考文档](../agents/project-reference.md) 的「上游同步流程（2026-10-18 固化）」章节。

## 分支结构决策（2026-10-18）

用户曾提出「另开分支合并上游、主分支剥离上游文件」。评估后**决定不剥离 master**，改以稀疏检出满足「只看产品树」的需要：

- 剥离后每次合并 upstream 都会对**每个上游改动文件**产生 `modify/delete` 冲突；会丢掉逐频道平移所需的 `electron/`、`src/` 行为参照；与「双栈隔离」「`docs/` 上游镜像不可改」「根 `pnpm-lock.yaml` 保持干净且禁删」相冲突；且剥离不减 `.git`（历史 blob 仍在）。
- 落地：`.pi\tmp\sparse.ps1 on|off|status`（会话工具，不入库）。`on` 后工作区只留 `tauri-app`、`docs-fork`、`.github` 与根文件；`electron/`、`src/`、`docs/` 等隐藏，但 `node_modules/`、`tauri-app/src-tauri/target/`、`.pi/tmp/` 不受影响；切换前检查未提交改动，有则拒绝。已实测 on/off 往返并确认 `git status` 干净。
- 上游合并分支**不需要常驻**，每次临时开 `chore/merge-upstream-<YYYYMMDD>` 即可（先例 20261010、20261018）。

## 六项任务实绩

### 图、行内与恢复（#26–28完成）

- a612cf32修复plan/event输入对象键序误判，6项回归；前端定向33及typecheck/lint通过。真实隐藏UI完成plan/event候选确认、重复确认、plot保存与重复确认；plot仍deterministic兜底，不代表真实模型质量，不改作者事实。
- 行内真实UI：润色→应用原选区；移动选区不重定向；正文新增后拒绝旧候选并保留输入，再通过UI保存。证据inline-ui.json。
- 3041a937显式新租约重新认证；381414cf正常标题栏关闭先挂起、失败不退出。真实退出→重开→图历史继续与侧栏行内继续各1物理请求，原run/root、新epoch；旧epoch均拒绝。

### 完整审修（#29完成，四频道开放）

- b4a8fb72有界prepared准入；702339c1材料闭合哈希；053db7a2默认可见续接算法visible-append-v1；7ae31540保留作者确认原始字节身份。
- 真实隐藏UI完成有效finding→作者确认→修稿→合并→定向复验持久化1次→末审2通过；未自动定稿。
- 8ecfc0fb正式开放 review-revision:prepare / commit-review / commit-revision / read-recovery。

### 复杂归档与冻结读面（#30进行中）

- 真实原生导出/选档/目标目录→恢复新副本：19正文、2稿、5修订、12报告、6周期、7finding及资产保留列一致；头像68字节SHA一致；UI可读草稿/定稿/报告。
- 93b3b27d：认证冻结cycle读面返回None。**f7f5658e**：`verify_m03_review_cycle` 原对整库要求执行链可重放证明，冻结历史被当成当前损坏并连带拒绝同库新周期；现按认证冻结键**逐行跳过**执行链校验，表结构/外键/merge-snapshot计数检查全保留，非冻结行仍严格；读面不发布、守卫拒写、重放仍拒。
- **c1547e83**：`drafts.source_dependencies`按R策略中和后，恢复副本对历史草稿发起新审稿准备即报`GENERATION_REVIEW_HISTORY_CHANGED`；现仅对认证冻结的草稿行按「无已记录依赖」继续，非冻结坏依赖仍拒绝。配套混存回归走真实 prepare→open→settle→commit_review。
- **72b91745**：`project_generation_in_flight`／`session_runs`／`proven_graph_own_event_ids`／`find_finalization_run` 四处读面原走 SQL `json_extract`，在中和空串上抛 malformed JSON，把恢复副本读面整条打死；现统一走新增共用读面 `frozen_aware_json_pointer_rows`（逐行指针匹配、冻结行跳过、非冻结行严格）。回归断言 SQL 形态必失败、空冻结集 fail closed、冻结后读面为空、新根/新运行/新 attempt 仍可读、非冻结坏 JSON 仍报错、源库未改写。
- **同类风险清单（R列中和破坏读面解析）累计已修**：① `drafts.source_dependencies`；② `import_run_receipts`（唯一列 + 冻结收据读面）；③ `verify_m03` 整库执行链要求；④ 本轮四处读面。**其余读R列JSON的路径仍须逐一核对**（生成绑定子串查询、恢复候选、角色提案 payload、审修 usage 收据读面等）。
- **不能扩大结论**：恢复副本中新审修链的**合并与复核**段尚未实际执行；旧handle实测resume为GENERATION_REQUEST_FAILED、read-recovery为GENERATION_JSON_PARSE_ERROR，是拒绝但不是精确冻结合同，不得改写为已通过。

## 构建与测试提速（沿用 2f60163e / 87da0fab，本轮无新增）

| 场景 | 耗时 |
|---|---|
| 本机改一行重编（debug=1 → debug=0） | 70.9s → **48.6s** |
| 本机 rust-lld 对照 | 66.5s（仅快约4.4s，CI不启用） |
| 云端优化前（39041910917） | 6分32秒 |
| 云端冷缓存 | 10分45秒 |
| 云端热缓存 | 5分26秒 / 6分30秒 / 6分29秒 / 6分36秒 |

- `[profile.test] debug = 0` + `incremental = false` 已写入 `tauri-app/src-tauri/Cargo.toml`；断点调试用 `CARGO_PROFILE_TEST_DEBUG=1` 覆盖。
- CI缓存键去掉提交SHA（命中后不再上传GB级target）；证据链仍是 `cargo test --locked` + `cargo check --locked --all-targets`。
- **cargo-nextest 不采用**（本机115.6s vs libtest 79.8s，且会改证据契约）；已装 `D:\Programs\cargo-nextest\bin` 仅手动排查。本机会话提速脚本（不入库）：`D:\Environment\cargo-fast.ps1`。

## 本机证据与恢复入口

所有临时文件/合成内容不入库。本轮未启动GUI/隐藏构建/合成服务，无端口占用。

- 云端artifact：`.pi/tmp/recovery/ci-72b91745/`（1679/0）、`ci-c1547e83/`（1678/0）、`ci-2f60163e/`、`ci-f7f5658e/`；更早见 ci-93b3b27d 等。
- 会话事实源：`.pi/tmp/recovery/background-session.json`；上一轮GUI证据根 `background-1792229720030`（历史，PID9692仅历史记录）。
- 图/行内/恢复：graph-resume-ui-checkpoint.json、inline-ui.json、specialized-native-reopen-ui.json；审修：review-recheck-ui.json、review-cycle-current.json、review-live-state.json；归档：complex-assets-review-ui.ainovel、complex-roundtrip-integrity.json、complex-restored-read.json、complex-avatar-seed.json、complex-restored-session.json。
- 本轮新增会话工具（不入库）：`.pi\tmp\sparse.ps1`（稀疏检出开关）。
- GUI脚本须通过 node .pi/tmp/recovery/background-inspect.cjs <模块路径> 调用；重启前先隐藏构建。

## 后台收口与边界

- 本轮无GUI：未启动隐藏窗口、未启动合成mock、未启动Vite；5190/9226/19537无监听。
- 无代理派发；自动定稿、真实模型、外部MCP、业务云、发布未授权；macOS/Linux未认证；旧库转换首版不支持。
- 冻结历史不是执行权威，原始收据身份不改写；新租约恢复必须重新认证。

## 下轮明确断点

1. TODO：#25–29完成；#30进行中（便携/导入读面收口）；#31待最终收口。
2. 恢复副本中新审修链的**合并与复核**段仍待实跑（新建与周期读面已由 c1547e83 覆盖）。
3. 继续扫描其余读R列JSON的路径（生成绑定子串查询、恢复候选、角色提案 payload、审修 usage 收据读面等），冻结过滤与当前损坏仍须拒绝。
4. 旧handle泛化拒绝错误如需规范化，添加先失败回归，保留坏当前收据拒绝与非冻结校验，不能冒充历史执行权威。
5. 上游 787 提交带来的 `electron/`（26）与 `src/`（411）改动作行为参照更新：后续平移对应频道时以合并后的最新上游实现为准。
6. 完成#30后再做#31最终交接/测试口径收口。
