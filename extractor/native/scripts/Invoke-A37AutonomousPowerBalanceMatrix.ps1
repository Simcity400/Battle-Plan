[CmdletBinding()]
param(
    [switch] $EnableLegacyClientDriving,
    [Parameter(Mandatory = $true)] [int] $ProcessId,
    [Parameter(Mandatory = $true)] [string] $OutputDirectory,
    [Parameter(Mandatory = $true)] [switch] $ConfirmExclusiveUiLease,
    [Parameter(Mandatory = $true)] [switch] $ConfirmInitialD04MapSurface,
    [int] $ExpectedClientWidth = 1920,
    [int] $ExpectedClientHeight = 1080,
    [int] $ExpectedGameHour = 60830441,
    [int] $ExpectedD04Handle = 368,
    [int] $ExpectedITAHandle = 16,
    [int] $UiSettleMilliseconds = 900,
    [int] $FreezeProbeMilliseconds = 1500,
    [int] $StableDwellMilliseconds = 1800
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not $EnableLegacyClientDriving) {
    throw 'Disabled by default: this legacy A37 matrix drives the game client. It is isolated from the passive production observer and requires explicit opt-in for manual harness work.'
}

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$gameStateGlobalRva = [uint64]0x03316e90
$gameHourOffset = [uint64]0x468
$primaryCountryOffset = [uint64]0x520
$fallbackCountryOffset = [uint64]0x524

if ($ProcessId -le 0) { throw 'ProcessId must be positive.' }
if (-not $ConfirmExclusiveUiLease) {
    throw 'Fail closed: the caller must hold and affirm the exclusive live/UI lease.'
}
if (-not $ConfirmInitialD04MapSurface) {
    throw 'Fail closed: the caller must affirm the clean D04 map surface with console and politics overlays closed.'
}
if ($ExpectedClientWidth -le 0 -or $ExpectedClientHeight -le 0) {
    throw 'Expected client dimensions must be positive.'
}
if ($UiSettleMilliseconds -lt 500 -or $UiSettleMilliseconds -gt 5000) {
    throw 'UiSettleMilliseconds must be in 500..5000.'
}
if ($FreezeProbeMilliseconds -lt 1000 -or $FreezeProbeMilliseconds -gt 5000) {
    throw 'FreezeProbeMilliseconds must be in 1000..5000.'
}
if ($StableDwellMilliseconds -lt 1000 -or $StableDwellMilliseconds -gt 10000) {
    throw 'StableDwellMilliseconds must be in 1000..10000.'
}

function Write-AtomicUtf8Text([string]$Path, [string]$Text) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($fullPath)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporaryPath = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText($temporaryPath, $Text, [Text.UTF8Encoding]::new($false))
        if ([IO.File]::Exists($fullPath)) {
            throw "Refusing to replace an existing A37 matrix receipt: $fullPath"
        }
        [IO.File]::Move($temporaryPath, $fullPath)
    } finally {
        if ([IO.File]::Exists($temporaryPath)) { [IO.File]::Delete($temporaryPath) }
    }
}

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Hoi4A37MatrixNative {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("kernel32.dll", SetLastError = true)] public static extern IntPtr OpenProcess(uint access, bool inherit, uint processId);
    [DllImport("kernel32.dll", SetLastError = true)] public static extern bool ReadProcessMemory(IntPtr process, IntPtr address, byte[] buffer, UIntPtr size, out UIntPtr read);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr handle);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool GetClientRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool ClientToScreen(IntPtr window, ref POINT point);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool GetCursorPos(out POINT point);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);
    [DllImport("user32.dll")] public static extern void keybd_event(byte virtualKey, byte scanCode, uint flags, UIntPtr extraInfo);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
}
'@

$output = [IO.Path]::GetFullPath($OutputDirectory)
if ([IO.Directory]::Exists($output) -or [IO.File]::Exists($output)) {
    throw "Refusing to reuse an A37 matrix output path: $output"
}
[IO.Directory]::CreateDirectory($output) | Out-Null
$receiptPath = [IO.Path]::Combine($output, 'a37_autonomous_matrix_receipt.json')
$failurePath = [IO.Path]::Combine($output, 'a37_autonomous_matrix_failure.json')

