import { useCallback, useEffect, useRef, useState } from 'react'
import { Save } from 'lucide-react'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle,
} from '../ui/Dialog'
import CodeMirrorEditor from '../editor/CodeMirrorEditor'
import { SaveFeedback, type SaveOutcome } from '../editor/save-feedback'
import NovelConfigEditor from '../editor/NovelConfigEditor'
import CharacterEditor from '../editor/CharacterEditor'
import ChapterCardEditor from '../editor/ChapterCardEditor'
import WorldBuildingEditor from '../editor/WorldBuildingEditor'
import ArchFileViewer from '../editor/ArchFileViewer'
import DraftEditor from '../editor/DraftEditor'
import VersionHistory from '../editor/VersionHistory'
import ReviewReport from '../editor/ReviewReport'
import NarrativeThreadEditor from '../editor/NarrativeThreadEditor'
import KnowledgeOverview from '../pages/KnowledgeOverview'
import ThreeWayMerge from '../editor/ThreeWayMerge'
import { registerEditorExitSaveHandler, useEditorStore, type EditorTab } from '../../stores/editor-store'
import { useLocaleStore } from '../../stores/locale-store'
import type { ProjectData } from '../../shared/ipc-channels'
import { ipc } from '../../services/ipc-client'
import { requireIpcSuccess } from '../../services/ipc-result'
import { toast } from '../ui/Toast'
import {
  captureProjectSession,
  isProjectSessionCurrent,
  isProjectSessionPath,
} from '../project-session-gate'
import { savePhysicalChapterForSession } from './editor-area-physical-save'
import '../editor/novel-editor.css'
import { pluralEn } from '../../shared/plural-en'

// ─── 正文章节编辑器包装层（含字数信息栏） ─────────────────────────────────────────────
function ProseEditorWrapper({
  tab,
  onSave,
  unsavedOnly = false,
  readOnly = false,
}: {
  tab: EditorTab
  onSave?: (text: string) => Promise<void>
  unsavedOnly?: boolean
  readOnly?: boolean
}) {
  const [wordCount, setWordCount] = useState(0)
  const [saving, setSaving] = useState(false)
  const [saveOutcome, setSaveOutcome] = useState<SaveOutcome>('idle')
  const fileName = tab.name
  // 追踪当前编辑器内容，供保存按钮使用（不触发重渲染）
  const currentContentRef = useRef(tab.content ?? '')
  const text = useLocaleStore(s => s.text)

  const handleSave = useCallback(async (content: string, propagateFailure = false) => {
    if (!onSave) return
    setSaving(true)
    setSaveOutcome('idle')
    try {
      await onSave(content)
      const settledTab = useEditorStore.getState().tabs.find(candidate => candidate.id === tab.id)
      setSaveOutcome(settledTab?.dirty ? 'idle' : 'saved')
    } catch (error) {
      setSaveOutcome('failed')
      toast.error(error instanceof Error ? error.message : text('保存失败', 'Save failed'))
      if (propagateFailure) throw error
    } finally {
      setSaving(false)
    }
  }, [onSave, tab.id, text])

  useEffect(() => {
    if (!onSave) return
    registerEditorExitSaveHandler({
      tabId: tab.id,
      type: tab.type,
      projectKey: tab.projectKey,
      save: () => handleSave(currentContentRef.current, true),
    })
  }, [handleSave, onSave, tab.id, tab.projectKey, tab.type])

  return (
    <div className="h-full flex flex-col overflow-hidden">
      {/* 顶部信息栏（背景与编辑区一致） */}
      <div
        className="flex items-center justify-between px-3 h-9 flex-shrink-0"
        style={{
          borderBottom: '1px solid var(--color-border)',
          backgroundColor: 'var(--color-editor-bg)',
        }}
      >
        {/* 左侧：文件名 */}
        <div className="min-w-0">
          <span className="block text-xs truncate" style={{ color: 'var(--color-text-muted)' }}>
            {fileName}
          </span>
          {unsavedOnly && (
            <span className="block text-[10px]" style={{ color: 'var(--color-warning)' }}>
              {text('项目恢复候选；不会自动写入正式草稿', 'Project recovery candidate; it is not written to the formal draft automatically')}
            </span>
          )}
        </div>

        {/* 右侧：字数 + dirty 指示灯 + 保存按钮 */}
        <div className="flex items-center gap-1.5 flex-shrink-0">
          {wordCount > 0 && (
            <span className="text-xs tabular-nums" style={{ color: 'var(--color-text-muted)' }}>
              {text(`${wordCount.toLocaleString()} 字`, `${wordCount.toLocaleString()} ${pluralEn(wordCount, 'word')}`)}
            </span>
          )}
          {/* 未保存圆点指示灯 */}
          {tab.dirty && (
            <span
              className="w-1.5 h-1.5 rounded-full"
              style={{ backgroundColor: 'var(--color-warning)' }}
              title={text('有未保存的修改', 'Unsaved changes')}
            />
          )}
          <SaveFeedback dirty={Boolean(tab.dirty)} saving={saving} outcome={saveOutcome} />
          {/* 保存按钮（有改动时显示） */}
          {tab.dirty && onSave && (
            <button
              className="icon-btn"
              style={{ width: 24, height: 22 }}
              onClick={() => { void handleSave(currentContentRef.current) }}
              disabled={saving}
              title={text('保存（⌘S）', 'Save (⌘S)')}
            >
              <Save size={13} strokeWidth={1.5} />
            </button>
          )}
        </div>
      </div>

      {/* 编辑器主体 */}
      <div className="flex-1 overflow-hidden">
        <CodeMirrorEditor
          key={tab.id}
          mode="prose"
          content={tab.content ?? ''}
          paperHead={{ title: fileName }}
          editable={!readOnly}
          filePath={tab.filePath}
          hideStatusBar
          onCharCountChange={setWordCount}
          onChange={(text) => {
            if (readOnly) return
            // 同步 ref，供保存按钮使用
            currentContentRef.current = text
            setSaveOutcome('idle')
            // 标记 tab.dirty
            useEditorStore.getState().updateTabContent(tab.id, text)
          }}
          onSave={onSave ? (text) => handleSave(text) : undefined}
        />
      </div>
    </div>
  )
}

