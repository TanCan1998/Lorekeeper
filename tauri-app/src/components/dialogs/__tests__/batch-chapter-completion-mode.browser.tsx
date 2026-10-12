import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { page } from 'vitest/browser'
import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'

import type { FinalizedSourceReadResult } from '../../../shared/finalized-continuity'
import type { FileNode, ModelExecutionLeaseReceipt, ModelProfile, ProjectData } from '../../../shared/ipc-channels'
import { setActiveProjectSessionContext } from '../../../shared/project-session-context'
import { clearProjectCustomPrompts } from '../../../services/prompt-templates'
import { disposeProjectService, initProjectService } from '../../../services/project-service'
import { useDraftStore } from '../../../stores/draft-store'
import { useEditorStore } from '../../../stores/editor-store'
import { useLayoutStore } from '../../../stores/layout-store'
import { useLLMStore } from '../../../stores/llm-store'
import { useLocaleStore } from '../../../stores/locale-store'
import { useProjectStore } from '../../../stores/project-store'
import { useWorkflowStore } from '../../../stores/workflow-store'
import DraftEditor from '../../editor/DraftEditor'
import BottomPanel from '../../panels/BottomPanel'
import ProjectTree from '../../panels/sidebar/ProjectTree'
import BatchChapterCreationDialog from '../BatchChapterCreationDialog'
import type { BeginGenerationRequest, BeginGenerationBatchRequest, GenerationBatchProgress, VisibleCompositionReceipt } from '../../../shared/generation-owner-contract'
import type { MainGenerationRunHandle, MainGenerationRunView } from '../../../services/generation/generation-runtime'
import type { GenerationOutcome } from '../../../services/generation/generation-harness'
import { hashAuthorText } from '../../../shared/source-ref'
import { composeDraftVisibleContinuation, isDraftVisibleTextVersion } from '../../../shared/draft-visible-text'
import { countDraftUnits } from '../../../services/workflows/commands/generate-draft.command'
import {
  installTauriInternals,
  type TauriInternalsHandle,
} from '../../../../test/helpers/tauri-internals'

const PROJECT_PATH = 'C:\\novels\\batch-completion-mode'
const PROJECT_SESSION = {
  projectId: 'batch-completion-mode',
  leaseId: 'batch-completion-mode-lease',
  projectPath: PROJECT_PATH,
}
const DRAFT_TEXT = '晨雾漫过旧教学楼，沈砺沿着湿润台阶进入档案室。他检查窗锁与登记簿，发现昨夜留下的墨迹已经干透，却有一页被人整齐撕走。管理员递来备用钥匙，提醒他午后停电。沈砺记下时间，决定先去钟楼核对监控。'
const FIRST_DRAFT_TAIL = '第一章尾部唯一线索：银色怀表在午夜停摆。'
const FIRST_DRAFT_TEXT = `${'雨'.repeat(70)}。${FIRST_DRAFT_TAIL}`
const FINALIZATION_ID = 'finalization-browser-1'
const FINALIZED_CONTENT_HASH = 'd'.repeat(64)
const originalDraftState = useDraftStore.getState()
const originalEditorState = useEditorStore.getState()
const originalLayoutState = useLayoutStore.getState()
const originalLLMState = useLLMStore.getState()
const originalProjectState = useProjectStore.getState()
const originalWorkflowState = useWorkflowStore.getState()