$process = $null
$window = [IntPtr]::Zero
$processHandle = [IntPtr]::Zero
$moduleBase = [uint64]0
$gameState = [uint64]0
$consoleOpen = $false
$restoreAttempted = $false
$repauseAttempted = $false
$restored = $false
$completed = $false
$initialObservation = $null
$latestObservation = $null
$steps = [Collections.Generic.List[object]]::new()
$captures = [Collections.Generic.List[object]]::new()
$transitions = [Collections.Generic.List[object]]::new()

function Assert-AuthenticatedWindow([string]$ActionLabel) {
    $current = Get-Process -Id $ProcessId -ErrorAction Stop
    if ($current.MainWindowHandle -ne $window -or $window -eq [IntPtr]::Zero) {
        throw "Fail closed: exact-a729 window identity changed before $ActionLabel."
    }
    $owner = [uint32]0
    [void][Hoi4A37MatrixNative]::GetWindowThreadProcessId($window, [ref]$owner)
    if ($owner -ne $ProcessId) {
        throw "Fail closed: window owner PID $owner differs before $ActionLabel."
    }
    if ([Hoi4A37MatrixNative]::GetForegroundWindow() -ne $window) {
        [void][Hoi4A37MatrixNative]::ShowWindowAsync($window, 9)
        [void][Hoi4A37MatrixNative]::SetForegroundWindow($window)
        Start-Sleep -Milliseconds 300
    }
    if ([Hoi4A37MatrixNative]::GetForegroundWindow() -ne $window) {
        throw "Fail closed: exact-a729 window is not foreground before $ActionLabel."
    }
    $rect = [Hoi4A37MatrixNative+RECT]::new()
    if (-not [Hoi4A37MatrixNative]::GetClientRect($window, [ref]$rect)) {
        throw "Fail closed: GetClientRect failed before $ActionLabel."
    }
    if (($rect.Right - $rect.Left) -ne $ExpectedClientWidth -or
        ($rect.Bottom - $rect.Top) -ne $ExpectedClientHeight) {
        throw "Fail closed: client dimensions changed before $ActionLabel."
    }
}

function Read-ExactBytes([uint64]$Address, [int]$Length) {
    if ($Address -eq 0 -or $Length -le 0) { throw 'Fail closed: invalid memory read request.' }
    $buffer = [byte[]]::new($Length)
    $read = [UIntPtr]::Zero
    $size = [UIntPtr]::new([uint64]$Length)
    if (-not [Hoi4A37MatrixNative]::ReadProcessMemory(
        $processHandle,
        [IntPtr]([int64]$Address),
        $buffer,
        $size,
        [ref]$read
    ) -or $read.ToUInt64() -ne [uint64]$Length) {
        throw ('Fail closed: exact registered memory read failed at RVA-backed address 0x{0:x}.' -f $Address)
    }
    return $buffer
}

function Read-U64([uint64]$Address) {
    return [BitConverter]::ToUInt64((Read-ExactBytes $Address 8), 0)
}

function Read-I32([uint64]$Address) {
    return [BitConverter]::ToInt32((Read-ExactBytes $Address 4), 0)
}

function Read-Observation([string]$Label) {
    $primary = Read-I32 ($gameState + $primaryCountryOffset)
    $fallback = Read-I32 ($gameState + $fallbackCountryOffset)
    $effective = if ($primary -gt 0) { $primary } else { $fallback }
    return [ordered]@{
        label = $Label
        timestamp_utc = [DateTime]::UtcNow.ToString('o')
        primary_country_handle = $primary
        fallback_country_handle = $fallback
        effective_country_handle = $effective
        game_hour = Read-I32 ($gameState + $gameHourOffset)
    }
}

