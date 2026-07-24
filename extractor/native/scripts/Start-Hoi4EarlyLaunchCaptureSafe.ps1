[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$Hoi4Exe,
    [Parameter(Mandatory=$true)][string]$PdxUserDir,
    [Parameter(Mandatory=$true)][string]$PreparedProductModsetDir,
    [Parameter(Mandatory=$true)][string]$OutDir,
    [ValidateSet('XN15L','Production','RawDebug')][string]$Purpose = 'XN15L',
    [double]$DurationSeconds = 300,
    [int]$MinGameHours = 1,
    [int]$SampleMilliseconds = 100,
    [string]$SessionId = '',
    [string]$CaptureId = '',
    [string]$PythonExe = 'python.exe',
    [string]$PrebuiltArtifactAttestationPath = '',
    [string]$SpeedRootGatePath = '',
    [string]$SpeedEvidenceTranscriptFileSha256 = '',
    [string]$CrashDumpTool = 'procdump.exe',
    [ValidateRange(30,900)][int]$StaticBundleTimeoutSeconds = 900,
    [ValidateRange(30,1800)][int]$WorldReadyTimeoutSeconds = 600,
    [string]$DiagnosticStagedHookMask = '',
    [switch]$KeepGameWindow,
    [switch]$OperatorApproved,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$nativeRoot = (Resolve-Path -LiteralPath (Join-Path $scriptDir '..')).Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $nativeRoot '..\..')).Path
$releaseRoot = Join-Path $repoRoot '.cargo-target\release'
$safeLateAttachDriver = Join-Path $scriptDir 'Start-Hoi4ProductionCaptureSafe.ps1'
$productModsetVerifier = Join-Path $repoRoot 'extractor\game_defs\verify_product_modset_capture_ready.py'
$staticBundleVerifier = Join-Path $repoRoot 'extractor\game_defs\static_bundle.py'
$sinkVerifier = Join-Path $scriptDir 'verify_sink_reconciliation.py'
$xn15lVerifier = Join-Path $repoRoot 'replay_interface\re_evidence\rendering_laws\XN15L\verify_xn15l_live.py'
$headlessControl = Join-Path $scriptDir 'Control-Hoi4Game.ps1'
. (Join-Path $scriptDir 'ProductionArtifact.Attestation.ps1')

function Write-DurableJson {
    param([Parameter(Mandatory=$true)][string]$Path,[Parameter(Mandatory=$true)]$Value)
    $parent=Split-Path -Parent $Path
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    $temporary="$Path.$([guid]::NewGuid().ToString('N')).tmp"
    [System.IO.File]::WriteAllText($temporary,($Value | ConvertTo-Json -Depth 20),[System.Text.UTF8Encoding]::new($false))
    $stream=[System.IO.File]::Open($temporary,[System.IO.FileMode]::Open,[System.IO.FileAccess]::ReadWrite,[System.IO.FileShare]::None)
    try { $stream.Flush($true) } finally { $stream.Dispose() }
    Move-Item -LiteralPath $temporary -Destination $Path -Force
}

function ConvertTo-NativeArgumentLine {
    param([Parameter(Mandatory=$true)][object[]]$Arguments)
    return (($Arguments | ForEach-Object {
        $value=[string]$_
        if ($value -match '[\s"]') { '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"' }
        else { $value }
    }) -join ' ')
}

