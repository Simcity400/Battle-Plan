<#
.SYNOPSIS
    The world-readiness wait, factored out so it can be tested without a game.

.DESCRIPTION
    Dot-source this file from capture launchers. It performs no process control,
    opens no handle to the game and reads no engine memory: the caller injects the
    observation, the liveness probe, the clock and the sleep, and this decides
    when the observed campaign is ready and what to say when it never is.

    WHAT "READY" MEANS, AND WHY IT CHANGED.

    The harness used to treat "the world singletons are readable and coherent"
    as readiness. That is strictly weaker than "the campaign is in play", and
    the difference is a whole savegame load wide.

    CGameState+0xA39 is a bool member of CGameState. What identifies the object
    is that each of the six setter call sites below is immediately preceded by
    the gamestate.h accessor assert pair --
    `_pInstance && "gamestate unitilialized"` and
    `_ThreadForbidCount == 0 && "Current thread is forbidden to access gamestate"`
    -- and loads its `this` from DAT_143316E90. That is a fact about those six
    sites; the 216 compare-shaped accesses to the same displacement elsewhere in
    .text were not individually attributed to an object and are not claimed to
    be CGameState.

    Enumerated from the pinned hoi4.exe (SHA-256 870ebf1f...c65ad), .text holds
    exactly four direct disp32 writes to a +0xA39 displacement, and exactly two
    of them are on the CGameState singleton DAT_143316E90:

      * 0x1401E01BA, in FUN_1401E0190: `mov byte [rcx+0xa39], r12b` with
        r12d = 0, beside `[rcx+0xa34] = 0` and `[rcx+0xa30] = 0`. The member is
        ZERO at construction/reset.
      * 0x1401EDC74, in FUN_1401EDC70: `mov byte [rcx+0xa39], dl` -- the
        generated `set(bool)` accessor. Six call sites, every one of them
        loading rcx from DAT_143316E90:

            FUN_1401A5120  dl=0   gameapplication.cpp, "Resetting game"
            FUN_140B2E880  dl=0   graphics/frontend.cpp, LOAD_GFX / InitMap
            FUN_140DB6BE0  dl=0   (virtual; no direct callers)
            FUN_140DBF240  dl=0   LOADING_MAIN_MENU / EXITING_TO_FRONTEND
            FUN_140DC5DC0  dl=0   LOADING_SAVEGAME_NO_NAME -- load begins
            FUN_140DC5310  dl=1   ingameidler.cpp -- the in-game session setup

    The other two writes (0x140259C74 in the "Unlocked faction military"
    console handler, 0x1404A9F9A on an object returned by FUN_141174150) reach
    +0xA39 of a DIFFERENT object; neither function loads DAT_143316E90 into the
    base register it stores through.

    WHAT THAT SEARCH DOES AND DOES NOT COVER. It is exhaustive for a DIRECT
    disp32 access from a register holding the object pointer, because any
    displacement above 0x7F must be encoded that way, and +0xA32..+0xA38 were
    swept as well so an overlapping wider scalar store would have shown up. It
    does NOT cover a write through an interior pointer, a bulk copy over the
    region, or a wide vector store. The list above is therefore the ENUMERATED
    writer set, not a proved exhaustive one.

    WHAT THE BYTE MEANS, STATED NO MORE STRONGLY THAN THE EVIDENCE ALLOWS.
    FUN_140DC5310 sets it at its own entry (0x140DC543D) and only THEN performs
    the ~35 further setup calls that end with the load's pause write to
    CInGameIdler+0x6C1. So a set byte says the engine has ENTERED its in-game
    session setup and holds the session eligible for player input -- which is
    exactly what FUN_140B58C10 tests, and exactly the precondition the player
    pause toggle FUN_140DCD3E0 needs. It does NOT say the engine has finished
    its post-load work. That is a different condition, and the harness has a
    separate phase for it: settle, which requires observed clock advancement.

    A campaign that is still deserialising presents readable, coherent
    CGameState and CInGameIdler singletons, a deserialised speed and a pause
    latch, and a ZERO gate. Admitting that as readiness is what let the harness
    reach the unpause step mid-load, where Control-Hoi4Game.ps1 correctly
    refuses to mirror a transition the engine would decline.

    WHAT THIS IS NOT. Waiting does not make the refusal weaker. A gate that
    never arrives inside the budget is still a refusal -- it just says how long
    it waited instead of reporting a single unlucky sample as a permanent state.

    NOTE ON DIRECTION. Non-zero does not prove the toggle will transition:
    FUN_140B58C10 has four call predicates this cannot evaluate, and the session
    must not be a CNetworkServer or CProxyServer. Control-Hoi4Game.ps1 still
    checks and reports all of that. This only removes the case where the gate is
    zero for the ordinary reason that the campaign had not finished loading.
