# Tauri 侧未使用导出审计（2026-10-23）

只读静态审计，未删除任何代码。

## 结论摘要

- `tauri-app/src` 非测试文件共 **1932** 个导出，其中 **87 个零引用**（生产与测试都不引用）、**66 个仅测试引用**。
- 零引用集中在迁移遗留的服务层：`knowledge-service`（11）、`draft-index`（6）、`version-service`（5）、`main-generation-transport`（5）、`style-presets`（3）、`chapter-workflow`（3），以及若干单点导出。
- **不建议一次性删除**。按下面三档处理：A 档可直接清理；B 档是契约类型/常量，删除收益低、误删风险高；C 档**不是死代码**（要么是契约断言辅助，要么是"功能已实现但 UI 未接入"）。

## 方法

1. 提取形如 `export (async) function|const|let|var|class|interface|type|enum NAME` 的声明，记录文件与行号、定义次数。
2. 用 ripgrep `-o -w` 做词边界计数，分两次扫描：
   - **生产**：`tauri-app/src`（排除 `**/__tests__/**`）+ `tauri-app/scripts`
   - **测试**：`tauri-app/src/**/__tests__/**` + `tauri-app/test`
3. 引用数 = 匹配数 − 定义出现次数。引用数 ≤ 0 且测试引用为 0 → 零引用；引用数 ≤ 0 但测试引用 > 0 → 仅测试引用。
4. 抽样人工验证 4 个零引用项（`getChapters`、`formatDate`、`applyPreset`、`getPresets`）：全仓范围内确实只有定义行，无任何调用。

## 局限

- 动态调用、字符串键调用、通过 `window` 暴露的入口无法被静态匹配；本次抽样未见此类情况，但删除前仍应 `git grep` 全仓（含 `docs-fork/`）复核一次。
- 删除类型导出不影响运行时，只降低契约表达力 —— 因此归入 B 档不动。
- Rust 侧（`src-tauri`）不引用 TS 标识符，但某些常量的语义在 Rust 里有对应实现（如迁移文件名、导入限制）；删除前需确认没有"两处必须一致"的约定。
- 仅测试引用的 66 项里，`assert*/may*/is*/resolve*/freeze*` 是契约断言辅助（设计如此）；`parse*/generate*/plan*/build*` 是**已实现但生产未接入**的功能，删掉会丢能力，不算清理。

---

## A 档：可安全删除（无生产引用、无测试引用、非契约类型）

| 文件 | 导出 | 行 |
|---|---|---|
| `services/knowledge-service.ts` | `listDocuments` / `getStats` / `loadKBData` / `getVectorlessCount` / `getVectorRebuildStatus` / `searchKB` / `backfillVectors` / `clearKnowledgeBase` / `importKnowledgeDocument` / `importKnowledgeFolder` / `removeKnowledgeDocument` | 78–133 |
| `services/draft-index.ts` | `getNextDraftVersion` / `getDraftMeta` / `getNextRevisionIndex` / `markRevisionMerged` / `getNextReviewIndex` / `readDraftIndex` | 143–280 |
| `services/version-service.ts` | `getChapters` / `getChapterVersions` / `getVersionContent` / `getChapterLatestContent` / `revertToVersion` | 33–75 |
| `services/generation/main-generation-transport.ts` | `beginMainGeneration` / `pauseMainGeneration` / `resumeMainGeneration` / `restartMainGeneration` / `discardMainGenerationCandidate` | 110–114 |
| `services/style-presets.ts` | `getPresets` / `applyPreset` / `getOverrideFields` | 143–158 |
| `services/workflows/chapter-workflow.ts` | `getDraftDir` / `getDraftPath` / `updateDraftStatus` | 109–205 |
| `services/project-templates.ts` | `getTemplates` / `getTemplateConfig` | 111–116 |
| `services/workflows/character-card-normalizer.ts` | `mergeExtractedCharacterCardsWithExisting` | 434 |
| `services/workflows/creative-workflow-launcher.ts` | `CreativeWorkflowLauncher`（const） | 258 |
| `services/workflows/workflow-utils.ts` | `isAllCriticalPassed` | 171 |
| `services/agent/intent-router.ts` | `isSlashCommand` | 121 |
| `services/prompt-templates.ts` | `getAllPromptTemplates` | 68 |
| `tokens/spacing.ts` | `tailwindSpacing` | 26 |
| `utils/time.ts` | `formatDate` | 21 |
| `utils/frontmatter.ts` | `mergeFrontmatterBody` | 46 |
| `stores/agent-store.ts` | `AGENT_GENERATION_BUDGET` | 32 |
| `stores/character-rename-ledger.ts` | `updateCharacterRename` | 37 |
| `components/settings/unsaved-settings.ts` | `confirmLeaveSettings` | 76 |

