# LoreKeeper迁移进度快照（2026-10-19）

> [当前入口](./CURRENT.md) · 第 **116** 次：产品身份启动器与全机安装记录已实现并真实构建 MSI/NSIS（仅构建，未安装）。身份包注册、激活入口与多用户置备未完成，发布暂停。**不是LoreKeeper产品修复或新版发布完成报告。**

## 已验证状态（Rust/前端功能基线沿用第110次，本轮构建检查另列）

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
| 真正包身份与真实归组 | 同一EXE五次无flag正反复验：无包前后拆组，带包三次均同组，正向7/7自有PID | 第113次，[研究§7](../research/webview-process-attribution.md#7-第113次真实包身份取得正向结果) |
| 临时Appx注册/移除 | **用户明确同意后执行**，同名包剩余0 | 唯一测试身份；第113次未改证书，第114次另获临时证书/TrustedPeople授权 |
| 签名稀疏身份原路径归组 | 同一EXE/同一路径五次、60快照，注册后三次7/7同组，撤销后7/7拆组 | 本轮新增，[研究§8](../research/webview-process-attribution.md#8-第114次签名稀疏身份包原路径正向验证)；首帧前置祖先不完整如实保留 |
| 临时证书/全机信任清理 | 包/全机TrustedPeople/个人证书均0、私钥不存在，独立复核通过 | 单独明确授权；未碰Root/既有证书，无私钥导出 |
| 产品安装/卸载/项目删除/注册表清理 | **未执行** | 临时测试部署不能替代原生产品生命周期验收 |
| 新版打包与发布 | **未触发** | beta.1 Release API返回404；没有创建新标签或改Latest |

## 本次更新：第116次产品启动器与安装记录

- 新增独立 crate `identity-launcher/`（仅官方 `windows-sys`，不进主 crate 依赖图）。包内模式：`GetCurrentPackageFullName` 通过 → 读记录 → 校验路径（绝对/无上级片段/无引号控制字符/文件名固定/父目录等于安装目录/拒绝链接与缺失）→ 以 `PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY`(0x20012)=2 启动主程序并透传退出码。包外模式：绝不无身份启动，只按仓库包全名推导包族名并 `explorer.exe shell:AppsFolder\<AUMID>` 激活。无参数，错误中文 MessageBox + 非 0 退出码。
- 安装记录 schema 1（`IdentityRecordSchema`/`IdentityPackageName`/`InstallScope`/`InstallLocation`，`MainExecutable` 可省按固定名补全）；读取候选 HKLM/HKCU 各含 WOW6432Node，声明范围必须与命中位置一致。NSIS `POSTINSTALL` 写当前用户；MSI 片段写 HKLM 并随卸载移除，键名 `Software\tancan1998\LoreKeeper`（模板 `Manufacturer`）。
- 本机验证：启动器 10/10；`pnpm tauri build --bundles msi --bundles nsis` EXIT=0（MSI 53,186,560 B / NSIS 49,738,678 B，WiX 3.14.1 自动下载）；`main.wxs` 含 `ComponentRef LoreKeeperInstallRecord`；MSI `Registry` 表只读查询到 4 条 HKLM 记录；`installer.nsi` 第 706 行插入 `NSIS_HOOK_POSTINSTALL`；Node 18 项合同通过（另发布 19 项）。
- 明确未做：没有安装、没有写注册表、没有注册身份包、没有改快捷方式。启动器尚未进入 MSIX 内容或作为安装资源；两个安装器均未注册/置备身份；快捷方式仍指向安装目录 exe；真实安装/升级/回滚/卸载、多用户、最低系统、工具 breakaway、macOS 未验证。
- 主 crate 与前端未改动（Rust 语义切片累计仍 0，云端全量基线仍 44259fa3/39148045359）。发布继续暂停，无新 tag/Release/Latest。
- 详见[研究§10](../research/webview-process-attribution.md#10-第116次产品启动器与全机安装记录接线构建验证)与[接入计划](../plans/windows-sparse-identity-integration.md)。

## 前轮更新：第115次未签名与自选目录

- 用户明确不要签名、要真实归组/主程序自选目录，接受小型完整MSIX身份启动器+外部主程序逻辑映射；MSI全机/多用户优先。不是WindowsApps联接或VFS伪装。
- 先完成消息窗口身份对照；普通CreateProcess不传递身份，公开桌面进程策略0x02则使外部主程序和WebView2读回同包身份。随后4次/48采样真实Taskmgr对照：两个目录正向各8/8自有PID共同应用组，普通创建/直接启动两个负向仍拆组。同字节主程序，无浏览器flag，运行库155.0.4283.62。
- 有效视觉轮只读UIA，不排序/展开；任务管理器和探针前台命中采样均0。B轮其他前台句柄变化如实保留，不宣称所有轮句柄不变。两次先行中止不计入完成矩阵。
- 临时唯一身份finally全用户剩余0，独立只读复核当前用户包0、自有进程0。先行自开Taskmgr33880已结束；有效轮的既有Taskmgr23564无所有权，保持不关闭，不写成“所有Taskmgr已清理”。
- 新增构建契约 `windows-identity.json`、`windows-identity.mjs` 与Node9项；预留证书文件/指纹、Publisher、时间戳但默认null，不自动读取，signed模式未接入则报错。小包清单只放启动器，固定identifier/数字版本，MSI机器级优先，releaseEnabled=false且未接线发布。
- 本轮身份9+安装6+发布19共34测试通过，typecheck/lint退出0，真实MakeAppx候选清单语法成功（实验占位二进制、未注册）。新清单只runFullTrust，探针额外能力差异和虚拟化仍待产品验证。未改Rust、前端功能、依赖、build.rs、tauri.conf、CI/发布配置或现有安装钩子，Rust切片累计仍0；1689全量仅沿用44259fa3。
- 下一步：原生启动器/安装记录、合成外部工具脱离包环境、真实LoreKeeper后台验证，再做MSI stage/provision/每用户注册及升级卸载回滚。macOS/原生权限和生命周期仍未验收；继续暂停发布。
- 详见[研究§9](../research/webview-process-attribution.md#9-第115次未签名小启动器与外部自选目录)和[接入计划](../plans/windows-sparse-identity-integration.md)。本轮无新证书、信任或开发模式修改，未执行真实产品安装卸载。

## 前轮更新：第114次签名稀疏身份原路径归组

- 用户再次明确“同意”生成一天有效、仅代码签名、私钥不可导出的唯一证书，并临时信任其公钥至LocalMachine/TrustedPeople（非Root），签署唯一测试身份包并精确清理。该范围不扩为生产证书或发布授权。
- 既有SignTool签名Valid；无PFX/私钥导出，无新工具、开发模式或共享运行库变更。普通CN签名稀疏包成功注册，EXE内嵌清单不再触发前轮未签名OID发布者SxS错误。
- 同一EXE、同一路径五次/60快照、无诊断flag：注册后三次正向均逐PID匹配7/7并共享应用容器（宿主+全部6个WebView2），撤销后7/7确认独立后台组。前置首帧浏览器祖先列表为空，只记录无包API状态，不当作完整视觉负向。
- 五次均正常退出、焦点采样0、相关UIA前后台不变；最终包/临时全机信任/个人证书为0，私钥不存在，独立只读复核通过。Taskmgr恢复排序并关闭，自有宿主/WebView2遗留0。
- [接入草案](../plans/windows-sparse-identity-integration.md)建议保留NSIS/MSI、identifier/数据根/UpgradeCode，配套正式签名身份与精确生命周期；仅草案，未改产品/依赖/CI/发布。正式可信签名来源与接入范围仍须确认，真实LoreKeeper/安装器与macOS未验收，发布暂停。
- 证据索引见[研究§8](../research/webview-process-attribution.md#8-第114次签名稀疏身份包原路径正向验证)。

## 前轮更新：第113次真实包身份正向归组

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
