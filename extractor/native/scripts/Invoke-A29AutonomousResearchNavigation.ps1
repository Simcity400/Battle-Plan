[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [int] $ProcessId,
    [Parameter(Mandatory = $true)] [string] $ReceiptPath,
    [Parameter(Mandatory = $true)] [string] $SessionId,
    [Parameter(Mandatory = $true)] [string] $CaptureId,
    [int] $ExpectedClientWidth = 1920,
    [int] $ExpectedClientHeight = 1080,
    [int] $InitialSettleSeconds = 75,
    [int] $UiSettleMilliseconds = 3000,
    [int] $FutureNodeHoverDwellMilliseconds = 5000
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$dateEpochRaw = 43799999
$expectedD04GameHourRaw = 60830441
$expectedD04GameHourNormalized = $expectedD04GameHourRaw - $dateEpochRaw
$gameStateSlotRva = 0x3316E90
$gameHourOffset = 0x468
$gameSpeedOffset = 0x4BC
$clockSingletonARva = 0x33172E0
$clockSingletonBRva = 0x33172E8
$clockVtableRva = 0x2952510
$pauseOffset = 0x6C1

foreach ($identity in @($SessionId, $CaptureId)) {
    if ($identity -cnotmatch '^[a-z0-9][a-z0-9_.-]{7,159}$') {
        throw "Session/capture identity is noncanonical: $identity"
    }
}

if ($InitialSettleSeconds -lt 0 -or $InitialSettleSeconds -gt 300) {
    throw 'InitialSettleSeconds must be in 0..300.'
}
if ($UiSettleMilliseconds -lt 500 -or $UiSettleMilliseconds -gt 10000) {
    throw 'UiSettleMilliseconds must be in 500..10000.'
}
if ($FutureNodeHoverDwellMilliseconds -lt 5000 -or $FutureNodeHoverDwellMilliseconds -gt 30000) {
    throw 'FutureNodeHoverDwellMilliseconds must be in 5000..30000.'
}

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
            throw "Refusing to replace an existing A29 navigation receipt: $fullPath"
        }
        [IO.File]::Move($temporaryPath, $fullPath)
    } finally {
        if ([IO.File]::Exists($temporaryPath)) {
            [IO.File]::Delete($temporaryPath)
        }
    }
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Hoi4A29AutonomousNavigation {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool GetClientRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool ClientToScreen(IntPtr hWnd, ref POINT point);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool GetCursorPos(out POINT point);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int command);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint source, uint target, bool attach);
    [DllImport("kernel32.dll", SetLastError = true)] public static extern bool ReadProcessMemory(IntPtr process, IntPtr address, byte[] buffer, UIntPtr size, out UIntPtr read);
}
'@

if ($InitialSettleSeconds -gt 0) {
    Start-Sleep -Seconds $InitialSettleSeconds
}

$process = Get-Process -Id $ProcessId -ErrorAction Stop
$exePath = $process.MainModule.FileName
$exeSha256 = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($exeSha256 -cne $expectedExeSha256) {
    throw "Fail closed: executable SHA-256 mismatch: $exeSha256"
}
$window = $process.MainWindowHandle
if ($window -eq [IntPtr]::Zero) {
    throw 'Fail closed: exact-a729 process has no main window after the load-settle interval.'
}
$windowOwner = [uint32]0
[void][Hoi4A29AutonomousNavigation]::GetWindowThreadProcessId($window, [ref]$windowOwner)
if ($windowOwner -ne $ProcessId) {
    throw "Fail closed: main-window owner PID $windowOwner does not match exact process PID $ProcessId."
}

$client = [Hoi4A29AutonomousNavigation+RECT]::new()
if (-not [Hoi4A29AutonomousNavigation]::GetClientRect($window, [ref]$client)) {
    throw 'Fail closed: GetClientRect failed.'
}
$width = $client.Right - $client.Left
$height = $client.Bottom - $client.Top
if ($width -ne $ExpectedClientWidth -or $height -ne $ExpectedClientHeight) {
    throw "Fail closed: A29 D04 navigation profile requires ${ExpectedClientWidth}x${ExpectedClientHeight}, observed ${width}x${height}."
}

