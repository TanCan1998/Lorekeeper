import { useState } from 'react'
import { Camera, ClipboardList, Network, Save, Trash2, Users, X } from 'lucide-react'
import { confirm } from '../ui/Confirm'
import { useCharacterStore, type CharacterCard } from '../../stores/character-store'
import { authorStateEdit, CHARACTER_STATE_FIELDS, stateFieldView } from './character-state-fields'
import RelationshipGraph from './RelationshipGraph'
import { EmptyState as BaseEmptyState } from '../ui/EmptyState'
import { Button } from '../ui/Button'
import { Input } from '../ui/Input'
import { Textarea } from '../ui/Textarea'
import { Label } from '../ui/Label'
import { NativeSelect } from '../ui/NativeSelect'
import { useLocaleStore } from '../../stores/locale-store'
import { CHARACTER_ROLES, getCharacterRoleLabels } from '../../shared/character-role'
import { relationshipStorageFromEditor } from '../../shared/relationship-presentation'
import { isProjectSessionCurrent } from '../project-session-gate'
import { SaveFeedback } from './save-feedback'
import { useCharacterAvatar } from './use-character-avatar'
import { useCharacterProfile } from './use-character-profile'
import { pluralEn } from '../../shared/plural-en'

/**
 * 角色卡编辑器 — 纯编辑区域（角色列表已移至侧栏）
 * 从 character-store 读取选中角色，仅渲染编辑表单。
 */
