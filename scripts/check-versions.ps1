#Requires -Version 5.1
<#
  Version consistency gate for GPO Autofish releases.
  Usage:
    scripts\check-versions.ps1            # compare manifests against each other
    scripts\check-versions.ps1 -Tag v4.3.0  # additionally require tag match (CI release)
  Fails (exit 1) on any mismatch. Never prints secrets.
#>
param([string]$Tag = "")

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

$pkg = Get-Content (Join-Path $root "package.json") | ConvertFrom-Json
$tauri = Get-Content (Join-Path $root "src-tauri\tauri.conf.json") | ConvertFrom-Json
$cargoText = Get-Content (Join-Path $root "src-tauri\Cargo.toml") -Raw
if ($cargoText -notmatch '(?m)^version\s*=\s*"([^"]+)"') { Write-Error "cannot parse src-tauri/Cargo.toml version"; exit 1 }
$cargoVer = $Matches[1]

$failures = @()
if ($pkg.version -ne $tauri.version) { $failures += "package.json ($($pkg.version)) != tauri.conf.json ($($tauri.version))" }
if ($pkg.version -ne $cargoVer) { $failures += "package.json ($($pkg.version)) != Cargo.toml ($cargoVer)" }
if ($Tag -ne "") {
  $tagVer = $Tag.TrimStart("v")
  if ($tagVer -ne $pkg.version) { $failures += "tag ($Tag) != package.json ($($pkg.version))" }
  if ($Tag -notmatch '^v\d+\.\d+\.\d+(-beta\.\d+)?$') { $failures += "tag ($Tag) is not vX.Y.Z or vX.Y.Z-beta.N" }
}

if ($failures.Count -gt 0) {
  $failures | ForEach-Object { Write-Error $_ }
  exit 1
}
Write-Output "versions consistent: $($pkg.version)"
