import { afterEach, describe, expect, it, vi } from 'vitest'
import { ipc } from '../../ipc-client'
import { useProjectStore } from '../../../stores/project-store'
import type { ProjectData } from '../../../shared/ipc-channels'
import type { WorkflowContext, StepCallbacks } from '../../../stores/workflow-store'
import type { MainGenerationRunHandle, MainGenerationRunView } from '../../generation/generation-runtime'
import { createWorkflowMainGenerationRuntime } from '../workflow-main-generation'

const session = { projectId: 'synthetic-project', leaseId: 'new-epoch', projectPath: 'C:/synthetic/workflow' }
const child: MainGenerationRunHandle = { projectId: session.projectId, epoch: session.leaseId, rootActionId: 'parent-root', runId: 'child-draft' }
const view: MainGenerationRunView = { handle: child, status: 'running', nonReplayable: false, artifacts: [],
  budget: { maxAttempts: 8, maxRequestedOutputTokens: 32000, maxRequestedOutputTokensPerAttempt: 4096, deadlineAt: 9999999999999 } }

function setup(registration?: string) {
  const invoke = vi.fn(async (command: string) => {
    if (['generation_begin', 'generation_read', 'generation_resume', 'generation_pause'].includes(command)) return view
    throw new Error(`未预期的合成命令：${command}`)
  })
  vi.stubGlobal('window', { __TAURI_INTERNALS__: { invoke } })
  const unsubscribe = vi.fn()
  vi.spyOn(ipc, 'on').mockReturnValue(unsubscribe)
  useProjectStore.setState({ currentProject: { id: session.projectId, path: session.projectPath, sessionLease: session.leaseId } as ProjectData })
  const context = { runId: 'renderer-workflow', projectPath: session.projectPath, projectSession: session,
    generationModelId: 'synthetic-model', data: {}, cancelled: false,
    mainGenerationRootHandle: { ...child, epoch: 'old-epoch', runId: 'agent-parent' },
    ...(registration ? { agentWorkflowRegistrationId: registration } : {}),
  } as WorkflowContext
  const callbacks: StepCallbacks = { log: vi.fn(), setProgress: vi.fn(), appendText: vi.fn(), replaceText: vi.fn() }
  return { invoke, context, callbacks, unsubscribe }
}

afterEach(() => {
  useProjectStore.setState({ currentProject: null })
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe('草稿工作流主进程接线', () => {
  it('已登记草稿携带原父根和登记身份，旧父纪元不替换当前会话', async () => {
    const f = setup('registered-draft')
    const runtime = await createWorkflowMainGenerationRuntime({ context: f.context, callbacks: f.callbacks,
      selection: { operation: 'chapter-draft', chapterNumber: 3, promptKeys: [], skillStages: [], output: 'visible-text' } })
    try {
      expect(f.invoke).toHaveBeenCalledWith('generation_begin', { selection: {
        operation: 'chapter-draft', chapterNumber: 3, promptKeys: [], skillStages: [], output: 'visible-text',
        selectedDraftIds: [], selectedFinalizedDraftIds: [], modelId: 'synthetic-model',
        uiActionNonce: 'renderer-workflow:chapter-draft:3', agentWorkflowRegistrationId: 'registered-draft', parentRootActionId: 'parent-root',
      }, projectSession: session }, undefined)
      expect(f.context.mainGenerationRunHandle).toEqual(child)
      expect(f.context.mainGenerationRootHandle).toEqual(child)
      expect(f.invoke.mock.calls.some(([command]) => command === 'generation_execute')).toBe(false)
    } finally { await runtime.close() }
    expect(f.unsubscribe).toHaveBeenCalledTimes(2)
  })

  it('普通草稿不继承已登记工作流的跨纪元父根例外', async () => {
    const f = setup()
    const originalRoot = f.context.mainGenerationRootHandle
    await expect(createWorkflowMainGenerationRuntime({ context: f.context, callbacks: f.callbacks,
      selection: { operation: 'chapter-draft', chapterNumber: 3, promptKeys: [], skillStages: [], output: 'visible-text' } }))
      .rejects.toThrow('GENERATION_WORKFLOW_RESUME_REQUIRED')
    expect(f.invoke).not.toHaveBeenCalled()
    expect(f.context.mainGenerationRootHandle).toBe(originalRoot)
    expect(f.context.requestMainGenerationCancellation).toBeUndefined()
  })

  it('登记不允许显式父根与工作流父根不一致', async () => {
    const f = setup('registered-draft')
    await expect(createWorkflowMainGenerationRuntime({ context: f.context, callbacks: f.callbacks,
      selection: { operation: 'chapter-draft', chapterNumber: 3, parentRootActionId: 'another-root', promptKeys: [], skillStages: [], output: 'visible-text' } }))
      .rejects.toThrow('GENERATION_WORKFLOW_ROOT_CHANGED')
    expect(f.invoke).not.toHaveBeenCalled()
    expect(f.context.mainGenerationRunHandle).toBeUndefined()
  })
})
