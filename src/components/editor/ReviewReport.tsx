import { useState } from 'react'
import {
  AlertTriangle,
  CheckCircle,
  CircleMinus,
  FileCheck2,
  HelpCircle,
  Info,
  ListChecks,
  Pencil,
  Plus,
  Quote,
  RotateCcw,
  Sparkles,
  X,
} from 'lucide-react'
import { cn } from '../../lib/utils'
import { Button } from '../ui/Button'
import {
  Dialog, DialogContent, DialogHeader, DialogFooter, DialogTitle, DialogDescription,
} from '../ui/Dialog'
import { Input } from '../ui/Input'
import { Label } from '../ui/Label'
import { NativeSelect } from '../ui/NativeSelect'
import { Textarea } from '../ui/Textarea'
import { useProjectStore } from '../../stores/project-store'
import { useLocaleStore } from '../../stores/locale-store'
import { useLLMStore } from '../../stores/llm-store'
import { availableGenerationModelId, isGenerationModel, preferredGenerationModelId } from '../../services/workflows/chapter-write-launch'
import { resolveWritingLanguage, type WritingLanguage } from '../../shared/writing-language'
import type { ReviewCycleProjection } from '../../shared/review-cycle'
import {
  hasIncludedReviewItems,
  renderHumanConfirmedReviewBrief,
  type HumanConfirmedReviewSnapshot,
} from '../../shared/human-confirmed-review'
import {
  normalizeSeverity,
  severityCopy,
  useReviewChecklist,
  useReviewCycle,
  type EditableReviewItem,
  type ReviewChecklistInput,
  type ReviewIssue,
} from './review-checklist'

interface ReviewReportProps {
  /** 原始审稿报告文本（JSON 或旧版 markdown） */
  reportText: string
  /** 审稿报告关联的草稿路径（用于触发修稿） */
  draftPath?: string
  /** 章节号 */
  chapterNumber?: number
  /** 章节目录 */
  chapterDir?: string
  /** 原始 AI 审稿报告的数据库 ID；确认快照必须持续指向该记录。 */
  reviewId?: number
  /** 审稿与草稿所属项目。 */
  projectKey: string
  /** Optional persisted cycle state; merge committed is deliberately not a resolved finding. */
  cycle?: ReviewCycleProjection | null
}


// ===== 视觉配置 =====

const SEVERITY_META: Record<ReviewIssue['severity'], {
  colorClass: string
  bgClass: string
  borderClass: string
}> = {
  unknown: {
    colorClass: 'text-[var(--color-text-muted)]',
    bgClass: 'bg-[var(--color-bg-elevated)]',
    borderClass: 'border-[var(--color-border)]',
  },
  error: {
    colorClass: 'text-[var(--color-error-text)]',
    bgClass: 'bg-red-500/10',
    borderClass: 'border-red-500/30',
  },
  warning: {
    colorClass: 'text-[var(--color-warning-text)]',
    bgClass: 'bg-yellow-500/10',
    borderClass: 'border-yellow-500/30',
  },
  pass: {
    colorClass: 'text-[var(--color-success-text)]',
    bgClass: 'bg-green-500/10',
    borderClass: 'border-green-500/30',
  },
}

function SeverityIcon({ severity }: { severity: ReviewIssue['severity'] }) {
  if (severity === 'unknown') return <HelpCircle size={14} className="flex-shrink-0 text-[var(--color-text-muted)]" />
  if (severity === 'error') return <AlertTriangle size={14} className="flex-shrink-0" style={{ color: 'var(--color-error)' }} />
  if (severity === 'warning') return <AlertTriangle size={14} className="flex-shrink-0" style={{ color: 'var(--color-warning)' }} />
  return <CheckCircle size={14} className="flex-shrink-0" style={{ color: 'var(--color-success)' }} />
}

