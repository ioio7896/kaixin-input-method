function Assert-ReleasePeMachine {
    param([string]$Path, [uint16]$Machine)
    $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($Path))
    try {
        if ($reader.ReadUInt16() -ne 0x5a4d) { throw "Invalid PE file: $Path" }
        $reader.BaseStream.Position = 0x3c
        $reader.BaseStream.Position = $reader.ReadUInt32()
        if ($reader.ReadUInt32() -ne 0x4550 -or $reader.ReadUInt16() -ne $Machine) {
            throw "Wrong PE architecture: $Path"
        }
    } finally {
        $reader.Dispose()
    }
}

function Assert-ReleasePayload {
    param([string]$Stage, [string]$Version, [bool]$IncludeOcr)
    if ([IO.File]::ReadAllText((Join-Path $Stage 'VERSION')).Trim() -ne $Version) {
        throw "Staged VERSION mismatch: $Stage"
    }
    $marker = [IO.File]::ReadAllText((Join-Path $Stage 'current_runtime_payload.txt')).Trim()
    if ($marker -notmatch '^runtime[\\/]pkg-[a-zA-Z0-9-]+$') { throw "Invalid runtime directory: $marker" }
    $runtime = Join-Path $Stage $marker
    foreach ($arch in 'x64', 'x86') {
        $machine = if ($arch -eq 'x64') { 0x8664 } else { 0x014c }
        foreach ($file in 'srf_tsf_tip.dll', 'srf_ime_overlay.exe') {
            Assert-ReleasePeMachine -Path (Join-Path $runtime "$arch/$file") -Machine $machine
        }
    }
    foreach ($file in 'LICENSE', 'LICENSE_SCOPE.md', 'NOTICE', 'THIRD_PARTY_NOTICES.md', 'user_data_manifest.json', 'lexicon/lexicon.pack', 'lexicon/cold_lexicon.sqlite', 'assets/kaixin-input.ico') {
        if ((Get-Item -LiteralPath (Join-Path $Stage $file)).Length -lt 1) { throw "Empty package file: $file" }
    }
    Assert-ReleasePeMachine -Path (Join-Path $Stage 'srf_ime_desktop.exe') -Machine 0x8664
    $ocrDirectory = Join-Path $Stage 'RapidOCRLite'
    if ($IncludeOcr) {
        Assert-ReleasePeMachine -Path (Join-Path $ocrDirectory 'rapidocrlite.dll') -Machine 0x8664
        foreach ($file in 'PP-OCRv6_det_medium.onnx', 'PP-OCRv6_det_small.onnx', 'PP-OCRv6_rec_medium.onnx', 'PP-OCRv6_rec_small.onnx', 'ppocrv6_dict.txt') {
            if ((Get-Item -LiteralPath (Join-Path $ocrDirectory "models/$file")).Length -lt 1) { throw "Empty OCR model or dictionary: $file" }
        }
    } elseif (Test-Path -LiteralPath $ocrDirectory) {
        throw 'IME-only package contains OCR files'
    }
    $privateFiles = Get-ChildItem -LiteralPath $Stage -Recurse -Force | Where-Object {
        $_.Name -match '(?i)^\.?python.*|\.py[cdow]?$|^(user_dict|clipboard_store|ocr_history|runtime_events)\.(sqlite|kxj)|^engine_capability\.' -or
        ($_.Attributes -band [IO.FileAttributes]::ReparsePoint)
    }
    if ($privateFiles) { throw "Unexpected runtime files: $($privateFiles.Name -join ', ')" }
    $runtime
}

