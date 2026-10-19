import assert from 'node:assert/strict'
import fs from 'node:fs'
import { test } from 'node:test'
import { createWindowsIdentityPlan, renderUnsignedIdentityManifest } from './windows-identity.mjs'

const config = JSON.parse(fs.readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'))
const identity = JSON.parse(fs.readFileSync(new URL('../src-tauri/installer/windows-identity.json', import.meta.url), 'utf8'))
const options = () => ({ tauriConfig: structuredClone(config), identityConfig: structuredClone(identity) })

test('身份沿用identifier和MSI数字版本，主程序仍由用户选目录', () => {
  const plan = createWindowsIdentityPlan(options())
  assert.equal(plan.identity.name, config.identifier)
  assert.equal(plan.identity.version, config.bundle.windows.wix.version)
  assert.equal(plan.layout.mainLocation, 'user-selected-install-directory')
  assert.equal(plan.layout.identityLocation, 'windows-managed')
  assert.equal(plan.layout.mainExecutable, 'lorekeeper.exe')
  assert.equal(plan.layout.embedSparseIdentityInMain, false)
  assert.equal(plan.layout.allowDirectoryJunction, false)
})

test('默认无签名，不导入证书信任；签名证书位置仅预留', () => {
  const plan = createWindowsIdentityPlan(options())
  assert.deepEqual(plan.signing, { mode: 'unsigned', allowUnsignedDeployment: true, importCertificateTrust: false })
  assert.match(plan.identity.publisher, /OID\.2\.25\.311729368913984317654407730594956997722=1$/)
  assert.deepEqual(identity.signing, {
    mode: 'unsigned', certificateFile: null, certificateThumbprint: null, publisher: null, timestampUrl: null,
  })
})

test('未签名注册需提权、不出现在应用列表，清单不声明可执行激活', () => {
  const plan = createWindowsIdentityPlan(options())
  assert.deepEqual(plan.registration, { requiresElevation: true, appListEntry: 'none', lastError: '0x80073D2B' })
  const xml = renderUnsignedIdentityManifest(options())
  // 未签名包不能出现可执行激活项（本机实测 0x80073D2B），入口改由安装器登记包内路径。
  assert.match(xml, /AppListEntry="none"/)
  assert(!xml.includes('shell:AppsFolder'))
})

test('只使用公开桌面进程策略，外部工具须另验脱离包环境', () => {
  const plan = createWindowsIdentityPlan(options())
  assert.equal(plan.processPolicy.attribute, 'PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY')
  assert.equal(plan.processPolicy.value, 2)
  assert.equal(plan.processPolicy.externalToolsRequireBreakaway, true)
  assert(!JSON.stringify(plan).includes('--edge-webview'))
})

test('MSI全机优先但注册不冒充所有用户已覆盖，项目默认保留', () => {
  const plan = createWindowsIdentityPlan(options())
  assert.deepEqual(plan.installation, {
    primaryScope: 'machine', perUserRegistrationRequired: true,
    systemRegistrationIsNotAllUsers: true, preserveProjectsByDefault: true,
  })
  assert.equal(plan.releaseEnabled, false)
  assert(plan.pendingAcceptance.includes('machine-stage-provision-and-per-user-registration'))
  assert(plan.pendingAcceptance.includes('real-lorekeeper-process-group'))
  assert(plan.pendingAcceptance.includes('external-tool-breakaway'))
})

test('清单只包含小启动器，不将外部主程序或自选路径塞入包内', () => {
  const xml = renderUnsignedIdentityManifest(options())
  assert.match(xml, /Executable="lorekeeper-identity-launcher\.exe"/)
  assert.match(xml, /RuntimeBehavior="packagedClassicApp"/)
  assert.match(xml, /TrustLevel="mediumIL"/)
  assert.match(xml, /Resource Language="zh-CN"/)
  assert.equal((xml.match(/rescap:Capability Name=/g) || []).length, 1)
  assert.match(xml, /Capability Name="runFullTrust"/)
  assert.doesNotMatch(xml, /AllowExternalContent|ExternalLocation|Executable="lorekeeper\.exe"|broadFileSystemAccess|unvirtualizedResources/)
  assert(!xml.includes('user-selected-install-directory'))
})

test('显式支持的架构与预览数字版本，拒绝非法或溢出版本', () => {
  const input = options()
  input.architecture = 'arm64'
  assert.match(renderUnsignedIdentityManifest(input), /ProcessorArchitecture="arm64"/)
  for (const version of ['1.0.0-beta.1', '1.0.0', '1.0.0.0.1', '1.0.0.65536', '01.0.0.0', '1.0.0.0"><x']) {
    const invalid = options()
    invalid.tauriConfig.bundle.windows.wix.version = version
    assert.throws(() => createWindowsIdentityPlan(invalid), /四段UInt16/)
  }
  input.architecture = 'x86'
  assert.throws(() => createWindowsIdentityPlan(input), /架构/)
})

test('改变产品身份或目录联接布局时拒绝生成', () => {
  const a = options()
  a.tauriConfig.identifier = 'com.other.app'
  assert.throws(() => createWindowsIdentityPlan(a), /identifier/)
  const b = options()
  b.tauriConfig.productName = 'Other'
  assert.throws(() => createWindowsIdentityPlan(b), /产品名/)
  const c = options()
  c.identityConfig.layout = 'junction'
  assert.throws(() => createWindowsIdentityPlan(c), /布局/)
})

test('签名尚未实现时明确失败，不能忽略证书、未知凭据或配置拼写错误', () => {
  const signed = options()
  signed.identityConfig.signing.mode = 'signed'
  assert.throws(() => createWindowsIdentityPlan(signed), /尚未接入/)
  for (const field of ['certificateFile', 'certificateThumbprint', 'publisher', 'timestampUrl']) {
    const input = options()
    input.identityConfig.signing[field] = 'reserved-test-value'
    assert.throws(() => createWindowsIdentityPlan(input), /必须为null/)
    delete input.identityConfig.signing[field]
    assert.throws(() => createWindowsIdentityPlan(input), /必须为null/)
  }
  for (const mutate of [
    x => { x.identityConfig.signing.password = 'synthetic-only' },
    x => { x.identityConfig.signign = {} },
    x => { x.identityConfig.schemaVersion = 2 },
    x => { x.identityConfig.signing = null },
    x => { x.identityConfig = [] },
  ]) {
    const input = options()
    mutate(input)
    assert.throws(() => createWindowsIdentityPlan(input))
  }
})

test('环境变量不能自动启用签名或改变发布者，不触发文件与证书访问', () => {
  const old = process.env.LOREKEEPER_WINDOWS_CERTIFICATE_FILE
  process.env.LOREKEEPER_WINDOWS_CERTIFICATE_FILE = 'Z:\\不存在\\reserved.pfx'
  try {
    const input = options()
    const before = JSON.stringify(input)
    const plan = createWindowsIdentityPlan(input)
    assert.equal(plan.signing.mode, 'unsigned')
    assert.equal(JSON.stringify(input), before)
    assert.match(renderUnsignedIdentityManifest(input), /CN=LoreKeeper, OID/)
  } finally {
    if (old === undefined) delete process.env.LOREKEEPER_WINDOWS_CERTIFICATE_FILE
    else process.env.LOREKEEPER_WINDOWS_CERTIFICATE_FILE = old
  }
})
