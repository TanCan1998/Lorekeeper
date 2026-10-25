import assert from 'node:assert/strict'
import test from 'node:test'
import { composeIconFrames, readIcoFrames, writeIco } from './compose-icon-frames.mjs'

const PNG = Buffer.from('89504e470d0a1a0a0000000d49484452', 'hex')

function fakeIco(sizes) {
  return writeIco(sizes.map(size => ({ size, png: Buffer.concat([PNG, Buffer.from(`-${size}`)] ) })))
}

test('解析与写回保持尺寸集合一致', () => {
  const frames = readIcoFrames(fakeIco([16, 24, 32, 48, 64, 256]))
  assert.deepEqual([...frames.keys()].sort((a, b) => a - b), [16, 24, 32, 48, 64, 256])
  assert.equal(frames.get(256).toString().endsWith('-256'), true, '256px 帧在目录里宽度字节写 0，解析后仍应还原为 256')
})

test('组合时小尺寸取简化稿、其余取完整稿', () => {
  const small = fakeIco([16, 24, 32, 48, 64, 256])
  const full = fakeIco([16, 24, 32, 48, 64, 256])
  const composed = readIcoFrames(composeIconFrames(small, full))
  for (const size of [16, 24, 32]) {
    assert.equal(composed.get(size).toString().endsWith(`-${size}`), true)
    assert.deepEqual([...composed.get(size)], [...readIcoFrames(small).get(size)], `${size}px 应逐字节来自简化稿`)
  }
  for (const size of [48, 64, 256]) {
    assert.deepEqual([...composed.get(size)], [...readIcoFrames(full).get(size)], `${size}px 应逐字节来自完整稿`)
  }
})

test('两套源图尺寸不同时取并集，缺帧回落另一套', () => {
  const small = fakeIco([16, 32])
  const full = fakeIco([16, 32, 48, 256])
  const composed = readIcoFrames(composeIconFrames(small, full))
  assert.deepEqual([...composed.keys()].sort((a, b) => a - b), [16, 32, 48, 256])
})

test('拒绝非 PNG 帧与越界帧，避免悄悄写出坏图标', () => {
  const bmpLike = Buffer.alloc(6 + 16 + 8)
  bmpLike.writeUInt16LE(1, 2)
  bmpLike.writeUInt16LE(1, 4)
  bmpLike.writeUInt8(16, 6)
  bmpLike.writeUInt8(16, 7)
  bmpLike.writeUInt32LE(8, 14)
  bmpLike.writeUInt32LE(22, 18)
  assert.throws(() => readIcoFrames(bmpLike), /不是 PNG 编码/)

  const truncated = fakeIco([16]).subarray(0, 30)
  assert.throws(() => readIcoFrames(truncated), /越界|不完整/)
})

test('没有帧时明确失败', () => {
  assert.throws(() => writeIco([]), /至少要有一帧/)
})
