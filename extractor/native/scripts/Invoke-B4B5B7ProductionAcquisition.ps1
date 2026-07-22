[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$RecorderExe,
    [Parameter(Mandatory = $true)][string]$StreamFoldExe,
    [Parameter(Mandatory = $true)][string]$Hoi4Exe,
    [Parameter(Mandatory = $true)][string]$PdxUserDir,
    [Parameter(Mandatory = $true)][string]$CollectorDll,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [Parameter(Mandatory = $true)][string]$ContinueSave,
    [Parameter(Mandatory = $true)][string]$EventsOracle,
    [Parameter(Mandatory = $true)][string]$StaticBundleManifest,
    [Parameter(Mandatory = $true)][string]$StaticBundleBindingsDir,
    [string]$PrebuiltArtifactAttestationPath = '',
    [int]$DurationSeconds = 180,
    [int]$SampleMilliseconds = 100,
    [int]$GameLoadTimeoutSeconds = 300,
    [string]$Python = 'python',
    [string]$ExpectedContinueSaveSha256 = '49aec0944fd7aacb1819808b8d8b44608c70eb5bce921a60181dbe74ddf2a497',
    [switch]$EnableLegacyClientDriving
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $EnableLegacyClientDriving) {
    throw 'This legacy owner launches and controls HOI4. It is disabled by default. Use B4_B5_B7_OFFLINE_ORACLES/PASSIVE_OBSERVER_WORKFLOW.json with the shared passive observer; only an explicit -EnableLegacyClientDriving opt-in may enter this quarantined path.'
}

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$expectedGameHourRaw = 60830441
$expectedHourIndex = 17030442
$expectedEventsOracleSha256 = '8ba17b3b87c26b320c046c0fdab62d53031c23a0da9a118d92bfcbfe3f06f97c'
$expectedDiplomacyOracleSha256 = '70a6f4843d296e994de91a6fa551b8825991788bd434f4256123a6d809ff3eae'
$expectedRaidOracleSha256 = '8c047f414db357d298c61e78df79f714816b6b9fff35ca40a81597701bc27b84'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$packet = Join-Path $repo 'extractor\re_evidence\B4_B5_B7_OFFLINE_ORACLES'
$b4Owner = Join-Path $PSScriptRoot 'Invoke-B4SunkConvoyHistoryFixture.ps1'
$b5Tool = Join-Path $packet 'capture_and_verify_h_b5.py'
$b7Tool = Join-Path $packet 'capture_and_verify_h_b7.py'
$diplomacyOracle = Join-Path $packet 'diplomacy.json'
$raidOracle = Join-Path $packet 'raid_instances.json'
$readinessHelper = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_wait_for_paused_save.ps1'
$productModsetVerifier = Join-Path $repo 'extractor\game_defs\verify_product_modset_capture_ready.py'
$staticBundleVerifier = Join-Path $repo 'extractor\game_defs\static_bundle.py'
$attestationHelpers = Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1'
$productionDriver = Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1'
$bindingNames = @('mod_state.json', 'dlc_state.json', 'load_order.json', 'source_inventory.json', 'resolution_report.json')

