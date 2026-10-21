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

## 发布结果（已完成）

- 发布源码 `f01b027b3f89e216bc5e9cf5cf5f41b417ae3b2e`；该 SHA 的 Tauri Rust CI [39539246181](https://github.com/TanCan1998/Lorekeeper/actions/runs/39539246181) 全量 **1749 通过 0 失败**、all-targets 通过；Commit message 39539246213、Gitleaks 39539246182、AtomGit mirror 39539246168 均 success。
- 打包发布 run [39539872741](https://github.com/TanCan1998/Lorekeeper/actions/runs/39539872741)：prepare 与三平台打包全部成功；发布作业在 GitHub 侧成功创建标签与九个资产并设为 Latest，随后因 AtomGit 同步超时退出非零。
- 第一次发布 run [39537492636](https://github.com/TanCan1998/Lorekeeper/actions/runs/39537492636) 失败于 macOS 打包：主配置 `beforeBuildCommand` 在两平台都执行 Windows 身份小包脚本。用户选择拆分配置：新增 `src-tauri/tauri.macos.conf.json` 覆盖为 `pnpm build`，主配置保留 Windows 身份构建，发布合同测试锁定两者；该修复提交后重跑云端全量。
- AtomGit 首次同步与第一次镜像重试均在 600 秒请求上限超时；两轮镜像 run 分别为 39542655304（失败）与补传后的 39544471952（success）。本机按"只补缺失附件"补齐九个附件（未覆盖已有文件），随后镜像工作流完成匿名读回、双源说明回写与两端一致性核验。
- 最终远端状态：GitHub `lorekeeper-v1.0.0-beta.1` 非草稿、非预发布、**Latest**（Release ID 419140959），九个资产与 SHA256SUMS 一致；AtomGit 同标签、同源码、非预发布、九个 attach 附件，说明含双源表格，读回 SHA256 与 GitHub 相同。旧 beta 保持不变。
- 发布说明：[docs-fork/releases/1.0.0-beta.1.md](../releases/1.0.0-beta.1.md)。
- AtomGit 中文首页复核（2026-10-21 只读抓取）：页面可访问且含「设定司」，但未出现「本 Fork 的目的」段落与新版本链接，**不能宣称首页已完整展示最新 README**；文件同步与 Release 成功不等于首页渲染通过，仍按既有说明等待平台审核或后续核验。

## 2026-10-21 后续：镜像方式改为 AtomGit 拉取

- 用户要求发布由 AtomGit 拉取，不再由 GitHub 推送。已删除 `.github/workflows/atomgit-mirror.yml` 与 `tauri-app/scripts/sync-atomgit.mjs`；`publish-release.mjs` 不再导入镜像模块、不要求 AtomGit Secret，发布作业只处理 GitHub。
- Git 镜像只同步代码与标签，**不同步 Release 安装包**：附件改为代码拉取完成后单独运行 `pnpm mirror:release`（`mirror-published-release.mjs`），只补缺失文件并做匿名 SHA256 读回。
- 平台配置已完成：AtomGit 仓库设置 → 仓库镜像 → Pull 启用（`mirroring_enabled=true`，`force=false`），并手动“立即同步”一次；实测 `master` = GitHub HEAD `ccdb3664`、标签 `lorekeeper-v1.0.0-beta.1` = `f01b027b`、失败数 0。平台会在后续 GitHub 提交进入时按自身间隔自动拉取，不再需要 GitHub 推送。手动“立即同步”有约 117 分钟冷却，重复触发返回 `BAD_REQUEST`；本次实测停在 `ccdb3664`，之后两个文档提交等待平台拉取。
- 拉取只更新代码与标签，不改动已上传的 Release；beta.1 九个附件与说明保持原样。
- 已发布的 `lorekeeper-v1.0.0-beta.1` 是旧推送流程产物，两端九附件与 Latest 状态不变；新流程从下一次发布开始生效。
- 本机验证：Node 合同 39 通过、脚本语法、Tauri typecheck/lint 通过。未新增依赖、schema 或权限。

## 2026-10-21 后续：本机安装验收（新产物）

用户确认先补前端错误码同步，再授权本机安装验收（含一次 UAC）。下载的两个包与发布清单 SHA256 一致。

- **MSI（`1.0.1.0`）**：提权静默安装 exit 0；HKLM 卸载项 `LoreKeeper 1.0.1.0`、`C:\Program Files\LoreKeeper` 含 `lorekeeper.exe` / `lorekeeper-activate.exe` / `identity\`；公共桌面快捷方式 1 个，指向激活器、无参数；Appx 身份 `com.tancan1998.lorekeeper 1.0.1.0` 注册成功。修复安装 `/famus` exit 0，版本与身份不变。静默卸载 exit 0：卸载项 0、安装目录删除、Appx 注销、快捷方式清理，`~/.lorekeeper` 保留。
- **NSIS（`1.0.0-beta.1`）**：当前用户静默安装 exit 0，装入 `%LOCALAPPDATA%\LoreKeeper`（含激活器与 `identity\`）；用户桌面与开始菜单快捷方式均指向 `lorekeeper-activate.exe`，无参数；HKCU 卸载项版本 `1.0.0-beta.1`。静默卸载 exit 0：目录、卸载项、快捷方式清零，`~/.lorekeeper` 保留；安装后未注册 Appx（需首次经激活器启动，未测）。
- **首次激活器注册**（无需启动主程序）：`--unregister-identity --quiet` exit 0 → Appx 0；`--register-identity --quiet` exit 0 → Appx `com.tancan1998.lorekeeper 1.0.1.0`（WindowsApps 内启动器）。HKLM 安装记录含 schema 1、machine 范围、安装目录与身份启动器路径；HKCU 记录含桌面/开始菜单/卸载快捷方式计数。卸载后安装目录、Appx、HKLM/HKCU 记录全部清零。
- **项目清理向导（NSIS 卸载）已通过**：用隔离 fixture（合法 .lore/project.json）勾选“清理所选项目”→ 卸载 → 目录选择对话框填入 fixture → 清理程序显示所选目录与删除范围 → 第一次「删除所选项目」→ 第二次确认 → 提示“所选项目已清理。其他项目和全局配置保持不变”。fixture 删除、快捷方式与卸载项清零、`~/.lorekeeper` 与全局配置保留。无效清单（缺 kind/storageFormat/storageVersion）会被拒绝并提示“只允许清理含有效 .lore/project.json 的 LoreKeeper 项目”。
- **发现卸载残留（未修复）**：NSIS 当前用户卸载调用激活器注销身份时因缺少管理员权限失败，把 `identity-launcher.log` 写进 `%LOCALAPPDATA%\LoreKeeper`，导致安装目录残留一个日志文件；Appx 身份与 HKLM/HKCU 记录最终均已清零。修复需改安装器钩子（日志写到临时目录或卸载时清理），属发布配置变更，待用户决定。
- 多用户 UAC 与旧版本升级仍未验收（需第二个登录账号 / 旧版安装包）。

## 仍未验收

- 本次四包的首次启动身份注册、项目删除、注册表清理与提权/多用户 UAC 未执行（安装/修复/卸载本身已按上节通过）；历史 MSI 证据属于旧产物。
- macOS 真机安装、活动监视器归属、签名公证未验收；Windows WebView2 任务管理器归组未修复。
- 真实模型、MCP、业务云与长期写作质量未认证；新版发布说明已披露。
