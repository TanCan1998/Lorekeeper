# Tauri 首版证明与本机验收审计（第101次）

当前全量验证代码：`b5850fbed96495e0a1e36ada51c4de9ba449765f` / 39102768655，1690/0，库196.58秒，all-targets35.54秒无告警；完整SHA/lock、两个退出码0、state=passed及两日志已核验。下文保留历史证明版本与GUI范围，不混用实测；当前开放边界以最新快照为准。主会话有界源码/合成回归核查，不是独立安全审计，不代表发布认证。范围与授权见[首版决策](./2026-10-17-tauri-completion-decisions.md)、[混合验证](../agents/tauri-rust-ci.md)及[当前快照](../handoffs/2026-10-18-tauri-migration-status.md)。

## 第101次：定稿后处理、血缘查找与新规则恢复GUI

- 用户对明确列出的剩余项回复“全部进行”，本次仅据此纳入已说明的定稿后处理读面修复；不改正文/不可逆/schema/权限/生命周期，不追溯旧取消，不放宽后台或外部业务限制。
- 58994666两后处理分支先失败回归后修read_finalization_effect吞坏JSON/畸形SavedEffect；SQL NULL及基线假值行为保留，可变性判断拒绝坏回执且无回写。云端39102267581独立1689/0、check37.99秒无告警。
- b5850fbe修parent确认查找吞其他当前根坏JSON；共用私有读面先过滤认证冻结attempt/run、再严格解析，保留声明唯一性和完整证明。复核原REVIEW_CYCLE_CONFLICT优先级保持。定向review_revision72/0、portable90/0、owner213/0；前片finalization26/0。
- b5850fbe/39102768655独立1690/0、库196.58秒、check35.54秒无告警；两片artifact大小/SHA256、完整expected/tested SHA、lock、两退出0、state=passed及日志已核对，累计0。两片代码规范/秘密扫描成功。
- 当前库正式归档服务准备新规则合成副本，隐藏真实WebView实际编辑保存→项目重开→应用正常退出重启，持久化、旧lease拒绝、定稿不变、零恢复候选通过；正式连接33/33历史run冻结、1个可变稿依赖[]、历史拒写无变化。
- Tauri隐藏/未聚焦及Win32非前台已核对；无原生对话框、无全局输入。服务准备不冒充完整命令层授权/原生对话框验收。窗口/Vite已关闭，默认构建恢复。
- 详细证据及辅助程序初期失败见当前快照；#30原生对话框仍受不抢焦点约束，#31不提前登记完成，不扩大为所有消费者/交错/真实模型/发布验收。

## 第100次：普通审修回执解析

- 定稿修复确认卡取消，不作授权；未修改该文件。继续普通审修，代码1cf260b4已推送，云端39100710833成功1688/0、check38.70秒无告警；产物31652字节及GitHub SHA256匹配，完整SHA/lock/两个退出0/state=passed与两日志已核验，累计0。代码SHA提交规范39100710788、秘密扫描39100710805均成功。
- saved不再吞坏JSON为无效果，正式read_recovery在来源/候选回调前返回Error: GENERATION_REVIEW_RECEIPT_INVALID；SQL NULL内部保留合法JSON null，无回执与当前空白损坏分开。
- 对齐基线假值无声明行为；NULL/null/false/0/空字符串不伪造效果，对象仍完整证明。新增真实SQLite先失败回归覆盖无写入/无回调与恢复原账本后的完整报告重读。
- 定向review_revision71/0、portable90/0、generation_owner213/0，格式/diff通过；未改schema/定稿/权限/准入/前端。全库parent/confirmation效果查找不因本片通过就登记全部完成。

## 第99次：角色提案与旧名册读面

