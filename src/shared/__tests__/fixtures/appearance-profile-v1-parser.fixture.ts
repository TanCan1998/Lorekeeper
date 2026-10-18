/**
 * eb0affc1（RD-3 之前的最后一版）外观配置解析器的原样拷贝，只把返回类型换成 unknown、去掉导出的错误类。
 * 新版本写出的配置要能被回退后的旧版本读回，测试用它当旧版本；不要随新版本修改。
 */
const THEMES = ['light', 'galaxy', 'paper', 'dark']
const FONTS = ['inter', 'noto-sans-sc', 'lxgw-wenkai', 'noto-serif-sc', 'system']
function record(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value)
}

class AppearanceProfileError extends Error {
  constructor(readonly code: string) { super(code); this.name = 'AppearanceProfileError' }
}

export function parseAppearanceProfileV1(raw: string): unknown {
  let value: unknown
  try { value = JSON.parse(raw) } catch { throw new AppearanceProfileError('APPEARANCE_JSON_INVALID') }
  if (!record(value) || value.schemaVersion !== 1 || !Number.isSafeInteger(value.revision) || Number(value.revision) < 1
    || typeof value.shellPreference !== 'string' || !['unset', 'classic', 'writer'].includes(value.shellPreference)
    || typeof value.colorTheme !== 'string' || !THEMES.includes(value.colorTheme) || typeof value.zoom !== 'number' || !Number.isFinite(value.zoom)
    || value.zoom < 0.7 || value.zoom > 1.5 || typeof value.writingFont !== 'string' || !FONTS.includes(value.writingFont)
    || typeof value.uiFont !== 'string' || !FONTS.includes(value.uiFont)
    || typeof value.origin !== 'string' || !['legacy-import', 'author'].includes(value.origin) || value.legacyImportCompleted !== true
    || (value.legacyFontDefaultsVersion !== undefined && (!Number.isSafeInteger(value.legacyFontDefaultsVersion) || Number(value.legacyFontDefaultsVersion) < 0))) {
    throw new AppearanceProfileError('APPEARANCE_PROFILE_INVALID')
  }
  const allowed = ['schemaVersion', 'revision', 'shellPreference', 'colorTheme', 'zoom', 'writingFont', 'uiFont', 'origin', 'legacyImportCompleted', 'legacyFontDefaultsVersion']
  if (Object.keys(value).some(key => !allowed.includes(key))) throw new AppearanceProfileError('APPEARANCE_UNKNOWN_FIELDS')
  return value
}
