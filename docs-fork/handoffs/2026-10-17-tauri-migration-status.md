# Tauri迁移交接快照（2026-10-17）

> [当前入口](./CURRENT.md) · [混合验证](../agents/tauri-rust-ci.md) · [首版决策](../plans/2026-10-17-tauri-completion-decisions.md) · [证明/GUI审计矩阵](../plans/2026-10-17-tauri-proof-audit.md)。动态进度仅在docs-fork展示，同日改写当前摘要。

## 已验证状态（第85次）

当前代码：`61ab33bc822f3864cc2b6c309aa30d265eacbb1b`，master，已推送origin。后续为文档收口。

| 范围 | 结果/版本 | 证据 |
|---|---|---|
| 当前SHA云端 | Tauri Rust CI运行中，未登记全量通过 | [38999595785](https://github.com/TanCan1998/Lorekeeper/actions/runs/38999595785)；提交规范38999595799、Gitleaks38999595870同样待结果 |
| 最近成功Rust云端全量 | **1637通过/0失败**，库227.11秒，SHA=`183d4886` | [38997082671](https://github.com/TanCan1998/Lorekeeper/actions/runs/38997082671)，本机`.pi/tmp/recovery/ci-183d4886/` |
| 最近成功all-targets | 同183d4886成功无告警；expectedSha/testedSha一致、两个退出码0、state=passed | 同run artifact已下载核对；不冒充当前SHA结果 |
| 本轮本机定向 | 仓储30/0、`generation_owner`过滤187/0、便携73/0、run服务38/0 | 对应cargo test过滤；不相加冒充全量 |
| 本轮新增回归 | 实际脱敏流水线到候选仓储，修复前1失败、修复后1通过 | `sanitized_generation_candidates_skip_frozen_run_before_decoding_budget_test`，包含在便携73项内 |
| 格式/隔离 | 两文件rustfmt与diff通过；无前端、Electron、schema、依赖或CI变更 | 本机定向检查 |
| 前端沿用 | 188文件2111通过/1跳过，typecheck/lint及上次build通过 | 本轮无前端改动，未重跑，不算当前新增验收 |
| 频道口径 | 注册291、契约注册290、事件7；功能289/290、业务285/290 | 本轮不变；四生产审稿频道继续关闭 |

最近全量基线=183d4886，**其后Rust语义切片累计1**；当前云端结果未完成，不清零。

## 本轮修复与有界结论

- `repositories/generation_run_repository.rs`：按指定运行读取候选时，原实现先调用`get`/`budget`解析已中和的JSON，再逐attempt检查冻结。即使全局候选列表已跳过冻结attempt，`list_candidates(Some(run_id))`仍报`GENERATION_JSON_PARSE_ERROR`。
- 修复：在查询/解析前通过统一`frozen_row`判断指定运行是否冻结，冻结则返回空候选；不新增独立判空规则，不改写守卫、准入语义、持久schema或权限。
- `services/portable_project_sanitize.rs`：建立合成源库→实际脱敏副本→生成冻结侧车→安装连接级守卫→读取全局/指定运行候选的回归。未安装守卫时空绑定仍报错，缺失运行仍报错；安装后两种候选读面均为空，副本绑定保持中和，冻结产物仍拒绝修改，源绑定不变。
- 本轮不启动窗口，未调用真实模型/MCP/业务云；不把数据库回归等同GUI验收或全部历史读面已通。

## 上轮已验证基线（第84次，非本轮GUI新增验收）

- `a25eab90`修复脱敏生成历史列表：`read_listable_run`区分可解码/脱敏中和/真实损坏；中和占位按projectId过滤跳过，保留projectId的坏值报错；`open`脱敏先导运行拒绝复用。收据读面兼容NULL/空串，非空坏值仍拒绝。
- `183d4886`统一`frozen_row`为仓库注入缝∨连接级`_lorekeeper_history_frozen`；修复恢复副本关闭/再次打开时误解析冻结账本，相关JSON查询加`json_valid`前置。
- Rust真实合成副本：恢复副本list空、recover/suspend/再次打开成功；源副本list为4。证据`portable-history-read-fix-evidence.md`。
- Windows开发窗口（当轮临时前台授权已结束）：恢复→源→恢复→恢复连续打开成功，generation:list为0/4/0/0。恢复AI输出面板无错误、无可继续候选；源面板正常显示候选、预算与确认继续按钮。源4条run的read-context成功，篡改leaseId被拒。
- 窗口经标题栏正常关闭，Vite5190/合成端点19537及9226监听均已收口。本轮不启动服务，无子代理；默认后台、不置顶/激活/抢焦点，不使用全局键鼠/SendKeys。

## 接续与尚未解除的缺口

1. 查询当前SHA的云端38999595785；成功后下载核对validation.json及两份日志，才能更新全量基线/累计数。
2. 暂停/恢复GUI完整链、失效会话交错GUI仍待验收；启动前须有不抢焦点的后台方案，旧前台脚本不得原样复用。
3. 含冻结历史、脱敏审稿与资产的复杂归档，以及完整审稿→作者确认→修稿→差异合并→复核GUI尚未完成，四生产审稿频道继续关闭。
4. 更广历史读面（审修材料、图生成、导入推理等）仍需逐一验证；本轮仅补指定运行候选读面。定位看原始错误，不以map_generation_error收敛码掩盖根因。
5. 功能/业务计数不变；真实模型、外部MCP、业务云、发布未授权，macOS/Linux未认证，不宣称全链安全。

活动状态及本机证据入口见`.pi/tmp/pipeline-state.md`；本机临时日志、合成数据及凭据不入库。