function Test-IsWithinPath {
    param([Parameter(Mandatory = $true)][string]$Candidate, [Parameter(Mandatory = $true)][string]$Root)
    $candidateFull = [IO.Path]::GetFullPath($Candidate).TrimEnd([char]'\')
    $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd([char]'\')
    return $candidateFull.Equals($rootFull, [StringComparison]::OrdinalIgnoreCase) -or
        $candidateFull.StartsWith($rootFull + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)
}

function ConvertTo-NativeArgumentLine {
    param([Parameter(Mandatory = $true)][object[]]$Arguments)
    return (($Arguments | ForEach-Object {
        $value = [string]$_
        if ($value -match '[\s"]') {
            '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
        } else { $value }
    }) -join ' ')
}

function Write-AtomicNewBytes {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][byte[]]$Bytes)
    $full = [IO.Path]::GetFullPath($Path)
    if ([IO.File]::Exists($full)) { throw "Refusing to replace immutable receipt: $full" }
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($full)) | Out-Null
    $temporary = [IO.Path]::Combine([IO.Path]::GetDirectoryName($full), ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try { $stream.Write($Bytes, 0, $Bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
        [IO.File]::Move($temporary, $full)
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

function Write-AtomicNewText {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][string]$Text)
    Write-AtomicNewBytes -Path $Path -Bytes ([Text.UTF8Encoding]::new($false).GetBytes($Text))
}

function Write-AtomicNewJson {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)]$Value)
    Write-AtomicNewText -Path $Path -Text (($Value | ConvertTo-Json -Depth 40) + "`n")
}

