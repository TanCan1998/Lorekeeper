// 镜像已公开且通过既有门禁的发布：同一批文件上传，不创建新版本、不重打包。
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { RELEASE_REPOSITORY, ghJson, digest, verifyUploadedAssets } from './release-helpers.mjs'
import {
  assertMirrorIdentity, createAtomgitClient, withDownloadSources,
  uploadMirrorAssets, verifyMirrorDownloads,
} from './atomgit-release.mjs'

export async function mirrorPublishedRelease(tag, options = {}) {
  assert.equal(process.env.GITHUB_REPOSITORY, RELEASE_REPOSITORY, '禁止镜像其他仓库')
  assert(/^lorekeeper-v[0-9A-Za-z.-]+$/.test(tag), '镜像标签无效')
  const api = createAtomgitClient(process.env.ATOMGIT_ACCESS_TOKEN)
  const release = ghJson(['api', `repos/${RELEASE_REPOSITORY}/releases/tags/${tag}`])
  assert.equal(release.draft, false, '不能镜像草稿')
  const directory = options.directory || fs.mkdtempSync(path.join(os.tmpdir(), 'lorekeeper-mirror-'))
  if (!options.directory) {
    execFileSync('gh', ['release', 'download', tag, '-R', RELEASE_REPOSITORY, '--dir', directory], {
      stdio: ['ignore', 'pipe', 'pipe'],
    })
  }
  const names = fs.readdirSync(directory)
  assert(names.every(name => /^[0-9A-Za-z_.-]+$/.test(name)), '附件名称不能包含目录或控制符')
  for (const name of names) assert(fs.lstatSync(path.join(directory, name)).isFile(), '输入必须为普通文件，不能为链接')
  const manifest = JSON.parse(fs.readFileSync(path.join(directory, 'release-manifest.json'), 'utf8'))
  assertMirrorIdentity(release, manifest)
  assert.deepEqual(names.sort(), release.assets.map(a => a.name).sort(), '本地附件集合不一致')
  const expected = names.map(name => {
    const file = path.join(directory, name)
    return { name, bytes: fs.statSync(file).size, sha256: digest(file) }
  })
  verifyUploadedAssets(release, expected)
  for (const item of manifest.packages) {
    const file = expected.find(a => a.name === item.name)
    assert.equal(file.bytes, item.bytes)
    assert.equal(file.sha256, item.sha256)
  }
  // 保留同 SHA 独立 Rust 全量门禁，不能借用旧提交或仅有上传成功的记录。
  const validation = ghJson(['api', `repos/${RELEASE_REPOSITORY}/actions/runs/${manifest.rustValidationRunId}`])
  assert.equal(validation.head_sha, manifest.sourceCommit, 'Rust 验证不属于发布源码')
  assert.equal(validation.status, 'completed')
  assert.equal(validation.conclusion, 'success')
  assert.equal(validation.path, '.github/workflows/tauri-rust-ci.yml')
  const githubTag = ghJson(['api', `repos/${RELEASE_REPOSITORY}/git/ref/tags/${tag}`])
  assert.equal(githubTag.object.type, 'commit', '发布标签必须直接指向已验证提交')
  assert.equal(githubTag.object.sha, manifest.sourceCommit)
  let mirrorTag
  for (let page = 1; ; page++) {
    const tags = await api(`/tags?per_page=100&page=${page}`)
    assert(Array.isArray(tags), 'AtomGit 标签查询失败')
    mirrorTag = tags.find(item => item.name === tag)
    if (mirrorTag || tags.length < 100) break
  }
  assert(mirrorTag, 'AtomGit 缺少源码标签；请先完成代码同步')
  assert.equal(mirrorTag.commit.sha || mirrorTag.commit.id, manifest.sourceCommit, '两端标签源码不一致')
  let remote = await api(`/releases/${encodeURIComponent(tag)}`)
  if (remote) {
    assert.equal(remote.tag_name, tag)
    assert.equal(remote.target_commitish, manifest.sourceCommit, '已有镜像版本源码不一致')
    assert.equal(remote.prerelease, release.prerelease, '已有镜像发布类型不一致')
  } else {
    // AtomGit 没有已确认的草稿接口：传输期间明确标识未完成，不提前声明下载可用。
    remote = await api('/releases', 'POST', {
      tag_name: tag, target_commitish: manifest.sourceCommit,
      name: release.name, ...(release.prerelease ? { release_status: 'pre' } : {}),
      body: '附件同步与摘要核验尚未完成，请勿将此状态视为完整发布。\n\n' + release.body,
    })
  }
  await uploadMirrorAssets(api, tag, expected, directory)
  remote = await verifyMirrorDownloads(api, tag, expected)
  assert.equal(remote.target_commitish, manifest.sourceCommit)
  const body = withDownloadSources(release.body || '', tag, manifest.packages)
  // 两平台无法原子提交。先验证全部附件再更新说明，失败保留可恢复状态，不删除、不覆盖文件。
  await api(`/releases/${encodeURIComponent(tag)}`, 'PATCH', { name: release.name, body })
  ghJson(['api', `repos/${RELEASE_REPOSITORY}/releases/${release.id}`, '--method', 'PATCH', '-f', `body=${body}`])
  const finalAtomgit = await api(`/releases/${encodeURIComponent(tag)}`)
  const finalGithub = ghJson(['api', `repos/${RELEASE_REPOSITORY}/releases/${release.id}`])
  assert.equal(finalAtomgit.body, body, 'AtomGit 发布说明读回不一致')
  assert.equal(finalGithub.body, body, 'GitHub 发布说明读回不一致')
  assert.equal(finalGithub.prerelease, release.prerelease)
  assert.equal(finalAtomgit.prerelease, release.prerelease)
  verifyUploadedAssets(finalGithub, expected)
  console.log('GitHub / AtomGit 同批附件与双下载源说明已完成核验：' + tag)
  return { tag, assets: expected.length }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    await mirrorPublishedRelease(process.env.MIRROR_RELEASE_TAG, {
      directory: process.env.MIRROR_ASSET_DIR || undefined,
    })
  } catch {
    // 不打印可能含临时签名地址、认证信息的网络/子进程异常。
    console.error('Release 镜像未完成；请检查同 SHA 验证、附件摘要、两端标签、Secret 和平台权限。未自动删除或覆盖附件。')
    process.exitCode = 1
  }
}
