import { ARCH_STEP_NAMES } from '../../../services/architecture-step-selection'

export interface ArchFile {
  key: string
  fileName: string
  label: string
  iconName: string
  desc: string
}

export const ARCH_FILES: ArchFile[] = [
  { key: 'premise', fileName: 'premise.md', label: ARCH_STEP_NAMES.premise[0], iconName: 'target', desc: 'Logline、核心冲突、金手指定位' },
  { key: 'characters', fileName: 'characters.md', label: ARCH_STEP_NAMES.characters[0], iconName: 'users', desc: '角色弧光、关系网、矛盾交织' },
  { key: 'worldbuilding', fileName: 'worldbuilding.md', label: ARCH_STEP_NAMES.worldbuilding[0], iconName: 'globe', desc: '核心规则、阶层断层、深层危机' },
  { key: 'synopsis', fileName: 'synopsis.md', label: ARCH_STEP_NAMES.synopsis[0], iconName: 'map', desc: '三幕式情节骨架' },
]
