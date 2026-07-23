[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [ValidateSet('Readiness', 'Unpause', 'Repause')] [string] $Action,
    [Parameter(Mandatory = $true)] [int] $ProcessId,
    [Parameter(Mandatory = $true)] [string] $ExpectedProcessStartUtc,
    [Parameter(Mandatory = $true)] [string] $SessionId,
    [Parameter(Mandatory = $true)] [string] $CaptureId,
    [Parameter(Mandatory = $true)] [string] $ReceiptPath,
    [int] $AdvanceProofSeconds = 5,
    [int] $ReadinessTimeoutSeconds = 240,
    [int] $ExpectedReadyGameHourRaw = 60830441
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$dateEpochRaw = 43799999
$gameStateSlotRva = 0x3316E90
$clockSingletonARva = 0x33172E0
$clockSingletonBRva = 0x33172E8
$clockVtableRva = 0x2952510
$pauseOffset = 0x6C1
$gameHourOffset = 0x468
$gameSpeedOffset = 0x4BC

if ($AdvanceProofSeconds -lt 1 -or $AdvanceProofSeconds -gt 30) {
    throw 'AdvanceProofSeconds must be in 1..30.'
}
if ($ReadinessTimeoutSeconds -lt 1 -or $ReadinessTimeoutSeconds -gt 600) {
    throw 'ReadinessTimeoutSeconds must be in 1..600.'
}
if ($ExpectedReadyGameHourRaw -le $dateEpochRaw) {
    throw "ExpectedReadyGameHourRaw must be a raw Jomini DateHour above epoch $dateEpochRaw."
}
foreach ($identity in @($SessionId, $CaptureId)) {
    if ($identity -cnotmatch '^[a-z0-9][a-z0-9_.-]{7,159}$') {
        throw "Session/capture identity is noncanonical: $identity"
    }
}
$expectedStart = [DateTime]::Parse(
    $ExpectedProcessStartUtc,
    [Globalization.CultureInfo]::InvariantCulture,
    [Globalization.DateTimeStyles]::RoundtripKind
).ToUniversalTime()

function Write-AtomicUtf8Text([string]$Path, [string]$Text) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($fullPath)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporaryPath = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText($temporaryPath, $Text, [Text.UTF8Encoding]::new($false))
        $stream = [IO.File]::Open($temporaryPath, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($fullPath)) {
            throw "Refusing to replace an existing A29 clock receipt: $fullPath"
        }
        [IO.File]::Move($temporaryPath, $fullPath)
    } finally {
        if ([IO.File]::Exists($temporaryPath)) { [IO.File]::Delete($temporaryPath) }
    }
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Hoi4A29ClockTransition {
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool ReadProcessMemory(IntPtr process, IntPtr address, byte[] buffer, UIntPtr size, out UIntPtr read);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool WriteProcessMemory(IntPtr process, IntPtr address, byte[] buffer, UIntPtr size, out UIntPtr written);
}
'@

function Read-Bytes([IntPtr]$Handle, [long]$Address, [int]$Count, [string]$Label) {
    $buffer = [byte[]]::new($Count)
    $read = [UIntPtr]::Zero
    if (-not [Hoi4A29ClockTransition]::ReadProcessMemory(
        $Handle, [IntPtr]$Address, $buffer, [UIntPtr]::new([uint64]$Count), [ref]$read
    ) -or $read.ToUInt64() -ne [uint64]$Count) {
        throw ('Fail closed: ReadProcessMemory failed for {0} at 0x{1:X}, Win32={2}.' -f $Label, $Address, [Runtime.InteropServices.Marshal]::GetLastWin32Error())
    }
    return $buffer
}

function Read-U64([IntPtr]$Handle, [long]$Address, [string]$Label) {
    return [BitConverter]::ToUInt64((Read-Bytes $Handle $Address 8 $Label), 0)
}

function Read-I32([IntPtr]$Handle, [long]$Address, [string]$Label) {
    return [BitConverter]::ToInt32((Read-Bytes $Handle $Address 4 $Label), 0)
}

