# 当前交接入口

- 最新：[第106次：Lorekeeper 1.0.0-beta已发布与历史引用清理](./2026-10-18-tauri-beta-release.md)。前序：[第105次RD-10迁移](./2026-10-18-tauri-rd10-stage.md)。
- 用户明确授权GitHub打包并发布、独立受限发布作业；指定 **1.0.0-beta / lorekeeper-v1.0.0-beta / 预发布非最新版**。
- [公开预发布](https://github.com/TanCan1998/Lorekeeper/releases/tag/lorekeeper-v1.0.0-beta)，ID416413071，源码3bce0445；打包发布run39122558975三平台及发布全部成功。四安装包+五校验清单共9资产，大小/摘要/标签SHA与跨清单已核验。
- 对应源码Rust CI39122557567独立1680/0及all-targets通过，artifact38901字节/摘要、SHA/锁/两退出0已核验，累计清零。CommitCI39122557568/Gitleaks39122557573成功。
- 本机18项发布门禁测试；前端222文件2748通过/1跳过，typecheck/lint/build及产物体检通过。本轮无前端产品语义改动，最近完整浏览器仍第105次76文件922通过，不冒充本轮复跑。
- 根Electron及根锁/配置未改，无新依赖；Tauri三版本及Cargo.lock项目版本变为1.0.0-beta，WiX数字版本1.0.0.0。
- 已获“同意完整清理”后，备份并清除origin51历史分支/26旧标签、本地1临时分支/31旧标签。独立离线恢复/fsck通过，备份详情见交接。现在仅master及唯一Lorekeeper发布标签；upstream未动，同步须--no-tags。
- 发布协议：[Tauri手动打包与预发布](../agents/tauri-release.md)。默认publish=false仍只打包；显式publish才写Release，拒绝覆盖已有版本。
- Windows无证书，macOS未开发者签名/公证；新包未真机安装/首次启动验收。真实模型/MCP/业务云及前台GUI仍未授权；无子代理。旧原生验收证据不等于本次安装包认证。
