param(
    [string]$InstallationRoot = $PSScriptRoot,
    [switch]$SkipRegistration,
    [switch]$RemoveUserData,
    [switch]$RemoveTransientUserData
)

$ErrorActionPreference = 'Stop'

$TextServiceClsid = '{E5A91C40-7B2D-4F8A-9C11-8F3E6D2A1B00}'
$ProfileGuid = '{A3F0B2C1-4D5E-6789-ABCD-EF0123456789}'
$SimplifiedChineseTip = ('0804:{0}{1}' -f $TextServiceClsid, $ProfileGuid)
$AppDisplayName = '开心输入法'
$AppPathName = 'kaixin'
$TrayRunEntryName = $AppDisplayName
$LegacyTrayRunEntryName = $AppDisplayName + (-join ([char[]](0x6258, 0x76D8)))
$EngineRunEntryName = $AppDisplayName + (-join ([char[]](0x5F15, 0x64CE)))
$RuntimePayloadManifestName = 'current_runtime_payload.txt'
$RuntimePayloadRootName = 'runtime'
$script:UninstallLogPath = $null

function Get-UserStateRoot {
    return Join-Path $env:LOCALAPPDATA $AppPathName
}

function Ensure-Directory {
    param([string]$Path)
    if ([string]::IsNullOrWhiteSpace($Path)) { return }
    if (-not (Test-Path -LiteralPath $Path)) {
        New-Item -ItemType Directory -Path $Path -Force | Out-Null
    }
}

function Get-DefaultUninstallLogPath {
    return Join-Path (Get-UserStateRoot) 'uninstall_user.log'
}

function Write-UninstallLog {
    param([string]$Message)
    if ([string]::IsNullOrWhiteSpace($script:UninstallLogPath)) { return }
    try {
        $directory = Split-Path -Parent $script:UninstallLogPath
        Ensure-Directory -Path $directory
        $line = ('{0:yyyy-MM-dd HH:mm:ss} {1}' -f (Get-Date), $Message)
        Add-Content -LiteralPath $script:UninstallLogPath -Value $line -Encoding UTF8 -ErrorAction Stop
    } catch {
        # Best-effort logging only.
    }
}

function Remove-InputMethodFromCurrentUserLanguageList {
    param([string]$Tip)

    $getCommand = Get-Command -Name Get-WinUserLanguageList -ErrorAction SilentlyContinue
    $setCommand = Get-Command -Name Set-WinUserLanguageList -ErrorAction SilentlyContinue
    if (-not $getCommand -or -not $setCommand) {
        Write-UninstallLog 'SKIP: Get/Set-WinUserLanguageList cmdlet unavailable'
        return
    }

    $languageList = Get-WinUserLanguageList
    $changed = $false
    foreach ($targetLanguage in @($languageList)) {
        $existingTips = New-Object 'System.Collections.Generic.List[string]'
        foreach ($existingTip in @($targetLanguage.InputMethodTips)) {
            if (-not [string]::IsNullOrWhiteSpace($existingTip) -and -not $existingTips.Contains($existingTip)) {
                [void]$existingTips.Add($existingTip)
            }
        }

        if (-not $existingTips.Remove($Tip)) {
            continue
        }

        if ($targetLanguage.InputMethodTips -and $targetLanguage.InputMethodTips.PSObject.Methods.Name -contains 'Clear') {
            $targetLanguage.InputMethodTips.Clear()
            foreach ($value in $existingTips) {
                [void]$targetLanguage.InputMethodTips.Add($value)
            }
        } else {
            $targetLanguage.InputMethodTips = [string[]]$existingTips
        }
        $changed = $true
    }

    if ($changed) {
        Set-WinUserLanguageList -LanguageList $languageList -Force
        Write-UninstallLog 'OK: removed TIP from current user language list'
    } else {
        Write-UninstallLog 'OK: TIP was not present in current user language list'
    }
}

