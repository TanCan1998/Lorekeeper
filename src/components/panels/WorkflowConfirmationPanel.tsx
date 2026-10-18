import { Play } from 'lucide-react'
import type { WorkflowRun } from '../../stores/workflow-store'
import { CharacterProposalSelectionPanel } from '../characters/CharacterProposalSelectionPanel'
import { useWorkflowConfirmation } from '../run/use-workflow-confirmation'

export function WorkflowConfirmationPanel({ run }: { run: WorkflowRun }) {
  const text = (zh: string, en: string) => run.uiLocale === 'en-US' ? en : zh
  const confirmation = useWorkflowConfirmation(run)
  if (!confirmation) return null
  const { proposal, blocked, ready, nextStepName } = confirmation

  return <div data-testid="workflow-confirmation-panel" className="flex min-h-0 flex-col gap-2 rounded border p-2"
    style={{ backgroundColor: 'rgba(var(--color-accent-rgb), 0.07)', borderColor: 'rgba(var(--color-accent-rgb), 0.25)' }}>
    {proposal && <details className="min-h-5 max-h-48 overflow-y-auto text-xs break-words">
      <summary className="cursor-pointer" style={{ color: 'var(--color-text-secondary)' }}>{proposal.summary}</summary>
      <div className="pt-2 pr-1">
        <CharacterProposalSelectionPanel batch={proposal.batch} choices={proposal.choices}
          identities={proposal.identities} disabled={!ready} onChange={proposal.onChange} />
      </div>
    </details>}
    {blocked && <p role="status" className="flex-shrink-0 text-xs" style={{ color: 'var(--color-text-secondary)' }}>{blocked}</p>}
    <p className="line-clamp-2 flex-shrink-0 break-words text-xs" style={{ color: 'var(--color-text-secondary)' }} title={nextStepName}>
      {text('下一步：', 'Next: ')}{nextStepName}
    </p>
    <button type="button" data-testid="workflow-confirmation-confirm" disabled={!ready} onClick={confirmation.confirm}
      className="writer-primary-button flex w-full flex-shrink-0 items-center justify-center gap-1 rounded px-2.5 py-1 text-xs font-medium disabled:cursor-not-allowed disabled:opacity-50">
      <Play size={10} className="flex-shrink-0" />
      {confirmation.confirmLabel}
    </button>
  </div>
}
