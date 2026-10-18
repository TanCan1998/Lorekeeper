import React from 'react'

/** 旧外壳的空状态，RD-10 删除旧外壳时一并删除。 */
interface LegacyProps extends React.HTMLAttributes<HTMLDivElement> {
  icon?: React.ReactNode
  message: string
  opacity?: number
}

/** 纸墨空状态：衬线标题、一句说明、一个动作。 */
interface InkProps {
  heading: string
  description?: string
  action: React.ReactNode
  icon?: React.ReactNode
}

export function EmptyState(props: LegacyProps | InkProps) {
  if ('heading' in props) {
    return (
      <div className="pi-empty">
        {props.icon}
        <h3 className="pi-empty-title">{props.heading}</h3>
        {props.description && <p className="pi-empty-text">{props.description}</p>}
        {props.action}
      </div>
    )
  }
  const { icon, message, opacity = 0.3, className, style, children, ...rest } = props
  return (
    <div
      className={`flex flex-col items-center justify-center h-full gap-3 ${className || ''}`}
      {...rest}
      style={{ opacity, ...style }}
    >
      {icon}
      <span className="text-sm">{message}</span>
      {children}
    </div>
  )
}
