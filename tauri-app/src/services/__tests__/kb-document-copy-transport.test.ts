import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { ipc } from '../ipc-client'
import { MIGRATED_CHANNELS } from '../../shared/migrated-channels'
import { setActiveProjectSessionContext } from '../../shared/project-session-context'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

const session = { projectId: 'project-A', leaseId: 'lease-A', projectPath: 'C:/projects/A' }
let runtime: TauriInternalsHandle

beforeEach(() => {
  runtime = installTauriInternals({
    commands: {
      kb_read_document_copy: { available: false, indexStatus: 'unavailable' },
      kb_save_document_copy: { success: true },
      kb_reindex_document_copy: { success: true, docId: 'doc-1', chunkCount: 2 },
    },
  })
  setActiveProjectSessionContext(session)
})
afterEach(() => {
  setActiveProjectSessionContext(null)
  runtime.uninstall()
})

describe('知识库文档副本传输合同', () => {
  it('三个频道均已迁移，按契约传参并注入冻结会话', async () => {
    for (const channel of ['kb:read-document-copy', 'kb:save-document-copy', 'kb:reindex-document-copy']) {
      expect(MIGRATED_CHANNELS.has(channel)).toBe(true)
    }
    await ipc.invoke('kb:read-document-copy', 'doc-1', session.projectPath)
    await ipc.invoke('kb:save-document-copy', 'doc-1', '测试内容', 'hash-1', session.projectPath)
    await ipc.invoke('kb:reindex-document-copy', 'doc-1', session.projectPath)
    const common = { docId: 'doc-1', expectedProjectPath: session.projectPath, projectSession: session }
    expect(runtime.invoke.mock.calls.map(call => call.slice(0, 2))).toEqual([
      ['kb_read_document_copy', common],
      ['kb_save_document_copy', { ...common, content: '测试内容', expectedContentHash: 'hash-1' }],
      ['kb_reindex_document_copy', common],
    ])
  })

  it('显式旧会话不被活动会话替换，交由后端拒绝', async () => {
    const old = { ...session, leaseId: 'lease-old' }
    await ipc.invokeWithProjectSession(old, 'kb:save-document-copy', 'doc-1', '', 'old-hash', session.projectPath)
    expect(runtime.invoke.mock.calls[0]?.[1]).toEqual({
      docId: 'doc-1', content: '', expectedContentHash: 'old-hash',
      expectedProjectPath: session.projectPath, projectSession: old,
    })
  })

  it('不扩大其它未开放域的权限', async () => {
    await expect(ipc.invoke('project:import-legacy-copy', 'source-grant', 'target-grant')).rejects.toThrow('尚未迁移')
    expect(runtime.invoke).not.toHaveBeenCalled()
  })
})
