# 当前交接入口

- 最新：[2026-10-17-tauri-migration-status.md](./2026-10-17-tauri-migration-status.md)，第 **88** 次（同日改写当前摘要）。
- 已按用户具体授权推送 `80dab6d534091d9d81dc9dcb068033ecdae6483e`：接通 `draft_review` 批次根/子代准入，绑定父根、项目、模型、章节顺序、前驱草稿、冻结作者输入与同根预算；`auto_finalize`仍拒绝。不改schema、依赖、会话/文件权限或定稿规则。
- **最新云端成功基线**：[39014411676](https://github.com/TanCan1998/Lorekeeper/actions/runs/39014411676)，SHA=`80dab6d5`，Rust **1648/0**（库254.45秒）、all-targets无告警（38.98秒）；artifact expectedSha/testedSha、两个退出码0及state=passed均已下载核对。同SHA提交规范/秘密扫描通过，语义切片累计0。
- 本机定向：owner192/0、agent56/0、草稿效应2/0、run38/0、generation命令4/0、portable77/0；4文件rustfmt/diff及all-targets通过。新增5项真实owner批次回归，包括两章真实提交、共享预算、错误准入无副作用、事务回滚与冻结历史不干扰新批次；当前坏绑定/收据仍拒绝。
- **真实隐藏WebView后台暂停/恢复通过**：正式UI选择两章草稿待审，第一章写完暂停，稳定等待后仍仅一稿；继续完成两稿，同一根、nextChapterNumber=null，定稿始终0。正常关闭/重开后两稿和批次持久化一致，旧租约读写拒绝。仅本机合成模型，无真实模型调用。
- Tauri主窗口持续visible=false/focused=false；同SHA另按Win32标题/类确认交互主窗口隐藏且非前台，不把Tao内部事件目标当用户窗口。未用全局键鼠/SendKeys。窗口正常关闭、5190/9226/19537无监听，已恢复默认开发构建；以后后台验证须重新隐藏构建。
- 前轮复杂合成归档后端往返/资产漂移与恢复候选证明沿用43a00848，不冒充本轮原生GUI验收。[审计矩阵](../plans/2026-10-17-tauri-proof-audit.md)与[批次频道状态](../plans/tauri-migration-channel-inventory.md#批量生成实际接线缺口第87次)已更新；扫描口径289/290、前端路由285/290及四审稿频道关闭不变。
- 完整审修链与复杂资产原生归档GUI仍未验收；真实模型/MCP/业务云/发布未授权，macOS/Linux未认证。无子代理、无本机服务在飞。[pipeline](../../.pi/tmp/pipeline-state.md)登记精确证据目录与后续入口。
