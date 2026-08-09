<#
.SYNOPSIS
Hold the PowerShell gap-ledger expander to the same vector Rust and Python are.

.DESCRIPTION
The sealed manifest's gap ledger is read by THREE implementations: the
recorder's own (`recorder_host/src/gap_ledger.rs`), the offline Python one
(`scripts/gap_ledger.py`), and `Expand-CaptureGapOutcomes` in
`ProductionCapture.Validation.ps1`, which the production capture validator walks
its rows through. Three expanders written from one description agree until the
first time one of them learns something the others did not, and the failure mode
is a capture promoted on rows nobody produced.

`gap_ledger_conformance.json` is the one vector all three are held to. The Rust
test owns its content; `test_gap_ledger.py` proves the Python side; this proves
the PowerShell side.

Prints one JSON verdict line and exits nonzero on any failure.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir 'ProductionCapture.Validation.ps1')

$failures = New-Object System.Collections.ArrayList
$cases = 0

function Assert-GapLedger {
    param([Parameter(Mandatory = $true)][bool]$Condition,
          [Parameter(Mandatory = $true)][string]$Because)
    $script:cases++
    if (-not $Condition) { [void]$script:failures.Add($Because) }
}

function Get-VectorRowCount {
    <#
    .SYNOPSIS
    How many rows a ledger's outcome classes expand to, SKIPPING the first
    `$SkipFirst` of them.

    .DESCRIPTION
    A mutation that replaces class 0's `hour_runs` changes how many rows the
    table expands to. Repairing `row_count` to the new truth is what stops the
    count check firing first and masking the guard actually under test.
    #>
    param([Parameter(Mandatory = $true)]$Ledger,
          [Parameter(Mandatory = $true)][int]$SkipFirst)
    $total = 0
    $classes = @($Ledger.domain_gap_outcomes.classes)
    for ($i = $SkipFirst; $i -lt $classes.Count; $i++) {
        $hours = 0
        foreach ($run in @($classes[$i].hour_runs)) { $hours += [int]$run[1] }
        $total += $hours * @($classes[$i].domains).Count
    }
    return $total
}