#>

Set-StrictMode -Version Latest

<#
.SYNOPSIS
    The observed value of the engine's in-game-session latch.

.DESCRIPTION
    Throws when the observation does not carry the field at all. An observer
    that stopped reporting the gate must not be readable as a set gate: that
    would restore the very defect this wait exists to close, with every guard
    still green.
#>
function Get-Hoi4PlayerInputGate {
    param(
        [Parameter(Mandatory = $true)]
        [AllowNull()]
        [object]$State
    )

    if ($null -eq $State) {
        throw 'Cannot read the player-input gate from a null observation.'
    }
    if (@($State.PSObject.Properties.Name) -notcontains 'PlayerInputGate') {
        throw (
            'The read-only observation carries no PlayerInputGate field, so the ' +
            'engine in-game-session latch at CGameState+0xA39 was never sampled. ' +
            'Readiness cannot be established from an observation that does not ' +
            'contain it.'
        )
    }
    $raw = $State.PlayerInputGate
    if ($null -eq $raw) {
        throw 'The read-only observation reported a null PlayerInputGate.'
    }
    return [int]$raw
}

<#
.SYNOPSIS
    True when the observation shows the engine's in-game session live.
#>
function Test-Hoi4PlayerInputGateSet {
    param(
        [Parameter(Mandatory = $true)]
        [AllowNull()]
        [object]$State
    )

    if ($null -eq $State) {
        return $false
    }
    return ((Get-Hoi4PlayerInputGate -State $State) -ne 0)
}

<#
.SYNOPSIS
    The refusal raised when the budget runs out, in the two shapes it has.

.DESCRIPTION
    The two outcomes are different diagnoses and must not collapse into one
    message. "No coherent observation ever came back" points at the process or
    the offsets; "observations came back and none of them saw the session live"
    points at the campaign, and is the one that names how long it waited.

    The second message is worded to claim only what polling establishes. It
    cannot say the byte was continuously zero -- nothing sampled the engine
    between observations, or after the last one -- so every statement in it is
    scoped to the observations that were actually taken.

    It does NOT enumerate the histories that fit the evidence, because that set
    is not enumerable: the byte may have been set and cleared any number of
    times while unsampled, including entirely before the first observation or
    entirely after the last. Two review rounds were spent adding a missing case
    to a list that could never be complete. The message gives examples and then
    states the only sound conclusion, which is that the current value is
    unknown. That is also why the refusal is attributed to the state LAST
    OBSERVED rather than to the engine now: a present-tense claim about the
    toggle would be exactly the unsound step.
#>
function New-Hoi4WorldReadyTimeoutMessage {
    param(
        [Parameter(Mandatory = $true)][int]$TargetProcessId,
        [Parameter(Mandatory = $true)][int]$TimeoutSeconds,
        [Parameter(Mandatory = $true)][int]$WaitedSeconds,
        [Parameter(Mandatory = $true)][bool]$SawObservation,
        [AllowNull()][object]$LastPlayerInputGate,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$LastError,
        [AllowEmptyString()][string]$StallDescription = ''
    )

    if (-not $SawObservation) {
        # The caller's extra evidence goes HERE, on the arm that says nothing
        # about why. "Did not expose stable world singletons" reads as an
        # offsets or process fault, and in t43/t44 it was neither -- the game
        # was at the frontend because it had declined the continue selection,
        # and nothing in this message pointed there.
        $extra = if ([string]::IsNullOrWhiteSpace($StallDescription)) {
            ''
        } else {
            " $StallDescription"
        }
        return (
            "the game PID $TargetProcessId did not expose stable world singletons within " +
            "$TimeoutSeconds s. Last probe error: $LastError$extra"
        )
    }
    $observed = if ($null -eq $LastPlayerInputGate) { 'unknown' } else { [string]$LastPlayerInputGate }
    return (
        "the game PID $TargetProcessId exposed stable world singletons, but no observation saw " +
        "its in-game-session latch CGameState+0xA39 set: waited $WaitedSeconds s and it " +
        "never became non-zero in any observation taken in that window " +
        "(budget $TimeoutSeconds s, last observed value $observed). Exactly one " +
        'enumerated engine routine sets that byte -- FUN_140DC5310, the in-game session ' +
        'setup -- and the savegame-load, frontend and reset paths clear it. What is ' +
        'established is only that no observation saw it set. Nothing sampled the engine ' +
        'between observations or after the last one, so the byte may have transitioned any ' +
        'number of times while unsampled -- the campaign may never have entered its ' +
        'in-game session, may have entered and left again, or may be in play right now -- ' +
        'and its current value is unknown. The refusal is on the state LAST OBSERVED -- at that value ' +
        'FUN_140B58C10 returns non-zero and the player toggle FUN_140DCD3E0 returns at ' +
        '0x140DCD409 without transitioning -- and is not a claim about the engine now.'
    )
}

