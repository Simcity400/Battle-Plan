<#
.SYNOPSIS
    Rung-by-rung acceptance-ladder orchestrator for production capture
    (MASTER_EXTRACTION_CONTRACT F6 tooling; the live runs are OWNER-GATED).

.DESCRIPTION
    Drives the engineering-acceptance ladder one rung at a time. Each rung is a
    launch -> validate -> record-verdict step whose verdict is persisted in a
    ladder-state JSON. Ordering is FAIL-CLOSED: a rung refuses to run until every
    predecessor has a recorded "passed" verdict, so nobody can jump to the six-hour
    soak before the speed-5 short proof has actually held.

    The ladder rungs (in order):
      1. synthetic-fault        - fault-injection proof: framing intact, per-domain
                                  degrade, acceptance telemetry present.
      2. paused-short-proof     - paused game short capture (SOURCE_UNAVAILABLE
                                  degrade path), telemetry present.
      3. speed5-short-proof     - speed-5 short capture: p95 frame time < 100 ms,
                                  zero skipped + zero coalesced game hours.
      4. five-minute-full-roster- five-minute full-roster capture, hourly met, zero
                                  skipped hours, frame budget held.
      5. weak-pc-six-core       - six-core CPU-affinity-constrained capture (weak-PC
                                  emulation), same gates as the full-roster rung.
      6. six-hour-soak          - six-hour soak: hourly met, zero skipped + zero
                                  coalesced hours, frame budget held for the run.

    This script builds and TESTS the machinery. Actually launching the game is
    owner-gated: a live rung requires the explicit -Live switch AND a running
    hoi4.exe. Every rung verdict must rest on evidence PROVABLY produced by THIS
    capture attempt. A rung is graded by RUNNING THE AUTHORITATIVE VALIDATOR
    IN-PROCESS over the run's own artifacts (Get-ArtifactGateVerdict ->
    Invoke-LadderArtifactValidator -> Test-ProductionCaptureArtifacts): the verdict
    is the validator's return value, never a user-writable
    production_validation_report.json sidecar (which a forger could hand-author).
    Each recorded run is also BOUND TO ITS RUNG (its capture plan's `rung` field),
    and predecessor ordering re-verifies each earlier rung by re-grading its bound
    run IN-PROCESS — refusing a run bound to a different rung and refusing two rungs
    that share one run_dir — rather than trusting the mutable state JSON status
    field. Where an authoritative attempt-bound live producer does not yet exist,
    the live path FAILS CLOSED (refuses to certify) rather than accept a forgeable
    boolean; the owed live producers are enumerated in Get-RungBoundProducerOwed
    and tracked under F6 in MASTER_EXTRACTION_CONTRACT.md. The state machine,
    ordering, rung binding, in-process grading, quoting, and CPU-affinity math are
    all covered by -SelfTest (which injects a validator override over fabricated
    runs) and never touch the game.

    PowerShell 5.1 compatible. No rig hardcoding.

.PARAMETER Rung
    Rung to run: a rung name, or "next" for the first pending rung. Requires -Live.

.PARAMETER Status
    Print the current ladder state and exit.

.PARAMETER Reset
    Reset the ladder state (all rungs back to pending) and exit.

.PARAMETER Live
    Actually launch the game for the requested rung (owner-gated). Without it, a
    rung run is refused fail-closed.

.PARAMETER StateFile
    Ladder-state JSON path. Default <scriptDir>\..\runs\acceptance_ladder_state.json.

.PARAMETER SelfTest
    Run the offline state-machine / ordering / affinity self-tests and exit.
#>
param(
    [string]$Rung = "",
    [switch]$Status,
    [switch]$Reset,
    [switch]$Live,
    [string]$StateFile = "",
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Capture our own switches BEFORE dot-sourcing: ProductionCapture.Validation.ps1
# declares `param([switch]$SelfTest)`, and dot-sourcing runs its param block in
# THIS scope, which would otherwise reset $SelfTest to its (false) default.
$runSelfTest = [bool]$SelfTest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "ProductionCapture.Validation.ps1")

# ---------------------------------------------------------------------------
# Rung definitions. This list is the SINGLE, IMMUTABLE, canonical rung sequence:
# fail-closed ordering (Test-PredecessorsPassed) keys off a rung's POSITION HERE,
# never off any mutable `order` value read from the state JSON. Each rung names
# the capture it launches, a per-rung `proof` predicate it must independently
# satisfy (empty = none beyond the full artifact gate), and whether it is a long
# run that must carry the launcher's -AcknowledgeLongRun acknowledgement.
# ---------------------------------------------------------------------------

function Get-LadderRungDefinitions {
    return @(
        [ordered]@{
            name        = "synthetic-fault"
            description = "fault-injection proof: framing intact, per-domain degrade, telemetry present"
            captureMode = "Smoke"
            proof       = "synthetic_fault"
        },
        [ordered]@{
            name        = "paused-short-proof"
            description = "paused game short capture (SOURCE_UNAVAILABLE degrade), telemetry present"
            captureMode = "Smoke"
            proof       = "paused"
        },
        [ordered]@{
            name        = "speed5-short-proof"
            description = "speed-5 short capture: p95 frame < 100ms, zero skipped + zero coalesced hours"
            captureMode = "ShortSpeed5"
            proof       = ""
        },
        [ordered]@{
            name        = "five-minute-full-roster"
            description = "five-minute full-roster capture: hourly met, zero skipped hours, frame budget held"
            captureMode = "ShortSpeed5"
            proof       = ""
        },
        [ordered]@{
            name          = "weak-pc-six-core"
            description   = "six-core CPU-affinity-constrained capture (weak-PC emulation), full-roster gates"
            captureMode   = "ShortSpeed5"
            affinityCores = 6
            proof         = ""
        },
        [ordered]@{
            name              = "six-hour-soak"
            description       = "six-hour soak: hourly met, zero skipped + zero coalesced hours, frame budget held"
            captureMode       = "FullGame"
            proof             = ""
            acknowledgeLongRun = $true
        }
    )
}

function Get-RungProofKind {
    param([Parameter(Mandatory = $true)]$RungDef)
    if ($RungDef.Contains("proof")) { return [string]$RungDef.proof }
    return ""
}

# ---------------------------------------------------------------------------
# Ladder-state machine.
# ---------------------------------------------------------------------------

function New-LadderState {
    $defs = Get-LadderRungDefinitions
    $rungs = @()
    for ($i = 0; $i -lt $defs.Count; $i++) {
        $rungs += [ordered]@{
            name        = $defs[$i].name
            order       = $i
            description = $defs[$i].description
            status      = "pending"
            run_dir     = $null
            problems    = @()
            verdict_utc = $null
        }
    }
    return [ordered]@{
        schema      = "acceptance_ladder_state_v1"
        created_utc = (Get-Date).ToUniversalTime().ToString("o")
        updated_utc = (Get-Date).ToUniversalTime().ToString("o")
        rungs       = $rungs
    }
}

function Read-LadderState {
    param([Parameter(Mandatory = $true)][string]$StateFile)
    if (-not (Test-Path -LiteralPath $StateFile -PathType Leaf)) {
        return New-LadderState
    }
    $raw = Get-Content -LiteralPath $StateFile -Raw
    if ($null -eq $raw -or $raw.Trim().Length -eq 0) {
        return New-LadderState
    }
    try {
        $parsed = $raw | ConvertFrom-Json
    } catch {
        throw "ladder state is not valid JSON ($StateFile): $($_.Exception.Message)"
    }
    # Fail-closed: a state file whose rung set does not match the current ladder
    # definition is stale and must not be trusted for ordering.
    $defNames = (Get-LadderRungDefinitions | ForEach-Object { $_.name })
    $stateNames = @($parsed.rungs | ForEach-Object { $_.name })
    if (($stateNames -join "|") -ne ($defNames -join "|")) {
        throw "ladder state rung set does not match current ladder definition; -Reset to rebuild ($StateFile)"
    }
    return $parsed
}

function Save-LadderState {
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)][string]$StateFile
    )
    $dir = Split-Path -Parent $StateFile
    if ($dir.Length -gt 0 -and -not (Test-Path -LiteralPath $dir -PathType Container)) {
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
    }
    $State.updated_utc = (Get-Date).ToUniversalTime().ToString("o")
    $State | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $StateFile -Encoding UTF8
}

function Get-RungByName {
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)][string]$Name
    )
    $match = @($State.rungs | Where-Object { $_.name -eq $Name })
    if ($match.Count -eq 0) {
        throw "unknown rung '$Name'; valid rungs: $((Get-LadderRungDefinitions | ForEach-Object { $_.name }) -join ', ')"
    }
    return $match[0]
}

function Get-CanonicalRungOrder {
    <#
        The rung's index in the SCRIPT-DEFINED canonical sequence (its identity,
        not any mutable state field). Fail-closed on an unknown name.
    #>
    param([Parameter(Mandatory = $true)][string]$Name)
    $defs = Get-LadderRungDefinitions
    for ($i = 0; $i -lt $defs.Count; $i++) {
        if ($defs[$i].name -eq $Name) { return $i }
    }
    throw "unknown rung '$Name'; valid rungs: $(($defs | ForEach-Object { $_.name }) -join ', ')"
}

function Get-StateRungByName {
    param($State, [string]$Name)
    return @($State.rungs | Where-Object { $_.name -eq $Name })[0]
}

function Get-NextPendingRungName {
    <#
        The first not-yet-passed rung in CANONICAL script order. Never trusts a
        mutable `order` value from the state JSON.
    #>
    param([Parameter(Mandatory = $true)]$State)
    foreach ($def in (Get-LadderRungDefinitions)) {
        $rung = Get-StateRungByName -State $State -Name $def.name
        if ($null -eq $rung -or $rung.status -ne "passed") { return $def.name }
    }
    return $null
}

