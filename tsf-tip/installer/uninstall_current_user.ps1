param(
    [string]$InstallationRoot = $PSScriptRoot,
    [switch]$SkipRegistration,
    [switch]$SkipLanguageList,
    [switch]$SkipRuntimeCleanup,
    [switch]$FromMachineUninstaller
)

$ErrorActionPreference = 'Stop'

$TextServiceClsid = '{E5A91C40-7B2D-4F8A-9C11-8F3E6D2A1B00}'
$ProfileGuid = '{A3F0B2C1-4D5E-6789-ABCD-EF0123456789}'
$SimplifiedChineseTip = ('0804:{0}{1}' -f $TextServiceClsid, $ProfileGuid)
$MicrosoftPinyinTip = '0804:{81D4E9C9-1D3B-41BC-9E6C-4B40BF79E35E}{FA550B04-5AD7-411F-A5AC-CA038EC515D7}'
$AppDisplayName = '开心输入法'
$AppPathName = 'kaixin'
$TrayRunEntryName = $AppDisplayName
$LegacyTrayRunEntryName = $AppDisplayName + ( -join ([char[]](0x6258, 0x76D8)))
$EngineRunEntryName = $AppDisplayName + ( -join ([char[]](0x5F15, 0x64CE)))
$RuntimePayloadManifestName = 'current_runtime_payload.txt'
$RuntimePayloadRootName = 'runtime'

function Write-UninstallLog {
    param([string]$Message)
    Write-Host $Message
}

function Invoke-CleanupAsDesktopUser {
    # An over-the-shoulder UAC prompt can start the machine uninstaller as a different account.
    $sessionId = (Get-Process -Id $PID).SessionId
    $explorer = Get-Process -Name explorer -ErrorAction SilentlyContinue |
        Where-Object { $_.SessionId -eq $sessionId } | Select-Object -First 1
    if (-not $explorer) { return $false }

    $shell = Get-CimInstance -ClassName Win32_Process -Filter "ProcessId = $($explorer.Id)"
    $owner = Invoke-CimMethod -InputObject $shell -MethodName GetOwnerSid
    if ($owner.ReturnValue -ne 0 -or -not $owner.Sid) {
        throw 'Cannot identify the signed-in desktop user for uninstall cleanup.'
    }
    if ($owner.Sid -eq [Security.Principal.WindowsIdentity]::GetCurrent().User.Value) {
        return $false
    }

    # Run under the desktop user's existing interactive logon, without storing credentials.
    $taskName = 'KaixinInputUninstall-' + [guid]::NewGuid().ToString('N')
    $arguments = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File "{0}" -InstallationRoot "{1}" -SkipRuntimeCleanup' -f $PSCommandPath, $InstallationRoot
    if ($SkipLanguageList) { $arguments += ' -SkipLanguageList' }
    if ($SkipRegistration) { $arguments += ' -SkipRegistration' }
    $action = New-ScheduledTaskAction -Execute (Join-Path $PSHOME 'powershell.exe') -Argument $arguments
    $principal = New-ScheduledTaskPrincipal -UserId $owner.Sid -LogonType Interactive
    $settings = New-ScheduledTaskSettingsSet -Hidden -ExecutionTimeLimit (New-TimeSpan -Minutes 2)
    Register-ScheduledTask -TaskName $taskName -Action $action -Principal $principal -Settings $settings | Out-Null
    try {
        $previousRun = (Get-ScheduledTaskInfo -TaskName $taskName).LastRunTime
        Start-ScheduledTask -TaskName $taskName
        $deadline = [DateTime]::UtcNow.AddMinutes(2)
        do {
            Start-Sleep -Milliseconds 300
            $state = (Get-ScheduledTask -TaskName $taskName).State
            $run = Get-ScheduledTaskInfo -TaskName $taskName
            if ($state -eq 'Ready' -and $run.LastRunTime -gt $previousRun) {
                if ($run.LastTaskResult -ne 0) {
                    throw "Desktop user uninstall cleanup failed ($($run.LastTaskResult))."
                }
                return $true
            }
        } while ([DateTime]::UtcNow -lt $deadline)
        throw 'Desktop user uninstall cleanup timed out.'
    } finally {
        Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
    }
}

