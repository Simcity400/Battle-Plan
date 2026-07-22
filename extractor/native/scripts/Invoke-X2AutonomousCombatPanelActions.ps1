[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][int]$ProcessId,
    [Parameter(Mandatory = $true)][string]$ReceiptPath,
    [Parameter(Mandatory = $true)][string]$SessionId,
    [Parameter(Mandatory = $true)][string]$CaptureId,
    [Parameter(Mandatory = $true)][string]$ExpectedCollectorSha256,
    [Parameter(Mandatory = $true)][string]$PrebuiltAttestationSha256,
    [Parameter(Mandatory = $true)][string]$ProfilePath,
    [Parameter(Mandatory = $true)][string]$ReadyPath,
    [Parameter(Mandatory = $true)][string]$CaptureManifestPath,
    [int]$ExpectedClientWidth = 1920,
    [int]$ExpectedClientHeight = 1080,
    [int]$CollectorLoadTimeoutSeconds = 600,
    [int]$GameHourAdvanceTimeoutSeconds = 90,
    [int]$CaptureFinalizeTimeoutSeconds = 2400,
    [int]$GameHoursToHold = 2,
    [int]$PanelMaterializationDwellMilliseconds = 4000,
    [switch]$EnableLegacyClientDriving
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not $EnableLegacyClientDriving) {
    throw 'This legacy X2 client-driving tool is isolated and disabled by default. Passive observation is the production path; use -EnableLegacyClientDriving only for an explicitly authorized legacy experiment.'
}

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$gameStateSlotRva = 0x3316E90
$gameHourOffset = 0x468
$gameSpeedOffset = 0x4BC
$clockSingletonARva = 0x33172E0
$clockSingletonBRva = 0x33172E8
$clockVtableRva = 0x2952510
$pauseOffset = 0x6C1
$resolvedProfile = (Resolve-Path -LiteralPath $ProfilePath).Path
$profileSha256 = (Get-FileHash -LiteralPath $resolvedProfile -Algorithm SHA256).Hash.ToLowerInvariant()
$profile = Get-Content -LiteralPath $resolvedProfile -Raw | ConvertFrom-Json
if ([string]$profile.schema -cne 'hoi4.x2.autonomous_combat_panel_action_profile.v1' -or
    [string]$profile.status -cne 'FROZEN' -or
    [string]$profile.program_sha256 -cne $expectedExeSha256 -or
    [string]$profile.profile_id -cne 'd04_active_land_battle_bubbles_1920x1080_v1' -or
    (@($profile.client_size) -join ',') -cne '1920,1080') {
    throw 'X2 autonomous panel action profile identity is malformed.'
}
$profileActions = @($profile.actions)
$expectedPoints = @('1009,154', '1077,154', '1073,800', '1107,818', '1135,841')
if ($profileActions.Count -ne $expectedPoints.Count) { throw 'X2 autonomous panel action profile candidate count is malformed.' }
for ($profileOrdinal = 0; $profileOrdinal -lt $profileActions.Count; $profileOrdinal++) {
    $profileAction = $profileActions[$profileOrdinal]
    if ([int]$profileAction.ordinal -ne $profileOrdinal -or
        [string]$profileAction.kind -cne 'select_and_hold_land_battle_panel' -or
        (@($profileAction.client_point) -join ',') -cne $expectedPoints[$profileOrdinal] -or
        [int]$profileAction.click_hold_milliseconds -ne 80 -or
        [int]$profileAction.minimum_materialization_dwell_milliseconds -gt $PanelMaterializationDwellMilliseconds -or
        [int]$profileAction.minimum_game_hours_held -gt $GameHoursToHold) {
        throw "X2 autonomous panel action profile candidate $profileOrdinal is malformed or under-satisfied."
    }
}
$selectionProfile = [string]$profile.profile_id
$candidatePoints = [Collections.Generic.List[object]]::new()
foreach ($profileAction in $profileActions) {
    $candidatePoints.Add([int[]]@([int]$profileAction.client_point[0], [int]$profileAction.client_point[1]))
}

