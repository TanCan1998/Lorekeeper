# 第106次：Lorekeeper 1.0.0-beta已发布与历史引用清理

## 用户授权

- 明确要求GitHub打包并发布，同意独立发布作业核验后创建新标签/Release。
- 指定1.0.0-beta，标签按推荐为lorekeeper-v1.0.0-beta，类型prerelease，不设最新版。
- 明确“同意完整清理”：完整备份后，本地/origin仅保留master及今后Lorekeeper发布标签；不触碰upstream仓库。

## 引用清理已完成

- origin删除51个历史分支（含20个HEAD外历史）、26旧标签；本地删除1个合并临时分支、31个上游旧标签。
- 备份 `D:/Temp/Lorekeeper-refs-before-cleanup-1792321676229.bundle`，200,369,098字节，SHA256 `d14cf2a7eb94b5e03b0815bae6904786b3c11da89a31466a427a0ec94383c3cd`。
- bundle校验、独立离线镜像恢复、git fsck通过，82个唯一引用对象可读取。恢复镜像为同路径加 `.verify.git`。引用清单及SHA在 `.pi/tmp/recovery/ref-cleanup/`，不入库。
- 原子SHA条件删除，清理时master未移动，发布工作区逐字未变；清理完成时本地及origin仅master。发布后新增唯一标签lorekeeper-v1.0.0-beta，origin/HEAD符号引用仍正常。
- 未来同步upstream请显式使用 `--no-tags`，否则可能把旧上游标签重新带回。未更改仓库Git配置或upstream远端。

## 发布准备

- Tauri package/Cargo/config及Cargo.lock项目版本同步1.0.0-beta；WiX数字版本1.0.0.0。不改Electron、项目数据库、数据路径/权限、业务执行安全规则。
- 默认仅打包，手动明确publish才发布；最小作业权限、同一源码SHA锁定、同run/attempt三平台精确四包核验、同SHA Rust成功记录、草稿上传后远端SHA256核验再公开。详见[发布协议](../agents/tauri-release.md)。
- 新增18项Node内置门禁测试通过；工作流YAML/权限/固定SHA/三平台合同与脚本语法通过。Tauri typecheck/lint/build成功，222文件2748通过/1跳过，产物39.2MiB/181文件/8WOFF2体检通过；Cargo locked metadata核对版本成功。
- 初次前端测试误将Node内置测试文件识别为Vitest套件，已采用独立node-test命名，不改变断言或产品实现；重跑成功。
- 无依赖升级；Cargo.lock仅本项目版本变更。Context7额度耗尽，WiX数字版本约束查阅本机已安装tauri-utils配置源码。
- 代码提交 **3bce04455054de1860292d970f96425a1fdbe2b7** 已推送。Commit CI **39122557568**、Gitleaks **39122557573** 成功。

## 云端全量与真实发布

- 对应源码Rust CI **39122557567** 独立 **1680通过/0失败**（库278.56秒），all-targets成功（105秒）。SHA/expectedSHA、run、锁、两退出0与state=passed均核验。
- Rust证据artifact **38,901字节**，SHA256 `92cf8d924181929619b59325191a7d03682c04d7b241f1e6607980ad000d6bdd`；Cargo.lock SHA256 `37abf17d07d6cd96b3bc4187827333a9109198e021a5aa8b4a242d422a75907e`。本机 `.pi/tmp/recovery/ci-3bce0445/`，累计Rust切片清零。
- 手动打包/发布run **39122558975**，同一源码SHA；准备、Windows x64、macOS ARM64、macOS Intel、发布作业全部成功。运行墙钟2026-10-18 11:16:19–11:34:47 UTC，18分28秒。
- 正式公开的**预发布**：[Lorekeeper 1.0.0-beta](https://github.com/TanCan1998/Lorekeeper/releases/tag/lorekeeper-v1.0.0-beta)，Release ID **416413071**，publishedAt **2026-10-18T11:34:43Z**；draft=false/prerelease=true，非最新版（latest端点无正式版）。
- 唯一发布标签 `lorekeeper-v1.0.0-beta` 精确指向3bce0445；已单独fetch到本地，没有重新引入上游旧标签。

| 安装包 | 字节 | SHA256 |
| --- | ---: | --- |
| Lorekeeper_1.0.0-beta_x64-setup.exe | 50,086,322 | `fe59b803b06b61edf2e33e7be2c32c4c35c0c90b52e0dd7148f4b143f8f00404` |
| Lorekeeper_1.0.0-beta_x64_en-US.msi | 54,132,736 | `4f18741bfbc3fe5a4b7cf7c083a46a32900759ea2f5f639ea2b8f78e6e57e11d` |
| Lorekeeper_1.0.0-beta_aarch64.dmg | 55,579,782 | `29c743e440fcbb6801b9caa370c4fe07b120deab2d4d42e480b0d337f63d8e2a` |
| Lorekeeper_1.0.0-beta_x64.dmg | 55,884,576 | `96096f5f4b51e8082cbef5655391eb6f0eb4990a79e6210833322bb4f23579ef` |

发布资产共 **9个**：四包、三平台原始清单、SHA256SUMS、release-manifest。CI已对真实字节核验并对上传资产读回大小/摘要；本机会话又下载五个小型清单并核对其实际大小/摘要、四包跨清单、源码/run/Rust验证绑定和Release API九资产摘要。没有把“API摘要相符”说成本机会话下载了四个大包。

本机发布证据 `.pi/tmp/recovery/beta-release/`，下载的小清单 `D:/Temp/Lorekeeper-release-1.0.0-beta/`，不入库。原始清单的published=false保留打包时事实，当前发布状态取自Release API。

## 保留限制

Windows无证书；macOS仅ad-hoc，未开发者签名/公证；本次新包未真机安装/首次启动验收。没有调用真实模型/MCP/业务云，没有前台窗口或全局键鼠操作。此次发布不放宽数据隔离、定稿身份或外部文件权限。
