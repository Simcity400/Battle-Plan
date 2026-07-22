[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][uint32]$ProcessId,
    [Parameter(Mandatory = $true)][string]$SessionId,
    [Parameter(Mandatory = $true)][string]$CaptureId,
    [Parameter(Mandatory = $true)][string]$ExpectedProcessStartUtc,
    [Parameter(Mandatory = $true)][string]$ReceiptPath,
    [Parameter(Mandatory = $true)][string]$ReadyPath,
    [int]$DurationMilliseconds = 60000,
    [int]$BaselineHoldMilliseconds = 1500,
    [int]$MarginPixels = 96
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
if ($DurationMilliseconds -lt 5000 -or $DurationMilliseconds -gt 120000) {
    throw 'DurationMilliseconds must be in 5000..120000.'
}
if ($BaselineHoldMilliseconds -lt 500 -or $BaselineHoldMilliseconds -gt 10000) {
    throw 'BaselineHoldMilliseconds must be in 500..10000.'
}
if ($MarginPixels -lt 16 -or $MarginPixels -gt 512) {
    throw 'MarginPixels must be in 16..512.'
}
if (Test-Path -LiteralPath $ReceiptPath) {
    throw "A32 action receipt is create-new and already exists: $ReceiptPath"
}
if (Test-Path -LiteralPath $ReadyPath) {
    throw "A32 ready receipt is create-new and already exists: $ReadyPath"
}

function Write-CreateNewAtomicJson([string]$Path, $Value) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($fullPath)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText(
            $temporary,
            (($Value | ConvertTo-Json -Depth 16) + "`n"),
            [Text.UTF8Encoding]::new($false)
        )
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
public static class A32PointerInput {
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int command);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hWnd, ref POINT point);
    [DllImport("user32.dll")] public static extern bool ScreenToClient(IntPtr hWnd, ref POINT point);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT point);
    [DllImport("user32.dll")] public static extern void keybd_event(byte virtualKey, byte scan, uint flags, UIntPtr extraInfo);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    public static extern IntPtr CreateWindowExW(uint exStyle, string className, string title, uint style,
        int x, int y, int width, int height, IntPtr parent, IntPtr menu, IntPtr instance, IntPtr parameter);
    [DllImport("user32.dll")] public static extern bool DestroyWindow(IntPtr hWnd);
    [DllImport("kernel32.dll")] public static extern bool QueryPerformanceCounter(out long value);
    [DllImport("kernel32.dll")] public static extern bool QueryPerformanceFrequency(out long value);
}
'@

function Get-Qpc {
    $value = [int64]0
    if (-not [A32PointerInput]::QueryPerformanceCounter([ref]$value) -or $value -le 0) {
        throw 'QueryPerformanceCounter failed.'
    }
    return $value
}

function Set-ExactForeground([IntPtr]$Window, [uint32]$ExpectedOwner, [string]$Label) {
    for ($attempt = 0; $attempt -lt 10; $attempt++) {
        [A32PointerInput]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero)
        [void][A32PointerInput]::SetForegroundWindow($Window)
        [A32PointerInput]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 50
        $foregroundWindow = [A32PointerInput]::GetForegroundWindow()
        $observedOwner = [uint32]0
        [void][A32PointerInput]::GetWindowThreadProcessId($foregroundWindow, [ref]$observedOwner)
        if ($foregroundWindow -eq $Window -and $observedOwner -eq $ExpectedOwner) { return }
    }
    throw "A32 could not establish $Label foreground ownership for process $ExpectedOwner."
}

