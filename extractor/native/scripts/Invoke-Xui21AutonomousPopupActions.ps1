[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [ValidateSet('SeedPendingSave', 'DriveMatrix')] [string] $Mode,
    [Parameter(Mandatory = $true)] [int] $ProcessId,
    [Parameter(Mandatory = $true)] [string] $ExpectedProcessStartUtc,
    [Parameter(Mandatory = $true)] [string] $ActionProfile,
    [Parameter(Mandatory = $true)] [string] $ReceiptPath,
    [string] $SeedSavePath = '',
    [string] $CoverageReceiptDirectory = '',
    [int] $ExpectedReadyGameHourRaw = 60830441,
    [int] $ReadyTimeoutSeconds = 240,
    [int] $MatrixTimeoutSeconds = 420,
    [int] $UiSettleMilliseconds = 900
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$profilePath = (Resolve-Path -LiteralPath $ActionProfile).Path
$profileBytes = [IO.File]::ReadAllBytes($profilePath)
$profile = [Text.Encoding]::UTF8.GetString($profileBytes) | ConvertFrom-Json
if ($profile.schema -cne 'hoi4-xui21-autonomous-action-profile-v1' -or
    $profile.authority.executable_sha256 -cne $expectedExeSha256 -or
    @($profile.authority.client_size).Count -ne 2 -or
    [int]$profile.authority.client_size[0] -ne 1920 -or [int]$profile.authority.client_size[1] -ne 1080) {
    throw 'XUI21 action profile authority/schema is not the sealed 1920x1080 contract.'
}
$cells = @($profile.cells)
if ($cells.Count -ne 20) { throw 'XUI21 action profile must contain exactly 20 cells.' }
$classes = @('country','news','state','unit_leader','operative_leader')
$paths = @('player_selection','ai_selection','timeout_default_option','timeout_no_choice')
for ($i = 0; $i -lt 20; $i++) {
    $row = $cells[$i]
    if ([int]$row.cell_index -ne $i -or [string]$row.popup_class -cne $classes[[Math]::Floor($i / 4)] -or
        [string]$row.resolution_path -cne $paths[$i % 4] -or [string]::IsNullOrWhiteSpace([string]$row.command)) {
        throw "XUI21 action profile cell $i is not the exact class/path ordinal."
    }
}

Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Xui21ActionNative {
 [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
 [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
 [DllImport("kernel32.dll", SetLastError=true)] public static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
 [DllImport("kernel32.dll", SetLastError=true)] public static extern bool ReadProcessMemory(IntPtr process, IntPtr address, byte[] buffer, UIntPtr size, out UIntPtr read);
 [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr handle);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window, out RECT rect);
 [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr window, ref POINT point);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
 [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
 [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr window, int command);
}
'@

function Write-CreateNewJson([string]$Path, [object]$Value) {
    $full = [IO.Path]::GetFullPath($Path)
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($full)) | Out-Null
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes((($Value | ConvertTo-Json -Depth 14) + "`n"))
    $stream = [IO.File]::Open($full, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $stream.Write($bytes,0,$bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
}
function Assert-Window([string]$Label) {
    $p = Get-Process -Id $ProcessId -ErrorAction Stop
    if ($p.StartTime.ToUniversalTime().ToString('o') -cne $ExpectedProcessStartUtc) { throw "Process start identity changed before $Label." }
    if ($p.MainWindowHandle -eq [IntPtr]::Zero) { throw "No the game window before $Label." }
    $owner = [uint32]0; [void][Xui21ActionNative]::GetWindowThreadProcessId($p.MainWindowHandle,[ref]$owner)
    if ($owner -ne $ProcessId) { throw "Window PID changed before $Label." }
    $rect=[Xui21ActionNative+RECT]::new(); if(-not [Xui21ActionNative]::GetClientRect($p.MainWindowHandle,[ref]$rect)){throw "GetClientRect failed before $Label."}
    if(($rect.Right-$rect.Left)-ne 1920 -or ($rect.Bottom-$rect.Top)-ne 1080){throw "Client is not exact 1920x1080 before $Label."}
    if([Xui21ActionNative]::GetForegroundWindow() -ne $p.MainWindowHandle){[void][Xui21ActionNative]::ShowWindowAsync($p.MainWindowHandle,9);[void][Xui21ActionNative]::SetForegroundWindow($p.MainWindowHandle);Start-Sleep -Milliseconds 300}
    if([Xui21ActionNative]::GetForegroundWindow() -ne $p.MainWindowHandle){throw "the game is not foreground before $Label."}
    return $p
}
function Read-Bytes([uint64]$Address,[int]$Count) {
    $b=[byte[]]::new($Count);$read=[UIntPtr]::Zero
    if(-not [Xui21ActionNative]::ReadProcessMemory($script:handle,[IntPtr]([int64]$Address),$b,[UIntPtr]::new([uint64]$Count),[ref]$read)-or$read.ToUInt64()-ne[uint64]$Count){throw ('ReadProcessMemory failed at 0x{0:x}.' -f $Address)}
    return $b
}
function Read-GameHour {
    $currentGameState=[BitConverter]::ToUInt64((Read-Bytes ($script:moduleBase + 0x3316e90) 8),0)
    if($currentGameState-eq 0){return [int]::MinValue}
    return [BitConverter]::ToInt32((Read-Bytes ($currentGameState + 0x468) 4),0)
}
function Send-Key([byte]$Key,[byte]$Scan,[string]$Label) { [void](Assert-Window $Label);[Xui21ActionNative]::keybd_event($Key,$Scan,0,[UIntPtr]::Zero);[Xui21ActionNative]::keybd_event($Key,$Scan,2,[UIntPtr]::Zero) }
function Invoke-Console([string]$Command,[string]$Label) {
    Send-Key 0xC0 0x29 "open_console_$Label"; Start-Sleep -Milliseconds 350
    $literalCommand=$Command -replace '([+^%~()\[\]{}])','{$1}'
    [System.Windows.Forms.SendKeys]::SendWait($literalCommand); Start-Sleep -Milliseconds 150
    Send-Key 0x0D 0x1C "execute_$Label"; Start-Sleep -Milliseconds $UiSettleMilliseconds
    Send-Key 0xC0 0x29 "close_console_$Label"; Start-Sleep -Milliseconds $UiSettleMilliseconds
    $script:steps.Add([ordered]@{label=$Label;command=$Command;game_hour=(Read-GameHour)})
}
function Click-Option([string]$Label) {
    $p=Assert-Window $Label;$point=[Xui21ActionNative+POINT]::new();$point.X=[int]$profile.option_button_client_point[0];$point.Y=[int]$profile.option_button_client_point[1]
    if(-not[Xui21ActionNative]::ClientToScreen($p.MainWindowHandle,[ref]$point)){throw "ClientToScreen failed for $Label."}
    if(-not[Xui21ActionNative]::SetCursorPos($point.X,$point.Y)){throw "SetCursorPos failed for $Label."}
    [Xui21ActionNative]::mouse_event(2,0,0,0,[UIntPtr]::Zero);[Xui21ActionNative]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds $UiSettleMilliseconds
    $script:steps.Add([ordered]@{label=$Label;action='authenticated_option_click';client_point=@([int]$profile.option_button_client_point[0],[int]$profile.option_button_client_point[1]);game_hour=(Read-GameHour)})
}
function Find-CompleteReceipt {
    if([string]::IsNullOrWhiteSpace($CoverageReceiptDirectory)-or-not(Test-Path -LiteralPath $CoverageReceiptDirectory -PathType Container)){return $null}
    foreach($file in @(Get-ChildItem -LiteralPath $CoverageReceiptDirectory -Filter 'xui21-*.json' -File|Sort-Object LastWriteTimeUtc -Descending)){
        try{
            $o=Get-Content -LiteralPath $file.FullName -Raw|ConvertFrom-Json;$telemetry=$o.hook_telemetry
            $bools=@('fire_hook_validated','option_hook_validated','materialisation_hook_validated','timeout_hook_validated','all_event_classes_covered','player_ai_timeout_no_choice_covered','capture_start_baseline_complete')
            $zeros=@('dropped_fire_rows','dropped_resolution_rows','correlation_failures','invalid_definition_joins','fired_ledger_shadow_mismatches')
            $zeroGap=$null-ne$telemetry -and @($bools|Where-Object{$telemetry.$_ -ne $true}).Count-eq 0 -and @($zeros|Where-Object{[int64]$telemetry.$_ -ne 0}).Count-eq 0
            if($o.schema -ceq 'hoi4-xui21-runtime-coverage-receipt-v1'-and [int64]$o.matrix_bits -eq 1048575 -and @($o.observations).Count -eq 20 -and $null-ne$o.lifecycle.repeated_event_id -and $null-ne$o.lifecycle.same_hour_multiple_firings -and $null-ne$o.lifecycle.capture_start_pending -and $null-ne$o.lifecycle.non_materialised_resolution -and $zeroGap){return $file}
        }catch{}
    }
    return $null
}

$process=Assert-Window 'initial';$exe=$process.MainModule.FileName
if((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()-cne$expectedExeSha256){throw 'Process is not exact a729.'}
$script:handle=[Xui21ActionNative]::OpenProcess(0x1010,$false,[uint32]$ProcessId);if($script:handle-eq[IntPtr]::Zero){throw 'Read-only OpenProcess failed.'}
$script:moduleBase=[uint64]$process.MainModule.BaseAddress.ToInt64()
$deadline=[DateTime]::UtcNow.AddSeconds($ReadyTimeoutSeconds);do{$a=Read-GameHour;Start-Sleep -Milliseconds 500;$b=Read-GameHour}while(($a-ne$ExpectedReadyGameHourRaw-or$b-ne$a)-and[DateTime]::UtcNow-lt$deadline)
if($a-ne$ExpectedReadyGameHourRaw-or$b-ne$a){throw 'Exact D04 paused-save readiness was not reached.'}
$script:steps=[Collections.Generic.List[object]]::new();$complete=$null
try {
    if($Mode -ceq 'SeedPendingSave'){
        if([string]::IsNullOrWhiteSpace($SeedSavePath)){throw 'SeedSavePath is required in SeedPendingSave mode.'}
        $seedFull=[IO.Path]::GetFullPath($SeedSavePath);if(Test-Path -LiteralPath $seedFull){throw "Refusing to overwrite seeded save: $seedFull"}
        Invoke-Console ([string]$profile.seed.command) 'seed_capture_start_pending'
        $saveName=[IO.Path]::GetFileNameWithoutExtension($seedFull)
        Invoke-Console "save_game $saveName" 'save_seeded_fixture'
        $saveDeadline=[DateTime]::UtcNow.AddSeconds(60);while(-not(Test-Path -LiteralPath $seedFull -PathType Leaf)-and[DateTime]::UtcNow-lt$saveDeadline){Start-Sleep -Milliseconds 500}
        if(-not(Test-Path -LiteralPath $seedFull -PathType Leaf)){throw 'Seeded save was not created by the engine.'}
        $result=[ordered]@{schema='hoi4-xui21-autonomous-action-receipt-v1';status='pass_seeded_pending_save';mode=$Mode;process_id=$ProcessId;process_start_utc=$ExpectedProcessStartUtc;executable_sha256=$expectedExeSha256;profile_sha256=(Get-FileHash -LiteralPath $profilePath -Algorithm SHA256).Hash.ToLowerInvariant();source_game_hour=$ExpectedReadyGameHourRaw;seed_event_id=[string]$profile.seed.event_id;seeded_save_file=[IO.Path]::GetFileName($seedFull);seeded_save_sha256=(Get-FileHash -LiteralPath $seedFull -Algorithm SHA256).Hash.ToLowerInvariant();steps=$script:steps;promotion_authorized=$false}
    } else {
        # Resolve the restored pending baseline through the real player option callback.
        Start-Sleep -Milliseconds $UiSettleMilliseconds; Click-Option 'resolve_capture_start_pending'
        $setupOrdinal=0
        foreach($command in @($profile.setup_commands)){Invoke-Console ([string]$command) ("setup_{0}" -f $setupOrdinal);$setupOrdinal++}
        foreach($row in $cells|Where-Object resolution_path -in @('player_selection','ai_selection')){
            Invoke-Console ([string]$row.command) ("fire_cell_{0}" -f [int]$row.cell_index)
            if($row.resolution_path -ceq 'player_selection'){Click-Option ("select_cell_{0}" -f [int]$row.cell_index)}
        }
        foreach($row in $cells|Where-Object resolution_path -in @('timeout_default_option','timeout_no_choice')){Invoke-Console ([string]$row.command) ("queue_cell_{0}" -f [int]$row.cell_index)}
        $startHour=Read-GameHour;Send-Key 0x20 0x39 'unpause_for_due_timeouts'
        $deadline=[DateTime]::UtcNow.AddSeconds($MatrixTimeoutSeconds)
        do{$complete=Find-CompleteReceipt;if($null-ne$complete){break};Start-Sleep -Milliseconds 750;if((Read-GameHour)-eq$startHour){Send-Key 0x20 0x39 'resume_after_popup_pause'}}while([DateTime]::UtcNow-lt$deadline)
        # End paused even on failure. The independent receipt loader is authoritative.
        $h1=Read-GameHour;Start-Sleep -Milliseconds 400;$h2=Read-GameHour;if($h2-ne$h1){Send-Key 0x20 0x39 'final_repause'}
        if($null-eq$complete){throw 'No raw complete 20-cell/four-edge XUI21 receipt appeared before timeout.'}
        $result=[ordered]@{schema='hoi4-xui21-autonomous-action-receipt-v1';status='pass_machine_drove_complete_runtime_receipt';mode=$Mode;process_id=$ProcessId;process_start_utc=$ExpectedProcessStartUtc;executable_sha256=$expectedExeSha256;profile_sha256=(Get-FileHash -LiteralPath $profilePath -Algorithm SHA256).Hash.ToLowerInvariant();source_game_hour=$ExpectedReadyGameHourRaw;declared_cell_count=20;declared_lifecycle_edge_count=4;raw_coverage_receipt_file=$complete.Name;raw_coverage_receipt_sha256=(Get-FileHash -LiteralPath $complete.FullName -Algorithm SHA256).Hash.ToLowerInvariant();steps=$script:steps;promotion_authorized=$false}
    }
    Write-CreateNewJson $ReceiptPath $result
    $result|ConvertTo-Json -Depth 14
} finally { if($script:handle-ne[IntPtr]::Zero){[void][Xui21ActionNative]::CloseHandle($script:handle)} }