function Stop-OwnedProcessBounded {
    param(
        [Parameter(Mandatory=$true)][System.Diagnostics.Process]$Process,
        [Parameter(Mandatory=$true)][ValidateRange(1000,60000)][int]$TimeoutMilliseconds,
        [Parameter(Mandatory=$true)][string]$Description
    )
    $processId=$Process.Id
    $wasRunning=-not $Process.HasExited
    if ($wasRunning) {
        try {
            $Process.Kill()
        } catch {
            $Process.Refresh()
            if (-not $Process.HasExited) {
                throw "Failed to terminate owned $Description PID $processId`: $($_.Exception.Message)"
            }
        }
    }
    if (-not $Process.WaitForExit($TimeoutMilliseconds)) {
        throw "Owned $Description PID $processId did not exit within $TimeoutMilliseconds milliseconds."
    }
    $Process.Refresh()
    if (-not $Process.HasExited) {
        throw "Owned $Description PID $processId remained live after its bounded exit wait."
    }
    return [ordered]@{
        process_id=$processId
        was_running_at_cleanup=$wasRunning
        exit_code=$Process.ExitCode
        wait_timeout_milliseconds=$TimeoutMilliseconds
        status=$(if ($wasRunning) { 'terminated_and_reaped' } else { 'already_exited_and_reaped' })
    }
}

if (-not $DryRun -and -not $OperatorApproved) {
    throw 'A live suspended launch requires explicit -OperatorApproved authority.'
}
if ($DurationSeconds -lt 30 -or $DurationSeconds -gt 86400) { throw 'DurationSeconds is outside 30..86400.' }
if ($MinGameHours -lt 1) { throw 'MinGameHours must be positive.' }
if ($SampleMilliseconds -lt 1 -or $SampleMilliseconds -gt 10000) { throw 'SampleMilliseconds is outside 1..10000.' }
$diagnosticStagedHookMaskCanonical=$null
if (-not [string]::IsNullOrWhiteSpace($DiagnosticStagedHookMask)) {
    if ($Purpose -cne 'RawDebug') {
        throw 'DiagnosticStagedHookMask is permitted only for RawDebug isolation.'
    }
    $diagnosticDigits=$DiagnosticStagedHookMask.Trim()
    if ($diagnosticDigits.StartsWith('0x',[StringComparison]::OrdinalIgnoreCase)) {
        $diagnosticDigits=$diagnosticDigits.Substring(2)
    }
    if ($diagnosticDigits -cnotmatch '^[0-9a-fA-F]{1,3}$') {
        throw 'DiagnosticStagedHookMask must be a hexadecimal staged-hook mask.'
    }
    $diagnosticMaskValue=[Convert]::ToUInt64($diagnosticDigits,16)
    if (($diagnosticMaskValue -band 0x3ff) -ne $diagnosticMaskValue -or
        ($diagnosticMaskValue -band 0x01) -eq 0 -or
        $diagnosticMaskValue -eq 0x3ff) {
        throw 'DiagnosticStagedHookMask must be a strict 0x3ff subset containing COM bit 0x01.'
    }
    $diagnosticStagedHookMaskCanonical=('0x{0:x}' -f $diagnosticMaskValue)
}
foreach($path in @($Hoi4Exe,$PdxUserDir,$PreparedProductModsetDir,$productModsetVerifier,$staticBundleVerifier,$sinkVerifier,$xn15lVerifier,$safeLateAttachDriver,$headlessControl)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required path is missing: $path" }
}
$Hoi4Exe=(Resolve-Path -LiteralPath $Hoi4Exe).Path
$PdxUserDir=(Resolve-Path -LiteralPath $PdxUserDir).Path
$PreparedProductModsetDir=(Resolve-Path -LiteralPath $PreparedProductModsetDir).Path
$observedExeSha256=(Get-FileHash -LiteralPath $Hoi4Exe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($observedExeSha256 -cne $expectedExeSha256) { throw "hoi4.exe is not exact a729 authority: $observedExeSha256" }
$continueGame=Join-Path $PdxUserDir 'continue_game.json'
if (-not (Test-Path -LiteralPath $continueGame -PathType Leaf)) { throw "Launcher continue selection is missing: $continueGame" }
$continueGameSha256=(Get-FileHash -LiteralPath $continueGame -Algorithm SHA256).Hash.ToLowerInvariant()
try { $null=Get-Content -LiteralPath $continueGame -Raw | ConvertFrom-Json } catch { throw "continue_game.json is not JSON: $($_.Exception.Message)" }

$existing=@(Get-Process -Name the game -ErrorAction SilentlyContinue)
if ($existing.Count -ne 0) {
    throw "Refusing a second game instance while the game PID(s) $(@($existing.Id) -join ',') exist; the crash monitor must bind the new instance unambiguously."
}
$OutDir=[System.IO.Path]::GetFullPath($OutDir)
if (Test-Path -LiteralPath $OutDir) {
    if (-not (Test-Path -LiteralPath $OutDir -PathType Container) -or @(Get-ChildItem -LiteralPath $OutDir -Force).Count -ne 0) {
        throw "OutDir must be absent or empty: $OutDir"
    }
} else { New-Item -ItemType Directory -Path $OutDir | Out-Null }
if ([string]::IsNullOrWhiteSpace($SessionId)) { $SessionId=[guid]::NewGuid().ToString() }
if ([string]::IsNullOrWhiteSpace($CaptureId)) { $CaptureId=[guid]::NewGuid().ToString() }
foreach($id in @($SessionId,$CaptureId)) {
    if ($id -cnotmatch '^[a-z0-9][a-z0-9_.-]{7,159}$') { throw "Session/capture id is noncanonical: $id" }
}
$python=@(Get-Command $PythonExe -CommandType Application -ErrorAction Stop)[0].Source
$procdump=(Get-Command $CrashDumpTool -CommandType Application -ErrorAction Stop).Source

if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) {
    $PrebuiltArtifactAttestationPath=Join-Path $releaseRoot 'hoi4-production-prebuilt-artifacts.json'
}
# The centralized attestation's source-input digest covers this launcher and
# its verifiers. Its v1 named driver remains the established production driver;
# the immutable plan below additionally pins this early-launch driver itself.
$prebuilt=Test-ProductionArtifactAttestation -RepoRoot $repoRoot -ReceiptPath $PrebuiltArtifactAttestationPath -ExpectedDriverPath $safeLateAttachDriver
$recorder=[string]$prebuilt.artifacts.recorder_host
$collector=[string]$prebuilt.artifacts.collector_dll