/** 当前标签的内容分发：按标签类型渲染对应编辑器。旧外壳的 EditorArea 与新外壳共用这一份。 */
export default function EditorTabContent({ activeTab, currentProject }: { activeTab: EditorTab | undefined; currentProject: ProjectData }) {
  const text = useLocaleStore(s => s.text)
  return (
    <>
      {activeTab?.type === 'chapter' && activeTab.projectKey === currentProject.path && (
        activeTab.filePath?.startsWith('ai-novel://draft/')
        || activeTab.filePath?.startsWith('ai-novel://manuscript/')
      ) && (
        // 草稿文件：使用 DraftEditor（工具栏含修稿/审稿/定稿按鈕）
        <DraftEditor
          key={activeTab.id}
          tabId={activeTab.id}
          filePath={activeTab.filePath}
          content={activeTab.content ?? ''}
          projectKey={activeTab.projectKey}
        />
      )}
      {activeTab?.type === 'chapter' && activeTab.projectKey === currentProject.path
        && activeTab.filePath?.startsWith('ai-novel://recovery/') && (
        <ProseEditorWrapper
          key={activeTab.id}
          tab={activeTab}
          onSave={async content => {
            const projectSession = captureProjectSession(currentProject)
            const candidateId = activeTab.filePath?.slice('ai-novel://recovery/'.length)
            if (
              !projectSession
              || !candidateId
              || !isProjectSessionPath(projectSession, activeTab.projectKey)
            ) return
            const tab = useEditorStore.getState().tabs.find(candidate => candidate.id === activeTab.id)
            if (!tab) return
            const snapshot = { content, contentRevision: tab.contentRevision ?? 0 }
            const result = await ipc.invokeWithProjectSession(
              projectSession,
              'db:recovery-candidate-update',
              candidateId,
              content,
              projectSession.projectPath,
            )
            requireIpcSuccess(result, '保存恢复候选')
            if (!isProjectSessionCurrent(projectSession)) return
            useEditorStore.getState().settleTabSave(activeTab.id, snapshot)
          }}
          unsavedOnly
        />
      )}
      {activeTab?.type === 'chapter' && activeTab.projectKey === currentProject.path
        && activeTab.filePath?.startsWith('ai-novel://revision/') && (
        <ProseEditorWrapper key={activeTab.id} tab={activeTab} readOnly />
      )}
      {activeTab?.type === 'chapter' && activeTab.projectKey === currentProject.path
        && !activeTab.filePath?.startsWith('ai-novel://draft/')
        && !activeTab.filePath?.startsWith('ai-novel://manuscript/')
        && !activeTab.filePath?.startsWith('ai-novel://revision/')
        && !activeTab.filePath?.startsWith('ai-novel://recovery/') && (
        // 【DB 迁移备注】：终稿目前作为物理文件保存在 manuscript/ 目录是合理的（用于外部阅读器或最终打包编译导出）
        // 终稿文件（manuscript/）：用 ProseEditorWrapper（含字数信息栏）
        <ProseEditorWrapper
          key={activeTab.id}
          tab={activeTab}
          onSave={async (text) => {
            if (!activeTab.filePath) return
            const projectSession = captureProjectSession(currentProject)
            if (
              !projectSession
              || !isProjectSessionPath(projectSession, activeTab.projectKey)
            ) return
            await savePhysicalChapterForSession({
              tabId: activeTab.id,
              filePath: activeTab.filePath,
              content: text,
              projectSession,
            })
          }}
        />
      )}
      {activeTab?.type === 'config' && activeTab.projectKey && (
        <NovelConfigEditor key={activeTab.id} projectKey={activeTab.projectKey} />
      )}
      {activeTab?.type === 'outline' && (
        <div className="h-full overflow-y-auto p-6">
          <pre
            className="text-sm whitespace-pre-wrap font-mono leading-6"
            style={{ color: 'var(--color-text)' }}
          >
            {activeTab.content || text('加载中...', 'Loading...')}
          </pre>
        </div>
      )}
      {activeTab?.type === 'character' && activeTab.projectKey && (
        <CharacterEditor key={activeTab.id} projectKey={activeTab.projectKey} />
      )}
      {activeTab?.type === 'chapter-card' && activeTab.projectKey && (
        <ChapterCardEditor
          key={activeTab.id}
          tabId={activeTab.id}
          projectKey={activeTab.projectKey}
          initialChapterNumber={activeTab.chapterNumber}
          selectRequest={activeTab.chapterSelectRequest}
        />
      )}
      {activeTab?.type === 'world-building' && activeTab.projectKey && (
        <WorldBuildingEditor key={activeTab.id} projectKey={activeTab.projectKey} />
      )}
      {activeTab?.type === 'narrative-thread' && activeTab.projectKey === currentProject.path && (
        <NarrativeThreadEditor
          key={activeTab.id}
          projectKey={activeTab.projectKey}
          initialView={activeTab.narrativeThreadView ?? 'plans'}
          viewRequest={activeTab.narrativeThreadViewRequest}
        />
      )}
      {activeTab?.type === 'arch-file' && activeTab.filePath && activeTab.projectKey && (
        <ArchFileViewer
          key={activeTab.id}
          tabId={activeTab.id}
          filePath={activeTab.filePath}
          projectKey={activeTab.projectKey}
          content={activeTab.content ?? ''}
          savedContent={activeTab.savedContent ?? activeTab.content ?? ''}
        />
      )}
      {activeTab?.type === 'version-history' && activeTab.projectKey === currentProject.path && (
        <VersionHistory projectKey={activeTab.projectKey} />
      )}
      {activeTab?.type === 'knowledge' && activeTab.projectKey === currentProject.path && (
        <KnowledgeOverview />
      )}
      {activeTab?.type === 'review-report' && activeTab.projectKey === currentProject.path && activeTab.content && (
        <ReviewReport
          reportText={activeTab.content}
          draftPath={activeTab.filePath}
          chapterNumber={activeTab.chapterNumber}
          chapterDir={activeTab.chapterDir}
          reviewId={activeTab.reviewId}
          projectKey={activeTab.projectKey}
        />
      )}
      {/* diff 合并视图 — 统一使用弹出式 Dialog（与 DraftEditor 一致） */}
      <Dialog
        open={
          activeTab?.type === 'diff'
          && activeTab.projectKey === currentProject.path
          && !!activeTab.originalContent
          && !!activeTab.content
        }
        onOpenChange={(v) => {
          if (!v && activeTab?.type === 'diff') {
            useEditorStore.getState().closeTab(activeTab.id)
          }
        }}
      >
        <DialogContent
          className="p-0"
          style={{
            width: '90vw',
            maxWidth: '90vw',
            height: '85vh',
            maxHeight: '85vh',
            overflow: 'hidden',
          }}
          onPointerDownOutside={(e) => e.preventDefault()}
          onEscapeKeyDown={(e) => e.preventDefault()}
        >
          <DialogHeader className="px-4 py-0" style={{ height: 38, display: 'flex', alignItems: 'center' }}>
            <DialogTitle className="flex items-center gap-2 text-[0.8rem]">
              {text(`修稿合并 — ${activeTab?.name ?? '对比视图'}`, `Revision merge — ${activeTab?.name ?? 'Comparison'}`)}
            </DialogTitle>
          </DialogHeader>
          <div className="flex-1 overflow-hidden" style={{ height: 'calc(85vh - 38px - 1px)' }}>
            {activeTab?.type === 'diff'
              && activeTab.projectKey === currentProject.path
              && activeTab.originalContent
              && activeTab.content && (
              <ThreeWayMerge
                originalContent={activeTab.originalContent}
                modifiedContent={activeTab.content}
                onComplete={async (mergedText) => {
                  const mergeTab = activeTab
                  const projectSession = captureProjectSession(currentProject)
                  if (!mergeTab || !projectSession || !isProjectSessionPath(projectSession, mergeTab.projectKey)) {
                    toast.error(text('项目会话已失效，未合并修订内容', 'The project session is no longer active; the revision was not merged.'))
                    return
                  }

                  let mergeCommitted = false
                  try {
                    const chapterDir = mergeTab.chapterDir
                    const filePath = mergeTab.filePath
                    const revPath = mergeTab.revisionPath
                    const chapterNum = mergeTab.chapterNumber
                    const expectedDraftContent = mergeTab.originalContent

                    if (chapterDir && filePath && revPath && expectedDraftContent !== undefined) {
                      const targetTab = useEditorStore.getState().tabs.find(tab => (
                        tab.type === 'chapter'
                        && tab.projectKey === mergeTab.projectKey
                        && tab.filePath === filePath
                      ))
                      if (targetTab && (
                        targetTab.dirty
                        || targetTab.content !== expectedDraftContent
                      )) {
                        toast.warning(text(
                          '打开对比后正文已变化，未提交修订；请保存后重新打开',
                          'The draft changed after the comparison opened. The revision was not committed; save and reopen it.',
                        ))
                        return
                      }
                      const { useDraftStore } = await import('../../stores/draft-store')
                      if (!isProjectSessionCurrent(projectSession)) return

                      const result = await useDraftStore.getState().applyMergedRevision(
                        chapterDir,
                        chapterNum,
                        filePath,
                        revPath,
                        mergedText,
                        expectedDraftContent,
                        projectSession.projectPath,
                        projectSession,
                      )
                      if (!isProjectSessionCurrent(projectSession)) return

                      if (result.success) {
                        mergeCommitted = true
                        toast.success(text('合并完成，草稿已更新', 'Merge complete; draft updated'))
                        if (result.postCommitError) toast.warning(text(
                          '合并已持久化，但自动复验未启动；请从审稿报告重试。',
                          'The merge is durable, but the automatic recheck did not start. Retry it from the review report.',
                        ))
                      } else {
                        toast.error(text(`合并失败：${result.error}`, `Merge failed: ${result.error}`))
                      }
                    }
                  } catch (e) {
                    if (!isProjectSessionCurrent(projectSession)) return
                    toast.error(text(`合并出错：${e}`, `Merge error: ${e}`))
                  } finally {
                    if (mergeCommitted && isProjectSessionCurrent(projectSession)) {
                      useEditorStore.getState().closeTab(mergeTab.id)
                    }
                  }
                }}
                onCancel={() => useEditorStore.getState().closeTab(activeTab.id)}
              />
            )}
          </div>
        </DialogContent>
      </Dialog>
    </>
  )
}
