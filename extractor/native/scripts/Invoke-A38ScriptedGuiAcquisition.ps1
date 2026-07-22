[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $RecorderExe,
    [Parameter(Mandatory = $true)] [string] $StreamFoldExe,
    [Parameter(Mandatory = $true)] [int] $Hoi4Pid,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [Parameter(Mandatory = $true)] [string] $PreparedProductModsetDir,
    [string] $PrebuiltArtifactAttestationPath = '',
    [int] $DurationSeconds = 900,
    [int] $SampleMilliseconds = 25,
    [string] $Python = 'python'
)

# Compatibility filename only. This is an attach-only passive observer. It never
# launches, stops, pauses, unpauses, focuses, resizes, or sends input to HOI4.
# The player alone determines which states become observable.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
if ($DurationSeconds -lt 30 -or $DurationSeconds -gt 86400) { throw 'DurationSeconds must be in 30..86400.' }
if ($SampleMilliseconds -lt 10 -or $SampleMilliseconds -gt 1000) { throw 'SampleMilliseconds must be in 10..1000.' }
foreach ($path in @($RecorderExe, $StreamFoldExe, $CollectorDll, $PreparedProductModsetDir)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required A38 passive-observer path does not exist: $path" }
}

$recorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$streamFold = (Resolve-Path -LiteralPath $StreamFoldExe).Path
$collector = (Resolve-Path -LiteralPath $CollectorDll).Path
$prepared = (Resolve-Path -LiteralPath $PreparedProductModsetDir).Path
$pythonExe = (Get-Command $Python -CommandType Application -ErrorAction Stop).Source
$out = [IO.Path]::GetFullPath($OutDir)
$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\..')).Path
$productionDriver = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1')).Path
$attestationHelpers = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1')).Path
$preparedVerifier = (Resolve-Path -LiteralPath (Join-Path $repo 'extractor\game_defs\verify_product_modset_capture_ready.py')).Path
$staticVerifier = (Resolve-Path -LiteralPath (Join-Path $repo 'extractor\game_defs\static_bundle.py')).Path
$admissionVerifier = (Resolve-Path -LiteralPath (Join-Path $repo 'extractor\native\lanes\scripted_state_flags_variables_live\verify_a38_production_capture.py')).Path

