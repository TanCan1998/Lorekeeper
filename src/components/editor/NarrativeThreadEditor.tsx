import { CheckCircle2, Clock3, Loader2, Pencil, Plus, Sparkles, Trash2 } from 'lucide-react'
import { canExecuteGraph } from '../../services/graph-generation'
import type { NarrativeThreadCandidateGenerator } from '../../services/narrative-thread-candidate-generator'
import type { GeneratePlotTreeInput } from '../../services/plot-tree-generator'
import type { NarrativeThreadEventType } from '../../shared/narrative-thread'
import type { PlotTreeSnapshot } from '../../shared/plot-tree'
import { hasUsablePlotTreeEventSource } from '../../shared/plot-tree'
import { useLocaleStore } from '../../stores/locale-store'
import { STATUS_LABELS, usePlotThreads } from '../stage/plot/use-plot-threads'
import { Button } from '../ui/Button'
import {
  Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
} from '../ui/Dialog'
import { Input } from '../ui/Input'
import { Label } from '../ui/Label'
import { NativeSelect } from '../ui/NativeSelect'
import { Textarea } from '../ui/Textarea'
import PlotTreeView from './PlotTreeView'
import { pluralEn } from '../../shared/plural-en'

interface NarrativeThreadEditorProps {
  projectKey: string
  candidateGenerator?: NarrativeThreadCandidateGenerator
  initialView?: 'plot-tree' | 'plans'
  viewRequest?: number
  plotTreeGenerator?: (request: GeneratePlotTreeInput) => Promise<PlotTreeSnapshot>
}

