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
    [string]$PrebuiltArtifactAttestationPath = "",
    [string]$PythonExe = "python.exe"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path

$capture = Join-Path $PSScriptRoot "Start-Hoi4ProductionCaptureSafe.ps1"
$verifier = Join-Path $PSScriptRoot "verify_x2_combat_panel_observation.py"
$actionDriver = Join-Path $PSScriptRoot "Invoke-X2AutonomousCombatPanelActions.ps1"
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$actionProfile = Join-Path $repo 'extractor\re_evidence\X2_LAND_COMBAT_RESIDUALS\AUTONOMOUS_PANEL_ACTION_PROFILE_1920X1080.json'
$ensurePaused = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\CAP6\authenticated_ensure_paused.ps1'
$attestationHelper = Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1'
foreach ($path in @(
    $capture, $verifier, $actionDriver, $actionProfile, $ensurePaused, $attestationHelper, $PreparedProductModsetDir,
    $SpeedEvidenceTranscriptPath, $SpeedEvidenceNonceLedgerRoot
)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required X2 panel probe input does not exist: $path"
    }
}
if ($MinGameHours -lt 2 -or $MinGameHours -gt 24) {
    throw 'MinGameHours must be in 2..24 for the frozen autonomous X2 action.'
}
if (Test-Path -LiteralPath $OutDir) {
    if (-not (Test-Path -LiteralPath $OutDir -PathType Container) -or
        @(Get-ChildItem -LiteralPath $OutDir -Force).Count -ne 0) {
        throw "OutDir must be absent or empty: $OutDir"
    }
}

if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) {
    $PrebuiltArtifactAttestationPath = Join-Path $repo '.cargo-target\release\hoi4-production-prebuilt-artifacts.json'
}
. $attestationHelper
$prebuilt = Test-ProductionArtifactAttestation `
    -RepoRoot $repo `
    -ReceiptPath $PrebuiltArtifactAttestationPath `
    -ExpectedDriverPath $capture
$resolvedAttestation = (Resolve-Path -LiteralPath $prebuilt.receipt_path).Path
$attestationSha256 = (Get-FileHash -LiteralPath $resolvedAttestation -Algorithm SHA256).Hash.ToLowerInvariant()
$collectorSha256 = [string]$prebuilt.artifact_sha256.collector_dll
if ($collectorSha256 -cnotmatch '^[0-9a-f]{64}$') {
    throw 'Central Release attestation lacks the collector.dll SHA-256.'
}

$sessionId = $SpeedEvidenceSessionId
$captureId = "capture-$sessionId"
$resolvedOut = [IO.Path]::GetFullPath($OutDir)
$actionTemporaryRoot = Join-Path ([IO.Path]::GetTempPath()) ("hoi4_x2_actions_" + $captureId)
$actionReadyReceipt = Join-Path $actionTemporaryRoot 'x2_autonomous_combat_panel_ready.json'
$failurePauseReceipt = Join-Path $actionTemporaryRoot 'x2_failure_cleanup_pause.json'
$captureManifest = Join-Path $resolvedOut 'native_session_manifest.json'
$actionReceipt = Join-Path $resolvedOut 'x2_autonomous_combat_panel_actions.json'
if (Test-Path -LiteralPath $actionTemporaryRoot) {
    throw "Refusing to reuse an X2 action staging directory: $actionTemporaryRoot"
}
[IO.Directory]::CreateDirectory($actionTemporaryRoot) | Out-Null
$actionArguments = @(
    '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
    '-File', ('"' + $actionDriver + '"'),
    '-ProcessId', [string]$Hoi4Pid,
    '-ReceiptPath', ('"' + $actionReceipt + '"'),
    '-SessionId', $sessionId,
    '-CaptureId', $captureId,
    '-ExpectedCollectorSha256', $collectorSha256,
    '-PrebuiltAttestationSha256', $attestationSha256,
    '-ProfilePath', ('"' + $actionProfile + '"'),
    '-ReadyPath', ('"' + $actionReadyReceipt + '"'),
    '-CaptureManifestPath', ('"' + $captureManifest + '"'),
    '-GameHoursToHold', [string]$MinGameHours
)
$actionWorker = Start-Process -FilePath 'powershell.exe' -ArgumentList $actionArguments `
    -PassThru -WindowStyle Hidden