;(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

let root: Root | undefined
let container: HTMLDivElement | undefined
let invoke: ReturnType<typeof vi.fn>
let tauriInternals: TauriInternalsHandle
let draftRecord: {
  id: number
  chapterNumber: number
  version: number
  status: 'draft' | 'finalized'
  wordCount: number
  createdAt: string
  source: 'write'
  content: string
} | null
let importedFinalizedDrafts: Array<{
  id: number
  chapterNumber: number
  version: number
  status: 'finalized'
  wordCount: number
  createdAt: string
  source: 'write'
  content: string
}> | null
let postProcessRunCreated: boolean
let draftCompletionIndex: number
let draftCompletions: string[]
let deferDraftCompletion: boolean
let pendingDraftCompletions: Array<() => void>
let notifyDraftCompletionReady: (() => void) | undefined
let changeDefaultAfterFirstDraft: boolean
let postProcessSteps: Array<{
  stepKey: string
  label: string
  critical: boolean
  ok: boolean
  completedAt: string | null
  errorMsg: string | null
  lastAttemptAt: string
  attemptCount: number
}>

function project(): ProjectData {
  return {
    id: PROJECT_SESSION.projectId,
    sessionLease: PROJECT_SESSION.leaseId,
    name: '批量完成模式浏览器测试',
    path: PROJECT_PATH,
    novelConfig: {
      genre: '悬疑',
      subGenre: '',
      targetAudience: '全龄',
      totalChapters: 3,
      wordsPerChapter: 100,
      plotStructure: 'three_act',
      narrativePOV: 'third_limited',
      coreOutline: '雨夜来信开启调查。',
      worldSetting: '',
      goldenFinger: '',
      protagonistProfile: '',
      globalGuidance: '',
    },
    characterStates: '',
    createdAt: '',
    updatedAt: '',
  }
}

function generationModel(): ModelProfile {
  return {
    id: 'grok-browser',
    name: 'Grok Browser',
    provider: 'custom',
    protocol: 'openai',
    modelName: 'grok-browser',
    apiKey: 'test-only-key',
    baseUrl: 'https://models.example/v1',
    temperature: 0.7,
    maxTokens: 4096,
    purposes: ['generation'],
  }
}

// eslint-disable-next-line react-refresh/only-export-components -- browser-only integration shell
function VisibleBatchShell() {
  const [dialogOpen, setDialogOpen] = useState(true)
  const activeChapterTab = useEditorStore(state => {
    const activeTab = state.tabs.find(tab => tab.id === state.activeTabId)
    return activeTab?.type === 'chapter' ? activeTab : undefined
  })

  return (
    <div>
      <BatchChapterCreationDialog
        isOpen={dialogOpen}
        startChapterNumber={1}
        onClose={() => setDialogOpen(false)}
      />
      <div data-testid="project-tree"><ProjectTree /></div>
      <div data-testid="chapter-editor">
        {activeChapterTab?.filePath && activeChapterTab.projectKey && (
          <DraftEditor
            tabId={activeChapterTab.id}
            filePath={activeChapterTab.filePath}
            content={activeChapterTab.content ?? ''}
            projectKey={activeChapterTab.projectKey}
          />
        )}
      </div>
      <div data-testid="task-panel"><BottomPanel /></div>
    </div>
  )
}

function blueprint(chapterNumber = 1) {
  return {
    chapterNumber,
    title: '雨夜来信',
    role: '开篇',
    purpose: '建立调查目标。',
    characters: ['沈砺'],
    keyEvents: '收到匿名信。',
    suspenseHook: '信封背面出现陌生署名。',
    userGuidance: '',
    notes: '',
  }
}

const MODEL_LEASE: ModelExecutionLeaseReceipt = {
  leaseId: 'batch-browser-model-lease',
  modelId: 'grok-browser',
  provider: 'custom',
  protocol: 'openai',
  modelName: 'grok-browser',
  modelRevision: 'a'.repeat(64),
  endpointFingerprint: 'b'.repeat(64),
  capabilityEvidence: {
    source: {
      contextWindowTokens: 'unknown',
      maxOutputTokens: 'legacy-profile',
      featureFlags: 'unknown',
    },
    subjectFingerprint: 'c'.repeat(64),
    contextWindowTokens: null,
    maxOutputTokens: 4096,
    reasoning: null,
    structuredOutput: true,
    usage: null,
  },
  createdAt: 1_000,
  expiresAt: 61_000,
}

function fileTree(): FileNode[] {
  if (draftRecord?.status === 'finalized') {
    return [{
      name: 'manuscript',
      path: `${PROJECT_PATH}\\manuscript`,
      isDir: true,
      children: [{
        name: '第1章 雨夜来信.txt',
        path: `${PROJECT_PATH}\\manuscript\\chapter_1.txt`,
        isDir: false,
      }],
    }]
  }
  return [{
    name: 'drafts',
    path: `${PROJECT_PATH}\\drafts`,
    isDir: true,
    children: draftRecord
      ? [{ name: '第1章 雨夜来信 v1', path: `ai-novel://draft/${draftRecord.id}`, isDir: false }]
      : [],
  }]
}

function installIpc() {
  let batch: GenerationBatchProgress | undefined
  let generationAttempt = 0
  const views = new Map<string, MainGenerationRunView>()
  const selections = new Map<string, BeginGenerationRequest>()
  const compositions = new Map<string, VisibleCompositionReceipt>()
  const purposes = new Map<string, string>()
  const readBatchProgress = () => {
    if (batch?.mode === 'auto_finalize' && draftRecord?.status === 'finalized' && postProcessRunCreated
      && postProcessSteps.filter(step => step.critical).every(step => step.ok)) {
      const saved = batch.completedChapters.find(item => item.chapterNumber === draftRecord!.chapterNumber)!
      saved.finalizationId = FINALIZATION_ID
      saved.postProcessComplete = true
      batch.nextChapterNumber = saved.chapterNumber === batch.range.endChapter ? null : saved.chapterNumber + 1
    }
    return structuredClone(batch)
  }
  // generation:* / finalization-generation:* 尚未迁移（不在 MIGRATED_CHANNELS，
  // ipc-client 前置拒绝），以下命令桩当前不可达；其命名参数键按契约元组推导，
  // 待迁移批次登记 CHANNEL_ARG_NAMES 后核对。
  const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
    generation_prepare_draft_context: async args => {
      const input = args.request as { chapterNumber: number; query: string; selectedDraftIds: number[] }
      return { preparationId: `browser-preparation-${input.chapterNumber}`,
        knowledgeSnapshot: { version: 1, state: 'empty', storageState: 'absent', query: input.query, topK: 5, canonicalRevision: null, documentsRevision: null, items: [] },
        selectedDrafts: await Promise.all(input.selectedDraftIds.map(async draftId => {
          if (!draftRecord || draftRecord.id !== draftId) throw new Error('GENERATION_SOURCE_MISSING')
          return { draftId, chapterNumber: draftRecord.chapterNumber, version: draftRecord.version, content: draftRecord.content,
            contentHash: await hashAuthorText(draftRecord.content) }
        })) }
    },
    generation_begin_batch: args => {
      const input = args.request as BeginGenerationBatchRequest
      batch = { ...input, batchId: 'browser-batch', rootHandle: { projectId: PROJECT_SESSION.projectId,
        epoch: PROJECT_SESSION.leaseId, rootActionId: 'browser-root', runId: 'browser-batch' },
        completedChapters: [], nextChapterNumber: input.range.startChapter }
      return structuredClone(batch)
    },
    generation_read_batch: () => readBatchProgress(),
    generation_confirm_batch_finalization: () => readBatchProgress(),
    generation_begin: args => {
      const input = args.request as BeginGenerationRequest
      if (!input.preparationId) throw new Error('GENERATION_DRAFT_PREPARATION_REQUIRED')
      const handle = { ...batch!.rootHandle, runId: `browser-chapter-${input.chapterNumber}` }
      const view: MainGenerationRunView = { handle, status: 'running', nonReplayable: false, artifacts: [],
        budget: { maxAttempts: 32, maxRequestedOutputTokens: 2000000, maxRequestedOutputTokensPerAttempt: 32768, deadlineAt: Date.now() + 3600000 } }
      selections.set(handle.runId, input); views.set(handle.runId, view)
      return structuredClone(view)
    },
    generation_read: args => structuredClone(views.get((args.handle as MainGenerationRunHandle).runId)),
    generation_cancel: args => structuredClone(views.get((args.handle as MainGenerationRunHandle).runId)),
    generation_execute: async args => {
      const request = args.request as { handle: MainGenerationRunHandle; task: { purpose: string } }
      const view = views.get(request.handle.runId)!
      const isOutline = request.task.purpose === 'chapter-draft-short-outline'
      const text = isOutline ? '目标：收到匿名信；行动：沈砺拆信检查署名；结果：开始调查。'
        : draftCompletions[draftCompletionIndex++] ?? DRAFT_TEXT
      if (!isOutline && changeDefaultAfterFirstDraft && draftCompletionIndex === 1) useLLMStore.setState({ defaultModelId: 'changed-default-model' })
      if (!isOutline && deferDraftCompletion) {
        await new Promise<void>(resolve => {
          pendingDraftCompletions.push(resolve)
          notifyDraftCompletionReady?.()
        })
      }
      const attempt = ++generationAttempt
      const artifact = { ...view.handle, artifactId: `${view.handle.runId}:artifact:${attempt}`, attemptId: `${view.handle.runId}:attempt:${attempt}`,
        revision: 1, durableRevision: 1, text, textHash: await hashAuthorText(text), status: 'completed' as const }
      view.artifacts = [...view.artifacts, artifact]
      purposes.set(artifact.artifactId, request.task.purpose)
      tauriInternals.emit('generation:snapshot', artifact)
      const outcome: GenerationOutcome = { status: 'completed', content: text, finishReason: 'stop', receipt: {
        model: { id: 'grok-browser', configurationRevision: 'a'.repeat(64), endpointFingerprint: 'b'.repeat(64) },
        capabilities: { contextWindowTokens: null, maxOutputTokens: 4096, reasoning: false, structuredOutput: false, usage: false,
          source: { contextWindowTokens: 'unknown', maxOutputTokens: 'user-operational-cap', featureFlags: 'unknown' } },
        budget: { attempt, maxAttempts: 32, requestedOutputTokens: 4096,
          cumulativeRequestedOutputTokens: attempt * 4096, maxRequestedOutputTokens: 2000000,
          maxRequestedOutputTokensPerAttempt: 32768, deadlineAt: view.budget.deadlineAt }, finishReason: 'stop',
        visibleArtifact: { artifactId: artifact.artifactId, attemptId: artifact.attemptId, revision: 1, textHash: artifact.textHash },
      } }
      return { outcome, run: structuredClone(view) }
    },
    generation_compose_visible: async args => {
      const handle = args.handle as MainGenerationRunHandle, ids = args.artifactIds as string[]
      const view = views.get(handle.runId)!
      const algorithm = args.algorithm
      if (!isDraftVisibleTextVersion(algorithm)) throw new Error('GENERATION_COMPOSITION_ALGORITHM_INVALID')
      const text = ids.reduce((text, id) => {
        const artifact = view.artifacts.find(item => item.artifactId === id)!
        return purposes.get(id) === 'chapter-draft-condense' ? artifact.text : composeDraftVisibleContinuation(text, artifact.text, algorithm)
      }, '')
      const receipt: VisibleCompositionReceipt = { algorithm, text, textHash: await hashAuthorText(text),
        artifactIds: ids, sources: view.artifacts.map(item => ({ artifactId: item.artifactId, revision: item.revision, textHash: item.textHash })) }
      expect(receipt.textHash).toBe(args.expectedCompositionHash); compositions.set(handle.runId, receipt); return receipt
    },
    generation_commit_draft: args => {
      const request = args.request as { handle: MainGenerationRunHandle; chapterNumber: number; expectedCompositionHash: string }
      const composition = compositions.get(request.handle.runId)!
      expect(request.expectedCompositionHash).toBe(composition.textHash)
      expect(selections.get(request.handle.runId)?.batchId).toBe(batch!.batchId)
      draftRecord = { id: 101, chapterNumber: request.chapterNumber, version: 1, status: 'draft', source: 'write',
        content: composition.text, wordCount: countDraftUnits(composition.text), createdAt: '2026-08-28T00:00:00.000Z' }
      batch!.completedChapters.push({ chapterNumber: request.chapterNumber, draftId: 101, version: 1,
        contentHash: composition.textHash, sourceRunHandle: request.handle })
      if (batch!.mode === 'draft_review') batch!.nextChapterNumber = request.chapterNumber === batch!.range.endChapter ? null : request.chapterNumber + 1
      return { success: true, id: 101, version: 1, content: composition.text, contentHash: composition.textHash }
    },
    prompt_load_global: () => ({ templates: [], diagnostics: [] }),
    db_draft_authority_sequence: () => ({
      status: 'empty',
      lastChapterNumber: 0,
      nextChapterNumber: 1,
      duplicateChapterNumbers: [],
      authorityFingerprint: 'a'.repeat(64),
    }),
    llm_begin_execution_lease: () => ({ success: true, lease: MODEL_LEASE }),
    llm_close_execution_lease: () => ({ success: true }),
    llm_generate_stream: args => {
      const requestId = String(args.requestId)
      const request = args.request as { purpose?: string; responseFormat?: { type?: string } }
      const completion = request.purpose === 'post-process'
        ? request.responseFormat?.type === 'json_object'
          ? '{"updates":[],"newCharacters":[]}'
          : '本章收到匿名信并开始调查。'
        : draftCompletions[draftCompletionIndex++] ?? DRAFT_TEXT
      if (changeDefaultAfterFirstDraft && request.purpose === 'chapter-draft' && draftCompletionIndex === 1) {
        useLLMStore.setState({ defaultModelId: 'changed-default-model' })
      }
      const complete = () => {
        tauriInternals.emit('llm:stream-chunk', { requestId, chunk: completion })
        tauriInternals.emit('llm:stream-done', { requestId, fullText: completion, finishReason: 'stop' })
      }
      if (deferDraftCompletion && request.purpose === 'chapter-draft') {
        pendingDraftCompletions.push(complete)
      } else {
        queueMicrotask(complete)
      }
      return { requestId, started: true }
    },
    fs_check_exists: () => false,
    fs_list_dir: args => args.dirPath === PROJECT_PATH ? fileTree() : [],
    db_blueprint_get_all: () => importedFinalizedDrafts ? [] : [blueprint(), blueprint(2)],
    db_continuity_list_before: () => [],
    db_consistency_exemption_list: () => [],
    db_blueprint_get: args => importedFinalizedDrafts ? null : blueprint(Number(args.chapterNumber)),
    db_character_get_all: () => [{ id: 1, name: '沈砺', role: 'protagonist', currentState: null }],
    db_project_core_get: () => ({ premise: '雨夜来信开启调查。', charactersArch: '', worldbuilding: '', synopsis: '' }),
    db_draft_get_latest: args => draftRecord?.chapterNumber === Number(args.chapterNumber) ? draftRecord : null,
    db_draft_get_finalized: () => null,
    db_draft_get_full: args => importedFinalizedDrafts?.find(draft => draft.id === Number(args.id)) ?? draftRecord,
    db_draft_next_version: () => 1,
    db_draft_create: args => {
      const input = args.params as { chapterNumber: number; version: number; content: string; wordCount: number }
      draftRecord = {
        id: 101,
        chapterNumber: input.chapterNumber,
        version: input.version,
        status: 'draft',
        wordCount: input.wordCount,
        createdAt: '2026-08-28T00:00:00.000Z',
        source: 'write',
        content: input.content,
      }
      return { success: true, id: draftRecord.id }
    },
    db_draft_list: () => draftRecord ? [draftRecord] : [],
    db_draft_list_all: () => importedFinalizedDrafts ?? (draftRecord ? [draftRecord] : []),
    db_draft_get_meta: () => draftRecord,
    db_revision_get_pending: () => [],
    db_review_list: () => [],
    chapter_list_incomplete_deletions: () => ({ success: true, operations: [] }),
    finalization_commit: args => {
      if (!draftRecord) throw new Error('Missing generated draft before finalization')
      const snapshot = args.snapshot as { contentRevision: number }
      expect(snapshot.contentRevision).toBe(1)
      draftRecord = { ...draftRecord, status: 'finalized' }
      return {
        success: true,
        committed: true,
        finalizationId: FINALIZATION_ID,
        contentHash: FINALIZED_CONTENT_HASH,
        contentRevision: snapshot.contentRevision,
        draftId: draftRecord.id,
        publicationStatus: 'published',
      }
    },
    db_continuity_read_source: args => {
      const draftId = Number(args.draftId)
      if (!draftRecord || draftRecord.status !== 'finalized' || draftRecord.id !== draftId) {
        return { status: 'invalid' } satisfies FinalizedSourceReadResult
      }
      return {
        status: 'valid',
        snapshot: {
          source: {
            draftId: draftRecord.id,
            finalizationId: FINALIZATION_ID,
            chapterNumber: draftRecord.chapterNumber,
            contentHash: FINALIZED_CONTENT_HASH,
          },
          chapterTitle: blueprint(draftRecord.chapterNumber).title,
          content: draftRecord.content,
          projectionGeneration: 0,
        },
      } satisfies FinalizedSourceReadResult
    },
    finalization_generation_read: args => {
      const slot = (args.request as { slot: { source: { draftId: number; finalizationId: string; chapterNumber: number; contentHash: string }; stepKey: string } }).slot
      expect(args.projectSession).toEqual(PROJECT_SESSION)
      expect(slot.source).toEqual({ draftId: 101, finalizationId: FINALIZATION_ID,
        chapterNumber: 1, contentHash: FINALIZED_CONTENT_HASH })
      expect(['chapter_notes', 'character_cards']).toContain(slot.stepKey)
      return { attemptCount: 0, view: { handle: { projectId: PROJECT_SESSION.projectId,
        epoch: PROJECT_SESSION.leaseId, rootActionId: 'browser-finalization', runId: `browser-${slot.stepKey}` },
        status: 'completed', artifacts: [] }, modelId: 'grok-browser', context: { slot }, sourceStatus: 'current',
        effect: slot.stepKey === 'chapter_notes'
          ? { success: true, stepKey: 'chapter_notes', chapterNotes: '匿名信引发调查。', factCount: 1, blueprintUpdated: true }
          : { success: true, stepKey: 'character_cards', applied: 0, unchanged: 0, candidates: [], unresolved: [] } }
    },
    kb_import_text: () => ({ success: true, chunkCount: 1, docId: 'knowledge-browser-1' }),
    db_finalization_link_knowledge_document: () => ({ success: true }),
    db_continuity_save_finalized: () => ({ success: true }),
    db_blueprint_update_notes: () => ({ success: true, updated: true }),
    db_character_roster_read: () => ({ status: 'empty', revision: 0, entries: [] }),
    db_post_process_get_latest_run: () => postProcessRunCreated
      ? {
        id: 701,
        sourceLabel: '第1章定稿',
        allCriticalPassed: postProcessSteps.filter(step => step.critical).every(step => step.ok),
        createdAt: '2026-08-28T00:00:00.000Z',
        updatedAt: '2026-08-28T00:00:00.000Z',
      }
      : null,
    db_post_process_create_run: args => {
      const input = args.params as { steps: Array<{ key: string; label: string; critical: boolean }> }
      postProcessRunCreated = true
      postProcessSteps = input.steps.map(step => ({
        stepKey: step.key,
        label: step.label,
        critical: step.critical,
        ok: false,
        completedAt: null,
        errorMsg: null,
        lastAttemptAt: '',
        attemptCount: 0,
      }))
      return { success: true, id: 701 }
    },
    db_post_process_get_steps: () => postProcessSteps,
    db_post_process_mark_step_ok: args => {
      const stepKey = String(args.stepKey)
      postProcessSteps = postProcessSteps.map(step => step.stepKey === stepKey
        ? {
          ...step,
          ok: true,
          completedAt: '2026-08-28T00:00:01.000Z',
          lastAttemptAt: '2026-08-28T00:00:01.000Z',
          attemptCount: 1,
        }
        : step)
      return { success: true }
    },
    db_post_process_mark_step_failed: args => {
      const stepKey = String(args.stepKey)
      const errorMsg = String(args.errorMsg)
      postProcessSteps = postProcessSteps.map(step => step.stepKey === stepKey
        ? {
          ...step,
          ok: false,
          errorMsg,
          lastAttemptAt: '2026-08-28T00:00:01.000Z',
          attemptCount: 3,
        }
        : step)
      return { success: true }
    },
  }
  tauriInternals = installTauriInternals({ commands })
  invoke = tauriInternals.invoke
}

