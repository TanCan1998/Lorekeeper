import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')

function uploadBlocks(source) {
  const lines = source.split('\n'), blocks = []
  for (let i = 0; i < lines.length; i++) {
    const match = lines[i].match(/^(\s*)(?:-\s*)?uses:\s*actions\/upload-artifact@/)
    if (!match) continue
    const stepIndent = match[1].length - (lines[i].trimStart().startsWith('-') ? 0 : 2)
    let end = i + 1
    while (end < lines.length) {
      const line = lines[end]
      if (line.trim() && !line.trimStart().startsWith('#') && line.search(/\S/) <= stepIndent) break
      end++
    }
    blocks.push(lines.slice(i, end).join('\n'))
  }
  return blocks
}
const hasShortestRetention = block => /^\s*retention-days:\s*1\s*(?:#.*)?$/m.test(block)
  && [...block.matchAll(/^\s*retention-days:/gm)].length === 1

test('每一个Actions上传步骤显式保留1天，拒绝遗漏或更长覆盖值', () => {
  const workflows = path.join(root, '.github/workflows')
  let count = 0
  for (const name of fs.readdirSync(workflows).filter(n => /\.ya?ml$/.test(n))) {
    for (const block of uploadBlocks(fs.readFileSync(path.join(workflows, name), 'utf8'))) {
      assert(hasShortestRetention(block), name + ' 中制品保留期不是显式1天')
      count++
    }
  }
  assert(count > 0)
})

test('边界检查拒绝缺省、14/90天、动态表达式与重复配置', () => {
  for (const value of ['14', '90', '0', '${{ inputs.days }}', '']) {
    assert.equal(hasShortestRetention('          retention-days: ' + value), false)
  }
  assert.equal(hasShortestRetention('retention-days: 1\nretention-days: 14'), false)
  assert.equal(hasShortestRetention('          retention-days: 1'), true)
})
