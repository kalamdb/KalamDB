param(
  [Parameter(Mandatory = $true)]
  [string]$ZipPath
)

$ErrorActionPreference = "Stop"

$RequiredDlls = @(
  "msvcp140.dll",
  "vcruntime140.dll",
  "vcruntime140_1.dll"
)

if (-not (Test-Path $ZipPath)) {
  throw "Release zip not found: $ZipPath"
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [System.IO.Compression.ZipFile]::OpenRead($ZipPath)

try {
  $entryNames = @(
    $zip.Entries |
      ForEach-Object { $_.Name.ToLowerInvariant() }
  )

  foreach ($required in $RequiredDlls) {
    if ($entryNames -notcontains $required) {
      throw "Release zip '$ZipPath' is missing required runtime DLL: $required"
    }
  }

  Write-Host "Verified runtime DLLs in $ZipPath"
} finally {
  $zip.Dispose()
}