<#
.SYNOPSIS
    Wait, bounded, for a campaign that is loaded AND in play.

.PARAMETER Observe
    Called with the remaining whole seconds. Returns the read-only observation,
    returns $null when it produced none, or throws while the singletons are not
    yet coherent.

.PARAMETER TestAlive
    Returns $false once the target process is gone.

.PARAMETER ReportProgress
    Called with a hashtable describing the wait so the caller can log it. The
    wait itself prints nothing.

.PARAMETER DescribeStall
    Optional. Called with the whole seconds waited, ONLY while building the
    timeout message, and only when no observation ever succeeded. Returns extra
    text to append to that message, or an empty string for "nothing to add".

    IT CANNOT SHORTEN OR FAIL THE WAIT, and that is the whole design. An
    earlier version of this parameter could: it let the caller refuse early when
    the engine's own log showed no session launch banner. That was wrong, and
    the reason is worth keeping. A campaign that is loading and a game parked at
    the frontend with its continue selection declined look identical from here
    -- no readable singletons, gate never seen set -- and every discriminator
    available off disk turned out to be a proxy. "No banner yet" is true of a
    slow load. "No banner and the log has gone quiet" is true of a silent
    deserialize, which a review reproduced against these very functions: a
    healthy campaign that became ready after six seconds was refused after
    three and a half. A modset that renames or suppresses the banner fails the
    same way.

    Refusing a healthy capture is strictly worse than the misleading timeout
    this was meant to improve on, so the diagnosis stays and the refusal goes.
    Closing the fast half needs an affirmative, engine-proven "this process is
    at the frontend menu" observable, which does not exist in this tree today.

    The callback runs inside the timeout path, so a throw from it is caught and
    reported rather than replacing the real refusal.

.OUTPUTS
    The FIRST observation whose gate is set. Never an observation with a zero
    gate: the only other exits are throws.

.NOTES
    HOW FAR PAST THE BUDGET THIS CAN RUN, with the production delegates.

    The loop condition is checked before each iteration, so the last iteration
    can begin just under the deadline. Inside it:

      * the observation is allowed ceil(remaining) seconds, clamped to 1..30, so
        within one second of the deadline its own budget is 1 s. The rounding
        alone therefore contributes < 1 s;
      * Invoke-BoundedNativeCapture enforces that budget itself. On NATURAL
        child exit it then waits <= 1 s for the redirected streams to flush. On
        EXPIRY it instead calls Stop-ExactProcess (<= 10 s) and throws, which
        skips the flush -- but the throw runs its finally, and if the child is
        somehow still alive there, Stop-ExactProcess runs a SECOND time for
        another <= 10 s;
      * the poll sleep adds 0.75 s.

    So the coded waits give deadline + < 2.75 s when the last child exits on its
    own, and deadline + < 21.75 s in the pathological case where the child
    overruns its budget AND survives the first termination attempt, on a 600 s
    default budget. Get-Process, Process.Kill and the process-epoch assertions
    are not separately bounded, so this is a bound on the coded waits rather
    than on the wall clock.

    A SUCCESSFUL observation is returned without re-checking the deadline, and
    that is deliberate: the budget bounds how long the harness WAITS, not how
    late a genuinely live session is allowed to be believed. Refusing a world
    that just became ready because a clock ticked over would discard a correct
    observation for no gain, and the phase behaved this way before this change.

    Termination depends on the injected delegates advancing the injected clock.
    The production caller passes the real clock and Start-Sleep; the tests pass
    a virtual clock that only advances inside Sleep. Both terminate.
