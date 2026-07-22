[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [int] $ProcessId,
    [Parameter(Mandatory = $true)] [string] $ObservationDirectory,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [int] $DurationSeconds = 14400,
    [int] $PollMilliseconds = 500,
    [string] $ExpectedProcessStartUtc = '',
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# This is the production A37 acquisition path.  It is deliberately incapable
# of driving the game: it does not launch, attach/inject, stop, focus, resize, click,
# type, open the console, change the clock, or send window/input messages.  It
# only authenticates an already-running process and consumes immutable receipts
# emitted by the separately installed passive collector while the user plays.
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$expectedSchema = 'hoi4.a37.raw_controller_selected_present.v1'
$expectedStatus = 'controller_spine_only_d74_unavailable'
$expectedControllerManifest = 'a540af1c030d9ef2fbb1cd290627087627771a196e7a60e4241cb5287b66e4e2'
$expectedPanelManifest = '2c921216fb9b825716685852c026aba92b7da88a5c88fdb625a569e49afedd9c'

if ($ProcessId -le 0) { throw 'ProcessId must be positive.' }
if ($DurationSeconds -lt 1 -or $DurationSeconds -gt 86400) { throw 'DurationSeconds must be in 1..86400.' }
if ($PollMilliseconds -lt 100 -or $PollMilliseconds -gt 10000) { throw 'PollMilliseconds must be in 100..10000.' }
if (-not (Test-Path -LiteralPath $ObservationDirectory -PathType Container)) {
    throw "ObservationDirectory must already exist: $ObservationDirectory"
}

$observationRoot = (Resolve-Path -LiteralPath $ObservationDirectory).Path
$outputRoot = [IO.Path]::GetFullPath($OutDir)
if ([IO.Directory]::Exists($outputRoot) -or [IO.File]::Exists($outputRoot)) {
    throw "OutDir must be new: $outputRoot"
}
if ($outputRoot.StartsWith($observationRoot.TrimEnd([char]'\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'OutDir must not be inside ObservationDirectory.'
}
[IO.Directory]::CreateDirectory($outputRoot) | Out-Null

$regrader = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\lanes\power_balance_presentation_live\regrade_a37_live_fixture.py')).Path
$resolvedPython = (Get-Command $Python -CommandType Application -ErrorAction Stop).Source

function Write-AtomicBytes([string] $Path, [byte[]] $Bytes) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    if ([IO.File]::Exists($fullPath) -or [IO.Directory]::Exists($fullPath)) {
        throw "Refusing to replace immutable A37 output: $fullPath"
    }
    $parent = [IO.Path]::GetDirectoryName($fullPath)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporaryPath = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllBytes($temporaryPath, $Bytes)
        $stream = [IO.File]::Open($temporaryPath, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        [IO.File]::Move($temporaryPath, $fullPath)
    } finally {
        if ([IO.File]::Exists($temporaryPath)) { [IO.File]::Delete($temporaryPath) }
    }
}

function Write-AtomicJson([string] $Path, [object] $Value) {
    $json = ($Value | ConvertTo-Json -Depth 16) + "`n"
    Write-AtomicBytes $Path ([Text.UTF8Encoding]::new($false).GetBytes($json))
}

function Get-AuthenticatedProcess {
    $process = Get-Process -Id $ProcessId -ErrorAction Stop
    $startUtc = $process.StartTime.ToUniversalTime().ToString('o')
    if (-not [string]::IsNullOrWhiteSpace($ExpectedProcessStartUtc) -and
        [DateTime]::Parse($startUtc).ToUniversalTime() -ne [DateTime]::Parse($ExpectedProcessStartUtc).ToUniversalTime()) {
        throw 'Fail closed: PID start identity changed.'
    }
    $exePath = $process.MainModule.FileName
    $exeSha256 = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($exeSha256 -cne $expectedExeSha256) { throw "Fail closed: executable SHA-256 mismatch: $exeSha256" }
    return [ordered]@{ process_start_utc = $startUtc; executable_sha256 = $exeSha256 }
}

function Read-StableCandidate([string] $Path) {
    try {
        $first = [IO.File]::ReadAllBytes($Path)
        if ($first.Length -eq 0) { return $null }
        Start-Sleep -Milliseconds 25
        $second = [IO.File]::ReadAllBytes($Path)
        if ($first.Length -ne $second.Length) { return $null }
        $firstSha = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($first)).ToLowerInvariant()
        $secondSha = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($second)).ToLowerInvariant()
        if ($firstSha -cne $secondSha) { return $null }
        $root = [Text.UTF8Encoding]::new($false, $true).GetString($second) | ConvertFrom-Json
        return [ordered]@{ bytes = $second; sha256 = $secondSha; root = $root }
    } catch {
        return $null
    }
}

