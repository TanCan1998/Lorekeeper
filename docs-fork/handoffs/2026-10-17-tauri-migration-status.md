# Tauri迁移进度快照（2026-10-17）

> [当前入口](./CURRENT.md) · [开发指南](../agents/pi-development.md) · [混合验证说明](../agents/tauri-rust-ci.md) · [首版决策D7](../plans/2026-10-17-tauri-completion-decisions.md)。动态进度仅在docs-fork展示。

## 已验证状态（第80次 —— 独立Windows Tauri CI与混合验收）

- 接续master，HEAD/本地origin/master=`0ead5794`，原工作区干净；无子代理/登记在飞工作。
- 用户明确授权“本机定向 + GitHub全量”混合模式及独立CI配置，要求使用最新Action。
- 新增工作流及脚本，不修改Rust业务源码/依赖/权限或Electron CI；本切片不增加Rust语义计数。

| 指标 | 状态 | 范围 | 证据 |
|---|---|---|---|
| CI脚本离线合同 | **8通过** | 成功、两个独立失败/双失败、SHA不匹配、脏工作区、Git失败、非法SHA | .pi/tmp/recovery/ci-contract.log |
| 工作流静态检查 | YAML解析/合同、全部run块语法、git diff --check通过 | 新工作流/PowerShell脚本 | 本机工具输出 |
| 本机最近审稿定向 | **74通过/0失败**，非本轮重跑 | 0ead5794代码 | .pi/tmp/recovery/history-review-final.log |
| 本机最近Rust全量 | **1611通过/0失败**，库64.76秒；非本轮重跑 | 0ead5794代码 | .pi/tmp/recovery/history-full.log |
| 本机最近all-targets | 无告警，12.44秒；非本轮重跑 | 0ead5794代码 | .pi/tmp/recovery/history-check.log |
| GitHub首次全量 | 配置落地后由push触发；**结果以对应SHA运行记录为准** | 本轮CI提交 | [工作流](https://github.com/TanCan1998/Lorekeeper/actions/workflows/tauri-rust-ci.yml)，run/SHA/状态见pipeline |
| GUI/真实模型/MCP/业务云 | 本轮未执行/未验收 | GUI仍本机；真实业务外部服务未授权 | 无 |

## 本次实现与执行规则

- `.github/workflows/tauri-rust-ci.yml`：Windows 2022、固定Rust1.99.0；Rust/CI文件push、面向master的PR及手动expected_sha入口。独立于Electron验证。
- `tauri-app/scripts/test-rust-ci.ps1`：检出SHA/干净工作区校验；分别执行cargo test --locked与cargo check --locked --all-targets；即使测试失败仍记录check结果，后者不覆盖前者，异常路径也保存失败证据。
- 仅contents:read，checkout不保留凭据，无业务Secrets、发布或打包步骤。缓存公开依赖/编译产物，上传仅3个显式日志/元数据文件、保留14天；不上传数据库/用户目录/可执行程序。
- `test-rust-ci.contract.ps1`：8项合成git/rustup替身测试，无网络/真实Rust编译。两个脚本显式要求PowerShell7，已用本机pwsh7.6.7按CI调用方式验证；不使用PS5默认ANSI解码运行UTF8中文脚本。
- 官方latest release/tag实时核对：checkout v7.0.1、cache v6.1.0、upload-artifact v7.0.2，均使用完整提交SHA，详见混合验证说明。Context7额度不足，改核实官方release/tag与action.yml。
- 本机定向/格式/diff继续执行；全量触发条件不变，改由GitHub执行。允许为CI先提交推送，只有对应SHA云端成功后才登记收口/累计清零，取消/跳过/失败均不计通过。
- GUI、原生文件授权与恢复生命周期仍本机；真实模型、外部MCP及业务云访问仍未授权，不开放审稿四生产频道。
- 已更新开发指南/决策D7与本机AGENTS（后者保持忽略、不入库）。未改根README、根配置/锁文件、Electron、上游docs、schema/依赖/ADR或发布配置。

## 全量基线与下一步

- 本机全量基线0ead5794，语义切片累计0；本次CI/文档不增加计数。首次云端通过后可登记对应SHA新云端基线，尚未通过不得冒充。
1. 核对首次GitHub运行与日志证据，修复CI接线问题（如有）；结果记录到pipeline，不因仅配置完成宣称云端通过。
2. 继续旧库入口明确不可用UI/blocked边界，不读取旧库、不创建副本、不消耗授权，不计作真实导入完成。
3. Windows本机合成GUI/归档恢复生命周期与前端合同验收、剩余证明审计明确清单。
4. 四审稿频道继续关闭；生产开放仍须完整链验收及以后另行授权的真实模型验收。
