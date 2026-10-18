import type { ReactNode } from 'react'
import { useAppearanceStore } from '../../stores/appearance-bootstrap'
import { useLocaleStore } from '../../stores/locale-store'
import type { Theme } from '../../stores/theme-store'
import { ErrorBoundary } from '../ErrorBoundary'
import type { WindowCloseGuard } from '../layout/use-window-close-guard'
import { Button } from '../ui/Button'
import { EmptyState } from '../ui/EmptyState'
import ExitGuardDialog from './ExitGuardDialog'
import { inkTheme } from './ink-theme'

/** 外壳偏好已经保存，外壳出错后重启仍会进新外壳，所以出错界面直接提供切回工作台。 */
function StageFallback() {
  const text = useLocaleStore(s => s.text)
  return (
    <main className="stage-fallback">
      <EmptyState
        heading={text('创作流程界面出错了', 'The creative flow layout ran into a problem')}
        description={text(
          '已打开的页面都还在。切回工作台继续写作，之后可以在设置的外观里再换回来。',
          'Your open pages are kept. Switch to the workbench to keep writing; you can change back under Settings, Appearance.',
        )}
        action={(
          <Button variant="primary" onClick={() => useAppearanceStore.getState().update({ shellPreference: 'unset' })}>
            {text('切回工作台', 'Switch to workbench')}
          </Button>
        )}
      />
    </main>
  )
}

/**
 * 新外壳的根：纸墨作用域、渲染出错兜底与退出确认。
 * 退出确认挂在外壳之外，外壳渲染出错后关窗仍能保存或放弃未保存的内容。
 */
export default function StageRoot({ theme, exitGuard, children }: { theme: Theme; exitGuard: WindowCloseGuard; children: ReactNode }) {
  return (
    <div className="paper-ink stage-root" data-theme={inkTheme(theme)}>
      <ErrorBoundary fallback={<StageFallback />}>{children}</ErrorBoundary>
      <ExitGuardDialog guard={exitGuard} />
    </div>
  )
}