$readyDeadline = [DateTime]::UtcNow.AddSeconds(30)
while (-not (Test-Path -LiteralPath $actionReadyReceipt -PathType Leaf) -and
    -not $actionWorker.HasExited -and [DateTime]::UtcNow -lt $readyDeadline) {
    Start-Sleep -Milliseconds 100
    $actionWorker.Refresh()
}
if (-not (Test-Path -LiteralPath $actionReadyReceipt -PathType Leaf)) {
    if (-not $actionWorker.HasExited) { Stop-Process -Id $actionWorker.Id -Force -ErrorAction SilentlyContinue }
    if (Get-Process -Id $Hoi4Pid -ErrorAction SilentlyContinue) {
        & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $ensurePaused `
            -ProcessId $Hoi4Pid -ReceiptPath $failurePauseReceipt
    }
    throw 'Autonomous X2 action driver did not establish exact speed-5 unpaused readiness.'
}
$ready = Get-Content -LiteralPath $actionReadyReceipt -Raw | ConvertFrom-Json
if ([string]$ready.schema -cne 'hoi4.x2.autonomous_combat_panel_ready.v1' -or
    [string]$ready.status -cne 'READY_SPEED5_UNPAUSED' -or
    [int]$ready.process_id -ne $Hoi4Pid -or
    [string]$ready.session_id -cne $sessionId -or
    [string]$ready.capture_id -cne $captureId -or
    [string]$ready.executable_sha256 -cne '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad' -or
    [int]$ready.game_speed_raw -ne 4 -or $ready.paused -isnot [bool] -or [bool]$ready.paused) {
    if (-not $actionWorker.HasExited) { Stop-Process -Id $actionWorker.Id -Force -ErrorAction SilentlyContinue }
    if (Get-Process -Id $Hoi4Pid -ErrorAction SilentlyContinue) {
        & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $ensurePaused `
            -ProcessId $Hoi4Pid -ReceiptPath $failurePauseReceipt
    }
    throw 'Autonomous X2 readiness receipt is malformed.'
}
$captureCompleted = $false
try {
    & $capture `
        -Hoi4Pid $Hoi4Pid `
        -DurationSeconds $DurationSeconds `
        -MinGameHours $MinGameHours `
        -OutDir $OutDir `
        -SessionId $sessionId `
        -CaptureId $captureId `
        -SpeedEvidenceTranscriptPath $SpeedEvidenceTranscriptPath `
        -SpeedEvidenceTranscriptSha256 $SpeedEvidenceTranscriptSha256 `
        -SpeedEvidenceTranscriptFileSha256 $SpeedEvidenceTranscriptFileSha256 `
        -SpeedEvidenceSessionId $SpeedEvidenceSessionId `
        -SpeedEvidenceNonce $SpeedEvidenceNonce `
        -SpeedEvidenceNonceLedgerRoot $SpeedEvidenceNonceLedgerRoot `
        -SpeedStaticDisasmSha256 $SpeedStaticDisasmSha256 `
        -PreparedProductModsetDir $PreparedProductModsetDir `
        -PrebuiltArtifactAttestationPath $resolvedAttestation `
        -PythonExe $PythonExe `
        -SkipBuild `
        -AllowExistingEmptyOutDir
    $captureCompleted = $true
} finally {
    if (-not $captureCompleted -and -not $actionWorker.HasExited) {
        Stop-Process -Id $actionWorker.Id -Force -ErrorAction SilentlyContinue
    }
    if (-not $captureCompleted -and (Get-Process -Id $Hoi4Pid -ErrorAction SilentlyContinue)) {
        & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $ensurePaused `
            -ProcessId $Hoi4Pid -ReceiptPath $failurePauseReceipt
    }
}
if (-not $actionWorker.WaitForExit(30000)) {
    Stop-Process -Id $actionWorker.Id -Force -ErrorAction SilentlyContinue
    throw 'Autonomous X2 action driver did not finish before the capture finalization deadline.'
}
$actionWorker.Refresh()
if ($actionWorker.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $actionReceipt -PathType Leaf)) {
    throw 'Autonomous X2 action driver failed or did not emit its immutable receipt.'
}
$readyReceipt = Join-Path $OutDir 'x2_autonomous_combat_panel_ready.json'
if (Test-Path -LiteralPath $readyReceipt) { throw "Refusing to replace X2 readiness receipt: $readyReceipt" }
$readyBytes = [IO.File]::ReadAllBytes($actionReadyReceipt)
$readyTemporary = Join-Path $resolvedOut ([IO.Path]::GetRandomFileName() + '.tmp')
try {
    [IO.File]::WriteAllBytes($readyTemporary, $readyBytes)
    $readyStream = [IO.File]::Open($readyTemporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try { $readyStream.Flush($true) } finally { $readyStream.Dispose() }
    if ([IO.File]::Exists($readyReceipt)) { throw "Refusing to replace X2 readiness receipt: $readyReceipt" }
    [IO.File]::Move($readyTemporary, $readyReceipt)
} finally {
    if ([IO.File]::Exists($readyTemporary)) { [IO.File]::Delete($readyTemporary) }
}
if ((Get-FileHash -LiteralPath $readyReceipt -Algorithm SHA256).Hash.ToLowerInvariant() -cne
    (Get-FileHash -LiteralPath $actionReadyReceipt -Algorithm SHA256).Hash.ToLowerInvariant()) {
    throw 'Preserved X2 readiness receipt differs from the authenticated staging receipt.'
}
[IO.File]::Delete($actionReadyReceipt)
Remove-Item -LiteralPath $actionTemporaryRoot -Force

$snapshots = Join-Path $OutDir "acceptance_snapshots"
$receipt = Join-Path $OutDir "x2_combat_panel_observation_verification.json"
& $PythonExe $verifier `
    --snapshots $snapshots `
    --capture-manifest $captureManifest `
    --action-receipt $actionReceipt `
    --readiness-receipt $readyReceipt `
    --action-profile $actionProfile `
    --prebuilt-attestation $resolvedAttestation `
    --out $receipt
if ($LASTEXITCODE -ne 0) {
    throw "X2 combat-panel observation was not admitted; inspect $receipt"
}
Write-Output "Autonomous X2 combat-panel observation accepted: $receipt"