$admissionPath=Join-Path $OutDir 'prepared_product_modset_admission.json'
& $python $productModsetVerifier --exe $Hoi4Exe --prepared $PreparedProductModsetDir --out $admissionPath
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $admissionPath -PathType Leaf)) { throw 'Prepared product-modset admission failed.' }
$admission=Get-Content -LiteralPath $admissionPath -Raw | ConvertFrom-Json
if ([string]$admission.schema -cne 'extractor.product_modset_capture_admission.v1' -or [string]$admission.verdict -cne 'passed') {
    throw 'Prepared product-modset admission did not emit a passing canonical receipt.'
}

$effectiveSampleMilliseconds=if ($Purpose -ceq 'RawDebug') { 1 } else { $SampleMilliseconds }
$arguments=@(
    '--early-launch-exe',$Hoi4Exe,
    '--early-launch-arg','--continuelastsave',
    '--early-launch-arg','--gdpr-compliant',
    '--early-hook-timeout-ms',([string]([Math]::Min($WorldReadyTimeoutSeconds,1800) * 1000)),
    '--pdx-user-dir',$PdxUserDir,
    '--dll',$collector,
    '--out-dir',$OutDir,
    '--profile','production',
    '--stream-format','compact',
    '--duration-s',([string]::Format([Globalization.CultureInfo]::InvariantCulture,'{0:0.###}',$DurationSeconds)),
    '--sample-ms',[string]$effectiveSampleMilliseconds,
    '--min-game-hours',[string]$MinGameHours,
    '--session-id',$SessionId,
    '--capture-id',$CaptureId,
    '--hour-latch',
    '--static-bundle-manifest',[string]$admission.paths.static_bundle_manifest,
    '--static-bundle-mod-state',[string]$admission.paths.mod_state,
    '--static-bundle-dlc-state',[string]$admission.paths.dlc_state,
    '--static-bundle-load-order',[string]$admission.paths.load_order,
    '--static-bundle-source-inventory',[string]$admission.paths.source_inventory,
    '--static-bundle-live-resolution-report',[string]$admission.paths.resolution_report,
    '--static-bundle-verifier',$staticBundleVerifier,
    '--static-bundle-python',$python,
    '--static-bundle-timeout-ms',([string]($StaticBundleTimeoutSeconds * 1000))
)
if ($KeepGameWindow) {
    # Window presentation is independent of the PID-bound pause/play/speed
    # controller. The default remains compatible with historical minimized
    # captures; this option leaves the game's normal shown window alone.
    $arguments += '--early-launch-keep-window'
}
if ($Purpose -ceq 'XN15L') {
    # A bounded render-transaction proof deliberately does not claim the full
    # D57 production gate. It still exercises the production H4CT and d77 path.
    $arguments += @('--scheduled-domains-exact','--scheduled-domain','session_header_live')
} elseif ($Purpose -ceq 'Production') {
    if (-not (Test-Path -LiteralPath $SpeedRootGatePath -PathType Leaf) -or $SpeedEvidenceTranscriptFileSha256 -cnotmatch '^[0-9a-f]{64}$') {
        throw 'Full Production mode requires a same-process D57 gate path and raw transcript file SHA-256.'
    }
    $arguments += @(
        '--domain57-game-speed-gate-v2',(Resolve-Path -LiteralPath $SpeedRootGatePath).Path,
        '--domain57-game-speed-transcript-file-sha256',$SpeedEvidenceTranscriptFileSha256
    )
} else {
    # RawDebug uses the complete production roster and the exact-build passive
    # D57 observer, but stops at sealed H4CR plus integrity/error telemetry.
    # Early H4CT/content authority is still required so one-shot render/UI
    # sources cannot be missed.
    $arguments += @(
        '--stream-compression','none',
        '--raw-debug-output'
    )
}
if ($null -ne $diagnosticStagedHookMaskCanonical) {
    # Bind host validation and child hook selection through the recorder's one
    # diagnostic authority. The recorder constructs the suspended child's
    # environment block; callers must not inject an unbound environment-only mask.
    $arguments += @(
        '--production',
        '--scheduled-domains-exact',
        '--scheduled-domain','session_header_live',
        '--diagnostic-staged-hook-mask',$diagnosticStagedHookMaskCanonical,
        '--diagnostic-no-commit'
    )
}

