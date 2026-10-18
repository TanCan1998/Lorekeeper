import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { StoreApi } from 'zustand'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'

import { useEditorStore } from '../../../stores/editor-store'
import { useWorkflowStore } from '../../../stores/workflow-store'
import { LibraryUpdateCard } from '../StageUpdate'

;(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

type Api = { invoke: (channel: string, ...args: unknown[]) => Promise<unknown>; on: () => () => void }
const win = window as unknown as { aiNovelAPI?: Api }

function snapshot<T>(store: StoreApi<T>): () => void {
  const state = store.getState()
  return () => store.setState(state, true)
}
const restores = [snapshot(useEditorStore), snapshot(useWorkflowStore)]
const DOWNLOADED = { status: 'downloaded', currentVersion: '1.2.2', availableVersion: '1.2.3', updateAction: 'download', isReminderDeferred: false }

let root: Root | undefined
let host: HTMLDivElement | undefined
let invoke: ReturnType<typeof vi.fn<(channel: string, ...args: unknown[]) => Promise<unknown>>>

async function mount() {
  host = document.createElement('div')
  host.className = 'paper-ink stage-shell'
  document.body.append(host)
  root = createRoot(host)
  await act(async () => root!.render(<LibraryUpdateCard />))
}

const button = (name: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find(candidate => candidate.textContent === name)
const dialogTitle = () => document.querySelector('[role="dialog"] h2')?.textContent ?? null

async function click(element: HTMLElement | undefined) {
  expect(element, '要点击的元素存在').toBeTruthy()
  await act(async () => element!.click())
}

beforeEach(() => {
  invoke = vi.fn(async (channel: string) => {
    if (channel === 'update:get-state') return DOWNLOADED
    if (channel === 'update:quit-and-install') return { success: true, state: DOWNLOADED }
    return undefined
  })
  win.aiNovelAPI = { invoke, on: () => () => undefined }
  useEditorStore.setState({ tabs: [], draftLedgers: {} } as never)
  useWorkflowStore.setState({ activeRuns: [] })
})

afterEach(async () => {
  await act(async () => root?.unmount())
  host?.remove()
  root = undefined
  host = undefined
  delete win.aiNovelAPI
  for (const restore of restores) restore()
})

describe('书房的更新提示', () => {
  it('有未保存修改时拦下重启，「返回保存」不安装', async () => {
    useEditorStore.setState({ tabs: [{ id: 'config', name: '设定', type: 'config', projectKey: 'p', dirty: true }] } as never)
    await mount()
    await click(button('立即重启更新'))
    expect(dialogTitle()).toBe('请先处理未保存的修改')
    await click(button('返回保存'))
    expect(dialogTitle()).toBeNull()
    expect(invoke).not.toHaveBeenCalledWith('update:quit-and-install')
  })

  it('有创作任务时只能返回处理任务', async () => {
    useWorkflowStore.setState({ activeRuns: [{ id: 'run' }] as never })
    await mount()
    await click(button('立即重启更新'))
    expect(dialogTitle()).toBe('创作任务尚未结束，暂不能更新')
    await click(button('知道了，返回处理任务'))
    expect(invoke).not.toHaveBeenCalledWith('update:quit-and-install')
  })

  it('没有未保存修改与任务时直接重启安装；主操作不用强调色按钮', async () => {
    await mount()
    expect(host!.querySelector('.pi-btn--primary')).toBeNull()
    await click(button('立即重启更新'))
    expect(invoke).toHaveBeenCalledWith('update:quit-and-install')
  })
})
