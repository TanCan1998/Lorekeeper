# Tauri 迁移进度快照（2026-10-25）

> 当前入口 CURRENT.md；规则见 ../agents/pi-development.md §9 与 ../agents/orchestration-policy.md。

## 已验证状态（第 136 次）

| 指标 | 结果 | 验证版本/范围 | verifiedAt | 证据路径 |
|---|---|---|---|---|
| invoke 契约/注册/覆盖/未迁移 | 288 / 289 / 287 / **0**，命令名与契约一一对应 | HEAD 工作区（本轮 shared 改动未提交时） | 2026-10-25 11:33 +08 | `pnpm run check:channels` 输出（exit 0） |
| 事件覆盖 | 7 | 同上 | 2026-10-25 11:33 +08 | 同上 |
| 前端 typecheck / lint | 0 / 0 | `tauri-app` 全量 | 2026-10-25 11:33 +08 | `pnpm run typecheck`、`pnpm run lint`（exit 0） |
| 定向 vitest | 32 文件 / **540 通过** / 0 失败 | `pnpm test src/shared` | 2026-10-25 11:33 +08 | 同上 |
| 安装器/发布契约 `node --test` | **51 通过 / 0 失败** | 五个 `*.node-test.mjs`（installer-contract、release-helpers、windows-identity、artifact-retention、atomgit-release） | 2026-10-25 11:44 +08 | `npm run test:contract`（EXIT=0） |
| Rust 全量 | 未重跑（本轮零 Rust 改动） | 基线 `6ba2bc8d` / CI 39982608773：1752 通过 + all-targets | 2026-10-23 | 引用原验证，不记为本轮结果 |
| GUI/双栈隔离 | 未做 | — | — | 本轮不涉界面与运行时 |

## 本次更新

- taskId 136：**对齐上游 1.3.2 遗留的 shared 增量，并补齐 LLM 响应契约**（上轮 CURRENT「遗留」明列的最后一项源码级事项）。
- 契约：`tauri-app/src/shared/ipc-channels.ts` 的 `LLMResponseBase` 补 `httpFailure?: { status: number; body: string }`。Rust `llm::chat::LlmResponse.http_failure` 早已序列化该字段（`#[serde(skip_serializing_if)]`），Tauri 契约副本缺声明属既有漂移；生成链仍按 `error` 判定，`httpFailure` 只作连接测试证据，故为可选字段，不改变成功判定。
- 注释增量：`character-proposal-parser`（3）、`import-run`（2）、`legacy-roster-generation-pure`（1）、`plot-tree`（8）共 14 处 `// i18n-exempt:` 与上游逐字同步，说明这些中文 `throw` 只在调用处被改写或仅进日志、作者看不到。仅注释，无行为变化。
- 判定依据：逐文件比对 `src/`（466 文件）与 `tauri-app/src/`（461 文件）——相对路径缺口只有 8 个 Storybook 故事与 2 个已按死代码审计删除的服务；175 个同名文件差异抽样核对为「类型走 `@baseline` 复用」「覆盖说明/恢复信号按 Tauri 语义重写」等刻意差异化，非未迁移。
- 未改：Rust 代码、schema、依赖、锁文件、权限；未重打包、未发布、未改 `docs/`、未派发子代理。

## 本次更新（第 137 次）

- **契约测试有了自动入口**：`tauri-app/package.json` 新增 `test:contract`，一次跑完五个 `node --test` 契约脚本；新增 `.github/workflows/tauri-contract-ci.yml`（fork 独有、Node-only、windows-2022、只读权限），路径过滤 `tauri-app/scripts/**`、`src-tauri/installer/**`、`tauri.conf.json`、`package.json` 与自身。
- **背景**：这批脚本（安装器合同 13 项、wix 版本门禁、身份清单、产物保留、发布附件校验）此前只在 [发布说明](../agents/tauri-release.md) 写着手工命令，`pnpm test`（vitest 默认匹配 `*.test.mjs`）与现有 CI 都不覆盖，属长期漏跑风险；CURRENT 的「新增回归用例未执行」由此销号。
- **交接口径纠偏**：CURRENT 删除「迁移资源操作仍暂缓（不编译/测试/打包/安装/卸载/提权）」「MSI 仅源码修改、未构建未安装」「等最终验证授权后本机一次性跑 cargo test」等已被后续轮次推翻的段落，改为按 [混合验证说明](../agents/tauri-rust-ci.md) 记录口径，并把未解除阻塞归成「需重打包/真机」「缺外部条件」「未授权」「待决策」「按审计不动」五类。
- 未改：Rust、schema、依赖、锁文件、权限；未重打包、未发布、未派发子代理。

