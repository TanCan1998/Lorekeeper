import { act } from 'react'
import type { ReactNode } from 'react'
import { EditorView } from '@codemirror/view'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { RecoveryCandidate } from '../../../shared/recovery-candidate'
import { setActiveProjectSessionContext } from '../../../shared/project-session-context'
import { saveDirtyEditorChangesForExit, useEditorStore } from '../../../stores/editor-store'
import { useLayoutStore } from '../../../stores/layout-store'
import { useProjectStore } from '../../../stores/project-store'
import { useWorkflowStore } from '../../../stores/workflow-store'
import AIOutputPanel from '../AIOutputPanel'
import EditorArea from '../EditorArea'
import ShellV2 from '../../layout/v2/ShellV2'

;(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

let root: Root | undefined
let container: HTMLDivElement | undefined
let invoke: ReturnType<typeof vi.fn>
let clipboardWrite: ReturnType<typeof vi.fn>
let storedCandidate: RecoveryCandidate

const projectPath = 'C:\\novels\\recovery-project'
const session = {
  projectId: 'recovery-project',
  leaseId: 'recovery-lease',
  projectPath,
}

function candidate(overrides: Partial<RecoveryCandidate> = {}): RecoveryCandidate {
  return {
    candidateId: 'candidate-1',
    runId: 'run-1',
    stepId: 'generate-draft',
    projectId: session.projectId,
    chapterNumber: 3,
    chapterTitle: '失控列车',
    sourceHash: 'a'.repeat(64),
    visibleText: '林岚推开驾驶室的门。',
    contentHash: 'b'.repeat(64),
    failureCode: 'PROVIDER_REQUEST_FAILED',
    failureReason: 'connection reset',
    status: 'pending',
    replacesCandidateId: null,
    sourceCurrent: true,
    createdAt: '2026-09-06T00:00:00.000Z',
    resolvedAt: null,
    ...overrides,
  }
}

function button(label: string): HTMLButtonElement | undefined {
  return Array.from(container?.querySelectorAll('button') ?? [])
    .find(item => item.textContent?.includes(label))
}

async function renderWriter(editor: ReactNode = <p>正文</p>) {
  await act(async () => {
    root?.render(
      <ShellV2
        theme="paper"
        titleBar={<span>Recovery project</span>}
        rail={<span>小说</span>}
        sidebar={<span>作品资料</span>}
        editor={editor}
        aiPanel={<AIOutputPanel />}
        bottom={<span>任务</span>}
        statusBar={<span>本地写作</span>}
      />,
    )
    await Promise.resolve()
  })
  expect(container?.querySelector('[data-shell-presentation="writer"]')).not.toBeNull()
  expect(container?.querySelector('[data-shell-variant="v3"]')).not.toBeNull()
}

beforeEach(() => {
  useWorkflowStore.setState({ activeRuns: [], history: [], currentRun: null })
  useProjectStore.setState({
    currentProject: {
      id: session.projectId,
      name: 'Recovery project',
      path: projectPath,
      sessionLease: session.leaseId,
      novelConfig: {},
    } as never,
  })
  setActiveProjectSessionContext(session)
  useEditorStore.setState({ tabs: [], activeTabId: null, draftLedgers: {} })
  useLayoutStore.setState({ sidebarView: 'project' })
  storedCandidate = candidate()
  invoke = vi.fn(async (command: string, args: Record<string, unknown> = {}) => {
    if (command === 'plugin:event|listen') return 1
    if (command === 'plugin:event|unlisten') return undefined
    if (command === 'db_recovery_candidate_list') return [storedCandidate]
    if (command === 'generation_list' || command === 'generation_list_batches') return []
    if (command === 'db_recovery_candidate_update') {
      storedCandidate = candidate({ candidateId: args.candidateId as string, visibleText: args.visibleText as string })
      return { success: true, candidate: storedCandidate }
    }
    if (command === 'db_recovery_candidate_resolve') return { success: true }
    throw new Error(`unexpected IPC: ${command}`)
  })
  clipboardWrite = vi.fn().mockResolvedValue(undefined)
  ;(window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {
    invoke,
    transformCallback: () => 1,
  }
  Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', {
    configurable: true,
    value: { unregisterListener: vi.fn() },
  })
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText: clipboardWrite },
  })
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root?.unmount())
  container?.remove()
  root = undefined
  container = undefined
  setActiveProjectSessionContext(null)
  delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
  delete (window as unknown as { __TAURI_EVENT_PLUGIN_INTERNALS__?: unknown }).__TAURI_EVENT_PLUGIN_INTERNALS__
})

