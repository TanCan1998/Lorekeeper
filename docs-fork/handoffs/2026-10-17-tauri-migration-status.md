# Tauri迁移交接快照（2026-10-17）

> [当前入口](./CURRENT.md) · [混合验证](../agents/tauri-rust-ci.md) · [证明审计](../plans/2026-10-17-tauri-proof-audit.md) · [频道清单](../plans/tauri-migration-channel-inventory.md)。本次为第 **92** 次交接；当前摘要替换同日旧摘要。

## 代码与验证基线

当前代码：**f7f5658e61a01b74c972f148215fc7fbc3f13678**，master，已推送origin。本轮无未提交改动。不把文档SHA冒充Rust测试SHA。

| 验证 | 结果 | 绑定证据 |
|---|---|---|
| 当前Rust全量 | 1677通过/0失败，库253.00秒 | [39051391021](https://github.com/TanCan1998/Lorekeeper/actions/runs/39051391021)，f7f5658e |
| 当前all-targets | 38.98秒，成功无告警 | 同上artifact：expectedSha/testedSha为完整f7f5658e，test/check退出0、state=passed，两日志已下载核对 |
| 当前提交规范/秘密扫描 | success | 39051388200 / 39051390550，同SHA f7f5658e |
| 本机当前切片定向 | review_revision 66/0、review_cycle 9/0、portable 86/0、冻结导入收据1/1、脱敏导入历史1/1 | 本轮 cargo test 输出；cargo fmt --check 与 git diff 通过 |
| 前端 | 无源码变化（本轮未改前端） | 上次全量：190文件2121通过/1跳过，typecheck/lint成功（8ecfc0fb） |
| 上一完整云端 | 2f60163e：1675/0（161.67秒），check 34.60秒 | 39049883469；同SHA冷缓存39049045752为10分45秒 |

本轮Rust语义切片累计 **2**（3607e6c2、f7f5658e）；CI/构建配置与文档切片不计入。全量基线上一次为93b3b27d。

### 本轮构建与CI提速（2f60163e / 87da0fab）

| 场景 | 耗时 |
|---|---|
| 本机改一行重编（debug=1 → debug=0） | 70.9s → **48.6s** |
| 本机 rust-lld 对照 | 66.5s（仅快约4.4s，CI不启用） |
| 云端优化前（39041910917） | 6分32秒 |
| 云端冷缓存（39049045752） | 10分45秒 |
| 云端热缓存（39049883469 / 39051391021） | **5分26秒 / 6分30秒** |

- `[profile.test] debug = 0` + `incremental = false` 已写入 `tauri-app/src-tauri/Cargo.toml`；断点调试用 `CARGO_PROFILE_TEST_DEBUG=1` 覆盖。
- CI缓存键去掉提交SHA（命中后不再上传GB级target，避免挤占10GB配额）；restore-keys保留v1前缀。证据链仍是 `cargo test --locked` + `cargo check --locked --all-targets`。
- **cargo-nextest 不采用**：本机实测115.6s vs libtest 79.8s，且会改动testCommand/日志名破坏脚本合同。已装 `D:\Programs\cargo-nextest\bin`（用户PATH）仅用于手动排查。
- 本机会话提速脚本（不入库）：`D:\Environment\cargo-fast.ps1`（RUSTUP_HOME/CARGO_HOME + 可选rust-lld + 测试debug=0）。

### 本轮会话其他环境变更（不入库）

- pi 技能 `rust-lib-ux`：微软 Rust Guidelines Libraries/UX 14条规则速查 + 审查清单 + 原文 references。
- pi 技能 junction `rust-skills` → `D:\Programs\rust-skills`（v2.0.9，38个技能，保留跨目录引用）。

## 六项任务实绩

### 图、行内与恢复（#26–28完成）

- a612cf32修复plan/event输入对象键序误判，6项回归；前端定向33及typecheck/lint通过。真实隐藏UI完成plan/event候选确认、重复确认、plot保存与重复确认；plot仍deterministic兜底，不代表真实模型质量，不改作者事实。
- 行内真实UI：润色→应用原选区；移动选区不重定向；正文新增后拒绝旧候选并保留输入，再通过UI保存。仅CodeMirror公开dispatch与真实可见按钮，证据inline-ui.json。
- 3041a937显式新租约重新认证，重新验证来源/模型/正文或原选区/预算；381414cf正常标题栏关闭先挂起、失败不退出。真实退出→重开→图历史继续与侧栏行内继续各1物理请求，原run/root、新epoch；旧epoch均拒绝。381对应云端39026151847完整核对1668/0。

### 完整审修（#29完成，四频道开放）

- b4a8fb72有界prepared准入；702339c1材料闭合哈希；053db7a2默认可见续接算法visible-append-v1；7ae31540保留作者确认原始字节身份。
- 真实隐藏UI完成有效finding→作者确认→修稿→合并→定向复验持久化1次→末审2通过；未自动定稿。
- 8ecfc0fb正式开放 review-revision:prepare / commit-review / commit-revision / read-recovery；DEFERRED_RENDERER_CHANNELS为空。

### 复杂归档与冻结读面（#30进行中）

- 真实原生导出/选档/目标目录→恢复新副本：19正文、2稿、5修订、12报告、6周期、7finding及资产保留列一致；头像68字节SHA一致；UI可读草稿/定稿/报告。
- 93b3b27d：认证冻结cycle的读面返回None，不发布当前执行投影。**f7f5658e补全**：`verify_m03_review_cycle` 原对整库要求执行链可重放证明，冻结历史被当成当前损坏并连带拒绝同库新周期；现按认证冻结键**逐行跳过**执行链校验，表结构/外键/merge-snapshot计数一致性检查全部保留，非冻结行仍严格。豁免不授予权威：读面不发布、守卫拒写、重放仍拒。
- 回归含两个对照（无守卫连接仍拒绝、空冻结集仍拒绝），证明豁免来自认证冻结键而非放宽校验。
- 导入侧（3607e6c2）：认证冻结`import_runs`的effect receipt读面返回None，不再退化为泛化「receipt损坏」；便携脱敏对参与`UNIQUE(...)`的R/X文本列改用`portable-neutral:<行身份>`确定性中和，修复多条`import_run_receipts`恢复撞唯一约束报`PORTABLE_UNSAFE_PROJECTION`。
- **不能扩大结论**：仍待恢复副本中**新**审修完整链的实际执行验证；旧handle实测resume为GENERATION_REQUEST_FAILED、read-recovery为GENERATION_JSON_PARSE_ERROR，是拒绝但不是精确冻结合同，不得改写为已通过。

## 本机证据与恢复入口

所有临时文件/合成内容不入库。**本轮未启动GUI/隐藏构建/合成服务**，无端口占用。

- 会话事实源：.pi/tmp/recovery/background-session.json；上一轮GUI证据根：.pi/tmp/recovery/background-1792229720030（历史，PID9692仅历史记录）。
- 云端artifact：`.pi/tmp/recovery/ci-2f60163e/`（1675/0）、`.pi/tmp/recovery/ci-f7f5658e/`（1677/0）；更早见 ci-93b3b27d 等。
- 图/行内/恢复：graph-resume-ui-checkpoint.json、inline-ui.json、specialized-native-reopen-ui.json。
- 审修：review-recheck-ui.json、review-cycle-current.json、review-live-state.json、review-mock-events.jsonl。
- 归档：complex-assets-review-ui.ainovel、complex-roundtrip-integrity.json、complex-restored-read.json、complex-avatar-seed.json、complex-restored-session.json。
- GUI脚本须通过 node .pi/tmp/recovery/background-inspect.cjs <模块路径> 调用；重启前先隐藏构建，不能直接启动当前默认二进制。

## 后台收口与边界

- 本轮无GUI：未启动隐藏窗口、未启动合成mock、未启动Vite；5190/9226/19537无监听。
- 无代理派发；自动定稿、真实模型、外部MCP、业务云、发布未授权；macOS/Linux未认证；旧库转换首版不支持。
- 冻结历史不是执行权威，原始收据身份不改写；新租约恢复必须重新认证。

## 下轮明确断点

1. TODO：#25–29完成；#30进行中（便携/导入读面收口）；#31待最终收口。
2. 在恢复副本中实际跑通**新**审修完整链（冻结历史与新周期混存下的新建/合并/复核），验证f7f5658e的逐行豁免不影响新周期严格性。
3. 旧handle泛化拒绝错误如需规范化，添加先失败回归，保留坏当前收据拒绝与非冻结校验，不能冒充历史执行权威。
4. 补齐其余便携/导入读面（含真实合成import_run来源槽位与生成读面），冻结过滤与当前损坏仍须拒绝。
5. 完成#30后再做#31最终交接/测试口径收口；交接快照与proof-audit同步为第92次。
