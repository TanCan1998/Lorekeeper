import { useState, useEffect, useCallback, useRef } from 'react'
import {
  Save, BookOpen, RefreshCw, Plus, Trash2,
  Sparkles, PenLine, ListChecks, AlertTriangle
} from 'lucide-react'
import { useProjectStore } from '../../stores/project-store'
import { ipc } from '../../services/ipc-client'
import {
  projectSessionContextFromProject,
  sameProjectSessionContext,
} from '../../shared/project-session-context'
import {
  parseTextBlueprintsStrict,
  type DirectoryWorkflowParams,
} from '../../services/workflows/directory-workflow'
import { launchCreativeWorkflow } from '../../services/workflows/creative-workflow-launcher'
import DirectoryConfigDialog from '../dialogs/DirectoryConfigDialog'
import BatchChapterCreationDialog from '../dialogs/BatchChapterCreationDialog'
import { Button } from '../ui/Button'
import { Input } from '../ui/Input'
import { Textarea } from '../ui/Textarea'
import { Label } from '../ui/Label'
import { NativeSelect } from '../ui/NativeSelect'
import { chapterRoleOptions, chapterRoleSelectValue, getChapterRoleLabels } from '../../shared/chapter-role'
import { cn } from '../../lib/utils'
import { confirm } from '../ui/Confirm'
import { useLocaleStore } from '../../stores/locale-store'
import { openPlanningRecoveryDraft, registerEditorExitSaveHandler, useEditorStore, type EditorTab, type PlanningRecoverySaveSnapshot } from '../../stores/editor-store'
import CodeMirrorEditor from './CodeMirrorEditor'
import { requireIpcSuccess } from '../../services/ipc-result'
import { SaveFeedback } from './save-feedback'
import { currentProjectSessionForPath, isCurrentProjectSession, useChapterBlueprints } from './use-chapter-blueprints'
import { pluralEn } from '../../shared/plural-en'

const ROLE_COLORS: Record<string, string> = {
  高潮: 'bg-red-500/20 text-[var(--color-error-text)]',
  冲突: 'bg-orange-500/20 text-[var(--color-warning-text)]',
  转折: 'bg-purple-500/20 text-[var(--color-category-review-text)]',
  建置: 'bg-blue-500/20 text-[var(--color-category-progress-text)]',
  收尾: 'bg-green-500/20 text-[var(--color-success-text)]',
}

/** 章节蓝图编辑器 — 读写 directory.json */
export default function ChapterCardEditor(props: { projectKey: string; initialChapterNumber?: number; selectRequest?: number; tabId?: string }) {
  const recoveryTab = useEditorStore(state => state.tabs.find(tab => tab.id === props.tabId && tab.planningRecovery))
  const currentLease = useProjectStore(state => state.currentProject?.sessionLease)
  return recoveryTab ? <BlueprintRecoveryEditor key={`${recoveryTab.id}:${currentLease}`} tab={recoveryTab} projectKey={props.projectKey} /> : <ChapterCardEditorSession {...props} />
}

