# 当前交接入口

- 最新快照：[2026-10-14-tauri-migration-status.md](./2026-10-14-tauri-migration-status.md)，第 50 次。
- 本轮定向验证代码：`4f48161e67f7a401fb3753449a1520a3223433e8` / master；代码与交接文档一并推送 origin/master，最终 HEAD 以 Git/本机 pipeline 为准。
- 本轮 Rust：owner 69、命令 3、运行服务 32，共 104 项通过；check 零告警、定向 rustfmt/diff 检查通过；未跑全量、TS/前端或 GUI。
- 最近全量是 `a1884ac9`（2026-10-14 21:55）：Rust 1267 项通过；该版本 typecheck/lint/频道检查与定向前端 43 项通过，不能归为本轮代码全量结果。
- 普通切片按用户授权自检后自动提交推送；Rust [分层验证](../agents/pi-development.md#7-测试约定)不逐切片全量，累计数/基线仅见 pipeline；先确认边界和禁止代理仍保留。
- 频道沿用上述历史实测：243 注册 / 242 invoke 覆盖 / 48 未迁移；契约 invoke 290、事件 7。本轮无注册改动。
- 活动任务和运行占用只见 [.pi/tmp/pipeline-state.md](../../.pi/tmp/pipeline-state.md)。

## 下一步与未解除阻塞

1. owner 准备/分发/投影已拆分，准备对象不借用 owner/连接；生产 IPC 尚未释放业务守卫，网络长锁仍存在。下一步为准备对象接服务短回调入口，命令层准备后释放锁；每次持久化/投影按 owner → 数据库重认证项目/会话/epoch/连接。
2. 关闭钩子须先中止，再在不持业务锁时等待执行控制退出，之后挂起/关库；补真实 AppState 读取/暂停/关闭/切换与本地 HTTP 并发测试，接线阶段收口跑全量。其他子代、内部 Agent、通用排队与关窗 GUI 未完成。
3. 快照/推理事件、材料提示哈希、短细纲重试、知识采集和特殊域准入未完成；既存扩展前端频道/参数/review camelCase 与 Agent store 失败未复验。B3-M4/M5、真实 Windows 更新、dirty 关窗和整屏瞬黑 GUI 专项待续；B3 非端到端完成。
4. 新依赖/schema/权限/CI/ADR/旧 Electron 清理仍须先确认。

频道事实：[频道清单](../plans/tauri-migration-channel-inventory.md)；开发/交接规则：[pi-development.md](../agents/pi-development.md)。
