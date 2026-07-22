[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [ValidateSet('A18DynamicModifier', 'A19DelayedEvent')] [string] $Mode,
    [Parameter(Mandatory = $true)] [int] $ProcessId,
    [Parameter(Mandatory = $true)] [string] $ExpectedProcessStartUtc,
    [Parameter(Mandatory = $true)] [string] $ReceiptPath,
    [Parameter(Mandatory = $true)] [ValidatePattern('^[A-Z0-9]{3}$')] [string] $OriginatorTag,
    [Parameter(Mandatory = $true)] [ValidatePattern('^[A-Z0-9]{3}$')] [string] $RecipientTag,
    [string] $TimedModifier,
    [string] $PermanentModifier,
    [string] $CheckpointSavePath,
    [string] $CheckpointDirectory,
    [string] $ActiveEventDefinitionPath,
    [string] $CancelledEventDefinitionSource,
    [string] $ExecutedEventId = 'battle_plan_a19.1',
    [string] $FailedTriggerEventId = 'battle_plan_a19.2',
    [string] $CancelledEventId = 'battle_plan_a19.3',
    [int] $InitialSettleSeconds = 30,
    [int] $ActionTimeoutSeconds = 600,
    [int] $UiSettleMilliseconds = 700
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$gameStateGlobalRva = [uint64]0x03316e90
$gameHourOffset = [uint64]0x468

function Write-AtomicNewJson([string]$Path, [object]$Value) {
    $full = [IO.Path]::GetFullPath($Path)
    if ([IO.File]::Exists($full) -or [IO.Directory]::Exists($full)) { throw "ReceiptPath must be new: $full" }
    $parent = [IO.Path]::GetDirectoryName($full)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $tmp = [IO.Path]::Combine($parent, [IO.Path]::GetRandomFileName())
    try {
        [IO.File]::WriteAllText($tmp, (($Value | ConvertTo-Json -Depth 16) + "`n"), [Text.UTF8Encoding]::new($false))
        $stream = [IO.File]::Open($tmp, 'Open', 'ReadWrite', 'None')
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        [IO.File]::Move($tmp, $full)
    } finally { if ([IO.File]::Exists($tmp)) { [IO.File]::Delete($tmp) } }
}

Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Hoi4A18A19Native {
  [DllImport("kernel32.dll", SetLastError=true)] public static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
  [DllImport("kernel32.dll", SetLastError=true)] public static extern bool ReadProcessMemory(IntPtr process, IntPtr address, byte[] buffer, UIntPtr size, out UIntPtr read);
  [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr handle);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
  [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr window, int command);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
}
'@

$process = Get-Process -Id $ProcessId -ErrorAction Stop
$processStartUtc = $process.StartTime.ToUniversalTime().ToString('o')
if ([DateTime]::Parse($processStartUtc).ToUniversalTime() -ne [DateTime]::Parse($ExpectedProcessStartUtc).ToUniversalTime()) { throw 'Process start identity changed.' }
$exeSha256 = (Get-FileHash -LiteralPath $process.MainModule.FileName -Algorithm SHA256).Hash.ToLowerInvariant()
if ($exeSha256 -cne $expectedExeSha256) { throw "Executable SHA-256 mismatch: $exeSha256" }
$window = $process.MainWindowHandle
if ($window -eq [IntPtr]::Zero) { throw 'Exact-a729 process has no main window.' }
$handle = [Hoi4A18A19Native]::OpenProcess(0x1010, $false, [uint32]$ProcessId)
if ($handle -eq [IntPtr]::Zero) { throw 'Read-only process handle failed.' }
$steps = [Collections.Generic.List[object]]::new()
$consoleOpen = $false

function Assert-Window([string]$Label) {
    $current = Get-Process -Id $ProcessId -ErrorAction Stop
    if ($current.MainWindowHandle -ne $window) { throw "Window changed before $Label." }
    $owner = [uint32]0
    [void][Hoi4A18A19Native]::GetWindowThreadProcessId($window, [ref]$owner)
    if ($owner -ne $ProcessId) { throw "Window owner changed before $Label." }
    if ([Hoi4A18A19Native]::GetForegroundWindow() -ne $window) {
        [void][Hoi4A18A19Native]::ShowWindowAsync($window, 9)
        [void][Hoi4A18A19Native]::SetForegroundWindow($window)
        Start-Sleep -Milliseconds 300
    }
    if ([Hoi4A18A19Native]::GetForegroundWindow() -ne $window) { throw "Exact-a729 window is not foreground before $Label." }
}
function Send-Key([byte]$Key, [byte]$Scan, [string]$Label) {
    Assert-Window $Label
    [Hoi4A18A19Native]::keybd_event($Key, $Scan, 0, [UIntPtr]::Zero)
    [Hoi4A18A19Native]::keybd_event($Key, $Scan, 2, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 120
}
function Send-ControlS([string]$Label) {
    Assert-Window $Label
    [Hoi4A18A19Native]::keybd_event(0x11, 0x1D, 0, [UIntPtr]::Zero)
    [Hoi4A18A19Native]::keybd_event(0x53, 0x1F, 0, [UIntPtr]::Zero)
    [Hoi4A18A19Native]::keybd_event(0x53, 0x1F, 2, [UIntPtr]::Zero)
    [Hoi4A18A19Native]::keybd_event(0x11, 0x1D, 2, [UIntPtr]::Zero)
    $steps.Add([ordered]@{ action=$Label; method='authenticated_physical_ctrl_s' })
}
function Open-Console {
    if (-not $script:consoleOpen) { Send-Key 0xC0 0x29 'open_console'; $script:consoleOpen=$true; Start-Sleep -Milliseconds $UiSettleMilliseconds }
}
function Close-Console {
    if ($script:consoleOpen) { Send-Key 0xC0 0x29 'close_console'; $script:consoleOpen=$false; Start-Sleep -Milliseconds $UiSettleMilliseconds }
}
function Invoke-Console([string]$Command, [string]$Label) {
    Open-Console
    Assert-Window "type_$Label"
    # SendKeys interprets braces, brackets, parentheses and modifier glyphs as
    # control syntax. Quote every such character so the Clausewitz console
    # receives the exact effect text recorded in the immutable receipt.
    $literalCommand = $Command -replace '([+^%~()\[\]{}])', '{$1}'
    [Windows.Forms.SendKeys]::SendWait($literalCommand)
    Send-Key 0x0D 0x1C "execute_$Label"
    $steps.Add([ordered]@{ action=$Label; command=$Command; method='authenticated_console_text_physical_enter' })
    Start-Sleep -Milliseconds $UiSettleMilliseconds
}
function Read-Hour {
    $moduleBase = [uint64](Get-Process -Id $ProcessId -ErrorAction Stop).MainModule.BaseAddress.ToInt64()
    $pointerBytes = [byte[]]::new(8); $read=[UIntPtr]::Zero
    if (-not [Hoi4A18A19Native]::ReadProcessMemory($handle,[IntPtr]([int64]($moduleBase+$gameStateGlobalRva)),$pointerBytes,[UIntPtr]::new(8),[ref]$read) -or $read.ToUInt64() -ne 8) { throw 'Game-state pointer read failed.' }
    $gameState=[BitConverter]::ToUInt64($pointerBytes,0); if($gameState -eq 0){throw 'Game-state singleton is null.'}
    $hourBytes=[byte[]]::new(4); $read=[UIntPtr]::Zero
    if (-not [Hoi4A18A19Native]::ReadProcessMemory($handle,[IntPtr]([int64]($gameState+$gameHourOffset)),$hourBytes,[UIntPtr]::new(4),[ref]$read) -or $read.ToUInt64() -ne 4) { throw 'Game-hour read failed.' }
    return [BitConverter]::ToInt32($hourBytes,0)
}
function Advance-One-Hour([string]$Label) {
    $before=Read-Hour
    Send-Key 0x31 0x02 "${Label}_speed1"
    Send-Key 0x20 0x39 "${Label}_resume"
    $deadline=[DateTime]::UtcNow.AddSeconds(30)
    do { Start-Sleep -Milliseconds 100; $after=Read-Hour } while($after -eq $before -and [DateTime]::UtcNow -lt $deadline)
    Send-Key 0x20 0x39 "${Label}_pause"
    if($after -ne ($before+1)){throw "$Label advanced $($after-$before) hours; exact one-hour pulse required."}
    $steps.Add([ordered]@{action=$Label;game_hour_before=$before;game_hour_after=$after;method='speed1_authenticated_pause_pulse'})
}
function Wait-File([string]$Path,[string]$Label) {
    $deadline=[DateTime]::UtcNow.AddSeconds($ActionTimeoutSeconds)
    $stableHash=$null;$stableSince=[DateTime]::MinValue
    while([DateTime]::UtcNow -lt $deadline){
        try{$hash=(Get-FileHash -LiteralPath $Path -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()}catch{$hash=$null}
        if($null -ne $hash -and $hash -ceq $stableHash){if(([DateTime]::UtcNow-$stableSince).TotalMilliseconds -ge 500){return}}
        else{$stableHash=$hash;$stableSince=[DateTime]::UtcNow}
        Start-Sleep -Milliseconds 125
    }
    throw "Timed out waiting for stable ${Label}: $Path"
}

try {
    if($InitialSettleSeconds -gt 0){Start-Sleep -Seconds $InitialSettleSeconds}
    $initialHour=Read-Hour
    Invoke-Console "tag $OriginatorTag" 'select_originator'
    if($Mode -ceq 'A18DynamicModifier'){
        if([string]::IsNullOrWhiteSpace($TimedModifier) -or [string]::IsNullOrWhiteSpace($PermanentModifier) -or [string]::IsNullOrWhiteSpace($CheckpointSavePath) -or [string]::IsNullOrWhiteSpace($CheckpointDirectory)){throw 'A18 target/checkpoint parameters are required.'}
        $save=[IO.Path]::GetFullPath($CheckpointSavePath)
        if([IO.Path]::GetFileName($save) -ine 'quicksave.hoi4'){throw 'A18 autonomy requires CheckpointSavePath to be quicksave.hoi4 so Ctrl+S has one deterministic target.'}
        Invoke-Console "eval_effect remove_dynamic_modifier = $TimedModifier" 'remove_stale_timed'
        Invoke-Console "eval_effect remove_dynamic_modifier = $PermanentModifier" 'remove_stale_permanent'
        Invoke-Console "eval_effect add_dynamic_modifier = { modifier = $PermanentModifier }" 'add_permanent'
        Invoke-Console "eval_effect add_dynamic_modifier = { modifier = $TimedModifier days = 2 }" 'add_timed_two_days'
        Close-Console; Send-ControlS 'save_positive_before'
        $checkpointRoot=[IO.Path]::GetFullPath($CheckpointDirectory)
        Wait-File (Join-Path $checkpointRoot 'a18_checkpoint_positive_before.json') 'positive_before checkpoint'
        foreach($phase in @('positive_after','post_removal')){
            $phasePath=Join-Path $checkpointRoot ("a18_checkpoint_{0}.json" -f $phase)
            for($pulse=1;$pulse -le 30 -and -not [IO.File]::Exists($phasePath);$pulse++){
                Advance-One-Hour "${phase}_pulse_$pulse"; Send-ControlS "save_${phase}_$pulse"; Start-Sleep -Milliseconds 1800
            }
            Wait-File $phasePath "$phase checkpoint"
        }
        $artifacts=@('positive_before','positive_after','post_removal')|ForEach-Object{$p=Join-Path $checkpointRoot ("a18_checkpoint_{0}.json" -f $_);[ordered]@{phase=$_;file=[IO.Path]::GetFileName($p);sha256=(Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash.ToLowerInvariant()}}
        $status='pass_three_exact_checkpoints'
    } else {
        if([string]::IsNullOrWhiteSpace($ActiveEventDefinitionPath) -or [string]::IsNullOrWhiteSpace($CancelledEventDefinitionSource)){throw 'A19 active/cancelled event-definition paths are required.'}
        $active=(Resolve-Path -LiteralPath $ActiveEventDefinitionPath).Path; $cancelled=(Resolve-Path -LiteralPath $CancelledEventDefinitionSource).Path
        Invoke-Console "eval_effect $RecipientTag = { country_event = { id = $ExecutedEventId hours = 3 } country_event = { id = $FailedTriggerEventId hours = 3 } country_event = { id = $CancelledEventId hours = 100 } }" 'queue_three_delayed_events'
        Close-Console
        1..4|ForEach-Object{Advance-One-Hour "delayed_lifecycle_hour_$_"}
        $beforeHash=(Get-FileHash -LiteralPath $active -Algorithm SHA256).Hash.ToLowerInvariant()
        $replacement=[IO.File]::ReadAllBytes($cancelled); $tmp="$active.a19-$([Guid]::NewGuid().ToString('N')).tmp"
        try{
            [IO.File]::WriteAllBytes($tmp,$replacement)
            $stream=[IO.File]::Open($tmp,'Open','ReadWrite','None');try{$stream.Flush($true)}finally{$stream.Dispose()}
            [IO.File]::Replace($tmp,$active,$null,$true)
        }finally{if(Test-Path -LiteralPath $tmp){Remove-Item -LiteralPath $tmp -Force}}
        $afterHash=(Get-FileHash -LiteralPath $active -Algorithm SHA256).Hash.ToLowerInvariant()
        Invoke-Console 'reload events' 'reload_events_without_cancelled_definition'; Close-Console
        $steps.Add([ordered]@{action='definition_transition';active_event_file=[IO.Path]::GetFileName($active);before_sha256=$beforeHash;after_sha256=$afterHash;cancelled_event_id=$CancelledEventId})
        $artifacts=@([ordered]@{kind='active_event_definition_after_cancel';sha256=$afterHash})
        $status='pass_queued_decremented_executed_failed_and_definition_reloaded'
    }
    $receipt=[ordered]@{schema='hoi4.a18_a19.autonomous_action_receipt.v1';status=$status;mode=$Mode;process_id=$ProcessId;process_start_utc=$processStartUtc;executable_sha256=$exeSha256;originator_tag=$OriginatorTag;recipient_tag=$RecipientTag;initial_game_hour=$initialHour;final_game_hour=(Read-Hour);steps=@($steps);artifacts=@($artifacts);immutable_receipt=$true}
    Write-AtomicNewJson $ReceiptPath $receipt
} finally {
    if($consoleOpen){try{Close-Console}catch{}}
    if($handle -ne [IntPtr]::Zero){[void][Hoi4A18A19Native]::CloseHandle($handle)}
}