function Assert-GapLedgerRefusal {
    <#
    .SYNOPSIS
    A ledger must be refused FOR THE STATED REASON.

    .DESCRIPTION
    Asserting only `-not $result.Ok` is not a test of the guard under test. Every
    mutation below changes how many rows a class expands to, so `row_count` stops
    matching and the expander refuses with `gap_ledger_row_count_mismatch` even
    with the intended guard removed. Naming the expected problem is what makes
    each case fail when its own guard is deleted.
    #>
    param([Parameter(Mandatory = $true)]$Ledger,
          [Parameter(Mandatory = $true)][string]$Problem,
          [Parameter(Mandatory = $true)][string]$Because)
    $result = Expand-CaptureGapOutcomes ([pscustomobject]@{ gap_ledger = $Ledger })
    $got = if ($null -eq $result) { '<null>' } else { [string]$result.Problem }
    Assert-GapLedger -Condition ($null -ne $result -and -not $result.Ok -and $got -ceq $Problem) `
        -Because "$Because (expected problem '$Problem', got '$got')"
}

$vector = Get-Content -LiteralPath (Join-Path $scriptDir 'gap_ledger_conformance.json') -Raw |
    ConvertFrom-Json
$compact = [pscustomobject]@{ gap_ledger = $vector.gap_ledger }

# ---------------------------------------------------------------------------
# 1. The vector's rows come back, ordinals and all.
# ---------------------------------------------------------------------------
$expansion = Expand-CaptureGapOutcomes $compact
Assert-GapLedger -Condition $expansion.Ok -Because 'the conformance vector must expand'
$expected = @($vector.expected_domain_gap_outcomes |
    Sort-Object -Property target_hour_raw, domain_id)
$actual = @($expansion.Rows | Sort-Object -Property target_hour_raw, domain_id)
Assert-GapLedger -Condition ($actual.Count -eq $expected.Count) `
    -Because "expanded $($actual.Count) rows, the vector says $($expected.Count)"
Assert-GapLedger -Condition ($expected.Count -gt 50) `
    -Because 'a vector this small would make every comparison below vacuous'

$mismatch = $null
for ($i = 0; $i -lt [Math]::Min($actual.Count, $expected.Count); $i++) {
    foreach ($field in @('domain_id', 'part', 'reason', 'target_hour_raw',
                         'range_start_hour_raw', 'range_end_hour_raw',
                         'tick_before', 'tick_after', 'queue_capacity',
                         'queue_depth', 'queue_dropped', 'outcome_ordinal')) {
        $got = [string]$actual[$i].$field
        $want = [string]$expected[$i].$field
        if ($got -cne $want) {
            $mismatch = "row $i field $field expanded to '$got', the vector says '$want'"
            break
        }
    }
    if ($mismatch) { break }
}
Assert-GapLedger -Condition ($null -eq $mismatch) `
    -Because "row-for-row comparison against the vector: $mismatch"

# ---------------------------------------------------------------------------
# 2. A `u64` tick above Int64::MaxValue must not kill the validator.
#    `[int64]` casts on these fields were a TERMINATING error, so a legal ledger
#    ended the run instead of being graded.
# ---------------------------------------------------------------------------
$wide = ConvertFrom-Json @'
{"gap_ledger":{"schema":"hoi4-gap-ledger-v2","version":2,
 "gaps":{"row_count":0,"class_count":0,"classes":[]},
 "domain_gap_outcomes":{"row_count":1,"class_count":1,
  "ordinal_encoding":"explicit",
  "classes":[{"reason":"torn","part":"state","domains":[4],
    "queue_capacity":0,"queue_depth":0,"queue_dropped":18446744073709551615,
    "range_start_rel":0,"range_end_rel":0,
    "tick_before":{"abs":18446744073709551615},
    "tick_after":{"abs":18446744073709551614},
    "hour_runs":[[100,1]],"ordinals":[0]}]}}}
'@
$wideExpansion = Expand-CaptureGapOutcomes $wide
Assert-GapLedger -Condition ($wideExpansion.Ok -and $wideExpansion.Rows.Count -eq 1) `
    -Because 'a u64 tick above Int64::MaxValue must expand, not terminate the validator'
# And the VALUES must survive. Asserting only success let a reader that replaced
# every wide field with zero pass -- which is the failure this case exists for.
$wideRow = @($wideExpansion.Rows)[0]
Assert-GapLedger -Condition (
    [string]$wideRow.tick_before -ceq '18446744073709551615' -and
    [string]$wideRow.tick_after -ceq '18446744073709551614' -and
    [string]$wideRow.queue_dropped -ceq '18446744073709551615') `
    -Because ("a u64 tick must survive its expansion, not be zeroed " +
              "(got $($wideRow.tick_before)/$($wideRow.tick_after)/$($wideRow.queue_dropped))")

# A NEGATIVE offset is what the recorder writes wherever a tick precedes its
# hour. Refusing it would refuse a document the producer routinely seals.
$negativeOffset = ConvertFrom-Json @'
{"gap_ledger":{"schema":"hoi4-gap-ledger-v2","version":2,
 "gaps":{"row_count":0,"class_count":0,"classes":[]},
 "domain_gap_outcomes":{"row_count":1,"class_count":1,
  "ordinal_encoding":"explicit",
  "classes":[{"reason":"torn","part":"state","domains":[4],
    "queue_capacity":0,"queue_depth":0,"queue_dropped":0,
    "range_start_rel":0,"range_end_rel":0,
    "tick_before":{"rel":-1},"tick_after":{"rel":0},
    "hour_runs":[[100,1]],"ordinals":[0]}]}}}
'@
$negativeResult = Expand-CaptureGapOutcomes $negativeOffset
Assert-GapLedger -Condition (
    $negativeResult.Ok -and [string](@($negativeResult.Rows)[0].tick_before) -ceq '99') `
    -Because 'a negative tick OFFSET is legal and must resolve, not be refused as unsigned'

# ---------------------------------------------------------------------------
# 3. Refusals: missing ledger, wrong version, a version that is only a string,
#    a row_count that is not an integer, a truncated class list.
# ---------------------------------------------------------------------------
Assert-GapLedger -Condition (-not (Expand-CaptureGapOutcomes ([pscustomobject]@{})).Ok) `
    -Because 'a manifest with no gap_ledger must be refused, not read as zero gaps'

# ---------------------------------------------------------------------------
# 3a. THE SHAPE A PROMOTABLE CAPTURE HAS. Every refusal below tightens what this
#     expander accepts, and the one document it must never refuse is the empty
#     ledger a zero-loss capture seals: empty `classes`, empty
#     `lane_ordinal_first`, `row_count` 0. A presence check written as an
#     array-ness or truthiness test breaks exactly this.
# ---------------------------------------------------------------------------
$zeroLoss = ConvertFrom-Json @'
{"gap_ledger":{"schema":"hoi4-gap-ledger-v2","version":2,
 "gaps":{"row_count":0,"class_count":0,"classes":[]},
 "domain_gap_outcomes":{"row_count":0,"class_count":0,
  "ordinal_encoding":"lane_sequential","lane_ordinal_first":[],"classes":[]}}}
'@
$zeroLossResult = Expand-CaptureGapOutcomes $zeroLoss
Assert-GapLedger -Condition ($zeroLossResult.Ok -and @($zeroLossResult.Rows).Count -eq 0) `
    -Because "a zero-loss capture's empty ledger must expand cleanly to no rows"

function Copy-Vector { ConvertFrom-Json ($vector.gap_ledger | ConvertTo-Json -Depth 20) }

$badVersion = Copy-Vector
$badVersion.version = 3
Assert-GapLedgerRefusal $badVersion 'gap_ledger_version_unreadable' `
    'an unknown version must be refused'

$stringVersion = Copy-Vector
$stringVersion.version = '2'
Assert-GapLedgerRefusal $stringVersion 'gap_ledger_version_unreadable' `
    'a version the recorder cannot write must be refused'

$badCount = Copy-Vector
$badCount.domain_gap_outcomes.row_count = '180'
Assert-GapLedgerRefusal $badCount 'gap_ledger_row_count_invalid' `
    'a row_count that is not an integer must be refused, not skipped'

$truncated = Copy-Vector
$truncated.domain_gap_outcomes.classes = @($truncated.domain_gap_outcomes.classes[0])
# `class_count` REPAIRED to the truncated list, so the refusal can only come from
# the ROW count and not from the class count noticing first.
$truncated.domain_gap_outcomes.class_count = 1
Assert-GapLedgerRefusal $truncated 'gap_ledger_row_count_mismatch' `
    'a truncated class list must be caught by the declared row_count'

# An hour outside the i32 range names an hour that cannot exist and that the
# recorder's own reader refuses. Accepting it here would be two readers of one
# document disagreeing about which hour a gap belongs to.
foreach ($run in @(@(2147483648, 1), @(2147483647, 4), @(-2147483649, 1))) {
    $outOfRange = Copy-Vector
    $outOfRange.domain_gap_outcomes.classes[0].hour_runs = @(, $run)
    # `row_count` REPAIRED to what this mutated ledger really expands to, so the
    # refusal below can only come from the hour bound and not from a count that
    # stopped matching as a side effect.
    $outOfRange.domain_gap_outcomes.row_count =
        $run[1] * @($outOfRange.domain_gap_outcomes.classes[0].domains).Count +
        (Get-VectorRowCount $outOfRange 1)
    Assert-GapLedgerRefusal $outOfRange 'gap_ledger_hour_out_of_range' `
        "an hour run at $($run[0]) for $($run[1]) must be refused as outside i32"
}

