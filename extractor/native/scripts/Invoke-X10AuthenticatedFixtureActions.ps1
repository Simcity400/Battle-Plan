[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $ProfilePath,
    [Parameter(Mandatory = $true)] [string] $ReceiptPath,
    [Parameter(Mandatory = $true)] [uint32] $ExistingPid,
    [Parameter(Mandatory = $true)] [string] $SessionId,
    [Parameter(Mandatory = $true)] [string] $CaptureId,
    [int] $ProcessTimeoutSeconds = 300,
    [switch] $EnableLegacyClientDriving
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $EnableLegacyClientDriving) {
    throw 'This legacy X10 client-driving tool is isolated and disabled by default. Passive observation is the production path; use -EnableLegacyClientDriving only for an explicitly authorized legacy experiment.'
}
$expectedSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedProfile = (Resolve-Path -LiteralPath $ProfilePath).Path
if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedSha256) {
    throw 'X10 action driver executable is not exact a729.'
}
$profileBytes = [IO.File]::ReadAllBytes($resolvedProfile)
$sha = [Security.Cryptography.SHA256]::Create()
try { $profileSha256 = ([BitConverter]::ToString($sha.ComputeHash($profileBytes))).Replace('-', '').ToLowerInvariant() }
finally { $sha.Dispose() }
$profile = Get-Content -LiteralPath $resolvedProfile -Raw | ConvertFrom-Json
if ($profile.schema -cne 'hoi4.x10.autonomous-owner-fixture-actions.v1' -or -not ($profile.actions -is [array])) {
    throw 'X10 action profile schema/actions are invalid.'
}
if ([string]$profile.executable_sha256 -cne $expectedSha256 -or
    [string]$profile.fixture_nonce_sha256 -cnotmatch '^[0-9a-f]{64}$' -or
    [string]$profile.product_modset_sha256 -cnotmatch '^[0-9a-f]{64}$') {
    throw 'X10 action profile authority identity is malformed.'
}
if ($profile.expected_client_width -lt 640 -or $profile.expected_client_height -lt 480) {
    throw 'X10 action profile client size is invalid.'
}

function Write-CreateNewUtf8([string]$Path, [string]$Text) {
    $full = [IO.Path]::GetFullPath($Path)
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($full)) | Out-Null
    $temporary = [IO.Path]::Combine([IO.Path]::GetDirectoryName($full), ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText($temporary, $Text, [Text.UTF8Encoding]::new($false))
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($full)) { throw "Refusing to replace immutable X10 action receipt: $full" }
        [IO.File]::Move($temporary, $full)
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class X10AuthenticatedInput {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public INPUTUNION U; }
    [StructLayout(LayoutKind.Explicit)] public struct INPUTUNION {
        [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki;
    }
    [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT {
        public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr extra;
    }
    [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT {
        public ushort vk, scan; public uint flags, time; public IntPtr extra;
    }
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr h, int command);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT rect);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT point);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint x, uint y, uint data, UIntPtr extra);
    [DllImport("user32.dll", SetLastError=true)] public static extern uint SendInput(uint count, INPUT[] input, int size);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool ReadProcessMemory(IntPtr p, IntPtr a, byte[] b, UIntPtr n, out UIntPtr read);
    public static void Key(ushort vk) {
        INPUT[] i=new INPUT[2]; i[0].type=1; i[0].U.ki.vk=vk; i[1].type=1; i[1].U.ki.vk=vk; i[1].U.ki.flags=2;
        if (SendInput(2,i,Marshal.SizeOf(typeof(INPUT)))!=2) throw new Exception("SendInput key failed");
    }
    public static void Scan(ushort scan) {
        INPUT[] i=new INPUT[2]; i[0].type=1; i[0].U.ki.scan=scan; i[0].U.ki.flags=8; i[1].type=1; i[1].U.ki.scan=scan; i[1].U.ki.flags=10;
        if (SendInput(2,i,Marshal.SizeOf(typeof(INPUT)))!=2) throw new Exception("SendInput scan failed");
    }
    public static void Text(string value) {
        foreach(char c in value) { INPUT[] i=new INPUT[2]; i[0].type=1; i[0].U.ki.scan=c; i[0].U.ki.flags=4; i[1].type=1; i[1].U.ki.scan=c; i[1].U.ki.flags=6; if(SendInput(2,i,Marshal.SizeOf(typeof(INPUT)))!=2) throw new Exception("SendInput text failed"); }
    }
}
'@

