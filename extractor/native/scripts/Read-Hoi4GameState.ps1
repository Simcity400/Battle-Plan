[CmdletBinding()]
param(
    [Parameter()]
    [ValidateRange(1, 2147483647)]
    [int]$TargetPid
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$gameStateSingletonRva = 0x3316E90L
$gameHourOffset = 0x468L
$gameSpeedOffset = 0x4BCL
$clockSingletonARva = 0x33172E0L
$clockSingletonBRva = 0x33172E8L
$clockVtableRva = 0x2952510L
$clockPausedOffset = 0x6C1L

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

    $initialSpeedRaw = Read-RemoteInt32 `
        $processHandle `
        ($gameState + $gameSpeedOffset)
    $initialPaused = Read-RemoteByte `
        $processHandle `
        ($clockA + $clockPausedOffset)
    if ($initialSpeedRaw -lt 0 -or $initialSpeedRaw -gt 4) {
        throw "Game speed field is outside the authenticated 0..4 range: $initialSpeedRaw."
    }
    if ($initialPaused -gt 1) {
        throw "Pause field is outside the authenticated 0..1 range: $initialPaused."
    }

    Start-Sleep -Milliseconds 250
    $hourA = Read-RemoteInt32 $processHandle ($gameState + $gameHourOffset)
    Start-Sleep -Milliseconds 750
    $hourB = Read-RemoteInt32 $processHandle ($gameState + $gameHourOffset)
    $finalSpeedRaw = Read-RemoteInt32 `
        $processHandle `
        ($gameState + $gameSpeedOffset)
    $finalPaused = Read-RemoteByte `
        $processHandle `
        ($clockA + $clockPausedOffset)
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

    [pscustomobject]@{
        Pid = $TargetPid
        ExeSha256 = $actualExeSha256
        Action = 'Status'
        Access = 'PROCESS_QUERY_INFORMATION|PROCESS_VM_READ'
        InitialPaused = [bool]$initialPaused
        InitialSpeed = $initialSpeedRaw + 1
        Paused = [bool]$finalPaused
        Speed = $finalSpeedRaw + 1
        HourA = $hourA
        HourB = $hourB
        HourDelta = $hourB - $hourA
        Advancing = $hourB -ne $hourA
    }
} finally {
    [void][Hoi4ReadOnlyStateNative]::CloseHandle($processHandle)
}