# Nothing in the expansion may TERMINATE the validator. A class missing a field
# it indexes, or a run pair with the wrong arity, is a PowerShell terminating
# error, and a validator that dies mid-manifest publishes no verdict at all.
$missingField = Copy-Vector
$missingField.domain_gap_outcomes.classes[0].PSObject.Properties.Remove('range_start_rel')
Assert-GapLedgerRefusal $missingField 'gap_ledger_unreadable' `
    'a class missing a field must be reported, not terminate the validator'

$badRun = Copy-Vector
$badRun.domain_gap_outcomes.classes[0].hour_runs = @(, @(1, 2, 3))
Assert-GapLedgerRefusal $badRun 'gap_ledger_hour_run_malformed' `
    'a malformed hour run must be reported, not terminate the validator'

# `[int64]1.5` is 2 in PowerShell, so a float run element expanded silently at a
# DIFFERENT hour than the one written, while Rust and Python refuse it.
$floatRun = Copy-Vector
$floatRun.domain_gap_outcomes.classes[0].hour_runs = @(, @(1.5, 1))
Assert-GapLedgerRefusal $floatRun 'gap_ledger_hour_run_not_integral' `
    'a float hour run must be refused, not rounded to a different hour'

# A run at the top of the hour space plus a positive range offset resolves to an
# hour that cannot exist. Rust and Python refuse it; admitting it here would be
# two readers of one document disagreeing about which hour a gap belongs to.
$overflowRange = Copy-Vector
$overflowRange.domain_gap_outcomes.classes[0].hour_runs = @(, @(2147483647, 1))
$overflowRange.domain_gap_outcomes.classes[0].range_end_rel = 1
$overflowRange.domain_gap_outcomes.row_count =
    @($overflowRange.domain_gap_outcomes.classes[0].domains).Count +
    (Get-VectorRowCount $overflowRange 1)
Assert-GapLedgerRefusal $overflowRange 'gap_ledger_unreadable' `
    'a range hour resolving past i32::MAX must be refused'