function Get-Sign([int64] $Value) {
    if ($Value -gt 0) { return 1 }
    if ($Value -lt 0) { return -1 }
    return 0
}

function Test-PassiveA37Receipt([object] $Root) {
    if ($Root.schema -cne $expectedSchema -or $Root.status -cne $expectedStatus) { return $false }
    if ($Root.executable_sha256 -cne $expectedExeSha256 -or
        $Root.controller_manifest_sha256 -cne $expectedControllerManifest -or
        $Root.panel_visibility_manifest_sha256 -cne $expectedPanelManifest) { return $false }
    $hook = $Root.hook_observation
    $present = $Root.selected_present
    if ($null -eq $hook -or $null -eq $present -or
        $hook.panel_open -ne $true -or $hook.gui_visible -ne $true -or
        $hook.selected_record_present -ne $true -or $hook.selected_range_present -ne $true) { return $false }
    if ([int64]$hook.materialization_qpc_ticks -le 0 -or [int64]$present.selected_qpc_ticks -le 0 -or
        [int64]$present.cutoff_qpc_ticks -le 0 -or
        [int64]$hook.materialization_qpc_ticks -gt [int64]$present.selected_qpc_ticks -or
        [int64]$present.selected_qpc_ticks -gt [int64]$present.cutoff_qpc_ticks -or
        [int64]$hook.qpc_frequency_hz -ne [int64]$present.qpc_frequency_hz) { return $false }
    if ([string]$present.receipt_sha256 -cnotmatch '^[0-9a-f]{64}$' -or
        [string]$present.receipt_sha256 -ceq ('0' * 64)) { return $false }
    if ($Root.pbp2_emitted -ne $false -or $Root.activation_authorized -ne $false) { return $false }
    return $true
}

$processIdentity = Get-AuthenticatedProcess
$deadline = [DateTime]::UtcNow.AddSeconds($DurationSeconds)
$seenHashes = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
$cases = [ordered]@{
    positive_current_negative_change = $null
    negative_current_positive_change = $null
}
$rejectedStableReceipts = 0

