<#
.SYNOPSIS
    Shared, fail-closed validation for a production capture run.

.DESCRIPTION
    Dot-source this file to reuse the gate functions. It carries two layers:

      * Test-ProductionCaptureArtifacts - the heavyweight production gate over a
        real run directory: registry roster, per-domain effective-failure
        fail-closed checks, passive per-domain outcome accounting,
        container/fold/speed
        independent verifiers, AND the acceptance-telemetry schema gate below.

      * Test-ProductionCaptureManifest + Get-AcceptanceTelemetryProblems - the
        fail-closed acceptance-telemetry schema
        and numeric gates over a parsed manifest, reusable without a live run.

    Every check is FAIL-CLOSED: a missing manifest, a missing telemetry field, or
    an out-of-range value is a FAILURE, never a silent pass. A field that the
    recorder emits as JSON null (e.g. a percentile over an empty sample) counts as
    PRESENT for the schema gate but FAILS any numeric threshold, so an empty or
    truncated capture cannot masquerade as a good one.

    Invoke directly with -SelfTest to run the offline unit tests over fabricated
    manifests (no live game, no capture required).

    PowerShell 5.1 compatible. No rig hardcoding.
#>
param(
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not ("ProductionCaptureAtomicPublishNative" -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

public static class ProductionCaptureAtomicPublishNative {
    private const uint MOVEFILE_REPLACE_EXISTING = 0x00000001;
    private const uint MOVEFILE_WRITE_THROUGH = 0x00000008;

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool MoveFileExW(string existingName, string newName, uint flags);

    public static void ReplaceWriteThrough(string temporary, string destination) {
        if (!MoveFileExW(temporary, destination,
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
            throw new Win32Exception(Marshal.GetLastWin32Error(),
                "durable atomic publication failed: " + destination);
        }
    }
}
'@
}

$script:CanonicalCompactFailureMask = [uint32]0x13CF
# The same mask without PARTIAL_DATA (bit 9): domain 42's raw-word clause uses
# it when the boundary decoder counted ZERO undeclared partials -- every d42
# PARTIAL_DATA hour in the capture is then the declared attribution gap the
# compact contract tolerates. A missing or malformed counter never licenses.
# Mirrors ProductionCapture.Promotion.ps1's scoped raw-word mask.
$script:CanonicalCompactFailureMaskNoPartialData = [uint32]0x11CF
$script:RequiredStaticArtifactNames = @(
    "templates_equipment_catalog",
    "localisation_join",
    "decisions_static_definitions",
    "idea_tech_loc_xref_static",
    "ship_definitions_static",
    "gfx_icon_asset_manifest_static",
    "technology_sharing_static"
)
$script:ProductionDomainStatuses = @(
    "existing",
    "production_complete",
    "production_complete_base"
)

# Acceptance-telemetry (F5) schema fields. Per-frame extraction time lives under
# compact_stream; the new p50/p99 join the pre-existing avg/p95/max. Host sink
# timing and clock skew live in the dedicated acceptance_telemetry block.
$script:RequiredFrameTimeFields = @("count", "avg", "p50", "p95", "p99", "max")
$script:RequiredSinkWriteFields = @("count", "avg", "p50", "p95", "p99", "max")
$script:RequiredClockSkewFields = @("wall_elapsed_us", "mono_elapsed_us", "drift_us", "wall_monotonic_ok")

function Write-AtomicUtf8Text {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text
    )
    $fullPath = [System.IO.Path]::GetFullPath($Path)
    $parent = Split-Path -Parent $fullPath
    if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    $temp = Join-Path $parent (".{0}.{1}.tmp" -f [System.IO.Path]::GetFileName($fullPath), [guid]::NewGuid().ToString("N"))
    try {
        [System.IO.File]::WriteAllText($temp, $Text, [System.Text.UTF8Encoding]::new($false))
        $stream = [System.IO.File]::Open($temp, [System.IO.FileMode]::Open, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if (Test-Path -LiteralPath $fullPath -PathType Leaf) {
            [ProductionCaptureAtomicPublishNative]::ReplaceWriteThrough($temp, $fullPath)
        } else {
            [System.IO.File]::Move($temp, $fullPath)
        }
    } finally {
        if (Test-Path -LiteralPath $temp -PathType Leaf) { Remove-Item -LiteralPath $temp -Force }
    }
}

function Write-AtomicJson {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)]$Value,
        [ValidateRange(1, 100)][int]$Depth = 12
    )
    Write-AtomicUtf8Text -Path $Path -Text ($Value | ConvertTo-Json -Depth $Depth)
}

function Get-ObjectProperty {
    param($Object, [Parameter(Mandatory = $true)][string]$Name)
    if ($null -eq $Object) { return $null }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) { return $null }
    return $property.Value
}

$script:GapLedgerSchema = 'hoi4-gap-ledger-v2'
$script:GapLedgerVersion = 2
# The wire vocabularies. The per-row walk below already rejects a part or reason
# outside these sets with its own problem code; refusing them at expansion keeps
# this reader's ACCEPTED set identical to the recorder's, so the two never
# disagree about whether a document is readable at all.
$script:GapLedgerParts = @('state', 'event', 'both')
# The EXACT field set an outcome class may carry. A closed set is the structural
# close on the whole "ignored field" family: a document carrying something no
# reader uses expands to what the document without it does, and -- worse -- an
# unread field still counts toward a class's IDENTITY, so adding one splits an
# aggregation key and walks through the same-hour check.
$script:GapLedgerOutcomeFields = @(
    'reason', 'part', 'domains', 'queue_capacity', 'queue_depth', 'queue_dropped',
    'range_start_rel', 'range_end_rel', 'tick_before', 'tick_after', 'hour_runs',
    'ordinals'
)
$script:GapLedgerLaneFields = @('domain_id', 'part', 'first')
$script:GapLedgerDomainReasons = @(
    'torn', 'overrun', 'backpressure', 'clock_unreadable', 'worker_panic',
    'source_unavailable', 'reader_silent', 'hour_label_mismatch'
)

function ConvertTo-GapLedgerCanonicalText {
    <#
    .SYNOPSIS
    A value's canonical text, with object members ordered ALL THE WAY DOWN.

    .DESCRIPTION
    Two classes are the same class when they carry the same fields, whatever
    order the document happened to write them in. Sorting only the top level left
    a nested object's two spellings with different texts, so two classes that
    Rust and Python call duplicates slipped past the check here. A canonical form
    that is not canonical all the way down is not one.

    `Drop` names the top-level members to leave out -- the ones that carry a
    class's hours rather than its identity.
    #>
    param($Value, [string[]]$Drop = @())
    if ($null -eq $Value) { return 'null' }
    if ($Value -is [string]) { return '"' + $Value + '"' }
    if ($Value -is [System.Array]) {
        $parts = New-Object System.Collections.ArrayList
        foreach ($item in $Value) {
            [void]$parts.Add((ConvertTo-GapLedgerCanonicalText $item))
        }
        return '[' + (@($parts) -join ',') + ']'
    }
    if ($Value -is [ValueType]) { return [string]$Value }
    if ($null -ne $Value.PSObject -and @($Value.PSObject.Properties).Count -gt 0) {
        $parts = New-Object System.Collections.ArrayList
        foreach ($member in @($Value.PSObject.Properties) | Sort-Object -Property Name) {
            if ($Drop -ccontains $member.Name) { continue }
            [void]$parts.Add(
                ('"' + $member.Name + '":' + (ConvertTo-GapLedgerCanonicalText $member.Value)))
        }
        return '{' + (@($parts) -join ',') + '}'
    }
    return [string]$Value
}

function Expand-CaptureGapOutcomes {
    <#
    .SYNOPSIS
    The rows the flat `compact_stream.domain_gap_outcomes` array used to carry.

    .DESCRIPTION
    The sealed manifest now carries a RUN-ENCODED `gap_ledger` instead: the flat
    arrays cost 33.5 KB of JSON per game hour, ~3.4 GB over a full campaign, and
    were read whole and re-parsed at pack time. This expands the ledger back into
    exactly the rows the walk below grades, so the grading is unchanged.

    ONE FORMAT: there is no fallback to the flat array this replaced (owner
    ruling 2026-08-09). A missing ledger, or one whose schema/version is not the
    one implemented here, is REFUSED -- reported as a problem rather than read as
    an empty ledger, because "this reader could not read the ledger" and "the
    capture had no gaps" are opposite facts and only one of them promotes.

    Returns a hashtable: Ok (bool), Rows (array), Problem (string or $null).
    #>
    param($Compact)
    # NOTHING IN HERE MAY TERMINATE THE VALIDATOR. A cast that overflows, a
    # property that is not the type it looks like, a run pair with the wrong
    # arity -- every one of those is a PowerShell terminating error, and a
    # validator that dies mid-manifest publishes no verdict at all. A capture
    # with odd data must still be graded and still be sealable, so the whole
    # expansion is bounded here and any failure becomes a named problem.
    try {
        return Expand-CaptureGapOutcomesCore $Compact
    }
    catch {
        return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_unreadable' }
    }
}

