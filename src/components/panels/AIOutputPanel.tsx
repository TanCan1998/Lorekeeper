import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { CheckCircle2, Loader2, Circle, Sparkles, X, ChevronRight, StopCircle, AlertTriangle, SlidersHorizontal, Copy, Pencil, Trash2 } from 'lucide-react'
import GenerationBudgetDiagnostics from './GenerationBudgetDiagnostics'
import { WorkflowConfirmationPanel } from './WorkflowConfirmationPanel'
import { useWorkflowStore, type WorkflowRun, type WorkflowStep } from '../../stores/workflow-store'
import { useLayoutStore } from '../../stores/layout-store'
import { useProjectStore } from '../../stores/project-store'
import { useWorkflowReasoningStore } from '../../stores/workflow-reasoning-store'
import { useLocaleStore } from '../../stores/locale-store'
import { projectSessionContextFromProject, sameProjectSessionContext } from '../../shared/project-session-context'
import type { Locale } from '../../i18n/types'
import MarkdownContent from '../ui/MarkdownContent'
import { canResumeDraft } from '../run/run-recovery'
import { useRunRecovery, type RunRecovery } from '../run/use-run-recovery'
import { isCurrentSessionRun, openNovelConfigurationFor, runFailure, useResumePlotOutline } from '../run/run-failure'
import { ACTIVE_RUN_STATUSES, primaryRun, projectRuns, splitThinking } from '../run/run-output'
import { useReviewWaitLines } from '../run/use-review-wait'

function runText(locale: Locale, zhCNText: string, enUSText: string): string {
  return locale === 'en-US' ? enUSText : zhCNText
}

/**
 * 右侧面板「AI 输出」视图
 * 参考 Cursor Agent 风格：扁平化、极简文字驱动、可折叠思考区
 */
