param(
    [switch]$Sign,
    [switch]$SignRequired,
    [switch]$SkipTests,
    [switch]$UserInstaller,
    [switch]$Check,
    [switch]$Help
)

$ErrorActionPreference = 'Stop'
$releaseArgs = @($args)
if ($Sign) { $releaseArgs += '--sign' }
if ($SignRequired) { $releaseArgs += '--sign-required' }
if ($SkipTests) { $releaseArgs += '--skip-tests' }
if ($UserInstaller) { $releaseArgs += '--user-installer' }
if ($Check) { $releaseArgs += '--check' }
if ($Help) { $releaseArgs += '--help' }

Push-Location -LiteralPath (Join-Path $PSScriptRoot '..')
try {
    & cargo xtask release @releaseArgs
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
