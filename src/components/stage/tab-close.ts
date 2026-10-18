import { discardAndCloseEditorTab } from '../../stores/editor-discard'
import { useEditorStore } from '../../stores/editor-store'
import { useStageStore } from '../../stores/navigation/stage-store'
import { useProjectStore } from '../../stores/project-store'
import { captureProjectSession, isProjectSessionCurrent } from '../project-session-gate'

export const STAGE_CONTENT_ID = 'stage-content'

/** 被关掉的标签带走了焦点（标签条可能随之收起），把焦点交给内容区，键盘不会落到页面之外。 */
function focusContent(): void {
  requestAnimationFrame(() => document.getElementById(STAGE_CONTENT_ID)?.focus({ preventScroll: true }))
}

/** 关闭标签：没有未保存修改就直接关；有修改时先请作者确认放弃。标签条与 ⌘W / Ctrl W 都走这里。 */
export function requestCloseTab(tabId: string): void {
  const tab = useEditorStore.getState().tabs.find(candidate => candidate.id === tabId)
  if (!tab || tab.pinned) return
  if (tab.dirty) {
    useStageStore.setState({ closingTabId: tabId })
    return
  }
  useEditorStore.getState().closeTab(tabId)
  focusContent()
}

export function confirmCloseTab(): void {
  const { closingTabId } = useStageStore.getState()
  const session = captureProjectSession(useProjectStore.getState().currentProject)
  if (closingTabId && session && isProjectSessionCurrent(session)) discardAndCloseEditorTab(closingTabId, session)
  useStageStore.setState({ closingTabId: null })
  focusContent()
}

export function cancelCloseTab(): void {
  useStageStore.setState({ closingTabId: null })
}
