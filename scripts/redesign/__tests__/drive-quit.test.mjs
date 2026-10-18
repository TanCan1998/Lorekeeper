import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { it, vi } from 'vitest'
vi.mock('playwright', () => ({ _electron: { launch: () => { throw new Error('UNIT_TEST_MUST_NOT_LAUNCH_ELECTRON') } } }))
import { quit } from '../drive.mjs'

const fakeApp = (child, close) => ({ process: () => child, close })

it('close 失败但进程已自行退出时不再强杀，也不把「not found」当成失败', async () => {
  const child = spawn(process.execPath, ['-e', ''], { stdio: 'ignore' })
  await once(child, 'exit')
  const result = await quit(fakeApp(child, () => Promise.reject(new Error('Target page, context or browser has been closed'))))
  assert.deepEqual(result, { forced: false })
})

it('宽限期内未关闭且进程仍在时强杀整棵进程树', async () => {
  const child = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'ignore' })
  const exited = once(child, 'exit')
  const result = await quit(fakeApp(child, () => new Promise(() => {})), 50)
  assert.deepEqual(result, { forced: true })
  await exited
})
