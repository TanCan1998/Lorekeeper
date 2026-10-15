import { ipc } from '../ipc-client'
import type { ProjectSessionContext } from '../../shared/ipc-channels'
import type { MainGenerationRunHandle } from '../generation/generation-runtime'
import type { AgentGenerationChannels, AgentGenerationRecovery, AgentGenerationRound, AgentToolActionRef } from '../../shared/agent-generation'
import type { AgentTool } from './tool-registry'

type ClaimRequest = AgentGenerationChannels['agent-generation:claim-tool']['args'][0]
type FinishRequest = AgentGenerationChannels['agent-generation:finish-tool']['args'][0]

// 工具回执不能借用同父根的另一轮次、尝试或动作身份。
function assertSameActionRef(expected: AgentToolActionRef, actual: AgentToolActionRef): void {
  if (expected.attemptId !== actual.attemptId || expected.toolCallId !== actual.toolCallId
    || (['projectId', 'epoch', 'rootActionId', 'runId'] as const).some(key => expected.handle[key] !== actual.handle[key])) {
    throw new Error('GENERATION_AGENT_TOOL_IDENTITY_MISMATCH')
  }
}

/** No provider or renderer-owned generation budget exists in this adapter. */
export class AgentGenerationClient {
  private current?: AgentGenerationRecovery
  private closed = false
  private cancelRequested = false
  constructor(private readonly projectSession: ProjectSessionContext,
    private readonly onRecovery: (recovery: AgentGenerationRecovery) => void = () => {}) {}

  get recovery(): AgentGenerationRecovery {
    if (!this.current) throw new Error('GENERATION_AGENT_HANDLE_REQUIRED')
    return this.current
  }

  private accept(recovery: AgentGenerationRecovery, expectedRootActionId?: string): AgentGenerationRecovery {
    if (expectedRootActionId && recovery.handle.rootActionId !== expectedRootActionId
      || recovery.handle.projectId !== this.projectSession.projectId
      || this.current && recovery.handle.rootActionId !== this.current.handle.rootActionId) throw new Error('GENERATION_AGENT_IDENTITY_MISMATCH')
    this.current = recovery
    if (!this.closed) this.onRecovery(recovery)
    return recovery
  }

  private assertActive() {
    if (this.closed || this.cancelRequested) throw new Error('GENERATION_AGENT_DETACHED')
    if (this.recovery.sourceStatus !== 'current') throw new Error('GENERATION_SOURCE_CHANGED')
  }

  async begin(request: AgentGenerationChannels['agent-generation:begin']['args'][0]) {
    const recovery = this.accept(await ipc.invokeWithProjectSession(this.projectSession, 'agent-generation:begin', request))
    if (this.cancelRequested) { await this.cancel(); throw new Error('GENERATION_AGENT_CANCELLED') }
    return recovery
  }

  async read(handle: MainGenerationRunHandle = this.recovery.handle) {
    return this.accept(await ipc.invokeWithProjectSession(this.projectSession, 'agent-generation:read', { handle }), handle.rootActionId)
  }

  async resume(handle: MainGenerationRunHandle) {
    const recovery = this.accept(await ipc.invokeWithProjectSession(this.projectSession, 'agent-generation:resume', { handle }), handle.rootActionId)
    if (this.cancelRequested) { await this.cancel(); throw new Error('GENERATION_AGENT_CANCELLED') }
    return recovery
  }

  async round(index: number): Promise<AgentGenerationRound> {
    this.assertActive()
    const existing = this.recovery.rounds.find(round => round.index === index)
    if (existing && existing.status !== 'not-started') return existing
    this.accept(await ipc.invokeWithProjectSession(this.projectSession, 'agent-generation:round', { handle: this.recovery.handle, index }))
    this.assertActive()
    const round = this.recovery.rounds.find(round => round.index === index)
    if (!round) throw new Error('GENERATION_AGENT_ROUND_MISSING')
    return round
  }

  async readRound(index: number) {
    await this.read()
    const round = this.recovery.rounds.find(round => round.index === index)
    if (!round) throw new Error('GENERATION_AGENT_ROUND_MISSING')
    return round
  }

  assertTool(tool: AgentTool) {
    const frozen = this.recovery.context.input.tools.find(entry => entry.name === tool.name)
    if (!frozen || frozen.source !== tool.source || frozen.requiresConfirmation !== tool.requiresConfirmation
      || frozen.isReadOnly !== tool.isReadOnly || JSON.stringify(frozen.inputSchema) !== JSON.stringify(tool.inputSchema)) {
      throw new Error('GENERATION_AGENT_TOOL_CHANGED')
    }
  }

  async claimTool(ref: AgentToolActionRef, confirmed: boolean, authorBlueprintProposals?: Readonly<NonNullable<ClaimRequest['authorBlueprintProposals']>>) {
    this.assertActive()
    const result = await ipc.invokeWithProjectSession(this.projectSession, 'agent-generation:claim-tool', {
      ref, confirmed, ...(authorBlueprintProposals?.length ? { authorBlueprintProposals: [...authorBlueprintProposals] } : {}),
    })
    assertSameActionRef(ref, result.action.ref)
    return result
  }

  async finishTool(request: FinishRequest) {
    const result = await ipc.invokeWithProjectSession(this.projectSession, 'agent-generation:finish-tool', request)
    assertSameActionRef(request.ref, result.ref)
    return result
  }

  async cancel() {
    this.cancelRequested = true
    if (this.current) await ipc.invokeWithProjectSession(this.projectSession, 'generation:cancel', this.current.handle)
  }

  close() { this.closed = true }
}
