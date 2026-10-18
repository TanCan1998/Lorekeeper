# 当前交接入口

- 最新：[第111次：进程归组深入调研、发布暂停](./2026-10-19-tauri-migration-status.md)。**不是归组修复或新版发布完成报告**。
- 代码44259fa3已推送：LoreKeeper / 1.0.0-beta.1，中文NSIS/MSI、安全清理入口与显式Latest选项已实现；标识/数据根/MSI UpgradeCode保持不变。
- 第110次代码验证：本机Rust清理9+访问9、Node发布19+安装6（含NSIS真实编译）、前端14均通过，typecheck/lint/build与定向格式/diff成功。没有运行安装卸载、真实删除或注册表修改。
- 44259fa3的Rust云端运行39148045359已success：1689项通过、all-targets成功，Rust语义切片累计清零；提交消息与Gitleaks同SHA均成功。
- **Windows WebView2归组未修复**：Tauri15567/WebView2Feedback5628仍open；macOS活动监视器归属待真机验证。不能以改名/图标冒充归组。
- 用户最新明确“不接受，继续搜索调研”，拒绝归组未修复先发布，故维持新版发布暂停，没有新标签/Latest。公开仍[旧1.0.0-beta](https://github.com/TanCan1998/Lorekeeper/releases/tag/lorekeeper-v1.0.0-beta)。
- 卸载默认保留，删除须显式目录+两次确认；只清本应用注册键，不碰共享WebView2、Vela或全局配置。原生权限/删除/恢复生命周期仍待验收。
- 新增[归组机制研究](../research/webview-process-attribution.md)：DPI shim的shell启动路径、真实包身份、WebKit XPC/LaunchServices。诊断flag禁止直接生产使用；下一步后台隔离A/B，尚未启动新原生宿主或修改产品。上轮测试结果不归给研究。
- 上游d21207c8与定稿单星图标e31c42d1已接入；禁止子代理、不抢焦点，其他未授权边界不变。活动状态见 `.pi/tmp/pipeline-state.md`。
