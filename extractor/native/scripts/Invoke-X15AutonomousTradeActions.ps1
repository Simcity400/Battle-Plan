[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [switch] $EnableLegacyClientDriving,
    [Parameter(Mandatory = $true)] [int] $ProcessId,
    [Parameter(Mandatory = $true)] [string] $TopbarGuiPath,
    [Parameter(Mandatory = $true)] [string] $ReceiptPath,
    [Parameter(Mandatory = $true)] [string] $SessionId,
    [Parameter(Mandatory = $true)] [string] $CaptureId,
    [int] $ExpectedClientWidth = 1920,
    [int] $ExpectedClientHeight = 1080,
    [int] $BaselineClosedDwellMilliseconds = 3000,
    [int] $OpenDwellMilliseconds = 5000,
    [int] $ClosedDwellMilliseconds = 3000
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not $EnableLegacyClientDriving) {
    throw 'This isolated legacy tool drives the game client. It is never a default or promotion dependency; use passive MTRA observation instead.'
}

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$expectedTopbarGuiSha256 = 'ba17f2348922f45a4ef71808d33de1cd10154cdc54e1a1bc8585147b0b69976f'
$expectedD04GameHourRaw = 60830441
$gameStateSlotRva = 0x3316E90
$gameHourOffset = 0x468
$clockSingletonARva = 0x33172E0
$clockSingletonBRva = 0x33172E8
$clockVtableRva = 0x2952510
$pauseOffset = 0x6C1

foreach ($identity in @($SessionId, $CaptureId)) {
    if ($identity -cnotmatch '^[a-z0-9][a-z0-9_.-]{7,159}$') {
        throw "Session/capture identity is noncanonical: $identity"
    }
}
if ($BaselineClosedDwellMilliseconds -lt 1000 -or $BaselineClosedDwellMilliseconds -gt 30000 -or
    $OpenDwellMilliseconds -lt 1000 -or $OpenDwellMilliseconds -gt 30000 -or
    $ClosedDwellMilliseconds -lt 1000 -or $ClosedDwellMilliseconds -gt 30000) {
    throw 'Dwell milliseconds must be in 1000..30000.'
}

function Write-AtomicUtf8([string]$Path, [string]$Text) {
    $full = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($full)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText($temporary, $Text, [Text.UTF8Encoding]::new($false))
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($full)) { throw "Refusing to replace X15 action receipt: $full" }
        [IO.File]::Move($temporary, $full)
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Hoi4X15TradeAction {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll", SetLastError=true)] public static extern bool GetClientRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr window);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern void keybd_event(byte virtualKey, byte scanCode, uint flags, UIntPtr extra);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool ReadProcessMemory(IntPtr process, IntPtr address, byte[] buffer, UIntPtr size, out UIntPtr read);
}
'@

$process = Get-Process -Id $ProcessId -ErrorAction Stop
$exePath = $process.MainModule.FileName
$exeSha256 = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($exeSha256 -cne $expectedExeSha256) { throw "Fail closed: exact-a729 executable mismatch: $exeSha256" }
$topbarGui = (Resolve-Path -LiteralPath $TopbarGuiPath).Path
$topbarGuiSha256 = (Get-FileHash -LiteralPath $topbarGui -Algorithm SHA256).Hash.ToLowerInvariant()
if ($topbarGuiSha256 -cne $expectedTopbarGuiSha256) { throw "Fail closed: authored topbar.gui identity mismatch: $topbarGuiSha256" }
$topbarText = [IO.File]::ReadAllText($topbarGui)
if ($topbarText -cnotmatch '(?s)name\s*=\s*"trade_button".*?shortcut\s*=\s*"r"') {
    throw 'Fail closed: authored trade_button shortcut r was not found in the pinned topbar.gui.'
}

$window = $process.MainWindowHandle
if ($window -eq [IntPtr]::Zero) { throw 'Fail closed: exact-a729 process has no main window.' }
$owner = [uint32]0
[void][Hoi4X15TradeAction]::GetWindowThreadProcessId($window, [ref]$owner)
if ($owner -ne $ProcessId) { throw 'Fail closed: main-window owner does not match exact process.' }
$rect = [Hoi4X15TradeAction+RECT]::new()
if (-not [Hoi4X15TradeAction]::GetClientRect($window, [ref]$rect) -or
    ($rect.Right - $rect.Left) -ne $ExpectedClientWidth -or
    ($rect.Bottom - $rect.Top) -ne $ExpectedClientHeight) {
    throw 'Fail closed: X15 action profile requires the exact 1920x1080 client.'
}
[void][Hoi4X15TradeAction]::ShowWindowAsync($window, 9)
[void][Hoi4X15TradeAction]::BringWindowToTop($window)
if (-not [Hoi4X15TradeAction]::SetForegroundWindow($window)) { throw 'Fail closed: foreground transfer rejected.' }
Start-Sleep -Milliseconds 500
if ([Hoi4X15TradeAction]::GetForegroundWindow() -ne $window) { throw 'Fail closed: exact game window is not foreground.' }

