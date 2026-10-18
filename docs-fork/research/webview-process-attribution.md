# WebView 进程归属与系统归组调研

日期：2026-10-19。第111次记录机制研究，第112次新增DPI实测（§6），第113次取得真实包身份的正向归组结果（§7）。产品代码仍为 `44259fa3`，未修改产品、依赖或发布配置。

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

第111次调研阶段没有启动新原生宿主。随后第112次按用户要求使用最简程序实测，见下节；不能再沿用“尚未测试DPI假设”的旧断点。两轮均没有修改Windows注册表/系统环境、执行真实安装或重新发布。

## 6. 第112次：最简程序实测

### 方法与授权

用户要求“测试，可用最简程序测以加快测试速度”，随后明确允许自行打开任务管理器，并以“同意”授权本次限定的管理员检查器。授权仅涵盖测试与任务管理器观察，不扩展到包注册、证书、系统配置、真实项目删除或发布。

使用系统已有C#编译器、已有WebView2Loader和稳定COM基接口，**不编译完整Tauri/Rust、不新增依赖或安装工具**。每次创建独立UDF，只用NavigateToString加载带严格CSP的内联合成HTML，不读真实项目、不调用模型/MCP/业务云。

探针仍以普通权限运行；管理员权限只用于读取管理员任务管理器的UIA树。读取器采用每轮UUID隔离通道和源码摘要校验，工作结束即退出。探针的BrowserProcessId由SDK接口取得，并与系统进程快照交叉检查，而非靠名称猜测归属。

### 后台机制矩阵：12次完整启动，36份快照

- 两种宿主：消息窗口HWND_MESSAGE、普通隐藏HWND。
- 每种宿主比较三组，各重复两次：无DPI清单+运行时PerMonitorV2（模拟Tao初始化）、同一二进制追加进程局部DPI诊断开关、显式PerMonitorV2清单且不加诊断开关。
- 每次三份父链快照；**全部浏览器均直接以探针为父**，三组没有差异。
- 显式清单确实使进程初始即为PerMonitorV2；非清单组由unaware经API变为PerMonitorV2，证明对照生效。
- 普通隐藏HWND组成功读回显式进程AUMID；宿主与浏览器的包身份查询均返回15700（无包身份）。显式AUMID不能被当成包身份。
- 两套6次矩阵各约22秒（含编译与等待），没有完整Rust编译。这不是整个调试会话耗时。

### 真实任务管理器矩阵：三组均未修复

探针创建普通顶层窗口，Win32可见性为true，带NOACTIVATE样式、非置顶，放在虚拟桌面外且置于Z序底部。不是用隐藏宿主来冒充视觉验收，也未调用全局键鼠或SetForegroundWindow。

先取得CPU排序下三次探索性UIA结果；随后明确改为**名称升序**，以宿主/浏览器及其子进程的精确PID匹配任务管理器UIA行，读取分组祖先，严格核验另外三次。每组探针保持12秒，完成12份父链采样。

| 名称排序下的组别 | 宿主PID → 浏览器PID | 实际父进程 | 任务管理器宿主位置 | 浏览器及helper位置 |
|---|---|---|---|---|
| 模拟Tao运行时DPI初始化 | 21280 → 19176 | 直接为宿主 | 应用 / LorekeeperProbe.exe | 后台进程 / WebView2管理器，6实例 |
| 追加DPI诊断开关 | 20432 → 32136 | 直接为宿主 | 应用 / LorekeeperProbe.exe | 后台进程 / WebView2管理器，6实例 |
| 显式PerMonitorV2清单 | 7616 → 33512 | 直接为宿主 | 应用 / LorekeeperProbe.exe | 后台进程 / WebView2管理器，6实例 |

六实例包含浏览器、GPU、网络、存储、renderer及Crashpad；UIA祖先中均没有宿主进程行。不能以renderer名称变为“WebView2: LoreKeeper Probe”冒充归组解决。

运行库为 **155.0.4283.62**，与前轮只读观察仍在运行的155.0.4283.45不同；本次各对照都使用.62，不混作跨版本回归证据。有效完整矩阵合计18次启动、108份原生采样，其中名称排序的三次为严格视觉归属核验。失败的检查器准备尝试、一次被旧会话停止消息中断的尝试不计入成功次数。

