# LoreKeeper迁移进度快照（2026-10-19）

> [当前入口](./CURRENT.md) · 第 **113** 次：完整MSIX最简宿主取得真实归组正向结果，同一EXE无诊断flag正反复验成立。产品尚未接入，用户拒绝带缺陷发布。**不是LoreKeeper产品修复或新版发布完成报告。**

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
| 最简原生WebView2探针 | 18次完整启动、108份采样；三次名称排序UIA精确PID核验均复现分列 | 第112次，运行库155.0.4283.62；[研究§6](../research/webview-process-attribution.md#6-第112次最简程序实测) |
| 真正包身份与真实归组 | 同一EXE五次无flag正反复验：无包前后拆组，带包三次均同组，正向7/7自有PID | 本轮新增，[研究§7](../research/webview-process-attribution.md#7-第113次真实包身份取得正向结果) |
| 临时Appx注册/移除 | **用户明确同意后执行**，逐轮全用户同名包剩余0 | 唯一测试身份，未改证书/开发模式/现有产品；原始收据忽略不入库 |
| 产品安装/卸载/项目删除/注册表清理 | **未执行** | 临时测试部署不能替代原生产品生命周期验收 |
| 新版打包与发布 | **未触发** | beta.1 Release API返回404；没有创建新标签或改Latest |

## 本次更新：真实包身份正向归组

- 无包身份四组AUMID/版本信息对照均无效；窗口属性已确实写入、读回、清空，版本描述变化也未改变分组。
- 用户单独选择“同意”唯一临时包注册与精确移除，知晓未签名包可能需管理员/涉及全用户；不包括证书、开发模式或正式产品安装。已有SDK打包，无新软件。
- 无签名外部位置包注册成功，但自有探针被SxS发布者校验14001拒绝，未取得带身份运行结果；不能据此宣布签名稀疏包不可行。
- 普通完整MSIX最简宿主无需任何浏览器诊断flag即正确归组。再用SHA256完全相同EXE做“普通目录→包目录默认→包目录另设AUMID→包目录默认→普通目录”五次验证：无包前后均拆组，带包三次均为宿主+6个WebView2同一应用容器；三个正向均匹配7/7自有原生PID，另有Windows Runtime Broker合计8实例。两边API读回同一包/应用ID，非仅改名。
- 本轮三套完整矩阵12次、153份原生采样，均正常退出、未发现探针占用前台，相关UIA操作前后台不变；失败准备和中断不计成功。此前第112次18次/108份是另轮历史。
- 临时身份逐轮全用户剩余0，最终当前用户同名包0、自有宿主/WebView2遗留0；Taskmgr恢复原排序并关闭，管理员监督器正常退出。未新增证书、开开发模式、动共享WebView2或改产品。
- **产品仍未修复**：需确认完整MSIX或既有NSIS/MSI配套签名身份路线；后者尚未测试，任何临时签名证书/信任变更都须另行明确同意。正式签名/CI/发布配置不在本次授权中。macOS仍待真机，发布继续暂停。
- 方法、限定授权、同一EXE摘要、PID对照与原始证据索引见[研究§7](../research/webview-process-attribution.md#7-第113次真实包身份取得正向结果)。

## 前轮更新：第112次最简程序实测

- 系统C#编译器+既有WebView2Loader，无新依赖，不编译完整Tauri/Rust。每次独立UDF及内联合成HTML，不读真实项目或业务云。
- 消息窗口与普通隐藏HWND各三组、各重复两次，12次/36份快照；再做CPU排序三次及名称排序三次真实Taskmgr观察，每次12份快照。有效完整矩阵合计18次/108份采样；失败准备尝试不计成功。
- 默认运行时PerMonitorV2、DPI诊断开关、显式PerMonitorV2清单均为浏览器直接以宿主为父；名称排序精确PID及UIA祖先核验却均显示宿主在“应用”、其6个浏览器/helper进程单列“后台进程/WebView2管理器”。**父链正确不等于视觉归组，DPI两种改法在本机未奏效。**
- 用户明确允许打开任务管理器，并以“同意”限定授权管理员UIA检查器；探针仍普通权限、窗口可见但不激活且位于屏幕外。焦点采样未发现探针占用，相关UIA操作前后前台句柄不变。
- 所有有效探针正常退出；自有进程遗留0、管理员检查器停止/退出码0；恢复Taskmgr原CPU降序，正常关闭本次自行打开的窗口。没有包注册、证书/开发模式修改、真实安装卸载或项目删除。
- 包身份尚无正反对照，不能宣称必然解决；临时注册或证书/系统配置须另行确认。macOS未真机测试。最简无激活宿主的结果不外推为所有前台应用或具体运行库版本的根因。
- 完整方法、PID归属与证据索引见[研究§6](../research/webview-process-attribution.md#6-第112次最简程序实测)，本机原始证据在 `.pi/tmp/process-group-probe/`，不入库。

## 前轮更新：第111次归组机制调研

用户明确“不接受，继续搜索调研”，因此归组缺陷解决前保持发布暂停，不再询问是否带此限制先公开。新增[研究结论与验证矩阵](../research/webview-process-attribution.md)：

- 微软官方记录DPI兼容shim可能经shell启动浏览器，发现可证伪的父链假设；相关flag被明确限定为开发诊断，不能直接用于生产。
- 比对Tauri默认清单和Tao运行时DPI初始化，提出默认/诊断/显式manifest的A/B；该前轮未运行假设已由本轮最简实验验证，未见改善。
- 真包身份不等于AUMID；MSIX/外部位置包是待评估路线，不能未经确认改安装注册或假定能复刻Teams。
- macOS已追到WebKit XPC宿主标识、LaunchServices内容进程命名源码，以及Apple层级视图文档；命名、宿主关联和视觉归组分别验收，不能混为一谈。
- 前轮仅只读观察Windows26300、WebView2 155.0.4283.45及现有浏览器父链，当时无LoreKeeper实例，不能当本应用归组测试；前轮未启动新窗口。本轮新探针各组运行库均为155.0.4283.62，两版本不混作回归证据。前轮材料在 `.pi/tmp/process-group-research-2/`。
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

1. **产品进程归组未修复，但Windows已有正向实验**：第111次核查[Tauri #15567](https://github.com/tauri-apps/tauri/issues/15567)与[WebView2 #5628](https://github.com/MicrosoftEdge/WebView2Feedback/issues/5628)，均open且无解决方案。AppUserModelID、名称和图标不能冒充归组修复。macOS WebKit源码会传宿主名/bundle-id，但活动监视器显示未真机验证。研究材料 `.pi/tmp/installer-research/`。
2. **发布明确暂停**：用户最新明确拒绝归组未修复先公开，并要求继续研究。没有触发新版打包/发布；不得再把此前普通Release+Latest方案授权解释为接受缺陷交付。父链/DPI最简实验已完成且三组均未修复；真实完整MSIX包身份已正向验证，下一步需确认产品接入路线或另行授权签名稀疏包实验；不盲加DPI flag，也不再次请求缺陷豁免。
3. **原生验收待授权**：中文EXE/MSI、升级身份、默认保留、目录选择/二次确认、真实项目清理与注册表清理、macOS生命周期及图标均需原生验收。无法后台完成的原生对话框不得切前台。
4. 44259fa3对应云端全量已成功；完整活动run与最终HEAD由 `.pi/tmp/pipeline-state.md` 单写维护。

公开仍为[旧1.0.0-beta预发布](https://github.com/TanCan1998/Lorekeeper/releases/tag/lorekeeper-v1.0.0-beta)，旧标签与九资产不覆盖。禁止子代理、不抢焦点；真实模型/MCP/业务云、签名公证与新包真机安装尚未认证。具体历史授权见本机 `.pi/tmp/branding-install-state.md`。
