[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][switch]$EnableLegacyClientDriving,
    [Parameter(Mandatory = $true)][string]$Hoi4Exe,
    [Parameter(Mandatory = $true)][string]$PdxUserDir,
    [Parameter(Mandatory = $true)][string]$ContinueSave,
    [Parameter(Mandatory = $true)][string]$PreparedProductModsetDir,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [string]$PrebuiltArtifactAttestationPath = '',
    [int]$DurationSeconds = 600,
    [int]$LoadTimeoutSeconds = 300,
    [int]$MaximumWheelSteps = 160,
    [string]$Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $EnableLegacyClientDriving) {
    throw 'This isolated legacy tool launches and drives the game client. It is never a default or promotion dependency; use the XN14Q passive observer contract instead.'
}
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$expectedD04SaveSha256 = 'c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735'
$expectedD04GameHourRaw = 60830441
$branches = @('near_hidden', 'near_fade', 'full', 'far_fade', 'far_hidden')
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$xn14q = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\XN14Q'
$cameraHelper = Join-Path $xn14q 'xn14q_autonomous_camera.py'
$verifier = Join-Path $xn14q 'verify_xn14q_same_session.py'
$readinessHelper = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_wait_for_paused_save.ps1'
$pulseHelper = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_one_hour_pulse.ps1'
$productVerifier = Join-Path $repo 'extractor\game_defs\verify_product_modset_capture_ready.py'
$staticVerifier = Join-Path $repo 'extractor\game_defs\static_bundle.py'
$attestationHelpers = Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1'
$driver = Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1'

function ConvertTo-NativeArgumentLine([object[]]$Arguments) {
    return (($Arguments | ForEach-Object {
        $value = [string]$_
        if ($value -match '[\s"]') {
            '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
        } else { $value }
    }) -join ' ')
}
function Get-Sha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}
function Get-Sha256Bytes([byte[]]$Bytes) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
}
function Write-AtomicReplaceBytes([string]$Path, [byte[]]$Bytes) {
    $full = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($full)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllBytes($temporary, $Bytes)
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($full)) { [IO.File]::Replace($temporary, $full, $null, $true) }
        else { [IO.File]::Move($temporary, $full) }
    } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
}
function Write-AtomicNewJson([string]$Path, $Value) {
    $full = [IO.Path]::GetFullPath($Path)
    if ([IO.File]::Exists($full)) { throw "Refusing to replace immutable XN14Q artifact: $full" }
    $parent = [IO.Path]::GetDirectoryName($full)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText($temporary, (($Value | ConvertTo-Json -Depth 30) + "`n"), [Text.UTF8Encoding]::new($false))
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        [IO.File]::Move($temporary, $full)
    } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
}
function Write-AtomicNewText([string]$Path, [string]$Text) {
    $full = [IO.Path]::GetFullPath($Path)
    if ([IO.File]::Exists($full)) { throw "Refusing to replace immutable XN14Q artifact: $full" }
    $parent = [IO.Path]::GetDirectoryName($full)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText($temporary, $Text, [Text.UTF8Encoding]::new($false))
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        [IO.File]::Move($temporary, $full)
    } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
}
function Invoke-CameraObserve([string]$Path, [int]$ExpectedHour) {
    & $pythonExe $cameraHelper observe --output $Path --require-paused --require-game-hour-raw $ExpectedHour | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "XN14Q camera observation failed: $Path" }
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}
function Invoke-Wheel([int]$Delta, [string]$Path, [int]$ProcessId) {
    & $pythonExe $cameraHelper wheel --process-id $ProcessId --delta $Delta --settle-ms 750 --receipt $Path | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "XN14Q authenticated wheel action failed: $Path" }
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}

