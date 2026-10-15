import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import {
  setActiveProjectSessionContext,
} from '../../shared/project-session-context'
import { ipc } from '../ipc-client'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

// Tauri 语义：ipc-client 直接调 `@tauri-apps/api/core` 的 invoke（命名参数），
// 不再有 Electron `window.velaAPI` 读取点。这里注入 Tauri 运行时的内部全局桩
// （`window.__TAURI_INTERNALS__.invoke`），让真实 ipc-client 逻辑（频道映射 /
// 命名参数 / 会话注入 / 未迁移前置拦截）得到完整覆盖。
let tauriInternals: TauriInternalsHandle

beforeEach(() => {
  tauriInternals = installTauriInternals({
    commands: {
      db_blueprint_get_all: [],
      fs_grant_write_file: { success: true },
      agent_generation_begin: null,
      agent_generation_read: null,
      agent_generation_resume: null,
      agent_generation_round: null,
      agent_generation_claim_tool: null,
      agent_generation_finish_tool: null,
      agent_generation_register_workflow: null,
      agent_generation_commit_domain_tool: null,
    },
  })
  setActiveProjectSessionContext({
    projectId: 'project-A',
    leaseId: 'lease-A',
    projectPath: 'C:/projects/A',
  })
})

afterEach(() => {
  setActiveProjectSessionContext(null)
  tauriInternals.uninstall()
})

describe('project-scoped IPC session transport', () => {
  it('appends the frozen active session as a named argument for project channels', async () => {

    await ipc.invoke('db:blueprint-get-all', 'C:/projects/A')

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    // 真实 Tauri core `invoke` 会带上第三个 `options` 参数，故只断言前两位
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual(['db_blueprint_get_all', {
      expectedProjectPath: 'C:/projects/A',
      projectSession: {
        projectId: 'project-A',
        leaseId: 'lease-A',
        projectPath: 'C:/projects/A',
      },
    }])
  })

  it.each([
    'agent-generation:begin', 'agent-generation:read', 'agent-generation:resume',
    'agent-generation:round', 'agent-generation:claim-tool', 'agent-generation:finish-tool',
    'agent-generation:register-workflow', 'agent-generation:commit-domain-tool',
  ] as const)('按命名参数发送 %s 并附带项目会话', async channel => {
    // 这里只验证传输打包；领域请求校验由后端合同测试覆盖。
    const request = { synthetic: 'agent-transport' }
    await ipc.invoke(channel, request as never)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      channel.replace(/[:-]/g, '_'),
      { request, projectSession: {
        projectId: 'project-A', leaseId: 'lease-A', projectPath: 'C:/projects/A',
      } },
    ])
  })


  it('allows a capability-grant filesystem request without an active project session', async () => {
    setActiveProjectSessionContext(null)

    await ipc.invoke('fs:grant-write-file', 'grant-export-1', 'chapter.txt', 'content')

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual(['fs_grant_write_file', {
      grantId: 'grant-export-1',
      relativePath: 'chapter.txt',
      content: 'content',
    }])
  })
})
