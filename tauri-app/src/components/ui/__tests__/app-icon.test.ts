import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const read = (path: string) => readFileSync(resolve(process.cwd(), path))
describe('定稿单星图标', () => {
  it.each([
    ['light', 'b68d5faec977bd49ea58ffa0fd8971d84be6af4945b0023886d0c64d05d06289'],
    ['dark', 'd5f90f2099cd95d22762bbf41af10bcd7b95845439585bd3f3e3985282ccb5d3'],
  ])('保留用户确认的%s源图，不混入候选或远程资源', (theme, hash) => {
    const bytes = read(`public/branding/app-icon-${theme}.svg`)
    const svg = bytes.toString('utf8')
    expect(createHash('sha256').update(bytes).digest('hex')).toBe(hash)
    const mask = svg.match(/<mask\b[^>]*id="spark-shape"[^>]*>([\s\S]*?)<\/mask>/)?.[1]
    expect(mask).toBeDefined()
    expect(mask?.match(/<path\b/g)).toHaveLength(1)
    expect(svg).not.toMatch(/<script\b|<image\b|<foreignObject\b|(?:href|src)=/i)
  })

  it('让启动页、顶栏与关于页采用同一资源，并保留窗口拖动区域', () => {
    const html = read('index.html').toString('utf8')
    expect(html).toContain('<link rel="icon" type="image/svg+xml" href="/branding/app-icon-light.svg"')
    expect(html).toContain('src="/branding/app-icon-light.svg"')
    expect(html).toContain('src="/branding/app-icon-dark.svg"')
    expect(html).toContain('.dark .ai-writer-brand-light')
    expect(html).toContain('.dark .ai-writer-brand-dark')
    for (const path of ['src/components/stage/StageTopBar.tsx', 'src/components/stage/settings/AboutSection.tsx']) {
      const source = read(path).toString('utf8')
      expect(source).toContain('<AppIcon size={')
      expect(source).not.toContain('BRAND_MARK')
    }
    const topbar = read('src/components/stage/StageTopBar.tsx').toString('utf8')
    expect(topbar).toContain('className="stage-brand" data-tauri-drag-region')
    expect(read('src/components/stage/styles/stage-shell.css').toString('utf8'))
      .toMatch(/\.stage-app-icon\s*\{[^}]*pointer-events: none;/)
  })

  it.each([
    ['32x32.png', 32], ['64x64.png', 64], ['128x128.png', 128],
    ['128x128@2x.png', 256], ['icon.png', 512],
  ])('提供有效的桌面PNG资源%s', (name, size) => {
    const bytes = read(`src-tauri/icons/${name}`)
    expect(bytes.subarray(0, 8).toString('hex')).toBe('89504e470d0a1a0a')
    expect(bytes.readUInt32BE(16)).toBe(size)
    expect(bytes.readUInt32BE(20)).toBe(size)
    expect(bytes.subarray(-8, -4).toString('ascii')).toBe('IEND')
  })

  it('提供包含小尺寸及256像素图层的完整Windows ICO', () => {
    const bytes = read('src-tauri/icons/icon.ico')
    expect(bytes.readUInt16LE(0)).toBe(0)
    expect(bytes.readUInt16LE(2)).toBe(1)
    const count = bytes.readUInt16LE(4)
    const sizes: number[] = []
    for (let index = 0; index < count; index++) {
      const offset = 6 + index * 16
      const size = bytes[offset] || 256
      sizes.push(size)
      expect(bytes[offset + 1] || 256).toBe(size)
      const length = bytes.readUInt32LE(offset + 8)
      const start = bytes.readUInt32LE(offset + 12)
      expect(length).toBeGreaterThan(0)
      expect(start).toBeGreaterThanOrEqual(6 + count * 16)
      expect(start + length).toBeLessThanOrEqual(bytes.length)
    }
    expect(sizes).toEqual(expect.arrayContaining([16, 24, 32, 48, 64, 256]))
  })

  it('提供完整的macOS ICNS图层，保持应用与升级身份不变', () => {
    const bytes = read('src-tauri/icons/icon.icns')
    expect(bytes.subarray(0, 4).toString('ascii')).toBe('icns')
    expect(bytes.readUInt32BE(4)).toBe(bytes.length)
    const types: string[] = []
    let offset = 8
    while (offset < bytes.length) {
      types.push(bytes.subarray(offset, offset + 4).toString('ascii'))
      const length = bytes.readUInt32BE(offset + 4)
      expect(length).toBeGreaterThan(8)
      offset += length
      expect(offset).toBeLessThanOrEqual(bytes.length)
    }
    expect(offset).toBe(bytes.length)
    expect(types).toEqual(expect.arrayContaining(['ic07', 'ic08', 'ic09', 'ic10']))
    const config = JSON.parse(read('src-tauri/tauri.conf.json').toString('utf8'))
    expect(config.productName).toBe('LoreKeeper')
    expect(config.bundle.windows.wix.upgradeCode).toBe('ed3e3199-4ddc-501a-a745-179a185e8dfb')
    expect(config.identifier).toBe('com.tancan1998.lorekeeper')
    expect(config.bundle.icon).toEqual(expect.arrayContaining(['icons/icon.ico', 'icons/icon.icns']))
  })
})
