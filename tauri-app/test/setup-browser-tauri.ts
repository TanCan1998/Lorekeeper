import { beforeEach } from 'vitest'
import { createTauriChannelFixture } from './helpers/channel-fixture'

/** 浏览器保留真实DOM；每个用例的合成频道API动态转接到Tauri内部接缝。 */
beforeEach(() => {
  const fixture = createTauriChannelFixture(() => {
    const win = window as unknown as { aiNovelAPI?: { invoke: unknown; on?: unknown }; velaAPI?: { invoke: unknown; on?: unknown } }
    return win.aiNovelAPI ?? win.velaAPI
  })
  for (const [key, value] of Object.entries(fixture)) {
    Object.defineProperty(window, key, { configurable: true, writable: true, value })
  }
})
