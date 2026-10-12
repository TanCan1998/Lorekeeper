import { afterEach, describe, expect, it, vi } from 'vitest'
import { syncBlueprintCharacterCandidates } from '../blueprint-character-sync'
import type { CharacterProposalBatch } from '../../../shared/character-proposal'
const projectPath = 'C:\\novels\\candidate-sync'
const session = { projectId: 'candidate-sync', leaseId: 'lease-candidate-sync', projectPath }
afterEach(() => vi.unstubAllGlobals())
describe('directory character proposals', () => {
  it.each(['declared', 'name-only', 'relationship-only'] as const)('stages %s from the durable operation without reading or writing the roster', async kind => {
    const source = { chapterNumber: 1, characters: ['同名', '同名'],
      newCharacterCandidates: kind === 'declared' ? [{ name: '同名', role: 'supporting' as const }] : [],
      relationshipHints: kind === 'relationship-only' ? [{ from: '同名', to: '另一人', relation: '原关系' }] : [] }
    const batch = { proposalBatchId: 'batch', revision: 0, status: 'pending-approval', items: [], source: { kind: 'directory', operationId: 'operation' } } satisfies CharacterProposalBatch
    const invoke = vi.fn(async () => batch)
    vi.stubGlobal('window', { __TAURI_INTERNALS__: { invoke } })
    expect(await syncBlueprintCharacterCandidates([source], projectPath, session, 'operation')).toBe(batch)
    // Tauri 语义：ipc-client 把频道名下划线化（character-proposal:stage →
    // character_proposal_stage），位置参数经 buildNamedArgs 打包为命名参数
    // 对象并追加 projectSession 尾参（真实 core invoke 恒带第 3 个 options 实参）。
    expect(invoke.mock.calls).toEqual([['character_proposal_stage', {
      request: { source: { kind: 'directory', operationId: 'operation' } },
      projectSession: session,
    }, undefined]])
  })
  it('refuses an operation-less request without attempting a name fallback', async () => {
    const invoke = vi.fn(); vi.stubGlobal('window', { __TAURI_INTERNALS__: { invoke } })
    await expect(syncBlueprintCharacterCandidates([], projectPath, session)).rejects.toThrow('CHARACTER_PROPOSAL_DIRECTORY_OPERATION_REQUIRED')
    expect(invoke).not.toHaveBeenCalled()
  })
  it('propagates durable staging failure without formal writes or success receipt', async () => {
    const invoke = vi.fn(async () => { throw new Error('SOURCE_STALE') }); vi.stubGlobal('window', { __TAURI_INTERNALS__: { invoke } })
    await expect(syncBlueprintCharacterCandidates([], projectPath, session, 'operation')).rejects.toThrow('SOURCE_STALE')
    expect(invoke).toHaveBeenCalledOnce()
  })
  it('reuses the exact durable operation on retry and never resends renderer candidate bytes', async () => {
    const invoke = vi.fn(async () => ({ proposalBatchId: 'same-batch' })); vi.stubGlobal('window', { __TAURI_INTERNALS__: { invoke } })
    await syncBlueprintCharacterCandidates([], projectPath, session, 'operation')
    await syncBlueprintCharacterCandidates([{ chapterNumber: 9, characters: ['renderer-changed'], relationshipHints: [] }], projectPath, session, 'operation')
    expect(invoke.mock.calls[0]).toEqual(invoke.mock.calls[1])
    expect(JSON.stringify(invoke.mock.calls)).not.toContain('renderer-changed')
  })
})
