import { clsx } from 'clsx'

interface ProgressRingProps {
  size: number
  stroke: number
  /** 0 到 1。 */
  ratio: number
  className?: string
}

/** 进度环：底轨加按比例填充的弧，从十二点钟方向顺时针增长；半径取 size / 2 - stroke 与画板一致。是状态指示，不是图标。 */
export function ProgressRing({ size, stroke, ratio, className }: ProgressRingProps) {
  const center = size / 2
  const radius = size / 2 - stroke
  const circumference = 2 * Math.PI * radius
  const filled = Math.min(1, Math.max(0, ratio)) * circumference
  return (
    <svg className={clsx('stage-ring', className)} width={size} height={size} viewBox={`0 0 ${size} ${size}`} aria-hidden="true">
      <circle className="stage-ring-track" cx={center} cy={center} r={radius} strokeWidth={stroke} />
      {filled > 0 && (
        <circle
          className="stage-ring-fill"
          cx={center}
          cy={center}
          r={radius}
          strokeWidth={stroke}
          strokeDasharray={`${filled} ${circumference}`}
          transform={`rotate(-90 ${center} ${center})`}
        />
      )}
    </svg>
  )
}
