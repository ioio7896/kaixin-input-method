param(
    [string]$OutputRoot,
    [string]$TipDllPath,
    [string]$OverlayExePath,
    [string]$X86TipDllPath,
    [string]$X86OverlayExePath,
    [switch]$Zip,
    [ValidateSet(0, 1)]
    [int]$IncludeOcr = 1,
    [ValidateSet('Debug', 'Release')]
    [string]$Profile = 'Release',
    [ValidateSet('standard', 'full')]
    [string]$LexiconProfile = 'standard'
)

$ErrorActionPreference = 'Stop'

$PackageManifestName = 'package_manifest.sha256'
$ComponentManifestName = 'component_manifest.ini'
$PythonPackageExcludeNames = @(
    '__pycache__',
    '.pytest_cache',
    '.mypy_cache',
    '.ruff_cache',
    'test',
    'tests',
    'testing',
    'benchmark',
    'benchmarks',
    'example',
    'examples',
    'pip',
    'setuptools'
)
$PythonPackageExcludePatterns = @(
    '*.pyc',
    '*.pyo',
    '*.pdb',
    '*.lib',
    '*.exp',
    '*.h',
    '*.hpp',
    '*.c',
    '*.cpp',
    '*.cxx',
    '*.pxd',
    '*.pyi',
    'pip-*.dist-info',
    'setuptools-*.dist-info'
)

function Get-LexiconDirectory {
    param([string]$RepoRoot)

    $fixed = Join-Path $RepoRoot 'lexicon'
    if (Test-Path -LiteralPath $fixed) {
        return Get-Item -LiteralPath $fixed
    }

    throw 'No lexicon directory found. Expected repo_root\lexicon.'
}

function Resolve-FirstExistingPath {
    param([string[]]$Candidates)

    foreach ($path in $Candidates) {
        if (Test-Path -LiteralPath $path) {
            return $path
        }
    }

    throw "Missing build output. Checked:`n$($Candidates -join "`n")"
}

function Resolve-FirstExistingRuntimePair {
    param([string[]]$TipDllCandidates)

    foreach ($dll in $TipDllCandidates) {
        $overlay = Join-Path (Split-Path -Parent $dll) 'srf_ime_overlay.exe'
        if ((Test-Path -LiteralPath $dll) -and (Test-Path -LiteralPath $overlay)) {
            return [pscustomobject]@{
                TipDll = [System.IO.Path]::GetFullPath($dll)
                OverlayExe = [System.IO.Path]::GetFullPath($overlay)
            }
        }
    }

    throw "Missing co-located TSF/overlay build outputs. Checked:`n$($TipDllCandidates -join "`n")"
}

function Assert-CoLocatedRuntimePair {
    param(
        [string]$TipDll,
        [string]$OverlayExe,
        [string]$Architecture
    )

    $tipDirectory = [System.IO.Path]::GetFullPath((Split-Path -Parent $TipDll))
    $overlayDirectory = [System.IO.Path]::GetFullPath((Split-Path -Parent $OverlayExe))
    if (-not [System.StringComparer]::OrdinalIgnoreCase.Equals($tipDirectory, $overlayDirectory)) {
        throw "$Architecture TSF DLL and overlay must come from the same build directory: $TipDll ; $OverlayExe"
    }
}

function Remove-ItemWithRetry {
    param(
        [string]$Path,
        [int]$Attempts = 5,
        [int]$DelayMilliseconds = 400
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }

    $lastError = $null
    for ($attempt = 1; $attempt -le $Attempts; $attempt++) {
        try {
            Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction Stop
            return
        } catch {
            $lastError = $_
            if ($attempt -lt $Attempts) {
                Start-Sleep -Milliseconds $DelayMilliseconds
            }
        }
    }

    throw $lastError
}

function Copy-FileIfChanged {
    param(
        [string]$SourcePath,
        [string]$DestinationPath
    )

    $destinationDir = Split-Path -Parent $DestinationPath
    if (-not (Test-Path -LiteralPath $destinationDir)) {
        New-Item -ItemType Directory -Path $destinationDir -Force | Out-Null
    }

    $sourceItem = Get-Item -LiteralPath $SourcePath
    if (Test-Path -LiteralPath $DestinationPath) {
        $destinationItem = Get-Item -LiteralPath $DestinationPath
        if ($sourceItem.Length -eq $destinationItem.Length -and
            $sourceItem.LastWriteTimeUtc -eq $destinationItem.LastWriteTimeUtc) {
            return
        }
    }

    Copy-Item -LiteralPath $SourcePath -Destination $DestinationPath -Force
    (Get-Item -LiteralPath $DestinationPath).LastWriteTimeUtc = $sourceItem.LastWriteTimeUtc
}

