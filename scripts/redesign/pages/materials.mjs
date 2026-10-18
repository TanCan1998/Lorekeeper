// 作品资料导入：知识库创作资料、角色卡与参考小说。都会在确认后启动工作流。
async function confirmSend(page) {
  await page.getByRole('button', { name: '发送并提取', exact: true }).click()
}

export async function importPlanningMaterial(page) {
  await page.getByRole('button', { name: '导入创作资料' }).click()
  await confirmSend(page)
}

export async function importCharacterCards(page, source) {
  await page.getByRole('button', { name: '粘贴 / 导入角色卡' }).first().click()
  await page.getByLabel('角色卡全文').fill(source)
  await page.getByRole('button', { name: 'AI 提取并预览' }).click()
  await confirmSend(page)
}

/** 从首页「小说拆解」把参考小说导入当前作品；文件由调用方预先桩好系统打开对话框。 */
export async function importReferenceNovelIntoCurrentProject(page, chapterCount) {
  await page.getByRole('button', { name: '小说拆解', exact: true }).click()
  const dialog = page.getByRole('dialog', { name: '小说拆解与仿写' })
  await dialog.getByTestId('import-target-current').click()
  await dialog.getByRole('button', { name: '选择', exact: true }).click()
  await dialog.getByRole('button', { name: `导入当前项目（${chapterCount} 章）`, disabled: false }).click()
  await dialog.waitFor({ state: 'hidden' })
}