#>
function Wait-Hoi4WorldReadyState {
    param(
        [Parameter(Mandatory = $true)][int]$TargetProcessId,
        [Parameter(Mandatory = $true)][ValidateRange(1, 86400)][int]$TimeoutSeconds,
        [Parameter(Mandatory = $true)][scriptblock]$Observe,
        [Parameter(Mandatory = $true)][scriptblock]$TestAlive,
        [AllowNull()][scriptblock]$ReportProgress,
        [AllowNull()][scriptblock]$DescribeStall,
        [AllowNull()][scriptblock]$Sleep,
        [AllowNull()][scriptblock]$UtcNow,
        [ValidateRange(1, 60000)][int]$PollMilliseconds = 750,
        [ValidateRange(1, 3600)][int]$ProgressIntervalSeconds = 10
    )

    if ($null -eq $UtcNow) { $UtcNow = { [DateTime]::UtcNow } }
    if ($null -eq $Sleep) { $Sleep = { param($ms) Start-Sleep -Milliseconds $ms } }

    $startedAt = & $UtcNow
    $deadline = $startedAt.AddSeconds($TimeoutSeconds)
    $lastError = 'not attempted'
    $sawObservation = $false
    $lastGate = $null
    $nextProgress = $startedAt

    while ((& $UtcNow) -lt $deadline) {
        if (-not (& $TestAlive)) {
            throw "the game PID $TargetProcessId exited before the world became ready."
        }
        $now = & $UtcNow
        $remainingSeconds = [Math]::Max(
            1,
            [Math]::Min(30, [Math]::Ceiling(($deadline - $now).TotalSeconds))
        )
        $state = $null
        try {
            $state = & $Observe $remainingSeconds
        } catch {
            # The singletons do not exist until the save is loaded, and an
            # observation taken across a load boundary can fail its own identity
            # re-check. Both are ordinary inside this wait, and only these are.
            $lastError = $_.Exception.Message
        }
        if ($null -ne $state) {
            $sawObservation = $true
            # OUTSIDE the retry catch, deliberately. A returned observation that
            # does not carry the gate is a wiring fault, not a slow load, and no
            # amount of waiting repairs it. Swallowing it here would spend the
            # whole budget and then blame the campaign for never entering play.
            $lastGate = Get-Hoi4PlayerInputGate -State $state
            if ($lastGate -ne 0) {
                return $state
            }
        }
        $now = & $UtcNow
        if ($null -ne $ReportProgress -and $now -ge $nextProgress) {
            & $ReportProgress @{
                TargetProcessId = $TargetProcessId
                WaitedSeconds = [int][Math]::Floor(($now - $startedAt).TotalSeconds)
                RemainingSeconds = [int][Math]::Max(
                    0, [Math]::Ceiling(($deadline - $now).TotalSeconds)
                )
                SawObservation = $sawObservation
                PlayerInputGate = $lastGate
                LastError = $lastError
            }
            $nextProgress = $now.AddSeconds($ProgressIntervalSeconds)
        }
        & $Sleep $PollMilliseconds
    }

    $waited = [int][Math]::Floor(((& $UtcNow) - $startedAt).TotalSeconds)
    # Only on the arm where the caller's extra evidence is relevant: an hour
    # spent with no coherent observation at all is the arm that used to blame
    # the singletons for a selection the engine declined.
    $stall = ''
    if ($null -ne $DescribeStall -and -not $sawObservation) {
        try {
            $stall = [string](& $DescribeStall $waited)
        } catch {
            # The describer is decoration on a refusal that is already
            # happening. Its failure must not replace the real cause, so it is
            # reported in place of what it would have said.
            $stall = "(stall description failed: $($_.Exception.Message))"
        }
    }
    throw (New-Hoi4WorldReadyTimeoutMessage `
        -TargetProcessId $TargetProcessId `
        -TimeoutSeconds $TimeoutSeconds `
        -WaitedSeconds $waited `
        -SawObservation $sawObservation `
        -LastPlayerInputGate $lastGate `
        -LastError $lastError `
        -StallDescription $stall)
}
