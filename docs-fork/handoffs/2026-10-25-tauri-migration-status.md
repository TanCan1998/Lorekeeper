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

## 本次更新（第 145 次）：设置页浏览器测试跑通并修正口径

- **发现**：`vitest.browser.config.ts` 支持 `AI_NOVEL_VITEST_CHROMIUM` 指定现成浏览器，本机 Edge 156 可直接复用——**不需要下载 Playwright chromium**（10-22 瘦身清掉的只是那份缓存）。已把用法写进[开发指南 §7](../agents/pi-development.md)。
- **首跑结果**：77 文件 / 943 用例中 9 失败，全是前两轮改动的口径没跟上：`model-discovery.browser.tsx` 8 个（还在按旧模型名输入框与旧 `invoke` 参数形状断言）、`advanced-model-settings.browser.tsx` 1 个（`Effective output allowance` 措辞已变）。
- **修正**：`model-discovery.browser.tsx` 取上游版本（combobox locator、`purposes` 参数、`no_models`/`no_suitable_models` 两档文案），并把 Electron 的 `window.aiNovelAPI` 赋值换成 `installTauriChannelFixture`；`advanced-model-settings` 断言改为 `/Effective output allowance: 6,000\. /`。
- **结果**：全量浏览器套件 **77 文件 / 983 用例全通过**（exit 0）。
- **已知偶发**：`AgentMessage.browser.tsx` 在整包并发下偶发 act 超时（单文件复跑通过），已在交接注明，不作为回归。
- **未提交真实失败**：本轮只改测试与文档，无产品代码变更，未触发 Rust CI。

## 本次更新（第 146 次）：#512 提交期数据变化的覆盖补齐

- **先核对产品面**：Rust `finalized_character_service` 已在提交期返回 `FINALIZED_CHARACTER_SOURCE_CONFLICT` / `FINALIZED_CHARACTER_FIELD_CONFLICT`，`shared/finalization-step-error.ts` 与上游逐字一致（四个码的可读说明 + `retry: described / never` + `planFinalizationRetry` 的 `retired` 分组），`PostProcessSteps.tsx` 已按 retired 呈现、不摆重试、不计入完成数，`app-errors.ts` 已映射两条新码。**产品面无需改动**，缺的是覆盖。
- **补的两处**：
  - `i18n/__tests__/app-errors.test.ts` 增加两条提交期码的端到端断言（中文说明含「当前状态」「生成期间」，且不再漏出 `FINALIZED_` 码）；
  - `mutation-failure-boundary.test.ts` 补 #512 的重试计划用例：现在流程里的步骤都能重试，`style_analysis` 这类旧步骤进 `retired` 而不进重试。
- `chapter-lifecycle.browser.tsx` 的四个 #512 场景（删角色提交失败、作者改要点、旧步骤仍列出且不摆重试、旧步骤当时成功也不计入完成数）此前已移植，本轮浏览器套件复跑全绿予以确认。
- **验证**：`typecheck`/`lint` 0、全量 vitest **227 文件 / 2873 通过 / 1 跳过**（较上轮 +3）；浏览器套件本轮未重跑（仅改单测）。
- **剩余**：真机类（卸载/重装、NSIS、多用户 UAC、旧版升级、macOS）与 Issue #515 依赖告警决策。

## 本次更新（第 147 次）：旧版升级验收（1.0.1.0 → 1.0.4.0）

- **起点**：本机已装 1.0.4.0。先试「装回 beta.2」，结果只走维护模式（退出码 0、无变更）——**本地 `bundle/` 里的 MSI 文件名只含应用版本（`LoreKeeper_1.0.0-beta.2_*.msi`），两次重打包已把内容换成 1.0.3.0/1.0.4.0，原 beta.2(1.0.2.0) 被覆盖**。要留旧包必须另存副本。
- **降级**：改装 `beta.1`（1.0.1.0）代替——退出码 0，ARP 变 1.0.1.0、`lorekeeper.exe` 回到 10-20 版本。**但 Appx 身份仍是 1.0.4.0**：Windows 不允许 Appx 降级注册，所以「降级安装」只降文件不降包身份。
- **升级**：再装 1.0.4.0——退出码 0；ARP 单条 **1.0.4.0**（`{163F9777-…}`）；`lorekeeper.exe` 79,958,528 B / 13:07 刷新；`lorekeeper-activate.exe` 同步更新；桌面与开始菜单快捷方式均指向 `lorekeeper-activate.exe` 且无参数、图标取自当前 ProductIcon；Appx 1.0.4.0。
- **图标**：升级后 exe 的 32px 图标与新简化帧**逐像素一致（0.0）**，与 beta.1 图标差 25.12。
- **运行**：经桌面快捷方式启动 → 三进程链、标题 `LoreKeeper`、CDP `Lorekeeper 设定司` / `http://tauri.localhost/`；仅发 `WM_CLOSE` 后 2.05s 全退；`~/.lorekeeper`(10-23)、`~/.vela`(10-16) 无新写入。
- **结论**：跨版本升级路径（旧→新）干净；降级安装不受支持且会让 Appx 与文件版本不一致，属 Windows 限制，发布说明里应说明「不支持降级，需卸载后重装」。
- **Issue #515**：确认它在上游仓库（`EthanYoQ/AI-Novel-Writer`），本 fork 的 issues 已关闭，故不改上游 issue；按用户决定记入本仓库交接——**本项目不迁移该 DSH 插件**，其 critical 告警不处理，将来若纳入维护再移植到 DSH ≥0.1.2-rc.1。
- 证据：`.pi/tmp/upgrade-2026-10-25/{evidence.json, install-beta2.log, install-beta1.log, upgrade-to-1.0.4.0.log, *.png}`（不入库）。