function Write-AtomicReplaceBytes {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][byte[]]$Bytes)
    $full = [IO.Path]::GetFullPath($Path)
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($full)) | Out-Null
    $temporary = [IO.Path]::Combine([IO.Path]::GetDirectoryName($full), ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try { $stream.Write($Bytes, 0, $Bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($full)) { [IO.File]::Replace($temporary, $full, $null, $true) }
        else { [IO.File]::Move($temporary, $full) }
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

function Invoke-HiddenNative {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][object[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$Stdout,
        [Parameter(Mandatory = $true)][string]$Stderr
    )
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Executable
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    if ($null -ne $start.PSObject.Properties['ArgumentList']) {
        foreach ($argument in $Arguments) { [void]$start.ArgumentList.Add([string]$argument) }
    } else { $start.Arguments = ConvertTo-NativeArgumentLine $Arguments }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        if (-not $process.Start()) { throw "Failed to start $Executable" }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        Write-AtomicNewText -Path $Stdout -Text ([string]$stdoutTask.Result)
        Write-AtomicNewText -Path $Stderr -Text ([string]$stderrTask.Result)
        return $process.ExitCode
    } finally { $process.Dispose() }
}

function Get-LowerSha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-RandomLowerHex([int]$Bytes) {
    $buffer = [byte[]]::new($Bytes)
    [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($buffer)
    return ([BitConverter]::ToString($buffer)).Replace('-', '').ToLowerInvariant()
}

if ($DurationSeconds -lt 60 -or $DurationSeconds -gt 900) { throw 'DurationSeconds must be in 60..900.' }
if ($SampleMilliseconds -lt 10 -or $SampleMilliseconds -gt 1000) { throw 'SampleMilliseconds must be in 10..1000.' }
if ($GameLoadTimeoutSeconds -lt 30 -or $GameLoadTimeoutSeconds -gt 1800) { throw 'GameLoadTimeoutSeconds must be in 30..1800.' }
if ($ExpectedContinueSaveSha256 -cnotmatch '^[0-9a-f]{64}$') { throw 'ExpectedContinueSaveSha256 must be lowercase SHA-256.' }
foreach ($path in @(
    $RecorderExe, $StreamFoldExe, $Hoi4Exe, $PdxUserDir, $CollectorDll, $ContinueSave,
    $EventsOracle, $StaticBundleManifest, $StaticBundleBindingsDir, $b4Owner, $b5Tool,
    $b7Tool, $diplomacyOracle, $raidOracle, $readinessHelper, $productModsetVerifier,
    $staticBundleVerifier, $attestationHelpers, $productionDriver
)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required path is absent: $path" }
}
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
    throw 'B4/B5/B7 production acquisition requires a clean launch and refuses every pre-existing hoi4.exe.'
}

$resolvedRecorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedStreamFold = (Resolve-Path -LiteralPath $StreamFoldExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedSave = (Resolve-Path -LiteralPath $ContinueSave).Path
$resolvedEvents = (Resolve-Path -LiteralPath $EventsOracle).Path
$resolvedBundle = (Resolve-Path -LiteralPath $StaticBundleManifest).Path
$resolvedBindings = (Resolve-Path -LiteralPath $StaticBundleBindingsDir).Path
$pythonExe = (Get-Command $Python -CommandType Application -ErrorAction Stop).Source
$resolvedOut = [IO.Path]::GetFullPath($OutDir)
if (Test-IsWithinPath $resolvedOut $repo) { throw "OutDir must be outside the repository: $resolvedOut" }
if (Test-IsWithinPath $resolvedOut $resolvedUserDir) { throw "OutDir must be outside the game user directory: $resolvedOut" }
if (Test-Path -LiteralPath $resolvedOut) { throw "OutDir must be absent for an immutable acquisition: $resolvedOut" }
[IO.Directory]::CreateDirectory($resolvedOut) | Out-Null

$bindingPaths = [ordered]@{}
foreach ($leaf in $bindingNames) {
    $path = Join-Path $resolvedBindings $leaf
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Prepared binding is absent: $path" }
    $bindingPaths[$leaf] = (Resolve-Path -LiteralPath $path).Path
}

$saveDirectory = [IO.Path]::GetFullPath((Join-Path $resolvedUserDir 'save games')).TrimEnd([char]'\')
$requestedSaveDirectory = [IO.Path]::GetFullPath((Split-Path -Parent $resolvedSave)).TrimEnd([char]'\')
if (-not $requestedSaveDirectory.Equals($saveDirectory, [StringComparison]::OrdinalIgnoreCase)) {
    throw "ContinueSave must be a direct child of $saveDirectory"
}
$exeSha256 = Get-LowerSha256 $resolvedExe
if ($exeSha256 -cne $expectedExeSha256) { throw "hoi4.exe exact-a729 identity mismatch: $exeSha256" }
$saveSha256 = Get-LowerSha256 $resolvedSave
if ($saveSha256 -cne $ExpectedContinueSaveSha256) { throw "ContinueSave is not the frozen original D04 oracle save: $saveSha256" }
if ((Get-LowerSha256 $resolvedEvents) -cne $expectedEventsOracleSha256) { throw 'EventsOracle does not match the frozen B4 oracle.' }
if ((Get-LowerSha256 $diplomacyOracle) -cne $expectedDiplomacyOracleSha256) { throw 'Tracked B5 diplomacy oracle identity mismatch.' }
if ((Get-LowerSha256 $raidOracle) -cne $expectedRaidOracleSha256) { throw 'Tracked B7 raid oracle identity mismatch.' }

. $attestationHelpers
if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) {
    $PrebuiltArtifactAttestationPath = Join-Path $repo '.cargo-target\release\hoi4-production-prebuilt-artifacts.json'
}
$prebuilt = Test-ProductionArtifactAttestation `
    -RepoRoot $repo `
    -ReceiptPath $PrebuiltArtifactAttestationPath `
    -ExpectedDriverPath $productionDriver
if (-not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.recorder_host).Equals($resolvedRecorder, [StringComparison]::OrdinalIgnoreCase) -or
    -not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.collector_dll).Equals($resolvedCollector, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Recorder or collector differs from the authenticated centralized Release artifacts.'
}
$attestationCopy = Join-Path $resolvedOut 'central_release_attestation.json'
Write-AtomicNewBytes -Path $attestationCopy -Bytes ([IO.File]::ReadAllBytes([string]$prebuilt.receipt_path))

$preparedRoot = [IO.Path]::GetFullPath((Split-Path -Parent $resolvedBindings))
if (-not (Test-Path -LiteralPath (Join-Path $preparedRoot 'product_modset_capture_ready.json') -PathType Leaf)) {
    throw 'StaticBundleBindingsDir is not the bindings child of a prepared product-modset handoff.'
}
$preparedAdmissionStaging = Join-Path $resolvedOut 'prepared_product_modset_admission.staging.json'
$preparedAdmissionPath = Join-Path $resolvedOut 'prepared_product_modset_admission.json'
& $pythonExe $productModsetVerifier --exe $resolvedExe --prepared $preparedRoot --out $preparedAdmissionStaging
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $preparedAdmissionStaging -PathType Leaf)) {
    throw 'Prepared product-modset admission failed before launch.'
}
$preparedAdmission = Get-Content -LiteralPath $preparedAdmissionStaging -Raw | ConvertFrom-Json
if ([string]$preparedAdmission.schema -cne 'extractor.product_modset_capture_admission.v1' -or
    [string]$preparedAdmission.verdict -cne 'passed') { throw 'Prepared product-modset admission did not pass.' }
if (-not [IO.Path]::GetFullPath([string]$preparedAdmission.paths.static_bundle_manifest).Equals($resolvedBundle, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'StaticBundleManifest differs from prepared product-modset admission.'
}
foreach ($role in @('mod_state', 'dlc_state', 'load_order', 'source_inventory', 'resolution_report')) {
    if (-not [IO.Path]::GetFullPath([string]$preparedAdmission.paths.$role).Equals(
        [IO.Path]::GetFullPath([string]$bindingPaths["${role}.json"]), [StringComparison]::OrdinalIgnoreCase)) {
        throw "Prepared product-modset admission differs for $role."
    }
}
[IO.File]::Move($preparedAdmissionStaging, $preparedAdmissionPath)
$productModsetSha256 = [string]$preparedAdmission.product_modset_sha256
if ($productModsetSha256 -cnotmatch '^[0-9a-f]{64}$') { throw 'Prepared admission has no product-modset SHA-256.' }

$sessionId = [guid]::NewGuid().ToString()
$captureId = [guid]::NewGuid().ToString()
$acceptanceNonce = Get-RandomLowerHex 32
$nativeOut = Join-Path $resolvedOut 'native_capture'
$arguments = @(
    '--early-launch-exe', $resolvedExe,
    '--early-launch-arg', '--continuelastsave',
    '--early-launch-arg', '--gdpr-compliant',
    '--pdx-user-dir', $resolvedUserDir,
    '--dll', $resolvedCollector,
    '--out-dir', $nativeOut,
    '--session-id', $sessionId,
    '--capture-id', $captureId,
    '--duration-s', [string]$DurationSeconds,
    '--sample-ms', [string]$SampleMilliseconds,
    '--min-game-hours', '0',
    '--stream-format', 'compact',
    '--stream-compression', 'none',
    '--profile', 'production',
    '--production',
    '--scheduled-domain', 'country_identity',
    '--scheduled-domain', 'sunk_convoy_history_live',
    '--scheduled-domains-exact',
    '--acceptance-test-mode', 'paused',
    '--acceptance-test-nonce', $acceptanceNonce,
    '--static-bundle-manifest', $resolvedBundle,
    '--static-bundle-mod-state', [string]$bindingPaths['mod_state.json'],
    '--static-bundle-dlc-state', [string]$bindingPaths['dlc_state.json'],
    '--static-bundle-load-order', [string]$bindingPaths['load_order.json'],
    '--static-bundle-source-inventory', [string]$bindingPaths['source_inventory.json'],
    '--static-bundle-live-resolution-report', [string]$bindingPaths['resolution_report.json'],
    '--static-bundle-verifier', $staticBundleVerifier,
    '--static-bundle-python', $pythonExe,
    '--static-bundle-timeout-ms', '300000',
    '--sidecar-log'
)

$planPath = Join-Path $resolvedOut 'b4_b5_b7_acquisition_plan.json'
Write-AtomicNewJson -Path $planPath -Value ([ordered]@{
    schema = 'hoi4.b4-b5-b7-production-acquisition-plan.v1'
    transaction = 'clean_exact_a729_original_d04_paused_b4_b5_b7'
    attempt_budget = 1
    leaves_game_running = $true
    leaves_game_paused = $true
    session_id = $sessionId
    capture_id = $captureId
    executable_sha256 = $exeSha256
    continue_save_sha256 = $saveSha256
    expected_game_hour_raw = $expectedGameHourRaw
    expected_game_hour_index = $expectedHourIndex
    central_attestation_sha256 = [string]$prebuilt.receipt_sha256
    production_input_sha256 = [string]$prebuilt.source_input_sha256
    recorder_sha256 = [string]$prebuilt.artifact_sha256.recorder_host
    collector_sha256 = [string]$prebuilt.artifact_sha256.collector_dll
    stream_fold_sha256 = Get-LowerSha256 $resolvedStreamFold
    prepared_admission_sha256 = Get-LowerSha256 $preparedAdmissionPath
    product_modset_sha256 = $productModsetSha256
    scheduled_domains = @('country_identity', 'sunk_convoy_history_live')
    paused_evidence_authority = 'nonce-bound recorder acceptance_test_mode=paused; production registry remains reserved'
    recorder_arguments = $arguments
})

$readinessReceipt = Join-Path $resolvedOut 'authenticated_paused_save_readiness.json'
$terminalReadinessReceipt = Join-Path $resolvedOut 'authenticated_terminal_paused_save_readiness.json'
$b5Dir = Join-Path $resolvedOut 'h_b5'
$b7Dir = Join-Path $resolvedOut 'h_b7'
$b4Dir = Join-Path $resolvedOut 'h_b4'
$captureManifest = Join-Path $nativeOut 'native_session_manifest.json'
$collectorStream = Join-Path $nativeOut 'collector_stream.bin'
$foldDir = Join-Path $resolvedOut 'b4_fold'
$foldReportStaging = Join-Path $resolvedOut 'b4_fold_report.staging.json'
$foldReport = Join-Path $resolvedOut 'b4_fold_report.json'
$continueGamePath = Join-Path $resolvedUserDir 'continue_game.json'
$continuePriorExists = Test-Path -LiteralPath $continueGamePath -PathType Leaf
$continuePriorBytes = if ($continuePriorExists) { [IO.File]::ReadAllBytes($continueGamePath) } else { $null }
$continueOverridden = $false
$recorder = [Diagnostics.Process]::new()
$recorderStarted = $false
$stdoutTask = $null
$stderrTask = $null
$game = $null
$transactionError = $null
$cleanupError = $null

try {
    $continueObject = if ($continuePriorExists) {
        Get-Content -LiteralPath $continueGamePath -Raw | ConvertFrom-Json
    } else {
        [pscustomobject]@{ title = [IO.Path]::GetFileNameWithoutExtension($resolvedSave); desc = ''; date = [DateTime]::UtcNow.ToString('R'); filename = ''; is_remote = $false }
    }
    $continueObject.filename = [IO.Path]::GetFileName($resolvedSave)
    $continueObject.is_remote = $false
    $continueOverridden = $true
    Write-AtomicReplaceBytes -Path $continueGamePath -Bytes ([Text.UTF8Encoding]::new($false).GetBytes((($continueObject | ConvertTo-Json -Depth 10) + "`n")))

    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $resolvedRecorder
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    if ($null -ne $start.PSObject.Properties['ArgumentList']) {
        foreach ($argument in $arguments) { [void]$start.ArgumentList.Add([string]$argument) }
    } else { $start.Arguments = ConvertTo-NativeArgumentLine $arguments }
    $recorder.StartInfo = $start
    if (-not $recorder.Start()) { throw 'collector_record failed to start.' }
    $recorderStarted = $true
    $stdoutTask = $recorder.StandardOutput.ReadToEndAsync()
    $stderrTask = $recorder.StandardError.ReadToEndAsync()

    $discoveryDeadline = [DateTime]::UtcNow.AddMinutes(30)
    while ([DateTime]::UtcNow -lt $discoveryDeadline -and -not $recorder.HasExited) {
        $candidates = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
        if ($candidates.Count -gt 1) { throw 'Clean launch produced more than one hoi4.exe process.' }
        if ($candidates.Count -eq 1) {
            $candidate = $candidates[0]
            try {
                $candidatePath = [string](Get-CimInstance Win32_Process -Filter "ProcessId=$($candidate.Id)" -ErrorAction Stop).ExecutablePath
                if ($candidate.MainWindowHandle -ne [IntPtr]::Zero -and
                    [IO.Path]::GetFullPath($candidatePath).Equals($resolvedExe, [StringComparison]::OrdinalIgnoreCase) -and
                    (Get-LowerSha256 $candidatePath) -ceq $expectedExeSha256) { $game = $candidate; break }
            } catch { }
        }
        Start-Sleep -Milliseconds 250
    }
    if ($null -eq $game) {
        if ($recorder.HasExited) { throw "collector_record exited before exact-a729 exposed a window (code $($recorder.ExitCode))." }
        throw 'Exact-a729 did not expose a window within the static-bundle admission budget.'
    }

    $readyExit = Invoke-HiddenNative -Executable 'powershell.exe' -Arguments @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $readinessHelper,
        '-ProcessId', [string]$game.Id, '-ReceiptPath', $readinessReceipt,
        '-ExpectedGameHourRaw', [string]$expectedGameHourRaw, '-TimeoutSeconds', [string]$GameLoadTimeoutSeconds,
        '-StableSamples', '4', '-PollMilliseconds', '250'
    ) -Stdout (Join-Path $resolvedOut 'readiness.stdout.log') -Stderr (Join-Path $resolvedOut 'readiness.stderr.log')
    if ($readyExit -ne 0 -or -not (Test-Path -LiteralPath $readinessReceipt -PathType Leaf)) { throw 'Authenticated paused D04 readiness failed.' }

    $b5Exit = Invoke-HiddenNative -Executable $pythonExe -Arguments @(
        $b5Tool, '--save', $resolvedSave, '--process-id', [string]$game.Id,
        '--oracle', $diplomacyOracle, '--output-dir', $b5Dir,
        '--output-prefix', 'h_b5_exact_d04', '--ack-read-only-paused-d04'
    ) -Stdout (Join-Path $resolvedOut 'h_b5.stdout.log') -Stderr (Join-Path $resolvedOut 'h_b5.stderr.log')
    if ($b5Exit -ne 0) { throw "H-B5 capture/regrade rejected with code $b5Exit." }

    $b7Exit = Invoke-HiddenNative -Executable $pythonExe -Arguments @(
        $b7Tool, '--save', $resolvedSave, '--process-id', [string]$game.Id,
        '--oracle', $raidOracle, '--output-dir', $b7Dir,
        '--output-prefix', 'h_b7_exact_d04', '--ack-read-only-paused-d04'
    ) -Stdout (Join-Path $resolvedOut 'h_b7.stdout.log') -Stderr (Join-Path $resolvedOut 'h_b7.stderr.log')
    if ($b7Exit -ne 0) { throw "H-B7 capture/regrade rejected with code $b7Exit." }

    if (-not $recorder.WaitForExit([Math]::Max(1000, ($DurationSeconds + 300) * 1000))) {
        throw 'collector_record exceeded the bounded paused evidence/finalization deadline.'
    }
    if ($recorder.ExitCode -ne 0) { throw "collector_record exited with code $($recorder.ExitCode)." }
} catch {
    $transactionError = $_
} finally {
    if ($recorderStarted -and -not $recorder.HasExited) {
        if (-not $recorder.WaitForExit([Math]::Max(1000, ($DurationSeconds + 300) * 1000)) -and $null -eq $cleanupError) {
            $cleanupError = [System.Management.Automation.RuntimeException]::new('collector_record did not detach inside its bounded normal deadline')
        }
    }
    if ($recorderStarted -and $recorder.HasExited) {
        try {
            if ($null -ne $stdoutTask) { Write-AtomicNewText -Path (Join-Path $resolvedOut 'collector_record.stdout.log') -Text ([string]$stdoutTask.Result) }
            if ($null -ne $stderrTask) { Write-AtomicNewText -Path (Join-Path $resolvedOut 'collector_record.stderr.log') -Text ([string]$stderrTask.Result) }
        } catch { if ($null -eq $cleanupError) { $cleanupError = $_ } }
    }
    $recorder.Dispose()
    if ($continueOverridden) {
        try {
            if ($continuePriorExists) { Write-AtomicReplaceBytes -Path $continueGamePath -Bytes $continuePriorBytes }
            elseif (Test-Path -LiteralPath $continueGamePath -PathType Leaf) { Remove-Item -LiteralPath $continueGamePath -Force }
        } catch { if ($null -eq $cleanupError) { $cleanupError = $_ } }
    }
}
if ($null -ne $cleanupError) {
    $primary = if ($null -ne $transactionError) { "$($transactionError.Exception.Message); " } else { '' }
    throw "$primary cleanup failure: $($cleanupError.Exception.Message)"
}
if ($null -ne $transactionError) { throw $transactionError }
foreach ($required in @($captureManifest, $collectorStream, (Join-Path $b5Dir 'h_b5_exact_d04.debug.json'), (Join-Path $b5Dir 'h_b5_exact_d04.portable.json'), (Join-Path $b7Dir 'h_b7_exact_d04.debug.json'), (Join-Path $b7Dir 'h_b7_exact_d04.portable.json'))) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "Required acquisition artifact is absent: $required" }
}

