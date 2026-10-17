# 当前交接入口

- 最新：[2026-10-17-tauri-migration-status.md](./2026-10-17-tauri-migration-status.md)，第 **87** 次（同日改写当前摘要）。
- 已推送两片：`7fb424dd`修复冻结恢复候选的只读展示，保留正文哈希校验、强制sourceCurrent=false；`43a00848a73ec331295d15182d0fbf9164382b12`增加复杂归档往返/资产漂移及新旧候选混合回归。无schema、权限、租约或定稿规则改动。
- **最新云端成功基线**：[39012654458](https://github.com/TanCan1998/Lorekeeper/actions/runs/39012654458)，SHA=`43a00848`，Rust **1643/0**、all-targets无告警；run成功，artifact expectedSha/testedSha、两个退出码0/state=passed均已下载核对。提交规范/秘密扫描同SHA通过，语义切片累计0。
- 本轮本机：恢复候选仓储6/0、便携最终77/0、db命令30/0，格式/diff通过；隐藏配置开发构建及恢复默认配置构建通过。无前端源码改动，不重跑TS。
- 复杂合成归档：草稿/审稿材料、冻结生成/导入历史、恢复候选、正式/孤儿头像与原始资产，导出→恢复→重复重开→再导出→第三身份恢复均通过；头像漂移拒读、拒覆盖、拒再导出，源库/资产不变。**这是后端往返回归，不是完整审修或原生归档GUI验收。**
- 真实隐藏WebView后台验证：会话构建覆盖visible=false/focus=false，Win32与Tauri API确认主窗口隐藏、不聚焦；同路径关闭/重开更换lease，延迟旧读/写均被拒，新数据未变。未用全局键鼠/SendKeys，窗口正常关闭、三测试端口均已收口；已恢复默认开发构建，后续不能把默认二进制直接当隐藏版启动。
- **新实测阻塞：批量创作无法启动**。GUI选择草稿待审后返回GENERATION_REQUEST_FAILED，草稿/批次/模型请求均0；`begin_batch → begin_internal`的遗留通用分支仍无条件拒绝batchIntent/batchId。暂停/恢复未通过。修复涉及批次根/子代准入，须先具体确认；不自行放开自动定稿/生产审稿。
- [审计矩阵](../plans/2026-10-17-tauri-proof-audit.md)与[频道清单补充](../plans/tauri-migration-channel-inventory.md#批量生成实际接线缺口第87次)明确此缺口。既有频道扫描口径289/290、前端路由285/290不代表批量业务已可用；四审稿频道仍关闭。
- 真实模型/MCP/业务云与发布未授权，macOS/Linux未认证。无子代理，无本机服务在飞。[pipeline](../../.pi/tmp/pipeline-state.md)登记下一步与本机证据。
