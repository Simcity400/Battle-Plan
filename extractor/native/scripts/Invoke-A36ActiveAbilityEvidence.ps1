[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $RecorderExe,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $PdxUserDir,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [Parameter(Mandatory = $true)] [uint32] $ExistingPid,
    [Parameter(Mandatory = $true)] [string] $StaticBundleManifest,
    [Parameter(Mandatory = $true)] [string] $StaticBundleBindingsDir,
    [string] $PrebuiltArtifactAttestationPath = '',
    [int] $DurationSeconds = 1800,
    [int] $SampleMilliseconds = 100,
    [int] $ProcessTimeoutSeconds = 300,
    [ValidateSet('C01', 'C02', 'C03', 'C04')] [string[]] $ValidateCard = @('C01', 'C02', 'C03', 'C04'),
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
foreach ($path in @($RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll, $StaticBundleManifest, $StaticBundleBindingsDir)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required path does not exist: $path" }
}
if ($ExistingPid -eq 0) { throw 'A36 passive observation requires an explicit already-running the game PID.' }
if ($DurationSeconds -lt 300) { throw 'DurationSeconds must be at least 300.' }
if ($SampleMilliseconds -lt 1 -or $SampleMilliseconds -gt 1000) { throw 'SampleMilliseconds must be in 1..1000.' }
if ($ProcessTimeoutSeconds -lt 30 -or $ProcessTimeoutSeconds -gt 1800) { throw 'ProcessTimeoutSeconds must be in 30..1800.' }

$resolvedRecorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedStaticBundleManifest = (Resolve-Path -LiteralPath $StaticBundleManifest).Path
$resolvedStaticBundleBindingsDir = (Resolve-Path -LiteralPath $StaticBundleBindingsDir).Path
$resolvedOut = [IO.Path]::GetFullPath($OutDir)

if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) {
    throw 'Hoi4Exe does not match exact a729 authority.'
}
$existing = Get-Process -Id $ExistingPid -ErrorAction Stop
if (-not [string]::Equals($existing.ProcessName, 'the game', [StringComparison]::OrdinalIgnoreCase)) {
    throw "ExistingPid $ExistingPid is not hoi4.exe."
}
$existingPath = $existing.Path
if (-not [IO.Path]::GetFullPath($existingPath).Equals($resolvedExe, [StringComparison]::OrdinalIgnoreCase)) {
    throw "ExistingPid $ExistingPid executable path differs from Hoi4Exe."
}
if ((Get-FileHash -LiteralPath $existingPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) {
    throw "ExistingPid $ExistingPid does not match exact a729 authority."
}

if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or
        @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty so stale A36 evidence cannot be selected: $resolvedOut"
    }
} else {
    [IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
}

$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$productionDriver = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1')).Path
$attestationHelpers = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1')).Path
$releaseRoot = Join-Path $repo '.cargo-target\release'
if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) {
    $PrebuiltArtifactAttestationPath = Join-Path $releaseRoot 'hoi4-production-prebuilt-artifacts.json'
}
. $attestationHelpers
$prebuilt = Test-ProductionArtifactAttestation `
    -RepoRoot $repo `
    -ReceiptPath $PrebuiltArtifactAttestationPath `
    -ExpectedDriverPath $productionDriver
$resolvedAttestation = (Resolve-Path -LiteralPath $prebuilt.receipt_path).Path
if (-not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.recorder_host).Equals($resolvedRecorder, [StringComparison]::OrdinalIgnoreCase) -or
    -not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.collector_dll).Equals($resolvedCollector, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'A36 observer artifacts must come from the current authenticated central Release attestation.'
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

function Write-AtomicBytes([string]$Path, [byte[]]$Bytes) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($fullPath)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporaryPath = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllBytes($temporaryPath, $Bytes)
        $stream = [IO.File]::Open($temporaryPath, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($fullPath)) { [IO.File]::Replace($temporaryPath, $fullPath, $null) }
        else { [IO.File]::Move($temporaryPath, $fullPath) }
    } finally {
        if ([IO.File]::Exists($temporaryPath)) { [IO.File]::Delete($temporaryPath) }
    }
}

function Write-AtomicUtf8Text([string]$Path, [string]$Text) {
    Write-AtomicBytes $Path ([Text.UTF8Encoding]::new($false).GetBytes($Text))
}

function Get-ArtifactRecord([string]$Path) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    return [ordered]@{
        path = [IO.Path]::GetRelativePath($resolvedOut, $fullPath).Replace('\\', '/')
        byte_count = (Get-Item -LiteralPath $fullPath).Length
        sha256 = (Get-FileHash -LiteralPath $fullPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

$verifier = Join-Path $repo 'extractor\documents\dossiers\A36\verify_a36_active_ability_live_fixture.py'
$captureManifest = Join-Path $resolvedOut 'native_session_manifest.json'
$copiedReceipt = Join-Path $resolvedOut 'a36_active_ability_evidence_capture.json'
$verification = Join-Path $resolvedOut 'a36_active_ability_live_fixture_regrade.json'
$passiveReceipt = Join-Path $resolvedOut 'a36_passive_observer_receipt.json'
$collectorReceipt = Join-Path (Split-Path -Parent $resolvedCollector) 'a36_active_ability_evidence_capture.json'
if (Test-Path -LiteralPath $collectorReceipt) {
    throw "Move or remove the prior create-new A36 receipt before observation: $collectorReceipt"
}
$captureStartedUtc = [DateTime]::UtcNow

$arguments = @(
    '--pid', [string]$ExistingPid,
    '--pdx-user-dir', $resolvedUserDir,
    '--dll', $resolvedCollector,
    '--out-dir', $resolvedOut,
    '--session-id', ([guid]::NewGuid().ToString()),
    '--capture-id', ([guid]::NewGuid().ToString()),
    '--duration-s', $DurationSeconds,
    '--sample-ms', $SampleMilliseconds,
    '--stream-format', 'compact',
    '--profile', 'debug',
    '--scheduled-domain', 'fuel_live',
    '--scheduled-domain', 'divisions',
    '--scheduled-domain', 'oob_command',
    '--scheduled-domain', 'characters_leaders_cabinet_live',
    '--scheduled-domains-exact',
    '--static-bundle-manifest', $resolvedStaticBundleManifest,
    '--static-bundle-mod-state', (Join-Path $resolvedStaticBundleBindingsDir 'mod_state.json'),
    '--static-bundle-dlc-state', (Join-Path $resolvedStaticBundleBindingsDir 'dlc_state.json'),
    '--static-bundle-load-order', (Join-Path $resolvedStaticBundleBindingsDir 'load_order.json'),
    '--static-bundle-source-inventory', (Join-Path $resolvedStaticBundleBindingsDir 'source_inventory.json'),
    '--static-bundle-live-resolution-report', (Join-Path $resolvedStaticBundleBindingsDir 'resolution_report.json'),
    '--static-bundle-timeout-ms', '1800000',
    '--sidecar-log',
    '--a36-active-ability-evidence'
)

Write-Output @'
A36 passive observation is armed against the authenticated existing process. It never
launches, stops, pauses, unpauses, focuses, resizes, or sends input to HOI4. Play normally.
Engine/UI transactions caused by the user's own play are retained automatically. Missing
C01-C04 states, WorldFrame joins, or selected-Present receipts remain explicitly unobserved.
'@

$recorder = [Diagnostics.Process]::new()
$recorderStarted = $false
$recorderStdout = $null
$recorderStderr = $null
$captureCompleted = $false
try {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $resolvedRecorder
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    if ($null -ne $start.PSObject.Properties['ArgumentList']) {
        foreach ($argument in $arguments) { [void]$start.ArgumentList.Add([string]$argument) }
    } else {
        $start.Arguments = ConvertTo-NativeArgumentLine $arguments
    }
    $recorder.StartInfo = $start
    if (-not $recorder.Start()) { throw 'A36 passive observer failed to start.' }
    $recorderStarted = $true
    $recorderStdout = $recorder.StandardOutput.ReadToEndAsync()
    $recorderStderr = $recorder.StandardError.ReadToEndAsync()
    if (-not $recorder.WaitForExit([Math]::Max(60000, ($DurationSeconds + 600) * 1000))) {
        throw 'A36 passive observer exceeded its bounded finalization deadline; the game was not touched.'
    }
    if ($recorder.ExitCode -ne 0) {
        throw "A36 passive observer exited with code $($recorder.ExitCode); retain $resolvedOut for diagnosis."
    }
    $captureCompleted = $true
} finally {
    if ($recorderStarted -and $recorder.HasExited) {
        if ($null -ne $recorderStdout) { Write-AtomicUtf8Text (Join-Path $resolvedOut 'collector_record.stdout.log') ([string]$recorderStdout.Result) }
        if ($null -ne $recorderStderr) { Write-AtomicUtf8Text (Join-Path $resolvedOut 'collector_record.stderr.log') ([string]$recorderStderr.Result) }
    }
    $recorder.Dispose()
}
if (-not $captureCompleted) { throw 'A36 passive observation did not complete.' }
if (-not (Test-Path -LiteralPath $captureManifest -PathType Leaf)) {
    throw "Recorder capture manifest missing: $captureManifest"
}

$manifestObject = Get-Content -LiteralPath $captureManifest -Raw | ConvertFrom-Json
$candidateRoots = [Collections.Generic.List[string]]::new()
$candidateRoots.Add($resolvedOut)
$candidateRoots.Add((Split-Path -Parent $resolvedCollector))
if ($manifestObject.dll) { $candidateRoots.Add((Split-Path -Parent ([string]$manifestObject.dll))) }
$candidate = Get-ChildItem -LiteralPath @($candidateRoots | Select-Object -Unique) `
    -Recurse -File -Filter 'a36_active_ability_evidence_capture.json' -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTimeUtc -ge $captureStartedUtc } |
    Sort-Object LastWriteTimeUtc -Descending |
    Select-Object -First 1