function Test-StagedEngine {
    param([string]$Repo, [string]$Stage, [string]$Runtime, [string]$Version, [string]$Work, [string]$Commit, [string]$Profile)
    $protocol = [int][IO.File]::ReadAllText((Join-Path $Repo 'shared/engine_protocol_version.txt')).Trim()
    $probes = [ordered]@{
        x64 = Join-Path $Repo "tsf-tip/build-package/$Profile/srf_release_probe.exe"
        x86 = Join-Path $Repo "tsf-tip/build-package-x86/$Profile/srf_release_probe.exe"
    }
    $engine = Join-Path $Stage 'srf_ime_engine.exe'
    $components = [ordered]@{
        engine = Invoke-ReleaseCapture -Program $engine -Arguments @('--build-info') -Directory $Stage | ConvertFrom-Json
    }
    $prefixLength = $Stage.TrimEnd('\').Length + 1
    $runtimeRelative = $Runtime.Substring($prefixLength)
    foreach ($arch in $probes.Keys) {
        $dll = Join-Path $Runtime "$arch/srf_tsf_tip.dll"
        $components[$arch] = Invoke-ReleaseCapture -Program $probes[$arch] -Arguments @('--metadata', $dll) -Directory $Stage | ConvertFrom-Json
    }
    foreach ($name in $components.Keys) {
        $metadata = $components[$name]
        $bits = if ($name -eq 'x86') { 32 } else { 64 }
        $kind = if ($name -eq 'engine') { 'engine' } else { 'tsf' }
        if ($metadata.schema -ne 1 -or $metadata.component -ne $kind -or $metadata.bits -ne $bits -or
            $metadata.version -ne $Version -or $metadata.protocol -ne $protocol -or $metadata.git -ne $Commit) {
            throw "Release metadata mismatch for $name in $Stage"
        }
    }

    $local = Join-Path $Work (Split-Path -Leaf $Stage)
    $isolated = Join-Path $local 'Programs/kaixin'
    [IO.Directory]::CreateDirectory($isolated) | Out-Null
    $sources = @($engine, (Join-Path $Stage 'VERSION'), (Join-Path $Stage 'current_runtime_payload.txt'))
    $sources += @(Get-ChildItem -LiteralPath (Join-Path $Stage 'lexicon') -File -Recurse | ForEach-Object FullName)
    $sources += @(Get-ChildItem -LiteralPath (Join-Path $Stage 'model-data') -File | ForEach-Object FullName)
    $sources += @($probes.Keys | ForEach-Object { Join-Path $Runtime "$_/srf_tsf_tip.dll" })
    $payload = [ordered]@{}
    foreach ($source in $sources) {
        $relative = $source.Substring($prefixLength)
        $target = Join-Path $isolated $relative
        [IO.Directory]::CreateDirectory((Split-Path -Parent $target)) | Out-Null
        [IO.File]::Copy($source, $target)
        $hash = Get-Sha256Hex -Path $source
        if ((Get-Sha256Hex -Path $target) -ne $hash) { throw "Smoke test payload copy differs: $relative" }
        $payload[$relative.Replace('\', '/')] = $hash
    }
    $checks = [ordered]@{}
    Invoke-IsolatedReleaseTest -Directory $local -Command {
        $info = New-ReleaseProcessInfo -Program (Join-Path $isolated 'srf_ime_engine.exe') -Arguments @('--no-warmup') -Directory $isolated
        $process = [Diagnostics.Process]::Start($info)
        try {
            $output = $process.StandardOutput.ReadToEndAsync()
            $errors = $process.StandardError.ReadToEndAsync()
            $process.StandardInput.Close()
            foreach ($arch in $probes.Keys) {
                $dll = Join-Path (Join-Path $isolated $runtimeRelative) "$arch/srf_tsf_tip.dll"
                $probe = Invoke-ReleaseCapture -Program $probes[$arch] -Arguments @('--probe', $dll) -Directory $isolated | ConvertFrom-Json
                if (-not $probe.handshake -or $probe.input -ne 'nihao' -or -not $probe.matched) { throw "Invalid $arch nihao smoke result" }
                $checks[$arch] = $probe
            }
            if ($process.HasExited) { throw 'Staged engine exited during release verification' }
        } finally {
            if (-not $process.HasExited) { $process.Kill() }
            $process.WaitForExit()
            [IO.File]::WriteAllText((Join-Path $local 'engine.log'), $output.Result + $errors.Result)
            $process.Dispose()
        }
    }
    $report = [ordered]@{ schema = 1; version = $Version; components = $components; probes = $checks; payload_sha256 = $payload }
    [IO.File]::WriteAllText((Join-Path $Repo ('dist/release-validation-{0}.json' -f (Split-Path -Leaf $Stage))), ($report | ConvertTo-Json -Depth 12))
    Write-Host 'Staged engine and x64/x86 TSF handshake passed'
}

function Test-StagedOcr {
    param([string]$Repo, [string]$Stage, [string]$Version, [string]$Work)
    $models = [ordered]@{ fast = 'small-det+small-rec'; balanced = 'small-det+medium-rec'; accurate = 'medium-det+medium-rec' }
    $inputLines = foreach ($profile in $models.Keys) {
        @{ command = 'warmup'; profile = $profile; lang = 'zh' } | ConvertTo-Json -Compress
        @{ command = 'warmup'; profile = $profile; lang = 'en' } | ConvertTo-Json -Compress
        @{ image = Join-Path $Repo 'assets/kaixin-input-icon.png'; profile = $profile; lang = 'auto'; min_score = 0.5 } | ConvertTo-Json -Compress
    }
    $inputText = ($inputLines + '{"command":"quit"}' -join "`n") + "`n"
    $isolated = Join-Path $Work 'ocr-user'
    $raw = Invoke-IsolatedReleaseTest -Directory $isolated -Command {
        Invoke-ReleaseCapture -Program (Join-Path $Stage 'srf_ime_desktop.exe') -Arguments @('--tool=ocr', '--ocr-worker', '--stdio', '--validate-installed', '--no-fast-mode') -Directory $Stage -InputText $inputText -TimeoutSeconds 180
    }
    $responses = @($raw -split '\r?\n' | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json })
    if ($responses.Count -ne 9) { throw 'Incomplete OCR smoke responses' }
    $index = 0
    # Expected text in the checked-in test image (U+5F00 U+5FC3 U+8F93 U+5165).
    $expectedText = -join (0x5f00, 0x5fc3, 0x8f93, 0x5165 | ForEach-Object { [char]$_ })
    foreach ($profile in $models.Keys) {
        $warm, $alias, $result = $responses[$index..($index + 2)]
        $index += 3
        if (-not ($warm.ok -and $alias.ok -and $result.ok -and $warm.warmed_detection -and $warm.warmed_recognition -and $alias.already_warm) -or
            $result.model_type -ne $models[$profile] -or ($result.text -replace '\s', '') -notlike "*$expectedText*" -or
            $result.auto_language_retry -or $null -eq $result.cached_engines -or $result.cached_engines -gt 2) {
            throw "OCR smoke check failed: $profile"
        }
    }
    $report = [ordered]@{ version = $Version; exit_code = 0; responses = $responses }
    [IO.File]::WriteAllText((Join-Path $Repo 'dist/ocr-validation-kaixin-package-ocr.json'), ($report | ConvertTo-Json -Depth 20))
    Write-Host 'Staged OCR profiles, warmup, and model cache passed'
}
