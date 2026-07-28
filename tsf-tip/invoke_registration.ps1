param(
    [string]$DllPath = (Join-Path $PSScriptRoot 'build\Release\srf_tsf_tip.dll'),
    [switch]$Machine,
    [switch]$Unregister
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath $DllPath)) {
    throw "DLL not found: $DllPath"
}

if (-not ('SrfTipRegistration.NativeMethods' -as [type])) {
    Add-Type -Language CSharp -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

namespace SrfTipRegistration {
    public static class NativeMethods {
        [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
        public static extern IntPtr LoadLibraryEx(string lpFileName, IntPtr hFile, uint dwFlags);

        [DllImport("kernel32.dll", SetLastError = true)]
        public static extern bool FreeLibrary(IntPtr hModule);

        [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Ansi)]
        public static extern IntPtr GetProcAddress(IntPtr hModule, string procName);
    }

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    public delegate int DllRegisterServerDelegate();

    [UnmanagedFunctionPointer(CallingConvention.StdCall, CharSet = CharSet.Unicode)]
    public delegate int DllInstallDelegate([MarshalAs(UnmanagedType.Bool)] bool install, string cmdLine);
}
"@
}

$LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR = 0x00000100
$LOAD_LIBRARY_SEARCH_SYSTEM32 = 0x00000800

function Get-Delegate {
    param(
        [IntPtr]$Module,
        [string]$Name,
        [Type]$DelegateType
    )

    $proc = [SrfTipRegistration.NativeMethods]::GetProcAddress($Module, $Name)
    if ($proc -eq [IntPtr]::Zero) {
        throw "Export not found: $Name"
    }
    return [Runtime.InteropServices.Marshal]::GetDelegateForFunctionPointer($proc, $DelegateType)
}

$dllPath = [System.IO.Path]::GetFullPath($DllPath)
$loadFlags = $LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR -bor $LOAD_LIBRARY_SEARCH_SYSTEM32
$module = [SrfTipRegistration.NativeMethods]::LoadLibraryEx($dllPath, [IntPtr]::Zero, [uint32]$loadFlags)
if ($module -eq [IntPtr]::Zero) {
    $code = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    throw "LoadLibraryEx failed ($code): $dllPath"
}

try {
    if ($Machine) {
        $delegate = Get-Delegate -Module $module -Name 'DllInstall' -DelegateType ([SrfTipRegistration.DllInstallDelegate])
        $hr = $delegate.Invoke(-not $Unregister, 'machine')
    } else {
        $exportName = if ($Unregister) { 'DllUnregisterServer' } else { 'DllRegisterServer' }
        $delegate = Get-Delegate -Module $module -Name $exportName -DelegateType ([SrfTipRegistration.DllRegisterServerDelegate])
        $hr = $delegate.Invoke()
    }

    if ($hr -lt 0) {
        $hrHex = '{0:X8}' -f ([uint32]([int64]$hr + 0x100000000))
    } else {
        $hrHex = '{0:X8}' -f ([uint32]$hr)
    }
    Write-Output "HRESULT=0x$hrHex"
    if ($hr -lt 0) {
        $scope = if ($Machine) { 'machine' } else { 'user' }
        $operation = if ($Unregister) { 'unregister' } else { 'register' }
        $processArch = if ([Environment]::Is64BitProcess) { 'x64' } else { 'x86' }
        $message = "Registration failed: HRESULT=0x$hrHex Operation=$operation Scope=$scope Process=$processArch DllPath=$dllPath"
        throw [System.Runtime.InteropServices.COMException]::new($message, $hr)
    }
}
finally {
    [void][SrfTipRegistration.NativeMethods]::FreeLibrary($module)
}
