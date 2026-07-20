[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$Destination
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$version = '0.4.5'
$expectedSha512 = 'Y8T/yXCDGagRGiQrtmuB6AhRcPucKFs/Dre3v8kJwNYqDccI4FzUPKclZ7djfmRZNjl7JUqPhZZP/PwDpQocMg=='
$url = "https://registry.npmjs.org/@opentui/core-win32-x64/-/core-win32-x64-$version.tgz"

$destinationPath = [IO.Path]::GetFullPath($Destination)
$dependencyDirectory = Split-Path -Parent $destinationPath
$licensePath = Join-Path $dependencyDirectory 'LICENSE.opentui'
if ((Test-Path -LiteralPath $destinationPath) -and
    (Test-Path -LiteralPath $licensePath)) {
  Write-Output $destinationPath
  return
}

$archivePath = Join-Path $dependencyDirectory "core-win32-x64-$version.tgz"
$extractDirectory = Join-Path $dependencyDirectory 'package-extract'
New-Item -ItemType Directory -Force -Path $dependencyDirectory | Out-Null
New-Item -ItemType Directory -Force -Path $extractDirectory | Out-Null

if (-not (Test-Path -LiteralPath $archivePath)) {
  Write-Host "Downloading OpenTUI native C ABI $version..."
  Invoke-WebRequest -Uri $url -OutFile $archivePath
}

$hash = Get-FileHash -LiteralPath $archivePath -Algorithm SHA512
$hashBytes = [Convert]::FromHexString($hash.Hash)
$actualSha512 = [Convert]::ToBase64String($hashBytes)
if ($actualSha512 -ne $expectedSha512) {
  throw "OpenTUI archive integrity check failed: $archivePath"
}

& tar.exe -xf $archivePath -C $extractDirectory
if ($LASTEXITCODE -ne 0) {
  throw "tar.exe failed extracting OpenTUI with exit code $LASTEXITCODE"
}

$sourceDll = Join-Path $extractDirectory 'package\opentui.dll'
$sourceLicense = Join-Path $extractDirectory 'package\LICENSE'
if (-not (Test-Path -LiteralPath $sourceDll)) {
  throw "OpenTUI package did not contain opentui.dll"
}
if (-not (Test-Path -LiteralPath $sourceLicense)) {
  throw "OpenTUI package did not contain its license"
}
Copy-Item -LiteralPath $sourceDll -Destination $destinationPath
Copy-Item -LiteralPath $sourceLicense -Destination $licensePath
Write-Output $destinationPath
