import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import {
  setActiveProjectSessionContext,
} from '../../shared/project-session-context'
import type { ImportGenerationRecovery, ImportGenerationSlot } from '../../shared/import-generation'
import type { MainGenerationExecuteReceipt } from '../generation/generation-runtime'
import type { GenerationAttemptReceipt, GenerationTask } from '../generation/generation-harness'
import { ipc } from '../ipc-client'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

let tauriInternals: TauriInternalsHandle

const mockSlot: ImportGenerationSlot = {
  runId: 'import-run-1',
  stage: 'global',
  batchId: 'done',
}

const mockRecovery: ImportGenerationRecovery = {
  view: {
    handle: {
      projectId: 'project-A',
      rootActionId: 'root-1',
      runId: 'run-1',
      epoch: 'lease-A',
    },
    status: 'running',
    operation: 'import-global-facts',
    nonReplayable: false,
    artifacts: [],
    budget: {
      maxAttempts: 1,
      maxRequestedOutputTokens: 2048,
      maxRequestedOutputTokensPerAttempt: 2048,
      deadlineAt: 1234567890,
    },
  },
  modelId: 'mock-model-1',
  frozenContext: {
    slot: mockSlot,
    manifestFingerprint: 'mock-fp',
    totalChapters: 10,
    totalWords: 5000,
    core: {
      projectName: '导入小说',
      genre: '都市',
      totalChapters: 10,
    } as unknown as import('@baseline/repositories/project-core-repository').ProjectCoreData,
    prompts: {},
    chapters: [],
  },
}

const mockTask: GenerationTask = {
  purpose: 'import-global-facts',
  output: 'structured-data',
  messages: [
    { role: 'system', content: '系统指令' },
    { role: 'user', content: '用户指令' },
  ],
}

const mockExecuteReceipt: MainGenerationExecuteReceipt = {
  outcome: {
    status: 'completed',
    content: '{"genre":"都市"}',
    finishReason: 'stop',
    receipt: {
      attemptId: 'att-import-1',
      runId: 'run-1',
      attemptNumber: 1,
      durationMs: 200,
    } as unknown as GenerationAttemptReceipt,
  },
  run: mockRecovery.view,
}

beforeEach(() => {
  tauriInternals = installTauriInternals({
    commands: {
      import_generation_read: mockRecovery,
      import_generation_execute: mockExecuteReceipt,
    },
  })
  setActiveProjectSessionContext({
    projectId: 'project-A',
    leaseId: 'lease-A',
    projectPath: 'C:/projects/A',
  })
})

afterEach(() => {
  setActiveProjectSessionContext(null)
  tauriInternals.uninstall()
})

describe('import-generation IPC transport', () => {
  it('invokes import-generation:read with request and injected projectSession', async () => {
    const request = { slot: mockSlot }

    const res = await ipc.invoke('import-generation:read', request)

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'import_generation_read',
      {
        request,
        projectSession: {
          projectId: 'project-A',
          leaseId: 'lease-A',
          projectPath: 'C:/projects/A',
        },
      },
    ])
    expect(res).toEqual(mockRecovery)
  })

  it('invokes import-generation:execute with request and injected projectSession', async () => {
    const request = {
      handle: mockRecovery.view.handle,
      ordinal: 0,
      task: mockTask,
      execution: {
        owner: 'renderer-1',
        epoch: 1,
      },
    }

    const res = await ipc.invoke('import-generation:execute', request)

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'import_generation_execute',
      {
        request,
        projectSession: {
          projectId: 'project-A',
          leaseId: 'lease-A',
          projectPath: 'C:/projects/A',
        },
      },
    ])
    expect(res).toEqual(mockExecuteReceipt)
  })
})
