import { formatResourceUri } from '../../shared/project-paths'
import { useState } from 'react'
import { Sparkles, Search, BadgeCheck, Save, FileStack, FileText, Wrench, Check } from 'lucide-react'

import { useProjectStore } from '../../stores/project-store'
import { useEditorStore } from '../../stores/editor-store'
import { useLocaleStore } from '../../stores/locale-store'
import CodeMirrorEditor from './CodeMirrorEditor'
import ThreeWayMerge from './ThreeWayMerge'
import { Button } from '../ui/Button'
import { confirm } from '../ui/Confirm'
import {
  Dialog, DialogContent, DialogHeader, DialogFooter, DialogTitle, DialogDescription,
} from '../ui/Dialog'
import { readDraftBody } from '../../stores/draft-store'
import { DRAFT_STATUS_LABEL, DRAFT_STATUS_COLOR } from '../../shared/draft-status'
import { PostProcessStatusPanel } from '../ui/PostProcessStatusPanel'
import { SaveFeedback } from './save-feedback'
import { getChapterFinalizeScope } from '../../services/workflows/workflow-utils'
import { reviewDimensions, useDraftActions } from './use-draft-actions'
import { pluralEn } from '../../shared/plural-en'
import {
  captureProjectSession,
  isProjectSessionCurrent,
  isProjectSessionPath,
} from '../project-session-gate'

const DRAFT_STATUS_EN: Record<string, string> = {
  draft: 'Draft',
  revised: 'Revised',
  reviewed: 'Reviewed',
  finalized: 'Finalized',
  archived: 'Archived',
}

interface Props {
  tabId: string
  filePath: string
  content: string
  projectKey: string
}

/**
 * 草稿编辑器
 * — 顶部工具栏：草稿状态 + 待合并修稿 + AI 修稿(含自定义提示词) / AI 审稿 / 定稿
 * — 正文：CodeMirrorEditor（prose 模式）
 */
export default function DraftEditor(props: Props) {
  const currentProject = useProjectStore(s => s.currentProject)
  const projectSession = captureProjectSession(currentProject)
  const sessionKey = projectSession && isProjectSessionPath(projectSession, props.projectKey)
    ? `${projectSession.projectId}:${projectSession.leaseId}`
    : `inactive:${props.projectKey}`

  // 同一路径重新打开会生成新 lease；用会话键重挂载，避免旧会话的本地 UI 状态短暂显示。
  return <DraftEditorSession key={sessionKey} {...props} />
}

