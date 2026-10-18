# Tauri迁移交接快照（2026-10-18）

> [当前入口](./CURRENT.md) · [混合验证](../agents/tauri-rust-ci.md) · [证明审计](../plans/2026-10-17-tauri-proof-audit.md) · [频道清单](../plans/tauri-migration-channel-inventory.md)。第 **99** 次交接；同日第95次内容可在提交4646837b查看，上一日快照为[第94次](./2026-10-17-tauri-migration-status.md)。

## 当前代码与已验证状态

代码 **7360f4f67f2c80e47839521199470367ecc6ca2c**，master，已推送。云端39098085472成功且产物已核验；前片a9f28824/39097717930被新提交取消，不计独立全量通过，最新SHA全量覆盖累计两片。第99次未启动GUI或调用真实模型。

| 验证 | 结果 | 版本/时间/证据 |
|---|---|---|
| 最新云端全量（已下载核验） | 1687通过/0失败；库175.47秒；all-targets 31.08秒无告警 | 7360f4f6 / [39098085472](https://github.com/TanCan1998/Lorekeeper/actions/runs/39098085472)，2026-10-18 04:50 UTC完成 |
| 第98次云端全量 | 1685通过/0失败；库265.84秒；all-targets 42.05秒无告警 | e093d7d4 / [39096912662](https://github.com/TanCan1998/Lorekeeper/actions/runs/39096912662) |
| 前片云端全量 | 1684通过/0失败；库269.87秒；all-targets 35.45秒无告警 | 39c1c885 / [39095854001](https://github.com/TanCan1998/Lorekeeper/actions/runs/39095854001) |
| 前片云端全量 | 1682通过/0失败；库255.35秒；all-targets 36.92秒无告警 | 561e91d0 / [39094795752](https://github.com/TanCan1998/Lorekeeper/actions/runs/39094795752) |
| 前片云端全量 | 1681通过/0失败；库152.81秒；all-targets 32.06秒无告警 | d709125a9af5a2ed242b7f05c56084dc9ab33526 / [39093896169](https://github.com/TanCan1998/Lorekeeper/actions/runs/39093896169)，2026-10-18 |
| 本机最新定向 | legacy_roster 28/0；portable 90/0；generation_owner 213/0；定向rustfmt/diff通过 | 7360f4f6；legacy-nonce-*日志 |
| 本轮前片定向 | character_proposal 12/0；portable 90/0；generation_owner 212/0 | a9f28824；character-import-*日志 |
| 第98次定向 | agent_generation 57/0；portable 89/0；generation_owner 212/0 | e093d7d4；agent-registration-*日志 |
| 本机前片定向 | graph_generation 23/0；portable 88/0；legacy_roster 27/0；main_generation_owner 211/0 | 39c1c885；read-proof-*日志，过滤范围不同不混用 |
| 本机前片审修定向 | review_revision 70/0；portable 87/0 | 561e91d0；typed-deps-final-*日志，本轮未重跑审修 |
| 本机前片定向 | owner 218/0；review_revision 69/0；generation_run_repository 30/0；portable 87/0 | d709125a，2026-10-18；old-handle-*日志 |
| 本机最近全量lib | 1681/0，74.15秒 | 6b8108dc提交前相同代码；本轮未在最新SHA重跑本机全量 |
| 前端 | 本轮未改；未重跑TS | 上次Tauri前端全量190文件2121通过/1跳过及typecheck/lint：8ecfc0fb |
| 提交规范/秘密扫描 | success | 7360f4f6：39098085516 / 39098085462 |
| 双栈隔离 | 相对4646837b未修改根配置/lock、electron、src或docs | 本轮git路径差异检查 |

已下载并核验的独立云端证据（完整expectedSha/testedSha匹配、两个独立退出码0、state=passed，lock哈希匹配，两日志无告警）：

- 51f3e89f9a6ec561b67cac5e959f696ab8fe0e87 / 39092294785：1680/0，库267.38秒，check 42.05秒。
- 6b8108dc09cbbe3b2ecb6db5d6d56080d3ab9ebc / 39092830893：1681/0，库261.51秒，check 38.63秒。
- d709125a9af5a2ed242b7f05c56084dc9ab33526 / 39093896169：1681/0，库152.81秒，check 32.06秒。
- 561e91d092bfda67c3f1050fda13a499598cc70c / 39094795752：1682/0，库255.35秒，check 36.92秒。

- 39c1c885605bae966a07b8a1e061f5fc3fe8c5fa / 39095854001：1684/0，库269.87秒，check35.45秒；产物31587字节、GitHub公布SHA256匹配，validation完整SHA/lock/退出码0/state=passed均已核对。

- e093d7d4533df2a24607224b7e8896835b531cbd / 39096912662：1685/0，库265.84秒，check42.05秒；产物31573字节及GitHub SHA256匹配，validation完整SHA/lock/两个退出0/state=passed已核对。

- 7360f4f67f2c80e47839521199470367ecc6ca2c / 39098085472：1687/0，库175.47秒，check31.08秒；产物31554字节及GitHub SHA256匹配，完整SHA/lock/两个退出0/state=passed与两日志已核验。

按“最近已核验全量后”口径，Rust语义切片当前累计 **0**（7360f4f6全量证据已核验）。

## 第99次：持续迁移与两处相邻读面

- 用户要求普通切片持续推进，不逐片停下等“继续”；不恢复代理派发，不取消schema/依赖/定稿/权限/准入等具体确认边界。
- **a9f28824**：角色提案import分支整库JSON1筛选payload_json，被冻结中和值阻塞；实际sanitizer回归先失败后改逐行排除认证冻结run/receipt。当前坏JSON/匹配收据哈希损坏仍拒绝，缺生成句柄与重复效果不伪造来源，源文件不变、旧行拒写。此为合成导入事实证明，不是完整生成导入或GUI链。
- **7360f4f6**：旧名册nonce扫描吞坏action/binding后误报冲突，null上下文亦误匹配。先失败回归后严格解析当前JSON、跳过认证冻结根/run，null与缺省不匹配，多个非空上下文仍GENERATION_NONCE_CONFLICT。模型不可用基线仍拒绝，无账本副作用、旧行拒写、源文件不变。
- 旧名册回归最初以MODEL_MISSING作为基线，实际映射为REQUEST_FAILED；已调整夹具，通过重复匹配上下文与无关坏行区分解析错误和nonce冲突，不为测试修改生产错误映射。历史错误日志不当作目标缺陷已复现。
- 本轮无schema/依赖/权限/会话准入/CI/前端改动。三文件定向rustfmt/diff均通过。
- **具体确认边界**：静态对照发现finalization_generation::read_finalization_effect吞坏JSON/畸形SavedEffect，基线JSON.parse及后续证明会拒绝；文件涉及定稿后处理，尚未改动或新增回归，须用户具体确认后再修，不把持续迁移泛授权当作豁免。
- GUI准备仅审阅旧隐藏配置/原生控件脚本及当前归档对话框代码；未启动窗口或对话框。原生save/pick创建阶段无法保证不激活，不因旧UIAutomation脚本未调用SetFocus就宣称全程不抢焦点。

## 第98次：代理工作流登记逐行过滤冻结历史

- workflow_registration仍整库SQL json_extract(binding_json)，真实sanitizer副本先失败复现malformed JSON。现复用既有frozen_aware_json_pointer_rows：只跳过认证冻结行，操作/project/登记唯一性及子代来源校验不变。
- 新回归在实际sanitizer/冻结侧车/连接守卫上，历史不提供登记（MISSING），读取不回写；同库加入当前父轮/子代后登记可读，with_child_effect完成合成效应，transition.actionId正确，冻结中和值未变且仍拒写、源库逐字节不变。
- 使用既有TestHost合成宿主和真实SQLite服务，不等于完整生产工作流、真实模型或GUI验收。无守卫中和值/当前非冻结坏绑定仍拒绝。
- 初轮修复后测试曾只因硬编码SQLite的malformed JSON文本失败：共用读面改用serde返回解析错误。测试调整为损坏错误不得变成MISSING/FROZEN，不为匹配旧文字放松生产门禁。
- 仅改agent_generation.rs；无schema/依赖/权限/会话准入/CI配置/前端变化。其他JSON消费路径尚未验收。

## 第97次：图效果回执严格校验与专用冻结入口

- 对照最新Electron发现stored_effects及事务版stored_effects_in将坏JSON、graphEffects:null吞成无效果。新增回归先失败（unwrap_err收到Ok([])），再修复为GENERATION_GRAPH_EFFECT_INVALID；缺省效果及合法[]仍可读。
- 合成SQLite证明回归覆盖SQL NULL、空串、坏JSON、JSON null、非数组、坏效果结构及超过8项；普通读面、assert_mutable及事务内读面均拒绝，不回写收据。不是所有效果证明字段的穷尽测试。
- 实际sanitizer→冻结侧车→连接守卫回归：图/旧名册require与confirm/stage先精确拒绝PORTABLE_RUNTIME_FROZEN，不进入来源回调；无写入、事务退出、旧attempt拒写、源文件逐字节不变。
- 无守卫中和值及同一恢复连接新增的坏绑定仍按损坏拒绝，不因恢复连接全局豁免。夹具绑定刻意中和，证明入口拒绝顺序；不冒充有效当前旧角色完整证明链。
- 未改schema、依赖、权限、会话准入或CI配置；本轮无前端变化，无新增GUI验收。

## 第96次：恢复副本继续创作与精确冻结拒绝

1. **51f3e89f：可空回执读取修复。** 审修准备扫描generation_attempts时，把可空usage_receipt_json当String读取，脱敏NULL导致Invalid column type Null。改为Option读取，NULL不产生执行收据声明。
2. **6b8108dc：用户明确选择收窄冻结范围。** 作者内容表drafts、character_relationships不再登记行冻结键；R列仍中和，不迁出原始依赖/来源。执行历史表、定稿出箱等仍冻结，未放宽定稿不可逆、文件授权或生命周期。恢复副本可编辑作者草稿。
3. **恢复副本审修全链实跑（SQLite集成测试）。** 在真实sanitizer副本和连接守卫上：审稿→作者确认（文风豁免/目标应用）→修稿→合并→定向复核，recheck_count=1且全局严格验证通过；冻结历史无候选、不可改写。此为合成后端集成链，**不是新增GUI或真实模型验收**。
4. **d709125a：旧handle精确拒绝。** get/budget在解析中和JSON之前检查认证冻结键；owner只额外透传精确PORTABLE_RUNTIME_FROZEN，不开放整个PORTABLE前缀。旧read_run/resume/read_recovery返回Error: PORTABLE_RUNTIME_FROZEN；来源/候选回调不执行。未认证中和值仍按JSON损坏拒绝，缺失身份仍保留原missing错误。
5. **561e91d0：收紧空依赖豁免。** 复验发现6b8108dc仅凭恢复连接身份允许空串，不能区分恢复后新增/损坏的当前草稿。先失败回归后，sanitizer将drafts.source_dependencies中和为合法空数组[]，移除is_restored_copy整库豁免。普通项目、恢复副本可变旧草稿、新增草稿的空串/空白/坏JSON都拒绝；源库不变。旧空串只对认证冻结草稿行兼容，且仍只读；该单元守卫缝**不等于任意旧归档安装兼容性验收**。
6. 中间失败保留为历史：旧handle回归先复现JSON解析错误；当前空依赖回归先复现被误放行；新增草稿夹具曾先改变最新版本身份，使旧稿先被来源门禁拒绝，已将对照顺序修正，不降低生产门禁。

## 可空R列扫描与剩余读面

按当前schema解析（逗号分隔列，排除NOT NULL/主键）共13个可空R列：

| 列族 | 核对结论 |
|---|---|
| blueprint_character_sync_operations.completion_receipt | 内部行类型Option<String> |
| import_run_receipts.effect_receipt_json | 内部行类型Option<String>；冻结effect读面不发布当前权威 |
| post_process_steps.error_msg | Option读取后默认值 |
| llm_calls的9个可空列 | 文本Option后默认；token/duration保持Option；success Option数值后布尔映射；id为保留主键，不是可空待中和字段 |
| generation_attempts.usage_receipt_json | 审修全库扫描NULL缺陷已修；generation_draft_effects/finalization/agent相关读点有Option处理；图stored_effects两处和旧角色saved仍有严格String读点 |

图/旧角色这类**证明阶段**不能统一改为吞NULL/坏JSON；最新上游对应逻辑同样直接JSON.parse后访问效果。第97次已修图普通/事务读面吞坏JSON与null效果，并证明两域外部require在解析脱敏收据前拒绝冻结历史；旧角色saved保留严格String/JSON读取。生成绑定查询、恢复候选、角色提案payload等其余JSON读面仍需继续核对，不把静态扫描或入口拒绝回归算成所有完整证明链验收。

## 沿用的已完成任务与历史GUI范围

- #25–29登记完成；图plan/event/plot真实确认、行内应用/选区漂移拒绝、新租约图/行内恢复、完整审修四频道开放均沿用第91次证据（非本轮新测）。
- 复杂头像/审修原生归档曾验证19正文、2稿、5修订、12报告、6周期、7finding与头像SHA一致，恢复副本UI可读；新冻结范围与新归档中和值尚未做新版GUI复验。
- #30仍进行中，#31最终验收未开始。测试通过不等于全部便携读面、所有交错或多平台已认证。

## 第95次同日决策保留

- 上游787提交（至f1d0bdb8）通过临时分支合并为051c8c83，仅.gitignore冲突；Tauri树不变。根typecheck 30.3秒、lint 64.5秒实际在本地合并后、推送前执行（纠正旧摘要“合并前”措辞）。
- 4ac2dd4e加入新增上游贡献者提交豁免，e6d03172修复该修改的YAML缩进；历史失败不能记为成功。
- 不剥离master，不保留常驻同步分支；本机.pi/tmp/sparse.ps1 on|off|status切换可见树，保留完整Electron参照；不删除target。操作见[项目参考](../agents/project-reference.md)。
- cargo-nextest不作为默认验证；libtest/CI证据契约不变。Rust环境使用D盘会话脚本cargo-fast.ps1。

## 证据与下轮断点

- 本轮证据：.pi/tmp/character-import-{red,character_proposal,portable,generation_owner}.log、legacy-nonce-{red,legacy_roster,portable,generation_owner}.log；云端.pi/tmp/recovery/ci-7360f4f6/；原包D:/Temp/lorekeeper-ci-7360f4f6/evidence.zip。
- 第98次证据：.pi/tmp/agent-registration-*.log；云端.pi/tmp/recovery/ci-e093d7d4/。
- 第97次证据：.pi/tmp/graph-receipt-red.log（UTF-16）、read-proof-{graph_generation,portable,legacy_roster,main_generation_owner}.log；云端.pi/tmp/recovery/ci-39c1c885/。
- 前片证据：.pi/tmp/recovery/ci-{51f3e89f,6b8108dc,d709125a,561e91d0}/；old-handle-*.log、blank-deps-red.log、typed-deps-final-*.log；均不入库。
- 活动事实源：.pi/tmp/pipeline-state.md；GUI历史位于background-1792229720030，PID仅历史，不视为当前运行。
- 下一步：① 定稿后处理效果读面修复须具体确认，再继续其余JSON消费路径有界回归；② 新归档规则与恢复后创作的后台GUI验收；③ #30有界收口后再做#31，不能把后端全量当成GUI验收。
- 当前没有新GUI/mock/Vite或子代理运行；没有新原生文件授权操作。真实模型、外部MCP、业务云、发布、自动定稿仍未授权；macOS/Linux未认证。
