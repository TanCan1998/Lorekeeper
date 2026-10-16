# Tauri迁移交接快照（2026-10-17）

> [当前入口](./CURRENT.md) · [混合验证](../agents/tauri-rust-ci.md) · [首版决策](../plans/2026-10-17-tauri-completion-decisions.md) · [证明/GUI审计矩阵](../plans/2026-10-17-tauri-proof-audit.md)。动态进度仅在docs-fork展示。

## 已验证状态（第83次）

代码版本：`24291a43d878203cf9b44774bb564edc6ef2410a`，master，已推送origin。之后收口仅文档。验证日期为本机2026-10-17（云端UTC日志为2026-10-16）。

| 范围 | 结果/版本 | 证据 |
|---|---|---|
| 最新Rust云端全量 | **1631通过/0失败**，库221.99秒，同上SHA | [run38956708373](https://github.com/TanCan1998/Lorekeeper/actions/runs/38956708373)；本机`.pi/tmp/recovery/ci-24291a43/` |
| 云端all-targets | 成功、无告警；expectedSha/testedSha一致，testExitCode/checkExitCode均0，state=passed | 同run artifact的validation.json、cargo-test.log、cargo-check.log已下载逐项核对 |
| 云端提交规范/秘密扫描 | 同SHA均成功 | Commit message CI 38956708445、Gitleaks 38956708370 |
| 本轮Rust定向 | 审修服务57/0；知识快照6/0、KB命令12/0、数据库120/0、便携归档74/0；最后owner域182/0、命令合同4/0 | identity-directed.log、knowledge-*.log、kb-schema-*.log、composition-recovery-test.log、transport-rust.log；按各切片执行，不相加冒充独立全量 |
| 前端 | 188文件、2111通过/1跳过；typecheck/lint通过；最终代码build通过（大chunk与构建计时提示） | transport-test.log、transport-typecheck.log、transport-lint.log、final-83-build.log；前端在9c01fe45后未再变化 |
| 格式/隔离 | 本轮11个Rust变更文件rustfmt、diff检查通过；根src/Electron/依赖锁及docs镜像无变更；AGENTS仍忽略 | final-83-rustfmt.log、git差异；最初格式命令误列不存在路径，纠正为实际变更文件后通过 |
| 开发构建与GUI | 最终代码cargo build --locked通过；Windows隔离合成GUI实际保存617字草稿，流式取消不新增草稿；归档正文一致、重启可读；**恢复副本生成历史列表失败** | knowledge-build.log、transport-gui-*、transport-archive-verification.json |
| 频道口径 | 注册291、契约注册290、事件7；功能迁移289/290、业务开放285/290，不变 | final-83-channels.log；blocked不计功能覆盖 |

本机证据均在`.pi/tmp/recovery/`，不入库。最新全量代码基线为24291a43，**之后Rust语义切片累计0**。本轮中间SHA的云端artifact也已核对：6b2c5785 / 38950154610为1616/0，512e759f / 38954708710为1629/0，9c01fe45 / 38955678898为1630/0，均all-targets成功、无告警；不能把它们的数量混成最终SHA结果。

## 本次更新

- `6b2c5785`：新增初审/修稿/复核project、epoch、artifact单字段身份错配回归，共36组；另验证合法恢复新租约后，历史产物仍保留原epoch。未放宽生产审稿准入。
- `512e759f`：按用户明确schema授权，给Tauri规范知识文档/块新增持久修订计数及事务触发器，幂等升级、回滚不耗修订、派生索引重建不冒充原文变化。便携元数据可选以兼容旧归档；损坏活跃元数据拒绝，不静默补零。
- 正文准备接通真实知识快照：一次检索后冻结精确文档/块ID、正文哈希、整文档块集合及修订；向量或FTS/jieba来源均做规范身份投影，排除reference/脏作者副本。空结果同样绑定真实修订；begin、执行及恢复只复核、不重搜替换。可选嵌入网络阶段不保留业务锁，返回及等待锁后重新认证会话。
- `9c01fe45`：恢复上下文及批次进度以request包装对齐TS/Rust合同；裸句柄频道不通用解包，仍保留会话尾参和身份验证。新增前端映射及Rust反序列化回归。
- `24291a43`：恢复投影先按当前run的物理归属将artifactId解析为attemptId，再读结束收据；补stop/length真实SQLite回归，不提前保存草稿。
- GUI实际越过知识库拒绝及恢复参数错误，完成本机合成短细纲、流式正文、合成收据、草稿提交；另一次流式中止后cancelled且草稿数不变。曾见合成端点重复正文触发NO_PROGRESS，未放宽校验；成功场景用500字目标、显示617字。
- 本地归档导出及新副本恢复成功、不替换当前项目；打开副本后两稿的章节号/版本/状态/正文哈希一致，正常关闭重启后仍显示617字。恢复实际路径以最终收据为准，保存在本机证据，不以自动化输入的期望路径代替。

## 真实窗口与授权收口

本轮用户通过确认卡临时授权前台激活和定向GUI交互，仅限隔离合成项目；未读真实项目、未调用真实模型/外部MCP/业务云，未使用全局键鼠或SendKeys。Windows开发窗口不等于发布包验收。

测试窗口已正常关闭，本机合成HTTP服务与Vite已停止，相关监听和Lorekeeper进程已核查无残留。**本轮临时前台授权结束，恢复默认后台、不置顶、不激活、不抢焦点规则。** 旧临时启动/选择脚本没有后台保证，后续不得原样重用。无子代理。

## 尚未解除的验收缺口

1. **新增阻塞：恢复副本生成历史列表报Error: GENERATION_REQUEST_FAILED。** 两稿正文可读且哈希一致，只读观察到归档脱敏后的generation_runs.binding_json为空；初步定位为owner.list按活动RunBinding解码与冻结历史读面衔接缺口，尚未修复或补回归。不能把正文恢复成功写成生成历史恢复成功，更不能补造执行权威。
2. 完整审稿→作者确认→修稿→差异合并→复核GUI未验收；四生产审稿频道继续关闭，不为验收绕过门禁。
3. 暂停/恢复、失效会话交错GUI尚未完成。流式取消已验证不等于暂停/恢复链通过；含冻结历史、脱敏审稿和资产的复杂归档尚未完成。
4. project/epoch/artifact已有新增有界回归，但完整威胁枚举未穷尽，不是全链安全认证。真实模型、外部MCP、业务云与发布未授权；macOS/Linux未认证。

## 交接给下次会话

- master；代码HEAD/origin为24291a43，随后仅本次文档收口。活动队列与本机证据入口见`.pi/tmp/pipeline-state.md`；无子代理、无测试服务在飞。
- 优先修复便携恢复后的生成历史只读投影：从字段策略、冻结元数据、owner.list及仓储解码逐层对照基线，先补“含真实生成记录的导出→恢复→list/read”合成回归。不得跳过坏记录或把脱敏历史升级成可重放运行；涉及schema/权限/ADR的新变化仍先确认。
- 之后按审计矩阵推进暂停/恢复及复杂归档；GUI只有后台安全方案或新一轮明确临时授权后再启动。继续保留生产审稿门禁与外部业务限制。
