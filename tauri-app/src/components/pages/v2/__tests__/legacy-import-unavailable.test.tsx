import { readFileSync } from 'node:fs'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { useLocaleStore } from '../../../../stores/locale-store'
import WelcomePageV2 from '../WelcomePageV2'

const originalLocale = useLocaleStore.getState().locale
afterEach(() => useLocaleStore.setState({ locale: originalLocale }))

describe('首版旧库入口不可用', () => {
  it.each([
    ['zh-CN', '导入旧项目（首版不支持）', '不会读取或修改旧项目'],
    ['en-US', 'Legacy import (not supported in this release)', 'does not import legacy databases'],
  ] as const)('%s显示禁用入口与明确说明', (locale, label, notice) => {
    useLocaleStore.setState({ locale })
    const callback = vi.fn()
    const html = renderToStaticMarkup(<WelcomePageV2 overview={{ state: 'empty' }} recentProjects={[]}
      onNewProject={callback} onOpenProject={callback} onImportNovel={callback} />)
    expect(html).toContain(label)
    expect(html).toContain(notice)
    expect(html).toContain('disabled="" aria-describedby="legacy-import-unavailable"')
    expect(html).toContain('id="legacy-import-unavailable" role="note"')
    expect(callback).not.toHaveBeenCalled()
  })

  it('欢迎页不再发起旧库选择、目标授权或转换调用', () => {
    const source = readFileSync(new URL('../use-project-overview.tsx', import.meta.url), 'utf8')
    for (const channel of ['dialog:select-legacy-project', 'dialog:select-project-restore-target', 'project:import-legacy-copy']) {
      expect(source).not.toContain(channel)
    }
  })
})
