[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [ValidateSet('A39', 'A40')] [string] $Owner,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $ProfilePath,
    [Parameter(Mandatory = $true)] [string] $ReceiptPath,
    [Parameter(Mandatory = $true)] [uint32] $ExistingPid,
    [int] $ProcessTimeoutSeconds = 420
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$expectedSchemas = @{
    A39 = 'hoi4.a39.authenticated-presentation-actions.v1'
    A40 = 'hoi4.a40.authenticated-design-peace-actions.v1'
}
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedProfile = (Resolve-Path -LiteralPath $ProfilePath).Path
if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) {
    throw "$Owner action driver executable is not exact a729."
}
$profile = Get-Content -LiteralPath $resolvedProfile -Raw | ConvertFrom-Json
if ($profile.schema -cne $expectedSchemas[$Owner] -or -not ($profile.actions -is [array]) -or $profile.actions.Count -eq 0) {
    throw "$Owner action profile schema/actions are invalid."
}
if ([int]$profile.expected_client_width -ne 1920 -or [int]$profile.expected_client_height -ne 1080) {
    throw "$Owner production profile must be frozen to a 1920x1080 client."
}

function Write-CreateNewUtf8([string]$Path, [string]$Text) {
    $full = [IO.Path]::GetFullPath($Path)
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($full)) | Out-Null
    $temporary = [IO.Path]::Combine([IO.Path]::GetDirectoryName($full), ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText($temporary, $Text, [Text.UTF8Encoding]::new($false))
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($full)) { throw "Refusing to replace $Owner action receipt: $full" }
        [IO.File]::Move($temporary, $full)
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class A39A40AuthenticatedInput {
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
    if (-not [A39A40AuthenticatedInput]::ReadProcessMemory($Handle, [IntPtr]$Address, $bytes, [UIntPtr]::new([uint64]$Count), [ref]$read) -or $read.ToUInt64() -ne $Count) {
        throw ('ReadProcessMemory failed at 0x{0:X}' -f $Address)
    }
    return $bytes
}
function Read-U64([IntPtr]$Handle, [long]$Address) { [BitConverter]::ToUInt64((Read-Bytes $Handle $Address 8), 0) }
function Read-I32([IntPtr]$Handle, [long]$Address) { [BitConverter]::ToInt32((Read-Bytes $Handle $Address 4), 0) }

