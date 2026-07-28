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
    Invoke-Checked { cargo test --lib }
    Invoke-Checked { cargo run --bin input_smoke -- ..\tests\input_cases.sqlite }
    if ((-not $Fast) -and (-not $SkipEval)) {
        Invoke-Checked { cargo run --bin input_eval -- --mixed }
        Invoke-Checked { cargo run --bin input_perf -- --workload-size 240 --warmup 1 --iterations 1 }
        Invoke-Checked { cargo run --bin input_eval -- --quality --min-top1 85 --min-top3 85 --min-top9 85 }
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
