import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useProjectStore } from '../../../../stores/project-store'
import { useLocaleStore } from '../../../../stores/locale-store'
import { useWorkflowStore } from '../../../../stores/workflow-store'
import { runAgentLoop } from '../../agent-engine'
import { toolRegistry, type AgentExecutionContext } from '../../tool-registry'
import type { AgentToolActionRef } from '../../../../shared/agent-generation'
import { launchCreativeWorkflow } from '../../../workflows/creative-workflow-launcher'
import { PROJECT_FACT_TARGETS } from '../../../project-fact-targets'
import { createAgentExecutionContext } from '../project-context'
import { startWorkflowTool as productionStartWorkflowTool } from '../start-workflow.tool'
import { writeFileTool } from '../write-file.tool'
import * as writingSkillBindings from '../../writing-skill-bindings'
import { GenerateDraftCommand } from '../../../workflows/commands/generate-draft.command'

const projectPath = 'C:\\novels\\issue-90'
const project = {
  id: 'issue-90',
  sessionLease: 'lease-issue-90',
  name: 'Issue 90 reproduction',
  path: projectPath,
  novelConfig: {
    genre: '奇幻',
    subGenre: '',
    targetAudience: '',
    totalChapters: 10,
    wordsPerChapter: 3000,
    plotStructure: 'three_act',
    narrativePOV: 'third_limited',
    coreOutline: '林舟必须在永夜吞没城市前找回太阳。',
    worldSetting: '',
    goldenFinger: '',
    protagonistProfile: '',
    globalGuidance: '',
  },
}

// This launcher test supplies a synthetic main-issued action. Real source and
// action admission are covered by the main SQLite tests; the launcher, guards,
// workflow registration, locale and cancellation below remain real.
const issuedActions = new Map<string, { args: Record<string, unknown>; modelId: string }>()
const startWorkflowTool = {
  ...productionStartWorkflowTool,
  execute: (args: Record<string, unknown>, context?: AgentExecutionContext) => {
    const ref: AgentToolActionRef = { handle: { projectId: project.id, epoch: project.sessionLease,
      rootActionId: 'synthetic-agent-root', runId: 'synthetic-agent-run' },
      attemptId: 'synthetic-attempt', toolCallId: `synthetic-action-${issuedActions.size}` }
    issuedActions.set(ref.toolCallId, { args: structuredClone(args), modelId: context?.selectedModelId ?? 'model' })
    return productionStartWorkflowTool.execute(args, { ...(context ?? createAgentExecutionContext()), agentToolAction: ref })
  },
}

function stubWorkflowIpc(overrides: Partial<Record<string, unknown>> = {}) {
  const invoke = vi.fn((command: string, args?: Record<string, unknown>) => {
    // Tauri 语义：频道名下划线化；位置参数经 buildNamedArgs 打包为
    // 命名参数对象（单对象参数登记为 `request`）并追加 projectSession 尾参。
    if (command in overrides) return Promise.resolve(overrides[command])
    if (command === 'agent_generation_register_workflow') {
      const ref = (args as { request: { ref: AgentToolActionRef } }).request.ref
      const issued = issuedActions.get(ref.toolCallId)
      if (!issued) return Promise.reject(new Error('Synthetic main action missing'))
      if (['review', 'refine', 'finalize'].includes(String(issued.args.workflow))) {
        return Promise.reject(new Error('工作流需要明确的草稿身份'))
      }
      return Promise.resolve({ registrationId: `registration-${ref.toolCallId}`, workflow: issued.args.workflow,
        chapterNumber: issued.args.chapter_number, parentHandle: ref.handle, modelId: issued.modelId,
        state: 'registered', childHandles: [] })
    }
    if (command === 'db_project_core_get') {
      const longEnough = '完整架构信息'.repeat(20)
      return Promise.resolve({ premise: longEnough, charactersArch: longEnough, worldbuilding: longEnough, synopsis: longEnough })
    }
    if (command === 'db_character_get_all') return Promise.resolve([{ name: '林舟' }])
    if (command === 'db_blueprint_get_all') return Promise.resolve([{ chapterNumber: 1 }])
    if (command === 'db_blueprint_get') {
      return Promise.resolve({
        chapterNumber: 1,
        title: '最后的灯',
        role: '开篇',
        purpose: '建立核心冲突',
        keyEvents: '林舟点亮灯塔',
        characters: ['林舟'],
        suspenseHook: '太阳去了哪里',
        userGuidance: '',
        notes: '',
        notesUpdatedAt: '',
      })
    }
    // Keep the real workflow registered while this launcher-seam test observes it.
    return new Promise(() => {})
  })
  vi.stubGlobal('window', { __TAURI_INTERNALS__: { invoke } })
  return invoke
}

