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
    "install_dev.ps1",
    "install_current_user.ps1",
    "restart_stale_hosts.ps1",
    "export_diagnostics.ps1",
    "uninstall_dev.ps1",
    "uninstall_current_user.ps1",
    "stage_package.ps1",
    "kaixin.iss",
    "kaixin-common-code.iss",
    "srf_ime_settings.exe.manifest",
    "srf_ime_tray.exe.manifest"
)
foreach ($name in $required) {
    $path = Join-Path $installer $name
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Missing installer source: $name"
    }
}

function Assert-FileContains {
    param(
        [string]$Name,
        [string]$Needle
    )

    $path = Join-Path $installer $Name
    $text = Get-Content -LiteralPath $path -Raw -Encoding UTF8
    if ($text -notlike "*$Needle*") {
        throw "Installer source $Name does not contain required text: $Needle"
    }
}

Assert-FileContains -Name "uninstall_current_user.ps1" -Needle "uninstall_user.log"
Assert-FileContains -Name "uninstall_current_user.ps1" -Needle "RunOnce"
Assert-FileContains -Name "uninstall_dev.ps1" -Needle "uninstall_machine.log"
Assert-FileContains -Name "uninstall_dev.ps1" -Needle "Remove-TipFromLoadedUserRegistry"
Assert-FileContains -Name "kaixin-common-code.iss" -Needle "CommandLineRequestsRemoveUserData"
Assert-FileContains -Name "kaixin-common-code.iss" -Needle "CommandLineRequestsRemoveTransientUserData"

[xml]$settingsManifest = Get-Content -LiteralPath (Join-Path $installer "srf_ime_settings.exe.manifest") -Raw
[xml]$trayManifest = Get-Content -LiteralPath (Join-Path $installer "srf_ime_tray.exe.manifest") -Raw
$null = $settingsManifest.assembly
$null = $trayManifest.assembly

Write-Host "Installer source check passed"