[void][Hoi4A29AutonomousNavigation]::ShowWindowAsync($window, 9)
$foregroundBefore = [Hoi4A29AutonomousNavigation]::GetForegroundWindow()
$foregroundProcessId = [uint32]0
$foregroundThread = if ($foregroundBefore -eq [IntPtr]::Zero) {
    [uint32]0
} else {
    [Hoi4A29AutonomousNavigation]::GetWindowThreadProcessId($foregroundBefore, [ref]$foregroundProcessId)
}
$targetThread = [Hoi4A29AutonomousNavigation]::GetWindowThreadProcessId($window, [ref]$windowOwner)
$attachedThreads = $false
try {
    if ($foregroundThread -ne 0 -and $targetThread -ne 0 -and $foregroundThread -ne $targetThread) {
        if (-not [Hoi4A29AutonomousNavigation]::AttachThreadInput($foregroundThread, $targetThread, $true)) {
            throw 'Fail closed: AttachThreadInput rejected authenticated foreground transfer.'
        }
        $attachedThreads = $true
    }
    [void][Hoi4A29AutonomousNavigation]::BringWindowToTop($window)
    if (-not [Hoi4A29AutonomousNavigation]::SetForegroundWindow($window)) {
        throw 'Fail closed: SetForegroundWindow rejected the exact-a729 window.'
    }
} finally {
    if ($attachedThreads) {
        [void][Hoi4A29AutonomousNavigation]::AttachThreadInput($foregroundThread, $targetThread, $false)
    }
}
Start-Sleep -Milliseconds 750

function Read-AuthenticatedBytes([IntPtr]$Handle, [long]$Address, [int]$Count, [string]$Label) {
    $buffer = [byte[]]::new($Count)
    $read = [UIntPtr]::Zero
    if (-not [Hoi4A29AutonomousNavigation]::ReadProcessMemory(
        $Handle, [IntPtr]$Address, $buffer, [UIntPtr]::new([uint64]$Count), [ref]$read
    ) -or $read.ToUInt64() -ne [uint64]$Count) {
        throw ('Fail closed: ReadProcessMemory failed for {0} at 0x{1:X}.' -f $Label, $Address)
    }
    return $buffer
}

function Read-AuthenticatedU64([IntPtr]$Handle, [long]$Address, [string]$Label) {
    return [BitConverter]::ToUInt64((Read-AuthenticatedBytes $Handle $Address 8 $Label), 0)
}

function Read-AuthenticatedI32([IntPtr]$Handle, [long]$Address, [string]$Label) {
    return [BitConverter]::ToInt32((Read-AuthenticatedBytes $Handle $Address 4 $Label), 0)
}

$moduleBase = $process.MainModule.BaseAddress.ToInt64()
$processHandle = $process.Handle
$clockA = Read-AuthenticatedU64 $processHandle ($moduleBase + $clockSingletonARva) 'clock singleton A'
$clockB = Read-AuthenticatedU64 $processHandle ($moduleBase + $clockSingletonBRva) 'clock singleton B'
$clockCandidates = @(@($clockA, $clockB) | Where-Object { $_ -ne 0 } | Select-Object -Unique)
if ($clockCandidates.Count -ne 1) {
    throw ('Fail closed: expected one D04 clock singleton, got A=0x{0:X}, B=0x{1:X}.' -f $clockA, $clockB)
}
$clock = [long]$clockCandidates[0]
$clockVtable = Read-AuthenticatedU64 $processHandle $clock 'clock vtable'
if ($clockVtable -ne [uint64]($moduleBase + $clockVtableRva)) {
    throw 'Fail closed: D04 clock vtable identity mismatch.'
}
$gameState = [long](Read-AuthenticatedU64 $processHandle ($moduleBase + $gameStateSlotRva) 'CGameState singleton')
if ($gameState -eq 0) { throw 'Fail closed: D04 CGameState singleton is null.' }
$pauseBeforeNavigation = (Read-AuthenticatedBytes $processHandle ($clock + $pauseOffset) 1 'pause byte')[0]
$gameHourRawBeforeNavigation = Read-AuthenticatedI32 $processHandle ($gameState + $gameHourOffset) 'raw Jomini DateHour'
$gameHourNormalizedBeforeNavigation = $gameHourRawBeforeNavigation - $dateEpochRaw
$gameSpeedRawBeforeNavigation = Read-AuthenticatedI32 $processHandle ($gameState + $gameSpeedOffset) 'raw speed index'
if ($pauseBeforeNavigation -ne 1 -or $gameHourRawBeforeNavigation -ne $expectedD04GameHourRaw -or
    $gameHourNormalizedBeforeNavigation -ne $expectedD04GameHourNormalized -or
    $gameSpeedRawBeforeNavigation -lt 0 -or $gameSpeedRawBeforeNavigation -gt 4) {
    throw "Fail closed: expected paused D04 raw hour $expectedD04GameHourRaw (normalized $expectedD04GameHourNormalized), observed pause=$pauseBeforeNavigation speed_raw=$gameSpeedRawBeforeNavigation raw=$gameHourRawBeforeNavigation normalized=$gameHourNormalizedBeforeNavigation."
}

