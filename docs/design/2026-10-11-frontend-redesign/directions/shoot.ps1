# 用无头 Edge 为每个方向与屏幕截图：.\shoot.ps1 [-Widths 1440,1920]
param([int[]]$Widths = @(1440))
$edge = @("C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe","C:\Program Files\Microsoft\Edge\Application\msedge.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
$out = Join-Path $PSScriptRoot "shots"
New-Item -ItemType Directory -Force $out | Out-Null
foreach ($w in $Widths) { $h = [int]($w * 0.625)
  foreach ($d in 'a','b','c') { foreach ($s in 'write','setup') {
    & $edge --headless=new --disable-gpu --hide-scrollbars --window-size="$w,$h" --virtual-time-budget=10000 --screenshot="$out\$d-$s-$w.png" "http://127.0.0.1:4780/directions/index.html#$d-$s-clean" 2>$null | Out-Null
  } }
}
Get-ChildItem $out | Select-Object Name, Length
