import { useThemeStore } from '../../stores/theme-store'

/** 复用已定稿的单星字标；默认亮色，随应用主题切换暗色。 */
export function AppIcon({ size }: { size: number }) {
  const theme = useThemeStore(state => state.resolvedTheme)
  return (
    <img
      className="stage-app-icon"
      src={`/branding/app-icon-${theme === 'dark' ? 'dark' : 'light'}.svg`}
      width={size}
      height={size}
      alt=""
      aria-hidden="true"
      draggable={false}
    />
  )
}
