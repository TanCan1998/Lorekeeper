/**
 * 连接测试失败时给作者看的说明：供应商返回非 2xx 时写清状态码与服务返回，
 * 其余情况沿用原始错误（对齐上游 1.3.2 的 `httpFailure` 文案，按界面语言分句）。
 */
export function modelTestFailureMessage(
  failure: { error?: string; httpFailure?: { status: number; body: string } },
  text: (zhCN: string, enUS: string) => string,
): string {
  const httpFailure = failure.httpFailure
  if (httpFailure) {
    const advice = text(
      `模型请求失败（HTTP ${httpFailure.status}）。请检查 Base URL、API Key 与模型名。`,
      `Model request failed (HTTP ${httpFailure.status}). Check the Base URL, API key, and model name.`,
    )
    const detail = httpFailure.body.trim()
    return detail
      ? `${advice}${text(`服务返回：${detail}`, ` The service returned: ${detail}`)}`
      : advice
  }
  return text(`连接失败：${failure.error}`, `Connection failed: ${failure.error}`)
}
