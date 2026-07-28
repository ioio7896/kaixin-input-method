# SPDX-License-Identifier: Apache-2.0

param(
    [string]$PythonExecutable = ".venv-rapidocr\Scripts\python.exe"
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$outputDir = Join-Path $repoRoot "docs\licenses\generated"
New-Item -ItemType Directory -Force -Path $outputDir | Out-Null

Push-Location $repoRoot
try {
    & cargo about --version *> $null
    if ($LASTEXITCODE -ne 0) {
        throw "cargo-about is required; install it with: cargo install cargo-about --locked"
    }
    & cargo about generate about.hbs --manifest-path pinyin-ime\Cargo.toml `
        --output-file (Join-Path $outputDir "rust-dependencies.html")
    if ($LASTEXITCODE -ne 0) { throw "cargo-about report generation failed" }

    if (-not (Test-Path -LiteralPath $PythonExecutable)) {
        throw "Python runtime not found: $PythonExecutable"
    }
    & $PythonExecutable scripts\generate_python_license_report.py `
        (Join-Path $outputDir "python-dependencies.md")
    if ($LASTEXITCODE -ne 0) { throw "Python dependency report generation failed" }

    Write-Host "License reports generated in $outputDir"
}
finally {
    Pop-Location
}
