# 当前交接入口

- 最新：[2026-10-17-tauri-migration-status.md](./2026-10-17-tauri-migration-status.md)，第 **89** 次（同日改写当前摘要）。
- 当前代码 `fa5cf8aa698d0e2d4748593bcc179d32533209ce` 已推送：按具体授权补齐图生成内部双字段/UUID准入、事务内单次上下文捕获和摘要；行内上下文变量稳定序列化、冻结模型身份读取；两入口nonce查询跳过冻结旧根/run。通用begin允许集不变，不改schema/依赖/文件权限/定稿规则。
- **最新云端成功基线**：[39018418991](https://github.com/TanCan1998/Lorekeeper/actions/runs/39018418991)，SHA=`fa5cf8aa`，Rust **1656/0**（库249.54秒）、all-targets无告警（36.78秒）；artifact完整SHA、两个退出码0及state=passed均已下载核对。同SHA提交规范/秘密扫描通过，语义切片累计0。
- 本机：owner198/0、源绑定4/0、sources6/0、图17/0、行内25/0、portable77/0、run38/0、generation命令4/0；4文件rustfmt/diff与all-targets通过。新增5项专用owner及2项哈希回归，覆盖稳定重读、真实脱敏/冻结共存、幂等冲突、回滚/租约、跨项目与作者源漂移拒绝。
- **真实隐藏WebView正式IPC/回环执行通过**：图与行内根创建/重读/幂等，行内20次稳定重读，伪造通用准入/选区/模型切换/外项目句柄拒绝；各1次物理请求、各1份候选。图结果为deterministic兜底，不是模型结构化质量证明；未确认图效果、应用行内输出或定稿，草稿/定稿均0。不是完整UI确认/应用链验收。
- 前片 `e9b67f91` 修复导入生成槽位读取被冻结中和绑定阻断；独立云端[39015932333](https://github.com/TanCan1998/Lorekeeper/actions/runs/39015932333)=1649/0且artifact已核对。第88次批量暂停/继续/重开持久化GUI沿用`80dab6d5`实测，不冒充本轮重复验证。
- Tauri/Win32确认交互主窗口隐藏、未聚焦/非前台，无全局键鼠；窗口正常关闭、5190/9226/19537无监听，默认开发构建已恢复。以后后台验证须重新隐藏构建，无子代理/本机服务在飞。
- [审计矩阵](../plans/2026-10-17-tauri-proof-audit.md)及[专用接线说明](../plans/tauri-migration-channel-inventory.md#图生成与行内编辑专用接线第89次)已更新。完整审修、复杂原生归档及专用确认/应用UI仍未验收；四生产审稿频道和自动定稿保持关闭，真实模型/MCP/业务云/发布未授权，macOS/Linux未认证。[pipeline](../../.pi/tmp/pipeline-state.md)登记精确证据与后续入口。
