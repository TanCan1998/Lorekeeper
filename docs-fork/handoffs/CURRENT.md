# 当前交接入口

- 最新：[2026-10-17-tauri-migration-status.md](./2026-10-17-tauri-migration-status.md)，第 **85** 次（同日改写当前摘要）。
- 当前代码已推送：`61ab33bc822f3864cc2b6c309aa30d265eacbb1b`，修复按运行读取候选时先解析脱敏绑定/预算的问题：`list_candidates(Some(run_id))` 先通过统一 `frozen_row` 跳过冻结运行。实际脱敏流水线回归先失败后通过，无守卫损坏绑定与缺失运行仍报错，冻结写保护和源库保持不变。
- 本轮本机定向：仓储30/0、`generation_owner`过滤187/0、便携73/0、run服务38/0；2文件rustfmt与diff通过。无前端改动，不重跑前端测试。
- **当前SHA云端待完成**：[Tauri Rust CI 38999595785](https://github.com/TanCan1998/Lorekeeper/actions/runs/38999595785)，最后查询为运行中。最近已成功全量仍是`183d4886` / [38997082671](https://github.com/TanCan1998/Lorekeeper/actions/runs/38997082671)：Rust1637/0、all-targets无告警，artifact已核对。之后Rust语义切片累计 **1**，未清零。
- 上轮已修复恢复副本的生成列表失败及再次打开失败，并完成真实开发窗口复验；本轮仅实际脱敏数据库合成回归，未启动GUI、不扩大GUI验收结论。默认后台、不置顶/激活/抢焦点，无子代理。
- [证明/GUI审计矩阵](../plans/2026-10-17-tauri-proof-audit.md)记录有界结论；暂停/恢复GUI、失效会话交错GUI、复杂归档与完整审修链仍待验收。四生产审稿频道继续关闭。
- 功能289/290、业务285/290，注册291/契约注册290/事件7，不变。真实模型/MCP/业务云与发布未授权，macOS/Linux未认证。
- [pipeline](../../.pi/tmp/pipeline-state.md)登记本机证据和下一步；根README不记录动态进度。