describe('AI output recovery candidates', () => {
  it('discovers a pending candidate after project reopen and copies or continues it explicitly', async () => {
    await renderWriter()

    expect(container?.textContent).toContain('恢复候选')
    expect(container?.textContent).toContain('原始候选')
    expect(container?.textContent).toContain('第3章 失控列车')
    expect(container?.textContent).toContain('林岚推开驾驶室的门。')

    await act(async () => button('复制')?.click())
    expect(clipboardWrite).toHaveBeenCalledWith('林岚推开驾驶室的门。')

    await act(async () => {
      button('继续编辑')?.click()
      await Promise.resolve()
    })
    expect(invoke).toHaveBeenCalledWith(
      'db_recovery_candidate_update',
      {
        candidateId: 'candidate-1',
        visibleText: '林岚推开驾驶室的门。',
        expectedProjectPath: projectPath,
        projectSession: session,
      },
      undefined,
    )
    expect(invoke).not.toHaveBeenCalledWith(
      'db_recovery_candidate_resolve',
      {
        candidateId: 'candidate-1',
        status: 'continued',
        expectedProjectPath: projectPath,
        projectSession: session,
      },
      undefined,
    )
    expect(useEditorStore.getState().tabs).toEqual([
      expect.objectContaining({
        type: 'chapter',
        content: '林岚推开驾驶室的门。',
        savedContent: '林岚推开驾驶室的门。',
        dirty: false,
        projectKey: projectPath,
        projectSessionLease: session.leaseId,
      }),
    ])
    expect(container?.textContent).not.toContain('林岚推开驾驶室的门。')

    await renderWriter(<EditorArea onNewProject={vi.fn()} />)
    expect(container?.querySelector('[data-shell-presentation="writer"]')).not.toBeNull()
    const recoveryTab = useEditorStore.getState().tabs.find(tab => (
      tab.filePath === 'ai-novel://recovery/candidate-1'
    ))!
    expect(container?.querySelector('[data-shell-presentation="writer"]')).not.toBeNull()
    await act(async () => useEditorStore.getState().setActiveTab(recoveryTab.id))

    expect(container?.querySelector('[data-shell-presentation="writer"]')).not.toBeNull()
    expect(container?.textContent).toContain('项目恢复候选')
    expect(container?.textContent).toContain('林岚推开驾驶室的门。')
    await act(async () => {
      const editor = EditorView.findFromDOM(container!.querySelector<HTMLElement>('.cm-editor')!)!
      editor.dispatch({
        changes: { from: 0, to: editor.state.doc.length, insert: '林岚停下列车，保存现场。' },
      })
    })
    expect(container?.querySelector('button[title^="保存"]')).not.toBeNull()
    expect(container?.querySelector('[data-shell-presentation="writer"]')).not.toBeNull()
    await act(async () => saveDirtyEditorChangesForExit(projectPath))
    expect(useEditorStore.getState().tabs.find(tab => tab.id === recoveryTab.id)).toEqual(
      expect.objectContaining({
        content: '林岚停下列车，保存现场。',
        dirty: false,
        savedContent: '林岚停下列车，保存现场。',
      }),
    )
    expect(invoke).toHaveBeenCalledWith(
      'db_recovery_candidate_update',
      {
        candidateId: 'candidate-1',
        visibleText: '林岚停下列车，保存现场。',
        expectedProjectPath: projectPath,
        projectSession: session,
      },
      undefined,
    )
    expect(invoke.mock.calls.some(([command]) => (
      command === 'fs_write_file'
      || command === 'db_draft_create'
      || command === 'db_draft_update_content'
      || command === 'db_finalization_upsert'
      || command.startsWith('kb_')
    ))).toBe(false)

    expect(container?.querySelector('[data-shell-presentation="writer"]')).not.toBeNull()
    expect(container?.textContent).toContain('林岚停下列车，保存现场。')
  })

  it('discards explicitly without creating an editor tab', async () => {
    await renderWriter()
    await act(async () => {
      button('放弃')?.click()
      await Promise.resolve()
    })

    expect(invoke).toHaveBeenCalledWith(
      'db_recovery_candidate_resolve',
      {
        candidateId: 'candidate-1',
        status: 'discarded',
        expectedProjectPath: projectPath,
        projectSession: session,
      },
      undefined,
    )
    expect(useEditorStore.getState().tabs).toEqual([])
  })

  it('blocks continue after the source changed while keeping copy and discard available', async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'plugin:event|listen') return 1
      if (command === 'plugin:event|unlisten') return undefined
      if (command === 'db_recovery_candidate_list') return [candidate({ sourceCurrent: false })]
      if (command === 'db_recovery_candidate_resolve') return { success: true }
      throw new Error(`unexpected IPC: ${command}`)
    })
    await renderWriter()

    expect(container?.textContent).toContain('源章节已变化')
    expect(button('继续编辑')).toBeDisabled()
    expect(button('复制')).not.toBeDisabled()
    expect(button('放弃')).not.toBeDisabled()
  })
})

