import { afterEach, expect, it } from 'vitest'
import { ipc } from '../ipc-client'

const originalZoom = document.documentElement.style.zoom
afterEach(() => { document.documentElement.style.zoom = originalZoom })

it('整页缩放真实改变布局尺寸，不仅改变设置数字', () => {
  const probe = document.createElement('div')
  probe.style.cssText = 'width:100px;height:20px;position:absolute'
  document.body.append(probe)
  try {
    for (const factor of [0.7, 1, 1.2, 1.5]) {
      ipc.setZoomFactor(factor)
      expect(probe.getBoundingClientRect().width).toBeCloseTo(100 * factor, 0)
      expect(ipc.getZoomLevel()).toBeCloseTo(Math.log(factor) / Math.log(1.2))
    }
  } finally { probe.remove() }
})

it('损坏与越界比例不能修改当前缩放，指数级别按既定边界钳制', () => {
  ipc.setZoomFactor(1.2)
  for (const factor of [NaN, Infinity, -1, 0, 0.6, 1.6]) ipc.setZoomFactor(factor)
  expect(document.documentElement.style.zoom).toBe('1.2')
  ipc.setZoomLevel(100)
  expect(document.documentElement.style.zoom).toBe('1.5')
  ipc.setZoomLevel(-100)
  expect(document.documentElement.style.zoom).toBe('0.7')
})
