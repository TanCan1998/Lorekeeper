# WebView 进程归属与系统归组调研

日期：2026-10-19。第111次记录机制研究，第112次新增DPI实测（§6），第113次取得完整MSIX包身份正向结果（§7），第114次验证签名稀疏身份在原路径也可归组（§8）。产品代码仍为 `44259fa3`，未修改产品、依赖或发布配置。

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

## 8. 第114次：签名稀疏身份包原路径正向验证

**签名外部位置身份包也取得正向结果：无需把EXE搬入完整MSIX安装目录。** 同一EXE、同一路径，注册后三次宿主与全部6个WebView2同组，撤销身份后再次拆组。这提供了保留NSIS/MSI的接入依据，但尚未实现或验收真实产品。

### 单独授权的临时信任

微软[包签名证书指引](https://learn.microsoft.com/en-us/windows/msix/package/create-certificate-package-signing)要求测试证书信任可能影响全机。明确说明后，用户再次选择“同意”：创建仅有效1天、仅代码签名用途、私钥不可导出的唯一测试证书，私钥留当前用户My；公钥临时加入**LocalMachine/TrustedPeople，而非Trusted Root**；只签署唯一测试身份包，结束精确移除包、信任、证书及私钥。不使用既有签名凭据，不安装新工具/开启开发模式/改产品或发布。

实际签名由既有SignTool完成，没有PFX/私钥导出或时间戳网络请求；签名前检查同名包、证书主体及CNG容器均不存在，提权用户SID须与普通权限父进程一致。签名指纹与生成证书一致，Get-AuthenticodeSignature为Valid。身份包使用普通CN发布者，不再使用前轮触发嵌入清单校验失败的未签名专用OID。

### 同字节、同路径、无诊断flag的五次对照

EXE SHA256：`e875add803a75aabf91be0cd73c9351aa87b9ff491d45d87d8eebea8041da445`。五次都从同一个临时external目录直接启动相同文件，普通权限、独立UDF、相同合成HTML、PerMonitorV2和屏幕外NOACTIVATE窗口；不用浏览器诊断flag。

| 对照 | 宿主PID → 浏览器PID | 两边包身份状态 | UIA结果 |
|---|---|---|---|
| 注册前 | 8648 → 7800 | 15700，无包 | 7/7行，但首帧浏览器分组祖先列表为空；不当作完整视觉负向证据 |
| 注册后，默认第1次 | 34268 → 31172 | 0，同包/应用ID | 7/7自有PID处于同一应用容器 |
| 注册后，另设进程/窗口AUMID | 5436 → 19176 | 0，同包/应用ID | 同一应用容器，7/7 |
| 注册后，默认第2次 | 33324 → 32020 | 0，同包/应用ID | 同一应用容器，7/7 |
| 撤销注册后 | 16516 → 14332 | 15700，无包 | 7/7；浏览器重新属于独立后台WebView2管理器组 |

正向界面为“应用 / LoreKeeper Process Probe，7实例”，恰为宿主+6个WebView2；该win32App稀疏包对照没有前轮完整包额外的Broker子行。成员仍可见，组内浏览器仍叫WebView2管理器，不靠改名或隐藏进程。宿主与浏览器API读回同一真实包/应用ID，所有父链仍直接为宿主。

五次均正常退出0，每次12份快照，合计60份；探针前台占用采样0，相关UIA操作前后台句柄不变。注册前的首帧限制如实保留，以撤销后的严格负向作为对照，不补造缺失祖先。

### 清理与后续边界

- 监督器finally按精确包名/发布者/版本/FullName移除测试身份，按本轮新生成指纹与主体移除全机TrustedPeople、当前用户My证书及其私钥；CNG容器还须复核不存在。
- 管理员回执：同名包0、全机临时信任0、个人测试证书0、私钥不存在、清理错误空。随后普通权限独立只读复核同样为0/false，自有宿主/WebView2遗留0、任务管理器已恢复排序并关闭。未改受信任根证书库或其他证书，不强杀系统Broker。
- 证据位于 `.pi/tmp/process-group-probe/signed/ec8d2211f1d6c119b820616878baad53/`：`verification-digest.json`、`independent-cleanup.json`、`control/`逐步回执及 `runs/2026-10-18T23-09-52-779Z/`。EXE、清单、控制器和检查器已固定；原始报告/包/证书信息不入库。
- **可行条件已经扩展到签名稀疏身份，不能再沿用“该路线未测试”的旧断点。** 但真实LoreKeeper二进制、安装器升级/卸载/失败回滚、文件权限及macOS仍未验证。
- [保留NSIS/MSI的接入草案](../plans/windows-sparse-identity-integration.md)只提出构建清单、签名、注册/升级/移除、MSI用户上下文及发布门禁要求；未改产品/CI/发布配置。本轮临时自签名证书已删除，不是正式发行的签名来源，也不授权在用户机器自动增加证书信任。生产接入与正式签名须另行确认，发布继续暂停。

## 9. 第115次：未签名小启动器与外部自选目录

用户随后明确“不签名但要归组，可预留签名证书位置”、安装时可选目录，并确认“小型完整MSIX身份启动器+外部主程序逻辑映射”方向。**不再把付费或正式签名稀疏包当实施前提**；不能通过联接WindowsApps/VFS伪装安装位置。

### 公开进程策略的正反对照

复用此前单独获准的唯一临时测试包名称/未签名部署范围，不增加信任或开发模式。普通权限包内启动器无窗口，使用同一个未签名最简EXE，SHA256仍为 `de67eedf218b0d1dc5e84acc10ebde8faeddb4b2f5ef148dba45114fc62e0c77`。真正宿主分别位于两个外部中文/空格目录，而不是WindowsApps中的主程序副本。

普通CreateProcess不会自动传递包身份。微软公开的[UpdateProcThreadAttribute](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)支持 `PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY`；使用 `PROCESS_CREATION_DESKTOP_APP_BREAKAWAY_DISABLE_PROCESS_TREE`（0x02）使外部主程序及后代保留桌面包环境。这里的DISABLE针对breakaway，不是关闭沙箱，也未伪造PPID或加入浏览器诊断参数。

先完成3次普通创建、4次包含策略的消息窗口实验，确认身份变化；随后同轮运行库155.0.4283.62、屏幕外NOACTIVATE窗口、隔离UDF和合成页面，在真实Taskmgr完成4次/48份原生采样：

| 外部主程序启动方式 | 宿主 → 浏览器PID | 两边包身份 | UIA逐PID及共同容器 |
|---|---|---|---|
| 策略0x02，中文目录A | 27600 → 14536 | 0，相同身份 | 8/8：主程序+6个WebView2+启动器21888，同一组 |
| 策略0x02，空格目录B | 30544 → 4332 | 0，相同身份 | 8/8，同一组；启动器25724 |
| 同一启动器但普通创建，目录A | 32996 → 8060 | 15700，无包 | 8/8均找到，但无共同组，浏览器仍独立 |
| 直接运行同字节主程序，目录A | 9884 → 34656 | 15700，无包 | 7/7均找到，无共同组 |

两个正向的界面均显示“LoreKeeper Process Probe，9实例”；核验的是8个自有PID及其相同TreeItem祖先，额外系统成员不计入自有统计。浏览器直接以真正主程序为父；所有成员可见，名称本身不当作归组证据。没有向外部主程序嵌入未签名OID稀疏身份清单，绕开了第113次未取得运行结果的SxS入口。

### 后台与收尾的精确记录

- 两次先行视觉尝试因前后台句柄变化而安全中止，未完成正向矩阵，不计入4次/48份有效结果；不能笼统写成每次句柄都不变。第一轮自开Taskmgr33880的关闭请求已在第二轮收尾发出，该PID最终不存在。
- 有效矩阵改成只读UIA：不选择页面、不调用排序或展开动作；保持既有CPU排序，不为了名称升序触发控件操作。启动阶段每50毫秒观察、检查器每20毫秒观察任务管理器焦点归属，累计前台命中0；探针自身前台采样也均0。
- 目录B的UIA读取前后句柄仍发生变化，但期间任务管理器前台采样0、探针前台采样0；如实保留该测量，不把用户在其他窗口切换误写成“句柄不变”，也不把采样当作逐指令绝对保证。
- 有效轮复用了已存在的Taskmgr23564，未取得其所有权，故未关闭；没有擅自结束其他窗口。读取器/注册监督器退出0，排序保持原样。唯一测试包finally移除全用户剩余0，普通权限独立复核当前用户包0、自有探针/WebView2遗留0；未强杀系统Broker。
- 本轮未新增证书、信任、工具、服务、计划任务或用户账户。此前临时证书已在第114次清理，本轮不复用其权限或私钥。
- 证据：`.pi/tmp/process-group-probe/placement/native/2026-10-18T23-47-56-104Z/`、`placement/runs/2026-10-18T23-58-09-014Z/`的summary/verification/逐PID UIA与原生记录；控制器/启动器源码与唯一包控制收据同目录留存，不入库。

### 首个构建契约切片与边界

新增 `windows-identity.json`、`windows-identity.mjs` 与9项Node测试：默认unsigned、签名字段null且不读证书，signed尚未实现则明确失败；固定identifier、数字包版本、外部主程序布局、MSI机器级优先和未验收项。只生成完整身份小包清单，未接入产品启动器或现有打包/发布入口。

Node身份9+安装6+发布19共34通过，含真实NSIS语法编译；typecheck/lint退出0。候选清单通过既有MakeAppx，仅使用实验占位启动器做语法检查，没有注册该产品名小包或当作可安装产品。候选仅runFullTrust，少于探针清单的额外unvirtualizedResources，仍须重验最小能力、文件/注册表虚拟化、数据根和外部工具兼容。

真实LoreKeeper、MSI机器级stage/provision/各用户注册与升级卸载/回滚、直接主EXE入口、外部CLI/MCP的脱离包环境策略及macOS均未验收。以上证明本机**未签名且主程序真实位于所选外部目录**可以归组，不等于安装器已经支持所有用户。接入步骤见[更新后的计划](../plans/windows-sparse-identity-integration.md)，发布仍暂停。

## 10. 第116次：产品启动器与全机安装记录接线（构建验证）

用户要求继续落地真实启动器与 MSI 全机/多用户。本轮把最简验证转成产品代码并真实构建两个安装包，但**没有执行安装/卸载，也没有注册身份包**。

### 新增产品代码

- 独立 crate `tauri-app/src-tauri/identity-launcher/`（仅用官方 `windows-sys`，不进入主 crate 依赖图，`cargo test` 10/10）：
  - **包内模式**：`GetCurrentPackageFullName` 成功且包名前缀为本产品时，读取安装记录 → 校验路径（绝对、无 `..`、无引号/控制字符、主程序必须是 `lorekeeper.exe` 且父目录等于安装目录）→ 拒绝链接与缺失文件 → 以 `PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY`(0x20012)=`DISABLE_PROCESS_TREE`(2) 启动真实主程序，等待并透传退出码。
  - **包外模式**：绝不无身份启动主程序；只从 `HKCU\...\AppModel\Repository\Packages` 推导包族名，用 `explorer.exe shell:AppsFolder\<AUMID>` 激活身份包。
  - 不接受任何命令行参数；错误以中文 MessageBox 呈现并返回非 0（2–7）。
- **安装记录契约**（schema 1）：`IdentityRecordSchema`、`IdentityPackageName`、`InstallScope`、`InstallLocation`；`MainExecutable` 可省略，缺省按固定文件名补全，避免两条安装链的目录属性拼接差异。读取候选为 HKLM/HKCU 各含 WOW6432Node 视图，命中的声明范围必须与位置一致。
- **安装器接线**：NSIS `NSIS_HOOK_POSTINSTALL` 写当前用户记录；MSI 新增 WiX 片段 `installer/lorekeeper-install-record.wxs`（`DirectoryRef Id="INSTALLDIR"`、`Root="HKLM"`、`Action="createAndRemoveOnUninstall"`、`Win64="yes"`），经 `wix.fragmentPaths`/`componentRefs` 接入。键名与启动器一致：`Software\tancan1998\LoreKeeper`（与模板 `Manufacturer=tancan1998` 对齐）。

### 本机构建验证（只构建，不安装）

| 项 | 结果 |
|---|---|
| 启动器单测 | 10/10 通过；覆盖记录校验、持久化范围一致性、AUMID 推导、引号/非法字符 |
| `pnpm tauri build --bundles msi --bundles nsis` | EXIT=0；产出 `LoreKeeper_1.0.0-beta.1_x64_zh-CN.msi`（53,186,560 B）与 `LoreKeeper_1.0.0-beta.1_x64-setup.exe`（49,738,678 B）；WiX 3.14.1 由 CLI 自动下载 |
| 生成模板 `main.wxs` | 含 `<ComponentRef Id="LoreKeeperInstallRecord"/>` |
| MSI `Registry` 表（WindowsInstaller 只读查询） | 4 条 HKLM 记录：`IdentityRecordSchema=#1`、`IdentityPackageName=com.tancan1998.lorekeeper`、`InstallScope=machine`、`InstallLocation=[INSTALLDIR]` |
| 生成 `installer.nsi` | `hooks.nsh` 已 include；`NSIS_HOOK_POSTINSTALL` 在安装段插入（第 706 行） |
| Node 合同 | 安装 7 + 身份 9 + 发布 19 全通过（含真实 `makensis` 语法编译） |

未安装、未写注册表、未注册身份包、未改快捷方式、未触碰共享 WebView2 或证书。

### 仍未完成（发布门禁不变）

1. **身份包产物链**：`windows-identity.mjs` 仍只生成清单；启动器二进制既未进入 MSIX 内容，也未作为资源随 NSIS/MSI 安装——即使安装包已就绪，产品还不会带身份启动。
2. **安装时注册/置备**：两个安装器都未注册 MSIX 身份；MSI 全机 stage/provision 与每用户注册、SYSTEM 上下文语义未实现。
3. **激活入口**：快捷方式仍指向安装目录 exe；直接双击主程序不带身份，包外启动器尚未被任何入口使用。
4. 真实安装/升级/修复/回滚/卸载、多用户、最低系统版本、外部工具 breakaway、macOS 均未验证。
5. 因此“任务管理器真实归组”仍只在最简宿主与本机机制层面成立，产品未验收，发布继续暂停。

## 11. 第117次：身份包产物、资源安装与真实安装/卸载回归

继续落地“全部”：身份包真实产出、随安装器分发、真实静默安装与卸载都执行了，并查出一条决定性的系统限制。

### 已真实执行
- `node scripts/build-windows-identity.mjs` 构建独立启动器（release，262,144 B）并打包真实 MSIX（134,256 B）：内容只有启动器与图标，清单 `AppListEntry="none"`，无主程序、无用户路径。构建接入 `beforeBuildCommand`，资源经 `tauri.windows.conf.json` 映射为 `identity\LoreKeeper.msix` 与 `lorekeeper-activate.exe`。
- 真实 MSI 与 NSIS 均重新构建成功；生成的 `installer.nsi` 含两条 `File` 与卸载 `Delete`。
- **真实静默安装**（`/S /NS /D=...` 到临时目录）：退出码 0，落盘 `lorekeeper.exe`（79,312,896 B）、`lorekeeper-activate.exe`、`identity\LoreKeeper.msix`；`HKCU\Software\tancan1998\LoreKeeper` 记录为 `schema=1 / scope=user / 正确安装目录与主程序`。
- **真实静默卸载**：安装目录、注册表键、身份包全部清零（`installDir=false, package=0, recordExists=false, WindowsApps=0`）。

### 决定性限制：未签名身份包注册必须提权

| 场景 | 结果 |
|---|---|
| 普通用户 `Add-AppxPackage -AllowUnsigned` | 失败 `0x80073D2B`：“未签名的程序包不能包含可执行的激活” |
| 加 `AppListEntry="none"` 后普通用户 | 仍为同一失败 |
| 提权后同一 MSIX | **注册成功**，`InstallLocation` 内启动器存在；测试后立即 `Remove-AppxPackage`，剩余 0 |

因此“不签名”这条路线在**当前用户安装**下无法自动登记身份：Tauri 的 NSIS `currentUser` 安装不请求管理员。安装器现已在注册失败时明确告警（静默安装不弹窗但保持非假成功），不再假装归组可用。真正可行的组合是 MSI 全机（安装本就提权）或显式提权的安装流程。

### 相应调整
- 未签名包不能有可执行激活项，`shell:AppsFolder` 激活方式作废；包外启动器改为读取安装器登记的 `IdentityLauncherPath`（必须在 `%ProgramFiles%\WindowsApps\com.tancan1998.lorekeeper_*` 内且文件名固定）后启动包内启动器，绝不无身份启动主程序。
- 契约新增 `registration: { requiresElevation: true, appListEntry: 'none', lastError: '0x80073D2B' }` 与 `elevated-unsigned-registration-flow` 验收项。

### CI 失败与修复（同一轮）

`Tauri Rust CI`（39184958870，提交 `4c66847c`）在编译阶段失败：`resource path target\identity\LoreKeeper.msix doesn't exist`。原因是主 crate 已把身份小包声明为安装资源，而 CI 只跑 `cargo test`。修复：`build-windows-identity.mjs` 改为按版本倒序探测本机 Windows SDK 的 `makeappx.exe`（不下载工具），CI 在测试前新增“构建身份小包”步骤（PATH 加入隔离 `CARGO_HOME\bin` 并固定 `RUSTUP_TOOLCHAIN`），`build.rs` 在缺包时给出可直接执行的修复提示。

### 仍未完成
1. MSI 侧的注册/置备自定义动作（提权上下文、安装用户 vs SYSTEM、`Add-AppxProvisionedPackage` 多用户语义）尚未实现与实测。
2. 真实产品 GUI 启动后的“任务管理器真实归组”未验证：本轮用户级注册被系统拒绝，必须先在提权路径成功后才能测。
3. 升级/修复/回滚、多用户、最低系统版本、外部工具 breakaway、macOS 仍未验证。
4. 发布继续暂停。

## 12. 第118次：真实产品归组与提权注册闭环

用户要求不再分片，故本轮把提权注册、真实安装、真实启动核验、卸载清理全部跑完。

### 已实测（真实产品、非探针）
- **提权静默安装**（`/S /NS /D=自选目录`）退出 0：身份包注册成功（`com.tancan1998.lorekeeper_1.0.1.0_x64__e8hwe6njnc7fy`，`Status=0`），记录含 `IdentityLauncherPath=C:\Program Files\WindowsApps\...\lorekeeper-identity-launcher.exe` 且文件存在。
- **真实产品启动核验**：经激活器启动后，主程序 PID 28944 位于用户所选目录 `...\install-test\lorekeeper.exe`，其包身份为上述包全名；**6 个 WebView2 子进程读回同一包身份**（去重后仅一个值）。这是真实 LoreKeeper 二进制，不是探针。
- **卸载清理**：提权静默卸载退出 0；安装目录、`HKCU\Software\tancan1998\LoreKeeper`、当前用户身份包、`WindowsApps` 内包目录全部为 0/false。测试实例先自行关闭（`CloseMainWindow` 被未保存内容确认拦截，最终只结束本次测试进程）。
- 关闭测试窗口后机器仍有 24 个其他应用的 WebView2 进程，属系统其他程序，未触碰。

### 提权注册实现
- 启动器新增唯一开关 `--register-identity`：非提权时明确报错（不自作提权、不假成功）；已提权时执行 `Add-AppxProvisionedPackage -Online -SkipLicense`（全机置备，供多用户）→ `Add-AppxPackage -AllowUnsigned`（当前用户注册）→ 取 `InstallLocation` 并把 `IdentityLauncherPath` 写回 `HKCU` 记录。
- NSIS `POSTINSTALL`：先尝试直接注册（安装器已提权时立即成功并写路径）；失败且非静默时用 `ExecShell "runas"` 调起提权助手；静默安装不弹 UAC，也不写假路径。

### 仍未完成
1. MSI 侧尚无调用同一助手的自定义动作（安装记录已写 HKLM，但注册要另接）。
2. 未在第二个 Windows 用户下验证“置备后新用户首次使用”；SYSTEM 上下文注册仍不等于全用户覆盖。
3. 升级/修复/回滚、最低系统版本、外部工具 breakaway、macOS 未验证。
4. 本轮实测使用提权安装；普通用户双击安装时的 UAC 提交流程未人工确认（静默不适用）。
