import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ipc } from '../ipc-client'
import { installTauriInternals, type TauriInternalsHandle } from '../../../test/helpers/tauri-internals'

let tauri: TauriInternalsHandle
beforeEach(() => { tauri = installTauriInternals() })
afterEach(() => { vi.restoreAllMocks(); tauri.uninstall() })
const flush = async () => { await new Promise<void>(resolve => setTimeout(resolve, 0)) }

describe('Tauri 生成事件订阅生命周期', () => {
  it('重复取消只撤销一次底层订阅，取消后的已排队事件不再转发', async () => {
    const received = vi.fn()
    const cancel = ipc.on('generation:reasoning', received)
    await flush()
    const callback = tauri.listenerCallbacks('generation:reasoning')[0]
    const event = { event: 'generation:reasoning', id: 1, payload: { text: '合成推理事件' } }
    callback(event)
    expect(received).toHaveBeenCalledTimes(1)
    cancel()
    cancel()
    callback(event)
    await flush()
    expect(received).toHaveBeenCalledTimes(1)
    expect(tauri.invoke.mock.calls.filter(([command]) => command === 'plugin:event|unlisten')).toHaveLength(1)
  })

  it('异步注册未返回前取消立即封闭事件入口，返回后释放订阅', async () => {
    const received = vi.fn()
    const original = tauri.invoke.getMockImplementation() as (command: string, args: Record<string, unknown>) => Promise<unknown>
    let release!: () => void
    const pending = new Promise<void>(resolve => { release = resolve })
    tauri.invoke.mockImplementation(async (command: string, args: Record<string, unknown>) => {
      const result = await original(command, args)
      if (command === 'plugin:event|listen') await pending
      return result
    })
    const cancel = ipc.on('generation:reasoning', received)
    cancel()
    tauri.emit('generation:reasoning', { text: '合成迟到事件' })
    expect(received).not.toHaveBeenCalled()
    release()
    await flush()
    expect(tauri.listeners.get('generation:reasoning')).toEqual([])
    expect(tauri.invoke.mock.calls.filter(([command]) => command === 'plugin:event|unlisten')).toHaveLength(1)
  })

  it('注册失败可观测且不会产生未处理拒绝，取消仍安全', async () => {
    const error = new Error('合成事件注册失败')
    tauri.invoke.mockRejectedValue(error)
    const log = vi.spyOn(console, 'error').mockImplementation(() => {})
    const cancel = ipc.on('generation:reasoning', vi.fn())
    cancel()
    await flush()
    expect(log).toHaveBeenCalledWith('[ipc-client.on] 事件监听注册失败:', 'generation:reasoning', error)
    expect(() => cancel()).not.toThrow()
  })
})
