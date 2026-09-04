<#
.SYNOPSIS
Passive, no-debug-scaffolding recorder test loop.

By default, attaches the production recorder to one already-running speed-5
campaign and never drives that user-owned client. An explicitly opted-in,
unshipped evidence mode may launch and configure the game before the graded
window. From recorder attach through authenticated teardown, every mode is
strictly read-only toward the client. A separate independent VM_READ witness
records speed, pause, and hour progression throughout that window.

The spool is stored with streaming zstd as it is captured (-StreamCompression
none restores the old uncompressed storage for a byte-level debugging run).
Storage is a codec: the decoded H4CR bytes, the fold, and every sealed
artifact are identical either way.

Deliberately NOT included: the debug capture profile, sidecar logs, per-read
RC counters. The point of this loop is a plain stream plus a plain manifest.

EXIT CODES

  0  The recorder exited 0, the game survived, and nothing was refused.
  1  An ordinary failure. Reported in prose; no caller keys on it.
  2  The compact-status verdict. Either the raw-capture report found status
     bits no contract defines, or -- the fail-closed half -- the run never
     reached a report that could answer, so the question stands unanswered.
     This code has ABSOLUTE precedence: it is the only machine-readable
     verdict that is gone forever if any other code is returned first, and
     every co-occurring failure is still printed beside it.
  3  A NAMED REFUSAL, and nothing outranks it. The attempt reached the end of
     its accounting with no other failure pending: the recorder exited 0 and
     authenticated its own teardown, the graded window closed, the cleanup
     prefix completed, nothing is unwinding, the report answered, and
     CAPTURE_REFUSAL_V1.json names what was refused. A refusal is not a crash --
     everything downstream of it still ran, which is exactly why it needs a code
     of its own: without one the attempt would return the recorder's 0 and read
     as a good capture.

     3 is the LAST code considered, never the first. An `exit` in a `finally`
     beats a pending exception, so a refusal is capable of suppressing every
     other failure here; each one is therefore checked ahead of it, including
     a still-unwinding abort and a terminating error in the cleanup prefix.
     A refused READY normally leaves through 2, because it aborts before the
     report can answer -- its refusal is still fully named, just not with 3.

Every refusal also prints machine-readable phase lines (`phase=capture_refused`,
`phase=capture_refused_clause`, `phase=supervision_contained`,
`phase=supervision_failed`, `phase=capture_refusals`), so a caller that reads the
transcript rather than the exit code sees the same thing. There is no route out
of this script that strands a run without one of these.
#>
[CmdletBinding()]
param(
    [string]$Hoi4Exe = 'X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe',

    [string]$PdxUserDir = 'C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV',


    [string]$OutDir = '',

    [ValidateRange(10, 86400)]
    [double]$DurationSeconds = 120,

    [ValidateRange(1, 5)]
    [int]$Speed = 5,

    # Not 0. At 0 the recorder's hour floor cannot fail, so a capture whose
    # clock never moved seals exactly like one that advanced 593 hours -- the
    # check was vacuous on every run to date, while still being the thing the
    # failure message named. 1 is the smallest value that makes it mean
    # something: the clock must have produced at least one game hour. Raise it
    # deliberately for a run that is supposed to cover a span.
    [ValidateRange(0, 100000)]
    [int]$MinGameHours = 1,

    [ValidateRange(1, 10000)]
    [int]$SampleMilliseconds = 1,

    [ValidateRange(1, 64)]
    [int]$ReaderConcurrency = 12,

    [ValidateRange(1, 25)]
    [int]$SchedulerPollMilliseconds = 2,

    [ValidateRange(1, 1)]
    [int]$SchedulerQueueCapacity = 1,

    # Storage encoding for collector_stream.bin. Level 3 zstd measured 1 483
    # MB/s on one core against this corpus (2.7 % of a core at the 39.55 MB/s
    # max-speed input rate) for a 23.7x storage saving; 'none' is for runs that
    # need the raw H4CR bytes readable straight off disk.
    [ValidateSet('zstd', 'none')]
    [string]$StreamCompression = 'zstd',

    # Seconds to wait for the loaded campaign to expose stable world singletons.
    [ValidateRange(30, 1800)]
    [int]$WorldReadyTimeoutSeconds = 600,

    # Startup + finalize headroom allowed on top of -DurationSeconds before the
    # recorder is treated as wedged rather than slow. `-1` DERIVES it from
    # -DurationSeconds; any other value is used exactly as given.
    #
    # This is the STARTING bound only. It is a wall clock with no relation to
    # the amount of post-capture work, and on 2026-08-06 that cost three live
    # trials: t27/t28/t29 were declared unresolved here while their folds were
    # provably still incomplete (443/542, 149/577 and 194/459 hours present in
    # `checkpoints.h4cp`, none of them having reached `hour_index.h4ix`). Once
    # the recorder publishes RECORDER_FINALIZE_PROGRESS_V1.json this budget
    # gives way to the progress bound below, which is the fact rather than the
    # guess. Raising this number was never the fix -- a wall clock cannot be
    # made correct, only made longer.
    #
    # It was nonetheless far too SHORT, and a flat 300 rested on nothing. The
    # derivation now behind the default, from the two live speed-5 trials the
    # teardown lanes ran on this box (evidence
    # `extractor/native/runs/lq_teardown_census`):
    #
    #   t65: a 60 s capture recorded 322 game hours / 1.70 GB of decoded stream
    #        and its own RECORDER_FINALIZE_PROGRESS_V1.json reports
    #        elapsed_ms = 661,213 from the READY anchor to phase "complete".
    #        Take the 60 s capture window off that and the recorder's non-capture
    #        work measured 601 s -- twice the whole default budget.
    #        601 s / 322 hours = 1.87 s of recorder work per recorded hour.
    #   t64: the same capture length recorded 630 hours / 2.42 GB and never
    #        finished under the 300 s default at all.
    #
    # Recorded hours are what the work scales with, and at max game speed they
    # scale with capture length: 322/60 = 5.4 h/s in t65 and 630/60 = 10.5 h/s
    # in t64. Worst observed rate times the measured per-hour cost is
    # 10.5 * 1.87 = 19.6 s of overhead per second of capture; doubled for margin
    # gives the factor below. For the 60 s captures both trials ran that yields
    # 2400 s, which is exactly the override t65 needed to complete.
    #
    # OVERRIDE: pass -RecorderOverheadSeconds explicitly to pin a flat budget
    # (e.g. a short smoke capture on a fast box, or a deliberately tight bound
    # while reproducing a wedge). An explicit value is never derived from and
    # never reduced.
    # The upper bound is 40 * the largest admitted -DurationSeconds, so the
    # derived default can never assign past its own range.
    [ValidateRange(-1, 3456000)]
    [int]$RecorderOverheadSeconds = -1,

    # How long the recorder's published finalize work quantity may stand STILL
    # before finalization is called wedged rather than slow.
    #
    # This bounds `work_unchanged_ms` from the progress receipt, never elapsed
    # time: a fold that is still writing checkpoint bytes extends its own wait
    # for as long as it keeps writing them, and one that stops moving is out of
    # budget however early in the run it is.
    [ValidateRange(15, 1800)]
    [int]$FinalizeNoProgressSeconds = 180,

    # Ceiling on GRANTED finalize extensions, measured from the recorder's READY
    # anchor. Backstop only: a recorder reporting progress forever (a fold
    # looping over work it never finishes) is still bounded. Past this ceiling
    # the harness stops the recorder ONLY when it can prove the recorder holds
    # no hooks in a live game -- its target gone, or teardown authenticated --
    # and it records a named refusal when it does; see the cleanup wait.
    #
    # It caps what the progress receipt can BUY; it is deliberately not a cap on
    # the recorder's whole lifetime. `-DurationSeconds + -RecorderOverheadSeconds`
    # is its own bound and is never reduced here, so a caller who sets that pair
    # beyond this ceiling keeps the longer of the two. Lowering an operator's
    # explicit budget from inside the extension path would be a surprise, and no
    # evidence this function reads is a reason to shorten a wait.
    [ValidateRange(300, 86400)]
    [int]$FinalizeCeilingSeconds = 7200,

    # Seconds of continuously-advancing clock required before the recorder is
    # allowed to inject. All settle observations are read-only.
    #
    # MITIGATION, not a fix. Injecting while the engine is still finishing its
    # parallel post-load work has crashed the game (access violation calling
    # address 0x0 on a tbb.dll worker thread); injecting into a settled process
    # has been reliable. Hook installation freezes threads per hook via MinHook,
    # and a thread created during that window can miss the freeze. Set to 0 to
    # deliberately reproduce the race.
    [ValidateRange(0, 900)]
    [int]$SettleSeconds = 90,

    # Maximum time to wait for collector_record to authenticate and atomically
    # publish its passive CaptureReadyV2 receipt.
    [ValidateRange(5, 120)]
    [int]$InjectionReadyTimeoutSeconds = 60,

    # Interval for the independent read-only speed/pause/hour witness during
    # recorder attach, capture, and teardown.
    [ValidateRange(2, 60)]
    [int]$WitnessIntervalSeconds = 5,

    # How much unexplained clock silence is a hung engine rather than a slow
    # frame. Both must be met before the run stops. A window in which the
    # engine reports a pause is an explanation, so it interrupts the run rather
    # than counting toward it.
    #
    # The engine dispatches a tick per frame at UI speed 5, so the default -- 3
    # consecutive silent windows spanning at least 30 s -- is dozens of missing
    # frames, not one unlucky sample.
    [ValidateRange(2, 100)]
    [int]$StallEscalationSamples = 3,

    [ValidateRange(5, 3600)]
    [int]$StallEscalationSeconds = 30,

    # Compatibility spelling for the now-default passive mode. The harness
    # always reuses one already-running game unless -EvidenceOnlyDriveGame is
    # explicitly supplied.
    [switch]$UseRunningGame,

    # Unshipped evidence-only setup mode. This is the sole mode allowed to
    # launch, stop, or configure the game, and those actions occur before attach.
    [switch]$EvidenceOnlyDriveGame,

    # Evidence-only: close a running hoi4.exe before launching a fresh one.
    [switch]$StopExistingGame,

    # Evidence-only: leave the harness-launched game running after teardown.
    [switch]$KeepGameAfterCapture,

    # Keep every spool byte on disk after the container seals. Measurement
    # runs need this: the spool trim releases the raw H4CR bytes once every
    # hour is durably checkpointed, and the d250 lane-wall / lateness probes
    # live ONLY in those bytes (the sealed container carries no stream
    # diagnostics), so a trimmed measurement run cannot be audited afterward.
    [switch]$KeepSpoolAfterSeal,

    # Where the recorder keeps the attach-time content census digest memo
    # across captures. Empty derives `<repo>\captures\.content_census_cache`
    # (captures/ is gitignored); the memo is validated per file and re-hashed
    # on any disagreement, so sharing one across captures costs nothing but
    # the 305 s a fresh install hash costs without it.
    [string]$ContentCensusCacheDir = '',

    # Where the recorder keeps the content-keyed fold cache (one certified
    # fold per spool sha256 and fold source fingerprint). Empty derives
    # `<repo>\captures\.fold_cache`; the recorder publishes every sealed fold
    # there and adopts a valid entry instead of re-folding the same spool.
    [string]$FoldCacheDir = '',

    # Restrict BOTH hoi4.exe and collector_record to this many PHYSICAL cores,
    # so a capture on this machine approximates the core count of the 4c/8 GB
    # ship machine the capture floor targets. 0 -- the default -- applies no
    # mask and leaves every process where the scheduler puts it.
    #
    # PHYSICAL cores, not logical processors, and that distinction is the whole
    # point. Masking to logical processors 0..3 on an SMT machine selects TWO
    # physical cores and their siblings -- a 2-core machine wearing a 4-core
    # label -- and would overstate the floor's cost by roughly a factor of two
    # while looking perfectly correct in Task Manager. The mask is therefore
    # built from GetLogicalProcessorInformationEx's per-core affinity masks and
    # never from an index range.
    #
    # 1 is refused below rather than range-excluded, because 0 has to stay
    # available as the off switch. A single core is not a floor anyone ships to
    # and it removes the only thing this switch exists to measure: how the
    # engine and the sweep compete for cores.
    #
    # What this does NOT emulate: per-core speed, cache size, and memory
    # bandwidth. A masked 9800X3D keeps its whole 96 MB L3 and its full DRAM
    # bandwidth, so a masked run is a LOWER BOUND on a real 4c/8 GB machine.
    # The 8 GB half is not emulated at all -- HARNESS_CPU_AFFINITY_V1.json
    # records why in the run directory that used it.
    [ValidateRange(0, 1024)]
    [int]$CpuAffinityCores = 0,

    # Select one logical processor per chosen core (a 4c/4t machine) instead of
    # every sibling of those cores (a 4c/8t machine). Both are real shipping
    # configurations and they do not cost the same: the sweep's 12 reader lanes
    # gain from SMT, the engine's critical path much less. Default is every
    # sibling, which is the more common cheap 4-core part.
    [switch]$CpuAffinityOneThreadPerCore,

    # Override the derived production roster with an explicit domain list.
    # Used to measure how the injection-crash rate scales with the number of
    # installed hooks; not a production configuration.
    [string[]]$ScheduledDomainOverride = @(),

    [switch]$SkipHostFold,

    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
# Declared HERE, before anything can throw (lane ME, 2026-08-17): the cleanup
# path tests `$null -ne $recorderProcess`, and under StrictMode a throw between
# this point and Phase 1 used to be REPLACED by "The variable '$recorderProcess'
# cannot be retrieved because it has not been set" - the real error (a launch
# collision with another lane's game, a static-catalog check, ...) was masked
# on three lane-ME runs. Phase 1 assigns it again; that is harmless.
$recorderProcess = $null

# Resolve the derived default. See the parameter's own derivation: 19.6 s of
# measured recorder work per second of speed-5 capture, doubled for margin, with
# the old flat 300 kept as a floor so a very short capture is never given LESS
# headroom than it used to have. An explicit operator value is never touched.
$recorderOverheadDerived = $false
if ($RecorderOverheadSeconds -lt 0) {
    # NOT clamped. The parameter's range below is widened to hold the largest
    # value this can derive (40 * the largest admitted -DurationSeconds), so the
    # budget goes on scaling with the measured work instead of flattening out
    # above a 36-minute capture and letting the recorder be called wedged while
    # it is still folding (Codex review
    # 01a00a62-58f3-70a0-8ca1-62a2c83feec6, MED 2).
    $RecorderOverheadSeconds = [Math]::Max(300, 40 * [long]$DurationSeconds)
    $recorderOverheadDerived = $true
}
Write-Host "phase=recorder_overhead_resolved overhead_s=$RecorderOverheadSeconds derived=$recorderOverheadDerived duration_s=$DurationSeconds"

function Get-SettingsGuardBaseException {
    param([Parameter(Mandatory = $true)][System.Exception]$Exception)

    return $Exception.GetBaseException()
}

function Get-SettingsGuardExceptionKind {
    param([Parameter(Mandatory = $true)][System.Exception]$Exception)

    $baseException = Get-SettingsGuardBaseException -Exception $Exception
    if ($baseException -is [System.IO.FileNotFoundException]) {
        return 'not_found'
    }
    if ($baseException -is [System.IO.DirectoryNotFoundException]) {
        return 'not_found'
    }
    if ($baseException -is [System.UnauthorizedAccessException]) {
        return 'access_denied'
    }
    if ($baseException -is [System.IO.IOException]) {
        $win32Code = [int]$baseException.HResult -band 0xFFFF
        if ($win32Code -eq 32 -or $win32Code -eq 33) {
            return 'sharing_violation'
        }
    }
    return 'other'
}

if ($null -eq ('Hoi4MpStats.SettingsGuardNativeMethods' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace Hoi4MpStats {
    public static class SettingsGuardNativeMethods {
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern uint GetFinalPathNameByHandle(
            SafeFileHandle file,
            StringBuilder path,
            uint pathLength,
            uint flags);
    }
}
'@
}

function Get-SettingsGuardCanonicalPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $stream = $null
    try {
        $share = [System.IO.FileShare](
            [int][System.IO.FileShare]::ReadWrite -bor
            [int][System.IO.FileShare]::Delete
        )
        $stream = [System.IO.File]::Open(
            $Path,
            [System.IO.FileMode]::Open,
            [System.IO.FileAccess]::Read,
            $share
        )

        # A volume-GUID final path gives junctions, symbolic links and SUBST
        # spellings one stable identity. Fall back only for providers that do
        # not expose a volume GUID (for example some network redirectors).
        $lastWin32Error = 0
        foreach ($flags in @(1, 2, 0)) {
            $capacity = 1024
            while ($capacity -le 32768) {
                $buffer = [System.Text.StringBuilder]::new($capacity)
                $length = [Hoi4MpStats.SettingsGuardNativeMethods]::GetFinalPathNameByHandle(
                    $stream.SafeFileHandle,
                    $buffer,
                    [uint32]$buffer.Capacity,
                    [uint32]$flags
                )
                if ($length -eq 0) {
                    $lastWin32Error = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
                    break
                }
                if ($length -lt [uint32]$buffer.Capacity) {
                    return $buffer.ToString()
                }
                $capacity = [int]$length + 1
            }
        }
        throw [ComponentModel.Win32Exception]::new(
            $lastWin32Error,
            "GetFinalPathNameByHandle failed for '$Path'"
        )
    } finally {
        if ($null -ne $stream) {
            $stream.Dispose()
        }
    }
}

function Get-SettingsGuardKey {
    param([Parameter(Mandatory = $true)][string]$Identity)

    $hash = [System.Security.Cryptography.SHA256]::Create()
    try {
        $keyBytes = [System.Text.Encoding]::UTF8.GetBytes(
            $Identity.ToUpperInvariant()
        )
        return ([System.BitConverter]::ToString(
            $hash.ComputeHash($keyBytes)
        )).Replace('-', '').ToLowerInvariant()
    } finally {
        $hash.Dispose()
    }
}

function Write-SettingsGuardNonThrowingWarning {
    param([Parameter(Mandatory = $true)][string]$Message)

    try {
        Write-Warning $Message -WarningAction Continue
    } catch {
        # A restore/cleanup diagnostic must never replace the capture error or
        # exit code already unwinding through the outer finally.
        try {
            [Console]::Error.WriteLine("WARNING: $Message")
        } catch {
        }
    }
}

function ConvertTo-NativeArgumentLine {
    param([Parameter(Mandatory = $true)][object[]]$Arguments)
    return (($Arguments | ForEach-Object {
        $value = [string]$_
        # The empty string needs quoting for a reason the other cases do not:
        # unquoted it emits NOTHING, so the argument disappears and every later
        # one shifts a position left. `--capture-id '' --session-id x` would
        # reach the collector as `--capture-id --session-id x`, binding the
        # capture id to a flag and the session id to nothing.
        if ($value.Length -eq 0 -or $value -match '[\s"]') { '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"' }
        else { $value }
    }) -join ' ')
}

function ConvertTo-PowerShellLiteral {
    param([Parameter(Mandatory = $true)][string]$Value)

    return "'" + $Value.Replace("'", "''") + "'"
}

# The collector_record command line, built as a VALUE rather than in place.
#
# This is the whole capture plan: which process, which spool encoding, which
# scheduler shape, and which domains are asked for. Every one of those is a
# decision the run cannot be re-made without, and until this was a function the
# only thing that ever executed it was a live capture -- a dropped
# `--min-game-hours`, a domain silently missing from the roster, or a flag
# spelled for a collector that no longer takes it would all have been found by
# burning a game hour rather than by a test.
#
# It is deliberately TOTAL and side-effect free: no file reads, no process
# lookups, no host writes. Everything it needs is a parameter, so the plan can
# be asserted argument-for-argument offline
# (Test-HeadlessRawCaptureUnits.ps1), and the caller keeps the job of proving
# those inputs are real.
function New-RecorderArgumentPlan {
    param(
        [Parameter(Mandatory = $true)][int]$GamePid,
        [Parameter(Mandatory = $true)][string]$CollectorDll,
        [Parameter(Mandatory = $true)][string]$OutputDirectory,
        [Parameter(Mandatory = $true)][string]$DurationArgument,
        [Parameter(Mandatory = $true)][int]$SampleMilliseconds,
        [Parameter(Mandatory = $true)][int]$MinGameHours,
        [Parameter(Mandatory = $true)][ValidateSet('zstd', 'none')]
        [string]$StreamCompression,
        [Parameter(Mandatory = $true)][int]$SchedulerPollMilliseconds,
        [Parameter(Mandatory = $true)][int]$SchedulerQueueCapacity,
        [Parameter(Mandatory = $true)][int]$ReaderConcurrency,
        [Parameter(Mandatory = $true)][string]$SessionId,
        [Parameter(Mandatory = $true)][string]$CaptureId,
        # An empty roster is refused here rather than passed on. The collector
        # falls back to its eight-domain compatibility set when it is given no
        # schedule, so an empty roster does not fail -- it quietly captures a
        # different, much smaller run that looks like a successful capture.
        [Parameter(Mandatory = $true)][AllowEmptyCollection()]
        [string[]]$ScheduledDomains,
        # [ordered] on purpose: this becomes an argument VECTOR, and a plain
        # hashtable would order the static-catalog flags by hash bucket.
        [Parameter(Mandatory = $true)]
        [System.Collections.Specialized.OrderedDictionary]$StaticCatalogs,
        [bool]$SkipHostFold = $false,
        [bool]$KeepSpoolAfterSeal = $false,
        # The attach-time content census digest memo. Empty means the recorder
        # re-hashes the whole install (305 s on rc_vanilla8) every capture.
        [string]$ContentCensusCacheDirectory = '',
        # The content-keyed fold cache: the certified fold of each spool, kept
        # once per (spool sha256, fold source fingerprint) for re-finalizes and
        # hour dumps. Empty means finalize neither adopts nor publishes one.
        [string]$FoldCacheDirectory = ''
    )

    if (@($ScheduledDomains).Count -eq 0) {
        throw 'The recorder argument plan requires a nonempty scheduled-domain roster; an empty roster silently selects the collector''s eight-domain compatibility set.'
    }

    $plan = @(
        '--pid', "$GamePid",
        '--dll', $CollectorDll,
        '--out-dir', $OutputDirectory,
        '--duration-s', $DurationArgument,
        '--sample-ms', "$SampleMilliseconds",
        '--min-game-hours', "$MinGameHours",
        '--stream-format', 'compact',
        '--stream-compression', $StreamCompression,
        '--compact-country-frame',
        '--read-only-scheduler',
        '--read-only-scheduler-poll-ms', "$SchedulerPollMilliseconds",
        '--read-only-scheduler-queue-cap', "$SchedulerQueueCapacity",
        '--reader-concurrency', "$ReaderConcurrency",
        '--session-id', $SessionId,
        '--capture-id', $CaptureId
    )
    foreach ($domain in $ScheduledDomains) {
        $plan += '--scheduled-domain'
        $plan += $domain
    }
    foreach ($entry in $StaticCatalogs.GetEnumerator()) {
        $plan += $entry.Key
        $plan += $entry.Value
    }
    if ($SkipHostFold) {
        $plan += '--skip-host-fold'
        $plan += '--package-raw-stream'
    }
    if ($KeepSpoolAfterSeal) { $plan += '--keep-spool-after-seal' }
    if (-not [string]::IsNullOrWhiteSpace($ContentCensusCacheDirectory)) {
        $plan += '--content-census-cache-dir'
        $plan += $ContentCensusCacheDirectory
    }
    if (-not [string]::IsNullOrWhiteSpace($FoldCacheDirectory)) {
        $plan += '--fold-cache-dir'
        $plan += $FoldCacheDirectory
    }
    # Emitted unrolled, NOT as `,$plan`. Every caller wraps the result in `@()`,
    # which restores the array; returning a wrapped array instead makes that
    # `@()` a one-element array whose single element is the plan, and
    # `$recorderArgs -join "`n"` then writes the string `System.Object[]` into
    # recorder_args.txt while the collector receives one nonsense argument.
    return $plan
}

function ConvertTo-ReceiptUInt32 {
    param(
        [AllowNull()][object]$Value,
        [Parameter(Mandatory = $true)][string]$Field
    )

    if (-not (
        $Value -is [byte] -or $Value -is [uint16] -or
        $Value -is [uint32] -or $Value -is [uint64] -or
        $Value -is [int16] -or $Value -is [int32] -or
        $Value -is [int64] -or $Value -is [decimal]
    )) {
        throw "Ready receipt field '$Field' is not a JSON integer value."
    }
    [uint32]$parsed = 0
    $text = [Convert]::ToString(
        $Value,
        [Globalization.CultureInfo]::InvariantCulture
    )
    if ([string]::IsNullOrEmpty($text) -or
        -not [uint32]::TryParse(
            $text,
            [Globalization.NumberStyles]::None,
            [Globalization.CultureInfo]::InvariantCulture,
            [ref]$parsed
        ) -or
        $text -cne $parsed.ToString([Globalization.CultureInfo]::InvariantCulture)) {
        throw "Ready receipt field '$Field' is not a canonical bounded u32."
    }
    return $parsed
}

function ConvertTo-ReceiptUInt64 {
    param(
        [AllowNull()][object]$Value,
        [Parameter(Mandatory = $true)][string]$Field
    )

    if (-not (
        $Value -is [byte] -or $Value -is [uint16] -or
        $Value -is [uint32] -or $Value -is [uint64] -or
        $Value -is [int16] -or $Value -is [int32] -or
        $Value -is [int64] -or $Value -is [decimal]
    )) {
        throw "Ready receipt field '$Field' is not a JSON integer value."
    }
    [uint64]$parsed = 0
    $text = [Convert]::ToString(
        $Value,
        [Globalization.CultureInfo]::InvariantCulture
    )
    if ([string]::IsNullOrEmpty($text) -or
        -not [uint64]::TryParse(
            $text,
            [Globalization.NumberStyles]::None,
            [Globalization.CultureInfo]::InvariantCulture,
            [ref]$parsed
        ) -or
        $text -cne $parsed.ToString([Globalization.CultureInfo]::InvariantCulture)) {
        throw "Ready receipt field '$Field' is not a canonical bounded u64."
    }
    return $parsed
}

function ConvertTo-ReceiptInt32 {
    param(
        [AllowNull()][object]$Value,
        [Parameter(Mandatory = $true)][string]$Field
    )

    if (-not (
        $Value -is [byte] -or $Value -is [uint16] -or
        $Value -is [uint32] -or $Value -is [uint64] -or
        $Value -is [int16] -or $Value -is [int32] -or
        $Value -is [int64] -or $Value -is [decimal]
    )) {
        throw "Ready receipt field '$Field' is not a JSON integer value."
    }
    [int32]$parsed = 0
    $text = [Convert]::ToString(
        $Value,
        [Globalization.CultureInfo]::InvariantCulture
    )
    if ([string]::IsNullOrEmpty($text) -or
        -not [int32]::TryParse(
            $text,
            [Globalization.NumberStyles]::AllowLeadingSign,
            [Globalization.CultureInfo]::InvariantCulture,
            [ref]$parsed
        ) -or
        $text -cne $parsed.ToString([Globalization.CultureInfo]::InvariantCulture)) {
        throw "Ready receipt field '$Field' is not a canonical bounded i32."
    }
    return $parsed
}

function Get-Utf8Sha256Hex {
    param([Parameter(Mandatory = $true)][string]$Value)

    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Value)
        return ([BitConverter]::ToString(
            $sha256.ComputeHash($bytes)
        )).Replace('-', '').ToLowerInvariant()
    } finally {
        $sha256.Dispose()
    }
}

function Assert-ExactJsonPropertySet {
    param(
        [Parameter(Mandatory = $true)][object]$Value,
        [Parameter(Mandatory = $true)][string[]]$Expected,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $actual = @($Value.PSObject.Properties.Name | Sort-Object)
    $wanted = @($Expected | Sort-Object)
    if ($actual.Count -ne $wanted.Count) {
        throw "$Label has $($actual.Count) properties; expected exactly $($wanted.Count)."
    }
    for ($i = 0; $i -lt $wanted.Count; $i++) {
        if ([string]$actual[$i] -cne [string]$wanted[$i]) {
            throw "$Label property set differs at index $i ('$($actual[$i])' versus '$($wanted[$i])')."
        }
    }
}

function Stop-ExactProcess {
    param(
        [Parameter(Mandatory = $true)][System.Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)][string]$Purpose,
        [ValidateRange(100, 30000)][int]$WaitMilliseconds = 10000
    )

    $Process.Refresh()
    if ($Process.HasExited) {
        return
    }
    try {
        $Process.Kill()
    } catch {
        $Process.Refresh()
        if (-not $Process.HasExited) {
            throw "Could not terminate exact $Purpose PID $($Process.Id): $($_.Exception.Message)"
        }
    }
    if (-not $Process.WaitForExit($WaitMilliseconds)) {
        throw "Exact $Purpose PID $($Process.Id) did not exit within $WaitMilliseconds ms after termination."
    }
}

function Invoke-BoundedNativeCapture {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][object[]]$Arguments,
        [Parameter(Mandatory = $true)][ValidateRange(1, 3600)][int]$TimeoutSeconds,
        [Parameter(Mandatory = $true)][string]$Purpose,
        [switch]$AllowNonzeroExit
    )

    $stdoutPath = [System.IO.Path]::GetTempFileName()
    $stderrPath = [System.IO.Path]::GetTempFileName()
    $process = $null
    try {
        $process = Start-Process -FilePath $FilePath `
            -ArgumentList (ConvertTo-NativeArgumentLine $Arguments) `
            -NoNewWindow -PassThru `
            -RedirectStandardOutput $stdoutPath `
            -RedirectStandardError $stderrPath
        # See the note at the recorder launch below: a `Start-Process -PassThru`
        # object caches no process handle, so `.ExitCode` reads back $null once
        # the child is gone -- and `[int]$null` is 0, which would make every
        # bounded helper look like it succeeded no matter what it returned.
        # Neither `Refresh()` nor `WaitForExit()` repairs it after the fact;
        # the handle has to be taken while the child is alive.
        $null = $process.Handle
        $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
        $nextProgress = [DateTime]::UtcNow.AddSeconds(5)
        while (-not $process.HasExited) {
            $remainingMilliseconds = [Math]::Ceiling(
                ($deadline - [DateTime]::UtcNow).TotalMilliseconds
            )
            if ($remainingMilliseconds -le 0) {
                Stop-ExactProcess -Process $process -Purpose $Purpose
                # TYPED, so a caller can tell "the bound was hit" apart from
                # "the child failed" without matching this sentence. The
                # latency ledger reads the distinction, and inferring it from
                # elapsed time mislabelled any slow failure as a timeout
                # (Codex review round 2). The wording is unchanged.
                throw [System.TimeoutException]::new(
                    "$Purpose timed out after $TimeoutSeconds s; exact child PID $($process.Id) was terminated."
                )
            }
            $waitMilliseconds = [int][Math]::Min(1000, $remainingMilliseconds)
            if ($process.WaitForExit($waitMilliseconds)) {
                break
            }
            if ([DateTime]::UtcNow -ge $nextProgress) {
                $remainingSeconds = [Math]::Max(
                    0,
                    [Math]::Ceiling(($deadline - [DateTime]::UtcNow).TotalSeconds)
                )
                Write-Host "phase=child_wait purpose=$Purpose pid=$($process.Id) remaining_s=$remainingSeconds"
                $nextProgress = [DateTime]::UtcNow.AddSeconds(5)
            }
        }
        # Bound the redirected-stream flush as well. The process is already
        # dead, so this cannot turn a completed child into an unbounded wait.
        [void]$process.WaitForExit(1000)
        $stdout = if (Test-Path -LiteralPath $stdoutPath) {
            Get-Content -LiteralPath $stdoutPath -Raw
        } else {
            ''
        }
        $stderr = if (Test-Path -LiteralPath $stderrPath) {
            Get-Content -LiteralPath $stderrPath -Raw
        } else {
            ''
        }
        $stdout = [string]$stdout
        $stderr = [string]$stderr
        # Never let an unreadable exit status become a zero. An unknown outcome
        # is a failed helper, not a passing one.
        $childExit = $process.ExitCode
        if ($null -eq $childExit) {
            throw "$Purpose exited without exposing a process exit code. stderr: $($stderr.Trim())"
        }
        if (-not $AllowNonzeroExit -and [int]$childExit -ne 0) {
            throw "$Purpose exited with code $childExit. stderr: $($stderr.Trim())"
        }
        return [pscustomobject]@{
            ExitCode = [int]$childExit
            Stdout = [string]$stdout
            Stderr = [string]$stderr
            Pid = [int]$process.Id
        }
    } finally {
        $canRemoveStreams = $true
        if ($null -ne $process) {
            $process.Refresh()
            if (-not $process.HasExited) {
                # A bounded helper that survives cleanup is a hard residual,
                # not a warning. Preserve its redirected evidence files and
                # terminate the caller rather than unwinding as if cleanup
                # succeeded.
                Stop-ExactProcess -Process $process -Purpose $Purpose
            }
            $process.Refresh()
            $canRemoveStreams = $process.HasExited
            $process.Dispose()
        }
        if ($canRemoveStreams) {
            Remove-Item -LiteralPath $stdoutPath -Force -ErrorAction SilentlyContinue
            Remove-Item -LiteralPath $stderrPath -Force -ErrorAction SilentlyContinue
        }
    }
}

