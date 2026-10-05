param(
    [switch]$Fast,
    [switch]$SkipEval,
    [switch]$SkipTsfBuild,
    [string]$EvalLexiconDir,
    [int]$PerfMaxP99Us = 40000
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
    $repoPrefix = $repoFull.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar
    if (-not $pathFull.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
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
if ([string]::IsNullOrWhiteSpace($EvalLexiconDir)) {
    $EvalLexiconDir = Join-Path $repo "lexicon"
}
$EvalLexiconDir = [System.IO.Path]::GetFullPath($EvalLexiconDir)
if ($PerfMaxP99Us -lt 1) { throw "PerfMaxP99Us must be positive" }

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

$previousVerifyLocalAppData = $env:LOCALAPPDATA
$env:LOCALAPPDATA = Join-Path $repo ("dist\verification-data\" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $env:LOCALAPPDATA | Out-Null
Push-Location $cargoDir
try {
    Invoke-Checked { cargo fmt "--" --check }
    Invoke-Checked { cargo test --workspace --locked --lib }
    Invoke-Checked { cargo test --locked --no-default-features --features dev-tools --bin learning_replay_eval --bin input_perf }
    Invoke-Checked {
        $clippyArgs = @("clippy", "--locked", "--lib", "--", "-D", "warnings", "-A", "clippy::too-many-arguments", "-A", "clippy::manual-is-multiple-of", "-A", "clippy::incompatible-msrv")
        & cargo @clippyArgs
    }
    if ((-not $Fast) -and (-not $SkipEval)) {
        Invoke-Checked { python (Join-Path $repo "scripts\check_lexicon_syllables.py") --check-syllable-count --strict-syllable-count }
        if (-not (Test-Path -LiteralPath (Join-Path $EvalLexiconDir "cold_lexicon.sqlite"))) {
            if ($EvalLexiconDir -ne (Join-Path $repo "lexicon")) {
                throw "Runtime evaluation requires cold_lexicon.sqlite: $EvalLexiconDir"
            }
            Invoke-Checked { powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repo "scripts\prebake_lexicon.ps1") }
        }
        $previousEvalLocalAppData = $env:LOCALAPPDATA
        $env:LOCALAPPDATA = Join-Path $repo ("dist\verification-data\" + [guid]::NewGuid().ToString("N"))
        New-Item -ItemType Directory -Force -Path $env:LOCALAPPDATA | Out-Null
        try {
        Invoke-Checked {
            cargo build --release --locked --no-default-features --features dev-tools --bin input_perf --bin phrase_len_eval --bin learning_replay_eval
        }
        $cargoMetadata = cargo metadata --no-deps --format-version 1 --locked | ConvertFrom-Json
        if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed" }
        $evalBinDir = Join-Path $cargoMetadata.target_directory "release"
        $perfExe = Join-Path $evalBinDir "input_perf.exe"
        $phraseEvalExe = Join-Path $evalBinDir "phrase_len_eval.exe"
        $learningEvalExe = Join-Path $evalBinDir "learning_replay_eval.exe"
        Invoke-Checked {
            $perfArgs = @("--engine-profile", "runtime-hot-cold", "--lexicon", $EvalLexiconDir, "--input", "shuru", "--input", "nihao", "--input", "zhongguo", "--workload-size", "240", "--warmup", "2", "--iterations", "3", "--max-p99-us", "$PerfMaxP99Us")
            & $perfExe @perfArgs
        }
        foreach ($mode in @("post-feedback", "post-commit")) {
            Invoke-Checked {
                $perfArgs = @("--mode", $mode, "--engine-profile", "runtime-hot-cold", "--lexicon", $EvalLexiconDir, "--input", "shuru", "--input", "nihao", "--input", "zhongguo", "--workload-size", "120", "--warmup", "2", "--iterations", "3", "--max-p99-us", "$PerfMaxP99Us")
                & $perfExe @perfArgs
            }
        }
        # Candidate quality gate: validate full-pinyin TOP9 recall and the
        # unrecalled rate against tests/popular_three_char_cases.tsv.
        Invoke-Checked {
            $evalArgs = @("--engine-profile", "runtime-hot-cold", "--lexicon", $EvalLexiconDir, "--limit", "400", "--min-full-top9", "60", "--max-full-unrecalled", "20")
            & $phraseEvalExe @evalArgs
        }
        # Do not let the mixed 2/3/4-character aggregate hide a three-character
        # regression in the heldout benchmark.
        Invoke-Checked {
            $threeCharArgs = @("--engine-profile", "runtime-hot-cold", "--lexicon", $EvalLexiconDir, "--only-popular-three", "--min-full-top9", "66", "--max-full-unrecalled", "16")
            & $phraseEvalExe @threeCharArgs
        }
        # Learning replay gate: measure ranking after commit/select events.
        Invoke-Checked {
            $learningArgs = @("--engine-profile", "runtime-hot-cold", "--lexicon", $EvalLexiconDir, "--min-top1", "70", "--max-missing", "0")
            & $learningEvalExe @learningArgs
        }
        } finally {
            $env:LOCALAPPDATA = $previousEvalLocalAppData
        }
    }
} finally {
    Pop-Location
    $env:LOCALAPPDATA = $previousVerifyLocalAppData
}

if ((-not $Fast) -and (-not $SkipTsfBuild)) {
    Push-Location $repo
    try {
        $x64Build = "tsf-tip\build-codex-current-x64"
        $x86Build = "tsf-tip\build-codex-current-x86"
        Ensure-CMakeBuildDir -BuildDir $x64Build -Arch "x64"
        Invoke-Checked { cmake --build $x64Build --parallel 1 --config Release --target srf_tsf_tip srf_ime_overlay srf_policy_tests srf_game_config_tests srf_security_io_tests -- /nodeReuse:false }
        Invoke-Checked { ctest --test-dir $x64Build -C Release --output-on-failure }
        Ensure-CMakeBuildDir -BuildDir $x86Build -Arch "Win32"
        Invoke-Checked { cmake --build $x86Build --parallel 1 --config Release --target srf_tsf_tip srf_ime_overlay srf_policy_tests srf_game_config_tests srf_security_io_tests -- /nodeReuse:false }
        Invoke-Checked { ctest --test-dir $x86Build -C Release --output-on-failure }
    } finally {
        Pop-Location
    }
}
