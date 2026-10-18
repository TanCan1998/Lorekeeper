/**
 * 三栏合并视图 — 基于相似度 DP 对齐的段落级 diff
 *
 * 核心算法改进：
 * - 使用字符重叠率计算段落相似度
 * - DP 动态规划支持 1:1、1:2、1:3、2:1、3:1 段落对齐
 * - 正确处理段落拆分（1段→2段）和合并（2段→1段）
 *
 * 布局：左栏原稿（只读）| 中栏合并结果（可编辑）| 右栏修稿（只读）
 */
import React, { useRef, useLayoutEffect } from 'react'
import { useMergeSegments } from './merge-segments'
import { ArrowLeft, ArrowRight, Check } from 'lucide-react'
import { Button } from '../ui/Button'
import { useLocaleStore } from '../../stores/locale-store'
import { pluralEn } from '../../shared/plural-en'
import './three-way-merge.css'

interface ThreeWayMergeProps {
  originalContent: string
  modifiedContent: string
  onComplete: (mergedText: string) => void
  onCancel?: () => void
}

// ===== 渲染辅助 =====

function HunkLines({ lines, padCount, cls, emptyLabel }: {
  lines: string[]; padCount: number; cls: string; emptyLabel: string
}) {
  return (
    <>
      {lines.length > 0
        ? lines.map((l, i) => <div key={i} className={cls}>{l || '\u00A0'}</div>)
        : <div className="twm-line-placeholder">{emptyLabel}</div>}
      {Array.from({ length: padCount }).map((_, i) => (
        <div key={`p${i}`} className="twm-line-padding">{'\u00A0'}</div>
      ))}
    </>
  )
}

/** contentEditable 子组件 — 仅在挂载时设置内容 */
function EditableCell({ text, onChange }: { text: string; onChange: (t: string) => void }) {
  const ref = useRef<HTMLDivElement>(null)
  useLayoutEffect(() => {
    if (ref.current) ref.current.textContent = text || '\u00A0'
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  return (
    <div ref={ref} className="twm-editable" contentEditable
      suppressContentEditableWarning
      onInput={e => onChange((e.target as HTMLDivElement).innerText)} />
  )
}

// ===== 主组件 =====

export default function ThreeWayMerge({
  originalContent, modifiedContent, onComplete, onCancel,
}: ThreeWayMergeProps) {
  const text = useLocaleStore(s => s.text)
  const { segments, hunks, picks, segTexts, choose, editSegment, chooseAll, mergedText } = useMergeSegments(originalContent, modifiedContent)
  const applied = (index: number) => picks[index] === 'revised'
  const processedCount = hunks.filter(hunk => applied(hunk.index)).length

  const getPad = (oLen: number, mLen: number) => {
    const lV = oLen > 0 ? oLen : 1, rV = mLen > 0 ? mLen : 1
    return { leftPad: Math.max(0, rV - lV), rightPad: Math.max(0, lV - rV) }
  }

  return (
    <div className="three-way-merge">
      <div className="twm-toolbar">
        <Button variant="ghost" size="sm" onClick={() => chooseAll('original')}><ArrowLeft size={13} />{text('全部原稿', 'Use all original')}</Button>
        <Button variant="ghost" size="sm" onClick={() => chooseAll('revised')}>{text('全部修稿', 'Use all revision')}<ArrowRight size={13} /></Button>
        <span className="twm-toolbar-progress">{text('已采用 {done}/{total} 处变更', `{done}/{total} ${pluralEn(hunks.length, 'change')} applied`, { done: processedCount, total: hunks.length })}</span>
        {onCancel && <Button variant="ghost" size="sm" onClick={onCancel}>{text('取消', 'Cancel')}</Button>}
        <Button variant="success" size="sm" onClick={() => onComplete(mergedText())}>{text('完成合并', 'Finish merge')}</Button>
      </div>

      {/* 固定表头 */}
      <div className="twm-headers">
        <div className="twm-header">{text('原稿', 'Original')} <span className="twm-tag readonly">{text('只读', 'Read only')}</span></div>
        <div className="twm-header">{text('合并结果', 'Merged result')} <span className="twm-tag editable">{text('可编辑', 'Editable')}</span></div>
        <div className="twm-header">{text('修稿', 'Revision')} <span className="twm-tag readonly">{text('只读', 'Read only')}</span></div>
      </div>

      {/* 单滚动容器 + CSS Grid 自动行高对齐 */}
      <div className="twm-scroll">
        <div className="twm-grid">
          {segments.map((seg, idx) => {
            if (seg.type === 'same') {
              // same 行：三栏静态文本，中栏可编辑
              return (
                <React.Fragment key={idx}>
                  <div className="twm-cell twm-cell-left">
                    {seg.lines?.map((l, i) => <div key={i} className="twm-line-same">{l || '\u00A0'}</div>)}
                  </div>
                  <div className="twm-cell twm-cell-center">
                    <EditableCell key={`s${idx}`} text={segTexts[idx] ?? ''}
                      onChange={t => editSegment(idx, t)} />
                  </div>
                  <div className="twm-cell twm-cell-right">
                    {seg.lines?.map((l, i) => <div key={i} className="twm-line-same">{l || '\u00A0'}</div>)}
                  </div>
                </React.Fragment>
              )
            }

            // hunk 行
            const hunk = seg.hunk!
            const isApplied = applied(hunk.index)
            const { leftPad, rightPad } = getPad(hunk.originalLines.length, hunk.modifiedLines.length)

            return (
              <React.Fragment key={idx}>
                {/* 左栏 */}
                <div className={`twm-cell twm-cell-left ${isApplied ? 'processed' : ''}`}>
                  <HunkLines lines={hunk.originalLines} padCount={leftPad} cls="twm-line-removed"
                    emptyLabel={`（新增 ${hunk.modifiedLines.length} 行）`} />
                </div>

                {/* 中栏 */}
                <div className={`twm-cell twm-cell-center ${isApplied ? 'adopted' : 'pending'}`}>
                  <EditableCell key={`h${idx}-${isApplied ? 1 : 0}`} text={segTexts[idx] ?? ''}
                    onChange={t => editSegment(idx, t)} />
                </div>

                {/* 右栏（含采用按钮） */}
                <div className={`twm-cell twm-cell-right ${isApplied ? 'processed' : ''}`}>
                  <div className="twm-hunk-row">
                    <button className={`twm-adopt ${isApplied ? 'adopted' : ''}`}
                      onClick={() => choose(hunk.index, isApplied ? 'original' : 'revised')}
                      title={isApplied ? '恢复原稿' : '采用修稿'}>
                      {isApplied
                        ? <Check size={14} aria-hidden="true" />
                        : <ArrowLeft size={14} aria-hidden="true" />}
                    </button>
                    <div className="twm-hunk-text">
                      <HunkLines lines={hunk.modifiedLines} padCount={rightPad} cls="twm-line-added"
                        emptyLabel={`（删除 ${hunk.originalLines.length} 行）`} />
                    </div>
                  </div>
                </div>
              </React.Fragment>
            )
          })}
        </div>
      </div>
    </div>
  )
}
