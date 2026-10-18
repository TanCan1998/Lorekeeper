import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useLayoutStore } from '../../../stores/layout-store'
import { useLocaleStore } from '../../../stores/locale-store'

import {
  installTauriInternals,
  type TauriInternalsHandle,
} from '../../../../test/helpers/tauri-internals'
;(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

let container: HTMLDivElement
let root: Root

let tauriInternals: TauriInternalsHandle
beforeEach(() => {
  useLocaleStore.setState({ locale: 'zh-CN', initialized: true })
  tauriInternals = installTauriInternals({
    commands: {
      update_get_state: {
        status: 'available',
        currentVersion: '0.9.2',
        availableVersion: '9.8.7',
        updateAction: 'download',
        isReminderDeferred: false,
      },
    },
  })
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  tauriInternals.uninstall()
  vi.restoreAllMocks()
})

describe('UpdateNotifier locale changes', () => {
  it('replaces an active update notice in the newly selected locale', async () => {
    const dismiss = vi.fn()
    const { actionToast } = await import('../../ui/ActionToast')
    const show = vi.spyOn(actionToast, 'show').mockReturnValue(dismiss)
    const { UpdateNotifier } = await import('../UpdateNotifier')
    await act(async () => root.render(<UpdateNotifier />))
    await vi.waitFor(() => expect(show).toHaveBeenCalledOnce())
    expect(show).toHaveBeenCalledWith(expect.objectContaining({
      message: '发现新版本 v9.8.7',
      actions: [expect.objectContaining({ label: '查看更新' })],
    }))

    await act(async () => {
      useLocaleStore.setState(state => ({
        locale: 'en-US',
        text: (_zhCNText, enUSText) => enUSText,
        t: state.t,
      }))
    })

    await vi.waitFor(() => expect(show).toHaveBeenCalledTimes(2))
    expect(dismiss).toHaveBeenCalledOnce()
    expect(show).toHaveBeenLastCalledWith(expect.objectContaining({
      message: 'New version v9.8.7 is available',
      actions: [expect.objectContaining({ label: 'View update' })],
    }))
  })
})

describe('UpdateNotifier 查看更新', () => {
  async function viewUpdate(version: string) {
    tauriInternals.invoke.mockImplementation(async (channel: string) => channel === 'update_get_state'
      ? { status: 'available', currentVersion: '0.9.2', availableVersion: version, updateAction: 'download', isReminderDeferred: false }
      : null)
    const { actionToast } = await import('../../ui/ActionToast')
    const show = vi.spyOn(actionToast, 'show').mockReturnValue(vi.fn())
    const { UpdateNotifier } = await import('../UpdateNotifier')
    await act(async () => root.render(<UpdateNotifier />))
    await vi.waitFor(() => expect(show).toHaveBeenCalled())
    const [{ actions }] = show.mock.lastCall!
    act(() => actions?.[0]?.onClick?.())
  }

  it.each([
    { where: '书房里', sidebarView: 'home', version: '9.9.0' },
    { where: '作品里', sidebarView: 'project', version: '9.9.1' },
  ] as const)('$where开着设置页时收起设置页，回到书房露出更新卡', async ({ sidebarView, version }) => {
    const restore = useLayoutStore.getState()
    useLayoutStore.setState({ sidebarView, settingsOpen: true })
    await viewUpdate(version)
    expect(useLayoutStore.getState()).toMatchObject({ sidebarView: 'home', settingsOpen: false })
    useLayoutStore.setState(restore, true)
  })
})

describe('UpdateNotifier 每个版本只提醒一次', () => {
  async function renderNotifier(version: string) {
    const available = { status: 'available', currentVersion: '0.9.2', availableVersion: version, updateAction: 'download', isReminderDeferred: false }
    const productionInvoke = tauriInternals.invoke.getMockImplementation()!
    tauriInternals.invoke.mockImplementation(async (channel: string, args: Record<string, unknown> = {}) => channel === 'update_get_state' ? available : productionInvoke(channel, args))
    const dismiss = vi.fn()
    const { actionToast } = await import('../../ui/ActionToast')
    const show = vi.spyOn(actionToast, 'show').mockReturnValue(dismiss)
    const { UpdateNotifier } = await import('../UpdateNotifier')
    await act(async () => root.render(<UpdateNotifier />))
    await vi.waitFor(() => expect(show).toHaveBeenCalledOnce())
    return { show, dismiss, emit: (patch: object) => act(async () => tauriInternals.emit('update:state', { ...available, ...patch })) }
  }

  it('同一版本从可下载变成已下载，仍只有一条常驻提醒', async () => {
    const { show, dismiss, emit } = await renderNotifier('9.9.2')
    expect(show).toHaveBeenCalledWith(expect.objectContaining({ duration: 0 }))
    await emit({ status: 'downloaded' })
    expect(show).toHaveBeenCalledOnce()
    expect(dismiss).not.toHaveBeenCalled()
  })

  it('延后提醒时收起提醒；不再延后后同一版本再提醒一次', async () => {
    const { show, dismiss, emit } = await renderNotifier('9.9.3')
    await emit({ status: 'downloaded', isReminderDeferred: true })
    expect(dismiss).toHaveBeenCalledOnce()
    await emit({ status: 'downloaded', isReminderDeferred: false })
    expect(show).toHaveBeenCalledTimes(2)
  })
})
