import type { AppearanceProfile } from './project-storage'

export const APPEARANCE_STORAGE_KEY = 'ai-novel-writer-appearance'
export const LEGACY_THEME_STORAGE_KEY = 'ai-novel-writer-theme'
export const LEGACY_UI_STORAGE_KEY = 'ai-novel-writer-ui-version'
export const RELEASE_DEFAULT_SHELL: Shell = 'writer'
/** stage 是 RD-3 起的流程脊新外壳，只有 schemaVersion 2 能存。 */
export type Shell = 'classic' | 'writer' | 'stage'
export type ShellPreference = Shell | 'unset'
export type ColorTheme = 'light' | 'galaxy' | 'paper' | 'dark'
export type AppearanceFont = 'inter' | 'noto-sans-sc' | 'lxgw-wenkai' | 'noto-serif-sc' | 'system'

/** 内存里的外观偏好。schemaVersion 只出现在存储里，由 serializeAppearanceProfile 按取值决定。 */
export interface CanonicalAppearanceProfile extends AppearanceProfile {
  shellPreference: ShellPreference
  colorTheme: ColorTheme
  writingFont: AppearanceFont
  uiFont: AppearanceFont
  /** Provenance only. Never triggers a font reset. */
  legacyFontDefaultsVersion?: number
}

export const INITIAL_APPEARANCE: Readonly<CanonicalAppearanceProfile> = Object.freeze({
  revision: 1, shellPreference: 'unset', colorTheme: 'paper', zoom: 1,
  writingFont: 'lxgw-wenkai', uiFont: 'noto-sans-sc', origin: 'legacy-import', legacyImportCompleted: true,
})

const THEMES = ['light', 'galaxy', 'paper', 'dark']
const FONTS = ['inter', 'noto-sans-sc', 'lxgw-wenkai', 'noto-serif-sc', 'system']
const SHELL_PREFERENCES = {
  1: ['unset', 'classic', 'writer'],
  2: ['unset', 'classic', 'writer', 'stage'],
} as const satisfies Record<number, readonly ShellPreference[]>
type SchemaVersion = keyof typeof SHELL_PREFERENCES
const FIELDS = ['schemaVersion', 'revision', 'shellPreference', 'colorTheme', 'zoom', 'writingFont', 'uiFont', 'origin', 'legacyImportCompleted', 'legacyFontDefaultsVersion']

function record(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value)
}

export class AppearanceProfileError extends Error {
  constructor(readonly code: string) { super(code); this.name = 'AppearanceProfileError' }
}

export function resolveShell(preference: ShellPreference, releaseDefault: Shell = RELEASE_DEFAULT_SHELL): Shell {
  return preference === 'unset' ? releaseDefault : preference
}

function assertStoredProfile(value: unknown): asserts value is Record<string, unknown> & { schemaVersion: SchemaVersion } {
  if (!record(value) || (value.schemaVersion !== 1 && value.schemaVersion !== 2) || !Number.isSafeInteger(value.revision) || Number(value.revision) < 1
    || typeof value.shellPreference !== 'string' || !(SHELL_PREFERENCES[value.schemaVersion] as readonly string[]).includes(value.shellPreference)
    || typeof value.colorTheme !== 'string' || !THEMES.includes(value.colorTheme) || typeof value.zoom !== 'number' || !Number.isFinite(value.zoom)
    || value.zoom < 0.7 || value.zoom > 1.5 || typeof value.writingFont !== 'string' || !FONTS.includes(value.writingFont)
    || typeof value.uiFont !== 'string' || !FONTS.includes(value.uiFont)
    || typeof value.origin !== 'string' || !['legacy-import', 'author'].includes(value.origin) || value.legacyImportCompleted !== true
    || (value.legacyFontDefaultsVersion !== undefined && (!Number.isSafeInteger(value.legacyFontDefaultsVersion) || Number(value.legacyFontDefaultsVersion) < 0))) {
    throw new AppearanceProfileError('APPEARANCE_PROFILE_INVALID')
  }
  if (Object.keys(value).some(key => !FIELDS.includes(key))) throw new AppearanceProfileError('APPEARANCE_UNKNOWN_FIELDS')
}

