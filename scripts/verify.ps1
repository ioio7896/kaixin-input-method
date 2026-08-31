param(
    [switch]$Fast,
    [switch]$SkipEval,
    [switch]$SkipTsfBuild
)

$ErrorActionPreference = "Stop"

foreach ($stream in @([Console]::OutputEncoding, [Console]::InputEncoding)) {
    $null = $stream
}
[Console]::InputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$env:PYTHONIOENCODING = "utf-8"

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)]
        [scriptblock]$Command
    )

    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "command exited with code $LASTEXITCODE"
    }
}

function Assert-PathUnderRepo {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $repoFull = [System.IO.Path]::GetFullPath($repo)
    $pathFull = [System.IO.Path]::GetFullPath($Path)
    if (-not $pathFull.StartsWith($repoFull, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "refusing to remove path outside repo: $pathFull"
    }
}

function Ensure-CMakeBuildDir {
    param(
        [Parameter(Mandatory = $true)]
        [string]$BuildDir,
        [Parameter(Mandatory = $true)]
        [string]$Arch
    )

    $sourceDir = Join-Path $repo "tsf-tip"
    $cache = Join-Path $BuildDir "CMakeCache.txt"
    if (Test-Path $cache) {
        $homeMatch = Select-String -Path $cache -Pattern "^CMAKE_HOME_DIRECTORY:INTERNAL=" | Select-Object -First 1
        if ($homeMatch) {
            $cachedSource = $homeMatch.Line.Substring($homeMatch.Line.IndexOf("=") + 1)
            $expectedSource = (Resolve-Path $sourceDir).Path
            $cachedFull = [System.IO.Path]::GetFullPath($cachedSource)
            $expectedFull = [System.IO.Path]::GetFullPath($expectedSource)
            if ($cachedFull -ne $expectedFull) {
                Assert-PathUnderRepo $BuildDir
                Write-Host "Cleaning stale CMake cache: $BuildDir"
                Remove-Item -LiteralPath $BuildDir -Recurse -Force
            }
        }
    }

    if (-not (Test-Path (Join-Path $BuildDir "CMakeCache.txt"))) {
        Invoke-Checked { cmake -S tsf-tip -B $BuildDir -A $Arch }
    }
}

$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$cargoDir = Join-Path $repo "pinyin-ime"

Push-Location $repo
try {
    if (-not $Fast) {
        Invoke-Checked { python scripts/check_utf8_sources.py }
        Invoke-Checked { python scripts/check_shared_rules.py }
        Invoke-Checked { python scripts/check_package_manifest.py }
        Invoke-Checked { powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check_installer_sources.ps1 }
        Invoke-Checked { powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check_clang_format.ps1 }
    }
} finally {
    Pop-Location
}

Push-Location $cargoDir
try {
    Invoke-Checked { cargo fmt -- --check }
    if ((-not $Fast) -and (-not $SkipEval)) {
        Invoke-Checked { python scripts/check_lexicon_syllables.py --check-syllable-count --strict-syllable-count }
        Invoke-Checked {
            cargo run --bin input_perf -- --input shuru --input nihao --input zhongguo `
                --workload-size 240 --warmup 1 --iterations 1
        }
        # 候选质量门禁：常用三字词基准（tests/popular_three_char_cases.tsv）按
        # 全拼输入 TOP9 召回与不可召回率检查候选排序质量。
        Invoke-Checked {
            cargo run --release --bin phrase_len_eval -- --limit 400 `
                --min-full-top9 60 --max-full-unrecalled 20
        }
        # Do not let the mixed 2/3/4-character aggregate hide a three-character
        # regression in the heldout benchmark.
        Invoke-Checked {
            cargo run --release --bin phrase_len_eval -- --only-popular-three `
                --min-full-top9 66 --max-full-unrecalled 16
        }
        # 学习回放门禁：commit/select 事件后同一输入的前置表现。
        Invoke-Checked {
            cargo run --release --bin learning_replay_eval -- `
                --min-top1 70 --max-missing 0
        }
    }
} finally {
    Pop-Location
}

if ((-not $Fast) -and (-not $SkipTsfBuild)) {
    Push-Location $repo
    try {
        $x64Build = "tsf-tip\build-codex-current-x64"
        $x86Build = "tsf-tip\build-codex-current-x86"
        Ensure-CMakeBuildDir -BuildDir $x64Build -Arch "x64"
        Invoke-Checked { cmake --build $x64Build --config Release --target srf_tsf_tip }
        Ensure-CMakeBuildDir -BuildDir $x86Build -Arch "Win32"
        Invoke-Checked { cmake --build $x86Build --config Release --target srf_tsf_tip }
    } finally {
        Pop-Location
    }
}
