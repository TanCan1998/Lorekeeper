# 主生成普通 execute 切片验证

## 基线与写面

- 实际 HEAD：`62f5b8e072e590ac9a7d14a10eeaa25387b37843`；原 CURRENT / pipeline 中 `efb96e0d` 已过期。
- 接续时已有三份未提交 Rust 改动：`commands/generation.rs`、`services/generation_run_service.rs`、`services/main_generation_owner.rs`。保留并复验，没有派发代理。
- 现有切片将普通 owner execute 接到计划构造、provider 请求、阻塞 runtime 分发与回执投影，加入语义任务校验、规划目标指令和幂等调用读回。
- 本轮修正 `materialDecision: null` 被当作有效材料决策的问题；新增非法语义任务在预算预留/分发前拒绝且不留下 pending 登记的测试。

## 验证

- `cargo check --all-targets`：exit 0（新增测试前执行）；完整 `cargo test`：exit 0，1248 passed / 0 failed，包含新增测试。
- `pnpm typecheck`、`pnpm run lint`、`pnpm run check:channels`：均 exit 0。
- 三份 generation runtime / renderer / harness 测试：43 passed / 0 failed。
- 证据：`.pi/tmp/b33-execute-{check,test,typecheck,lint,channels,vitest}.log`。
- 未调用真实外部模型，未进行 GUI 或完整前端套件验收。既存扩展前端失败未解除。

## 未完成与下一步

1. **优先解决锁生命周期**：目前 generation_execute 在持有 owner 守卫和项目数据库锁时执行网络等待。`block_in_place` 只释放 Tokio 调度容量，不释放业务锁；暂停/取消、项目关闭及同项目读取可能等待整个网络执行结束。不能宣称运行控制或端到端完成。
2. 网络 await 前分离执行上下文；持久化 flush/settle 重新认证项目身份，允许取消控制不依赖网络等待期间持有的锁。需要保持数据库/epoch 安全，不得简单释放后继续写旧项目。
3. 快照/推理事件接线尚未完成；材料决策提示哈希与短细纲重试仍 fail-closed，特殊域准入仍未开放。
4. 补充 owner → 服务合成分发/回执/重放专项测试，再做本地流式假服务器和生命周期并发验证；本次全量 Rust 单测不等于新增网络路径端到端验收。
5. 用户已通过确认卡确认逐项核验并同意提交；本切片提交后继续处理锁生命周期，不推送。

无新增依赖、schema、权限、CI 或 Electron 清理；主会话继续，不派发。
