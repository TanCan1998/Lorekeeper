# Tauri迁移交接快照（2026-10-17）

> [当前入口](./CURRENT.md) · [混合验证](../agents/tauri-rust-ci.md) · [首版决策](../plans/2026-10-17-tauri-completion-decisions.md) · [证明/GUI审计矩阵](../plans/2026-10-17-tauri-proof-audit.md)。动态进度仅在docs-fork展示，同日改写当前摘要。

## 已验证状态（第90次）

当前代码：`80a3228ce37c67778fe6dee5e75afd28ccb71bb6`，master，已推送origin；后续为文档收口。本轮在既有specialized_generation_scope具体授权内修复专用重复调用；不扩大公共begin、schema、依赖、文件权限、会话/恢复准入、定稿、生产审稿、真实模型或代理授权。

| 范围 | 结果/版本 | 证据 |
|---|---|---|
| 最新Rust云端全量 | **1661通过/0失败**，库167.47秒，SHA=80a3228c，run completed/success | [39020182558](https://github.com/TanCan1998/Lorekeeper/actions/runs/39020182558)，`.pi/tmp/recovery/ci-80a3228c/` |
| 同SHA all-targets | 成功无告警，29.05秒 | artifact expectedSha/testedSha为完整80a3228c、两个退出码0、state=passed，两日志和工作流日志已核对 |
| 同SHA提交规范/秘密扫描 | 均success | 39020182531 / 39020182562 |
| 本机定向 | owner203/0、run38/0、图17/0、行内25/0、portable77/0、generation命令4/0 | 5项新增专用回归，专用owner累计10项；最终owner覆盖原有裸尝试冲突测试 |
| 格式/构建 | 两文件rustfmt/diff、本机all-targets、隐藏及默认开发构建通过 | 会话配置覆盖，不改跟踪配置；当前debug二进制恢复默认行为 |
| 隐藏WebView专用IPC | 两根创建/重读/幂等、行内20次稳定读取及伪造/选区/模型切换/跨项目拒绝重验 | `background-1792228373632/specialized-ipc.json` passed=true |
| 在途/完成重放 | plot/refine均在已有1次物理尝试且首请求未完成时重复execute，完成后再执行；候选ID/版本/哈希一致 | 同目录specialized-replay.json；两入口合计2次实际合成HTTP请求，各1份候选 |
| 正常关闭/重开 | 新租约下历史候选不变，nonReplayable=true；旧租约拒绝，旧epoch两入口均GENERATION_EPOCH_STALE，无额外模型请求 | 同目录specialized-reopen-guards.json passed=true；不是跨epoch恢复执行成功 |
| 后台/收口 | Tauri API隐藏/不聚焦，Win32 Tauri Window主窗口隐藏、非前台，正常标题栏关闭 | 同目录win32-owned-window、closed-window、session；5190/9226/19537无监听 |
| 前端沿用 | 188文件2111通过/1跳过，typecheck/lint及上次build通过 | 本轮无前端源码改动，未重跑，不算新增TS验收 |

最新全量基线80a3228c，**其后Rust语义切片累计0**。上一成功基线fa5cf8aa / [39018418991](https://github.com/TanCan1998/Lorekeeper/actions/runs/39018418991)独立1656/0、all-targets无告警，artifact已核对；不混用版本。

## 本轮代码与有界结论

### 重复调用遗漏与修复

- 两专用prepare入口原先在已有同一nonce调用时先按attempt_count报冲突，未进入基线通用重放路径；首次执行成功不代表重复execute可用。
- 新增私有既有调用识别：固定graph:0/editor-inline:0，按冻结任务、model_lease_revision及policy_hash计算请求摘要，并由仓储读取/校验真实收据。精确匹配才转入既有通用准备与服务层在途合并。
- 裸尝试、NULL/坏JSON/缺失或非字符串摘要不能被当作新调用；回到原可变性/尝试门禁拒绝，图域原有坏收据错误优先级保留。可验证但请求摘要不一致仍报调用冲突，数据库错误不吞掉。
- 不新增epoch、根或模型身份，不重绑来源/预算，不调用恢复准入。通用身份/epoch/任务合同及实际Dispatch门禁不变；已结算纯收据重放不等于允许旧作者源再次生成。

### 五项新增真实owner回归

- 已结算调用进入Replay，作者蓝图变化后仍返回原内容；没有新物理尝试、数据库写入或租约增长。
- reserved/dispatched调用复用冻结任务摘要和固定nonce，保留单一尝试；核对pending guard引用/释放生命周期。准备回归不冒充真实网络合并，实际合并另由后台验证。
- 陌生nonce和请求哈希冲突仍拒绝，无副作用。
- 新owner旧epoch仍拒绝；同epoch已取消根只能纯重放旧收据，不复活根或改写账本。
- 两入口的NULL、缺摘要、坏JSON和非字符串摘要均保留各域原有拒绝，无写入/租约变化。扩大定向曾暴露旧裸尝试错误契约回归，已保留原门禁；图域坏收据负例按原可变性守卫验证，未放宽生产规则。

## 真实隐藏WebView验证边界

- 独立home/合成项目，TAURI_CONFIG会话覆盖visible=false/focus=false；正式UI打开、写合成配置/蓝图，真实WebView调用正式IPC。仅127.0.0.1合成HTTP/SSE，无全局键鼠、置顶/激活或SendKeys。
- plot与refine各在首请求未完成、正式读取账本已显示1次物理请求后发出相同execute；两个等待者成功返回同一artifactId、revision和textHash。完成后再execute仍返回相同候选。合成服务实际总请求数2，账本各1，不只依赖调用退出码判断合并。
- 正常关闭项目并从欢迎页重开同路径，租约发生变化；旧租约请求拒绝，新租约携旧epoch句柄执行也拒绝。历史候选可读、身份/版本/哈希不变且nonReplayable=true，实际模型请求仍2。
- 图仍为deterministic兜底，effects=0、草稿/定稿0。未确认图效果、未应用编辑结果；不是模型结构化质量、所有图类型或完整UI链证明。
- Tauri API反复false/false，Win32准确识别当前进程Tauri Window主窗口隐藏/非前台。窗口正常标题栏关闭，模型正常收口、Vite身份核对后停止，三端口无监听，默认开发构建恢复。

## 沿用证明、未完成项与下一步

- 第89次fa5cf8aa仅内部图双字段/UUID准入、事务内单次上下文、持久化Value摘要；行内内嵌变量稳定排序、冻结modelReceipt身份读取、旧双变量有界完整摘要验证；图/行内nonce扫描排除冻结旧根/run。通用begin不开放，正文/选区合同严格，实际脱敏共存、回滚/租约、跨项目及源漂移回归沿用。
- e9b67f91导入槽位在JSON1匹配前过滤冻结旧绑定，旧槽位null、当前槽位可读/损坏拒绝，保持原SQL语义与旧写保护；不扩大导入执行准入。
- 第88次80dab6d5两章草稿待审批次UI暂停/继续、同根预算、重开持久化与旧租约拒绝沿用原实测；本轮未重跑批次GUI。自动定稿仍拒绝。
- 第87次43a00848复杂归档后端两次身份转移、重复重开、资产漂移拒绝，以及7fb424dd冻结恢复候选只读证明沿用；不能替代复杂资产/审修原生归档GUI。
- 后续逐域核对图plan/event、确认效果、行内选区应用UI及更广便携审修/导入读面。**同epoch重放与重开历史/拒绝已验，不代表跨epoch专用恢复执行接通**；若需新增恢复准入，须具体确认后再改。
- 完整审稿→作者确认→修稿→合并→复核GUI未验收，四生产审稿频道关闭；真实模型/MCP/业务云/发布未授权，macOS/Linux未认证，不宣称全链安全。
- 注册291/契约290/事件7、旧扫描289/290、前端路由285/290沿用。静态数量不代表所有业务路径验收。

活动状态见`.pi/tmp/pipeline-state.md`；临时日志、合成数据、构建产物和凭据不入库。无子代理、无本机服务在飞。下次后台验证须重新隐藏构建，不得直接运行当前默认二进制或旧前台脚本。
