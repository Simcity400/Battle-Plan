[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [switch] $AllowLegacyClientDriving,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $ProfilePath,
    [Parameter(Mandatory = $true)] [string] $ReceiptPath,
    [uint32] $ExistingPid = 0,
    [long] $NotBeforeFileTimeUtc = 0,
    [int] $ProcessTimeoutSeconds = 300
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $AllowLegacyClientDriving) {
    throw 'Legacy A36 client driving is disabled. Use Invoke-A36ActiveAbilityEvidence.ps1 for passive observation.'
}
$acknowledgement = 'This legacy tool sends input, changes focus, writes the game clock, and is never called by a passive observer.'
Write-Warning $acknowledgement
$expectedSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedProfile = (Resolve-Path -LiteralPath $ProfilePath).Path
if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedSha256) {
    throw 'A36 action driver executable is not exact a729.'
}
$profile = Get-Content -LiteralPath $resolvedProfile -Raw | ConvertFrom-Json
if ($profile.schema -cne 'hoi4.a36.authenticated-matrix-actions.v1' -or -not ($profile.actions -is [array])) {
    throw 'A36 action profile schema/actions are invalid.'
}

function Write-CreateNewUtf8([string]$Path, [string]$Text) {
    $full = [IO.Path]::GetFullPath($Path)
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($full)) | Out-Null
    $temporary = [IO.Path]::Combine([IO.Path]::GetDirectoryName($full), ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText($temporary, $Text, [Text.UTF8Encoding]::new($false))
        if ([IO.File]::Exists($full)) { throw "Refusing to replace A36 action receipt: $full" }
        [IO.File]::Move($temporary, $full)
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class A36AuthenticatedInput {
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
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT point);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint x, uint y, uint data, UIntPtr extra);
    [DllImport("user32.dll", SetLastError=true)] public static extern uint SendInput(uint count, INPUT[] input, int size);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool ReadProcessMemory(IntPtr p, IntPtr a, byte[] b, UIntPtr n, out UIntPtr read);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool WriteProcessMemory(IntPtr p, IntPtr a, byte[] b, UIntPtr n, out UIntPtr written);
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
    $bytes=[byte[]]::new($Count); $read=[UIntPtr]::Zero
    if (-not [A36AuthenticatedInput]::ReadProcessMemory($Handle,[IntPtr]$Address,$bytes,[UIntPtr]::new([uint64]$Count),[ref]$read) -or $read.ToUInt64() -ne $Count) { throw ('ReadProcessMemory failed at 0x{0:X}' -f $Address) }
    $bytes
}
function Read-U64([IntPtr]$Handle,[long]$Address) { [BitConverter]::ToUInt64((Read-Bytes $Handle $Address 8),0) }
function Read-I32([IntPtr]$Handle,[long]$Address) { [BitConverter]::ToInt32((Read-Bytes $Handle $Address 4),0) }
function Write-U8([IntPtr]$Handle,[long]$Address,[byte]$Value) {
    $written=[UIntPtr]::Zero; $bytes=[byte[]]@($Value)
    if (-not [A36AuthenticatedInput]::WriteProcessMemory($Handle,[IntPtr]$Address,$bytes,[UIntPtr]::new(1),[ref]$written) -or $written.ToUInt64() -ne 1) { throw 'WriteProcessMemory pause write failed.' }
}

