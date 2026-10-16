# Tauri 迁移进度快照（2026-10-17）

> 当前入口 [CURRENT.md](./CURRENT.md)；规则见 [开发指南 §9](../agents/pi-development.md)。上一轮见 [第73次快照](./2026-10-16-tauri-migration-status.md)。

## 已验证状态（第 74 次 —— 多段修稿来源归属与续写回归）

- master，提交前 HEAD/本地 origin/master 均为 `afe6321d`，接续时工作区干净。
- 主会话直接推进，不派发子代理；本轮为普通内部校验切片，按既有授权自检后提交推送，最终 SHA 见本机 pipeline。
- **完整 proveEffect 审计仍未收口，审稿四频道保持关闭。**

| 指标 | 结果 | 验证版本/范围 | verifiedAt | 证据路径（本机，不入库） |
|---|---|---|---|---|
| 审稿域定向 | **51通过，0失败**，含本轮新增7项续写测试 | afe6321d + 本轮三个Rust文件 | 2026-10-17 | .pi/tmp/recovery/continuation-directed.log |
| Rust格式及差异 | 三文件rustfmt --check、git diff --check通过 | 同一最终代码 | 2026-10-17 | 工具输出 |
| 最近Rust全量 | **1581通过，0失败**，库62.27秒；非本轮重跑 | afe6321d代码基线 | 2026-10-16 | .pi/tmp/recovery/chain-full.log |
| 最近all-targets | 无告警；非本轮重跑 | afe6321d代码基线 | 2026-10-16 | .pi/tmp/recovery/chain-check.log |
| invoke/事件/前端/GUI | 本轮未重测、未开放频道 | 沿用前次口径，不是本轮验收 | 历史 | 上次快照 |

本轮为内部私有证明查询与测试改动，相关目标已由定向测试编译；未改公共签名、准入规则、schema或构建目标，按分层验证策略不机械重跑all-targets/全量。此切片提交后，距最近全量累计1个Rust语义切片。

## 本次更新

- 任务范围：多段修稿续写与持久来源归属，新增 `review_revision_continuation_tests.rs`；复用真实初审/确认/修稿/合并SQLite链。
- 提取测试用生成/追加产物辅助；双段链首段合法以length结束、末段以stop结束，保持可见产物原字节与业务修稿正文分离。
- 覆盖双段正文保存和只读恢复（不新建attempt）、达到既有48字符门槛的重叠去重、重复正文/纯标点无增长拒绝且无业务写入。
- **复现并修复归属漏洞**：此前只验证首段artifact的run_id及attempt JSON身份；物理attempt的数据库run_id被移至同根另一run时仍可通过。现交叉验证各段artifact、attempt的数据库run_id与root_action_id，不能借JSON自述掩盖数据库归属漂移。
- 新增14项首段JSON篡改矩阵（归入一个测试）：artifact身份/epoch/fingerprint/revision/hash、attempt身份/终态、receipt产物身份/epoch/结束原因/失败码。每次恢复原值重新通过。
- 验证合成来源乱序、重复、遗漏首段均拒绝；新增物理attempt根列被移至另一真实root的拒绝回归。
- 初始重叠夹具不足48字符导致断言失败，已按既有去重合同修正夹具，未改去重算法；复现日志 `continuation-before.log` 中另一个失败为真实attempt归属漏洞。
- 没有改schema/依赖/生产权限/ADR/频道开放；根配置与锁文件、Electron、上游docs不变。

### 仍待完成

1. 必现目标与作者豁免混合复核、历史报告/策略版本兼容性。
2. 全链跨root/run组合及唯一效果证明继续审计；本轮只覆盖多段来源关系，不宣称完整proveEffect等价。
3. 真实模型与GUI恢复全链验收仍未完成，不能以定向测试通过开放审稿频道。

## 交接给下次会话

- 先核对 HEAD、工作区及 `.pi/tmp/pipeline-state.md`，最新运行状态/代码指纹以该文件为准。
- 下一步优先构建必现/作者豁免混合fixture并审计历史策略；保持作者事实、冻结历史不可重放及originalReceiptVerified=false边界。
- 本轮定向不是全量；最新全量仍为afe6321d基线。累计满5个Rust代码切片或阶段收口时按规则触发全量。
