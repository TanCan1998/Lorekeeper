import type { CSSProperties } from 'react'

interface LifecycleProps {
  /** 生命周期整体的可访问名称，例如「第 4 章进度」。 */
  label: string
  /** 按顺序排列的步骤名，章节为写作、审稿、修稿、定稿。 */
  steps: readonly string[]
  /** 当前步骤下标；等于 steps.length 表示全部完成。 */
  current: number
}

/** 章节生命周期：已完成为墨色，当前为黛蓝并标 aria-current，未开始为浅线与说明色文字。 */
export function Lifecycle({ label, steps, current }: LifecycleProps) {
  return (
    <ol className="pi-life" aria-label={label} style={{ '--pi-life-steps': steps.length } as CSSProperties}>
      {steps.map((step, index) => {
        const state = index < current ? 'done' : index === current ? 'current' : 'todo'
        return (
          <li key={step} className="pi-life-step" data-state={state} aria-current={state === 'current' ? 'step' : undefined}>
            <span className="pi-life-bar" aria-hidden />
            {step}
          </li>
        )
      })}
    </ol>
  )
}
