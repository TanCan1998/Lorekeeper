# 当前交接入口

- 最新：[第104次：上游v1.3.0同步](./2026-10-18-upstream-v130-sync.md)。上一轮：[第103次：Tauri打包证据](./2026-10-18-tauri-migration-status.md)。
- 合入 `upstream/master` **3a5ba452**，99提交、503文件，无文本冲突；Electron相关删除经明确确认后随上游采纳。
- 根Electron版本升至 **1.3.0**；Tauri三处版本保持 **1.1.0**，不是新版界面迁移。
- 根 `src/`、`electron/`、`docs/` 与本次上游一致；根锁/config未改，fork提交消息脚本增强保留。
- 上游移除三个skin频道并改纸墨外壳/light-dark/v3外观；**Tauri保留四主题、图片皮肤及三个skin频道**，不自动迁移外观schema。契约差异明确保留。
- 唯一Tauri改动：Storybook跨栈文档测试按上游light/dark断言；原Tauri四主题/皮肤测试继续保留。
- 本机：根typecheck/lint通过、定向4文件70项；Tauri typecheck/lint/build通过、190文件2121通过/1跳过、频道290+7/未迁移0、打包内容体检通过。
- Rust未改，本次未重跑Rust全量；最近核验 **e21d95d7 / 39108477718，1692/0**。三平台安装包仍为 **495f5632 / 39110738571** 的证据，不归为本次合并版本。
- 保持不发布、不派子代理、不抢焦点；真实模型/MCP/业务云未授权；新版本GUI/安装包未认证。