function Test-ContainedPath([string]$Candidate, [string]$Root) {
    $candidateFull = [IO.Path]::GetFullPath($Candidate).TrimEnd('\')
    $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd('\')
    return $candidateFull.Equals($rootFull, [StringComparison]::OrdinalIgnoreCase) -or
        $candidateFull.StartsWith($rootFull + '\', [StringComparison]::OrdinalIgnoreCase)
}
if (Test-ContainedPath $out $repo) { throw 'OutDir must be outside the repository.' }
if (Test-Path -LiteralPath $out) {
    if (-not (Test-Path -LiteralPath $out -PathType Container) -or @(Get-ChildItem -LiteralPath $out -Force).Count -ne 0) {
        throw "OutDir must be absent or empty: $out"
    }
} else { [IO.Directory]::CreateDirectory($out) | Out-Null }

function Write-AtomicText([string]$Path, [string]$Text) {
    $full = [IO.Path]::GetFullPath($Path)
    if ([IO.File]::Exists($full)) { throw "Refusing to replace existing A38 observer artifact: $full" }
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
function Get-ArtifactRow([string]$Path, [string]$RelativePath) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $null }
    $item = Get-Item -LiteralPath $Path
    return [ordered]@{
        path = $RelativePath
        bytes = [int64]$item.Length
        sha256 = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

# Read-only process authentication. No process lifecycle operation is performed.
$game = Get-Process -Id $Hoi4Pid -ErrorAction Stop
try { $exe = [IO.Path]::GetFullPath($game.MainModule.FileName) } catch { throw 'Cannot authenticate the requested the game process image.' }
if ([IO.Path]::GetFileName($exe) -cne 'hoi4.exe' -or
    (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) {
    throw 'Hoi4Pid is not the exact a729 authority process.'
}

$releaseRoot = Join-Path $repo '.cargo-target\release'
if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) {
    $PrebuiltArtifactAttestationPath = Join-Path $releaseRoot 'hoi4-production-prebuilt-artifacts.json'
}
. $attestationHelpers
$prebuilt = Test-ProductionArtifactAttestation -RepoRoot $repo -ReceiptPath $PrebuiltArtifactAttestationPath -ExpectedDriverPath $productionDriver
if (-not $recorder.Equals([string]$prebuilt.artifacts.recorder_host, [StringComparison]::OrdinalIgnoreCase) -or
    -not $collector.Equals([string]$prebuilt.artifacts.collector_dll, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'A38 passive observation requires recorder and collector from the authenticated central Release attestation.'
}
$expectedStreamFold = [IO.Path]::GetFullPath((Join-Path $releaseRoot 'stream_fold.exe'))
if (-not $streamFold.Equals($expectedStreamFold, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'A38 passive observation requires the central Release stream_fold artifact.'
}

$preflightTemp = Join-Path ([IO.Path]::GetTempPath()) ("a38-passive-preflight-" + [guid]::NewGuid().ToString('N') + '.json')
$preflight = Join-Path $out 'a38_product_modset_preflight.json'
try {
    & $pythonExe $preparedVerifier --exe $exe --prepared $prepared --out $preflightTemp
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $preflightTemp -PathType Leaf)) {
        throw 'A38 prepared product-modset preflight failed.'
    }
    [IO.File]::Move($preflightTemp, $preflight)
} finally { if (Test-Path -LiteralPath $preflightTemp) { Remove-Item -LiteralPath $preflightTemp -Force } }
$preflightObject = Get-Content -LiteralPath $preflight -Raw | ConvertFrom-Json
if ([string]$preflightObject.schema -cne 'extractor.product_modset_capture_admission.v1' -or
    [string]$preflightObject.verdict -cne 'passed') { throw 'A38 prepared product-modset receipt is noncanonical.' }
$bundleManifest = (Resolve-Path -LiteralPath ([string]$preflightObject.paths.static_bundle_manifest)).Path
$binding = [ordered]@{
    mod_state = (Resolve-Path -LiteralPath ([string]$preflightObject.paths.mod_state)).Path
    dlc_state = (Resolve-Path -LiteralPath ([string]$preflightObject.paths.dlc_state)).Path
    load_order = (Resolve-Path -LiteralPath ([string]$preflightObject.paths.load_order)).Path
    source_inventory = (Resolve-Path -LiteralPath ([string]$preflightObject.paths.source_inventory)).Path
    resolution_report = (Resolve-Path -LiteralPath ([string]$preflightObject.paths.resolution_report)).Path
}

$sessionId = 'a38-passive-' + [guid]::NewGuid().ToString('N')
$captureId = 'capture-' + [guid]::NewGuid().ToString('N')
$manifest = Join-Path $out 'native_session_manifest.json'
$capture = Join-Path $out 'collector_stream.bin'
$foldDir = Join-Path $out 'folded_snapshots'
$foldReport = Join-Path $out 'a38_fold_report.json'
$admission = Join-Path $out 'a38_production_admission.json'
$receiptPath = Join-Path $out 'a38_passive_observer_receipt.json'
$failure = $null

$arguments = @(
    '--pid', [string]$Hoi4Pid, '--dll', $collector, '--out-dir', $out,
    '--session-id', $sessionId, '--capture-id', $captureId,
    '--duration-s', [string]$DurationSeconds, '--sample-ms', [string]$SampleMilliseconds,
    '--min-game-hours', '0', '--stream-format', 'compact', '--profile', 'debug',
    '--static-bundle-manifest', $bundleManifest, '--static-bundle-mod-state', $binding.mod_state,
    '--static-bundle-dlc-state', $binding.dlc_state, '--static-bundle-load-order', $binding.load_order,
    '--static-bundle-source-inventory', $binding.source_inventory,
    '--static-bundle-live-resolution-report', $binding.resolution_report,
    '--static-bundle-verifier', $staticVerifier, '--static-bundle-python', $pythonExe,
    '--static-bundle-timeout-ms', '300000', '--sidecar-log', '--hour-latch'
)

try {
    # Synchronous attach-only observer. Recorder completion detaches its own
    # instrumentation; this script never owns or terminates the game process.
    & $recorder @arguments
    if ($LASTEXITCODE -ne 0) { throw "passive recorder exited with code $LASTEXITCODE" }
    foreach ($required in @($manifest, $capture, $preflight)) {
        if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "A38 observer artifact is absent: $required" }
    }
    if (Test-Path -LiteralPath $foldDir) { throw "A38 fold directory must be create-new: $foldDir" }
    & $streamFold $capture $foldDir --game-id a38_scripted_gui_runtime --json $foldReport --workers 1 --identity-mode identity-v1
    if ($LASTEXITCODE -ne 0) { throw 'stream_fold rejected the A38 passive observation.' }
    & $pythonExe $admissionVerifier --capture $capture --snapshots $foldDir --fold-report $foldReport --session-manifest $manifest --out $admission
    if ($LASTEXITCODE -ne 0) { throw 'A38 observations did not satisfy production admission; the unobserved state remains unavailable.' }
} catch {
    $failure = $_.Exception.Message
}

$admissionObject = if (Test-Path -LiteralPath $admission -PathType Leaf) {
    Get-Content -LiteralPath $admission -Raw | ConvertFrom-Json
} else { $null }
$passed = $null -ne $admissionObject -and
    [string]$admissionObject.status -ceq 'PASS_A38_H4CT_OWNER_CONSUMER_D84' -and
    [bool]$admissionObject.production_admission_ready
$receipt = [ordered]@{
    schema = 'extractor.a38_passive_observer.v1'
    status = if ($passed) { 'PASS_A38_PASSIVELY_OBSERVED' } else { 'FAIL_CLOSED_A38_UNOBSERVED' }
    production_admission_ready = [bool]$passed
    client_interaction = 'none'
    observation_policy = 'user_drives_client_observer_records_only'
    missing_state_policy = 'unavailable_never_fabricated'
    failure = if ($passed) { $null } elseif ($null -ne $failure) { $failure } else { 'no admissible A38 frame was observed' }
    timestamp_utc = [DateTime]::UtcNow.ToString('o')
    session_id = $sessionId
    capture_id = $captureId
    observed_pid = $Hoi4Pid
    executable_sha256 = $expectedExeSha256
    tools = [ordered]@{
        recorder = Get-ArtifactRow $recorder ([IO.Path]::GetFileName($recorder))
        collector = Get-ArtifactRow $collector ([IO.Path]::GetFileName($collector))
        stream_fold = Get-ArtifactRow $streamFold ([IO.Path]::GetFileName($streamFold))
        admission_verifier = Get-ArtifactRow $admissionVerifier 'verify_a38_production_capture.py'
        prebuilt_attestation = Get-ArtifactRow ([string]$prebuilt.receipt_path) 'hoi4-production-prebuilt-artifacts.json'
    }
    artifacts = [ordered]@{
        capture = Get-ArtifactRow $capture 'collector_stream.bin'
        session_manifest = Get-ArtifactRow $manifest 'native_session_manifest.json'
        fold_report = Get-ArtifactRow $foldReport 'a38_fold_report.json'
        admission = Get-ArtifactRow $admission 'a38_production_admission.json'
        product_modset_preflight = Get-ArtifactRow $preflight 'a38_product_modset_preflight.json'
    }
}
Write-AtomicText $receiptPath (($receipt | ConvertTo-Json -Depth 12) + "`n")
Write-Output "A38 passive observer receipt: $receiptPath"
if (-not $passed) { exit 2 }
