# 当前交接入口

- 最新快照：[2026-10-14-tauri-migration-status.md](./2026-10-14-tauri-migration-status.md)，第 49 次。
- 已验证代码提交：`a1884ac96d1b1c7c7342b6d07a71b47096fdf815` / master；关闭切片 `cf23c0ba` 已推送。短数据库访问代码与本快照一起推送 origin/master；最终 HEAD 以 Git 和本机 pipeline 为准。
- 最近完整验证（代码 `a1884ac9`，2026-10-14 21:55）：Rust 1267 passed / 0 failed，check/typecheck/lint/check:channels/定向 rustfmt 通过；定向前端三文件 43 项通过，不代表全量前端或 GUI 验收。本次验证频率文档更新没有重跑代码测试。
- 用户已明确授权后续普通迁移切片自检通过后直接提交、推送，免逐次人工审核；先确认边界及禁止派发代理仍保留。
- Rust 改为分层验证：[开发指南 §7](../agents/pi-development.md#7-测试约定)。普通切片默认相关测试；阶段收口、累计 5 个 Rust 代码切片、最终验收、异常跨域回归或关键安全规则变更才跑全量。全量基线/累计数仅见本机 pipeline；纯文档不计数。
- 频道实测：243 注册命令 / 242 invoke 覆盖 / 48 未迁移；契约 invoke 290、事件 7。
- 活动任务和运行占用只见 [.pi/tmp/pipeline-state.md](../../.pi/tmp/pipeline-state.md)。主会话直接执行，不派发代理。

## 下一步与未解除阻塞

1. 服务层已支持短数据库访问回调，生产 owner/IPC 尚未切换，网络长锁仍存在。下一步拆开 owner 准备、分发和回执投影；回调逐次按 owner → 数据库重新认证项目/会话/epoch/连接，避免回写旧库。
2. 短锁化接线时关闭钩子须先中止并在不持业务锁时等待控制登记退出，再挂起 owner、关闭库；补真实 AppState 并发读取/暂停/关闭/切换及本地 HTTP 假服务器测试。其他子代、内部 Agent、通用排队竞争和关窗 GUI 未完成。
3. 快照/推理事件、材料提示哈希、短细纲重试、知识采集与特殊域准入未完成；既存扩展前端频道/参数/review camelCase 与 Agent store 失败未复验。B3-M4/M5、真实 Windows 更新、dirty 关窗和整屏瞬黑 GUI 专项待续；B3 不是端到端完成。
4. 普通迁移切片按新授权自动提交推送；新依赖/schema/权限/CI/ADR/旧 Electron 清理等仍须先确认。

频道事实：[频道清单](../plans/tauri-migration-channel-inventory.md)；开发/交接规则：[pi-development.md](../agents/pi-development.md)。
