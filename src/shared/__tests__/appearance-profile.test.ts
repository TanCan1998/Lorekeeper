import { describe, expect, it } from 'vitest'
import { INITIAL_APPEARANCE, migrateLegacyAppearance, parseAppearanceProfile, resolveShell, serializeAppearanceProfile } from '../appearance-profile'
import { parseAppearanceProfileV1 } from './fixtures/appearance-profile-v1-parser.fixture'

describe('appearance preference conversion', () => {
  it('resolves unset to the release Writer default without persisting a choice', () => {
    const profile = migrateLegacyAppearance(null, null, 'light')
    expect(profile.shellPreference).toBe('unset')
    expect(resolveShell(profile.shellPreference)).toBe('writer')
    expect(resolveShell(profile.shellPreference, 'classic')).toBe('classic')
    expect(resolveShell(profile.shellPreference, 'writer')).toBe('writer')
    expect(profile.shellPreference).toBe('unset')
    expect(resolveShell('classic')).toBe('classic')
    expect(resolveShell('writer')).toBe('writer')
    expect(resolveShell('classic', 'writer')).toBe('classic')
  })
  it.each(['v1', '{"version":"v1"}', '{"state":{"uiVersion":"v1"},"version":0}'])('migrates old Classic shape %s into Writer', raw => {
    expect(migrateLegacyAppearance(null, raw, 'light').shellPreference).toBe('writer')
  })
  it.each(['v2', '"v2"', '{"state":{"uiVersion":"v2"}}'])('maps old Writer shape %s', raw => {
    expect(migrateLegacyAppearance(null, raw, 'light').shellPreference).toBe('writer')
  })
  it('preserves explicit fonts even when equal to old defaults and donor version is zero', () => {
    const profile = migrateLegacyAppearance(JSON.stringify({ state: {
      theme: 'paper', zoom: 1.25, writingFont: 'lxgw-wenkai', uiFont: 'noto-sans-sc', fontDefaultsVersion: 0,
    }, version: 1 }), 'v2', 'light')
    expect(profile).toMatchObject({ colorTheme: 'paper', zoom: 1.25, writingFont: 'lxgw-wenkai',
      uiFont: 'noto-sans-sc', legacyFontDefaultsVersion: 0, shellPreference: 'writer' })
    expect(profile).not.toHaveProperty('backgroundSkin')
  })
  it('preserves legacy night/system display meaning without modifying the source', () => {
    expect(migrateLegacyAppearance('{"state":{"theme":"night"}}', null, 'light').colorTheme).toBe('dark')
    expect(migrateLegacyAppearance('{"state":{"theme":"system"}}', null, 'dark').colorTheme).toBe('dark')
  })
  it.each(['future', '{bad-json', '{"version":"v3"}'])('rejects unknown shell %s for safe Classic fallback', raw => {
    expect(() => migrateLegacyAppearance(null, raw, 'light')).toThrow('LEGACY_SHELL_UNKNOWN')
  })
  it.each(['{bad-json', '{"state":null}', '{"version":9,"state":{}}', '{"state":{"writingFont":"future-font"}}', '{"state":{"zoom":9}}', '{"state":{"extra":true}}'])('refuses unsupported legacy theme %s without conversion', raw => {
    expect(() => migrateLegacyAppearance(raw, null, 'light')).toThrow()
  })
  it.each([{ colorTheme: ['dark'] }, { schemaVersion: 3 }, { backgroundSkin: 'classic' }, { revision: 0 }, { zoom: null }])('rejects malformed/future canonical profile %j', patch => {
    expect(() => parseAppearanceProfile(JSON.stringify({ schemaVersion: 1, ...INITIAL_APPEARANCE, ...patch }))).toThrow()
  })
})

describe('schemaVersion：读 v1 与 v2，只有新外壳写成 v2', () => {
  const stored = (fields: Record<string, unknown> = {}) => JSON.stringify({
    schemaVersion: 1, revision: 7, shellPreference: 'classic', colorTheme: 'galaxy', zoom: 1.25, writingFont: 'noto-serif-sc',
    uiFont: 'inter', origin: 'author', legacyImportCompleted: true, legacyFontDefaultsVersion: 0, ...fields,
  })

  it.each(['unset', 'classic', 'writer'])('v1 的 %s 读回再写出与原文逐字节相同，内存里不带版本号', shell => {
    const raw = stored({ shellPreference: shell })
    const profile = parseAppearanceProfile(raw)
    expect(profile).not.toHaveProperty('schemaVersion')
    expect(serializeAppearanceProfile(profile)).toBe(raw)
  })

  it('只有 stage 写成 v2；v2 里不含 stage 的配置再写出时回到 v1', () => {
    const stage = serializeAppearanceProfile({ ...INITIAL_APPEARANCE, shellPreference: 'stage' })
    expect(JSON.parse(stage)).toMatchObject({ schemaVersion: 2, shellPreference: 'stage' })
    expect(resolveShell(parseAppearanceProfile(stage).shellPreference)).toBe('stage')
    expect(serializeAppearanceProfile(parseAppearanceProfile(stored({ schemaVersion: 2, shellPreference: 'writer' }))))
      .toBe(stored({ shellPreference: 'writer' }))
    expect(JSON.parse(serializeAppearanceProfile(INITIAL_APPEARANCE))).toMatchObject({ schemaVersion: 1 })
  })

  it('不含 stage 的写入，RD-3 之前的解析器原样读回；含 stage 的写入它按未来版本拒绝', () => {
    for (const shellPreference of ['unset', 'classic', 'writer'] as const) {
      const raw = serializeAppearanceProfile({ ...INITIAL_APPEARANCE, shellPreference, legacyFontDefaultsVersion: 2 })
      expect(parseAppearanceProfileV1(raw)).toEqual(JSON.parse(raw))
    }
    expect(() => parseAppearanceProfileV1(serializeAppearanceProfile({ ...INITIAL_APPEARANCE, shellPreference: 'stage' })))
      .toThrow('APPEARANCE_PROFILE_INVALID')
  })

  it('v1 里不能有 stage；未知字段、未来版本与坏 JSON 都拒绝，写入前同样校验', () => {
    expect(() => parseAppearanceProfile(stored({ shellPreference: 'stage' }))).toThrow('APPEARANCE_PROFILE_INVALID')
    expect(() => parseAppearanceProfile(stored({ backgroundSkin: 'anime' }))).toThrow('APPEARANCE_UNKNOWN_FIELDS')
    expect(() => parseAppearanceProfile(stored({ schemaVersion: 3 }))).toThrow('APPEARANCE_PROFILE_INVALID')
    expect(() => parseAppearanceProfile('{bad-json')).toThrow('APPEARANCE_JSON_INVALID')
    expect(() => serializeAppearanceProfile({ ...INITIAL_APPEARANCE, zoom: 9 })).toThrow('APPEARANCE_PROFILE_INVALID')
  })
})
