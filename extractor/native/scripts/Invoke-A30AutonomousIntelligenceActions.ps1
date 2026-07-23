[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [uint32] $ProcessId,
    [Parameter(Mandatory = $true)] [string] $ExpectedProcessStartUtc,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $ProfilePath,
    [Parameter(Mandatory = $true)] [string] $SelectedPresentDirectory,
    [Parameter(Mandatory = $true)] [string] $ReceiptPath,
    [int] $InitialSettleSeconds = 75,
    [int] $SelectedPresentTimeoutSeconds = 90
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$gameStateGlobalRva = [uint64]0x03316E90
$gameHourOffset = [uint64]0x468
$primaryCountryOffset = [uint64]0x520
$fallbackCountryOffset = [uint64]0x524
$clockGlobalRvas = @([uint64]0x033172E0, [uint64]0x033172E8)
$clockVtableRva = [uint64]0x02952510
$clockPauseOffset = [uint64]0x6C1

function Write-CreateNewUtf8([string]$Path, [string]$Text) {
    $full = [IO.Path]::GetFullPath($Path)
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($full)) | Out-Null
    $temporary = [IO.Path]::Combine([IO.Path]::GetDirectoryName($full), ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText($temporary, $Text, [Text.UTF8Encoding]::new($false))
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($full) -or [IO.Directory]::Exists($full)) { throw "Refusing to replace A30 action receipt: $full" }
        [IO.File]::Move($temporary, $full)
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class A30AuthenticatedInput {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public INPUTUNION U; }
    [StructLayout(LayoutKind.Explicit)] public struct INPUTUNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
    [DllImport("kernel32.dll", SetLastError=true)] public static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool ReadProcessMemory(IntPtr p, IntPtr a, byte[] b, UIntPtr n, out UIntPtr read);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr p);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr h, int command);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT rect);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT point);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll", SetLastError=true)] public static extern uint SendInput(uint count, INPUT[] input, int size);
    public static void Key(ushort vk, bool down) {
        INPUT[] i=new INPUT[1]; i[0].type=1; i[0].U.ki.vk=vk; i[0].U.ki.flags=down?0u:2u;
        if(SendInput(1,i,Marshal.SizeOf(typeof(INPUT)))!=1) throw new Exception("SendInput key failed");
    }
    public static void Click() {
        INPUT[] i=new INPUT[2]; i[0].type=0; i[0].U.mi.dwFlags=2; i[1].type=0; i[1].U.mi.dwFlags=4;
        if(SendInput(2,i,Marshal.SizeOf(typeof(INPUT)))!=2) throw new Exception("SendInput click failed");
    }
}
'@

$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedProfile = (Resolve-Path -LiteralPath $ProfilePath).Path
$selectedRoot = [IO.Path]::GetFullPath($SelectedPresentDirectory)
$resolvedReceipt = [IO.Path]::GetFullPath($ReceiptPath)
if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) { throw 'A30 action executable is not exact a729.' }
$profile = Get-Content -LiteralPath $resolvedProfile -Raw | ConvertFrom-Json
if ($profile.schema -cne 'hoi4.a30.autonomous-action-profile.v1' -or -not ($profile.actions -is [array])) { throw 'A30 action profile is invalid.' }
if ($profile.authority.executable_sha256 -cne $expectedExeSha256) { throw 'A30 action profile executable authority differs.' }

$process = Get-Process -Id $ProcessId -ErrorAction Stop
if ([DateTime]::Parse($process.StartTime.ToUniversalTime().ToString('o')).ToUniversalTime() -ne [DateTime]::Parse($ExpectedProcessStartUtc).ToUniversalTime()) { throw 'A30 PID was reused or process start identity changed.' }
if (-not [string]::Equals($process.MainModule.FileName, $resolvedExe, [StringComparison]::OrdinalIgnoreCase)) { throw 'A30 process executable path differs.' }
$loadedCollector = @($process.Modules | Where-Object { [string]::Equals($_.FileName, $resolvedCollector, [StringComparison]::OrdinalIgnoreCase) })
if ($loadedCollector.Count -ne 1) { throw 'A30 action owner requires exactly one authenticated collector module.' }
$window = $process.MainWindowHandle
if ($window -eq [IntPtr]::Zero) { throw 'A30 exact process has no main window.' }
$handle = [A30AuthenticatedInput]::OpenProcess(0x1010, $false, $ProcessId)
if ($handle -eq [IntPtr]::Zero) { throw 'A30 read-only OpenProcess failed.' }
$moduleBase = [uint64]$process.MainModule.BaseAddress.ToInt64()