function Expand-CaptureGapOutcomesCore {
    param($Compact)
    $ledger = $null
    if ($null -ne $Compact -and $null -ne $Compact.PSObject.Properties['gap_ledger']) {
        $ledger = $Compact.PSObject.Properties['gap_ledger'].Value
    }
    if ($null -eq $ledger -or $ledger -is [System.Array]) {
        return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_missing' }
    }
    # THE LEDGER ITSELF IS THE LAST CONTAINER, and it was the last one open.
    # Closing the classes and then their tables still left this escape one level
    # farther out. `compact_stream` above it is not this format's to close.
    foreach ($member in @($ledger.PSObject.Properties)) {
        if (@('schema', 'version', 'note', 'gaps',
              'domain_gap_outcomes') -cnotcontains $member.Name) {
            return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_unknown_field' }
        }
    }
    # AND `note` IS TYPED, not merely named: allowlisting the name left an
    # OBJECT-SHAPED HOLE, because a name allowlist says nothing about the value.
    $noteMember = $ledger.PSObject.Properties['note']
    if ($null -ne $noteMember -and $noteMember.Value -isnot [string]) {
        return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_note_not_a_string' }
    }
    # BOTH exact, and the version read as a NUMBER: `[string]` comparison would
    # admit the string "2", which the recorder cannot write, and a reader that
    # accepts what the writer cannot produce has a second undocumented format.
    $ledgerVersion = $null
    $ledgerSchema = $null
    if ($null -ne $ledger.PSObject.Properties['version']) {
        $ledgerVersion = $ledger.PSObject.Properties['version'].Value
    }
    if ($null -ne $ledger.PSObject.Properties['schema']) {
        $ledgerSchema = $ledger.PSObject.Properties['schema'].Value
    }
    if ($ledgerSchema -isnot [string] -or $ledgerSchema -cne $script:GapLedgerSchema -or
        -not (Test-GapLedgerIntegerToken $ledgerVersion) -or
        [bigint]$ledgerVersion -ne [bigint]$script:GapLedgerVersion) {
        return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_version_unreadable' }
    }
    $table = $null
    if ($null -ne $ledger.PSObject.Properties['domain_gap_outcomes']) {
        $table = $ledger.PSObject.Properties['domain_gap_outcomes'].Value
    }
    if ($null -eq $table -or $table -is [System.Array]) {
        return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_outcomes_missing' }
    }
    $encoding = $null
    if ($null -ne $table.PSObject.Properties['ordinal_encoding']) {
        $encoding = $table.PSObject.Properties['ordinal_encoding'].Value
    }
    if ($encoding -isnot [string] -or
        ($encoding -cne 'lane_sequential' -and $encoding -cne 'explicit')) {
        return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_ordinal_encoding_unknown' }
    }
    $explicit = $encoding -ceq 'explicit'
    # The two branches must not carry each other's fields: a `lane_sequential`
    # table ignores a class's `ordinals` and an `explicit` one ignores
    # `lane_ordinal_first`, so a document carrying the ignored field means what
    # the document without it means -- two documents, one expansion.
    if ($explicit -and $null -ne $table.PSObject.Properties['lane_ordinal_first']) {
        return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_lane_table_ignored' }
    }
    # CASE-SENSITIVE, ordinal. PowerShell's default hashtable matches keys
    # case-insensitively, so an uppercase lane entry matched a lowercase class
    # lane and the two disagreed with Rust and Python about which lane is which.
    $laneFirst = New-Object 'System.Collections.Hashtable' ([StringComparer]::Ordinal)
    if (-not $explicit) {
        # Required, as the recorder's own reader requires it. Absence already
        # ended in an error one step later (every lane reporting no base), but
        # not the SAME error, and a reader that fails differently from the
        # producer's is a reader nobody can reason about.
        if ($null -eq $table.PSObject.Properties['lane_ordinal_first']) {
            return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_lane_table_missing' }
        }
        $laneValue = $table.PSObject.Properties['lane_ordinal_first'].Value
        if ($null -eq $laneValue -or -not ($laneValue -is [System.Array])) {
            return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_lane_table_not_an_array' }
        }
        foreach ($entry in $laneValue) {
            # A null entry is REFUSED, not skipped: skipping it drops a lane's
            # base silently, and the other two readers reject it.
            if ($null -eq $entry) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_lane_entry_null' }
            }
            # `first` is REQUIRED. `[bigint]$null` is 0, so an absent base would
            # quietly number the whole lane from zero -- an ordinal nobody wrote,
            # attached to rows that did happen. A missing `domain_id` or `part`
            # cannot hide the same way: they only build a key that matches no
            # row's lane, and the lane then reports `gap_ledger_lane_first_missing`.
            $laneDomain = [int64](Get-GapLedgerInteger $entry 'domain_id')
            if ($laneDomain -lt 0 -or $laneDomain -gt 65535) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_lane_domain_invalid' }
            }
            foreach ($member in @($entry.PSObject.Properties)) {
                if ($script:GapLedgerLaneFields -cnotcontains $member.Name) {
                    return @{ Ok = $false; Rows = @()
                              Problem = 'gap_ledger_lane_unknown_field' }
                }
            }
            $lanePart = Get-GapLedgerName $entry 'part'
            if ($script:GapLedgerParts -cnotcontains $lanePart) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_lane_part_unknown' }
            }
            $key = "${laneDomain}:${lanePart}"
            # A DUPLICATE entry silently overwrote its predecessor, so two
            # different tables expanded identically.
            if ($laneFirst.ContainsKey($key)) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_lane_duplicate' }
            }
            $laneBase = [bigint](Get-GapLedgerWideInteger $entry 'first')
            if ($laneBase -gt [bigint]::Parse('18446744073709551615')) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_lane_first_out_of_range' }
            }
            $laneFirst[$key] = $laneBase
        }
    }
    # `classes` must BE PUBLISHED. `@($null)` is an empty array, so a table with
    # no `classes` at all graded as a capture with no gaps -- and the recorder's
    # own reader refuses it. Two readers of one document disagreeing about
    # whether it says anything.
    #
    # Tested for PRESENCE, not for array-ness: `Get-ObjectProperty` hands back a
    # one-element JSON array as the bare element, so an `-is [Array]` test would
    # reject the single-class table the recorder legitimately writes.
    # Read through `.PSObject.Properties[...].Value`, NOT `Get-ObjectProperty`:
    # the latter unrolls a one-element JSON array to the bare element, which is
    # why an earlier version of this check could not tell "one class" from "a
    # scalar where a list belongs". The property value keeps its `Object[]` type,
    # so the array shape can be required -- and it must be, or a scalar class
    # expands here while Rust and Python refuse it.
    if ($null -eq $table.PSObject.Properties['classes']) {
        return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_classes_missing' }
    }
    $classValue = $table.PSObject.Properties['classes'].Value
    if ($null -eq $classValue -or -not ($classValue -is [System.Array])) {
        return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_classes_not_an_array' }
    }
    # `class_count` is published beside the list and must agree with it: a count
    # that disagrees is a second claim about the same list, and the reader that
    # ignores it lets two documents differ in a field neither reads.
    $declaredClasses = $null
    if ($null -ne $table.PSObject.Properties['class_count']) {
        $declaredClasses = $table.PSObject.Properties['class_count'].Value
    }
    if (-not (Test-GapLedgerIntegerToken $declaredClasses) -or
        [int64]$declaredClasses -ne $classValue.Count) {
        return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_class_count_mismatch' }
    }
    # Read BEFORE the class loop: it is the bound on how much work any one class
    # may ask for, and the check against the expansion still runs at the end.
    $declaredRowCount = $null
    if ($null -ne $table.PSObject.Properties['row_count']) {
        $declaredRowCount = $table.PSObject.Properties['row_count'].Value
    }
    if (-not (Test-GapLedgerIntegerToken $declaredRowCount) -or [int64]$declaredRowCount -lt 0) {
        return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_row_count_invalid' }
    }
    # THE TABLE IS CLOSED TOO: closing the classes but not their container left
    # the same escape one level up.
    foreach ($member in @($table.PSObject.Properties)) {
        if (@('row_count', 'class_count', 'classes', 'ordinal_encoding',
              'lane_ordinal_first') -cnotcontains $member.Name) {
            return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_table_unknown_field' }
        }
    }
    $rows = New-Object System.Collections.ArrayList
    # TWO CLASSES WITH ONE IDENTITY WOULD HAVE BEEN ONE CLASS. Splitting a run
    # across two otherwise identical classes expands to exactly what the merged
    # class does, which the within-class maximal-run rule alone does not close.
    $seenIdentities = New-Object 'System.Collections.Hashtable' ([StringComparer]::Ordinal)
    $claimedHours = New-Object 'System.Collections.Hashtable' ([StringComparer]::Ordinal)
    foreach ($class in $classValue) {
        # A null class is REFUSED, not skipped, for the same reason.
        if ($null -eq $class) {
            return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_class_null' }
        }
        # Read as a PROPERTY VALUE and required to BE an array: `@(...)`
        # normalizes a scalar into a one-element list, so `"domains": 4` expanded
        # here while Rust and Python refuse it. A class must also name at least
        # one domain, or it expands to nothing and any number of such classes
        # mean what the empty table means.
        $domainValue = $null
        if ($null -ne $class.PSObject.Properties['domains']) {
            $domainValue = $class.PSObject.Properties['domains'].Value
        }
        if ($null -eq $domainValue -or -not ($domainValue -is [System.Array]) -or
            $domainValue.Count -eq 0) {
            return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_domains_invalid' }
        }
        # NON-DECREASING, as the encoder writes it: `[4,5]` and `[5,4]` expand to
        # the same rows after the reader's own sort. Not STRICTLY increasing -- a
        # repeated domain is legal and carries multiplicity.
        for ($d = 1; $d -lt $domainValue.Count; $d++) {
            if ([int64]$domainValue[$d - 1] -gt [int64]$domainValue[$d]) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_domains_unordered' }
            }
        }
        $domains = $domainValue
        # TYPE-CHECKED, not cast. `[int]4.5` is 4 in PowerShell, so a
        # fractional domain id expanded silently onto a DIFFERENT domain than
        # the one written, while Rust and Python refuse it.
        foreach ($domain in $domains) {
            if (-not (Test-GapLedgerIntegerToken $domain) -or
                [bigint]$domain -lt [bigint]0 -or [bigint]$domain -gt [bigint]65535) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_domain_invalid' }
            }
        }
        # RECURSIVELY canonical. Sorting only the top-level members left
        # `tick_before:{"rel":0,"x":1}` and `{"x":1,"rel":0}` with different
        # identities, so two classes that Rust and Python call duplicates slipped
        # past here. A canonical form that is not canonical all the way down is
        # not one.
        $identity = ConvertTo-GapLedgerCanonicalText $class @('hour_runs', 'ordinals')
        if ($seenIdentities.ContainsKey($identity)) {
            return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_duplicate_class_identity' }
        }
        $seenIdentities[$identity] = $true
        # Keyed WITHOUT the domains as well: the encoder aggregates every domain
        # affected at one hour into ONE class, so two classes differing only in
        # their domain list may not both name the same hour. The identity above
        # cannot see that, because the domain list is part of it.
        $aggregationKey =
            ConvertTo-GapLedgerCanonicalText $class @('hour_runs', 'ordinals', 'domains')
        if (-not $claimedHours.ContainsKey($aggregationKey)) {
            $claimedHours[$aggregationKey] =
                New-Object 'System.Collections.Hashtable' ([StringComparer]::Ordinal)
        }
        $hoursClaimed = $claimedHours[$aggregationKey]
        foreach ($member in @($class.PSObject.Properties)) {
            if ($script:GapLedgerOutcomeFields -cnotcontains $member.Name) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_class_unknown_field' }
            }
        }
        $classPart = Get-GapLedgerName $class 'part'
        $classReason = Get-GapLedgerName $class 'reason'
        if ($script:GapLedgerParts -cnotcontains $classPart) {
            return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_part_unknown' }
        }
        if ($script:GapLedgerDomainReasons -cnotcontains $classReason) {
            return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_reason_unknown' }
        }
        if (-not $explicit -and $null -ne $class.PSObject.Properties['ordinals']) {
            return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_ordinals_ignored' }
        }
        $ordinals = @()
        if ($explicit) {
            # Required to BE an array, for the same reason as `domains`.
            $ordinalValue = $null
            if ($null -ne $class.PSObject.Properties['ordinals']) {
                $ordinalValue = $class.PSObject.Properties['ordinals'].Value
            }
            if ($null -eq $ordinalValue -or -not ($ordinalValue -is [System.Array])) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_ordinals_invalid' }
            }
            $ordinals = $ordinalValue
        }
        # A class that names NO hour expands to nothing, so a table could carry
        # any number of them and still mean exactly what the empty table means.
        # The encoder never writes one, and admitting it makes the format
        # non-injective.
        # Assigned in two statements. `$x = if (...) { <one-element array> }`
        # UNWRAPS to the element, and `hour_runs` with a single run is exactly a
        # one-element array of arrays -- so the `if` form handed back the inner
        # `[first, count]` pair and every single-run class read as malformed.
        # This is the third distinct place PowerShell's unwrap bit this file.
        $runValue = $null
        if ($null -ne $class.PSObject.Properties['hour_runs']) {
            $runValue = $class.PSObject.Properties['hour_runs'].Value
        }
        if ($null -eq $runValue -or -not ($runValue -is [System.Array]) -or
            $runValue.Count -eq 0) {
            return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_hour_runs_empty' }
        }
        $position = 0
        $previousRunEnd = $null
        $classHours = [int64]0
        foreach ($run in $runValue) {
            if ($null -eq $run -or @($run).Count -ne 2) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_hour_run_malformed' }
            }
            # TYPE-CHECKED BEFORE THE CAST. `[int64]1.5` is 2 in PowerShell, so
            # a float run element expanded silently at a DIFFERENT hour than the
            # one written, while Rust and Python refuse the same input.
            if (-not (Test-GapLedgerIntegerToken $run[0]) -or
                -not (Test-GapLedgerIntegerToken $run[1])) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_hour_run_not_integral' }
            }
            $first = [int64]$run[0]
            $count = [int64]$run[1]
            # BOUNDED TO `u32`, as the recorder's own reader narrows it. Without
            # the bound, a count of 4294967296 is refused there and looped over
            # here -- a hang rather than a verdict.
            if ($count -lt 1 -or $count -gt 4294967295) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_hour_run_empty' }
            }
            # And bounded by the WORK the table admits to, BEFORE the walk:
            # 4294967295 is inside the `u32` and still 4.29 billion iterations.
            # `row_count` is itself checked against the expansion, so this bounds
            # the work by a number the document has to be honest about.
            # Counted in ROWS, not hours: an outcome class emits one row per
            # listed domain per hour, so an hour bound alone is the wrong bound.
            $classHours = $classHours + ($count * [int64][Math]::Max($domains.Count, 1))
            if ($classHours -gt [int64]$declaredRowCount) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_class_exceeds_row_count' }
            }
            # Hours are `i32` on the wire and in the recorder's own reader. A run
            # that walks outside that range names an hour which cannot exist and
            # which the producer's decoder refuses -- two readers of one document
            # disagreeing about which hour a gap belongs to.
            if ($first -lt -2147483648 -or ($first + $count - 1) -gt 2147483647) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_hour_out_of_range' }
            }
            # CANONICAL AND MAXIMAL. `[[101,1],[102,2]]` expands to exactly what
            # `[[101,3]]` does, so admitting both lets two documents mean one
            # thing. The encoder never splits a run or emits them out of order.
            # `-le $previousRunEnd + 1` catches both overlap AND adjacency: two
            # runs that touch would have been one run, so the split spelling is
            # the non-canonical one.
            if ($null -ne $previousRunEnd -and $first -le ($previousRunEnd + 1)) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_hour_runs_not_canonical' }
            }
            $previousRunEnd = $first + $count - 1
            for ($step = 0; $step -lt $count; $step++) {
                $target = $first + $step
                if ($hoursClaimed.ContainsKey([string]$target)) {
                    return @{ Ok = $false; Rows = @()
                              Problem = 'gap_ledger_hour_domains_not_aggregated' }
                }
                $hoursClaimed[[string]$target] = $true
                foreach ($domain in $domains) {
                    $tickBeforeField = $null
                    $tickAfterField = $null
                    if ($null -ne $class.PSObject.Properties['tick_before']) {
                        $tickBeforeField = $class.PSObject.Properties['tick_before'].Value
                    }
                    if ($null -ne $class.PSObject.Properties['tick_after']) {
                        $tickAfterField = $class.PSObject.Properties['tick_after'].Value
                    }
                    $tickBefore = Resolve-GapLedgerAnchor $tickBeforeField $target
                    $tickAfter = Resolve-GapLedgerAnchor $tickAfterField $target
                    if ($null -eq $tickBefore -or $null -eq $tickAfter) {
                        return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_tick_anchor_ambiguous' }
                    }
                    $ordinal = if ($explicit) {
                        if ($position -ge $ordinals.Count) {
                            return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_ordinals_short' }
                        }
                        # `Test-IsJsonInteger` caps at Int64 because its other
                        # callers cast through `[int64]`; an ordinal is a `u64`,
                        # and capping here refused exact `[decimal]` ordinals
                        # that Rust and Python accept.
                        $rawOrdinal = $ordinals[$position]
                        if (-not (Test-GapLedgerIntegerToken $rawOrdinal)) {
                            throw 'gap ledger explicit ordinal is not an integer'
                        }
                        if ([bigint]$rawOrdinal -lt [bigint]0 -or
                            [bigint]$rawOrdinal -gt [bigint]::Parse('18446744073709551615')) {
                            throw 'gap ledger explicit ordinal is outside the u64 range'
                        }
                        [bigint]$rawOrdinal
                    } else { [bigint]0 }
                    [void]$rows.Add([pscustomobject]@{
                        domain_id            = [int]$domain
                        part                 = $classPart
                        reason               = $classReason
                        target_hour_raw      = $target
                        range_start_hour_raw = Assert-GapLedgerHour (
                            $target + [int64](Get-GapLedgerInteger $class 'range_start_rel'))
                        range_end_hour_raw   = Assert-GapLedgerHour (
                            $target + [int64](Get-GapLedgerInteger $class 'range_end_rel'))
                        tick_before          = $tickBefore
                        tick_after           = $tickAfter
                        queue_capacity       = Assert-GapLedgerU32 (
                            [int64](Get-GapLedgerInteger $class 'queue_capacity'))
                        queue_depth          = Assert-GapLedgerU32 (
                            [int64](Get-GapLedgerInteger $class 'queue_depth'))
                        queue_dropped        = [bigint](Get-GapLedgerWideInteger $class 'queue_dropped')
                        outcome_ordinal      = $ordinal
                    })
                    $position++
                }
            }
        }
        if ($explicit -and $ordinals.Count -ne $position) {
            return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_ordinals_length_mismatch' }
        }
    }
    if (-not $explicit) {
        # One pass per (domain, part) lane in hour order, mirroring the check the
        # encoder ran before it chose this encoding. Two rows of one lane at one
        # hour makes the derivation ambiguous, and the encoder refuses to pick
        # `lane_sequential` then -- so a ledger claiming it here is one this
        # reader must not interpret.
        $lanes = New-Object 'System.Collections.Hashtable' ([StringComparer]::Ordinal)
        foreach ($row in $rows) {
            $key = "$($row.domain_id):$($row.part)"
            if (-not $lanes.ContainsKey($key)) { $lanes[$key] = New-Object System.Collections.ArrayList }
            [void]$lanes[$key].Add($row)
        }
        # An UNUSED lane entry changes nothing about the expansion, so a table
        # carrying one means what the table without it means.
        foreach ($declared in @($laneFirst.Keys)) {
            if (-not $lanes.ContainsKey($declared)) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_lane_unused' }
            }
        }
        foreach ($key in @($lanes.Keys)) {
            $laneRows = @($lanes[$key] | Sort-Object -Property target_hour_raw)
            $distinct = @($laneRows | Select-Object -ExpandProperty target_hour_raw -Unique)
            if ($distinct.Count -ne $laneRows.Count) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_lane_hour_ambiguous' }
            }
            if (-not $laneFirst.ContainsKey($key)) {
                return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_lane_first_missing' }
            }
            for ($i = 0; $i -lt $laneRows.Count; $i++) {
                $derived = [bigint]$laneFirst[$key] + [bigint]$i
                # The ordinal is a `u64`, and the recorder's own reader adds with
                # `checked_add`. An unbounded sum would publish an ordinal no
                # producer could have written.
                if ($derived -gt [bigint]::Parse('18446744073709551615')) {
                    return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_ordinal_overflow' }
                }
                $laneRows[$i].outcome_ordinal = $derived
            }
        }
    }
    # REQUIRED and checked. The count is the only thing that can catch a
    # truncated class list, and a check that skips itself when the field is
    # absent or a string is the check not running.
    $declared = $null
    if ($null -ne $table.PSObject.Properties['row_count']) {
        $declared = $table.PSObject.Properties['row_count'].Value
    }
    if (-not (Test-GapLedgerIntegerToken $declared)) {
        return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_row_count_invalid' }
    }
    if ([int64]$declared -ne $rows.Count) {
        return @{ Ok = $false; Rows = @(); Problem = 'gap_ledger_row_count_mismatch' }
    }
    $ordered = @($rows | Sort-Object -Property target_hour_raw, domain_id)
    return @{ Ok = $true; Rows = $ordered; Problem = $null }
}

function Test-GapLedgerIntegerToken {
    <#
    .SYNOPSIS
    Whether a decoded JSON value was written as an INTEGER TOKEN.

    .DESCRIPTION
    Windows PowerShell 5.1 parses `2` as `Int32`/`Int64` and `2.0` as `Decimal`,
    and it uses `Decimal` again for a plain integer too large for `Int64`. So a
    `Decimal` is a legitimate integer token ONLY when its value exceeds the
    `Int64` range -- below that, a `Decimal` means the document wrote a float,
    which the recorder never does and which Rust and Python refuse.

    A `Double`/`Single` is refused outright: it was rounded at parse time, so the
    digits are gone before this can look at them.
    #>
    param($Value)
    if ($null -eq $Value -or $Value -is [bool] -or $Value -is [string] -or
        $Value -is [double] -or $Value -is [single] -or $Value -is [System.Array]) {
        return $false
    }
    if ($Value -is [decimal]) {
        $decimal = [decimal]$Value
        if ([math]::Truncate($decimal) -ne $decimal) { return $false }
        # `.NET Decimal` PRESERVES SCALE, so `9223372036854775808.0` still
        # stringifies with its point even though both spellings parse to Decimal.
        # That is the only surviving way to tell a wide integer token from a wide
        # float, and Rust and Python refuse the float.
        if ([string]$decimal -like '*.*') { return $false }
        return ($decimal -gt [decimal][int64]::MaxValue -or
                $decimal -lt [decimal][int64]::MinValue)
    }
    return ($Value -is [int] -or $Value -is [long] -or $Value -is [int64] -or
            $Value -is [int16] -or $Value -is [sbyte] -or $Value -is [uint16] -or
            $Value -is [uint32] -or $Value -is [uint64] -or $Value -is [byte])
}

function Get-GapLedgerInteger {
    <#
    .SYNOPSIS
    A gap-ledger field that MUST be present and MUST be an integer.

    .DESCRIPTION
    `Get-ObjectProperty` returns $null for an absent field and PowerShell reads
    `[int64]$null` as 0 -- so a class missing `range_start_rel` expanded quietly
    into rows whose range started at the target hour. That is worse than a crash:
    it is the ledger attributing a gap to hours nobody recorded, which is the one
    outcome it exists to prevent. Throws; `Expand-CaptureGapOutcomes` bounds the
    throw and reports it.
    #>
    param($Object, [Parameter(Mandatory = $true)][string]$Name)
    $value = $null
    $gapLedgerProperty = if ($null -ne $Object) { $Object.PSObject.Properties[$Name] } else { $null }
    if ($null -ne $gapLedgerProperty) { $value = $gapLedgerProperty.Value }
    if (-not (Test-GapLedgerIntegerToken $value) -or
        [bigint]$value -gt [bigint][int64]::MaxValue -or
        [bigint]$value -lt [bigint][int64]::MinValue) {
        throw "gap ledger class field '$Name' is missing or not an integer"
    }
    return $value
}

function Assert-GapLedgerU32 {
    <#
    .SYNOPSIS
    A queue witness, refused when it falls outside the `u32` domain.

    .DESCRIPTION
    The contract's fields are unsigned. A negative queue depth is a figure the
    producer could not have written and its own decoder refuses.
    #>
    param([Parameter(Mandatory = $true)][int64]$Value)
    if ($Value -lt 0 -or $Value -gt 4294967295) {
        throw "gap ledger queue witness $Value is outside the u32 range"
    }
    return $Value
}

function Assert-GapLedgerHour {
    <#
    .SYNOPSIS
    A resolved game hour, refused when it falls outside the `i32` domain.

    .DESCRIPTION
    A run may sit at the top of the hour space while a class states a positive
    range offset, and `target + rel` then names an hour that cannot exist. Rust
    and Python refuse it; admitting it here would be two readers of one document
    disagreeing about which hour a gap belongs to.
    #>
    param([Parameter(Mandatory = $true)][int64]$Hour)
    if ($Hour -lt -2147483648 -or $Hour -gt 2147483647) {
        throw "gap ledger resolves an hour to $Hour, outside the i32 hour range"
    }
    return $Hour
}

function Get-GapLedgerWideInteger {
    <#
    .SYNOPSIS
    A gap-ledger field that MUST be present and integral, WITHOUT an Int64 bound.

    .DESCRIPTION
    `Test-IsJsonInteger` caps at Int64 on purpose: every one of its callers casts
    through `[int64]`, and a wider value would throw at the cast. `queue_dropped`
    is a `u64` on the wire and is carried here as a `[bigint]`, so that cap would
    reject a legal ledger. Integrality is still required, and a `[double]` is
    still refused -- it was rounded at parse time, so the digits are already gone.
    #>
    param($Object, [Parameter(Mandatory = $true)][string]$Name)
    $value = $null
    $gapLedgerProperty = if ($null -ne $Object) { $Object.PSObject.Properties[$Name] } else { $null }
    if ($null -ne $gapLedgerProperty) { $value = $gapLedgerProperty.Value }
    if (-not (Test-GapLedgerIntegerToken $value)) {
        throw "gap ledger class field '$Name' is missing or not an integer"
    }
    # UNSIGNED and BOUNDED. The contract's field is a `u64`: a reader that admits
    # a negative, or one above the type, hands its caller a figure the producer
    # could not have written and its own decoder refuses.
    if ([bigint]$value -lt [bigint]0 -or
        [bigint]$value -gt [bigint]::Parse('18446744073709551615')) {
        throw "gap ledger class field '$Name' is outside the u64 range"
    }
    return $value
}

function Get-GapLedgerSignedInteger {
    <#
    .SYNOPSIS
    A gap-ledger OFFSET: present, integral, and allowed to be negative.

    .DESCRIPTION
    `Get-GapLedgerWideInteger` refuses a negative, which is right for a tick and
    wrong for the offset that produces one: the recorder writes `{"rel": -1}`
    wherever a torn sweep's tick precedes its hour. Integrality is still
    required, and a `[double]` is still refused -- it was rounded at parse time.
    #>
    param($Object, [Parameter(Mandatory = $true)][string]$Name)
    $value = $null
    $gapLedgerProperty = if ($null -ne $Object) { $Object.PSObject.Properties[$Name] } else { $null }
    if ($null -ne $gapLedgerProperty) { $value = $gapLedgerProperty.Value }
    if (-not (Test-GapLedgerIntegerToken $value)) {
        throw "gap ledger class field '$Name' is missing or not an integer"
    }
    return $value
}

