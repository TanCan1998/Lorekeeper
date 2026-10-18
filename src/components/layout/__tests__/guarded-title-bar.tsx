import TitleBar from '../TitleBar'
import { useWindowCloseGuard } from '../use-window-close-guard'

/** 关窗守卫由 App 在外壳之外持有；测试按 App 的方式把它接到标题栏上。 */
export function GuardedTitleBar() {
  return <TitleBar exitGuard={useWindowCloseGuard()} />
}
