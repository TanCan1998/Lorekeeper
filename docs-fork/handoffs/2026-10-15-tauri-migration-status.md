# Tauri 迁移进度快照（2026-10-15）

> 第 52 次；按用户“收口交接”要求停止开发并更新。同日替换当前摘要，第51次内容保留于 Git 历史（`d4ea9dfd`）；旧日期快照冻结。
> 当前入口：[CURRENT.md](CURRENT.md)；维护规则：[开发指南 §9](../agents/pi-development.md#9-交接状态与快照维护)。本轮由主会话实施，没有派发子代理。

## 已验证状态（第52次）

最新代码：`87c43d87b23081f6e8990b66b1e2bb2d7f694afe` / `master`，已推送 `origin/master`（首次推送 TLS EOF，重试成功）。文档更新前工作区干净；最终文档 HEAD 以 Git 为准。以下结果均核对自本机日志，日期为 2026-10-15。

| 指标 | 结果 | 验证版本 / 范围 / 时间 | 证据（本机，不入库） |
|---|---|---|---|
| 最新前端定向 | **3文件、60测试通过**；typecheck、lint 退出0 | `87c43d87`；助手客户端、Agent store、草稿主生成工作流；17:35 +08:00 | `.pi/tmp/b83-{target,typecheck,lint}.log` |
| 前一批前端组合 | 4文件、64测试通过；含恢复产物打开会话隔离；typecheck、lint通过 | `f8fbb83d`；17:32 +08:00；不与最新60项加总 | `.pi/tmp/b82-{target,typecheck,lint}.log` |
| 最近前端全量扫描 | **170文件中7文件失败；2019测试：1895通过、123失败、1待定**；退出1，未全绿 | `f15a1b71`；17:24 +08:00；后续切片仅做定向，不冒充最新HEAD全量 | `.pi/tmp/b78-results.json`、`.pi/tmp/b78-full.log` |
| 本轮 Rust 定向 | 序列化合同2项、对账5项通过；`cargo check --all-targets` 零告警；定向rustfmt通过 | `f3be4dd3`；17:14 +08:00；本轮唯一Rust切片 | `.pi/tmp/b73-{serialization,reconciliation,check,rustfmt,contract}.log` |
| 最近全量 Rust（沿用） | **1381 passed / 0 failed，56.01s** | `90792744`，此前草稿链路阶段收口；此后累计Rust切片 **1**，未重跑全量 | `.pi/tmp/b53-workflow-full.log` |
| 频道 / 事件计数（沿用） | 243注册、242 invoke覆盖、48未迁移；契约invoke290、事件7 | `a1884ac9` 的2026-10-14历史实测；本轮未改变注册或契约数，未重新计数 | [频道盘点](../plans/tauri-migration-channel-inventory.md) |
| 传输 / 静态合同 | 已修复工作流命名参数、Rust入参驼峰与前端静态合同；明确区分Rust已注册和渲染层已开放 | `27f24383` 至 `56eca579`；各批定向/typecheck/lint通过 | `.pi/tmp/b72-*` 至 `.pi/tmp/b79-*` |
| GUI / 真实模型 / MCP | 未验收 | 合成Tauri传输和本地HTTP不能替代真实模型、主进程SQLite或完整产品端到端证据 | 本轮无新增验收结论 |

Rust验证目录为 `tauri-app/src-tauri/`，显式设置 `RUSTUP_HOME=D:/Environment/rust/rustup`、`CARGO_HOME=D:/Environment/rust/cargo`；前端在 `tauri-app/` 执行 `pnpm exec vitest run <文件>`、`pnpm typecheck`、`pnpm run lint`。本次收口为纯文档，不重复运行代码测试。差异、链接、版本、提交消息和泄漏扫描另行核验。

## 本轮交付与关键决策

1. **草稿真实Tauri传输接线**：草稿命令、工作流登记、父根恢复、历史句柄和暂停/订阅清理回归接到下划线命令名及命名参数。`generation:begin` 使用 `selection`，对象命令使用 `request`，携带冻结项目会话。关键提交：`e6291a70`、`991cce34`、`fd87e00d`、`21f8cf9e`、`56d3f93c`、`6cdb08fb`、`27f24383`。
2. **未知结果只补导航、不重放写入**：running/unknown动作保留已登记草稿导航，仍标记 `result_unknown`；按登记、项目和完整会话去重，保留既有收据、其他登记和历史lease。产物点击只打开持久候选面板，不开始任务、不读文件；旧lease不借用新会话。关键提交：`c0ddffb7`、`6c7ce201`、`e2b3daa1`、`88727b14`；证据 `.pi/tmp/b80-*`。
3. **恢复身份与工具回执分别校验**：恢复允许同父根历史epoch/runId，但首次read/resume也必须匹配请求父根，空父根不关闭匹配。工具claim/finish回执精确比较projectId、epoch、rootActionId、runId、attemptId、toolCallId。请求在首次await前复制身份，构造时复制冻结会话，防调用方突变漂移。关键提交：`e665c760`、`e57811dc`、`b9260577`、`f8fbb83d`、`87c43d87`；在途突变14项、会话突变3项、空父根2项均先复现失败再修复。证据 `.pi/tmp/b81-before.log`、`.pi/tmp/b82-before.log`、`.pi/tmp/b83-before.log` 及各批定向日志。
4. **取消与事件生命周期**：认领在途取消、同项目更换lease后不调用工具、不进入下一轮；同步工具异常也清理abort监听。事件取消立即封闭排队回调，幂等释放；异步注册失败显式记录。关键提交：`66524e69`、`5f923337`。
5. **休眠契约修复，不等于开放特殊域**：Rust的修稿提交请求及连续性视图补齐camelCase，对账原始结构体补显式声明；修复角色提案/蓝图/首轮参数断言，静态合同改查Rust命令和仓储；品牌核对Lorekeeper身份，Storybook仅静态引用保留的上游文档。关键提交：`f3be4dd3`、`0041ae03`、`20374c2b`、`f15a1b71`。没有更改schema、权限、依赖、CI、ADR或正式生命周期。
6. **守住未开放边界**：四个review-revision频道虽已在Rust登记，前端仍无命名参数映射；图谱、定稿派生等特殊域仍拒绝。新增真实适配器零传输断言，不以补齐映射来让失败测试通过。关键提交：`2a45b765`、`56eca579`。

此前生产短锁、关闭围栏、快照/推理事件、Agent首/后轮与恢复、材料/短细纲/对账、普通重启，以及已登记 `generate_draft → chapter-draft` 的有限准入与草稿保存到父Agent续轮保持原范围。真实SQLite与本地HTTP证据仍以 `90792744` 的阶段验证为准，本轮前端测试桩不扩大这些结论。

## 尚未解除的失败与阻塞

最近全量扫描仍有以下 **7文件 / 123失败**，未删除、跳过或放开生产边界来制造全绿：

| 测试文件（相对 `tauri-app/src/`） | 失败数 | 主要断点 |
|---|---|---|
| `services/__tests__/export-service-integration.test.ts` | 1 | 仍引入Electron控制器；`ipcMain.handle`不可用，未证明Tauri真实SQLite/授权写出 |
| `services/__tests__/narrative-thread-candidate-generator.test.ts` | 2 | `graph-generation:begin`前端未迁移 |
| `services/__tests__/plot-tree-generator.test.ts` | 1 | 同上，不擅自开放图谱 |
| `services/workflows/commands/__tests__/finalize-chapter.blueprint-entities.test.ts` | 1 | 后处理失败分支触达未适配的mock命令；需先核对特殊域链路 |
| `services/workflows/commands/__tests__/finalize-postprocess-character.integration.test.ts` | 15 | 定稿角色后处理特殊域及回执测试缺口，未声称真实持久化通过 |
| `services/workflows/commands/__tests__/mutation-failure-boundary.test.ts` | 63 | 全部受 `finalized-character:read-context` 或 `review-revision:prepare` 未开放阻断 |
| `services/workflows/commands/__tests__/refine-draft.command.test.ts` | 40 | `review-revision:prepare`前端拒绝；Rust已注册不等于完整迁移 |

计数来自 `b78-results.json`，不是最新HEAD重跑。此前旧快照的22文件/219失败不再作为最近扫描结果，但仍保留其历史日志。约83%仅为历史频道覆盖率，不是产品完成度；B3仍未完成完整端到端验收。

## 交接给下次会话

- 分支 `master`；最新代码 `87c43d87` 已推送，文档提交前无未提交代码。当前活动/待续入口：`.pi/tmp/pipeline-state.md`（本机，不入库）。未创建代理run；收口后无本会话主动开发任务。
- **首要待续**：核对 `AgentGenerationClient.close()` / `cancel()` 后是否还允许新的 `begin()` / `resume()`；仅开始读取源码和定位，**尚未写测试或修改实现，问题未确认**。保留迟到启动收据的取消能力，不把必要结算误封闭。会话todo #31待续。
- 继续已登记草稿在途/unknown、父根生命周期、恢复导航的组合回归；按修改范围运行定向/typecheck/lint。前端全量剩余失败应先判断是测试适配、真实迁移缺口还是尚未开放域，不能直接补映射绕过拒绝。
- `generate_blueprint`、`generate_architecture`、其他普通父根工作流、规划续作、知识采集、图谱/审稿/定稿派生特殊域不因持续迁移自动开放；新准入、schema、依赖、定稿/文件权限、CI/发布和ADR仍先确认。开发子代理派发仍禁止。
- 真实模型、完整GUI、MCP、Windows更新、dirty关窗、整屏瞬黑等未验收。Electron、根 `src/`、上游 `docs/`、根配置及锁文件保持隔离。

本次已按用户要求停止开发。仅完成交接文档检查、提交和推送；之后等待新的接续指令。
