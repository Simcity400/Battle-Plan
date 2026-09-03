<#
.SYNOPSIS
Exercises the harness's finalize progress bound without launching the game or the
recorder.

.DESCRIPTION
`Invoke-HeadlessRawCapture.ps1` used to bound post-capture finalization with a
fixed wall clock (`-RecorderOverheadSeconds`, then a flat 300 s default) that had no
relation to the amount of work. On 2026-08-06 that cost three live trials:
t27/t28/t29 were declared unresolved while their folds were provably still
incomplete -- counting the per-hour records in each run's `checkpoints.h4cp`
against the hour count in its own manifest gives 443/542, 149/577 and 194/459,
and none had reached `hour_index.h4ix`.

`Update-RecorderFinalizeDeadline` replaces that clock with the recorder's
published `RECORDER_FINALIZE_PROGRESS_V1.json`. This script extracts that one
function out of the harness by AST -- the harness itself is never executed --
and drives its whole decision table over temporary receipts.

The load-bearing case is `advancing work extends the deadline`: before this
change no evidence could extend it, which is exactly why a working recorder was
called wedged.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$harness = Join-Path $scriptDir 'Invoke-HeadlessRawCapture.ps1'

# Pull the single function out of the harness rather than dot-sourcing it:
# dot-sourcing would run a capture.
$parseErrors = $null
$tokens = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    $harness, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -gt 0) {
    throw "harness does not parse: $($parseErrors[0].Message)"
}
$functionAst = $ast.FindAll(
    {
        param($node)
        $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -eq 'Update-RecorderFinalizeDeadline'
    }, $true) | Select-Object -First 1
if ($null -eq $functionAst) {
    throw 'Update-RecorderFinalizeDeadline is not defined in the harness.'
}
# The stall spelling both the grant function and the terminal reason call.
$stallReasonAst = $ast.FindAll(
    {
        param($node)
        $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -eq 'Get-RecorderFinalizeStallReason'
    }, $true) | Select-Object -First 1
if ($null -eq $stallReasonAst) {
    throw 'Get-RecorderFinalizeStallReason is not defined in the harness.'
}
. ([scriptblock]::Create($stallReasonAst.Extent.Text))

# THE TWO FUNCTIONS THAT DECIDE WHAT HAPPENS WHEN THE BOUND ABOVE RUNS OUT, and
# they belong here for the reason the bound does: both carry defects that
# reading cannot see, and neither can be exercised without running it.
#
# `Get-RecorderFinalizeTerminalReason` exists because the reason a timed-out run
# printed was structurally the opposite of the truth. The grant function writes
# its reason as a side effect of GRANTING, so the last thing it wrote before a
# stall ended the wait was `work_advancing_unchanged_<n>ms` -- the extension it
# had just handed out. `no_progress_for_<n>s` is only ever written by a call
# made AFTER the counter crosses the threshold, and the loop never makes one,
# because by then it has walked past its deadline and left. Run t60 timed out on
# a receipt frozen for 342 s and reported work advancing.
#
# `Test-TargetProcessGone` is what makes stopping a recorder admissible at all.
# "Never terminate collector_record after injection" protects hooks and the
# injected DLL inside a LIVE the game; when the attached process is gone there is
# nothing left to orphan. So the predicate must be exact about identity -- a pid
# alone is not one, because Windows reuses pids -- and must answer "not gone" to
# everything it cannot determine, since it only ever unlocks the more forceful
# action.
$watchdogFunctionAsts = @(
    $ast.FindAll(
        {
            param($node)
            $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
            @('Get-RecorderFinalizeTerminalReason', 'Test-TargetProcessGone') -contains $node.Name
        }, $true))
if ($watchdogFunctionAsts.Count -ne 2) {
    throw ('The harness must define both Get-RecorderFinalizeTerminalReason and ' +
        'Test-TargetProcessGone; found ' + $watchdogFunctionAsts.Count + '.')
}
foreach ($lifted in $watchdogFunctionAsts) {
    . ([scriptblock]::Create([string]$lifted.Extent.Text))
}