/** 读取已保存的配置：v1 与 v2 各按自己的规则严格校验，v1 里出现 stage 或更高版本都拒绝。 */
export function parseAppearanceProfile(raw: string): CanonicalAppearanceProfile {
  let value: unknown
  try { value = JSON.parse(raw) } catch { throw new AppearanceProfileError('APPEARANCE_JSON_INVALID') }
  assertStoredProfile(value)
  const profile: Record<string, unknown> = { ...value }
  delete profile.schemaVersion
  return profile as unknown as CanonicalAppearanceProfile
}

/**
 * 写成能表示这份取值的最低版本：只有 stage 需要 v2，其余都写 v1。
 * 回退到 RD-3 之前的版本时，只要没选新外壳，旧版本照样能读这份配置。
 */
export function serializeAppearanceProfile(profile: CanonicalAppearanceProfile): string {
  const stored = { schemaVersion: profile.shellPreference === 'stage' ? 2 : 1, ...profile }
  assertStoredProfile(stored)
  return JSON.stringify(stored)
}

/** Pure conversion. Legacy bytes stay in their original keys, including unknown/corrupt values. */
export function migrateLegacyAppearance(themeRaw: string | null, shellRaw: string | null, systemTheme: 'light' | 'dark'): CanonicalAppearanceProfile {
  const profile: CanonicalAppearanceProfile = { ...INITIAL_APPEARANCE }
  if (themeRaw !== null) {
    let parsed: unknown
    try { parsed = JSON.parse(themeRaw) } catch { throw new AppearanceProfileError('LEGACY_THEME_JSON_INVALID') }
    if (!record(parsed) || !record(parsed.state)) throw new AppearanceProfileError('LEGACY_THEME_INVALID')
    if (parsed.version !== undefined && parsed.version !== 0 && parsed.version !== 1) throw new AppearanceProfileError('LEGACY_THEME_VERSION_UNKNOWN')
    const state = parsed.state
    if (Object.keys(state).some(key => !['theme', 'zoom', 'writingFont', 'uiFont', 'fontDefaultsVersion'].includes(key))) {
      throw new AppearanceProfileError('LEGACY_THEME_FIELDS_UNKNOWN')
    }
    if (state.theme !== undefined) profile.colorTheme = (state.theme === 'night' ? 'dark' : state.theme === 'system' ? systemTheme : state.theme) as ColorTheme
    if (state.zoom !== undefined) profile.zoom = state.zoom as number
    if (state.writingFont !== undefined) profile.writingFont = state.writingFont as AppearanceFont
    if (state.uiFont !== undefined) profile.uiFont = state.uiFont as AppearanceFont
    if (state.fontDefaultsVersion !== undefined) profile.legacyFontDefaultsVersion = state.fontDefaultsVersion as number
  }
  if (shellRaw !== null) {
    let candidate: unknown = shellRaw
    if (shellRaw !== 'v1' && shellRaw !== 'v2') {
      try {
        const parsed: unknown = JSON.parse(shellRaw)
        candidate = record(parsed) ? (record(parsed.state) ? parsed.state.uiVersion : parsed.version) : parsed
      } catch { throw new AppearanceProfileError('LEGACY_SHELL_UNKNOWN') }
    }
    if (candidate !== 'v1' && candidate !== 'v2') throw new AppearanceProfileError('LEGACY_SHELL_UNKNOWN')
    profile.shellPreference = 'writer'
  }
  return parseAppearanceProfile(serializeAppearanceProfile(profile))
}

export type AppearancePatch = Partial<Pick<CanonicalAppearanceProfile, 'shellPreference' | 'colorTheme' | 'zoom' | 'writingFont' | 'uiFont'>>
