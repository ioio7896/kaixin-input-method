param([switch]$Fix)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$module = Join-Path $repo 'dist\dev-tools\PSScriptAnalyzer\1.24.0\PSScriptAnalyzer.psd1'
if (-not (Test-Path -LiteralPath $module)) {
    throw 'PSScriptAnalyzer 1.24.0 is required. Run scripts/setup_dev_tools.ps1.'
}
Import-Module $module -Force
$settings = Join-Path $PSScriptRoot 'PSScriptAnalyzerSettings.psd1'
$files = @(Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.ps1' -File)
$files += @(Get-ChildItem -LiteralPath (Join-Path $repo 'tsf-tip') -Filter '*.ps1' -File)
$files += @(Get-ChildItem -LiteralPath (Join-Path $repo 'tsf-tip\installer') -Filter '*.ps1' -File |
        Where-Object { $_.Name -notlike '*.generated.ps1' })
$diagnostics = @()
foreach ($file in $files) {
    $source = [System.IO.File]::ReadAllText($file.FullName)
    $formatted = Invoke-Formatter -ScriptDefinition $source -Settings $settings
    if ($Fix -and $source -ne $formatted) {
        # Windows PowerShell 5.1 needs a BOM to decode non-ASCII source reliably.
        [System.IO.File]::WriteAllText($file.FullName, $formatted.Replace("`r`n", "`n"), [System.Text.UTF8Encoding]::new($true))
    }
    $diagnostics += @(Invoke-ScriptAnalyzer -Path $file.FullName -Settings $settings)
}
if ($diagnostics.Count -gt 0) {
    $diagnostics | Format-Table ScriptName, Line, RuleName, Message -Wrap
    throw "PowerShell checks failed: $($diagnostics.Count) diagnostic(s)"
}
Write-Output "PowerShell checks passed: $($files.Count) script(s)"
