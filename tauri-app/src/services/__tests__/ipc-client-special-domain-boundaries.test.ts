import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { ipc } from '../ipc-client'
import { MIGRATED_CHANNELS } from '../../shared/migrated-channels'
import type { ProjectSessionContext } from '../../shared/ipc-channels'
import { getActiveProjectSessionContext, setActiveProjectSessionContext } from '../../shared/project-session-context'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

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
// finalized-character 全五频道 / character-avatar 四频道）不再属于本回归；
// 改用仍未迁移的 cloud-backup 域（需先确认凭据安全存储与网络策略）。
const cases = [
  { channel: 'cloud-backup:connect',
    invoke: () => ipc.invoke('cloud-backup:connect',
      { endpoint: 'https://example.invalid', username: 'synthetic', secret: 'synthetic' }) },
  { channel: 'cloud-backup:list',
    invoke: () => ipc.invoke('cloud-backup:list',
      { localEndpointAccountId: 'synthetic', cloudBookId: 'synthetic' }) },
  { channel: 'cloud-backup:restore-copy',
    invoke: () => ipc.invoke('cloud-backup:restore-copy',
      { localEndpointAccountId: 'synthetic', cloudBookId: 'synthetic', operationId: 'synthetic',
        generationId: 'synthetic', targetGrantId: 'synthetic' }) },
] as const

describe('未开放特殊域的 Tauri 传输边界', () => {
  it.each(cases)('$channel 在传输前被拒绝', async ({ channel, invoke }) => {
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
