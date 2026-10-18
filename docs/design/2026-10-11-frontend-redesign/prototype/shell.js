// 原型外壳：每个屏幕只写自己的内容模板，外壳负责顶栏、流程脊、标签条和侧页布局。
// 主题：URL 带 ?theme=dark 时使用深色。截图时带 ?clean 隐藏原型导航。
const params = new URLSearchParams(location.search);
if (params.get('theme') === 'dark') document.documentElement.dataset.theme = 'dark';

const SAMPLE = {
  work: '雨夜列车',
  chapters: [
    [1, '灰色的信', 'final'], [2, '母亲的旧照片', 'final'], [3, '雨夜车站', 'reviewed'], [4, '第三节车厢', 'running'],
    [5, '不要回头', 'todo'], [6, '阅读灯', 'todo'], [7, '终点站以北', 'todo'], [8, '失物招领处', 'todo'],
    [9, '第二封信', 'todo'], [10, '守夜人', 'todo'], [11, '空座位', 'todo'], [12, '返程票', 'todo'],
  ],
};
const CHAPTER_STATUS = {
  final: ['ok', '定稿'], revised: ['accent', '已修稿'], reviewed: ['accent', '已审稿'], draft: ['warn', '草稿'],
  running: ['run', '生成中'], todo: ['', '待写'],
};
const STAGE_PRESETS = {
  mid: { setup: ['done', '已完成'], arch: ['done', '4 / 4 已确认'], plan: ['done', '24 章'], text: ['now', '2 / 24 定稿'] },
  new: { setup: ['now', '还差 1 项必填'], arch: ['todo', '前提、角色、世界观、大纲'], plan: ['todo', '逐章规划'], text: ['todo', '写作、审稿、修稿、定稿'] },
  arch: { setup: ['done', '已完成'], arch: ['now', '2 / 4 已确认'], plan: ['todo', '架构确认后开始'], text: ['todo', ''] },
};
const STAGES = [['setup', 'i', '设定'], ['arch', 'ii', '架构'], ['plan', 'iii', '蓝图'], ['text', 'iv', '正文']];
const REFS = [['plot', 'git-branch', '剧情与伏笔', '7'], ['knowledge', 'library', '知识库', '12'], ['history', 'history', '版本历史', '']];

