function Assert-PackageTrimPath {
    param([string]$Root, [string]$Relative)
    if ([IO.Path]::IsPathRooted($Relative) -or $Relative -match '(^|[/\\])\.\.($|[/\\])|:') {
        throw "Unsafe package trim path: $Relative"
    }
    $prefix = [IO.Path]::GetFullPath($Root).TrimEnd('\') + '\'
    $full = [IO.Path]::GetFullPath((Join-Path $Root $Relative))
    if (-not $full.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Package trim path escapes stage: $Relative"
    }
    $parent = $full
    while ($parent.Length -ge $prefix.Length) {
        if ((Test-Path -LiteralPath $parent) -and ((Get-Item -LiteralPath $parent -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Reparse point in package trim path: $parent"
        }
        $parent = [IO.Path]::GetDirectoryName($parent)
    }
    $full
}

function Remove-PackageDeveloperAssets {
    param([string]$RepoRoot, [string]$Stage)
    $policyPath = Join-Path $RepoRoot 'shared/package_trim.json'
    $policy = [IO.File]::ReadAllText($policyPath) | ConvertFrom-Json
    if ($null -eq $policy.directories -or $null -eq $policy.file_globs) { throw 'Invalid package trim policy' }
    if ($policy.directories.Count -eq 0 -and $policy.file_globs.Count -eq 0) { return }
    $distPrefix = [IO.Path]::GetFullPath((Join-Path $RepoRoot 'dist')).TrimEnd('\') + '\'
    $stageItem = Get-Item -LiteralPath $Stage -Force
    if (-not $stageItem.FullName.StartsWith($distPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        ($stageItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Unsafe package trim stage: $Stage" }
    foreach ($relative in $policy.directories) {
        $target = Assert-PackageTrimPath -Root $stageItem.FullName -Relative $relative
        if (-not (Test-Path -LiteralPath $target -PathType Container)) { continue }
        $links = Get-ChildItem -LiteralPath $target -Force -Recurse | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }
        if ($links) { throw "Reparse point in package trim directory: $target" }
        Remove-Item -LiteralPath $target -Recurse -Force
    }
    foreach ($pattern in $policy.file_globs) {
        $null = Assert-PackageTrimPath -Root $stageItem.FullName -Relative ($pattern -replace '[*?\[\]]', '_')
        foreach ($file in @(Get-ChildItem -Path (Join-Path $stageItem.FullName $pattern) -File -Force)) {
            $relative = $file.FullName.Substring($stageItem.FullName.Length + 1)
            $target = Assert-PackageTrimPath -Root $stageItem.FullName -Relative $relative
            Remove-Item -LiteralPath $target -Force
        }
    }
}
