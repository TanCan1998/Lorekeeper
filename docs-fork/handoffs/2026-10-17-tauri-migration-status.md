# Tauri迁移交接快照（2026-10-17）

> [当前入口](./CURRENT.md) · [混合验证](../agents/tauri-rust-ci.md) · [首版决策](../plans/2026-10-17-tauri-completion-decisions.md) · [证明/GUI审计矩阵](../plans/2026-10-17-tauri-proof-audit.md)。动态进度仅在docs-fork展示，同日改写当前摘要。

## 已验证状态（第89次）

当前代码：`fa5cf8aa698d0e2d4748593bcc179d32533209ce`，master，已推送origin；后续为文档收口。用户确认卡`specialized_generation_scope=allow-specialized-generation`具体授权图生成内部专用准入、行内上下文接线、冻结读取、合成回归/全量和后台验证；不扩展schema、依赖、文件权限、定稿、生产审稿、真实模型或代理授权。

| 范围 | 结果/版本 | 证据 |
|---|---|---|
| 最新Rust云端全量 | **1656通过/0失败**，库249.54秒，SHA=fa5cf8aa，run completed/success | [39018418991](https://github.com/TanCan1998/Lorekeeper/actions/runs/39018418991)，`.pi/tmp/recovery/ci-fa5cf8aa/` |
| 同SHA all-targets | 成功无告警，36.78秒 | artifact expectedSha/testedSha一致、两个退出码0、state=passed，两日志已核对 |
| 同SHA提交规范/秘密扫描 | 均success | 39018418990 / 39018418996 |
| 本机定向 | owner198/0、源绑定4/0、sources6/0、图17/0、行内25/0、portable77/0、run38/0、generation命令4/0 | 5项新增专用owner与2项领域哈希回归；诊断与最终定向日志保留本机 |
| 格式/构建 | 4文件rustfmt/diff、本机all-targets、隐藏及默认开发构建通过 | 会话配置覆盖，不改跟踪配置；当前debug二进制已恢复默认行为 |
| 隐藏WebView专用IPC | 两根创建/重读/幂等、行内20次稳定重读、伪造/错选区/模型切换/外项目句柄拒绝 | `background-1792226708206/specialized-ipc.json` passed=true |
| 同轮回环执行 | 图与行内各1次请求/1份候选，图为deterministic兜底，effects=0，草稿/定稿0 | 同目录specialized-execution.json passed=true；未确认剧情效果或应用编辑输出 |
| 后台/收口 | Tauri API隐藏/不聚焦，Win32 Tauri Window主窗口隐藏、非前台；正常标题栏关闭 | 同目录win32-owned-window、closed-window、session；5190/9226/19537均无监听 |
| 前端沿用 | 188文件2111通过/1跳过，typecheck/lint及上次build通过 | 本轮无前端源码改动，未重跑，不算新增TS验收 |

最新全量基线fa5cf8aa，**其后Rust语义切片累计0**。前片e9b67f91 / [39015932333](https://github.com/TanCan1998/Lorekeeper/actions/runs/39015932333)独立成功1649/0，库245.63秒、all-targets41.65秒无告警，artifact已核对；不混用版本。

## 本轮代码与有界结论

### 导入生成槽位只读修复（e9b67f91）

- `read_import_generation`曾对全表binding_json直接json_extract，实际脱敏副本中无关旧绑定为空，导致malformed JSON。先按统一连接冻结事实排除旧run，再逐行保留原SQLite谓词。
- 真实源库→sanitizer→冻结侧车/守卫回归先失败后通过：未装守卫的中和数据仍报错；冻结旧槽位返回null，当前新槽位可读；当前重复槽位、非冻结坏绑定仍拒绝，非字符串键保持原SQL比较语义。只读不增加changes，源库字节不变、旧写保护保留。
- 不扩大导入执行准入，不代表完整导入推理链可用；该片本机owner193/0、import服务16/0、portable77/0、run38/0、命令4/0及格式/all-targets通过。

### 图生成专用接线（fa5cf8aa）

- 对齐基线内部`graphInitialization`：仅graph_admission放行graphGenerationInput/graphGenerationKey，主进程生成UUID；公共BEGIN_ALLOWED_KEYS不增加字段，专用频道也拒绝外部伪造key。
- 原入口在普通源库先被通用键集拒绝；补后进一步暴露提交后再次捕获/改写上下文，导致key与原摘要/来源绑定不一致。现只复用begin事务内捕获，不提交后重采样，不额外改写账本。
- 图上下文/任务摘要按实际持久化JSON Value计算，与既有严格读面一致，不混用结构体字段顺序。无事件源仍按原规则拒绝，未绕过NO_EVENT_SOURCES。

### 行内稳定上下文与nonce读取

- 内嵌PromptTemplate.variables为HashMap，原上下文哈希随反序列化随机键序变化，独立夹具重复后复现GENERATION_EDITOR_CONTEXT_INVALID。仅对行内上下文内嵌模板的变量字典稳定排序，保持其余字段/值/字段顺序；不改全局PromptTemplate类型或不可变合同。
- 旧双变量两种排列仅做有界完整摘要验证，不尝试任意排列、不改写旧收据、不自动授予旧源当前执行权威；正文与选区仍严格校验，未知/不匹配摘要拒绝。
- 行内恢复/幂等从冻结modelReceipt.modelId读取模型身份，保留旧平面字段只读回退，修复相同请求被误判nonce冲突。
- 图/行内nonce查重共用私有只读扫描：在JSON1解析前排除冻结run/root，当前非冻结损坏仍报错，不用json_valid静默吞掉。实际sanitizer/守卫回归覆盖冻结旧记录与当前新根共存、幂等/冲突、旧写保护和源库不变。
- 5项owner回归还覆盖12个独立随机HashMap夹具、多次重读/重建绑定稳定、通用入口越权、事务失败根/run/租约回滚、跨项目句柄、模型切换与作者来源漂移拒绝；不调用模型。

## 本轮真实隐藏WebView验证边界

- 独立home/合成项目，TAURI_CONFIG会话覆盖visible=false/focus=false；正式UI打开项目、写合成配置/蓝图，再从真实WebView走正式IPC。全程无置顶/激活、全局键鼠或SendKeys；只用127.0.0.1合成HTTP/SSE端点。
- 两专用根创建及幂等各保留同一句柄，图重读成功；行内20次重读保持documentHash、sourceStatus=current及正确冻结模型。伪造通用begin、错选区、模型切换、外项目句柄被拒，运行数仅0→2。
- 同轮正式execute IPC各产生1次物理请求及1份持久候选，来源仍current。图合成响应是普通文本，正式解析走deterministic兜底并可读，不是模型结构化质量或全图种类证明。
- 未调用graph confirm、未把行内结果应用到正文，effects=0、草稿0、定稿0。该结果是**后台真实WebView的IPC/回环执行验收，不是完整UI确认/编辑应用链验收**。
- Tauri API反复false/false，Win32仅识别当前进程的Tauri Window主窗口，确认隐藏/非前台，不把Tao内部事件目标当用户窗口。窗口按正常标题栏协议关闭，模型正常收口、Vite身份核对后停止，三端口无监听，默认开发构建已恢复。

## 沿用证明、未完成项与下一步

- 第88次`80dab6d5`的两章草稿待审批次UI暂停/继续、同根预算与重开持久化/旧租约拒绝沿用原实测；本轮未重跑批次GUI。自动定稿仍拒绝，原会话/文件权限/定稿规则不变。
- 第87次43a00848复杂归档后端两次身份转移、重复重开、资产漂移拒绝，以及7fb424dd冻结恢复候选只读证明沿用；不能替代复杂资产/审修原生归档GUI。
- 后续逐域核对图plan/event、确认效果与行内选区应用UI、专用execute重放/重开恢复及更广便携审修/导入读面；本轮plot兜底和行内候选不覆盖上述链路。
- 完整审稿→作者确认→修稿→合并→复核GUI仍未验收，四生产审稿频道继续关闭；真实模型/MCP/业务云/发布未授权，macOS/Linux未认证，不宣称全链安全。
- 注册291/契约290/事件7、旧扫描289/290、前端路由285/290沿用。静态注册/扫描数不代表所有业务路径验收。

活动状态见`.pi/tmp/pipeline-state.md`；临时日志、合成数据、构建产物和凭据不入库。无子代理、无本机服务在飞。下次后台验证须重新隐藏构建，不得直接运行当前默认二进制或旧前台脚本。
