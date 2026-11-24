function Find-ReleaseTool {
    param([string]$Name, [string[]]$Candidates = @())
    $command = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($command) { return $command.Source }
    foreach ($candidate in $Candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
    }
    throw "Required release tool not found: $Name"
}

function Get-ReleaseCMake {
    $programFiles = [Environment]::GetEnvironmentVariable('ProgramFiles(x86)')
    $vswhere = Join-Path $programFiles 'Microsoft Visual Studio/Installer/vswhere.exe'
    $candidates = @()
    if (Test-Path -LiteralPath $vswhere -PathType Leaf) {
        $instances = @(& $vswhere -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
        $candidates = @($instances | ForEach-Object { Join-Path $_ 'Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe' })
    }
    Find-ReleaseTool -Name 'cmake.exe' -Candidates $candidates
}

function Get-ReleaseInnoSetup {
    $candidates = foreach ($programFiles in @([Environment]::GetEnvironmentVariable('ProgramFiles(x86)'), $env:ProgramFiles) | Select-Object -Unique) {
        foreach ($version in '7', '6') { Join-Path $programFiles "Inno Setup $version/ISCC.exe" }
    }
    Find-ReleaseTool -Name 'ISCC.exe' -Candidates $candidates
}

function Get-ReleaseSigning {
    param([bool]$Requested, [bool]$Required)
    $thumbprint = $env:KX_SIGN_CERT_SHA1
    $certificate = $env:KX_SIGN_PFX
    if (-not ($Requested -or $Required -or $thumbprint -or $certificate)) { return }
    $candidates = @()
    if ($env:KX_SIGNTOOL) { $candidates += $env:KX_SIGNTOOL }
    foreach ($programFiles in @([Environment]::GetEnvironmentVariable('ProgramFiles(x86)'), $env:ProgramFiles) | Select-Object -Unique) {
        $path = Join-Path $programFiles 'Windows Kits/10/bin/*/x64/signtool.exe'
        $candidates += @(Get-ChildItem -Path $path -File -ErrorAction SilentlyContinue | Sort-Object FullName -Descending | ForEach-Object FullName)
    }
    try {
        $tool = if ($env:KX_SIGNTOOL -and (Test-Path -LiteralPath $env:KX_SIGNTOOL -PathType Leaf)) {
            (Resolve-Path -LiteralPath $env:KX_SIGNTOOL).Path
        } else {
            Find-ReleaseTool -Name 'signtool.exe' -Candidates $candidates
        }
        if (-not ($thumbprint -or $certificate)) { throw 'Set KX_SIGN_CERT_SHA1 or KX_SIGN_PFX' }
    } catch {
        if ($Required) { throw }
        Write-Warning "Signing skipped: $_"
        return
    }
    $parameters = @('sign', '/fd', 'SHA256')
    $timestamp = [Environment]::GetEnvironmentVariable('KX_SIGN_TIMESTAMP_URL', 'Process')
    if ($null -eq $timestamp) { $timestamp = 'http://timestamp.digicert.com' }
    if ($timestamp) { $parameters += @('/tr', $timestamp, '/td', 'SHA256') }
    if ($thumbprint) {
        $parameters += @('/sha1', $thumbprint)
    } else {
        $parameters += @('/f', $certificate)
        if ($env:KX_SIGN_PFX_PASSWORD) { $parameters += @('/p', $env:KX_SIGN_PFX_PASSWORD) }
    }
    [pscustomobject]@{ Program = $tool; Arguments = $parameters }
}

function Set-ReleaseFileSignature {
    param($Signing, [string]$Path)
    if ($Signing) {
        $arguments = $Signing.Arguments + @($Path)
        $null = Invoke-ReleaseCapture -Program $Signing.Program -Arguments $arguments -Directory (Split-Path -Parent $Path) -TimeoutSeconds 120 -SensitiveValue $env:KX_SIGN_PFX_PASSWORD
    }
}

function Set-ReleaseRustFlags {
    param([string]$Repo)
    $separator = [char]31
    $flags = @(if (Test-Path Env:CARGO_ENCODED_RUSTFLAGS) { $env:CARGO_ENCODED_RUSTFLAGS.Split($separator) | Where-Object { $_ } } else { $env:RUSTFLAGS -split '\s+' | Where-Object { $_ } })
    $user = [Environment]::GetFolderPath('UserProfile')
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $user '.cargo' }
    $rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $user '.rustup' }
    $mappings = [ordered]@{ '/build/user' = $user; '/build/kaixin' = $Repo; '/build/cargo' = $cargoHome; '/build/rustup' = $rustupHome }
    foreach ($key in $mappings.Keys) {
        $path = [IO.Path]::GetFullPath($mappings[$key])
        foreach ($spelling in @($path, $path.Replace('\', '/')) | Select-Object -Unique) {
            $flags += "--remap-path-prefix=$spelling=$key"
        }
    }
    $env:CARGO_ENCODED_RUSTFLAGS = $flags -join $separator
}

function Invoke-ReleaseRustTests {
    param([string]$Repo, [string]$Cargo, [string]$Work)
    $plan = [IO.File]::ReadAllText((Join-Path $Repo 'shared/verification.json')) | ConvertFrom-Json
    Invoke-IsolatedReleaseTest -Directory (Join-Path $Work 'rust-test-user') -Command {
        foreach ($profile in $plan.rust_profiles.PSObject.Properties) {
            $options = @('--workspace', '--all-targets', '--locked') + @($profile.Value)
            Write-Host "Rust test profile: $($profile.Name)"
            Invoke-ReleaseCommand -Program $Cargo -Arguments (@('clippy') + $options + @('--', '-D', 'warnings'))
            Invoke-ReleaseCommand -Program $Cargo -Arguments (@('test') + $options)
        }
    }
}

function Invoke-ReleaseNativeBuild {
    param([string]$Repo, [string]$CMake, [bool]$SkipTests)
    $testSetting = if ($SkipTests) { 'OFF' } else { 'ON' }
    $source = Join-Path $Repo 'tsf-tip'
    $directories = [ordered]@{ x64 = 'build-package'; Win32 = 'build-package-x86' }
    foreach ($arch in $directories.Keys) {
        $build = Join-Path $source $directories[$arch]
        Invoke-ReleaseCommand -Program $CMake -Arguments @('-S', $source, '-B', $build, '-A', $arch, "-DBUILD_TESTING=$testSetting")
        Invoke-ReleaseCommand -Program $CMake -Arguments @('--build', $build, '--config', 'Release', '--parallel', '1', '--', '/nodeReuse:false')
        if (-not $SkipTests) {
            $ctest = Join-Path (Split-Path -Parent $CMake) 'ctest.exe'
            Invoke-ReleaseCommand -Program $ctest -Arguments @('--test-dir', $build, '-C', 'Release', '--output-on-failure')
        }
    }
}

function Invoke-ReleaseInstallers {
    param([string]$Repo, [string]$Inno, [string]$Pending, [string]$Version, [string]$Stamp, [bool]$UserInstaller)
    $script = if ($UserInstaller) { 'kaixin-user.iss' } else { 'kaixin.iss' }
    $setup = if ($UserInstaller) { 'kaixin-user-setup' } else { 'kaixin-setup' }
    $jobs = [Collections.Generic.List[object]]::new()
    try {
        foreach ($variant in 'ime', 'ocr') {
            $stage = Join-Path $Repo "dist/kaixin-package-$variant"
            $baseName = "$setup-$variant-$Version-$Stamp"
            $component = [IO.File]::ReadAllText((Join-Path $stage 'component_manifest.ini'))
            if ($component -notmatch '(?m)^rapidocr_payload=(absent|[a-f0-9]{64})\r?$') { throw "Invalid OCR component manifest: $stage" }
            $componentId = $Matches[1]
            $ocr = [int]($variant -eq 'ocr')
            $options = @("/O$Pending", "/DKXAppVersion=$Version", "/DKXPackageDir=$stage", "/DKXOutputBaseFilename=$baseName", "/DKXIncludeOcr=$ocr", '/DKXIncludeTranslation=1', "/DKXComponentRapidOcrPayload=$componentId", (Join-Path $Repo "tsf-tip/installer/$script"))
            Write-Host "Compiling $baseName.exe"
            $info = New-ReleaseProcessInfo -Program $Inno -Arguments $options -Directory $Repo
            $job = [Diagnostics.Process]::Start($info)
            $job.StandardInput.Close()
            $record = [pscustomobject]@{
                Program = $job
                Output = $job.StandardOutput.ReadToEndAsync()
                Errors = $job.StandardError.ReadToEndAsync()
                Path = Join-Path $Pending "$baseName.exe"
                Log = Join-Path $Repo "dist/inno-$variant-$Stamp.log"
            }
            $jobs.Add($record)
        }
        foreach ($job in $jobs) {
            $job.Program.WaitForExit()
            [IO.File]::WriteAllText($job.Log, $job.Output.Result + $job.Errors.Result)
            if ($job.Program.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $job.Path -PathType Leaf)) {
                throw "Inno Setup failed. See $($job.Log)"
            }
            if ((Get-Item -LiteralPath $job.Path).VersionInfo.ProductVersion -ne $Version) {
                throw "Installer version differs: $($job.Path)"
            }
        }
        $jobs.Path
    } finally {
        foreach ($job in $jobs) {
            if (-not $job.Program.HasExited) { $job.Program.Kill(); $job.Program.WaitForExit() }
            $job.Program.Dispose()
        }
    }
}
