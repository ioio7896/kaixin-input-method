param(
    [string]$InstallationRoot = $PSScriptRoot,
    [switch]$Machine,
    [switch]$SkipFileRemoval,
    [switch]$SkipRegistration,
    [switch]$SkipUninstallEntry,
    [switch]$SkipCurrentUserCleanup,
    [switch]$SkipLanguageList
)

$ErrorActionPreference = 'Stop'

$TextServiceClsid = '{E5A91C40-7B2D-4F8A-9C11-8F3E6D2A1B00}'
$ProfileGuid = '{A3F0B2C1-4D5E-6789-ABCD-EF0123456789}'
$SimplifiedChineseTip = ('0804:{0}{1}' -f $TextServiceClsid, $ProfileGuid)
$AppDisplayName = '开心输入法'
$AppPathName = 'kaixin'
$RuntimePayloadManifestName = 'current_runtime_payload.txt'
$RuntimePayloadRootName = 'runtime'

function Get-MachineStateRoot {
    $programData = [Environment]::GetFolderPath('CommonApplicationData')
    if ([string]::IsNullOrWhiteSpace($programData)) {
        $programData = $env:ProgramData
    }
    return Join-Path $programData $AppPathName
}

function Write-UninstallLog {
    param([string]$Message)
    Write-Host $Message
}

function Get-UninstallKeyPath {
    param([bool]$MachineScope)
    $keyRoot = if ($MachineScope) { 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall' } else { 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' }
    return Join-Path $keyRoot $AppDisplayName
}

function Start-DeferredRemoval {
    param([string]$TargetPath)

    $cleanupScript = Join-Path $env:TEMP ("kaixin-ime-cleanup-{0}.ps1" -f ([guid]::NewGuid().ToString('N')))
    $cleanupContent = @"
Start-Sleep -Seconds 2
if (Test-Path -LiteralPath '$($TargetPath.Replace("'", "''"))') {
    Remove-Item -LiteralPath '$($TargetPath.Replace("'", "''"))' -Recurse -Force
}
Remove-Item -LiteralPath '$($cleanupScript.Replace("'", "''"))' -Force -ErrorAction SilentlyContinue
"@
    Set-Content -LiteralPath $cleanupScript -Value $cleanupContent -Encoding UTF8
    Start-Process -FilePath powershell.exe -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $cleanupScript) -WindowStyle Hidden | Out-Null
    Write-UninstallLog ("OK: started deferred removal for {0}" -f $TargetPath)
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
        $cleanupScript = Join-Path ([IO.Path]::GetTempPath()) ("cleanup-{0}.ps1" -f ([guid]::NewGuid().ToString('N')))
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

