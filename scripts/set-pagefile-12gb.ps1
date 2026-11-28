# 以【管理员】身份运行：右键 → 使用 PowerShell 运行（或管理员 PowerShell 里执行本文件）
# 目的：把页面文件固定成 12 GB（64 GB 内存下峰值只用到 1 MB，26 GB 纯属浪费）
if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  Write-Host "需要管理员权限，请右键以管理员身份运行。" -ForegroundColor Red; exit 1 }

$cs = Get-CimInstance Win32_ComputerSystem
if ($cs.AutomaticManagedPagefile) {
  Set-CimInstance -InputObject $cs -Property @{ AutomaticManagedPagefile = $false }
  Write-Host "已关闭自动管理"
} else { Write-Host "自动管理本来就是关的" }

$pf = Get-CimInstance Win32_PageFileSetting | Where-Object { $_.Name -like "C:*" }
if ($pf) {
  Set-CimInstance -InputObject $pf -Property @{ InitialSize = 12288; MaximumSize = 12288 }
  Write-Host "已设为 12288 / 12288 MB"
} else { Write-Host "没找到 C: 的页面文件设置" -ForegroundColor Yellow }

Write-Host ""
Get-CimInstance Win32_PageFileSetting | ForEach-Object { Write-Host ("  现在: {0}  初始 {1} MB / 最大 {2} MB" -f $_.Name, $_.InitialSize, $_.MaximumSize) }
Write-Host "重启后生效（C: 可多出约 2.5 GB，主要价值是不再让 Windows 扩到 26 GB）" -ForegroundColor Cyan