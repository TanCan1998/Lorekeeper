# WebView 进程归属与系统归组调研

日期：2026-10-19。产品代码仍为 `44259fa3`，本轮不修改产品、依赖或发布配置。

**用户明确不接受归组缺陷，要求继续调研。新版继续暂停发布，不再把“带已知限制先发布”作为默认下一步。本文不是修复报告。**

## 1. 验收对象

需要同时区分四件事：

1. 宿主与 WebView 浏览器/内容/GPU/网络进程的实际归属；
2. Windows 父进程链、包身份以及任务管理器“进程”页的视觉归组；
3. macOS XPC 宿主身份、LaunchServices 显示名及活动监视器的层级视图；
4. 各进程自身的可执行文件名与显示名称。

改图标、productName、AUMID或自定义协议不能单独证明第1–3项通过；也不允许靠隐藏进程、修改共享运行库或关闭沙箱达标。

## 2. Windows：新增的两条机制线索

### 2.1 DPI 兼容 shim 的 shell 启动路径

[微软浏览器开关文档](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/webview-features-flags)明确说明：

> edge-webview-no-dpi-workaround：关闭 DPI awareness app compatibility shim 的绕行机制；该绕行会通过 shell 启动 WebView 浏览器进程，以避免继承应用兼容 shim。

