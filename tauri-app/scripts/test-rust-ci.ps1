#Requires -Version 7.0
# 仅用于CI的合成验证；本机日常继续使用定向测试，不在此启动GUI或业务网络请求。
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-fA-F]{40}$')]
    [string]$ExpectedSha,
    [Parameter(Mandatory = $true)]
    [string]$EvidenceDirectory,
    [string]$Toolchain = '1.99.0'
)

$ErrorActionPreference = 'Stop'
# PowerShell 7的原生命令失败由下面捕获的退出码处理，不能丢失后续检查与证据。
$PSNativeCommandUseErrorActionPreference = $false
$crateDirectory = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../src-tauri'))
$null = New-Item -ItemType Directory -Path $EvidenceDirectory -Force
$evidencePath = Join-Path $EvidenceDirectory 'validation.json'
$evidence = [ordered]@{
    schemaVersion = 1
    expectedSha = $ExpectedSha.ToLowerInvariant()
    testedSha = $null
    eventName = $env:GITHUB_EVENT_NAME
    pullRequestHeadSha = $env:PR_HEAD_SHA
    runId = $env:GITHUB_RUN_ID
    runAttempt = $env:GITHUB_RUN_ATTEMPT
    runnerOs = $env:RUNNER_OS
    runnerImage = $env:ImageVersion
    toolchain = $Toolchain
    startedAt = [DateTime]::UtcNow.ToString('o')
    finishedAt = $null
    testCommand = "rustup run $Toolchain cargo test --locked"
    checkCommand = "rustup run $Toolchain cargo check --locked --all-targets"
    testExitCode = $null
    checkExitCode = $null
    cargoLockSha256 = $null
    state = 'failed'
}

Push-Location $crateDirectory
try {
    $sha = (git rev-parse HEAD).Trim()
    $code = $LASTEXITCODE
    if ($code -ne 0) { throw '无法读取待验证提交SHA' }
    $evidence.testedSha = $sha
    if ($sha -ne $ExpectedSha) { throw '检出版本与expected_sha不一致，拒绝登记验收结果' }
    $dirty = @(git status --porcelain --untracked-files=normal)
    $code = $LASTEXITCODE
    if ($code -ne 0 -or $dirty.Count -ne 0) { throw '工作区不干净，不能登记为提交级验收' }
    $evidence.cargoLockSha256 = (Get-FileHash -Algorithm SHA256 'Cargo.lock').Hash.ToLowerInvariant()

    & rustup run $Toolchain cargo test --locked 2>&1 |
        Tee-Object -FilePath (Join-Path $EvidenceDirectory 'cargo-test.log')
    $evidence.testExitCode = $LASTEXITCODE

    # 测试失败仍执行编译检查，分别记录退出码，不能让后者掩盖前者。
    & rustup run $Toolchain cargo check --locked --all-targets 2>&1 |
        Tee-Object -FilePath (Join-Path $EvidenceDirectory 'cargo-check.log')
    $evidence.checkExitCode = $LASTEXITCODE
    if ($evidence.testExitCode -eq 0 -and $evidence.checkExitCode -eq 0) {
        $evidence.state = 'passed'
    }
} finally {
    Pop-Location
    $evidence.finishedAt = [DateTime]::UtcNow.ToString('o')
    [IO.File]::WriteAllText($evidencePath, ($evidence | ConvertTo-Json -Depth 5), [Text.UTF8Encoding]::new($false))
    if ($env:GITHUB_STEP_SUMMARY) {
        @(
            '## Tauri Rust验证'
            "- 提交：$($evidence.testedSha)"
            "- 状态：$($evidence.state)"
            "- cargo test退出码：$($evidence.testExitCode)"
            "- cargo check退出码：$($evidence.checkExitCode)"
            '- 仅合成测试，不代表GUI或真实模型/MCP/云业务验收。'
        ) | Out-File -FilePath $env:GITHUB_STEP_SUMMARY -Append -Encoding utf8
    }
}
if ($evidence.state -ne 'passed') { exit 1 }
exit 0
