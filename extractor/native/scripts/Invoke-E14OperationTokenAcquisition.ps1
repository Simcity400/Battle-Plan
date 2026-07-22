[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$RecorderExe,
    [Parameter(Mandatory = $true)][string]$StreamFoldExe,
    [Parameter(Mandatory = $true)][string]$Hoi4Exe,
    [Parameter(Mandatory = $true)][string]$PdxUserDir,
    [Parameter(Mandatory = $true)][string]$CollectorDll,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [Parameter(Mandatory = $true)][string]$ContinueSave,
    [Parameter(Mandatory = $true)][string]$SaveOracle,
    [Parameter(Mandatory = $true)][string]$StaticBundleManifest,
    [Parameter(Mandatory = $true)][string]$StaticBundleBindingsDir,
    [string]$PrebuiltArtifactAttestationPath = '',
    [string]$Python = 'python',
    [int]$DurationSeconds = 240,
    [int]$SampleMilliseconds = 25,
    [int]$GameLoadTimeoutSeconds = 300,
    [int]$FixtureClientX = 960,
    [int]$FixtureClientY = 540,
    [string]$ExpectedContinueSaveSha256 = 'c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735',
    [string]$ExpectedSaveOracleSha256 = 'cb5b93ef8877e710f834f55924a2311e41a52e6f6664668ed0ce0de59d9ba5e4',
    [switch]$EnableLegacyClientDriving
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $EnableLegacyClientDriving) {
    throw 'This legacy owner launches, inputs to, and controls HOI4. It is disabled by default. Use E14_TOKENS/PASSIVE_OBSERVER_WORKFLOW.json with the shared passive observer; only an explicit -EnableLegacyClientDriving opt-in may enter this quarantined path.'
}

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$domain = 'diplomacy_intel_operatives_live'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$productionDriver = Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1'
$attestationHelpers = Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1'
$staticBundleVerifier = Join-Path $repo 'extractor\game_defs\static_bundle.py'
$productModsetVerifier = Join-Path $repo 'extractor\game_defs\verify_product_modset_capture_ready.py'
$readinessHelper = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_wait_for_paused_save.ps1'
$actionHelper = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_strong_key_hover.ps1'
$regrader = Join-Path $repo 'extractor\re_evidence\E14_TOKENS\regrade_operation_token_census.py'
$bindingNames = @('mod_state.json', 'dlc_state.json', 'load_order.json', 'source_inventory.json', 'resolution_report.json')

function ConvertTo-NativeArgumentLine {
    param([Parameter(Mandatory = $true)][object[]]$Arguments)
    (($Arguments | ForEach-Object {
        $value = [string]$_
        if ($value -match '[\s"]') { '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"' } else { $value }
    }) -join ' ')
}

function Write-AtomicBytes {
    param([string]$Path, [byte[]]$Bytes, [switch]$AllowReplace)
    $fullPath = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($fullPath)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllBytes($temporary, $Bytes)
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($fullPath)) {
            if (-not $AllowReplace) { throw "Refusing to replace immutable file: $fullPath" }
            [IO.File]::Replace($temporary, $fullPath, $null, $true)
        } else {
            [IO.File]::Move($temporary, $fullPath)
        }
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

function Write-AtomicText {
    param([string]$Path, [string]$Text, [switch]$AllowReplace)
    Write-AtomicBytes -Path $Path -Bytes ([Text.UTF8Encoding]::new($false).GetBytes($Text)) -AllowReplace:$AllowReplace
}

function Write-AtomicJson {
    param([string]$Path, $Value, [switch]$AllowReplace)
    Write-AtomicText -Path $Path -Text (($Value | ConvertTo-Json -Depth 30) + "`n") -AllowReplace:$AllowReplace
}