## 本次更新（第 148 次）：卸载/重装 + NSIS 安装包验收

- **链路**：卸载 MSI 1.0.4.0 → 装 NSIS 包 → 启动/关闭 → 卸 NSIS → 重装 MSI → 启动/关闭（每步退出码 0）。
- **MSI 卸载干净**：ARP 清除、Appx 0 个、`D:\Program Files\LoreKeeper` 目录删除、公共桌面与开始菜单快捷方式删除；`~/.lorekeeper`(10-23)、`~/.vela`(10-16) 无写入。
- **NSIS（currentUser）**：**不需要提权**，12s 装完，落 `%LOCALAPPDATA%\LoreKeeper`（`lorekeeper.exe` 79,958,528 B、activator、`uninstall.exe`、`identity/`）；ARP 在 HKCU，显示应用版本 `1.0.0-beta.2`；用户级开始菜单与桌面快捷方式均指向 activator 且无参数。
- **按需注册生效**：NSIS 安装后 **Appx 为空**，首次启动由激活器注册成功（`1.0.4.0`）；三进程链、标题 `LoreKeeper`、CDP `Lorekeeper 设定司`；`WM_CLOSE` 2.03s 全退。
- **NSIS 卸载**：ARP、快捷方式、Appx 全部清理，程序文件删除；**残留 `identity-launcher.log`（1,110 B 诊断日志）**；数据根未变。
- **MSI 重装**：回到默认目录 **`C:\Program Files\LoreKeeper`**（不记忆上次自定义的 `D:\Program Files`），ARP 单条 1.0.4.0、Appx 1.0.4.0、快捷方式指向 activator、32px 图标与新简化帧**逐像素一致（0.0）**；启动与关闭正常（2.04s）。
- **四条写入指南的事实**：①NSIS 装完 Appx 为空、首次启动按需注册；②NSIS 卸载保留启动器日志；③MSI 卸载再装回默认目录；④NSIS 的 ARP 版本是应用版本，两套包产品项独立、不应并存安装。后两条已补进 [安装指南](../agents/tauri-release.md)，「不支持降级」也写在那里（beta.2 发布说明属已发布记录，不改正文，下个版本带上）。
- 证据：`.pi/tmp/install-cycle-2026-10-25/{evidence.json, msi-uninstall.log, msi-reinstall.log, reinstalled-32.png}`（不入库）。

## 本次更新（第 149 次）：安装与首次启动不再闪控制台窗口

- **根因**：`identity-launcher` 自身是 GUI 子系统（`windows_subsystem = "windows"`），但它有 **3 处** `std::process::Command` 调用 `powershell.exe` 时没加 `CREATE_NO_WINDOW`，Windows 给子进程分配了控制台窗口。触发路径正是安装与首启：NSIS 钩子 `ExecShell "runas" … --register-identity`，以及首次启动按需注册。
- **本来就是隐藏的**：NSIS 钩子的其它 PowerShell 走 `nsExec::ExecToStack`；MSI 自定义动作走 `WixQuietExec`（`BinaryKey="WixCA"`）；Wix 的 WebView2 bootstrap 自带 `-windowstyle hidden`。所以只有激活器这一条会闪。
- **修复**：`identity-launcher/src/win.rs` 增加 `const CREATE_NO_WINDOW: u32 = 0x0800_0000`、导入 `std::os::windows::process::CommandExt`，3 处调用追加 `.creation_flags(CREATE_NO_WINDOW)`。
- **验证**（每 150ms 采样 `powershell/conhost/cmd` 的 `MainWindowHandle` 与标题，窗口可见即 hwnd≠0 且有标题）：
  - 启动器单测 `cargo test` **14 通过**；重新打包 MSI 53,997,568 B / NSIS 50,222,001 B。
  - NSIS 安装：退出码 0、采样 1274 次、**可见控制台窗口 0**；首启：采样 1800 次、**0**，Appx 按需注册 1.0.4.0、三进程链正常。
  - MSI 安装：退出码 0、采样 320 次、**0**，ARP 与 Appx 均 1.0.4.0。
- **机器现状**：已回到 MSI 1.0.4.0（新构建，含修复后的激活器）。
- **云端**：`aac0985d` 的 Tauri Rust CI **1754 通过 0 失败** + all-targets（含重编身份启动器）；Commit message CI、Gitleaks success。

