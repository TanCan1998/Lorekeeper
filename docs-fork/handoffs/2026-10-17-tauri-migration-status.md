# Tauri迁移交接快照（2026-10-17）

> [当前入口](./CURRENT.md) · [混合验证](../agents/tauri-rust-ci.md) · [首版决策](../plans/2026-10-17-tauri-completion-decisions.md) · [证明/GUI审计矩阵](../plans/2026-10-17-tauri-proof-audit.md)。动态进度仅在docs-fork展示。

## 已验证状态（第82次）

代码版本：`729cda50233747a29e2c46c7130498bdffa3bbdd`，master，已推送origin。此后的本轮收口仅文档，无代码语义变化。验证日期为本机2026-10-17（云端UTC日志为2026-10-16）。

| 范围 | 结果/版本 | 证据 |
|---|---|---|
| Rust云端全量 | **1614通过/0失败**，库197.15秒，同上SHA | [run38946375676](https://github.com/TanCan1998/Lorekeeper/actions/runs/38946375676)；本机`.pi/tmp/recovery/ci-729cda50/` |
| 云端all-targets | 成功、无告警，testExitCode/checkExitCode均0，state=passed | 同run artifact的validation.json与cargo-check.log |
| 云端增量规范/秘密扫描 | 同SHA的Commit message CI、Gitleaks均成功 | run38946375629、38946375642 |
| 本机Rust定向 | 沿用第81次同代码：项目域22/0、SSE域13/0 | legacy-directed-final.log、legacy-sse.log |
| 前端 | 沿用第81次同代码：187文件、2108通过/1跳过；typecheck/lint通过 | legacy-frontend.log、legacy-typecheck.log、legacy-lint.log |
| 本轮格式/静态复核 | 四Rust文件rustfmt、diff、隔离、频道检查、YAML及新增路径、离线CI合同8项通过 | 工具输出及CI合同 |
| 本机开发构建 | 前端build通过（大chunk提示）；Rust build --locked通过，11.57秒、无告警 | current-gui-frontend-build.log、current-gui-rust-build.log |
| 本机真实GUI | 旧库disabled说明、新建/配置保存、本地归档导出/新副本恢复、打开副本/配置一致、正常关闭重启及最近列表重开通过；仅配置级合成项目 | current-gui-ui.json、current-gui-archive-result.json与截图 |
| 频道口径 | 注册291、契约注册290、事件7；功能迁移289/290、业务开放285/290，不变 | 本轮check:channels；blocked不计功能覆盖 |

本机证据均在`.pi/tmp/recovery/`，不入库。新全量基线已从0ead5794更新为729cda50；**之后Rust语义切片累计0**。此前云端1444/167与1610/1是失败历史，不能覆盖本次成功证据。

## 本次实现与提交

- `15ed11f4`：首版旧库入口禁用并显示中英文说明；适配器/原生命令返回`{state:"blocked",code:"LEGACY_IMPORT_UNSUPPORTED"}`，不读旧库、不建副本、不消耗授权。保留IPC合同，范围清单用于生成器排除拒绝桩，290分母不变。已补Rust及前端回归。
- `729cda50`：Windows SSE测试连接显式恢复阻塞模式，保留5秒超时和全部断言；CI push/PR补范围清单触发路径。只改测试夹具，不改真实模型行为。
- 对跨root/run效果唯一性、物理attempt归属、历史版本/可选字段、畸形输入失败关闭做了有界源码与既有回归核查；云端实际执行审稿服务55项。见审计矩阵，不宣称全部证明审计完成。
- 真实Tauri开发窗口使用独立数据根与合成项目；原生授权完成导出/恢复，副本独立projectId、原项目未被替换，重启后配置一致。不是发布包或含章节/定稿/审稿材料的复杂归档验收。

## 最新用户要求：真实窗口仅后台验证

用户在本轮GUI后明确要求：后续验证不得置顶或打扰其操作。已写入本机AGENTS/pipeline：**不激活、不抢焦点、不用全局键鼠/SendKeys**；启动窗口也须保证后台。无法后台完成的原生交互记为待验收，不擅自切前台。

本轮测试应用已正常关闭。之前的临时GUI启动/选择脚本没有后台不抢焦点保证，**不得原样重用**；下一轮先准备后台安全方案。运行占用只在pipeline登记。

## 尚未解除的验收缺口

1. 完整审修复核GUI、暂停/取消/恢复、失效会话交错，以及含冻结历史/资产的复杂归档恢复未验收。当前配置级GUI不能替代这些场景。
2. 跨project/epoch/artifact的完整证明威胁枚举仍须继续逐项核查。现有回归通过不是无边界的安全结论。
3. **四生产审稿频道仍关闭**。不得为GUI临时绕过门禁；真实模型、外部MCP、业务云与发布未授权，macOS/Linux未认证。

## 接续顺序

1. 核对HEAD/工作区/pipeline；本轮文档提交后应无入库文件改动。没有待等待的本轮Rust CI，不能重记为进行中。
2. 按审计矩阵继续源码/合成回归，不涉及新依赖、schema、权限或ADR时由主会话直接推进。
3. 只有后台不抢焦点方案可用时再做GUI；否则保留对应未验收项，继续不依赖窗口的工作。不启动子代理。
