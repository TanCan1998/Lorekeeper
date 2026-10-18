// 外壳级选择器。替换外壳时只改本目录。
const SHELL_READY = '.app-skin-root [data-shell-presentation="writer"][data-shell-variant="v3"], [data-shell-presentation="stage"]'
const STARTUP_NOTICE = '[role="status"].fixed.inset-x-0.top-10'

/** 旧外壳（ShellV2）或新外壳（流程脊）任一渲染出来即就绪。 */
export async function waitForShell(page, timeout) {
  await page.locator(SHELL_READY).first().waitFor({ state: 'visible', timeout })
}

export async function dismissStartupNotice(page) {
  const acknowledge = page.locator(STARTUP_NOTICE).getByRole('button', { name: '知道了', exact: true })
  if (await acknowledge.isVisible()) await acknowledge.click()
}

/** 工作流完成提示没有角色或类名，只能从「打开查看」找到同卡片的「忽略」。审稿报告里也有「忽略」按钮，不能按名字直接点。 */
const completionToastIgnore = page =>
  page.getByRole('button', { name: '打开查看', exact: true }).locator('xpath=following-sibling::button[1]')

export async function waitForCompletionToast(page) {
  await completionToastIgnore(page).first().waitFor({ state: 'visible', timeout: 5_000 }).catch(() => {})
}

/** 提示出现与自动消失的时机不固定，留着会让截图不可重复。 */
export async function dismissCompletionToasts(page) {
  for (const button of await completionToastIgnore(page).all()) await button.click().catch(() => {})
  await completionToastIgnore(page).last().waitFor({ state: 'detached' })
}

/** 首个可点击控件：书房的「新建作品」。startup-timing 以它出现为可交互时刻。 */
export function firstInteractiveControl(page) {
  return page.getByRole('button', { name: '新建作品', exact: true })
}

const LEFT_RAIL = '.writer-left-rail'
const PANELS = { sidebar: '#writer-sidebar', assistant: '#writer-assistant', bottom: '#writer-bottom' }

export async function clickRail(page, label) {
  await page.locator(LEFT_RAIL).getByRole('button', { name: label }).click()
}

/** 旧外壳三块可收起面板当前是否显示。 */
export function panels(page) {
  return page.evaluate(ids => Object.fromEntries(Object.entries(ids)
    .map(([name, selector]) => [name, !document.querySelector(selector)?.hasAttribute('hidden')])), PANELS)
}

/** 底部面板标题头显示的当前分页名。 */
export function bottomTabLabel(page, label) {
  return page.locator(PANELS.bottom).getByText(label, { exact: true })
}

/** 首页的最近作品书架；只在首页渲染。 */
export function shelf(page) {
  return page.locator('.writer-shelf')
}

export async function isImmersive(page) {
  return await page.locator('[data-writer-immersive="true"]').count() > 0
}

export async function toggleImmersion(page) {
  await page.getByRole('button', { name: /^(进入|退出)沉浸写作$/ }).click()
}

export async function closeBottomPanel(page) {
  await page.locator(PANELS.bottom).getByTitle('关闭面板', { exact: true }).click()
  await page.locator(PANELS.bottom).waitFor({ state: 'hidden' })
}

/** 标题栏当前作品名按钮，点开系统选文件夹对话框。 */
export async function switchProject(page) {
  await page.locator('button[title="切换项目"]').click()
}

export async function openBackupSettings(page) {
  await page.getByRole('button', { name: '备份', exact: true }).click()
}

export async function clickStatusBarModel(page) {
  await page.locator('[title^="当前模型："]').click()
}

/** 收起右侧 AI 输出：再点一次右侧工具栏的「AI 输出」。trunk 与 head 都是这个行为。 */
export async function hideRunPanel(page) {
  await page.getByRole('button', { name: 'AI 输出', exact: true }).click()
}

/** 像点窗口关闭按钮或按 Alt F4 一样从主进程关窗，渲染端的关窗守卫会收到请求。 */
export async function requestWindowClose(app) {
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]?.close())
}
