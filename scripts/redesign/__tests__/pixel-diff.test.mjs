import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { crc32, deflateSync } from 'node:zlib'
import { afterEach, it } from 'vitest'

const CLI = fileURLToPath(new URL('../pixel-diff.mjs', import.meta.url))
const directories = []
afterEach(() => { for (const directory of directories.splice(0)) fs.rmSync(directory, { recursive: true, force: true }) })

function chunk(type, data) {
  const length = Buffer.alloc(4)
  length.writeUInt32BE(data.length)
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data])
  const crc = Buffer.alloc(4)
  crc.writeUInt32BE(crc32(body))
  return Buffer.concat([length, body, crc])
}

const paeth = (left, up, upLeft) => {
  const p = left + up - upLeft
  const [pa, pb, pc] = [Math.abs(p - left), Math.abs(p - up), Math.abs(p - upLeft)]
  return pa <= pb && pa <= pc ? left : pb <= pc ? up : upLeft
}

/** rgba 为逐像素 [r,g,b,a]；第 y 行用 filters[y] 编码，覆盖解码器的五种行过滤。 */
function encodePng({ width, height, rgba, channels = 4, filters = [] }) {
  const stride = width * channels
  const raw = Buffer.alloc(height * (stride + 1))
  let previous = Buffer.alloc(stride)
  for (let y = 0; y < height; y++) {
    const line = Buffer.alloc(stride)
    for (let x = 0; x < width; x++) for (let c = 0; c < channels; c++) line[x * channels + c] = rgba[y * width + x][c]
    const filter = filters[y] ?? 0
    raw[y * (stride + 1)] = filter
    for (let x = 0; x < stride; x++) {
      const left = x >= channels ? line[x - channels] : 0
      const up = previous[x]
      const upLeft = x >= channels ? previous[x - channels] : 0
      const predictor = [0, left, up, (left + up) >> 1, paeth(left, up, upLeft)][filter]
      raw[y * (stride + 1) + 1 + x] = (line[x] - predictor) & 0xff
    }
    previous = line
  }
  const header = Buffer.alloc(13)
  header.writeUInt32BE(width, 0)
  header.writeUInt32BE(height, 4)
  header[8] = 8
  header[9] = channels === 4 ? 6 : 2
  return Buffer.concat([Buffer.from('89504e470d0a1a0a', 'hex'), chunk('IHDR', header),
    chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0))])
}

function gradient(width, height) {
  return Array.from({ length: width * height }, (_, index) => [(index * 37) % 256, (index * 91 + 13) % 256, (index * 53 + 200) % 256, 255])
}

/** 取值只有 0～3，相邻像素频繁相等，Paeth 预测会走到平局分支。 */
function noise(width, height) {
  let seed = 7
  const next = () => (seed = (seed * 1103515245 + 12345) % 2147483648) >> 16 & 3
  return Array.from({ length: width * height }, () => [next(), next(), next(), 255])
}

function writeImages(...images) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'pixel-diff-'))
  directories.push(directory)
  return images.map((image, index) => {
    const file = path.join(directory, `${index}.png`)
    fs.writeFileSync(file, encodePng(image))
    return file
  })
}

function runCli(files) {
  const child = spawnSync(process.execPath, [CLI, ...files], { encoding: 'utf8' })
  return { status: child.status, stderr: child.stderr, report: child.stdout ? JSON.parse(child.stdout) : null }
}

const base = { width: 4, height: 4, rgba: gradient(4, 4) }
const nudged = (amount) => ({ ...base, rgba: base.rgba.map((pixel, index) => index === 5 ? [pixel[0], pixel[1] + amount, pixel[2], pixel[3]] : pixel) })

it('同一图像无论用哪种行过滤或 RGB/RGBA 编码，逐字节都相同', () => {
  const image = { width: 8, height: 10, rgba: noise(8, 10) }
  const { status, report } = runCli(writeImages(image, { ...image, filters: [1, 2, 3, 4, 4, 3, 2, 1, 4, 4] },
    { ...image, channels: 3, filters: [4, 4, 3, 3, 2, 2, 1, 1, 4, 3] }))
  assert.equal(status, 0)
  assert.deepEqual(report.ratios.map(item => [item.ratio, item.strictRatio]), [[0, 0], [0, 0]])
  assert.equal(report.pass, true)
})

it('通道差 2 级不计入 ratio，但计入 strictRatio', () => {
  const { status, report } = runCli(writeImages(base, nudged(2)))
  assert.equal(status, 0)
  assert.equal(report.ratios[0].ratio, 0)
  assert.equal(report.ratios[0].strictRatio, 1 / 16)
})

it('通道差 3 级计入 ratio，超过 0.5% 时退出码 1', () => {
  const { status, report } = runCli(writeImages(base, nudged(3)))
  assert.equal(status, 1)
  assert.equal(report.ratios[0].ratio, 1 / 16)
  assert.equal(report.maxRatio, 1 / 16)
  assert.equal(report.pass, false)
})

it('尺寸不同输出不同结果与原因，退出码 1，不抛堆栈', () => {
  const { status, stderr, report } = runCli(writeImages(base, { width: 4, height: 3, rgba: gradient(4, 3) }))
  assert.equal(status, 1)
  assert.equal(stderr, '')
  assert.equal(report.ratios[0].ratio, 1)
  assert.match(report.ratios[0].reason, /尺寸不同：4×4 对 4×3/)
  assert.equal(report.pass, false)
})
