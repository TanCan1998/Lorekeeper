import { afterEach, describe, expect, it } from 'vitest'

import '../../../styles/paper-ink/index.css'

let host: HTMLDivElement | undefined

afterEach(() => {
  host?.remove()
  host = undefined
})

/** 本文件只引入纸墨样式，不引入旧外壳的 src/index.css，用来证明 RD-10 删掉旧样式后字体仍然可用。 */
describe('纸墨字体自洽', () => {
  it('不依赖旧外壳样式即可加载 Noto Serif SC 与 Instrument Serif', async () => {
    const serifFaces = [...document.styleSheets].flatMap((sheet) => [...sheet.cssRules])
      .filter((rule): rule is CSSFontFaceRule => rule instanceof CSSFontFaceRule && rule.style.getPropertyValue('font-family').includes('Noto Serif SC'))
    expect(serifFaces).toHaveLength(1)
    expect((serifFaces[0].parentStyleSheet!.ownerNode as HTMLElement).dataset.viteDevId).toMatch(/\/src\/styles\/paper-ink\//)

    const [serif] = await document.fonts.load('600 30px "Noto Serif SC"', '雨夜')
    const [number] = await document.fonts.load('italic 56px "Instrument Serif"', '3')
    expect(serif?.status).toBe('loaded')
    expect(number?.status).toBe('loaded')
  })

  it('界面字体按 Noto Sans SC、Microsoft YaHei UI、PingFang SC、system-ui 回退', () => {
    host = document.createElement('div')
    host.className = 'paper-ink'
    document.body.append(host)
    expect(getComputedStyle(host).fontFamily).toBe('"Noto Sans SC", "Microsoft YaHei UI", "PingFang SC", system-ui, sans-serif')
  })
})