$deadline = [DateTime]::UtcNow.AddSeconds($ProcessTimeoutSeconds)
$process = $null
while ([DateTime]::UtcNow -lt $deadline -and $null -eq $process) {
    foreach ($candidate in @(Get-Process -Id $ExistingPid -ErrorAction SilentlyContinue)) {
        try {
            if ($candidate.MainWindowHandle -eq [IntPtr]::Zero -or
                -not [IO.Path]::GetFullPath($candidate.MainModule.FileName).Equals($resolvedExe, [StringComparison]::OrdinalIgnoreCase) -or
                (Get-FileHash -LiteralPath $candidate.MainModule.FileName -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) { continue }
            $loaded = @($candidate.Modules | Where-Object {
                [IO.Path]::GetFullPath([string]$_.FileName).Equals($resolvedCollector, [StringComparison]::OrdinalIgnoreCase)
            })
            if ($loaded.Count -eq 1) { $process = $candidate; break }
        } catch { }
    }
    if ($null -eq $process) { Start-Sleep -Milliseconds 250 }
}
if ($null -eq $process) { throw "$Owner action driver did not observe one authenticated process/window/collector tuple." }

$window = $process.MainWindowHandle
$ownerPid = [uint32]0
[void][A39A40AuthenticatedInput]::GetWindowThreadProcessId($window, [ref]$ownerPid)
if ($ownerPid -ne $process.Id) { throw "$Owner main-window owner mismatch." }
$client = [A39A40AuthenticatedInput+RECT]::new()
if (-not [A39A40AuthenticatedInput]::GetClientRect($window, [ref]$client)) { throw 'GetClientRect failed.' }
$width = $client.Right - $client.Left; $height = $client.Bottom - $client.Top
if ($width -ne [int]$profile.expected_client_width -or $height -ne [int]$profile.expected_client_height) {
    throw "$Owner profile requires $($profile.expected_client_width)x$($profile.expected_client_height), observed ${width}x${height}."
}
[void][A39A40AuthenticatedInput]::ShowWindowAsync($window, 9)
if (-not [A39A40AuthenticatedInput]::SetForegroundWindow($window)) { throw 'SetForegroundWindow failed.' }
Start-Sleep -Milliseconds 750

function Assert-Foreground {
    if ([A39A40AuthenticatedInput]::GetForegroundWindow() -ne $window) { throw "$Owner exact window lost foreground ownership." }
}
function Move-Cursor([int]$X, [int]$Y) {
    Assert-Foreground
    if ($X -lt 0 -or $Y -lt 0 -or $X -ge $width -or $Y -ge $height) { throw "$Owner client point outside profile: $X,$Y" }
    $point = [A39A40AuthenticatedInput+POINT]::new(); $point.X = $X; $point.Y = $Y
    if (-not [A39A40AuthenticatedInput]::ClientToScreen($window, [ref]$point) -or -not [A39A40AuthenticatedInput]::SetCursorPos($point.X, $point.Y)) {
        throw 'Authenticated cursor positioning failed.'
    }
    return $point
}
function Click([int]$X, [int]$Y, [int]$Count) {
    $point = Move-Cursor $X $Y
    foreach ($ordinal in 1..$Count) {
        [A39A40AuthenticatedInput]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
        [A39A40AuthenticatedInput]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
        if ($ordinal -lt $Count) { Start-Sleep -Milliseconds 100 }
    }
    return $point
}
function Wait-LoadedPaused([int]$ExpectedHour, [int]$TimeoutSeconds, [int]$StableMilliseconds) {
    $until = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds); $last = 'not ready'
    while ([DateTime]::UtcNow -lt $until) {
        try {
            $base = $process.MainModule.BaseAddress.ToInt64(); $handle = $process.Handle
            $state = [long](Read-U64 $handle ($base + 0x3316E90L))
            $clockA = [long](Read-U64 $handle ($base + 0x33172E0L)); $clockB = [long](Read-U64 $handle ($base + 0x33172E8L))
            if ($state -eq 0 -or $clockA -eq 0 -or $clockA -ne $clockB) { throw 'singleton identity is not stable' }
            if ((Read-U64 $handle $clockA) -ne [uint64]($base + 0x2952510L)) { throw 'clock vtable mismatch' }
            $hour = Read-I32 $handle ($state + 0x468L); $pause = (Read-Bytes $handle ($clockA + 0x6C1L) 1)[0]
            if ($hour -ne $ExpectedHour -or $pause -ne 1) { throw "pause=$pause hour=$hour" }
            Start-Sleep -Milliseconds $StableMilliseconds
            if ((Read-I32 $handle ($state + 0x468L)) -eq $ExpectedHour -and (Read-Bytes $handle ($clockA + 0x6C1L) 1)[0] -eq 1) {
                return [ordered]@{ game_hour_raw = $ExpectedHour; pause_byte = 1; stable_milliseconds = $StableMilliseconds }
            }
        } catch { $last = $_.Exception.Message }
        Start-Sleep -Milliseconds 250
    }
    throw "$Owner action driver did not authenticate the loaded paused save: $last"
}

