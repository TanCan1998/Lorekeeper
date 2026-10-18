import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { useThemeStore } from '../../../stores/theme-store'
import { AppIcon } from '../AppIcon'

;(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const initialTheme = useThemeStore.getState().resolvedTheme
let host: HTMLDivElement
let root: Root

beforeEach(() => {
  useThemeStore.setState({ resolvedTheme: 'light' })
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})

afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  useThemeStore.setState({ resolvedTheme: initialTheme })
})

describe('单星图标浏览器渲染', () => {
  it.each([22, 52])('在%s像素表面实际载入亮色SVG，不遮挡父级拖动区域', async size => {
    await act(async () => root.render(<span data-tauri-drag-region><AppIcon size={size} /></span>))
    const icon = host.querySelector('img')!
    await icon.decode()
    expect(icon.getAttribute('src')).toBe('/branding/app-icon-light.svg')
    expect(icon.naturalWidth).toBe(1024)
    expect(icon.getBoundingClientRect().width).toBe(size)
    expect(icon.getBoundingClientRect().height).toBe(size)
    expect(icon.draggable).toBe(false)
    expect(icon.alt).toBe('')
    expect(icon.getAttribute('aria-hidden')).toBe('true')
    expect(getComputedStyle(icon).pointerEvents).toBe('none')
    expect(icon.parentElement!.hasAttribute('data-tauri-drag-region')).toBe(true)
  })

  it('同一组件随应用主题切换亮暗资源，不依赖操作系统主题', async () => {
    await act(async () => root.render(<AppIcon size={52} />))
    const icon = host.querySelector('img')!
    for (const theme of ['dark', 'light'] as const) {
      await act(async () => useThemeStore.setState({ resolvedTheme: theme }))
      await icon.decode()
      expect(icon.getAttribute('src')).toBe(`/branding/app-icon-${theme}.svg`)
      expect(icon.naturalWidth).toBe(1024)
      expect(icon.getBoundingClientRect().width).toBe(52)
    }
  })
})