## B 档：需先确认（契约常量 / 类型 / 可能与 Rust 或 IPC 契约对应）

| 文件 | 导出 | 判定要点 |
|---|---|---|
| `shared/project-paths.ts` | `DIR_LORE_INTERNAL` | `.lore` 目录名，可能与 Rust 常量成对 |
| `shared/import-limits.ts` | `DEFAULT_IMPORT_RESOURCE_LIMITS` | 导入限制契约 |
| `shared/generation-owner-contract.ts` | `MATERIAL_DECISION_MAX_SOURCES` / `PlanningGenerationScope` / `PlanningContinuationReceipt` | 生成所有者契约 |
| `shared/project-storage.ts` | `MigrationId` / `MigrationPhysicalPhase` / `GlobalStorageLocators` / `PROJECT_MIGRATION_JOURNAL` / `STORAGE_ENVIRONMENT` / `PortableReceiptProjection` | 迁移与存储契约 |
| `shared/plot-outline-contract.ts` | `plotOutlinePolicy` / `plotOutlineExpected` / `parsePlotOutlinePurpose` / `PlotOutlineAuthorEdit` | 剧情大纲契约辅助 |
| `shared/import-run.ts` | `isAuthorImportPreviewStaleError` / `isImportRunDirectCheckpointStage` / `ImportSourceFileIdentity` / `ImportRunPrepareRequest` / `ImportRunBeginParsingRequest` | 导入运行契约 |
| `shared/generation-contract.ts` | `safeTransportError` / `GenerationRun` / `ProviderUsagePolicy` | 生成契约 |
| `shared/character-avatar.ts` | `MAX_CHARACTER_AVATAR_INPUT_BYTES` / `PortableCharacterAvatarRow` | 头像体积上限与备份行 |
| `shared/character-identity.ts` | `CharacterAlias` / `CharacterApproval` / `CharacterProposal` | 身份契约类型 |
| `shared/writing-skills.ts` | `WRITING_SKILL_NAME_PATTERN` | 技能名校验 |
| `shared/agent-generation.ts` | `normalizeAgentPlanningArguments` | Agent 参数归一 |
| `shared/provider-presets.ts` | `resolveModelProfileStructuredOutput` | 模型结构化输出解析 |
| `shared/review-cycle.ts` | `unshownMustShowRecheckFindings` | 审稿复检规则 |
| `shared/model-provider-resources.ts` | `isModelProviderResourceId` | 资源 id 判定 |
| `shared/ipc-channels.ts` | `MCPConfigLoadResult` | IPC 结果类型 |
| `shared/update-types.ts` | `UpdateCheckResult` | 更新检查类型 |
| `shared/cloud-backup.ts` | `CloudBackupOperationState` | 云备份状态 |
| `shared/project-overview.ts` | `ProjectPeekCapability` | 预览能力 |
| `components/run/run-failure.ts` | `RunFailure` | 运行失败类型 |
| `stores/theme-store.ts` | `WritingFont` | 字体类型 |
| `components/stage/architecture/architecture-sections.ts` | `BadgeTone` | 展示类型 |
| `components/stage/library/shelf-format.ts` | `CoverTone` | 展示类型 |

