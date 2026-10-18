import type { Theme } from '../../stores/theme-store'

/** 纸墨只有浅色、深色两套：旧主题里的深色与星空对应深色，其余对应浅色。 */
export function inkTheme(theme: Theme): 'light' | 'dark' {
  return theme === 'dark' || theme === 'galaxy' ? 'dark' : 'light'
}
