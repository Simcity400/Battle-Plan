[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $ReceiptPath,
    [Parameter(Mandatory = $true)] [long] $NotBeforeFileTimeUtc,
    [int] $TimeoutSeconds = 600
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$gameStateSlotRva = 0x3316E90L
$clockSingletonARva = 0x33172E0L
$clockSingletonBRva = 0x33172E8L
$clockVtableRva = 0x2952510L
$gameHourOffset = 0x468L
$tickProgressOffset = 0x4B8L
$speedOffset = 0x4BCL
$pauseOffset = 0x6C1L

if ($TimeoutSeconds -lt 30 -or $TimeoutSeconds -gt 3600) {
    throw 'TimeoutSeconds must be 30..3600.'
}
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) {
    throw 'Hoi4Exe is not exact a729 authority.'
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class X12ActionNative {
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool ReadProcessMemory(
        IntPtr process, IntPtr address, byte[] buffer, UIntPtr size, out UIntPtr read);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessageW(IntPtr window, uint message, UIntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
}
'@

function Read-ExactBytes([IntPtr] $Handle, [long] $Address, [int] $Count) {
    $buffer = [byte[]]::new($Count)
    $read = [UIntPtr]::Zero
    if (-not [X12ActionNative]::ReadProcessMemory(
        $Handle,
        [IntPtr]$Address,
        $buffer,
        [UIntPtr]::new([uint64]$Count),
        [ref]$read
    ) -or $read.ToUInt64() -ne [uint64]$Count) {
        throw ('ReadProcessMemory failed at 0x{0:X}, Win32={1}' -f
            $Address, [Runtime.InteropServices.Marshal]::GetLastWin32Error())
    }
    return $buffer
}

function Read-U64([IntPtr] $Handle, [long] $Address) {
    return [BitConverter]::ToUInt64((Read-ExactBytes $Handle $Address 8), 0)
}

function Read-I32([IntPtr] $Handle, [long] $Address) {
    return [BitConverter]::ToInt32((Read-ExactBytes $Handle $Address 4), 0)
}

function Read-U32([IntPtr] $Handle, [long] $Address) {
    return [BitConverter]::ToUInt32((Read-ExactBytes $Handle $Address 4), 0)
}

function Read-U8([IntPtr] $Handle, [long] $Address) {
    return (Read-ExactBytes $Handle $Address 1)[0]
}

function Read-AuthenticatedState($Process) {
    $base = $Process.MainModule.BaseAddress.ToInt64()
    $handle = $Process.Handle
    $gameState = [long](Read-U64 $handle ($base + $gameStateSlotRva))
    $clockA = [long](Read-U64 $handle ($base + $clockSingletonARva))
    $clockB = [long](Read-U64 $handle ($base + $clockSingletonBRva))
    if ($gameState -eq 0 -or $clockA -eq 0 -or $clockA -ne $clockB) {
        throw 'X12 game-state/clock singleton identity is not ready.'
    }
    if ((Read-U64 $handle $clockA) -ne [uint64]($base + $clockVtableRva)) {
        throw 'X12 clock vtable does not match exact a729.'
    }
    $hour = Read-I32 $handle ($gameState + $gameHourOffset)
    $speed = Read-I32 $handle ($gameState + $speedOffset)
    $pause = Read-U8 $handle ($clockA + $pauseOffset)
    $tickBits = Read-U32 $handle ($gameState + $tickProgressOffset)
    $tick = [BitConverter]::ToSingle([BitConverter]::GetBytes($tickBits), 0)
    if ($hour -lt 1 -or $speed -lt 0 -or $speed -gt 4 -or
        $pause -gt 1 -or [Single]::IsNaN($tick) -or
        [Single]::IsInfinity($tick) -or $tick -lt 0.0 -or $tick -gt 1.0 -or
        $tickBits -eq 2147483648L) {
        throw 'X12 exact state failed range/finite validation.'
    }
    return [ordered]@{
        observed_filetime_utc = [DateTime]::UtcNow.ToFileTimeUtc()
        game_hour_raw = $hour
        speed_raw = $speed
        paused = ($pause -eq 1)
        tick_progress_f32_bits = ('0x{0:x8}' -f $tickBits)
    }
}

function Post-Key([IntPtr] $Window, [int] $VirtualKey) {
    $down = [X12ActionNative]::PostMessageW(
        $Window, 0x0100, [UIntPtr]::new([uint64]$VirtualKey), [IntPtr]::Zero)
    $up = [X12ActionNative]::PostMessageW(
        $Window, 0x0101, [UIntPtr]::new([uint64]$VirtualKey), [IntPtr]::Zero)
    if (-not $down -or -not $up) {
        throw ('PostMessageW failed for VK 0x{0:X2}, Win32={1}' -f
            $VirtualKey, [Runtime.InteropServices.Marshal]::GetLastWin32Error())
    }
}

function Wait-State($Process, [scriptblock] $Predicate, [string] $Label, [int] $Seconds = 10) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    do {
        try {
            $state = Read-AuthenticatedState $Process
            if (& $Predicate $state) {
                return $state
            }
        } catch {
            if ([DateTime]::UtcNow -ge $deadline) { throw }
        }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for authenticated X12 state: $Label"
}

$deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
$process = $null
while ($null -eq $process -and [DateTime]::UtcNow -lt $deadline) {
    $candidates = @(
        Get-Process -Name the game -ErrorAction SilentlyContinue |
            Where-Object {
                try {
                    $_.StartTime.ToUniversalTime().ToFileTimeUtc() -ge $NotBeforeFileTimeUtc -and
                    [IO.Path]::GetFullPath($_.MainModule.FileName) -ieq
                        [IO.Path]::GetFullPath($resolvedExe)
                } catch {
                    $false
                }
            }
    )
    if ($candidates.Count -gt 1) {
        throw 'More than one new exact-a729 the game process is eligible for X12 actions.'
    }
    if ($candidates.Count -eq 1) {
        $process = $candidates[0]
        break
    }
    Start-Sleep -Milliseconds 100
}
if ($null -eq $process) {
    throw 'Timed out waiting for the new exact-a729 the game process.'
}

$window = [IntPtr]::Zero
while ($window -eq [IntPtr]::Zero -and [DateTime]::UtcNow -lt $deadline) {
    $process.Refresh()
    $window = $process.MainWindowHandle
    if ($window -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 100 }
}
if ($window -eq [IntPtr]::Zero) {
    throw 'Timed out waiting for the exact the game main window.'
}
$windowOwner = [uint32]0
[void][X12ActionNative]::GetWindowThreadProcessId($window, [ref]$windowOwner)
if ($windowOwner -ne $process.Id) {
    throw 'the game main-window owner does not match the exact process.'
}

$ready = Wait-State $process {
    param($state)
    $state.game_hour_raw -ge 1
} 'loaded game state' ([Math]::Max(10, $TimeoutSeconds))
$actions = [Collections.Generic.List[object]]::new()

# Establish a fresh pause edge. If the continued save was already paused,
# unpause first; neither the action receipt nor key delivery is trusted as
# evidence—the in-process exact mutation hooks and replay regrade are.
if ($ready.paused) {
    $before = $ready
    Post-Key $window 0x20
    $ready = Wait-State $process { param($state) -not $state.paused } 'initial unpause'
    $actions.Add([ordered]@{ action = 'unpause_if_needed'; virtual_key = '0x20'; before = $before; after = $ready })
}
$beforePause = $ready
Post-Key $window 0x20
$paused = Wait-State $process { param($state) $state.paused } 'fixture pause'
$actions.Add([ordered]@{ action = 'pause'; virtual_key = '0x20'; before = $beforePause; after = $paused })
$frozenHour = $paused.game_hour_raw

$targetOne = if ($paused.speed_raw -ne 0) { 0 } else { 1 }
$targetTwo = if ($targetOne -ne 3) { 3 } else { 4 }
foreach ($target in @($targetOne, $targetTwo)) {
    $before = Read-AuthenticatedState $process
    Post-Key $window (0x31 + $target)
    $after = Wait-State $process {
        param($state)
        $state.paused -and
        $state.game_hour_raw -eq $frozenHour -and
        $state.speed_raw -eq $target
    } "paused speed target $target"
    $actions.Add([ordered]@{
        action = 'set_speed_while_paused'
        virtual_key = ('0x{0:X2}' -f (0x31 + $target))
        before = $before
        after = $after
    })
}

$beforeResume = Read-AuthenticatedState $process
Post-Key $window 0x20
$resumed = Wait-State $process {
    param($state)
    # Polling may first observe the process after the resumed clock has already
    # crossed the boundary. The in-process hook timestamp, not this diagnostic
    # poll, is the exact same-hour authority.
    -not $state.paused -and $state.game_hour_raw -ge $frozenHour
} 'fixture resume'
$actions.Add([ordered]@{ action = 'resume'; virtual_key = '0x20'; before = $beforeResume; after = $resumed })
$drained = Wait-State $process {
    param($state)
    -not $state.paused -and $state.game_hour_raw -gt $frozenHour
} 'next-hour D57 drain' 60

$receipt = [ordered]@{
    schema = 'battle-plan.x12_authenticated_action_attempt.v1'
    authority = 'diagnostic_action_attempt_not_replay_evidence'
    executable_path = $resolvedExe
    executable_sha256 = $expectedExeSha256
    process_id = $process.Id
    process_start_filetime_utc = $process.StartTime.ToUniversalTime().ToFileTimeUtc()
    module_base = ('0x{0:X}' -f $process.MainModule.BaseAddress.ToInt64())
    window = ('0x{0:X}' -f $window.ToInt64())
    window_owner_process_id = $windowOwner
    frozen_fixture_hour = $frozenHour
    actions = @($actions)
    next_hour_observation = $drained
    evidence_status = 'not_authority_use_x12_replay_regrade'
}
$parent = Split-Path -Parent ([IO.Path]::GetFullPath($ReceiptPath))
[IO.Directory]::CreateDirectory($parent) | Out-Null
[IO.File]::WriteAllText(
    [IO.Path]::GetFullPath($ReceiptPath),
    ($receipt | ConvertTo-Json -Depth 12) + "`n",
    [Text.UTF8Encoding]::new($false)
)
$receipt | ConvertTo-Json -Depth 12
