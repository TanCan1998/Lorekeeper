/* eslint-disable react-refresh/only-export-components */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import SettingsModal from '../SettingsModal'
import { useLayoutStore } from '../../../stores/layout-store'
import { useLocaleStore } from '../../../stores/locale-store'

;(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const originalLayout = useLayoutStore.getState()
const originalLocale = useLocaleStore.getState()

let container: HTMLDivElement
let root: Root

function SettingsHarness() {
  const open = useLayoutStore(state => state.settingsOpen)
  const close = useLayoutStore(state => state.closeSettings)
  return <SettingsModal open={open} onClose={close} />
}

const heading = () => container.querySelector('h2')?.textContent

beforeEach(() => {
  Object.defineProperty(window, 'aiNovelAPI', {
    configurable: true,
    value: {
      invoke: vi.fn(async (channel: string) => channel === 'llm:list-models' ? [] : { success: true }),
      on: vi.fn(() => () => {}),
      once: vi.fn(),
      send: vi.fn(),
      setZoomLevel: vi.fn(),
      setZoomFactor: vi.fn(),
      getZoomLevel: vi.fn(() => 0),
    },
  })
  useLocaleStore.setState({ locale: 'zh-CN' })
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
  useLayoutStore.setState(originalLayout, true)
  useLocaleStore.setState(originalLocale, true)
  Reflect.deleteProperty(window, 'aiNovelAPI')
})

describe('设置弹窗的分区', () => {
  it('切换外壳会重挂全局对话框，重挂后仍停在作者点开的分区', async () => {
    useLayoutStore.setState({ settingsOpen: true, settingsSection: 'llm' })
    await act(async () => root.render(<SettingsHarness key="workbench" />))
    await vi.waitFor(() => expect(heading()).toBe('AI 生成模型'))
    await act(async () => [...container.querySelectorAll('button')].find(button => button.textContent === '外观')!.click())
    await vi.waitFor(() => expect(heading()).toBe('外观'))

    await act(async () => root.render(<SettingsHarness key="stage" />))
    await vi.waitFor(() => expect(heading()).toBe('外观'))
    expect(container.querySelector('[role="dialog"]')?.getAttribute('aria-modal')).toBe('true')
  })
})
