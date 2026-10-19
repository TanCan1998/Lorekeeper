// 仅生成身份小包的构建契约；不注册包、不读取证书、不执行安装或发布。
const identifier = 'com.tancan1998.lorekeeper'
const unsignedPublisher = 'CN=LoreKeeper, OID.2.25.311729368913984317654407730594956997722=1'
const certificateFields = ['certificateFile', 'certificateThumbprint', 'publisher', 'timestampUrl']

function keys(value, allowed, label) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${label}必须是对象`)
  for (const key of Object.keys(value)) {
    if (!allowed.includes(key)) throw new Error(`${label}包含未知字段：${key}`)
  }
}

function packageVersion(value) {
  if (typeof value !== 'string' || !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(value)
    || value.split('.').some(part => Number(part) > 65535)) {
    throw new Error('身份包必须使用四段UInt16数字版本，不能直接使用预览版SemVer')
  }
  return value
}

export function createWindowsIdentityPlan({ tauriConfig, identityConfig, architecture = 'x64' }) {
  if (tauriConfig?.identifier !== identifier || tauriConfig?.productName !== 'LoreKeeper') {
    throw new Error('不得通过身份小包改变应用identifier或产品名')
  }
  keys(identityConfig, ['schemaVersion', 'layout', 'signing'], '身份配置')
  if (identityConfig.schemaVersion !== 1 || identityConfig.layout !== 'external-main') {
    throw new Error('只支持版本1的外部主程序布局，不使用WindowsApps目录联接')
  }
  keys(identityConfig.signing, ['mode', ...certificateFields], '签名配置')
  if (identityConfig.signing.mode !== 'unsigned') {
    throw new Error('签名模式仅预留接口，尚未接入，不能静默降级为未签名')
  }
  for (const field of certificateFields) {
    if (identityConfig.signing[field] !== null) {
      throw new Error(`未签名模式的${field}必须为null，不读取证书或忽略已填配置`)
    }
  }
  if (!['x64', 'arm64'].includes(architecture)) throw new Error('不支持的Windows身份包架构')

  return {
    schemaVersion: 1,
    identity: {
      name: identifier,
      applicationId: 'LoreKeeper',
      publisher: unsignedPublisher,
      version: packageVersion(tauriConfig.bundle?.windows?.wix?.version),
      architecture,
      minimumWindowsVersion: '10.0.19041.0',
    },
    layout: {
      mainExecutable: 'lorekeeper.exe',
      mainLocation: 'user-selected-install-directory',
      launcherExecutable: 'lorekeeper-identity-launcher.exe',
      identityLocation: 'windows-managed',
      allowDirectoryJunction: false,
      embedSparseIdentityInMain: false,
    },
    processPolicy: {
      attribute: 'PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY',
      value: 2,
      name: 'PROCESS_CREATION_DESKTOP_APP_BREAKAWAY_DISABLE_PROCESS_TREE',
      externalToolsRequireBreakaway: true,
    },
    signing: { mode: 'unsigned', allowUnsignedDeployment: true, importCertificateTrust: false },
    installation: {
      primaryScope: 'machine',
      perUserRegistrationRequired: true,
      systemRegistrationIsNotAllUsers: true,
      preserveProjectsByDefault: true,
    },
    // 不能把清单生成成功当作产品归组或安装器验收成功。
    releaseEnabled: false,
    pendingAcceptance: [
      'native-launcher-and-install-record',
      'real-lorekeeper-process-group',
      'machine-stage-provision-and-per-user-registration',
      'upgrade-repair-rollback-uninstall',
      'external-tool-breakaway',
      'project-permissions-and-data-lifecycle',
      'supported-windows-versions',
    ],
  }
}

export function renderUnsignedIdentityManifest(options) {
  const plan = createWindowsIdentityPlan(options)
  const { name, publisher, version, architecture, applicationId, minimumWindowsVersion } = plan.identity
  // 字段来自固定常量或严格数字/架构枚举，不拼接外部目录、命令行或凭据。
  return `<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
 xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
 xmlns:uap10="http://schemas.microsoft.com/appx/manifest/uap/windows10/10"
 xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
 IgnorableNamespaces="uap uap10 rescap">
 <Identity Name="${name}" Publisher="${publisher}" Version="${version}" ProcessorArchitecture="${architecture}"/>
 <Properties><DisplayName>LoreKeeper</DisplayName><PublisherDisplayName>LoreKeeper</PublisherDisplayName><Logo>Assets\\storelogo.png</Logo></Properties>
 <Resources><Resource Language="zh-CN"/></Resources>
 <Dependencies><TargetDeviceFamily Name="Windows.Desktop" MinVersion="${minimumWindowsVersion}" MaxVersionTested="10.0.26100.0"/></Dependencies>
 <Capabilities><rescap:Capability Name="runFullTrust"/></Capabilities>
 <Applications><Application Id="${applicationId}" Executable="${plan.layout.launcherExecutable}"
  uap10:TrustLevel="mediumIL" EntryPoint="Windows.FullTrustApplication" uap10:RuntimeBehavior="packagedClassicApp">
  <uap:VisualElements DisplayName="LoreKeeper" Description="LoreKeeper" BackgroundColor="transparent"
   Square150x150Logo="Assets\\Square150x150Logo.png" Square44x44Logo="Assets\\Square44x44Logo.png"/>
 </Application></Applications>
</Package>
`
}
