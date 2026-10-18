import { useEffect, useMemo, useState } from 'react'
import type { Locale } from '../../i18n/types'
import { coarseRuntimePlatform, formatSafeCallDiagnostic, type SafeDiagnosticWorkflow } from '../../services/safe-call-diagnostic'
import { diagnosticWorkflowForCall, type LLMCallRecord, type LLMStats } from '../../services/stats-service'
import type { ProjectSessionContext } from '../../shared/ipc-channels'
import { projectSessionContextFromProject, sameProjectSessionContext } from '../../shared/project-session-context'
import { useProjectStore } from '../../stores/project-store'
import { useWorkflowStore } from '../../stores/workflow-store'
import { LLMDataRequestGate } from '../panels/llm-data-request-gate'
import { toast } from '../ui/Toast'

const REFRESH_MS = 2000
const HISTORY_LIMIT = 30

export interface ModelCalls {
  stats: LLMStats | null
  history: LLMCallRecord[]
  /** 这条调用属于哪次运行；复制安全诊断时附上运行名与步骤。 */
  workflowFor: (call: LLMCallRecord) => SafeDiagnosticWorkflow | undefined
}

/**
 * 这部作品最近的模型调用记录，每 2 秒刷新一次。数据与读取时的会话绑在一起：
 * 旧请求晚到也不会显示到新会话，界面上永远只有当前会话的统计。
 */
export function useModelCalls(): ModelCalls {
  const project = useProjectStore(s => s.currentProject)
  const session = useMemo(() => projectSessionContextFromProject(project), [project])
  const [gate] = useState(() => new LLMDataRequestGate())
  const activeRuns = useWorkflowStore(s => s.activeRuns)
  const history = useWorkflowStore(s => s.history)
  const runs = useMemo(() => [...activeRuns, ...history], [activeRuns, history])
  const [data, setData] = useState<{ session: ProjectSessionContext; stats: LLMStats; history: LLMCallRecord[] } | null>(null)

  useEffect(() => {
    if (!session) {
      gate.invalidate()
      return
    }
    let disposed = false
    const load = () => {
      const ticket = gate.begin(session)
      void import('../../services/stats-service')
        .then(({ loadLLMData }) => loadLLMData(session, HISTORY_LIMIT))
        .then(loaded => {
          if (disposed || !gate.isCurrent(ticket, projectSessionContextFromProject(useProjectStore.getState().currentProject))) return
          setData({ session: ticket.projectSession, ...loaded })
        })
        .catch(() => {
          // 读取失败时保持原样；旧请求也不能反向写入界面。
        })
    }
    load()
    const timer = window.setInterval(load, REFRESH_MS)
    return () => {
      disposed = true
      window.clearInterval(timer)
      gate.invalidate()
    }
  }, [session, gate])

  const visible = data && sameProjectSessionContext(data.session, session) ? data : null
  return {
    stats: visible?.stats ?? null,
    history: visible?.history ?? [],
    workflowFor: call => session ? diagnosticWorkflowForCall(call, runs, session.projectPath) : undefined,
  }
}

/** 复制一条调用的安全诊断（不含正文与密钥），结果用提示告诉作者。 */
export async function copySafeDiagnostic(call: LLMCallRecord, workflow: SafeDiagnosticWorkflow | undefined, locale: Locale): Promise<void> {
  const t = (zh: string, en: string) => locale === 'en-US' ? en : zh
  if (!navigator.clipboard?.writeText) {
    toast.error(t('当前环境不支持复制安全诊断', 'Safe diagnostic copying is unavailable in this environment.'))
    return
  }
  const diagnostic = formatSafeCallDiagnostic({
    locale,
    appVersion: __APP_VERSION__,
    platform: coarseRuntimePlatform(navigator.platform),
    call,
    workflow,
  })
  try {
    await navigator.clipboard.writeText(diagnostic)
    toast.success(t('已复制安全诊断', 'Safe diagnostics copied.'))
  } catch {
    toast.error(t('复制安全诊断失败', 'Could not copy safe diagnostics.'))
  }
}
