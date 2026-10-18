# docs-fork —— 本 fork 专属文档

> **这是 TanCan1998/Lorekeeper 全部 fork 专属文档的入口。**
> 上游 `docs/` 保持与 `upstream/master` 逐字节一致，不含任何 fork 内容；
> 因此 fork 的规则、快照与调研**只在这里**。

## 迁移进度入口

迁移进度的 README 展示仅在本文件维护；根 `README.md` 只保留 Fork 目的和文档入口，不再记录覆盖率、测试结果、待办或接续轮次。

2026-10-18第106次：**[Lorekeeper 1.0.0-beta已公开预发布](https://github.com/TanCan1998/Lorekeeper/releases/tag/lorekeeper-v1.0.0-beta)**，提供Windows x64与macOS ARM64/Intel安装包及校验清单，不设为最新版。

Tauri已跟随RD-10采用纸墨流程外壳、浅深主题及外观v3，保留独立数据目录与作者/定稿安全边界；频道285项功能迁移、1项旧库导入明确不支持，未迁移0。比例不是产品完成率，旧轮次的290频道口径不代表当前契约。

> #30/#31内部范围验收已收口；真实模型/MCP/业务云、签名公证、新包真机安装/首次启动仍未认证。当前授权仅手动打包与预发布，不放宽后台窗口规则。准确源码SHA、CI与发布证据见[当前交接](handoffs/CURRENT.md)，频道明细见[盘点](plans/tauri-migration-channel-inventory.md)。


## 接续工作从这里开始

| 我要… | 去哪里 |
|---|---|
| 接续迁移任务 | [当前交接](handoffs/CURRENT.md) → [第106次发布快照](handoffs/2026-10-18-tauri-beta-release.md) |
| 新建一份快照 | 复制 [`handoffs/_TEMPLATE-tauri-migration-status.md`](handoffs/_TEMPLATE-tauri-migration-status.md)，规则见 [`agents/pi-development.md`](agents/pi-development.md) §9 |
| 查频道盘点 | [`plans/tauri-migration-channel-inventory.md`](plans/tauri-migration-channel-inventory.md) |
| 查迁移开发规范 | [`agents/pi-development.md`](agents/pi-development.md) |
| 查看本机/云端测试分工 | [Tauri Rust混合验证](agents/tauri-rust-ci.md) |
| 手动打包与发布 | [Tauri受限发布协议](agents/tauri-release.md) |
| 查看预发布说明 | [1.0.0-beta](releases/1.0.0-beta.md) |
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
| [`releases/`](releases/) | Lorekeeper版本发布说明及验收限制 |

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
