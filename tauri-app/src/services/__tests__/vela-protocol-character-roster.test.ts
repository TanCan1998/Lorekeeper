import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { ProjectSessionContext } from '../../shared/ipc-channels'
import { ipc } from '../ipc-client'
import { readCoreContent, writeCoreContent } from '../resource-protocol'
import { normalizeSourceEol } from '../../../test/source-contract'

vi.mock('../ipc-client', () => ({
  ipc: {
    invokeWithProjectSession: vi.fn(),
  },
}))

const projectSession: ProjectSessionContext = {
  projectId: 'roster-contract-project',
  leaseId: 'roster-contract-lease',
  projectPath: 'C:/projects/roster-contract',
}

function source(file: string): string {
  return normalizeSourceEol(readFileSync(resolve(process.cwd(), file), 'utf8'))
}

describe('vela core character roster seam', () => {
  beforeEach(() => {
    vi.mocked(ipc.invokeWithProjectSession).mockReset()
  })

  it('reads the ready deterministic roster projection instead of project-core charactersArch', async () => {
    vi.mocked(ipc.invokeWithProjectSession).mockResolvedValueOnce({
      status: 'ready',
      renderedMarkdown: '# 角色图谱\n\n## 主角：陆舟',
      legacyMarkdown: '不应读取的旧投影',
    } as never)

    await expect(readCoreContent('ai-novel://core/characters', projectSession))
      .resolves.toBe('# 角色图谱\n\n## 主角：陆舟')
    expect(ipc.invokeWithProjectSession).toHaveBeenCalledExactlyOnceWith(
      projectSession,
      'db:character-roster-read',
      projectSession.projectPath,
    )
  })

  it('only exposes preserved legacy evidence while the roster is not ready', async () => {
    vi.mocked(ipc.invokeWithProjectSession).mockResolvedValueOnce({
      status: 'legacy_repair_required',
      renderedMarkdown: '',
      legacyMarkdown: '旧角色图谱原文，供作者复制或显式修复。',
    } as never)

    await expect(readCoreContent('ai-novel://core/characters', projectSession))
      .resolves.toBe('旧角色图谱原文，供作者复制或显式修复。')
    expect(vi.mocked(ipc.invokeWithProjectSession).mock.calls.map(([, channel]) => channel))
      .toEqual(['db:character-roster-read'])
  })

  it('rejects direct character-projection writes without reaching project-core IPC', async () => {
    await expect(writeCoreContent('ai-novel://core/characters', '不能直接覆盖', projectSession))
      .resolves.toBe(false)
    expect(ipc.invokeWithProjectSession).not.toHaveBeenCalled()
  })
})

describe('structured character roster static contract', () => {
  it('keeps retired mutation IPC, generic projection writes, and normal extraction paths closed', () => {
    const protocol = source('src/shared/ipc-channels.ts')
    const controller = source('src-tauri/src/commands/db.rs')
    const projectCore = source('src-tauri/src/repositories/project_core_repository.rs').split('#[cfg(test)]')[0]
    const projectClear = source('src-tauri/src/repositories/project_clear_repository.rs').split('#[cfg(test)]')[0]
    const roster = source('src-tauri/src/repositories/character_roster_repository.rs').split('#[cfg(test)]')[0]
    const workflow = source('src/services/workflows/architecture-workflow.ts')
    const projectTree = source('src/components/panels/sidebar/ProjectTree.tsx')
    const worldBuilding = source('src/components/editor/WorldBuildingEditor.tsx')
    const archFileViewer = source('src/components/editor/ArchFileViewer.tsx')

    for (const channel of [
      'db:character-upsert',
      'db:character-save-all',
      'db:character-delete',
      'db:character-update-state',
    ]) {
      expect(protocol).not.toContain(channel)
      const command = channel.replace(/[:-]/gu, '_')
      expect(controller).not.toContain('pub fn ' + command + '(')
      expect(source('src-tauri/src/lib.rs')).not.toContain('commands::' + command + ',')
    }
    expect(protocol).toContain("'db:character-roster-commit'")
    expect(controller).toContain('pub fn db_character_roster_commit(')
    expect(source('src-tauri/src/lib.rs')).toContain('commands::db_character_roster_commit,')
    expect(projectCore).toContain('const READONLY_FIELDS: [&str; 1] = ["charactersArch"]')
    expect(projectCore).toContain('for field in READONLY_FIELDS {\n        if data.contains_key(field) {\n            return Err(CHARACTERS_ARCH_READONLY_MESSAGE.to_string());')
    expect(projectCore).not.toContain('("charactersArch", "characters_arch")')
    // 清空创作字段经作者退休路径删除角色卡，不直接写 roster 表或 characters_arch 投影。
    expect(projectClear).not.toContain('characters_arch')
    expect(projectClear).not.toContain('DELETE FROM character')
    expect(projectClear).toContain('roster::commit_author_character_roster(')
    // 身份投影和升级前的受保护写入路径仍共处同一 Rust 仓储。
    expect(roster.match(/SET characters_arch\s*=/g)).toHaveLength(2)
    expect(roster).toContain('if characters::has_character_identity_schema(conn) {\n        return Err("CHARACTER_ID_WRITE_REQUIRED".to_string());')
    expect(workflow).not.toContain('runArchCharacterExtract')
    expect(workflow).not.toContain('createCharacterExtractSteps')
    expect(workflow).not.toContain('runPostProcessPipeline')
    expect(projectTree).toContain("const isCharacterProjection = f.key === 'characters'")
    expect(projectTree).toContain('...(isCharacterProjection ? [] : [')
    expect(worldBuilding).toContain('useCharacterRosterRepair')
    expect(archFileViewer).toContain('useCharacterRosterRepair')
    expect(worldBuilding).toContain("characters: rosterSnapshot?.status === 'ready'")
  })
})
