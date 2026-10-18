import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import type { ProjectData } from '../../../shared/ipc-channels'
import { useEditorStore } from '../../editor-store'
import { useLayoutStore, type LayoutFields } from '../../layout-store'
import { useProjectStore } from '../../project-store'
import { navigate, type NavigationIntent } from '../navigation-intents'

const PROJECT_PATH = 'C:\\novels\\intents'

const START: LayoutFields = {
  sidebarOpen: false,
  sidebarView: 'characters',
  sidebarWidth: 260,
  activeRailItem: 'characters',
  aiPanelOpen: false,
  aiPanelWidth: 320,
  rightPanelRunId: null,
  bottomPanelOpen: false,
  bottomTab: 'log',
  bottomPanelHeight: 200,
  immersive: true,
  preImmersivePanels: { sidebarOpen: true, aiPanelOpen: true, bottomPanelOpen: true },
  settingsOpen: false,
  settingsSection: 'llm',
  newProjectOpen: false,
  exportOpen: false,
  importNovelOpen: false,
  chapterCreationOpen: false,
  chapterCreationPrefill: null,
}

const EXITED = { immersive: false, preImmersivePanels: null }

const WRITE = { title: '夜雨', role: '发展', purpose: '交代来信', keyEvents: '收到密信', characters: '林岚', userGuidance: '' }

function layoutFields(): Partial<LayoutFields> {
  const state = useLayoutStore.getState()
  return Object.fromEntries(Object.keys(START).map(key => [key, state[key as keyof LayoutFields]]))
}

const originalLayout = useLayoutStore.getState()
const originalEditor = useEditorStore.getState()
const originalProject = useProjectStore.getState()

beforeEach(() => {
  useLayoutStore.setState(START)
  useEditorStore.setState({ tabs: [], activeTabId: null })
  useProjectStore.setState({ currentProject: { id: 'intents', name: 'intents', path: PROJECT_PATH } as ProjectData })
})

afterEach(() => {
  useLayoutStore.setState(originalLayout, true)
  useEditorStore.setState(originalEditor, true)
  useProjectStore.setState(originalProject, true)
})