function Test-PredecessorsPassed {
    <#
        Fail-closed ordering check keyed off rung IDENTITY against the fixed,
        script-defined canonical sequence. Returns Ok ($true only when every
        canonically-earlier rung has a "passed" status in the state) and Blocking
        (the first such predecessor that has not passed, or $null).

        Deliberately IGNORES the `order` field in the state JSON: that field is
        cosmetic and mutable, so editing e.g. speed5-short-proof.order to -1 must
        NOT let it jump ahead of its true predecessors.

        A predecessor counts as passed only when its recorded verdict is BACKED BY
        its bound artifact (finding 6): the persisted state `status` field alone is
        not trusted, because it is hand-editable. Each predecessor is re-verified
        against its recorded run directory's bound report (and, for a proof rung,
        its manifest evidence) via Test-RungBoundVerdict, so forging a predecessor
        to status="passed" without a verifying bound artifact does NOT unlock a
        later rung.
    #>
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)][string]$Name
    )
    $targetIdx = Get-CanonicalRungOrder -Name $Name
    $defs = Get-LadderRungDefinitions
    $seenRunDirs = @{}
    for ($i = 0; $i -lt $targetIdx; $i++) {
        $predName = $defs[$i].name
        $predRung = Get-StateRungByName -State $State -Name $predName
        $predRunDir = if ($null -eq $predRung) { "" } else { [string]$predRung.run_dir }

        # Distinct-attempt binding (finding 3): no two rungs may share ONE run_dir.
        # Pointing speed5 / five-minute / weak-pc all at a single valid speed5 run
        # would otherwise let one bound run verify several rungs and unlock the soak.
        # Checked here (before the per-rung reverify) over each PASSED predecessor's
        # recorded directory, so a shared path is refused even if the run would
        # otherwise verify. Pending/unset run_dirs are left to Test-RungBoundVerdict
        # to diagnose precisely.
        if (-not [string]::IsNullOrWhiteSpace($predRunDir) -and (Test-Path -LiteralPath $predRunDir -PathType Container)) {
            $key = ([System.IO.Path]::GetFullPath($predRunDir)).TrimEnd([char]'\', [char]'/').ToLowerInvariant()
            if ($seenRunDirs.ContainsKey($key)) {
                return [pscustomobject]@{ Ok = $false; Blocking = $predName; Reason = "run_dir_shared_with_rung_'$($seenRunDirs[$key])'_not_a_distinct_attempt" }
            }
            $seenRunDirs[$key] = $predName
        }

        # Re-derive the predecessor's verdict from its bound artifacts, re-graded
        # IN-PROCESS (identity binding + the authoritative validator), never trusting
        # the mutable status field.
        $verdict = Test-RungBoundVerdict -State $State -Name $predName
        if (-not $verdict.Passed) {
            return [pscustomobject]@{ Ok = $false; Blocking = $predName; Reason = $verdict.Reason }
        }
    }
    return [pscustomobject]@{ Ok = $true; Blocking = $null; Reason = $null }
}

function Set-RungVerdict {
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][bool]$Passed,
        [AllowEmptyCollection()][string[]]$Problems = @(),
        [string]$RunDir = $null
    )
    $rung = Get-RungByName -State $State -Name $Name
    if ($Passed) { $rung.status = "passed" } else { $rung.status = "failed" }
    $rung.problems = $Problems
    $rung.run_dir = $RunDir
    $rung.verdict_utc = (Get-Date).ToUniversalTime().ToString("o")
}

# ---------------------------------------------------------------------------
# CPU affinity (weak-PC rung).
# ---------------------------------------------------------------------------

function Get-CpuAffinityMask {
    <#
        Affinity bitmask selecting the lowest `CoreCount` logical processors:
        the low CoreCount bits set. 6 cores -> 0b111111 = 63. Fail-closed on a
        non-positive count or a count above the machine's processor count.
    #>
    param(
        [Parameter(Mandatory = $true)][int]$CoreCount,
        [int]$AvailableCores = 0
    )
    if ($CoreCount -le 0) {
        throw "affinity core count must be >= 1 (got $CoreCount)"
    }
    if ($AvailableCores -le 0) {
        $AvailableCores = [int][System.Environment]::ProcessorCount
    }
    if ($CoreCount -gt $AvailableCores) {
        throw "requested $CoreCount cores but only $AvailableCores available; cannot emulate a weaker machine by adding cores"
    }
    $mask = [int64]0
    for ($i = 0; $i -lt $CoreCount; $i++) {
        $mask = $mask -bor ([int64]1 -shl $i)
    }
    return $mask
}

# ---------------------------------------------------------------------------
# Capture producers (the launch seam). A capture function takes ($rungDef,
# $runDir) and MUST leave a native_session_manifest.json in $runDir, returning
# the run dir. The live producer reuses Start-Hoi4ProductionSpeed5Capture.ps1;
# self-test injects a fabricated-manifest producer.
# ---------------------------------------------------------------------------

