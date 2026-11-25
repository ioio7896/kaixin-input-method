param(
    [Parameter(Mandatory)]
    [ValidateSet('Check', 'Preflight', 'Version', 'RustTests', 'Native', 'Stage', 'Perf', 'Publish', 'Skins', 'CollectLogs', 'RegenerateLexicons')]
    [string]$Action,
    [ValidateSet('Debug', 'Release')]
    [string]$Profile = 'Release',
    [string]$Version,
    [string]$Stamp,
    [string]$Work,
    [string]$Commit,
    [ValidateSet('ime', 'ocr')]
    [string]$Variant,
    [ValidateSet('ime', 'ocr')]
    [string]$PreparedVariant,
    [string]$PackageTool,
    [string]$Verification,
    [int]$MaxP99,
    [int]$MaxIncrementalP99,
    [switch]$Verify,
    [switch]$Smoke,
    [switch]$Portable,
    [switch]$Sign,
    [switch]$SignRequired,
    [switch]$UserInstaller,
    [switch]$NativeBuild,
    [switch]$Installers,
    [switch]$RuntimeProfile
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'release-process.ps1')
. (Join-Path $PSScriptRoot 'release-files.ps1')
. (Join-Path $PSScriptRoot 'release-build.ps1')
. (Join-Path $PSScriptRoot 'release-validation.ps1')
. (Join-Path $PSScriptRoot 'package-manifest.ps1')

$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$dist = Join-Path $repo 'dist'
$bin = Join-Path $repo "pinyin-ime/target/$($Profile.ToLowerInvariant())"

