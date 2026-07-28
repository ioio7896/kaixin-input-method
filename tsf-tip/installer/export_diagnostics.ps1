param(
    [string]$InstallationRoot = $PSScriptRoot,
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'

$appStateRoot = Join-Path $env:LOCALAPPDATA 'kaixin'
$commonAppData = [Environment]::GetFolderPath('CommonApplicationData')
if ([string]::IsNullOrWhiteSpace($commonAppData)) {
    $commonAppData = $env:ProgramData
}
$machineStateRoot = Join-Path $commonAppData 'kaixin'
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $diagnosticsRoot = Join-Path $appStateRoot 'diagnostics'
    New-Item -ItemType Directory -Path $diagnosticsRoot -Force | Out-Null
    $OutputPath = Join-Path $diagnosticsRoot ("kaixin-diagnostics-{0}.zip" -f $stamp)
}

$tempRoot = Join-Path $env:TEMP ("kaixin-diagnostics-{0}-{1}" -f $stamp, [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null

function Copy-IfExists {
    param(
        [string]$Path,
        [string]$RelativePath
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }
    $destination = Join-Path $tempRoot $RelativePath
    $parent = Split-Path -Parent $destination
    if (-not [string]::IsNullOrWhiteSpace($parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    Copy-Item -LiteralPath $Path -Destination $destination -Recurse -Force -ErrorAction SilentlyContinue
}

function Get-RedactionPairs {
    $pairs = @(
        @{ Token = '<INSTALLATION_ROOT>'; Value = $InstallationRoot },
        @{ Token = '<USER_STATE_ROOT>'; Value = $appStateRoot },
        @{ Token = '<MACHINE_STATE_ROOT>'; Value = $machineStateRoot },
        @{ Token = '<USERPROFILE>'; Value = $env:USERPROFILE },
        @{ Token = '<LOCALAPPDATA>'; Value = $env:LOCALAPPDATA },
        @{ Token = '<APPDATA>'; Value = $env:APPDATA },
        @{ Token = '<TEMP>'; Value = $env:TEMP },
        @{ Token = '<COMPUTERNAME>'; Value = $env:COMPUTERNAME },
        @{ Token = '<USERNAME>'; Value = $env:USERNAME }
    )
    $pairs |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_.Value) } |
        Sort-Object { $_.Value.Length } -Descending
}

function Sanitize-Text {
    param([string]$Text)

    if ($null -eq $Text) {
        return ''
    }
    $result = $Text
    foreach ($pair in Get-RedactionPairs) {
        $pattern = [regex]::Escape($pair.Value)
        $result = [regex]::Replace(
            $result,
            $pattern,
            $pair.Token,
            [System.Text.RegularExpressions.RegexOptions]::IgnoreCase)
    }
    $result = [regex]::Replace(
        $result,
        '[A-Za-z]:\\Users\\[^\\\r\n\t ]+',
        '<USERPROFILE>',
        [System.Text.RegularExpressions.RegexOptions]::IgnoreCase)
    return $result
}

function Copy-TextIfExistsSanitized {
    param(
        [string]$Path,
        [string]$RelativePath
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return
    }
    $destination = Join-Path $tempRoot $RelativePath
    $parent = Split-Path -Parent $destination
    if (-not [string]::IsNullOrWhiteSpace($parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    $text = Get-Content -LiteralPath $Path -Raw -Encoding UTF8 -ErrorAction SilentlyContinue
    if ($null -eq $text) {
        $text = Get-Content -LiteralPath $Path -Raw -ErrorAction SilentlyContinue
    }
    Set-Content -LiteralPath $destination -Value (Sanitize-Text $text) -Encoding UTF8
}

function Copy-DirectoryTextSanitized {
    param(
        [string]$Path,
        [string]$RelativePath
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        return
    }
    foreach ($file in @(Get-ChildItem -LiteralPath $Path -File -Recurse -ErrorAction SilentlyContinue)) {
        $relative = $file.FullName.Substring($Path.Length).TrimStart('\', '/')
        Copy-TextIfExistsSanitized -Path $file.FullName -RelativePath (Join-Path $RelativePath $relative)
    }
}

try {
    Copy-IfExists -Path (Join-Path $InstallationRoot 'VERSION') -RelativePath 'install\VERSION'
    Copy-IfExists -Path (Join-Path $InstallationRoot 'current_runtime_payload.txt') -RelativePath 'install\current_runtime_payload.txt'
    Copy-TextIfExistsSanitized -Path (Join-Path $InstallationRoot 'install_machine.log') -RelativePath 'install\install_machine.log'
    Copy-TextIfExistsSanitized -Path (Join-Path $InstallationRoot 'tip_registration.log') -RelativePath 'install\tip_registration.log'

    Copy-TextIfExistsSanitized -Path (Join-Path $appStateRoot 'install_user.log') -RelativePath 'user\install_user.log'
    Copy-TextIfExistsSanitized -Path (Join-Path $appStateRoot 'install_language_list.log') -RelativePath 'user\install_language_list.log'
    Copy-TextIfExistsSanitized -Path (Join-Path $appStateRoot 'uninstall_user.log') -RelativePath 'user\uninstall_user.log'
    Copy-TextIfExistsSanitized -Path (Join-Path $appStateRoot 'tip_registration_user.log') -RelativePath 'user\tip_registration_user.log'
    Copy-TextIfExistsSanitized -Path (Join-Path $appStateRoot 'kaixin.ini') -RelativePath 'user\kaixin.ini'
    Copy-DirectoryTextSanitized -Path (Join-Path $appStateRoot 'logs') -RelativePath 'user\logs'

    Copy-TextIfExistsSanitized -Path (Join-Path $machineStateRoot 'uninstall_machine.log') -RelativePath 'machine\uninstall_machine.log'

    $summary = @(
        "created=$((Get-Date).ToString('o'))",
        "installation_root=<INSTALLATION_ROOT>",
        "user_state_root=<USER_STATE_ROOT>",
        "machine_state_root=<MACHINE_STATE_ROOT>",
        "computer_name=<COMPUTERNAME>",
        "user_name=<USERNAME>",
        "os=$([System.Environment]::OSVersion.VersionString)",
        "note=diagnostic text files are redacted before packaging"
    )
    Set-Content -LiteralPath (Join-Path $tempRoot 'summary.txt') -Value $summary -Encoding UTF8

    if (Test-Path -LiteralPath $OutputPath) {
        Remove-Item -LiteralPath $OutputPath -Force
    }
    Compress-Archive -Path (Join-Path $tempRoot '*') -DestinationPath $OutputPath -Force
    Write-Host "Diagnostics package: $OutputPath"
    Start-Process -FilePath explorer.exe -ArgumentList ('/select,"{0}"' -f $OutputPath) -ErrorAction SilentlyContinue | Out-Null
} finally {
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
