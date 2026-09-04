$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot
$CargoDir = Join-Path $RepoRoot 'pinyin-ime'

Push-Location $CargoDir
try {
    cargo run --locked --offline --bin build_ai_lexicons
    if ($LASTEXITCODE -ne 0) {
        throw "build_ai_lexicons failed with exit code $LASTEXITCODE"
    }
}
finally {
    Pop-Location
}

$LexiconDir = Join-Path $RepoRoot 'lexicon'
$CoreDir = Join-Path $LexiconDir 'core'
$LegacyExtDir = Join-Path $LexiconDir 'zh-ext'
New-Item -ItemType Directory -Force -Path $CoreDir | Out-Null
foreach ($Name in @('kaixin_explicit.txt', 'kaixin_polyphone.txt', 'kaixin_pronunciation_aliases.txt')) {
    $LegacyPath = Join-Path $LegacyExtDir $Name
    if (Test-Path -LiteralPath $LegacyPath) {
        Move-Item -LiteralPath $LegacyPath -Destination (Join-Path $CoreDir $Name) -Force
    }
}

python (Join-Path $PSScriptRoot 'merge_zh_ext_lexicons.py')
if ($LASTEXITCODE -ne 0) {
    throw "merge_zh_ext_lexicons.py failed with exit code $LASTEXITCODE"
}

python (Join-Path $PSScriptRoot 'build_short_hot_lexicons.py')
if ($LASTEXITCODE -ne 0) {
    throw "build_short_hot_lexicons.py failed with exit code $LASTEXITCODE"
}

Write-Host 'AI-maintained lexicons and the pinyin-supported character table were regenerated.'