foreach ($identity in @($SessionId, $CaptureId)) {
    if ($identity -cnotmatch '^[A-Za-z0-9_.-]{8,160}$') {
        throw "Session/capture identity is noncanonical: $identity"
    }
}
if ($ExpectedCollectorSha256 -cnotmatch '^[0-9a-f]{64}$') {
    throw 'ExpectedCollectorSha256 must be lowercase SHA-256.'
}
if ($PrebuiltAttestationSha256 -cnotmatch '^[0-9a-f]{64}$') {
    throw 'PrebuiltAttestationSha256 must be lowercase SHA-256.'
}
if ($ExpectedClientWidth -ne 1920 -or $ExpectedClientHeight -ne 1080) {
    throw 'The frozen X2 D04 action profile supports only a 1920x1080 client.'
}
if ($CollectorLoadTimeoutSeconds -lt 1 -or $CollectorLoadTimeoutSeconds -gt 600) {
    throw 'CollectorLoadTimeoutSeconds must be in 1..600.'
}
if ($GameHourAdvanceTimeoutSeconds -lt 15 -or $GameHourAdvanceTimeoutSeconds -gt 300) {
    throw 'GameHourAdvanceTimeoutSeconds must be in 15..300.'
}
if ($CaptureFinalizeTimeoutSeconds -lt 60 -or $CaptureFinalizeTimeoutSeconds -gt 7200) {
    throw 'CaptureFinalizeTimeoutSeconds must be in 60..7200.'
}
if ($GameHoursToHold -lt 1 -or $GameHoursToHold -gt 24) {
    throw 'GameHoursToHold must be in 1..24.'
}
if ($PanelMaterializationDwellMilliseconds -lt 2000 -or $PanelMaterializationDwellMilliseconds -gt 30000) {
    throw 'PanelMaterializationDwellMilliseconds must be in 2000..30000.'
}

function Write-AtomicCreateNewUtf8Json([string]$Path, [object]$Value) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($fullPath)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    if ([IO.File]::Exists($fullPath)) {
        throw "Refusing to replace immutable X2 action receipt: $fullPath"
    }
    $temporaryPath = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        $payload = (($Value | ConvertTo-Json -Depth 12) + [Environment]::NewLine)
        [IO.File]::WriteAllText($temporaryPath, $payload, [Text.UTF8Encoding]::new($false))
        $stream = [IO.File]::Open($temporaryPath, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($fullPath)) {
            throw "Refusing to replace immutable X2 action receipt: $fullPath"
        }
        [IO.File]::Move($temporaryPath, $fullPath)
    } finally {
        if ([IO.File]::Exists($temporaryPath)) { [IO.File]::Delete($temporaryPath) }
    }
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Hoi4X2AutonomousPanelActions {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool GetClientRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool ClientToScreen(IntPtr hWnd, ref POINT point);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool GetCursorPos(out POINT point);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);
    [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extraInfo);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int command);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint source, uint target, bool attach);
    [DllImport("kernel32.dll", SetLastError = true)] public static extern bool ReadProcessMemory(IntPtr process, IntPtr address, byte[] buffer, UIntPtr size, out UIntPtr read);
}
'@

function Read-AuthenticatedBytes([IntPtr]$Handle, [long]$Address, [int]$Count, [string]$Label) {
    $buffer = [byte[]]::new($Count)
    $read = [UIntPtr]::Zero
    if (-not [Hoi4X2AutonomousPanelActions]::ReadProcessMemory(
        $Handle, [IntPtr]$Address, $buffer, [UIntPtr]::new([uint64]$Count), [ref]$read
    ) -or $read.ToUInt64() -ne [uint64]$Count) {
        throw ('Fail closed: ReadProcessMemory failed for {0} at 0x{1:X}.' -f $Label, $Address)
    }
    return $buffer
}

function Read-AuthenticatedU64([IntPtr]$Handle, [long]$Address, [string]$Label) {
    return [BitConverter]::ToUInt64((Read-AuthenticatedBytes $Handle $Address 8 $Label), 0)
}

function Read-AuthenticatedI32([IntPtr]$Handle, [long]$Address, [string]$Label) {
    return [BitConverter]::ToInt32((Read-AuthenticatedBytes $Handle $Address 4 $Label), 0)
}