# A table with no `classes` at all is malformed, not a capture with no gaps.
$noClasses = Copy-Vector
$noClasses.domain_gap_outcomes.PSObject.Properties.Remove('classes')
$noClasses.domain_gap_outcomes.row_count = 0
Assert-GapLedgerRefusal $noClasses 'gap_ledger_classes_missing' `
    'a table with no classes must be refused, not read as zero rows'

# A lane base that is absent must NOT default to zero: `[bigint]$null` is 0, and
# an invented base numbers a whole lane with ordinals nobody wrote.
# A scalar where a list belongs. `@(...)` normalizes it into a one-element list,
# so `"domains": 4` expanded here while Rust and Python refuse it.
$scalarDomains = Copy-Vector
$scalarDomains.domain_gap_outcomes.classes[0].domains = 4
Assert-GapLedgerRefusal $scalarDomains 'gap_ledger_domains_invalid' `
    'a scalar where the domain list belongs must be refused, not normalized'

# A class that names no domain expands to nothing, so any number of them would
# mean what the empty table means.
$noDomains = Copy-Vector
$noDomains.domain_gap_outcomes.classes[0].domains = @()
$noDomains.domain_gap_outcomes.row_count = Get-VectorRowCount $noDomains 1
Assert-GapLedgerRefusal $noDomains 'gap_ledger_domains_invalid' `
    'a class naming no domain must be refused, not expanded to nothing'

# The contract's counts, ticks and ordinals are unsigned.
$negativeQueue = Copy-Vector
$negativeQueue.domain_gap_outcomes.classes[0].queue_depth = -1
Assert-GapLedgerRefusal $negativeQueue 'gap_ledger_unreadable' `
    'a negative queue witness must be refused'