function Get-GapLedgerName {
    <#
    .SYNOPSIS
    A gap-ledger field that MUST be a non-empty string.
    #>
    param($Object, [Parameter(Mandatory = $true)][string]$Name)
    $value = $null
    $gapLedgerProperty = if ($null -ne $Object) { $Object.PSObject.Properties[$Name] } else { $null }
    if ($null -ne $gapLedgerProperty) { $value = $gapLedgerProperty.Value }
    if ($value -isnot [string] -or [string]::IsNullOrEmpty($value)) {
        throw "gap ledger class field '$Name' is missing or not a name"
    }
    return $value
}

function Resolve-GapLedgerAnchor {
    <#
    .SYNOPSIS
    A tick stated either relative to its hour or absolutely; $null when neither
    or both, which has no single meaning.
    #>
    param($Field, [int64]$Anchor)
    # An ARRAY is not the object PowerShell would unwrap it into.
    if ($null -eq $Field -or $Field -is [System.Array]) { return $null }
    # CLOSED, as Rust and Python close it. Without this an extra nested field
    # reached the class's identity and its aggregation key -- two classes at one
    # hour differing only by a field nothing reads passed the same-hour rule.
    foreach ($member in @($Field.PSObject.Properties)) {
        if ($member.Name -cne 'rel' -and $member.Name -cne 'abs') { return $null }
    }
    $hasRel = $null -ne $Field.PSObject.Properties['rel']
    $hasAbs = $null -ne $Field.PSObject.Properties['abs']
    if ($hasRel -eq $hasAbs) { return $null }
    # `[bigint]`, not `[int64]`: a tick is a `u64` on the wire and `u64::MAX`
    # overflows `Int64`, which in PowerShell is a TERMINATING cast error -- the
    # validator would die on a legal ledger instead of grading it.
    #
    # And TYPE-CHECKED before the cast, because `[bigint]1.5` is 2: a fractional
    # offset resolved silently to a different tick than the one written, while
    # Rust and Python refuse it.
    #
    # `abs` is UNSIGNED (a `u64` tick) and `rel` is SIGNED (an `i64` offset) --
    # conflating them refused `{"rel": -1}`, which the recorder writes for every
    # torn sweep whose tick precedes its hour. The RESOLVED value is the tick and
    # is bounded to `u64` in either spelling.
    if ($hasAbs) {
        $abs = [bigint](Get-GapLedgerWideInteger $Field 'abs')
        if ($abs -gt [bigint]::Parse('18446744073709551615')) {
            throw 'gap ledger tick is outside the u64 range'
        }
        return $abs
    }
    $rel = [bigint](Get-GapLedgerSignedInteger $Field 'rel')
    if ($rel -lt [bigint]::Parse('-9223372036854775808') -or
        $rel -gt [bigint]::Parse('9223372036854775807')) {
        throw 'gap ledger tick offset is outside the i64 range'
    }
    $resolved = [bigint]$Anchor + $rel
    if ($resolved -lt [bigint]0 -or $resolved -gt [bigint]::Parse('18446744073709551615')) {
        throw 'gap ledger tick resolves outside the u64 range'
    }
    return $resolved
}

function Get-RegistryCaptureDomains {
    param([Parameter(Mandatory = $true)][string]$RegistryPath)
    if (-not (Test-Path -LiteralPath $RegistryPath -PathType Leaf)) {
        throw "domain registry not found: $RegistryPath"
    }
    $registry = Get-Content -LiteralPath $RegistryPath -Raw | ConvertFrom-Json
    if ([string](Get-ObjectProperty $registry "schema") -ne "battle-plan.compact_domain_registry.v1") {
        throw "unsupported domain registry schema in $RegistryPath"
    }
    # `capture_scheduled: false` marks a domain the owner ruled out of capture
    # scope; the key is absent for every domain that is still scheduled. The
    # recorder does not schedule these, so demanding a data record for them
    # would fail every capture. Owned by
    # compact_contract::DESCHEDULED_CAPTURE_DOMAIN_IDS.
    $domains = @(
        $registry.domains |
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
                $script:ProductionDomainStatuses -contains ([string]$_.status)
            } |
            Sort-Object { [int]$_.id }
    )
    if ($domains.Count -eq 0) { throw "domain registry contains no production-profile domains" }
    $duplicateIds = @($domains | Group-Object id | Where-Object Count -ne 1)
    if ($duplicateIds.Count -ne 0) { throw "domain registry has duplicate schedulable ids" }
    return $domains
}

function Add-CaptureProblem {
    param([System.Collections.Generic.List[string]]$Problems, [string]$Problem)
    if (-not $Problems.Contains($Problem)) { $Problems.Add($Problem) }
}

function Test-HexSha256 {
    param($Value)
    return $null -ne $Value -and ([string]$Value) -match '^[0-9a-fA-F]{64}$'
}

function ConvertTo-NativeArgumentLine {
    param([Parameter(Mandatory = $true)][object[]]$Arguments)
    return (($Arguments | ForEach-Object {
        $value = [string]$_
        # Kept byte-identical to the harness's copy in
        # Invoke-HeadlessRawCapture.ps1. An unquoted empty string emits
        # NOTHING, dropping the argument and shifting every later one a
        # position left.
        if ($value.Length -eq 0 -or $value -match '[\s"]') { '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"' }
        else { $value }
    }) -join ' ')
}

function Invoke-IndependentCaptureGate {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][object[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$LogDir,
        [int]$TimeoutSeconds = 900,
        [int]$CancellationGraceSeconds = 5
    )
    New-Item -ItemType Directory -Path $LogDir -Force | Out-Null
    $stdout = Join-Path $LogDir "$Name.stdout.log"
    $stderr = Join-Path $LogDir "$Name.stderr.log"
    if (-not (Test-Path -LiteralPath $FilePath -PathType Leaf)) {
        return [pscustomobject]@{ name=$Name; passed=$false; started=$false; exit_code=$null; timed_out=$false; problem="tool_missing"; stdout=$stdout; stderr=$stderr; executable_sha256=$null; arguments=@($Arguments) }
    }
    $executableSha256 = (Get-FileHash -LiteralPath $FilePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $process = $null
    $timedOut = $false
    try {
        $start=[System.Diagnostics.ProcessStartInfo]::new()
        $start.FileName=$FilePath; $start.Arguments=ConvertTo-NativeArgumentLine $Arguments
        $start.UseShellExecute=$false; $start.CreateNoWindow=$true
        $start.RedirectStandardOutput=$true; $start.RedirectStandardError=$true
        $process=[System.Diagnostics.Process]::new(); $process.StartInfo=$start
        if (-not $process.Start()) { throw "process start returned false" }
        $stdoutTask=$process.StandardOutput.ReadToEndAsync(); $stderrTask=$process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit([Math]::Max(1, $TimeoutSeconds) * 1000)) {
            $timedOut = $true
            try { [void]$process.CloseMainWindow() } catch {}
            if (-not $process.WaitForExit([Math]::Max(1, $CancellationGraceSeconds) * 1000)) {
                try { $process.Kill() } catch {}
                [void]$process.WaitForExit(5000)
            }
        }
        if ($process.HasExited) {
            Write-AtomicUtf8Text -Path $stdout -Text ([string]$stdoutTask.Result)
            Write-AtomicUtf8Text -Path $stderr -Text ([string]$stderrTask.Result)
        }
        $process.Refresh()
        $exitCode = if ($process.HasExited) { $process.ExitCode } else { $null }
        return [pscustomobject]@{ name=$Name; passed=(-not $timedOut -and $exitCode -eq 0); started=$true; exit_code=$exitCode; timed_out=$timedOut; problem=$(if ($timedOut) { "timeout" } elseif ($exitCode -ne 0) { "nonzero_exit" } else { $null }); stdout=$stdout; stderr=$stderr; executable_sha256=$executableSha256; arguments=@($Arguments) }
    }
    catch {
        return [pscustomobject]@{ name=$Name; passed=$false; started=$false; exit_code=$null; timed_out=$timedOut; problem=$_.Exception.Message; stdout=$stdout; stderr=$stderr; executable_sha256=$executableSha256; arguments=@($Arguments) }
    }
    finally { if ($null -ne $process) { $process.Dispose() } }
}

function Get-ProductionFoldCheckArguments {
    param(
        [Parameter(Mandatory = $true)]$Manifest,
        [Parameter(Mandatory = $true)][string]$RawPath,
        [Parameter(Mandatory = $true)][string]$ReportPath
    )
    # The recorder publishes the mode bound from authenticated READY before H4RA.
    # fold_check independently checks the selected grammar/CID1 in the raw bytes;
    # its legacy CLI default is not authority for a missing manifest field.
    $property = $Manifest.PSObject.Properties['compact_stream_identity_mode']
    if ($null -eq $property) { throw 'compact_stream_identity_mode is missing' }
    # Read directly: a function return would unwrap a one-element JSON array.
    $declared = $property.Value
    if ($declared -isnot [string]) { throw 'compact_stream_identity_mode must be an explicit string' }
    $mode = switch -CaseSensitive ($declared) {
        'identity_v1' { 'identity-v1' }
        'legacy' { 'legacy' }
        default { throw 'compact_stream_identity_mode must be identity_v1 or legacy' }
    }
    return @('hash', $RawPath, '--expected-profile', 'production', '--identity-mode', $mode, '--out', $ReportPath)
}

