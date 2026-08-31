param(
    [string]$PackageRoot = $PSScriptRoot,
    [string]$InstallationRoot,
    [switch]$Machine,
    [switch]$SkipFileCopy,
    [switch]$SkipRegistration,
    [switch]$SkipLanguageList,
    [switch]$SkipTextServiceRestart,
    [switch]$SkipTextInputUx,
    [switch]$SkipUninstallEntry,
    [switch]$SkipTraySetup,
    [switch]$SkipStaleHostRestart,
    [switch]$PromptStaleHostRestart,
    [switch]$SkipPackageIntegrityCheck,
    [switch]$SkipAclHardening,
    [switch]$SkipRuntimeCleanup,
    [switch]$SkipStaleHostDiagnostics,
    [switch]$SkipHealthCheck
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
$PackageManifestName = 'package_manifest.sha256'
$RuntimeSuccessManifestName = 'successful_runtime_payload.txt'
$RuntimePayloadRootName = 'runtime'
$CurrentConfigVersion = 13
$script:InstallLogPath = $null

function Get-DefaultInstallRoot {
    param([bool]$MachineScope)
    if ($MachineScope) {
        return Join-Path ${env:ProgramFiles(x86)} $AppPathName
    }
    return Join-Path $env:LOCALAPPDATA ("Programs\" + $AppPathName)
}

function Get-UserStateRoot {
    return Join-Path $env:LOCALAPPDATA $AppPathName
}

function Get-UserConfigPath {
    return Join-Path (Get-UserStateRoot) 'kaixin.ini'
}

function Ensure-Directory {
    param([string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path)) {
        return
    }
    if (-not (Test-Path -LiteralPath $Path)) {
        New-Item -ItemType Directory -Path $Path -Force | Out-Null
    }
}

function Write-InstallLog {
    param([string]$Message)

    if ([string]::IsNullOrWhiteSpace($script:InstallLogPath)) {
        return
    }

    try {
        $directory = Split-Path -Parent $script:InstallLogPath
        Ensure-Directory -Path $directory
        $line = ('{0:yyyy-MM-dd HH:mm:ss} {1}' -f (Get-Date), $Message)
        Add-Content -LiteralPath $script:InstallLogPath -Value $line -Encoding UTF8 -ErrorAction Stop
    } catch {
        # Best-effort logging only.
    }
}

function Remove-StalePerUserComRegistration {
    param(
        [string]$ExpectedDllX64,
        [string]$ExpectedDllX86
    )

    $entries = @(
        @{ Path = "HKCU:\Software\Classes\CLSID\$TextServiceClsid\InprocServer32"; Expected = $ExpectedDllX64 },
        @{ Path = "HKCU:\Software\Classes\WOW6432Node\CLSID\$TextServiceClsid\InprocServer32"; Expected = $ExpectedDllX86 }
    )

    foreach ($entry in $entries) {
        $keyPath = $entry.Path
        if (-not (Test-Path -LiteralPath $keyPath)) {
            continue
        }

        $actual = (Get-Item -LiteralPath $keyPath).GetValue('')
        $expected = [System.IO.Path]::GetFullPath($entry.Expected)
        $actualFull = $null
        if (-not [string]::IsNullOrWhiteSpace($actual)) {
            try {
                $actualFull = [System.IO.Path]::GetFullPath($actual)
            } catch {
                $actualFull = [string]$actual
            }
        }

        if ($actualFull -and $actualFull.Equals($expected, [System.StringComparison]::OrdinalIgnoreCase)) {
            continue
        }

        Remove-Item -LiteralPath (Split-Path -Parent $keyPath) -Recurse -Force -ErrorAction SilentlyContinue
        Write-InstallLog ("OK: removed stale per-user COM registration {0} -> {1}" -f $keyPath, $actual)
    }
}

function Get-DefaultInstallLogPath {
    param(
        [string]$InstallRoot,
        [bool]$MachineScope
    )

    if ($MachineScope) {
        return Join-Path $InstallRoot 'install_machine.log'
    }

    return Join-Path (Get-UserStateRoot) 'install_user.log'
}

function Get-LanguageListLogPath {
    return Join-Path (Get-UserStateRoot) 'install_language_list.log'
}

function Ensure-File {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) {
        throw "Missing required file: $Path"
    }
}

function Test-IniHasKey {
    param(
        [string[]]$Lines,
        [string]$Section,
        [string]$Key
    )

    $inSection = $false
    foreach ($line in $Lines) {
        if ($line -match '^\s*\[(.+)\]\s*$') {
            $inSection = $matches[1].Equals($Section, [System.StringComparison]::OrdinalIgnoreCase)
            continue
        }
        if ($inSection -and $line -match ('^\s*' + [regex]::Escape($Key) + '\s*=')) {
            return $true
        }
    }
    return $false
}

function Get-IniValue {
    param(
        [string[]]$Lines,
        [string]$Section,
        [string]$Key
    )

    $inSection = $false
    foreach ($line in $Lines) {
        if ($line -match '^\s*\[(.+)\]\s*$') {
            $inSection = $matches[1].Equals($Section, [System.StringComparison]::OrdinalIgnoreCase)
            continue
        }
        if ($inSection -and $line -match ('^\s*' + [regex]::Escape($Key) + '\s*=\s*(.*)$')) {
            return $matches[1].Trim()
        }
    }
    return $null
}

function Set-IniValue {
    param(
        [ref]$Lines,
        [string]$Section,
        [string]$Key,
        [string]$Value
    )

    $currentLines = [string[]]$Lines.Value
    $updated = New-Object 'System.Collections.Generic.List[string]'
    $inSection = $false
    $changed = $false

    foreach ($line in $currentLines) {
        if ($line -match '^\s*\[(.+)\]\s*$') {
            $inSection = $matches[1].Equals($Section, [System.StringComparison]::OrdinalIgnoreCase)
            [void]$updated.Add($line)
            continue
        }
        if ($inSection -and $line -match ('^\s*' + [regex]::Escape($Key) + '\s*=')) {
            $newLine = ('{0}={1}' -f $Key, $Value)
            [void]$updated.Add($newLine)
            if (-not $line.Equals($newLine, [System.StringComparison]::Ordinal)) {
                $changed = $true
            }
            continue
        }
        [void]$updated.Add($line)
    }

    if ($changed) {
        $Lines.Value = [string[]]$updated
    }
    return $changed
}

function Remove-IniKey {
    param(
        [ref]$Lines,
        [string]$Section,
        [string]$Key
    )

    $currentLines = [string[]]$Lines.Value
    $updated = New-Object 'System.Collections.Generic.List[string]'
    $inSection = $false
    $changed = $false

    foreach ($line in $currentLines) {
        if ($line -match '^\s*\[(.+)\]\s*$') {
            $inSection = $matches[1].Equals($Section, [System.StringComparison]::OrdinalIgnoreCase)
            [void]$updated.Add($line)
            continue
        }
        if ($inSection -and $line -match ('^\s*' + [regex]::Escape($Key) + '\s*=')) {
            $changed = $true
            continue
        }
        [void]$updated.Add($line)
    }

    if ($changed) {
        $Lines.Value = [string[]]$updated
    }
    return $changed
}

function Set-IniValueOrAdd {
    param(
        [ref]$Lines,
        [string]$Section,
        [string]$Key,
        [string]$Value
    )

    if (Set-IniValue -Lines $Lines -Section $Section -Key $Key -Value $Value) {
        return $true
    }
    return Add-IniDefaultIfMissing -Lines $Lines -Section $Section -Key $Key -Value $Value
}

function Add-IniDefaultIfMissing {
    param(
        [ref]$Lines,
        [string]$Section,
        [string]$Key,
        [string]$Value
    )

    $currentLines = [string[]]$Lines.Value
    if (Test-IniHasKey -Lines $currentLines -Section $Section -Key $Key) {
        return $false
    }

    $sectionStart = -1
    $insertAt = $currentLines.Count
    for ($i = 0; $i -lt $currentLines.Count; $i++) {
        if ($currentLines[$i] -match '^\s*\[(.+)\]\s*$') {
            if ($matches[1].Equals($Section, [System.StringComparison]::OrdinalIgnoreCase)) {
                $sectionStart = $i
                $insertAt = $currentLines.Count
                for ($j = $i + 1; $j -lt $currentLines.Count; $j++) {
                    if ($currentLines[$j] -match '^\s*\[.+\]\s*$') {
                        $insertAt = $j
                        break
                    }
                }
                break
            }
        }
    }

    $updated = New-Object 'System.Collections.Generic.List[string]'
    if ($sectionStart -ge 0) {
        for ($i = 0; $i -lt $currentLines.Count; $i++) {
            if ($i -eq $insertAt) {
                [void]$updated.Add(('{0}={1}' -f $Key, $Value))
            }
            [void]$updated.Add($currentLines[$i])
        }
        if ($insertAt -eq $currentLines.Count) {
            [void]$updated.Add(('{0}={1}' -f $Key, $Value))
        }
    } else {
        foreach ($line in $currentLines) {
            [void]$updated.Add($line)
        }
        if ($updated.Count -gt 0 -and -not [string]::IsNullOrWhiteSpace($updated[$updated.Count - 1])) {
            [void]$updated.Add('')
        }
        [void]$updated.Add(('[' + $Section + ']'))
        [void]$updated.Add(('{0}={1}' -f $Key, $Value))
    }

    $Lines.Value = [string[]]$updated
    return $true
}

function Get-AppIniSections {
    param(
        [string[]]$Lines
    )

    $sections = New-Object 'System.Collections.Generic.List[string]'
    foreach ($line in $Lines) {
        if ($line -match '^\s*\[(.+)\]\s*$') {
            $section = $matches[1].Trim()
            if ($section.StartsWith('app:', [System.StringComparison]::OrdinalIgnoreCase) -and -not $sections.Contains($section)) {
                [void]$sections.Add($section)
            }
        }
    }
    return [string[]]$sections
}

function Migrate-AppSectionAlias {
    param(
        [ref]$Lines,
        [string]$Section,
        [string]$CanonicalKey,
        [string[]]$Aliases
    )

    $changed = $false
    if (-not (Test-IniHasKey -Lines $Lines.Value -Section $Section -Key $CanonicalKey)) {
        foreach ($alias in $Aliases) {
            $value = Get-IniValue -Lines $Lines.Value -Section $Section -Key $alias
            if ($null -eq $value) {
                continue
            }
            if (Set-IniValueOrAdd -Lines $Lines -Section $Section -Key $CanonicalKey -Value $value) {
                $changed = $true
            }
            break
        }
    }
    foreach ($alias in $Aliases) {
        if (Remove-IniKey -Lines $Lines -Section $Section -Key $alias) {
            $changed = $true
        }
    }
    return $changed
}

function Save-TextAtomically {
    param(
        [string]$Path,
        [string[]]$Lines
    )

    $directory = Split-Path -Parent $Path
    Ensure-Directory -Path $directory
    $tempPath = Join-Path $directory ('.' + [System.IO.Path]::GetFileName($Path) + '.tmp')
    Set-Content -LiteralPath $tempPath -Value $Lines -Encoding UTF8
    Move-Item -LiteralPath $tempPath -Destination $Path -Force
}

function Backup-UserConfigForMigration {
    param([string]$ConfigPath)

    if (-not (Test-Path -LiteralPath $ConfigPath)) {
        return
    }

    $versionPath = Join-Path $InstallationRoot 'VERSION'
    $version = 'unknown'
    if (Test-Path -LiteralPath $versionPath) {
        $version = (Get-Content -LiteralPath $versionPath -Raw -Encoding UTF8 -ErrorAction SilentlyContinue).Trim()
    }
    if ([string]::IsNullOrWhiteSpace($version)) {
        $version = 'unknown'
    }
    $safeVersion = ($version -replace '[^0-9A-Za-z._-]', '_')
    $backupPath = ('{0}.backup.{1}.{2}' -f $ConfigPath, $safeVersion, (Get-Date -Format 'yyyyMMdd-HHmmss'))

    try {
        Copy-Item -LiteralPath $ConfigPath -Destination $backupPath -Force -ErrorAction Stop
        Write-InstallLog ("OK: backed up user config before migration: {0}" -f $backupPath)
    } catch {
        Write-InstallLog ("WARN: could not back up user config before migration: {0}" -f $_.Exception.Message)
    }
}

function Invoke-ConfigMigration {
    $configPath = Get-UserConfigPath
    if (-not (Test-Path -LiteralPath $configPath)) {
        Write-InstallLog 'SKIP: user config migration; config file does not exist yet'
        return
    }

    $lines = [string[]](Get-Content -LiteralPath $configPath -ErrorAction Stop)
    $changed = $false
    $existingConfigVersion = 0
    $configVersionText = Get-IniValue -Lines $lines -Section 'general' -Key 'config_version'
    if (-not [string]::IsNullOrWhiteSpace($configVersionText)) {
        [void][int]::TryParse($configVersionText, [ref]$existingConfigVersion)
    }

    if ($existingConfigVersion -lt 4) {
        $screenshotAutoSave = Get-IniValue -Lines $lines -Section 'screenshot' -Key 'auto_save'
        if ([string]::IsNullOrWhiteSpace($screenshotAutoSave) -or $screenshotAutoSave.Trim().Equals('0', [System.StringComparison]::OrdinalIgnoreCase)) {
            if (Set-IniValue -Lines ([ref]$lines) -Section 'screenshot' -Key 'auto_save' -Value '1') {
                $changed = $true
                Write-InstallLog 'OK: migrated screenshot auto_save old default 0 -> 1'
            }
        }
    }

    if ($existingConfigVersion -lt 5) {
        $longLookupSoftBudget = Get-IniValue -Lines $lines -Section 'engine' -Key 'long_lookup_soft_budget_ms'
        if ([string]::IsNullOrWhiteSpace($longLookupSoftBudget) -or $longLookupSoftBudget.Trim().Equals('4', [System.StringComparison]::OrdinalIgnoreCase)) {
            if (Set-IniValue -Lines ([ref]$lines) -Section 'engine' -Key 'long_lookup_soft_budget_ms' -Value '16') {
                $changed = $true
                Write-InstallLog 'OK: migrated long_lookup_soft_budget_ms old default 4 -> 16'
            }
        }
    }


    if ($existingConfigVersion -lt 10) {
        $canonicalizedLegacyKeys = $false
        if (Remove-IniKey -Lines ([ref]$lines) -Section 'style' -Key 'candidate_layout_variant') {
            $canonicalizedLegacyKeys = $true
        }
        foreach ($appSection in @(Get-AppIniSections -Lines $lines)) {
            $canonicalizedLegacyKeys = (Migrate-AppSectionAlias -Lines ([ref]$lines) -Section $appSection -CanonicalKey 'ascii_mode' -Aliases @('ascii')) -or $canonicalizedLegacyKeys
            $canonicalizedLegacyKeys = (Migrate-AppSectionAlias -Lines ([ref]$lines) -Section $appSection -CanonicalKey 'hide_ui' -Aliases @('hide')) -or $canonicalizedLegacyKeys
            $canonicalizedLegacyKeys = (Migrate-AppSectionAlias -Lines ([ref]$lines) -Section $appSection -CanonicalKey 'inline_preedit' -Aliases @('inline')) -or $canonicalizedLegacyKeys
            $canonicalizedLegacyKeys = (Migrate-AppSectionAlias -Lines ([ref]$lines) -Section $appSection -CanonicalKey 'enhanced_position' -Aliases @('position')) -or $canonicalizedLegacyKeys
            $canonicalizedLegacyKeys = (Migrate-AppSectionAlias -Lines ([ref]$lines) -Section $appSection -CanonicalKey 'candidate_topmost' -Aliases @('topmost')) -or $canonicalizedLegacyKeys
            $canonicalizedLegacyKeys = (Migrate-AppSectionAlias -Lines ([ref]$lines) -Section $appSection -CanonicalKey 'focus_policy' -Aliases @('focus')) -or $canonicalizedLegacyKeys
            $canonicalizedLegacyKeys = (Migrate-AppSectionAlias -Lines ([ref]$lines) -Section $appSection -CanonicalKey 'commit_transport' -Aliases @('commit', 'transport')) -or $canonicalizedLegacyKeys
            $canonicalizedLegacyKeys = (Migrate-AppSectionAlias -Lines ([ref]$lines) -Section $appSection -CanonicalKey 'game_profile' -Aliases @('game', 'profile', 'candidate_profile')) -or $canonicalizedLegacyKeys
            $canonicalizedLegacyKeys = (Migrate-AppSectionAlias -Lines ([ref]$lines) -Section $appSection -CanonicalKey 'overlay_anchor' -Aliases @('anchor')) -or $canonicalizedLegacyKeys
            $canonicalizedLegacyKeys = (Migrate-AppSectionAlias -Lines ([ref]$lines) -Section $appSection -CanonicalKey 'overlay_offset_x' -Aliases @('offset_x')) -or $canonicalizedLegacyKeys
            $canonicalizedLegacyKeys = (Migrate-AppSectionAlias -Lines ([ref]$lines) -Section $appSection -CanonicalKey 'overlay_offset_y' -Aliases @('offset_y')) -or $canonicalizedLegacyKeys
            $canonicalizedLegacyKeys = (Migrate-AppSectionAlias -Lines ([ref]$lines) -Section $appSection -CanonicalKey 'overlay_scale' -Aliases @('scale')) -or $canonicalizedLegacyKeys
            $canonicalizedLegacyKeys = (Migrate-AppSectionAlias -Lines ([ref]$lines) -Section $appSection -CanonicalKey 'overlay_monitor' -Aliases @('monitor')) -or $canonicalizedLegacyKeys
            $canonicalizedLegacyKeys = (Migrate-AppSectionAlias -Lines ([ref]$lines) -Section $appSection -CanonicalKey 'overlay_backend' -Aliases @('backend')) -or $canonicalizedLegacyKeys
        }
        if ($canonicalizedLegacyKeys) {
            $changed = $true
            Write-InstallLog 'OK: canonicalized legacy app rule aliases and removed obsolete config keys'
        }
    }

    $defaults = @(
        @{ Section = 'general'; Key = 'config_version'; Value = [string]$CurrentConfigVersion },
        @{ Section = 'diagnostics'; Key = 'log_level'; Value = 'basic' },
        @{ Section = 'style'; Key = 'candidate_horizontal'; Value = '1' },
        @{ Section = 'style'; Key = 'candidate_density'; Value = 'comfortable' },
        @{ Section = 'style'; Key = 'candidate_vertical_layout_variant'; Value = 'compact' },
        @{ Section = 'style'; Key = 'candidate_horizontal_layout_variant'; Value = 'classic' },
        @{ Section = 'compatibility'; Key = 'fullscreen_detection'; Value = '1' },
        @{ Section = 'compatibility'; Key = 'fullscreen_policy'; Value = 'show_ui' },
        @{ Section = 'compatibility'; Key = 'commit_transport'; Value = 'tsf' },
        @{ Section = 'compatibility'; Key = 'builtin_game_list'; Value = '1' },
        @{ Section = 'compatibility'; Key = 'auto_suggest_app_options'; Value = '1' },
        @{ Section = 'privacy'; Key = 'never_learn_processes'; Value = '' },
        @{ Section = 'privacy'; Key = 'never_clipboard_processes'; Value = '' },
        @{ Section = 'privacy'; Key = 'never_candidate_processes'; Value = '' },
        @{ Section = 'clipboard'; Key = 'background_enabled'; Value = '1' },
        @{ Section = 'clipboard'; Key = 'max_history_items'; Value = '60' },
        @{ Section = 'clipboard'; Key = 'max_pinned_items'; Value = '24' },
        @{ Section = 'clipboard'; Key = 'max_text_utf16_units'; Value = '20000' },
        @{ Section = 'clipboard'; Key = 'max_age_days'; Value = '0' },
        @{ Section = 'screenshot'; Key = 'hotkey'; Value = 'off' },
        @{ Section = 'screenshot'; Key = 'auto_save'; Value = '1' },
        @{ Section = 'screenshot'; Key = 'save_dir'; Value = '' },
        @{ Section = 'screenshot'; Key = 'silent_copy_enabled'; Value = '0' },
        @{ Section = 'screenshot'; Key = 'silent_copy_dir'; Value = '' },
        @{ Section = 'screenshot'; Key = 'name_pattern'; Value = '{datetime}' },
        @{ Section = 'screenshot'; Key = 'format'; Value = 'png' },
        @{ Section = 'screenshot'; Key = 'copy_after_capture'; Value = '1' },
        @{ Section = 'screenshot'; Key = 'ocr_after_capture'; Value = '0' },
        @{ Section = 'screenshot'; Key = 'translate_after_capture'; Value = '0' },
        @{ Section = 'screenshot'; Key = 'mode'; Value = 'manual_region' },
        @{ Section = 'screenshot'; Key = 'confirm_on_release'; Value = '0' },
        @{ Section = 'screenshot'; Key = 'show_instructions'; Value = '1' },
        @{ Section = 'clipboard'; Key = 'hotkey'; Value = 'off' },
        @{ Section = 'tools'; Key = 'settings_hotkey'; Value = 'off' },
        @{ Section = 'tools'; Key = 'handwrite_hotkey'; Value = 'off' },
        @{ Section = 'tools'; Key = 'ocr_hotkey'; Value = 'off' },
        @{ Section = 'tools'; Key = 'translate_hotkey'; Value = 'off' },
        @{ Section = 'input'; Key = 'traditional_hotkey'; Value = 'off' },
        @{ Section = 'input'; Key = 'game_mode_hotkey'; Value = 'off' },
        @{ Section = 'input'; Key = 'temporary_ascii_hotkey'; Value = 'off' },
        @{ Section = 'input'; Key = 'shift_tap_hotkey'; Value = '1' },
        @{ Section = 'input'; Key = 'candidate_number_select'; Value = '1' },
        @{ Section = 'input'; Key = 'symbol_fullwidth'; Value = '0' },
        @{ Section = 'input'; Key = 'shift_symbol_temporary_ascii'; Value = '0' },
        @{ Section = 'input'; Key = 'page_minus_equal'; Value = '1' },
        @{ Section = 'input'; Key = 'page_comma_period'; Value = '1' },
        @{ Section = 'engine'; Key = 'retry_on_failure'; Value = '1' },
        @{ Section = 'engine'; Key = 'long_lookup_soft_budget_ms'; Value = '16' }
    )

    foreach ($entry in $defaults) {
        if (Add-IniDefaultIfMissing -Lines ([ref]$lines) -Section $entry.Section -Key $entry.Key -Value $entry.Value) {
            $changed = $true
        }
    }
    if (Set-IniValue -Lines ([ref]$lines) -Section 'general' -Key 'config_version' -Value ([string]$CurrentConfigVersion)) {
        $changed = $true
    }

    if ($changed) {
        Backup-UserConfigForMigration -ConfigPath $configPath
        Save-TextAtomically -Path $configPath -Lines $lines
        Write-InstallLog ("OK: migrated user config defaults and canonicalized legacy keys: {0}" -f $configPath)
    } else {
        Write-InstallLog 'OK: user config migration not needed'
    }
}

function Get-PackagedLexiconDirectories {
    param([string]$Root)

    $dirs = @()

    $fixed = Join-Path $Root 'lexicon'
    if (Test-Path -LiteralPath $fixed) {
        $dirs += Get-Item -LiteralPath $fixed
    }

    return @($dirs | Sort-Object FullName -Unique)
}

function Stop-RunningHelpers {
    $backgroundHelpers = @('srf_ime_engine', 'srf_ime_tray')
    $visibleTools = @('srf_ime_settings', 'srf_ime_clipboard', 'srf_ime_handwrite', 'srf_ime_ocr', 'srf_ime_translate_result')
    foreach ($processName in @($backgroundHelpers + $visibleTools)) {
        $processes = @(Get-Process -Name $processName -ErrorAction SilentlyContinue)
        foreach ($process in $processes) {
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
        $processes = @(Get-Process -Name $processName -ErrorAction SilentlyContinue)
        foreach ($process in $processes) {
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
            Write-InstallLog ("WARN: visible helper app is still running and was not force-closed: {0}({1})" -f $process.ProcessName, $process.Id)
        }
    }
    Write-InstallLog 'OK: stopped running tray/settings helpers before install'
}

function Stop-TextInputStackForInstall {
    foreach ($processName in @('TextInputHost', 'ctfmon')) {
        $processes = @(Get-Process -Name $processName -ErrorAction SilentlyContinue)
        foreach ($process in $processes) {
            try {
                if (-not $process.HasExited) {
                    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
                }
            } catch {
            }
        }
    }
    Write-InstallLog 'OK: stopped text input stack before file replacement'
}

function Clear-TransientUserState {
    $stateRoot = Get-UserStateRoot
    if (-not (Test-Path -LiteralPath $stateRoot)) {
        return
    }

    $resolvedStateRoot = [System.IO.Path]::GetFullPath($stateRoot)
    $expectedStateRoot = [System.IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA $AppPathName))
    if (-not $resolvedStateRoot.Equals($expectedStateRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to clean unexpected user state path: $resolvedStateRoot"
    }

    $transientNames = @(
        'cache',
        'logs',
        'install_user.log',
        'install_language_list.log'
    )
    foreach ($item in Get-ChildItem -LiteralPath $resolvedStateRoot -Force -ErrorAction SilentlyContinue) {
        if ($transientNames -contains $item.Name) {
            Remove-Item -LiteralPath $item.FullName -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
    Write-InstallLog 'OK: cleaned known transient user state while preserving all other settings and user data'
}

function Clear-InstallRootForPureCopy {
    param(
        [string]$InstallRoot,
        [string]$SourceRoot
    )

    if (-not (Test-Path -LiteralPath $InstallRoot)) {
        return
    }

    $resolvedInstallRoot = [System.IO.Path]::GetFullPath($InstallRoot)
    $resolvedSourceRoot = [System.IO.Path]::GetFullPath($SourceRoot)
    if ($resolvedInstallRoot.Equals($resolvedSourceRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        Write-InstallLog 'SKIP: install root equals package root; not clearing source files'
        return
    }

    Assert-TrustedInstallRoot -Path $resolvedInstallRoot
    foreach ($item in Get-ChildItem -LiteralPath $resolvedInstallRoot -Force -ErrorAction SilentlyContinue) {
        Remove-Item -LiteralPath $item.FullName -Recurse -Force -ErrorAction SilentlyContinue
    }
    Write-InstallLog 'OK: cleaned previous install root before copying package files'
}

function Assert-TrustedInstallRoot {
    param([string]$Path)

    $normalized = [System.IO.Path]::GetFullPath($Path)
    $trustedRoots = @(
        [System.IO.Path]::GetFullPath((Join-Path ${env:ProgramFiles(x86)} $AppPathName)),
        [System.IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA ("Programs\" + $AppPathName)))
    ) | Select-Object -Unique

    foreach ($root in $trustedRoots) {
        if ($normalized.Equals($root, [System.StringComparison]::OrdinalIgnoreCase)) {
            return
        }
    }

    throw "Refusing to install to untrusted path: $normalized"
}

function Read-ExpectedSha256 {
    param([string]$Path)

    $raw = (Get-Content -LiteralPath $Path -Raw -Encoding ASCII).Trim().ToLowerInvariant()
    if ($raw -notmatch '^[0-9a-f]{64}$') {
        throw "Invalid SHA-256 manifest: $Path"
    }
    return $raw
}

function Assert-FileHash {
    param(
        [string]$FilePath,
        [string]$Sha256Path
    )

    $expected = Read-ExpectedSha256 -Path $Sha256Path
    $actual = (Get-FileHash -LiteralPath $FilePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) {
        throw "SHA-256 mismatch for $FilePath"
    }
}

function Get-PackageFileHashInfo {
    param([string]$Path)

    $hash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
    $item = Get-Item -LiteralPath $Path -ErrorAction Stop
    return [pscustomobject]@{
        Hash = $hash
        Length = [int64]$item.Length
        LastWriteTimeUtc = $item.LastWriteTimeUtc.ToString('o')
    }
}

function Try-ApplyPendingInstallerReplacement {
    param(
        [string]$Path,
        [string]$Relative,
        [string]$Expected
    )

    $dir = Split-Path -Parent $Path
    if ([string]::IsNullOrWhiteSpace($dir) -or -not (Test-Path -LiteralPath $dir)) {
        return $false
    }

    foreach ($candidate in @(Get-ChildItem -LiteralPath $dir -Filter 'is-*.tmp' -File -ErrorAction SilentlyContinue)) {
        $candidateInfo = $null
        try {
            $candidateInfo = Get-PackageFileHashInfo -Path $candidate.FullName
        } catch {
            continue
        }
        if (-not $candidateInfo.Hash.Equals($Expected, [System.StringComparison]::OrdinalIgnoreCase)) {
            continue
        }

        Write-InstallLog ("WARN: applying pending installer replacement for {0} from {1}" -f $Relative, $candidate.Name)
        Stop-RunningHelpers
        try {
            Move-Item -LiteralPath $candidate.FullName -Destination $Path -Force -ErrorAction Stop
            $fixedInfo = Get-PackageFileHashInfo -Path $Path
            if ($fixedInfo.Hash.Equals($Expected, [System.StringComparison]::OrdinalIgnoreCase)) {
                Write-InstallLog ("OK: applied pending installer replacement for {0}" -f $Relative)
                return $true
            }
            Write-InstallLog ("WARN: pending installer replacement for {0} still mismatched after move; actual={1}" -f $Relative, $fixedInfo.Hash)
        } catch {
            Write-InstallLog ("WARN: failed to apply pending installer replacement for {0}: {1}" -f $Relative, $_.Exception.Message)
        }
    }

    return $false
}

function Assert-PackageFileHash {
    param(
        [string]$Path,
        [string]$Relative,
        [string]$Expected,
        [string]$ManifestPath
    )

    $maxAttempts = 5
    $lastInfo = $null
    $lastError = $null
    for ($attempt = 1; $attempt -le $maxAttempts; $attempt++) {
        try {
            $lastInfo = Get-PackageFileHashInfo -Path $Path
            $lastError = $null
            if ($lastInfo.Hash.Equals($Expected, [System.StringComparison]::OrdinalIgnoreCase)) {
                return
            }
            if (Try-ApplyPendingInstallerReplacement -Path $Path -Relative $Relative -Expected $Expected) {
                return
            }
            if ($attempt -lt $maxAttempts) {
                Write-InstallLog ("WARN: package hash mismatch attempt={0}/{1} file={2} expected={3} actual={4} size={5} last_write_utc={6}" -f
                    $attempt, $maxAttempts, $Relative, $Expected, $lastInfo.Hash, $lastInfo.Length, $lastInfo.LastWriteTimeUtc)
            }
        } catch {
            $lastError = $_.Exception.Message
            if ($attempt -lt $maxAttempts) {
                Write-InstallLog ("WARN: package hash read failed attempt={0}/{1} file={2} error={3}" -f
                    $attempt, $maxAttempts, $Relative, $lastError)
            }
        }
        if ($attempt -lt $maxAttempts) {
            Start-Sleep -Milliseconds (200 * $attempt)
        }
    }

    $actual = '(unavailable)'
    $size = '(unavailable)'
    $lastWrite = '(unavailable)'
    if ($null -ne $lastInfo) {
        $actual = $lastInfo.Hash
        $size = $lastInfo.Length
        $lastWrite = $lastInfo.LastWriteTimeUtc
    }
    $errorSuffix = ''
    if (-not [string]::IsNullOrWhiteSpace($lastError)) {
        $errorSuffix = ('; hash_error={0}' -f $lastError)
    }
    throw ("Package integrity check failed: {0}; expected={1}; actual={2}; size={3}; last_write_utc={4}; manifest={5}{6}" -f
        $Relative, $Expected, $actual, $size, $lastWrite, $ManifestPath, $errorSuffix)
}

function Test-PathWithinRoot {
    param(
        [string]$Root,
        [string]$Path
    )

    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\')
    $pathFull = [System.IO.Path]::GetFullPath($Path).TrimEnd('\')
    return $pathFull.Equals($rootFull, [System.StringComparison]::OrdinalIgnoreCase) -or
        $pathFull.StartsWith($rootFull + '\', [System.StringComparison]::OrdinalIgnoreCase)
}

function Assert-PackageHashManifest {
    param([string]$Root)

    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\')
    $manifestPath = Join-Path $rootFull $PackageManifestName
    Ensure-File $manifestPath

    $lines = @(Get-Content -LiteralPath $manifestPath -Encoding UTF8 -ErrorAction Stop)
    $verified = 0
    foreach ($line in $lines) {
        $trimmed = $line.Trim()
        if ([string]::IsNullOrWhiteSpace($trimmed) -or $trimmed.StartsWith('#')) {
            continue
        }
        if ($trimmed -notmatch '^([0-9a-fA-F]{64}) \*(.+)$') {
            throw "Invalid package manifest line: $trimmed"
        }
        $expected = $matches[1].ToLowerInvariant()
        $relative = $matches[2].Replace('/', '\')
        if ($relative.Contains(':') -or
            [System.IO.Path]::IsPathRooted($relative) -or
            $relative.Split('\') -contains '..') {
            throw "Unsafe package manifest path: $relative"
        }
        $path = Join-Path $rootFull $relative
        if (-not (Test-PathWithinRoot -Root $rootFull -Path $path)) {
            throw "Package manifest path escapes package root: $relative"
        }
        try {
            Ensure-File $path
            Assert-PackageFileHash -Path $path -Relative $relative -Expected $expected -ManifestPath $manifestPath
        } catch {
            # A component marker is only an upgrade optimization. Remove it on
            # any integrity failure so rerunning the installer performs a full
            # component refresh instead of repeatedly skipping damaged files.
            Remove-Item -LiteralPath (Join-Path $rootFull 'component_manifest.ini') -Force -ErrorAction SilentlyContinue
            throw
        }
        $verified++
    }
    if ($verified -le 0) {
        throw "Package integrity manifest is empty: $manifestPath"
    }
    Write-InstallLog ("OK: verified package integrity manifest entries={0}" -f $verified)
}

function Protect-DirectoryAclBestEffort {
    param(
        [string]$Path,
        [switch]$ReadExecuteForBuiltinUsers
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }
    $icacls = Join-Path $env:WINDIR 'System32\icacls.exe'
    if (-not (Test-Path -LiteralPath $icacls)) {
        Write-InstallLog ("WARN: icacls.exe not found; skipped ACL hardening for {0}" -f $Path)
        return
    }
    $sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $grants = @(
        "*$($sid):(OI)(CI)F",
        '*S-1-5-18:(OI)(CI)F',
        '*S-1-5-32-544:(OI)(CI)F'
    )
    if ($ReadExecuteForBuiltinUsers) {
        $grants += '*S-1-5-32-545:(OI)(CI)RX'
    }
    try {
        $args = @($Path, '/inheritance:r')
        foreach ($grant in $grants) {
            $args += @('/grant:r', $grant)
        }
        $args += @('/C', '/Q')
        & $icacls @args | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Write-InstallLog ("WARN: ACL hardening returned exit code {0} for {1}" -f $LASTEXITCODE, $Path)
        } else {
            Write-InstallLog ("OK: hardened ACL for {0}" -f $Path)
        }

        $aclMarker = Join-Path $Path '.kaixin-acl-policy-v1'
        if (-not (Test-Path -LiteralPath $aclMarker)) {
            $children = Join-Path $Path '*'
            & $icacls $children /reset /T /C /Q | Out-Null
            if ($LASTEXITCODE -ne 0) {
                Write-InstallLog ("WARN: ACL child reset returned exit code {0} for {1}" -f $LASTEXITCODE, $Path)
            } else {
                Set-Content -LiteralPath $aclMarker -Value '1' -Encoding ASCII
                Write-InstallLog ("OK: reset child ACL inheritance for {0}" -f $Path)
            }
        } else {
            Write-InstallLog ("OK: ACL policy marker is current for {0}; skipped recursive child reset" -f $Path)
        }
    } catch {
        Write-InstallLog ("WARN: ACL hardening failed for {0}: {1}" -f $Path, $_.Exception.Message)
    }
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

function ConvertTo-InstallRelativePath {
    param(
        [string]$InstallRoot,
        [string]$Path
    )

    $rootFull = [System.IO.Path]::GetFullPath($InstallRoot).TrimEnd('\')
    $pathFull = [System.IO.Path]::GetFullPath($Path).TrimEnd('\')
    $rootPrefix = $rootFull + '\'
    if ($pathFull.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        return $pathFull.Substring($rootPrefix.Length)
    }
    return $pathFull
}

function Resolve-SuccessfulRuntimePayloadRoot {
    param([string]$InstallRoot)

    $marker = Join-Path $InstallRoot $RuntimeSuccessManifestName
    if (-not (Test-Path -LiteralPath $marker)) {
        return $null
    }
    $relative = (Get-Content -LiteralPath $marker -Raw -Encoding ASCII).Trim()
    if ([string]::IsNullOrWhiteSpace($relative)) {
        return $null
    }
    $candidate = [System.IO.Path]::GetFullPath((Join-Path $InstallRoot $relative))
    if (-not (Test-Path -LiteralPath $candidate)) {
        return $null
    }
    return $candidate
}

function Write-RuntimePayloadMarker {
    param(
        [string]$InstallRoot,
        [string]$PayloadRoot
    )

    $relative = ConvertTo-InstallRelativePath -InstallRoot $InstallRoot -Path $PayloadRoot
    [System.IO.File]::WriteAllText((Join-Path $InstallRoot $RuntimePayloadManifestName), $relative, [System.Text.Encoding]::ASCII)
    [System.IO.File]::WriteAllText((Join-Path $InstallRoot $RuntimeSuccessManifestName), $relative, [System.Text.Encoding]::ASCII)
}

function Try-RollbackRuntimePayload {
    param(
        [string]$InstallRoot,
        [string]$PreviousPayloadRoot,
        [string]$RegistrationScript,
        [bool]$MachineScope
    )

    if ([string]::IsNullOrWhiteSpace($PreviousPayloadRoot) -or -not (Test-Path -LiteralPath $PreviousPayloadRoot)) {
        Write-InstallLog 'WARN: no previous successful runtime payload available for rollback'
        return $false
    }

    try {
        $previousX64 = Join-Path $PreviousPayloadRoot 'x64'
        $previousX86 = Join-Path $PreviousPayloadRoot 'x86'
        if (-not (Test-Path -LiteralPath $previousX64)) { $previousX64 = $PreviousPayloadRoot }
        $previousDll = Join-Path $previousX64 'srf_tsf_tip.dll'
        $previousDllX86 = Join-Path $previousX86 'srf_tsf_tip.dll'
        Ensure-File $previousDll
        Ensure-File $previousDllX86
        Write-RuntimePayloadMarker -InstallRoot $InstallRoot -PayloadRoot $PreviousPayloadRoot
        Invoke-TipRegistration -RegistrationScript $RegistrationScript -DllPath $previousDll -Arch x64 -MachineScope:$MachineScope
        Invoke-TipRegistration -RegistrationScript $RegistrationScript -DllPath $previousDllX86 -Arch x86 -MachineScope:$MachineScope
        Write-InstallLog ("OK: rolled back to previous runtime payload {0}" -f $PreviousPayloadRoot)
        return $true
    } catch {
        Write-InstallLog ("FAIL: runtime payload rollback failed: {0}" -f $_.Exception.Message)
        return $false
    }
}

function Invoke-TipRegistration {
    param(
        [string]$RegistrationScript,
        [string]$DllPath,
        [ValidateSet('x64', 'x86')]
        [string]$Arch,
        [bool]$MachineScope
    )

    $scriptArgs = @('-DllPath', $DllPath)
    if ($MachineScope) {
        $scriptArgs += '-Machine'
    }

    if ($Arch -eq 'x86') {
        $powerShell = Join-Path $env:WINDIR 'SysWOW64\WindowsPowerShell\v1.0\powershell.exe'
        if (-not (Test-Path -LiteralPath $powerShell)) {
            Write-InstallLog ("WARN: 32-bit PowerShell not found at {0}; falling back to powershell.exe" -f $powerShell)
            $powerShell = 'powershell.exe'
        }
    } else {
        $powerShell = Join-Path $env:WINDIR 'System32\WindowsPowerShell\v1.0\powershell.exe'
        if (-not (Test-Path -LiteralPath $powerShell)) {
            Write-InstallLog ("WARN: 64-bit PowerShell not found at {0}; falling back to powershell.exe" -f $powerShell)
            $powerShell = 'powershell.exe'
        }
    }

    $processArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $RegistrationScript) + $scriptArgs
    Write-InstallLog ("BEGIN: {0} TIP registration DllPath={1} Machine={2}" -f $Arch, $DllPath, $MachineScope)
    $registrationOutput = @(& $powerShell @processArgs 2>&1)
    $exitCode = $LASTEXITCODE
    foreach ($line in $registrationOutput) {
        if ($null -ne $line) {
            Write-InstallLog ("TIP registration {0}: {1}" -f $Arch, ($line | Out-String).Trim())
        }
    }
    if ($exitCode -ne 0) {
        throw "$Arch TIP registration failed with exit code $exitCode"
    }
}

function Add-InputMethodToCurrentUserLanguageList {
    param([string]$Tip)

    $languageLogPath = Get-LanguageListLogPath

    function Write-ImeLanguageLog {
        param([string]$Message)

        try {
            $directory = Split-Path -Parent $languageLogPath
            Ensure-Directory -Path $directory
            $line = ('{0:yyyy-MM-dd HH:mm:ss} {1}' -f (Get-Date), $Message)
            Add-Content -LiteralPath $languageLogPath -Value $line -Encoding UTF8 -ErrorAction Stop
        } catch {
            # Best-effort logging only.
        }
    }

    $getCommand = Get-Command -Name Get-WinUserLanguageList -ErrorAction SilentlyContinue
    $setCommand = Get-Command -Name Set-WinUserLanguageList -ErrorAction SilentlyContinue
    if (-not $getCommand -or -not $setCommand) {
        Write-ImeLanguageLog 'SKIP: Get/Set-WinUserLanguageList cmdlet unavailable'
        Write-Warning 'Windows language list cmdlets are unavailable on this machine; skipping Time & Language registration.'
        Write-InstallLog 'SKIP: Get/Set-WinUserLanguageList cmdlet unavailable'
        return
    }

    $languageList = Get-WinUserLanguageList
    $targetLanguage = $languageList | Where-Object { $_.LanguageTag -in @('zh-Hans-CN', 'zh-CN') } | Select-Object -First 1
    if (-not $targetLanguage) {
        $targetLanguage = $languageList | Where-Object { $_.LanguageTag -like 'zh-Hans*' -or $_.LanguageTag -like 'zh-CN*' } | Select-Object -First 1
    }
    if (-not $targetLanguage) {
        $newLanguageList = New-WinUserLanguageList 'zh-Hans-CN'
        $targetLanguage = $newLanguageList[0]
        [void]$languageList.Add($targetLanguage)
        Write-ImeLanguageLog 'OK: created zh-Hans-CN language entry for current user'
        Write-InstallLog 'OK: created zh-Hans-CN language entry for current user'
    }

    $existingTips = New-Object 'System.Collections.Generic.List[string]'
    foreach ($existingTip in @($targetLanguage.InputMethodTips)) {
        if (-not [string]::IsNullOrWhiteSpace($existingTip) -and -not $existingTips.Contains($existingTip)) {
            [void]$existingTips.Add($existingTip)
        }
    }

    if ($existingTips.Contains($Tip)) {
        $message = 'OK: TIP already in Chinese input method list'
        Write-ImeLanguageLog $message
        Write-InstallLog $message
        Write-Output ("{0} is already present in the current user's language list." -f $AppDisplayName)
        return
    }

    [void]$existingTips.Add($Tip)
    if ($targetLanguage.InputMethodTips -and $targetLanguage.InputMethodTips.PSObject.Methods.Name -contains 'Clear') {
        $targetLanguage.InputMethodTips.Clear()
        foreach ($value in $existingTips) {
            [void]$targetLanguage.InputMethodTips.Add($value)
        }
    } else {
        $targetLanguage.InputMethodTips = [string[]]$existingTips
    }

    try {
        Set-WinUserLanguageList -LanguageList $languageList -Force
    } catch {
        $message = ("FAIL: Set-WinUserLanguageList: {0}" -f $_.Exception.Message)
        Write-ImeLanguageLog $message
        Write-InstallLog $message
        throw
    }

    $verified = $false
    foreach ($language in @(Get-WinUserLanguageList)) {
        if ($language.InputMethodTips -contains $Tip) {
            $verified = $true
            break
        }
    }
    if (-not $verified) {
        $message = "FAIL: Windows rejected TIP after Set-WinUserLanguageList; TSF profile is not registered: $Tip"
        Write-ImeLanguageLog $message
        Write-InstallLog $message
        throw $message
    }

    Write-ImeLanguageLog 'OK: added and verified TIP in Chinese input method list'
    Write-InstallLog 'OK: added and verified TIP in Chinese input method list'
    Write-Output ("Added {0} to the current user's Chinese (Simplified) language list." -f $AppDisplayName)
}

function Restart-TextInputServices {
    Stop-Process -Name 'TextInputHost' -Force -ErrorAction SilentlyContinue

    Start-Sleep -Milliseconds 500

    Start-TextInputUx
    Write-InstallLog 'OK: restarted text input stack'
    Write-Output 'Restarted text input stack (TextInputHost + ctfmon).'
}

function Start-TextInputUx {
    $ctfmonPath = Join-Path $env:WINDIR 'System32\ctfmon.exe'
    if (Test-Path -LiteralPath $ctfmonPath) {
        Start-Process -FilePath $ctfmonPath -WindowStyle Hidden -ErrorAction SilentlyContinue | Out-Null
        Write-InstallLog 'OK: started ctfmon.exe'
        Write-Output 'Started ctfmon.exe to refresh the text input UI.'
    } else {
        Write-InstallLog ("WARN: ctfmon.exe missing at {0}" -f $ctfmonPath)
        Write-Warning "ctfmon.exe was not found at $ctfmonPath"
    }
}

function Write-UninstallEntry {
    param(
        [string]$InstallRoot,
        [bool]$MachineScope
    )

    $keyRoot = if ($MachineScope) { 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall' } else { 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' }
    $keyPath = Join-Path $keyRoot $AppDisplayName
    $scriptPath = Join-Path $InstallRoot 'uninstall_dev.ps1'
    $args = @('-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f $scriptPath), '-InstallationRoot', ('"{0}"' -f $InstallRoot))
    if ($MachineScope) { $args += '-Machine' }
    $command = 'powershell.exe ' + ($args -join ' ')
    $sizeKb = [int](([math]::Max((Get-ChildItem -LiteralPath $InstallRoot -Recurse -File | Measure-Object -Property Length -Sum).Sum, 0)) / 1KB)
    $versionPath = Join-Path $InstallRoot 'VERSION'
    $displayVersion = if (Test-Path -LiteralPath $versionPath) {
        (Get-Content -LiteralPath $versionPath -Raw -Encoding UTF8).Trim()
    } else {
        '0.0.0'
    }

    New-Item -Path $keyPath -Force | Out-Null
    New-ItemProperty -Path $keyPath -Name DisplayName -Value $AppDisplayName -PropertyType String -Force | Out-Null
    New-ItemProperty -Path $keyPath -Name DisplayVersion -Value $displayVersion -PropertyType String -Force | Out-Null
    New-ItemProperty -Path $keyPath -Name Publisher -Value $AppDisplayName -PropertyType String -Force | Out-Null
    New-ItemProperty -Path $keyPath -Name InstallLocation -Value $InstallRoot -PropertyType String -Force | Out-Null
    New-ItemProperty -Path $keyPath -Name DisplayIcon -Value (Join-Path $InstallRoot 'srf_ime_settings.exe') -PropertyType String -Force | Out-Null
    New-ItemProperty -Path $keyPath -Name UninstallString -Value $command -PropertyType String -Force | Out-Null
    New-ItemProperty -Path $keyPath -Name QuietUninstallString -Value $command -PropertyType String -Force | Out-Null
    New-ItemProperty -Path $keyPath -Name NoModify -Value 1 -PropertyType DWord -Force | Out-Null
    New-ItemProperty -Path $keyPath -Name NoRepair -Value 1 -PropertyType DWord -Force | Out-Null
    New-ItemProperty -Path $keyPath -Name EstimatedSize -Value $sizeKb -PropertyType DWord -Force | Out-Null
    Write-InstallLog 'OK: wrote uninstall entry'
}

function Write-TrayRunEntry {
    param([string]$InstallRoot)

    $runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    New-Item -Path $runKey -Force | Out-Null
    $trayExe = Join-Path $InstallRoot 'srf_ime_tray.exe'
    $engineExe = Join-Path $InstallRoot 'srf_ime_engine.exe'
    if (Test-Path -LiteralPath $trayExe) {
        $command = ('"{0}"' -f $trayExe)
        New-ItemProperty -Path $runKey -Name $TrayRunEntryName -Value $command -PropertyType String -Force | Out-Null
    }
    if (Test-Path -LiteralPath $engineExe) {
        # The engine derives its per-install IPC names from its own path when
        # started without TSF-provided arguments.
        $engineCommand = ('"{0}" --startup-warmup-delay-ms 750' -f $engineExe)
        New-ItemProperty -Path $runKey -Name $EngineRunEntryName -Value $engineCommand -PropertyType String -Force | Out-Null
    }
    Remove-ItemProperty -Path $runKey -Name $LegacyTrayRunEntryName -ErrorAction SilentlyContinue
    $startupApprovedRunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
    if (Test-Path -Path $startupApprovedRunKey) {
        Remove-ItemProperty -Path $startupApprovedRunKey -Name $LegacyTrayRunEntryName -ErrorAction SilentlyContinue
    }
    Write-InstallLog 'OK: wrote tray and engine autorun entries'
}

function Start-TrayHelper {
    param([string]$InstallRoot)

    $trayExe = Join-Path $InstallRoot 'srf_ime_tray.exe'
    if (Test-Path -LiteralPath $trayExe) {
        Start-Process -FilePath $trayExe -WindowStyle Hidden -ErrorAction SilentlyContinue | Out-Null
    }
    Write-InstallLog 'OK: started tray helper'
}

function Get-LoadedRuntimePayloadRoots {
    param([string]$InstallRoot)

    $loaded = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::OrdinalIgnoreCase)
    $runtimeRoot = Join-Path $InstallRoot $RuntimePayloadRootName
    if (-not (Test-Path -LiteralPath $runtimeRoot)) {
        return $loaded
    }

    $runtimeFull = [System.IO.Path]::GetFullPath($runtimeRoot).TrimEnd('\')
    $runtimePrefix = $runtimeFull + '\'
    foreach ($process in @(Get-Process -ErrorAction SilentlyContinue)) {
        try {
            foreach ($module in @($process.Modules)) {
                $fileName = $module.FileName
                if ([string]::IsNullOrWhiteSpace($fileName)) { continue }
                if (-not [System.IO.Path]::GetFileName($fileName).Equals('srf_tsf_tip.dll', [System.StringComparison]::OrdinalIgnoreCase)) { continue }
                $full = [System.IO.Path]::GetFullPath($fileName)
                if (-not $full.StartsWith($runtimePrefix, [System.StringComparison]::OrdinalIgnoreCase)) { continue }
                $archDir = Split-Path -Parent $full
                $payload = if ([System.IO.Path]::GetFileName($archDir) -in @('x64', 'x86')) {
                    Split-Path -Parent $archDir
                } else {
                    $archDir
                }
                $payloadFull = [System.IO.Path]::GetFullPath($payload).TrimEnd('\')
                if ($payloadFull.StartsWith($runtimePrefix, [System.StringComparison]::OrdinalIgnoreCase) -and
                    [System.IO.Path]::GetFileName($payloadFull).StartsWith('pkg-', [System.StringComparison]::OrdinalIgnoreCase)) {
                    [void]$loaded.Add($payloadFull)
                }
            }
        } catch {
            # Access-denied/system processes are expected.
        }
    }
    return $loaded
}

function Restore-LoadedRuntimePayloadRoots {
    param(
        [string]$InstallRoot,
        $LoadedPayloadRoots
    )

    $runtimeRoot = Join-Path $InstallRoot $RuntimePayloadRootName
    if (-not (Test-Path -LiteralPath $runtimeRoot)) {
        return
    }

    foreach ($payloadRoot in @($LoadedPayloadRoots)) {
        if ([string]::IsNullOrWhiteSpace($payloadRoot)) { continue }
        if (Test-Path -LiteralPath $payloadRoot) { continue }
        $payloadName = [System.IO.Path]::GetFileName($payloadRoot)
        if ([string]::IsNullOrWhiteSpace($payloadName)) { continue }
        $prefix = '.delete-pending-' + $payloadName + '-'
        $pending = Get-ChildItem -LiteralPath $runtimeRoot -Force -Directory -ErrorAction SilentlyContinue |
            Where-Object { $_.Name.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase) } |
            Sort-Object LastWriteTimeUtc -Descending |
            Select-Object -First 1
        if (-not $pending) { continue }
        try {
            Move-Item -LiteralPath $pending.FullName -Destination $payloadRoot -Force -ErrorAction Stop
            Write-InstallLog ("OK: restored loaded runtime payload {0} from deferred-delete directory" -f $payloadName)
        } catch {
            try {
                Copy-Item -LiteralPath $pending.FullName -Destination $payloadRoot -Recurse -Force -ErrorAction Stop
                Write-InstallLog ("OK: copied back loaded runtime payload {0} from deferred-delete directory" -f $payloadName)
            } catch {
                Write-InstallLog ("WARN: could not restore loaded runtime payload {0}: {1}" -f $payloadName, $_.Exception.Message)
            }
        }
    }
}

function Remove-OldRuntimePayloads {
    param(
        [string]$InstallRoot,
        [string]$CurrentPayloadRoot,
        [int]$KeepCount = 1
    )

    $runtimeRoot = Join-Path $InstallRoot $RuntimePayloadRootName
    if (-not (Test-Path -LiteralPath $runtimeRoot)) {
        return
    }
    $resolvedRuntimeRoot = [System.IO.Path]::GetFullPath($runtimeRoot).TrimEnd('\')
    $current = [System.IO.Path]::GetFullPath($CurrentPayloadRoot).TrimEnd('\')
    $entries = @(Get-ChildItem -LiteralPath $runtimeRoot -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like 'pkg-*' } |
        Sort-Object LastWriteTimeUtc -Descending)

    $keep = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::OrdinalIgnoreCase)
    [void]$keep.Add($current)
    $loadedPayloads = @(Get-LoadedRuntimePayloadRoots -InstallRoot $InstallRoot)
    Restore-LoadedRuntimePayloadRoots -InstallRoot $InstallRoot -LoadedPayloadRoots $loadedPayloads
    foreach ($loadedPayload in $loadedPayloads) {
        [void]$keep.Add($loadedPayload)
        if (-not $loadedPayload.Equals($current, [System.StringComparison]::OrdinalIgnoreCase)) {
            Write-InstallLog ("OK: preserving runtime payload still loaded by a host process: {0}" -f $loadedPayload)
        }
    }
    foreach ($entry in $entries) {
        if ($keep.Count -ge $KeepCount) { break }
        $path = [System.IO.Path]::GetFullPath($entry.FullName).TrimEnd('\')
        [void]$keep.Add($path)
    }

    foreach ($entry in $entries) {
        $path = [System.IO.Path]::GetFullPath($entry.FullName).TrimEnd('\')
        if ($keep.Contains($path)) { continue }
        if (-not $path.StartsWith($resolvedRuntimeRoot + '\', [System.StringComparison]::OrdinalIgnoreCase)) {
            Write-InstallLog ("WARN: skip unexpected runtime payload path {0}" -f $path)
            continue
        }
        try {
            Remove-Item -LiteralPath $entry.FullName -Recurse -Force -ErrorAction Stop
            Write-InstallLog ("OK: removed old runtime payload {0}" -f $entry.Name)
        } catch {
            $pending = Join-Path $runtimeRoot ('.delete-pending-' + $entry.Name + '-' + [guid]::NewGuid().ToString('N'))
            try {
                Move-Item -LiteralPath $entry.FullName -Destination $pending -Force -ErrorAction Stop
                $escapedPending = $pending.Replace("'", "''")
                $cleanupCommand = "Start-Sleep -Seconds 5; Remove-Item -LiteralPath '$escapedPending' -Recurse -Force -ErrorAction SilentlyContinue"
                Start-Process -FilePath powershell.exe -ArgumentList @(
                    '-NoProfile', '-ExecutionPolicy', 'Bypass', '-WindowStyle', 'Hidden',
                    '-Command', $cleanupCommand
                ) -WindowStyle Hidden -ErrorAction SilentlyContinue | Out-Null
                Write-InstallLog ("WARN: marked locked old runtime payload for deferred deletion {0}" -f $pending)
            } catch {
                Write-InstallLog ("WARN: could not remove old runtime payload {0}: {1}" -f $entry.FullName, $_.Exception.Message)
            }
        }
    }
    Write-InstallLog ("OK: runtime payload cleanup complete; keep_count={0}" -f $KeepCount)
}

function Write-RuntimePayloadVerification {
    param(
        [string]$InstallRoot,
        [string]$PayloadRoot,
        [string]$DllX64,
        [string]$DllX86
    )

    Ensure-File $DllX64
    Ensure-File $DllX86
    $rootFull = [System.IO.Path]::GetFullPath($InstallRoot)
    $payloadFull = [System.IO.Path]::GetFullPath($PayloadRoot)
    $relativePayload = $payloadFull
    $rootPrefix = $rootFull.TrimEnd('\') + '\'
    if ($payloadFull.StartsWith($rootPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        $relativePayload = $payloadFull.Substring($rootPrefix.Length)
    }
    $hash64 = (Get-FileHash -LiteralPath $DllX64 -Algorithm SHA256).Hash.ToLowerInvariant()
    $hash86 = (Get-FileHash -LiteralPath $DllX86 -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-InstallLog ("OK: verified runtime payload={0} x64={1} sha256={2} x86={3} sha256={4}" -f
        $relativePayload, $DllX64, $hash64, $DllX86, $hash86)
}

function Get-StaleRuntimeHosts {
    param(
        [string]$InstallRoot,
        [string]$CurrentPayloadRoot
    )

    $currentFull = [System.IO.Path]::GetFullPath($CurrentPayloadRoot).TrimEnd('\')
    $currentPrefix = $currentFull + '\'
    $hostsById = @{}

    foreach ($process in @(Get-Process -ErrorAction SilentlyContinue)) {
        try {
            foreach ($module in @($process.Modules)) {
                $fileName = $module.FileName
                if ([string]::IsNullOrWhiteSpace($fileName)) { continue }
                if (-not [System.IO.Path]::GetFileName($fileName).Equals('srf_tsf_tip.dll', [System.StringComparison]::OrdinalIgnoreCase)) { continue }
                $full = [System.IO.Path]::GetFullPath($fileName)
                if ($full.StartsWith($currentPrefix, [System.StringComparison]::OrdinalIgnoreCase)) { continue }
                if (-not $hostsById.ContainsKey($process.Id)) {
                    $commandLine = $null
                    $executablePath = $null
                    try {
                        $cim = Get-CimInstance -ClassName Win32_Process -Filter ("ProcessId = {0}" -f $process.Id) -ErrorAction SilentlyContinue
                        if ($cim) {
                            $commandLine = [string]$cim.CommandLine
                            $executablePath = [string]$cim.ExecutablePath
                        }
                    } catch {
                    }
                    if ([string]::IsNullOrWhiteSpace($executablePath)) {
                        try {
                            $executablePath = [string]$process.Path
                        } catch {
                        }
                    }
                    $hostsById[$process.Id] = [pscustomobject]@{
                        Id = [int]$process.Id
                        ProcessName = [string]$process.ProcessName
                        LoadedDll = [string]$full
                        CommandLine = [string]$commandLine
                        ExecutablePath = [string]$executablePath
                    }
                }
            }
        } catch {
            # Access-denied/system processes are expected; this is best-effort maintenance.
        }
    }

    return @($hostsById.Values | Sort-Object ProcessName, Id)
}

function Format-StaleRuntimeHostLine {
    param($HostInfo)
    return ('{0}({1}) -> {2}' -f $HostInfo.ProcessName, $HostInfo.Id, $HostInfo.LoadedDll)
}

function Write-StaleRuntimeHostDiagnostics {
    param(
        [string]$InstallRoot,
        [string]$CurrentPayloadRoot
    )

    $hosts = @(Get-StaleRuntimeHosts -InstallRoot $InstallRoot -CurrentPayloadRoot $CurrentPayloadRoot)
    if ($hosts.Count -eq 0) {
        Write-InstallLog 'OK: no running host process is using an old runtime payload'
        return
    }

    Write-InstallLog ("WARN: {0} running host process(es) still use old TSF runtime; restart those apps to load the newest DLL" -f $hosts.Count)
    foreach ($hostInfo in $hosts) {
        Write-InstallLog ("WARN: stale runtime host: {0}" -f (Format-StaleRuntimeHostLine $hostInfo))
    }
}

function Format-StaleRuntimeHostSummary {
    param(
        $HostInfos,
        [int]$MaxItems = 12
    )

    $names = New-Object 'System.Collections.Generic.SortedSet[string]' ([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($hostInfo in @($HostInfos)) {
        $display = [string]$hostInfo.ProcessName
        if ([string]::IsNullOrWhiteSpace($display)) { continue }
        [void]$names.Add($display)
    }

    $items = @($names | Select-Object -First $MaxItems)
    $lines = New-Object 'System.Collections.Generic.List[string]'
    foreach ($item in $items) {
        [void]$lines.Add((' - {0}' -f $item))
    }
    if ($names.Count -gt $items.Count) {
        [void]$lines.Add((' - ... plus {0} more' -f ($names.Count - $items.Count)))
    }
    return ($lines -join "`r`n")
}

function Confirm-StaleRuntimeHostRestart {
    param($HostInfos)

    $summary = Format-StaleRuntimeHostSummary -HostInfos $HostInfos
    if ([string]::IsNullOrWhiteSpace($summary)) {
        return $false
    }

    $message = "以下应用仍在使用旧版开心输入法运行时，重启后才能加载新版候选栏：`r`n`r`n$summary`r`n`r`n继续前请先保存正在编辑的内容。是否现在请求这些应用正常关闭并重启？"
    try {
        $shell = New-Object -ComObject WScript.Shell
        $yesNoExclamationDefaultNo = 4 + 48 + 256
        $result = $shell.Popup($message, 0, '开心输入法安装', $yesNoExclamationDefaultNo)
        return ($result -eq 6)
    } catch {
        Write-InstallLog ("WARN: could not show stale runtime host restart prompt: {0}" -f $_.Exception.Message)
        return $false
    }
}

function Restart-StaleRuntimeHosts {
    param(
        [string]$InstallRoot,
        [string]$CurrentPayloadRoot,
        [switch]$Prompt
    )

    $hosts = @(Get-StaleRuntimeHosts -InstallRoot $InstallRoot -CurrentPayloadRoot $CurrentPayloadRoot)
    if ($hosts.Count -eq 0) {
        Write-InstallLog 'OK: no stale runtime host process needs restart'
        return
    }

    if ($Prompt -and -not (Confirm-StaleRuntimeHostRestart -HostInfos $hosts)) {
        Write-InstallLog 'SKIP: user declined stale runtime host restart'
        return
    }

    $skipNames = @(
        'ApplicationFrameHost',
        'explorer',
        'SearchHost',
        'ShellExperienceHost',
        'StartMenuExperienceHost',
        'SystemSettings'
    )
    $restarted = 0
    $skipped = 0
    Write-InstallLog ("BEGIN: request restart for {0} stale runtime host process(es)" -f $hosts.Count)
    foreach ($hostInfo in $hosts) {
        $line = Format-StaleRuntimeHostLine $hostInfo
        if ($skipNames -contains $hostInfo.ProcessName) {
            $skipped++
            Write-InstallLog ("SKIP: stale runtime host is a shell/system process: {0}" -f $line)
            continue
        }

        $restartCommand = [string]$hostInfo.CommandLine
        if ([string]::IsNullOrWhiteSpace($restartCommand)) {
            if ([string]::IsNullOrWhiteSpace([string]$hostInfo.ExecutablePath)) {
                $skipped++
                Write-InstallLog ("SKIP: stale runtime host has no restart command: {0}" -f $line)
                continue
            }
            $restartCommand = ('"{0}"' -f $hostInfo.ExecutablePath)
        }

        $process = Get-Process -Id $hostInfo.Id -ErrorAction SilentlyContinue
        if (-not $process) {
            Write-InstallLog ("OK: stale runtime host already exited before restart: {0}" -f $line)
            continue
        }

        $requestedClose = $false
        try {
            if ($process.MainWindowHandle -ne 0) {
                $requestedClose = $process.CloseMainWindow()
            }
        } catch {
        }
        if (-not $requestedClose) {
            $skipped++
            Write-InstallLog ("SKIP: stale runtime host has no closable main window: {0}" -f $line)
            continue
        }

        if (-not $process.WaitForExit(10000)) {
            $skipped++
            Write-InstallLog ("WARN: stale runtime host did not exit after close request; leaving it open: {0}" -f $line)
            continue
        }

        try {
            $creator = [wmiclass]'Win32_Process'
            $result = $creator.Create($restartCommand)
            if ($result.ReturnValue -eq 0) {
                $restarted++
                Write-InstallLog ("OK: restarted stale runtime host {0} as process {1}" -f $line, $result.ProcessId)
            } else {
                $skipped++
                Write-InstallLog ("WARN: failed to restart stale runtime host {0}; Win32_Process.Create returned {1}" -f $line, $result.ReturnValue)
            }
        } catch {
            $skipped++
            Write-InstallLog ("WARN: failed to restart stale runtime host {0}: {1}" -f $line, $_.Exception.Message)
        }
    }
    Write-InstallLog ("OK: stale runtime host restart request complete; restarted={0} skipped={1}" -f $restarted, $skipped)
}

function Format-InstallHealthValue {
    param([object]$Value)

    if ($null -eq $Value) { return '(none)' }
    $text = [string]$Value
    if ([string]::IsNullOrWhiteSpace($text)) { return '(none)' }
    $text = [regex]::Replace($text, '[\s,;=]+', '_')
    if ($text.Length -gt 240) {
        $text = $text.Substring(0, 240)
    }
    return $text
}

function Get-InstallHealthVersion {
    param([string]$InstallRoot)

    $versionPath = Join-Path $InstallRoot 'VERSION'
    if (-not (Test-Path -LiteralPath $versionPath)) { return '(missing)' }
    try {
        $version = (Get-Content -LiteralPath $versionPath -Raw -Encoding ASCII).Trim()
        if ([string]::IsNullOrWhiteSpace($version)) { return '(empty)' }
        return $version
    } catch {
        return 'error'
    }
}

function Test-InstallHealthComRegistration {
    param(
        [string]$DllX64,
        [string]$DllX86,
        [bool]$MachineScope
    )

    $classRoot = if ($MachineScope) { 'HKLM:\Software\Classes' } else { 'HKCU:\Software\Classes' }
    $entries = @(
        @{ Arch = 'x64'; Path = "$classRoot\CLSID\$TextServiceClsid\InprocServer32"; Expected = $DllX64 },
        @{ Arch = 'x86'; Path = "$classRoot\WOW6432Node\CLSID\$TextServiceClsid\InprocServer32"; Expected = $DllX86 }
    )
    $failures = New-Object 'System.Collections.Generic.List[string]'
    foreach ($entry in $entries) {
        if (-not (Test-Path -LiteralPath $entry.Path)) {
            [void]$failures.Add(($entry.Arch + '_missing'))
            continue
        }
        $actual = (Get-Item -LiteralPath $entry.Path).GetValue('')
        try {
            $actualFull = [System.IO.Path]::GetFullPath([string]$actual)
            $expectedFull = [System.IO.Path]::GetFullPath($entry.Expected)
        } catch {
            [void]$failures.Add(($entry.Arch + '_path_error'))
            continue
        }
        if (-not $actualFull.Equals($expectedFull, [System.StringComparison]::OrdinalIgnoreCase)) {
            [void]$failures.Add(($entry.Arch + '_mismatch'))
        }
    }
    if ($failures.Count -eq 0) { return 'ok' }
    return ($failures -join '+')
}

function Test-InstallHealthProfileRegistry {
    param([bool]$MachineScope)

    $roots = if ($MachineScope) {
        @('HKLM:\Software\Microsoft\CTF\TIP', 'HKCU:\Software\Microsoft\CTF\TIP')
    } else {
        @('HKCU:\Software\Microsoft\CTF\TIP', 'HKLM:\Software\Microsoft\CTF\TIP')
    }
    foreach ($root in $roots) {
        $profilePath = "$root\$TextServiceClsid\LanguageProfile\0x00000804\$ProfileGuid"
        if (Test-Path -LiteralPath $profilePath) { return 'ok' }
    }
    return 'missing'
}

function Test-InstallHealthLanguageList {
    param(
        [string]$Tip,
        [bool]$Expected
    )

    if (-not $Expected) { return 'skipped' }
    $getCommand = Get-Command -Name Get-WinUserLanguageList -ErrorAction SilentlyContinue
    if (-not $getCommand) { return 'unavailable' }
    try {
        foreach ($language in @(Get-WinUserLanguageList)) {
            if ($language.InputMethodTips -contains $Tip) {
                return 'ok'
            }
        }
        return 'missing'
    } catch {
        return 'error'
    }
}

function Invoke-InstallHealthEngineProbe {
    param([string]$InstallRoot)

    $engineExe = Join-Path $InstallRoot 'srf_ime_engine.exe'
    if (-not (Test-Path -LiteralPath $engineExe)) {
        return @{ Status = 'missing'; ExitCode = '(none)'; Direct = 'unknown'; Pipe = 'unknown'; Reason = 'engine_missing' }
    }

    $stdoutPath = Join-Path ([System.IO.Path]::GetTempPath()) ('kaixin-engine-probe-' + [guid]::NewGuid().ToString('N') + '.out')
    $stderrPath = Join-Path ([System.IO.Path]::GetTempPath()) ('kaixin-engine-probe-' + [guid]::NewGuid().ToString('N') + '.err')
    try {
        $process = Start-Process -FilePath $engineExe -ArgumentList @('--install-health-check', '--probe', 'nihao') -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath -ErrorAction Stop
        if (-not $process.WaitForExit(15000)) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
            return @{ Status = 'timeout'; ExitCode = '(none)'; Direct = 'unknown'; Pipe = 'unknown'; Reason = 'timeout' }
        }
        $probeText = ''
        if (Test-Path -LiteralPath $stdoutPath) {
            $probeText += (Get-Content -LiteralPath $stdoutPath -Raw -ErrorAction SilentlyContinue)
        }
        if (Test-Path -LiteralPath $stderrPath) {
            $probeText += "`n" + (Get-Content -LiteralPath $stderrPath -Raw -ErrorAction SilentlyContinue)
        }
        $process.Refresh()
        $exitCode = $process.ExitCode
        $exitCodeValue = if ($null -eq $exitCode) { '(none)' } else { $exitCode }
        $exitCodeOk = ($null -ne $exitCode -and $exitCode -eq 0)
        $direct = if ($exitCodeOk) { 'ok' } else { 'unknown' }
        $pipe = if ($exitCodeOk) { 'ok' } else { 'unknown' }
        $sawProbeStatus = $false
        $reason = 'none'
        if ($probeText -match 'direct=([^\s]+)') { $direct = $matches[1]; $sawProbeStatus = $true }
        if ($probeText -match 'pipe=([^\s]+)') { $pipe = $matches[1]; $sawProbeStatus = $true }
        if ($probeText -match 'direct_reason=([^\s]+)') { $reason = 'direct_' + $matches[1] }
        if ($probeText -match 'pipe_reason=([^\s]+)' -and $matches[1] -ne 'none') { $reason = 'pipe_' + $matches[1] }
        $probeOk = if ($sawProbeStatus) { ($direct -eq 'ok' -and $pipe -eq 'ok') } else { $exitCodeOk }
        if ($probeOk) {
            return @{ Status = 'ok'; ExitCode = $exitCodeValue; Direct = $direct; Pipe = $pipe; Reason = $reason }
        }
        return @{ Status = 'failed'; ExitCode = $exitCodeValue; Direct = $direct; Pipe = $pipe; Reason = $reason }
    } catch {
        return @{ Status = 'error'; ExitCode = '(none)'; Direct = 'unknown'; Pipe = 'unknown'; Reason = $_.Exception.Message }
    } finally {
        Remove-Item -LiteralPath $stdoutPath, $stderrPath -Force -ErrorAction SilentlyContinue
    }
}

function Invoke-InstallHealthCheck {
    param(
        [string]$InstallRoot,
        [string]$PayloadRoot,
        [string]$DllX64,
        [string]$DllX86,
        [bool]$MachineScope,
        [bool]$LanguageListExpected
    )

    try {
        $version = Get-InstallHealthVersion -InstallRoot $InstallRoot
        $comStatus = Test-InstallHealthComRegistration -DllX64 $DllX64 -DllX86 $DllX86 -MachineScope:$MachineScope
        $profileStatus = Test-InstallHealthProfileRegistry -MachineScope:$MachineScope
        $languageStatus = Test-InstallHealthLanguageList -Tip $SimplifiedChineseTip -Expected $LanguageListExpected
        $probe = Invoke-InstallHealthEngineProbe -InstallRoot $InstallRoot

        $reasons = New-Object 'System.Collections.Generic.List[string]'
        if ($comStatus -ne 'ok') { [void]$reasons.Add(('com_' + $comStatus)) }
        if ($profileStatus -ne 'ok') { [void]$reasons.Add(('profile_' + $profileStatus)) }
        if ($languageStatus -notin @('ok', 'skipped')) { [void]$reasons.Add(('language_' + $languageStatus)) }
        if ($probe.Status -ne 'ok') { [void]$reasons.Add(('engine_probe_' + $probe.Status)) }

        $status = if ($reasons.Count -eq 0) { 'ok' } else { 'failed' }
        $reason = if ($reasons.Count -eq 0) { 'none' } else { ($reasons -join '+') }
        Write-InstallLog ("event=install_health_check status={0} version={1} machine={2} install_root={3} payload_root={4} com={5} profile={6} language_list={7} engine_probe={8} engine_direct_probe={9} engine_pipe_probe={10} probe_exit={11} probe_reason={12} reason={13}" -f
            (Format-InstallHealthValue $status),
            (Format-InstallHealthValue $version),
            (Format-InstallHealthValue ([int]$MachineScope)),
            (Format-InstallHealthValue $InstallRoot),
            (Format-InstallHealthValue $PayloadRoot),
            (Format-InstallHealthValue $comStatus),
            (Format-InstallHealthValue $profileStatus),
            (Format-InstallHealthValue $languageStatus),
            (Format-InstallHealthValue $probe.Status),
            (Format-InstallHealthValue $probe.Direct),
            (Format-InstallHealthValue $probe.Pipe),
            (Format-InstallHealthValue $probe.ExitCode),
            (Format-InstallHealthValue $probe.Reason),
            (Format-InstallHealthValue $reason))
    } catch {
        Write-InstallLog ("event=install_health_check status=failed reason={0}" -f (Format-InstallHealthValue $_.Exception.Message))
    }
}

$PackageRoot = [System.IO.Path]::GetFullPath($PackageRoot)
if (-not $InstallationRoot) {
    $InstallationRoot = Get-DefaultInstallRoot -MachineScope:$Machine.IsPresent
}
$InstallationRoot = [System.IO.Path]::GetFullPath($InstallationRoot)
Assert-TrustedInstallRoot -Path $InstallationRoot
$script:InstallLogPath = Get-DefaultInstallLogPath -InstallRoot $InstallationRoot -MachineScope:$Machine.IsPresent
$lexiconPrefix = (-join ([char[]](0x8BCD, 0x5E93, 0x002D)))

Write-InstallLog ("BEGIN: install_dev.ps1 Machine={0} SkipFileCopy={1} SkipRegistration={2} SkipLanguageList={3} SkipTextServiceRestart={4} SkipTextInputUx={5} SkipUninstallEntry={6} SkipTraySetup={7} SkipStaleHostRestart={8} PromptStaleHostRestart={9}" -f
    $Machine.IsPresent, $SkipFileCopy.IsPresent, $SkipRegistration.IsPresent, $SkipLanguageList.IsPresent,
    $SkipTextServiceRestart.IsPresent, $SkipTextInputUx.IsPresent, $SkipUninstallEntry.IsPresent,
    $SkipTraySetup.IsPresent, $SkipStaleHostRestart.IsPresent, $PromptStaleHostRestart.IsPresent)

trap {
    Write-InstallLog ("FAIL: {0}" -f $_.Exception.Message)
    throw
}

$requiredFiles = @(
    $RuntimePayloadManifestName,
    $PackageManifestName,
    'build-manifest.json',
    'component_manifest.ini',
    'VERSION',
    'install_dev.ps1',
    'srf_ime_settings.exe',
    'srf_ime_settings.exe.manifest',
    'srf_ime_tray.exe',
    'srf_ime_tray.exe.manifest',
    'srf_ime_engine.exe',
    'srf_ime_clipboard.exe',
    'srf_ime_clipboard_svc.exe',
    'srf_ime_handwrite.exe',
    'invoke_registration.ps1',
    'install_current_user.ps1',
    'repair_install.ps1',
    'restart_stale_hosts.ps1',
    'export_diagnostics.ps1',
    'uninstall_dev.ps1',
    'uninstall_current_user.ps1',
    'user_data_manifest.json'
)

$ocrExe = Join-Path $PackageRoot 'srf_ime_ocr.exe'
if (Test-Path -LiteralPath $ocrExe) {
    $requiredFiles += 'srf_ime_ocr.exe'
    Write-InstallLog "OK: OCR extension package detected"
} else {
    Write-InstallLog "OK: OCR extension not packaged; skipping OCR executable checks"
}

foreach ($file in $requiredFiles) {
    Ensure-File (Join-Path $PackageRoot $file)
}
if ($SkipPackageIntegrityCheck) {
    Write-InstallLog 'SKIP: package integrity verification suppressed by trusted installer handoff'
} else {
    Assert-PackageHashManifest -Root $PackageRoot
}

$packagePayloadRoot = Resolve-RuntimePayloadRoot -Root $PackageRoot
$packagePayloadX64 = Resolve-RuntimePayloadArchRoot -Root $PackageRoot -Arch x64
$packagePayloadX86 = Resolve-RuntimePayloadArchRoot -Root $PackageRoot -Arch x86
$packageTipDll = Join-Path $packagePayloadX64 'srf_tsf_tip.dll'
$packageTipDllX86 = Join-Path $packagePayloadX86 'srf_tsf_tip.dll'
Ensure-File $packageTipDll
Ensure-File $packageTipDllX86

$lexiconDirs = Get-PackagedLexiconDirectories -Root $PackageRoot
if (-not $lexiconDirs) {
    throw "No packaged lexicon directory was found in $PackageRoot"
}
$fontDir = Join-Path $PackageRoot 'font1'
$skinDir = Join-Path $PackageRoot 'skins'
$assetsDir = Join-Path $PackageRoot 'assets'
$toolsDir = Join-Path $PackageRoot 'tools'
$rapidOcrDir = Join-Path $PackageRoot 'RapidOCR-3.9.0'
$rapidOcrPackagesDir = Join-Path $PackageRoot '.python-packages'
$pythonRuntimeDir = Join-Path $PackageRoot '.python-runtime'

if (-not $SkipFileCopy) {
    Stop-TextInputStackForInstall
    Stop-RunningHelpers
    Clear-TransientUserState
    Clear-InstallRootForPureCopy -InstallRoot $InstallationRoot -SourceRoot $PackageRoot
    New-Item -ItemType Directory -Path $InstallationRoot -Force | Out-Null

    foreach ($file in $requiredFiles) {
        $source = Join-Path $PackageRoot $file
        if (Test-Path -LiteralPath $source) {
            Copy-Item -LiteralPath $source -Destination (Join-Path $InstallationRoot $file) -Force
        }
    }

    $installedPayloadRoot = Resolve-RuntimePayloadRoot -Root $InstallationRoot -AllowMissing
    if ([string]::IsNullOrWhiteSpace($installedPayloadRoot)) {
        throw "Installed runtime payload manifest did not resolve under $InstallationRoot"
    }
    $installedPayloadParent = Split-Path -Parent $installedPayloadRoot
    New-Item -ItemType Directory -Path $installedPayloadParent -Force | Out-Null
    if (-not (Test-Path -LiteralPath $installedPayloadRoot)) {
        Copy-Item -LiteralPath $packagePayloadRoot -Destination $installedPayloadParent -Recurse -Force
    }

        foreach ($dir in $lexiconDirs) {
            $target = Join-Path $InstallationRoot $dir.Name
            if (Test-Path -LiteralPath $target) {
                Remove-Item -LiteralPath $target -Recurse -Force
            }
            Copy-Item -LiteralPath $dir.FullName -Destination $target -Recurse -Force
        }

        if (Test-Path -LiteralPath $fontDir) {
            $target = Join-Path $InstallationRoot 'font1'
            if (Test-Path -LiteralPath $target) {
                Remove-Item -LiteralPath $target -Recurse -Force
            }
            Copy-Item -LiteralPath $fontDir -Destination $target -Recurse -Force
        }

        if (Test-Path -LiteralPath $skinDir) {
            $target = Join-Path $InstallationRoot 'skins'
            if (Test-Path -LiteralPath $target) {
                Remove-Item -LiteralPath $target -Recurse -Force
            }
            Copy-Item -LiteralPath $skinDir -Destination $target -Recurse -Force
        }

        foreach ($optionalDir in @(
            @{ Source = $assetsDir; Name = 'assets' },
            @{ Source = $toolsDir; Name = 'tools' },
            @{ Source = $pythonRuntimeDir; Name = '.python-runtime' },
            @{ Source = $rapidOcrPackagesDir; Name = '.python-packages' },
            @{ Source = $rapidOcrDir; Name = 'RapidOCR-3.9.0' }
        )) {
            if (Test-Path -LiteralPath $optionalDir.Source) {
                $target = Join-Path $InstallationRoot $optionalDir.Name
                if (Test-Path -LiteralPath $target) {
                    Remove-Item -LiteralPath $target -Recurse -Force
                }
                Copy-Item -LiteralPath $optionalDir.Source -Destination $target -Recurse -Force
            }
        }

    Write-InstallLog 'OK: copied package files'
}

if ($SkipAclHardening) {
    Write-InstallLog 'SKIP: ACL hardening already completed by the machine install step'
} else {
    Protect-DirectoryAclBestEffort -Path $InstallationRoot -ReadExecuteForBuiltinUsers
    Ensure-Directory -Path (Get-UserStateRoot)
    Protect-DirectoryAclBestEffort -Path (Get-UserStateRoot)
}
Invoke-ConfigMigration

$installedPayloadRoot = Resolve-RuntimePayloadRoot -Root $InstallationRoot
$installedPayloadX64 = Resolve-RuntimePayloadArchRoot -Root $InstallationRoot -Arch x64
$installedPayloadX86 = Resolve-RuntimePayloadArchRoot -Root $InstallationRoot -Arch x86
$installedDll = Join-Path $installedPayloadX64 'srf_tsf_tip.dll'
$installedDllX86 = Join-Path $installedPayloadX86 'srf_tsf_tip.dll'
$registrationScript = Join-Path $InstallationRoot 'invoke_registration.ps1'
Ensure-File $installedDll
Ensure-File $installedDllX86
Ensure-File $registrationScript
$previousSuccessfulPayloadRoot = Resolve-SuccessfulRuntimePayloadRoot -InstallRoot $InstallationRoot
Write-RuntimePayloadVerification -InstallRoot $InstallationRoot -PayloadRoot $installedPayloadRoot -DllX64 $installedDll -DllX86 $installedDllX86

$signature = Get-AuthenticodeSignature -FilePath $installedDll
if ($signature.Status -ne 'Valid') {
    $warningMessage = ("srf_tsf_tip.dll signature status is '{0}'. If registration fails on your Windows build, sign the DLL with a trusted code-signing certificate before distribution." -f $signature.Status)
    Write-InstallLog ("WARN: {0}" -f $warningMessage)
    Write-Warning $warningMessage
}

if (-not $SkipRegistration) {
    $registrationLogPath = Join-Path $InstallationRoot 'tip_registration.log'
    $previousRegLog = [Environment]::GetEnvironmentVariable('SRF_TSF_REG_LOG', 'Process')
    [Environment]::SetEnvironmentVariable('SRF_TSF_REG_LOG', $registrationLogPath, 'Process')
    try {
        Invoke-TipRegistration -RegistrationScript $registrationScript -DllPath $installedDll -Arch x64 -MachineScope:$Machine.IsPresent
        Invoke-TipRegistration -RegistrationScript $registrationScript -DllPath $installedDllX86 -Arch x86 -MachineScope:$Machine.IsPresent
        if ($Machine.IsPresent) {
            Remove-StalePerUserComRegistration -ExpectedDllX64 $installedDll -ExpectedDllX86 $installedDllX86
        }
    } catch {
        $message = ("TIP registration failed. Detail log: {0}. {1}" -f $registrationLogPath, $_.Exception.Message)
        Write-InstallLog ("FAIL: {0}" -f $message)
        [void](Try-RollbackRuntimePayload -InstallRoot $InstallationRoot -PreviousPayloadRoot $previousSuccessfulPayloadRoot -RegistrationScript $registrationScript -MachineScope:$Machine.IsPresent)
        throw $message
    } finally {
        [Environment]::SetEnvironmentVariable('SRF_TSF_REG_LOG', $previousRegLog, 'Process')
    }
    Write-InstallLog ("OK: completed TIP registration; detail log: {0}" -f $registrationLogPath)
}

Write-RuntimePayloadMarker -InstallRoot $InstallationRoot -PayloadRoot $installedPayloadRoot
if ($SkipRuntimeCleanup) {
    Write-InstallLog 'SKIP: runtime payload cleanup already completed by the machine install step'
} else {
    Remove-OldRuntimePayloads -InstallRoot $InstallationRoot -CurrentPayloadRoot $installedPayloadRoot
}
if ($SkipStaleHostDiagnostics) {
    Write-InstallLog 'SKIP: stale runtime host diagnostics suppressed for fast install'
} else {
    Write-StaleRuntimeHostDiagnostics -InstallRoot $InstallationRoot -CurrentPayloadRoot $installedPayloadRoot
}

if (-not $SkipLanguageList) {
    try {
        Add-InputMethodToCurrentUserLanguageList -Tip $SimplifiedChineseTip
    } catch {
        Write-InstallLog ("WARN: current-user language list setup failed; continuing install: {0}" -f $_.Exception.Message)
    }
}

if (-not $SkipUninstallEntry) {
    Write-UninstallEntry -InstallRoot $InstallationRoot -MachineScope:$Machine.IsPresent
}

if (-not $SkipTraySetup) {
    Write-TrayRunEntry -InstallRoot $InstallationRoot
    Start-TrayHelper -InstallRoot $InstallationRoot
}

if (-not $SkipTextServiceRestart) {
    Restart-TextInputServices
} elseif (-not $SkipTextInputUx) {
    # 不强制重启输入栈时，仍尝试轻量拉起 ctfmon，让语言栏尽快重新枚举。
    Start-TextInputUx
} else {
    Write-InstallLog 'SKIP: text input stack refresh suppressed by caller'
}

if ($SkipStaleHostRestart) {
    Write-InstallLog 'SKIP: stale runtime host restart suppressed by caller'
} elseif ($PromptStaleHostRestart) {
    Restart-StaleRuntimeHosts -InstallRoot $InstallationRoot -CurrentPayloadRoot $installedPayloadRoot -Prompt
} else {
    Write-InstallLog 'SKIP: stale runtime host restart not requested; listed apps can be restarted manually'
}

if ($SkipHealthCheck) {
    Write-InstallLog 'SKIP: install health check deferred to the final current-user step'
} else {
    Invoke-InstallHealthCheck -InstallRoot $InstallationRoot -PayloadRoot $installedPayloadRoot -DllX64 $installedDll -DllX86 $installedDllX86 -MachineScope:$Machine.IsPresent -LanguageListExpected:(-not $SkipLanguageList)
}

Write-InstallLog ("OK: installed to {0}" -f $InstallationRoot)
Write-Output ("Installed {0} to: {1}" -f $AppDisplayName, $InstallationRoot)