function ConvertTo-QuotedArgumentLine {
    <#
        Build a Windows command-line string from an argument array, DOUBLE-QUOTING
        any element that contains whitespace or a quote. Start-Process joins a
        raw -ArgumentList ARRAY with bare spaces and does NOT quote, so a path like
        "...\Documents\Paradox Interactive\..." splits into "...\Documents\Paradox"
        + "Interactive\..." and the child tries to execute a nonexistent path.
        Passing a single, correctly quoted STRING to -ArgumentList avoids that.
    #>
    # AllowEmptyString so an empty-string ELEMENT binds (finding 6): a mandatory
    # [string[]] otherwise rejects an "" element at parameter binding, before this
    # function can emit it as "". AllowEmptyCollection keeps an empty ARRAY legal.
    param([Parameter(Mandatory = $true)][AllowEmptyCollection()][AllowEmptyString()][string[]]$Arguments)
    # Correct Windows CommandLineToArgvW quoting: when an element needs quotes,
    # DOUBLE every run of backslashes that immediately precedes a quote (then
    # escape the quote), and DOUBLE any run of trailing backslashes (which would
    # otherwise escape the closing quote). Without this, an arg like
    # C:\a "b"\c\ or one ending in a backslash does not round-trip through the
    # child process. Mirrors ConvertTo-NativeArgumentLine in the validation module.
    return (($Arguments | ForEach-Object {
        # An EMPTY argument (finding 6) must round-trip as an explicit "" token.
        # Without this it matches neither branch below, is emitted as the empty
        # string, and vanishes when the pieces are space-joined — so `-A "" -B x`
        # would reach the child as `-A -B x`, shifting every following argument by
        # one. Emit "" so the empty positional is preserved.
        if ($_ -eq '') { '""' }
        elseif ($_ -match '[\s"]') { '"' + ($_ -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"' }
        else { $_ }
    }) -join ' ')
}

function Get-CaptureLauncherArgs {
    <#
        The PowerShell child-process argument array for one rung's capture launch.
        The FullGame (six-hour) rung is a deliberate long run, so it — and ONLY it
        — carries -AcknowledgeLongRun, without which the launcher's turnaround
        guard rejects it. All other rungs run under the default guard.
    #>
    param(
        [Parameter(Mandatory = $true)]$RungDef,
        [Parameter(Mandatory = $true)][string]$CaptureScript,
        [Parameter(Mandatory = $true)][string]$RunDir
    )
    $args = @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $CaptureScript,
        "-Mode", [string]$RungDef.captureMode,
        "-OutDir", $RunDir
    )
    if ($RungDef.Contains("acknowledgeLongRun") -and $RungDef.acknowledgeLongRun -eq $true) {
        $args += "-AcknowledgeLongRun"
    }
    return $args
}

function Get-RungBoundProducerOwed {
    <#
        Every rung verdict must rest on evidence PROVABLY produced by THIS capture
        attempt. This maps a rung to the attempt-bound evidence producer it needs
        and that is NOT YET WIRED into the ladder, returning an "owed" diagnosis
        (or $null when a bound producer exists). The live path must FAIL-CLOSED for
        any rung whose bound producer is owed, never launch a capture that cannot
        emit the bound evidence the gate requires and never certify a forgeable
        boolean.

        OWED LIVE PRODUCERS (F6 acceptance ladder; tracked in
        MASTER_EXTRACTION_CONTRACT.md under F6):

          * GRADED rungs (speed5-short-proof, five-minute-full-roster,
            weak-pc-six-core, six-hour-soak) need a launcher that emits an
            attempt-bound production_validation_report.json (manifest_sha256 +
            capture_id/session_id). Start-Hoi4ProductionSpeed5Capture.ps1 (the
            historical live launcher) emits NO such report, so a genuine live
            graded rung could never satisfy the bound artifact gate.
            Start-Hoi4ProductionCaptureSafe.ps1 DOES write the bound report, but
            requires out-of-band evidence pins (Hoi4Pid, speed-evidence
            transcript/nonce/verifier/probe/static-disasm hashes, static bundle +
            mod/dlc/load-order + live-resolution report) that the ladder does not
            yet collect. OWED: wire the ladder to drive
            Start-Hoi4ProductionCaptureSafe.ps1 (or invoke
            Test-ProductionCaptureArtifacts in-ladder) with that pin set so a live
            graded rung produces a bound report.

          * synthetic-fault needs an INJECTED-FAULT HARNESS that provokes a real
            fault and records attempt-linked synthetic_fault_proof evidence in the
            manifest. OWED: injected-fault harness.

          * paused-short-proof needs a PAUSE PROVER that pauses the live game and
            records attempt-linked pause_proof evidence in the manifest. OWED:
            pause prover.

        The Get-CaptureLauncherArgs / ConvertTo-QuotedArgumentLine / affinity
        machinery is retained (and unit-tested) as the seam a future wired
        producer will use; it is intentionally NOT reached while producers are owed.
    #>
    param([Parameter(Mandatory = $true)]$RungDef)
    $kind = Get-RungProofKind -RungDef $RungDef
    if ($kind -eq "synthetic_fault") {
        return "injected_fault_evidence_producer_owed: no harness yet provokes a real fault and records attempt-linked synthetic_fault_proof in this attempt's manifest; refusing to certify (fail-closed)"
    }
    if ($kind -eq "paused") {
        return "pause_evidence_producer_owed: no prover yet pauses the live game and records attempt-linked pause_proof in this attempt's manifest; refusing to certify (fail-closed)"
    }
    return "bound_validation_report_producer_owed: the ladder's live launcher (Start-Hoi4ProductionSpeed5Capture.ps1) emits no attempt-bound production_validation_report.json, and the report-producing launcher (Start-Hoi4ProductionCaptureSafe.ps1) is not yet wired into the ladder with its evidence pins; refusing to certify (fail-closed)"
}

function Invoke-LiveCapture {
    param(
        [Parameter(Mandatory = $true)]$RungDef,
        [Parameter(Mandatory = $true)][string]$RunDir
    )
    # FAIL-CLOSED producer gate (findings 2 + 3). A live rung is owner-gated AND
    # requires an attempt-bound evidence producer to exist. No forgeable shortcut:
    # if the bound producer for this rung is not wired, refuse HERE rather than
    # launch a capture whose evidence could not be bound to this attempt.
    $owed = Get-RungBoundProducerOwed -RungDef $RungDef
    if ($null -ne $owed) {
        throw "cannot run live rung '$($RungDef.name)' (evidence producer not available): $owed"
    }

    # Reachable only once a bound producer is wired (see Get-RungBoundProducerOwed).
    $captureScript = Join-Path $scriptDir "Start-Hoi4ProductionCaptureSafe.ps1"
    if (-not (Test-Path -LiteralPath $captureScript -PathType Leaf)) {
        throw "bound-report capture launcher not found: $captureScript"
    }
    $running = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
    if ($running.Count -eq 0) {
        throw "no running hoi4.exe; a live acceptance rung needs the game running (owner-gated step)"
    }
    throw "live capture producer wiring is owed for rung '$($RungDef.name)' (see Get-RungBoundProducerOwed); refusing to certify (fail-closed)"
}

# ---------------------------------------------------------------------------
# Authoritative-validator seam (finding 1). A rung is graded by RUNNING the real
# validator IN-PROCESS over THIS attempt's artifacts and using ITS return value —
# never by trusting a user-writable production_validation_report.json sidecar.
#
# ProductionCapture.Validation.ps1 is dot-sourced ONCE at the top of this script,
# so Test-ProductionCaptureArtifacts (and the manifest gate, Get-ObjectProperty,
# etc.) are already in scope here. We deliberately do NOT dot-source it again
# inside any function: re-dot-sourcing would re-run its `param([switch]$SelfTest)`
# block (clobbering switches) and its bottom-of-file self-test dispatch. Calling
# the already-imported function is the whole point — no per-call import.
#
# $script:LadderArtifactValidatorOverride is the single test seam: when set to a
# scriptblock, Invoke-LadderArtifactValidator calls it instead of the heavyweight
# gate, so the offline self-test can inject deterministic validator verdicts over
# fabricated runs WITHOUT standing up native exes / a real capture. In production
# it is $null and the real Test-ProductionCaptureArtifacts runs.
# ---------------------------------------------------------------------------

$script:LadderArtifactValidatorOverride = $null

function Get-LadderAuthoritativeVerdict {
    <#
        DEFAULT authoritative grade: run the SAME heavyweight production gate a real
        production PASS uses (Test-ProductionCaptureArtifacts) over THIS attempt's
        run directory, returning the validator's own verdict object (schema / ok /
        problems). Repo-fixed inputs (domain registry, the native gate exes, the
        python verifiers) resolve from the native root; run-local inputs resolve
        from $RunDir. Evidence pins the ladder does not yet collect are passed empty
        so the validator FAILS CLOSED until a bound live producer supplies them
        (see Get-RungBoundProducerOwed). Reached in production only once such a
        producer is wired; today every live rung fails closed earlier in
        Invoke-LiveCapture, so this default path is exercised by the self-test's
        override seam rather than by a live capture.
    #>
    param([Parameter(Mandatory = $true)][string]$RunDir)
    $nativeRoot = Split-Path -Parent $scriptDir
    $releaseRoot = Join-Path (Join-Path $nativeRoot "target") "release"
    return (Test-ProductionCaptureArtifacts `
        -RunDir $RunDir `
        -RegistryPath (Join-Path $nativeRoot "domain_registry.json") `
        -SpeedProofPath (Join-Path $RunDir "speed5_proof.json") `
        -CompactGateExe (Join-Path $releaseRoot "compact_replay_gate.exe") `
        -FoldCheckExe (Join-Path $releaseRoot "fold_check.exe") `
        -ContainerVerifyExe (Join-Path $releaseRoot "pack_replay.exe") `
        -PythonExe "python" `
        -StaticBundleScript (Join-Path (Join-Path (Split-Path -Parent $nativeRoot) "game_defs") "static_bundle.py") `
        -StaticBundleDir $RunDir `
        -CapturePlanPath (Join-Path $RunDir "production_capture_plan.json") `
        -SpeedStreamVerifier (Join-Path $scriptDir "verify_speed5_stream.py") `
        -SpeedEvidenceVerifier (Join-Path $scriptDir "verify_speed_root_evidence_v2.py"))
}

function Invoke-LadderArtifactValidator {
    <#
        The single seam through which a rung's artifacts are graded. Uses the test
        override when one is registered, otherwise the real heavyweight validator.
        Returns the validator's verdict object (never reads a sidecar).
    #>
    param([Parameter(Mandatory = $true)][string]$RunDir)
    if ($null -ne $script:LadderArtifactValidatorOverride) {
        return (& $script:LadderArtifactValidatorOverride $RunDir)
    }
    return (Get-LadderAuthoritativeVerdict -RunDir $RunDir)
}

function Get-ArtifactGateVerdict {
    <#
        Fail-closed full-artifact grade for a rung's run directory (finding 1). The
        verdict is produced by RUNNING THE AUTHORITATIVE VALIDATOR IN-PROCESS over
        THIS attempt's artifacts (Invoke-LadderArtifactValidator ->
        Test-ProductionCaptureArtifacts): the SAME heavyweight gate a real
        production PASS uses (raw stream, replay container, registry roster,
        per-domain effective-failure checks, and the independent
        compact/fold/container/static/speed verifiers).

        The verdict is the VALIDATOR'S RETURN VALUE, never a pre-existing
        production_validation_report.json sidecar. A sidecar is user-writable, so a
        forger could hand-author one with a matching manifest sha / capture id / ok
        field; it is therefore IGNORED here entirely (a producer may still write one
        for provenance, but it is never consulted for pass/fail). This removes
        sidecar forgery as an attack surface: the grade is recomputed in-process
        from the artifacts on THIS attempt.

        ok must be STRICTLY boolean $true — a numeric 1 or the string "true" is a
        malformed/forged verdict and must not pass on PowerShell truthiness.
        Returns { Passed; Problems }.
    #>
    param([Parameter(Mandatory = $true)][string]$RunDir)
    $problems = [System.Collections.Generic.List[string]]::new()

    if (-not (Test-Path -LiteralPath $RunDir -PathType Container)) {
        $problems.Add("run_dir_missing: no run directory to grade in-process (fail-closed)")
        return [pscustomobject]@{ Passed = $false; Problems = $problems.ToArray() }
    }

    $report = $null
    try {
        $report = Invoke-LadderArtifactValidator -RunDir $RunDir
    } catch {
        $problems.Add("authoritative_validator_threw: $($_.Exception.Message)")
        return [pscustomobject]@{ Passed = $false; Problems = $problems.ToArray() }
    }
    if ($null -eq $report) {
        $problems.Add("authoritative_validator_returned_no_verdict (fail-closed)")
        return [pscustomobject]@{ Passed = $false; Problems = $problems.ToArray() }
    }

    $schema = [string](Get-ObjectProperty $report "schema")
    if ($schema -ne "battle-plan.production_capture_validation.v2") {
        $problems.Add("authoritative_validator_wrong_schema (expected battle-plan.production_capture_validation.v2, got '$schema')")
    }

    # STRICT boolean $true only (reject ok:1 / "true").
    $ok = Get-ObjectProperty $report "ok"
    if ($ok -isnot [bool]) {
        $problems.Add("artifact_gate_ok_not_boolean: validator ok is '$ok' ($(if ($null -eq $ok) { 'null' } else { $ok.GetType().Name })), must be boolean true (fail-closed)")
    } elseif ($ok -ne $true) {
        $reported = @(Get-ObjectProperty $report "problems")
        if ($reported.Count -eq 0) {
            $problems.Add("artifact_gate_failed (validator ok=false, no detail)")
        } else {
            foreach ($p in $reported) { $problems.Add("artifact_gate: $p") }
        }
    }
    return [pscustomobject]@{ Passed = ($problems.Count -eq 0); Problems = $problems.ToArray() }
}