/** 审稿报告查看器 */
export default function ReviewReport(props: ReviewReportProps) {
  const { snapshot, cycle, reloadCycle, sessionKey } = useReviewCycle(props)
  return <ReviewReportSession key={sessionKey} {...props} cycle={cycle}
    initialSnapshot={snapshot} reloadCycle={reloadCycle} />
}

function ReviewReportSession(props: ReviewChecklistInput) {
  const { reportText, projectKey } = props
  const text = useLocaleStore(s => s.text)
  const writingLanguage = useProjectStore(s => resolveWritingLanguage(
    s.currentProject?.path === projectKey
      ? s.currentProject.novelConfig.writingLanguage
      : undefined,
  ))
  const {
    items, authorGuidance, setAuthorGuidance, confirmed, editingChecklist, setEditingChecklist,
    checklistError, setChecklistError, confirming, processing, rechecking, recheckMessage,
    summary, goalReview, canManageChecklist, cycle,
    updateItem, removeItem, addAuthorItem, confirmChecklist, startConfirmedRevision, retryRequiredRecheck,
  } = useReviewChecklist(props)
  const initialSnapshot = props.initialSnapshot
  const [showRevisionDialog, setShowRevisionDialog] = useState(false)
  const [showLegend, setShowLegend] = useState(false)

  // 按分类分组
  const categories = new Map<string, EditableReviewItem[]>()
  for (const item of items) {
    const list = categories.get(item.category) || []
    list.push(item)
    categories.set(item.category, list)
  }

  // 统计
  const errorCount = items.filter((i) => i.severity === 'error').length
  const warningCount = items.filter((i) => i.severity === 'warning').length
  const passCount = items.filter((i) => i.severity === 'pass').length
  const unknownCount = items.filter((i) => i.severity === 'unknown').length
  const errorCopy = severityCopy('error', text)
  const warningCopy = severityCopy('warning', text)
  const passCopy = severityCopy('pass', text)

  return (
    <div className="h-full overflow-y-auto">
      <div className="max-w-2xl mx-auto px-6 py-4">
        {/* 统计栏 */}
        <div className="flex items-center gap-4 mb-4 pb-3 border-b border-[var(--color-border)]">
          <h3 className="text-base font-bold text-[var(--color-text)]">{text('审稿报告', 'Review report')}</h3>
          {cycle && (
            <span
              data-review-cycle-status={cycle.revisionStatus}
              className="rounded border border-[var(--color-border)] px-2 py-0.5 text-xs text-[var(--color-text-secondary)]"
            >
              {cycle.revisionStatus === 'generated'
                ? text('修稿已生成', 'Revision generated')
                : cycle.revisionStatus === 'merge-committed'
                  ? text('合并已提交', 'Merge committed')
                  : text('尚未生成修稿', 'Revision not generated')}
            </span>
          )}
          <div className="flex items-center gap-3 text-xs ml-auto">
            {errorCount > 0 && (
              <span className="flex items-center gap-1 px-2 py-0.5 rounded bg-red-500/20 text-[var(--color-error-text)]">
                <SeverityIcon severity="error" /> {errorCount} {errorCopy.countLabel}
              </span>
            )}
            {warningCount > 0 && (
              <span className="flex items-center gap-1 px-2 py-0.5 rounded bg-yellow-500/20 text-[var(--color-warning-text)]">
                <SeverityIcon severity="warning" /> {warningCount} {warningCopy.countLabel}
              </span>
            )}
            {unknownCount > 0 && (
              <span className="flex items-center gap-1 text-[var(--color-text-muted)]">
                <SeverityIcon severity="unknown" /> {unknownCount} {text('待核实', 'unverified')}
              </span>
            )}
            <span className="flex items-center gap-1 px-2 py-0.5 rounded bg-green-500/20 text-[var(--color-success-text)]">
              <SeverityIcon severity="pass" /> {passCount} {passCopy.countLabel}
            </span>
            {/* 图例帮助按钮 */}
            <button
              className="flex items-center justify-center rounded-full hover:bg-[var(--color-hover)] transition-colors"
              style={{ width: 22, height: 22 }}
              onClick={() => setShowLegend(!showLegend)}
              title={text('颜色说明', 'Color legend')}
            >
              <HelpCircle size={14} style={{ color: 'var(--color-text-muted)' }} />
            </button>
          </div>
        </div>

        {/* 颜色图例说明 */}
        {showLegend && (
          <div
            className="mb-4 rounded-lg border p-3 text-xs space-y-2"
            style={{
              backgroundColor: 'var(--color-bg-elevated)',
              borderColor: 'var(--color-border)',
            }}
          >
            <div className="font-medium text-[var(--color-text)] mb-1.5">{text('颜色标记说明', 'Color legend')}</div>
            {(['error', 'warning', 'unknown', 'pass'] as const).map(sev => {
              const meta = SEVERITY_META[sev]
              const copy = severityCopy(sev, text)
              return (
                <div key={sev} className="flex items-center gap-2">
                  <span className={cn(
                    'inline-flex items-center gap-1 px-2 py-0.5 rounded',
                    meta.bgClass, meta.colorClass
                  )}>
                    <SeverityIcon severity={sev} /> {copy.label}
                  </span>
                  <span style={{ color: 'var(--color-text-secondary)' }}>
                    — {copy.actionLabel}
                  </span>
                </div>
              )
            })}
          </div>
        )}

        {/* 总体评价（如有） */}
        {summary && (
          <div
            className="mb-4 px-4 py-3 rounded-lg border text-sm"
            style={{
              backgroundColor: 'var(--color-bg-elevated)',
              borderColor: 'var(--color-border)',
              color: 'var(--color-text)',
            }}
          >
            <span className="font-medium">{text('总体评价：', 'Overall assessment:')}</span>
            <span style={{ color: 'var(--color-text-secondary)' }}>{summary}</span>
          </div>
        )}

        {goalReview && (
          <p className="mb-4 text-xs text-[var(--color-text-muted)]">
            {goalReview.coverage === 'complete'
              ? text('本章目标已逐项检查；覆盖完整不代表全部完成。', 'Chapter goals checked individually; full coverage does not mean all goals are completed.')
              : goalReview.coverage === 'not_configured'
                ? text('本章未配置可检查的目标，未进行目标验收。', 'No chapter goals are configured; goal acceptance was not performed.')
                : text('本章目标检查不完整，尚有待核实项。', 'Chapter goal review is incomplete and needs verification.')}
          </p>
        )}
        {cycle?.revisionStatus === 'merge-committed' && cycle.recheckDisposition === 'required' && (
          <section className="mb-4 rounded-lg border border-[var(--color-warning-border)] p-3 text-xs">
            <p className="text-[var(--color-warning-text)]">
              {text(
                '合并已持久化，但定向复验尚未完成。关闭或重开后仍可从这里继续。',
                'The merge is durable, but the targeted recheck is incomplete. You can resume it here after reopening.',
              )}
            </p>
            <Button className="mt-2" variant="outline" size="sm" onClick={retryRequiredRecheck} disabled={rechecking}>
              <RotateCcw size={12} />
              {rechecking ? text('正在复验…', 'Rechecking…') : text('重试合并后复验', 'Retry post-merge recheck')}
            </Button>
          </section>
        )}
        {recheckMessage && (
          <p role={recheckMessage.kind === 'error' ? 'alert' : 'status'}
            className={cn('mb-4 text-xs', recheckMessage.kind === 'error'
              ? 'text-[var(--color-error-text)]' : 'text-[var(--color-success-text)]')}>
            {recheckMessage.text}
          </p>
        )}
        {/* 分类展示 */}
        {items.length === 0 ? (
          <div className="text-center py-8 text-[var(--color-text-muted)] text-sm">
            <CheckCircle size={32} className="mx-auto mb-2" style={{ color: 'var(--color-success)' }} />
            {goalReview && goalReview.coverage !== 'complete'
              ? text('暂无已确认的检查结果', 'No confirmed review results yet')
              : text('审稿通过，未发现问题', 'Review passed. No issues found.')}
          </div>
        ) : (
          <div className="space-y-4">
            {Array.from(categories.entries()).map(([category, items]) => (
              <div key={category}>
                <h4 className="text-sm font-semibold text-[var(--color-text)] mb-2 flex items-center gap-1.5">
                  <Info size={14} className="text-[var(--color-text-muted)]" />
                  {category}
                </h4>
                <div className="space-y-1.5 pl-1">
                  {items.map((item) => {
                    const meta = SEVERITY_META[item.severity]
                    const copy = severityCopy(item.severity, text)
                    const isPass = item.severity === 'pass'
                    const goal = goalReview?.items.find(goal => goal.id === item.goalId)
                    const isEmptyAuthorIssue = item.origin === 'author' && !item.description.trim()
                    const projectedFinding = cycle?.findings.find(finding => (
                      item.findingId
                        ? finding.findingId === item.findingId
                        : finding.reviewItemIndex === item.reviewItemIndex
                    ))
                    const findingId = item.findingId ?? projectedFinding?.findingId
                    const findingStatus = item.decision === 'waive'
                      ? 'author-waived'
                      : projectedFinding?.status ?? item.findingStatus
                    const findingStatusLabel = findingStatus === 'resolved'
                      ? text('已解决', 'Resolved')
                      : findingStatus === 'unresolved'
                        ? text('未解决', 'Unresolved')
                        : findingStatus === 'unknown'
                          ? text('待核实', 'Unknown')
                          : findingStatus === 'author-waived'
                            ? text('作者选择不修改', 'Author chose not to revise')
                            : findingStatus === 'unverified'
                              ? text('未验证', 'Unverified')
                              : null
                    return (
                      <div
                        key={item.id}
                        className={cn(
                          'px-3 py-2 rounded-md border text-xs leading-relaxed',
                          meta.borderClass, meta.bgClass
                        )}
                      >
                        <div className="flex items-start gap-2">
                          <SeverityIcon severity={item.severity} />
                          <div className="flex-1 min-w-0 space-y-2">
                            {findingStatusLabel && (
                              <span
                                data-review-finding-status={findingStatus}
                                className="inline-flex rounded border border-[var(--color-border)] px-2 py-0.5 text-[0.7rem] text-[var(--color-text-secondary)]"
                              >
                                {findingStatusLabel}
                              </span>
                            )}
                            {goal && (
                              <div className="space-y-1" aria-label={text('本章目标与证据', 'Chapter goal and evidence')}>
                                <p className="font-medium">
                                  {text('本章目标：', 'Chapter goal: ')}{goal.text}
                                  {' — '}{goal.status === 'completed' ? text('已完成', 'Completed') : goal.status === 'unmet' ? text('未完成', 'Unmet') : text('待核实', 'Needs verification')}
                                </p>
                                {goal.evidence.map((evidence, index) => (
                                  <blockquote key={index} className="border-l-2 border-[var(--color-border)] pl-2">
                                    <Quote size={10} className="inline mr-1" />{evidence.quote}
                                  </blockquote>
                                ))}
                                {goal.evidence.length === 0 && <p>{text('暂无可定位正文证据', 'No locatable chapter evidence')}</p>}
                              </div>
                            )}
                            {editingChecklist && !isPass ? (
                              <>
                                <div className="grid grid-cols-[minmax(0,1fr)_130px] gap-2">
                                  <Input
                                    aria-label={text('问题分类', 'Issue category')}
                                    value={item.category}
                                    onChange={(event) => updateItem(item.id, { category: event.target.value })}
                                  />
                                  <NativeSelect
                                    aria-label={text('严重程度', 'Severity')}
                                    value={item.severity}
                                    onChange={(event) => {
                                      const severity = normalizeSeverity(event.target.value)
                                      updateItem(item.id, {
                                        severity,
                                        ...(severity === 'unknown' ? { decision: 'ignore' } : {}),
                                      })
                                    }}
                                  >
                                    <option value="error">{text('严重问题', 'Critical issue')}</option>
                                    <option value="warning">{text('改进建议', 'Improvement')}</option>
                                    <option value="unknown">{text('待核实', 'Needs verification')}</option>
                                  </NativeSelect>
                                </div>
                                <Textarea
                                  aria-label={text('审稿问题', 'Review issue')}
                                  aria-invalid={isEmptyAuthorIssue || undefined}
                                  aria-describedby={isEmptyAuthorIssue ? `author-issue-hint-${item.id}` : undefined}
                                  value={item.description}
                                  placeholder={item.origin === 'author' ? text(
                                    '请填写需要纳入本次修稿的具体问题',
                                    'Describe the specific issue to include in this revision',
                                  ) : undefined}
                                  onChange={(event) => updateItem(item.id, { description: event.target.value })}
                                />
                                {isEmptyAuthorIssue && (
                                  <p
                                    id={`author-issue-hint-${item.id}`}
                                    className="text-[0.7rem] text-[var(--color-warning-text)]"
                                  >
                                    {text(
                                      '请填写具体问题，或移除这一项。',
                                      'Describe the issue, or remove this item.',
                                    )}
                                  </p>
                                )}
                                <Input
                                  aria-label={text('相关原文（可选）', 'Related text (optional)')}
                                  value={item.quote ?? ''}
                                  placeholder={text('相关原文（可选）', 'Related text (optional)')}
                                  onChange={(event) => updateItem(item.id, { quote: event.target.value })}
                                />
                              </>
                            ) : (
                              <div>
                                <span className="text-[var(--color-text-secondary)]">{item.description}</span>
                                <span className={cn('ml-2 text-[0.65rem] opacity-70', meta.colorClass)}>
                                  [{copy.actionLabel}]
                                </span>
                              </div>
                            )}
                            {item.sourceChapter && (
                              <p className="text-[0.7rem] text-[var(--color-text-muted)]">
                                {text(
                                  `来源：第${item.sourceChapter}章`,
                                  `Source: Chapter ${item.sourceChapter}`,
                                )}
                              </p>
                            )}
                            {!isPass && editingChecklist && (
                              <div className="flex items-center justify-between gap-2">
                                <span className={cn('text-[0.7rem]', meta.colorClass)}>
                                  {item.decision === 'apply'
                                    ? text('已纳入本次修稿', 'Included in this revision')
                                    : item.decision === 'waive'
                                      ? text('本项不纳入修稿，可在编辑清单时重新纳入。', 'This item is excluded from revision. You can include it again when editing the checklist.')
                                      : text('已忽略，不会传给模型', 'Ignored; not sent to the model')}
                                </span>
                                <div className="flex items-center gap-1">
                                  <Button
                                    variant="outline"
                                    size="sm"
                                    onClick={() => updateItem(item.id, {
                                      decision: item.decision === 'apply' ? 'ignore' : 'apply',
                                    })}
                                  >
                                    {item.decision === 'apply'
                                      ? <CircleMinus size={12} />
                                      : <RotateCcw size={12} />}
                                    {item.decision === 'apply'
                                      ? text('忽略', 'Ignore')
                                      : item.goalId || item.severity === 'unknown' ? text('明确纳入修稿', 'Explicitly include in revision') : text('恢复', 'Restore')}
                                  </Button>
                                  {item.origin === 'ai' && findingId && findingStatus !== 'resolved' && (
                                    <Button
                                      variant="outline"
                                      size="sm"
                                      title={item.decision === 'waive'
                                        ? text('重新选择后，请决定是否将本项纳入修稿。', 'After choosing again, decide whether to include this item in revision.')
                                        : text('本项不纳入修稿，可在编辑清单时重新纳入。', 'This item is excluded from revision. You can include it again when editing the checklist.')}
                                      onClick={() => updateItem(item.id, {
                                        decision: item.decision === 'waive' ? 'ignore' : 'waive',
                                      })}
                                    >
                                      <FileCheck2 size={12} />
                                      {item.decision === 'waive'
                                        ? text('重新选择', 'Choose again')
                                        : text('不修改此项', 'Do not revise this item')}
                                    </Button>
                                  )}
                                  {item.origin === 'author' && (
                                    <Button
                                      variant="ghost"
                                      size="sm"
                                      onClick={() => removeItem(item.id)}
                                      title={text('移除人工问题', 'Remove author issue')}
                                    >
                                      <X size={12} />
                                      {text('移除', 'Remove')}
                                    </Button>
                                  )}
                                </div>
                              </div>
                            )}
                          </div>
                        </div>
                        {/* 引用原文（如有） */}
                        {!editingChecklist && item.quote && (
                          <div
                            className="mt-1.5 ml-5 pl-2 text-[0.7rem] italic"
                            style={{
                              borderLeft: '2px solid var(--color-border)',
                              color: 'var(--color-text-muted)',
                            }}
                          >
                            <Quote size={10} className="inline mr-1 opacity-60" />
                            {item.quote}
                          </div>
                        )}
                      </div>
                    )
                  })}
                </div>
              </div>
            ))}
          </div>
        )}

        <section className="mt-6 rounded-lg border border-[var(--color-border)] p-4 space-y-3">
          <div className="flex items-center justify-between gap-3">
            <div>
              <h4 className="text-sm font-semibold text-[var(--color-text)] flex items-center gap-1.5">
                <ListChecks size={15} className="text-[var(--color-accent)]" />
                {text('人工确认修稿清单', 'Human-confirmed revision checklist')}
              </h4>
              <p className="mt-1 text-xs text-[var(--color-text-muted)]">
                {editingChecklist
                  ? text('普通错误和建议默认纳入；本章目标与待核实项默认忽略，只有明确选择才纳入；“通过”仅展示。', 'General issues and suggestions are included by default; chapter goals and unverified items require explicit inclusion; passed checks are display-only.')
                  : text('已保存为新的不可变确认快照；原始 AI 审稿未被修改。', 'Saved as a new immutable confirmation snapshot; the original AI review was not modified.')}
              </p>
            </div>
            {confirmed && !editingChecklist && (
              <span className="inline-flex items-center gap-1 text-xs text-[var(--color-success-text)]">
                <FileCheck2 size={14} />
                {text('已确认', 'Confirmed')}
              </span>
            )}
          </div>

          {editingChecklist && (
            <>
              <div>
                <Label htmlFor="review-author-guidance">
                  {text('总体修稿指导（可选）', 'Overall revision guidance (optional)')}
                </Label>
                <Textarea
                  id="review-author-guidance"
                  value={authorGuidance}
                  onChange={(event) => setAuthorGuidance(event.target.value)}
                  placeholder={text(
                    '例如：优先修复角色动机的前后不一致，保持本章克制的叙事节奏。',
                    'For example: prioritize inconsistent character motivation while keeping this chapter’s restrained pace.',
                  )}
                />
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <Button variant="outline" size="sm" onClick={addAuthorItem}>
                  <Plus size={13} />
                  {text('新增人工问题', 'Add author issue')}
                </Button>
                {canManageChecklist ? (
                  <Button variant="ai" size="sm" onClick={() => void confirmChecklist()} disabled={confirming}>
                    <FileCheck2 size={13} />
                    {confirmed
                      ? text('重新确认审稿清单', 'Confirm updated checklist')
                      : text('确认审稿清单', 'Confirm review checklist')}
                  </Button>
                ) : (
                  <span className="text-xs text-[var(--color-text-muted)]">
                    {text('关联草稿后可确认清单。', 'Link this report to a draft to confirm the checklist.')}
                  </span>
                )}
              </div>
            </>
          )}

          {confirmed && !editingChecklist && (
            <div className="flex flex-wrap items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => {
                  setEditingChecklist(true)
                  setChecklistError(null)
                }}
              >
                <Pencil size={13} />
                {text('编辑清单', 'Edit checklist')}
              </Button>
              <Button
                variant="ai"
                size="sm"
                onClick={() => {
                  if (!hasIncludedReviewItems(confirmed.snapshot)) {
                    setChecklistError(text(
                      '未纳入任何审稿项，无法启动修稿。请恢复至少一项错误或建议后重新确认。',
                      'No review item is included. Restore at least one issue or suggestion, then confirm again before revising.',
                    ))
                    return
                  }
                  setChecklistError(null)
                  setShowRevisionDialog(true)
                }}
                disabled={processing}
              >
                <Sparkles size={13} />
                {text('按确认意见修稿', 'Revise from confirmed checklist')}
              </Button>
            </div>
          )}

          {checklistError && (
            <p role="alert" className="text-xs text-[var(--color-error-text)]">
              {checklistError}
            </p>
          )}
        </section>

        {/* 原始文本折叠 */}
        <details className="mt-6">
          <summary className="text-xs text-[var(--color-text-muted)] cursor-pointer hover:text-[var(--color-text)]">
            {text(
              initialSnapshot ? '查看此确认记录原文' : '查看原始 AI 审稿文本',
              initialSnapshot ? 'View this confirmation record' : 'View original AI review text',
            )}
          </summary>
          <pre className="mt-2 text-xs whitespace-pre-wrap font-mono leading-5 text-[var(--color-text-secondary)] bg-[var(--color-sidebar)] rounded-md p-3 border border-[var(--color-border)]">
            {reportText}
          </pre>
        </details>
      </div>

      {showRevisionDialog && confirmed && (
        <ConfirmedRevisionDialog
          snapshot={confirmed.snapshot}
          writingLanguage={writingLanguage}
          onClose={() => setShowRevisionDialog(false)}
          onStart={modelId => startConfirmedRevision(modelId)}
        />
      )}
    </div>
  )
}

