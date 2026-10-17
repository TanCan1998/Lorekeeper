import { afterEach, describe, expect, it, vi } from 'vitest'
import type { GraphGenerationInput, GraphGenerationRecovery } from '../../shared/graph-generation'
import { createGraphGeneration } from '../graph-generation'
import { ipc } from '../ipc-client'

const session = { projectId: 'graph-project', leaseId: 'graph-lease', projectPath: 'C:/synthetic-graph' }
const handle = { projectId: session.projectId, epoch: session.leaseId, rootActionId: 'graph-root', runId: 'graph-run' }
// 本组只测试客户端输入身份比较，不把传输夹具当作后端事实证明。
function response(input: GraphGenerationInput): GraphGenerationRecovery {
  return { view: { handle }, context: { projectId: session.projectId, kind: input.kind, input } } as unknown as GraphGenerationRecovery
}
afterEach(() => vi.restoreAllMocks())

describe('图生成输入身份比较', () => {
  it.each([
    { input: { kind: 'plan', chapterNumber: 1 }, returned: { chapterNumber: 1, kind: 'plan' } },
    { input: { kind: 'event', planId: 2, draftId: 3 }, returned: { draftId: 3, kind: 'event', planId: 2 } },
  ] satisfies { input: GraphGenerationInput; returned: GraphGenerationInput }[])('接受字段值一致但键序不同的$input.kind输入', async ({ input, returned }) => {
    const recovery = response(returned)
    const invoke = vi.spyOn(ipc, 'invokeWithProjectSession').mockResolvedValue(recovery)
    await expect(createGraphGeneration(session).open({ input, modelId: 'synthetic', uiActionNonce: 'ordered-input' })).resolves.toBe(recovery)
    expect(invoke).toHaveBeenCalledExactlyOnceWith(session, 'graph-generation:begin', { input, modelId: 'synthetic', uiActionNonce: 'ordered-input' })
  })

  it.each([
    { kind: 'plan', chapterNumber: 2 },
    { kind: 'plan', chapterNumber: '1' },
    { kind: 'plan', chapterNumber: 1, unexpected: true },
    { kind: 'plan' },
  ])('拒绝字段值、类型或键集合变化：%j', async changed => {
    vi.spyOn(ipc, 'invokeWithProjectSession').mockResolvedValue(response(changed as unknown as GraphGenerationInput))
    await expect(createGraphGeneration(session).open({ input: { kind: 'plan', chapterNumber: 1 }, modelId: 'synthetic', uiActionNonce: 'changed-input' })).rejects.toThrow('GRAPH_GENERATION_CONTEXT_MISMATCH')
  })
})
