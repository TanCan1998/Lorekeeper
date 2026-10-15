import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import {
  setActiveProjectSessionContext,
} from '../../shared/project-session-context'
import type { ProjectOverview } from '../../shared/project-overview'
import { ipc } from '../ipc-client'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

let tauriInternals: TauriInternalsHandle

beforeEach(() => {
  tauriInternals = installTauriInternals({
    commands: {
      project_overview_current: {
        state: 'ready',
        projectId: 'project-A',
        revision: 'rev-fact-1',
        name: '测试小说A',
        totalWords: 1500,
        characters: 3,
        finalizedChapters: 1,
        stages: [
          { id: 'configuration', status: 'completed', count: 9 },
          { id: 'architecture', status: 'completed', count: 4 },
          { id: 'blueprint', status: 'in-progress', count: 2 },
          { id: 'drafting', status: 'in-progress', count: 2 },
          { id: 'review', status: 'in-progress', count: 1 },
          { id: 'finalization', status: 'in-progress', count: 1 },
        ],
        excerpt: '最新章节结尾240字片段',
      } satisfies ProjectOverview,
      project_peek_overview: {
        state: 'ready',
        projectId: 'project-B',
        revision: 'rev-fact-2',
        name: '离线预览小说B',
        totalWords: 3000,
        characters: 5,
        finalizedChapters: 2,
        stages: [
          { id: 'configuration', status: 'completed', count: 9 },
          { id: 'architecture', status: 'completed', count: 4 },
          { id: 'blueprint', status: 'completed', count: 5 },
          { id: 'drafting', status: 'completed', count: 5 },
          { id: 'review', status: 'completed', count: 3 },
          { id: 'finalization', status: 'completed', count: 2 },
        ],
      } satisfies ProjectOverview,
    },
  })
})

afterEach(() => {
  setActiveProjectSessionContext(null)
  tauriInternals.uninstall()
})

describe('project overview IPC transport', () => {
  it('invokes project:overview-current with active session context injected', async () => {
    setActiveProjectSessionContext({
      projectId: 'project-A',
      leaseId: 'lease-A',
      projectPath: 'C:/projects/A',
    })

    const overview = await ipc.invoke('project:overview-current')

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'project_overview_current',
      {
        projectSession: {
          projectId: 'project-A',
          leaseId: 'lease-A',
          projectPath: 'C:/projects/A',
        },
      },
    ])
    expect(overview.state).toBe('ready')
    if (overview.state === 'ready') {
      expect(overview.name).toBe('测试小说A')
      expect(overview.totalWords).toBe(1500)
      expect(overview.excerpt).toBe('最新章节结尾240字片段')
      expect(overview.stages).toHaveLength(6)
    }
  })

  it('invokes project:peek-overview with capabilityId and without project session context', async () => {
    // 即使存在活跃会话，peek 也只使用凭证，不带 projectSession
    setActiveProjectSessionContext({
      projectId: 'project-A',
      leaseId: 'lease-A',
      projectPath: 'C:/projects/A',
    })

    const overview = await ipc.invoke('project:peek-overview', 'cap-uuid-123')

    expect(tauriInternals.invoke).toHaveBeenCalledTimes(1)
    expect(tauriInternals.invoke.mock.calls[0]?.slice(0, 2)).toEqual([
      'project_peek_overview',
      {
        capabilityId: 'cap-uuid-123',
      },
    ])
    expect(overview.state).toBe('ready')
    if (overview.state === 'ready') {
      expect(overview.name).toBe('离线预览小说B')
      expect(overview.totalWords).toBe(3000)
      expect(overview.excerpt).toBeUndefined()
    }
  })
})