if ($DurationSeconds -lt 300 -or $DurationSeconds -gt 1800) { throw 'DurationSeconds must be 300..1800.' }
if ($MaximumWheelSteps -lt 40 -or $MaximumWheelSteps -gt 400) { throw 'MaximumWheelSteps must be 40..400.' }
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) { throw 'XN14Q requires a clean early launch.' }
$exe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$user = (Resolve-Path -LiteralPath $PdxUserDir).Path
$save = (Resolve-Path -LiteralPath $ContinueSave).Path
$prepared = (Resolve-Path -LiteralPath $PreparedProductModsetDir).Path
$pythonExe = (Get-Command $Python -CommandType Application -ErrorAction Stop).Source
$out = [IO.Path]::GetFullPath($OutDir)
if ((Get-Sha256 $exe) -cne $expectedExeSha256) { throw 'Hoi4Exe is not exact a729.' }
if ((Get-Sha256 $save) -cne $expectedD04SaveSha256) { throw 'ContinueSave is not frozen New D04 Save Deep War.' }
$saveRoot = [IO.Path]::GetFullPath((Join-Path $user 'save games')).TrimEnd([char]'\')
if (-not [string]::Equals((Split-Path -Parent $save).TrimEnd([char]'\'), $saveRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "ContinueSave must be a direct child of $saveRoot"
}
foreach ($required in @($cameraHelper, $verifier, $readinessHelper, $pulseHelper, $productVerifier, $staticVerifier, $attestationHelpers, $driver)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "Missing XN14Q helper: $required" }
}
if (Test-Path -LiteralPath $out) { throw 'XN14Q OutDir must be absent (immutable acquisition).' }
[IO.Directory]::CreateDirectory($out) | Out-Null

. $attestationHelpers
if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) {
    $PrebuiltArtifactAttestationPath = Join-Path $repo '.cargo-target\release\hoi4-production-prebuilt-artifacts.json'
}
$prebuilt = Test-ProductionArtifactAttestation -RepoRoot $repo -ReceiptPath $PrebuiltArtifactAttestationPath -ExpectedDriverPath $driver
$recorderExe = (Resolve-Path -LiteralPath ([string]$prebuilt.artifacts.recorder_host)).Path
$collectorDll = (Resolve-Path -LiteralPath ([string]$prebuilt.artifacts.collector_dll)).Path
$admissionPath = Join-Path $out 'prepared_product_modset_admission.json'
& $pythonExe $productVerifier --exe $exe --prepared $prepared --out $admissionPath
if ($LASTEXITCODE -ne 0) { throw 'Prepared product-modset admission failed.' }
$admission = Get-Content -LiteralPath $admissionPath -Raw | ConvertFrom-Json

$sessionId = [guid]::NewGuid().ToString()
$captureId = [guid]::NewGuid().ToString()
$actionsDir = Join-Path $out 'authenticated-actions'
[IO.Directory]::CreateDirectory($actionsDir) | Out-Null
$readinessReceipt = Join-Path $actionsDir 'paused-save-readiness.json'
$ledgerPath = Join-Path $out 'xn14q_action_ledger.json'
$manifestPath = Join-Path $out 'native_session_manifest.json'
$streamPath = Join-Path $out 'collector_stream.bin'
$verificationPath = Join-Path $out 'xn14q_same_session_verification.json'
$arguments = @(
    '--early-launch-exe', $exe,
    '--early-launch-arg', '--continuelastsave',
    '--early-launch-arg', '--gdpr-compliant',
    '--pdx-user-dir', $user,
    '--dll', $collectorDll,
    '--out-dir', $out,
    '--session-id', $sessionId,
    '--capture-id', $captureId,
    '--duration-s', [string]$DurationSeconds,
    '--sample-ms', '25',
    # Five consecutive branch target frames span four raw-hour intervals.
    '--min-game-hours', '4',
    '--stream-format', 'compact',
    '--stream-compression', 'none',
    '--profile', 'debug',
    '--scheduled-domain', 'session_header_live',
    '--scheduled-domains-exact',
    '--static-bundle-manifest', [string]$admission.paths.static_bundle_manifest,
    '--static-bundle-mod-state', [string]$admission.paths.mod_state,
    '--static-bundle-dlc-state', [string]$admission.paths.dlc_state,
    '--static-bundle-load-order', [string]$admission.paths.load_order,
    '--static-bundle-source-inventory', [string]$admission.paths.source_inventory,
    '--static-bundle-live-resolution-report', [string]$admission.paths.resolution_report,
    '--static-bundle-verifier', $staticVerifier,
    '--static-bundle-python', $pythonExe,
    '--static-bundle-timeout-ms', '1800000',
    '--sidecar-log',
    '--hour-latch'
)
$start = [Diagnostics.ProcessStartInfo]::new()
$start.FileName = $recorderExe
$start.Arguments = ConvertTo-NativeArgumentLine $arguments
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$recorder = [Diagnostics.Process]::new()
$recorder.StartInfo = $start

$continuePath = Join-Path $user 'continue_game.json'
$priorExists = Test-Path -LiteralPath $continuePath -PathType Leaf
$priorBytes = if ($priorExists) { [IO.File]::ReadAllBytes($continuePath) } else { $null }
$priorSha256 = if ($priorExists) { Get-Sha256Bytes $priorBytes } else { $null }
$continue = if ($priorExists) { Get-Content -LiteralPath $continuePath -Raw | ConvertFrom-Json } else {
    [pscustomobject]@{ title = [IO.Path]::GetFileNameWithoutExtension($save); desc = ''; date = [DateTime]::UtcNow.ToString('R'); filename = ''; is_remote = $false }
}
if ($null -eq $continue.PSObject.Properties['filename']) { $continue | Add-Member -NotePropertyName filename -NotePropertyValue '' }
if ($null -eq $continue.PSObject.Properties['is_remote']) { $continue | Add-Member -NotePropertyName is_remote -NotePropertyValue $false }
$continue.filename = [IO.Path]::GetFileName($save)
$continue.is_remote = $false
$replacementBytes = [Text.UTF8Encoding]::new($false).GetBytes((($continue | ConvertTo-Json -Depth 10) + "`n"))
$replacementSha256 = Get-Sha256Bytes $replacementBytes
$overridden = $false
$started = $false
$stdoutTask = $null
$stderrTask = $null
$owned = $null
$ownedStartUtc = $null
$transactionError = $null
$branchRows = [Collections.Generic.List[object]]::new()

try {
    $overridden = $true
    Write-AtomicReplaceBytes $continuePath $replacementBytes
    if ((Get-Sha256 $continuePath) -cne $replacementSha256) { throw 'continue_game.json override changed after publication.' }
    if (-not $recorder.Start()) { throw 'collector_record failed to start.' }
    $started = $true
    $stdoutTask = $recorder.StandardOutput.ReadToEndAsync()
    $stderrTask = $recorder.StandardError.ReadToEndAsync()
    $deadline = [DateTime]::UtcNow.AddSeconds($LoadTimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline -and -not $recorder.HasExited -and $null -eq $owned) {
        $processes = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
        if ($processes.Count -gt 1) { throw 'Clean XN14Q launch produced multiple hoi4.exe processes.' }
        if ($processes.Count -eq 1) {
            try {
                $candidate = $processes[0]
                $parent = Get-CimInstance Win32_Process -Filter "ProcessId=$($candidate.Id)" -ErrorAction Stop
                if ($candidate.MainWindowHandle -ne [IntPtr]::Zero -and
                    [int]$parent.ParentProcessId -eq $recorder.Id -and
                    [IO.Path]::GetFullPath([string]$candidate.MainModule.FileName).Equals($exe, [StringComparison]::OrdinalIgnoreCase) -and
                    (Get-Sha256 ([string]$candidate.MainModule.FileName)) -ceq $expectedExeSha256) {
                    $owned = $candidate
                    $ownedStartUtc = $candidate.StartTime.ToUniversalTime().ToString('o')
                }
            } catch { }
        }
        if ($null -eq $owned) { Start-Sleep -Milliseconds 250 }
    }
    if ($null -eq $owned) { throw 'Owned exact-a729 window did not become ready.' }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $readinessHelper -ProcessId $owned.Id -ReceiptPath $readinessReceipt -ExpectedGameHourRaw $expectedD04GameHourRaw -TimeoutSeconds $LoadTimeoutSeconds -StableSamples 4 -PollMilliseconds 250 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'XN14Q paused-save readiness failed.' }
    $ready = Get-Content -LiteralPath $readinessReceipt -Raw | ConvertFrom-Json
    if ($ready.process_start_utc -cne $ownedStartUtc -or $ready.game_hour -ne $expectedD04GameHourRaw) { throw 'Readiness receipt differs from owned D04 process.' }

    $stepOrdinal = 0
    $probePath = Join-Path $actionsDir ('camera-probe-{0:D3}.json' -f $stepOrdinal)
    $probe = Invoke-CameraObserve $probePath $expectedD04GameHourRaw
    while ([string]$probe.derived_camera_branch -cne 'near_hidden' -or [string]$probe.derived_target_camera_branch -cne 'near_hidden') {
        if ($stepOrdinal -ge $MaximumWheelSteps) { throw 'Could not reach XN14Q near_hidden within bounded wheel steps.' }
        $stepOrdinal++
        $wheelPath = Join-Path $actionsDir ('wheel-close-{0:D3}.json' -f $stepOrdinal)
        $wheel = Invoke-Wheel 120 $wheelPath $owned.Id
        $probe = $wheel.after
    }

    $currentHour = $expectedD04GameHourRaw
    for ($targetIndex = 0; $targetIndex -lt $branches.Count; $targetIndex++) {
        $target = $branches[$targetIndex]
        $targetSteps = 0
        while ([string]$probe.derived_camera_branch -cne $target -or [string]$probe.derived_target_camera_branch -cne $target) {
            if ($targetSteps -ge $MaximumWheelSteps) { throw "Could not reach XN14Q $target within bounded wheel steps." }
            $targetSteps++
            $stepOrdinal++
            $wheelPath = Join-Path $actionsDir ('wheel-open-{0:D3}.json' -f $stepOrdinal)
            $wheel = Invoke-Wheel -120 $wheelPath $owned.Id
            $probe = $wheel.after
            $actualIndex = [Array]::IndexOf($branches, [string]$probe.derived_camera_branch)
            if ($actualIndex -gt $targetIndex) { throw "Wheel traversal skipped XN14Q branch $target; stopped fail closed." }
        }
        $cameraReceipt = Join-Path $actionsDir ("camera-observation-$target.json")
        $camera = Invoke-CameraObserve $cameraReceipt $currentHour
        if ([string]$camera.derived_camera_branch -cne $target -or [string]$camera.derived_target_camera_branch -cne $target) {
            throw "Authenticated current/target feedback did not retain $target."
        }
        $pulseReceipt = Join-Path $actionsDir ("clock-pulse-$target.json")
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $pulseHelper -ProcessId $owned.Id -ReceiptPath $pulseReceipt -TargetHours 1 -TimeoutMilliseconds 60000 -PollMilliseconds 1 | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "Authenticated one-hour pulse failed for $target." }
        $pulse = Get-Content -LiteralPath $pulseReceipt -Raw | ConvertFrom-Json
        if ($pulse.status -cne 'PASS' -or $pulse.game_hour_before -ne $currentHour -or $pulse.game_hour_after -ne ($currentHour + 1)) {
            throw "Authenticated pulse receipt disagrees with XN14Q $target transaction."
        }
        $branchRows.Add([ordered]@{
            branch = $target
            source_game_hour_raw = $currentHour
            selected_present_game_hour_raw = [int]$pulse.game_hour_after
            camera_observation = ('authenticated-actions/' + [IO.Path]::GetFileName($cameraReceipt))
            camera_observation_sha256 = Get-Sha256 $cameraReceipt
            pulse_receipt = ('authenticated-actions/' + [IO.Path]::GetFileName($pulseReceipt))
            pulse_receipt_sha256 = Get-Sha256 $pulseReceipt
        })
        $currentHour = [int]$pulse.game_hour_after
        $probe = $camera
    }
    Write-AtomicNewJson $ledgerPath ([ordered]@{
        schema = 'hoi4-xn14q-five-camera-branch-action-ledger-v1'
        status = 'COMPLETE'
        executable_sha256 = $expectedExeSha256
        process_id = $owned.Id
        process_start_utc = $ownedStartUtc
        session_id = $sessionId
        capture_id = $captureId
        traversal = 'near_hidden_to_far_hidden_authenticated_wheel_sweep'
        branches = @($branchRows)
    })
    if (-not $recorder.WaitForExit(($DurationSeconds + 600) * 1000)) { throw 'collector_record did not finalize before deadline.' }
    if ($recorder.ExitCode -ne 0) { throw "collector_record exited with code $($recorder.ExitCode)." }
} catch {
    $transactionError = $_
} finally {
    if ($overridden) {
        try {
            if ((Get-Sha256 $continuePath) -cne $replacementSha256) { throw 'continue_game.json changed during owned XN14Q launch.' }
            if ($priorExists) {
                Write-AtomicReplaceBytes $continuePath $priorBytes
                if ((Get-Sha256 $continuePath) -cne $priorSha256) { throw 'continue_game.json prior bytes were not restored.' }
            } else { Remove-Item -LiteralPath $continuePath -Force }
        } finally { $overridden = $false }
    }
    if ($started -and -not $recorder.HasExited) {
        Stop-Process -Id $recorder.Id -Force -ErrorAction SilentlyContinue
        [void]$recorder.WaitForExit(30000)
    }
    if ($started -and $recorder.HasExited -and $null -ne $stdoutTask -and $null -ne $stderrTask) {
        Write-AtomicNewText (Join-Path $out 'collector_record.stdout.log') ([string]$stdoutTask.Result)
        Write-AtomicNewText (Join-Path $out 'collector_record.stderr.log') ([string]$stderrTask.Result)
    }
    $recorder.Dispose()
}
if ($null -ne $transactionError) { throw $transactionError }
foreach ($required in @($manifestPath, $streamPath, $ledgerPath)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "XN14Q finalized artifact is absent: $required" }
}
& $pythonExe $verifier --stream $streamPath --capture-manifest $manifestPath --readiness-receipt $readinessReceipt --action-ledger $ledgerPath --output $verificationPath
if ($LASTEXITCODE -ne 0) { throw "XN14Q same-session transaction was rejected; inspect $verificationPath" }
Write-Output "XN14Q five-branch selected-Present transaction accepted: $verificationPath"