function DraftEditorSession({ tabId, filePath, content, projectKey }: Props) {
  const text = useLocaleStore(s => s.text)
  const locale = useLocaleStore(s => s.locale)
  const draft = useDraftActions({ tabId, filePath, content, projectKey })
  const {
    meta, status, isReadonly, isChapterBusy, isDirty, finalizationPending, finalizationConflict,
    pendingRevisions, saving, saveOutcome, currentBodyRef, mergeData,
  } = draft
  const reviewCount = draft.reviews.length
  // 后处理失败状态（用于控制是否展示修复按钮）
  const [hasProcessFailure, setHasProcessFailure] = useState(false)
  const [confirmAction, setConfirmAction] = useState<'refine' | 'review' | null>(null)
  const [userRefinePrompt, setUserRefinePrompt] = useState('')
  // 审稿维度多选
  const REVIEW_DIMS = reviewDimensions(text)
  const [reviewDims, setReviewDims] = useState<Record<string, boolean>>(
    Object.fromEntries(REVIEW_DIMS.map(d => [d.key, true]))
  )
  const [charCount, setCharCount] = useState(0)

  const doSave = draft.save
  const doRefine = () => draft.refine(userRefinePrompt)
  const doReview = () => draft.review(REVIEW_DIMS.filter(d => reviewDims[d.key]).map(d => d.promptLabel).join('、') || undefined)
  const doRetryManuscriptPublication = draft.retryManuscriptPublication
  const doRepairFinalize = draft.repairFinalize
  const openPendingRevision = draft.openPendingRevision
  const handleMergeComplete = (mergedText: string) => { void draft.completeMerge(mergedText) }

  /** 定稿 */
  const doFinalize = async () => {
    if (!meta || !draft.canFinalize) return
    const ok = await confirm(
      text(
        `确定要将第 ${meta.chapterNumber} 章定稿吗？\n\n定稿后章节将标记为完成，不再支持修改和重新后处理。`,
        `Finalize Chapter ${meta.chapterNumber}?\n\nIt will be marked complete and can no longer be edited or post-processed again.`,
      ),
      {
        title: text('确认定稿', 'Confirm finalization'),
        confirmText: text('确认定稿', 'Finalize'),
      }
    )
    if (ok) await draft.finalize()
  }

  /** 打开最新的审稿报告 */
  const openLatestReview = async () => {
    const projectSession = captureProjectSession(useProjectStore.getState().currentProject)
    if (!meta || !projectSession || !isProjectSessionPath(projectSession, projectKey)) return
    const chapterDir = `ai-novel://draft/ch${meta.chapterNumber}`
    const { getLatestReview } = await import('../../services/draft-index')
    if (!isProjectSessionCurrent(projectSession)) return
    const latest = await getLatestReview(chapterDir, meta.version, projectKey)
    if (!isProjectSessionCurrent(projectSession)) return
    if (!latest) return

    // 使用 review 的数据库 ID 读取审稿报告内容
    const reportContent = await readDraftBody(formatResourceUri({ kind: 'review', id: latest.id }), projectKey, projectSession)
    if (!isProjectSessionCurrent(projectSession)) return
    if (!reportContent) return

    useEditorStore.getState().openFile({
      id: `review-report-${meta.chapterNumber}-${latest.id}`,
      name: text(`审稿报告 v${meta.version}`, `Review report v${meta.version}`),
      type: 'review-report',
      content: reportContent,
      filePath,
      reviewReport: reportContent,
      chapterNumber: meta.chapterNumber,
      chapterDir,
      draftId: meta.id,
      reviewId: latest.id,
      projectKey,
    })
  }

  return (
    <div className="h-full flex flex-col overflow-hidden">
      {/* 顶部工具栏 */}
      <div
        className="flex items-center justify-between gap-2 px-3 h-9 flex-shrink-0"
        style={{
          borderBottom: '1px solid var(--color-border)',
          backgroundColor: 'var(--color-editor-bg)',
        }}
      >
        {/* 左侧：章节标题 + 版本 */}
        <div className="flex items-center gap-1.5 min-w-0">
          <span className="text-xs font-medium truncate" style={{ color: 'var(--color-text-secondary)' }}>
              {meta
                ? text(`第 ${meta.chapterNumber} 章 — ${meta.chapterTitle || '未知标题'}`, `Chapter ${meta.chapterNumber} — ${meta.chapterTitle || 'Untitled'}`)
                : text('草稿', 'Draft')}
          </span>
          {meta && (
            <span className="text-[0.7rem] flex-shrink-0" style={{ color: 'var(--color-text-muted)' }}>
              v{meta.version}
            </span>
          )}
        </div>

        {/* 右侧：字数 + 状态 + 待合并 + AI操作 + 定稿 */}
        {!isReadonly && (
          <div className="flex items-center gap-1.5 flex-shrink-0">
            {/* 字数 */}
            {charCount > 0 && (
              <span className="text-xs tabular-nums mr-1" style={{ color: 'var(--color-text-muted)' }}>
                {text(`${charCount.toLocaleString(locale)} 字`, `${charCount.toLocaleString(locale)} ${pluralEn(charCount, 'word')}`)}
              </span>
            )}

            {/* 未保存指示灯 */}
            {isDirty && (
              <span
                className="w-1.5 h-1.5 rounded-full flex-shrink-0 mr-0.5"
                style={{ backgroundColor: 'var(--color-warning)' }}
                title={text('有未保存的修改', 'There are unsaved changes')}
              />
            )}

            <SaveFeedback dirty={isDirty} saving={saving} outcome={saveOutcome} />

            {/* 保存按钮 */}
            {isDirty && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => { void doSave(currentBodyRef.current) }}
                disabled={saving}
                title={text('保存（⌘S）', 'Save (Ctrl+S)')}
              >
                <Save size={12} />
                {saving ? text('保存中...', 'Saving...') : text('保存', 'Save')}
              </Button>
            )}

            {/* 状态标签 */}
            <span
              className="text-[0.7rem] px-1.5 py-0.5 rounded flex-shrink-0"
              style={{
                backgroundColor: 'var(--color-hover)',
                color: DRAFT_STATUS_COLOR[status] ?? 'var(--color-text-muted)',
              }}
            >
              {text(DRAFT_STATUS_LABEL[status] ?? status, DRAFT_STATUS_EN[status] ?? status)}
            </span>

            {finalizationConflict && (
              <span
                className="text-[0.7rem] px-1.5 py-0.5 rounded flex-shrink-0"
                style={{ color: 'var(--color-warning-text)', backgroundColor: 'var(--color-hover)' }}
                title={text(
                  '定稿完成事件没有覆盖这次编辑；请先处理本地后续修改与已定稿版本的差异。',
                  'Finalization did not overwrite this edit. Resolve the difference between your later local changes and the finalized version first.',
                )}
              >
                {text('已保留后续编辑', 'Later edits kept')}
              </span>
            )}

            {finalizationPending && (
              <Button
                variant="outline"
                size="sm"
                onClick={doRetryManuscriptPublication}
                title={text('定稿已提交、实体稿待发布；只重试已提交的发布记录', 'Finalization is submitted and the manuscript is pending publication. Retry only the submitted publication.')}
              >
                <Wrench size={12} />
                {text('重试实体稿', 'Retry manuscript')}
              </Button>
            )}

            {/* Pending revisions */}
            {pendingRevisions.length > 0 && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => openPendingRevision(pendingRevisions[0])}
                title={text('有待合并的修稿，点击打开三栏合并视图', 'Open the three-way merge view for pending revisions')}
              >
                <FileStack size={12} />
                {text(`待合并(${pendingRevisions.length})`, `Pending (${pendingRevisions.length})`)}
              </Button>
            )}

            {/* Review report */}
            {reviewCount > 0 && (
              <Button
                variant="outline"
                size="sm"
                onClick={openLatestReview}
                title={text('查看最新审稿报告', 'View the latest review report')}
              >
                <FileText size={12} />
                {text(`审稿报告(${reviewCount})`, `Review report (${reviewCount})`)}
              </Button>
            )}

            {/* AI 修稿 */}
            <Button
              variant="ai"
              size="sm"
              onClick={() => { setUserRefinePrompt(''); setConfirmAction('refine') }}
              disabled={isChapterBusy}
                title={text('AI 修稿 — 专业精修章节，生成修订并打开合并视图', 'AI revision — professionally refine the chapter, create a revision, and open the merge view')}
            >
              <Sparkles size={12} />
                {text('AI 修稿', 'AI revise')}
            </Button>

            {/* AI 审稿 */}
            <Button
              variant="ai"
              size="sm"
              onClick={() => setConfirmAction('review')}
              disabled={isChapterBusy}
                title={text('AI 审稿 — 一致性检查，生成审稿报告', 'AI review — run a consistency check and create a review report')}
            >
              <Search size={12} />
                {text('AI 审稿', 'AI review')}
            </Button>

            {/* 定稿 */}
            <Button
              variant="success"
              size="sm"
              onClick={doFinalize}
              disabled={isChapterBusy || !!finalizationConflict || finalizationPending}
                title={text('定稿 — 确认终稿并写入正文章节', 'Finalize — confirm the final draft and write it to the manuscript')}
            >
              <BadgeCheck size={12} />
                {text('定稿', 'Finalize')}
            </Button>
          </div>
        )}

        {/* 已定稿/归档显示只读提示 */}
        {isReadonly && (
          <div className="flex items-center gap-2 flex-shrink-0">
            {charCount > 0 && (
              <span className="text-xs tabular-nums" style={{ color: 'var(--color-text-muted)' }}>
                {text(`${charCount.toLocaleString(locale)} 字`, `${charCount.toLocaleString(locale)} ${pluralEn(charCount, 'word')}`)}
              </span>
            )}
            <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
              {status === 'finalized'
                ? text('已定稿（只读）', 'Finalized (read-only)')
                : text('已归档（只读）', 'Archived (read-only)')}
            </span>
            {finalizationPending && (
              <Button
                variant="outline"
                size="sm"
                onClick={doRetryManuscriptPublication}
                title={text('定稿已提交、实体稿待发布；只重试已提交的发布记录', 'Finalization is submitted and the manuscript is pending publication. Retry only the submitted publication.')}
              >
                <Wrench size={11} />
                {text('重试实体稿', 'Retry manuscript')}
              </Button>
            )}
            {/* 已定稿 → 有失败项时显示修复定稿按钮 */}
            {status === 'finalized' && meta && hasProcessFailure && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => void doRepairFinalize()}
                disabled={isChapterBusy}
                title={text('重新执行失败的后处理步骤（角色卡、知识库等）', 'Retry failed post-processing steps such as character cards and knowledge indexing')}
              >
                <Wrench size={11} />
                {text('修复定稿', 'Repair finalization')}
              </Button>
            )}
          </div>
        )}
      </div>

      {/* 后处理状态面板（仅定稿草稿显示） */}
      {status === 'finalized' && meta && (
        <div className="px-3 py-1.5" style={{ borderBottom: '1px solid var(--color-border)' }}>
          <PostProcessStatusPanel
            scope={getChapterFinalizeScope(meta.chapterNumber)}
            onRetry={doRepairFinalize}
            onStatusLoad={setHasProcessFailure}
          />
        </div>
      )}

      {/* 正文区 */}
      <div className="flex-1 overflow-hidden relative">
        <CodeMirrorEditor
          mode="prose"
          content={content}
          filePath={filePath}
          editable={!isReadonly && !isChapterBusy}
          hideStatusBar
          onCharCountChange={setCharCount}
          onChange={draft.onBodyChange}
          onSave={(text) => doSave(text)}
          paperHead={meta?.chapterTitle ? {
            title: meta.chapterTitle,
            subtitle: text(`第 ${meta.chapterNumber} 章`, `Chapter ${meta.chapterNumber}`),
          } : undefined}
        />


      </div>

      {/* AI 操作确认弹窗（修稿含自定义提示词输入框） */}
      <Dialog open={confirmAction !== null} onOpenChange={(v) => !v && setConfirmAction(null)}>
        <DialogContent className="max-w-[440px]">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Sparkles size={15} className="text-[var(--color-accent)]" />
              {confirmAction === 'refine'
                ? text('AI 修稿确认', 'Confirm AI revision')
                : text('AI 审稿确认', 'Confirm AI review')}
            </DialogTitle>
            <DialogDescription>
              {text('对象：', 'Target: ')}{meta
                ? `${meta.chapterTitle || text('未知标题', 'Untitled')} v${meta.version}`
                : text('当前草稿', 'Current draft')}
            </DialogDescription>
          </DialogHeader>
          <div className="px-5 py-2 text-sm space-y-1.5" style={{ color: 'var(--color-text-secondary)' }}>
            {confirmAction === 'refine' ? (
              <>
                <div className="font-medium text-[var(--color-text)]">{text('本次【直接修稿】范围：', 'This direct revision will:')}</div>
                <div>{text('1. 全文基础润色、词汇优化，增强画面与表现力。', '1. Polish the full chapter, improve wording, and strengthen imagery and expression.')}</div>
                <div>{text('2. 可在下方指定的额外修稿要求。', '2. Follow any additional revision instructions below.')}</div>
              </>
            ) : (
              <>
                <div>{text('将调用 AI 对本章草稿进行一致性检查，并生成审稿报告。', 'AI will check this chapter for consistency and generate a review report.')}</div>
                <div className="mt-3">
                  <div className="text-xs font-medium mb-2" style={{ color: 'var(--color-text)' }}>{text('重点检查维度：', 'Review focus:')}</div>
                  <div className="flex flex-wrap gap-2">
                    {REVIEW_DIMS.map(d => (
                      <label
                        key={d.key}
                        className="flex items-center gap-1.5 cursor-pointer select-none px-2 py-1 rounded-md text-xs"
                        style={{
                          border: `1px solid ${reviewDims[d.key] ? 'var(--color-accent)' : 'var(--color-border)'}`,
                          backgroundColor: reviewDims[d.key] ? 'rgba(var(--color-accent-rgb),0.1)' : 'transparent',
                          color: reviewDims[d.key] ? 'var(--color-accent)' : 'var(--color-text-muted)',
                        }}
                        onClick={() => setReviewDims(prev => ({ ...prev, [d.key]: !prev[d.key] }))}
                      >
                        <div
                          className="w-3 h-3 rounded flex items-center justify-center flex-shrink-0"
                          style={{
                            backgroundColor: reviewDims[d.key] ? 'var(--color-accent)' : 'transparent',
                            border: `1.5px solid ${reviewDims[d.key] ? 'var(--color-accent)' : 'var(--color-border)'}`,
                          }}
                        >
                          {reviewDims[d.key] && (
                            <Check size={9} strokeWidth={3} color="white" aria-hidden="true" />
                          )}
                        </div>
                        {d.label}
                      </label>
                    ))}
                  </div>
                </div>
              </>
            )}
          </div>

          {/* 修稿时显示自定义提示词输入框 */}
          {confirmAction === 'refine' && (
            <div className="px-5 pb-2">
              <label className="text-xs font-medium block mb-1.5" style={{ color: 'var(--color-text-secondary)' }}>
                {text('附加修稿要求（可选）：', 'Additional revision guidance (optional):')}
              </label>
              <textarea
                className="w-full px-3 py-2 rounded-md text-sm"
                style={{
                  background: 'var(--color-bg-elevated)',
                  border: '1px solid var(--color-border)',
                  color: 'var(--color-text)',
                  minHeight: 72,
                  resize: 'vertical',
                  outline: 'none',
                }}
                placeholder={text(
                  '例如：加强打斗场面的画面感；把结尾的伏笔改为更隐晦的暗示；对白太书面化，改为口语化风格...',
                  'For example: make action scenes more vivid; make the final foreshadowing subtler; make dialogue less formal...',
                )}
                value={userRefinePrompt}
                onChange={e => setUserRefinePrompt(e.target.value)}
              />
            </div>
          )}

          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirmAction(null)}>{text('取消', 'Cancel')}</Button>
            <Button
              variant="ai"
              onClick={() => {
                const act = confirmAction
                setConfirmAction(null)
                if (act === 'refine') doRefine()
                else if (act === 'review') doReview()
              }}
            >
              {text('确认执行', 'Run')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* 弹出式三栏合并视图 —— 使用统一 Dialog 组件 */}
      <Dialog open={mergeData !== null} onOpenChange={(v) => !v && draft.closeMerge()}>
        <DialogContent
          className="p-0"
          style={{
            width: '90vw',
            maxWidth: '90vw',
            height: '85vh',
            maxHeight: '85vh',
            overflow: 'hidden',
          }}
          /* 阻止点击遮罩关闭，防止误触丢失合并进度 */
          onPointerDownOutside={(e) => e.preventDefault()}
          onEscapeKeyDown={(e) => e.preventDefault()}
        >
          <DialogHeader className="px-4 py-0" style={{ height: 38, display: 'flex', alignItems: 'center' }}>
            <DialogTitle className="flex items-center gap-2 text-[0.8rem]">
              {text(
                `修稿合并 — 第${meta?.chapterNumber ?? ''}章 ${meta?.chapterTitle || '未知标题'}`,
                `Revision merge — Chapter ${meta?.chapterNumber ?? ''} ${meta?.chapterTitle || 'Untitled'}`,
              )}
            </DialogTitle>
          </DialogHeader>
          {/* 合并视图主体 */}
          <div className="flex-1 overflow-hidden" style={{ height: 'calc(85vh - 38px - 1px)' }}>
            {mergeData && (
              <div className="flex h-full flex-col">
                {mergeData.staleReason && (
                  <div role="alert" className="border-b border-[var(--color-border)] px-4 py-2 text-sm text-[var(--color-warning-text)]">
                    {mergeData.staleReason}
                  </div>
                )}
                <div className="min-h-0 flex-1">
                  <ThreeWayMerge
                    originalContent={mergeData.originalContent}
                    modifiedContent={mergeData.modifiedContent}
                    onComplete={handleMergeComplete}
                    onCancel={draft.closeMerge}
                  />
                </div>
              </div>
            )}
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}
