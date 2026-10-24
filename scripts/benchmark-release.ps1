param(
  [ValidateSet("fat","thin")][string]$Lto="fat",
  [ValidateSet(1,4,8)][int]$CodegenUnits=1,
  [ValidateRange(1,5)][int]$Iterations=3
)
$ErrorActionPreference="Stop"
$env:CARGO_PROFILE_RELEASE_LTO=$Lto
$env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS="$CodegenUnits"
$env:CARGO_INCREMENTAL="0"
$out=Join-Path $PSScriptRoot "../benchmark-results"
New-Item -ItemType Directory -Force $out | Out-Null
$source=Join-Path $PSScriptRoot "../src-tauri/src/lib.rs"
$original=[IO.File]::ReadAllBytes($source)
$records=@()
try {
  Push-Location (Join-Path $PSScriptRoot "../src-tauri")
  & cargo build --release --locked -j 2
  if($LASTEXITCODE -ne 0){throw "Warm-up build failed"}
  for($i=1;$i -le $Iterations;$i++){
    $suffix=[Text.Encoding]::UTF8.GetBytes([Environment]::NewLine+"// isolated benchmark rebuild $i"+[Environment]::NewLine)
    [IO.File]::WriteAllBytes($source, [byte[]]($original+$suffix))
    $watch=[Diagnostics.Stopwatch]::StartNew()
    & cargo build --release --locked -j 2
    $buildExit=$LASTEXITCODE
    $watch.Stop()
    if($buildExit -ne 0){throw "Measured build failed"}
    $exe=Get-Item "target/release/gpo-autofish.exe"
    $records+=@{iteration=$i;seconds=$watch.Elapsed.TotalSeconds;bytes=$exe.Length}
  }
} finally {
  [IO.File]::WriteAllBytes($source,$original)
  Pop-Location
}
$report=@{lto=$Lto;codegen_units=$CodegenUnits;iterations=$records;native_gameplay_verified=$false}
$report | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $out "$Lto-$CodegenUnits.json") -Encoding utf8