$windowsPowerShell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'

function Assert-Hoi4ProcessEpoch {
    param(
        [Parameter(Mandatory = $true)][int]$TargetProcessId,
        [Parameter(Mandatory = $true)][uint64]$ExpectedCreationFiletimeUtc,
        [Parameter(Mandatory = $true)][string]$Phase
    )

    $process = Get-Process -Id $TargetProcessId -ErrorAction Stop
    $observed = [uint64](
        $process.StartTime.ToUniversalTime().ToFileTimeUtc()
    )
    if ($observed -ne $ExpectedCreationFiletimeUtc) {
        throw "the game PID $TargetProcessId creation epoch changed during $Phase (expected=$ExpectedCreationFiletimeUtc observed=$observed)."
    }
    return $process
}

function Invoke-BoundedHoi4Control {
    param(
        [Parameter(Mandatory = $true)][int]$TargetProcessId,
        [Parameter(Mandatory = $true)][ValidateRange(1, 5)][int]$RequestedSpeed,
        [Parameter(Mandatory = $true)][ValidateSet('Play', 'Pause')][string]$Action,
        [ValidateRange(1, 60)][int]$TimeoutSeconds = 10,
        [uint64]$ExpectedCreationFiletimeUtc = 0
    )

    Assert-Hoi4ClientDrivingOptIn -Caller $PSCommandPath
    if ($ExpectedCreationFiletimeUtc -gt 0) {
        [void](Assert-Hoi4ProcessEpoch `
            -TargetProcessId $TargetProcessId `
            -ExpectedCreationFiletimeUtc $ExpectedCreationFiletimeUtc `
            -Phase "before $Action control")
    }
    $invocation = "& $(ConvertTo-PowerShellLiteral -Value $headlessControl) -TargetPid $TargetProcessId"
    $invocation += " -Speed $RequestedSpeed"
    if ($Action -eq 'Play') {
        $invocation += ' -Play'
    } elseif ($Action -eq 'Pause') {
        $invocation += ' -Pause'
    }
    $command = @"
`$ErrorActionPreference = 'Stop'
`$result = $invocation
if (`$null -eq `$result) { throw 'Control-Hoi4Game returned no state.' }
`$result | ConvertTo-Json -Compress -Depth 8
"@
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    # WHAT THE 10 s BOUND IS WORTH, measured rather than assumed. Every control
    # call records how long it actually took, so the default can be set from a
    # distribution instead of from a round number; the file is append-only
    # evidence and nothing reads it back.
    #
    # The row is written in `finally`, so a call that TIMED OUT or failed is
    # recorded too -- those are the samples that decide the bound, and writing
    # only after a successful return left them out of the distribution
    # entirely (Codex review round 1). `outcome` says which kind of sample it
    # is, taken from the EXCEPTION TYPE rather than from elapsed time, because
    # a failure that happens to be slow is not a timeout.
    #
    # WRITING THE LEDGER MUST NEVER CHANGE WHAT THE CALLER SEES. Under
    # `$ErrorActionPreference = 'Stop'` a failing `Add-Content` inside
    # `finally` would REPLACE the pending timeout/control exception with a
    # disk error, so the append has its own non-throwing guard and reports the
    # logging failure as a warning (Codex review round 2).
    $controlStopwatch = [Diagnostics.Stopwatch]::StartNew()
    $controlOutcome = 'failed'
    try {
        $result = Invoke-BoundedNativeCapture `
            -FilePath $windowsPowerShell `
            -Arguments @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-EncodedCommand', $encoded) `
            -TimeoutSeconds $TimeoutSeconds `
            -Purpose "headless control for the game PID $TargetProcessId"
        $controlOutcome = 'returned'
    } catch [System.TimeoutException] {
        $controlOutcome = 'timed_out'
        throw
    } finally {
        $controlStopwatch.Stop()
        if ($script:ControlLatencyLogPath) {
            try {
                $sample = [ordered]@{
                    schema            = 'hoi4-bounded-control-latency-v1'
                    observed_at_utc   = (Get-Date).ToUniversalTime().ToString('o')
                    action            = $Action
                    requested_speed   = $RequestedSpeed
                    timeout_seconds   = $TimeoutSeconds
                    elapsed_ms        = [int]$controlStopwatch.Elapsed.TotalMilliseconds
                    outcome           = $controlOutcome
                }
                Add-Content -LiteralPath $script:ControlLatencyLogPath -Encoding utf8 `
                    -Value ($sample | ConvertTo-Json -Compress -Depth 4)
            } catch {
                # Through the genuinely non-throwing helper: this script is an
                # advanced script, so a plain `Write-Warning` terminates under
                # `-WarningAction Stop` or an inherited
                # `$WarningPreference = 'Stop'` -- which would put the
                # diagnostic in place of the very exception it is reporting
                # about, or fail a control call that had SUCCEEDED (Codex
                # review round 3).
                Write-SettingsGuardNonThrowingWarning -Message (
                    "bounded control latency sample not recorded ({0}): {1}" -f `
                        $controlOutcome, $_.Exception.Message)
            }
        }
    }
    try {
        $state = $result.Stdout.Trim() | ConvertFrom-Json
    } catch {
        throw "Headless control for the game PID $TargetProcessId returned invalid JSON: $($_.Exception.Message)"
    }
    if ($ExpectedCreationFiletimeUtc -gt 0) {
        [void](Assert-Hoi4ProcessEpoch `
            -TargetProcessId $TargetProcessId `
            -ExpectedCreationFiletimeUtc $ExpectedCreationFiletimeUtc `
            -Phase "after $Action control")
    }
    return $state
}

function Invoke-BoundedHoi4Observation {
    param(
        [Parameter(Mandatory = $true)][int]$TargetProcessId,
        # Read-Hoi4GameState.ps1 spends up to its own clock-progress budget
        # looking for movement before it reports none, so this bound must clear
        # that budget plus PowerShell start-up or a stalled clock is reported as
        # a harness timeout instead of as the stall it is.
        [ValidateRange(1, 120)][int]$TimeoutSeconds = 30,
        [uint64]$ExpectedCreationFiletimeUtc = 0
    )

    if ($ExpectedCreationFiletimeUtc -gt 0) {
        [void](Assert-Hoi4ProcessEpoch `
            -TargetProcessId $TargetProcessId `
            -ExpectedCreationFiletimeUtc $ExpectedCreationFiletimeUtc `
            -Phase 'before read-only observation')
    }
    $invocation = "& $(ConvertTo-PowerShellLiteral -Value $readOnlyObserver) -TargetPid $TargetProcessId"
    $command = @"
`$ErrorActionPreference = 'Stop'
`$result = $invocation
if (`$null -eq `$result) { throw 'Read-Hoi4GameState returned no state.' }
`$result | ConvertTo-Json -Compress -Depth 8
"@
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    $result = Invoke-BoundedNativeCapture `
        -FilePath $windowsPowerShell `
        -Arguments @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-EncodedCommand', $encoded) `
        -TimeoutSeconds $TimeoutSeconds `
        -Purpose "read-only observation for the game PID $TargetProcessId"
    try {
        $state = $result.Stdout.Trim() | ConvertFrom-Json
    } catch {
        throw "Read-only observation for the game PID $TargetProcessId returned invalid JSON: $($_.Exception.Message)"
    }
    if ($ExpectedCreationFiletimeUtc -gt 0) {
        [void](Assert-Hoi4ProcessEpoch `
            -TargetProcessId $TargetProcessId `
            -ExpectedCreationFiletimeUtc $ExpectedCreationFiletimeUtc `
            -Phase 'after read-only observation')
    }
    return $state
}

function Invoke-BoundedWmiLaunch {
    param(
        [Parameter(Mandatory = $true)][string]$CommandLine,
        [Parameter(Mandatory = $true)][string]$CurrentDirectory,
        [Parameter(Mandatory = $true)][string]$ExpectedExecutablePath,
        [Parameter(Mandatory = $true)][DateTime]$LaunchStartedUtc,
        [int[]]$PreexistingProcessIds = @(),
        [ValidateRange(1, 120)][int]$TimeoutSeconds = 30
    )

    Assert-Hoi4ClientDrivingOptIn -Caller $PSCommandPath
    $commandLiteral = ConvertTo-PowerShellLiteral -Value $CommandLine
    $directoryLiteral = ConvertTo-PowerShellLiteral -Value $CurrentDirectory
    $expectedExecutableLiteral = ConvertTo-PowerShellLiteral -Value $ExpectedExecutablePath
    $launchReceiptPath = Join-Path (
        [System.IO.Path]::GetTempPath()
    ) ("hoi4-wmi-launch-" + [Guid]::NewGuid().ToString('N') + '.json')
    $launchReceiptTempPath = "$launchReceiptPath.tmp"
    $launchPidPath = "$launchReceiptPath.pid"
    $launchPidTempPath = "$launchPidPath.tmp"
    $receiptLiteral = ConvertTo-PowerShellLiteral -Value $launchReceiptPath
    $receiptTempLiteral = ConvertTo-PowerShellLiteral -Value $launchReceiptTempPath
    $pidLiteral = ConvertTo-PowerShellLiteral -Value $launchPidPath
    $pidTempLiteral = ConvertTo-PowerShellLiteral -Value $launchPidTempPath
    $command = @"
`$ErrorActionPreference = 'Stop'
`$created = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -OperationTimeoutSec $TimeoutSeconds -Arguments @{
    CommandLine = $commandLiteral
    CurrentDirectory = $directoryLiteral
}
if ([uint32]`$created.ReturnValue -eq 0 -and [uint32]`$created.ProcessId -gt 0) {
    `$launched = Get-Process -Id ([int]`$created.ProcessId) -ErrorAction Stop
    `$launchedPath = `$launched.MainModule.FileName
    if (-not [StringComparer]::OrdinalIgnoreCase.Equals(
        `$launchedPath,
        $expectedExecutableLiteral
    )) {
        throw "WMI-created process image mismatch: '`$launchedPath'."
    }
    `$handoff = [ordered]@{
        pid = [uint32]`$created.ProcessId
        process_creation_filetime_utc = [uint64](
            `$launched.StartTime.ToUniversalTime().ToFileTimeUtc()
        )
        executable_path = `$launchedPath
    } | ConvertTo-Json -Compress
    [System.IO.File]::WriteAllText(
        $pidTempLiteral,
        `$handoff,
        [System.Text.UTF8Encoding]::new(`$false)
    )
    [System.IO.File]::Move($pidTempLiteral, $pidLiteral)
}
`$json = `$created | Select-Object ReturnValue, ProcessId | ConvertTo-Json -Compress
[System.IO.File]::WriteAllText(
    $receiptTempLiteral,
    `$json,
    [System.Text.UTF8Encoding]::new(`$false)
)
[System.IO.File]::Move($receiptTempLiteral, $receiptLiteral)
`$json
"@
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
    try {
        $helperFailure = $null
        try {
            [void](Invoke-BoundedNativeCapture `
                -FilePath $windowsPowerShell `
                -Arguments @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-EncodedCommand', $encoded) `
                -TimeoutSeconds ($TimeoutSeconds + 5) `
                -Purpose 'Win32_Process.Create the game launch')
        } catch {
            $helperFailure = $_
        }
        try {
            if (-not (Test-Path -LiteralPath $launchReceiptPath -PathType Leaf)) {
                if ($null -ne $helperFailure) {
                    throw $helperFailure
                }
                throw 'Win32_Process.Create helper exited without its atomic launch receipt.'
            }
            $json = Get-Content -LiteralPath $launchReceiptPath -Raw
            if ($null -ne $helperFailure) {
                Write-Warning 'Recovered the exact WMI-created the game PID from its atomic launch receipt after the bounded helper failed.'
            }
            $created = $json.Trim() | ConvertFrom-Json
            $returnValue = ConvertTo-ReceiptUInt32 `
                -Value $created.ReturnValue `
                -Field 'wmi.ReturnValue'
            $handoffPid = [uint32]0
            $handoffCreation = [uint64]0
            $handoffExecutablePath = ''
            if ($returnValue -eq 0) {
                if (-not (Test-Path -LiteralPath $launchPidPath -PathType Leaf)) {
                    throw 'successful WMI JSON lacks its prior atomic PID handoff'
                }
                $handoff = Get-Content -LiteralPath $launchPidPath -Raw |
                    ConvertFrom-Json
                Assert-ExactJsonPropertySet `
                    -Value $handoff `
                    -Expected @('pid', 'process_creation_filetime_utc', 'executable_path') `
                    -Label 'WMI atomic process handoff'
                $handoffPid = ConvertTo-ReceiptUInt32 `
                    -Value $handoff.pid `
                    -Field 'wmi_handoff.pid'
                $handoffCreation = ConvertTo-ReceiptUInt64 `
                    -Value $handoff.process_creation_filetime_utc `
                    -Field 'wmi_handoff.process_creation_filetime_utc'
                if (-not ($handoff.executable_path -is [string]) -or
                    -not [StringComparer]::OrdinalIgnoreCase.Equals(
                        [string]$handoff.executable_path,
                        $ExpectedExecutablePath
                    )) {
                    throw 'WMI atomic handoff executable path differs from authority.'
                }
                $handoffExecutablePath = [string]$handoff.executable_path
                $jsonPid = ConvertTo-ReceiptUInt32 `
                    -Value $created.ProcessId `
                    -Field 'wmi.ProcessId'
                if ($handoffPid -eq 0 -or $jsonPid -ne $handoffPid) {
                    throw "WMI JSON PID $jsonPid differs from atomic PID handoff $handoffPid."
                }
                $handedProcess = Get-Process -Id ([int]$handoffPid) -ErrorAction Stop
                $handedCreation = [uint64](
                    $handedProcess.StartTime.ToUniversalTime().ToFileTimeUtc()
                )
                if ($handedCreation -ne $handoffCreation -or
                    -not [StringComparer]::OrdinalIgnoreCase.Equals(
                        $handedProcess.Path,
                        $ExpectedExecutablePath
                    )) {
                    throw 'WMI-created PID no longer matches its atomic creation epoch and image.'
                }
            }
            return [pscustomobject]@{
                ReturnValue = $returnValue
                ProcessId = $handoffPid
                ProcessCreationFiletimeUtc = $handoffCreation
                ExecutablePath = $handoffExecutablePath
            }
        } catch {
            # The atomic PID/creation/image handoff exists before the WMI JSON
            # receipt. If reporting fails after creation, stop only that exact
            # process epoch because the caller cannot own it yet.
            $cleanedExactPid = $false
            if (Test-Path -LiteralPath $launchPidPath -PathType Leaf) {
                try {
                    $orphanHandoff = Get-Content -LiteralPath $launchPidPath -Raw |
                        ConvertFrom-Json
                    Assert-ExactJsonPropertySet `
                        -Value $orphanHandoff `
                        -Expected @('pid', 'process_creation_filetime_utc', 'executable_path') `
                        -Label 'WMI orphan process handoff'
                    $orphanPid = ConvertTo-ReceiptUInt32 `
                        -Value $orphanHandoff.pid `
                        -Field 'orphan_handoff.pid'
                    $orphanCreation = ConvertTo-ReceiptUInt64 `
                        -Value $orphanHandoff.process_creation_filetime_utc `
                        -Field 'orphan_handoff.process_creation_filetime_utc'
                    $orphan = Get-Process -Id ([int]$orphanPid) -ErrorAction SilentlyContinue
                    if ($null -ne $orphan) {
                        $orphanObservedCreation = [uint64](
                            $orphan.StartTime.ToUniversalTime().ToFileTimeUtc()
                        )
                        $orphanPathMatches = (
                            $orphanHandoff.executable_path -is [string] -and
                            [StringComparer]::OrdinalIgnoreCase.Equals(
                                [string]$orphanHandoff.executable_path,
                                $ExpectedExecutablePath
                            ) -and
                            [StringComparer]::OrdinalIgnoreCase.Equals(
                                $orphan.Path,
                                $ExpectedExecutablePath
                            )
                        )
                        if ($orphanObservedCreation -eq $orphanCreation -and
                            $orphanPathMatches) {
                            Stop-ExactProcess `
                                -Process $orphan `
                                -Purpose 'unhanded WMI-created the game'
                            $cleanedExactPid = $true
                        }
                    } else {
                        # The exact handed-off epoch has already exited.
                        $cleanedExactPid = $true
                    }
                } catch {
                    $cleanedExactPid = $false
                }
            }
            if (-not $cleanedExactPid) {
                # Never convert a same-path/time heuristic into kill authority.
                # List candidates only as operator evidence; without the atomic
                # PID + creation FILETIME + image handoff there is no exact
                # process epoch that this harness is authorized to terminate.
                $candidates = @(
                    Get-Process -Name the game -ErrorAction SilentlyContinue |
                        Where-Object {
                            $candidate = $_
                            if ($PreexistingProcessIds -contains [int]$candidate.Id) {
                                return $false
                            }
                            try {
                                $samePath = [StringComparer]::OrdinalIgnoreCase.Equals(
                                    $candidate.Path,
                                    $ExpectedExecutablePath
                                )
                                $createdAfterStart = (
                                    $candidate.StartTime.ToUniversalTime() -ge
                                    $LaunchStartedUtc.AddSeconds(-1)
                                )
                                return $samePath -and $createdAfterStart
                            } catch {
                                return $false
                            }
                        }
                )
                throw "Win32_Process.Create exact-epoch handoff failed; no process was killed. New exact-path candidate PID(s): $(@($candidates.Id) -join ',')."
            }
            throw "Win32_Process.Create did not complete an exact PID handoff: $($_.Exception.Message)"
        }
    } finally {
        Remove-Item -LiteralPath $launchPidTempPath -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $launchPidPath -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $launchReceiptTempPath -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $launchReceiptPath -Force -ErrorAction SilentlyContinue
    }
}

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir '..\..\..\portable_replay\native\scripts\Hoi4ClientDriving.OptIn.ps1')
. (Join-Path $scriptDir 'PassiveCapture.Lifecycle.ps1')
. (Join-Path $scriptDir 'Hoi4WorldReady.Gate.ps1')
$reuseRunningGame = -not $EvidenceOnlyDriveGame
if ($EvidenceOnlyDriveGame) {
    if ($UseRunningGame) {
        throw '-UseRunningGame and -EvidenceOnlyDriveGame are mutually exclusive.'
    }
    Assert-Hoi4ClientDrivingOptIn -Caller $PSCommandPath
} elseif ($StopExistingGame -or $KeepGameAfterCapture) {
    throw '-StopExistingGame and -KeepGameAfterCapture require -EvidenceOnlyDriveGame.'
}
if ($Speed -ne 5) {
    throw 'Every graded capture window is speed-5-only; lower speeds are diagnostic and cannot use this promotion harness.'
}
if ($CpuAffinityCores -eq 1) {
    throw '-CpuAffinityCores 1 is refused. Use 0 to leave the scheduler alone, or 2 or more to emulate a floor machine; a one-core mask measures nothing this switch exists to measure.'
}
if ($CpuAffinityOneThreadPerCore -and $CpuAffinityCores -eq 0) {
    throw '-CpuAffinityOneThreadPerCore selects logical processors within the cores -CpuAffinityCores chooses, so it means nothing on its own.'
}
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $scriptDir '..\..\..')).Path
$releaseRoot = Join-Path $repoRoot '.cargo-target\release'
$nativeRoot = Join-Path $repoRoot 'extractor\native'
$collectorDll = Join-Path $releaseRoot 'collector.dll'
$collectorRecord = Join-Path $releaseRoot 'collector_record.exe'
$headlessControl = Join-Path $scriptDir 'Control-Hoi4Game.ps1'
$readOnlyObserver = Join-Path $scriptDir 'Read-Hoi4GameState.ps1'
# The verdict authority for everything the read-only observer reports. The
# harness records observations; this decides what they mean.
$passiveClockWitnessTool = Join-Path $releaseRoot 'passive_clock_witness.exe'
$streamReport = Join-Path $scriptDir 'report_raw_capture.py'
# The paired free-run / with-collector rate summary. Deliberately NOT in the
# required-path list below: a missing summary must never stop a capture, and
# cleanup tests for it before calling it.
$clockBaselineSummary = Join-Path $scriptDir 'clock_baseline_summary.py'
$gameDefsOut = Join-Path $repoRoot 'extractor\game_defs\out'
# Get-Command can return several python.exe entries (real install plus the
# WindowsApps stub); take the first rather than joining them into one string.
$pythonExe = @(Get-Command python.exe -CommandType Application -ErrorAction Stop)[0].Source

$staticCatalogs = [ordered]@{
    '--static-templates-equipment-catalog' = Join-Path $gameDefsOut 'templates_equipment_catalog\templates_equipment_catalog_live_domain59\domain59_static_catalog.json'
    '--static-localisation-join'           = Join-Path $gameDefsOut 'localisation_gate\static_localisation_join_r3\localisation_catalog.json'
    '--static-decisions-definitions'       = Join-Path $gameDefsOut 'decisions_static\decisions_static_definitions\decision_definitions_catalog.json'
    '--static-idea-tech-loc-xref'          = Join-Path $gameDefsOut 'idea_tech_loc_xref\idea_tech_loc_xref_static\idea_tech_loc_xref_catalog.json'
    '--static-ship-definitions'            = Join-Path $gameDefsOut 'ship_definitions_static\ship_definitions_static\ship_definitions_catalog.json'
    '--static-gfx-icon-manifest'           = Join-Path $gameDefsOut 'gfx_icon_manifest\gfx_icon_asset_manifest_static\gfx_icon_manifest_catalog.json'
    # The two catalogs t51/t54 packaged as ABSENT. Nothing about attach timing
    # kept them out: their generators are purely offline
    # (extractor/game_defs/technology_sharing_static_gate.py and
    # focus_defs_static_gate.py, both BATTLE_PLAN_VANILLA + --out-dir, no pid), the
    # recorder had no CLI flag to accept them, and this list had no rows for
    # them. All three are closed; a missing file still throws below rather than
    # silently producing another six-of-eight recording.
    '--static-technology-sharing'          = Join-Path $gameDefsOut 'technology_sharing_static\technology_sharing_static\technology_sharing_catalog.json'
    '--static-focus-defs'                  = Join-Path $gameDefsOut 'focus_defs_static\focus_defs_static\focus_defs_catalog.json'
}

foreach ($path in @($Hoi4Exe, $PdxUserDir, $collectorDll, $collectorRecord, $headlessControl, $readOnlyObserver, $passiveClockWitnessTool)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required path is missing: $path" }
}
$headlessControlSha256 = (
    Get-FileHash -LiteralPath $headlessControl -Algorithm SHA256
).Hash.ToLowerInvariant()
$readOnlyObserverSha256 = (
    Get-FileHash -LiteralPath $readOnlyObserver -Algorithm SHA256
).Hash.ToLowerInvariant()
foreach ($entry in $staticCatalogs.GetEnumerator()) {
    # A control character in one of these literals is invisible in a diff and
    # `Test-Path -LiteralPath` rejects the string as an illegal path, so the
    # capture dies on a "missing catalog" that is really a typo. That is exactly
    # how two rows above were first written: a tab and a form feed where the
    # backslashes belonged, each having swallowed the following letter. Named
    # separately from "missing" so the next one is diagnosed in one read.
    if ($entry.Value -match '[\x00-\x1F]') {
        throw ("Static catalog path for $($entry.Key) contains a control character; " +
               "it is a source typo, not a missing file: " +
               ($entry.Value | ConvertTo-Json))
    }
    if (-not (Test-Path -LiteralPath $entry.Value -PathType Leaf)) {
        throw "Static catalog for $($entry.Key) is missing: $($entry.Value)"
    }
}

# `--profile production` additionally demands a process-bound static bundle
# (`production/profile capture requires --static-bundle-manifest`), which this
# harness does not assemble: it passes loose static catalogs instead. So a late
# attach selects the production roster explicitly, exactly as the shipped
# product worker does, and the recorder derives production_profile.enabled from
# that roster rather than from the flag. Passing no schedule at all would
# silently fall back to the eight-domain compatibility set, so the roster is
# mandatory.
$rosterTool = Join-Path $releaseRoot 'domain_roster.exe'
if (-not (Test-Path -LiteralPath $rosterTool -PathType Leaf)) {
    throw "Production roster tool is missing: $rosterTool (cargo build --release -p recorder_host)"
}
# Read HERE rather than at ready validation. This binary's READY vocabulary is
# what admits the CaptureReadyV2 receipt, so a host that cannot state it would
# otherwise launch the game, settle, inject, and only then refuse -- which is
# how rc_vanilla8's first attempt burned 11 minutes.
[void](Get-PassiveReadyAuthority -RecorderHostPath $collectorRecord)
if ($ScheduledDomainOverride.Count -gt 0) {
    # `powershell -File` binds a comma-joined argument as ONE literal string —
    # it never gets PowerShell's array splitting. The 2026-07-26 crash trials
    # passed "session_header_live,fuel_live" that way; the recorder rejected it
    # as a single unknown domain and exited before injecting, so the whole
    # minimal arm silently measured nothing. Split here so both invocation
    # styles mean the same thing.
    $scheduledDomains = @($ScheduledDomainOverride | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
    # An override that collapses to nothing -- `-ScheduledDomainOverride ','`,
    # or a list of empty strings -- is refused HERE, before the settings guard
    # and the graded window. The derived path already has this check; the
    # override path did not, and the collector does not supply one either: given
    # no schedule it falls back to its eight-domain compatibility set, so the
    # run would capture something entirely different and seal looking
    # successful. New-RecorderArgumentPlan refuses it again as a backstop, but
    # that is inside the graded window and this is not.
    if ($scheduledDomains.Count -eq 0) {
        throw "-ScheduledDomainOverride was supplied but names no domain (received: $($ScheduledDomainOverride -join '|')). An empty roster does not fail the collector; it silently selects the eight-domain compatibility set."
    }
    Write-Warning "Using an explicit $($scheduledDomains.Count)-domain roster override; this is a diagnostic configuration, not production coverage."
} else {
    $rosterResult = Invoke-BoundedNativeCapture `
        -FilePath $rosterTool `
        -Arguments @('--native-root', $nativeRoot) `
        -TimeoutSeconds 30 `
        -Purpose 'production domain roster derivation'
    $scheduledDomains = @(
        $rosterResult.Stdout -split '\r?\n' |
            ForEach-Object { $_.Trim() } |
            Where-Object { $_ }
    )
    if ($scheduledDomains.Count -eq 0) { throw 'Derived production domain roster is empty.' }
}
$scheduledDomainRosterSha256 = Get-Utf8Sha256Hex -Value (
    $scheduledDomains -join "`n"
)

$Hoi4Exe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$PdxUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$observedExeSha256 = (Get-FileHash -LiteralPath $Hoi4Exe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($observedExeSha256 -cne $expectedExeSha256) {
    throw "hoi4.exe is not the pinned exact-a729 build: $observedExeSha256"
}
$collectorDllSha256 = (
    Get-FileHash -LiteralPath $collectorDll -Algorithm SHA256
).Hash.ToLowerInvariant()

$stamp = Get-Date -Format 'yyyyMMdd_HHmmss'
if ([string]::IsNullOrWhiteSpace($OutDir)) {
    $OutDir = Join-Path $repoRoot "extractor\native\runs\headless_raw_$stamp"
}
if ([string]::IsNullOrWhiteSpace($ContentCensusCacheDir)) {
    $ContentCensusCacheDir = Join-Path $repoRoot 'captures\.content_census_cache'
}
$ContentCensusCacheDir = [System.IO.Path]::GetFullPath($ContentCensusCacheDir)
if ([string]::IsNullOrWhiteSpace($FoldCacheDir)) {
    $FoldCacheDir = Join-Path $repoRoot 'captures\.fold_cache'
}
$FoldCacheDir = [System.IO.Path]::GetFullPath($FoldCacheDir)
$OutDir = [System.IO.Path]::GetFullPath($OutDir)
if (Test-Path -LiteralPath $OutDir) {
    if (@(Get-ChildItem -LiteralPath $OutDir -Force).Count -ne 0) {
        throw "OutDir must be absent or empty: $OutDir"
    }
} else {
    New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
}

$settingsPath = Join-Path $PdxUserDir 'settings.txt'
$settingsPath = [System.IO.Path]::GetFullPath($settingsPath)
if (-not (Test-Path -LiteralPath $settingsPath -PathType Leaf)) {
    throw "Required settings.txt is missing: '$settingsPath' (derived from PdxUserDir '$PdxUserDir'). The capture harness requires this launch-settings evidence to resolve the running process modset."
}
$settingsGuardCanonicalPath = Get-SettingsGuardCanonicalPath -Path $settingsPath
$settingsGuardKey = Get-SettingsGuardKey -Identity $settingsGuardCanonicalPath
$settingsGuardLegacyKey = Get-SettingsGuardKey -Identity $settingsPath
$settingsGuardLocalAppData = [Environment]::GetFolderPath(
    [Environment+SpecialFolder]::LocalApplicationData
)
if ([string]::IsNullOrWhiteSpace($settingsGuardLocalAppData)) {
    throw "Settings guard cannot create checkout-independent kill-safe recovery evidence because the machine-global LocalApplicationData folder resolved empty for '$settingsPath'."
}
$settingsGuardRoot = Join-Path (
    Join-Path $settingsGuardLocalAppData 'battle-plan'
) 'settings_guard'
$settingsGuardCanonicalSentinel = Join-Path $settingsGuardRoot "$settingsGuardKey.json"
$settingsGuardLegacySentinel = Join-Path $settingsGuardRoot "$settingsGuardLegacyKey.json"
$settingsGuardSentinel = $settingsGuardCanonicalSentinel
$settingsGuardPriorReadOnly = $false
$settingsGuardRestoreRequired = $false
$settingsGuardOwnerStream = $null
$captureSucceeded = $false
$collectorInjectionStarted = $false
$collectorTeardownAuthenticated = $false
$collectorTeardownReceiptPath = Join-Path $OutDir 'CAPTURE_TEARDOWN_V1.json'
$readyReceiptSha256 = $null
$readyPayloadSha256 = $null
$readyPublicReceiptCoreSha256 = $null
$readyAckSha256 = $null
$readySessionIdSha256 = $null
$readyPipeTokenSha256 = $null
$sharedReadyAuthority = $null
# The custody half of the same receipt, published by the verifier even when it
# refuses the capture. See -CustodyAuthorityOut in PassiveCapture.Lifecycle.ps1:
# a refused READY still identifies which collector is instrumenting which
# process, and that is the whole basis on which the recorder's later teardown
# record can be believed. Holding only $sharedReadyAuthority is what stranded
# t20's game and settings guard behind an authority that had been established
# and then discarded.
$sharedReadyCustodyAuthority = $null
$sharedTeardownAuthority = $null
$gradedWindowStarted = $false
$clientDrivingOptInSuspended = $null
$recorderDeadline = $null
# Declared with the deadline it belongs to: cleanup reports the budget it waited
# out, and under StrictMode reading it on a route that threw between
# Start-Process and the budget assignment would itself be a terminating error
# inside the cleanup.
$recorderBudgetSeconds = 0
# Absolute backstop for the progress-extended finalize wait, and the reason the
# last extension was granted. Both are read by the cleanup path, so both are
# declared with the deadline for the same StrictMode reason.
$recorderFinalizeCeiling = $null
$recorderFinalizeExtensionReason = $null
$scriptExitCode = 1
# Set only by the raw-capture report's UNDEFINED-STATUS-BIT verdict (exit 2).
# Declared here so the teardown path can read it whatever happened upstream.
$undefinedStatusBitFailure = $null
$passiveWitnessPath = Join-Path $OutDir 'passive_speed5_witness.jsonl'
# The hooks-free paired control, written during settle only. Separate file from
# the witness so the verdict authority's schema and its stall policy stay
# untouched by evidence that is deliberately not gated.
$settleControlPath = Join-Path $OutDir 'settle_control.jsonl'
# Append-only measurement of what every bounded control call costs, so the
# 10 s default in Invoke-BoundedHoi4Control is a number with evidence.
$script:ControlLatencyLogPath = Join-Path $OutDir 'bounded_control_latency.jsonl'
# Consumer-facing split of the graded window into advancing, paused, and
# tick-held intervals. A consumer averaging a rate over the capture must divide
# by active time, and a paused world is not slow -- it is stopped, on purpose.
$passiveClockIntervalsPath = Join-Path $OutDir 'PASSIVE_CLOCK_INTERVALS_V2.json'
# What the run knows about its own witness stream, so cleanup can say why the
# stream ends where it does. Sampling stops for exactly one reason -- the
# harness stopped asking -- and the artifacts never said so.
$passiveWitnessSampleCount = 0
$passiveWitnessLastPhase = 'none'
$harnessAbortReason = $null
# The FACT of an abort, distinct from its text. See the outer catch: a
# message-less exception leaves the reason string empty, and the refusal
# arbitration must not read that as "nothing went wrong".
$harnessAborted = $false

# --- named refusals ----------------------------------------------------------
#
# TWO DIFFERENT THINGS, KEPT APART, because they call for opposite handling.
#
# $captureRefusals holds refusals of the CAPTURE: a READY receipt the shared
# verifier would not admit. The capture is over; the recorder is not, and the
# whole point of naming the refusal rather than dying of it is that cleanup
# still waits the recorder out and still authenticates its teardown.
#
# $supervisionFailures holds failures of the HARNESS's own bookkeeping AFTER
# the recorder was launched -- the CPU-affinity pin, the receipt writes beside
# it. Run t38 is what this list exists for: a parameter-binding error in the
# affinity path threw out of the capture phase, and because the recorder budget
# had not been computed yet, cleanup's wait loop found a null deadline, skipped
# itself entirely, and abandoned a recorder that then ran to completion, sealed
# a 36 MB replay and published its own teardown receipt with nobody watching.
# The right answer was never "collapse": it was "name it, leave the recorder
# alone, and keep waiting for its receipts".
#
# NEITHER list makes an attempt succeed. A capture whose mask did not take is
# not a floor measurement, and a refused READY is a refused READY. They make it
# FAIL BY NAME, at the end of its normal accounting, at exit code 3.
$captureRefusals = [Collections.Generic.List[object]]::new()
$supervisionFailures = [Collections.Generic.List[string]]::new()
$captureRefusalReceiptPath = Join-Path $OutDir 'CAPTURE_REFUSAL_V1.json'
# Set by the LAST statement of the cleanup prefix, and read only by the refusal
# arbitration. There is no way to ask PowerShell "is an exception in flight?"
# from inside a `finally`, and the refusal code must never be the thing that
# suppresses one: $ErrorActionPreference is 'Stop', so any failing cmdlet in the
# cleanup prefix is terminating, and an `exit` in the enclosing `finally` would
# swallow it and report a refusal instead of the failure.
$cleanupPrefixCompleted = $false

# Runs one piece of post-launch supervision and REFUSES TO LET IT ABANDON THE
# ATTEMPT.
#
# Scope is deliberately narrow and is enforced by the caller, not by this
# function: it is only ever wrapped around bookkeeping whose failure is not a
# verdict about the client or about hook custody. Anything that decides whether
# the game may be touched, whether the graded window may close, or whether a
# receipt may be believed must keep throwing -- containment there would convert
# a custody violation into a warning, which is the failure this whole file is
# built to prevent.
function Invoke-CaptureSupervisionStep {
    param(
        [Parameter(Mandatory = $true)][string]$Step,
        [Parameter(Mandatory = $true)][scriptblock]$Action,
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [Collections.Generic.List[string]]$Failures
    )

    try {
        [void](& $Action)
        return $true
    } catch {
        # Message first, type as the fallback: an exception with an empty
        # message would otherwise record a step that failed for no stated
        # reason, which is the unnamed strand in miniature.
        $reason = ''
        try { $reason = ([string]$_.Exception.Message -replace '\s+', ' ').Trim() } catch { }
        if ([string]::IsNullOrWhiteSpace($reason)) {
            $reason = 'no message'
            try { $reason = [string]$_.Exception.GetType().FullName } catch { }
        }
        $Failures.Add("${Step}: $reason")
        # NOT Write-Error: $ErrorActionPreference is 'Stop' for this script, so
        # Write-Error is terminating and would re-throw the very failure this
        # function just contained.
        Write-Host "phase=supervision_contained step=$Step reason=$reason"
        return $false
    }
}

# The refusal receipt, rewritten in full every time a refusal is added.
#
# Non-throwing on purpose and for the same reason as everything else in the
# cleanup path: a run that cannot write its refusal receipt still has to finish
# its accounting and still has to return the refusal code. The receipt is the
# durable copy of what the transcript already said, not the only copy.
function Write-CaptureRefusalReceipt {
    try {
        if (-not (Test-Path -LiteralPath $OutDir -PathType Container)) { return }
        ([ordered]@{
            schema               = 'hoi4-capture-refusal-v1'
            out_dir              = $OutDir
            session_id           = $sessionId
            written_at_utc       = [DateTime]::UtcNow.ToString('o')
            capture_refusals     = @($captureRefusals.ToArray())
            supervision_failures = [string[]]$supervisionFailures.ToArray()
        } | ConvertTo-Json -Depth 8) |
            Set-Content -LiteralPath $captureRefusalReceiptPath -Encoding UTF8
    } catch {
        Write-Host "phase=capture_refusal_receipt_failed reason=$(($_.Exception.Message -replace '\s+', ' ').Trim())"
    }
}

function Write-PassiveWitnessSample {
    param(
        [Parameter(Mandatory = $true)][string]$Phase,
        [Parameter(Mandatory = $true)][int]$TargetProcessId,
        [Parameter(Mandatory = $true)][uint64]$ExpectedCreationFiletimeUtc,
        [Parameter(Mandatory = $true)][int]$ExpectedSpeed
    )

    $state = Invoke-BoundedHoi4Observation `
        -TargetProcessId $TargetProcessId `
        -TimeoutSeconds 30 `
        -ExpectedCreationFiletimeUtc $ExpectedCreationFiletimeUtc
    # Both ends of the bracket come from the observer, which is the only place
    # that knows when its own window opened and closed. Timestamping here would
    # add this process's subprocess teardown to the interval and make the
    # bracket unsatisfiable.
    $sample = [ordered]@{
        schema                         = 'hoi4-passive-client-state-witness-v3'
        observed_at_utc                = [string]$state.ObservedAtUtc
        observed_at_unix_ms            = [int64]$state.ObservedAtUnixMs
        phase                          = $Phase
        pid                            = $TargetProcessId
        process_creation_filetime_utc  = $ExpectedCreationFiletimeUtc
        exe_sha256                     = [string]$state.ExeSha256
        observer_sha256                = $readOnlyObserverSha256
        access                         = 'PROCESS_QUERY_INFORMATION|PROCESS_VM_READ'
        expected_speed                 = $ExpectedSpeed
        speed_initial                  = [int]$state.InitialSpeed
        speed_final                    = [int]$state.Speed
        paused_initial                 = [bool]$state.InitialPaused
        paused_final                   = [bool]$state.Paused
        pause_forced_initial           = [bool]$state.InitialPauseForced
        pause_forced_final             = [bool]$state.PauseForced
        tick_inhibited_initial         = [bool]$state.InitialTickInhibited
        tick_inhibited_final           = [bool]$state.TickInhibited
        paused_in_window               = [bool]$state.PausedInWindow
        pause_forced_in_window         = [bool]$state.PauseForcedInWindow
        tick_inhibited_in_window       = [bool]$state.TickInhibitedInWindow
        hour_a                         = [int]$state.HourA
        hour_b                         = [int]$state.HourB
        hour_delta                     = [int]$state.HourDelta
        tick_progress_bits_a           = [int]$state.TickProgressBitsA
        tick_progress_bits_b           = [int]$state.TickProgressBitsB
        window_started_at_utc          = [string]$state.WindowStartedAtUtc
        window_started_at_unix_ms      = [int64]$state.WindowStartedAtUnixMs
        observed_window_ms             = [int64]$state.ObservedWindowMs
        clock_progress_waited_ms       = [int64]$state.ClockProgressWaitedMs
        clock_progress_budget_ms       = [int64]$state.ClockProgressBudgetMs
        processor_milliseconds         = [int64]$state.ProcessorMilliseconds
        # HOST AND PACING CONTEXT (v3). Everything above this line describes
        # the engine; nothing above it describes the machine the engine was
        # running on, and that gap is what made lane TG's session confound
        # cost a whole lane to diagnose. The observer holds all three already
        # -- the affinity-aware processor count from its own environment, the
        # fault count and resident bytes off the handle it opened for the
        # reads above -- so this is a wider row, not a wider access.
        available_parallelism          = [int]$state.AvailableParallelism
        # DELIBERATELY UNCAST. Each is $null when its query failed, and
        # [int]$null is 0 -- casting would publish "zero cores" or "zero
        # resident bytes" as if measured. The observer already typed them.
        target_processor_affinity_cores = $state.TargetProcessorAffinityCores
        target_page_fault_count        = $state.TargetPageFaultCount
        target_working_set_bytes       = $state.TargetWorkingSetBytes
        host_state_unavailable         = $state.HostStateUnavailable
        advancing                      = [bool]$state.Advancing
    }
    # Record BEFORE deciding. The previous order threw on rejection and wrote
    # the sample afterwards, so the witness file was structurally incapable of
    # holding the one observation that ended a run: every surviving sample was
    # a success and the abort was reconstructed from a message string. Two
    # confidently wrong diagnoses of run 20260730-postmerge-04 came from that.
    ($sample | ConvertTo-Json -Compress -Depth 8) |
        Add-Content -LiteralPath $passiveWitnessPath -Encoding UTF8
    $script:passiveWitnessSampleCount = $script:passiveWitnessSampleCount + 1
    $script:passiveWitnessLastPhase = $Phase
    Write-Host (
        "phase=passive_witness witness_phase=$Phase speed=$($state.Speed) " +
        "paused=$($state.Paused) pause_forced=$($state.PauseForced) " +
        "tick_inhibited=$($state.TickInhibited) advancing=$($state.Advancing) " +
        "hour_a=$($state.HourA) hour_b=$($state.HourB) " +
        "window_ms=$($state.ObservedWindowMs)"
    )
    Assert-PassiveWitnessVerdict
    return $state
}

# Asks the verdict authority whether the accumulated witness permits the run to
# continue. The three questions the old single boolean conflated -- did the
# recorder write to the client, is the engine executing, is anything worth
# recording happening -- are answered separately there, with named failures,
# and under Rust tests. The harness holds no opinion of its own.
function Assert-PassiveWitnessVerdict {
    if (-not (Test-Path -LiteralPath $passiveClockWitnessTool -PathType Leaf)) {
        throw "The passive witness verdict authority is missing: '$passiveClockWitnessTool'. Build the native workspace before capturing."
    }
    $verdictArgs = @(
        '--witness', $passiveWitnessPath,
        '--intervals-out', $passiveClockIntervalsPath,
        '--stall-samples', "$StallEscalationSamples",
        '--stall-span-ms', "$($StallEscalationSeconds * 1000)"
    )
    $result = Invoke-BoundedNativeCapture `
        -FilePath $passiveClockWitnessTool `
        -Arguments $verdictArgs `
        -TimeoutSeconds 30 `
        -Purpose 'passive witness verdict' `
        -AllowNonzeroExit
    if ([int]$result.ExitCode -eq 0) {
        if (-not [string]::IsNullOrWhiteSpace($result.Stdout)) {
            Write-Host $result.Stdout.Trim()
        }
        return
    }
    $reason = $result.Stderr.Trim()
    if ([string]::IsNullOrWhiteSpace($reason)) {
        $reason = "the verdict authority exited $($result.ExitCode) without a reason"
    }
    throw "Passive capture stopped. $reason"
}

# Is the collector provably out of the client, WITHOUT an admitted teardown
# receipt?
#
# The graded window exists so nothing touches the client while hook custody is
# unresolved, and everything the harness owes an operator afterwards -- stopping
# the game it launched, releasing the settings guard, giving the affinity mask
# back -- is gated on closing it. A READY receipt the verifier cannot admit is
# not evidence that the hooks are still in: e2e_01 was refused on two stale
# PowerShell constants while HOOK_RESTORATION_OUTCOMES_V1.json beside it recorded
# 64 of 64 pinned sites and 2 of 2 import slots back at their baseline digest and
# the collector module unloaded. The harness held the window open anyway, so it
# left the game running and settings.txt read-only for a human to reason about
# from receipts by hand.
#
# The recorder must have EXITED first: a live one still owns the lease. And
# the roster is the CONTRACT'S, not the receipt's own: a self-consistent
# document proves only that the rows it still carries say "restored", which a
# truncated report satisfies while omitted hooks stay installed.
function Test-CollectorHookReleaseProof {
    param(
        [Parameter(Mandatory = $true)][string]$RunDir,
        # MANDATORY AND NON-NULL. Injection started, so a recorder exists; a
        # $null here would mean "nothing to wait for" and admit a run whose
        # writer was never observed at all.
        [Parameter(Mandatory = $true)][AllowNull()]$RecorderProcess,
        # The canonical roster, derived from the contract rather than from the
        # receipt. Self-consistency proves only that every row PRESENT says
        # restored; it says nothing about the rows that are missing.
        [Parameter(Mandatory = $true)][object]$HookAuthority,
        [Parameter(Mandatory = $true)][ref]$ProofOut
    )

    $ProofOut.Value = ''
    if ($null -eq $RecorderProcess) {
        $ProofOut.Value = 'no collector_record process was observed for this attempt'
        return $false
    }
    try {
        $RecorderProcess.Refresh()
        if (-not $RecorderProcess.HasExited) {
            $ProofOut.Value = "collector_record PID $($RecorderProcess.Id) is still running"
            return $false
        }
    } catch {
        $ProofOut.Value = 'collector_record liveness is unreadable'
        return $false
    }
    $expectedNames = @($HookAuthority.LogicalNames)
    $expectedSlotCount = [int]$HookAuthority.ImportSlotCount
    if ($expectedNames.Count -eq 0 -or $expectedSlotCount -le 0) {
        $ProofOut.Value = 'the pinned hook roster authority names no site or no import slot'
        return $false
    }
    $receiptPath = Join-Path $RunDir 'HOOK_RESTORATION_OUTCOMES_V1.json'
    if (-not (Test-Path -LiteralPath $receiptPath -PathType Leaf)) {
        $ProofOut.Value = 'the collector published no hook-restoration receipt'
        return $false
    }
    $receipt = $null
    try {
        $receipt = [IO.File]::ReadAllText($receiptPath) | ConvertFrom-Json
    } catch {
        $ProofOut.Value = "the hook-restoration receipt is unreadable: $(($_.Exception.Message -replace '\s+', ' ').Trim())"
        return $false
    }
    try {
        Assert-ExactJsonPropertySet `
            -Value $receipt `
            -Expected @(
                'schema', 'outcome', 'collector_module_disposition',
                'restoration_wait_ms', 'pinned_hook_site_count',
                'unrestored_hook_site_count', 'import_slot_count',
                'unrestored_import_slot_count',
                'pinned_hook_site_baseline_sha256',
                'pinned_hook_site_observed_sha256',
                'unrestored_hook_names', 'unrestored_hook_domains',
                'sites', 'import_slots'
            ) `
            -Label 'HookRestorationOutcomesV1'
    } catch {
        $ProofOut.Value = "the hook-restoration receipt has an unexpected shape: $(($_.Exception.Message -replace '\s+', ' ').Trim())"
        return $false
    }

    $failures = [Collections.Generic.List[string]]::new()
    # Each entry is read the same way, so a roster row cannot escape the name
    # binding by being shaped differently from its neighbours.
    function Read-HookRosterNames {
        param($Entries, [string]$Label, $Failures)

        $names = [Collections.Generic.List[string]]::new()
        foreach ($entry in @($Entries)) {
            if ($null -eq $entry -or $entry -is [array] -or
                $entry -is [string] -or $entry -is [valuetype]) {
                [void]$Failures.Add("$Label has an entry that is not an object")
                continue
            }
            $fields = @($entry.PSObject.Properties.Name)
            if (@($fields | Where-Object { $_ -ceq 'logical_name' }).Count -ne 1 -or
                @($fields | Where-Object { $_ -ceq 'status' }).Count -ne 1 -or
                @($fields | Where-Object { $_ -cnotin @('logical_name', 'domain', 'status') }).Count -ne 0) {
                [void]$Failures.Add("$Label has an entry whose fields are $($fields -join ',')")
                continue
            }
            if ($entry.logical_name -isnot [string] -or
                [string]::IsNullOrEmpty([string]$entry.logical_name)) {
                [void]$Failures.Add("$Label has an entry with no logical name")
                continue
            }
            if ([string]$entry.status -cne 'restored') {
                [void]$Failures.Add("$Label entry '$($entry.logical_name)' is '$($entry.status)'")
            }
            [void]$names.Add([string]$entry.logical_name)
        }
        return $names
    }

    # @() ON THE RESULT. PowerShell unrolls a returned collection, so an empty
    # roster comes back as $null and every .Count below dies under StrictMode --
    # which is a THROW out of the cleanup prefix, not a refusal.
    $siteNames = @(Read-HookRosterNames -Entries $receipt.sites -Label 'sites' -Failures $failures)
    $slotNames = @(Read-HookRosterNames -Entries $receipt.import_slots -Label 'import_slots' -Failures $failures)
    # ORDINAL distinctness. `Sort-Object -Unique` folds case, so two roster names
    # differing only in case would collapse into one. Both directions of that are
    # already fail-closed here, but the count this feeds is meant to be "how many
    # DIFFERENT names did the receipt list", and that is an ordinal question.
    $distinctSites = @([Linq.Enumerable]::Distinct(
        [string[]]$siteNames, [StringComparer]::Ordinal))
    $distinctSlots = @([Linq.Enumerable]::Distinct(
        [string[]]$slotNames, [StringComparer]::Ordinal))
    # SET EQUALITY WITH THE CONTRACT'S ROSTER, both directions. A truncated
    # report -- every row it still carries saying "restored" -- is exactly the
    # document self-consistency admits and this refuses.
    $missing = @($expectedNames | Where-Object { $_ -cnotin $distinctSites })
    $foreign = @($distinctSites | Where-Object { $_ -cnotin $expectedNames })
    if ($missing.Count -ne 0 -or $foreign.Count -ne 0 -or
        $distinctSites.Count -ne $siteNames.Count) {
        [void]$failures.Add(
            "sites roster: missing=$($missing.Count) foreign=$($foreign.Count) " +
            "duplicated=$($siteNames.Count - $distinctSites.Count) " +
            "first_missing=$(if ($missing.Count -gt 0) { $missing[0] } else { '-' }) " +
            "first_foreign=$(if ($foreign.Count -gt 0) { $foreign[0] } else { '-' })")
    }
    # The import slots are the subset of the same roster that is hooked through
    # an import table, so they are bound to it too and their count is the
    # contract's, never the receipt's own.
    $foreignSlots = @($distinctSlots | Where-Object { $_ -cnotin $expectedNames })
    if ($distinctSlots.Count -ne $expectedSlotCount -or
        $foreignSlots.Count -ne 0 -or
        $distinctSlots.Count -ne $slotNames.Count) {
        [void]$failures.Add(
            "import_slots roster: named=$($distinctSlots.Count) expected=$expectedSlotCount " +
            "foreign=$($foreignSlots.Count) duplicated=$($slotNames.Count - $distinctSlots.Count)")
    }

    $baseline = [string]$receipt.pinned_hook_site_baseline_sha256
    $observed = [string]$receipt.pinned_hook_site_observed_sha256
    if ([string]$receipt.schema -cne 'hoi4-hook-restoration-outcomes-v1') {
        [void]$failures.Add("schema=$($receipt.schema)")
    }
    if ([string]$receipt.outcome -cne 'complete') {
        [void]$failures.Add("outcome=$($receipt.outcome)")
    }
    # "unloaded" means OBSERVED unloaded: the recorder no longer substitutes it
    # for a terminal state it could not read (it publishes nothing instead).
    if ([string]$receipt.collector_module_disposition -cne 'unloaded') {
        [void]$failures.Add("collector_module_disposition=$($receipt.collector_module_disposition)")
    }
    if ([int]$receipt.pinned_hook_site_count -ne $expectedNames.Count) {
        [void]$failures.Add("pinned_hook_site_count=$($receipt.pinned_hook_site_count) expected=$($expectedNames.Count)")
    }
    if ([int]$receipt.import_slot_count -ne $expectedSlotCount) {
        [void]$failures.Add("import_slot_count=$($receipt.import_slot_count) expected=$expectedSlotCount")
    }
    if ([int]$receipt.unrestored_hook_site_count -ne 0 -or
        [int]$receipt.unrestored_import_slot_count -ne 0 -or
        @($receipt.unrestored_hook_names).Count -ne 0 -or
        @($receipt.unrestored_hook_domains).Count -ne 0) {
        [void]$failures.Add(
            "unrestored sites=$($receipt.unrestored_hook_site_count) " +
            "slots=$($receipt.unrestored_import_slot_count) " +
            "named=$(@($receipt.unrestored_hook_names) -join ',')")
    }
    if ($baseline -cnotmatch '^[0-9a-f]{64}$' -or $baseline -cmatch '^0{64}$' -or
        $observed -cne $baseline) {
        [void]$failures.Add("baseline=$baseline observed=$observed")
    }
    if ($failures.Count -ne 0) {
        $ProofOut.Value = "the hook-restoration receipt does not prove release: $($failures -join '; ')"
        return $false
    }
    $ProofOut.Value = (
        "hook_sites=$($distinctSites.Count) import_slots=$($distinctSlots.Count) " +
        "roster=contract module=unloaded baseline_sha256=$baseline receipt=$receiptPath")
    return $true
}

function Confirm-CollectorTeardownReceipt {
    # An admitted READY is preferred, but a CUSTODY-only READY is enough to
    # decide the only question this function asks: did the collector give the
    # client's code back. Refusing to ask it because the capture was refused
    # does not make the client safer -- it makes the hooks unaccounted for,
    # holds the graded window open, and leaves the settings guard applied and
    # the game running for a human to reason about from receipts by hand.
    $authority = $sharedReadyAuthority
    if ($null -eq $authority) {
        $authority = $sharedReadyCustodyAuthority
    }
    if ($null -eq $authority) {
        # NAME THE REASON THE AUTHORITY IS MISSING. Both authorities are $null
        # only because READY validation threw before either was published, and
        # the operator reading this warning is the one who then has to decide
        # by hand whether the hooks are gone. Run t36 got the bare sentence,
        # while the sentence that explained it -- "Passive READY wire field
        # order differs at ..." -- was printed elsewhere as a witness-stop
        # reason, so the two had to be joined up by eye. $harnessAbortReason is
        # already recorded by the catch that routes every failure here.
        $reason = $harnessAbortReason
        if ([string]::IsNullOrWhiteSpace([string]$reason)) {
            $reason = 'READY validation published neither an admitted nor a custody authority, and recorded no reason'
        }
        throw (
            'Collector teardown cannot be admitted without shared READY ' +
            "authority. READY validation failed with: $reason"
        )
    }
    $script:sharedTeardownAuthority = Test-PassiveCaptureTeardownReceipt `
        -RunDir $OutDir `
        -TargetPid ([uint32]$gamePid) `
        -TargetCreationFiletimeUtc $gameCreationFiletimeUtc `
        -SessionId $sessionId `
        -CaptureId $captureId `
        -ReadyAuthority $authority
    if (-not [bool]$authority.CaptureAdmitted) {
        Write-Host (
            'phase=teardown_admitted_on_custody_only ' +
            "ready_admission_failures=$($authority.AdmissionFailures.Count) " +
            'note=hook_custody_is_settled_the_capture_is_still_refused'
        )
    }
    return $true
}

# --- 4c floor emulation: CPU affinity ---------------------------------------
#
# Every published 4-core number for this capture is DERIVED from measurements
# taken on a 16-logical-processor dev machine; none of them has been measured
# with four cores. These functions make the core-count half of the floor
# testable here by pinning the engine and the recorder to the same n physical
# cores, so they compete for those cores exactly as they would on an n-core box.
#
# The topology comes from the OS rather than from an assumption about how
# logical processors are numbered. Sibling numbering is a convention, not a
# contract, and on a hybrid part the cores are not even interchangeable; a mask
# built from a guessed layout is wrong in a way that still produces a plausible
# number, which is the worst failure available here.
if ($null -eq ('Hoi4MpStats.CpuTopologyNativeMethods' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace Hoi4MpStats {
    public static class CpuTopologyNativeMethods {
        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool GetLogicalProcessorInformationEx(
            int relationshipType,
            IntPtr buffer,
            ref uint returnedLength);
    }
}
'@
}

# KAFFINITY is pointer-width and PowerShell has no unsigned shift that survives
# bit 63, so masks are carried as [uint64] and manipulated through their bytes.
function Get-Hoi4AffinityBitIndexes {
    param([Parameter(Mandatory = $true)][uint64]$Mask)

    $bits = [System.Collections.BitArray]::new(
        [System.BitConverter]::GetBytes($Mask)
    )
    $indexes = New-Object System.Collections.Generic.List[int]
    for ($bit = 0; $bit -lt $bits.Length; $bit++) {
        if ($bits[$bit]) { [void]$indexes.Add($bit) }
    }
    return $indexes.ToArray()
}

function New-Hoi4AffinityMask {
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][int[]]$BitIndexes)

    $bytes = New-Object byte[] 8
    foreach ($bit in $BitIndexes) {
        if ($bit -lt 0 -or $bit -ge 64) {
            throw "Logical processor index $bit is outside the 64-bit affinity mask of a single processor group."
        }
        $byte = [int][math]::Floor($bit / 8)
        $bytes[$byte] = [byte]($bytes[$byte] -bor (1 -shl ($bit % 8)))
    }
    return [System.BitConverter]::ToUInt64($bytes, 0)
}

function ConvertTo-Hoi4AffinityUInt64 {
    param([Parameter(Mandatory = $true)][IntPtr]$Value)

    # A mask with bit 63 set reads back as a NEGATIVE Int64, and a direct
    # [uint64] cast of that throws. Reinterpret the bytes instead.
    return [System.BitConverter]::ToUInt64(
        [System.BitConverter]::GetBytes($Value.ToInt64()),
        0
    )
}

function Format-Hoi4AffinityMask {
    param([Parameter(Mandatory = $true)][uint64]$Mask)

    return ('0x{0:x16}' -f $Mask)
}

# One record per PHYSICAL core, carrying the affinity mask of every logical
# processor on that core plus its efficiency class and processor group.
function Get-Hoi4PhysicalCoreTopology {
    $relationProcessorCore = 0
    [uint32]$length = 0
    $null = [Hoi4MpStats.CpuTopologyNativeMethods]::GetLogicalProcessorInformationEx(
        $relationProcessorCore,
        [IntPtr]::Zero,
        [ref]$length
    )
    $sizeError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    if ($length -eq 0) {
        throw [ComponentModel.Win32Exception]::new(
            $sizeError,
            'GetLogicalProcessorInformationEx could not size the processor-core table'
        )
    }
    $buffer = [Runtime.InteropServices.Marshal]::AllocHGlobal([int]$length)
    try {
        if (-not [Hoi4MpStats.CpuTopologyNativeMethods]::GetLogicalProcessorInformationEx(
                $relationProcessorCore,
                $buffer,
                [ref]$length)) {
            throw [ComponentModel.Win32Exception]::new(
                [Runtime.InteropServices.Marshal]::GetLastWin32Error(),
                'GetLogicalProcessorInformationEx failed for RelationProcessorCore'
            )
        }
        $cores = New-Object System.Collections.Generic.List[object]
        $offset = 0
        $coreIndex = 0
        while ($offset -lt [int]$length) {
            $record = [IntPtr]::Add($buffer, $offset)
            $relationship = [Runtime.InteropServices.Marshal]::ReadInt32($record, 0)
            $size = [Runtime.InteropServices.Marshal]::ReadInt32($record, 4)
            # A zero or negative size would walk this loop forever or backwards.
            if ($size -le 0) {
                throw "GetLogicalProcessorInformationEx returned a $size-byte record; the processor-core table cannot be walked."
            }
            if ($offset + $size -gt [int]$length) {
                throw "GetLogicalProcessorInformationEx record at offset $offset claims $size bytes, past the $length-byte table."
            }
            if ($relationship -eq $relationProcessorCore) {
                # PROCESSOR_RELATIONSHIP begins at offset 8: Flags, then
                # EfficiencyClass, 20 reserved bytes, GroupCount at 30, and the
                # GROUP_AFFINITY array (16 bytes each on x64) at 32.
                $efficiencyClass = [int][Runtime.InteropServices.Marshal]::ReadByte($record, 9)
                $groupCount = [int][Runtime.InteropServices.Marshal]::ReadInt16($record, 30)
                if ($groupCount -ne 1) {
                    throw "Physical core $coreIndex spans $groupCount processor groups. A single-mask affinity plan cannot express that; run the floor measurement on a machine with one processor group."
                }
                $mask = [System.BitConverter]::ToUInt64(
                    [System.BitConverter]::GetBytes(
                        [Runtime.InteropServices.Marshal]::ReadInt64($record, 32)
                    ),
                    0
                )
                $group = [int][Runtime.InteropServices.Marshal]::ReadInt16($record, 40)
                $logicalProcessors = @(Get-Hoi4AffinityBitIndexes -Mask $mask)
                if ($logicalProcessors.Count -eq 0) {
                    throw "Physical core $coreIndex reported an empty affinity mask."
                }
                [void]$cores.Add([pscustomobject]@{
                    CoreIndex                   = $coreIndex
                    EfficiencyClass             = $efficiencyClass
                    Group                       = $group
                    Mask                        = $mask
                    LogicalProcessors           = $logicalProcessors
                    PrimaryLogicalProcessor     = $logicalProcessors[0]
                })
                $coreIndex++
            }
            $offset += $size
        }
        if ($cores.Count -eq 0) {
            throw 'GetLogicalProcessorInformationEx reported no physical cores.'
        }
        return $cores.ToArray()
    } finally {
        [Runtime.InteropServices.Marshal]::FreeHGlobal($buffer)
    }
}

# Choose the cores and produce the one mask both processes will share. They
# SHARE it deliberately: a floor machine has n cores in total, not n for the
# game and another n for the recorder, and the whole question this switch exists
# to answer is what happens when the engine and the sweep compete for them.
function Get-Hoi4CpuAffinityPlan {
    param(
        [Parameter(Mandatory = $true)][int]$CoreCount,
        [Parameter(Mandatory = $true)][bool]$OneThreadPerCore
    )

    if ($CoreCount -lt 2) {
        throw "A CPU affinity plan needs at least 2 physical cores; $CoreCount was requested."
    }
    $topology = @(Get-Hoi4PhysicalCoreTopology)

    # Keep the selection homogeneous. On a hybrid part, four cores drawn from
    # both classes is not a 4-core machine anyone ships, and the resulting
    # number could not be compared with anything. Prefer the class with the most
    # cores, and on a tie the more performant one (a higher EfficiencyClass is
    # the faster class).
    $classes = @(
        $topology |
            Group-Object -Property EfficiencyClass |
            Sort-Object `
                @{ Expression = { $_.Count }; Descending = $true }, `
                @{ Expression = { [int]$_.Name }; Descending = $true }
    )
    $chosenClass = $classes[0]
    $available = @($chosenClass.Group | Sort-Object -Property CoreIndex)
    if ($available.Count -lt $CoreCount) {
        $inventory = (
            $classes | ForEach-Object { "class $($_.Name)=$($_.Count)" }
        ) -join ', '
        throw "-CpuAffinityCores $CoreCount exceeds the largest homogeneous core group on this machine ($inventory). Pick a count that fits one class."
    }
    $selected = @($available[0..($CoreCount - 1)])

    $groups = @($selected | ForEach-Object { $_.Group } | Sort-Object -Unique)
    if ($groups.Count -ne 1) {
        throw "The selected cores span processor groups $($groups -join ','). Affinity masks are per-group; this plan cannot be applied."
    }

    # Refuse anything this script cannot express, rather than applying a mask
    # that reads back correct and means something else.
    #
    # `Process.ProcessorAffinity` is SetProcessAffinityMask, which carries a
    # bare KAFFINITY and no group. Windows applies it to the process's PRIMARY
    # group. So on a multi-group machine a plan whose cores live in group 1
    # would put group 1's bit NUMBERS onto group 0's processors -- different
    # cores entirely -- and the numeric read-back below would confirm it,
    # because the number is what it asked for. The measurement would be wrong
    # and would look right, which is the one failure worth failing closed on.
    # (Codex review 019fdc22-13b0-7c93-a8fc-f71ae498a852, finding 9.)
    $machineGroups = @($topology | ForEach-Object { $_.Group } | Sort-Object -Unique)
    if ($machineGroups.Count -ne 1 -or [int]$machineGroups[0] -ne 0 -or [int]$groups[0] -ne 0) {
        throw "This machine reports processor group(s) $($machineGroups -join ',') and the plan selected group $($groups[0]). The affinity API this harness uses carries no group and silently targets the process's primary group, so only a single-group (group 0) machine can be masked correctly here. Use a group-aware tool or a single-group machine."
    }

    $bits = New-Object System.Collections.Generic.List[int]
    foreach ($core in $selected) {
        if ($OneThreadPerCore) {
            [void]$bits.Add($core.PrimaryLogicalProcessor)
        } else {
            foreach ($logical in $core.LogicalProcessors) { [void]$bits.Add($logical) }
        }
    }
    $mask = New-Hoi4AffinityMask -BitIndexes $bits.ToArray()
    if ($mask -eq 0) {
        throw 'The computed affinity mask selected no logical processor.'
    }

    return [pscustomobject]@{
        RequestedCores            = $CoreCount
        OneThreadPerCore          = $OneThreadPerCore
        EfficiencyClass           = [int]$chosenClass.Name
        Group                     = [int]$groups[0]
        Mask                      = $mask
        MaskHex                   = (Format-Hoi4AffinityMask -Mask $mask)
        SelectedCoreIndexes       = @($selected | ForEach-Object { $_.CoreIndex })
        LogicalProcessors         = @(Get-Hoi4AffinityBitIndexes -Mask $mask)
        MachinePhysicalCores      = $topology.Count
        MachineLogicalProcessors  = ($topology | ForEach-Object { $_.LogicalProcessors.Count } | Measure-Object -Sum).Sum
    }
}

# Apply the mask to one already-created process and PROVE it took.
#
# Post-create is the only option for the game: it is launched through
# Win32_Process.Create (see the launch path below) precisely so it inherits
# nothing from this host, and WMI offers no creation-time affinity. The PID is
# therefore re-bound to its creation epoch here rather than trusted, because
# between the launch and this call the only thing tying a number to a process is
# that epoch. Threads inherit the process affinity at creation, so applying this
# before injection also covers every reader lane the collector starts later.
function Set-Hoi4ProcessAffinity {
    param(
        [Parameter(Mandatory = $true)][int]$ProcessId,
        [Parameter(Mandatory = $true)][uint64]$Mask,
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)][AllowNull()][object]$ExpectedCreationFiletimeUtc,
        # Receives the mask this process had BEFORE the change, published the
        # instant it is read and before anything is written. The return value
        # cannot do that job: this function can mutate a user-owned game and
        # then throw during verification, and a caller that learns the prior
        # mask only from a successful return has no way to put it back.
        # (Codex review 019fdc22-13b0-7c93-a8fc-f71ae498a852, finding 8.)
        #
        # Typed [object], not [ref]: PowerShell 5.1 refuses to bind a literal
        # $null to a [ref] parameter no matter what [AllowNull()] says, and the
        # callers that do not want the prior mask pass exactly that. The [ref]
        # requirement is enforced below instead, where $null is legal.
        [AllowNull()][object]$PriorMaskOut
    )

    if ($null -ne $PriorMaskOut -and $PriorMaskOut -isnot [ref]) {
        throw "PriorMaskOut must be a [ref] cell or `$null; got [$($PriorMaskOut.GetType().FullName)]."
    }
    $process = Get-Process -Id $ProcessId -ErrorAction Stop
    if ($null -ne $ExpectedCreationFiletimeUtc) {
        $observed = [uint64]($process.StartTime.ToUniversalTime().ToFileTimeUtc())
        if ($observed -ne [uint64]$ExpectedCreationFiletimeUtc) {
            throw "$Label PID $ProcessId creation epoch changed before its CPU affinity was applied (expected=$ExpectedCreationFiletimeUtc observed=$observed)."
        }
    }
    # A PID whose process has just exited is still resolvable for as long as
    # something holds a handle on it, and `ProcessorAffinity` then reads back as
    # $null rather than throwing. Without this the run reports "Cannot convert
    # null to type System.IntPtr" -- a PowerShell type error standing in for the
    # only fact that matters, which is that the process is gone.
    if ($process.HasExited) {
        throw "$Label PID $ProcessId has exited; its CPU affinity cannot be set."
    }
    $priorAffinity = $process.ProcessorAffinity
    if ($null -eq $priorAffinity) {
        throw "$Label PID $ProcessId exposed no processor affinity mask; the process is exiting or the handle lost its query right."
    }
    $prior = ConvertTo-Hoi4AffinityUInt64 -Value $priorAffinity
    if ($null -ne $PriorMaskOut) {
        $PriorMaskOut.Value = $prior
    }
    $requested = [IntPtr][System.BitConverter]::ToInt64(
        [System.BitConverter]::GetBytes($Mask),
        0
    )
    $process.ProcessorAffinity = $requested
    # Read back through a FRESH handle. The setter succeeding is not proof the
    # mask took: Windows silently intersects a requested mask with the system
    # affinity mask and with any job-object CPU rate/affinity limit in force, so
    # a partially-applied mask would otherwise be measured as if it were whole.
    # Everything from here can fail with the process ALREADY changed, so each
    # failure puts the old mask back before it reports. Rolling back is
    # best-effort by nature -- if the write failed the restore may fail too --
    # so its outcome is named in the message rather than assumed.
    $verifyFailure = $null
    $applied = 0
    try {
        $verify = Get-Process -Id $ProcessId -ErrorAction Stop
        $verifyAffinity = $verify.ProcessorAffinity
        if ($null -eq $verifyAffinity) {
            throw "could not be re-read, so the mask is unverified"
        }
        $applied = ConvertTo-Hoi4AffinityUInt64 -Value $verifyAffinity
        if ($applied -ne $Mask) {
            throw "kept affinity $(Format-Hoi4AffinityMask -Mask $applied)"
        }
    } catch {
        $verifyFailure = ($_.Exception.Message -replace '\s+', ' ').Trim()
    }
    if ($null -ne $verifyFailure) {
        $rollback = 'rolled back'
        try {
            (Get-Process -Id $ProcessId -ErrorAction Stop).ProcessorAffinity = $priorAffinity
        } catch {
            $rollback = "ROLLBACK FAILED, restore $(Format-Hoi4AffinityMask -Mask $prior) by hand: $(($_.Exception.Message -replace '\s+', ' ').Trim())"
        }
        throw "$Label PID $ProcessId $verifyFailure after $(Format-Hoi4AffinityMask -Mask $Mask) was requested ($rollback)."
    }
    Write-Host "phase=cpu_affinity_applied target=$Label pid=$ProcessId mask=$(Format-Hoi4AffinityMask -Mask $applied) prior=$(Format-Hoi4AffinityMask -Mask $prior)"
    return [pscustomobject]@{
        Label       = $Label
        ProcessId   = $ProcessId
        PriorMask   = $prior
        AppliedMask = $applied
    }
}

$cpuAffinityPlan = $null
$cpuAffinityApplied = New-Object System.Collections.Generic.List[object]
$gameAffinityPrior = $null
if ($CpuAffinityCores -gt 0) {
    $cpuAffinityPlan = Get-Hoi4CpuAffinityPlan `
        -CoreCount $CpuAffinityCores `
        -OneThreadPerCore ([bool]$CpuAffinityOneThreadPerCore)
    Write-Host (
        'phase=cpu_affinity_plan ' +
        "cores=$($cpuAffinityPlan.RequestedCores) " +
        "one_thread_per_core=$($cpuAffinityPlan.OneThreadPerCore) " +
        "mask=$($cpuAffinityPlan.MaskHex) " +
        "group=$($cpuAffinityPlan.Group) " +
        "efficiency_class=$($cpuAffinityPlan.EfficiencyClass) " +
        "cores_selected=$($cpuAffinityPlan.SelectedCoreIndexes -join ',') " +
        "logical_processors=$($cpuAffinityPlan.LogicalProcessors -join ',') " +
        "machine_cores=$($cpuAffinityPlan.MachinePhysicalCores) " +
        "machine_logical=$($cpuAffinityPlan.MachineLogicalProcessors)"
    )
    Write-Warning 'CPU affinity emulates the core COUNT of the floor machine and nothing else. Per-core speed, cache size and memory bandwidth are this machine''s, and physical RAM is not restricted at all, so the result is a lower bound on a real 4c/8 GB box.'
}

$harnessPolicyPath = Join-Path $OutDir 'passive_capture_policy.json'
$harnessPolicy = [ordered]@{
    schema                              = 'hoi4-passive-capture-policy-v1'
    mode                                = if ($EvidenceOnlyDriveGame) {
        'evidence_only_pre_window_setup'
    } else {
        'passive_reuse'
    }
    requested_speed                     = $Speed
    reuses_user_owned_game              = $reuseRunningGame
    client_driving_opt_in_required      = $EvidenceOnlyDriveGame
    pre_window_setup_may_drive_client   = $EvidenceOnlyDriveGame
    attach_through_teardown_policy      = 'zero_client_writes_launch_stop_or_ui'
    graded_observer_access              = 'PROCESS_QUERY_INFORMATION|PROCESS_VM_READ'
    graded_observer_sha256              = $readOnlyObserverSha256
    evidence_only_driver_sha256         = $headlessControlSha256
    ready_contract                      = 'hoi4-capture-ready-v2'
    teardown_contract                   = 'hoi4-capture-teardown-v2'
    # The PLAN, recorded before anything is launched. What was actually applied
    # to which PID is HARNESS_CPU_AFFINITY_V1.json, written once both processes
    # exist -- a plan that never reached a process must not read as a mask that
    # did.
    cpu_affinity_requested              = if ($null -eq $cpuAffinityPlan) {
        [ordered]@{ enabled = $false }
    } else {
        [ordered]@{
            enabled                    = $true
            physical_cores             = $cpuAffinityPlan.RequestedCores
            one_thread_per_core        = $cpuAffinityPlan.OneThreadPerCore
            mask                       = $cpuAffinityPlan.MaskHex
            processor_group            = $cpuAffinityPlan.Group
            efficiency_class           = $cpuAffinityPlan.EfficiencyClass
            selected_core_indexes      = $cpuAffinityPlan.SelectedCoreIndexes
            selected_logical_processors = $cpuAffinityPlan.LogicalProcessors
            machine_physical_cores     = $cpuAffinityPlan.MachinePhysicalCores
            machine_logical_processors = $cpuAffinityPlan.MachineLogicalProcessors
            emulates                   = 'core_count_only'
        }
    }
}
$harnessPolicy | ConvertTo-Json -Depth 8 |
    Set-Content -LiteralPath $harnessPolicyPath -Encoding UTF8

# Extend the recorder deadline for as long as finalization is demonstrably
# advancing, and refuse to extend it when it is not.
#
# The recorder publishes RECORDER_FINALIZE_PROGRESS_V1.json throughout the
# post-teardown fold. Its `sample_ordinal` is LIVENESS ONLY and is deliberately
# not consulted here -- a recorder spinning on an unsatisfiable condition would
# tick it forever. The only field that grants time is `work_unchanged_ms`: how
# long the recorder's own finalize outputs have stood still.
#
# Identity is checked against this attempt's session/capture ids. The harness
# already learned this lesson on the teardown receipt: a file left in a reused
# OutDir by an earlier attempt proves nothing about this one, and here it would
# be worse than nothing, because it would hand out real time.
#
# Absence is not failure. A recorder that predates the receipt, or one whose
# publisher could not start, simply keeps the fixed budget it has today.
# The one spelling of a stall, shared by the grant function and the terminal
# reason so the two cannot name the same receipt differently. A receipt whose
# work never moved since its first sample is a finalize that NEVER STARTED, which
# is a different diagnosis from one that advanced and then stopped; the stage the
# recorder publishes says where.
function Get-RecorderFinalizeStallReason {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][hashtable]$Fields,
        [Parameter(Mandatory = $true)][int64]$UnchangedMs
    )
    $seconds = [Math]::Floor($UnchangedMs / 1000)
    $neverAdvanced = $false
    if ($null -ne $Fields['work_changed_ordinal']) {
        try { $neverAdvanced = ([int64]$Fields['work_changed_ordinal'] -eq 0) } catch { }
    }
    $reason = if ($neverAdvanced) {
        "finalize_never_advanced_for_${seconds}s"
    } else {
        "no_progress_for_${seconds}s"
    }
    $stage = [string]$Fields['stage']
    if (-not [string]::IsNullOrWhiteSpace($stage)) {
        $reason += "_in_$($stage -replace '[^A-Za-z0-9_]', '_')"
    }
    return $reason
}

function Update-RecorderFinalizeDeadline {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RunDir,
        [Parameter(Mandatory = $true)][string]$SessionId,
        [Parameter(Mandatory = $true)][string]$CaptureId,
        [Parameter(Mandatory = $true)][ref]$Deadline,
        [Parameter(Mandatory = $true)][AllowNull()][object]$Ceiling,
        [Parameter(Mandatory = $true)][int]$NoProgressSeconds,
        [Parameter(Mandatory = $true)][ref]$Reason,
        # A receipt older than this is a dead publisher, not a slow one.
        [int]$MaxReceiptAgeSeconds = 30,
        # One-shot window granted by a terminal `complete` sample.
        [int]$ExitGraceSeconds = 30
    )
    if ($null -eq $Ceiling) { return $false }
    $path = Join-Path $RunDir 'RECORDER_FINALIZE_PROGRESS_V1.json'
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return $false }
    # Flattened into a hashtable before anything is read out of it. Under
    # `Set-StrictMode -Version Latest` a missing property on the PSCustomObject
    # ConvertFrom-Json returns is a TERMINATING error, and this function is
    # called from the cleanup `finally` -- where a terminating error abandons
    # the compact-status arbitration and leaves the run at exit 1. A missing
    # hashtable key is just $null, which every check below already treats as
    # "grants nothing".
    $fields = @{}
    $writtenUtc = $null
    try {
        # The publisher republishes by rename, so content and metadata read
        # separately can come from DIFFERENT generations. Straddling a rename
        # can pair one sample's staleness with another sample's timestamp,
        # which grants either far too much or -- worse -- almost nothing, on a
        # receipt that was perfectly healthy.
        #
        # Two defences, because neither alone is conclusive:
        #
        #  * The content is read TWICE and must come back identical. Every
        #    generation changes `sample_ordinal`, `elapsed_ms` and
        #    `sample_unix_ms`, so two different generations cannot produce equal
        #    text. This is the real generation identity; last-write-time plus
        #    length is not, since two same-length publications can land inside
        #    one clock tick and be indistinguishable.
        #  * The metadata values are materialised the instant each `Get-Item`
        #    returns. `FileInfo` populates its properties on FIRST ACCESS and
        #    caches thereafter, so reading `$before.LastWriteTimeUtc` after the
        #    content read would sample the file at that later moment and compare
        #    it against itself -- a check that can never fail.
        $beforeItem = Get-Item -LiteralPath $path -ErrorAction Stop
        $beforeWriteUtc = $beforeItem.LastWriteTimeUtc
        $beforeLength = $beforeItem.Length
        $rawFirst = Get-Content -LiteralPath $path -Raw -ErrorAction Stop
        $rawSecond = Get-Content -LiteralPath $path -Raw -ErrorAction Stop
        $afterItem = Get-Item -LiteralPath $path -ErrorAction Stop
        $afterWriteUtc = $afterItem.LastWriteTimeUtc
        $afterLength = $afterItem.Length
        if ($rawFirst -ne $rawSecond -or
            $beforeWriteUtc -ne $afterWriteUtc -or
            $beforeLength -ne $afterLength) {
            # Republished mid-read. Not evidence, and not an error either --
            # the next tick reads a settled generation.
            return $false
        }
        $receipt = $rawFirst | ConvertFrom-Json
        if ($null -eq $receipt) { return $false }
        $writtenUtc = $afterWriteUtc
        foreach ($property in $receipt.PSObject.Properties) {
            $fields[$property.Name] = $property.Value
        }
    } catch {
        # A torn or unparseable read is transient, not evidence. Grant nothing
        # and let the caller try again on its next tick.
        return $false
    }
    try {
        if ($fields['schema'] -ne 'battle-plan-recorder-finalize-progress-v1') { return $false }
        if ($fields['session_id'] -ne $SessionId -or $fields['capture_id'] -ne $CaptureId) {
            $Reason.Value = 'foreign_progress_receipt'
            return $false
        }
        # Only the phases this protocol defines. An unrecognised phase with
        # matching ids and a low staleness would otherwise buy time.
        $phase = [string]$fields['phase']
        if ($phase -ne 'finalizing' -and $phase -ne 'complete') { return $false }
        # A receipt carrying no staleness at all states nothing about progress,
        # so it buys nothing -- absent is not zero.
        if ($null -eq $fields['work_unchanged_ms']) { return $false }
        $unchangedMs = [int64]$fields['work_unchanged_ms']
        if ($unchangedMs -lt 0) { return $false }

        # FRESHNESS -- a question about the PUBLISHER, answered by the file.
        # The publisher republishes every couple of seconds, so a receipt that
        # has stopped being rewritten means the publisher is gone: its thread
        # failed to start, or its writes are failing. Its content is then a
        # photograph, and a photograph showing "work was advancing" would keep
        # buying time forever. This is measured on the file precisely because a
        # dead publisher cannot update a field inside it.
        $ageSeconds = ([DateTime]::UtcNow - $writtenUtc).TotalSeconds
        if ($ageSeconds -gt $MaxReceiptAgeSeconds -or $ageSeconds -lt -60) {
            $Reason.Value = "progress_receipt_stale_$([Math]::Floor([Math]::Abs($ageSeconds)))s"
            return $false
        }

        # ANCHOR -- a question about the WORK, answered by the sample.
        # `sample_unix_ms` is stamped next to the monotonic reading that
        # produced `work_unchanged_ms`. The file's modified time is NOT that
        # instant: it trails it by the metering walk plus serialize, write,
        # sync and rename. Using the file here would push the reconstructed
        # last-progress instant forward by that latency and grant time the
        # recorder never earned -- and the freshness gate could not catch it,
        # because such a receipt is genuinely newly written.
        if ($null -eq $fields['sample_unix_ms']) { return $false }
        $sampleUnixMs = [int64]$fields['sample_unix_ms']
        if ($sampleUnixMs -le 0) { return $false }
        $sampleUtc = [DateTimeOffset]::FromUnixTimeMilliseconds($sampleUnixMs).UtcDateTime
        # Cross-check the sample against the file that carries it. The sample is
        # taken BEFORE the write, so it can legitimately be older than the file
        # -- that is just publication latency, and being older only ever costs
        # the recorder time. It must never be materially NEWER: that means the
        # wall clock stepped backwards between the sample and the write, and an
        # apparently-future sample would grant time up to the ceiling while the
        # freshness gate saw a perfectly fresh file.
        if ($sampleUtc -gt $writtenUtc.AddSeconds(60)) {
            $Reason.Value = 'progress_sample_ahead_of_its_receipt'
            return $false
        }

        # The candidate is ABSOLUTE, derived from when progress last happened,
        # never `now + window`. A rolling `now + window` grants a fresh full
        # window on every poll, so a stall that has already consumed most of its
        # allowance still buys another whole one -- a 180 s window would admit
        # nearly 360 s -- and a terminal receipt would renew as a lease on every
        # read until the ceiling. Anchoring on the sample makes a frozen receipt
        # yield a FIXED deadline that the loop simply walks past.
        if ($phase -eq 'complete') {
            # The fold is done and the process is on its way out. One bounded
            # exit grace, anchored on the sample so it cannot renew.
            $candidate = $sampleUtc.AddSeconds($ExitGraceSeconds)
        } else {
            if ($unchangedMs -ge ([int64]$NoProgressSeconds * 1000)) {
                # A PROVEN STALL CUTS THE DEADLINE TO NOW. Declining to extend
                # was not enough: the launch/READY budget was still standing,
                # so a finalize that never advanced was waited out to that
                # clock -- 531 s on rc_vanilla8's refused attempt, on a receipt
                # that had said "no progress" since its first sample. Every
                # finalize stage now reports work units (census files, folded
                # hours, proved hours) beside the bytes it writes, so a receipt
                # that stops moving is a recorder that stopped, and the cut
                # names the stage it stopped in. Termination stays gated on the
                # hooks being provably released; the cut only ends the wait.
                $Reason.Value = Get-RecorderFinalizeStallReason `
                    -Fields $fields -UnchangedMs $unchangedMs
                $now = [DateTime]::UtcNow
                if ($now -lt $Deadline.Value) {
                    $Deadline.Value = $now
                    return $true
                }
                return $false
            }
            $lastProgressUtc = $sampleUtc.AddMilliseconds(-$unchangedMs)
            $candidate = $lastProgressUtc.AddSeconds($NoProgressSeconds)
        }
        if ($candidate -gt $Ceiling) { $candidate = $Ceiling }
        if ($candidate -le $Deadline.Value) { return $false }
        $Deadline.Value = $candidate
        $Reason.Value = if ($phase -eq 'complete') {
            'finalize_complete_exit_grace'
        } else {
            "work_advancing_unchanged_${unchangedMs}ms"
        }
        return $true
    } catch {
        # Any unexpected shape or conversion is no evidence, never a throw:
        # this runs inside the cleanup `finally`, where a terminating error
        # abandons the compact-status arbitration.
        return $false
    }
}

# WHY THE RUN STOPPED WAITING, read at the moment it stopped.
#
# `Update-RecorderFinalizeDeadline` publishes its reason as a side effect of
# GRANTING time, and the grant and the timeout are one tick apart: the last call
# before the loop falls out sees `work_unchanged_ms` just under the threshold,
# writes `work_advancing_unchanged_<n>ms`, and extends the deadline to an instant
# that then passes. `no_progress_for_<n>s` is only ever written on a call that is
# ALREADY past the threshold -- which the loop does not make, because it has
# walked past its deadline and exited. So the last `finalize_bound=` a timed-out
# run printed was structurally the opposite of the truth: it named the last
# extension on a run that then died of not being extended.
#
# This states the cause from the receipt AT THE EXIT, not from the last grant.
# It is deliberately separate from the grant function: a reason is an
# observation, and the function that hands out time must not be asked to also be
# the one that explains why it stopped. Non-throwing for the same reason
# everything else here is -- it runs inside the cleanup `finally`.
function Get-RecorderFinalizeTerminalReason {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$RunDir,
        [Parameter(Mandatory = $true)][string]$SessionId,
        [Parameter(Mandatory = $true)][string]$CaptureId,
        [Parameter(Mandatory = $true)][AllowNull()][object]$Deadline,
        [Parameter(Mandatory = $true)][AllowNull()][object]$Ceiling,
        [Parameter(Mandatory = $true)][int]$NoProgressSeconds,
        [int]$MaxReceiptAgeSeconds = 30
    )
    try {
        $path = Join-Path $RunDir 'RECORDER_FINALIZE_PROGRESS_V1.json'
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            return 'no_progress_receipt'
        }
        $item = Get-Item -LiteralPath $path -ErrorAction Stop
        $writtenUtc = $item.LastWriteTimeUtc
        $receipt = (Get-Content -LiteralPath $path -Raw -ErrorAction Stop) | ConvertFrom-Json
        if ($null -eq $receipt) { return 'progress_receipt_unreadable' }
        $fields = @{}
        foreach ($property in $receipt.PSObject.Properties) {
            $fields[$property.Name] = $property.Value
        }
        if ($fields['schema'] -ne 'battle-plan-recorder-finalize-progress-v1') {
            return 'progress_receipt_foreign_schema'
        }
        if ($fields['session_id'] -ne $SessionId -or $fields['capture_id'] -ne $CaptureId) {
            return 'foreign_progress_receipt'
        }
        $ageSeconds = ([DateTime]::UtcNow - $writtenUtc).TotalSeconds
        if ($ageSeconds -gt $MaxReceiptAgeSeconds -or $ageSeconds -lt -60) {
            return "progress_receipt_stale_$([Math]::Floor([Math]::Abs($ageSeconds)))s"
        }
        if ($null -eq $fields['work_unchanged_ms']) { return 'progress_receipt_without_staleness' }
        $unchangedMs = [int64]$fields['work_unchanged_ms']
        # THE FIELD IS A READING TAKEN AT `sample_unix_ms`, NOT AT NOW.
        #
        # Comparing the sampled number against the threshold reproduces the very
        # off-by-one-tick this function exists to fix, one layer down: a receipt
        # sampled at 178 s of stall and read two seconds later is a 180 s stall,
        # and reporting `deadline_passed_unchanged_178000ms` for it is the same
        # kind of near-miss as reporting the last extension. The stall is
        # reconstructed from when progress last happened, exactly as the grant
        # function anchors its deadline. (Codex review round 3, finding D3.)
        $effectiveUnchangedMs = $unchangedMs
        if ($null -ne $fields['sample_unix_ms']) {
            $sampleUnixMs = [int64]$fields['sample_unix_ms']
            if ($sampleUnixMs -gt 0) {
                $sampleUtc = [DateTimeOffset]::FromUnixTimeMilliseconds($sampleUnixMs).UtcDateTime
                $lastProgressUtc = $sampleUtc.AddMilliseconds(-$unchangedMs)
                $sinceProgressMs = [int64](([DateTime]::UtcNow - $lastProgressUtc).TotalMilliseconds)
                # Only ever forward. A clock that stepped backwards must not
                # shorten a stall that has already been observed.
                if ($sinceProgressMs -gt $effectiveUnchangedMs) {
                    $effectiveUnchangedMs = $sinceProgressMs
                }
            }
        }
        if ($effectiveUnchangedMs -ge ([int64]$NoProgressSeconds * 1000)) {
            return (Get-RecorderFinalizeStallReason -Fields $fields -UnchangedMs $effectiveUnchangedMs)
        }
        $unchangedMs = $effectiveUnchangedMs
        # Still advancing by its own account, and yet out of time: the only way
        # that happens is the absolute ceiling, which no amount of progress
        # extends past.
        if ($null -ne $Ceiling -and $null -ne $Deadline -and
            [DateTime]$Deadline -ge ([DateTime]$Ceiling).AddSeconds(-1)) {
            return "finalize_ceiling_reached_unchanged_${unchangedMs}ms"
        }
        return "deadline_passed_unchanged_${unchangedMs}ms"
    } catch {
        return 'progress_receipt_unreadable'
    }
}

# Is the process the collector attached to GONE?
#
# Identity is the pid AND its creation time, never the pid alone: Windows reuses
# pids, and a recycled pid answering `Get-Process` is a DIFFERENT process that
# this capture never touched. Both answers below mean the same thing for our
# purposes -- the process that held our hooks does not exist any more -- so both
# return $true, and the caller is told which.
#
# Anything it cannot determine returns $false. This predicate only ever unlocks a
# more forceful action, so "I could not tell" must read as "do nothing".
function Test-TargetProcessGone {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][int]$TargetProcessId,
        [Parameter(Mandatory = $true)][uint64]$ExpectedCreationFiletimeUtc,
        [Parameter(Mandatory = $true)][ref]$Detail
    )
    try {
        $process = Get-Process -Id $TargetProcessId -ErrorAction SilentlyContinue
        if ($null -eq $process) {
            $Detail.Value = 'pid_absent'
            return $true
        }
        $creation = [uint64]($process.StartTime.ToUniversalTime().ToFileTimeUtc())
        if ($creation -ne $ExpectedCreationFiletimeUtc) {
            $Detail.Value = 'pid_recycled'
            return $true
        }
        $Detail.Value = 'target_alive'
        return $false
    } catch {
        $Detail.Value = 'target_liveness_undetermined'
        return $false
    }
}

try {
    if ($DryRun) {
        if ($reuseRunningGame) {
            Write-Host "phase=settings_guard action=skip reason=use_running_game dry_run=True path=$settingsPath sentinel=$settingsGuardSentinel"
            Write-Warning 'The settings guard cannot protect an already-running game because the boot-time settings.txt read has already happened; modset resolution may be unresolved for this attach.'
        } else {
            Write-Host "phase=settings_guard action=skip reason=dry_run would_apply=True path=$settingsPath sentinel=$settingsGuardSentinel"
        }
    } else {
        $canonicalSentinelExists = Test-Path `
            -LiteralPath $settingsGuardCanonicalSentinel `
            -PathType Leaf
        $legacySentinelIsDistinct = -not [System.StringComparer]::OrdinalIgnoreCase.Equals(
            $settingsGuardLegacySentinel,
            $settingsGuardCanonicalSentinel
        )
        $legacySentinelExists = (
            $legacySentinelIsDistinct -and
            (Test-Path -LiteralPath $settingsGuardLegacySentinel -PathType Leaf)
        )
        if ($canonicalSentinelExists -and $legacySentinelExists) {
            throw "Settings guard found both canonical and legacy sentinels for '$settingsPath': '$settingsGuardCanonicalSentinel' and '$settingsGuardLegacySentinel'. Refusing to choose between concurrent recovery records; verify all holders and restore the attribute manually."
        }
        if ($legacySentinelExists) {
            $settingsGuardSentinel = $settingsGuardLegacySentinel
            Write-Warning "Settings guard found a legacy DOS-path sentinel at '$settingsGuardSentinel'; it will be recovered before this run publishes the canonical guard at '$settingsGuardCanonicalSentinel'."
        }
        if (Test-Path -LiteralPath $settingsGuardSentinel -PathType Leaf) {
            $settingsGuardReadStream = $null
            $settingsGuardReader = $null
            $settingsGuardSentinelVanishedDuringRead = $false
            try {
                $settingsGuardReadStream = [System.IO.File]::Open(
                    $settingsGuardSentinel,
                    [System.IO.FileMode]::Open,
                    [System.IO.FileAccess]::Read,
                    [System.IO.FileShare]::ReadWrite
                )
                $settingsGuardReader = [System.IO.StreamReader]::new(
                    $settingsGuardReadStream,
                    [System.Text.UTF8Encoding]::new($false),
                    $true,
                    1024,
                    $true
                )
                $settingsGuardRecordJson = $settingsGuardReader.ReadToEnd()
                $settingsGuardRecord = $settingsGuardRecordJson | ConvertFrom-Json
            } catch {
                $sentinelReadException = Get-SettingsGuardBaseException -Exception $_.Exception
                $sentinelReadExceptionKind = Get-SettingsGuardExceptionKind -Exception $_.Exception
                if ($sentinelReadExceptionKind -eq 'not_found') {
                    $settingsGuardSentinelVanishedDuringRead = $true
                } elseif ($sentinelReadExceptionKind -eq 'sharing_violation') {
                    throw "Settings guard sentinel '$settingsGuardSentinel' has an incompatible live holder before its recovery record can be read; this may be the capture writer or an exclusive reader. Refusing to touch '$settingsPath' or the sentinel. Wait for all holders to close, then retry. $($sentinelReadException.Message)"
                } else {
                    throw "Settings guard sentinel is unreadable: $settingsGuardSentinel. Inspect it and manually restore the read-only state of '$settingsPath' before removing it. $($_.Exception.Message)"
                }
            } finally {
                if ($null -ne $settingsGuardReader) {
                    $settingsGuardReader.Dispose()
                }
                if ($null -ne $settingsGuardReadStream) {
                    $settingsGuardReadStream.Dispose()
                }
            }
            if ($settingsGuardSentinelVanishedDuringRead) {
                Write-Warning "Settings guard sentinel '$settingsGuardSentinel' vanished before its recovery record could be read; another run already recovered it. Continuing without touching '$settingsPath'."
                Write-Host "phase=settings_guard action=skip reason=sentinel_vanished path=$settingsPath sentinel=$settingsGuardSentinel"
            } else {
                if ($null -eq $settingsGuardRecord) {
                    throw "Settings guard sentinel is invalid: $settingsGuardSentinel. It must be a schema-version 2 JSON object. Inspect it and manually restore '$settingsPath' before removing it."
                }
                $schemaProperty = $settingsGuardRecord.PSObject.Properties['schema_version']
                $recordedPathProperty = $settingsGuardRecord.PSObject.Properties['settings_path']
                $recordedPriorProperty = $settingsGuardRecord.PSObject.Properties['prior_read_only']
                $recordedOwnerPidProperty = $settingsGuardRecord.PSObject.Properties['owner_pid']
                $recordedOwnerStartProperty = $settingsGuardRecord.PSObject.Properties['owner_start_utc']
                if ($null -eq $schemaProperty -or $schemaProperty.Value -isnot [int] -or
                    [int]$schemaProperty.Value -ne 2 -or
                    $null -eq $recordedPathProperty -or
                    $null -eq $recordedPriorProperty -or
                    $recordedPriorProperty.Value -isnot [bool] -or
                    $null -eq $recordedOwnerPidProperty -or
                    $recordedOwnerPidProperty.Value -isnot [int] -or
                    [int]$recordedOwnerPidProperty.Value -le 0 -or
                    $null -eq $recordedOwnerStartProperty -or
                    $recordedOwnerStartProperty.Value -isnot [string] -or
                    [string]::IsNullOrWhiteSpace([string]$recordedOwnerStartProperty.Value)) {
                    throw "Settings guard sentinel is invalid or from an unsupported schema: $settingsGuardSentinel. It must contain schema_version=2, settings_path, a Boolean prior_read_only, a positive integer owner_pid, and owner_start_utc. Inspect it and manually restore '$settingsPath' before removing it."
                }
                $recordedSettingsPathRaw = [string]$recordedPathProperty.Value
                if ([string]::IsNullOrWhiteSpace($recordedSettingsPathRaw)) {
                    throw "Settings guard sentinel contains an empty settings_path: $settingsGuardSentinel. Inspect it and manually restore '$settingsPath' before removing it."
                }
                if ([System.StringComparer]::OrdinalIgnoreCase.Equals(
                    $recordedSettingsPathRaw,
                    $settingsGuardCanonicalPath
                ) -or [System.StringComparer]::OrdinalIgnoreCase.Equals(
                    $recordedSettingsPathRaw,
                    $settingsPath
                )) {
                    $recordedSettingsPath = $recordedSettingsPathRaw
                } else {
                    try {
                        $recordedSettingsPath = [System.IO.Path]::GetFullPath(
                            $recordedSettingsPathRaw
                        )
                    } catch {
                        throw "Settings guard sentinel contains an invalid settings_path: $settingsGuardSentinel. Inspect it and manually restore '$settingsPath' before removing it. $($_.Exception.Message)"
                    }
                }
                if ([System.StringComparer]::OrdinalIgnoreCase.Equals(
                    $recordedSettingsPath,
                    $settingsGuardCanonicalPath
                ) -or [System.StringComparer]::OrdinalIgnoreCase.Equals(
                    $recordedSettingsPath,
                    $settingsPath
                )) {
                    # The second arm accepts schema-2 sentinels written before
                    # canonical final paths were introduced.
                    $recordedSettingsCanonicalPath = $settingsGuardCanonicalPath
                } elseif (Test-Path -LiteralPath $recordedSettingsPath -PathType Leaf) {
                    try {
                        $recordedSettingsCanonicalPath = Get-SettingsGuardCanonicalPath -Path $recordedSettingsPath
                    } catch {
                        throw "Settings guard sentinel settings_path could not be canonicalized: '$recordedSettingsPath' ($settingsGuardSentinel). Refusing to touch '$settingsPath'. $($_.Exception.Message)"
                    }
                } else {
                    $recordedSettingsCanonicalPath = $recordedSettingsPath
                }
                if (-not [System.StringComparer]::OrdinalIgnoreCase.Equals(
                    $recordedSettingsCanonicalPath,
                    $settingsGuardCanonicalPath
                )) {
                    throw "Settings guard sentinel path mismatch: '$recordedSettingsPath' does not identify '$settingsPath' (canonical '$settingsGuardCanonicalPath'; $settingsGuardSentinel). Refusing to overwrite recovery evidence."
                }
                try {
                    $recordedOwnerStart = [DateTimeOffset]::ParseExact(
                        [string]$recordedOwnerStartProperty.Value,
                        'o',
                        [Globalization.CultureInfo]::InvariantCulture,
                        [Globalization.DateTimeStyles]::RoundtripKind
                    ).ToUniversalTime()
                } catch {
                    throw "Settings guard sentinel contains an invalid owner_start_utc: $settingsGuardSentinel. Inspect it and manually restore '$settingsPath' before removing it. $($_.Exception.Message)"
                }

                $recordedPriorReadOnly = [bool]$recordedPriorProperty.Value
                $recordedOwnerPid = [int]$recordedOwnerPidProperty.Value
                $settingsGuardRecoveryClaim = $null
                $settingsGuardRecoveryClaimStatus = ''
                $settingsGuardSentinelVanished = $false
                $settingsGuardClaimAttempts = 3
                for ($claimAttempt = 1; $claimAttempt -le $settingsGuardClaimAttempts; $claimAttempt++) {
                    try {
                        $settingsGuardRecoveryClaim = [System.IO.File]::Open(
                            $settingsGuardSentinel,
                            [System.IO.FileMode]::Open,
                            [System.IO.FileAccess]::ReadWrite,
                            [System.IO.FileShare]::None
                        )
                        $settingsGuardRecoveryClaimStatus = 'exclusive'
                        break
                    } catch {
                        $claimException = Get-SettingsGuardBaseException -Exception $_.Exception
                        $claimExceptionKind = Get-SettingsGuardExceptionKind -Exception $_.Exception
                        if ($claimExceptionKind -eq 'not_found') {
                            $settingsGuardSentinelVanished = $true
                            break
                        }
                        if ($claimExceptionKind -eq 'access_denied') {
                            throw "Settings guard cannot take a recovery claim on sentinel '$settingsGuardSentinel' because access was denied. Refusing to touch '$settingsPath' or the sentinel; this is not evidence that a live capture run holds the guard. $($claimException.Message)"
                        }
                        if ($claimExceptionKind -ne 'sharing_violation') {
                            $claimHResult = '0x{0:X8}' -f ([int]$claimException.HResult)
                            throw "Settings guard recovery claim failed unexpectedly for sentinel '$settingsGuardSentinel' ($($claimException.GetType().FullName), HResult=$claimHResult). Refusing to touch '$settingsPath' or the sentinel; this is not evidence that a live capture run holds the guard. $($claimException.Message)"
                        }

                        # An exclusive claim also conflicts with an ordinary reader. A
                        # read-only probe whose share excludes writers distinguishes that
                        # benign case from the guard owner's live write handle.
                        $settingsGuardReaderOnlyProbe = $null
                        try {
                            $settingsGuardReaderOnlyProbe = [System.IO.File]::Open(
                                $settingsGuardSentinel,
                                [System.IO.FileMode]::Open,
                                [System.IO.FileAccess]::Read,
                                [System.IO.FileShare]::Read
                            )
                            $settingsGuardRecoveryClaimStatus = 'readers_only'
                            break
                        } catch {
                            $readerProbeException = Get-SettingsGuardBaseException -Exception $_.Exception
                            $readerProbeExceptionKind = Get-SettingsGuardExceptionKind -Exception $_.Exception
                            if ($readerProbeExceptionKind -eq 'not_found') {
                                $settingsGuardSentinelVanished = $true
                                break
                            }
                            if ($readerProbeExceptionKind -eq 'access_denied') {
                                throw "Settings guard cannot inspect the holders of sentinel '$settingsGuardSentinel' because read access was denied. Refusing to touch '$settingsPath' or the sentinel; this is not evidence that a live capture run holds the guard. $($readerProbeException.Message)"
                            }
                            if ($readerProbeExceptionKind -ne 'sharing_violation') {
                                $readerProbeHResult = '0x{0:X8}' -f ([int]$readerProbeException.HResult)
                                throw "Settings guard holder inspection failed unexpectedly for sentinel '$settingsGuardSentinel' ($($readerProbeException.GetType().FullName), HResult=$readerProbeHResult). Refusing to touch '$settingsPath' or the sentinel; this is not evidence that a live capture run holds the guard. $($readerProbeException.Message)"
                            }
                        } finally {
                            if ($null -ne $settingsGuardReaderOnlyProbe) {
                                $settingsGuardReaderOnlyProbe.Dispose()
                            }
                        }

                        if ($claimAttempt -lt $settingsGuardClaimAttempts) {
                            Start-Sleep -Milliseconds 200
                        }
                    }
                }

                if ($settingsGuardSentinelVanished) {
                    Write-Warning "Settings guard sentinel '$settingsGuardSentinel' vanished before its recovery claim could be taken; another run already recovered it. Continuing without touching '$settingsPath'."
                    Write-Host "phase=settings_guard action=skip reason=sentinel_vanished path=$settingsPath sentinel=$settingsGuardSentinel"
                } elseif ([string]::IsNullOrWhiteSpace($settingsGuardRecoveryClaimStatus)) {
                    throw "Settings guard sentinel '$settingsGuardSentinel' still has an incompatible live holder after $settingsGuardClaimAttempts recovery-claim attempts; this may be the capture writer or an exclusive reader (recorded writer PID $recordedOwnerPid at $($recordedOwnerStart.ToString('o'))). Refusing to touch '$settingsPath' or the sentinel. Wait for all holders to close, or remove the sentinel deliberately only after verifying no run holds the guard."
                } else {
                    try {
                        Write-Warning "Interrupted settings guard detected at '$settingsGuardSentinel' (no live holder; written by PID $recordedOwnerPid at $($recordedOwnerStart.ToString('o'))); restoring IsReadOnly=$recordedPriorReadOnly on '$settingsPath' before this run."
                        $recoveryItem = Get-Item -LiteralPath $settingsPath -Force
                        $recoveryItem.IsReadOnly = $recordedPriorReadOnly
                        $recoveryItem.Refresh()
                        if ([bool]$recoveryItem.IsReadOnly -ne $recordedPriorReadOnly) {
                            throw "Interrupted settings guard recovery did not restore IsReadOnly=$recordedPriorReadOnly on '$settingsPath'. Sentinel retained at '$settingsGuardSentinel'."
                        }
                    } finally {
                        if ($null -ne $settingsGuardRecoveryClaim) {
                            try {
                                $settingsGuardRecoveryClaim.Dispose()
                            } finally {
                                $settingsGuardRecoveryClaim = $null
                            }
                        }
                    }
                    $settingsGuardRecoverySentinelRemoved = $false
                    try {
                        Remove-Item -LiteralPath $settingsGuardSentinel -Force
                        $settingsGuardRecoverySentinelRemoved = $true
                    } catch {
                        if (-not (Test-Path -LiteralPath $settingsGuardSentinel -PathType Leaf)) {
                            $settingsGuardRecoverySentinelRemoved = $true
                        } else {
                            Write-Warning "Settings guard restored IsReadOnly=$recordedPriorReadOnly on '$settingsPath', but could not remove recovered sentinel '$settingsGuardSentinel'. The recovery is complete and the sentinel was retained for the next run. Error: $($_.Exception.Message)"
                        }
                    }
                    Write-Host "phase=settings_guard action=recover owner_status=no_live_holder claim_status=$settingsGuardRecoveryClaimStatus writer_pid=$recordedOwnerPid writer_start_utc=$($recordedOwnerStart.ToString('o')) restored_read_only=$recordedPriorReadOnly sentinel_removed=$settingsGuardRecoverySentinelRemoved path=$settingsPath sentinel=$settingsGuardSentinel"
                    if (-not $settingsGuardRecoverySentinelRemoved) {
                        throw "Settings guard recovery restored IsReadOnly=$recordedPriorReadOnly on '$settingsPath', but retained sentinel '$settingsGuardSentinel' prevents a new guard from being applied. Retry after the process reading the sentinel releases it."
                    }
                }
            }
        }

        $settingsGuardSentinel = $settingsGuardCanonicalSentinel
        if ($reuseRunningGame) {
            Write-Host "phase=settings_guard action=skip reason=use_running_game path=$settingsPath sentinel=$settingsGuardSentinel"
            Write-Warning 'The settings guard cannot protect an already-running game because the boot-time settings.txt read has already happened; modset resolution may be unresolved for this attach.'
        } else {
            $settingsItem = Get-Item -LiteralPath $settingsPath -Force
            $settingsGuardPriorReadOnly = [bool]$settingsItem.IsReadOnly
            $settingsGuardOwnerProcess = Get-Process -Id $PID -ErrorAction Stop
            try {
                $settingsGuardOwnerStartUtc = ([DateTimeOffset]$settingsGuardOwnerProcess.StartTime).ToUniversalTime().ToString('o')
            } catch {
                throw "Settings guard cannot read its own process start time for PID $PID, so it cannot create kill-safe ownership evidence for '$settingsPath'. $($_.Exception.Message)"
            }
            New-Item -ItemType Directory -Path $settingsGuardRoot -Force | Out-Null
            $settingsGuardRecord = [ordered]@{
                schema_version  = 2
                settings_path   = $settingsGuardCanonicalPath
                prior_read_only = $settingsGuardPriorReadOnly
                created_utc     = [DateTime]::UtcNow.ToString('o')
                owner_pid       = $PID
                owner_start_utc = $settingsGuardOwnerStartUtc
            }
            $settingsGuardJson = $settingsGuardRecord | ConvertTo-Json -Compress
            $settingsGuardEncoding = [System.Text.UTF8Encoding]::new($false)
            $settingsGuardBytes = $settingsGuardEncoding.GetBytes($settingsGuardJson)
            $settingsGuardOwnerStream = [System.IO.File]::Open(
                $settingsGuardSentinel,
                [System.IO.FileMode]::CreateNew,
                [System.IO.FileAccess]::Write,
                [System.IO.FileShare]::ReadWrite
            )
            try {
                $settingsGuardOwnerStream.Write(
                    $settingsGuardBytes,
                    0,
                    $settingsGuardBytes.Length
                )
                $settingsGuardOwnerStream.Flush($true)
            } catch {
                $settingsGuardWriteFailure = $_
                try {
                    $settingsGuardOwnerStream.Dispose()
                } catch {
                    # Preserve the original write/flush failure below.
                } finally {
                    $settingsGuardOwnerStream = $null
                }
                # A hard kill between CreateNew and Flush can still strand an
                # incomplete sentinel because no finally can run. In-process I/O
                # failures are cleaned here before settings.txt is made read-only.
                Remove-Item -LiteralPath $settingsGuardSentinel -Force -ErrorAction SilentlyContinue
                throw $settingsGuardWriteFailure
            }
            $settingsGuardRestoreRequired = $true

            $settingsItem.IsReadOnly = $true
            $settingsItem.Refresh()
            if (-not [bool]$settingsItem.IsReadOnly) {
                throw "Settings guard could not set IsReadOnly=True on '$settingsPath'. Sentinel retained at '$settingsGuardSentinel'."
            }
            Write-Host "phase=settings_guard action=apply prior_read_only=$settingsGuardPriorReadOnly path=$settingsPath sentinel=$settingsGuardSentinel"
        }
    }

function Get-Hoi4Processes {
    # PowerShell unwraps single-element arrays on return, which breaks `.Count`
    # under StrictMode; callers wrap this in @() to restore array semantics.
    return @(Get-Process -Name the game -ErrorAction SilentlyContinue)
}

# --- The continue selection, and the refusal it used to hide ------------------
#
# t43's exit wrote a `continue_game.json` the game then silently declined. Every
# `--continuelastsave` launch dropped to the frontend menu, where no in-game
# session exists, so every world-ready observation failed on a null CInGameIdler
# pair and the phase burned its entire 600 s budget -- twice -- before saying
# anything, and what it finally said blamed the singletons. The discriminator
# was never in the harness's own log. It was in the game's, which showed the
# menu bookmark and no session launch at all.
#
# Two guards. They cover DIFFERENT failures and neither is the other's fallback:
#
#   * BEFORE launch, `Resolve-Hoi4ContinueSelection`: the selection must name a
#     save that is actually on disk. The harness used to check only that
#     continue_game.json EXISTED, so a selection left pointing at a deleted save
#     read as perfectly healthy.
#   * IN THE TIMEOUT, a stall description: the engine can decline a selection
#     whose file IS present -- a foreign game version, an incompatible modset, a
#     truncated save. Nothing readable off disk predicts that, and it is what
#     actually happened in t43: the declined selection named
#     `JAP_1941_09_20_17.hoi4`, a file that is still in `save games` to this day.
#     So the world-ready timeout now names the selection the process was
#     launched on, and says whether the engine's own log shows it starting a
#     campaign session (frontend.cpp writes that banner when, and only when, one
#     starts).
#
# WHAT THIS DELIBERATELY DOES NOT DO IS REFUSE EARLY. Two versions of this code
# tried. Refusing when the banner was absent aborts a slow load that has not
# reached its banner yet. Refusing when the banner is absent AND the log has
# gone quiet aborts a silent deserialize -- a review reproduced exactly that
# against these functions, refusing a campaign that became ready seconds later
# -- and a modset that renames or suppresses the banner fails the same way.
#
# Refusing a healthy capture costs a game hour; a misleading timeout costs the
# reader ten minutes. Those are not the same price, so the description ships and
# the early refusal does not. Closing the fast half needs an affirmative,
# engine-proven "this process is at the frontend menu" observable, which does
# not exist in this tree today -- recorded as a residual rather than
# approximated by a proxy.

<#
.SYNOPSIS
    The whole text of a file the game still holds open.

.DESCRIPTION
    the game keeps its logs open for the whole session, so this takes the widest
    share mode and never asks for write access.

    Reads the file WHOLE. Deciding which part belongs to the current session is
    `Select-Hoi4CurrentSessionLog`'s job, and it is separated from the I/O so it
    can be tested without one.

    Returns `$null` on any failure. The caller must treat that as "no opinion",
    never as "no banner" -- see the block comment above.
#>
function Read-Hoi4SharedText {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Path
    )

    $stream = $null
    try {
        $share = [System.IO.FileShare](
            [int][System.IO.FileShare]::ReadWrite -bor
            [int][System.IO.FileShare]::Delete
        )
        $stream = [System.IO.File]::Open(
            $Path,
            [System.IO.FileMode]::Open,
            [System.IO.FileAccess]::Read,
            $share
        )
        $reader = [System.IO.StreamReader]::new($stream)
        try {
            return $reader.ReadToEnd()
        } finally {
            $reader.Dispose()
            $stream = $null
        }
    } catch {
        return $null
    } finally {
        if ($null -ne $stream) {
            try { $stream.Dispose() } catch { }
        }
    }
}

<#
.SYNOPSIS
    The part of a log that belongs to the session launched after the sample.

.DESCRIPTION
    LENGTH ALONE CANNOT IDENTIFY A FILE GENERATION. If the engine truncates
    `game.log` at launch and then writes MORE than the file previously held --
    ordinary after a minute of loading -- a length comparison says "grew", and
    slicing at the pre-launch offset lands in the middle of the CURRENT session,
    past its launch banner.

    NEITHER CAN A HEAD SAMPLE. Two consecutive sessions open with near-identical
    lines, so a rewrite that preserves the first few hundred characters compares
    equal and gets sliced when it should not have been.

    So the whole pre-launch region has to be proven unchanged, and this takes a
    DIGEST of exactly that region. Same length or longer, and the first
    `PriorLength` characters still hash to what they hashed to before the
    launch: an append, and only the remainder is new. Anything else -- shorter,
    different digest, a digest the caller could not compute -- is a file this
    function cannot prove continuous, and it returns the whole text.

    AMBIGUITY RESOLVES TOWARD MORE TEXT, always. The caller uses this to decide
    what it can SAY about a stalled launch, so extra text can at worst make it
    describe an older session's banner, while missing text would make it report
    a launch that did happen as one that did not.
#>
function Select-Hoi4CurrentSessionLog {
    param(
        [Parameter(Mandatory = $true)][AllowNull()][AllowEmptyString()][string]$AllText,
        [Parameter(Mandatory = $true)][long]$PriorLength,
        [Parameter(Mandatory = $true)][AllowNull()][AllowEmptyString()][string]$PriorDigest
    )

    if ($null -eq $AllText) {
        return $null
    }
    # Nothing was there before, or nothing could be proven about it, so all of
    # this is treated as the current session's.
    if ($PriorLength -le 0 -or [string]::IsNullOrWhiteSpace($PriorDigest)) {
        return $AllText
    }
    if ($AllText.Length -lt $PriorLength) {
        return $AllText
    }
    $head = $AllText.Substring(0, [int]$PriorLength)
    if (-not (Get-Hoi4TextDigest -Text $head).Equals(
            $PriorDigest, [StringComparison]::OrdinalIgnoreCase)) {
        return $AllText
    }
    if ($AllText.Length -eq $PriorLength) {
        return ''
    }
    return $AllText.Substring([int]$PriorLength)
}

<#
.SYNOPSIS
    A stable digest of a string, for proving a file region is unchanged.
#>
function Get-Hoi4TextDigest {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Text
    )

    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($Text)
        return [BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-', '')
    } finally {
        $sha.Dispose()
    }
}

<#
.SYNOPSIS
    True when the log text shows the engine starting a campaign session.

.DESCRIPTION
    The engine emits one `[[ Launching <MODE>-game ]]` banner from frontend.cpp
    per campaign start, immediately followed by the campaign's start date. A
    launch that lands on the frontend menu instead never reaches it.

    `$null` and empty text are FALSE, and the caller must not confuse that with
    a proven absence: a log that could not be read returns `$null` from
    `Read-Hoi4SharedText` and the caller stops there.
#>
function Test-Hoi4SessionLaunchBanner {
    param(
        [Parameter(Mandatory = $true)][AllowNull()][AllowEmptyString()][string]$LogText
    )

    if ([string]::IsNullOrEmpty($LogText)) {
        return $false
    }
    return ($LogText -match '\[\[\s*Launching\s+[A-Za-z]+-game\s*\]\]')
}

<#
.SYNOPSIS
    Admit the launcher's continue selection, or refuse it by name.

.DESCRIPTION
    Takes the raw `continue_game.json` text rather than a path so the admission
    is a function of its inputs and can be exercised without a game.

    A selection flagged `is_remote` is NOT checked against the local save
    directory: the engine resolves those through the cloud provider, and
    refusing one for being absent locally would be a false refusal.
#>
function Resolve-Hoi4ContinueSelection {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Json,
        [Parameter(Mandatory = $true)][string]$SaveGamesDir,
        [Parameter(Mandatory = $true)][string]$ContinueGamePath
    )

    $parsed = $null
    try {
        $parsed = $Json | ConvertFrom-Json
    } catch {
        throw (
            "ContinueSelectionRefused: '$ContinueGamePath' is not readable as JSON " +
            "($($_.Exception.Message)). --continuelastsave would drop the game to the " +
            'frontend menu and the world-ready wait would then spend its whole budget ' +
            'on a campaign that was never loaded.'
        )
    }
    if ($null -eq $parsed) {
        throw "ContinueSelectionRefused: '$ContinueGamePath' parsed as JSON null."
    }
    $present = @($parsed.PSObject.Properties.Name)
    foreach ($field in @('title', 'filename')) {
        if ($present -notcontains $field) {
            throw (
                "ContinueSelectionRefused: '$ContinueGamePath' carries no '$field' " +
                "field, so the launcher selection cannot be checked at all."
            )
        }
    }
    $title = [string]$parsed.title
    $filename = [string]$parsed.filename
    if ([string]::IsNullOrWhiteSpace($filename)) {
        throw "ContinueSelectionRefused: '$ContinueGamePath' names an empty save filename."
    }
    # The engine resolves the selection inside its own save directory. A name
    # carrying a separator is not a selection this can check, and silently
    # accepting one would let the guard pass on a path it never looked at.
    if ($filename.IndexOfAny([char[]]@('\', '/', ':')) -ge 0) {
        throw (
            "ContinueSelectionRefused: '$ContinueGamePath' names '$filename', which is " +
            'a path rather than a save-directory entry.'
        )
    }
    $isRemote = ($present -contains 'is_remote') -and [bool]$parsed.is_remote
    $savePath = Join-Path $SaveGamesDir $filename
    if (-not $isRemote -and -not (Test-Path -LiteralPath $savePath -PathType Leaf)) {
        throw (
            "ContinueSelectionRefused: launcher continue selection '$title' names " +
            "'$filename', which is not in '$SaveGamesDir'. --continuelastsave would " +
            'drop the game to the frontend menu and the world-ready wait would then ' +
            'spend its whole budget on a campaign that was never loaded. Resume a save ' +
            'interactively once to let the launcher rewrite the selection.'
        )
    }
    return [pscustomobject]@{
        Title      = $title
        Filename   = $filename
        SavePath   = $savePath
        IsRemote   = $isRemote
        SaveOnDisk = (-not $isRemote)
    }
}

<#
.SYNOPSIS
    What to add to a world-ready timeout about the launch that never arrived.

.DESCRIPTION
    Reports two facts and draws no conclusion from them: which continue
    selection the process was launched on, and whether the engine's own log
    shows it starting a campaign session.

    IT DOES NOT SAY THE SELECTION WAS DECLINED. `$BannerSeen` false is
    consistent with a declined selection AND with a load that never got that
    far, and nothing available here separates them -- an earlier version of this
    code refused captures on that distinction and was wrong to. What it does is
    put the selection in front of the reader, which is exactly what t43/t44
    lacked: the failure was diagnosable only from the game's own log, and the
    harness's message pointed at the singletons instead.
#>
function New-Hoi4StallDescription {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Title,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$Filename,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$GameLogPath,
        [Parameter(Mandatory = $true)][AllowNull()][object]$BannerSeen
    )

    $launch = (
        "This process was launched with --continuelastsave on '$Title' " +
        "($Filename)."
    )
    if ($null -eq $BannerSeen) {
        return (
            "$launch The engine's log at '$GameLogPath' could not be read, so " +
            'whether it started a campaign session is unknown from here.'
        )
    }
    if ([bool]$BannerSeen) {
        return (
            "$launch The engine's log at '$GameLogPath' DOES show it starting a " +
            'campaign session, so the selection was accepted and the stall is ' +
            'after the load began.'
        )
    }
    return (
        "$launch The engine's log at '$GameLogPath' shows NO campaign session " +
        'starting. That is consistent with the engine declining the selection ' +
        'and dropping to the frontend menu -- the t43/t44 failure, where a save ' +
        'that was present on disk was refused for reasons invisible from here -- ' +
        'and equally consistent with a load that never reached its session. ' +
        'Read that log before blaming the singletons or the offsets. If the ' +
        'selection is the cause, resuming the save interactively once lets the ' +
        'launcher rewrite continue_game.json.'
    )
}

# Readiness is "the campaign is loaded AND in play", not "the singletons are
# readable". Those are not the same condition and the gap between them is a
# whole savegame load: the CGameState and CInGameIdler singletons are coherent,
# the speed is already deserialised and the pause latch is set well before the
# engine finishes entering the session. This phase used to accept that state,
# and the harness then reached the evidence-mode unpause mid-load, where
# Control-Hoi4Game.ps1 correctly refuses to mirror a transition the engine
# itself would decline -- reporting one unlucky sample as a permanent state and
# aborting the whole capture.
#
# The engine's own statement of the missing condition is CGameState+0xA39; the
# provenance, and the limits of what it proves, are in Hoi4WorldReady.Gate.ps1.
# It says the engine has ENTERED its in-game session, not that its post-load
# work has drained -- the settle phase below is what covers that, and this does
# not replace it. Waiting for the gate belongs HERE rather than in a second wait
# bolted after this phase, for three reasons:
#
#   * this phase already claims the condition. Its parameter is documented as
#     the wait "for the loaded campaign to expose stable world singletons", and
#     the singleton set was only ever a proxy for "the campaign is loaded".
#   * every consumer downstream needs it. The line right after this one asserts
#     on $readyState.Speed, which mid-load is a deserialised-but-not-live value;
#     settle, attach and the recorder all assume a live session.
#   * a second wait would only cover the evidence-drive path, because that is
#     the only path that invokes the control helper. The PASSIVE path -- the
#     shipped default -- would keep attaching the recorder to a process that is
#     still loading. Fixing the unshipped path and leaving the shipped one
#     exposed is the wrong place to put it.
#
# The budget is this phase's existing one. A refusal is still a refusal: it now
# says how long it waited rather than reporting a single sample.
function Wait-Hoi4WorldReady {
    param(
        [Parameter(Mandatory = $true)][int]$TargetProcessId,
        [Parameter(Mandatory = $true)][int]$TimeoutSeconds,
        [Parameter(Mandatory = $true)][uint64]$ExpectedCreationFiletimeUtc,
        # The launcher selection this process was started on, or $null when the
        # harness did not launch it. The PASSIVE path reuses a game the owner
        # started by hand: there is no selection to blame and no launch offset
        # to read from, so it gets no probe at all.
        [AllowNull()][object]$ContinueSelection,
        [AllowEmptyString()][string]$GameLogPath = '',
        [long]$GameLogPriorLength = 0,
        [AllowEmptyString()][string]$GameLogPriorDigest = ''
    )
    # Captured as a VALUE, not called by name: `.GetNewClosure()` parents the
    # closure's dynamic module on the GLOBAL scope, so a script-scoped function
    # resolves only when the script itself was run with `powershell -File` (which
    # makes its functions global). Run via `-Command "& .\script"` the script
    # keeps its own scope and a by-name call inside the closure dies with
    # CommandNotFound -- which is how t27's world-ready wait spent its whole
    # 600 s budget failing every probe. A variable IS captured by the closure,
    # in every invocation mode.
    $observeFn = ${function:Invoke-BoundedHoi4Observation}
    # Same capture-by-value rule for the four the describer needs.
    $readTextFn = ${function:Read-Hoi4SharedText}
    $selectSessionFn = ${function:Select-Hoi4CurrentSessionLog}
    $bannerFn = ${function:Test-Hoi4SessionLaunchBanner}
    $describeFn = ${function:New-Hoi4StallDescription}
    $describeStall = $null
    if ($null -ne $ContinueSelection -and -not [string]::IsNullOrWhiteSpace($GameLogPath)) {
        # Runs ONCE, inside the timeout path, and cannot shorten or fail the
        # wait. $null BannerSeen means the log could not be read, which the
        # description reports as unknown rather than as an absent banner.
        $describeStall = {
            param($WaitedSeconds)
            $bannerSeen = $null
            $all = & $readTextFn -Path $GameLogPath
            if ($null -ne $all) {
                $text = & $selectSessionFn `
                    -AllText $all `
                    -PriorLength $GameLogPriorLength `
                    -PriorDigest $GameLogPriorDigest
                if ($null -ne $text) {
                    $bannerSeen = [bool](& $bannerFn -LogText $text)
                }
            }
            return (& $describeFn `
                -Title $ContinueSelection.Title `
                -Filename $ContinueSelection.Filename `
                -GameLogPath $GameLogPath `
                -BannerSeen $bannerSeen)
        }.GetNewClosure()
    }
    return (Wait-Hoi4WorldReadyState `
        -TargetProcessId $TargetProcessId `
        -TimeoutSeconds $TimeoutSeconds `
        -DescribeStall $describeStall `
        -Observe {
            param($RemainingSeconds)
            & $observeFn `
                -TargetProcessId $TargetProcessId `
                -TimeoutSeconds $RemainingSeconds `
                -ExpectedCreationFiletimeUtc $ExpectedCreationFiletimeUtc
        }.GetNewClosure() `
        -TestAlive {
            $null -ne (Get-Process -Id $TargetProcessId -ErrorAction SilentlyContinue)
        }.GetNewClosure() `
        -ReportProgress {
            param($Progress)
            Write-Host (
                "phase=world_ready_wait pid=$($Progress.TargetProcessId) " +
                "waited_s=$($Progress.WaitedSeconds) remaining_s=$($Progress.RemainingSeconds) " +
                "observed=$($Progress.SawObservation) " +
                "player_input_gate=$(if ($null -eq $Progress.PlayerInputGate) { 'none' } else { $Progress.PlayerInputGate })"
            )
        })
}

# --- Phase 1: obtain a live campaign process -------------------------------
$launchedProcess = $null
$gamePid = 0
$recorderProcess = $null
# Set only on the launch path. The passive path reuses a game the owner started
# and configured, so the harness has neither a selection it chose nor a launch
# boundary in the log to read from.
$continueSelection = $null
$gameLogPath = Join-Path (Join-Path $PdxUserDir 'logs') 'game.log'
$gameLogPriorLength = 0L
$gameLogPriorDigest = ''

if ($reuseRunningGame) {
    $running = @(Get-Hoi4Processes)
    if ($running.Count -ne 1) {
        throw "Passive capture needs exactly one already-running hoi4.exe; found $($running.Count). Start and configure the game yourself before running the harness."
    }
    $gamePid = [int]$running[0].Id
    Write-Host "phase=reuse_passive pid=$gamePid"
} else {
    $running = @(Get-Hoi4Processes)
    if ($running.Count -ne 0) {
        if (-not $StopExistingGame) {
            throw "hoi4.exe PID(s) $(@($running.Id) -join ',') are running. Evidence-only launch requires -StopExistingGame; passive default mode reuses exactly one running game."
        }
        foreach ($proc in $running) {
            Assert-Hoi4ClientDrivingOptIn -Caller $PSCommandPath
            Write-Host "phase=stop_existing pid=$($proc.Id)"
            if ($DryRun) { continue }
            try { $proc.CloseMainWindow() | Out-Null } catch { }
            if (-not $proc.WaitForExit(15000)) {
                Stop-ExactProcess -Process $proc -Purpose 'pre-existing the game shutdown'
            }
        }
    }

    $continueGame = Join-Path $PdxUserDir 'continue_game.json'
    if (-not (Test-Path -LiteralPath $continueGame -PathType Leaf)) {
        throw "Launcher continue selection is missing: $continueGame"
    }
    $continueSelection = Resolve-Hoi4ContinueSelection `
        -Json (Get-Content -LiteralPath $continueGame -Raw) `
        -SaveGamesDir (Join-Path $PdxUserDir 'save games') `
        -ContinueGamePath $continueGame
    # Sampled BEFORE the game is created, so the stall description can tell this
    # session's log from the previous one. Length AND a digest of exactly that
    # much text: neither the size nor a head sample can identify a generation on
    # its own, because a truncating engine that then writes more than the file
    # used to hold looks like an append and two sessions open alike. See
    # Select-Hoi4CurrentSessionLog.
    $gameLogPriorLength = 0L
    $gameLogPriorDigest = ''
    if (Test-Path -LiteralPath $gameLogPath -PathType Leaf) {
        # No sample means the whole file is treated as current, which can only
        # make the description mention an older session's banner.
        $priorText = Read-Hoi4SharedText -Path $gameLogPath
        if ($null -ne $priorText) {
            $gameLogPriorLength = [long]$priorText.Length
            $gameLogPriorDigest = Get-Hoi4TextDigest -Text $priorText
        }
    }
    Write-Host (
        "phase=launch continue_title=$($continueSelection.Title) " +
        "continue_file=$($continueSelection.Filename) " +
        "save_on_disk=$($continueSelection.SaveOnDisk) " +
        "game_log_prior_chars=$gameLogPriorLength " +
        "game_log_prior_digest=$(if ([string]::IsNullOrEmpty($gameLogPriorDigest)) { 'none' } else { $gameLogPriorDigest.Substring(0, 12) })"
    )

    # Launch through WMI rather than Start-Process. The recorder refuses a
    # target that already belongs to a job object (it takes its own injection
    # lease as a job), and a process started directly inherits the caller's job
    # when this script runs under an automation host that uses one. Win32_Process
    # creates the game from the WMI service, outside any such job.
    $launchCommandLine = '"' + $Hoi4Exe + '" --continuelastsave --gdpr-compliant "-userdir=' + $PdxUserDir + '"'
    if ($DryRun) {
        Write-Host "DRY RUN launch: $launchCommandLine"
    } else {
        $preWmiLaunchIds = @(
            Get-Hoi4Processes | ForEach-Object { [int]$_.Id }
        )
        $wmiLaunchStartedUtc = [DateTime]::UtcNow
        $created = Invoke-BoundedWmiLaunch `
            -CommandLine $launchCommandLine `
            -CurrentDirectory (Split-Path -Parent $Hoi4Exe) `
            -ExpectedExecutablePath $Hoi4Exe `
            -LaunchStartedUtc $wmiLaunchStartedUtc `
            -PreexistingProcessIds $preWmiLaunchIds
        if ([int]$created.ReturnValue -ne 0) {
            throw "Win32_Process.Create failed for hoi4.exe with return value $($created.ReturnValue)."
        }
        $gamePid = [int]$created.ProcessId
        $launchedProcess = Get-Process -Id $gamePid -ErrorAction Stop
        $launchedProcessCreation = [uint64](
            $launchedProcess.StartTime.ToUniversalTime().ToFileTimeUtc()
        )
        if ($launchedProcessCreation -ne
                [uint64]$created.ProcessCreationFiletimeUtc -or
            -not [StringComparer]::OrdinalIgnoreCase.Equals(
                $launchedProcess.Path,
                [string]$created.ExecutablePath
            ) -or
            -not [StringComparer]::OrdinalIgnoreCase.Equals(
                [string]$created.ExecutablePath,
                $Hoi4Exe
            )) {
            throw 'Caller could not preserve the exact WMI PID/creation/image handoff.'
        }
        Write-Host "phase=launched pid=$gamePid"
    }
}

if ($DryRun) {
    Write-Host "DRY RUN: stopping before world-ready wait. out_dir=$OutDir"
    return
}

$gameProcessForEpoch = Get-Process -Id $gamePid -ErrorAction Stop
$gameCreationFiletimeUtc = [uint64](
    $gameProcessForEpoch.StartTime.ToUniversalTime().ToFileTimeUtc()
)
if ($gameCreationFiletimeUtc -eq 0) {
    throw "the game PID $gamePid exposed an invalid zero process-creation FILETIME."
}
if ($EvidenceOnlyDriveGame -and
    $gameCreationFiletimeUtc -ne [uint64]$created.ProcessCreationFiletimeUtc) {
    throw 'WMI-launched game creation epoch changed before world readiness.'
}

# Pin the game to the floor machine's cores, if one is being emulated.
#
# HERE, and not at either launch site, because this is the first point at which
# BOTH paths -- the reused user-owned game and the WMI-created one -- have a PID
# bound to a verified creation epoch. It is also before world readiness, so the
# post-load work a real floor machine would also do at four cores is inside the
# emulation rather than outside it.
#
# For the reused game this is the harness touching a process it does not own, so
# the prior mask is kept and restored in cleanup. Affinity is a scheduling
# attribute: it writes no game state, no memory and no input, and it is applied
# well before the graded window opens.
if ($null -ne $cpuAffinityPlan) {
    # `-PriorMaskOut` rather than the return value: a verification failure
    # throws, and cleanup still has to know what this game's mask was.
    [void]$cpuAffinityApplied.Add((Set-Hoi4ProcessAffinity `
        -ProcessId $gamePid `
        -Mask $cpuAffinityPlan.Mask `
        -Label 'the game' `
        -ExpectedCreationFiletimeUtc $gameCreationFiletimeUtc `
        -PriorMaskOut ([ref]$gameAffinityPrior)))
}

