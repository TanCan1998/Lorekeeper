import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const nativeRoot = path.join(root, 'tauri-app/src-tauri')
const read = file => fs.readFileSync(path.join(nativeRoot, file), 'utf8')
const config = JSON.parse(read('tauri.conf.json'))
const hook = read('installer/hooks.nsh')
const fragment = read('installer/lorekeeper-install-record.wxs')
const launcher = read('identity-launcher/src/win.rs')

test('中文双安装包保留应用标识、安装范围及既有MSI升级身份', () => {
  assert.equal(config.productName, 'LoreKeeper')
  assert.equal(config.identifier, 'com.tancan1998.lorekeeper')
  assert.equal(config.bundle.windows.wix.language, 'zh-CN')
  assert.equal(config.bundle.windows.wix.upgradeCode, 'ed3e3199-4ddc-501a-a745-179a185e8dfb')
  // MSI 只比较前三段，不能仅递增第四段。
  assert.equal(config.bundle.windows.wix.version, '1.0.1.0')
  const nsis = config.bundle.windows.nsis
  assert.deepEqual(nsis.languages, ['SimpChinese'])
  assert.equal(nsis.installMode, 'currentUser')
  assert.equal(nsis.installerHooks, 'installer/hooks.nsh')
  assert.equal(nsis.customLanguageFiles.SimpChinese, 'installer/SimpChinese.nsh')
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
  assert(fragment.includes('Action="createAndRemoveOnUninstall"'))
  assert(fragment.includes('Win64="yes"'))
  // 记录只追加命名值，不搬运文件、不建链接、不改模板默认值。
  assert.doesNotMatch(fragment, /junction|symboliclink|CopyFile/i)
  assert.doesNotMatch(hook, /CopyFiles|SetOutPath|CreateDirectory/)
  // 快捷方式改指激活器：只允许这一种 CreateShortCut，且目标必须是激活器。
  const shortcuts = [...hook.matchAll(/CreateShortcut "([^"]+)" "([^"]+)"/g)]
  assert(shortcuts.length > 0)
  for (const [, , target] of shortcuts) assert.match(target, /\$INSTDIR\\lorekeeper-activate\.exe$/)
})

test('WiX片段已接入MSI构建，且不进入主crate依赖图', () => {
  const wix = config.bundle.windows.wix
  assert.deepEqual(wix.fragmentPaths, ['installer/lorekeeper-install-record.wxs'])
  assert.deepEqual(wix.componentRefs, ['LoreKeeperInstallRecord'])
  assert.equal(wix.version, '1.0.1.0')
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
  assert(launcher.includes('windowsapps'))
  assert(launcher.includes('lorekeeper-identity-launcher.exe'))
  assert(!launcher.includes('shell:AppsFolder'))
  assert(launcher.includes('EXTENDED_STARTUPINFO_PRESENT'))
  // 反向约束：不关闭沙箱、不加浏览器诊断参数、不按命令行任意路径启动。
  assert.doesNotMatch(launcher, /--edge-webview|no-sandbox|disable-gpu|CREATE_NEW_CONSOLE/)
  for (const key of ['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS', 'RegistryWrite', 'CreateSymbolicLink']) {
    assert(!launcher.includes(key), `启动器不应包含 ${key}`)
  }
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
