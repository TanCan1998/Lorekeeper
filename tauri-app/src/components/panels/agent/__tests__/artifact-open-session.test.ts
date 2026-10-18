import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({
  alertError: vi.fn(),
}))

vi.mock('../../../ui/AlertDialog', () => ({
  alertError: mocks.alertError,
}))

import { openArtifactInEditor } from '../artifact-open'
import { setActiveProjectSessionContext } from '../../../../shared/project-session-context'
import { useEditorStore } from '../../../../stores/editor-store'
import { useStageStore } from '../../../../stores/navigation/stage-store'
import { useLocaleStore } from '../../../../stores/locale-store'
import { useProjectStore } from '../../../../stores/project-store'
import { installTauriInternals, type TauriInternalsHandle } from '../../../../../test/helpers/tauri-internals'

const projectPath = 'C:\\novels\\same-project'

const originalLocale = useLocaleStore.getState().locale

function projectWithLease(leaseId: string) {
  return {
    id: 'same-project-id',
    sessionLease: leaseId,
    name: 'Same project',
    path: projectPath,
    novelConfig: {},
  } as never
}

let tauriInternals: TauriInternalsHandle
beforeEach(() => {
  mocks.alertError.mockReset()
  useStageStore.setState({ sheet: null })
  useLocaleStore.setState({ locale: 'en-US' })
  useEditorStore.getState().clearTabs()
  useProjectStore.setState({ currentProject: projectWithLease('lease-B') })
  setActiveProjectSessionContext({
    projectId: 'same-project-id',
    leaseId: 'lease-B',
    projectPath,
  })
})

afterEach(() => {
  tauriInternals?.uninstall()
  vi.unstubAllGlobals()
  useEditorStore.getState().clearTabs()
  useProjectStore.setState({ currentProject: null })
  setActiveProjectSessionContext(null)
  useLocaleStore.setState({ locale: originalLocale })
})

describe('agent artifact open session ownership', () => {
  it('presents artifact-open failures in the active application locale', async () => {
    await expect(openArtifactInEditor({
      type: 'file_modified',
      name: 'chapter_1.md',
      path: `${projectPath}\\manuscript\\chapter_1.md`,
    } as never)).resolves.toBe(false)

    expect(mocks.alertError).toHaveBeenCalledWith(
      'The artifact is missing source project information and cannot be opened safely.',
      { title: 'Cannot open artifact' },
    )
  })

  it('fails closed when a legacy artifact has no frozen project session', async () => {
    tauriInternals = installTauriInternals({})

    await expect(openArtifactInEditor({
      type: 'file_modified',
      name: 'chapter_1.md',
      path: `${projectPath}\\manuscript\\chapter_1.md`,
      projectPath,
    } as never)).resolves.toBe(false)

    expect(tauriInternals.invoke).not.toHaveBeenCalled()
    expect(useEditorStore.getState().tabs).toEqual([])
    expect(mocks.alertError).toHaveBeenCalledOnce()
  })

  it('fails closed for an artifact from an older lease of the same project path', async () => {
    tauriInternals = installTauriInternals({})
    const result = await openArtifactInEditor({
      type: 'file_modified',
      name: 'chapter_1.md',
      path: `${projectPath}\\manuscript\\chapter_1.md`,
      projectPath,
      projectSession: {
        projectId: 'same-project-id',
        leaseId: 'lease-A',
        projectPath,
      },
    } as never)

    expect(result).toBe(false)
    expect(tauriInternals.invoke).not.toHaveBeenCalled()
    expect(useEditorStore.getState().tabs).toEqual([])
    expect(mocks.alertError).toHaveBeenCalledOnce()
  })
  it('同会话的工作流产物只打开持久候选面板，不重启任务或读取文件', async () => {
    tauriInternals = installTauriInternals({})
    expect(await openArtifactInEditor({ type: 'workflow_started', name: 'generate_draft',
      runId: 'synthetic-registration', status: 'waiting', projectPath,
      projectSession: { projectId: 'same-project-id', leaseId: 'lease-B', projectPath },
    })).toBe(true)
    expect(useStageStore.getState()).toMatchObject({ sheet: { kind: 'run', tab: 'active' } })
    expect(tauriInternals.invoke).not.toHaveBeenCalled()
    expect(useEditorStore.getState().tabs).toEqual([])
    expect(mocks.alertError).not.toHaveBeenCalled()
  })

  it.each(['lease-A', ''])('工作流产物不借用旧会话或缺失的登记身份：%s', async leaseId => {
    tauriInternals = installTauriInternals({})
    expect(await openArtifactInEditor({ type: 'workflow_started', name: 'generate_draft',
      runId: leaseId ? 'synthetic-registration' : '', status: 'waiting', projectPath,
      projectSession: { projectId: 'same-project-id', leaseId: leaseId || 'lease-B', projectPath },
    })).toBe(false)
    expect(useStageStore.getState()).toMatchObject({ sheet: null })
    expect(tauriInternals.invoke).not.toHaveBeenCalled()
  })

})
