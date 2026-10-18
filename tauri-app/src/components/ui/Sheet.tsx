import { useEffect, useId, useRef, type KeyboardEvent, type ReactNode } from 'react'
import { X } from 'lucide-react'
import { clsx } from 'clsx'
import { Button } from './Button'

export interface SheetTab {
  id: string
  label: string
}

interface SheetProps {
  open: boolean
  title: string
  subtitle?: string
  closeLabel: string
  onClose: () => void
  /** 侧页分页；给出时 children 是当前分页的内容。segmented 是标题下一条分段胶囊，underline 是下划线页签。 */
  tabs?: { items: readonly SheetTab[]; active: string; onChange: (id: string) => void; style?: 'underline' | 'segmented' }
  footer?: ReactNode
  wide?: boolean
  children: ReactNode
}

const TAB_KEYS: Record<string, (index: number, count: number) => number> = {
  ArrowRight: (index, count) => (index + 1) % count,
  ArrowLeft: (index, count) => (index - 1 + count) % count,
  Home: () => 0,
  End: (_, count) => count - 1,
}

/**
 * 内容区右侧的按需侧页。由所在布局决定位置，侧页本身不遮挡正文。
 * 非模态：打开时焦点移到标题，Tab 可以离开侧页；点在侧页空白处时焦点落在侧页自身（tabIndex=-1），Esc 仍然有效。
 * 关闭时若焦点随侧页一起消失，就还给打开前的元素。
 */
export function Sheet({ open, title, subtitle, closeLabel, onClose, tabs, footer, wide, children }: SheetProps) {
  const titleId = useId()
  const panelId = useId()
  const titleRef = useRef<HTMLHeadingElement>(null)
  const openerRef = useRef<HTMLElement | null>(null)

  useEffect(() => {
    if (!open) {
      openerRef.current = null
      return
    }
    // StrictMode 会重跑 effect，那时焦点已在标题上；只记录本次打开前的元素。
    openerRef.current ??= document.activeElement as HTMLElement | null
    titleRef.current?.focus()
    return () => {
      if (document.activeElement === document.body || document.activeElement === null) openerRef.current?.focus()
    }
  }, [open])

  if (!open) return null

  const tabId = (id: string) => `${panelId}-tab-${id}`

  const onTabKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const move = TAB_KEYS[event.key]
    if (!tabs || !move) return
    event.preventDefault()
    const index = tabs.items.findIndex((tab) => tab.id === tabs.active)
    const next = tabs.items[move(index, tabs.items.length)]
    tabs.onChange(next.id)
    document.getElementById(tabId(next.id))?.focus()
  }

  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key === 'Escape') {
      event.stopPropagation()
      onClose()
    }
  }

  return (
    <aside className={clsx('pi-sheet', wide && 'pi-sheet--wide', tabs?.style === 'segmented' && 'pi-sheet--segmented')} aria-labelledby={titleId} tabIndex={-1} onKeyDown={onKeyDown}>
      <header className="pi-sheet-head">
        <div>
          <h2 id={titleId} ref={titleRef} tabIndex={-1} className="pi-sheet-title">{title}</h2>
          {subtitle && <p className="pi-sheet-sub">{subtitle}</p>}
        </div>
        <Button variant="icon" size="sm" aria-label={closeLabel} onClick={onClose}>
          <X size={16} aria-hidden />
        </Button>
      </header>
      {tabs && (
        <div className="pi-sheet-tabs" role="tablist" onKeyDown={onTabKeyDown}>
          {tabs.items.map((tab) => (
            <button
              key={tab.id}
              id={tabId(tab.id)}
              type="button"
              role="tab"
              className="pi-sheet-tab pi-focusable"
              aria-selected={tab.id === tabs.active}
              aria-controls={panelId}
              tabIndex={tab.id === tabs.active ? 0 : -1}
              onClick={() => tabs.onChange(tab.id)}
            >
              {tab.label}
            </button>
          ))}
        </div>
      )}
      <div
        className="pi-sheet-body"
        id={panelId}
        role={tabs ? 'tabpanel' : undefined}
        aria-labelledby={tabs ? tabId(tabs.active) : undefined}
      >
        {children}
      </div>
      {footer && <footer className="pi-sheet-foot">{footer}</footer>}
    </aside>
  )
}