function BlueprintRecoveryEditor({ tab, projectKey }: { tab: EditorTab; projectKey: string }) {
  const text = useLocaleStore(state => state.text)
  const [session] = useState(() => currentProjectSessionForPath(projectKey))
  const currentProject = useProjectStore(state => state.currentProject)
  const sessionCurrent = sameProjectSessionContext(session, projectSessionContextFromProject(currentProject))
  const planningTargetUnit = currentProject?.novelConfig.writingLanguage === 'en-US' ? text('词', 'words') : text('字', 'characters')
  const [context, setContext] = useState(tab.planningRecovery)
  const [end, setEnd] = useState(String(tab.planningSaveEnd ?? ''))
  const endRef = useRef(end)
  const [busy, setBusy] = useState(false)
  const busyRef = useRef(false)
  const [error, setError] = useState('')
  const currentTab = useEditorStore(state => state.tabs.find(item => item.id === tab.id)) ?? tab
  const refresh = useCallback(async () => {
    const expectedSession = session
    const original = tab.planningRecovery
    if (!expectedSession || !isCurrentProjectSession(expectedSession) || !original) throw new Error(text('项目会话已切换，请重新打开恢复稿', 'The project session changed. Reopen the recovery draft.'))
    const next = await ipc.invokeWithProjectSession(expectedSession, 'generation:read-context', { handle: original.handle })
    if (!isCurrentProjectSession(expectedSession)) throw new Error(text('项目会话已切换', 'The project session changed.'))
    setContext(next)
    return next
  }, [session, tab.planningRecovery, text])
  useEffect(() => { void Promise.resolve().then(refresh).catch(reason => setError(String(reason))) }, [refresh])
  const save = useCallback(async (propagate = false) => {
    if (busyRef.current) { if (propagate) throw new Error(text('保存仍在进行', 'Saving is still in progress.')); return }
    busyRef.current = true
    setBusy(true)
    const snapshot = useEditorStore.getState().tabs.find(item => item.id === tab.id)
    try {
      if (!session) throw new Error(text('项目会话已切换', 'The project session changed.'))
      const fresh = await refresh()
      const candidate = fresh.blueprintRecovery
      if (candidate?.writeState === 'unavailable') throw new Error(text('旧恢复稿不能保存或续接，可继续编辑或复制', 'This recovery draft cannot be saved or continued. You can still edit or copy it.'))
      const originalRange = tab.planningRecovery?.blueprintRecovery?.editRange
      if (!snapshot || !candidate || candidate.leaseEpoch !== session.leaseId || !originalRange || !snapshot.planningSaveOperationId
        || candidate.writeState !== 'ready' && !candidate.saved) throw new Error(text('当前无法保存，请等待生成结束或检查来源变化', 'Wait for generation to finish or check changed sources before saving.'))
      const pending = useEditorStore.getState().tabs.find(item => item.id === tab.id)?.planningSaveSnapshot
      if (pending?.status === 'saved' || candidate.saved && !pending) throw new Error(text('本次恢复已保存，后续修改仅供复制。', 'This recovery is saved. Further edits can only be copied.'))
      let submission = pending
      if (!submission) {
        const authorEnd = Number(endRef.current)
        if (!Number.isSafeInteger(authorEnd) || authorEnd < originalRange.from || authorEnd > originalRange.to) throw new Error(text('请选择连续完整章节的终点', 'Choose the last complete consecutive chapter.'))
        const blueprints = parseTextBlueprintsStrict(snapshot.content ?? '', originalRange.from, authorEnd, 'author')
        submission = {
          content: snapshot.content ?? '', contentRevision: snapshot.contentRevision ?? 0,
          status: 'pending', channel: 'db:blueprint-commit-range',
          request: { mode: 'replace-range', operationId: snapshot.planningSaveOperationId,
            startChapter: originalRange.from, endChapter: authorEnd, blueprints,
            authorRecovery: { sourceHandle: candidate.sourceHandle, leaseEpoch: candidate.leaseEpoch } },
        } satisfies PlanningRecoverySaveSnapshot
        useEditorStore.getState().setPlanningSaveSnapshot(tab.id, submission)
      }
      if (submission.channel !== 'db:blueprint-commit-range') throw new Error('INVALID_PLANNING_SAVE_CHANNEL')
      const result = await ipc.invokeWithProjectSession(session, 'db:blueprint-commit-range', {
        ...submission.request, authorRecovery: { ...submission.request.authorRecovery, leaseEpoch: candidate.leaseEpoch },
      }, projectKey)
      if (!result.success && !pending && isCurrentProjectSession(session)) useEditorStore.getState().setPlanningSaveSnapshot(tab.id, undefined)
      requireIpcSuccess(result, '保存恢复蓝图')
      if (!isCurrentProjectSession(session)) return
      useEditorStore.getState().setPlanningSaveSnapshot(tab.id, { ...submission, status: 'saved' })
      useEditorStore.getState().settleTabSave(tab.id, submission)
      setError('')
      await refresh()
    } catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); if (propagate) throw reason }
    finally { busyRef.current = false; setBusy(false) }
  }, [projectKey, refresh, session, tab.id, tab.planningRecovery, text])
  useEffect(() => {
    registerEditorExitSaveHandler({ tabId: tab.id, type: 'chapter-card', save: () => save(true) })
  }, [save, tab.id])
  const continuePlanning = async () => {
    if (busyRef.current) return
    busyRef.current = true; setBusy(true)
    try {
      if (!session) throw new Error(text('项目会话已切换', 'The project session changed.'))
      const fresh = await refresh()
      const continuation = fresh.planningContinuation
      if (!continuation?.remainingRange || !['ready', 'continued'].includes(continuation.state)) throw new Error(text('当前不能继续，请等待生成结束或检查来源变化', 'Wait for generation to finish or check changed sources before continuing.'))
      await launchCreativeWorkflow({ workflow: 'generate_blueprint', params: { mode: 'append',
        startChapter: continuation.remainingRange.from, count: continuation.remainingRange.to - continuation.remainingRange.from + 1,
        targetUnits: continuation.targetUnits, restartFrom: continuation.sourceHandle,
      } }, session)
      await refresh()
      setError('')
    } catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)) }
    finally { busyRef.current = false; setBusy(false) }
  }
  const candidate = context?.blueprintRecovery
  const continuation = context?.planningContinuation
  return <div className="h-full flex flex-col" aria-label={text('蓝图恢复', 'Blueprint recovery')}>
    <div className="p-3 text-xs space-y-2">
      {currentTab.planningSaveSnapshot?.status === 'saved' && <p>{text('本次恢复已保存，后续修改仅供复制。', 'This recovery is saved. Further edits can only be copied.')}</p>}
      {candidate?.writeState === 'unavailable' ? <>
        <p role="status">{candidate.diagnostic.code === 'GENERATION_BLUEPRINTS_CLEARED'
          ? text('蓝图已清空，旧恢复稿不能保存或续接', 'Blueprints were cleared. This recovery draft cannot be saved or continued.')
          : candidate.diagnostic.code === 'GENERATION_DIRECTORY_PROGRESS_INVALID'
            ? text('蓝图生成进度校验失败，旧恢复稿不能保存或续接', 'Blueprint generation progress failed validation. This recovery draft cannot be saved or continued.')
            : text('蓝图作者保存记录校验失败，旧恢复稿不能保存或续接', 'The blueprint author save record failed validation. This recovery draft cannot be saved or continued.')}</p>
        <p>{text('原文已保留，可继续编辑或复制', 'The original text is preserved. You can still edit or copy it.')}</p>
      </> : <p>{text('蓝图未完整生成，原文已保留。请补齐 JSON 中的引号、括号和必填内容，只保留要保存的连续完整章节。保存只写入本地，不会调用模型。', 'The incomplete blueprint text is preserved. Complete the JSON quotes, brackets and required fields. Keep only complete consecutive chapters to save. Saving is local and does not call a model.')}</p>}
      <label>{text('保存到第几章', 'Last complete chapter')} <input type="number" disabled={candidate?.writeState === 'unavailable'} min={tab.planningRecovery?.blueprintRecovery?.editRange?.from} max={tab.planningRecovery?.blueprintRecovery?.editRange?.to} value={end} onChange={event => {
        const value = event.target.value
        endRef.current = value; setEnd(value)
        useEditorStore.setState(state => ({ tabs: state.tabs.map(item => item.id === tab.id ? { ...item, planningSaveEnd: value } : item) }))
      }} /></label>
      <Button disabled={busy || !sessionCurrent || currentTab.planningSaveSnapshot?.status === 'saved' || candidate?.writeState !== 'ready' && !(candidate?.saved && currentTab.planningSaveSnapshot?.status === 'pending')} onClick={() => { void save() }}>{text('保存恢复蓝图', 'Save recovered blueprints')}</Button>
      <Button disabled={busy || !sessionCurrent || !continuation?.remainingRange || !['ready', 'continued'].includes(continuation.state)} onClick={() => { void continuePlanning() }}>{text('继续生成缺少的章节', 'Generate remaining chapters')}</Button>
      {continuation?.remainingRange && <p>{text(`待完成第 ${continuation.remainingRange.from} 至 ${continuation.remainingRange.to} 章，每章目标 ${continuation.targetUnits} ${planningTargetUnit}。`, `Chapters ${continuation.remainingRange.from} to ${continuation.remainingRange.to} remain. Target ${continuation.targetUnits} ${planningTargetUnit} per chapter.`)}</p>}
      {candidate && ['in-flight', 'source-changed', 'continued'].includes(candidate.writeState) && <p>{text('当前正式内容不能覆盖。可继续编辑或复制恢复稿。', 'Saved content cannot currently be replaced. You can still edit or copy this recovery draft.')}</p>}
      {error && <p role="alert">{error}</p>}
      <details><summary>{text('原始候选', 'Original candidate')}</summary><pre className="whitespace-pre-wrap max-h-40 overflow-auto">{tab.originalContent}</pre></details>
    </div>
    <div className="flex-1 min-h-0"><CodeMirrorEditor mode="document" content={currentTab.content ?? ''} hideStatusBar onChange={value => useEditorStore.getState().updateTabContent(tab.id, value)} onSave={() => save()} /></div>
  </div>
}

