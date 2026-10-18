import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { execFileSync, spawnSync } from 'node:child_process'

export const RELEASE_REPOSITORY = 'TanCan1998/Lorekeeper'
const SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/
export const digest = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')

export function releaseIdentity(root, env) {
  assert.equal(env.GITHUB_REPOSITORY, RELEASE_REPOSITORY, '禁止向其他仓库发布')
  const version = JSON.parse(fs.readFileSync(path.join(root, 'tauri-app/package.json'), 'utf8')).version
  assert(SEMVER.test(version), '无效的应用版本')
  const config = JSON.parse(fs.readFileSync(path.join(root, 'tauri-app/src-tauri/tauri.conf.json'), 'utf8'))
  const cargo = fs.readFileSync(path.join(root, 'tauri-app/src-tauri/Cargo.toml'), 'utf8')
  const lock = fs.readFileSync(path.join(root, 'tauri-app/src-tauri/Cargo.lock'), 'utf8')
  assert.equal(config.version, version, 'Tauri配置版本不一致')
  assert.equal(cargo.match(/^version = "([^"]+)"/m)?.[1], version, 'Cargo版本不一致')
  assert.equal(lock.match(/name = "lorekeeper"\r?\nversion = "([^"]+)"/)?.[1], version, 'Cargo锁项目版本不一致')
  const sha = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
  assert(/^[a-f0-9]{40}$/.test(sha), '检出SHA无效')
  if (env.EXPECTED_SHA) assert.equal(env.EXPECTED_SHA, sha, '检出SHA与指定目标不一致')
  const tag = env.RELEASE_TAG || `lorekeeper-v${version}`
  assert.equal(tag, `lorekeeper-v${version}`, '标签必须匹配Lorekeeper版本，禁止复用上游标签')
  assert(['true', 'false'].includes(env.PUBLISH), '发布选项无效')
  assert(['true', 'false'].includes(env.PRERELEASE), '预发布选项无效')
  assert(['true', 'false'].includes(env.MAKE_LATEST), '最新版选项无效')
  const prerelease = env.PRERELEASE === 'true'
  const makeLatest = env.MAKE_LATEST === 'true'
  assert(!(prerelease && makeLatest), 'GitHub预发布不能设为最新版')
  const preview = version.includes('-')
  // beta名称可以使用普通Release，但发布说明必须明确预览，不能冒充正式验收。
  if (env.PUBLISH === 'true' && preview) {
    const notes = fs.readFileSync(path.join(root, `docs-fork/releases/${version}.md`), 'utf8')
    assert(/^# .*预览/m.test(notes) && /非正式版/.test(notes), '先行版本发布说明必须明确预览及非正式版')
  }
  return { version, sha, tag, prerelease, makeLatest, preview }
}

export function ghJson(args) {
  return JSON.parse(execFileSync('gh', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }))
}

export function assertReleaseAbsent(tag) {
  for (const endpoint of [`releases/tags/${tag}`, `git/ref/tags/${tag}`]) {
    const result = spawnSync('gh', ['api', `repos/${RELEASE_REPOSITORY}/${endpoint}`], { encoding: 'utf8' })
    if (result.status === 0) throw new Error('标签或Release已存在，拒绝覆盖：' + tag)
    assert(/HTTP 404/.test(result.stderr), '无法确认标签/Release不存在：' + result.stderr)
  }
}

function walk(dir) {
  return fs.readdirSync(dir).flatMap(name => {
    const file = path.join(dir, name), stat = fs.lstatSync(file)
    assert(!stat.isSymbolicLink(), '发布输入不能包含符号链接')
    return stat.isDirectory() ? walk(file) : [file]
  })
}

export function collectReleaseAssets(artifactRoot, identity, runId, runAttempt) {
  assert(/^\d+$/.test(runId) && /^\d+$/.test(runAttempt), '运行身份无效')
  const { version, sha } = identity
  const specs = [
    { label: 'windows-x64', os: 'Windows', arch: 'X64', names: [
      ['nsis', `LoreKeeper_${version}_x64-setup.exe`], ['msi', `LoreKeeper_${version}_x64_zh-CN.msi`],
    ] },
    { label: 'macos-arm64', os: 'macOS', arch: 'ARM64', names: [['dmg', `LoreKeeper_${version}_aarch64.dmg`]] },
    { label: 'macos-x64', os: 'macOS', arch: 'X64', names: [['dmg', `LoreKeeper_${version}_x64.dmg`]] },
  ]
  const directories = specs.map(s => `tauri-release-${s.label}-${sha}-${runId}-${runAttempt}`)
  assert.deepEqual(fs.readdirSync(artifactRoot).sort(), [...directories].sort(), '必须且只能有本次三平台产物')
  const assets = [], manifests = []
  for (const [index, spec] of specs.entries()) {
    const files = walk(path.join(artifactRoot, directories[index]))
    const manifestFiles = files.filter(f => path.basename(f) === 'manifest.json')
    assert.equal(manifestFiles.length, 1, '平台清单数量错误')
    const manifest = JSON.parse(fs.readFileSync(manifestFiles[0], 'utf8'))
    assert.equal(manifest.schemaVersion, 1)
    assert.equal(manifest.productName, 'LoreKeeper')
    assert.equal(manifest.version, version)
    assert.equal(manifest.commit, sha, '产物SHA与检出不一致')
    assert.equal(manifest.runId, runId)
    assert.equal(manifest.runAttempt, runAttempt)
    assert.equal(manifest.runnerOs, spec.os)
    assert.equal(manifest.runnerArch, spec.arch)
    assert.equal(manifest.signed, false, '此流程不声称证书签名')
    assert.equal(manifest.published, false, '输入必须是尚未发布的打包清单')
    assert.deepEqual(manifest.packages.map(p => [p.kind, p.name]).sort(), [...spec.names].sort(), '安装包集合错误')
    assert.equal(files.length, 1 + spec.names.length, '产物中存在额外文件')
    for (const item of manifest.packages) {
      assert.equal(item.name, path.basename(item.name), '安装包名不能包含目录')
      const matches = files.filter(f => path.basename(f) === item.name)
      assert.equal(matches.length, 1, '安装包缺失或重名')
      assert(Number.isSafeInteger(item.bytes) && item.bytes > 0)
      assert(/^[a-f0-9]{64}$/.test(item.sha256))
      assert.equal(fs.statSync(matches[0]).size, item.bytes, '安装包体积不匹配')
      assert.equal(digest(matches[0]), item.sha256, '安装包摘要不匹配')
      assets.push({ ...item, file: matches[0] })
    }
    manifests.push({ label: spec.label, file: manifestFiles[0] })
  }
  return { assets: assets.sort((a, b) => a.name.localeCompare(b.name)), manifests }
}

export function verifyUploadedAssets(release, expected) {
  assert.deepEqual(release.assets.map(a => a.name).sort(), expected.map(a => a.name).sort(), '远端资产集合不一致')
  for (const item of expected) {
    const remote = release.assets.find(a => a.name === item.name)
    assert.equal(remote.state, 'uploaded')
    assert.equal(remote.size, item.bytes, '远端资产大小错误')
    assert.equal(remote.digest, `sha256:${item.sha256}`, '远端资产摘要错误或缺失')
  }
}
