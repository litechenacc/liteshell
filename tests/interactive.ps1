[CmdletBinding()]
param(
  [string]$Executable = (Join-Path $PSScriptRoot '..\build\release\liteshell.exe')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$compiler = Get-Command cl.exe -ErrorAction SilentlyContinue
if (-not $compiler -or -not $env:INCLUDE -or -not $env:LIB) {
  throw 'Run this test from an x64 Visual Studio Developer PowerShell.'
}

$projectRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$executablePath = (Resolve-Path -LiteralPath $Executable).Path
$outputDirectory = Join-Path $projectRoot 'build\tests'
$testExecutable = Join-Path $outputDirectory 'console-input-test.exe'
$testObject = Join-Path $outputDirectory 'console-input-test.obj'
New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null

& $compiler.Source '/nologo' '/std:c++20' '/EHsc' '/permissive-' '/utf-8' '/W4' '/MT' `
    (Join-Path $PSScriptRoot 'console_input_test.cpp') "/Fo$testObject" `
    "/Fe$testExecutable" 'user32.lib'
if ($LASTEXITCODE -ne 0) {
  throw "Failed to compile console input test: $LASTEXITCODE"
}

& $testExecutable $executablePath $projectRoot
if ($LASTEXITCODE -ne 0) {
  throw "Console input interaction test failed: $LASTEXITCODE"
}
