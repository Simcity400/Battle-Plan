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

    CGameState+0xA39 is a bool member of CGameState -- every access to it goes
    through the gamestate.h accessor whose assert pair is
    `_pInstance && "gamestate unitilialized"` and
    `_ThreadForbidCount == 0 && "Current thread is forbidden to access gamestate"`.
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

    So the byte is a one-way, post-load latch: cleared on every transition INTO
    loading, the frontend or a reset, and set by exactly one routine --
    FUN_140DC5310, which sets it at its own entry (0x140DC543D) and only then
    performs the ~35 further setup calls that end with the load's pause write to
    CInGameIdler+0x6C1. It is the engine's own statement that the in-game
    session is live, and FUN_140B58C10 -- which gates the player pause toggle
    FUN_140DCD3E0 -- tests it as the second term of its conjunction.

    A campaign that is still deserialising therefore presents readable, coherent
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
    the offsets; "observations came back and the session never went live" points
    at the campaign, and is the one that names how long it waited.
#>
function New-Hoi4WorldReadyTimeoutMessage {
    param(
        [Parameter(Mandatory = $true)][int]$TargetProcessId,
        [Parameter(Mandatory = $true)][int]$TimeoutSeconds,
        [Parameter(Mandatory = $true)][int]$WaitedSeconds,
        [Parameter(Mandatory = $true)][bool]$SawObservation,
        [AllowNull()][object]$LastPlayerInputGate,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string]$LastError
    )

    if (-not $SawObservation) {
        return (
            "the game PID $TargetProcessId did not expose stable world singletons within " +
            "$TimeoutSeconds s. Last probe error: $LastError"
        )
    }
    $observed = if ($null -eq $LastPlayerInputGate) { 'unknown' } else { [string]$LastPlayerInputGate }
    return (
        "the game PID $TargetProcessId exposed stable world singletons, but its in-game-session " +
        "latch CGameState+0xA39 never left zero: waited $WaitedSeconds s and it never became " +
        "non-zero (budget $TimeoutSeconds s, last observed value $observed). Exactly one " +
        'engine routine sets that byte -- FUN_140DC5310, the in-game session setup -- and the ' +
        'savegame-load, frontend and reset paths clear it, so the campaign never finished ' +
        'entering play. Driving the client from here is refused, not deferred: the player ' +
        'toggle FUN_140DCD3E0 would return at 0x140DCD409 without transitioning.'
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

.OUTPUTS
    The FIRST observation whose gate is set. Never an observation with a zero
    gate: the only other exits are throws.
#>
function Wait-Hoi4WorldReadyState {
    param(
        [Parameter(Mandatory = $true)][int]$TargetProcessId,
        [Parameter(Mandatory = $true)][ValidateRange(1, 86400)][int]$TimeoutSeconds,
        [Parameter(Mandatory = $true)][scriptblock]$Observe,
        [Parameter(Mandatory = $true)][scriptblock]$TestAlive,
        [AllowNull()][scriptblock]$ReportProgress,
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
    throw (New-Hoi4WorldReadyTimeoutMessage `
        -TargetProcessId $TargetProcessId `
        -TimeoutSeconds $TimeoutSeconds `
        -WaitedSeconds $waited `
        -SawObservation $sawObservation `
        -LastPlayerInputGate $lastGate `
        -LastError $lastError)
}
