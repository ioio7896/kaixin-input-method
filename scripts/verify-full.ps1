param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$RemainingArgs
)

$ErrorActionPreference = "Stop"
& "$PSScriptRoot\verify.ps1" @RemainingArgs
exit $LASTEXITCODE