- 证据：`.pi/tmp/console-hide-2026-10-25/{evidence.json, build.log, msi-install.log, msi-uninstall.log}`（不入库）。

## 本次更新（第 150 次）：本轮收口与完整本机基线

- **本机全量扫描**（HEAD `67718389`，`tauri-app/`）：
  - `typecheck` 0、`lint` 0、`check:channels` 288 invoke / 7 事件 / **未迁移 0**、`test:contract` **56 通过**；
  - 单测 **227 文件 / 2873 通过 / 1 跳过**；
  - 浏览器套件（Edge 复用 `AI_NOVEL_VITEST_CHROMIUM`）**77 文件 / 983 通过**。
- **云端**：`aac0985d` 的 Tauri Rust CI **1754 通过 0 失败** + all-targets；其后仅文档提交，基线仍有效。
- **上游**：`05f180f0` 已追平（领先 0），本轮无新提交。
- **本轮完成清单**（2026-10-25 共 15 轮）：LLM 契约补齐 → 上游 1.3.2 尾巴 → 图标两次放大 + 16–32px 专用帧 → wix 1.0.4.0 重打包与覆盖安装 → 上游 2026-10-25 增量（151 提交）合并与回灌 → 报错文案显示时取语言 → 模型发现按用途筛选 + 模型名候选控件 → 后处理重试按当前状态重算 → 设置页浏览器测试跑通 → #512 覆盖补齐 → 旧版升级验收 → 卸载/重装与 NSIS 验收 → 激活器隐藏控制台窗口。
- **未做**：多用户 UAC（用户明确先不做）、macOS 真机（无设备）、发布新版本（`wix.version` 已是 1.0.4.0，发布需另行授权）。

## 本次更新（第 152 次）：1.0.0-beta.3 已发布并完成发布产物验收

- **发布**：标签 `lorekeeper-v1.0.0-beta.3` → 源码 **`f39249d0`**（该 SHA 有 Rust 全量 success 记录）；Release id `423392746`，非草稿、非预发布、已设为 **Latest**；run [40192325548](https://github.com/TanCan1998/Lorekeeper/actions/runs/40192325548) 的 prepare / 三平台打包 / 发布作业全部 success。
- **九个附件**（4 包 + 3 平台清单 + release-manifest + SHA256SUMS）：
  - MSI 53,989,376 B（ProductVersion **1.0.5.0**，UpgradeCode 未变）；
  - NSIS 50,223,097 B；
  - macos aarch64 DMG 91,915,101 B；macos x64 DMG 91,918,117 B。
  - 下载 MSI 复算 SHA256 与 `SHA256SUMS.txt` **完全一致**。
- **发布产物装机验收**：从已装 1.0.4.0 安装发布版 MSI（退出码 0）→ ARP `1.0.5.0`、Appx `1.0.5.0`、桌面与开始菜单均指向 `lorekeeper-activate.exe` 且无参数、32px 图标与仓库 `icon.ico` 逐像素一致（0.0）、启动采样 1050 次**无可见控制台窗口**、CDP `Lorekeeper 设定司`、仅 `WM_CLOSE` 后 2.03 秒全退、`~/.lorekeeper`(10-23) 与 `~/.vela`(10-16) 无写入。
- **文档**：`docs-fork/releases/1.0.0-beta.3.md` 改为「已发布」并回填附件表与验收记录；清单文档保留预热记录。
- **未完成 — AtomGit**：平台尚未拉取该标签（`GET /releases/lorekeeper-v1.0.0-beta.3` 用 Bearer 认证返回 404），因此 `pnpm mirror:release` 未执行；待平台拉取后上传九附件并匿名读回大小/SHA256。
- **未验证**：多用户 UAC（用户先不做）、macOS 真机（无设备）。
- 证据：`.pi/tmp/beta3-release/{evidence.json, install-published.log, installed-beta3-32.png, SHA256SUMS.txt, release-manifest.json, LoreKeeper_1.0.0-beta.3_x64_zh-CN.msi}`（不入库）。

## 交接给下次会话

- 分支 `master`，HEAD 为本快照所在提交；工作区仅既存未跟踪 `tmp-check/`。
- 当前状态入口：.pi/tmp/pipeline-state.md。
- 下一步候选：①上游新的 `src/`、`electron/` 增量的下一次同步（本轮已把 1.3.2 的 shared 尾巴收完）；②低层 provider 文案仍为中文句子（`API 调用失败 (status)` 等），上游 a71bbcb3 将其码化为 `LLM_REQUEST_FAILED` / `LLM_STREAM_*`，若要对齐需同时改 Rust 分类、契约与展示面，属独立切片，需先评估消费面；③beta.2 打包后才可见的桌面图标与安装入口验收。
- 未解除阻塞（来源 CURRENT.md）：beta.2 多用户 UAC、旧版升级、卸载/重装、NSIS 原生安装、macOS 真机、真实模型/MCP/云业务；B 档 17 个契约常量与 C 档 66 个仅测试引用按审计结论不动。
- 验证命令与范围见上表；本轮全部为 `tauri-app` 内定向验证，无 Rust 与根工程改动。
