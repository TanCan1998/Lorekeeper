import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import type { StoreApi } from 'zustand'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import App from '../App'
import type { Locale } from '../i18n/types'
import type { ProjectData } from '../shared/ipc-channels'
import { setActiveProjectSessionContext } from '../shared/project-session-context'
import { useAppearanceStore } from '../stores/appearance-bootstrap'
import { useLayoutStore } from '../stores/layout-store'
import { useLocaleStore } from '../stores/locale-store'
import { useProjectStore } from '../stores/project-store'
import { useWorkflowStore } from '../stores/workflow-store'

;(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const CHAT_WORDS = /Agent|助手|聊天|对话|assistant|chat/i
const session = { projectId: '灯塔', projectPath: 'C:/novels/lighthouse', leaseId: '本次打开' }
const project = { id: session.projectId, name: '灯塔', path: session.projectPath, sessionLease: session.leaseId, novelConfig: {} } as unknown as ProjectData

const restores = [useAppearanceStore, useLayoutStore, useLocaleStore, useProjectStore, useWorkflowStore].map(store => {
  const state = (store as StoreApi<unknown>).getState()
  return () => (store as StoreApi<unknown>).setState(state, true)
})

let configLocale: Locale
let host: HTMLDivElement
let root: Root
let invoke: ReturnType<typeof vi.fn<(channel: string, ...args: unknown[]) => Promise<unknown>>>

/** 主进程的回复按渠道给出最小的合法形状；列表类给空列表。 */
function reply(channel: string): unknown {
  if (channel === 'config:get') return { locale: configLocale }
  if (channel === 'project:smoke-open-request') return null
  if (/list|history|models/i.test(channel)) return []
  if (channel === 'fs:check-exists') return false
  return { success: true }
}

beforeEach(() => {
  configLocale = 'zh-CN'
  invoke = vi.fn(async (channel: string) => reply(channel))
  Object.defineProperty(window, 'aiNovelAPI', { configurable: true, value: { invoke, on: () => () => {}, once: () => {}, send: () => {} } })
  useAppearanceStore.setState({ resolvedShell: 'writer' })
  useProjectStore.setState({ currentProject: project })
  setActiveProjectSessionContext(session)
  useLayoutStore.setState({ sidebarView: 'project', activeRailItem: 'project', aiPanelOpen: true, immersive: false })
  useWorkflowStore.setState({ activeRuns: [], history: [] })
  host = document.createElement('div')
  host.style.cssText = 'width: 1440px; height: 900px; display: flex'
  document.body.append(host)
  root = createRoot(host)
})

afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  setActiveProjectSessionContext(null)
  for (const restore of restores) restore()
  vi.restoreAllMocks()
  delete window.aiNovelAPI
})

describe('应用里没有 AI 聊天', () => {
  it.each<[Locale, string]>([['zh-CN', 'AI 输出'], ['en-US', 'AI output']])('%s：旧外壳右侧面板是 AI 输出，不是聊天', async (locale, outputLabel) => {
    configLocale = locale
    useLocaleStore.setState({ locale })
    await act(async () => root.render(<App />))
    const panel = await vi.waitFor(() => {
      const region = host.querySelector<HTMLElement>(`aside[aria-label="${outputLabel}"]`)
      expect(region?.querySelector(`button[title="${locale === 'en-US' ? 'Close AI output' : '收起 AI 输出'}"]`)).toBeTruthy()
      return region!
    })
    expect(panel.textContent).toContain(outputLabel)
    expect(panel.textContent).not.toMatch(CHAT_WORDS)
    expect([...panel.querySelectorAll('[aria-label],[title],[placeholder]')]
      .map(node => ['aria-label', 'title', 'placeholder'].map(name => node.getAttribute(name) ?? '').join(' ')).join(' ')).not.toMatch(CHAT_WORDS)
  })

  it('启动时不连接 MCP 服务器', async () => {
    await act(async () => root.render(<App />))
    await vi.waitFor(() => expect(invoke.mock.calls.map(([channel]) => channel)).toContain('project:recent-list'))
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 50)) })
    expect(invoke.mock.calls.map(([channel]) => channel).filter(channel => channel.startsWith('mcp:'))).toEqual([])
  })
})