beforeEach(() => {
  draftRecord = null
  importedFinalizedDrafts = null
  draftCompletionIndex = 0
  draftCompletions = [DRAFT_TEXT]
  deferDraftCompletion = false
  pendingDraftCompletions = []
  notifyDraftCompletionReady = undefined
  changeDefaultAfterFirstDraft = false
  postProcessRunCreated = false
  postProcessSteps = []
  disposeProjectService()
  clearProjectCustomPrompts()
  useLocaleStore.setState({ locale: 'zh-CN' })
  useProjectStore.setState({ currentProject: project(), fileTree: [], loading: false })
  setActiveProjectSessionContext(PROJECT_SESSION)
  useLLMStore.setState({
    models: [generationModel()],
    defaultModelId: 'grok-browser',
    defaultEmbeddingModelId: null,
    activeRequests: new Map(),
    loaded: true,
  })
  useWorkflowStore.setState({
    activeRuns: [],
    history: [],
    globalLogs: [],
    waitingRuns: {},
    currentRun: null,
    waitingForConfirm: false,
    waitingAfterStepIndex: -1,
    startWorkflow: originalWorkflowState.startWorkflow,
    addLog: originalWorkflowState.addLog,
  })
  useEditorStore.setState({ tabs: [], activeTabId: null, draftLedgers: {} })
  useLayoutStore.setState({ bottomPanelOpen: true, bottomTab: 'tasks' })
  useDraftStore.setState({
    draftsByChapter: {},
    loading: false,
    dataProjectKey: null,
    dataProjectSession: null,
    loadingProjectKey: null,
    loadingProjectSession: null,
  })
  installIpc()
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root?.unmount())
  container?.remove()
  root = undefined
  container = undefined
  tauriInternals.uninstall()
  disposeProjectService()
  setActiveProjectSessionContext(null)
  clearProjectCustomPrompts()
  useDraftStore.setState(originalDraftState)
  useEditorStore.setState(originalEditorState)
  useLayoutStore.setState(originalLayoutState)
  useLLMStore.setState(originalLLMState)
  useProjectStore.setState(originalProjectState)
  useWorkflowStore.setState(originalWorkflowState)
})

