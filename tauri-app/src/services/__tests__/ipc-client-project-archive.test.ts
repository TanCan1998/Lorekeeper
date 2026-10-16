import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { ipc } from '../ipc-client'
import { MIGRATED_CHANNELS } from '../../shared/migrated-channels'
import type { ProjectSessionContext } from '../../shared/ipc-channels'
import { setActiveProjectSessionContext } from '../../shared/project-session-context'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

const session: ProjectSessionContext = {
  projectId: 'project-A', leaseId: 'lease-A', projectPath: 'C:/projects/A',
}
const archiveGrant = { grantId: 'grant-1', displayName: '第一部小说.ainovel' }
const restoreGrant = { grantId: 'grant-2', displayName: '第一部小说-恢复副本' }
const legacyGrant = { grantId: 'grant-3', displayName: '旧项目' }

let tauri: TauriInternalsHandle

beforeEach(() => {
  tauri = installTauriInternals({
    commands: {
      dialog_select_project_archive_export: archiveGrant,
      dialog_select_project_archive: archiveGrant,
      dialog_select_project_restore_target: restoreGrant,
      dialog_select_legacy_project: legacyGrant,
    },
  })
  setActiveProjectSessionContext(session)
})

afterEach(() => {
  setActiveProjectSessionContext(null)
  tauri.uninstall()
})

// 能力域频道：即便存在活动项目会话，也不得把 `projectSession` 注入 dialog 选择器。
describe('dialog 归档选择的 Tauri 适配面（B109）', () => {
  it('四个 dialog 频道都已登记为已迁移频道', () => {
    for (const channel of [
      'dialog:select-project-archive-export',
      'dialog:select-project-archive',
      'dialog:select-project-restore-target',
      'dialog:select-legacy-project',
    ]) {
      expect(MIGRATED_CHANNELS.has(channel)).toBe(true)
    }
  })

  it('导出选择器只下发命名参数 suggestedName', async () => {
    const res = await ipc.invoke('dialog:select-project-archive-export', '第一部小说')

    expect(tauri.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'dialog_select_project_archive_export',
      { suggestedName: '第一部小说' },
    ])
    expect(res).toEqual(archiveGrant)
  })

  it('恢复目标选择器同样只下发命名参数 suggestedName', async () => {
    const res = await ipc.invoke('dialog:select-project-restore-target', '第一部小说-恢复副本')

    expect(tauri.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'dialog_select_project_restore_target',
      { suggestedName: '第一部小说-恢复副本' },
    ])
    expect(res).toEqual(restoreGrant)
  })

  it('无参频道不带任何参数（也不注入项目会话）', async () => {
    await ipc.invoke('dialog:select-project-archive')
    expect(tauri.invoke.mock.calls[0]?.slice(0, 2)).toEqual(['dialog_select_project_archive', {}])

    await ipc.invoke('dialog:select-legacy-project')
    expect(tauri.invoke.mock.calls[1]?.slice(0, 2)).toEqual(['dialog_select_legacy_project', {}])
  })
})
