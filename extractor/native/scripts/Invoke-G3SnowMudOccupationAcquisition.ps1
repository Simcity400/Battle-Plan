[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$RecorderExe,
    [Parameter(Mandatory = $true)][string]$Hoi4Exe,
    [Parameter(Mandatory = $true)][string]$PdxUserDir,
    [Parameter(Mandatory = $true)][string]$CollectorDll,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [Parameter(Mandatory = $true)][string]$ContinueSave,
    [Parameter(Mandatory = $true)][string]$StaticBundleManifest,
    [Parameter(Mandatory = $true)][string]$StaticBundleBindingsDir,
    [Parameter(Mandatory = $true)][string]$ReplayApp,
    [Parameter(Mandatory = $true)][string]$GameDir,
    [Parameter(Mandatory = $true)][string]$SnowEvidenceDir,
    [Parameter(Mandatory = $true)][string]$OccupationEvidenceDir,
    [Parameter(Mandatory = $true)][string]$G3Registry,
    [int]$DurationSeconds = 900,
    [int]$InitialSettleSeconds = 75,
    [int]$ProcessTimeoutSeconds = 240,
    [int]$MaxPulses = 24,
    [int]$HoursPerPulse = 4,
    [int]$PulseTimeoutMilliseconds = 60000,
    [int]$PostActionSettleMilliseconds = 1000,
    [string[]]$EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string]$Python = 'python',
    [switch]$EnableLegacyClientDriving
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $EnableLegacyClientDriving) {
    throw 'This legacy owner launches, inputs to, and controls HOI4. It is disabled by default. Use G3_FIDELITY_RECOUNT/PASSIVE_OBSERVER_WORKFLOW.json with the shared passive observer; only an explicit -EnableLegacyClientDriving opt-in may enter this quarantined path.'
}
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$expectedD04SaveSha256 = 'c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))

function Write-AtomicReplaceBytes([string]$Path, [byte[]]$Bytes) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($fullPath)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporaryPath = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllBytes($temporaryPath, $Bytes)
        if ([IO.File]::Exists($fullPath)) { [IO.File]::Replace($temporaryPath, $fullPath, $null) }
        else { [IO.File]::Move($temporaryPath, $fullPath) }
    } finally {
        if ([IO.File]::Exists($temporaryPath)) { [IO.File]::Delete($temporaryPath) }
    }
}

foreach ($path in @(
    $RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll, $ContinueSave,
    $StaticBundleManifest, $StaticBundleBindingsDir, $ReplayApp, $GameDir, $G3Registry
)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required path does not exist: $path" }
}
if ($DurationSeconds -lt 300 -or $DurationSeconds -gt 3600) { throw 'DurationSeconds must be in 300..3600.' }
if ($InitialSettleSeconds -lt 30 -or $InitialSettleSeconds -gt 300) { throw 'InitialSettleSeconds must be in 30..300.' }
if ($ProcessTimeoutSeconds -lt 30 -or $ProcessTimeoutSeconds -gt 600) { throw 'ProcessTimeoutSeconds must be in 30..600.' }
if ($MaxPulses -lt 2 -or $MaxPulses -gt 96) { throw 'MaxPulses must be in 2..96.' }
if ($HoursPerPulse -lt 1 -or $HoursPerPulse -gt 24) { throw 'HoursPerPulse must be in 1..24.' }
if ($PulseTimeoutMilliseconds -lt 1000 -or $PulseTimeoutMilliseconds -gt 300000) { throw 'PulseTimeoutMilliseconds must be in 1000..300000.' }
if ($PostActionSettleMilliseconds -lt 250 -or $PostActionSettleMilliseconds -gt 10000) { throw 'PostActionSettleMilliseconds must be in 250..10000.' }

$resolvedRecorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedPdxUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedSave = (Resolve-Path -LiteralPath $ContinueSave).Path
$resolvedStaticManifest = (Resolve-Path -LiteralPath $StaticBundleManifest).Path
$resolvedStaticBindings = (Resolve-Path -LiteralPath $StaticBundleBindingsDir).Path
$resolvedReplayApp = (Resolve-Path -LiteralPath $ReplayApp).Path
$resolvedGameDir = (Resolve-Path -LiteralPath $GameDir).Path
$resolvedRegistry = (Resolve-Path -LiteralPath $G3Registry).Path
$resolvedOut = [IO.Path]::GetFullPath($OutDir)
$resolvedSnowEvidence = [IO.Path]::GetFullPath($SnowEvidenceDir)
$resolvedOccupationEvidence = [IO.Path]::GetFullPath($OccupationEvidenceDir)
$repoPrefix = $repoRoot.TrimEnd([char]'\') + [IO.Path]::DirectorySeparatorChar
foreach ($evidencePath in @($resolvedSnowEvidence, $resolvedOccupationEvidence)) {
    if (-not $evidencePath.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "G3 raw observation evidence must be stored inside the repository: $evidencePath"
    }
}

if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) {
    throw 'Hoi4Exe is not the exact a729 authority.'
}
if ((Get-FileHash -LiteralPath $resolvedSave -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedD04SaveSha256) {
    throw 'ContinueSave is not the frozen New D04 Save Deep War identity.'
}
$saveDirectory = [IO.Path]::GetFullPath((Join-Path $resolvedPdxUserDir 'save games')).TrimEnd([char]'\')
if (-not [string]::Equals((Split-Path -Parent $resolvedSave).TrimEnd([char]'\'), $saveDirectory, [StringComparison]::OrdinalIgnoreCase)) {
    throw "ContinueSave must be a direct child of $saveDirectory"
}
$continueCount = @($EarlyLaunchArgument | Where-Object { $_ -ieq '--continuelastsave' }).Count
$gdprIndex = -1
$continueIndex = -1
for ($index = 0; $index -lt $EarlyLaunchArgument.Count; $index++) {
    if ($EarlyLaunchArgument[$index] -ieq '--continuelastsave') { $continueIndex = $index }
    if ($EarlyLaunchArgument[$index] -ieq '--gdpr-compliant') { $gdprIndex = $index }
}
if ($continueCount -ne 1 -or $gdprIndex -lt 0 -or $gdprIndex -lt $continueIndex) {
    throw 'EarlyLaunchArgument must contain exactly one --continuelastsave followed by --gdpr-compliant.'
}
$existing = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
if ($existing.Count -ne 0) { throw "Clean early launch required; close the game PID(s) $(@($existing.Id) -join ',') first." }
foreach ($fresh in @($resolvedOut, $resolvedSnowEvidence, $resolvedOccupationEvidence)) {
    if (Test-Path -LiteralPath $fresh) {
        if (-not (Test-Path -LiteralPath $fresh -PathType Container) -or @(Get-ChildItem -LiteralPath $fresh -Force).Count -ne 0) {
            throw "Create-new evidence directory must be absent or empty: $fresh"
        }
    } else {
        [IO.Directory]::CreateDirectory($fresh) | Out-Null
    }
}

$sendInputScript = Join-Path $repoRoot 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_foreground_send_input.ps1'
$pulseScript = Join-Path $repoRoot 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_one_hour_pulse.ps1'
$readinessScript = Join-Path $repoRoot 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_wait_for_paused_save.ps1'
$monitorScript = Join-Path $repoRoot 'extractor\re_evidence\SNOW_MUD_PREUPLOAD_SEAM\monitor_g3_combined_stream.py'
$combinedScript = Join-Path $repoRoot 'replay_interface\re_evidence\rendering_laws\G3_FIDELITY_RECOUNT\capture_and_admit_combined.py'
foreach ($helper in @($sendInputScript, $pulseScript, $readinessScript, $monitorScript, $combinedScript)) {
    if (-not (Test-Path -LiteralPath $helper -PathType Leaf)) { throw "Missing acquisition helper: $helper" }
}

$sessionId = [guid]::NewGuid().ToString()
$captureId = [guid]::NewGuid().ToString()
$captureManifest = Join-Path $resolvedOut 'native_session_manifest.json'
$rawStream = Join-Path $resolvedOut 'collector_stream.bin'
$actionsDir = Join-Path $resolvedOut 'authenticated-actions'
[IO.Directory]::CreateDirectory($actionsDir) | Out-Null
$f6Receipt = Join-Path $actionsDir 'occupation-resistance-f6.json'
$readinessReceipt = Join-Path $actionsDir 'paused-save-readiness.json'
$monitorState = Join-Path $resolvedOut 'g3-combined-live-monitor.json'
$combinedWork = Join-Path $resolvedOut 'combined-admission-work'
$combinedReceipt = Join-Path $resolvedOut 'combined-admission.json'
$continueGamePath = Join-Path $resolvedPdxUserDir 'continue_game.json'
$continuePriorExists = Test-Path -LiteralPath $continueGamePath -PathType Leaf
$continuePriorBytes = if ($continuePriorExists) { [IO.File]::ReadAllBytes($continueGamePath) } else { $null }
$continueOverridden = $false
$continueObject = if ($continuePriorExists) {
    Get-Content -LiteralPath $continueGamePath -Raw | ConvertFrom-Json
} else {
    [pscustomobject]@{ title = [IO.Path]::GetFileNameWithoutExtension($resolvedSave); desc = ''; date = [DateTime]::UtcNow.ToString('R'); filename = ''; is_remote = $false }
}
$continueObject.filename = [IO.Path]::GetFileName($resolvedSave)
$continueObject.is_remote = $false
$continueReplacement = ($continueObject | ConvertTo-Json -Depth 8) + [Environment]::NewLine

$arguments = @(
    '--early-launch-exe', $resolvedExe, '--pdx-user-dir', $resolvedPdxUserDir,
    '--dll', $resolvedCollector, '--out-dir', $resolvedOut,
    '--session-id', $sessionId, '--capture-id', $captureId,
    '--duration-s', $DurationSeconds, '--sample-ms', '25',
    '--min-game-hours', ([Math]::Max(2, $HoursPerPulse)).ToString(),
    '--stream-format', 'compact', '--stream-compression', 'none', '--profile', 'debug',
    '--scheduled-domain', 'session_header_live', '--scheduled-domains-exact',
    '--static-bundle-manifest', $resolvedStaticManifest,
    '--static-bundle-mod-state', (Join-Path $resolvedStaticBindings 'mod_state.json'),
    '--static-bundle-dlc-state', (Join-Path $resolvedStaticBindings 'dlc_state.json'),
    '--static-bundle-load-order', (Join-Path $resolvedStaticBindings 'load_order.json'),
    '--static-bundle-source-inventory', (Join-Path $resolvedStaticBindings 'source_inventory.json'),
    '--static-bundle-live-resolution-report', (Join-Path $resolvedStaticBindings 'resolution_report.json'),
    '--static-bundle-timeout-ms', '1800000', '--sidecar-log', '--hour-latch'
)
foreach ($value in $EarlyLaunchArgument) { $arguments += @('--early-launch-arg', $value) }

$recorderJob = $null
$pulseReceipts = [Collections.Generic.List[string]]::new()
try {
    $continueOverridden = $true
    Write-AtomicReplaceBytes $continueGamePath ([Text.UTF8Encoding]::new($false).GetBytes($continueReplacement))
    $recorderJob = Start-Job -ScriptBlock {
        param([string]$Recorder, [string[]]$RecorderArguments)
        & $Recorder @RecorderArguments
        if ($LASTEXITCODE -ne 0) { throw "collector_record exited with code $LASTEXITCODE" }
    } -ArgumentList $resolvedRecorder, (,$arguments)

    $deadline = [DateTime]::UtcNow.AddSeconds($ProcessTimeoutSeconds)
    $exactProcess = $null
    while ([DateTime]::UtcNow -lt $deadline -and $null -eq $exactProcess) {
        if ($recorderJob.State -in @('Failed', 'Stopped', 'Disconnected')) { throw 'Recorder terminated before exact-a729 exposed its window.' }
        foreach ($candidate in @(Get-Process -Name the game -ErrorAction SilentlyContinue)) {
            try {
                if ([string]::Equals($candidate.MainModule.FileName, $resolvedExe, [StringComparison]::OrdinalIgnoreCase) -and
                    $candidate.MainWindowHandle -ne [IntPtr]::Zero -and
                    (Get-FileHash -LiteralPath $candidate.MainModule.FileName -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $expectedExeSha256) {
                    $exactProcess = $candidate
                    break
                }
            } catch { }
        }
        if ($null -eq $exactProcess) { Start-Sleep -Milliseconds 500 }
    }
    if ($null -eq $exactProcess) { throw 'Exact-a729 did not expose an authenticated main window in time.' }
    Start-Sleep -Seconds $InitialSettleSeconds
    if ($recorderJob.State -in @('Failed', 'Stopped', 'Disconnected')) { throw 'Recorder terminated during initial settlement.' }

    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $readinessScript -ProcessId $exactProcess.Id -ReceiptPath $readinessReceipt -TimeoutSeconds $ProcessTimeoutSeconds
    if ($LASTEXITCODE -ne 0) { throw 'Exact-a729 paused continued save never reached authenticated readiness.' }

    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $sendInputScript -ProcessId $exactProcess.Id -Action OccupationResistanceF6 -ReceiptPath $f6Receipt
    if ($LASTEXITCODE -ne 0) { throw 'Authenticated occupation F6 action failed.' }
    Start-Sleep -Milliseconds $PostActionSettleMilliseconds

    $ready = $false
    for ($index = 1; $index -le $MaxPulses -and -not $ready; $index++) {
        if ($recorderJob.State -in @('Failed', 'Stopped', 'Disconnected', 'Completed')) { throw 'Recorder ended before combined semantic targets were secured.' }
        $pulseReceipt = Join-Path $actionsDir ('clock-pulse-{0:D3}.json' -f $index)
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $pulseScript -ProcessId $exactProcess.Id -ReceiptPath $pulseReceipt -TargetHours $HoursPerPulse -TimeoutMilliseconds $PulseTimeoutMilliseconds
        if ($LASTEXITCODE -ne 0) { throw "Authenticated clock pulse $index failed; game restoration receipt retained." }
        $pulseReceipts.Add($pulseReceipt)
        Start-Sleep -Milliseconds $PostActionSettleMilliseconds
        if (Test-Path -LiteralPath $rawStream -PathType Leaf) {
            & $Python $monitorScript --stream $rawStream --state $monitorState
            if ($LASTEXITCODE -ne 0) { throw "Combined stream monitor rejected pulse $index." }
            $ready = (Get-Content -LiteralPath $monitorState -Raw | ConvertFrom-Json).ready -eq $true
        }
    }
    if (-not $ready) { throw "Combined semantic targets were not secured within $MaxPulses bounded pulses." }

    $completedJob = Wait-Job -Job $recorderJob -Timeout ($DurationSeconds + 600)
    if ($null -eq $completedJob) { throw "Recorder did not terminate within the bounded $($DurationSeconds + 600)-second wait." }
    $recorderState = $recorderJob.State
    Receive-Job -Job $recorderJob -ErrorAction Continue | ForEach-Object { Write-Verbose ([string]$_) }
    if ($recorderState -ne 'Completed') { throw "Recorder ended in state $recorderState" }
} finally {
    # continue_game.json is needed only to select the launched save. Restore it
    # before any recorder wait so a hung capture cannot retain the override.
    if ($continueOverridden) {
        if ($continuePriorExists) { Write-AtomicReplaceBytes $continueGamePath $continuePriorBytes }
        elseif (Test-Path -LiteralPath $continueGamePath) { Remove-Item -LiteralPath $continueGamePath -Force }
        $continueOverridden = $false
    }
    if ($null -ne $recorderJob) {
        if ($recorderJob.State -notin @('Completed', 'Failed', 'Stopped', 'Disconnected')) {
            $terminalJob = Wait-Job -Job $recorderJob -Timeout 30
            if ($null -eq $terminalJob) {
                Stop-Job -Job $recorderJob -ErrorAction SilentlyContinue
            }
        }
        Receive-Job -Job $recorderJob -ErrorAction SilentlyContinue | Out-Null
        Remove-Job -Job $recorderJob -Force -ErrorAction SilentlyContinue
    }
}

if (-not (Test-Path -LiteralPath $captureManifest -PathType Leaf)) { throw 'Recorder did not produce native_session_manifest.json.' }
$manifest = Get-Content -LiteralPath $captureManifest -Raw | ConvertFrom-Json
$replayName = [string]$manifest.replay_file
if ([string]::IsNullOrWhiteSpace($replayName) -or [IO.Path]::GetFileName($replayName) -cne $replayName) { throw 'Manifest replay_file is absent or not a direct filename.' }
$recording = [IO.Path]::GetFullPath((Join-Path $resolvedOut $replayName))
if (-not (Test-Path -LiteralPath $recording -PathType Leaf)) { throw "Finalized recording missing: $recording" }

$combinedArguments = @(
    $combinedScript, '--recording', $recording, '--capture-manifest', $captureManifest,
    '--readiness-receipt', $readinessReceipt, '--f6-receipt', $f6Receipt, '--monitor-state', $monitorState,
    '--replay-app', $resolvedReplayApp, '--game', $resolvedGameDir,
    '--work-dir', $combinedWork, '--snow-evidence-dir', $resolvedSnowEvidence,
    '--occupation-evidence-dir', $resolvedOccupationEvidence,
    '--registry', $resolvedRegistry, '--output', $combinedReceipt
)
foreach ($receipt in $pulseReceipts) { $combinedArguments += @('--pulse-receipt', $receipt) }
& $Python @combinedArguments
if ($LASTEXITCODE -ne 0) { throw "Combined post-capture admission failed; inspect $combinedWork" }
Write-Output "Combined SnowMud and occupation admission accepted: $combinedReceipt"
