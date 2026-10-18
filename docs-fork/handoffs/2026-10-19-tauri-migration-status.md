# LoreKeeper迁移进度快照（2026-10-19）

> [当前入口](./CURRENT.md) · 第 **111** 次：进程归组深入调研，用户明确拒绝带缺陷发布。本轮只读研究，无产品修改或新代码测试。**不是进程归组修复或新版发布完成报告。**

## 已验证状态（代码验证沿用第110次，不冒充本轮重测）

- 代码 **44259fa3fff24134fe548ed72bf35dfe1b8b705b** 已提交推送 `origin/master`。名称改为 LoreKeeper，应用版本为 1.0.0-beta.1；标识、数据根、既有MSI UpgradeCode保持不变。
- 本轮未修改根 `package.json`、`pnpm-lock.yaml`、`vite.config.ts`、`.npmrc`、`electron/`、根 `src/` 或上游镜像 `docs/`；无新依赖、schema或渲染层IPC权限。
- 上轮上游同步仍有效：合入 `d21207c8` 的19提交，合并 `460fcd77`；英文README与上游一致，中文仅保留顶部fork说明。图标代码 `e31c42d1` 已接入，详见[图标登记](../design/branding.md)。

| 验证范围 | 最近结果 | 版本与证据 |
|---|---|---|
| Rust 清理守卫 | 9通过，覆盖默认保留、根/主目录/全局根、嵌套项目、Vela、junction、大小写、确认期间目录/清单替换 | 44259fa3；本机 `.pi/tmp/installer-validation/rust-cleanup.log` |
| Rust 项目访问回归 | 9通过 | 同上 `project-access.log` |
| Node 发布与安装合同 | 19+6通过；含真实NSIS编译器语法验证，仅编译不运行 | 同上 `contracts.log` |
| Tauri 前端 | 品牌/图标/设置14通过；typecheck、lint、build成功，构建有大chunk提示 | 同上 `icon-test.log`、`typecheck.log`、`lint.log`、`build.log` |
| 格式与双栈卫生 | 定向rustfmt、diff检查通过；保护目录无变更 | 同上 `format-check.log`、`diff.log` |
| Rust 云端全量 / all-targets | **1689通过、0失败，all-targets成功** | [44259fa3 / 39148045359](https://github.com/TanCan1998/Lorekeeper/actions/runs/39148045359)，2026-10-19 01:15 +08:00；本机 `.pi/tmp/installer-validation/cloud-full.log`，累计Rust语义切片清零 |
| 提交与秘密扫描CI | Commit message CI39148045247、Gitleaks39148045288均success | 同一代码44259fa3，不替代原生验收 |
| 原生安装/卸载/删除/注册表 | **未执行** | 无前台GUI、真实文件删除或系统配置修改 |
| 新版打包与发布 | **未触发** | beta.1 Release API返回404；没有创建新标签或改Latest |

## 本次更新：归组机制调研

用户明确“不接受，继续搜索调研”，因此归组缺陷解决前保持发布暂停，不再询问是否带此限制先公开。新增[研究结论与验证矩阵](../research/webview-process-attribution.md)：

- 微软官方记录DPI兼容shim可能经shell启动浏览器，发现可证伪的父链假设；相关flag被明确限定为开发诊断，不能直接用于生产。
- 比对Tauri默认清单和Tao运行时DPI初始化，拟以隔离宿主做默认/诊断/显式manifest的A/B；尚未运行，也不承诺manifest会修复归组。
- 真包身份不等于AUMID；MSIX/外部位置包是待评估路线，不能未经确认改安装注册或假定能复刻Teams。
- macOS已追到WebKit XPC宿主标识、LaunchServices内容进程命名源码，以及Apple层级视图文档；命名、宿主关联和视觉归组分别验收，不能混为一谈。
- 本机仅只读观察Windows26300、WebView2 155.0.4283.45及现有浏览器父链，当时无LoreKeeper实例，不能当本应用归组测试。证据在忽略目录 `.pi/tmp/process-group-research-2/`，无新窗口或系统配置修改。
- 部分搜索引擎为空/TLS失败/无关结果，不作为证据；Context7本轮可用，关键事实由官方文档与固定源码交叉确认。

## 前一代码切片：已实现，原生验收未完成

### 中文安装与安全清理

- NSIS固定当前用户、SimpChinese，安装/卸载图标采用定稿资源。MSI采用zh-CN，数字版本从1.0.0.0递增至1.0.1.0，固定UpgradeCode `ed3e3199-4ddc-501a-a745-179a185e8dfb`。
- EXE卸载默认保留项目。勾选“清理所选项目”后启动独立 `--cleanup-project` 模式，原生选择一个有效LoreKeeper项目目录，并两次明确选择删除才执行清理。保留、取消、关闭与未知返回均不能授权删除。
- 清理模式不启动WebView/业务服务、不读最近项目、不接收目录参数、不开放IPC。确认后重验目录身份和清单；拒绝系统/程序/主目录/全局数据根、链接、嵌套项目及Vela数据，不扫盘。
- 接管模板原AppData勾选框，禁止模板随后按AppData路径递归删除。本应用安装位置/语言注册键与“保留项目”解耦；共享厂商键仅在空时移除。模板继续清理卸载项与AppUserModelId，不删除共享WebView2。
- 静默、被动、升级与 `_?=` 安装器维护调用均不启动项目清理。MSI与macOS拖拽卸载默认保留，卸载前可运行手动清理入口；macOS应用包附中文说明。系统拖拽流程不是自定义中文卸载向导。
- 以上是代码与本机守卫验证，不是实际安装/卸载或删除验收。

### 预览版Latest选项

- 工作流新增独立 `make_latest`，默认false；禁止预发布+Latest组合。
- beta名称允许普通Release，但说明须明确预览和非正式版、标题带预览标签。草稿始终不设Latest，公开时才应用选项，选择Latest后核对远端指针。
- 四包精确核验采用LoreKeeper名称与zh-CN MSI，保留同一SHA云端全量、三平台、大小/摘要和不覆盖旧标签等门禁。
- [beta.1说明](../releases/1.0.0-beta.1.md)已准备，仍未公开。详见[发布协议](../agents/tauri-release.md)。

## 尚未完成与下一步

1. **进程归组未修复**：本轮再次核查[Tauri #15567](https://github.com/tauri-apps/tauri/issues/15567)与[WebView2 #5628](https://github.com/MicrosoftEdge/WebView2Feedback/issues/5628)，均open且无解决方案。AppUserModelID、名称和图标不能冒充归组修复。macOS WebKit源码会传宿主名/bundle-id，但活动监视器显示未真机验证。研究材料 `.pi/tmp/installer-research/`。
2. **发布明确暂停**：用户最新明确拒绝归组未修复先公开，并要求继续研究。没有触发新版打包/发布；不得再把此前普通Release+Latest方案授权解释为接受缺陷交付。下一步优先后台隔离父链/DPI实验，而非再次请求豁免。
3. **原生验收待授权**：中文EXE/MSI、升级身份、默认保留、目录选择/二次确认、真实项目清理与注册表清理、macOS生命周期及图标均需原生验收。无法后台完成的原生对话框不得切前台。
4. 44259fa3对应云端全量已成功；完整活动run与最终HEAD由 `.pi/tmp/pipeline-state.md` 单写维护。

公开仍为[旧1.0.0-beta预发布](https://github.com/TanCan1998/Lorekeeper/releases/tag/lorekeeper-v1.0.0-beta)，旧标签与九资产不覆盖。禁止子代理、不抢焦点；真实模型/MCP/业务云、签名公证与新包真机安装尚未认证。具体历史授权见本机 `.pi/tmp/branding-install-state.md`。
