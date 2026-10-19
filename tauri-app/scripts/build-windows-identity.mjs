// 构建 Windows 身份小包：启动器二进制 + 清单 + 图标 → 真实 MSIX。
// 只做本地构建，不注册包、不读证书、不执行安装或发布。
import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { createWindowsIdentityPlan, renderUnsignedIdentityManifest } from './windows-identity.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))
const nativeRoot = path.resolve(here, '../src-tauri')
const read = file => JSON.parse(fs.readFileSync(path.join(nativeRoot, file), 'utf8'))
const plan = createWindowsIdentityPlan({
  tauriConfig: read('tauri.conf.json'),
  identityConfig: read('installer/windows-identity.json'),
  architecture: process.env.TAURI_ENV_ARCH === 'aarch64' || process.env.IDENTITY_ARCH === 'arm64' ? 'arm64' : 'x64',
})
const outRoot = path.join(nativeRoot, 'target/identity')
const stage = path.join(outRoot, 'stage')
const launcherCrate = path.join(nativeRoot, 'identity-launcher')

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: 'utf8', timeout: 600000, ...options })
  if (result.status !== 0) {
    throw new Error(`${command} 失败（${result.status}）：\n${result.stdout || ''}\n${result.stderr || ''}`)
  }
  return result.stdout || ''
}

function copyFile(from, to) {
  fs.mkdirSync(path.dirname(to), { recursive: true })
  fs.copyFileSync(from, to)
}

// 图标映射固定，避免把任意文件塞进包内容。
const icons = [
  ['icons/128x128.png', 'Assets/storelogo.png'],
  ['icons/128x128.png', 'Assets/Square150x150Logo.png'],
  ['icons/32x32.png', 'Assets/Square44x44Logo.png'],
]

if (process.argv.includes('--check')) {
  console.log(JSON.stringify({ stage, manifest: plan.identity, icons }, null, 2))
  process.exit(0)
}

// 1. 构建启动器（独立 crate，不进入主应用依赖图）。
run('cargo', ['build', '--release', '--manifest-path', path.join(launcherCrate, 'Cargo.toml')])
const launcherExe = path.join(launcherCrate, 'target/release/lorekeeper-identity-launcher.exe')
if (!fs.existsSync(launcherExe)) throw new Error('缺少启动器构建产物')

// 2. 组装包内容：只放启动器与图标，不放主程序或用户路径。
fs.rmSync(stage, { recursive: true, force: true })
copyFile(launcherExe, path.join(stage, plan.layout.launcherExecutable))
for (const [from, to] of icons) copyFile(path.join(nativeRoot, from), path.join(stage, to))
fs.writeFileSync(path.join(stage, 'AppxManifest.xml'),
  renderUnsignedIdentityManifest({ tauriConfig: read('tauri.conf.json'), identityConfig: read('installer/windows-identity.json') }),
  'utf8')

// 3. 打包：MakeAppx 由 Windows SDK 提供，路径可显式覆盖，不做隐式搜索下载。
const makeAppx = process.env.MAKEAPPX || 'C:/Program Files (x86)/Windows Kits/10/bin/10.0.26100.0/x64/makeappx.exe'
if (!fs.existsSync(makeAppx)) throw new Error(`找不到 MakeAppx：${makeAppx}`)
// 文件名保持稳定：版本在清单内，升级按清单版本比较，不靠文件名。
const msix = path.join(outRoot, 'LoreKeeper.msix')
fs.rmSync(msix, { force: true })
run(makeAppx, ['pack', '/o', '/d', stage, '/p', msix]).trim()

const stat = fs.statSync(msix)
const launcherStat = fs.statSync(path.join(stage, plan.layout.launcherExecutable))
console.log(JSON.stringify({
  msix, msixBytes: stat.size, stage,
  launcher: plan.layout.launcherExecutable, launcherBytes: launcherStat.size,
  identity: plan.identity, signing: plan.signing,
}, null, 2))