function Copy-PowerShellScriptUtf8Bom {
    param(
        [string]$SourcePath,
        [string]$DestinationPath
    )

    $destinationDir = Split-Path -Parent $DestinationPath
    if (-not (Test-Path -LiteralPath $destinationDir)) {
        New-Item -ItemType Directory -Path $destinationDir -Force | Out-Null
    }

    $text = [System.IO.File]::ReadAllText($SourcePath, [System.Text.Encoding]::UTF8)
    $utf8Bom = New-Object System.Text.UTF8Encoding($true)
    [System.IO.File]::WriteAllText($DestinationPath, $text, $utf8Bom)
    (Get-Item -LiteralPath $DestinationPath).LastWriteTimeUtc =
        (Get-Item -LiteralPath $SourcePath).LastWriteTimeUtc
}

function Get-Sha256Hex {
    param([string]$Path)

    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $stream = [System.IO.File]::OpenRead($Path)
        try {
            $hashBytes = $sha256.ComputeHash($stream)
        } finally {
            $stream.Dispose()
        }
    } finally {
        $sha256.Dispose()
    }

    return ([System.BitConverter]::ToString($hashBytes)).Replace('-', '').ToLowerInvariant()
}

function Test-ExcludedEntryName {
    param(
        [string]$Name,
        [System.Collections.Generic.HashSet[string]]$ExcludedNames,
        [string[]]$ExcludePatterns
    )

    if ($ExcludedNames.Contains($Name)) {
        return $true
    }
    foreach ($pattern in $ExcludePatterns) {
        if ($Name -like $pattern) {
            return $true
        }
    }
    return $false
}

