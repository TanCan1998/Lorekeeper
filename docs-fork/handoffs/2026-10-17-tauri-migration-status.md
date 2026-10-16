# Tauri 迁移进度快照（2026-10-17）

> 当前入口 [CURRENT.md](./CURRENT.md)；规则见 [开发指南 §9](../agents/pi-development.md)。上一日快照：[第73次](./2026-10-16-tauri-migration-status.md)。

## 已验证状态（第 75 次 —— 必现/作者豁免混合复核与历史策略）

- master，接续时 HEAD/本地 origin/master 均为 `3b4c2265`，工作区干净；主会话直接推进，无子代理。
- 本轮普通内部证明校验切片按既有授权自检后提交推送；最终 Git 结果与 SHA 见本机 pipeline。
- **完整 proveEffect 审计尚未收口，审稿四频道继续关闭。**

| 指标 | 结果 | 验证范围/版本 | verifiedAt | 证据（本机，不入库） |
|---|---|---|---|---|
| 审稿域定向 | **58通过，0失败**，含新增7项混合/历史测试 | 3b4c2265 + 本轮四个Rust文件 | 2026-10-17 | .pi/tmp/recovery/mixed-directed.log |
| 格式与差异 | 四文件rustfmt --check、git diff --check通过 | 同一最终代码 | 2026-10-17 | 工具输出 |
| 最近Rust全量 | **1581通过，0失败**；库62.27秒，非本轮重跑 | afe6321d代码基线 | 2026-10-16 | .pi/tmp/recovery/chain-full.log |
| 最近all-targets | 无告警，非本轮重跑 | afe6321d代码基线 | 2026-10-16 | .pi/tmp/recovery/chain-check.log |
| 前端/GUI/频道 | 本轮未重测、未开放频道 | 沿用历史口径，不是本轮验收 | 历史 | 前次快照 |

本轮仅私有审稿证明逻辑与测试，定向完成编译；未改公共接口、schema、权限/会话准入及构建目标，按分层策略不重复all-targets/全量。本轮提交后全量以来累计2个Rust语义切片；阶段收口或满5个时再触发全量。

## 本次更新

- 新增 `review_revision_mixed_tests.rs`，用真实SQLite初审/确认/修稿/合并/复核链同时覆盖：已锚定目标、无原稿引文的必现目标、作者明确豁免的文风建议。
- 新执行维持保守策略：模型声称resolved不会自动解决问题；无锚点必现项保持unverified，作者豁免不再进入复核。
- **复现并修复作者豁免证明缺口**：原先只校验确认正文哈希与索引存在，不核对该索引实际指向的作者选择。现在双向验证确认快照与持久状态，并逐项核对schemaVersion/cycleId/findingId/origin/decision，拒绝越界、错项、把apply伪造为waive以及清除既有豁免。
- **复现并修复历史v1兼容问题**：历史效果验证不能沿用新执行计划对resolved项的过滤。提取内部重建函数，历史验证保留已解决项，正常新计划仍排除，未放宽执行准入。
- 合成重建v1冻结上下文、报告、证据映射与持久状态，验证历史resolved事实仍可证明和只读恢复，但不授权新执行；不涉及旧库导入、原始收据伪造或originalReceiptVerified提升。
- 新增7项回归：混合正常复核、v2模型resolved保守状态、豁免索引篡改、伪造豁免、历史v1正向、清除持久豁免、降级/未知回执策略拒绝。
- 漏洞复现证据：`mixed-before.log`（错误豁免索引仍通过）、`mixed-history-before.log`（历史resolved上下文漏项）；最终定向均通过。
- 未改schema/依赖/生产权限/ADR；根配置与锁文件、Electron、上游docs保持不变。

### 同日已完成范围（沿用3b4c2265验证）

- 第74次新增7项多段修稿回归，修复各段artifact与物理attempt数据库run/root归属漏检。
- 覆盖length→stop双段保存/恢复、48字符门槛重叠去重、无增长拒绝、首段14项篡改矩阵、合成来源乱序/重复/遗漏及跨run/root关系。
- 当时审稿域定向51通过（`continuation-directed.log`）；不是本轮新增测试，也不是全量。

## 交接与下一步

1. 核对最新HEAD、工作区及 `.pi/tmp/pipeline-state.md`，保留全量基线与累计数。
2. 继续审计完整finding集合来源重建、未参与复核项的持久状态、效果唯一性及剩余跨root/run组合；当前两处修复不等于完整proveEffect等价。
3. 历史可选上下文字段与版本组合仍需扩展；新执行不得借历史策略降级，冻结恢复不可重放，不伪造originalReceiptVerified。
4. 真实模型、GUI导入恢复全链验收尚未完成；频道开放必须另行完整验收，不能由测试通过推断。
