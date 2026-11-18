param(
    [switch]$Fast,
    [switch]$SkipEval,
    [switch]$SkipTsfBuild,
    [string]$EvalLexiconDir,
    [int]$PerfMaxP99Us = 40000
)

$ErrorActionPreference = "Stop"

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

$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$cargoDir = Join-Path $repo "pinyin-ime"
if ([string]::IsNullOrWhiteSpace($EvalLexiconDir)) {
    $EvalLexiconDir = Join-Path $repo "lexicon"
}
$EvalLexiconDir = [System.IO.Path]::GetFullPath($EvalLexiconDir)
if ($PerfMaxP99Us -lt 1) { throw "PerfMaxP99Us must be positive" }

Push-Location $repo
try {
    Invoke-Checked { python scripts/verify.py source }
    Invoke-Checked { python scripts/verify.py rust }
} finally {
    Pop-Location
}

$previousVerifyLocalAppData = $env:LOCALAPPDATA
$env:LOCALAPPDATA = Join-Path $repo ("dist\verification-data\" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $env:LOCALAPPDATA | Out-Null
Push-Location $cargoDir
try {
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
        Invoke-Checked { python scripts/verify.py native }
    } finally {
        Pop-Location
    }
}
