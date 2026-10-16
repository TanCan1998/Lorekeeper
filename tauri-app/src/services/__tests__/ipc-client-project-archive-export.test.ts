import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { ipc } from '../ipc-client'
import { MIGRATED_CHANNELS } from '../../shared/migrated-channels'
import type { ProjectSessionContext } from '../../shared/ipc-channels'
import { setActiveProjectSessionContext } from '../../shared/project-session-context'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

const session: ProjectSessionContext = {
  projectId: 'project-A', leaseId: 'lease-A', projectPath: 'C:/projects/A',
}
const receipt = {
  originProjectId: 'project-A',
  snapshotGeneration: 'gen-1',
  targetSha256: 'a'.repeat(64),
  targetByteSize: 4096,
  sourceEvidence: {
    schemaVersion: 7,
    schemaFingerprint: 'b'.repeat(64),
    tableCount: 51,
    fieldCount: 482,
  },
  entryCount: 3,
  requiresRuntimeFreezeGuard: true,
}

let tauri: TauriInternalsHandle

beforeEach(() => {
  tauri = installTauriInternals({
    commands: {
      project_archive_export: { success: true, receipt },
    },
  })
  setActiveProjectSessionContext(session)
})

afterEach(() => {
  setActiveProjectSessionContext(null)
  tauri.uninstall()
})

// `project:archive-export` 的会话在 request 内显式携带（契约如此），
// 因此适配器只下发命名参数 `request`，不追加尾参 `projectSession`。
describe('project:archive-export IPC transport', () => {
  it('频道已登记为已迁移频道', () => {
    expect(MIGRATED_CHANNELS.has('project:archive-export')).toBe(true)
  })

  it('只下发 request，且 request 内自带 projectSession', async () => {
    const request = { targetArchiveGrantId: "grant-1", projectSession: session }
    const res = await ipc.invoke('project:archive-export', request)

    expect(tauri.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'project_archive_export',
      { request },
    ])
    expect(res).toEqual({ success: true, receipt })
  })
})
