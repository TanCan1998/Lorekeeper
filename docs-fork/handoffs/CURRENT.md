# 当前交接入口

- 最新：[2026-10-17-tauri-migration-status.md](./2026-10-17-tauri-migration-status.md)，第 **82** 次。
- 代码已提交推送：15ed11f4旧库不可用边界、729cda50 SSE夹具/CI触发补齐；代码验证SHA=`729cda50233747a29e2c46c7130498bdffa3bbdd`。之后收口仅文档。
- **首次云端成功基线**：[38946375676](https://github.com/TanCan1998/Lorekeeper/actions/runs/38946375676)，Rust **1614/0**、all-targets无告警，artifact两个退出码均0且SHA相符；语义切片累计0。同SHA提交规范/秘密扫描通过。
- 本机定向沿用同代码22+13通过，前端187文件2108通过/1跳过、typecheck/lint通过；本轮格式/CI合同及开发build通过。
- 真实Tauri合成GUI已过：旧库禁用说明、新建/配置保存、归档导出/新副本恢复、打开副本、正常关闭重启后重开；仅配置级，不代表完整审修复核/冻结历史验收。
- **用户新要求：后续真实窗口仅后台验证，不置顶/激活/抢焦点，不使用全局键鼠/SendKeys。无法后台完成的原生交互列待验收；旧GUI脚本不得直接复用。** 测试应用已正常关闭。
- [证明/GUI审计矩阵](../plans/2026-10-17-tauri-proof-audit.md)记录已核查边界及剩余项。跨project/epoch/artifact审计、完整审修复核/暂停取消恢复/失效会话GUI、复杂归档仍待完成。
- 功能289/290、业务285/290不变，注册291/契约注册290/事件7；旧库blocked不算功能迁移。四生产审稿频道继续关闭，真实模型/MCP/业务云及发布未授权。
- [pipeline](../../.pi/tmp/pipeline-state.md)登记本机运行与证据；无子代理。根README不记录动态进度。
