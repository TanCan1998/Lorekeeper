import { type ReactNode, useEffect } from 'react'
import { type Theme, useThemeStore } from './stores/theme-store'
import { useLayoutStore } from './stores/layout-store'
import { useAppearanceStore } from './stores/appearance-bootstrap'
import { navigate } from './stores/navigation/navigation-intents'
import { useLLMStore } from './stores/llm-store'
import { useProjectStore } from './stores/project-store'
import { useWorkflowStore } from './stores/workflow-store'
import { useLocaleStore } from './stores/locale-store'
import { useSkinStore } from './stores/skin-store'
import { ipc } from './services/ipc-client'
import TitleBar from './components/layout/TitleBar'
import { useWindowCloseGuard } from './components/layout/use-window-close-guard'
import StatusBar from './components/layout/StatusBar'
import LeftToolWindowBar from './components/layout/LeftToolWindowBar'
import RightToolWindowBar from './components/layout/RightToolWindowBar'
import ShellV2 from './components/layout/v2/ShellV2'
import Sidebar from './components/panels/Sidebar'
import EditorArea from './components/panels/EditorArea'
import AIOutputPanel from './components/panels/AIOutputPanel'
import BottomPanel from './components/panels/BottomPanel'
import NewProjectDialog from './components/dialogs/NewProjectDialog'
import ImportNovelDialog from './components/dialogs/ImportNovelDialog'
import ChapterCreationDialog from './components/dialogs/ChapterCreationDialog'
import ExportDialog from './components/dialogs/ExportDialog'
import SettingsModal from './components/settings/SettingsModal'
import { leaveSettingsThen, openProjectFolder } from './components/settings/unsaved-settings'
import { ANIME_SKIN_URL } from './components/settings/AppearanceSettings'
import { ErrorBoundary } from './components/ErrorBoundary'
import StageRoot from './components/stage/StageRoot'
import StageShell from './components/stage/StageShell'
import { actionToast } from './components/ui/ActionToast'
import { UpdateNotifier } from './components/updates/UpdateNotifier'
import { globalEventBus } from './shared/event-bus'
import {
  projectSessionContextFromProject,
  sameProjectSessionContext,
} from './shared/project-session-context'
import { getAutoNextChapterPrefill, type NextChapterBlueprint } from './services/auto-next-chapter'
import type { SkinId } from './shared/skin-types'

// eslint-disable-next-line react-refresh/only-export-components
export function resolveSkinBackgroundUrl(skinId: SkinId, customUrl: string | null): string | null {
  if (skinId === 'anime') return ANIME_SKIN_URL
  if (skinId === 'custom') return customUrl
  return null
}

/** The sole decorative skin layer at the App root. It never participates in accessibility. */
export function SkinBackgroundLayer({
  skinId,
  backgroundUrl,
  onImageError,
}: {
  skinId: SkinId
  backgroundUrl: string | null
  onImageError?: () => void
}) {
  const imageUrl = resolveSkinBackgroundUrl(skinId, backgroundUrl)
  return (
    <div
      aria-hidden="true"
      data-skin-background={skinId}
      className="app-skin-background"
    >
      {imageUrl && <img src={imageUrl} alt="" className="app-skin-background-image" onError={onImageError} />}
    </div>
  )
}

/** Stable root semantics let image skins opt into their own readability tokens for every color theme. */
export function AppSkinRoot({
  theme,
  skinId,
  children,
}: {
  theme: Theme
  skinId: SkinId
  children: ReactNode
}) {
  return (
    <div
      className="app-skin-root flex flex-col w-full h-full overflow-hidden"
      data-theme={theme}
      data-skin={skinId}
      data-skin-readability={skinId === 'classic' ? 'theme-default' : 'high-contrast'}
    >
      {children}
    </div>
  )
}

/**
 * 主应用组件
 * 使用 react-resizable-panels 实现可拖拽调整大小的四区布局
 */