$foldExit = Invoke-HiddenNative -Executable $resolvedStreamFold -Arguments @(
    $collectorStream, $foldDir, '--game-id', 'h_b4_exact_d04', '--json', $foldReportStaging,
    '--workers', '1', '--identity-mode', 'legacy'
) -Stdout (Join-Path $resolvedOut 'stream_fold.stdout.log') -Stderr (Join-Path $resolvedOut 'stream_fold.stderr.log')
if ($foldExit -ne 0 -or -not (Test-Path -LiteralPath $foldReportStaging -PathType Leaf)) { throw 'stream_fold rejected the B4 paused evidence stream.' }
[IO.File]::Move($foldReportStaging, $foldReport)

$snapshotCandidates = [Collections.Generic.List[string]]::new()
foreach ($candidate in @(Get-ChildItem -LiteralPath $foldDir -Filter '*.snapshot.json' -File)) {
    try {
        $snapshot = Get-Content -LiteralPath $candidate.FullName -Raw | ConvertFrom-Json
        if ([int64]$snapshot.hour_index -eq $expectedHourIndex -and
            $null -ne $snapshot._r2_sunk_convoy_history_live_candidate) {
            $snapshotCandidates.Add($candidate.FullName)
        }
    } catch { }
}
if ($snapshotCandidates.Count -ne 1) { throw "Expected one exact-hour B4 folded snapshot, found $($snapshotCandidates.Count)." }
$productionSnapshot = $snapshotCandidates[0]