$virtualKeys = @{ escape = 0x1B; enter = 0x0D; space = 0x20; tab = 0x09; f2 = 0x71; f3 = 0x72; f5 = 0x74; left = 0x25; up = 0x26; right = 0x27; down = 0x28 }
$steps = [Collections.Generic.List[object]]::new()
foreach ($action in $profile.actions) {
    Assert-Foreground
    $kind = [string]$action.kind
    $step = [ordered]@{ label = [string]$action.label; kind = $kind; started_utc = [DateTime]::UtcNow.ToString('o') }
    switch ($kind) {
        'wait_for_loaded_paused_save' {
            $step.loaded_save = Wait-LoadedPaused ([int]$action.game_hour_raw) ([int]$action.timeout_seconds) ([int]$action.stable_milliseconds)
        }
        'console_command' {
            [A39A40AuthenticatedInput]::Scan(0x29); Start-Sleep -Milliseconds 250
            [A39A40AuthenticatedInput]::Text([string]$action.command); [A39A40AuthenticatedInput]::Key(0x0D)
            Start-Sleep -Milliseconds ([int]$action.settle_milliseconds); [A39A40AuthenticatedInput]::Scan(0x29)
            $hasher = [Security.Cryptography.SHA256]::Create()
            try {
                $step.command_sha256 = ([BitConverter]::ToString($hasher.ComputeHash([Text.Encoding]::UTF8.GetBytes([string]$action.command))).Replace('-', '').ToLowerInvariant())
            } finally { $hasher.Dispose() }
        }
        'key' {
            $keyName = ([string]$action.key).ToLowerInvariant()
            if (-not $virtualKeys.ContainsKey($keyName)) { throw "Unknown virtual key name: $keyName" }
            [A39A40AuthenticatedInput]::Key([uint16]$virtualKeys[$keyName]); Start-Sleep -Milliseconds ([int]$action.settle_milliseconds)
            $step.key = $keyName
        }
        'click' {
            $point = Click ([int]$action.x) ([int]$action.y) 1; Start-Sleep -Milliseconds ([int]$action.settle_milliseconds)
            $step.client_point = @([int]$action.x, [int]$action.y); $step.screen_point = @($point.X, $point.Y)
        }
        'double_click' {
            $point = Click ([int]$action.x) ([int]$action.y) 2; Start-Sleep -Milliseconds ([int]$action.settle_milliseconds)
            $step.client_point = @([int]$action.x, [int]$action.y); $step.screen_point = @($point.X, $point.Y)
        }
        'hover' {
            $point = Move-Cursor ([int]$action.x) ([int]$action.y); Start-Sleep -Milliseconds ([int]$action.dwell_milliseconds)
            $step.client_point = @([int]$action.x, [int]$action.y); $step.screen_point = @($point.X, $point.Y)
        }
        'grid_sweep' {
            $points = [Collections.Generic.List[object]]::new()
            $columns = [int]$action.columns; $rows = [int]$action.rows
            if ($columns -lt 1 -or $columns -gt 32 -or $rows -lt 1 -or $rows -gt 32) { throw 'grid_sweep bounds are invalid.' }
            for ($row = 0; $row -lt $rows; $row++) {
                for ($column = 0; $column -lt $columns; $column++) {
                    $x = [int]$action.first_x + ($column * [int]$action.stride_x); $y = [int]$action.first_y + ($row * [int]$action.stride_y)
                    $point = if ([bool]$action.click) { Click $x $y 1 } else { Move-Cursor $x $y }
                    Start-Sleep -Milliseconds ([int]$action.dwell_milliseconds)
                    $points.Add([ordered]@{ row = $row; column = $column; client_point = @($x, $y); screen_point = @($point.X, $point.Y) })
                }
            }
            $step.points = $points
        }
        'wait' { Start-Sleep -Milliseconds ([int]$action.milliseconds) }
        default { throw "Unknown $Owner action kind: $kind" }
    }
    $step.completed_utc = [DateTime]::UtcNow.ToString('o'); $steps.Add($step)
}

$receipt = [ordered]@{
    schema = 'hoi4.authenticated-ui-action-receipt.v1'
    owner = $Owner
    authority = 'diagnostic action provenance; collector receipt and independent regrade are sole semantic evidence'
    process_id = $process.Id
    process_start_utc = $process.StartTime.ToUniversalTime().ToString('o')
    executable_sha256 = $expectedExeSha256
    collector_sha256 = (Get-FileHash -LiteralPath $resolvedCollector -Algorithm SHA256).Hash.ToLowerInvariant()
    profile_schema = [string]$profile.schema
    profile_id = [string]$profile.profile_id
    profile_sha256 = (Get-FileHash -LiteralPath $resolvedProfile -Algorithm SHA256).Hash.ToLowerInvariant()
    client_size = @($width, $height)
    steps = $steps
    semantic_observations_emitted = $false
    production_activation_authorized = $false
}
Write-CreateNewUtf8 $ReceiptPath (($receipt | ConvertTo-Json -Depth 24) + "`n")
