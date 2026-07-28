param(
    [string]$InstallationRoot = $PSScriptRoot
)

$ErrorActionPreference = 'Stop'

$InstallationRoot = [System.IO.Path]::GetFullPath($InstallationRoot)
$installDev = Join-Path $InstallationRoot 'install_dev.ps1'
$installCurrentUser = Join-Path $InstallationRoot 'install_current_user.ps1'

function Test-IsAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Invoke-RepairStep {
    param(
        [string]$ScriptPath,
        [string[]]$Arguments
    )

    if (-not (Test-Path -LiteralPath $ScriptPath)) {
        throw "Missing repair script: $ScriptPath"
    }

    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $ScriptPath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Repair step failed: $ScriptPath exit=$LASTEXITCODE"
    }
}

$isAdmin = Test-IsAdministrator
if ($isAdmin -and (Test-Path -LiteralPath $installDev)) {
    Invoke-RepairStep -ScriptPath $installDev -Arguments @(
        '-InstallationRoot', $InstallationRoot,
        '-Machine',
        '-SkipFileCopy',
        '-SkipUninstallEntry',
        '-SkipLanguageList',
        '-SkipTraySetup',
        '-SkipTextServiceRestart',
        '-SkipTextInputUx',
        '-PromptStaleHostRestart'
    )
}

Invoke-RepairStep -ScriptPath $installCurrentUser -Arguments @(
    '-InstallationRoot', $InstallationRoot,
    '-PromptStaleHostRestart'
)

Write-Host 'Kaixin input method repair completed.'
