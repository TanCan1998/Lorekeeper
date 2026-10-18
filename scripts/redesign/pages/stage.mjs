// 新外壳（流程脊）。选择器用角色与可访问名称；只有正文编辑区沿用 CodeMirror 的 .cm-content。
import { decodePng } from '../png.mjs'

const SHELL = '[data-shell-presentation="stage"]'

export const root = page => page.locator(SHELL)
export const spine = page => page.getByRole('navigation', { name: '创作流程' })
export const chapterList = page => spine(page).getByRole('list', { name: '章节' })
export const tabList = page => page.getByRole('tablist', { name: '已打开的页面' })
/** 角色名单页左栏的名单（RD-6 起）。 */
export const characterList = page => root(page).getByRole('complementary', { name: '角色名单' })
export const prose = page => page.locator(`${SHELL} .stage-content .cm-content`)

const escape = value => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

/** 阶段行的可访问名称以阶段名开头，后面是状态说明。 */
export const stageButton = (page, name) => spine(page).getByRole('button', { name: new RegExp(`^${escape(name)}`) })

export async function stageStates(page) {
  return spine(page).locator('[data-stage]').evaluateAll(rows => Object.fromEntries(rows.map(row => [row.dataset.stage, row.dataset.state])))
}

export const chapterButton = (page, number) => chapterList(page).getByRole('button', { name: new RegExp(`^${number}\\D`) })

export async function chapterRows(page) {
  return chapterList(page).getByRole('button').evaluateAll(rows => rows.map(row => ({
    number: Number(row.querySelector('.stage-chapter-no')?.textContent),
    phase: row.dataset.phase,
    current: row.getAttribute('aria-current') === 'page',
  })))
}

/** 书房的书架与右侧概览。点封面只预览，概览里的主按钮才打开。 */
export const shelf = page => page.getByRole('list', { name: '最近的作品' })
export const cover = (page, name) => shelf(page).getByRole('button', { name: `预览《${name}》` })
export const overview = (page, name) => page.getByRole('region', { name: `《${name}》概览` })

export async function previewBook(page, name) {
  await cover(page, name).click()
  await overview(page, name).waitFor({ state: 'visible' })
}

/** 英文界面的书房：封面是「Preview “书名”」，概览里的主按钮是「Open “书名”」。 */
export async function openFromLibraryEn(page, name) {
  await page.getByRole('list', { name: 'Recent books' }).getByRole('button', { name: `Preview “${name}”` }).click()
  await page.getByRole('region', { name: `“${name}” overview` }).getByRole('button', { name: `Open “${name}”` }).click()
}

/** 书架上的书还没打开：先点封面预览，再点概览里的「打开《书名》」。 */
export async function openFromLibrary(page, name) {
  await previewBook(page, name)
  await overview(page, name).getByRole('button', { name: `打开《${name}》` }).click()
  await spine(page).waitFor({ state: 'visible' })
}

/** 从 Ctrl K 打开书架上的书；同一部作品已打开时也会重新打开（新的项目会话）。 */
export async function openFromPalette(page, name) {
  await page.keyboard.press('Control+K')
  await paletteInput(page).waitFor({ state: 'visible' })
  await paletteInput(page).fill(name)
  await palette(page).getByRole('option', { name: new RegExp(`打开《${escape(name)}》`) }).click()
  await spine(page).waitFor({ state: 'visible' })
}

export async function bookMenu(page, name, item) {
  await shelf(page).getByRole('button', { name: `《${name}》更多操作` }).click()
  await page.getByRole('menuitem', { name: item }).click()
}

export async function waitForProse(page, text) {
  await prose(page).getByText(text).first().waitFor({ state: 'visible', timeout: 20_000 })
}

export const tab = (page, name) => tabList(page).getByRole('tab', { name })
export const closeTabButton = (page, name) => tabList(page).getByRole('button', { name: new RegExp(`^关闭「${escape(name)}」`) })

export const saveStatus = page => root(page).getByRole('banner').getByRole('status')

export const focusToggle = page => page.getByRole('button', { name: /^(进入|退出)沉浸写作$/ })

export const palette = page => page.getByRole('dialog', { name: /^(搜索与跳转|查找章节)$/ })
export const paletteInput = page => palette(page).getByRole('combobox')

export async function findChapter(page, query) {
  await page.keyboard.press('Control+P')
  await paletteInput(page).waitFor({ state: 'visible' })
  await paletteInput(page).fill(query)
  await page.keyboard.press('Enter')
  await palette(page).waitFor({ state: 'hidden' })
}

export const capsule = page => root(page).getByRole('button', { expanded: false }).filter({ hasText: /运行中|%|等你确认/ }).first()

