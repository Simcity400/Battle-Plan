[CmdletBinding()]
param(
    [Parameter()]
    [ValidateRange(1, 2147483647)]
    [int]$TargetPid,

    [Parameter()]
    [ValidateRange(1, 5)]
    [int]$Speed,

    [Parameter()]
    [switch]$Play,

    [Parameter()]
    [switch]$Pause
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($Play -and $Pause) {
    throw '-Play and -Pause are mutually exclusive.'
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$gameStateSingletonRva = 0x3316E90L
$gameHourOffset = 0x468L
$gameSpeedOffset = 0x4BCL
$clockSingletonARva = 0x33172E0L
$clockSingletonBRva = 0x33172E8L
$clockVtableRva = 0x2952510L

# The CInGameIdler pause cluster, enumerated from the binary rather than
# assumed, by searching hoi4.exe's .text for the exact little-endian disp32 of
# each offset. Two earlier methods DID miss sites -- a linear sweep desynced
# past FUN_1401F0210's 0x4C0 test, and a per-.pdata decode skipped
# FUN_140DBE260 entirely because .pdata does not list frameless leaf functions
# -- so both are retained as negative controls in the scan, not as evidence.
#
# WHAT THAT SEARCH DOES AND DOES NOT COVER. It is exhaustive for a DIRECT
# disp32 access from a register holding the object pointer, because any
# displacement above 0x7F must be encoded that way. It does NOT cover a write
# through an interior pointer (`lea rax,[rcx+0x680]` then `mov byte
# [rax+0x41],1` reaches +0x6C1 without encoding it), an overlapping wider
# store, or a bulk copy over the region. Those need dataflow, which was not
# done. The cluster below is therefore the enumerated writer set, not a proved
# exhaustive one; the one partial mitigation taken was to enumerate +0x6C0 as
# well, which is the interior-pointer base an access to this cluster would most
# plausibly use.
#
# The six routines that write +0x6C1, and therefore constitute a pause
# transition:
#   FUN_140DBE260  forced pause          -> +0x6C1=1, +0x6C3=1
#   FUN_140DC5310  load/setup            -> +0x6C1 = 0 or 1
#   FUN_140DC6520  general pause entry   -> +0x6C0, strings, +0x800, +0x808,
#                                           +0x6C1, +0x6C3
#   FUN_140DCBF00  pause when not server -> +0x6C1=1
#   FUN_140DCD3E0  player toggle         -> +0x6C1, +0x6C2
#   FUN_140DCD770  player toggle (route) -> +0x6C0=1, +0x6C1
# Every one of them ends by calling FUN_140B5F980, which is the in-game
# interface handler's GUI update (it asserts "_Idler.CanUpdateGui()" against
# ingameinterfacehandler.cpp:0x5AD). See the WHAT THIS CANNOT DO note below.
#
# +0x6C4/+0x6C5/+0x6C6 are also written in the CInGameIdler code region, by
# functions that do NOT write +0x6C1. They are idler state, not pause
# transition state, and are deliberately not touched.
$clockPauseRouteOffset = 0x6C0L
$clockPausedOffset = 0x6C1L
$clockPauseKindOffset = 0x6C2L
$clockPauseForcedOffset = 0x6C3L
# DAT_1433723EC: a dword cleared by both FUN_140DC6520 and FUN_140DCD3E0 on an
# unpause transition, and never set by either.
$pauseCounterRva = 0x33723ECL

$nativeSource = @'
using System;
using System.Runtime.InteropServices;

public static class Hoi4HeadlessControlNative
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
    public static extern bool WriteProcessMemory(
        IntPtr process,
        IntPtr address,
        byte[] buffer,
        UIntPtr size,
        out UIntPtr bytesWritten);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool CloseHandle(IntPtr handle);
}
'@

if (-not ('Hoi4HeadlessControlNative' -as [type])) {
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
$actualExeSha256 = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualExeSha256 -ne $expectedExeSha256) {
    throw "Refusing to control PID ${TargetPid}: unsupported hoi4.exe SHA-256 $actualExeSha256."
}

$processQueryInformation = 0x0400
$processVmOperation = 0x0008
$processVmRead = 0x0010
$processVmWrite = 0x0020
$desiredAccess = $processQueryInformation -bor $processVmRead
if ($Speed -or $Play -or $Pause) {
    $desiredAccess = $desiredAccess -bor $processVmOperation -bor $processVmWrite
}

$processHandle = [Hoi4HeadlessControlNative]::OpenProcess(
    [uint32]$desiredAccess,
    $false,
    $TargetPid)
if ($processHandle -eq [IntPtr]::Zero) {
    $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    throw "OpenProcess($TargetPid) failed with Win32 error $errorCode."
}

function Read-RemoteBytes {
    param(
        [Parameter(Mandatory)]
        [IntPtr]$ProcessHandle,

        [Parameter(Mandatory)]
        [long]$Address,

        [Parameter(Mandatory)]
        [ValidateRange(1, 8)]
        [int]$Count
    )

    $buffer = [byte[]]::new($Count)
    $bytesRead = [UIntPtr]::Zero
    $size = [UIntPtr]::new([uint64]$Count)
    $ok = [Hoi4HeadlessControlNative]::ReadProcessMemory(
        $ProcessHandle,
        [IntPtr]$Address,
        $buffer,
        $size,
        [ref]$bytesRead)
    if (-not $ok) {
        $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        throw ('ReadProcessMemory(0x{0:X}) failed with Win32 error {1}.' -f $Address, $errorCode)
    }
    if ($bytesRead.ToUInt64() -ne [uint64]$Count) {
        throw ('ReadProcessMemory(0x{0:X}) returned {1} of {2} bytes.' -f
            $Address, $bytesRead.ToUInt64(), $Count)
    }
    return $buffer
}

function Read-RemoteInt32 {
    param([IntPtr]$ProcessHandle, [long]$Address)
    return [BitConverter]::ToInt32((Read-RemoteBytes $ProcessHandle $Address 4), 0)
}

function Read-RemoteInt64 {
    param([IntPtr]$ProcessHandle, [long]$Address)
    return [BitConverter]::ToInt64((Read-RemoteBytes $ProcessHandle $Address 8), 0)
}

function Read-RemoteByte {
    param([IntPtr]$ProcessHandle, [long]$Address)
    return (Read-RemoteBytes $ProcessHandle $Address 1)[0]
}

function Write-RemoteBytes {
    param(
        [Parameter(Mandatory)]
        [IntPtr]$ProcessHandle,

        [Parameter(Mandatory)]
        [long]$Address,

        [Parameter(Mandatory)]
        [byte[]]$Bytes
    )

    $bytesWritten = [UIntPtr]::Zero
    $size = [UIntPtr]::new([uint64]$Bytes.Length)
    $ok = [Hoi4HeadlessControlNative]::WriteProcessMemory(
        $ProcessHandle,
        [IntPtr]$Address,
        $Bytes,
        $size,
        [ref]$bytesWritten)
    if (-not $ok) {
        $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        throw ('WriteProcessMemory(0x{0:X}) failed with Win32 error {1}.' -f $Address, $errorCode)
    }
    if ($bytesWritten.ToUInt64() -ne [uint64]$Bytes.Length) {
        throw ('WriteProcessMemory(0x{0:X}) returned {1} of {2} bytes.' -f
            $Address, $bytesWritten.ToUInt64(), $Bytes.Length)
    }
}

try {
    $moduleBase = $process.MainModule.BaseAddress.ToInt64()
    $gameState = Read-RemoteInt64 $processHandle ($moduleBase + $gameStateSingletonRva)
    $stableGameState = Read-RemoteInt64 $processHandle ($moduleBase + $gameStateSingletonRva)
    if ($gameState -eq 0 -or $gameState -ne $stableGameState -or ($gameState -band 7) -ne 0) {
        throw 'CGameState singleton is null, unstable, or misaligned.'
    }

    $clockA = Read-RemoteInt64 $processHandle ($moduleBase + $clockSingletonARva)
    $clockB = Read-RemoteInt64 $processHandle ($moduleBase + $clockSingletonBRva)
    if ($clockA -eq 0 -or $clockA -ne $clockB -or ($clockA -band 7) -ne 0) {
        throw 'CInGameIdler singleton pair is null, incoherent, or misaligned.'
    }
    $clockVtable = Read-RemoteInt64 $processHandle $clockA
    $expectedClockVtable = $moduleBase + $clockVtableRva
    if ($clockVtable -ne $expectedClockVtable) {
        throw ('CInGameIdler vtable mismatch: expected 0x{0:X}, observed 0x{1:X}.' -f
            $expectedClockVtable, $clockVtable)
    }

    $initialSpeedRaw = Read-RemoteInt32 $processHandle ($gameState + $gameSpeedOffset)
    $initialPaused = Read-RemoteByte $processHandle ($clockA + $clockPausedOffset)
    if ($initialSpeedRaw -lt 0 -or $initialSpeedRaw -gt 4) {
        throw "Game speed field is outside the authenticated 0..4 range: $initialSpeedRaw."
    }
    if ($initialPaused -gt 1) {
        throw "Pause field is outside the authenticated 0..1 range: $initialPaused."
    }

    if ($Speed) {
        $speedRaw = $Speed - 1
        Write-RemoteBytes $processHandle ($gameState + $gameSpeedOffset) ([BitConverter]::GetBytes($speedRaw))
        if ((Read-RemoteInt32 $processHandle ($gameState + $gameSpeedOffset)) -ne $speedRaw) {
            throw "Game speed $Speed did not survive immediate readback."
        }
    }

    # +0x6C0..+0x6C3 are contiguous, so the whole cluster comes back from ONE
    # read. Reading the fields separately let the engine change state between
    # them, and a decision taken from a torn snapshot can name a routine that
    # would not make the requested transition from the state that actually
    # holds -- selecting the player toggle from a stale "paused" while the game
    # is already unpaused would toggle it back to paused.
    function Read-PauseCluster {
        param([IntPtr]$ProcessHandle, [long]$ClockAddress)
        $bytes = Read-RemoteBytes $ProcessHandle ($ClockAddress + $clockPauseRouteOffset) 4
        foreach ($index in 1, 3) {
            if ($bytes[$index] -gt 1) {
                throw "Pause cluster byte +0x6C$index is outside the authenticated 0..1 range: $($bytes[$index])."
            }
        }
        return [pscustomobject]@{
            Route  = $bytes[0]
            Paused = $bytes[1]
            Kind   = $bytes[2]
            Forced = $bytes[3]
        }
    }

    function Assert-PauseClusterUnchanged {
        param([Parameter(Mandatory)]$Expected)
        $now = Read-PauseCluster -ProcessHandle $processHandle -ClockAddress $clockA
        if ($now.Route -ne $Expected.Route -or $now.Paused -ne $Expected.Paused -or
            $now.Kind -ne $Expected.Kind -or $now.Forced -ne $Expected.Forced) {
            throw (
                'The pause state changed between selecting an engine routine to mirror and ' +
                'writing it, so the routine named is no longer one that makes this ' +
                "transition from the state that holds (was $($Expected.Route)/$($Expected.Paused)/" +
                "$($Expected.Kind)/$($Expected.Forced), now $($now.Route)/$($now.Paused)/$($now.Kind)/$($now.Forced))."
            )
        }
    }

    $entryCluster = Read-PauseCluster -ProcessHandle $processHandle -ClockAddress $clockA
    $initialPauseKind = $entryCluster.Kind
    $initialPauseForced = $entryCluster.Forced
    $initialPauseRoute = $entryCluster.Route
    $mirroredRoutine = 'none'
    $unreproducibleSteps = @()
    $expectedPost = $null

    if ($Play -or $Pause) {
        $wantPaused = [bool]$Pause
        # Decide from a snapshot taken now, not from the entry read: the speed
        # write sits between them.
        $decisionCluster = Read-PauseCluster -ProcessHandle $processHandle -ClockAddress $clockA
        $initialPaused = $decisionCluster.Paused
        $initialPauseKind = $decisionCluster.Kind
        $initialPauseForced = $decisionCluster.Forced
        $initialPauseRoute = $decisionCluster.Route
        if ([bool]$decisionCluster.Paused -eq $wantPaused) {
            # The engine performs no transition when it is already in the
            # requested state, so neither do we. Writing the fields anyway
            # would be a transition the engine never makes.
            $mirroredRoutine = 'none (already in the requested state)'
        } elseif ($decisionCluster.Forced -ne 0) {
            # A forced pause cannot be cleared by any player route:
            # FUN_140DC6520's manual arm runs only while +0x6C3 is clear, so
            # the keyboard and the date button are inert. The only engine
            # routine that reaches the cleared state is FUN_140DC6520's forced
            # arm from (+0x6C1=1, +0x6C3=1), and this mirrors exactly its
            # writes: entry route flag, then the pair, then the counter.
            if ($wantPaused) {
                throw 'Refusing to pause: the engine already holds a forced pause, and no engine routine pauses again from that state.'
            }
            $mirroredRoutine = 'FUN_140DC6520 forced arm (clears a forced pause; no player route exists)'
            $expectedPost = [pscustomobject]@{
                Route = 0; Paused = 0; Kind = $decisionCluster.Kind; Forced = 0
            }
            Assert-PauseClusterUnchanged -Expected $decisionCluster
            Write-RemoteBytes $processHandle ($clockA + $clockPauseRouteOffset) ([byte[]](0))
            Write-RemoteBytes $processHandle ($clockA + $clockPausedOffset) ([byte[]](0))
            Write-RemoteBytes $processHandle ($clockA + $clockPauseForcedOffset) ([byte[]](0))
            Write-RemoteBytes $processHandle ($moduleBase + $pauseCounterRva) ([BitConverter]::GetBytes([int]0))
            $unreproducibleSteps += 'FUN_140B5F980 in-game interface GUI update'
            $unreproducibleSteps += 'MSVC std::string assign to CInGameIdler+0x7E0 (heap-allocating)'
        } else {
            # The player's own route. FUN_140DCD3E0 toggles +0x6C1, writes
            # +0x6C2 from its argument, and clears the counter when it is the
            # unpause direction. It leaves +0x6C0 and +0x6C3 alone, so this
            # does too.
            if (($decisionCluster.Kind -ne 0) -and ($decisionCluster.Paused -ne 0)) {
                throw 'Refusing to drive: FUN_140DCD3E0 returns without transitioning from this state (+0x6C2 and +0x6C1 both set), so there is no player transition to mirror.'
            }
            $mirroredRoutine = 'FUN_140DCD3E0 player toggle'
            $expectedPost = [pscustomobject]@{
                Route = $decisionCluster.Route; Paused = [int]$wantPaused
                Kind = 0; Forced = $decisionCluster.Forced
            }
            Assert-PauseClusterUnchanged -Expected $decisionCluster
            Write-RemoteBytes $processHandle ($clockA + $clockPausedOffset) (
                [byte[]]([byte]([int]$wantPaused))
            )
            Write-RemoteBytes $processHandle ($clockA + $clockPauseKindOffset) ([byte[]](0))
            if (-not $wantPaused) {
                Write-RemoteBytes $processHandle ($moduleBase + $pauseCounterRva) ([BitConverter]::GetBytes([int]0))
            }
            $unreproducibleSteps += 'FUN_140B5F980 in-game interface GUI update'
            $unreproducibleSteps += 'FUN_140202A90 pause-intent broadcast'
            if ($wantPaused) {
                $unreproducibleSteps += 'MSVC std::string assign to CInGameIdler+0x7C0 (heap-allocating), CInGameIdler+0x800 timestamp, +0x808 duration'
            } else {
                $unreproducibleSteps += 'MSVC std::string assign to CInGameIdler+0x7E0 (heap-allocating)'
            }
        }

        # Read the WHOLE cluster back, in one read, and check every field the
        # mirrored routine is responsible for -- not just the byte we care
        # about. A transition that left any field disagreeing with the routine
        # it claims to mirror is exactly the silent incoherence this replaces.
        if ($null -eq $expectedPost) {
            $expectedPost = $decisionCluster
        }
        $postCluster = Read-PauseCluster -ProcessHandle $processHandle -ClockAddress $clockA
        $postCounter = Read-RemoteInt32 $processHandle ($moduleBase + $pauseCounterRva)
        # Compare against the EXACT cluster the mirrored routine produces from
        # the state we decided on, field by field.
        #
        # WHAT THIS IS: best-effort DIVERGENCE detection. It catches an
        # interleaving that leaves the cluster somewhere the mirrored routine
        # would not have left it.
        #
        # WHAT IT CANNOT DETECT, and this is not a gap that a further read
        # closes: a CONVERGENT interleaving, where the engine's own transition
        # lands on the same endpoint we were about to write. Concretely -- we
        # decide from (route 0, paused 0, kind 0, forced 0) and request a pause;
        # FUN_140DCBF00 concurrently sets paused alone; the cluster now already
        # equals what we expected to produce; we write it again; every field
        # matches and the transition is reported as mirrored. But
        # FUN_140DCD3E0, the routine named in that report, invoked from the
        # state that actually held at write time, would have toggled back to
        # unpaused. The endpoints are equal and the transitions are not, and no
        # comparison of endpoints can tell them apart. ABA sequences are
        # invisible for the same reason.
        #
        # Distinguishing them needs something endpoint comparison does not
        # have: an authenticated monotonic transition generation on the engine
        # side, which this build does not expose here -- the global at
        # 0x1433723EC is cleared on unpause and never incremented, so it is not
        # one -- or serialisation against the engine, which means suspending the
        # target's threads. Suspension is a new class of intervention into the
        # player's game with its own deadlock hazard, and needs owner sign-off.
        #
        # So this refuses on divergence and does NOT certify a mirror. The
        # MirrorVerification field says so on every transition.
        if ($postCluster.Route -ne $expectedPost.Route -or
            $postCluster.Paused -ne $expectedPost.Paused -or
            $postCluster.Kind -ne $expectedPost.Kind -or
            $postCluster.Forced -ne $expectedPost.Forced) {
            throw (
                'The pause cluster after the write diverges from the one the mirrored routine ' +
                "produces: expected $($expectedPost.Route)/$($expectedPost.Paused)/" +
                "$($expectedPost.Kind)/$($expectedPost.Forced), observed " +
                "$($postCluster.Route)/$($postCluster.Paused)/$($postCluster.Kind)/$($postCluster.Forced). " +
                'The engine changed the pause state during the transition. Note that the ' +
                'converse does not hold: matching fields do not establish that this script ' +
                'made the transition.'
            )
        }
        if ($postCluster.Paused -ne [int]$wantPaused) {
            throw "Pause latch did not survive immediate readback: wanted $([int]$wantPaused), observed $($postCluster.Paused)."
        }
        if ($mirroredRoutine -like 'FUN_140DC6520*') {
            if ($postCluster.Route -ne 0 -or $postCluster.Forced -ne 0) {
                throw "Forced-arm mirror left route=$($postCluster.Route) forced=$($postCluster.Forced); FUN_140DC6520's forced clear leaves both zero."
            }
        } elseif ($mirroredRoutine -like 'FUN_140DCD3E0*') {
            if ($postCluster.Kind -ne 0) {
                throw "Player-toggle mirror left kind=$($postCluster.Kind); FUN_140DCD3E0 writes it from its argument, which is zero here."
            }
            if ($postCluster.Route -ne $decisionCluster.Route -or
                $postCluster.Forced -ne $decisionCluster.Forced) {
                throw "Player-toggle mirror changed route or forced, which FUN_140DCD3E0 never writes."
            }
        }
        if (-not $wantPaused) {
            if ($postCluster.Forced -ne 0) {
                throw "Unpause left the forced-pause flag set ($($postCluster.Forced)); the resulting state is one no engine routine produces."
            }
            if ($postCounter -ne 0) {
                throw "Unpause left the pause counter at $postCounter; every engine unpause clears it."
            }
        }
    }

    Start-Sleep -Milliseconds 250
    $hourA = Read-RemoteInt32 $processHandle ($gameState + $gameHourOffset)
    Start-Sleep -Milliseconds 750
    $hourB = Read-RemoteInt32 $processHandle ($gameState + $gameHourOffset)
    $finalSpeedRaw = Read-RemoteInt32 $processHandle ($gameState + $gameSpeedOffset)
    # One coherent snapshot for the whole report. Reading the reported fields
    # one at a time let an ordinary engine pause land between them and produce
    # a returned object describing no state the engine was ever in.
    $finalCluster = Read-PauseCluster -ProcessHandle $processHandle -ClockAddress $clockA
    $finalPauseCounter = Read-RemoteInt32 $processHandle ($moduleBase + $pauseCounterRva)
    $finalPaused = $finalCluster.Paused
    $finalGameState = Read-RemoteInt64 $processHandle ($moduleBase + $gameStateSingletonRva)
    $finalClockA = Read-RemoteInt64 $processHandle ($moduleBase + $clockSingletonARva)
    $finalClockB = Read-RemoteInt64 $processHandle ($moduleBase + $clockSingletonBRva)
    if ($finalGameState -ne $gameState -or $finalClockA -ne $clockA -or $finalClockB -ne $clockA) {
        throw 'Engine singleton identity changed during the control observation.'
    }

    [pscustomobject]@{
        Pid = $TargetPid
        ExeSha256 = $actualExeSha256
        Action = if ($Play) { 'Play' } elseif ($Pause) { 'Pause' } elseif ($Speed) { 'SetSpeed' } else { 'Status' }
        InitialPaused = [bool]$initialPaused
        InitialSpeed = $initialSpeedRaw + 1
        Paused = [bool]$finalPaused
        Speed = $finalSpeedRaw + 1
        InitialPauseForced = [bool]$initialPauseForced
        InitialPauseKind = [bool]$initialPauseKind
        InitialPauseRoute = [bool]$initialPauseRoute
        PauseForced = [bool]$finalCluster.Forced
        PauseKind = [bool]$finalCluster.Kind
        PauseRoute = [bool]$finalCluster.Route
        PauseCounter = $finalPauseCounter
        # Which engine routine's writes this mirrored, and what it could not do.
        # WHAT THIS CANNOT DO: every engine pause transition ends by calling
        # FUN_140B5F980, the in-game interface handler's GUI update, and the
        # transitions also assign MSVC std::strings that allocate in the target
        # heap. WriteProcessMemory cannot perform either. The engine's DATA
        # state is mirrored; its UI state is not, so the pause overlay may
        # remain lit even though the simulation is running. This field exists so
        # that is never a silent partial success.
        MirroredEngineRoutine = $mirroredRoutine
        # Deliberately not called verification. Endpoint comparison refuses a
        # divergent interleaving and cannot see a convergent or ABA one, so a
        # clean result is evidence that nothing diverged and NOT evidence that
        # this script performed the transition.
        MirrorVerification = 'best-effort divergence detection; convergent and ABA engine transitions are undetectable by endpoint comparison'
        UnreproducibleEngineSteps = $unreproducibleSteps
        HourA = $hourA
        HourB = $hourB
        HourDelta = $hourB - $hourA
        Advancing = $hourB -ne $hourA
    }
} finally {
    [void][Hoi4HeadlessControlNative]::CloseHandle($processHandle)
}
