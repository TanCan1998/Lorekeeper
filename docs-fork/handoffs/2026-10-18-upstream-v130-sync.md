# 第104次交接：上游 v1.3.0 同步（2026-10-18）

## 代码与范围

- 同步前 fork：`74694b049b5e4259a3c8cf9021f7dc237e73bc0c`。
- 合入上游：`3a5ba452`（`upstream/master`，含 v1.3.0 及平台快捷键测试修复），99个新提交。
- 上游差异503文件，7438行新增、41666行删除；主要为纸墨外壳收口、旧外壳/图片皮肤清理、界面及旅程回归、发布资料。
- `git merge-tree --write-tree` 预演无冲突，在临时分支 `chore/merge-upstream-20261018-v130` 正式合并。
- 用户在明确说明 Electron 皮肤 controller/service 与测试删除影响后选择“同意”；删除仅跟随上游，不额外清理 Electron 基线。

## 双栈边界与契约差异

- 根 `package.json` 仅随上游变为1.3.0并移除废弃脚本；根 `pnpm-lock.yaml`、`vite.config.ts`、`.npmrc` 不变。
- `docs/`、`electron/`、根 `src/` 与本次上游树一致，不修改上游镜像。
- `scripts/check-commit-msg.mjs` 与其测试继续保留 fork 既有增强；同步没有改动它们。
- 上游把皮肤 controller/service、`SkinChannels` 及 `skin:get-state` / `skin:execute` / `skin:read-custom-asset` 移除，外壳收口为纸墨，颜色主题为light/dark，外观存储改为v3。
- **Tauri 不随合并自动迁移外壳/外观存储**：仍保留四主题与图片皮肤、原外观存储及三个皮肤频道；版本三处仍1.1.0。本次合并不是新版界面迁移，也不是开放读取上游外观数据目录。
- Tauri频道重算仍290 invoke、7事件，291注册命令，289功能迁移+1明确不支持、未迁移0。上游根契约相对Tauri已有明确差异，不能再称两边逐字相同。
- Rust代码、数据库schema、依赖声明、文件权限与定稿生命周期未变；不重跑Rust全量，最近已核验全量仍1692/0（e21d95d7 / 39108477718）。三平台打包证据仍495f5632 / 39110738571，不冒称为本次合并版本的安装包。

## 唯一 Tauri 兼容修正

`tauri-app/src/tokens/__tests__/tokens.test.ts` 原先读取上游 `src/stories/Introduction.mdx` 并断言四主题。上游文档已仅列light/dark，导致合并后的Tauri全量出现1例确定性失败。

调整跨栈Storybook文档断言为上游当前light/dark，并明确它只约束共享语义变量，不约束Tauri保留的四主题/图片皮肤；原有Tauri四主题色板、角色色与皮肤测试继续完整执行。未复制上游Storybook，也未修改任何产品运行时代码。

## 本机验证

| 项目 | 结果 |
|---|---|
| 根 typecheck / lint | 全部退出0 |
| 根定向测试 | 4文件70通过；覆盖外观配置、token、启动controller与fork提交消息检查 |
| Tauri typecheck / lint | 全部退出0 |
| Tauri全量 | 190文件2121通过/1跳过，最终25.16秒 |
| Tauri build | 退出0，沿用既有chunk体积/插件耗时警告 |
| 频道扫描 | 290 invoke / 7事件 / 未迁移0，退出0 |
| 打包内容体检 | dist39.2MB，6个WOFF2，无TTF、无受检上游/Electron标识 |
| diff检查 | 无冲突标记、无新增尾空白；根锁文件未修改 |
| 上游作者豁免 | 新提交仅已有豁免邮箱，无须修改CI配置 |

首次根定向测试的CLI文件名过滤同时选中了Tauri同名测试，并在根工作目录执行，出现错误夹具路径与色板失败；明确排除 `tauri-app/**` 后根测试70项通过。此为测试选择失误，不是新增产品缺陷。前述Tauri文档断言失败独立复现并修复。

未运行Electron全量/浏览器旅程或真机GUI；没有真实模型、MCP、业务云调用，没有发布。不会把根定向回归与Tauri单元测试冒称全平台或上游全部503文件逐项验收。

## 接续

- 主会话直接同步，不派子代理；保持默认后台、不抢焦点。
- 待需要时单独规划上游新外壳与Tauri的差异迁移；不在本次上游原样同步中夹带外观schema/准入或产品范围变更。
- 本机日志：`.pi/tmp/upstream-v130-*.log`；不提交合成数据、依赖、构建产物。
