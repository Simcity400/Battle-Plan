[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Hoi4Exe,
    [Parameter(Mandatory = $true)][string]$PdxUserDir,
    [Parameter(Mandatory = $true)][string]$PreparedProductModsetDir,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [string]$PrebuiltArtifactAttestationPath = '',
    [int]$DurationSeconds = 360,
    [int]$SampleMilliseconds = 100,
    [int]$GameLoadTimeoutSeconds = 240,
    [string]$Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$releaseRoot = Join-Path $repo '.cargo-target\release'
$productionDriver = Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1'
$attestationHelpers = Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1'
$staticBundleVerifier = Join-Path $repo 'extractor\game_defs\static_bundle.py'
$productModsetVerifier = Join-Path $repo 'extractor\game_defs\verify_product_modset_capture_ready.py'
$pulse = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_one_hour_pulse.ps1'
$ensurePaused = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\CAP6\authenticated_ensure_paused.ps1'
$receiptVerifier = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\CAP6\verify_live_final_d3d_receipt.py'

function ConvertTo-NativeArgumentLine {
    param([Parameter(Mandatory = $true)][object[]]$Arguments)
    return (($Arguments | ForEach-Object {
        $value = [string]$_
        if ($value -match '[\s"]') {
            '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
        } else {
            $value
        }
    }) -join ' ')
}

function Write-AtomicJson {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)]$Value)
    $parent = Split-Path -Parent $Path
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = "$Path.$([guid]::NewGuid().ToString('N')).tmp"
    try {
        [IO.File]::WriteAllText($temporary, (($Value | ConvertTo-Json -Depth 20) + "`n"), [Text.UTF8Encoding]::new($false))
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        Move-Item -LiteralPath $temporary -Destination $Path -Force
    } finally {
        if (Test-Path -LiteralPath $temporary -PathType Leaf) { Remove-Item -LiteralPath $temporary -Force }
    }
}

