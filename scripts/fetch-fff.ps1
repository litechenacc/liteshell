[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$Destination
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$version = '0.10.0'
$expectedSha256 = '2d643319aee9899980084245e4fd6752c084c7a343e47228449458550ad55966'
$assetName = 'c-lib-x86_64-pc-windows-msvc.dll'
$baseUrl = "https://github.com/dmtrKovalenko/fff/releases/download/v$version"
$licenseUrl = "https://raw.githubusercontent.com/dmtrKovalenko/fff/v$version/LICENSE"

$destinationPath = [IO.Path]::GetFullPath($Destination)
$dependencyDirectory = Split-Path -Parent $destinationPath
$licensePath = Join-Path $dependencyDirectory 'LICENSE.fff'
if ((Test-Path -LiteralPath $destinationPath) -and
    (Test-Path -LiteralPath $licensePath)) {
  Write-Output $destinationPath
  return
}

New-Item -ItemType Directory -Force -Path $dependencyDirectory | Out-Null
if (-not (Test-Path -LiteralPath $destinationPath)) {
  Write-Host "Downloading fff C library $version..."
  Invoke-WebRequest -Uri "$baseUrl/$assetName" -OutFile $destinationPath
}

$actualSha256 = (Get-FileHash -LiteralPath $destinationPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha256 -ne $expectedSha256) {
  throw "fff DLL integrity check failed: $destinationPath"
}

if (-not (Test-Path -LiteralPath $licensePath)) {
  Invoke-WebRequest -Uri $licenseUrl -OutFile $licensePath
}
Write-Output $destinationPath