## 本次更新（第 138 次）：重打包与覆盖安装验收

- **版本门禁**：`tauri.conf.json` 的 `bundle.windows.wix.version` 1.0.2.0 → 1.0.3.0。同版本 MSI 会被 Windows Installer 视作修复而不替换文件，覆盖验收必须递增；`test:contract` 的 `assertWixVersion` 用例在该版本下 51 项全通过。
- **构建**：本机 `pnpm tauri build --bundles msi,nsis`（Rust release 增量 3m55s，产物见下表）。构建告警：`src/commands/generation.rs:66` 未使用导入 `ProjectCoreSynopsisExpected`（既有，未处理）。
- **覆盖安装**：`msiexec /i … /qn /norestart` 退出码 0；ARP 由 1.0.2.0（`{0D60D879-…}`）升级为单条 1.0.3.0（`{88025DBA-…}`），未出现并存安装；Appx 重注册 1.0.3.0；桌面与开始菜单快捷方式目标均为 `D:\Program Files\LoreKeeper\lorekeeper-activate.exe` 且无参数。
- **图标**：MSI 的 `ProductIcon` 与 exe 资源均为新星芒；安装后 exe 的 32px 图标与 `icons/icon.ico` 32px 帧逐像素一致（差异 0.0），与旧 PageKin 图标差异 132.27。**遗留设计问题**：16/24/32px 帧由整图缩放生成，「司」与星芒在极小尺寸糊成文档状，建议后续做小尺寸专用简化稿。
- **启动与关闭**：真实桌面快捷方式启动 → 三进程链正常，窗口标题 `LoreKeeper`，CDP `Lorekeeper 设定司` / `http://tauri.localhost/`；`WM_CLOSE` 后 2.04s 内全部退出；数据根无新写入；全程未置顶、未激活、未用键鼠。
- **云端全量（新基线）**：提交 `9eb064a8` 上 Tauri Rust CI [40160190800](https://github.com/TanCan1998/Lorekeeper/actions/runs/40160190800) **1752 通过 0 失败**（260.83s）+ all-targets 通过；同 SHA 的 Tauri contract tests、Commit message CI、Gitleaks 均 success。Rust 全量基线由 `6ba2bc8d` 更新为 **`9eb064a8`**，切片累计清零。
- **未做**：卸载/重装、NSIS 原生安装、多用户 UAC、旧版升级、macOS 真机；未发布。
- 证据：`.pi/tmp/repack-2026-10-25/{evidence.json,build.log,build.err.log,install.log,exe-icon.png,installed-exe-icon.png}`（不入库）。

## 本次更新（第 139 次）：16–32px 图标专用帧

- **问题**：安装核验时发现整图缩放出的 16/24/32px 帧与旧 PageKin 图标肉眼难分——「司」与四色星芒在极小尺寸糊成文档状。
- **方案**：新增 `public/branding/app-icon-light-small.svg`（星芒改四块实色并放大到 `scale(3.2)`，「司」放大 1.1 倍，去掉径向渐变）；`scripts/compose-icon-frames.mjs` 把两套源图生成的 ICO 按尺寸拼成一份，`icon.ico` 的 16/24/32 取简化稿、48/64/256 取完整稿，`icons/32x32.png` 同步换用简化稿。
- **可重复性**：`pnpm icons:compose`，组合结果与提交的 `icon.ico` 逐字节一致（SHA256 `821cf027…`）。新增 `scripts/icon-frames.node-test.mjs`（5 项，覆盖解析/组合/缺帧回落/坏帧拒绝），`pnpm test:contract` 由 51 增至 **56 通过**。
- **重打包核验**：重建 MSI/NSIS 后，exe 的 32px 图标与新 `icon.ico` 32px 帧**逐像素一致（差异 0.0）**，与旧帧差异 23.6；说明打包链确实取用简化帧。
- **验证**：`typecheck`/`lint` 0；`app-icon.test.ts` **11 通过**（含小尺寸源图摘要、无渐变、四色实色、放大倍数断言）。
- **云端**：`5bdaf7b7` 上 Tauri Rust CI [40161780975](https://github.com/TanCan1998/Lorekeeper/actions/runs/40161780975) **1752 通过 0 失败** + all-targets、contract tests、Commit message CI、Gitleaks 全 success。
- **未做**：本机已装的 1.0.3.0 仍是旧帧；要看任务栏/资源管理器效果需把 `wix.version` 升到 1.0.4.0 再重装（未执行）。

## 本次更新（第 140 次）：1.0.4.0 重装并确认小图标生效

- **版本门禁**：`wix.version` 1.0.3.0 → **1.0.4.0**（同版本 MSI 只走修复，无法刷新已装文件）；`test:contract` 56 通过。
- **构建**：本机 `pnpm tauri build --bundles msi,nsis`，MSI 53,927,936 B / NSIS 50,162,216 B。
- **覆盖安装**：`msiexec /i … /qn /norestart` 退出码 **0**；ARP 由 1.0.3.0 升为单条 **1.0.4.0**（ProductCode `{163F9777-…}`，无并存）；Appx 重注册 1.0.4.0；桌面与开始菜单快捷方式仍指向 `lorekeeper-activate.exe` 且无参数。
- **图标生效**：安装后 `lorekeeper.exe` 的 32px 图标与新 `icon.ico` 32px 简化帧**逐像素一致（差异 0.0）**，与上一版整图缩放帧差异 23.6。
- **启动与关闭**：经桌面快捷方式拉起，三进程链正常，窗口标题 `LoreKeeper`，CDP `Lorekeeper 设定司` / `http://tauri.localhost/`；`WM_CLOSE` 2.04s 内全部退出；数据根无新写入；未置顶、未激活、未用键鼠。
- **提醒**：开发树 `wix.version` 现为 1.0.4.0，下次发布须继续递增。

## 本次更新（第 141 次）：上游 2026-10-25 同步并回灌 Tauri 副本

- **上游**：`305ee755` → `05f180f0`（151 提交 / 123 文件 / +6659 −1350，PR #507/#509/#510/#511/#514 等），主题是定稿后处理步骤错误码与重试边界、reasoning policy / provider presets、verify 快照稳健性。
- **合并**：分支 `chore/merge-upstream-20261025`，合并零冲突（本地根目录改动 17 个文件与上游 123 个文件交集 0）；`package.json` / `pnpm-lock.yaml` / `.npmrc` / `vite.config.ts` 上游均未改，**无需装包**。已快进合入 `master` 并推送，最终 `23f6899e`。
- **根验证**：`typecheck` 0、`lint` 0、`check:i18n` 0。
- **Tauri 回灌**：45 个非测试文件中 **37 个已回灌**（13 个逐字节相同直接套补丁，其余按 hunk 上下文与「取上游版本 + 重贴 fork 差异」处理），另套用 7 个镜像测试。要点：新增 `containsHan()` 统一汉字判断（替换 14 个 agent 工具的裸正则与 ShelfBook 的 CJK 常量）、`pluralEn` 处理英文复数（10 处）、`provider-presets` 取上游版本（Gemini 3 系列预设、retiredModels、本地端点判定）并把数据根改回 `~/.lorekeeper`、角色当前状态按字段三方合并（`character-rename-ledger`）、两个新的角色状态冲突码接上可读文案。
- **Tauri 验证**：`typecheck` 0、`lint` 0、`check:channels` 288/7/未迁移 0、`test:contract` 56 通过、全量 vitest **227 文件 / 2863 通过 / 1 跳过 / 0 失败**（合并前 2833，净增 30）。
- **云端**：分支 `23f6899e` Rust CI [40169235788](https://github.com/TanCan1998/Lorekeeper/actions/runs/40169235788) **1752 通过 0 失败** + all-targets success；Tauri contract tests、Gitleaks 亦 success。
- **未回灌（后续切片）**：8 个文件——`ModelForm.tsx`（模型发现重构，需与 `ipc-channels` 的 discovery 契约和新增 `ModelNameField.tsx` 一起搬）、`chapter-workflow.ts`（按当前状态重算可重试步骤，需先给 `finalize-chapter.command` 加 `stepKeys` 参数）、以及 5 个依赖 `displayedErrorMessage` 的「显示时取语言」改造（`CharacterCardImportButton`、`use-draft-actions`、`use-run-recovery`、`use-blueprint-recovery`、`use-knowledge-base`）——Tauri 侧无该函数，需先定等价收敛方案。
- **分支清理**：删除已合并的本地 `chore/merge-upstream-20261021`、`chore/merge-upstream-20261025` 与远端 `origin/chore/merge-upstream-20261025`；现在只剩 `master`。

## 本次更新（第 142 次）：报错文案改在显示时取语言（A2）

- **新增 Tauri 侧等价辅助**：`i18n/app-errors.ts` 补 `displayedErrorMessage(locale, error, fallback?)`、`englishWithReason(summary, reason)` 与 `DISPLAY_PREFIX`（去掉调用通道与 `Error:` 类名前缀）。语义与上游一致：登记过的码按语言说明，认不出的保留原文，完全空时用调用方兜底。
- **5 个文件改为显示时取语言**：`CharacterCardImportButton`、`use-draft-actions`、`use-run-recovery`、`use-blueprint-recovery`、`use-knowledge-base`——把订阅来的 `locale` 换成 `useLocaleStore.getState().locale`，并从依赖数组移除，结果晚到时也能按切换后的语言显示。
- **验证**：`typecheck` 0、`lint` 0、全量 vitest **227 文件 / 2863 通过 / 1 跳过 / 0 失败**。提交 `0c997e55`，无 Rust 与依赖变更（不触发 Rust CI）。
- **A1 本轮回滚（结论记录）**：`chapter-workflow` 的「按当前状态重算可重试步骤」+ `finalize-chapter.command` 的 `stepKeys` 参数本身可编译，但上游把重试语义与 `RunFinalizePostProcessCommand` 的 `explicit` 映射重构、`#502/#512` 的提交期数据变化判定绑在一起；只搬一半会让 `repair-finalize-workflow`、`finalize-postprocess-character.integration` 两个契约测试（Tauri 副本为旧断言）失败。已整体回滚，登记为需要连同上游 finalize 重构一起搬的独立切片。

## 本次更新（第 143 次）：模型发现按用途筛选 + 模型名候选控件

- **Rust（`llm/discovery.rs`）**：`ModelDiscoveryErrorCode` 把笼统的 `empty` 拆成 **`no_models`**（端点一个模型都没列）与 **`no_suitable_models`**（列了模型但没有适合当前用途的）；新增 `DiscoveryPurpose`，从请求的 `purposes` 判断用途：Gemini 按 `supportedGenerationMethods` + 命名黑名单（tts/image/live/transcribe/audio/lyria/nano-banana/computer-use/deep-research/antigravity/robotics）筛，OpenAI 兼容接口按名字认向量模型并排在前面。保留 `parse_gemini_models` / `interpret_discovery_response` 兼容入口，旧测试不用改签名。
- **契约**：`ipc-channels` 的 `ModelDiscoveryErrorCode` 同步为两档，`ModelDiscoveryRequest` 加 `purposes`。
- **前端**：新增 `components/settings/ModelNameField.tsx`（可输入 + 候选列表的 combobox，含键盘导航、输入法合成保护、刷新与状态行）；`ModelForm` 改用该控件 + `ModelDiscoveryState` 状态机（端点多停 500ms 自动取列表，旧列表在手动刷新时保留）+ 六档提示文案。
- **验证**：Rust `cargo test --lib llm::discovery` **14 通过**（含 2 个新用例：无模型/无适合模型区分、检索模型排序），定向 rustfmt 通过；`typecheck`/`lint` 0、`check:channels` 288/7/未迁移 0、`test:contract` 56、全量 vitest **227 文件 / 2863 通过 / 1 跳过**。
- **云端**：`5337b3a2` 的 Rust CI [40171928682](https://github.com/TanCan1998/Lorekeeper/actions/runs/40171928682) **1754 通过 0 失败**（+2 为新用例）+ all-targets success；Commit message CI、Gitleaks success。
- **未验证**：settings 的浏览器测试本轮没跑——本机 Playwright chromium 在 10-22 瘦身时被清掉，`pnpm test:browser` 报可执行文件缺失。需要时先 `pnpm exec playwright install chromium`（会下载，属新装工具，另行确认）。
- **同轮附带**：清掉 `commands/generation.rs` 未使用的 `ProjectCoreSynopsisExpected` 导入（`e54bc7a5`）。

## 本次更新（第 144 次）：后处理重试按当前状态重算（补完 A1）

- **源码取上游**：`finalize-chapter.command.ts` 与 `chapter-workflow.ts` 整份换成上游版本——引入 `stepKeys`（替代单 `stepKey`，旧版本遗留步骤不再挡住重试）、`explicit` 按步骤映射的重试来源、成功后按角色卡刷新 `characterCards` 资源；`createRepairFinalizeWorkflow` 重试前先 `readPostProcessStatus` + `planFinalizationRetry`，没有可重试步骤时明确报错，而不是空跑一次。
- **补齐被依赖的缺口**：
  - Rust `finalization_recovery` 新增 `latestAttempt`（最近一次物理尝试的 id、是否在飞、结束原因、失败码）——上游 `explicit` 逻辑靠它决定 `recoverFrom`；
  - `shared/finalization-generation.ts` 同步该字段；
  - `workflows/workflow-project-session.ts` 补 `workflowMainProcessError()`（登记码按语言说明、未登记码保留并挂 `code`）。
- **测试**：`repair-finalize-workflow.test.ts` 取上游版本（含 #502 的三个新场景），把 `vi.stubGlobal('window', …)` 换成 `installTauriChannelFixture`；`finalize-postprocess-character.integration.test.ts` 保留 Tauri 版并在 `command()` 里把 `stepKey` 桥接成 `stepKeys`，让旧断言继续成立。
- **验证**：`typecheck`/`lint` 0、`check:channels` 288/7/未迁移 0、定向 rustfmt 通过、Rust `llm::discovery` 14 通过、全量 vitest **227 文件 / 2870 通过 / 1 跳过**（较上一轮 +7）。
- **云端**：`2b8649fe` 的 Rust CI [40175871769](https://github.com/TanCan1998/Lorekeeper/actions/runs/40175871769) **1754 通过 0 失败** + all-targets；Commit message CI、Gitleaks success。
- **仍待验证**：settings 浏览器测试（缺 Playwright chromium）；`#512` 的四个「提交期数据变化」场景只在 Electron 侧上游测试里存在，Tauri 副本未搬（属上游 electron 服务行为，Rust 侧需另行对齐）。

## 交接给下次会话

- 分支 `master`，HEAD 为本快照所在提交；工作区仅既存未跟踪 `tmp-check/`。
- 当前状态入口：.pi/tmp/pipeline-state.md。
- 下一步候选：①上游新的 `src/`、`electron/` 增量的下一次同步（本轮已把 1.3.2 的 shared 尾巴收完）；②低层 provider 文案仍为中文句子（`API 调用失败 (status)` 等），上游 a71bbcb3 将其码化为 `LLM_REQUEST_FAILED` / `LLM_STREAM_*`，若要对齐需同时改 Rust 分类、契约与展示面，属独立切片，需先评估消费面；③beta.2 打包后才可见的桌面图标与安装入口验收。
- 未解除阻塞（来源 CURRENT.md）：beta.2 多用户 UAC、旧版升级、卸载/重装、NSIS 原生安装、macOS 真机、真实模型/MCP/云业务；B 档 17 个契约常量与 C 档 66 个仅测试引用按审计结论不动。
- 验证命令与范围见上表；本轮全部为 `tauri-app` 内定向验证，无 Rust 与根工程改动。
