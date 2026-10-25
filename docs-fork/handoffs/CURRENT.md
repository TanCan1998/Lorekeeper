# 当前交接入口

- **最新维护（2026-10-25 第138次）**：重打包并完成覆盖安装与后台启动验收，桌面图标与安装入口落地。
  - **打包**：`wix.version` 1.0.2.0 → **1.0.3.0**（同版本 MSI 会被当「修复」而不刷新文件，必须递增；UpgradeCode 保持 `{ED3E3199-…}`）。本地 `pnpm tauri build --bundles msi,nsis`：MSI 53,923,840 B / NSIS 50,166,023 B，SHA256 见证据文件。
  - **覆盖安装**：`msiexec /i … /qn` 退出码 **0**；旧 1.0.2.0 条目被升级移除，ARP 只剩单条 **v1.0.3.0**（ProductCode `{88025DBA-…}`）；Appx 重注册为 **1.0.3.0**；桌面与开始菜单快捷方式均指向 `lorekeeper-activate.exe` 且无参数，图标取自 MSI `ProductIcon`。
  - **图标核验**：安装后 `lorekeeper.exe` 的 32px 图标与新 `icons/icon.ico` 逐像素一致（差异 0.0），与旧 PageKin 图标差异 132.27 → 打包链用的是新星芒图标。**发现的真问题**：16/24/32px 帧是整图等比缩小，低分辨率下「司」与星芒糊成文档状，需小尺寸专用简化稿（未做，待定）。
  - **后台启动**：经真实桌面快捷方式拉起 → `lorekeeper` + `lorekeeper-activate` + `lorekeeper-identity-launcher`，窗口标题 `LoreKeeper`，CDP 探到 `Lorekeeper 设定司` / `http://tauri.localhost/`；仅发 `WM_CLOSE` 后 2.04s 内三进程全部退出；`~/.lorekeeper`（10-23）与 `~/.vela`（10-16）均无新写入；未置顶、未激活、未用键鼠。
  - **云端全量（新基线）**：`9eb064a8` 上 Tauri Rust CI [40160190800](https://github.com/TanCan1998/Lorekeeper/actions/runs/40160190800) **1752 通过 0 失败** + all-targets 通过；同 SHA 的 contract tests、Commit message CI、Gitleaks 均 success。Rust 基线由 `6ba2bc8d` 更新为 `9eb064a8`。
  - **未做**：卸载/重装、NSIS 原生安装、多用户 UAC、旧版升级、macOS 真机；未发布，beta.2 的 Release/标签/附件未动。证据：`.pi/tmp/repack-2026-10-25/evidence.json`（不入库）。
- **上轮（2026-10-25 第137次）**：把安装器/发布契约测试接上自动入口，并清掉交接口径里已被推翻的过时条目。把安装器/发布契约测试接上自动入口，并清掉交接口径里已被推翻的过时条目。
  - **契约测试入口**：新增 `test:contract`（`tauri-app/package.json`），一次跑完 installer-contract / release-helpers / windows-identity / artifact-retention / atomgit-release 五个 `node --test` 脚本；新增 `.github/workflows/tauri-contract-ci.yml`（fork 独有、Node-only、windows-2022，路径过滤 `tauri-app/scripts/**`、`src-tauri/installer/**`、`tauri.conf.json`、`package.json`）。此前这批测试只写在 [发布说明](../agents/tauri-release.md) 里，靠人手逐条敲。
  - **实测**：`npm run test:contract` 在当前 HEAD **51 通过 / 0 失败 / EXIT=0**（安装器合同、wix 版本门禁、身份清单、产物保留、发布附件校验）。CURRENT 旧条目「新增回归用例未执行」据此销号。
  - **口径纠偏**：删除「迁移资源操作仍暂缓（不编译/测试/打包/安装/卸载/提权）」「MSI 仅源码修改、未构建未安装」「等最终验证授权后本机一次性跑 cargo test」等已被后续轮次推翻的段落；验证口径以 [混合验证说明](../agents/tauri-rust-ci.md) 为准（本机定向 + 云端全量），安装/卸载/打包类操作仍逐次取用户确认。
- **上轮（2026-10-25 第136次）**：收掉上游 1.3.2 的最后一项源码级遗留。
  - **LLM 契约补齐**：`tauri-app/src/shared/ipc-channels.ts` 的 `LLMResponseBase` 新增可选 `httpFailure?: { status: number; body: string }`；Rust `llm::chat::LlmResponse.http_failure` 早已序列化该字段，此前 Tauri 契约副本缺声明属漂移。生成链仍按 `error` 判定，该字段只作连接测试证据，不改变成功语义。
  - **shared 注释增量**：`character-proposal-parser`(3)、`import-run`(2)、`legacy-roster-generation-pure`(1)、`plot-tree`(8) 共 14 处 `// i18n-exempt:` 与上游逐字同步，纯注释。
  - **缺口复核**：逐文件比对 `src/`(466) 与 `tauri-app/src/`(461)——相对路径缺口仅 8 个 Storybook 故事 + 2 个已按死代码审计删除的服务；175 个同名差异抽样为「类型走 `@baseline`」「覆盖说明/恢复信号按 Tauri 语义重写」等刻意差异化，非未迁移。功能迁移 **287/288、未迁移 0**。
  - **验证（定向）**：`check:channels` 288 invoke / 7 事件 / 未迁移 0；`typecheck`/`lint` exit 0；`pnpm test src/shared` **32 文件 / 540 通过 / 0 失败**。零 Rust 改动，Rust 全量基线仍为 `6ba2bc8d` / CI 39982608773（1752 通过 + all-targets），未记作该轮结果。详见[快照](./2026-10-25-tauri-migration-status.md)。
- **上轮收口（2026-10-23）**：仓库 `master` = `origin/master` = `56abaa4d`，零未推送。
  - **上游同步**：`7e6b827f` → `305ee755`（124 提交 / 27 PR / 274 文件），合并 `c4583267`，零冲突（合并前本地 1190 改文件与上游 274 改文件交集 0）；根依赖随上游升 vite 8.2.4 / Storybook 10.6.2 / js-yaml 4.3.2 + source-map-js override，三份根清单 install 后未被改写。详见[快照](./2026-10-23-upstream-v132-sync.md)。
  - **beta.2 原生拉起**：安装目录主程序 SHA256 与 beta.2 MSI 一致；无包身份 `lorekeeper.exe` 1 秒内交棒 `lorekeeper-activate` + identity-launcher、以包身份重启并加载 `http://tauri.localhost/`；仅发 `WM_CLOSE` 时三进程 1 秒内自行退出；数据根无新写入、启动器日志落 `%TEMP%`。首轮 CDP `/json/close` 导致的「不退出」经对照复验确认为验证手段干扰。
  - **诊断与失败文案**：`54486fc7` 蓝图恢复稿接 `structuredContractDiagnosticText`；`e3ead6b1` 模型输出 3 处失败文案改可读摘要、技术码进收据；`character-card-normalizer` 的 9 处中文 `throw` 经核查无生产调用面。详见[诊断审计](../plans/2026-10-23-structured-diagnostic-display-audit.md)。
  - **httpFailure 契约**：`e44adbde` Rust 新增 `HttpFailure{status,body}` 并透传，连接测试按界面语言写状态码与服务返回；刻意保留原 `error` 文案以免影响 `llm:generate`。云端 Tauri Rust CI [39982608773](https://github.com/TanCan1998/Lorekeeper/actions/runs/39982608773) **1752 通过 0 失败**、all-targets 通过，Rust 累计切片清零。
  - **死代码清理**：审计 1932 个导出（87 零引用 / 66 仅测试）；A 档 `7a4a8019` + `52ec598c` + `e93cc108` 共删 50 个导出、2 个整文件、783 行；复扫零引用 87 → 43（余 17 个代码类全在 B 档，未动）。详见[导出审计](../plans/2026-10-23-dead-export-audit.md)。

- **更早维护**：[第125次剩余迁移合并](./2026-10-19-tauri-migration-status.md#本次更新第125次剩余迁移合并实现已提交云端验证通过)（云端 `011b4b0f` 全 success；MSI 已构建并完成静默安装/修复/卸载与真实快捷方式启动验收，快捷方式与安装后启动均指向 `lorekeeper-activate.exe` 且无 `LAUNCHAPPARGS`）；[第126次产物瘦身与依赖裁剪](./2026-10-22-tauri-migration-status.md)；[AtomGit 独立镜像](../agents/atomgit-mirror.md)（2026-10-21 起改为平台拉取代码 + 附件单独上传）。

- **最近发布**：[2026-10-21 上游合并与 beta.2 发布](./2026-10-21-upstream-merge-and-release.md#2026-10-21-后续beta2-发布)。用户最终确认保留 beta.1，只新发 `1.0.0-beta.2`（普通 Release + Latest、三平台四包、双源），不删除或覆盖旧版。**GitHub 已发布**：源码 `058ec936`，Rust CI `39555957392` 全量 **1750 通过**及 all-targets 通过，打包发布 `39556656055` success；Release ID `419244854`、九附件、Latest 与标签 SHA 已核验。AtomGit 已平台拉取同一标签/源码并独立上传九附件、匿名读回大小/SHA256；beta.1 保留。beta.2 实际重启链路与数据根只读已验收；多用户 UAC、macOS 真机与真实业务仍未认证。

- **当前基线**：`tauri-app` typecheck/lint 0、`check:channels` 288/7/未迁移 0、`test:contract` 51 通过；全量 vitest **227 文件 / 2833 通过 / 1 跳过 / 0 失败**（2026-10-23 实测，其后仅图标与 shared 改动，未重跑全量）；根 typecheck/lint/`check:i18n` 0，根 vitest 6173 通过 + 17 条环境类失败（symlink 权限、并发超时、macOS 夹具缺失、fork README 与 `AGENTS.md` 既存断言，已逐条归因）；Rust 全量基线 **`9eb064a8` / CI 40160190800：1752 通过 0 失败 + all-targets**（2026-10-25，切片累计清零）。

- **未解除阻塞**：
  - **需真机/后续包**：卸载与重装验收、NSIS 原生安装包验收、macOS 真机；小尺寸（16–32px）图标辨识度需专用简化稿（当前是整图缩放）。
  - **发布前提醒**：开发树 `wix.version` 已是 1.0.3.0（beta.2 发布时是 1.0.2.0），下次发布必须继续递增，且不得低于应用版本修订号；`assertWixVersion` 门禁会拦截违规。
  - **缺外部条件**：beta.2 多用户 UAC（需第二个登录账号）、旧版升级路径（需旧版安装包）。
  - **未授权**：真实模型 / MCP / 业务云验收。
  - **待维护者决策**：[Issue #515](https://github.com/TanCan1998/Lorekeeper/issues/515) DSH 插件 2 条 critical 告警（本地 HTTP 控制面可绕过认证，修复版 npm 不存在）+ 11 条 devDependency/锁文件告警。
  - **按审计结论不动**：B 档 17 个契约常量/判据函数、C 档 66 个仅测试引用。
- **下一步候选**：①卸载/重装与 NSIS 包验收（逐次确认）；②小尺寸图标简化稿；③下次上游同步；④provider 失败文案码化（对齐上游 a71bbcb3，需同时动 Rust 分类、契约与展示面）；⑤`tauri-app` 全量 vitest 复核；⑥清理 `src/commands/generation.rs:66` 的未使用导入告警。

- 禁止子代理、抢焦点、修改 Electron/根构建锁文件或上游 `docs/`；本轮未新增依赖、未改系统配置或文件权限。
