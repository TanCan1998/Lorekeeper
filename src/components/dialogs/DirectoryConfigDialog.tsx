import { FileText, RotateCcw } from 'lucide-react'
import {
  Dialog, DialogContent, DialogHeader, DialogFooter, DialogTitle, DialogDescription,
} from '../ui/Dialog'
import { Button } from '../ui/Button'
import { Label } from '../ui/Label'
import { Textarea } from '../ui/Textarea'
import { getBlueprintBatchAdvice } from '../../services/workflows/blueprint-batch-policy'
import { resolveWritingLanguage } from '../../shared/writing-language'
import { PLANNING_ACTION_CHAPTER_LIMIT, PLANNING_TARGET_UNITS_LIMIT } from '../../shared/plot-outline-contract'
import { useDirectoryConfigForm, type DirectoryConfigProps } from './use-directory-config-form'
import { pluralEn } from '../../shared/plural-en'

/** 蓝图生成配置弹框 — 选择生成范围和模式 */
export default function DirectoryConfigDialog(props: DirectoryConfigProps) {
  const form = useDirectoryConfigForm(props)
  if (!form) return null
  const { isOpen, onClose, existingCount } = props
  const {
    text, currentProject, total, appendStart, hasPriorAuthority, previewCost,
    rangeMode, setRangeMode, overwriteMode, setOverwriteMode, frontN, setFrontN, rangeStart, setRangeStart, rangeEnd, setRangeEnd,
    targetUnits, setTargetUnits, preferenceLoading, pacingGuidance, setPacingGuidance,
    isConfirming, launchError, authorityError, authorityLoading,
    pendingCharacterSyncs, isRecoveryLoading, isRecovering, recoveryError,
    progressError, diagnostics, progress, handleCharacterSyncRecovery, handleConfirm, continueOperation,
  } = form
  return (
    <Dialog open={isOpen} onOpenChange={(v) => !v && onClose()}>
      <DialogContent className="max-h-[90vh] max-w-[480px] overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <FileText size={16} className="text-[var(--color-accent)]" />
            {text('生成章节蓝图', 'Generate chapter blueprints')}
          </DialogTitle>
          <DialogDescription>
            {existingCount > 0
              ? text(`当前已存在 ${existingCount} 章蓝图，选择下一步操作：`, `${existingCount} chapter ${pluralEn(existingCount, 'blueprint')} already ${pluralEn(existingCount, 'exists', 'exist')}. Choose what to do next:`)
              : hasPriorAuthority
                ? text(`权威正文已定稿至第 ${appendStart - 1} 章，下一章蓝图从第 ${appendStart} 章开始。`, `Finalized manuscript authority ends at Chapter ${appendStart - 1}; the next blueprint starts at Chapter ${appendStart}.`)
              : text(`项目共 ${total} 章，请选择生成范围：`, `The project has ${total} ${pluralEn(total, 'chapter')}. Choose a generation range:`)}
          </DialogDescription>
        </DialogHeader>

        <div className="px-5 py-4 space-y-4">
          {progressError && <p role="alert" className="text-xs">{progressError}</p>}
          {diagnostics.length > 0 && (
            <div role="status" className="rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)] p-3 text-xs space-y-1">
              {[...new Set(diagnostics.map(item => item.code))].map(code => <p key={code}>
                {code === 'GENERATION_BLUEPRINTS_CLEARED'
                  ? text('蓝图已清空，旧生成记录不能续接', 'Blueprints were cleared. Previous generation runs cannot continue.')
                  : code === 'GENERATION_DIRECTORY_PROGRESS_INVALID'
                    ? text('旧蓝图生成进度校验失败，不能续接', 'Previous blueprint generation progress failed validation and cannot continue.')
                    : text('旧蓝图作者保存记录校验失败，不能续接', 'A previous author save record failed validation and cannot continue.')}
              </p>)}
              <p>{text('旧恢复稿仍可编辑或复制，也可开始新的蓝图生成', 'You can still edit or copy old recovery drafts, or start new blueprint generation.')}</p>
            </div>
          )}
          {progress.map(progress => (
            <div key={progress.operationId} className="rounded-lg border p-3 text-xs">
              <p>{text(
                '已保存第 ' + progress.committedRange.startChapter + '–' + progress.committedRange.endChapter + ' 章蓝图。',
                'Blueprints for chapters ' + progress.committedRange.startChapter + '–' + progress.committedRange.endChapter + ' are saved.',
              )}</p>
              {progress.remainingRange ? (
                <Button variant="outline" className="mt-2" disabled={isConfirming} onClick={() => void continueOperation(progress.operationId)}>
                  {text('继续第 ' + progress.remainingRange.startChapter + '–' + progress.remainingRange.endChapter + ' 章（沿用原预算）',
                    'Continue chapters ' + progress.remainingRange.startChapter + '–' + progress.remainingRange.endChapter + ' (same budget)')}
                </Button>
              ) : <p>{text('该范围已完成。', 'This range is complete.')}</p>}
            </div>
          ))}
          {(isRecoveryLoading || pendingCharacterSyncs.length > 0 || recoveryError) && (
            <div
              className="rounded-lg border px-3 py-2.5 text-xs"
              style={{
                color: 'var(--color-text-secondary)',
                backgroundColor: 'var(--color-panel)',
                borderColor: 'var(--color-border)',
              }}
            >
              <p>
                {isRecoveryLoading
                  ? text('正在检查角色同步待办...', 'Checking pending character syncs...')
                  : pendingCharacterSyncs.length > 0
                    ? text(
                        `发现 ${pendingCharacterSyncs.length} 次角色同步待修复；蓝图已安全保存，无需重新生成。`,
                        `${pendingCharacterSyncs.length} character sync operation(s) need repair. The blueprints are already saved.`,
                      )
                    : recoveryError}
              </p>
              {!isRecoveryLoading && (pendingCharacterSyncs.length > 0 || recoveryError) && (
                <Button
                  variant="outline"
                  className="mt-2 h-7 text-xs"
                  onClick={handleCharacterSyncRecovery}
                  disabled={isRecovering}
                >
                  <RotateCcw size={13} />
                  {isRecovering
                    ? text('修复中...', 'Repairing...')
                    : text('重试角色同步', 'Retry character sync')}
                </Button>
              )}
            </div>
          )}
          <div>
            <Label className="text-xs font-semibold mb-2 block" style={{ color: 'var(--color-text)' }}>
              {text('生成数量 / 范围', 'Quantity / range')}
            </Label>
            <div className="space-y-3 mt-2">
              <RadioOption
                checked={rangeMode === 'front'}
                onChange={() => setRangeMode('front')}
                label={
                  <span className="flex items-center gap-2">
                    {text('批量连续生成', 'Generate next')}
                    <input
                      type="number"
                      min={1} max={PLANNING_ACTION_CHAPTER_LIMIT} step={1}
                      aria-label={text('本次蓝图章数', 'Blueprint chapters in this action')}
                      value={frontN}
                      onChange={e => setFrontN(e.target.value === '' ? '' : Number(e.target.value))}
                      className="w-16 h-6 rounded-md border border-[var(--color-border)] bg-[var(--color-panel)] text-xs px-2 py-0"
                      onClick={e => e.stopPropagation()}
                    />
                    {text('章', 'chapters')}
                  </span>
                }
              />
              <RadioOption
                checked={rangeMode === 'range'}
                onChange={() => setRangeMode('range')}
                label={
                  <span className="flex items-center gap-2">
                    {text('指定生成：第', 'Generate range:')}
                    <input
                      type="number"
                      min={1} max={total} step={1}
                      aria-label={text('蓝图起始章', 'First blueprint chapter')}
                      value={rangeStart}
                      onChange={e => setRangeStart(e.target.value === '' ? '' : Number(e.target.value))}
                      className="w-16 h-6 rounded-md border border-[var(--color-border)] bg-[var(--color-panel)] text-xs px-2 py-0"
                      onClick={e => e.stopPropagation()}
                    />
                    {text('到 第', 'to')}
                    <input
                      type="number"
                      min={1} max={total} step={1}
                      aria-label={text('蓝图结束章', 'Last blueprint chapter')}
                      value={rangeEnd}
                      onChange={e => setRangeEnd(e.target.value === '' ? '' : Number(e.target.value))}
                      className="w-16 h-6 rounded-md border border-[var(--color-border)] bg-[var(--color-panel)] text-xs px-2 py-0"
                      onClick={e => e.stopPropagation()}
                    />
                    {text('章', 'chapter')}
                  </span>
                }
              />
              <RadioOption
                checked={rangeMode === 'full'}
                onChange={() => setRangeMode('full')}
                label={text(`全量生成（共 ${total} 章）`, `Generate all ${total} ${pluralEn(total, 'chapter')}`)}
              />
            </div>
            <p
              className="mt-3 rounded-md border px-3 py-2 text-xs leading-5"
              style={{
                color: 'var(--color-text-secondary)',
                backgroundColor: 'var(--color-panel)',
                borderColor: 'var(--color-border)',
              }}
            >
              {text(
                getBlueprintBatchAdvice('zh-CN', previewCost.chapterCount),
                getBlueprintBatchAdvice('en-US', previewCost.chapterCount),
              )}
              {previewCost.exceedsHardLimit && text(
                ' 大范围可能耗尽本次预算；已验证的连续蓝图会保存，剩余范围保持未完成。',
                ' Large ranges may exhaust this action’s budget. Validated consecutive blueprints are saved; the rest remain incomplete.',
              )}
            </p>
          </div>

          <div>
            <Label htmlFor="blueprint-target-units">
              {resolveWritingLanguage(currentProject.novelConfig.writingLanguage) === 'zh-CN'
                ? text('每章蓝图目标字数', 'Blueprint target characters per chapter')
                : text('每章蓝图目标词数', 'Blueprint target words per chapter')}
            </Label>
            <input id="blueprint-target-units" type="number" min={1} max={PLANNING_TARGET_UNITS_LIMIT} step={1}
              className="mt-1 h-7 w-full rounded-md border border-[var(--color-border)] bg-[var(--color-panel)] px-3 text-xs"
              value={targetUnits} disabled={preferenceLoading} onChange={event => setTargetUnits(event.target.value)} />
            <p className="mt-1 text-xs text-[var(--color-text-muted)]">{text('仅作生成目标，实际内容可多可少。', 'A generation target. Actual content may be longer or shorter.')}</p>
          </div>

          {(existingCount > 0 || hasPriorAuthority) && (
            <div
              className="rounded-lg p-3 space-y-2 mt-4"
              style={{ backgroundColor: 'var(--color-panel)', border: '1px solid var(--color-border)' }}
            >
              <p className="text-xs font-medium" style={{ color: 'var(--color-text-muted)' }}>
                {text('针对已有数据的处理方式：', 'Existing blueprint handling:')}
              </p>
              <div className="space-y-3 mt-2">
                <RadioOption
                  checked={overwriteMode === 'append'}
                  onChange={() => setOverwriteMode('append')}
                  label={text(`追加模式：保留现有蓝图，从第 ${appendStart} 章起往后生成`, `Append: keep existing blueprints and continue from chapter ${appendStart}`)}
                />
                <RadioOption
                  checked={overwriteMode === 'full'}
                  onChange={() => setOverwriteMode('full')}
                  label={text('覆盖模式：无视现有蓝图，从第 1 章起强制覆盖生成', 'Overwrite: regenerate from chapter 1 and replace existing blueprints')}
                />
              </div>
            </div>
          )}

          {/* 节奏/风格指导（可选） */}
          <div>
            <Label className="text-xs font-semibold mb-2 block" style={{ color: 'var(--color-text)' }}>
              {text('节奏/风格指导（可选）', 'Pacing / style guidance (optional)')}
            </Label>
            <Textarea
              value={pacingGuidance}
              onChange={e => setPacingGuidance(e.target.value)}
              placeholder={text('如：“前30章快节奏，每章安排一个爽点。中期适当铺设伏笔和角色成长。”', 'e.g. Keep the first 30 chapters fast-paced, then add foreshadowing and character growth.')}
              rows={2}
              className="text-xs"
            />
          </div>
          {launchError && (
            <p className="whitespace-pre-line rounded-lg border border-yellow-500/30 bg-yellow-500/10 px-3 py-2.5 text-xs text-[var(--color-warning-text)]">
              {launchError}
            </p>
          )}
          {authorityError && (
            <p className="whitespace-pre-line rounded-lg border border-yellow-500/30 bg-yellow-500/10 px-3 py-2.5 text-xs text-[var(--color-warning-text)]">
              {authorityError}
            </p>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={isConfirming || isRecovering}>{text('取消', 'Cancel')}</Button>
          <Button
            variant="default"
            onClick={handleConfirm}
            disabled={
              isConfirming
              || preferenceLoading
              || authorityLoading
              || Boolean(authorityError)
              || isRecovering
            }
          >
            <FileText size={13} />
            {isConfirming ? text('启动中...', 'Starting...') : text('开始生成', 'Generate')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/** 单选按钮选项 */
function RadioOption({
  checked,
  onChange,
  label,
}: {
  checked: boolean
  onChange: () => void
  label: React.ReactNode
}) {
  return (
    <label
      className="flex items-center gap-2 text-xs cursor-pointer select-none"
      style={{ color: 'var(--color-text-secondary)' }}
      onClick={onChange}
    >
      <div
        className="w-3.5 h-3.5 rounded-full border flex items-center justify-center flex-shrink-0"
        style={{
          borderColor: checked ? 'var(--color-accent)' : 'var(--color-border)',
          backgroundColor: checked ? 'var(--color-accent)' : 'transparent',
        }}
      >
        {checked && <div className="w-1.5 h-1.5 rounded-full bg-white" />}
      </div>
      {label}
    </label>
  )
}
