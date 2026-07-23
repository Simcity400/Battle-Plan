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
      3. speed5-short-proof     - speed-5 short capture: max frame time < 100 ms,
                                  zero skipped + zero coalesced game hours.
      4. five-minute-full-roster- five-minute full-roster capture, hourly met, zero
                                  skipped hours, frame budget held.
      5. weak-pc-six-core       - six-core CPU-affinity-constrained capture (weak-PC
                                  emulation), same gates as the full-roster rung.
      6. six-hour-soak          - >=21,600-second soak: hourly met, zero skipped +
                                  zero coalesced hours, elapsed/resource observations
                                  receipt-bound, frame budget held for the run.

    This script orchestrates and can self-test the machinery. It never builds per
    rung: live launches pass -SkipBuild and reuse the final centrally built,
    hash-attested binaries. The safe launcher independently rejects a missing or
    stale centralized Release receipt.
    Actually launching the game is
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
    field. The two diagnostic proof rungs use an explicitly authenticated,
    non-production acceptance mode and independently regrade safely extracted
    folded snapshots; ordinary production defaults cannot activate that mode.
    Get-RungBoundProducerOwed remains as the fail-closed producer inventory seam.
    The state machine,
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
    [string]$PinsFile = "",
    [string]$RunRoot = "",
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Capture our own switches BEFORE dot-sourcing: ProductionCapture.Validation.ps1
# declares `param([switch]$SelfTest)`, and dot-sourcing runs its param block in
# THIS scope, which would otherwise reset $SelfTest to its (false) default.
$runSelfTest = [bool]$SelfTest
$script:SixHourSoakMinimumDurationSeconds = 21600.0
$script:SixHourSoakMinimumElapsedUs = 21600000000L
$script:SixHourSoakMinimumGameHours = 48

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "ProductionCapture.Validation.ps1")

# Owner-supplied evidence-pins file for the bound-report producer (F6 graded rungs).
# Derived at runtime, never rig-hardcoded: the -PinsFile argument wins, else the
# BATTLE_PLAN_LADDER_SAFE_PINS environment variable. Empty => the graded live producer
# fail-closes ("awaiting owner pins") rather than fabricating a pass. The self-test
# sets this module variable directly to exercise the resolve/assemble paths offline.
$script:LadderSafeLauncherPinsFile = ""
if (-not [string]::IsNullOrWhiteSpace($PinsFile)) { $script:LadderSafeLauncherPinsFile = $PinsFile }
elseif (-not [string]::IsNullOrWhiteSpace($(if (Test-Path Env:BATTLE_PLAN_LADDER_SAFE_PINS) { $env:BATTLE_PLAN_LADDER_SAFE_PINS } else { $env:HOI4_LADDER_SAFE_PINS }))) { $script:LadderSafeLauncherPinsFile = $(if (Test-Path Env:BATTLE_PLAN_LADDER_SAFE_PINS) { $env:BATTLE_PLAN_LADDER_SAFE_PINS } else { $env:HOI4_LADDER_SAFE_PINS }) }

