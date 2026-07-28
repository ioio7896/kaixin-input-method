# 在词库目录内生成 lexicon.bin（SRFLX002），供运行时跳过 txt/yaml 解析。
# 用法：在仓库根目录执行  .\scripts\prebake_lexicon.ps1
# 需已安装 Rust；会先 cargo build --release bake_lexicon，再写入 <词库目录>\lexicon.bin
# 词库目录解析与 tsf-tip/installer/stage_package.ps1 中 Get-LexiconDirectory 一致。
# 会生成 lexicon.bin（standard/full）和 hot_lexicon.bin（hot）。

param(
    [ValidateSet('standard', 'full')]
    [string]$LexiconProfile = 'standard'
)

$ErrorActionPreference = 'Stop'

function Get-LexiconRootDirectory {
    param([string]$RepoRoot)

    $fixed = Join-Path $RepoRoot 'lexicon'
    if (Test-Path -LiteralPath $fixed) {
        return (Get-Item -LiteralPath $fixed).FullName
    }

    throw "未找到词库目录。请在仓库根创建 lexicon。"
}

$RepoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$LexiconDir = Get-LexiconRootDirectory -RepoRoot $RepoRoot

$CargoToml = Join-Path $RepoRoot 'pinyin-ime\Cargo.toml'
if (-not (Test-Path -LiteralPath $CargoToml)) {
    throw "找不到 pinyin-ime\Cargo.toml"
}

Push-Location (Join-Path $RepoRoot 'pinyin-ime')
try {
    cargo build --release --bin bake_lexicon
    if ($LASTEXITCODE -ne 0) { throw "cargo build bake_lexicon 失败" }
} finally {
    Pop-Location
}

$BakeExe = Join-Path $RepoRoot 'pinyin-ime\target\release\bake_lexicon.exe'
if (-not (Test-Path -LiteralPath $BakeExe)) {
    throw "未找到 $BakeExe"
}

$OutBin = Join-Path $LexiconDir 'lexicon.bin'
& $BakeExe $LexiconDir $OutBin --profile $LexiconProfile
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $OutBin)) {
    throw "bake_lexicon 失败: $OutBin"
}

$HotOutBin = Join-Path $LexiconDir 'hot_lexicon.bin'
& $BakeExe $LexiconDir $HotOutBin --profile hot
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $HotOutBin)) {
    throw "bake_lexicon hot 失败: $HotOutBin"
}

Write-Output "已写入: $OutBin ($LexiconProfile)"
Write-Output "已写入: $HotOutBin (hot)"
