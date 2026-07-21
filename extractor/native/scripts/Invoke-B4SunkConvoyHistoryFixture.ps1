[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Save,

    [Parameter(Mandatory = $true)]
    [string]$EventsOracle,

    [Parameter(Mandatory = $true)]
    [string]$ProductionSnapshot,

    [Parameter(Mandatory = $true)]
    [string]$OutDir,

    [string]$PythonExe = "python"
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$packet = Join-Path $repo "extractor\re_evidence\B4_B5_B7_OFFLINE_ORACLES"
$captureTool = Join-Path $packet "capture_b4_sunk_convoy_fixture.py"
$verifyTool = Join-Path $packet "verify_b4_sunk_convoy_fixture.py"
$savePath = (Resolve-Path -LiteralPath $Save).Path
$oraclePath = (Resolve-Path -LiteralPath $EventsOracle).Path
$snapshotPath = (Resolve-Path -LiteralPath $ProductionSnapshot).Path

New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
$output = (Resolve-Path -LiteralPath $OutDir).Path
$capture = Join-Path $output "b4_sunk_convoy_raw_debug.json"
$report = Join-Path $output "b4_sunk_convoy_fixture_regrade.json"
if ((Test-Path -LiteralPath $capture) -or (Test-Path -LiteralPath $report)) {
    throw "Refusing to overwrite an existing B4 evidence bundle in $output"
}

& $PythonExe $captureTool `
    --save $savePath `
    --oracle $oraclePath `
    --production-snapshot $snapshotPath `
    --output $capture `
    --ack-read-only-paused-d04
if ($LASTEXITCODE -ne 0) {
    throw "B4 raw owner/vector acquisition failed"
}

& $PythonExe $verifyTool `
    --capture $capture `
    --save $savePath `
    --oracle $oraclePath `
    --production-snapshot $snapshotPath `
    --output $report
if ($LASTEXITCODE -ne 0) {
    throw "B4 independent regrade failed; raw evidence was preserved in $output"
}

Write-Host "B4 exact 441-row paused fixture passed: $report"
Write-Host "Activation remains false until root review pins the report hash and centrally validates."
