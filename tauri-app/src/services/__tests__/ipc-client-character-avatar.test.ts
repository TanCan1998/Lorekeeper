import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { ipc } from '../ipc-client'
import { MIGRATED_CHANNELS } from '../../shared/migrated-channels'
import type { ProjectSessionContext } from '../../shared/ipc-channels'
import { setActiveProjectSessionContext } from '../../shared/project-session-context'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

const session: ProjectSessionContext = {
  projectId: 'project-A', leaseId: 'lease-A', projectPath: 'C:/projects/A',
}
const avatarView = {
  characterId: 'char-1', assetRevision: 1, mime: 'image/png', base64: 'cG5n',
}

let tauri: TauriInternalsHandle

beforeEach(() => {
  tauri = installTauriInternals({
    commands: {
      character_avatar_choose: { success: true, cancelled: false, image: { characterId: 'char-1', mime: 'image/png', base64: 'cG5n' } },
      character_avatar_commit: { success: true, avatar: avatarView },
      character_avatar_read_batch: { success: true, avatars: [avatarView] },
      character_avatar_remove: { success: true },
    },
  })
  setActiveProjectSessionContext(session)
})

afterEach(() => {
  setActiveProjectSessionContext(null)
  tauri.uninstall()
})

// 项目域频道：`character-avatar:*` 由 ipc-client 自动注入 `projectSession` 尾参，
// 参数名以契约位置顺序机械映射（`characterId` / `base64` / `characterIds`）。
describe('character-avatar IPC transport', () => {
  it('四个频道都已登记为已迁移频道', () => {
    for (const channel of [
      'character-avatar:choose',
      'character-avatar:commit',
      'character-avatar:read-batch',
      'character-avatar:remove',
    ]) {
      expect(MIGRATED_CHANNELS.has(channel)).toBe(true)
    }
  })

  it('choose 注入项目会话并携带 characterId', async () => {
    const res = await ipc.invoke('character-avatar:choose', 'char-1')

    expect(tauri.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'character_avatar_choose',
      { characterId: 'char-1', projectSession: session },
    ])
    expect(res).toEqual({ success: true, cancelled: false, image: { characterId: 'char-1', mime: 'image/png', base64: 'cG5n' } })
  })

  it('commit 下发 characterId 与 base64 两个位置参数', async () => {
    const res = await ipc.invoke('character-avatar:commit', 'char-1', 'cG5n')

    expect(tauri.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'character_avatar_commit',
      { characterId: 'char-1', base64: 'cG5n', projectSession: session },
    ])
    expect(res).toEqual({ success: true, avatar: avatarView })
  })

  it('read-batch 下发 characterIds 数组', async () => {
    const res = await ipc.invoke('character-avatar:read-batch', ['char-1', 'char-2'])

    expect(tauri.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'character_avatar_read_batch',
      { characterIds: ['char-1', 'char-2'], projectSession: session },
    ])
    expect(res).toEqual({ success: true, avatars: [avatarView] })
  })

  it('remove 只下发 characterId', async () => {
    const res = await ipc.invoke('character-avatar:remove', 'char-1')

    expect(tauri.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'character_avatar_remove',
      { characterId: 'char-1', projectSession: session },
    ])
    expect(res).toEqual({ success: true })
  })
})