$negativeBase = Copy-Vector
$negativeBase.domain_gap_outcomes.lane_ordinal_first[0].first = -1
Assert-GapLedgerRefusal $negativeBase 'gap_ledger_unreadable' `
    'a negative lane ordinal base must be refused'

# A null entry must be REFUSED, not skipped: skipping drops a real class or a
# lane's base silently, and the other two readers reject it.
$nullClass = Copy-Vector
$nullClass.domain_gap_outcomes.classes = @($null) + @($nullClass.domain_gap_outcomes.classes)
$nullClass.domain_gap_outcomes.class_count = @($nullClass.domain_gap_outcomes.classes).Count
Assert-GapLedgerRefusal $nullClass 'gap_ledger_class_null' `
    'a null class must be refused, not skipped'

# A lane entry nobody expands to, and a lane named twice, both leave the same
# expansion -- so a table carrying either means what the table without it means.
# `class_count` is published beside the class list and must agree with it.
$badClassCount = Copy-Vector
$badClassCount.domain_gap_outcomes.class_count = 999
Assert-GapLedgerRefusal $badClassCount 'gap_ledger_class_count_mismatch' `
    'a class_count that disagrees with the list must be refused'

# `[[101,1],[102,2]]` expands to exactly what `[[101,3]]` does, so admitting both
# lets two documents mean one thing.
# `[[101,1],[102,2]]` expands to exactly what `[[101,3]]` does, so admitting both
# lets two documents mean one thing. Written as a literal rather than mutated out
# of the vector, because PowerShell's array handling makes rebuilding a jagged
# run list in place its own source of bugs -- and the two ledgers below differ in
# nothing but the run encoding.
$canonicalRuns = ConvertFrom-Json @'
{"gap_ledger":{"schema":"hoi4-gap-ledger-v2","version":2,
 "gaps":{"row_count":0,"class_count":0,"classes":[]},
 "domain_gap_outcomes":{"row_count":3,"class_count":1,
  "ordinal_encoding":"explicit",
  "classes":[{"reason":"torn","part":"state","domains":[4],
    "queue_capacity":0,"queue_depth":0,"queue_dropped":0,
    "range_start_rel":0,"range_end_rel":0,
    "tick_before":{"rel":0},"tick_after":{"rel":0},
    "hour_runs":[[101,3]],"ordinals":[0,1,2]}]}}}
'@
$canonicalResult = Expand-CaptureGapOutcomes $canonicalRuns
Assert-GapLedger -Condition ($canonicalResult.Ok -and @($canonicalResult.Rows).Count -eq 3) `
    -Because 'a maximal run must expand'

$splitRuns = ConvertFrom-Json @'
{"gap_ledger":{"schema":"hoi4-gap-ledger-v2","version":2,
 "gaps":{"row_count":0,"class_count":0,"classes":[]},
 "domain_gap_outcomes":{"row_count":3,"class_count":1,
  "ordinal_encoding":"explicit",
  "classes":[{"reason":"torn","part":"state","domains":[4],
    "queue_capacity":0,"queue_depth":0,"queue_dropped":0,
    "range_start_rel":0,"range_end_rel":0,
    "tick_before":{"rel":0},"tick_after":{"rel":0},
    "hour_runs":[[101,1],[102,2]],"ordinals":[0,1,2]}]}}}
'@
Assert-GapLedgerRefusal $splitRuns.gap_ledger 'gap_ledger_hour_runs_not_canonical' `
    'a split run must be refused; runs must be maximal'

# A one-element array is not the scalar PowerShell unwraps it into.
# Written as JSON literals: assigning `@(2)` onto an existing scalar member
# coerces it back, which would make these cases test nothing.
$arrayVersion = ConvertFrom-Json @'
{"schema":"hoi4-gap-ledger-v2","version":[2],
 "gaps":{"row_count":0,"class_count":0,"classes":[]},
 "domain_gap_outcomes":{"row_count":0,"class_count":0,
  "ordinal_encoding":"lane_sequential","lane_ordinal_first":[],"classes":[]}}
'@
Assert-GapLedgerRefusal $arrayVersion 'gap_ledger_version_unreadable' `
    'a one-element array where a scalar belongs must be refused, not unwrapped'