- 已连续推送a9f28824、7360f4f6，前片云端39097717930被新提交取消，不计独立通过；最新39098085472成功1687/0、check31.08秒无告警。产物31554字节及GitHub SHA256匹配，完整SHA/lock/两个退出0/state=passed与两日志已核验；累计0。代码SHA提交规范39098085516、秘密扫描39098085462成功。
- 角色提案import证明逐行跳过认证冻结run/receipt，不解析已中和payload；当前坏JSON、匹配哈希损坏仍拒绝，缺句柄/重复效果不编造生成来源。真实sanitizer合成SQLite回归覆盖源库不变、历史拒写，不冒充完整导入生成链。
- 旧名册nonce扫描先排冻结根/run、严格解析当前动作与绑定；null上下文不匹配、有效重复仍冲突。回归先复现吞坏绑定误报nonce冲突后通过；未调整错误映射或执行准入。
- 本机前片character_proposal12/0、portable90/0、generation_owner212/0；当前legacy_roster28/0、portable90/0、generation_owner213/0；格式/diff通过。无新GUI/前端变化。
- 下一静态发现涉及定稿后处理read_finalization_effect吞坏回执/效果；尚未改动，需具体确认。持续推进不放宽原具体确认边界。

## 第98次：代理工作流登记冻结过滤

- e093d7d4已推送，云端39096912662成功1685/0、all-targets42.05秒无告警；产物31573字节及GitHub SHA256匹配，完整SHA/lock/两个退出0/state=passed与两日志已核验。语义切片累计0，同SHA提交规范39096912657、Gitleaks39096912651成功。
- 真实脱敏SQLite回归先复现workflow_registration整库JSON1扫描malformed JSON；复用逐行冻结过滤，保持当前坏绑定严格拒绝，不更改登记身份/唯一性/子代来源准入。
- 恢复连接上的历史不发布登记（MISSING），同库新父轮/子代仍可读登记并with_child_effect写当前transition；历史中和值不变、拒写、源文件不变。使用合成TestHost，不扩大为完整生产工作流/GUI证明。
- 本机agent_generation57/0、portable89/0、generation_owner212/0，定向格式/diff通过；无前端变化，无新schema/权限/会话准入/CI配置。

## 第97次：图效果收据与冻结专用入口

- 代码39c1c885已推送，云端39095854001成功1684/0、all-targets35.45秒无告警；产物大小31587字节及GitHub SHA256匹配，完整SHA/lock/两个退出0/state=passed与两日志已核验。当前语义切片累计0；同SHA提交规范39095853945、Gitleaks39095854010成功。
- 对照最新Electron修复图stored_effects/stored_effects_in把坏JSON或graphEffects:null降格为无效果；先失败回归后改为GENERATION_GRAPH_EFFECT_INVALID，合法缺省/[]保持。
- 新增图SQLite回归覆盖NULL/空串/坏JSON/JSON null/非数组/坏结构/超限，普通效果读取、可变性及事务内证明一致拒绝，无回写。
- 实际sanitizer/侧车/连接守卫回归证明图与旧名册require及confirm/stage在收据证明前精确拒绝冻结handle；来源回调不执行、事务退出、拒写、源文件不变。无守卫/非冻结新坏绑定不获豁免。不是有效当前角色完整证明链或任意旧档安装验收。
- 本机graph_generation23/0、portable88/0、legacy_roster27/0、main_generation_owner211/0，格式/diff通过；无前端变更，未新增GUI验收。#30继续，#31待办。

## 第96次：恢复后完整审修与精确历史拒绝