function Read-Bytes([IntPtr]$Handle, [long]$Address, [int]$Count) {
    $bytes = [byte[]]::new($Count); $read = [UIntPtr]::Zero
    if (-not [X10AuthenticatedInput]::ReadProcessMemory($Handle, [IntPtr]$Address, $bytes, [UIntPtr]::new([uint64]$Count), [ref]$read) -or $read.ToUInt64() -ne $Count) {
        throw ('ReadProcessMemory failed at 0x{0:X}' -f $Address)
    }
    return $bytes
}
function Read-U64([IntPtr]$Handle, [long]$Address) { return [BitConverter]::ToUInt64((Read-Bytes $Handle $Address 8), 0) }
function Read-I32([IntPtr]$Handle, [long]$Address) { return [BitConverter]::ToInt32((Read-Bytes $Handle $Address 4), 0) }

$deadline = [DateTime]::UtcNow.AddSeconds($ProcessTimeoutSeconds)
$process = $null
while ([DateTime]::UtcNow -lt $deadline -and $null -eq $process) {
    foreach ($candidate in @(Get-Process -Id $ExistingPid -ErrorAction SilentlyContinue)) {
        try {
            if ($candidate.MainWindowHandle -eq [IntPtr]::Zero -or
                -not [string]::Equals($candidate.MainModule.FileName, $resolvedExe, [StringComparison]::OrdinalIgnoreCase) -or
                (Get-FileHash -LiteralPath $candidate.MainModule.FileName -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedSha256) { continue }
            $loaded = @($candidate.Modules | Where-Object {
                [string]::Equals([IO.Path]::GetFullPath($_.FileName), $resolvedCollector, [StringComparison]::OrdinalIgnoreCase)
            })
            if ($loaded.Count -eq 1) { $process = $candidate; break }
        } catch { }
    }
    if ($null -eq $process) { Start-Sleep -Milliseconds 250 }
}
if ($null -eq $process) { throw 'X10 action driver did not observe one authenticated process/window/collector tuple.' }

$window = $process.MainWindowHandle
$owner = [uint32]0
[void][X10AuthenticatedInput]::GetWindowThreadProcessId($window, [ref]$owner)
if ($owner -ne $process.Id) { throw 'X10 action window owner mismatch.' }
$client = [X10AuthenticatedInput+RECT]::new()
if (-not [X10AuthenticatedInput]::GetClientRect($window, [ref]$client)) { throw 'X10 GetClientRect failed.' }
$width = $client.Right - $client.Left; $height = $client.Bottom - $client.Top
if ([int]$profile.expected_client_width -ne $width -or [int]$profile.expected_client_height -ne $height) {
    throw "X10 profile requires $($profile.expected_client_width)x$($profile.expected_client_height), observed ${width}x${height}."
}
[void][X10AuthenticatedInput]::ShowWindowAsync($window, 9)
if (-not [X10AuthenticatedInput]::SetForegroundWindow($window)) { throw 'X10 SetForegroundWindow failed.' }
Start-Sleep -Milliseconds 750

function Assert-Foreground {
    if ([X10AuthenticatedInput]::GetForegroundWindow() -ne $window) { throw 'X10 exact window lost foreground ownership.' }
}
function Read-ClockState {
    $base = $process.MainModule.BaseAddress.ToInt64(); $handle = $process.Handle
    $state = [long](Read-U64 $handle ($base + 0x3316E90L))
    if ($state -eq 0) { throw 'X10 game-state singleton is null.' }
    $clockA = Read-U64 $handle ($base + 0x33172E0L); $clockB = Read-U64 $handle ($base + 0x33172E8L)
    $clocks = @(@($clockA, $clockB) | Where-Object { $_ -ne 0 } | Select-Object -Unique)
    if ($clocks.Count -ne 1) { throw 'X10 clock singleton is ambiguous.' }
    $clock = [long]$clocks[0]
    if ((Read-U64 $handle $clock) -ne [uint64]($base + 0x2952510L)) { throw 'X10 clock vtable mismatch.' }
    return [ordered]@{
        game_state = $state
        clock = $clock
        game_hour_raw = Read-I32 $handle ($state + 0x468L)
        speed_index = Read-I32 $handle ($state + 0x4BCL)
        pause_byte = (Read-Bytes $handle ($clock + 0x6C1L) 1)[0]
    }
}
function Wait-ForLoadedPausedSave([int]$ExpectedGameHourRaw, [int]$TimeoutSeconds, [int]$StableMilliseconds) {
    if ($ExpectedGameHourRaw -le 0 -or $TimeoutSeconds -lt 1 -or $StableMilliseconds -lt 250) { throw 'X10 paused-save readiness parameters are invalid.' }
    $until = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds); $lastFailure = 'no authenticated singleton observed'
    while ([DateTime]::UtcNow -lt $until) {
        try {
            $first = Read-ClockState
            if ($first.pause_byte -ne 1 -or $first.game_hour_raw -ne $ExpectedGameHourRaw) { throw 'continued save is not the expected paused fixture hour' }
            Start-Sleep -Milliseconds $StableMilliseconds
            $second = Read-ClockState
            if ($second.game_state -ne $first.game_state -or $second.clock -ne $first.clock -or $second.pause_byte -ne 1 -or $second.game_hour_raw -ne $ExpectedGameHourRaw) {
                throw 'continued save singletons/hour were not stable'
            }
            return [ordered]@{ game_hour_raw = $ExpectedGameHourRaw; pause_byte = 1; stable_milliseconds = $StableMilliseconds; singletons_stable = $true }
        } catch { $lastFailure = $_.Exception.Message; Start-Sleep -Milliseconds 250 }
    }
    throw "X10 action driver did not observe the exact loaded paused save: $lastFailure"
}
function Move-Cursor([int]$X, [int]$Y) {
    Assert-Foreground
    if ($X -lt 0 -or $Y -lt 0 -or $X -ge $width -or $Y -ge $height) { throw "X10 client point outside profile: $X,$Y" }
    $point = [X10AuthenticatedInput+POINT]::new(); $point.X = $X; $point.Y = $Y
    if (-not [X10AuthenticatedInput]::ClientToScreen($window, [ref]$point) -or -not [X10AuthenticatedInput]::SetCursorPos($point.X, $point.Y)) {
        throw 'X10 client-to-screen cursor placement failed.'
    }
    return $point
}
function Advance-Hours([int]$Hours, [int]$TimeoutSeconds) {
    if ($Hours -lt 1 -or $TimeoutSeconds -lt 1) { throw 'X10 advance_hours bounds are invalid.' }
    $before = Read-ClockState
    if ($before.pause_byte -ne 1) { throw 'X10 advance_hours requires a paused game.' }
    $target = $before.game_hour_raw + $Hours
    [X10AuthenticatedInput]::Key(0x31) # Number 1, exact game speed one through the UI path.
    Start-Sleep -Milliseconds 100
    $speed = Read-ClockState
    if ($speed.speed_index -ne 0) { throw "X10 speed-one input did not produce exact raw speed index zero; observed $($speed.speed_index)." }
    [X10AuthenticatedInput]::Key(0x20) # Space, unpause through the UI path.
    $watch = [Diagnostics.Stopwatch]::StartNew()
    try {
        while ((Read-ClockState).game_hour_raw -lt $target) {
            if ($watch.Elapsed.TotalSeconds -ge $TimeoutSeconds) { throw "X10 clock did not reach $target" }
            Start-Sleep -Milliseconds 5
        }
    } finally {
        $current = Read-ClockState
        if ($current.pause_byte -eq 0) { [X10AuthenticatedInput]::Key(0x20); Start-Sleep -Milliseconds 100 }
    }
    $after = Read-ClockState
    if ($after.pause_byte -ne 1 -or $after.game_hour_raw -ne $target) { throw "X10 advance_hours overshot or undershot exact target $target; observed $($after.game_hour_raw)." }
    return [ordered]@{ game_hour_before = $before.game_hour_raw; game_hour_after = $after.game_hour_raw; requested_hours = $Hours; speed_index = 0; exact_target = $true; elapsed_milliseconds = $watch.ElapsedMilliseconds; repaused = $true }
}

