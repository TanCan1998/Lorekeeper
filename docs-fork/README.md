# docs-fork —— 本 fork 专属文档

> **这是 TanCan1998/Lorekeeper 全部 fork 专属文档的入口。**
> 上游 `docs/` 保持与 `upstream/master` 逐字节一致，不含任何 fork 内容；
> 因此 fork 的规则、快照与调研**只在这里**。

## 仓库与下载

- [项目首页](../README.md) · [镜像与同步说明](agents/atomgit-mirror.md)。GitHub 与 AtomGit 共用同一份 README，本项目链接使用相对路径，上游链接保持原地址。
- [安装包与 SHA256 校验：AtomGit（国内推荐）/ GitHub 双源](releases/1.0.0-beta.md)。当前为已公开预览版，不重新压缩或打包。
- [自动代码同步、双平台 Release 与恢复说明](agents/atomgit-mirror.md)。两端共用 README，仓库内文档和图片使用相对路径。

## 迁移进度入口

迁移进度的 README 展示仅在本文件维护；根 `README.md` 只保留 Fork 目的和文档入口，不再记录覆盖率、测试结果、待办或接续轮次。

2026-10-19第125次：[剩余迁移合并实现，已提交且云端验证通过](handoffs/2026-10-19-tauri-migration-status.md#本次更新第125次剩余迁移合并实现已提交云端验证通过)。用户要求不切片、最后统一验证；本轮完成大纲/蓝图作者恢复全链路、目录进度登记、合成前五个可变性守卫、头像双语与缓存、MSI 快捷方式与安装后启动改走激活器，并纠正陈旧注释。仅新增既有 `windows-sys` feature。用户随后授权完整验证：`cargo check --all-targets` 0 错误、`cargo test --lib` **1734 通过 0 失败**、定向 344 通过、rustfmt 与 `pnpm typecheck`/`lint` 通过、安装合同 13 通过、频道缺失 0；前端 vitest 2758 通过、1 skipped、0 失败（既存品牌大小写断言已改为引用 `APP_BRAND.enName`）。随后按主题分成 7 个提交推送 `3e8101ba..011b4b0f`；云端同一 SHA 上 Tauri Rust CI、Commit message CI、Gitleaks 与 AtomGit mirror 全部 success。并在干净机器上完成 MSI 真实安装验收（静默安装、修复安装、身份重注册幂等、卸载清理全部退出码 0；快捷方式与安装后启动均走 `lorekeeper-activate.exe`）。**未发布**；从快捷方式启动的前台 GUI、旧版本升级路径、多用户 UAC 仍未验收。升级/回滚与多用户验收未完成；新版发布继续暂停。精确状态以[当前交接](handoffs/CURRENT.md)为准。

当前已公开的仍是第106次 **[Lorekeeper 1.0.0-beta 预发布与双源下载](releases/1.0.0-beta.md)**，提供Windows x64与macOS ARM64/Intel安装包及校验清单，旧版不设为最新版、不覆盖。

Tauri已跟随RD-10采用纸墨流程外壳、浅深主题及外观v3，保留独立数据目录与作者/定稿安全边界；频道285项功能迁移、1项旧库导入明确不支持，未迁移0。比例不是产品完成率，旧轮次的290频道口径不代表当前契约。

> #30/#31内部范围验收已收口；真实模型/MCP/业务云、签名公证、所有正常安装入口及升级/回滚仍未认证。历史手动激活器身份验证不等于产品全生命周期通过。用户明确拒绝归组未修复先公开，资源操作恢复前不编译、运行或推送触发CI；新版继续暂停发布，后台窗口规则与系统变更确认边界不变。


## 接续工作从这里开始

| 我要… | 去哪里 |
|---|---|
| 接续迁移任务 | [当前交接](handoffs/CURRENT.md) → [第125次状态与待验清单](handoffs/2026-10-19-tauri-migration-status.md) |
| 新建一份快照 | 复制 [`handoffs/_TEMPLATE-tauri-migration-status.md`](handoffs/_TEMPLATE-tauri-migration-status.md)，规则见 [`agents/pi-development.md`](agents/pi-development.md) §9 |
| 查频道盘点 | [`plans/tauri-migration-channel-inventory.md`](plans/tauri-migration-channel-inventory.md) |
| 查迁移开发规范 | [`agents/pi-development.md`](agents/pi-development.md) |
| 查看本机/云端测试分工 | [Tauri Rust混合验证](agents/tauri-rust-ci.md) |
| 手动打包与发布 | [Tauri受限发布协议](agents/tauri-release.md) |
| 管理Actions存储 | [1天保留与历史制品清理](agents/actions-storage.md) |
| 查看Windows身份与自选目录接入 | [已选无签名方案及待实现生命周期](plans/windows-sparse-identity-integration.md) |
| 查看WebView进程归组研究 | [机制证据与后台验证矩阵](research/webview-process-attribution.md) |
| 查看定稿图标与生成说明 | [图标登记](design/branding.md) |
| 查看预发布说明 | [旧1.0.0-beta](releases/1.0.0-beta.md) · [未发布beta.1](releases/1.0.0-beta.1.md) |
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
