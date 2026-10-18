# 为原型每个屏幕截浅色与深色图：.\shoot.ps1 [-Pages chapter-review,setup] [-Width 1440]
# 依赖本地服务：在 docs/design/2026-10-11-frontend-redesign 下运行 python -m http.server 4780 --bind 127.0.0.1
param([string[]]$Pages, [int]$Width = 1440)
$edge = @("C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe","C:\Program Files\Microsoft\Edge\Application\msedge.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
$out = Join-Path $PSScriptRoot "shots"
New-Item -ItemType Directory -Force $out | Out-Null
if (-not $Pages) { $Pages = Get-ChildItem $PSScriptRoot -Filter *.html | Where-Object Name -ne 'index.html' | ForEach-Object BaseName }
$h = [int]($Width * 0.625)
foreach ($p in $Pages) { foreach ($t in 'light','dark') {
  & $edge --headless=new --disable-gpu --hide-scrollbars --window-size="$Width,$h" --virtual-time-budget=16000 --screenshot="$out\$p-$t.png" "http://127.0.0.1:4780/prototype/$p.html?theme=$t&clean" 2>$null | Out-Null
} }
Get-ChildItem $out | Where-Object { $Pages -contains ($_.BaseName -replace '-(light|dark)$','') } | Select-Object Name, Length
