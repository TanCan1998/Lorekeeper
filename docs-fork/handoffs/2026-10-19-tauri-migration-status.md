# LoreKeeper迁移进度快照（2026-10-19）

> [当前入口](./CURRENT.md) · 第 **125** 次：用户要求把剩余未迁移项一次性合并迁移、不切片，并授权最后统一验证与提交推送。已分 7 个主题提交并推送 `3e8101ba..011b4b0f`，**云端 Rust 全量与配套检查全部通过**。已完成源码：大纲/蓝图作者恢复全链路、目录进度登记、合成前五个可变性守卫、头像双语与缓存、MSI 正常入口改走激活器、陈旧“未迁移”注释纠正。**验证通过**：`cargo check --all-targets` 0 错误，`cargo test --lib` **1734 通过 0 失败**，定向 344 通过，24 文件 rustfmt 通过，`pnpm typecheck`/`lint` 通过，安装合同 13 通过，频道覆盖无缺失；前端 vitest 2757 通过、1 项既存品牌大小写断言失败（前端源码与测试均未改）。只新增既有 windows-sys feature，无新 crate、schema、锁或权限变更；HEAD仍为`3e8101ba`。**未构建/安装 MSI、未注册提权、未提交推送、未发布**。
>
> 第 **124** 次：已静态接入项目文件系统错误文案的已保存界面语言，补7个回归用例未执行；只改文案选择与错误呈现，不改权限/租约/文件操作。
>
> 第 **119** 次：MSI 全机安装后注册、卸载前注销均已实测通过；`Add-AppxProvisionedPackage` 未生效，多用户待解决。
>
> 第 **118** 次：真实产品经激活器启动后，主程序与 6 个 WebView2 读回同一包身份；提权注册、真实安装与卸载清理全部实测通过。MSI 注册与多用户仍待做。
>
> 第 **117** 次：身份包真实产出、随安装器分发，并完成真实静默安装/卸载回归；发现未签名注册必须提权（0x80073D2B）。归组未在产品上验收，发布暂停。
>
> 第 **116** 次：产品身份启动器与全机安装记录已实现并真实构建 MSI/NSIS（仅构建，未安装）。身份包注册、激活入口与多用户置备未完成，发布暂停。**不是LoreKeeper产品修复或新版发布完成报告。**

## 独立维护：AtomGit 镜像与双源下载

用户单独确认配置 GitHub→AtomGit 自动同步、仓库 Secret、同批发布附件以及双源下载表格。镜像配置提交为 `467029c5`，未把本地待验 Rust/安装器改动混入；提交使用 `[skip ci]`，没有触发编译/打包工作流。

- GitHub/AtomGit 的既有 `lorekeeper-v1.0.0-beta` 九附件原样传输，AtomGit 全部附件匿名读回大小与 SHA256 后，两端正文更新为相同双源表格，国内推荐 AtomGit；未发布 beta.1。
- 代码同步首轮 `39228355899`、最终提交 `39228758897` 均成功；两轮发布作业均跳过。README 使用仓库内文档/图片相对路径，附件采用两平台绝对 URL。
- 本机仅脚本语法、YAML、链接与 diff 等轻量检查；新增单元回归尚未运行。完整新版本双端发布、首次缺失附件的 Node 上传路径仍待后续有授权的验收。
- 这不更新主应用 Rust 全量基线、不清零切片累计，不解除资源操作与原生生命周期暂停；详见[镜像协议](../agents/atomgit-mirror.md)。

## 本次更新：第125次剩余迁移合并实现（已提交，云端验证通过）

