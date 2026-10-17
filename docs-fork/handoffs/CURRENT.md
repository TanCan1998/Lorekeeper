# 当前交接入口

- 最新：[2026-10-17-tauri-migration-status.md](./2026-10-17-tauri-migration-status.md)，第 **86** 次（同日改写当前摘要）。
- 当前代码已推送：`e5562957aafd16090b33fe71d4afdc3c25c282ba`，补齐基线`listResumable`的冻结导入过滤；历史`get`保持只读可查，新身份仍可列举/启动，原写保护不变。连接级`frozen_row`抽至便携守卫模块，生成仓储继续保持注入缝∨连接守卫。
- 本轮本机定向：导入仓储50/0、生成仓储30/0、便携74/0、`generation_owner`过滤187/0、run服务38/0、db命令30/0；4文件rustfmt、diff及本机all-targets通过，无告警。实际脱敏流水线回归先失败后通过，覆盖两用途×四状态。
- **当前SHA云端待完成**：[39000071204](https://github.com/TanCan1998/Lorekeeper/actions/runs/39000071204)，最后查询运行中。**最近成功全量已更新**为`61ab33bc` / [38999595785](https://github.com/TanCan1998/Lorekeeper/actions/runs/38999595785)：Rust **1638/0**、all-targets无告警，run成功且artifact SHA/两个退出码/state已核对。之后Rust语义切片累计 **1**，不归零、不把上版结果归给当前版。
- 上轮`61ab33bc`已修复按运行读取候选时先解析脱敏绑定/预算的问题；更早的生成列表/再次打开失败也已修复并经GUI复验。本轮未启动GUI、未派子代理，不扩大旧GUI结论。默认后台、不置顶/激活/抢焦点。
- [证明/GUI审计矩阵](../plans/2026-10-17-tauri-proof-audit.md)记录有界结论；暂停/恢复GUI、失效会话交错GUI、复杂归档与完整审修链仍待验收。四生产审稿频道继续关闭。
- 功能289/290、业务285/290，注册291/契约注册290/事件7，不变。真实模型/MCP/业务云与发布未授权，macOS/Linux未认证。
- [pipeline](../../.pi/tmp/pipeline-state.md)登记本机证据和下一步；根README不记录动态进度。