function Write-PauseByte([IntPtr]$Handle, [long]$Address, [byte]$Value) {
    $written = [UIntPtr]::Zero
    $buffer = [byte[]]@($Value)
    if (-not [Hoi4A29ClockTransition]::WriteProcessMemory(
        $Handle, [IntPtr]$Address, $buffer, [UIntPtr]::new([uint64]1), [ref]$written
    ) -or $written.ToUInt64() -ne 1) {
        throw ('Fail closed: WriteProcessMemory failed at authenticated pause byte, Win32={0}.' -f [Runtime.InteropServices.Marshal]::GetLastWin32Error())
    }
}

$process = Get-Process -Id $ProcessId -ErrorAction Stop
$observedStart = $process.StartTime.ToUniversalTime()
if ($observedStart.Ticks -ne $expectedStart.Ticks) {
    throw 'Fail closed: process start identity changed before A29 clock transition.'
}
$exePath = $process.MainModule.FileName
$exeSha256 = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($exeSha256 -cne $expectedExeSha256) {
    throw "Fail closed: executable SHA-256 mismatch: $exeSha256"
}
$moduleBase = $process.MainModule.BaseAddress.ToInt64()
$handle = $process.Handle
function Read-AuthenticatedClockState {
    $currentProcess = Get-Process -Id $ProcessId -ErrorAction Stop
    if ($currentProcess.StartTime.ToUniversalTime().Ticks -ne $expectedStart.Ticks) {
        throw 'process start identity changed'
    }
    $clockA = Read-U64 $handle ($moduleBase + $clockSingletonARva) 'clock singleton A'
    $clockB = Read-U64 $handle ($moduleBase + $clockSingletonBRva) 'clock singleton B'
    if ($clockA -eq 0 -or $clockA -ne $clockB) {
        throw ('clock singleton slots are not equal non-null pointers: A=0x{0:X}, B=0x{1:X}' -f $clockA, $clockB)
    }
    $clock = [long]$clockA
    $actualVtable = Read-U64 $handle $clock 'clock vtable'
    if ($actualVtable -ne [uint64]($moduleBase + $clockVtableRva)) {
        throw ('clock vtable 0x{0:X} != exact authority 0x{1:X}' -f $actualVtable, ($moduleBase + $clockVtableRva))
    }
    $gameState = [long](Read-U64 $handle ($moduleBase + $gameStateSlotRva) 'CGameState singleton')
    if ($gameState -eq 0) { throw 'CGameState singleton is null' }
    $pauseRaw = (Read-Bytes $handle ($clock + $pauseOffset) 1 'pause byte')[0]
    $gameHourRaw = Read-I32 $handle ($gameState + $gameHourOffset) 'raw Jomini DateHour'
    $speedRaw = Read-I32 $handle ($gameState + $gameSpeedOffset) 'raw speed index'
    if ($pauseRaw -notin @(0, 1) -or $gameHourRaw -le $dateEpochRaw -or $speedRaw -lt 0 -or $speedRaw -gt 4) {
        throw "implausible exact clock state pause=$pauseRaw speed_raw=$speedRaw game_hour_raw=$gameHourRaw"
    }
    return [pscustomobject]@{
        Clock = $clock
        GameState = $gameState
        PauseRaw = $pauseRaw
        GameHourRaw = $gameHourRaw
        GameHourNormalized = $gameHourRaw - $dateEpochRaw
        SpeedRaw = $speedRaw
    }
}

