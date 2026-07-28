$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot
$CargoDir = Join-Path $RepoRoot 'pinyin-ime'

Push-Location $CargoDir
try {
    cargo run --locked --offline --bin build_clean_lexicons
    if ($LASTEXITCODE -ne 0) {
        throw "build_clean_lexicons failed with exit code $LASTEXITCODE"
    }
}
finally {
    Pop-Location
}

Write-Host 'Clean lexicons and the pinyin-supported character table were regenerated.'
