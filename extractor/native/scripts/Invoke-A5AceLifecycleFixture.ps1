[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$RecorderExe,
    [Parameter(Mandatory = $true)][string]$StreamFoldExe,
    [Parameter(Mandatory = $true)][string]$Hoi4Exe,
    [Parameter(Mandatory = $true)][string]$PdxUserDir,
    [Parameter(Mandatory = $true)][string]$CollectorDll,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [Parameter(Mandatory = $true)][string]$ContinueSave,
    [Parameter(Mandatory = $true)][string]$StaticBundleManifest,
    [Parameter(Mandatory = $true)][string]$StaticBundleBindingsDir,
    [string]$PrebuiltArtifactAttestationPath = '',
    [string]$ExpectedContinueSaveSha256 = 'c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735',
    [string]$Rakaly = '',
    [int]$DurationSeconds = 90,
    [int]$SampleMilliseconds = 100,
    [int]$GameLoadTimeoutSeconds = 420,
    [string]$Python = 'python',
    [switch]$EnableLegacyClientDriving
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $EnableLegacyClientDriving) {
    throw 'This legacy owner launches and controls HOI4. It is disabled by default. Use A5_ACE_LIFECYCLE/PASSIVE_OBSERVER_WORKFLOW.json with the shared passive observer; only an explicit -EnableLegacyClientDriving opt-in may enter this quarantined path.'
}

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$packet = Join-Path $repo 'extractor\re_evidence\A5_ACE_LIFECYCLE'
$parseAbilities = Join-Path $repo 'extractor\save_oracle\parse_abilities.py'
$captureTool = Join-Path $packet 'capture_a5_lifecycle_fixture.py'
$selectSnapshot = Join-Path $packet 'select_a5_production_snapshot.py'
$verifyTool = Join-Path $packet 'verify_a5_lifecycle_fixture.py'
$admitTool = Join-Path $packet 'admit_a5_lifecycle_fixture.py'
$productionDriver = Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1'
$attestationHelpers = Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1'
$readinessHelper = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_wait_for_paused_save.ps1'
$productModsetVerifier = Join-Path $repo 'extractor\game_defs\verify_product_modset_capture_ready.py'
$bindingNames = @('mod_state.json', 'dlc_state.json', 'load_order.json', 'source_inventory.json', 'resolution_report.json')