function Send-PhysicalKey([byte]$VirtualKey, [byte]$ScanCode, [string]$ActionLabel) {
    Assert-AuthenticatedWindow $ActionLabel
    [Hoi4A37MatrixNative]::keybd_event($VirtualKey, $ScanCode, 0, [UIntPtr]::Zero)
    [Hoi4A37MatrixNative]::keybd_event($VirtualKey, $ScanCode, 2, [UIntPtr]::Zero)
}

function Try-Repause([string]$Reason) {
    $script:repauseAttempted = $true
    if ($script:consoleOpen) { Close-Console }
    Send-PhysicalKey 0x20 0x39 "emergency_repause_$Reason"
    Start-Sleep -Milliseconds $FreezeProbeMilliseconds
}

function Assert-FrozenState([int]$CountryHandle, [int]$GameHour, [string]$Label) {
    $first = Read-Observation "${Label}_probe_a"
    Start-Sleep -Milliseconds $FreezeProbeMilliseconds
    $second = Read-Observation "${Label}_probe_b"
    $script:latestObservation = $second
    $valid = $first.effective_country_handle -eq $CountryHandle -and
        $second.effective_country_handle -eq $CountryHandle -and
        $first.game_hour -eq $GameHour -and
        $second.game_hour -eq $GameHour
    if (-not $valid) {
        if ($first.game_hour -ne $second.game_hour -or $second.game_hour -ne $GameHour) {
            Try-Repause $Label
        }
        throw "Fail closed: country/hour freeze invariant failed at $Label."
    }
    return $second
}

function Move-AuthenticatedCursor([int]$X, [int]$Y, [string]$ActionLabel) {
    Assert-AuthenticatedWindow $ActionLabel
    if ($X -lt 0 -or $X -ge $ExpectedClientWidth -or $Y -lt 0 -or $Y -ge $ExpectedClientHeight) {
        throw "Fail closed: client point ($X,$Y) is outside the frozen A37 profile."
    }
    $point = [Hoi4A37MatrixNative+POINT]::new()
    $point.X = $X
    $point.Y = $Y
    if (-not [Hoi4A37MatrixNative]::ClientToScreen($window, [ref]$point)) {
        throw "Fail closed: ClientToScreen failed for $ActionLabel."
    }
    if (-not [Hoi4A37MatrixNative]::SetCursorPos($point.X, $point.Y)) {
        throw "Fail closed: SetCursorPos failed for $ActionLabel."
    }
    $actual = [Hoi4A37MatrixNative+POINT]::new()
    if (-not [Hoi4A37MatrixNative]::GetCursorPos([ref]$actual) -or
        $actual.X -ne $point.X -or $actual.Y -ne $point.Y) {
        throw "Fail closed: cursor placement was not exact for $ActionLabel."
    }
    return @($point.X, $point.Y)
}

function Invoke-AuthenticatedClick([int]$X, [int]$Y, [string]$ActionLabel) {
    $screen = Move-AuthenticatedCursor $X $Y $ActionLabel
    [Hoi4A37MatrixNative]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    [Hoi4A37MatrixNative]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    $steps.Add([ordered]@{
        action = $ActionLabel
        method = 'physical_setcursorpos_mouse_event'
        client_point = @($X, $Y)
        screen_point = $screen
    })
    Start-Sleep -Milliseconds $UiSettleMilliseconds
}

function Open-Console {
    if ($script:consoleOpen) { return }
    Send-PhysicalKey 0xC0 0x29 'open_console'
    $script:consoleOpen = $true
    $steps.Add([ordered]@{ action = 'open_console'; method = 'physical_keybd_event'; virtual_key = '0xc0'; scan_code = '0x29' })
    Start-Sleep -Milliseconds $UiSettleMilliseconds
}

function Close-Console {
    if (-not $script:consoleOpen) { return }
    Send-PhysicalKey 0xC0 0x29 'close_console'
    $script:consoleOpen = $false
    $steps.Add([ordered]@{ action = 'close_console'; method = 'physical_keybd_event'; virtual_key = '0xc0'; scan_code = '0x29' })
    Start-Sleep -Milliseconds $UiSettleMilliseconds
}

