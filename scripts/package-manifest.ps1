function Get-Sha256Hex {
    param([string]$Path)

    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $stream = [System.IO.File]::OpenRead($Path)
        try {
            $hashBytes = $sha256.ComputeHash($stream)
        } finally {
            $stream.Dispose()
        }
    } finally {
        $sha256.Dispose()
    }

    return ([System.BitConverter]::ToString($hashBytes)).Replace('-', '').ToLowerInvariant()
}

function Write-PackageHashManifest {
    param(
        [string]$Root,
        [string]$ManifestName
    )

    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\')
    $manifestPath = Join-Path $rootFull $ManifestName
    $files = Get-ChildItem -LiteralPath $rootFull -Recurse -File -Force |
        Where-Object { -not $_.FullName.Equals($manifestPath, [System.StringComparison]::OrdinalIgnoreCase) } |
        Sort-Object FullName

    $lines = New-Object 'System.Collections.Generic.List[string]'
    foreach ($file in $files) {
        $full = [System.IO.Path]::GetFullPath($file.FullName)
        $relative = $full.Substring($rootFull.Length + 1).Replace('\', '/')
        $hash = Get-Sha256Hex -Path $full
        [void]$lines.Add(('{0} *{1}' -f $hash, $relative))
    }

    Set-Content -LiteralPath $manifestPath -Value ([string[]]$lines) -Encoding UTF8
}

function Get-StringSha256Hex {
    param([string]$Value)

    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($Value)
        $hashBytes = $sha256.ComputeHash($bytes)
    } finally {
        $sha256.Dispose()
    }
    return ([System.BitConverter]::ToString($hashBytes)).Replace('-', '').ToLowerInvariant()
}

function Write-ComponentManifestFromPackageManifest {
    param(
        [string]$Root,
        [string]$PackageManifest,
        [string]$ComponentManifest
    )

    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\')
    $packageManifestPath = Join-Path $rootFull $PackageManifest
    $componentManifestPath = Join-Path $rootFull $ComponentManifest
    $packageLines = @(Get-Content -LiteralPath $packageManifestPath -Encoding UTF8 | Where-Object {
            -not [string]::IsNullOrWhiteSpace($_) -and -not $_.StartsWith('#')
        })
    $components = [ordered]@{
        rapidocr_payload = 'RapidOCRLite/'
    }
    $output = New-Object 'System.Collections.Generic.List[string]'
    [void]$output.Add('[components]')
    foreach ($entry in $components.GetEnumerator()) {
        $prefix = $entry.Value
        $matching = @($packageLines | Where-Object {
                $separator = $_.IndexOf(' *')
                $separator -ge 0 -and $_.Substring($separator + 2).StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)
            })
        $componentId = if ($matching.Count -gt 0) {
            Get-StringSha256Hex -Value (($matching -join "`n") + "`n")
        } else {
            'absent'
        }
        [void]$output.Add(('{0}={1}' -f $entry.Key, $componentId))
    }
    Set-Content -LiteralPath $componentManifestPath -Value ([string[]]$output) -Encoding UTF8

    $relative = [System.IO.Path]::GetFileName($componentManifestPath)
    $componentHash = Get-Sha256Hex -Path $componentManifestPath
    Add-Content -LiteralPath $packageManifestPath -Value ('{0} *{1}' -f $componentHash, $relative) -Encoding UTF8
}

function Write-StageManifests {
    param([string]$Root)
    $componentName = 'component_manifest.ini'
    $manifestName = 'package_manifest.sha256'
    $componentPath = Join-Path $Root $componentName
    if (Test-Path -LiteralPath $componentPath -PathType Leaf) {
        Remove-Item -LiteralPath $componentPath -Force
    }
    Write-PackageHashManifest -Root $Root -ManifestName $manifestName
    Write-ComponentManifestFromPackageManifest -Root $Root -PackageManifest $manifestName -ComponentManifest $componentName
}

function Assert-StageManifests {
    param([string]$Root)
    $prefix = [IO.Path]::GetFullPath($Root).TrimEnd('\') + '\'
    $manifest = Join-Path $Root 'package_manifest.sha256'
    $listed = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($line in [IO.File]::ReadAllLines($manifest)) {
        if ($line -notmatch '^([0-9a-fA-F]{64}) \*([^\\:]+)$') { throw "Invalid package manifest line: $line" }
        $expected = $Matches[1]
        $relative = $Matches[2]
        if ($relative.Split('/') -contains '..') { throw "Unsafe package entry: $relative" }
        $path = [IO.Path]::GetFullPath((Join-Path $Root $relative))
        if (-not $path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) -or
            $path -eq $manifest -or -not $listed.Add($path)) { throw "Invalid or duplicate package entry: $relative" }
        if ((Get-Sha256Hex -Path $path) -ne $expected) { throw "Package hash mismatch: $relative" }
    }
    $files = @(Get-ChildItem -LiteralPath $Root -Recurse -File -Force | Where-Object FullName -ne $manifest)
    if ($listed.Count -eq 0 -or $listed.Count -ne $files.Count -or ($files | Where-Object { -not $listed.Contains($_.FullName) })) {
        throw "Package manifest coverage mismatch: $Root"
    }
    Write-Host "Verified $($listed.Count) package hashes"
}
