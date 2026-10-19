// AtomGit 中文首页优先展示 README_zh.md；该文件只由 README.md 生成，不单独维护。
import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const source = fs.readFileSync(path.join(root, 'README.md'))
const target = path.join(root, 'README_zh.md')
const args = process.argv.slice(2)
assert(args.length === 0 || (args.length === 1 && args[0] === '--check'), '仅支持 --check 或无参数同步')
if (args[0] === '--check') {
  assert(source.equals(fs.readFileSync(target)), 'README_zh.md 已漂移，请运行 node tauri-app/scripts/sync-readme.mjs')
  console.log('完整 README 与中文首页副本逐字节一致')
} else {
  fs.writeFileSync(target, source)
  console.log('已从 README.md 同步 README_zh.md，两端中文首页展示同一完整内容')
}