$arrayReason = ConvertFrom-Json @'
{"schema":"hoi4-gap-ledger-v2","version":2,
 "gaps":{"row_count":0,"class_count":0,"classes":[]},
 "domain_gap_outcomes":{"row_count":1,"class_count":1,
  "ordinal_encoding":"explicit",
  "classes":[{"reason":["torn"],"part":"state","domains":[4],
    "queue_capacity":0,"queue_depth":0,"queue_dropped":0,
    "range_start_rel":0,"range_end_rel":0,
    "tick_before":{"rel":0},"tick_after":{"rel":0},
    "hour_runs":[[100,1]],"ordinals":[0]}]}}
'@
Assert-GapLedgerRefusal $arrayReason 'gap_ledger_unreadable' `
    'a one-element array where a name belongs must be refused'

$unusedLane = Copy-Vector
$unusedLane.domain_gap_outcomes.lane_ordinal_first =
    @($unusedLane.domain_gap_outcomes.lane_ordinal_first) +
    @([pscustomobject]@{ domain_id = 999; part = 'both'; first = 0 })
Assert-GapLedgerRefusal $unusedLane 'gap_ledger_lane_unused' `
    'a lane entry no class expands to must be refused'

$duplicateLane = Copy-Vector
$duplicateLane.domain_gap_outcomes.lane_ordinal_first =
    @($duplicateLane.domain_gap_outcomes.lane_ordinal_first) +
    @($duplicateLane.domain_gap_outcomes.lane_ordinal_first[0])
Assert-GapLedgerRefusal $duplicateLane 'gap_ledger_lane_duplicate' `
    'a lane named twice must be refused, not silently overwritten'

$laneDomain = Copy-Vector
$laneDomain.domain_gap_outcomes.lane_ordinal_first[0].domain_id = 65536
Assert-GapLedgerRefusal $laneDomain 'gap_ledger_lane_domain_invalid' `
    'a lane domain outside u16 must be refused'

# `2` parses as Int and `2.0` as Decimal in PowerShell 5.1, so a Decimal below
# Int64 means the document wrote a FLOAT -- which the recorder never does and
# which Rust and Python refuse.
$floatVersion = Copy-Vector
$floatVersion.version = 2.0
Assert-GapLedgerRefusal $floatVersion 'gap_ledger_version_unreadable' `
    'a float-form version must be refused, not read as the integer it rounds to'

$floatDomain = Copy-Vector
$floatDomain.domain_gap_outcomes.classes[0].domains = @(4.0)
Assert-GapLedgerRefusal $floatDomain 'gap_ledger_domain_invalid' `
    'a float-form domain id must be refused'

# `queue_dropped` is a u64. Above the type is a figure the producer cannot write.
$wideDropped = Copy-Vector
$wideDropped.domain_gap_outcomes.classes[0].queue_dropped =
    [decimal]::Parse('18446744073709551616')
Assert-GapLedgerRefusal $wideDropped 'gap_ledger_unreadable' `
    'a queue_dropped above u64::MAX must be refused'

# The wire names are lowercase. `-notcontains` is case-INsensitive, so an
# uppercase spelling was admitted and republished.
$upperReason = Copy-Vector
$upperReason.domain_gap_outcomes.classes[0].reason = 'TORN'
Assert-GapLedgerRefusal $upperReason 'gap_ledger_reason_unknown' `
    'an uppercase reason must be refused, not matched case-insensitively'

$upperPart = Copy-Vector
$upperPart.domain_gap_outcomes.classes[0].part = 'BOTH'
Assert-GapLedgerRefusal $upperPart 'gap_ledger_part_unknown' `
    'an uppercase part must be refused'

# The two encodings must not carry each other's fields: whichever is ignored
# leaves the expansion unchanged, so the document means what it means without it.
$strayOrdinals = Copy-Vector
$strayOrdinals.domain_gap_outcomes.classes[0] |
    Add-Member -NotePropertyName 'ordinals' -NotePropertyValue @(999)
Assert-GapLedgerRefusal $strayOrdinals 'gap_ledger_ordinals_ignored' `
    'a lane_sequential class carrying ordinals must be refused'

