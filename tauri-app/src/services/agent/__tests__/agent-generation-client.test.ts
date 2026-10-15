import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { AgentGenerationClient } from '../agent-generation-client'
import { AgentHostFixture } from './agent-generation.fixture'

const session = { projectId: 'synthetic-project', leaseId: 'synthetic-lease', projectPath: 'C:/synthetic/client' }
let fixture: AgentHostFixture
beforeEach(() => {
  fixture = new AgentHostFixture(session).install()
  fixture.initialize({ mode: 'fast', uiLocale: 'zh-CN', historyMessages: [], userMessage: '合成客户端输入', tools: [] })
})
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks() })

describe('助手恢复客户端身份约束', () => {
  it.each(['read', 'resume'] as const)('首次 %s 也必须匹配请求的父根，而非只匹配项目', async method => {
    const accepted = vi.fn()
    const client = new AgentGenerationClient(session, accepted)
    const requested = { ...fixture.recovery.handle, rootActionId: 'different-requested-root' }
    await expect(client[method](requested)).rejects.toThrow('GENERATION_AGENT_IDENTITY_MISMATCH')
    expect(accepted).not.toHaveBeenCalled()
    expect(() => client.recovery).toThrow('GENERATION_AGENT_HANDLE_REQUIRED')
  })

  it('允许主进程将历史句柄解析为同父根的当前轮次，不要求旧纪元和旧轮次逐字一致', async () => {
    const client = new AgentGenerationClient(session)
    const requested = { ...fixture.recovery.handle, epoch: 'historical-epoch', runId: 'historical-round' }
    await expect(client.read(requested)).resolves.toEqual(fixture.recovery)
    expect(client.recovery.handle).toEqual(fixture.recovery.handle)
    expect(fixture.invoke.mock.calls.at(-1)?.[1]).toEqual({ request: { handle: requested }, projectSession: session })
  })

  it('后续轮次不接受另一父根，失败时保留已接受的恢复上下文', async () => {
    const accepted = vi.fn()
    const client = new AgentGenerationClient(session, accepted)
    const original = await client.read(fixture.recovery.handle)
    fixture.recovery.handle = { ...fixture.recovery.handle, rootActionId: 'different-response-root' }
    await expect(client.round(0)).rejects.toThrow('GENERATION_AGENT_IDENTITY_MISMATCH')
    expect(client.recovery).toBe(original)
    expect(accepted).toHaveBeenCalledTimes(1)
  })
  describe.each(['claim', 'finish'] as const)('%s 工具回执', method => {
    it.each(['projectId', 'epoch', 'rootActionId', 'runId', 'attemptId', 'toolCallId'] as const)('拒绝错配 %s 的动作，不把其他动作的回执交给执行层', async field => {
      const client = new AgentGenerationClient(session)
      const ref = { handle: { ...fixture.recovery.handle }, attemptId: 'synthetic-attempt', toolCallId: 'synthetic-tool' }
      fixture.recovery.rounds[0].actions = [{ ref, name: 'synthetic-probe', arguments: {}, status: 'running' }]
      await client.read(fixture.recovery.handle)
      const original = fixture.invoke.getMockImplementation()!
      fixture.invoke.mockImplementation(async (command, args) => {
        const result = await original(command, args)
        if (command !== (method === 'claim' ? 'agent_generation_claim_tool' : 'agent_generation_finish_tool')) return result
        const mismatched = structuredClone(result) as { ref?: typeof ref; action?: { ref: typeof ref } }
        const target = method === 'claim' ? mismatched.action!.ref : mismatched.ref!
        if (field === 'attemptId' || field === 'toolCallId') target[field] = 'different-action'
        else target.handle[field] = 'different-handle'
        return mismatched
      })
      const request = method === 'claim' ? client.claimTool(ref, true)
        : client.finishTool({ ref, status: 'completed', observation: '合成完成回执' })
      await expect(request).rejects.toThrow('GENERATION_AGENT_TOOL_IDENTITY_MISMATCH')
    })
  })

  it('同一父根的历史动作句柄仍可原样认领和完成', async () => {
    const client = new AgentGenerationClient(session)
    const ref = { handle: { ...fixture.recovery.handle, epoch: 'historical-epoch', runId: 'historical-round' },
      attemptId: 'synthetic-attempt', toolCallId: 'synthetic-tool' }
    fixture.recovery.rounds[0].actions = [{ ref, name: 'synthetic-probe', arguments: {}, status: 'pending' }]
    await client.read(fixture.recovery.handle)
    await expect(client.claimTool(ref, true)).resolves.toMatchObject({ execute: true, action: { ref, status: 'running' } })
    await expect(client.finishTool({ ref, status: 'completed', observation: '合成完成回执' })).resolves.toMatchObject({ ref, status: 'completed' })
  })

  it.each(['read', 'resume'] as const)('%s 在途时调用方改写句柄也不能认领另一父根的回执', async method => {
    const accepted = vi.fn()
    const client = new AgentGenerationClient(session, accepted)
    const requested = { ...fixture.recovery.handle, rootActionId: 'requested-original-root' }
    const original = fixture.invoke.getMockImplementation()!
    let release!: () => void
    const pending = new Promise<void>(resolve => { release = resolve })
    fixture.invoke.mockImplementation(async (command, args) => {
      const result = await original(command, args)
      await pending
      return result
    })
    const request = client[method](requested)
    requested.rootActionId = fixture.recovery.handle.rootActionId
    release()
    await expect(request).rejects.toThrow('GENERATION_AGENT_IDENTITY_MISMATCH')
    expect(accepted).not.toHaveBeenCalled()
    expect(() => client.recovery).toThrow('GENERATION_AGENT_HANDLE_REQUIRED')
  })

  describe.each(['claim', 'finish'] as const)('%s 在途身份冻结', method => {
    it.each(['projectId', 'epoch', 'rootActionId', 'runId', 'attemptId', 'toolCallId'] as const)('调用方突变 %s 不得让错配回执通过', async field => {
      const client = new AgentGenerationClient(session)
      const ref = { handle: { ...fixture.recovery.handle }, attemptId: 'synthetic-attempt', toolCallId: 'synthetic-tool' }
      fixture.recovery.rounds[0].actions = [{ ref: structuredClone(ref), name: 'synthetic-probe', arguments: {}, status: 'running' }]
      await client.read(fixture.recovery.handle)
      const original = fixture.invoke.getMockImplementation()!
      let release!: () => void
      const pending = new Promise<void>(resolve => { release = resolve })
      fixture.invoke.mockImplementation(async (command, args) => {
        const result = structuredClone(await original(command, args)) as { ref?: typeof ref; action?: { ref: typeof ref } }
        const target = method === 'claim' ? result.action!.ref : result.ref!
        if (field === 'attemptId' || field === 'toolCallId') target[field] = 'different-action'
        else target.handle[field] = 'different-handle'
        await pending
        return result
      })
      const request = method === 'claim' ? client.claimTool(ref, true)
        : client.finishTool({ ref, status: 'completed', observation: '合成完成回执' })
      if (field === 'attemptId' || field === 'toolCallId') ref[field] = 'different-action'
      else ref.handle[field] = 'different-handle'
      release()
      await expect(request).rejects.toThrow('GENERATION_AGENT_TOOL_IDENTITY_MISMATCH')
    })
  })

  it.each(['projectId', 'leaseId', 'projectPath'] as const)('调用方改写构造参数的 %s 不得替换已冻结的项目会话', async field => {
    const mutableSession = { ...session }
    const client = new AgentGenerationClient(mutableSession)
    mutableSession[field] = 'different-session-value'
    await expect(client.read(fixture.recovery.handle)).resolves.toEqual(fixture.recovery)
    expect(fixture.invoke.mock.calls.at(-1)?.[1].projectSession).toEqual(session)
  })

})
