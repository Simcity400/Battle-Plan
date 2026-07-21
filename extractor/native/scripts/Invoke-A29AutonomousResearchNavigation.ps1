[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [int] $ProcessId,
    [Parameter(Mandatory = $true)] [string] $ReceiptPath,
    [int] $ExpectedClientWidth = 1920,
    [int] $ExpectedClientHeight = 1080,
    [int] $InitialSettleSeconds = 75,
    [int] $UiSettleMilliseconds = 3000,
    [int] $FutureNodeHoverDwellMilliseconds = 5000
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'

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
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
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
if (-not [Hoi4A29AutonomousNavigation]::SetForegroundWindow($window)) {
    throw 'Fail closed: SetForegroundWindow rejected the exact-a729 window.'
}
Start-Sleep -Milliseconds 750

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
    settle_milliseconds = $UiSettleMilliseconds
})
Start-Sleep -Milliseconds $UiSettleMilliseconds

$move = Move-AuthenticatedCursor 195 340 'open_active_engineer_company_iii'
[Hoi4A29AutonomousNavigation]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
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

$canonicalSteps = ($steps | ForEach-Object {
    $delay = if ($_.PSObject.Properties['settle_milliseconds']) { $_.settle_milliseconds } else { $_.dwell_milliseconds }
    '{0}|{1}|{2}|{3}|{4}|{5}' -f $_.action, $_.label, $_.client_point[0], $_.client_point[1], $_.method, $delay
}) -join ';'
$stepBytes = [Text.Encoding]::UTF8.GetBytes($canonicalSteps)
$sha = [Security.Cryptography.SHA256]::Create()
try {
    $stepSequenceSha256 = ([BitConverter]::ToString($sha.ComputeHash($stepBytes))).Replace('-', '').ToLowerInvariant()
} finally {
    $sha.Dispose()
}

$receipt = [ordered]@{
    schema = 'hoi4.a29.autonomous-research-navigation.v2'
    acquisition_owner = 'machine'
    result = 'EXACT_D04_RESEARCH_TREE_HOVER_COMPLETE_A29_HOOK_VERDICT_REQUIRED'
    timestamp_utc = [DateTime]::UtcNow.ToString('o')
    process_id = $ProcessId
    executable_path = $exePath
    executable_sha256 = $exeSha256
    window = ('0x{0:X}' -f $window.ToInt64())
    window_owner_process_id = $windowOwner
    client_size = @($width, $height)
    navigation_profile = 'New D04 Save Deep War/paused/1944.2.11.18/1920x1080'
    navigation_policy = 'exact_d04_two_read_only_ui_clicks_then_future_node_hover_no_technology_click'
    initial_settle_seconds = $InitialSettleSeconds
    step_count = $steps.Count
    step_sequence_sha256 = $stepSequenceSha256
    steps = $steps
    final_cursor_screen = @($finalCursor.X, $finalCursor.Y)
    expected_visual_diagnostic = 'Technology is 0.88 years ahead of time.'
    semantic_authority = 'exact A29 branch-1/2 hook observation and selected-Present verifier only'
    promotion_authorized = $false
}
$receiptJson = $receipt | ConvertTo-Json -Depth 8
Write-AtomicUtf8Text $ReceiptPath ($receiptJson + "`n")
$receiptJson
