import { readFileSync, readdirSync } from 'node:fs'
import path from 'node:path'
import ts from 'typescript'
import { describe, expect, it } from 'vitest'
import { MIGRATED_CHANNELS } from '../../shared/migrated-channels'

describe('已注册频道的参数映射完整性', () => {
  it('逐个检查全部合同，而非仅依赖Rust注册数量', () => {
    const declared = new Set<string>()
    const required = new Set<string>()
    const mapped = new Set<string>()
    // 沿用现有门禁事实源，不把刻意关闭的审稿写面误当遗漏而自动开放。
    const gateSource=readFileSync(path.resolve(process.cwd(),'test/channel-migration-coverage.test.ts'),'utf8')
    const gate=gateSource.match(/const DEFERRED_RENDERER_CHANNELS = \[([\s\S]*?)\] as const/u)
    expect(gate).not.toBeNull()
    const deferred=new Set(Array.from(gate![1].matchAll(/'([^']+)'/gu),match=>match[1]))
    const shared = path.resolve(process.cwd(), 'src/shared')
    for (const file of readdirSync(shared).filter(name => name.endsWith('.ts'))) {
      const source = ts.createSourceFile(file, readFileSync(path.join(shared, file), 'utf8'), ts.ScriptTarget.Latest, true)
      const visit = (node: ts.Node): void => {
        if (ts.isPropertySignature(node) && ts.isStringLiteral(node.name)
          && MIGRATED_CHANNELS.has(node.name.text) && node.type && ts.isTypeLiteralNode(node.type)) {
          const args = node.type.members.find(member => member.name?.getText(source) === 'args')
          if (args && ts.isPropertySignature(args) && args.type && ts.isTupleTypeNode(args.type)) {
            declared.add(node.name.text)
            if (args.type.elements.length) required.add(node.name.text)
          }
        }
        ts.forEachChild(node, visit)
      }
      visit(source)
    }
    const client = path.resolve(process.cwd(), 'src/services/ipc-client.ts')
    const source = ts.createSourceFile(client, readFileSync(client, 'utf8'), ts.ScriptTarget.Latest, true)
    function visit(node: ts.Node): void {
      if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'CHANNEL_ARG_NAMES'
        && node.initializer && ts.isObjectLiteralExpression(node.initializer)) {
        for (const property of node.initializer.properties) {
          if (ts.isPropertyAssignment(property) && ts.isStringLiteral(property.name)) {
            mapped.add(property.name.text)
          }
        }
      }
      ts.forEachChild(node, visit)
    }
    visit(source)
    // 若合同改为别名或继承，必须同步扩展扫描器，不能悄悄跳过该频道。
    expect([...declared].sort()).toEqual([...MIGRATED_CHANNELS].sort())
    expect([...mapped].sort()).toEqual([...required].filter(channel=>!deferred.has(channel)).sort())
  })
})
