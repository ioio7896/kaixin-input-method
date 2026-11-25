param()

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$installer = Join-Path $repo "tsf-tip\installer"

$scripts = Get-ChildItem -LiteralPath $installer -Filter "*.ps1"
foreach ($script in $scripts) {
    $tokens = $null
    $errors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile(
        $script.FullName,
        [ref]$tokens,
        [ref]$errors
    )
    if ($errors -and $errors.Count -gt 0) {
        throw "PowerShell parser errors in $($script.FullName): $($errors | Out-String)"
    }
}

$required = @(
    "restart_stale_hosts.ps1",
    "export_diagnostics.ps1",
    "stage_package.ps1",
    "kaixin.iss",
    "kaixin-user.iss",
    "kaixin-common-code.iss",
    "srf_ime_desktop.exe.manifest"
)
foreach ($name in $required) {
    $path = Join-Path $installer $name
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Missing installer source: $name"
    }
}

$nativeSetupSources = @(
    "pinyin-ime\src\bin\srf_ime_setup.rs",
    "pinyin-ime\src\bin\srf_ime_setup\config_migrate.rs",
    "pinyin-ime\src\bin\srf_ime_setup\manifest.rs",
    "pinyin-ime\src\bin\srf_ime_setup\win_acl.rs",
    "pinyin-ime\src\bin\srf_ime_setup\win_system.rs"
)
foreach ($rel in $nativeSetupSources) {
    $path = Join-Path $repo $rel
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Missing native setup source: $rel"
    }
}

function Assert-PathContains {
    param(
        [string]$Path,
        [string]$Needle
    )

    $text = Get-Content -LiteralPath $Path -Raw -Encoding UTF8
    if ($text -notlike "*$Needle*") {
        throw "Source $Path does not contain required text: $Needle"
    }
}

$winSystem = Join-Path $repo "pinyin-ime\src\bin\srf_ime_setup\win_system.rs"
Assert-PathContains -Path $winSystem -Needle "restore_microsoft_pinyin"
Assert-PathContains -Path $winSystem -Needle "InputMethodOverride"
Assert-PathContains -Path $winSystem -Needle "RunOnce"
Assert-PathContains -Path $winSystem -Needle "remove_tip_from_loaded_user_registry"
Assert-PathContains -Path (Join-Path $installer "kaixin.iss") -Needle "ArchitecturesAllowed=x64os"
Assert-PathContains -Path (Join-Path $installer "kaixin.iss") -Needle "RemoveCurrentUserData()"
Assert-PathContains -Path (Join-Path $installer "kaixin.iss") -Needle "srf_ime_setup.exe"
Assert-PathContains -Path (Join-Path $installer "kaixin-user.iss") -Needle "ArchitecturesAllowed=x64os"
Assert-PathContains -Path (Join-Path $installer "kaixin-user.iss") -Needle "RemoveCurrentUserData()"
Assert-PathContains -Path (Join-Path $installer "kaixin-user.iss") -Needle "srf_ime_setup.exe"

[xml]$settingsManifest = Get-Content -LiteralPath (Join-Path $installer "srf_ime_desktop.exe.manifest") -Raw
[xml]$trayManifest = Get-Content -LiteralPath (Join-Path $installer "srf_ime_desktop.exe.manifest") -Raw
$null = $settingsManifest.assembly
$null = $trayManifest.assembly

Write-Host "Installer source check passed"