$process = Get-Process -Id $ProcessId -ErrorAction Stop
$exePath = $process.MainModule.FileName
$exeSha256 = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($exeSha256 -cne $expectedExeSha256) { throw "Exact-a729 executable mismatch: $exeSha256" }
$window = $process.MainWindowHandle
if ($window -eq [IntPtr]::Zero) { throw 'Exact-a729 process has no main window.' }
$windowOwner = [uint32]0
[void][Hoi4X2AutonomousPanelActions]::GetWindowThreadProcessId($window, [ref]$windowOwner)
if ($windowOwner -ne $ProcessId) { throw 'Exact-a729 main-window owner mismatch.' }

$client = [Hoi4X2AutonomousPanelActions+RECT]::new()
if (-not [Hoi4X2AutonomousPanelActions]::GetClientRect($window, [ref]$client)) {
    throw 'GetClientRect failed for the exact-a729 window.'
}
$clientWidth = $client.Right - $client.Left
$clientHeight = $client.Bottom - $client.Top
if ($clientWidth -ne $ExpectedClientWidth -or $clientHeight -ne $ExpectedClientHeight) {
    throw "Frozen X2 action profile requires ${ExpectedClientWidth}x${ExpectedClientHeight}; observed ${clientWidth}x${clientHeight}."
}

$moduleBase = $process.MainModule.BaseAddress.ToInt64()
$handle = $process.Handle
$clockA = Read-AuthenticatedU64 $handle ($moduleBase + $clockSingletonARva) 'clock singleton A'
$clockB = Read-AuthenticatedU64 $handle ($moduleBase + $clockSingletonBRva) 'clock singleton B'
$clockCandidates = @(@($clockA, $clockB) | Where-Object { $_ -ne 0 } | Select-Object -Unique)
if ($clockCandidates.Count -ne 1) {
    throw ('Expected one exact clock singleton, got A=0x{0:X}, B=0x{1:X}.' -f $clockA, $clockB)
}
$clock = [long]$clockCandidates[0]
$clockVtable = Read-AuthenticatedU64 $handle $clock 'clock vtable'
if ($clockVtable -ne [uint64]($moduleBase + $clockVtableRva)) { throw 'Exact clock vtable mismatch.' }
$gameState = [long](Read-AuthenticatedU64 $handle ($moduleBase + $gameStateSlotRva) 'CGameState singleton')
if ($gameState -eq 0) { throw 'CGameState singleton is null.' }

function Read-PauseByte {
    return (Read-AuthenticatedBytes $handle ($clock + $pauseOffset) 1 'pause byte')[0]
}

function Read-GameHourRaw {
    return Read-AuthenticatedI32 $handle ($gameState + $gameHourOffset) 'raw Jomini DateHour'
}

function Read-GameSpeedRaw {
    return Read-AuthenticatedI32 $handle ($gameState + $gameSpeedOffset) 'raw game speed index'
}

function Assert-AuthenticatedWindow([string]$Label) {
    $current = Get-Process -Id $ProcessId -ErrorAction Stop
    if ($current.MainWindowHandle -ne $window) { throw "Main-window identity changed before $Label." }
    $owner = [uint32]0
    [void][Hoi4X2AutonomousPanelActions]::GetWindowThreadProcessId($window, [ref]$owner)
    if ($owner -ne $ProcessId) { throw "Window owner changed before $Label." }
    if ([Hoi4X2AutonomousPanelActions]::GetForegroundWindow() -ne $window) {
        throw "Exact-a729 window is not foreground before $Label."
    }
    return $owner
}