### 收尾与证据

- 所有有效探针退出码0，控制器正常关闭；采样未发现探针宿主/浏览器占用前台窗口，相关UIA操作前后前台句柄不变。用户同意后的UAC权限交互不伪装成无提权测试。
- 任务管理器原为CPU降序；收尾恢复该排序并正常关闭本次自行启动的窗口。最终检查自有探针遗留进程0、任务管理器已关闭，管理员检查器停止/退出码0。
- 没有修改现有应用身份、全局环境、开发模式或证书，没有执行注册表写入命令或注册Appx/MSIX包。任务管理器本次导航/排序变化已恢复；原始UDF及证据保留在忽略目录，未自动删除。
- 本机证据：`.pi/tmp/process-group-probe/runs/`（两套隐藏实验）、`visual/2026-10-18T22-11-03-585Z/`（名称排序与PID祖先核验）、`result-digest.json`、`taskmgr-finish.json`、`final-cleanup.json`。最终矩阵的源码、清单、检查器和二进制已归档并记录摘要；不把完整UIA报告、第三方DLL或可执行文件入库。

### 结论及局限

1. **本机该最简宿主中，DPI开关和显式DPI清单都不是修复。** 正确PPID与任务管理器归组可以同时出现分歧；不能再以“父链正确”登记通过。
2. 不依赖Tauri/Wry的原生COM最简宿主也能复现分列，因此不是只有改Tauri代码才能观察的问题。但本实验是无激活、屏幕外窗口，不能由此断言所有普通前台宿主都失败，也不能证明具体Windows/运行库版本就是根因。
3. **包身份尚未对照测试。** 现有结果只证明两边都无包身份；不能宣称包身份必然解决。后续若需临时包注册、签名/证书或开发模式变化，须另行明确确认，不能复用只读检查器提权授权。
4. macOS尚未真机测试；Windows探针不替代跨平台验收。新版继续暂停发布。

## 7. 第113次：真实包身份取得正向结果

**结论：本机普通完整MSIX测试包在不加浏览器诊断开关时，实现了宿主与全部6个WebView2进程的真实应用归组。不是改名或隐藏进程；尚不是LoreKeeper产品修复。**

### 先排除窗口身份与版本信息

四组无包身份对照分别为：不设显式AUMID、只设进程AUMID、进程与窗口PKEY_AppUserModel_ID一致、再补齐EXE版本元数据。均正常退出，但仍拆组。窗口属性通过SHGetPropertyStoreForWindow写入/读回成功，退出前用VT_EMPTY清空；最后一组FileDescription确实由空变为LoreKeeper Probe，却不改变分组祖先。不能把任务栏身份/描述变化当成此需求的解决。

### 新的限定授权与临时部署

用户在说明风险后明确选择“同意”：允许为唯一随机名 `Lorekeeper.ProcessProbe.7b39e5c860a6` 注册并精确移除临时测试包，知道未签名可执行包可能需要管理员并涉及所有用户。仅扩展此测试包操作，不允许证书/开发模式变更、现有产品安装、项目删除或发布。

