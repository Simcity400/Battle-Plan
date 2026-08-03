[CmdletBinding()]
param(
    [Parameter()]
    [ValidateRange(1, 2147483647)]
    [int]$TargetPid,

    # How long to keep looking for the engine's hour to move before reporting
    # that it did not. The previous fixed 750 ms single-shot window decided
    # "advancing" from one strict inequality; at UI speed 5 the observed hour
    # delta over such a window ranged 3..11, so 0 sat inside the ordinary
    # distribution and a healthy game was regularly declared stopped. This is a
    # budget, not a dwell: the loop returns the instant the hour moves, so a
    # healthy sample is now faster than the old fixed window, and only a clock
    # that really is not moving pays the full budget.
    [ValidateRange(250, 60000)]
    [int]$ClockProgressBudgetMs = 3000,

    [ValidateRange(1, 1000)]
    [int]$ClockProgressPollMs = 25
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$gameStateSingletonRva = 0x3316E90L
$gameHourOffset = 0x468L
$gameTickProgressOffset = 0x4B8L
$gameSpeedOffset = 0x4BCL
# Byte gate on the tick-dispatch branch of the pacing function FUN_1401F0210:
#   if (*(char *)(CGameState + 0x4C0) == 0 && 1.0f <= progress) { dispatch tick }
# Non-zero holds the hour still while pause (CInGameIdler+0x6C1) stays clear, so
# it is a distinct engine state from a pause and must be sampled separately.
$gameTickInhibitOffset = 0x4C0L
# The engine's own in-game-session latch, and the SAME offset
# Control-Hoi4Game.ps1 refuses on as $gameStatePlayerInputGateOffset. Reported
# here, never gated on: this helper is the read-only witness, so it states what
# the byte is and the harness decides what that means.
#
# A bool member of CGameState, reached only through the gamestate.h accessor.
# .text holds four direct disp32 writes to a +0xA39 displacement and exactly two
# are on DAT_143316E90: FUN_1401E0190 zeroes it at construction, and the
# generated setter FUN_1401EDC70 is called from six sites -- five clearing it
# (game reset, frontend InitMap, exit-to-frontend, LOADING_SAVEGAME_NO_NAME, and
# one virtual) and exactly one setting it, FUN_140DC5310, the ingameidler.cpp
# in-game session setup, at its own entry 0x140DC543D. So zero here means the
# campaign is loading, at the frontend or resetting -- NOT that it is in play,
# which the singleton checks above cannot distinguish.
$gameStatePlayerInputGateOffset = 0xA39L
$clockSingletonARva = 0x33172E0L
$clockSingletonBRva = 0x33172E8L
$clockVtableRva = 0x2952510L
$clockPausedOffset = 0x6C1L
# The engine's forced-pause flag. FUN_140DBE260 and the forced arm of
# FUN_140DC6520 set +0x6C1 and +0x6C3 together; the player's own toggle arm of
# FUN_140DC6520 runs only when +0x6C3 is clear and moves +0x6C1 alone. So
# paused && forced is an engine-initiated pause, and paused && !forced is not.
$clockPauseForcedOffset = 0x6C3L

$nativeSource = @'
using System;
using System.Runtime.InteropServices;

public static class Hoi4ReadOnlyStateNative
{
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern IntPtr OpenProcess(
        UInt32 desiredAccess,
        [MarshalAs(UnmanagedType.Bool)] bool inheritHandle,
        Int32 processId);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool ReadProcessMemory(
        IntPtr process,
        IntPtr address,
        byte[] buffer,
        UIntPtr size,
        out UIntPtr bytesRead);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool CloseHandle(IntPtr handle);
}
'@

if (-not ('Hoi4ReadOnlyStateNative' -as [type])) {
    Add-Type -TypeDefinition $nativeSource
}

if (-not $PSBoundParameters.ContainsKey('TargetPid')) {
    $candidates = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
    if ($candidates.Count -ne 1) {
        throw "Expected exactly one the game process, found $($candidates.Count); pass -TargetPid."
    }
    $TargetPid = $candidates[0].Id
}

$process = Get-Process -Id $TargetPid -ErrorAction Stop
$exePath = $process.MainModule.FileName
$actualExeSha256 = (
    Get-FileHash -LiteralPath $exePath -Algorithm SHA256
).Hash.ToLowerInvariant()
if ($actualExeSha256 -cne $expectedExeSha256) {
    throw "Refusing to observe PID ${TargetPid}: unsupported hoi4.exe SHA-256 $actualExeSha256."
}

# This helper requests only query/read access. It is the independent
# graded-window witness and contains no write-capable P/Invoke.
$processQueryInformation = 0x0400
$processVmRead = 0x0010
$desiredAccess = $processQueryInformation -bor $processVmRead
$processHandle = [Hoi4ReadOnlyStateNative]::OpenProcess(
    [uint32]$desiredAccess,
    $false,
    $TargetPid)
if ($processHandle -eq [IntPtr]::Zero) {
    $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    throw "OpenProcess($TargetPid, QUERY|VM_READ) failed with Win32 error $errorCode."
}

function Read-RemoteBytes {
    param(
        [Parameter(Mandatory)][IntPtr]$ProcessHandle,
        [Parameter(Mandatory)][long]$Address,
        [Parameter(Mandatory)][ValidateRange(1, 8)][int]$Count
    )

    $buffer = [byte[]]::new($Count)
    $bytesRead = [UIntPtr]::Zero
    $ok = [Hoi4ReadOnlyStateNative]::ReadProcessMemory(
        $ProcessHandle,
        [IntPtr]$Address,
        $buffer,
        [UIntPtr]::new([uint64]$Count),
        [ref]$bytesRead)
    if (-not $ok) {
        $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        throw ('ReadProcessMemory(0x{0:X}) failed with Win32 error {1}.' -f
            $Address, $errorCode)
    }
    if ($bytesRead.ToUInt64() -ne [uint64]$Count) {
        throw ('ReadProcessMemory(0x{0:X}) returned {1} of {2} bytes.' -f
            $Address, $bytesRead.ToUInt64(), $Count)
    }
    return $buffer
}

function Read-RemoteInt32 {
    param([IntPtr]$ProcessHandle, [long]$Address)
    return [BitConverter]::ToInt32(
        (Read-RemoteBytes $ProcessHandle $Address 4),
        0)
}

function Read-RemoteInt64 {
    param([IntPtr]$ProcessHandle, [long]$Address)
    return [BitConverter]::ToInt64(
        (Read-RemoteBytes $ProcessHandle $Address 8),
        0)
}

function Read-RemoteByte {
    param([IntPtr]$ProcessHandle, [long]$Address)
    return (Read-RemoteBytes $ProcessHandle $Address 1)[0]
}

try {
    $moduleBase = $process.MainModule.BaseAddress.ToInt64()
    $gameState = Read-RemoteInt64 `
        $processHandle `
        ($moduleBase + $gameStateSingletonRva)
    $stableGameState = Read-RemoteInt64 `
        $processHandle `
        ($moduleBase + $gameStateSingletonRva)
    if ($gameState -eq 0 -or
        $gameState -ne $stableGameState -or
        ($gameState -band 7) -ne 0) {
        throw 'CGameState singleton is null, unstable, or misaligned.'
    }

    $clockA = Read-RemoteInt64 `
        $processHandle `
        ($moduleBase + $clockSingletonARva)
    $clockB = Read-RemoteInt64 `
        $processHandle `
        ($moduleBase + $clockSingletonBRva)
    if ($clockA -eq 0 -or $clockA -ne $clockB -or ($clockA -band 7) -ne 0) {
        throw 'CInGameIdler singleton pair is null, incoherent, or misaligned.'
    }
    $clockVtable = Read-RemoteInt64 $processHandle $clockA
    $expectedClockVtable = $moduleBase + $clockVtableRva
    if ($clockVtable -ne $expectedClockVtable) {
        throw ('CInGameIdler vtable mismatch: expected 0x{0:X}, observed 0x{1:X}.' -f
            $expectedClockVtable, $clockVtable)
    }

    # Two timers, because two different questions are being asked.
    #
    # The BRACKET timer says which stretch of wall clock this sample's evidence
    # covers. It opens before the first read that feeds the sample and closes
    # after the last one, so a hold raised outside the window can never be
    # attributed to it and the sample has one timing basis rather than one per
    # reader.
    #
    # The PROGRESS timer enforces the clock-progress budget and starts only
    # once the hour and accumulator baselines exist. Spending that budget on the
    # setup reads would let a slow scheduler exhaust it before there was
    # anything to compare, and report a healthy advancing game as stalled
    # without ever taking a second clock reading.
    $windowStartedAt = [DateTimeOffset]::UtcNow
    $bracketStopwatch = [Diagnostics.Stopwatch]::StartNew()
    $initialSpeedRaw = Read-RemoteInt32 `
        $processHandle `
        ($gameState + $gameSpeedOffset)
    $initialPaused = Read-RemoteByte `
        $processHandle `
        ($clockA + $clockPausedOffset)
    $initialPauseForced = Read-RemoteByte `
        $processHandle `
        ($clockA + $clockPauseForcedOffset)
    $initialTickInhibit = Read-RemoteByte `
        $processHandle `
        ($gameState + $gameTickInhibitOffset)
    if ($initialSpeedRaw -lt 0 -or $initialSpeedRaw -gt 4) {
        throw "Game speed field is outside the authenticated 0..4 range: $initialSpeedRaw."
    }
    if ($initialPaused -gt 1) {
        throw "Pause field is outside the authenticated 0..1 range: $initialPaused."
    }
    if ($initialPauseForced -gt 1) {
        throw "Forced-pause field is outside the authenticated 0..1 range: $initialPauseForced."
    }

    # Budgeted rate observation. The hour is the finest-grained progress signal
    # available at UI speed 5: FUN_1401F0210 selects a zero-second interval for
    # the fastest slot, pins f32[CGameState+0x4B8] at 1.0f, and dispatches a
    # tick on every frame, so at speed 5 the hour IS the frame counter and
    # +0x4B8 is constant. At the four slower slots the reverse holds -- +0x4B8
    # accumulates every frame while the hour moves rarely -- so both are
    # sampled and either one moving proves the pacing block ran.
    #
    # Every hold flag is sampled on every poll, not only at the two ends. A
    # pause that begins and ends inside the window is invisible to the endpoint
    # reads alone, and the window would then be reported as an unexplained
    # stall - which is how a run of such windows would become a false hang
    # report.
    $hourA = Read-RemoteInt32 $processHandle ($gameState + $gameHourOffset)
    $tickProgressBitsA = Read-RemoteInt32 `
        $processHandle `
        ($gameState + $gameTickProgressOffset)
    $progressStopwatch = [Diagnostics.Stopwatch]::StartNew()
    $hourB = $hourA
    $tickProgressBitsB = $tickProgressBitsA
    $observedPaused = $initialPaused
    $observedPauseForced = $initialPauseForced
    $observedTickInhibit = $initialTickInhibit
    $pausedInWindow = ($initialPaused -ne 0)
    $pauseForcedInWindow = ($initialPauseForced -ne 0)
    $tickInhibitedInWindow = ($initialTickInhibit -ne 0)
    while ($progressStopwatch.ElapsedMilliseconds -lt $ClockProgressBudgetMs) {
        if ($hourB -ne $hourA -or $tickProgressBitsB -ne $tickProgressBitsA) {
            break
        }
        # A paused engine will not move either field for as long as the pause
        # lasts, and a pause is ordinary the game behaviour rather than a fault, so
        # stop spending the budget the moment the engine says it is paused.
        if ($observedPaused -ne 0) { break }
        # Sleep no further than the budget allows, and re-check after waking.
        # An unconditional full-poll sleep lets a caller whose poll interval is
        # longer than its budget accept movement that happened well outside the
        # window it is about to report.
        $remainingBudgetMs = $ClockProgressBudgetMs - [int]$progressStopwatch.ElapsedMilliseconds
        if ($remainingBudgetMs -le 0) { break }
        Start-Sleep -Milliseconds ([Math]::Min($ClockProgressPollMs, $remainingBudgetMs))
        if ($progressStopwatch.ElapsedMilliseconds -ge $ClockProgressBudgetMs) { break }
        # Read into candidates first. A deadline check taken BEFORE the reads
        # only proves the poll was authorised to start: the thread can be
        # descheduled between the check and the reads, and the values that come
        # back would then describe engine state from outside the budget this
        # sample is about to report. They are accepted only if the monotonic
        # clock is still inside the budget afterwards.
        $candidateHour = Read-RemoteInt32 $processHandle ($gameState + $gameHourOffset)
        $candidateTickProgressBits = Read-RemoteInt32 `
            $processHandle `
            ($gameState + $gameTickProgressOffset)
        $candidatePaused = Read-RemoteByte `
            $processHandle `
            ($clockA + $clockPausedOffset)
        $candidatePauseForced = Read-RemoteByte `
            $processHandle `
            ($clockA + $clockPauseForcedOffset)
        $candidateTickInhibit = Read-RemoteByte `
            $processHandle `
            ($gameState + $gameTickInhibitOffset)
        if ($progressStopwatch.ElapsedMilliseconds -gt $ClockProgressBudgetMs) { break }
        if ($candidatePaused -gt 1) {
            throw "Pause field is outside the authenticated 0..1 range: $candidatePaused."
        }
        if ($candidatePauseForced -gt 1) {
            throw "Forced-pause field is outside the authenticated 0..1 range: $candidatePauseForced."
        }
        $hourB = $candidateHour
        $tickProgressBitsB = $candidateTickProgressBits
        $observedPaused = $candidatePaused
        $observedPauseForced = $candidatePauseForced
        $observedTickInhibit = $candidateTickInhibit
        if ($observedPaused -ne 0) { $pausedInWindow = $true }
        if ($observedPauseForced -ne 0) { $pauseForcedInWindow = $true }
        if ($observedTickInhibit -ne 0) { $tickInhibitedInWindow = $true }
    }
    # Stop the progress timer with the loop it measures. Letting it run through
    # the closing speed and hold reads would charge those to the budgeted wait,
    # which is a different quantity from how long the sample looked for the
    # clock to move.
    $progressStopwatch.Stop()
    $finalSpeedRaw = Read-RemoteInt32 `
        $processHandle `
        ($gameState + $gameSpeedOffset)
    $finalPaused = Read-RemoteByte `
        $processHandle `
        ($clockA + $clockPausedOffset)
    $finalPauseForced = Read-RemoteByte `
        $processHandle `
        ($clockA + $clockPauseForcedOffset)
    $finalTickInhibit = Read-RemoteByte `
        $processHandle `
        ($gameState + $gameTickInhibitOffset)
    # Sampled with the closing reads so the reported value is the freshest one
    # inside the bracket, and inside it so the identity re-check below still
    # covers it. Deliberately NOT range-checked: this observer must not acquire
    # a new refusal, and the raw byte is the evidence a caller reasons from.
    $playerInputGate = Read-RemoteByte `
        $processHandle `
        ($gameState + $gameStatePlayerInputGateOffset)
    if ($finalPaused -gt 1) {
        throw "Pause field is outside the authenticated 0..1 range: $finalPaused."
    }
    if ($finalPauseForced -gt 1) {
        throw "Forced-pause field is outside the authenticated 0..1 range: $finalPauseForced."
    }
    if ($finalPaused -ne 0) { $pausedInWindow = $true }
    if ($finalPauseForced -ne 0) { $pauseForcedInWindow = $true }
    if ($finalTickInhibit -ne 0) { $tickInhibitedInWindow = $true }
    $bracketStopwatch.Stop()
    # The measured duration is the monotonic stopwatch; both wall-clock
    # endpoints are read from the wall clock. Synthesising the end as
    # start-plus-elapsed assumed the two clocks were started at the same
    # instant, and a scheduling gap between them would report a later
    # observation as having happened earlier than it did, which distorts the
    # spans the stall escalation reasons about. The wall span may therefore
    # exceed the measured duration, and the consumer treats the difference as
    # unobserved rather than as evidence.
    $observedWindowMs = [int]$bracketStopwatch.ElapsedMilliseconds
    $clockProgressWaitedMs = [int]$progressStopwatch.ElapsedMilliseconds
    $observedAt = [DateTimeOffset]::UtcNow
    $finalGameState = Read-RemoteInt64 `
        $processHandle `
        ($moduleBase + $gameStateSingletonRva)
    $finalClockA = Read-RemoteInt64 `
        $processHandle `
        ($moduleBase + $clockSingletonARva)
    $finalClockB = Read-RemoteInt64 `
        $processHandle `
        ($moduleBase + $clockSingletonBRva)
    if ($finalGameState -ne $gameState -or
        $finalClockA -ne $clockA -or
        $finalClockB -ne $clockA) {
        throw 'Engine singleton identity changed during read-only observation.'
    }
    # Process CPU time is deliberately NOT a memory read: it answers a
    # different question from every field above -- whether the process is
    # executing at all -- and is the only signal here that survives a game
    # whose engine state has stopped being updated.
    $process.Refresh()
    $processorMilliseconds = [int64]$process.TotalProcessorTime.TotalMilliseconds

    [pscustomobject]@{
        Pid = $TargetPid
        ExeSha256 = $actualExeSha256
        Action = 'Status'
        Access = 'PROCESS_QUERY_INFORMATION|PROCESS_VM_READ'
        InitialPaused = [bool]$initialPaused
        InitialSpeed = $initialSpeedRaw + 1
        Paused = [bool]$finalPaused
        Speed = $finalSpeedRaw + 1
        InitialPauseForced = [bool]$initialPauseForced
        PauseForced = [bool]$finalPauseForced
        InitialTickInhibited = [bool]$initialTickInhibit
        TickInhibited = [bool]$finalTickInhibit
        # The raw CGameState+0xA39 byte. Zero means the engine is loading, at
        # the frontend or resetting; non-zero means the in-game session is live.
        # Reported as the observed integer rather than a bool so a caller can
        # see the value the engine actually holds.
        PlayerInputGate = [int]$playerInputGate
        # Sampled on every poll, so a hold that begins and ends between the two
        # endpoint reads is still reported. These are what the verdict
        # authority classifies on; the endpoint fields are kept as evidence.
        PausedInWindow = [bool]$pausedInWindow
        PauseForcedInWindow = [bool]$pauseForcedInWindow
        TickInhibitedInWindow = [bool]$tickInhibitedInWindow
        HourA = $hourA
        HourB = $hourB
        HourDelta = $hourB - $hourA
        TickProgressBitsA = $tickProgressBitsA
        TickProgressBitsB = $tickProgressBitsB
        WindowStartedAtUtc = $windowStartedAt.UtcDateTime.ToString('o')
        WindowStartedAtUnixMs = [int64]$windowStartedAt.ToUnixTimeMilliseconds()
        # The window END, taken from the same basis as its start. A caller that
        # timestamps the sample when the observer subprocess returns is
        # measuring process teardown as well, and no measured window can span
        # that.
        ObservedAtUtc = $observedAt.UtcDateTime.ToString('o')
        ObservedAtUnixMs = [int64]$observedAt.ToUnixTimeMilliseconds()
        ObservedWindowMs = $observedWindowMs
        ClockProgressWaitedMs = $clockProgressWaitedMs
        ClockProgressBudgetMs = $ClockProgressBudgetMs
        ProcessorMilliseconds = $processorMilliseconds
        # True when the engine's pacing block ran at least once inside the
        # budget. Kept named `Advancing` because callers already read it, but
        # it is now a budgeted rate observation rather than one strict
        # inequality over a fixed 750 ms.
        Advancing = ($hourB -ne $hourA) -or ($tickProgressBitsB -ne $tickProgressBitsA)
    }
} finally {
    [void][Hoi4ReadOnlyStateNative]::CloseHandle($processHandle)
}