function Sync-DirectoryContents {
    param(
        [string]$SourceDir,
        [string]$DestinationDir,
        [string[]]$ExcludeNames = @(),
        [string[]]$ExcludePatterns = @()
    )

    if (-not (Test-Path -LiteralPath $DestinationDir)) {
        New-Item -ItemType Directory -Path $DestinationDir -Force | Out-Null
    }

    $excludedNames = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($excludeName in $ExcludeNames) {
        [void]$excludedNames.Add($excludeName)
    }

    $sourceEntries = @(
        Get-ChildItem -LiteralPath $SourceDir -Force |
            Where-Object {
                -not (Test-ExcludedEntryName `
                    -Name $_.Name `
                    -ExcludedNames $excludedNames `
                    -ExcludePatterns $ExcludePatterns)
            }
    )
    $sourceNames = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($entry in $sourceEntries) {
        [void]$sourceNames.Add($entry.Name)
    }

    foreach ($destinationEntry in @(Get-ChildItem -LiteralPath $DestinationDir -Force -ErrorAction SilentlyContinue)) {
        if (-not $sourceNames.Contains($destinationEntry.Name)) {
            Remove-ItemWithRetry -Path $destinationEntry.FullName
        }
    }

    foreach ($sourceEntry in $sourceEntries) {
        $destinationPath = Join-Path $DestinationDir $sourceEntry.Name
        if ($sourceEntry.PSIsContainer) {
            Sync-DirectoryContents `
                -SourceDir $sourceEntry.FullName `
                -DestinationDir $destinationPath `
                -ExcludeNames $ExcludeNames `
                -ExcludePatterns $ExcludePatterns
        } else {
            Copy-FileIfChanged -SourcePath $sourceEntry.FullName -DestinationPath $destinationPath
        }
    }
}

function Write-ThirdPartyDistributionNotices {
    param(
        [string]$SourcePath,
        [string]$DestinationPath
    )

    $text = Get-Content -LiteralPath $SourcePath -Raw -Encoding UTF8
    [System.IO.File]::WriteAllText(
        $DestinationPath,
        $text,
        [System.Text.UTF8Encoding]::new($false)
    )
}

function Get-VenvPythonHome {
    param([string]$VenvDir)

    $cfg = Join-Path $VenvDir 'pyvenv.cfg'
    if (-not (Test-Path -LiteralPath $cfg)) {
        throw "Python venv config not found: $cfg"
    }

    foreach ($line in Get-Content -LiteralPath $cfg -Encoding UTF8) {
        if ($line -match '^\s*home\s*=\s*(.+?)\s*$') {
            $pythonHomeCandidate = $Matches[1].Trim().Trim('"')
            if (-not [System.IO.Path]::IsPathRooted($pythonHomeCandidate)) {
                $pythonHomeCandidate = [System.IO.Path]::GetFullPath((Join-Path $VenvDir $pythonHomeCandidate))
            }
            if (-not (Test-Path -LiteralPath $pythonHomeCandidate)) {
                throw "Python home from $cfg does not exist: $pythonHomeCandidate"
            }
            return [System.IO.Path]::GetFullPath($pythonHomeCandidate)
        }
    }

    throw "Python home is missing from $cfg"
}

function Sync-PythonRuntimeFromVenv {
    param(
        [string]$VenvDir,
        [string]$DestinationDir
    )

    $pythonHome = Get-VenvPythonHome -VenvDir $VenvDir
    $pythonExe = Join-Path $pythonHome 'python.exe'
    if (-not (Test-Path -LiteralPath $pythonExe)) {
        throw "Python runtime executable not found: $pythonExe"
    }

    Sync-DirectoryContents `
        -SourceDir $pythonHome `
        -DestinationDir $DestinationDir `
        -ExcludeNames @(
            'Doc',
            'include',
            'libs',
            'Scripts',
            'tcl',
            'site-packages',
            'test',
            '__pycache__',
            'idlelib',
            'tkinter',
            'turtledemo',
            'ensurepip'
        ) `
        -ExcludePatterns $PythonPackageExcludePatterns

    $runtimePython = Join-Path $DestinationDir 'python.exe'
    $runtimeStdlib = Join-Path $DestinationDir 'Lib\os.py'
    if (-not (Test-Path -LiteralPath $runtimePython)) {
        throw "Staged Python runtime executable is missing: $runtimePython"
    }
    if (-not (Test-Path -LiteralPath $runtimeStdlib)) {
        throw "Staged Python standard library is missing: $runtimeStdlib"
    }
    if (-not (Get-ChildItem -LiteralPath $DestinationDir -Filter 'python*.dll' -File -ErrorAction SilentlyContinue)) {
        throw "Staged Python runtime DLLs are missing: $DestinationDir"
    }
}

function Sync-PythonPackagesFromVenv {
    param(
        [string]$VenvDir,
        [string]$DestinationDir
    )

    $sitePackages = Join-Path $VenvDir 'Lib\site-packages'
    if (-not (Test-Path -LiteralPath $sitePackages)) {
        throw "Python site-packages directory not found: $sitePackages"
    }
    Sync-DirectoryContents `
        -SourceDir $sitePackages `
        -DestinationDir $DestinationDir `
        -ExcludeNames $PythonPackageExcludeNames `
        -ExcludePatterns $PythonPackageExcludePatterns
}

function Write-PackageHashManifest {
    param(
        [string]$Root,
        [string]$ManifestName
    )

    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\')
    $manifestPath = Join-Path $rootFull $ManifestName
    $files = Get-ChildItem -LiteralPath $rootFull -Recurse -File -Force |
        Where-Object { -not $_.FullName.Equals($manifestPath, [System.StringComparison]::OrdinalIgnoreCase) } |
        Sort-Object FullName

    $lines = New-Object 'System.Collections.Generic.List[string]'
    foreach ($file in $files) {
        $full = [System.IO.Path]::GetFullPath($file.FullName)
        $relative = $full.Substring($rootFull.Length + 1).Replace('\', '/')
        $hash = Get-Sha256Hex -Path $full
        [void]$lines.Add(('{0} *{1}' -f $hash, $relative))
    }

    Set-Content -LiteralPath $manifestPath -Value ([string[]]$lines) -Encoding UTF8
}

function Get-StringSha256Hex {
    param([string]$Value)

    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($Value)
        $hashBytes = $sha256.ComputeHash($bytes)
    } finally {
        $sha256.Dispose()
    }
    return ([System.BitConverter]::ToString($hashBytes)).Replace('-', '').ToLowerInvariant()
}

function Write-ComponentManifestFromPackageManifest {
    param(
        [string]$Root,
        [string]$PackageManifest,
        [string]$ComponentManifest
    )

    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\')
    $packageManifestPath = Join-Path $rootFull $PackageManifest
    $componentManifestPath = Join-Path $rootFull $ComponentManifest
    $packageLines = @(Get-Content -LiteralPath $packageManifestPath -Encoding UTF8 | Where-Object {
        -not [string]::IsNullOrWhiteSpace($_) -and -not $_.StartsWith('#')
    })
    $components = [ordered]@{
        python_runtime = '.python-runtime/'
        rapidocr_packages = '.python-packages/'
        rapidocr_payload = 'RapidOCR-3.9.0/'
    }
    $output = New-Object 'System.Collections.Generic.List[string]'
    [void]$output.Add('[components]')
    foreach ($entry in $components.GetEnumerator()) {
        $prefix = $entry.Value
        $matching = @($packageLines | Where-Object {
            $separator = $_.IndexOf(' *')
            $separator -ge 0 -and $_.Substring($separator + 2).StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)
        })
        $componentId = if ($matching.Count -gt 0) {
            Get-StringSha256Hex -Value (($matching -join "`n") + "`n")
        } else {
            'absent'
        }
        [void]$output.Add(('{0}={1}' -f $entry.Key, $componentId))
    }
    Set-Content -LiteralPath $componentManifestPath -Value ([string[]]$output) -Encoding UTF8

    $relative = [System.IO.Path]::GetFileName($componentManifestPath)
    $componentHash = Get-Sha256Hex -Path $componentManifestPath
    Add-Content -LiteralPath $packageManifestPath -Value ('{0} *{1}' -f $componentHash, $relative) -Encoding UTF8
}

$RepoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$TsfTipRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$profileLower = $Profile.ToLowerInvariant()

if (-not $OutputRoot) {
    $OutputRoot = Join-Path $RepoRoot 'dist\kaixin-package'
}
$OutputRoot = [System.IO.Path]::GetFullPath($OutputRoot)

$x64RuntimePair = $null
$tipDll = if ($TipDllPath) {
    if (-not (Test-Path -LiteralPath $TipDllPath)) {
        throw "Missing build output: $TipDllPath"
    }
    [System.IO.Path]::GetFullPath($TipDllPath)
} else {
    $x64RuntimePair = Resolve-FirstExistingRuntimePair -TipDllCandidates @(
        (Join-Path $TsfTipRoot 'build-current\Release\srf_tsf_tip.dll'),
        (Join-Path $TsfTipRoot 'build-current\srf_tsf_tip.dll'),
        (Join-Path $TsfTipRoot 'build-package\Release\srf_tsf_tip.dll'),
        (Join-Path $TsfTipRoot 'build-package\srf_tsf_tip.dll'),
        (Join-Path $TsfTipRoot 'build-codex\Release\srf_tsf_tip.dll'),
        (Join-Path $TsfTipRoot 'build\Release\srf_tsf_tip.dll'),
        (Join-Path $TsfTipRoot 'build\srf_tsf_tip.dll')
    )
    $x64RuntimePair.TipDll
}
$bakeLexiconExe = Join-Path $RepoRoot "pinyin-ime\target\$profileLower\bake_lexicon.exe"
$settingsExe = Join-Path $RepoRoot "pinyin-ime\target\$profileLower\srf_ime_settings.exe"
$trayExe = Join-Path $RepoRoot "pinyin-ime\target\$profileLower\srf_ime_tray.exe"
$engineExe = Join-Path $RepoRoot "pinyin-ime\target\$profileLower\srf_ime_engine.exe"
$clipboardExe = Join-Path $RepoRoot "pinyin-ime\target\$profileLower\srf_ime_clipboard.exe"
$clipboardSvcExe = Join-Path $RepoRoot "pinyin-ime\target\$profileLower\srf_ime_clipboard_svc.exe"
$handwriteExe = Join-Path $RepoRoot "pinyin-ime\target\$profileLower\srf_ime_handwrite.exe"
$ocrExe = Join-Path $RepoRoot "pinyin-ime\target\$profileLower\srf_ime_ocr.exe"
$settingsManifest = Join-Path $PSScriptRoot 'srf_ime_settings.exe.manifest'
$trayManifest = Join-Path $PSScriptRoot 'srf_ime_tray.exe.manifest'
$userDataManifest = Join-Path $PSScriptRoot 'user_data_manifest.json'
$repairInstallScript = Join-Path $PSScriptRoot 'repair_install.ps1'
$versionFile = Join-Path $RepoRoot 'VERSION'
$projectLicenseFile = Join-Path $RepoRoot 'LICENSE'
$licenseScopeFile = Join-Path $RepoRoot 'LICENSE_SCOPE.md'
$projectNoticeFile = Join-Path $RepoRoot 'NOTICE'
$thirdPartyNoticesFile = Join-Path $RepoRoot 'THIRD_PARTY_NOTICES.md'
$generatedLicenseDir = Join-Path $RepoRoot 'docs\licenses\generated'
$distributionLicenseDir = Join-Path $RepoRoot 'docs\licenses\distribution'
$lexiconDir = Get-LexiconDirectory -RepoRoot $RepoRoot
$fontDir = Join-Path $RepoRoot 'font1'
$skinDir = Join-Path $RepoRoot 'skins'
$assetsDir = Join-Path $RepoRoot 'assets'
$toolsDir = Join-Path $RepoRoot 'tools'
$rapidOcrDir = Join-Path $RepoRoot 'RapidOCR-3.9.0'
$rapidOcrVenvDir = Join-Path $RepoRoot '.venv-rapidocr'
$pythonRuntimeDirName = '.python-runtime'
$pythonPackagesDirName = '.python-packages'
$pythonRuntimeSourceVenv = $null
if ($IncludeOcr -and (Test-Path -LiteralPath $rapidOcrVenvDir)) {
    $pythonRuntimeSourceVenv = $rapidOcrVenvDir
}

$requiredOutputs = @($tipDll, $bakeLexiconExe, $settingsExe, $trayExe, $engineExe, $clipboardExe, $clipboardSvcExe, $handwriteExe, $settingsManifest, $trayManifest, $userDataManifest, $repairInstallScript, $versionFile, $projectLicenseFile, $licenseScopeFile, $projectNoticeFile, $thirdPartyNoticesFile)
if ($IncludeOcr) {
    $requiredOutputs += $ocrExe
}
foreach ($path in $requiredOutputs) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Missing build output: $path"
    }
}

$x86RuntimePair = $null
$x86TipDll = if ($X86TipDllPath) {
    if (-not (Test-Path -LiteralPath $X86TipDllPath)) {
        throw "Missing x86 build output: $X86TipDllPath"
    }
    [System.IO.Path]::GetFullPath($X86TipDllPath)
} else {
    $x86RuntimePair = Resolve-FirstExistingRuntimePair -TipDllCandidates @(
        (Join-Path $TsfTipRoot 'build-package-x86\Release\srf_tsf_tip.dll'),
        (Join-Path $TsfTipRoot 'build-package-x86\srf_tsf_tip.dll')
    )
    $x86RuntimePair.TipDll
}
$overlayExe = if ($OverlayExePath) {
    if (-not (Test-Path -LiteralPath $OverlayExePath)) {
        throw "Missing x64 overlay build output: $OverlayExePath"
    }
    [System.IO.Path]::GetFullPath($OverlayExePath)
} else {
    if ($x64RuntimePair) {
        $x64RuntimePair.OverlayExe
    } else {
        Join-Path (Split-Path -Parent $tipDll) 'srf_ime_overlay.exe'
    }
}
$x86OverlayExe = if ($X86OverlayExePath) {
    if (-not (Test-Path -LiteralPath $X86OverlayExePath)) {
        throw "Missing x86 overlay build output: $X86OverlayExePath"
    }
    [System.IO.Path]::GetFullPath($X86OverlayExePath)
} else {
    if ($x86RuntimePair) {
        $x86RuntimePair.OverlayExe
    } else {
        Join-Path (Split-Path -Parent $x86TipDll) 'srf_ime_overlay.exe'
    }
}
foreach ($path in @($x86TipDll, $overlayExe, $x86OverlayExe)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Missing TSF/overlay build output: $path"
    }
}
Assert-CoLocatedRuntimePair -TipDll $tipDll -OverlayExe $overlayExe -Architecture 'x64'
Assert-CoLocatedRuntimePair -TipDll $x86TipDll -OverlayExe $x86OverlayExe -Architecture 'x86'

$tipHash = Get-Sha256Hex -Path $tipDll
$x86TipHash = Get-Sha256Hex -Path $x86TipDll
$packageStamp = (Get-Date).ToUniversalTime().ToString('yyyyMMddHHmmss')
$runtimePayloadId = ('pkg-{0}-tip64-{1}-tip32-{2}' -f $packageStamp, $tipHash.Substring(0, 8), $x86TipHash.Substring(0, 8))
$runtimeRelativePath = Join-Path 'runtime' $runtimePayloadId
$runtimeOutputRoot = Join-Path $OutputRoot $runtimeRelativePath
$runtimeOutputX64 = Join-Path $runtimeOutputRoot 'x64'
$runtimeOutputX86 = Join-Path $runtimeOutputRoot 'x86'

New-Item -ItemType Directory -Path $OutputRoot -Force | Out-Null

$managedRootEntries = @(
    'current_runtime_payload.txt',
    $PackageManifestName,
    $ComponentManifestName,
    'VERSION',
    'install_dev.ps1',
    'install_current_user.ps1',
    'restart_stale_hosts.ps1',
    'invoke_registration.ps1',
    'repair_install.ps1',
    'export_diagnostics.ps1',
    'runtime',
    'srf_ime_settings.exe',
    'srf_ime_settings.exe.manifest',
    'srf_ime_tray.exe',
    'srf_ime_tray.exe.manifest',
    'srf_ime_engine.exe',
    'srf_ime_clipboard.exe',
    'srf_ime_clipboard_svc.exe',
    'srf_ime_handwrite.exe',
    'uninstall_dev.ps1',
    'uninstall_current_user.ps1',
    'user_data_manifest.json',
    'LICENSE',
    'LICENSE_SCOPE.md',
    'NOTICE',
    'licenses',
    'THIRD_PARTY_NOTICES.md',
    $lexiconDir.Name
)
if ($IncludeOcr) {
    $managedRootEntries += 'srf_ime_ocr.exe'
}
if (Test-Path -LiteralPath $fontDir) {
    $managedRootEntries += 'font1'
}
if (Test-Path -LiteralPath $skinDir) {
    $managedRootEntries += 'skins'
}
if (Test-Path -LiteralPath $assetsDir) {
    $managedRootEntries += 'assets'
}
if ($IncludeOcr -and (Test-Path -LiteralPath $toolsDir)) {
    $managedRootEntries += 'tools'
}
if ($IncludeOcr -and (Test-Path -LiteralPath $rapidOcrDir)) {
    $managedRootEntries += 'RapidOCR-3.9.0'
}
if ($IncludeOcr -and (Test-Path -LiteralPath $rapidOcrVenvDir)) {
    $managedRootEntries += $pythonPackagesDirName
}
if ($pythonRuntimeSourceVenv) {
    $managedRootEntries += $pythonRuntimeDirName
}

$managedEntryNames = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::OrdinalIgnoreCase)
foreach ($entryName in $managedRootEntries) {
    [void]$managedEntryNames.Add($entryName)
}

foreach ($existingEntry in @(Get-ChildItem -LiteralPath $OutputRoot -Force -ErrorAction SilentlyContinue)) {
    if (-not $managedEntryNames.Contains($existingEntry.Name)) {
        Remove-ItemWithRetry -Path $existingEntry.FullName
    }
}

Remove-ItemWithRetry -Path (Join-Path $OutputRoot 'runtime')
New-Item -ItemType Directory -Path $runtimeOutputX64 -Force | Out-Null
New-Item -ItemType Directory -Path $runtimeOutputX86 -Force | Out-Null

Copy-FileIfChanged -SourcePath $tipDll -DestinationPath (Join-Path $runtimeOutputX64 'srf_tsf_tip.dll')
Copy-FileIfChanged -SourcePath $x86TipDll -DestinationPath (Join-Path $runtimeOutputX86 'srf_tsf_tip.dll')
Copy-FileIfChanged -SourcePath $overlayExe -DestinationPath (Join-Path $runtimeOutputX64 'srf_ime_overlay.exe')
Copy-FileIfChanged -SourcePath $x86OverlayExe -DestinationPath (Join-Path $runtimeOutputX86 'srf_ime_overlay.exe')
Copy-FileIfChanged -SourcePath $settingsExe -DestinationPath (Join-Path $OutputRoot 'srf_ime_settings.exe')
Copy-FileIfChanged -SourcePath $settingsManifest -DestinationPath (Join-Path $OutputRoot 'srf_ime_settings.exe.manifest')
Copy-FileIfChanged -SourcePath $trayExe -DestinationPath (Join-Path $OutputRoot 'srf_ime_tray.exe')
Copy-FileIfChanged -SourcePath $trayManifest -DestinationPath (Join-Path $OutputRoot 'srf_ime_tray.exe.manifest')
Copy-FileIfChanged -SourcePath $engineExe -DestinationPath (Join-Path $OutputRoot 'srf_ime_engine.exe')
Copy-FileIfChanged -SourcePath $clipboardExe -DestinationPath (Join-Path $OutputRoot 'srf_ime_clipboard.exe')
Copy-FileIfChanged -SourcePath $clipboardSvcExe -DestinationPath (Join-Path $OutputRoot 'srf_ime_clipboard_svc.exe')
Copy-FileIfChanged -SourcePath $handwriteExe -DestinationPath (Join-Path $OutputRoot 'srf_ime_handwrite.exe')
if ($IncludeOcr) {
    Copy-FileIfChanged -SourcePath $ocrExe -DestinationPath (Join-Path $OutputRoot 'srf_ime_ocr.exe')
} else {
    Remove-ItemWithRetry -Path (Join-Path $OutputRoot 'srf_ime_ocr.exe')
}
Remove-ItemWithRetry -Path (Join-Path $OutputRoot 'srf_ime_translate.exe')
Copy-PowerShellScriptUtf8Bom -SourcePath (Join-Path $TsfTipRoot 'invoke_registration.ps1') -DestinationPath (Join-Path $OutputRoot 'invoke_registration.ps1')
Copy-PowerShellScriptUtf8Bom -SourcePath (Join-Path $PSScriptRoot 'install_dev.ps1') -DestinationPath (Join-Path $OutputRoot 'install_dev.ps1')
Copy-PowerShellScriptUtf8Bom -SourcePath (Join-Path $PSScriptRoot 'install_current_user.ps1') -DestinationPath (Join-Path $OutputRoot 'install_current_user.ps1')
Copy-PowerShellScriptUtf8Bom -SourcePath (Join-Path $PSScriptRoot 'restart_stale_hosts.ps1') -DestinationPath (Join-Path $OutputRoot 'restart_stale_hosts.ps1')
Copy-PowerShellScriptUtf8Bom -SourcePath $repairInstallScript -DestinationPath (Join-Path $OutputRoot 'repair_install.ps1')
Copy-PowerShellScriptUtf8Bom -SourcePath (Join-Path $PSScriptRoot 'export_diagnostics.ps1') -DestinationPath (Join-Path $OutputRoot 'export_diagnostics.ps1')
Copy-PowerShellScriptUtf8Bom -SourcePath (Join-Path $PSScriptRoot 'uninstall_dev.ps1') -DestinationPath (Join-Path $OutputRoot 'uninstall_dev.ps1')
Copy-PowerShellScriptUtf8Bom -SourcePath (Join-Path $PSScriptRoot 'uninstall_current_user.ps1') -DestinationPath (Join-Path $OutputRoot 'uninstall_current_user.ps1')
Copy-FileIfChanged -SourcePath $userDataManifest -DestinationPath (Join-Path $OutputRoot 'user_data_manifest.json')
Copy-FileIfChanged -SourcePath $versionFile -DestinationPath (Join-Path $OutputRoot 'VERSION')
Copy-FileIfChanged -SourcePath $projectLicenseFile -DestinationPath (Join-Path $OutputRoot 'LICENSE')
Copy-FileIfChanged -SourcePath $licenseScopeFile -DestinationPath (Join-Path $OutputRoot 'LICENSE_SCOPE.md')
Copy-FileIfChanged -SourcePath $projectNoticeFile -DestinationPath (Join-Path $OutputRoot 'NOTICE')
Write-ThirdPartyDistributionNotices `
    -SourcePath $thirdPartyNoticesFile `
    -DestinationPath (Join-Path $OutputRoot 'THIRD_PARTY_NOTICES.md')
$stagedLicenseReportDir = Join-Path $OutputRoot 'licenses'
if ((Test-Path -LiteralPath $generatedLicenseDir) -or (Test-Path -LiteralPath $distributionLicenseDir)) {
    Remove-ItemWithRetry -Path $stagedLicenseReportDir
    New-Item -ItemType Directory -Path $stagedLicenseReportDir -Force | Out-Null
    if (Test-Path -LiteralPath $distributionLicenseDir) {
        foreach ($licenseFile in @(Get-ChildItem -LiteralPath $distributionLicenseDir -File)) {
            Copy-FileIfChanged -SourcePath $licenseFile.FullName -DestinationPath (Join-Path $stagedLicenseReportDir $licenseFile.Name)
        }
    }
    if (Test-Path -LiteralPath $generatedLicenseDir) {
        foreach ($reportFile in @(Get-ChildItem -LiteralPath $generatedLicenseDir -File)) {
            Copy-FileIfChanged -SourcePath $reportFile.FullName -DestinationPath (Join-Path $stagedLicenseReportDir $reportFile.Name)
        }
    }
} else {
    Remove-ItemWithRetry -Path $stagedLicenseReportDir
}
[System.IO.File]::WriteAllText((Join-Path $OutputRoot 'current_runtime_payload.txt'), $runtimeRelativePath, [System.Text.Encoding]::ASCII)

$lexiconTarget = Join-Path $OutputRoot $lexiconDir.Name
$lexiconSyncExcludes = @('translate')
Sync-DirectoryContents -SourceDir $lexiconDir.FullName -DestinationDir $lexiconTarget -ExcludeNames $lexiconSyncExcludes
Remove-ItemWithRetry -Path (Join-Path $lexiconTarget 'translate')

# 与仓库 scripts/prebake_lexicon.ps1 相同产物：词库目录内的 lexicon.bin（运行时优先加载）
# Keep behavior aligned with scripts/prebake_lexicon.ps1:
# generate lexicon.bin inside the staged lexicon directory.
$lexiconBin = Join-Path $lexiconTarget 'lexicon.bin'
& $bakeLexiconExe $lexiconDir.FullName $lexiconBin --profile $LexiconProfile
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $lexiconBin)) {
    throw "Failed to prebake lexicon.bin: $lexiconBin"
}

$hotLexiconBin = Join-Path $lexiconTarget 'hot_lexicon.bin'
& $bakeLexiconExe $lexiconDir.FullName $hotLexiconBin --profile hot
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $hotLexiconBin)) {
    throw "Failed to prebake hot_lexicon.bin: $hotLexiconBin"
}

if (Test-Path -LiteralPath $fontDir) {
    Sync-DirectoryContents -SourceDir $fontDir -DestinationDir (Join-Path $OutputRoot 'font1')
}
if (Test-Path -LiteralPath $skinDir) {
    Sync-DirectoryContents -SourceDir $skinDir -DestinationDir (Join-Path $OutputRoot 'skins')
}
if (Test-Path -LiteralPath $assetsDir) {
    Sync-DirectoryContents -SourceDir $assetsDir -DestinationDir (Join-Path $OutputRoot 'assets')
}
if ($IncludeOcr -and (Test-Path -LiteralPath $toolsDir)) {
    $toolsTarget = Join-Path $OutputRoot 'tools'
    if (-not (Test-Path -LiteralPath $toolsTarget)) {
        New-Item -ItemType Directory -Path $toolsTarget -Force | Out-Null
    }
    foreach ($existingTool in @(Get-ChildItem -LiteralPath $toolsTarget -Force -ErrorAction SilentlyContinue)) {
        Remove-ItemWithRetry -Path $existingTool.FullName
    }
    $toolNames = @()
    if ($IncludeOcr) {
        $toolNames += @('kaixin_ocr_engine.py', 'kaixin_ocr_engine.cmd', 'kaixin_cv_crop.py')
    }
    foreach ($toolName in ($toolNames | Select-Object -Unique)) {
        $sourceTool = Join-Path $toolsDir $toolName
        if (-not (Test-Path -LiteralPath $sourceTool)) {
            throw "Missing tool helper: $sourceTool"
        }
        Copy-FileIfChanged -SourcePath $sourceTool -DestinationPath (Join-Path $toolsTarget $toolName)
    }
} else {
    Remove-ItemWithRetry -Path (Join-Path $OutputRoot 'tools')
}
if ($IncludeOcr -and (Test-Path -LiteralPath $rapidOcrVenvDir)) {
    Sync-PythonPackagesFromVenv `
        -VenvDir $rapidOcrVenvDir `
        -DestinationDir (Join-Path $OutputRoot $pythonPackagesDirName)
} else {
    Remove-ItemWithRetry -Path (Join-Path $OutputRoot $pythonPackagesDirName)
}
# Remove the legacy full virtual environment from restaged packages.
Remove-ItemWithRetry -Path (Join-Path $OutputRoot '.venv-rapidocr')
if ($pythonRuntimeSourceVenv) {
    Sync-PythonRuntimeFromVenv `
        -VenvDir $pythonRuntimeSourceVenv `
        -DestinationDir (Join-Path $OutputRoot $pythonRuntimeDirName)
} else {
    Remove-ItemWithRetry -Path (Join-Path $OutputRoot $pythonRuntimeDirName)
}
Remove-ItemWithRetry -Path (Join-Path $OutputRoot '.venv-translate')
Remove-ItemWithRetry -Path (Join-Path $OutputRoot 'models\translate')
if ($IncludeOcr -and (Test-Path -LiteralPath $rapidOcrDir)) {
    $rapidOcrTarget = Join-Path $OutputRoot 'RapidOCR-3.9.0'
    $rapidOcrPythonTarget = Join-Path $rapidOcrTarget 'python'
    if (-not (Test-Path -LiteralPath $rapidOcrPythonTarget)) {
        New-Item -ItemType Directory -Path $rapidOcrPythonTarget -Force | Out-Null
    }
    Sync-DirectoryContents `
        -SourceDir (Join-Path $rapidOcrDir 'python\rapidocr') `
        -DestinationDir (Join-Path $rapidOcrPythonTarget 'rapidocr') `
        -ExcludeNames $PythonPackageExcludeNames `
        -ExcludePatterns $PythonPackageExcludePatterns
    foreach ($rapidOcrRootFile in @('README.md', 'README-CN.md', 'LICENSE')) {
        $source = Join-Path $rapidOcrDir $rapidOcrRootFile
        if (Test-Path -LiteralPath $source) {
            Copy-FileIfChanged -SourcePath $source -DestinationPath (Join-Path $rapidOcrTarget $rapidOcrRootFile)
        }
    }
} else {
    Remove-ItemWithRetry -Path (Join-Path $OutputRoot 'RapidOCR-3.9.0')
}
$componentManifestPath = Join-Path $OutputRoot $ComponentManifestName
Remove-ItemWithRetry -Path $componentManifestPath
Write-PackageHashManifest -Root $OutputRoot -ManifestName $PackageManifestName
Write-ComponentManifestFromPackageManifest `
    -Root $OutputRoot `
    -PackageManifest $PackageManifestName `
    -ComponentManifest $ComponentManifestName
if ($Zip) {
    $zipPath = "$OutputRoot.zip"
    if (Test-Path -LiteralPath $zipPath) {
        Remove-Item -LiteralPath $zipPath -Force
    }
    Compress-Archive -Path (Join-Path $OutputRoot '*') -DestinationPath $zipPath
    Write-Output "Created zip package: $zipPath"
}

Write-Output "Staged package: $OutputRoot"
Write-Output "OCR extension: $([bool]$IncludeOcr)"
Write-Output "Translation runtime: external WinTranslator (not bundled)"