describe('旧外壳下每种意图映射到原字段组合', () => {
  const cases: Array<[string, NavigationIntent, Partial<LayoutFields>]> = [
    ['回到流程脊', { kind: 'stage' }, { sidebarView: 'project', activeRailItem: 'project', sidebarOpen: true, ...EXITED }],
    ['设定阶段', { kind: 'stage', stage: 'setup' }, { sidebarView: 'project', activeRailItem: 'project', sidebarOpen: true, ...EXITED }],
    ['架构阶段', { kind: 'stage', stage: 'architecture' }, { sidebarView: 'project', activeRailItem: 'world', sidebarOpen: true, ...EXITED }],
    ['蓝图阶段', { kind: 'stage', stage: 'blueprint' }, { sidebarView: 'project', activeRailItem: 'blueprint', sidebarOpen: true, ...EXITED }],
    ['正文阶段', { kind: 'stage', stage: 'manuscript' }, { sidebarView: 'project', activeRailItem: 'project', sidebarOpen: true, ...EXITED }],
    ['剧情资料', { kind: 'resource', page: 'plot' }, { sidebarView: 'project', activeRailItem: 'plot-tree', sidebarOpen: true, ...EXITED }],
    ['知识库资料', { kind: 'resource', page: 'knowledge' }, { sidebarView: 'knowledge', activeRailItem: 'knowledge', sidebarOpen: true, ...EXITED }],
    ['历史资料', { kind: 'resource', page: 'history' }, { sidebarView: 'project', activeRailItem: 'project', sidebarOpen: true, ...EXITED }],
    ['运行输出并定位运行', { kind: 'run', facet: 'output', runId: 'run-7' }, { aiPanelOpen: true, rightPanelRunId: 'run-7', ...EXITED }],
    ['运行任务', { kind: 'run', facet: 'tasks' }, { bottomPanelOpen: true, bottomTab: 'tasks', activeRailItem: 'tasks', ...EXITED }],
    ['运行日志', { kind: 'run', facet: 'log' }, { bottomPanelOpen: true, bottomTab: 'log', activeRailItem: 'log', ...EXITED }],
    ['运行模型调用', { kind: 'run', facet: 'models' }, { bottomPanelOpen: true, bottomTab: 'models', activeRailItem: 'models', ...EXITED }],
    ['设置分组不退出沉浸', { kind: 'settings', section: 'backup' }, { settingsOpen: true, settingsSection: 'backup', activeRailItem: 'settings' }],
    ['打开章节', { kind: 'chapter', chapterNumber: 3 }, { sidebarView: 'project', activeRailItem: 'project', sidebarOpen: true, ...EXITED }],
    ['写作此章只开创作弹窗', { kind: 'chapter', chapterNumber: 3, write: WRITE },
      { chapterCreationOpen: true, chapterCreationPrefill: { chapterNumber: 3, ...WRITE } }],
  ]

  it.each(cases)('%s', (_name, intent, changed) => {
    navigate(intent)
    expect(layoutFields()).toEqual({ ...START, ...changed })
  })

  it.each([
    ['运行输出', { kind: 'run', facet: 'output' }, { aiPanelOpen: true, rightPanelRunId: null }],
    ['运行任务', { kind: 'run', facet: 'tasks' }, { bottomPanelOpen: true, bottomTab: 'tasks', activeRailItem: 'tasks' }],
  ] as Array<[string, NavigationIntent, Partial<LayoutFields>]>)('从首页打开%s时先回到项目视图', (_name, intent, changed) => {
    useLayoutStore.setState({ sidebarView: 'home', activeRailItem: 'home', immersive: false, preImmersivePanels: null })
    navigate(intent)
    expect(layoutFields()).toEqual({
      ...START,
      sidebarView: 'project',
      activeRailItem: 'project',
      sidebarOpen: true,
      ...EXITED,
      ...changed,
    })
  })

  it('写作参数里不能再带一个章号', () => {
    // @ts-expect-error 章号只由意图的 chapterNumber 给出
    const intent: NavigationIntent = { kind: 'chapter', chapterNumber: 3, write: { ...WRITE, chapterNumber: 4 } }
    navigate(intent)
    expect(useLayoutStore.getState().chapterCreationPrefill?.chapterNumber).toBe(3)
  })
})

describe('旧外壳下阶段与资料意图打开对应标签', () => {
  it.each([
    [{ kind: 'stage', stage: 'setup' }, { id: 'config', type: 'config', name: '小说配置' }],
    [{ kind: 'stage', stage: 'architecture' }, { id: 'world-building-editor', type: 'world-building', name: '故事架构' }],
    [{ kind: 'stage', stage: 'blueprint' }, { id: 'chapter-card-editor', type: 'chapter-card', name: '章节蓝图' }],
    [{ kind: 'resource', page: 'plot' }, { id: 'narrative-thread-editor', type: 'narrative-thread', narrativeThreadView: 'plot-tree' }],
    [{ kind: 'resource', page: 'history' }, { id: 'version-history', type: 'version-history', name: '版本历史' }],
  ] as Array<[NavigationIntent, Record<string, unknown>]>)('%j', (intent, tab) => {
    navigate(intent)
    const { tabs, activeTabId } = useEditorStore.getState()
    expect(tabs).toHaveLength(1)
    const { id, ...fields } = tab
    expect(tabs[0]).toMatchObject({ ...fields, projectKey: PROJECT_PATH })
    expect(tabs[0].id.startsWith(String(id))).toBe(true)
    expect(activeTabId).toBe(tabs[0].id)
  })

  it.each([
    { kind: 'stage' },
    { kind: 'stage', stage: 'manuscript' },
    { kind: 'resource', page: 'knowledge' },
    { kind: 'run', facet: 'output' },
    { kind: 'settings', section: 'llm' },
    { kind: 'chapter', chapterNumber: 1 },
  ] as NavigationIntent[])('%j 不开标签', (intent) => {
    navigate(intent)
    expect(useEditorStore.getState().tabs).toEqual([])
  })

  it('没有打开作品时只改布局，不开标签', () => {
    useProjectStore.setState({ currentProject: null })
    navigate({ kind: 'stage', stage: 'setup' })
    expect(useEditorStore.getState().tabs).toEqual([])
    expect(useLayoutStore.getState().sidebarView).toBe('project')
  })
})
