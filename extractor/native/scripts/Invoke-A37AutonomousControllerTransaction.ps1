[CmdletBinding()]
param(
    [switch] $EnableLegacyClientDriving,
    [Parameter(Mandatory = $true)] [int] $ProcessId,
    [Parameter(Mandatory = $true)] [string] $ExpectedProcessStartUtc,
    [Parameter(Mandatory = $true)] [ValidateSet('PositiveCurrentNegativeChange', 'NegativeCurrentPositiveChange')] [string] $Case,
    [Parameter(Mandatory = $true)] [string] $EvidenceDirectory,
    [Parameter(Mandatory = $true)] [string] $ReceiptPath,
    [Parameter(Mandatory = $true)] [string] $SessionId,
    [Parameter(Mandatory = $true)] [string] $CaptureId,
    [int] $ExpectedClientWidth = 1920,
    [int] $ExpectedClientHeight = 1080,
    [int] $ExpectedGameHour = 60830441,
    [int] $ExpectedD04Handle = 368,
    [int] $InitialSettleSeconds = 75,
    [int] $EvidenceTimeoutSeconds = 90,
    [int] $UiSettleMilliseconds = 1100
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not $EnableLegacyClientDriving) {
    throw 'Disabled by default: this legacy A37 harness drives the game client. Use Invoke-A37PromotionAcquisition.ps1 for passive observation. Explicit opt-in is required only for isolated manual harness work.'
}

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$gameStateGlobalRva = [uint64]0x03316e90
$gameHourOffset = [uint64]0x468
$primaryCountryOffset = [uint64]0x520
$fallbackCountryOffset = [uint64]0x524

if ($ProcessId -le 0) { throw 'ProcessId must be positive.' }
if ($InitialSettleSeconds -lt 0 -or $InitialSettleSeconds -gt 300) { throw 'InitialSettleSeconds must be in 0..300.' }
if ($EvidenceTimeoutSeconds -lt 15 -or $EvidenceTimeoutSeconds -gt 300) { throw 'EvidenceTimeoutSeconds must be in 15..300.' }
if ($UiSettleMilliseconds -lt 500 -or $UiSettleMilliseconds -gt 5000) { throw 'UiSettleMilliseconds must be in 500..5000.' }

$resolvedEvidence = [IO.Path]::GetFullPath($EvidenceDirectory)
$resolvedReceipt = [IO.Path]::GetFullPath($ReceiptPath)
if ([IO.Directory]::Exists($resolvedEvidence) -or [IO.File]::Exists($resolvedEvidence)) {
    throw "EvidenceDirectory must not exist before the target transaction: $resolvedEvidence"
}
if ([IO.File]::Exists($resolvedReceipt) -or [IO.Directory]::Exists($resolvedReceipt)) {
    throw "ReceiptPath must be new: $resolvedReceipt"
}

function Write-AtomicUtf8Text([string]$Path, [string]$Text) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($fullPath)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporaryPath = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText($temporaryPath, $Text, [Text.UTF8Encoding]::new($false))
        $stream = [IO.File]::Open($temporaryPath, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($fullPath)) { throw "Refusing to replace existing receipt: $fullPath" }
        [IO.File]::Move($temporaryPath, $fullPath)
    } finally {
        if ([IO.File]::Exists($temporaryPath)) { [IO.File]::Delete($temporaryPath) }
    }
}

Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Hoi4A37TransactionNative {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("kernel32.dll", SetLastError = true)] public static extern IntPtr OpenProcess(uint access, bool inherit, uint processId);
    [DllImport("kernel32.dll", SetLastError = true)] public static extern bool ReadProcessMemory(IntPtr process, IntPtr address, byte[] buffer, UIntPtr size, out UIntPtr read);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr handle);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool GetClientRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool ClientToScreen(IntPtr window, ref POINT point);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);
    [DllImport("user32.dll")] public static extern void keybd_event(byte virtualKey, byte scanCode, uint flags, UIntPtr extraInfo);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
}
'@

