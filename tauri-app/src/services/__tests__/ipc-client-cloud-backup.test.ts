import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { CloudBackupNowRequest } from '../../shared/cloud-backup'
import { ipc } from '../ipc-client'
import { MIGRATED_CHANNELS } from '../../shared/migrated-channels'
import { setActiveProjectSessionContext } from '../../shared/project-session-context'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

const session = { projectId: 'selected', leaseId: 'lease', projectPath: 'C:/selected' }
const channels = [
  'cloud-backup:view', 'cloud-backup:connect', 'cloud-backup:confirm-binding',
  'cloud-backup:backup', 'cloud-backup:list', 'cloud-backup:restore-copy',
  'cloud-backup:cancel', 'cloud-backup:clear-credential',
] as const
let tauri: TauriInternalsHandle
beforeEach(() => {
  tauri = installTauriInternals({
    commands: Object.fromEntries(channels.map(channel => [
      channel.replace(/[:-]/g, '_'), { success: true },
    ])),
  })
  setActiveProjectSessionContext(null)
})
afterEach(() => {
  setActiveProjectSessionContext(null)
  tauri.uninstall()
})
function expectInvoke(command: string, args: Record<string, unknown>) {
  expect(tauri.invoke.mock.calls[0]?.slice(0, 2)).toEqual([command, args])
}
describe('云备份八频道的命名参数与显式会话', () => {
  it('八个频道均登记为已迁移', () => {
    for (const channel of channels) expect(MIGRATED_CHANNELS.has(channel)).toBe(true)
  })
  it('view使用显式会话，不被活动项目覆盖', async () => {
    setActiveProjectSessionContext({ projectId: 'other', leaseId: 'other-lease', projectPath: 'C:/other' })
    await ipc.invoke('cloud-backup:view', session)
    expectInvoke('cloud_backup_view', { projectSession: session })
  })
  it('connect不要求活动项目且只携带请求', async () => {
    const request = { endpoint: 'https://example.test/dav/', username: 'test-user', secret: 'unit-test-only' }
    await ipc.invoke('cloud-backup:connect', request)
    expectInvoke('cloud_backup_connect', { request })
  })
  it('confirm-binding保持修订号、父代选择及请求内会话', async () => {
    const request = { projectSession: session, localEndpointAccountId: 'account',
      lastSelectedParentGenerationIds: ['parent-2', 'parent-1'], expectedRevision: 3 }
    await ipc.invoke('cloud-backup:confirm-binding', request)
    expectInvoke('cloud_backup_confirm_binding', { request })
  })
  it('backup不补写披露确认或额外会话', async () => {
    const request: CloudBackupNowRequest = { operationId: 'operation', projectSession: session, disclosureConfirmed: false }
    await ipc.invoke('cloud-backup:backup', request)
    expectInvoke('cloud_backup_backup', { request })
  })
  it('list是账户与书籍能力域请求', async () => {
    const request = { localEndpointAccountId: 'account', cloudBookId: 'book' }
    await ipc.invoke('cloud-backup:list', request)
    expectInvoke('cloud_backup_list', { request })
  })
  it('restore-copy只传目标授权，不传本地路径', async () => {
    const request = { operationId: 'operation', localEndpointAccountId: 'account',
      cloudBookId: 'book', generationId: 'generation', targetGrantId: 'target-grant' }
    await ipc.invoke('cloud-backup:restore-copy', request)
    expectInvoke('cloud_backup_restore_copy', { request })
  })
  it('cancel命名operationId', async () => {
    await ipc.invoke('cloud-backup:cancel', 'operation')
    expectInvoke('cloud_backup_cancel', { operationId: 'operation' })
  })
  it('clear-credential命名localEndpointAccountId', async () => {
    await ipc.invoke('cloud-backup:clear-credential', 'account')
    expectInvoke('cloud_backup_clear_credential', { localEndpointAccountId: 'account' })
  })
})
