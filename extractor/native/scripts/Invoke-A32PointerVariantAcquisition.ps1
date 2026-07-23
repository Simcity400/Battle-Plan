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
    [int]$DurationSeconds = 240,
    [int]$SampleMilliseconds = 25,
    [int]$ProcessTimeoutSeconds = 300,
    [int]$InitialSettleSeconds = 30,
    [int]$PointerActionMilliseconds = 60000,
    [string[]]$EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string]$Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$expectedD04SaveSha256 = 'c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735'
$expectedD04HourRaw = 60830441
if ($DurationSeconds -lt 120 -or $DurationSeconds -gt 900) { throw 'DurationSeconds must be in 120..900.' }
if ($SampleMilliseconds -lt 10 -or $SampleMilliseconds -gt 250) { throw 'SampleMilliseconds must be in 10..250.' }
if ($PointerActionMilliseconds -lt 5000 -or $PointerActionMilliseconds -gt 120000) { throw 'PointerActionMilliseconds must be in 5000..120000.' }
if ($ProcessTimeoutSeconds -lt 30 -or $ProcessTimeoutSeconds -gt 600) { throw 'ProcessTimeoutSeconds must be in 30..600.' }
if ($InitialSettleSeconds -lt 0 -or $InitialSettleSeconds -gt 300) { throw 'InitialSettleSeconds must be in 0..300.' }
foreach ($path in @($RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll, $ContinueSave, $StaticBundleManifest, $StaticBundleBindingsDir)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required A32 path does not exist: $path" }
}

$recorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$exe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$user = (Resolve-Path -LiteralPath $PdxUserDir).Path
$collector = (Resolve-Path -LiteralPath $CollectorDll).Path
$save = (Resolve-Path -LiteralPath $ContinueSave).Path
$bundleManifest = (Resolve-Path -LiteralPath $StaticBundleManifest).Path
$bindings = (Resolve-Path -LiteralPath $StaticBundleBindingsDir).Path
$bundleDir = Split-Path -Parent $bundleManifest
$pythonExe = (Get-Command $Python -CommandType Application -ErrorAction Stop).Source
$out = [IO.Path]::GetFullPath($OutDir)
$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\..')).Path
$productionDriver = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1')).Path
$attestationHelpers = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1')).Path
$staticVerifier = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\game_defs\static_bundle.py')).Path
$readinessHelper = (Resolve-Path -LiteralPath (Join-Path $repo 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_wait_for_paused_save.ps1')).Path
$pulseHelper = (Resolve-Path -LiteralPath (Join-Path $repo 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_one_hour_pulse.ps1')).Path
$actionDriver = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Invoke-A32AuthenticatedPointerMovement.ps1')).Path
$verifier = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'verify_a32_pointer_variant_capture.py')).Path
if ((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) { throw 'Hoi4Exe is not exact a729.' }
if ((Get-FileHash -LiteralPath $save -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedD04SaveSha256) { throw 'A32 requires New D04 Save Deep War.' }
$saveRoot = [IO.Path]::GetFullPath((Join-Path $user 'save games')).TrimEnd([char]'\')
if (-not [IO.Path]::GetFullPath((Split-Path -Parent $save)).TrimEnd([char]'\').Equals($saveRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "ContinueSave must be a direct child of $saveRoot"
}
$continueIndex = [Array]::IndexOf($EarlyLaunchArgument, '--continuelastsave')
$gdprIndex = [Array]::IndexOf($EarlyLaunchArgument, '--gdpr-compliant')
if ($continueIndex -lt 0 -or $gdprIndex -lt $continueIndex) { throw 'EarlyLaunchArgument must contain --continuelastsave followed by --gdpr-compliant.' }
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) { throw 'A32 requires a clean launch with no existing hoi4.exe.' }
if (Test-Path -LiteralPath $out) {
    if (-not (Test-Path -LiteralPath $out -PathType Container) -or @(Get-ChildItem -LiteralPath $out -Force).Count -ne 0) {
        throw "A32 OutDir must be absent or empty: $out"
    }
} else { [IO.Directory]::CreateDirectory($out) | Out-Null }

function Write-AtomicBytes([string]$Path, [byte[]]$Bytes, [switch]$Replace) {
    $full = [IO.Path]::GetFullPath($Path); $parent = [IO.Path]::GetDirectoryName($full)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllBytes($temporary, $Bytes)
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($full)) {
            if (-not $Replace) { throw "Refusing to replace $full" }
            [IO.File]::Replace($temporary, $full, $null, $true)
        } else { [IO.File]::Move($temporary, $full) }
    } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
}
function Write-AtomicText([string]$Path, [string]$Text, [switch]$Replace) {
    Write-AtomicBytes $Path ([Text.UTF8Encoding]::new($false).GetBytes($Text)) -Replace:$Replace
}

. $attestationHelpers
if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) { $PrebuiltArtifactAttestationPath = Join-Path $repo '.cargo-target\release\hoi4-production-prebuilt-artifacts.json' }
$prebuilt = Test-ProductionArtifactAttestation -RepoRoot $repo -ReceiptPath $PrebuiltArtifactAttestationPath -ExpectedDriverPath $productionDriver
$attestation = (Resolve-Path -LiteralPath $prebuilt.receipt_path).Path
if (-not $recorder.Equals([string]$prebuilt.artifacts.recorder_host, [StringComparison]::OrdinalIgnoreCase) -or
    -not $collector.Equals([string]$prebuilt.artifacts.collector_dll, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'A32 recorder and collector must come from the authenticated central Release attestation.'
}

$binding = [ordered]@{
    mod_state = Join-Path $bindings 'mod_state.json'; dlc_state = Join-Path $bindings 'dlc_state.json'
    load_order = Join-Path $bindings 'load_order.json'; source_inventory = Join-Path $bindings 'source_inventory.json'
    resolution_report = Join-Path $bindings 'resolution_report.json'
}
foreach ($key in @($binding.Keys)) {
    if (-not (Test-Path -LiteralPath $binding[$key] -PathType Leaf)) { throw "A32 binding missing: $key" }
    $binding[$key] = (Resolve-Path -LiteralPath $binding[$key]).Path
}
$preflightOutput = @(& $pythonExe $staticVerifier verify --bundle $bundleDir --exe $exe `
    --mod-state $binding.mod_state --dlc-state $binding.dlc_state --load-order $binding.load_order `
    --source-inventory $binding.source_inventory --live-resolution-report $binding.resolution_report `
    --allow-deferred-token-gate)
if ($LASTEXITCODE -ne 0) { throw 'A32 static-bundle preflight failed.' }
$preflightText = $preflightOutput -join [Environment]::NewLine
$preflight = $preflightText | ConvertFrom-Json
if ($preflight.passed -ne $true -or $preflight.verification_phase -cne 'pre_injection_deferred_allowed') { throw 'A32 preflight did not admit deferred token verification.' }
$preflightReceipt = Join-Path $out 'a32_static_bundle_preflight.json'
Write-AtomicText $preflightReceipt ($preflightText + [Environment]::NewLine)

$continuePath = Join-Path $user 'continue_game.json'
$continueExisted = Test-Path -LiteralPath $continuePath -PathType Leaf
$continueBytes = if ($continueExisted) { [IO.File]::ReadAllBytes($continuePath) } else { $null }
$continueObject = if ($continueExisted) { Get-Content -LiteralPath $continuePath -Raw | ConvertFrom-Json } else {
    [pscustomobject]@{ title = [IO.Path]::GetFileNameWithoutExtension($save); desc = ''; date = [DateTime]::UtcNow.ToString('R'); filename = ''; is_remote = $false }
}
$continueObject.filename = [IO.Path]::GetFileName($save); $continueObject.is_remote = $false
$continueReplacement = ($continueObject | ConvertTo-Json -Depth 8) + [Environment]::NewLine

$sessionId = [guid]::NewGuid().ToString(); $captureId = [guid]::NewGuid().ToString()
$arguments = @(
    '--early-launch-exe', $exe, '--pdx-user-dir', $user, '--dll', $collector, '--out-dir', $out,
    '--session-id', $sessionId, '--capture-id', $captureId, '--duration-s', [string]$DurationSeconds,
    '--sample-ms', [string]$SampleMilliseconds, '--min-game-hours', '1', '--stream-format', 'compact',
    '--profile', 'debug', '--scheduled-domain', 'divisions', '--scheduled-domains-exact',
    '--static-bundle-manifest', $bundleManifest, '--static-bundle-mod-state', $binding.mod_state,
    '--static-bundle-dlc-state', $binding.dlc_state, '--static-bundle-load-order', $binding.load_order,
    '--static-bundle-source-inventory', $binding.source_inventory,
    '--static-bundle-live-resolution-report', $binding.resolution_report,
    '--static-bundle-verifier', $staticVerifier, '--static-bundle-python', $pythonExe,
    '--static-bundle-timeout-ms', '300000', '--sidecar-log', '--hour-latch'
)
foreach ($value in $EarlyLaunchArgument) { $arguments += @('--early-launch-arg', $value) }

$manifestPath = Join-Path $out 'native_session_manifest.json'
$readinessReceipt = Join-Path $out 'a32_paused_save_readiness.json'
$pulseReceipt = Join-Path $out 'a32_authenticated_one_hour_pulse.json'
$actionReceipt = Join-Path $out 'a32_authenticated_pointer_actions.json'
$actionReady = Join-Path $out 'a32_pointer_current_position_ready.json'
$actionStdout = Join-Path $out 'a32_action_driver.stdout.log'
$actionStderr = Join-Path $out 'a32_action_driver.stderr.log'
$admissionPath = Join-Path $out 'a32_xui25_immutable_admission.json'
$job = $null; $action = $null; $game = $null; $processStartUtc = $null
$completed = $false; $continueOverridden = $false
try {
    $continueOverridden = $true
    Write-AtomicText $continuePath $continueReplacement -Replace
    $job = Start-Job -ScriptBlock {
        param([string]$Recorder, [string[]]$Arguments)
        & $Recorder @Arguments
        if ($LASTEXITCODE -ne 0) { throw "collector_record exited with code $LASTEXITCODE" }
    } -ArgumentList $recorder, (,$arguments)
    $deadline = [DateTime]::UtcNow.AddSeconds($ProcessTimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline -and $null -eq $game) {
        if ($job.State -in @('Failed', 'Stopped', 'Disconnected')) { throw "collector_record terminated during A32 startup: $($job.State)" }
        foreach ($candidate in @(Get-Process -Name the game -ErrorAction SilentlyContinue)) {
            try {
                if ($candidate.MainWindowHandle -ne [IntPtr]::Zero -and
                    [IO.Path]::GetFullPath($candidate.MainModule.FileName).Equals($exe, [StringComparison]::OrdinalIgnoreCase) -and
                    (Get-FileHash -LiteralPath $candidate.MainModule.FileName -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $expectedExeSha256) {
                    $game = $candidate; break
                }
            } catch { }
        }
        if ($null -eq $game) { Start-Sleep -Milliseconds 500 }
    }
    if ($null -eq $game) { throw 'Exact-a729 did not expose an A32 window before timeout.' }
    $processStartUtc = $game.StartTime.ToUniversalTime().ToString('o')
    Start-Sleep -Seconds $InitialSettleSeconds
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $readinessHelper `
        -ProcessId $game.Id -ReceiptPath $readinessReceipt -ExpectedGameHourRaw $expectedD04HourRaw `
        -TimeoutSeconds $ProcessTimeoutSeconds -StableSamples 4 -PollMilliseconds 250
    if ($LASTEXITCODE -ne 0) { throw 'A32 exact paused-save readiness failed.' }
    $actionArguments = @(
        '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', ('"' + $actionDriver + '"'),
        '-ProcessId', [string]$game.Id, '-SessionId', ('"' + $sessionId + '"'), '-CaptureId', ('"' + $captureId + '"'),
        '-ExpectedProcessStartUtc', ('"' + $processStartUtc + '"'), '-ReceiptPath', ('"' + $actionReceipt + '"'),
        '-ReadyPath', ('"' + $actionReady + '"'), '-DurationMilliseconds', [string]$PointerActionMilliseconds,
        '-BaselineHoldMilliseconds', '1500'
    )
    $action = Start-Process -FilePath 'powershell.exe' -ArgumentList $actionArguments -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $actionStdout -RedirectStandardError $actionStderr
    $actionReadyDeadline = [DateTime]::UtcNow.AddSeconds(20)
    while ([DateTime]::UtcNow -lt $actionReadyDeadline -and -not $action.HasExited -and
        -not (Test-Path -LiteralPath $actionReady -PathType Leaf)) { Start-Sleep -Milliseconds 50 }
    if (-not (Test-Path -LiteralPath $actionReady -PathType Leaf)) { throw "A32 pointer current-position readiness was not produced; inspect $actionStderr" }
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $pulseHelper `
        -ProcessId $game.Id -ReceiptPath $pulseReceipt -TargetHours 1 -TimeoutMilliseconds 120000 -PollMilliseconds 1
    if ($LASTEXITCODE -ne 0) { throw 'A32 authenticated one-hour pulse failed.' }
    if (-not $action.WaitForExit($PointerActionMilliseconds + 30000) -or $action.ExitCode -ne 0 -or
        -not (Test-Path -LiteralPath $actionReceipt -PathType Leaf)) {
        throw "A32 pointer action driver failed; inspect $actionStderr"
    }
    Wait-Job -Job $job | Out-Null
    $state = $job.State
    Receive-Job -Job $job -ErrorAction Continue | ForEach-Object { Write-Verbose ([string]$_) }
    if ($state -ne 'Completed') { throw "collector_record ended in state $state" }
    $completed = $true
} finally {
    if ($null -ne $action -and -not $action.HasExited) { Stop-Process -Id $action.Id -Force -ErrorAction SilentlyContinue }
    if ($null -ne $job) {
        if ($job.State -notin @('Completed', 'Failed', 'Stopped', 'Disconnected')) { if (-not $completed) { Stop-Job -Job $job -ErrorAction SilentlyContinue }; Wait-Job -Job $job | Out-Null }
        Receive-Job -Job $job -ErrorAction SilentlyContinue | Out-Null
        Remove-Job -Job $job -Force -ErrorAction SilentlyContinue
    }
    if ($continueOverridden) {
        if ($continueExisted) { Write-AtomicBytes $continuePath $continueBytes -Replace }
        elseif (Test-Path -LiteralPath $continuePath) { Remove-Item -LiteralPath $continuePath -Force }
    }
}
if (-not $completed -or -not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { throw 'A32 recorder capture did not complete.' }
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
$replayName = [string]$manifest.replay_file
if ([string]::IsNullOrWhiteSpace($replayName) -or [IO.Path]::GetFileName($replayName) -cne $replayName) { throw 'A32 replay_file is absent or nonlocal.' }
$replay = [IO.Path]::GetFullPath((Join-Path $out $replayName))
if (-not (Test-Path -LiteralPath $replay -PathType Leaf)) { throw "A32 replay was not produced: $replay" }
& $pythonExe $verifier --capture-manifest $manifestPath --prebuilt-attestation $attestation `
    --static-bundle-preflight $preflightReceipt --readiness-receipt $readinessReceipt `
    --pulse-receipt $pulseReceipt --action-receipt $actionReceipt --action-ready-receipt $actionReady `
    --replay $replay --out $admissionPath
if ($LASTEXITCODE -ne 0) { throw 'A32/XUI25 immutable admission failed closed.' }
Write-Output "A32/XUI25 same-launch pointer witness admitted: $admissionPath"
