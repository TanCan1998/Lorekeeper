import source from '../../src/services/ipc-client.ts?raw'
import { vi } from 'vitest'
import { MIGRATED_CHANNELS } from '../../src/shared/migrated-channels'

type SyntheticApi = { invoke: unknown; on?: unknown; [key: string]: unknown }
type SyntheticWindow = { aiNovelAPI?: SyntheticApi; velaAPI?: SyntheticApi }

/** 只在测试中桥接上游合成频道桩，仍经过生产Tauri参数映射和事件包装。 */
export function createTauriChannelFixture(getApi: () => SyntheticApi | undefined) {
  const parameterNames = new Map<string, string[]>()
  for (const match of source.matchAll(/'([^']+)': \[([^\]]*)\]/g)) {
    parameterNames.set(match[1], [...match[2].matchAll(/'([^']+)'/g)].map(item => item[1]))
  }
  const channels = new Map([...MIGRATED_CHANNELS].map(channel => [channel.replace(/[:-]/g, '_'), channel]))
  const callbacks = new Map<number, (...args: unknown[]) => unknown>()
  const unlisteners = new Map<number, () => void>()
  let sequence = 0
  return {
    __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
    __TAURI_INTERNALS__: {
      transformCallback(callback: (...args: unknown[]) => unknown) {
        callbacks.set(++sequence, callback)
        return sequence
      },
      invoke(command: string, args: Record<string, unknown> = {}) {
        const api = getApi()
        if (command === 'plugin:event|listen') {
          const id = ++sequence
          const listener = (payload: unknown) => callbacks.get(Number(args.handler))?.({
            event: args.event, id, payload,
          })
          const unsubscribe = (api?.on as ((...args: unknown[]) => unknown) | undefined)?.(args.event, listener)
          if (typeof unsubscribe === 'function') unlisteners.set(id, unsubscribe as () => void)
          return Promise.resolve(id)
        }
        if (command === 'plugin:event|unlisten') {
          unlisteners.get(Number(args.eventId))?.()
          unlisteners.delete(Number(args.eventId))
          return Promise.resolve()
        }
        if (!api || typeof api.invoke !== 'function') throw new Error('合成频道夹具缺少invoke')
        const channel = channels.get(command)
        if (!channel) throw new Error('未知的Tauri命令：' + command)
        const positional = (parameterNames.get(channel) ?? []).map(name => args[name])
        if ('projectSession' in args && !parameterNames.get(channel)?.includes('projectSession')) positional.push(args.projectSession)
        return Promise.resolve((api.invoke as (...args: unknown[]) => unknown)(channel, ...positional))
      },
    },
  }
}

/** Node测试无真实window，注入内部接缝；产品代码没有Electron回退。 */
export function installTauriChannelFixture(fixture: SyntheticWindow) {
  const api = fixture.aiNovelAPI ?? fixture.velaAPI
  if (!api || typeof api.invoke !== 'function') throw new Error('合成频道夹具缺少invoke')
  const internals = createTauriChannelFixture(() => api)
  if (typeof document !== 'undefined' && typeof window.addEventListener === 'function') {
    for (const [key, value] of Object.entries({ ...fixture, ...internals })) {
      Object.defineProperty(window, key, { configurable: true, writable: true, value })
    }
  } else vi.stubGlobal('window', internals)
}
