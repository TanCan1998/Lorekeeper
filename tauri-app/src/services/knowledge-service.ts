/**
 * knowledge-service — 知识库数据访问服务
 *
 * 封装 KnowledgeOverview 和 KnowledgePanel 中的 IPC 调用，
 * 避免组件直接与 IPC 通信。
 */

import { ipc } from './ipc-client'
import type {
  AppErrorCode,
  AppFailure,
  ProjectSessionContext,
} from '../shared/ipc-channels'

export interface PlanningMaterial {
  fileName: string
  text: string
}

export class KnowledgeBaseServiceError extends Error {
  constructor(public readonly code: AppErrorCode, message?: string) {
    super(message ?? code)
    this.name = 'KnowledgeBaseServiceError'
  }
}

export function unwrapKnowledgeValue<T>(result: T | AppFailure): T {
  if (
    typeof result === 'object' &&
    result !== null &&
    'success' in result &&
    result.success === false &&
    'errorCode' in result
  ) {
    throw new KnowledgeBaseServiceError(result.errorCode, result.error)
  }
  return result as T
}

/** 已导入文档 */
export interface KBDocument {
  id: string
  fileName: string
  importedAt: string
  chunkCount: number
  filePath: string
  /** 由定稿收入时的章号（来自定稿登记）；作者导入的资料没有。 */
  finalizedChapter?: number
}

/** 检索结果 */
export interface SearchResult {
  text: string
  score: number
  fileName: string
}

/** 知识库统计 */
export interface KBStatsData {
  documentCount: number
  totalChunks: number
  vectorDimension: number
}

/**
 * A provider-free capability/status read for the explicit vector rebuild
 * action.  `embeddingConfigured` must be true before the UI offers a button.
 */
export interface VectorRebuildStatus {
  embeddingConfigured: boolean
  canRebuild: boolean
  totalChunks: number
  vectorlessCount: number
  activeVectorDimension: number
}

/** 读取用户通过系统文件选择器明确授权的创作资料。 */
export async function selectPlanningMaterials(): Promise<PlanningMaterial[]> {
  const grants = await ipc.invoke('dialog:select-knowledge-files')
  if (!grants) return []
  const materials: PlanningMaterial[] = []
  for (const grant of grants) {
    const result = await ipc.invoke('fs:grant-read-file', grant.grantId)
    if (!result.success) throw new Error(result.error || `Could not read ${grant.displayName}`)
    materials.push({ fileName: grant.displayName, text: result.content })
  }
  return materials
}

/** 将普通创作资料写入当前冻结项目的 project-knowledge 语料。 */
export async function importPlanningMaterial(
  projectSession: ProjectSessionContext,
  material: PlanningMaterial,
) {
  return ipc.invokeWithProjectSession(
    projectSession,
    'kb:import-planning-text',
    material.text,
    material.fileName,
    projectSession.projectPath,
  )
}