/** 设置在新外壳里是内容面板里的页面：左侧「设置分组」导航，当前一页的标题是一级标题。 */
export const settingsNav = page => root(page).getByRole('navigation', { name: '设置分组' })
export const settingsLink = (page, name) => settingsNav(page).getByRole('button', { name, exact: true })

export async function openSettings(page) {
  await spine(page).getByRole('button', { name: '设置', exact: true }).click()
  await settingsNav(page).waitFor({ state: 'visible' })
}

export async function showSettingsPage(page, name) {
  await settingsLink(page, name).click()
  await root(page).getByRole('heading', { level: 1, name, exact: true }).waitFor({ state: 'visible' })
}

export function currentSettingsPage(page) {
  return settingsNav(page).locator('[aria-current="page"]').innerText()
}

/** 设置分组导航：每组的名字与其中各页。 */
export function settingsGroups(page) {
  return settingsNav(page).getByRole('group').evaluateAll(groups => groups.map(group => ({
    group: group.getAttribute('aria-label'),
    pages: [...group.querySelectorAll('button')].map(button => button.textContent.trim()),
  })))
}

export async function closeSettings(page) {
  await settingsNav(page).getByRole('button', { name: /^返回(作品|书房)$/ }).click()
  await settingsNav(page).waitFor({ state: 'detached' })
}

/**
 * 关窗时的退出确认。旧外壳的 Radix 对话框开着时会把它之外的节点设为 aria-hidden，后开的退出确认也在其中，
 * 按角色找不到，所以按类名找。
 */
export const exitDialog = page => page.locator('.stage-dialog').filter({ hasText: '退出前处理未保存的内容' })

export async function openWorkMenuItem(page, name, item) {
  await root(page).getByRole('banner').getByRole('button', { name: `《${name}》作品菜单` }).click()
  await page.getByRole('menuitem', { name: item }).click()
}

export async function backToLibrary(page, name) {
  await root(page).getByRole('banner').getByRole('button', { name: `《${name}》作品菜单` }).click()
  await page.getByRole('menuitem', { name: '回到书房' }).click()
  await shelf(page).waitFor({ state: 'visible' })
}

/**
 * 元素中心点上画在最上面的是否是它：被别的层盖住时为 false。
 * Radix 模态打开时 body 是 pointer-events: none，elementFromPoint 会跳过这些元素，所以临时让所有元素都参与命中测试。
 */
export function isTopmost(locator) {
  return locator.evaluate(element => {
    const probe = document.createElement('style')
    probe.textContent = '* { pointer-events: auto !important; }'
    document.head.append(probe)
    try {
      const box = element.getBoundingClientRect()
      return element.contains(document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2))
    } finally { probe.remove() }
  })
}

/**
 * 下层对话框露在上层对话框外面的一点（上沿、下沿或两侧）：画在最上面的元素的 class，以及截图里那一点与上层对话框纸面的亮度。
 * 上层的遮罩压住下层时，那一点比上层纸面暗；没压住（重影）时两处是同一张纸的颜色。下层整个被上层挡住时为 null。
 */
export async function exposedEdge(page, lower, upper) {
  const probe = await lower.evaluate((element, upperElement) => {
    const l = element.getBoundingClientRect(), u = upperElement.getBoundingClientRect()
    const point = [[l.left + l.width / 2, l.top + 3], [l.left + l.width / 2, l.bottom - 3], [l.left + 3, l.top + l.height / 2], [l.right - 3, l.top + l.height / 2]]
      .find(([x, y]) => x < u.left || x > u.right || y < u.top || y > u.bottom)
    if (!point) return null
    const style = document.createElement('style')
    style.textContent = '* { pointer-events: auto !important; }'
    document.head.append(style)
    try {
      return { point, paper: [u.left + u.width / 2, u.top + 3], hit: document.elementFromPoint(point[0], point[1])?.getAttribute('class') ?? '', scale: devicePixelRatio }
    } finally { style.remove() }
  }, await upper.elementHandle())
  if (!probe) return null
  const image = decodePng(await page.screenshot({ animations: 'disabled' }))
  const luma = ([x, y]) => {
    const offset = (Math.round(y * probe.scale) * image.width + Math.round(x * probe.scale)) * 4
    const [r, g, b] = image.pixels.subarray(offset, offset + 3)
    return Math.round(0.2126 * r + 0.7152 * g + 0.0722 * b)
  }
  return { hit: probe.hit, edgeLuma: luma(probe.point), paperLuma: luma(probe.paper) }
}

/** 落点停在哪：正文章号、其他页面的位置面包屑，或空状态。 */
export async function landing(page) {
  const crumbs = await page.getByRole('navigation', { name: '位置' }).locator('span').allInnerTexts().catch(() => [])
  const empty = await root(page).getByRole('heading', { name: '从流程脊选一步开始' }).count()
  return { crumbs, empty: empty > 0 }
}