function Assert-AuthenticatedWindow([string]$ActionLabel) {
    $currentProcess = Get-Process -Id $ProcessId -ErrorAction Stop
    if ($currentProcess.MainWindowHandle -ne $window) {
        throw "Fail closed: exact-a729 main-window identity changed before $ActionLabel."
    }
    $currentWindowOwner = [uint32]0
    [void][Hoi4A29AutonomousNavigation]::GetWindowThreadProcessId($window, [ref]$currentWindowOwner)
    if ($currentWindowOwner -ne $ProcessId) {
        throw "Fail closed: window owner PID $currentWindowOwner does not match exact process before $ActionLabel."
    }
    $foreground = [Hoi4A29AutonomousNavigation]::GetForegroundWindow()
    if ($foreground -ne $window) {
        throw "Fail closed: exact-a729 main window is not foreground before $ActionLabel."
    }
    $foregroundOwner = [uint32]0
    [void][Hoi4A29AutonomousNavigation]::GetWindowThreadProcessId($foreground, [ref]$foregroundOwner)
    if ($foregroundOwner -ne $ProcessId) {
        throw "Fail closed: foreground owner PID $foregroundOwner does not match exact process before $ActionLabel."
    }
    $currentClient = [Hoi4A29AutonomousNavigation+RECT]::new()
    if (-not [Hoi4A29AutonomousNavigation]::GetClientRect($window, [ref]$currentClient)) {
        throw "Fail closed: GetClientRect failed before $ActionLabel."
    }
    if (($currentClient.Right - $currentClient.Left) -ne $ExpectedClientWidth -or
        ($currentClient.Bottom - $currentClient.Top) -ne $ExpectedClientHeight) {
        throw "Fail closed: client size changed before $ActionLabel."
    }
    return $foregroundOwner
}

function Move-AuthenticatedCursor([int]$X, [int]$Y, [string]$ActionLabel) {
    if ($X -lt 0 -or $X -ge $ExpectedClientWidth -or $Y -lt 0 -or $Y -ge $ExpectedClientHeight) {
        throw "Fail closed: client point ($X,$Y) is outside the frozen A29 profile for $ActionLabel."
    }
    $foregroundOwner = Assert-AuthenticatedWindow $ActionLabel
    $screenPoint = [Hoi4A29AutonomousNavigation+POINT]::new()
    $screenPoint.X = $X
    $screenPoint.Y = $Y
    if (-not [Hoi4A29AutonomousNavigation]::ClientToScreen($window, [ref]$screenPoint)) {
        throw "Fail closed: ClientToScreen failed for $ActionLabel."
    }
    if (-not [Hoi4A29AutonomousNavigation]::SetCursorPos($screenPoint.X, $screenPoint.Y)) {
        throw "Fail closed: SetCursorPos failed for $ActionLabel."
    }
    $observed = [Hoi4A29AutonomousNavigation+POINT]::new()
    if (-not [Hoi4A29AutonomousNavigation]::GetCursorPos([ref]$observed) -or
        $observed.X -ne $screenPoint.X -or $observed.Y -ne $screenPoint.Y) {
        throw "Fail closed: cursor placement was not exact for $ActionLabel."
    }
    return [ordered]@{
        foreground_owner_process_id = $foregroundOwner
        client_point = @($X, $Y)
        screen_point = @($screenPoint.X, $screenPoint.Y)
        cursor_confirmed = $true
    }
}

