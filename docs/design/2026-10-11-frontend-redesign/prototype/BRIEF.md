# 原型制作说明（给并行制作屏幕的子 Agent）

本目录是前端重新设计方向 A「纸墨」的高保真静态原型。它用来确认设计，不是产品代码。

## 必读

1. `../ia.md`：信息架构与已确认决定。结构以它为准。
2. `../feature-matrix.md`：功能保全矩阵。你负责的屏幕要覆盖分配给你的 F 编号。
3. `chapter-review.html`、`setup.html`：参考屏。照它们的写法、密度和语气做。
4. `tokens.css`、`ui.css`、`shell.js`、`icons.js`：共用层。先读完再动手。

## 写法

- 每个屏幕一个 HTML，结构照参考屏：`<template id="screen">` 放主内容；需要侧页时加 `<template id="sheet">`；需要对话框、菜单、弹层、提示时加 `<template id="overlay">`（绝对定位，放在 `.app` 里）。最后调用 `Shell.mount({...})`。
- `Shell.mount` 参数：`title`、`stage`（setup / arch / plan / text）、`ref`（plot / knowledge / history / settings）、`preset`（mid / new / arch）、`chapter`（正文当前章号）、`chapters`（是否在流程脊显示章节列表）、`crumbs`、`tabs`（多于一个才显示）、`run`（`{label, pct, open}`）、`saved`、`spine:false` 与 `work:null`（书房用）。
- **禁止修改** `tokens.css`、`ui.css`、`shell.js`、`icons.js` 与参考屏。需要新组件时，写在你自己页面的 `<style>` 里，并在回报中列出，由主 Agent 决定是否提升到共用层。
- 页面 `<style>` 里只能用 `var(--…)` 令牌，不写任何十六进制或 rgb 颜色值，也不按元素位置、类名组合或文字内容选中元素。
- 图标只用 `icons.js` 里已有的。确实缺的，用 better-icons MCP 取 Lucide 图标，以 `<svg width="0" height="0" style="position:absolute"><symbol id="i-名称" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">…</symbol></svg>` 内嵌在你的页面里，并删掉 MCP 返回的内层 `fill="currentColor"`。禁止用 emoji 或文字符号充当图标。

## 设计规则（审查会逐条检查）

- 每个屏幕**只有一个主按钮**（`.btn-primary`），它是流程上的下一步。AI 生成类次要操作用 `.btn-secondary.btn-ai`。
- **危险操作**（删除、清空、覆盖）不放在一级工具栏；放进“更多”菜单的独立分组（`.menu-item.danger`），或页面底部的 `.danger-zone`。触发后用危险确认对话框（`.dialog.danger` + `.btn-danger-solid`）。矩阵里标为“无确认”的危险操作，新设计必须有确认。
- 不出现实现术语：LanceDB、FTS、BM25、Embedding、向量、query、MCP、token 等。必须出现的技术项只在设置“高级”里，用作者能懂的话说明。
- 空状态要设计：一句衬线标题 + 一句说明 + 一个动作。
- 示例内容统一用作品《雨夜列车》（悬疑，24 章，主角林舟），章节名见 `shell.js` 的 `SAMPLE`。中文文案简短、具体，不用“赋能”“一键”之类的词。
- 动效只用于状态变化（侧页滑入、弹层出现），不做装饰动画。

## 自检

1. 本地服务已在 `http://127.0.0.1:4780/` 运行，根目录是 `docs/design/2026-10-11-frontend-redesign`。不要另起服务。
2. 截图：在 PowerShell 里运行 `& "<本目录>\shoot.ps1" -Pages 你的页面名`，会生成浅色与深色两张 1440×900 截图到 `shots/`。
3. 用 Read 工具逐张看截图，修掉文字溢出、折行、对齐、对比度不足、深色模式下看不清的问题，再重拍，直到满意。

## 回报

- 新建的文件列表。
- 覆盖表：F 编号 → 屏幕与元素。分配给你但没覆盖的 F 编号，写明原因。
- 你写在页面里、建议提升到共用层的组件。
- 你认为信息架构里需要主 Agent 决定的问题（不要自己改 `ia.md`）。