function Test-LadderRungProof {
    <#
        The per-rung condition that must be INDEPENDENTLY proven from the manifest,
        beyond the generic full-artifact gate. Fail-closed: if the capture tooling
        did not (yet) record the evidence a rung claims, the rung FAILS with an
        "evidence not available" problem rather than recording an unproven pass.

          synthetic_fault - the injected fault must actually have occurred and been
                            handled by per-domain degrade, framing intact.
          paused          - the capture must actually have been paused, taking the
                            SOURCE_UNAVAILABLE degrade path.
          "" (none)       - no extra proof; the full artifact gate is sufficient.

        Returns { Passed; Problems }.
    #>
    param(
        [Parameter(Mandatory = $true)]$RungDef,
        [Parameter(Mandatory = $true)]$Manifest
    )
    $problems = [System.Collections.Generic.List[string]]::new()
    $kind = Get-RungProofKind -RungDef $RungDef

    function Test-ProofFlag {
        param($Manifest, [string]$Path, [string]$Label, $Problems)
        $r = Resolve-JsonPath -Manifest $Manifest -Path $Path
        # STRICT boolean (finding 4): a proof flag must be a real JSON boolean $true.
        # PowerShell's `-ne $true` coerces, so JSON 1 or the string "true" would
        # otherwise satisfy `$r.Value -ne $true` (1 -eq $true) and pass a fault/pause
        # proof that never actually occurred. Require [bool] $true, nothing looser.
        if (-not $r.Found -or $r.Value -isnot [bool] -or $r.Value -ne $true) {
            $Problems.Add("$Label`: $Path must be boolean true (proof-not-occurred or non-boolean flag; fail-closed)")
        }
    }

    if ($kind -eq "synthetic_fault") {
        foreach ($f in @("fault_injected", "fault_observed", "per_domain_degrade_applied", "framing_intact")) {
            Test-ProofFlag -Manifest $Manifest -Path "synthetic_fault_proof.$f" -Label "synthetic_fault_evidence_not_available" -Problems $problems
        }
    } elseif ($kind -eq "paused") {
        foreach ($f in @("observed_paused", "source_unavailable_degrade")) {
            Test-ProofFlag -Manifest $Manifest -Path "pause_proof.$f" -Label "paused_evidence_not_available" -Problems $problems
        }
    }

    return [pscustomobject]@{ Passed = ($problems.Count -eq 0); Problems = $problems.ToArray() }
}

function Get-RunDirBoundRung {
    <#
        The rung name a run directory declares ITSELF to be for (finding 3). The
        capture plan the producer writes records `rung` (the capture-kind / rung
        identity), so a run produced for one rung cannot stand in for another.
        Returns the recorded rung name, or $null when the plan is absent/unreadable
        or carries no rung field (all of which fail the identity binding).
    #>
    param([Parameter(Mandatory = $true)][string]$RunDir)
    $planPath = Join-Path $RunDir "production_capture_plan.json"
    if (-not (Test-Path -LiteralPath $planPath -PathType Leaf)) { return $null }
    try { $plan = Get-Content -LiteralPath $planPath -Raw | ConvertFrom-Json }
    catch { return $null }
    $rung = [string](Get-ObjectProperty $plan "rung")
    if ([string]::IsNullOrWhiteSpace($rung)) { return $null }
    return $rung
}

function Test-RungBoundVerdict {
    <#
        Re-derive whether a rung GENUINELY passed from its bound artifacts, not from
        the mutable state `status` field (finding 6). A rung counts as passed only
        when:
          * its recorded status is "passed", AND
          * its recorded run_dir still exists, AND
          * that run_dir is BOUND TO THIS RUNG: its capture plan's `rung` field
            equals this rung's name (finding 3), so a valid run for a DIFFERENT rung
            cannot be pointed at this one, AND
          * that run_dir re-passes the full artifact gate RE-GRADED IN-PROCESS by the
            authoritative validator (Get-ArtifactGateVerdict -> the in-process
            Test-ProductionCaptureArtifacts, finding 1), not a trusted status field
            or a sidecar, AND
          * for a proof rung, its manifest still carries the required proof evidence
            (Test-LadderRungProof).
        Any other case (hand-edited status, missing/deleted run_dir, a run bound to a
        different rung, artifacts that no longer verify in-process) returns
        Passed=$false with a Reason. Returns { Passed; Reason }.
    #>
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)][string]$Name
    )
    $rung = Get-StateRungByName -State $State -Name $Name
    if ($null -eq $rung) { return [pscustomobject]@{ Passed = $false; Reason = "rung_absent_from_state" } }
    if ([string]$rung.status -ne "passed") { return [pscustomobject]@{ Passed = $false; Reason = "status_not_passed" } }
    $runDir = [string]$rung.run_dir
    if ([string]::IsNullOrWhiteSpace($runDir) -or -not (Test-Path -LiteralPath $runDir -PathType Container)) {
        return [pscustomobject]@{ Passed = $false; Reason = "recorded_run_dir_missing_cannot_reverify_bound_evidence" }
    }
    # Rung-identity binding (finding 3): the recorded run must declare itself as
    # THIS rung, so a valid speed5 run cannot verify five-minute / weak-pc.
    $boundRung = Get-RunDirBoundRung -RunDir $runDir
    if ($boundRung -ne $Name) {
        return [pscustomobject]@{ Passed = $false; Reason = "run_dir_not_bound_to_this_rung (capture plan rung='$boundRung', expected '$Name')" }
    }
    $artifact = Get-ArtifactGateVerdict -RunDir $runDir
    if (-not $artifact.Passed) {
        return [pscustomobject]@{ Passed = $false; Reason = "bound_artifact_no_longer_verifies: $($artifact.Problems -join '; ')" }
    }
    $rungDef = @(Get-LadderRungDefinitions | Where-Object { $_.name -eq $Name })[0]
    if ($null -ne $rungDef -and (Get-RungProofKind -RungDef $rungDef) -ne "") {
        $manifest = $null
        try { $manifest = Read-CaptureManifest -RunDir $runDir }
        catch { return [pscustomobject]@{ Passed = $false; Reason = "bound_manifest_unreadable_for_proof_reverify: $($_.Exception.Message)" } }
        $proof = Test-LadderRungProof -RungDef $rungDef -Manifest $manifest
        if (-not $proof.Passed) {
            return [pscustomobject]@{ Passed = $false; Reason = "bound_proof_no_longer_verifies: $($proof.Problems -join '; ')" }
        }
    }
    return [pscustomobject]@{ Passed = $true; Reason = $null }
}

function Invoke-LadderRung {
    <#
        Run one rung: enforce fail-closed ordering, launch the capture (via the
        injected producer), grade the run through the FULL artifact gate plus the
        rung's independent proof, and record + persist the verdict. Returns the
        verdict object.
    #>
    param(
        [Parameter(Mandatory = $true)]$State,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$StateFile,
        [Parameter(Mandatory = $true)][scriptblock]$CaptureFunction,
        [Parameter(Mandatory = $true)][string]$RunRoot
    )
    $defs = Get-LadderRungDefinitions
    $rungDef = @($defs | Where-Object { $_.name -eq $Name })[0]
    if ($null -eq $rungDef) {
        throw "unknown rung '$Name'"
    }

    $order = Test-PredecessorsPassed -State $State -Name $Name
    if (-not $order.Ok) {
        throw "fail-closed ordering: rung '$Name' cannot run until predecessor '$($order.Blocking)' has passed"
    }

    # Unique per invocation: a fresh, empty run dir every time so a rung can never
    # inherit STALE ARTIFACTS from an earlier run and mis-grade this one. The in-
    # process validator grades whatever artifacts sit in this dir, so the dir must
    # hold only THIS attempt's. Second-resolution timestamps alone can collide when
    # a rung is re-run quickly, so a random suffix guarantees a clean dir.
    $runDir = Join-Path $RunRoot ("ladder_" + $Name + "_" + (Get-Date -Format "yyyyMMdd_HHmmss") + "_" + [guid]::NewGuid().ToString("N").Substring(0, 8))
    New-Item -ItemType Directory -Path $runDir -Force | Out-Null

    # The producer must leave the run's artifacts (a manifest, a capture plan bound
    # to this rung, the raw stream / container / etc.) in $runDir. Grading is done
    # in-process over those artifacts, NOT from any sidecar the producer may write.
    $producedDir = & $CaptureFunction $rungDef $runDir
    if ([string]::IsNullOrEmpty($producedDir)) { $producedDir = $runDir }

    # Grade = full artifact gate RE-GRADED IN-PROCESS (finding 1, no sidecar trust)
    # AND rung-identity binding (finding 3) AND the rung's independent proof over the
    # manifest (proof-not-occurred fails closed).
    $problems = [System.Collections.Generic.List[string]]::new()

    # Rung-identity binding (finding 3): the produced run must declare ITSELF as the
    # rung we launched, so its recorded run_dir can never later stand in for another.
    $boundRung = Get-RunDirBoundRung -RunDir $producedDir
    if ($boundRung -ne $Name) {
        $problems.Add("run_not_bound_to_rung: capture plan rung='$boundRung' does not match launched rung '$Name' (fail-closed)")
    }

    $artifact = Get-ArtifactGateVerdict -RunDir $producedDir
    foreach ($p in $artifact.Problems) { $problems.Add($p) }

    $manifest = $null
    try { $manifest = Read-CaptureManifest -RunDir $producedDir }
    catch { $problems.Add("manifest_unreadable_for_rung_proof: $($_.Exception.Message)") }
    if ($null -ne $manifest) {
        $proof = Test-LadderRungProof -RungDef $rungDef -Manifest $manifest
        foreach ($p in $proof.Problems) { $problems.Add($p) }
    }

    $verdict = [pscustomobject]@{
        Passed   = ($problems.Count -eq 0)
        Problems = $problems.ToArray()
    }

    Set-RungVerdict -State $State -Name $Name -Passed $verdict.Passed -Problems $verdict.Problems -RunDir $producedDir
    Save-LadderState -State $State -StateFile $StateFile

    if ($verdict.Passed) {
        Write-Host "rung '$Name' PASSED." -ForegroundColor Green
    } else {
        Write-Host "rung '$Name' FAILED:" -ForegroundColor Red
        foreach ($p in $verdict.Problems) { Write-Host "  - $p" -ForegroundColor Red }
    }
    return $verdict
}

