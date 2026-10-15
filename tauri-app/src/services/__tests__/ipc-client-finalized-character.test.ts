import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import {
  setActiveProjectSessionContext,
} from '../../shared/project-session-context'
import type {
  FinalizedCharacterContext,
  FinalizedCharacterStateDecisionReceipt,
  PendingFinalizedCharacterStateCandidate,
  PendingFinalizedCharacterStateCandidateSummary,
} from '../../shared/finalized-continuity'
import { ipc } from '../ipc-client'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

let tauriInternals: TauriInternalsHandle

const mockSource = {
  draftId: 7,
  finalizationId: 'fin-1',
  chapterNumber: 7,
  contentHash: 'a'.repeat(64),
}

const mockContext = {
  projectId: 'project-A',
  epoch: 'lease-A',
  source: mockSource,
  content: '测试正文',
  projectionGeneration: 3,
  identityRevision: 1,
  identityStatus: 'bound',
  sourceOrder: {
    continuityEpoch: 'project-A:3',
    chapterNumber: 7,
    authoritativeFinalizationRevision: 5,
  },
  characters: [],
} as unknown as FinalizedCharacterContext

const mockReadContext = {
  contextId: 'ctx-1',
  context: mockContext,
  originProjectId: 'origin-1',
}

const mockSummary = {
  draftId: 7,
  candidateKey: 'fcs-state:abc',
  finalizationId: 'fin-1',
  characterId: 'char-1',
  characterName: '角色一',
  field: 'location',
} as PendingFinalizedCharacterStateCandidateSummary

const mockCandidate = {
  ...mockSummary,
  value: '北境',
  source: mockSource,
  expectedFieldRevision: 0,
  expectedFieldValueHash: 'b'.repeat(64),
} as unknown as PendingFinalizedCharacterStateCandidate

const mockReceipt = {
  candidateKey: 'fcs-state:abc',
  operationId: 'op-1',
  payloadHash: 'c'.repeat(64),
  decision: 'accept',
  source: mockSource,
  idempotent: false,
} as FinalizedCharacterStateDecisionReceipt

const session = {
  projectId: 'project-A',
  leaseId: 'lease-A',
  projectPath: 'C:/projects/A',
}

beforeEach(() => {
  tauriInternals = installTauriInternals({
    commands: {
      finalized_character_read_context: mockReadContext,
      finalized_character_list_state_candidates: [mockSummary],
      finalized_character_read_state_candidate: mockCandidate,
      finalized_character_decide_state_candidate: mockReceipt,
    },
  })
  setActiveProjectSessionContext(session)
})

afterEach(() => {
  setActiveProjectSessionContext(null)
  tauriInternals.uninstall()
})

describe('finalized-character IPC transport', () => {
  it('invokes finalized-character:read-context with request and injected projectSession', async () => {
    const request = { draftId: 7 }

    const res = await ipc.invoke('finalized-character:read-context', request)

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'finalized_character_read_context',
      { request, projectSession: session },
    ])
    expect(res).toEqual(mockReadContext)
  })

  it('invokes finalized-character:list-state-candidates without positional args', async () => {
    const res = await ipc.invoke('finalized-character:list-state-candidates')

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'finalized_character_list_state_candidates',
      { projectSession: session },
    ])
    expect(res).toEqual([mockSummary])
  })

  it('invokes finalized-character:read-state-candidate with request', async () => {
    const request = { draftId: 7, candidateKey: 'fcs-state:abc' }

    const res = await ipc.invoke('finalized-character:read-state-candidate', request)

    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'finalized_character_read_state_candidate',
      { request, projectSession: session },
    ])
    expect(res).toEqual(mockCandidate)
  })

  it('invokes finalized-character:decide-state-candidate with request', async () => {
    const request = {
      draftId: 7,
      candidateKey: 'fcs-state:abc',
      characterId: 'char-1',
      field: 'location',
      expectedFieldRevision: 0,
      expectedFieldValueHash: 'b'.repeat(64),
      operationId: 'op-1',
      decision: 'accept',
    } as const

    const res = await ipc.invoke('finalized-character:decide-state-candidate', request)

    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'finalized_character_decide_state_candidate',
      { request, projectSession: session },
    ])
    expect(res).toEqual(mockReceipt)
  })
})
