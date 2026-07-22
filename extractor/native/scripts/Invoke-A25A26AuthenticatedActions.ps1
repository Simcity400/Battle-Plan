[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $ProfilePath,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [Parameter(Mandatory = $true)] [string] $ReceiptPath,
    [int] $ProcessTimeoutSeconds = 420
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$expectedA26Cases = @(
    'hidden_baseline_1920x1080',
    'capture_start_visible_1920x1080',
    'show_1920x1080',
    'hide_1920x1080',
    'destroyed_1920x1080',
    'owner_change_1920x1080',
    'multiline_dynamic_1920x1080',
    'edge_geometry_1920x1080',
    'edge_geometry_2560x1440'
)

function Write-CreateNewUtf8([string]$Path, [string]$Text) {
    $full = [IO.Path]::GetFullPath($Path)
    if ([IO.File]::Exists($full)) { throw "Refusing to replace immutable A25/A26 action artifact: $full" }
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($full)) | Out-Null
    $temporary = [IO.Path]::Combine([IO.Path]::GetDirectoryName($full), ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText($temporary, $Text, [Text.UTF8Encoding]::new($false))
        [IO.File]::Move($temporary, $full)
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

foreach ($path in @($Hoi4Exe, $CollectorDll, $ProfilePath)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Required file does not exist: $path" }
}
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedProfile = (Resolve-Path -LiteralPath $ProfilePath).Path
$resolvedOut = [IO.Path]::GetFullPath($OutDir)
$resolvedReceipt = [IO.Path]::GetFullPath($ReceiptPath)
if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) {
    throw 'A25/A26 action driver executable is not exact a729.'
}
if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container)) {
    throw "Harness output directory must exist before the action driver starts: $resolvedOut"
}
if ([IO.File]::Exists($resolvedReceipt)) { throw "Action receipt must be create-new: $resolvedReceipt" }

$profile = Get-Content -LiteralPath $resolvedProfile -Raw | ConvertFrom-Json
if ($profile.schema -cne 'hoi4.a25-a26.authenticated-tooltip-glyph-actions.v1' -or -not ($profile.actions -is [array])) {
    throw 'A25/A26 action profile schema/actions are invalid.'
}
if ([string]$profile.authority.executable_sha256 -cne $expectedExeSha256) { throw 'Action profile executable authority mismatch.' }
$profileA26Cases = @($profile.actions | Where-Object { $_.kind -ceq 'capture_case' -and $_.lane -ceq 'A26' } | ForEach-Object { [string]$_.case_id })
if ($profileA26Cases.Count -ne $expectedA26Cases.Count -or @($profileA26Cases | Sort-Object -Unique).Count -ne $expectedA26Cases.Count) {
    throw 'Action profile does not contain exactly one request for every A26 matrix case.'
}
foreach ($case in $expectedA26Cases) { if ($profileA26Cases -cnotcontains $case) { throw "Action profile is missing A26 case $case." } }
$profileA25Cases = @($profile.actions | Where-Object { $_.kind -ceq 'capture_case' -and $_.lane -ceq 'A25' })
if ($profileA25Cases.Count -ne 1 -or [string]$profileA25Cases[0].case_id -cne 'mixed_glyph_inline_icon_1920x1080') {
    throw 'Action profile must contain exactly one A25 mixed-glyph/inline-icon request.'
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class A25A26AuthenticatedInput {
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
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT rect);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT point);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
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
    if (-not [A25A26AuthenticatedInput]::ReadProcessMemory($Handle, [IntPtr]$Address, $bytes, [UIntPtr]::new([uint64]$Count), [ref]$read) -or $read.ToUInt64() -ne $Count) {
        throw ('ReadProcessMemory failed at 0x{0:X}' -f $Address)
    }
    $bytes
}
function Read-U64([IntPtr]$Handle,[long]$Address) { [BitConverter]::ToUInt64((Read-Bytes $Handle $Address 8),0) }
function Read-I32([IntPtr]$Handle,[long]$Address) { [BitConverter]::ToInt32((Read-Bytes $Handle $Address 4),0) }

