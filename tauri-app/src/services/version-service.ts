/**
 * version-service — 版本历史数据访问服务
 *
 * 封装 VersionHistory 中的所有 IPC 调用，
 * 避免组件直接与 IPC 通信。
 */

/** 章节元数据（从数据库返回） */
export interface ChapterRecord {
  chapter_id: string
  file_path: string
  file_name: string
  updated_at: string
  chapter_number: number
  status: string
}

/** 版本记录 */
export interface VersionRecord {
  id: number
  version: number
  type: string
  word_count: number
  created_at: string
  dependencies_stale: boolean
}