function Focus-AuthenticatedWindow {
    [void][Hoi4X2AutonomousPanelActions]::ShowWindowAsync($window, 9)
    $foregroundBefore = [Hoi4X2AutonomousPanelActions]::GetForegroundWindow()
    $foregroundOwner = [uint32]0
    $foregroundThread = if ($foregroundBefore -eq [IntPtr]::Zero) { [uint32]0 } else {
        [Hoi4X2AutonomousPanelActions]::GetWindowThreadProcessId($foregroundBefore, [ref]$foregroundOwner)
    }
    $targetOwner = [uint32]0
    $targetThread = [Hoi4X2AutonomousPanelActions]::GetWindowThreadProcessId($window, [ref]$targetOwner)
    $attached = $false
    try {
        if ($foregroundThread -ne 0 -and $targetThread -ne 0 -and $foregroundThread -ne $targetThread) {
            if (-not [Hoi4X2AutonomousPanelActions]::AttachThreadInput($foregroundThread, $targetThread, $true)) {
                throw 'AttachThreadInput rejected authenticated foreground transfer.'
            }
            $attached = $true
        }
        [void][Hoi4X2AutonomousPanelActions]::BringWindowToTop($window)
        if (-not [Hoi4X2AutonomousPanelActions]::SetForegroundWindow($window)) {
            throw 'SetForegroundWindow rejected exact-a729 foreground transfer.'
        }
    } finally {
        if ($attached) { [void][Hoi4X2AutonomousPanelActions]::AttachThreadInput($foregroundThread, $targetThread, $false) }
    }
    Start-Sleep -Milliseconds 750
    [void](Assert-AuthenticatedWindow 'machine action sequence')
}

function Move-AuthenticatedCursor([int]$X, [int]$Y, [string]$Label) {
    $owner = Assert-AuthenticatedWindow $Label
    $screen = [Hoi4X2AutonomousPanelActions+POINT]::new()
    $screen.X = $X
    $screen.Y = $Y
    if (-not [Hoi4X2AutonomousPanelActions]::ClientToScreen($window, [ref]$screen)) {
        throw "ClientToScreen failed for $Label."
    }
    if (-not [Hoi4X2AutonomousPanelActions]::SetCursorPos($screen.X, $screen.Y)) {
        throw "SetCursorPos failed for $Label."
    }
    $observed = [Hoi4X2AutonomousPanelActions+POINT]::new()
    if (-not [Hoi4X2AutonomousPanelActions]::GetCursorPos([ref]$observed) -or
        $observed.X -ne $screen.X -or $observed.Y -ne $screen.Y) {
        throw "Cursor placement was not exact for $Label."
    }
    return [ordered]@{ foreground_owner_process_id = $owner; screen_point = @($screen.X, $screen.Y) }
}

function Send-PhysicalKey([byte]$VirtualKey, [string]$Label) {
    [void](Assert-AuthenticatedWindow $Label)
    [Hoi4X2AutonomousPanelActions]::keybd_event($VirtualKey, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 80
    [Hoi4X2AutonomousPanelActions]::keybd_event($VirtualKey, 0, 0x0002, [UIntPtr]::Zero)
}

function Send-PhysicalSpace([string]$Label) {
    Send-PhysicalKey 0x20 $Label
}

function Wait-PauseState([byte]$Expected, [int]$TimeoutMilliseconds, [string]$Label) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMilliseconds)
    do {
        $observed = Read-PauseByte
        if ($observed -eq $Expected) { return }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for pause=$Expected after $Label; last observed=$observed."
}

Focus-AuthenticatedWindow
$pausedBefore = (Read-PauseByte) -eq 1
if (-not $pausedBefore) { throw 'Autonomous X2 actions require an initially paused exact-a729 save.' }
$hourBeforeAll = Read-GameHourRaw
$speedBefore = Read-GameSpeedRaw
if ($speedBefore -ne 4) {
    Send-PhysicalKey 0x35 'select exact game speed 5 before X2 capture'
    $speedDeadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        Start-Sleep -Milliseconds 50
        $speedBefore = Read-GameSpeedRaw
    } while ($speedBefore -ne 4 -and [DateTime]::UtcNow -lt $speedDeadline)
    if ($speedBefore -ne 4) { throw 'Physical speed-5 action did not produce authenticated game speed raw 4.' }
}