# The harness must define it BEFORE the main try block. The cleanup `finally`
# calls it, and PowerShell only defines a function once execution reaches it, so
# a definition placed after the recorder is launched would turn any upstream
# throw into an undefined-command error inside the cleanup -- abandoning the
# compact-status arbitration the harness depends on.
# TOP-LEVEL statements only, not "the first try past line 1000". That line
# threshold was a stand-in for "past the function definitions", and the harness
# outgrew it: it now selects a `try` nested inside a `Where-Object` filter in
# `Invoke-BoundedWmiLaunch`, which is not a cleanup boundary and is not even a
# statement the script executes at load. The check then failed on a harness that
# is correct -- a red that says nothing is worse than no check, because the next
# reader learns to ignore this file.
#
# The real boundary is structural: a `try` that is a direct statement of the
# script's end block is one whose `finally` runs on the way out of the capture.
# Taking the FIRST of them is the strictest form of the requirement.
$mainTry = @($ast.EndBlock.Statements |
    Where-Object { $_ -is [System.Management.Automation.Language.TryStatementAst] } |
    Sort-Object { $_.Extent.StartLineNumber }) |
    Select-Object -First 1
if ($null -eq $mainTry) { throw 'could not locate the harness main try block.' }
if ($functionAst.Extent.StartLineNumber -ge $mainTry.Extent.StartLineNumber) {
    throw ("Update-RecorderFinalizeDeadline is defined at line " +
        "$($functionAst.Extent.StartLineNumber), at or after the main try block at " +
        "line $($mainTry.Extent.StartLineNumber); the cleanup path could call it " +
        'before it exists.')
}

# `FileInfo` populates its properties on FIRST ACCESS and caches thereafter, so
# the pre-read metadata must be MATERIALISED into plain values before the
# content is read. Leaving it as `$before.LastWriteTimeUtc` and touching it only
# after `Get-Content` samples the file at that later moment and compares it
# against itself -- a straddled-rename check that can never fail. That mistake
# is invisible at runtime (the guard simply always passes), so it is pinned
# here structurally rather than behaviourally.
$functionText = $functionAst.Extent.Text
$functionLines = $functionText -split "`r?`n"
$firstContentRead = [Array]::FindIndex(
    $functionLines, [Predicate[string]] { param($line) $line -match 'Get-Content' })
$materialised = [Array]::FindIndex(
    $functionLines,
    [Predicate[string]] { param($line) $line -match '\$beforeWriteUtc\s*=' })
if ($firstContentRead -lt 0) { throw 'the bound function no longer reads the receipt.' }
if ($materialised -lt 0 -or $materialised -gt $firstContentRead) {
    throw ('the pre-read receipt metadata is not materialised before the content read; ' +
        'the straddled-rename guard would compare the file against itself.')
}

. ([scriptblock]::Create($functionAst.Extent.Text))

$sessionId = '11111111-2222-3333-4444-555555555555'
$captureId = 'headless-raw-capture-selftest'
$noProgressSeconds = 60

$root = Join-Path $env:TEMP "recorder_finalize_bound_$PID"
if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
[void](New-Item -ItemType Directory -Path $root -Force)

$failures = New-Object System.Collections.Generic.List[string]
function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { $script:failures.Add($Message) }
}

function Write-Receipt {
    param(
        [string]$Dir,
        [string]$Phase = 'finalizing',
        [int64]$UnchangedMs = 0,
        [string]$Schema = 'battle-plan-recorder-finalize-progress-v1',
        [string]$Session = $sessionId,
        [string]$Capture = $captureId,
        [int64]$WorkBytes = 4096,
        # Defaults to "sampled just now", which is what a live publisher writes.
        [object]$SampleUtc = $null,
        [int64]$WorkChangedOrdinal = 6,
        [string]$Stage = ''
    )
    if ($null -eq $SampleUtc) { $SampleUtc = [DateTime]::UtcNow }
    $receipt = [ordered]@{
        schema               = $Schema
        pid                  = 4321
        session_id           = $Session
        capture_id           = $Capture
        phase                = $Phase
        stage                = $Stage
        sample_ordinal       = 7
        elapsed_ms           = 30000
        work_bytes           = $WorkBytes
        work_changed_ordinal = $WorkChangedOrdinal
        work_unchanged_ms    = $UnchangedMs
        sample_unix_ms       = [DateTimeOffset]::new(
            [DateTime]::SpecifyKind($SampleUtc, [DateTimeKind]::Utc)
        ).ToUnixTimeMilliseconds()
    }
    $receipt | ConvertTo-Json -Depth 4 |
        Set-Content -LiteralPath (Join-Path $Dir 'RECORDER_FINALIZE_PROGRESS_V1.json') -Encoding UTF8
}

