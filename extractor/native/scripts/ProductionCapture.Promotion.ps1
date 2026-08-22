Set-StrictMode -Version Latest

$script:ProductionPromotionFailureMask = [uint32]0x13CF
# Complement of COMPACT_STATUS_V1_KNOWN_MASK
# (compact_contract/src/status_flags_base.rs). Bits 0..=14 are assigned; bit 14
# is COMPACT_STATUS_SOURCE_EMPTY, the informational badge a domain publishes
# when its read reached the source and the source's answer was empty. It is
# deliberately NOT in the failure mask above: a measured zero is an answer, and
# blocking promotion on it would refuse a capture that has nothing wrong with
# it. Was 'FFFFC000', which still counted bit 14 as an unassigned wire
# extension.
$script:ProductionPromotionUnknownStatusMask =
    [uint32]::Parse('FFFF0000', [Globalization.NumberStyles]::HexNumber)
# Domain-scoped lane-local status vocabulary, mirroring
# `compact_contract::lane_local_status_mask`. These bits are DEFINED -- they are
# just defined per domain rather than globally, which is why the manifest tags
# their rows `status_flags_scope = raw_global_plus_lane_local`. d53's bits
# 16..=22 and d60's 16..=17 sit above the assigned v1 vocabulary, so applying
# the unknown-status mask to the RAW per-domain word made every capture
# containing either domain throw "lacks real-source completeness" for a bit the
# contract names: runs/live_unlock/t15 publishes d53=0x530300 and d60=0x30300.
# The word must be globalized (raw AND NOT the domain's registered mask) before
# the unknown-status test, exactly as
# `compact_status_for_global_interpretation` does in the Rust consumers.
# Pinned against the contract by
# `compact_contract/tests/compact_contract_suite/script_status_vocabulary_sweep.rs`.
$script:ProductionPromotionLaneLocalStatusMask = @{
    49 = [uint32]0x00000800
    51 = [uint32]0x00003800
    53 = [uint32]0x007F0000
    60 = [uint32]0x00030000
    99 = [uint32]0x00070000
}

function Get-ProductionPromotionGlobalStatusFlags {
    <#
    .SYNOPSIS
    One per-domain status word as the GLOBAL vocabulary sees it.

    .DESCRIPTION
    Strips only the bits the contract has registered as that domain's
    lane-local badges. Nothing else is masked: an undefined bit survives this
    and is still caught by the unknown-status test, which is the whole point of
    that test.
    #>
    param(
        [Parameter(Mandatory = $true)][int]$DomainId,
        [Parameter(Mandatory = $true)][uint32]$StatusFlags
    )
    $laneLocal = [uint32]0
    if ($script:ProductionPromotionLaneLocalStatusMask.ContainsKey($DomainId)) {
        $laneLocal = $script:ProductionPromotionLaneLocalStatusMask[$DomainId]
    }
    return [uint32]($StatusFlags -band (-bnot $laneLocal))
}

$script:ProductionPromotionStatuses = @(
    'existing',
    'production_complete',
    'production_complete_base'
)

function Get-ProductionPromotionProperty {
    param(
        $Object,
        [Parameter(Mandatory = $true)][string]$Name
    )
    if ($null -eq $Object) { return $null }
    # The authorities this reads are a mix. Evidence parsed from JSON arrives as
    # PSCustomObject, but the artifact attestation is built in process and hands
    # over ordered dictionaries. PSObject.Properties does not see dictionary
    # keys, so a dictionary member read through here came back $null and every
    # comparison against it failed - which is how the attested collector
    # identity check below rejected an artifact whose hash matched exactly.
    if ($Object -is [System.Collections.IDictionary]) {
        if (-not $Object.Contains($Name)) { return $null }
        return $Object[$Name]
    }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) { return $null }
    return $property.Value
}

function Get-ProductionPromotionRequiredInteger {
    <#
    .SYNOPSIS
    Read a property that MUST be present and MUST be an integer, or throw.

    .DESCRIPTION
    PowerShell coerces `$null` to zero: `[uint32]$null` is `0`, `[int64]$null`
    is `0`. Every gate clause in this file that reads a count or a status word
    and requires it to be zero therefore PASSED when the property was missing.
    That is the exact inverse of a gate -- absence became the cleanest possible
    evidence.

    It was not hypothetical. The per-domain clause below requires
    `effective_failure_status_flags_or`, and the manifest did not publish that
    field at all, so for every domain of every capture the gate masked `0`
    against the failure mask and passed. The same shape has bitten this project
    before, with a `$null` exit code reading as `0`.

    This helper is the fix, and the reason it is a helper rather than an inline
    `$null` check at each site is that the trap is a property of the LANGUAGE:
    a new clause written in the obvious way reintroduces it. A clause that
    cannot be written without naming the property it requires cannot silently
    accept the property's absence.

    A JSON number arrives from ConvertFrom-Json as [int], [long] or [double];
    a whole-valued [double] is accepted and converted, a fractional one is not,
    and a string that merely looks numeric is refused outright.
    #>
    param(
        $Object,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Context
    )
    if ($null -eq $Object) {
        throw "$Context is missing the object that must carry '$Name'"
    }
    $hasProperty = if ($Object -is [System.Collections.IDictionary]) {
        $Object.Contains($Name)
    }
    else {
        $null -ne $Object.PSObject.Properties[$Name]
    }
    if (-not $hasProperty) {
        throw "$Context does not publish '$Name'; a gate must not read an absent field as 0"
    }
    $value = Get-ProductionPromotionProperty $Object $Name
    if ($null -eq $value) {
        throw "$Context publishes '$Name' as null; a gate must not read null as 0"
    }
    if ($value -is [bool] -or $value -is [string]) {
        throw "$Context publishes '$Name' as $($value.GetType().Name), not an integer"
    }
    # Integrality is tested in the value's OWN type. Windows PowerShell's
    # ConvertFrom-Json hands back `System.Decimal` for a JSON number with more
    # precision than a double carries, and casting that to double FIRST erases
    # the fraction: `[Math]::Floor([double]10.000000000000000000000000001)`
    # equals the double, so a non-integral count would have been accepted as 10.
    # `[Math]::Floor` has a decimal overload; letting it pick keeps the digits.
    if ($value -is [decimal]) {
        if ([Math]::Floor($value) -ne $value) {
            throw "$Context publishes '$Name' as a non-integral number"
        }
        return [int64]$value
    }
    if ($value -is [double] -or $value -is [single]) {
        if ([double]::IsNaN($value) -or [double]::IsInfinity($value)) {
            throw "$Context publishes '$Name' as a non-finite number"
        }
        if ([Math]::Floor([double]$value) -ne [double]$value) {
            throw "$Context publishes '$Name' as a non-integral number"
        }
        return [int64]$value
    }
    if ($value -isnot [int] -and $value -isnot [long] -and
        $value -isnot [uint32] -and $value -isnot [uint64] -and
        $value -isnot [int16] -and $value -isnot [byte]) {
        throw "$Context publishes '$Name' as $($value.GetType().Name), not an integer"
    }
    return [int64]$value
}

