[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [int] $ProcessId,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $ReceiptPath,
    [Parameter(Mandatory = $true)] [ValidateSet('nonempty', 'measured_empty')] [string] $CaseId,
    [Parameter(Mandatory = $true)] [string] $SessionId,
    [Parameter(Mandatory = $true)] [string] $CaptureId,
    [Parameter(Mandatory = $true)] [int] $ExpectedGameHourRaw,
    [int] $ExpectedClientWidth = 1920,
    [int] $ExpectedClientHeight = 1080,
    [int] $ReadinessTimeoutSeconds = 300,
    [int] $SettleMilliseconds = 8000
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$gameStateSlotRva = 0x3316E90
$gameHourOffset = 0x468
$clockSingletonARva = 0x33172E0
$clockSingletonBRva = 0x33172E8
$clockVtableRva = 0x2952510
$pauseOffset = 0x6C1
$decisionButtonX = 150
$decisionButtonY = 55

if ($SettleMilliseconds -lt 2000 -or $SettleMilliseconds -gt 30000) {
    throw 'SettleMilliseconds must be in 2000..30000.'
}
if ($ReadinessTimeoutSeconds -lt 30 -or $ReadinessTimeoutSeconds -gt 1800) {
    throw 'ReadinessTimeoutSeconds must be in 30..1800.'
}
foreach ($identity in @($SessionId, $CaptureId)) {
    if ($identity -cnotmatch '^[a-zA-Z0-9][a-zA-Z0-9_.-]{7,159}$') {
        throw "Session/capture identity is noncanonical: $identity"
    }
}

function Write-AtomicCreateNewUtf8([string] $Path, [string] $Text) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($fullPath)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    if ([IO.File]::Exists($fullPath)) { throw "Refusing to replace immutable A34 action receipt: $fullPath" }
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText($temporary, $Text, [Text.UTF8Encoding]::new($false))
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        [IO.File]::Move($temporary, $fullPath)
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Hoi4A34DecisionPanelActionNative {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool GetClientRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool PostMessageW(IntPtr hWnd, uint message, UIntPtr wParam, IntPtr lParam);
    [DllImport("kernel32.dll", SetLastError = true)] public static extern bool ReadProcessMemory(IntPtr process, IntPtr address, byte[] buffer, UIntPtr size, out UIntPtr read);
}
'@

function Read-Bytes([IntPtr] $Handle, [long] $Address, [int] $Count, [string] $Label) {
    $buffer = [byte[]]::new($Count)
    $read = [UIntPtr]::Zero
    if (-not [Hoi4A34DecisionPanelActionNative]::ReadProcessMemory(
        $Handle, [IntPtr]$Address, $buffer, [UIntPtr]::new([uint64]$Count), [ref]$read
    ) -or $read.ToUInt64() -ne [uint64]$Count) {
        throw ('Fail closed: ReadProcessMemory failed for {0} at 0x{1:X}.' -f $Label, $Address)
    }
    return $buffer
}

function Read-U64([IntPtr] $Handle, [long] $Address, [string] $Label) {
    return [BitConverter]::ToUInt64((Read-Bytes $Handle $Address 8 $Label), 0)
}

function Read-I32([IntPtr] $Handle, [long] $Address, [string] $Label) {
    return [BitConverter]::ToInt32((Read-Bytes $Handle $Address 4 $Label), 0)
}

$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$process = Get-Process -Id $ProcessId -ErrorAction Stop
$processExe = $process.MainModule.FileName
$exeSha256 = (Get-FileHash -LiteralPath $processExe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($exeSha256 -cne $expectedExeSha256 -or
    -not [IO.Path]::GetFullPath($processExe).Equals($resolvedExe, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Fail closed: A34 action target is not the exact-a729 executable.'
}
$loadedCollectors = @($process.Modules | Where-Object {
    [IO.Path]::GetFullPath([string]$_.FileName).Equals($resolvedCollector, [StringComparison]::OrdinalIgnoreCase)
})
if ($loadedCollectors.Count -ne 1) { throw 'Fail closed: exact centrally attested collector is not loaded exactly once.' }

$window = $process.MainWindowHandle
if ($window -eq [IntPtr]::Zero) { throw 'Fail closed: exact-a729 process has no main window.' }
$windowOwner = [uint32]0
[void][Hoi4A34DecisionPanelActionNative]::GetWindowThreadProcessId($window, [ref]$windowOwner)
if ($windowOwner -ne [uint32]$ProcessId) { throw 'Fail closed: A34 window owner differs from the exact process.' }
$client = [Hoi4A34DecisionPanelActionNative+RECT]::new()
if (-not [Hoi4A34DecisionPanelActionNative]::GetClientRect($window, [ref]$client)) {
    throw 'Fail closed: GetClientRect failed for A34.'
}
$clientWidth = $client.Right - $client.Left
$clientHeight = $client.Bottom - $client.Top
if ($clientWidth -ne $ExpectedClientWidth -or $clientHeight -ne $ExpectedClientHeight) {
    throw "Fail closed: A34 profile requires ${ExpectedClientWidth}x${ExpectedClientHeight}, observed ${clientWidth}x${clientHeight}."
}

$moduleBase = $process.MainModule.BaseAddress.ToInt64()
$handle = $process.Handle
$clock = [long]0
$gameState = [long]0
$hourBefore = [int]::MinValue
$pauseBefore = [byte]0
$lastReadinessError = 'clock roots were not readable'
$readinessDeadline = [DateTime]::UtcNow.AddSeconds($ReadinessTimeoutSeconds)
while ([DateTime]::UtcNow -lt $readinessDeadline) {
    try {
        $clockA = Read-U64 $handle ($moduleBase + $clockSingletonARva) 'clock singleton A'
        $clockB = Read-U64 $handle ($moduleBase + $clockSingletonBRva) 'clock singleton B'
        $clocks = @(@($clockA, $clockB) | Where-Object { $_ -ne 0 } | Select-Object -Unique)
        if ($clocks.Count -ne 1) { throw 'expected exactly one live clock singleton' }
        $candidateClock = [long]$clocks[0]
        if ((Read-U64 $handle $candidateClock 'clock vtable') -ne [uint64]($moduleBase + $clockVtableRva)) {
            throw 'clock vtable identity mismatch'
        }
        $candidateGameState = [long](Read-U64 $handle ($moduleBase + $gameStateSlotRva) 'CGameState singleton')
        if ($candidateGameState -eq 0) { throw 'CGameState singleton is null' }
        $candidateHour = Read-I32 $handle ($candidateGameState + $gameHourOffset) 'pre-action game hour'
        $candidatePause = (Read-Bytes $handle ($candidateClock + $pauseOffset) 1 'pre-action pause byte')[0]
        if ($candidateHour -eq $ExpectedGameHourRaw -and $candidatePause -eq 1) {
            $clock = $candidateClock
            $gameState = $candidateGameState
            $hourBefore = $candidateHour
            $pauseBefore = $candidatePause
            break
        }
        $lastReadinessError = "observed hour=$candidateHour pause=$candidatePause"
    } catch {
        $lastReadinessError = $_.Exception.Message
    }
    Start-Sleep -Milliseconds 100
}
if ($clock -eq 0 -or $gameState -eq 0) {
    throw "Fail closed: A34 did not reach paused raw hour $ExpectedGameHourRaw within $ReadinessTimeoutSeconds seconds ($lastReadinessError)."
}

# The exact shipped topbar places decisionview_button so (150,55) is an
# interior client point at 1920x1080. Targeted window messages avoid global
# input and are accepted only for the authenticated main-window handle.
$lParam = [IntPtr](($decisionButtonY -shl 16) -bor $decisionButtonX)
if (-not [Hoi4A34DecisionPanelActionNative]::PostMessageW($window, 0x0201, [UIntPtr]1, $lParam) -or
    -not [Hoi4A34DecisionPanelActionNative]::PostMessageW($window, 0x0202, [UIntPtr]0, $lParam)) {
    throw 'Fail closed: decision-view targeted click could not be posted.'
}
Start-Sleep -Milliseconds $SettleMilliseconds

$process = Get-Process -Id $ProcessId -ErrorAction Stop
if ($process.MainWindowHandle -ne $window) { throw 'Fail closed: A34 main-window identity changed after the action.' }
$hourAfter = Read-I32 $handle ($gameState + $gameHourOffset) 'post-action game hour'
$pauseAfter = (Read-Bytes $handle ($clock + $pauseOffset) 1 'post-action pause byte')[0]
if ($hourAfter -ne $hourBefore -or $pauseAfter -ne 1) {
    throw 'Fail closed: read-only A34 panel navigation changed the paused game clock.'
}

$receipt = [ordered]@{
    schema = 'hoi4.a34.autonomous-decision-panel-action.v1'
    acquisition_owner = 'machine'
    result = 'AUTHENTICATED_DECISION_VIEW_OPEN_ACTION_COMPLETE_HOOK_VERDICT_REQUIRED'
    production_activation_authorized = $false
    timestamp_utc = [DateTime]::UtcNow.ToString('o')
    case_id = $CaseId
    session_id = $SessionId
    capture_id = $CaptureId
    process_id = $ProcessId
    process_start_utc = $process.StartTime.ToUniversalTime().ToString('o')
    executable_sha256 = $exeSha256
    collector_dll_sha256 = (Get-FileHash -LiteralPath $resolvedCollector -Algorithm SHA256).Hash.ToLowerInvariant()
    window_owner_process_id = $windowOwner
    client_size = @($clientWidth, $clientHeight)
    action = [ordered]@{
        method = 'targeted_postmessage_to_authenticated_output_window'
        control = 'decisionview_button'
        client_point = @($decisionButtonX, $decisionButtonY)
        wm_lbuttondown = $true
        wm_lbuttonup = $true
        settle_milliseconds = $SettleMilliseconds
        coordinate_authority = 'shipped topbar.gui at 1920x1080; independently retained UI13 targeted trace'
    }
    expected_game_hour_raw = $ExpectedGameHourRaw
    readiness_timeout_seconds = $ReadinessTimeoutSeconds
    game_hour_raw_before = $hourBefore
    game_hour_raw_after = $hourAfter
    pause_before = $pauseBefore
    pause_after = $pauseAfter
    semantic_authority = 'A34 hook-minted DPN1 transaction and independent regrade only'
}
Write-AtomicCreateNewUtf8 $ReceiptPath (($receipt | ConvertTo-Json -Depth 8) + "`n")
