# 当前交接入口

- 最新：[2026-10-18-tauri-migration-status.md](./2026-10-18-tauri-migration-status.md)，第 **103** 次。
- 代码 **495f5632**，master，已推送；Rust 指纹仍为 0f0880d2（1692/0）。
- **Tauri 三平台打包流水线可用**：`.github/workflows/tauri-release-package.yml` 手动 dispatch，Windows x64 / macOS ARM64 / macOS Intel 并行，只出 artifacts 不发布。run 39110738571 三平台全 success，墙钟 21 分 12 秒。
- 产物：setup.exe 47.7MB、msi 51.6MB、aarch64.dmg 53MB、x64.dmg 53.3MB；四个 SHA256 与各自 manifest 逐项核对一致；均未签名、未发布、未做真机安装验证。
- **字体瘦身**：Tauri 复用根 `public/fonts`，TTF 91.1MB→WOFF2 34.8MB，dist 96→39.2MB。界面旧品牌统一为设定司/Lorekeeper，`tauri-app/public` 上游残留已删，打包前有 `assert-package-contents` 拦截上游标识与 TTF。
- **工具链**：pnpm 12.14.0（本机与 tauri-app，根树仍 11.11.0）、Node 24.22.0、checkout v7.0.1 / setup-node v7.1.0 / upload-artifact v7.0.2 / pnpm-action v6.1.0。
- 本轮三次 CI 失败均已定位修复（tsc 跨目录解析、pnpm 版本冲突）；pnpm 升级后曾见 1 例测试失败，连续 5 次未复现。
- #30 可登记关闭；#31 首版范围 290 频道全部有处置（289 迁移 + 1 明确不支持）、事件 7、版本 1.1.0。
- 不能推出真实模型/MCP/业务云/发布就绪；macOS/Linux 未认证，签名与真机安装未验证。
- 不派子代理、不删 target、不改双栈保护目录与根配置。
