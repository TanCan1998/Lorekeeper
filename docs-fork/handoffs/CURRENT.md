# 当前交接入口

- 最新：[第109次：上游同步与定稿图标接入](./2026-10-19-tauri-migration-status.md)。**不是迁移/发布完成报告**。
- 上游已合入至 `d21207c8`（19个新增提交），合并提交 `460fcd77` 已推送且两项CI成功。再次fetch确认中英文README均为上游最新；中文版仅保留顶部fork说明，英文版与上游一致。
- 单颗彩色四芒星“司”字图标已接入：默认亮色、保留暗色，启动页/顶栏/关于页及七份桌面资源。代码 `e31c42d1`；17项单测、20项无头浏览器通过，typecheck/lint/build成功。详见[图标登记](../design/branding.md)。原生包图标仍待打包验收。
- 未完成：WebView2/WebKit子进程归组、中文安装卸载与安全项目清理、1.0.0-beta.1重新发布；不能用改名或图标冒充归组解决。
- 卸载默认保留项目，删除需显式目录与二次确认；只清本应用注册键。新版保留beta名，普通Release（prerelease=false）+Latest，正文明确预览，旧版不覆盖。
- 名称/版本/安装配置本轮未改。旧WiX UpgradeCode已取得，后续改productName大小写须保持升级身份。完整授权历史见本机 `.pi/tmp/branding-install-state.md`，活动状态见 `.pi/tmp/pipeline-state.md`。
- 禁止子代理、不抢焦点；真实模型/MCP/业务云及新包真机安装仍未认证。Rust本轮未重跑，最近全量仍a5418614/run39125354221/1680项通过，不能记作新代码验收。
- 公开仍[旧1.0.0-beta预发布](https://github.com/TanCan1998/Lorekeeper/releases/tag/lorekeeper-v1.0.0-beta)，beta.1未发布。Actions证据保留1天，不重跑旧清理脚本。
