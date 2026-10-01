[CmdletBinding()]
param([switch]$Apply, [switch]$HistoryOnly, [switch]$CleanRustDebug, [ValidateRange(1,20)][int]$KeepBatches = 3)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..')).TrimEnd('\')
$targets = [Collections.Generic.List[string]]::new()
$history = Join-Path $root 'dist\previous-installers'
if (Test-Path -LiteralPath $history) {
    Get-ChildItem -LiteralPath $history -Directory | Sort-Object Name -Descending | Select-Object -Skip $KeepBatches | ForEach-Object { $targets.Add($_.FullName) }
}
if (-not $HistoryOnly) {
    foreach ($name in @('build','tsf-tip\build-codex-current-x64','tsf-tip\build-codex-current-x86','tsf-tip\build-codex-fix-x64','tsf-tip\build-policy-tests')) {
        $target = Join-Path $root $name
        if (Test-Path -LiteralPath $target) { $targets.Add($target) }
    }
}
if ($CleanRustDebug) {
    $target = Join-Path $root 'pinyin-ime\target\debug'
    if (Test-Path -LiteralPath $target) { $targets.Add($target) }
}
[long]$removed = 0
foreach ($target in $targets) {
    $absolute = [IO.Path]::GetFullPath($target)
    if (-not $absolute.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)) { throw "Outside workspace: $absolute" }
    $item = Get-Item -LiteralPath $absolute -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Refusing reparse point: $absolute" }
    $items = @(Get-ChildItem -LiteralPath $absolute -Recurse -Force)
    if ($items | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }) { throw "Refusing nested reparse point: $absolute" }
    [long]$bytes = ($items | Where-Object { -not $_.PSIsContainer } | Measure-Object Length -Sum).Sum
    Write-Output "$absolute : $bytes bytes"
    if ($Apply) { Remove-Item -LiteralPath $absolute -Recurse -Force; $removed += $bytes }
}
Write-Output "Removed bytes: $removed (Apply=$Apply; retained history batches=$KeepBatches)"