function Restore-MicrosoftPinyin {
    $languageList = Get-WinUserLanguageList
    $chinese = $languageList | Where-Object { $_.LanguageTag -in @('zh-CN', 'zh-Hans-CN') } | Select-Object -First 1
    $changed = $false
    if (-not $chinese) {
        $chinese = (New-WinUserLanguageList 'zh-CN')[0]
        $languageList.Add($chinese)
        $changed = $true
    }
    if ($chinese.InputMethodTips -notcontains $MicrosoftPinyinTip) {
        $chinese.InputMethodTips.Insert(0, $MicrosoftPinyinTip)
        $changed = $true
    }
    foreach ($language in $languageList) {
        for ($i = $language.InputMethodTips.Count - 1; $i -ge 0; $i--) {
            if ($language.InputMethodTips[$i] -eq $SimplifiedChineseTip) {
                $language.InputMethodTips.RemoveAt($i)
                $changed = $true
            }
        }
    }
    if ($changed) {
        Set-WinUserLanguageList -LanguageList $languageList -Force
    }
    $updatedLanguages = Get-WinUserLanguageList
    $updatedTips = @($updatedLanguages | ForEach-Object { $_.InputMethodTips })
    if ($updatedTips -contains $SimplifiedChineseTip -or $updatedTips -notcontains $MicrosoftPinyinTip) {
        throw 'Windows did not update the installed input methods.'
    }
    Set-WinDefaultInputMethodOverride -InputTip $MicrosoftPinyinTip

    if ((Get-WinDefaultInputMethodOverride).InputMethodTip -ne $MicrosoftPinyinTip) {
        throw 'Windows did not set Microsoft Pinyin as the default input method.'
    }
    Write-UninstallLog 'OK: restored Microsoft Pinyin and removed Kaixin from the current user language list'
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
        $cleanupScript = Join-Path ([IO.Path]::GetTempPath()) ("kaixin-runtime-cleanup-{0}.ps1" -f ([guid]::NewGuid().ToString('N')))
        $escapedTarget = $TargetPath.Replace("'", "''")
        $escapedMarker = (Join-Path $InstallationRoot $RuntimePayloadManifestName).Replace("'", "''")
        $cleanupContent = @"
Start-Sleep -Seconds 3
if ((Test-Path -LiteralPath '$escapedTarget') -and -not (Test-Path -LiteralPath '$escapedMarker')) {
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

function Remove-UserDataTree {
    param([string]$Path)

    try {
        $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    } catch [System.Management.Automation.ItemNotFoundException] {
        return
    }
    if ($item.PSIsContainer) {
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            [IO.Directory]::Delete($item.FullName)
            return
        }
        foreach ($child in (Get-ChildItem -LiteralPath $item.FullName -Force)) {
            Remove-UserDataTree -Path $child.FullName
        }
    }
    Remove-Item -LiteralPath $item.FullName -Force
}

function Remove-KaixinUserData {
    $runOnce = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\RunOnce'
    $runOnceKey = Get-Item -LiteralPath $runOnce -ErrorAction SilentlyContinue
    if ($runOnceKey) {
        foreach ($name in $runOnceKey.GetValueNames()) {
            if ($name -like 'KaixinInputCleanup-*') {
                Remove-ItemProperty -LiteralPath $runOnce -Name $name
            }
        }
    }
    $stateKey = 'HKCU:\Software\' + $AppPathName
    if (Test-Path -LiteralPath $stateKey) {
        Remove-Item -LiteralPath $stateKey -Recurse -Force
    }
    Remove-UserDataTree -Path ([IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA $AppPathName)))
    # These internal capture handoffs can survive if a helper was interrupted.
    $tempRoot = [IO.Path]::GetTempPath()
    foreach ($entry in (Get-ChildItem -LiteralPath $tempRoot -Force)) {
        if (($entry.PSIsContainer -and $entry.Name -match '^kaixin-(frame|uia)-[0-9]+-[0-9]+-[0-9]+$') -or
            (-not $entry.PSIsContainer -and $entry.Name -match '^kaixin_ocr_(capture|crop|processed|border|file)_[0-9]{17}\.png$')) {
            Remove-UserDataTree -Path $entry.FullName
        }
    }
    Write-UninstallLog 'OK: removed current user data'
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
        $powerShellDir = if ([Environment]::Is64BitProcess) { 'System32' } else { 'Sysnative' }
        $powerShell = Join-Path $env:WINDIR "$powerShellDir\WindowsPowerShell\v1.0\powershell.exe"
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
    $sessionId = (Get-Process -Id $PID).SessionId
    $names = @(
        'srf_ime_engine', 'srf_ime_tray', 'srf_ime_clipboard_svc', 'srf_ime_overlay',
        'srf_ime_settings', 'srf_ime_clipboard', 'srf_ime_handwrite', 'srf_ime_symbols',
        'srf_ime_ocr', 'srf_ime_translate', 'srf_ime_translate_result', 'KaixinShareX'
    )
    $helpers = @(Get-Process -Name $names -ErrorAction SilentlyContinue | Where-Object { $_.SessionId -eq $sessionId })
    foreach ($helper in $helpers) {
        if ($helper.MainWindowHandle) {
            [void]$helper.CloseMainWindow()
        }
    }
    foreach ($helper in $helpers) {
        if (-not $helper.WaitForExit(500)) {
            Stop-Process -InputObject $helper -Force -ErrorAction SilentlyContinue
            [void]$helper.WaitForExit(1000)
        }
    }
}

$InstallationRoot = [System.IO.Path]::GetFullPath($InstallationRoot)
if ($FromMachineUninstaller -and (Invoke-CleanupAsDesktopUser)) {
    return
}
$failures = @()

# Disable autostart before closing the helpers so they cannot reopen user data.
Remove-TrayRunEntry
if (-not $SkipLanguageList) {
    try {
        Restore-MicrosoftPinyin
    } catch {
        $failures += $_.Exception.Message
    }
}

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
        $failures += $_.Exception.Message
    }
} else {
    Write-UninstallLog 'SKIP: TIP unregistration not requested or registration files missing'
}

try {
    Stop-RunningHelpers
    Remove-KaixinUserData
} catch {
    $failures += $_.Exception.Message
}

if ($failures.Count) {
    throw ($failures -join [Environment]::NewLine)
}
if (-not ($SkipRuntimeCleanup -or $FromMachineUninstaller)) {
    $runtimeRoot = Join-Path $InstallationRoot $RuntimePayloadRootName
    if ((Test-Path -LiteralPath $runtimeRoot) -and -not (Add-PendingDeleteOperations -Path $runtimeRoot)) {
        if (-not (Register-CurrentUserRunOnceRemoval -TargetPath $runtimeRoot)) {
            Write-Warning 'Some locked runtime files could not be scheduled for removal.'
        }
    }
}