$steps = [Collections.Generic.List[object]]::new()

# This exact paused-D04 route was observed on the pinned 1920x1080 build. The
# first two clicks only navigate into the already-active technology tree. The
# future technology node is hover-only and therefore cannot replace research.
$move = Move-AuthenticatedCursor 241 56 'open_research_topbar'
[Hoi4A29AutonomousNavigation]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 80
[Hoi4A29AutonomousNavigation]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
$steps.Add([ordered]@{
    action = 'research_button_click'
    method = 'physical_setcursorpos_mouse_event'
    label = 'open_research_topbar'
    client_point = $move.client_point
    screen_point = $move.screen_point
    foreground_owner_process_id = $move.foreground_owner_process_id
    window_owner_process_id = $windowOwner
    cursor_confirmed = $move.cursor_confirmed
    click_hold_milliseconds = 80
    settle_milliseconds = $UiSettleMilliseconds
})
Start-Sleep -Milliseconds $UiSettleMilliseconds

$move = Move-AuthenticatedCursor 195 340 'open_active_engineer_company_iii'
[Hoi4A29AutonomousNavigation]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 80
[Hoi4A29AutonomousNavigation]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
$steps.Add([ordered]@{
    action = 'active_row_click'
    method = 'physical_setcursorpos_mouse_event'
    label = 'open_active_engineer_company_iii'
    client_point = $move.client_point
    screen_point = $move.screen_point
    foreground_owner_process_id = $move.foreground_owner_process_id
    window_owner_process_id = $windowOwner
    cursor_confirmed = $move.cursor_confirmed
    click_hold_milliseconds = 80
    settle_milliseconds = $UiSettleMilliseconds
})
Start-Sleep -Milliseconds $UiSettleMilliseconds

$move = Move-AuthenticatedCursor 1404 302 'hover_engineer_company_iv_1945'
$steps.Add([ordered]@{
    action = 'future_node_hover'
    method = 'physical_setcursorpos_hover_only'
    label = 'hover_engineer_company_iv_1945'
    client_point = $move.client_point
    screen_point = $move.screen_point
    foreground_owner_process_id = $move.foreground_owner_process_id
    window_owner_process_id = $windowOwner
    cursor_confirmed = $move.cursor_confirmed
    dwell_milliseconds = $FutureNodeHoverDwellMilliseconds
})
Start-Sleep -Milliseconds $FutureNodeHoverDwellMilliseconds
[void](Assert-AuthenticatedWindow 'complete_future_node_hover')
$finalCursor = [Hoi4A29AutonomousNavigation+POINT]::new()
if (-not [Hoi4A29AutonomousNavigation]::GetCursorPos([ref]$finalCursor) -or
    $finalCursor.X -ne $move.screen_point[0] -or $finalCursor.Y -ne $move.screen_point[1]) {
    throw 'Fail closed: future-node hover did not remain stationary for the required dwell.'
}
$pauseAfterNavigation = (Read-AuthenticatedBytes $processHandle ($clock + $pauseOffset) 1 'post-navigation pause byte')[0]
$gameHourRawAfterNavigation = Read-AuthenticatedI32 $processHandle ($gameState + $gameHourOffset) 'post-navigation raw Jomini DateHour'
$gameHourNormalizedAfterNavigation = $gameHourRawAfterNavigation - $dateEpochRaw
$gameSpeedRawAfterNavigation = Read-AuthenticatedI32 $processHandle ($gameState + $gameSpeedOffset) 'post-navigation raw speed index'
if ($pauseAfterNavigation -ne 1 -or $gameHourRawAfterNavigation -ne $expectedD04GameHourRaw -or
    $gameHourNormalizedAfterNavigation -ne $expectedD04GameHourNormalized -or
    $gameSpeedRawAfterNavigation -ne $gameSpeedRawBeforeNavigation) {
    throw "Fail closed: read-only navigation changed D04 clock state: pause=$pauseAfterNavigation speed_raw=$gameSpeedRawAfterNavigation raw=$gameHourRawAfterNavigation normalized=$gameHourNormalizedAfterNavigation."
}