function Show-LadderStatus {
    param([Parameter(Mandatory = $true)]$State)
    Write-Host "acceptance ladder ($($State.schema)) updated $($State.updated_utc):"
    # Display in CANONICAL script order (not the mutable `order` field).
    $idx = 0
    foreach ($def in (Get-LadderRungDefinitions)) {
        $r = Get-StateRungByName -State $State -Name $def.name
        if ($null -eq $r) { $idx++; continue }
        $marker = switch ($r.status) {
            "passed" { "[PASS]" }
            "failed" { "[FAIL]" }
            default  { "[    ]" }
        }
        Write-Host ("  {0} {1}. {2}" -f $marker, $idx, $r.name)
        if ($r.status -eq "failed" -and @($r.problems).Count -gt 0) {
            foreach ($p in $r.problems) { Write-Host "         - $p" }
        }
        $idx++
    }
}

# ---------------------------------------------------------------------------
# Default state-file location.
# ---------------------------------------------------------------------------

function Get-DefaultStateFile {
    $runsDir = Join-Path (Split-Path -Parent $scriptDir) "runs"
    return (Join-Path $runsDir "acceptance_ladder_state.json")
}

# ---------------------------------------------------------------------------
# Offline self-tests: exercise the state machine, ordering, and affinity math
# with a fabricated-manifest capture producer. No live game.
# ---------------------------------------------------------------------------

function New-FakeCaptureForRung {
    <#
        Fabricate a rung's run directory for the OFFLINE self-test: a manifest, a
        production_capture_plan.json that RECORDS THE RUNG (capture-kind identity,
        finding 3), and — for provenance realism — a production_validation_report
        .json sidecar. The verdict the ladder records comes from the IN-PROCESS
        validator seam (finding 1), NEVER from the sidecar here; the sidecar exists
        precisely so a test can prove it is IGNORED.

        The self-test registers a validator override that re-grades this run's
        MANIFEST in-process (Test-ProductionCaptureManifest, a real subset of the
        heavyweight gate), so a healthy manifest -> ok=true and a bad manifest ->
        ok=false, regardless of what the sidecar claims.

        Knobs:
          -RungName        the rung this run is for; written to the plan's `rung`
                           field so the run is BOUND to that rung (finding 3).
          -IncludeProof:$false   omit the rung's proof evidence (proof-not-occurred
                                 -> must FAIL fail-closed).
          -ValidatorReject:$true fabricate a manifest that the IN-PROCESS validator
                                 REJECTS (null sink-write percentiles = no data), so
                                 the recorded verdict is FAIL even if a rosy sidecar
                                 is present (proves finding 1: sidecar is ignored).
          -SidecarOk       the ok value written into the (ignored) sidecar. Defaults
                           to $true so a rosy sidecar can be shown to NOT rescue a
                           run the in-process validator rejects.
          -WriteSidecar:$false   omit the sidecar entirely (proves a run with NO
                                 sidecar still grades purely from the in-process
                                 validator).
    #>
    param(
        [Parameter(Mandatory = $true)][string]$RunDir,
        [string]$RungName = "",
        [string]$ProofKind = "",
        [bool]$IncludeProof = $true,
        [bool]$ValidatorReject = $false,
        $SidecarOk = $true,
        [bool]$WriteSidecar = $true
    )
    $sessionId = [guid]::NewGuid().ToString()
    $captureId = "capture-" + [guid]::NewGuid().ToString("N").Substring(0, 12)
    $manifest = [ordered]@{
        schema              = "native_collector_session_v1"
        session_id          = $sessionId
        skipped_game_hours  = 0
        hourly_requirement  = [ordered]@{ required = $true; met = $true; problems = @() }
        compact_stream      = [ordered]@{
            frame_time_us = [ordered]@{ count = 10; avg = 500.0; p50 = 400; p95 = 5000; p99 = 900; max = 1000 }
            hour_latch    = [ordered]@{ hours_coalesced = 0 }
        }
        acceptance_telemetry = [ordered]@{
            sink_write_us = [ordered]@{ count = 10; avg = 25.0; p50 = 20; p95 = 60; p99 = 100; max = 120 }
            clock_skew    = [ordered]@{ wall_elapsed_us = 300000000; mono_elapsed_us = 300000100; drift_us = -100; wall_monotonic_ok = $true }
        }
    }
    if ($ValidatorReject) {
        # No-data capture: count>0 but null percentiles. The in-process manifest gate
        # fails this on VALUES, so ok=false regardless of any rosy sidecar.
        foreach ($pf in @("avg", "p50", "p95", "p99", "max")) { $manifest.acceptance_telemetry.sink_write_us[$pf] = $null }
    }
    if ($IncludeProof) {
        if ($ProofKind -eq "synthetic_fault") {
            $manifest["synthetic_fault_proof"] = [ordered]@{ fault_injected = $true; fault_observed = $true; per_domain_degrade_applied = $true; framing_intact = $true }
        } elseif ($ProofKind -eq "paused") {
            $manifest["pause_proof"] = [ordered]@{ observed_paused = $true; source_unavailable_degrade = $true }
        }
    }
    if (-not (Test-Path -LiteralPath $RunDir -PathType Container)) {
        New-Item -ItemType Directory -Path $RunDir -Force | Out-Null
    }
    $manifestPath = Join-Path $RunDir "native_session_manifest.json"
    $manifest | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $manifestPath -Encoding UTF8

    # Capture plan: records the attempt's capture_id/session_id AND the rung this run
    # is for (finding 3 identity binding), as a real bound producer must.
    $plan = [ordered]@{ schema = "battle-plan.production_capture_plan.v1"; capture_id = $captureId; session_id = $sessionId; rung = $RungName }
    $plan | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $RunDir "production_capture_plan.json") -Encoding UTF8

    if ($WriteSidecar) {
        # A fully-bound, rosy sidecar for provenance. It is IGNORED by the grader:
        # the in-process validator's verdict decides pass/fail. A test can set
        # -SidecarOk $true while -ValidatorReject $true to prove the sidecar cannot
        # rescue a run the in-process validator rejects.
        $manifestSha = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $report = [ordered]@{
            schema          = "battle-plan.production_capture_validation.v2"
            ok              = $SidecarOk
            manifest_path   = $manifestPath
            manifest_sha256 = $manifestSha
            capture_id      = $captureId
            session_id      = $sessionId
            problems        = @()
        }
        $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $RunDir "production_validation_report.json") -Encoding UTF8
    }
    return $RunDir
}

function Get-LadderSelfTestValidatorOverride {
    <#
        The IN-PROCESS authoritative-validator stand-in for the offline self-test.
        Re-grades THIS run's manifest with the REAL reusable manifest gate
        (Test-ProductionCaptureManifest — a genuine, native-exe-free subset of
        Test-ProductionCaptureArtifacts) and returns a v2 verdict object. This
        exercises the finding-1 seam: the verdict is recomputed from the artifacts,
        never read from a sidecar.
    #>
    return {
        param($RunDir)
        try { $m = Read-CaptureManifest -RunDir $RunDir }
        catch { return [pscustomobject]@{ schema = "battle-plan.production_capture_validation.v2"; ok = $false; problems = @("manifest_unreadable_in_process: $($_.Exception.Message)") } }
        $r = Test-ProductionCaptureManifest -Manifest $m -MaxFrameTimeUs 100000 -RequireZeroSkipped -RequireZeroCoalesced -RequireHourlyMet
        return [pscustomobject]@{ schema = "battle-plan.production_capture_validation.v2"; ok = [bool]$r.Passed; problems = @($r.Problems) }
    }
}

