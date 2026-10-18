# LoreKeeper任务交接快照（2026-10-18）

> [当前入口](./CURRENT.md) · 第 **108** 次：图标已定稿，其余任务未完成。核对时间2026-10-18 23:45 +08:00。按同日快照规则更新摘要；此前第103次正文保留在Git历史 `48d2a7ab`，第104–107次专题快照不改写。

## 已验证状态

交接前 `master` 与 `origin/master` 同为 **48d2a7ab425d694877b233a7e537317ec31cfe82**，工作区干净。此前只有本机忽略目录与D盘设计文件变化，**没有产品代码修改、图标接入、上游fetch/合并、新打包或新版发布**。本次提交只更新交接文档。

| 项目 | 本轮结果 | 版本/证据 |
|---|---|---|
| 原创图标 | 用户明确选择单颗彩色四芒星，**默认亮色，保留亮暗两套** | 下文源文件与本机审批备份 |
| SVG/PNG | Tauri CLI渲染成功，含1024/256/64/32/16像素；SVG星芒mask仅1条轮廓 | `.pi/tmp/branding-options/si-single-*.log`、定稿备份 |
| 边距 | 星芒28×28，口部中心106.5/163；上下各10，左右各27.5（256单位） | `geometry.json`，生成时断言通过 |
| GUI/安装卸载/进程归组 | 本轮均未验收，不能用设计预览冒充 | 无新认证 |
| TS/Rust/浏览器测试 | 没有产品变更，本轮未重跑 | 最近Rust全量仍为a5418614/run39125354221：1680/0及all-targets成功，见[第107次](./2026-10-18-actions-storage.md)，非本轮新测试 |
| 上游 | 本地跟踪引用仍3a5ba4521a1225ff7892954e4344efe4c9a84c02 | 用户最新说又有提交；**尚未核查远端最新SHA/差异/冲突** |

## 已定稿图标与采用范围

用户迭代后采用：古韵与现代结合的手绘“司”字标，顶横与中横收锋；仅一颗彩色四芒星置于口部中央，颜色参考Gemini当前四色渐变，字与背景仍黑白灰。不得退回三颗星、机械粗黑体或其他弃用候选，也不再反复询问是否采用。

- 本地展示：`D:/Temp/LoreKeeper-icon-options/司字-单星版/图标对比.png`。
- 源目录同上，有`司-亮色版.svg`、`司-暗色版.svg`、无底字标与独立星芒。
- 项目内稳定备份：`.pi/tmp/recovery/branding-selected-single-star/`，已确认被Git忽略；含五份SVG、几何JSON、预览PNG、`SHA256SUMS.txt`及`approval.json`。
- 亮色SVG SHA256：`52ba2b020e139e9dfda4a272945bb975bc8290907802eb04377d2735c200877b`。
- 暗色SVG SHA256：`ef45ace7cb2a8bc4f5c3ea74ad1a49221ea5684eb32ae33733c6636d86a821b3`。
- 用户原要求候选仅本地看，选定后才上传；现仅**选中这一版**允许作为正式资源入库。交接提交不上传图标、旧草稿、用户参考图、Gemini官方图或临时生成目录。
- 没有安装系统字体、引入新依赖或开启窗口。当前定稿为手绘SVG字标，不冒称直接使用思源原字形，也不复制第三方字样/印记。

## 当前任务、确认与边界

1. **上游新增核查**：用户再次要求看看是否能合并，尚未执行。先 `git fetch upstream --no-tags`，核对差异与冲突；不重新引入已清理的旧tag。不把“看看是否能合并”当作不受限的依赖/schema/CI/权限变更批准。
2. **应用所属子进程归组**：用户要求Windows任务管理器将本应用WebView2进程归于LoreKeeper，macOS WebKit同理；不是隐藏/改名全系统Web进程。已找到[WebView2Feedback#5628](https://github.com/MicrosoftEdge/WebView2Feedback/issues/5628)与[Tauri#15567](https://github.com/tauri-apps/tauri/issues/15567)，维护者认可同类归组异常，Teams可正常归组；AppUserModelID不保证解决任务管理器归组。尚无已验证修复，macOS尚未有效核查。不能简单断言系统不可能，也不能把改图标/名称冒充归组修复。原始资料在`.pi/tmp/process-group-research/`。
3. **中文安装/卸载**：用户已确认Windows EXE中文向导默认保留项目，只有显式选目录并二次确认才删除；MSI静默/macOS拖拽默认保留，另提供中文清理入口。注册表只清本应用创建的键，不扫盘、不碰Vela/其他项目/系统目录/共享WebView2。尚未实现；本确认不等于允许现在运行真实数据删除或前台安装卸载测试。
4. **重新发布**：新版 **1.0.0-beta.1**，旧版标签和资产不覆盖。用户明确选择：保留beta版本名，GitHub属性使用普通Release（`prerelease=false`）并标Latest，正文醒目说明仍是预览版，不宣称正式业务/安装认证。GitHub不允许Pre-release同时Latest；须提供明确最新版选项并拒绝不支持的组合。现有脚本禁止beta关闭prerelease，需按本次具体确认调整，不能直接沿用旧门禁。
5. 主会话继续直接执行，禁止子代理；不抢焦点/不打开前台窗口。真实模型/MCP/业务云、签名公证与新包真机安装仍未授权。上述已有确认不放宽AGENTS中的其他先确认边界。

## 接续断点与下一步

1. 读AGENTS、本快照、`.pi/tmp/pipeline-state.md`；完整设计迭代与确认在`.pi/tmp/branding-install-state.md`。先处理新增上游核查，给出可合并范围，不盲合未知改动。
2. 核验定稿备份摘要后接入默认亮色图标及暗色配套资源；统一LoreKeeper展示身份但保留identifier/data roots。当前配置仍为`Lorekeeper`、1.0.0-beta，Cargo描述仍有上游旧品牌，尚未修改。
   - 改productName大小写前先取得并固定**旧WiX UpgradeCode**，避免改变既有升级身份。上次Node `spawnSync('pnpm.cmd')`返回status=null，没有成功取得GUID，输出空文件不能当结果；改由PowerShell已安装pnpm shim执行`pnpm tauri inspect wix-upgrade-code`并检查退出码，不安装新工具。
   - Tauri build会生成ProductName/FileDescription，具体fallback需继续查本机tauri-build源码；不能把Cargo描述调整当作进程归组验证。
   - 打包清单与发布辅助脚本硬编码Lorekeeper文件名及MSI en-US，品牌大小写/中文语言变化时必须同步约束和测试。
3. 独立处理归组可行性、中文安装卸载与安全清理，然后版本/发布工作流与验证。任何未验收能力应明确待验证，不以包构建成功替代。

## 发布与证据保全

- 已公开的[1.0.0-beta预发布](https://github.com/TanCan1998/Lorekeeper/releases/tag/lorekeeper-v1.0.0-beta)仍是旧版，tag指3bce04455054de1860292d970f96425a1fdbe2b7；本轮未移动tag或更改九资产。
- Actions制品/新日志原生保留1天；重要证据及时下载，过期应重跑相应资格，不能重启旧制品清理脚本。现存本机完整证据与限制见[第107次](./2026-10-18-actions-storage.md)。
- 本轮仅文档收口，不重复全量测试；最终文档提交与推送状态由本机短状态记录。没有在飞子代理、GUI、Vite、新CI或下载任务登记；本机备份不入库。