# --- Phase 2: establish and verify pre-attach speed --------------------------
$readyState = Wait-Hoi4WorldReady `
    -TargetProcessId $gamePid `
    -TimeoutSeconds $WorldReadyTimeoutSeconds `
    -ExpectedCreationFiletimeUtc $gameCreationFiletimeUtc `
    -ContinueSelection $continueSelection `
    -GameLogPath $gameLogPath `
    -GameLogPriorLength $gameLogPriorLength `
    -GameLogPriorDigest $gameLogPriorDigest
Write-Host "phase=world_ready pid=$gamePid initial_paused=$($readyState.Paused) initial_speed=$($readyState.Speed) player_input_gate=$($readyState.PlayerInputGate)"

if ($EvidenceOnlyDriveGame) {
    # This is unshipped setup, before the graded attach boundary. The sharp
    # helper independently requires the process-local evidence opt-in token.
    $control = $null
    $controlDeadline = [DateTime]::UtcNow.AddSeconds(120)
    while ([DateTime]::UtcNow -lt $controlDeadline) {
        $attempt = Invoke-BoundedHoi4Control `
            -TargetProcessId $gamePid `
            -RequestedSpeed $Speed `
            -Action Play `
            -ExpectedCreationFiletimeUtc $gameCreationFiletimeUtc
        # The game can die mid-loop. Without this guard the next property
        # access throws PropertyNotFoundStrict and the real event -- the game
        # process disappearing -- is reported as a PowerShell type error.
        if ($null -eq $attempt) {
            $alive = @(Get-Process -Id $gamePid -ErrorAction SilentlyContinue)
            if ($alive.Count -eq 0) {
                throw "hoi4.exe PID $gamePid exited during evidence setup; no control sample could be taken."
            }
            Write-Host "phase=evidence_setup_retry sample=none pid_alive=True"
            Start-Sleep -Milliseconds 1000
            continue
        }
        if ([int]$attempt.Speed -eq $Speed -and
            -not [bool]$attempt.Paused -and
            [bool]$attempt.Advancing) {
            $control = $attempt
            break
        }
        Write-Host "phase=evidence_setup_retry speed=$($attempt.Speed) paused=$($attempt.Paused) advancing=$($attempt.Advancing)"
        Start-Sleep -Milliseconds 1000
    }
    if ($null -eq $control) {
        throw "Evidence-only setup could not hold speed $Speed with an advancing clock within 120 s."
    }
    Write-Host "phase=evidence_setup_complete speed=$($control.Speed) paused=$($control.Paused) hour_delta=$($control.HourDelta)"
    $controlPath = Join-Path $OutDir 'evidence_only_pre_window_control.json'
    ($control | ConvertTo-Json -Depth 8) |
        Set-Content -LiteralPath $controlPath -Encoding UTF8
} elseif ([int]$readyState.Speed -ne $Speed) {
    throw (
        "Passive capture requires the user-owned game already at speed $Speed; " +
        "observed speed=$($readyState.Speed). Configure the game before invoking the harness."
    )
}

# Settle exists for one reason: injecting while the engine is still finishing
# its parallel post-load work has crashed the game, and observed advancement is
# the evidence that work has drained.
#
# It used to demand advancement from EVERY sample, which made an ordinary
# auto-pause -- the game raises one for events, decisions, focus completion, war
# declarations and peace conferences -- indistinguishable from the post-load
# race it was built to avoid. It now requires a CUMULATIVE amount of observed
# advancement inside a hard deadline instead. A paused or tick-held sample
# neither counts toward the requirement nor fails it: it simply does not
# advance the settle, and the hard deadline still terminates the phase.
if ($SettleSeconds -gt 0) {
    Write-Host "phase=settle seconds=$SettleSeconds"
    $settleStarted = [DateTime]::UtcNow
    # Requiring the same number of advancing samples the old loop would have
    # taken keeps the mitigation's strength while removing its pause fragility.
    $settleSamplesRequired = [Math]::Max(1, [int][Math]::Floor($SettleSeconds / 10))
    $settleSamplesObserved = 0
    # A pause may delay the settle but it may never extend this phase forever.
    # Two minutes of recovery headroom is enough for one full control retry
    # window and leaves a deterministic hard stop for unattended captures.
    $settleHardDeadline = $settleStarted.AddSeconds($SettleSeconds + 120)
    while ($settleSamplesObserved -lt $settleSamplesRequired) {
        if ([DateTime]::UtcNow -ge $settleHardDeadline) {
            throw (
                "Passive settle observed $settleSamplesObserved of $settleSamplesRequired required " +
                "advancing samples before its hard deadline. The engine spent the settle window " +
                "paused, held, or stopped; the harness will not repair client state."
            )
        }
        Start-Sleep -Seconds 10
        $sample = Invoke-BoundedHoi4Observation `
            -TargetProcessId $gamePid `
            -ExpectedCreationFiletimeUtc $gameCreationFiletimeUtc
        # Speed is the one settle observation that is still fatal on sight: the
        # engine's pause path provably never writes CGameState+0x4BC, so a
        # changed speed is a write by something other than the engine.
        if ([int]$sample.Speed -ne $Speed) {
            throw (
                "Passive settle observed a game-speed change: expected $Speed, observed " +
                "$($sample.Speed). The harness will not repair client state."
            )
        }
        if ([bool]$sample.Advancing) {
            $settleSamplesObserved++
        }
        # RECORD the settle observation. It is the only hooks-free measurement
        # of this exact process on this exact save at this exact speed that the
        # run ever takes, which makes it the paired control for everything
        # observed after injection -- same process, same save, same speed, no
        # collector. Runs 20260730-postmerge-04 and 20260731-postmerge-06 took
        # these samples and threw them away, so their post-attach CPU and clock
        # numbers had nothing to be compared against and could not, on their
        # own, indicate anything.
        #
        # Deliberately NOT routed through Write-PassiveWitnessSample: that calls
        # the witness verdict authority, which would let a settle-phase stall
        # abort the run. Settle legitimately tolerates pauses; this is evidence
        # collection, not a gate, and it must not change what the harness does.
        $settleControl = [ordered]@{
            schema                        = 'hoi4-settle-control-sample-v2'
            observed_at_utc               = [string]$sample.ObservedAtUtc
            observed_at_unix_ms           = [int64]$sample.ObservedAtUnixMs
            phase                         = 'settle_no_collector'
            pid                           = $gamePid
            process_creation_filetime_utc = $gameCreationFiletimeUtc
            expected_speed                = $Speed
            speed_final                   = [int]$sample.Speed
            paused_final                  = [bool]$sample.Paused
            pause_forced_final            = [bool]$sample.PauseForced
            tick_inhibited_final          = [bool]$sample.TickInhibited
            hour_a                        = [int]$sample.HourA
            hour_b                        = [int]$sample.HourB
            hour_delta                    = [int]$sample.HourDelta
            tick_progress_bits_a          = [int]$sample.TickProgressBitsA
            tick_progress_bits_b          = [int]$sample.TickProgressBitsB
            observed_window_ms            = [int64]$sample.ObservedWindowMs
            processor_milliseconds        = [int64]$sample.ProcessorMilliseconds
            # The same host and pacing context the witness row carries, and
            # it has to be here too: this file IS the free-run control arm,
            # and a control that cannot be compared to the treatment on host
            # state is not a control. TG's confound lived exactly in the gap
            # between these two arms.
            available_parallelism         = [int]$sample.AvailableParallelism
            # Uncast for the same reason as the witness row above.
            target_processor_affinity_cores = $sample.TargetProcessorAffinityCores
            target_page_fault_count       = $sample.TargetPageFaultCount
            target_working_set_bytes      = $sample.TargetWorkingSetBytes
            host_state_unavailable        = $sample.HostStateUnavailable
            advancing                     = [bool]$sample.Advancing
        }
        ($settleControl | ConvertTo-Json -Compress -Depth 8) |
            Add-Content -LiteralPath $settleControlPath -Encoding UTF8
        $hardRemaining = [Math]::Max(
            0,
            [Math]::Ceiling(($settleHardDeadline - [DateTime]::UtcNow).TotalSeconds)
        )
        Write-Host (
            "phase=settle_progress speed=$($sample.Speed) advancing=$($sample.Advancing) " +
            "paused=$($sample.Paused) pause_forced=$($sample.PauseForced) " +
            "tick_inhibited=$($sample.TickInhibited) " +
            "hour_a=$($sample.HourA) hour_b=$($sample.HourB) " +
            "window_ms=$($sample.ObservedWindowMs) " +
            "cpu_ms=$($sample.ProcessorMilliseconds) " +
            "advancing_samples=$settleSamplesObserved/$settleSamplesRequired " +
            "hard_remaining_s=$hardRemaining"
        )
    }
    Write-Host 'phase=settled'
}

# The final pre-attach witness establishes the graded boundary. From this call
# until authenticated collector teardown, this script performs only read-only
# status observations against HOI4. It never pauses, resumes, changes speed,
# launches, stops, focuses, minimizes, or otherwise drives the client.
# Withdraw the client-driving authority for the whole graded window.
#
# The passive default never grants it, but -EvidenceOnlyDriveGame does, and the
# token was previously left set for the rest of the process. The sharp control
# helper would then still have authorized a pause or a speed write inside the
# window -- and a pause write is invisible to the witness, because writing
# CInGameIdler+0x6C1 leaves the speed selector untouched and looks exactly like
# the game pausing itself. So the structural control has to be real rather than
# assumed: the token is removed here and restored only after the window closes.
$clientDrivingOptInSuspended = [Environment]::GetEnvironmentVariable(
    'BATTLE_PLAN_CLIENT_DRIVING_OPT_IN',
    [EnvironmentVariableTarget]::Process
)
[Environment]::SetEnvironmentVariable(
    'BATTLE_PLAN_CLIENT_DRIVING_OPT_IN',
    $null,
    [EnvironmentVariableTarget]::Process
)
if ($null -ne [Environment]::GetEnvironmentVariable(
        'BATTLE_PLAN_CLIENT_DRIVING_OPT_IN',
        [EnvironmentVariableTarget]::Process
    )) {
    throw 'The graded window cannot open while the client-driving opt-in token is still set.'
}

[void](Write-PassiveWitnessSample `
    -Phase 'pre_attach' `
    -TargetProcessId $gamePid `
    -ExpectedCreationFiletimeUtc $gameCreationFiletimeUtc `
    -ExpectedSpeed $Speed)
$gradedWindowStarted = $true
Write-Host "phase=graded_window_start policy=zero_client_writes speed=$Speed pid=$gamePid"

# --- Phase 3: raw H4CR capture --------------------------------------
$sessionId = [guid]::NewGuid().ToString('D').ToLowerInvariant()
$captureId = "headless-raw-capture-$stamp"
$durationArg = [string]::Format([Globalization.CultureInfo]::InvariantCulture, '{0:0.###}', $DurationSeconds)
$recorderArgs = @(New-RecorderArgumentPlan `
    -GamePid $gamePid `
    -CollectorDll $collectorDll `
    -OutputDirectory $OutDir `
    -DurationArgument $durationArg `
    -SampleMilliseconds $SampleMilliseconds `
    -MinGameHours $MinGameHours `
    -StreamCompression $StreamCompression `
    -SchedulerPollMilliseconds $SchedulerPollMilliseconds `
    -SchedulerQueueCapacity $SchedulerQueueCapacity `
    -ReaderConcurrency $ReaderConcurrency `
    -SessionId $sessionId `
    -CaptureId $captureId `
    -ScheduledDomains $scheduledDomains `
    -StaticCatalogs $staticCatalogs `
    -SkipHostFold ([bool]$SkipHostFold) `
    -KeepSpoolAfterSeal ([bool]$KeepSpoolAfterSeal) `
    -ContentCensusCacheDirectory $ContentCensusCacheDir `
    -FoldCacheDirectory $FoldCacheDir)

($recorderArgs -join "`n") | Set-Content -LiteralPath (Join-Path $OutDir 'recorder_args.txt') -Encoding UTF8

