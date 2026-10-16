# docs-fork —— 本 fork 专属文档

> **这是 TanCan1998/Lorekeeper 全部 fork 专属文档的入口。**
> 上游 `docs/` 保持与 `upstream/master` 逐字节一致，不含任何 fork 内容；
> 因此 fork 的规则、快照与调研**只在这里**。

## 迁移总进度（收口时更新）

迁移进度的 README 展示仅在本文件维护；根 `README.md` 只保留 Fork 目的和文档入口，不再记录覆盖率、测试结果、待办或接续轮次。

```text
Rust频道结构覆盖  🟩🟩🟩🟩🟩🟩🟩🟩🟩🟩🟩🟩🟩🟩🟨  289/290 ≈99.7%
生产前端频道开放  🟩🟩🟩🟩🟩🟩🟩🟩🟩🟩🟩🟩🟩🟩🟨  285/290 ≈98.3%
```

> 2026-10-17第80次：用户确认[混合验证](agents/tauri-rust-ci.md)，新增独立Windows Tauri全量/all-targets工作流，Action实时核对最新稳定版并固定SHA。本轮离线脚本合同8通过，首次云端结果以[运行记录](https://github.com/TanCan1998/Lorekeeper/actions/workflows/tauri-rust-ci.yml)为准；最近本机全量仍为0ead5794代码1611通过。四审稿频道仍关闭，真实业务外部验证未授权。
> **注意**：上述比例不是产品总完成率，也不因首版排除旧库导入而改分母。方案已确定不等于实现/验收完成；剩余证明审计、Windows合成GUI与旧库入口不可用边界待落实。GitHub合成CI已授权，真实模型/MCP/业务云验收仍未授权，不得混为已验证。
> 口径：当前已合并上游（@ `21d67211`，2026-10-10）为全集，**上游新提交不计入**；权威实测值见 [当前交接入口](handoffs/CURRENT.md) 指标表，频道明细见 [频道盘点](plans/tauri-migration-channel-inventory.md)。


## 接续工作从这里开始

| 我要… | 去哪里 |
|---|---|
| 接续迁移任务 | [当前交接](handoffs/CURRENT.md) → [第80次快照（2026-10-17）](handoffs/2026-10-17-tauri-migration-status.md) |
| 新建一份快照 | 复制 [`handoffs/_TEMPLATE-tauri-migration-status.md`](handoffs/_TEMPLATE-tauri-migration-status.md)，规则见 [`agents/pi-development.md`](agents/pi-development.md) §9 |
| 查频道盘点 | [`plans/tauri-migration-channel-inventory.md`](plans/tauri-migration-channel-inventory.md) |
| 查迁移开发规范 | [`agents/pi-development.md`](agents/pi-development.md) |
| 查看本机/云端测试分工 | [Tauri Rust混合验证](agents/tauri-rust-ci.md) |
| 查 Tauri 迁移调研 | [`research/`](research/) |
| 看 fork 改过上游什么 | [`notes-on-upstream-changes.md`](notes-on-upstream-changes.md) |
| 查 ADR 索引 | [`adr/README.md`](adr/README.md) |

## 目录结构

| 目录 | 内容 |
|---|---|
| [`agents/`](agents/) | fork 的开发规范（`pi-development.md`） |
| [`adr/`](adr/) | fork 维护的 ADR 索引（上游 ADR 正文仍在 `docs/adr/`） |
| [`handoffs/`](handoffs/) | 迁移进度快照与快照模板 |
| [`plans/`](plans/) | 迁移计划与频道盘点 |
| [`research/`](research/) | Tauri 迁移调研（含 `archive/` 历史归档） |

## 边界（硬规则）

- `docs/` = **上游镜像**，与 `upstream/master` 逐字节一致，**永不修改、永不新增**；
- `docs-fork/` = **fork 专属**，**新增文档一律放这里**；
- 想改上游文档 → 写入 [`notes-on-upstream-changes.md`](notes-on-upstream-changes.md)，**不动 `docs/`**；
- 合并上游 → `docs/` 直接接受上游版本，`docs-fork/` 不受影响。

校验命令（合并上游或迁移后必跑）：

```bash
git diff upstream/master -- docs/     # 必须无输出
```

## 关于根目录 AGENTS.md

根 `AGENTS.md` **不属于本目录、也不入 git**：上游 `scripts/__tests__/public-repository-hygiene.test.ts`
将其列为禁用路径（`internal process material`）、上游 `.gitignore:62` 忽略它、上游 `docs/README.md`
说明它是「可选的本机 Agent 上下文，按仓库卫生规则保持忽略，不是公共文档」。

它对**本机 Agent 会话**是有效的接续入口（会被自动注入），但对 clone 后的其他人 / CI
**不可见**。因此本 README 才是 fork 的**公共入库入口**，`AGENTS.md` 是**本机会话入口** ——
两者互补，不可互相替代。

## 迁移历史

2026-10-09：fork 专属文档从 `docs/` 整体迁至 `docs-fork/`，使 `docs/` 与上游逐字节一致，
以消除长期合并冲突。迁移前 fork 对上游 `docs/` 的改动登记于
[`notes-on-upstream-changes.md`](notes-on-upstream-changes.md)。
