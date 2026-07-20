[CmdletBinding()]
param(
  [ValidateSet('Debug', 'Release')]
  [string]$Configuration = 'Release'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$projectRoot = Split-Path -Parent $PSCommandPath
$sourceDirectory = Join-Path $projectRoot 'src'
$sourcePaths = @(Get-ChildItem -LiteralPath $sourceDirectory -Filter '*.cpp' |
    Sort-Object Name |
    ForEach-Object FullName)
$configurationName = $Configuration.ToLowerInvariant()
$outputDirectory = Join-Path $projectRoot "build\$configurationName"
$executablePath = Join-Path $outputDirectory 'liteshell.exe'
$compilerPdbPath = Join-Path $outputDirectory 'liteshell-compiler.pdb'
$linkerPdbPath = Join-Path $outputDirectory 'liteshell.pdb'
$openTuiVersion = '0.4.5'
$openTuiDll = Join-Path $projectRoot "build\deps\opentui-$openTuiVersion\opentui.dll"
$openTuiLicense = Join-Path $projectRoot "build\deps\opentui-$openTuiVersion\LICENSE.opentui"
$fffVersion = '0.10.0'
$fffDll = Join-Path $projectRoot "build\deps\fff-$fffVersion\fff_c.dll"
$fffLicense = Join-Path $projectRoot "build\deps\fff-$fffVersion\LICENSE.fff"

$compiler = Get-Command cl.exe -ErrorAction SilentlyContinue
if (-not $compiler -or -not $env:INCLUDE -or -not $env:LIB) {
  throw @'
MSVC is not configured in this terminal. Open "Developer PowerShell for VS"
(x64), then run .\build.ps1 again.
'@
}

New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null

$commonCompilerArguments = @(
  '/nologo'
  '/std:c++20'
  '/EHsc'
  '/permissive-'
  '/utf-8'
  '/W4'
  '/DUNICODE'
  '/D_UNICODE'
  "/I$sourceDirectory"
  "/Fd$compilerPdbPath"
  '/c'
)

$linkerArguments = @(
  '/link'
  '/INCREMENTAL:NO'
  "/PDB:$linkerPdbPath"
)

if ($Configuration -eq 'Release') {
  $commonCompilerArguments += @('/O2', '/GL', '/MT', '/DNDEBUG')
  $linkerArguments += @('/LTCG', '/OPT:REF', '/OPT:ICF')
} else {
  $commonCompilerArguments += @('/Od', '/Zi', '/MTd')
  $linkerArguments += '/DEBUG'
}

$objectPaths = @()
foreach ($sourcePath in $sourcePaths) {
  $objectPath = Join-Path $outputDirectory "$([IO.Path]::GetFileNameWithoutExtension($sourcePath)).obj"
  & $compiler.Source @commonCompilerArguments "/Fo$objectPath" $sourcePath
  if ($LASTEXITCODE -ne 0) {
    throw "cl.exe failed compiling $sourcePath with exit code $LASTEXITCODE"
  }
  $objectPaths += $objectPath
}

& $compiler.Source '/nologo' @objectPaths "/Fe$executablePath" @linkerArguments
if ($LASTEXITCODE -ne 0) {
  throw "cl.exe failed linking with exit code $LASTEXITCODE"
}

$fetchOpenTuiScript = Join-Path $projectRoot 'scripts\fetch-opentui.ps1'
& $fetchOpenTuiScript -Destination $openTuiDll | Out-Host
Copy-Item -LiteralPath $openTuiDll -Destination (Join-Path $outputDirectory 'opentui.dll')
Copy-Item -LiteralPath $openTuiLicense -Destination (Join-Path $outputDirectory 'LICENSE.opentui')

$fetchFffScript = Join-Path $projectRoot 'scripts\fetch-fff.ps1'
& $fetchFffScript -Destination $fffDll | Out-Host
Copy-Item -LiteralPath $fffDll -Destination (Join-Path $outputDirectory 'fff_c.dll')
Copy-Item -LiteralPath $fffLicense -Destination (Join-Path $outputDirectory 'LICENSE.fff')

Write-Host "Built $executablePath (OpenTUI + fff)"
