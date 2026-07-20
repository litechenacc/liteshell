[CmdletBinding()]
param(
  [string]$Executable = (Join-Path $PSScriptRoot '..\build\release\liteshell.exe')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$executablePath = (Resolve-Path -LiteralPath $Executable).Path
$projectRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$smokeRoot = Join-Path $projectRoot 'build\smoke'
$unicodeDirectory = Join-Path $smokeRoot '中文 path'
$localAppData = Join-Path $smokeRoot 'local-app-data'
New-Item -ItemType Directory -Force -Path $unicodeDirectory, $localAppData | Out-Null
Set-Content -LiteralPath (Join-Path $unicodeDirectory 'utf8 file.txt') -Encoding utf8NoBOM -Value @(
  'alpha'
  'beta'
  'gamma'
)
[IO.File]::WriteAllBytes((Join-Path $smokeRoot 'binary.bin'), [byte[]](0, 1, 2, 3, 0, 255))

$previousLocalAppData = $env:LOCALAPPDATA
$previousTestValue = $env:LITESHELL_TEST_VALUE
$env:LOCALAPPDATA = $localAppData
$env:LITESHELL_TEST_VALUE = 'ENV_OK'
try {
  $commands = @(
    'pwd'
    'pwd'
    'cd "build\smoke\中文 path"'
    'pwd'
    'ls -la'
    'cat "utf8 file.txt"'
    'cd "$HOME"'
    "cd `"$projectRoot`""
    'tail -n 2 tests\fixtures\tail.txt'
    'less tests\fixtures\tail.txt'
    "cat 'tests\fixtures\com.txt'"
    'which ls cmd'
    'tests\fixtures\hello.cmd "argument with spaces"'
    'tests\fixtures\hello.bat "argument with spaces"'
    'tests\fixtures\hello.ps1 "argument with spaces"'
    'more.com tests\fixtures\com.txt'
    'cmd /d /c "echo EXE_OK"'
    'cmd /d /c "echo $LITESHELL_TEST_VALUE"'
    'cmd /d /c "echo %LITESHELL_TEST_VALUE%"'
    'find fff-index-probe'
    'rg LITESHELL_FFF_CONTENT_PROBE'
    'cat build\smoke\binary.bin'
    'exit'
  )
  $output = ($commands | & $executablePath 2>&1 | Out-String)
} finally {
  $env:LOCALAPPDATA = $previousLocalAppData
  $env:LITESHELL_TEST_VALUE = $previousTestValue
}

$requiredPatterns = @(
  '中文 path'
  'utf8 file.txt'
  'alpha'
  'gamma'
  'line-4'
  'line-5'
  'ls: builtin'
  'cmd: C:\Windows'
  'CMD_OK:argument with spaces'
  'BAT_OK:argument with spaces'
  'PS1_OK:argument with spaces'
  'COM_OK'
  'EXE_OK'
  'ENV_OK'
  'tests\fixtures\fff-index-probe.txt'
  'LITESHELL_FFF_CONTENT_PROBE'
  'refusing to print binary file'
)

foreach ($pattern in $requiredPatterns) {
  if ($output -notmatch [regex]::Escape($pattern)) {
    Write-Host $output
    throw "Smoke test output did not contain: $pattern"
  }
}

$historyPath = Join-Path $localAppData 'LiteShell\history'
if (-not (Test-Path -LiteralPath $historyPath)) {
  throw "History was not persisted to $historyPath"
}
$historyLines = @(Get-Content -LiteralPath $historyPath -Encoding utf8)
for ($index = 1; $index -lt $historyLines.Count; $index++) {
  if ($historyLines[$index] -eq $historyLines[$index - 1]) {
    throw 'History contains adjacent duplicate commands.'
  }
}
if ($historyLines.Count -gt 5000) {
  throw "History exceeded configured capacity: $($historyLines.Count)"
}

Write-Host 'LiteShell smoke tests passed.'