function Test-ProductionCaptureArtifacts {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RunDir,
        [Parameter(Mandatory = $true)][string]$RegistryPath,
        [Parameter(Mandatory = $true)][string]$SpeedProofPath,
        [hashtable]$StaticArtifactPaths = @{},
        [string]$ExpectedStagedDll = "",
        [string]$CompactGateExe = "",
        [string]$FoldCheckExe = "",
        [string]$ContainerVerifyExe = "",
        [string]$PythonExe = "",
        [string]$StaticBundleScript = "",
        [string]$StaticBundleDir = "",
        [string]$GameExePath = "",
        [string]$ModStatePath = "",
        [string]$DlcStatePath = "",
        [string]$LoadOrderPath = "",
        [string]$SourceInventoryPath = "",
        [string]$LiveResolutionReportPath = "",
        [string]$CapturePlanPath = "",
        [string]$SpeedRootGatePath = "",
        [string]$CollectorControlPath = "",
        [string]$SpeedMonitorPath = "",
        [string]$SpeedStreamVerifier = "",
        [string]$SpeedEvidenceTranscriptPath = "",
        [string]$SpeedEvidenceNonceLedgerRoot = "",
        [string]$SpeedEvidenceVerifier = "",
        [string]$ExpectedSpeedEvidenceVerifierSha256 = "",
        [string]$ExpectedSpeedEvidenceProbeSha256 = "",
        [string]$ExpectedSpeedStaticDisasmSha256 = "",
        [int]$ToolTimeoutSeconds = 900,
        [int]$CancellationGraceSeconds = 5
    )

    $problems = [System.Collections.Generic.List[string]]::new()
    $manifestPath = Join-Path $RunDir "native_session_manifest.json"
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        Add-CaptureProblem $problems "manifest_missing"
        return [pscustomobject]@{ ok = $false; problems = @($problems); manifest_path = $manifestPath }
    }
    try { $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json }
    catch {
        Add-CaptureProblem $problems "manifest_not_json"
        return [pscustomobject]@{ ok = $false; problems = @($problems); manifest_path = $manifestPath }
    }

    $expectedDomains = @(Get-RegistryCaptureDomains -RegistryPath $RegistryPath)
    $expectedIds = @($expectedDomains | ForEach-Object { [int]$_.id })
    $expectedNames = @($expectedDomains | ForEach-Object { [string]$_.name })
    $expectedRoster = @(
        for ($domainIndex = 0; $domainIndex -lt $expectedDomains.Count; $domainIndex++) {
            [pscustomobject]@{
                id = [int]$expectedIds[$domainIndex]
                name = [string]$expectedNames[$domainIndex]
            }
        }
    )
    $expectedDataIds = @($expectedDomains | Where-Object { $_.requires_data_record -eq $true } | ForEach-Object { [int]$_.id })

    if ([string](Get-ObjectProperty $manifest "stream_format") -ne "compact") { Add-CaptureProblem $problems "stream_format_not_compact" }
    $profileEnabled = Get-ObjectProperty (Get-ObjectProperty $manifest "production_profile") "enabled"
    if ($profileEnabled -isnot [bool]) { Add-CaptureProblem $problems "production_profile_enabled_not_published" }
    elseif (-not $profileEnabled) { Add-CaptureProblem $problems "production_profile_not_enabled" }
    if (@(Get-ObjectProperty (Get-ObjectProperty $manifest "production_profile") "mandatory_domains_not_production_complete").Count -ne 0) { Add-CaptureProblem $problems "mandatory_domains_not_production_complete" }
    if (@(Get-ObjectProperty (Get-ObjectProperty $manifest "production_profile") "profile_domains_not_scheduled").Count -ne 0) { Add-CaptureProblem $problems "profile_domains_not_scheduled" }
    if (@(Get-ObjectProperty (Get-ObjectProperty $manifest "production_profile") "missing_required_static_artifacts").Count -ne 0) { Add-CaptureProblem $problems "required_static_artifacts_missing_from_profile" }

    $compact = Get-ObjectProperty $manifest "compact_stream"
    if ($null -eq $compact) { Add-CaptureProblem $problems "compact_stream_summary_missing" }
    $frameCountRaw = Get-ObjectProperty $compact "frame_end_record_count"
    if (-not (Test-IsJsonInteger $frameCountRaw)) { Add-CaptureProblem $problems "frame_end_record_count_not_published" }
    $frameCount = if (Test-IsJsonInteger $frameCountRaw) { [int64]$frameCountRaw } else { 0 }
    if ($frameCount -le 0) { Add-CaptureProblem $problems "no_frame_end_records" }
    $frameTiming = Get-ObjectProperty $compact "frame_time_us"
    if ($null -eq $frameTiming) { Add-CaptureProblem $problems "frame_time_us_missing" }
    else {
        # STRICT count type (re-gate2 finding 3): the old bare [int64] coercion
        # silently accepted a numeric STRING ("10" -ne 10 is false after coercion)
        # and THREW mid-gate on a bare JSON NaN. Require a genuine positive finite
        # JSON integer BEFORE any comparison; wrong type = its own typed problem.
        $frameTimingCount = Get-ObjectProperty $frameTiming "count"
        if (-not (Test-IsJsonInteger $frameTimingCount) -or [int64]$frameTimingCount -le 0) {
            Add-CaptureProblem $problems "frame_time_count_not_positive_integer"
        } elseif ([int64]$frameTimingCount -ne $frameCount) {
            Add-CaptureProblem $problems "frame_time_sample_count_mismatch"
        }
        if ($null -eq (Get-ObjectProperty $frameTiming "max") -or [double](Get-ObjectProperty $frameTiming "max") -ge 100000.0) { Add-CaptureProblem $problems "frame_time_over_100ms" }
    }
    # Acceptance-telemetry schema gate (F5): fail-closed on any absent frame_time
    # percentile (incl. the new p50/p99) or any missing acceptance_telemetry
    # sink_write_us / clock_skew field. Keeps this validator in lockstep with the
    # recorder's acceptance_telemetry manifest block and timing_summary_json shape.
    foreach ($p in (Get-AcceptanceTelemetryProblems -Manifest $manifest)) { Add-CaptureProblem $problems ("acceptance_telemetry_$p") }
    $decodable = Get-ObjectProperty $compact "decodable"
    if ($decodable -isnot [bool]) { Add-CaptureProblem $problems "compact_stream_decodable_not_published" }
    elseif (-not $decodable) { Add-CaptureProblem $problems "compact_stream_not_decodable" }
    if (@(Get-ObjectProperty $manifest "parse_errors").Count -ne 0 -or @(Get-ObjectProperty $compact "parse_errors").Count -ne 0) { Add-CaptureProblem $problems "parse_errors_present" }
    if (@(Get-ObjectProperty $manifest "ordering_errors").Count -ne 0 -or @(Get-ObjectProperty $compact "ordering_errors").Count -ne 0) { Add-CaptureProblem $problems "ordering_errors_present" }
    # `[int64]$null` is 0, so each of these clauses used to PASS when the field
    # was absent -- "we skipped no hours" asserted by a manifest that never said.
    # Absence is now its own typed problem, exactly as the frame_time count
    # above already treats it.
    $skippedGameHours = Get-ObjectProperty $manifest "skipped_game_hours"
    if (-not (Test-IsJsonInteger $skippedGameHours)) { Add-CaptureProblem $problems "skipped_game_hours_not_published" }
    elseif ([int64]$skippedGameHours -ne 0) { Add-CaptureProblem $problems "skipped_game_hours" }
    $uniqueGameHourCount = Get-ObjectProperty $manifest "unique_game_hour_count"
    if (-not (Test-IsJsonInteger $uniqueGameHourCount)) { Add-CaptureProblem $problems "unique_game_hour_count_not_published" }
    elseif ([int64]$uniqueGameHourCount -ne $frameCount) { Add-CaptureProblem $problems "frame_hour_count_mismatch" }
    $hourly = Get-ObjectProperty $manifest "hourly_requirement"
    $hourlyMet = Get-ObjectProperty $hourly "met"
    if ($null -eq $hourly -or $hourlyMet -isnot [bool]) { Add-CaptureProblem $problems "hourly_requirement_met_not_published" }
    elseif (-not $hourlyMet) { Add-CaptureProblem $problems "recorder_hourly_requirement_not_met" }
    $requestedGameHoursRaw = Get-ObjectProperty $hourly "requested_game_hours"
    if (-not (Test-IsJsonInteger $requestedGameHoursRaw)) { Add-CaptureProblem $problems "requested_game_hours_not_published" }
    else {
        $requestedGameHours = [int64]$requestedGameHoursRaw
        if ($requestedGameHours -le 0) { Add-CaptureProblem $problems "requested_game_hours_not_positive" }
    }

    # An absent status word masked clean, so a manifest that published no
    # failure_status_flags_or at all read as a capture with no failures.
    $failureFlagsRaw = Get-ObjectProperty $compact "failure_status_flags_or"
    if (-not (Test-IsJsonInteger $failureFlagsRaw)) {
        Add-CaptureProblem $problems "failure_status_flags_or_not_published"
        $failureFlags = [uint32]0
    }
    else {
        $failureFlags = [uint32][int64]$failureFlagsRaw
    }
    if (($failureFlags -band $script:CanonicalCompactFailureMask) -ne 0) { Add-CaptureProblem $problems ("canonical_failure_status_seen_0x{0:X}" -f ($failureFlags -band $script:CanonicalCompactFailureMask)) }
    # `-eq $true` on an absent property is false, so a manifest that never
    # published failure_status_seen asserted "no failure was seen" by saying
    # nothing. The flag must be a published boolean.
    $failureStatusSeen = Get-ObjectProperty $compact "failure_status_seen"
    if ($failureStatusSeen -isnot [bool]) { Add-CaptureProblem $problems "failure_status_seen_not_published" }
    elseif ($failureStatusSeen) { Add-CaptureProblem $problems "recorder_failure_status_seen" }
    $coverage = Get-ObjectProperty $compact "domain_coverage"
    $coverageOk = Get-ObjectProperty $coverage "ok"
    if ($null -eq $coverage -or $coverageOk -isnot [bool]) { Add-CaptureProblem $problems "domain_coverage_ok_not_published" }
    elseif (-not $coverageOk) { Add-CaptureProblem $problems "domain_coverage_not_ok" }
    if (@(Get-ObjectProperty $coverage "errors").Count -ne 0) { Add-CaptureProblem $problems "domain_coverage_errors" }

    $seenById = @{}
    $perDomain = Get-ObjectProperty $compact "per_domain"
    if ($null -ne $perDomain) {
        foreach ($property in $perDomain.PSObject.Properties) {
            $row = $property.Value
            # `[int]"47"` is 47, so a row publishing its domain_id as a STRING
            # was entered under the integer key and satisfied the expected
            # roster. The promotion gate rejects it independently, but this gate
            # was returning ok = true for it.
            $idRaw = Get-ObjectProperty $row "domain_id"
            if (-not (Test-IsJsonInteger $idRaw)) {
                Add-CaptureProblem $problems "per_domain_domain_id_not_an_integer"
                continue
            }
            $id = [int]$idRaw
            if ($seenById.ContainsKey($id)) { Add-CaptureProblem $problems "duplicate_domain_id_$id" }
            else { $seenById[$id] = $row }
        }
    }
    $gapExpansion = Expand-CaptureGapOutcomes $compact
    if (-not $gapExpansion.Ok) {
        # A ledger this validator cannot read is NOT a capture with no gaps.
        # Recorded as its own problem so the walk below cannot pass vacuously on
        # a manifest nobody managed to open.
        Add-CaptureProblem $problems $gapExpansion.Problem
    }
    $gapRows = @($gapExpansion.Rows)
    $declaredGapCount = Get-ObjectProperty $compact "domain_gap_record_count"
    if (-not (Test-IsJsonInteger $declaredGapCount) -or [int64]$declaredGapCount -lt 0) {
        Add-CaptureProblem $problems "domain_gap_record_count_invalid"
    } elseif ([int64]$declaredGapCount -ne $gapRows.Count) {
        Add-CaptureProblem $problems "domain_gap_record_count_mismatch"
    }
    if ($gapRows.Count -ne 0) {
        Add-CaptureProblem $problems "production_zero_loss_domain_gaps_present"
    }
    $gapCountById = @{}
    $gapOrdinalByPart = @{}
    $gapRangeEndByPart = @{}
    $gapTargetByDomain = @{}
    foreach ($gap in $gapRows) {
        if ($null -eq $gap) { continue }
        $id = [int](Get-ObjectProperty $gap "domain_id")
        $part = [string](Get-ObjectProperty $gap "part")
        $reason = [string](Get-ObjectProperty $gap "reason")
        $start = Get-ObjectProperty $gap "range_start_hour_raw"
        $end = Get-ObjectProperty $gap "range_end_hour_raw"
        $target = Get-ObjectProperty $gap "target_hour_raw"
        $ordinal = Get-ObjectProperty $gap "outcome_ordinal"
        if ($expectedIds -notcontains $id) { Add-CaptureProblem $problems "domain_gap_unknown_domain_$id"; continue }
        if (@("state","event","both") -notcontains $part) { Add-CaptureProblem $problems "domain_${id}_gap_part_invalid" }
        if (@("torn","overrun","backpressure","clock_unreadable","worker_panic") -notcontains $reason) { Add-CaptureProblem $problems "domain_${id}_gap_reason_invalid" }
        if (-not (Test-IsJsonInteger $start) -or -not (Test-IsJsonInteger $end) -or [int64]$start -gt [int64]$end) {
            Add-CaptureProblem $problems "domain_${id}_gap_range_invalid"
        } else {
            $rangeKey = "${id}:$part"
            if ($gapRangeEndByPart.ContainsKey($rangeKey) -and
                [int64]$start -le [int64]$gapRangeEndByPart[$rangeKey]) {
                Add-CaptureProblem $problems "domain_${id}_${part}_gap_range_overlap_or_regression"
            }
            $gapRangeEndByPart[$rangeKey] = [int64]$end
            if (-not (Test-IsJsonInteger $target) -or
                [int64]$target -lt [int64]$start -or [int64]$target -gt [int64]$end) {
                Add-CaptureProblem $problems "domain_${id}_gap_target_outside_range"
            } else {
                $targetKey = "${id}:$([int64]$target)"
                if ($gapTargetByDomain.ContainsKey($targetKey)) {
                    Add-CaptureProblem $problems "domain_${id}_duplicate_gap_terminal_for_frame"
                }
                $gapTargetByDomain[$targetKey] = $true
            }
        }
        if (-not (Test-IsJsonInteger $ordinal) -or [int64]$ordinal -lt 0) {
            Add-CaptureProblem $problems "domain_${id}_gap_ordinal_invalid"
        } else {
            $ordinalKey = "${id}:$part"
            $expectedOrdinal = if ($gapOrdinalByPart.ContainsKey($ordinalKey)) { [int64]$gapOrdinalByPart[$ordinalKey] } else { 0 }
            if ([int64]$ordinal -ne $expectedOrdinal) { Add-CaptureProblem $problems "domain_${id}_${part}_gap_ordinal_discontinuous" }
            $gapOrdinalByPart[$ordinalKey] = $expectedOrdinal + 1
        }
        if ($gapCountById.ContainsKey($id)) { $gapCountById[$id] = [int64]$gapCountById[$id] + 1 }
        else { $gapCountById[$id] = [int64]1 }
    }
    $actualIds = @(@($seenById.Keys) + @($gapCountById.Keys) | Sort-Object -Unique)
    if (($actualIds -join ',') -ne (($expectedIds | Sort-Object) -join ',')) { Add-CaptureProblem $problems "registry_roster_mismatch" }
    foreach ($id in $expectedIds) {
        $gapCount = if ($gapCountById.ContainsKey($id)) { [int64]$gapCountById[$id] } else { 0 }
        if (-not $seenById.ContainsKey($id)) {
            if ($gapCount -ne $frameCount) { Add-CaptureProblem $problems "domain_${id}_missing" }
            continue
        }
        $row = $seenById[$id]
        $acceptedRaw = Get-ObjectProperty $row "record_count"
        if (-not (Test-IsJsonInteger $acceptedRaw) -or [int64]$acceptedRaw -lt 0) {
            Add-CaptureProblem $problems "domain_${id}_record_count_invalid"
            continue
        }
        $acceptedCount = [int64]$acceptedRaw
        if ($acceptedCount + $gapCount -ne $frameCount) { Add-CaptureProblem $problems "domain_${id}_terminal_outcome_count_mismatch" }
        $dataCountRaw = Get-ObjectProperty $row "data_record_count"
        if ($expectedDataIds -contains $id -and
            (-not (Test-IsJsonInteger $dataCountRaw) -or [int64]$dataCountRaw -ne $acceptedCount)) {
            Add-CaptureProblem $problems "domain_${id}_data_record_count_mismatch"
        }
        # The effective word below already fails closed on absence. The RAW word
        # did not: `[uint32]$null` is 0 and 0 masks clean, so a row that
        # published no status_flags_or at all read as a clean domain. Same
        # treatment for both, so the pair cannot disagree about what absence
        # means.
        $rawFlagsRaw = Get-ObjectProperty $row "status_flags_or"
        if (-not (Test-IsJsonInteger $rawFlagsRaw)) {
            Add-CaptureProblem $problems "domain_${id}_status_flags_or_missing"
        }
        $rawFlags = if (Test-IsJsonInteger $rawFlagsRaw) { [uint32][int64]$rawFlagsRaw } else { [uint32]0 }
        $effectiveFlags = Get-ObjectProperty $row "effective_failure_status_flags_or"
        # D42'S DECLARATION IS THE PAYLOAD SPELLING: the boundary decoder
        # counts an undeclared partial (failing word tolerable BY FLAGS while
        # the region section is still present) as
        # `d42_undeclared_expected_partial`. With that counter PRESENT and
        # zero, every d42 PARTIAL_DATA hour omitted its region section -- the
        # declared gap the compact contract tolerates -- so the raw-word
        # clause licenses exactly that bit for domain 42. A missing or
        # malformed counter never licenses. Mirrors Promotion.ps1.
        $rawFailureMask = $script:CanonicalCompactFailureMask
        if ($id -eq 42) {
            $d42Problems = Get-ObjectProperty $row "problems"
            $d42Undeclared = $null
            if ($null -ne $d42Problems) {
                $d42Undeclared = Get-ObjectProperty $d42Problems "d42_undeclared_expected_partial"
            }
            if ((Test-IsJsonInteger $d42Undeclared) -and [int64]$d42Undeclared -eq 0) {
                $rawFailureMask = $script:CanonicalCompactFailureMaskNoPartialData
            }
        }
        if (($rawFlags -band $rawFailureMask) -ne 0) {
            # A raw source-unavailable, hard-disabled, partial, bridge, or decode
            # status is never production-complete. Effective-status masking is
            # useful for replay semantics, but it cannot promote missing source
            # data into a successful real capture.
            Add-CaptureProblem $problems "domain_${id}_raw_failure_status"
        }
        if (-not (Test-IsJsonInteger $effectiveFlags)) {
            # Fail-closed: an absent, null, or wrongly-typed per-domain
            # effective-failure field means we cannot evaluate whether this
            # domain masked a real failure, so it is a validation FAILURE, not a
            # silent pass. The check used to be `$null -eq`, which let the JSON
            # string "0" and the boolean $false through -- both cast to [uint32]
            # 0 and both then masked clean.
            Add-CaptureProblem $problems "domain_${id}_effective_failure_status_flags_or_missing"
        }
        elseif (([uint32][int64]$effectiveFlags -band $script:CanonicalCompactFailureMask) -ne 0) {
            Add-CaptureProblem $problems "domain_${id}_effective_failure_status"
        }
    }

    if ($null -ne (Get-ObjectProperty $compact "hour_latch")) {
        Add-CaptureProblem $problems "forbidden_hour_latch_summary_present"
    }

    $finalize = Get-ObjectProperty $manifest "host_finalize"
    $fold = Get-ObjectProperty $finalize "fold"
    # `-eq $true` on an absent property is false, so a fold receipt that never
    # published `skipped` asserted "I did not skip" by saying nothing.
    $foldSkipped = Get-ObjectProperty $fold "skipped"
    if ($null -eq $fold -or $foldSkipped -isnot [bool]) { Add-CaptureProblem $problems "host_fold_missing_or_skipped" }
    elseif ($foldSkipped) { Add-CaptureProblem $problems "host_fold_missing_or_skipped" }
    else {
        # Both counts must be zero, and both used to accept absence as zero.
        $truncatedTailBytes = Get-ObjectProperty $fold "truncated_tail_bytes"
        if (-not (Test-IsJsonInteger $truncatedTailBytes)) { Add-CaptureProblem $problems "fold_truncated_tail_bytes_not_published" }
        elseif ([int64]$truncatedTailBytes -ne 0) { Add-CaptureProblem $problems "fold_truncated_tail" }
        if ($null -ne (Get-ObjectProperty $fold "stop_reason")) { Add-CaptureProblem $problems "fold_stop_reason_present" }
        $endsAtFrameEnd = Get-ObjectProperty $fold "ends_at_frame_end"
        if ($endsAtFrameEnd -isnot [bool]) { Add-CaptureProblem $problems "fold_ends_at_frame_end_not_published" }
        elseif (-not $endsAtFrameEnd) { Add-CaptureProblem $problems "fold_not_closed_at_frame_end" }
        $malformedPayloadRecords = Get-ObjectProperty $fold "malformed_payload_records"
        if (-not (Test-IsJsonInteger $malformedPayloadRecords)) { Add-CaptureProblem $problems "fold_malformed_payload_records_not_published" }
        elseif ([int64]$malformedPayloadRecords -ne 0) { Add-CaptureProblem $problems "fold_malformed_payload_records" }
    }
    $verify = Get-ObjectProperty $finalize "verify"
    if ($null -eq $verify) { Add-CaptureProblem $problems "container_verify_missing" }
    else {
        $streamShaMatches = Get-ObjectProperty $verify "stream_sha256_matches_capture"
        if ($streamShaMatches -isnot [bool]) { Add-CaptureProblem $problems "container_stream_sha_match_not_published" }
        elseif (-not $streamShaMatches) { Add-CaptureProblem $problems "container_stream_sha_mismatch" }
        $entriesVerified = Get-ObjectProperty $verify "entries_verified"
        if (-not (Test-IsJsonInteger $entriesVerified)) { Add-CaptureProblem $problems "container_entries_verified_not_published" }
        elseif ([int64]$entriesVerified -le 0) { Add-CaptureProblem $problems "container_no_entries_verified" }
    }

    $replayFile = [string](Get-ObjectProperty $manifest "replay_file")
    if ([string]::IsNullOrWhiteSpace($replayFile) -or [System.IO.Path]::GetFileName($replayFile) -ne $replayFile) { Add-CaptureProblem $problems "invalid_replay_file_name" }
    elseif (-not (Test-Path -LiteralPath (Join-Path $RunDir $replayFile) -PathType Leaf)) { Add-CaptureProblem $problems "replay_container_missing" }
    $rawPath = Join-Path $RunDir "collector_stream.bin"
    if (-not (Test-Path -LiteralPath $rawPath -PathType Leaf)) { Add-CaptureProblem $problems "raw_stream_missing" }

    $packaging = Get-ObjectProperty $manifest "replay_packaging"
    $packaged = @{}
    foreach ($item in @(Get-ObjectProperty $packaging "static_artifacts")) {
        $name = [string](Get-ObjectProperty $item "name")
        $sha = [string](Get-ObjectProperty $item "sha256")
        if ($packaged.ContainsKey($name)) { Add-CaptureProblem $problems "duplicate_static_artifact_$name" }
        $packaged[$name] = $sha
    }
    if ((@($packaged.Keys | Sort-Object) -join ',') -ne (($script:RequiredStaticArtifactNames | Sort-Object) -join ',')) { Add-CaptureProblem $problems "static_artifact_roster_mismatch" }
    foreach ($name in $script:RequiredStaticArtifactNames) {
        if (-not $packaged.ContainsKey($name) -or -not (Test-HexSha256 $packaged[$name])) { Add-CaptureProblem $problems "static_artifact_${name}_sha_invalid"; continue }
        if ($StaticArtifactPaths.ContainsKey($name)) {
            $path = [string]$StaticArtifactPaths[$name]
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { Add-CaptureProblem $problems "static_artifact_${name}_source_missing" }
            else {
                $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
                if ($actual -ne ([string]$packaged[$name]).ToLowerInvariant()) { Add-CaptureProblem $problems "static_artifact_${name}_source_sha_mismatch" }
            }
        }
    }

    if (-not (Test-Path -LiteralPath $SpeedProofPath -PathType Leaf)) { Add-CaptureProblem $problems "speed5_proof_missing" }
    else {
        try { $speedProof = Get-Content -LiteralPath $SpeedProofPath -Raw | ConvertFrom-Json }
        catch { $speedProof = $null; Add-CaptureProblem $problems "speed5_proof_not_json" }
        if ($null -ne $speedProof) {
            $speedProofRaw = Get-ObjectProperty $speedProof "speed_raw"
            if ([string](Get-ObjectProperty $speedProof "schema") -ne "battle-plan.speed5_proof.v2" -or
                -not (Test-IsJsonInteger $speedProofRaw) -or [int64]$speedProofRaw -ne 4) { Add-CaptureProblem $problems "speed5_not_verified" }
            if (-not (Test-HexSha256 (Get-ObjectProperty $speedProof "domain57_game_speed_gate_v2_sha256"))) { Add-CaptureProblem $problems "speed_root_semantics_not_verified" }
            $speedSampleCount = Get-ObjectProperty $speedProof "sample_count"
            if (-not (Test-IsJsonInteger $speedSampleCount) -or [int64]$speedSampleCount -lt 3) { Add-CaptureProblem $problems "speed5_insufficient_samples" }
            if (-not (Test-HexSha256 (Get-ObjectProperty $speedProof "exe_sha256"))) { Add-CaptureProblem $problems "speed5_exe_sha_invalid" }
            if (-not (Test-HexSha256 (Get-ObjectProperty $speedProof "collector_control_sha256"))) { Add-CaptureProblem $problems "collector_control_sha_invalid" }
        }
    }

    if (-not [string]::IsNullOrWhiteSpace($ExpectedStagedDll)) {
        if ([string](Get-ObjectProperty $manifest "dll") -ne $ExpectedStagedDll) { Add-CaptureProblem $problems "manifest_dll_is_not_unique_staged_copy" }
    }

    $capturePlan = $null
    $validatedDomainRosterAuthorityPath = $null
    $validatedDomainRosterAuthoritySha256 = $null
    if (-not [string]::IsNullOrWhiteSpace($CapturePlanPath) -and (Test-Path -LiteralPath $CapturePlanPath -PathType Leaf)) {
        try { $capturePlan = Get-Content -LiteralPath $CapturePlanPath -Raw | ConvertFrom-Json }
        catch { $capturePlan = $null; Add-CaptureProblem $problems "capture_plan_not_json" }
        if ($null -ne $capturePlan) {
            $planDomainIds = @(Get-ObjectProperty $capturePlan "production_profile_domain_ids")
            $planDomainNames = @(Get-ObjectProperty $capturePlan "production_profile_domain_names")
            $planDomainRoster = @(Get-ObjectProperty $capturePlan "production_profile_domain_roster")
            $planScheduledNames = @(Get-ObjectProperty $capturePlan "scheduled_domain_names")
            $planScheduledRoster = @(Get-ObjectProperty $capturePlan "scheduled_domain_roster")
            if (
                $planDomainIds.Count -ne $expectedRoster.Count -or
                $planDomainNames.Count -ne $expectedRoster.Count -or
                $planDomainRoster.Count -ne $expectedRoster.Count -or
                $planScheduledNames.Count -ne $expectedRoster.Count -or
                $planScheduledRoster.Count -ne $expectedRoster.Count
            ) {
                Add-CaptureProblem $problems "capture_plan_domain_roster_count_mismatch"
            } else {
                $seenScheduledNames = @{}
                for ($domainIndex = 0; $domainIndex -lt $expectedRoster.Count; $domainIndex++) {
                    $planRosterRow = $planDomainRoster[$domainIndex]
                    $planScheduledRow = $planScheduledRoster[$domainIndex]
                    $scheduledName = $planScheduledNames[$domainIndex]
                    if (
                        -not (Test-IsJsonInteger $planDomainIds[$domainIndex]) -or
                        [int64]$planDomainIds[$domainIndex] -ne [int64]$expectedRoster[$domainIndex].id -or
                        $planDomainNames[$domainIndex] -isnot [string] -or
                        [string]$planDomainNames[$domainIndex] -cne [string]$expectedRoster[$domainIndex].name -or
                        $null -eq $planRosterRow -or
                        (@($planRosterRow.PSObject.Properties.Name | Sort-Object) -join ',') -cne 'id,name' -or
                        -not (Test-IsJsonInteger (Get-ObjectProperty $planRosterRow "id")) -or
                        [int64](Get-ObjectProperty $planRosterRow "id") -ne [int64]$expectedRoster[$domainIndex].id -or
                        (Get-ObjectProperty $planRosterRow "name") -isnot [string] -or
                        [string](Get-ObjectProperty $planRosterRow "name") -cne [string]$expectedRoster[$domainIndex].name -or
                        $scheduledName -isnot [string] -or
                        [string]::IsNullOrWhiteSpace([string]$scheduledName) -or
                        $seenScheduledNames.ContainsKey([string]$scheduledName) -or
                        $null -eq $planScheduledRow -or
                        (@($planScheduledRow.PSObject.Properties.Name | Sort-Object) -join ',') -cne 'id,wire_name' -or
                        -not (Test-IsJsonInteger (Get-ObjectProperty $planScheduledRow "id")) -or
                        [int64](Get-ObjectProperty $planScheduledRow "id") -ne [int64]$expectedRoster[$domainIndex].id -or
                        (Get-ObjectProperty $planScheduledRow "wire_name") -isnot [string] -or
                        [string](Get-ObjectProperty $planScheduledRow "wire_name") -cne [string]$scheduledName
                    ) {
                        Add-CaptureProblem $problems "capture_plan_domain_roster_pair_mismatch"
                        break
                    }
                    $seenScheduledNames[[string]$scheduledName] = $true
                }
                if ((@((Get-ObjectProperty $manifest "scheduled_domains")) -join "`n") -cne ($planScheduledNames -join "`n")) {
                    Add-CaptureProblem $problems "manifest_scheduled_domain_roster_mismatch"
                }
                if ((@((Get-ObjectProperty $compact "expected_domains")) -join "`n") -cne ($planScheduledNames -join "`n")) {
                    Add-CaptureProblem $problems "compact_expected_domain_roster_mismatch"
                }
            }
            if (-not (Test-HexSha256 (Get-ObjectProperty $capturePlan "domain_roster_tool_sha256"))) {
                Add-CaptureProblem $problems "capture_plan_domain_roster_tool_sha256_invalid"
            }
            $authorityPath = [string](Get-ObjectProperty $capturePlan "domain_roster_authority_path")
            $expectedAuthorityPath = Join-Path (Split-Path -Parent $CapturePlanPath) "production_domain_roster_authority.json"
            $authoritySha256 = [string](Get-ObjectProperty $capturePlan "domain_roster_authority_sha256")
            if (
                [string]::IsNullOrWhiteSpace($authorityPath) -or
                [System.IO.Path]::GetFullPath($authorityPath) -cne [System.IO.Path]::GetFullPath($expectedAuthorityPath) -or
                -not (Test-Path -LiteralPath $authorityPath -PathType Leaf) -or
                -not (Test-HexSha256 $authoritySha256) -or
                (Get-FileHash -LiteralPath $authorityPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $authoritySha256
            ) {
                Add-CaptureProblem $problems "capture_plan_domain_roster_authority_binding_invalid"
            } else {
                # The parse has to land in a variable before it is wrapped:
                # ConvertFrom-Json emits a top-level JSON array as one
                # un-enumerated object, so @(<pipeline>) counted 1 row for any
                # roster and the count comparison below could never agree with
                # the multi-row $expectedRoster.
                try {
                    $authorityDocument = Get-Content -LiteralPath $authorityPath -Raw | ConvertFrom-Json
                    $authorityRoster = @($authorityDocument)
                }
                catch { $authorityRoster = @(); Add-CaptureProblem $problems "domain_roster_authority_not_json" }
                if (
                    $authorityRoster.Count -ne $expectedRoster.Count -or
                    $planScheduledNames.Count -ne $expectedRoster.Count
                ) {
                    Add-CaptureProblem $problems "domain_roster_authority_count_mismatch"
                } else {
                    for ($domainIndex = 0; $domainIndex -lt $expectedRoster.Count; $domainIndex++) {
                        $authorityRow = $authorityRoster[$domainIndex]
                        if (
                            $null -eq $authorityRow -or
                            (@($authorityRow.PSObject.Properties.Name | Sort-Object) -join ',') -cne 'id,name,wire_name' -or
                            -not (Test-IsJsonInteger (Get-ObjectProperty $authorityRow "id")) -or
                            [int64](Get-ObjectProperty $authorityRow "id") -ne [int64]$expectedRoster[$domainIndex].id -or
                            (Get-ObjectProperty $authorityRow "name") -isnot [string] -or
                            [string](Get-ObjectProperty $authorityRow "name") -cne [string]$expectedRoster[$domainIndex].name -or
                            (Get-ObjectProperty $authorityRow "wire_name") -isnot [string] -or
                            [string](Get-ObjectProperty $authorityRow "wire_name") -cne [string]$planScheduledNames[$domainIndex]
                        ) {
                            Add-CaptureProblem $problems "domain_roster_authority_pair_mismatch"
                            break
                        }
                    }
                }
                if (-not (@($problems) -contains "domain_roster_authority_not_json") -and
                    -not (@($problems) -contains "domain_roster_authority_count_mismatch") -and
                    -not (@($problems) -contains "domain_roster_authority_pair_mismatch")) {
                    $validatedDomainRosterAuthorityPath = [System.IO.Path]::GetFullPath($authorityPath)
                    $validatedDomainRosterAuthoritySha256 = $authoritySha256
                }
            }
            $planSourceInventory=[string](Get-ObjectProperty $capturePlan "static_bundle_source_inventory")
            if (-not (Test-Path -LiteralPath $SourceInventoryPath -PathType Leaf)) { Add-CaptureProblem $problems "source_inventory_missing" }
            elseif ([string]::IsNullOrWhiteSpace($planSourceInventory) -or [System.IO.Path]::GetFullPath($planSourceInventory) -cne [System.IO.Path]::GetFullPath($SourceInventoryPath) -or [string](Get-ObjectProperty $capturePlan "static_bundle_source_inventory_sha256") -cne (Get-FileHash -LiteralPath $SourceInventoryPath -Algorithm SHA256).Hash.ToLowerInvariant()) { Add-CaptureProblem $problems "source_inventory_plan_binding_mismatch" }
            $planLiveResolution=[string](Get-ObjectProperty $capturePlan "static_bundle_live_resolution_report")
            if (-not (Test-Path -LiteralPath $LiveResolutionReportPath -PathType Leaf)) { Add-CaptureProblem $problems "live_resolution_report_missing" }
            elseif ([string]::IsNullOrWhiteSpace($planLiveResolution) -or [System.IO.Path]::GetFullPath($planLiveResolution) -cne [System.IO.Path]::GetFullPath($LiveResolutionReportPath) -or [string](Get-ObjectProperty $capturePlan "static_bundle_live_resolution_report_sha256") -cne (Get-FileHash -LiteralPath $LiveResolutionReportPath -Algorithm SHA256).Hash.ToLowerInvariant()) { Add-CaptureProblem $problems "live_resolution_report_plan_binding_mismatch" }
            else {
                try { $liveResolution = Get-Content -LiteralPath $LiveResolutionReportPath -Raw | ConvertFrom-Json }
                catch { $liveResolution = $null; Add-CaptureProblem $problems "live_resolution_report_not_json" }
                if ($null -ne $liveResolution) {
                    $reportedProcess = Get-ObjectProperty $liveResolution "process"
                    $reportedPid = [int](Get-ObjectProperty $reportedProcess "pid")
                    $reportedCreation = [int64](Get-ObjectProperty $reportedProcess "creation_filetime_utc")
                    $plannedPid = [int](Get-ObjectProperty $capturePlan "pid")
                    $plannedCreation = [int64](Get-ObjectProperty $capturePlan "process_creation_filetime_utc")
                    if (
                        $reportedPid -le 0 -or $reportedCreation -le 0 -or
                        $plannedPid -le 0 -or $plannedCreation -le 0 -or
                        $reportedPid -ne $plannedPid -or $reportedCreation -ne $plannedCreation
                    ) { Add-CaptureProblem $problems "live_resolution_process_identity_mismatch" }
                }
            }
        }
    }
    # STRICT identity types (re-gate2 finding 4): capture_id / session_id must be
    # GENUINE, non-blank JSON STRINGS. The old [string] coercion silently minted a
    # usable identity from a numeric capture_id (123 -> "123") or a boolean
    # session_id (true -> "True"). Wrong type = its own fail-closed problem, and the
    # coerced value is NEVER adopted as the attempt identity.
    $rawCaptureId = Get-ObjectProperty $capturePlan "capture_id"
    $expectedCaptureId = ""
    if ($rawCaptureId -is [string]) { $expectedCaptureId = $rawCaptureId }
    elseif ($null -ne $rawCaptureId) { Add-CaptureProblem $problems "capture_id_not_a_json_string" }
    $rawSessionId = Get-ObjectProperty $manifest "session_id"
    $expectedSessionId = ""
    if ($rawSessionId -is [string]) { $expectedSessionId = $rawSessionId }
    elseif ($null -ne $rawSessionId) { Add-CaptureProblem $problems "session_id_not_a_json_string" }
    $expectedStreamSha256 = [string](Get-ObjectProperty $finalize "stream_sha256")
    if ([string]::IsNullOrWhiteSpace($expectedCaptureId)) { Add-CaptureProblem $problems "capture_id_missing_from_plan" }
    if ([string]::IsNullOrWhiteSpace($expectedSessionId)) { Add-CaptureProblem $problems "session_id_missing_from_manifest" }
    if (-not (Test-HexSha256 $expectedStreamSha256)) { Add-CaptureProblem $problems "capture_stream_sha256_invalid" }
    if ($null -ne $capturePlan) {
        $rawPlanSessionId = Get-ObjectProperty $capturePlan "session_id"
        if ($null -ne $rawPlanSessionId -and $rawPlanSessionId -isnot [string]) {
            Add-CaptureProblem $problems "plan_session_id_not_a_json_string"
        } elseif ([string]$rawPlanSessionId -cne $expectedSessionId) {
            Add-CaptureProblem $problems "capture_plan_session_id_mismatch"
        }
    }

    # These gates are deliberately independent of recorder-authored claims. A
    # caller cannot obtain a production PASS without executing every verifier.
    $toolResults = [System.Collections.Generic.List[object]]::new()
    $toolLogDir = Join-Path $RunDir "independent_validation"
    $requiredConfig = @($CompactGateExe,$FoldCheckExe,$ContainerVerifyExe,$PythonExe,$StaticBundleScript,$StaticBundleDir,$GameExePath,$ModStatePath,$DlcStatePath,$LoadOrderPath,$SourceInventoryPath,$LiveResolutionReportPath,$CapturePlanPath,$SpeedRootGatePath,$CollectorControlPath,$SpeedMonitorPath,$SpeedStreamVerifier,$SpeedEvidenceTranscriptPath,$SpeedEvidenceNonceLedgerRoot,$SpeedEvidenceVerifier,$ExpectedSpeedEvidenceVerifierSha256,$ExpectedSpeedEvidenceProbeSha256,$ExpectedSpeedStaticDisasmSha256)
    if (@($requiredConfig | Where-Object { [string]::IsNullOrWhiteSpace([string]$_) }).Count -ne 0) {
        Add-CaptureProblem $problems "independent_gate_configuration_missing"
    }
    else {
        $compactReport = Join-Path $toolLogDir "compact_gate.report.json"
        $compactGateResult = Invoke-IndependentCaptureGate -Name "compact_gate" -FilePath $CompactGateExe -Arguments @("--run-dir",$RunDir,"--min-game-hours",([string]$requestedGameHours),"--max-stream-bytes","2199023255552","--max-record-bytes","268435456","--max-bytes-per-frame","2147483648","--require-zero-loss","--write-report",$compactReport) -LogDir $toolLogDir -TimeoutSeconds $ToolTimeoutSeconds -CancellationGraceSeconds $CancellationGraceSeconds
        $compactGateResult | Add-Member -NotePropertyName report_path -NotePropertyValue ([IO.Path]::GetFullPath($compactReport))
        $compactGateResult | Add-Member -NotePropertyName report_sha256 -NotePropertyValue $(if (Test-Path -LiteralPath $compactReport -PathType Leaf) { (Get-FileHash -LiteralPath $compactReport -Algorithm SHA256).Hash.ToLowerInvariant() } else { $null })
        $toolResults.Add($compactGateResult)
        $foldReport = Join-Path $toolLogDir "fold_check.report.json"
        $foldArguments = @()
        try { $foldArguments = @(Get-ProductionFoldCheckArguments -Manifest $manifest -RawPath $rawPath -ReportPath $foldReport) }
        catch { Add-CaptureProblem $problems 'compact_stream_identity_mode_missing_or_invalid' }
        if ($foldArguments.Count -gt 0) {
            $toolResults.Add((Invoke-IndependentCaptureGate -Name "fold_check" -FilePath $FoldCheckExe -Arguments $foldArguments -LogDir $toolLogDir -TimeoutSeconds $ToolTimeoutSeconds -CancellationGraceSeconds $CancellationGraceSeconds))
        }
        $replayPath = if ([string]::IsNullOrWhiteSpace($replayFile)) { Join-Path $RunDir "native_session.battleplan" } else { Join-Path $RunDir $replayFile }
        $toolResults.Add((Invoke-IndependentCaptureGate -Name "container_verify" -FilePath $ContainerVerifyExe -Arguments @("verify",$replayPath) -LogDir $toolLogDir -TimeoutSeconds $ToolTimeoutSeconds -CancellationGraceSeconds $CancellationGraceSeconds))
        $toolResults.Add((Invoke-IndependentCaptureGate -Name "static_bundle_verify" -FilePath $PythonExe -Arguments @($StaticBundleScript,"verify","--bundle",$StaticBundleDir,"--exe",$GameExePath,"--mod-state",$ModStatePath,"--dlc-state",$DlcStatePath,"--load-order",$LoadOrderPath,"--source-inventory",$SourceInventoryPath,"--live-resolution-report",$LiveResolutionReportPath,"--expected-capture-id",$expectedCaptureId,"--expected-session-id",$expectedSessionId,"--expected-stream-sha256",$expectedStreamSha256) -LogDir $toolLogDir -TimeoutSeconds $ToolTimeoutSeconds -CancellationGraceSeconds $CancellationGraceSeconds))
        $speedReport = Join-Path $toolLogDir "speed5_stream_gate.report.json"
        $toolResults.Add((Invoke-IndependentCaptureGate -Name "speed5_stream_gate" -FilePath $PythonExe -Arguments @($SpeedStreamVerifier,"--stream",$rawPath,"--manifest",$manifestPath,"--speed-proof",$SpeedProofPath,"--monitor",$SpeedMonitorPath,"--capture-plan",$CapturePlanPath,"--root-gate",$SpeedRootGatePath,"--collector-control",$CollectorControlPath,"--evidence-transcript",$SpeedEvidenceTranscriptPath,"--nonce-ledger-root",$SpeedEvidenceNonceLedgerRoot,"--evidence-verifier",$SpeedEvidenceVerifier,"--expected-verifier-sha256",$ExpectedSpeedEvidenceVerifierSha256,"--expected-probe-sha256",$ExpectedSpeedEvidenceProbeSha256,"--expected-static-sha256",$ExpectedSpeedStaticDisasmSha256,"--out",$speedReport) -LogDir $toolLogDir -TimeoutSeconds $ToolTimeoutSeconds -CancellationGraceSeconds $CancellationGraceSeconds))
        foreach ($toolResult in $toolResults) { if (-not $toolResult.passed) { Add-CaptureProblem $problems ("independent_{0}_failed" -f $toolResult.name) } }
    }

    # Attempt-binding fields: a consumer (the acceptance ladder) can prove this
    # report was produced for THIS capture attempt by matching the manifest it
    # graded (path + content sha) and the capture/session identity it carries. A
    # hand-written {schema, ok:true} report cannot supply a manifest_sha256 that
    # matches the run's actual manifest, so it fails the ladder's binding check.
    $manifestSha256 = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
    return [pscustomobject]@{
        schema = "battle-plan.production_capture_validation.v2"
        ok = ($problems.Count -eq 0)
        canonical_failure_mask = ("0x{0:X}" -f $script:CanonicalCompactFailureMask)
        expected_domain_count = $expectedIds.Count
        expected_domain_roster = @($expectedRoster)
        scheduled_domain_roster = @(
            if ($null -ne $capturePlan) {
                @(Get-ObjectProperty $capturePlan "scheduled_domain_roster")
            }
        )
        domain_roster_authority_path = $validatedDomainRosterAuthorityPath
        domain_roster_authority_sha256 = $validatedDomainRosterAuthoritySha256
        frame_count = $frameCount
        manifest_path = $manifestPath
        manifest_sha256 = $manifestSha256
        capture_id = $expectedCaptureId
        session_id = $expectedSessionId
        stream_sha256 = $expectedStreamSha256
        problems = @($problems)
        independent_gates = @($toolResults)
    }
}