$canonicalSteps = ($steps | ForEach-Object {
    $delay = if ($_.PSObject.Properties['settle_milliseconds']) { $_.settle_milliseconds } else { $_.dwell_milliseconds }
    $clickHold = if ($_.PSObject.Properties['click_hold_milliseconds']) { $_.click_hold_milliseconds } else { 0 }
    '{0}|{1}|{2}|{3}|{4}|{5}|{6}' -f $_.action, $_.label, $_.client_point[0], $_.client_point[1], $_.method, $delay, $clickHold
}) -join ';'
$stepBytes = [Text.Encoding]::UTF8.GetBytes($canonicalSteps)
$sha = [Security.Cryptography.SHA256]::Create()
try {
    $stepSequenceSha256 = ([BitConverter]::ToString($sha.ComputeHash($stepBytes))).Replace('-', '').ToLowerInvariant()
} finally {
    $sha.Dispose()
}

$receipt = [ordered]@{
    schema = 'hoi4.a29.autonomous-research-navigation.v4'
    acquisition_owner = 'machine'
    result = 'EXACT_D04_RESEARCH_TREE_HOVER_COMPLETE_A29_HOOK_VERDICT_REQUIRED'
    timestamp_utc = [DateTime]::UtcNow.ToString('o')
    process_id = $ProcessId
    process_start_utc = $process.StartTime.ToUniversalTime().ToString('o')
    session_id = $SessionId
    capture_id = $CaptureId
    executable_sha256 = $exeSha256
    window_owner_process_id = $windowOwner
    client_size = @($width, $height)
    navigation_profile = 'New D04 Save Deep War/paused/1944.2.11.18/1920x1080'
    navigation_policy = 'exact_d04_two_read_only_ui_clicks_then_future_node_hover_no_technology_click'
    game_state_slot_rva = ('0x{0:X}' -f $gameStateSlotRva)
    clock_singleton_a_rva = ('0x{0:X}' -f $clockSingletonARva)
    clock_singleton_b_rva = ('0x{0:X}' -f $clockSingletonBRva)
    clock_vtable_rva = ('0x{0:X}' -f $clockVtableRva)
    pause_offset = ('0x{0:X}' -f $pauseOffset)
    game_hour_offset = ('0x{0:X}' -f $gameHourOffset)
    game_speed_offset = ('0x{0:X}' -f $gameSpeedOffset)
    date_epoch_raw = $dateEpochRaw
    expected_d04_game_hour_raw = $expectedD04GameHourRaw
    expected_d04_game_hour_normalized = $expectedD04GameHourNormalized
    game_hour_raw_before_navigation = $gameHourRawBeforeNavigation
    game_hour_raw_after_navigation = $gameHourRawAfterNavigation
    game_hour_normalized_before_navigation = $gameHourNormalizedBeforeNavigation
    game_hour_normalized_after_navigation = $gameHourNormalizedAfterNavigation
    game_speed_raw_before_navigation = $gameSpeedRawBeforeNavigation
    game_speed_raw_after_navigation = $gameSpeedRawAfterNavigation
    pause_before_navigation = $pauseBeforeNavigation
    pause_after_navigation = $pauseAfterNavigation
    initial_settle_seconds = $InitialSettleSeconds
    step_count = $steps.Count
    step_sequence_sha256 = $stepSequenceSha256
    steps = $steps
    final_cursor_screen = @($finalCursor.X, $finalCursor.Y)
    previously_observed_visual_diagnostic = 'Technology is 0.88 years ahead of time.'
    semantic_authority = 'exact A29 branch-1/2 hook observation and selected-Present verifier only'
    promotion_authorized = $false
}
$receiptJson = $receipt | ConvertTo-Json -Depth 8
Write-AtomicUtf8Text $ReceiptPath ($receiptJson + "`n")
$receiptJson
