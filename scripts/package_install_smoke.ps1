param(
    [string]$PackageRoot = '',
    [string]$SmokeRoot = (Join-Path $env:TEMP ('kaixin-install-smoke-' + [guid]::NewGuid().ToString('N'))),
    [switch]$ExerciseLanguageList,
    [switch]$KeepSmokeRoot
)

$ErrorActionPreference = 'Stop'

$TextServiceClsid = '{E5A91C40-7B2D-4F8A-9C11-8F3E6D2A1B00}'
$ProfileGuid = '{A3F0B2C1-4D5E-6789-ABCD-EF0123456789}'
$SimplifiedChineseTip = ('0804:{0}{1}' -f $TextServiceClsid, $ProfileGuid)
$AppDisplayName = '开心输入法'
$RunEntryName = $AppDisplayName

function Assert-PathExists {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) {
        throw "Missing expected path: $Path"
    }
}

function Assert-PathMissing {
    param([string]$Path)
    if (Test-Path -LiteralPath $Path) {
        throw "Unexpected path exists: $Path"
    }
}

function Assert-RegistryPath {
    param([string]$Path)
    if (-not (Test-Path -Path $Path)) {
        throw "Missing expected registry path: $Path"
    }
}

function Resolve-SmokePayloadRoot {
    param([string]$InstallRoot)
    $marker = Join-Path $InstallRoot 'current_runtime_payload.txt'
    Assert-PathExists $marker
    $relative = (Get-Content -LiteralPath $marker -Raw -Encoding ASCII).Trim()
    if ([string]::IsNullOrWhiteSpace($relative)) {
        throw "current_runtime_payload.txt is empty"
    }
    $payload = [System.IO.Path]::GetFullPath((Join-Path $InstallRoot $relative))
    Assert-PathExists $payload
    return $payload
}

function Test-LanguageListContainsTip {
    $getCommand = Get-Command -Name Get-WinUserLanguageList -ErrorAction SilentlyContinue
    if (-not $getCommand) {
        return $false
    }
    foreach ($language in @(Get-WinUserLanguageList)) {
        if ($language.InputMethodTips -contains $SimplifiedChineseTip) {
            return $true
        }
    }
    return $false
}

if ([string]::IsNullOrWhiteSpace($PackageRoot)) {
    foreach ($candidateName in @('kaixin-package-full', 'kaixin-package-ocr', 'kaixin-package-ime', 'kaixin-package')) {
        $candidate = Join-Path $PSScriptRoot ("..\dist\" + $candidateName)
        if (Test-Path -LiteralPath $candidate) {
            $PackageRoot = $candidate
            break
        }
    }
}

$PackageRoot = [System.IO.Path]::GetFullPath($PackageRoot)
Assert-PathExists $PackageRoot
Assert-PathExists (Join-Path $PackageRoot 'component_manifest.ini')

$previousLocalAppData = $env:LOCALAPPDATA
$SmokeRoot = [System.IO.Path]::GetFullPath($SmokeRoot)
$smokeLocalAppData = Join-Path $SmokeRoot 'LocalAppData'
$installRoot = Join-Path $smokeLocalAppData 'Programs\kaixin'
New-Item -ItemType Directory -Path $smokeLocalAppData -Force | Out-Null

try {
    $env:LOCALAPPDATA = $smokeLocalAppData
    $installScript = Join-Path $PackageRoot 'install_dev.ps1'
    $uninstallScript = Join-Path $installRoot 'uninstall_dev.ps1'
    Assert-PathExists $installScript

    $installArgs = @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $installScript,
        '-PackageRoot', $PackageRoot,
        '-InstallationRoot', $installRoot,
        '-SkipTextServiceRestart'
    )
    if (-not $ExerciseLanguageList) {
        $installArgs += '-SkipLanguageList'
    }

    & powershell.exe @installArgs
    if ($LASTEXITCODE -ne 0) {
        throw "install_dev.ps1 failed with exit code $LASTEXITCODE"
    }

    $payload = Resolve-SmokePayloadRoot -InstallRoot $installRoot
    Assert-PathExists (Join-Path $payload 'x64\srf_tsf_tip.dll')
    Assert-PathExists (Join-Path $payload 'x86\srf_tsf_tip.dll')
    Assert-PathExists (Join-Path $installRoot 'srf_ime_settings.exe')
    Assert-PathExists (Join-Path $installRoot 'user_data_manifest.json')
    Assert-PathExists (Join-Path $installRoot 'component_manifest.ini')

    Assert-RegistryPath ("HKCU:\Software\Classes\CLSID\{0}\InprocServer32" -f $TextServiceClsid)
    Assert-RegistryPath ("HKCU:\Software\Classes\WOW6432Node\CLSID\{0}\InprocServer32" -f $TextServiceClsid)

    $runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    $runProps = Get-ItemProperty -Path $runKey -Name $RunEntryName -ErrorAction SilentlyContinue
    $runValue = if ($runProps) { $runProps.PSObject.Properties[$RunEntryName].Value } else { $null }
    if ([string]::IsNullOrWhiteSpace($runValue)) {
        throw "Tray Run entry was not created"
    }

    if ($ExerciseLanguageList -and -not (Test-LanguageListContainsTip)) {
        throw "TIP was not added to the current user's language list"
    }

    $settings = Join-Path $installRoot 'srf_ime_settings.exe'
    $process = Start-Process -FilePath $settings -PassThru -WindowStyle Hidden
    Start-Sleep -Milliseconds 900
    if ($process.HasExited -and $process.ExitCode -ne 0) {
        throw "settings helper exited with code $($process.ExitCode)"
    }
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    }

    Assert-PathExists $uninstallScript
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $uninstallScript -InstallationRoot $installRoot
    if ($LASTEXITCODE -ne 0) {
        throw "uninstall_dev.ps1 failed with exit code $LASTEXITCODE"
    }
    Start-Sleep -Seconds 4

    if (Test-Path -Path ("HKCU:\Software\Classes\CLSID\{0}" -f $TextServiceClsid)) {
        throw "x64 current-user COM registration remains after uninstall"
    }
    if (Test-Path -Path ("HKCU:\Software\Classes\WOW6432Node\CLSID\{0}" -f $TextServiceClsid)) {
        throw "x86 current-user COM registration remains after uninstall"
    }
    if ((Get-ItemProperty -Path $runKey -Name $RunEntryName -ErrorAction SilentlyContinue)) {
        throw "Tray Run entry remains after uninstall"
    }
    if ($ExerciseLanguageList -and (Test-LanguageListContainsTip)) {
        throw "TIP remains in language list after uninstall"
    }

    Write-Host "package install smoke passed: $installRoot"
} finally {
    $env:LOCALAPPDATA = $previousLocalAppData
    foreach ($name in @('srf_ime_engine', 'srf_ime_tray', 'srf_ime_settings', 'srf_ime_clipboard', 'srf_ime_handwrite', 'srf_ime_ocr')) {
        Stop-Process -Name $name -Force -ErrorAction SilentlyContinue
    }
    if (-not $KeepSmokeRoot -and (Test-Path -LiteralPath $SmokeRoot)) {
        Remove-Item -LiteralPath $SmokeRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
