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
    [string]$PrebuiltArtifactAttestationPath = '',
    [int]$DurationSeconds = 600,
    [int]$SampleMilliseconds = 25,
    [int]$GameLoadTimeoutSeconds = 300,
    [int]$MinimumGameHours = 8,
    [string]$ExpectedContinueSaveSha256 = 'c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735',
    [string]$Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$attestationHelper = Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1'
$productionDriver = Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1'
$actionDriver = Join-Path $PSScriptRoot 'Invoke-X2AutonomousCombatPanelActions.ps1'
$regrader = Join-Path $PSScriptRoot 'verify_x2_loss_width_acquisition.py'
$snapshotExtractor = Join-Path $PSScriptRoot 'extract_f6_replay_snapshots.py'
$profile = Join-Path $repo 'extractor\re_evidence\X2_LAND_COMBAT_RESIDUALS\AUTONOMOUS_PANEL_ACTION_PROFILE_1920X1080.json'
$waitPaused = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_wait_for_paused_save.ps1'
$staticBundleVerifier = Join-Path $repo 'extractor\game_defs\static_bundle.py'
$productModsetVerifier = Join-Path $repo 'extractor\game_defs\verify_product_modset_capture_ready.py'
$bindingNames = @('mod_state.json', 'dlc_state.json', 'load_order.json', 'source_inventory.json', 'resolution_report.json')

if ($DurationSeconds -lt 300 -or $DurationSeconds -gt 1800) { throw 'DurationSeconds must be in 300..1800.' }
if ($SampleMilliseconds -lt 1 -or $SampleMilliseconds -gt 1000) { throw 'SampleMilliseconds must be in 1..1000.' }
if ($GameLoadTimeoutSeconds -lt 30 -or $GameLoadTimeoutSeconds -gt 1800) { throw 'GameLoadTimeoutSeconds must be in 30..1800.' }
if ($MinimumGameHours -lt 2 -or $MinimumGameHours -gt 24) { throw 'MinimumGameHours must be in 2..24.' }
if ($ExpectedContinueSaveSha256 -cnotmatch '^[0-9a-f]{64}$') { throw 'ExpectedContinueSaveSha256 must be lowercase SHA-256.' }

foreach ($path in @(
    $RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll, $ContinueSave, $StaticBundleManifest,
    $StaticBundleBindingsDir, $attestationHelper, $productionDriver, $actionDriver, $regrader,
    $snapshotExtractor, $profile, $waitPaused, $staticBundleVerifier, $productModsetVerifier
)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required X2 acquisition input is absent: $path" }
}
foreach ($leaf in $bindingNames) {
    if (-not (Test-Path -LiteralPath (Join-Path $StaticBundleBindingsDir $leaf) -PathType Leaf)) {
        throw "Prepared product-modset binding is absent: $leaf"
    }
}
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
    throw 'H-X2-LOSS/WIDTH requires a clean exact-save launch; close every existing hoi4.exe first.'
}

$resolvedRecorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedUser = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedSave = (Resolve-Path -LiteralPath $ContinueSave).Path
$resolvedManifest = (Resolve-Path -LiteralPath $StaticBundleManifest).Path
$resolvedBindings = (Resolve-Path -LiteralPath $StaticBundleBindingsDir).Path
$resolvedPython = (Get-Command $Python -CommandType Application -ErrorAction Stop).Source
$resolvedOut = [IO.Path]::GetFullPath($OutDir)

