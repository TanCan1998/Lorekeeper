import { useEditorStore } from './editor-store'
import { useProjectStore } from './project-store'

/** 打开内置编辑器 */
export function openBuiltinEditor(
  id: string,
  name: string,
  type: 'chapter-card' | 'character' | 'world-building' | 'version-history' | 'narrative-thread',
  narrativeThreadView?: 'plot-tree' | 'plans',
  chapterNumber?: number,
): void {
  const projectKey = useProjectStore.getState().currentProject?.path
  if (!projectKey) return
  useEditorStore.getState().openFile({
    id,
    name,
    type,
    ...(type === 'narrative-thread'
      ? { narrativeThreadView: narrativeThreadView ?? 'plans' }
      : {}),
    ...(type === 'chapter-card' && chapterNumber !== undefined ? { chapterNumber } : {}),
    projectKey,
  })
}
