import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import type { StoreApi } from 'zustand'

import type { ProjectData } from '../../../shared/ipc-channels'
import { useProjectStore } from '../../../stores/project-store'
import { useWorkflowStore, type WorkflowRun } from '../../../stores/workflow-store'
import RunCapsule from '../RunCapsule'

;(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const HERE = 'C:\\novels\\here'
const ELSEWHERE = 'C:\\novels\\elsewhere'
const THERE = 'C:\\novels\\there'

function snapshot<T>(store: StoreApi<T>): () => void {
  const state = store.getState()
  return () => store.setState(state, true)
}
const restores = [snapshot(useProjectStore), snapshot(useWorkflowStore)]

let root: Root | undefined
let host: HTMLDivElement | undefined

function run(id: string, status: WorkflowRun['status'], projectPath = HERE): WorkflowRun {
  return {
    id, projectPath, status, title: `运行 ${id}`, currentStepIndex: 0,
    steps: [{ id: 'write', name: '写草稿', status: status === 'running' ? 'running' : 'completed', progress: 40, logs: [] }],
    ...(status === 'running' ? {} : { completedAt: new Date().toISOString() }),
  } as unknown as WorkflowRun
}

const capsule = () => host!.querySelector('.stage-capsule')?.textContent ?? null

async function set(state: Partial<{ activeRuns: WorkflowRun[]; history: WorkflowRun[] }>) {
  await act(async () => useWorkflowStore.setState(state))
}

beforeEach(async () => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] })
  useProjectStore.setState({ currentProject: { id: 'here', sessionLease: 'l', name: '这里', path: HERE, novelConfig: {} } as unknown as ProjectData })
  useWorkflowStore.setState({ activeRuns: [], history: [] })
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () => root!.render(<RunCapsule />))
})

afterEach(async () => {
  await act(async () => root?.unmount())
  host?.remove()
  vi.useRealTimers()
  for (const restore of restores) restore()
})

describe('运行胶囊', () => {
  it('完成后显示 1.8 秒「运行已完成」再收起', async () => {
    await set({ activeRuns: [run('a', 'running')] })
    expect(capsule()).toContain('运行 a')
    await set({ activeRuns: [], history: [run('a', 'completed')] })
    expect(capsule()).toBe('运行已完成')
    await act(async () => vi.advanceTimersByTime(1900))
    expect(capsule()).toBeNull()
  })

  it('完成后 1.8 秒内又开始一次运行、这次失败了：不显示「运行已完成」，也不会一直卡着', async () => {
    await set({ activeRuns: [], history: [run('a', 'completed')] })
    expect(capsule()).toBe('运行已完成')
    await act(async () => vi.advanceTimersByTime(500))
    await set({ activeRuns: [run('b', 'running')] })
    expect(capsule()).toContain('运行 b')
    await set({ activeRuns: [], history: [run('b', 'failed'), run('a', 'completed')] })
    expect(capsule()).toBeNull()
    await act(async () => vi.advanceTimersByTime(5000))
    expect(capsule()).toBeNull()
  })

  it('历史里出现一条结束已超过 1.8 秒的完成运行时不显示完成提示', async () => {
    await act(async () => vi.advanceTimersByTime(10_000))
    const late = { ...run('z', 'completed'), completedAt: new Date(Date.now() - 5000).toISOString() } as WorkflowRun
    await set({ activeRuns: [], history: [late] })
    await act(async () => vi.advanceTimersByTime(60_000))
    expect(capsule()).toBeNull()
  })

  it('别的作品的运行完成不显示在这部作品的胶囊上，换作品时也不会把别处的完成带过来', async () => {
    await set({ activeRuns: [run('a', 'running'), run('x', 'running', ELSEWHERE)] })
    await set({ activeRuns: [run('a', 'running')], history: [run('x', 'completed', ELSEWHERE)] })
    expect(capsule()).toContain('运行 a')
    await act(async () => useProjectStore.setState({ currentProject: { id: 'there', sessionLease: 'l', name: '那里', path: THERE, novelConfig: {} } as unknown as ProjectData }))
    await act(async () => vi.advanceTimersByTime(10))
    expect(capsule()).toBeNull()
  })
})
