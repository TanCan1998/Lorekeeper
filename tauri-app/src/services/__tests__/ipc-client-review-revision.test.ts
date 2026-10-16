import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { ipc } from '../ipc-client'
import { setActiveProjectSessionContext } from '../../shared/project-session-context'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

const session = { projectId: 'project', leaseId: 'lease', projectPath: 'C:/project' }
const handle = { projectId: 'project', epoch: 'lease', runId: 'run', rootActionId: 'root' }
const prepare = { operation: 'review-chapter' as const, draftId: 1,
  expectedDraft: { chapterNumber: 1, version: 1, status: 'draft' as const, contentHash: 'a'.repeat(64) },
  authorInputs: [], uiLocale: 'zh-CN' as const }
let tauri: TauriInternalsHandle
beforeEach(() => { setActiveProjectSessionContext(null); tauri=installTauriInternals() })
afterEach(() => { tauri.uninstall(); setActiveProjectSessionContext(null) })
describe('审稿修稿保持显式关闭的生产传输合同', () => {
  it('即使提供形状完整的四种请求，也不因业务夹具可执行而开放生产写面', async () => {
    const review = { contextId: 'context', handle, artifact: { artifactId: 'artifact', revision: 1, textHash: 'b'.repeat(64) } }
    const revision = { contextId: 'context', handle, expectedCompositionHash: 'c'.repeat(64) }
    for(const call of [
      () => ipc.invokeWithProjectSession(session,'review-revision:prepare',prepare),
      () => ipc.invokeWithProjectSession(session,'review-revision:commit-review',review),
      () => ipc.invokeWithProjectSession(session,'review-revision:commit-revision',revision),
      () => ipc.invokeWithProjectSession(session,'review-revision:read-recovery',{handle}),
    ]) await expect(call()).rejects.toThrow('尚未迁移（参数名未登记）')
    expect(tauri.invoke).not.toHaveBeenCalled()
  })
  it('活动项目也不能让关闭门禁失效', async () => {
    setActiveProjectSessionContext(session)
    await expect(ipc.invoke('review-revision:prepare',Object.freeze(prepare))).rejects.toThrow('尚未迁移')
    expect(tauri.invoke).not.toHaveBeenCalled()
  })
  it('没有会话时先拒绝', async () => {
    await expect(ipc.invoke('review-revision:prepare',prepare)).rejects.toThrow('缺少当前项目会话')
    expect(tauri.invoke).not.toHaveBeenCalled()
  })
})