# ---------------------------------------------------------------------------
# Acceptance-telemetry (F5) schema/threshold gate + offline self-tests. Reused
# by the ladder orchestrator and wired into Test-ProductionCaptureArtifacts above.
# ---------------------------------------------------------------------------

function Get-ManifestPath {
    param([Parameter(Mandatory = $true)][string]$RunDir)
    return (Join-Path $RunDir "native_session_manifest.json")
}

function Read-CaptureManifest {
    <#
        Load and parse a run's native_session_manifest.json. Throws (fail-closed)
        when the directory or manifest is missing or is not valid JSON.
    #>
    param([Parameter(Mandatory = $true)][string]$RunDir)
    if (-not (Test-Path -LiteralPath $RunDir -PathType Container)) {
        throw "run directory not found: $RunDir"
    }
    $manifestPath = Get-ManifestPath -RunDir $RunDir
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "native_session_manifest.json not found in run directory: $manifestPath"
    }
    $raw = Get-Content -LiteralPath $manifestPath -Raw
    if ($null -eq $raw -or $raw.Trim().Length -eq 0) {
        throw "manifest is empty: $manifestPath"
    }
    try {
        return $raw | ConvertFrom-Json
    } catch {
        throw "manifest is not valid JSON ($manifestPath): $($_.Exception.Message)"
    }
}