function Remove-TrayRunEntry {
    $runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    if (Test-Path -Path $runKey) {
        Remove-ItemProperty -Path $runKey -Name $TrayRunEntryName -ErrorAction SilentlyContinue
        Remove-ItemProperty -Path $runKey -Name $LegacyTrayRunEntryName -ErrorAction SilentlyContinue
        Remove-ItemProperty -Path $runKey -Name $EngineRunEntryName -ErrorAction SilentlyContinue
    }
    $startupApprovedRunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
    if (Test-Path -Path $startupApprovedRunKey) {
        Remove-ItemProperty -Path $startupApprovedRunKey -Name $TrayRunEntryName -ErrorAction SilentlyContinue
        Remove-ItemProperty -Path $startupApprovedRunKey -Name $LegacyTrayRunEntryName -ErrorAction SilentlyContinue
        Remove-ItemProperty -Path $startupApprovedRunKey -Name $EngineRunEntryName -ErrorAction SilentlyContinue
    }
    Write-UninstallLog 'OK: removed current user Run entries'
}

function Remove-IfExists {
    param([string]$Path)
    if (Test-Path -LiteralPath $Path) {
        Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Remove-IfEmpty {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }
    $item = Get-Item -LiteralPath $Path -ErrorAction SilentlyContinue
    if (-not $item -or -not $item.PSIsContainer) {
        return
    }
    $hasEntries = Get-ChildItem -LiteralPath $Path -Force -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $hasEntries) {
        Remove-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
    }
}

function ConvertTo-PendingDeletePath {
    param([string]$Path)

    $full = [System.IO.Path]::GetFullPath($Path)
    if ($full.StartsWith('\\')) {
        return '\??\UNC\' + $full.TrimStart([char]92)
    }
    return '\??\' + $full
}

function Add-PendingDeleteOperations {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        Write-UninstallLog ("SKIP: pending delete target missing: {0}" -f $Path)
        return $true
    }

    $sessionManager = 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager'
    $existing = @()
    try {
        $value = (Get-ItemProperty -Path $sessionManager -Name PendingFileRenameOperations -ErrorAction SilentlyContinue).PendingFileRenameOperations
        if ($value) {
            $existing = @($value)
        }
    } catch {
        $existing = @()
    }

    $operations = New-Object 'System.Collections.Generic.List[string]'
    foreach ($item in $existing) {
        [void]$operations.Add([string]$item)
    }

    $files = @(Get-ChildItem -LiteralPath $Path -Recurse -Force -File -ErrorAction SilentlyContinue)
    foreach ($file in $files) {
        [void]$operations.Add((ConvertTo-PendingDeletePath -Path $file.FullName))
        [void]$operations.Add('')
    }

    $dirs = @(Get-ChildItem -LiteralPath $Path -Recurse -Force -Directory -ErrorAction SilentlyContinue |
        Sort-Object FullName -Descending)
    foreach ($dir in $dirs) {
        [void]$operations.Add((ConvertTo-PendingDeletePath -Path $dir.FullName))
        [void]$operations.Add('')
    }

    [void]$operations.Add((ConvertTo-PendingDeletePath -Path $Path))
    [void]$operations.Add('')

    try {
        New-ItemProperty -Path $sessionManager -Name PendingFileRenameOperations -Value ([string[]]$operations) -PropertyType MultiString -Force | Out-Null
        Write-Host ("Marked locked install files for deletion on next reboot: {0}" -f $Path)
        Write-UninstallLog ("OK: marked pending reboot deletion: {0}" -f $Path)
        return $true
    } catch {
        Write-UninstallLog ("WARN: could not mark pending reboot deletion for {0}: {1}" -f $Path, $_.Exception.Message)
        Write-Warning ("Could not mark files for reboot cleanup: {0}" -f $_.Exception.Message)
        return $false
    }
}