function Wait-ForLoadedPausedSave([int]$ExpectedGameHourRaw,[int]$TimeoutSeconds,[int]$StableMilliseconds) {
    if($ExpectedGameHourRaw -le 0){throw 'wait_for_loaded_paused_save requires a positive game_hour_raw.'}
    if($TimeoutSeconds -lt 1 -or $StableMilliseconds -lt 250){throw 'wait_for_loaded_paused_save timing is invalid.'}
    $deadline=[DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $lastFailure='no authenticated game singleton has been observed'
    while([DateTime]::UtcNow -lt $deadline){
        try {
            $base=$process.MainModule.BaseAddress.ToInt64();$handle=$process.Handle
            $state=[long](Read-U64 $handle ($base+0x3316E90L))
            if($state -eq 0){throw 'game-state singleton is null'}
            $clockA=Read-U64 $handle ($base+0x33172E0L);$clockB=Read-U64 $handle ($base+0x33172E8L)
            $clocks=@(@($clockA,$clockB)|Where-Object{$_ -ne 0}|Select-Object -Unique)
            if($clocks.Count-ne 1){throw 'clock singleton is ambiguous'}
            $clock=[long]$clocks[0]
            if((Read-U64 $handle $clock)-ne [uint64]($base+0x2952510L)){throw 'clock vtable mismatch'}
            $pause=$clock+0x6C1L
            $hour=Read-I32 $handle ($state+0x468L)
            if((Read-Bytes $handle $pause 1)[0]-ne 1){throw 'continued save is not paused'}
            if($hour-ne $ExpectedGameHourRaw){throw "loaded game hour $hour does not match $ExpectedGameHourRaw"}
            Start-Sleep -Milliseconds $StableMilliseconds
            if([long](Read-U64 $handle ($base+0x3316E90L))-ne $state){throw 'game-state singleton changed during stability read'}
            $clockA2=Read-U64 $handle ($base+0x33172E0L);$clockB2=Read-U64 $handle ($base+0x33172E8L)
            $clocks2=@(@($clockA2,$clockB2)|Where-Object{$_ -ne 0}|Select-Object -Unique)
            if($clocks2.Count-ne 1-or[long]$clocks2[0]-ne $clock){throw 'clock singleton changed during stability read'}
            if((Read-Bytes $handle $pause 1)[0]-ne 1-or(Read-I32 $handle ($state+0x468L))-ne $ExpectedGameHourRaw){throw 'paused save did not remain stable'}
            return [ordered]@{game_hour_raw=$ExpectedGameHourRaw;pause_byte=1;stable_milliseconds=$StableMilliseconds;singletons_stable=$true}
        } catch {
            $lastFailure=$_.Exception.Message
            Start-Sleep -Milliseconds 250
        }
    }
    throw "A36 action driver did not observe the exact loaded paused save: $lastFailure"
}

$deadline=[DateTime]::UtcNow.AddSeconds($ProcessTimeoutSeconds); $process=$null
while ([DateTime]::UtcNow -lt $deadline -and $null -eq $process) {
    $candidates = if ($ExistingPid) { @(Get-Process -Id $ExistingPid -ErrorAction SilentlyContinue) } else { @(Get-Process -Name the game -ErrorAction SilentlyContinue) }
    foreach ($candidate in $candidates) {
        try {
            if ($candidate.StartTime.ToFileTimeUtc() -lt $NotBeforeFileTimeUtc -or $candidate.MainWindowHandle -eq [IntPtr]::Zero) { continue }
            if (-not [string]::Equals($candidate.MainModule.FileName,$resolvedExe,[StringComparison]::OrdinalIgnoreCase)) { continue }
            if ((Get-FileHash -LiteralPath $candidate.MainModule.FileName -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedSha256) { continue }
            $loaded = @($candidate.Modules | Where-Object { [string]::Equals($_.FileName,$resolvedCollector,[StringComparison]::OrdinalIgnoreCase) })
            if ($loaded.Count -eq 1) { $process=$candidate; break }
        } catch {}
    }
    if ($null -eq $process) { Start-Sleep -Milliseconds 250 }
}
if ($null -eq $process) { throw 'A36 action driver did not observe one authenticated process/window/collector tuple.' }
$window=$process.MainWindowHandle; $owner=[uint32]0; [void][A36AuthenticatedInput]::GetWindowThreadProcessId($window,[ref]$owner)
if ($owner -ne $process.Id) { throw 'A36 window owner mismatch.' }
$client=[A36AuthenticatedInput+RECT]::new(); if(-not [A36AuthenticatedInput]::GetClientRect($window,[ref]$client)){throw 'GetClientRect failed.'}
$width=$client.Right-$client.Left; $height=$client.Bottom-$client.Top
if ($profile.expected_client_width -ne $width -or $profile.expected_client_height -ne $height) { throw "A36 profile requires $($profile.expected_client_width)x$($profile.expected_client_height), observed ${width}x${height}." }
[void][A36AuthenticatedInput]::ShowWindowAsync($window,9); if(-not [A36AuthenticatedInput]::SetForegroundWindow($window)){throw 'SetForegroundWindow failed.'}; Start-Sleep -Milliseconds 750

function Assert-Foreground { if([A36AuthenticatedInput]::GetForegroundWindow() -ne $window){throw 'A36 exact window lost foreground ownership.'} }
function Move-Cursor([int]$X,[int]$Y) {
    Assert-Foreground; if($X -lt 0 -or $Y -lt 0 -or $X -ge $width -or $Y -ge $height){throw "A36 client point outside profile: $X,$Y"}
    $point=[A36AuthenticatedInput+POINT]::new();$point.X=$X;$point.Y=$Y;if(-not [A36AuthenticatedInput]::ClientToScreen($window,[ref]$point)){throw 'ClientToScreen failed.'};if(-not [A36AuthenticatedInput]::SetCursorPos($point.X,$point.Y)){throw 'SetCursorPos failed.'}
    $point
}
function Advance-Hours([int]$Hours,[int]$TimeoutSeconds) {
    if($Hours -lt 1){throw 'advance_hours requires a positive count.'}
    $base=$process.MainModule.BaseAddress.ToInt64();$handle=$process.Handle;$state=[long](Read-U64 $handle ($base+0x3316E90L));$clockA=Read-U64 $handle ($base+0x33172E0L);$clockB=Read-U64 $handle ($base+0x33172E8L);$clocks=@($clockA,$clockB|Where-Object{$_ -ne 0}|Select-Object -Unique);if($clocks.Count-ne 1){throw 'Clock singleton is ambiguous.'};$clock=[long]$clocks[0];if((Read-U64 $handle $clock)-ne [uint64]($base+0x2952510L)){throw 'Clock vtable mismatch.'}
    $pause=$clock+0x6C1L;$before=Read-I32 $handle ($state+0x468L);$target=$before+$Hours;if((Read-Bytes $handle $pause 1)[0]-ne 1){throw 'A36 action expected a paused game.'}
    [A36AuthenticatedInput]::Key(0x74); Start-Sleep -Milliseconds 250
    $watch=[Diagnostics.Stopwatch]::StartNew();try{Write-U8 $handle $pause 0;while((Read-I32 $handle ($state+0x468L)) -lt $target){if($watch.Elapsed.TotalSeconds -ge $TimeoutSeconds){throw "A36 clock did not reach $target"};Start-Sleep -Milliseconds 2}}finally{Write-U8 $handle $pause 1}
    [ordered]@{game_hour_before=$before;game_hour_after=(Read-I32 $handle ($state+0x468L));requested_hours=$Hours;elapsed_milliseconds=$watch.ElapsedMilliseconds}
}

$steps=[Collections.Generic.List[object]]::new()
foreach($action in $profile.actions){
    Assert-Foreground; $kind=[string]$action.kind; $step=[ordered]@{label=[string]$action.label;kind=$kind;started_utc=[DateTime]::UtcNow.ToString('o')}
    switch($kind){
        'wait_for_loaded_paused_save' { $step.loaded_save=Wait-ForLoadedPausedSave ([int]$action.game_hour_raw) ([int]$action.timeout_seconds) ([int]$action.stable_milliseconds) }
        'console_command' { [A36AuthenticatedInput]::Scan(0x29);Start-Sleep -Milliseconds 250;[A36AuthenticatedInput]::Text([string]$action.command);[A36AuthenticatedInput]::Key(0x0D);Start-Sleep -Milliseconds ([int]$action.settle_milliseconds);[A36AuthenticatedInput]::Scan(0x29);$step.command=[string]$action.command }
        'click' { $point=Move-Cursor ([int]$action.x) ([int]$action.y);[A36AuthenticatedInput]::mouse_event(2,0,0,0,[UIntPtr]::Zero);[A36AuthenticatedInput]::mouse_event(4,0,0,0,[UIntPtr]::Zero);Start-Sleep -Milliseconds ([int]$action.settle_milliseconds);$step.client_point=@([int]$action.x,[int]$action.y);$step.screen_point=@($point.X,$point.Y) }
        'hover' { $point=Move-Cursor ([int]$action.x) ([int]$action.y);Start-Sleep -Milliseconds ([int]$action.dwell_milliseconds);$step.client_point=@([int]$action.x,[int]$action.y);$step.screen_point=@($point.X,$point.Y) }
        'hover_ability_rows' {
            $count=[int]$action.count;$firstX=[int]$action.first_x;$strideX=[int]$action.stride_x;$rowY=[int]$action.y;$dwell=[int]$action.dwell_milliseconds
            if($count-lt 1-or$count-gt 32-or$strideX-lt 1-or$dwell-lt 100){throw 'hover_ability_rows bounds are invalid.'}
            $points=[Collections.Generic.List[object]]::new()
            for($ordinal=0;$ordinal-lt$count;$ordinal++){
                $x=$firstX+($ordinal*$strideX);$point=Move-Cursor $x $rowY;Start-Sleep -Milliseconds $dwell
                $points.Add([ordered]@{ordinal=$ordinal;client_point=@($x,$rowY);screen_point=@($point.X,$point.Y)})
            }
            $step.rows=$points
        }
        'advance_hours' { $step.clock=Advance-Hours ([int]$action.hours) ([int]$action.timeout_seconds) }
        'wait' { Start-Sleep -Milliseconds ([int]$action.milliseconds) }
        default { throw "Unknown A36 action kind: $kind" }
    }
    $step.completed_utc=[DateTime]::UtcNow.ToString('o');$steps.Add($step)
}
$profileSha=(Get-FileHash -LiteralPath $resolvedProfile -Algorithm SHA256).Hash.ToLowerInvariant()
$receipt=[ordered]@{schema='hoi4.a36.authenticated-matrix-action-receipt.v1';semantic_authority='A36 hook receipt and mechanical regrader only';process_id=$process.Id;process_start_utc=$process.StartTime.ToUniversalTime().ToString('o');executable_path=$resolvedExe;executable_sha256=$expectedSha256;collector_path=$resolvedCollector;collector_sha256=(Get-FileHash -LiteralPath $resolvedCollector -Algorithm SHA256).Hash.ToLowerInvariant();profile_path=$resolvedProfile;profile_sha256=$profileSha;client_size=@($width,$height);steps=$steps;promotion_authorized=$false}
Write-CreateNewUtf8 $ReceiptPath (($receipt|ConvertTo-Json -Depth 20)+"`n")