/** 正文编辑区里 CodeMirror 的搜索面板。 */
export const proseSearch = page => root(page).locator('.stage-content .cm-search')

/** 新作品首屏的版式检查：进度环半径、设定进行中时说明在名字下一行、作品资料图标跟随行文字色。 */
export function spineLayout(page) {
  return spine(page).evaluate(nav => {
    const setup = nav.querySelector('[data-stage="setup"]')
    const name = setup.querySelector('.stage-row-name').getBoundingClientRect()
    const meta = setup.querySelector('.stage-row-meta').getBoundingClientRect()
    return {
      ringRadius: setup.querySelector('.stage-ring circle')?.getAttribute('r') ?? null,
      setupMetaBelowName: meta.top >= name.bottom - 1,
      resourceIconsFollowRow: [...nav.querySelectorAll('[data-resource]')]
        .every(row => getComputedStyle(row.querySelector('svg')).color === getComputedStyle(row).color),
    }
  })
}

/** 外壳里当前可见的主按钮文字；每层只应有一个。 */
export function primaryButtons(page) {
  return root(page).locator('.pi-btn--primary').evaluateAll(buttons => buttons
    .filter(button => button.getClientRects().length > 0)
    .map(button => button.textContent.trim()))
}

export const modelMissingNotice = page => root(page).getByRole('status').filter({ hasText: '还没有连接 AI 模型' })

/** 概览卡里四个阶段的进度：done、active 或 todo。 */
export function overviewBars(page, name) {
  return overview(page, name).getByRole('list', { name: '阶段进度' }).locator('[data-stage]')
    .evaluateAll(rows => Object.fromEntries(rows.map(row => [row.dataset.stage, row.dataset.state])))
}

export const updateCard = page => root(page).getByRole('status', { name: '应用更新' })

/** 设定页。核心大纲的名字在缺这一项时带「 · 必填」。 */
export const setupOutline = page => root(page).getByRole('textbox', { name: /^核心大纲/ })
export const setupNext = page => root(page).getByRole('button', { name: /^下一步：故事架构/ })
/** 设定页底栏：「下一步」旁的提示与保存失败说明都在这里。 */
export const setupFoot = page => root(page).locator('.stage-setup-foot')
export const setupHint = page => setupFoot(page).getByRole('status')
/** 标红的必填项名称。 */
export const missingSetupLabels = page => root(page).locator('.stage-setup-label[data-missing]')
/** 设置页的分组导航。模态确认打开时其余节点是 aria-hidden，按角色找不到它，用这个看它还在不在。 */
export const settingsNavElement = page => root(page).locator('.stage-set-nav')
/** 设置里沿用旧面板的那几页（模型编辑、提示词模板、写作 Skills、项目备份）。 */
export const legacySettings = page => root(page).locator('.stage-set-legacy')
export const setupFill = page => root(page).getByRole('button', { name: /^(AI 补全其余设定|AI 正在补全…)$/ })
export const setupChip = (page, name) => root(page).getByRole('group', { name: '更多设定' }).getByRole('button', { name: new RegExp(`^${escape(name)}`) })
export const setupPanel = (page, name) => root(page).getByRole('region', { name })
/** 三项必填的下拉；名字在缺这一项时带「 · 必填」。 */
export const setupField = (page, name) => root(page).getByRole('combobox', { name: new RegExp(`^${escape(name)}`) })

/** 在核心大纲末尾补一句，让设定标签变成未保存。 */
export async function dirtySetup(page, addition = '还有一盏灯没有熄灭。') {
  const outline = setupOutline(page)
  await outline.fill(`${await outline.inputValue()}${addition}`)
  await saveStatus(page).filter({ hasText: '未保存' }).waitFor({ state: 'visible' })
}

/** 右下角的提示栈；栈里还没在退场的卡片。 */
export const toastStack = page => page.locator('.pi-toast-stack')
export const LIVE_TOAST = '.pi-toast:not([data-exiting])'
/** 纸墨按钮的变体：上墨的主按钮与次按钮。 */
export const PRIMARY_BUTTON = '.pi-btn--primary'
export const SECONDARY_BUTTON_CLASS = 'pi-btn--secondary'
/** 标签条里的一个内容标签（焦点环画在它的 ::after 上）。 */
export const CONTENT_TAB = '.stage-tab'
/** 顶栏作品名：RD-X3 起是按钮里单独的一段，之前是按钮本身。 */
export const WORK_NAME = ['.stage-work-name', '.stage-work-title']
export const WINDOW_CLOSE = '.stage-window-close'
