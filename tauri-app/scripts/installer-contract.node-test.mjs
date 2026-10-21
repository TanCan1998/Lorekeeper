import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { assertWixVersion } from './release-helpers.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const nativeRoot = path.join(root, 'tauri-app/src-tauri')
const read = file => fs.readFileSync(path.join(nativeRoot, file), 'utf8')
const config = JSON.parse(read('tauri.conf.json'))
const hook = read('installer/hooks.nsh')
const fragment = read('installer/lorekeeper-install-record.wxs')
const wixTemplate = read('installer/main.wxs')
const launcher = read('identity-launcher/src/win.rs')
const launcherMain = read('identity-launcher/src/main.rs')
const launcherRecord = read('identity-launcher/src/record.rs')
const identityBuilder = read('../scripts/build-windows-identity.mjs')

test('中文双安装包保留应用标识、安装范围及既有MSI升级身份', () => {
  assert.equal(config.productName, 'LoreKeeper')
  assert.equal(config.identifier, 'com.tancan1998.lorekeeper')
  assert.equal(config.bundle.windows.wix.language, 'zh-CN')
  assert.equal(config.bundle.windows.wix.upgradeCode, 'ed3e3199-4ddc-501a-a745-179a185e8dfb')
  // MSI 只比较前三段，不能仅递增第四段；规则在 release-helpers 里统一校验。
  assertWixVersion(config.version, config.bundle.windows.wix.version)
  const nsis = config.bundle.windows.nsis
  assert.deepEqual(nsis.languages, ['SimpChinese'])
  assert.equal(nsis.installMode, 'currentUser')
  assert.equal(nsis.installerHooks, 'installer/hooks.nsh')
  assert.equal(nsis.customLanguageFiles.SimpChinese, 'installer/SimpChinese.nsh')
})

test('MSI桌面、开始菜单和安装结束启动只经过身份激活器', () => {
  assert.equal(config.bundle.windows.wix.template, 'installer/main.wxs')
  for (const id of ['ApplicationDesktopShortcut', 'ApplicationStartMenuShortcut']) {
    const start = wixTemplate.indexOf(`<Shortcut Id="${id}"`)
    assert(start >= 0, `缺少 ${id}`)
    const shortcut = wixTemplate.slice(start, wixTemplate.indexOf('>', start) + 1)
    assert(shortcut.includes('Target="[INSTALLDIR]lorekeeper-activate.exe"'))
    assert(!shortcut.includes('[!Path]'))
    assert(shortcut.includes('Icon="ProductIcon"'))
  }
  const action = wixTemplate.match(/<CustomAction Id="LaunchApplication"[^>]+\/>/)?.[0]
  assert(action?.includes('Directory="INSTALLDIR"'))
  assert(action?.includes('ExeCommand="&quot;[INSTALLDIR]lorekeeper-activate.exe&quot;"'))
  assert(!action?.includes('FileKey="Path"'))
  assert(!action?.includes('[LAUNCHAPPARGS]'))
  assert(wixTemplate.includes('<File Id="Path" Source="{{main_binary_path}}"'))
  assert(wixTemplate.includes('{{resources}}'))
  assert(wixTemplate.includes('{{#each component_refs'))
  assert(wixTemplate.includes('AUTOLAUNCHAPP AND NOT Installed'))
  // macOS 不需要 Windows 身份小包：平台配置只构建前端，主配置继续供 Windows 使用。
  const macosConfig = JSON.parse(read('tauri.macos.conf.json'))
  assert.equal(macosConfig.build.beforeBuildCommand, 'pnpm build')
  assert(!macosConfig.build.beforeBuildCommand.includes('build-windows-identity'))
  assert(config.build.beforeBuildCommand.includes('build-windows-identity'))
  assert(!fs.existsSync(path.join(nativeRoot, 'tauri.linux.conf.json')), 'Linux 未在发布矩阵内，不新增平台配置')
  const windowsConfig = JSON.parse(read('tauri.windows.conf.json'))
  assert.equal(windowsConfig.bundle.resources['target/identity/stage/lorekeeper-identity-launcher.exe'], 'lorekeeper-activate.exe')
})