$planPath=Join-Path $OutDir 'early_launch_authorization_plan.json'
$plan=[ordered]@{
    schema='battle-plan.early_launch_authorization.v1'
    purpose=$Purpose
    operator_approved=[bool]$OperatorApproved
    dry_run=[bool]$DryRun
    attempt_budget=1
    session_id=$SessionId
    capture_id=$CaptureId
    executable_sha256=$observedExeSha256
    collector_sha256=(Get-FileHash -LiteralPath $collector -Algorithm SHA256).Hash.ToLowerInvariant()
    recorder_sha256=(Get-FileHash -LiteralPath $recorder -Algorithm SHA256).Hash.ToLowerInvariant()
    launcher_sha256=(Get-FileHash -LiteralPath $MyInvocation.MyCommand.Path -Algorithm SHA256).Hash.ToLowerInvariant()
    prebuilt_attestation_sha256=[string]$prebuilt.receipt_sha256
    crash_dump_tool_sha256=(Get-FileHash -LiteralPath $procdump -Algorithm SHA256).Hash.ToLowerInvariant()
    prepared_admission_sha256=(Get-FileHash -LiteralPath $admissionPath -Algorithm SHA256).Hash.ToLowerInvariant()
    product_modset_sha256=[string]$admission.product_modset_sha256
    continue_game_sha256=$continueGameSha256
    semantic_arguments=@('--continuelastsave','--gdpr-compliant')
    diagnostic_staged_hook_mask=$diagnosticStagedHookMaskCanonical
    keep_game_window=[bool]$KeepGameWindow
    static_bundle_timeout_seconds=$StaticBundleTimeoutSeconds
    post_static_world_ready_timeout_seconds=$WorldReadyTimeoutSeconds
    protected_existing_hoi4_process_ids=@($existing | ForEach-Object { $_.Id })
    recorder_arguments=$arguments
    launch_blocker=$(if ($Purpose -ceq 'Production') { 'same_process_d57_gate_must_be_created_after_internal_suspended_process_creation' } else { $null })
}
Write-DurableJson -Path $planPath -Value $plan
if ($DryRun) {
    Write-Host "Dry-run early-launch plan retained without starting the game: $planPath"
    return
}
if ($Purpose -ceq 'Production') {
    throw 'Full Production launch is fail-closed: the D57 owner must add same-process gate orchestration after recorder-owned suspended creation. No the game process was started.'
}

