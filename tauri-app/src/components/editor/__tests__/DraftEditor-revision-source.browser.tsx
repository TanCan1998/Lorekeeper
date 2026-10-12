import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { page } from 'vitest/browser'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { setActiveProjectSessionContext } from '../../../shared/project-session-context'
import { saveDirtyEditorChangesForExit, useEditorStore } from '../../../stores/editor-store'
import { useProjectStore } from '../../../stores/project-store'
import { useWorkflowStore } from '../../../stores/workflow-store'
import DraftEditor from '../DraftEditor'

const PROJECT_PATH = 'C:\\novels\\revision-source'
const PROJECT_SESSION = Object.freeze({
  projectId: 'revision-source-project',
  leaseId: 'revision-source-lease',
  projectPath: PROJECT_PATH,
})
const SOURCE = '生成修订时的源稿 A。'
const CURRENT = `${SOURCE}作者后来保存的 B。`
const REVISION = 'AI 基于 A 生成的修订稿。'

let root: Root
let container: HTMLDivElement
let invoke: ReturnType<typeof vi.fn>
const originalEditorState = useEditorStore.getState()
const originalProjectState = useProjectStore.getState()
const originalWorkflowState = useWorkflowStore.getState()

;(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

beforeEach(async () => {
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  invoke = vi.fn(async (channel: string) => {
    if (channel === 'db_draft_get_meta') {
      return {
        id: 7,
        chapterNumber: 1,
        version: 1,
        status: 'draft',
        source: 'write',
        contentId: 70,
        wordCount: CURRENT.length,
        createdAt: '',
        updatedAt: '',
      }
    }
    if (channel === 'db_blueprint_get_all' || channel === 'db_review_list') return []
    if (channel === 'db_draft_list') return [{ id: 7, version: 1 }]
    if (channel === 'db_revision_get_pending') {
      return [{
        id: 31,
        baseDraftId: 7,
        revisionIndex: 1,
        revisionType: 'refine',
        status: 'pending',
        mergedToDraftId: null,
        userPrompt: '',
        reviewSourceId: null,
        sourceDraft: {
          id: 7,
          chapterNumber: 1,
          version: 1,
          status: 'draft',
          content: SOURCE,
        },
        contentId: 71,
        wordCount: REVISION.length,
        createdAt: '',
        updatedAt: '',
      }]
    }
    if (channel === 'db_revision_get_full') {
      return {
        id: 31,
        baseDraftId: 7,
        revisionIndex: 1,
        revisionType: 'refine',
        status: 'pending',
        mergedToDraftId: null,
        userPrompt: '',
        reviewSourceId: null,
        sourceDraft: {
          id: 7,
          chapterNumber: 1,
          version: 1,
          status: 'draft',
          content: SOURCE,
        },
        contentId: 71,
        wordCount: REVISION.length,
        createdAt: '',
        updatedAt: '',
        content: REVISION,
      }
    }
    if (channel === 'db_draft_get_full') return { id: 7, content: CURRENT }
    throw new Error(`Unexpected IPC channel: ${channel}`)
  })
  ;(window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = { invoke }
  useProjectStore.setState({
    currentProject: {
      id: PROJECT_SESSION.projectId,
      name: 'Revision source',
      path: PROJECT_PATH,
      sessionLease: PROJECT_SESSION.leaseId,
      novelConfig: { writingLanguage: 'zh-CN' },
    } as never,
  })
  setActiveProjectSessionContext(PROJECT_SESSION)
  useEditorStore.setState({
    tabs: [{
      id: 'draft-7',
      name: 'Chapter 1',
      type: 'chapter',
      filePath: 'ai-novel://draft/7',
      content: CURRENT,
      savedContent: CURRENT,
      dirty: false,
      draftId: 7,
      draftStatus: 'draft',
      chapterNumber: 1,
      projectKey: PROJECT_PATH,
      projectSessionLease: PROJECT_SESSION.leaseId,
      contentRevision: 0,
    }],
    activeTabId: 'draft-7',
  })
  useWorkflowStore.setState({ activeRuns: [] })

  await act(async () => root.render(
    <DraftEditor
      tabId="draft-7"
      filePath="ai-novel://draft/7"
      content={CURRENT}
      projectKey={PROJECT_PATH}
    />,
  ))
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
  setActiveProjectSessionContext(null)
  useEditorStore.getState().clearTabs()
  useEditorStore.setState(originalEditorState)
  useProjectStore.setState(originalProjectState)
  useWorkflowStore.setState(originalWorkflowState)
})

describe('DraftEditor revision source binding', () => {
  it('shows an old revision against its frozen source but refuses to merge it into a newer saved draft', async () => {
    await act(async () => {
      await vi.waitFor(() => expect(container.textContent).toContain('待合并(1)'))
      await page.getByRole('button', { name: '待合并(1)' }).click()
    })

    await expect.element(page.getByText('当前草稿已不是该修订稿的生成时源稿，修订仍可查看但不能合并。')).toBeVisible()
    const merge = document.querySelector('.three-way-merge')
    expect(merge?.textContent).toContain(SOURCE)
    expect(merge?.textContent).toContain(REVISION)

    await act(async () => page.getByRole('button', { name: '完成合并' }).click())
    expect(invoke.mock.calls.some(([channel]) => channel === 'db_revision_merge')).toBe(false)
  })

  it('saves each dirty draft through its own retained handler after switching the active tab', async () => {
    await act(async () => root.unmount())
    root = createRoot(container)

    const firstContent = '第一章后台修改。'
    const secondContent = '第二章当前修改。'
    const tabs = [
      {
        id: 'draft-7', name: 'Chapter 1', type: 'chapter' as const,
        filePath: 'ai-novel://draft/7', content: firstContent, savedContent: SOURCE,
        dirty: true, draftId: 7, draftStatus: 'draft' as const, chapterNumber: 1,
        projectKey: PROJECT_PATH, projectSessionLease: PROJECT_SESSION.leaseId, contentRevision: 1,
      },
      {
        id: 'draft-8', name: 'Chapter 2', type: 'chapter' as const,
        filePath: 'ai-novel://draft/8', content: secondContent, savedContent: '第二章旧稿。',
        dirty: true, draftId: 8, draftStatus: 'draft' as const, chapterNumber: 2,
        projectKey: PROJECT_PATH, projectSessionLease: PROJECT_SESSION.leaseId, contentRevision: 1,
      },
    ]
    useEditorStore.setState({ tabs, activeTabId: 'draft-7', draftLedgers: {} })
    invoke.mockImplementation(async (channel: string, ...args: unknown[]) => {
      if (channel === 'db_draft_get_meta') {
        const id = Number((args[0] as { id: number }).id)
        return {
          id,
          chapterNumber: id === 7 ? 1 : 2,
          version: 1,
          status: 'draft',
          source: 'write',
          contentId: id * 10,
          wordCount: id === 7 ? firstContent.length : secondContent.length,
          createdAt: '',
          updatedAt: '',
        }
      }
      if (channel === 'db_draft_list') return [{ id: 7, version: 1 }, { id: 8, version: 1 }]
      if (channel === 'db_blueprint_get_all' || channel === 'db_review_list' || channel === 'db_revision_get_pending') return []
      if (channel === 'db_draft_update_content') return { success: true }
      throw new Error(`Unexpected IPC channel: ${channel}`)
    })

    await act(async () => root.render(
      <DraftEditor key="draft-7" tabId="draft-7" filePath="ai-novel://draft/7" content={firstContent} projectKey={PROJECT_PATH} />,
    ))
    useEditorStore.getState().setActiveTab('draft-8')
    await act(async () => root.render(
      <DraftEditor key="draft-8" tabId="draft-8" filePath="ai-novel://draft/8" content={secondContent} projectKey={PROJECT_PATH} />,
    ))

    await act(async () => saveDirtyEditorChangesForExit(PROJECT_PATH))

    const writes = invoke.mock.calls.filter(([channel]) => channel === 'db_draft_update_content')
    expect(writes.map(([, named]) => [(named as { id: number }).id, (named as { content: string }).content])).toEqual([
      [7, firstContent],
      [8, secondContent],
    ])
    expect(useEditorStore.getState().activeTabId).toBe('draft-8')
    expect(useEditorStore.getState().tabs.every(tab => !tab.dirty)).toBe(true)
  })
})