test('卸载默认保留；接管原勾选框而不按AppData目录递归删除', () => {
  assert.match(read('installer/SimpChinese.nsh'), /清理所选项目（默认保留，须二次确认）/)
  assert(hook.indexOf('StrCpy $R8 $DeleteAppDataCheckboxState') < hook.indexOf('StrCpy $DeleteAppDataCheckboxState 0'))
  assert(hook.indexOf('StrCpy $DeleteAppDataCheckboxState 0') < hook.indexOf('ExecWait'))
  for (const gate of ['$R8 = 1', '$UpdateMode <> 1', '$PassiveMode <> 1', '${Silent}', '"_?="']) assert(hook.includes(gate))
  assert.equal((hook.match(/ExecWait/g) || []).length, 1)
  assert.match(hook, /--cleanup-project/)
  assert.doesNotMatch(hook, /RmDir|WebView2.*Delete|HKLM|Software\\Classes/)
})

test('注册表清理仅限本应用键，共享厂商键只在空时移除', () => {
  assert(hook.includes('DeleteRegKey SHCTX "${MANUPRODUCTKEY}"'))
  assert(hook.includes('DeleteRegKey /ifempty SHCTX "${MANUKEY}"'))
  assert(hook.includes('DeleteRegKey /ifempty HKCU "${MANUKEY}"'))
  assert.doesNotMatch(hook, /DeleteRegKey (?:SHCTX|HKCU) "\$\{MANUKEY\}"/)
})