function Get-ProductionPromotionRequiredUInt32 {
    <#
    .SYNOPSIS
    A required integer that must also fit an unsigned 32-bit status word.
    #>
    param(
        $Object,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Context
    )
    $value = Get-ProductionPromotionRequiredInteger `
        -Object $Object -Name $Name -Context $Context
    if ($value -lt 0 -or $value -gt [uint32]::MaxValue) {
        throw "$Context publishes '$Name' = $value, outside the uint32 status word"
    }
    return [uint32]$value
}

function Get-ProductionPromotionRequiredString {
    <#
    .SYNOPSIS
    Read a property that MUST be a published, non-empty string, or throw.

    .DESCRIPTION
    `[string]123` is "123" and `[string]$null` is "", so casting an identity to
    a string before comparing it accepts a value of the wrong type and turns an
    absent one into the empty string. The identities this guards -- capture_id
    and session_id -- are what bind a receipt to the plan and the manifest.
    #>
    param(
        $Object,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Context
    )
    if ($null -eq $Object) {
        throw "$Context is missing the object that must carry '$Name'"
    }
    $hasProperty = if ($Object -is [System.Collections.IDictionary]) {
        $Object.Contains($Name)
    }
    else {
        $null -ne $Object.PSObject.Properties[$Name]
    }
    if (-not $hasProperty) {
        throw "$Context does not publish '$Name'"
    }
    $value = Get-ProductionPromotionProperty $Object $Name
    if ($value -isnot [string]) {
        throw "$Context publishes '$Name' as a non-string identity"
    }
    if ([string]::IsNullOrWhiteSpace($value)) {
        throw "$Context publishes '$Name' as an empty identity"
    }
    return $value
}

function Assert-ProductionPromotionJsonIntegerToken {
    <#
    .SYNOPSIS
    Refuse a field whose JSON token was a FLOAT, even a whole-valued one.

    .DESCRIPTION
    `Get-ProductionPromotionRequiredInteger` accepts a whole-valued [double] and
    converts it, which is right for a figure `ConvertFrom-Json` may have widened.
    It is wrong for a field whose producer writes a literal integer and whose
    every other reader refuses the float form: the gap ledger's `version` and its
    `row_count` are written by the recorder as `2` and as a count, and Rust,
    Python and the PowerShell expander all reject `2.0`. A gate that admits what
    its own producer cannot emit has a second, undocumented format.

    Use this BESIDE the required-integer read, not instead of it: this narrows
    the accepted token, that one still owns absence, null and range.
    #>
    param(
        $Object,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Context
    )
    # Read WITHOUT enumeration: `Get-ProductionPromotionProperty` returns through
    # the pipeline, and a one-element array comes back as its element -- so
    # `"row_count": [0]` read as scalar zero and promoted.
    $value = $null
    if ($null -ne $Object -and $null -ne $Object.PSObject.Properties[$Name]) {
        $value = $Object.PSObject.Properties[$Name].Value
    }
    if ($value -is [double] -or $value -is [single] -or $value -is [System.Array]) {
        throw "$Context publishes '$Name' as $($value.GetType().Name); it must be an integer token"
    }
    # A `Decimal` is how ConvertFrom-Json carries a plain integer too large for
    # Int64, so refusing it outright would reject a legitimately widened token.
    # `.NET Decimal` preserves SCALE, so the float spelling still shows its point.
    if ($value -is [decimal] -and ([string]$value -like '*.*')) {
        throw "$Context publishes '$Name' in float form; it must be an integer token"
    }
}

function Get-ProductionPromotionRequiredArray {
    <#
    .SYNOPSIS
    Read a property that MUST be a published list, or throw.

    .DESCRIPTION
    The array form of the same trap. `@($null)` is not an empty array in the
    way a reader expects -- but `@(Get-...Property $x 'absent')` yields a
    zero-length array, and every clause of the shape
    `@(...).Count -ne 0 { throw }` therefore PASSED when the list was missing.

    "The validator found no problems" and "the validator did not publish a
    problems list" are different facts, and only one of them is evidence.
    #>
    param(
        $Object,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Context
    )
    if ($null -eq $Object) {
        throw "$Context is missing the object that must carry '$Name'"
    }
    $hasProperty = if ($Object -is [System.Collections.IDictionary]) {
        $Object.Contains($Name)
    }
    else {
        $null -ne $Object.PSObject.Properties[$Name]
    }
    if (-not $hasProperty) {
        throw "$Context does not publish the list '$Name'; an absent list is not an empty one"
    }
    # NOT through Get-ProductionPromotionProperty, and NOT through an `if`
    # expression either. PowerShell UNROLLS an empty array out of both: a
    # function that returns `@()` returns nothing, and `$x = if (...) { @() }`
    # assigns `$null`. Either one turns a PUBLISHED empty list back into the
    # absence this function exists to distinguish it from. The unary comma is
    # what keeps the array intact across each boundary.
    if ($Object -is [System.Collections.IDictionary]) {
        $wrapped = , $Object[$Name]
    }
    else {
        $wrapped = , $Object.PSObject.Properties[$Name].Value
    }
    $value = $wrapped[0]
    if ($null -eq $value) {
        throw "$Context publishes '$Name' as null; an absent list is not an empty one"
    }
    return , @($value)
}

function Read-ProductionPromotionJson {
    param([Parameter(Mandatory = $true)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "production promotion evidence file is missing: $Path"
    }
    try {
        return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    }
    catch {
        throw "production promotion evidence is not JSON: $Path"
    }
}

function Get-ProductionPromotionSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Resolve-ContainedProductionPromotionPath {
    param(
        [Parameter(Mandatory = $true)][string]$RunDir,
        [Parameter(Mandatory = $true)][string]$Value,
        [Parameter(Mandatory = $true)][string]$Name
    )
    if ([string]::IsNullOrWhiteSpace($Value)) {
        throw "production promotion evidence path is empty: $Name"
    }
    $candidate = if ([IO.Path]::IsPathRooted($Value)) {
        [IO.Path]::GetFullPath($Value)
    }
    else {
        [IO.Path]::GetFullPath((Join-Path $RunDir $Value))
    }
    $root = [IO.Path]::GetFullPath($RunDir).TrimEnd('\') + '\'
    if (-not $candidate.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) {
        throw "production promotion evidence path escapes the capture run: $Name"
    }
    if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
        throw "production promotion evidence path is missing: $Name"
    }
    return $candidate
}

function Assert-ProductionPromotionHash {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Expected,
        [Parameter(Mandatory = $true)][string]$Name
    )
    if ($Expected -cnotmatch '^[0-9a-f]{64}$' -or
        (Get-ProductionPromotionSha256 -Path $Path) -cne $Expected) {
        throw "production promotion evidence hash mismatch: $Name"
    }
}

function Get-ProductionPromotionGateArgument {
    param(
        [Parameter(Mandatory = $true)][object[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$Switch
    )

    $matches = @()
    for ($index = 0; $index -lt $Arguments.Count; $index++) {
        if ([string]$Arguments[$index] -ceq $Switch) {
            $matches += $index
        }
    }
    if ($matches.Count -ne 1 -or
        [int]$matches[0] + 1 -ge $Arguments.Count) {
        throw "independent production gate lacks one exact $Switch argument"
    }
    return [string]$Arguments[[int]$matches[0] + 1]
}

function Get-ProductionCapturePromotionEvidence {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$CaptureReceiptPath,
        [Parameter(Mandatory = $true)][string]$CurrentArtifactAttestationPath
    )

    $repo = [IO.Path]::GetFullPath($RepoRoot)
    $receiptPath = [IO.Path]::GetFullPath($CaptureReceiptPath)
    $runDir = Split-Path -Parent $receiptPath
    $receipt = Read-ProductionPromotionJson -Path $receiptPath
    if ([string](Get-ProductionPromotionProperty $receipt 'schema') -cne
            'battle-plan.production_capture_validation_receipt.v1' -or
        [string](Get-ProductionPromotionProperty $receipt 'acceptance_mode') -cne 'production' -or
        (Get-ProductionPromotionProperty $receipt 'admissible_for_production') -isnot [bool] -or
        -not (Get-ProductionPromotionProperty $receipt 'admissible_for_production')) {
        throw 'capture receipt has no production-admissible authority'
    }
    if ([IO.Path]::GetFullPath([string](Get-ProductionPromotionProperty $receipt 'receipt_path')) -ine
        $receiptPath) {
        throw 'capture receipt does not identify its own canonical path'
    }

    $paths = [ordered]@{}
    foreach ($spec in @(
        @('capture_plan', 'capture_plan_path', 'capture_plan_sha256'),
        @('manifest', 'manifest_path', 'manifest_sha256'),
        @('ready', 'capture_ready_receipt_path', 'capture_ready_receipt_sha256'),
        @('teardown', 'capture_teardown_receipt_path', 'capture_teardown_receipt_sha256'),
        @('validation', 'validation_report_path', 'validation_report_sha256'),
        @('sink', 'sink_reconciliation_report_path', 'sink_reconciliation_report_sha256'),
        @('retained_attestation', 'prebuilt_artifact_attestation_path',
            'prebuilt_artifact_attestation_sha256')
    )) {
        $path = Resolve-ContainedProductionPromotionPath `
            -RunDir $runDir `
            -Value ([string](Get-ProductionPromotionProperty $receipt $spec[1])) `
            -Name $spec[0]
        Assert-ProductionPromotionHash `
            -Path $path `
            -Expected ([string](Get-ProductionPromotionProperty $receipt $spec[2])) `
            -Name $spec[0]
        $paths[$spec[0]] = $path
    }
    $speedPath = Resolve-ContainedProductionPromotionPath `
        -RunDir $runDir -Value 'speed5_proof.json' -Name 'speed5_proof'

    $plan = Read-ProductionPromotionJson -Path $paths.capture_plan
    $manifest = Read-ProductionPromotionJson -Path $paths.manifest
    $validation = Read-ProductionPromotionJson -Path $paths.validation
    $sink = Read-ProductionPromotionJson -Path $paths.sink
    $speed = Read-ProductionPromotionJson -Path $speedPath
    $currentAttestationPath = [IO.Path]::GetFullPath($CurrentArtifactAttestationPath)
    $currentAttestation = Read-ProductionPromotionJson -Path $currentAttestationPath
    $retainedAttestation = Read-ProductionPromotionJson -Path $paths.retained_attestation
    # Neutral, shared build machinery; see Invoke-ReleaseManual.ps1.
    $attestationHelper = Join-Path $repo 'portable_replay\native\scripts\ProductionArtifact.Attestation.ps1'
    $captureDriver = Join-Path $repo 'extractor\native\scripts\Start-Hoi4ProductionCaptureSafe.ps1'
    $lifecycleHelper = Join-Path $repo 'extractor\native\scripts\PassiveCapture.Lifecycle.ps1'
    . $attestationHelper
    . $lifecycleHelper
    $currentArtifactAuthority = Test-ProductionArtifactAttestation `
        -RepoRoot $repo `
        -ReceiptPath $currentAttestationPath `
        -ExpectedDriverPath $captureDriver
    $retainedArtifactAuthority = Test-ProductionArtifactAttestation `
        -RepoRoot $repo `
        -ReceiptPath $paths.retained_attestation `
        -ExpectedDriverPath $captureDriver
    if ([string]$currentArtifactAuthority.source_input_sha256 -cne
            [string]$retainedArtifactAuthority.source_input_sha256 -or
        (($currentArtifactAuthority.artifact_sha256 | ConvertTo-Json -Compress) -cne
            ($retainedArtifactAuthority.artifact_sha256 | ConvertTo-Json -Compress))) {
        throw 'captured prebuilt artifact authority differs from the current Release'
    }

    # Required STRINGS, not "whatever casts to one". Casting first meant a
    # receipt publishing `"capture_id": 123` compared equal to a plan and a
    # manifest publishing the string "123", so a receipt of the wrong type
    # authenticated a capture it never named.
    $captureId = Get-ProductionPromotionRequiredString `
        $receipt 'capture_id' 'capture receipt'
    $sessionId = Get-ProductionPromotionRequiredString `
        $receipt 'session_id' 'capture receipt'
    # The plan's and the manifest's identities are required strings too. Casting
    # only the receipt's would have left `"capture_id": 123` in the MANIFEST
    # matching a receipt and plan that both say "123".
    $planCaptureId = Get-ProductionPromotionRequiredString `
        $plan 'capture_id' 'capture plan'
    $planSessionId = Get-ProductionPromotionRequiredString `
        $plan 'session_id' 'capture plan'
    $manifestCaptureId = Get-ProductionPromotionRequiredString `
        $manifest 'capture_id' 'capture manifest'
    $manifestSessionId = Get-ProductionPromotionRequiredString `
        $manifest 'session_id' 'capture manifest'
    if ([string](Get-ProductionPromotionProperty $plan 'schema') -cne
            'battle-plan.production_capture_plan.v1' -or
        $planCaptureId -cne $captureId -or
        $planSessionId -cne $sessionId -or
        $manifestCaptureId -cne $captureId -or
        $manifestSessionId -cne $sessionId -or
        [string](Get-ProductionPromotionProperty $plan 'exe_sha256') -cne
            '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad' -or
        (Get-ProductionPromotionRequiredInteger `
            $plan 'speed_required_raw' 'capture plan') -ne 4 -or
        (Get-ProductionPromotionProperty $plan 'unpaused_required') -isnot [bool] -or
        -not (Get-ProductionPromotionProperty $plan 'unpaused_required') -or
        (Get-ProductionPromotionProperty $plan 'read_only_scheduler_required') -isnot [bool] -or
        -not (Get-ProductionPromotionProperty $plan 'read_only_scheduler_required')) {
        throw 'capture plan has no exact production speed-five identity authority'
    }
    foreach ($closure in @(
        (Get-ProductionPromotionProperty $plan 'interaction_closure'),
        (Get-ProductionPromotionProperty $receipt 'interaction_closure')
    )) {
        # Four counts that must all be zero. An interaction closure that simply
        # omitted them read as four zeroes -- "we drove nothing" asserted by a
        # document that says nothing at all.
        $closureCounts = @(
            'client_commands_during_capture',
            'semantic_engine_state_writes',
            'game_process_lifecycle_commands',
            'game_ui_commands'
        ) | ForEach-Object {
            Get-ProductionPromotionRequiredInteger `
                $closure $_ 'capture interaction closure'
        }
        if ([string](Get-ProductionPromotionProperty $closure 'policy') -cne
                'passive_injected_observer' -or
            @($closureCounts | Where-Object { $_ -ne 0 }).Count -ne 0) {
            throw 'capture interaction closure permits production client driving'
        }
    }

    $speedFirstHour = Get-ProductionPromotionProperty $speed 'first_game_hour_raw'
    $speedLastHour = Get-ProductionPromotionProperty $speed 'last_game_hour_raw'
    $speedCaptureFirstHour =
        Get-ProductionPromotionProperty $speed 'capture_first_game_hour_raw'
    $speedCaptureLastHour =
        Get-ProductionPromotionProperty $speed 'capture_last_game_hour_raw'
    $manifestFirstHour =
        Get-ProductionPromotionProperty $manifest 'first_game_hour_raw'
    $manifestLastHour =
        Get-ProductionPromotionProperty $manifest 'last_game_hour_raw'
    $speedSampleCount =
        Get-ProductionPromotionProperty $speed 'sample_count'
    foreach ($hour in @(
        $speedFirstHour,
        $speedLastHour,
        $speedCaptureFirstHour,
        $speedCaptureLastHour,
        $manifestFirstHour,
        $manifestLastHour
    )) {
        if ($hour -isnot [int] -and $hour -isnot [long]) {
            throw 'capture speed-five proof has a non-integer game-hour boundary'
        }
    }
    if ([string](Get-ProductionPromotionProperty $speed 'schema') -cne
            'battle-plan.speed5_proof.v2' -or
        [string](Get-ProductionPromotionProperty $speed 'session_id') -cne $sessionId -or
        [string](Get-ProductionPromotionProperty $speed 'acceptance_mode') -cne 'production' -or
        (Get-ProductionPromotionRequiredInteger `
            $speed 'speed_raw' 'speed-five proof') -ne 4 -or
        (Get-ProductionPromotionProperty $speed 'paused') -isnot [bool] -or
        (Get-ProductionPromotionProperty $speed 'paused') -or
        ($speedSampleCount -isnot [int] -and
            $speedSampleCount -isnot [long]) -or
        [int64]$speedSampleCount -lt 3 -or
        [int](Get-ProductionPromotionProperty $speed 'pid') -ne
            [int](Get-ProductionPromotionProperty $plan 'pid') -or
        [int](Get-ProductionPromotionProperty $speed 'pid') -le 0 -or
        [string](Get-ProductionPromotionProperty $speed 'exe_sha256') -cne
            '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad' -or
        [string](Get-ProductionPromotionProperty $speed 'monitor_sha256') -cnotmatch
            '^[0-9a-f]{64}$' -or
        [string](Get-ProductionPromotionProperty $speed 'collector_control_sha256') -cnotmatch
            '^[0-9a-f]{64}$' -or
        [string](Get-ProductionPromotionProperty $speed 'domain57_game_speed_gate_v2_sha256') -cnotmatch
            '^[0-9a-f]{64}$' -or
        [int64]$speedFirstHour -gt [int64]$speedCaptureFirstHour -or
        [int64]$speedCaptureFirstHour -ne
            [int64]$manifestFirstHour -or
        [int64]$speedCaptureLastHour -ne
            [int64]$manifestLastHour -or
        [int64]$speedCaptureFirstHour -gt [int64]$speedCaptureLastHour -or
        [int64]$speedCaptureLastHour -gt [int64]$speedLastHour -or
        [string](Get-ProductionPromotionProperty $speed 'capture_plan_sha256') -cne
            (Get-ProductionPromotionSha256 -Path $paths.capture_plan)) {
        throw 'capture has no continuous unpaused speed-five proof'
    }

    $registryPath = Join-Path $repo 'extractor\native\domain_registry.json'
    $registry = Read-ProductionPromotionJson -Path $registryPath
    if ([string](Get-ProductionPromotionProperty $registry 'schema') -cne
        'battle-plan.compact_domain_registry.v1') {
        throw 'production domain registry schema is invalid'
    }
    # `capture_scheduled: false` marks a domain the owner ruled out of capture
    # scope; the key is absent for every domain that is still scheduled. The
    # capture plan's scheduled roster is derived from the same authority, so
    # leaving these in would make every promotion fail the roster count and
    # ordering checks below. Owned by
    # compact_contract::DESCHEDULED_CAPTURE_DOMAIN_IDS.
    $expectedDomains = @(
        @(Get-ProductionPromotionProperty $registry 'domains') |
            Where-Object {
                # Read through PSObject.Properties: the key is ABSENT on every
                # domain that is still scheduled -- it marks the exception, not
                # the rule -- and a direct
                # `$_.capture_scheduled` is a terminating PropertyNotFoundStrict
                # under Set-StrictMode -Version Latest. Four scripts carried the
                # same direct read; this is one of them.
                $descheduled = $_.PSObject.Properties['capture_scheduled']
                $_.schedulable -eq $true -and
                -not ($null -ne $descheduled -and $descheduled.Value -eq $false) -and
                $script:ProductionPromotionStatuses -ccontains [string]$_.status
            } |
            Sort-Object { [int]$_.id }
    )
    if ($expectedDomains.Count -le 0) {
        throw 'production domain registry has no promotable domains'
    }
    $expectedCanonical = @(
        $expectedDomains | ForEach-Object {
            [ordered]@{ id = [int]$_.id; name = [string]$_.name }
        }
    )
    if ((@(Get-ProductionPromotionProperty $plan 'production_profile_domain_roster') |
            ConvertTo-Json -Depth 4 -Compress) -cne
        ($expectedCanonical | ConvertTo-Json -Depth 4 -Compress)) {
        throw 'capture plan omits or reorders current production domains'
    }
    $scheduled = @(Get-ProductionPromotionProperty $plan 'scheduled_domain_roster')
    $scheduledNames = @(Get-ProductionPromotionProperty $plan 'scheduled_domain_names')
    if ($scheduled.Count -ne $expectedDomains.Count -or
        $scheduledNames.Count -ne $expectedDomains.Count) {
        throw 'capture plan scheduled-domain roster is incomplete'
    }
    $authorityPath = Resolve-ContainedProductionPromotionPath `
        -RunDir $runDir `
        -Value ([string](Get-ProductionPromotionProperty $plan 'domain_roster_authority_path')) `
        -Name 'domain_roster_authority'
    Assert-ProductionPromotionHash `
        -Path $authorityPath `
        -Expected ([string](Get-ProductionPromotionProperty $plan 'domain_roster_authority_sha256')) `
        -Name 'domain_roster_authority'
    # The roster document has to land in a variable before it is wrapped.
    # ConvertFrom-Json emits a top-level JSON array as a single un-enumerated
    # object, and that survives the function's return, so @(<call>) counts one
    # item no matter how many rows the file has - while $expectedDomains, built
    # through a Where-Object pipeline, counts them all. The two could never be
    # equal for a roster with more than one row, so this gate rejected every
    # well-formed authority it was ever given.
    $authorityDocument = Read-ProductionPromotionJson -Path $authorityPath
    $authority = @($authorityDocument)
    if ($authority.Count -ne $expectedDomains.Count) {
        throw 'capture domain-roster authority has a noncanonical row count'
    }
    for ($index = 0; $index -lt $expectedDomains.Count; $index++) {
        $domain = $expectedDomains[$index]
        $scheduledRow = $scheduled[$index]
        $authorityRow = $authority[$index]
        $wireName = [string](Get-ProductionPromotionProperty $scheduledRow 'wire_name')
        if ([int](Get-ProductionPromotionProperty $scheduledRow 'id') -ne [int]$domain.id -or
            [string]$scheduledNames[$index] -cne $wireName -or
            [int](Get-ProductionPromotionProperty $authorityRow 'id') -ne [int]$domain.id -or
            [string](Get-ProductionPromotionProperty $authorityRow 'name') -cne
                [string]$domain.name -or
            [string](Get-ProductionPromotionProperty $authorityRow 'wire_name') -cne
                $wireName -or [string]::IsNullOrWhiteSpace($wireName)) {
            throw "capture domain-roster authority is invalid at index $index"
        }
    }

    $targetPid = Get-ProductionPromotionProperty $plan 'pid'
    $targetCreationFiletimeUtc =
        Get-ProductionPromotionProperty $plan 'process_creation_filetime_utc'
    $requestedDurationMs =
        Get-ProductionPromotionProperty $plan 'requested_duration_ms'
    $schedulerPollMs =
        Get-ProductionPromotionProperty $plan 'read_only_scheduler_poll_ms'
    $schedulerQueueCapacity =
        Get-ProductionPromotionProperty $plan 'read_only_scheduler_queue_cap'
    $schedulerWorkerCount =
        Get-ProductionPromotionProperty $plan 'scheduler_worker_count'
    foreach ($integerInput in @(
        $targetPid,
        $targetCreationFiletimeUtc,
        $requestedDurationMs,
        $schedulerPollMs,
        $schedulerQueueCapacity,
        $schedulerWorkerCount
    )) {
        if ($integerInput -isnot [int] -and
            $integerInput -isnot [long] -and
            $integerInput -isnot [uint32] -and
            $integerInput -isnot [uint64]) {
            throw 'capture plan has a non-integer passive lifecycle input'
        }
    }
    $collectorDllSha256 =
        [string](Get-ProductionPromotionProperty $plan 'staged_dll_sha256')
    $attestedCollectorDllSha256 = [string](
        Get-ProductionPromotionProperty `
            (Get-ProductionPromotionProperty $currentArtifactAuthority 'artifact_sha256') `
            'collector_dll'
    )
    if ([uint64]$targetPid -eq 0 -or
        [uint64]$targetPid -gt [uint32]::MaxValue -or
        [uint64]$targetCreationFiletimeUtc -eq 0 -or
        [uint64]$requestedDurationMs -eq 0 -or
        [uint64]$requestedDurationMs -gt [uint32]::MaxValue -or
        [uint64]$schedulerPollMs -eq 0 -or
        [uint64]$schedulerPollMs -gt [uint32]::MaxValue -or
        [uint64]$schedulerQueueCapacity -gt [uint32]::MaxValue -or
        [uint64]$schedulerWorkerCount -eq 0 -or
        [uint64]$schedulerWorkerCount -gt [uint32]::MaxValue -or
        $collectorDllSha256 -cnotmatch '^[0-9a-f]{64}$' -or
        $collectorDllSha256 -cne $attestedCollectorDllSha256) {
        throw 'capture plan has no exact attested passive lifecycle identity'
    }
    $collectorControlPath = Resolve-ContainedProductionPromotionPath `
        -RunDir $runDir `
        -Value 'collector_control.json' `
        -Name 'collector_control'
    if ([string](Get-ProductionPromotionProperty $speed 'collector_control_sha256') -cne
        (Get-ProductionPromotionSha256 -Path $collectorControlPath)) {
        throw 'speed-five proof does not bind the authenticated collector control'
    }
    $readyAuthority = Test-PassiveCaptureReadyReceipt `
        -RunDir $runDir `
        -TargetPid ([uint32]$targetPid) `
        -TargetCreationFiletimeUtc ([uint64]$targetCreationFiletimeUtc) `
        -SessionId $sessionId `
        -CaptureId $captureId `
        -RequestedDurationMs ([uint32]$requestedDurationMs) `
        -TargetExecutableSha256 `
            '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad' `
        -CollectorDllSha256 $collectorDllSha256 `
        -ScheduledDomainNames $scheduledNames `
        -SchedulerPollMs ([uint32]$schedulerPollMs) `
        -SchedulerQueueCapacity ([uint32]$schedulerQueueCapacity) `
        -SchedulerWorkerCount ([uint32]$schedulerWorkerCount) `
        -CollectorControlPath $collectorControlPath
    $teardownAuthority = Test-PassiveCaptureTeardownReceipt `
        -RunDir $runDir `
        -TargetPid ([uint32]$targetPid) `
        -TargetCreationFiletimeUtc ([uint64]$targetCreationFiletimeUtc) `
        -SessionId $sessionId `
        -CaptureId $captureId `
        -ReadyAuthority $readyAuthority
    if ([IO.Path]::GetFullPath([string]$readyAuthority.ReceiptPath) -ine
            [IO.Path]::GetFullPath([string]$paths.ready) -or
        [IO.Path]::GetFullPath([string]$teardownAuthority.ReceiptPath) -ine
            [IO.Path]::GetFullPath([string]$paths.teardown)) {
        throw 'capture receipt paths differ from independently authenticated lifecycle receipts'
    }

    $compact = Get-ProductionPromotionProperty $manifest 'compact_stream'
    $frameCount = Get-ProductionPromotionProperty $compact 'frame_end_record_count'
    # The gap ledger is RUN ENCODED (`recorder_host::gap_ledger`): the flat
    # `domain_gap_outcomes` array cost 33.5 KB of manifest per game hour. This
    # gate never walked the rows -- it required the list to be PUBLISHED and to
    # be EMPTY -- so it reads the ledger's own declared count, which keeps both
    # halves: a missing count throws, and a nonzero one fails the zero-loss
    # clause below exactly as a nonempty array did.
    #
    # The count is not taken on trust. Two independent full expansions check it
    # against the classes they actually expand: the validator
    # (`Expand-CaptureGapOutcomes`) and the sink verifier
    # (`verify_sink_reconciliation.py`), and this gate refuses to promote without
    # both of their reports.
    # Read WITHOUT enumeration, and refuse an array: `Get-ProductionPromotionProperty`
    # returns through the pipeline, so a one-element array comes back as its
    # element and `"gap_ledger": [{...}]` promoted as the object inside it.
    $gapLedger = $null
    if ($null -ne $compact -and $null -ne $compact.PSObject.Properties['gap_ledger']) {
        $gapLedger = $compact.PSObject.Properties['gap_ledger'].Value
    }
    if ($null -eq $gapLedger -or $gapLedger -is [System.Array]) {
        throw 'capture manifest compact_stream does not publish a gap_ledger object'
    }
    # The version is required as a NUMBER, and as an INTEGER-TYPED one.
    # `Get-ProductionPromotionRequiredInteger` deliberately accepts a whole-valued
    # [double] -- right for figures ConvertFrom-Json may widen, wrong here: the
    # recorder writes `2`, never `2.0`, and Rust, Python and the PowerShell
    # expander all refuse the float form. A gate that admits what its own
    # producer cannot emit, and what every reader of the document rejects, has a
    # second undocumented format.
    Assert-ProductionPromotionJsonIntegerToken `
        $gapLedger 'version' 'capture manifest gap_ledger'
    $gapLedgerVersion = Get-ProductionPromotionRequiredInteger `
        $gapLedger 'version' 'capture manifest gap_ledger'
    if ([string](Get-ProductionPromotionProperty $gapLedger 'schema') -cne 'hoi4-gap-ledger-v2' -or
        $gapLedgerVersion -ne 2) {
        throw 'capture manifest publishes an unrecognised gap_ledger schema or version'
    }
    $gapOutcomeTable = $null
    if ($null -ne $gapLedger.PSObject.Properties['domain_gap_outcomes']) {
        $gapOutcomeTable = $gapLedger.PSObject.Properties['domain_gap_outcomes'].Value
    }
    if ($null -eq $gapOutcomeTable -or $gapOutcomeTable -is [System.Array]) {
        throw 'capture manifest gap_ledger does not publish a domain_gap_outcomes object'
    }
    Assert-ProductionPromotionJsonIntegerToken `
        $gapOutcomeTable 'row_count' 'capture manifest gap_ledger domain_gap_outcomes'
    $gapRowCount = Get-ProductionPromotionRequiredInteger `
        $gapOutcomeTable 'row_count' 'capture manifest gap_ledger domain_gap_outcomes'
    if ($frameCount -isnot [int] -and $frameCount -isnot [long]) {
        throw 'capture frame count is not an integer'
    }
    $frameCount = [int64]$frameCount
    # Every clause below required a value to be ZERO, and every one of them read
    # a missing property as zero. They now demand the field.
    $manifestSkippedGameHours = Get-ProductionPromotionRequiredInteger `
        $manifest 'skipped_game_hours' 'capture manifest'
    $manifestUniqueGameHours = Get-ProductionPromotionRequiredInteger `
        $manifest 'unique_game_hour_count' 'capture manifest'
    $compactGapRecordCount = Get-ProductionPromotionRequiredInteger `
        $compact 'domain_gap_record_count' 'capture manifest compact_stream'
    $compactFailureFlags = Get-ProductionPromotionRequiredUInt32 `
        $compact 'failure_status_flags_or' 'capture manifest compact_stream'
    $compactFailureSeen = Get-ProductionPromotionProperty $compact 'failure_status_seen'
    if ($compactFailureSeen -isnot [bool]) {
        throw 'capture manifest compact_stream does not publish a boolean failure_status_seen'
    }
    if ($frameCount -le 0 -or
        $manifestSkippedGameHours -ne 0 -or
        $manifestUniqueGameHours -ne $frameCount -or
        $compactGapRecordCount -ne 0 -or
        $gapRowCount -ne 0 -or
        ($compactFailureFlags -band $script:ProductionPromotionFailureMask) -ne 0 -or
        ($compactFailureFlags -band $script:ProductionPromotionUnknownStatusMask) -ne 0 -or
        $compactFailureSeen) {
        throw 'capture is not a zero-loss all-hour production stream'
    }
    # An undeclared diagnostic unit means a published figure whose aggregation
    # rule nobody stated. See
    # `compact_contract::diagnostic_channel::Accumulation`.
    $accumulation = Get-ProductionPromotionProperty $compact 'diagnostic_unit_accumulation'
    if ($null -eq $accumulation) {
        throw 'capture manifest compact_stream does not publish diagnostic_unit_accumulation'
    }
    if ([string](Get-ProductionPromotionProperty $accumulation 'schema') -cne
            'battle-plan.diagnostic_unit_accumulation.v1') {
        throw 'capture manifest publishes an unrecognised diagnostic_unit_accumulation schema'
    }
    $undeclaredUnits = @(Get-ProductionPromotionProperty $accumulation 'undeclared')
    if ((Get-ProductionPromotionRequiredInteger `
                $accumulation 'undeclared_unit_count' 'diagnostic_unit_accumulation') -ne
            $undeclaredUnits.Count) {
        throw 'diagnostic_unit_accumulation undeclared count disagrees with its own list'
    }
    if ($undeclaredUnits.Count -ne 0) {
        throw ('capture publishes ' + $undeclaredUnits.Count +
            ' diagnostic unit(s) with no declared accumulation class')
    }
    if ((@(Get-ProductionPromotionProperty $manifest 'scheduled_domains') -join "`0") -cne
        ($scheduledNames -join "`0")) {
        throw 'capture manifest scheduled-domain roster differs from the plan'
    }
    $perDomain = Get-ProductionPromotionProperty $compact 'per_domain'
    $domainCompletenessReceipt = [System.Collections.Generic.List[object]]::new()
    for ($index = 0; $index -lt $expectedDomains.Count; $index++) {
        $wireName = [string]$scheduledNames[$index]
        $row = Get-ProductionPromotionProperty $perDomain $wireName
        # `$row` is required BEFORE anything is read out of it: the old code
        # tested `$null -eq $row` inside the same clause that had already
        # coerced two of its properties, and `[uint32]` on a property of $null
        # is 0, so a missing domain row reached the failure-mask test as a clean
        # status word.
        if ($null -eq $row) {
            throw "capture manifest has no per_domain row for $wireName"
        }
        $rowContext = "capture manifest per_domain.$wireName"
        $rawStatusFlags = Get-ProductionPromotionRequiredUInt32 `
            $row 'status_flags_or' $rowContext
        # THE field the manifest did not publish. Absent, it read as 0 and every
        # domain passed the two clauses below for want of data.
        $effectiveStatusFlags = Get-ProductionPromotionRequiredUInt32 `
            $row 'effective_failure_status_flags_or' $rowContext
        $rowDomainId = Get-ProductionPromotionRequiredInteger `
            $row 'domain_id' $rowContext
        $rowRecordCount = Get-ProductionPromotionRequiredInteger `
            $row 'record_count' $rowContext
        $rowDataRecordCount = Get-ProductionPromotionRequiredInteger `
            $row 'data_record_count' $rowContext
        # The lane-local mask is selected by the PLAN's domain id, never by the
        # manifest's, so a manifest cannot nominate the exemption it wants. The
        # two are required to agree by the first clause below.
        $globalStatusFlags = Get-ProductionPromotionGlobalStatusFlags `
            -DomainId ([int]$expectedDomains[$index].id) -StatusFlags $rawStatusFlags
        # Definedness is tested on the globalized word: a REGISTERED lane badge
        # is defined, and an undefined bit still survives the strip. The failure
        # mask deliberately stays on the RAW word -- whether a lane-local badge
        # that collides with a global failure bit (d51 bit 12) should block
        # promotion is a policy question this gate keeps answering "yes" to.
        if ($rowDomainId -ne [int]$expectedDomains[$index].id -or
            $rowRecordCount -ne $frameCount -or
            $rowDataRecordCount -ne $frameCount -or
            ($rawStatusFlags -band $script:ProductionPromotionFailureMask) -ne 0 -or
            ($effectiveStatusFlags -band $script:ProductionPromotionFailureMask) -ne 0 -or
            ($globalStatusFlags -band $script:ProductionPromotionUnknownStatusMask) -ne 0 -or
            ($effectiveStatusFlags -band $script:ProductionPromotionUnknownStatusMask) -ne 0) {
            throw "production domain $($expectedDomains[$index].id) lacks real-source completeness"
        }
        $domainCompletenessReceipt.Add([ordered]@{
            id = [int]$expectedDomains[$index].id
            name = [string]$expectedDomains[$index].name
            wire_name = $wireName
            record_count = $rowRecordCount
            data_record_count = $rowDataRecordCount
            raw_failure_status_flags_or = $rawStatusFlags
            effective_failure_status_flags_or = $effectiveStatusFlags
            real_source_complete =
                ($rowDataRecordCount -eq $frameCount -and
                ($rawStatusFlags -band $script:ProductionPromotionFailureMask) -eq 0 -and
                ($effectiveStatusFlags -band $script:ProductionPromotionFailureMask) -eq 0 -and
                ($globalStatusFlags -band $script:ProductionPromotionUnknownStatusMask) -eq 0 -and
                ($effectiveStatusFlags -band $script:ProductionPromotionUnknownStatusMask) -eq 0)
        })
    }
    if (@($perDomain.PSObject.Properties).Count -ne $expectedDomains.Count) {
        throw 'capture manifest contains a noncanonical production domain census'
    }
    $allDomainsRealSourceComplete =
        ($domainCompletenessReceipt.Count -eq $expectedDomains.Count -and
        @($domainCompletenessReceipt | Where-Object {
            (Get-ProductionPromotionProperty $_ 'real_source_complete') -ne $true
        }).Count -eq 0)
    if (-not $allDomainsRealSourceComplete) {
        throw 'capture has no exact all-domain real-source completeness receipt'
    }

    if ([string](Get-ProductionPromotionProperty $validation 'schema') -cne
            'battle-plan.production_capture_validation.v2' -or
        (Get-ProductionPromotionProperty $validation 'ok') -isnot [bool] -or
        -not (Get-ProductionPromotionProperty $validation 'ok') -or
        (Get-ProductionPromotionRequiredArray `
            $validation 'problems' 'production validation report').Count -ne 0 -or
        [string](Get-ProductionPromotionProperty $validation 'manifest_sha256') -cne
            (Get-ProductionPromotionSha256 -Path $paths.manifest) -or
        [int](Get-ProductionPromotionProperty $validation 'expected_domain_count') -ne
            $expectedDomains.Count -or
        ((@(Get-ProductionPromotionProperty $validation 'expected_domain_roster') |
                ConvertTo-Json -Depth 4 -Compress) -cne
            ($expectedCanonical | ConvertTo-Json -Depth 4 -Compress))) {
        throw 'production validation report has no exact all-domain pass'
    }
    $compactGates = @(
        @(Get-ProductionPromotionProperty $validation 'independent_gates') |
            Where-Object { [string]$_.name -ceq 'compact_gate' }
    )
    if ($compactGates.Count -ne 1 -or
        (Get-ProductionPromotionProperty $compactGates[0] 'passed') -isnot [bool] -or
        -not (Get-ProductionPromotionProperty $compactGates[0] 'passed') -or
        (@(Get-ProductionPromotionProperty $compactGates[0] 'arguments') -cnotcontains
            '--require-zero-loss')) {
        throw 'production validation lacks an independent zero-loss compact gate'
    }
    $speedGates = @(
        @(Get-ProductionPromotionProperty $validation 'independent_gates') |
            Where-Object { [string]$_.name -ceq 'speed5_stream_gate' }
    )
    $speedGateExitCode = if ($speedGates.Count -eq 1) {
        Get-ProductionPromotionProperty $speedGates[0] 'exit_code'
    }
    else {
        $null
    }
    if ($speedGates.Count -ne 1 -or
        (Get-ProductionPromotionProperty $speedGates[0] 'passed') -isnot [bool] -or
        -not (Get-ProductionPromotionProperty $speedGates[0] 'passed') -or
        (Get-ProductionPromotionProperty $speedGates[0] 'started') -isnot [bool] -or
        -not (Get-ProductionPromotionProperty $speedGates[0] 'started') -or
        ($speedGateExitCode -isnot [int] -and
            $speedGateExitCode -isnot [long]) -or
        [int64]$speedGateExitCode -ne 0 -or
        (Get-ProductionPromotionProperty $speedGates[0] 'timed_out') -isnot [bool] -or
        (Get-ProductionPromotionProperty $speedGates[0] 'timed_out') -or
        $null -ne (Get-ProductionPromotionProperty $speedGates[0] 'problem')) {
        throw 'production validation lacks one successful executed speed-five stream gate'
    }
    $speedGateArguments = @(
        Get-ProductionPromotionProperty $speedGates[0] 'arguments'
    )
    $speedVerifierPath = Join-Path `
        $repo 'extractor\native\scripts\verify_speed5_stream.py'
    if ($speedGateArguments.Count -lt 2 -or
        [IO.Path]::GetFullPath([string]$speedGateArguments[0]) -ine
            [IO.Path]::GetFullPath($speedVerifierPath)) {
        throw 'speed-five stream gate did not execute the current canonical verifier'
    }
    $speedGateStreamPath = Resolve-ContainedProductionPromotionPath `
        -RunDir $runDir `
        -Value (Get-ProductionPromotionGateArgument `
            -Arguments $speedGateArguments -Switch '--stream') `
        -Name 'speed5_stream'
    $speedGateMonitorPath = Resolve-ContainedProductionPromotionPath `
        -RunDir $runDir `
        -Value (Get-ProductionPromotionGateArgument `
            -Arguments $speedGateArguments -Switch '--monitor') `
        -Name 'speed5_monitor'
    $speedGateReportPath = Resolve-ContainedProductionPromotionPath `
        -RunDir $runDir `
        -Value (Get-ProductionPromotionGateArgument `
            -Arguments $speedGateArguments -Switch '--out') `
        -Name 'speed5_stream_gate_report'
    $speedGateRootPath = [IO.Path]::GetFullPath(
        (Get-ProductionPromotionGateArgument `
            -Arguments $speedGateArguments -Switch '--root-gate')
    )
    $speedGateTranscriptPath = [IO.Path]::GetFullPath(
        (Get-ProductionPromotionGateArgument `
            -Arguments $speedGateArguments -Switch '--evidence-transcript')
    )
    $speedGateEvidenceVerifierPath = [IO.Path]::GetFullPath(
        (Get-ProductionPromotionGateArgument `
            -Arguments $speedGateArguments -Switch '--evidence-verifier')
    )
    $speedGateNonceLedgerRoot = [IO.Path]::GetFullPath(
        (Get-ProductionPromotionGateArgument `
            -Arguments $speedGateArguments -Switch '--nonce-ledger-root')
    )
    foreach ($path in @($speedGateRootPath, $speedGateTranscriptPath)) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "speed-five stream gate retained authority is missing: $path"
        }
    }
    if (-not (Test-Path -LiteralPath $speedGateEvidenceVerifierPath -PathType Leaf) -or
        -not (Test-Path -LiteralPath $speedGateNonceLedgerRoot -PathType Container) -or
        $speedGateNonceLedgerRoot -ine [IO.Path]::GetFullPath(
            [string](Get-ProductionPromotionProperty $plan 'speed_evidence_nonce_ledger_root')
        )) {
        throw 'speed-five stream gate verifier or nonce-ledger authority is missing'
    }
    $expectedEvidenceVerifierSha = Get-ProductionPromotionGateArgument `
        -Arguments $speedGateArguments -Switch '--expected-verifier-sha256'
    $expectedProbeSha = Get-ProductionPromotionGateArgument `
        -Arguments $speedGateArguments -Switch '--expected-probe-sha256'
    $expectedStaticSha = Get-ProductionPromotionGateArgument `
        -Arguments $speedGateArguments -Switch '--expected-static-sha256'
    if ($expectedEvidenceVerifierSha -cnotmatch '^[0-9a-f]{64}$' -or
        $expectedProbeSha -cnotmatch '^[0-9a-f]{64}$' -or
        $expectedStaticSha -cnotmatch '^[0-9a-f]{64}$' -or
        (Get-ProductionPromotionSha256 -Path $speedGateEvidenceVerifierPath) -cne
            $expectedEvidenceVerifierSha) {
        throw 'speed-five stream gate verifier/probe/static pins are invalid'
    }
    foreach ($binding in @(
        [pscustomobject]@{
            Switch = '--manifest'
            Expected = [IO.Path]::GetFullPath([string]$paths.manifest)
        },
        [pscustomobject]@{
            Switch = '--speed-proof'
            Expected = [IO.Path]::GetFullPath($speedPath)
        },
        [pscustomobject]@{
            Switch = '--capture-plan'
            Expected = [IO.Path]::GetFullPath([string]$paths.capture_plan)
        },
        [pscustomobject]@{
            Switch = '--collector-control'
            Expected = [IO.Path]::GetFullPath($collectorControlPath)
        }
    )) {
        $actual = [IO.Path]::GetFullPath(
            (Get-ProductionPromotionGateArgument `
                -Arguments $speedGateArguments `
                -Switch ([string]$binding.Switch))
        )
        if ($actual -ine [string]$binding.Expected) {
            throw "speed-five stream gate has a mismatched $($binding.Switch) binding"
        }
    }
    if ($speedGateMonitorPath -ine
            [IO.Path]::GetFullPath((Join-Path $runDir 'speed5_monitor.jsonl')) -or
        (Get-ProductionPromotionSha256 -Path $speedGateMonitorPath) -cne
            [string](Get-ProductionPromotionProperty $speed 'monitor_sha256') -or
        $speedGateRootPath -ine [IO.Path]::GetFullPath(
            [string](Get-ProductionPromotionProperty $plan 'domain57_game_speed_gate_v2')
        ) -or
        (Get-ProductionPromotionSha256 -Path $speedGateRootPath) -cne
            [string](Get-ProductionPromotionProperty $speed 'domain57_game_speed_gate_v2_sha256') -or
        (Get-ProductionPromotionSha256 -Path $speedGateRootPath) -cne
            [string](Get-ProductionPromotionProperty $plan 'domain57_game_speed_gate_v2_sha256') -or
        $speedGateTranscriptPath -ine [IO.Path]::GetFullPath(
            [string](Get-ProductionPromotionProperty $plan 'speed_evidence_transcript_path')
        ) -or
        (Get-ProductionPromotionSha256 -Path $speedGateTranscriptPath) -cne
            [string](Get-ProductionPromotionProperty $plan 'speed_evidence_transcript_file_sha256')) {
        throw 'speed-five stream gate evidence paths or hashes differ from the capture'
    }
    $speedGateReport = Read-ProductionPromotionJson -Path $speedGateReportPath
    foreach ($integerField in @(
        (Get-ProductionPromotionProperty $speedGateReport 'completed_frames'),
        (Get-ProductionPromotionProperty $speedGateReport 'first_game_hour_raw'),
        (Get-ProductionPromotionProperty $speedGateReport 'last_game_hour_raw'),
        (Get-ProductionPromotionProperty $speedGateReport 'monitor_samples')
    )) {
        if ($integerField -isnot [int] -and $integerField -isnot [long]) {
            throw 'speed-five stream gate report has a non-integer capture census'
        }
    }
    if ([string](Get-ProductionPromotionProperty $speedGateReport 'schema') -cne
            'battle-plan.speed5_stream_gate.v2' -or
        (Get-ProductionPromotionProperty $speedGateReport 'passed') -isnot [bool] -or
        -not (Get-ProductionPromotionProperty $speedGateReport 'passed') -or
        [int64](Get-ProductionPromotionProperty $speedGateReport 'completed_frames') -ne
            $frameCount -or
        [int64](Get-ProductionPromotionProperty $speedGateReport 'first_game_hour_raw') -ne
            [int64]$manifestFirstHour -or
        [int64](Get-ProductionPromotionProperty $speedGateReport 'last_game_hour_raw') -ne
            [int64]$manifestLastHour -or
        [int64](Get-ProductionPromotionProperty $speedGateReport 'monitor_samples') -ne
            [int64](Get-ProductionPromotionProperty $speed 'sample_count') -or
        [string](Get-ProductionPromotionProperty $speedGateReport 'domain57_game_speed_gate_v2_sha256') -cne
            (Get-ProductionPromotionSha256 -Path $speedGateRootPath) -or
        [string](Get-ProductionPromotionProperty $speedGateReport 'speed_evidence_transcript_file_sha256') -cne
            (Get-ProductionPromotionSha256 -Path $speedGateTranscriptPath) -or
        [string](Get-ProductionPromotionProperty $speedGateReport 'speed_evidence_transcript_sha256') -cne
            [string](Get-ProductionPromotionProperty $plan 'speed_evidence_transcript_sha256') -or
        [string](Get-ProductionPromotionProperty $speedGateReport 'nonce_seal_sha256') -cne
            [string](Get-ProductionPromotionProperty $plan 'speed_evidence_nonce_seal_sha256') -or
        [string](Get-ProductionPromotionProperty $speedGateReport 'collector_control_sha256') -cne
            (Get-ProductionPromotionSha256 -Path $collectorControlPath)) {
        throw 'speed-five stream gate report is absent, failed, or capture-unbound'
    }

    $sinkEvidence = Get-ProductionPromotionProperty $sink 'evidence'
    $sinkManifestSha =
        [string](Get-ProductionPromotionProperty $sinkEvidence 'manifest_sha256')
    $sinkPlanSha =
        [string](Get-ProductionPromotionProperty $sinkEvidence 'production_capture_plan_sha256')
    $sinkSpeedSha =
        [string](Get-ProductionPromotionProperty $sinkEvidence 'speed5_proof_sha256')
    if ([string](Get-ProductionPromotionProperty $sink 'schema') -cne
            'extractor.sink-reconciliation-verification.v2' -or
        [string](Get-ProductionPromotionProperty $sink 'expected_outcome') -cne 'success' -or
        [string](Get-ProductionPromotionProperty $sink 'verdict') -cne 'passed' -or
        (Get-ProductionPromotionRequiredArray `
            $sink 'problems' 'sink reconciliation report').Count -ne 0 -or
        [IO.Path]::GetFullPath([string](Get-ProductionPromotionProperty $sink 'run_dir')) -ine
            [IO.Path]::GetFullPath($runDir) -or
        $sinkManifestSha -cne
            (Get-ProductionPromotionSha256 -Path $paths.manifest) -or
        $sinkPlanSha -cne
            (Get-ProductionPromotionSha256 -Path $paths.capture_plan) -or
        $sinkSpeedSha -cne
            (Get-ProductionPromotionSha256 -Path $speedPath)) {
        throw 'independent sink reconciliation is absent or unbound'
    }

    foreach ($attestation in @($currentAttestation, $retainedAttestation)) {
        if ([string](Get-ProductionPromotionProperty $attestation 'schema') -cne
                'battle-plan.central_rust_prebuilt_artifacts.v1' -or
            [string](Get-ProductionPromotionProperty $attestation 'status') -cne
                'PASS_CENTRAL_RELEASE_PREBUILT_ARTIFACTS' -or
            (Get-ProductionPromotionProperty $attestation 'successful_exit') -isnot [bool] -or
            -not (Get-ProductionPromotionProperty $attestation 'successful_exit')) {
            throw 'capture artifact attestation lacks explicit ReleaseManual authority'
        }
    }
    $currentSource = Get-ProductionPromotionProperty $currentAttestation 'source_input'
    $retainedSource = Get-ProductionPromotionProperty $retainedAttestation 'source_input'
    $currentSourceSha = [string](Get-ProductionPromotionProperty $currentSource 'sha256')
    $retainedSourceSha = [string](Get-ProductionPromotionProperty $retainedSource 'sha256')
    if ($currentSourceSha -cnotmatch '^[0-9a-f]{64}$' -or
        $retainedSourceSha -cne $currentSourceSha -or
        [string](Get-ProductionPromotionProperty $receipt 'prebuilt_source_input_sha256') -cne
            $currentSourceSha -or
        [string](Get-ProductionPromotionProperty $plan 'prebuilt_source_input_sha256') -cne
            $currentSourceSha) {
        throw 'capture was not produced by the exact current production source inputs'
    }
    $currentRows = @(Get-ProductionPromotionProperty $currentAttestation 'artifacts')
    $retainedRows = @(Get-ProductionPromotionProperty $retainedAttestation 'artifacts')
    if ($currentRows.Count -eq 0 -or $retainedRows.Count -ne $currentRows.Count) {
        throw 'capture artifact attestation roster is incomplete'
    }
    $artifactHashes = [ordered]@{}
    foreach ($currentRow in $currentRows) {
        $role = [string](Get-ProductionPromotionProperty $currentRow 'role')
        # Not $matches. That name is PowerShell's automatic match-group
        # variable, and the -cnotmatch two lines down overwrites it with the
        # groups of its own successful match before the reads below run - so
        # $matches[0] was the hash string rather than the retained row, its
        # 'sha256' read came back $null, and this gate rejected every roster it
        # was ever given, including two byte-identical attestations.
        $retainedMatches = @($retainedRows | Where-Object { [string]$_.role -ceq $role })
        $hash = [string](Get-ProductionPromotionProperty $currentRow 'sha256')
        if ([string]::IsNullOrWhiteSpace($role) -or
            $hash -cnotmatch '^[0-9a-f]{64}$' -or
            $retainedMatches.Count -ne 1 -or
            [string](Get-ProductionPromotionProperty $retainedMatches[0] 'sha256') -cne $hash) {
            throw "capture artifact bytes differ from the current Release: $role"
        }
        $artifactHashes[$role] = $hash
    }

    return [ordered]@{
        schema = 'battle-plan.production_capture_promotion_evidence.v1'
        capture_receipt_path = $receiptPath
        capture_receipt_sha256 = Get-ProductionPromotionSha256 -Path $receiptPath
        capture_id = $captureId
        session_id = $sessionId
        current_artifact_attestation_path = $currentAttestationPath
        current_artifact_attestation_sha256 =
            Get-ProductionPromotionSha256 -Path $currentAttestationPath
        captured_artifact_attestation_sha256 =
            Get-ProductionPromotionSha256 -Path $paths.retained_attestation
        source_input_sha256 = $currentSourceSha
        artifact_sha256 = $artifactHashes
        production_domain_registry_sha256 =
            Get-ProductionPromotionSha256 -Path $registryPath
        production_domain_count = $expectedDomains.Count
        production_domain_roster = $expectedCanonical
        frame_count = $frameCount
        per_domain_real_source_receipt = @($domainCompletenessReceipt)
        all_domains_real_source_complete = $allDomainsRealSourceComplete
        passive_lifecycle_evidence = [ordered]@{
            ready_receipt_sha256 = [string]$readyAuthority.ReceiptSha256
            ready_payload_sha256 = [string]$readyAuthority.ReadyPayloadSha256
            ready_ack_sha256 = [string]$readyAuthority.ReadyAckSha256
            host_binding_sha256 = [string]$readyAuthority.HostBindingSha256
            active_hook_count = [uint32]$readyAuthority.ActiveHookCount
            teardown_receipt_sha256 =
                [string]$teardownAuthority.ReceiptSha256
            data_pipe_eof_observed =
                [bool]$teardownAuthority.DataPipeEofObserved
            hook_lease_released_after_eof =
                [bool]$teardownAuthority.HookLeaseReleasedAfterEof
            collector_module_disposition =
                [string]$teardownAuthority.CollectorModuleDisposition
            target_process_epoch_revalidated =
                [bool]$teardownAuthority.TargetProcessEpochRevalidated
            target_code_temporarily_instrumented =
                [bool]$teardownAuthority.TargetCodeTemporarilyInstrumented
            pinned_hook_target_code_restored =
                [bool]$teardownAuthority.PinnedHookTargetCodeRestored
            pinned_hook_sites_restored =
                [bool]$teardownAuthority.PinnedHookSitesRestored
            pinned_hook_site_count =
                [uint32]$teardownAuthority.PinnedHookSiteCount
        }
        zero_loss_evidence = [ordered]@{
            manifest_sha256 = Get-ProductionPromotionSha256 -Path $paths.manifest
            validation_report_sha256 =
                Get-ProductionPromotionSha256 -Path $paths.validation
            sink_reconciliation_report_sha256 =
                Get-ProductionPromotionSha256 -Path $paths.sink
            frame_count = $frameCount
            unique_game_hour_count = $manifestUniqueGameHours
            skipped_game_hours = $manifestSkippedGameHours
            domain_gap_record_count = $compactGapRecordCount
            independent_compact_gate_require_zero_loss = $true
            undeclared_diagnostic_unit_count = $undeclaredUnits.Count
        }
        zero_loss = (
            $manifestSkippedGameHours -eq 0 -and
            $compactGapRecordCount -eq 0 -and
            $gapRowCount -eq 0
        )
        speed5_evidence = [ordered]@{
            speed5_proof_sha256 = Get-ProductionPromotionSha256 -Path $speedPath
            speed5_stream_gate_report_sha256 =
                Get-ProductionPromotionSha256 -Path $speedGateReportPath
            sample_count = [int64](Get-ProductionPromotionProperty $speed 'sample_count')
            pid = [int](Get-ProductionPromotionProperty $speed 'pid')
            exe_sha256 = [string](Get-ProductionPromotionProperty $speed 'exe_sha256')
            monitor_sha256 =
                [string](Get-ProductionPromotionProperty $speed 'monitor_sha256')
            collector_control_sha256 =
                [string](Get-ProductionPromotionProperty $speed 'collector_control_sha256')
            first_game_hour_raw =
                [int](Get-ProductionPromotionProperty $speed 'first_game_hour_raw')
            last_game_hour_raw =
                [int](Get-ProductionPromotionProperty $speed 'last_game_hour_raw')
            capture_first_game_hour_raw =
                [int](Get-ProductionPromotionProperty $speed 'capture_first_game_hour_raw')
            capture_last_game_hour_raw =
                [int](Get-ProductionPromotionProperty $speed 'capture_last_game_hour_raw')
        }
        speed_raw = [int](Get-ProductionPromotionProperty $speed 'speed_raw')
        unpaused = (-not [bool](Get-ProductionPromotionProperty $speed 'paused'))
    }
}
