import { useEffect, useState } from 'react'
import type { Locale } from '../../i18n/types'
import type { WorkflowStep } from '../../stores/workflow-store'

/**
 * 审稿等模型整份回复时没有逐字输出，只能报等了多久、最近一次响应与有效输出在多久之前。
 * 主进程给的是收到快照时的耗时，这里每秒在它上面累加本地流逝的时间。
 */
export function useReviewWaitLines(step: WorkflowStep, locale: Locale): string[] {
  const diagnostics = step.generationActivity?.diagnostics
  const [clock, setClock] = useState({ source: diagnostics, elapsed: diagnostics?.elapsedMs ?? 0 })
  useEffect(() => {
    const receivedAt = performance.now()
    const timer = window.setInterval(() => setClock({ source: diagnostics,
      elapsed: (diagnostics?.elapsedMs ?? 0) + performance.now() - receivedAt }), 1000)
    return () => window.clearInterval(timer)
  }, [diagnostics])
  const t = (zh: string, en: string) => locale === 'en-US' ? en : zh
  const elapsed = clock.source === diagnostics ? clock.elapsed : diagnostics?.elapsedMs ?? 0
  const seconds = (ms: number) => `${Math.floor(Math.max(0, ms) / 1000)}${locale === 'en-US' ? 's' : '秒'}`
  const response = diagnostics?.lastResponseMs
  const output = diagnostics?.lastOutputMs
  return [
    t(`已等待 ${seconds(elapsed)}`, `Waited ${seconds(elapsed)}`),
    !diagnostics ? t('响应时间暂不可用。', 'Response timing is unavailable.') : response == null ? t('尚未收到响应。', 'No response received yet.')
      : t(`距最近响应 ${seconds(elapsed - response)}`, `Last response ${seconds(elapsed - response)} ago`),
    !diagnostics ? t('有效输出时间暂不可用。', 'Output timing is unavailable.') : output == null ? t('尚未收到有效输出；连接心跳不代表审稿进度。', 'No output yet; connection heartbeats do not indicate review progress.')
      : t(`距最近有效输出 ${seconds(elapsed - output)}（含思考）`, `Last output ${seconds(elapsed - output)} ago (including reasoning)`),
    t('尚无完整审稿报告。等待期间可随时中止。', 'No complete review report yet. You can stop while waiting.'),
  ]
}
