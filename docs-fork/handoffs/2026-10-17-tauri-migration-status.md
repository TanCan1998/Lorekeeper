# Tauri迁移交接快照（2026-10-17）

> [当前入口](./CURRENT.md) · [混合验证](../agents/tauri-rust-ci.md) · [首版决策](../plans/2026-10-17-tauri-completion-decisions.md) · [证明/GUI审计矩阵](../plans/2026-10-17-tauri-proof-audit.md)。动态进度仅在docs-fork展示，同日改写当前摘要。

## 已验证状态（第86次）

当前代码：`e5562957aafd16090b33fe71d4afdc3c25c282ba`，master，已推送origin。后续为文档收口。

| 范围 | 结果/版本 | 证据 |
|---|---|---|
| 当前SHA云端 | Rust CI运行中，未登记全量通过 | [39000071204](https://github.com/TanCan1998/Lorekeeper/actions/runs/39000071204)；同SHA提交规范39000071191、Gitleaks39000071223最后查询运行中 |
| 最近成功Rust云端全量 | **1638通过/0失败**，库232.47秒，SHA=`61ab33bc` | [38999595785](https://github.com/TanCan1998/Lorekeeper/actions/runs/38999595785)，run completed/success；本机`.pi/tmp/recovery/ci-61ab33bc/` |
| 最近成功all-targets | 同61ab33bc成功无告警，35.97秒 | artifact expectedSha/testedSha一致、testExitCode/checkExitCode均0、state=passed；两日志逐项核对 |
| 本轮本机定向 | 导入仓储50/0、生成仓储30/0、便携74/0、`generation_owner`过滤187/0、run服务38/0、db命令30/0 | 本机`import-history-read-fix-evidence.md`；不相加冒充全量 |
| 本轮新增回归 | 实际脱敏导入历史：修复前1失败、修复后1通过，已包含在便携74项 | `sanitized_import_history_is_readable_but_not_listed_as_resumable_test` |
| 本轮编译/格式 | 本机`cargo check --all-targets --locked`通过无告警，4文件rustfmt与diff通过 | 因抽取跨模块冻结读函数追加检查；无持久schema/权限/租约规则改动 |
| 前端沿用 | 188文件2111通过/1跳过，typecheck/lint及上次build通过 | 本轮无前端改动，未重跑，不算当前新增验收 |
| 频道口径 | 注册291、契约注册290、事件7；功能289/290、业务285/290 | 本轮不变；四生产审稿频道继续关闭 |

最近全量基线更新为61ab33bc，**其后Rust语义切片累计1**；当前云端未完成，不清零。前一基线183d4886/38997082671为1637/0，已被替代。

## 本轮修复与有界结论

- 基线`electron/repositories/import-run-repository.ts::listResumable`会过滤`freeze.isFrozen('import_runs', row.id)`；Tauri读面漏迁，实际脱敏历史仍显示为可继续任务，尽管原连接触发器拒绝续写。
- `repositories/import_run_repository.rs::list_resumable`在构建快照前跳过冻结运行。`get`仍保留历史字段，不把历史`resumable`状态改写为当前执行权威；启动、租约、准入和权限规则均未改动。
- 将已有连接级冻结查询抽为`services/portable_project_guard.rs::frozen_row`，导入/生成读面共用；生成仓储包装继续保持注入缝∨连接守卫、短路与错误处理语义，不新增各自判空规则。
- 实际源库→脱敏副本→冻结侧车→连接守卫回归覆盖reference/author-manuscript两用途、ready/running/failed/cancelled四状态：冻结行从可继续列表排除；`get`可查、脱敏字段保持中和；原写保护拒绝start_or_resume；同连接新身份正常列举/启动，当前损坏checkpoint仍报错；源库不变。
- 本轮未启动GUI/服务、未派子代理，未调用真实模型/MCP/业务云。不将数据库合成回归等同GUI验收、全部历史读面已通或安全认证。

## 最近切片与GUI基线（非本轮新增GUI验收）

- 第85次`61ab33bc`：按指定运行读取候选先通过`frozen_row`跳过冻结运行，避免在过滤attempt前解析中和绑定/预算。实际脱敏回归覆盖全局/指定运行空候选、无守卫损坏拒绝、缺失运行拒绝、冻结产物拒写及源库不变。该版本现已云端全量通过。
- 第84次`a25eab90/183d4886`：修复脱敏生成列表及恢复副本关闭/再次打开失败；统一冻结事实来源并给相关JSON查询加`json_valid`前置。Rust真实合成副本list/recover/suspend/重开成功，源副本list=4。
- 第84次Windows开发窗口（当轮临时前台授权已结束）：恢复→源→恢复→恢复连续打开成功，generation:list为0/4/0/0。恢复AI面板无错误/无可继续候选；源面板显示候选、预算与确认继续。源4条read-context成功、篡改leaseId拒绝。证据`transport-gui-*`与`portable-history-read-fix-evidence.md`。
- 当轮窗口经标题栏正常关闭，Vite5190/合成端点19537及9226均收口；本轮无启动，无本机测试服务在飞。默认后台、不置顶/激活/抢焦点，不使用全局键鼠/SendKeys。

## 接续与尚未解除的缺口

1. 查询当前SHA云端39000071204；成功后下载核对validation.json及两份日志，才能更新全量基线/累计数。
2. 暂停/恢复GUI完整链、失效会话交错GUI仍待验收；启动前须有不抢焦点后台方案，旧前台脚本不得原样复用。
3. 含冻结历史、脱敏审稿与资产的复杂归档，以及完整审稿→作者确认→修稿→差异合并→复核GUI未完成，四生产审稿频道继续关闭。
4. 更广历史读面（审修材料、图生成、导入推理等）仍需逐一验证；本轮仅补导入任务列表，不等同导入推理完整链。定位看原始错误，不以map_generation_error收敛码掩盖根因。
5. 功能/业务计数不变；真实模型、外部MCP、业务云、发布未授权，macOS/Linux未认证。

活动状态及本机证据入口见`.pi/tmp/pipeline-state.md`；临时日志、合成数据及凭据不入库。
