import * as React from 'react'
import * as DialogPrimitive from '@radix-ui/react-dialog'
import { X } from 'lucide-react'
import { cn } from '../../lib/utils'
import { useLocaleStore } from '../../stores/locale-store'
import { useAppearanceStore } from '../../stores/appearance-bootstrap'
import { Button } from './Button'
import { useInkOverlayHost } from './ink-overlay'
import { useDialogFocusReturn } from './focus-return'
import { keepToastStackClickable } from './ToastStack'

const Dialog = DialogPrimitive.Root
const DialogTrigger = DialogPrimitive.Trigger
const DialogPortal = DialogPrimitive.Portal
const DialogClose = DialogPrimitive.Close

/**
 * 新外壳下，旧对话框的调用方（导入、导出等）不改一行：内容 Portal 进 body 下的纸墨宿主，外壳换成 pi-dialog，
 * 头、标题、说明、底栏换成纸墨原语；内容区的旧 --color-* 由 stage-legacy-bridge.css 映射到纸墨令牌。
 */
const InkDialogContext = React.createContext(false)
const useInkDialog = () => React.useContext(InkDialogContext)

/** 遮罩层 - 增强的模糊效果 */
const DialogOverlay = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Overlay>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Overlay>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Overlay
    ref={ref}
    className={cn(
      'fixed inset-0 z-50 bg-black/30 backdrop-blur-sm',
      'data-[state=open]:animate-in data-[state=closed]:animate-out',
      'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
      className
    )}
    style={{
      backdropFilter: 'blur(12px)',
    }}
    {...props}
  />
))
DialogOverlay.displayName = DialogPrimitive.Overlay.displayName

/** 对话框主体 - 增强的阴影和动画 */
const DialogContent = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Content>
>(({ className, children, onPointerDownOutside, onOpenAutoFocus, onCloseAutoFocus, ...props }, ref) => {
  const text = useLocaleStore(s => s.text)
  const ink = useAppearanceStore(s => s.resolvedShell === 'stage')
  const focusReturn = useDialogFocusReturn()
  const host = useInkOverlayHost(ink)
  if (ink) {
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
          {...props}
        >
          <InkDialogContext.Provider value>{children}</InkDialogContext.Provider>
          <DialogPrimitive.Close asChild>
            <Button variant="icon" size="sm" className="pi-dialog-close" aria-label={text('关闭', 'Close')}>
              <X aria-hidden />
            </Button>
          </DialogPrimitive.Close>
        </DialogPrimitive.Content>
      </DialogPortal>
    )
  }
  return (
  <DialogPortal>
    <DialogOverlay />
    <DialogPrimitive.Content
      ref={ref}
      className={cn(
        'fixed left-[50%] top-[50%] z-50 w-full max-w-lg translate-x-[-50%] translate-y-[-50%]',
        'rounded-2xl outline-none',
        'bg-[var(--color-bg)] border border-[var(--color-border)]',
        'shadow-2xl shadow-black/20',
        'duration-300 ease-out data-[state=open]:animate-in data-[state=closed]:animate-out',
        'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
        'data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95',
        'data-[state=closed]:slide-out-to-left-1/2 data-[state=closed]:slide-out-to-top-[50%]',
        'data-[state=open]:slide-in-from-left-1/2 data-[state=open]:slide-in-from-top-[50%]',
        className
      )}
      style={{
        boxShadow: '0 25px 50px -12px rgba(0, 0, 0, 0.25), 0 0 0 1px rgba(255, 255, 255, 0.05)',
      }}
      onPointerDownOutside={onPointerDownOutside}
      onOpenAutoFocus={onOpenAutoFocus}
      onCloseAutoFocus={onCloseAutoFocus}
      {...props}
    >
      {children}
      <DialogPrimitive.Close
        className="absolute right-4 top-4 rounded-md opacity-60 hover:opacity-100 transition-all duration-200 hover:bg-[var(--color-hover)] p-1"
        style={{
          background: 'transparent',
          border: 'none',
          cursor: 'pointer',
          color: 'var(--color-text-muted)',
        }}
      >
        <X size={16} />
        <span className="sr-only">{text('关闭', 'Close')}</span>
      </DialogPrimitive.Close>
    </DialogPrimitive.Content>
  </DialogPortal>
  )
})
DialogContent.displayName = DialogPrimitive.Content.displayName

/** 对话框头部 */
const DialogHeader = ({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) => (
  <div
    className={useInkDialog() ? cn('pi-dialog-head', className) : cn(
      'px-6 py-4 border-b border-[var(--color-border)]',
      className
    )}
    {...props}
  />
)
DialogHeader.displayName = 'DialogHeader'

/** 对话框底部 */
const DialogFooter = ({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) => (
  <div
    className={useInkDialog() ? cn('pi-dialog-foot justify-between', className) : cn(
      'flex items-center justify-between px-6 py-4 border-t border-[var(--color-border)]',
      className
    )}
    {...props}
  />
)
DialogFooter.displayName = 'DialogFooter'

/** 对话框标题 */
const DialogTitle = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Title>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Title>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Title
    ref={ref}
    className={useInkDialog() ? cn('pi-dialog-title', className) : cn('text-base font-semibold text-[var(--color-text)]', className)}
    {...props}
  />
))
DialogTitle.displayName = DialogPrimitive.Title.displayName

/** 对话框描述 */
const DialogDescription = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Description>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Description>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Description
    ref={ref}
    className={useInkDialog() ? cn('pi-dialog-sub', className) : cn('text-sm text-[var(--color-text-muted)]', className)}
    {...props}
  />
))
DialogDescription.displayName = DialogPrimitive.Description.displayName

export {
  Dialog,
  DialogPortal,
  DialogOverlay,
  DialogClose,
  DialogTrigger,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
}
