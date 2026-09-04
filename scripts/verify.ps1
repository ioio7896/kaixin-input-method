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
    Invoke-Checked { cargo fmt "--" --check }
    Invoke-Checked { cargo test --locked --lib }
    Invoke-Checked {
        $clippyArgs = @("clippy", "--locked", "--lib", "--", "-D", "warnings", "-A", "clippy::too-many-arguments", "-A", "clippy::manual-is-multiple-of", "-A", "clippy::incompatible-msrv")
        & cargo @clippyArgs
    }
    if ((-not $Fast) -and (-not $SkipEval)) {
        Invoke-Checked { python scripts/check_lexicon_syllables.py --check-syllable-count --strict-syllable-count }
        Invoke-Checked {
            $perfArgs = @("run", "--bin", "input_perf", "--", "--input", "shuru", "--input", "nihao", "--input", "zhongguo", "--workload-size", "240", "--warmup", "1", "--iterations", "1")
            & cargo @perfArgs
        }
        # Candidate quality gate: validate full-pinyin TOP9 recall and the
        # unrecalled rate against tests/popular_three_char_cases.tsv.
        Invoke-Checked {
            $evalArgs = @("run", "--release", "--bin", "phrase_len_eval", "--", "--limit", "400", "--min-full-top9", "60", "--max-full-unrecalled", "20")
            & cargo @evalArgs
        }
        # Do not let the mixed 2/3/4-character aggregate hide a three-character
        # regression in the heldout benchmark.
        Invoke-Checked {
            $threeCharArgs = @("run", "--release", "--bin", "phrase_len_eval", "--", "--only-popular-three", "--min-full-top9", "66", "--max-full-unrecalled", "16")
            & cargo @threeCharArgs
        }
        # Learning replay gate: measure ranking after commit/select events.
        Invoke-Checked {
            $learningArgs = @("run", "--release", "--bin", "learning_replay_eval", "--", "--min-top1", "70", "--max-missing", "0")
            & cargo @learningArgs
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
        Invoke-Checked { cmake --build $x64Build --config Release --target srf_tsf_tip srf_policy_tests }
        Invoke-Checked { ctest --test-dir $x64Build -C Release --output-on-failure }
        Ensure-CMakeBuildDir -BuildDir $x86Build -Arch "Win32"
        Invoke-Checked { cmake --build $x86Build --config Release --target srf_tsf_tip }
    } finally {
        Pop-Location
    }
}
