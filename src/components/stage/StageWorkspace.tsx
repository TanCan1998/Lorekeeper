import { useEffect } from 'react'
import { useProjectStore } from '../../stores/project-store'
import { useEditorStore, type EditorTab } from '../../stores/editor-store'
import { useLayoutStore } from '../../stores/layout-store'
import { useLocaleStore } from '../../stores/locale-store'
import { navigate } from '../../stores/navigation/navigation-intents'
import { useStageStore } from '../../stores/navigation/stage-store'
import type { ProjectData } from '../../shared/ipc-channels'
import { ErrorBoundary } from '../ErrorBoundary'
import EditorTabContent from '../panels/EditorTabContent'
import { Button } from '../ui/Button'
import { EmptyState } from '../ui/EmptyState'
import { DangerConfirm } from '../ui/DangerConfirm'
import RunSheet from './run/RunSheet'
import StageArchitecture from './architecture/StageArchitecture'
import StageCharacters from './characters/StageCharacters'
import StageLibrary from './library/StageLibrary'
import { SettingsPage, SetupPage } from './stage-pages'
import StageHistory from './history/StageHistory'
import StageKnowledge from './knowledge/StageKnowledge'
import StagePlot from './plot/StagePlot'
import StageTabs from './StageTabs'
import { commandLabel } from './shortcut-keys'
import StageBlueprint from './blueprint/StageBlueprint'
import ChapterView from './chapter/ChapterView'
import { installChapterTabRouting, isChapterViewTab } from './chapter/chapter-tab-routing'
import { stageTabName } from './stage-text'
import { cancelCloseTab, confirmCloseTab, requestCloseTab, STAGE_CONTENT_ID } from './tab-close'

/** 设定、架构、角色名单、蓝图、单章正文与资料页是新外壳的页面，其余标签沿用旧编辑区。 */
function TabContent({ tab, project }: { tab: EditorTab; project: ProjectData }) {
  if (tab.type === 'config' && tab.projectKey) return <SetupPage projectKey={tab.projectKey} />
  if (tab.type === 'world-building' && tab.projectKey) return <StageArchitecture projectKey={tab.projectKey} />
  if (tab.type === 'character' && tab.projectKey) return <StageCharacters projectKey={tab.projectKey} />
  if (isChapterViewTab(tab)) return <ChapterView tab={tab} />
  if (tab.type === 'chapter-card' && tab.projectKey && !tab.planningRecovery) {
    return <StageBlueprint projectKey={tab.projectKey} initialChapterNumber={tab.chapterNumber} selectRequest={tab.chapterSelectRequest} />
  }
  if (tab.type === 'version-history' && tab.projectKey === project.path) return <StageHistory projectKey={tab.projectKey} />
  if (tab.type === 'knowledge' && tab.projectKey === project.path) return <StageKnowledge tabId={tab.id} />
  if (tab.type === 'narrative-thread' && tab.projectKey === project.path) {
    return <StagePlot projectKey={tab.projectKey} initialView={tab.narrativeThreadView ?? 'plans'} viewRequest={tab.narrativeThreadViewRequest} />
  }
  return <EditorTabContent activeTab={tab} currentProject={project} />
}

export default function StageWorkspace({ library, focus }: { library: boolean; focus: boolean }) {
  const settingsOpen = useLayoutStore(s => s.settingsOpen)
  const text = useLocaleStore(s => s.text)
  const project = useProjectStore(s => s.currentProject)
  const tabs = useEditorStore(s => s.tabs)
  const activeTabId = useEditorStore(s => s.activeTabId)
  const sheet = useStageStore(s => s.sheet)
  const tabDomId = (tabId: string) => `${STAGE_CONTENT_ID}-tab-${tabId.replace(/[^\w-]/g, '_')}`
  const closingTabId = useStageStore(s => s.closingTabId)
  const pending = tabs.find(tab => tab.id === closingTabId) ?? null
  const pendingName = pending ? stageTabName(pending, text) : ''

  const activeTab = tabs.find(tab => tab.id === activeTabId)
  useEffect(() => installChapterTabRouting(), [])
  useEffect(() => {
    if (tabs.length > 0 && !activeTab) useEditorStore.getState().setActiveTab(tabs[0].id)
  }, [tabs, activeTab])

  if (library || !project) {
    return (
      <main className="stage-panel" data-page={settingsOpen ? 'settings' : 'library'}>
        {settingsOpen ? <SettingsPage library /> : <div className="stage-library"><StageLibrary /></div>}
      </main>
    )
  }

  // 设置页盖在内容区上，标签内容保持挂载（编辑器的撤销历史、滚动位置不丢），运行侧页照常在右侧。
  const showTabs = !focus && !settingsOpen && tabs.length > 1
  return (
    <main className="stage-panel">
      {showTabs && (
        <StageTabs
          tabs={tabs}
          activeTabId={activeTab?.id ?? null}
          onSelect={tabId => useEditorStore.getState().setActiveTab(tabId)}
          onClose={requestCloseTab}
          tabDomId={tabDomId}
          panelId={STAGE_CONTENT_ID}
        />
      )}
      <div className="stage-body">
        {settingsOpen && <SettingsPage library={false} />}
        <div
          className="stage-content"
          hidden={settingsOpen}
          id={STAGE_CONTENT_ID}
          tabIndex={-1}
          role={showTabs ? 'tabpanel' : undefined}
          aria-labelledby={showTabs && activeTab ? tabDomId(activeTab.id) : undefined}
        >
          {activeTab
            ? (
              <ErrorBoundary key={activeTab.id} fallbackLabel={text('编辑区渲染失败', 'Editor failed to render')}>
                <TabContent tab={activeTab} project={project} />
              </ErrorBoundary>
            )
            : (
              <EmptyState
                heading={text('从流程脊选一步开始', 'Pick a step in the flow to begin')}
                description={text('左侧按设定、架构、蓝图、正文排列，点任意一步打开它。', 'Setup, architecture, blueprint and manuscript are listed on the left. Open any of them.')}
                action={<Button variant="primary" onClick={() => navigate({ kind: 'stage', stage: 'setup' })}>{text('打开设定', 'Open setup')}</Button>}
              />
            )}
        </div>
        {sheet?.kind === 'run' && !focus && (
          <div className="stage-sheet-slot" data-wide={sheet.tab === 'diagnostics' || undefined}>
            <ErrorBoundary fallbackLabel={text('运行侧页渲染失败', 'Run sheet failed to render')}>
              <RunSheet sheet={sheet} />
            </ErrorBoundary>
          </div>
        )}
      </div>
      <DangerConfirm
        open={pending !== null}
        onOpenChange={open => { if (!open) cancelCloseTab() }}
        title={text('放弃未保存的修改？', 'Discard unsaved changes?')}
        consequence={text(`「${pendingName}」有未保存的修改，关闭后这些修改会丢失。`, `“${pendingName}” has unsaved changes. Closing it discards them.`)}
        recoverability={text(`丢弃后无法恢复。想保留就选「继续编辑」，再按 ${commandLabel('s')} 保存。`, `This cannot be undone. Choose “Keep editing” and press ${commandLabel('s')} to keep them.`)}
        confirmLabel={text('放弃并关闭', 'Discard and close')}
        cancelLabel={text('继续编辑', 'Keep editing')}
        onConfirm={confirmCloseTab}
      />
    </main>
  )
}
