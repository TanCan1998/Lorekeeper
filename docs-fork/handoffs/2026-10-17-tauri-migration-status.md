# Tauri迁移交接快照（2026-10-17）

> [当前入口](./CURRENT.md) · [混合验证](../agents/tauri-rust-ci.md) · [首版决策](../plans/2026-10-17-tauri-completion-decisions.md) · [证明/GUI审计矩阵](../plans/2026-10-17-tauri-proof-audit.md)。动态进度仅在docs-fork展示。同日改写当前摘要，不追加多份。

## 已验证状态（第84次）

代码版本：`183d488633dc26d264b12b68bfc2d0640ddfb1f5`，master，已推送origin。之后收口仅文档。验证日期2026-10-17（云端UTC日志2026-10-16/17）。

| 范围 | 结果/版本 | 证据 |
|---|---|---|
| 最新Rust云端全量 | **1637通过/0失败**，库227.11秒，同上SHA | [run38997082671](https://github.com/TanCan1998/Lorekeeper/actions/runs/38997082671)；本机`.pi/tmp/recovery/ci-183d4886/` |
| 云端all-targets | 成功、无告警；expectedSha/testedSha一致，testExitCode/checkExitCode均0，state=passed | 同run artifact的validation.json、cargo-test.log、cargo-check.log逐项核对 |
| 云端提交规范/秘密扫描 | 同SHA均成功 | Commit message CI 38997082846、Gitleaks 38997082672 |
| 本轮Rust定向 | owner域186/0、仓储域30/0、审修59/0、便携72/0、数据库120/0、命令4/0、run服务38/0、知识源6/0、角色提案9/0 | `portable-history-read-fix-evidence.md`；按各切片执行，不相加冒充全量 |
| 前端 | 188文件、2111通过/1跳过；typecheck/lint通过；上次build通过（本轮无前端改动） | transport-test.log、transport-typecheck.log、transport-lint.log、final-83-build.log |
| 格式/隔离 | 本轮2个Rust文件rustfmt、diff检查通过；根src/Electron/依赖锁及docs镜像无变更；AGENTS仍忽略 | 本机rustfmt/diff输出；git差异 |
| GUI（真实Windows开发窗口） | 恢复副本可重复打开；AI输出面板无错误；源副本候选面板正常；失效会话被拒 | transport-gui-close-cycle.*、transport-gui-restored-ai-output.*、transport-gui-ai-output.*、transport-gui-resume-and-session.* |
| 频道口径 | 注册291、契约注册290、事件7；功能迁移289/290、业务开放285/290，不变 | final-83-channels.log；blocked不计功能覆盖 |

本机证据均在`.pi/tmp/recovery/`，不入库。最新全量基线=183d4886，**其后Rust语义切片累计0**。前一个全量基线24291a43为1631/0，中间a25eab90已被183d4886取代，不单独登记。

## 本次更新

- `a25eab90`：`read_listable_run` 让列表读面逐行区分可解码、脱敏中和占位/外域与真实损坏；中和占位按基线 `projectId` 过滤语义跳过，保留projectId的坏值仍报错。`open` 复用分支命中脱敏先导运行时报 `GENERATION_NONCE_CONFLICT`，不复用/重放。`query_usage_receipts` 兼容基线NULL与空串，`list_directory_progress` 跳过中和收据但非空坏值仍 fail-closed。
- `183d4886`：新增 `frozen_row`，把仓库的 `is_frozen` 注入缝与连接上的便携守卫标量函数 `_lorekeeper_history_frozen` 取或；`frozen_root`、`read_run_view`、`list_candidates` 全部改用它。`author_edited_root`/`is_private_planning_history` 在 `json_type`/`json_extract` 前加 `json_valid`。
- 根因（本轮实测）：生产冻结只体现在连接级TEMP触发器，仓库注入缝默认恒 false，脱敏历史被当作可变历史解析——`recover_interrupted → frozen_root → author_edited_root → json_type('')` 抛 malformed JSON，被 `map_generation_error` 收敛成 GENERATION_REQUEST_FAILED，用户看到「打开项目失败：项目数据库初始化失败」。
- 真实副本端到端（Rust侧）：恢复副本 f41cb30b 修复前 `list`/`suspend` 均报错；修复后 `list`=Ok([])、`recover_interrupted`=Ok、`suspend_with_database`=Ok、再次打开=Ok；源副本 80941c1d `list`=Ok(4) 无回归。
- GUI（用户当轮临时前台授权，仅隔离合成项目）：恢复副本→源副本→恢复副本→恢复副本 连续切换全部打开成功，`generation:list` 为 0/4/0/0，全程无「打开项目失败」与 GENERATION_REQUEST_FAILED；这正是修复前失败的那一步。恢复副本的AI输出面板已挂载且无错误文本、无可继续候选；同面板在源副本显示"第1章候选"、预算诊断、已用2/3次请求与"确认继续已选正文"，证明面板本身可用。源副本4条运行的 `generation:read-context` 全部成功。篡改leaseId后同一句柄被拒。

## 真实窗口与授权收口

本轮用户通过确认卡再次临时授权前台激活和定向GUI交互，仅限隔离合成项目；未读真实项目、未调用真实模型/外部MCP/业务云，未使用全局键鼠或SendKeys。Windows开发窗口不等于发布包验收。

测试窗口经标题栏"关闭"正常结束，Lorekeeper进程与9226监听已核实消失。**本轮临时前台授权结束，恢复默认后台、不置顶、不激活、不抢焦点规则。**

## 尚未解除的验收缺口

1. 暂停/恢复（batch暂停按钮完整链）与失效会话交错仅完成IPC层拒绝验证，尚未完成GUI端到端暂停/恢复。含冻结历史、脱敏审稿与资产的复杂归档仍未完成。
2. 完整"审稿→作者确认→修稿→差异合并→复核"GUI未验收；四个生产审稿频道继续关闭，不为验收绕过门禁。
3. project/epoch/artifact已有有界回归，但威胁枚举未穷尽，不是全链安全认证。真实模型、外部MCP、业务云与发布未授权；macOS/Linux未认证。

## 交接给下次会话

- master；代码HEAD/origin=183d4886，随后仅本轮文档收口。活动队列与本机证据入口见`.pi/tmp/pipeline-state.md`；无子代理、无测试服务在飞。
- 下一步优先级：①暂停/恢复GUI链；②含脱敏审稿/资产复杂归档；③继续按审计矩阵推进（同域先补合成回归，错误码不得靠 `map_generation_error` 掩盖，定位时先用原始错误）。GUI需后台安全方案或新一轮明确临时授权。
- 便携只读边界继续成立：脱敏历史不进入可列举/可继续/可重放读面；`frozen_row` 是新的统一入口，新增写路径应复用而不是各自判空。
