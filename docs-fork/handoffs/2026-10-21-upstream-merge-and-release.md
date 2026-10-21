# 上游同步与 1.0.0-beta.1 发布

## 授权与范围

- 用户要求合并上游，并明确同意根依赖/锁文件更新、Electron 发布配置/ADR/会话修复导入、CI 上游贡献者邮箱白名单调整及验证后推送。
- 随后确认先迁移上游新增的本机 Skill 能力，再发布 `1.0.0-beta.1`：普通 Release、Latest，Windows x64 MSI/EXE 与 macOS ARM64/Intel DMG，同步 GitHub 与 AtomGit。不覆盖旧 beta。
- 本轮恢复相关自动验证和发布流水线；不授权真实模型/MCP/业务云、前台 GUI、安装/卸载/系统注册或多用户 UAC。禁止子代理继续有效。

## 已实现

- `a831b2d8` → 上游 `7e6b827f`：285 个提交、245 个上游文件，临时分支无冲突合并，合并提交 `f830e861`。`docs/` 与上游一致，fork 品牌/窗口/ESC/独立数据域保留；根版本 1.3.1 仅为 Electron 基线。
- 本机 Skill 两个新频道及 `bundle` 返回字段已同步。Rust 原生选择、窗口绑定且 10 分钟过期的一次性候选、重新读取并核对指纹、引用文本打包、清单/SHA256 校验、生成来源观察及哈希接通；渲染层不接收源路径。
- 复用既有 dialog、哈希和文件身份能力，无新增依赖、schema、锁文件或 capability 权限。清单命名 `.lorekeeper-bundle.json`，不读取 Electron 库。
- 检查 64 KiB 单文件/合并正文、16 个附带文本、200 个文件夹条目、4 层目录；拒绝越界、非 UTF-8、链接/硬链接和不兼容提示词。安装先写清单，失败保留损坏状态，不递归清理未经复核的目录。
- 最新频道统计：288 invoke / 7 事件，289 注册命令，287 功能迁移、1 项旧库明确不支持、缺失 0。

## 本机验证

- 根 typecheck（进程级 Node 内存上限 4096 MiB 后重试通过）、lint；根 Skill 相关 Vitest 120 通过。根测试显式排除 `.pi/**` 临时历史副本和 `tauri-app/**`，不改上游配置。
- Tauri typecheck/lint 通过；Vitest 223 文件、2798 通过、1 skipped、0 失败。
- Rust `writing_skills` 23 通过；`commands::app_data` 5 通过；`generation_source_binding` 5 通过。新增本机包用例在 Windows 已通过 14 项，Unix 专用链接用例待对应平台。
- Node 发布/安装器/镜像合同 39 通过。存在既有 `ProjectCoreSynopsisExpected` 未用导入警告，不据此修改无关代码。
- 初次前端检查暴露类型引用与已迁移集合问题，均修复并复验；初次浏览器检查缺少 Playwright 自带浏览器，改用本机 Edge 无头模式，Skill 设置页 7 测试通过（有 React act 告警），不安装浏览器、不抢焦点。

## 发布门禁与待办

- 此快照提交时尚未推送，不能登记新 SHA 云端全量成功。按混合政策等待同一发布 SHA 的 Tauri Rust CI 全量与 all-targets 成功，再启动发布流程。
- 发布必须核验同一 SHA 的三平台四包、精确九附件、摘要、GitHub Latest 与 AtomGit 读回；未完成前不宣称已发布。
- 当前原生 Skill 选择器仍待人工验收；旧版本升级、多用户 UAC、macOS 真机、真实模型与长期写作质量未认证。历史 MSI 证据不能替代本轮产物。