const Shell = {
  mount(o = {}) {
    const screen = document.getElementById('screen');
    const sheet = document.getElementById('sheet');
    const overlay = document.getElementById('overlay');
    const preset = STAGE_PRESETS[o.preset || 'mid'];
    const showSpine = o.spine !== false;
    const showChapters = o.chapters ?? (o.stage === 'text');
    const tabs = o.tabs || [];

    const stagesHtml = STAGES.map(([id, no, name]) => {
      const [state, meta] = preset[id];
      const current = o.stage === id;
      return `<li class="stage" data-state="${state === 'now' ? 'now' : state}" ${current ? 'aria-current="page"' : ''}>
        <span class="stage-no">${no}</span><span class="stage-name">${name}</span>${meta ? `<span class="stage-meta">${meta}</span>` : ''}</li>`;
    }).join('');

    const chaptersHtml = showChapters ? `<div class="spine-chapters">
        <div class="spine-search">${icon('search', 'i-sm')}查找章节<span class="kbd" style="margin-left:auto">⌘P</span></div>
        <ol class="chapter-list">${SAMPLE.chapters.map(([n, t, s0]) => {
          const s = (o.chapterStatus && o.chapterStatus[n]) || s0;
          const [tone, label] = CHAPTER_STATUS[s];
          return `<li class="chapter" data-status="${s}" ${o.chapter === n ? 'aria-current="page"' : ''}>
            <span class="chapter-no">${String(n).padStart(2, '0')}</span><span class="chapter-title">${t}</span>
            <span class="status"><span class="dot ${tone}"></span>${label}</span></li>`;
        }).join('')}</ol></div>` : '';

    const refsHtml = REFS.map(([id, ic, name, count]) =>
      `<li class="ref" ${o.ref === id ? 'aria-current="page"' : ''}>${icon(ic)}${name}${count ? `<span class="count">${count}</span>` : ''}</li>`).join('');

    const spine = showSpine ? `<nav class="spine" aria-label="创作流程">
        <div class="spine-label">创作流程</div>
        <ol class="stages">${stagesHtml}</ol>
        ${chaptersHtml}
        <div class="refs-label">作品资料</div>
        <ul class="refs">${refsHtml}</ul>
        <div class="spine-foot"><div class="ref" ${o.ref === 'settings' ? 'aria-current="page"' : ''}>${icon('settings')}设置</div></div>
      </nav>` : '';

    const crumbs = (o.crumbs || []).map((c, i, a) => i === a.length - 1 ? `<b>${c}</b>` : `${c}<span>/</span>`).join(' ');
    const run = o.run ? `<button class="capsule" style="--p:${o.run.pct}" aria-expanded="${!!o.run.open}">
        <span class="ring"></span>${o.run.label}${o.run.pct != null ? ` <b>${o.run.pct}%</b>` : ''}</button>` : '';
    const workTitle = o.work === null
      ? `<span class="work-switch" style="font-family:var(--font-display)">AI 小说作家</span>`
      : `<button class="work-switch">${o.work || SAMPLE.work} ${icon('chevron-down', 'i-sm')}</button>`;

    const tabsHtml = tabs.length > 1 ? `<div class="tabs" role="tablist">${tabs.map(t =>
      `<div class="tab" role="tab" aria-selected="${!!t.active}" data-dirty="${!!t.dirty}"><span class="tab-title">${t.title}</span>
        <button class="tab-x" aria-label="关闭">${icon('x', 'i-sm')}</button></div>`).join('')}</div>` : '<div></div>';

    const app = document.createElement('div');
    app.className = 'app';
    app.innerHTML = `
      <header class="titlebar">
        <div class="brand">墨</div>
        ${workTitle}
        ${crumbs ? `<div class="crumbs">${icon('chevron-right', 'i-sm')}${crumbs}</div>` : ''}
        <div class="tb-gap"></div>
        ${run}
        ${o.saved === undefined ? '' : `<span class="saved" data-dirty="${!o.saved}">${icon(o.saved ? 'check' : 'circle-dashed', 'i-sm')}${o.saved ? '已保存' : '未保存'}</span>`}
        ${showSpine ? `<button class="tb-btn">${icon('maximize', 'i-sm')}沉浸</button>` : ''}
        <div class="winctl"><button aria-label="最小化">${icon('minus')}</button><button aria-label="最大化">${icon('square')}</button><button class="close" aria-label="关闭">${icon('x')}</button></div>
      </header>
      <div class="frame" data-spine="${showSpine ? 'shown' : 'hidden'}">
        ${spine}
        <div class="content">
          ${tabsHtml}
          <div class="workspace" data-sheet="${sheet ? (o.sheetWide ? 'wide' : 'open') : 'closed'}">
            <main class="main" id="main"></main>
            ${sheet ? '<aside class="sheet" id="sheet-slot"></aside>' : ''}
          </div>
        </div>
      </div>`;
    document.body.prepend(app);
    app.querySelector('#main').append(screen.content.cloneNode(true));
    if (sheet) app.querySelector('#sheet-slot').append(sheet.content.cloneNode(true));
    if (overlay) app.append(overlay.content.cloneNode(true));
    document.title = o.title || document.title;
    if (!params.has('clean')) Shell.navigator();
  },

  // 原型导航：切换屏幕与明暗。不属于设计。
  navigator() {
    const nav = document.createElement('div');
    nav.style.cssText = 'position:fixed;left:50%;bottom:14px;transform:translateX(-50%);z-index:99;display:flex;gap:4px;padding:5px;border-radius:12px;background:rgba(20,20,22,.88);backdrop-filter:blur(10px);font:500 12px "Noto Sans SC",sans-serif';
    const dark = document.documentElement.dataset.theme === 'dark';
    const link = (href, label, on) => `<a href="${href}" style="padding:6px 10px;border-radius:8px;text-decoration:none;color:${on ? '#111' : '#bbb'};background:${on ? '#fff' : 'transparent'}">${label}</a>`;
    nav.innerHTML = link('index.html', '目录', false) + link('?theme=light', '浅色', !dark) + link('?theme=dark', '深色', dark);
    document.body.append(nav);
  },
};
