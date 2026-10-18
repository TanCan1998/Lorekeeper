import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it } from 'vitest'
import { useLocaleStore } from '../../../stores/locale-store'
import ThreeWayMerge from '../ThreeWayMerge'

;(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true
const initialLocale = useLocaleStore.getState()
let root: Root
let container: HTMLDivElement
beforeEach(() => {
  useLocaleStore.setState({ locale: 'en-US', initialized: true })
  container = document.createElement('div'); document.body.append(container); root = createRoot(container)
})
afterEach(async () => {
  await act(async () => root.unmount()); container.remove()
  useLocaleStore.setState(initialLocale)
})

it.each([
  ['甲\n乙\n丙', '甲\n改\n丙', '0/1 change applied'],
  ['灯塔熄灭的那一夜。\n\n林岚走上石阶。\n\n钟声响了三下。\n\n潮水退去。', '灯塔熄灭的那一夜。\n\n林岚独自走上湿滑的石阶。\n\n钟声响了三下。\n\n潮水退得很快。', '0/2 changes applied'],
])('英文界面的合并进度按处数写单复数：%#', async (original, revised, progress) => {
  await act(async () => root.render(<ThreeWayMerge originalContent={original} modifiedContent={revised} onComplete={() => {}} />))
  expect(container.querySelector('.twm-toolbar-progress')?.textContent).toBe(progress)
})