export default function NarrativeThreadEditor({
  projectKey,
  candidateGenerator,
  initialView = 'plans',
  viewRequest,
  plotTreeGenerator,
}: NarrativeThreadEditorProps) {
  const text = useLocaleStore(s => s.text)
  const {
    view, setView, sourcePlanId,
    threads, finalizedDrafts, blueprints, dormantThreshold, busy,
    plan, setPlan, editingId, startEditingPlan, savePlan, deletePlan,
    eventPlanId, startEvent, eventDraftId, setEventDraftId, eventType, setEventType,
    eventEvidence, setEventEvidence, eventReason, setEventReason, eventError, saveEvent,
    generationModels, plotSources, selectedPlotModel, setPlotModelId, plotBusy, plotError, plotModelSelectionError,
    refreshPlotTree, clearPlotTree, openPlotSource,
    aiOpen, aiMode, aiBusy, aiError, selectedModelId, setAiModelId, modelSelectionError,
    aiBlueprintChapter, setAiBlueprintChapter, planCandidates, eventCandidates,
    openAI, closeAI, generatePlanCandidates, generateEventCandidates,
    confirmPlanCandidate, rejectPlanCandidate, confirmEventCandidate, rejectEventCandidate,
    graphRuns, graphRecovery, graphError, loadGraphRuns, restoreGraph, continueGraph, cancelGraph,
    copyOriginalResult, cancelOriginalRun, saveOriginalPlotTree, setAiError,
  } = usePlotThreads(projectKey, { candidateGenerator, initialView, viewRequest, plotTreeGenerator })

  return (
    <div className="h-full overflow-y-auto p-5" style={{ color: 'var(--color-text)' }}>
      <div className="mx-auto max-w-5xl space-y-5">
        <header className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="text-lg font-semibold">{text('剧情树与叙事线索', 'Plot tree & narrative threads')}</h2>
          <div className="flex gap-2" role="tablist" aria-label={text('剧情编辑器视图', 'Plot editor views')}>
            <Button
              size="sm"
              variant={view === 'plot-tree' ? 'default' : 'outline'}
              role="tab"
              aria-selected={view === 'plot-tree'}
              onClick={() => setView('plot-tree')}
            >
              {text('剧情树', 'Plot tree')}
            </Button>
            <Button
              size="sm"
              variant={view === 'plans' ? 'default' : 'outline'}
              role="tab"
              aria-selected={view === 'plans'}
              onClick={() => setView('plans')}
            >
              {text('计划清单', 'Plan list')}
            </Button>
          </div>
        </header>

        <section aria-label={text('生成记录恢复', 'Recover generation')} className="rounded-lg border p-3 space-y-2" style={{ borderColor: 'var(--color-border)' }}>
          <Button size="sm" variant="outline" onClick={() => void loadGraphRuns()}>{text('刷新生成记录', 'Refresh generation history')}</Button>
          {(aiBusy && !candidateGenerator || plotBusy && !plotTreeGenerator) && <Button size="sm" variant="outline" onClick={() => void cancelGraph()}>{text('取消当前生成', 'Cancel current generation')}</Button>}
          {graphRuns.map(run => <Button key={run.handle.runId} size="sm" variant="ghost" disabled={plotBusy || aiBusy} onClick={() => void restoreGraph(run)}>{text('恢复原生成', 'Recover generation')} · {run.operation === 'plot-tree-snapshot' ? text('剧情树', 'Plot tree') : run.operation === 'narrative-thread-plan-candidate' ? text('计划候选', 'Plan candidates') : text('事件候选', 'Event candidates')} · {run.handle.runId}</Button>)}
          {graphError && <p role="alert">{graphError}</p>}
          {graphRecovery && <div className="space-y-2">
            {(plotBusy || aiBusy) && <p role="status">{text('正在生成，请等待本次结果', 'Generation is in progress. Please wait for the result.')}</p>}
            {!plotBusy && !aiBusy && graphRecovery.result?.kind === 'plot' && (graphRecovery.effects.length > 0
              ? <p role="status">{text('生成结果已保存', 'The generated result has been saved.')}</p>
              : graphRecovery.sourceStatus === 'current' && graphRecovery.view.status !== 'cancelled' && <p role="status">{text('生成已完成，结果尚未保存。请确认后保存', 'Generation is complete. Confirm and save the result.')}</p>)}
            <p>{text('原模型', 'Original model')}: {graphRecovery.modelId} · {text('已确认', 'Confirmed')}: {graphRecovery.effects.length}</p>
            {graphRecovery.sourceStatus === 'conflict' && <p role="status">{text('来源已变化，仅可查看和复制。', 'Sources changed. Viewing and copying remain available.')}</p>}
            <Button size="sm" variant="outline" onClick={() => void copyOriginalResult()}>{text('复制原结果', 'Copy original result')}</Button>
            <Button size="sm" disabled={plotBusy || aiBusy || !canExecuteGraph(graphRecovery)} onClick={() => void continueGraph()}>{text('继续原运行', 'Continue original run')}</Button>
            <Button size="sm" variant="outline" onClick={cancelOriginalRun}>{text('取消原运行', 'Cancel original run')}</Button>
            {graphRecovery.result?.kind === 'plot' && <><pre className="max-h-40 overflow-auto whitespace-pre-wrap text-xs">{JSON.stringify(graphRecovery.result.snapshot, null, 2)}</pre><Button size="sm" disabled={plotBusy || aiBusy || graphRecovery.sourceStatus !== 'current' || graphRecovery.view.status === 'cancelled' || graphRecovery.effects.length > 0} onClick={saveOriginalPlotTree}>{text('保存原剧情树', 'Save original plot tree')}</Button></>}
          </div>}
        </section>

        {view === 'plot-tree' ? (
          <PlotTreeView
            key={plotSources?.snapshot?.generatedAt ?? 'empty'}
            snapshot={plotSources?.snapshot ?? null}
            sourceRevision={plotSources?.sourceRevision ?? ''}
            currentChapter={Math.max(1, ...(plotSources?.finalizedChapters.map(chapter => chapter.chapterNumber) ?? []))}
            models={generationModels}
            selectedModelId={selectedPlotModel?.id ?? null}
            busy={plotBusy || aiBusy}
            error={plotError || plotModelSelectionError}
            sourceReady={Boolean(plotSources && hasUsablePlotTreeEventSource(plotSources))}
            storedSnapshotInvalid={plotSources?.storedSnapshotInvalid === true}
            onModelChange={setPlotModelId}
            onGenerate={() => void refreshPlotTree()}
            onClear={() => void clearPlotTree()}
            onOpenSource={openPlotSource}
          />
        ) : <>
        <div className="flex items-start justify-between gap-3">
          <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
            {text('设置埋设与预计回收章节；活跃计划会自动注入后续写作，逾期或沉寂时提醒。只有人工确认的定稿内容才记为已发生事件。', 'Set setup and expected payoff chapters. Active plans are injected into later writing and flagged when overdue or dormant; only user-confirmed finalized text becomes an event.')}
          </p>
          <Button className="shrink-0" variant="ai" size="sm" onClick={() => openAI('plan')} disabled={plotBusy || aiBusy || blueprints.length === 0}>
            <Sparkles size={13} />{text('AI 建议伏笔与线索', 'Suggest foreshadowing with AI')}
          </Button>
        </div>

        <section className="rounded-lg border p-4 space-y-3" style={{ borderColor: 'var(--color-border)', background: 'var(--color-panel)' }}>
          <div className="flex items-center gap-2 font-medium"><Plus size={16} />{editingId === null ? text('新建计划', 'New plan') : text('编辑计划', 'Edit plan')}</div>
          <div className="grid grid-cols-2 gap-3">
            <label><Label>{text('标题', 'Title')}</Label><Input value={plan.title} onChange={event => setPlan({ ...plan, title: event.target.value })} /></label>
            <label><Label>{text('类型', 'Type')}</Label><Input value={plan.type} onChange={event => setPlan({ ...plan, type: event.target.value })} /></label>
            <label><Label>{text('计划埋设 / 开始章', 'Setup / start chapter')}</Label><Input type="number" min={1} value={plan.targetStartChapter} onChange={event => setPlan({ ...plan, targetStartChapter: Number(event.target.value) })} /></label>
            <label><Label>{text('预计回收 / 结束章', 'Expected payoff / end chapter')}</Label><Input type="number" min={1} value={plan.targetEndChapter} onChange={event => setPlan({ ...plan, targetEndChapter: Number(event.target.value) })} /></label>
          </div>
          <label><Label>{text('作者意图 / 理由', 'Author intent / rationale')}</Label><Textarea value={plan.authorIntent} onChange={event => setPlan({ ...plan, authorIntent: event.target.value })} /></label>
          <Button onClick={() => void savePlan()} disabled={busy || !plan.title.trim() || !plan.type.trim() || !plan.authorIntent.trim() || plan.targetEndChapter < plan.targetStartChapter}>{text('保存计划', 'Save plan')}</Button>
        </section>

        {threads.length === 0 && <p className="text-sm text-center py-8" style={{ color: 'var(--color-text-muted)' }}>{text('暂无伏笔或叙事线索', 'No foreshadowing or narrative threads yet')}</p>}
        {threads.map(thread => (
          <section
            id={`narrative-plan-${thread.id}`}
            key={thread.id}
            className="rounded-lg border p-4 space-y-3"
            style={{
              borderColor: sourcePlanId === thread.id ? 'var(--color-accent)' : 'var(--color-border)',
              background: 'var(--color-panel)',
            }}
          >
            <div className="flex items-start justify-between gap-3">
              <div>
                <h3 className="font-semibold">{thread.title}</h3>
                <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>{thread.type} · {text(`埋设/开始 ${thread.targetStartChapter} · 预计回收/结束 ${thread.targetEndChapter}`, `Setup/start ${thread.targetStartChapter} · expected payoff/end ${thread.targetEndChapter}`)}</p>
              </div>
              <span className="text-xs rounded px-2 py-1" style={{ background: 'var(--color-bg)' }}>{text(...STATUS_LABELS[thread.status])}</span>
            </div>
            <p className="text-sm">{thread.authorIntent}</p>
            {thread.status !== 'resolved' && thread.status !== 'abandoned' && (
              <div className="flex gap-3 text-xs" style={{ color: 'var(--color-text-muted)' }}>
                <span className="flex items-center gap-1"><Clock3 size={13} />{text(`沉寂 ${thread.dormantChapters} 章`, `Dormant ${thread.dormantChapters} ${pluralEn(thread.dormantChapters, 'chapter')}`)}</span>
                {thread.dormantChapters >= dormantThreshold && (
                  <span role="status">{text('已达到项目沉寂提醒阈值', 'Project dormant threshold reached')}</span>
                )}
                {thread.overdue && <span>{text('已逾期', 'Overdue')}</span>}
              </div>
            )}
            <div className="space-y-1">
              {thread.events.map(event => <div key={event.id} className="text-xs flex gap-2"><CheckCircle2 size={13} /><span>{text(`第${event.chapterNumber}章`, `Chapter ${event.chapterNumber}`)} · {text(...STATUS_LABELS[event.type])} · {event.evidence}</span></div>)}
            </div>
            <div className="flex gap-2">
              <Button variant="outline" size="sm" onClick={() => startEditingPlan(thread)}><Pencil size={13} />{text('编辑', 'Edit')}</Button>
              <Button variant="outline" size="sm" onClick={() => startEvent(thread.id)} disabled={finalizedDrafts.length === 0}>{text('确认定稿事件', 'Confirm finalized event')}</Button>
              <Button variant="ghost" size="sm" onClick={() => void deletePlan(thread.id)}><Trash2 size={13} />{text('删除', 'Delete')}</Button>
            </div>
            {eventPlanId === thread.id && <div className="grid grid-cols-2 gap-2 border-t pt-3" style={{ borderColor: 'var(--color-border)' }}>
              <NativeSelect value={eventDraftId} onChange={event => setEventDraftId(Number(event.target.value))}>{finalizedDrafts.map(draft => <option key={draft.id} value={draft.id}>{text(`第${draft.chapterNumber}章 · 定稿 v${draft.version}`, `Chapter ${draft.chapterNumber} · Finalized v${draft.version}`)}</option>)}</NativeSelect>
              <NativeSelect value={eventType} onChange={event => setEventType(event.target.value as NarrativeThreadEventType)}><option value="planted">{text('埋设', 'Planted')}</option><option value="progressing">{text('推进', 'Progressing')}</option><option value="resolved">{text('解决', 'Resolved')}</option><option value="abandoned">{text('放弃', 'Abandoned')}</option></NativeSelect>
              <div className="col-span-2">
                <Input placeholder={text('粘贴该定稿章节中的短原文', 'Paste a short excerpt from this finalized chapter')} value={eventEvidence} onChange={event => setEventEvidence(event.target.value)} />
                <p className="mt-1 text-xs" style={{ color: 'var(--color-text-muted)' }}>{text('证据必须逐字来自所选定稿章节。', 'Evidence must be copied from the selected finalized chapter.')}</p>
              </div>
              <Input className="col-span-2" placeholder={text('确认理由', 'Confirmation rationale')} value={eventReason} onChange={event => setEventReason(event.target.value)} />
              {eventError && <p className="col-span-2 text-xs" style={{ color: 'var(--color-error-text)' }}>{eventError}</p>}
              <div className="col-span-2 flex gap-2">
                <Button onClick={() => void saveEvent()} disabled={busy || !eventEvidence.trim() || !eventReason.trim()}>{text('保存事件', 'Save event')}</Button>
                <Button variant="ai" onClick={() => openAI('event')} disabled={busy}>
                  <Sparkles size={13} />{text('AI 识别定稿事件', 'Find finalized events with AI')}
                </Button>
              </div>
            </div>}
          </section>
        ))}
        </>}
      </div>
      <Dialog open={aiOpen} onOpenChange={open => { if (!open) closeAI() }}>
        <DialogContent className="max-w-[620px]">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2"><Sparkles size={15} />{aiMode === 'plan'
              ? text('蓝图计划候选', 'Blueprint plan candidates')
              : text('定稿事件候选', 'Finalized event candidates')}</DialogTitle>
            <DialogDescription>{aiMode === 'plan'
              ? text(
                  'AI 只提出人工计划候选；确认前不会写入项目，也不会产生章节事件。',
                  'AI proposes author-plan candidates only. Nothing is saved and no chapter event is created before confirmation.',
                )
              : text(
                  'AI 只从当前绑定的定稿正文提出带原文证据的事件候选；确认前不会写入项目。',
                  'AI proposes evidence-bound events only from the selected finalized chapter. Nothing is saved before confirmation.',
                )}</DialogDescription>
          </DialogHeader>
          <div className="px-6 py-4 space-y-4 max-h-[60vh] overflow-y-auto">
            {!candidateGenerator && graphRecovery && <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>{text('原运行模型', 'Original run model')}: {graphRecovery.modelId} · {text('已确认', 'Confirmed')}: {graphRecovery.effects.length}</p>}
            {!candidateGenerator && graphRecovery?.sourceStatus === 'conflict' && <p role="status">{text('来源已变化，仅可查看和复制。', 'Sources changed. Viewing and copying remain available.')}</p>}
            {!candidateGenerator && graphRecovery?.result && <Button size="sm" variant="outline" onClick={() => void copyOriginalResult().catch(() => setAiError(text('无法复制结果。', 'Could not copy the result.')))}>{text('复制原结果', 'Copy original result')}</Button>}
            <label className="block">
              <Label htmlFor="narrative-thread-ai-model">{text('本次识别模型', 'Model for this analysis')}</Label>
              <NativeSelect
                id="narrative-thread-ai-model"
                value={selectedModelId ?? ''}
                disabled={generationModels.length === 0 || aiBusy}
                onChange={event => setAiModelId(event.target.value || null)}
              >
                <option value="" disabled>{text('请选择可用生成模型', 'Select a generation model')}</option>
                {generationModels.map(model => <option key={model.id} value={model.id}>{model.name || model.modelName}</option>)}
              </NativeSelect>
              <p className="mt-1 text-xs" style={{ color: 'var(--color-text-muted)' }}>{text(
                '仅用于本次识别，不会更改默认模型。',
                'Used for this analysis only; it does not change the default model.',
              )}</p>
              {modelSelectionError && <p role="alert" className="mt-1 text-xs" style={{ color: 'var(--color-error-text)' }}>{modelSelectionError}</p>}
            </label>
            {aiMode === 'plan' && <label className="block">
              <Label htmlFor="narrative-thread-ai-blueprint">{text('蓝图章节', 'Blueprint chapter')}</Label>
              <NativeSelect id="narrative-thread-ai-blueprint" value={aiBlueprintChapter} disabled={aiBusy} onChange={event => setAiBlueprintChapter(Number(event.target.value))}>
                {blueprints.map(blueprint => <option key={blueprint.chapterNumber} value={blueprint.chapterNumber}>{text(`第${blueprint.chapterNumber}章 · ${blueprint.title}`, `Chapter ${blueprint.chapterNumber} · ${blueprint.title}`)}</option>)}
              </NativeSelect>
            </label>}
            {aiError && <p role="alert" className="text-xs" style={{ color: 'var(--color-error-text)' }}>{aiError}</p>}
            {planCandidates.map((candidate, index) => (
              <section key={`${candidate.title}:${index}`} className="rounded-lg border p-3 space-y-2" style={{ borderColor: 'var(--color-border)' }}>
                <div className="font-medium">{candidate.title}</div>
                <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>{candidate.type} · {text('目标', 'Target')} {candidate.targetStartChapter}–{candidate.targetEndChapter}</p>
                <p className="text-sm">{candidate.authorIntent}</p>
                <div className="flex gap-2">
                  <Button size="sm" onClick={() => void confirmPlanCandidate(candidate)} disabled={busy || candidate.generationIndex !== undefined && graphRecovery?.sourceStatus !== 'current'}>{text('确认计划', 'Confirm plan')}</Button>
                  <Button size="sm" variant="ghost" onClick={() => rejectPlanCandidate(candidate)} disabled={busy}>{text('拒绝候选', 'Reject candidate')}</Button>
                </div>
              </section>
            ))}
            {eventCandidates.map((candidate, index) => (
              <section key={`${candidate.type}:${candidate.evidence}:${index}`} className="rounded-lg border p-3 space-y-2" style={{ borderColor: 'var(--color-border)' }}>
                <div className="font-medium">{text(...STATUS_LABELS[candidate.type])} · {text(`第${candidate.chapterNumber}章`, `Chapter ${candidate.chapterNumber}`)}</div>
                <blockquote className="text-sm border-l-2 pl-3" style={{ borderColor: 'var(--color-border)' }}>{candidate.evidence}</blockquote>
                <p className="text-sm">{candidate.reason}</p>
                <div className="flex gap-2">
                  <Button size="sm" onClick={() => void confirmEventCandidate(candidate)} disabled={busy || candidate.generationIndex !== undefined && graphRecovery?.sourceStatus !== 'current'}>{text('确认事件', 'Confirm event')}</Button>
                  <Button size="sm" variant="ghost" onClick={() => rejectEventCandidate(candidate)} disabled={busy}>{text('拒绝候选', 'Reject candidate')}</Button>
                </div>
              </section>
            ))}
          </div>
          <DialogFooter>
            {!candidateGenerator && (aiBusy || graphRecovery) && <Button variant="outline" onClick={() => void cancelGraph()}>{text('取消原运行', 'Cancel original run')}</Button>}
            <Button variant="ghost" onClick={closeAI} disabled={aiBusy}>{text('关闭', 'Close')}</Button>
            <Button variant="ai" onClick={() => void (aiMode === 'plan' ? generatePlanCandidates() : generateEventCandidates())} disabled={plotBusy || aiBusy || Boolean(modelSelectionError) || (aiMode === 'plan' && !aiBlueprintChapter)}>
              {aiBusy ? <Loader2 size={13} className="animate-spin" /> : <Sparkles size={13} />}
              {aiBusy ? text('识别中...', 'Analyzing...') : !candidateGenerator && graphRecovery ? text('生成新候选', 'Generate new candidates') : text('生成候选', 'Generate candidates')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
