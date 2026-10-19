import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { execFileSync } from 'node:child_process'
import { mirrorPublishedRelease } from './mirror-published-release.mjs'
import {
  RELEASE_REPOSITORY, releaseIdentity, assertReleaseAbsent, collectReleaseAssets,
  digest, ghJson, verifyUploadedAssets,
} from './release-helpers.mjs'

assert.equal(process.env.PUBLISH, 'true', '必须显式选择发布')
assert(process.env.ATOMGIT_ACCESS_TOKEN, '双平台发布必须先配置 AtomGit 仓库 Secret')
const identity = releaseIdentity(process.cwd(), process.env)
// 不在打包流运行测试；发布前必须已有同一源码SHA的独立Rust全量成功记录。
const validations = ghJson(['api', `repos/${RELEASE_REPOSITORY}/actions/workflows/tauri-rust-ci.yml/runs?head_sha=${identity.sha}&per_page=20`])
const rustValidation = validations.workflow_runs.find(r => r.head_sha === identity.sha && r.status === 'completed' && r.conclusion === 'success')
assert(rustValidation, '同一源码SHA的Rust全量CI尚未成功，拒绝发布')
assertReleaseAbsent(identity.tag)
const { assets, manifests } = collectReleaseAssets(
  process.env.ARTIFACT_ROOT, identity, process.env.GITHUB_RUN_ID, process.env.GITHUB_RUN_ATTEMPT,
)
const output = process.env.RELEASE_OUTPUT
// 使用独立新目录，避免把旧文件或其他运行产物一并上传。
fs.mkdirSync(output)
for (const manifest of manifests) {
  fs.copyFileSync(manifest.file, path.join(output, `package-manifest-${manifest.label}.json`))
}
fs.writeFileSync(path.join(output, 'SHA256SUMS.txt'), assets.map(p => `${p.sha256}  ${p.name}`).join('\n') + '\n')
fs.writeFileSync(path.join(output, 'release-manifest.json'), JSON.stringify({
  schemaVersion: 1, productName: 'LoreKeeper', version: identity.version, sourceCommit: identity.sha,
  tag: identity.tag, prerelease: identity.prerelease, makeLatest: identity.makeLatest,
  preview: identity.preview, runId: process.env.GITHUB_RUN_ID,
  runAttempt: process.env.GITHUB_RUN_ATTEMPT, rustValidationRunId: String(rustValidation.id),
  certificateSigned: false, notarized: false,
  installationSmokeTested: false, packages: assets.map(({ file: _file, ...item }) => item),
}, null, 2) + '\n')
const upload = [...assets.map(p => p.file), ...fs.readdirSync(output).map(n => path.join(output, n))]
const expected = upload.map(file => ({ name: path.basename(file), bytes: fs.statSync(file).size, sha256: digest(file) }))
const run = args => execFileSync('gh', args, { stdio: ['ignore', 'pipe', 'pipe'], encoding: 'utf8' })
const repo = ['-R', RELEASE_REPOSITORY]
// 原子创建新标签；并发出现同名标签也失败，不复用/移动别人的标签。
ghJson(['api', `repos/${RELEASE_REPOSITORY}/git/refs`, '--method', 'POST',
  '-f', `ref=refs/tags/${identity.tag}`, '-f', `sha=${identity.sha}`])
// 先建立草稿并上传，任何失败都保留草稿，不公开半成品，不覆盖已有版本。
const created = ghJson(['api', `repos/${RELEASE_REPOSITORY}/releases`, '--method', 'POST',
  '-f', `tag_name=${identity.tag}`, '-f', `target_commitish=${identity.sha}`,
  '-f', `name=LoreKeeper ${identity.version}${identity.preview ? '（预览版）' : ''}`, '-f', `body=${fs.readFileSync(`docs-fork/releases/${identity.version}.md`, 'utf8')}`,
  '-F', 'draft=true', '-F', `prerelease=${identity.prerelease}`, '-f', 'make_latest=false'])
assert(Number.isSafeInteger(created.id) && created.id > 0)
run(['release', 'upload', identity.tag, ...repo, ...upload])
const endpoint = `repos/${RELEASE_REPOSITORY}/releases/${created.id}`
const draft = ghJson(['api', endpoint])
assert.equal(draft.draft, true)
assert.equal(draft.tag_name, identity.tag)
assert.equal(draft.prerelease, identity.prerelease)
verifyUploadedAssets(draft, expected)
const beforePublish = ghJson(['api', `repos/${RELEASE_REPOSITORY}/git/ref/tags/${identity.tag}`])
assert.equal(beforePublish.object.sha, identity.sha)
ghJson(['api', endpoint, '--method', 'PATCH', '-F', 'draft=false',
  '-F', `prerelease=${identity.prerelease}`, '-f', `make_latest=${identity.makeLatest}`])
const published = ghJson(['api', endpoint])
assert.equal(published.draft, false)
assert.equal(published.prerelease, identity.prerelease)
verifyUploadedAssets(published, expected)
const tag = ghJson(['api', `repos/${RELEASE_REPOSITORY}/git/ref/tags/${identity.tag}`])
assert.equal(tag.object.type, 'commit')
assert.equal(tag.object.sha, identity.sha, '发布标签不指向已校验源码')
if (identity.makeLatest) {
  const latest = ghJson(['api', `repos/${RELEASE_REPOSITORY}/releases/latest`])
  assert.equal(latest.id, published.id, '最新版指针未指向本次发布')
}
// GITHUB_TOKEN 创建 Release 不触发另一条 release 工作流，必须在本发布作业内显式同步。
// 同步只推送已提交对象和上传原样文件，不重新构建。
try {
  execFileSync(process.execPath, ['tauri-app/scripts/sync-atomgit.mjs'], { stdio: ['ignore', 'pipe', 'pipe'] })
  await mirrorPublishedRelease(identity.tag)
} catch {
  throw new Error('GitHub 已发布，但 AtomGit 同步未完成；请用镜像工作流重试同一标签，不要重跑打包或覆盖附件')
}
fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, `## 双平台发布完成\n\n${published.html_url}\n\n源码：\`${identity.sha}\`；安装包4个，全部资产${expected.length}个，大小/SHA256已核验。\n`)
console.log('发布完成：', published.html_url, '源码：', identity.sha, '资产：', expected.length)
