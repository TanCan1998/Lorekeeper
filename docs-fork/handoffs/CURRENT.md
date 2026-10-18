# 当前交接入口

- 最新：[第114次：签名稀疏身份原路径归组、产品待接入](./2026-10-19-tauri-migration-status.md)。**不是归组修复或新版发布完成报告**。
- 代码44259fa3已推送：LoreKeeper / 1.0.0-beta.1，中文NSIS/MSI、安全清理入口与显式Latest选项已实现；标识/数据根/MSI UpgradeCode保持不变。
- 第110次代码验证：本机Rust清理9+访问9、Node发布19+安装6（含NSIS真实编译）、前端14均通过，typecheck/lint/build与定向格式/diff成功。没有运行安装卸载、真实删除或注册表修改。
- 44259fa3的Rust云端运行39148045359已success：1689项通过、all-targets成功，Rust语义切片累计清零；提交消息与Gitleaks同SHA均成功。
- **Windows两条身份路线均有正向证据，产品尚未修复**：完整MSIX及签名稀疏身份包最简宿主均无需诊断flag即可让宿主+全部6个WebView2同组；后者同一EXE、同一路径注册后三次正向、撤销后负向成立。Tauri15567/WebView2Feedback5628此前仍open；macOS待真机。不能以探针结果冒充产品完成。
- 用户最新明确“不接受，继续搜索调研”，拒绝归组未修复先发布，故维持新版发布暂停，没有新标签/Latest。公开仍[旧1.0.0-beta](https://github.com/TanCan1998/Lorekeeper/releases/tag/lorekeeper-v1.0.0-beta)。
- 卸载默认保留，删除须显式目录+两次确认；只清本应用注册键，不碰共享WebView2、Vela或全局配置。原生权限/删除/恢复生命周期仍待验收。
- [归组研究§8](../research/webview-process-attribution.md#8-第114次签名稀疏身份包原路径正向验证)：签名稀疏包五次/60快照，三个正向各7/7自有PID共同应用组；注册前首帧祖先不完整不冒充严格负向，撤销后7/7确认拆组。[保留NSIS/MSI接入草案](../plans/windows-sparse-identity-integration.md)待确认，产品/CI尚未改。
- 用户单独“同意”一天、代码签名、私钥不可导出的临时证书及全机TrustedPeople信任（不是Root）。测试后包/全机信任/个人证书均0、私钥不存在，独立复核通过；探针无遗留、Taskmgr恢复并关闭。该授权不涵盖生产签名、真实产品安装或发布配置。
- 上游d21207c8与定稿单星图标e31c42d1已接入；禁止子代理、不抢焦点，其他未授权边界不变。活动状态见 `.pi/tmp/pipeline-state.md`。