function Test-IsWithinPath([string]$Candidate, [string]$Root) {
    $candidateFull = [IO.Path]::GetFullPath($Candidate).TrimEnd([char]'\')
    $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd([char]'\')
    return $candidateFull.Equals($rootFull, [StringComparison]::OrdinalIgnoreCase) -or
        $candidateFull.StartsWith($rootFull + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)
}

function Write-AtomicBytes([string]$Path, [byte[]]$Bytes, [switch]$AllowReplace) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($fullPath)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllBytes($temporary, $Bytes)
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($fullPath)) {
            if (-not $AllowReplace) { throw "Refusing to replace immutable receipt: $fullPath" }
            [IO.File]::Replace($temporary, $fullPath, $null, $true)
        } else {
            [IO.File]::Move($temporary, $fullPath)
        }
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

function Write-AtomicUtf8([string]$Path, [string]$Text, [switch]$AllowReplace) {
    Write-AtomicBytes $Path ([Text.UTF8Encoding]::new($false).GetBytes($Text)) -AllowReplace:$AllowReplace
}

if ((Test-IsWithinPath $resolvedOut $repo) -or (Test-IsWithinPath $resolvedOut $resolvedUser)) {
    throw 'OutDir must be outside both source authority and the game user directory.'
}
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty: $resolvedOut"
    }
} else {
    [IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
}

if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) {
    throw 'Hoi4Exe is not exact-a729 authority.'
}
if ((Get-FileHash -LiteralPath $resolvedSave -Algorithm SHA256).Hash.ToLowerInvariant() -cne $ExpectedContinueSaveSha256) {
    throw 'ContinueSave differs from the frozen exact D04 save identity.'
}
$saveDirectory = [IO.Path]::GetFullPath((Join-Path $resolvedUser 'save games')).TrimEnd([char]'\')
if (-not [IO.Path]::GetFullPath((Split-Path -Parent $resolvedSave)).TrimEnd([char]'\').Equals($saveDirectory, [StringComparison]::OrdinalIgnoreCase)) {
    throw "ContinueSave must be a direct child of $saveDirectory"
}

. $attestationHelper
if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) {
    $PrebuiltArtifactAttestationPath = Join-Path $repo '.cargo-target\release\hoi4-production-prebuilt-artifacts.json'
}
$prebuilt = Test-ProductionArtifactAttestation -RepoRoot $repo -ReceiptPath $PrebuiltArtifactAttestationPath -ExpectedDriverPath $productionDriver
$resolvedAttestation = (Resolve-Path -LiteralPath $prebuilt.receipt_path).Path
if (-not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.recorder_host).Equals($resolvedRecorder, [StringComparison]::OrdinalIgnoreCase) -or
    -not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.collector_dll).Equals($resolvedCollector, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'X2 acquisition artifacts differ from the central Release attestation.'
}
$collectorSha256 = [string]$prebuilt.artifact_sha256.collector_dll
$attestationSha256 = (Get-FileHash -LiteralPath $resolvedAttestation -Algorithm SHA256).Hash.ToLowerInvariant()

