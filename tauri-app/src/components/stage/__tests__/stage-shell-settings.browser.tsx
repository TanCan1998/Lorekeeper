import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { StoreApi } from 'zustand'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'

import '../../../index.css'
import '../../../styles/paper-ink/index.css'
import '../stage.css'
import { setActiveProjectSessionContext } from '../../../shared/project-session-context'
import { useEditorStore } from '../../../stores/editor-store'
import { useLayoutStore } from '../../../stores/layout-store'
import { useLLMStore } from '../../../stores/llm-store'
import { useLocaleStore } from '../../../stores/locale-store'
import { useStageStore } from '../../../stores/navigation/stage-store'
import { useProjectStore } from '../../../stores/project-store'
import { toast } from '../../ui/Toast'
import StageShell from '../StageShell'

;(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

function snapshot<T>(store: StoreApi<T>): () => void {
  const state = store.getState()
  return () => store.setState(state, true)
}
const restores = [useEditorStore, useLayoutStore, useLLMStore, useLocaleStore, useStageStore, useProjectStore]
  .map(store => snapshot(store as StoreApi<unknown>))

const TARGET = 'C:\\novels\\harbor-copy'

let root: Root | undefined
let host: HTMLDivElement | undefined
let invoke: ReturnType<typeof vi.fn<(channel: string, ...args: unknown[]) => Promise<unknown>>>

const discardDialog = () => [...document.querySelectorAll('[role="dialog"]')].find(dialog => dialog.querySelector('h2')?.textContent === '放弃未保存的修改？')
const dialogButton = (name: string) => [...discardDialog()!.querySelectorAll('button')].find(button => button.textContent === name)
const skillUrl = () => host!.querySelector<HTMLInputElement>('input[aria-label="GitHub Skill 地址"]')!
const opens = () => invoke.mock.calls.filter(([channel]) => channel === 'project:open')
const paletteOption = (name: string) => [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(option => option.textContent?.includes(name))!

beforeEach(async () => {
  invoke = vi.fn(async (channel: string) => {
    if (channel === 'config:get') return {}
    if (channel === 'dialog:select-legacy-project') return { grantId: 'legacy-grant', displayName: '旧港' }
    if (channel === 'dialog:select-project-restore-target') return { grantId: 'target-grant' }
    if (channel === 'project:import-legacy-copy') return { state: 'ready', projectId: 'harbor', targetRoot: TARGET }
    if (channel === 'project:open') return { success: false, error: '测试里不真的打开' }
    return channel.endsWith('-list') || channel.endsWith('get-all') ? [] : undefined
  })
  Object.defineProperty(window, 'aiNovelAPI', { configurable: true, value: { invoke, on: () => () => undefined } })
  useLocaleStore.setState({ locale: 'zh-CN' })
  useLLMStore.setState({ loaded: true, models: [], defaultModelId: null })
  useEditorStore.setState({ tabs: [], activeTabId: null })
  useStageStore.setState({ sheet: null, focus: false, palette: null, work: null })
  useProjectStore.setState({ currentProject: null, recentProjects: [], legacyImportNotice: '' })
  useLayoutStore.setState({ sidebarView: 'project', settingsOpen: true, settingsSection: 'skills' })
  host = document.createElement('div')
  host.style.cssText = 'width: 1280px; height: 800px; display: flex;'
  document.body.append(host)
  root = createRoot(host)
  await act(async () => root!.render(<StageShell theme="light" />))
  await vi.waitFor(() => expect(skillUrl()).toBeTruthy())
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(skillUrl(), 'https://github.com/owner/skill')
    skillUrl().dispatchEvent(new Event('input', { bubbles: true }))
  })
})

afterEach(async () => {
  await act(async () => root?.unmount())
  host?.remove()
  Reflect.deleteProperty(window, 'aiNovelAPI')
  setActiveProjectSessionContext(null)
  for (const restore of restores) restore()
  vi.restoreAllMocks()
})

