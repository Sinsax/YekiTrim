#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Package a V-Trim release (Windows x64).

.DESCRIPTION
    Steps: cargo build --release -> assemble vtrim.exe + webui/dist + resources/ + release notes
    -> structural checks -> optional smoke test -> zip. Output goes to <repo>/release/ (gitignored).

    NOTE: keep this script ASCII-only. Windows PowerShell 5.1 reads .ps1 without a BOM as ANSI,
    so non-ASCII text here would corrupt string literals and break parsing.

.EXAMPLE
    pwsh scripts/package-release.ps1                # full build + package
    pwsh scripts/package-release.ps1 -SkipBuild     # reuse existing target/release/vtrim.exe
    pwsh scripts/package-release.ps1 -SmokeTest     # package, then boot it once and verify
#>
[CmdletBinding()]
param(
    [string]$Version = "",
    [switch]$SkipBuild,
    [switch]$SmokeTest,
    [string]$OutDir = ""
)

$ErrorActionPreference = 'Stop'

# scripts/ -> repo root
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not $Version) {
    $m = Select-String -Path 'crates/vtrim-cli/Cargo.toml' -Pattern '^version\s*=\s*"([^"]+)"'
    if (-not $m) { throw "cannot read version from crates/vtrim-cli/Cargo.toml" }
    $Version = $m.Matches[0].Groups[1].Value
}
if (-not $OutDir) { $OutDir = Join-Path $root 'release' }

$pkgName = "vtrim-v$Version-win-x64"
$pkg = Join-Path $OutDir $pkgName
$zip = Join-Path $OutDir "$pkgName.zip"

Write-Host "==> version: $Version"

if (-not $SkipBuild) {
    Write-Host "==> cargo build --release -p vtrim-cli --locked"
    # PowerShell 5.1 turns native stderr (cargo's linker note) into a terminating error under
    # $ErrorActionPreference='Stop'; relax it for the call and trust $LASTEXITCODE instead.
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    cargo build --release -p vtrim-cli --locked 2>&1 | ForEach-Object { Write-Host "    $_" }
    $buildExit = $LASTEXITCODE
    $ErrorActionPreference = $prevEap
    if ($buildExit -ne 0) { throw "cargo build failed (exit $buildExit)" }
}

$exe = Join-Path $root 'target/release/vtrim.exe'
if (-not (Test-Path $exe)) { throw "missing $exe" }

# WebUI comes only from webui/dist (single-file fallback was removed), so a stale dist ships a stale UI.
if (-not (Test-Path (Join-Path $root 'webui/dist/index.html'))) {
    throw "webui/dist missing: run 'npm --prefix webui run build' first"
}
$distIndex = Get-Item (Join-Path $root 'webui/dist/index.html')
$srcNewest = Get-ChildItem 'webui/src' -Recurse -File |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($srcNewest -and $srcNewest.LastWriteTime -gt $distIndex.LastWriteTime.AddSeconds(1)) {
    Write-Warning "webui/src is newer than webui/dist ($($srcNewest.Name)): run 'npm --prefix webui run build' first"
}

$font = Join-Path $root 'crates/vtrim-core/resources/lemi-bobo.ttf'

Write-Host "==> assembling $pkgName"
if (Test-Path $pkg) { Remove-Item -Recurse -Force $pkg }
New-Item -ItemType Directory -Path $pkg | Out-Null
New-Item -ItemType Directory -Path (Join-Path $pkg 'webui') | Out-Null
New-Item -ItemType Directory -Path (Join-Path $pkg 'resources') | Out-Null

Copy-Item $exe (Join-Path $pkg 'vtrim.exe')
Copy-Item -Recurse (Join-Path $root 'webui/dist') (Join-Path $pkg 'webui/dist')
if (Test-Path $font) { Copy-Item $font (Join-Path $pkg 'resources/lemi-bobo.ttf') }

$notes = Join-Path $root "docs/releases/v$Version.md"
if (Test-Path $notes) {
    Copy-Item $notes (Join-Path $pkg 'RELEASE-NOTES.md')
} else {
    Write-Warning "release notes not found: $notes"
}

# The server serves the UI from <exe_dir>/webui/dist
foreach ($required in @('vtrim.exe', 'webui/dist/index.html')) {
    if (-not (Test-Path (Join-Path $pkg $required))) { throw "package is missing $required" }
}

if ($SmokeTest) {
    Write-Host "==> smoke test: boot the packaged exe and verify"
    $port = Get-Random -Minimum 18000 -Maximum 18999
    $proc = Start-Process -FilePath (Join-Path $pkg 'vtrim.exe') `
        -ArgumentList @('server', '--port', "$port") -PassThru -WindowStyle Hidden
    try {
        $ok = $false
        foreach ($i in 1..20) {
            Start-Sleep -Milliseconds 500
            try {
                $health = Invoke-RestMethod -Uri "http://127.0.0.1:$port/api/health" -TimeoutSec 3
                $index = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$port/" -TimeoutSec 5
                if ($health.ok -and $index.StatusCode -eq 200 -and $index.Content -match '<div id="app">') {
                    Write-Host "    /api/health ok, / served the packaged WebUI"
                    $ok = $true
                    break
                }
            } catch { }
        }
        if (-not $ok) { throw "smoke test failed: server did not return /api/health and the WebUI in time" }
    } finally {
        if ($proc -and -not $proc.HasExited) { Stop-Process -Id $proc.Id -Force }
        # first run creates .vtrim / stickers next to the exe; runtime state must not ship
        foreach ($generated in @('.vtrim', 'stickers')) {
            $p = Join-Path $pkg $generated
            if (Test-Path $p) { Remove-Item -Recurse -Force $p }
        }
    }
}

Write-Host "==> compressing $zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path $pkg -DestinationPath $zip -CompressionLevel Optimal

$hash = (Get-FileHash -Algorithm SHA256 $zip).Hash
$sizeMb = [math]::Round((Get-Item $zip).Length / 1MB, 1)
Write-Host ""
Write-Host "done: $zip"
Write-Host "  size:   $sizeMb MB"
Write-Host "  sha256: $hash"
