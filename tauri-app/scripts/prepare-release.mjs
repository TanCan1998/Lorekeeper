import fs from 'node:fs'
import { releaseIdentity, assertReleaseAbsent } from './release-helpers.mjs'

const identity = releaseIdentity(process.cwd(), process.env)
if (process.env.PUBLISH === 'true') {
  assertReleaseAbsent(identity.tag)
  if (!fs.existsSync(`docs-fork/releases/${identity.version}.md`)) throw new Error('缺少该版本发布说明')
}
fs.appendFileSync(process.env.GITHUB_OUTPUT, `source_sha=${identity.sha}\nversion=${identity.version}\ntag=${identity.tag}\n`)
console.log('准备完成：', JSON.stringify(identity), '发布：', process.env.PUBLISH)