function Remove-TipFromLoadedUserRegistry {
    param([string]$Tip)

    $removedValues = 0
    $removedKeys = 0
    $visitedUsers = 0
    foreach ($sidKey in @(Get-ChildItem -LiteralPath 'Registry::HKEY_USERS' -ErrorAction SilentlyContinue)) {
        $sid = [System.IO.Path]::GetFileName($sidKey.Name)
        if ([string]::IsNullOrWhiteSpace($sid) -or $sid.EndsWith('_Classes')) { continue }
        if ($sid -notmatch '^S-\d-\d+-(\d+-)+\d+$') { continue }
        $visitedUsers++

        $candidateRoots = @(
            "Registry::HKEY_USERS\$sid\Control Panel\International\User Profile",
            "Registry::HKEY_USERS\$sid\Software\Microsoft\CTF"
        )
        foreach ($root in $candidateRoots) {
            if (-not (Test-Path -LiteralPath $root)) { continue }
            $keys = @()
            try {
                $keys = @((Get-Item -LiteralPath $root -ErrorAction Stop)) +
                @(Get-ChildItem -LiteralPath $root -Recurse -ErrorAction SilentlyContinue)
            } catch {
                Write-UninstallLog ("WARN: could not enumerate loaded user language registry root {0}: {1}" -f $root, $_.Exception.Message)
                continue
            }

            foreach ($key in $keys) {
                try {
                    if ([System.IO.Path]::GetFileName($key.Name).Equals($Tip, [System.StringComparison]::OrdinalIgnoreCase)) {
                        Remove-Item -LiteralPath ('Registry::' + $key.Name) -Recurse -Force -ErrorAction Stop
                        $removedKeys++
                        continue
                    }

                    $writable = Get-Item -LiteralPath ('Registry::' + $key.Name) -ErrorAction Stop
                    foreach ($valueName in @($writable.GetValueNames())) {
                        $deleteValue = $false
                        $newValue = $null
                        $kind = $writable.GetValueKind($valueName)
                        $value = $writable.GetValue($valueName)

                        if ($valueName.Equals($Tip, [System.StringComparison]::OrdinalIgnoreCase)) {
                            $deleteValue = $true
                        } elseif ($value -is [string[]]) {
                            $filtered = @($value | Where-Object { -not ([string]$_).Equals($Tip, [System.StringComparison]::OrdinalIgnoreCase) })
                            if ($filtered.Count -ne $value.Count) {
                                if ($filtered.Count -eq 0) {
                                    $deleteValue = $true
                                } else {
                                    $newValue = [string[]]$filtered
                                }
                            }
                        } elseif ($value -is [string]) {
                            $text = [string]$value
                            if ($text.Equals($Tip, [System.StringComparison]::OrdinalIgnoreCase)) {
                                $deleteValue = $true
                            } elseif ($text.IndexOf($Tip, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) {
                                $parts = @($text -split '[;,]' | Where-Object {
                                        -not ([string]$_).Trim().Equals($Tip, [System.StringComparison]::OrdinalIgnoreCase)
                                    })
                                if ($parts.Count -eq 0) {
                                    $deleteValue = $true
                                } else {
                                    $newValue = ($parts -join ';')
                                }
                            }
                        }

                        if ($deleteValue) {
                            $writable.DeleteValue($valueName, $false)
                            $removedValues++
                        } elseif ($null -ne $newValue) {
                            $writable.SetValue($valueName, $newValue, $kind)
                            $removedValues++
                        }
                    }
                } catch {
                    Write-UninstallLog ("WARN: loaded user registry cleanup skipped {0}: {1}" -f $key.Name, $_.Exception.Message)
                }
            }
        }
    }
    Write-UninstallLog ("OK: loaded user language registry cleanup complete; users={0} removed_values={1} removed_keys={2}" -f $visitedUsers, $removedValues, $removedKeys)
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
        [string]$Arch,
        [bool]$MachineScope
    )

    $registrationArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $RegistrationScript, '-DllPath', $DllPath, '-Unregister')
    if ($MachineScope) {
        $registrationArgs += '-Machine'
    }

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

    Write-UninstallLog ("BEGIN: {0} TIP unregistration DllPath={1} Machine={2}" -f $Arch, $DllPath, $MachineScope)
    & $powerShell @registrationArgs
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) {
        throw "$Arch TIP unregistration failed with exit code $exitCode"
    }
    Write-UninstallLog ("OK: {0} TIP unregistration complete" -f $Arch)
}

