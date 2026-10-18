export type ArchStepKey = 'premise' | 'characters' | 'worldbuilding' | 'synopsis'

const ARCH_STEP_ORDER: ArchStepKey[] = ['premise', 'characters', 'worldbuilding', 'synopsis']

/** 架构四节的名字（中、英）。架构页、生成对话框、提交提示与单节文档标签共用这一张；名单页的图形视图叫「关系图谱」，不与名单混名。 */
export const ARCH_STEP_NAMES: Record<ArchStepKey, readonly [string, string]> = {
  premise: ['故事前提', 'Premise'],
  characters: ['角色名单', 'Cast'],
  worldbuilding: ['世界观', 'World'],
  synopsis: ['情节大纲', 'Outline'],
}

export function createDefaultArchitectureSelection(
  archStatus: Record<string, boolean>,
  initialSelectedSteps?: ArchStepKey[],
): Record<ArchStepKey, boolean> {
  const selected = new Set<ArchStepKey>()

  for (const step of ARCH_STEP_ORDER) {
    if (!archStatus[step]) selected.add(step)
  }

  if (initialSelectedSteps) {
    for (const step of initialSelectedSteps) {
      selected.add(step)
    }
  }

  return {
    premise: selected.has('premise'),
    characters: selected.has('characters'),
    worldbuilding: selected.has('worldbuilding'),
    synopsis: selected.has('synopsis'),
  }
}
