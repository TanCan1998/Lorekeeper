import { useEffect, useState } from 'react'
import { Check } from 'lucide-react'
import { sameProjectPathKey } from '../../shared/project-session-context'
import { useLocaleStore } from '../../stores/locale-store'
import { navigate } from '../../stores/navigation/navigation-intents'
import { useStageStore } from '../../stores/navigation/stage-store'
import { useProjectStore } from '../../stores/project-store'
import { useWorkflowStore } from '../../stores/workflow-store'
import { runProgress } from '../../stores/navigation/stage-progress'
import { ProgressRing } from './ProgressRing'

const DONE_VISIBLE_MS = 1800

/**
 * 顶栏运行胶囊：这部作品有运行时显示进度；没有运行时，若这部作品最近一次结束的运行是完成、且结束不到 1.8 秒，显示「运行已完成」。
 * 完成态由这次运行的结束时间推出，接着又有运行、运行失败或换了作品时自然不再显示。
 */
export default function RunCapsule() {
  const text = useLocaleStore(s => s.text)
  const projectPath = useProjectStore(s => s.currentProject?.path)
  const allRuns = useWorkflowStore(s => s.activeRuns)
  const lastEnded = useWorkflowStore(s => projectPath ? s.history.find(run => sameProjectPathKey(run.projectPath, projectPath)) : undefined)
  const sheetOpen = useStageStore(s => s.sheet?.kind === 'run')
  const runs = projectPath ? allRuns.filter(run => sameProjectPathKey(run.projectPath, projectPath)) : []
  const doneUntil = lastEnded?.status === 'completed' && lastEnded.completedAt ? Date.parse(lastEnded.completedAt) + DONE_VISIBLE_MS : 0

  const toggle = (runId?: string) => {
    if (sheetOpen) useStageStore.setState({ sheet: null })
    else navigate({ kind: 'run', facet: 'output', runId })
  }

  if (runs.length === 0) {
    return doneUntil ? <DoneCapsule key={doneUntil} until={doneUntil} expanded={sheetOpen} onToggle={() => toggle()} /> : null
  }

  const waiting = runs.filter(run => run.status === 'waiting').length
  if (runs.length > 1) {
    return (
      <button type="button" className="stage-capsule" aria-expanded={sheetOpen} onClick={() => toggle()}>
        <ProgressRing size={18} stroke={2} ratio={runs.reduce((sum, run) => sum + runProgress(run), 0) / runs.length} />
        {waiting > 0
          ? text(`${runs.length} 项运行中 · ${waiting} 项等你确认`, `${runs.length} running · ${waiting} need you`)
          : text(`${runs.length} 项运行中`, `${runs.length} running`)}
      </button>
    )
  }

  const [run] = runs
  const step = run.steps[run.currentStepIndex]
  const percent = Math.round(runProgress(run) * 100)
  return (
    <button type="button" className="stage-capsule" data-tone={waiting ? 'warn' : undefined} aria-expanded={sheetOpen} onClick={() => toggle(run.id)}>
      <ProgressRing size={18} stroke={2} ratio={percent / 100} />
      <span className="stage-capsule-label">{waiting ? text(`${run.title} · 等你确认`, `${run.title} · needs you`) : step && !run.title.includes(step.name) ? `${run.title} · ${step.name}` : run.title}</span>
      <b className="stage-capsule-value">{percent}%</b>
    </button>
  )
}

/** 按 until 重新挂载：挂载时已过期（这条完成运行进入历史时就晚了）直接不显示，否则到点收起。 */
function DoneCapsule({ until, expanded, onToggle }: { until: number; expanded: boolean; onToggle: () => void }) {
  const text = useLocaleStore(s => s.text)
  const [visible, setVisible] = useState(() => until > Date.now())

  useEffect(() => {
    if (!visible) return
    const timer = setTimeout(() => setVisible(false), until - Date.now())
    return () => clearTimeout(timer)
  }, [until, visible])

  if (!visible) return null
  return (
    <button type="button" className="stage-capsule" data-tone="ok" aria-expanded={expanded} onClick={onToggle}>
      <Check size={14} aria-hidden="true" />
      {text('运行已完成', 'Run complete')}
    </button>
  )
}
