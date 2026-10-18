import type { ProjectSessionContext } from '../../shared/ipc-channels'
import { useLocaleStore } from '../../stores/locale-store'
import { useProjectStore } from '../../stores/project-store'
import { captureProjectSession } from '../project-session-gate'
import { Button } from '../ui/Button'
import { CharacterProposalSelectionPanel } from './CharacterProposalSelectionPanel'
import { usePendingCharacterProposals } from './use-pending-character-proposals'

export function FinalizedCharacterProposalPanel() {
  const project = useProjectStore(state => state.currentProject)
  const session = captureProjectSession(project)
  return session ? <SessionPanel key={`${session.projectId}:${session.leaseId}`} session={session} /> : null
}

function SessionPanel({ session }: { session: ProjectSessionContext }) {
  const text = useLocaleStore(state => state.text)
  const { summaries, batch, choices, setChoices, busy, error, identities, read, decide } = usePendingCharacterProposals(session)

  if (!summaries.length && !error) return null
  return <section className="border-b border-[var(--color-border)] p-2 space-y-2" aria-label={text('待处理角色决策', 'Pending character decisions')}>
    {summaries.length > 0 && <>
      <div className="text-xs font-medium">{text(`待处理角色决策（${summaries.length}）`, `Pending character decisions (${summaries.length})`)}</div>
      {!batch && summaries.map(summary => <Button key={summary.proposalBatchId} variant="outline" size="sm" disabled={busy}
        aria-label={summary.importOperationId ? text('检查导入角色决策', 'Review imported character decision') : text('检查定稿角色决策', 'Review finalized character decision')} onClick={() => void read(summary)}>
        {summary.importOperationId ? text('检查导入角色决策', 'Review imported character decision') : text('检查定稿角色决策', 'Review finalized character decision')} · {summary.importOperationId ?? summary.finalizationId}
      </Button>)}
    </>}
    {error && <p role="alert" className="text-xs text-[var(--color-error)] break-all">{error}</p>}
    {batch && choices && <>
      <CharacterProposalSelectionPanel batch={batch} identities={identities} choices={choices} onChange={setChoices} disabled={busy} />
      <div className="flex gap-2">
        <Button variant="outline" size="sm" disabled={busy} onClick={() => void decide('cancel')}>{text('拒绝这些决策', 'Reject decisions')}</Button>
        <Button size="sm" disabled={busy} onClick={() => void decide('approve')}>{text('接受这些决策', 'Accept decisions')}</Button>
      </div>
    </>}
  </section>
}
