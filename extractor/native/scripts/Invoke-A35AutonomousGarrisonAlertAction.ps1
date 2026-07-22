[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $CaptureManifest,
    [Parameter(Mandatory = $true)] [string] $Replay,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $PrebuiltArtifactAttestationPath,
    [Parameter(Mandatory = $true)] [string] $ReceiptPath,
    [int] $StablePolls = 3,
    [int] $PollMilliseconds = 500
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
if ($StablePolls -lt 2 -or $StablePolls -gt 20) { throw 'StablePolls must be in 2..20.' }
if ($PollMilliseconds -lt 100 -or $PollMilliseconds -gt 5000) { throw 'PollMilliseconds must be in 100..5000.' }
foreach ($path in @($CaptureManifest, $Replay, $CollectorDll, $PrebuiltArtifactAttestationPath)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Required passive A35 input is missing: $path" }
}

function Write-AtomicCreateNewUtf8([string] $Path, [string] $Text) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    if ([IO.File]::Exists($fullPath)) { throw "Refusing to replace immutable A35 passive receipt: $fullPath" }
    $parent = [IO.Path]::GetDirectoryName($fullPath)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText($temporary, $Text, [Text.UTF8Encoding]::new($false))
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        [IO.File]::Move($temporary, $fullPath)
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

function Get-StableFileIdentity([string] $Path, [string] $Label) {
    $resolved = (Resolve-Path -LiteralPath $Path).Path
    $prior = $null
    for ($ordinal = 0; $ordinal -lt $StablePolls; $ordinal++) {
        $item = Get-Item -LiteralPath $resolved
        $current = '{0}|{1}' -f $item.Length, (Get-FileHash -LiteralPath $resolved -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($null -ne $prior -and $current -cne $prior) {
            throw "Fail closed: $Label changed while the passive observer authenticated it."
        }
        $prior = $current
        if ($ordinal + 1 -lt $StablePolls) { Start-Sleep -Milliseconds $PollMilliseconds }
    }
    $parts = $prior.Split('|', 2)
    return [ordered]@{ path = $resolved; bytes = [int64]$parts[0]; sha256 = $parts[1] }
}

$manifestIdentity = Get-StableFileIdentity $CaptureManifest 'capture manifest'
$replayIdentity = Get-StableFileIdentity $Replay 'replay'
$collectorIdentity = Get-StableFileIdentity $CollectorDll 'collector DLL'
$attestationIdentity = Get-StableFileIdentity $PrebuiltArtifactAttestationPath 'central Release attestation'
$manifest = Get-Content -LiteralPath $manifestIdentity.path -Raw | ConvertFrom-Json
$attestation = Get-Content -LiteralPath $attestationIdentity.path -Raw | ConvertFrom-Json

if ($manifest.schema -cne 'native_collector_session_v1' -or
    [string]$manifest.stream_format -cne 'compact' -or
    [int64]$manifest.pid -le 0 -or
    [string]::IsNullOrWhiteSpace([string]$manifest.session_id) -or
    [string]::IsNullOrWhiteSpace([string]$manifest.capture_id)) {
    throw 'Fail closed: A35 passive capture manifest identity is invalid.'
}
if (@($manifest.scheduled_domains) -cnotcontains 'occupation_resistance_compliance_live') {
    throw 'Fail closed: A35 passive capture did not schedule domain 52.'
}
if ([IO.Path]::GetFileName([string]$manifest.replay_file) -cne [IO.Path]::GetFileName($replayIdentity.path)) {
    throw 'Fail closed: A35 capture manifest/replay filename identity differs.'
}
if ($manifest.parse_errors.Count -ne 0 -or $manifest.ordering_errors.Count -ne 0 -or
    $manifest.compact_stream.decodable -ne $true -or $manifest.compact_stream.ordered -ne $true -or
    $manifest.compact_stream.control_invalid_seen -ne $false -or
    $manifest.compact_stream.bridge_unavailable_seen -ne $false) {
    throw 'Fail closed: A35 passive compact stream is not admitted.'
}
if ($attestation.schema -cne 'battle-plan.central_rust_prebuilt_artifacts.v1' -or
    $attestation.status -cne 'PASS_CENTRAL_RELEASE_PREBUILT_ARTIFACTS' -or
    $attestation.successful_exit -ne $true -or
    $attestation.central_validation.mode -cne 'Release' -or
    @($attestation.central_validation.validated_workspaces) -cnotcontains 'Extractor') {
    throw 'Fail closed: A35 central Release attestation is invalid.'
}
$collectorRows = @($attestation.artifacts | Where-Object { $_.role -ceq 'collector_dll' })
if ($collectorRows.Count -ne 1 -or [string]$collectorRows[0].sha256 -cne $collectorIdentity.sha256) {
    throw 'Fail closed: A35 collector DLL does not match the unique central Release artifact.'
}

$receipt = [ordered]@{
    schema = 'hoi4.a35.passive-observer-binding.v1'
    status = 'PASSIVE_CAPTURE_BOUND_GAR1_VERDICT_REQUIRED'
    observer_mode = 'passive_only'
    acquisition_owner = 'user_gameplay'
    created_utc = [DateTime]::UtcNow.ToString('o')
    executable_sha256 = $expectedExeSha256
    session_id = [string]$manifest.session_id
    capture_id = [string]$manifest.capture_id
    process_id = [int64]$manifest.pid
    collector_dll_sha256 = $collectorIdentity.sha256
    capture_manifest = $manifestIdentity
    replay = $replayIdentity
    central_release_attestation = $attestationIdentity
    interaction_policy = [ordered]@{
        scripted_input = $false
        console_commands = $false
        unpause_or_speed_change = $false
        focus_or_cursor_control = $false
        display_change = $false
        launch_or_relaunch = $false
        process_control = $false
    }
    observation_contract = [ordered]@{
        storage = 'immutable_per_observation_files'
        engine = 'required: exact-a729 GAR1 producer payload'
        world_frame = 'required: one authenticated replay snapshot member with exact hour and owner join'
        ui = 'A25 text and A28 materialization/order remain independently required by XUI16 UI admission; this producer receipt cannot substitute for them'
        selected_present = 'not applicable to the A35 producer-to-WorldFrame activation claim; any pixel or final-output claim still requires an independent same-session CAP6 selected-Present join'
        identity = 'binary sha256 plus process/session/capture/hour/frame-member sha256'
    }
    selection_policy = 'admit only exact GAR1 observations already present in the authenticated replay WorldFrame; no alert or UI state is selected, induced, or inferred'
    unobserved_state = 'UNAVAILABLE_FAIL_CLOSED'
    semantic_authority = 'verify_a35_live_fixture.py alone may admit an actually observed active GAR1 owner/hour joined to the matching save oracle'
}
Write-AtomicCreateNewUtf8 $ReceiptPath (($receipt | ConvertTo-Json -Depth 12) + "`n")
Write-Output "A35 passive observer binding written: $ReceiptPath"