$deadline = [DateTime]::UtcNow.AddSeconds($ProcessTimeoutSeconds); $process = $null
while ([DateTime]::UtcNow -lt $deadline -and $null -eq $process) {
    foreach ($candidate in @(Get-Process -Name the game -ErrorAction SilentlyContinue)) {
        try {
            if ($candidate.MainWindowHandle -eq [IntPtr]::Zero) { continue }
            if (-not [string]::Equals($candidate.MainModule.FileName, $resolvedExe, [StringComparison]::OrdinalIgnoreCase)) { continue }
            if ((Get-FileHash -LiteralPath $candidate.MainModule.FileName -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) { continue }
            $loaded = @($candidate.Modules | Where-Object { [string]::Equals($_.FileName, $resolvedCollector, [StringComparison]::OrdinalIgnoreCase) })
            if ($loaded.Count -eq 1) { $process = $candidate; break }
        } catch {}
    }
    if ($null -eq $process) { Start-Sleep -Milliseconds 250 }
}
if ($null -eq $process) { throw 'A25/A26 action driver did not observe one authenticated process/window/collector tuple.' }
$window = $process.MainWindowHandle
$owner = [uint32]0; [void][A25A26AuthenticatedInput]::GetWindowThreadProcessId($window,[ref]$owner)
if ($owner -ne $process.Id) { throw 'A25/A26 window owner mismatch.' }
[void][A25A26AuthenticatedInput]::ShowWindowAsync($window,9)
if (-not [A25A26AuthenticatedInput]::SetForegroundWindow($window)) { throw 'SetForegroundWindow failed.' }
Start-Sleep -Milliseconds 750

function Assert-Foreground {
    $foreground = [A25A26AuthenticatedInput]::GetForegroundWindow(); $foregroundOwner = [uint32]0
    [void][A25A26AuthenticatedInput]::GetWindowThreadProcessId($foreground,[ref]$foregroundOwner)
    if ($foregroundOwner -ne $process.Id -or $foreground -ne $window) { throw 'Exact the game window lost foreground ownership.' }
}
function Get-ClientSize {
    $client = [A25A26AuthenticatedInput+RECT]::new()
    if (-not [A25A26AuthenticatedInput]::GetClientRect($window,[ref]$client)) { throw 'GetClientRect failed.' }
    @($client.Right-$client.Left,$client.Bottom-$client.Top)
}
function Require-ClientSize([int]$Width,[int]$Height,[int]$TimeoutSeconds) {
    $limit = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds); $stable = 0
    while ([DateTime]::UtcNow -lt $limit) {
        $size = Get-ClientSize
        if ($size[0] -eq $Width -and $size[1] -eq $Height) { $stable++ } else { $stable = 0 }
        if ($stable -ge 5) { return @($Width,$Height) }
        Start-Sleep -Milliseconds 100
    }
    $actual = Get-ClientSize
    throw "Client size did not stabilize at ${Width}x${Height}; observed $($actual[0])x$($actual[1])."
}
function Resize-ExactClient([int]$Width,[int]$Height,[int]$SettleMilliseconds) {
    Assert-Foreground
    for ($attempt=0; $attempt -lt 5; $attempt++) {
        $client = [A25A26AuthenticatedInput+RECT]::new(); $outer = [A25A26AuthenticatedInput+RECT]::new()
        if (-not [A25A26AuthenticatedInput]::GetClientRect($window,[ref]$client) -or -not [A25A26AuthenticatedInput]::GetWindowRect($window,[ref]$outer)) { throw 'Cannot read window geometry.' }
        $borderWidth = ($outer.Right-$outer.Left)-($client.Right-$client.Left)
        $borderHeight = ($outer.Bottom-$outer.Top)-($client.Bottom-$client.Top)
        if (-not [A25A26AuthenticatedInput]::SetWindowPos($window,[IntPtr]::Zero,0,0,$Width+$borderWidth,$Height+$borderHeight,0x16)) { throw 'SetWindowPos failed.' }
        Start-Sleep -Milliseconds 300
        $actual = Get-ClientSize
        if ($actual[0] -eq $Width -and $actual[1] -eq $Height) { Start-Sleep -Milliseconds $SettleMilliseconds; return @($Width,$Height) }
    }
    $last = Get-ClientSize
    throw "Authenticated client resize failed: wanted ${Width}x${Height}, observed $($last[0])x$($last[1])."
}
function Move-Cursor([int]$X,[int]$Y) {
    Assert-Foreground; $size = Get-ClientSize
    if ($X -lt 0 -or $Y -lt 0 -or $X -ge $size[0] -or $Y -ge $size[1]) { throw "Client point outside $($size[0])x$($size[1]): $X,$Y" }
    $point=[A25A26AuthenticatedInput+POINT]::new();$point.X=$X;$point.Y=$Y
    if(-not [A25A26AuthenticatedInput]::ClientToScreen($window,[ref]$point) -or -not [A25A26AuthenticatedInput]::SetCursorPos($point.X,$point.Y)){throw 'Authenticated cursor move failed.'}
    @($point.X,$point.Y)
}
function Wait-ForLoadedPausedSave([int]$ExpectedGameHourRaw,[int]$TimeoutSeconds,[int]$StableMilliseconds) {
    $limit=[DateTime]::UtcNow.AddSeconds($TimeoutSeconds);$lastFailure='no authenticated game singleton has been observed'
    while([DateTime]::UtcNow -lt $limit){
        try {
            $base=$process.MainModule.BaseAddress.ToInt64();$handle=$process.Handle;$state=[long](Read-U64 $handle ($base+0x3316E90L));if($state-eq 0){throw 'game-state singleton is null'}
            $clockA=Read-U64 $handle ($base+0x33172E0L);$clockB=Read-U64 $handle ($base+0x33172E8L);$clocks=@(@($clockA,$clockB)|Where-Object{$_-ne 0}|Select-Object -Unique);if($clocks.Count-ne 1){throw 'clock singleton is ambiguous'}
            $clock=[long]$clocks[0];if((Read-U64 $handle $clock)-ne [uint64]($base+0x2952510L)){throw 'clock vtable mismatch'}
            $hour=Read-I32 $handle ($state+0x468L);if((Read-Bytes $handle ($clock+0x6C1L) 1)[0]-ne 1){throw 'continued save is not paused'};if($hour-ne $ExpectedGameHourRaw){throw "loaded game hour $hour does not match $ExpectedGameHourRaw"}
            Start-Sleep -Milliseconds $StableMilliseconds
            if((Read-I32 $handle ($state+0x468L))-ne $ExpectedGameHourRaw-or(Read-Bytes $handle ($clock+0x6C1L) 1)[0]-ne 1){throw 'paused save did not remain stable'}
            return [ordered]@{game_hour_raw=$hour;pause_byte=1;stable_milliseconds=$StableMilliseconds}
        } catch {$lastFailure=$_.Exception.Message;Start-Sleep -Milliseconds 250}
    }
    throw "Exact loaded paused save was not observed: $lastFailure"
}
function Wait-ForCreateNewFile([string]$Path,[int]$TimeoutSeconds) {
    $limit=[DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while([DateTime]::UtcNow-lt$limit){if(Test-Path -LiteralPath $Path -PathType Leaf){$first=(Get-Item -LiteralPath $Path).Length;Start-Sleep -Milliseconds 250;$second=(Get-Item -LiteralPath $Path).Length;if($first-gt 0-and$first-eq$second){return}};Start-Sleep -Milliseconds 100}
    throw "Harness did not create the requested immutable receipt: $Path"
}

$profileSha = (Get-FileHash -LiteralPath $resolvedProfile -Algorithm SHA256).Hash.ToLowerInvariant()
$signalDir = Join-Path $resolvedOut 'action_signals'; [IO.Directory]::CreateDirectory($signalDir) | Out-Null
$a26CaseDir = Join-Path $resolvedOut 'a26_cases'; [IO.Directory]::CreateDirectory($a26CaseDir) | Out-Null
$a25Receipt = Join-Path $resolvedOut 'a25_owner_glyph_capture.json'
$steps = [Collections.Generic.List[object]]::new(); $ordinal = 0
foreach ($action in $profile.actions) {
    Assert-Foreground; $kind=[string]$action.kind
    $step=[ordered]@{ordinal=$ordinal;label=[string]$action.label;kind=$kind;started_utc=[DateTime]::UtcNow.ToString('o');client_size_before=(Get-ClientSize)}
    switch($kind){
        'wait_for_loaded_paused_save' {$step.loaded_save=Wait-ForLoadedPausedSave ([int]$action.game_hour_raw) ([int]$action.timeout_seconds) ([int]$action.stable_milliseconds)}
        'require_client_size' {$step.client_size=Require-ClientSize ([int]$action.width) ([int]$action.height) ([int]$action.timeout_seconds)}
        'key' {[A25A26AuthenticatedInput]::Key([uint16]$action.virtual_key);Start-Sleep -Milliseconds ([int]$action.settle_milliseconds);$step.virtual_key=[int]$action.virtual_key}
        'console_command' {[A25A26AuthenticatedInput]::Scan(0x29);Start-Sleep -Milliseconds 250;[A25A26AuthenticatedInput]::Text([string]$action.command);[A25A26AuthenticatedInput]::Key(0x0D);Start-Sleep -Milliseconds ([int]$action.settle_milliseconds);[A25A26AuthenticatedInput]::Scan(0x29);$step.command=[string]$action.command}
        'click' {$screen=Move-Cursor ([int]$action.x) ([int]$action.y);[A25A26AuthenticatedInput]::mouse_event(2,0,0,0,[UIntPtr]::Zero);[A25A26AuthenticatedInput]::mouse_event(4,0,0,0,[UIntPtr]::Zero);Start-Sleep -Milliseconds ([int]$action.settle_milliseconds);$step.client_point=@([int]$action.x,[int]$action.y);$step.screen_point=$screen}
        'hover' {$screen=Move-Cursor ([int]$action.x) ([int]$action.y);Start-Sleep -Milliseconds ([int]$action.dwell_milliseconds);$step.client_point=@([int]$action.x,[int]$action.y);$step.screen_point=$screen}
        'resize_client' {$step.client_size=Resize-ExactClient ([int]$action.width) ([int]$action.height) ([int]$action.settle_milliseconds)}
        'resize_client_roundtrip' {[void](Resize-ExactClient ([int]$action.transient_width) ([int]$action.transient_height) ([int]$action.settle_milliseconds));$step.client_size=Resize-ExactClient ([int]$action.final_width) ([int]$action.final_height) ([int]$action.settle_milliseconds)}
        'capture_case' {
            $requiredSize=Require-ClientSize ([int]$action.width) ([int]$action.height) 30
            $caseId=[string]$action.case_id;$lane=[string]$action.lane
            $rawPath=if($lane-ceq'A25'){$a25Receipt}else{Join-Path $a26CaseDir ($caseId+'.json')}
            if(Test-Path -LiteralPath $rawPath){throw "Refusing stale/raw duplicate case receipt: $rawPath"}
            $signalPath=Join-Path $signalDir ('{0:D3}_{1}.json' -f $ordinal,$caseId)
            $signal=[ordered]@{schema='hoi4.a25-a26.harness-action-signal.v1';ordinal=$ordinal;profile_sha256=$profileSha;process_id=$process.Id;process_start_utc=$process.StartTime.ToUniversalTime().ToString('o');executable_sha256=$expectedExeSha256;lane=$lane;case_id=$caseId;harness_command=[string]$action.harness_command;client_size=$requiredSize;issued_utc=[DateTime]::UtcNow.ToString('o')}
            Write-CreateNewUtf8 $signalPath (($signal|ConvertTo-Json -Depth 8)+"`n")
            Wait-ForCreateNewFile $rawPath ([int]$action.timeout_seconds)
            $step.capture=[ordered]@{lane=$lane;case_id=$caseId;harness_command=[string]$action.harness_command;signal_file=('action_signals/'+[IO.Path]::GetFileName($signalPath));signal_sha256=(Get-FileHash -LiteralPath $signalPath -Algorithm SHA256).Hash.ToLowerInvariant();raw_receipt_file=if($lane-ceq'A25'){[IO.Path]::GetFileName($rawPath)}else{'a26_cases/'+[IO.Path]::GetFileName($rawPath)};raw_receipt_sha256=(Get-FileHash -LiteralPath $rawPath -Algorithm SHA256).Hash.ToLowerInvariant()}
        }
        default {throw "Unknown A25/A26 action kind: $kind"}
    }
    $step.client_size_after=Get-ClientSize;$step.completed_utc=[DateTime]::UtcNow.ToString('o');$steps.Add($step);$ordinal++
}

$receipt=[ordered]@{
    schema='hoi4.a25-a26.authenticated-action-receipt.v1';status='PASS';semantic_authority='raw owner probes and mechanical A25/A26 regraders only';promotion_authorized=$false
    process=[ordered]@{process_id=$process.Id;process_start_utc=$process.StartTime.ToUniversalTime().ToString('o');executable_file=[IO.Path]::GetFileName($resolvedExe);executable_sha256=$expectedExeSha256;collector_file=[IO.Path]::GetFileName($resolvedCollector);collector_sha256=(Get-FileHash -LiteralPath $resolvedCollector -Algorithm SHA256).Hash.ToLowerInvariant()}
    profile=[ordered]@{file=[IO.Path]::GetFileName($resolvedProfile);sha256=$profileSha;profile_id=[string]$profile.profile_id}
    steps=$steps
}
Write-CreateNewUtf8 $resolvedReceipt (($receipt|ConvertTo-Json -Depth 20)+"`n")
Write-Output "A25/A26 authenticated actions completed: $resolvedReceipt"