- 51f3e89f修审修可空usage_receipt_json读取；6b8108dc按用户明确选择，drafts/character_relationships只中和R列、不再冻结作者内容行，执行账本仍冻结。
- 恢复副本经实际sanitizer/连接守卫，在SQLite合成集成回归完成审稿→作者确认→修稿→合并→定向复核，recheck_count=1且严格验证通过；历史不发布候选、拒写。不扩大为新版GUI验收。
- d709125a先失败回归后在get/budget解码前检查认证冻结键；旧read_run/resume/read_recovery精确返回Error: PORTABLE_RUNTIME_FROZEN。未认证中和值仍按坏JSON拒绝，缺失身份仍保留missing错误，owner不扩大整个PORTABLE前缀透传。
- 561e91d0修复6b8108dc的整库空依赖豁免：新档中和为[]，仅认证冻结旧草稿兼容空串；普通项目、恢复副本旧可变稿和新稿的空白/坏JSON均拒绝。源库不变；旧档守卫缝不等于完整安装兼容性验收。
- 已独立核验：51f3e89f/39092294785（1680/0，check42.05秒）；6b8108dc/39092830893（1681/0，check38.63秒）；d709125a/39093896169（1681/0，check32.06秒）；561e91d0/39094795752（1682/0，库255.35秒，check36.92秒）。四组均完整SHA/lock匹配、两个退出0、state=passed，两日志无告警，不混用SHA。
- 最新本机review_revision70/0、portable87/0，格式/diff通过；可空R列扫描13列，图stored_effects与旧角色saved的严格String读点仍须按证明阶段逐域核对，不统一吞NULL或坏JSON。
- 旧快照累计4属于历史；按最近核验全量口径当前累计0（561e91d0全量已核验）。#30尚未最终收口，#31待办；无新GUI、真实模型或发布授权。


## 第95次：上游同步与分支结构决策（无Rust语义切片）

- 上游同步：落后 **787** 个提交（含 v1.2.3-Preview）。临时分支 `chore/merge-upstream-20261018` 试合并仅 **1 个冲突**（`.gitignore`，两边都留）；`tauri-app/`、`docs-fork/` **零变更**，fork 的 CI 文件与 `check-commit-msg.mjs` 全保留，根 `pnpm-lock.yaml`/`vite.config.ts`/`.npmrc` 未变，根 `package.json` 仅随上游升版本与脚本。合并前在临时分支跑根 TS 自检（typecheck 30.3 秒、lint 64.5 秒）通过，再 `--ff-only` 合入 master（051c8c83），临时分支已删除。
- **Rust 基线不变**：因 `tauri-app/` 零变更，全量证据仍为 72b91745 / 39088431086（1679/0，all-targets 38.37 秒无告警），不重复跑、不把合并后的文档 SHA 冒充 Rust 测试 SHA。
- 连带修复两处 CI（4ac2dd4e、e6d03172）：① 上游新贡献者 `cheng274771204@gmail.com` 不在 `UPSTREAM_IGNORE_AUTHORS`，使 master 的 Commit message CI 变红 → 追加该邮箱；② 首次改动把注释写在该 `run: |` 块的 10 空格缩进之外，GitHub 判为无效工作流并生成日志为空的失败运行 → 恢复缩进、本地 `js-yaml` 解析校验后重推。合并后提交规范 39090445181 与 Gitleaks 39090445076 均绿。
- 分支结构决策：不剥离 master。理由：剥离后每次上游合并都对每个上游改动文件产生 `modify/delete` 冲突；会丢掉逐频道平移所需的 `electron/`、`src/` 参照；与双栈隔离、`docs/` 上游镜像不可改、根 lock 保持干净且禁删相冲突；剥离不减 `.git`。改以稀疏检出满足「只看产品树」：`.pi\tmp\sparse.ps1 on|off|status`（会话工具，不入库），`on` 时工作区只留 `tauri-app`/`docs-fork`/`.github` 与根文件，`node_modules`/`target`/`.pi/tmp` 不受影响，切换前检查未提交改动；已实测 on/off 往返且 `git status` 干净。
- 行为参照更新：上游本轮改 `electron/` 26 个、`src/` 411 个文件，后续平移对应频道时以合并后的最新上游实现为准。
- 本轮无 Rust 语义切片，累计仍为 **4**（3607e6c2、f7f5658e、c1547e83、72b91745）。

## 第94次：中和JSON列的四处读面改为逐行判定

