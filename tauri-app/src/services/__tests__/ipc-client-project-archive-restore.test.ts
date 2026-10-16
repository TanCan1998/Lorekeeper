import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { ipc } from '../ipc-client'
import { MIGRATED_CHANNELS } from '../../shared/migrated-channels'
import { setActiveProjectSessionContext } from '../../shared/project-session-context'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

const receipt = {
  originProjectId: 'origin-project',
  targetProjectId: 'new-project',
  targetProjectRoot: 'C:/restored',
  snapshotGeneration: 'gen-1',
  portableDatabaseSha256: 'a'.repeat(64),
  requiresRuntimeFreezeGuard: true,
}
let tauri: TauriInternalsHandle

beforeEach(() => {
  tauri = installTauriInternals({
    commands: {
      project_archive_restore: { success: true, receipt, recentProjectUpdated: true },
    },
  })
  setActiveProjectSessionContext(null)
})
afterEach(() => {
  setActiveProjectSessionContext(null)
  tauri.uninstall()
})

describe('project:archive-restore IPC transport', () => {
  it('已登记迁移频道，未打开项目也可凭两个授权恢复', async () => {
    expect(MIGRATED_CHANNELS.has('project:archive-restore')).toBe(true)
    const request = { archiveGrantId: 'archive-grant', targetGrantId: 'target-grant' }
    expect(await ipc.invoke('project:archive-restore', request)).toEqual({
      success: true, receipt, recentProjectUpdated: true,
    })
    expect(tauri.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'project_archive_restore', { request },
    ])
  })

  it('已有活动项目时也不把其会话或路径注入恢复请求', async () => {
    setActiveProjectSessionContext({ projectId: 'current', leaseId: 'lease', projectPath: 'C:/current' })
    const request = { archiveGrantId: 'read-once', targetGrantId: 'create-once' }
    await ipc.invoke('project:archive-restore', request)
    expect(tauri.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'project_archive_restore', { request },
    ])
  })
})
