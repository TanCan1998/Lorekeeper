# Tauri 首版证明与本机验收审计（第82次）

本轮验证代码：`729cda50233747a29e2c46c7130498bdffa3bbdd`。这是主会话的有界源码/既有回归证据核查，不是独立安全审计，也不代表生产审稿开放。范围与授权见[首版决策](./2026-10-17-tauri-completion-decisions.md)和[混合验证](../agents/tauri-rust-ci.md)。

## 已核对的证明点

以下路径均相对`tauri-app/src-tauri/src/`；测试名见相应文件。相关既有测试实际包含在本轮云端1614项全量内，非新增或仅推断通过。审稿服务模块共55项通过；不把该数量当全部安全性质的证明。

| 核查点 | 源码入口 | 已执行回归与结论 |
|---|---|---|
| 初审、修稿、复核效果跨run/root唯一 | `repositories/review_cycle_repository.rs`：`has_unique_effect_claim`、`prove_review_effect`、`prove_revision_effect`；复核复用初审证明 | `services/review_revision_effect_tests.rs`：三项`effect_proof_rejects_duplicate_*_in_same_and_foreign_roots`均通过。全表计数不受当前root过滤限制，不接受另一真实run重复认领同一业务效果。 |
| root物理身份与JSON操作合同 | 同文件`verify_cycle_row`校验rootActionId及review-chapter；证明路径比较run-root | `effect_proof_binds_root_action_json_identity_and_operation`通过。不是仅凭有效JSON或自洽哈希接受。 |
| 多段修稿的物理attempt归属 | 同文件`prove_revision_effect`逐段查询artifact与attempt的run/root，拒绝重序、重复及缺段 | `services/review_revision_continuation_tests.rs`：`multipart_revision_rejects_attempt_owner_moved_to_another_run`、`multipart_revision_rejects_attempt_root_column_moved_to_another_root`及重序/重复回归均通过。 |
| 已知历史策略与报告版本 | `services/review_revision_generation.rs`：`context_of`、`saved`；仓储复核投影重建 | `services/review_revision_history_tests.rs`：已知组合只读恢复通过，`can_resume=false`且无新attempt；未知上下文、缺失/矛盾/降级策略均拒绝。 |
| 历史可选字段不能伪造缺省证据 | 同文件及仓储`verify_recheck_projection` | `historical_review_preserves_optional_field_contract_without_inventing_evidence`通过：真实可选值可为空，但删除实际必需的冻结原稿或已定位引文不能获证明。不是承诺任意缺字段历史兼容。 |
| 畸形报告不得panic或写入业务效果 | `parse_review_generation_result`、`review_body`、`saved`及事务提交路径 | `historical_review_invalid_initial_artifact_returns_error_without_business_writes`与`effect_proof_rejects_malformed_rehashed_review_artifact_without_panicking`通过，覆盖首次提交及已保存恢复。 |
| 脱敏历史保留可读而非重放权威 | 已有恢复/脱敏链 | `services/review_revision_chain_tests.rs`：`real_sqlite_chain_redacted_history_is_readable_but_cannot_replay`通过。不替代含审稿材料的真实归档GUI验证。 |

本轮只核查上述入口与对应回归，没有修改schema、权限、会话准入或生产证明算法。完整威胁枚举及跨project/epoch/artifact全链仍须后续逐项对照，不能据此宣称审计全部收口。

## 本机真实GUI已完成的范围

- Windows真实Tauri开发窗口，独立`AI_NOVEL_LOREKEEPER_HOME`与合成项目；前端为本机Vite，后端为同SHA的`cargo build --locked`开发二进制。不是发布包测试。
- 欢迎页旧库入口确为disabled，原因和替代路径实际显示；未触发旧库读写。
- 原生目录选择 → 新建合成项目 → 保存合成配置 → 原生保存对话框导出`.ainovel`存档。
- 原生归档选择与恢复目标目录授权 → 新副本恢复成功，原项目仍为当前项目，没有自动打开/替换；原/副本清单projectId不同。
- 通过原生项目选择器打开恢复副本，运行上下文指向副本且dbReady=true，合成配置一致。
- 标题栏正常关闭 → 重启 → 无自动打开项目，最近列表保留两条 → 从最近列表重开副本，配置仍一致 → 再正常关闭。
- 原生目录选择会先导航、再确认；结论以最终收据及实际路径为准，未把自动化调用成功或等待超时当业务成功。截图/JSON仅在`.pi/tmp/recovery/current-gui-*`，不入库。

## 仍未验收，不得扩大结论

1. 当前GUI项目没有章节、定稿或审修材料：未验收正文生成、完整审稿→作者确认→修稿→差异合并→复核，以及暂停/取消/恢复GUI。
2. 未通过GUI证明含冻结历史、脱敏审稿或资产的复杂归档恢复；本轮只覆盖新项目/配置级本地归档链。
3. 未在本轮GUI注入失效会话或做并发交错；既有Rust回归不替代这些交互场景。
4. 四生产审稿频道仍关闭；真实模型、外部MCP、业务云与发布未授权，macOS/Linux未认证。后续不得为GUI临时绕过频道门禁或放宽权限。

## 证据绑定

- [云端运行38946375676](https://github.com/TanCan1998/Lorekeeper/actions/runs/38946375676)：1614通过/0失败，库197.15秒；all-targets成功且无告警。artifact的expectedSha/testedSha均为本页SHA，两个退出码均0，state=passed。
- 云端证据本机副本：`.pi/tmp/recovery/ci-729cda50/`下validation.json、cargo-test.log、cargo-check.log；artifact保存14天，本机日志不入库。
- GUI关键证据：`current-gui-ui.json`、`current-gui-archive-result.json`、`current-gui-home.png`、`current-gui-created.png`、`current-gui-restored.png`、`current-gui-reopened-copy.png`，均在`.pi/tmp/recovery/`。
- 构建：`current-gui-frontend-build.log`、`current-gui-rust-build.log`。前端构建成功，有大chunk及构建计时提示；Rust开发构建11.57秒，无告警。