function ChapterCardEditorSession({
  projectKey,
  initialChapterNumber,
  selectRequest,
}: {
  projectKey: string
  initialChapterNumber?: number
  /** 再次请求选中同一章时递增，选中才会重新生效。 */
  selectRequest?: number
}) {
  const text = useLocaleStore(s => s.text)
  const locale = useLocaleStore(s => s.locale)
  const {
    loading,
    projectMatches,
    projectDataReady,
    blueprints: visibleBlueprints,
    selectedIdx,
    setSelectedIdx,
    selected,
    dirty: visibleDirty,
    saving,
    saveOutcome,
    nextWriteChapter,
    nextWritableBlueprint,
    authorityError,
    legacyImportedTextRecoveryChapter,
    recoveringLegacyImportedText,
    planningRecovery,
    reload: loadBlueprints,
    updateField,
    saveOne: handleSaveOne,
    saveAll: handleSaveAll,
    addChapter: handleAddChapter,
    deleteChapter,
    clearAll,
    generateBlueprints,
    writeChapter: handleWriteChapter,
    clearLegacyImportedText,
  } = useChapterBlueprints({ projectKey, initialChapterNumber, selectRequest })

  // 蓝图生成弹窗（替代原 inline 批量面板）
  const [showBlueprintDialog, setShowBlueprintDialog] = useState(false)
  const [showBatchCreationDialog, setShowBatchCreationDialog] = useState(false)

  const roleLabel = (role: string) => {
    const labels = getChapterRoleLabels(role)
    return labels ? text(labels.zhCN, labels.enUS) : role
  }

  /** 删除选中章节 */
  const handleDeleteChapter = async () => {
    if (!selected) return
    const ok = await confirm(text(
      `确认删除第 ${selected.chapterNumber} 章蓝图？\n此操作不可撤销。`,
      `Delete the blueprint for Chapter ${selected.chapterNumber}?\nThis cannot be undone.`,
    ), {
      title: text('删除章节蓝图', 'Delete chapter blueprint'),
      confirmText: text('删除', 'Delete'),
      danger: true,
    })
    if (ok) await deleteChapter(selected.chapterNumber)
  }

  /** 清空全部章节蓝图 */
  const handleClearAllBlueprints = async () => {
    if (visibleBlueprints.length === 0) return
    const ok = await confirm(text(
      `确认清空全部 ${visibleBlueprints.length} 章蓝图？\n此操作不可撤销，但不会删除草稿或正文章节。`,
      `Clear the ${visibleBlueprints.length} chapter ${pluralEn(visibleBlueprints.length, 'blueprint')}?\nThis cannot be undone, but drafts and manuscript chapters will remain.`,
    ), {
      title: text('清空全部蓝图', 'Clear all blueprints'),
      confirmText: text('清空全部', 'Clear all'),
      danger: true,
    })
    if (ok) await clearAll()
  }

  /** 触发蓝图批量生成（来自 DirectoryConfigDialog 的确认回调） */
  const handleBatchGenerate = (params: DirectoryWorkflowParams) => generateBlueprints(params, message => confirm(text(
    `${message}\n\n是否仍要继续生成？`,
    'A precondition warning was reported. Continue generating anyway?',
  ), {
    title: text('前置条件警告', 'Precondition warning'),
    confirmText: text('继续生成', 'Continue'),
  }))

  /**
   * 旧版“小说拆解与仿写”曾把参考原文误写为草稿和定稿；此处只给用户一个
   * 明确确认后的恢复入口，不尝试自动判定或删除任何项目内容。
   */
  const handleClearLegacyImportedText = async () => {
    const ok = await confirm(text(
      '仅当当前草稿和正文来自旧版“小说拆解与仿写”导入时，才继续清除。\n\n此操作会永久清除草稿、定稿、审稿和摘要等正文产物；会保留角色、故事架构、章节蓝图与知识库。若只是尚未生成下一章蓝图，请取消并先补充蓝图。',
      'Continue only if the current drafts and manuscript text came from a legacy “Novel analysis and imitation” import.\n\nThis permanently clears drafts, final manuscript text, reviews, and summaries. Characters, story architecture, chapter blueprints, and the knowledge base are kept. If the next blueprint is simply missing, cancel and add that blueprint first.',
    ), {
      title: text('清除误导入正文', 'Clear incorrectly imported text'),
      confirmText: text('清除误导入正文', 'Clear incorrectly imported text'),
      danger: true,
    })
    if (ok) await clearLegacyImportedText()
  }


  if (loading) {
    return (
      <div className="flex items-center justify-center h-full gap-2" style={{ color: 'var(--color-text-muted)' }}>
        <RefreshCw size={16} className="animate-spin" /> {text('加载章节蓝图...', 'Loading chapter blueprints...')}
      </div>
    )
  }

  if (!projectMatches) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-3 opacity-40">
        <BookOpen size={36} />
        <span className="text-sm">{text('此标签属于另一个项目，请切回原项目后继续。', 'This tab belongs to another project. Switch back to continue.')}</span>
      </div>
    )
  }

  const canRecoverLegacyImportedText = legacyImportedTextRecoveryChapter !== null

  return (
    <div className="h-full flex flex-col overflow-hidden">
      {/* 顶部工具栏 */}
      <div
        className="flex items-center justify-between gap-2 px-3 h-10 flex-shrink-0 border-b"
        style={{ borderColor: 'var(--color-border)', backgroundColor: 'var(--color-sidebar)' }}
      >
        <div className="flex items-center gap-1.5">
          <BookOpen size={13} style={{ color: 'var(--color-text-muted)' }} />
          <span className="text-sm font-medium" style={{ color: 'var(--color-text)' }}>
            {text('章节蓝图', 'Chapter blueprints')}
            {visibleBlueprints.length > 0 && (
              <span style={{ color: 'var(--color-text-muted)' }} className="ml-1 font-normal">
                {text(`(${visibleBlueprints.length} 章)`, `(${visibleBlueprints.length} ${pluralEn(visibleBlueprints.length, 'chapter')})`)}
              </span>
            )}
          </span>
          {visibleDirty && (
            <span className="inline-flex items-center gap-1 text-[0.7rem]" style={{ color: 'var(--color-accent)' }}>
              <span
                aria-hidden="true"
                className="h-1.5 w-1.5 rounded-full"
                style={{ backgroundColor: 'currentColor' }}
              />
              {text('未保存', 'Unsaved')}
            </span>
          )}
          <SaveFeedback dirty={visibleDirty} saving={saving} outcome={saveOutcome} />
          {planningRecovery && <Button size="sm" onClick={() => {
            const session = currentProjectSessionForPath(projectKey)
            if (session && isCurrentProjectSession(session)) openPlanningRecoveryDraft(planningRecovery, projectKey, text('蓝图恢复稿', 'Blueprint recovery draft'))
          }}>{text('编辑恢复稿', 'Edit recovery draft')}</Button>}
        </div>
        <div className="flex items-center gap-1">
          {/* 写作入口 — 仅下一章可写时显示 */}
          {projectDataReady && nextWritableBlueprint && (
            <Button
              variant="ai"
              size="sm"
              onClick={() => handleWriteChapter(nextWritableBlueprint)}
            >
              <PenLine size={12} />
              {text(`写作第${nextWritableBlueprint.chapterNumber}章`, `Write Chapter ${nextWritableBlueprint.chapterNumber}`)}
            </Button>
          )}
          {projectDataReady && nextWritableBlueprint && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => setShowBatchCreationDialog(true)}
              title={text('按连续章节蓝图启动受控批量创作任务（最高10章）', 'Start a controlled batch writing task from consecutive chapter blueprints (maximum 10 chapters).')}
            >
              <ListChecks size={12} />
              {text('批量创作', 'Batch write')}
            </Button>
          )}
          {/* AI 生成蓝图 → 弹出 DirectoryConfigDialog */}
          <Button
            variant="ai"
            size="sm"
            onClick={() => setShowBlueprintDialog(true)}
            disabled={!projectDataReady || Boolean(authorityError)}
            title={text('AI 生成章节蓝图（选择范围和模式）', 'Generate chapter blueprints with AI (choose the range and mode)')}
          >
            <Sparkles size={12} />
            {text('AI 生成蓝图', 'AI generate blueprints')}
          </Button>
          <Button variant="ghost" size="icon" onClick={() => loadBlueprints()} title={text('重新加载', 'Reload')} disabled={loading}>
            <RefreshCw size={14} className={loading ? "animate-spin" : ""} />
          </Button>
          <Button variant="ghost" size="icon" onClick={handleAddChapter} disabled={!projectDataReady || nextWriteChapter === null || Boolean(authorityError)} title={text('新建章节', 'New chapter')}>
            <Plus size={14} />
          </Button>
          <Button
            variant="destructive"
            size="sm"
            onClick={handleClearAllBlueprints}
            disabled={saving || visibleBlueprints.length === 0 || !projectDataReady}
            title={text('清空全部章节蓝图', 'Clear all chapter blueprints')}
          >
            <Trash2 size={12} />
            {text('清空全部蓝图', 'Clear all blueprints')}
          </Button>
          {visibleDirty && (
            <Button variant="outline" size="sm" onClick={() => { void handleSaveAll() }} disabled={saving || !projectDataReady}>
            <Save size={12} /> {saving ? text('保存中...', 'Saving...') : text('保存全部', 'Save all')}
            </Button>
          )}
        </div>
      </div>

      {canRecoverLegacyImportedText && (
        <div
          className="flex flex-wrap items-center justify-between gap-3 border-b px-3 py-2 text-xs"
          style={{
            borderColor: 'color-mix(in srgb, var(--color-warning) 42%, var(--color-border))',
            backgroundColor: 'color-mix(in srgb, var(--color-warning) 8%, transparent)',
          }}
        >
          <div className="flex min-w-0 items-start gap-2" style={{ color: 'var(--color-text-secondary)' }}>
            <AlertTriangle size={15} className="mt-0.5 flex-shrink-0" style={{ color: 'var(--color-warning)' }} />
            <p className="max-w-3xl leading-5">
              {text(
                `检测到后续正文但第 ${legacyImportedTextRecoveryChapter} 章尚未写作，可能是旧版“小说拆解与仿写”误导入的参考原文。系统不会自动清除任何内容；确认“清除误导入正文”后会保留角色、故事架构、章节蓝图与知识库，并可从第 ${legacyImportedTextRecoveryChapter} 章开始写作。`,
                `Later manuscript text exists while Chapter ${legacyImportedTextRecoveryChapter} has not been written. This may be reference text incorrectly imported by a legacy “Novel analysis and imitation” workflow. Nothing is cleared automatically; after you confirm “Clear incorrectly imported text”, characters, story architecture, chapter blueprints, and the knowledge base are kept, and you can start writing from Chapter ${legacyImportedTextRecoveryChapter}.`,
              )}
            </p>
          </div>
          <Button
            variant="destructive"
            size="sm"
            onClick={handleClearLegacyImportedText}
            disabled={recoveringLegacyImportedText}
          >
            <Trash2 size={12} />
            {recoveringLegacyImportedText
              ? text('清除中...', 'Clearing...')
              : text('清除误导入正文', 'Clear incorrectly imported text')}
          </Button>
        </div>
      )}

      {projectDataReady && authorityError && !canRecoverLegacyImportedText && (
        <div
          className="flex items-start gap-2 border-b px-3 py-2 text-xs"
          style={{
            color: 'var(--color-warning-text)',
            borderColor: 'color-mix(in srgb, var(--color-warning) 42%, var(--color-border))',
            backgroundColor: 'color-mix(in srgb, var(--color-warning) 8%, transparent)',
          }}
        >
          <AlertTriangle size={15} className="mt-0.5 flex-shrink-0" style={{ color: 'var(--color-warning)' }} />
          <p className="leading-5">{authorityError}</p>
        </div>
      )}

      {/* 蓝图生成配置弹窗 */}
        <DirectoryConfigDialog
        isOpen={showBlueprintDialog}
        onClose={() => setShowBlueprintDialog(false)}
        existingCount={visibleBlueprints.length}
          onConfirm={handleBatchGenerate}
        />
          <BatchChapterCreationDialog
          isOpen={showBatchCreationDialog}
          startChapterNumber={nextWritableBlueprint?.chapterNumber ?? null}
          onClose={() => setShowBatchCreationDialog(false)}
        />

      {/* 主区域：左侧列表 + 右侧编辑 */}
      <div className="flex-1 flex overflow-hidden">
        {/* 左侧章节列表 */}
        <div
          className="flex flex-col flex-shrink-0 w-[200px] border-r overflow-hidden"
          style={{ borderColor: 'var(--color-border)', backgroundColor: 'var(--color-sidebar)' }}
        >
          {visibleBlueprints.length === 0 ? (
            <div className="flex flex-col items-center justify-center flex-1 gap-3 opacity-40 p-4">
              <BookOpen size={28} />
              <span className="text-xs text-center">{text(
                '暂无蓝图，可点击右上角「+」手动新建，或用「AI 生成蓝图」批量创建。',
                'No blueprints yet. Use “+” to add one manually, or “AI generate blueprints” to create a batch.',
              )}</span>
            </div>
          ) : (
          <div className="flex-1 overflow-y-auto p-1">
            {visibleBlueprints.map((bp, idx) => (
              <div
                key={bp.chapterNumber}
                className={cn(
                  'group relative px-2.5 py-2 rounded-md text-xs cursor-pointer mb-0.5 transition-colors',
                  selectedIdx === idx
                    ? 'bg-[var(--color-active)] text-[var(--color-text)]'
                    : 'text-[var(--color-text-secondary)] hover:bg-[var(--color-hover)]'
                )}
                onClick={() => setSelectedIdx(idx)}
              >
                <div className="flex items-center gap-1.5">
                  <span className="font-mono text-[0.7rem] opacity-40 flex-shrink-0">
                    {bp.chapterNumber}
                  </span>
                  <span className="font-medium truncate flex-1">{bp.title || text('未命名', 'Untitled')}</span>
                </div>
                <div className="flex items-center gap-1 mt-0.5">
                  <span className={cn(
                    'text-[0.7rem] px-1 py-0.5 rounded',
                    ROLE_COLORS[bp.role] || 'bg-[var(--color-hover)] text-[var(--color-text-muted)]'
                  )}>
                    {roleLabel(bp.role)}
                  </span>
                  {bp.userGuidance && (
                    <span
                      className="text-[0.7rem] px-1 py-0.5 rounded"
                      style={{ backgroundColor: 'rgba(var(--accent-rgb), 0.15)', color: 'var(--color-accent)' }}
                      title={text('已有作者微操指导', 'Author guidance is available')}
                    >
                      {text('有指导', 'Guidance')}
                    </span>
                  )}
                  {bp.notes && (
                    <span
                      className="text-[0.7rem] px-1 py-0.5 rounded"
                      style={{ backgroundColor: 'rgba(34,197,94,0.15)', color: 'rgb(34,197,94)' }}
                      title={text('已生成章节要点', 'Chapter notes are available')}
                    >
                      {text('有要点', 'Notes')}
                    </span>
                  )}
                </div>
              </div>
            ))}
          </div>
          )}
        </div>

        {/* 右侧编辑区 */}
        <div className="flex-1 overflow-y-auto">
          {selected ? (
            <div className="max-w-2xl mx-auto px-5 py-4">
              {/* 编辑区头部 */}
              <div className="flex items-center justify-between mb-4">
                <h3 className="text-sm font-bold" style={{ color: 'var(--color-text)' }}>
                  {text(
                    `第 ${selected.chapterNumber} 章：${selected.title || '未命名'}`,
                    `Chapter ${selected.chapterNumber}: ${selected.title || 'Untitled'}`,
                  )}
                </h3>
                <div className="flex items-center gap-1.5">
                  {/* 仅下一章允许写作 */}
                  {nextWritableBlueprint && selected.chapterNumber === nextWritableBlueprint.chapterNumber && (
                    <Button
                      variant="ai"
                      size="sm"
                      onClick={() => handleWriteChapter(selected)}
                      title={text('以当前蓝图信息生成草稿', 'Create a draft from this blueprint')}
                    >
                      <PenLine size={12} /> {text('写作此章', 'Write this chapter')}
                    </Button>
                  )}
                  <Button variant="destructive" size="sm" onClick={handleDeleteChapter} title={text('删除此章', 'Delete this chapter')}>
                    <Trash2 size={12} />
                    {text('删除此章', 'Delete chapter')}
                  </Button>
                  <Button variant="outline" size="sm" onClick={() => { void handleSaveOne() }} disabled={saving}>
                    <Save size={12} /> {saving ? text('保存中...', 'Saving...') : text('保存', 'Save')}
                  </Button>
                </div>
              </div>

              <div className="space-y-3">
                {/* 基本信息 */}
                <div className="grid grid-cols-3 gap-3">
                  <div>
                    <Label>{text('章节号', 'Chapter number')}</Label>
                    <Input
                      type="number"
                      value={selected.chapterNumber}
                      readOnly
                      aria-readonly="true"
                      title={text('章节号是现有内容的稳定标识，不能在普通编辑中修改', 'The chapter number is a stable identifier and cannot be changed in ordinary editing.')}
                    />
                  </div>
                  <div className="col-span-2">
                    <Label>{text('章节标题', 'Chapter title')}</Label>
                    <Input
                      value={selected.title}
                      onChange={e => updateField('title', e.target.value)}
                      placeholder={text('引人入胜的章节标题', 'A compelling chapter title')}
                    />
                  </div>
                </div>

                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <Label>{text('章节定位', 'Chapter role')}</Label>
                    <NativeSelect value={chapterRoleSelectValue(selected.role)} onChange={e => updateField('role', e.target.value)}>
                      {chapterRoleOptions(selected.role).map(({ value, labels }) => (
                        <option key={value} value={value}>{labels ? text(labels.zhCN, labels.enUS) : value}</option>
                      ))}
                    </NativeSelect>
                  </div>
                  <div>
                    <Label>{text('出场关键人（逗号分隔）', 'Key characters (comma-separated)')}</Label>
                    <Input
                      key={selected.chapterNumber}
                      defaultValue={selected.characters.join('、')}
                      onChange={e => updateField('characters', e.target.value.split(/[,，、\s]+/).filter(Boolean))}
                      placeholder={text('如：主角、反派A', 'For example: protagonist, antagonist A')}
                    />
                  </div>
                </div>

                <div>
                  <Label>{text('主角小目标（本章最想解决的事）', 'Protagonist goal (the main thing to resolve in this chapter)')}</Label>
                  <Textarea
                    value={selected.purpose}
                    onChange={e => updateField('purpose', e.target.value)}
                    placeholder={text('本章主角最迫切要解决的一件事...', 'The most urgent thing the protagonist needs to resolve in this chapter...')}
                    rows={2}
                  />
                </div>

                <div>
                  <Label>{text('实质冲突与转折', 'Core conflict and turning point')}</Label>
                  <Textarea
                    value={selected.keyEvents}
                    onChange={e => updateField('keyEvents', e.target.value)}
                    placeholder={text('主角做了什么，遭遇了什么反转，金手指怎么用的...', 'What the protagonist does, the reversal they encounter, and how special abilities are used...')}
                    rows={4}
                  />
                </div>

                <div>
                  <Label>{text('末尾悬念钩子', 'Ending suspense hook')}</Label>
                  <Textarea
                    value={selected.suspenseHook}
                    onChange={e => updateField('suspenseHook', e.target.value)}
                    placeholder={text('一句话说明结尾留了什么悬念...', 'In one sentence, describe the suspense left at the end...')}
                    rows={2}
                  />
                </div>

                {/* 作者微操指导 — 特别标注，写稿时注入为最高优先级 */}
                <div
                  className="p-3 rounded-lg border"
                  style={{
                    borderColor: 'var(--color-accent)',
                    backgroundColor: 'rgba(var(--accent-rgb, 99 102 241), 0.06)',
                  }}
                >
                  <Label className="flex items-center gap-1.5">
                    <span>{text('作者微操指导', 'Author guidance')}</span>
                    <span
                      className="text-[0.7rem] font-normal"
                      style={{ color: 'var(--color-text-muted)' }}
                    >
                      {text('（写稿时会作为最高优先级注入 AI — 可覆盖蓝图）', '(Used as the highest-priority instruction during drafting; it can override the blueprint.)')}
                    </span>
                  </Label>
                  <Textarea
                    value={selected.userGuidance}
                    onChange={e => updateField('userGuidance', e.target.value)}
                    placeholder={text(
                      '我想在这章加入一个意外的背叛...\n让反派在这章露出破绽...\n（不填则完全按蓝图走）',
                      'Add an unexpected betrayal in this chapter...\nLet the antagonist reveal a weakness...\n(Leave blank to follow the blueprint exactly.)',
                    )}
                    rows={3}
                    style={{ marginTop: 6 }}
                  />
                </div>
                {/* 章节要点（定稿后自动生成，也可手动编辑） */}
                <div
                  className="p-3 rounded-lg border"
                  style={{
                    borderColor: 'var(--color-border)',
                    backgroundColor: 'rgba(34,197,94,0.04)',
                  }}
                >
                  <Label className="flex items-center gap-1.5">
                    <span>{text('章节要点', 'Chapter notes')}</span>
                    <span
                      className="text-[0.7rem] font-normal"
                      style={{ color: 'var(--color-text-muted)' }}
                    >
                      {selected.notesUpdatedAt
                        ? text(
                          `（定稿后自动生成 — ${new Date(selected.notesUpdatedAt).toLocaleDateString(locale)}）`,
                          `(Generated after finalization — ${new Date(selected.notesUpdatedAt).toLocaleDateString(locale)})`,
                        )
                        : text('（定稿后自动生成，也可手动填写）', '(Generated after finalization, or enter it manually.)')
                      }
                    </span>
                  </Label>
                  <Textarea
                    value={selected.notes || ''}
                    onChange={e => updateField('notes', e.target.value)}
                    placeholder={text(
                      '定稿后 AI 会自动填充本章要点（事件进展/角色变化/伏笔埋点），也可以提前手动输入给 AI 作参考',
                      'After finalization, AI fills these notes with plot progress, character changes, and foreshadowing. You can also enter them beforehand as AI reference.',
                    )}
                    rows={4}
                  />
                </div>
              </div>
            </div>
          ) : (
            <div className="flex flex-col items-center justify-center h-full gap-3 opacity-30">
              <BookOpen size={36} />
              <span className="text-sm">{text('在左侧选择一章开始编辑', 'Choose a chapter on the left to start editing')}</span>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