- 授权变化：用户明确「还没有迁移的一起迁移不切片，等最终一次性验证」，并在集中确认卡中选择「按上述范围合并迁移」+「最后再确认验证授权，本轮仅合并源码」。故本轮只写源码与用例声明，**不运行**测试/编译/格式器/前端检查，也不打包或安装 MSI。
- **大纲作者恢复**：`ProjectCoreSynopsisCommitRequest` 补 `generationRunHandle` / `authorRecovery`（区分键缺失与显式 null），`db:project-core-synopsis-commit` 改为三分支信封（作者恢复 / 生成运行 / 普通提交），返回 `receipt`。命令层新增 `commit_plot_outline_author_edit`、`assert_synopsis_commit`、`plot_outline_target_current`：事务内校验会话租约、epoch、期望快照、回执幂等、源一致性与在途状态，成功才写台账并记录回执。
- **蓝图作者恢复**：`BlueprintCommitRangeRequest` 补 `authorRecovery`，回执补 `authorEdit` / `generationProgress`；命令层新增 `commit_blueprint_author_edit`，复用 `commit_range_in_existing_transaction` 在同一事务内完成范围提交与回执登记，并按基线返回 `GENERATION_BLUEPRINT_AUTHOR_*` 状态码。新增 `services/blueprint_author_payload.rs` 重新校验作者载荷（章节精确覆盖、必填文本、角色唯一、关系端点/自指/重复、新角色候选角色枚举）。
- **目录进度**：新增 `record_directory_commit`，`db:blueprint-commit-range` 分支③改为单事务「范围提交 → 子代效应 → 目录进度登记」，幂等重放同样登记/校验进度，回执不再省略 `generationProgress`。恢复读面新增 `unavailable` 分支（`GENERATION_BLUEPRINTS_CLEARED` / `GENERATION_DIRECTORY_PROGRESS_INVALID`）。
- **可变性守卫**：`compose_visible` 补齐基线 5 个子生成器 `assertMutable`（reviewRevisions / imports / finalizations / graphs / legacyRosters）；`in_progress` 判据叠加执行面 `has_pending_execution()`。
- **头像**：错误文案与文件选择标题接入已保存界面语言（缺省回退系统语言，仍不读项目写作语言）；新增 `commands/character_avatar/cache.rs` 会话域缓存（项目 + 租约 + 数据根隔离、按资产修订索引、FIFO 上限 512、commit/remove 后失效、每次仍校验数据库记录）。仅复用既有 `windows-sys` 并启用 `Win32_Globalization`，未新增 crate。
- **MSI 正常入口**：新增 `installer/main.wxs`（从已安装 Tauri CLI 2.12.1 内嵌模板导出，附 `main.wxs.LICENSE`），桌面/开始菜单快捷方式与安装结束启动改为 `[INSTALLDIR]lorekeeper-activate.exe`，移除 `[LAUNCHAPPARGS]`；`tauri.conf.json` 指定 `wix.template`；安装合同新增静态断言。**未构建 MSI，未安装**。
- **注释纠正**（不改语义）：`import_generation` 头注（两频道已实现）、`character_proposal` 两处 TODO（解析器已迁移）、`compose_visible` / `begin_batch` / `generation.rs` 源探针说明、`db/schema.rs` 三张空表用途（Electron 内部服务，无 IPC 频道）。
- 回归声明：本轮新增 11 个（config 1、db 2、avatar cache 3、蓝图载荷 5），全部**未执行**；未提交的累计声明为 49 个，不得计为通过。
- 静态检查：`git diff --check` 通过，保护目录（`electron/`、`src/`、`docs/`、根构建与锁文件）与 `Cargo.lock` 无差异，`Cargo.toml` 仅新增既有 feature；39 个改动/新增文件指纹见本机 `.pi/tmp/migration-batch-125/static-check.json`。

### 最终一次性验证结果（用户授权：完整验证）