if ($Action -ceq 'Readiness') {
    $deadline = [DateTime]::UtcNow.AddSeconds($ReadinessTimeoutSeconds)
    $state = $null
    $lastReadinessFailure = 'no authenticated state observed'
    do {
        try {
            $candidate = Read-AuthenticatedClockState
            if ($candidate.PauseRaw -eq 1 -and $candidate.GameHourRaw -eq $ExpectedReadyGameHourRaw) {
                $state = $candidate
                break
            }
            $lastReadinessFailure = "pause=$($candidate.PauseRaw) game_hour_raw=$($candidate.GameHourRaw)"
        } catch {
            $lastReadinessFailure = $_.Exception.Message
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($null -eq $state) {
        throw "Fail closed: paused-save readiness was not authenticated within $ReadinessTimeoutSeconds seconds ($lastReadinessFailure)."
    }
} else {
    $state = Read-AuthenticatedClockState
}

$clock = [long]$state.Clock
$gameState = [long]$state.GameState
$pauseAddress = $clock + $pauseOffset
$pauseBefore = [byte]$state.PauseRaw
$gameHourRawBefore = [int]$state.GameHourRaw
$gameHourNormalizedBefore = [int]$state.GameHourNormalized
$speedRaw = [int]$state.SpeedRaw

if ($Action -ceq 'Readiness') {
    $pauseAfter = $pauseBefore
    $gameHourRawAfter = $gameHourRawBefore
    $gameHourNormalizedAfter = $gameHourNormalizedBefore
    $result = 'AUTHENTICATED_PAUSED_SAVE_READY'
} elseif ($Action -ceq 'Unpause') {
    if ($pauseBefore -ne 1) { throw "Fail closed: A29 expected a paused save, observed pause byte $pauseBefore." }
    if ($gameHourRawBefore -ne $ExpectedReadyGameHourRaw) {
        throw "Fail closed: unpause expected D04 raw hour $ExpectedReadyGameHourRaw, observed $gameHourRawBefore."
    }
    Write-PauseByte $handle $pauseAddress 0
    $pauseAfter = (Read-Bytes $handle $pauseAddress 1 'unpause confirmation')[0]
    Start-Sleep -Seconds $AdvanceProofSeconds
    $gameHourRawAfter = Read-I32 $handle ($gameState + $gameHourOffset) 'advanced raw Jomini DateHour'
    $gameHourNormalizedAfter = $gameHourRawAfter - $dateEpochRaw
    if ($pauseAfter -ne 0 -or $gameHourRawAfter -le $gameHourRawBefore) {
        throw "Fail closed: unpause did not advance the authenticated clock (pause=$pauseAfter raw_before=$gameHourRawBefore raw_after=$gameHourRawAfter)."
    }
    $result = 'AUTHENTICATED_PAUSED_TO_RUNNING_AND_ADVANCING'
} else {
    if ($pauseBefore -notin @(0, 1)) { throw "Fail closed: invalid pause byte before repause: $pauseBefore." }
    if ($pauseBefore -eq 0) { Write-PauseByte $handle $pauseAddress 1 }
    $pauseAfter = (Read-Bytes $handle $pauseAddress 1 'repause confirmation')[0]
    $gameHourRawAfter = Read-I32 $handle ($gameState + $gameHourOffset) 'repaused raw Jomini DateHour'
    $gameHourNormalizedAfter = $gameHourRawAfter - $dateEpochRaw
    if ($pauseAfter -ne 1) { throw "Fail closed: unconditional A29 repause did not hold (pause=$pauseAfter)." }
    $result = if ($pauseBefore -eq 1) { 'AUTHENTICATED_ALREADY_PAUSED' } else { 'AUTHENTICATED_RUNNING_TO_PAUSED' }
}

$receipt = [ordered]@{
    schema = 'hoi4.shared.authenticated-clock-transition.v2'
    action = $Action.ToLowerInvariant()
    result = $result
    timestamp_utc = [DateTime]::UtcNow.ToString('o')
    process_id = $ProcessId
    process_start_utc = $observedStart.ToString('o')
    session_id = $SessionId
    capture_id = $CaptureId
    executable_sha256 = $exeSha256
    game_state_slot_rva = ('0x{0:X}' -f $gameStateSlotRva)
    clock_singleton_a_rva = ('0x{0:X}' -f $clockSingletonARva)
    clock_singleton_b_rva = ('0x{0:X}' -f $clockSingletonBRva)
    clock_vtable_rva = ('0x{0:X}' -f $clockVtableRva)
    pause_offset = ('0x{0:X}' -f $pauseOffset)
    game_hour_offset = ('0x{0:X}' -f $gameHourOffset)
    game_speed_offset = ('0x{0:X}' -f $gameSpeedOffset)
    date_epoch_raw = $dateEpochRaw
    expected_ready_game_hour_raw = $ExpectedReadyGameHourRaw
    expected_ready_game_hour_normalized = $ExpectedReadyGameHourRaw - $dateEpochRaw
    pause_before = $pauseBefore
    pause_after = $pauseAfter
    speed_raw = $speedRaw
    game_hour_raw_before = $gameHourRawBefore
    game_hour_raw_after = $gameHourRawAfter
    game_hour_normalized_before = $gameHourNormalizedBefore
    game_hour_normalized_after = $gameHourNormalizedAfter
}
$receiptJson = $receipt | ConvertTo-Json -Depth 8
Write-AtomicUtf8Text $ReceiptPath ($receiptJson + "`n")
$receiptJson