function Test-IsWithinPath {
    param([string]$Candidate, [string]$Root)
    $candidateFull = [IO.Path]::GetFullPath($Candidate).TrimEnd([char]'\')
    $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd([char]'\')
    $candidateFull.Equals($rootFull, [StringComparison]::OrdinalIgnoreCase) -or
        $candidateFull.StartsWith($rootFull + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)
}

function Start-HiddenNative {
    param([string]$Executable, [object[]]$Arguments)
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
    if (-not $process.Start()) { throw "Failed to start $Executable" }
    [pscustomobject]@{
        Process = $process
        StdoutTask = $process.StandardOutput.ReadToEndAsync()
        StderrTask = $process.StandardError.ReadToEndAsync()
    }
}

if ($DurationSeconds -lt 120 -or $DurationSeconds -gt 1800) { throw 'DurationSeconds must be in 120..1800.' }
if ($SampleMilliseconds -lt 1 -or $SampleMilliseconds -gt 1000) { throw 'SampleMilliseconds must be in 1..1000.' }
if ($GameLoadTimeoutSeconds -lt 30 -or $GameLoadTimeoutSeconds -gt 1800) { throw 'GameLoadTimeoutSeconds must be in 30..1800.' }
if ($FixtureClientX -lt 0 -or $FixtureClientY -lt 0) { throw 'Fixture client coordinates must be non-negative.' }
foreach ($hash in @($ExpectedContinueSaveSha256, $ExpectedSaveOracleSha256)) {
    if ($hash -cnotmatch '^[0-9a-f]{64}$') { throw 'Expected identities must be lowercase SHA-256.' }
}
foreach ($path in @(
    $RecorderExe, $StreamFoldExe, $Hoi4Exe, $PdxUserDir, $CollectorDll, $ContinueSave, $SaveOracle,
    $StaticBundleManifest, $StaticBundleBindingsDir, $productionDriver, $attestationHelpers,
    $staticBundleVerifier, $productModsetVerifier, $readinessHelper, $actionHelper, $regrader
)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required path is absent: $path" }
}
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
    throw 'E14 requires one clean early launch and refuses all existing hoi4.exe processes.'
}

$resolvedRecorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedStreamFold = (Resolve-Path -LiteralPath $StreamFoldExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedSave = (Resolve-Path -LiteralPath $ContinueSave).Path
$resolvedOracle = (Resolve-Path -LiteralPath $SaveOracle).Path
$resolvedBundle = (Resolve-Path -LiteralPath $StaticBundleManifest).Path
$resolvedBindings = (Resolve-Path -LiteralPath $StaticBundleBindingsDir).Path
$resolvedOut = [IO.Path]::GetFullPath($OutDir)
$pythonExe = (Get-Command $Python -CommandType Application -ErrorAction Stop).Source
if (Test-IsWithinPath $resolvedOut $repo) { throw 'OutDir must be outside the repository.' }
if (Test-IsWithinPath $resolvedOut $resolvedUserDir) { throw 'OutDir must be outside the game user directory.' }
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty: $resolvedOut"
    }
} else { [IO.Directory]::CreateDirectory($resolvedOut) | Out-Null }

