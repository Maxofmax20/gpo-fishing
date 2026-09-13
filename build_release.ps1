$ErrorActionPreference = "Stop"
$keyPath = Join-Path $PSScriptRoot ".tauri\updater_key"
$env:TAURI_SIGNING_PRIVATE_KEY_PATH = $keyPath
$env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content -Raw $keyPath)
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "gpo-autofish"

Write-Host "Building GPO Autofish 4.0.4..."
npm run app:build

$outDir = Join-Path $PSScriptRoot "src-tauri\target\release\bundle\nsis"
Write-Host "Generating latest.json in $outDir..."
& "$PSScriptRoot\src-tauri\latest-json.ps1" -Out $outDir

$relDir = Join-Path $PSScriptRoot "release"
if (!(Test-Path $relDir)) { New-Item -ItemType Directory -Path $relDir }

Copy-Item "$outDir\GPO Autofish_4.0.4_x64-setup.exe" "$relDir\GPO.Autofish_4.0.4_x64-setup.exe" -Force
Copy-Item "$outDir\GPO Autofish_4.0.4_x64-setup.exe.sig" "$relDir\GPO.Autofish_4.0.4_x64-setup.exe.sig" -Force
Copy-Item "$outDir\latest.json" "$relDir\latest.json" -Force

Write-Host "Build and packaging complete!"
