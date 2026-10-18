import { afterEach, describe, expect, it } from 'vitest'

import { useLocaleStore } from '../../../stores/locale-store'
import type { ProjectSessionContext } from '../../../shared/ipc-channels'
import {
  createChapterWorkflow,
  createFinalizeWorkflow,
  createRefineFromReviewWorkflow,
  createRefineOnlyWorkflow,
  createReviewOnlyWorkflow,
} from '../chapter-workflow'

const projectPath = 'C:\\novels\\english-chapter'
const projectSession: ProjectSessionContext = {
  projectId: 'english-chapter',
  leaseId: 'english-chapter-lease',
  projectPath,
}
const originalLocale = useLocaleStore.getState().locale
const sourceDraft = {
  id: 2,
  chapterNumber: 2,
  version: 1,
  status: 'draft' as const,
  contentRevision: 0,
}

afterEach(() => useLocaleStore.setState({ locale: originalLocale }))

describe('chapter workflow locale', () => {
  it('freezes English titles, steps, descriptions, and completion copy for chapter workflows', () => {
    useLocaleStore.setState({ locale: 'en-US' })

    const workflows = [
      createChapterWorkflow({
        projectPath,
        chapterNumber: 2,
        title: 'A New Term',
        role: 'Development',
        purpose: 'Deepen the relationship',
        characters: ['Maya'],
        keyEvents: 'Maya joins the literature club.',
      }, projectSession),
      createReviewOnlyWorkflow({
        projectPath,
        chapterNumber: 2,
        chapterTitle: 'A New Term',
        draftPath: 'ai-novel://draft/2',
        draftContent: 'Draft',
        sourceDraft,
      }, projectSession),
      createRefineOnlyWorkflow({
        projectPath,
        chapterNumber: 2,
        chapterTitle: 'A New Term',
        draftPath: 'ai-novel://draft/2',
        draftContent: 'Draft',
        sourceDraft,
      }, projectSession),
      createRefineFromReviewWorkflow({
        projectPath,
        chapterNumber: 2,
        chapterTitle: 'A New Term',
        draftPath: 'ai-novel://draft/2',
        draftContent: 'Draft',
      }, projectSession),
      createFinalizeWorkflow({
        projectPath,
        chapterNumber: 2,
        chapterTitle: 'A New Term',
        draftPath: 'ai-novel://draft/2',
        draftContent: 'Draft',
      }, projectSession),
    ]

    expect(workflows.map(workflow => ({
      locale: workflow.uiLocale,
      title: workflow.title,
      step: workflow.steps[0].name,
      description: workflow.steps[0].description,
      completion: workflow.onComplete?.message,
    }))).toEqual([
      expect.objectContaining({ locale: 'en-US', title: 'Chapter 2 · Draft', step: 'Draft chapter' }),
      expect.objectContaining({ locale: 'en-US', title: 'Chapter 2 · Review', step: 'Review chapter' }),
      expect.objectContaining({ locale: 'en-US', title: 'Chapter 2 · Revise', step: 'Revise draft' }),
      expect.objectContaining({ locale: 'en-US', title: 'Chapter 2 · Apply review', step: 'Revise from review' }),
      expect.objectContaining({ locale: 'en-US', title: 'Chapter 2 · Finalize', step: 'Finalize chapter' }),
    ])
    expect(workflows.flatMap(workflow => [
      workflow.title,
      workflow.steps[0].name,
      workflow.steps[0].description,
      workflow.onComplete?.message ?? '',
    ]).join('\n')).not.toMatch(/[\u4e00-\u9fff]/u)
  })
})