export default function AIOutputPanel() {
  // 使用 selector 精确订阅，避免 globalLogs 高频更新导致整个面板重渲染
  const allActiveRuns = useWorkflowStore(s => s.activeRuns)
  const allHistory = useWorkflowStore(s => s.history)
  const currentLocale = useLocaleStore(s => s.locale)
  const currentProject = useProjectStore(s => s.currentProject)
  const activeRuns = projectRuns(allActiveRuns, currentProject?.path)
  const history = projectRuns(allHistory, currentProject?.path)
  const activeRun = primaryRun(activeRuns)
  const activeRunId = activeRun?.id
  const mainSession = useMemo(() => projectSessionContextFromProject(currentProject), [currentProject])
  const [viewRunId, setViewRunId] = useState<string | null>(null)
  const requestedRunId = useLayoutStore(state => state.rightPanelRunId)
  const lastActiveRunId = useRef<string | undefined>(undefined)
  const recovery = useRunRecovery()

  // 显式定位优先；消费请求后的重渲染不再触发自动跟随。
  useEffect(() => {
    const activeChanged = lastActiveRunId.current !== activeRunId
    lastActiveRunId.current = activeRunId
    if (!requestedRunId && !activeChanged) return
    const syncTimer = window.setTimeout(() => {
      if (requestedRunId) {
        if (useLayoutStore.getState().rightPanelRunId !== requestedRunId) return
        const workflow = useWorkflowStore.getState()
        const target = workflow.activeRuns.find(run => run.id === requestedRunId)
        if (target?.status === 'waiting' && workflow.waitingRuns[target.id]?.waitingForConfirm
          && sameProjectSessionContext(target.projectSession, projectSessionContextFromProject(useProjectStore.getState().currentProject))) {
          setViewRunId(target.id)
        }
        useLayoutStore.setState(state => state.rightPanelRunId === requestedRunId ? { rightPanelRunId: null } : state)
      } else if (activeRunId) {
        setViewRunId(activeRunId)
      }
    }, 0)
    return () => window.clearTimeout(syncTimer)
  }, [activeRunId, requestedRunId, mainSession])

  const viewRun: WorkflowRun | undefined =
    activeRuns.find(r => r.id === viewRunId) ||
    history.find(r => r.id === viewRunId) ||
    activeRun ||
    undefined

  const recentHistory = history.slice(0, 10)
  const visibleLocale = viewRun?.uiLocale ?? currentLocale
  const recoveryAndHistory = <>
    <GenerationRecoverySection recovery={recovery} locale={currentLocale} />
    <RecoveryCandidateSection recovery={recovery} locale={currentLocale} />
    {viewRun && (activeRun || recentHistory.length > 0) && (
      <div className="max-h-36 overflow-y-auto border-b px-3 py-2" style={{ borderColor: 'var(--color-border)' }}>
        {activeRun && viewRun.id !== activeRun.id && (
          <button type="button" className="text-xs" onClick={() => setViewRunId(activeRun.id)}>
            {runText(visibleLocale, '当前生成', 'Current generation')}
          </button>
        )}
        {recentHistory.length > 0 && <HistoryList items={recentHistory} onSelect={setViewRunId} locale={visibleLocale} />}
      </div>
    )}
  </>

  return (
    <div
      className="writer-ai-panel flex flex-col h-full overflow-hidden"
    >
      {/* 面板头部 */}
      <div
        className="no-select flex items-center justify-between gap-1.5 px-2 flex-shrink-0"
        style={{
          height: 'var(--height-panel-header)',
          borderBottom: '1px solid var(--color-border)',
        }}
      >
        <span
          className="text-xs font-medium uppercase tracking-widest"
          style={{ color: 'var(--color-text-muted)' }}
        >
          {runText(visibleLocale, 'AI 输出', 'AI output')}
        </span>
        <button
          onClick={() => useLayoutStore.getState().setAIPanelOpen(false)}
          title={runText(visibleLocale, '收起 AI 输出', 'Close AI output')}
          className="icon-btn"
          style={{ width: 20, height: 20 }}
        >
          <X size={13} strokeWidth={1.5} />
        </button>
      </div>

      {/* 内容区 */}
      <div className="flex-1 min-h-0 overflow-hidden flex flex-col">
        {viewRun ? (
          <ActiveRunView
            run={viewRun}
            activeRuns={activeRuns}
            onSwitchRun={setViewRunId}
          >{recoveryAndHistory}</ActiveRunView>
        ) : (
          <div className="h-full overflow-y-auto">
            {recoveryAndHistory}
            {recentHistory.length === 0 ? (
              <EmptyState />
            ) : (
              <div className="px-3 py-3">
                <HistoryList items={recentHistory} onSelect={setViewRunId} locale={visibleLocale} />
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  )
}

const candidateTextButtonStyle = { width: 'fit-content', height: 'auto' } as const

function TextButton({ disabled, onClick, children }: { disabled?: boolean; onClick: () => void; children: ReactNode }) {
  return <button type="button" className="icon-btn px-2" style={candidateTextButtonStyle} disabled={disabled} onClick={onClick}>{children}</button>
}

/** 中断运行的恢复区，数据与动作来自两种外壳共用的 useRunRecovery。 */
function GenerationRecoverySection({ recovery, locale }: { recovery: RunRecovery; locale: Locale }) {
  const entries = recovery.entries.filter(entry => entry.kind !== 'candidate')
  const { busy, generationError: error } = recovery
  if (!entries.length && !error) return null
  return <section className="max-h-80 overflow-y-auto border-b p-3 text-xs" aria-label={runText(locale, '持久正文候选', 'Saved draft candidates')}>
    {error && <p role="alert">{error}</p>}
    {entries.map(entry => {
      switch (entry.kind) {
        case 'planning': return <article key={entry.key} className="mb-3">
          <p>{entry.title}</p>
          <p>{entry.note}</p>
          <TextButton disabled={busy} onClick={() => recovery.openPlanning(entry)}>{entry.actionLabel}</TextButton>
        </article>
        case 'editor': return <article key={entry.key} className="mb-3">
          <p>{entry.title}</p>
          <p className="whitespace-pre-wrap">{entry.selectedText.slice(0, 100)}</p>
          <GenerationBudgetDiagnostics diagnostics={entry.budget} locale={locale} />
          {entry.usage && <p>{entry.usage}</p>}
          {entry.conflictNote && <p>{entry.conflictNote}</p>}
          {entry.suggestions.map(suggestion => <div key={suggestion.id}>
            <p className="whitespace-pre-wrap">{suggestion.text.slice(0, 180)}</p>
            {suggestion.incompleteNote && <p>{suggestion.incompleteNote}</p>}
            <TextButton onClick={() => recovery.copy(suggestion.text)}>{entry.labels.copySuggestion}</TextButton>
          </div>)}
          {entry.tails.map(tail => <div key={tail.id}>
            <p className="whitespace-pre-wrap">{tail.text.slice(0, 180)}</p>
            <TextButton onClick={() => recovery.copy(tail.text)}>{entry.labels.copyTail}</TextButton>
          </div>)}
          <TextButton onClick={() => recovery.copy(entry.documentText)}>{entry.labels.copyDocument}</TextButton>
          <TextButton disabled={busy || !entry.canContinue} onClick={() => recovery.continueEditor(entry)}>{entry.labels.continue}</TextButton>
        </article>
        case 'review': return <article key={entry.key} className="mb-3">
          <p>{entry.title}</p>
          <GenerationBudgetDiagnostics diagnostics={entry.budget} locale={locale} />
          {entry.usage && <p>{entry.usage}</p>}
          {entry.notes.map(note => <p key={note}>{note}</p>)}
          {entry.artifacts.map(artifact => <div key={artifact.id}>
            <span className="whitespace-pre-wrap">{artifact.text.slice(0, 180)}</span>
            <TextButton onClick={() => recovery.copy(artifact.text)}>{entry.labels.copy}</TextButton>
          </div>)}
          {entry.tails.map(tail => <div key={tail.id}>
            <span className="whitespace-pre-wrap">{tail.text.slice(0, 180)}</span>
            <TextButton onClick={() => recovery.copy(tail.text)}>{entry.labels.copyTail}</TextButton>
          </div>)}
          <TextButton disabled={busy || !entry.canAct} onClick={() => recovery.resumeReview(entry)}>{entry.actionLabel}</TextButton>
        </article>
        case 'batch': return <article key={entry.key} className="mb-3">
          <p>{entry.title}</p>
          {entry.staleNote && <p>{entry.staleNote}</p>}
          <TextButton disabled={busy || !entry.canContinue} onClick={() => recovery.resumeBatch(entry)}>{entry.actionLabel}</TextButton>
        </article>
        case 'draft': {
          const picked = recovery.picked(entry)
          return <article key={entry.key} className="mb-3">
            <p>{entry.title}</p>
            <GenerationBudgetDiagnostics diagnostics={entry.budget} locale={locale} />
            {entry.usage && <p>{entry.usage}</p>}
            {entry.artifacts.map(artifact => <label key={artifact.id} className="block">
              <input type="checkbox" checked={picked.includes(artifact.id)} disabled={busy || !artifact.selectable}
                onChange={event => recovery.togglePick(entry, artifact.id, event.target.checked)} />
              <span className="whitespace-pre-wrap">{artifact.text.slice(0, 180)}</span>
              <TextButton onClick={() => recovery.copy(artifact.text)}>{entry.labels.copy}</TextButton>
            </label>)}
            {entry.tails.map(tail => <div key={tail.id}>
              <p>{entry.labels.tailLead}</p>
              <span className="whitespace-pre-wrap">{tail.text.slice(0, 180)}</span>
              <TextButton onClick={() => recovery.copy(tail.text)}>{entry.labels.copy}</TextButton>
            </div>)}
            <p>{entry.note}</p>
            <TextButton disabled={busy || !canResumeDraft(entry, picked)} onClick={() => recovery.resumeDraft(entry)}>{entry.actionLabel}</TextButton>
          </article>
        }
      }
    })}
  </section>
}

function RecoveryCandidateSection({ recovery, locale }: { recovery: RunRecovery; locale: Locale }) {
  const candidates = recovery.entries.filter(entry => entry.kind === 'candidate')
  const error = recovery.candidateError
  if (!candidates.length && !error) return null
  return (
    <section className="max-h-[45%] overflow-y-auto border-b px-3 py-2" style={{ borderColor: 'var(--color-border)' }}>
      <div className="mb-2 text-xs font-medium" style={{ color: 'var(--color-text)' }}>
        {runText(locale, '恢复候选', 'Recovery candidates')}
      </div>
      {error && <p role="alert" className="mb-2 text-xs" style={{ color: 'var(--color-error-text)' }}>{error}</p>}
      {candidates.map(entry => (
        <article key={entry.key} className="mb-2 rounded-md border p-2 text-xs" style={{ borderColor: 'var(--color-border)' }}>
          <div className="font-medium" style={{ color: 'var(--color-text)' }}>{entry.title}</div>
          {entry.staleNote && (
            <p className="mt-1" style={{ color: 'var(--color-warning-text)' }}>{entry.staleNote}</p>
          )}
          <div className="mt-1 max-h-24 overflow-y-auto whitespace-pre-wrap" style={{ color: 'var(--color-text-secondary)' }}>
            {entry.text}
          </div>
          <div className="mt-2 flex gap-1.5">
            <button type="button" className="icon-btn px-2" style={candidateTextButtonStyle} onClick={() => recovery.copyCandidate(entry)}><Copy size={12} />{runText(locale, '复制', 'Copy')}</button>
            <button type="button" className="icon-btn px-2" style={candidateTextButtonStyle} disabled={!entry.canContinue} onClick={() => recovery.continueCandidate(entry)}><Pencil size={12} />{runText(locale, '继续编辑', 'Continue editing')}</button>
            <button type="button" className="icon-btn px-2" style={candidateTextButtonStyle} onClick={() => recovery.discardCandidate(entry)}><Trash2 size={12} />{runText(locale, '放弃', 'Discard')}</button>
          </div>
        </article>
      ))}
    </section>
  )
}


// ===== 空状态 =====

function EmptyState() {
  const text = useLocaleStore(s => s.text)
  return (
    <div className="flex flex-col items-center justify-center h-full gap-2 px-6" style={{ color: 'var(--color-text-muted)' }}>
      <Sparkles size={20} style={{ opacity: 0.2 }} />
      <span className="text-xs opacity-60">{text('暂无输出', 'No output')}</span>
    </div>
  )
}


// ===== 活跃任务视图（Cursor 风格） =====

function ActiveRunView({
  run,
  activeRuns,
  onSwitchRun,
  children,
}: {
  run: WorkflowRun
  activeRuns: WorkflowRun[]
  onSwitchRun: (id: string) => void
  children: ReactNode
}) {
  const locale = run.uiLocale
  const transientReasoning = useWorkflowReasoningStore(state => state.entries[run.id]?.text ?? '')
  const scrollRef = useRef<HTMLDivElement>(null)
  const [autoScroll, setAutoScroll] = useState(true)
  const isActive = ACTIVE_RUN_STATUSES.has(run.status)
  const canCancel = run.status !== 'cancelling'
  const cancelWorkflow = useWorkflowStore.getState().cancelWorkflow
  const prevLenRef = useRef(0)

  const resumeOutline = useResumePlotOutline(run)
  const currentStep = run.steps[run.currentStepIndex] || run.steps[0]
  const { content } = splitThinking(currentStep?.result)

  // 节流自动滚动：仅在正文内容长度变化时触发，思考区不自动滚动
  const contentLen = content.length
  useEffect(() => {
    if (!autoScroll || !scrollRef.current) return
    if (contentLen !== prevLenRef.current) {
      prevLenRef.current = contentLen
      requestAnimationFrame(() => {
        if (scrollRef.current) {
          scrollRef.current.scrollTop = scrollRef.current.scrollHeight
        }
      })
    }
  }, [contentLen, autoScroll, run.currentStepIndex])

  const handleScroll = () => {
    const el = scrollRef.current
    if (!el) return
    setAutoScroll(el.scrollHeight - el.scrollTop - el.clientHeight < 60)
  }

  // 整体进度百分比
  const completedCount = run.steps.filter(s => s.status === 'completed').length
  const overallProgress = run.steps.length > 0
    ? Math.round(((completedCount + (currentStep?.progress || 0) / 100) / run.steps.length) * 100)
    : 0

  return (
    <div className="flex min-h-0 flex-col h-full overflow-hidden">
      {/* 多任务切换（多于1个任务时显示） */}
      {activeRuns.length > 1 && (
        <div
          className="flex items-center gap-1 px-2 py-1.5 flex-shrink-0 overflow-x-auto"
          style={{ borderBottom: '1px solid var(--color-border)' }}
        >
          {activeRuns.map(r => (
            <button
              key={r.id}
              onClick={() => onSwitchRun(r.id)}
              className="text-[0.68rem] px-2 py-0.5 rounded transition-all flex-shrink-0"
              style={{
                backgroundColor: r.id === run.id ? 'var(--color-hover)' : 'transparent',
                color: r.id === run.id ? 'var(--color-text)' : 'var(--color-text-muted)',
              }}
            >
              {r.title}
            </button>
          ))}
        </div>
      )}

      {/* 整体进度条（细线） */}
      {currentStep?.generationActivity?.operation !== 'review-chapter' && <div className="flex-shrink-0" style={{ height: 2, backgroundColor: 'var(--color-border)' }}>
        <div
          style={{
            height: '100%',
            width: `${Math.max(isActive ? 3 : 0, overallProgress)}%`,
            backgroundColor: run.status === 'completed' ? 'var(--color-success)' : 'var(--color-accent)',
            borderRadius: 1,
            transition: 'width 0.6s ease',
          }}
        />
      </div>}

      {/* 滚动内容区 */}
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className="flex-1 min-h-0 overflow-y-auto"
      >
        {children}
        {/* 步骤进度区及独立输出流 */}
        <div className="px-2 pt-2 pb-4">
          {run.steps.map((step, i) => (
            <StepOutputBlock
              key={step.id}
              step={step}
              index={i}
              total={run.steps.length}
              isActiveRun={isActive}
              isCurrentStep={i === run.currentStepIndex}
              transientReasoning={isActive && i === run.currentStepIndex ? transientReasoning : ''}
              locale={locale}
            />
          ))}

          {run.status === 'failed' && (
            <WorkflowFailureNotice
              run={run}
              resumingSynopsis={resumeOutline.resuming}
              onResumeSynopsis={() => { void resumeOutline.resume() }}
            />
          )}

          {/* 全局完成状态（所有步骤走完之后展示） */}
          {!isActive && run.status === 'completed' && (
            <div
              className="flex items-center justify-center gap-1.5 pt-4 pb-2 mb-2 text-xs"
              style={{ color: 'var(--color-success-text)', borderTop: '1px dashed var(--color-border)' }}
            >
              <CheckCircle2 size={12} />
              {runText(locale, '整个工作流已全部完成', 'The workflow is complete')}
            </div>
          )}
        </div>

      </div>

      {/* 确认与中止操作保持在输出滚动区外 */}
      {isActive && canCancel && (
        <div className="flex max-h-[75%] min-h-0 flex-shrink-0 flex-col gap-2 p-2">
          <WorkflowConfirmationPanel key={run.id} run={run} />
          <button
            onClick={() => cancelWorkflow(run.id)}
            className="flex flex-shrink-0 self-center items-center justify-center gap-1.5 text-xs px-3 py-1.5 rounded-full transition-all"
            style={{
              color: 'var(--color-text)',
              backgroundColor: 'var(--color-hover)',
              border: '1px solid var(--color-border)'
            }}
            onMouseEnter={e => {
              e.currentTarget.style.color = '#fff'
              e.currentTarget.style.backgroundColor = 'var(--color-error)'
              e.currentTarget.style.borderColor = 'var(--color-error)'
            }}
            onMouseLeave={e => {
              e.currentTarget.style.color = 'var(--color-text)'
              e.currentTarget.style.backgroundColor = 'var(--color-hover)'
              e.currentTarget.style.borderColor = 'var(--color-border)'
            }}
          >
            <StopCircle size={13} />
            <span className="font-medium tracking-wide">{runText(locale, '中止生成', 'Stop generation')}</span>
          </button>
        </div>
      )}
    </div>
  )
}


// ===== 新版渲染单步结果（支持查看所有历史步骤数据） =====
function StepOutputBlock({ step, index, total, isActiveRun, isCurrentStep, transientReasoning, locale }: { step: WorkflowStep; index: number; total: number; isActiveRun: boolean; isCurrentStep: boolean; transientReasoning: string; locale: Locale }) {
  const isRunning = step.status === 'running'
  const isCompleted = step.status === 'completed'
  const isFailed = step.status === 'failed'

  const { raw: rawText, thinking, content } = splitThinking(step.result)
  const displayedThinking = transientReasoning || thinking
  const hasOutput = Boolean(rawText || displayedThinking)

  // 当前激活的步骤默认展开，过去/未来的默认折叠（只有产生了内容的步骤才允许展开）
  const [expanded, setExpanded] = useState(isCurrentStep)

  // 监听如果步骤被激活，则自动展开
  useEffect(() => {
    let mounted = true
    if (isCurrentStep) {
      Promise.resolve().then(() => {
        if (mounted) setExpanded(true)
      })
    }
    return () => { mounted = false }
  }, [isCurrentStep])

  return (
    <div className="mb-1.5">
      {/* 头部摘要项，点击折叠/展开 */}
      <div
        onClick={() => { if (hasOutput) setExpanded(!expanded) }}
        className="flex items-center gap-2 px-2 py-1.5 rounded-md text-xs transition-colors"
        style={{
          cursor: hasOutput ? 'pointer' : 'default',
          backgroundColor: isRunning ? 'var(--color-hover)' : 'transparent',
          color: isRunning ? 'var(--color-text)' :
                 isCompleted ? 'var(--color-text-secondary)' :
                 isFailed ? 'var(--color-error-text)' :
                 'var(--color-text-muted)',
        }}
        title={hasOutput ? runText(locale, '点击查看该步骤的历史输出', 'View output history for this step') : undefined}
      >
        {/* 状态图标 */}
        <span className="flex-shrink-0 w-4 flex justify-center">
          {isCompleted && <CheckCircle2 size={11} style={{ color: 'var(--color-success)' }} />}
          {isRunning && <Loader2 size={11} className="animate-spin" style={{ color: 'var(--color-accent)' }} />}
          {isFailed && <Circle size={11} style={{ color: 'var(--color-error)', fill: 'var(--color-error)' }} />}
          {(step.status === 'pending' || step.status === 'skipped') && (
            <span
              className="w-1.5 h-1.5 rounded-full"
              style={{ backgroundColor: 'var(--color-border)' }}
            />
          )}
        </span>

        {/* 步骤名 */}
        <span className="truncate flex-1" style={{ fontWeight: isRunning ? 500 : 400 }}>
          {step.name}
        </span>

        {/* 进度 */}
        {isRunning && step.progress !== undefined && step.generationActivity?.operation !== 'review-chapter' && (
          <span className="font-mono text-[0.62rem] flex-shrink-0 opacity-60">
            {step.progress}%
          </span>
        )}

        {/* 展开角标或序号 */}
        {(hasOutput && !isRunning) ? (
          <ChevronRight
            size={11}
            style={{
              transition: 'transform 0.2s',
              transform: expanded ? 'rotate(90deg)' : 'none',
              opacity: 0.4,
            }}
          />
        ) : (
          <span className="font-mono text-[0.6rem] flex-shrink-0 opacity-30">
            {index + 1}/{total}
          </span>
        )}
      </div>

      {isRunning && isActiveRun && step.generationActivity?.operation === 'review-chapter' && <ReviewWaitStatus step={step} locale={locale} />}

      {/* 展开的对应输出数据 */}
      {expanded && hasOutput && (
        <div className="pl-[4px] pr-1 pt-1 pb-3 text-xs w-full max-w-full break-words">
          {/* 思维链区域 */}
          {displayedThinking && (
            <ThinkingBlock
              thinking={displayedThinking}
              showCursor={isRunning && isActiveRun && !content && step.generationActivity?.operation !== 'review-chapter'}
              hasContent={!!content}
              locale={locale}
            />
          )}
          
          {/* 实际正文区域 */}
          {content && (
            <div className="mt-1">
              <MarkdownContent content={content} streaming={isRunning && isActiveRun} />
            </div>
          )}
        </div>
      )}

      {/* 如果是单一正在执行等待，则显示一个等待骨架 */}
      {!hasOutput && isRunning && isActiveRun && (
        <div className="pl-[4px] pr-1 pt-1 pb-3 text-xs text-center" style={{ color: 'var(--color-text-muted)', opacity: 0.7 }}>
          {runText(locale, '等待指令响应...', 'Waiting for the workflow step...')}
        </div>
      )}
    </div>
  )
}

function ReviewWaitStatus({ step, locale }: { step: WorkflowStep; locale: Locale }) {
  const lines = useReviewWaitLines(step, locale)
  return <div className="px-2 py-1 text-xs" role="status" style={{ color: 'var(--color-text-secondary)' }}>
    {lines.map(line => <p key={line}>{line}</p>)}
  </div>
}

function WorkflowFailureNotice({
  run,
  resumingSynopsis,
  onResumeSynopsis,
}: {
  run: WorkflowRun
  resumingSynopsis: boolean
  onResumeSynopsis: () => void
}) {
  const { title, errorCode, presentation, canResumeOutline: resumeSynopsisAvailable } = runFailure(run)
  const locale = run.uiLocale
  const matchesCurrentProject = useProjectStore(() => isCurrentSessionRun(run))

  return (
    <div
      role="alert"
      className="mt-3 mx-2 flex gap-2 rounded-md px-2.5 py-2 text-xs leading-relaxed"
      style={{
        color: 'var(--color-error-text)',
        backgroundColor: 'color-mix(in srgb, var(--color-error) 10%, transparent)',
        border: '1px solid color-mix(in srgb, var(--color-error) 35%, transparent)',
      }}
    >
      <AlertTriangle size={14} className="mt-0.5 flex-shrink-0" aria-hidden="true" />
      <div className="min-w-0">
        <p className="m-0 break-words">{title}</p>
        <p className="font-medium m-0">
          {presentation.heading}
        </p>
        <p className="m-0 mt-0.5 break-words">{presentation.reason}</p>
        {errorCode && <p className="m-0 mt-1 break-all">{runText(locale, '错误码：', 'Error code: ')}{errorCode}</p>}
        {presentation.persistence && <p className="m-0 mt-1">{presentation.persistence}</p>}
        {presentation.guidance && <p className="m-0 mt-1">{presentation.guidance}</p>}
        {presentation.action === 'open-novel-config' && presentation.actionLabel && (
          <button
            type="button"
            onClick={() => openNovelConfigurationFor(run)}
            disabled={!matchesCurrentProject}
            className="mt-2 inline-flex items-center gap-1.5 rounded px-2 py-1 font-medium transition-colors"
            style={{
              color: 'var(--color-text)',
              backgroundColor: 'var(--color-hover)',
              border: '1px solid var(--color-border)',
              opacity: matchesCurrentProject ? 1 : 0.55,
              cursor: matchesCurrentProject ? 'pointer' : 'not-allowed',
            }}
          >
            <SlidersHorizontal size={12} aria-hidden="true" />
            {presentation.actionLabel}
          </button>
        )}
        {presentation.action === 'open-novel-config' && !matchesCurrentProject && (
          <p className="m-0 mt-1" style={{ color: 'var(--color-text-muted)' }}>
            {locale === 'zh-CN'
              ? '此结果属于另一项目会话。请切回该项目后再打开小说配置。'
              : 'This result belongs to another project session. Switch back to that project before opening Novel configuration.'}
          </p>
        )}

        {/* 断点续写：已完成部分已自动保存，点击后 AI 接着往下写 */}
        {resumeSynopsisAvailable && (
          <div className="mt-2">
            <button
              type="button"
              onClick={onResumeSynopsis}
              disabled={!matchesCurrentProject || resumingSynopsis}
              className="inline-flex items-center gap-1.5 rounded-full px-3.5 py-1.5 text-xs font-medium shadow-sm transition-colors"
              style={{
                color: '#fff',
                backgroundColor: 'var(--color-accent)',
                border: '1px solid var(--color-accent)',
                opacity: matchesCurrentProject ? 1 : 0.5,
                cursor: matchesCurrentProject ? 'pointer' : 'not-allowed',
              }}
            >
              {resumingSynopsis
                ? <Loader2 size={12} className="animate-spin" aria-hidden="true" />
                : <Sparkles size={12} aria-hidden="true" />}
              {resumingSynopsis
                ? runText(locale, '正在从断点续写...', 'Resuming from the break point...')
                : runText(locale, '继续生成情节大纲（断点续写）', 'Continue plot outline (resume)')}
            </button>
            <p className="m-0 mt-1.5" style={{ color: 'var(--color-text-muted)' }}>
              {runText(
                locale,
                '已完成的部分已保存为不完整大纲，不会被覆盖；本次将让 AI 接着上次的末尾继续写。',
                'The completed part is already saved as an incomplete outline and will not be lost; the AI will continue from where it stopped.',
              )}
            </p>
            {!matchesCurrentProject && (
              <p className="m-0 mt-1" style={{ color: 'var(--color-text-muted)' }}>
                {runText(
                  locale,
                  '此结果属于另一项目会话。请切回该项目后再续写。',
                  'This result belongs to another project session. Switch back to that project before continuing.',
                )}
              </p>
            )}
          </div>
        )}
      </div>
    </div>
  )
}


// ===== 思考区块（Cursor "Worked for" 风格） =====

function ThinkingBlock({ thinking, showCursor, hasContent, locale }: { thinking: string; showCursor: boolean; hasContent: boolean; locale: Locale }) {
  // 正文未开始时默认展开，正文开始后默认关闭
  const [expanded, setExpanded] = useState(!hasContent)
  const scrollRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    let mounted = true
    if (hasContent) {
      Promise.resolve().then(() => {
        if (mounted) setExpanded(false)
      })
    }
    return () => { mounted = false }
  }, [hasContent])

  useEffect(() => {
    if (expanded && scrollRef.current) {
      const el = scrollRef.current
      // 只有在用户没有往上滚拉太多时，才自动贴近底部
      if (el.scrollHeight - el.scrollTop - el.clientHeight < 80) {
        // 使用 requestAnimationFrame 确保在 DOM 更新后执行
        requestAnimationFrame(() => {
          if (scrollRef.current) {
            scrollRef.current.scrollTop = scrollRef.current.scrollHeight
          }
        })
      }
    }
  }, [thinking, expanded])

  return (
    <div className="mb-0.5">
      {/* 可折叠标题按钮 — 参考 Cursor 的 "Worked for Xm" */}
      <button
        type="button"
        onClick={() => setExpanded(v => !v)}
        className="group flex items-center gap-1.5 w-full text-left text-xs min-h-6 py-1 select-none transition-colors"
        style={{ color: 'var(--color-text-muted)', opacity: 0.8 }}
        onMouseEnter={e => { e.currentTarget.style.color = 'var(--color-text)' }}
        onMouseLeave={e => { e.currentTarget.style.color = 'var(--color-text-muted)' }}
      >
        <ChevronRight
          size={12}
          style={{
            transition: 'transform 0.2s',
            transform: expanded ? 'rotate(90deg)' : 'none',
          }}
        />
        <span>
          {showCursor
            ? runText(locale, '思考中...', 'Thinking...')
            : runText(locale, '思考过程', 'Thinking process')}
        </span>
        {showCursor && !expanded && (
          <span className="ai-stream-cursor" style={{ height: 11, width: 3 }} />
        )}
      </button>

      {/* 展开的思考内容 */}
      {expanded && (
        <div
          ref={scrollRef}
          className="ml-0.5 pl-3 border-l-2 py-0.5 mb-2 mt-1 text-xs leading-relaxed whitespace-pre-wrap overflow-y-auto"
          style={{
            borderColor: 'var(--color-border)',
            color: 'var(--color-text-muted)',
            maxHeight: 250,
            fontFamily: 'var(--font-mono)',
            fontSize: '0.75rem',
            lineHeight: 1.6,
          }}
        >
          {thinking}
          {showCursor && <span className="ai-stream-cursor" style={{ height: 12, width: 3 }} />}
        </div>
      )}
    </div>
  )
}


// ===== 历史列表 =====

function HistoryList({ items, onSelect, locale }: { items: WorkflowRun[]; onSelect: (id: string) => void; locale: Locale }) {
  return (
    <div>
      <p
        className="text-[0.68rem] font-medium mb-2 px-1 uppercase tracking-widest"
        style={{ color: 'var(--color-text-muted)', opacity: 0.7 }}
      >
        {runText(locale, '历史', 'History')}
      </p>
      <div className="flex flex-col gap-0.5">
        {items.map(run => (
          <button
            key={run.id}
            onClick={() => onSelect(run.id)}
            className="w-full flex items-center gap-2 px-2 py-1.5 rounded-md text-left transition-colors"
            onMouseEnter={e => { e.currentTarget.style.backgroundColor = 'var(--color-hover)' }}
            onMouseLeave={e => { e.currentTarget.style.backgroundColor = 'transparent' }}
          >
            {run.status === 'completed'
              ? <CheckCircle2 size={10} style={{ color: 'var(--color-success)', flexShrink: 0, opacity: 0.6 }} />
              : <Circle size={10} style={{ color: 'var(--color-error)', flexShrink: 0, opacity: 0.6 }} />
            }
            <span className="text-xs truncate flex-1" style={{ color: 'var(--color-text-secondary)' }}>
              {run.title}
            </span>
            <span className="text-[0.6rem] flex-shrink-0 font-mono opacity-30">
              {new Date(run.createdAt).toLocaleTimeString(run.uiLocale, { hour: '2-digit', minute: '2-digit' })}
            </span>
          </button>
        ))}
      </div>
    </div>
  )
}