function Read-Bytes([long]$Address, [int]$Count, [string]$Label) {
    $bytes = [byte[]]::new($Count)
    $read = [UIntPtr]::Zero
    if (-not [Hoi4X15TradeAction]::ReadProcessMemory($process.Handle, [IntPtr]$Address, $bytes, [UIntPtr]::new([uint64]$Count), [ref]$read) -or
        $read.ToUInt64() -ne [uint64]$Count) { throw "Fail closed: ReadProcessMemory failed for $Label." }
    return $bytes
}
function Read-U64([long]$Address, [string]$Label) { [BitConverter]::ToUInt64((Read-Bytes $Address 8 $Label), 0) }
function Read-I32([long]$Address, [string]$Label) { [BitConverter]::ToInt32((Read-Bytes $Address 4 $Label), 0) }
function Assert-PausedD04([string]$Label) {
    if ([Hoi4X15TradeAction]::GetForegroundWindow() -ne $window) { throw "Fail closed: foreground changed before $Label." }
    $base = $process.MainModule.BaseAddress.ToInt64()
    $clockA = Read-U64 ($base + $clockSingletonARva) 'clock singleton A'
    $clockB = Read-U64 ($base + $clockSingletonBRva) 'clock singleton B'
    $clocks = @(@($clockA, $clockB) | Where-Object { $_ -ne 0 } | Select-Object -Unique)
    if ($clocks.Count -ne 1) { throw 'Fail closed: exact D04 clock singleton is ambiguous.' }
    $clock = [long]$clocks[0]
    if ((Read-U64 $clock 'clock vtable') -ne [uint64]($base + $clockVtableRva)) { throw 'Fail closed: clock vtable identity mismatch.' }
    $gameState = [long](Read-U64 ($base + $gameStateSlotRva) 'game-state singleton')
    $hour = Read-I32 ($gameState + $gameHourOffset) 'raw game hour'
    $paused = (Read-Bytes ($clock + $pauseOffset) 1 'pause byte')[0]
    if ($hour -ne $expectedD04GameHourRaw -or $paused -ne 1) { throw "Fail closed: expected paused D04 at raw hour $expectedD04GameHourRaw; observed hour=$hour pause=$paused." }
}
function Send-TradeShortcut([string]$Label) {
    Assert-PausedD04 $Label
    # VK_R / scan 0x13 comes directly from authored topbar.gui shortcut="r".
    [Hoi4X15TradeAction]::keybd_event(0x52, 0x13, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 80
    [Hoi4X15TradeAction]::keybd_event(0x52, 0x13, 2, [UIntPtr]::Zero)
    return [DateTime]::UtcNow.ToString('o')
}

$beforeUtc = [DateTime]::UtcNow.ToString('o')
Start-Sleep -Milliseconds $BaselineClosedDwellMilliseconds
$openUtc = Send-TradeShortcut 'open country trade'
Start-Sleep -Milliseconds $OpenDwellMilliseconds
$closeUtc = Send-TradeShortcut 'close country trade'
Start-Sleep -Milliseconds $ClosedDwellMilliseconds
Assert-PausedD04 'seal X15 action receipt'
$receipt = [ordered]@{
    schema = 'hoi4-x15-autonomous-trade-actions-v1'
    status = 'ACTION_SEQUENCE_COMPLETE_NOT_PIXEL_ADMISSION'
    session_id = $SessionId
    capture_id = $CaptureId
    process_id = $ProcessId
    executable_sha256 = $exeSha256
    client_size = @($ExpectedClientWidth, $ExpectedClientHeight)
    authored_source = 'interface/topbar.gui'
    authored_source_sha256 = $topbarGuiSha256
    authored_control = 'trade_button'
    authored_shortcut = 'r'
    game_hour_raw = $expectedD04GameHourRaw
    paused = $true
    sequence = @(
        [ordered]@{ action = 'baseline_closed_dwell'; utc = $beforeUtc; dwell_milliseconds = $BaselineClosedDwellMilliseconds },
        [ordered]@{ action = 'open_countrytradeview'; method = 'physical_keybd_event'; virtual_key = '0x52'; scan_code = '0x13'; utc = $openUtc; dwell_milliseconds = $OpenDwellMilliseconds },
        [ordered]@{ action = 'close_countrytradeview'; method = 'physical_keybd_event'; virtual_key = '0x52'; scan_code = '0x13'; utc = $closeUtc; dwell_milliseconds = $ClosedDwellMilliseconds }
    )
    admission_requirement = 'authenticated MTRA closed-active-closed transition joined to same-session d43 and replay pixel output'
}
Write-AtomicUtf8 $ReceiptPath (($receipt | ConvertTo-Json -Depth 8) + [Environment]::NewLine)
Write-Output "X15 autonomous trade action sequence complete: $ReceiptPath"
