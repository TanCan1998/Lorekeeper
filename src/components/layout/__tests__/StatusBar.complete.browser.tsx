import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it } from 'vitest'

import StatusBar from '../StatusBar'
import { useLocaleStore } from '../../../stores/locale-store'
import { useWorkflowStore, type WorkflowRun } from '../../../stores/workflow-store'

;(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

let root: Root | undefined
let container: HTMLDivElement | undefined

afterEach(() => {
  act(() => root?.unmount())
  container?.remove()
  root = undefined
  container = undefined
  useWorkflowStore.setState({ activeRuns: [], history: [] })
})

async function renderCompleted(title: string, locale: 'zh-CN' | 'en-US') {
  useLocaleStore.setState({ locale })
  useWorkflowStore.setState({
    activeRuns: [],
    history: [{ id: `run-${title}`, title, status: 'completed' } as WorkflowRun],
  })
  container = document.createElement('div')
  document.body.appendChild(container)
  root = createRoot(container)
  await act(async () => root!.render(<StatusBar />))
  return container.querySelector('.ai-task-capsule--complete')?.textContent
}

describe('StatusBar 完成胶囊', () => {
  it.each([
    ['第 1 章 · 写稿', 'zh-CN', '第 1 章 · 写稿 完成'],
    ['批量草稿待审 · 第 5 至 8 章', 'zh-CN', '批量草稿待审 · 第 5 至 8 章 完成'],
    ['Chapter 1 · Draft', 'en-US', 'Chapter 1 · Draft complete'],
  ] as const)('显示完整运行标题：%s', async (title, locale, expected) => {
    expect(await renderCompleted(title, locale)).toBe(expected)
  })
})
