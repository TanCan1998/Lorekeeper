import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { collectReleaseAssets, digest, releaseIdentity, verifyUploadedAssets } from './release-helpers.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const identity = { version: '1.0.0-beta.1', sha: 'a'.repeat(40), tag: 'lorekeeper-v1.0.0-beta.1', prerelease: true }

function fixture() {
  const dir = fs.mkdtempSync(path.join(root, '.pi/tmp/release-unit-'))
  const specs = [
    ['windows-x64', 'Windows', 'X64', [['nsis', 'x64-setup.exe'], ['msi', 'x64_zh-CN.msi']]],
    ['macos-arm64', 'macOS', 'ARM64', [['dmg', 'aarch64.dmg']]],
    ['macos-x64', 'macOS', 'X64', [['dmg', 'x64.dmg']]],
  ]
  const manifests = [], files = []
  for (const [label, os, arch, packages] of specs) {
    const platform = path.join(dir, `tauri-release-${label}-${identity.sha}-100-1`)
    fs.mkdirSync(platform)
    const items = packages.map(([kind, suffix]) => {
      const name = `LoreKeeper_${identity.version}_${suffix}`, file = path.join(platform, name)
      fs.writeFileSync(file, '仅用于验证发布门禁的合成字节：' + name)
      files.push(file)
      return { kind, name, bytes: fs.statSync(file).size, sha256: digest(file) }
    })
    const manifest = path.join(platform, 'manifest.json')
    fs.writeFileSync(manifest, JSON.stringify({
      schemaVersion: 1, productName: 'LoreKeeper', version: identity.version, commit: identity.sha,
      runId: '100', runAttempt: '1', runnerOs: os, runnerArch: arch,
      signed: false, published: false, packages: items,
    }))
    manifests.push(manifest)
  }
  return { dir, manifests, files }
}
const collect = f => collectReleaseAssets(f.dir, identity, '100', '1')
function mutate(file, change) {
  const data = JSON.parse(fs.readFileSync(file, 'utf8'))
  change(data)
  fs.writeFileSync(file, JSON.stringify(data))
}

test('精确收集同一SHA/运行的四个安装包和三份清单', () => {
  const result = collect(fixture())
  assert.equal(result.assets.length, 4)
  assert.equal(result.manifests.length, 3)
})

for (const [key, value] of [
  ['schemaVersion', 2], ['productName', 'Other'], ['version', '9.9.9'], ['commit', 'b'.repeat(40)],
  ['runId', '101'], ['runAttempt', '2'], ['runnerOs', 'Linux'], ['runnerArch', 'ARM64'],
  ['signed', true], ['published', true],
]) test('拒绝清单身份/声明漂移：' + key, () => {
  const f = fixture()
  mutate(f.manifests[0], m => { m[key] = value })
  assert.throws(() => collect(f))
})

for (const kind of ['sha256', 'bytes', 'name']) test('拒绝安装包清单损坏：' + kind, () => {
  const f = fixture()
  mutate(f.manifests[0], m => { m.packages[0][kind] = kind === 'bytes' ? 1 : kind === 'name' ? '../escape.exe' : '0'.repeat(64) })
  assert.throws(() => collect(f))
})

test('拒绝实际字节被替换或截断', () => {
  const f = fixture()
  fs.writeFileSync(f.files[0], '截断')
  assert.throws(() => collect(f))
})

test('拒绝额外文件、清单与平台', () => {
  for (const extra of ['file', 'manifest', 'platform']) {
    const f = fixture()
    if (extra === 'file') fs.writeFileSync(path.join(path.dirname(f.manifests[0]), 'unexpected.txt'), '多余')
    if (extra === 'manifest') { const d = path.join(path.dirname(f.manifests[0]), 'nested'); fs.mkdirSync(d); fs.copyFileSync(f.manifests[0], path.join(d, 'manifest.json')) }
    if (extra === 'platform') fs.mkdirSync(path.join(f.dir, 'another-run'))
    assert.throws(() => collect(f))
  }
})

test('远端必须大小、摘要、状态与完整资产集合全部匹配', () => {
  const expected = [{ name: 'installer.exe', bytes: 10, sha256: 'a'.repeat(64) }]
  const valid = { assets: [{ name: 'installer.exe', size: 10, digest: 'sha256:' + 'a'.repeat(64), state: 'uploaded' }] }
  verifyUploadedAssets(valid, expected)
  for (const change of [
    r => { r.assets[0].size++ }, r => { r.assets[0].digest = undefined },
    r => { r.assets[0].state = 'new' }, r => { r.assets.push({ ...r.assets[0], name: 'extra' }) },
  ]) { const r = structuredClone(valid); change(r); assert.throws(() => verifyUploadedAssets(r, expected)) }
})

test('版本身份拒绝错误仓库、标签、SHA和无效发布选项', () => {
  const env = { GITHUB_REPOSITORY: 'TanCan1998/Lorekeeper', PUBLISH: 'true', PRERELEASE: 'true', MAKE_LATEST: 'false' }
  assert.equal(releaseIdentity(root, env).tag, identity.tag)
  for (const patch of [
    { GITHUB_REPOSITORY: 'EthanYoQ/AI-Novel-Writer' }, { RELEASE_TAG: 'v1.0.0' },
    { EXPECTED_SHA: 'b'.repeat(40) }, { PRERELEASE: 'other' }, { PUBLISH: 'other' },
    { MAKE_LATEST: 'other' }, { MAKE_LATEST: undefined }, { MAKE_LATEST: 'true' },
  ]) assert.throws(() => releaseIdentity(root, { ...env, ...patch }))
})

test('beta名称可作为普通Release并显式选择Latest，仍保留预览身份', () => {
  for (const makeLatest of ['true', 'false']) {
    const result = releaseIdentity(root, {
      GITHUB_REPOSITORY: 'TanCan1998/Lorekeeper', PUBLISH: 'true', PRERELEASE: 'false', MAKE_LATEST: makeLatest,
    })
    assert.equal(result.preview, true)
    assert.equal(result.prerelease, false)
    assert.equal(result.makeLatest, makeLatest === 'true')
    assert.equal(result.version, '1.0.0-beta.1')
  }
})
