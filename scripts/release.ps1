param(
    [switch]$Sign,
    [switch]$SignRequired,
    [switch]$SkipTests,
    [switch]$UserInstaller,
    [switch]$Check,
    [switch]$Help
)

$ErrorActionPreference = 'Stop'
foreach ($option in $args) {
    switch ($option) {
        '--sign' { $Sign = $true }
        '--sign-required' { $SignRequired = $true }
        '--skip-tests' { $SkipTests = $true }
        '--help' { $Help = $true }
        default { throw "Unknown release option: $option" }
    }
}
if ($Help) {
    Write-Host 'release.ps1 [-Sign] [-SignRequired] [-SkipTests] [-UserInstaller] [-Check]'
    Write-Host 'Default: verify and build two timestamped installers and a source ZIP.'
    Write-Host '-Check: validate source and lexicon preparation without compiling installers.'
    exit 0
}

. (Join-Path $PSScriptRoot 'release-process.ps1')
. (Join-Path $PSScriptRoot 'release-files.ps1')
. (Join-Path $PSScriptRoot 'release-build.ps1')
. (Join-Path $PSScriptRoot 'release-validation.ps1')
. (Join-Path $PSScriptRoot 'package-manifest.ps1')

$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$dist = Join-Path $repo 'dist'
$target = Join-Path $repo 'pinyin-ime/target'
$bin = Join-Path $target 'release'
$tool = Join-Path $bin 'kaixin-release.exe'
$oldTarget = $env:CARGO_TARGET_DIR
$oldFlags = $env:CARGO_ENCODED_RUSTFLAGS
[IO.Directory]::CreateDirectory($dist) | Out-Null
$lock = $null
$work = $null
$completed = $false
Push-Location -LiteralPath $repo
try {
    $lock = [IO.File]::Open((Join-Path $dist '.release.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
    $git = Find-ReleaseTool -Name 'git.exe'
    $cargo = Find-ReleaseTool -Name 'cargo.exe'
    $rustc = Find-ReleaseTool -Name 'rustc.exe'
    $root = (Invoke-ReleaseCapture -Program $git -Arguments @('rev-parse', '--show-toplevel') -Directory $repo).Trim()
    if ([IO.Path]::GetFullPath($root) -ne $repo) { throw 'Build from a Git clone of the project' }
    $commit = (Invoke-ReleaseCapture -Program $git -Arguments @('rev-parse', '--short=12', 'HEAD') -Directory $repo).Trim()
    $toolchain = [IO.File]::ReadAllText((Join-Path $repo 'release-toolchain.json')) | ConvertFrom-Json
    foreach ($compiler in @(@($cargo, $toolchain.cargo), @($rustc, $toolchain.rustc))) {
        $installed = (Invoke-ReleaseCapture -Program $compiler[0] -Arguments @('--version') -Directory $repo).Trim()
        if (($installed -split ' ')[1] -ne $compiler[1]) { throw "Expected version $($compiler[1]); found $installed" }
    }
    $hostTarget = Invoke-ReleaseCapture -Program $rustc -Arguments @('-vV') -Directory $repo
    if ($hostTarget -notmatch '(?m)^host: x86_64-pc-windows-msvc\r?$') { throw 'The release requires x64 Windows MSVC Rust' }

    $env:CARGO_TARGET_DIR = $target
    if (-not $SkipTests -or $Check) {
        Invoke-ReleaseCommand -Program $cargo -Arguments @('fmt', '--all', '--', '--check')
        Assert-ReleaseSourceEncoding -Repo $repo -Git $git
        & (Join-Path $PSScriptRoot 'check_installer_sources.ps1')
    }
    if (-not $Check) {
        $cmake = Get-ReleaseCMake
        $inno = Get-ReleaseInnoSetup
        $signing = Get-ReleaseSigning -Requested ([bool]$Sign) -Required ([bool]$SignRequired)
        $shanghai = [DateTimeOffset]::UtcNow.ToOffset([TimeSpan]::FromHours(8))
        $version = '{0}.{1}.{2}' -f $shanghai.Year, [int]$shanghai.ToString('MMdd'), [int]$shanghai.ToString('HHmm')
        $stamp = $shanghai.ToString('yyyyMMdd-HHmmss')
        Set-ReleaseVersion -Repo $repo -Version $version
        Write-Host "Release version $version"
        $work = Join-Path $dist ('.release-work-' + [guid]::NewGuid().ToString('N'))
        [IO.Directory]::CreateDirectory($work) | Out-Null
    }

    Invoke-ReleaseCommand -Program $cargo -Arguments @('build', '--locked', '--release', '-p', 'kaixin-release')
    Invoke-ReleaseCommand -Program $tool -Arguments @('corpus', $repo)
    Invoke-ReleaseCommand -Program $tool -Arguments @('check-lexicons', $repo)
    if ($Check) {
        Write-Host 'Source and lexicon preparation passed'
        $completed = $true
        return
    }
    if (-not $SkipTests) { Invoke-ReleaseRustTests -Repo $repo -Cargo $cargo -Work $work }
    Invoke-ReleaseNativeBuild -Repo $repo -CMake $cmake -SkipTests ([bool]$SkipTests)
    Set-ReleaseRustFlags -Repo $repo
    Invoke-ReleaseCommand -Program $cargo -Arguments @('build', '--locked', '--release', '-p', 'pinyin-ime', '--no-default-features', '--features', 'dev-tools', '--bin', 'srf_ime_engine', '--bin', 'srf_ime_setup', '--bin', 'srf_ime_clipboard_svc', '--bin', 'bake_lexicon')
    Invoke-ReleaseCommand -Program $cargo -Arguments @('build', '--locked', '--release', '-p', 'pinyin-ime', '--features', 'dev-tools', '--bin', 'srf_ime_settings', '--bin', 'srf_ime_tray', '--bin', 'srf_ime_clipboard', '--bin', 'srf_ime_handwrite', '--bin', 'srf_ime_symbols', '--bin', 'srf_ime_ocr', '--bin', 'input_perf')
    $stageScript = Join-Path $repo 'tsf-tip/installer/stage_package.ps1'
    $prepared = $null
    foreach ($variant in 'ime', 'ocr') {
        $stage = Join-Path $dist "kaixin-package-$variant"
        $includeOcr = $variant -eq 'ocr'
        $stageArgs = @{ OutputRoot = $stage; IncludeOcr = [int]$includeOcr; Profile = 'Release'; PackageTool = $tool; FinalizeInBuild = $true }
        if ($prepared) { $stageArgs.PreparedLexiconDir = $prepared }
        & $stageScript @stageArgs
        $runtime = Assert-ReleasePayload -Stage $stage -Version $version -IncludeOcr $includeOcr
        if (-not $SkipTests) {
            Test-StagedEngine -Repo $repo -Stage $stage -Runtime $runtime -Version $version -Work $work -Commit $commit
            if ($includeOcr) {
                Test-StagedOcr -Repo $repo -Stage $stage -Version $version -Work $work
            } else {
                Invoke-IsolatedReleaseTest -Directory (Join-Path $work 'perf-user') -Command {
                    Invoke-ReleaseCommand -Program (Join-Path $bin 'input_perf.exe') -Arguments @('--engine-profile', 'runtime-hot-cold', '--lexicon', (Join-Path $stage 'lexicon'), '--workload-size', '240', '--warmup', '1', '--iterations', '1')
                }
            }
        }
        $binaries = @(Get-ChildItem -LiteralPath $stage -Filter '*.exe' -File)
        $binaries += @(Get-ChildItem -LiteralPath $runtime -Recurse -File | Where-Object Extension -In '.dll', '.exe')
        foreach ($file in $binaries) { Set-ReleaseFileSignature -Signing $signing -Path $file.FullName }
        $manifest = [ordered]@{
            schema = 1
            version = $version
            profile = 'release'
            package_variant = $variant
            runtime_payload = $runtime.Substring($stage.Length + 1).Replace('\', '/')
            artifacts = @($binaries | ForEach-Object { @{ path = $_.FullName.Substring($stage.Length + 1).Replace('\', '/'); bytes = $_.Length } })
        }
        [IO.File]::WriteAllText((Join-Path $stage 'build-manifest.json'), ($manifest | ConvertTo-Json -Depth 10))
        Write-StageManifests -Root $stage
        Assert-StageManifests -Root $stage
        if (-not $prepared) { $prepared = Join-Path $stage 'lexicon' }
    }

    $installers = @(Invoke-ReleaseInstallers -Repo $repo -Inno $inno -Pending $work -Version $version -Stamp $stamp -UserInstaller ([bool]$UserInstaller))
    if ($installers.Count -ne 2) { throw 'Expected two installers' }
    $records = foreach ($installer in $installers) {
        Set-ReleaseFileSignature -Signing $signing -Path $installer
        $file = Get-Item -LiteralPath $installer
        [ordered]@{ name = $file.Name; bytes = $file.Length; sha256 = Get-Sha256Hex -Path $installer }
    }
    $backup = Join-Path $work "kaixin-source-$version.zip"
    New-ReleaseSourceBackup -Repo $repo -Git $git -Path $backup -Version $version
    $verification = if ($SkipTests) { 'skipped_by_request' } else { 'passed' }
    $report = [ordered]@{ version = $version; verification = $verification; installers = @($records); source_backup = Split-Path -Leaf $backup }
    $reportPath = Join-Path $work "release-$version.json"
    [IO.File]::WriteAllText($reportPath, ($report | ConvertTo-Json -Depth 12) + "`n")
    Publish-ReleaseFiles -Repo $repo -Installers $installers -Backup $backup -Report $reportPath -UserInstaller ([bool]$UserInstaller)
    $completed = $true
    Write-Host "Verified release: $(Join-Path $dist "release-$version.json")"
} finally {
    $env:CARGO_TARGET_DIR = $oldTarget
    $env:CARGO_ENCODED_RUSTFLAGS = $oldFlags
    if ($work -and $completed -and (Test-Path -LiteralPath $work -PathType Container)) {
        $full = (Resolve-Path -LiteralPath $work).Path
        $parent = [IO.Path]::GetDirectoryName($full)
        if ($parent -eq $dist -and (Split-Path -Leaf $full) -like '.release-work-*') {
            Remove-Item -LiteralPath $full -Recurse -Force
        }
    }
    if ($lock) { $lock.Dispose() }
    Pop-Location
}