# ---------------------------------------------------------------------------
# Rung definitions. This list is the SINGLE, IMMUTABLE, canonical rung sequence:
# fail-closed ordering (Test-PredecessorsPassed) keys off a rung's POSITION HERE,
# never off any mutable `order` value read from the state JSON. Each rung names
# the capture it launches, a per-rung `proof` predicate it must independently
# satisfy (empty = none beyond the full artifact gate). The Safe-launcher path
# enforces >=21,600 seconds directly in its argument assembly and retained
# elapsed proof.
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
            description = "speed-5 short capture: max frame < 100ms, zero skipped + zero coalesced hours"
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
            name        = "six-hour-soak"
            description = "six-hour soak: hourly met, zero skipped + zero coalesced hours, frame budget held"
            captureMode = "FullGame"
            proof       = "soak_elapsed"
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
    Write-AtomicJson -Path $StateFile -Value $State -Depth 12
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
    $seenIdentities = @{}
    for ($i = 0; $i -lt $targetIdx; $i++) {
        $predName = $defs[$i].name
        $predRung = Get-StateRungByName -State $State -Name $predName
        $predRunDir = if ($null -eq $predRung) { "" } else { [string]$predRung.run_dir }

        # Distinct-attempt binding (findings 3 + 4, re-gate2 finding 1): no two rungs
        # may share ONE run_dir path or ONE bound capture/session identity. Enforced
        # via the SHARED Test-AttemptUniqueness helper (identical logic to the
        # completion certification, so the two enforcement sites cannot drift).
        # Checked here (before the per-rung reverify) so a shared/duplicated attempt
        # is refused even if each copy would individually verify. Blank/missing
        # run_dirs are left to Test-RungBoundVerdict to diagnose precisely.
        $uniqueness = Test-AttemptUniqueness -SeenRunDirs $seenRunDirs -SeenIdentities $seenIdentities -RungName $predName -RunDir $predRunDir
        if ($null -ne $uniqueness) {
            return [pscustomobject]@{ Ok = $false; Blocking = $predName; Reason = $uniqueness }
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

function Set-ProcessAffinityFailClosed {
    <#
        APPLY a CPU-affinity bitmask to a live process and VERIFY the OS honoured it,
        fail-closed. Get-CpuAffinityMask computes the mask (weak-PC emulation); this is
        the missing APPLICATION seam that actually constrains a process to those cores.
        The mask is written via the Win32-backed .NET ProcessorAffinity setter, then
        RE-READ from a refreshed process handle and compared: if the readback differs
        from the requested mask (the OS clamped/ignored it, or the process died mid-set),
        this THROWS rather than pretend the constraint took. Returns the verified applied
        mask. No rig hardcoding: the caller supplies the target pid (resolved at runtime).

        The target process (game vs recorder) is deliberately NOT chosen here — that is
        an owner-gated OPEN QUESTION (LIVE_ACCEPTANCE_RUNBOOK); the caller passes whichever
        pid the owner selected. This function only applies+verifies a mask on a given pid.
    #>
    param(
        [Parameter(Mandatory = $true)][int]$ProcessId,
        [Parameter(Mandatory = $true)][int64]$Mask
    )
    if ($Mask -le 0) { throw "affinity mask must be a positive bitmask (got $Mask)" }
    $procCount = [int][System.Environment]::ProcessorCount
    if ($procCount -lt 63) {
        $fullMask = ([int64]1 -shl $procCount) - 1
        if (($Mask -band (-bnot $fullMask)) -ne 0) {
            throw ("affinity mask 0x{0:X} sets bits beyond the {1} available processors (cannot emulate a bigger machine)" -f $Mask, $procCount)
        }
    }
    # Get-Process throws (fail-closed) when the pid is gone — a dead target is never a
    # silent success.
    $proc = Get-Process -Id $ProcessId -ErrorAction Stop
    $proc.ProcessorAffinity = [System.IntPtr][int64]$Mask
    # Re-read from a freshly-queried handle: the setter can be silently clamped by the
    # OS, so the recorded verdict must come from the process's own reported affinity.
    $verifyProc = Get-Process -Id $ProcessId -ErrorAction Stop
    $applied = [int64]($verifyProc.ProcessorAffinity.ToInt64())
    if ($applied -ne $Mask) {
        throw ("affinity apply verification failed: requested 0x{0:X} but process {1} reports 0x{2:X} (fail-closed)" -f $Mask, $ProcessId, $applied)
    }
    return $applied
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

# ---------------------------------------------------------------------------
# Bound-report producer pin plumbing (F6 graded rungs). Start-Hoi4ProductionCapture
# Safe.ps1 emits the attempt-bound production_validation_report.json the ladder grades
# in-process, but it needs a full out-of-band evidence-pin set the owner assembles at
# live time (PID, sealed D57 transcript sha/nonce/ledger/probe/static-disasm hashes,
# static bundle + mod/dlc/load-order + live-resolution report). These helpers read and
# validate that pin set and assemble the launcher's argument line; they do NOT invent
# any pin — an absent/blank required pin fails closed.
# ---------------------------------------------------------------------------

$script:SafeLauncherRequiredStringPins = @(
    "SpeedEvidenceTranscriptPath",
    "SpeedEvidenceTranscriptSha256",
    "SpeedEvidenceTranscriptFileSha256",
    "SpeedEvidenceSessionId",
    "SpeedEvidenceNonce",
    "SpeedEvidenceNonceLedgerRoot",
    "SpeedStaticDisasmSha256",
    "StaticBundleDir",
    "ModStatePath",
    "DlcStatePath",
    "LoadOrderPath",
    "SourceInventoryPath",
    "LiveResolutionReportPath"
)

function Get-SafeLauncherPins {
    <#
        Load and validate the owner-supplied evidence-pins JSON for the bound-report
        launcher. FAIL-CLOSED: a missing file, non-JSON, a non-positive Hoi4Pid, or any
        blank/absent required string pin THROWS with the offending pin named — the ladder
        never fabricates a pin. OutDir is deliberately not a pin: the ladder allocates
        a fresh unpredictable local run directory and hands that exact empty directory
        to the Safe launcher.
        Returns the parsed pins object. Optional passthrough knobs (DurationSeconds,
        MinGameHours, EmittedTokensPath, TokenOmissionsPath, PythonExe, AttemptStateRoot,
        AffinityTarget, DeploymentProfilePath) are returned as-is when present.
    #>
    param([Parameter(Mandatory = $true)][string]$PinsFile)
    if ([string]::IsNullOrWhiteSpace($PinsFile) -or -not (Test-Path -LiteralPath $PinsFile -PathType Leaf)) {
        throw "bound-report evidence-pins file not found: '$PinsFile' (fail-closed; owner supplies it at live time)"
    }
    try { $pins = Get-Content -LiteralPath $PinsFile -Raw | ConvertFrom-Json }
    catch { throw "bound-report evidence-pins file is not valid JSON ($PinsFile): $($_.Exception.Message)" }
    $rawPid = Get-ObjectProperty $pins "Hoi4Pid"
    if (-not (Test-IsJsonInteger $rawPid) -or [int64]$rawPid -le 0) {
        throw "bound-report pins: Hoi4Pid must be a positive integer (got '$rawPid'); fail-closed"
    }
    foreach ($pinName in $script:SafeLauncherRequiredStringPins) {
        $val = Get-ObjectProperty $pins $pinName
        if ($val -isnot [string] -or [string]::IsNullOrWhiteSpace($val)) {
            throw "bound-report pins: required pin '$pinName' is missing or not a non-blank string; fail-closed"
        }
    }
    return $pins
}

function Get-SafeLauncherArgs {
    <#
        Assemble the PowerShell child-process argument array that drives
        Start-Hoi4ProductionCaptureSafe.ps1 for one graded rung from a validated pin set
        (Get-SafeLauncherPins). -Rung binds the produced run to this rung; -OutDir is the
        ladder-allocated fresh local path; -AllowExistingEmptyOutDir is passed because
        the ladder created and checked it immediately before launch. Optional DurationSeconds/MinGameHours and the
        emitted/omitted-token and python/attempt-state pins are forwarded when present.
    #>
    param(
        [Parameter(Mandatory = $true)]$RungDef,
        [Parameter(Mandatory = $true)][string]$CaptureScript,
        [Parameter(Mandatory = $true)]$Pins,
        [Parameter(Mandatory = $true)][string]$RunDir,
        [string]$AcceptanceTestNonce = ""
    )
    $a = @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-File", $CaptureScript,
        "-Rung", [string]$RungDef.name,
        "-Hoi4Pid", ([string][int64](Get-ObjectProperty $Pins "Hoi4Pid")),
        "-OutDir", $RunDir,
        "-AllowExistingEmptyOutDir",
        "-SkipBuild"
    )
    foreach ($pinName in $script:SafeLauncherRequiredStringPins) {
        $a += @(("-" + $pinName), ([string](Get-ObjectProperty $Pins $pinName)))
    }
    if ([string]$RungDef.name -eq "six-hour-soak") {
        $deploymentProfilePath = Get-ObjectProperty $Pins "DeploymentProfilePath"
        if ($deploymentProfilePath -isnot [string] -or [string]::IsNullOrWhiteSpace($deploymentProfilePath)) {
            throw "six-hour-soak requires a nonblank DeploymentProfilePath pin; fail-closed"
        }
        $durationValue = Get-ObjectProperty $Pins "DurationSeconds"
        if ($null -eq $durationValue -or [string]::IsNullOrWhiteSpace([string]$durationValue)) {
            $durationValue = $script:SixHourSoakMinimumDurationSeconds
        }
        if (-not (Test-IsJsonNumber $durationValue) -or [double]$durationValue -lt $script:SixHourSoakMinimumDurationSeconds) {
            throw "six-hour-soak requires numeric DurationSeconds >= $($script:SixHourSoakMinimumDurationSeconds); fail-closed"
        }
        $minHoursValue = Get-ObjectProperty $Pins "MinGameHours"
        if ($null -eq $minHoursValue -or [string]::IsNullOrWhiteSpace([string]$minHoursValue)) {
            $minHoursValue = $script:SixHourSoakMinimumGameHours
        }
        if (-not (Test-IsJsonInteger $minHoursValue) -or [int64]$minHoursValue -lt $script:SixHourSoakMinimumGameHours) {
            throw "six-hour-soak requires integer MinGameHours >= $($script:SixHourSoakMinimumGameHours); fail-closed"
        }
        $a += @(
            "-DurationSeconds", ([string]$durationValue),
            "-MinGameHours", ([string]$minHoursValue),
            "-DeploymentProfilePath", $deploymentProfilePath
        )
    }
    foreach ($opt in @("EmittedTokensPath", "TokenOmissionsPath", "PythonExe", "AttemptStateRoot")) {
        $val = Get-ObjectProperty $Pins $opt
        if ($null -ne $val -and -not ([string]::IsNullOrWhiteSpace([string]$val))) {
            $a += @(("-" + $opt), ([string]$val))
        }
    }
    if ([string]$RungDef.name -ne "six-hour-soak") {
        foreach ($opt in @("DurationSeconds", "MinGameHours")) {
            $val = Get-ObjectProperty $Pins $opt
            if ($null -ne $val -and -not ([string]::IsNullOrWhiteSpace([string]$val))) {
                $a += @(("-" + $opt), ([string]$val))
            }
        }
    }
    $proofKind = Get-RungProofKind -RungDef $RungDef
    if ($proofKind -in @("synthetic_fault", "paused")) {
        if ($AcceptanceTestNonce -cnotmatch '^[0-9a-f]{64}$') { throw "proof rung requires a fresh canonical acceptance-test nonce" }
        $mode = if ($proofKind -eq "synthetic_fault") { "synthetic-fault" } else { "paused" }
        $a += @("-AcceptanceMode", $mode, "-AcceptanceTestNonce", $AcceptanceTestNonce)
    }
    return $a
}

function Get-RungBoundProducerOwed {
    <#
        Every rung verdict must rest on evidence PROVABLY produced by THIS capture
        attempt. This compatibility seam returns an "owed" diagnosis only if a
        future rung is introduced without an attempt-bound producer. All six current
        rungs are wired through the Safe launcher and independently re-graded from
        their retained artifacts; the ladder never trusts a launcher's summary
        boolean as its verdict.

        The synthetic-fault and paused rungs use explicit acceptance modes plus a
        fresh nonce. Their dedicated proof tools bind the observed fault/pause
        evidence to the capture plan and retained stream before the in-process
        ladder grade accepts it. The four production-performance rungs use the same
        Safe-launcher binding without an acceptance-test mode.

        Current live rungs use Get-SafeLauncherArgs. The obsolete generic launch
        helper was removed because it described a different script contract and had
        no production caller.
    #>
    param([Parameter(Mandatory = $true)]$RungDef)
    # Every rung now has a real Safe-launcher producer and an independent
    # attempt-bound re-grade over retained artifacts.
    return $null
}

function Invoke-LiveCapture {
    param(
        [Parameter(Mandatory = $true)]$RungDef,
        [Parameter(Mandatory = $true)][string]$RunDir
    )
    # FAIL-CLOSED producer gate (findings 2 + 3). A live rung is owner-gated AND
    # requires an attempt-bound evidence producer to exist. No forgeable shortcut:
    # if the bound producer for this rung is not fully wired, refuse HERE rather than
    # launch a capture whose evidence could not be bound-and-regraded for this attempt.
    # Every current rung has pin plumbing, rung binding, a Safe-launcher producer,
    # and an independent in-process re-grade. Keep this check for fail-closed future
    # extension: a newly added rung must not launch until it has the same producer.
    $owed = Get-RungBoundProducerOwed -RungDef $RungDef
    if ($null -ne $owed) {
        throw "cannot run live rung '$($RungDef.name)' (evidence producer not available): $owed"
    }

    $captureScript = Join-Path $scriptDir "Start-Hoi4ProductionCaptureSafe.ps1"
    if (-not (Test-Path -LiteralPath $captureScript -PathType Leaf)) {
        throw "bound-report capture launcher not found: $captureScript"
    }
    if ([string]::IsNullOrWhiteSpace($script:LadderSafeLauncherPinsFile)) {
        throw "graded rung '$($RungDef.name)' requires -PinsFile (or BATTLE_PLAN_LADDER_SAFE_PINS); fail-closed"
    }
    $pins = Get-SafeLauncherPins -PinsFile $script:LadderSafeLauncherPinsFile
    $pid = [int](Get-ObjectProperty $pins "Hoi4Pid")
    $target = Get-Process -Id $pid -ErrorAction Stop
    if ($target.ProcessName -cne "the game") { throw "pinned Hoi4Pid $pid resolves to '$($target.ProcessName)', not the game; fail-closed" }

    $originalAffinity = $null
    $affinityProofPath = Join-Path $RunDir "affinity_proof.json"
    $affinityProof = $null
    if ($RungDef.Contains("affinityCores")) {
        $affinityTarget = [string](Get-ObjectProperty $pins "AffinityTarget")
        if ($affinityTarget -cne "game") {
            throw "weak-pc-six-core requires pins AffinityTarget='game'; recorder targeting is not implemented and cannot be guessed"
        }
        $mask = Get-CpuAffinityMask -CoreCount ([int]$RungDef.affinityCores)
        $originalAffinity = [int64]($target.ProcessorAffinity.ToInt64())
        $appliedMask = Set-ProcessAffinityFailClosed -ProcessId $pid -Mask $mask
        $affinityProof = [ordered]@{
            schema = "battle-plan.f6_affinity_proof.v1"
            target = "game"
            pid = $pid
            process_creation_filetime_utc = [int64]$target.StartTime.ToUniversalTime().ToFileTimeUtc()
            requested_core_count = [int]$RungDef.affinityCores
            requested_mask = [int64]$mask
            applied_mask = [int64]$appliedMask
            readback_mask = [int64]((Get-Process -Id $pid -ErrorAction Stop).ProcessorAffinity.ToInt64())
            applied_before_capture = $true
            original_mask = [int64]$originalAffinity
            restored_after_capture = $false
            restored_mask = $null
        }
        Write-AtomicJson -Path $affinityProofPath -Value $affinityProof -Depth 5
    }

    $acceptanceNonce = ""
    if ((Get-RungProofKind -RungDef $RungDef) -in @("synthetic_fault", "paused")) {
        $nonceBytes = [byte[]]::new(32)
        $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
        try { $rng.GetBytes($nonceBytes) } finally { $rng.Dispose() }
        $acceptanceNonce = -join ($nonceBytes | ForEach-Object { $_.ToString('x2') })
    }
    $args = Get-SafeLauncherArgs -RungDef $RungDef -CaptureScript $captureScript -Pins $pins -RunDir $RunDir -AcceptanceTestNonce $acceptanceNonce
    $stdout = Join-Path $RunDir "ladder_safe_launcher.stdout.log"
    $stderr = Join-Path $RunDir "ladder_safe_launcher.stderr.log"
    $proc = $null
    try {
        $proc = Start-Process -FilePath "powershell.exe" -ArgumentList (ConvertTo-QuotedArgumentLine -Arguments $args) -PassThru -Wait -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr
        if ($proc.ExitCode -ne 0) {
            throw "Safe capture launcher failed for rung '$($RungDef.name)' with exit code $($proc.ExitCode); see $stderr"
        }
    } finally {
        if ($null -ne $originalAffinity -and $null -ne $affinityProof) {
            $restored = Set-ProcessAffinityFailClosed -ProcessId $pid -Mask $originalAffinity
            $affinityProof.restored_after_capture = $true
            $affinityProof.restored_mask = [int64]$restored
            Write-AtomicJson -Path $affinityProofPath -Value $affinityProof -Depth 5
        }
    }
    return $RunDir
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

function Get-RecordedArgumentValue {
    <# Return one exact value from a persisted recorder argument vector. #>
    param(
        [Parameter(Mandatory = $true)][object[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$Name
    )
    $matches = [System.Collections.Generic.List[string]]::new()
    for ($i = 0; $i -lt $Arguments.Count; $i++) {
        if ([string]$Arguments[$i] -cne $Name) { continue }
        if (($i + 1) -ge $Arguments.Count) { throw "capture plan recorder argument '$Name' has no value" }
        $matches.Add([string]$Arguments[$i + 1])
    }
    if ($matches.Count -ne 1 -or [string]::IsNullOrWhiteSpace($matches[0])) {
        throw "capture plan must contain exactly one nonblank '$Name' recorder argument (found $($matches.Count)); fail-closed"
    }
    return $matches[0]
}

function Get-RunStaticArtifactPaths {
    <# Reconstruct the exact seven finalized static artifacts from the bound run. #>
    param([Parameter(Mandatory = $true)][string]$BundleDir)
    $manifestPath = Join-Path $BundleDir "static_bundle_manifest.json"
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "finalized static bundle manifest missing: $manifestPath"
    }
    $bundle = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    $paths = @{}
    foreach ($entry in @(Get-ObjectProperty $bundle "artifacts")) {
        $role = [string](Get-ObjectProperty $entry "role")
        $file = [string](Get-ObjectProperty $entry "file")
        if ($script:RequiredStaticArtifactNames -notcontains $role -or $paths.ContainsKey($role) -or [string]::IsNullOrWhiteSpace($file)) {
            throw "finalized static bundle has duplicate, unknown, or blank artifact role '$role'"
        }
        $path = Join-Path $BundleDir $file
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "finalized static artifact missing ($role): $path" }
        $paths[$role] = $path
    }
    if ($paths.Count -ne $script:RequiredStaticArtifactNames.Count) {
        throw "finalized static bundle does not contain the exact seven required roles"
    }
    return $paths
}

function Get-LadderAuthoritativeVerdict {
    <#
        DEFAULT authoritative grade: run the SAME heavyweight production gate a real
        production PASS uses (Test-ProductionCaptureArtifacts) over THIS attempt's
        run directory, returning the validator's own verdict object (schema / ok /
        problems). Repo-fixed inputs (domain registry, native gate exes, Python
        verifiers) resolve from the native root. Every attempt-specific input is
        reconstructed from the run's capture plan, finalized static-bundle manifest,
        speed proof, and speed-root gate. The pre-existing validation sidecar is not
        read. Missing/duplicate plan arguments or missing artifacts throw fail-closed.
    #>
    param([Parameter(Mandatory = $true)][string]$RunDir)
    $nativeRoot = Split-Path -Parent $scriptDir
    $repoRoot = Split-Path -Parent (Split-Path -Parent $nativeRoot)
    $targetRoot = if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) { Join-Path $repoRoot '.cargo-target' } else { [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR) }
    $releaseRoot = Join-Path $targetRoot "release"
    $planPath = Join-Path $RunDir "production_capture_plan.json"
    if (-not (Test-Path -LiteralPath $planPath -PathType Leaf)) { throw "capture plan missing: $planPath" }
    $plan = Get-Content -LiteralPath $planPath -Raw | ConvertFrom-Json
    $arguments = @(Get-ObjectProperty $plan "recorder_arguments")
    if ($arguments.Count -eq 0) { throw "capture plan recorder_arguments missing" }
    $acceptanceMode = [string](Get-ObjectProperty $plan "acceptance_mode")
    if ($acceptanceMode -in @("synthetic-fault", "paused")) {
        if ((Get-ObjectProperty $plan "admissible_for_production") -isnot [bool] -or (Get-ObjectProperty $plan "admissible_for_production") -ne $false) {
            throw "acceptance plan does not explicitly deny production admission"
        }
        $pythonPath = Get-RecordedArgumentValue -Arguments $arguments -Name "--static-bundle-python"
        if (-not (Test-Path -LiteralPath $pythonPath -PathType Leaf)) { throw "recorded Python executable is unavailable: $pythonPath" }
        $proofScript = if ($acceptanceMode -eq "synthetic-fault") { Join-Path $scriptDir "verify_f6_synthetic_fault_proof.py" } else { Join-Path $scriptDir "verify_f6_pause_proof.py" }
        $snapshotDir = Join-Path $RunDir "acceptance_snapshots"
        $extractionReport = Join-Path $RunDir "acceptance_snapshot_extraction.json"
        $regradePath = Join-Path $RunDir "ladder_acceptance_regrade.json"
        & $pythonPath $proofScript --run-dir $RunDir --snapshot-dir $snapshotDir --extraction-report $extractionReport --out $regradePath | Out-Null
        if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $regradePath -PathType Leaf)) {
            return [pscustomobject]@{ schema = "battle-plan.production_capture_validation.v2"; ok = $false; problems = @("independent acceptance proof re-grade failed") }
        }
        $proof = Get-Content -LiteralPath $regradePath -Raw | ConvertFrom-Json
        $proofOk = (Get-ObjectProperty $proof "ok")
        $proofProblems = if ($proofOk -is [bool] -and $proofOk -eq $true) { @() } else { @(Get-ObjectProperty $proof "problems") }
        $receiptRegrade = $null
        if ($acceptanceMode -eq "paused" -and $proofOk -is [bool] -and $proofOk -eq $true) {
            # The Safe launcher transports a durable receipt inside the run and
            # hash-binds that receipt from the external resolved-attempt marker.
            # Re-grade that join as well as the replay-derived pause evidence, so
            # deleting or editing either receipt invalidates the rung.
            $receiptScript = Join-Path $scriptDir "verify_f6_pause_attempt_receipt.py"
            $receiptRegradePath = Join-Path $RunDir "ladder_pause_receipt_regrade.json"
            & $pythonPath $receiptScript --run-dir $RunDir --out $receiptRegradePath | Out-Null
            if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $receiptRegradePath -PathType Leaf)) {
                return [pscustomobject]@{ schema = "battle-plan.production_capture_validation.v2"; ok = $false; problems = @("independent paused-attempt receipt re-grade failed") }
            }
            $receiptRegrade = Get-Content -LiteralPath $receiptRegradePath -Raw | ConvertFrom-Json
            $receiptOk = Get-ObjectProperty $receiptRegrade "ok"
            if ($receiptOk -isnot [bool] -or $receiptOk -ne $true) {
                return [pscustomobject]@{ schema = "battle-plan.production_capture_validation.v2"; ok = $false; problems = @(Get-ObjectProperty $receiptRegrade "problems"); acceptance_proof = $proof; acceptance_receipt = $receiptRegrade }
            }
        }
        return [pscustomobject]@{
            schema = "battle-plan.production_capture_validation.v2"
            ok = ($proofOk -is [bool] -and $proofOk -eq $true)
            problems = $proofProblems
            acceptance_proof = $proof
            acceptance_receipt = $receiptRegrade
        }
    }

    $bundleDir = [string](Get-ObjectProperty $plan "finalized_static_bundle")
    if ([string]::IsNullOrWhiteSpace($bundleDir) -or -not (Test-Path -LiteralPath $bundleDir -PathType Container)) {
        throw "capture plan finalized_static_bundle is missing or unavailable: '$bundleDir'"
    }
    $staticPaths = Get-RunStaticArtifactPaths -BundleDir $bundleDir

    $speedProofPath = Join-Path $RunDir "speed5_proof.json"
    if (-not (Test-Path -LiteralPath $speedProofPath -PathType Leaf)) { throw "speed proof missing: $speedProofPath" }
    $speedProof = Get-Content -LiteralPath $speedProofPath -Raw | ConvertFrom-Json
    $gameExePath = [string](Get-ObjectProperty $speedProof "exe_path")
    if ([string]::IsNullOrWhiteSpace($gameExePath)) { throw "speed proof exe_path missing" }

    $speedRootGatePath = [string](Get-ObjectProperty $plan "domain57_game_speed_gate_v2")
    if ([string]::IsNullOrWhiteSpace($speedRootGatePath) -or -not (Test-Path -LiteralPath $speedRootGatePath -PathType Leaf)) {
        throw "capture plan speed-root gate path missing or unavailable"
    }
    $speedRootGate = Get-Content -LiteralPath $speedRootGatePath -Raw | ConvertFrom-Json
    $expectedStaticDisasmSha = [string](Get-ObjectProperty $speedRootGate "static_disasm_sha256")
    if ($expectedStaticDisasmSha -cnotmatch '^[0-9a-f]{64}$') { throw "speed-root gate static disassembly hash is noncanonical" }

    $stagedDll = [string](Get-ObjectProperty $plan "staged_dll")
    if ([string]::IsNullOrWhiteSpace($stagedDll)) { throw "capture plan staged_dll missing" }
    $collectorControlPath = Join-Path (Split-Path -Parent $stagedDll) "collector_control.json"
    $pythonPath = Get-RecordedArgumentValue -Arguments $arguments -Name "--static-bundle-python"
    if (-not (Test-Path -LiteralPath $pythonPath -PathType Leaf)) { throw "recorded Python executable is unavailable: $pythonPath" }
    return (Test-ProductionCaptureArtifacts `
        -RunDir $RunDir `
        -RegistryPath (Join-Path $nativeRoot "domain_registry.json") `
        -SpeedProofPath $speedProofPath `
        -StaticArtifactPaths $staticPaths `
        -ExpectedStagedDll $stagedDll `
        -CompactGateExe (Join-Path $releaseRoot "compact_replay_gate.exe") `
        -FoldCheckExe (Join-Path $releaseRoot "fold_check.exe") `
        -ContainerVerifyExe (Join-Path $releaseRoot "pack_replay.exe") `
        -PythonExe $pythonPath `
        -StaticBundleScript (Join-Path (Join-Path (Split-Path -Parent $nativeRoot) "game_defs") "static_bundle.py") `
        -StaticBundleDir $bundleDir `
        -GameExePath $gameExePath `
        -ModStatePath (Get-RecordedArgumentValue -Arguments $arguments -Name "--static-bundle-mod-state") `
        -DlcStatePath (Get-RecordedArgumentValue -Arguments $arguments -Name "--static-bundle-dlc-state") `
        -LoadOrderPath (Get-RecordedArgumentValue -Arguments $arguments -Name "--static-bundle-load-order") `
        -SourceInventoryPath ([string](Get-ObjectProperty $plan "static_bundle_source_inventory")) `
        -LiveResolutionReportPath ([string](Get-ObjectProperty $plan "static_bundle_live_resolution_report")) `
        -CapturePlanPath $planPath `
        -SpeedRootGatePath $speedRootGatePath `
        -CollectorControlPath $collectorControlPath `
        -SpeedMonitorPath (Join-Path $RunDir "speed5_monitor.jsonl") `
        -SpeedStreamVerifier (Join-Path $scriptDir "verify_speed5_stream.py") `
        -SpeedEvidenceTranscriptPath ([string](Get-ObjectProperty $plan "speed_evidence_transcript_path")) `
        -SpeedEvidenceNonceLedgerRoot ([string](Get-ObjectProperty $plan "speed_evidence_nonce_ledger_root")) `
        -SpeedEvidenceVerifier (Join-Path $scriptDir "verify_speed_root_evidence_v2.py") `
        -ExpectedSpeedEvidenceVerifierSha256 "1a8057e0084e8507b1c8cde6a7d7c0198652db6efca0bbcb891bc7dc99cb8e28" `
        -ExpectedSpeedEvidenceProbeSha256 "7302d6981b74b5eba149a58b30e359c9e519e7b7e275273c07ca4214615b406b" `
        -ExpectedSpeedStaticDisasmSha256 $expectedStaticDisasmSha)
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