// 历史侧栏不自行重绑身份；显式继续复用专用客户端的主进程重认证。
it.each([false,true])('旧租约行内任务侧栏显式恢复，后端拒绝=%s', async denied => {
  const body = '合成原文'
  const documentHash = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(body))), b => b.toString(16).padStart(2, '0')).join('')
  const oldHandle = { projectId: session.projectId, epoch: 'old-editor-lease', rootActionId: 'editor-root', runId: 'editor-run' }
  const recovery = { view: { handle: oldHandle, operation: 'editor-inline', status: 'paused', ledger: { physicalRequests: 0 }, artifacts: [] as unknown[] }, modelId: 'synthetic', sourceStatus: 'conflict', context: { kind: 'author-draft', action: 'refine', documentText: body, documentHash, from: 0, to: 2, selectedText: '合成' } }
  const previous = invoke.getMockImplementation() as (command: string, args: Record<string, unknown>) => Promise<unknown>
  invoke.mockImplementation(async (command: string, args: Record<string, unknown> = {}) => {
    if (command === 'generation_list') return [structuredClone(recovery.view)]
    if (command === 'generation_read_context') return { operation: 'editor-inline' }
    if (command === 'editor_inline_read_recovery') return structuredClone(recovery)
    if (command === 'generation_resume') {
      if (denied) throw new Error('GENERATION_RECOVERY_UNAUTHORIZED')
      recovery.view.handle = { ...oldHandle, epoch: session.leaseId }; recovery.sourceStatus = 'current'; recovery.view.status = 'running'
      return structuredClone(recovery.view)
    }
    if (command === 'editor_inline_execute') {
      expect(args.request).toEqual({ handle: { ...oldHandle, epoch: session.leaseId } })
      recovery.view.ledger.physicalRequests = 1
      recovery.view.artifacts = [{ artifactId: 'suggestion', text: '恢复后的合成建议', status: 'completed' }]
      return { run: structuredClone(recovery.view), outcome: { status: 'completed', content: '恢复后的合成建议', finishReason: 'stop' } }
    }
    return previous(command, args)
  })
  await renderWriter()
  await vi.waitFor(() => expect(button('继续原选区任务')).toBeDefined())
  expect(button('继续原选区任务')?.disabled).toBe(false)
  expect(invoke.mock.calls.some(([c]) => c === 'generation_resume')).toBe(false)
  await act(async () => button('继续原选区任务')?.click())
  await vi.waitFor(() => expect(container?.textContent).toContain(denied ? 'GENERATION_RECOVERY_UNAUTHORIZED' : '恢复后的合成建议'))
  expect(invoke.mock.calls.some(([c]) => c === 'editor_inline_execute')).toBe(!denied)
})
