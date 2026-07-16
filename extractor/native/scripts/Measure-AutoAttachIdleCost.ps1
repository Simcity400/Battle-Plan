<#
.SYNOPSIS
    Self-timing idle-cost measurement + grading harness for the resident
    auto-attach supervisor. Removes the last clock-bound manual step from the
    live runbook: the owner starts one window and walks away instead of
    hand-timing six 30-minute measurements.

.DESCRIPTION
    Implements the AUTO_ATTACH_DESIGN.md section 10 "Idle-cost gate"
    (extractor/documents/AUTO_ATTACH_DESIGN.md:416-423) as a fail-closed,
    sealed-report, deny-extra acceptance tool, in the style of the sibling
    acceptance scripts (ProductionCapture.Validation.ps1,
    collect_campaign_identity_evidence.py + verify_campaign_identity_evidence.py).

    NO GAME is required (idle-cost is explicitly a "30 minutes with no game"
    measurement, AUTO_ATTACH_DESIGN.md:418). The target process PID is derived at
    runtime from a name (or an explicit -TargetPid for a spawned self-test
    target) -- never hardcoded. The harness self-times the window off a monotonic
    stopwatch (no owner stopwatch) and is safe to walk away from.

    Why PowerShell 5.1 (justification): the runbook explicitly names PS 5.1 for
    idle-cost (LIVE_ACCEPTANCE_RUNBOOK.md:490-500 -> "Get-Counter / Get-Process
    ProcessorTime / private bytes"), every sibling acceptance script is .ps1, and
    the three externally-observable metrics are locale-independent PS/CIM
    primitives: System.Diagnostics.Process.TotalProcessorTime (CPU),
    .PrivateMemorySize64 (private bytes), and CIM
    Win32_PerfRawData_PerfProc_Thread.ContextSwitchesPersec summed by IDProcess
    (context switches). These avoid Get-Counter's LOCALIZED counter-path names,
    which break on non-English Windows (portability law: no rig hardcoding).

    ---------------------------- METRIC MAP (section 10) ----------------------------
    Doc refs: AUTO_ATTACH_DESIGN.md:419-422 (the five metrics + fail conditions),
    :420-421 (61 / 900 snapshot bounds); policy.rs:18-80 grounds the cadence
    (HEALTHY_RECONCILE_MS 30000 -> 61, DEGRADED_RECONCILE_MS 2000 -> 900).

      1. CPU time            -- DIRECTLY SAMPLED (TotalProcessorTime delta).
                                Graded for "busy loop" (:421) via core-fraction.
      2. private bytes       -- DIRECTLY SAMPLED (PrivateMemorySize64 series).
                                Graded for "unbounded growth" (:421) from the
                                series shape + net growth.
      3. wakeups/context sw. -- context-switch side DIRECTLY SAMPLED (CIM thread
                                raw counter). "wakeups" proper (timer/power
                                wakeups) is a distinct, not-freely-available
                                counter -> not claimed; recorded as evidence.
      4. snapshot count      -- OWED. Bound 61 (healthy) / 900 (degraded) is a
                                HARD numeric (:420-421) BUT it is a
                                supervisor-INTERNAL reconcile count. The
                                supervisor as-is exposes NO external per-snapshot
                                counter (journal.rs records only external state
                                actions; run_supervisor prints nothing,
                                auto_attach.rs:116-119). Sampling it directly needs
                                a supervisor idle-cost telemetry emit, which the
                                activation freeze forbids. NOT fabricated: reported
                                OWED, with the policy-DERIVED expected max recorded
                                for reference and EXTERNAL CORROBORATION that the
                                supervisor is not exceeding it (a runaway snapshot
                                loop necessarily shows as a CPU busy loop, metric 1).
      5. WMI reconnects      -- OWED. Also supervisor-internal, no external surface
                                (section 10 gives no numeric bound; it says only
                                "record"). Reported OWED, not fabricated.

    The grader therefore emits, per metric: PASS / FAIL (metrics 1,2 -- the
    section-10 qualitative fail conditions), RECORDED (evidence deltas that
    section 10 requires be reported, :422), or OWED (metrics 4,5). Overall verdict
    is PASS_WITH_OWED at best (never a clean unconditional PASS) because two
    metrics are OWED -- the honest state until a one-line supervisor telemetry
    emit lands. Any fail-closed condition or any FAIL bound -> FAIL.

    ANTI-TRUST GRADING (the grader trusts NO self-reported summary):
      * The graded window duration is RE-DERIVED from the sample timestamps
        (first..last) and must be >= the real 1800 s gate with NO slack -- a
        stored requested/elapsed is never trusted, and a short dry-run window can
        never reach a passing verdict.
      * CPU delta, private-bytes series/growth, context-switch total, sample
        count, and cadence are all RE-DERIVED from the raw per-sample series and
        cross-checked against any stored summary; a mismatch is fail-closed.
      * The nonce ledger seal is MANDATORY: an absent ledger root / missing seal
        is fail-closed (a grade cannot be bypassed).
      * Deny-extra is RECURSIVE (window, target, tool, derived_bounds, each
        sample, deltas), start_time_filetime is required on EVERY sample, and a
        negative/wrapped context-switch difference is OWED evidence -- never a
        fabricated zero, and a stored negative total is rejected.

    ---------------------------- OWNER CLI (per state) ----------------------------
    Six windows total map to LIVE_ACCEPTANCE_RUNBOOK.md Seq 4 (main PC, 3 windows,
    lines 490-508) and Seq 5 (weaker PC, 3 windows, lines 564-575). Run each of
    the three states on EACH machine (:496-497). NO GAME running during any window.

    The supervisor is started separately (out of this lane's scope) as, e.g.:
      collector_record.exe --auto-attach --dll <collector.dll> --out-root <dir>
    The state is INDUCED externally before the window (this harness measures the
    supervisor as-is; it never flips WMI or scanning -- activation freeze):
      * healthy-wmi        : supervisor running normally (WMI provider healthy).
      * degraded-scanning  : WMI provider loss simulated first (Seq 4 "WMI
                             unavailable" arm, :483) so the 2 s degraded scan is
                             active, THEN start the window.
      * app-without-watcher: the resident app with the watcher NOT running. NOTE:
                             the canonical target here is the section-9 tray host,
                             which is UNBUILT (design section 12 non-closure;
                             main.rs run_auto_attach comment). As-is there is no
                             separate app process; measure whatever process the
                             owner names for this state (expected snapshot bound 0).

    1) MEASURE (writes a sealed report; emits NO verdict):
       powershell -File Measure-AutoAttachIdleCost.ps1 -Mode Measure `
         -State healthy-wmi `
         -TargetProcessName collector_record `
         -OutReport <abs report.json> `
         -NonceLedgerRoot <abs durable dir> `
         -ActionToken MEASURE_AUTO_ATTACH_IDLE_COST_healthy-wmi
       (default WindowSeconds 1800 = the real 30-min gate; SampleIntervalSeconds
        15. -NonceLedgerRoot is REQUIRED. Use -TargetPid to bind a specific PID.
        -WindowSeconds may be lowered for a dry run, but the grader requires a
        re-derived >= 1800 s span, so a dry run can NEVER reach a passing verdict.)

    2) GRADE (the ONLY verdict; fail-closed + recursive deny-extra; ledger
       MANDATORY; every graded quantity re-derived from the samples):
       powershell -File Measure-AutoAttachIdleCost.ps1 -Mode Grade `
         -Report <abs report.json> `
         -OutVerdict <abs verdict.json> `
         -NonceLedgerRoot <abs durable dir> `
         -ActionToken GRADE_AUTO_ATTACH_IDLE_COST

    3) SELF-TEST (fast synthetic windows + one live short window; the real 1800 s
       gate path is exercised via a synthetic full-duration sample series, so no
       30-min wait; no game, no supervisor):
       powershell -File Measure-AutoAttachIdleCost.ps1 -Mode SelfTest

    PowerShell 5.1 compatible. Windows-only. No rig hardcoding. Fail-closed +
    sealed + deny-extra. Never fabricates a metric.
#>
[CmdletBinding()]
param(
    [ValidateSet("Measure", "Grade", "SelfTest")]
    [string]$Mode = "SelfTest",

    # -- Measure --
    [ValidateSet("app-without-watcher", "healthy-wmi", "degraded-scanning")]
    [string]$State,
    [string]$TargetProcessName,
    [int]$TargetPid = 0,
    [int]$WindowSeconds = 1800,
    [int]$SampleIntervalSeconds = 15,
    [string]$OutReport,
    [string]$AttemptNonce,

    # -- Grade --
    [string]$Report,
    [string]$OutVerdict,

    # -- shared --
    [string]$NonceLedgerRoot,
    [string]$ActionToken,

    # -- graded envelopes (section 10 gives NO CPU/byte number; these are
    #    transparent envelopes DERIVED from the design's own sparse cadence, and
    #    are overridable. They are NOT presented as section-10 literals.) --
    [double]$BusyLoopCoreFractionCeiling = 0.10,
    [long]$UnboundedGrowthByteCeiling = 8388608,

    # Per-gap cadence tolerance (seconds) for the fixed-cadence contract. 0 = auto:
    # max(2, interval/2), tight enough to reject clustered/jumped timestamps yet
    # loose enough for real sampling jitter. Overridable; derived, not a section-10
    # literal.
    [double]$CadenceToleranceSeconds = 0
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:ReportSchema  = "battle-plan.auto_attach_idle_cost_report.v1"
$script:VerdictSchema = "battle-plan.auto_attach_idle_cost_verdict.v1"
$script:SealSchema    = "battle-plan.auto_attach_idle_cost_seal.v1"
$script:ValidStates   = @("app-without-watcher", "healthy-wmi", "degraded-scanning")
$script:Hex64         = [regex]"^[0-9a-f]{64}$"

# The real idle-cost gate window is 30 minutes with NO game (AUTO_ATTACH_DESIGN.md
# :418). A grade only satisfies the gate if the sampled window ACTUALLY spans this
# long (recomputed from sample timestamps -- never trusted from a stored field);
# -WindowSeconds stays injectable for the self-test / a dry run, but a dry run can
# never reach a passing verdict.
$script:GateWindowSeconds = 1800

# section 10 (:420-421) + policy.rs: derived expected max reconcile snapshots per
# state over a 30-min window. app-without-watcher has no watcher -> 0. These are
# DERIVED references, never recorded as sampled counts.
$script:DerivedSnapshotMax = @{
    "app-without-watcher" = 0
    "healthy-wmi"         = 61
    "degraded-scanning"   = 900
}
# NOTE on provenance: the healthy 61 is max_snapshots_in_window(Healthy, 1800000)
# = 1 startup + 60x30s reconciles. The degraded 900 is
# max_reconciliations_in_window(Degraded, 1800000) = 900x2s -- it is the
# reconciliation count, NOT max_snapshots_in_window (which would be 901 with the
# startup snapshot included). policy.rs states the degraded figure in these terms.
$script:DerivedSnapshotBasis = "AUTO_ATTACH_DESIGN.md:420-421 + policy.rs: healthy 61 = max_snapshots_in_window(Healthy,1800000ms) (1 startup + 60x30s reconciles); degraded 900 = max_reconciliations_in_window(Degraded,1800000ms) (900x2s; snapshots-inclusive would be 901); app-without-watcher no watcher = 0"

function Fail-Closed {
    param([string]$Message)
    throw "IDLE-COST FAIL-CLOSED: $Message"
}

# ------------------------------------------------------------------ canonical JSON

# Deterministic, sorted-key, compact serializer whose output is STABLE across a
# write -> parse (ConvertFrom-Json) -> re-serialize round trip, so the seal digest
# recomputes identically at grade time. Integral numerics (incl. whole-valued
# doubles, which ConvertFrom-Json re-reads as [long]) always emit as integers;
# fractional doubles emit via 'R' (round-trip) invariant, which is idempotent.
function Format-CanonicalNumber {
    param($Value)
    $ci = [System.Globalization.CultureInfo]::InvariantCulture
    # Integer types are formatted EXACTLY (never routed through [double], which
    # would lose precision above 2^53 -- e.g. FILETIME values ~1.3e17 would
    # collapse and mask a PID-recycle attribution mismatch).
    if ($Value -is [int] -or $Value -is [long] -or $Value -is [int16] -or $Value -is [uint16] -or
        $Value -is [uint32] -or $Value -is [byte] -or $Value -is [sbyte]) {
        return ([long]$Value).ToString($ci)
    }
    if ($Value -is [decimal]) {
        $dec = [decimal]$Value
        if ([math]::Truncate($dec) -eq $dec) { return ([long]$dec).ToString($ci) }
        return $dec.ToString($ci)
    }
    # Floating point: emit whole values within the safe-integer range as integers
    # (ConvertFrom-Json re-reads them as [long], so this keeps the seal stable),
    # otherwise the shortest round-trippable 'R' form (idempotent).
    $d = [double]$Value
    if ([double]::IsNaN($d) -or [double]::IsInfinity($d)) { Fail-Closed "non-finite number cannot be sealed" }
    if ([math]::Floor($d) -eq $d -and [math]::Abs($d) -lt 9007199254740992.0) {
        return ([long]$d).ToString($ci)
    }
    return $d.ToString("R", $ci)
}

function Escape-JsonString {
    param([string]$Value)
    $sb = New-Object System.Text.StringBuilder
    [void]$sb.Append('"')
    foreach ($ch in $Value.ToCharArray()) {
        $code = [int]$ch
        switch ($ch) {
            '"'  { [void]$sb.Append('\"') }
            '\'  { [void]$sb.Append('\\') }
            "`b" { [void]$sb.Append('\b') }
            "`f" { [void]$sb.Append('\f') }
            "`n" { [void]$sb.Append('\n') }
            "`r" { [void]$sb.Append('\r') }
            "`t" { [void]$sb.Append('\t') }
            default {
                if ($code -lt 0x20) { [void]$sb.Append(('\u{0:x4}' -f $code)) }
                else { [void]$sb.Append($ch) }
            }
        }
    }
    [void]$sb.Append('"')
    return $sb.ToString()
}

