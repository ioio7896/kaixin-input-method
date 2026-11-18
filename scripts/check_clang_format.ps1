param(
    [switch]$Fix
)

$ErrorActionPreference = "Stop"

$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$clangFormat = Join-Path $repo 'dist\dev-tools\python\clang_format\data\bin\clang-format.exe'
if (-not (Test-Path -LiteralPath $clangFormat)) {
    $command = Get-Command clang-format -ErrorAction SilentlyContinue
    if (-not $command) { throw 'clang-format 20.1.8 is required. Run scripts/setup_dev_tools.ps1.' }
    $clangFormat = $command.Source
}
$version = & $clangFormat --version
if ($version -notmatch 'version 20\.1\.8\b') { throw "Expected clang-format 20.1.8, found $version" }
$extensions = @(".c", ".cpp", ".h", ".hpp", ".ipp")
$paths = @(
    (Join-Path $repo "tsf-tip\src"),
    (Join-Path $repo "tsf-tip\include"),
    (Join-Path $repo "tsf-tip\tests")
)

$files = Get-ChildItem -Path $paths -Recurse -File |
    Where-Object {
        $extensions -contains $_.Extension.ToLowerInvariant() -and
        $_.Name -notlike '*.generated.h' -and $_.Name -ne 'config_schema.h'
    } |
    Sort-Object FullName

if (-not $files) {
    Write-Host "clang-format check passed: 0 C++ files found"
    exit 0
}

$filePaths = @($files | ForEach-Object { $_.FullName })
if ($Fix) {
    & $clangFormat -i --style=file @filePaths
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Host "clang-format applied: $($filePaths.Count) C++ file(s)"
    exit 0
}

& $clangFormat --dry-run --Werror --style=file @filePaths
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Host "clang-format check passed: $($filePaths.Count) C++ file(s)"