function Invoke-TagSwitch([string]$Tag, [int]$ExpectedHandle, [string]$Label) {
    Open-Console
    Assert-AuthenticatedWindow "type_$Label"
    [System.Windows.Forms.SendKeys]::SendWait("tag $Tag")
    Start-Sleep -Milliseconds 250
    Send-PhysicalKey 0x0D 0x1C "execute_$Label"
    $steps.Add([ordered]@{
        action = $Label
        method = 'authenticated_console_text_plus_physical_enter'
        tag = $Tag
        expected_country_handle = $ExpectedHandle
    })
    Start-Sleep -Milliseconds $UiSettleMilliseconds
    return Assert-FrozenState $ExpectedHandle $ExpectedGameHour "${Label}_post"
}

function Save-AtomicClientPng([string]$FileName) {
    Assert-AuthenticatedWindow "capture_$FileName"
    $fullPath = [IO.Path]::Combine($output, $FileName)
    if ([IO.File]::Exists($fullPath)) { throw "Refusing to replace capture $FileName." }
    $origin = [Hoi4A37MatrixNative+POINT]::new()
    $origin.X = 0
    $origin.Y = 0
    if (-not [Hoi4A37MatrixNative]::ClientToScreen($window, [ref]$origin)) {
        throw "Fail closed: ClientToScreen failed for capture $FileName."
    }
    $temporaryPath = [IO.Path]::Combine($output, ([IO.Path]::GetRandomFileName() + '.png'))
    $bitmap = [Drawing.Bitmap]::new($ExpectedClientWidth, $ExpectedClientHeight)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($origin.X, $origin.Y, 0, 0, $bitmap.Size)
        $bitmap.Save($temporaryPath, [Drawing.Imaging.ImageFormat]::Png)
        [IO.File]::Move($temporaryPath, $fullPath)
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
        if ([IO.File]::Exists($temporaryPath)) { [IO.File]::Delete($temporaryPath) }
    }
    $item = Get-Item -LiteralPath $fullPath
    return [ordered]@{
        file_name = $FileName
        bytes = $item.Length
        sha256 = (Get-FileHash -LiteralPath $fullPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

function Capture-MatrixStage([string]$Stage, [string]$FileName, [int]$CountryHandle) {
    $before = Assert-FrozenState $CountryHandle $ExpectedGameHour "${Stage}_before_capture"
    $image = Save-AtomicClientPng $FileName
    $after = Assert-FrozenState $CountryHandle $ExpectedGameHour "${Stage}_after_capture"
    $row = [ordered]@{
        stage = $Stage
        country_handle = $CountryHandle
        game_hour = $ExpectedGameHour
        before = $before
        after = $after
        image = $image
    }
    $captures.Add($row)
    return $row
}

function Write-FailureReceipt([string]$Message) {
    if ([IO.File]::Exists($failurePath)) { return }
    $failure = [ordered]@{
        schema = 'hoi4.a37.autonomous-power-balance-matrix-failure.v1'
        status = 'fail_closed'
        timestamp_utc = [DateTime]::UtcNow.ToString('o')
        executable_sha256_expected = $expectedExeSha256
        process_id = $ProcessId
        error = $Message
        restore_attempted = $restoreAttempted
        repause_attempted = $repauseAttempted
        restored_original_country_and_hour = $restored
        initial_observation = $initialObservation
        final_observation = $latestObservation
        completed_capture_count = $captures.Count
        completed_transition_count = $transitions.Count
        promotion_authorized = $false
    }
    Write-AtomicUtf8Text $failurePath (($failure | ConvertTo-Json -Depth 10) + "`n")
}

try {
    $process = Get-Process -Id $ProcessId -ErrorAction Stop
    $exePath = $process.MainModule.FileName
    $exeSha256 = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($exeSha256 -cne $expectedExeSha256) {
        throw "Fail closed: executable SHA-256 mismatch: $exeSha256"
    }
    $moduleBase = [uint64]$process.MainModule.BaseAddress.ToInt64()
    $window = $process.MainWindowHandle
    if ($window -eq [IntPtr]::Zero) { throw 'Fail closed: exact-a729 process has no main window.' }
    $processHandle = [Hoi4A37MatrixNative]::OpenProcess(0x1010, $false, [uint32]$ProcessId)
    if ($processHandle -eq [IntPtr]::Zero) { throw 'Fail closed: read-only OpenProcess failed.' }
    Assert-AuthenticatedWindow 'initial_preflight'
    $gameState = Read-U64 ($moduleBase + $gameStateGlobalRva)
    if ($gameState -eq 0) { throw 'Fail closed: game-state singleton is null.' }

    $initialObservation = Read-Observation 'initial'
    $latestObservation = $initialObservation
    if ($initialObservation.effective_country_handle -ne $ExpectedD04Handle) {
        throw "Fail closed: A37 matrix must begin on D04 handle $ExpectedD04Handle."
    }
    if ($initialObservation.game_hour -ne $ExpectedGameHour) {
        throw "Fail closed: A37 matrix expected game hour $ExpectedGameHour."
    }
    [void](Capture-MatrixStage 'stable_d04_map_initial' '00_stable_d04_map_initial.png' $ExpectedD04Handle)

    Invoke-AuthenticatedClick 45 45 'open_d04_politics'
    [void](Capture-MatrixStage 'stable_d04_politics_no_balance_launcher' '01_stable_d04_politics.png' $ExpectedD04Handle)
    Invoke-AuthenticatedClick 45 45 'close_d04_politics'

    $d04Before = Assert-FrozenState $ExpectedD04Handle $ExpectedGameHour 'd04_to_ita_before'
    $itaAfter = Invoke-TagSwitch 'ITA' $ExpectedITAHandle 'd04_to_ita'
    [void](Capture-MatrixStage 'd04_to_ita_console_transition' '02_d04_to_ita_console.png' $ExpectedITAHandle)
    $transitions.Add([ordered]@{
        direction = 'D04_to_ITA'
        before = $d04Before
        after = $itaAfter
        game_hour_preserved = $true
    })
    Close-Console
    [void](Capture-MatrixStage 'stable_ita_map' '03_stable_ita_map.png' $ExpectedITAHandle)

    Invoke-AuthenticatedClick 45 45 'open_ita_politics'
    [void](Capture-MatrixStage 'stable_ita_politics_launcher_visible' '04_stable_ita_politics.png' $ExpectedITAHandle)
    Invoke-AuthenticatedClick 402 321 'open_ita_power_balance'
    [void](Move-AuthenticatedCursor 1000 1000 'park_cursor_for_stable_bop')
    [void](Capture-MatrixStage 'stable_ita_bop_open_a' '05_stable_ita_bop_open_a.png' $ExpectedITAHandle)
    Start-Sleep -Milliseconds $StableDwellMilliseconds
    [void](Capture-MatrixStage 'stable_ita_bop_open_b' '06_stable_ita_bop_open_b.png' $ExpectedITAHandle)

    $itaBefore = Assert-FrozenState $ExpectedITAHandle $ExpectedGameHour 'ita_to_d04_before'
    $d04After = Invoke-TagSwitch 'D04' $ExpectedD04Handle 'ita_to_d04_while_bop_open'
    [void](Capture-MatrixStage 'ita_to_d04_no_match_close_console' '07_ita_to_d04_no_match_close_console.png' $ExpectedD04Handle)
    $transitions.Add([ordered]@{
        direction = 'ITA_to_D04_while_BOP_open'
        before = $itaBefore
        after = $d04After
        game_hour_preserved = $true
    })
    Close-Console
    [void](Move-AuthenticatedCursor 1000 1000 'park_cursor_for_restored_d04')
    [void](Capture-MatrixStage 'stable_d04_restored' '08_stable_d04_restored.png' $ExpectedD04Handle)

    $latestObservation = Assert-FrozenState $ExpectedD04Handle $ExpectedGameHour 'final_restore'
    $restored = $true
    $receipt = [ordered]@{
        schema = 'hoi4.a37.autonomous-power-balance-matrix.v1'
        status = 'pass_guarded_matrix_restored'
        acquisition_owner = 'machine'
        timestamp_utc = [DateTime]::UtcNow.ToString('o')
        process_id = $ProcessId
        executable_file_name = [IO.Path]::GetFileName($exePath)
        executable_sha256 = $exeSha256
        client_size = @($ExpectedClientWidth, $ExpectedClientHeight)
        registered_memory_contract = [ordered]@{
            game_state_global_rva = '0x3316e90'
            game_hour_offset = '0x468'
            primary_country_offset = '0x520'
            fallback_country_offset = '0x524'
            current_country_rule = 'primary signed i32 when positive, otherwise fallback signed i32'
        }
        initial_observation = $initialObservation
        expected_game_hour = $ExpectedGameHour
        exact_hour_preserved = $true
        repause_attempted = $repauseAttempted
        original_country_restored = $restored
        console_closed = (-not $consoleOpen)
        stable_cases = @(
            'D04 map/no matching balance',
            'D04 Politics/no balance launcher',
            'ITA map/matching balance',
            'ITA Politics/launcher visible',
            'ITA BOP open across stable dwell',
            'D04 restored/no-match panel closed'
        )
        cross_direction_transitions = $transitions
        captures = $captures
        step_count = $steps.Count
        steps = $steps
        final_observation = $latestObservation
        isolation_policy = 'new output directory only; no shared recorder, manifest, hook, save, launch, or process-close mutation'
        caller_preconditions = [ordered]@{
            exclusive_ui_lease = $ConfirmExclusiveUiLease.IsPresent
            initial_clean_d04_map_surface = $ConfirmInitialD04MapSurface.IsPresent
        }
        promotion_authorized = $false
    }
    $receiptJson = $receipt | ConvertTo-Json -Depth 14
    Write-AtomicUtf8Text $receiptPath ($receiptJson + "`n")
    $completed = $true
    $receiptJson
} catch {
    $failureMessage = $_.Exception.Message
    throw
} finally {
    if (-not $completed -and $processHandle -ne [IntPtr]::Zero -and $window -ne [IntPtr]::Zero) {
        $restoreAttempted = $true
        try {
            $current = Read-Observation 'restore_probe'
            if ($null -ne $initialObservation -and
                $current.effective_country_handle -ne $initialObservation.effective_country_handle) {
                if (-not $consoleOpen) { Open-Console }
                [System.Windows.Forms.SendKeys]::SendWait('tag D04')
                Start-Sleep -Milliseconds 250
                Send-PhysicalKey 0x0D 0x1C 'restore_d04_after_failure'
                Start-Sleep -Milliseconds $UiSettleMilliseconds
            }
            if ($consoleOpen) { Close-Console }
            $probeA = Read-Observation 'restore_freeze_a'
            Start-Sleep -Milliseconds $FreezeProbeMilliseconds
            $probeB = Read-Observation 'restore_freeze_b'
            if ($probeA.game_hour -ne $probeB.game_hour) {
                Try-Repause 'restore_after_failure'
                $probeB = Read-Observation 'restore_after_repause'
            }
            $latestObservation = $probeB
            $restored = $null -ne $initialObservation -and
                $probeB.effective_country_handle -eq $initialObservation.effective_country_handle -and
                $probeB.game_hour -eq $initialObservation.game_hour
        } catch {
            $restored = $false
            if ([string]::IsNullOrWhiteSpace($failureMessage)) {
                $failureMessage = $_.Exception.Message
            } else {
                $failureMessage = "$failureMessage; restore error: $($_.Exception.Message)"
            }
        }
        if (-not [string]::IsNullOrWhiteSpace($failureMessage)) {
            Write-FailureReceipt $failureMessage
        }
    }
    if ($processHandle -ne [IntPtr]::Zero) {
        [void][Hoi4A37MatrixNative]::CloseHandle($processHandle)
    }
}