$stdoutPath = Join-Path $OutDir 'collector_record.stdout.log'
$stderrPath = Join-Path $OutDir 'collector_record.stderr.log'
$readyReceiptPath = Join-Path $OutDir 'CAPTURE_READY_V2.json'
Write-Host "phase=capture duration_s=$durationArg compression=none scheduled_domains=$($scheduledDomains.Count) out_dir=$OutDir"

# SAMPLED BEFORE THE RECORDER EXISTS, which is the only place the answer means
# anything. A teardown receipt present at this instant cannot be this attempt's,
# so its existence is not evidence that this attempt's collector has detached;
# the witness-phase boundary below keys on that. Testing it any later -- after
# injection, after READY -- lets this attempt's OWN receipt be mistaken for a
# stale one on a short or early-finalizing capture, which disables the boundary
# and leaves detached samples labelled `capture` (Codex gate finding, round 2).
$teardownReceiptPreexisting = Test-Path -LiteralPath $collectorTeardownReceiptPath -PathType Leaf
if ($teardownReceiptPreexisting) {
    Write-Host "phase=teardown_receipt_preexisting path=$collectorTeardownReceiptPath detail=witness_phase_boundary_disabled_for_this_attempt"
}

# Redirect straight to files rather than in-memory pipes: an in-memory read
# hides the recorder's progress until it exits, which makes a startup hang
# indistinguishable from a slow capture.
$recorderProcess = Start-Process -FilePath $collectorRecord `
    -ArgumentList (ConvertTo-NativeArgumentLine $recorderArgs) `
    -NoNewWindow -PassThru `
    -RedirectStandardOutput $stdoutPath `
    -RedirectStandardError $stderrPath