$stdoutPath=Join-Path $OutDir 'collector_record.stdout.log'
$stderrPath=Join-Path $OutDir 'collector_record.stderr.log'
$crashDir=Join-Path $OutDir 'crash_evidence'
New-Item -ItemType Directory -Path $crashDir -Force | Out-Null
$procdumpStdout=Join-Path $crashDir 'procdump.stdout.log'
$procdumpStderr=Join-Path $crashDir 'procdump.stderr.log'
$dumpMonitor=$null
$process=$null
$stdout=$null
$stderr=$null
$recorderExitCode=$null
$rawRecorderExitNonzero=$false
$headlessControlResult=$null
$headlessControlError=$null
$dumpMonitorCleanup=$null
$dumpMonitorCleanupError=$null
$dumpMonitorStopTimeoutMilliseconds=15000
try {
    $dumpMonitor=Start-Process -FilePath $procdump -ArgumentList @(
        '-accepteula','-ma','-e','-w','hoi4.exe',$crashDir
    ) -PassThru -WindowStyle Hidden -RedirectStandardOutput $procdumpStdout -RedirectStandardError $procdumpStderr
    $start=[System.Diagnostics.ProcessStartInfo]::new()
    $start.FileName=$recorder
    $start.UseShellExecute=$false
    $start.CreateNoWindow=$true
    $start.RedirectStandardOutput=$true
    $start.RedirectStandardError=$true
    if ($null -ne $start.PSObject.Properties['ArgumentList']) {
        foreach($argument in $arguments) { [void]$start.ArgumentList.Add([string]$argument) }
    } else { $start.Arguments=ConvertTo-NativeArgumentLine $arguments }
    $process=[System.Diagnostics.Process]::new();$process.StartInfo=$start
    if (-not $process.Start()) { throw 'collector_record process start returned false' }
    $stdout=$process.StandardOutput.ReadToEndAsync();$stderr=$process.StandardError.ReadToEndAsync()
    if ($Purpose -ceq 'RawDebug') {
        # The recorder verifies the complete static bundle while the game remains
        # suspended. That independent bounded phase must not consume the
        # subsequent world-readiness budget used by the headless controller.
        $headlessStartupBudgetSeconds=$StaticBundleTimeoutSeconds + $WorldReadyTimeoutSeconds
        $worldDeadline=[DateTime]::UtcNow.AddSeconds($headlessStartupBudgetSeconds)
        while (-not $process.HasExited -and [DateTime]::UtcNow -lt $worldDeadline) {
            $gameCandidates=@(Get-Process -Name the game -ErrorAction SilentlyContinue)
            if ($gameCandidates.Count -eq 1) {
                try {
                    $candidateResult=& $headlessControl -TargetPid $gameCandidates[0].Id -Speed 5 -Play
                    if (
                        $null -ne $candidateResult -and
                        [int]$candidateResult.Pid -eq [int]$gameCandidates[0].Id -and
                        [int]$candidateResult.Speed -eq 5 -and
                        [string]$candidateResult.Action -ceq 'Play'
                    ) {
                        $headlessControlResult=$candidateResult
                        break
                    }
                } catch {
                    # The exact process exists before CGameState/CInGameIdler.
                    # Retry until the authenticated world singletons become
                    # stable; no window activation or foreground input occurs.
                    $headlessControlError=$_.Exception.Message
                }
            }
            Start-Sleep -Milliseconds 500
            $process.Refresh()
        }
        if ($null -eq $headlessControlResult) {
            if ($process.HasExited) {
                throw "Recorder exited before headless world-ready control completed (exit $($process.ExitCode))."
            }
            throw "the game did not expose authenticated world-ready pause/speed roots within the $headlessStartupBudgetSeconds-second startup budget (static verification up to $StaticBundleTimeoutSeconds seconds plus $WorldReadyTimeoutSeconds seconds for world readiness). Last control error: $headlessControlError"
        }
        Write-DurableJson -Path (Join-Path $OutDir 'raw_debug_headless_control.json') -Value ([ordered]@{
            schema='battle-plan.raw_debug_headless_control.v1'
            process_id=[int]$headlessControlResult.Pid
            executable_sha256=[string]$headlessControlResult.ExeSha256
            action=[string]$headlessControlResult.Action
            speed=[int]$headlessControlResult.Speed
            paused=[bool]$headlessControlResult.Paused
            hour_a=[int]$headlessControlResult.HourA
            hour_b=[int]$headlessControlResult.HourB
            advancing=[bool]$headlessControlResult.Advancing
            controlled_utc=[DateTime]::UtcNow.ToString('o')
        })
    }
    $process.WaitForExit()
    $recorderExitCode=$process.ExitCode
    [System.IO.File]::WriteAllText($stdoutPath,[string]$stdout.Result,[System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText($stderrPath,[string]$stderr.Result,[System.Text.UTF8Encoding]::new($false))
    if ($process.ExitCode -ne 0) {
        if ($Purpose -ceq 'RawDebug') {
            # Capture-integrity/error telemetry is the raw-debug deliverable
            # even when a domain gate fails. Continue to hash/rebind the H4CR,
            # then return a failing result after the evidence is durable.
            $rawRecorderExitNonzero=$true
        } else {
            throw "collector_record exited with code $($process.ExitCode); retained logs are in $OutDir"
        }
    }
} finally {
    if ($null -ne $process) {
        if (-not $process.HasExited) {
            try { $process.Kill() } catch {}
            try { [void]$process.WaitForExit(10000) } catch {}
        }
        if ($process.HasExited) {
            if ($null -ne $stdout) {
                try { [System.IO.File]::WriteAllText($stdoutPath,[string]$stdout.Result,[System.Text.UTF8Encoding]::new($false)) } catch {}
            }
            if ($null -ne $stderr) {
                try { [System.IO.File]::WriteAllText($stderrPath,[string]$stderr.Result,[System.Text.UTF8Encoding]::new($false)) } catch {}
            }
        }
        $process.Dispose()
    }
    if ($null -ne $dumpMonitor) {
        try {
            $dumpMonitorCleanup=Stop-OwnedProcessBounded `
                -Process $dumpMonitor `
                -TimeoutMilliseconds $dumpMonitorStopTimeoutMilliseconds `
                -Description 'ProcDump crash monitor'
        } catch {
            $dumpMonitorCleanupError=$_.Exception.Message
        } finally {
            # ProcDump owns the redirected log handles until it exits. Closing
            # our process handle only after the bounded reap prevents the
            # subsequent evidence hash pass from racing those writers.
            $dumpMonitor.Dispose()
        }
    }
    $retainedLogs=Join-Path $OutDir 'game_logs_after_attempt'
    New-Item -ItemType Directory -Path $retainedLogs -Force | Out-Null
    foreach($name in @('error.log','game.log','system.log','setup.log')) {
        $source=Join-Path $PdxUserDir "logs\$name"
        if (Test-Path -LiteralPath $source -PathType Leaf) { Copy-Item -LiteralPath $source -Destination (Join-Path $retainedLogs $name) -Force }
    }
    if ($null -ne $dumpMonitorCleanupError) {
        $cleanupFailurePath=Join-Path $crashDir 'procdump_cleanup_failure.json'
        Write-DurableJson -Path $cleanupFailurePath -Value ([ordered]@{
            schema='battle-plan.procdump_cleanup_failure.v1'
            error=[string]$dumpMonitorCleanupError
            wait_timeout_milliseconds=$dumpMonitorStopTimeoutMilliseconds
            failed_utc=[DateTime]::UtcNow.ToString('o')
            evidence_preserved=$true
            crash_artifacts_hashed=$false
        })
        throw "ProcDump cleanup failed closed before crash-evidence hashing; evidence was preserved and details are in $cleanupFailurePath"
    }
    $crashFiles=@()
    foreach($file in @(Get-ChildItem -LiteralPath $crashDir -File -ErrorAction SilentlyContinue | Sort-Object Name)) {
        $crashFiles += [ordered]@{
            file=$file.Name
            bytes=[int64]$file.Length
            sha256=(Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    Write-DurableJson -Path (Join-Path $crashDir 'attempt_evidence_manifest.json') -Value ([ordered]@{
        schema='battle-plan.early_launch_crash_evidence.v1'
        recorder_exit_code=$recorderExitCode
        crash_monitor_cleanup=$dumpMonitorCleanup
        process_dump_count=@($crashFiles | Where-Object { [string]$_.file -like '*.dmp' }).Count
        artifacts=$crashFiles
    })
}

$manifestPath=Join-Path $OutDir 'native_session_manifest.json'
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { throw 'Recorder succeeded without a native session manifest.' }
$sessionManifest=Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
$resolvedAttemptPath=Join-Path $OutDir 'early_launch_resolved_attempt.json'
Write-DurableJson -Path $resolvedAttemptPath -Value ([ordered]@{
    schema='battle-plan.early_launch_resolved_attempt.v1'
    authorization_plan_sha256=(Get-FileHash -LiteralPath $planPath -Algorithm SHA256).Hash.ToLowerInvariant()
    native_session_manifest_sha256=(Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
    session_id=[string]$sessionManifest.session_id
    capture_id=[string]$sessionManifest.capture_id
    process_id=[int]$sessionManifest.early_render_capture_receipt.process_id
    process_start_nonce=[uint64]$sessionManifest.early_render_capture_receipt.process_start_nonce
    early_render_capture_receipt_sha256=[string]$sessionManifest.early_render_capture_receipt_sha256
    product_modset_sha256=[string]$sessionManifest.portable_content_manifest.product_modset_sha256
})

$verificationPath=Join-Path $OutDir $(if ($Purpose -ceq 'XN15L') { 'xn15l_live_verification.json' } elseif ($Purpose -ceq 'RawDebug') { 'raw_debug_early_launch_integrity.json' } else { 'sink_reconciliation_verification.json' })
if ($Purpose -ceq 'XN15L') {
    & $python $xn15lVerifier --run-dir $OutDir --out $verificationPath
} elseif ($Purpose -ceq 'Production') {
    & $python $sinkVerifier --run-dir $OutDir --expect success --out $verificationPath
} else {
    $rawReceiptPath=Join-Path $OutDir 'raw_debug_output.json'
    if (-not (Test-Path -LiteralPath $rawReceiptPath -PathType Leaf)) {
        throw 'Raw-debug early launch emitted no raw_debug_output.json integrity receipt.'
    }
    $rawReceipt=Get-Content -LiteralPath $rawReceiptPath -Raw | ConvertFrom-Json
    if (
        [string]$rawReceipt.schema -cne 'battle-plan.raw_debug_output.v2' -or
        [bool]$rawReceipt.admissible_for_production -or
        [bool]$rawReceipt.container_emitted -or
        [bool]$rawReceipt.container_compression_applied -or
        -not [bool]$rawReceipt.stream.sealed -or
        [string]$rawReceipt.stream.compression -cne 'none' -or
        [bool]$rawReceipt.derived_artifacts.host_fold_performed -or
        [bool]$rawReceipt.derived_artifacts.hour_index_h4ix_built -or
        [bool]$rawReceipt.derived_artifacts.checkpoint_cache_h4cp_built -or
        [bool]$rawReceipt.derived_artifacts.reconstruction_parity_run -or
        [bool]$rawReceipt.derived_artifacts.bounded_seek_certification_run -or
        [bool]$rawReceipt.derived_artifacts.replay_container_built -or
        [bool]$rawReceipt.derived_artifacts.compression_run
    ) {
        throw 'Raw-debug early launch receipt violates the direct-H4CR-only contract.'
    }
    $relativeStream=[string]$rawReceipt.stream.path
    $rawStreamPath=[System.IO.Path]::GetFullPath((Join-Path $OutDir $relativeStream))
    $outRootWithSeparator=[System.IO.Path]::GetFullPath($OutDir).TrimEnd('\') + '\'
    if (-not $rawStreamPath.StartsWith($outRootWithSeparator,[StringComparison]::OrdinalIgnoreCase)) {
        throw 'Raw-debug H4CR receipt escapes the capture directory.'
    }
    if (-not (Test-Path -LiteralPath $rawStreamPath -PathType Leaf)) {
        throw "Raw-debug H4CR is missing: $rawStreamPath"
    }
    $rawItem=Get-Item -LiteralPath $rawStreamPath
    $rawSha256=(Get-FileHash -LiteralPath $rawStreamPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if (
        [int64]$rawReceipt.stream.bytes -ne [int64]$rawItem.Length -or
        [string]$rawReceipt.stream.sha256 -cne $rawSha256 -or
        -not [bool]$rawReceipt.capture_integrity.sha256_matches_capture_receipt
    ) {
        throw 'Raw-debug H4CR bytes/hash differ from the capture integrity receipt.'
    }
    if (
        -not [bool]$sessionManifest.early_render_capture_receipt.created_suspended -or
        [uint64]$sessionManifest.early_render_capture_receipt.preexisting_game_d3d_object_count -ne 0 -or
        [string]$sessionManifest.early_render_launch_manifest.semantic_arguments[0] -cne '--continuelastsave' -or
        [string]$sessionManifest.early_render_launch_manifest.semantic_arguments[1] -cne '--gdpr-compliant' -or
        $null -eq $sessionManifest.portable_content_manifest -or
        $null -eq $sessionManifest.raw_debug_pre_resume_static_bundle
    ) {
        throw 'Raw-debug capture lost its authenticated suspended-launch/content authority.'
    }
    Write-DurableJson -Path $verificationPath -Value ([ordered]@{
        schema='battle-plan.raw_debug_early_launch_integrity.v1'
        admissible_for_production=$false
        direct_h4cr_only=$true
        process_id=[int]$sessionManifest.early_render_capture_receipt.process_id
        created_suspended=$true
        preexisting_game_d3d_object_count=0
        semantic_arguments=@('--continuelastsave','--gdpr-compliant')
        h4cr_file=$relativeStream
        h4cr_bytes=[int64]$rawItem.Length
        h4cr_sha256=$rawSha256
        headless_speed_play_receipt='raw_debug_headless_control.json'
        derived_artifacts_emitted=$false
    })
}
if ($Purpose -cne 'RawDebug' -and $LASTEXITCODE -ne 0) {
    throw "Post-capture mechanical regrade failed; inspect $verificationPath"
}
if ($Purpose -ceq 'RawDebug') {
    Write-Host "Early-launch raw-debug H4CR sealed and hash-verified without replay certification: $verificationPath"
    if ($rawRecorderExitNonzero -or -not [bool]$rawReceipt.error_telemetry.capture_gates_met) {
        throw "Raw-debug capture retained a hash-verified H4CR with capture errors; inspect $rawReceiptPath and $verificationPath"
    }
} else {
    Write-Host "Early-launch capture mechanically admitted: $verificationPath"
}
