[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$Hoi4Exe,
    [Parameter(Mandatory=$true)][string]$PdxUserDir,
    [Parameter(Mandatory=$true)][string]$PreparedProductModsetDir,
    [Parameter(Mandatory=$true)][string]$OutDir,
    [ValidateSet('XN15L','Production')][string]$Purpose = 'XN15L',
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

if (-not $DryRun -and -not $OperatorApproved) {
    throw 'A live suspended launch requires explicit -OperatorApproved authority.'
}
if ($DurationSeconds -lt 30 -or $DurationSeconds -gt 86400) { throw 'DurationSeconds is outside 30..86400.' }
if ($MinGameHours -lt 1) { throw 'MinGameHours must be positive.' }
if ($SampleMilliseconds -lt 1 -or $SampleMilliseconds -gt 10000) { throw 'SampleMilliseconds is outside 1..10000.' }
foreach($path in @($Hoi4Exe,$PdxUserDir,$PreparedProductModsetDir,$productModsetVerifier,$staticBundleVerifier,$sinkVerifier,$xn15lVerifier,$safeLateAttachDriver)) {
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
$python=(Get-Command $PythonExe -CommandType Application -ErrorAction Stop).Source
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

$arguments=@(
    '--early-launch-exe',$Hoi4Exe,
    '--early-launch-arg','--continuelastsave',
    '--early-launch-arg','--gdpr-compliant',
    '--pdx-user-dir',$PdxUserDir,
    '--dll',$collector,
    '--out-dir',$OutDir,
    '--profile','production',
    '--stream-format','compact',
    '--duration-s',([string]::Format([Globalization.CultureInfo]::InvariantCulture,'{0:0.###}',$DurationSeconds)),
    '--sample-ms',[string]$SampleMilliseconds,
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
    '--static-bundle-timeout-ms','300000'
)
if ($Purpose -ceq 'XN15L') {
    # A bounded render-transaction proof deliberately does not claim the full
    # D57 production gate. It still exercises the production H4CT and d77 path.
    $arguments += @('--scheduled-domains-exact','--scheduled-domain','session_header_live')
} else {
    if (-not (Test-Path -LiteralPath $SpeedRootGatePath -PathType Leaf) -or $SpeedEvidenceTranscriptFileSha256 -cnotmatch '^[0-9a-f]{64}$') {
        throw 'Full Production mode requires a same-process D57 gate path and raw transcript file SHA-256.'
    }
    $arguments += @(
        '--domain57-game-speed-gate-v2',(Resolve-Path -LiteralPath $SpeedRootGatePath).Path,
        '--domain57-game-speed-transcript-file-sha256',$SpeedEvidenceTranscriptFileSha256
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
    protected_existing_hoi4_process_ids=@($existing.Id)
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
$recorderExitCode=$null
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
    $process.WaitForExit()
    $recorderExitCode=$process.ExitCode
    [System.IO.File]::WriteAllText($stdoutPath,[string]$stdout.Result,[System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText($stderrPath,[string]$stderr.Result,[System.Text.UTF8Encoding]::new($false))
    if ($process.ExitCode -ne 0) { throw "collector_record exited with code $($process.ExitCode); retained logs are in $OutDir" }
} finally {
    if ($null -ne $process) { $process.Dispose() }
    if ($null -ne $dumpMonitor) {
        if (-not $dumpMonitor.HasExited) { Stop-Process -Id $dumpMonitor.Id -Force -ErrorAction SilentlyContinue }
        $dumpMonitor.Dispose()
    }
    $retainedLogs=Join-Path $OutDir 'game_logs_after_attempt'
    New-Item -ItemType Directory -Path $retainedLogs -Force | Out-Null
    foreach($name in @('error.log','game.log','system.log','setup.log')) {
        $source=Join-Path $PdxUserDir "logs\$name"
        if (Test-Path -LiteralPath $source -PathType Leaf) { Copy-Item -LiteralPath $source -Destination (Join-Path $retainedLogs $name) -Force }
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

$verificationPath=Join-Path $OutDir $(if ($Purpose -ceq 'XN15L') { 'xn15l_live_verification.json' } else { 'sink_reconciliation_verification.json' })
if ($Purpose -ceq 'XN15L') {
    & $python $xn15lVerifier --run-dir $OutDir --out $verificationPath
} else {
    & $python $sinkVerifier --run-dir $OutDir --expect success --out $verificationPath
}
if ($LASTEXITCODE -ne 0) { throw "Post-capture mechanical regrade failed; inspect $verificationPath" }
Write-Host "Early-launch capture mechanically admitted: $verificationPath"
