[CmdletBinding(DefaultParameterSetName = 'Register')]
param(
    [string]$DllPath = (Join-Path $PSScriptRoot 'build\Release\srf_tsf_tip.dll'),
    [ValidateSet('x64', 'x86')]
    [string]$Arch = $(if ([Environment]::Is64BitProcess) { 'x64' } else { 'x86' }),
    [Parameter(ParameterSetName = 'Register')]
    [switch]$Machine,
    [Parameter(ParameterSetName = 'Register')]
    [switch]$Unregister,
    [Parameter(Mandatory, ParameterSetName = 'Profile')]
    [ValidateSet('enable-profile', 'notify-language')]
    [string]$ProfileAction
)

$ErrorActionPreference = 'Stop'
$dll = Get-Item -LiteralPath $DllPath -ErrorAction Stop
$systemDir = if ($Arch -eq 'x86' -and [Environment]::Is64BitOperatingSystem) {
    'SysWOW64'
} elseif ($Arch -eq 'x64' -and -not [Environment]::Is64BitProcess) {
    'Sysnative'
} else {
    'System32'
}
$registrar = Join-Path $env:WINDIR "$systemDir\regsvr32.exe"
$arguments = @('/s')
if ($Unregister) { $arguments += '/u' }
if ($Machine) { $arguments += '/n', '/i:machine' }
if ($ProfileAction) { $arguments += '/n', "/i:$ProfileAction" }
$arguments += ('"{0}"' -f $dll.FullName)

# Wait for the registrar itself. A desktop service started by TSF may outlive it.
$process = Start-Process -FilePath $registrar -ArgumentList $arguments -WindowStyle Hidden -PassThru
try {
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) {
        throw ("{0} regsvr32 failed (exit {1}): {2}" -f $Arch, $process.ExitCode, ($arguments -join ' '))
    }
} finally {
    $process.Dispose()
}
