import type { ReactNode } from 'react'

export interface StatItem {
  id: string
  label: string
  value: ReactNode
  hint?: string
}

interface StatGridProps {
  /** 统计格整体的可访问名称。 */
  label: string
  items: readonly StatItem[]
}

/** 统计格：数字用 Instrument Serif 斜体，名称在下。 */
export function StatGrid({ label, items }: StatGridProps) {
  return (
    <dl className="pi-stats" aria-label={label}>
      {items.map((item) => (
        <div key={item.id} className="pi-stat">
          <dt className="pi-stat-label">{item.label}</dt>
          <dd className="pi-stat-value">{item.value}</dd>
          {item.hint && <dd className="pi-stat-hint">{item.hint}</dd>}
        </div>
      ))}
    </dl>
  )
}