function Invoke-LadderSelfTest {
    $failures = [System.Collections.Generic.List[string]]::new()
    function Assert-True {
        param([bool]$Condition, [string]$Message)
        if (-not $Condition) { $failures.Add($Message) }
    }

    # Register the IN-PROCESS validator seam (finding 1): the ladder grades rungs by
    # re-running the authoritative validator over each run's artifacts, so the whole
    # self-test drives the real grading path (not a trusted sidecar). Cleared in the
    # finally so the override never leaks past the self-test.
    $priorOverride = $script:LadderArtifactValidatorOverride
    $script:LadderArtifactValidatorOverride = (Get-LadderSelfTestValidatorOverride)

    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("ladder_selftest_" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $tmp -Force | Out-Null
    try {
        $stateFile = Join-Path $tmp "state.json"
        $runRoot = Join-Path $tmp "runs"

        # A healthy producer: full manifest (with the rung's proof evidence), a plan
        # bound to the rung, and a rosy (ignored) sidecar — what a real production
        # PASS leaves. The IN-PROCESS validator re-grades the manifest and passes it.
        $healthy = { param($rungDef, $runDir) New-FakeCaptureForRung -RunDir $runDir -RungName $rungDef.name -ProofKind (Get-RungProofKind -RungDef $rungDef) }

        # 1. Fresh state: all rungs pending, next = first rung.
        $state = Read-LadderState -StateFile $stateFile
        Assert-True (@($state.rungs).Count -eq 6) "expected 6 rungs, got $(@($state.rungs).Count)"
        Assert-True ((Get-NextPendingRungName -State $state) -eq "synthetic-fault") "first pending rung should be synthetic-fault"

        # 2. Fail-closed ordering: cannot run rung 3 before rungs 1-2 pass.
        $threw = $false
        try { Invoke-LadderRung -State $state -Name "speed5-short-proof" -StateFile $stateFile -CaptureFunction $healthy -RunRoot $runRoot | Out-Null }
        catch { $threw = $true }
        Assert-True $threw "running speed5-short-proof before predecessors must throw fail-closed"

        # 3. Run rungs in order; each passes and unlocks the next.
        $order = @("synthetic-fault", "paused-short-proof", "speed5-short-proof", "five-minute-full-roster", "weak-pc-six-core", "six-hour-soak")
        foreach ($name in $order) {
            $v = Invoke-LadderRung -State $state -Name $name -StateFile $stateFile -CaptureFunction $healthy -RunRoot $runRoot
            Assert-True $v.Passed "rung $name should pass with a healthy run; problems: $($v.Problems -join '; ')"
        }
        Assert-True ($null -eq (Get-NextPendingRungName -State $state)) "all rungs should be passed"

        # 4. State persists across a reload and reports all passed.
        $reloaded = Read-LadderState -StateFile $stateFile
        Assert-True (@($reloaded.rungs | Where-Object { $_.status -eq "passed" }).Count -eq 6) "reloaded state should show 6 passed rungs"

        # 5. The IN-PROCESS validator rejected the run (ok=false) -> the rung FAILS
        #    and does NOT advance; successor stays blocked. Models the authoritative
        #    validator rejecting a bad (no-data) capture on THIS attempt's artifacts.
        $stateFile2 = Join-Path $tmp "state2.json"
        $state2 = Read-LadderState -StateFile $stateFile2
        $v = Invoke-LadderRung -State $state2 -Name "synthetic-fault" -StateFile $stateFile2 -CaptureFunction $healthy -RunRoot $runRoot
        Assert-True $v.Passed "synthetic-fault should pass"
        $v = Invoke-LadderRung -State $state2 -Name "paused-short-proof" -StateFile $stateFile2 -CaptureFunction $healthy -RunRoot $runRoot
        Assert-True $v.Passed "paused-short-proof should pass"
        $rejected = { param($rungDef, $runDir) New-FakeCaptureForRung -RunDir $runDir -RungName $rungDef.name -ProofKind (Get-RungProofKind -RungDef $rungDef) -ValidatorReject $true }
        $v = Invoke-LadderRung -State $state2 -Name "speed5-short-proof" -StateFile $stateFile2 -CaptureFunction $rejected -RunRoot $runRoot
        Assert-True (-not $v.Passed) "speed5 the in-process validator rejects must fail"
        Assert-True (($v.Problems -join "`n") -match "artifact_gate:.*sink_write_us") "failure must relay the in-process validator's problems"
        $blocked = Test-PredecessorsPassed -State $state2 -Name "five-minute-full-roster"
        Assert-True (-not $blocked.Ok) "five-minute-full-roster must stay blocked while speed5 failed"
        Assert-True ($blocked.Blocking -eq "speed5-short-proof") "blocking rung should be speed5-short-proof"

        # 6. Finding 1 (the exact defect): a run with NO sidecar is graded purely by
        #    the in-process validator. A healthy manifest with no sidecar PASSES
        #    (proving the grade never depended on a sidecar); a bad manifest with no
        #    sidecar FAILS.
        $stateFile6 = Join-Path $tmp "state6.json"
        $state6 = Read-LadderState -StateFile $stateFile6
        $noSidecarHealthy = { param($rungDef, $runDir) New-FakeCaptureForRung -RunDir $runDir -RungName $rungDef.name -ProofKind (Get-RungProofKind -RungDef $rungDef) -WriteSidecar $false }
        $v = Invoke-LadderRung -State $state6 -Name "synthetic-fault" -StateFile $stateFile6 -CaptureFunction $noSidecarHealthy -RunRoot $runRoot
        Assert-True $v.Passed "a healthy run with NO sidecar must PASS purely on the in-process validator; problems: $($v.Problems -join '; ')"
        $noSidecarBad = { param($rungDef, $runDir) New-FakeCaptureForRung -RunDir $runDir -RungName $rungDef.name -ProofKind (Get-RungProofKind -RungDef $rungDef) -WriteSidecar $false -ValidatorReject $true }
        $v = Invoke-LadderRung -State $state6 -Name "paused-short-proof" -StateFile $stateFile6 -CaptureFunction $noSidecarBad -RunRoot $runRoot
        Assert-True (-not $v.Passed) "a bad run with no sidecar must FAIL on the in-process validator"

        # 7. F3: proof-not-occurred fails closed. The artifact gate is ok=true but
        #    the rung's own evidence is absent -> the rung still FAILS.
        $stateFile7 = Join-Path $tmp "state7.json"
        $state7 = Read-LadderState -StateFile $stateFile7
        $noFaultProof = { param($rungDef, $runDir) New-FakeCaptureForRung -RunDir $runDir -RungName $rungDef.name -ProofKind (Get-RungProofKind -RungDef $rungDef) -IncludeProof $false }
        $v = Invoke-LadderRung -State $state7 -Name "synthetic-fault" -StateFile $stateFile7 -CaptureFunction $noFaultProof -RunRoot $runRoot
        Assert-True (-not $v.Passed) "synthetic-fault with no fault evidence must fail fail-closed even with ok=true artifact gate"
        Assert-True (($v.Problems -join "`n") -match "synthetic_fault_evidence_not_available") "failure must name the missing synthetic-fault evidence"
        # pass synthetic-fault legitimately, then a paused rung with no pause proof.
        Invoke-LadderRung -State $state7 -Name "synthetic-fault" -StateFile $stateFile7 -CaptureFunction $healthy -RunRoot $runRoot | Out-Null
        $noPauseProof = { param($rungDef, $runDir) New-FakeCaptureForRung -RunDir $runDir -RungName $rungDef.name -ProofKind (Get-RungProofKind -RungDef $rungDef) -IncludeProof $false }
        $v = Invoke-LadderRung -State $state7 -Name "paused-short-proof" -StateFile $stateFile7 -CaptureFunction $noPauseProof -RunRoot $runRoot
        Assert-True (-not $v.Passed) "paused-short-proof with no pause evidence must fail fail-closed"
        Assert-True (($v.Problems -join "`n") -match "paused_evidence_not_available") "failure must name the missing paused evidence"

        # 8. F5: a MUTATED `order` value in the state JSON must NOT let a rung jump
        #    ahead. Predecessor ordering is keyed off the immutable script sequence.
        $mutFile = Join-Path $tmp "mutated_order.json"
        $mut = New-LadderState
        foreach ($r in $mut.rungs) { if ($r.name -eq "speed5-short-proof") { $r.order = -1 } }  # forge a low order
        Save-LadderState -State $mut -StateFile $mutFile
        $mutState = Read-LadderState -StateFile $mutFile
        $ord = Test-PredecessorsPassed -State $mutState -Name "speed5-short-proof"
        Assert-True (-not $ord.Ok) "speed5-short-proof with a forged order=-1 must STILL be blocked by its true predecessors"
        Assert-True ($ord.Blocking -eq "synthetic-fault") "canonical ordering must name synthetic-fault as the blocker, ignoring the mutated order field"
        $threwMut = $false
        try { Invoke-LadderRung -State $mutState -Name "speed5-short-proof" -StateFile $mutFile -CaptureFunction $healthy -RunRoot $runRoot | Out-Null }
        catch { $threwMut = $true }
        Assert-True $threwMut "running a forged-order rung ahead of its predecessors must throw fail-closed"

        # 9. F6: only the FullGame (six-hour) rung carries -AcknowledgeLongRun.
        $defs = Get-LadderRungDefinitions
        $sixHour = @($defs | Where-Object { $_.name -eq "six-hour-soak" })[0]
        $speed5 = @($defs | Where-Object { $_.name -eq "speed5-short-proof" })[0]
        $sixHourArgs = Get-CaptureLauncherArgs -RungDef $sixHour -CaptureScript "C:\some path\launch.ps1" -RunDir "C:\out dir"
        $speed5Args = Get-CaptureLauncherArgs -RungDef $speed5 -CaptureScript "C:\some path\launch.ps1" -RunDir "C:\out dir"
        Assert-True ($sixHourArgs -contains "-AcknowledgeLongRun") "the six-hour FullGame rung must pass -AcknowledgeLongRun"
        Assert-True (-not ($speed5Args -contains "-AcknowledgeLongRun")) "short rungs must NOT pass -AcknowledgeLongRun"

        # 10. F1: space-containing path args are correctly quoted, and a child
        #     PowerShell process receives them intact (mirrors the real repo path
        #     under "...\Documents\Paradox Interactive\Hearts of Iron IV\...").
        $spacedDir = Join-Path $tmp "Paradox Interactive\Hearts of Iron IV"
        New-Item -ItemType Directory -Path $spacedDir -Force | Out-Null
        $reflect = Join-Path $spacedDir "reflect script.ps1"
        Set-Content -LiteralPath $reflect -Encoding UTF8 -Value @'
param([string]$Mode, [string]$OutDir, [string]$Sentinel)
Set-Content -LiteralPath $Sentinel -Value ("MODE=$Mode`nOUTDIR=$OutDir") -Encoding UTF8
'@
        $sentinel = Join-Path $spacedDir "reflected args.txt"
        $reflectArgs = @("-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", $reflect, "-Mode", "ShortSpeed5", "-OutDir", $spacedDir, "-Sentinel", $sentinel)
        $line = ConvertTo-QuotedArgumentLine -Arguments $reflectArgs
        Assert-True ($line -match '"[^"]*Paradox Interactive[^"]*"') "the quoted argument line must wrap the space-containing path in quotes"
        $psExe = (Get-Process -Id $PID).Path
        if ([string]::IsNullOrEmpty($psExe)) { $psExe = "powershell.exe" }
        $proc = Start-Process -FilePath $psExe -ArgumentList $line -PassThru -NoNewWindow -Wait
        Assert-True ($proc.ExitCode -eq 0) "child process with spaced -File/-OutDir must exit 0 (not fail on a split path)"
        Assert-True (Test-Path -LiteralPath $sentinel -PathType Leaf) "the spaced-path child must have run and written the sentinel"
        if (Test-Path -LiteralPath $sentinel -PathType Leaf) {
            $reflected = Get-Content -LiteralPath $sentinel -Raw
            Assert-True ($reflected -match ([regex]::Escape("OUTDIR=$spacedDir"))) "the child must receive the full spaced -OutDir intact"
        }

        # 11. Affinity mask math.
        Assert-True ((Get-CpuAffinityMask -CoreCount 6 -AvailableCores 12) -eq 63) "6-core mask should be 63"
        Assert-True ((Get-CpuAffinityMask -CoreCount 1 -AvailableCores 12) -eq 1) "1-core mask should be 1"
        $threwAff = $false
        try { Get-CpuAffinityMask -CoreCount 0 -AvailableCores 12 | Out-Null } catch { $threwAff = $true }
        Assert-True $threwAff "zero-core affinity must throw"
        $threwAff2 = $false
        try { Get-CpuAffinityMask -CoreCount 16 -AvailableCores 8 | Out-Null } catch { $threwAff2 = $true }
        Assert-True $threwAff2 "requesting more cores than available must throw"

        # 12. Stale state (mismatched rung set) is rejected fail-closed.
        $staleFile = Join-Path $tmp "stale.json"
        ([ordered]@{ schema = "acceptance_ladder_state_v1"; rungs = @([ordered]@{ name = "not-a-rung"; order = 0; status = "passed" }) } | ConvertTo-Json -Depth 8) | Set-Content -LiteralPath $staleFile -Encoding UTF8
        $threwStale = $false
        try { Read-LadderState -StateFile $staleFile | Out-Null } catch { $threwStale = $true }
        Assert-True $threwStale "a state file with a mismatched rung set must be rejected"

        # 13. FINDING 1 (the core defect): a hand-authored ROSY SIDECAR is IGNORED.
        #     The producer writes a fully-bound production_validation_report.json with
        #     ok=true (right manifest sha / capture id / session id — exactly the
        #     forgery the old binding gate trusted), but the MANIFEST is bad (no-data).
        #     The in-process validator re-grades the manifest and returns ok=false, so
        #     the rung FAILS despite the rosy sidecar. Fail-before: the same layout
        #     with a healthy manifest PASSES (proves the verdict follows the in-process
        #     validator, not the sidecar).
        $stateFile13 = Join-Path $tmp "state13.json"
        $state13 = Read-LadderState -StateFile $stateFile13
        $rosySidecarBadRun = { param($rungDef, $runDir) New-FakeCaptureForRung -RunDir $runDir -RungName $rungDef.name -ProofKind (Get-RungProofKind -RungDef $rungDef) -ValidatorReject $true -SidecarOk $true }
        $v = Invoke-LadderRung -State $state13 -Name "synthetic-fault" -StateFile $stateFile13 -CaptureFunction $rosySidecarBadRun -RunRoot $runRoot
        Assert-True (-not $v.Passed) "a bad run with a hand-authored ROSY ok=true sidecar must still FAIL (sidecar is ignored; verdict from in-process validator)"
        Assert-True (($v.Problems -join "`n") -match "artifact_gate:") "failure must come from the in-process validator, not the sidecar"
        # Sanity: the rosy sidecar really was present and claimed ok=true, so the
        # failure is proof the sidecar was ignored, not proof it was missing.
        $rosyRunDir = [string](Get-StateRungByName -State $state13 -Name "synthetic-fault").run_dir
        $rosySidecar = Get-Content -LiteralPath (Join-Path $rosyRunDir "production_validation_report.json") -Raw | ConvertFrom-Json
        Assert-True ($rosySidecar.ok -eq $true) "the ignored sidecar must genuinely claim ok=true (else the test proves nothing)"
        $stateFile13b = Join-Path $tmp "state13b.json"
        $state13b = Read-LadderState -StateFile $stateFile13b
        $v = Invoke-LadderRung -State $state13b -Name "synthetic-fault" -StateFile $stateFile13b -CaptureFunction $healthy -RunRoot $runRoot
        Assert-True $v.Passed "a healthy run must pass (proves the reject was in-process-validator-driven, not sidecar-driven)"

        # 13b. FINDING 1: ok must be STRICTLY boolean $true. A validator verdict of
        #      ok:1 (numeric) must be REJECTED, not coerced true. Inject a one-off
        #      override that returns ok=1 over a healthy run.
        $stateFile13d = Join-Path $tmp "state13d.json"
        $state13d = Read-LadderState -StateFile $stateFile13d
        $savedOverride = $script:LadderArtifactValidatorOverride
        $script:LadderArtifactValidatorOverride = { param($RunDir) [pscustomobject]@{ schema = "battle-plan.production_capture_validation.v2"; ok = 1; problems = @() } }
        $v = Invoke-LadderRung -State $state13d -Name "synthetic-fault" -StateFile $stateFile13d -CaptureFunction $healthy -RunRoot $runRoot
        $script:LadderArtifactValidatorOverride = $savedOverride
        Assert-True (-not $v.Passed) "a validator verdict of ok:1 (numeric, not boolean) must be rejected"
        Assert-True (($v.Problems -join "`n") -match "artifact_gate_ok_not_boolean") "failure must name the non-boolean ok"

        # 13c. FINDING 3 (the soak-unlock attack): pointing five-minute's state entry
        #      at speed5's ONE valid run must NOT let that run verify five-minute. The
        #      speed5 run is BOUND to speed5 (its capture plan's rung), so five-minute
        #      fails rung-identity binding and the soak stays blocked. No single run
        #      can stand in for multiple rungs.
        $stateFileShare = Join-Path $tmp "shared_rundir.json"
        $shareState = Read-LadderState -StateFile $stateFileShare
        foreach ($nm in @("synthetic-fault", "paused-short-proof", "speed5-short-proof")) {
            $vv = Invoke-LadderRung -State $shareState -Name $nm -StateFile $stateFileShare -CaptureFunction $healthy -RunRoot $runRoot
            Assert-True $vv.Passed "$nm should pass to set up the shared-run_dir test"
        }
        $speed5Dir = [string](Get-StateRungByName -State $shareState -Name "speed5-short-proof").run_dir
        # Forge five-minute as "passed" pointing at speed5's run_dir (the attack).
        $fiveMin = Get-StateRungByName -State $shareState -Name "five-minute-full-roster"
        $fiveMin.status = "passed"; $fiveMin.run_dir = $speed5Dir; $fiveMin.verdict_utc = (Get-Date).ToUniversalTime().ToString("o")
        Save-LadderState -State $shareState -StateFile $stateFileShare
        $shareReloaded = Read-LadderState -StateFile $stateFileShare
        $fmVerdict = Test-RungBoundVerdict -State $shareReloaded -Name "five-minute-full-roster"
        Assert-True (-not $fmVerdict.Passed) "five-minute pointed at a speed5 run must fail rung-identity binding"
        Assert-True ($fmVerdict.Reason -match "not_bound_to_this_rung") "the reason must name the rung-identity mismatch"
        $soakBlocked = Test-PredecessorsPassed -State $shareReloaded -Name "six-hour-soak"
        Assert-True (-not $soakBlocked.Ok) "the soak must stay blocked when five-minute is pointed at speed5's run"

        # 13d. FINDING 3 distinct-attempt guard: even if a shared directory's plan is
        #      (implausibly) rebound so identity passes, two rungs recorded against the
        #      SAME run_dir path are refused. Give five-minute AND weak-pc the same dir,
        #      rebind that dir's plan to five-minute, and put a distinct valid speed5
        #      run before them; the shared path across five-minute/weak-pc trips the
        #      distinct-attempt check.
        $distFile = Join-Path $tmp "distinct_attempt.json"
        $distState = Read-LadderState -StateFile $distFile
        foreach ($nm in @("synthetic-fault", "paused-short-proof", "speed5-short-proof", "five-minute-full-roster")) {
            Invoke-LadderRung -State $distState -Name $nm -StateFile $distFile -CaptureFunction $healthy -RunRoot $runRoot | Out-Null
        }
        $fmDir2 = [string](Get-StateRungByName -State $distState -Name "five-minute-full-roster").run_dir
        # Forge weak-pc "passed" against five-minute's SAME dir, and rebind that dir's
        # plan to name BOTH rungs is impossible; so rebind it to weak-pc and re-point
        # five-minute at it too -> both bound-ish, but they SHARE one path.
        $weak = Get-StateRungByName -State $distState -Name "weak-pc-six-core"
        $weak.status = "passed"; $weak.run_dir = $fmDir2; $weak.verdict_utc = (Get-Date).ToUniversalTime().ToString("o")
        Save-LadderState -State $distState -StateFile $distFile
        $distReloaded = Read-LadderState -StateFile $distFile
        $distGuard = Test-PredecessorsPassed -State $distReloaded -Name "six-hour-soak"
        Assert-True (-not $distGuard.Ok) "two rungs (five-minute + weak-pc) recorded against one shared run_dir must be refused"

        # 13e. FINDING 4: a proof flag set to JSON 1 (not boolean true) must be
        #      REJECTED (coercive equality would otherwise accept it).
        $coerceManifest = [ordered]@{
            synthetic_fault_proof = [ordered]@{ fault_injected = 1; fault_observed = $true; per_domain_degrade_applied = $true; framing_intact = $true }
        } | ConvertTo-Json -Depth 6 | ConvertFrom-Json
        $coerceDef = @(Get-LadderRungDefinitions | Where-Object { $_.name -eq "synthetic-fault" })[0]
        $coerceProof = Test-LadderRungProof -RungDef $coerceDef -Manifest $coerceManifest
        Assert-True (-not $coerceProof.Passed) "a proof flag of numeric 1 (not boolean true) must be rejected (finding 4)"
        Assert-True (($coerceProof.Problems -join "`n") -match "must be boolean true") "the reject must name the strict-boolean requirement"
        $trueManifest = [ordered]@{
            synthetic_fault_proof = [ordered]@{ fault_injected = $true; fault_observed = $true; per_domain_degrade_applied = $true; framing_intact = $true }
        } | ConvertTo-Json -Depth 6 | ConvertFrom-Json
        $trueProof = Test-LadderRungProof -RungDef $coerceDef -Manifest $trueManifest
        Assert-True $trueProof.Passed "a proof flag of boolean true must pass (fail-before/pass-after for finding 4)"

        # 13f. FINDING 6: an EMPTY argument round-trips as "". `-A "" -B x` must reach
        #      the child with an empty -A value, not shift -B into -A's slot.
        $emptyLine = ConvertTo-QuotedArgumentLine -Arguments @("-A", "", "-B", "x")
        Assert-True ($emptyLine -eq '-A "" -B x') "an empty argument must be emitted as `"`" (got: [$emptyLine])"

        # 14. F2/F3 live producer fail-closed: the live producer refuses to certify
        #     any rung whose attempt-bound evidence producer is not wired (owed),
        #     rather than launch a capture that cannot emit bound evidence.
        $defsOwed = Get-LadderRungDefinitions
        foreach ($rn in @("synthetic-fault", "paused-short-proof", "speed5-short-proof", "six-hour-soak")) {
            $rd = @($defsOwed | Where-Object { $_.name -eq $rn })[0]
            $owed = Get-RungBoundProducerOwed -RungDef $rd
            Assert-True ($null -ne $owed) "rung $rn must report an owed bound producer"
            $threwOwed = $false; $owedMsg = ""
            try { Invoke-LiveCapture -RungDef $rd -RunDir $runRoot } catch { $threwOwed = $true; $owedMsg = $_.Exception.Message }
            Assert-True $threwOwed "the live producer for $rn must fail-closed (throw) while its bound producer is owed"
            Assert-True ($owedMsg -match "evidence producer not available") "the fail-closed message must name the owed producer for $rn"
        }
        $faultOwed = Get-RungBoundProducerOwed -RungDef (@($defsOwed | Where-Object { $_.name -eq "synthetic-fault" })[0])
        Assert-True ($faultOwed -match "injected_fault_evidence_producer_owed") "synthetic-fault must name the injected-fault harness as owed"
        $pauseOwed = Get-RungBoundProducerOwed -RungDef (@($defsOwed | Where-Object { $_.name -eq "paused-short-proof" })[0])
        Assert-True ($pauseOwed -match "pause_evidence_producer_owed") "paused rung must name the pause prover as owed"
        $gradedOwed = Get-RungBoundProducerOwed -RungDef (@($defsOwed | Where-Object { $_.name -eq "speed5-short-proof" })[0])
        Assert-True ($gradedOwed -match "bound_validation_report_producer_owed") "graded rung must name the bound-report producer as owed"

        # 15. F6 forged predecessor: hand-editing a predecessor's status to "passed"
        #     without a verifying bound artifact must NOT unlock a later rung; the
        #     predecessor is re-verified against its bound artifact, not trusted
        #     from the mutable state JSON status field.
        $forgeFile = Join-Path $tmp "forged_status.json"
        $forge = New-LadderState
        foreach ($r in $forge.rungs) {
            if ($r.name -eq "synthetic-fault") { $r.status = "passed"; $r.run_dir = $null; $r.verdict_utc = (Get-Date).ToUniversalTime().ToString("o") }
        }
        Save-LadderState -State $forge -StateFile $forgeFile
        $forgeState = Read-LadderState -StateFile $forgeFile
        $fp = Test-PredecessorsPassed -State $forgeState -Name "paused-short-proof"
        Assert-True (-not $fp.Ok) "a forged status=passed predecessor with no bound run_dir must NOT count as passed"
        Assert-True ($fp.Blocking -eq "synthetic-fault") "the forged predecessor must be named as the blocker"
        $threwForge = $false
        try { Invoke-LadderRung -State $forgeState -Name "paused-short-proof" -StateFile $forgeFile -CaptureFunction $healthy -RunRoot $runRoot | Out-Null } catch { $threwForge = $true }
        Assert-True $threwForge "running a rung behind a forged-status predecessor must throw fail-closed"
        # Tamper variant: legitimately pass synthetic-fault, then DELETE its recorded
        # MANIFEST so the IN-PROCESS validator can no longer re-grade it -> the bound
        # artifact no longer verifies and the predecessor check fails. (Deleting the
        # ignored sidecar would NOT change the verdict, precisely because finding 1
        # grades from the artifacts in-process, not from the sidecar.)
        $tamperFile = Join-Path $tmp "tamper_status.json"
        $tamperState = Read-LadderState -StateFile $tamperFile
        $v = Invoke-LadderRung -State $tamperState -Name "synthetic-fault" -StateFile $tamperFile -CaptureFunction $healthy -RunRoot $runRoot
        Assert-True $v.Passed "synthetic-fault should pass legitimately before tampering"
        $passedRung = Get-StateRungByName -State $tamperState -Name "synthetic-fault"
        # Deleting the (ignored) sidecar must NOT flip the verdict.
        Remove-Item -LiteralPath (Join-Path ([string]$passedRung.run_dir) "production_validation_report.json") -Force
        $tpSidecar = Test-PredecessorsPassed -State $tamperState -Name "paused-short-proof"
        Assert-True $tpSidecar.Ok "deleting the ignored sidecar must NOT change the verdict (grade is in-process; finding 1)"
        # Deleting the MANIFEST makes the in-process validator unable to grade -> fail.
        Remove-Item -LiteralPath (Join-Path ([string]$passedRung.run_dir) "native_session_manifest.json") -Force
        $tp = Test-PredecessorsPassed -State $tamperState -Name "paused-short-proof"
        Assert-True (-not $tp.Ok) "a predecessor whose manifest was deleted must no longer verify in-process"
        Assert-True ($tp.Blocking -eq "synthetic-fault") "the tampered predecessor must be named as the blocker"

        # 16. F7 quoting: backslash-before-quote and trailing-backslash args must
        #     round-trip through a child process (Windows CommandLineToArgvW rules).
        $bsDir = Join-Path $tmp "bsargs"
        New-Item -ItemType Directory -Path $bsDir -Force | Out-Null
        $reflect2 = Join-Path $bsDir "reflect2.ps1"
        Set-Content -LiteralPath $reflect2 -Encoding UTF8 -Value @'
param([string]$A, [string]$B, [string]$C, [string]$Sentinel)
Set-Content -LiteralPath $Sentinel -Value ("A=[$A]`nB=[$B]`nC=[$C]") -Encoding UTF8
'@
        $sentinel2 = Join-Path $bsDir "reflected2.txt"
        $argA = 'C:\tmp\a b\'   # spaces AND a trailing backslash
        $argB = 'q"z'           # a bare embedded double-quote
        $argC = 'p\"q'          # a double-quote preceded by a backslash
        $reflect2Args = @("-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", $reflect2, "-A", $argA, "-B", $argB, "-C", $argC, "-Sentinel", $sentinel2)
        $line2 = ConvertTo-QuotedArgumentLine -Arguments $reflect2Args
        $psExe2 = (Get-Process -Id $PID).Path
        if ([string]::IsNullOrEmpty($psExe2)) { $psExe2 = "powershell.exe" }
        $proc2 = Start-Process -FilePath $psExe2 -ArgumentList $line2 -PassThru -NoNewWindow -Wait
        Assert-True ($proc2.ExitCode -eq 0) "child with backslash/quote args must exit 0"
        if (Test-Path -LiteralPath $sentinel2 -PathType Leaf) {
            $ref2 = Get-Content -LiteralPath $sentinel2 -Raw
            Assert-True ($ref2 -match ([regex]::Escape("A=[$argA]"))) "trailing-backslash arg must round-trip intact"
            Assert-True ($ref2 -match ([regex]::Escape("B=[$argB]"))) "bare-quote arg must round-trip intact"
            Assert-True ($ref2 -match ([regex]::Escape("C=[$argC]"))) "backslash-before-quote arg must round-trip intact"
        } else {
            Assert-True $false "the backslash/quote child must have written its sentinel"
        }

    } finally {
        # Clear the in-process validator seam so it never leaks past the self-test.
        $script:LadderArtifactValidatorOverride = $priorOverride
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }

    if ($failures.Count -gt 0) {
        Write-Host "Invoke-AcceptanceLadder self-test FAILED:" -ForegroundColor Red
        foreach ($f in $failures) { Write-Host "  - $f" -ForegroundColor Red }
        return $false
    }
    Write-Host "Invoke-AcceptanceLadder self-test: all checks passed."
    return $true
}

# ---------------------------------------------------------------------------
# Entry point dispatch.
# ---------------------------------------------------------------------------

if ($runSelfTest) {
    if (Invoke-LadderSelfTest) { exit 0 } else { exit 1 }
}

if ($StateFile.Length -eq 0) { $StateFile = Get-DefaultStateFile }

if ($Reset) {
    $fresh = New-LadderState
    Save-LadderState -State $fresh -StateFile $StateFile
    Write-Host "ladder state reset: $StateFile"
    Show-LadderStatus -State $fresh
    exit 0
}

$state = Read-LadderState -StateFile $StateFile

if ($Status) {
    Show-LadderStatus -State $state
    exit 0
}

if ($Rung.Length -eq 0) {
    Show-LadderStatus -State $state
    Write-Host ""
    Write-Host "Pass -Rung <name|next> (with -Live for a real capture), -Status, -Reset, or -SelfTest."
    exit 0
}

$targetRung = $Rung
if ($Rung -eq "next") {
    $targetRung = Get-NextPendingRungName -State $state
    if ($null -eq $targetRung) {
        Write-Host "all ladder rungs already passed; nothing to run."
        exit 0
    }
}

if (-not $Live) {
    throw "refusing to run rung '$targetRung' without -Live: launching the game is owner-gated. Re-run with -Live once the game is up, or use -SelfTest for the offline machinery checks."
}

$runRoot = Join-Path (Split-Path -Parent $scriptDir) "runs"
$verdict = Invoke-LadderRung -State $state -Name $targetRung -StateFile $StateFile `
    -CaptureFunction ${function:Invoke-LiveCapture} -RunRoot $runRoot
Show-LadderStatus -State $state
if (-not $verdict.Passed) { exit 1 }
exit 0
