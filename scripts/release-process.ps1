function Invoke-ReleaseCommand {
    param([string]$Program, [string[]]$Arguments)
    Write-Host ("+ {0} {1}" -f (Split-Path -Leaf $Program), ($Arguments -join ' '))
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$(Split-Path -Leaf $Program) exited with code $LASTEXITCODE"
    }
}

function New-ReleaseProcessInfo {
    param([string]$Program, [string[]]$Arguments, [string]$Directory)
    # Windows PowerShell 5.1 uses ProcessStartInfo.Arguments, a Windows command line.
    $quoted = foreach ($arg in $Arguments) {
        '"' + ($arg -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
    }
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Program
    $info.Arguments = $quoted -join ' '
    $info.WorkingDirectory = $Directory
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.RedirectStandardInput = $true
    $info.StandardOutputEncoding = [Text.Encoding]::UTF8
    $info.StandardErrorEncoding = [Text.Encoding]::UTF8
    $info
}

function Invoke-ReleaseCapture {
    param(
        [string]$Program, [string[]]$Arguments, [string]$Directory,
        [string]$InputText, [int]$TimeoutSeconds = 35, [string]$SensitiveValue
    )
    $info = New-ReleaseProcessInfo -Program $Program -Arguments $Arguments -Directory $Directory
    $process = [Diagnostics.Process]::Start($info)
    try {
        $output = $process.StandardOutput.ReadToEndAsync()
        $errors = $process.StandardError.ReadToEndAsync()
        if ($InputText) {
            $bytes = [Text.Encoding]::UTF8.GetBytes($InputText)
            $process.StandardInput.BaseStream.Write($bytes, 0, $bytes.Length)
        }
        $process.StandardInput.Close()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            $process.Kill()
            $process.WaitForExit()
            throw "Timed out running $(Split-Path -Leaf $Program)"
        }
        if ($process.ExitCode -ne 0) {
            $message = $errors.Result
            if ($SensitiveValue) { $message = $message.Replace($SensitiveValue, '[redacted]') }
            throw "$(Split-Path -Leaf $Program) exited with code $($process.ExitCode): $message"
        }
        $output.Result
    } finally {
        $process.Dispose()
    }
}

function Invoke-IsolatedReleaseTest {
    param([string]$Directory, [scriptblock]$Command)
    [IO.Directory]::CreateDirectory($Directory) | Out-Null
    $previous = @{}
    $names = @('LOCALAPPDATA', 'APPDATA', 'SRF_USER_DICT_PATH', 'SRF_IME_LOG_LEVEL')
    $names += @(Get-ChildItem Env: | Where-Object Name -Like 'SRF_*' | ForEach-Object Name)
    foreach ($name in ($names | Select-Object -Unique)) {
        $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        [Environment]::SetEnvironmentVariable($name, $null, 'Process')
    }
    try {
        $env:LOCALAPPDATA = $Directory
        $env:APPDATA = $Directory
        $env:SRF_USER_DICT_PATH = Join-Path $Directory 'user_dict.kxj'
        $env:SRF_IME_LOG_LEVEL = 'error'
        & $Command
    } finally {
        foreach ($name in $previous.Keys) {
            [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process')
        }
    }
}
