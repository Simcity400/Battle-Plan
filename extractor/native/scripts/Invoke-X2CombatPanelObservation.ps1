[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][int]$Hoi4Pid,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [Parameter(Mandatory = $true)][string]$PreparedProductModsetDir,
    [Parameter(Mandatory = $true)][string]$SpeedEvidenceTranscriptPath,
    [Parameter(Mandatory = $true)][string]$SpeedEvidenceTranscriptSha256,
    [Parameter(Mandatory = $true)][string]$SpeedEvidenceTranscriptFileSha256,
    [Parameter(Mandatory = $true)][string]$SpeedEvidenceSessionId,
    [Parameter(Mandatory = $true)][string]$SpeedEvidenceNonce,
    [Parameter(Mandatory = $true)][string]$SpeedEvidenceNonceLedgerRoot,
    [Parameter(Mandatory = $true)][string]$SpeedStaticDisasmSha256,
    [double]$DurationSeconds = 300,
    [int]$MinGameHours = 2,
    [string]$PythonExe = "python.exe"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$capture = Join-Path $PSScriptRoot "Start-Hoi4ProductionCaptureSafe.ps1"
$verifier = Join-Path $PSScriptRoot "verify_x2_combat_panel_observation.py"
foreach ($path in @($capture, $verifier, $PreparedProductModsetDir, $SpeedEvidenceTranscriptPath, $SpeedEvidenceNonceLedgerRoot)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required X2 panel probe input does not exist: $path"
    }
}
if (Test-Path -LiteralPath $OutDir) {
    if (-not (Test-Path -LiteralPath $OutDir -PathType Container) -or
        @(Get-ChildItem -LiteralPath $OutDir -Force).Count -ne 0) {
        throw "OutDir must be absent or empty: $OutDir"
    }
}

Write-Host "During this bounded capture, select one active land battle and keep its combat detail panel open long enough for both sides to refresh."
& $capture `
    -Hoi4Pid $Hoi4Pid `
    -DurationSeconds $DurationSeconds `
    -MinGameHours $MinGameHours `
    -OutDir $OutDir `
    -SpeedEvidenceTranscriptPath $SpeedEvidenceTranscriptPath `
    -SpeedEvidenceTranscriptSha256 $SpeedEvidenceTranscriptSha256 `
    -SpeedEvidenceTranscriptFileSha256 $SpeedEvidenceTranscriptFileSha256 `
    -SpeedEvidenceSessionId $SpeedEvidenceSessionId `
    -SpeedEvidenceNonce $SpeedEvidenceNonce `
    -SpeedEvidenceNonceLedgerRoot $SpeedEvidenceNonceLedgerRoot `
    -SpeedStaticDisasmSha256 $SpeedStaticDisasmSha256 `
    -PreparedProductModsetDir $PreparedProductModsetDir `
    -PythonExe $PythonExe `
    -SkipBuild `
    -AllowExistingEmptyOutDir

$snapshots = Join-Path $OutDir "acceptance_snapshots"
$receipt = Join-Path $OutDir "x2_combat_panel_observation_verification.json"
& $PythonExe $verifier --snapshots $snapshots --out $receipt
if ($LASTEXITCODE -ne 0) {
    throw "X2 combat-panel observation was not admitted; inspect $receipt"
}
Write-Output "X2 combat-panel observation accepted: $receipt"
