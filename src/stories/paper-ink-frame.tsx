import type { Decorator } from '@storybook/react'

/** 把 Storybook 工具栏的主题映射到纸墨作用域：dark 与 galaxy 为深色，其余为浅色。 */
export const paperInkFrame: Decorator = (Story, context) => {
  const theme = (context.globals as { theme?: string }).theme
  return (
    <div className="paper-ink" data-theme={theme === 'dark' || theme === 'galaxy' ? 'dark' : 'light'} style={{ padding: 32, minHeight: 360 }}>
      <Story />
    </div>
  )
}
