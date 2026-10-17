# Tauri 首版证明与本机验收审计（第86次）

上轮全量验证代码：`183d488633dc26d264b12b68bfc2d0640ddfb1f5`（上一版为24291a43）。主会话有界源码/合成回归核查，不是独立安全审计，不代表生产审稿开放。范围与授权见[首版决策](./2026-10-17-tauri-completion-decisions.md)、[混合验证](../agents/tauri-rust-ci.md)及[当前快照](../handoffs/2026-10-17-tauri-migration-status.md)。

## 第86次：冻结导入任务列表

- 当前代码`e5562957aafd16090b33fe71d4afdc3c25c282ba`已推送；[云端39000071204](https://github.com/TanCan1998/Lorekeeper/actions/runs/39000071204)待完成。最新成功全量已更新为61ab33bc/38999595785：1638/0、all-targets无告警，run成功且artifact SHA/两个退出码/state已核对；其后语义切片累计1。下文183d4886证明点仍是历史绑定，不冒充当前版全量。
- 对齐基线listResumable的冻结过滤，历史get仍可查看；抽取连接级frozen_row共用查询，生成仓储继续保持注入缝取或，不改触发器/租约/准入/权限。
- 实际脱敏回归先失败后通过：两用途×四状态冻结任务从可继续列表排除，get可查且字段保持中和、原写保护拒绝启动；新身份正常列举/启动，当前损坏checkpoint拒绝，源库不变。
- 本机导入仓储50/0、生成仓储30/0、便携74/0、generation_owner过滤187/0、run服务38/0、db命令30/0；4文件rustfmt/diff及本机all-targets通过无告警。未启动GUI，不扩大旧GUI结论。

## 第85次：指定运行候选读面

- 代码`61ab33bc822f3864cc2b6c309aa30d265eacbb1b`已推送；[云端38999595785](https://github.com/TanCan1998/Lorekeeper/actions/runs/38999595785)现已成功，1638/0及all-targets无告警，artifact已核对。不能沿用下表183d4886的旧证据冒充新版实测。
- `GenerationRunRepository::list_candidates(Some(run_id))`在解析绑定/预算前用`frozen_row`跳过冻结运行，修复实际脱敏副本的JSON解析错误。
- 新增实际源库→脱敏→冻结侧车→连接守卫→候选读取回归，先失败后通过；覆盖全局/指定运行空候选、无守卫损坏拒绝、缺失运行拒绝、冻结产物拒写与源库不变。
- 本机仓储30/0、generation_owner过滤187/0、便携73/0、run服务38/0及两文件rustfmt/diff通过。本轮未启动GUI，不扩大下文上轮GUI结论。

## 已核对的证明点

路径相对`tauri-app/src-tauri/src/`。表内审修回归在最终SHA的云端1637项内通过；审修服务59项另在本机定向执行。不把测试数量等同全部安全性质的证明。

| 核查点 | 源码入口 | 已执行回归与有界结论 |
|---|---|---|
| 初审、修稿、复核效果跨run/root唯一 | `repositories/review_cycle_repository.rs`：has_unique_effect_claim、prove_review_effect、prove_revision_effect | 三项effect_proof_rejects_duplicate_*_in_same_and_foreign_roots通过；另一真实run不能重复认领同一业务效果。 |
| root物理身份与JSON操作 | 同文件verify_cycle_row及run-root证明 | effect_proof_binds_root_action_json_identity_and_operation通过；不只凭有效JSON或自洽哈希接受。 |
| 多段修稿attempt物理归属 | 同文件逐段查询run/root及顺序 | review_revision_continuation_tests中的owner/root错配、重序、重复、缺段回归通过。 |
| project/epoch/artifact字段错配（本轮新增） | `services/review_revision_effect_tests.rs` | effect_proof_rejects_project_epoch_and_artifact_identity_mismatches：初审/修稿/复核真实SQLite链36组单字段篡改拒绝；不宣称穷尽所有组合。 |
| 合法恢复租约与历史产物epoch（本轮新增） | 同文件 | effect_proof_preserves_historical_artifact_epoch_after_legitimate_resume：提交前暂停，以新epoch恢复，历史artifact保留生成时epoch仍合法；不能要求所有epoch一律相等。 |
| 已知历史策略与报告版本 | `services/review_revision_generation.rs`的context_of、saved与仓储复核投影 | 已知组合只读恢复，can_resume=false且无新attempt；未知、缺失、矛盾、降级策略拒绝。 |
| 历史可选字段不伪造证据 | 同服务与verify_recheck_projection | historical_review_preserves_optional_field_contract_without_inventing_evidence通过；不承诺任意缺字段兼容。 |
| 畸形报告不panic、不写业务效果 | parse_review_generation_result、review_body、saved与提交事务 | historical_review_invalid_initial_artifact_returns_error_without_business_writes和malformed_rehashed回归通过。 |
| 脱敏审修历史只读不可重放 | `services/review_revision_chain_tests.rs` | real_sqlite_chain_redacted_history_is_readable_but_cannot_replay通过；**不等于当前GUI便携恢复后的主生成列表正常**，见下方缺口。 |

完整威胁枚举仍须继续核对，未开放生产证明写面。

## 本轮生成知识与接线回归

- 用户具体授权Tauri知识修订schema升级；`db/kb/revisions.rs`区分规范/派生，涵盖幂等升级、回滚、外层事务、双连接已提交可见性、缺行与溢出拒绝。旧便携schema可升级，Electron及旧库不读不改。
- `services/generation_knowledge_source.rs`真实捕获修订、规范文档/块身份、正文与整文档块集合；空结果也绑定修订。覆盖reference/脏副本排除、向量ID规范错配、缺块/重复ID/畸形快照及未选块变化。
- owner的prepared_knowledge_drift_rejects_begin_and_execution_without_new_effects验证拒绝旧准备、不新增run/lease/attempt；commands::kb等待源锁后失效会话拒绝。网络阶段不持有owner/DB/知识锁，返回重新认证。
- generation:read-context和read-batch请求包装已对齐；裸句柄合同保持不变。前端映射与Rust解码拒绝错误包装回归通过。
- read_recovery_context将composition末尾artifact按当前run物理归属解析attempt，再读finishReason；stop/length回归通过，恢复读取不提前提交草稿。

## 本机真实GUI已完成范围

仅Windows真实Tauri**开发窗口**、独立home/合成项目、本机Vite及127.0.0.1合成HTTP/SSE端点，无真实模型或外部业务调用。

1. 沿用第82次已验证的旧库禁用说明、新建/配置保存、配置级归档导出/独立副本恢复、正常关闭重启后重开；前轮代码729cda50，不冒充最终代码新增实测。
2. 本轮补合成蓝图、角色和本机模型配置；最终代码实际完成知识准备、短细纲、正文流式、合成收据及草稿保存。界面显示617字、流程完成；未点击定稿/审稿。
3. 另一次流式请求中点击“中止生成”，界面显示取消、账本status=cancelled，草稿数保持2，不将残片保存成完成草稿。不是暂停/恢复验收。
4. 原生授权导出含两稿及四个生成run的归档，恢复为新projectId，不替换当前项目。打开新副本，两稿章节号/版本/状态/正文SHA256均一致；正常关闭重启后从最近列表重开，617字仍可见。
5. **生成历史列表失败（已修复）**：新副本`generation:list`曾返回Error: GENERATION_REQUEST_FAILED；只读SQLite观察到4个run的binding_json均已脱敏为空，而源项目均非空。
6. **随后暴露的第二处缺陷（已修复）**：离开并再次打开同一恢复副本时弹出「打开项目失败：项目数据库初始化失败：Error: GENERATION_REQUEST_FAILED」。Rust级复现定位到`suspend_for_project_close → recover_interrupted → frozen_root → author_edited_root → json_type('')`抛malformed JSON：生产冻结只体现在连接级TEMP触发器，仓库`is_frozen`注入缝恒false，脱敏历史被当可变历史解析。修复为`frozen_row`（注入缝∨连接守卫函数）与`json_valid`前置；两项均有合成回归与真实副本端到端证据。详见本机`portable-history-read-fix-evidence.md`。
7. 修复后的GUI复验：恢复副本→源副本→恢复副本→恢复副本连续切换全部打开成功，`generation:list`为0/4/0/0，无「打开项目失败」与 GENERATION_REQUEST_FAILED；恢复副本AI输出面板挂载且无错误文本、无"部分任务暂时无法读取"、无可继续候选（脱敏历史只读）；同一面板在源副本显示"第1章候选"、预算诊断、已用2/3次请求与"确认继续已选正文"；源副本4条运行`generation:read-context`全部成功；篡改leaseId后同一句柄被拒（失效会话交错IPC层）。
8. 原生选择曾发生导航/文件未找到提示，后续通过定向控件重新选择。结论依据最终业务收据与实际路径；目标位于本机ci-729cda50目录下的合成副本，不是脚本最初期望路径。不能把UIAutomation调用退出0直接当作成功。

本轮用户临时授权前台激活/定向交互已结束；测试窗口正常关闭，合成服务与Vite停止，恢复默认后台、不置顶/激活/抢焦点、不使用全局键鼠或SendKeys。后续脚本不得因本轮授权而原样复用。

## 未验收，不得扩大结论

- 便携恢复的两处缺陷已修复并有回归，但同类更广读面（审修材料、图生成、导入推理等）尚未逐一在恢复副本上验证；不得据两处修复推断全部便携读面已通。
- 完整审稿→作者确认→修稿→差异合并→复核GUI未验收，四生产审稿频道继续关闭。
- 暂停/恢复、失效会话交错GUI未完成；含冻结历史、脱敏审稿与资产的复杂归档未完成。两稿正文一致不能替代它们。
- 真实模型/外部MCP/业务云/发布未授权；macOS/Linux未认证，不宣称全链安全或发布认证。

## 证据绑定

- [最新云端38997082671](https://github.com/TanCan1998/Lorekeeper/actions/runs/38997082671)：最终SHA=183d4886，1637/0、库227.11秒，all-targets成功无告警；artifact的expectedSha/testedSha一致，test/check退出0、state=passed。
- 中间SHA：6b2c5785 / 38950154610 / 1616；512e759f / 38954708710 / 1629；9c01fe45 / 38955678898 / 1630。均已下载核对，不混用版本。
- 本机`.pi/tmp/recovery/ci-{提交前缀}/`保存云端日志；artifact保存14天。未脱敏日志/用户数据不入库。
- 定向/前端：identity-directed、knowledge-*、kb-schema-*、composition-recovery-test、transport-*、final-83-*日志与结果JSON。
- GUI：transport-gui-cancel、transport-gui-archive-ui、transport-gui-copy、transport-gui-reopened截图/JSON；transport-archive-verification.json只记录合成身份/计数/哈希及历史失败事实。合成正文和本机凭据不入库。