export default function CharacterEditor({ projectKey }: { projectKey: string }) {
  const profile = useCharacterProfile(projectKey)
  const {
    currentProject, projectMatches, dataReady, characters, selectedId, selectedCard, relationshipEditorText, lastError,
    saving, dirty, identityBusy, saveOutcome, saveError, handleSave, updateCurrentField, renameCurrentCharacter,
  } = profile
  const currentDerivedFields = useCharacterStore(s => s.currentDerivedFields)
  const [viewMode, setViewMode] = useState<'edit' | 'state' | 'graph'>('edit')
  const text = useLocaleStore(s => s.text)
  const roleLabel = (role: CharacterCard['role']) => {
    const { zhCN, enUS } = getCharacterRoleLabels(role)
    return text(zhCN, enUS)
  }
  const avatar = useCharacterAvatar(selectedCard?.characterId ?? null, dataReady && viewMode === 'edit')

  const handleDelete = async () => {
    const projectSession = profile.beginDelete('selected')
    if (!projectSession || !selectedCard) return
    const ok = await confirm(
      text(`确定要删除角色「${selectedCard.name || '未命名'}」吗？此操作不可撤销。`, `Delete character “${selectedCard.name || 'Untitled'}”? This cannot be undone.`),
      { title: text('删除角色', 'Delete character'), confirmText: text('删除', 'Delete'), danger: true }
    )
    if (!ok || !isProjectSessionCurrent(projectSession)) return
    await profile.deleteSelected(projectSession)
  }

  const handleDeleteAllCharacters = async () => {
    const projectSession = profile.beginDelete('all')
    if (!projectSession) return
    const ok = await confirm(
      text(
        `确定删除全部 ${characters.length} 个角色及其关系吗？角色名单全文与关系图谱随之清空。此操作不可撤销。`,
        `${characters.length === 1 ? 'Delete the only character and its relationships?' : `Delete all ${characters.length} ${pluralEn(characters.length, 'character')} and their relationships?`} The graph is a projection of the roster and cannot be cleared independently. This cannot be undone.`,
      ),
      {
        title: text('删除全部角色与关系', 'Delete all characters and relationships'),
        confirmText: text('确认删除全部', 'Delete all'),
        danger: true,
      },
    )
    if (!ok || !isProjectSessionCurrent(projectSession)) return
    await profile.deleteAll(projectSession)
  }

  // ===== 渲染 =====

  if (!projectMatches) {
    return (
      <BaseEmptyState
        icon={<Users size={36} />}
        message={text('此标签属于另一个项目，请切回原项目后继续。', 'This tab belongs to another project. Switch back to continue.')}
        opacity={0.4}
      />
    )
  }

  return (
    <div className="h-full flex flex-col overflow-hidden bg-[var(--color-bg)]">
      {/* 统一顶部工具栏 */}
      <div
        className="flex items-center justify-between gap-2 px-3 h-9 flex-shrink-0"
        style={{
          borderBottom: '1px solid var(--color-border)',
          backgroundColor: 'var(--color-editor-bg)',
        }}
      >
        <div className="flex items-center gap-1.5 min-w-0">
          <span className="text-xs font-medium truncate text-[var(--color-text-secondary)]">
            {viewMode === 'graph'
              ? text('关系图谱', 'Relationship graph')
              : selectedCard
                ? `${selectedCard.name || text('新角色', 'New character')} ${viewMode === 'state' ? text('— 当前状态', '— Current state') : text('— 编辑档案', '— Edit profile')}`
                : text('角色档案', 'Character profile')}
          </span>
        </div>
        
        <div className="flex items-center gap-1.5 flex-shrink-0">
          {viewMode === 'graph' ? (
            <>
              <Button
                variant="destructive"
                size="sm"
                onClick={handleDeleteAllCharacters}
                disabled={identityBusy || !dataReady || characters.length === 0}
                title={text('清空图谱会删除作为事实源的全部角色', 'Clearing the graph deletes every character in the source roster')}
              >
                <Trash2 size={12} /> {text('删除全部角色与关系', 'Delete all characters and relationships')}
              </Button>
              <Button variant="outline" size="sm" onClick={() => setViewMode('edit')} title={text('返回编辑', 'Return to editing')}>
                <Users size={12} /> {text('编辑模式', 'Edit mode')}
              </Button>
            </>
          ) : selectedCard ? (
            <>
              {viewMode === 'state' ? (
                <Button variant="outline" size="sm" onClick={() => setViewMode('edit')} title={text('返回基础设定', 'Return to core profile')}>
                  <Users size={12} /> {text('基础设定', 'Core profile')}
                </Button>
              ) : (
                <Button variant="outline" size="sm" onClick={() => setViewMode('state')} title={text('查看当前进展/状态', 'View current state')}>
                  <ClipboardList size={13} /> {text('当前状态', 'Current state')}
                </Button>
              )}
              <Button variant="outline" size="sm" onClick={() => setViewMode('graph')} title={text('查看全员关系网', 'View all character relationships')}>
                <Network size={12} /> {text('关系图谱', 'Relationship graph')}
              </Button>
              <Button variant="destructive" size="sm" onClick={handleDelete} disabled={identityBusy || !dataReady}>
                <Trash2 size={12} /> {text('删除', 'Delete')}
              </Button>
              <SaveFeedback dirty={dirty} saving={saving} outcome={saveOutcome} />
              <Button variant="outline" size="sm" onClick={() => { void handleSave() }} disabled={identityBusy || !dataReady}>
                <Save size={12} /> {saving ? text('保存中...', 'Saving...') : text('保存', 'Save')}
              </Button>
            </>
          ) : (
            <Button variant="outline" size="sm" onClick={() => setViewMode('graph')} title={text('查看全员关系网', 'View all character relationships')}>
              <Network size={12} /> {text('关系图谱', 'Relationship graph')}
            </Button>
          )}
        </div>
      </div>

      {/* 主体区 */}
      <div className="flex-1 overflow-y-auto relative">
        {viewMode === 'graph' ? (
          <RelationshipGraph
            characters={characters}
            selectedCharacterId={selectedId}
            onOpenCharacter={(characterId) => {
              useCharacterStore.getState().setSelectedId(characterId)
              setViewMode('edit')
            }}
          />
        ) : !selectedCard ? (
          <BaseEmptyState 
            icon={<Users size={36} />} 
            message={lastError
              ? text(`角色卡读取失败：${lastError}`, `Could not load character cards: ${lastError}`)
              : (currentProject ? text('在左侧选择或创建角色卡', 'Select or create a character card on the left') : text('请先打开项目', 'Open a project first'))}
            opacity={currentProject ? 0.3 : 0.4}
          />
        ) : viewMode === 'state' ? (
          <div className="max-w-2xl mx-auto px-6 py-4">
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-base font-bold text-[var(--color-text)]">
                {text('当前状态档案', 'Current state profile')}
              </h3>
              <span className="text-xs text-[var(--color-text-secondary)]">
                {text(
                  `最后更新：第 ${selectedCard.currentState?.updatedAtChapter ?? 0} 章`,
                  `Last updated: Chapter ${selectedCard.currentState?.updatedAtChapter ?? 0}`,
                )}
              </span>
            </div>
            <div className="space-y-3">
              {CHARACTER_STATE_FIELDS.map(([field, zh, en]) => {
                const label = text(zh, en)
                const view = stateFieldView(selectedCard, field, currentDerivedFields)
                return <div key={field}>
                  <Label>
                    {label}
                    <span className="ml-2 text-[0.65rem] font-normal text-[var(--color-text-secondary)]">
                      {view.source === 'historical'
                        ? text('历史派生（非当前）', 'Historical derived value (not current)')
                        : view.source === 'author'
                        ? text('作者输入', 'Author input')
                        : view.source === 'derived'
                          ? text('定稿派生', 'Derived from finalized prose')
                          : text('来源未知', 'Unknown source')}
                    </span>
                  </Label>
                  <Textarea
                    value={view.value}
                    onChange={(e) => updateCurrentField(selectedId!, 'currentState', authorStateEdit(selectedCard, field, e.target.value))}
                    rows={2}
                    placeholder={`${label}...`}
                  />
                  {view.history && (
                    <p className="mt-1 text-xs text-[var(--color-text-secondary)]">
                      {text('历史记录（来源已失效或未核验）：', 'History (source invalid or unverified): ')}{view.history}
                    </p>
                  )}
                </div>
              })}
            </div>
            {!selectedCard.currentState && (
              <div className="mt-4 p-3 rounded-lg bg-[var(--color-hover)] text-xs text-[var(--color-text-secondary)]">
                {text('当前状态档案将在章节定稿后由 AI 自动更新，也可手动填写初始状态。', 'AI updates this profile after a chapter is finalized. You can also enter an initial state manually.')}
              </div>
            )}
          </div>
        ) : (
          <div className="max-w-2xl mx-auto px-6 py-4">
            <div className="space-y-3">
              <div className="flex items-center gap-3 rounded-lg border border-[var(--color-border)] p-3">
                {avatar.avatarUrl ? (
                  <img src={avatar.avatarUrl} alt={text(`${selectedCard.name}头像预览`, `${selectedCard.name} avatar preview`)} className="h-16 w-16 rounded-full object-cover" />
                ) : (
                  <div className="flex h-16 w-16 items-center justify-center rounded-full bg-[var(--color-hover)] text-[var(--color-text-muted)]"><Users size={24} /></div>
                )}
                <div className="min-w-0 flex-1">
                  <Label>{text('角色头像', 'Character avatar')}</Label>
                  <div className="mt-1 flex flex-wrap gap-1.5">
                    <Button variant="outline" size="sm" disabled={avatar.busy || !selectedCard.characterId || selectedCard.characterId.startsWith('draft:')} onClick={() => { void avatar.chooseAvatar() }}><Camera size={12} />{avatar.avatarUrl ? text('替换头像', 'Replace avatar') : text('选择头像', 'Choose avatar')}</Button>
                    {avatar.avatarUrl && <Button variant="outline" size="sm" disabled={avatar.busy} onClick={avatar.stageRemoval}><Trash2 size={12} />{text('移除头像', 'Remove avatar')}</Button>}
                    {avatar.staged && <Button variant="ghost" size="sm" disabled={avatar.busy} onClick={avatar.discardStaged}><X size={12} />{text('取消头像更改', 'Discard avatar change')}</Button>}
                  </div>
                  {(!selectedCard.characterId || selectedCard.characterId.startsWith('draft:')) && <p className="mt-1 text-[0.7rem] text-[var(--color-text-secondary)]">{text('先保存角色，再选择头像。', 'Save the character before choosing an avatar.')}</p>}
                  {avatar.staged && <p className="mt-1 text-[0.7rem] text-[var(--color-text-secondary)]">{text('头像更改将在保存角色档案时提交。', 'The avatar change is committed when you save the character profile.')}</p>}
                  {avatar.notice && <p role="alert" className="mt-1 text-[0.7rem] text-[var(--color-danger)]">{avatar.notice}</p>}
                  {saveError && <p role="alert" className="mt-1 text-[0.7rem] text-[var(--color-danger)]">{saveError}</p>}
                </div>
              </div>
              <div className="grid grid-cols-3 gap-3">
                <div><Label>{text('姓名', 'Name')}</Label><Input value={selectedCard.name} disabled={identityBusy} onChange={(e) => renameCurrentCharacter(selectedId!, e.target.value)} /></div>
                <div><Label>{text('性别', 'Gender')}</Label><Input value={selectedCard.gender} onChange={(e) => updateCurrentField(selectedId!, 'gender', e.target.value)} /></div>
                <div><Label>{text('年龄', 'Age')}</Label><Input value={selectedCard.age} onChange={(e) => updateCurrentField(selectedId!, 'age', e.target.value)} /></div>
              </div>
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <Label>{text('定位', 'Role')}</Label>
                  <NativeSelect value={selectedCard.role} onChange={(e) => updateCurrentField(selectedId!, 'role', e.target.value as typeof selectedCard.role)}>
                    {CHARACTER_ROLES.map(role => (
                      <option key={role} value={role}>{roleLabel(role)}</option>
                    ))}
                  </NativeSelect>
                </div>
              </div>
              <div><Label>{text('外貌描写', 'Appearance')}</Label><Textarea value={selectedCard.appearance} onChange={(e) => updateCurrentField(selectedId!, 'appearance', e.target.value)} rows={3} placeholder={text('输入外貌描写...', 'Describe appearance...')} /></div>
              <div><Label>{text('性格特征', 'Personality')}</Label><Textarea value={selectedCard.personality} onChange={(e) => updateCurrentField(selectedId!, 'personality', e.target.value)} rows={3} placeholder={text('输入性格特征...', 'Describe personality...')} /></div>
              <div><Label>{text('背景故事', 'Background')}</Label><Textarea value={selectedCard.background} onChange={(e) => updateCurrentField(selectedId!, 'background', e.target.value)} rows={4} placeholder={text('输入背景故事...', 'Describe background...')} /></div>
              <div><Label>{text('能力/技能', 'Abilities / skills')}</Label><Textarea value={selectedCard.abilities} onChange={(e) => updateCurrentField(selectedId!, 'abilities', e.target.value)} rows={3} placeholder={text('输入能力/技能...', 'Describe abilities or skills...')} /></div>
              <div><Label>{text('核心动机', 'Core motivation')}</Label><Textarea value={selectedCard.motivation} onChange={(e) => updateCurrentField(selectedId!, 'motivation', e.target.value)} rows={2} placeholder={text('输入核心动机...', 'Describe core motivation...')} /></div>
              <div>
                <Label>{text('关系网', 'Relationships')}</Label>
                <Textarea
                  value={relationshipEditorText}
                  onChange={(e) => updateCurrentField(
                    selectedId!,
                    'relationships',
                    relationshipStorageFromEditor(e.target.value, {
                      identities: characters,
                      selfCharacterId: selectedCard.characterId,
                      selfName: selectedCard.name,
                      previousStorage: selectedCard.relationships,
                    }),
                  )}
                  rows={3}
                  placeholder={text(
                    '每行一位角色，例如：陆云飞：竞争对手（权力斗争）',
                    'One character per line, for example: Lu Yunfei: rival (power struggle)',
                  )}
                />
              </div>
              <div><Label>{text('成长轨迹', 'Character arc')}</Label><Textarea value={selectedCard.arc} onChange={(e) => updateCurrentField(selectedId!, 'arc', e.target.value)} rows={3} placeholder={text('输入成长轨迹...', 'Describe the character arc...')} /></div>
              <div><Label>{text('备注', 'Notes')}</Label><Textarea value={selectedCard.notes} onChange={(e) => updateCurrentField(selectedId!, 'notes', e.target.value)} rows={2} placeholder={text('输入备注...', 'Enter notes...')} /><p className="text-xs text-muted-foreground">{text('本章须在正文中明示的内容，另起一行写【第N章必现】具体要求', 'For a must-show requirement, use its own line: 【第N章必现】specific requirement')}</p></div>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