describe('batch chapter completion mode browser flow', () => {
  it('shows imported finalized chapters in the project tree when no blueprints exist', async () => {
    importedFinalizedDrafts = [1, 2].map(chapterNumber => ({
      id: 200 + chapterNumber,
      chapterNumber,
      version: 1,
      status: 'finalized' as const,
      wordCount: 18,
      createdAt: '2026-08-29T00:00:00.000Z',
      source: 'write' as const,
      content: `第${chapterNumber}章作者原稿正文`,
    }))

    await act(async () => root?.render(
      <div data-testid="project-tree"><ProjectTree /></div>,
    ))

    await vi.waitFor(() => {
      expect(useDraftStore.getState().draftsByChapter).toMatchObject({
        1: [{ id: 201, status: 'finalized' }],
        2: [{ id: 202, status: 'finalized' }],
      })
    })
    await vi.waitFor(() => {
      const manuscriptHeader = Array.from(container?.querySelectorAll('.tree-item') ?? [])
        .find(element => element.textContent?.includes('正文章节'))
      const manuscriptSectionText = manuscriptHeader?.parentElement?.textContent ?? ''
      expect(manuscriptSectionText).toContain('2 章')
      expect(manuscriptSectionText).toContain('第1章')
      expect(manuscriptSectionText).toContain('第2章')
      expect(manuscriptSectionText).not.toContain('暂无定稿章节')
    })
    expect(invoke).toHaveBeenCalledWith(
      'db_draft_list_all',
      { expectedProjectPath: PROJECT_PATH, projectSession: PROJECT_SESSION },
      undefined,
    )
    expect(invoke.mock.calls.some(([channel]) => channel === 'db_draft_list')).toBe(false)
  })

  it.each([
    {
      locale: 'zh-CN',
      mode: 'draft_review',
      modeLabel: '生成草稿待审',
      startLabel: '启动批量创作',
      taskLog: '开始第1章：生成草稿待审。',
      treeSection: '草稿箱',
      chapterLabel: '第1章 雨夜来信',
      editorState: '草稿',
      reviewLabel: 'AI 审稿',
    },
    {
      locale: 'en-US',
      mode: 'draft_review',
      modeLabel: 'Generate review drafts',
      startLabel: 'Start batch writing',
      taskLog: 'Starting Chapter 1: generate a review draft.',
      treeSection: 'Draft box',
      chapterLabel: 'Chapter 1 雨夜来信',
      editorState: 'Draft',
      reviewLabel: 'AI review',
    },
    {
      locale: 'zh-CN',
      mode: 'auto_finalize',
      modeLabel: '自动定稿',
      startLabel: '确认自动定稿并启动',
      taskLog: '开始第1章：生成草稿、自动定稿并完成后处理。',
      treeSection: '正文章节',
      chapterLabel: '第1章 雨夜来信',
      editorState: '已定稿（只读）',
      reviewLabel: 'AI 审稿',
    },
    {
      locale: 'en-US',
      mode: 'auto_finalize',
      modeLabel: 'Auto-finalize',
      startLabel: 'Confirm auto-finalize and start',
      taskLog: 'Starting Chapter 1: generate, auto-finalize, and post-process.',
      treeSection: 'Manuscript chapters',
      chapterLabel: 'Chapter 1 雨夜来信',
      editorState: 'Finalized (read-only)',
      reviewLabel: 'AI review',
    },
  ] as const)(
    'renders visible $locale $mode tree, editor, review, and task-log state',
    async ({ locale, mode, modeLabel, startLabel, taskLog, treeSection, chapterLabel, editorState, reviewLabel }) => {
      useLocaleStore.setState({ locale })
      deferDraftCompletion = true
      initProjectService()
      await act(async () => root?.render(<VisibleBatchShell />))

      await act(async () => page.getByRole('radio', { name: modeLabel }).click())
      if (mode === 'auto_finalize') {
        await act(async () => page.getByRole('button', {
          name: locale === 'en-US' ? 'Review auto-finalize' : '继续确认自动定稿',
        }).click())
      }
      await act(async () => page.getByRole('button', { name: startLabel }).click())

      await vi.waitFor(() => expect(pendingDraftCompletions).toHaveLength(1))
      await vi.waitFor(() => {
        expect(container?.querySelector('[data-testid="task-panel"]')?.textContent).toContain(taskLog)
      })

      await act(async () => pendingDraftCompletions.shift()?.())
      await vi.waitFor(() => {
        expect(useWorkflowStore.getState().history[0]?.status).toBe('completed')
      })
      await vi.waitFor(() => {
        const treeText = container?.querySelector('[data-testid="project-tree"]')?.textContent ?? ''
        const editor = container?.querySelector('[data-testid="chapter-editor"]')
        expect(treeText).toContain(treeSection)
        expect(treeText).toContain(chapterLabel)
        expect(editor?.textContent).toContain(editorState)
        if (mode === 'draft_review') {
          expect(editor?.textContent).toContain(reviewLabel)
        } else {
          expect(editor?.textContent).not.toContain(reviewLabel)
        }
      })

      const prose = page.getByTestId('chapter-editor').getByRole('textbox')
      if (mode === 'draft_review') {
        const editedText = locale === 'en-US'
          ? 'Author-editable batch draft prose.'
          : '作者可编辑的批量草稿正文。'
        await act(async () => prose.fill(editedText))
        await expect.element(prose).toHaveTextContent(editedText)
      } else {
        await expect.element(prose).toHaveTextContent(DRAFT_TEXT)
      }
    },
  )

  it('feeds the frozen first review draft tail into the second provider prompt without finalized or post-process work', async () => {
    draftCompletions = [FIRST_DRAFT_TEXT, DRAFT_TEXT]
    changeDefaultAfterFirstDraft = true
    await act(async () => {
      root?.render(<BatchChapterCreationDialog isOpen startChapterNumber={1} onClose={vi.fn()} />)
    })

    await act(async () => page.getByRole('spinbutton', { name: '本次章节数' }).fill('2'))
    await act(async () => page.getByRole('button', { name: '启动批量创作' }).click())

    await vi.waitFor(() => {
      expect(useWorkflowStore.getState().history[0]?.status).toBe('completed')
    })

    const draftRequests = invoke.mock.calls
      .filter(([channel, request]) => (
        channel === 'generation_execute'
        && (request as { request?: { task?: { purpose?: string } } } | undefined)?.request?.task?.purpose === 'chapter-draft'
      ))
      .map(([, request]) => (request as { request: { task: { messages: Array<{ content: string }> } } }).request.task)
    expect(draftRequests).toHaveLength(2)
    expect(draftRequests[1].messages.map(message => message.content).join('\n')).toContain(FIRST_DRAFT_TAIL)
    expect(invoke.mock.calls.filter(([channel]) => channel === 'generation_begin').map(([, input]) => (input as { request: BeginGenerationRequest }).request.modelId))
      .toEqual(['grok-browser', 'grok-browser'])
    expect(invoke.mock.calls.some(([channel]) => channel === 'llm_begin_execution_lease')).toBe(false)
    expect(invoke.mock.calls.some(([channel]) => (
      channel === 'finalization_commit'
      || channel === 'kb_import_text'
      || channel === 'db_blueprint_update_notes'
      || channel === 'db_character_roster_commit'
      || String(channel).startsWith('db_post_process_')
    ))).toBe(false)
  })

  it('shows one chapter 5 overlength notice and completes chapters 6 through 10 after saving the full 1350-unit revision', async () => {
    const nextDraftReady = () => new Promise<void>(resolve => { notifyDraftCompletionReady = resolve })
    let draftReady = nextDraftReady()
    const completeNextDraft = async () => {
      await act(async () => { await draftReady })
      expect(pendingDraftCompletions).toHaveLength(1)
      draftReady = nextDraftReady()
      await act(async () => pendingDraftCompletions.shift()?.())
    }
    await page.viewport(1440, 900)
    const chapterTexts = Array.from({ length: 10 }, (_, index) => `${String.fromCharCode(0x4e00 + index).repeat(1000)}。`)
    const condensed = `${'缩'.repeat(1350)}。`
    draftCompletions = [...chapterTexts.slice(0, 4), `${'长'.repeat(1400)}。`, condensed, ...chapterTexts.slice(5)]
    const current = useProjectStore.getState().currentProject!
    useProjectStore.setState({ currentProject: { ...current, novelConfig: { ...current.novelConfig, totalChapters: 10, wordsPerChapter: 1000 } } })
    deferDraftCompletion = true
    initProjectService()
    await act(async () => root?.render(<VisibleBatchShell />))
    await act(async () => page.getByRole('spinbutton', { name: '本次章节数' }).fill('10'))
    await act(async () => page.getByRole('button', { name: '启动批量创作' }).click())
    for (let attempt = 0; attempt < 6; attempt++) {
      await completeNextDraft()
    }
    await act(async () => { await draftReady })
    expect(pendingDraftCompletions).toHaveLength(1)
    expect(draftRecord).toMatchObject({ chapterNumber: 5, content: condensed, wordCount: 1350 })
    await act(async () => useLayoutStore.setState({ bottomTab: 'log' }))
    await expect.element(page.getByText('第5章字数超过约定', { exact: true })).toBeVisible()
    for (let chapter = 6; chapter <= 10; chapter++) {
      await completeNextDraft()
    }
    await act(async () => {
      await vi.waitFor(() => expect(useWorkflowStore.getState().history[0]?.status).toBe('completed'))
    })
    const run = useWorkflowStore.getState().history[0]!
    expect(run.steps).toHaveLength(10)
    expect(run.steps.every(step => step.status === 'completed')).toBe(true)
    expect(run.steps.flatMap(step => step.logs).filter(log => log.includes('字数超过约定'))).toEqual([expect.stringContaining('第5章字数超过约定')])
    const requests = invoke.mock.calls.filter(([channel]) => channel === 'generation_execute')
      .map(([, request]) => (request as { request: { task: { purpose: string; messages: Array<{ content: string }> } } }).request)
    expect(requests.filter(request => request.task.purpose === 'chapter-draft-condense')).toHaveLength(1)
    const drafts = requests.filter(request => request.task.purpose === 'chapter-draft')
    expect(drafts).toHaveLength(10)
    const preparationIndex = invoke.mock.calls.findIndex(([channel, request]) => channel === 'generation_prepare_draft_context'
      && (request as { request: { chapterNumber: number } }).request.chapterNumber === 6)
    const preparation = await invoke.mock.results[preparationIndex].value
    expect(preparation.selectedDrafts).toEqual([expect.objectContaining({ content: condensed, contentHash: await hashAuthorText(condensed) })])
    expect(drafts[5].task.messages.map(message => message.content).join('\n')).toContain(condensed.slice(-500))
    expect(invoke.mock.calls.filter(([channel]) => channel === 'generation_commit_draft')).toHaveLength(10)
    expect(invoke.mock.calls.some(([channel]) => channel === 'generation_pause')).toBe(false)
    expect(draftRecord?.chapterNumber).toBe(10)
    await expect.element(page.getByText('第5章字数超过约定', { exact: true })).toBeVisible()
  })

  it('creates an editable draft in the project tree without finalization side effects', async () => {
    await act(async () => {
      root?.render(<BatchChapterCreationDialog isOpen startChapterNumber={1} onClose={vi.fn()} />)
    })

    await expect.element(page.getByRole('radio', { name: '生成草稿待审' })).toBeChecked()
    await act(async () => page.getByRole('button', { name: '启动批量创作' }).click())

    await vi.waitFor(() => {
      expect(useWorkflowStore.getState().history[0]?.status).toBe('completed')
    })

    expect(useProjectStore.getState().fileTree).toEqual(fileTree())
    expect(useDraftStore.getState().draftsByChapter[1]?.[0]).toMatchObject({
      id: 101,
      status: 'draft',
    })
    expect(useEditorStore.getState().tabs).toEqual([
      expect.objectContaining({
        filePath: 'ai-novel://draft/101',
        type: 'chapter',
        content: DRAFT_TEXT,
        savedContent: DRAFT_TEXT,
      }),
    ])
    expect(useEditorStore.getState().tabs[0]?.dirty).not.toBe(true)
    expect(useWorkflowStore.getState().history[0]).toMatchObject({
      title: '批量草稿待审 — 第1–1章',
      steps: [expect.objectContaining({
        name: '第1章：生成草稿待审',
        result: '第1章草稿已生成并保存，等待审稿。',
      })],
    })
    expect(useWorkflowStore.getState().history[0]?.steps[0]?.logs.some(log => (
      log.includes('开始第1章：生成草稿待审。')
    ))).toBe(true)
    expect(invoke).toHaveBeenCalledWith(
      'generation_begin',
      expect.objectContaining({
        request: expect.objectContaining({ modelId: 'grok-browser' }),
        projectSession: PROJECT_SESSION,
      }),
      undefined,
    )
    expect(invoke.mock.calls.some(([channel]) => channel === 'llm_begin_execution_lease')).toBe(false)
    expect(invoke.mock.calls.some(([channel]) => (
      channel === 'finalization_commit'
      || channel === 'kb_import_text'
      || String(channel).startsWith('db_post_process_')
    ))).toBe(false)
  })

  it('confirms and completes auto-finalize with a read-only project result and post-processing log', async () => {
    initProjectService()
    await act(async () => {
      root?.render(<BatchChapterCreationDialog isOpen startChapterNumber={1} onClose={vi.fn()} />)
    })

    await act(async () => page.getByRole('radio', { name: '自动定稿' }).click())
    await expect.element(page.getByText(/发布实体稿并运行角色与连续性后处理/)).toBeVisible()
    await act(async () => page.getByRole('button', { name: '继续确认自动定稿' }).click())
    expect(invoke.mock.calls.some(([channel]) => channel === 'finalization_commit')).toBe(false)

    await expect.element(page.getByText('即将自动定稿第1–1章（共1章）。完成后章节只读，不能直接编辑。')).toBeVisible()
    await act(async () => page.getByRole('button', { name: '确认自动定稿并启动' }).click())

    await vi.waitFor(() => {
      const completedRun = useWorkflowStore.getState().history[0]
      expect({ status: completedRun?.status, error: completedRun?.error }).toEqual({
        status: 'completed',
        error: undefined,
      })
      expect(useDraftStore.getState().draftsByChapter[1]?.[0]?.status).toBe('finalized')
    })

    expect(useProjectStore.getState().fileTree).toEqual(fileTree())
    expect(useEditorStore.getState().tabs.some(tab => (
      tab.filePath === 'ai-novel://draft/101' && tab.draftStatus !== 'finalized'
    ))).toBe(false)
    expect(useWorkflowStore.getState().history[0]).toMatchObject({
      title: '批量自动定稿 — 第1–1章',
      steps: [expect.objectContaining({
        name: '第1章：自动定稿与后处理',
        result: '第1章已定稿，后处理全部通过。',
      })],
    })
    expect(useWorkflowStore.getState().history[0]?.steps[0]?.logs.some(log => (
      log.includes('开始第1章：生成草稿、自动定稿并完成后处理。')
      && !log.includes('生成草稿待审')
    ))).toBe(true)
    expect(invoke.mock.calls.some(([channel]) => channel === 'finalization_commit')).toBe(true)
    expect(invoke.mock.calls.some(([channel]) => channel === 'kb_import_text')).toBe(true)
    expect(invoke.mock.calls.some(([channel]) => channel === 'db_post_process_mark_step_ok')).toBe(true)
    expect(invoke).toHaveBeenCalledWith(
      'db_continuity_read_source',
      { draftId: 101, expectedProjectPath: PROJECT_PATH, projectSession: PROJECT_SESSION },
      undefined,
    )
    expect(invoke.mock.calls.filter(([channel]) => channel === 'finalization_generation_read')
      .map(([, request]) => (request as { request: { slot: { stepKey: string } } }).request.slot.stepKey))
      .toEqual(['chapter_notes', 'character_cards'])
  })
})
