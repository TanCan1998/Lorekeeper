import assert from 'node:assert/strict'
import crypto from 'node:crypto'
import fs from 'node:fs'
import { Readable } from 'node:stream'

export const MIRROR_REPOSITORY = 'TanCan1998/Lorekeeper'
export const ATOMGIT_API = `https://api.atomgit.com/api/v5/repos/${MIRROR_REPOSITORY}`

export function assetDownloadUrl(platform, tag, name) {
  assert(['github', 'atomgit'].includes(platform), '未知下载平台')
  assert(/^lorekeeper-v[0-9A-Za-z.-]+$/.test(tag), '标签无效')
  assert(/^[0-9A-Za-z_.-]+$/.test(name), '附件名称无效')
  return `https://${platform}.com/${MIRROR_REPOSITORY}/releases/download/${tag}/${name}`
}

export function withDownloadSources(notes, tag, packages) {
  assert.equal(packages.length, 4, '下载表格必须包含四个安装包')
  const label = item => item.kind === 'nsis' ? 'Windows x64 · EXE'
    : item.kind === 'msi' ? 'Windows x64 · MSI'
      : item.name.endsWith('_aarch64.dmg') ? 'macOS Apple Silicon' : 'macOS Intel'
  const begin = '<!-- lorekeeper-downloads:start -->'
  const end = '<!-- lorekeeper-downloads:end -->'
  const table = [
    begin,
    '**中国大陆用户推荐使用 AtomGit；两端安装包字节完全相同，无需重复下载。**',
    '',
    '| 平台 / 格式 | AtomGit（国内推荐） | GitHub |',
    '| --- | --- | --- |',
    ...packages.map(item => `| ${label(item)} | [下载](${assetDownloadUrl('atomgit', tag, item.name)}) | [下载](${assetDownloadUrl('github', tag, item.name)}) |`),
    '',
    `SHA256 校验文件：[AtomGit](${assetDownloadUrl('atomgit', tag, 'SHA256SUMS.txt')}) · [GitHub](${assetDownloadUrl('github', tag, 'SHA256SUMS.txt')})。`,
    'Windows 的 EXE 和 MSI 二选一；镜像不改变版本、签名或已知限制。',
    end,
  ].join('\n')
  if (notes.includes(begin)) {
    assert(notes.includes(end), '下载表格标记不完整')
    const start = notes.indexOf(begin), finish = notes.indexOf(end) + end.length
    return notes.slice(0, start) + table + notes.slice(finish)
  }
  const oldTable = /(^## 下载选择\s*\n)(?:\s*\n)*((?:\|[^\n]*\n)+)/m
  if (oldTable.test(notes)) return notes.replace(oldTable, (_all, heading) => heading + '\n' + table + '\n')
  return notes.trimEnd() + '\n\n## 下载选择\n\n' + table + '\n'
}

export function assertMirrorIdentity(release, manifest) {
  assert.equal(release.draft, false, '不镜像 GitHub 草稿')
  assert.equal(manifest.schemaVersion, 1)
  assert.equal(release.tag_name, `lorekeeper-v${manifest.version}`)
  assert.equal(manifest.tag, release.tag_name)
  assert(/^[a-f0-9]{40}$/.test(manifest.sourceCommit), '清单源码身份无效')
  assert(/^\d+$/.test(String(manifest.rustValidationRunId)), '缺少独立 Rust 验证记录')
  assert.equal(manifest.prerelease, release.prerelease)
  const legacy = manifest.version === '1.0.0-beta'
  assert.equal(manifest.productName, legacy ? 'Lorekeeper' : 'LoreKeeper')
  const prefix = `${manifest.productName}_${manifest.version}`
  const names = [
    `${prefix}_x64-setup.exe`, `${prefix}_x64_${legacy ? 'en-US' : 'zh-CN'}.msi`,
    `${prefix}_aarch64.dmg`, `${prefix}_x64.dmg`,
  ]
  assert.deepEqual(manifest.packages.map(p => p.name).sort(), names.sort(), '安装包集合错误')
  assert.deepEqual(manifest.packages.map(p => p.kind).sort(), ['dmg', 'dmg', 'msi', 'nsis'])
  const expected = [...names, 'SHA256SUMS.txt', 'release-manifest.json',
    'package-manifest-windows-x64.json', 'package-manifest-macos-arm64.json', 'package-manifest-macos-x64.json']
  assert.deepEqual(release.assets.map(a => a.name).sort(), expected.sort(), 'GitHub 资产集合不完整')
}

export function createAtomgitClient(token, request = fetch) {
  assert(typeof token === 'string' && token.length > 0, '缺少 AtomGit 仓库 Secret')
  return async (suffix, method = 'GET', data) => {
    assert(suffix.startsWith('/') && !suffix.includes('..'), 'API 路径无效')
    const response = await request(ATOMGIT_API + suffix, {
      method, redirect: 'error', signal: AbortSignal.timeout(60000),
      headers: { Authorization: `Bearer ${token}`, Accept: 'application/json', 'Content-Type': 'application/json' },
      ...(data === undefined ? {} : { body: JSON.stringify(data) }),
    })
    if (response.status === 404 && method === 'GET') return null
    assert(response.ok, `AtomGit API 请求失败（HTTP ${response.status}）`)
    return response.json()
  }
}

export async function verifyMirrorDownloads(api, tag, expected, request = fetch) {
  const remote = await api(`/releases/${encodeURIComponent(tag)}`)
  assert(remote, 'AtomGit Release 不存在')
  const attachments = remote.assets.filter(a => a.type === 'attach')
  assert.deepEqual(attachments.map(a => a.name).sort(), expected.map(a => a.name).sort(), 'AtomGit 附件集合不一致')
  // AtomGit 未返回附件 SHA256 或大小，因此匿名流式读回实际文件，不把上传成功当作校验通过。
  for (const item of expected) {
    const url = `${ATOMGIT_API}/releases/${encodeURIComponent(tag)}/attach_files/${encodeURIComponent(item.name)}/download`
    const response = await request(url, { signal: AbortSignal.timeout(600000) })
    assert(response.ok && response.body, `AtomGit 附件匿名读取失败：${item.name}`)
    const hash = crypto.createHash('sha256')
    let bytes = 0
    for await (const chunk of response.body) {
      bytes += chunk.length
      assert(bytes <= item.bytes, '镜像附件超出预期大小')
      hash.update(chunk)
    }
    assert.equal(bytes, item.bytes, '镜像附件大小不一致')
    assert.equal(hash.digest('hex'), item.sha256, '镜像附件摘要不一致')
  }
  return remote
}

export async function uploadMirrorAssets(api, tag, expected, directory, request = fetch) {
  const remote = await api(`/releases/${encodeURIComponent(tag)}`)
  const existing = new Set(remote.assets.filter(a => a.type === 'attach').map(a => a.name))
  assert([...existing].every(name => expected.some(a => a.name === name)), '存在未知镜像附件，拒绝覆盖')
  for (const item of expected) {
    if (existing.has(item.name)) continue
    const upload = await api(`/releases/${encodeURIComponent(tag)}/upload_url?file_name=${encodeURIComponent(item.name)}`)
    const url = new URL(upload.url)
    assert.equal(url.protocol, 'https:')
    assert.equal(url.hostname, 'file.gitcode.com', '非预期上传域名')
    assert(!url.username && !url.password, '上传地址不能带用户凭据')
    const headers = { ...upload.headers, 'Content-Length': String(item.bytes) }
    assert(!Object.keys(headers).some(k => /authorization|cookie|host/i.test(k)), '上传头不能包含额外凭据')
    const response = await request(url, {
      method: 'PUT', redirect: 'error', signal: AbortSignal.timeout(600000),
      headers, body: Readable.toWeb(fs.createReadStream(directory + '/' + item.name)), duplex: 'half',
    })
    assert(response.ok, `AtomGit 上传失败（HTTP ${response.status}）`)
    await response.arrayBuffer()
  }
}