$virtualKeys = @{ escape = 0x1B; enter = 0x0D; space = 0x20; f1 = 0x70; f2 = 0x71; f3 = 0x72; f4 = 0x73; f5 = 0x74 }
$steps = [Collections.Generic.List[object]]::new()
foreach ($action in $profile.actions) {
    Assert-Foreground
    $kind = [string]$action.kind
    $step = [ordered]@{ label = [string]$action.label; kind = $kind; started_utc = [DateTime]::UtcNow.ToString('o'); completed = $false; semantic_effect_confirmed = $false }
    switch ($kind) {
        'wait_for_loaded_paused_save' {
            $step.loaded_save = Wait-ForLoadedPausedSave ([int]$action.game_hour_raw) ([int]$action.timeout_seconds) ([int]$action.stable_milliseconds)
        }
        'console_command' {
            $command = [string]$action.command
            if ([string]::IsNullOrWhiteSpace($command) -or $command.Length -gt 1024 -or $command.Contains("`r") -or $command.Contains("`n")) { throw 'X10 console command is malformed.' }
            [X10AuthenticatedInput]::Scan(0x29); Start-Sleep -Milliseconds 250
            [X10AuthenticatedInput]::Text($command); [X10AuthenticatedInput]::Key(0x0D)
            Start-Sleep -Milliseconds ([int]$action.settle_milliseconds); [X10AuthenticatedInput]::Scan(0x29)
            $commandHasher = [Security.Cryptography.SHA256]::Create()
            try { $step.command_sha256 = ([BitConverter]::ToString($commandHasher.ComputeHash([Text.Encoding]::UTF8.GetBytes($command)))).Replace('-', '').ToLowerInvariant() }
            finally { $commandHasher.Dispose() }
        }
        'click' {
            $point = Move-Cursor ([int]$action.x) ([int]$action.y)
            [X10AuthenticatedInput]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero); [X10AuthenticatedInput]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
            Start-Sleep -Milliseconds ([int]$action.settle_milliseconds)
            $step.client_point = @([int]$action.x, [int]$action.y); $step.screen_point = @($point.X, $point.Y)
        }
        'keypress' {
            $name = ([string]$action.key).ToLowerInvariant()
            if (-not $virtualKeys.ContainsKey($name)) { throw "X10 unsupported authenticated key: $name" }
            [X10AuthenticatedInput]::Key([uint16]$virtualKeys[$name]); Start-Sleep -Milliseconds ([int]$action.settle_milliseconds)
            $step.key = $name
        }
        'advance_hours' { $step.clock = Advance-Hours ([int]$action.hours) ([int]$action.timeout_seconds) }
        'wait' {
            if ([int]$action.milliseconds -lt 0 -or [int]$action.milliseconds -gt 600000) { throw 'X10 wait is outside 0..600000 ms.' }
            Start-Sleep -Milliseconds ([int]$action.milliseconds)
        }
        default { throw "Unknown X10 action kind: $kind" }
    }
    $step.completed = $true; $step.completed_utc = [DateTime]::UtcNow.ToString('o'); $steps.Add($step)
}
$finalClock = Read-ClockState
if ($finalClock.pause_byte -ne 1) { throw 'X10 action profile did not finish with the game paused.' }
$receipt = [ordered]@{
    schema = 'hoi4.x10.authenticated-owner-fixture-action-receipt.v1'
    status = 'COMPLETED'
    semantic_authority = 'Input dispatch only; exact D45E verifier confirms every semantic transaction.'
    process_id = $process.Id
    process_start_utc = $process.StartTime.ToUniversalTime().ToString('o')
    session_id = $SessionId
    capture_id = $CaptureId
    executable_sha256 = $expectedSha256
    collector_sha256 = (Get-FileHash -LiteralPath $resolvedCollector -Algorithm SHA256).Hash.ToLowerInvariant()
    profile_sha256 = $profileSha256
    fixture_nonce_sha256 = [string]$profile.fixture_nonce_sha256
    product_modset_sha256 = [string]$profile.product_modset_sha256
    client_size = @($width, $height)
    final_game_hour_raw = $finalClock.game_hour_raw
    final_pause_byte = $finalClock.pause_byte
    steps = $steps
    production_activation_authorized = $false
}
Write-CreateNewUtf8 $ReceiptPath (($receipt | ConvertTo-Json -Depth 20) + "`n")
