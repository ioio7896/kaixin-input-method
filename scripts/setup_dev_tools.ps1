param()
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$toolsDir = Join-Path $repo 'dist\dev-tools'
New-Item -ItemType Directory -Force -Path $toolsDir | Out-Null
& python -m pip install --disable-pip-version-check --target (Join-Path $toolsDir 'python') -r (Join-Path $PSScriptRoot 'requirements-dev.txt')
if ($LASTEXITCODE -ne 0) { throw 'Unable to install Python development tools' }
$moduleDir = Join-Path $toolsDir 'PSScriptAnalyzer\1.24.0'
if (-not (Test-Path -LiteralPath (Join-Path $moduleDir 'PSScriptAnalyzer.psd1'))) {
    $archive = Join-Path $toolsDir 'PSScriptAnalyzer.1.24.0.zip'
    Invoke-WebRequest -Uri 'https://www.powershellgallery.com/api/v2/package/PSScriptAnalyzer/1.24.0' -OutFile $archive
    Expand-Archive -LiteralPath $archive -DestinationPath $moduleDir -Force
}
Write-Output 'Development tools are ready in dist/dev-tools.'