- 72b91745 / [39088431086](https://github.com/TanCan1998/Lorekeeper/actions/runs/39088431086)：**1679/0**，库267.99秒，all-targets 38.37秒无告警；完整expected/tested SHA一致、两个退出码0、state=passed，artifact与两日志已核对。本轮Rust语义切片累计4。
- 断点3（扫描读R列JSON的读面）完成首轮并修四处：`project_generation_in_flight`（写作在途判定）、`AgentGeneration::session_runs`（代理会话登记）、`proven_graph_own_event_ids`（图上下文归属）、`find_finalization_run`（定稿槽位查找）此前都走 SQL `json_extract`，而 R 策略把 `binding_json`/`attempt_json` 中和为常量，空串触发 malformed JSON → **恢复副本上写作/代理/图/定稿读面整条失败**。
- 新增共用读面 `portable_project_guard::frozen_aware_json_pointer_rows`：按 rowid 顺序逐行判定 JSON 指针，认证冻结行跳过，非冻结行仍严格解析（坏 JSON 照原样报错）。`find_finalization_run` 的 `open_key` 属保留列，仍走 SQL。
- 回归 `sanitized_json_columns_are_read_row_wise_and_fail_closed_when_unfrozen_test` 断言：SQL 形态在中和列上必失败（修复目标）、空冻结集时仍 fail closed、认证冻结后读面返回空、同一连接的新根/新运行/新 attempt 仍可读到、非冻结行坏 JSON 仍报错、源库未被改写。
- 本机全量 lib **1679/0**（65.99秒）；`cargo fmt --check` 与 git diff 通过。
- **同类风险仍需继续扫**：其余读 R 列 JSON 的路径（生成绑定子串查询、恢复候选、角色提案 payload、审修 usage 收据的相关读面）应按同一模式逐一核对；不得因本轮四处已修就假设读面全通。

## 第93次：恢复副本可新建审修周期且冻结历史保持只读

- c1547e83 / [39086559979](https://github.com/TanCan1998/Lorekeeper/actions/runs/39086559979)：**1678/0**，库257.70秒，all-targets 37.69秒无告警；完整expected/tested SHA一致、两个退出码0、state=passed，artifact与两日志已核对。本轮Rust语义切片累计3。
- 断点1（恢复副本混存）实测暴露第二处真实缺陷：便携脱敏按R策略中和`drafts.source_dependencies`，恢复副本里对历史草稿发起新审稿准备直接报`GENERATION_REVIEW_HISTORY_CHANGED`——即恢复副本无法继续写作。现仅对**认证冻结**的草稿行按「无已记录依赖」继续（不猜前驱、不伪造材料），非冻结行的坏依赖仍按损坏拒绝。
- 新增混存回归：同一恢复副本连接上经真实 prepare→open→reserve→snapshot→settle→commit_review 建立**当前**周期；断言新周期可操作（`get_by_review_id`为Some）、冻结老周期仍不发布（None）、破坏新周期执行链收据后全局校验失败（豁免只覆盖冻结行）、非冻结坏依赖仍拒绝、冻结行仍拒写。
- 本机定向：review_revision 67/0、review_cycle 9/0、portable 86/0、draft 77/0；`cargo fmt --check`与git diff通过。
- **同类风险待扫**：R策略中和会破坏JSON列的读面解析（本轮已修`drafts.source_dependencies`与`import_run_receipts`两处）。其余读R列JSON的读面（生成绑定、图/行内槽位、恢复候选等）须逐个核对，不能假设已全部覆盖。
- 仍待：恢复副本中新审修链的**合并与复核**段实际执行、旧handle泛化拒绝错误规范化、其他便携/导入读面收口。

## 第92次：冻结审修历史逐行豁免、导入收据读面与CI提速

- f7f5658e / [39051391021](https://github.com/TanCan1998/Lorekeeper/actions/runs/39051391021)：**1677/0**，库253.00秒，all-targets 38.98秒无告警；完整expected/tested SHA一致、两个退出码0、state=passed，artifact与两日志已核对。同SHA提交规范/秘密扫描成功。本轮Rust语义切片累计2。
- 便携副本按H策略保留审修原始行，但执行链收据（R/X列）已中和。`verify_m03_review_cycle`原先对整库要求可重放证明，导致恢复副本里认证冻结的历史被当成当前损坏，并连带拒绝同库新周期。现按认证冻结键**逐行**跳过执行链校验；表结构、外键、merge/snapshot计数一致性检查全部保留，非冻结行仍逐行严格。
- 豁免不等于授权：`get_by_review_id`仍不发布投影、连接守卫拒绝改写、重放仍被拒。新增便携脱敏回归同时断言「无守卫连接仍拒绝」与「空冻结集仍拒绝」两个对照，证明豁免来自认证冻结键而非放宽校验；既有 `real_sqlite_chain_redacted_history_is_readable_but_cannot_replay` 的断言按新语义拆分，其余读面/写保护/重放证据保留。
- 导入侧（3607e6c2）：认证冻结 `import_runs` 的 effect receipt 读面返回 None，不再退化成泛化「receipt 损坏」；便携脱敏对参与 `UNIQUE(...)` 的 R/X 文本列改用 `portable-neutral:<行身份>` 确定性中和，修复多条 `import_run_receipts` 恢复时撞唯一约束报 `PORTABLE_UNSAFE_PROJECTION`。
- 本机定向：review_revision 66/0、review_cycle 9/0、portable 86/0、冻结导入收据1/1、脱敏导入历史1/1；`cargo fmt --check` 与 git diff 通过。
- 构建/CI提速（2f60163e）：`[profile.test] debug=0`（本机增量重编70.9s→48.6s）、CI缓存键去掉提交SHA（命中后不再上传GB级target）；云端热缓存作业5分26秒，优化前基线6分32秒。实测 cargo-nextest 比 libtest 慢约45%且会改动证据契约，不采用；rust-lld 本机仅快约4.4s且CI需额外组件，不启用。
- 仍待：恢复副本中**新**审修完整链的实际执行验证、旧handle泛化拒绝错误的规范化（须先失败回归）、其他便携/导入读面收口。

## 第90次：专用在途合并、收据重放与旧epoch拒绝

- 80a3228c / [39020182558](https://github.com/TanCan1998/Lorekeeper/actions/runs/39020182558)：**1661/0**，库167.47秒，all-targets29.05秒无告警；完整SHA、两个退出码0、state=passed及artifact/工作流日志已核对，累计0。同SHA提交规范39020182531、秘密扫描39020182562成功。
- 图/行内prepare原先在已有相同调用时被attempt_count抢先拒绝。现在仅固定nonce、冻结任务、模型修订/策略摘要与真实收据匹配时进入原通用准备；复用既有Replay或服务层在途合并，不新增恢复、身份/epoch或来源权威。
- NULL/坏JSON/缺失或非字符串摘要仍由原可变性/尝试门禁拒绝；陌生尝试、哈希冲突、数据库错误不放行。5项新增真实owner回归还覆盖已结算纯重放无写入/新请求、reserved/dispatched准备及pending guard、旧epoch拒绝、取消根不复活。定向owner203/0、run38/0、图17/0、行内25/0、portable77/0、命令4/0；格式/diff/all-targets与隐藏/默认构建通过。
- 隐藏真实WebView中plot/refine各在正式账本已1次物理请求、首调用尚未完成时再execute，两个等待者及完成后重放返回同一候选ID/版本/哈希；合成服务实际合计2次HTTP请求，账本各1。图deterministic、effects=0、草稿/定稿0，未确认图或应用编辑。
- 正常关闭/欢迎页重开同路径后，新租约下历史候选身份/版本/哈希保持、nonReplayable=true；旧租约拒绝，旧epoch两入口均GENERATION_EPOCH_STALE，无新增模型请求。**这是历史持久化/身份拒绝证明，不是跨epoch恢复执行成功。**
- Tauri/Win32主窗口隐藏、不聚焦/非前台；正常关闭、三端口收口、默认构建恢复。证据background-1792228373632/的specialized-replay、specialized-reopen-guards及session；无本机服务/代理在飞。
- 图plan/event/确认、行内应用、跨epoch恢复执行、完整审修与复杂原生归档GUI仍待；若新增会话/恢复准入须具体确认，其他关闭边界不变。

## 第89次：导入槽位读面与有界专用接线

- 该轮fa5cf8aa / [39018418991](https://github.com/TanCan1998/Lorekeeper/actions/runs/39018418991)：**1656/0**，库249.54秒，all-targets36.78秒无告警；artifact完整SHA/两个退出码0/state=passed与两日志已核对，累计0，同SHA提交规范/秘密扫描通过。前片e9b67f91 / 39015932333独立1649/0并核对产物。
- e9b67f91在JSON1槽位匹配前排除冻结旧绑定。实际sanitizer/守卫回归证明旧槽位null、新槽位可读，当前重复/坏绑定拒绝、非字符串键原SQL语义保留、无只读写入/源库改变；不扩大导入执行准入。
- 具体授权下仅图内部双字段准入并主发UUID，通用begin不开放；去掉提交后第二次捕获，复用事务内上下文/来源/摘要。图摘要按持久化Value，与严格读面一致；空事件源仍拒绝。
- 行内内嵌模板变量稳定排序，修复HashMap随机哈希与伪源漂移；全局模板类型/不可变合同不改。旧双变量仅两排列完整摘要验证，正文/选区严格、旧源不自动获执行权威。恢复/幂等读取真实冻结modelReceipt身份。
- 两专用nonce读面先跳过冻结根/run，再按原JSON1谓词查重；当前损坏仍拒绝。5项owner+2项领域回归覆盖真实脱敏共存、稳定重建、幂等冲突、回滚/租约、外项目句柄和源漂移。定向owner198/0、绑定4/0、sources6/0、图17/0、行内25/0、portable77/0、run38/0、命令4/0；格式/diff/all-targets与隐藏/默认构建通过。
- 隐藏真实WebView正式IPC通过两根创建/重读/幂等，行内20次稳定读取及四类拒绝；回环执行各1次请求/1候选，图为deterministic兜底，不是模型结构化质量证明。图effects=0、草稿/定稿0，未确认剧情效果/应用行内输出；不冒充完整UI链。
- Tauri/Win32主窗口隐藏、未聚焦/非前台，无全局输入；正常关闭、三端口收口、默认构建恢复。本机证据background-1792226708206/；旧批次GUI仍绑定80dab6d5。
- 完整审修/复杂原生归档、图plan/event与确认、行内应用、专用重放/重开恢复仍待；生产审稿/自动定稿/真实模型等边界不变。

## 第88次：授权草稿批次准入与真实后台暂停/恢复

- 当前代码80dab6d5 / [39014411676](https://github.com/TanCan1998/Lorekeeper/actions/runs/39014411676)：**1648/0**、库254.45秒、all-targets38.98秒无告警；run成功，artifact完整SHA/两个退出码0/state=passed已下载核对，语义切片累计0。提交规范/秘密扫描同SHA通过。
- 用户具体授权只接通draft_review根/子代准入，保持auto_finalize拒绝。根仅内部初始化；子代绑定真实项目、父根、模型、下一章、精确前驱草稿与冻结作者输入，事务前/事务内复核，两个真实草稿共享根预算。5项新增owner回归覆盖错误准入无副作用、幂等、跨项目及回滚。
- 首次owner声明恢复与草稿批次读取在解析中和JSON前排除冻结run/attempt；非冻结坏绑定/收据仍拒绝，冻结历史不回写、不获执行权威。使用中和样本+正式冻结侧车守卫，不冒充完整便携GUI链。
- 本机owner192/0、agent56/0、草稿效应2/0、run38/0、generation命令4/0、portable77/0；4文件rustfmt/diff、本机all-targets与隐藏/默认开发构建通过，无前端源码变化。
- 隐藏真实WebView经正式UI选择两章草稿待审，请求暂停后第一章完成即paused，稳定等待仍仅一稿；继续完成两稿，同一root、下一章null、定稿始终0。关闭/重开后两稿与批次持久化一致；旧租约读写拒绝、chapter99未写入。
- 合成模型重复正文曾被质量门禁正确拒绝；仅修正本机夹具的不同章节响应、空messages请求计数与联合等待，不降低生产规则。最终独立轮次passed=true。
- 全程Tauri主窗口隐藏/不聚焦，同SHAWin32按标题和类核对交互主窗口隐藏、非前台；Tao内部事件目标不作为用户窗口。无全局输入，窗口正常关闭、三端口收口、默认构建恢复。
- 完整审修、复杂资产原生归档GUI仍未验收；四生产审稿频道、自动定稿、真实模型/MCP/业务云与发布不开放。详细证据目录见当前快照与pipeline。

## 第87次：冻结恢复候选、复杂往返与后台WebView

- 当前最终代码43a00848 / [39012654458](https://github.com/TanCan1998/Lorekeeper/actions/runs/39012654458)：1643/0、all-targets无告警；run成功，artifact SHA/两个退出码/state已核对，语义切片累计0。上一切片7fb424dd / 39012194665也独立1641/0通过。
- 冻结恢复候选按基线只读展示，不校验已中和源快照但始终校验可见正文哈希，sourceCurrent=false；非冻结源校验、旧写保护不变。无守卫中和数据、正文篡改拒绝；完整源冻结也不获当前权威。
- 合成归档包含草稿/审稿材料、冻结生成/导入历史、恢复候选、正式/孤儿PNG与原始资产；两次身份转移、每份重复重开、二次导出恢复通过。源库/源资产不变，正式头像服务可读，孤儿核对不误改；恢复头像漂移拒读、同档恢复不覆盖、再导出拒绝且不发布。不是完整审修效果证明或原生GUI归档验收。
- 本机恢复候选6/0、便携最终77/0、db命令30/0，相关rustfmt/diff与隐藏/默认两种开发构建通过；本轮无前端源码改动。
- 真实隐藏WebView通过会话配置visible=false/focus=false，Win32/Tauri确认主窗口隐藏且非前台；同路径真实关闭/欢迎页重开后，旧lease读写拒绝，chapter99未写入、原两章未变。未用全局键鼠；窗口正常关闭，三端口已收口，debug二进制已恢复默认配置。
- **新阻塞**：草稿待审批量启动返回GENERATION_REQUEST_FAILED；源码通用分支仍拒绝batchIntent/batchId。草稿/批次/模型请求均0，暂停/恢复未验收。批次准入接线须先具体确认，不自行放开auto_finalize/生产审稿。

## 第86次：冻结导入任务列表

- 当前代码`e5562957aafd16090b33fe71d4afdc3c25c282ba`已推送；[云端39000071204](https://github.com/TanCan1998/Lorekeeper/actions/runs/39000071204)现已成功1639/0、all-targets无告警，artifact已核对。当轮成功全量为61ab33bc/38999595785：1638/0、all-targets无告警，run成功且artifact SHA/两个退出码/state已核对；其后语义切片累计1。下文183d4886证明点仍是历史绑定，不冒充当前版全量。
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

## 第84次生成知识与接线回归（历史范围）

- 用户具体授权Tauri知识修订schema升级；`db/kb/revisions.rs`区分规范/派生，涵盖幂等升级、回滚、外层事务、双连接已提交可见性、缺行与溢出拒绝。旧便携schema可升级，Electron及旧库不读不改。
- `services/generation_knowledge_source.rs`真实捕获修订、规范文档/块身份、正文与整文档块集合；空结果也绑定修订。覆盖reference/脏副本排除、向量ID规范错配、缺块/重复ID/畸形快照及未选块变化。
- owner的prepared_knowledge_drift_rejects_begin_and_execution_without_new_effects验证拒绝旧准备、不新增run/lease/attempt；commands::kb等待源锁后失效会话拒绝。网络阶段不持有owner/DB/知识锁，返回重新认证。
- generation:read-context和read-batch请求包装已对齐；裸句柄合同保持不变。前端映射与Rust解码拒绝错误包装回归通过。
- read_recovery_context将composition末尾artifact按当前run物理归属解析attempt，再读finishReason；stop/length回归通过，恢复读取不提前提交草稿。

## 历史前台GUI已完成范围（第82–84次）

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

- 便携生成列表、候选、导入任务/生成槽位及图/行内nonce读面已逐步补齐；审修材料、完整导入推理、图各类型确认及跨epoch恢复执行等仍须逐域核对（同epoch专用重放已于第90次有界验证），不能推断全部便携读面已通。
- 完整审稿→作者确认→修稿→差异合并→复核GUI未验收，四生产审稿频道继续关闭。
- 草稿待审批次暂停/恢复及有界旧会话交错已于第88次后台验收；任意在途/跨项目组合未穷尽。复杂归档后端往返于第87次补齐，含复杂资产/审修的原生归档GUI仍未完成。
- 真实模型/外部MCP/业务云/发布未授权；macOS/Linux未认证，不宣称全链安全或发布认证。

## 历史证据绑定（第84次）

- [第84次云端38997082671](https://github.com/TanCan1998/Lorekeeper/actions/runs/38997082671)：最终SHA=183d4886，1637/0、库227.11秒，all-targets成功无告警；artifact的expectedSha/testedSha一致，test/check退出0、state=passed。
- 中间SHA：6b2c5785 / 38950154610 / 1616；512e759f / 38954708710 / 1629；9c01fe45 / 38955678898 / 1630。均已下载核对，不混用版本。
- 本机`.pi/tmp/recovery/ci-{提交前缀}/`保存云端日志；artifact保存14天。未脱敏日志/用户数据不入库。
- 定向/前端：identity-directed、knowledge-*、kb-schema-*、composition-recovery-test、transport-*、final-83-*日志与结果JSON。
- GUI：transport-gui-cancel、transport-gui-archive-ui、transport-gui-copy、transport-gui-reopened截图/JSON；transport-archive-verification.json只记录合成身份/计数/哈希及历史失败事实。合成正文和本机凭据不入库。


## 第91次增量审计（覆盖上述历史待验状态）

- 当前代码93b3b27d：云端39041910917已下载核对完整SHA、1674/0、两个退出码0、state=passed、all-targets无告警。8ecfc0fb/39041387445与7ae31540/39039141837也独立核对1674/0，不混用SHA。
- 图plan/event/plot真实确认、行内应用/选区移动/正文漂移保护、标题栏正常退出后图/行内新租约恢复已验；冻结旧句柄执行仍拒绝。
- 审修有效finding→人工明确纳入/确认→修稿保存→合并→定向复验持久化1次→末审2通过完成，四生产request信封于8ecfc0fb开放；仅合成模型，finding未知状态不自动变作者事实，未自动定稿。
- 复杂头像/审修原生归档核心链通过，正文/审修保留列及资产哈希一致、恢复副本UI可读；93b3b27d修冻结周期不发布当前执行投影，不称脱敏链执行证明成功。
- 仍待其他便携/导入非空读面、冻结与新周期混存核查；旧handle的泛化/JSON错误仅证明拒绝，不是精确冻结错误合同。本轮不登记六项最终完成。
- 窗口正常关闭，mock正常退出，Vite停，三端口无监听，默认开发构建恢复。细节、证据路径与断点见[第91次快照](../handoffs/2026-10-17-tauri-migration-status.md)。