$preparedRoot = [IO.Path]::GetFullPath((Split-Path -Parent $resolvedBindings))
$preparedAdmissionPath = Join-Path $resolvedOut 'prepared_product_modset_admission.json'
$preparedAdmissionStage = Join-Path $resolvedOut ([IO.Path]::GetRandomFileName() + '.tmp')
try {
    & $resolvedPython $productModsetVerifier --exe $resolvedExe --prepared $preparedRoot --out $preparedAdmissionStage
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $preparedAdmissionStage -PathType Leaf)) {
        throw 'Prepared product-modset admission failed.'
    }
    $admissionStream = [IO.File]::Open($preparedAdmissionStage, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try { $admissionStream.Flush($true) } finally { $admissionStream.Dispose() }
    if ([IO.File]::Exists($preparedAdmissionPath)) { throw "Refusing to replace prepared admission: $preparedAdmissionPath" }
    [IO.File]::Move($preparedAdmissionStage, $preparedAdmissionPath)
} finally {
    if ([IO.File]::Exists($preparedAdmissionStage)) { [IO.File]::Delete($preparedAdmissionStage) }
}
$preparedAdmission = Get-Content -LiteralPath $preparedAdmissionPath -Raw | ConvertFrom-Json
if ([string]$preparedAdmission.verdict -cne 'passed' -or
    -not [IO.Path]::GetFullPath([string]$preparedAdmission.paths.static_bundle_manifest).Equals($resolvedManifest, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Prepared product-modset admission differs from the requested static bundle.'
}

$bindings = [ordered]@{}
foreach ($leaf in $bindingNames) { $bindings[$leaf] = (Resolve-Path -LiteralPath (Join-Path $resolvedBindings $leaf)).Path }
$sessionId = [guid]::NewGuid().ToString()
$captureId = "capture-$sessionId"
$lossEvidenceNonce = ([guid]::NewGuid().ToString('N') + [guid]::NewGuid().ToString('N')).ToLowerInvariant()
$captureManifest = Join-Path $resolvedOut 'native_session_manifest.json'
$actionReceipt = Join-Path $resolvedOut 'x2_autonomous_loss_width_actions.json'
$readinessReceipt = Join-Path $resolvedOut 'x2_autonomous_loss_width_ready.json'
$saveReadinessReceipt = Join-Path $resolvedOut 'x2_exact_save_readiness.json'
$snapshotDirectory = Join-Path $resolvedOut 'acceptance_snapshots'
$snapshotReport = Join-Path $resolvedOut 'x2_snapshot_extraction_report.json'
$regradeReceipt = Join-Path $resolvedOut 'x2_loss_width_regrade.json'

$arguments = @(
    '--early-launch-exe', $resolvedExe,
    '--early-launch-arg', '--continuelastsave',
    '--early-launch-arg', '--gdpr-compliant',
    '--pdx-user-dir', $resolvedUser,
    '--dll', $resolvedCollector,
    '--out-dir', $resolvedOut,
    '--session-id', $sessionId,
    '--capture-id', $captureId,
    '--duration-s', [string]$DurationSeconds,
    '--sample-ms', [string]$SampleMilliseconds,
    '--min-game-hours', [string]$MinimumGameHours,
    '--stream-format', 'compact',
    '--stream-compression', 'none',
    '--profile', 'debug',
    '--scheduled-domain', 'country_identity',
    '--scheduled-domain', 'land_combats_live',
    '--scheduled-domain', 'land_combat_detail_events_live',
    '--scheduled-domains-exact',
    '--hour-latch',
    '--hour-latch-poll-ms', '1',
    '--hour-latch-queue-cap', '64',
    '--static-bundle-manifest', $resolvedManifest,
    '--static-bundle-mod-state', [string]$bindings['mod_state.json'],
    '--static-bundle-dlc-state', [string]$bindings['dlc_state.json'],
    '--static-bundle-load-order', [string]$bindings['load_order.json'],
    '--static-bundle-source-inventory', [string]$bindings['source_inventory.json'],
    '--static-bundle-live-resolution-report', [string]$bindings['resolution_report.json'],
    '--static-bundle-verifier', $staticBundleVerifier,
    '--static-bundle-python', $resolvedPython,
    '--static-bundle-timeout-ms', '300000',
    '--sidecar-log'
)

$continuePath = Join-Path $resolvedUser 'continue_game.json'
$continuePriorExists = Test-Path -LiteralPath $continuePath -PathType Leaf
$continuePriorBytes = if ($continuePriorExists) { [IO.File]::ReadAllBytes($continuePath) } else { $null }
$continueObject = if ($continuePriorExists) {
    Get-Content -LiteralPath $continuePath -Raw | ConvertFrom-Json
} else {
    [pscustomobject]@{ title = [IO.Path]::GetFileNameWithoutExtension($resolvedSave); desc = ''; date = [DateTime]::UtcNow.ToString('R'); filename = ''; is_remote = $false }
}
$continueObject.filename = [IO.Path]::GetFileName($resolvedSave)
$continueObject.is_remote = $false
$recorderJob = $null
$exactProcess = $null
$completed = $false
$continueOverridden = $false
try {
    $continueOverridden = $true
    Write-AtomicUtf8 $continuePath (($continueObject | ConvertTo-Json -Depth 8) + "`n") -AllowReplace
    $recorderJob = Start-Job -ScriptBlock {
        param([string]$Recorder, [string[]]$RecorderArguments, [string]$LossEvidenceNonce)
        $env:BATTLE_PLAN_X2_WIDTH_EVIDENCE_CAPTURE = '1'
        $env:BATTLE_PLAN_X2_LOSS_EVIDENCE_CAPTURE = '1'
        $env:BATTLE_PLAN_X2_LOSS_EVIDENCE_NONCE = $LossEvidenceNonce
        & $Recorder @RecorderArguments
        if ($LASTEXITCODE -ne 0) { throw "collector_record exited with code $LASTEXITCODE" }
    } -ArgumentList $resolvedRecorder, (,$arguments), $lossEvidenceNonce

    $deadline = [DateTime]::UtcNow.AddSeconds($GameLoadTimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline -and $null -eq $exactProcess) {
        if ($recorderJob.State -in @('Failed', 'Stopped', 'Disconnected')) { throw "collector_record ended during exact-save launch: $($recorderJob.State)" }
        foreach ($candidate in @(Get-Process -Name the game -ErrorAction SilentlyContinue)) {
            try {
                if ($candidate.MainWindowHandle -ne [IntPtr]::Zero -and
                    [IO.Path]::GetFullPath($candidate.MainModule.FileName).Equals($resolvedExe, [StringComparison]::OrdinalIgnoreCase) -and
                    (Get-FileHash -LiteralPath $candidate.MainModule.FileName -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $expectedExeSha256) {
                    $exactProcess = $candidate
                    break
                }
            } catch { }
        }
        if ($null -eq $exactProcess) { Start-Sleep -Milliseconds 500 }
    }
    if ($null -eq $exactProcess) { throw 'Exact-a729 did not expose a window before the launch deadline.' }

    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $waitPaused `
        -ProcessId $exactProcess.Id -ReceiptPath $saveReadinessReceipt -ExpectedGameHourRaw 60830441 -TimeoutSeconds $GameLoadTimeoutSeconds
    if ($LASTEXITCODE -ne 0) { throw 'Exact D04 paused-save readiness failed.' }

    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $actionDriver `
        -ProcessId $exactProcess.Id `
        -ReceiptPath $actionReceipt `
        -SessionId $sessionId `
        -CaptureId $captureId `
        -ExpectedCollectorSha256 $collectorSha256 `
        -PrebuiltAttestationSha256 $attestationSha256 `
        -ProfilePath $profile `
        -ReadyPath $readinessReceipt `
        -CaptureManifestPath $captureManifest `
        -GameHoursToHold $MinimumGameHours `
        -GameHourAdvanceTimeoutSeconds 300
    if ($LASTEXITCODE -ne 0) { throw 'Autonomous X2 battle-fixture action failed.' }

    Wait-Job -Job $recorderJob | Out-Null
    $state = $recorderJob.State
    Receive-Job -Job $recorderJob -ErrorAction Continue | ForEach-Object { Write-Verbose ([string]$_) }
    if ($state -ne 'Completed') { throw "collector_record ended in state $state" }
    if (-not (Test-Path -LiteralPath $captureManifest -PathType Leaf)) { throw 'Recorder did not finalize native_session_manifest.json.' }
    $manifest = Get-Content -LiteralPath $captureManifest -Raw | ConvertFrom-Json
    $replayFile = [string]$manifest.replay_file
    if ([string]::IsNullOrWhiteSpace($replayFile) -or [IO.Path]::GetFileName($replayFile) -cne $replayFile) {
        throw 'Recorder manifest has no canonical replay_file leaf.'
    }
    $replayPath = Join-Path $resolvedOut $replayFile
    if (-not (Test-Path -LiteralPath $replayPath -PathType Leaf)) { throw 'Recorder replay is absent.' }

    & $resolvedPython $snapshotExtractor --replay $replayPath --out-dir $snapshotDirectory --report $snapshotReport
    if ($LASTEXITCODE -ne 0) { throw 'X2 replay snapshot extraction failed.' }
    & $resolvedPython $regrader `
        --snapshots $snapshotDirectory `
        --capture-manifest $captureManifest `
        --action-receipt $actionReceipt `
        --readiness-receipt $readinessReceipt `
        --action-profile $profile `
        --prebuilt-attestation $resolvedAttestation `
        --out $regradeReceipt
    if ($LASTEXITCODE -ne 0) { throw "H-X2-LOSS/WIDTH regrade failed; inspect $regradeReceipt" }
    $completed = $true
} finally {
    if ($null -ne $exactProcess -and $null -ne (Get-Process -Id $exactProcess.Id -ErrorAction SilentlyContinue)) {
        Stop-Process -Id $exactProcess.Id -Force -ErrorAction SilentlyContinue
    }
    if ($null -ne $recorderJob) {
        if ($recorderJob.State -notin @('Completed', 'Failed', 'Stopped', 'Disconnected')) {
            Stop-Job -Job $recorderJob -ErrorAction SilentlyContinue
            Wait-Job -Job $recorderJob | Out-Null
        }
        Receive-Job -Job $recorderJob -ErrorAction SilentlyContinue | Out-Null
        Remove-Job -Job $recorderJob -Force -ErrorAction SilentlyContinue
    }
    if ($continueOverridden) {
        if ($continuePriorExists) {
            Write-AtomicBytes $continuePath $continuePriorBytes -AllowReplace
        } elseif (Test-Path -LiteralPath $continuePath) {
            Remove-Item -LiteralPath $continuePath -Force
        }
    }
}

if (-not $completed) { throw 'H-X2-LOSS/WIDTH acquisition did not complete.' }
$finalReceiptPath = Join-Path $resolvedOut 'x2_loss_width_acquisition_receipt.json'
$regrade = Get-Content -LiteralPath $regradeReceipt -Raw | ConvertFrom-Json
$receipt = [ordered]@{
    schema = 'hoi4.x2.loss-width-autonomous-acquisition.v1'
    status = 'PASS'
    timestamp_utc = [DateTime]::UtcNow.ToString('o')
    program_sha256 = $expectedExeSha256
    save = [ordered]@{ logical_name = [IO.Path]::GetFileName($resolvedSave); sha256 = $ExpectedContinueSaveSha256 }
    session_id = $sessionId
    capture_id = $captureId
    clean_exact_save_launch = $true
    disposable_process_closed = $true
    environment = [ordered]@{
        BATTLE_PLAN_X2_WIDTH_EVIDENCE_CAPTURE = '1'
        BATTLE_PLAN_X2_LOSS_EVIDENCE_CAPTURE = '1'
        BATTLE_PLAN_X2_LOSS_EVIDENCE_NONCE = $lossEvidenceNonce
    }
    h_x2_loss = [ordered]@{ status = [string]$regrade.h_x2_loss.status; human_input_required = $false }
    h_x2_width = [ordered]@{ status = [string]$regrade.h_x2_width.status; human_input_required = $false }
    immutable_inputs = [ordered]@{
        native_session_manifest_sha256 = (Get-FileHash -LiteralPath $captureManifest -Algorithm SHA256).Hash.ToLowerInvariant()
        autonomous_action_sha256 = (Get-FileHash -LiteralPath $actionReceipt -Algorithm SHA256).Hash.ToLowerInvariant()
        autonomous_readiness_sha256 = (Get-FileHash -LiteralPath $readinessReceipt -Algorithm SHA256).Hash.ToLowerInvariant()
        exact_save_readiness_sha256 = (Get-FileHash -LiteralPath $saveReadinessReceipt -Algorithm SHA256).Hash.ToLowerInvariant()
        snapshot_extraction_report_sha256 = (Get-FileHash -LiteralPath $snapshotReport -Algorithm SHA256).Hash.ToLowerInvariant()
        regrade_sha256 = (Get-FileHash -LiteralPath $regradeReceipt -Algorithm SHA256).Hash.ToLowerInvariant()
        action_profile_sha256 = (Get-FileHash -LiteralPath $profile -Algorithm SHA256).Hash.ToLowerInvariant()
        central_release_attestation_sha256 = $attestationSha256
    }
    production_activation_changed = $false
}
Write-AtomicUtf8 $finalReceiptPath (($receipt | ConvertTo-Json -Depth 12) + "`n")
Write-Output "Autonomous H-X2-LOSS/WIDTH capture accepted: $finalReceiptPath"
