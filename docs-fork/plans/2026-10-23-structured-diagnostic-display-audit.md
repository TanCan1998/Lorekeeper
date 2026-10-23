# 结构化合同诊断的显示面审计（2026-10-23）

只读审计，未改动代码。目的是弄清 `StructuredContractDiagnostic` 从抛出到作者可见的完整路径，判断还有多少处会把 `code=… path=… field=…` 直接暴露。

## 一、抛出点分布（非测试文件）

| 文件 | 抛出数 | 用途 |
|---|---|---|
| `tauri-app/src/shared/blueprint-semantic-contract.ts` | 32 | 蓝图语义契约解码（模型输出） |
| `tauri-app/src/services/workflows/commands/import-inference-contract.ts` | 18 | 导入推断契约解码（模型输出） |
| `tauri-app/src/shared/character-proposal-parser.ts` | 13 | 角色身份清单/角色详情/素材抽取解码（模型输出） |

## 二、提取函数 `structuredContractDiagnostic(error)` 的消费点

| 位置 | 作用 | 是否显示给作者 |
|---|---|---|
| `components/stage/blueprint/use-blueprint-recovery.ts:31` | `failureText` → `structuredContractDiagnosticText` | **是（已修，`54486fc7`）** |
| `services/workflows/directory-workflow.ts:163` | 保留诊断类型继续上抛 | 间接 |
| `services/workflows/structured-batch-executor.ts:403` | 决定拆半重试 / 紧凑单项回退，并把诊断塞进 `StructuredBatchFailure` | 间接 |

## 三、作者可见路径分类

### A. 作者手稿（作者可以改）

- 蓝图恢复稿三条失败路径 —— 已用 `structuredContractDiagnosticText`，状态正确。
- 未发现其它解析作者手写文本的诊断入口：`character-card-normalizer` 虽然解析 AI 输出/角色名单，但抛的是中文 `Error`（不带诊断码），归入第四节的附带发现。

### B. 模型输出（作者不能改，只能重试）

| # | 位置 | 语言 | 当前显示 | 判定 |
|---|---|---|---|---|
| B1 | `commands/directory.command.ts:99-122` `DirectoryBlueprintContractError`（由 447 抛出） | 中/英 | `结构化合同诊断 code=invalid_value path=blueprints[3].title field=title`（英文同构 `Structured contract diagnostic …`） | **技术串暴露** |
| B2 | `commands/import-novel.command.ts:741` | 中文 | `蓝图推演失败：结构化合同诊断 code=… path=… field=…` | **技术串暴露**；英文分支只用 `reason`，是干净的 |
| B3 | `commands/architecture.command.ts:1188` | 中文 | `结构化合同诊断 code=…；角色详情失败收据：code=… reason=… diagnosticCode=…` | **技术串暴露**；英文分支是通用句 + 收据 |

已确认**安全**的两处：
- `directory.command.ts:454-455`：有诊断时先走 B1 抛出，该分支只会拿到「结构化输出无法按合同解码」这类通用消息。
- `planning-material.command.ts:118`：只用 `failure.reason`。

## 四、建议（未执行）

1. **不要**把 `structuredContractDiagnosticText` 直接套到 B1–B3。那个函数是写给「作者手写内容不合契约」的（「内容格式不对：blueprints[3].title 缺少这个字段。」），对模型输出会误导——作者改不了模型返回的 JSON。
2. B1–B3 应改成「生成失败 + 可读摘要」，技术码只进 `callbacks.log` 的诊断收据（现在收据已经在写，见 `generationSummary` / `failureReceipt`）：
   - 中：`模型返回的蓝图结构不合要求，已保留原有内容。可重试；技术诊断见本次运行的诊断日志。`
   - 英：`The model returned blueprints that do not match the expected structure; existing content was kept. Retry, or check this run's diagnostics.`
3. 改造会影响 4 处既有断言，需同步：
   - `services/workflows/__tests__/directory-workflow.test.ts:250,261,269`（断言底层解析的 `结构化合同诊断 code=…`，属**契约层**，改造 B1–B3 不必动它们）
   - `services/workflows/commands/__tests__/directory.command.test.ts:1309`（断言 `failure.message` 含 `Structured contract diagnostic`，**改造 B1 时必须一起改**）
4. 优先级：B2、B3 只动中文分支与英文保持对称，风险低；B1 需要连带调整 `DirectoryBlueprintContractError` 的 message 生成并保留 `diagnostic` 字段（上层仍可能要判类型）。

## 五、附带发现（不在本审计范围）

- `services/workflows/character-card-normalizer.ts` 有 9 处 `throw new Error('中文…')`（如「角色卡包含重复角色名，未写入角色列表」）。这些不带诊断码，属**双语文案覆盖**问题而非诊断显示问题；是否显示给作者取决于上游包装，需另立切片确认。

## 六、未覆盖

- 只做静态路径追踪，未逐一构造失败场景做 UI 实测。
- 未审计 Rust 侧错误文案（`llm_*`、`db_*` 等），本轮只针对 `StructuredContractDiagnostic`。
- 未统计 `*.browser.tsx` 里对这些文案的断言（4 处均为 node 侧 vitest）。
