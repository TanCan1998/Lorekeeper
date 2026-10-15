import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { ipc } from '../ipc-client'
import { MIGRATED_CHANNELS } from '../../shared/migrated-channels'
import type { ProjectSessionContext } from '../../shared/ipc-channels'
import { getActiveProjectSessionContext, setActiveProjectSessionContext } from '../../shared/project-session-context'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

const session: ProjectSessionContext = {
  projectId: 'synthetic-project', leaseId: 'synthetic-lease', projectPath: 'C:/synthetic-project',
}
const slot = {
  source: { draftId: 7, finalizationId: 'synthetic-finalization', chapterNumber: 2, contentHash: 'a'.repeat(64) },
  stepKey: 'chapter_notes' as const,
}
let tauri: TauriInternalsHandle
let priorSession: ProjectSessionContext | null
beforeEach(() => {
  priorSession = getActiveProjectSessionContext()
  setActiveProjectSessionContext(null)
  tauri = installTauriInternals()
})
afterEach(() => {
  tauri.uninstall()
  setActiveProjectSessionContext(priorSession)
})

// 使用真实适配器而非 ipc mock，证明前端不会把未迁移特殊域送到主进程。
const cases = [
  { channel: 'graph-generation:begin',
    invoke: () => ipc.invokeWithProjectSession(session, 'graph-generation:begin',
      { input: { kind: 'plot' }, modelId: 'synthetic-model', uiActionNonce: 'synthetic-action' }) },
  { channel: 'finalized-character:read-context',
    invoke: () => ipc.invokeWithProjectSession(session, 'finalized-character:read-context', { draftId: 7 }) },
  { channel: 'finalized-character:list-state-candidates',
    invoke: () => ipc.invokeWithProjectSession(session, 'finalized-character:list-state-candidates') },
  { channel: 'finalization-generation:begin',
    invoke: () => ipc.invokeWithProjectSession(session, 'finalization-generation:begin',
      { slot, modelId: 'synthetic-model' }) },
] as const

describe('未开放特殊域的 Tauri 传输边界', () => {
  it.each(cases)('$channel 即使提供冻结会话也在传输前拒绝', async ({ channel, invoke }) => {
    expect(MIGRATED_CHANNELS.has(channel)).toBe(false)
    await expect(invoke()).rejects.toThrow(new RegExp(`频道 ${channel} 尚未迁移`))
    expect(tauri.invoke).not.toHaveBeenCalled()
  })

  it('缺少项目会话时先拒绝，而不是借用外部授权或触达主进程', async () => {
    await expect(ipc.invoke('graph-generation:begin',
      { input: { kind: 'plot' }, modelId: 'synthetic-model', uiActionNonce: 'synthetic-action' }))
      .rejects.toThrow('缺少当前项目会话，已拒绝项目数据访问')
    expect(tauri.invoke).not.toHaveBeenCalled()
  })
})
