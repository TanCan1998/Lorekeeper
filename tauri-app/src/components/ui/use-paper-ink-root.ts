import { useLayoutEffect, useState, type RefObject } from 'react'

/**
 * 浮层（菜单、弹层、危险确认）Portal 的目标：离挂载点最近的 .paper-ink 根。
 * 挂到 body 会脱离纸墨令牌与深色作用域；留在原处会被滚动容器裁切，
 * 也会被带 transform、filter、contain 的祖先改掉 fixed 定位与层叠。
 * 返回 undefined 表示尚未挂载，null 表示不在纸墨作用域内。
 */
export function usePaperInkRoot(mountRef: RefObject<Element | null>): Element | null | undefined {
  const [root, setRoot] = useState<Element | null | undefined>(undefined)
  useLayoutEffect(() => {
    setRoot(mountRef.current?.closest('.paper-ink') ?? null)
  }, [mountRef])
  return root
}
