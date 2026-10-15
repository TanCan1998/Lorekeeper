import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import {
  setActiveProjectSessionContext,
} from '../../shared/project-session-context'
import type {
  GraphGenerationArtifact,
  GraphGenerationEffect,
  GraphGenerationInput,
  GraphGenerationRecovery,
} from '../../shared/graph-generation'
import type {
  MainGenerationExecuteReceipt,
  MainGenerationRunHandle,
  MainGenerationRunView,
} from '../generation/generation-runtime'
import { ipc } from '../ipc-client'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

let tauriInternals: TauriInternalsHandle

const mockInput: GraphGenerationInput = {
  kind: 'plot',
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
  operation: 'plot-tree-snapshot',
  nonReplayable: false,
  artifacts: [],
  budget: {
    maxAttempts: 1,
    maxRequestedOutputTokens: 4096,
    maxRequestedOutputTokensPerAttempt: 4096,
    deadlineAt: 1234567890,
  },
}

const mockRecovery: GraphGenerationRecovery = {
  attemptCount: 0,
  view: mockView,
  modelId: 'mock-model-1',
  context: {
    projectId: 'project-A',
    originEpoch: 'lease-A',
    key: 'graph:mock:1',
    createdAt: '2026-10-15T00:00:00.000Z',
    input: mockInput,
    writingLanguage: 'zh-CN',
    kind: 'plot',
    sources: {
      writingLanguage: 'zh-CN',
      synopsis: {
        content: '大纲概要',
      },
      blueprints: [],
      finalizedChapters: [],
      narrativeThreads: [],
      sourceRevision: 1,
      snapshot: null,
    } as unknown as import('../../shared/plot-tree').PlotTreeSourceBundle,
    targetBaselineHash: 'baseline-hash-1',
  },
  effects: [],
  sourceStatus: 'current',
}

const mockExecuteReceipt: MainGenerationExecuteReceipt = {
  outcome: {
    status: 'completed',
    content: '{"snapshot":{}}',
    finishReason: 'stop',
    receipt: {
      attemptId: 'att-graph-1',
      runId: 'run-1',
      attemptNumber: 1,
      durationMs: 250,
    } as unknown as import('../generation/generation-harness').GenerationAttemptReceipt,
  },
  run: mockView,
}

const mockArtifact: GraphGenerationArtifact = {
  artifactId: 'art-graph-1',
  revision: 1,
  textHash: 'hash-graph-1',
}

const mockEffect: GraphGenerationEffect = {
  success: true,
  index: 0,
  kind: 'plot',
  snapshot: {
    nodes: [],
    edges: [],
  } as unknown as import('../../shared/plot-tree').PlotTreeSnapshot,
  derivation: 'model',
}

beforeEach(() => {
  tauriInternals = installTauriInternals({
    commands: {
      graph_generation_read: mockRecovery,
      graph_generation_begin: mockRecovery,
      graph_generation_execute: mockExecuteReceipt,
      graph_generation_confirm: mockEffect,
      graph_generation_cancel: mockView,
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

describe('graph-generation IPC transport', () => {
  it('invokes graph-generation:read with request and injected projectSession', async () => {
    const request = { handle: mockHandle }

    const res = await ipc.invoke('graph-generation:read', request)

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'graph_generation_read',
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

  it('invokes graph-generation:begin with request and injected projectSession', async () => {
    const request = {
      input: mockInput,
      modelId: 'mock-model-1',
      uiActionNonce: 'graph:test:1',
    }

    const res = await ipc.invoke('graph-generation:begin', request)

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'graph_generation_begin',
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

  it('invokes graph-generation:execute with request and injected projectSession', async () => {
    const request = { handle: mockHandle }

    const res = await ipc.invoke('graph-generation:execute', request)

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'graph_generation_execute',
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

  it('invokes graph-generation:confirm with request and injected projectSession', async () => {
    const request = {
      handle: mockHandle,
      artifact: mockArtifact,
      index: 0,
    }

    const res = await ipc.invoke('graph-generation:confirm', request)

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'graph_generation_confirm',
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

  it('invokes graph-generation:cancel with request and injected projectSession', async () => {
    const request = { handle: mockHandle }

    const res = await ipc.invoke('graph-generation:cancel', request)

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'graph_generation_cancel',
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