$saveDirectory = [IO.Path]::GetFullPath((Join-Path $resolvedUserDir 'save games')).TrimEnd([char]'\')
if (-not [IO.Path]::GetFullPath((Split-Path -Parent $resolvedSave)).TrimEnd([char]'\').Equals($saveDirectory, [StringComparison]::OrdinalIgnoreCase)) {
    throw "ContinueSave must be a direct child of $saveDirectory"
}
$exeSha = (Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant()
$saveSha = (Get-FileHash -LiteralPath $resolvedSave -Algorithm SHA256).Hash.ToLowerInvariant()
$oracleSha = (Get-FileHash -LiteralPath $resolvedOracle -Algorithm SHA256).Hash.ToLowerInvariant()
if ($exeSha -cne $expectedExeSha256) { throw "hoi4.exe exact-a729 mismatch: $exeSha" }
if ($saveSha -cne $ExpectedContinueSaveSha256) { throw "ContinueSave identity mismatch: $saveSha" }
if ($oracleSha -cne $ExpectedSaveOracleSha256) { throw "SaveOracle identity mismatch: $oracleSha" }
$oracleObject = Get-Content -LiteralPath $resolvedOracle -Raw | ConvertFrom-Json
if ([string]$oracleObject.meta.source_filename -cne [IO.Path]::GetFileName($resolvedSave)) { throw 'SaveOracle source filename differs from ContinueSave.' }
if ([string]$oracleObject.meta.save_version -notmatch '1\.19\.2\.0\.a729') { throw 'SaveOracle is not exact a729.' }

$bindingPaths = [ordered]@{}
foreach ($leaf in $bindingNames) {
    $path = Join-Path $resolvedBindings $leaf
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Static-bundle binding is absent: $path" }
    $bindingPaths[$leaf] = (Resolve-Path -LiteralPath $path).Path
}
. $attestationHelpers
if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) {
    $PrebuiltArtifactAttestationPath = Join-Path $repo '.cargo-target\release\hoi4-production-prebuilt-artifacts.json'
}
$prebuilt = Test-ProductionArtifactAttestation -RepoRoot $repo -ReceiptPath $PrebuiltArtifactAttestationPath -ExpectedDriverPath $productionDriver
$resolvedAttestation = (Resolve-Path -LiteralPath ([string]$prebuilt.receipt_path)).Path
if (-not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.recorder_host).Equals($resolvedRecorder, [StringComparison]::OrdinalIgnoreCase) -or
    -not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.collector_dll).Equals($resolvedCollector, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'E14 artifacts differ from the centralized Release attestation.'
}

$preparedRoot = [IO.Path]::GetFullPath((Split-Path -Parent $resolvedBindings))
$preparedAdmissionPath = Join-Path $resolvedOut 'prepared_product_modset_admission.json'
& $pythonExe $productModsetVerifier --exe $resolvedExe --prepared $preparedRoot --out $preparedAdmissionPath
if ($LASTEXITCODE -ne 0) { throw 'Prepared product-modset admission failed.' }
$prepared = Get-Content -LiteralPath $preparedAdmissionPath -Raw | ConvertFrom-Json
if ([string]$prepared.verdict -cne 'passed' -or -not [IO.Path]::GetFullPath([string]$prepared.paths.static_bundle_manifest).Equals($resolvedBundle, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Prepared product-modset admission does not bind the requested static bundle.'
}
$bundleObject = Get-Content -LiteralPath $resolvedBundle -Raw | ConvertFrom-Json
$productModsetSha = [string]$bundleObject.bindings.product_modset.product_modset_sha256
if ($productModsetSha -cnotmatch '^[0-9a-f]{64}$' -or $productModsetSha -cne [string]$prepared.product_modset_sha256) {
    throw 'Prepared and source product-modset identities differ.'
}

$staticPreflightPath = Join-Path $resolvedOut 'e14_static_bundle_preflight.json'
$preflightOutput = @(& $pythonExe $staticBundleVerifier verify --bundle (Split-Path -Parent $resolvedBundle) --exe $resolvedExe `
    --mod-state $bindingPaths['mod_state.json'] --dlc-state $bindingPaths['dlc_state.json'] `
    --load-order $bindingPaths['load_order.json'] --source-inventory $bindingPaths['source_inventory.json'] `
    --live-resolution-report $bindingPaths['resolution_report.json'] --allow-deferred-token-gate)
if ($LASTEXITCODE -ne 0) { throw 'Static-bundle pre-injection verification failed.' }
$preflightText = ($preflightOutput -join [Environment]::NewLine) + [Environment]::NewLine
$preflight = $preflightText | ConvertFrom-Json
if ($preflight.passed -ne $true -or [string]$preflight.verification_phase -cne 'pre_injection_deferred_allowed') { throw 'Static preflight did not admit deferred token binding.' }
Write-AtomicText -Path $staticPreflightPath -Text $preflightText

$sessionId = [guid]::NewGuid().ToString()
$captureId = [guid]::NewGuid().ToString()
$arguments = @(
    '--early-launch-exe', $resolvedExe, '--early-launch-arg', '--continuelastsave', '--early-launch-arg', '--gdpr-compliant',
    '--pdx-user-dir', $resolvedUserDir, '--dll', $resolvedCollector, '--out-dir', $resolvedOut,
    '--session-id', $sessionId, '--capture-id', $captureId, '--duration-s', [string]$DurationSeconds,
    '--sample-ms', [string]$SampleMilliseconds, '--min-game-hours', '0', '--stream-format', 'compact',
    '--stream-compression', 'none', '--profile', 'debug', '--scheduled-domain', $domain, '--scheduled-domains-exact',
    '--static-bundle-manifest', $resolvedBundle, '--static-bundle-mod-state', $bindingPaths['mod_state.json'],
    '--static-bundle-dlc-state', $bindingPaths['dlc_state.json'], '--static-bundle-load-order', $bindingPaths['load_order.json'],
    '--static-bundle-source-inventory', $bindingPaths['source_inventory.json'],
    '--static-bundle-live-resolution-report', $bindingPaths['resolution_report.json'], '--static-bundle-verifier', $staticBundleVerifier,
    '--static-bundle-python', $pythonExe, '--static-bundle-timeout-ms', '300000', '--sidecar-log'
)
$planPath = Join-Path $resolvedOut 'e14_operation_token_acquisition_plan.json'
$readinessPath = Join-Path $resolvedOut 'e14_paused_save_readiness.json'
$terminalReadinessPath = Join-Path $resolvedOut 'e14_terminal_paused_save_readiness.json'
$actionPath = Join-Path $resolvedOut 'e14_authenticated_fixture_action.json'
$manifestPath = Join-Path $resolvedOut 'native_session_manifest.json'
$streamPath = Join-Path $resolvedOut 'collector_stream.bin'
$foldDir = Join-Path $resolvedOut 'e14_fold'
$foldReport = Join-Path $resolvedOut 'e14_fold_report.json'
$regradePath = Join-Path $resolvedOut 'e14_operation_token_census_regrade.json'
$ownerReceiptPath = Join-Path $resolvedOut 'e14_operation_token_acquisition_receipt.json'
Write-AtomicJson -Path $planPath -Value ([ordered]@{
    schema = 'hoi4.e14.operation-token-autonomous-acquisition-plan.v1'; transaction = 'unattended_exact_a729_e14_operation_token_census';
    attempt_budget = 1; clean_launch = $true; leaves_game_running = $true; leaves_game_paused = $true;
    session_id = $sessionId; capture_id = $captureId; executable_sha256 = $exeSha; continue_save_sha256 = $saveSha;
    save_oracle_sha256 = $oracleSha; collector_sha256 = (Get-FileHash $resolvedCollector -Algorithm SHA256).Hash.ToLowerInvariant();
    recorder_sha256 = (Get-FileHash $resolvedRecorder -Algorithm SHA256).Hash.ToLowerInvariant();
    stream_fold_sha256 = (Get-FileHash $resolvedStreamFold -Algorithm SHA256).Hash.ToLowerInvariant();
    acquisition_owner_sha256 = (Get-FileHash $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant();
    independent_regrader_sha256 = (Get-FileHash $regrader -Algorithm SHA256).Hash.ToLowerInvariant();
    authenticated_action_helper_sha256 = (Get-FileHash $actionHelper -Algorithm SHA256).Hash.ToLowerInvariant();
    source_static_bundle_manifest_sha256 = (Get-FileHash $resolvedBundle -Algorithm SHA256).Hash.ToLowerInvariant();
    prebuilt_attestation_sha256 = (Get-FileHash $resolvedAttestation -Algorithm SHA256).Hash.ToLowerInvariant();
    prepared_admission_sha256 = (Get-FileHash $preparedAdmissionPath -Algorithm SHA256).Hash.ToLowerInvariant();
    product_modset_sha256 = $productModsetSha; scheduled_domains = @($domain); fixture_action = 'authenticated Escape plus center hover while paused';
    save_hour = [string]$oracleObject.meta.save_date; stream_format = 'compact'; stream_compression = 'none'; min_game_hours = 0
})

$continueGamePath = Join-Path $resolvedUserDir 'continue_game.json'
$priorExists = Test-Path -LiteralPath $continueGamePath -PathType Leaf
$priorBytes = if ($priorExists) { [IO.File]::ReadAllBytes($continueGamePath) } else { $null }
$continueOverridden = $false
$capture = $null
$game = $null
$transactionError = $null
try {
    $continueObject = if ($priorExists) { Get-Content $continueGamePath -Raw | ConvertFrom-Json } else { [pscustomobject]@{ title=''; desc=''; date=[DateTime]::UtcNow.ToString('R'); filename=''; is_remote=$false } }
    $continueObject.filename = [IO.Path]::GetFileName($resolvedSave)
    $continueObject.is_remote = $false
    $continueOverridden = $true
    Write-AtomicText -Path $continueGamePath -Text (($continueObject | ConvertTo-Json -Depth 10) + "`n") -AllowReplace
    $capture = Start-HiddenNative -Executable $resolvedRecorder -Arguments $arguments
    $deadline = [DateTime]::UtcNow.AddMinutes(30)
    while ([DateTime]::UtcNow -lt $deadline -and -not $capture.Process.HasExited) {
        $candidates = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
        if ($candidates.Count -gt 1) { throw 'Clean launch produced multiple hoi4.exe processes.' }
        if ($candidates.Count -eq 1) {
            try {
                $candidate = $candidates[0]
                $path = [string](Get-CimInstance Win32_Process -Filter "ProcessId=$($candidate.Id)" -ErrorAction Stop).ExecutablePath
                if ($candidate.MainWindowHandle -ne [IntPtr]::Zero -and [IO.Path]::GetFullPath($path).Equals($resolvedExe, [StringComparison]::OrdinalIgnoreCase) -and
                    (Get-FileHash $path -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $expectedExeSha256) { $game = $candidate; break }
            } catch { }
        }
        Start-Sleep -Milliseconds 250
    }
    if ($null -eq $game) { throw 'Exact-a729 did not expose an authenticated window.' }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $readinessHelper -ProcessId $game.Id -ReceiptPath $readinessPath `
        -TimeoutSeconds $GameLoadTimeoutSeconds -StableSamples 4 -PollMilliseconds 250 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Authenticated paused-save readiness failed.' }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $actionHelper -ProcessId $game.Id -VirtualKey 0x1B `
        -ClientX $FixtureClientX -ClientY $FixtureClientY -ReceiptPath $actionPath -SettleMilliseconds 3000 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Authenticated E14 fixture action failed.' }
    if (-not $capture.Process.WaitForExit([Math]::Max(1000, ($DurationSeconds + 180) * 1000))) { throw 'collector_record exceeded its bounded finalization deadline.' }
    if ($capture.Process.ExitCode -ne 0) { throw "collector_record exited with code $($capture.Process.ExitCode)." }
    $initialReadiness = Get-Content -LiteralPath $readinessPath -Raw | ConvertFrom-Json
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $readinessHelper -ProcessId $game.Id -ReceiptPath $terminalReadinessPath `
        -ExpectedGameHourRaw ([int]$initialReadiness.game_hour) -TimeoutSeconds 30 -StableSamples 4 -PollMilliseconds 250 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Authenticated terminal paused-save readiness failed.' }
} catch { $transactionError = $_ } finally {
    if ($null -ne $capture -and -not $capture.Process.HasExited) {
        if (-not $capture.Process.WaitForExit([Math]::Max(1000, ($DurationSeconds + 180) * 1000)) -and $null -eq $transactionError) {
            $transactionError = [System.Management.Automation.RuntimeException]::new('collector_record did not detach within its bounded normal-finalization deadline')
        }
    }
    if ($null -ne $capture -and $capture.Process.HasExited) {
        Write-AtomicText (Join-Path $resolvedOut 'collector_record.stdout.log') ([string]$capture.StdoutTask.Result)
        Write-AtomicText (Join-Path $resolvedOut 'collector_record.stderr.log') ([string]$capture.StderrTask.Result)
        $capture.Process.Dispose()
    }
    if ($continueOverridden) {
        if ($priorExists) { Write-AtomicBytes -Path $continueGamePath -Bytes $priorBytes -AllowReplace }
        elseif (Test-Path -LiteralPath $continueGamePath) { Remove-Item -LiteralPath $continueGamePath -Force }
    }
}
if ($null -ne $transactionError) { throw $transactionError }
foreach ($required in @($manifestPath, $streamPath, $readinessPath, $terminalReadinessPath, $actionPath, $planPath)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "Capture output absent: $required" }
}

$fold = Start-HiddenNative -Executable $resolvedStreamFold -Arguments @($streamPath, $foldDir, '--game-id', 'e14_operation_tokens', '--json', $foldReport, '--workers', '1', '--identity-mode', 'legacy')
$fold.Process.WaitForExit()
Write-AtomicText (Join-Path $resolvedOut 'stream_fold.stdout.log') ([string]$fold.StdoutTask.Result)
Write-AtomicText (Join-Path $resolvedOut 'stream_fold.stderr.log') ([string]$fold.StderrTask.Result)
$foldExit = $fold.Process.ExitCode
$fold.Process.Dispose()
if ($foldExit -ne 0) { throw "stream_fold rejected E14 capture with code $foldExit" }

& $pythonExe $regrader --fold-dir $foldDir --save-oracle $resolvedOracle --capture-manifest $manifestPath `
    --acquisition-plan $planPath --readiness-receipt $readinessPath --terminal-readiness-receipt $terminalReadinessPath --action-receipt $actionPath `
    --source-static-bundle-manifest $resolvedBundle --prepared-admission $preparedAdmissionPath `
    --prebuilt-attestation $resolvedAttestation --continue-save $resolvedSave --collector-dll $resolvedCollector `
    --recorder-exe $resolvedRecorder --stream-fold-exe $resolvedStreamFold --out $regradePath | Out-Null
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $regradePath -PathType Leaf)) { throw "E14 independent regrade rejected the capture; inspect $resolvedOut" }
$regrade = Get-Content $regradePath -Raw | ConvertFrom-Json
if ($regrade.ok -ne $true -or [string]$regrade.verdict -cne 'PASS_EXACT_SAME_HOUR_OPERATION_TOKEN_CENSUS') { throw 'E14 regrade did not emit its exact passing verdict.' }

Write-AtomicJson -Path $ownerReceiptPath -Value ([ordered]@{
    schema = 'hoi4.e14.operation-token-autonomous-acquisition-receipt.v1'; verdict = 'passed';
    transaction = 'unattended_exact_a729_e14_operation_token_census'; executable_sha256 = $exeSha;
    session_id = $sessionId; capture_id = $captureId; process_id = $regrade.owner_transaction.process_id;
    save_hour = $regrade.owner_transaction.same_hour; membership_count = $regrade.live_membership_count;
    product_modset_sha256 = $productModsetSha; activation_authorized = $false;
    artifacts = [ordered]@{
        acquisition_plan = [ordered]@{ file = [IO.Path]::GetFileName($planPath); sha256 = (Get-FileHash $planPath -Algorithm SHA256).Hash.ToLowerInvariant() };
        native_session_manifest = [ordered]@{ file = [IO.Path]::GetFileName($manifestPath); sha256 = (Get-FileHash $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant() };
        readiness = [ordered]@{ file = [IO.Path]::GetFileName($readinessPath); sha256 = (Get-FileHash $readinessPath -Algorithm SHA256).Hash.ToLowerInvariant() };
        terminal_readiness = [ordered]@{ file = [IO.Path]::GetFileName($terminalReadinessPath); sha256 = (Get-FileHash $terminalReadinessPath -Algorithm SHA256).Hash.ToLowerInvariant() };
        fixture_action = [ordered]@{ file = [IO.Path]::GetFileName($actionPath); sha256 = (Get-FileHash $actionPath -Algorithm SHA256).Hash.ToLowerInvariant() };
        collector_stream = [ordered]@{ file = [IO.Path]::GetFileName($streamPath); sha256 = (Get-FileHash $streamPath -Algorithm SHA256).Hash.ToLowerInvariant() };
        fold_report = [ordered]@{ file = [IO.Path]::GetFileName($foldReport); sha256 = (Get-FileHash $foldReport -Algorithm SHA256).Hash.ToLowerInvariant() };
        folded_snapshot = [ordered]@{ file = [string]$regrade.owner_transaction.snapshot_relative_to_fold; sha256 = [string]$regrade.snapshot_sha256 };
        continue_save = [ordered]@{ logical_name = [IO.Path]::GetFileName($resolvedSave); sha256 = $saveSha };
        save_oracle = [ordered]@{ logical_name = [IO.Path]::GetFileName($resolvedOracle); sha256 = $oracleSha };
        source_static_bundle_manifest = [ordered]@{ logical_name = [IO.Path]::GetFileName($resolvedBundle); sha256 = (Get-FileHash $resolvedBundle -Algorithm SHA256).Hash.ToLowerInvariant() };
        prepared_product_modset_admission = [ordered]@{ file = [IO.Path]::GetFileName($preparedAdmissionPath); sha256 = (Get-FileHash $preparedAdmissionPath -Algorithm SHA256).Hash.ToLowerInvariant() };
        prebuilt_artifact_attestation = [ordered]@{ logical_name = [IO.Path]::GetFileName($resolvedAttestation); sha256 = (Get-FileHash $resolvedAttestation -Algorithm SHA256).Hash.ToLowerInvariant() };
        independent_regrade = [ordered]@{ file = [IO.Path]::GetFileName($regradePath); sha256 = (Get-FileHash $regradePath -Algorithm SHA256).Hash.ToLowerInvariant() }
    };
    semantic_authority = 'same-hour OPTK complete-set carrier plus independent save regrade; action receipt is provenance only';
    tooling = [ordered]@{
        acquisition_owner_sha256 = (Get-FileHash $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant();
        independent_regrader_sha256 = (Get-FileHash $regrader -Algorithm SHA256).Hash.ToLowerInvariant();
        authenticated_action_helper_sha256 = (Get-FileHash $actionHelper -Algorithm SHA256).Hash.ToLowerInvariant()
    };
    leaves_game_running = $true; leaves_game_paused = $true
})
Write-Output "E14 exact same-hour operation-token census admitted: $ownerReceiptPath"
