import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { ipc } from '../ipc-client'
import { setActiveProjectSessionContext } from '../../shared/project-session-context'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

const session = { projectId: 'project', leaseId: 'lease', projectPath: 'C:/project' }
const handle = { projectId: 'project', epoch: 'lease', runId: 'run', rootActionId: 'root' }
const prepare = { operation: 'review-chapter' as const, draftId: 1,
  expectedDraft: { chapterNumber: 1, version: 1, status: 'draft' as const, contentHash: 'a'.repeat(64) },
  authorInputs: [], uiLocale: 'zh-CN' as const }
const review = { contextId: 'context', handle, artifact: { artifactId: 'artifact', revision: 1, textHash: 'b'.repeat(64) } }
const revision = { contextId: 'context', handle, expectedCompositionHash: 'c'.repeat(64) }
let tauri: TauriInternalsHandle
beforeEach(() => { setActiveProjectSessionContext(null); tauri = installTauriInternals({ fallback: { accepted: true } }) })
afterEach(() => { tauri.uninstall(); setActiveProjectSessionContext(null) })
describe('审稿修稿的有界生产传输合同', () => {
  it('四频道仅透传完整 request 信封，并由独立会话参数提供项目权威', async () => {
    const requests = [prepare, review, revision, { handle }]
    const calls = [
      () => ipc.invokeWithProjectSession(session, 'review-revision:prepare', Object.freeze(prepare)),
      () => ipc.invokeWithProjectSession(session, 'review-revision:commit-review', Object.freeze(review)),
      () => ipc.invokeWithProjectSession(session, 'review-revision:commit-revision', Object.freeze(revision)),
      () => ipc.invokeWithProjectSession(session, 'review-revision:read-recovery', { handle }),
    ]
    const commands = ['prepare', 'commit_review', 'commit_revision', 'read_recovery']
    for (const [index, call] of calls.entries()) {
      await call()
      expect(tauri.invoke).toHaveBeenLastCalledWith('review_revision_' + commands[index], {
        request: requests[index], projectSession: session,
      }, undefined)
    }
    expect(tauri.invoke).toHaveBeenCalledTimes(4)
  })
  it('活动项目入口复用相同信封，不修改冻结请求', async () => {
    setActiveProjectSessionContext(session)
    await ipc.invoke('review-revision:prepare', Object.freeze(prepare))
    expect(tauri.invoke).toHaveBeenCalledWith('review_revision_prepare', {
      request: prepare, projectSession: session,
    }, undefined)
  })
  it('没有会话时在传输前拒绝', async () => {
    await expect(ipc.invoke('review-revision:prepare', prepare)).rejects.toThrow('缺少当前项目会话')
    expect(tauri.invoke).not.toHaveBeenCalled()
  })
})
