import { describe, expect, it } from 'vitest'

import { StructuredContractDiagnostic, structuredContractDiagnosticText } from '../structured-contract-diagnostic'

describe('structuredContractDiagnosticText', () => {
  it('根路径不把 "$" 露给作者，说成「整体内容」（#474）', () => {
    const root = new StructuredContractDiagnostic('invalid_json', '$')
    expect(structuredContractDiagnosticText('en-US', root))
      .toBe('The content does not match the expected format: the whole content is not complete JSON.')
    expect(structuredContractDiagnosticText('zh-CN', root))
      .toBe('内容格式不对：整体内容不是完整的 JSON。')
  })

  it('字段路径原样写进说明，双语各用各自句式', () => {
    const field = new StructuredContractDiagnostic('missing_field', 'blueprints[0].characters')
    expect(structuredContractDiagnosticText('en-US', field))
      .toBe('The content does not match the expected format: blueprints[0].characters is missing.')
    expect(structuredContractDiagnosticText('zh-CN', field))
      .toBe('内容格式不对：blueprints[0].characters 缺少这个字段。')
  })

  it('关系类诊断按各自码给出可读原因', () => {
    const selfReference = new StructuredContractDiagnostic('relationship_self_reference', 'characters[1].relationships[0]')
    expect(structuredContractDiagnosticText('zh-CN', selfReference))
      .toBe('内容格式不对：characters[1].relationships[0] 关系指向了角色自己。')
    const endpoint = new StructuredContractDiagnostic('relationship_endpoint_not_in_characters', 'characters[0].relationships[2]')
    expect(structuredContractDiagnosticText('en-US', endpoint))
      .toBe('The content does not match the expected format: characters[0].relationships[2] has a relationship with someone who is not among the chapter\'s characters.')
  })
})
