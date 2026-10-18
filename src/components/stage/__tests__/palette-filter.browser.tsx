import { afterEach, describe, expect, it } from 'vitest'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'

import StagePalette from '../StagePalette'
import { chapterQuery, filterPalette, paletteItem, type PaletteItem } from '../palette-filter'

;(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true

const noop = () => undefined
const ITEMS: PaletteItem[] = [
  paletteItem('stage', { id: 'stage-setup', label: '设定', run: noop }),
  paletteItem('stage', { id: 'stage-blueprint', label: '蓝图', run: noop }),
  paletteItem('chapter', { id: 'chapter-1', chapter: 1, label: '第 1 章 · 3 号站台', run: noop }),
  ...[3, 13, 30].map(chapter => paletteItem('chapter', { id: `chapter-${chapter}`, chapter, label: `第 ${chapter} 章 · 假设${chapter}`, run: noop })),
  paletteItem('reference', { id: 'characters', label: '角色', keywords: 'roster 名单', run: noop }),
  paletteItem('action', { id: 'settings', label: '设置', run: noop }),
]
const ids = (items: readonly PaletteItem[]) => items.map(item => item.id)

let root: Root | undefined
let host: HTMLDivElement | undefined

afterEach(async () => {
  await act(async () => root?.unmount())
  host?.remove()
  root = undefined
  host = undefined
})

describe('Ctrl K 与 Ctrl P 的匹配', () => {
  it('「3」「第3章」「第 3 章」「chapter 3」「Ch.3」都指第 3 章，其余写法不算章号', () => {
    expect(['3', '第3章', '第 3 章', 'chapter 3', 'Ch.3', 'ch 3'].map(chapterQuery)).toEqual([3, 3, 3, 3, 3, 3])
    expect(['第三章', '3a', '', '章', '第3节'].map(chapterQuery)).toEqual([null, null, null, null, null])
  })

  it('查找章节只在章节里找，章号完全相同的章排在只是标题或章号里含这个数字的章前面', () => {
    expect(ids(filterPalette(ITEMS, '3', 'chapters'))).toEqual(['chapter-3', 'chapter-1', 'chapter-13', 'chapter-30'])
    expect(ids(filterPalette(ITEMS, '第 30 章', 'chapters'))).toEqual(['chapter-30'])
    expect(ids(filterPalette(ITEMS, '', 'chapters'))).toEqual(['chapter-1', 'chapter-3', 'chapter-13', 'chapter-30'])
  })

  it('搜索按标签与关键词匹配，结果按分组显示顺序排列', () => {
    expect(ids(filterPalette(ITEMS, '设', 'commands'))).toEqual(['stage-setup', 'chapter-3', 'chapter-13', 'chapter-30', 'settings'])
    expect(ids(filterPalette(ITEMS, '名单', 'commands'))).toEqual(['characters'])
  })

  it('上下键按看到的顺序移动：设定之后是第 3 章，不是排在最后的设置', async () => {
    host = document.createElement('div')
    document.body.append(host)
    root = createRoot(host)
    await act(async () => root!.render(<StagePalette mode="commands" items={ITEMS} container={null} onClose={noop} />))
    const input = document.querySelector<HTMLInputElement>('[role="combobox"]')!
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, '设')
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await act(async () => { input.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true })) })
    expect(document.querySelector('[role="option"][aria-selected="true"]')?.textContent).toContain('第 3 章 · 假设3')
  })
})
