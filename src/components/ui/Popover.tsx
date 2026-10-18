import { useEffect, useId, type ReactNode } from 'react'
import { Button, type InkButtonVariant } from './Button'
import { useAnchoredLayer } from './use-anchored-layer'

interface PopoverProps {
  /** 触发按钮文字。 */
  label: string
  /** 弹层标题，同时是弹层的可访问名称。 */
  title: string
  triggerVariant?: Exclude<InkButtonVariant, 'icon' | 'danger-solid'>
  align?: 'start' | 'end'
  children: ReactNode
}

/** 锚定在触发按钮下方的非模态弹层，打开时焦点进入弹层，Esc 关闭并回到触发按钮。 */
export function Popover({ label, title, triggerVariant = 'secondary', align = 'start', children }: PopoverProps) {
  const { open, layerId, anchorRef, triggerRef, layerRef, renderLayer, toggle } = useAnchoredLayer(align)
  const titleId = useId()

  useEffect(() => {
    if (open) layerRef.current?.focus()
  }, [open, layerRef])

  return (
    <div className="pi-anchor" ref={anchorRef}>
      <Button
        ref={triggerRef}
        variant={triggerVariant}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? layerId : undefined}
        onClick={toggle}
      >
        {label}
      </Button>
      {open && renderLayer(
        <div
          id={layerId}
          ref={layerRef}
          role="dialog"
          aria-labelledby={titleId}
          tabIndex={-1}
          className="pi-popover"
        >
          <h3 id={titleId} className="pi-popover-title">{title}</h3>
          {children}
        </div>
      )}
    </div>
  )
}
