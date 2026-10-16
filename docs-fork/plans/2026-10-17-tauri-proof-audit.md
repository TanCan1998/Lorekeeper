# Tauri 首版证明与本机验收审计（第83次）

最终验证代码：`24291a43d878203cf9b44774bb564edc6ef2410a`。主会话有界源码/合成回归核查，不是独立安全审计，不代表生产审稿开放。范围与授权见[首版决策](./2026-10-17-tauri-completion-decisions.md)、[混合验证](../agents/tauri-rust-ci.md)及[当前快照](../handoffs/2026-10-17-tauri-migration-status.md)。

## 已核对的证明点

路径相对`tauri-app/src-tauri/src/`。表内审修回归在最终SHA的云端1631项内通过；审修服务57项另在本机定向执行。不把测试数量等同全部安全性质的证明。

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
5. **生成历史列表失败**：新副本generation:list返回Error: GENERATION_REQUEST_FAILED；只读SQLite观察到4个run的binding_json均已脱敏为空，而源项目均非空。初步定位为活动RunBinding解码与冻结历史读面衔接缺口，尚未修复/补回归，不能算历史恢复成功。
6. 原生选择曾发生导航/文件未找到提示，后续通过定向控件重新选择。结论依据最终业务收据与实际路径；目标位于本机ci-729cda50目录下的合成副本，不是脚本最初期望路径。不能把UIAutomation调用退出0直接当作成功。

本轮用户临时授权前台激活/定向交互已结束；测试窗口正常关闭，合成服务与Vite停止，恢复默认后台、不置顶/激活/抢焦点、不使用全局键鼠或SendKeys。后续脚本不得因本轮授权而原样复用。

## 未验收，不得扩大结论

- 新发现的便携恢复主生成历史列表错误优先处理；不得绕过解码或为脱敏历史补造重放权威。
- 完整审稿→作者确认→修稿→差异合并→复核GUI未验收，四生产审稿频道继续关闭。
- 暂停/恢复、失效会话交错GUI未完成；含冻结历史、脱敏审稿与资产的复杂归档未完成。两稿正文一致不能替代它们。
- 真实模型/外部MCP/业务云/发布未授权；macOS/Linux未认证，不宣称全链安全或发布认证。

## 证据绑定

- [最新云端38956708373](https://github.com/TanCan1998/Lorekeeper/actions/runs/38956708373)：最终SHA，1631/0、库221.99秒，all-targets成功无告警；artifact SHA一致，test/check退出0、state=passed。
- 中间SHA：6b2c5785 / 38950154610 / 1616；512e759f / 38954708710 / 1629；9c01fe45 / 38955678898 / 1630。均已下载核对，不混用版本。
- 本机`.pi/tmp/recovery/ci-{提交前缀}/`保存云端日志；artifact保存14天。未脱敏日志/用户数据不入库。
- 定向/前端：identity-directed、knowledge-*、kb-schema-*、composition-recovery-test、transport-*、final-83-*日志与结果JSON。
- GUI：transport-gui-cancel、transport-gui-archive-ui、transport-gui-copy、transport-gui-reopened截图/JSON；transport-archive-verification.json只记录合成身份/计数/哈希及历史失败事实。合成正文和本机凭据不入库。