function Test-HasProperty {
    # PS 5.1: distinguish an ABSENT member from a member whose value is $null.
    param(
        [Parameter(Mandatory = $true)][AllowNull()]$Object,
        [Parameter(Mandatory = $true)][string]$Name
    )
    if ($null -eq $Object) { return $false }
    if ($Object -isnot [psobject]) { return $false }
    return ($Object.PSObject.Properties.Name -contains $Name)
}

function Test-IsJsonNumber {
    <#
        A real numeric JSON value round-trips through ConvertFrom-Json as a numeric
        .NET type, NEVER a bool and NEVER a string. A hand-written "not-a-number"
        (string) or a JSON bool must not masquerade as a timing statistic. Used to
        reject malformed non-null telemetry values, not just nulls.
    #>
    param($Value)
    if ($null -eq $Value) { return $false }
    if ($Value -is [bool]) { return $false }
    $isNumericType = ($Value -is [int] -or $Value -is [long] -or $Value -is [double] -or
            $Value -is [single] -or $Value -is [decimal] -or $Value -is [int16] -or
            $Value -is [uint16] -or $Value -is [uint32] -or $Value -is [uint64] -or $Value -is [byte])
    if (-not $isNumericType) { return $false }
    # A telemetry number must be FINITE (finding 2). PS 5.1 ConvertFrom-Json parses
    # bare JSON NaN / Infinity / -Infinity as System.Double, so a count=1 manifest
    # with non-finite percentiles or clock-skew values would otherwise sail through
    # as a "real number". A NaN/Infinity timing statistic is malformed -> reject.
    if (($Value -is [double]) -and ([double]::IsNaN($Value) -or [double]::IsInfinity($Value))) { return $false }
    if (($Value -is [single]) -and ([single]::IsNaN($Value) -or [single]::IsInfinity($Value))) { return $false }
    return $true
}

function Test-IsJsonInteger {
    <#
        A count/positive-integer field must be a genuine whole JSON INTEGER token.
        ConvertFrom-Json parses a plain JSON integer as [long] (or [decimal] when it
        exceeds Int64 but fits 96-bit decimal); it parses any number written with a
        decimal point OR an exponent (e.g. 10.0, 1e3, 9007199254740993e0) as a
        [double], which is ALREADY ROUNDED at parse time. A [double] therefore cannot
        be trusted for lossless integer validation — the precision is gone before this
        predicate ever runs (re-gate: 9007199254740993e0 and 9007199254740992.5e0 both
        arrive as the same rounded double). So a count is accepted ONLY as an integer
        type or an exact [decimal]; a [double]/[single] is always rejected. This costs
        nothing real: the Rust recorder serialises counts as plain JSON integers
        (serde_json never emits a float for an integer field), so a float-form count is
        malformed input, not a legitimate value. Bools and strings are rejected too.
    #>
    param($Value)
    if ($null -eq $Value) { return $false }
    if ($Value -is [bool]) { return $false }
    # Integer-typed values are accepted only when they land inside the signed
    # Int64 range, because EVERY caller subsequently casts through [int64]. A
    # [uint64] above Int64.MaxValue (JSON 9223372036854775808 parses as uint64
    # on some PS 5.1 hosts) would otherwise pass and then THROW an overflow at
    # the cast site. Types whose whole domain is inside Int64 are safe outright.
    if ($Value -is [int] -or $Value -is [long] -or $Value -is [int64] -or
        $Value -is [int16] -or $Value -is [sbyte] -or
        $Value -is [uint16] -or $Value -is [uint32] -or $Value -is [byte]) { return $true }
    if ($Value -is [uint64]) {
        return ([uint64]$Value -le [uint64][int64]::MaxValue)
    }
    if ($Value -is [double] -or $Value -is [single]) {
        # Already-rounded at parse time — never lossless. Reject unconditionally
        # (this also covers NaN/±Infinity). A legitimate count is never a float.
        return $false
    }
    if ($Value -is [decimal]) {
        # [decimal] preserves the exact JSON digits (ConvertFrom-Json uses it for
        # large plain integers), so integrality is decided EXACTLY via Truncate in
        # decimal space; then require Int64 range (also exact in decimal).
        $dec = [decimal]$Value
        if ([math]::Truncate($dec) -ne $dec) { return $false }
        return ($dec -ge [decimal][int64]::MinValue -and $dec -le [decimal][int64]::MaxValue)
    }
    return $false
}

function Resolve-JsonPath {
    <#
        Navigate a dotted path (e.g. "compact_stream.frame_time_us") through a
        parsed manifest. Returns a result object with:
          Found   - $true only if every segment is a PRESENT member
          Value   - the resolved value (may itself be $null if the leaf is null)
          Missing - the first path prefix that did not resolve
    #>
    param(
        [Parameter(Mandatory = $true)]$Manifest,
        [Parameter(Mandatory = $true)][string]$Path
    )
    $current = $Manifest
    $walked = @()
    foreach ($segment in ($Path -split "\.")) {
        $walked += $segment
        if (-not (Test-HasProperty -Object $current -Name $segment)) {
            return [pscustomobject]@{
                Found   = $false
                Value   = $null
                Missing = ($walked -join ".")
            }
        }
        $current = $current.$segment
    }
    return [pscustomobject]@{
        Found   = $true
        Value   = $current
        Missing = $null
    }
}

function Test-TimingSummaryFields {
    <#
        Assert a timing-summary object carries every required percentile field.
        Appends "<label>.<field> missing" problems (fail-closed) for absentees.
    #>
    param(
        [Parameter(Mandatory = $true)]$Summary,
        [Parameter(Mandatory = $true)][string[]]$RequiredFields,
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][System.Collections.Generic.List[string]]$Problems
    )
    foreach ($field in $RequiredFields) {
        if (-not (Test-HasProperty -Object $Summary -Name $field)) {
            $Problems.Add("$Label.$field missing")
        }
    }
}

function Get-AcceptanceTelemetryProblems {
    <#
        Fail-closed schema gate for the acceptance-telemetry fields (F5). Returns
        a (possibly empty) string list of problems. Empty == the manifest carries
        every required frame-time / sink-write / clock-skew field.
    #>
    param([Parameter(Mandatory = $true)]$Manifest)
    $problems = [System.Collections.Generic.List[string]]::new()

    $acceptance = Resolve-JsonPath -Manifest $Manifest -Path "acceptance_telemetry"
    if (-not $acceptance.Found) {
        $problems.Add("acceptance_telemetry missing")
    } elseif ($null -eq $acceptance.Value) {
        $problems.Add("acceptance_telemetry is null")
    } else {
        $schema = Resolve-JsonPath -Manifest $Manifest -Path "acceptance_telemetry.schema"
        if (-not $schema.Found -or $schema.Value -cne "battle-plan.acceptance_telemetry.v1") {
            $problems.Add("acceptance_telemetry.schema is absent or unsupported")
        }
        $complete = Resolve-JsonPath -Manifest $Manifest -Path "acceptance_telemetry.complete"
        if (-not $complete.Found -or $complete.Value -isnot [bool] -or $complete.Value -ne $true) {
            $problems.Add("acceptance_telemetry.complete is not boolean true")
        }
        $producerProblems = Resolve-JsonPath -Manifest $Manifest -Path "acceptance_telemetry.problems"
        if (-not $producerProblems.Found -or $null -eq $producerProblems.Value) {
            $problems.Add("acceptance_telemetry.problems missing")
        } elseif (@($producerProblems.Value).Count -ne 0) {
            $problems.Add("acceptance_telemetry.problems is not empty")
        }
    }
    $clockScope = Resolve-JsonPath -Manifest $Manifest -Path "acceptance_telemetry.clock_skew_scope"
    if (-not $clockScope.Found -or $clockScope.Value -cne "complete physical H4CR capture; capture-set scoped during rollover") {
        $problems.Add("acceptance_telemetry.clock_skew_scope is absent or unsupported")
    }

    # frame_time_us (per-frame extraction time) under compact_stream.
    $frame = Resolve-JsonPath -Manifest $Manifest -Path "compact_stream.frame_time_us"
    if (-not $frame.Found) {
        $problems.Add("compact_stream.frame_time_us missing (at $($frame.Missing))")
    } else {
        Test-TimingSummaryFields -Summary $frame.Value -RequiredFields $script:RequiredFrameTimeFields `
            -Label "compact_stream.frame_time_us" -Problems $problems
        # VALUE gate (finding 2): frame_time_us percentiles were PRESENCE-ONLY, so a
        # non-finite value slipped through. PS 5.1 ConvertFrom-Json parses bare JSON
        # NaN / Infinity / -Infinity as System.Double, and the downstream ceiling
        # compare (Test-NumericThreshold: `NaN -gt Max` is FALSE) then accepted it.
        # Route each percentile through the SAME strict finite-number validation the
        # sink_write_us / clock_skew fields use (Test-IsJsonNumber rejects null, bools,
        # strings, AND NaN/Infinity), so a count>0 frame block with a non-finite or
        # non-numeric p95 / max cannot pass on presence alone.
        # count VALUE gate (re-gate2 finding 3): count was presence-only, so a string
        # "10" or a bare JSON NaN count sailed through. Mirror the sink_write_us.count
        # handling: a genuine POSITIVE finite JSON INTEGER, or FAIL.
        $frameCountField = Resolve-JsonPath -Manifest $Manifest -Path "compact_stream.frame_time_us.count"
        if ($frameCountField.Found) {
            if ($null -eq $frameCountField.Value) {
                $problems.Add("compact_stream.frame_time_us.count is null (no frame-time telemetry)")
            } elseif (-not (Test-IsJsonInteger $frameCountField.Value)) {
                $problems.Add("compact_stream.frame_time_us.count is not an integer ('$($frameCountField.Value)')")
            } elseif ([int64]$frameCountField.Value -le 0) {
                $problems.Add("compact_stream.frame_time_us.count is $($frameCountField.Value) (no frame-time samples captured)")
            }
        }
        foreach ($pf in @("avg", "p50", "p95", "p99", "max")) {
            $fv = Resolve-JsonPath -Manifest $Manifest -Path "compact_stream.frame_time_us.$pf"
            if ($fv.Found) {
                if ($null -eq $fv.Value) {
                    $problems.Add("compact_stream.frame_time_us.$pf is null (empty/absent sample distribution)")
                } elseif (-not (Test-IsJsonNumber $fv.Value)) {
                    $problems.Add("compact_stream.frame_time_us.$pf is not numeric ('$($fv.Value)')")
                }
            }
        }
    }

    # Host sink timing under acceptance_telemetry. The passive H4CR stream and
    # its authenticated sequence/FRAME_END/H4DG closure are the delivery
    # authority; no collector-side pause/latch receipt exists.
    $sink = Resolve-JsonPath -Manifest $Manifest -Path "acceptance_telemetry.sink_write_us"
    if (-not $sink.Found) {
        $problems.Add("acceptance_telemetry.sink_write_us missing (at $($sink.Missing))")
    } else {
        Test-TimingSummaryFields -Summary $sink.Value -RequiredFields $script:RequiredSinkWriteFields `
            -Label "acceptance_telemetry.sink_write_us" -Problems $problems
        # VALUE gate (not just presence): count == 0 means no sink-write samples
        # were captured, and the recorder emits null percentiles over an empty
        # sample. Either signals NO DATA and must FAIL, never pass on presence.
        # count must be a genuine POSITIVE INTEGER, consistent with the percentiles.
        # A malformed non-null count (e.g. the string "1", a bool, a fractional
        # double) is as bad as a null one and must FAIL, not pass on non-nullness.
        $sinkCount = Resolve-JsonPath -Manifest $Manifest -Path "acceptance_telemetry.sink_write_us.count"
        if ($sinkCount.Found) {
            if ($null -eq $sinkCount.Value) {
                $problems.Add("acceptance_telemetry.sink_write_us.count is null (no sink-write telemetry)")
            } elseif (-not (Test-IsJsonInteger $sinkCount.Value)) {
                $problems.Add("acceptance_telemetry.sink_write_us.count is not an integer ('$($sinkCount.Value)')")
            } elseif ([int64]$sinkCount.Value -le 0) {
                $problems.Add("acceptance_telemetry.sink_write_us.count is $($sinkCount.Value) (no sink-write samples captured)")
            }
        }
        # Each percentile must be a REAL NUMBER. Reject null (empty distribution)
        # AND malformed non-numeric values (e.g. "not-a-number"), so a count=1 with
        # non-numeric timing stats cannot pass on presence alone.
        foreach ($pf in @("avg", "p50", "p95", "p99", "max")) {
            $pv = Resolve-JsonPath -Manifest $Manifest -Path "acceptance_telemetry.sink_write_us.$pf"
            if ($pv.Found) {
                if ($null -eq $pv.Value) {
                    $problems.Add("acceptance_telemetry.sink_write_us.$pf is null (empty/absent sample distribution)")
                } elseif (-not (Test-IsJsonNumber $pv.Value)) {
                    $problems.Add("acceptance_telemetry.sink_write_us.$pf is not numeric ('$($pv.Value)')")
                }
            }
        }
    }

    # clock_skew under acceptance_telemetry.
    $skew = Resolve-JsonPath -Manifest $Manifest -Path "acceptance_telemetry.clock_skew"
    if (-not $skew.Found) {
        $problems.Add("acceptance_telemetry.clock_skew missing (at $($skew.Missing))")
    } elseif ($null -eq $skew.Value) {
        # unmeasured() now serializes as JSON null (a distinct "no live loop ran
        # here" state), NOT a zero-drift wall_monotonic_ok=true sample. Treat it
        # as unmeasured and FAIL a live acceptance capture fail-closed, rather
        # than accepting a fabricated passing measurement.
        $problems.Add("acceptance_telemetry.clock_skew is null (unmeasured: no live capture loop ran; not a passing clock sample)")
    } else {
        foreach ($field in $script:RequiredClockSkewFields) {
            if (-not (Test-HasProperty -Object $skew.Value -Name $field)) {
                $problems.Add("acceptance_telemetry.clock_skew.$field missing")
            }
        }
        # VALUE gate: the numeric skew fields must be REAL NUMBERS. A present but
        # null/non-numeric elapsed or drift value is a malformed timing statistic
        # and must FAIL, not pass on mere presence.
        foreach ($nf in @("wall_elapsed_us", "mono_elapsed_us", "drift_us")) {
            if (Test-HasProperty -Object $skew.Value -Name $nf) {
                $nv = $skew.Value.$nf
                if ($null -eq $nv) {
                    $problems.Add("acceptance_telemetry.clock_skew.$nf is null (unmeasured timing value)")
                } elseif (-not (Test-IsJsonNumber $nv)) {
                    $problems.Add("acceptance_telemetry.clock_skew.$nf is not numeric ('$nv')")
                }
            }
        }
        # VALUE gate: wall_monotonic_ok must be a REAL BOOLEAN true. A present-but-
        # false flag means the wall clock ran backwards; a non-boolean truthy value
        # (numeric 1, the string "true") is a forged/malformed flag. Either must
        # FAIL, not pass on presence or on PowerShell's loose truthiness coercion.
        if (Test-HasProperty -Object $skew.Value -Name "wall_monotonic_ok") {
            $wmo = $skew.Value.wall_monotonic_ok
            if ($wmo -isnot [bool] -or $wmo -ne $true) {
                $problems.Add("acceptance_telemetry.clock_skew.wall_monotonic_ok is not true (must be boolean true; got '$wmo' - wall clock non-monotonic or malformed flag)")
            }
        }
    }

    return , $problems.ToArray()
}

function Test-NumericThreshold {
    <#
        Compare a resolved numeric telemetry value against a maximum. Fail-closed:
        an absent field, a null value, or a non-numeric value is a PROBLEM, as is a
        value strictly greater than $Max.
    #>
    param(
        [Parameter(Mandatory = $true)]$Manifest,
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][double]$Max,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][System.Collections.Generic.List[string]]$Problems
    )
    $resolved = Resolve-JsonPath -Manifest $Manifest -Path $Path
    if (-not $resolved.Found) {
        $Problems.Add("$Path missing (threshold check)")
        return
    }
    $value = $resolved.Value
    if ($null -eq $value) {
        $Problems.Add("$Path is null (threshold check <= $Max)")
        return
    }
    $parsed = 0.0
    if (-not [double]::TryParse(
            [string]$value,
            [System.Globalization.NumberStyles]::Float,
            [System.Globalization.CultureInfo]::InvariantCulture,
            [ref]$parsed)) {
        $Problems.Add("$Path is not numeric ('$value')")
        return
    }
    # Fail-closed on NON-FINITE (finding 2): PS 5.1 ConvertFrom-Json parses bare JSON
    # NaN / Infinity / -Infinity as System.Double, and TryParse likewise accepts the
    # strings "NaN" / "Infinity". `NaN -gt $Max` and `Infinity -gt $Max` both evaluate
    # WRONG for a ceiling gate (NaN comparisons are false), so a non-finite statistic
    # would silently pass. Reject any non-finite value BEFORE the threshold compare.
    if ([double]::IsNaN($parsed) -or [double]::IsInfinity($parsed)) {
        $Problems.Add("$Path is not a finite number ('$value') (threshold check <= $Max)")
        return
    }
    if ($parsed -gt $Max) {
        $Problems.Add("$Path = $parsed exceeds max $Max")
    }
}