function Get-SoakElapsedObservation {
    <# Re-derive the six-hour wall/monotonic and requested-duration proof. #>
    param(
        [Parameter(Mandatory = $true)]$Manifest,
        [Parameter(Mandatory = $true)][string]$RunDir
    )
    $problems = [System.Collections.Generic.List[string]]::new()
    $durationMs = Get-ObjectProperty $Manifest "duration_ms"
    if (-not (Test-IsJsonInteger $durationMs) -or [int64]$durationMs -lt ([int64]($script:SixHourSoakMinimumDurationSeconds * 1000))) {
        $problems.Add("soak_manifest_duration_below_six_hours")
    }
    $wall = (Resolve-JsonPath -Manifest $Manifest -Path "acceptance_telemetry.clock_skew.wall_elapsed_us").Value
    $mono = (Resolve-JsonPath -Manifest $Manifest -Path "acceptance_telemetry.clock_skew.mono_elapsed_us").Value
    foreach ($row in @(@("wall", $wall), @("mono", $mono))) {
        if (-not (Test-IsJsonInteger $row[1]) -or [int64]$row[1] -lt $script:SixHourSoakMinimumElapsedUs) {
            $problems.Add("soak_$($row[0])_elapsed_below_six_hours")
        }
    }
    $peakPrivateBytes = (Resolve-JsonPath -Manifest $Manifest -Path "host_finalize.peak_private_bytes").Value
    $peakWorkingSetBytes = (Resolve-JsonPath -Manifest $Manifest -Path "host_finalize.peak_working_set_bytes").Value
    $streamBytes = (Resolve-JsonPath -Manifest $Manifest -Path "host_finalize.stream_bytes").Value
    foreach ($row in @(@("peak_private_bytes", $peakPrivateBytes), @("peak_working_set_bytes", $peakWorkingSetBytes), @("stream_bytes", $streamBytes))) {
        if (-not (Test-IsJsonInteger $row[1]) -or [int64]$row[1] -le 0) {
            $problems.Add("soak_$($row[0])_missing_or_nonpositive")
        }
    }
    $planPath = Join-Path $RunDir "production_capture_plan.json"
    $requestedDurationSeconds = $null
    $requestedMinGameHours = $null
    $deploymentProfileFile = $null
    $deploymentProfileSha256 = $null
    $deploymentProfileSourceFile = $null
    try {
        $plan = Get-Content -LiteralPath $planPath -Raw | ConvertFrom-Json
        $arguments = @(Get-ObjectProperty $plan "recorder_arguments")
        $requestedDurationSeconds = [Convert]::ToDouble(
            (Get-RecordedArgumentValue -Arguments $arguments -Name "--duration-s"),
            [Globalization.CultureInfo]::InvariantCulture
        )
        $requestedMinGameHours = [Convert]::ToInt64(
            (Get-RecordedArgumentValue -Arguments $arguments -Name "--min-game-hours"),
            [Globalization.CultureInfo]::InvariantCulture
        )
        if ([double]::IsNaN($requestedDurationSeconds) -or [double]::IsInfinity($requestedDurationSeconds) -or $requestedDurationSeconds -lt $script:SixHourSoakMinimumDurationSeconds) {
            $problems.Add("soak_plan_duration_below_six_hours")
        }
        if ($requestedMinGameHours -lt $script:SixHourSoakMinimumGameHours) {
            $problems.Add("soak_plan_min_game_hours_below_required")
        }
        $deploymentProfileSourceFile = Get-ObjectProperty $plan "deployment_profile_source_file"
        $deploymentProfileFile = Get-ObjectProperty $plan "deployment_profile_file"
        $deploymentProfileSha256 = Get-ObjectProperty $plan "deployment_profile_sha256"
        if ($deploymentProfileSourceFile -isnot [string] -or [string]::IsNullOrWhiteSpace($deploymentProfileSourceFile) -or
            [IO.Path]::GetFileName($deploymentProfileSourceFile) -cne $deploymentProfileSourceFile) {
            $problems.Add("soak_deployment_profile_source_file_not_portable")
        }
        if ($deploymentProfileFile -isnot [string] -or [string]::IsNullOrWhiteSpace($deploymentProfileFile) -or
            [IO.Path]::GetFileName($deploymentProfileFile) -cne $deploymentProfileFile) {
            $problems.Add("soak_deployment_profile_file_not_portable")
        }
        if ($deploymentProfileSha256 -isnot [string] -or $deploymentProfileSha256 -cnotmatch '^[0-9a-f]{64}$') {
            $problems.Add("soak_deployment_profile_sha256_invalid")
        } elseif ($deploymentProfileFile -is [string] -and [IO.Path]::GetFileName($deploymentProfileFile) -ceq $deploymentProfileFile) {
            $retainedDeploymentProfilePath = Join-Path $RunDir $deploymentProfileFile
            if (-not (Test-Path -LiteralPath $retainedDeploymentProfilePath -PathType Leaf) -or
                (Get-FileHash -LiteralPath $retainedDeploymentProfilePath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $deploymentProfileSha256) {
                $problems.Add("soak_retained_deployment_profile_hash_mismatch")
            }
        }
    } catch {
        $problems.Add("soak_capture_plan_duration_unreadable: $($_.Exception.Message)")
    }
    return [pscustomobject]@{
        Passed = ($problems.Count -eq 0)
        Problems = $problems.ToArray()
        DurationMs = $durationMs
        WallElapsedUs = $wall
        MonoElapsedUs = $mono
        RequestedDurationSeconds = $requestedDurationSeconds
        RequestedMinGameHours = $requestedMinGameHours
        DeploymentProfileSourceFile = $deploymentProfileSourceFile
        DeploymentProfileFile = $deploymentProfileFile
        DeploymentProfileSha256 = $deploymentProfileSha256
        PeakPrivateBytes = $peakPrivateBytes
        PeakWorkingSetBytes = $peakWorkingSetBytes
        StreamBytes = $streamBytes
    }
}

function Write-SoakAcceptanceReceipt {
    <# Publish one immutable, attempt-bound six-hour observation receipt. #>
    param([Parameter(Mandatory = $true)][string]$RunDir)
    $path = Join-Path $RunDir "six_hour_soak_acceptance_receipt.json"
    if (Test-Path -LiteralPath $path) { throw "soak acceptance receipt already exists: $path" }
    $manifestPath = Join-Path $RunDir "native_session_manifest.json"
    $planPath = Join-Path $RunDir "production_capture_plan.json"
    $manifest = Read-CaptureManifest -RunDir $RunDir
    $plan = Get-Content -LiteralPath $planPath -Raw | ConvertFrom-Json
    $observation = Get-SoakElapsedObservation -Manifest $manifest -RunDir $RunDir
    if (-not $observation.Passed) { throw "cannot publish soak receipt: $($observation.Problems -join '; ')" }
    $receipt = [ordered]@{
        schema = "battle-plan.six_hour_soak_acceptance.v1"
        admissible_for_production = $true
        rung = "six-hour-soak"
        capture_id = Get-ObjectProperty $plan "capture_id"
        session_id = Get-ObjectProperty $manifest "session_id"
        manifest_sha256 = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
        capture_plan_sha256 = (Get-FileHash -LiteralPath $planPath -Algorithm SHA256).Hash.ToLowerInvariant()
        requested_duration_seconds = $observation.RequestedDurationSeconds
        requested_min_game_hours = $observation.RequestedMinGameHours
        deployment_profile_source_file = $observation.DeploymentProfileSourceFile
        deployment_profile_file = $observation.DeploymentProfileFile
        deployment_profile_sha256 = $observation.DeploymentProfileSha256
        manifest_duration_ms = $observation.DurationMs
        wall_elapsed_us = $observation.WallElapsedUs
        mono_elapsed_us = $observation.MonoElapsedUs
        peak_private_bytes = $observation.PeakPrivateBytes
        peak_working_set_bytes = $observation.PeakWorkingSetBytes
        stream_bytes = $observation.StreamBytes
        artifact_gate = "regraded_in_process"
    }
    $temp = "$path.$([guid]::NewGuid().ToString('N')).tmp"
    $utf8 = [Text.UTF8Encoding]::new($false)
    [IO.File]::WriteAllText($temp, ($receipt | ConvertTo-Json -Depth 8), $utf8)
    $stream = [IO.File]::Open($temp, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try { $stream.Flush($true) } finally { $stream.Dispose() }
    Move-Item -LiteralPath $temp -Destination $path
    return $path
}

function Test-SoakAcceptanceReceipt {
    param([Parameter(Mandatory = $true)][string]$RunDir)
    $problems = [System.Collections.Generic.List[string]]::new()
    $path = Join-Path $RunDir "six_hour_soak_acceptance_receipt.json"
    try { $receipt = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json }
    catch { return [pscustomobject]@{ Passed = $false; Problems = @("soak_acceptance_receipt_missing_or_unreadable") } }
    $manifestPath = Join-Path $RunDir "native_session_manifest.json"
    $planPath = Join-Path $RunDir "production_capture_plan.json"
    if ([string](Get-ObjectProperty $receipt "schema") -cne "battle-plan.six_hour_soak_acceptance.v1") { $problems.Add("soak_receipt_schema_invalid") }
    if ((Get-ObjectProperty $receipt "admissible_for_production") -isnot [bool] -or (Get-ObjectProperty $receipt "admissible_for_production") -ne $true) { $problems.Add("soak_receipt_not_production_admissible") }
    if ([string](Get-ObjectProperty $receipt "rung") -cne "six-hour-soak") { $problems.Add("soak_receipt_rung_invalid") }
    if ([string](Get-ObjectProperty $receipt "manifest_sha256") -cne (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()) { $problems.Add("soak_receipt_manifest_hash_mismatch") }
    if ([string](Get-ObjectProperty $receipt "capture_plan_sha256") -cne (Get-FileHash -LiteralPath $planPath -Algorithm SHA256).Hash.ToLowerInvariant()) { $problems.Add("soak_receipt_plan_hash_mismatch") }
    $manifest = Read-CaptureManifest -RunDir $RunDir
    $plan = Get-Content -LiteralPath $planPath -Raw | ConvertFrom-Json
    $receiptCaptureId = Get-ObjectProperty $receipt "capture_id"
    $receiptSessionId = Get-ObjectProperty $receipt "session_id"
    if ($receiptCaptureId -isnot [string] -or [string]::IsNullOrWhiteSpace($receiptCaptureId) -or $receiptCaptureId -cne [string](Get-ObjectProperty $plan "capture_id")) { $problems.Add("soak_receipt_capture_id_mismatch") }
    if ($receiptSessionId -isnot [string] -or [string]::IsNullOrWhiteSpace($receiptSessionId) -or $receiptSessionId -cne [string](Get-ObjectProperty $manifest "session_id")) { $problems.Add("soak_receipt_session_id_mismatch") }
    $observation = Get-SoakElapsedObservation -Manifest $manifest -RunDir $RunDir
    foreach ($problem in $observation.Problems) { $problems.Add($problem) }
    foreach ($field in @("manifest_duration_ms", "wall_elapsed_us", "mono_elapsed_us", "peak_private_bytes", "peak_working_set_bytes", "stream_bytes")) {
        if (-not (Test-IsJsonInteger (Get-ObjectProperty $receipt $field))) { $problems.Add("soak_receipt_$field`_invalid") }
    }
    if ((Get-ObjectProperty $receipt "manifest_duration_ms") -ne $observation.DurationMs) { $problems.Add("soak_receipt_manifest_duration_mismatch") }
    if ((Get-ObjectProperty $receipt "wall_elapsed_us") -ne $observation.WallElapsedUs) { $problems.Add("soak_receipt_wall_elapsed_mismatch") }
    if ((Get-ObjectProperty $receipt "mono_elapsed_us") -ne $observation.MonoElapsedUs) { $problems.Add("soak_receipt_mono_elapsed_mismatch") }
    if ((Get-ObjectProperty $receipt "peak_private_bytes") -ne $observation.PeakPrivateBytes) { $problems.Add("soak_receipt_peak_private_bytes_mismatch") }
    if ((Get-ObjectProperty $receipt "peak_working_set_bytes") -ne $observation.PeakWorkingSetBytes) { $problems.Add("soak_receipt_peak_working_set_bytes_mismatch") }
    if ((Get-ObjectProperty $receipt "stream_bytes") -ne $observation.StreamBytes) { $problems.Add("soak_receipt_stream_bytes_mismatch") }
    if ((Get-ObjectProperty $receipt "requested_duration_seconds") -ne $observation.RequestedDurationSeconds) { $problems.Add("soak_receipt_requested_duration_mismatch") }
    if ((Get-ObjectProperty $receipt "requested_min_game_hours") -ne $observation.RequestedMinGameHours) { $problems.Add("soak_receipt_min_game_hours_mismatch") }
    if ([string](Get-ObjectProperty $receipt "deployment_profile_source_file") -cne [string]$observation.DeploymentProfileSourceFile) { $problems.Add("soak_receipt_deployment_profile_source_file_mismatch") }
    if ([string](Get-ObjectProperty $receipt "deployment_profile_file") -cne [string]$observation.DeploymentProfileFile) { $problems.Add("soak_receipt_deployment_profile_file_mismatch") }
    if ([string](Get-ObjectProperty $receipt "deployment_profile_sha256") -cne [string]$observation.DeploymentProfileSha256) { $problems.Add("soak_receipt_deployment_profile_sha256_mismatch") }
    if ([string](Get-ObjectProperty $receipt "artifact_gate") -cne "regraded_in_process") { $problems.Add("soak_receipt_artifact_gate_invalid") }
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
        [Parameter(Mandatory = $true)]$Manifest,
        [string]$RunDir = ""
    )
    $problems = [System.Collections.Generic.List[string]]::new()
    $kind = Get-RungProofKind -RungDef $RungDef

    if ($kind -in @("synthetic_fault", "paused") -and -not [string]::IsNullOrWhiteSpace($RunDir)) {
        $planPath = Join-Path $RunDir "production_capture_plan.json"
        if (Test-Path -LiteralPath $planPath -PathType Leaf) {
            $boundPlan = Get-Content -LiteralPath $planPath -Raw | ConvertFrom-Json
            $expectedMode = if ($kind -eq "synthetic_fault") { "synthetic-fault" } else { "paused" }
            if ([string](Get-ObjectProperty $boundPlan "acceptance_mode") -eq $expectedMode) {
                try { $regrade = Get-LadderAuthoritativeVerdict -RunDir $RunDir }
                catch {
                    $problems.Add("acceptance_proof_regrade_threw: $($_.Exception.Message)")
                    return [pscustomobject]@{ Passed = $false; Problems = $problems.ToArray() }
                }
                $boundProof = Get-ObjectProperty $regrade "acceptance_proof"
                $expectedSchema = if ($kind -eq "synthetic_fault") { "battle-plan.f6_synthetic_fault_proof.v1" } else { "battle-plan.f6_pause_proof.v1" }
                if ([string](Get-ObjectProperty $boundProof "schema") -cne $expectedSchema -or (Get-ObjectProperty $boundProof "ok") -isnot [bool] -or (Get-ObjectProperty $boundProof "ok") -ne $true) {
                    $problems.Add("attempt-bound acceptance proof did not independently verify")
                }
                return [pscustomobject]@{ Passed = ($problems.Count -eq 0); Problems = $problems.ToArray() }
            }
        }
    }

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
    } elseif ($kind -eq "soak_elapsed") {
        if ([string]::IsNullOrWhiteSpace($RunDir)) {
            $problems.Add("soak_run_dir_missing")
        } else {
            $elapsed = Get-SoakElapsedObservation -Manifest $Manifest -RunDir $RunDir
            foreach ($problem in $elapsed.Problems) { $problems.Add($problem) }
        }
    }

    return [pscustomobject]@{ Passed = ($problems.Count -eq 0); Problems = $problems.ToArray() }
}

function Test-LadderAffinityProof {
    <# Grade the weak-PC rung's OS apply/readback/restore evidence. #>
    param(
        [Parameter(Mandatory = $true)]$RungDef,
        [Parameter(Mandatory = $true)][string]$RunDir
    )
    $problems = [System.Collections.Generic.List[string]]::new()
    if (-not $RungDef.Contains("affinityCores")) {
        return [pscustomobject]@{ Passed = $true; Problems = @() }
    }
    $path = Join-Path $RunDir "affinity_proof.json"
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        $problems.Add("affinity_proof_missing")
        return [pscustomobject]@{ Passed = $false; Problems = $problems.ToArray() }
    }
    try { $proof = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json }
    catch {
        $problems.Add("affinity_proof_not_json")
        return [pscustomobject]@{ Passed = $false; Problems = $problems.ToArray() }
    }
    $expectedMask = Get-CpuAffinityMask -CoreCount ([int]$RungDef.affinityCores)
    if ([string](Get-ObjectProperty $proof "schema") -cne "battle-plan.f6_affinity_proof.v1") { $problems.Add("affinity_proof_wrong_schema") }
    if ([string](Get-ObjectProperty $proof "target") -cne "game") { $problems.Add("affinity_target_not_game") }
    $integerFieldsOk = $true
    foreach ($field in @("pid", "process_creation_filetime_utc", "requested_core_count", "requested_mask", "applied_mask", "readback_mask", "original_mask", "restored_mask")) {
        if (-not (Test-IsJsonInteger (Get-ObjectProperty $proof $field))) {
            $problems.Add("affinity_$field`_not_integer")
            $integerFieldsOk = $false
        }
    }
    if ($integerFieldsOk) {
        if ([int64](Get-ObjectProperty $proof "pid") -le 0) { $problems.Add("affinity_pid_not_positive") }
        if ([int64](Get-ObjectProperty $proof "process_creation_filetime_utc") -le 0) { $problems.Add("affinity_process_creation_not_positive") }
        if ([int64](Get-ObjectProperty $proof "requested_core_count") -ne [int64]$RungDef.affinityCores) { $problems.Add("affinity_requested_core_count_mismatch") }
        if ([int64](Get-ObjectProperty $proof "requested_mask") -ne $expectedMask -or [int64](Get-ObjectProperty $proof "applied_mask") -ne $expectedMask -or [int64](Get-ObjectProperty $proof "readback_mask") -ne $expectedMask) { $problems.Add("affinity_requested_applied_readback_mismatch") }
        if ([int64](Get-ObjectProperty $proof "original_mask") -le 0 -or [int64](Get-ObjectProperty $proof "restored_mask") -le 0) { $problems.Add("affinity_original_or_restored_mask_not_positive") }
        if ([int64](Get-ObjectProperty $proof "restored_mask") -ne [int64](Get-ObjectProperty $proof "original_mask")) { $problems.Add("affinity_restore_readback_mismatch") }
    }
    if ((Get-ObjectProperty $proof "applied_before_capture") -isnot [bool] -or (Get-ObjectProperty $proof "applied_before_capture") -ne $true) { $problems.Add("affinity_not_applied_before_capture") }
    if ((Get-ObjectProperty $proof "restored_after_capture") -isnot [bool] -or (Get-ObjectProperty $proof "restored_after_capture") -ne $true) { $problems.Add("affinity_not_restored_after_capture") }
    $planPath = Join-Path $RunDir "production_capture_plan.json"
    try { $plan = Get-Content -LiteralPath $planPath -Raw | ConvertFrom-Json }
    catch { $plan = $null; $problems.Add("affinity_capture_plan_unreadable") }
    if ($null -ne $plan) {
        $planPid = Get-ObjectProperty $plan "pid"
        $planCreated = Get-ObjectProperty $plan "process_creation_filetime_utc"
        if (-not (Test-IsJsonInteger $planPid)) {
            $problems.Add("affinity_capture_plan_pid_not_integer")
        }
        if (-not (Test-IsJsonInteger $planCreated)) {
            $problems.Add("affinity_capture_plan_process_creation_not_integer")
        }
        if ($integerFieldsOk) {
            if (Test-IsJsonInteger $planPid) {
                if ([int64]$planPid -ne [int64](Get-ObjectProperty $proof "pid")) { $problems.Add("affinity_pid_not_bound_to_capture_plan") }
            }
            if (Test-IsJsonInteger $planCreated) {
                if ([int64]$planCreated -ne [int64](Get-ObjectProperty $proof "process_creation_filetime_utc")) { $problems.Add("affinity_process_creation_not_bound_to_capture_plan") }
            }
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

function Get-RunDirAttemptIdentity {
    <#
        The capture/session identity a run directory's BOUND ARTIFACTS declare
        (finding 4): the capture plan's `capture_id` and the manifest's `session_id`
        — producer-written evidence, NOT the user-editable ladder-state summary. Two
        rung entries that resolve to the SAME identity are copies of ONE attempt, not
        the distinct attempts the ladder requires: an attacker can copy one valid
        attempt into several directories (defeating path-uniqueness) and rebind each
        plan's `rung` + hash (defeating the rung-string check), but the copies still
        share the ORIGINAL attempt's capture/session identity.

        STRICT TYPES + FAIL-CLOSED RESULT (re-gate2 finding 4): the identity fields
        must be GENUINE, non-blank JSON STRINGS. The old [string] coercion minted a
        usable identity from a numeric capture_id (123 -> "123") or a boolean
        session_id (true -> "true"), and — worse — an unreadable identity returned
        $null, which SKIPPED deduplication entirely (fail-open). Returns
        { Ok; Identity; Problem }: Ok=$false (with a named Problem) for a missing/
        unparsable plan or manifest or a wrong-typed/blank identity field, so the
        caller can refuse fail-closed instead of silently skipping the check.
    #>
    param([Parameter(Mandatory = $true)][string]$RunDir)
    function New-IdentityFailure([string]$Problem) {
        return [pscustomobject]@{ Ok = $false; Identity = $null; Problem = $Problem }
    }
    $planPath = Join-Path $RunDir "production_capture_plan.json"
    $manifestPath = Join-Path $RunDir "native_session_manifest.json"
    if (-not (Test-Path -LiteralPath $planPath -PathType Leaf)) { return (New-IdentityFailure "capture_plan_missing") }
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { return (New-IdentityFailure "manifest_missing") }
    try { $plan = Get-Content -LiteralPath $planPath -Raw | ConvertFrom-Json } catch { return (New-IdentityFailure "capture_plan_unparsable") }
    try { $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json } catch { return (New-IdentityFailure "manifest_unparsable") }
    $captureId = Get-ObjectProperty $plan "capture_id"
    if ($captureId -isnot [string] -or [string]::IsNullOrWhiteSpace($captureId)) {
        return (New-IdentityFailure "capture_id_not_a_nonblank_json_string (got '$captureId')")
    }
    $sessionId = Get-ObjectProperty $manifest "session_id"
    if ($sessionId -isnot [string] -or [string]::IsNullOrWhiteSpace($sessionId)) {
        return (New-IdentityFailure "session_id_not_a_nonblank_json_string (got '$sessionId')")
    }
    return [pscustomobject]@{
        Ok       = $true
        Identity = ("cap=" + $captureId.Trim().ToLowerInvariant() + "|sess=" + $sessionId.Trim().ToLowerInvariant())
        Problem  = $null
    }
}

function Test-AttemptUniqueness {
    <#
        The SINGLE shared distinct-attempt invariant (re-gate2 finding 1): no two
        rung state entries may share one normalized run_dir path OR one bound
        capture/session identity. Factored here so BOTH enforcement sites —
        Test-PredecessorsPassed (ordering) and Get-LadderCompletionVerdict (the
        `-Rung next` all-passed certification) — run the IDENTICAL logic and cannot
        drift. Mutates the caller's $SeenRunDirs / $SeenIdentities maps. Returns
        $null when this rung's dir is distinct (or blank/missing — left to the
        bound-verdict reverify to diagnose precisely), otherwise a refusal reason.
        An UNREADABLE/WRONG-TYPED identity is itself a refusal (fail-closed,
        re-gate2 finding 4), never a silent skip of the dedup.
    #>
    param(
        [Parameter(Mandatory = $true)][hashtable]$SeenRunDirs,
        [Parameter(Mandatory = $true)][hashtable]$SeenIdentities,
        [Parameter(Mandatory = $true)][string]$RungName,
        [AllowEmptyString()][string]$RunDir = ""
    )
    if ([string]::IsNullOrWhiteSpace($RunDir) -or -not (Test-Path -LiteralPath $RunDir -PathType Container)) { return $null }
    $key = ([System.IO.Path]::GetFullPath($RunDir)).TrimEnd([char]'\', [char]'/').ToLowerInvariant()
    if ($SeenRunDirs.ContainsKey($key)) {
        return "run_dir_shared_with_rung_'$($SeenRunDirs[$key])'_not_a_distinct_attempt"
    }
    $SeenRunDirs[$key] = $RungName
    $identity = Get-RunDirAttemptIdentity -RunDir $RunDir
    if (-not $identity.Ok) {
        return "attempt_identity_unverifiable: $($identity.Problem) (fail-closed)"
    }
    if ($SeenIdentities.ContainsKey($identity.Identity)) {
        return "capture_session_identity_shared_with_rung_'$($SeenIdentities[$identity.Identity])'_not_a_distinct_attempt"
    }
    $SeenIdentities[$identity.Identity] = $RungName
    return $null
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
        $proof = Test-LadderRungProof -RungDef $rungDef -Manifest $manifest -RunDir $runDir
        if (-not $proof.Passed) {
            return [pscustomobject]@{ Passed = $false; Reason = "bound_proof_no_longer_verifies: $($proof.Problems -join '; ')" }
        }
    }
    if ($null -ne $rungDef) {
        $affinity = Test-LadderAffinityProof -RungDef $rungDef -RunDir $runDir
        if (-not $affinity.Passed) {
            return [pscustomobject]@{ Passed = $false; Reason = "bound_affinity_proof_no_longer_verifies: $($affinity.Problems -join '; ')" }
        }
    }
    if ($Name -eq "six-hour-soak") {
        $soakReceipt = Test-SoakAcceptanceReceipt -RunDir $runDir
        if (-not $soakReceipt.Passed) {
            return [pscustomobject]@{ Passed = $false; Reason = "bound_soak_receipt_no_longer_verifies: $($soakReceipt.Problems -join '; ')" }
        }
    }
    return [pscustomobject]@{ Passed = $true; Reason = $null }
}

function Get-LadderCompletionVerdict {
    <#
        Fail-closed re-derivation of "the whole ladder is already complete" (finding
        1). The `-Rung next` short-circuit must NOT declare the ladder finished by
        trusting the mutable state `status` fields (Get-NextPendingRungName keys off
        `status` alone): a hand-authored state with all six entries status="passed"
        and run_dir=null would otherwise print "all passed" and exit 0 without ever
        calling the authoritative validator. This re-verifies EVERY rung against its
        BOUND run_dir artifacts via Test-RungBoundVerdict (the SAME in-process
        authoritative validator path predecessor ordering uses), so a passed entry
        with a null/missing/unvalidatable run_dir is an UNVERIFIED completion.

        CROSS-RUNG invariants (re-gate2 finding 1): per-rung reverification alone is
        NOT sufficient — six distinct directories cloned from ONE valid attempt (each
        plan's `rung` rebound, all sharing one capture_id/session_id) individually
        re-verify, yet are one attempt masquerading as six. Completion therefore also
        enforces the SAME cross-rung path/identity deduplication predecessor ordering
        enforces, through the SHARED Test-AttemptUniqueness helper (factored so the
        two callers cannot drift). Returns:
          Complete   - $true only when every rung genuinely re-verifies in-process
                       AND every rung is a distinct attempt
          Unverified - [string[]] "rung: reason" for each rung that does not
    #>
    param([Parameter(Mandatory = $true)]$State)
    $unverified = @()
    $seenRunDirs = @{}
    $seenIdentities = @{}
    foreach ($def in (Get-LadderRungDefinitions)) {
        $rung = Get-StateRungByName -State $State -Name $def.name
        $runDir = if ($null -eq $rung) { "" } else { [string]$rung.run_dir }
        # Same order as predecessor ordering: uniqueness first, then the reverify.
        $uniqueness = Test-AttemptUniqueness -SeenRunDirs $seenRunDirs -SeenIdentities $seenIdentities -RungName $def.name -RunDir $runDir
        if ($null -ne $uniqueness) {
            $unverified += ("{0}: {1}" -f $def.name, $uniqueness)
            continue
        }
        $bv = Test-RungBoundVerdict -State $State -Name $def.name
        if (-not $bv.Passed) { $unverified += ("{0}: {1}" -f $def.name, $bv.Reason) }
    }
    return [pscustomobject]@{ Complete = ($unverified.Count -eq 0); Unverified = $unverified }
}

function Assert-LadderFreshRunDir {
    <#
        Create the fresh run directory for THIS invocation, refusing a pre-existing
        candidate path (re-gate2 finding 5). The old `New-Item -Force` silently
        REUSED an already-existing directory WITHOUT clearing it, so an attacker (or
        a stale earlier run) could plant artifacts at a predicted path and have them
        survive into the graded dir. A GUID-suffixed candidate colliding honestly is
        astronomically unlikely, so an existing path is treated as tampering/staleness
        and REFUSED fail-closed — never cleared, never reused. No -Force: creation
        itself must be this invocation's.
    #>
    param([Parameter(Mandatory = $true)][string]$Path)
    if (Test-Path -LiteralPath $Path) {
        throw "fresh run dir candidate already exists: '$Path'; refusing to reuse a pre-existing directory for a new attempt (planted/stale artifacts would be graded as this run's; fail-closed)"
    }
    New-Item -ItemType Directory -Path $Path | Out-Null
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
    # a rung is re-run quickly, so a random suffix guarantees a clean dir. The
    # allocation REFUSES a pre-existing candidate (re-gate2 finding 5) rather than
    # silently reusing whatever already sits at that path.
    $runDir = Join-Path $RunRoot ("ladder_" + $Name + "_" + (Get-Date -Format "yyyyMMdd_HHmmss") + "_" + [guid]::NewGuid().ToString("N").Substring(0, 8))
    Assert-LadderFreshRunDir -Path $runDir

    # The producer must leave the run's artifacts (a manifest, a capture plan bound
    # to this rung, the raw stream / container / etc.) in $runDir. Grading is done
    # in-process over those artifacts, NOT from any sidecar the producer may write.
    $produced = & $CaptureFunction $rungDef $runDir
    # Pin the graded directory to the FRESH dir allocated for THIS invocation
    # (finding 3). A producer must populate $runDir IN PLACE; returning a DIFFERENT
    # directory (e.g. a previously-passing same-rung run) would let a STALE attempt be
    # regraded and recorded as this run's evidence. An empty/blank return means
    # "populated in place" and grades $runDir; ANY other returned path is refused
    # fail-closed (normalized-path equality), never silently graded.
    if (-not [string]::IsNullOrWhiteSpace([string]$produced)) {
        $normProduced = ([System.IO.Path]::GetFullPath([string]$produced)).TrimEnd([char]'\', [char]'/').ToLowerInvariant()
        $normFresh = ([System.IO.Path]::GetFullPath($runDir)).TrimEnd([char]'\', [char]'/').ToLowerInvariant()
        if ($normProduced -ne $normFresh) {
            throw "producer for rung '$Name' returned '$produced', not this invocation's fresh run dir '$runDir'; refusing to grade a non-fresh/foreign attempt (fail-closed)"
        }
    }
    $producedDir = $runDir

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
        $proof = Test-LadderRungProof -RungDef $rungDef -Manifest $manifest -RunDir $producedDir
        foreach ($p in $proof.Problems) { $problems.Add($p) }
    }
    $affinity = Test-LadderAffinityProof -RungDef $rungDef -RunDir $producedDir
    foreach ($p in $affinity.Problems) { $problems.Add($p) }
    if ($Name -eq "six-hour-soak" -and $problems.Count -eq 0) {
        try { Write-SoakAcceptanceReceipt -RunDir $producedDir | Out-Null }
        catch { $problems.Add("soak_acceptance_receipt_publish_failed: $($_.Exception.Message)") }
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
    <#
        INFORMATIONAL display of the persisted state file. The statuses shown here
        are RECORDED STATE ONLY — they are NOT revalidated against bound artifacts
        (re-gate2 finding 2), so no line may read "[PASS]": that wording would
        assert a validated pass from a hand-editable JSON field. Certification only
        happens through the in-process validator paths (Test-RungBoundVerdict /
        Get-LadderCompletionVerdict on `-Rung next`).
    #>
    param([Parameter(Mandatory = $true)]$State)
    Write-Host "acceptance ladder ($($State.schema)) updated $($State.updated_utc):"
    Write-Host "  (statuses below are RECORDED state only, not revalidated; certification requires the in-process validator via -Rung next)"
    # Display in CANONICAL script order (not the mutable `order` field).
    $idx = 0
    foreach ($def in (Get-LadderRungDefinitions)) {
        $r = Get-StateRungByName -State $State -Name $def.name
        if ($null -eq $r) { $idx++; continue }
        $marker = switch ($r.status) {
            "passed" { "[RECORDED passed - not revalidated]" }
            "failed" { "[RECORDED failed]" }
            default  { "[pending]" }
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
    $isSoak = $RungName -eq "six-hour-soak"
    $fakeDurationMs = if ($isSoak) { 21600000L } else { 300000L }
    $fakeElapsedUs = if ($isSoak) { 21600000100L } else { 300000100L }
    $fakeDurationSeconds = if ($isSoak) { "21600" } else { "300" }
    $manifest = [ordered]@{
        schema              = "native_collector_session_v1"
        session_id          = $sessionId
        duration_ms         = $fakeDurationMs
        skipped_game_hours  = 0
        hourly_requirement  = [ordered]@{ required = $true; met = $true; problems = @() }
        compact_stream      = [ordered]@{
            frame_time_us = [ordered]@{ count = 10; avg = 500.0; p50 = 400; p95 = 5000; p99 = 900; max = 1000 }
            hour_latch    = [ordered]@{ hours_coalesced = 0 }
        }
        acceptance_telemetry = [ordered]@{
            schema        = "battle-plan.acceptance_telemetry.v1"
            complete      = $true
            problems      = @()
            collector_sink_write_us = [ordered]@{ count = 10; total = 250; avg = 25.0; p50 = 20; p99 = 100; max = 120 }
            sink_write_us = [ordered]@{ count = 10; avg = 25.0; p50 = 20; p95 = 60; p99 = 100; max = 120 }
            clock_skew    = [ordered]@{ wall_elapsed_us = ($fakeElapsedUs - 100); mono_elapsed_us = $fakeElapsedUs; drift_us = -100; wall_monotonic_ok = $true }
            clock_skew_scope = "complete physical H4CR capture; capture-set scoped during rollover"
        }
        host_finalize = [ordered]@{ peak_private_bytes = 268435456; peak_working_set_bytes = 201326592; stream_bytes = 1048576 }
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
    Write-AtomicJson -Path $manifestPath -Value $manifest -Depth 12

    # Capture plan: records the attempt's capture_id/session_id AND the rung this run
    # is for (finding 3 identity binding), as a real bound producer must.
    $plan = [ordered]@{
        schema = "battle-plan.production_capture_plan.v1"; capture_id = $captureId
        session_id = $sessionId; pid = 4242; process_creation_filetime_utc = 133000000000000000
        rung = $RungName
        recorder_arguments = @("--duration-s", $fakeDurationSeconds, "--min-game-hours", "48")
    }
    if ($isSoak) {
        $fakeProfilePath = Join-Path $RunDir "selected_deployment_profile.bin"
        Set-Content -LiteralPath $fakeProfilePath -Value '{"profile":"self-test"}' -Encoding UTF8
        $plan["deployment_profile_source_file"] = "self_test_profile.json"
        $plan["deployment_profile_file"] = "selected_deployment_profile.bin"
        $plan["deployment_profile_sha256"] = (Get-FileHash -LiteralPath $fakeProfilePath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $plan | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $RunDir "production_capture_plan.json") -Encoding UTF8
    if ($RungName -eq "weak-pc-six-core") {
        [ordered]@{
            schema = "battle-plan.f6_affinity_proof.v1"; target = "game"; pid = 4242
            process_creation_filetime_utc = 133000000000000000
            requested_core_count = 6; requested_mask = 63; applied_mask = 63; readback_mask = 63
            applied_before_capture = $true; original_mask = 255; restored_after_capture = $true; restored_mask = 255
        } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $RunDir "affinity_proof.json") -Encoding UTF8
    }

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
        Write-AtomicJson -Path (Join-Path $RunDir "production_validation_report.json") -Value $report -Depth 8
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

        # 9. F1: space-containing path args are correctly quoted, and a child
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
        Write-AtomicJson -Path $staleFile -Value ([ordered]@{ schema = "acceptance_ladder_state_v1"; rungs = @([ordered]@{ name = "not-a-rung"; order = 0; status = "passed" }) }) -Depth 8
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

        # 14. Every rung now has an explicit Safe-launcher producer. Proof rungs
        #     remain non-production and are independently regraded from extracted
        #     replay snapshots rather than trusting manifest booleans.
        $defsOwed = Get-LadderRungDefinitions
        foreach ($rn in @("synthetic-fault", "paused-short-proof", "speed5-short-proof", "five-minute-full-roster", "weak-pc-six-core", "six-hour-soak")) {
            $rd = @($defsOwed | Where-Object { $_.name -eq $rn })[0]
            $owed = Get-RungBoundProducerOwed -RungDef $rd
            Assert-True ($null -eq $owed) "rung $rn must use a wired Safe-launcher producer and in-process re-grade"
        }

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

        # 17. FINDING 3 (fresh-dir pinning): a producer that IGNORES the fresh dir
        #     allocated for this invocation and returns a DIFFERENT (previously-passing,
        #     same-rung) directory must be REFUSED, so a stale attempt can never be
        #     regraded and recorded as the new run. Fail-before: the old code took the
        #     producer's returned dir verbatim and graded the stale (healthy) run,
        #     recording it as this attempt.
        $pinFile = Join-Path $tmp "pin_fresh_dir.json"
        $pinState = Read-LadderState -StateFile $pinFile
        $vPin = Invoke-LadderRung -State $pinState -Name "synthetic-fault" -StateFile $pinFile -CaptureFunction $healthy -RunRoot $runRoot
        Assert-True $vPin.Passed "synthetic-fault should pass to create a prior run dir for the stale-dir test"
        # A $script:-scoped stale dir so the producer scriptblock (invoked in the ladder
        # function's scope) can reach it without a fragile closure.
        $script:LadderSelfTestStaleDir = [string](Get-StateRungByName -State $pinState -Name "synthetic-fault").run_dir
        $staleReturner = {
            param($rungDef, $runDir)
            # Populate the FRESH dir healthily (so grading a fresh dir WOULD pass), but
            # return the STALE prior dir — the attack the pin must refuse.
            New-FakeCaptureForRung -RunDir $runDir -RungName $rungDef.name -ProofKind (Get-RungProofKind -RungDef $rungDef) | Out-Null
            return $script:LadderSelfTestStaleDir
        }
        $threwPin = $false; $pinMsg = ""
        try { Invoke-LadderRung -State $pinState -Name "synthetic-fault" -StateFile $pinFile -CaptureFunction $staleReturner -RunRoot $runRoot | Out-Null }
        catch { $threwPin = $true; $pinMsg = $_.Exception.Message }
        Assert-True $threwPin "a producer returning a directory other than the fresh run dir must be refused (fail-closed)"
        Assert-True ($pinMsg -match "fresh run dir") "the refusal must name the fresh-run-dir pin (got: $pinMsg)"
        $script:LadderSelfTestStaleDir = $null

        # 18. FINDING 4 (shared capture/session identity): copying ONE valid attempt
        #     into a DIFFERENT directory and rebinding the copy's plan `rung` defeats
        #     the path-uniqueness and rung-identity checks, but the copy still carries
        #     the ORIGINAL attempt's capture_id/session_id. Distinct-attempt must ALSO
        #     require a distinct capture/session identity, so the soak stays blocked.
        $idFile = Join-Path $tmp "shared_identity.json"
        $idState = Read-LadderState -StateFile $idFile
        foreach ($nm in @("synthetic-fault", "paused-short-proof", "speed5-short-proof")) {
            $vv = Invoke-LadderRung -State $idState -Name $nm -StateFile $idFile -CaptureFunction $healthy -RunRoot $runRoot
            Assert-True $vv.Passed "$nm should pass to set up the shared-identity test"
        }
        $speed5IdDir = [string](Get-StateRungByName -State $idState -Name "speed5-short-proof").run_dir
        # Copy speed5's attempt into a NEW dir (distinct path) and REBIND the copy's
        # plan rung to five-minute (keeping the ORIGINAL capture_id/session_id).
        $copyDir = Join-Path $runRoot ("copy_of_speed5_" + [guid]::NewGuid().ToString("N").Substring(0, 8))
        Copy-Item -LiteralPath $speed5IdDir -Destination $copyDir -Recurse -Force
        $copyPlanPath = Join-Path $copyDir "production_capture_plan.json"
        $copyPlan = Get-Content -LiteralPath $copyPlanPath -Raw | ConvertFrom-Json
        $copyPlan.rung = "five-minute-full-roster"
        $copyPlan | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $copyPlanPath -Encoding UTF8
        # Fail-before proof: the copy now BINDS to five-minute and its path is distinct,
        # so ONLY the identity check can catch it (path + rung-string both pass).
        Assert-True ((Get-RunDirBoundRung -RunDir $copyDir) -eq "five-minute-full-roster") "the copy's plan must rebind to five-minute (so path + rung-string checks pass; only identity catches it)"
        $fmId = Get-StateRungByName -State $idState -Name "five-minute-full-roster"
        $fmId.status = "passed"; $fmId.run_dir = $copyDir; $fmId.verdict_utc = (Get-Date).ToUniversalTime().ToString("o")
        Save-LadderState -State $idState -StateFile $idFile
        $idReloaded = Read-LadderState -StateFile $idFile
        # Check ordering for weak-pc-six-core: its predecessors (synthetic/paused/
        # speed5/five-minute) are ALL forged-passed with valid bound artifacts, so
        # NOTHING but the shared-identity check can block it. Targeting six-hour-soak
        # instead would trip on weak-pc being pending (status_not_passed) and mask
        # whether the identity guard actually fired.
        $idGuard = Test-PredecessorsPassed -State $idReloaded -Name "weak-pc-six-core"
        Assert-True (-not $idGuard.Ok) "two rungs sharing one capture/session identity must be refused as not-distinct"
        Assert-True ($idGuard.Reason -match "identity_shared") "the refusal must name the shared capture/session identity (got: $($idGuard.Reason))"
        Assert-True ($idGuard.Blocking -eq "five-minute-full-roster") "the identity-duplicate rung must be named as the blocker (got: $($idGuard.Blocking))"

        # 19. FINDING 1 (all-passed short-circuit trusts persisted statuses): a
        #     hand-authored state with all six entries status="passed" and run_dir=null
        #     must NOT be certified complete. Get-LadderCompletionVerdict re-verifies
        #     every rung against its bound artifacts in-process; a null/missing run_dir
        #     is unverifiable -> not complete. Fail-before: a genuinely-passed ladder
        #     (all six run healthily) re-verifies -> complete.
        $completeFile = Join-Path $tmp "all_passed_complete.json"
        $completeState = Read-LadderState -StateFile $completeFile
        foreach ($nm in @("synthetic-fault", "paused-short-proof", "speed5-short-proof", "five-minute-full-roster", "weak-pc-six-core", "six-hour-soak")) {
            Invoke-LadderRung -State $completeState -Name $nm -StateFile $completeFile -CaptureFunction $healthy -RunRoot $runRoot | Out-Null
        }
        $genuine = Get-LadderCompletionVerdict -State $completeState
        Assert-True $genuine.Complete "a genuinely-passed ladder (all six run healthily) must re-verify as complete; unverified: $($genuine.Unverified -join '; ')"
        # The forged all-passed state: hand-write six passed entries with run_dir=null.
        $forgeCompleteFile = Join-Path $tmp "forged_all_passed.json"
        $forgeComplete = New-LadderState
        foreach ($r in $forgeComplete.rungs) {
            $r.status = "passed"; $r.run_dir = $null; $r.verdict_utc = (Get-Date).ToUniversalTime().ToString("o")
        }
        Save-LadderState -State $forgeComplete -StateFile $forgeCompleteFile
        $forgeCompleteState = Read-LadderState -StateFile $forgeCompleteFile
        # Get-NextPendingRungName trusts status -> would report "all passed" (null).
        Assert-True ($null -eq (Get-NextPendingRungName -State $forgeCompleteState)) "the forged state trips the status-only next check (proving the short-circuit would fire)"
        $forgedVerdict = Get-LadderCompletionVerdict -State $forgeCompleteState
        Assert-True (-not $forgedVerdict.Complete) "a forged all-passed state (run_dir=null) must NOT re-verify as complete (finding 1)"
        Assert-True ($forgedVerdict.Unverified.Count -eq 6) "all six forged rungs must be reported unverified"
        Assert-True (($forgedVerdict.Unverified -join "`n") -match "recorded_run_dir_missing") "the unverified reason must name the missing bound run_dir"

        # 20. RE-GATE2 FINDING 1: completion must enforce the SAME cross-rung
        #     distinct-attempt invariants as predecessor ordering. Six DISTINCT
        #     directories cloned from ONE valid attempt (each plan's `rung` rebound,
        #     all sharing one capture_id/session_id, statuses passed) previously
        #     re-verified rung-by-rung and returned Complete=true — while
        #     Test-PredecessorsPassed correctly refused the same state.
        $cloneSrc = Join-Path $runRoot ("clone_src_" + [guid]::NewGuid().ToString("N").Substring(0, 8))
        New-FakeCaptureForRung -RunDir $cloneSrc -RungName "synthetic-fault" -ProofKind "synthetic_fault" | Out-Null
        # Give the source manifest BOTH proof blocks so every per-rung proof
        # reverify holds on the clones — isolating the cross-rung invariant.
        $cloneManifestPath = Join-Path $cloneSrc "native_session_manifest.json"
        $cloneManifest = Get-Content -LiteralPath $cloneManifestPath -Raw | ConvertFrom-Json
        $cloneManifest | Add-Member -NotePropertyName pause_proof -NotePropertyValue ([pscustomobject]@{ observed_paused = $true; source_unavailable_degrade = $true })
        Write-AtomicJson -Path $cloneManifestPath -Value $cloneManifest -Depth 12
        $cloneFile = Join-Path $tmp "cloned_identity_complete.json"
        $cloneState = New-LadderState
        foreach ($r in $cloneState.rungs) {
            $dst = Join-Path $runRoot ("clone_" + $r.name + "_" + [guid]::NewGuid().ToString("N").Substring(0, 8))
            Copy-Item -LiteralPath $cloneSrc -Destination $dst -Recurse -Force
            $dstPlanPath = Join-Path $dst "production_capture_plan.json"
            $dstPlan = Get-Content -LiteralPath $dstPlanPath -Raw | ConvertFrom-Json
            $dstPlan.rung = $r.name
            if ($r.name -eq "six-hour-soak") {
                $dstPlan.recorder_arguments = @("--duration-s", "21600", "--min-game-hours", "48")
                $dstProfilePath = Join-Path $dst "selected_deployment_profile.bin"
                Set-Content -LiteralPath $dstProfilePath -Value '{"profile":"self-test"}' -Encoding UTF8
                $dstPlan | Add-Member -NotePropertyName deployment_profile_source_file -NotePropertyValue "self_test_profile.json"
                $dstPlan | Add-Member -NotePropertyName deployment_profile_file -NotePropertyValue "selected_deployment_profile.bin"
                $dstPlan | Add-Member -NotePropertyName deployment_profile_sha256 -NotePropertyValue ((Get-FileHash -LiteralPath $dstProfilePath -Algorithm SHA256).Hash.ToLowerInvariant())
                $dstManifestPath = Join-Path $dst "native_session_manifest.json"
                $dstManifest = Get-Content -LiteralPath $dstManifestPath -Raw | ConvertFrom-Json
                $dstManifest.duration_ms = 21600000L
                $dstManifest.acceptance_telemetry.clock_skew.wall_elapsed_us = 21600000000L
                $dstManifest.acceptance_telemetry.clock_skew.mono_elapsed_us = 21600000100L
                Write-AtomicJson -Path $dstManifestPath -Value $dstManifest -Depth 12
            }
            $dstPlan | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $dstPlanPath -Encoding UTF8
            if ($r.name -eq "six-hour-soak") { Write-SoakAcceptanceReceipt -RunDir $dst | Out-Null }
            $r.status = "passed"; $r.run_dir = $dst; $r.verdict_utc = (Get-Date).ToUniversalTime().ToString("o")
        }
        Save-LadderState -State $cloneState -StateFile $cloneFile
        $cloneReloaded = Read-LadderState -StateFile $cloneFile
        # Sanity: each clone INDIVIDUALLY re-verifies (else this test would not
        # isolate the cross-rung invariant — the exact re-gate2 reproduction).
        $cloneSoloVerdict = Test-RungBoundVerdict -State $cloneReloaded -Name "six-hour-soak"
        Assert-True $cloneSoloVerdict.Passed "each clone must individually re-verify (test 20 precondition); reason: $($cloneSoloVerdict.Reason)"
        Assert-True (-not (Test-PredecessorsPassed -State $cloneReloaded -Name "six-hour-soak").Ok) "predecessor ordering must refuse the cloned-identity state (verified-good baseline)"
        $cloneVerdict = Get-LadderCompletionVerdict -State $cloneReloaded
        Assert-True (-not $cloneVerdict.Complete) "completion must refuse six rungs cloned from ONE attempt exactly like predecessor ordering (re-gate2 finding 1)"
        Assert-True (($cloneVerdict.Unverified -join "`n") -match "identity_shared") "the completion refusal must name the shared capture/session identity"

        # 21. RE-GATE2 FINDING 2: the informational status display must never print
        #     "[PASS]" from persisted (unvalidated) statuses. Reuse the forged
        #     all-passed state from 19: its display must label recorded passes as
        #     RECORDED-only. (Write-Host emits on the information stream; 6>&1.)
        $statusOut = (Show-LadderStatus -State $forgeCompleteState 6>&1 | Out-String)
        Assert-True ($statusOut -notmatch "\[PASS\]") "status display must not print [PASS] from unvalidated persisted statuses (re-gate2 finding 2)"
        Assert-True ($statusOut -match "RECORDED passed") "status display must label recorded passes as RECORDED-only"
        Assert-True ($statusOut -match "not revalidated") "status display must state that recorded statuses are not revalidated"

        # 22. RE-GATE2 FINDING 5: the fresh-run-dir allocation must REFUSE a
        #     pre-existing candidate path. `New-Item -Force` silently reused the
        #     directory WITHOUT clearing it, so a planted stale_artifact.json
        #     survived into the graded dir.
        $plantedDir = Join-Path $runRoot ("planted_" + [guid]::NewGuid().ToString("N").Substring(0, 8))
        New-Item -ItemType Directory -Path $plantedDir | Out-Null
        Set-Content -LiteralPath (Join-Path $plantedDir "stale_artifact.json") -Value '{"planted":true}' -Encoding UTF8
        $threwPlanted = $false; $plantedMsg = ""
        try { Assert-LadderFreshRunDir -Path $plantedDir } catch { $threwPlanted = $true; $plantedMsg = $_.Exception.Message }
        Assert-True $threwPlanted "allocating a fresh run dir over a pre-existing directory must throw fail-closed (re-gate2 finding 5)"
        Assert-True ($plantedMsg -match "already exists") "the refusal must name the pre-existing candidate (got: $plantedMsg)"
        Assert-True (Test-Path -LiteralPath (Join-Path $plantedDir "stale_artifact.json") -PathType Leaf) "the planted dir must be left untouched (never cleared or adopted)"
        $freshCandidate = Join-Path $runRoot ("fresh_" + [guid]::NewGuid().ToString("N").Substring(0, 8))
        Assert-LadderFreshRunDir -Path $freshCandidate
        Assert-True (Test-Path -LiteralPath $freshCandidate -PathType Container) "a non-existing candidate must be created (pass-after baseline)"
        Assert-True (@(Get-ChildItem -LiteralPath $freshCandidate -Force).Count -eq 0) "the freshly created run dir must be empty"

        # 23. RE-GATE2 FINDING 4: attempt identity must come from GENUINE JSON
        #     strings. A plan with numeric capture_id (123) or a manifest with a
        #     boolean session_id previously string-coerced into a usable identity
        #     ("cap=123|sess=true") — and an unreadable identity SKIPPED dedup
        #     entirely. Wrong type must now refuse fail-closed at the dedup site.
        $badIdFile = Join-Path $tmp "bad_identity_types.json"
        $badIdState = Read-LadderState -StateFile $badIdFile
        $v = Invoke-LadderRung -State $badIdState -Name "synthetic-fault" -StateFile $badIdFile -CaptureFunction $healthy -RunRoot $runRoot
        Assert-True $v.Passed "synthetic-fault should pass before its plan identity is corrupted"
        $badIdDir = [string](Get-StateRungByName -State $badIdState -Name "synthetic-fault").run_dir
        $badPlanPath = Join-Path $badIdDir "production_capture_plan.json"
        $badPlan = Get-Content -LiteralPath $badPlanPath -Raw | ConvertFrom-Json
        $badPlan.capture_id = 123
        $badPlan | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $badPlanPath -Encoding UTF8
        $badGuard = Test-PredecessorsPassed -State $badIdState -Name "paused-short-proof"
        Assert-True (-not $badGuard.Ok) "a numeric plan capture_id must refuse fail-closed, not silently skip identity dedup (re-gate2 finding 4)"
        Assert-True ($badGuard.Reason -match "capture_id_not_a") "the refusal must name the non-string capture_id (got: $($badGuard.Reason))"
        # Variant: boolean manifest session_id (restore the plan's string capture_id
        # first so ONLY the manifest field is wrong-typed).
        $badPlan2 = Get-Content -LiteralPath $badPlanPath -Raw | ConvertFrom-Json
        $badPlan2.capture_id = "capture-restored-string"
        $badPlan2 | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $badPlanPath -Encoding UTF8
        $badManifestPath = Join-Path $badIdDir "native_session_manifest.json"
        $badManifest = Get-Content -LiteralPath $badManifestPath -Raw | ConvertFrom-Json
        $badManifest.session_id = $true
        Write-AtomicJson -Path $badManifestPath -Value $badManifest -Depth 12
        $badGuard2 = Test-PredecessorsPassed -State $badIdState -Name "paused-short-proof"
        Assert-True (-not $badGuard2.Ok) "a boolean manifest session_id must refuse fail-closed (re-gate2 finding 4)"
        Assert-True ($badGuard2.Reason -match "session_id_not_a") "the refusal must name the non-string session_id (got: $($badGuard2.Reason))"

        # 24. CPU-affinity APPLICATION (F6 rung 5 producer): Set-ProcessAffinityFailClosed
        #     actually applies a mask to a live process and VERIFIES the OS honoured it.
        #     Exercised against a THROWAWAY local process (never the game), then killed.
        #     Fail-before: mask was computed (Get-CpuAffinityMask) but never applied;
        #     pass-after: the mask is applied and independently read back.
        $affPsExe = (Get-Process -Id $PID).Path
        if ([string]::IsNullOrEmpty($affPsExe)) { $affPsExe = "powershell.exe" }
        $sleeper = Start-Process -FilePath $affPsExe -ArgumentList @("-NoProfile", "-NonInteractive", "-Command", "Start-Sleep -Seconds 120") -PassThru -WindowStyle Hidden
        try {
            $singleCoreMask = Get-CpuAffinityMask -CoreCount 1   # mask 1 (CPU0): valid on every machine
            $applied = Set-ProcessAffinityFailClosed -ProcessId $sleeper.Id -Mask $singleCoreMask
            Assert-True ($applied -eq 1) "affinity mask 1 must be applied and verified on a throwaway process (got $applied)"
            $independentReadback = [int64]((Get-Process -Id $sleeper.Id).ProcessorAffinity.ToInt64())
            Assert-True ($independentReadback -eq 1) "an independent readback must confirm the applied affinity (got $independentReadback)"
        } finally {
            try { Stop-Process -Id $sleeper.Id -Force -ErrorAction SilentlyContinue } catch {}
        }
        # Fail-closed: applying to an exited/nonexistent process throws (never a silent pass).
        $deadProc = Start-Process -FilePath $affPsExe -ArgumentList @("-NoProfile", "-NonInteractive", "-Command", "exit 0") -PassThru -WindowStyle Hidden
        [void]$deadProc.WaitForExit(10000)
        $threwDeadAff = $false
        try { Set-ProcessAffinityFailClosed -ProcessId $deadProc.Id -Mask 1 | Out-Null } catch { $threwDeadAff = $true }
        Assert-True $threwDeadAff "applying affinity to an exited process must fail closed"
        $threwZeroMask = $false
        try { Set-ProcessAffinityFailClosed -ProcessId $PID -Mask 0 | Out-Null } catch { $threwZeroMask = $true }
        Assert-True $threwZeroMask "a non-positive affinity mask must be rejected"

        # 25. Bound-report pin plumbing: Get-SafeLauncherPins validates the owner pin set
        #     fail-closed. A complete pins file parses; a missing required pin or a missing
        #     file throws naming the fault. No pin is ever fabricated.
        $pinsGood = [ordered]@{
            Hoi4Pid                            = 4321
            SpeedEvidenceTranscriptPath        = (Join-Path $tmp "transcript.json")
            SpeedEvidenceTranscriptSha256      = ('a' * 64)
            SpeedEvidenceTranscriptFileSha256  = ('b' * 64)
            SpeedEvidenceSessionId             = "sess-01234567"
            SpeedEvidenceNonce                 = ('c' * 64)
            SpeedEvidenceNonceLedgerRoot       = $tmp
            SpeedStaticDisasmSha256            = ('d' * 64)
            StaticBundleDir                    = (Join-Path $tmp "bundle")
            ModStatePath                       = (Join-Path $tmp "mod.json")
            DlcStatePath                       = (Join-Path $tmp "dlc.json")
            LoadOrderPath                      = (Join-Path $tmp "load_order.json")
            SourceInventoryPath                = (Join-Path $tmp "source_inventory.json")
            LiveResolutionReportPath           = (Join-Path $tmp "live_resolution.json")
            OutDir                             = "D:\hoi4_local_captures\rung"
            DurationSeconds                    = 300
            AffinityTarget                     = "game"
        }
        $pinsPath = Join-Path $tmp "pins_good.json"
        $pinsGood | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $pinsPath -Encoding UTF8
        $parsedPins = Get-SafeLauncherPins -PinsFile $pinsPath
        Assert-True ([int64](Get-ObjectProperty $parsedPins "Hoi4Pid") -eq 4321) "a complete pins file must parse and carry Hoi4Pid"
        $threwMissingFile = $false
        try { Get-SafeLauncherPins -PinsFile (Join-Path $tmp "no_such_pins.json") | Out-Null } catch { $threwMissingFile = $true }
        Assert-True $threwMissingFile "a missing pins file must fail closed"
        $pinsMissing = [ordered]@{}; foreach ($k in $pinsGood.Keys) { $pinsMissing[$k] = $pinsGood[$k] }; $pinsMissing.Remove("SpeedEvidenceNonce")
        $pinsMissingPath = Join-Path $tmp "pins_missing.json"
        $pinsMissing | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $pinsMissingPath -Encoding UTF8
        $threwMissingPin = $false; $missingPinMsg = ""
        try { Get-SafeLauncherPins -PinsFile $pinsMissingPath | Out-Null } catch { $threwMissingPin = $true; $missingPinMsg = $_.Exception.Message }
        Assert-True $threwMissingPin "a pins file missing a required pin must fail closed"
        Assert-True ($missingPinMsg -match "SpeedEvidenceNonce") "the refusal must name the missing pin (got: $missingPinMsg)"
        $pinsBadPid = [ordered]@{}; foreach ($k in $pinsGood.Keys) { $pinsBadPid[$k] = $pinsGood[$k] }; $pinsBadPid.Hoi4Pid = 0
        $pinsBadPidPath = Join-Path $tmp "pins_badpid.json"
        $pinsBadPid | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $pinsBadPidPath -Encoding UTF8
        $threwBadPid = $false
        try { Get-SafeLauncherPins -PinsFile $pinsBadPidPath | Out-Null } catch { $threwBadPid = $true }
        Assert-True $threwBadPid "a non-positive Hoi4Pid must fail closed"

        # 26. Bound-report launcher arg assembly: Get-SafeLauncherArgs threads the full
        #     mandatory pin set + rung binding + OutDir into the Safe-launcher argument
        #     array, and forwards optional knobs that are present.
        $speed5Def = @(Get-LadderRungDefinitions | Where-Object { $_.name -eq "speed5-short-proof" })[0]
        $safeArgsAssembled = Get-SafeLauncherArgs -RungDef $speed5Def -CaptureScript "C:\some path\Start-Hoi4ProductionCaptureSafe.ps1" -Pins $parsedPins -RunDir "D:\hoi4_local_captures\rung"
        Assert-True ($safeArgsAssembled -contains "-Rung") "assembled args must bind the rung"
        Assert-True ($safeArgsAssembled -contains "speed5-short-proof") "assembled args must name this rung"
        Assert-True ($safeArgsAssembled -contains "-Hoi4Pid" -and $safeArgsAssembled -contains "4321") "assembled args must carry the pinned Hoi4Pid"
        Assert-True ($safeArgsAssembled -contains "-OutDir" -and ($safeArgsAssembled -contains "D:\hoi4_local_captures\rung")) "assembled args must carry the owner-chosen OutDir"
        foreach ($needPin in @("-SpeedEvidenceTranscriptPath", "-SpeedEvidenceNonce", "-SpeedEvidenceNonceLedgerRoot", "-SpeedStaticDisasmSha256", "-StaticBundleDir", "-ModStatePath", "-DlcStatePath", "-LoadOrderPath", "-SourceInventoryPath", "-LiveResolutionReportPath")) {
            Assert-True ($safeArgsAssembled -contains $needPin) "assembled args must include mandatory pin flag $needPin"
        }
        Assert-True ($safeArgsAssembled -contains "-DurationSeconds" -and $safeArgsAssembled -contains "300") "an optional knob present in the pins must be forwarded"
        Assert-True ($safeArgsAssembled -contains "-AllowExistingEmptyOutDir") "arg assembly must explicitly hand off the ladder's fresh empty run directory"
        Assert-True ($safeArgsAssembled -contains "-SkipBuild") "each rung must reuse the final prebuilt binaries instead of compiling per rung"

        $soakDef = @(Get-LadderRungDefinitions | Where-Object { $_.name -eq "six-hour-soak" })[0]
        $shortSoakPins = ($parsedPins | ConvertTo-Json -Depth 8 | ConvertFrom-Json)
        $shortSoakPins | Add-Member -NotePropertyName DeploymentProfilePath -NotePropertyValue (Join-Path $tmp "selected_deployment_profile.json")
        $threwShortSoak = $false
        try { Get-SafeLauncherArgs -RungDef $soakDef -CaptureScript "C:\capture.ps1" -Pins $shortSoakPins -RunDir "D:\soak" | Out-Null }
        catch { $threwShortSoak = $true }
        Assert-True $threwShortSoak "six-hour soak must reject a 300-second pins override"
        $soakPins = ($parsedPins | ConvertTo-Json -Depth 8 | ConvertFrom-Json)
        $soakPins.DurationSeconds = 21600
        $soakPins | Add-Member -NotePropertyName MinGameHours -NotePropertyValue 48
        $soakPins | Add-Member -NotePropertyName DeploymentProfilePath -NotePropertyValue (Join-Path $tmp "selected_deployment_profile.json")
        $soakArgs = Get-SafeLauncherArgs -RungDef $soakDef -CaptureScript "C:\capture.ps1" -Pins $soakPins -RunDir "D:\soak"
        Assert-True ($soakArgs -contains "21600") "six-hour soak must carry an explicit >=21600-second duration"
        Assert-True ($soakArgs -contains "-DeploymentProfilePath") "six-hour soak must bind the selected deployment profile"

        # 27. Owed-producer mapping post-wiring: all rungs have a Safe-launcher
        #     producer plus independent in-process re-grade.
        foreach ($gradedName in @("synthetic-fault", "paused-short-proof", "speed5-short-proof", "five-minute-full-roster", "weak-pc-six-core", "six-hour-soak")) {
            $gDef = @(Get-LadderRungDefinitions | Where-Object { $_.name -eq $gradedName })[0]
            $gOwed = Get-RungBoundProducerOwed -RungDef $gDef
            Assert-True ($null -eq $gOwed) "$gradedName must have a bound producer and independent in-process re-grade"
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
        # Fail-closed (finding 1): every rung's `status` says passed, but that field is
        # hand-editable. Do NOT declare the ladder complete on the persisted statuses
        # alone — re-verify every rung against its bound run_dir artifacts through the
        # authoritative in-process validator. A forged all-passed state (status=passed,
        # run_dir=null) must ERROR here, not silently exit 0.
        $completion = Get-LadderCompletionVerdict -State $state
        if (-not $completion.Complete) {
            throw ("refusing to certify the ladder complete: $($completion.Unverified.Count) rung(s) claim passed but do not re-verify against their bound artifacts (fail-closed): " + ($completion.Unverified -join '; '))
        }
        Write-Host "all ladder rungs already passed (re-verified in-process against their bound artifacts); nothing to run."
        exit 0
    }
}

if (-not $Live) {
    throw "refusing to run rung '$targetRung' without -Live: launching the game is owner-gated. Re-run with -Live once the game is up, or use -SelfTest for the offline machinery checks."
}

$runRoot = $RunRoot
if ([string]::IsNullOrWhiteSpace($runRoot)) {
    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        throw "LOCALAPPDATA is unavailable; pass -RunRoot with a local non-repository capture root"
    }
    $runRoot = Join-Path $env:LOCALAPPDATA "Hoi4MpStats\acceptance_ladder_runs"
}
if (-not (Test-Path -LiteralPath $runRoot -PathType Container)) {
    New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
}
$verdict = Invoke-LadderRung -State $state -Name $targetRung -StateFile $StateFile `
    -CaptureFunction ${function:Invoke-LiveCapture} -RunRoot $runRoot
Show-LadderStatus -State $state
if (-not $verdict.Passed) { exit 1 }
exit 0
