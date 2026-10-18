# 第106次：Lorekeeper 1.0.0-beta发布准备与历史引用清理

## 用户授权

- 明确要求GitHub打包并发布，同意独立发布作业核验后创建新标签/Release。
- 指定1.0.0-beta，标签按推荐为lorekeeper-v1.0.0-beta，类型prerelease，不设最新版。
- 明确“同意完整清理”：完整备份后，本地/origin仅保留master及今后Lorekeeper发布标签；不触碰upstream仓库。

## 引用清理已完成

- origin删除51个历史分支（含20个HEAD外历史）、26旧标签；本地删除1个合并临时分支、31个上游旧标签。
- 备份 `D:/Temp/Lorekeeper-refs-before-cleanup-1792321676229.bundle`，200,369,098字节，SHA256 `d14cf2a7eb94b5e03b0815bae6904786b3c11da89a31466a427a0ec94383c3cd`。
- bundle校验、独立离线镜像恢复、git fsck通过，82个唯一引用对象可读取。恢复镜像为同路径加 `.verify.git`。引用清单及SHA在 `.pi/tmp/recovery/ref-cleanup/`，不入库。
- 原子SHA条件删除，master未移动，发布工作区逐字未变；本地及origin实际仅master，origin/HEAD符号引用仍正常。
- 未来同步upstream请显式使用 `--no-tags`，否则可能把旧上游标签重新带回。未更改仓库Git配置或upstream远端。

## 发布准备

- Tauri package/Cargo/config及Cargo.lock项目版本同步1.0.0-beta；WiX数字版本1.0.0.0。不改Electron、项目数据库、数据路径/权限、业务执行安全规则。
- 默认仅打包，手动明确publish才发布；最小作业权限、同一源码SHA锁定、同run/attempt三平台精确四包核验、同SHA Rust成功记录、草稿上传后远端SHA256核验再公开。详见[发布协议](../agents/tauri-release.md)。
- 新增18项Node内置门禁测试通过；工作流YAML/权限/固定SHA/三平台合同与脚本语法通过。Tauri typecheck/lint/build成功，222文件2748通过/1跳过，产物39.2MiB/181文件/8WOFF2体检通过；Cargo locked metadata核对版本成功。
- 初次前端测试误将Node内置测试文件识别为Vitest套件，已采用独立node-test命名，不改变断言或产品实现；重跑成功。
- 无依赖升级；Cargo.lock仅本项目版本变更。Context7额度耗尽，WiX数字版本约束查阅本机已安装tauri-utils配置源码。
- 当前准备提交以便云端打包；尚未创建Release或发布标签。提交后对应SHA的Rust CI、三平台包、发布URL/资产核验结果待登记，不沿用旧包或旧测试认证。