describe('书房里的设置页有没保存的输入时，导入旧项目副本完成后要自动打开作品', () => {
  it('首版拒绝旧库导入，设置页与未保存输入保留，不弹文件选择器', async () => {
    let importing!: Promise<boolean>
    await act(async () => { importing = useProjectStore.getState().importLegacyProject() })
    await expect(importing).resolves.toBe(false)
    expect(discardDialog()).toBeUndefined()
    expect(opens()).toEqual([])
    expect(invoke.mock.calls.some(([channel]) => channel.startsWith('dialog:'))).toBe(false)
    expect(useLayoutStore.getState().settingsOpen).toBe(true)
    expect(skillUrl().value).toBe('https://github.com/owner/skill')
    expect(useProjectStore.getState().legacyImportNotice).toBe('首版不支持旧库导入，不会读取或修改旧项目。')
  })

  it('命令面板「打开作品」先问放弃，再弹选文件夹；继续编辑就不弹', async () => {
    await act(async () => useStageStore.setState({ palette: 'commands' }))
    const option = await vi.waitFor(() => {
      const found = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(candidate => candidate.textContent?.includes('打开作品'))
      expect(found).toBeTruthy()
      return found!
    })
    await act(async () => option.click())
    expect(discardDialog()).toBeTruthy()
    expect(invoke.mock.calls.filter(([channel]) => channel === 'dialog:select-folder')).toEqual([])
    await act(async () => dialogButton('继续编辑')!.click())
    expect(invoke.mock.calls.filter(([channel]) => channel === 'dialog:select-folder')).toEqual([])
    expect(skillUrl().value).toBe('https://github.com/owner/skill')
  })

  describe('从命令面板「打开作品」，选文件夹时取消', () => {
    const folderDialogs = () => invoke.mock.calls.filter(([channel]) => channel === 'dialog:select-folder')
    async function openFromPalette() {
      await act(async () => useStageStore.setState({ palette: 'commands' }))
      await act(async () => paletteOption('打开作品').click())
    }

    it('没有没保存的输入：设置页留着', async () => {
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(skillUrl(), '')
        skillUrl().dispatchEvent(new Event('input', { bubbles: true }))
      })
      await openFromPalette()
      await vi.waitFor(() => expect(folderDialogs()).toHaveLength(1))
      expect(discardDialog()).toBeUndefined()
      expect(useLayoutStore.getState().settingsOpen).toBe(true)
      expect(skillUrl()).toBeTruthy()
    })

    it('选了放弃修改再取消选文件夹：设置页与输入都留着', async () => {
      await openFromPalette()
      await act(async () => dialogButton('放弃修改')!.click())
      await vi.waitFor(() => expect(folderDialogs()).toHaveLength(1))
      expect(useLayoutStore.getState().settingsOpen).toBe(true)
      expect(skillUrl().value).toBe('https://github.com/owner/skill')
      expect(opens()).toEqual([])
    })

    it('选了放弃修改并选到文件夹：收起设置页，打开选到的作品，只问一次', async () => {
      const plain = invoke.getMockImplementation()!
      invoke.mockImplementation(async (channel: string, ...args: unknown[]) => channel === 'dialog:select-folder' ? TARGET : plain(channel, ...args))
      await openFromPalette()
      await act(async () => dialogButton('放弃修改')!.click())
      await vi.waitFor(() => expect(opens().map(([, path]) => path)).toEqual([TARGET]))
      expect(useLayoutStore.getState().settingsOpen).toBe(false)
      expect(discardDialog()).toBeUndefined()
    })
  })

  it('拒绝旧库导入后仍可处理新建请求，继续编辑不丢设置输入', async () => {
    expect(await useProjectStore.getState().importLegacyProject()).toBe(false)
    await act(async () => useStageStore.setState({ palette: 'commands' }))
    await act(async () => paletteOption('新建作品').click())
    await vi.waitFor(() => expect(discardDialog()).toBeTruthy())
    await act(async () => dialogButton('继续编辑')!.click())
    expect(useProjectStore.getState().legacyImportBusy).toBe(false)
    expect(useLayoutStore.getState().newProjectOpen).toBe(false)
    expect(skillUrl().value).toBe('https://github.com/owner/skill')
  })

  it('重复请求旧库导入始终失败关闭，不打开任何作品或收起设置', async () => {
    for (let count = 0; count < 2; count++) {
      expect(await useProjectStore.getState().importLegacyProject()).toBe(false)
    }
    expect(opens()).toEqual([])
    expect(useProjectStore.getState().legacyImportBusy).toBe(false)
    expect(useLayoutStore.getState().settingsOpen).toBe(true)
  })
})

describe('放弃确认开着时又来了离开请求', () => {
  const toastTitles = () => [...document.querySelectorAll('.pi-toast-title')].map(node => node.textContent)

  it('作者又点了新建作品：提示先处理开着的确认，原来的确认还在', async () => {
    await act(async () => useStageStore.setState({ palette: 'commands' }))
    await act(async () => paletteOption('打开作品').click())
    expect(discardDialog()).toBeTruthy()
    await act(async () => useStageStore.setState({ palette: 'commands' }))
    await act(async () => paletteOption('新建作品').click())
    await vi.waitFor(() => expect(toastTitles()).toContain('请先处理「放弃未保存的修改？」'))
    expect(discardDialog()).toBeTruthy()
    expect(useLayoutStore.getState().newProjectOpen).toBe(false)
  })

  it('确认开着时旧库导入仍明确不支持，不替换或吞掉既有确认', async () => {
    await act(async () => useStageStore.setState({ palette: 'commands' }))
    await act(async () => paletteOption('打开作品').click())
    expect(discardDialog()).toBeTruthy()
    const info = vi.spyOn(toast, 'info')
    let importing!: Promise<boolean>
    await act(async () => { importing = useProjectStore.getState().importLegacyProject() })
    await expect(importing).resolves.toBe(false)
    expect(opens()).toEqual([])
    expect(useProjectStore.getState().legacyImportNotice)
      .toBe('首版不支持旧库导入，不会读取或修改旧项目。')
    expect(info).not.toHaveBeenCalled()
    expect(discardDialog()).toBeTruthy()
  })
})
