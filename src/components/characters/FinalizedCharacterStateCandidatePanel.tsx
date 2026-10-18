import type { ProjectSessionContext } from '../../shared/ipc-channels'
import { useLocaleStore } from '../../stores/locale-store'
import { useProjectStore } from '../../stores/project-store'
import { captureProjectSession } from '../project-session-gate'
import { Button } from '../ui/Button'
import { STATE_FIELD_LABELS, usePendingStateCandidates } from './use-pending-state-candidates'

export function FinalizedCharacterStateCandidatePanel() {
  const project = useProjectStore(state => state.currentProject)
  const session = captureProjectSession(project)
  return session ? <SessionPanel key={`${session.projectId}:${session.leaseId}`} session={session} /> : null
}

function SessionPanel({ session }: { session: ProjectSessionContext }) {
  const text = useLocaleStore(state => state.text)
  const { summaries, candidate, current, busy, error, read, decide } = usePendingStateCandidates(session)

  if (!summaries.length && !error) return null
  return <section className="border-b border-[var(--color-border)] p-2 space-y-2"
    aria-label={text('待处理角色状态建议', 'Pending state suggestions')}>
    {summaries.length > 0 && <>
      <div className="text-xs font-medium">{text(`待处理角色状态建议（${summaries.length}）`, `Pending state suggestions (${summaries.length})`)}</div>
      {!candidate && summaries.map(summary => <Button key={summary.candidateKey} variant="outline" size="sm" disabled={busy}
        aria-label={text('检查定稿状态建议', 'Review finalized state suggestion')} onClick={() => void read(summary)}>
        {summary.characterName} · {text(...STATE_FIELD_LABELS[summary.field])}
      </Button>)}
    </>}
    {error && <p role="alert" className="text-xs text-[var(--color-error)] break-all">{error}</p>}
    {candidate && <>
      <div className="text-xs space-y-1">
        <div className="font-medium">{candidate.characterName} · {text(...STATE_FIELD_LABELS[candidate.field])}</div>
        <div>{text('当前：', 'Current: ')}{current || text('空', 'Empty')}</div>
        <div>{text('建议：', 'Suggested: ')}{candidate.value}</div>
      </div>
      <div className="flex gap-2">
        <Button variant="outline" size="sm" disabled={busy} onClick={() => void decide('decline')}>
          {text('拒绝状态建议', 'Reject state suggestion')}
        </Button>
        <Button size="sm" disabled={busy} onClick={() => void decide('accept')}>
          {text('接受状态建议', 'Accept state suggestion')}
        </Button>
      </div>
    </>}
  </section>
}