function Test-IsWithinPath([string]$Child, [string]$Parent) {
    $childFull = [IO.Path]::GetFullPath($Child).TrimEnd('\')
    $parentFull = [IO.Path]::GetFullPath($Parent).TrimEnd('\')
    return $childFull.Equals($parentFull, [StringComparison]::OrdinalIgnoreCase) -or
        $childFull.StartsWith($parentFull + '\', [StringComparison]::OrdinalIgnoreCase)
}

function ConvertTo-NativeArgumentLine([object[]]$Arguments) {
    return (($Arguments | ForEach-Object {
        $value = [string]$_
        if ($value -match '[\s"]') {
            '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
        } else { $value }
    }) -join ' ')
}

function Write-AtomicBytes([string]$Path, [byte[]]$Bytes) {
    $full = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($full)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllBytes($temporary, $Bytes)
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($full)) { [IO.File]::Replace($temporary, $full, $null) }
        else { [IO.File]::Move($temporary, $full) }
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

function Write-AtomicUtf8Text([string]$Path, [string]$Text) {
    Write-AtomicBytes $Path ([Text.UTF8Encoding]::new($false).GetBytes($Text))
}

function Write-AtomicNewJson([string]$Path, $Value) {
    if (Test-Path -LiteralPath $Path) { throw "Refusing to overwrite create-new receipt: $Path" }
    Write-AtomicUtf8Text $Path (($Value | ConvertTo-Json -Depth 20) + "`n")
}

function Get-GameHourRaw([string]$SaveDate) {
    $parts = @($SaveDate.Split('.') | ForEach-Object { [int]$_ })
    if ($parts.Count -ne 4) { throw "Invalid Clausewitz save date: $SaveDate" }
    $monthLengths = @(31,28,31,30,31,30,31,31,30,31,30,31)
    $year = $parts[0]; $month = $parts[1]; $day = $parts[2]; $hour = $parts[3]
    if ($year -lt 1850 -or $year -gt 2200 -or $month -lt 1 -or $month -gt 12 -or
        $day -lt 1 -or $day -gt $monthLengths[$month - 1] -or $hour -lt 0 -or $hour -gt 23) {
        throw "Invalid Clausewitz save date: $SaveDate"
    }
    $dayOfYear = $day - 1
    for ($index = 0; $index -lt ($month - 1); $index++) { $dayOfYear += $monthLengths[$index] }
    return [int](43799999 + (($year * 365 + $dayOfYear) * 24) + $hour)
}

if ($DurationSeconds -lt 30 -or $DurationSeconds -gt 900) { throw 'DurationSeconds must be in 30..900.' }
if ($SampleMilliseconds -lt 1 -or $SampleMilliseconds -gt 1000) { throw 'SampleMilliseconds must be in 1..1000.' }
if ($GameLoadTimeoutSeconds -lt 30 -or $GameLoadTimeoutSeconds -gt 1800) { throw 'GameLoadTimeoutSeconds must be in 30..1800.' }
if ($ExpectedContinueSaveSha256 -cnotmatch '^[0-9a-f]{64}$') { throw 'ExpectedContinueSaveSha256 must be lowercase SHA-256.' }
foreach ($path in @(
    $RecorderExe, $StreamFoldExe, $Hoi4Exe, $PdxUserDir, $CollectorDll, $ContinueSave,
    $StaticBundleManifest, $StaticBundleBindingsDir, $parseAbilities, $captureTool,
    $selectSnapshot, $verifyTool, $admitTool, $productionDriver, $attestationHelpers,
    $readinessHelper, $productModsetVerifier
)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required path does not exist: $path" }
}
foreach ($leaf in $bindingNames) {
    if (-not (Test-Path -LiteralPath (Join-Path $StaticBundleBindingsDir $leaf) -PathType Leaf)) {
        throw "Static-bundle binding is absent: $leaf"
    }
}
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
    throw 'A5 autonomous acquisition requires a clean early launch and refuses caller-selected or pre-existing PIDs.'
}

$resolvedRecorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedStreamFold = (Resolve-Path -LiteralPath $StreamFoldExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedSave = (Resolve-Path -LiteralPath $ContinueSave).Path
$resolvedBundle = (Resolve-Path -LiteralPath $StaticBundleManifest).Path
$resolvedBindings = (Resolve-Path -LiteralPath $StaticBundleBindingsDir).Path
$resolvedOut = [IO.Path]::GetFullPath($OutDir)
$pythonExe = (Get-Command $Python -CommandType Application -ErrorAction Stop).Source
$bindingPaths = [ordered]@{}
foreach ($leaf in $bindingNames) { $bindingPaths[$leaf] = (Resolve-Path -LiteralPath (Join-Path $resolvedBindings $leaf)).Path }
if (Test-IsWithinPath $resolvedOut $repo) { throw 'OutDir must be outside the repository.' }
if (Test-IsWithinPath $resolvedOut $resolvedUserDir) { throw 'OutDir must be outside the game user directory.' }
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty: $resolvedOut"
    }
} else { [IO.Directory]::CreateDirectory($resolvedOut) | Out-Null }

$saveDirectory = [IO.Path]::GetFullPath((Join-Path $resolvedUserDir 'save games')).TrimEnd('\')
if (-not [IO.Path]::GetFullPath((Split-Path -Parent $resolvedSave)).TrimEnd('\').Equals($saveDirectory, [StringComparison]::OrdinalIgnoreCase)) {
    throw "ContinueSave must be a direct child of $saveDirectory"
}
$exeSha256 = (Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($exeSha256 -cne $expectedExeSha256) { throw "Hoi4Exe exact-a729 identity mismatch: $exeSha256" }
$saveSha256 = (Get-FileHash -LiteralPath $resolvedSave -Algorithm SHA256).Hash.ToLowerInvariant()
if ($saveSha256 -cne $ExpectedContinueSaveSha256) { throw "ContinueSave identity mismatch: $saveSha256" }

. $attestationHelpers
if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) {
    $PrebuiltArtifactAttestationPath = Join-Path $repo '.cargo-target\release\hoi4-production-prebuilt-artifacts.json'
}
$prebuilt = Test-ProductionArtifactAttestation -RepoRoot $repo -ReceiptPath $PrebuiltArtifactAttestationPath -ExpectedDriverPath $productionDriver
if (-not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.recorder_host).Equals($resolvedRecorder, [StringComparison]::OrdinalIgnoreCase) -or
    -not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.collector_dll).Equals($resolvedCollector, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'A5 recorder and collector differ from the current centralized Release attestation.'
}

$preparedRoot = [IO.Path]::GetFullPath((Split-Path -Parent $resolvedBindings))
$preparedAdmissionPath = Join-Path $resolvedOut 'prepared_product_modset_admission.json'
& $pythonExe $productModsetVerifier --exe $resolvedExe --prepared $preparedRoot --out $preparedAdmissionPath
if ($LASTEXITCODE -ne 0) { throw 'Prepared product-modset admission failed.' }
$preparedAdmission = Get-Content -LiteralPath $preparedAdmissionPath -Raw | ConvertFrom-Json
if ([string]$preparedAdmission.verdict -cne 'passed') { throw 'Prepared product-modset admission did not pass.' }
if (-not [IO.Path]::GetFullPath([string]$preparedAdmission.paths.static_bundle_manifest).Equals($resolvedBundle, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Prepared admission static bundle differs from the requested bundle.'
}

# The owner creates the oracle directly from the exact source save.  The caller
# cannot inject a parsed JSON document or choose a folded snapshot.
$oracleDir = Join-Path $resolvedOut 'save_oracle'
[IO.Directory]::CreateDirectory($oracleDir) | Out-Null
$oracleArgs = @($parseAbilities, '--out-dir', $oracleDir)
if (-not [string]::IsNullOrWhiteSpace($Rakaly)) { $oracleArgs += @('--rakaly', (Resolve-Path -LiteralPath $Rakaly).Path) }
$oracleArgs += $resolvedSave
& $pythonExe @oracleArgs
if ($LASTEXITCODE -ne 0) { throw 'Owner-generated abilities oracle failed.' }
$abilities = Join-Path $oracleDir 'abilities.json'
if (-not (Test-Path -LiteralPath $abilities -PathType Leaf)) { throw 'Owner-generated abilities.json is absent.' }
$abilitiesObject = Get-Content -LiteralPath $abilities -Raw | ConvertFrom-Json
$saveDate = [string]$abilitiesObject.meta.save_date
$expectedGameHourRaw = Get-GameHourRaw $saveDate
$aceRows = @($abilitiesObject.aces.PSObject.Properties | ForEach-Object { @($_.Value.aces) })
$livingRows = @($aceRows | Where-Object { $_.alive -eq $true })
$deadHandledRows = @($aceRows | Where-Object { $_.alive -eq $false -and $_.handled -eq $true })
if ($livingRows.Count -eq 0 -or $deadHandledRows.Count -eq 0) {
    throw "Machine discovery found no qualifying living/dead-handled controls in the frozen exact fixture save (living=$($livingRows.Count), dead_handled=$($deadHandledRows.Count))."
}

$sessionId = [guid]::NewGuid().ToString()
$captureId = [guid]::NewGuid().ToString()
$captureManifest = Join-Path $resolvedOut 'native_session_manifest.json'
$collectorStream = Join-Path $resolvedOut 'collector_stream.bin'
$rawCapture = Join-Path $resolvedOut 'a5_raw_lifecycle_fixture.json'
$readinessReceipt = Join-Path $resolvedOut 'a5_paused_save_readiness.json'
$planPath = Join-Path $resolvedOut 'a5_autonomous_acquisition_plan.json'
$foldDir = Join-Path $resolvedOut 'a5_fold'
$foldReport = Join-Path $resolvedOut 'a5_fold_report.json'
$productionSnapshot = Join-Path $resolvedOut 'a5_production.snapshot.json'
$regrade = Join-Path $resolvedOut 'a5_lifecycle_regrade.json'
$admission = Join-Path $resolvedOut 'a5_lifecycle_admission.json'
$sourceBundle = Get-Content -LiteralPath $resolvedBundle -Raw | ConvertFrom-Json
$productModsetSha256 = [string]$sourceBundle.bindings.product_modset.product_modset_sha256
if ($productModsetSha256 -cnotmatch '^[0-9a-f]{64}$' -or $productModsetSha256 -cne [string]$preparedAdmission.product_modset_sha256) {
    throw 'Static-bundle product-modset identity is absent or differs from prepared admission.'
}

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
    '--stream-format', 'compact',
    '--stream-compression', 'none',
    '--profile', 'debug',
    '--scheduled-domain', 'country_identity',
    '--scheduled-domain', 'air_variant_archetypes',
    '--scheduled-domains-exact',
    '--static-bundle-manifest', $resolvedBundle,
    '--static-bundle-mod-state', [string]$bindingPaths['mod_state.json'],
    '--static-bundle-dlc-state', [string]$bindingPaths['dlc_state.json'],
    '--static-bundle-load-order', [string]$bindingPaths['load_order.json'],
    '--static-bundle-source-inventory', [string]$bindingPaths['source_inventory.json'],
    '--static-bundle-live-resolution-report', [string]$bindingPaths['resolution_report.json'],
    '--static-bundle-timeout-ms', '300000',
    '--sidecar-log'
)
Write-AtomicNewJson $planPath ([ordered]@{
    schema = 'a5_ace_lifecycle_autonomous_acquisition_plan.v1'
    transaction = 'clean_exact_a729_owner_generated_a5_lifecycle'
    clean_early_launch = $true
    caller_supplied_pid = $false
    caller_supplied_oracle = $false
    caller_supplied_snapshot = $false
    fixture_selection = 'frozen_source_save_machine_discovery_proved_living_and_dead_handled_controls'
    executable_sha256 = $exeSha256
    source_save_sha256 = $saveSha256
    recorder_sha256 = (Get-FileHash -LiteralPath $resolvedRecorder -Algorithm SHA256).Hash.ToLowerInvariant()
    stream_fold_sha256 = (Get-FileHash -LiteralPath $resolvedStreamFold -Algorithm SHA256).Hash.ToLowerInvariant()
    collector_sha256 = (Get-FileHash -LiteralPath $resolvedCollector -Algorithm SHA256).Hash.ToLowerInvariant()
    expected_game_hour_raw = $expectedGameHourRaw
    save_date = $saveDate
    discovered_living_rows = $livingRows.Count
    discovered_dead_handled_rows = $deadHandledRows.Count
    session_id = $sessionId
    capture_id = $captureId
    product_modset_sha256 = $productModsetSha256
    central_prebuilt_attestation_sha256 = [string]$prebuilt.receipt_sha256
    prepared_product_modset_admission_sha256 = (Get-FileHash -LiteralPath $preparedAdmissionPath -Algorithm SHA256).Hash.ToLowerInvariant()
    scheduled_domains = @('country_identity', 'air_variant_archetypes')
    recorder_arguments = $arguments
})

$continueGamePath = Join-Path $resolvedUserDir 'continue_game.json'
$continuePriorExists = Test-Path -LiteralPath $continueGamePath -PathType Leaf
$continuePriorBytes = if ($continuePriorExists) { [IO.File]::ReadAllBytes($continueGamePath) } else { $null }
$continueOverridden = $false
$recorder = [Diagnostics.Process]::new()
$recorderStarted = $false
$stdoutTask = $null
$stderrTask = $null
$game = $null
$cleanupError = $null
try {
    $continueObject = if ($continuePriorExists) { Get-Content -LiteralPath $continueGamePath -Raw | ConvertFrom-Json } else {
        [pscustomobject]@{ title = [IO.Path]::GetFileNameWithoutExtension($resolvedSave); desc = ''; date = [DateTime]::UtcNow.ToString('R'); filename = ''; is_remote = $false }
    }
    $continueObject.filename = [IO.Path]::GetFileName($resolvedSave)
    $continueObject.is_remote = $false
    $continueOverridden = $true
    Write-AtomicUtf8Text $continueGamePath (($continueObject | ConvertTo-Json -Depth 10) + "`n")

    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $resolvedRecorder
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    if ($null -ne $start.PSObject.Properties['ArgumentList']) { foreach ($argument in $arguments) { [void]$start.ArgumentList.Add([string]$argument) } }
    else { $start.Arguments = ConvertTo-NativeArgumentLine $arguments }
    $recorder.StartInfo = $start
    if (-not $recorder.Start()) { throw 'collector_record failed to start.' }
    $recorderStarted = $true
    $stdoutTask = $recorder.StandardOutput.ReadToEndAsync()
    $stderrTask = $recorder.StandardError.ReadToEndAsync()

    $deadline = [DateTime]::UtcNow.AddSeconds($GameLoadTimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline -and -not $recorder.HasExited -and $null -eq $game) {
        $candidates = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
        if ($candidates.Count -gt 1) { throw 'Clean A5 launch produced more than one hoi4.exe process.' }
        if ($candidates.Count -eq 1) {
            $candidate = $candidates[0]
            try {
                $cim = Get-CimInstance Win32_Process -Filter "ProcessId=$($candidate.Id)" -ErrorAction Stop
                $loadedCollector = @($candidate.Modules | Where-Object { [IO.Path]::GetFullPath([string]$_.FileName).Equals($resolvedCollector, [StringComparison]::OrdinalIgnoreCase) })
                if ([uint32]$cim.ParentProcessId -eq [uint32]$recorder.Id -and $candidate.MainWindowHandle -ne [IntPtr]::Zero -and
                    [IO.Path]::GetFullPath([string]$cim.ExecutablePath).Equals($resolvedExe, [StringComparison]::OrdinalIgnoreCase) -and
                    (Get-FileHash -LiteralPath ([string]$cim.ExecutablePath) -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $expectedExeSha256 -and
                    $loadedCollector.Count -eq 1) { $game = $candidate }
            } catch { }
        }
        if ($null -eq $game) { Start-Sleep -Milliseconds 250 }
    }
    if ($null -eq $game) { throw 'Authenticated exact-a729 A5 process/collector tuple did not become ready.' }

    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $readinessHelper `
        -ProcessId $game.Id -ReceiptPath $readinessReceipt -ExpectedGameHourRaw $expectedGameHourRaw `
        -TimeoutSeconds $GameLoadTimeoutSeconds -StableSamples 4 -PollMilliseconds 250
    if ($LASTEXITCODE -ne 0) { throw 'Paused fixture readiness failed.' }
    if ($continueOverridden) {
        if ($continuePriorExists) { Write-AtomicBytes $continueGamePath $continuePriorBytes }
        elseif (Test-Path -LiteralPath $continueGamePath) { Remove-Item -LiteralPath $continueGamePath -Force }
        $continueOverridden = $false
    }

    & $pythonExe $captureTool --pid $game.Id --expected-game-hour-raw $expectedGameHourRaw --save-date $saveDate --out $rawCapture
    if ($LASTEXITCODE -ne 0) { throw 'A5 autonomous raw lifecycle capture failed.' }
    if (-not $recorder.WaitForExit([Math]::Max(60000, ($DurationSeconds + 300) * 1000))) { throw 'collector_record exceeded the A5 capture deadline.' }
    if ($recorder.ExitCode -ne 0) { throw "collector_record exited with code $($recorder.ExitCode)." }
} finally {
    if ($continueOverridden) {
        if ($continuePriorExists) { Write-AtomicBytes $continueGamePath $continuePriorBytes }
        elseif (Test-Path -LiteralPath $continueGamePath) { Remove-Item -LiteralPath $continueGamePath -Force }
    }
    if ($recorderStarted -and -not $recorder.HasExited) {
        if (-not $recorder.WaitForExit(30000)) { Stop-Process -Id $recorder.Id -Force -ErrorAction SilentlyContinue }
    }
    if ($recorderStarted -and $recorder.HasExited) {
        if ($null -ne $stdoutTask) { Write-AtomicUtf8Text (Join-Path $resolvedOut 'collector_record.stdout.log') ([string]$stdoutTask.Result) }
        if ($null -ne $stderrTask) { Write-AtomicUtf8Text (Join-Path $resolvedOut 'collector_record.stderr.log') ([string]$stderrTask.Result) }
    }
    if ($null -ne $game -and -not $game.HasExited) {
        try {
            if ((Get-FileHash -LiteralPath $game.MainModule.FileName -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) {
                throw 'Owner-launched game identity changed before teardown.'
            }
            Stop-Process -Id $game.Id -Force -ErrorAction Stop
            $game.WaitForExit(30000) | Out-Null
            if (-not $game.HasExited) { throw 'Owner-launched exact-a729 process did not exit during bounded teardown.' }
        } catch { $cleanupError = $_ }
    }
    $recorder.Dispose()
}

if ($null -ne $cleanupError) { throw "A5 owner teardown failed: $($cleanupError.Exception.Message)" }

foreach ($required in @($captureManifest, $collectorStream, $rawCapture, $readinessReceipt, $planPath)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "A5 capture output is absent: $required" }
}
if (Test-Path -LiteralPath $foldDir) { throw "Fold output must be create-new: $foldDir" }
& $resolvedStreamFold $collectorStream $foldDir --game-id a5_lifecycle --json $foldReport --workers 1 --identity-mode legacy
if ($LASTEXITCODE -ne 0) { throw 'stream_fold rejected the A5 compact capture.' }
& $pythonExe $selectSnapshot --fold-dir $foldDir --abilities $abilities --out $productionSnapshot
if ($LASTEXITCODE -ne 0) { throw 'No unique exact-hour/full-roster production snapshot was selected.' }
& $pythonExe $verifyTool --capture $rawCapture --abilities $abilities --production-snapshot $productionSnapshot --out $regrade
if ($LASTEXITCODE -ne 0) { throw "A5 independent lifecycle regrade failed; retain $resolvedOut" }
& $pythonExe $admitTool `
    --capture $rawCapture --abilities $abilities --production-snapshot $productionSnapshot --regrade $regrade `
    --session-manifest $captureManifest --acquisition-plan $planPath --readiness-receipt $readinessReceipt `
    --collector-stream $collectorStream --source-save $resolvedSave --recorder-exe $resolvedRecorder `
    --stream-fold-exe $resolvedStreamFold --collector-dll $resolvedCollector `
    --prebuilt-attestation ([string]$prebuilt.receipt_path) --prepared-admission $preparedAdmissionPath --out $admission
if ($LASTEXITCODE -ne 0) { throw "A5 immutable admission failed; retain $resolvedOut" }
Write-Output "A5 autonomous lifecycle acquisition admitted: $admission"
