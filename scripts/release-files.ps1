function Set-ReleaseVersion {
    param([string]$Repo, [string]$Version)
    $versionFile = Join-Path $Repo 'VERSION'
    $old = [IO.File]::ReadAllText($versionFile).Trim()
    $manifestPattern = [regex]'(?m)^version = "[^"\r\n]+"$'
    foreach ($relative in 'pinyin-ime/Cargo.toml', 'crates/kaixin-core/Cargo.toml') {
        $path = Join-Path $Repo $relative
        $text = [IO.File]::ReadAllText($path).Replace("`r`n", "`n")
        if (-not $manifestPattern.IsMatch($text)) { throw "Version field missing: $relative" }
        [IO.File]::WriteAllText($path, $manifestPattern.Replace($text, ('version = "{0}"' -f $Version), 1))
    }
    foreach ($relative in 'Cargo.lock', 'pinyin-ime/Cargo.lock') {
        $path = Join-Path $Repo $relative
        $text = [IO.File]::ReadAllText($path).Replace("`r`n", "`n")
        $pattern = [regex]'(?m)(^name = "(?:pinyin-ime|kaixin-core)"\nversion = ")[^"\r\n]+"$'
        if ($pattern.Matches($text).Count -ne 2) { throw "Application version fields missing: $relative" }
        [IO.File]::WriteAllText($path, $pattern.Replace($text, ('${1}' + $Version + '"')))
    }
    $readme = Join-Path $Repo 'README.md'
    $text = [IO.File]::ReadAllText($readme)
    $pattern = [regex]('(?m)^([^\r\n]+\*\*)' + [regex]::Escape($old) + '(\*\*)')
    if (-not $pattern.IsMatch($text)) { throw 'Current README version marker missing' }
    [IO.File]::WriteAllText($readme, $pattern.Replace($text, ('${1}' + $Version + '${2}'), 1))
    [IO.File]::WriteAllText($versionFile, $Version + "`n")
}

function Get-ReleaseSourcePaths {
    param([string]$Repo, [string]$Git)
    $text = Invoke-ReleaseCapture -Program $Git -Arguments @('ls-files', '-z', '--cached', '--others', '--exclude-standard') -Directory $Repo
    $relativeNames = $text.Split([char]0) | Where-Object { $_ -and $_ -notlike 'kaixin-source-*.zip' } | Sort-Object -Unique -CaseSensitive
    $prefix = [IO.Path]::GetFullPath($Repo).TrimEnd('\') + '\'
    foreach ($relative in $relativeNames) {
        $path = [IO.Path]::GetFullPath((Join-Path $Repo $relative))
        if (-not $path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) { throw "Source path escaped repository: $relative" }
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            $entry = Get-Item -LiteralPath $path -Force
            if (-not ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                [pscustomobject]@{ Relative = $relative; Path = $path }
            }
        }
    }
}

function Assert-ReleaseSourceEncoding {
    param([string]$Repo, [string]$Git)
    $utf8 = [Text.UTF8Encoding]::new($false, $true)
    foreach ($source in Get-ReleaseSourcePaths -Repo $Repo -Git $Git) {
        if ($source.Relative -match '\.(rs|cpp|h|hpp|ipp|ps1|json|toml|yml|yaml|iss|md)$') {
            try { $null = $utf8.GetString([IO.File]::ReadAllBytes($source.Path)) }
            catch { throw "Invalid UTF-8 source: $($source.Relative)" }
        }
        if ($source.Relative -like '*.ps1') {
            $tokens = $null
            $parseErrors = $null
            $null = [System.Management.Automation.Language.Parser]::ParseFile($source.Path, [ref]$tokens, [ref]$parseErrors)
            if ($parseErrors) { throw "PowerShell source parse error in $($source.Relative): $parseErrors" }
        }
    }
}

function New-ReleaseSourceBackup {
    param([string]$Repo, [string]$Git, [string]$Path, [string]$Version)
    Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
    $entries = @(Get-ReleaseSourcePaths -Repo $Repo -Git $Git)
    $archive = [IO.Compression.ZipFile]::Open($Path, [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($entry in $entries) {
            $null = [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $entry.Path, $entry.Relative, [IO.Compression.CompressionLevel]::Optimal)
        }
    } finally {
        $archive.Dispose()
    }
    $archive = [IO.Compression.ZipFile]::OpenRead($Path)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        if ($archive.Entries.Count -ne $entries.Count) { throw 'Incomplete source backup' }
        foreach ($entry in $entries) {
            $stream = $archive.GetEntry($entry.Relative).Open()
            try {
                $actual = [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '').ToLowerInvariant()
            } finally {
                $stream.Dispose()
            }
            if ($actual -ne (Get-Sha256Hex -Path $entry.Path)) { throw "Source backup differs: $($entry.Relative)" }
        }
        $reader = [IO.StreamReader]::new($archive.GetEntry('VERSION').Open())
        try {
            if ($reader.ReadToEnd().Trim() -ne $Version) { throw 'Source backup version differs' }
        } finally {
            $reader.Dispose()
        }
    } finally {
        $sha.Dispose()
        $archive.Dispose()
    }
}

function Publish-ReleaseFiles {
    param([string]$Repo, [string[]]$Installers, [string]$Backup, [string]$Report, [bool]$UserInstaller)
    $dist = Join-Path $Repo 'dist'
    $previous = Join-Path $dist 'previous-installers'
    [IO.Directory]::CreateDirectory($previous) | Out-Null
    $prefix = if ($UserInstaller) { 'kaixin-user-setup' } else { 'kaixin-setup' }
    $moves = [Collections.Generic.List[object]]::new()
    foreach ($file in (Get-ChildItem -LiteralPath $dist -File -Filter "$prefix-*.exe")) {
        if ($file.Name -match ('^' + $prefix + '-(ime|ocr)-')) {
            $moves.Add(@($file.FullName, (Join-Path $previous $file.Name)))
        }
    }
    foreach ($installer in $Installers) { $moves.Add(@($installer, (Join-Path $dist (Split-Path -Leaf $installer)))) }
    $moves.Add(@($Backup, (Join-Path $Repo (Split-Path -Leaf $Backup))))
    $moves.Add(@($Report, (Join-Path $dist (Split-Path -Leaf $Report))))
    foreach ($move in $moves) {
        if (Test-Path -LiteralPath $move[1]) { throw "Release destination already exists: $($move[1])" }
    }
    $finished = 0
    try {
        foreach ($move in $moves) {
            Move-Item -LiteralPath $move[0] -Destination $move[1]
            $finished++
        }
    } catch {
        for ($index = $finished - 1; $index -ge 0; $index--) {
            Move-Item -LiteralPath $moves[$index][1] -Destination $moves[$index][0]
        }
        throw
    }
}