function Test-ProductionCaptureManifest {
    <#
        The reusable production-capture gate. Runs the fail-closed schema gate, the
        hourly-requirement gate, and the optional per-rung numeric gates, returning
        a verdict object the ladder can persist:
          Passed   - [bool] every gate held
          Problems - [string[]] each failure, empty when Passed

        Optional per-rung gates:
          -MaxFrameTimeUs        frame_time_us.p95 <= this (e.g. 100000 for the
                                 speed-5 "<100ms/frame" rung)
          -RequireZeroSkipped    skipped_game_hours must be 0
          -RequireZeroDomainGaps compact_stream.domain_gap_record_count must be 0
          -RequireHourlyMet      hourly_requirement.met must be $true
    #>
    param(
        [Parameter(Mandatory = $true, ParameterSetName = "RunDir")]
        [string]$RunDir,

        [Parameter(Mandatory = $true, ParameterSetName = "Manifest")]
        $Manifest,

        [double]$MaxFrameTimeUs = 0,
        [switch]$RequireZeroSkipped,
        [switch]$RequireZeroDomainGaps,
        [switch]$RequireHourlyMet
    )

    $problems = [System.Collections.Generic.List[string]]::new()

    if ($PSCmdlet.ParameterSetName -eq "RunDir") {
        try {
            $Manifest = Read-CaptureManifest -RunDir $RunDir
        } catch {
            return [pscustomobject]@{
                Passed   = $false
                Problems = @("manifest load failed: $($_.Exception.Message)")
            }
        }
    }

    # 1. Acceptance-telemetry schema gate (fail-closed if any field absent).
    foreach ($p in (Get-AcceptanceTelemetryProblems -Manifest $Manifest)) {
        $problems.Add($p)
    }

    # 2. Hourly-requirement gate (the standing "met" flag).
    if ($RequireHourlyMet) {
        $met = Resolve-JsonPath -Manifest $Manifest -Path "hourly_requirement.met"
        if (-not $met.Found) {
            $problems.Add("hourly_requirement.met missing")
        } elseif ($met.Value -isnot [bool]) {
            # PowerShell coerces on -eq: `1 -eq $true` and `"true" -eq $true` are
            # both True, so a manifest publishing the NUMBER 1 or the STRING
            # "true" satisfied a boolean requirement it never made.
            $problems.Add("hourly_requirement.met is not a boolean")
        } elseif (-not $met.Value) {
            $problems.Add("hourly_requirement.met is not true")
        }
    }

    # 3. Zero-skipped-hours gate.
    if ($RequireZeroSkipped) {
        $skipped = Resolve-JsonPath -Manifest $Manifest -Path "skipped_game_hours"
        if (-not $skipped.Found) {
            $problems.Add("skipped_game_hours missing")
        } elseif (-not (Test-IsJsonInteger $skipped.Value)) {
            # `[int64]"0"` is 0 and `[int64]$false` is 0, so the string "0" and
            # the boolean false both satisfied "must be 0".
            $problems.Add("skipped_game_hours is not an integer")
        } elseif ([int64]$skipped.Value -ne 0) {
            $problems.Add("skipped_game_hours = $($skipped.Value) (must be 0)")
        }
    }

    # 4. Optional clean-capture gate. The product accepts typed H4DG outcomes,
    # but a speed-five clean-capture rung may require that none occurred.
    if ($RequireZeroDomainGaps) {
        $gapCount = Resolve-JsonPath -Manifest $Manifest -Path "compact_stream.domain_gap_record_count"
        if (-not $gapCount.Found) {
            $problems.Add("compact_stream.domain_gap_record_count missing")
        } elseif (-not (Test-IsJsonInteger $gapCount.Value)) {
            $problems.Add("compact_stream.domain_gap_record_count is not an integer")
        } elseif ([int64]$gapCount.Value -ne 0) {
            $problems.Add("compact_stream.domain_gap_record_count = $($gapCount.Value) (must be 0)")
        }
    }

    # 5. Per-frame time ceiling (speed-5 rung: p95 frame time under the budget).
    if ($MaxFrameTimeUs -gt 0) {
        Test-NumericThreshold -Manifest $Manifest -Path "compact_stream.frame_time_us.p95" `
            -Max $MaxFrameTimeUs -Problems $problems
    }

    return [pscustomobject]@{
        Passed   = ($problems.Count -eq 0)
        Problems = $problems.ToArray()
    }
}

# ---------------------------------------------------------------------------
# Offline self-tests: fabricate manifests and assert the fail-closed behavior.
# ---------------------------------------------------------------------------

function New-TestTimingSummary {
    param(
        [int]$Count = 10,
        [double]$Avg = 500,
        [int]$P50 = 400,
        [int]$P95 = 900,
        [int]$P99 = 980,
        [int]$Max = 1000
    )
    return [ordered]@{
        count = $Count
        avg   = $Avg
        p50   = $P50
        p95   = $P95
        p99   = $P99
        max   = $Max
    }
}

function New-TestManifest {
    <#
        A schema-complete, gate-passing manifest, as an ordered hashtable that
        ConvertTo-Json | ConvertFrom-Json round-trips into the shape the validator
        walks. Frame-time p95 defaults well under 100 ms.
    #>
    param(
        [int]$FrameP95 = 5000,
        [int]$SkippedHours = 0,
        [int]$DomainGapRecordCount = 0,
        [bool]$HourlyMet = $true
    )
    return [ordered]@{
        schema              = "native_collector_session_v1"
        skipped_game_hours  = $SkippedHours
        hourly_requirement  = [ordered]@{ required = $true; met = $HourlyMet; problems = @() }
        compact_stream      = [ordered]@{
            frame_time_us = (New-TestTimingSummary -P95 $FrameP95)
            domain_gap_record_count = $DomainGapRecordCount
        }
        acceptance_telemetry = [ordered]@{
            schema       = "battle-plan.acceptance_telemetry.v1"
            complete     = $true
            problems     = @()
            sink_write_us = (New-TestTimingSummary -P95 60 -Max 120 -Avg 25 -P50 20 -P99 100)
            clock_skew   = [ordered]@{
                wall_elapsed_us   = 300000000
                mono_elapsed_us   = 300000100
                drift_us          = -100
                wall_monotonic_ok = $true
            }
            clock_skew_scope = "complete physical H4CR capture; capture-set scoped during rollover"
        }
    }
}

function ConvertTo-RoundTrippedManifest {
    param([Parameter(Mandatory = $true)]$HashManifest)
    return ($HashManifest | ConvertTo-Json -Depth 12 | ConvertFrom-Json)
}

function Invoke-ValidationSelfTest {
    $failures = [System.Collections.Generic.List[string]]::new()

    function Assert-True {
        param([bool]$Condition, [string]$Message)
        if (-not $Condition) { $failures.Add($Message) }
    }

    # Explicit producer modes must reach the independent CLI without its default.
    foreach ($case in @(@('identity_v1','identity-v1'), @('legacy','legacy'))) {
        $modeManifest = ConvertTo-RoundTrippedManifest @{ compact_stream_identity_mode=$case[0] }
        $actualArgs = @(Get-ProductionFoldCheckArguments -Manifest $modeManifest -RawPath 'C:\capture with spaces\raw.bin' -ReportPath 'C:\report with spaces\fold.json')
        $expectedArgs = @('hash','C:\capture with spaces\raw.bin','--expected-profile','production','--identity-mode',$case[1],'--out','C:\report with spaces\fold.json')
        Assert-True (($actualArgs -join '|') -ceq ($expectedArgs -join '|')) "explicit mode $($case[0]) must select exact independent fold grammar and preserve path arguments"
    }
    foreach ($invalidMode in @($null, '', 'Identity_V1', 'identity-v1', 'identity_v1 ', 'unknown', 1, $true, @('identity_v1'), @{mode='identity_v1'})) {
        $modeManifest = ConvertTo-RoundTrippedManifest @{ compact_stream_identity_mode=$invalidMode }
        $refused = $false
        try { [void](Get-ProductionFoldCheckArguments -Manifest $modeManifest -RawPath 'raw.bin' -ReportPath 'fold.json') }
        catch { $refused = $_.Exception.Message -match 'compact_stream_identity_mode' }
        Assert-True $refused 'malformed identity mode must refuse rather than select legacy'
    }
    $missingModeRefused = $false
    try { [void](Get-ProductionFoldCheckArguments -Manifest ([pscustomobject]@{}) -RawPath 'raw.bin' -ReportPath 'fold.json') }
    catch { $missingModeRefused = $_.Exception.Message -match 'compact_stream_identity_mode' }
    Assert-True $missingModeRefused 'absent identity mode must refuse rather than select legacy'

    # 1. Full, healthy manifest passes every gate.
    $good = ConvertTo-RoundTrippedManifest (New-TestManifest)
    $r = Test-ProductionCaptureManifest -Manifest $good -MaxFrameTimeUs 100000 `
        -RequireZeroSkipped -RequireZeroDomainGaps -RequireHourlyMet
    Assert-True $r.Passed "healthy manifest should pass; problems: $($r.Problems -join '; ')"

    # 2. Missing p99 on frame_time_us fails the schema gate (fail-closed on absence).
    $h = New-TestManifest
    $h.compact_stream.frame_time_us.Remove("p99") | Out-Null
    $missing = ConvertTo-RoundTrippedManifest $h
    $r = Test-ProductionCaptureManifest -Manifest $missing
    Assert-True (-not $r.Passed) "manifest missing frame_time_us.p99 must fail"
    Assert-True (($r.Problems -join "`n") -match "frame_time_us\.p99 missing") "problem must name the missing p99 field"

    # 3. Missing the whole acceptance_telemetry block fails fail-closed.
    $h2 = New-TestManifest
    $h2.Remove("acceptance_telemetry") | Out-Null
    $missing2 = ConvertTo-RoundTrippedManifest $h2
    $r = Test-ProductionCaptureManifest -Manifest $missing2
    Assert-True (-not $r.Passed) "manifest missing acceptance_telemetry must fail"
    Assert-True (($r.Problems -join "`n") -match "sink_write_us missing") "problem must name missing sink_write_us"
    Assert-True (($r.Problems -join "`n") -match "clock_skew missing") "problem must name missing clock_skew"

    # 4. Missing a clock_skew field fails.
    $h3 = New-TestManifest
    $h3.acceptance_telemetry.clock_skew.Remove("drift_us") | Out-Null
    $r = Test-ProductionCaptureManifest -Manifest (ConvertTo-RoundTrippedManifest $h3)
    Assert-True (-not $r.Passed) "manifest missing clock_skew.drift_us must fail"

    # 5. Frame time over the ceiling fails the numeric gate (schema still complete).
    $slow = ConvertTo-RoundTrippedManifest (New-TestManifest -FrameP95 150000)
    $r = Test-ProductionCaptureManifest -Manifest $slow -MaxFrameTimeUs 100000
    Assert-True (-not $r.Passed) "frame p95 150ms must fail the 100ms ceiling"
    Assert-True (($r.Problems -join "`n") -match "frame_time_us\.p95 = 150000 exceeds max 100000") "problem must name the exceeded ceiling"

    # 6. Non-zero skipped hours or typed domain-gap outcomes fail clean-rung gates.
    $skipped = ConvertTo-RoundTrippedManifest (New-TestManifest -SkippedHours 3)
    $r = Test-ProductionCaptureManifest -Manifest $skipped -RequireZeroSkipped
    Assert-True (-not $r.Passed) "3 skipped hours must fail -RequireZeroSkipped"
    $gapped = ConvertTo-RoundTrippedManifest (New-TestManifest -DomainGapRecordCount 2)
    $r = Test-ProductionCaptureManifest -Manifest $gapped -RequireZeroDomainGaps
    Assert-True (-not $r.Passed) "2 typed domain gaps must fail -RequireZeroDomainGaps"

    # 7. Null percentile is PRESENT (schema ok) but FAILS a numeric threshold.
    $h4 = New-TestManifest
    $h4.compact_stream.frame_time_us.p95 = $null
    $r = Test-ProductionCaptureManifest -Manifest (ConvertTo-RoundTrippedManifest $h4) -MaxFrameTimeUs 100000
    Assert-True (-not $r.Passed) "null p95 must fail the numeric ceiling"
    Assert-True (($r.Problems -join "`n") -match "frame_time_us\.p95 is null") "null p95 must be reported as null, not absent"

    # 8. hourly_requirement.met false fails.
    $notmet = ConvertTo-RoundTrippedManifest (New-TestManifest -HourlyMet $false)
    $r = Test-ProductionCaptureManifest -Manifest $notmet -RequireHourlyMet
    Assert-True (-not $r.Passed) "hourly_requirement.met=false must fail -RequireHourlyMet"

    # 9. VALUE gate: sink_write_us.count=0 with null percentiles must FAIL even
    #    though every field is PRESENT (no data masquerading as good telemetry).
    $h5 = New-TestManifest
    $h5.acceptance_telemetry.sink_write_us.count = 0
    foreach ($pf in @("avg", "p50", "p95", "p99", "max")) { $h5.acceptance_telemetry.sink_write_us.$pf = $null }
    $r = Test-ProductionCaptureManifest -Manifest (ConvertTo-RoundTrippedManifest $h5)
    Assert-True (-not $r.Passed) "sink_write_us.count=0 with null percentiles must fail on values, not pass on presence"
    Assert-True (($r.Problems -join "`n") -match "sink_write_us\.count is 0") "problem must name the zero sink-write count"
    Assert-True (($r.Problems -join "`n") -match "sink_write_us\.p95 is null") "problem must name the null percentile"

    # 10. VALUE gate: clock_skew.wall_monotonic_ok=false (wall clock ran backwards)
    #     must FAIL even with every field present.
    $h6 = New-TestManifest
    $h6.acceptance_telemetry.clock_skew.wall_monotonic_ok = $false
    $r = Test-ProductionCaptureManifest -Manifest (ConvertTo-RoundTrippedManifest $h6)
    Assert-True (-not $r.Passed) "clock_skew.wall_monotonic_ok=false must fail"
    Assert-True (($r.Problems -join "`n") -match "wall_monotonic_ok is not true") "problem must name the broken clock flag"

    # 11. unmeasured clock skew: the recorder emits clock_skew as JSON null for a
    #     deferred repack (no live loop). It must be treated as unmeasured and
    #     FAIL fail-closed, never as a passing zero-drift measurement.
    $h7 = New-TestManifest
    $h7.acceptance_telemetry.clock_skew = $null
    $r = Test-ProductionCaptureManifest -Manifest (ConvertTo-RoundTrippedManifest $h7)
    Assert-True (-not $r.Passed) "null (unmeasured) clock_skew must fail fail-closed"
    Assert-True (($r.Problems -join "`n") -match "clock_skew is null \(unmeasured") "problem must name unmeasured clock skew distinctly"

    # 12. TYPED-VALUE gate (finding 4): a MALFORMED non-null percentile (a string
    #     "not-a-number") with a positive count must FAIL. Fail-before check: the
    #     same manifest with real numbers passes, so the reject is value-driven.
    $baseline = ConvertTo-RoundTrippedManifest (New-TestManifest)
    $rBaseline = Test-ProductionCaptureManifest -Manifest $baseline
    Assert-True $rBaseline.Passed "baseline (real numeric sink stats) must pass so #12 proves the value reject"
    $h8 = New-TestManifest
    $h8.acceptance_telemetry.sink_write_us.count = 1
    foreach ($pf in @("avg", "p50", "p95", "p99", "max")) { $h8.acceptance_telemetry.sink_write_us.$pf = "not-a-number" }
    $r = Test-ProductionCaptureManifest -Manifest (ConvertTo-RoundTrippedManifest $h8)
    Assert-True (-not $r.Passed) "count=1 with non-numeric percentiles must fail on values, not pass on presence"
    Assert-True (($r.Problems -join "`n") -match "sink_write_us\.p95 is not numeric") "problem must name the non-numeric percentile"

    # 13. TYPED-VALUE gate: numeric wall_monotonic_ok=1 (not a real boolean) must
    #     FAIL — PowerShell would coerce 1 -eq $true, so a strict [bool] test is
    #     required. Fail-before: wall_monotonic_ok=$true passes (test 1 baseline).
    $h9 = New-TestManifest
    $h9.acceptance_telemetry.clock_skew.wall_monotonic_ok = 1
    $r = Test-ProductionCaptureManifest -Manifest (ConvertTo-RoundTrippedManifest $h9)
    Assert-True (-not $r.Passed) "numeric wall_monotonic_ok=1 (non-boolean) must fail the strict boolean gate"
    Assert-True (($r.Problems -join "`n") -match "wall_monotonic_ok is not true") "problem must name the non-boolean clock flag"

    # 14. TYPED-VALUE gate: a non-numeric clock_skew drift value must FAIL.
    $h10 = New-TestManifest
    $h10.acceptance_telemetry.clock_skew.drift_us = "NaN"
    $r = Test-ProductionCaptureManifest -Manifest (ConvertTo-RoundTrippedManifest $h10)
    Assert-True (-not $r.Passed) "non-numeric clock_skew.drift_us must fail"
    Assert-True (($r.Problems -join "`n") -match "clock_skew\.drift_us is not numeric") "problem must name the non-numeric drift value"

    # 15. FINITE-VALUE gate (finding 2): PS 5.1 ConvertFrom-Json parses bare JSON
    #     NaN/Infinity/-Infinity as System.Double. A count=1 sink_write_us block with
    #     a non-finite percentile must FAIL — a non-finite value is not a real timing
    #     statistic. Test the predicate directly (bare NaN/Infinity cannot survive a
    #     PowerShell hashtable -> ConvertTo-Json round-trip, so drive Test-IsJsonNumber
    #     with the exact doubles ConvertFrom-Json would produce).
    Assert-True (-not (Test-IsJsonNumber ([double]::NaN))) "Test-IsJsonNumber must reject NaN"
    Assert-True (-not (Test-IsJsonNumber ([double]::PositiveInfinity))) "Test-IsJsonNumber must reject +Infinity"
    Assert-True (-not (Test-IsJsonNumber ([double]::NegativeInfinity))) "Test-IsJsonNumber must reject -Infinity"
    Assert-True (Test-IsJsonNumber ([double]500.0)) "Test-IsJsonNumber must still accept a finite double"
    Assert-True (Test-IsJsonNumber ([int]42)) "Test-IsJsonNumber must still accept a finite int"

    # 16. FINITE-VALUE gate end-to-end: a sink_write_us percentile parsed from bare
    #     JSON NaN/Infinity must fail Get-AcceptanceTelemetryProblems. Fail-before:
    #     the same manifest text with a finite percentile passes.
    $jsonFinite = '{"schema":"native_collector_session_v1","skipped_game_hours":0,"hourly_requirement":{"required":true,"met":true,"problems":[]},"compact_stream":{"frame_time_us":{"count":10,"avg":500,"p50":400,"p95":900,"p99":980,"max":1000},"domain_gap_record_count":0},"acceptance_telemetry":{"schema":"battle-plan.acceptance_telemetry.v1","complete":true,"problems":[],"sink_write_us":{"count":1,"avg":25,"p50":20,"p95":60,"p99":100,"max":120},"clock_skew":{"wall_elapsed_us":300000000,"mono_elapsed_us":300000100,"drift_us":-100,"wall_monotonic_ok":true},"clock_skew_scope":"complete physical H4CR capture; capture-set scoped during rollover"}}'
    $mFinite = $jsonFinite | ConvertFrom-Json
    [string[]]$probsFinite = Get-AcceptanceTelemetryProblems -Manifest $mFinite
    Assert-True ($probsFinite.Count -eq 0) "finite sink_write_us telemetry must pass (fail-before baseline); problems: $($probsFinite -join '; ')"
    $jsonNaN = $jsonFinite -replace '"p95":60', '"p95":NaN'
    $mNaN = $jsonNaN | ConvertFrom-Json
    [string[]]$probsNaN = Get-AcceptanceTelemetryProblems -Manifest $mNaN
    Assert-True (($probsNaN -join "`n") -match "sink_write_us\.p95 is not numeric") "a NaN sink_write_us.p95 must be rejected as non-numeric"
    $jsonInf = $jsonFinite -replace '"max":120', '"max":Infinity'
    $mInf = $jsonInf | ConvertFrom-Json
    [string[]]$probsInf = Get-AcceptanceTelemetryProblems -Manifest $mInf
    Assert-True (($probsInf -join "`n") -match "sink_write_us\.max is not numeric") "an Infinity sink_write_us.max must be rejected as non-numeric"

    # 17. FINDING 2 (frame_time_us presence-only + non-finite ceiling): the frame-time
    #     percentiles were validated for PRESENCE only, and Test-NumericThreshold
    #     compared the value with `-gt` without rejecting non-finite first, so a bare
    #     JSON NaN/Infinity frame_time_us.p95/max returned Passed=True. Prove BOTH
    #     holes closed, with a finite fail-before baseline for each.
    $jsonFrameFinite = '{"schema":"native_collector_session_v1","skipped_game_hours":0,"hourly_requirement":{"required":true,"met":true,"problems":[]},"compact_stream":{"frame_time_us":{"count":10,"avg":500,"p50":400,"p95":900,"p99":980,"max":1000},"domain_gap_record_count":0},"acceptance_telemetry":{"schema":"battle-plan.acceptance_telemetry.v1","complete":true,"problems":[],"sink_write_us":{"count":1,"avg":25,"p50":20,"p95":60,"p99":100,"max":120},"clock_skew":{"wall_elapsed_us":300000000,"mono_elapsed_us":300000100,"drift_us":-100,"wall_monotonic_ok":true},"clock_skew_scope":"complete physical H4CR capture; capture-set scoped during rollover"}}'
    # (a) schema/value gate: bare NaN/Infinity frame percentiles are rejected.
    [string[]]$probsFrameFinite = Get-AcceptanceTelemetryProblems -Manifest ($jsonFrameFinite | ConvertFrom-Json)
    Assert-True ($probsFrameFinite.Count -eq 0) "finite frame_time_us telemetry must pass (fail-before baseline); problems: $($probsFrameFinite -join '; ')"
    $jsonFrameNaN = $jsonFrameFinite -replace '"p95":900', '"p95":NaN'
    [string[]]$probsFrameNaN = Get-AcceptanceTelemetryProblems -Manifest ($jsonFrameNaN | ConvertFrom-Json)
    Assert-True (($probsFrameNaN -join "`n") -match "frame_time_us\.p95 is not numeric") "a NaN frame_time_us.p95 must be rejected as non-numeric (finding 2)"
    $jsonFrameInf = $jsonFrameFinite -replace '"max":1000', '"max":Infinity'
    [string[]]$probsFrameInf = Get-AcceptanceTelemetryProblems -Manifest ($jsonFrameInf | ConvertFrom-Json)
    Assert-True (($probsFrameInf -join "`n") -match "frame_time_us\.max is not numeric") "an Infinity frame_time_us.max must be rejected as non-numeric (finding 2)"
    # (b) end-to-end: Test-ProductionCaptureManifest with -MaxFrameTimeUs. OLD code
    #     returned Passed=True on a NaN p95 (NaN -gt Max is false); it must now FAIL.
    $rFrameFinite = Test-ProductionCaptureManifest -Manifest ($jsonFrameFinite | ConvertFrom-Json) -MaxFrameTimeUs 100000 -RequireZeroSkipped -RequireZeroDomainGaps -RequireHourlyMet
    Assert-True $rFrameFinite.Passed "finite frame manifest must pass -MaxFrameTimeUs (fail-before); problems: $($rFrameFinite.Problems -join '; ')"
    $rFrameNaN = Test-ProductionCaptureManifest -Manifest ($jsonFrameNaN | ConvertFrom-Json) -MaxFrameTimeUs 100000 -RequireZeroSkipped -RequireZeroDomainGaps -RequireHourlyMet
    Assert-True (-not $rFrameNaN.Passed) "a NaN frame_time_us.p95 must FAIL Test-ProductionCaptureManifest, not pass on NaN -gt Max being false (finding 2)"
    # (c) Test-NumericThreshold rejects a non-finite value BEFORE the ceiling compare.
    $ntNaN = [System.Collections.Generic.List[string]]::new()
    Test-NumericThreshold -Manifest ('{"compact_stream":{"frame_time_us":{"p95":NaN}}}' | ConvertFrom-Json) -Path "compact_stream.frame_time_us.p95" -Max 100000 -Problems $ntNaN
    Assert-True (($ntNaN -join "`n") -match "not a finite number") "Test-NumericThreshold must reject NaN before comparing (finding 2)"
    $ntInf = [System.Collections.Generic.List[string]]::new()
    Test-NumericThreshold -Manifest ('{"compact_stream":{"frame_time_us":{"p95":Infinity}}}' | ConvertFrom-Json) -Path "compact_stream.frame_time_us.p95" -Max 100000 -Problems $ntInf
    Assert-True (($ntInf -join "`n") -match "not a finite number") "Test-NumericThreshold must reject Infinity before comparing (finding 2)"

    # 18. RE-GATE2 FINDING 3: frame_time_us.count must be a genuine positive finite
    #     JSON integer. A string count passed the reusable gate (presence-only) and
    #     the heavyweight [int64] coercion; a bare JSON NaN count also passed the
    #     reusable gate. Fail-before baseline: integer count 10 passes (test 1).
    $hCountStr = New-TestManifest
    $hCountStr.compact_stream.frame_time_us.count = "10"
    $r = Test-ProductionCaptureManifest -Manifest (ConvertTo-RoundTrippedManifest $hCountStr)
    Assert-True (-not $r.Passed) "a string frame_time_us.count must fail (was presence-only; re-gate2 finding 3)"
    Assert-True (($r.Problems -join "`n") -match "frame_time_us\.count is not an integer") "problem must name the non-integer frame-time count"
    $hCountZero = New-TestManifest
    $hCountZero.compact_stream.frame_time_us.count = 0
    $r = Test-ProductionCaptureManifest -Manifest (ConvertTo-RoundTrippedManifest $hCountZero)
    Assert-True (-not $r.Passed) "frame_time_us.count=0 must fail (no frame-time samples)"
    Assert-True (($r.Problems -join "`n") -match "frame_time_us\.count is 0") "problem must name the zero frame-time count"
    $jsonCountNaN = $jsonFrameFinite -replace '"frame_time_us":\{"count":10', '"frame_time_us":{"count":NaN'
    [string[]]$probsCountNaN = Get-AcceptanceTelemetryProblems -Manifest ($jsonCountNaN | ConvertFrom-Json)
    Assert-True (($probsCountNaN -join "`n") -match "frame_time_us\.count is not an integer") "a bare JSON NaN frame_time_us.count must be rejected (re-gate2 finding 3)"
    Assert-True (-not (Test-IsJsonInteger ([double]::NaN))) "Test-IsJsonInteger must reject NaN"
    Assert-True (-not (Test-IsJsonInteger ([double]::PositiveInfinity))) "Test-IsJsonInteger must reject +Infinity"
    #     re-gate6: a [double] is already rounded at JSON parse time, so it can never
    #     be trusted for lossless integer validation — reject ALL doubles (a legitimate
    #     count is emitted as a plain JSON integer, never a float). This closes the
    #     exponent-form hole: 9007199254740993e0 and 9007199254740992.5e0 both parse to
    #     the SAME rounded double, so neither may be accepted.
    Assert-True (-not (Test-IsJsonInteger ([double]10.0))) "Test-IsJsonInteger must reject a whole-valued double (floats are lossy; counts are integer tokens)"
    Assert-True (-not (Test-IsJsonInteger (('{"c":9007199254740992.5e0}' | ConvertFrom-Json).c))) "Test-IsJsonInteger must reject an exponent-form fractional value (parsed as a rounded double)"
    Assert-True (-not (Test-IsJsonInteger (('{"c":9007199254740993e0}' | ConvertFrom-Json).c))) "Test-IsJsonInteger must reject an exponent-form whole value that a double cannot represent exactly"
    #     re-gate3: a [decimal] with a fractional part above 2^53 lost its .5 when
    #     coerced through [double]; decide integrality in decimal space instead.
    Assert-True (-not (Test-IsJsonInteger 9007199254740992.5d)) "Test-IsJsonInteger must reject a non-integral decimal above 2^53 (decimal-space truncation, not double coercion)"
    Assert-True (Test-IsJsonInteger 9007199254740992d) "Test-IsJsonInteger must accept a whole decimal in range"
    #     re-gate3: JSON 9223372036854775808 (2^63) parses as [uint64] in PS 5.1;
    #     it must fail closed here so the downstream [int64] cast never overflow-throws.
    Assert-True (-not (Test-IsJsonInteger ([uint64]9223372036854775808))) "Test-IsJsonInteger must reject a uint64 above Int64.MaxValue (overflow-safe downstream cast)"
    Assert-True (Test-IsJsonInteger ([int64]9223372036854775807)) "Test-IsJsonInteger must accept Int64.MaxValue itself"
    #     End-to-end: both flagged values in a real manifest must FAIL the gate,
    #     not pass and not throw an unhandled Int64 overflow.
    $jsonDecCount = '{"schema_version":1,"compact_stream":{"frame_time_us":{"count":9007199254740992.5,"avg":1,"p50":1,"p95":1,"p99":1,"max":1}}}'
    $probsDec = Get-AcceptanceTelemetryProblems -Manifest ($jsonDecCount | ConvertFrom-Json)
    Assert-True (($probsDec -join "`n") -match "frame_time_us\.count is not an integer") "a fractional decimal frame count above 2^53 must be rejected, not silently truncated"
    $jsonOverCount = '{"schema_version":1,"compact_stream":{"frame_time_us":{"count":9223372036854775808,"avg":1,"p50":1,"p95":1,"p99":1,"max":1}}}'
    $probsOver = $null
    $threw = $false
    try { $probsOver = Get-AcceptanceTelemetryProblems -Manifest ($jsonOverCount | ConvertFrom-Json) } catch { $threw = $true }
    Assert-True (-not $threw) "an out-of-Int64-range frame count must return a typed problem, never throw an overflow"
    Assert-True (($probsOver -join "`n") -match "frame_time_us\.count is not an integer") "a frame count above Int64.MaxValue must be rejected as not-an-integer"
    #     Heavyweight path: Test-ProductionCaptureArtifacts previously coerced the
    #     count through [int64] (string "10" -eq frame count 10 after coercion). It
    #     must now emit a typed problem instead.
    $selfDir = Split-Path -Parent $PSCommandPath
    $realRegistry = Join-Path (Split-Path -Parent $selfDir) "domain_registry.json"
    Assert-True (Test-Path -LiteralPath $realRegistry -PathType Leaf) "domain_registry.json must exist for the heavyweight count/identity type checks ($realRegistry)"
    if (Test-Path -LiteralPath $realRegistry -PathType Leaf) {
        $hwTmp = Join-Path ([System.IO.Path]::GetTempPath()) ("pcv_selftest_hw_" + [guid]::NewGuid().ToString("N"))
        New-Item -ItemType Directory -Path $hwTmp | Out-Null
        try {
            $hwManifest = New-TestManifest
            $hwManifest.compact_stream["frame_end_record_count"] = 10
            $hwManifest.compact_stream.frame_time_us.count = "10"
            $hwManifest | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $hwTmp "native_session_manifest.json") -Encoding UTF8
            $hwReport = Test-ProductionCaptureArtifacts -RunDir $hwTmp -RegistryPath $realRegistry -SpeedProofPath (Join-Path $hwTmp "speed5_proof.json")
            Assert-True (@($hwReport.problems) -contains "frame_time_count_not_positive_integer") "heavyweight gate must reject a string frame_time_us.count with a typed problem, not coerce it through [int64] (re-gate2 finding 3)"
            Assert-True ($hwReport.ok -eq $false) "the heavyweight verdict over the string-count manifest must be ok=false"
            # Fail-before contrast: a proper integer count does NOT emit the typed
            # count problem (the run still fails for other reasons in this bare dir).
            $hwManifestOk = New-TestManifest
            $hwManifestOk.compact_stream["frame_end_record_count"] = 10
            $hwManifestOk | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $hwTmp "native_session_manifest.json") -Encoding UTF8
            $hwReportOk = Test-ProductionCaptureArtifacts -RunDir $hwTmp -RegistryPath $realRegistry -SpeedProofPath (Join-Path $hwTmp "speed5_proof.json")
            Assert-True (-not (@($hwReportOk.problems) -contains "frame_time_count_not_positive_integer")) "an integer frame_time_us.count must not trip the typed count problem"
        } finally { Remove-Item -LiteralPath $hwTmp -Recurse -Force -ErrorAction SilentlyContinue }
    }

    # 19. RE-GATE2 FINDING 4: heavyweight identity fields must be genuine JSON
    #     strings. plan {"capture_id":123,"session_id":true} + manifest
    #     {"session_id":true} previously string-coerced ("123"/"True") and were
    #     adopted as the attempt identity.
    if (Test-Path -LiteralPath $realRegistry -PathType Leaf) {
        $idTmp = Join-Path ([System.IO.Path]::GetTempPath()) ("pcv_selftest_id_" + [guid]::NewGuid().ToString("N"))
        New-Item -ItemType Directory -Path $idTmp | Out-Null
        try {
            $idManifest = New-TestManifest
            $idManifest["session_id"] = $true
            $idManifest | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $idTmp "native_session_manifest.json") -Encoding UTF8
            $idPlanPath = Join-Path $idTmp "production_capture_plan.json"
            ([ordered]@{ schema = "battle-plan.production_capture_plan.v1"; capture_id = 123; session_id = $true } | ConvertTo-Json -Depth 6) | Set-Content -LiteralPath $idPlanPath -Encoding UTF8
            $idReport = Test-ProductionCaptureArtifacts -RunDir $idTmp -RegistryPath $realRegistry -SpeedProofPath (Join-Path $idTmp "speed5_proof.json") -CapturePlanPath $idPlanPath -SourceInventoryPath (Join-Path $idTmp "src_inv.json") -LiveResolutionReportPath (Join-Path $idTmp "live_res.json")
            Assert-True (@($idReport.problems) -contains "capture_id_not_a_json_string") "a numeric plan capture_id must be rejected as not-a-string, never coerced (re-gate2 finding 4)"
            Assert-True (@($idReport.problems) -contains "session_id_not_a_json_string") "a boolean manifest session_id must be rejected as not-a-string, never coerced (re-gate2 finding 4)"
            Assert-True (@($idReport.problems) -contains "plan_session_id_not_a_json_string") "a boolean plan session_id must be rejected as not-a-string"
            Assert-True ([string]$idReport.capture_id -eq "") "the coerced numeric capture_id must NOT be adopted into the report identity"
            # Fail-before contrast: genuine string identities emit NONE of the typed
            # identity problems (other problems remain — bare dir).
            $idManifest2 = New-TestManifest
            $idManifest2["session_id"] = "sess-x"
            $idManifest2 | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $idTmp "native_session_manifest.json") -Encoding UTF8
            ([ordered]@{ schema = "battle-plan.production_capture_plan.v1"; capture_id = "cap-x"; session_id = "sess-x" } | ConvertTo-Json -Depth 6) | Set-Content -LiteralPath $idPlanPath -Encoding UTF8
            $idReport2 = Test-ProductionCaptureArtifacts -RunDir $idTmp -RegistryPath $realRegistry -SpeedProofPath (Join-Path $idTmp "speed5_proof.json") -CapturePlanPath $idPlanPath -SourceInventoryPath (Join-Path $idTmp "src_inv.json") -LiveResolutionReportPath (Join-Path $idTmp "live_res.json")
            foreach ($tp in @("capture_id_not_a_json_string", "session_id_not_a_json_string", "plan_session_id_not_a_json_string", "capture_plan_session_id_mismatch")) {
                Assert-True (-not (@($idReport2.problems) -contains $tp)) "genuine string identities must not trip '$tp'"
            }
        } finally { Remove-Item -LiteralPath $idTmp -Recurse -Force -ErrorAction SilentlyContinue }
    }

    # 20. The consumer must honor the producer's complete/problems verdict; a
    # structurally populated block cannot override a reported producer gap.
    $incomplete = New-TestManifest
    $incomplete.acceptance_telemetry.complete = $false
    $incomplete.acceptance_telemetry.problems = @("acceptance_exact_roster_mismatch")
    $rIncomplete = Test-ProductionCaptureManifest -Manifest (ConvertTo-RoundTrippedManifest $incomplete)
    Assert-True (-not $rIncomplete.Passed) "acceptance_telemetry.complete=false must fail"
    Assert-True (($rIncomplete.Problems -join "`n") -match "complete is not boolean true") "consumer must report the false producer verdict"
    Assert-True (($rIncomplete.Problems -join "`n") -match "problems is not empty") "consumer must report producer problems"
    $nullAcceptance = New-TestManifest
    $nullAcceptance.acceptance_telemetry = $null
    $nullThrew = $false
    $rNullAcceptance = $null
    try { $rNullAcceptance = Test-ProductionCaptureManifest -Manifest (ConvertTo-RoundTrippedManifest $nullAcceptance) } catch { $nullThrew = $true }
    Assert-True (-not $nullThrew) "null acceptance_telemetry must return a failed verdict, never throw"
    Assert-True (-not $rNullAcceptance.Passed) "null acceptance_telemetry must fail closed"

    if ($failures.Count -gt 0) {
        Write-Host "ProductionCapture.Validation self-test FAILED:" -ForegroundColor Red
        foreach ($f in $failures) { Write-Host "  - $f" -ForegroundColor Red }
        return $false
    }
    Write-Host "ProductionCapture.Validation self-test: all checks passed."
    return $true
}

# Only run self-tests when invoked directly (not when dot-sourced for reuse).
if ($SelfTest) {
    if (Invoke-ValidationSelfTest) { exit 0 } else { exit 1 }
}
