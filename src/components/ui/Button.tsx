/* eslint-disable react-refresh/only-export-components */
import * as React from 'react'
import { Slot } from '@radix-ui/react-slot'
import { Check } from 'lucide-react'
import { cva, type VariantProps } from 'class-variance-authority'
import { clsx } from 'clsx'
import { cn } from '../../lib/utils'
import { useAppearanceStore } from '../../stores/appearance-bootstrap'

 

export const buttonVariants = cva(
  /* 基础：加入顺滑过渡和点击缩小回弹效果，提升交互手感 */
  'inline-flex items-center justify-center gap-1.5 whitespace-nowrap text-xs font-medium transition-all duration-200 ease-out active:scale-[0.96] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-offset-1 focus-visible:ring-offset-[var(--color-bg)] focus-visible:ring-[var(--color-accent)] disabled:pointer-events-none disabled:opacity-50 cursor-pointer',
  {
    variants: {
      variant: {
        default:
          'btn-primary text-white',
        ai:
          'ai-glow text-white shadow-sm hover:shadow-md relative overflow-hidden',
        destructive:
          'bg-[var(--color-error)] text-[var(--color-error-foreground)] shadow-sm hover:shadow-md hover:shadow-[var(--color-error)]/30 transition-all duration-200 hover:brightness-110 active:scale-[0.96]',
        outline:
          'border border-[var(--color-border)] bg-transparent text-[var(--color-text)] hover:bg-[var(--color-hover)] hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-all duration-200',
        ghost:
          'text-[var(--color-text-secondary)] hover:bg-[var(--color-hover)] hover:text-[var(--color-text)] active:scale-[0.98] transition-all duration-200',
        success:
          'bg-[var(--color-success)] text-[var(--color-success-foreground)] shadow-sm hover:shadow-md hover:shadow-[var(--color-success)]/30 transition-all duration-200 hover:brightness-110 active:scale-[0.96]',
      },
      size: {
        default: 'h-7 px-3 py-1 rounded-[var(--radius-md)]',    /* 28px 高 */
        sm:      'h-6 px-2.5 py-0.5 rounded-[var(--radius-sm)]', /* 24px 高 */
        lg:      'h-8 px-4 py-1.5 rounded-[var(--radius-lg)]',   /* 32px 高 */
        icon:    'h-7 w-7 p-0 rounded-[var(--radius-md)]',       /* 28x28 方形图标按钮 */
      },
    },
    defaultVariants: {
      variant: 'default',
      size: 'default',
    },
  }
)

type NativeButtonProps = React.ButtonHTMLAttributes<HTMLButtonElement>

/** 旧外壳的按钮变体，RD-10 删除旧外壳时一并删除，届时 Button 只剩纸墨六种。 */
type LegacyButtonProps = NativeButtonProps & VariantProps<typeof buttonVariants> & {
  asChild?: boolean
}

/** 纸墨按钮：主、次、文字（幽灵）、危险描边、危险实心与图标。 */
export type InkButtonVariant = 'primary' | 'secondary' | 'text' | 'danger' | 'danger-solid' | 'icon'
export type InkButtonSize = 'sm' | 'md' | 'lg'

type InkButtonProps = NativeButtonProps & { size?: InkButtonSize } & (
  | { variant: Exclude<InkButtonVariant, 'icon'> }
  | { variant: 'icon'; 'aria-label': string }
)

export type ButtonProps = LegacyButtonProps | InkButtonProps

const inkVariants: ReadonlySet<string> = new Set<InkButtonVariant>(['primary', 'secondary', 'text', 'danger', 'danger-solid', 'icon'])

function isInkButton(props: ButtonProps): props is InkButtonProps {
  return typeof props.variant === 'string' && inkVariants.has(props.variant)
}

type LegacyVariant = NonNullable<LegacyButtonProps['variant']>
type LegacySize = NonNullable<LegacyButtonProps['size']>

/**
 * 新外壳里仍有旧对话框与旧面板（导入、导出、项目备份、模型表单、架构编辑等）用旧变体。
 * 旧 default/ai 读旧强调色，弹窗里与页面里的主按钮颜色不一样；新外壳下一律换成纸墨六种，旧外壳不变。
 * 旧面板拿 default 标切换按钮的选中项（带 aria-pressed 或 aria-selected），那不是这一层的主操作：一律换成次按钮，
 * 选中时强调色边线加淡底；aria-pressed 的选中项前面再加对勾，不只靠颜色区分（标签页不加）。
 */
const LEGACY_INK_VARIANT: Record<LegacyVariant, Exclude<InkButtonVariant, 'icon'>> = {
  default: 'primary', ai: 'primary', success: 'primary', outline: 'secondary', ghost: 'text', destructive: 'danger-solid',
}
/** 旧尺寸 28/24/32px 换成纸墨最接近的一档。 */
const LEGACY_INK_SIZE: Record<LegacySize, InkButtonSize> = { default: 'sm', sm: 'sm', lg: 'md', icon: 'sm' }

function inkClasses(variant: InkButtonVariant, size: InkButtonSize, className: string | undefined) {
  return clsx('pi-btn', `pi-btn--${variant}`, size !== 'md' && `pi-btn--${size}`, className)
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>((props, ref) => {
  const stage = useAppearanceStore(s => s.resolvedShell === 'stage')
  if (isInkButton(props)) {
    const { className, variant, size = 'md', type = 'button', ...rest } = props
    return <button ref={ref} type={type} className={inkClasses(variant, size, className)} {...rest} />
  }
  const { className, variant, size, asChild = false, children, ...rest } = props
  const Comp = asChild ? Slot : 'button'
  if (stage) {
    const toggle = rest['aria-pressed'] !== undefined || rest['aria-selected'] !== undefined
    const inkVariant = size === 'icon' ? 'icon' : toggle ? 'secondary' : LEGACY_INK_VARIANT[variant ?? 'default']
    const checked = !asChild && (rest['aria-pressed'] === true || rest['aria-pressed'] === 'true')
    return (
      <Comp ref={ref} className={inkClasses(inkVariant, LEGACY_INK_SIZE[size ?? 'default'], className)} {...rest}>
        {checked ? <><Check aria-hidden />{children}</> : children}
      </Comp>
    )
  }
  return (
    <Comp
      className={cn(buttonVariants({ variant, size, className }))}
      ref={ref}
      {...rest}
    >
      {children}
    </Comp>
  )
})
Button.displayName = 'Button'

export { Button }