test('清理入口不接收目录参数、不注册IPC、不加载业务数据', () => {
  const source = read('src/project_cleanup.rs')
  assert.doesNotMatch(source, /#\[tauri::command\]|recent_projects|ensure_lorekeeper_home|AppState/)
  assert(source.includes('context.config_mut().app.windows.clear()'))
  assert(source.includes('blocking_pick_folder'))
  assert.equal((source.match(/if !confirm\(/g) || []).length, 2)
  assert(source.includes('self.revalidate(home, protected)?'))
  assert(source.includes('same_object(&self.identity, &latest.identity)'))
})

test('安装记录在MSI写全机、NSIS写当前用户，键名与启动器单一来源', () => {
  // 与 Tauri 生成的模板一致：Manufacturer=tancan1998，Directory Id=INSTALLDIR。
  assert(launcher.includes('r"Software\\tancan1998\\LoreKeeper"'))
  assert(fragment.includes('Key="Software\\tancan1998\\LoreKeeper"'))
  assert(hook.includes('"${MANUPRODUCTKEY}"'))
  for (const name of ['IdentityRecordSchema', 'IdentityPackageName', 'InstallScope', 'InstallLocation']) {
    assert(fragment.includes(`Name="${name}"`), `片段缺少 ${name}`)
    assert(hook.includes(`"${name}"`), `NSIS 缺少 ${name}`)
  }
  assert(fragment.includes('Root="HKLM"'))
  assert(fragment.includes('Value="machine"'))
  assert(hook.includes('"user"'))
  assert(fragment.includes('DirectoryRef Id="INSTALLDIR"'))
  assert(fragment.includes('ForceDeleteOnUninstall="yes"'))
  assert(fragment.includes('Win64="yes"'))
  // 记录只追加命名值，不搬运文件、不建链接、不改模板默认值。
  assert.doesNotMatch(fragment, /junction|symboliclink|CopyFile/i)
  assert.doesNotMatch(hook, /CopyFiles|SetOutPath|CreateDirectory/)
  // 快捷方式改指激活器：只允许这一种 CreateShortCut，且目标必须是激活器。
  const shortcuts = [...hook.matchAll(/CreateShortcut "([^"]+)" "([^"]+)"/g)]
  assert(shortcuts.length > 0)
  for (const [, , target] of shortcuts) assert.match(target, /\$INSTDIR\\lorekeeper-activate\.exe$/)
})

test('MSI在安装后注册身份、卸载前注销，且只在对应阶段触发', () => {
  assert(fragment.includes('SetRegisterIdentityCmd'))
  assert(fragment.includes('--register-identity'))
  assert(fragment.includes('--unregister-identity'))
  assert(fragment.includes('WixQuietExec'))
  assert(fragment.includes('BinaryKey="WixCA"'))
  // 安装/升级/修复刷新；只有非升级的完整卸载注销，不能把修复当卸载。
  const registerCondition = 'NOT (REMOVE~="ALL")'
  const unregisterCondition = 'Installed AND (REMOVE~="ALL") AND NOT UPGRADINGPRODUCTCODE'
  for (const [action, position, condition] of [
    ['SetRegisterIdentityCmd', 'Before="RegisterIdentity"', registerCondition],
    ['RegisterIdentity', 'After="InstallFinalize"', registerCondition],
    ['SetUnregisterIdentityCmd', 'Before="UnregisterIdentity"', unregisterCondition],
    ['UnregisterIdentity', 'Before="RemoveFiles"', unregisterCondition],
  ]) {
    assert(fragment.includes(`<Custom Action="${action}" ${position}>${condition}</Custom>`), action)
  }
  assert(fragment.includes('--register-identity --quiet'))
  assert(fragment.includes('--unregister-identity --quiet'))
  // 记录键随组件卸载删除，不残留全机键。
  assert(fragment.includes('ForceDeleteOnUninstall="yes"'))
})

test('WiX片段已接入MSI构建，且不进入主crate依赖图', () => {
  const wix = config.bundle.windows.wix
  assert.deepEqual(wix.fragmentPaths, ['installer/lorekeeper-install-record.wxs'])
  assert.deepEqual(wix.componentRefs, ['LoreKeeperInstallRecord'])
  assertWixVersion(config.version, wix.version)
  // 启动器是独立 crate：不改主应用依赖、不参与 Tauri 打包主体。
  assert.doesNotMatch(read('Cargo.toml'), /identity-launcher/)
  assert(fs.existsSync(path.join(nativeRoot, 'identity-launcher/Cargo.toml')))
  assert(fs.existsSync(path.join(nativeRoot, 'identity-launcher/src/main.rs')))
})

test('启动器只在真实包身份下启动主程序，无身份时只激活身份包', () => {
  assert(launcher.includes('GetCurrentPackageFullName'))
  assert(launcher.includes('PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY'))
  assert(launcher.includes('POLICY_DISABLE_PROCESS_TREE: u32 = 2'))
  // 未签名包不允许可执行激活，因此只登记并转交 WindowsApps 内的包启动器路径。
  assert(launcherRecord.includes('WindowsApps'))
  assert(launcher.includes('lorekeeper-identity-launcher.exe'))
  assert(!launcher.includes('shell:AppsFolder'))
  assert(launcher.includes('EXTENDED_STARTUPINFO_PRESENT'))
  // 提权注册助手：置备全机 → 当前用户注册 → 回写包内启动器路径。
  assert(launcherMain.includes('--register-identity'))
  assert(launcher.includes('Add-AppxProvisionedPackage'))
  assert(launcher.includes('IdentityLauncherPath'))
  assert(launcher.includes('TokenElevation'))
  // 反向约束：不关闭沙箱、不加浏览器诊断参数、不按命令行任意路径启动。
  assert.doesNotMatch(launcher, /--edge-webview|no-sandbox|disable-gpu|CREATE_NEW_CONSOLE/)
  for (const key of ['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS', 'RegistryWrite', 'CreateSymbolicLink']) {
    assert(!launcher.includes(key), `启动器不应包含 ${key}`)
  }
})

test('身份版本从同一构建计划写入启动器和清单，旧包仍存在也必须刷新', () => {
  assert(identityBuilder.includes('LOREKEEPER_IDENTITY_VERSION: plan.identity.version'))
  assert(identityBuilder.includes('createWindowsIdentityPlan(identityOptions)'))
  assert(identityBuilder.includes('renderUnsignedIdentityManifest(identityOptions)'))
  assert(launcher.includes('option_env!("LOREKEEPER_IDENTITY_VERSION")'))
  assert(launcher.includes('package_matches_version(&full_name, expected_version)'))
  assert(launcher.includes('ready_packaged_launcher(&record, expected_version)'))
  assert(launcher.includes('ready_packaged_launcher(&refreshed_record, expected_version)'))
  assert(launcher.includes("$_.Version -eq [version]'{expected_version}'"))
  // 注册成功且版本匹配后才允许写回；不能把任意 Get-AppxPackage 结果当成新版本。
  assert(launcher.indexOf("$_.Version -eq [version]'{expected_version}'")
    < launcher.indexOf("Set-ItemProperty -LiteralPath {key_path}"))
})

test('静默标记覆盖提权助手和包内启动器，自选目录按字面量传递', () => {
  assert(launcherMain.includes('const QUIET: &str = "--quiet"'))
  assert(launcher.includes("-ArgumentList '--register-identity','--quiet'"))
  assert(launcher.includes('command.arg(crate::QUIET)'))
  assert(launcher.includes('quote_powershell_literal(self_exe)'))
  assert(launcher.includes('let msix = quote_powershell_literal(&msix)'))
  assert(launcher.includes('Test-Path -LiteralPath $launcher -PathType Leaf'))
  assert(launcher.includes('output.status.code() == Some(1223)'))
})

test('macOS保持拖拽默认保留，提供中文说明及宿主显示名', () => {
  assert.equal(config.bundle.macOS.bundleName, 'LoreKeeper')
  assert.match(read('installer/Info.plist'), /CFBundleDisplayName[\s\S]*LoreKeeper/)
  const guide = read('installer/安装与卸载说明.txt')
  assert.match(guide, /默认保留/)
  assert.match(guide, /--cleanup-project/)
})

const compiler = process.platform === 'win32'
  ? path.join(process.env.LOCALAPPDATA || '', 'tauri/NSIS/makensis.exe') : ''
test('真实NSIS编译器验证挂钩语法（仅编译，不执行安装卸载）', {
  skip: !compiler || !fs.existsSync(compiler),
}, () => {
  const dir = fs.mkdtempSync(path.join(root, '.pi/tmp/installer-syntax-'))
  const source = path.join(dir, 'syntax.nsi')
  fs.writeFileSync(source, [
    'Unicode true',
    '!include LogicLib.nsh',
    '!include FileFunc.nsh',
    '!define MAINBINARYNAME "lorekeeper"',
    '!define PRODUCTNAME "LoreKeeper"',
    '!define MANUPRODUCTKEY "Software\\LoreKeeperTest\\LoreKeeper"',
    '!define MANUKEY "Software\\LoreKeeperTest"',
    'Var DeleteAppDataCheckboxState',
    'Var UpdateMode',
    'Var PassiveMode',
    '!macro CheckIfAppIsRunning binary name',
    '!macroend',
    `!include "${path.join(nativeRoot, 'installer/hooks.nsh')}"`,
    `OutFile "${path.join(dir, 'syntax-only.exe')}"`,
    'Section',
    'WriteUninstaller "$TEMP\\lorekeeper-syntax-only-uninstaller.exe"',
    'SectionEnd',
    'Section Uninstall',
    '!insertmacro NSIS_HOOK_PREUNINSTALL',
    '!insertmacro NSIS_HOOK_POSTUNINSTALL',
    'SectionEnd',
  ].join('\n'), 'utf8')
  const result = spawnSync(compiler, ['/V2', source], { encoding: 'utf8' })
  assert.equal(result.status, 0, (result.stdout || '') + (result.stderr || ''))
  assert(fs.statSync(path.join(dir, 'syntax-only.exe')).size > 0)
})