function New-Case {
    param([string]$Name)
    $dir = Join-Path $root $Name
    [void](New-Item -ItemType Directory -Path $dir -Force)
    return $dir
}

function Invoke-Bound {
    param(
        [string]$Dir,
        [ref]$Deadline,
        [object]$Ceiling,
        [ref]$Reason
    )
    return (Update-RecorderFinalizeDeadline `
        -RunDir $Dir `
        -SessionId $sessionId `
        -CaptureId $captureId `
        -Deadline $Deadline `
        -Ceiling $Ceiling `
        -NoProgressSeconds $noProgressSeconds `
        -Reason $Reason)
}

$farCeiling = [DateTime]::UtcNow.AddSeconds(7200)

# --- 1. No receipt: the fixed budget stands, exactly as before this change. ---
$dir = New-Case 'absent'
$deadline = [DateTime]::UtcNow.AddSeconds(10)
$original = $deadline
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True (-not $extended) 'a missing receipt extended the deadline'
Assert-True ($deadline -eq $original) 'a missing receipt moved the deadline'

# --- 2. THE regression case: advancing work buys time. ---
$dir = New-Case 'advancing'
Write-Receipt -Dir $dir -UnchangedMs 1500
$deadline = [DateTime]::UtcNow.AddSeconds(10)
$original = $deadline
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True $extended 'advancing finalize work did not extend the deadline'
Assert-True ($deadline -gt $original) 'advancing finalize work did not move the deadline forward'
Assert-True ($deadline -gt [DateTime]::UtcNow.AddSeconds($noProgressSeconds - 5)) `
    "extension was shorter than the no-progress window (deadline=$($deadline.ToString('o')))"
Assert-True ($reason -like 'work_advancing*') "extension reason was '$reason'"

# --- 3. Stalled work CUTS the deadline to now, however much budget stood. ---
# rc_vanilla8's refused attempt waited 531 s on a receipt that had reported
# no progress since its first sample, because the READY-anchored budget was
# still standing and a stall only ever declined to extend it.
$dir = New-Case 'stalled'
Write-Receipt -Dir $dir -UnchangedMs ([int64]$noProgressSeconds * 1000 + 1)
$deadline = [DateTime]::UtcNow.AddSeconds(600)
$before = [DateTime]::UtcNow
$reason = $null
$cut = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True $cut 'a stalled finalize did not report that it moved the deadline'
Assert-True ($deadline -le [DateTime]::UtcNow -and $deadline -ge $before) `
    "a stalled finalize left the deadline at $($deadline.ToString('o')) instead of cutting it to now"
Assert-True ($reason -like 'no_progress_for_*') "stall reason was '$reason'"
# A deadline already in the past is not moved forward by a cut.
$deadline = [DateTime]::UtcNow.AddSeconds(-30)
$original = $deadline
$reason = $null
$cut = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True (-not $cut) 'a cut moved a deadline that had already passed'
Assert-True ($deadline -eq $original) 'a cut moved a past deadline forward'

# --- 3b. A finalize that NEVER advanced is named as such, with its stage. ---
$dir = New-Case 'never_advanced'
Write-Receipt -Dir $dir -UnchangedMs ([int64]$noProgressSeconds * 1000 + 5000) `
    -WorkChangedOrdinal 0 -Stage 'content_census'
$deadline = [DateTime]::UtcNow.AddSeconds(3000)
$reason = $null
$cut = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True $cut 'a never-started finalize did not cut the deadline'
Assert-True ($deadline -le [DateTime]::UtcNow) 'a never-started finalize kept its budget'
Assert-True ($reason -eq "finalize_never_advanced_for_$($noProgressSeconds + 5)s_in_content_census") `
    "never-advanced reason was '$reason'"

# --- 3c. A stall that stopped in a named stage names it. ---
$dir = New-Case 'stalled_in_stage'
Write-Receipt -Dir $dir -UnchangedMs ([int64]$noProgressSeconds * 1000 + 1000) -Stage 'parity_verify'
$deadline = [DateTime]::UtcNow.AddSeconds(3000)
$reason = $null
[void](Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason))
Assert-True ($reason -eq "no_progress_for_$($noProgressSeconds + 1)s_in_parity_verify") `
    "staged stall reason was '$reason'"

# --- 3d. Below the threshold nothing is cut: the ordinary extension stands. ---
$dir = New-Case 'almost_stalled'
Write-Receipt -Dir $dir -UnchangedMs ([int64]$noProgressSeconds * 1000 - 2000) -Stage 'fold_and_encode'
$deadline = [DateTime]::UtcNow.AddSeconds(1)
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True $extended 'work just under the stall threshold was not extended'
Assert-True ($deadline -gt [DateTime]::UtcNow) 'work just under the stall threshold was cut'
Assert-True ($reason -like 'work_advancing*') "near-threshold reason was '$reason'"

# --- 4. A receipt from another attempt in a reused OutDir proves nothing. ---
$dir = New-Case 'foreign'
Write-Receipt -Dir $dir -UnchangedMs 0 -Session '99999999-9999-9999-9999-999999999999'
$deadline = [DateTime]::UtcNow.AddSeconds(10)
$original = $deadline
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True (-not $extended) 'a foreign session_id extended the deadline'
Assert-True ($deadline -eq $original) 'a foreign session_id moved the deadline'
Assert-True ($reason -eq 'foreign_progress_receipt') "foreign reason was '$reason'"

$dir = New-Case 'foreign_capture'
Write-Receipt -Dir $dir -UnchangedMs 0 -Capture 'headless-raw-capture-someone-else'
$deadline = [DateTime]::UtcNow.AddSeconds(10)
$original = $deadline
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True (-not $extended) 'a foreign capture_id extended the deadline'
Assert-True ($deadline -eq $original) 'a foreign capture_id moved the deadline'

# --- 5. `complete` covers the last writes and the exit. ---
$dir = New-Case 'complete'
Write-Receipt -Dir $dir -Phase 'complete' -UnchangedMs 0
$deadline = [DateTime]::UtcNow.AddSeconds(1)
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True $extended 'a completed finalize did not get its exit window'
Assert-True ($reason -eq 'finalize_complete_exit_grace') "complete reason was '$reason'"

# A `complete` receipt whose staleness exceeds the window still gets the exit
# window: the fold is done, and the bound is on WORK, which there is no longer
# any of.
$dir = New-Case 'complete_stale'
Write-Receipt -Dir $dir -Phase 'complete' -UnchangedMs ([int64]$noProgressSeconds * 1000 + 5000)
$deadline = [DateTime]::UtcNow.AddSeconds(1)
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True $extended 'a completed finalize was refused its exit window'

# --- 6. The ceiling is absolute: progress cannot buy unbounded time. ---
$dir = New-Case 'ceiling'
Write-Receipt -Dir $dir -UnchangedMs 0
$ceiling = [DateTime]::UtcNow.AddSeconds(5)
$deadline = [DateTime]::UtcNow.AddSeconds(1)
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $ceiling -Reason ([ref]$reason)
Assert-True $extended 'progress below the ceiling did not extend'
Assert-True ($deadline -le $ceiling) `
    "the extension ran past the ceiling ($($deadline.ToString('o')) > $($ceiling.ToString('o')))"

# Already at the ceiling: nothing further is granted.
$dir = New-Case 'ceiling_reached'
Write-Receipt -Dir $dir -UnchangedMs 0
$ceiling = [DateTime]::UtcNow.AddSeconds(5)
$deadline = $ceiling
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $ceiling -Reason ([ref]$reason)
Assert-True (-not $extended) 'the deadline was pushed past an already-reached ceiling'
Assert-True ($deadline -eq $ceiling) 'the ceiling moved'

# A null ceiling (a route that threw before READY) grants nothing and throws
# nothing -- the cleanup path reaches this function on exactly that route.
$dir = New-Case 'null_ceiling'
Write-Receipt -Dir $dir -UnchangedMs 0
$deadline = [DateTime]::UtcNow.AddSeconds(10)
$original = $deadline
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $null -Reason ([ref]$reason)
Assert-True (-not $extended) 'a null ceiling extended the deadline'
Assert-True ($deadline -eq $original) 'a null ceiling moved the deadline'

# --- 7. A torn or foreign document is transient/unknown, never a grant. ---
$dir = New-Case 'torn'
Set-Content -LiteralPath (Join-Path $dir 'RECORDER_FINALIZE_PROGRESS_V1.json') `
    -Value '{"schema":"battle-plan-recorder-finalize-progr' -Encoding UTF8
$deadline = [DateTime]::UtcNow.AddSeconds(10)
$original = $deadline
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True (-not $extended) 'a torn receipt extended the deadline'
Assert-True ($deadline -eq $original) 'a torn receipt moved the deadline'

$dir = New-Case 'schema'
Write-Receipt -Dir $dir -UnchangedMs 0 -Schema 'battle-plan-recorder-finalize-progress-v0'
$deadline = [DateTime]::UtcNow.AddSeconds(10)
$original = $deadline
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True (-not $extended) 'an unknown schema extended the deadline'
Assert-True ($deadline -eq $original) 'an unknown schema moved the deadline'

# --- 8. A well-formed JSON document missing the fields must not THROW. ---
# This function is called from the harness cleanup `finally`. Under
# `Set-StrictMode -Version Latest` a missing property on the object
# ConvertFrom-Json returns is a terminating error, and one thrown there
# abandons the compact-status arbitration.
$dir = New-Case 'missing_fields'
Set-Content -LiteralPath (Join-Path $dir 'RECORDER_FINALIZE_PROGRESS_V1.json') `
    -Value '{"schema":"battle-plan-recorder-finalize-progress-v1"}' -Encoding UTF8
$deadline = [DateTime]::UtcNow.AddSeconds(10)
$original = $deadline
$reason = $null
$threw = $false
try {
    $extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
} catch {
    $threw = $true
    $extended = $false
}
Assert-True (-not $threw) 'a receipt missing its fields threw out of the bound function'
Assert-True (-not $extended) 'a receipt missing its fields extended the deadline'
Assert-True ($deadline -eq $original) 'a receipt missing its fields moved the deadline'

# Identity present but staleness absent: absent is not zero.
$dir = New-Case 'missing_staleness'
Set-Content -LiteralPath (Join-Path $dir 'RECORDER_FINALIZE_PROGRESS_V1.json') `
    -Value "{`"schema`":`"battle-plan-recorder-finalize-progress-v1`",`"session_id`":`"$sessionId`",`"capture_id`":`"$captureId`",`"phase`":`"finalizing`"}" `
    -Encoding UTF8
$deadline = [DateTime]::UtcNow.AddSeconds(10)
$original = $deadline
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True (-not $extended) 'a receipt with no work_unchanged_ms extended the deadline'
Assert-True ($deadline -eq $original) 'a receipt with no work_unchanged_ms moved the deadline'

# --- 9. An empty JSON object is neither a grant nor a throw. ---
$dir = New-Case 'empty_object'
Set-Content -LiteralPath (Join-Path $dir 'RECORDER_FINALIZE_PROGRESS_V1.json') -Value '{}' -Encoding UTF8
$deadline = [DateTime]::UtcNow.AddSeconds(10)
$original = $deadline
$reason = $null
$threw = $false
try {
    $extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
} catch {
    $threw = $true
    $extended = $false
}
Assert-True (-not $threw) 'an empty receipt object threw out of the bound function'
Assert-True (-not $extended) 'an empty receipt object extended the deadline'
Assert-True ($deadline -eq $original) 'an empty receipt object moved the deadline'

# --- 10. A receipt that has stopped being rewritten is no evidence. ---
# Publication failures are swallowed inside the recorder so they cannot fail a
# good capture, and a thread that never started leaves its first sample behind.
# Either way the file freezes. A frozen sample still reading "work advancing"
# would keep buying time until the ceiling, so age is checked on the FILE --
# a dead publisher cannot update a field inside it.
$dir = New-Case 'frozen'
Write-Receipt -Dir $dir -UnchangedMs 0
$receiptPath = Join-Path $dir 'RECORDER_FINALIZE_PROGRESS_V1.json'
(Get-Item -LiteralPath $receiptPath).LastWriteTimeUtc = [DateTime]::UtcNow.AddSeconds(-120)
$deadline = [DateTime]::UtcNow.AddSeconds(10)
$original = $deadline
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True (-not $extended) 'a frozen receipt extended the deadline'
Assert-True ($deadline -eq $original) 'a frozen receipt moved the deadline'
Assert-True ($reason -like 'progress_receipt_stale_*') "frozen reason was '$reason'"

# --- 11. The granted window is ABSOLUTE, not `now + window` on every poll. ---
# A rolling window hands a stall a fresh full allowance on each read, so a
# 60 s window would admit nearly 120 s. The deadline must be anchored on when
# progress last happened, so repeated polls converge instead of accumulating.
$dir = New-Case 'absolute_window'
Write-Receipt -Dir $dir -UnchangedMs 45000
$deadline = [DateTime]::UtcNow.AddSeconds(1)
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True $extended 'work advancing within the window did not extend'
# 45 s of the 60 s window is already spent, so at most ~15 s may remain.
$granted = ($deadline - [DateTime]::UtcNow).TotalSeconds
Assert-True ($granted -le ($noProgressSeconds - 45 + 5)) `
    "a stall 45s into a ${noProgressSeconds}s window was granted $([Math]::Round($granted,1))s"
Assert-True ($granted -gt 0) "the remaining window was not granted at all ($granted s)"

# Polling again must not accumulate: the anchor has not moved.
$first = $deadline
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True (-not $extended) 're-reading the same receipt granted more time'
Assert-True ($deadline -eq $first) 'the deadline accumulated across polls of one receipt'

# --- 12. A terminal `complete` grants one bounded grace, not a lease. ---
$dir = New-Case 'complete_once'
Write-Receipt -Dir $dir -Phase 'complete' -UnchangedMs 0
$deadline = [DateTime]::UtcNow.AddSeconds(1)
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True $extended 'a completed finalize did not get its exit grace'
$firstComplete = $deadline
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True (-not $extended) 'the terminal receipt renewed as a lease'
Assert-True ($deadline -eq $firstComplete) 'the terminal receipt kept pushing the deadline'

# --- 13. An unrecognised phase buys nothing even with matching identity. ---
$dir = New-Case 'unknown_phase'
Write-Receipt -Dir $dir -Phase 'reticulating' -UnchangedMs 0
$deadline = [DateTime]::UtcNow.AddSeconds(10)
$original = $deadline
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True (-not $extended) 'an unrecognised phase extended the deadline'
Assert-True ($deadline -eq $original) 'an unrecognised phase moved the deadline'

# --- 14. The anchor is the SAMPLE time, not the file's modified time. ---
# The file's stamp trails the measurement by the metering walk plus serialize,
# write, sync and rename. Anchoring on the file would push the reconstructed
# last-progress instant forward by that latency and grant time the recorder
# never earned -- and the freshness gate cannot catch it, because such a
# receipt is genuinely newly written.
$dir = New-Case 'sample_anchor'
# Sampled 50 s ago with zero staleness at that moment; published just now.
Write-Receipt -Dir $dir -UnchangedMs 0 -SampleUtc ([DateTime]::UtcNow.AddSeconds(-50))
$deadline = [DateTime]::UtcNow.AddSeconds(1)
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True $extended 'a freshly published sample did not extend'
$granted = ($deadline - [DateTime]::UtcNow).TotalSeconds
# Anchored on the sample: 60 - 50 = ~10 s left. Anchored on the file: ~60 s.
Assert-True ($granted -le ($noProgressSeconds - 50 + 5)) `
    "the grant was anchored on the file, not the sample ($([Math]::Round($granted,1))s)"

# --- 15. A receipt with no sample time buys nothing. ---
$dir = New-Case 'no_sample_time'
Set-Content -LiteralPath (Join-Path $dir 'RECORDER_FINALIZE_PROGRESS_V1.json') `
    -Value "{`"schema`":`"battle-plan-recorder-finalize-progress-v1`",`"session_id`":`"$sessionId`",`"capture_id`":`"$captureId`",`"phase`":`"finalizing`",`"work_unchanged_ms`":0}" `
    -Encoding UTF8
$deadline = [DateTime]::UtcNow.AddSeconds(10)
$original = $deadline
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True (-not $extended) 'a receipt with no sample_unix_ms extended the deadline'
Assert-True ($deadline -eq $original) 'a receipt with no sample_unix_ms moved the deadline'

# --- 16. A sample stamped ahead of its own receipt is refused. ---
# That shape means the wall clock stepped backwards between the sample and the
# write. The freshness gate cannot catch it -- the file is genuinely fresh --
# and an apparently-future sample would buy time all the way to the ceiling.
$dir = New-Case 'sample_from_the_future'
Write-Receipt -Dir $dir -UnchangedMs 0 -SampleUtc ([DateTime]::UtcNow.AddSeconds(600))
$deadline = [DateTime]::UtcNow.AddSeconds(10)
$original = $deadline
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True (-not $extended) 'a sample stamped after its receipt extended the deadline'
Assert-True ($deadline -eq $original) 'a sample stamped after its receipt moved the deadline'
Assert-True ($reason -eq 'progress_sample_ahead_of_its_receipt') "future-sample reason was '$reason'"

# A sample slightly older than its receipt is the ORDINARY case -- the sample is
# taken before the write -- and must still be honoured.
$dir = New-Case 'sample_slightly_older'
Write-Receipt -Dir $dir -UnchangedMs 0 -SampleUtc ([DateTime]::UtcNow.AddSeconds(-2))
$deadline = [DateTime]::UtcNow.AddSeconds(1)
$reason = $null
$extended = Invoke-Bound -Dir $dir -Deadline ([ref]$deadline) -Ceiling $farCeiling -Reason ([ref]$reason)
Assert-True $extended 'ordinary publication latency was refused'

# ---------------------------------------------------------------------------
# WHY THE WAIT ENDED, and whether the recorder may then be stopped.
# ---------------------------------------------------------------------------

function Invoke-TerminalReason {
    param([string]$Dir, [object]$Deadline = $null, [object]$Ceiling = $null, [int]$NoProgressSeconds = 180)
    if ($null -eq $Deadline) { $Deadline = [DateTime]::UtcNow.AddSeconds(-1) }
    if ($null -eq $Ceiling) { $Ceiling = [DateTime]::UtcNow.AddHours(1) }
    return (Get-RecorderFinalizeTerminalReason -RunDir $Dir -SessionId $sessionId `
        -CaptureId $captureId -Deadline $Deadline -Ceiling $Ceiling `
        -NoProgressSeconds $NoProgressSeconds)
}

# THE t60 CASE. A receipt frozen far past the threshold must name the stall.
$dir = New-Case 'terminal_stalled'
Write-Receipt -Dir $dir -UnchangedMs 342775
$reason = Invoke-TerminalReason -Dir $dir
Assert-True ($reason -eq 'no_progress_for_342s') "a 342 s stall reported '$reason'"

# THE rc_vanilla8 CASE. A receipt whose work never moved names the finalize
# that never started, and the stage the recorder was in.
$dir = New-Case 'terminal_never_advanced'
Write-Receipt -Dir $dir -UnchangedMs 350131 -WorkChangedOrdinal 0 -Stage 'content_census'
$reason = Invoke-TerminalReason -Dir $dir
Assert-True ($reason -eq 'finalize_never_advanced_for_350s_in_content_census') `
    "a never-started finalize reported '$reason'"

# THE OFF-BY-ONE-TICK THE SAMPLED FIELD ALONE CANNOT SEE. `work_unchanged_ms` is
# a reading taken at `sample_unix_ms`, not at now. A receipt sampled at 178 s of
# stall and read three seconds later IS a 181 s stall, and naming the sampled
# number reproduces the same near-miss one layer down.
$dir = New-Case 'terminal_stalled_by_elapsed'
Write-Receipt -Dir $dir -UnchangedMs 178000 -SampleUtc ([DateTime]::UtcNow.AddSeconds(-3))
$reason = Invoke-TerminalReason -Dir $dir
Assert-True ($reason -like 'no_progress_for_18*s') `
    "a 178 s sample read three seconds later reported '$reason', not the stall it had become"

# Still advancing by its own account and out of time anyway: that is the
# absolute ceiling, and calling it a stall sends the next reader hunting a spin
# that is not there.
$dir = New-Case 'terminal_ceiling'
Write-Receipt -Dir $dir -UnchangedMs 4012
$at = [DateTime]::UtcNow.AddMinutes(5)
$reason = Invoke-TerminalReason -Dir $dir -Deadline $at -Ceiling $at
# The number is the EFFECTIVE stall (the sampled 4012 ms plus however long the
# read took), so the assertion is on the verdict, not on the arithmetic that
# would make it flaky.
Assert-True ($reason -like 'finalize_ceiling_reached_unchanged_*ms') "a ceiling stop reported '$reason'"

# A receipt from another session is not evidence about this run.
$dir = New-Case 'terminal_foreign'
Write-Receipt -Dir $dir -UnchangedMs 342775 -Session 'someone-else'
$reason = Invoke-TerminalReason -Dir $dir
Assert-True ($reason -eq 'foreign_progress_receipt') "a foreign receipt reported '$reason'"

# A publisher that stopped republishing is dead, and its content is a
# photograph. Naming the frozen field would grant that photograph authority.
$dir = New-Case 'terminal_stale'
Write-Receipt -Dir $dir -UnchangedMs 1000
(Get-Item -LiteralPath (Join-Path $dir 'RECORDER_FINALIZE_PROGRESS_V1.json')).LastWriteTimeUtc =
    [DateTime]::UtcNow.AddMinutes(-10)
$reason = Invoke-TerminalReason -Dir $dir
Assert-True ($reason -like 'progress_receipt_stale_*') "a dead publisher reported '$reason'"

# No receipt at all.
$dir = New-Case 'terminal_absent'
$reason = Invoke-TerminalReason -Dir $dir
Assert-True ($reason -eq 'no_progress_receipt') "an absent receipt reported '$reason'"

# Unparseable is its own answer, and must never throw: this runs inside the
# cleanup `finally`, where a terminating error abandons the status arbitration.
$dir = New-Case 'terminal_garbage'
Set-Content -LiteralPath (Join-Path $dir 'RECORDER_FINALIZE_PROGRESS_V1.json') `
    -Value '{ not json' -Encoding UTF8
$reason = Invoke-TerminalReason -Dir $dir
Assert-True ($reason -eq 'progress_receipt_unreadable') "an unparseable receipt reported '$reason'"

# THE PREDICATE THAT UNLOCKS TERMINATION.
# A pid that cannot be a live client id.
$detail = ''
$gone = Test-TargetProcessGone -TargetProcessId 2147483647 -ExpectedCreationFiletimeUtc 1 `
    -Detail ([ref]$detail)
Assert-True ($gone -and $detail -eq 'pid_absent') "an absent pid read gone=$gone detail='$detail'"

# A LIVE pid whose creation time disagrees is a RECYCLED pid: a different
# process entirely. Matching on the pid alone would keep an unsupervised
# recorder alive on the strength of a stranger.
$self = Get-Process -Id $PID
$selfCreation = [uint64]($self.StartTime.ToUniversalTime().ToFileTimeUtc())
$detail = ''
$gone = Test-TargetProcessGone -TargetProcessId $PID `
    -ExpectedCreationFiletimeUtc ($selfCreation - 1) -Detail ([ref]$detail)
Assert-True ($gone -and $detail -eq 'pid_recycled') "a recycled pid read gone=$gone detail='$detail'"

# The live, identity-matching target. This is the case the post-injection
# termination rule exists for, and it must never unlock a stop.
$detail = ''
$gone = Test-TargetProcessGone -TargetProcessId $PID `
    -ExpectedCreationFiletimeUtc $selfCreation -Detail ([ref]$detail)
Assert-True ((-not $gone) -and $detail -eq 'target_alive') `
    "a live matching target read gone=$gone detail='$detail'"

Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) { Write-Host "FAIL: $failure" }
    throw "$($failures.Count) finalize progress bound assertion(s) failed."
}
Write-Host 'phase=finalize_progress_bound_selftest result=pass cases=48'
exit 0