function Register-CurrentUserRunOnceRemoval {
    param([string]$TargetPath)

    if (-not (Test-Path -LiteralPath $TargetPath)) {
        Write-UninstallLog ("SKIP: RunOnce cleanup target missing: {0}" -f $TargetPath)
        return $true
    }

    try {
        $stateRoot = Get-UserStateRoot
        Ensure-Directory -Path $stateRoot
        $cleanupScript = Join-Path $stateRoot ("cleanup-{0}.ps1" -f ([guid]::NewGuid().ToString('N')))
        $escapedTarget = $TargetPath.Replace("'", "''")
        $cleanupContent = @"
Start-Sleep -Seconds 3
if (Test-Path -LiteralPath '$escapedTarget') {
    Remove-Item -LiteralPath '$escapedTarget' -Recurse -Force -ErrorAction SilentlyContinue
}
Remove-Item -LiteralPath `$MyInvocation.MyCommand.Path -Force -ErrorAction SilentlyContinue
"@
        Set-Content -LiteralPath $cleanupScript -Value $cleanupContent -Encoding UTF8
        $runOnceKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\RunOnce'
        New-Item -Path $runOnceKey -Force | Out-Null
        $valueName = 'KaixinInputCleanup-' + ([guid]::NewGuid().ToString('N'))
        $command = 'powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "{0}"' -f $cleanupScript
        New-ItemProperty -Path $runOnceKey -Name $valueName -Value $command -PropertyType String -Force | Out-Null
        Write-UninstallLog ("OK: registered HKCU RunOnce cleanup for {0}: {1}" -f $TargetPath, $cleanupScript)
        return $true
    } catch {
        Write-UninstallLog ("WARN: could not register HKCU RunOnce cleanup for {0}: {1}" -f $TargetPath, $_.Exception.Message)
        return $false
    }
}

function Get-UserDataRemovalManifest {
    $fallback = [pscustomobject]@{
        files = @(
            'kaixin.ini',
            'user_dict.sqlite',
            'user_dict.sqlite.partial',
            'user_dict.sqlite.previous',
            'user_dict.sqlite.bak',
            'user_dict.sqlite.lock',
            'user_dict.sqlite.reset',
            'ocr_history.sqlite',
            'ocr_history.sqlite.write.tmp',
            'runtime_events.sqlite',
            'runtime_events.sqlite-wal',
            'runtime_events.sqlite-shm',
            'runtime_events.sqlite-journal',
            'cedict_cache.sqlite',
            'cedict_cache.sqlite-journal',
            'clipboard_store.sqlite.tmp',
            'clipboard_store.sqlite.write.tmp',
            'clipboard_store.sqlite.lock',
            'clipboard_store.sqlite-journal',
            'engine_capability.dat',
            'clipboard_manager_window.sqlite',
            'clipboard_manager_window.sqlite-journal',
            'install_user.log',
            'install_language_list.log'
        )
        directories = @('cache', 'logs')
        registry_subkeys = @('State')
    }

    $manifestPath = Join-Path $InstallationRoot 'user_data_manifest.json'
    if (-not (Test-Path -LiteralPath $manifestPath)) {
        return $fallback
    }
    try {
        $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
        return $manifest
    } catch {
        Write-Warning ("Could not read user data manifest; using fallback list. {0}" -f $_.Exception.Message)
        return $fallback
    }
}

function Remove-SrfUserData {
    $configDir = Join-Path $env:LOCALAPPDATA $AppPathName
    $stateRoot = 'HKCU:\Software\' + $AppPathName
    $manifest = Get-UserDataRemovalManifest

    foreach ($name in @($manifest.files)) {
        Remove-IfExists -Path (Join-Path $configDir $name)
    }
    foreach ($name in @($manifest.directories)) {
        Remove-IfExists -Path (Join-Path $configDir $name)
    }

    foreach ($name in @($manifest.registry_subkeys)) {
        $keyPath = Join-Path $stateRoot $name
        if (Test-Path -Path $keyPath) {
            Remove-Item -Path $keyPath -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
    Remove-IfEmpty -Path $configDir
    if (Test-Path -Path $stateRoot) {
        $hasKeys = Get-ChildItem -Path $stateRoot -ErrorAction SilentlyContinue | Select-Object -First 1
        if (-not $hasKeys) {
            Remove-Item -Path $stateRoot -Force -ErrorAction SilentlyContinue
        }
    }
}

function Remove-SrfTransientUserData {
    $configDir = Join-Path $env:LOCALAPPDATA $AppPathName
    $manifest = Get-UserDataRemovalManifest

    foreach ($name in @($manifest.files)) {
        if ($name -like '*.log') {
            Remove-IfExists -Path (Join-Path $configDir $name)
        }
    }
    foreach ($name in @($manifest.directories)) {
        if ($name -in @('cache', 'logs')) {
            Remove-IfExists -Path (Join-Path $configDir $name)
        }
    }
    Remove-IfEmpty -Path $configDir
}

function Resolve-RuntimePayloadRoot {
    param(
        [string]$Root,
        [switch]$AllowMissing
    )

    $resolvedRoot = [System.IO.Path]::GetFullPath($Root)
    $manifestPath = Join-Path $resolvedRoot $RuntimePayloadManifestName
    if (Test-Path -LiteralPath $manifestPath) {
        $relative = (Get-Content -LiteralPath $manifestPath -Raw -Encoding ASCII).Trim()
        if (-not [string]::IsNullOrWhiteSpace($relative)) {
            $candidate = [System.IO.Path]::GetFullPath((Join-Path $resolvedRoot $relative))
            $rootPrefix = $resolvedRoot.TrimEnd('\') + '\'
            if (-not $candidate.Equals($resolvedRoot, [System.StringComparison]::OrdinalIgnoreCase) -and
                -not $candidate.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
                throw "Runtime payload path escapes install root: $candidate"
            }
            return $candidate
        }
    }

    $runtimeRoot = Join-Path $resolvedRoot $RuntimePayloadRootName
    if (Test-Path -LiteralPath $runtimeRoot) {
        $candidate = Get-ChildItem -LiteralPath $runtimeRoot -Directory -ErrorAction SilentlyContinue |
            Sort-Object Name -Descending |
            Where-Object {
                (Test-Path -LiteralPath (Join-Path $_.FullName 'srf_tsf_tip.dll')) -or
                (Test-Path -LiteralPath (Join-Path $_.FullName 'x64\srf_tsf_tip.dll'))
            } |
            Select-Object -First 1
        if ($candidate) {
            return $candidate.FullName
        }
    }

    if ($AllowMissing) {
        return $null
    }

    throw "Could not resolve runtime payload under $resolvedRoot"
}

function Resolve-RuntimePayloadArchRoot {
    param(
        [string]$Root,
        [ValidateSet('x64', 'x86')]
        [string]$Arch,
        [switch]$AllowMissing
    )

    $payloadRoot = Resolve-RuntimePayloadRoot -Root $Root -AllowMissing:$AllowMissing
    if ([string]::IsNullOrWhiteSpace($payloadRoot)) {
        return $null
    }
    $archRoot = Join-Path $payloadRoot $Arch
    if (Test-Path -LiteralPath $archRoot) {
        return $archRoot
    }
    if ($Arch -eq 'x64') {
        return $payloadRoot
    }
    if ($AllowMissing) {
        return $null
    }
    throw "Could not resolve $Arch runtime payload under $payloadRoot"
}

function Invoke-TipUnregistration {
    param(
        [string]$RegistrationScript,
        [string]$DllPath,
        [ValidateSet('x64', 'x86')]
        [string]$Arch
    )

    if ($Arch -eq 'x86') {
        $wowPowerShell = Join-Path $env:WINDIR 'SysWOW64\WindowsPowerShell\v1.0\powershell.exe'
        if (Test-Path -LiteralPath $wowPowerShell) {
            $powerShell = $wowPowerShell
        } else {
            Write-Warning "32-bit PowerShell was not found at $wowPowerShell; falling back to powershell.exe for x86 unregistration."
            Write-UninstallLog ("WARN: 32-bit PowerShell not found at {0}; falling back to powershell.exe" -f $wowPowerShell)
            $powerShell = 'powershell.exe'
        }
    } else {
        $powerShell = 'powershell.exe'
    }

    Write-UninstallLog ("BEGIN: {0} TIP unregistration DllPath={1}" -f $Arch, $DllPath)
    & $powerShell -NoProfile -ExecutionPolicy Bypass -File $RegistrationScript -DllPath $DllPath -Unregister
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) {
        throw "$Arch TIP unregistration failed with exit code $exitCode"
    }
    Write-UninstallLog ("OK: {0} TIP unregistration complete" -f $Arch)
}

function Stop-RunningHelpers {
    $backgroundHelpers = @('srf_ime_engine', 'srf_ime_tray')
    $visibleTools = @('srf_ime_settings', 'srf_ime_clipboard', 'srf_ime_handwrite', 'srf_ime_ocr', 'srf_ime_translate_result')
    foreach ($processName in @($backgroundHelpers + $visibleTools)) {
        foreach ($process in @(Get-Process -Name $processName -ErrorAction SilentlyContinue)) {
            try {
                if ($process.MainWindowHandle -ne 0) {
                    [void]$process.CloseMainWindow()
                }
            } catch {
            }
        }
    }
    Start-Sleep -Milliseconds 700
    foreach ($processName in $backgroundHelpers) {
        foreach ($process in @(Get-Process -Name $processName -ErrorAction SilentlyContinue)) {
            try {
                if (-not $process.HasExited) {
                    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
                }
            } catch {
            }
        }
    }
    foreach ($processName in $visibleTools) {
        foreach ($process in @(Get-Process -Name $processName -ErrorAction SilentlyContinue)) {
            Write-Host ("Visible helper app is still running and was not force-closed: {0}({1})" -f $process.ProcessName, $process.Id)
        }
    }
}

$InstallationRoot = [System.IO.Path]::GetFullPath($InstallationRoot)
$script:UninstallLogPath = Get-DefaultUninstallLogPath
Write-UninstallLog ("BEGIN: uninstall_current_user.ps1 SkipRegistration={0} RemoveUserData={1} RemoveTransientUserData={2} InstallationRoot={3}" -f
    $SkipRegistration.IsPresent, $RemoveUserData.IsPresent, $RemoveTransientUserData.IsPresent, $InstallationRoot)

trap {
    Write-UninstallLog ("FAIL: {0}" -f $_.Exception.Message)
    throw
}

Stop-RunningHelpers
Write-UninstallLog 'OK: stopped running helpers'

$registrationScript = Join-Path $InstallationRoot 'invoke_registration.ps1'
$runtimePayloadRoot = Resolve-RuntimePayloadRoot -Root $InstallationRoot -AllowMissing
$runtimePayloadX64 = Resolve-RuntimePayloadArchRoot -Root $InstallationRoot -Arch x64 -AllowMissing
$runtimePayloadX86 = Resolve-RuntimePayloadArchRoot -Root $InstallationRoot -Arch x86 -AllowMissing
$installedDll = if ($runtimePayloadX64) { Join-Path $runtimePayloadX64 'srf_tsf_tip.dll' } elseif ($runtimePayloadRoot) { Join-Path $runtimePayloadRoot 'srf_tsf_tip.dll' } else { Join-Path $InstallationRoot 'srf_tsf_tip.dll' }
$installedDllX86 = if ($runtimePayloadX86) { Join-Path $runtimePayloadX86 'srf_tsf_tip.dll' } else { $null }

if (-not $SkipRegistration -and (Test-Path -LiteralPath $registrationScript) -and (Test-Path -LiteralPath $installedDll)) {
    try {
        Invoke-TipUnregistration -RegistrationScript $registrationScript -DllPath $installedDll -Arch x64
        if ($installedDllX86 -and (Test-Path -LiteralPath $installedDllX86)) {
            Invoke-TipUnregistration -RegistrationScript $registrationScript -DllPath $installedDllX86 -Arch x86
        } else {
            Write-UninstallLog 'SKIP: x86 TIP DLL missing for current-user unregistration'
        }
    } catch {
        Write-UninstallLog ("WARN: TIP unregistration failed: {0}" -f $_.Exception.Message)
        Write-Warning $_
    }
} else {
    Write-UninstallLog 'SKIP: TIP unregistration not requested or registration files missing'
}

try {
    Remove-InputMethodFromCurrentUserLanguageList -Tip $SimplifiedChineseTip
} catch {
    Write-UninstallLog ("WARN: current user language-list cleanup failed: {0}" -f $_.Exception.Message)
    Write-Warning $_
}

Remove-TrayRunEntry

if ($RemoveUserData) {
    try {
        Remove-SrfUserData
        Write-UninstallLog 'OK: removed current user data from manifest'
    } catch {
        Write-UninstallLog ("WARN: current user data removal failed: {0}" -f $_.Exception.Message)
        Write-Warning $_
    }
} elseif ($RemoveTransientUserData) {
    try {
        Remove-SrfTransientUserData
        Write-UninstallLog 'OK: removed transient current user data'
    } catch {
        Write-UninstallLog ("WARN: transient current user data cleanup failed: {0}" -f $_.Exception.Message)
        Write-Warning $_
    }
} else {
    Write-UninstallLog 'OK: preserved current user data'
}

$runtimeRoot = Join-Path $InstallationRoot $RuntimePayloadRootName
if (Test-Path -LiteralPath $runtimeRoot) {
    if (-not (Add-PendingDeleteOperations -Path $runtimeRoot)) {
        [void](Register-CurrentUserRunOnceRemoval -TargetPath $runtimeRoot)
    }
} else {
    Write-UninstallLog ("OK: runtime root already absent: {0}" -f $runtimeRoot)
}

Write-UninstallLog ("OK: current-user uninstall cleanup complete for {0}" -f $InstallationRoot)
