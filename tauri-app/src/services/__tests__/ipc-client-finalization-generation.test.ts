import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import {
  setActiveProjectSessionContext,
} from '../../shared/project-session-context'
import type {
  FinalizationGenerationEffect,
  FinalizationGenerationRecovery,
  FinalizationGenerationSlot,
} from '../../shared/finalization-generation'
import type { FinalizedCharacterArtifact } from '../../shared/finalized-character-generation'
import type {
  MainGenerationExecuteReceipt,
  MainGenerationRunHandle,
  MainGenerationRunView,
} from '../generation/generation-runtime'
import { ipc } from '../ipc-client'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

let tauriInternals: TauriInternalsHandle

const mockSlot: FinalizationGenerationSlot = {
  source: {
    draftId: 10,
    chapterNumber: 1,
    finalizationId: 'fin-1',
    contentHash: '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
  },
  stepKey: 'chapter_notes',
}

const mockHandle: MainGenerationRunHandle = {
  projectId: 'project-A',
  rootActionId: 'root-1',
  runId: 'run-1',
  epoch: 'lease-A',
}

const mockView: MainGenerationRunView = {
  handle: mockHandle,
  status: 'running',
  operation: 'finalized-chapter-notes',
  nonReplayable: false,
  artifacts: [],
  budget: {
    maxAttempts: 3,
    maxRequestedOutputTokens: 2048,
    maxRequestedOutputTokensPerAttempt: 2048,
    deadlineAt: 1234567890,
  },
}

const mockRecovery: FinalizationGenerationRecovery = {
  attemptCount: 0,
  view: mockView,
  modelId: 'mock-model-1',
  context: {
    slot: mockSlot,
    identity: {
      source: mockSlot.source,
      content: 'prose content',
      projectionGeneration: 1,
      identityRevision: 1,
      identityStatus: 'bound',
      sourceOrder: {
        continuityEpoch: 'lease-A',
        chapterNumber: 1,
        authoritativeFinalizationRevision: 1,
      },
      characters: [],
      epoch: 'lease-A',
      projectId: 'project-A',
    },
    chapterTitle: 'Chapter 1',
    chapterEntities: [],
    writingLanguage: 'zh-CN',
    template: {
      key: 'generate_chapter_notes',
      name: 'Generate Chapter Notes',
      description: '',
      content: '',
      variables: {},
    },
    notesBaseline: {
      blueprint: { exists: false, notes: '' },
      continuityHash: 'hash',
    },
  },
  sourceStatus: 'current',
}

const mockExecuteReceipt: MainGenerationExecuteReceipt = {
  outcome: {
    status: 'completed',
    content: 'notes content',
    finishReason: 'stop',
    receipt: {
      attemptId: 'att-fin-1',
      runId: 'run-1',
      attemptNumber: 1,
      durationMs: 150,
    } as unknown as import('../generation/generation-harness').GenerationAttemptReceipt,
  },
  run: mockView,
}

const mockArtifact: FinalizedCharacterArtifact = {
  artifactId: 'art-1',
  revision: 1,
  textHash: 'hash-art-1',
}

const mockEffect: FinalizationGenerationEffect = {
  success: true,
  stepKey: 'chapter_notes',
  chapterNotes: 'notes content',
  factCount: 0,
  blueprintUpdated: true,
}

beforeEach(() => {
  tauriInternals = installTauriInternals({
    commands: {
      finalization_generation_read: mockRecovery,
      finalization_generation_begin: mockRecovery,
      finalization_generation_execute: mockExecuteReceipt,
      finalization_generation_commit: mockEffect,
      finalization_generation_cancel: mockView,
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

describe('finalization-generation IPC transport', () => {
  it('invokes finalization-generation:read with request and injected projectSession', async () => {
    const request = { slot: mockSlot }

    const res = await ipc.invoke('finalization-generation:read', request)

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'finalization_generation_read',
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

  it('invokes finalization-generation:begin with request and injected projectSession', async () => {
    const request = {
      slot: mockSlot,
      modelId: 'mock-model-1',
    }

    const res = await ipc.invoke('finalization-generation:begin', request)

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'finalization_generation_begin',
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

  it('invokes finalization-generation:execute with request and injected projectSession', async () => {
    const request = { handle: mockHandle }

    const res = await ipc.invoke('finalization-generation:execute', request)

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'finalization_generation_execute',
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

  it('invokes finalization-generation:commit with request and injected projectSession', async () => {
    const request = {
      handle: mockHandle,
      artifact: mockArtifact,
    }

    const res = await ipc.invoke('finalization-generation:commit', request)

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'finalization_generation_commit',
      {
        request,
        projectSession: {
          projectId: 'project-A',
          leaseId: 'lease-A',
          projectPath: 'C:/projects/A',
        },
      },
    ])
    expect(res).toEqual(mockEffect)
  })

  it('invokes finalization-generation:cancel with request and injected projectSession', async () => {
    const request = { handle: mockHandle }

    const res = await ipc.invoke('finalization-generation:cancel', request)

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'finalization_generation_cancel',
      {
        request,
        projectSession: {
          projectId: 'project-A',
          leaseId: 'lease-A',
          projectPath: 'C:/projects/A',
        },
      },
    ])
    expect(res).toEqual(mockView)
  })
})
