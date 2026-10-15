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
})