function ConvertTo-CanonicalJson {
    param($Value)
    if ($null -eq $Value) { return "null" }
    if ($Value -is [bool]) { if ($Value) { return "true" } else { return "false" } }
    if ($Value -is [string]) { return (Escape-JsonString $Value) }
    if ($Value -is [int] -or $Value -is [long] -or $Value -is [uint32] -or $Value -is [int64] -or $Value -is [double] -or $Value -is [single] -or $Value -is [decimal]) {
        return (Format-CanonicalNumber $Value)
    }
    if ($Value -is [System.Collections.IDictionary]) {
        $keys = @($Value.Keys) | Sort-Object { [string]$_ }
        $parts = foreach ($k in $keys) { (Escape-JsonString ([string]$k)) + ":" + (ConvertTo-CanonicalJson $Value[$k]) }
        return "{" + ($parts -join ",") + "}"
    }
    if ($Value -is [System.Management.Automation.PSCustomObject]) {
        $props = $Value.PSObject.Properties | Sort-Object Name
        $parts = foreach ($p in $props) { (Escape-JsonString $p.Name) + ":" + (ConvertTo-CanonicalJson $p.Value) }
        return "{" + ($parts -join ",") + "}"
    }
    if ($Value -is [System.Collections.IEnumerable]) {
        $parts = foreach ($item in $Value) { ConvertTo-CanonicalJson $item }
        return "[" + ($parts -join ",") + "]"
    }
    Fail-Closed "unserializable value of type $($Value.GetType().FullName)"
}

function Get-Sha256Hex {
    param([string]$Text)
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($Text)
        return ([System.BitConverter]::ToString($sha.ComputeHash($bytes))).Replace("-", "").ToLowerInvariant()
    } finally { $sha.Dispose() }
}

# Digest over the report/verdict with its own hash field removed.
function Get-DocumentDigest {
    param([hashtable]$Document, [string]$HashField)
    $clone = @{}
    foreach ($k in $Document.Keys) { if ($k -ne $HashField) { $clone[$k] = $Document[$k] } }
    return (Get-Sha256Hex (ConvertTo-CanonicalJson $clone))
}

function Write-CanonicalFile {
    param([string]$Path, $Value)
    $dir = Split-Path -Parent $Path
    if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    [System.IO.File]::WriteAllText($Path, (ConvertTo-CanonicalJson $Value), (New-Object System.Text.UTF8Encoding($false)))
}

# ConvertFrom-Json yields PSCustomObject trees; convert to plain hashtables/arrays
# so the canonicalizer re-serializes byte-identically to the write path.
function ConvertTo-PlainTree {
    param($Value)
    if ($null -eq $Value) { return $null }
    if ($Value -is [System.Management.Automation.PSCustomObject]) {
        $h = @{}
        foreach ($p in $Value.PSObject.Properties) { $h[$p.Name] = (ConvertTo-PlainTree $p.Value) }
        return $h
    }
    if ($Value -is [object[]]) {
        $arr = @(); foreach ($item in $Value) { $arr += ,(ConvertTo-PlainTree $item) }
        return ,$arr
    }
    return $Value
}

# Two numerics are "the same" for cross-checking iff their canonical forms match
# (normalizes the int/double round-trip so recomputed == stored is exact).
function Test-CanonicalNumberEqual {
    param($A, $B)
    return ((Format-CanonicalNumber $A) -eq (Format-CanonicalNumber $B))
}

# THE single source of truth for every graded scalar. Both the measurer (to write
# the human-readable `deltas` summary) and the grader (to RE-DERIVE the graded
# quantities) call this over the raw per-sample series -- the grader never trusts a
# stored delta. Validates the WHOLE series, not just endpoints (round 2): the CPU
# (TotalProcessorTime) series must be monotonic non-decreasing and its busy-loop
# figure is the MAX per-interval rate (so an intermediate spike that averages out
# cannot hide); the context-switch total is emitted only if the series is
# non-decreasing across EVERY consecutive pair, else churn (no fabricated total).
function Compute-SampleDeltas {
    param([object[]]$PresentSamples, [double]$Elapsed, [bool]$ContextAvailable)
    $d = @{}
    $n = $PresentSamples.Count
    if ($n -lt 1) { return $d }
    $cpuStart = [double]$PresentSamples[0]["cpu_seconds"]
    $cpuEnd   = [double]$PresentSamples[-1]["cpu_seconds"]
    $d["cpu_seconds"] = [math]::Round(($cpuEnd - $cpuStart), 6)
    $wall = [math]::Max($Elapsed, 0.001)
    $d["cpu_core_fraction"] = [math]::Round((($cpuEnd - $cpuStart) / $wall), 6)
    # Full-series CPU: monotonic non-decreasing (a cumulative processor-time counter
    # can never fall) + the max per-interval core fraction across consecutive pairs.
    $cpuMono = $true
    $maxRate = 0.0
    for ($i = 1; $i -lt $n; $i++) {
        $prev = [double]$PresentSamples[$i-1]["cpu_seconds"]
        $cur  = [double]$PresentSamples[$i]["cpu_seconds"]
        if ($cur -lt $prev) { $cpuMono = $false }
        $gap = [double]$PresentSamples[$i]["monotonic_seconds"] - [double]$PresentSamples[$i-1]["monotonic_seconds"]
        if ($gap -le 0) { $gap = 0.001 }
        $rate = ($cur - $prev) / $gap
        if ($rate -gt $maxRate) { $maxRate = $rate }
    }
    $d["cpu_monotonic_nondecreasing"] = $cpuMono
    $d["cpu_max_interval_core_fraction"] = [math]::Round($maxRate, 6)

    $pbStart = [long]$PresentSamples[0]["private_bytes"]
    $pbEnd   = [long]$PresentSamples[-1]["private_bytes"]
    $d["private_bytes_start"] = $pbStart
    $d["private_bytes_end"] = $pbEnd
    $d["private_bytes_net_growth"] = ($pbEnd - $pbStart)
    $mono = ($n -ge 2)
    for ($i = 1; $i -lt $n; $i++) {
        if ([long]$PresentSamples[$i]["private_bytes"] -le [long]$PresentSamples[$i-1]["private_bytes"]) { $mono = $false; break }
    }
    $d["private_bytes_strictly_monotonic_increasing"] = $mono

    if ($ContextAvailable) {
        # A real cumulative context-switch counter is non-decreasing. ANY
        # intermediate decrease (thread exit / counter wrap), not just at the
        # endpoints, means NO valid cumulative total -> churn (never a fabricated 0).
        $csMono = $true
        for ($i = 1; $i -lt $n; $i++) {
            if ([long]$PresentSamples[$i]["context_switches"] -lt [long]$PresentSamples[$i-1]["context_switches"]) { $csMono = $false; break }
        }
        if ($csMono) {
            $d["context_switches_total"] = [long]$PresentSamples[-1]["context_switches"] - [long]$PresentSamples[0]["context_switches"]
            $d["context_switches_thread_churn"] = $false
        } else {
            $d["context_switches_thread_churn"] = $true
        }
    }
    return $d
}