$process = Get-Process -Id $ProcessId -ErrorAction Stop
$observedStartUtc = $process.StartTime.ToUniversalTime().ToString('o')
if ([DateTime]::Parse($observedStartUtc).ToUniversalTime() -ne [DateTime]::Parse($ExpectedProcessStartUtc).ToUniversalTime()) {
    throw 'Exact-a729 PID was reused or process start identity changed.'
}
$exePath = $process.MainModule.FileName
$exeSha256 = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($exeSha256 -cne $expectedExeSha256) { throw "Executable SHA-256 mismatch: $exeSha256" }
$moduleBase = [uint64]$process.MainModule.BaseAddress.ToInt64()
$window = $process.MainWindowHandle
if ($window -eq [IntPtr]::Zero) { throw 'Exact-a729 process has no main window.' }
$processHandle = [Hoi4A37TransactionNative]::OpenProcess(0x1010, $false, [uint32]$ProcessId)
if ($processHandle -eq [IntPtr]::Zero) { throw 'Read-only OpenProcess failed.' }
$steps = [Collections.Generic.List[object]]::new()
$consoleOpen = $false

function Assert-AuthenticatedWindow([string]$Label) {
    $current = Get-Process -Id $ProcessId -ErrorAction Stop
    if ($current.MainWindowHandle -ne $window -or $window -eq [IntPtr]::Zero) { throw "Window identity changed before $Label." }
    $owner = [uint32]0
    [void][Hoi4A37TransactionNative]::GetWindowThreadProcessId($window, [ref]$owner)
    if ($owner -ne $ProcessId) { throw "Window owner changed before $Label." }
    if ([Hoi4A37TransactionNative]::GetForegroundWindow() -ne $window) {
        [void][Hoi4A37TransactionNative]::ShowWindowAsync($window, 9)
        [void][Hoi4A37TransactionNative]::SetForegroundWindow($window)
        Start-Sleep -Milliseconds 350
    }
    if ([Hoi4A37TransactionNative]::GetForegroundWindow() -ne $window) { throw "Exact-a729 window is not foreground before $Label." }
    $rect = [Hoi4A37TransactionNative+RECT]::new()
    if (-not [Hoi4A37TransactionNative]::GetClientRect($window, [ref]$rect)) { throw "GetClientRect failed before $Label." }
    if (($rect.Right - $rect.Left) -ne $ExpectedClientWidth -or ($rect.Bottom - $rect.Top) -ne $ExpectedClientHeight) {
        throw "Client dimensions differ from the exact 1920x1080 A37 profile before $Label."
    }
}

function Read-ExactBytes([uint64]$Address, [int]$Length) {
    $buffer = [byte[]]::new($Length)
    $read = [UIntPtr]::Zero
    if (-not [Hoi4A37TransactionNative]::ReadProcessMemory($processHandle, [IntPtr]([int64]$Address), $buffer, [UIntPtr]::new([uint64]$Length), [ref]$read) -or $read.ToUInt64() -ne [uint64]$Length) {
        throw ('Registered read failed at 0x{0:x}.' -f $Address)
    }
    return $buffer
}

function Read-U64([uint64]$Address) { return [BitConverter]::ToUInt64((Read-ExactBytes $Address 8), 0) }
function Read-I32([uint64]$Address) { return [BitConverter]::ToInt32((Read-ExactBytes $Address 4), 0) }

$gameState = Read-U64 ($moduleBase + $gameStateGlobalRva)
if ($gameState -eq 0) { throw 'Game-state singleton is null.' }

function Read-Observation([string]$Label) {
    $primary = Read-I32 ($gameState + $primaryCountryOffset)
    $fallback = Read-I32 ($gameState + $fallbackCountryOffset)
    return [ordered]@{
        label = $Label
        timestamp_utc = [DateTime]::UtcNow.ToString('o')
        primary_country_handle = $primary
        fallback_country_handle = $fallback
        effective_country_handle = $(if ($primary -gt 0) { $primary } else { $fallback })
        game_hour = Read-I32 ($gameState + $gameHourOffset)
    }
}

function Assert-Frozen([int]$ExpectedHandle, [string]$Label) {
    $first = Read-Observation "${Label}_a"
    Start-Sleep -Milliseconds 1200
    $second = Read-Observation "${Label}_b"
    if ($first.game_hour -ne $ExpectedGameHour -or $second.game_hour -ne $ExpectedGameHour -or
        $first.effective_country_handle -ne $ExpectedHandle -or $second.effective_country_handle -ne $ExpectedHandle) {
        throw "Country/hour readiness or freeze invariant failed at $Label."
    }
    return $second
}

