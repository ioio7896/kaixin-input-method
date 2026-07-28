param(
    [string]$BinaryRoot = $PSScriptRoot,
    [string]$Subject = 'CN=开心输入法 Dev',
    [switch]$TrustCurrentUser
)

$ErrorActionPreference = 'Stop'

function Get-SignToolPath {
    $candidates = Get-ChildItem -Path 'C:\Program Files (x86)\Windows Kits\10\bin' -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
        Sort-Object FullName -Descending
    if (-not $candidates) {
        throw 'signtool.exe was not found.'
    }
    return $candidates[0].FullName
}

function Get-OrCreateCertificate {
    param([string]$CertificateSubject)

    $existing = Get-ChildItem Cert:\CurrentUser\My |
        Where-Object {
            $_.Subject -eq $CertificateSubject -and
            $_.HasPrivateKey -and
            ($_.EnhancedKeyUsageList | Where-Object { $_.FriendlyName -eq 'Code Signing' })
        } |
        Sort-Object NotAfter -Descending |
        Select-Object -First 1

    if ($existing) {
        return $existing
    }

    return New-SelfSignedCertificate -Type CodeSigningCert -Subject $CertificateSubject -CertStoreLocation 'Cert:\CurrentUser\My'
}

function Import-CertificateIfMissing {
    param(
        [string]$StorePath,
        [string]$CerPath,
        [string]$Thumbprint
    )

    $existing = Get-ChildItem $StorePath | Where-Object { $_.Thumbprint -eq $Thumbprint } | Select-Object -First 1
    if (-not $existing) {
        Import-Certificate -FilePath $CerPath -CertStoreLocation $StorePath | Out-Null
    }
}

$BinaryRoot = [System.IO.Path]::GetFullPath($BinaryRoot)
$targets = @(
    (Join-Path $BinaryRoot 'srf_tsf_tip.dll')
) | Where-Object { Test-Path -LiteralPath $_ }

if (-not $targets) {
    throw "No signable DLLs were found in $BinaryRoot"
}

$cert = Get-OrCreateCertificate -CertificateSubject $Subject
if ($TrustCurrentUser) {
    $cerPath = Join-Path $env:TEMP ("srf-ime-dev-{0}.cer" -f $cert.Thumbprint)
    Export-Certificate -Cert $cert -FilePath $cerPath -Force | Out-Null
    try {
        Import-CertificateIfMissing -StorePath 'Cert:\CurrentUser\TrustedPublisher' -CerPath $cerPath -Thumbprint $cert.Thumbprint
        Import-CertificateIfMissing -StorePath 'Cert:\CurrentUser\Root' -CerPath $cerPath -Thumbprint $cert.Thumbprint
    } finally {
        Remove-Item -LiteralPath $cerPath -Force -ErrorAction SilentlyContinue
    }
}

$signtool = Get-SignToolPath
foreach ($target in $targets) {
    & $signtool sign /fd SHA256 /sha1 $cert.Thumbprint $target
    if ($LASTEXITCODE -ne 0) {
        throw "signtool failed for $target with exit code $LASTEXITCODE"
    }
}

Write-Output "Signed binaries with certificate: $($cert.Subject)"
Write-Output "Thumbprint: $($cert.Thumbprint)"