# Recursive deny-extra for a FIXED-schema nested object: every required key present,
# no key outside the allowed set (finding 4). Non-recursive on values; call again on
# each nested object that itself has a fixed schema.
function Assert-ExactKeys {
    param([hashtable]$Map, [string[]]$Allowed, [string]$Where)
    if ($null -eq $Map) { Fail-Closed "$Where is missing/null" }
    foreach ($k in $Allowed) { if (-not $Map.ContainsKey($k)) { Fail-Closed "$Where missing key '$k'" } }
    foreach ($k in $Map.Keys) { if ($Allowed -notcontains $k) { Fail-Closed "$Where has unexpected key '$k' (deny-extra)" } }
}

# Deny-extra ONLY (no key outside the allowed set) for an object with OPTIONAL
# members -- e.g. `deltas`, whose required-ness is enforced by the cross-check
# against the re-derived values, and which legitimately omits context fields on a
# churn/OWED run.
function Assert-NoExtraKeys {
    param([hashtable]$Map, [string[]]$Allowed, [string]$Where)
    if ($null -eq $Map) { Fail-Closed "$Where is missing/null" }
    foreach ($k in $Map.Keys) { if ($Allowed -notcontains $k) { Fail-Closed "$Where has unexpected key '$k' (deny-extra)" } }
}

# --------------------------------------------------------------- target sampling

function Resolve-TargetProcess {
    param([string]$Name, [int]$ProcId)
    if ($ProcId -gt 0) {
        $proc = $null
        try { $proc = Get-Process -Id $ProcId -ErrorAction Stop } catch { Fail-Closed "target PID $ProcId is not a live process" }
        if ($Name) {
            if ($proc.ProcessName -ne $Name -and $proc.ProcessName -ne ($Name -replace '\.exe$','')) {
                Fail-Closed "PID $ProcId is '$($proc.ProcessName)', not '$Name'"
            }
        }
        return $proc
    }
    if (-not $Name) { Fail-Closed "-TargetProcessName or -TargetPid is required" }
    $base = $Name -replace '\.exe$',''
    $matches = @(Get-Process -Name $base -ErrorAction SilentlyContinue)
    if ($matches.Count -eq 0) { Fail-Closed "no live process named '$base' (start the supervisor first; NO game needed)" }
    if ($matches.Count -gt 1) { Fail-Closed "ambiguous: $($matches.Count) processes named '$base' -- pass -TargetPid to disambiguate" }
    return $matches[0]
}

function Get-ProcessStartFiletime {
    param($Proc)
    try { return [long]$Proc.StartTime.ToFileTimeUtc() } catch { return $null }
}

# Sum raw cumulative context switches across the process's threads. WMI class +
# property names are locale-invariant. Returns $null if the perf class is
# unavailable -> caller marks context switches OWED (never fabricated).
function Get-ProcessContextSwitches {
    param([int]$ProcId)
    try {
        # Server-side WQL filter by IDProcess -> only the target's threads are
        # returned (fast; the unfiltered class enumerates every system thread).
        $threads = @(Get-CimInstance -Query "SELECT ContextSwitchesPersec,IDProcess FROM Win32_PerfRawData_PerfProc_Thread WHERE IDProcess=$ProcId" -ErrorAction Stop)
        if ($threads.Count -eq 0) { return [long]0 }
        $sum = [long]0
        foreach ($t in $threads) { $sum += [long]$t.ContextSwitchesPersec }
        return $sum
    } catch { return $null }
}

function Take-Sample {
    param([int]$Index, [double]$MonotonicSeconds, [int]$ProcId, [long]$ExpectedStartFiletime, [bool]$ContextAvailable)
    # start_time_filetime is present on EVERY sample (0 when unreadable/absent) so
    # the grader can require it universally and never silently skip attribution.
    $sample = @{
        index             = [long]$Index
        monotonic_seconds = [math]::Round($MonotonicSeconds, 3)
        start_time_filetime = [long]0
    }
    $proc = $null
    try { $proc = Get-Process -Id $ProcId -ErrorAction Stop } catch { $proc = $null }
    if ($null -eq $proc) {
        $sample["process_present"] = $false
        return $sample
    }
    $startFt = Get-ProcessStartFiletime -Proc $proc
    if ($null -eq $startFt -or $startFt -ne $ExpectedStartFiletime) {
        # PID recycled to a different process, or start time unreadable ->
        # unattributable. Record the fact; the grader fails closed on it.
        $sample["process_present"] = $false
        $sample["start_time_filetime"] = $(if ($null -eq $startFt) { [long]0 } else { $startFt })
        return $sample
    }
    $sample["process_present"] = $true
    $sample["start_time_filetime"] = [long]$startFt
    $sample["cpu_seconds"] = [math]::Round($proc.TotalProcessorTime.TotalSeconds, 6)
    $sample["private_bytes"] = [long]$proc.PrivateMemorySize64
    if ($ContextAvailable) {
        $cs = Get-ProcessContextSwitches -ProcId $ProcId
        if ($null -ne $cs) { $sample["context_switches"] = [long]$cs }
    }
    return $sample
}

# --------------------------------------------------------------------- MEASURE

