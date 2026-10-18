import { LegacyRosterRecoveryPanel } from './LegacyRosterRecoveryPanel'
import { Sparkles, CheckCircle2, Circle, RefreshCw, FileText, BookOpen, AlertTriangle, FolderTree, Eye, Copy } from 'lucide-react'
import { useLocaleStore } from '../../stores/locale-store'
import { renderIcon } from '../panels/sidebar/sidebar-icons'

import ArchitectureConfirmDialog from '../dialogs/ArchitectureConfirmDialog'

import { Button } from '../ui/Button'
import { EmptyState } from '../ui/EmptyState'
import { ARCH_FILES, useArchitectureOverview } from './use-architecture-overview'
import { ARCH_STEP_NAMES } from '../../services/architecture-step-selection'

/** 故事架构编辑器 — 显示四个架构文件状态，并提供 AI 生成入口 */
export default function WorldBuildingEditor({ projectKey }: { projectKey: string }) {
  const text = useLocaleStore(s => s.text)
  const overview = useArchitectureOverview(projectKey)
  const {
    projectMatches, loading, archStatus, wordCounts, planningRecovery,
    loadStatus, openArchFile, handleResumeSynopsis, handleResumeWorldBuilding,
    copyCandidate, handleContinueOutlineBatch, openGenerateDialog, openRecoveryDraft,
  } = overview
  const {
    progress: synopsisProgress, recoveryFailed: synopsisRecoveryFailed, busy: synopsisBusy, candidate: synopsisCandidate,
    candidateShown: showSynopsisCandidate, setCandidateShown: setShowSynopsisCandidate, handle: synopsisHandle,
  } = overview.synopsis
  const synopsisIncomplete = synopsisProgress.kind === 'interrupted' && Boolean(synopsisHandle)
  const synopsisCoveredTo = synopsisProgress.kind === 'partial' ? synopsisProgress.coveredTo : 0
  const synopsisTotalChapters = synopsisProgress.kind === 'partial' ? synopsisProgress.total : 0
  const {
    busy: worldBuildingBusy, candidate: worldBuildingCandidate, candidateShown: showWorldBuildingCandidate,
    setCandidateShown: setShowWorldBuildingCandidate, handle: worldHandle,
  } = overview.worldBuilding
  const { presentation: rosterPresentation, canRepair: canRepairRoster, repairing: extracting, repair: handleRepairCharacterRoster } = overview.roster

  if (!projectMatches) {
    return (
      <div className="h-full flex flex-col overflow-hidden bg-[var(--color-bg)]">
        <div
          className="flex items-center justify-between gap-2 px-3 h-9 flex-shrink-0"
          style={{
            borderBottom: '1px solid var(--color-border)',
            backgroundColor: 'var(--color-editor-bg)',
          }}
        >
          <div className="flex items-center gap-1.5 min-w-0">
            <span className="text-xs font-medium truncate text-[var(--color-text-secondary)]">
              {text('故事架构', 'Story architecture')}
            </span>
          </div>
        </div>
        <div className="flex-1 overflow-y-auto relative">
          <EmptyState icon={<BookOpen size={36} />} message={text('请先打开项目', 'Open a project to continue')} opacity={0.4} />
        </div>
      </div>
    )
  }

  const generatedCount = ARCH_FILES.filter(f => (
    archStatus[f.key] && !(f.key === 'synopsis' && synopsisRecoveryFailed)
  )).length

  return (
    <div className="h-full flex flex-col overflow-hidden">
      {/* 顶部工具栏 */}
      <div
        className="flex items-center justify-between gap-2 px-3 h-10 flex-shrink-0 border-b"
        style={{ borderColor: 'var(--color-border)', backgroundColor: 'var(--color-sidebar)' }}
      >
        <div className="flex items-center gap-1.5">
          <FolderTree size={14} style={{ color: 'var(--color-text-muted)' }} />
          <span className="text-sm font-medium" style={{ color: 'var(--color-text)' }}>
            {text('故事架构', 'Story architecture')}
          </span>
          <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
            {generatedCount}/{ARCH_FILES.length} {text('已生成', 'generated')}
          </span>
        </div>
        <div className="flex items-center gap-1.5">
          <Button
            variant="ghost"
            size="icon"
            onClick={loadStatus}
            title={text('刷新状态', 'Refresh status')}
          >
            <RefreshCw size={13} className={loading ? 'animate-spin' : ''} />
          </Button>
          {/* AI 生成架构 — 与小说配置/章节蓝图保持一致的按钮位置 */}
          <Button
            variant="ai"
            size="sm"
            onClick={() => openGenerateDialog()}
            title={text('AI 生成故事架构（选择要生成的步骤）', 'Generate story architecture (choose steps to generate)')}
          >
            <Sparkles size={12} />
            {text('AI 生成架构', 'Generate story architecture')}
          </Button>
        </div>
      </div>

      {/* 文件卡片列表 */}
      <div className="flex-1 overflow-y-auto p-4 space-y-3">
        {ARCH_FILES.map(f => {
          const generated = archStatus[f.key]
          const synopsisNeedsRecovery = f.key === 'synopsis' && synopsisRecoveryFailed
          const words = wordCounts[f.key] ?? 0
          const isCharacters = f.key === 'characters'
          const isWorldBuildingCandidate = f.key === 'worldbuilding' && Boolean(worldBuildingCandidate)
          const isSynopsisCandidate = f.key === 'synopsis' && Boolean(synopsisCandidate)
          const hasCandidate = isWorldBuildingCandidate || isSynopsisCandidate
          const candidateText = isSynopsisCandidate ? synopsisCandidate : worldBuildingCandidate
          const candidateShown = isSynopsisCandidate ? showSynopsisCandidate : showWorldBuildingCandidate
          const candidateBusy = isSynopsisCandidate ? synopsisBusy : worldBuildingBusy
          const candidateHandle = isSynopsisCandidate ? synopsisHandle : worldHandle
          const rosterNeedsAttention = isCharacters && rosterPresentation
            && rosterPresentation.kind !== 'ready'
            && rosterPresentation.kind !== 'empty'
          // 动态边框颜色：明确失败/异常 → 红 | 显式修复/采用 → 警告 | 已生成 → 绿
          const cardBorderColor = rosterPresentation?.kind === 'failed_with_data_preserved'
            || rosterPresentation?.kind === 'inconsistent'
            ? 'var(--color-error, #ef4444)'
            : rosterNeedsAttention
              ? 'var(--color-warning)'
              : synopsisNeedsRecovery
                ? 'var(--color-warning)'
                : hasCandidate
                  ? 'var(--color-warning)'
               : generated
                    ? 'var(--color-success)'
                    : 'var(--color-border)'
          return (
            <div key={f.key} className="space-y-2">
              <div
                className="rounded-lg border p-4 flex items-center gap-4 cursor-pointer transition-all"
                style={{
                  borderColor: cardBorderColor,
                  backgroundColor: rosterPresentation?.kind === 'failed_with_data_preserved'
                    || rosterPresentation?.kind === 'inconsistent'
                    ? 'rgba(239, 68, 68, 0.03)'
                    : 'var(--color-panel)',
                  opacity: loading ? 0.6 : 1,
                }}
                onClick={() => openArchFile(f)}
                onMouseEnter={e => (e.currentTarget as HTMLElement).style.borderColor = 'var(--color-accent)'}
                onMouseLeave={e => (e.currentTarget as HTMLElement).style.borderColor = cardBorderColor}
                title={`${text('点击查看', 'Open')} — ${text(f.descZh, f.descEn)}`}
              >
                {/* 状态图标 */}
                {hasCandidate || synopsisNeedsRecovery
                  ? <AlertTriangle size={18} style={{ flexShrink: 0, color: 'var(--color-warning)' }} />
                  : generated
                    ? <CheckCircle2 size={18} style={{ flexShrink: 0, color: 'var(--color-success)' }} />
                    : <Circle size={18} style={{ flexShrink: 0, color: 'var(--color-text-muted)' }} />
                }

                {/* 图标 */}
                <span className="flex-shrink-0" style={{ color: generated ? 'var(--color-text-secondary)' : 'var(--color-text-muted)' }}>{renderIcon(f.iconName, 24)}</span>

                {/* 标题 + 描述 */}
                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium" style={{ color: 'var(--color-text)' }}>
                    {text(...ARCH_STEP_NAMES[f.key])}
                  </div>
                  <div className="text-xs mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
                    {text(f.descZh, f.descEn)}
                  </div>
                  {isCharacters && rosterPresentation && (
                    <div
                      role="status"
                      className="text-xs mt-1 leading-5"
                      style={{
                        color: rosterNeedsAttention
                          ? 'var(--color-warning-text)'
                          : 'var(--color-text-muted)',
                      }}
                    >
                      {rosterPresentation.label} · {rosterPresentation.description}
                    </div>
                  )}
                </div>

                {/* 右侧状态标签 / 字数 / 提取按钮 */}
                <div className="flex flex-col items-end gap-1 flex-shrink-0">
                  {isCharacters && rosterPresentation ? (
                    <>
                      <span
                        className="text-[0.7rem] px-1.5 py-0.5 rounded font-medium"
                        style={{
                          backgroundColor: rosterNeedsAttention
                            ? 'rgba(245, 158, 11, 0.12)'
                            : 'rgba(34, 197, 94, 0.1)',
                          color: rosterNeedsAttention
                            ? 'var(--color-warning-text)'
                            : 'var(--color-success-text)',
                        }}
                      >
                        {rosterPresentation.label}
                      </span>
                      {generated && (
                        <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
                          {words.toLocaleString()} {text('字符', 'characters')}
                        </span>
                      )}
                    </>
                  ) : hasCandidate ? (
                    <>
                      <span className="text-[0.7rem] px-1.5 py-0.5 rounded font-medium bg-yellow-500/15 text-[var(--color-warning-text)]">
                        {text(
                          generated ? '正式内容保留 · 有未完成候选' : '未完成候选 · 未写入正式内容',
                          generated ? 'Formal content kept · incomplete candidate' : 'Incomplete candidate · not formal content',
                        )}
                      </span>
                      {generated && (
                        <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
                          {words.toLocaleString()} {text('字符（正式内容）', 'characters (formal)')}
                        </span>
                      )}
                      <div className="flex flex-wrap justify-end gap-1 mt-0.5">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={(e) => {
                            e.stopPropagation()
                            if (isSynopsisCandidate) setShowSynopsisCandidate(value => !value)
                            else setShowWorldBuildingCandidate(value => !value)
                          }}
                        >
                          <Eye size={12} />
                          {candidateShown ? text('收起候选', 'Hide candidate') : text('查看候选', 'View candidate')}
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={(e) => {
                            e.stopPropagation()
                            void copyCandidate(candidateText)
                          }}
                        >
                          <Copy size={12} />
                          {text('复制', 'Copy')}
                        </Button>
                        <Button
                          size="sm"
                          disabled={candidateBusy || !candidateHandle || loading}
                          className="gap-1.5 bg-gradient-to-r from-amber-500 to-orange-500 text-white border-none"
                          onClick={(e) => {
                            e.stopPropagation()
                            if (isSynopsisCandidate && planningRecovery) openRecoveryDraft()
                            else void (isSynopsisCandidate ? handleResumeSynopsis() : handleResumeWorldBuilding())
                          }}
                        >
                          <RefreshCw size={12} className={candidateBusy ? 'animate-spin' : ''} />
                          {isSynopsisCandidate && planningRecovery ? text('编辑恢复稿', 'Edit recovery draft') : candidateBusy ? text('续写中...', 'Resuming...') : text('断点续写', 'Resume')}
                        </Button>
                      </div>
                      {!candidateHandle && <span role="status" className="text-xs text-[var(--color-warning-text)]">
                        {text('此候选无法确认恢复来源，可查看或复制后重新生成。', 'Recovery source unavailable. View or copy this candidate, then generate again.')}
                      </span>}
                    </>
                  ) : generated ? (
                    <>
                      {f.key === 'synopsis' && synopsisRecoveryFailed ? (
                        <span className="text-[0.7rem] px-1.5 py-0.5 rounded font-medium bg-yellow-500/15 text-[var(--color-warning-text)]">
                          {text('不完整 · 检查点不可恢复', 'Incomplete · checkpoint unavailable')}
                        </span>
                      ) : f.key === 'synopsis' && synopsisIncomplete ? (
                        <span className="text-[0.7rem] px-1.5 py-0.5 rounded font-medium bg-yellow-500/15 text-[var(--color-warning-text)]">
                          {text('不完整 · 已存部分', 'Incomplete · partial saved')}
                        </span>
                      ) : f.key === 'synopsis' && synopsisCoveredTo > 0 && synopsisCoveredTo < synopsisTotalChapters ? (
                        <span className="text-[0.7rem] px-1.5 py-0.5 rounded font-medium bg-yellow-500/15 text-[var(--color-warning-text)]">
                          {text(`已覆盖至第 ${synopsisCoveredTo} 章 · 待续批`, `Covered to ch. ${synopsisCoveredTo} · pending`)}
                        </span>
                      ) : (
                        <span className="text-[0.7rem] px-1.5 py-0.5 rounded font-medium bg-green-500/10 text-[var(--color-success-text)]">
                          {text('已生成', 'Generated')}
                        </span>
                      )}
                      <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
                        {words.toLocaleString()} {text('字符', 'characters')}
                      </span>
                      {f.key === 'synopsis' && synopsisIncomplete && !loading && (
                        <Button
                          size="sm"
                          disabled={synopsisBusy}
                          className="gap-1.5 mt-0.5 bg-gradient-to-r from-amber-500 to-orange-500 text-white shadow-sm hover:from-amber-600 hover:to-orange-600 border-none hover:shadow hover:-translate-y-[0.5px] transition-all"
                          onClick={(e) => {
                            e.stopPropagation()
                            void handleResumeSynopsis()
                          }}
                          title={text(
                            '上次生成被输出长度中断，已完成部分已保存。点击后 AI 从断点继续生成当前批次。',
                            'The previous run stopped at the output length limit and the completed part was saved. Click to continue the current batch from the break point.',
                          )}
                        >
                          {synopsisBusy
                            ? <RefreshCw size={12} className="animate-spin opacity-90" />
                            : <RefreshCw size={12} className="opacity-90" />
                          }
                          {synopsisBusy
                            ? text('续写中...', 'Resuming...')
                            : text('断点续写大纲', 'Continue outline')}
                        </Button>
                      )}
                      {f.key === 'synopsis' && !synopsisIncomplete && !synopsisRecoveryFailed
                        && synopsisCoveredTo > 0 && synopsisCoveredTo < synopsisTotalChapters && !loading && (
                        <Button
                          size="sm"
                          disabled={synopsisBusy}
                          className="gap-1.5 mt-0.5 bg-gradient-to-r from-indigo-500 to-blue-500 text-white shadow-sm hover:from-indigo-600 hover:to-blue-600 border-none hover:shadow hover:-translate-y-[0.5px] transition-all"
                          onClick={(e) => {
                            e.stopPropagation()
                            void handleContinueOutlineBatch()
                          }}
                          title={text(
                            `从第 ${synopsisCoveredTo + 1} 章起继续生成剩余章节（已确认的第 1–${synopsisCoveredTo} 章保持不变）。`,
                            `Continue generating the remaining chapters from chapter ${synopsisCoveredTo + 1} (confirmed chapters 1-${synopsisCoveredTo} stay unchanged).`,
                          )}
                        >
                          {synopsisBusy
                            ? <RefreshCw size={12} className="animate-spin opacity-90" />
                            : <RefreshCw size={12} className="opacity-90" />
                          }
                          {synopsisBusy
                            ? text('生成中...', 'Generating...')
                            : text(`续批（第 ${synopsisCoveredTo + 1} 章起）`, `Continue (ch. ${synopsisCoveredTo + 1}+)`)}
                        </Button>
                      )}
                    </>
                  ) : (
                    <span
                      className="text-[0.7rem] px-1.5 py-0.5 rounded"
                      style={{ backgroundColor: 'rgba(var(--color-accent-rgb,99 102 241),0.1)', color: 'var(--color-accent)' }}
                    >
                      {text('待生成', 'Not generated')}
                    </span>
                  )}
                  {/* 显式安全修复，或采用受保护的既有角色卡。 */}
                  {isCharacters && !loading && canRepairRoster && rosterPresentation?.actionLabel && (
                    <Button
                      size="sm"
                      disabled={extracting}
                      className="gap-1.5 mt-0.5 bg-gradient-to-r from-amber-500 to-orange-500 text-white shadow-sm hover:from-amber-600 hover:to-orange-600 border-none hover:shadow hover:-translate-y-[0.5px] transition-all"
                      onClick={(e) => {
                        e.stopPropagation()
                        void handleRepairCharacterRoster()
                      }}
                      title={rosterPresentation.actionTitle}
                    >
                      {extracting
                        ? <RefreshCw size={12} className="animate-spin opacity-90" />
                        : <AlertTriangle size={12} className="opacity-90" />
                      }
                      {extracting ? text('处理中...', 'Working...') : rosterPresentation.actionLabel}
                    </Button>
                  )}
                  {/* 查看箭头提示 */}
                  {generated && !(isCharacters && !loading && canRepairRoster) && (
                    <span className="text-[0.7rem] flex items-center gap-0.5" style={{ color: 'var(--color-text-muted)' }}>
                      <FileText size={10} /> {text('点击查看', 'Open')}
                    </span>
                  )}
                </div>
              </div>
              {isCharacters && <LegacyRosterRecoveryPanel onRecover={handleRepairCharacterRoster} busy={extracting} />}
              {hasCandidate && candidateShown && (
                <div
                  role="status"
                  className="rounded-lg border p-3"
                  style={{ borderColor: 'var(--color-warning)', backgroundColor: 'var(--color-panel)' }}
                >
                  <div className="text-xs font-medium mb-2" style={{ color: 'var(--color-warning-text)' }}>
                    {text('未完成候选（不会自动写入正式内容）', 'Incomplete candidate (not written to formal content)')}
                  </div>
                  <pre className="text-xs whitespace-pre-wrap max-h-64 overflow-y-auto" style={{ color: 'var(--color-text-secondary)' }}>
                    {candidateText}
                  </pre>
                </div>
              )}
            </div>
          )
        })}
      </div>

      {/* AI 生成架构确认弹窗 */}
      <ArchitectureConfirmDialog {...overview.generateDialog} />
    </div>
  )
}