$process = Get-Process -Id $ProcessId -ErrorAction Stop
$exePath = $process.MainModule.FileName
$exeSha256 = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($exeSha256 -cne $expectedExeSha256) { throw "Exact-a729 executable mismatch: $exeSha256" }
$processStartUtc = $process.StartTime.ToUniversalTime().ToString('o')
if ($processStartUtc -cne $ExpectedProcessStartUtc) {
    throw "A32 process lifetime mismatch: $processStartUtc"
}
$window = $process.MainWindowHandle
if ($window -eq [IntPtr]::Zero) { throw 'Exact-a729 process has no main window.' }
$owner = [uint32]0
[void][A32PointerInput]::GetWindowThreadProcessId($window, [ref]$owner)
if ($owner -ne $ProcessId) { throw "A32 main-window owner mismatch: $owner" }
$rect = [A32PointerInput+RECT]::new()
if (-not [A32PointerInput]::GetClientRect($window, [ref]$rect)) { throw 'GetClientRect failed.' }
$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
if ($width -le (2 * $MarginPixels) -or $height -le (2 * $MarginPixels)) {
    throw "A32 client is too small for two interior pointer targets: ${width}x${height}"
}
$targets = @(
    @($MarginPixels, $MarginPixels),
    @(($width - 1 - $MarginPixels), ($height - 1 - $MarginPixels))
)
$frequency = [int64]0
if (-not [A32PointerInput]::QueryPerformanceFrequency([ref]$frequency) -or $frequency -le 0) {
    throw 'QueryPerformanceFrequency failed.'
}

[void][A32PointerInput]::ShowWindowAsync($window, 9)
Set-ExactForeground $window $ProcessId 'game preflight'
Start-Sleep -Milliseconds 500
$foreground = [A32PointerInput]::GetForegroundWindow()
$foregroundOwner = [uint32]0
[void][A32PointerInput]::GetWindowThreadProcessId($foreground, [ref]$foregroundOwner)
if ($foregroundOwner -ne $ProcessId) { throw "A32 target is not foreground: $foregroundOwner" }

$actionStartedQpc = Get-Qpc
$resetWindow = [A32PointerInput]::CreateWindowExW(0, 'STATIC', 'A32 observer reset', 0x10CF0000, 0, 0, 64, 64, [IntPtr]::Zero, [IntPtr]::Zero, [IntPtr]::Zero, [IntPtr]::Zero)
if ($resetWindow -eq [IntPtr]::Zero) { throw 'A32 could not create its foreign-foreground reset window.' }
$resetStartedQpc = Get-Qpc
try {
    Set-ExactForeground $resetWindow ([uint32][Diagnostics.Process]::GetCurrentProcess().Id) 'observer reset'
    Start-Sleep -Milliseconds 750
    $resetForeground = [A32PointerInput]::GetForegroundWindow()
    $resetOwner = [uint32]0
    [void][A32PointerInput]::GetWindowThreadProcessId($resetForeground, [ref]$resetOwner)
    if ($resetOwner -ne [Diagnostics.Process]::GetCurrentProcess().Id -or $resetOwner -eq $ProcessId) {
        throw "A32 foreign-foreground reset owner mismatch: $resetOwner"
    }
} finally {
    [void][A32PointerInput]::DestroyWindow($resetWindow)
}
$resetCompletedQpc = Get-Qpc

