# 1.0.0-beta.3 发布清单（2026-10-25 起草）

> 状态：**未发布**。本清单只准备事实与步骤，任何一步执行前需用户明确授权（打包、发 Release、动标签都算）。
> 相关：[发布说明草稿](../releases/1.0.0-beta.3.md)、[发布与打包流程](../agents/tauri-release.md)、[安装约束](../agents/tauri-release.md#安装与版本限制)。

## 1. 版本位点（必须四处一致，发布门禁会逐一校验）

| 文件 | 当前 | 目标 |
|---|---|---|
| `tauri-app/package.json` → `version` | `1.0.0-beta.2` | `1.0.0-beta.3` |
| `tauri-app/src-tauri/tauri.conf.json` → `version` | `1.0.0-beta.2` | `1.0.0-beta.3` |
| `tauri-app/src-tauri/tauri.conf.json` → `bundle.windows.wix.version` | `1.0.4.0` | **`1.0.5.0`** |
| `tauri-app/src-tauri/Cargo.toml` → `version` | `1.0.0-beta.2` | `1.0.0-beta.3` |
| `tauri-app/src-tauri/Cargo.lock` → `name = "lorekeeper"` 的 version | `1.0.0-beta.2` | `1.0.0-beta.3` |
| `tauri-app/src-tauri/identity-launcher/Cargo.toml` → `version`（一致性） | `1.0.0-beta.2` | `1.0.0-beta.3` |

约束（`assertWixVersion` / `releaseIdentity` 强制）：
- MSI 版本必须是纯数字 `major.minor.patch[.build]`，主/次版本必须等于应用版本的 `1.0`，修订号 ≥ 应用修订号；`1.0.5.0` 满足且高于已装的 `1.0.4.0`，覆盖安装才会刷新文件。
- 标签必须等于 `lorekeeper-v${version}`，即 `lorekeeper-v1.0.0-beta.3`；禁止复用上游标签。
- `GITHUB_REPOSITORY` 必须是 `TanCan1998/Lorekeeper`。
- `productName` / `identifier` / `upgradeCode`（`ed3e3199-4ddc-501a-a745-179a185e8dfb`）**保持不动**，否则断掉升级关系。

## 2. 发布门禁（同一 SHA 上必须全绿）

- [ ] Tauri Rust CI 全量 + `--all-targets` success（混合验证：本机定向、云端全量）。
- [ ] Tauri contract tests（`test:contract` 56 项）success。
- [ ] Commit message CI、Gitleaks success。
- [ ] 本机 `typecheck` / `lint` / `check:channels`（未迁移 0）/ 单测 / 浏览器套件全绿。
- [ ] 工作区干净，`tmp-check/` 之类既存未跟踪项不受影响；不提交构建产物、日志或未脱敏报告。

## 3. 打包（`.github/workflows/tauri-release-package.yml`，仅 workflow_dispatch）

第一轮先只打包、不发布：
- `expected_sha`：填将要发布的 40 位 SHA（留空则用触发 ref 的 HEAD）。
- `publish` = **false**、`prerelease` = true、`make_latest` = false（只产出产物与清单）。

矩阵（三平台四包）：
| label | runner | bundles |
|---|---|---|
| Windows x64 | windows | `nsis,msi` |
| macOS ARM64 | macOS | `dmg` |
| macOS Intel | macOS | `dmg` |

产物核对（共 9 个附件）：四个安装包（MSI / NSIS / 两个 DMG）+ 三份平台清单 + `SHA256SUMS.txt` + `release-manifest.json`；核版本、平台、run/attempt、大小与 SHA256，拒绝多余文件与错世代产物。

第二轮确认无误后再 `publish` = **true**（beta 名称仍标注预览；若要设 Latest 需 `prerelease` = false 且 `make_latest` = true，与 beta.2 一致）。

## 4. 发布后本机验收（打包产物，不是源码构建）

- [ ] MSI 覆盖安装：从本机现有 `1.0.4.0` 升到新包，ARP 单条、版本正确、程序文件刷新、快捷方式指向 `lorekeeper-activate.exe` 且无参数。
- [ ] 首次启动：包身份按需注册成功；桌面图标为新帧；**观察不到 PowerShell/控制台窗口**。
- [ ] 关闭：仅发 `WM_CLOSE` 后进程自行退出；`~/.lorekeeper` 与 `~/.vela` 无意外写入。
- [ ] 卸载：ARP/Appx/目录/快捷方式清理干净，项目数据保留。
- [ ] NSIS 安装与卸载：当前用户安装、ARP 在 HKCU、用户级快捷方式、卸载后仅留启动器日志。
- [ ] 旧版升级：`beta.2`（1.0.2.0）→ 新包，覆盖后版本与文件一致。
- [ ] 若需回退：**不支持降级**，必须卸载后装旧包（Windows 不允许 Appx 降级注册）。

## 5. AtomGit 镜像

- [ ] 平台侧拉取新 tag 与源码（不推送代码）。
- [ ] `pnpm mirror:release` 上传九个附件（只补缺失，不覆盖）。
- [ ] 匿名流式读回大小与 SHA256，两端下载表格内容一致。

## 6. 收口

- [ ] 更新 `docs-fork/handoffs/CURRENT.md`“最近发布”段与当日快照；`README` 下载表随快照同步。
- [ ] 回填发布说明里的双源链接与最终份额。
- [ ] 记录未解除阻塞：多用户 UAC、macOS 真机、真实模型/MCP/业务云。
- [ ] 不删除、不移动、不覆盖任何旧标签、Release 或附件。

## 附：2026-10-25 预热记录（publish=false）

- 首次尝试 SHA `5e5ece3e`：Rust CI 失败——`identity-launcher/Cargo.lock` 仍写 `1.0.0-beta.2`，构建身份包时被 cargo 改写，触发 `test-rust-ci.ps1` 的「工作区不干净」门禁。已补提交 `f39249d0`。
- 正式预热：SHA **`f39249d0`**、run **40188547546**（attempt 1）、`publish=false`。
- 三平台作业全 success（Windows x64 / macOS ARM64 / macOS x64），发布作业按预期 skipped。
- Windows 产物与清单核对：
  - `LoreKeeper_1.0.0-beta.3_x64_zh-CN.msi` 53,989,376 B，SHA256 `14badadbfa1b34ff…`，**ProductVersion 1.0.5.0**，UpgradeCode 未变；
  - `LoreKeeper_1.0.0-beta.3_x64-setup.exe` 50,217,907 B，SHA256 `cf9df8259f8d9b34…`；
  - `manifest.json`：`version=1.0.0-beta.3`、`commit=f39249d0…`、`runId=40188547546`、`runAttempt=1`、`signed=false`、`published=false`；两份产物的字节数与 SHA256 与清单一致。
- macOS 两个 DMG 仅在产物归档内（各约 91 MB），本轮未下载；由发布作业的清单校验兜底。
- **待办**：正式发布（`publish=true`）需用户明确授权；届时 `prerelease=false` + `make_latest=true` 与 beta.2 一致。
