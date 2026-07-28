param(
    [switch]$Fix
)

$ErrorActionPreference = "Stop"

$clangFormat = Get-Command clang-format -ErrorAction SilentlyContinue
if (-not $clangFormat) {
    Write-Warning "Skipping clang-format check: clang-format was not found on PATH"
    exit 0
}

$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$extensions = @(".c", ".cpp", ".h", ".hpp", ".ipp")
$paths = @(
    (Join-Path $repo "tsf-tip\src"),
    (Join-Path $repo "tsf-tip\include")
)

$files = Get-ChildItem -Path $paths -Recurse -File |
    Where-Object { $extensions -contains $_.Extension.ToLowerInvariant() } |
    Sort-Object FullName

if (-not $files) {
    Write-Host "clang-format check passed: 0 C++ files found"
    exit 0
}

$filePaths = @($files | ForEach-Object { $_.FullName })
if ($Fix) {
    & $clangFormat.Source -i --style=file @filePaths
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Host "clang-format applied: $($filePaths.Count) C++ file(s)"
    exit 0
}

& $clangFormat.Source --dry-run --Werror --style=file @filePaths
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Host "clang-format check passed: $($filePaths.Count) C++ file(s)"