$b4Exit = Invoke-HiddenNative -Executable 'powershell.exe' -Arguments @(
    '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $b4Owner,
    '-Save', $resolvedSave, '-ProcessId', [string]$game.Id, '-EventsOracle', $resolvedEvents,
    '-ProductionSnapshot', $productionSnapshot, '-OutDir', $b4Dir, '-PythonExe', $pythonExe
) -Stdout (Join-Path $resolvedOut 'h_b4.stdout.log') -Stderr (Join-Path $resolvedOut 'h_b4.stderr.log')
if ($b4Exit -ne 0) { throw "H-B4 capture/independent regrade rejected with code $b4Exit." }

$terminalReadyExit = Invoke-HiddenNative -Executable 'powershell.exe' -Arguments @(
    '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $readinessHelper,
    '-ProcessId', [string]$game.Id, '-ReceiptPath', $terminalReadinessReceipt,
    '-ExpectedGameHourRaw', [string]$expectedGameHourRaw, '-TimeoutSeconds', '30',
    '-StableSamples', '4', '-PollMilliseconds', '250'
) -Stdout (Join-Path $resolvedOut 'terminal_readiness.stdout.log') -Stderr (Join-Path $resolvedOut 'terminal_readiness.stderr.log')
if ($terminalReadyExit -ne 0) { throw 'Terminal exact-D04 pause authentication failed.' }