# Immediately after the process exists, and before anything that can throw:
# from here on there may be a recorder holding hook custody, and every cleanup
# decision below turns on knowing that.
$collectorInjectionStarted = $true
# From this instant a capture EXISTS, so "was it checked for undefined status
# bits?" has a real answer, and until something produces one the answer is NO.
# The default is therefore the blocking verdict, cleared only by an actual
# answer further down. Set here rather than at the report because everything
# between here and the report can throw -- a passive-witness write, a control
# call, any bug -- and a throw jumps straight to cleanup, where a null verdict
# used to mean "checked and clean" when nothing had been checked at all.
$undefinedStatusBitFailure =
    'the capture started but never reached the raw-capture report, so whether it ' +
    'published an undefined status bit was never asked.'
# THE WAIT THAT CLEANUP PERFORMS IS BOUNDED FROM HERE, not from READY.
#
# These three used to be computed further down -- after the handle grab and
# after the whole CPU-affinity block. Cleanup's finalize wait is guarded by
# `$null -ne $recorderDeadline`, and `Update-RecorderFinalizeDeadline` returns
# immediately on a null ceiling, so a throw anywhere in between left BOTH null
# and the wait silently skipped itself. That is not a wait that gave up early;
# it is a wait that never ran. Run t38 threw in the affinity block and exited
# with the game and collector_record both still live, while the recorder went on to
# finish, fold, seal and publish an authenticated teardown receipt four minutes
# later with nobody left to read it.
#
# The budget is re-anchored at READY below and only ever EXTENDS, so anchoring
# it here costs nothing and buys the invariant: from the instant a recorder
# exists, there is a bound on how long this script will wait for it, and the
# bound is never null.
$recorderBudgetSeconds = $DurationSeconds + $RecorderOverheadSeconds
$recorderDeadline = [DateTime]::UtcNow.AddSeconds($recorderBudgetSeconds)
$recorderFinalizeCeiling = [DateTime]::UtcNow.AddSeconds($FinalizeCeilingSeconds)
# Take a real handle on the child NOW, while it is alive.
#
# `Start-Process -PassThru` under Windows PowerShell 5.1 hands back a Process
# object with no cached process handle. `HasExited` still works -- it re-queries
# by PID -- but `.ExitCode` reads back $null forever, and neither `Refresh()`
# nor `WaitForExit()` repairs it once the child is gone. This script read
# `.ExitCode` at the end and threw 'collector_record did not expose a process
# exit code', so every capture that actually SUCCEEDED was reported as a failed
# attempt. Measured on 5.1.26100.8875: baseline null, handle touched null-free.
#
# Touching `.Handle` forces .NET to open and keep one, which both makes the
# exit status readable afterwards and pins the PID against reuse. It throws if
# the child is already gone; that is not fatal here, because the ready-wait
# loop below reports an instantly-dead recorder with a far better message.
try {
    $null = $recorderProcess.Handle
} catch {
    Write-Host "phase=recorder_handle_unavailable reason=$($_.Exception.Message)"
}