switch ($Action) {
    'Check' {
        Assert-ReleaseSourceEncoding -Repo $repo -Git (Find-ReleaseTool -Name 'git.exe')
        & (Join-Path $PSScriptRoot 'check_installer_sources.ps1')
    }
    'Preflight' {
        if ($NativeBuild) { $null = Get-ReleaseCMake }
        if ($Installers) {
            $null = Get-ReleaseInnoSetup
            $null = Get-ReleaseSigning -Requested ([bool]$Sign) -Required ([bool]$SignRequired)
        }
    }
    'Version' { Set-ReleaseVersion -Repo $repo -Version $Version }
    'RustTests' {
        Invoke-ReleaseRustTests -Repo $repo -Cargo (Find-ReleaseTool -Name 'cargo.exe') -Work $Work
    }
    'Native' {
        Invoke-ReleaseNativeBuild -Repo $repo -CMake (Get-ReleaseCMake) -Profile $Profile -SkipTests (-not $Verify)
    }
    'Skins' {
        Invoke-ReleaseCommand -Program (Find-ReleaseTool -Name 'python.exe') -Arguments @((Join-Path $PSScriptRoot 'build_extras.py'), 'skins')
    }
    'CollectLogs' {
        Invoke-ReleaseCommand -Program (Find-ReleaseTool -Name 'python.exe') -Arguments @((Join-Path $PSScriptRoot 'build_extras.py'), 'logs')
    }
    'RegenerateLexicons' { & (Join-Path $PSScriptRoot 'regenerate_lexicons.ps1') }
    'Perf' {
        $lexicon = Join-Path $dist "kaixin-package-$Variant/lexicon"
        Invoke-IsolatedReleaseTest -Directory (Join-Path $Work 'input-perf-user') -Command {
            $perf = Join-Path $bin 'input_perf.exe'
            $common = @('--lexicon', $lexicon, '--workload-size', '500')
            Invoke-ReleaseCommand -Program $perf -Arguments ($common + @('--warmup', '2', '--iterations', '2', '--max-p99-us', $MaxP99))
            Invoke-ReleaseCommand -Program $perf -Arguments ($common + @('--incremental', '--warmup', '0', '--iterations', '1', '--max-p99-us', $MaxIncrementalP99))
        }
    }
    'Stage' {
        $stage = Join-Path $dist "kaixin-package-$Variant"
        $includeOcr = $Variant -eq 'ocr'
        $stageArgs = @{ OutputRoot = $stage; IncludeOcr = [int]$includeOcr; Profile = $Profile; PackageTool = $PackageTool; FinalizeInBuild = $true }
        $stageArgs.TipDllPath = Join-Path $repo "tsf-tip/build-package/$Profile/srf_tsf_tip.dll"
        $stageArgs.X86TipDllPath = Join-Path $repo "tsf-tip/build-package-x86/$Profile/srf_tsf_tip.dll"
        if ($PreparedVariant) { $stageArgs.PreparedLexiconDir = Join-Path $dist "kaixin-package-$PreparedVariant/lexicon" }
        & (Join-Path $repo 'tsf-tip/installer/stage_package.ps1') @stageArgs
        $runtime = Assert-ReleasePayload -Stage $stage -Version $Version -IncludeOcr $includeOcr
        if ($Smoke) {
            Test-StagedEngine -Repo $repo -Stage $stage -Runtime $runtime -Version $Version -Work $Work -Commit $Commit -Profile $Profile
            if ($RuntimeProfile) {
                Invoke-IsolatedReleaseTest -Directory (Join-Path $Work 'perf-user') -Command {
                    Invoke-ReleaseCommand -Program (Join-Path $bin 'input_perf.exe') -Arguments @('--engine-profile', 'runtime-hot-cold', '--lexicon', (Join-Path $stage 'lexicon'), '--workload-size', '240', '--warmup', '1', '--iterations', '1')
                }
            }
        }
        $signing = Get-ReleaseSigning -Requested ([bool]$Sign) -Required ([bool]$SignRequired)
        $binaries = @(Get-ChildItem -LiteralPath $stage -Filter '*.exe' -File)
        $binaries += @(Get-ChildItem -LiteralPath $runtime -Recurse -File | Where-Object Extension -In '.dll', '.exe')
        foreach ($file in $binaries) { Set-ReleaseFileSignature -Signing $signing -Path $file.FullName }
        $manifest = [ordered]@{
            schema = 1
            version = $Version
            profile = $Profile.ToLowerInvariant()
            package_variant = $Variant
            runtime_payload = $runtime.Substring($stage.Length + 1).Replace('\', '/')
            artifacts = @($binaries | ForEach-Object { @{ path = $_.FullName.Substring($stage.Length + 1).Replace('\', '/'); bytes = $_.Length } })
        }
        [IO.File]::WriteAllText((Join-Path $stage 'build-manifest.json'), ($manifest | ConvertTo-Json -Depth 10))
        Write-StageManifests -Root $stage
        Assert-StageManifests -Root $stage
        if ($Smoke -and $includeOcr) {
            Test-StagedOcr -Repo $repo -Stage $stage -Version $Version -Work $Work
        }
        if ($Portable) {
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            $portablePath = Join-Path $dist "kaixin-portable-$Variant-$Version-$Profile-$Stamp.zip"
            $pending = Join-Path $Work "portable-$Variant.zip"
            [IO.Compression.ZipFile]::CreateFromDirectory($stage, $pending)
            Move-Item -LiteralPath $pending -Destination $portablePath
            Copy-Item -LiteralPath $portablePath -Destination (Join-Path $dist "kaixin-portable-$Variant.zip") -Force
            Write-Host "Portable ZIP: $portablePath"
        }
    }
    'Publish' {
        $installerPaths = @(Invoke-ReleaseInstallers -Repo $repo -Inno (Get-ReleaseInnoSetup) -Pending $Work -Version $Version -Stamp $Stamp -UserInstaller ([bool]$UserInstaller))
        if ($installerPaths.Count -ne 2) { throw 'Expected two installers' }
        $signing = Get-ReleaseSigning -Requested ([bool]$Sign) -Required ([bool]$SignRequired)
        $records = foreach ($installer in $installerPaths) {
            Set-ReleaseFileSignature -Signing $signing -Path $installer
            $file = Get-Item -LiteralPath $installer
            [ordered]@{ name = $file.Name; bytes = $file.Length; sha256 = Get-Sha256Hex -Path $installer }
        }
        $backup = Join-Path $Work "kaixin-source-$Version.zip"
        New-ReleaseSourceBackup -Repo $repo -Git (Find-ReleaseTool -Name 'git.exe') -Path $backup -Version $Version
        $report = [ordered]@{ version = $Version; verification = $Verification; installers = @($records); source_backup = Split-Path -Leaf $backup }
        $reportPath = Join-Path $Work "release-$Version.json"
        [IO.File]::WriteAllText($reportPath, ($report | ConvertTo-Json -Depth 12) + "`n")
        Publish-ReleaseFiles -Repo $repo -Installers $installerPaths -Backup $backup -Report $reportPath -UserInstaller ([bool]$UserInstaller)
        Write-Host "Verified release: $(Join-Path $dist "release-$Version.json")"
    }
}
