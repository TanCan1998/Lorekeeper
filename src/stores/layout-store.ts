import { create } from 'zustand'

/** 左侧活动栏的视图类型 */
export type SidebarView = 'home' | 'project' | 'knowledge' | 'characters' | 'settings'

/** 下方工具窗口 Tab */
export type BottomTab = 'tasks' | 'log' | 'models'

/** 左侧主导航当前视觉激活项 */
export type LeftRailItem = SidebarView | 'blueprint' | 'world' | 'plot-tree' | BottomTab

/** 设置弹窗分类 */
export type SettingsSection = 'appearance' | 'llm' | 'embedding' | 'proxy' | 'editor' | 'prompts' | 'skills' | 'backup' | 'about'

/** 写作此章时预填到章节创作弹窗的参数，章号由 chapter 意图给出。 */
export type ChapterWritePrefill = {
  title: string
  role: string
  purpose: string
  keyEvents: string
  characters: string
  userGuidance: string
}

/** 章节创建对话框的预填参数 */
export type ChapterCreationPrefill = (ChapterWritePrefill & { chapterNumber: number }) | null

/** 旧外壳读取的布局字段。跨层导航经 `navigate` 写入，见 `navigation/legacy-layout.ts`。 */
export interface LayoutFields {
  // ===== 侧边栏 =====
  sidebarOpen: boolean
  sidebarView: SidebarView
  sidebarWidth: number
  activeRailItem: LeftRailItem

  // ===== 右侧 AI 输出面板 =====
  aiPanelOpen: boolean
  aiPanelWidth: number
  /** AI 输出要定位的运行，面板消费后清空。 */
  rightPanelRunId: string | null

  // ===== 底部面板 =====
  bottomPanelOpen: boolean
  bottomTab: BottomTab
  bottomPanelHeight: number

  /** 沉浸前面板状态只在当前窗口保留。 */
  immersive: boolean
  preImmersivePanels: { sidebarOpen: boolean; aiPanelOpen: boolean; bottomPanelOpen: boolean } | null

  // ===== 全局弹窗状态（替代 window.dispatchEvent 事件总线）=====
  /** 设置弹窗是否打开 */
  settingsOpen: boolean
  /** 设置弹窗当前的分类：打开时由入口指定，作者点左侧分类时更新；切换外壳重挂弹窗后仍停在这里。 */
  settingsSection: SettingsSection
  /** 新建项目对话框是否打开 */
  newProjectOpen: boolean
  /** 导出对话框是否打开 */
  exportOpen: boolean
  /** 导入小说对话框是否打开 */
  importNovelOpen: boolean
  /** 章节创建对话框是否打开 */
  chapterCreationOpen: boolean
  /** 章节创建对话框的预填参数 */
  chapterCreationPrefill: ChapterCreationPrefill
}

interface LayoutState extends LayoutFields {
  toggleSidebar: () => void
  setSidebarView: (view: SidebarView, activeRailItem?: LeftRailItem) => void
  setSidebarWidth: (width: number) => void
  toggleAIPanel: () => void
  setAIPanelOpen: (open: boolean) => void
  setAIPanelWidth: (width: number) => void
  toggleBottomPanel: () => void
  setBottomTab: (tab: BottomTab) => void
  setBottomPanelHeight: (height: number) => void
  toggleImmersion: () => void

  // ===== 全局弹窗 Actions =====
  closeSettings: () => void
  openNewProject: () => void
  closeNewProject: () => void
  openExport: () => void
  closeExport: () => void
  openImportNovel: () => void
  closeImportNovel: () => void
  closeChapterCreation: () => void
}

export const useLayoutStore = create<LayoutState>()((set, get) => ({
  // 默认值
  sidebarOpen: true,
  sidebarView: 'project',
  sidebarWidth: 260,
  activeRailItem: 'project',

  aiPanelOpen: true,
  aiPanelWidth: 320,
  rightPanelRunId: null,

  bottomPanelOpen: true,
  bottomTab: 'tasks',
  bottomPanelHeight: 200,
  immersive: false,
  preImmersivePanels: null,

  // 全局弹窗默认关闭
  settingsOpen: false,
  settingsSection: 'llm',
  newProjectOpen: false,
  exportOpen: false,
  importNovelOpen: false,
  chapterCreationOpen: false,
  chapterCreationPrefill: null,

  // Actions
  toggleSidebar: () => set((s) => ({ sidebarOpen: !s.sidebarOpen, immersive: false, preImmersivePanels: null })),
  setSidebarView: (view, activeRailItem) =>
    set((s) => {
      const nextRailItem = activeRailItem ?? view
      const sameButton = s.sidebarView === view && s.activeRailItem === nextRailItem
      return {
        sidebarView: view,
        activeRailItem: nextRailItem,
        sidebarOpen: sameButton ? !s.sidebarOpen : true,
        immersive: false,
        preImmersivePanels: null,
      }
    }),
  setSidebarWidth: (width) => set({ sidebarWidth: Math.max(200, Math.min(500, width)) }),

  toggleAIPanel: () => set((s) => ({ aiPanelOpen: !s.aiPanelOpen, immersive: false, preImmersivePanels: null })),
  setAIPanelOpen: (open) => set({ aiPanelOpen: open, ...(open ? { immersive: false, preImmersivePanels: null } : {}) }),
  setAIPanelWidth: (width) => set({ aiPanelWidth: Math.max(260, Math.min(600, width)) }),

  toggleBottomPanel: () => set((s) => ({ bottomPanelOpen: !s.bottomPanelOpen, immersive: false, preImmersivePanels: null })),
  setBottomTab: (tab) => {
    if (get().sidebarView === 'home') get().setSidebarView('project')
    set((s) => {
      const sameButton = s.bottomTab === tab && s.activeRailItem === tab
      return {
        bottomTab: tab,
        activeRailItem: tab,
        bottomPanelOpen: sameButton ? !s.bottomPanelOpen : true,
        immersive: false,
        preImmersivePanels: null,
      }
    })
  },
  setBottomPanelHeight: (height) => set({ bottomPanelHeight: Math.max(100, Math.min(500, height)) }),
  toggleImmersion: () => set((s) => s.immersive
    ? { ...s.preImmersivePanels, immersive: false, preImmersivePanels: null }
    : {
      immersive: true,
      preImmersivePanels: {
        sidebarOpen: s.sidebarOpen,
        aiPanelOpen: s.aiPanelOpen,
        bottomPanelOpen: s.bottomPanelOpen,
      },
      sidebarOpen: false,
      aiPanelOpen: false,
      bottomPanelOpen: false,
    }),

  // 全局弹窗 Actions
  closeSettings: () => set({ settingsOpen: false }),
  openNewProject: () => set({ newProjectOpen: true }),
  closeNewProject: () => set({ newProjectOpen: false }),
  openExport: () => set({ exportOpen: true }),
  closeExport: () => set({ exportOpen: false }),
  openImportNovel: () => set({ importNovelOpen: true }),
  closeImportNovel: () => set({ importNovelOpen: false }),
  closeChapterCreation: () => set({ chapterCreationOpen: false, chapterCreationPrefill: null }),
}))