使用已有Windows SDK 10.0.26100.0的MakeAppx及系统Add-AppxPackage/Remove-AppxPackage，无新工具安装。按[微软未签名包文档](https://learn.microsoft.com/en-us/windows/msix/package/unsigned-package)使用专用OID发布者，包身份与任何签名发行包分离；没有新增信任证书。每轮注册前要求同名包不存在，结束按精确Name/FullName/Publisher/Version校验后移除，不做通配卸载。

外部位置包虽注册成功，嵌入msix身份清单的探针启动被SxS错误14001拒绝；仅匹配自有路径的SideBySide事件87明确指向publisher校验。该尝试**没有带身份的运行/归组结论**，也不能据此宣布签名稀疏包不可行。随后改为普通完整测试包（packagedClassicApp/mediumIL、标准打包校验），没有关闭系统校验或沙箱，也未使用可能强制前台的应用激活方式。已有NSIS/MSI和产品配置未变。

### 首轮正向对照

同轮运行库155.0.4283.62、名称升序、普通权限探针、独立UDF及相同合成页面。未打包基线仍拆组；完整包默认组的宿主与浏览器GetPackageFullName/GetApplicationUserModelId均成功，具有相同包身份，UIA显示“应用 / LoreKeeper Process Probe，8实例”，包含宿主、6个WebView2进程与Windows Runtime Broker。额外诊断组也归组，但**默认组不需要 `--edge-webview-run-with-package-id`**，该flag不是生产方案。

### 同一二进制、无诊断flag的正反复验

为排除编译差异，再用SHA256完全相同的EXE，交替从普通临时目录与已注册包的安装目录启动；五次均不传浏览器诊断参数。

EXE SHA256：`de67eedf218b0d1dc5e84acc10ebde8faeddb4b2f5ef148dba45114fc62e0c77`。

| 对照 | 宿主PID → 浏览器PID | 两边包身份状态 | 实际任务管理器归属 |
|---|---|---|---|
| 普通目录，之前 | 16792 → 26372 | 15700，无包 | 浏览器独立后台WebView2管理器组 |
| 完整包，默认第1次 | 11692 → 15776 | 0，相同包/应用ID | 宿主+6个WebView2在同一应用组 |
| 完整包，另设进程/窗口AUMID | 29076 → 20792 | 0，相同包/应用ID | 仍在同一应用组 |
| 完整包，默认第2次 | 33300 → 24452 | 0，相同包/应用ID | 宿主+6个WebView2在同一应用组 |
| 普通目录，之后 | 35584 → 24136 | 15700，无包 | 再次独立后台WebView2管理器组 |

三个正向用例均逐PID匹配到**7/7个自有原生进程**，UIA祖先具有同一个应用进程组容器；Windows额外添加的Runtime Broker使界面合计8实例。“WebView2管理器”仍是应用组内的浏览器子行，而不是应用之外的独立后台组。前一个负向用例受UI虚拟化影响仅匹配6/7行，但宿主和浏览器行均可确认分离；后一个负向用例匹配7/7行，确认没有共同应用容器。不能以名称相似推断归组。

五次父链均直接归宿主；每次12份快照、正常退出0、探针前台占用采样0，相关UIA操作前后台句柄不变。此前四组窗口身份矩阵与完整包首轮另计；只把三套完整矩阵的12次/153份原生采样作为本轮有效结果，失败准备/中断及未成功启动的稀疏包不计入。

### 收尾、证据与产品边界

- 每次注册监督器均在finally中清理并报告全用户同名包剩余0；最终普通权限再次核验当前用户同名包0、自有宿主/WebView2遗留0、任务管理器已恢复原排序并关闭。管理员读取器与注册监督器正常退出，不强制结束Windows系统Broker。
- 未装证书、未开开发者模式、未安装新工具、未更换/修改共享WebView2。仅发生用户授权的临时Appx部署与移除；不能再写成“从未修改系统注册记录”。源码、包文件、UDF和证据留在忽略目录，不入库。
- 核心证据：`.pi/tmp/process-group-probe/identity/2026-10-18T22-54-38-100Z/`、`package-verification-digest.json`、`package-final-cleanup.json`、`package/7b39e5c860a6/control/`的逐轮收据。完整包首轮在 `identity/2026-10-18T22-50-50-176Z/`；无包四组在 `identity/2026-10-18T22-37-36-523Z/`。同一二进制、清单、检查器与控制器已归档。
- **证明的是本机最简宿主在真实完整包身份下可正确归组**；未证明身份是所有Windows环境中的唯一成因，未证明现有NSIS/MSI已经修好，未验证LoreKeeper真实二进制或macOS。
- 生产接入有两条待评估路线：完整MSIX，或为既有NSIS/MSI配套受支持的签名外部位置身份包。前者涉及发布/升级与卸载体验变化；后者尚未正向验证，测试签名证书的生成、当前用户临时信任及移除须另行明确同意，不能复用本次无证书包注册授权。正式签名、发布配置与产品权限变更仍须先确认。
- **发布继续暂停**，不再宣称Windows“只能接受系统限制”，也不以最简探针正向结果冒充产品/三平台完成。
