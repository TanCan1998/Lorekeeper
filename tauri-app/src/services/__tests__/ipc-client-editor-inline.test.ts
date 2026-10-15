import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import {
  setActiveProjectSessionContext,
} from '../../shared/project-session-context'
import type { EditorInlineInput, EditorInlineRecovery } from '../../shared/editor-inline-generation'
import type { MainGenerationExecuteReceipt } from '../generation/generation-runtime'
import type { GenerationAttemptReceipt } from '../generation/generation-harness'
import { ipc } from '../ipc-client'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

let tauriInternals: TauriInternalsHandle

const mockInput: EditorInlineInput = {
  action: 'refine',
  documentText: '春眠不觉晓，处处闻啼鸟。',
  from: 0,
  to: 6,
  selectedText: '春眠不觉晓，',
}

const mockRecovery: EditorInlineRecovery = {
  view: {
    handle: {
      projectId: 'project-A',
      rootActionId: 'root-1',
      runId: 'run-1',
      epoch: 'lease-A',
    },
    status: 'running',
    operation: 'editor-inline',
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
  context: {
    ...mockInput,
    kind: 'author-draft',
    documentHash: 'mock-hash',
    writingLanguage: 'zh-CN',
    template: {
      key: 'edit_selected_text',
      name: '编辑选中文本',
      description: '',
      content: '{{edit_instruction}}\\n{{selected_text}}',
      variables: {},
    },
  },
  sourceStatus: 'current',
}

const mockExecuteReceipt: MainGenerationExecuteReceipt = {
  outcome: {
    status: 'completed',
    content: '夜来风雨声，花落知多少。',
    finishReason: 'stop',
    receipt: {
      attemptId: 'att-1',
      runId: 'run-1',
      attemptNumber: 1,
      durationMs: 120,
    } as unknown as GenerationAttemptReceipt,
  },
  run: mockRecovery.view,
}

beforeEach(() => {
  tauriInternals = installTauriInternals({
    commands: {
      editor_inline_begin: mockRecovery,
      editor_inline_read_recovery: mockRecovery,
      editor_inline_execute: mockExecuteReceipt,
      editor_inline_cancel: mockRecovery.view,
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

describe('editor-inline IPC transport', () => {
  it('invokes editor-inline:begin with request and injected projectSession', async () => {
    const request = {
      input: mockInput,
      modelId: 'mock-model-1',
      uiActionNonce: 'nonce-inline-1',
    }

    const res = await ipc.invoke('editor-inline:begin', request)

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'editor_inline_begin',
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

  it('invokes editor-inline:read-recovery with request and injected projectSession', async () => {
    const request = { handle: mockRecovery.view.handle }

    const res = await ipc.invoke('editor-inline:read-recovery', request)

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'editor_inline_read_recovery',
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

  it('invokes editor-inline:execute with request and injected projectSession', async () => {
    const request = { handle: mockRecovery.view.handle }

    const res = await ipc.invoke('editor-inline:execute', request)

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'editor_inline_execute',
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

  it('invokes editor-inline:cancel with request and injected projectSession', async () => {
    const request = { handle: mockRecovery.view.handle }

    const res = await ipc.invoke('editor-inline:cancel', request)

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'editor_inline_cancel',
      {
        request,
        projectSession: {
          projectId: 'project-A',
          leaseId: 'lease-A',
          projectPath: 'C:/projects/A',
        },
      },
    ])
    expect(res).toEqual(mockRecovery.view)
  })
})