try {
    while ([DateTime]::UtcNow -lt $deadline -and ($null -eq $cases.positive_current_negative_change -or $null -eq $cases.negative_current_positive_change)) {
        $currentIdentity = Get-AuthenticatedProcess
        if ($currentIdentity.process_start_utc -cne $processIdentity.process_start_utc) {
            throw 'Fail closed: observed the game process was replaced.'
        }
        foreach ($candidatePath in @(Get-ChildItem -LiteralPath $observationRoot -Filter '*.json' -File -Recurse | Sort-Object FullName)) {
            $candidate = Read-StableCandidate $candidatePath.FullName
            if ($null -eq $candidate -or -not $seenHashes.Add($candidate.sha256)) { continue }
            if (-not (Test-PassiveA37Receipt $candidate.root)) {
                $rejectedStableReceipts++
                continue
            }
            $currentSign = Get-Sign ([int64]$candidate.root.hook_observation.current_value_raw)
            $changeSign = Get-Sign ([int64]$candidate.root.hook_observation.change_raw)
            $caseKey = if ($currentSign -eq 1 -and $changeSign -eq -1) {
                'positive_current_negative_change'
            } elseif ($currentSign -eq -1 -and $changeSign -eq 1) {
                'negative_current_positive_change'
            } else {
                $null
            }
            if ($null -eq $caseKey -or $null -ne $cases[$caseKey]) { continue }
            $destinationName = "$caseKey.json"
            Write-AtomicBytes (Join-Path $outputRoot $destinationName) $candidate.bytes
            $cases[$caseKey] = [ordered]@{
                file = $destinationName
                source_logical_name = $candidatePath.Name
                source_sha256 = $candidate.sha256
                source_bytes = $candidate.bytes.Length
                selected_present_receipt_sha256 = [string]$candidate.root.selected_present.receipt_sha256
                observed_current_sign = $currentSign
                observed_change_sign = $changeSign
            }
        }
        if ($null -eq $cases.positive_current_negative_change -or $null -eq $cases.negative_current_positive_change) {
            Start-Sleep -Milliseconds $PollMilliseconds
        }
    }

    if ($null -eq $cases.positive_current_negative_change -or $null -eq $cases.negative_current_positive_change) {
        $missing = @($cases.Keys | Where-Object { $null -eq $cases[$_] })
        $failure = [ordered]@{
            schema = 'hoi4.a37.passive-observation.v1'
            status = 'fail_closed_unobserved_states'
            executable_sha256 = $expectedExeSha256
            process_start_utc = $processIdentity.process_start_utc
            duration_seconds = $DurationSeconds
            observed_cases = @($cases.Keys | Where-Object { $null -ne $cases[$_] })
            missing_cases = $missing
            rejected_stable_receipts = $rejectedStableReceipts
            client_interaction = 'none'
            promotion_authorized = $false
        }
        Write-AtomicJson (Join-Path $outputRoot 'a37_passive_observation_failure.json') $failure
        throw "Fail closed: required A37 states were not passively observed: $($missing -join ', ')"
    }

    $regradePath = Join-Path $outputRoot 'a37_live_fixture_regrade.json'
    & $resolvedPython $regrader $outputRoot --output $regradePath
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $regradePath -PathType Leaf)) {
        throw 'A37 passive observations failed independent regrade.'
    }
    $regrade = Get-Content -LiteralPath $regradePath -Raw | ConvertFrom-Json
    if ($regrade.status -cne 'pass_static_regrade_activation_unchanged' -or $regrade.activation_authorized -ne $false) {
        throw 'A37 regrade did not preserve fail-closed activation.'
    }

    $receipt = [ordered]@{
        schema = 'hoi4.a37.passive-observation.v1'
        status = 'pass_passive_observations_regraded_activation_unchanged'
        executable_sha256 = $expectedExeSha256
        process_id = $ProcessId
        process_start_utc = $processIdentity.process_start_utc
        cases = @($cases.Values)
        regrade = [ordered]@{
            file = 'a37_live_fixture_regrade.json'
            sha256 = (Get-FileHash -LiteralPath $regradePath -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        observer_capabilities = @(
            'read process metadata for exact-build authentication',
            'read immutable collector receipts',
            'copy authenticated content into a new output directory',
            'grade only actually observed state'
        )
        forbidden_capabilities = @(
            'launch, stop, suspend, resume, inject into, or otherwise control a process',
            'send keyboard, mouse, console, window, focus, display, or clock input',
            'fabricate or infer an unobserved sign case'
        )
        pbp2_emitted = $false
        activation_authorized = $false
    }
    $receiptPath = Join-Path $outputRoot 'a37_passive_observation_receipt.json'
    Write-AtomicJson $receiptPath $receipt
    Get-Content -LiteralPath $receiptPath -Raw
} catch {
    throw
}
