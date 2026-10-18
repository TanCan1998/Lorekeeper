#!/usr/bin/env node
/**
 * 打包前端产物体检 —— 保证安装包里只有本项目内容。
 *
 * 背景：Tauri 前端通过 `@baseline` 别名引用 Electron 基线的**纯类型**契约，
 * 并复用仓库根 `public/fonts`（与 Electron 共用一份字体来源）。这两条路径都
 * 在 `tauri-app/` 之外，一旦有人把它们写成值导入，上游代码就会被打进安装包。
 * 本脚本在 `tauri build` 之前拦截这类回归。
 *
 * 用法：node scripts/assert-package-contents.mjs [--dist <目录>]
 */
import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'

const args = process.argv.slice(2)
const distIndex = args.indexOf('--dist')
const distRoot = path.resolve(distIndex === -1 ? 'dist' : args[distIndex + 1])

/** 出现在产物里即判定失败的上游/Electron 标识。 */
const FORBIDDEN_TOKENS = [
  'EthanYoQ', // 上游仓库归属
  'AI-Novel-Writer', // 上游仓库名
  'electron-vite', // 脚手架残留品牌
  'AI Novel Writer', // 旧英文品牌（界面已统一为 Lorekeeper）
  'AI小说作家', // 旧中文品牌
  'contextBridge', // Electron preload 专有
  'ipcMain', // Electron 主进程专有
  'better-sqlite3', // Electron 侧数据库驱动
]

/** 需要做文本扫描的产物类型（字体/图片是二进制，跳过）。 */
const TEXT_FILE = /\.(html|js|mjs|cjs|css|json|svg|txt|webmanifest)$/i

if (!fs.existsSync(distRoot)) {
  console.error(`产物目录不存在：${distRoot}`)
  process.exit(1)
}

/** @returns {string[]} 绝对路径列表 */
function walk(dir) {
  const out = []
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) out.push(...walk(full))
    else if (entry.isFile()) out.push(full)
  }
  return out
}

const files = walk(distRoot)
const problems = []
const fonts = []
let totalBytes = 0
const digests = crypto.createHash('sha256')

for (const file of files) {
  const stat = fs.statSync(file)
  totalBytes += stat.size
  const rel = path.relative(distRoot, file).split(path.sep).join('/')

  if (/\.ttf$/i.test(rel)) {
    problems.push(`TTF 不应进入产物（应全部为 WOFF2）：${rel}`)
  }
  if (/\.woff2$/i.test(rel)) fonts.push(rel)

  if (!TEXT_FILE.test(rel)) continue
  const text = fs.readFileSync(file, 'utf8')
  for (const token of FORBIDDEN_TOKENS) {
    if (text.includes(token)) problems.push(`产物含上游/Electron 标识 "${token}"：${rel}`)
  }
  digests.update(rel).update('\0').update(text)
}

if (fonts.length === 0) {
  problems.push('产物中没有任何 WOFF2 字体，字体引用可能已断链')
}

const sizeMb = (totalBytes / 1048576).toFixed(1)
console.log(`产物目录：${distRoot}`)
console.log(`  总体积 ${sizeMb} MB，文件 ${files.length} 个，WOFF2 字体 ${fonts.length} 个`)
console.log(`  内容摘要 ${digests.digest('hex').slice(0, 16)}…`)

if (problems.length > 0) {
  console.error(`\n发现 ${problems.length} 个问题：`)
  for (const problem of problems) console.error(`  - ${problem}`)
  process.exit(1)
}

console.log('  校验通过：无上游标识、无 TTF、字体为 WOFF2')
