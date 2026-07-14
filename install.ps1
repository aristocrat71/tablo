# tablo installer — Windows. Pulls the latest build from GitHub Releases.
#   irm https://raw.githubusercontent.com/unravel-team/tablo/main/install.ps1 | iex
$ErrorActionPreference = 'Stop'

$repo = 'unravel-team/tablo'
$api  = "https://api.github.com/repos/$repo/releases/latest"

function Say($m) { Write-Host "==> $m" -ForegroundColor Green }

Say "Fetching the latest tablo release..."
$release = Invoke-RestMethod -Uri $api -Headers @{ 'User-Agent' = 'tablo-install' }

# Prefer the NSIS setup .exe; fall back to the .msi.
$asset = $release.assets | Where-Object { $_.name -match '-setup\.exe$' } | Select-Object -First 1
if (-not $asset) { $asset = $release.assets | Where-Object { $_.name -match '\.msi$' } | Select-Object -First 1 }
if (-not $asset) { throw "no Windows installer (.exe / .msi) in the latest release yet" }

$out = Join-Path $env:TEMP $asset.name
Say "Downloading $($asset.name)..."
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $out -UseBasicParsing

# Clear the "mark of the web" so SmartScreen doesn't show the "protected your PC" prompt.
Unblock-File -Path $out

Say "Launching the installer..."
Start-Process -FilePath $out -Wait
Say "Done. tablo updates itself automatically from here on."
