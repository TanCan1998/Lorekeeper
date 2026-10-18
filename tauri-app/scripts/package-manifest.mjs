#!/usr/bin/env node
/**
 * 安装包清单 —— 为本次打包产出可核验的 SHA256 与体积记录。
 *
 * 只读 `src-tauri/target/release/bundle/` 下本次真实生成的安装包，
 * 不修改、不重打包；清单随 artifact 一起上传，供人工核对。
 *
 * 用法：node scripts/package-manifest.mjs [--out <输出目录>]
 */
import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { execFileSync } from 'node:child_process'

const args = process.argv.slice(2)
const outIndex = args.indexOf('--out')
const outDir = path.resolve(outIndex === -1 ? 'package-manifest' : args[outIndex + 1])

const BUNDLE_ROOT = path.join('src-tauri', 'target', 'release', 'bundle')
const KINDS = [
  ['nsis', /\.exe$/i],
  ['msi', /\.msi$/i],
  ['dmg', /\.dmg$/i],
]

const packages = []
for (const [kind, pattern] of KINDS) {
  const dir = path.join(BUNDLE_ROOT, kind)
  if (!fs.existsSync(dir)) continue
  for (const name of fs.readdirSync(dir)) {
    if (!pattern.test(name)) continue
    const file = path.join(dir, name)
    const bytes = fs.statSync(file).size
    const sha256 = crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')
    packages.push({ kind, name, bytes, sha256 })
  }
}

if (packages.length === 0) {
  console.error(`未找到任何安装包（查找目录：${BUNDLE_ROOT}）`)
  process.exit(1)
}
packages.sort((a, b) => a.name.localeCompare(b.name))

const version = JSON.parse(fs.readFileSync('package.json', 'utf8')).version
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim()
if (process.env.PACKAGE_SOURCE_SHA && process.env.PACKAGE_SOURCE_SHA !== commit) {
  throw new Error('打包检出SHA与准备作业不一致')
}
const manifest = {
  schemaVersion: 1,
  productName: 'Lorekeeper',
  version,
  commit,
  workflowCommit: process.env.GITHUB_SHA || '',
  runId: process.env.GITHUB_RUN_ID || '',
  runAttempt: process.env.GITHUB_RUN_ATTEMPT || '',
  runnerOs: process.env.RUNNER_OS || '',
  runnerArch: process.env.RUNNER_ARCH || '',
  signed: false,
  published: false,
  packages,
}

fs.mkdirSync(outDir, { recursive: true })
fs.writeFileSync(path.join(outDir, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`)

console.log(`记录 ${packages.length} 个安装包 -> ${path.join(outDir, 'manifest.json')}`)
for (const item of packages) {
  console.log(`  ${item.name}  ${(item.bytes / 1048576).toFixed(1)} MB  sha256=${item.sha256}`)
}
