import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { ipc } from '../ipc-client'
import { MIGRATED_CHANNELS } from '../../shared/migrated-channels'
import type { ProjectSessionContext } from '../../shared/ipc-channels'
import { getActiveProjectSessionContext, setActiveProjectSessionContext } from '../../shared/project-session-context'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

const session: ProjectSessionContext = {
  projectId: 'synthetic-project', leaseId: 'synthetic-lease', projectPath: 'C:/synthetic-project',
}
let tauri: TauriInternalsHandle
let priorSession: ProjectSessionContext | null
beforeEach(() => {
  priorSession = getActiveProjectSessionContext()
  setActiveProjectSessionContext(null)
  tauri = installTauriInternals()
})
afterEach(() => {
  tauri.uninstall()
  setActiveProjectSessionContext(priorSession)
})

// 使用真实适配器而非 ipc mock，证明前端不会把未迁移特殊域送到主进程。
// 已成为正式迁移面的频道（graph-generation / finalization-generation /
// finalized-character 读面）不再属于本回归；改用仍未迁移的域：
// finalized-character:commit（源证明准入）与 character-avatar 头像域。
const cases = [
  { channel: 'finalized-character:commit',
    invoke: () => ipc.invokeWithProjectSession(session, 'finalized-character:commit', {
      contextId: 'synthetic-context',
      handle: {
        projectId: 'synthetic-project',
        epoch: 'synthetic-lease',
        rootActionId: 'synthetic-root',
        runId: 'synthetic-run',
      },
      artifact: { artifactId: 'synthetic-artifact', revision: 1, textHash: 'a'.repeat(64) },
    }) },
  { channel: 'character-avatar:choose',
    invoke: () => ipc.invokeWithProjectSession(session, 'character-avatar:choose', 'synthetic-character') },
  { channel: 'character-avatar:read-batch',
    invoke: () => ipc.invokeWithProjectSession(session, 'character-avatar:read-batch', ['synthetic-character']) },
] as const

describe('未开放特殊域的 Tauri 传输边界', () => {
  it.each(cases)('$channel 即使提供冻结会话也在传输前拒绝', async ({ channel, invoke }) => {
    expect(MIGRATED_CHANNELS.has(channel)).toBe(false)
    await expect(invoke()).rejects.toThrow(new RegExp(`频道 ${channel} 尚未迁移`))
    expect(tauri.invoke).not.toHaveBeenCalled()
  })

  it('缺少项目会话时先拒绝，而不是借用外部授权或触达主进程', async () => {
    await expect(ipc.invoke('graph-generation:begin',
      { input: { kind: 'plot' }, modelId: 'synthetic-model', uiActionNonce: 'synthetic-action' }))
      .rejects.toThrow('缺少当前项目会话，已拒绝项目数据访问')
    expect(tauri.invoke).not.toHaveBeenCalled()
  })
})