| 范围 | 结果 | 证据 |
|---|---|---|
| `cargo check --all-targets` | **0 错误** | `D:\Temp\lorekeeper-verify\check.log` |
| `cargo test --lib`（全量库测试） | **1734 通过，0 失败**，55.27s | `full-tests.log` |
| 定向 9 个模块（config/fs/epub/import/avatar/db/generation_run_service/main_generation_owner/blueprint payload） | **344 通过，0 失败** | `test-*.log` |
| `rustfmt --edition 2021 --check`（24 个改动文件） | **通过**（首次检查发现 18 个文件需格式化，已就地格式化后复检通过） | `fmt-*.log` |
| `pnpm typecheck` / `pnpm run lint` | **退出码 0** | `frontend.ps1` 输出 |
| 安装合同 `node --test scripts/installer-contract.node-test.mjs` | **13 通过，0 失败** | 同上 |
| 频道覆盖 `verify-channel-coverage.mjs` | 286 invoke / 7 事件 / 287 注册 / 285 计入迁移 / 1 明确不支持 / **缺失 0** | 同上 |
| 前端 `vitest` | 2758 通过、1 skipped、0 失败 | `vitest-full2.log` |

- 修复记录（编译阶段）：`source_row` 闭包错误类型、`ImportGenerationContext`/`ProjectCoreData` 缺 `Deserialize`、`MainGenerationRunHandle` 误用 `is_null`、`AvatarCache` 缺 `Debug`（改为只打印作用域与条目数的自定义实现）；`installer-contract` 里模板字符串转义导致正则失配，改为 `indexOf` 切片断言。
- 首轮 `vitest` 有 1 项既存失败：`src/services/__tests__/safe-call-diagnostic.test.ts:60` 硬编码期望 `# Lorekeeper safe diagnostics`，而实现取 `APP_BRAND.enName`（`LoreKeeper`）。已改为引用 `APP_BRAND.enName` 拼装期望，既修复断言又避免品牌再变时悄悄失效；复跑全套 2758 通过、1 skipped、0 失败，`pnpm typecheck` 与 `pnpm run lint` 仍为 0。
- 未做：MSI 构建 / 安装 / 卸载 / 包注册 / 提权 / GUI 与原生文件选择器 / 真实模型·MCP·云业务 / 发布。

### 提交与云端验证（2026-10-19）

工作区按主题分成 7 个提交，范围 `3e8101ba..011b4b0f`，已推送 `origin/master`；提交消息本地检查 7/7 合规，无 BOM/CRLF/尾空白。

| 提交 | 主题 |
|---|---|
| `ea635620` | 身份包版本刷新与安装记录生命周期 |
| `07e242e1` | EPUB 导入的作者原稿与参考语料 |
| `cb91857f` | 项目文件系统错误文案跟随界面语言 |
| `0ec40039` | 角色头像文案本地化与会话域缓存 |
| `3f9db5bd` | 大纲与蓝图作者恢复写入链 |
| `12ef37f8` | MSI 正常入口改由身份激活器启动 |
| `011b4b0f` | 第125次交接与验证记录 |

云端对应同一 SHA `011b4b0f`：

