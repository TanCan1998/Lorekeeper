# 当前交接入口

- 最新：[2026-10-17-tauri-migration-status.md](./2026-10-17-tauri-migration-status.md)，第 **84** 次（同日改写当前摘要）。
- 代码已推送：`a25eab90` 便携脱敏历史的列表读面；`183d4886` 便携冻结守卫覆盖写路径与项目关闭链（新增 `frozen_row`，`json_valid` 前置）。最终代码SHA=`183d488633dc26d264b12b68bfc2d0640ddfb1f5`，后续仅文档。
- **最新云端成功基线**：[38997082671](https://github.com/TanCan1998/Lorekeeper/actions/runs/38997082671)，Rust **1637/0**、all-targets无告警；artifact的expectedSha/testedSha一致、两个退出码0、state=passed，语义切片累计0。同SHA提交规范/秘密扫描通过。
- 本机定向：owner186/0、仓储30/0、审修59/0、便携72/0、db120/0、命令4/0、run服务38/0、知识源6/0、角色提案9/0；rustfmt与diff通过。前端沿用188文件2111通过/1跳过（本轮无前端改动）。
- **已修复（不再列为阻塞）**：恢复副本 `generation:list` 的 GENERATION_REQUEST_FAILED，以及随后暴露的「打开项目失败：项目数据库初始化失败」。根因是脱敏历史被写路径当可变历史解析（生产冻结只在连接级触发器，仓库注入缝恒 false）。
- 隔离合成GUI实测：恢复副本↔源副本连续切换均可打开，`generation:list` 0/4/0/0，无「打开项目失败」与 GENERATION_REQUEST_FAILED；恢复副本AI输出面板无错误、无可继续候选，源副本同一面板正常显示候选与"确认继续已选正文"；篡改leaseId被拒。**本轮临时前台授权结束，默认仅后台、不置顶/激活/抢焦点。** 窗口已正常关闭，无子代理。
- [证明/GUI审计矩阵](../plans/2026-10-17-tauri-proof-audit.md)记录本轮两项修复与有界结论；暂停/恢复GUI链、失效会话交错GUI、复杂归档与完整审修复核仍待验收。
- 功能289/290、业务285/290不变，注册291/契约注册290/事件7；旧库首版不支持，四生产审稿频道继续关闭。真实模型/MCP/业务云与发布未授权，macOS/Linux未认证。
- [pipeline](../../.pi/tmp/pipeline-state.md)登记本机证据和下一步；根README不记录动态进度。
