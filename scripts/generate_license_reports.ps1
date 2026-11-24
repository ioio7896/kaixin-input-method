# SPDX-License-Identifier: Apache-2.0

param()

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

    Write-Host "License reports generated in $outputDir"
}
finally {
    Pop-Location
}
