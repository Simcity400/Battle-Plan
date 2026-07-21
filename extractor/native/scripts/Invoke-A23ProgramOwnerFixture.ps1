param(
    [Parameter(Mandatory = $true)]
    [int]$Pid,
    [Parameter(Mandatory = $true)]
    [string]$SaveProjectsJson,
    [Parameter(Mandatory = $true)]
    [int]$ExpectedGameHourRaw,
    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$probe = Join-Path $repo "extractor\native\project_progress_live_lane\tools\program_owner_probe.py"
$verify = Join-Path $repo "extractor\native\project_progress_live_lane\tools\verify_program_owner_receipt.py"
$out = [System.IO.Path]::GetFullPath($OutputDirectory)
[System.IO.Directory]::CreateDirectory($out) | Out-Null
$receipt = Join-Path $out "a23_program_owner_receipt.json"
$verification = Join-Path $out "a23_program_owner_verification.json"

& python $probe `
    --pid $Pid `
    --save-projects-json ([System.IO.Path]::GetFullPath($SaveProjectsJson)) `
    --expected-game-hour-raw $ExpectedGameHourRaw `
    --out $receipt
if ($LASTEXITCODE -ne 0) {
    throw "A23 owner probe failed with exit code $LASTEXITCODE"
}

& python $verify $receipt --out $verification
if ($LASTEXITCODE -ne 0) {
    throw "A23 owner receipt verification failed with exit code $LASTEXITCODE"
}

Write-Output $verification