beforeEach(() => {
  issuedActions.clear()
  stubWorkflowIpc()
  useLocaleStore.setState({ locale: 'zh-CN' })
  useProjectStore.setState({ currentProject: project as never })
  useWorkflowStore.setState({
    activeRuns: [],
    history: [],
    globalLogs: [],
    waitingRuns: {},
    currentRun: null,
    waitingForConfirm: false,
    waitingAfterStepIndex: -1,
  })
})

afterEach(() => {
  vi.useRealTimers()
  toolRegistry.unregister('start_workflow')
  vi.unstubAllGlobals()
  useProjectStore.setState({ currentProject: null })
})

describe('Issue #90 AI assistant project actions', () => {
  it.each([
    {
      label: 'architecture config',
      intent: { workflow: 'generate_architecture' } as const,
      prepare: () => useProjectStore.setState({ currentProject: { ...project, novelConfig: { ...project.novelConfig, coreOutline: '', protagonistProfile: '', worldSetting: '' } } as never }),
      expected: '小说配置',
    },
    {
      label: 'directory prerequisites',
      intent: { workflow: 'generate_blueprint' } as const,
      prepare: () => stubWorkflowIpc({ db_project_core_get: { premise: '', charactersArch: '', worldbuilding: '', synopsis: '' } }),
      expected: '故事前提',
    },
    {
      label: 'draft character roster',
      intent: { workflow: 'generate_draft', chapterNumber: 1 } as const,
      prepare: () => stubWorkflowIpc({ db_character_get_all: [] }),
      expected: '角色卡',
    },
    {
      label: 'previous chapter finalization',
      intent: { workflow: 'generate_draft', chapterNumber: 2 } as const,
      prepare: () => stubWorkflowIpc({ db_draft_get_finalized: null }),
      expected: '第 1 章尚未定稿',
    },
    {
      label: 'previous chapter post-processing',
      intent: { workflow: 'generate_draft', chapterNumber: 2 } as const,
      prepare: () => stubWorkflowIpc({
        db_draft_get_finalized: { id: 1 },
        db_post_process_get_latest_run: {
          id: 'post-1', sourceLabel: '第1章定稿', createdAt: '', updatedAt: '', allCriticalPassed: false,
        },
        db_post_process_get_steps: [{
          id: 1, runId: 'post-1', stepKey: 'chapter_notes', label: '章节要点', critical: true,
          ok: false, errorMsg: 'failed', attemptCount: 1, completedAt: '', lastAttemptAt: '',
        }],
      }),
      expected: '定稿后处理未完成',
    },
  ])('rejects failed canonical $label guard before registering a run', async ({ intent, prepare, expected }) => {
    prepare()
    const projectSession = createAgentExecutionContext().projectSession
    if (!projectSession) throw new Error('test project session missing')

    await expect(launchCreativeWorkflow(intent, projectSession)).rejects.toThrow(expected)
    expect(useWorkflowStore.getState().activeRuns).toEqual([])
    expect(useWorkflowStore.getState().history).toEqual([])
  })

  it('returns no workflow artifact when a canonical guard rejects the Agent action', async () => {
    stubWorkflowIpc({ db_character_get_all: [] })

    const result = await startWorkflowTool.execute({ workflow: 'generate_draft', chapter_number: 1 }, createAgentExecutionContext())

    expect(result).toMatchObject({ success: false })
    expect('artifacts' in result).toBe(false)
    expect(useWorkflowStore.getState().activeRuns).toEqual([])
    expect(useWorkflowStore.getState().history).toEqual([])
  })

  it('launches through the public seam only after a real run is registered', async () => {
    stubWorkflowIpc()
    const projectSession = createAgentExecutionContext().projectSession
    if (!projectSession) throw new Error('test project session missing')

    const receipt = await launchCreativeWorkflow({
      workflow: 'generate_draft',
      chapterNumber: 1,
    }, projectSession)

    expect(receipt).toMatchObject({
      accepted: true,
      workflow: 'generate_draft',
      projectPath,
      runId: expect.any(String),
      status: 'running',
    })
    expect(useWorkflowStore.getState().activeRuns).toContainEqual(expect.objectContaining({
      id: receipt.runId,
      projectSession,
    }))
  })

  it('marks the write boundary when the Agent registers a real workflow run', async () => {
    stubWorkflowIpc()
    const markSideEffectStarted = vi.fn()

    const result = await startWorkflowTool.execute(
      { workflow: 'generate_draft', chapter_number: 1 },
      { ...createAgentExecutionContext(), markSideEffectStarted },
    )

    expect(result.success).toBe(true)
    expect(markSideEffectStarted).toHaveBeenCalledTimes(1)
  })

  it('does not register a workflow when the Agent is stopped during an asynchronous guard', async () => {
    let finishGuard: ((core: { premise: string; charactersArch: string; worldbuilding: string; synopsis: string }) => void) | undefined
    const guardResult = new Promise<{ premise: string; charactersArch: string; worldbuilding: string; synopsis: string }>((resolve) => {
      finishGuard = resolve
    })
    const invoke = stubWorkflowIpc({ db_project_core_get: guardResult })
    const controller = new AbortController()
    const markSideEffectStarted = vi.fn()
    const launched = startWorkflowTool.execute(
      { workflow: 'generate_blueprint' },
      { ...createAgentExecutionContext(), abortSignal: controller.signal, markSideEffectStarted },
    )
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith(
      'db_project_core_get',
      { expectedProjectPath: projectPath, projectSession: expect.anything() },
      undefined,
    ))

    controller.abort()
    const complete = '完整架构信息'.repeat(20)
    finishGuard?.({ premise: complete, charactersArch: complete, worldbuilding: complete, synopsis: complete })
    const result = await launched

    expect(result).toMatchObject({ success: false, error: expect.stringMatching(/取消|cancel/u) })
    expect(markSideEffectStarted).not.toHaveBeenCalled()
    expect(useWorkflowStore.getState().activeRuns).toEqual([])
    expect(useWorkflowStore.getState().history).toEqual([])
  })

  it('does not register a late workflow after an asynchronous guard exceeds the Agent tool timeout', async () => {
    vi.useFakeTimers()
    let finishGuard: ((core: { premise: string; charactersArch: string; worldbuilding: string; synopsis: string }) => void) | undefined
    const guardResult = new Promise<{ premise: string; charactersArch: string; worldbuilding: string; synopsis: string }>((resolve) => {
      finishGuard = resolve
    })
    stubWorkflowIpc({ db_project_core_get: guardResult })
    toolRegistry.register(startWorkflowTool)
    const callbacks = {
      onTextChunk: vi.fn(),
      onToolCallStart: vi.fn(),
      onToolCallComplete: vi.fn(),
      onToolCallConfirmRequired: vi.fn(async () => true),
      onDone: vi.fn(),
      onError: vi.fn(),
    }
    const run = runAgentLoop(
      'system',
      [],
      '生成章节蓝图',
      'model',
      vi.fn()
        .mockResolvedValueOnce('start_workflow\n{"workflow":"generate_blueprint"}')
        .mockResolvedValueOnce('已停止。'),
      callbacks,
      undefined,
      createAgentExecutionContext(),
    )

    await vi.advanceTimersByTimeAsync(30_000)
    await run
    const complete = '完整架构信息'.repeat(20)
    finishGuard?.({ premise: complete, charactersArch: complete, worldbuilding: complete, synopsis: complete })
    await vi.advanceTimersByTimeAsync(0)

    expect(callbacks.onToolCallComplete).toHaveBeenCalledWith(expect.objectContaining({
      status: 'failed',
      error: expect.stringContaining('超时'),
    }))
    expect(useWorkflowStore.getState().activeRuns).toEqual([])
    expect(useWorkflowStore.getState().history).toEqual([])
  })

  it('freezes the English locale before asynchronous draft guards complete', async () => {
    stubWorkflowIpc()
    useLocaleStore.setState({ locale: 'en-US' })
    const projectSession = createAgentExecutionContext().projectSession
    if (!projectSession) throw new Error('test project session missing')

    const launched = launchCreativeWorkflow({ workflow: 'generate_draft', chapterNumber: 1 }, projectSession)
    useLocaleStore.setState({ locale: 'zh-CN' })
    const receipt = await launched

    expect(useWorkflowStore.getState().activeRuns).toContainEqual(expect.objectContaining({
      id: receipt.runId,
      uiLocale: 'en-US',
      title: 'Draft — Chapter 1 · 最后的灯',
      steps: [expect.objectContaining({ name: 'Draft chapter' })],
    }))
  })

  it.each([
    { workflow: 'generate_draft', chapter_number: 1 },
    { workflow: 'generate_architecture' },
    { workflow: 'generate_blueprint' },
  ])('returns a real launch receipt for $workflow', async (args) => {
    stubWorkflowIpc()

    const result = await startWorkflowTool.execute(args, createAgentExecutionContext())

    expect(result).toMatchObject({
      success: true,
      artifacts: [{
        type: 'workflow_started',
        projectPath,
        runId: expect.any(String),
        status: 'running',
      }],
    })
    const runId = result.artifacts?.[0]?.runId
    expect(useWorkflowStore.getState().activeRuns).toContainEqual(expect.objectContaining({
      id: runId,
      projectPath,
      status: 'running',
    }))
  })

  it('将主进程登记身份和父根传到实际草稿命令，重放不重复启动命令', async () => {
    const freeze = vi.spyOn(writingSkillBindings, 'freezeWritingSkillsSnapshot').mockResolvedValue({})
    let finish!: (text: string) => void
    const pending = new Promise<string>(resolve => { finish = resolve })
    const execute = vi.spyOn(GenerateDraftCommand.prototype, 'execute').mockReturnValue(pending)
    const invoke = stubWorkflowIpc()
    try {
      const result = await startWorkflowTool.execute({ workflow: 'generate_draft', chapter_number: 1 }, createAgentExecutionContext())
      await vi.waitFor(() => expect(execute).toHaveBeenCalledTimes(1))
      const context = execute.mock.calls[0][0].context
      expect(context).toMatchObject({ runId: 'registration-synthetic-action-0', generationModelId: 'model',
        agentWorkflowRegistrationId: 'registration-synthetic-action-0',
        mainGenerationRootHandle: { projectId: project.id, epoch: project.sessionLease,
          rootActionId: 'synthetic-agent-root', runId: 'synthetic-agent-run' } })
      expect(result.artifacts?.[0]?.runId).toBe(context.runId)
      const registration = { registrationId: context.runId, workflow: 'generate_draft', chapterNumber: 1,
        parentHandle: context.mainGenerationRootHandle, modelId: 'model', state: 'registered', childHandles: [] }
      const original = invoke.getMockImplementation()!
      invoke.mockImplementation((command, args) => command === 'agent_generation_register_workflow'
        ? Promise.resolve(registration) : original(command, args))
      const replay = await startWorkflowTool.execute({ workflow: 'generate_draft', chapter_number: 1 }, createAgentExecutionContext())
      expect(replay.artifacts?.[0]?.runId).toBe(context.runId)
      expect(execute).toHaveBeenCalledTimes(1)
      expect(useWorkflowStore.getState().activeRuns).toHaveLength(1)
      finish('合成草稿命令结果')
      await vi.waitFor(() => expect(useWorkflowStore.getState().history).toContainEqual(expect.objectContaining({ id: context.runId, status: 'completed' })))
    } finally {
      finish('合成草稿命令结果')
      execute.mockRestore()
      freeze.mockRestore()
    }
  })

  it('returns an English start receipt from the frozen project writing language', async () => {
    stubWorkflowIpc()
    useLocaleStore.setState({ locale: 'zh-CN' })
    useProjectStore.setState({
      currentProject: {
        ...project,
        novelConfig: { ...project.novelConfig, writingLanguage: 'en-US' },
      } as never,
    })

    const context = createAgentExecutionContext()
    const result = await startWorkflowTool.execute(
      { workflow: 'generate_draft', chapter_number: 1 },
      context,
    )

    expect(result.success).toBe(true)
    expect(result.content).toMatch(/^Started the draft \(Chapter 1\) workflow/)
    expect(result.content).not.toMatch(/[\u3400-\u9fff]/u)

    const failure = await startWorkflowTool.execute(
      { workflow: 'review', chapter_number: 1 },
      context,
    )
    expect(failure.error).toBe('Could not start the review (Chapter 1) workflow.')
  })

  it('executes a confirmed raw start_workflow response through the real draft launcher', async () => {
    stubWorkflowIpc()
    toolRegistry.register(startWorkflowTool)
    const callbacks = {
      onTextChunk: vi.fn(),
      onToolCallStart: vi.fn(),
      onToolCallComplete: vi.fn(),
      onToolCallConfirmRequired: vi.fn(async () => true),
      onDone: vi.fn(),
      onError: vi.fn(),
    }
    const generate = vi.fn()
      .mockResolvedValueOnce('start_workflow\n{"workflow":"generate_draft","chapter_number":1}')
      .mockResolvedValueOnce('已开始生成第一章。')

    await runAgentLoop(
      'system',
      [],
      '生成第一章',
      'model',
      generate,
      callbacks,
      undefined,
      createAgentExecutionContext(),
    )

    expect(callbacks.onToolCallConfirmRequired).toHaveBeenCalledOnce()
    expect(callbacks.onTextChunk).toHaveBeenCalledWith('已开始生成第一章。')
    expect(callbacks.onTextChunk).not.toHaveBeenCalledWith(expect.stringContaining('start_workflow'))
    expect(useWorkflowStore.getState().activeRuns).toContainEqual(expect.objectContaining({
      type: 'chapter_creation',
      projectPath,
      status: 'running',
    }))
  })

  it('freezes the Agent-selected model into a confirmed draft workflow instead of falling back to the default model', async () => {
    stubWorkflowIpc()
    toolRegistry.register(startWorkflowTool)
    const callbacks = {
      onTextChunk: vi.fn(),
      onToolCallStart: vi.fn(),
      onToolCallComplete: vi.fn(),
      onToolCallConfirmRequired: vi.fn(async () => true),
      onDone: vi.fn(),
      onError: vi.fn(),
    }
    const generate = vi.fn()
      .mockResolvedValueOnce('start_workflow\n{"workflow":"generate_draft","chapter_number":1,"model_id":"untrusted-model"}')
      .mockResolvedValueOnce('已开始生成第一章。')

    await runAgentLoop(
      'system',
      [],
      '生成第一章',
      'grok-selected-model',
      generate,
      callbacks,
    )

    expect(useWorkflowStore.getState().activeRuns).toContainEqual(expect.objectContaining({
      type: 'chapter_creation',
      projectPath,
      generationModelId: 'grok-selected-model',
      chapterWordsTarget: 3000,
      resourceKeys: ['chapter:1'],
    }))
  })

  it.each(['review', 'refine', 'finalize'])('fails closed when %s lacks a draft identity', async (workflow) => {
    const result = await startWorkflowTool.execute({
      workflow,
      chapter_number: 1,
    }, createAgentExecutionContext())

    expect(result).toMatchObject({ success: false })
    expect(result.error).toContain('草稿')
  })

  it.each(PROJECT_FACT_TARGETS.flatMap(target => target.fileNames.map(fileName => [
    `nested/${fileName}`,
    target.workflow,
  ] as const)))('rejects reserved semantic target %s and names the domain workflow', async (filePath, workflow) => {
    const invoke = stubWorkflowIpc()

    const result = await writeFileTool.execute({
      file_path: filePath,
      content: '不应成为游离的项目事实',
    }, createAgentExecutionContext())

    expect(result).toMatchObject({ success: false })
    expect(result.error).toContain(workflow)
    expect(invoke).not.toHaveBeenCalled()
  })
})