function Send-PhysicalKey([byte]$VirtualKey, [byte]$ScanCode, [string]$Label) {
    Assert-AuthenticatedWindow $Label
    [Hoi4A37TransactionNative]::keybd_event($VirtualKey, $ScanCode, 0, [UIntPtr]::Zero)
    [Hoi4A37TransactionNative]::keybd_event($VirtualKey, $ScanCode, 2, [UIntPtr]::Zero)
}

function Open-Console {
    if ($script:consoleOpen) { return }
    Send-PhysicalKey 0xC0 0x29 'open_console'
    $script:consoleOpen = $true
    Start-Sleep -Milliseconds $UiSettleMilliseconds
}

function Close-Console {
    if (-not $script:consoleOpen) { return }
    Send-PhysicalKey 0xC0 0x29 'close_console'
    $script:consoleOpen = $false
    Start-Sleep -Milliseconds $UiSettleMilliseconds
}

function Invoke-ConsoleCommand([string]$Command, [string]$Label) {
    Open-Console
    Assert-AuthenticatedWindow "type_$Label"
    $sendKeysText = [Text.StringBuilder]::new()
    foreach ($character in $Command.ToCharArray()) {
        if ($character -eq [char]'{') {
            [void]$sendKeysText.Append('{{}')
        } elseif ($character -eq [char]'}') {
            [void]$sendKeysText.Append('{}}')
        } else {
            [void]$sendKeysText.Append($character)
        }
    }
    [System.Windows.Forms.SendKeys]::SendWait($sendKeysText.ToString())
    Start-Sleep -Milliseconds 250
    Send-PhysicalKey 0x0D 0x1C "execute_$Label"
    $steps.Add([ordered]@{ action = $Label; command = $Command; method = 'authenticated_console_text_plus_physical_enter' })
    Start-Sleep -Milliseconds $UiSettleMilliseconds
}

function Invoke-Click([int]$X, [int]$Y, [string]$Label) {
    Assert-AuthenticatedWindow $Label
    $point = [Hoi4A37TransactionNative+POINT]::new()
    $point.X = $X
    $point.Y = $Y
    if (-not [Hoi4A37TransactionNative]::ClientToScreen($window, [ref]$point)) { throw "ClientToScreen failed for $Label." }
    if (-not [Hoi4A37TransactionNative]::SetCursorPos($point.X, $point.Y)) { throw "SetCursorPos failed for $Label." }
    [Hoi4A37TransactionNative]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    [Hoi4A37TransactionNative]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    $steps.Add([ordered]@{ action = $Label; client_point = @($X, $Y); method = 'physical_setcursorpos_mouse_event' })
    Start-Sleep -Milliseconds $UiSettleMilliseconds
}