# Two classes with ONE identity would have been one class: splitting a run across
# them expands to exactly what the merged class does.
$twinClass = Copy-Vector
$twinClasses = New-Object System.Collections.ArrayList
foreach ($class in @($twinClass.domain_gap_outcomes.classes)) { [void]$twinClasses.Add($class) }
[void]$twinClasses.Add(($twinClass.domain_gap_outcomes.classes[0] | ConvertTo-Json -Depth 20 | ConvertFrom-Json))
$twinClass.domain_gap_outcomes.classes = $twinClasses.ToArray()
$twinClass.domain_gap_outcomes.class_count = $twinClasses.Count
$twinClass.domain_gap_outcomes.row_count =
    (Get-VectorRowCount $twinClass 0)
Assert-GapLedgerRefusal $twinClass 'gap_ledger_duplicate_class_identity' `
    'two classes with one identity must be refused'

# A domain list out of order expands to the same rows after the reader's sort.
$unorderedDomains = ConvertFrom-Json @'
{"schema":"hoi4-gap-ledger-v2","version":2,
 "gaps":{"row_count":0,"class_count":0,"classes":[]},
 "domain_gap_outcomes":{"row_count":2,"class_count":1,
  "ordinal_encoding":"explicit",
  "classes":[{"reason":"torn","part":"state","domains":[5,4],
    "queue_capacity":0,"queue_depth":0,"queue_dropped":0,
    "range_start_rel":0,"range_end_rel":0,
    "tick_before":{"rel":0},"tick_after":{"rel":0},
    "hour_runs":[[100,1]],"ordinals":[0,1]}]}}
'@
Assert-GapLedgerRefusal $unorderedDomains 'gap_ledger_domains_unordered' `
    'a domain list out of order must be refused'

# A run count outside `u32` is refused before the loop that would walk it.
$hugeRun = ConvertFrom-Json @'
{"schema":"hoi4-gap-ledger-v2","version":2,
 "gaps":{"row_count":0,"class_count":0,"classes":[]},
 "domain_gap_outcomes":{"row_count":1,"class_count":1,
  "ordinal_encoding":"explicit",
  "classes":[{"reason":"torn","part":"state","domains":[4],
    "queue_capacity":0,"queue_depth":0,"queue_dropped":0,
    "range_start_rel":0,"range_end_rel":0,
    "tick_before":{"rel":0},"tick_after":{"rel":0},
    "hour_runs":[[-2147483648,4294967296]],"ordinals":[0]}]}}
'@
Assert-GapLedgerRefusal $hugeRun 'gap_ledger_hour_run_empty' `
    'a run count outside u32 must be refused before the loop walks it'

# A one-element array where an OBJECT belongs is not that object.
$arrayLedger = ConvertFrom-Json @'
{"gap_ledger":[{"schema":"hoi4-gap-ledger-v2","version":2,
 "gaps":{"row_count":0,"class_count":0,"classes":[]},
 "domain_gap_outcomes":{"row_count":0,"class_count":0,
  "ordinal_encoding":"lane_sequential","lane_ordinal_first":[],"classes":[]}}]}
'@
$arrayLedgerResult = Expand-CaptureGapOutcomes $arrayLedger
Assert-GapLedger -Condition (
    $null -ne $arrayLedgerResult -and -not $arrayLedgerResult.Ok -and
    [string]$arrayLedgerResult.Problem -ceq 'gap_ledger_missing') `
    -Because ('a gap_ledger wrapped in an array must be refused AS AN ARRAY. ' +
              'Asserting only failure was vacuous: with the array check removed the ' +
              'array has no top-level version member and the version clause refuses ' +
              "it anyway (got '$($arrayLedgerResult.Problem)')")

# THE STRUCTURAL CLOSE on the "ignored field" family: an unread field is a second
# spelling, and it still counts toward a class identity, so it can split an
# aggregation key and walk through the same-hour check.
# Closing the CLASSES but not their CONTAINER left the same escape one level up.
$strayTableField = Copy-Vector
$strayTableField.domain_gap_outcomes |
    Add-Member -NotePropertyName 'novel_table_field' -NotePropertyValue 1
