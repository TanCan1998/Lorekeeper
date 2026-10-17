import { afterEach, describe, expect, it, vi } from 'vitest'
import type { GraphGenerationRecovery } from '../../shared/graph-generation'
import type { EditorInlineRecovery } from '../../shared/editor-inline-generation'
import type { MainGenerationRunView, MainGenerationExecuteReceipt } from '../generation/generation-runtime'
import { createEditorInlineGeneration } from '../editor-inline-generation'
import { createGraphGeneration } from '../graph-generation'
import { ipc } from '../ipc-client'

const session = { projectId: 'synthetic-project', leaseId: 'current-lease', projectPath: 'C:/synthetic-recovery' }
const handle = { projectId: session.projectId, epoch: 'old-lease', rootActionId: 'root', runId: 'run' }
function view(epoch: string): MainGenerationRunView {
  return { handle: { ...handle, epoch }, status: 'running', operation: 'editor-inline', nonReplayable: false, artifacts: [], ledger: { physicalRequests: 0, tokenLiability: 0, activeElapsedMs: 0, blockedCode: null, policy: { maxPhysicalRequests: 1, maxTokenLiability: 10000, maxOutputPerRequest: 4096, maxActiveElapsedMs: 120000 } }, budget: { maxAttempts: 1, maxRequestedOutputTokens: 4096, maxRequestedOutputTokensPerAttempt: 4096, deadlineAt: 9999999999999 } } as MainGenerationRunView
}
async function recovery(kind: string, epoch: string) {
  const hash = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode('甲乙'))), b => b.toString(16).padStart(2, '0')).join('')
  // 只模拟客户端传输状态；真实恢复/来源/预算权威由Rust和隐藏WebView另验。
  return (kind === 'graph'
    ? { view: view(epoch), sourceStatus: epoch === session.leaseId ? 'current' : 'conflict', effects: [], attemptCount: 0, context: { projectId: session.projectId, kind: 'plot', input: { kind: 'plot' } } }
    : { view: view(epoch), sourceStatus: epoch === session.leaseId ? 'current' : 'conflict', context: { kind: 'author-draft', action: 'refine', documentText: '甲乙', from: 0, to: 1, selectedText: '甲', documentHash: hash } }) as unknown as GraphGenerationRecovery | EditorInlineRecovery
}
afterEach(() => vi.restoreAllMocks())

for (const kind of ['graph', 'editor']) describe(`${kind}显式恢复边界`, () => {
  const create = () => kind === 'graph' ? createGraphGeneration(session) : createEditorInlineGeneration(session)
  const executeChannel = kind === 'graph' ? 'graph-generation:execute' : 'editor-inline:execute'
  it('读取旧任务不恢复，显式继续必须先重认证并使用新句柄', async () => {
    const old = await recovery(kind, handle.epoch), current = await recovery(kind, session.leaseId)
    const invoke = vi.spyOn(ipc, 'invokeWithProjectSession')
      .mockResolvedValueOnce(old).mockResolvedValueOnce(current.view).mockResolvedValueOnce(current)
      .mockResolvedValueOnce({ run: current.view } as unknown as MainGenerationExecuteReceipt).mockResolvedValueOnce(current)
    const client = create()
    await client.open({ handle })
    expect(invoke).toHaveBeenCalledTimes(1)
    await expect(client.execute()).resolves.toBeDefined()
    expect(invoke).toHaveBeenNthCalledWith(2, session, 'generation:resume', handle)
    expect(invoke).toHaveBeenNthCalledWith(4, session, executeChannel, { handle: current.view.handle })
  })
  it.each(['wrong-epoch', 'wrong-run', 'denied'])('恢复返回错误身份或拒绝时绝不执行：%s', async failure => {
    const invoke = vi.spyOn(ipc, 'invokeWithProjectSession').mockResolvedValueOnce(await recovery(kind, handle.epoch))
    if (failure === 'denied') invoke.mockRejectedValueOnce(new Error('GENERATION_RECOVERY_UNAUTHORIZED'))
    else invoke.mockResolvedValueOnce({ ...view(failure === 'wrong-epoch' ? 'unexpected-lease' : session.leaseId), handle: { ...handle, epoch: failure === 'wrong-epoch' ? 'unexpected-lease' : session.leaseId, runId: failure === 'wrong-run' ? 'foreign-run' : handle.runId } })
    const client = create(); await client.open({ handle })
    await expect(client.execute()).rejects.toThrow()
    expect(invoke.mock.calls.map(call => call[1])).not.toContain(executeChannel)
  })
})
