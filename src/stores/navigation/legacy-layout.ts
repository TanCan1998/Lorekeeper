import type { ChapterWritePrefill, LayoutFields, LeftRailItem } from '../layout-store'
import type { NavigationIntent, ResourcePage, WorkStage } from './navigation-intents'

const EXIT_IMMERSION = { immersive: false, preImmersivePanels: null } as const

const STAGE_RAIL: Record<WorkStage, LeftRailItem> = {
  setup: 'project',
  architecture: 'world',
  blueprint: 'blueprint',
  manuscript: 'project',
}

const RESOURCE_SIDEBAR: Record<ResourcePage, Pick<LayoutFields, 'sidebarView' | 'activeRailItem'>> = {
  plot: { sidebarView: 'project', activeRailItem: 'plot-tree' },
  knowledge: { sidebarView: 'knowledge', activeRailItem: 'knowledge' },
  history: { sidebarView: 'project', activeRailItem: 'project' },
}

/** 运行侧页在旧外壳里拆在右侧面板与底部面板，从首页打开时要先回到项目视图才看得见。 */
function leaveHome(state: Pick<LayoutFields, 'sidebarView'>): Partial<LayoutFields> {
  return state.sidebarView === 'home'
    ? { sidebarView: 'project', activeRailItem: 'project', sidebarOpen: true }
    : {}
}

/** 写作此章：打开章节创作弹窗并预填参数。两种外壳都由这个旧弹窗承接，RD-7 换成新弹层。 */
export function chapterWritePrefill(chapterNumber: number, write: ChapterWritePrefill): Pick<LayoutFields, 'chapterCreationOpen' | 'chapterCreationPrefill'> {
  return { chapterCreationOpen: true, chapterCreationPrefill: { ...write, chapterNumber } }
}

/** 意图在旧外壳下对应的布局字段组合。 */
export function legacyLayoutPatch(state: Pick<LayoutFields, 'sidebarView'>, intent: NavigationIntent): Partial<LayoutFields> {
  switch (intent.kind) {
    case 'stage':
      return { sidebarView: 'project', activeRailItem: STAGE_RAIL[intent.stage ?? 'manuscript'], sidebarOpen: true, ...EXIT_IMMERSION }
    case 'resource':
      return { ...RESOURCE_SIDEBAR[intent.page], sidebarOpen: true, ...EXIT_IMMERSION }
    case 'run':
      if (intent.facet === 'output') {
        return { ...leaveHome(state), aiPanelOpen: true, rightPanelRunId: intent.runId ?? null, ...EXIT_IMMERSION }
      }
      return { ...leaveHome(state), bottomPanelOpen: true, bottomTab: intent.facet, activeRailItem: intent.facet, ...EXIT_IMMERSION }
    case 'settings':
      return { settingsOpen: true, settingsSection: intent.section, activeRailItem: 'settings' }
    case 'chapter':
      return intent.write
        ? chapterWritePrefill(intent.chapterNumber, intent.write)
        : { sidebarView: 'project', activeRailItem: 'project', sidebarOpen: true, ...EXIT_IMMERSION }
  }
}
