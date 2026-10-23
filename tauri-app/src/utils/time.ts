/**
 * 格式化相对时间（如：刚刚 / 5分钟前 / 2小时前 / 3天前）
 */
export function formatRelativeTime(timestamp: number, locale: 'zh-CN' | 'en-US' = 'zh-CN'): string {
  const now = Date.now()
  const diff = now - timestamp
  const minutes = Math.floor(diff / 60000)
  const hours = Math.floor(diff / 3600000)
  const days = Math.floor(diff / 86400000)

  if (minutes < 1) return locale === 'en-US' ? 'just now' : '刚刚'
  if (minutes < 60) return locale === 'en-US' ? `${minutes}m ago` : `${minutes}分钟前`
  if (hours < 24) return locale === 'en-US' ? `${hours}h ago` : `${hours}小时前`
  if (days < 7) return locale === 'en-US' ? `${days}d ago` : `${days}天前`
  return new Date(timestamp).toLocaleDateString(locale, { month: 'short', day: 'numeric' })
}
