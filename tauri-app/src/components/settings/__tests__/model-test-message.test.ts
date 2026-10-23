import { describe, expect, it } from 'vitest'

import { modelTestFailureMessage } from '../model-test-message'

const zh = (_zhCN: string) => _zhCN
const en = (_zhCN: string, enUS: string) => enUS

describe('modelTestFailureMessage', () => {
  it('有供应商状态码时写清 HTTP 状态与建议，不再暴露中文前缀', () => {
    const failure = { error: 'API 调用失败 (401): {"error":"invalid api key"}', httpFailure: { status: 401, body: '{"error":"invalid api key"}' } }
    expect(modelTestFailureMessage(failure, zh))
      .toBe('模型请求失败（HTTP 401）。请检查 Base URL、API Key 与模型名。服务返回：{"error":"invalid api key"}')
    expect(modelTestFailureMessage(failure, en))
      .toBe('Model request failed (HTTP 401). Check the Base URL, API key, and model name. The service returned: {"error":"invalid api key"}')
  })

  it('响应正文为空时只给建议，不留悬空的「服务返回」', () => {
    const failure = { error: 'API 调用失败 (502): ', httpFailure: { status: 502, body: '   ' } }
    expect(modelTestFailureMessage(failure, zh))
      .toBe('模型请求失败（HTTP 502）。请检查 Base URL、API Key 与模型名。')
    expect(modelTestFailureMessage(failure, en)).not.toContain('The service returned')
  })

  it('没有结构化状态时沿用原始错误（网络失败等）', () => {
    const failure = { error: 'error sending request for url (https://api.example.com)' }
    expect(modelTestFailureMessage(failure, zh)).toBe('连接失败：error sending request for url (https://api.example.com)')
    expect(modelTestFailureMessage(failure, en)).toBe('Connection failed: error sending request for url (https://api.example.com)')
  })
})
