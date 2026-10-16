#Requires -Version 7.0
# 无网络、无Rust编译的脚本合同测试：子进程中的git/rustup均为合成替身。
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$entryPoint = Join-Path $PSScriptRoot 'test-rust-ci.ps1'
$root = Join-Path ([IO.Path]::GetTempPath()) ("lorekeeper-ci-contract-" + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $root -Force
$hostExecutable = (Get-Process -Id $PID).Path
$harness = Join-Path $root 'harness.ps1'
@'
param([string]$EntryPoint, [string]$Evidence, [string]$Expected)
$ErrorActionPreference = 'Stop'
$env:GITHUB_STEP_SUMMARY = $null
function global:git {
    $global:LASTEXITCODE = 0
    if ($args[0] -eq 'rev-parse') { return ('0' * 40) }
    if ($env:CI_CONTRACT_DIRTY -eq 'true') { Write-Output ' M synthetic.rs' }
    if ($env:CI_CONTRACT_GIT_FAIL -eq 'true') { $global:LASTEXITCODE = 128 }
}
function global:rustup {
    $operation = $args[3]
    Add-Content -Path (Join-Path $env:CI_CONTRACT_CALLS 'calls.log') -Value $operation
    Write-Output "合成命令：cargo $operation"
    if ($operation -eq 'test') { $global:LASTEXITCODE = [int]$env:CI_CONTRACT_TEST_EXIT }
    else { $global:LASTEXITCODE = [int]$env:CI_CONTRACT_CHECK_EXIT }
}
& $EntryPoint -ExpectedSha $Expected -EvidenceDirectory $Evidence
exit $LASTEXITCODE
'@ | Set-Content -Path $harness -Encoding utf8

$cases = @(
    @{ Name = '成功'; Test = 0; Check = 0; ExpectedExit = 0; Commands = 2 }
    @{ Name = '测试失败不能被check成功覆盖'; Test = 101; Check = 0; ExpectedExit = 1; Commands = 2 }
    @{ Name = 'check失败'; Test = 0; Check = 101; ExpectedExit = 1; Commands = 2 }
    @{ Name = '两个命令均失败'; Test = 101; Check = 101; ExpectedExit = 1; Commands = 2 }
    @{ Name = '提交不匹配'; Test = 0; Check = 0; ExpectedExit = 1; Commands = 0; Mismatch = $true }
    @{ Name = '工作区不干净'; Test = 0; Check = 0; ExpectedExit = 1; Commands = 0; Dirty = $true }
    @{ Name = 'Git状态读取失败'; Test = 0; Check = 0; ExpectedExit = 1; Commands = 0; GitFail = $true }
    @{ Name = '非法SHA参数'; Test = 0; Check = 0; ExpectedExit = 1; Commands = 0; Invalid = $true }
)
$variables = @('CI_CONTRACT_CALLS','CI_CONTRACT_TEST_EXIT','CI_CONTRACT_CHECK_EXIT','CI_CONTRACT_DIRTY','CI_CONTRACT_GIT_FAIL')
$original = @{}
foreach ($name in $variables) { $original[$name] = [Environment]::GetEnvironmentVariable($name) }
try {
    for ($i = 0; $i -lt $cases.Count; $i++) {
        $case = $cases[$i]
        $directory = Join-Path $root $i.ToString()
        $null = New-Item -ItemType Directory -Path $directory -Force
        $env:CI_CONTRACT_CALLS = $directory
        $env:CI_CONTRACT_TEST_EXIT = $case.Test.ToString()
        $env:CI_CONTRACT_CHECK_EXIT = $case.Check.ToString()
        $env:CI_CONTRACT_DIRTY = [string][bool]$case.Dirty
        $env:CI_CONTRACT_GIT_FAIL = [string][bool]$case.GitFail
        $expected = '0' * 40
        if ($case.Mismatch) { $expected = '1' * 40 }
        if ($case.Invalid) { $expected = 'invalid' }
        & $hostExecutable -NoProfile -File $harness -EntryPoint $entryPoint -Evidence $directory -Expected $expected *> (Join-Path $directory 'child.log')
        $code = $LASTEXITCODE
        if ($code -ne $case.ExpectedExit) { throw "$($case.Name)：退出码$code不符合预期" }
        $callPath = Join-Path $directory 'calls.log'
        $calls = @()
        if (Test-Path $callPath) { $calls = @(Get-Content $callPath) }
        if ($calls.Count -ne $case.Commands) { throw "$($case.Name)：命令执行数量错误" }
        if ($case.Commands -eq 2 -and ($calls[0] -ne 'test' -or $calls[1] -ne 'check')) {
            throw "$($case.Name)：验证命令顺序错误"
        }
        $evidencePath = Join-Path $directory 'validation.json'
        if ($case.Invalid) {
            if (Test-Path $evidencePath) { throw '非法参数不能产生通过证据' }
        } else {
            $evidence = Get-Content -Raw $evidencePath | ConvertFrom-Json
            $state = if ($code -eq 0) { 'passed' } else { 'failed' }
            if ($evidence.state -ne $state -or $evidence.testedSha -ne ('0' * 40)) { throw "$($case.Name)：状态或提交绑定错误" }
            if (-not $evidence.finishedAt) { throw "$($case.Name)：未保存结束时间" }
            if ($case.Commands -eq 2) {
                if ($evidence.testExitCode -ne $case.Test -or $evidence.checkExitCode -ne $case.Check) {
                    throw "$($case.Name)：未分别保存命令退出码"
                }
                if (-not (Test-Path (Join-Path $directory 'cargo-test.log')) -or -not (Test-Path (Join-Path $directory 'cargo-check.log'))) {
                    throw "$($case.Name)：缺少验证日志"
                }
            } elseif ($null -ne $evidence.testExitCode -or $null -ne $evidence.checkExitCode) {
                throw "$($case.Name)：未运行命令不能伪造退出码"
            }
        }
        Write-Output "通过：$($case.Name)"
    }
} finally {
    foreach ($name in $variables) { [Environment]::SetEnvironmentVariable($name, $original[$name]) }
}
Write-Output "脚本合同：$($cases.Count)通过；全部使用合成命令。"
exit 0
