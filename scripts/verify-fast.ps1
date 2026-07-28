param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$RemainingArgs
)

$ErrorActionPreference = "Stop"
& "$PSScriptRoot\verify.ps1" -Fast @RemainingArgs
exit $LASTEXITCODE
