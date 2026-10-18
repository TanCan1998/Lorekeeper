import type { ReactNode } from 'react'
import { Check as CheckIcon } from 'lucide-react'

interface CheckProps {
  checked: boolean
  onChange: (checked: boolean) => void
  label: ReactNode
  /** 复选框用于勾选列表，开关用于立即生效的设置。 */
  kind?: 'checkbox' | 'switch'
  disabled?: boolean
}

/** 基于原生 checkbox，键盘与读屏行为由浏览器提供；开关形态加 role="switch"。 */
export function Check({ checked, onChange, label, kind = 'checkbox', disabled }: CheckProps) {
  return (
    <label className="pi-check">
      <input
        type="checkbox"
        role={kind === 'switch' ? 'switch' : undefined}
        className="pi-check-input"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
      />
      {kind === 'switch'
        ? <span className="pi-switch" aria-hidden />
        : <span className="pi-check-box" aria-hidden><CheckIcon strokeWidth={3} /></span>}
      <span className="pi-check-label">{label}</span>
    </label>
  )
}