export default function App() {
  const initTheme = useThemeStore((s) => s.initTheme)
  const resolvedTheme = useThemeStore((s) => s.resolvedTheme)
  const initLocale = useLocaleStore((s) => s.init)
  const text = useLocaleStore((s) => s.text)
  const sidebarOpen = useLayoutStore(s => s.sidebarOpen)
  const sidebarView = useLayoutStore(s => s.sidebarView)
  const currentProject = useProjectStore(s => s.currentProject)
  const aiPanelOpen = useLayoutStore(s => s.aiPanelOpen)
  const bottomPanelOpen = useLayoutStore(s => s.bottomPanelOpen)
  const immersive = useLayoutStore(s => s.immersive)
  const settingsOpen = useLayoutStore(s => s.settingsOpen)
  const closeSettings = useLayoutStore(s => s.closeSettings)
  const newProjectOpen = useLayoutStore(s => s.newProjectOpen)
  const closeNewProject = useLayoutStore(s => s.closeNewProject)
  const exportOpen = useLayoutStore(s => s.exportOpen)
  const closeExport = useLayoutStore(s => s.closeExport)
  const importNovelOpen = useLayoutStore(s => s.importNovelOpen)
  const closeImportNovel = useLayoutStore(s => s.closeImportNovel)
  const chapterCreationOpen = useLayoutStore(s => s.chapterCreationOpen)
  const chapterCreationPrefill = useLayoutStore(s => s.chapterCreationPrefill)
  const closeChapterCreation = useLayoutStore(s => s.closeChapterCreation)
  const initLLM = useLLMStore((s) => s.init)
  const loadRecentProjects = useProjectStore((s) => s.loadRecentProjects)
  const skinState = useSkinStore((s) => s.skinState)
  const skinBackgroundUrl = useSkinStore((s) => s.backgroundUrl)
  const initSkin = useSkinStore((s) => s.init)
  const disposeSkin = useSkinStore((s) => s.dispose)
  const recoverFromImageFailure = useSkinStore((s) => s.recoverFromImageFailure)
  const shell = useAppearanceStore((s) => s.resolvedShell)
  const exitGuard = useWindowCloseGuard()

  // 初始化：主题 + LLM 模型 + 最近项目 + 缩放级别
  useEffect(() => {
    initLocale()
    initTheme()
    initLLM()
    loadRecentProjects()
    // 初始化 ProjectService — 注册全局事件监听（生命周期与 App 一致）
    import('./services/project-service').then(({ initProjectService }) => {
      initProjectService()
    }).catch(e => console.warn('[ProjectService] 初始化失败:', e))

    // C) 工作流完成时弹出 ActionToast 通知（不依赖任何面板状态）
    const unsubActionToast = globalEventBus.on('WORKFLOW_COMPLETE', ({ projectSession, runId }) => {
      if (!sameProjectSessionContext(
        projectSession,
        projectSessionContextFromProject(useProjectStore.getState().currentProject),
      )) return
      const text = useLocaleStore.getState().text
      const { activeRuns, history } = useWorkflowStore.getState()
      const completedRun = activeRuns.find(r => r.id === runId)
        ?? history.find(r => r.id === runId)
      if (!completedRun) return
      actionToast.workflowComplete(
        text(`「${completedRun.title}」已完成`, `“${completedRun.title}” completed`),
        () => navigate({ kind: 'run', facet: 'output', runId }),
      )
    })

    const unsubAutoOpenNextChapter = globalEventBus.on('FINALIZE_COMPLETE', ({
      chapterNumber,
      projectSession,
      source,
    }) => {
      void (async () => {
        try {
          if (source === 'batch') return
          const isCurrentProjectSession = () => sameProjectSessionContext(
            projectSession,
            projectSessionContextFromProject(useProjectStore.getState().currentProject),
          )
          if (!isCurrentProjectSession()) return
          const config = await ipc.invoke('config:get')
          if (!isCurrentProjectSession()) return
          if (!config.autoOpenNextChapterAfterFinalize) return

          const nextChapterNumber = chapterNumber + 1
          const [blueprint, existingDraft] = await Promise.all([
            ipc.invokeWithProjectSession(
              projectSession,
              'db:blueprint-get',
              nextChapterNumber,
              projectSession.projectPath,
            ),
            ipc.invokeWithProjectSession(
              projectSession,
              'db:draft-get-latest',
              nextChapterNumber,
              projectSession.projectPath,
            ),
          ])
          if (!isCurrentProjectSession()) return
          const prefill = getAutoNextChapterPrefill(
            true,
            chapterNumber,
            blueprint as NextChapterBlueprint | null,
            existingDraft !== null,
          )
          if (prefill) navigate({ kind: 'chapter', chapterNumber: nextChapterNumber, write: prefill })
        } catch (error) {
          console.warn('[AutoNextChapter] 无法打开下一章创作窗口:', error)
        }
      })()
    })

    return () => {
      // App 卸载时销毁 ProjectService（开发环境 HMR 时会触发）
      import('./services/project-service').then(({ disposeProjectService }) => {
        disposeProjectService()
      }).catch(() => {})
      unsubActionToast()
      unsubAutoOpenNextChapter()
    }
  }, [initLocale, initTheme, initLLM, loadRecentProjects])

  useEffect(() => {
    if (!ipc.isElectron) return undefined
    void initSkin()
    return disposeSkin
  }, [disposeSkin, initSkin])

  useEffect(() => {
    if (!ipc.isElectron) return
    void ipc.invoke('project:smoke-open-request').then(async (request) => {
      if (!request) return
      const opened = await useProjectStore.getState().openProject(request.projectPath)
      if (!opened) throw new Error('烟测项目打开失败')
      const confirmed = await ipc.invoke('project:smoke-open-confirm', request.projectPath)
      if (!confirmed.success) throw new Error(confirmed.error || '烟测项目确认失败')
    }).catch(error => console.error('[SmokeProjectOpen]', error))
  }, [])

  // 全局快捷键: Cmd+N 新建项目，Cmd+O 打开项目，Cmd+= / Cmd+- / Cmd+0 缩放；两种外壳共用
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey
      if (!mod) return
      if (e.key === 'n' || e.key === 'N') {
        e.preventDefault()
        leaveSettingsThen(() => useLayoutStore.getState().openNewProject())
      } else if (e.key === 'o' || e.key === 'O') {
        e.preventDefault()
        openProjectFolder()
      } else if (e.key === '=' || e.key === '+') {
        e.preventDefault()
        useThemeStore.getState().zoomIn()
      } else if (e.key === '-') {
        e.preventDefault()
        useThemeStore.getState().zoomOut()
      } else if (e.key === '0') {
        e.preventDefault()
        useThemeStore.getState().zoomReset()
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [])

  const globalDialogs = (
    <>
      {/* 全局对话框 — 由 layout-store 控制开关，不再依赖 window.dispatchEvent */}
      <ImportNovelDialog
        open={importNovelOpen}
        onClose={closeImportNovel}
      />
      <ChapterCreationDialog
        isOpen={chapterCreationOpen}
        prefill={chapterCreationPrefill}
        onClose={closeChapterCreation}
      />
      <ExportDialog
        isOpen={exportOpen}
        onClose={closeExport}
      />
    </>
  )

  if (shell === 'stage') {
    return (
      <>
        <StageRoot theme={resolvedTheme} exitGuard={exitGuard}>
          <StageShell theme={resolvedTheme} />
        </StageRoot>
        <UpdateNotifier />
        {globalDialogs}
      </>
    )
  }

  return (
    <AppSkinRoot theme={resolvedTheme} skinId={skinState.activeSkin}>
      <SkinBackgroundLayer
        skinId={skinState.activeSkin}
        backgroundUrl={skinBackgroundUrl}
        onImageError={() => void recoverFromImageFailure()}
      />
      <UpdateNotifier />
      <ShellV2
        home={!currentProject || sidebarView === 'home'}
        theme={resolvedTheme}
        titleBar={<TitleBar exitGuard={exitGuard} />}
        rail={<LeftToolWindowBar />}
        sidebar={(
          <ErrorBoundary fallbackLabel={text('侧边栏渲染失败', 'Sidebar failed to render')}>
            <Sidebar />
          </ErrorBoundary>
        )}
        editor={(
          <ErrorBoundary fallbackLabel={text('编辑区渲染失败', 'Editor failed to render')}>
            <EditorArea onNewProject={() => useLayoutStore.getState().openNewProject()} />
          </ErrorBoundary>
        )}
        aiPanel={(
          <ErrorBoundary fallbackLabel={text('AI 面板渲染失败', 'AI panel failed to render')}>
            <AIOutputPanel />
          </ErrorBoundary>
        )}
        bottom={<BottomPanel />}
        statusBar={<StatusBar />}
        rightRail={<RightToolWindowBar />}
        sidebarOpen={sidebarOpen}
        aiPanelOpen={aiPanelOpen}
        bottomOpen={bottomPanelOpen}
        immersive={immersive}
      />

      {/* 新外壳有自己的纸墨新建对话框与设置页，这两个弹窗只属于旧外壳。 */}
      <NewProjectDialog
        open={newProjectOpen}
        onClose={closeNewProject}
      />
      {globalDialogs}
      {/* 全屏设置弹窗 */}
      <SettingsModal
        open={settingsOpen}
        onClose={closeSettings}
      />
    </AppSkinRoot>
  )
}