| 工作流 | 运行 | 结论 |
|---|---|---|
| Tauri Rust CI（Windows 全量） | [39282333603](https://github.com/TanCan1998/Lorekeeper/actions/runs/39282333603) | **success** |
| Commit message CI | [39282333517](https://github.com/TanCan1998/Lorekeeper/actions/runs/39282333517) | success |
| Gitleaks | [39282333549](https://github.com/TanCan1998/Lorekeeper/actions/runs/39282333549) | success |
| AtomGit mirror | [39282333525](https://github.com/TanCan1998/Lorekeeper/actions/runs/39282333525) | success |

- README 与交接此前只写“未提交推送”，已在本次文档提交中更正；代码提交 `3e8101ba..12ef37f8` 之后未再改动源码。
- 前端 vitest 的既存品牌大小写断言已随文档提交一并修正为引用品牌常量；云端 PR CI 只在 PR 上运行，本次推送未触发它。

### MSI 正常入口真实安装验收（2026-10-20，用户授权）

构建产物 `LoreKeeper_1.0.0-beta.1_x64_zh-CN.msi`（53 841 920 字节），在**未安装过 LoreKeeper 的干净状态**下完成全流程，全部退出码 0。

**只读表检查（安装前）**

- 开始菜单与桌面快捷方式 Target 均为 `[INSTALLDIR]lorekeeper-activate.exe`，Arguments 为空。
- `Property` 表已无 `LAUNCHAPPARGS`；`LaunchApplication` 自定义动作 Target 为 `"[INSTALLDIR]lorekeeper-activate.exe"`，不带参数。
- `RegisterIdentity` / `UnregisterIdentity` 仍按 `--register-identity` / `--unregister-identity --quiet` 接线。

**静默安装（`/qn`）**

- 文件：`lorekeeper.exe` 79 703 552 字节、`lorekeeper-activate.exe` 306 176 字节、`identity` 目录、卸载快捷方式。
- 注册表：`DisplayName=LoreKeeper`、`DisplayVersion=1.0.1.0`、`InstallLocation=C:\Program Files\LoreKeeper\`、`UninstallString=MsiExec.exe /X{A9F7D83B-D0C5-4FA1-9BCB-369F70065CE7}`。
- 快捷方式：公共桌面 `LoreKeeper.lnk` 与开始菜单 `LoreKeeper\LoreKeeper.lnk` 均指向 `lorekeeper-activate.exe`，Arguments 为空。
- 身份：`com.tancan1998.lorekeeper_1.0.1.0_x64__e8hwe6njnc7fy` 已注册，`Get-StartApps` 显示 `LoreKeeper => com.tancan1998.lorekeeper`。
- 安装期间未自动启动主程序（无 `lorekeeper` 进程），未抢焦点。

**修复安装与身份重注册**

- `REINSTALL=ALL REINSTALLMODE=vomus` 退出码 0，可执行文件与身份包版本不变。
- `lorekeeper-activate.exe --register-identity --quiet` 重复执行退出码 0，包全名不变（幂等）。

**卸载**

- `msiexec /x {A9F7D83B-…} /qn` 退出码 0：安装目录、卸载注册表项、Appx 包、`Get-StartApps` 条目、公共桌面与开始菜单快捷方式全部清除，无残留进程；`~/.lorekeeper` 用户数据未被删除。

**从快捷方式启动（同一轮，用户另行授权）**

重装后走真实桌面快捷方式 `C:\Users\Public\Desktop\LoreKeeper.lnk`（解析结果：Target `C:\Program Files\LoreKeeper\lorekeeper-activate.exe`、Arguments 空、工作目录为安装目录）：

- 2 秒内拉起 `lorekeeper-activate.exe` 与 `lorekeeper.exe`，两者 `Responding=True`。
- 主窗口句柄非零，标题 `LoreKeeper`；WebView2 运行时进程 30 个，CDP 探到 1 个 `type=page` 目标，标题 `Lorekeeper 设定司`、URL `http://tauri.localhost/`，说明渲染器已完整加载而非仅进程存活。
- `CloseMainWindow()` 后主程序与激活器都自行退出，无需强杀。
- 本次运行 `~/.lorekeeper` 与 `~/.vela` 都没有今日写入（各 0 个文件），未出现跨栈数据污染；`~/.vela` 为 Electron 基线历史目录，时间戳未变。
- 随后静默卸载退出码 0，目录、注册表、Appx、`Get-StartApps`、桌面与开始菜单快捷方式再次全部清除。

**已知差异与未验收**

- 注册表 `DisplayVersion=1.0.1.0`（WiX 数值版本）与可执行文件 `FileVersion=1.0.0-beta.1` 不同：Windows Installer 的 `ProductVersion` 只接受纯数字点分格式（major/minor ≤ 255，patch/build ≤ 65535，Tauri schema 明确说明），`-beta.1` 不可用，故两项必须分开维护；该字段在 `44259fa3` 显式固定，未填时 Tauri 会尝试从产品版本推导。
- `lorekeeper-activate.exe` 无文件版本资源（小体积启动器）。
- 仍未验收：旧版本升级路径（用户决定搁置到下个版本）、多用户 UAC 与本机全量注册/注销回归。

## 前轮更新：第124次项目文件系统错误文案双语迁移（未验收）

- 用户在询问是否恢复EPUB最小验证时明确选择“继续迁移未迁移项”；不恢复测试/构建，转向残余子功能。蓝图作者恢复缺少规划恢复准入和持久回执链，涉及安全/恢复语义，未据泛授权擅自实现。
- 范围：`commands/config.rs`补无系统语言参数的`main_text`，对齐基线`mainText(undefined, zh, en)`；逐次读取已保存的`locale`，不缓存、不写回、不使用项目写作语言；未保存/损坏配置按基线回落英文。这不是全部主进程或系统语言回退迁移完成。
- `commands/fs.rs`七频道错误呈现接入该文案函数；保留守卫错误优先、文件缺失工具提示、IO脱敏和响应信封。`mkdir/read-json/write-json`不再丢弃已判定的错误类别后固定返回中文；底层守卫、原子写、读取/列目录及提交状态规则不变。
- 新增配置3项、文件系统4项合成回归（**7个声明，未执行**）：已保存语言即时切换、损坏/缺失配置不改写、语言与写作语言隔离、守卫/频道矩阵、缺失文件提示、原始路径脱敏与信封形状。
- 本轮只做源码结构核对、`git diff --check`、空白/BOM与保护路径/依赖差异检查；证据`.pi/tmp/fs-locale-migration/static-check.json`。EPUB5个源码文件与前轮指纹一致；不更新任何测试/GUI/全量通过结果，不提交推送。
- 下步验证仍待恢复资源操作：定向`cargo test commands::config::tests`及`cargo test commands::fs::tests`、定向格式、跨模块all-targets；连同前轮EPUB切片按既有累计/阶段门槛收口，不以34个新增用例声明冒充34通过。
- 非本轮范围：头像缓存/系统语言回退、蓝图作者恢复、MSI正常入口与原生生命周期。没有新增依赖、schema、权限、定稿或会话准入变化；频道映射与注册统计不变。

## 前轮更新：第123次 EPUB 静态迁移（未验收）

- 授权：用户先选择只读核查，确认 EPUB 属于 Tauri 未迁移后明确要求“迁移”并“继续”；仅推进该源码切片，不解释为恢复测试/构建、修改安装器或发布授权。
- 实现：`src/import/epub.rs` 纯内存读取 ZIP；container→OPF manifest/spine 阅读顺序；跳过导航、非线性和非正文项；严格 UTF-8/UTF-16 解码、正文/标题/实体提取，不执行 HTML、不访问外链、不落地条目。
- 防护：条目数10000、单条目32MiB、累计解压128MiB，并收窄至剩余导入预算；拒绝损坏/重复/越界引用及加密 ZIP，DRM 与已知字体混淆例外按上游判定。针对现有zip9.0.3合并原始同名条目的行为，补充已定位中央目录的计数一致性检查；解压、ZIP64与CRC仍由zip处理。
- 接线：`commands/import.rs` 保留原文本读取行为，EPUB正文走现有拆章、字数/哈希、作者重号校验、参考语料冻结与失败台账；不改schema、定稿、租约或外部文件授权规则。`Cargo.toml`/`Cargo.lock`不变。
- 回归：新增提取器20项、命令层7项（源码声明计数，**不是通过数**），覆盖顺序、编码、DRM/字体、Zip64/CRC、重复名称/伪造大小、预算及作者/参考路径；旧EPUB占位回归改为损坏格式拒绝。
- 本轮实测仅`git diff --check`、新文件空白/BOM与保护路径/依赖差异核查；源码指纹和静态证据见本机`.pi/tmp/epub-migration/static-check.json`。**cargo/rustfmt/TS/GUI均未执行，不记迁移验收完成**；没有提交、推送或在飞运行。
- 只读口径复核：当前源码286 invoke/7事件、287注册命令（含health）、285计入迁移/1旧库明确不支持，无缺失注册；不因本次填充频道内功能而增减统计。旧290是历史口径，注册覆盖不等于功能完整。
- 后续：用户恢复资源操作后，先定向`cargo test import::epub::tests`与`cargo test commands::import::tests`、定向rustfmt，再按跨模块规则补all-targets；阶段收口全量遵循既有本机/云端分工。首次原生文件选择器需遵守不抢焦点规则，无法后台完成则标待验收。
- 另行待办：蓝图`authorRecovery`分支存在前后端接线缺口，仅只读确认，本轮未修改。MSI入口、升级/回滚、多用户和真实外部服务边界仍按CURRENT保留。

## 已验证历史基线（第110至119次，不代表第124次工作区通过）

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

## 前轮更新：第122次升级版本刷新与修复条件静态修改

| 范围 | 本轮状态 | 版本/证据 |
|---|---|---|
| 分支、HEAD、工作区 | `master`；迁移切片仍未提交；HEAD/origin 后续已推进至独立镜像提交 `467029c5` | 本轮本地只读 Git 检查，2026-10-19 |
| 身份版本刷新 | 已编辑：构建版本注入、启动前精确匹配、补注册后重读复核 | `build-windows-identity.mjs`、独立启动器 `record.rs/win.rs` |
| MSI 升级/修复/卸载条件 | 已编辑：修复不注销、非完整卸载刷新、移除文件前才注销 | `installer/lorekeeper-install-record.wxs`；未运行 candle 或 msiexec |
| 静默与路径回归 | 已补用例，未运行；静默传至提权及包内助手，单引号按字面量转义 | `installer-contract.node-test.mjs`、`record.rs` |
| 本轮测试/构建/GUI/系统变更/CI | **全部暂缓**，未提交推送；只做源码审阅与轻量差异检查 | 用户要求“涉及到资源占用的操作先不执行” |
| 最近独立启动器验证 | 沿用第121次 Node 39、启动器10、clippy与三个CI成功记录 | 仅属于 `05b740b5`，不归给本轮改动 |

- 本轮没有修改产品版本号、identifier、UpgradeCode、数据根、schema、新依赖、权限或 CI/发布配置；没有后台遗留构建/测试任务。
- 更正当前口径：生成 MSI 的两类快捷方式 `Target="[!Path]"`、安装后 `LaunchApplication FileKey="Path"` 都直指主程序。尚未接入激活器，手动激活器身份采样不能当作这些入口验证；不得继续声明 MSI 所有正常入口均归组。
- 方案 A 只要求全机程序文件 + 每用户按需注册；真实第二账号与不同凭据 UAC 未验收，同账号删包后自愈不能替代。既有置备失败记录也不等于证明所有未签名置备路线不可能。
- 下一步：先静态修正 MSI 实际入口；用户恢复资源操作后再做定向测试/格式、增量版本构建、升级/修复/回滚/卸载矩阵与后台原生验收。生产产物通过既有身份构建脚本注入版本，不能拿无版本的直接 cargo 构建替代。
- 失败回滚、最低系统、breakaway、macOS、跨账号/真实数据权限生命周期仍阻塞发布；未创建 tag、Release 或 Latest。
- 证据与待验证矩阵见[研究§16](../research/webview-process-attribution.md#16-第122次升级版本刷新与修复条件静态修改)，活动状态见 `.pi/tmp/pipeline-state.md`。

## 前轮更新：第119次 MSI 全机注册/注销

- WiX 片段新增两个即时自定义动作（`WixQuietExec`）：安装 `InstallFinalize` 后 `--register-identity`，卸载 `InstallFinalize` 前 `--unregister-identity`；条件分别 `NOT Installed` 与 `Installed AND NOT UPGRADINGPRODUCTCODE`。序列内条件必须写成元素文本，写成属性或子元素都会让 candle 失败。
- 实测（提权静默）：`msiexec /i ... /qn INSTALLDIR=自选目录` → 退出 0、HKLM 记录 `scope=machine` 且含 `IdentityLauncherPath`、包计数 1；`msiexec /x ... /qn` → 退出 0、目录/HKLM 记录/包全部清零。
- 记录写回位置按命中的蜂巢（HKLM/HKCU，含 WOW6432Node），修掉了此前固定写 HKCU 的错误。
- **缺口**：`Add-AppxProvisionedPackage -SkipLicense` 未生效（`provisioned=0`），多用户置备不成立；升级后包版本变化会让启动器路径指向旧目录，重新注册/路径刷新未实现。
- 详见[研究§13](../research/webview-process-attribution.md#13-第119次msi-全机注册注销闭环与置备缺口)。

## 前轮更新：第118次真实产品归组与提权注册

- 提权静默安装（`/S /NS /D=自选目录`）退出 0：身份包注册成功（Status=0），`IdentityLauncherPath` 指向 WindowsApps 内启动器且文件存在。
- **真实产品启动核验**：经激活器启动后，主程序 PID 28944 位于所选目录，包身份与 6 个 WebView2 子进程完全一致（去重后仅一个包全名）。
- 提权静默卸载退出 0：目录、HKCU 记录、当前用户包、WindowsApps 包目录全部清零；测试实例自行关闭，未触碰其他应用（机器上仍有 24 个他人 WebView2 进程）。
- 启动器新增 `--register-identity`（非提权明确报错）：`Add-AppxProvisionedPackage -SkipLicense` + `Add-AppxPackage -AllowUnsigned` + 回写 `IdentityLauncherPath`；NSIS 失败且非静默时用 `ExecShell "runas"` 调起。
- 云 CI：`4c66847c` 的 Tauri Rust CI 因已声明资源未预生成失败，`76fec45` 修复后 success。
- 未完成：MSI 注册自定义动作、第二个用户下的置备验证、升级/回滚、最低系统、breakaway、macOS。详见[研究§12](../research/webview-process-attribution.md#12-第118次真实产品归组与提权注册闭环)。

## 前轮更新：第117次身份包产物与真实安装回归

- 真实产出身份包：独立启动器 release 262,144 B → MSIX 134,256 B（仅启动器+图标，`AppListEntry="none"`）；构建接入 `beforeBuildCommand`，资源经 `tauri.windows.conf.json` 随 NSIS/MSI 安装为 `identity\LoreKeeper.msix` 与 `lorekeeper-activate.exe`。
- **真实静默安装**到用户自选目录：退出 0，落盘主程序、激活器与身份包；`HKCU` 记录 schema=1/scope=user/路径正确。**真实静默卸载**后目录、注册表键、身份包全部清零。
- **决定性限制**：未签名 MSIX 普通用户注册失败（`0x80073D2B`，加 `AppListEntry=none` 仍失败），提权后同一包注册成功、包内启动器就位（测试后已移除）。故 Tauri NSIS `currentUser` 无法自动获得身份；安装器改为失败明确告警，不假成功。
- 未签名包不能有可执行激活项 → `shell:AppsFolder` 作废；包外启动器改为读取登记的 `IdentityLauncherPath`（限定 WindowsApps 内本产品启动器）再转交，绝不无身份启动主程序。
- 合同 Node 38 项通过；启动器单测 10/10、clippy 无告警；`git diff --check` 通过。未验证：MSI 注册/置备自定义动作、真实 GUI 归组、升级/回滚、多用户、breakaway、macOS。
- 详见[研究§11](../research/webview-process-attribution.md#11-第117次身份包产物资源安装与真实安装卸载回归)。

## 前轮更新：第116次产品启动器与安装记录

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