function Read-Bytes([uint64]$Address, [int]$Count) {
    $bytes=[byte[]]::new($Count); $read=[UIntPtr]::Zero
    if (-not [A30AuthenticatedInput]::ReadProcessMemory($handle,[IntPtr]([int64]$Address),$bytes,[UIntPtr]::new([uint64]$Count),[ref]$read) -or $read.ToUInt64() -ne [uint64]$Count) { throw ('A30 ReadProcessMemory failed at 0x{0:X}.' -f $Address) }
    $bytes
}
function Read-U64([uint64]$Address) { [BitConverter]::ToUInt64((Read-Bytes $Address 8),0) }
function Read-I32([uint64]$Address) { [BitConverter]::ToInt32((Read-Bytes $Address 4),0) }
function Assert-Window([string]$Label) {
    $owner=[uint32]0; [void][A30AuthenticatedInput]::GetWindowThreadProcessId($window,[ref]$owner)
    if ($owner -ne $ProcessId) { throw "A30 window owner changed before $Label." }
    [void][A30AuthenticatedInput]::ShowWindowAsync($window,9); [void][A30AuthenticatedInput]::SetForegroundWindow($window); Start-Sleep -Milliseconds 350
    if ([A30AuthenticatedInput]::GetForegroundWindow() -ne $window) { throw "A30 window is not foreground before $Label." }
    $rect=[A30AuthenticatedInput+RECT]::new(); if(-not [A30AuthenticatedInput]::GetClientRect($window,[ref]$rect)){throw 'A30 GetClientRect failed.'}
    if (($rect.Right-$rect.Left) -ne [int]$profile.expected_client_size[0] -or ($rect.Bottom-$rect.Top) -ne [int]$profile.expected_client_size[1]) { throw 'A30 client dimensions differ from the frozen profile.' }
}
function Read-FrozenIdentity {
    $state=Read-U64 ($moduleBase+$gameStateGlobalRva); if($state -eq 0){throw 'A30 game-state singleton is null.'}
    $clocks=@($clockGlobalRvas | ForEach-Object { Read-U64 ($moduleBase+$_) } | Where-Object { $_ -ne 0 } | Select-Object -Unique)
    if($clocks.Count-ne 1){throw 'A30 clock singleton is ambiguous.'};$clock=[uint64]$clocks[0]
    if((Read-U64 $clock)-ne ($moduleBase+$clockVtableRva)){throw 'A30 clock vtable mismatch.'}
    $hour=Read-I32 ($state+$gameHourOffset);$primary=Read-I32 ($state+$primaryCountryOffset);$fallback=Read-I32 ($state+$fallbackCountryOffset)
    if((Read-Bytes ($clock+$clockPauseOffset) 1)[0]-ne 1){throw 'A30 acquisition requires the paused D04 save.'}
    if($hour-ne[int]$profile.expected_game_hour_raw){throw "A30 game hour $hour differs from the frozen profile."}
    $owner=if($primary-gt 0){$primary}else{$fallback};if($owner-ne 368){throw "A30 selected owner $owner is not D04 handle 368."}
    [ordered]@{game_hour_raw=$hour;owner_tag_db_id=$owner;pause_byte=1;module_base=('0x{0:X}' -f $moduleBase)}
}
function Selected-Files { @((Get-ChildItem -LiteralPath $selectedRoot -File -Filter '*.json' -ErrorAction SilentlyContinue | Sort-Object FullName)) }
function Wait-NewSelectedPresent([string[]]$BeforeNames,[string]$Label) {
    $deadline=[DateTime]::UtcNow.AddSeconds($SelectedPresentTimeoutSeconds)
    while([DateTime]::UtcNow-lt$deadline){$new=@(Selected-Files|Where-Object{$BeforeNames-cnotcontains$_.Name});if($new.Count-gt1){throw "A30 collector published multiple selected-Present receipts for one action: $Label"};if($new.Count-eq1){$file=$new[0];return [ordered]@{label=$Label;file_name=$file.Name;bytes=$file.Length;sha256=(Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}};Start-Sleep -Milliseconds 250}
    throw "A30 collector did not publish a new immutable selected-Present receipt for $Label."
}
function Send-Key([ushort]$Key){[A30AuthenticatedInput]::Key($Key,$true);[A30AuthenticatedInput]::Key($Key,$false)}
function Send-Chord([object[]]$Keys){foreach($key in $Keys){[A30AuthenticatedInput]::Key([ushort]$key,$true)};for($i=$Keys.Count-1;$i-ge 0;$i--){[A30AuthenticatedInput]::Key([ushort]$Keys[$i],$false)}}
function Click-Client([int]$X,[int]$Y){$point=[A30AuthenticatedInput+POINT]::new();$point.X=$X;$point.Y=$Y;if(-not[A30AuthenticatedInput]::ClientToScreen($window,[ref]$point)){throw 'A30 ClientToScreen failed.'};if(-not[A30AuthenticatedInput]::SetCursorPos($point.X,$point.Y)){throw 'A30 SetCursorPos failed.'};[A30AuthenticatedInput]::Click();@($point.X,$point.Y)}

try {
    Assert-Window 'initial readiness'
    if($InitialSettleSeconds-gt 0){Start-Sleep -Seconds $InitialSettleSeconds}
    $initial=Read-FrozenIdentity
    [IO.Directory]::CreateDirectory($selectedRoot) | Out-Null
    $steps=[Collections.Generic.List[object]]::new()
    foreach($action in $profile.actions){
        Assert-Window ([string]$action.label);$beforeNames=@(Selected-Files|ForEach-Object{$_.Name});$qpcBefore=[Diagnostics.Stopwatch]::GetTimestamp();$step=[ordered]@{label=[string]$action.label;kind=[string]$action.kind;qpc_before=$qpcBefore;started_utc=[DateTime]::UtcNow.ToString('o')}
        switch([string]$action.kind){
            'key' { Send-Key ([ushort]$action.keys[0]) }
            'chord' { Send-Chord @($action.keys) }
            'click' { $screen=Click-Client ([int]$action.client_point[0]) ([int]$action.client_point[1]);$step.client_point=@([int]$action.client_point[0],[int]$action.client_point[1]);$step.screen_point=$screen }
            default { throw "Unknown A30 action kind: $($action.kind)" }
        }
        Start-Sleep -Milliseconds ([int]$action.settle_milliseconds)
        $step.qpc_after=[Diagnostics.Stopwatch]::GetTimestamp();$step.frozen_identity=Read-FrozenIdentity
        if($action.requires_new_selected_present-eq$true){$step.selected_present=Wait-NewSelectedPresent $beforeNames ([string]$action.label)}
        $step.completed_utc=[DateTime]::UtcNow.ToString('o');$steps.Add($step)
    }
    $installRoot=Split-Path -Parent $resolvedExe
    $guiFiles=[ordered]@{topbar='interface\topbar.gui';agency='interface\countryintelligenceagencyview.gui';diplomacy='interface\countrydiplomacyview.gui'}
    $guiHashes=[ordered]@{}
    foreach($name in $guiFiles.Keys){$path=Join-Path $installRoot $guiFiles[$name];if(-not(Test-Path -LiteralPath $path -PathType Leaf)){throw "A30 authored GUI authority missing: $path"};$guiHashes[$name]=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()}
    if ($guiHashes.topbar -cne [string]$profile.authority.topbar_gui_sha256 -or
        $guiHashes.agency -cne [string]$profile.authority.agency_gui_sha256 -or
        $guiHashes.diplomacy -cne [string]$profile.authority.diplomacy_gui_sha256) {
        throw 'A30 authored GUI authority hash drifted.'
    }
    $receipt=[ordered]@{schema='hoi4.a30.autonomous-action-receipt.v1';status='actions_complete_selected_present_receipts_published';process_id=$ProcessId;process_start_utc=$process.StartTime.ToUniversalTime().ToString('o');executable_sha256=$expectedExeSha256;collector_sha256=(Get-FileHash -LiteralPath $resolvedCollector -Algorithm SHA256).Hash.ToLowerInvariant();profile_sha256=(Get-FileHash -LiteralPath $resolvedProfile -Algorithm SHA256).Hash.ToLowerInvariant();client_size=@([int]$profile.expected_client_size[0],[int]$profile.expected_client_size[1]);initial_identity=$initial;gui_authority_sha256=$guiHashes;steps=$steps;final_identity=Read-FrozenIdentity;mutation_scope='physical UI input in a fresh disposable process; save bytes and game memory are never written';promotion_authorized=$false}
    Write-CreateNewUtf8 $resolvedReceipt (($receipt|ConvertTo-Json -Depth 20)+"`n")
    $receipt|ConvertTo-Json -Depth 20
} finally {
    if($handle-ne[IntPtr]::Zero){[void][A30AuthenticatedInput]::CloseHandle($handle)}
}
