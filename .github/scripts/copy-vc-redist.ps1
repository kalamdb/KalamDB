param(
  [Parameter(Mandatory = $true)]
  [string]$DestinationDir
)

$ErrorActionPreference = "Stop"

$RequiredDlls = @(
  "msvcp140.dll",
  "vcruntime140.dll",
  "vcruntime140_1.dll"
)

$searchRoots = @(
  Join-Path $env:ProgramFiles "Microsoft Visual Studio\2022"
  Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\2022"
)

$crtDirs = foreach ($root in $searchRoots) {
  if (-not (Test-Path $root)) {
    continue
  }

  Get-ChildItem -Path $root -Directory -ErrorAction SilentlyContinue | ForEach-Object {
    Get-ChildItem -Path (Join-Path $_.FullName "VC\Redist\MSVC\*\x64\Microsoft.VC*CRT") -Directory -ErrorAction SilentlyContinue
  }
} | Sort-Object FullName -Descending

if (-not $crtDirs -or $crtDirs.Count -eq 0) {
  throw "Could not locate Microsoft VC++ CRT redistributable directory on this runner"
}

$crtDir = $crtDirs[0]
Write-Host "Bundling VC++ CRT DLLs from $($crtDir.FullName)"

New-Item -ItemType Directory -Force -Path $DestinationDir | Out-Null
Copy-Item -Path (Join-Path $crtDir.FullName "*.dll") -Destination $DestinationDir -Force

Get-ChildItem -Path $DestinationDir -Filter "*.dll" | ForEach-Object {
  Write-Host "  $($_.Name)"
}

foreach ($required in $RequiredDlls) {
  $match = Get-ChildItem -Path $DestinationDir -Filter $required -File -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $match) {
    throw "Bundled VC++ runtime is missing required DLL: $required"
  }
}
