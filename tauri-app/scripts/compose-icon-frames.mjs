// 组合 Windows 图标：小尺寸（16/24/32）用简化稿，其余尺寸用完整稿。
//
// 背景：`tauri icon` 只能从一张源图生成全部帧，16–32px 帧是整图等比缩小，
// 「司」与四色星芒在极小尺寸会糊成一片。这里把两套源图各自生成的 ICO 按尺寸
// 拼成一份，Windows 按需挑帧时就自然用上简化版。
//
// 用法（在 tauri-app/ 下）：
//   node scripts/compose-icon-frames.mjs --small <small.ico> --full <full.ico> --out <icon.ico>
import fs from 'node:fs'

const ICO_HEADER_BYTES = 6
const ICO_ENTRY_BYTES = 16

/** 解析 ICO，返回 { size: pngBytes }：只接受 PNG 编码帧（与 tauri icon 产物一致）。 */
export function readIcoFrames(bytes) {
  if (bytes.length < ICO_HEADER_BYTES) throw new Error('ICO 头部不完整')
  const reserved = bytes.readUInt16LE(0)
  const type = bytes.readUInt16LE(2)
  const count = bytes.readUInt16LE(4)
  if (reserved !== 0 || type !== 1) throw new Error('不是有效的 ICO 文件')
  if (count === 0) throw new Error('ICO 没有任何帧')
  const frames = new Map()
  for (let index = 0; index < count; index += 1) {
    const entry = ICO_HEADER_BYTES + index * ICO_ENTRY_BYTES
    const width = bytes[entry] || 256
    const height = bytes[entry + 1] || 256
    if (width !== height) throw new Error(`第 ${index} 帧不是正方形：${width}x${height}`)
    const length = bytes.readUInt32LE(entry + 8)
    const start = bytes.readUInt32LE(entry + 12)
    if (length === 0 || start + length > bytes.length) throw new Error(`第 ${index} 帧越界`)
    const data = bytes.subarray(start, start + length)
    if (data.subarray(0, 8).toString('hex') !== '89504e470d0a1a0a') {
      throw new Error(`第 ${index} 帧不是 PNG 编码，无法按尺寸重组`)
    }
    frames.set(width, data)
  }
  return frames
}

/**
 * 按尺寸映射组合 ICO。
 * @param {Array<{ size: number, png: Uint8Array }>} frames 期望的帧序列（顺序即写入顺序）
 */
export function writeIco(frames) {
  if (!frames.length) throw new Error('至少要有一帧')
  const header = Buffer.alloc(ICO_HEADER_BYTES)
  header.writeUInt16LE(0, 0)
  header.writeUInt16LE(1, 2)
  header.writeUInt16LE(frames.length, 4)
  const entries = Buffer.alloc(ICO_ENTRY_BYTES * frames.length)
  let offset = ICO_HEADER_BYTES + entries.length
  frames.forEach((frame, index) => {
    const at = index * ICO_ENTRY_BYTES
    entries[at] = frame.size >= 256 ? 0 : frame.size
    entries[at + 1] = frame.size >= 256 ? 0 : frame.size
    entries[at + 2] = 0
    entries[at + 3] = 0
    entries.writeUInt16LE(1, at + 4)
    entries.writeUInt16LE(32, at + 6)
    entries.writeUInt32LE(frame.png.length, at + 8)
    entries.writeUInt32LE(offset, at + 12)
    offset += frame.png.length
  })
  return Buffer.concat([header, entries, ...frames.map(frame => Buffer.from(frame.png))])
}

/** 小尺寸用简化稿、其余用完整稿的组合结果。 */
export function composeIconFrames(smallIco, fullIco, smallSizes = [16, 24, 32]) {
  const small = readIcoFrames(smallIco)
  const full = readIcoFrames(fullIco)
  const sizes = [...new Set([...small.keys(), ...full.keys()])].sort((a, b) => a - b)
  return writeIco(sizes.map(size => {
    const useSmall = smallSizes.includes(size)
    const source = useSmall ? small : full
    const png = source.get(size) ?? (useSmall ? full.get(size) : small.get(size))
    if (!png) throw new Error(`两套源图都没有 ${size}px 帧`)
    return { size, png }
  }))
}

function readArgument(flag) {
  const index = process.argv.indexOf(flag)
  if (index < 0) throw new Error(`缺少参数 ${flag}`)
  const value = process.argv[index + 1]
  if (!value) throw new Error(`${flag} 没有取值`)
  return value
}

if (process.argv[1] && process.argv[1].endsWith('compose-icon-frames.mjs')) {
  const small = fs.readFileSync(readArgument('--small'))
  const full = fs.readFileSync(readArgument('--full'))
  const out = readArgument('--out')
  const result = composeIconFrames(small, full)
  fs.writeFileSync(out, result)
  console.log(`已写出 ${out}（${result.length} 字节，16/24/32 来自简化稿）`)
}