try {
    Assert-AuthenticatedWindow 'initial_readiness'
    if ($InitialSettleSeconds -gt 0) { Start-Sleep -Seconds $InitialSettleSeconds }
    $initial = Assert-Frozen $ExpectedD04Handle 'fresh_d04_readiness'

    Invoke-ConsoleCommand 'tag ITA' 'select_ita'
    $itaProbe = Read-Observation 'ita_selected'
    if ($itaProbe.effective_country_handle -le 0 -or $itaProbe.effective_country_handle -eq $ExpectedD04Handle -or $itaProbe.game_hour -ne $ExpectedGameHour) {
        throw 'ITA selection did not produce a distinct positive current-country handle at the frozen D04 hour.'
    }
    $itaHandle = $itaProbe.effective_country_handle
    [void](Assert-Frozen $itaHandle 'ita_selected_frozen')

    if ($Case -ceq 'PositiveCurrentNegativeChange') {
        Invoke-ConsoleCommand 'eval_effect set_power_balance = { id = ITA_power_balance set_value = 0.50 }' 'set_positive_current'
        Invoke-ConsoleCommand 'eval_effect add_power_balance_modifier = { id = ITA_power_balance modifier = SWI_DEBUG_all_council }' 'force_negative_change'
    } else {
        Invoke-ConsoleCommand 'eval_effect set_power_balance = { id = ITA_power_balance set_value = -0.50 }' 'set_negative_current'
        foreach ($modifier in @(
            'FIN_bop_modifier_eradicate_socialism',
            'FIN_bop_modifier_eradicate_extremists',
            'FIN_bop_modifier_dream_of_greater_finland',
            'FIN_bop_modifier_reliable_leader',
            'FIN_bop_modifier_realist_conservative',
            'FIN_bop_modifier_strong_nation',
            'FIN_bop_modifier_united_nation',
            'FIN_bop_modifier_anti_fascist_crusade',
            'FIN_bop_modifier_obliterated_european_fascism',
            'FIN_bop_modifier_a_solar_system_of_organizations'
        )) {
            Invoke-ConsoleCommand "eval_effect add_power_balance_modifier = { id = ITA_power_balance modifier = $modifier }" "force_positive_change_$modifier"
        }
    }
    Close-Console
    [void](Assert-Frozen $itaHandle 'post_transaction_frozen')

    Invoke-Click 45 45 'open_ita_politics'
    Invoke-Click 402 321 'open_ita_power_balance'
    [void](Assert-Frozen $itaHandle 'open_panel_frozen')

    # The collector has evidence mode from process creation, but its one-shot
    # write cannot succeed until this exact point because the directory did
    # not exist. Exposing it only after the target transaction and open-panel
    # callback prevents an earlier zero-change/stale controller receipt.
    [IO.Directory]::CreateDirectory($resolvedEvidence) | Out-Null
    $rawPath = Join-Path $resolvedEvidence 'a37_raw_controller_selected_present.json'
    $deadline = [DateTime]::UtcNow.AddSeconds($EvidenceTimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline -and -not (Test-Path -LiteralPath $rawPath -PathType Leaf)) {
        Start-Sleep -Milliseconds 250
    }
    if (-not (Test-Path -LiteralPath $rawPath -PathType Leaf)) { throw 'Collector did not publish the A37 raw selected-Present receipt before timeout.' }

    $raw = Get-Content -LiteralPath $rawPath -Raw | ConvertFrom-Json
    $expectedCurrentSign = if ($Case -ceq 'PositiveCurrentNegativeChange') { 1 } else { -1 }
    $expectedChangeSign = -$expectedCurrentSign
    $currentRaw = [int64]$raw.hook_observation.current_value_raw
    $changeRaw = [int64]$raw.hook_observation.change_raw
    if ($raw.schema -cne 'hoi4.a37.raw_controller_selected_present.v1' -or
        $raw.hook_observation.panel_open -ne $true -or $raw.hook_observation.gui_visible -ne $true -or
        $currentRaw -eq 0 -or [Math]::Sign($currentRaw) -ne $expectedCurrentSign -or
        $changeRaw -eq 0 -or [Math]::Sign($changeRaw) -ne $expectedChangeSign) {
        throw "Raw A37 receipt did not carry the required $Case signs and open/visible join."
    }

    $final = Assert-Frozen $itaHandle 'post_receipt_frozen'
    $receipt = [ordered]@{
        schema = 'hoi4.a37.autonomous-controller-transaction.v1'
        status = 'pass_target_signs_selected_present_published'
        case = $Case
        timestamp_utc = [DateTime]::UtcNow.ToString('o')
        session_id = $SessionId
        capture_id = $CaptureId
        process_id = $ProcessId
        process_start_utc = $observedStartUtc
        executable_sha256 = $exeSha256
        initial_observation = $initial
        target_country_handle = $itaHandle
        expected_game_hour = $ExpectedGameHour
        exact_hour_preserved = $true
        evidence_directory_exposed_after_transaction = $true
        required_current_sign = $expectedCurrentSign
        required_change_sign = $expectedChangeSign
        observed_current_raw = $currentRaw
        observed_change_raw = $changeRaw
        panel_open = $raw.hook_observation.panel_open
        gui_visible = $raw.hook_observation.gui_visible
        raw_receipt_file_name = 'a37_raw_controller_selected_present.json'
        raw_receipt_bytes = (Get-Item -LiteralPath $rawPath).Length
        raw_receipt_sha256 = (Get-FileHash -LiteralPath $rawPath -Algorithm SHA256).Hash.ToLowerInvariant()
        steps = $steps
        final_observation = $final
        mutation_scope = 'fresh disposable process only; frozen save bytes are never written'
    }
    Write-AtomicUtf8Text $resolvedReceipt (($receipt | ConvertTo-Json -Depth 12) + "`n")
    $receipt | ConvertTo-Json -Depth 12
} finally {
    if ($consoleOpen) {
        try { Close-Console } catch { }
    }
    if ($processHandle -ne [IntPtr]::Zero) { [void][Hoi4A37TransactionNative]::CloseHandle($processHandle) }
}