Assert-GapLedgerRefusal $strayTableField 'gap_ledger_table_unknown_field' `
    'a table field this format does not define must be refused'

# The NESTED escape, exactly as round 13 demonstrated it: two classes at one hour
# differing only by a field nothing reads. Without a closed anchor the extra
# field reaches the aggregation key and the same-hour rule never fires.
$strayAnchorField = Copy-Vector
$anchorClass = $strayAnchorField.domain_gap_outcomes.classes[0]
$anchorTwin = $anchorClass | ConvertTo-Json -Depth 20 | ConvertFrom-Json
$anchorClass.domains = @(10)
$anchorTwin.domains = @(11)
$anchorTwin.tick_before | Add-Member -NotePropertyName 'novel' -NotePropertyValue 1
$strayAnchorField.domain_gap_outcomes.classes = @($anchorClass, $anchorTwin)
$strayAnchorField.domain_gap_outcomes.class_count = 2
# NAMED, not just "not Ok". Asserting only failure would have passed on the
# UNWRAPPED ledger's `gap_ledger_missing` -- which is exactly how this case was
# vacuous when first written.
Assert-GapLedgerRefusal $strayAnchorField 'gap_ledger_tick_anchor_ambiguous' `
    ('a nested field nothing reads must not separate two classes at one hour; ' +
     'PowerShell returned Ok with two rows before the anchor was closed, while ' +
     'Rust and Python refused the same document')

# Allowlisting `note` BY NAME left an object-shaped hole.
$objectNote = Copy-Vector
$objectNote.note = (ConvertFrom-Json '{"novel_nested_field":1}')
Assert-GapLedgerRefusal $objectNote 'gap_ledger_note_not_a_string' `
    'a note that is an object, not a string, must be refused'

# A DIFFERENT string is still fine: the prose is not load-bearing.
$otherNote = Copy-Vector
$otherNote.note = 'anything at all'
$otherNoteResult = Expand-CaptureGapOutcomes ([pscustomobject]@{ gap_ledger = $otherNote })
Assert-GapLedger -Condition $otherNoteResult.Ok `
    -Because 'the note prose is documentation, so a different string must still expand'

$strayLedgerField = Copy-Vector
$strayLedgerField | Add-Member -NotePropertyName 'novel_ledger_field' -NotePropertyValue 1
Assert-GapLedgerRefusal $strayLedgerField 'gap_ledger_unknown_field' `
    'a ledger field this format does not define must be refused'

$strayClassField = Copy-Vector
$strayClassField.domain_gap_outcomes.classes[0] |
    Add-Member -NotePropertyName 'repeat' -NotePropertyValue 1
Assert-GapLedgerRefusal $strayClassField 'gap_ledger_class_unknown_field' `
    'a class field this format does not define must be refused'

$strayLaneField = Copy-Vector
$strayLaneField.domain_gap_outcomes.lane_ordinal_first[0] |
    Add-Member -NotePropertyName 'novel' -NotePropertyValue 1
Assert-GapLedgerRefusal $strayLaneField 'gap_ledger_lane_unknown_field' `
    'a lane field this format does not define must be refused'

$noBase = Copy-Vector
$noBase.domain_gap_outcomes.lane_ordinal_first[0].PSObject.Properties.Remove('first')
Assert-GapLedgerRefusal $noBase 'gap_ledger_unreadable' `
    'a lane with no ordinal base must be refused, not numbered from zero'

$verdict = [ordered]@{
    schema = 'battle-plan.gap_ledger_expansion_test.v1'
    status = if ($failures.Count -eq 0) { 'PASS' } else { 'FAIL' }
    cases = $cases
    failures = @($failures)
}
$verdict | ConvertTo-Json -Depth 5 -Compress
if ($failures.Count -ne 0) { exit 1 }