function Invoke-HiddenPowerShell {
    param(
        [Parameter(Mandatory = $true)][string]$Script,
        [Parameter(Mandatory = $true)][object[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$Stdout,
        [Parameter(Mandatory = $true)][string]$Stderr
    )
    $allArguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $Script) + $Arguments
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = 'powershell.exe'
    $info.Arguments = ConvertTo-NativeArgumentLine -Arguments $allArguments
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $child = [Diagnostics.Process]::new()
    $child.StartInfo = $info
    try {
        if (-not $child.Start()) { throw "Failed to start authenticated helper: $Script" }
        $stdoutTask = $child.StandardOutput.ReadToEndAsync()
        $stderrTask = $child.StandardError.ReadToEndAsync()
        $child.WaitForExit()
        [IO.File]::WriteAllText($Stdout, [string]$stdoutTask.Result, [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($Stderr, [string]$stderrTask.Result, [Text.UTF8Encoding]::new($false))
        return $child.ExitCode
    } finally {
        $child.Dispose()
    }
}

function Get-ExactHoi4Process {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][string]$Sha256
    )
    $matches = [Collections.Generic.List[Diagnostics.Process]]::new()
    foreach ($candidate in @(Get-Process -Name the game -ErrorAction SilentlyContinue)) {
        try {
            $path = [string](Get-CimInstance Win32_Process -Filter "ProcessId=$($candidate.Id)" -ErrorAction Stop).ExecutablePath
            if (-not [string]::IsNullOrWhiteSpace($path) -and
                [IO.Path]::GetFullPath($path).Equals($Executable, [StringComparison]::OrdinalIgnoreCase) -and
                (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $Sha256) {
                $matches.Add($candidate)
            }
        } catch {
            # A terminating process is not an authenticated cleanup target.
        }
    }
    if ($matches.Count -gt 1) { throw 'More than one exact-a729 hoi4.exe process exists.' }
    if ($matches.Count -eq 1) { return $matches[0] }
    return $null
}

if ($DurationSeconds -lt 30 -or $DurationSeconds -gt 1800) { throw 'DurationSeconds must be in 30..1800.' }
if ($SampleMilliseconds -lt 1 -or $SampleMilliseconds -gt 1000) { throw 'SampleMilliseconds must be in 1..1000.' }
if ($GameLoadTimeoutSeconds -lt 30 -or $GameLoadTimeoutSeconds -gt 900) { throw 'GameLoadTimeoutSeconds must be in 30..900.' }
if ($DurationSeconds -lt ($GameLoadTimeoutSeconds + 60)) { throw 'DurationSeconds must leave at least 60 seconds after GameLoadTimeoutSeconds for the evidence window.' }
foreach ($path in @(
    $Hoi4Exe, $PdxUserDir, $PreparedProductModsetDir, $productionDriver,
    $attestationHelpers, $staticBundleVerifier, $productModsetVerifier, $pulse,
    $ensurePaused, $receiptVerifier
)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required path is absent: $path" }
}
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
    throw 'CAP6 clean early launch requires no existing hoi4.exe process; it never closes or replaces a protected session.'
}

$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedPrepared = (Resolve-Path -LiteralPath $PreparedProductModsetDir).Path
$resolvedOut = [IO.Path]::GetFullPath($OutDir)
$pythonExe = (Get-Command $Python -CommandType Application -ErrorAction Stop).Source
$exeSha256 = (Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($exeSha256 -cne $expectedExeSha256) { throw "hoi4.exe exact-a729 SHA-256 mismatch: $exeSha256" }
$continueGame = Join-Path $resolvedUserDir 'continue_game.json'
if (-not (Test-Path -LiteralPath $continueGame -PathType Leaf)) { throw "Continue selection is absent: $continueGame" }
$continue = Get-Content -LiteralPath $continueGame -Raw | ConvertFrom-Json
if ([string]::IsNullOrWhiteSpace([string]$continue.filename) -or [bool]$continue.is_remote) {
    throw 'continue_game.json does not select one local save.'
}
$continuedSave = Join-Path (Join-Path $resolvedUserDir 'save games') ([string]$continue.filename)
if (-not (Test-Path -LiteralPath $continuedSave -PathType Leaf)) { throw "Selected continue save is absent: $continuedSave" }

if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or
        @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty: $resolvedOut"
    }
} else {
    [IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
}

. $attestationHelpers
if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) {
    $PrebuiltArtifactAttestationPath = Join-Path $releaseRoot 'hoi4-production-prebuilt-artifacts.json'
}
$prebuilt = Test-ProductionArtifactAttestation `
    -RepoRoot $repo `
    -ReceiptPath $PrebuiltArtifactAttestationPath `
    -ExpectedDriverPath $productionDriver
$resolvedRecorder = [string]$prebuilt.artifacts.recorder_host
$resolvedCollector = [string]$prebuilt.artifacts.collector_dll

$preparedAdmissionPath = Join-Path $resolvedOut 'prepared_product_modset_admission.json'
& $pythonExe $productModsetVerifier --exe $resolvedExe --prepared $resolvedPrepared --out $preparedAdmissionPath
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $preparedAdmissionPath -PathType Leaf)) {
    throw 'Prepared product-modset admission failed before launch.'
}
$preparedAdmission = Get-Content -LiteralPath $preparedAdmissionPath -Raw | ConvertFrom-Json
if ([string]$preparedAdmission.schema -cne 'extractor.product_modset_capture_admission.v1' -or
    [string]$preparedAdmission.verdict -cne 'passed') {
    throw 'Prepared product-modset admission did not emit a passing canonical receipt.'
}

$sessionId = [guid]::NewGuid().ToString()
$captureId = [guid]::NewGuid().ToString()
$arguments = @(
    '--early-launch-exe', $resolvedExe,
    '--early-launch-arg', '--continuelastsave',
    '--early-launch-arg', '--gdpr-compliant',
    '--pdx-user-dir', $resolvedUserDir,
    '--dll', $resolvedCollector,
    '--out-dir', $resolvedOut,
    '--session-id', $sessionId,
    '--capture-id', $captureId,
    '--duration-s', [string]$DurationSeconds,
    '--sample-ms', [string]$SampleMilliseconds,
    '--min-game-hours', '1',
    '--stream-format', 'compact',
    '--profile', 'debug',
    '--scheduled-domain', 'session_header_live',
    '--scheduled-domains-exact',
    '--static-bundle-manifest', [string]$preparedAdmission.paths.static_bundle_manifest,
    '--static-bundle-mod-state', [string]$preparedAdmission.paths.mod_state,
    '--static-bundle-dlc-state', [string]$preparedAdmission.paths.dlc_state,
    '--static-bundle-load-order', [string]$preparedAdmission.paths.load_order,
    '--static-bundle-source-inventory', [string]$preparedAdmission.paths.source_inventory,
    '--static-bundle-live-resolution-report', [string]$preparedAdmission.paths.resolution_report,
    '--static-bundle-verifier', $staticBundleVerifier,
    '--static-bundle-python', $pythonExe,
    '--static-bundle-timeout-ms', '300000',
    '--sidecar-log'
)

$planPath = Join-Path $resolvedOut 'cap6_clean_early_launch_plan.json'
Write-AtomicJson -Path $planPath -Value ([ordered]@{
    schema = 'hoi4.cap6.clean-early-launch-plan.v1'
    transaction = 'fully_unattended_exact_a729_cap6_final_d3d'
    attempt_budget = 1
    executable_sha256 = $exeSha256
    continued_save_sha256 = (Get-FileHash -LiteralPath $continuedSave -Algorithm SHA256).Hash.ToLowerInvariant()
    continue_game_sha256 = (Get-FileHash -LiteralPath $continueGame -Algorithm SHA256).Hash.ToLowerInvariant()
    session_id = $sessionId
    capture_id = $captureId
    duration_seconds = $DurationSeconds
    sample_milliseconds = $SampleMilliseconds
    game_load_timeout_seconds = $GameLoadTimeoutSeconds
    prebuilt_attestation_sha256 = [string]$prebuilt.receipt_sha256
    production_input_sha256 = [string]$prebuilt.source_input_sha256
    collector_sha256 = [string]$prebuilt.artifact_sha256.collector_dll
    recorder_sha256 = [string]$prebuilt.artifact_sha256.recorder_host
    prepared_admission_sha256 = (Get-FileHash -LiteralPath $preparedAdmissionPath -Algorithm SHA256).Hash.ToLowerInvariant()
    product_modset_sha256 = [string]$preparedAdmission.product_modset_sha256
    exact_scheduled_domains = @('session_header_live')
    pulse_contract = 'one authenticated hour followed by unconditional exact-a729 repause'
    leaves_game_running = $true
    leaves_game_paused = $true
})

$start = [Diagnostics.ProcessStartInfo]::new()
$start.FileName = $resolvedRecorder
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.EnvironmentVariables['BATTLE_PLAN_A25_GUI_BRIDGE_EVIDENCE_OUT_DIR'] = $resolvedOut
if ($null -ne $start.PSObject.Properties['ArgumentList']) {
    foreach ($argument in $arguments) { [void]$start.ArgumentList.Add([string]$argument) }
} else {
    $start.Arguments = ConvertTo-NativeArgumentLine $arguments
}

$recorder = [Diagnostics.Process]::new()
$recorder.StartInfo = $start
$recorderStarted = $false
$stdoutTask = $null
$stderrTask = $null
$game = $null
$transactionError = $null
$cleanupError = $null
$pulseAccepted = $false
$attemptErrors = [Collections.Generic.List[string]]::new()
try {
    if (-not $recorder.Start()) { throw 'collector_record failed to start.' }
    $recorderStarted = $true
    $stdoutTask = $recorder.StandardOutput.ReadToEndAsync()
    $stderrTask = $recorder.StandardError.ReadToEndAsync()

    # The recorder owns a suspended process while it authenticates the static
    # bundle. Only accept the one exact executable after it has a real window.
    $processDiscoveryDeadline = [DateTime]::UtcNow.AddMinutes(10)
    while ([DateTime]::UtcNow -lt $processDiscoveryDeadline -and -not $recorder.HasExited) {
        $candidates = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
        if ($candidates.Count -eq 1) {
            $candidatePath = $null
            try {
                $candidatePath = [string](Get-CimInstance Win32_Process -Filter "ProcessId=$($candidates[0].Id)" -ErrorAction Stop).ExecutablePath
            } catch {
                $candidatePath = $null
            }
            if ($candidates[0].MainWindowHandle -ne [IntPtr]::Zero -and
                -not [string]::IsNullOrWhiteSpace($candidatePath) -and
                [string]::Equals([IO.Path]::GetFullPath($candidatePath), $resolvedExe, [StringComparison]::OrdinalIgnoreCase)) {
                $game = $candidates[0]
                break
            }
        } elseif ($candidates.Count -gt 1) {
            throw 'Clean launch produced more than one hoi4.exe process.'
        }
        Start-Sleep -Milliseconds 250
    }
    if ($null -eq $game) {
        if ($recorder.HasExited) { throw "collector_record exited before exact-a729 the game became ready (code $($recorder.ExitCode))." }
        throw 'Exact-a729 the game did not become ready before the clean-launch timeout.'
    }

    $deadline = [DateTime]::UtcNow.AddSeconds($GameLoadTimeoutSeconds)
    $pulseReceipt = Join-Path $resolvedOut 'cap6_authenticated_one_hour_pulse.json'
    while ([DateTime]::UtcNow -lt $deadline -and -not $recorder.HasExited) {
        $pulseStdout = Join-Path $resolvedOut 'cap6_pulse_attempt.stdout.tmp.log'
        $pulseStderr = Join-Path $resolvedOut 'cap6_pulse_attempt.stderr.tmp.log'
        $pulseProcess = Invoke-HiddenPowerShell `
            -Script $pulse `
            -Arguments @(
                '-ProcessId', [string]$game.Id,
                '-ReceiptPath', $pulseReceipt,
                '-TimeoutMilliseconds', '30000',
                '-PollMilliseconds', '1'
            ) `
            -Stdout $pulseStdout `
            -Stderr $pulseStderr
        if ($pulseProcess -eq 0 -and (Test-Path -LiteralPath $pulseReceipt -PathType Leaf)) {
            $pulseAccepted = $true
            Remove-Item -LiteralPath $pulseStdout, $pulseStderr -Force -ErrorAction SilentlyContinue
            break
        }
        $message = if (Test-Path -LiteralPath $pulseStderr -PathType Leaf) {
            (Get-Content -LiteralPath $pulseStderr -Raw).Trim()
        } else {
            "pulse attempt exited $pulseProcess"
        }
        if (-not [string]::IsNullOrWhiteSpace($message)) { $attemptErrors.Add($message) }
        Remove-Item -LiteralPath $pulseStdout, $pulseStderr -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2
    }
    if (-not $pulseAccepted) {
        $tail = @($attemptErrors | Select-Object -Last 3) -join ' | '
        throw "CAP6 clean launch never reached an authenticated paused one-hour pulse state. $tail"
    }

    $remainingMs = [Math]::Max(1000, ($DurationSeconds + 90) * 1000)
    if (-not $recorder.WaitForExit($remainingMs)) { throw 'collector_record exceeded the bounded CAP6 capture/finalization deadline.' }
    if ($recorder.ExitCode -ne 0) { throw "collector_record exited with code $($recorder.ExitCode)." }
} catch {
    $transactionError = $_
} finally {
    # Whether capture succeeds or fails, restore the exact live process to
    # paused before returning. This helper only writes after exact SHA/vtable
    # validation and records whether a write was required.
    if ($null -eq $game) {
        try { $game = Get-ExactHoi4Process -Executable $resolvedExe -Sha256 $expectedExeSha256 }
        catch { if ($null -eq $cleanupError) { $cleanupError = $_ } }
    }
    if ($null -ne $game -and -not $game.HasExited) {
        $pauseReceipt = Join-Path $resolvedOut 'cap6_final_ensure_paused.json'
        $pauseStdout = Join-Path $resolvedOut 'cap6_final_ensure_paused.stdout.log'
        $pauseStderr = Join-Path $resolvedOut 'cap6_final_ensure_paused.stderr.log'
        try {
            $pauseProcess = Invoke-HiddenPowerShell `
                -Script $ensurePaused `
                -Arguments @('-ProcessId', [string]$game.Id, '-ReceiptPath', $pauseReceipt) `
                -Stdout $pauseStdout `
                -Stderr $pauseStderr
            if ($pauseProcess -ne 0 -or -not (Test-Path -LiteralPath $pauseReceipt -PathType Leaf)) {
                $detail = if (Test-Path -LiteralPath $pauseStderr -PathType Leaf) { (Get-Content -LiteralPath $pauseStderr -Raw).Trim() } else { '' }
                throw "authenticated final repause failed (code $pauseProcess): $detail"
            }
        } catch {
            $cleanupError = $_
        }
    }

    if ($recorderStarted -and -not $recorder.HasExited) {
        [void]$recorder.WaitForExit([Math]::Max(1000, ($DurationSeconds + 90) * 1000))
    }
    if ($recorderStarted -and $recorder.HasExited) {
        if ($null -ne $stdoutTask) {
            [IO.File]::WriteAllText((Join-Path $resolvedOut 'collector_record.stdout.log'), [string]$stdoutTask.Result, [Text.UTF8Encoding]::new($false))
        }
        if ($null -ne $stderrTask) {
            [IO.File]::WriteAllText((Join-Path $resolvedOut 'collector_record.stderr.log'), [string]$stderrTask.Result, [Text.UTF8Encoding]::new($false))
        }
    } elseif ($recorderStarted) {
        $recorderCleanupError = [System.Management.Automation.RuntimeException]::new('collector_record remained live after its bounded cleanup deadline')
        if ($null -eq $cleanupError) { $cleanupError = $recorderCleanupError }
    }
    $recorder.Dispose()
}

if ($null -ne $cleanupError) {
    $primary = if ($null -ne $transactionError) { "$($transactionError.Exception.Message); " } else { '' }
    throw "$primary cleanup failure: $($cleanupError.Exception.Message)"
}
if ($null -ne $transactionError) { throw $transactionError }

$manifest = Join-Path $resolvedOut 'native_session_manifest.json'
$pulseReceipt = Join-Path $resolvedOut 'cap6_authenticated_one_hour_pulse.json'
$pauseReceipt = Join-Path $resolvedOut 'cap6_final_ensure_paused.json'
if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) { throw 'CAP6 run has no native session manifest.' }
$verification = Join-Path $resolvedOut 'cap6_final_d3d_verification.json'
& $pythonExe $receiptVerifier `
    --capture-root $resolvedOut `
    --collector-dll $resolvedCollector `
    --recorder-exe $resolvedRecorder `
    --session-manifest $manifest `
    --pulse-receipt $pulseReceipt `
    --pause-receipt $pauseReceipt `
    --prebuilt-attestation $PrebuiltArtifactAttestationPath `
    --prepared-admission $preparedAdmissionPath `
    --launch-plan $planPath `
    --out $verification
if ($LASTEXITCODE -ne 0) { throw "CAP6 final-D3D verification failed; inspect $resolvedOut" }
Write-Output "CAP6 final-D3D receipt mechanically admitted: $verification"