# Prepare the same panel/camera state before Start-Hoi4ProductionCaptureSafe's
# strict speed-5 preflight. This click is navigation only; hooks are not yet
# assumed present and it is never admitted as panel evidence.
$preparePoint = $candidatePoints[0]
$prepareMove = Move-AuthenticatedCursor $preparePoint[0] $preparePoint[1] 'prepare X2 active-battle panel'
[Hoi4X2AutonomousPanelActions]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 80
[Hoi4X2AutonomousPanelActions]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 500
Send-PhysicalSpace 'unpause exact speed-5 X2 capture preflight'
Wait-PauseState 0 5000 'unpause exact speed-5 X2 capture preflight'
if ((Read-GameSpeedRaw) -ne 4) { throw 'X2 capture preparation lost exact game speed raw 4.' }
Write-AtomicCreateNewUtf8Json $ReadyPath ([ordered]@{
    schema = 'hoi4.x2.autonomous_combat_panel_ready.v1'
    status = 'READY_SPEED5_UNPAUSED'
    process_id = $ProcessId
    session_id = $SessionId
    capture_id = $CaptureId
    executable_sha256 = $exeSha256
    selection_profile_sha256 = $profileSha256
    game_hour_raw = Read-GameHourRaw
    game_speed_raw = Read-GameSpeedRaw
    paused = $false
    window_owner_process_id = $windowOwner
    foreground_owner_process_id = $prepareMove.foreground_owner_process_id
})