# Place the baseline while the foreign window still owns foreground. Therefore
# the first collector poll after game foreground restoration can only observe
# target zero; it cannot resurrect the pre-reset cursor as the new baseline.
$baselineScreen = [A32PointerInput+POINT]::new()
$baselineScreen.X = $targets[0][0]
$baselineScreen.Y = $targets[0][1]
if (-not [A32PointerInput]::ClientToScreen($window, [ref]$baselineScreen) -or
    -not [A32PointerInput]::SetCursorPos($baselineScreen.X, $baselineScreen.Y)) {
    throw 'A32 could not pre-place the post-reset baseline pointer.'
}
[void][A32PointerInput]::ShowWindowAsync($window, 9)
Set-ExactForeground $window $ProcessId 'game after observer reset'
$actions = [Collections.Generic.List[object]]::new()
for ($ordinal = 0; $ordinal -lt 2; $ordinal++) {
    $target = $targets[$ordinal]
    $clientPoint = [A32PointerInput+POINT]::new()
    $clientPoint.X = $target[0]
    $clientPoint.Y = $target[1]
    $screenPoint = [A32PointerInput+POINT]::new()
    $screenPoint.X = $clientPoint.X
    $screenPoint.Y = $clientPoint.Y
    if (-not [A32PointerInput]::ClientToScreen($window, [ref]$screenPoint)) { throw 'ClientToScreen failed.' }
    $moveStartedQpc = Get-Qpc
    if (-not [A32PointerInput]::SetCursorPos($screenPoint.X, $screenPoint.Y)) { throw 'SetCursorPos failed.' }
    Start-Sleep -Milliseconds ([Math]::Min(20, [Math]::Max(1, [int]($IntervalMilliseconds / 4))))
    $observedScreen = [A32PointerInput+POINT]::new()
    if (-not [A32PointerInput]::GetCursorPos([ref]$observedScreen)) { throw 'GetCursorPos failed.' }
    $observedClient = [A32PointerInput+POINT]::new()
    $observedClient.X = $observedScreen.X
    $observedClient.Y = $observedScreen.Y
    if (-not [A32PointerInput]::ScreenToClient($window, [ref]$observedClient)) { throw 'ScreenToClient failed.' }
    $moveCompletedQpc = Get-Qpc
    if ($observedClient.X -ne $clientPoint.X -or $observedClient.Y -ne $clientPoint.Y) {
        throw "Pointer left authenticated A32 target ($($clientPoint.X),$($clientPoint.Y))."
    }
    $foreground = [A32PointerInput]::GetForegroundWindow()
    [void][A32PointerInput]::GetWindowThreadProcessId($foreground, [ref]$foregroundOwner)
    if ($foregroundOwner -ne $ProcessId) { throw "A32 foreground ownership changed: $foregroundOwner" }
    $actions.Add([ordered]@{
        ordinal = $ordinal
        target_index = $ordinal
        client_position = @($observedClient.X, $observedClient.Y)
        screen_position = @($observedScreen.X, $observedScreen.Y)
        move_started_qpc_ticks = [string]$moveStartedQpc
        move_completed_qpc_ticks = [string]$moveCompletedQpc
    })
    if ($ordinal -eq 0) {
        Start-Sleep -Milliseconds $BaselineHoldMilliseconds
    } else {
        $ready = [ordered]@{
            schema = 'hoi4.a32.pointer-current-position-ready.v1'
            status = 'PASS'
            session_id = $SessionId
            capture_id = $CaptureId
            process_id = $ProcessId
            process_start_utc = $processStartUtc
            executable_sha256 = $exeSha256
            client_extent = @($width, $height)
            baseline_client_position = @($targets[0][0], $targets[0][1])
            current_client_position = @($targets[1][0], $targets[1][1])
            reset_started_qpc_ticks = [string]$resetStartedQpc
            reset_completed_qpc_ticks = [string]$resetCompletedQpc
            current_position_completed_qpc_ticks = [string]$moveCompletedQpc
        }
        Write-CreateNewAtomicJson $ReadyPath $ready
        Start-Sleep -Milliseconds $DurationMilliseconds
    }
}
$actionCompletedQpc = Get-Qpc
if ($actions.Count -ne 2) { throw 'A32 driver did not complete its exact baseline/current pointer pair.' }

$receipt = [ordered]@{
    schema = 'hoi4.a32.authenticated-pointer-movement-actions.v1'
    status = 'PASS'
    session_id = $SessionId
    capture_id = $CaptureId
    process_id = $ProcessId
    process_start_utc = $processStartUtc
    executable_sha256 = $exeSha256
    window = ('0x{0:X}' -f $window.ToInt64())
    foreground_owner_process_id = $foregroundOwner
    client_extent = @($width, $height)
    qpc_frequency_hz = [string]$frequency
    action_started_qpc_ticks = [string]$actionStartedQpc
    action_completed_qpc_ticks = [string]$actionCompletedQpc
    duration_milliseconds = $DurationMilliseconds
    baseline_hold_milliseconds = $BaselineHoldMilliseconds
    foreign_foreground_reset_process_id = [Diagnostics.Process]::GetCurrentProcess().Id
    foreign_foreground_reset_started_qpc_ticks = [string]$resetStartedQpc
    foreign_foreground_reset_completed_qpc_ticks = [string]$resetCompletedQpc
    target_positions = @(
        [ordered]@{ x = $targets[0][0]; y = $targets[0][1] },
        [ordered]@{ x = $targets[1][0]; y = $targets[1][1] }
    )
    action_count = $actions.Count
    actions = @($actions)
}
Write-CreateNewAtomicJson $ReceiptPath $receipt
$receipt | ConvertTo-Json -Depth 16
