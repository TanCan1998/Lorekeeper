import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { ipc } from '../ipc-client'
import { setActiveProjectSessionContext } from '../../shared/project-session-context'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

const session = { projectId: 'synthetic-project', leaseId: 'synthetic-epoch', projectPath: 'C:/synthetic-project' }
const handle = { projectId: session.projectId, epoch: session.leaseId, rootActionId: 'synthetic-root', runId: 'synthetic-run' }
let backend: TauriInternalsHandle
beforeEach(() => {
  backend = installTauriInternals({ commands: {
    generation_read_context: { handle }, generation_read_batch: { batchId: 'synthetic-batch' },
    generation_read: { handle }, generation_read_visible_composition: null,
  } })
  setActiveProjectSessionContext(session)
})
afterEach(() => { setActiveProjectSessionContext(null); backend.uninstall() })

describe('主生成读面请求包装', () => {
  it('恢复上下文保留单层 request.handle，项目会话单独注入', async () => {
    await ipc.invokeWithProjectSession(session, 'generation:read-context', { handle })
    expect(backend.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'generation_read_context', { request: { handle }, projectSession: session },
    ])
  })
  it('批次进度保留 request.batchId，不把请求对象误作 ID', async () => {
    await ipc.invokeWithProjectSession(session, 'generation:read-batch', { batchId: 'synthetic-batch' })
    expect(backend.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'generation_read_batch', { request: { batchId: 'synthetic-batch' }, projectSession: session },
    ])
  })
  it('裸句柄合同仍映射到 handle，不作通用解包', async () => {
    await ipc.invokeWithProjectSession(session, 'generation:read', handle)
    await ipc.invokeWithProjectSession(session, 'generation:read-visible-composition', handle)
    expect(backend.invoke.mock.calls.map(call => call.slice(0, 2))).toEqual([
      ['generation_read', { handle, projectSession: session }],
      ['generation_read_visible_composition', { handle, projectSession: session }],
    ])
  })
})
