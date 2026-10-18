import * as React from 'react'
import * as DialogPrimitive from '@radix-ui/react-dialog'
import { X } from 'lucide-react'
import { cn } from '../../lib/utils'
import { useLocaleStore } from '../../stores/locale-store'
import { Button } from './Button'
import { useInkOverlayHost } from './ink-overlay'
import { useDialogFocusReturn } from './focus-return'
import { keepToastStackClickable } from './ToastStack'

// 仅真正打开的受控弹窗进入栈，避免退出动画占住Radix最高层后ESC失效。
const dialogStack: symbol[] = []
const DialogCloseContext = React.createContext<{ open: boolean; close: () => void } | null>(null)
function Dialog({ open, onOpenChange, ...props }: React.ComponentProps<typeof DialogPrimitive.Root>) {
  const value = React.useMemo(() => ({
    open: open !== false,
    close: () => onOpenChange?.(false),
  }), [open, onOpenChange])
  return <DialogCloseContext.Provider value={value}>
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange} {...props} />
  </DialogCloseContext.Provider>
}
const DialogTrigger = DialogPrimitive.Trigger
const DialogPortal = DialogPrimitive.Portal
const DialogClose = DialogPrimitive.Close

/**
 * 沿用的旧对话框（导入、导出等）的外壳：内容 Portal 进 body 下的纸墨宿主，外壳是 pi-dialog，
 * 头、标题、说明、底栏是纸墨原语；内容区的旧 --color-* 由 stage-legacy-bridge.css 映射到纸墨令牌。
 */
const DialogContent = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Content>
>(({ className, children, onPointerDownOutside, onOpenAutoFocus, onCloseAutoFocus, onEscapeKeyDown, ...props }, ref) => {
  const text = useLocaleStore(s => s.text)
  const controlled = React.useContext(DialogCloseContext)
  const layer = React.useRef(Symbol('dialog-layer'))
  React.useEffect(() => {
    if (!controlled?.open) return
    const token = layer.current
    dialogStack.push(token)
    const escape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.isComposing || dialogStack.at(-1) !== token || event.defaultPrevented) return
      onEscapeKeyDown?.(event)
      if (event.defaultPrevented) return
      event.preventDefault()
      event.stopImmediatePropagation()
      controlled.close()
    }
    document.addEventListener('keydown', escape, true)
    return () => {
      document.removeEventListener('keydown', escape, true)
      const index = dialogStack.indexOf(token)
      if (index >= 0) dialogStack.splice(index, 1)
    }
  }, [controlled, onEscapeKeyDown])
  const focusReturn = useDialogFocusReturn()
  const host = useInkOverlayHost()
  if (!host) return null
  return (
    <DialogPortal container={host}>
      <DialogPrimitive.Overlay className="pi-scrim" />
      <DialogPrimitive.Content
        ref={ref}
        className={cn('pi-dialog pi-dialog--legacy max-w-lg', className)}
        onPointerDownOutside={event => {
          keepToastStackClickable(event)
          onPointerDownOutside?.(event)
        }}
        onOpenAutoFocus={event => {
          focusReturn.remember()
          onOpenAutoFocus?.(event)
        }}
        onCloseAutoFocus={event => {
          onCloseAutoFocus?.(event)
          if (!event.defaultPrevented) focusReturn.onCloseAutoFocus(event)
        }}
        onEscapeKeyDown={onEscapeKeyDown}
        {...props}
      >
        {children}
        <DialogPrimitive.Close asChild>
          <Button variant="icon" size="sm" className="pi-dialog-close" aria-label={text('关闭', 'Close')}>
            <X aria-hidden />
          </Button>
        </DialogPrimitive.Close>
      </DialogPrimitive.Content>
    </DialogPortal>
  )
})
DialogContent.displayName = DialogPrimitive.Content.displayName

/** 对话框头部 */
const DialogHeader = ({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) => (
  <div className={cn('pi-dialog-head', className)} {...props} />
)
DialogHeader.displayName = 'DialogHeader'

/** 对话框底部 */
const DialogFooter = ({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) => (
  <div className={cn('pi-dialog-foot justify-between', className)} {...props} />
)
DialogFooter.displayName = 'DialogFooter'

/** 对话框标题 */
const DialogTitle = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Title>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Title>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Title ref={ref} className={cn('pi-dialog-title', className)} {...props} />
))
DialogTitle.displayName = DialogPrimitive.Title.displayName

/** 对话框描述 */
const DialogDescription = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Description>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Description>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Description ref={ref} className={cn('pi-dialog-sub', className)} {...props} />
))
DialogDescription.displayName = DialogPrimitive.Description.displayName

export {
  Dialog,
  DialogPortal,
  DialogClose,
  DialogTrigger,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
}
