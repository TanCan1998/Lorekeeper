# Tauri迁移交接快照（2026-10-17）

> [当前入口](./CURRENT.md) · [混合验证](../agents/tauri-rust-ci.md) · [证明审计](../plans/2026-10-17-tauri-proof-audit.md) · [频道清单](../plans/tauri-migration-channel-inventory.md)。本次为第 **91** 次交接；当前摘要替换同日旧摘要。

## 代码与验证基线

当前代码：**93b3b27d0964b81234ae83798d759c84a1cc8c24**，master，已推送origin。本次后续仅文档提交；不把文档SHA冒充Rust测试SHA。

| 验证 | 结果 | 绑定证据 |
|---|---|---|
| 当前Rust全量 | 1674通过/0失败，库192.06秒 | [39041910917](https://github.com/TanCan1998/Lorekeeper/actions/runs/39041910917)，93b3b27d |
| 当前all-targets | 36.78秒，成功无告警 | artifact expectedSha/testedSha为完整93b3b27d，test/check退出0、state=passed，日志已下载核对 |
| 当前提交规范/秘密扫描 | success | 39041910893 / 39041910912 |
| 本机当前切片定向 | review_revision64/0、portable_project44/0、review_cycle9/0 | review-frozen-*.log；两文件rustfmt、diff通过 |
| 前端最终接线 | 190文件，2121通过/1跳过；typecheck/lint成功 | 8ecfc0fb接线；review-final-pnpm-*.log |
| 其他审修回归 | owner211/0，材料source_binding5/0 | 分别记录在review-confirmation/append相关日志；不与当前定向合并计数 |
| 上一完整云端 | 8ecfc0fb：1674/0，库184.76秒，check37.57秒 | 39041387445，完整SHA及产物核对 |
| 作者确认身份修复云端 | 7ae31540：1674/0，库253.62秒，check36.88秒 | 39039141837，完整产物核对 |

当前全量基线93b3b27d，之后Rust语义切片累计 **0**。云端artifact本机分别保存在 `.pi/tmp/recovery/ci-{SHA前缀}/`。

## 六项任务实绩

### 图、行内与恢复（#26–28完成）

- a612cf32修复plan/event输入对象键序误判，6项回归；前端定向33及typecheck/lint通过。真实隐藏UI完成plan/event候选确认、重复确认、plot保存与重复确认；plot仍deterministic兜底，不代表真实模型质量，不改作者事实。
- 行内真实UI：润色→应用原选区；移动选区不重定向；正文新增后拒绝旧候选并保留输入，再通过UI保存。仅CodeMirror公开dispatch与真实可见按钮，证据inline-ui.json。
- 3041a937显式新租约重新认证，重新验证来源/模型/正文或原选区/预算；381414cf正常标题栏关闭先挂起、失败不退出。真实退出→重开→图历史继续与侧栏行内继续各1物理请求，原run/root、新epoch；旧epoch均拒绝。
- 381对应云端39026151847完整核对1668/0、all-targets无告警；不能把旧句柄拒绝或历史可读当成跨会话执行成功。

### 完整审修（#29完成，四频道开放）

- b4a8fb72增加缓存签发的有界prepared准入，绑定project/epoch、操作、来源/谱系、模型、输出及父根，事务内外重捕获；拒绝额外覆盖、批次/Agent字段，保存效果封存新执行，精确Replay保留。
- 702339c1修材料闭合哈希：空前缀去重、UTF-16长度排序、typed IPC JSON键序。053db7a2将未注册默认算法改为visible-append-v1，并撤回不必要的修稿draft-visible-v2白名单扩张。
- 7ae31540保留数据库作者确认原始内容字节供哈希身份；仍严格解析比较快照。回归先复现、修复后审修64/0、owner211/0。
- 真实隐藏UI完成有效finding的作者明确纳入、确认清单、修稿生成保存、全部修稿、完成合并、重试合并后复验，以及末次审稿。
- 已读回周期merge-committed、recheck_count=1、recheckDisposition=completed。模型声称resolved未被自动升级为作者事实，finding保守为unknown。末次审稿另存且显示2通过，有正文证据；未自动定稿。
- 8ecfc0fb正式开放四个request信封频道：review-revision:prepare / commit-review / commit-revision / read-recovery。独立projectSession提供权威，新增完整信封/冻结请求不变/缺会话传输前拒绝测试；DEFERRED_RENDERER_CHANNELS为空，不代表所有业务已最终验收。

### 复杂归档（#30进行中）

- 在隔离项目通过正式IPC写入合成1像素头像夹具，再经真实设置→项目备份→导出按钮、原生保存框、恢复按钮、原生选归档框及目标目录框恢复新副本。未绕过路径grant，未调用业务云。
- 三类对话框均限定本进程HWND、控件ID，定向WM_SETTEXT/BM_CLICK，无全局键鼠/焦点切换。UIAutomation控件查询不完整，前期失败不计成功证据；最终以实际归档文件、UI成功收据及恢复数据验证为准。
- 新副本身份与源不同；只读SQLite核对：19 contents、2 drafts、5 revisions、12 reviews、6 review_cycles、7 findings、5 merge快照、角色及头像行保留列完全一致。drafts.source_dependencies按既定R策略中和，不宣称整库逐字节相同。
- 头像68字节内容及SHA256一致，真实角色UI列表和预览均成功加载。恢复后真实UI可读草稿（已修稿）、既有定稿章及末审报告2通过。
- 发现周期读面错误：归档已中和执行绑定/收据，但读面仍要求证明执行链，报REVIEW_CYCLE_CONFLICT。93b3b27d在连接认证的冻结cycle检查后返回None，不发布当前可操作周期；原材料保留，非冻结周期仍完整校验。新增回归明确verify_m03对脱敏链仍为false，不伪造执行证明。
- 恢复副本正式IPC：review-list12、revision-list5、cycle-get为null、头像可读；generation:list/list-batches/list-directory-progress均空。
- **不能扩大结论**：旧handle尝试虽拒绝，但实测resume为GENERATION_REQUEST_FAILED，read-recovery为GENERATION_JSON_PARSE_ERROR，不是精确PORTABLE_RUNTIME_FROZEN合同。下一轮若要调整需先定位，不得在证据中改写错误。
- 其他便携/导入读面尚未完成，当前项目没有import_runs，不能用空数据宣称冻结导入历史通过。早先复杂后端两次身份转移及漂移拒绝仅作为独立历史回归，不替代本轮GUI。

## 本机证据与恢复入口

所有临时文件/合成内容不入库。

- 会话事实源：.pi/tmp/recovery/background-session.json；证据根：.pi/tmp/recovery/background-1792229720030。
- 当前脚本已关闭应用；登记PID9692仅历史记录，不代表在飞。backendCodeSha=93b3b27d。源项目路径在session.project；恢复副本在session.work/complex-assets-restored-parent/下；源与恢复项目绝不可混用。
- 图/行内/恢复：graph-resume-ui-checkpoint.json、inline-ui.json、specialized-native-reopen-ui.json。
- 审修：review-recheck-ui.json、review-cycle-current.json、review-live-state.json、review-mock-events.jsonl及valid-chain-changed-*.log。background-review-open-latest脚本后来复用过，review-valid-open-ui.json会覆盖，不单独作为原审修全链证据。
- 归档：complex-assets-review-ui.ainovel、complex-roundtrip-integrity.json、complex-restored-read.json、complex-avatar-seed.json、complex-restored-session.json；complex-backup-ui-state.json为轮询状态文件，后续可能覆盖。
- 可复用脚本：background-native-hwnd.ps1、background-archive-click.cjs、background-archive-restore-click.cjs、background-restored-open.cjs、background-restored-read.cjs、background-restored-avatar-ui.cjs。
- GUI脚本须通过node .pi/tmp/recovery/background-inspect.cjs <模块路径>调用；直接node导出模块不操作。重启前先隐藏构建，不能直接启动当前默认二进制。
- 动态import的store操作后UI曾仍停留欢迎页；真实点击书架打开按钮后UI正常。后续GUI验收以真实可见控件和实际激活项目为准，不只以动态模块store成功返回认定UI切换。

## 后台收口与边界

- 无活跃工作流时通过真实标题栏正常关闭，合成服务/__shutdown正常退出；核对PID/命令后停止本次Vite，5190/9226/19537均无监听。
- 默认cargo build --locked恢复成功。首次误将TAURI_CONFIG置空字符串导致JSON EOF，已改为从子进程环境删除变量后构建成功；不保留空字符串覆盖。
- 无代理派发，自动定稿、真实模型、外部MCP、业务云、发布未授权；macOS/Linux未认证；旧库转换首版不支持。
- 冻结历史不是执行权威，原始收据身份不改写；新租约恢复必须重新认证，不能让旧句柄直接执行。

## 下轮明确断点

1. 保留TODO计划#6：#25–29完成；#30进行中，#31待最终收口。当前93b3b27d云端全量已经核对，勿再列为等待。
2. 补齐其他便携/导入读面，使用含真实合成import_run/章节/收据/来源槽位的隔离数据，核对冻结过滤与当前损坏仍拒绝；现有UI归档核心链已完成，不重复从零造审修闭环。
3. 核查冻结审修周期与新当前周期混存时verify_m03全局验证的影响：本轮只修读面返回None，未豁免全局执行链校验，未验证恢复副本中新审修完整链。
4. 对旧handle的泛化拒绝/JSON错误如需规范化，添加先失败回归，保留坏当前收据拒绝和非冻结校验，不能冒充历史执行权威。
5. 完成#30后再做#31最终交接/测试口径收口；本次交接不登记六项全部完成。