> 类型与契约常量不产生运行时成本。**默认不动**；只有确认对应能力已废弃时才删。

---

## C 档：仅测试引用（66 项，**不是死代码**）

两类要分开看：

- **契约断言/规则辅助**（`assert*`、`may*`、`is*`、`resolve*`、`freeze*`、`decide*`、`classify*`）：生产由其他路径实现同等判定，这些导出是给测试断言用的**单一事实源**，保留。
- **功能已实现但生产未接入**（`parse*`、`generate*`、`plan*`、`build*`，例如 `generatePlotTree`、`planTaskBudget`、`parseArchitectureCharacterProposal`）：删除等于丢能力，应视为"待接入功能"，而不是清理对象。

| 文件:行 | 导出 |
|---|---|
| `components\dialogs\import-novel-paths.ts:1` | `inferImportedNovelProjectName` |
| `components\editor\character-extraction-policy.ts:13` | `isCharacterExtractionReady` |
| `components\panels\sidebar\sidebar-file-openers.ts:28` | `confirmCurrentProjectSession` |
| `services\agent\agent-completion.ts:8` | `requireCompleteAgentResponse` |
| `services\agent\context-builder.ts:40` | `buildAgentSystemPrompt` |
| `services\agent\writing-skill-bindings.ts:137` | `freezeWritingSkill` |
| `services\architecture-service.ts:17` | `checkArchStatus` |
| `services\generation\generation-runtime.ts:138` | `listMainGenerationRuns` |
| `services\generation\generation-runtime.ts:141` | `readMainGenerationRun` |
| `services\generation\prompt-budget-failure.ts:160` | `formatTaskBudgetDecisionFailure` |
| `services\generation\task-budget-planner.ts:231` | `planTaskBudget` |
| `services\plot-tree-generator.ts:24` | `generatePlotTree` |
| `services\workflows\character-card-normalizer.ts:459` | `extractCompleteCharacterCards` |
| `services\workflows\character-card-normalizer.ts:484` | `parseCharacterProposalCards` |
| `services\workflows\character-card-normalizer.ts:477` | `parseCharacterCardsFromModelOrSource` |
| `services\workflows\context-snapshot.ts:51` | `buildChapterContextSnapshot` |
| `services\workflows\directory-workflow.ts:278` | `getBlueprintCount` |
| `services\workflows\directory-workflow.ts:126` | `parseTextBlueprints` |
| `services\workflows\directory-workflow.ts:250` | `verifyBlueprintsPersisted` |
| `shared\agent-response-protocol.ts:154` | `projectInterruptedAgentVisibleText` |
| `shared\blueprint-character-sync-evidence.ts:63` | `blueprintCharacterSyncFactError` |
| `shared\builtin-writing-skills.ts:274` | `readBuiltinWritingSkill` |
| `shared\chapter-goal-review.ts:25` | `freezeChapterGoals` |
| `shared\character-identity.ts:116` | `resolveScopedCharacterIdentity` |
| `shared\character-identity.ts:96` | `rejectedCharacterProposalKey` |
| `shared\character-identity.ts:13` | `resolveCharacterIdentity` |
| `shared\character-identity.ts:20` | `assertCharacterResolution` |
| `shared\character-proposal-parser.ts:49` | `parsePlanningMaterialCharacterProposal` |
| `shared\character-proposal-parser.ts:16` | `parseArchitectureCharacterProposal` |
| `shared\draft-reconciliation.ts:108` | `stripDraftReconciliationBlock` |
| `shared\draft-short-outline.ts:20` | `stripDraftShortOutlineBlock` |
| `shared\draft-units.ts:5` | `DRAFT_UNIT_ALGORITHM_VERSION` |
| `shared\draft-units.ts:25` | `countLegacyDraftUnitsV1` |
| `shared\finalization-step-error.ts:35` | `finalizationStepErrorCode` |
| `shared\finalized-continuity.ts:177` | `parseFinalizedCharacterStateResponse` |
| `shared\generation-contract.ts:96` | `assertVisibleSnapshot` |
| `shared\generation-contract.ts:73` | `assertReservation` |
| `shared\generation-contract.ts:91` | `assertAttemptTransition` |
| `shared\generation-owner-contract.ts:78` | `MATERIAL_DECISION_SOURCE_ID` |
| `shared\generation-owner-contract.ts:321` | `generationOutputContract` |
| `shared\human-confirmed-review.ts:225` | `hasIncludedReviewWork` |
| `shared\legacy-roster-generation-pure.ts:86` | `parseLegacyRosterJson` |
| `shared\legacy-roster-generation-pure.ts:79` | `buildLegacyRosterReplacementTask` |
| `shared\official-homepage.ts:5` | `OFFICIAL_HOMEPAGE_URL` |
| `shared\plot-outline-contract.ts:164` | `derivePlotOutlineCursor` |
| `shared\plot-outline-contract.ts:319` | `plotOutlineRecoveryDraft` |
| `shared\plot-tree-generation-pure.ts:119` | `parsePlotTreeSnapshot` |
| `shared\project-format.ts:5` | `CANONICAL_PROJECT_DATABASE` |
| `shared\project-format.ts:26` | `createCanonicalProjectManifest` |
| `shared\project-storage.ts:43` | `mayReadTransferredAuthority` |
| `shared\project-storage.ts:64` | `mayReplayTransferredExecution` |
| `shared\project-storage.ts:21` | `assertDistinctStorageRoots` |
| `shared\review-cycle.ts:216` | `findingAnchorKey` |
| `shared\review-cycle.ts:175` | `buildReviewCycleRecheckReport` |
| `shared\review-cycle.ts:241` | `assertSingleRecheck` |
| `shared\review-cycle.ts:226` | `decideFindingStatus` |
| `shared\review-cycle.ts:132` | `classifyFindingEvidenceChange` |
| `shared\review-generation-report.ts:110` | `buildReviewGenerationReport` |
| `shared\source-ref.ts:25` | `validateSourceRef` |
| `shared\visible-continuation.ts:1` | `VISIBLE_CONTINUATION_VERSION` |
| `shared\writing-skills.ts:242` | `inspectLocalWritingSkill` |
| `shared\writing-skills.ts:292` | `parseGitHubWritingSkillUrl` |
| `stores\character-rename-ledger.ts:103` | `rebuildCharacterRenamesAfterSave` |
| `stores\editor-discard.ts:101` | `discardCurrentProjectEditorChanges` |
| `stores\project-editor-draft-ledger.ts:111` | `mergeNamedRecordDraftWithRemote` |
| `utils\frontmatter.ts:27` | `extractFrontmatter` |


---

## 执行建议

1. **A 档分批清理**，每批一个域（如"知识库遗留导出"、"版本服务遗留导出"），每批后跑 `pnpm typecheck && pnpm run lint && pnpm test`。
2. 删除前用 `git grep -n '<name>' -- ':!tauri-app/src'` 复核 docs 与脚本；本次已扫 `tauri-app/scripts`、`tauri-app/test`，但 `docs-fork/` 未纳入词边界计数。
3. **不要删 B 档**；如确有废弃能力，先删能力本身（含 Rust 命令与 IPC 频道），再删其类型。
4. C 档先不动；若要清，需先决定"这些未接入功能是否还要保留"。
5. 预估 A 档收益：约 30 个导出、300–500 行代码（`knowledge-service` 与 `draft-index` 两组占大头）。

## 未覆盖

- 根 `src/`（Electron 基线）不在本审计范围。
- 未做文件级死文件分析（整文件无引用），下一步可补。
- 未评估删除对打包体积的影响（导出未使用通常已被 tree-shaking 排除，收益主要在可维护性）。
