# 当前交接入口

- 最新：[第113次：包身份正向归组、产品待接入](./2026-10-19-tauri-migration-status.md)。**不是归组修复或新版发布完成报告**。
- 代码44259fa3已推送：LoreKeeper / 1.0.0-beta.1，中文NSIS/MSI、安全清理入口与显式Latest选项已实现；标识/数据根/MSI UpgradeCode保持不变。
- 第110次代码验证：本机Rust清理9+访问9、Node发布19+安装6（含NSIS真实编译）、前端14均通过，typecheck/lint/build与定向格式/diff成功。没有运行安装卸载、真实删除或注册表修改。
- 44259fa3的Rust云端运行39148045359已success：1689项通过、all-targets成功，Rust语义切片累计清零；提交消息与Gitleaks同SHA均成功。
- **Windows找到正向条件，产品尚未修复**：完整MSIX最简宿主无需诊断flag即可把宿主+全部6个WebView2放在同一应用组；同一EXE正反复验成立。Tauri15567/WebView2Feedback5628此前仍open；macOS待真机。不能以探针结果冒充产品完成。
- 用户最新明确“不接受，继续搜索调研”，拒绝归组未修复先发布，故维持新版发布暂停，没有新标签/Latest。公开仍[旧1.0.0-beta](https://github.com/TanCan1998/Lorekeeper/releases/tag/lorekeeper-v1.0.0-beta)。
- 卸载默认保留，删除须显式目录+两次确认；只清本应用注册键，不碰共享WebView2、Vela或全局配置。原生权限/删除/恢复生命周期仍待验收。
- [归组机制研究§7](../research/webview-process-attribution.md#7-第113次真实包身份取得正向结果)：同一二进制五次无flag对照，无包前后拆组，带包默认两次及另设窗口AUMID一次均归组；正向逐PID匹配7/7自有进程，共同应用组另含Windows Broker，合计8实例。DPI、无包AUMID和EXE描述对照均无效。
- 用户明确“同意”唯一临时测试包注册/精确移除（可能全用户），不包括证书或开发模式。测试包全用户剩余0、自有宿主/WebView2遗留0、任务管理器排序已恢复且关闭；未新增证书/工具或改产品。正式MSIX/安装器配套身份、签名与发布配置仍须另行确认。
- 上游d21207c8与定稿单星图标e31c42d1已接入；禁止子代理、不抢焦点，其他未授权边界不变。活动状态见 `.pi/tmp/pipeline-state.md`。
