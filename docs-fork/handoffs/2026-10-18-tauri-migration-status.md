# Tauri迁移交接快照（2026-10-18）

> [当前入口](./CURRENT.md) · [混合验证](../agents/tauri-rust-ci.md) · [证明审计](../plans/2026-10-17-tauri-proof-audit.md) · [频道清单](../plans/tauri-migration-channel-inventory.md)。第 **101** 次交接；同日第100次摘要见提交1e2e1dc0，上一日为[第94次](./2026-10-17-tauri-migration-status.md)。

## 当前代码与已验证状态

代码 **b5850fbed96495e0a1e36ada51c4de9ba449765f**，master，已推送。当前文档不改变Rust代码指纹；本轮包含两个后端切片和一次Windows隐藏WebView验收，没有真实模型/MCP/业务云调用或发布。

| 验证 | 结果 | 版本/证据 |
|---|---|---|
| 最新云端全量 | 1690通过/0失败，库196.58秒；all-targets35.54秒无告警 | b5850fbe / [39102768655](https://github.com/TanCan1998/Lorekeeper/actions/runs/39102768655)，2026-10-18 06:11 UTC完成 |
| 前片云端全量 | 1689/0，库260.91秒；all-targets37.99秒无告警 | 58994666 / [39102267581](https://github.com/TanCan1998/Lorekeeper/actions/runs/39102267581)，06:03 UTC完成 |
| 本机定向 | finalization26/0、portable90/0、generation_owner213/0；后片review_revision72/0、portable90/0、generation_owner213/0 | finalization-read-*、review-parent-*日志；定向rustfmt/diff通过 |
| 当前后台GUI | 新归档服务导出/恢复后，作者编辑保存、项目关闭重开、应用正常退出重启、旧租约拒绝、定稿不变通过 | b5850fbe隐藏开发构建；current-rules-gui-1792303725757 |
| 本机构建 | 隐藏构建通过；验收后默认构建恢复通过（32.29秒） | current-gui-build.log、current-gui-default-build.log |
| 前端 | 本轮未改，未重跑TS；历史190文件2121通过/1跳过、typecheck/lint沿用8ecfc0fb | 不是新前端全量 |
| 代码规范/秘密扫描 | 两片均success | 58994666：39102267567/39102267572；b5850fbe：39102768641/39102768697 |
| 双栈隔离 | electron、根src/docs、根配置/lock无差异 | 相对1e2e1dc0保护路径差异检查 |

两片产物已分别下载核验：58994666包31674字节，b5850fbe包31677字节，均与GitHub SHA256匹配。完整expectedSha/testedSha、两个独立退出0、lock哈希、state=passed及两日志已核对。Rust语义切片累计 **0**（b5850fbe全量已核验）。

## 本次授权与定稿后处理读面

用户在总进度明确列出剩余读面、后台GUI及待授权定稿后处理修复后回复“全部进行”。本次仅把已说明的read_finalization_effect读取校验及合成回归纳入；不追溯把旧确认卡取消当同意，不扩大为定稿正文/不可逆生命周期/schema/文件权限修改。仍不派子代理，不允许抢焦点或真实外部业务调用。

**58994666**：

- 两类后处理（chapter_notes、character_cards）均先复现坏JSON返回Ok(None)，再改为GENERATION_FINALIZATION_EFFECT_INVALID；畸形SavedEffect也不再被吞。
- SQL NULL保持既有“尚无回执”合同；JSON null回执拒绝。finalizationEffect的null/false/0/空字符串沿用基线假值无声明行为；已声明的对象/数组等必须完成解码与后续证明。
- 合成SQLite回归覆盖坏JSON/空白/JSON null/畸形声明、可变性判断不误放行及零回写；没有重写任何定稿事实。既有来源、产物身份与生命周期门禁不变。
- 此为读取/声明校验的有界回归，不等于穷尽所有定稿后处理证明字段。

## 审修确认与复核血缘查找

**b5850fbe**：

- 原parent查找对全库usage回执使用.ok()，当前其他根的坏JSON会被略过；作者确认分支已先失败复现。
- 抽取私有review_effect_attempts，逐行先排认证冻结attempt/run，再严格解析当前JSON；仅按kind/id定位声明，保持匹配数量唯一性与后续完整证明。
- SQL NULL内部合法null不声明血缘。确认分支坏JSON返回Error: GENERATION_REVIEW_LINEAGE_UNPROVEN；复核分支既有计划完整性检查会更早返回Error: REVIEW_CYCLE_CONFLICT，保留错误优先级，不为测试改生产门禁。
- 真实审稿→确认→修稿→合并链加独立根账本，覆盖两个分支坏JSON/空白、无写入、NULL无声明仍可读取合法血缘。
- 最初无关运行夹具sourceManifest为空被GENERATION_BINDING_INVALID先拒绝，已修正夹具；后续复核错误优先级的断言也据实际合同调整。两者不冒充目标缺陷证据。

## 新归档规则与恢复后创作的后台GUI

仅隔离合成项目、当前正式服务与真实Tauri开发WebView，无真实模型服务：

1. 用本机不入库Rust辅助程序链接当前库，调用正式export_portable_project→restore_portable_project→ProjectDatabase::open，创建**新规则**归档及新项目身份的恢复副本。不是对话框选取或完整命令层授权流程。
2. 归档2231498字节，SHA256=c98dd7391221ca69a0f389818099c93f06fe06d65dc01c1cff61feed4dffd8cb；恢复projectId=e8be2614-1a0a-4509-a085-488f8a87a59c，与源身份不同。
3. 隐藏WebView通过正式项目store打开副本，实际CodeMirror中编辑第二章作者稿，点击真实保存按钮；generation:list为空（冻结历史无可继续候选）。
4. 正式项目关闭/重开后仓储正文和真实编辑器均与保存内容一致；新lease不同，旧lease读取拒绝，第一章定稿完整内容/元数据不变。
5. 标题栏正常退出应用后，以同隔离home重新启动隐藏窗口，重复验证持久化、定稿不变及旧lease拒绝通过。
6. Tauri visible=false/focused=false，Win32目标进程非前台、Lorekeeper主窗口不可见；没有SetFocus/前台激活/全局键鼠/SendKeys。
7. 恢复库正式连接确认33个generation run全部认证冻结、1个非定稿作者稿可变且依赖为[]；改写历史被PORTABLE_RUNTIME_FROZEN拒绝、total_changes不变。
8. 窗口正常关闭，独占Vite进程按PID/命令行所有权核对后停止；5190/9226均已收口，无本机Lorekeeper进程；已恢复默认构建。

辅助程序最初链接缺Cargo原生CRT参数，后改为复用既有build输出参数；第一次混用斜线路径被正式安全检查拒绝，改为规范Windows路径后通过。首个启动脚本等待Vite超时留下本次独占Vite，核对后续启隐藏窗口，最后已停止。未为夹具改动生产路径/链接/权限规则。

**仍未计为通过**：原生save/pick对话框在新规则下的完整点击导出/恢复链。其创建可能激活窗口，在“不抢焦点”限制下不擅自启动；本轮服务层准备+GUI恢复后编辑不冒充原生对话框验收。复杂头像/审修原生归档沿用第91次历史证据，不归为本轮新测。

## 总进度与未解除项

- #25–29保持完成。#30当前已补齐上述后端缺陷与新副本后台编辑/重启证据；原生对话框约束等仍留明确待验收，暂不登记全项关闭。#31最终验收不提前宣布完成。
- 频道功能迁移289/290；旧库转换明确首版范围外。四审修频道已于第91次接线开放，旧文档的285/290关闭状态是历史，不反向覆盖当前源码。
- 不用测试数量或命令注册数推出所有JSON消费者、交错、生产质量或发布准备已认证。macOS/Linux未认证，真实模型/MCP/业务云/发布未授权。
- 无需为普通切片再次等“继续”；只有实际触及具体确认/无法后台交互边界才停下说明。

## 证据与接续

- 本机证据：.pi/tmp/finalization-read-{red,finalization,portable,generation_owner}.log、review-parent-{red,review_revision,portable,generation_owner}.log；云端.pi/tmp/recovery/ci-{58994666,b5850fbe}/。
- GUI：.pi/tmp/recovery/current-rules-gui-1792303725757/内archive-service.log、gui-edited.json、gui-reopened.json、gui-app-restarted.json、gui-win32.json、guard-verify.log等；会话登记.pi/tmp/current-rules-gui-session.json已closed，PID仅历史。
- 隐藏二进制SHA256=95d471d5bd8f3075fd560e63b4f6cb83b0416f3bb585d4e51eecc9e37593e205；验收后已恢复默认构建，不把当前默认exe当隐藏指纹。
- 当前活动事实源.pi/tmp/pipeline-state.md；上述辅助源码、构建、归档、合成正文和日志均不入库。
