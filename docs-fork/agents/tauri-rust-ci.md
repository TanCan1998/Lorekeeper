# Tauri Rust 混合验证

2026-10-17用户确认：采用“本机定向 + GitHub Actions全量”的混合模式，授权独立Windows Tauri CI配置及合成云测试。仅此范围覆盖原本机全量执行位置限制，不授权真实模型、外部MCP或云业务测试，不授权发布。

## 分工与验收

- 本机仍执行受影响模块/直接调用方的定向测试、修改文件rustfmt及git diff检查；前端有改动仍按原规则typecheck、相关测试、lint。
- 云端工作流：`.github/workflows/tauri-rust-ci.yml`，Windows 2022，Rust 1.99.0；在`tauri-app/src-tauri`执行`cargo test --locked`及`cargo check --locked --all-targets`。
- GUI、原生文件授权、归档恢复生命周期仍在本机合成验收；云端单测通过不自动开放审稿生产频道。
- 累计5个Rust切片、领域阶段收口、安全例外及最终交付的全量触发条件不变。**为运行CI，允许先提交推送；对应提交的云端全量成功后才登记验收完成/全量累计清零。** 等待、取消、跳过、失败均不能算通过；安全修改本身仍须先确认，云端成功前不进入正式交付/生产开放。
- 普通Rust推送/PR会自动运行全量，这是CI执行策略；不要求本机重复全量，也不强制每轮普通切片等待云端结果，触发全量门槛的切片必须等待。
- 证据必须绑定被检出SHA，不能把上一提交、未提交工作区或PR头提交的结果冒充当前版本。PR默认测试GitHub合并提交，额外记录PR head SHA；合入后以目标分支实际SHA验证为准。
- 本机全量可作为CI不可用时的明确回退，但应记录原因/代码指纹，不能用失败云端结果冒充通过，不能静默放宽失败。

## 触发与查看

Rust目录、CI脚本或本工作流有变化时，push/面向master的PR自动触发；纯文档不会触发。并发新运行会取消同分支/PR旧运行；取消项不计为全量基线。

手动运行（始终显式指定fork，避免gh默认选中上游）：

```powershell
$sha = git rev-parse HEAD
gh workflow run tauri-rust-ci.yml --repo TanCan1998/Lorekeeper --ref master -f expected_sha=$sha
gh run list --repo TanCan1998/Lorekeeper --workflow tauri-rust-ci.yml --commit $sha
gh run view <run-id> --repo TanCan1998/Lorekeeper
```

首次配置提交通过push触发即可，不重复手动创建同一SHA运行。手动传入expected_sha必须与所选ref检出版本完全一致，否则失败，不悄悄改测另一个版本。

- [工作流入口](https://github.com/TanCan1998/Lorekeeper/actions/workflows/tauri-rust-ci.yml)。
- 最多60分钟；首次无缓存可能明显慢于本机增量编译，不承诺提速。
- 独立RUSTUP_HOME/CARGO_HOME与Lorekeeper合成根；关闭增量与调试信息以降低托管runner磁盘占用。
- 缓存仅公开依赖与target，key包含OS/架构/固定Rust/manifest+lock哈希/提交SHA，复用同工具链和锁的先前编译。不缓存整个用户目录、凭据文件或业务数据。
- 仅contents:read，checkout不保留凭据。无Secrets输入、仓库写入、打包、签名或发布步骤。
- 只上传validation.json、cargo-test.log、cargo-check.log，保留14天。validation记录SHA、lock哈希、运行编号、工具链、两个独立退出码和时间；test失败不会被check成功覆盖，异常路径也保存失败状态。

## Action版本依据

于2026-10-17实时核对官方GitHub latest稳定release及tag提交，非凭历史版本猜测；执行时固定完整SHA。后续升级需重新核对官方release，而不是浮动跟随main。

| Action | 最新稳定版 | 固定SHA |
|---|---|---|
| [actions/checkout](https://github.com/actions/checkout/releases/tag/v7.0.1) | v7.0.1 | 3d3c42e5aac5ba805825da76410c181273ba90b1 |
| [actions/cache](https://github.com/actions/cache/releases/tag/v6.1.0) | v6.1.0 | 55cc8345863c7cc4c66a329aec7e433d2d1c52a9 |
| [actions/upload-artifact](https://github.com/actions/upload-artifact/releases/tag/v7.0.2) | v7.0.2 | cf430e030ddbb5b0abf93d22962f4752f3646cd9 |

使用GitHub托管runner满足这些Action的运行时要求，不更改既有Electron工作流的Action版本。Context7查询本轮额度耗尽，版本与接口改用官方release/tag及action.yml核实。

## 本机接续记录

pipeline分别保留“本机定向”“本机最近全量”“云端最近全量/等待运行”，记录SHA、run URL、结果及累计数。CI/文档切片不增加Rust语义切片计数；仅添加工作流并不意味着首次云端运行已经通过。