if ($candidate) {
    $candidatePath = [IO.Path]::GetFullPath($candidate.FullName)
    if (-not $candidatePath.Equals([IO.Path]::GetFullPath($copiedReceipt), [StringComparison]::OrdinalIgnoreCase)) {
        Write-AtomicBytes $copiedReceipt ([IO.File]::ReadAllBytes($candidatePath))
    }
}

$regradeExitCode = $null
if (Test-Path -LiteralPath $copiedReceipt -PathType Leaf) {
    $verifyArguments = @(
        $verifier,
        '--receipt', $copiedReceipt,
        '--capture-manifest', $captureManifest,
        '--collector-dll', $resolvedCollector,
        '--out', $verification
    )
    foreach ($card in $ValidateCard) { $verifyArguments += @('--required-card', $card) }
    & $Python @verifyArguments
    $regradeExitCode = $LASTEXITCODE
}

$worldFrameArtifacts = @(Get-ChildItem -LiteralPath $resolvedOut -Recurse -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match '(?i)world[_-]?frame' } |
    Sort-Object FullName | ForEach-Object { Get-ArtifactRecord $_.FullName })
$selectedPresentArtifacts = @(Get-ChildItem -LiteralPath $resolvedOut -Recurse -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match '(?i)selected[_-]?present' } |
    Sort-Object FullName | ForEach-Object { Get-ArtifactRecord $_.FullName })
