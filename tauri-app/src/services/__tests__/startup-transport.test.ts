import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { MIGRATED_CHANNELS } from '../../shared/migrated-channels'
import { APPEARANCE_STORAGE_KEY } from '../../shared/appearance-profile'
import type { AppearanceReadbackAck } from '../../shared/startup-contract'
import { ipc } from '../ipc-client'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

let internals: TauriInternalsHandle
beforeEach(() => {
  internals = installTauriInternals({ commands: {
    startup_get_state: { state: 'ready', globalGeneration: 'gen-1' },
    startup_appearance_ack: true,
  } })
})
afterEach(() => internals.uninstall())

describe('纸墨启动门禁传输合同', () => {
  it('只开放两个启动频道，旧皮肤四频道全部移除', () => {
    expect(MIGRATED_CHANNELS.has('startup:get-state')).toBe(true)
    expect(MIGRATED_CHANNELS.has('startup:appearance-ack')).toBe(true)
    for (const old of ['startup:skin-snapshot', 'skin:get-state', 'skin:execute', 'skin:read-custom-asset']) {
      expect(MIGRATED_CHANNELS.has(old)).toBe(false)
    }
  })
  it('状态读取与v3确认不注入项目会话或皮肤字段', async () => {
    await expect(ipc.invoke('startup:get-state')).resolves.toEqual({
      state: 'ready', globalGeneration: 'gen-1',
    })
    const ack: AppearanceReadbackAck = {
      storageKey: APPEARANCE_STORAGE_KEY, profileRevision: 1, globalGeneration: 'gen-1',
    }
    expect(ack.storageKey).toBe('ai-novel-writer-appearance-v3')
    await expect(ipc.invoke('startup:appearance-ack', ack)).resolves.toBe(true)
    expect(internals.invoke.mock.calls.map(call => call.slice(0, 2))).toEqual([
      ['startup_get_state', {}], ['startup_appearance_ack', { ack }],
    ])
  })
  it('新外壳仍不授权旧库转换', async () => {
    await expect(ipc.invoke('project:import-legacy-copy', 'source', 'target')).resolves
      .toEqual({ state: 'blocked', code: 'LEGACY_IMPORT_UNSUPPORTED' })
    expect(internals.invoke).not.toHaveBeenCalled()
  })
})