$collector = $null
$collectorDeadline = [DateTime]::UtcNow.AddSeconds($CollectorLoadTimeoutSeconds)
while ([DateTime]::UtcNow -lt $collectorDeadline -and $null -eq $collector) {
    $current = Get-Process -Id $ProcessId -ErrorAction Stop
    $matches = @($current.Modules | Where-Object { [IO.Path]::GetFileName([string]$_.FileName) -ieq 'collector.dll' })
    if ($matches.Count -gt 1) { throw 'More than one collector.dll is loaded in the exact-a729 process.' }
    if ($matches.Count -eq 1) {
        $candidateHash = (Get-FileHash -LiteralPath $matches[0].FileName -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($candidateHash -cne $ExpectedCollectorSha256) {
            throw "Loaded collector.dll differs from the centrally attested hash: $candidateHash"
        }
        $collector = $matches[0]
        break
    }
    Start-Sleep -Milliseconds 250
}
if ($null -eq $collector) { throw 'Timed out waiting for the authenticated collector to load.' }

$actions = [Collections.Generic.List[object]]::new()
$selectedHoldStartHour = Read-GameHourRaw

for ($ordinal = 0; $ordinal -lt $candidatePoints.Count; $ordinal++) {
    $point = $candidatePoints[$ordinal]
    if ((Read-PauseByte) -ne 0 -or (Read-GameSpeedRaw) -ne 4) {
        throw "Game was not exact speed-5 unpaused before X2 candidate $ordinal."
    }
    if ($ordinal -gt 0) {
        Send-PhysicalKey 0x1B "close prior X2 candidate panel $ordinal"
        Start-Sleep -Milliseconds 250
    }
    $hourBefore = Read-GameHourRaw
    $move = Move-AuthenticatedCursor $point[0] $point[1] "X2 battle-bubble candidate $ordinal"
    $qpcBefore = [Diagnostics.Stopwatch]::GetTimestamp()
    [Hoi4X2AutonomousPanelActions]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 80
    [Hoi4X2AutonomousPanelActions]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds $PanelMaterializationDwellMilliseconds

    $hourAfter = Read-GameHourRaw
    $qpcAfter = [Diagnostics.Stopwatch]::GetTimestamp()
    $actions.Add([ordered]@{
        ordinal = $ordinal
        client_point = @([int]$point[0], [int]$point[1])
        screen_point = $move.screen_point
        method = 'physical_setcursorpos_mouse_event'
        foreground_owner_process_id = $move.foreground_owner_process_id
        window_owner_process_id = $windowOwner
        cursor_confirmed = $true
        click_hold_milliseconds = 80
        materialization_dwell_milliseconds = $PanelMaterializationDwellMilliseconds
        required_hold_game_hours = $GameHoursToHold
        game_hour_raw_before = $hourBefore
        game_hour_raw_after = $hourAfter
        qpc_ticks_before = $qpcBefore
        qpc_ticks_after = $qpcAfter
    })
}

$holdDeadline = [DateTime]::UtcNow.AddSeconds($GameHourAdvanceTimeoutSeconds)
while ((Read-GameHourRaw) -lt ($selectedHoldStartHour + $GameHoursToHold) -and [DateTime]::UtcNow -lt $holdDeadline) {
    Start-Sleep -Milliseconds 50
}
if ((Read-GameHourRaw) -lt ($selectedHoldStartHour + $GameHoursToHold)) {
    throw "X2 candidate sweep was not retained across $GameHoursToHold authoritative game hours."
}

$captureDeadline = [DateTime]::UtcNow.AddSeconds($CaptureFinalizeTimeoutSeconds)
while (-not (Test-Path -LiteralPath $CaptureManifestPath -PathType Leaf) -and [DateTime]::UtcNow -lt $captureDeadline) {
    if ((Read-PauseByte) -ne 0 -or (Read-GameSpeedRaw) -ne 4) {
        throw 'Exact speed-5 unpaused state changed while holding the selected X2 panel.'
    }
    Start-Sleep -Milliseconds 250
}
if (-not (Test-Path -LiteralPath $CaptureManifestPath -PathType Leaf)) {
    throw 'Timed out waiting for the same-capture native manifest while holding the selected X2 panel.'
}
Send-PhysicalSpace 'repause after finalized X2 capture manifest'
Wait-PauseState 1 5000 'repause after finalized X2 capture manifest'
$actions[$actions.Count - 1].qpc_ticks_after = [Diagnostics.Stopwatch]::GetTimestamp()
$actions[$actions.Count - 1].game_hour_raw_after = Read-GameHourRaw
$pausedAfter = (Read-PauseByte) -eq 1
$hourAfterAll = Read-GameHourRaw
if (-not $pausedAfter -or $hourAfterAll -lt $actions[$actions.Count - 1].game_hour_raw_after) {
    throw 'X2 action sequence did not finish in an authenticated paused state.'
}
$canonicalActions = ($actions | ForEach-Object {
    '{0}|{1}|{2}|{3}|{4}|{5}|{6}|{7}|{8}|{9}|{10}' -f `
        $_.ordinal, $_.client_point[0], $_.client_point[1], $_.screen_point[0], $_.screen_point[1], `
        $_.game_hour_raw_before, $_.game_hour_raw_after, $_.method, $_.click_hold_milliseconds, `
        $_.materialization_dwell_milliseconds, $_.required_hold_game_hours
}) -join ';'
$sha = [Security.Cryptography.SHA256]::Create()
try {
    $actionSequenceSha256 = ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($canonicalActions)))).Replace('-', '').ToLowerInvariant()
} finally { $sha.Dispose() }

$receipt = [ordered]@{
    schema = 'hoi4.x2.autonomous_combat_panel_actions.v1'
    status = 'PASS'
    acquisition_owner = 'machine'
    timestamp_utc = [DateTime]::UtcNow.ToString('o')
    process_id = $ProcessId
    process_start_utc = $process.StartTime.ToUniversalTime().ToString('o')
    session_id = $SessionId
    capture_id = $CaptureId
    executable_sha256 = $exeSha256
    collector_loaded = $true
    collector_sha256 = $ExpectedCollectorSha256
    central_release_attestation_sha256 = $PrebuiltAttestationSha256
    window_owner_process_id = $windowOwner
    client_size = @($clientWidth, $clientHeight)
    selection_profile = $selectionProfile
    selection_profile_sha256 = $profileSha256
    paused_before = $pausedBefore
    paused_after = $pausedAfter
    game_hour_raw_before = $hourBeforeAll
    game_hour_raw_after = $hourAfterAll
    actions = @($actions)
    action_sequence_sha256 = $actionSequenceSha256
    postcondition = 'SELECTED_PANEL_HELD_ACROSS_AUTHORITATIVE_GAME_HOUR; FOLDED_BOTH_SIDE_MATERIALIZATION_REQUIRED'
}
Write-AtomicCreateNewUtf8Json $ReceiptPath $receipt
Write-Output "Autonomous X2 battle-panel actions complete: $ReceiptPath"
