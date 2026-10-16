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
  it('首版旧库转换明确blocked，不因其它特殊域迁移而误开放', async () => {
    expect(MIGRATED_CHANNELS.has('project:import-legacy-copy')).toBe(false)
    await expect(ipc.invoke('project:import-legacy-copy', 'source-grant', 'target-grant'))
      .resolves.toEqual({ state: 'blocked', code: 'LEGACY_IMPORT_UNSUPPORTED' })
    expect(tauri.invoke).not.toHaveBeenCalled()
  })
  it.each([['', ''], ['missing', 'missing'], ['../synthetic', 'C:/synthetic-target']])('无效/重复授权标识仍明确blocked且不读取会话或调用主进程：%s', async (source, target) => {
    expect(getActiveProjectSessionContext()).toBeNull()
    for (let i = 0; i < 2; i++) {
      await expect(ipc.invoke('project:import-legacy-copy', source, target)).resolves.toEqual({ state: 'blocked', code: 'LEGACY_IMPORT_UNSUPPORTED' })
    }
    expect(tauri.invoke).not.toHaveBeenCalled()
  })

  it('缺少项目会话时先拒绝，而不是借用外部授权或触达主进程', async () => {
    await expect(ipc.invoke('graph-generation:begin',
      { input: { kind: 'plot' }, modelId: 'synthetic-model', uiActionNonce: 'synthetic-action' }))
      .rejects.toThrow('缺少当前项目会话，已拒绝项目数据访问')
    expect(tauri.invoke).not.toHaveBeenCalled()
  })
})