$b4Report = Get-Content -LiteralPath (Join-Path $b4Dir 'b4_sunk_convoy_fixture_regrade.json') -Raw | ConvertFrom-Json
$b5Portable = Get-Content -LiteralPath (Join-Path $b5Dir 'h_b5_exact_d04.portable.json') -Raw | ConvertFrom-Json
$b7Portable = Get-Content -LiteralPath (Join-Path $b7Dir 'h_b7_exact_d04.portable.json') -Raw | ConvertFrom-Json
if ([string]$b4Report.status -cne 'ACCEPTED_EXACT_441_OF_441' -or
    [string]$b5Portable.status -cne 'ACCEPTED_EXACT_958_OF_958' -or
    [string]$b7Portable.status -cne 'ACCEPTED_EXACT_440_PARENTS_167_CHILDREN') {
    throw 'One or more B4/B5/B7 portable verdicts are not exact acceptance.'
}

$finalReceipt = Join-Path $resolvedOut 'b4_b5_b7_combined_acquisition_receipt.json'
$artifactRows = @()
foreach ($file in @(Get-ChildItem -LiteralPath $resolvedOut -Recurse -File | Sort-Object FullName)) {
    if ($file.FullName -eq $finalReceipt) { continue }
    $relative = $file.FullName.Substring($resolvedOut.Length).TrimStart([char]'\').Replace('\', '/')
    $artifactRows += [ordered]@{ path = $relative; bytes = [int64]$file.Length; sha256 = Get-LowerSha256 $file.FullName }
}
Write-AtomicNewJson -Path $finalReceipt -Value ([ordered]@{
    schema = 'hoi4.b4-b5-b7-combined-production-acquisition.v1'
    verdict = 'PASS_EXACT_B4_441_B5_958_B7_440_167'
    transaction = 'clean_exact_a729_original_d04_paused_b4_b5_b7'
    session_id = $sessionId
    capture_id = $captureId
    process_id = $game.Id
    executable_sha256 = $exeSha256
    continue_save_sha256 = $saveSha256
    game_hour_raw = $expectedGameHourRaw
    game_hour_index = $expectedHourIndex
    product_modset_sha256 = $productModsetSha256
    central_attestation_sha256 = Get-LowerSha256 $attestationCopy
    prepared_admission_sha256 = Get-LowerSha256 $preparedAdmissionPath
    b4 = [ordered]@{ status = [string]$b4Report.status; rows = [int]$b4Report.verification.raw_rows; production_activation_authorized = [bool]$b4Report.production_activation_authorized }
    b5 = [ordered]@{ status = [string]$b5Portable.status; rows = [int]$b5Portable.verification.joined_rows }
    b7 = [ordered]@{ status = [string]$b7Portable.status; parents = [int]$b7Portable.verification.parents; children = [int]$b7Portable.verification.children }
    terminal_pause_authenticated = $true
    leaves_game_running = $true
    leaves_game_paused = $true
    artifact_count = $artifactRows.Count
    artifacts = $artifactRows
})
Write-Output "Combined B4/B5/B7 production acquisition mechanically admitted: $finalReceipt"
