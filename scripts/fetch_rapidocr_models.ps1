# SPDX-License-Identifier: Apache-2.0
[CmdletBinding()]
param(
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$modelDir = Join-Path $repoRoot 'RapidOCR-3.9.0\python\rapidocr\models'
$models = @(
    @{
        Name = 'PP-OCRv6_det_medium.onnx'
        Url = 'https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.2/onnx/PP-OCRv6/det/PP-OCRv6_det_medium.onnx'
        Sha256 = '92078b7355007ccfffcd4c8cd441a3afd4538904d06881b29a155e1e679907c2'
    },
    @{
        Name = 'PP-OCRv6_det_small.onnx'
        Url = 'https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.2/onnx/PP-OCRv6/det/PP-OCRv6_det_small.onnx'
        Sha256 = '090f04abcd9d9a7498bc4ebf677e4cb9bdce1fe4197ddb7e529f1ef44e1ff94f'
    },
    @{
        Name = 'PP-OCRv6_rec_medium.onnx'
        Url = 'https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.2/onnx/PP-OCRv6/rec/PP-OCRv6_rec_medium.onnx'
        Sha256 = 'eef444829dbbe18d7fea59a3f6eb75647518d2b3a9568d27c92e42940204894b'
    }
)

New-Item -ItemType Directory -Path $modelDir -Force | Out-Null
foreach ($model in $models) {
    $destination = Join-Path $modelDir $model.Name
    if ((-not $Force) -and (Test-Path -LiteralPath $destination)) {
        $actual = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -eq $model.Sha256) {
            Write-Host "verified: $($model.Name)"
            continue
        }
    }
    $temporary = "$destination.download"
    Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
    Invoke-WebRequest -Uri $model.Url -OutFile $temporary
    $actual = (Get-FileHash -LiteralPath $temporary -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $model.Sha256) {
        Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
        throw "SHA-256 mismatch for $($model.Name): expected $($model.Sha256), got $actual"
    }
    Move-Item -LiteralPath $temporary -Destination $destination -Force
    Write-Host "downloaded: $($model.Name)"
}
