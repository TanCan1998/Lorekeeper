# Tauri手动打包与预发布

## 边界

用户于2026-10-18明确授权GitHub打包并发布，指定首版 **1.0.0-beta**、标签 **lorekeeper-v1.0.0-beta**、预发布且不设最新版。只修改Tauri版本，不改Electron根版本。

工作流 `.github/workflows/tauri-release-package.yml` 只接受 `workflow_dispatch`。默认 `publish=false` 仍仅打包；明确 `publish=true` 才允许独立发布作业写Release/标签。三平台为Windows x64（NSIS/MSI）、macOS ARM64/Intel（DMG），不提供Linux安装包。

## 流程

1. prepare锁定完整源码SHA，核对package/Cargo/config/Cargo.lock项目版本、新标签命名与发布说明；已存在同名标签或Release立即失败。
2. 三个平台以同一SHA构建，冻结依赖，前端体检并生成真实安装包；打包流不执行测试。
3. 发布作业仅拥有 `contents:write`、`actions:read`，其他作业保持只读。下载当前run/attempt且同一源码SHA的三份产物。
4. 要求同一源码SHA已有独立Tauri Rust CI全量成功记录；不以旧SHA测试或打包成功替代。
5. 对精确四包及三清单校验版本、平台、run/attempt、大小与SHA256，拒绝额外文件、坏摘要及错世代产物。
6. 原子创建新标签，创建草稿、上传9个资产（四安装包、三平台原清单、SHA256SUMS、release-manifest），核对远端大小/摘要/集合后公开。
7. 再读回Release与标签SHA。发生错误不覆盖、不自动删除标签或草稿，须核查后处理；不能盲目重跑发布作业覆盖现有版本。

所有Actions固定commit SHA；本次download-artifact v8.0.2为 `9000827ccba6bdab643e8b6fd33ac0654aef8333`。

## 离线自检

在仓库根执行：

```powershell
node --test tauri-app/scripts/release-helpers.node-test.mjs
node --check tauri-app/scripts/publish-release.mjs
```

再做Tauri的typecheck、相关测试、lint与build/diff检查。Node内置测试使用独立命名，避免被Vitest当作自己的测试套件扫描。版本/配置变化按项目混合验证政策触发对应SHA的Rust云端验证。

原始打包清单 `published:false` 是打包时状态，不在发布后重写它；Release实时状态以GitHub API为准。清单摘要不是代码签名或安装验收。

## 安装与版本限制

Windows无证书签名；macOS仅ad-hoc、未开发者签名/公证。MSI只支持数字版本，配置WiX版本为1.0.0.0，应用版本仍为1.0.0-beta。后续beta/正式版须另外规划MSI数字版本递增；不承诺从内部1.1.0版本降级的安装路径。

发布不授权前台窗口、真实模型/MCP/业务云或安装实测，也不把历史495f5632安装包当作本次产物。具体限制必须写入版本发布说明。