# Put the recorder on the SAME cores as the game.
#
# collector_record is the host; the reader lanes live in the injected DLL and
# are already covered by the game's mask. What is masked here is the spool
# writer, the compressor and the post-capture fold -- which on a four-core box
# compete with the engine for the same four cores, and would otherwise get free
# cores no floor machine has.
#
# AFTER the injection-started flag is set and after the handle grab, both
# (the flag is not spelled out here: `check_no_stranded_graded_window` counts
# occurrences of that assignment and requires exactly one, so quoting it in a
# comment turns a passing order check into a duplicate-assignment failure --
# which is what it did on this line until lane EQ noticed the red)
# deliberately. Everything between Start-Process and that assignment must be
# incapable of throwing, or a failure there leaves a live recorder that cleanup
# does not know exists.
#
# CONTAINED, NOT FATAL, and t38 is why. The reasoning used to be "failing is the
# right outcome; a capture whose mask did not take is not a floor measurement",
# and the first half of that is still true -- it just does not follow that the
# attempt should be abandoned. This block pins a mask and writes a receipt about
# it. It decides nothing about the client, nothing about hook custody, and
# nothing about whether a receipt may be believed, so its failure is a fact to
# RECORD, not a reason to walk away from a healthy recorder mid-capture. t38's
# throw here killed the witness, left the graded window open, exited 2, and left
# the game and collector_record running; the recorder then finished, tore down and
# sealed entirely on its own, which is the proof that waiting was available the
# whole time.
#
# The refusal is not softened: `$supervisionFailures` is non-empty afterwards,
# the attempt returns the refusal code, and CAPTURE_REFUSAL_V1.json names the
# step. What changes is that it gets there through the normal end-of-run
# accounting instead of through an abandoned try block.
[void](Invoke-CaptureSupervisionStep `
    -Step 'recorder_cpu_affinity' `
    -Failures $supervisionFailures `
    -Action {
if ($null -ne $cpuAffinityPlan) {
    $recorderCreationFiletimeUtc = $null
    try {
        $recorderCreationFiletimeUtc = [uint64](
            $recorderProcess.StartTime.ToUniversalTime().ToFileTimeUtc()
        )
    } catch {
        # An unreadable start time is not a reason to skip the mask; it only
        # means the PID cannot be re-bound to its epoch, and this PID came
        # straight out of Start-Process moments ago.
        Write-Host "phase=recorder_creation_epoch_unavailable reason=$(($_.Exception.Message -replace '\s+', ' ').Trim())"
    }
    [void]$cpuAffinityApplied.Add((Set-Hoi4ProcessAffinity `
        -ProcessId $recorderProcess.Id `
        -Mask $cpuAffinityPlan.Mask `
        -Label 'collector_record' `
        -ExpectedCreationFiletimeUtc $recorderCreationFiletimeUtc `
        -PriorMaskOut $null))

    # Both masks are now on real processes, so the applied record can be
    # published. Kept separate from passive_capture_policy.json on purpose: that
    # file is written before anything is launched and carries the PLAN, and a
    # plan is not evidence that a process ever ran under it.
    $cpuAffinityReceipt = [ordered]@{
        schema                      = 'hoi4-harness-cpu-affinity-v1'
        emulates                    = 'core_count_only'
        physical_cores              = $cpuAffinityPlan.RequestedCores
        one_thread_per_core         = $cpuAffinityPlan.OneThreadPerCore
        mask                        = $cpuAffinityPlan.MaskHex
        processor_group             = $cpuAffinityPlan.Group
        efficiency_class            = $cpuAffinityPlan.EfficiencyClass
        selected_core_indexes       = $cpuAffinityPlan.SelectedCoreIndexes
        selected_logical_processors = $cpuAffinityPlan.LogicalProcessors
        machine_physical_cores      = $cpuAffinityPlan.MachinePhysicalCores
        machine_logical_processors  = $cpuAffinityPlan.MachineLogicalProcessors
        # Context only. This runs INSIDE the graded window, so a WMI hiccup
        # here must not be the thing that stops a capture that is already
        # injected -- an absent RAM figure is a worse receipt, not a worse run.
        machine_physical_memory_bytes = $(
            try { (Get-CimInstance -ClassName Win32_ComputerSystem).TotalPhysicalMemory }
            catch { $null }
        )
        processes                   = @(
            $cpuAffinityApplied | ForEach-Object {
                [ordered]@{
                    label        = $_.Label
                    pid          = $_.ProcessId
                    prior_mask   = (Format-Hoi4AffinityMask -Mask $_.PriorMask)
                    applied_mask = (Format-Hoi4AffinityMask -Mask $_.AppliedMask)
                    verified     = $true
                }
            }
        )
        # NOT masked, and named so nobody has to guess: the harness's own
        # PowerShell host, its bounded helper processes, the 5 s read-only
        # witness, and everything the OS does. A real floor machine pays for
        # those out of the same four cores, so this measurement is optimistic by
        # whatever they cost.
        unmasked_participants       = @(
            'harness powershell host',
            'bounded control/observer helpers',
            'passive clock witness',
            'operating system and drivers'
        )
        memory_floor_simulated      = $false
        memory_floor_note           = @(
            'The 8 GB half of the floor is deliberately NOT emulated here.',
            'A job-object memory cap limits COMMITTED memory, so it produces allocation failure where a small machine produces paging: the game would OOM and the recorder''s 300-400 MiB transient raw custody would fail closed as backpressure for a reason no 8 GB machine would produce.',
            'Assigning the game to a job also requires PROCESS_SET_QUOTA|PROCESS_TERMINATE on the client, contradicting the graded observer access this harness publishes and the target lease''s target_quota_right/target_termination_right = false.',
            'The game is launched through Win32_Process.Create specifically to keep it out of any job object; putting it back into one by hand re-creates the condition that launch path exists to avoid.',
            'To restrict physical RAM, restrict it for real: bcdedit /set {current} removememory (or truncatememory) plus a reboot, or run on the actual floor machine.'
        )
    }
    $cpuAffinityReceipt | ConvertTo-Json -Depth 8 |
        Set-Content `
            -LiteralPath (Join-Path $OutDir 'HARNESS_CPU_AFFINITY_V1.json') `
            -Encoding UTF8
}
})

