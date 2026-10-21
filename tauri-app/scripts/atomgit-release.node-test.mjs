import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'
import {
  assetDownloadUrl, withDownloadSources, assertMirrorIdentity, createAtomgitClient,
} from './atomgit-release.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const tag = 'lorekeeper-v1.0.0-beta.1'
const packages = [
  { kind: 'nsis', name: 'LoreKeeper_1.0.0-beta.1_x64-setup.exe' },
  { kind: 'msi', name: 'LoreKeeper_1.0.0-beta.1_x64_zh-CN.msi' },
  { kind: 'dmg', name: 'LoreKeeper_1.0.0-beta.1_aarch64.dmg' },
  { kind: 'dmg', name: 'LoreKeeper_1.0.0-beta.1_x64.dmg' },
]

test('两端使用明确绝对下载地址，拒绝路径注入', () => {
  assert.equal(assetDownloadUrl('atomgit', tag, packages[0].name),
    'https://atomgit.com/TanCan1998/Lorekeeper/releases/download/' + tag + '/' + packages[0].name)
  assert.throws(() => assetDownloadUrl('other', tag, 'a.exe'))
  assert.throws(() => assetDownloadUrl('github', '../tag', 'a.exe'))
  assert.throws(() => assetDownloadUrl('github', tag, '../a.exe'))
})

test('同一说明只生成一份双源表格，保留既有限制和安装提醒', () => {
  const notes = '# 预览版\n\n## 下载选择\n\n| 系统 | 安装包 |\n| --- | --- |\n| Windows | 旧链接 |\n\n重要安装提醒。\n\n## 限制\n仍未认证。\n'
  const first = withDownloadSources(notes, tag, packages)
  assert.equal(withDownloadSources(first, tag, packages), first)
  assert(first.includes('中国大陆用户推荐使用 AtomGit'))
  assert(first.includes('重要安装提醒。'))
  assert(first.includes('仍未认证。'))
  assert(!first.includes('旧链接'))
  for (const name of packages.map(p => p.name)) {
    assert(first.includes(assetDownloadUrl('github', tag, name)))
    assert(first.includes(assetDownloadUrl('atomgit', tag, name)))
  }
})

test('原生发布声明、同版本标签和精确九附件均须保留', () => {
  const manifest = {
    schemaVersion: 1, version: '1.0.0-beta.1', productName: 'LoreKeeper', tag,
    sourceCommit: 'a'.repeat(40), rustValidationRunId: '123', prerelease: true, packages,
  }
  const release = {
    draft: false, tag_name: tag, prerelease: true,
    assets: [...packages.map(p => p.name), 'SHA256SUMS.txt', 'release-manifest.json',
      'package-manifest-windows-x64.json', 'package-manifest-macos-arm64.json',
      'package-manifest-macos-x64.json'].map(name => ({ name })),
  }
  assertMirrorIdentity(release, manifest)
  assert.throws(() => assertMirrorIdentity({ ...release, draft: true }, manifest))
  assert.throws(() => assertMirrorIdentity({ ...release, prerelease: false }, manifest))
  assert.throws(() => assertMirrorIdentity({ ...release, assets: release.assets.slice(1) }, manifest))
  assert.throws(() => assertMirrorIdentity(release, { ...manifest, sourceCommit: 'invalid' }))
})

test('认证只发往固定官方 API，禁止自动重定向携带令牌', async () => {
  let captured
  const client = createAtomgitClient('仅用于合成测试的占位值', async (url, options) => {
    captured = { url, options }
    return { ok: true, status: 200, json: async () => ({ tag_name: tag }) }
  })
  await client('/releases/' + tag)
  assert(captured.url.startsWith('https://api.atomgit.com/api/v5/repos/TanCan1998/Lorekeeper/'))
  assert.equal(captured.options.redirect, 'error')
  await assert.rejects(client('/../other'))
  assert.throws(() => createAtomgitClient(''))
})

test('GitHub 发布不再推送 AtomGit；代码由平台镜像拉取，附件上传保持为独立工具', () => {
  const publish = fs.readFileSync(path.join(root, 'tauri-app/scripts/publish-release.mjs'), 'utf8')
  const workflowRoot = path.join(root, '.github/workflows')
  // 代码同步已改为 AtomGit 平台拉取：GitHub 侧不留推送工作流与推送脚本。
  assert(!fs.readdirSync(workflowRoot).some(name => name === 'atomgit-mirror.yml'))
  assert(!fs.existsSync(path.join(root, 'tauri-app/scripts/sync-atomgit.mjs')))
  assert(!publish.includes('mirrorPublishedRelease'))
  assert(!publish.includes('sync-atomgit'))
  // 附件不在 Git 镜像范围，仍保留只上传原样文件的手动工具，且不重新构建。
  const mirror = fs.readFileSync(path.join(root, 'tauri-app/scripts/mirror-published-release.mjs'), 'utf8')
  assert(mirror.includes('uploadMirrorAssets'))
  assert(!/cargo (?:build|test)|pnpm (?:build|test)|tauri build/.test(mirror))
})