$rawObserved = Test-Path -LiteralPath $copiedReceipt -PathType Leaf
$regradeObserved = Test-Path -LiteralPath $verification -PathType Leaf
$passive = [ordered]@{
    schema = 'hoi4-a36-passive-observer-receipt-v1'
    status = if ($regradeExitCode -eq 0) { 'observed_required_cards' } else { 'fail_closed_incomplete_observation' }
    client_interaction = [ordered]@{
        user_is_only_controller = $true
        scripted_input = $false
        console_commands = $false
        clock_writes = $false
        focus_or_display_changes = $false
        launch_or_process_control = $false
    }
    authority = [ordered]@{
        process_id = [uint32]$ExistingPid
        executable_sha256 = $expectedExeSha256
        collector_dll_sha256 = (Get-FileHash -LiteralPath $resolvedCollector -Algorithm SHA256).Hash.ToLowerInvariant()
        prebuilt_attestation_sha256 = (Get-FileHash -LiteralPath $resolvedAttestation -Algorithm SHA256).Hash.ToLowerInvariant()
        capture_manifest = Get-ArtifactRecord $captureManifest
    }
    planes = [ordered]@{
        engine = [ordered]@{
            status = if ($rawObserved) { 'observed' } else { 'unobserved' }
            receipt = if ($rawObserved) { Get-ArtifactRecord $copiedReceipt } else { $null }
        }
        ui = [ordered]@{
            status = if ($regradeExitCode -eq 0) { 'observed_and_mechanically_joined' } elseif ($regradeObserved) { 'observed_but_incomplete' } else { 'unobserved' }
            regrade_exit_code = $regradeExitCode
            receipt = if ($regradeObserved) { Get-ArtifactRecord $verification } else { $null }
        }
        world_frame = [ordered]@{
            status = if ($worldFrameArtifacts.Count -gt 0) { 'observed' } else { 'unobserved' }
            artifacts = $worldFrameArtifacts
        }
        selected_present = [ordered]@{
            status = if ($selectedPresentArtifacts.Count -gt 0) { 'observed' } else { 'unobserved' }
            artifacts = $selectedPresentArtifacts
        }
    }
    fail_closed_law = 'No absent plane, missing card, or incomplete join is synthesized, inferred, or promoted.'
}
Write-AtomicUtf8Text $passiveReceipt (($passive | ConvertTo-Json -Depth 12) + "`n")

if (-not $rawObserved) { throw "The passive observer produced no A36 engine/UI receipt; fail-closed receipt: $passiveReceipt" }
if ($regradeExitCode -ne 0) { throw "A36 passive observation did not witness every requested card; fail-closed receipt: $passiveReceipt" }
Write-Output "A36 passive observation accepted for promotion review: $passiveReceipt"
