import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { MIGRATED_CHANNELS } from '../../shared/migrated-channels'
import { APPEARANCE_STORAGE_KEY } from '../../shared/appearance-profile'
import type { AppearanceReadbackAck } from '../../shared/startup-contract'
import { ipc } from '../ipc-client'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

// B7 启动态子集：起点是「渲染层启动页能拿到主进程就绪态」。
// 这里只断言**传输合同**（频道已迁移、命令名、命名参数、无项目会话注入），
// 不代表真实 SQLite / 皮肤资产 / GUI 端到端验收。

let tauriInternals: TauriInternalsHandle

beforeEach(() => {
  tauriInternals = installTauriInternals({
    commands: {
      startup_get_state: { state: 'ready', globalGeneration: 'gen-1', skinRevision: 0 },
      startup_skin_snapshot: { globalGeneration: 'gen-1', skinRevision: 0, backgroundSkin: 'classic' },
      startup_appearance_ack: true,
    },
  })
})

afterEach(() => {
  tauriInternals.uninstall()
})

describe('启动门禁（startup:*）传输合同', () => {
  it('三个频道都已登记为已迁移', () => {
    for (const channel of ['startup:get-state', 'startup:skin-snapshot', 'startup:appearance-ack']) {
      expect(MIGRATED_CHANNELS.has(channel)).toBe(true)
    }
  })

  it('无参 get-state 不带命名参数，也不注入项目会话', async () => {
    await expect(ipc.invoke('startup:get-state')).resolves.toEqual({
      state: 'ready', globalGeneration: 'gen-1', skinRevision: 0,
    })
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual(['startup_get_state', {}])
    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
  })

  it('skin-snapshot 与 appearance-ack 使用契约命名参数 ready / ack', async () => {
    const ready = { state: 'ready' as const, globalGeneration: 'gen-1', skinRevision: 0 }
    await expect(ipc.invoke('startup:skin-snapshot', ready)).resolves.toEqual({
      globalGeneration: 'gen-1', skinRevision: 0, backgroundSkin: 'classic',
    })
    // 显式标注契约类型：对象字面量属性会把字面量 const 拓宽为 string，
    // 标注后这里校验的仍是真实契约形状（storageKey 字面量）。
    const ack: AppearanceReadbackAck = { storageKey: APPEARANCE_STORAGE_KEY, profileRevision: 1,
      globalGeneration: 'gen-1', skinRevision: 0 }
    await expect(ipc.invoke('startup:appearance-ack', ack)).resolves.toBe(true)
    expect(tauriInternals.invoke.mock.calls.map(call => call.slice(0, 2))).toEqual([
      ['startup_skin_snapshot', { ready }],
      ['startup_appearance_ack', { ack }],
    ])
  })

  it('启动门禁已迁移不改变其它未迁移频道的前置拦截', async () => {
    await expect(ipc.invokeWithProjectSession(
      { projectId: 'project-A', leaseId: 'lease-A', projectPath: 'C:/projects/A' },
      'project:overview-current',
    )).rejects.toThrow('尚未迁移')
    expect(tauriInternals.invoke).not.toHaveBeenCalled()
  })
})