function Invoke-Measure {
    if (-not $State) { Fail-Closed "-State is required (one of: $($script:ValidStates -join ', '))" }
    $expectedToken = "MEASURE_AUTO_ATTACH_IDLE_COST_$State"
    if ($ActionToken -ne $expectedToken) { Fail-Closed "action token mismatch: expected '$expectedToken'" }
    if (-not $OutReport) { Fail-Closed "-OutReport is required" }
    # The durable attempt seal is mandatory end to end: a report with no ledger seal
    # can never be graded (the grader requires it), so refuse to produce one.
    if (-not $NonceLedgerRoot) { Fail-Closed "-NonceLedgerRoot is required (the sealed report must be ledgered so it can be graded)" }
    if ($WindowSeconds -lt 1) { Fail-Closed "-WindowSeconds must be >= 1" }
    if ($SampleIntervalSeconds -lt 1) { Fail-Closed "-SampleIntervalSeconds must be >= 1" }
    if ($SampleIntervalSeconds -ge $WindowSeconds) { Fail-Closed "-SampleIntervalSeconds must be strictly less than -WindowSeconds (interval >= window yields a degenerate 2-sample series that cannot support the cadence/series checks)" }

    $nonce = $AttemptNonce
    if (-not $nonce) {
        $rng = New-Object byte[] 32
        [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($rng)
        $nonce = ([System.BitConverter]::ToString($rng)).Replace("-", "").ToLowerInvariant()
    }
    if (-not $script:Hex64.IsMatch($nonce)) { Fail-Closed "-AttemptNonce must be 64 lowercase hex chars" }

    $proc = Resolve-TargetProcess -Name $TargetProcessName -ProcId $TargetPid
    $ProcId = [int]$proc.Id
    $startFt = Get-ProcessStartFiletime -Proc $proc
    if ($null -eq $startFt) { Fail-Closed "cannot read StartTime of PID $ProcId (needed for recycle attribution)" }
    $exePath = ""
    try { $exePath = [string]$proc.Path } catch { $exePath = "" }

    # Probe context-switch availability up front; if the perf class is missing it
    # is marked OWED for the whole run rather than faked per-sample.
    $contextProbe = Get-ProcessContextSwitches -ProcId $ProcId
    $contextAvailable = ($null -ne $contextProbe)

    # ceil (not floor): for an interval that does not divide the window exactly
    # (e.g. 17s into 1800s), floor would stop the series one interval short of the
    # window edge (last sample at 1785s), false-failing window_complete on a
    # LEGITIMATE run. ceil adds the final sample, which the min(i*interval, window)
    # schedule caps exactly at the window edge (a short final gap). For a divisible
    # interval (the default 15s/1800s) ceil == floor, so the default is unchanged.
    $expectedSampleCount = [long]([math]::Ceiling($WindowSeconds / $SampleIntervalSeconds)) + 1

    Write-Host "[idle-cost] MEASURE state=$State target=$($proc.ProcessName) pid=$ProcId window=${WindowSeconds}s interval=${SampleIntervalSeconds}s (self-timed; NO game needed; safe to walk away)"
    if (-not $contextAvailable) { Write-Host "[idle-cost] NOTE: context-switch perf class unavailable -> that metric will be marked OWED (not faked)." }

    # Take EXACTLY $expectedSampleCount samples on a fixed wall-clock schedule
    # (sample i targeted at min(i*interval, window), last at the window edge). The
    # count is deterministic regardless of per-sample cost, so a truncated report
    # is detectable; if a sample overruns its slot the schedule still spans the
    # full window (elapsed >= window), which is the genuine short-window guard.
    $samples = New-Object System.Collections.ArrayList
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $startedUtc = (Get-Date).ToUniversalTime().ToString("o")
    for ($idx = 0; $idx -lt $expectedSampleCount; $idx++) {
        $targetAt = [math]::Min([double]$idx * $SampleIntervalSeconds, [double]$WindowSeconds)
        $sleep = $targetAt - $sw.Elapsed.TotalSeconds
        if ($sleep -gt 0) { Start-Sleep -Milliseconds ([int][math]::Round($sleep * 1000.0)) }
        $sample = Take-Sample -Index $idx -MonotonicSeconds ($sw.Elapsed.TotalSeconds) -ProcId $ProcId -ExpectedStartFiletime $startFt -ContextAvailable $contextAvailable
        [void]$samples.Add($sample)
    }
    $sw.Stop()
    $endedUtc = (Get-Date).ToUniversalTime().ToString("o")

    $present = @($samples | Where-Object { $_["process_present"] -eq $true })

    # Keep the per-sample context metric UNIFORM: if the perf class was available
    # at probe time but any present sample failed to read it, downgrade the WHOLE
    # run's context evidence to OWED and strip the partial values (so a present
    # sample never carries some metrics but not others; never fabricate a gap).
    if ($contextAvailable) {
        $missingCtx = @($present | Where-Object { -not $_.ContainsKey("context_switches") })
        if ($missingCtx.Count -gt 0) {
            $contextAvailable = $false
            foreach ($s in $samples) { if ($s.ContainsKey("context_switches")) { [void]$s.Remove("context_switches") } }
        }
    }

    # Elapsed is the recomputable span of the sample timestamps (first..last), so
    # the grader re-derives the exact same number -- never a free-floating wall
    # clock the report could overstate.
    $elapsedTotal = 0.0
    if ($samples.Count -ge 1) {
        $elapsedTotal = [math]::Round(([double]$samples[$samples.Count-1]["monotonic_seconds"] - [double]$samples[0]["monotonic_seconds"]), 3)
    }

    # Human-readable summary ONLY -- graded quantities are RE-DERIVED by the grader
    # from the samples via this same function, then cross-checked against this.
    $deltas = Compute-SampleDeltas -PresentSamples $present -Elapsed $elapsedTotal -ContextAvailable $contextAvailable

    $metricsAvailable = @("cpu_time", "private_bytes")
    if ($contextAvailable) { $metricsAvailable += "context_switches" }
    $metricsOwed = @{
        "snapshot_count" = "supervisor-internal reconcile count; no external counter emitted as-is (journal.rs records only external state actions; run_supervisor prints nothing). Needs a supervisor idle-cost telemetry emit (activation-frozen)."
        "wmi_reconnects" = "supervisor-internal counter; no external surface as-is. section 10 gives no numeric bound (record only). Needs the same telemetry emit."
    }
    if (-not $contextAvailable) {
        $metricsOwed["context_switches"] = "Win32_PerfRawData_PerfProc_Thread perf class unavailable on this host; not faked."
    }

    $report = @{
        schema = $script:ReportSchema
        attempt_nonce = $nonce
        state = $State
        tool = @{
            script_path = [string]$PSCommandPath
            script_sha256 = (Get-Sha256Hex ([System.IO.File]::ReadAllText($PSCommandPath)))
        }
        target = @{
            process_name = [string]$proc.ProcessName
            pid = [long]$ProcId
            exe_path = $exePath
            start_time_filetime = [long]$startFt
        }
        window = @{
            requested_seconds = [long]$WindowSeconds
            sample_interval_seconds = [long]$SampleIntervalSeconds
            expected_sample_count = [long]$expectedSampleCount
            started_utc = $startedUtc
            ended_utc = $endedUtc
            monotonic_elapsed_seconds = $elapsedTotal
        }
        metrics_available = $metricsAvailable
        metrics_owed = $metricsOwed
        derived_bounds = @{
            snapshot_count_max = [long]$script:DerivedSnapshotMax[$State]
            snapshot_count_basis = $script:DerivedSnapshotBasis
        }
        samples = @($samples.ToArray())
        deltas = $deltas
        report_sha256 = ""
    }
    $report["report_sha256"] = Get-DocumentDigest -Document $report -HashField "report_sha256"

    Write-CanonicalFile -Path $OutReport -Value $report

    if ($NonceLedgerRoot) {
        if (-not (Test-Path -LiteralPath $NonceLedgerRoot)) { New-Item -ItemType Directory -Force -Path $NonceLedgerRoot | Out-Null }
        $sealPath = Join-Path $NonceLedgerRoot ("$nonce.seal.json")
        if (Test-Path -LiteralPath $sealPath) { Fail-Closed "seal for nonce $nonce already exists (attempt nonce must be fresh)" }
        $seal = @{
            schema = $script:SealSchema
            attempt_nonce = $nonce
            state = $State
            report_path = [string]((Resolve-Path -LiteralPath $OutReport).Path)
            report_sha256 = $report["report_sha256"]
            sealed_utc = (Get-Date).ToUniversalTime().ToString("o")
        }
        Write-CanonicalFile -Path $sealPath -Value $seal
    }

    Write-Host "[idle-cost] sealed report -> $OutReport (sha256 $($report["report_sha256"]))"
    Write-Host "[idle-cost] samples=$($report["samples"].Count) elapsed=${elapsedTotal}s. Run -Mode Grade for the verdict."
    return $report
}

# ----------------------------------------------------------------------- GRADE

function Read-SealedReport {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { Fail-Closed "report not found: $Path" }
    $len = (Get-Item -LiteralPath $Path).Length
    if ($len -lt 2) { Fail-Closed "report is empty/truncated" }
    if ($len -gt 33554432) { Fail-Closed "report absurdly large ($len bytes)" }
    $raw = [System.IO.File]::ReadAllText($Path)
    $obj = $null
    try { $obj = $raw | ConvertFrom-Json } catch { Fail-Closed "report is not valid JSON: $($_.Exception.Message)" }
    return (ConvertTo-PlainTree $obj)
}

function Invoke-Grade {
    if ($ActionToken -ne "GRADE_AUTO_ATTACH_IDLE_COST") { Fail-Closed "action token mismatch: expected 'GRADE_AUTO_ATTACH_IDLE_COST'" }
    if (-not $Report) { Fail-Closed "-Report is required" }
    if (-not $OutVerdict) { Fail-Closed "-OutVerdict is required" }
    # Finding 2: the nonce ledger is MANDATORY for a real grade -- an absent root
    # cannot be silently bypassed into a pass.
    if (-not $NonceLedgerRoot) { Fail-Closed "-NonceLedgerRoot is required (the durable attempt seal is mandatory; a grade cannot bypass it)" }

    $r = Read-SealedReport -Path $Report

    # ---- structural / deny-extra (finding 4: RECURSIVE) ----
    Assert-ExactKeys -Map $r -Where "report" -Allowed @(
        "schema","attempt_nonce","state","tool","target","window",
        "metrics_available","metrics_owed","derived_bounds","samples","deltas","report_sha256")
    if ($r["schema"] -ne $script:ReportSchema) { Fail-Closed "wrong report schema '$($r['schema'])'" }
    if (-not $script:Hex64.IsMatch([string]$r["attempt_nonce"])) { Fail-Closed "attempt_nonce not 64-hex" }
    if ($script:ValidStates -notcontains [string]$r["state"]) { Fail-Closed "invalid state '$($r['state'])'" }
    Assert-ExactKeys -Map $r["tool"] -Where "report.tool" -Allowed @("script_path","script_sha256")
    Assert-ExactKeys -Map $r["target"] -Where "report.target" -Allowed @("process_name","pid","exe_path","start_time_filetime")
    Assert-ExactKeys -Map $r["window"] -Where "report.window" -Allowed @(
        "requested_seconds","sample_interval_seconds","expected_sample_count","started_utc","ended_utc","monotonic_elapsed_seconds")
    Assert-ExactKeys -Map $r["derived_bounds"] -Where "report.derived_bounds" -Allowed @("snapshot_count_max","snapshot_count_basis")
    # metrics_owed: fixed pair, optional context entry only.
    foreach ($k in @("snapshot_count","wmi_reconnects")) { if (-not $r["metrics_owed"].ContainsKey($k)) { Fail-Closed "report.metrics_owed missing '$k'" } }
    foreach ($k in $r["metrics_owed"].Keys) { if (@("snapshot_count","wmi_reconnects","context_switches") -notcontains $k) { Fail-Closed "report.metrics_owed unexpected key '$k' (deny-extra)" } }
    foreach ($m in @($r["metrics_available"])) { if (@("cpu_time","private_bytes","context_switches") -notcontains [string]$m) { Fail-Closed "report.metrics_available unexpected entry '$m'" } }
    $contextAvailable = (@($r["metrics_available"]) -contains "context_switches")
    # deltas: only known summary keys allowed (deny-extra, but members are OPTIONAL
    # -- a churn/OWED run omits the context total); values are re-derived below.
    Assert-NoExtraKeys -Map $r["deltas"] -Where "report.deltas" -Allowed @(
        "cpu_seconds","cpu_core_fraction","cpu_monotonic_nondecreasing","cpu_max_interval_core_fraction",
        "private_bytes_start","private_bytes_end",
        "private_bytes_net_growth","private_bytes_strictly_monotonic_increasing",
        "context_switches_total","context_switches_thread_churn")

    # ---- seal_intact ----
    $storedHash = [string]$r["report_sha256"]
    $recomputed = Get-DocumentDigest -Document $r -HashField "report_sha256"
    $sealIntact = ($storedHash -eq $recomputed) -and $script:Hex64.IsMatch($storedHash)

    # ---- ledger cross-check (mandatory) ----
    $ledgerChecked = $true
    $ledgerSealMatches = $false
    $sealPath = Join-Path $NonceLedgerRoot ("$($r['attempt_nonce']).seal.json")
    if (Test-Path -LiteralPath $sealPath) {
        $seal = ConvertTo-PlainTree ([System.IO.File]::ReadAllText($sealPath) | ConvertFrom-Json)
        $ledgerSealMatches = ($seal["schema"] -eq $script:SealSchema) -and
            ($seal["attempt_nonce"] -eq $r["attempt_nonce"]) -and
            ($seal["report_sha256"] -eq $storedHash) -and
            ($seal["state"] -eq $r["state"])
    }

    $window = $r["window"]
    $target = $r["target"]
    $samples = @($r["samples"])
    $interval = [long]$window["sample_interval_seconds"]
    if ($interval -lt 1) { Fail-Closed "window.sample_interval_seconds must be >= 1" }
    # Reject the degenerate interval >= window (Grade must enforce Measure's own upper
    # bound): such a report is only a 2-sample first/last pair, which cannot support the
    # cadence / full-series checks. Measure rejects interval >= window; a re-sealed report
    # claiming it must fail closed here too.
    if ($interval -ge [long]$window["requested_seconds"]) { Fail-Closed "window.sample_interval_seconds ($interval) must be strictly less than requested window ($($window['requested_seconds'])) (degenerate 2-sample series)" }

    # ---- per-sample deny-extra + universal FILETIME (finding 4) ----
    $attributable = $true
    $attrReasons = @()
    $expStart = [long]$target["start_time_filetime"]
    if ($samples.Count -eq 0) { $attributable = $false; $attrReasons += "no samples" }
    $baseSampleKeys = @("index","monotonic_seconds","process_present","start_time_filetime")
    foreach ($s in $samples) {
        # Every sample MUST carry the base keys (incl. start_time_filetime -- never
        # skipped) and no unexpected key.
        $allowed = @($baseSampleKeys)
        if ($s.ContainsKey("process_present") -and $s["process_present"] -eq $true) {
            $allowed += @("cpu_seconds","private_bytes")
            if ($contextAvailable) { $allowed += "context_switches" }
        }
        Assert-ExactKeys -Map $s -Where "report.samples[$($s['index'])]" -Allowed $allowed
        if ($s["process_present"] -ne $true) { $attributable = $false; $attrReasons += "sample $($s['index']) not attributable (process absent or PID recycled)"; break }
        if ([long]$s["start_time_filetime"] -ne $expStart) {
            $attributable = $false; $attrReasons += "sample $($s['index']) start_time mismatch (PID recycled)"; break
        }
    }

    # ---- RE-DERIVE every graded quantity from the raw samples (finding 3) ----
    # recElapsed is the authoritative window span (first..last timestamp), never a
    # trusted stored field.
    $recElapsed = 0.0
    if ($samples.Count -ge 1) {
        $recElapsed = [math]::Round(([double]$samples[$samples.Count-1]["monotonic_seconds"] - [double]$samples[0]["monotonic_seconds"]), 3)
    }
    $present = @($samples | Where-Object { $_["process_present"] -eq $true })
    $recDeltas = Compute-SampleDeltas -PresentSamples $present -Elapsed $recElapsed -ContextAvailable $contextAvailable

    # ---- FULL-SERIES structure (round 2, finding 1): sequential indexes,
    # strictly-increasing timestamps, and every consecutive gap on-cadence. Catches
    # re-sealed inputs whose endpoints look right but whose interior is fabricated
    # (all-zero middle timestamps, all-zero indexes, clustered-then-jumped gaps).
    $seriesReasons = @()
    $cadenceTol = $CadenceToleranceSeconds
    if ($cadenceTol -le 0) { $cadenceTol = [math]::Max(2.0, $interval / 2.0) }
    $reqWindow = [double]$window["requested_seconds"]
    for ($i = 0; $i -lt $samples.Count; $i++) {
        if ([long]$samples[$i]["index"] -ne [long]$i) { $seriesReasons += "sample position $i has non-sequential index $($samples[$i]['index'])"; break }
    }
    for ($i = 1; $i -lt $samples.Count; $i++) {
        $gap = [double]$samples[$i]["monotonic_seconds"] - [double]$samples[$i-1]["monotonic_seconds"]
        if ($gap -le 0) { $seriesReasons += "monotonic_seconds not strictly increasing at sample $i (gap ${gap}s)"; break }
        # Compare against the SCHEDULED gap, not the raw interval: the measurer targets
        # min(i*interval, window), so the final capped sample has a SHORT scheduled gap
        # (window - (i-1)*interval) for a non-divisible interval. Using the raw interval
        # here false-failed a legitimate capped final gap (e.g. 23s interval -> 6s final
        # gap). For every non-final sample the scheduled gap equals the interval, so this
        # does not weaken the check (accumulated drift is still caught by the cumulative
        # absolute-schedule check below).
        $schedGap = [math]::Min([double]$i * $interval, $reqWindow) - [math]::Min([double]($i - 1) * $interval, $reqWindow)
        if ([math]::Abs($gap - $schedGap) -gt $cadenceTol) { $seriesReasons += "off-cadence gap at sample $i (${gap}s vs scheduled ${schedGap}s, tolerance ${cadenceTol}s)"; break }
    }
    # CUMULATIVE schedule check (re-gate: per-gap tolerance alone permits accumulated
    # phase drift — e.g. 60 short gaps then 60 long gaps, each individually in-band,
    # leaving the midpoint hundreds of seconds off schedule). The measurer targets
    # ABSOLUTE marks (sample i at min(i*interval, window), Measure loop ~:539-545), so
    # each sample must land within cadenceTol of its absolute expected offset from the
    # first sample; drift cannot accumulate for a genuine monotonic-stopwatch run.
    $firstMono = [double]$samples[0]["monotonic_seconds"]
    $reqWindow = [double]$window["requested_seconds"]
    for ($i = 0; $i -lt $samples.Count; $i++) {
        $expectedOffset = [math]::Min([double]$i * $interval, $reqWindow)
        $actualOffset = [double]$samples[$i]["monotonic_seconds"] - $firstMono
        if ([math]::Abs($actualOffset - $expectedOffset) -gt $cadenceTol) {
            $seriesReasons += "cumulative schedule drift at sample $i (offset $([math]::Round($actualOffset,3))s vs expected ${expectedOffset}s, tolerance ${cadenceTol}s)"
            break
        }
    }

    # Cross-check every stored summary field against the re-derived value; any
    # divergence is a fabricated/stale summary => fail-closed.
    $crossReasons = @()
    $crossReasons += $seriesReasons
    $stored = $r["deltas"]
    foreach ($k in $recDeltas.Keys) {
        if ($stored.ContainsKey($k)) {
            $sv = $stored[$k]; $rv = $recDeltas[$k]
            $eq = $false
            if (($rv -is [bool]) -or ($sv -is [bool])) { $eq = ([bool]$sv -eq [bool]$rv) }
            else { $eq = (Test-CanonicalNumberEqual $sv $rv) }
            if (-not $eq) { $crossReasons += "deltas.$k stored=$sv != recomputed=$rv" }
        } else {
            $crossReasons += "deltas.$k missing from report (recomputed=$($recDeltas[$k]))"
        }
    }
    # And reject stored delta fields the re-derivation did NOT produce (e.g. a
    # stored context_switches_total when the samples show churn / a negative diff).
    foreach ($k in $stored.Keys) {
        if (-not $recDeltas.ContainsKey($k)) { $crossReasons += "deltas.$k present in report but not derivable from samples (fabricated)" }
    }
    # Explicitly reject any stored negative context-switch total (finding 5).
    if ($stored.ContainsKey("context_switches_total") -and [long]$stored["context_switches_total"] -lt 0) {
        $crossReasons += "deltas.context_switches_total is negative (invalid; churn/wrap must be OWED, not a stored count)"
    }
    # Stored elapsed / expected_sample_count must match the recomputation.
    if (-not (Test-CanonicalNumberEqual $window["monotonic_elapsed_seconds"] $recElapsed)) {
        $crossReasons += "window.monotonic_elapsed_seconds stored=$($window['monotonic_elapsed_seconds']) != recomputed=$recElapsed"
    }
    $storedExpected = [long]$window["expected_sample_count"]
    if ($storedExpected -ne [long]$samples.Count) {
        $crossReasons += "window.expected_sample_count stored=$storedExpected != actual sample count $($samples.Count)"
    }
    # The cadence formula the measurer used must reproduce the actual count
    # (ceil so a non-divisible interval reaches the window edge; see Measure).
    $reqSeconds = [long]$window["requested_seconds"]
    $cadenceExpected = [long]([math]::Ceiling($reqSeconds / $interval)) + 1
    if ($cadenceExpected -ne [long]$samples.Count) {
        $crossReasons += "sample count $($samples.Count) != cadence ceil($reqSeconds/$interval)+1 = $cadenceExpected"
    }
    $crossConsistent = ($crossReasons.Count -eq 0)

    # ---- window_complete: authoritative REAL-duration gate (finding 1) ----
    # No slack that admits a short window; a dry run (recElapsed < 1800) can never
    # satisfy the gate.
    $windowComplete = $true
    $windowReasons = @()
    if ($recElapsed -lt $script:GateWindowSeconds) {
        $windowComplete = $false
        $windowReasons += "recomputed window span ${recElapsed}s < required gate $($script:GateWindowSeconds)s (dry-run/short window cannot satisfy the gate)"
    }
    if ($reqSeconds -lt $script:GateWindowSeconds) {
        $windowComplete = $false
        $windowReasons += "report requested_seconds=$reqSeconds < gate $($script:GateWindowSeconds)s"
    }

    # ---- bound: cpu busy loop -- RE-DERIVED, graded on the MAX per-interval rate
    # (round 2, finding 2): an intermediate busy spike that averages out cannot
    # hide, and any intermediate decrease of the cumulative counter is impossible
    # -> FAIL.
    $cpuBound = @{ metric = "cpu_busy_loop"; source = "AUTO_ATTACH_DESIGN.md:421"; ceiling_core_fraction = $BusyLoopCoreFractionCeiling; ceiling_basis = "derived idle envelope (design sparse cadence, policy.rs); NOT a section-10 literal; override -BusyLoopCoreFractionCeiling" }
    if ($recDeltas.ContainsKey("cpu_core_fraction")) {
        $cf = [double]$recDeltas["cpu_core_fraction"]
        $cpuMax = [double]$recDeltas["cpu_max_interval_core_fraction"]
        $cpuMono = [bool]$recDeltas["cpu_monotonic_nondecreasing"]
        $cpuBound["core_fraction"] = $cf
        $cpuBound["max_interval_core_fraction"] = $cpuMax
        $cpuBound["cpu_monotonic_nondecreasing"] = $cpuMono
        $cpuBound["recomputed_from_samples"] = $true
        if (-not $cpuMono) {
            $cpuBound["verdict"] = "FAIL"; $cpuBound["detail"] = "TotalProcessorTime decreased mid-series (impossible for a cumulative counter)"
        } elseif ($cpuMax -gt $BusyLoopCoreFractionCeiling) {
            $cpuBound["verdict"] = "FAIL"; $cpuBound["detail"] = "max per-interval core fraction $cpuMax exceeds busy-loop ceiling $BusyLoopCoreFractionCeiling (avg $cf)"
        } else { $cpuBound["verdict"] = "PASS" }
    } else { $cpuBound["verdict"] = "FAIL"; $cpuBound["detail"] = "no CPU delta derivable from samples (fail-closed)" }

    # ---- bound: private-bytes unbounded growth -- graded on RE-DERIVED series ----
    $memBound = @{ metric = "private_bytes_unbounded_growth"; source = "AUTO_ATTACH_DESIGN.md:421"; ceiling_bytes = $UnboundedGrowthByteCeiling; ceiling_basis = "derived idle envelope; NOT a section-10 literal; override -UnboundedGrowthByteCeiling" }
    if ($recDeltas.ContainsKey("private_bytes_net_growth")) {
        $growth = [long]$recDeltas["private_bytes_net_growth"]
        $mono = [bool]$recDeltas["private_bytes_strictly_monotonic_increasing"]
        $memBound["net_growth"] = $growth
        $memBound["strictly_monotonic_increasing"] = $mono
        $memBound["recomputed_from_samples"] = $true
        if ($mono -or $growth -gt $UnboundedGrowthByteCeiling) {
            $memBound["verdict"] = "FAIL"
            $memBound["detail"] = "unbounded growth: net_growth=$growth strictly_monotonic=$mono"
        } else { $memBound["verdict"] = "PASS" }
    } else { $memBound["verdict"] = "FAIL"; $memBound["detail"] = "no private-bytes series derivable from samples (fail-closed)" }

    # ---- OWED + RECORDED metrics (context switches from RE-DERIVED total) ----
    $snapshotBound = @{
        metric = "snapshot_count"
        verdict = "OWED"
        derived_max = [long]$r["derived_bounds"]["snapshot_count_max"]
        derived_basis = [string]$r["derived_bounds"]["snapshot_count_basis"]
        detail = [string]$r["metrics_owed"]["snapshot_count"]
        corroboration = "not directly counted; a runaway snapshot loop would surface as cpu_busy_loop=FAIL (currently '$($cpuBound['verdict'])')"
    }
    $wmiBound = @{ metric = "wmi_reconnects"; verdict = "OWED"; detail = [string]$r["metrics_owed"]["wmi_reconnects"] }
    $csBound = @{ metric = "context_switches" }
    if (-not $contextAvailable) {
        $csBound["verdict"] = "OWED"
        $d = "context switches not sampled on this host"
        if ($r["metrics_owed"].ContainsKey("context_switches")) { $d = [string]$r["metrics_owed"]["context_switches"] }
        $csBound["detail"] = $d
    } elseif ($recDeltas.ContainsKey("context_switches_total")) {
        $csBound["verdict"] = "RECORDED"; $csBound["total"] = [long]$recDeltas["context_switches_total"]; $csBound["recomputed_from_samples"] = $true
    } else {
        # Any intermediate (or endpoint) decrease -> churn (round 2 extends finding
        # 5 to the whole series): OWED/invalid evidence, never a fabricated zero.
        $csBound["verdict"] = "OWED"; $csBound["detail"] = "context-switch counter decreased somewhere in the series (thread churn / wrap); no valid cumulative total -- OWED, not zero"
    }

    $cpuRecorded = @{ metric = "cpu_delta"; verdict = "RECORDED" }
    if ($recDeltas.ContainsKey("cpu_seconds")) { $cpuRecorded["cpu_seconds"] = [double]$recDeltas["cpu_seconds"] }
    $memRecorded = @{ metric = "private_bytes_delta"; verdict = "RECORDED" }
    if ($recDeltas.ContainsKey("private_bytes_net_growth")) { $memRecorded["net_growth"] = [long]$recDeltas["private_bytes_net_growth"] }

    # ---- overall verdict ----
    $failReasons = @()
    if (-not $sealIntact) { $failReasons += "seal not intact (report tampered or hash mismatch)" }
    if (-not $ledgerSealMatches) { $failReasons += "ledger seal missing or mismatched" }
    if (-not $crossConsistent) { $failReasons += ($crossReasons -join "; ") }
    if (-not $windowComplete) { $failReasons += ($windowReasons -join "; ") }
    if (-not $attributable) { $failReasons += ($attrReasons -join "; ") }
    if ($cpuBound["verdict"] -eq "FAIL") { $failReasons += "cpu_busy_loop FAIL: $($cpuBound['detail'])" }
    if ($memBound["verdict"] -eq "FAIL") { $failReasons += "private_bytes_unbounded_growth FAIL: $($memBound['detail'])" }

    $overall = "PASS_WITH_OWED"
    if ($failReasons.Count -gt 0) { $overall = "FAIL" }

    $verdict = @{
        schema = $script:VerdictSchema
        attempt_nonce = [string]$r["attempt_nonce"]
        state = [string]$r["state"]
        report_sha256 = $storedHash
        graded_utc = (Get-Date).ToUniversalTime().ToString("o")
        checks = @{
            seal_intact = $sealIntact
            ledger_checked = $ledgerChecked
            ledger_seal_matches = $ledgerSealMatches
            cross_field_consistent = $crossConsistent
            window_complete = $windowComplete
            target_attributable = $attributable
            sample_count = [long]$samples.Count
            recomputed_window_seconds = $recElapsed
            gate_window_seconds = [long]$script:GateWindowSeconds
        }
        bounds = @{
            snapshot_count = $snapshotBound
            cpu_busy_loop = $cpuBound
            private_bytes_unbounded_growth = $memBound
            wmi_reconnects = $wmiBound
            context_switches = $csBound
            cpu_delta = $cpuRecorded
            private_bytes_delta = $memRecorded
        }
        fail_reasons = @($failReasons)
        verdict = $overall
        verdict_sha256 = ""
    }
    $verdict["verdict_sha256"] = Get-DocumentDigest -Document $verdict -HashField "verdict_sha256"
    Write-CanonicalFile -Path $OutVerdict -Value $verdict

    Write-Host "[idle-cost] VERDICT=$overall (state=$($r['state']))  -> $OutVerdict"
    if ($failReasons.Count -gt 0) { foreach ($fr in $failReasons) { Write-Host "  FAIL: $fr" } }
    Write-Host "  snapshot_count: OWED (derived max $($snapshotBound['derived_max'])) | wmi_reconnects: OWED"
    return $verdict
}

# -------------------------------------------------------------------- SELF-TEST

# Recompute the report's internal summary (deltas, elapsed, expected count) from
# its own samples, then re-seal -- keeps a mutated report INTERNALLY CONSISTENT so
# a test isolates exactly one graded property.
function Rebuild-Report {
    param([hashtable]$Rep)
    $samples = @($Rep["samples"])
    $present = @($samples | Where-Object { $_["process_present"] -eq $true })
    $ctx = (@($Rep["metrics_available"]) -contains "context_switches")
    $elapsed = 0.0
    if ($samples.Count -ge 1) { $elapsed = [math]::Round(([double]$samples[$samples.Count-1]["monotonic_seconds"] - [double]$samples[0]["monotonic_seconds"]), 3) }
    $Rep["window"]["monotonic_elapsed_seconds"] = $elapsed
    $Rep["window"]["expected_sample_count"] = [long]$samples.Count
    $Rep["deltas"] = Compute-SampleDeltas -PresentSamples $present -Elapsed $elapsed -ContextAvailable $ctx
    $Rep["report_sha256"] = Get-DocumentDigest -Document $Rep -HashField "report_sha256"
    return $Rep
}

# Re-hash ONLY (leave any deliberately-injected inconsistency intact).
function Seal-Report {
    param([hashtable]$Rep)
    $Rep["report_sha256"] = Get-DocumentDigest -Document $Rep -HashField "report_sha256"
    return $Rep
}

# A full-gate-duration, within-bounds report: N samples on a fixed cadence spanning
# (N-1)*Interval seconds (default 121 @ 15 s = 1800 s = the real gate). Deltas are
# derived by the SAME function the grader uses, so cross-checks pass by construction.
function New-BaselineReport {
    param([string]$Nonce, [string]$St, [long]$ProcId, [long]$StartFt, [int]$N = 121, [int]$Interval = 15)
    $samples = @()
    for ($i = 0; $i -lt $N; $i++) {
        $samples += ,@{
            index = [long]$i
            monotonic_seconds = [math]::Round(($i * $Interval * 1.0), 3)
            process_present = $true
            start_time_filetime = [long]$StartFt
            cpu_seconds = [math]::Round((0.001 * $i), 6)              # ~0.12 CPU-s over the window
            private_bytes = [long](10485760 + ($i % 3) * 4096)       # bounded jitter, non-monotonic
            context_switches = [long](1000 + $i * 5)                  # steadily increasing
        }
    }
    $rep = @{
        schema = $script:ReportSchema
        attempt_nonce = $Nonce
        state = $St
        tool = @{ script_path = "selftest"; script_sha256 = (Get-Sha256Hex "selftest") }
        target = @{ process_name = "synthetic"; pid = [long]$ProcId; exe_path = ""; start_time_filetime = [long]$StartFt }
        window = @{
            requested_seconds = [long](($N - 1) * $Interval)
            sample_interval_seconds = [long]$Interval
            expected_sample_count = [long]$N
            started_utc = "2026-01-01T00:00:00.0000000Z"
            ended_utc = "2026-01-01T00:30:00.0000000Z"
            monotonic_elapsed_seconds = [double](($N - 1) * $Interval)
        }
        metrics_available = @("cpu_time","private_bytes","context_switches")
        metrics_owed = @{ snapshot_count = "owed-internal"; wmi_reconnects = "owed-internal" }
        derived_bounds = @{ snapshot_count_max = [long]$script:DerivedSnapshotMax[$St]; snapshot_count_basis = "selftest" }
        samples = $samples
        deltas = @{}
        report_sha256 = ""
    }
    return (Rebuild-Report $rep)
}

function Write-LedgerSeal {
    param([string]$LedgerRoot, [hashtable]$Rep)
    if (-not (Test-Path -LiteralPath $LedgerRoot)) { New-Item -ItemType Directory -Force -Path $LedgerRoot | Out-Null }
    $seal = @{
        schema = $script:SealSchema
        attempt_nonce = [string]$Rep["attempt_nonce"]
        state = [string]$Rep["state"]
        report_path = "selftest"
        report_sha256 = [string]$Rep["report_sha256"]
        sealed_utc = "2026-01-01T00:00:00.0000000Z"
    }
    Write-CanonicalFile -Path (Join-Path $LedgerRoot ("$($Rep['attempt_nonce']).seal.json")) -Value $seal
}

# Grade an in-memory report. By default a matching ledger seal is written to a temp
# ledger so structural tests isolate the OTHER property; -NoSeal omits the seal
# (missing-seal test); -NullLedger passes no ledger root (mandatory-ledger test).
function Grade-InMemory {
    param([hashtable]$Rep, [switch]$NoSeal, [switch]$NullLedger)
    $dir = Join-Path ([System.IO.Path]::GetTempPath()) ("ic_grade_" + [System.Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $tmp = Join-Path $dir "report.json"
    $out = Join-Path $dir "verdict.json"
    $led = Join-Path $dir "ledger"
    try {
        Write-CanonicalFile -Path $tmp -Value $Rep
        if (-not $NullLedger -and -not $NoSeal) { Write-LedgerSeal -LedgerRoot $led -Rep $Rep }
        $script:Report = $tmp; $script:OutVerdict = $out
        $script:ActionToken = "GRADE_AUTO_ATTACH_IDLE_COST"
        if ($NullLedger) { $script:NonceLedgerRoot = $null } else { $script:NonceLedgerRoot = $led }
        return (Invoke-Grade)
    } finally {
        Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Assert-True { param([bool]$Cond, [string]$Msg) if (-not $Cond) { throw "SELF-TEST FAILED: $Msg" } else { Write-Host "  ok: $Msg" } }
function Assert-Throws { param([scriptblock]$Block, [string]$Msg) $threw = $false; try { & $Block | Out-Null } catch { $threw = $true }; Assert-True $threw $Msg }

function Invoke-SelfTest {
    Write-Host "[idle-cost] SELF-TEST (fast synthetic windows + one live short window; NO game, NO supervisor)"
    $ft = 133000000000000000
    $sandbox = Join-Path ([System.IO.Path]::GetTempPath()) ("idlecost_selftest_" + [System.Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $sandbox | Out-Null
    $ledger = Join-Path $sandbox "ledger"
    New-Item -ItemType Directory -Force -Path $ledger | Out-Null
    try {
        # ============ MEASURE works (live, real short window) ============
        # live_measure_produces_sealed_ledgered_report
        $child = Start-Process -FilePath "powershell.exe" -ArgumentList @("-NoProfile","-Command","Start-Sleep -Seconds 60") -PassThru -WindowStyle Hidden
        Start-Sleep -Milliseconds 400
        $rep1Path = Join-Path $sandbox "measure.json"
        $nonce = ([System.BitConverter]::ToString((1..32 | ForEach-Object { [byte](Get-Random -Max 256) }))).Replace("-","").ToLowerInvariant()
        $script:Mode = "Measure"; $script:State = "healthy-wmi"; $script:TargetProcessName = ""
        $script:TargetPid = [int]$child.Id; $script:WindowSeconds = 5; $script:SampleIntervalSeconds = 1
        $script:OutReport = $rep1Path; $script:AttemptNonce = $nonce; $script:NonceLedgerRoot = $ledger
        $script:ActionToken = "MEASURE_AUTO_ATTACH_IDLE_COST_healthy-wmi"
        $liveReport = Invoke-Measure
        Assert-True ($liveReport["samples"].Count -eq $liveReport["window"]["expected_sample_count"]) "live_measure_produces_sealed_ledgered_report: fixed cadence sample count"
        Assert-True ($script:Hex64.IsMatch($liveReport["report_sha256"])) "live_measure_produces_sealed_ledgered_report: report sealed (64-hex)"
        Assert-True (Test-Path -LiteralPath (Join-Path $ledger "$nonce.seal.json")) "live_measure_produces_sealed_ledgered_report: ledger seal written"

        # FINDING 1 fail-before: dry_run_short_window_rejected
        $script:ActionToken = "GRADE_AUTO_ATTACH_IDLE_COST"; $script:Report = $rep1Path
        $script:OutVerdict = (Join-Path $sandbox "verdict.json"); $script:NonceLedgerRoot = $ledger
        $vShort = Invoke-Grade
        Assert-True ($vShort["verdict"] -eq "FAIL") "dry_run_short_window_rejected: 5s real window grades FAIL"
        Assert-True ($vShort["checks"]["window_complete"] -eq $false) "dry_run_short_window_rejected: window_complete FALSE"
        Assert-True ([double]$vShort["checks"]["recomputed_window_seconds"] -lt 1800) "dry_run_short_window_rejected: recomputed span < 1800s"
        try { Stop-Process -Id $child.Id -Force -ErrorAction SilentlyContinue } catch {}

        # ============ GREEN: full-duration synthetic passes ============
        # full_duration_synthetic_passes
        $good = New-BaselineReport -Nonce ("a"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        $vg = Grade-InMemory -Rep $good
        Assert-True ($vg["verdict"] -eq "PASS_WITH_OWED") "full_duration_synthetic_passes: 1800s synthetic grades PASS_WITH_OWED"
        Assert-True ($vg["checks"]["window_complete"] -eq $true) "full_duration_synthetic_passes: window_complete TRUE at 1800s"
        Assert-True ($vg["checks"]["cross_field_consistent"] -eq $true) "full_duration_synthetic_passes: cross_field_consistent TRUE"
        Assert-True ($vg["checks"]["target_attributable"] -eq $true) "full_duration_synthetic_passes: target_attributable TRUE"
        Assert-True ($vg["bounds"]["snapshot_count"]["verdict"] -eq "OWED") "full_duration_synthetic_passes: snapshot_count OWED"
        Assert-True ($vg["bounds"]["wmi_reconnects"]["verdict"] -eq "OWED") "full_duration_synthetic_passes: wmi_reconnects OWED"
        Assert-True ($vg["bounds"]["context_switches"]["verdict"] -eq "RECORDED") "full_duration_synthetic_passes: context_switches RECORDED (recomputed)"

        # ============ over-bound (graded from RE-DERIVED samples) ============
        # busy_loop_rejected -- spike CPU in the samples, rebuild consistently
        $busy = New-BaselineReport -Nonce ("b"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        $busy["samples"][-1]["cpu_seconds"] = [double]([double]$busy["samples"][0]["cpu_seconds"] + 1750.0)  # ~0.97 core over 1800s
        $busy = Rebuild-Report $busy
        $vb = Grade-InMemory -Rep $busy
        Assert-True ($vb["verdict"] -eq "FAIL") "busy_loop_rejected: grades FAIL"
        Assert-True ($vb["bounds"]["cpu_busy_loop"]["verdict"] -eq "FAIL") "busy_loop_rejected: cpu_busy_loop FAIL from recomputed fraction"

        # unbounded_growth_rejected -- strictly increasing private bytes
        $grow = New-BaselineReport -Nonce ("c"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        for ($i = 0; $i -lt $grow["samples"].Count; $i++) { $grow["samples"][$i]["private_bytes"] = [long](10485760 + $i * 1048576) }
        $grow = Rebuild-Report $grow
        $vgr = Grade-InMemory -Rep $grow
        Assert-True ($vgr["verdict"] -eq "FAIL") "unbounded_growth_rejected: grades FAIL"
        Assert-True ($vgr["bounds"]["private_bytes_unbounded_growth"]["verdict"] -eq "FAIL") "unbounded_growth_rejected: bound FAIL from recomputed series"

        # ============ FINDING 3: fabricated summaries rejected ============
        # fabricated_expected_sample_count_rejected (expected=1 with full samples + stale deltas)
        $fakeN = New-BaselineReport -Nonce ("d"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        $fakeN["window"]["expected_sample_count"] = [long]1
        $fakeN = Seal-Report $fakeN
        $vfn = Grade-InMemory -Rep $fakeN
        Assert-True ($vfn["verdict"] -eq "FAIL") "fabricated_expected_sample_count_rejected: grades FAIL"
        Assert-True ($vfn["checks"]["cross_field_consistent"] -eq $false) "fabricated_expected_sample_count_rejected: cross_field_consistent FALSE"

        # fabricated_cpu_delta_rejected (endpoint shows real busy CPU but stored deltas benign)
        $fakeD = New-BaselineReport -Nonce ("e"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        # Endpoint shows 200 CPU-s (> 0.10 core * 1800 s) while the stored deltas
        # stay benign (0.12 s). The grader recomputes from the samples: cross-check
        # catches the fabrication AND the busy-loop bound fires off the real value.
        $fakeD["samples"][-1]["cpu_seconds"] = [double]200.0
        $fakeD = Seal-Report $fakeD                          # stored deltas NOT recomputed -> stale/benign
        $vfd = Grade-InMemory -Rep $fakeD
        Assert-True ($vfd["verdict"] -eq "FAIL") "fabricated_cpu_delta_rejected: grades FAIL"
        Assert-True ($vfd["checks"]["cross_field_consistent"] -eq $false) "fabricated_cpu_delta_rejected: cross_field_consistent FALSE (stored != recomputed)"
        Assert-True (([double]$vfd["bounds"]["cpu_delta"]["cpu_seconds"]) -eq 200) "fabricated_cpu_delta_rejected: recorded cpu delta comes from samples (200), not stored (0.12)"
        Assert-True ($vfd["bounds"]["cpu_busy_loop"]["verdict"] -eq "FAIL") "fabricated_cpu_delta_rejected: cpu graded from RECOMPUTED samples = FAIL"

        # ============ FINDING 4: recursive deny-extra + per-sample FILETIME ============
        # nested_extra_key_rejected (window.unexpected_nested_key)
        $nest = New-BaselineReport -Nonce ("f"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        $nest["window"]["unexpected_nested_key"] = "boom"
        $nest = Seal-Report $nest
        Assert-Throws { Grade-InMemory -Rep $nest } "nested_extra_key_rejected: window nested extra key fails closed"

        # missing_per_sample_filetime_rejected
        $noft = New-BaselineReport -Nonce ("0"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        [void]$noft["samples"][3].Remove("start_time_filetime")
        $noft = Seal-Report $noft
        Assert-Throws { Grade-InMemory -Rep $noft } "missing_per_sample_filetime_rejected: sample without FILETIME fails closed"

        # ============ FINDING 5: negative context switch -> OWED not zero ============
        # negative_context_switch_is_owed_not_zero (churn: last < first)
        $churn = New-BaselineReport -Nonce ("1"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        $churn["samples"][-1]["context_switches"] = [long]0   # endpoint below start -> negative diff
        $churn = Rebuild-Report $churn
        $vc = Grade-InMemory -Rep $churn
        Assert-True ($vc["bounds"]["context_switches"]["verdict"] -eq "OWED") "negative_context_switch_is_owed_not_zero: context_switches OWED"
        Assert-True (-not $vc["bounds"]["context_switches"].ContainsKey("total")) "negative_context_switch_is_owed_not_zero: NO fabricated total"
        Assert-True ($vc["verdict"] -eq "PASS_WITH_OWED") "negative_context_switch_is_owed_not_zero: churn is evidence, not a gate FAIL"

        # negative_stored_context_switch_rejected (re-sealed total=-1 with benign samples)
        $negStored = New-BaselineReport -Nonce ("2"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        $negStored["deltas"]["context_switches_total"] = [long](-1)
        $negStored = Seal-Report $negStored
        $vns = Grade-InMemory -Rep $negStored
        Assert-True ($vns["verdict"] -eq "FAIL") "negative_stored_context_switch_rejected: stored negative total grades FAIL"
        Assert-True ($vns["checks"]["cross_field_consistent"] -eq $false) "negative_stored_context_switch_rejected: cross_field_consistent FALSE"

        # ==== ROUND 2 finding 1: FULL-SERIES structure (endpoints look right) ====
        # flat_then_jump_timestamps_rejected (samples 1..119 monotonic=0; first=0,last=1800)
        $flat = New-BaselineReport -Nonce ("3"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        for ($i = 1; $i -lt ($flat["samples"].Count - 1); $i++) { $flat["samples"][$i]["monotonic_seconds"] = [double]0 }
        $flat = Rebuild-Report $flat
        $vflat = Grade-InMemory -Rep $flat
        Assert-True ($vflat["verdict"] -eq "FAIL") "flat_then_jump_timestamps_rejected: grades FAIL"
        Assert-True ($vflat["checks"]["cross_field_consistent"] -eq $false) "flat_then_jump_timestamps_rejected: cross_field_consistent FALSE (non-increasing timestamps)"
        Assert-True ([double]$vflat["checks"]["recomputed_window_seconds"] -eq 1800) "flat_then_jump_timestamps_rejected: endpoints still span 1800s (interior was the tell)"

        # nonsequential_index_rejected (every index=0)
        $idx0 = New-BaselineReport -Nonce ("4"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        for ($i = 0; $i -lt $idx0["samples"].Count; $i++) { $idx0["samples"][$i]["index"] = [long]0 }
        $idx0 = Rebuild-Report $idx0
        $vidx = Grade-InMemory -Rep $idx0
        Assert-True ($vidx["verdict"] -eq "FAIL") "nonsequential_index_rejected: grades FAIL"
        Assert-True ($vidx["checks"]["cross_field_consistent"] -eq $false) "nonsequential_index_rejected: cross_field_consistent FALSE"

        # off_cadence_gap_rejected (one middle gap far from interval; still strictly increasing)
        $offc = New-BaselineReport -Nonce ("5"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        $offc["samples"][60]["monotonic_seconds"] = [double]([double]$offc["samples"][59]["monotonic_seconds"] + 0.5)
        $offc = Rebuild-Report $offc
        $voff = Grade-InMemory -Rep $offc
        Assert-True ($voff["verdict"] -eq "FAIL") "off_cadence_gap_rejected: grades FAIL"
        Assert-True ($voff["checks"]["cross_field_consistent"] -eq $false) "off_cadence_gap_rejected: cross_field_consistent FALSE (off-cadence gap)"

        # cumulative_phase_drift_rejected (re-gate: EVERY gap within +/-7.5s tolerance,
        # but 60 short (7.6s) gaps then 60 long (22.4s) gaps leave the midpoint 444s off
        # schedule; endpoints still span 1800s. Per-gap check passes; only the cumulative
        # absolute-schedule check catches it.)
        $drift = New-BaselineReport -Nonce ("f"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        for ($i = 0; $i -lt $drift["samples"].Count; $i++) {
            if ($i -le 60) { $mono = [double]$i * 7.6 } else { $mono = 456.0 + [double]($i - 60) * 22.4 }
            $drift["samples"][$i]["monotonic_seconds"] = [double]$mono
        }
        $drift = Rebuild-Report $drift
        $vdrift = Grade-InMemory -Rep $drift
        Assert-True ($vdrift["verdict"] -eq "FAIL") "cumulative_phase_drift_rejected: clustered-then-jumped series grades FAIL"
        Assert-True ($vdrift["checks"]["cross_field_consistent"] -eq $false) "cumulative_phase_drift_rejected: cross_field_consistent FALSE (cumulative schedule drift)"
        Assert-True ([double]$vdrift["checks"]["recomputed_window_seconds"] -eq 1800) "cumulative_phase_drift_rejected: endpoints still span 1800s (interior drift was the tell)"

        # nondivisible_interval_reaches_window_edge (re-gate: a LEGITIMATE 17s interval
        # that does not divide 1800 must still reach the window edge via ceil + the
        # capped final sample, and PASS — not false-FAIL window_complete. Mirrors real
        # Measure output: 107 samples, 0..105 at i*17 (0..1785), sample 106 capped at 1800.)
        $ndInterval = 17
        $ndWindow = 1800
        $ndN = [int]([math]::Ceiling($ndWindow / $ndInterval)) + 1
        $nd = New-BaselineReport -Nonce ("a"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft -N $ndN -Interval $ndInterval
        $nd["window"]["requested_seconds"] = [long]$ndWindow
        $nd["samples"][$ndN-1]["monotonic_seconds"] = [double]$ndWindow
        $nd = Rebuild-Report $nd
        $vnd = Grade-InMemory -Rep $nd
        Assert-True ($vnd["verdict"] -eq "PASS_WITH_OWED") "nondivisible_interval_reaches_window_edge: legitimate 17s interval grades PASS_WITH_OWED (not false-FAIL)"
        Assert-True ($vnd["checks"]["window_complete"] -eq $true) "nondivisible_interval_reaches_window_edge: window_complete TRUE (final sample capped at window edge)"
        Assert-True ($vnd["checks"]["cross_field_consistent"] -eq $true) "nondivisible_interval_reaches_window_edge: cross_field_consistent TRUE (ceil count + on-cadence + cumulative all pass)"

        # nondivisible_interval_short_final_gap (re-gate: 23s interval -> final capped gap
        # is only 6s (1800-78*23=1800-1794); the per-gap check must accept it as the
        # SCHEDULED gap, not reject it against the full 23s interval)
        $sgInterval = 23
        $sgWindow = 1800
        $sgN = [int]([math]::Ceiling($sgWindow / $sgInterval)) + 1
        $sg = New-BaselineReport -Nonce ("b"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft -N $sgN -Interval $sgInterval
        $sg["window"]["requested_seconds"] = [long]$sgWindow
        $sg["samples"][$sgN-1]["monotonic_seconds"] = [double]$sgWindow
        $sg = Rebuild-Report $sg
        $vsg = Grade-InMemory -Rep $sg
        Assert-True ($vsg["verdict"] -eq "PASS_WITH_OWED") "nondivisible_interval_short_final_gap: 23s interval (6s final capped gap) grades PASS_WITH_OWED"
        Assert-True ($vsg["checks"]["cross_field_consistent"] -eq $true) "nondivisible_interval_short_final_gap: cross_field_consistent TRUE (scheduled final gap accepted, not vs full interval)"

        # degenerate_interval_ge_window_rejected (re-gate: interval >= window yields only a
        # 2-sample first/last pair that cannot support the cadence/series checks; Measure
        # rejects it and Grade must too, fail-closed)
        $deg = New-BaselineReport -Nonce ("c"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft -N 2 -Interval 1800
        $deg = Seal-Report $deg
        Assert-Throws { Grade-InMemory -Rep $deg } "degenerate_interval_ge_window_rejected: interval == window (2-sample series) fails closed"

        # ==== ROUND 2 finding 2: cumulative series between endpoints ====
        # intermediate_cpu_decrease_rejected (TotalProcessorTime dips mid-series)
        $cpuDec = New-BaselineReport -Nonce ("6"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        $cpuDec["samples"][60]["cpu_seconds"] = [double]0   # below sample 59 -> impossible
        $cpuDec = Rebuild-Report $cpuDec
        $vcd = Grade-InMemory -Rep $cpuDec
        Assert-True ($vcd["verdict"] -eq "FAIL") "intermediate_cpu_decrease_rejected: grades FAIL"
        Assert-True ($vcd["bounds"]["cpu_busy_loop"]["verdict"] -eq "FAIL") "intermediate_cpu_decrease_rejected: cpu_busy_loop FAIL (monotonic violated)"
        Assert-True ($vcd["bounds"]["cpu_busy_loop"]["cpu_monotonic_nondecreasing"] -eq $false) "intermediate_cpu_decrease_rejected: cpu_monotonic_nondecreasing FALSE"

        # intermediate_cpu_spike_rejected_via_maxrate (avg benign, one interval busy)
        $spike = New-BaselineReport -Nonce ("7"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        for ($i = 60; $i -lt $spike["samples"].Count; $i++) { $spike["samples"][$i]["cpu_seconds"] = [double]170 }  # jump at 60, flat after (monotonic)
        $spike = Rebuild-Report $spike
        $vsp = Grade-InMemory -Rep $spike
        Assert-True ($vsp["verdict"] -eq "FAIL") "intermediate_cpu_spike_rejected_via_maxrate: grades FAIL"
        Assert-True (([double]$vsp["bounds"]["cpu_busy_loop"]["core_fraction"]) -le $BusyLoopCoreFractionCeiling) "intermediate_cpu_spike_rejected_via_maxrate: endpoint AVG is within ceiling (would hide)"
        Assert-True (([double]$vsp["bounds"]["cpu_busy_loop"]["max_interval_core_fraction"]) -gt $BusyLoopCoreFractionCeiling) "intermediate_cpu_spike_rejected_via_maxrate: MAX per-interval rate exceeds ceiling (caught)"

        # intermediate_context_decrease_is_owed (dip mid-series; endpoints positive)
        $csDec = New-BaselineReport -Nonce ("9"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        $csDec["samples"][60]["context_switches"] = [long]0   # dip below neighbours; endpoint diff still +600
        $csDec = Rebuild-Report $csDec
        $vcsd = Grade-InMemory -Rep $csDec
        Assert-True ($vcsd["bounds"]["context_switches"]["verdict"] -eq "OWED") "intermediate_context_decrease_is_owed: context_switches OWED (intermediate decrease)"
        Assert-True (-not $vcsd["bounds"]["context_switches"].ContainsKey("total")) "intermediate_context_decrease_is_owed: NO fabricated total"
        Assert-True ($vcsd["verdict"] -eq "PASS_WITH_OWED") "intermediate_context_decrease_is_owed: churn is evidence, not a gate FAIL"

        # ============ FINDING 2: ledger mandatory ============
        # null_ledger_rejected
        $nl = New-BaselineReport -Nonce ("3"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        Assert-Throws { Grade-InMemory -Rep $nl -NullLedger } "null_ledger_rejected: absent ledger root fails closed"
        # missing_ledger_seal_rejected
        $ms = New-BaselineReport -Nonce ("4"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        $vms = Grade-InMemory -Rep $ms -NoSeal
        Assert-True ($vms["verdict"] -eq "FAIL") "missing_ledger_seal_rejected: grades FAIL"
        Assert-True ($vms["checks"]["ledger_seal_matches"] -eq $false) "missing_ledger_seal_rejected: ledger_seal_matches FALSE"

        # ============ tamper + attribution + measure ============
        # tamper_without_reseal_rejected (mutate a sample AFTER sealing, no re-hash)
        $tamper = New-BaselineReport -Nonce ("5"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        $tamper["samples"][2]["private_bytes"] = [long]999999999
        $vt = Grade-InMemory -Rep $tamper
        Assert-True ($vt["checks"]["seal_intact"] -eq $false) "tamper_without_reseal_rejected: seal_intact FALSE"
        Assert-True ($vt["verdict"] -eq "FAIL") "tamper_without_reseal_rejected: grades FAIL"

        # pid_recycle_rejected (one sample start_time differs by a realistic delta)
        $recy = New-BaselineReport -Nonce ("6"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        $recy["samples"][3]["start_time_filetime"] = [long]($ft + 100000000)
        $recy = Seal-Report $recy
        $vr = Grade-InMemory -Rep $recy
        Assert-True ($vr["verdict"] -eq "FAIL") "pid_recycle_rejected: grades FAIL"
        Assert-True ($vr["checks"]["target_attributable"] -eq $false) "pid_recycle_rejected: target_attributable FALSE"

        # top_level_extra_key_rejected
        $extra = New-BaselineReport -Nonce ("7"*64) -St "healthy-wmi" -ProcId 4242 -StartFt $ft
        $extra["surprise"] = "boom"
        $extra = Seal-Report $extra
        Assert-Throws { Grade-InMemory -Rep $extra } "top_level_extra_key_rejected: unexpected top-level key fails closed"

        # absent_pid_measure_fails_closed
        Assert-Throws {
            $script:Mode="Measure"; $script:State="healthy-wmi"; $script:TargetProcessName=""
            $script:TargetPid=999999; $script:WindowSeconds=2; $script:SampleIntervalSeconds=1
            $script:OutReport=(Join-Path $sandbox "never.json"); $script:AttemptNonce=("8"*64); $script:NonceLedgerRoot=$ledger
            $script:ActionToken="MEASURE_AUTO_ATTACH_IDLE_COST_healthy-wmi"
            Invoke-Measure
        } "absent_pid_measure_fails_closed: measuring an absent PID fails closed"

        Write-Host "[idle-cost] SELF-TEST PASSED"
        return $true
    } finally {
        Remove-Item -LiteralPath $sandbox -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# ------------------------------------------------------------------------- main

switch ($Mode) {
    "Measure"  { Invoke-Measure | Out-Null }
    "Grade"    { $v = Invoke-Grade; if ($v["verdict"] -ne "PASS_WITH_OWED") { exit 2 } }
    "SelfTest" { Invoke-SelfTest | Out-Null }
    default    { Fail-Closed "unknown -Mode '$Mode'" }
}