$InstallationRoot = [System.IO.Path]::GetFullPath($InstallationRoot)
if (-not $SkipCurrentUserCleanup) {
    & (Join-Path $InstallationRoot 'uninstall_current_user.ps1') -InstallationRoot $InstallationRoot `
        -SkipRegistration:($SkipRegistration -or $Machine) -SkipLanguageList:$SkipLanguageList -SkipRuntimeCleanup -FromMachineUninstaller:$Machine
}

$registrationScript = Join-Path $InstallationRoot 'invoke_registration.ps1'
$runtimePayloadRoot = Resolve-RuntimePayloadRoot -Root $InstallationRoot -AllowMissing
$runtimePayloadX64 = Resolve-RuntimePayloadArchRoot -Root $InstallationRoot -Arch x64 -AllowMissing
$runtimePayloadX86 = Resolve-RuntimePayloadArchRoot -Root $InstallationRoot -Arch x86 -AllowMissing
$installedDll = if ($runtimePayloadX64) { Join-Path $runtimePayloadX64 'srf_tsf_tip.dll' } elseif ($runtimePayloadRoot) { Join-Path $runtimePayloadRoot 'srf_tsf_tip.dll' } else { Join-Path $InstallationRoot 'srf_tsf_tip.dll' }
$installedDllX86 = if ($runtimePayloadX86) { Join-Path $runtimePayloadX86 'srf_tsf_tip.dll' } else { $null }

if (-not $SkipRegistration -and ($Machine -or $SkipCurrentUserCleanup) -and (Test-Path -LiteralPath $registrationScript) -and (Test-Path -LiteralPath $installedDll)) {
    try {
        Invoke-TipUnregistration -RegistrationScript $registrationScript -DllPath $installedDll -Arch x64 -MachineScope:$Machine.IsPresent
        if ($installedDllX86 -and (Test-Path -LiteralPath $installedDllX86)) {
            Invoke-TipUnregistration -RegistrationScript $registrationScript -DllPath $installedDllX86 -Arch x86 -MachineScope:$Machine.IsPresent
        } else {
            Write-UninstallLog 'SKIP: x86 TIP DLL missing for unregistration'
        }
    } catch {
        Write-UninstallLog ("WARN: TIP unregistration failed: {0}" -f $_.Exception.Message)
        Write-Warning $_
    }
} else {
    Write-UninstallLog 'SKIP: TIP unregistration not requested or registration files missing'
}

if (-not $SkipRegistration -and $Machine.IsPresent) {
    try {
        Remove-TipFromLoadedUserRegistry -Tip $SimplifiedChineseTip
    } catch {
        Write-UninstallLog ("WARN: loaded user language registry cleanup failed: {0}" -f $_.Exception.Message)
        Write-Warning $_
    }
}

if (-not $SkipUninstallEntry) {
    $keyPath = Get-UninstallKeyPath -MachineScope:$Machine.IsPresent
    if (Test-Path -Path $keyPath) {
        Remove-Item -Path $keyPath -Recurse -Force
        Write-UninstallLog ("OK: removed uninstall registry entry: {0}" -f $keyPath)
    } else {
        Write-UninstallLog ("OK: uninstall registry entry already absent: {0}" -f $keyPath)
    }
}

if ($Machine) {
    $machineData = Get-Item -LiteralPath (Get-MachineStateRoot) -Force -ErrorAction SilentlyContinue
    if ($machineData) {
        $isLink = [bool]($machineData.Attributes -band [IO.FileAttributes]::ReparsePoint)
        [IO.Directory]::Delete($machineData.FullName, (-not $isLink))
    }
}

if (-not $SkipFileRemoval) {
    $allowedRoots = @(
        [System.IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA ("Programs\" + $AppPathName))),
        [System.IO.Path]::GetFullPath((Join-Path ${env:ProgramFiles(x86)} $AppPathName))
    )
    $insideAllowedRoot = $false
    $normalizedInstallationRoot = $InstallationRoot.TrimEnd('\')
    foreach ($root in $allowedRoots) {
        $normalizedRoot = $root.TrimEnd('\')
        if ($normalizedInstallationRoot.Equals($normalizedRoot, [System.StringComparison]::OrdinalIgnoreCase) -or
            $normalizedInstallationRoot.StartsWith($normalizedRoot + '\', [System.StringComparison]::OrdinalIgnoreCase)) {
            $insideAllowedRoot = $true
            break
        }
    }
    if (-not $insideAllowedRoot) {
        throw "Refusing to remove unexpected install path: $InstallationRoot"
    }
    $sentinelFiles = @(
        (Join-Path $InstallationRoot 'uninstall_dev.ps1'),
        (Join-Path $InstallationRoot 'invoke_registration.ps1'),
        $installedDll
    )
    if (-not ($sentinelFiles | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1)) {
        throw "Refusing to remove $InstallationRoot because no install sentinel files were found."
    }
    Start-DeferredRemoval -TargetPath $InstallationRoot
    if (-not (Add-PendingDeleteOperations -Path $InstallationRoot)) {
        [void](Register-CurrentUserRunOnceRemoval -TargetPath $InstallationRoot)
    }
}

Write-UninstallLog ("OK: uninstall cleanup complete for {0}" -f $InstallationRoot)
Write-Output ("Uninstalled {0} from: {1}" -f $AppDisplayName, $InstallationRoot)