interface ConfirmedRevisionDialogProps {
  snapshot: HumanConfirmedReviewSnapshot
  writingLanguage: WritingLanguage
  onClose: () => void
  onStart: (generationModelId: string) => Promise<string | null>
}

/**
 * Mounted only while open so every revision gets a fresh local snapshot of the
 * global default model. The chooser never writes to the global default.
 */
function ConfirmedRevisionDialog({ snapshot, writingLanguage, onClose, onStart }: ConfirmedRevisionDialogProps) {
  const text = useLocaleStore(s => s.text)
  const models = useLLMStore(s => s.models)
  const defaultModelId = useLLMStore(s => s.defaultModelId)
  const [generationModelId, setGenerationModelId] = useState<string | null>(() => (
    preferredGenerationModelId(models, defaultModelId)
  ))
  const [starting, setStarting] = useState(false)
  const [startError, setStartError] = useState<string | null>(null)
  const generationModels = models.filter(isGenerationModel)
  const fallbackGenerationModelId = preferredGenerationModelId(models, defaultModelId)
  const selectedGenerationModelId = generationModelId ?? fallbackGenerationModelId
  const selectedGenerationModel = generationModels.find(model => model.id === selectedGenerationModelId)
  const modelSelectionError = generationModels.length === 0
    ? text(
      '没有已配置且可用于文本生成的模型。请在设置中添加或启用一项生成模型。',
      'No configured model can generate text. Add or enable a generation model in Settings.',
    )
    : !selectedGenerationModel
      ? text(
        '所选修稿模型已不可用。请选择一项可用于文本生成的模型后再试。',
        'The selected revision model is no longer available. Select a compatible generation model and try again.',
      )
      : null
  const confirmedBrief = renderHumanConfirmedReviewBrief(snapshot, writingLanguage)

  const start = async () => {
    if (modelSelectionError || !selectedGenerationModelId) {
      setStartError(modelSelectionError ?? text(
        '请选择一项可用于文本生成的模型。',
        'Select a compatible generation model before starting.',
      ))
      return
    }

    const frozenGenerationModelId = availableGenerationModelId(
      useLLMStore.getState().models,
      selectedGenerationModelId,
    )
    if (!frozenGenerationModelId) {
      setStartError(text(
        '所选修稿模型已不可用。请选择一项可用于文本生成的模型后再试。',
        'The selected revision model is no longer available. Select a compatible generation model and try again.',
      ))
      return
    }

    setStarting(true)
    try {
      const error = await onStart(frozenGenerationModelId)
      if (error) {
        setStartError(error)
        return
      }
      onClose()
    } catch (error) {
      setStartError(error instanceof Error
        ? error.message
        : text('启动审稿修稿时发生错误。', 'An error occurred while starting the review-driven revision.'))
    } finally {
      setStarting(false)
    }
  }

  return (
    <Dialog open onOpenChange={(open) => {
      if (!open && !starting) onClose()
    }}>
      <DialogContent className="max-w-[520px]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Sparkles size={15} className="text-[var(--color-accent)]" />
            {text('按确认意见修稿', 'Revise from confirmed checklist')}
          </DialogTitle>
          <DialogDescription>
            {text(
              '仅将已确认纳入的审稿项和作者指导传给修稿流程；已忽略项与原始 AI 报告不会成为模型指令。',
              'Only confirmed review items and author guidance are sent to revision. Ignored items and the raw AI report are not model instructions.',
            )}
          </DialogDescription>
        </DialogHeader>
        <div className="px-5 py-2 space-y-3">
          <div>
            <Label htmlFor="review-revision-model">
              {text('本次修稿模型', 'Model for this revision')}
            </Label>
            <NativeSelect
              id="review-revision-model"
              value={selectedGenerationModelId ?? ''}
              onChange={(event) => {
                setGenerationModelId(event.target.value || null)
                setStartError(null)
              }}
              disabled={generationModels.length === 0 || starting}
            >
              <option value="" disabled>{text('请选择可用生成模型', 'Select a generation model')}</option>
              {generationModels.map(model => (
                <option key={model.id} value={model.id}>{model.name || model.modelName}</option>
              ))}
            </NativeSelect>
            <p className="mt-1 text-[0.7rem] text-[var(--color-text-muted)]">
              {text('仅用于本次修稿，不会更改默认模型。', 'Used for this revision only; it does not change the default model.')}
            </p>
            {modelSelectionError && (
              <p role="alert" className="mt-1 text-xs text-[var(--color-error-text)]">
                {modelSelectionError}
              </p>
            )}
          </div>
          <div className="rounded-md border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-3">
            <div className="mb-1 text-xs font-medium text-[var(--color-text)]">
              {text('将传给修稿流程的已确认意见', 'Confirmed guidance sent to revision')}
            </div>
            <pre className="max-h-40 overflow-y-auto whitespace-pre-wrap font-sans text-xs leading-5 text-[var(--color-text-secondary)]">
              {confirmedBrief}
            </pre>
          </div>
          {startError && (
            <p role="alert" className="text-xs text-[var(--color-error-text)]">
              {startError}
            </p>
          )}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose} disabled={starting}>
            {text('取消', 'Cancel')}
          </Button>
          <Button variant="ai" onClick={start} disabled={starting || Boolean(modelSelectionError)}>
            <Sparkles size={13} />
            {text('开始修稿', 'Start revision')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
