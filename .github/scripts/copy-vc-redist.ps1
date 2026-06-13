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

function Get-VcCrtDirectory {
  $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
  if (Test-Path $vswhere) {
    $installPath = & $vswhere `
      -latest `
      -products * `
      -requires Microsoft.VisualStudio.Component.VC.Redist.14.Latest `
      -property installationPath 2>$null

    if ($installPath) {
      $redistRoot = Join-Path $installPath "VC\Redist\MSVC"
      if (Test-Path $redistRoot) {
        $crtDirs = @(
          Get-ChildItem -Path (Join-Path $redistRoot "*\x64\Microsoft.VC*CRT") -Directory -ErrorAction SilentlyContinue
        )
        if ($crtDirs.Count -gt 0) {
          return ($crtDirs | Sort-Object FullName -Descending | Select-Object -First 1)
        }
      }
    }
  }

  $searchRoots = @(
    Join-Path $env:ProgramFiles "Microsoft Visual Studio"
    Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio"
  )

  $crtDirs = @()
  foreach ($root in $searchRoots) {
    if (-not (Test-Path $root)) {
      continue
    }

    $editionDirs = Get-ChildItem -Path $root -Directory -ErrorAction SilentlyContinue
    foreach ($editionDir in $editionDirs) {
      $matches = Get-ChildItem -Path (Join-Path $editionDir.FullName "VC\Redist\MSVC\*\x64\Microsoft.VC*CRT") -Directory -ErrorAction SilentlyContinue
      if ($matches) {
        $crtDirs += $matches
      }
    }
  }

  if ($crtDirs.Count -gt 0) {
    return ($crtDirs | Sort-Object FullName -Descending | Select-Object -First 1)
  }

  return $null
}

function Copy-RequiredDllsFromDirectory {
  param(
    [Parameter(Mandatory = $true)]
    [string]$SourceDir
  )

  New-Item -ItemType Directory -Force -Path $DestinationDir | Out-Null

  foreach ($required in $RequiredDlls) {
    $match = Get-ChildItem -Path $SourceDir -Filter $required -File -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $match) {
      throw "Could not find required runtime DLL '$required' under '$SourceDir'"
    }

    Copy-Item -Path $match.FullName -Destination (Join-Path $DestinationDir $required) -Force
    Write-Host "  $required"
  }
}

function Import-VcRedistFromDownload {
  $redistUrl = "https://aka.ms/vs/17/release/vc_redist.x64.exe"
  $redistExe = Join-Path $env:RUNNER_TEMP "vc_redist.x64.exe"
  $extractDir = Join-Path $env:RUNNER_TEMP "vc_redist_extract"

  if (Test-Path $extractDir) {
    Remove-Item -Path $extractDir -Recurse -Force
  }
  New-Item -ItemType Directory -Force -Path $extractDir | Out-Null

  Write-Host "Downloading VC++ redistributable from $redistUrl"
  Invoke-WebRequest -Uri $redistUrl -OutFile $redistExe

  Write-Host "Extracting VC++ redistributable to $extractDir"
  $extractProcess = Start-Process -FilePath $redistExe -ArgumentList "/extract:$extractDir", "/quiet", "/norestart" -Wait -PassThru -NoNewWindow
  if ($extractProcess.ExitCode -ne 0) {
    throw "vc_redist.x64.exe extract failed with exit code $($extractProcess.ExitCode)"
  }

  return $extractDir
}

$crtDir = Get-VcCrtDirectory
if ($crtDir) {
  Write-Host "Bundling VC++ CRT DLLs from $($crtDir.FullName)"
  Copy-RequiredDllsFromDirectory -SourceDir $crtDir.FullName
} else {
  Write-Host "Visual Studio CRT directory not found; falling back to official vc_redist.x64.exe"
  $extractDir = Import-VcRedistFromDownload
  Copy-RequiredDllsFromDirectory -SourceDir $extractDir
}

foreach ($required in $RequiredDlls) {
  $match = Join-Path $DestinationDir $required
  if (-not (Test-Path $match)) {
    throw "Bundled VC++ runtime is missing required DLL: $required"
  }
}
