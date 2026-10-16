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
describe('未开放特殊域的 Tauri 传输边界', () => {
  it('遗留导入在传输前拒绝，不因其它特殊域迁移而误开放', async () => {
    expect(MIGRATED_CHANNELS.has('project:import-legacy-copy')).toBe(false)
    await expect(ipc.invoke('project:import-legacy-copy', 'source-grant', 'target-grant'))
      .rejects.toThrow(/频道 project:import-legacy-copy 尚未迁移/u)
    expect(tauri.invoke).not.toHaveBeenCalled()
  })
  it('缺少项目会话时先拒绝，而不是借用外部授权或触达主进程', async () => {
    await expect(ipc.invoke('graph-generation:begin',
      { input: { kind: 'plot' }, modelId: 'synthetic-model', uiActionNonce: 'synthetic-action' }))
      .rejects.toThrow('缺少当前项目会话，已拒绝项目数据访问')
    expect(tauri.invoke).not.toHaveBeenCalled()
  })
})