[void](Write-PassiveWitnessSample `
    -Phase 'attach_started' `
    -TargetProcessId $gamePid `
    -ExpectedCreationFiletimeUtc $gameCreationFiletimeUtc `
    -ExpectedSpeed $Speed)

$injectionReadyDeadline = [DateTime]::UtcNow.AddSeconds($InjectionReadyTimeoutSeconds)
$injectionProgressDeadline = [DateTime]::UtcNow
$nextPassiveWitnessDeadline = [DateTime]::UtcNow.AddSeconds($WitnessIntervalSeconds)
$gameDied = $false
while ($true) {
    $recorderProcess.Refresh()
    if ($recorderProcess.HasExited) {
        throw "collector_record exited with code $($recorderProcess.ExitCode) before publishing its authenticated ready receipt."
    }
    if (-not (Get-Process -Id $gamePid -ErrorAction SilentlyContinue)) {
        $gameDied = $true
        throw "the game PID $gamePid exited before collector_record published its authenticated ready receipt."
    }
    if (Test-Path -LiteralPath $readyReceiptPath -PathType Leaf) {
        break
    }
    $now = [DateTime]::UtcNow
    if ($now -ge $injectionReadyDeadline) {
        throw "collector_record published no authenticated passive ready receipt within $InjectionReadyTimeoutSeconds s."
    }
    if ($now -ge $nextPassiveWitnessDeadline) {
        [void](Write-PassiveWitnessSample `
            -Phase 'ready_wait' `
            -TargetProcessId $gamePid `
            -ExpectedCreationFiletimeUtc $gameCreationFiletimeUtc `
            -ExpectedSpeed $Speed)
        $nextPassiveWitnessDeadline = [DateTime]::UtcNow.AddSeconds(
            $WitnessIntervalSeconds
        )
    }
    if ($now -ge $injectionProgressDeadline) {
        $remaining = [Math]::Max(
            0,
            [Math]::Ceiling(($injectionReadyDeadline - $now).TotalSeconds)
        )
        Write-Host "phase=injection_wait remaining_s=$remaining"
        $injectionProgressDeadline = $now.AddSeconds(5)
    }
    Start-Sleep -Milliseconds 250
}

# Re-anchor the recorder budget on READY.
#
# `-RecorderOverheadSeconds` is documented as startup + finalize headroom ON TOP
# of `-DurationSeconds`, but the deadline was taken at Start-Process while the
# recorder's own `--duration-s` clock starts here, when it publishes READY.
# Everything in between -- process start, injection, hook installation, the
# authenticated handshake -- was therefore silently subtracted from the headroom
# left to seal the stream. t20 spent 24.6 s reaching READY on a run whose fold
# took ~4 minutes to turn 2.46 GB into a 770 MB replay, so the margin an
# operator asked for and the margin they got were not the same number.
#
# Only ever extends: the launch anchor stays in force until READY, so a recorder
# that never gets there is still bounded by it, and the bound never moves
# backwards. Total exposure remains -InjectionReadyTimeoutSeconds + the budget.
$recorderReadyDeadline = [DateTime]::UtcNow.AddSeconds($recorderBudgetSeconds)
if ($recorderReadyDeadline -gt $recorderDeadline) {
    $recorderDeadline = $recorderReadyDeadline
}
# The progress bound never runs past this, so a recorder that keeps reporting
# work forever is still bounded. Anchored at READY like the budget above, and
# like the budget it replaces a launch-time value rather than a null: READY is
# always later than Start-Process, so this only ever moves the ceiling out.
$recorderFinalizeCeiling = [DateTime]::UtcNow.AddSeconds($FinalizeCeilingSeconds)
Write-Host "phase=recorder_budget_anchored_at_ready budget_s=$recorderBudgetSeconds duration_s=$DurationSeconds overhead_s=$RecorderOverheadSeconds deadline_utc=$($recorderDeadline.ToString('o')) finalize_no_progress_s=$FinalizeNoProgressSeconds finalize_ceiling_utc=$($recorderFinalizeCeiling.ToString('o'))"

$expectedDurationMilliseconds = [uint32](
    [decimal]::Parse(
        $durationArg,
        [Globalization.CultureInfo]::InvariantCulture
    ) * 1000
)
$collectorControlPath = Join-Path $releaseRoot 'collector_control.json'
# EVERY WAY THIS CAN FAIL IS NAMED BEFORE IT PROPAGATES.
#
# The verifier refuses a receipt in two very different ways and, until this
# `catch`, the harness could tell them apart only by reading prose. The clause
# accumulator produces a refusal that already knows which field disagreed and
# whether the disagreement costs custody. The checks that run BEFORE it -- the
# mirror against the collector's own producer source, the receipt's shape, the
# control file -- throw before either authority is published, so the run loses
# custody as well as the capture. That second shape is issue #151 and it is what
# ended t36: the mirror had drifted one field at offset 20, the throw arrived
# here with nothing structured attached, and the attempt stopped with the game
# pinned, the settings guard applied, the graded window open, and no
# machine-readable statement of the mismatch anywhere on disk.
#
# The DECISION is unchanged -- a refused READY still stops the capture, and the
# bare `throw` re-raises the original error untouched so the outer handler still
# records `$harnessAbortReason` and `Confirm-CollectorTeardownReceipt` still
# quotes it. What is added is the naming: the class, the field, and both sides
# of the comparison, on stdout and in CAPTURE_REFUSAL_V1.json, before anything
# unwinds. Cleanup then handles the game under the custody rules it already
# has, and now it can say why.
try {
    $sharedReadyAuthority = Test-PassiveCaptureReadyReceipt `
    -RunDir $OutDir `
    -TargetPid ([uint32]$gamePid) `
    -TargetCreationFiletimeUtc $gameCreationFiletimeUtc `
    -SessionId $sessionId `
    -CaptureId $captureId `
    -RequestedDurationMs $expectedDurationMilliseconds `
    -TargetExecutableSha256 $observedExeSha256 `
    -CollectorDllSha256 $collectorDllSha256 `
    -ScheduledDomainNames $scheduledDomains `
    -SchedulerPollMs ([uint32]$SchedulerPollMilliseconds) `
    -SchedulerQueueCapacity ([uint32]$SchedulerQueueCapacity) `
    -SchedulerWorkerCount ([uint32]$ReaderConcurrency) `
    -CollectorControlPath $collectorControlPath `
    -CustodyAuthorityOut ([ref]$sharedReadyCustodyAuthority)
} catch {
    $readyRefusal = New-PassiveReadyRefusalRecord `
        -ErrorRecord $_ `
        -CustodyEstablished ($null -ne $sharedReadyCustodyAuthority) `
        -Stage 'ready_validation'
    [void]$captureRefusals.Add($readyRefusal)
    Write-Host (
        "phase=capture_refused stage=$($readyRefusal.stage) " +
        "class=$($readyRefusal.class) clauses=$(@($readyRefusal.clauses).Count) " +
        "custody_established=$($readyRefusal.custody_established)"
    )
    foreach ($clause in @($readyRefusal.clauses)) {
        Write-Host (
            "phase=capture_refused_clause side=$($clause.side) " +
            "field=$($clause.field) expected=$($clause.expected) " +
            "observed=$($clause.observed)"
        )
    }
    Write-CaptureRefusalReceipt
    # Unchanged decision, unchanged exception. `throw` with no argument inside a
    # `catch` re-raises the current error exactly as it arrived, so the outer
    # recorder keeps the message it has always had.
    throw
}
$readyReceiptSha256 = [string]$sharedReadyAuthority.ReceiptSha256
$readyPayloadSha256 = [string]$sharedReadyAuthority.ReadyPayloadSha256
$readyPublicReceiptCoreSha256 = [string]$sharedReadyAuthority.PublicReceiptCoreSha256
$readyAckSha256 = [string]$sharedReadyAuthority.ReadyAckSha256
$readySessionIdSha256 = [string]$sharedReadyAuthority.SessionIdSha256
$readyPipeTokenSha256 = [string]$sharedReadyAuthority.PipeTokenSha256
$recorderProcess.Refresh()
if ($recorderProcess.HasExited) {
    throw "collector_record exited with code $($recorderProcess.ExitCode) immediately after publishing its ready receipt."
}
$currentGameProcess = Get-Process -Id $gamePid -ErrorAction SilentlyContinue
if ($null -eq $currentGameProcess) {
    $gameDied = $true
    throw "the game PID $gamePid exited immediately after collector_record published its ready receipt."
}
$currentGameCreationFiletimeUtc = [uint64](
    $currentGameProcess.StartTime.ToUniversalTime().ToFileTimeUtc()
)
if ($currentGameCreationFiletimeUtc -ne $gameCreationFiletimeUtc) {
    throw "the game PID $gamePid creation epoch changed during passive readiness."
}

[void](Write-PassiveWitnessSample `
    -Phase 'ready_validated' `
    -TargetProcessId $gamePid `
    -ExpectedCreationFiletimeUtc $gameCreationFiletimeUtc `
    -ExpectedSpeed $Speed)
$nextPassiveWitnessDeadline = [DateTime]::UtcNow.AddSeconds(
    $WitnessIntervalSeconds
)
Write-Host "phase=injection_ready receipt=$readyReceiptPath ready_sha256=$readyPayloadSha256 policy=zero_client_writes"

# Bound the wait: capture duration plus a startup/finalize margin, EXTENDED for
# as long as post-capture finalization is demonstrably advancing. Hanging here
# forever is what previously left orphaned recorders holding machine-wide
# capture state; treating "slow" as "wedged" is what made t27/t28/t29 unresolved
# with their folds provably part-done. The progress receipt is what separates
# the two, and the ceiling is what keeps the wait bounded either way.
#
# Also watch the game itself. Collector injection can kill the game, and the
# recorder does not always notice promptly -- it sat idle for the whole budget,
# turning a dead attempt into an 8-minute stall. There is nothing left to
# capture once the target is gone, so end the attempt immediately.
$captureProgressDeadline = [DateTime]::UtcNow
while (-not $recorderProcess.HasExited) {
    [void](Update-RecorderFinalizeDeadline `
        -RunDir $OutDir `
        -SessionId $sessionId `
        -CaptureId $captureId `
        -Deadline ([ref]$recorderDeadline) `
        -Ceiling $recorderFinalizeCeiling `
        -NoProgressSeconds $FinalizeNoProgressSeconds `
        -Reason ([ref]$recorderFinalizeExtensionReason))
    if ([DateTime]::UtcNow -ge $recorderDeadline) {
        Write-Host "phase=capture_timeout finalize_bound=$recorderFinalizeExtensionReason"
        throw "collector_record exceeded its $recorderBudgetSeconds s natural duration/teardown budget (finalize progress bound: $(if ($recorderFinalizeExtensionReason) { $recorderFinalizeExtensionReason } else { 'no progress receipt published' })). Post-injection termination is forbidden while the target may still hold its hooks; the attempt remains unresolved until the recorder publishes authenticated teardown."
    }
    if (-not (Get-Process -Id $gamePid -ErrorAction SilentlyContinue)) {
        $gameDied = $true
        Write-Host "phase=game_died pid=$gamePid"
        Write-Warning "the game PID $gamePid exited during capture (collector injection can crash the game); ending this attempt."
        # Give the recorder a short opportunity to finalize naturally. Killing
        # it while the client may still hold its hooks could strand its hook/DLL
        # state there. The cleanup wait may stop it later, but only once that
        # hazard is provably gone.
        if (-not $recorderProcess.WaitForExit(30000)) {
            throw "the game exited and collector_record did not naturally finalize within 30 seconds. Termination is not taken here; the cleanup wait stops it only after proving no hooks can be live."
        }
        break
    }
    if ([DateTime]::UtcNow -ge $nextPassiveWitnessDeadline) {
        # THE COLLECTOR DETACHES LONG BEFORE THE RECORDER EXITS, and this loop
        # runs until the RECORDER exits. The dwell is `requested_duration_ms`
        # (60 s in every live_unlock run); the recorder then spends minutes
        # converting a multi-GB stream into a session and a manifest on this
        # same machine, with the game still running and no collector attached.
        # Labelling those samples `capture` made them the numerator of
        # `clock_baseline_summary.py`'s with-collector rate: in t26, 55 of 65
        # `capture` samples were taken after detach, so the published
        # "with collector 7.17 h/s" was mostly a measurement of THIS SCRIPT's
        # own post-capture conversion competing with the game, not of the
        # collector.
        #
        # CAPTURE_TEARDOWN_V1.json is the recorder's authenticated proof that
        # the hook lease is released and the target's bytes are restored, and
        # the recorder publishes it while it is still alive. Its presence is
        # therefore the exact, evidence-backed boundary between "attached" and
        # "the host is finalizing". Samples after it are still recorded --
        # they are the only measurement of what finalization costs the game --
        # but under their own phase, so no consumer can mistake them for
        # collector cost.
        # `$teardownReceiptPreexisting` is the guard on using existence as
        # evidence at all. This loop cannot AUTHENTICATE the receipt -- that is
        # `Confirm-CollectorTeardownReceipt`, which cannot run until the
        # recorder exits -- so a stale file left in this OutDir by an earlier
        # attempt would label the WHOLE capture `recorder_finalize` and silently
        # empty the with-collector numerator. When one is already present before
        # injection its existence proves nothing, every sample keeps the
        # `capture` label, and `clock_baseline_summary.py` still bounds the
        # window from the dwell, so such a run is reported rather than lost.
        #
        # RESIDUAL, and deliberately left to the summary tool: the check happens
        # before the observation, so the ONE bracket during which the receipt is
        # published is still labelled `capture` while spanning detach. That
        # bracket is excluded downstream by `sample_inside_window`, which drops
        # any sample not wholly inside the dwell -- and the receipt lands at the
        # dwell end. Relabelling it here instead would mean deciding a phase
        # after the row is already appended.
        $witnessPhase = 'capture'
        if ((-not $teardownReceiptPreexisting) -and
            (Test-Path -LiteralPath $collectorTeardownReceiptPath -PathType Leaf)) {
            $witnessPhase = 'recorder_finalize'
        }
        [void](Write-PassiveWitnessSample `
            -Phase $witnessPhase `
            -TargetProcessId $gamePid `
            -ExpectedCreationFiletimeUtc $gameCreationFiletimeUtc `
            -ExpectedSpeed $Speed)
        $nextPassiveWitnessDeadline = [DateTime]::UtcNow.AddSeconds(
            $WitnessIntervalSeconds
        )
    }
    if ([DateTime]::UtcNow -ge $captureProgressDeadline) {
        $captureRemaining = [Math]::Max(
            0,
            [Math]::Ceiling(($recorderDeadline - [DateTime]::UtcNow).TotalSeconds)
        )
        Write-Host "phase=capture_wait recorder_pid=$($recorderProcess.Id) game_pid=$gamePid remaining_budget_s=$captureRemaining"
        $captureProgressDeadline = [DateTime]::UtcNow.AddSeconds(10)
    }
    Start-Sleep -Seconds 2
}
if (-not $recorderProcess.HasExited) {
    throw 'collector_record remained live after its bounded natural capture window; termination is not taken here. The cleanup wait stops it only once it can prove the recorder holds no hooks in a live game.'
}
$recorderExit = $recorderProcess.ExitCode
Write-Host "phase=capture_done exit=$recorderExit"

# Keep the graded window open until the recorder proves that its hook lease is
# released and the collector DLL is unloaded. Waiting is bounded and remains
# read-only toward HOI4.
$teardownDeadline = [DateTime]::UtcNow.AddSeconds(30)
$nextTeardownWitnessDeadline = [DateTime]::UtcNow
while (-not (Test-Path -LiteralPath $collectorTeardownReceiptPath -PathType Leaf) -and
    [DateTime]::UtcNow -lt $teardownDeadline) {
    if ($null -ne (Get-Process -Id $gamePid -ErrorAction SilentlyContinue) -and
        [DateTime]::UtcNow -ge $nextTeardownWitnessDeadline) {
        [void](Write-PassiveWitnessSample `
            -Phase 'teardown_wait' `
            -TargetProcessId $gamePid `
            -ExpectedCreationFiletimeUtc $gameCreationFiletimeUtc `
            -ExpectedSpeed $Speed)
        $nextTeardownWitnessDeadline = [DateTime]::UtcNow.AddSeconds(
            $WitnessIntervalSeconds
        )
    }
    Start-Sleep -Milliseconds 250
}
if (-not (Test-Path -LiteralPath $collectorTeardownReceiptPath -PathType Leaf)) {
    throw 'collector_record exited without publishing an authenticated teardown receipt within 30 s.'
}
[void](Confirm-CollectorTeardownReceipt)
$collectorTeardownAuthenticated = $true
# The post-teardown observation is EVIDENCE, and it must not be able to hold the
# window open. Write-PassiveWitnessSample asks the verdict authority, which
# throws on a stall -- and a throw here jumps past `$gradedWindowStarted =
# $false` with the teardown already authenticated, so the cleanup branch that
# closes the window is skipped (it only fires when teardown is NOT
# authenticated) and the settings guard is never restored. What closes this
# window is the teardown record, which has just been admitted; a slow frame
# afterwards is not a hook still being installed.
if ($null -ne (Get-Process -Id $gamePid -ErrorAction SilentlyContinue)) {
    try {
        [void](Write-PassiveWitnessSample `
            -Phase 'post_teardown' `
            -TargetProcessId $gamePid `
            -ExpectedCreationFiletimeUtc $gameCreationFiletimeUtc `
            -ExpectedSpeed $Speed)
    } catch {
        Write-Host "phase=post_teardown_witness_skipped reason=$(($_.Exception.Message -replace '\s+', ' ').Trim())"
    }
}
$gradedWindowStarted = $false
Write-Host "phase=teardown_authenticated pid=$gamePid receipt=$collectorTeardownReceiptPath"

# --- Phase 4: read the outputs directly -------------------------------------
Write-Host ''
Get-ChildItem -LiteralPath $OutDir -Force | Sort-Object Length -Descending |
    Select-Object Name, Length | Format-Table -AutoSize | Out-String | Write-Host

# An attempt whose game died mid-capture legitimately has no manifest. Reading
# the outputs is reporting, not a gate: it must never turn a failed attempt into
# a terminating error for whatever is driving this script.
$manifestPath = Join-Path $OutDir 'native_session_manifest.json'
$haveManifest = Test-Path -LiteralPath $manifestPath -PathType Leaf
$haveStreamReport = Test-Path -LiteralPath $streamReport -PathType Leaf
if ($haveManifest -and -not $haveStreamReport) {
    # A finalized capture with no reporter is an UNASKED question, and the two
    # conditions are tested apart precisely so this case cannot hide inside a
    # combined `-and` and fall through to the "no manifest" branch below.
    $undefinedStatusBitFailure =
        "the raw-capture report is missing ($streamReport), so whether this capture " +
        "published an undefined status bit was never asked."
}
if ($haveStreamReport -and $haveManifest) {
    try {
        $reportResult = Invoke-BoundedNativeCapture `
            -FilePath $pythonExe `
            -Arguments @($streamReport, '--run-dir', $OutDir) `
            -TimeoutSeconds 60 `
            -Purpose 'raw-capture report' `
            -AllowNonzeroExit
        if (-not [string]::IsNullOrWhiteSpace($reportResult.Stdout)) {
            Write-Host $reportResult.Stdout.TrimEnd()
        }
        if (-not [string]::IsNullOrWhiteSpace($reportResult.Stderr)) {
            Write-Host $reportResult.Stderr.TrimEnd()
        }
        if ($reportResult.ExitCode -ne 0) {
            Write-Host "report failed: exit code $($reportResult.ExitCode)"
        }
        # Exit code 2 is report_raw_capture.py's UNDEFINED-STATUS-BIT class and
        # it is the one report verdict this harness refuses to walk past. Every
        # other non-zero exit is an ordinary residual failure (a PARTIAL_DATA
        # hour, a silent domain): classifiable outcomes that a graded capture is
        # allowed to carry and that the operator weighs. A status bit no
        # contract defines is not weighable -- the replay fold, the gate and the
        # promotion check all read that same word and none of them can say what
        # it means, so the capture's status is not merely bad, it is unreadable.
        # Kept OUT of $scriptExitCode's normal recorder-exit path deliberately:
        # this must surface even when the recorder itself exited 0.
        #
        # The exit code is only believed when the report also printed its
        # verdict receipt. A SyntaxError, a failed import or a missing
        # interpreter kills python BEFORE any of the report's own code runs and
        # exits 1 -- which is the report's code for an ANSWER, and this harness
        # walks past answers. The receipt is printed only by paths that reached
        # a verdict, so its ABSENCE means the check did not run.
        # STDERR: `--json` puts one complete JSON document on stdout and a
        # caller pipes it straight into a parser, so the receipt cannot live
        # there.
        $verdictReceipt = $null
        if (-not [string]::IsNullOrWhiteSpace($reportResult.Stderr)) {
            $verdictReceipt = @(
                $reportResult.Stderr -split "`n" |
                    ForEach-Object { $_.Trim() } |
                    Where-Object { $_ -like 'COMPACT_STATUS_VERDICT=*' }
            ) | Select-Object -Last 1
        }
        if ($null -eq $verdictReceipt) {
            $undefinedStatusBitFailure =
                "the raw-capture report exited $($reportResult.ExitCode) without printing its " +
                "verdict receipt (COMPACT_STATUS_VERDICT=), so its own code never ran and " +
                "whether this capture published an undefined status bit is UNKNOWN."
        } elseif ($verdictReceipt -ne "COMPACT_STATUS_VERDICT=$($reportResult.ExitCode)") {
            $undefinedStatusBitFailure =
                "the raw-capture report exited $($reportResult.ExitCode) but its receipt says " +
                "'$verdictReceipt'; the two disagree, so its verdict is UNKNOWN."
        } elseif ($reportResult.ExitCode -eq 2) {
            $undefinedStatusBitFailure =
                "capture published status bits no compact-stream contract defines; see the " +
                "CONTRACT VIOLATION section above. Declare the bit in compact_contract " +
                "(and register it in lane_local_status_mask) or stop the producer writing it."
        } elseif ($reportResult.ExitCode -eq 0 -or $reportResult.ExitCode -eq 1) {
            # The ONLY place the verdict is CLEARED. Exit 0 and 1 are the
            # report's two answers ("clean" and "blocking domains"), both backed
            # by a matching receipt, so the definedness check demonstrably ran
            # and said no undefined bits. Every other route leaves the standing
            # verdict set, which is the point: it defaults to blocking from the
            # moment the capture starts and only an ANSWER retires it.
            $undefinedStatusBitFailure = $null
        } else {
            # Exit 3 is the report's own "I reached NO verdict" code (it crashed,
            # or could not read the run), and anything outside its vocabulary
            # means the same thing.
            $undefinedStatusBitFailure =
                "the raw-capture report exited $($reportResult.ExitCode) and reached no verdict " +
                "(0 clean / 1 blocking / 2 undefined status bits / 3 report error), so whether " +
                "this capture published an undefined status bit is UNKNOWN."
        }
    } catch {
        Write-Host "report failed: $($_.Exception.Message)"
        # Same reason: a timed-out or unlaunchable report is an unanswered
        # question, not an answer. Invoke-BoundedNativeCapture THROWS on
        # timeout, so without this the 60 s bound silently skipped the check.
        $undefinedStatusBitFailure =
            "the raw-capture report did not complete ($($_.Exception.Message)), so whether " +
            "this capture published an undefined status bit is UNKNOWN."
    }
} elseif (-not $haveStreamReport) {
    Write-Host "raw-capture report not found: $streamReport"
} elseif ($gameDied) {
    # The one case where the absence is EXPLAINED: the game died, so there is no
    # capture to grade and no status word to read. The death is its own signal,
    # and it is the only route other than an actual report ANSWER that retires
    # the standing verdict.
    Write-Host 'no manifest: the game died during capture, so there is nothing to finalize'
    $undefinedStatusBitFailure = $null
} else {
    # The game lived and the recorder still produced no manifest. Nothing was
    # graded, so nothing may be reported as graded-and-clean.
    Write-Host "no manifest under $OutDir"
    $undefinedStatusBitFailure =
        "the capture produced no manifest under $OutDir and the game did not die, so " +
        "whether it published an undefined status bit was never asked."
}