这为“为何浏览器不再直接归宿主”提供了**可检验假设**，并不证明本应用实际命中了该路径。微软在[问题3804](https://github.com/MicrosoftEdge/WebView2Feedback/issues/3804#issuecomment-1736335518)及[问题3692](https://github.com/MicrosoftEdge/WebView2Feedback/issues/3692#issuecomment-1736336634)中曾用该开关诊断Office宿主DPI问题；这些问题不是任务管理器归组修复证据。

重要限制：同一文档明确写明 **“For production apps, do not use these flags.”** 因此该开关只能用于隔离的开发A/B，不能直接加入正式配置，也不能写用户/系统注册表或全局环境变量。

[Tauri 2.12.1 默认Windows清单](https://github.com/tauri-apps/tauri/blob/tauri-v2.12.1/crates/tauri-build/src/windows-app-manifest.xml)只声明Common Controls依赖，没有DPI声明。[Tao 0.37.1](https://github.com/tauri-apps/tao/blob/tao-v0.37.1/src/platform_impl/windows/dpi.rs)在运行时调用 `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)`，失败则回退。

[微软DPI初始化规范](https://learn.microsoft.com/en-us/windows/win32/hidpi/setting-the-default-dpi-awareness-for-a-process)推荐在manifest声明；程序调用虽受支持，但须发生在任何HWND创建前。

**候选实验，而非结论**：在同一宿主/运行库/隔离UDF下比较默认方案、进程局部诊断开关、显式PerMonitorV2 manifest。若浏览器原本已直接以LoreKeeper为父进程，不能继续用“shell改变PPID”解释视觉拆组；manifest实验即使成功，也须去掉诊断开关单独复验，追加缩放与多屏回归。

### 2.2 真正的包身份不是 AUMID

上述微软文档另列：

- `edge-webview-run-with-package-id`：Desktop Bridge应用携带包身份启动WebView进程；
- `edge-webview-run-without-package-id`：反向诊断选项。

[WebView2问题3556](https://github.com/MicrosoftEdge/WebView2Feedback/issues/3556#issuecomment-1587654479)提到已封装Desktop Bridge宿主可尝试携带其包身份；作者明确说明当时未实测，场景是Assigned Access白名单，**不是归组验收**。

[微软身份检测规范](https://learn.microsoft.com/en-us/windows/msix/detect-package-identity)区分：

- `GetCurrentPackageFullName` 判定进程是否真正拥有包身份；
- 任务管理器“详细信息→包名称”可以观察身份；
- 包名显示不是唯一标识，也不是视觉归组证明。

普通NSIS/MSI安装和设置AppUserModelID不会凭空赋予MSIX身份。[外部位置/稀疏包](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/grant-identity-to-nonpackaged-apps)是受支持的赋予身份路径，但涉及新的安装注册、签名及升级策略；不能未经确认就改变本项目打包路线，也不能假定Teams的行为仅由身份决定。诊断开关本身同样不用于生产。

优先做只读身份对照，确认真实差异后再决定是否值得提出该方案。

### 2.3 正常共享与视觉归组不是同一个合同

[官方进程模型](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-model)的“WebView2 process group”指浏览器、renderer和helper集合；集合与UDF关联。相同配置和UDF可让多个environment甚至不同宿主共享浏览器。它不是对任务管理器UI归组的承诺。

[官方FAQ](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/end-user-faq)说明通常应按父应用显示，按非名称列排序时视图可能不正确；“详细信息”页仍显示 `msedgewebview2.exe`。验收必须记录页签/排序/筛选，而不是更改用户显示设置来掩盖产品问题。

[Tauri #15567](https://github.com/tauri-apps/tauri/issues/15567)和[WebView2 #5628](https://github.com/MicrosoftEdge/WebView2Feedback/issues/5628)本轮重新读取仍open。前者维护者承认同机复现，后者仍无评论；不是已经存在可直接套用的上游修复。

## 3. macOS：宿主身份与显示名已有可追踪源码

固定源码版本：WebKit `3a41ad1d523ccc432418410723dfe5e60f1f9a63`。源码代表机制，不能替代目标机器所安装WebKit的验证。

- [ProcessLauncherCocoa.mm](https://github.com/WebKit/WebKit/blob/3a41ad1d523ccc432418410723dfe5e60f1f9a63/Source/WebKit/UIProcess/Launcher/cocoa/ProcessLauncherCocoa.mm)：XPC启动消息包含 `client-bundle-identifier`、`ui-process-name`。名称优先取 `NSRunningApplication.currentApplication.localizedName`，回退到 `NSProcessInfo.processInfo.processName`。
- [XPCServiceEntryPoint.mm](https://github.com/WebKit/WebKit/blob/3a41ad1d523ccc432418410723dfe5e60f1f9a63/Source/WebKit/Shared/EntryPointUtilities/Cocoa/XPCService/XPCServiceEntryPoint.mm)：读取宿主bundle标识。
- [WebProcessCocoa.mm](https://github.com/WebKit/WebKit/blob/3a41ad1d523ccc432418410723dfe5e60f1f9a63/Source/WebKit/WebProcess/cocoa/WebProcessCocoa.mm#L733-L792)：`updateProcessName`会构造“宿主名 Web Content / Web Inspector / Service Worker”等显示名，经LaunchServices更新，或交由网络进程更新。相关私有调用属于WebKit内部，不能据此让应用调用私有API强制改系统进程。
- [Tauri #9813的维护者回复](https://github.com/tauri-apps/tauri/issues/9813#issuecomment-5590164990)：`tauri://localhost`类内容进程名称不能由普通Tauri配置任意控制；换协议最多改其中的协议部分。这仍是名称问题，不等于解决归组。

[Apple活动监视器文档](https://support.apple.com/guide/activity-monitor/view-information-about-processes-actmntr1001/mac)区分“All Processes”和“All Processes, Hierarchically”，后者展示父子关系。XPC宿主关联、内核PPID、LaunchServices名称及界面层级应分别核查；不能承诺应用能改变活动监视器当前视图，也不能把名字变成LoreKeeper当成“全部挂在LoreKeeper下”。

验证必须使用已打包 `.app`，同时与直接启动二进制的开发方式区分，核对bundle标识、宿主名称、子进程归属及界面分组。不改私有WebKit进程、不关闭沙箱。当前Windows环境没有完成这项Mac真机验证。

## 4. 本轮只读观察与证据

- 本机Windows build **26300**，观察到运行中的系统/其他宿主使用WebView2 **155.0.4283.45**。
- 被观察的浏览器根进程以各自宿主进程为父，未携带上述两个诊断开关；会话未设置 `__COMPAT_LAYER`。
- **当时没有LoreKeeper实例**。这些观察不证明LoreKeeper的父链、其他软件的视觉归组，或兼容性shim不存在。仅在内存提取必要的进程字段，未输出或保存完整命令行；没有读取页面、小说或凭据文件，仅保留宿主名、父进程名、版本与两个开关是否存在的摘要。
- Moli搜索覆盖DuckDuckGo、Brave、Yahoo、Baidu、Bing和Naver。Brave/Yahoo出现空正文，Naver出现TLS失败，Bing命中过无关结果；这些不计作有效证据。主要结论取自微软/Apple文档、固定源码和维护者原帖。
- Context7本轮已恢复可用；它检出的实验cluster接口、文件系统权限接口不是任务管理器归组API，不据此引入实验依赖。
- 原始证据在本机忽略目录 `.pi/tmp/process-group-research-2/`，不提交大输出、临时工具或第三方完整源码。公开文档只保留脱敏结论与来源。
- 外部库文件的context-mode路径访问被拒绝后，改查公开的固定版本源码；没有更改工具权限规则。

## 5. 下一步验证矩阵

| 顺序 | 操作 | 判定与边界 |
|---|---|---|
| 1 | 无激活的专用宿主，唯一隔离UDF，仅加载本地合成页面；读取BrowserProcessId、父链、启动时间、运行库版本、宿主/浏览器包身份 | 不接触真实数据，不复用现有用户UDF；禁止全局键鼠和激活窗口 |
| 2 | 默认 / 进程局部DPI诊断 / 显式manifest的A/B | 保留Wry默认参数，仅进程局部追加；不关闭沙箱或安全策略。比较真实父链；隐藏窗口实验不能冒充任务管理器视觉归组通过 |
| 3 | 条件允许时后台读取任务管理器现有UI归组 | 主窗口自身不可因测试隐藏；页签/名称排序/搜索等条件一致；若无法保证不抢焦点则标待验收 |
| 4 | 检验支持的生产候选 | 所有诊断开关移除；多屏DPI/启动/关闭/多实例/UDF隔离均回归后，才讨论产品修改 |
| 5 | macOS打包.app同类核验 | 对应Mac真机；不调用私有API、不操纵系统进程或活动监视器设置 |

本轮**没有启动该矩阵的新原生宿主**，没有改Windows注册表/系统环境，没有执行真实安装或重新发布。尚无可登记为已解决的跨平台方案。后续优先用父链证据证伪DPI假设，再研究包身份，不盲加flag或等待未承诺的上游修复。