Write-Host "out_dir=$OutDir"
Write-Host "recorder_exit=$recorderExit"
Write-Host "game_died=$gameDied"
if ($null -eq $recorderExit) {
    throw 'collector_record did not expose a process exit code.'
}
$captureSucceeded = ([int]$recorderExit -eq 0 -and -not $gameDied)
$scriptExitCode = [int]$recorderExit
} catch {
    # RECORD, then rethrow unchanged. This block exists so cleanup can name the
    # reason the run stopped next to the artifacts that stop with it -- the
    # witness stream above all, whose truncation was twice read as an
    # instrument failure. A bare `throw` inside `catch` rethrows the current
    # exception, so every route out of this script is exactly what it was.
    $harnessAbortReason = ($_.Exception.Message -replace '\s+', ' ').Trim()
    # THE FACT, KEPT SEPARATELY FROM THE TEXT. The refusal arbitration has to
    # know that an abort is unwinding so it does not return a code and suppress
    # it, and it was asking the REASON STRING -- which is empty whenever the
    # exception carries no message. `throw [InvalidOperationException]::new('')`
    # is enough: the reason trims to '', the arbitration reads "no abort", and
    # `exit 3` overwrites a pending exception. A boolean cannot be emptied by a
    # silent exception.
    $harnessAborted = $true
    # Best effort at saying something useful even so. An abort reported as a
    # blank is the unnamed strand this lane exists to remove, one layer up.
    if ([string]::IsNullOrWhiteSpace($harnessAbortReason)) {
        $harnessAbortReason = 'the attempt aborted without a message'
        try {
            $harnessAbortReason =
                "the attempt aborted with a message-less $($_.Exception.GetType().FullName)"
        } catch { }
    }
    throw
} finally {
    $cleanupFatalMessage = $null
    # The whole cleanup body is wrapped so the compact-status arbitration point
    # can sit in an INNER `finally`. Being last in the outer `finally` was not
    # enough: $ErrorActionPreference is 'Stop' for this script, so any failing
    # cmdlet in the cleanup prefix below (a Test-Path on a vanished drive, a
    # SetEnvironmentVariable denial) is TERMINATING and abandons the rest of the
    # outer `finally` -- including the arbitration -- leaving exit 1. The inner
    # `finally` is the only construct that still runs in that case.
    try {
    # Never terminate collector_record while its hooks may still be live in the
    # client. Its configured duration is the cooperative stop authority, and
    # hard termination can orphan hooks or the injected DLL before authenticated
    # teardown is published. The ONE exception, taken at the end of this wait
    # and nowhere else, is a recorder that has outlived its budget after that
    # hazard is provably gone -- see the branch that reasons about it.
    if ($null -ne $recorderProcess) {
        try {
            $recorderProcess.Refresh()
            $nextPassiveCleanupNotice = [DateTime]::UtcNow
            # Consult the receipt ONCE before the loop condition is first
            # evaluated. Entering cleanup a moment after the current deadline
            # would otherwise skip the loop entirely and never read a receipt
            # that justified going on -- the extension would be lost to the
            # instant the run happened to arrive here.
            [void](Update-RecorderFinalizeDeadline `
                -RunDir $OutDir `
                -SessionId $sessionId `
                -CaptureId $captureId `
                -Deadline ([ref]$recorderDeadline) `
                -Ceiling $recorderFinalizeCeiling `
                -NoProgressSeconds $FinalizeNoProgressSeconds `
                -Reason ([ref]$recorderFinalizeExtensionReason))
            # The progress line used to name only the PID, so the two ways this
            # loop can end -- the recorder finished, or the budget ran out --
            # looked identical from the transcript and a run that waited its
            # full budget and then some was read as one that gave up early.
            # Print what the capture wait prints: how much budget is left.
            # This loop is where a run that threw upstream still gives the fold
            # its chance to land, so it extends on the same published progress
            # the capture wait uses. Without that, an attempt that reached here
            # with minutes of fold left is abandoned on the same fixed clock.
            while (-not $recorderProcess.HasExited -and
                $null -ne $recorderDeadline -and
                [DateTime]::UtcNow -lt $recorderDeadline) {
                [void](Update-RecorderFinalizeDeadline `
                    -RunDir $OutDir `
                    -SessionId $sessionId `
                    -CaptureId $captureId `
                    -Deadline ([ref]$recorderDeadline) `
                    -Ceiling $recorderFinalizeCeiling `
                    -NoProgressSeconds $FinalizeNoProgressSeconds `
                    -Reason ([ref]$recorderFinalizeExtensionReason))
                if ([DateTime]::UtcNow -ge $nextPassiveCleanupNotice) {
                    $finalizeRemaining = [Math]::Max(
                        0,
                        [Math]::Ceiling(($recorderDeadline - [DateTime]::UtcNow).TotalSeconds)
                    )
                    Write-Host "phase=passive_recorder_finalize_wait recorder_pid=$($recorderProcess.Id) remaining_budget_s=$finalizeRemaining finalize_bound=$recorderFinalizeExtensionReason"
                    $nextPassiveCleanupNotice = [DateTime]::UtcNow.AddSeconds(10)
                }
                [void]$recorderProcess.WaitForExit(500)
                $recorderProcess.Refresh()
            }
            if (-not $recorderProcess.HasExited) {
                # THE TRUTHFUL CAUSE, read now rather than inherited from the
                # last successful extension. See Get-RecorderFinalizeTerminalReason.
                $recorderFinalizeTerminalReason = Get-RecorderFinalizeTerminalReason `
                    -RunDir $OutDir `
                    -SessionId $sessionId `
                    -CaptureId $captureId `
                    -Deadline $recorderDeadline `
                    -Ceiling $recorderFinalizeCeiling `
                    -NoProgressSeconds $FinalizeNoProgressSeconds
                Write-Host "phase=passive_recorder_finalize_wait_end reason=budget_exhausted recorder_pid=$($recorderProcess.Id) budget_s=$recorderBudgetSeconds finalize_bound=$recorderFinalizeExtensionReason terminal_reason=$recorderFinalizeTerminalReason"
                # "NEVER TERMINATE AFTER INJECTION" PROTECTS A LIVE TARGET, AND
                # ONLY A LIVE TARGET.
                #
                # The rule above this loop states its own reason: hard
                # termination "can orphan hooks or the injected DLL before
                # authenticated teardown is published". Both hazards live inside
                # hoi4.exe. When the process the collector attached to no longer
                # exists, there is nothing left to orphan -- its hooks, its
                # patched bytes and the injected DLL went with its address
                # space -- and the rule is protecting nobody. What is left is a
                # recorder that has outlived its supervision, which is exactly
                # what t60 was: the harness warned, returned, and left a
                # recorder folding unwatched for fifty more minutes until it
                # filled the system disk.
                #
                # So: only when the target is INDEPENDENTLY verifiable as gone
                # (its pid absent, or answering with a different creation time
                # and therefore a different process) is the recorder stopped,
                # and the stop is recorded as a named supervision failure rather
                # than performed quietly. A live or undetermined target keeps
                # exactly the behaviour it had.
                #
                # There are TWO ways to know nothing is left to orphan, and
                # either is enough (Codex review round 3, finding D2): the target
                # epoch is gone, or teardown was already AUTHENTICATED for this
                # run -- CAPTURE_TEARDOWN_V1.json asserts the data pipe reached
                # EOF, the hook lease was released, and the pinned hook sites and
                # target code were restored. This never re-authenticates the
                # receipt here; it reads only the flag an earlier, throwing
                # verification already set, so a run that never proved teardown
                # keeps exactly the behaviour it had.
                $targetGoneDetail = ''
                $targetGone = Test-TargetProcessGone `
                    -TargetProcessId $gamePid `
                    -ExpectedCreationFiletimeUtc $gameCreationFiletimeUtc `
                    -Detail ([ref]$targetGoneDetail)
                $hooksProvenReleased = $targetGone -or $collectorTeardownAuthenticated
                if ($hooksProvenReleased) {
                    $releaseProof = if ($targetGone) {
                        "its target the game PID $gamePid was already gone ($targetGoneDetail)"
                    } else {
                        "teardown was authenticated for this run, so its hook sites and the target's own code were already restored"
                    }
                    $terminationFailure =
                        "collector_record PID $($recorderProcess.Id) outlived its finalize budget " +
                        "($recorderFinalizeTerminalReason) and $releaseProof. The harness stopped it: a " +
                        "recorder holding no hooks and no injected DLL in a live game leaves the " +
                        "post-injection termination rule nothing to protect, and leaving it unsupervised " +
                        "is how a run fills the system disk. This attempt is REFUSED."
                    Write-Host "phase=passive_recorder_terminated recorder_pid=$($recorderProcess.Id) target_pid=$gamePid target_state=$targetGoneDetail teardown_authenticated=$collectorTeardownAuthenticated terminal_reason=$recorderFinalizeTerminalReason"
                    try {
                        # THE EXISTING EXACT-PROCESS STOP, not a bare Kill().
                        # It re-checks HasExited first (the recorder can exit in
                        # the window between the decision and the stop, and a
                        # Kill on an exited handle throws), and it THROWS when
                        # the process does not actually go away -- which a
                        # discarded WaitForExit would report as a clean stop on
                        # a recorder that is still running and still writing.
                        # (Codex review round 3, finding D1.)
                        Stop-ExactProcess `
                            -Process $recorderProcess `
                            -Purpose 'collector_record whose target is gone' `
                            -WaitMilliseconds 30000
                        $recorderProcess.Refresh()
                    } catch {
                        $terminationFailure +=
                            " The stop itself failed: $(($_.Exception.Message -replace '\s+', ' ').Trim())"
                    }
                    [void]$supervisionFailures.Add($terminationFailure)
                    Write-SettingsGuardNonThrowingWarning -Message $terminationFailure
                } else {
                    Write-SettingsGuardNonThrowingWarning -Message "collector_record PID $($recorderProcess.Id) remains live after its bounded natural-stop budget ($recorderFinalizeTerminalReason), its target the game PID $gamePid is $targetGoneDetail, and teardown is not authenticated. The harness will not terminate it after injection; this attempt is unresolved until CAPTURE_TEARDOWN_V1.json authenticates cleanup."
                }
            } else {
                Write-Host "phase=passive_recorder_finalize_wait_end reason=recorder_exited recorder_pid=$($recorderProcess.Id)"
                # The teardown record is published before the fold, so by the
                # time the recorder exits it is normally long since on disk.
                # "Normally" is not a guarantee, and the admission below is a
                # bare Test-Path: a receipt still being flushed at that instant
                # would be treated exactly like one that was never written, and
                # strand the window for a run that had proved its hooks gone.
                # Wait for it, bounded by whatever budget is left.
                $finalizeReceiptDeadline = [DateTime]::UtcNow.AddSeconds(30)
                if ($null -ne $recorderDeadline -and
                    $finalizeReceiptDeadline -gt $recorderDeadline) {
                    $finalizeReceiptDeadline = $recorderDeadline
                }
                while (-not (Test-Path -LiteralPath $collectorTeardownReceiptPath -PathType Leaf) -and
                    [DateTime]::UtcNow -lt $finalizeReceiptDeadline) {
                    Start-Sleep -Milliseconds 250
                }
            }
        } catch {
            $cleanupFatalMessage = "Could not observe recorder PID $($recorderProcess.Id) during passive cleanup: $($_.Exception.Message)"
            Write-SettingsGuardNonThrowingWarning -Message $cleanupFatalMessage
        }
    }
    if ($collectorInjectionStarted -and
        -not $collectorTeardownAuthenticated -and
        (Test-Path -LiteralPath $collectorTeardownReceiptPath -PathType Leaf)) {
        try {
            [void](Confirm-CollectorTeardownReceipt)
            $collectorTeardownAuthenticated = $true
            # Same reason as the main path above: an evidence observation may
            # not be the thing that decides whether the window closes. Here it
            # is worse, because this whole block is the last chance to close it.
            if ($null -ne (Get-Process -Id $gamePid -ErrorAction SilentlyContinue)) {
                try {
                    [void](Write-PassiveWitnessSample `
                        -Phase 'post_teardown' `
                        -TargetProcessId $gamePid `
                        -ExpectedCreationFiletimeUtc $gameCreationFiletimeUtc `
                        -ExpectedSpeed $Speed)
                } catch {
                    Write-Host "phase=cleanup_post_teardown_witness_skipped reason=$(($_.Exception.Message -replace '\s+', ' ').Trim())"
                }
            }
            $gradedWindowStarted = $false
            Write-Host "phase=cleanup_teardown_authenticated pid=$gamePid receipt=$collectorTeardownReceiptPath"
        } catch {
            Write-SettingsGuardNonThrowingWarning -Message "Collector teardown receipt was rejected; reused game state will not be changed: $($_.Exception.Message)"
        }
    }
    # The refusal path's own exit. Teardown was never authenticated -- the
    # attempt stays refused and still returns nonzero -- but hook custody is
    # settled, so the window that forbids touching the client is closed and the
    # cleanup below runs: a game this harness launched is closed when the
    # harness is finished with it. A game it did NOT launch is still never
    # touched; $launchedProcess is null for those.
    if ($collectorInjectionStarted -and
        $gradedWindowStarted -and
        -not $collectorTeardownAuthenticated) {
        $hookReleaseProof = ''
        $targetGoneDetail = ''
        # NON-THROWING. This runs in the cleanup prefix, where every failing
        # cmdlet is terminating and a throw abandons the settings-guard restore
        # and the refusal receipt below it. An unreadable proof holds the window.
        try {
            if (Test-TargetProcessGone `
                    -TargetProcessId $gamePid `
                    -ExpectedCreationFiletimeUtc $gameCreationFiletimeUtc `
                    -Detail ([ref]$targetGoneDetail)) {
                $gradedWindowStarted = $false
                Write-Host "phase=graded_window_closed_target_gone pid=$gamePid target_state=$targetGoneDetail"
            } elseif (Test-CollectorHookReleaseProof `
                    -RunDir $OutDir `
                    -RecorderProcess $recorderProcess `
                    -HookAuthority (Get-PassivePinnedHookAllowlistAuthority) `
                    -ProofOut ([ref]$hookReleaseProof)) {
                $gradedWindowStarted = $false
                Write-Host "phase=graded_window_closed_on_hook_restoration_receipt pid=$gamePid $hookReleaseProof"
            } else {
                Write-Host "phase=graded_window_held_open pid=$gamePid reason=$hookReleaseProof"
            }
        } catch {
            Write-Host "phase=graded_window_held_open pid=$gamePid reason=$(($_.Exception.Message -replace '\s+', ' ').Trim())"
        }
    }
    if ($reuseRunningGame -and
        $collectorInjectionStarted -and
        -not $collectorTeardownAuthenticated -and
        $null -ne (Get-Process -Id $gamePid -ErrorAction SilentlyContinue)) {
        Write-SettingsGuardNonThrowingWarning -Message "Reused the game PID $gamePid survived capture without an authenticated teardown receipt. The harness made no client-state change and will not stop or otherwise drive the user-owned process."
    }

    # --- what the witness stream means, on EVERY route out ---------------------
    #
    # PASSIVE_CLOCK_INTERVALS_V2.json is written by the verdict authority, and
    # the harness only invokes that authority when it takes a sample. So a run
    # that stopped between two samples leaves the intervals file frozen at the
    # last verdict, and the artifact then reads as though the clock witness
    # itself had died: t20's "samples=5, wall_clock_ms=23934" was diagnosed that
    # way twice. Nothing died. passive_clock_witness.exe is a one-shot reader of
    # the whole witness JSONL, not a daemon; it stops being run when the harness
    # stops taking samples, and t20 stopped taking samples because it refused a
    # READY receipt at 23.9 s.
    #
    # Two things fix that permanently: regenerate the report over every sample
    # the run actually took, and SAY why the stream ends there. Both are
    # reporting, so neither may change what the harness does -- the tool runs
    # with -AllowNonzeroExit and the whole block is non-throwing.
    $witnessStopReason = $harnessAbortReason
    if ([string]::IsNullOrWhiteSpace($witnessStopReason)) {
        $witnessStopReason = 'run_completed'
    }
    Write-Host (
        "phase=passive_witness_stopped samples=$passiveWitnessSampleCount " +
        "last_phase=$passiveWitnessLastPhase reason=$witnessStopReason"
    )
    if ($passiveWitnessSampleCount -gt 0 -and
        (Test-Path -LiteralPath $passiveWitnessPath -PathType Leaf) -and
        (Test-Path -LiteralPath $passiveClockWitnessTool -PathType Leaf)) {
        try {
            $finalIntervalsResult = Invoke-BoundedNativeCapture `
                -FilePath $passiveClockWitnessTool `
                -Arguments @(
                    '--witness', $passiveWitnessPath,
                    '--intervals-out', $passiveClockIntervalsPath,
                    '--stall-samples', "$StallEscalationSamples",
                    '--stall-span-ms', "$($StallEscalationSeconds * 1000)"
                ) `
                -TimeoutSeconds 30 `
                -Purpose 'passive witness final interval report' `
                -AllowNonzeroExit
            if (-not [string]::IsNullOrWhiteSpace($finalIntervalsResult.Stdout)) {
                Write-Host $finalIntervalsResult.Stdout.Trim()
            }
            if (-not [string]::IsNullOrWhiteSpace($finalIntervalsResult.Stderr)) {
                Write-Host $finalIntervalsResult.Stderr.Trim()
            }
            Write-Host "phase=passive_witness_intervals_final exit=$([int]$finalIntervalsResult.ExitCode) path=$passiveClockIntervalsPath"
        } catch {
            Write-Host "phase=passive_witness_intervals_final_failed reason=$(($_.Exception.Message -replace '\s+', ' ').Trim())"
        }
    }

    # --- the free-run denominator, printed rather than hand-derived ------------
    #
    # Loss is with_collector / free_run, and the free-run term is the paired
    # settle control this run already recorded in settle_control.jsonl -- same
    # process, same save, same speed, no collector. It was written and then left
    # for an operator to aggregate by hand against the witness, which is how
    # rates got quoted with no denominator at all. Compute both here, on every
    # route out, including the routes where the capture was refused: a refused
    # capture still measured a free-run baseline, and that is exactly the number
    # the next attempt is planned from.
    if (Test-Path -LiteralPath $clockBaselineSummary -PathType Leaf) {
        try {
            $baselineResult = Invoke-BoundedNativeCapture `
                -FilePath $pythonExe `
                -Arguments @($clockBaselineSummary, '--run-dir', $OutDir) `
                -TimeoutSeconds 60 `
                -Purpose 'clock baseline summary' `
                -AllowNonzeroExit
            if (-not [string]::IsNullOrWhiteSpace($baselineResult.Stdout)) {
                Write-Host $baselineResult.Stdout.TrimEnd()
            }
            if (-not [string]::IsNullOrWhiteSpace($baselineResult.Stderr)) {
                Write-Host $baselineResult.Stderr.TrimEnd()
            }
        } catch {
            Write-Host "phase=clock_baseline_failed reason=$(($_.Exception.Message -replace '\s+', ' ').Trim())"
        }
    }

    # A failure between the graded boundary and the recorder starting leaves the
    # window open with nothing in it: no recorder, no injection, no hook custody
    # to authenticate. Holding it open then serves no purpose and does real
    # harm, because it suppresses the token restore below and a game this
    # harness launched can no longer be stopped. Close it explicitly.
    if ($gradedWindowStarted -and -not $collectorInjectionStarted) {
        $gradedWindowStarted = $false
        Write-Host 'phase=graded_window_closed_without_injection'
    }
    # Give the evidence-mode client-driving authority back, but only now: this
    # sits after every assignment above that can close the graded window --
    # including the late teardown authentication -- and immediately before the
    # one cleanup step that needs it. Restoring at the top of this block would
    # run while a late-authenticating window was still nominally open, and then
    # never run again. The token is only ever restored to what it held before.
    if (-not $gradedWindowStarted -and
        $null -ne $clientDrivingOptInSuspended) {
        [Environment]::SetEnvironmentVariable(
            'BATTLE_PLAN_CLIENT_DRIVING_OPT_IN',
            $clientDrivingOptInSuspended,
            [EnvironmentVariableTarget]::Process
        )
        $clientDrivingOptInSuspended = $null
    }
    if ($null -ne $launchedProcess -and
        (-not $captureSucceeded -or -not $KeepGameAfterCapture) -and
        (-not $collectorInjectionStarted -or -not $gradedWindowStarted)) {
        try {
            Assert-Hoi4ClientDrivingOptIn -Caller $PSCommandPath
            $launchedProcess.Refresh()
            if (-not $launchedProcess.HasExited) {
                try { $launchedProcess.CloseMainWindow() | Out-Null } catch { }
                if (-not $launchedProcess.WaitForExit(20000)) {
                    Stop-ExactProcess -Process $launchedProcess -Purpose 'harness-launched the game'
                }
                Write-Host "phase=cleanup_game pid=$gamePid"
            }
        } catch {
            $cleanupFatalMessage = "Could not clean up launched the game PID ${gamePid}: $($_.Exception.Message)"
            Write-SettingsGuardNonThrowingWarning -Message $cleanupFatalMessage
        }
    } elseif ($null -ne $launchedProcess -and
        $collectorInjectionStarted -and
        $gradedWindowStarted) {
        Write-SettingsGuardNonThrowingWarning -Message "Harness-launched the game PID $gamePid remains running because hook custody is unresolved: neither an authenticated teardown receipt nor a complete hook-restoration receipt. Stopping it inside the graded window is forbidden."
    }

    # Give the game its CPU affinity back.
    #
    # The passive default reuses a game the operator started and will keep
    # using, and leaving it pinned to four cores forever is a change this
    # harness made and nobody asked for. Gated on the graded window being closed
    # for the same reason the settings guard is: touching the client while hook
    # custody is unresolved is forbidden, and a message an operator can act on
    # is better than a mask silently left behind.
    #
    # Best-effort and non-throwing. A restore diagnostic must never replace the
    # capture's own verdict, and the run's data is already on disk by here.
    # A game this harness launched has normally already been stopped by the
    # block above, and a mask on a process that no longer exists is nothing to
    # restore. Liveness is checked first so that ordinary case stays silent
    # instead of reporting a restore failure that means only "it exited".
    if ($null -ne $gameAffinityPrior -and
        $gamePid -gt 0 -and
        $null -ne (Get-Process -Id $gamePid -ErrorAction SilentlyContinue)) {
        if ($gradedWindowStarted) {
            Write-SettingsGuardNonThrowingWarning -Message "the game PID $gamePid remains pinned to CPU affinity $(Format-Hoi4AffinityMask -Mask $cpuAffinityPlan.Mask) because the passive graded window did not close. After independently proving the hooks are gone, restore it to $(Format-Hoi4AffinityMask -Mask $gameAffinityPrior) or restart the game."
        } else {
            try {
                [void](Set-Hoi4ProcessAffinity `
                    -ProcessId $gamePid `
                    -Mask $gameAffinityPrior `
                    -Label 'hoi4_affinity_restore' `
                    -ExpectedCreationFiletimeUtc $gameCreationFiletimeUtc `
                    -PriorMaskOut $null)
                $gameAffinityPrior = $null
            } catch {
                Write-SettingsGuardNonThrowingWarning -Message "Could not restore the game PID $gamePid to CPU affinity $(Format-Hoi4AffinityMask -Mask $gameAffinityPrior). Set it by hand or restart the game. Error: $($_.Exception.Message)"
            }
        }
    }

    $settingsGuardRestoreSucceeded = $false
    if ($settingsGuardRestoreRequired -and -not $gradedWindowStarted) {
        try {
            $settingsRestoreItem = Get-Item -LiteralPath $settingsPath -Force
            $settingsRestoreItem.IsReadOnly = $settingsGuardPriorReadOnly
            $settingsRestoreItem.Refresh()
            if ([bool]$settingsRestoreItem.IsReadOnly -ne $settingsGuardPriorReadOnly) {
                throw "IsReadOnly remained $($settingsRestoreItem.IsReadOnly)"
            }
            $settingsGuardRestoreSucceeded = $true
        } catch {
            Write-SettingsGuardNonThrowingWarning -Message "SETTINGS GUARD RESTORE FAILED for '$settingsPath'. Manually set IsReadOnly=$settingsGuardPriorReadOnly, then remove sentinel '$settingsGuardSentinel'. The sentinel was retained for recovery. Error: $($_.Exception.Message)"
        }
    } elseif ($settingsGuardRestoreRequired -and $gradedWindowStarted) {
        Write-SettingsGuardNonThrowingWarning -Message "Settings guard remains applied to '$settingsPath' because hook custody is unresolved: neither an authenticated teardown receipt nor a complete hook-restoration receipt. Restoring launcher/user settings inside the graded window is forbidden. After independently proving hooks are gone, set IsReadOnly=$settingsGuardPriorReadOnly and remove sentinel '$settingsGuardSentinel'."
    }

    $settingsGuardOwnerStreamClosed = $true
    if ($null -ne $settingsGuardOwnerStream) {
        try {
            $settingsGuardOwnerStream.Dispose()
            $settingsGuardOwnerStream = $null
        } catch {
            $settingsGuardOwnerStreamClosed = $false
            Write-SettingsGuardNonThrowingWarning -Message "Settings guard restored=$settingsGuardRestoreSucceeded for '$settingsPath', but could not close its live-owner handle for sentinel '$settingsGuardSentinel'. The sentinel was retained. Error: $($_.Exception.Message)"
        }
    }

    if ($settingsGuardRestoreRequired -and
        $settingsGuardRestoreSucceeded -and
        $settingsGuardOwnerStreamClosed) {
        Write-Host "phase=settings_guard action=release prior_read_only=$settingsGuardPriorReadOnly restored_read_only=$settingsGuardPriorReadOnly path=$settingsPath sentinel=$settingsGuardSentinel"
        try {
            Remove-Item -LiteralPath $settingsGuardSentinel -Force
        } catch {
            Write-SettingsGuardNonThrowingWarning -Message "Settings guard restored IsReadOnly=$settingsGuardPriorReadOnly on '$settingsPath', but could not remove sentinel '$settingsGuardSentinel'. Remove it manually after verifying the attribute. Error: $($_.Exception.Message)"
        }
    }

    # --- what this attempt refused, on EVERY route out -------------------------
    #
    # Last in the cleanup prefix so it reports what the whole prefix ended up
    # doing, and so a supervision failure raised anywhere above is still counted.
    # The receipt is rewritten here rather than only at the moment of refusal,
    # because a refusal recorded during the capture phase and a run that then
    # went on to seal normally must leave ONE file describing both.
    if ($captureRefusals.Count -gt 0 -or $supervisionFailures.Count -gt 0) {
        Write-Host (
            "phase=capture_refusals capture=$($captureRefusals.Count) " +
            "supervision=$($supervisionFailures.Count) " +
            "receipt=$captureRefusalReceiptPath"
        )
        foreach ($supervisionFailure in $supervisionFailures) {
            Write-Host "phase=supervision_failed detail=$supervisionFailure"
        }
        Write-CaptureRefusalReceipt
    }

    # THE LAST STATEMENT OF THE CLEANUP PREFIX, and it must stay last. Reaching
    # it is the only evidence available inside the arbitration below that the
    # prefix did not end on a terminating error, and the refusal code declines
    # to fire without it rather than swallow whatever is unwinding.
    $cleanupPrefixCompleted = $true
    } finally {
    # --- the single compact-status arbitration point -------------------------
    #
    # There must be exactly one of these, and it must be the body of the INNER
    # `finally` that wraps the whole cleanup prefix. That nesting is what makes
    # it unconditional. Every other placement loses the verdict on some route:
    #
    #   * after the outer `finally`: a `throw` from the try body (an
    #     unauthenticated teardown, a null recorder exit code, any bug) resumes
    #     there and skips it, returning 1;
    #   * last inside the outer `finally`: any TERMINATING error in the cleanup
    #     prefix above -- and every failing cmdlet is terminating here, because
    #     $ErrorActionPreference is 'Stop' -- abandons the rest of that block,
    #     including this one;
    #   * before the cleanup: the capture would not yet be finalized, receipted
    #     and left on disk, and that artifact is exactly what someone has to
    #     read to fix the producer.
    #
    # `exit` from `finally` beats a pending exception in PowerShell 5.1 -- that
    # is measured, not assumed (`the_harness_status_verdict_survives_every_
    # route_out` runs this block with an exception already in flight, and with
    # one thrown by the cleanup prefix).
    #
    # Exit code 2 wins over every co-occurring failure, and every one of them is
    # still PRINTED. That is a decision, not an accident: each of the others
    # reports through exit code 1 and so has no machine-readable code to lose,
    # while 2 exists precisely so a caller can key on it and is gone forever if
    # anything else exits first.
    if (-not [string]::IsNullOrWhiteSpace($undefinedStatusBitFailure)) {
        # The DIAGNOSTICS are wrapped and the `exit 2` is in their `finally`, so
        # the code is delivered even if reporting it is not. Every write below
        # can throw -- a closed or failing stderr writer is enough -- and under
        # $ErrorActionPreference = 'Stop' that abandons this block and returns 1,
        # losing the one signal callers key on. The message is best-effort; the
        # exit code is not.
        try {
            # NOT Write-Error: 'Stop' makes it TERMINATING, so it would jump out
            # of this try and skip the messages after it. Straight to stderr.
            if (-not [string]::IsNullOrWhiteSpace($cleanupFatalMessage)) {
                [Console]::Error.WriteLine("cleanup failure: $cleanupFatalMessage")
            }
            if ($collectorInjectionStarted -and -not $collectorTeardownAuthenticated) {
                [Console]::Error.WriteLine(
                    'Capture ended without an authenticated collector teardown receipt.')
            }
            if ($gradedWindowStarted) {
                [Console]::Error.WriteLine(
                    'Capture ended without closing the passive graded window.')
            }
            # Named refusals are PRINTED here even though 2 is what gets
            # returned. A run that both refused something and never reached a
            # report has two things wrong with it, and the one that keeps its
            # numeric code must not be the only one an operator can see.
            foreach ($refusal in $captureRefusals) {
                [Console]::Error.WriteLine(
                    "capture refused: stage=$($refusal.stage) class=$($refusal.class) $($refusal.message)")
            }
            foreach ($supervisionFailure in $supervisionFailures) {
                [Console]::Error.WriteLine("supervision failure: $supervisionFailure")
            }
            [Console]::Error.WriteLine("compact status verdict: $undefinedStatusBitFailure")
            [Console]::Error.WriteLine(
                'Exit code 2 reports the compact status verdict. Any failure printed above still needs an operator.')
        } finally {
            exit 2
        }
    }

    # --- the named-refusal code, strictly below the arbitration above ----------
    #
    # Reached only when the compact-status question was ANSWERED, which is what
    # makes 3 a code rather than dead weight: `$undefinedStatusBitFailure` is set
    # the instant a recorder exists and is retired only by a report verdict, so
    # anything that stops before the report already leaves through 2. What is
    # left for 3 is exactly the shape this lane was built for -- a supervision
    # step failed, the recorder was nonetheless left alone, it finished, its
    # teardown authenticated, the report ran and answered clean -- and without
    # this the attempt would return the recorder's own 0 and be read as a good
    # capture. It was not: something was refused, and CAPTURE_REFUSAL_V1.json
    # says what.
    #
    # 3 IS THE LAST CODE CONSIDERED, NEVER THE FIRST. An `exit` inside a
    # `finally` beats a pending exception, so this block is capable of
    # suppressing every other failure in the script, and each one it could
    # suppress is checked before it. That includes the two it cannot see
    # directly: an exception still unwinding from the try body, which the outer
    # `catch` records as $harnessAbortReason before re-raising, and a
    # terminating error in the cleanup prefix, whose absence is what
    # $cleanupPrefixCompleted attests. When either is true this block PRINTS the
    # refusal and declines to return a code, leaving the real failure to decide.
    #
    # A refused READY therefore normally leaves through 2, not 3: it aborts
    # before the report runs, so the compact-status question stands unanswered.
    # Its refusal is still named on stdout, on stderr, and in
    # CAPTURE_REFUSAL_V1.json. 3 is the code for a run that finished its
    # accounting and still refused something.
    if ($captureRefusals.Count -gt 0 -or $supervisionFailures.Count -gt 0) {
        # DECIDE FIRST, REPORT SECOND, and never the other way round.
        #
        # The earlier shape interleaved them -- write a line, then `exit`, in a
        # `finally`. Every one of those writes can throw (a closed or failing
        # stderr is enough), and under $ErrorActionPreference = 'Stop' a throw
        # abandons the rest of the block: the branch that was going to fall
        # through to `exit $scriptExitCode` never got there and PowerShell
        # returned a bare 1, so a refusal alongside recorder exit 5 reported 1.
        # Measured on 5.1, not assumed.
        #
        # So the outcome is computed with nothing but variable reads -- none of
        # which can fail -- and only then is anything printed. The reporting is
        # wrapped and non-throwing, because by that point it cannot change what
        # this attempt returns, and it must not be able to.
        $refusalExitCode = $null
        $refusalNote = $null
        # Everything that outranks a refusal, in the order it outranks it. The
        # first three would otherwise be SWALLOWED by `exit 3`, because this
        # block runs before the routes that normally raise them: a cleanup
        # failure is thrown after the inner `finally` returns, and the two
        # custody guards sit after the outer one. A refusal must never be the
        # reason a custody problem or a failed cleanup went unreported.
        if (-not [string]::IsNullOrWhiteSpace($cleanupFatalMessage)) {
            $refusalExitCode = 1
            $refusalNote = "cleanup failure: $cleanupFatalMessage"
        } elseif ($collectorInjectionStarted -and -not $collectorTeardownAuthenticated) {
            $refusalExitCode = 1
            $refusalNote = 'Capture ended without an authenticated collector teardown receipt.'
        } elseif ($gradedWindowStarted) {
            $refusalExitCode = 1
            $refusalNote = 'Capture ended without closing the passive graded window.'
        } elseif (-not $cleanupPrefixCompleted) {
            # The two failures this block cannot see from here. Returning any
            # code at all would suppress them, so it returns none: the refusal
            # is already printed and receipted, and whatever is unwinding gets
            # to finish and report itself.
            $refusalNote =
                'The cleanup prefix ended on a terminating error. That failure ' +
                'outranks the refusal printed above and is left to report itself.'
        } elseif ($harnessAborted) {
            $refusalNote =
                "The attempt also aborted: $harnessAbortReason. That failure " +
                'outranks the refusal printed above and is left to report itself.'
        } elseif ($scriptExitCode -ne 0) {
            # THE RECORDER'S OWN VERDICT OUTRANKS THE REFUSAL. By here the try
            # body ran to completion, so $scriptExitCode holds collector_record's
            # exit status -- and returning 3 over a nonzero one would report
            # "the harness refused something" for a run whose RECORDER failed,
            # losing the code that says which. 3 means "everything ran and the
            # harness still refused"; it cannot also mean "and the recorder fell
            # over".
            $refusalNote =
                "collector_record exited $scriptExitCode. That failure outranks " +
                'the refusal printed above and is what this attempt returns.'
        } else {
            $refusalExitCode = 3
        }
        try {
            foreach ($refusal in $captureRefusals) {
                [Console]::Error.WriteLine(
                    "capture refused: stage=$($refusal.stage) class=$($refusal.class) $($refusal.message)")
            }
            foreach ($supervisionFailure in $supervisionFailures) {
                [Console]::Error.WriteLine("supervision failure: $supervisionFailure")
            }
            # Names the receipt, NOT the exit code: on four of the branches
            # above the answer is not 3, and announcing it here read as a
            # promise this block does not always keep.
            [Console]::Error.WriteLine(
                "The refusal above is recorded in $captureRefusalReceiptPath.")
            if (-not [string]::IsNullOrWhiteSpace($refusalNote)) {
                [Console]::Error.WriteLine($refusalNote)
            }
        } catch {
            # A refusal that could not be printed is still a refusal. The code
            # below is the part that must survive.
        }
        if ($null -ne $refusalExitCode) {
            exit $refusalExitCode
        }
    }
    }
    if (-not [string]::IsNullOrWhiteSpace($cleanupFatalMessage)) {
        throw $cleanupFatalMessage
    }
}

if ($collectorInjectionStarted -and -not $collectorTeardownAuthenticated) {
    Write-Error 'Capture ended without an authenticated collector teardown receipt.'
    exit 1
}
if ($gradedWindowStarted) {
    Write-Error 'Capture ended without closing the passive graded window.'
    exit 1
}
exit $scriptExitCode
