[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [uint32] $GamePid,
    [Parameter(Mandatory = $true)] [string] $ExpectedProcessStartUtc,
    [Parameter(Mandatory = $true)] [string] $ProfilePath,
    [Parameter(Mandatory = $true)] [string] $ActionDirectory,
    [Parameter(Mandatory = $true)] [string] $CaptureReadyFile,
    [Parameter(Mandatory = $true)] [string] $ReceiptPath,
    [int] $HandshakeTimeoutSeconds = 300
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot '..\..\scripts\Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$gameStateRva = 0x3316E90L
$clockRvaA = 0x33172E0L
$clockRvaB = 0x33172E8L
$clockVtableRva = 0x2952510L
$gameHourOffset = 0x468L
$primaryCountryOffset = 0x520L
$fallbackCountryOffset = 0x524L
$pauseOffset = 0x6C1L

function Write-CreateNewUtf8Json([string] $Path, $Value) {
    $full = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($full)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllText($temporary, (($Value | ConvertTo-Json -Depth 30) + "`n"), [Text.UTF8Encoding]::new($false))
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($full) -or [IO.Directory]::Exists($full)) { throw "Refusing to replace immutable A27 artifact: $full" }
        [IO.File]::Move($temporary, $full)
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

function Wait-NewFile([string] $Path, [string] $Label) {
    $deadline = [DateTime]::UtcNow.AddSeconds($HandshakeTimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ([IO.File]::Exists($Path)) { return }
        Start-Sleep -Milliseconds 20
    }
    throw "Timed out waiting for A27 $Label at $Path"
}

function Wait-SealedTimelineHour([string] $TimelinePath, [int] $Hour, [string] $Label) {
    $deadline = [DateTime]::UtcNow.AddSeconds($HandshakeTimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        try {
            $value = Get-Content -LiteralPath $TimelinePath -Raw | ConvertFrom-Json
            if ($value.captures.Count -gt 0) {
                $last = $value.captures[-1]
                if ([int]$last.capture_hour_index -eq $Hour -and [int]$last.same_hour_rechecks -ge 1) { return $value }
            }
        } catch { }
        Start-Sleep -Milliseconds 20
    }
    throw "Timed out waiting for A27 sealed timeline hour $Hour after $Label."
}

Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class A27AuthenticatedInput {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public INPUTUNION U; }
    [StructLayout(LayoutKind.Explicit)] public struct INPUTUNION { [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
    [DllImport("kernel32.dll", SetLastError=true)] public static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool ReadProcessMemory(IntPtr p, IntPtr a, byte[] b, UIntPtr n, out UIntPtr read);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool WriteProcessMemory(IntPtr p, IntPtr a, byte[] b, UIntPtr n, out UIntPtr written);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr h, int command);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT rect);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT point);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint x, uint y, uint data, UIntPtr extra);
    [DllImport("user32.dll", SetLastError=true)] public static extern uint SendInput(uint count, INPUT[] input, int size);
    public static void Key(ushort vk) {
        INPUT[] values = new INPUT[2]; values[0].type=1; values[0].U.ki.vk=vk;
        values[1].type=1; values[1].U.ki.vk=vk; values[1].U.ki.flags=2;
        if (SendInput(2, values, Marshal.SizeOf(typeof(INPUT))) != 2) throw new System.ComponentModel.Win32Exception();
    }
}
'@

$profileFull = (Resolve-Path -LiteralPath $ProfilePath).Path
$profile = Get-Content -LiteralPath $profileFull -Raw | ConvertFrom-Json
if ($profile.schema -cne 'extractor.a27.authenticated_designer_transactions_profile.v1' -or
    [string]$profile.expected_target_stable_id -cne '371:52' -or
    -not ($profile.transactions -is [array]) -or $profile.transactions.Count -ne 4) {
    throw 'A27 transaction profile schema/count is invalid.'
}
$requiredOrder = @('rename', 'edit', 'obsolete', 'reactivate')
for ($index = 0; $index -lt $requiredOrder.Count; $index++) {
    if ([string]$profile.transactions[$index].transaction -cne $requiredOrder[$index] -or
        -not ($profile.transactions[$index].steps -is [array]) -or
        $profile.transactions[$index].steps.Count -lt 1) {
        throw "A27 profile transaction $index is not the required nonempty $($requiredOrder[$index]) action."
    }
}
$profileSha256 = (Get-FileHash -LiteralPath $profileFull -Algorithm SHA256).Hash.ToLowerInvariant()
$actionRoot = [IO.Path]::GetFullPath($ActionDirectory)
if ([IO.Directory]::Exists($actionRoot) -or [IO.File]::Exists($actionRoot)) { throw "A27 ActionDirectory must be new: $actionRoot" }
if ([IO.File]::Exists([IO.Path]::GetFullPath($ReceiptPath))) { throw 'A27 action receipt path must be new.' }
[IO.Directory]::CreateDirectory($actionRoot) | Out-Null

$process = Get-Process -Id $GamePid -ErrorAction Stop
$observedStartUtc = $process.StartTime.ToUniversalTime().ToString('o')
if ([DateTime]::Parse($observedStartUtc).ToUniversalTime() -ne [DateTime]::Parse($ExpectedProcessStartUtc).ToUniversalTime()) { throw 'A27 PID/start identity changed.' }
$exePath = $process.MainModule.FileName
$exeSha256 = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($exeSha256 -cne $expectedExeSha256) { throw "A27 executable SHA-256 mismatch: $exeSha256" }
$gameRoot = [IO.Path]::GetDirectoryName($exePath)
$coordinateSources = [Collections.Generic.List[object]]::new()
foreach ($source in $profile.coordinate_sources) {
    $relative = ([string]$source.relpath).Replace('/', [IO.Path]::DirectorySeparatorChar)
    if ([IO.Path]::IsPathRooted($relative) -or $relative.Contains('..')) { throw 'A27 coordinate source relpath is not portable.' }
    $sourcePath = [IO.Path]::Combine($gameRoot, $relative)
    $observedSha = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($observedSha -cne [string]$source.sha256) { throw "A27 coordinate source drift: $relative" }
    $coordinateSources.Add([ordered]@{ relpath = ([string]$source.relpath); sha256 = $observedSha })
}
$moduleBase = $process.MainModule.BaseAddress.ToInt64()
$window = $process.MainWindowHandle
if ($window -eq [IntPtr]::Zero) { throw 'A27 process has no main window.' }
$owner = [uint32]0
[void][A27AuthenticatedInput]::GetWindowThreadProcessId($window, [ref]$owner)
if ($owner -ne $GamePid) { throw 'A27 main-window owner mismatch.' }
$client = [A27AuthenticatedInput+RECT]::new()
if (-not [A27AuthenticatedInput]::GetClientRect($window, [ref]$client)) { throw 'A27 GetClientRect failed.' }
$width = $client.Right - $client.Left
$height = $client.Bottom - $client.Top
if ($width -ne [int]$profile.expected_client_width -or $height -ne [int]$profile.expected_client_height) { throw "A27 profile/client dimensions differ: ${width}x${height}." }
$handle = [A27AuthenticatedInput]::OpenProcess(0x1038, $false, $GamePid)
if ($handle -eq [IntPtr]::Zero) { throw 'A27 read/pause-only process handle failed.' }

function Read-Bytes([long] $Address, [int] $Length) {
    $bytes = [byte[]]::new($Length); $read = [UIntPtr]::Zero
    if (-not [A27AuthenticatedInput]::ReadProcessMemory($handle, [IntPtr]$Address, $bytes, [UIntPtr]::new([uint64]$Length), [ref]$read) -or $read.ToUInt64() -ne [uint64]$Length) { throw ('A27 ReadProcessMemory failed at 0x{0:X}.' -f $Address) }
    return $bytes
}
function Read-U64([long] $Address) { [BitConverter]::ToUInt64((Read-Bytes $Address 8), 0) }
function Read-I32([long] $Address) { [BitConverter]::ToInt32((Read-Bytes $Address 4), 0) }
function Read-U8([long] $Address) { (Read-Bytes $Address 1)[0] }
function Write-U8([long] $Address, [byte] $Value) {
    $written = [UIntPtr]::Zero; $bytes = [byte[]]@($Value)
    if (-not [A27AuthenticatedInput]::WriteProcessMemory($handle, [IntPtr]$Address, $bytes, [UIntPtr]::new(1), [ref]$written) -or $written.ToUInt64() -ne 1) { throw 'A27 pause-byte write failed.' }
}
function Read-ClockState {
    $state = [long](Read-U64 ($moduleBase + $gameStateRva)); $clockA = [long](Read-U64 ($moduleBase + $clockRvaA)); $clockB = [long](Read-U64 ($moduleBase + $clockRvaB))
    if ($state -eq 0 -or $clockA -eq 0 -or $clockA -ne $clockB -or (Read-U64 $clockA) -ne [uint64]($moduleBase + $clockVtableRva)) { throw 'A27 game-state/clock identity is invalid.' }
    $primary = Read-I32 ($state + $primaryCountryOffset); $fallback = Read-I32 ($state + $fallbackCountryOffset)
    [ordered]@{ state = $state; clock = $clockA; hour = Read-I32 ($state + $gameHourOffset); paused = (Read-U8 ($clockA + $pauseOffset)); effective_country_handle = $(if ($primary -gt 0) { $primary } else { $fallback }) }
}
function Assert-Foreground {
    if ([A27AuthenticatedInput]::GetForegroundWindow() -ne $window) { [void][A27AuthenticatedInput]::ShowWindowAsync($window, 9); [void][A27AuthenticatedInput]::SetForegroundWindow($window); Start-Sleep -Milliseconds 400 }
    if ([A27AuthenticatedInput]::GetForegroundWindow() -ne $window) { throw 'A27 exact window is not foreground.' }
}
function Invoke-Click([int] $X, [int] $Y) {
    Assert-Foreground
    if ($X -lt 0 -or $Y -lt 0 -or $X -ge $width -or $Y -ge $height) { throw "A27 click is outside the authenticated client: $X,$Y" }
    $point = [A27AuthenticatedInput+POINT]::new(); $point.X = $X; $point.Y = $Y
    if (-not [A27AuthenticatedInput]::ClientToScreen($window, [ref]$point) -or -not [A27AuthenticatedInput]::SetCursorPos($point.X, $point.Y)) { throw 'A27 authenticated cursor movement failed.' }
    [A27AuthenticatedInput]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero); [A27AuthenticatedInput]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
}
function Invoke-ConsoleCommand([string] $Command, [int] $SettleMilliseconds) {
    Assert-Foreground; [A27AuthenticatedInput]::Key(0xC0); Start-Sleep -Milliseconds 300
    [System.Windows.Forms.SendKeys]::SendWait($Command); [A27AuthenticatedInput]::Key(0x0D); Start-Sleep -Milliseconds $SettleMilliseconds
    [A27AuthenticatedInput]::Key(0xC0); Start-Sleep -Milliseconds 300
}
function Invoke-ProfileSteps($Steps) {
    $observed = [Collections.Generic.List[object]]::new()
    foreach ($step in $Steps) {
        $kind = [string]$step.kind; $entry = [ordered]@{ label = [string]$step.label; kind = $kind; started_utc = [DateTime]::UtcNow.ToString('o') }
        switch ($kind) {
            'click' { Invoke-Click ([int]$step.x) ([int]$step.y); Start-Sleep -Milliseconds ([int]$step.settle_milliseconds); $entry.client_point = @([int]$step.x, [int]$step.y) }
            'key' { Assert-Foreground; [A27AuthenticatedInput]::Key([Convert]::ToUInt16([string]$step.virtual_key, 16)); Start-Sleep -Milliseconds ([int]$step.settle_milliseconds); $entry.virtual_key = [string]$step.virtual_key }
            'text' { Assert-Foreground; [System.Windows.Forms.SendKeys]::SendWait([string]$step.text); Start-Sleep -Milliseconds ([int]$step.settle_milliseconds); $entry.text_sha256 = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes([string]$step.text))).ToLowerInvariant() }
            'console_command' { Invoke-ConsoleCommand ([string]$step.command) ([int]$step.settle_milliseconds); $entry.command = [string]$step.command }
            'wait' { Start-Sleep -Milliseconds ([int]$step.milliseconds) }
            default { throw "Unsupported A27 action kind: $kind" }
        }
        $entry.completed_utc = [DateTime]::UtcNow.ToString('o'); $observed.Add($entry)
    }
    return @($observed)
}
function Advance-ExactlyOneHour([int] $FromHour) {
    $clock = Read-ClockState
    if ($clock.hour -ne $FromHour -or $clock.paused -ne 1) { throw 'A27 action did not preserve its paused baseline hour.' }
    $pauseAddress = $clock.clock + $pauseOffset; Write-U8 $pauseAddress 0
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    try {
        while ((Read-I32 ($clock.state + $gameHourOffset)) -lt ($FromHour + 1)) {
            if ([DateTime]::UtcNow -ge $deadline) { throw 'A27 clock did not reach the next hour.' }
            Start-Sleep -Milliseconds 2
        }
    } finally { Write-U8 $pauseAddress 1 }
    $after = Read-ClockState
    if ($after.hour -ne ($FromHour + 1) -or $after.paused -ne 1) { throw 'A27 action crossed more than one hour or failed to repause.' }
    return $after.hour
}

$transactions = [Collections.Generic.List[object]]::new()
try {
    Wait-NewFile ([IO.Path]::GetFullPath($CaptureReadyFile)) 'initial census readiness'
    Assert-Foreground
    $timelinePath = Join-Path ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($CaptureReadyFile))) 'a27_live_catalog_timeline.source.json'
    $initialState = Read-ClockState
    if ($initialState.hour -ne [int]$profile.authority.initial_game_hour_raw -or
        $initialState.effective_country_handle -ne [int]$profile.authority.expected_country_handle -or
        $initialState.paused -ne 1) {
        throw 'A27 process is not the authenticated paused New-D04 fixture state.'
    }
    foreach ($index in 0..3) {
        $transaction = $profile.transactions[$index]; $clock = Read-ClockState
        if ($clock.paused -ne 1 -or $clock.effective_country_handle -ne [int]$profile.authority.expected_country_handle) { throw "A27 $($transaction.transaction) requires the paused authenticated country baseline." }
        $timeline = Wait-SealedTimelineHour $timelinePath $clock.hour "prior transaction"
        $baseline = $timeline.captures[-1]
        if ([int]$baseline.capture_hour_index -ne $clock.hour) { throw 'A27 capture baseline and game clock differ before action request.' }
        $requestPath = Join-Path $actionRoot ('request_{0:D2}.json' -f $index)
        Write-CreateNewUtf8Json $requestPath ([ordered]@{ schema = 'extractor.a27.authenticated_transaction_request.v1'; sequence = $index; transaction = [string]$transaction.transaction; from_hour = $clock.hour; baseline_snapshot_sha256 = [string]$baseline.snapshot_sha256; profile_sha256 = $profileSha256 })
        $readyPath = Join-Path $actionRoot ('ready_{0:D2}.json' -f $index); Wait-NewFile $readyPath "$($transaction.transaction) census suspension"
        $ready = Get-Content -LiteralPath $readyPath -Raw | ConvertFrom-Json
        if ($ready.transaction -cne [string]$transaction.transaction -or [int]$ready.from_hour -ne $clock.hour) { throw 'A27 action-ready identity is invalid.' }
        $steps = Invoke-ProfileSteps $transaction.steps
        $toHour = Advance-ExactlyOneHour $clock.hour
        $releasePath = Join-Path $actionRoot ('release_{0:D2}.json' -f $index)
        $readySha = (Get-FileHash -LiteralPath $readyPath -Algorithm SHA256).Hash.ToLowerInvariant()
        Write-CreateNewUtf8Json $releasePath ([ordered]@{ schema = 'extractor.a27.authenticated_transaction_release.v1'; sequence = $index; transaction = [string]$transaction.transaction; from_hour = $clock.hour; to_hour = $toHour; profile_sha256 = $profileSha256; ready_sha256 = $readySha })
        $resumedPath = Join-Path $actionRoot ('resumed_{0:D2}.json' -f $index); Wait-NewFile $resumedPath "$($transaction.transaction) census resume"
        $resumed = Get-Content -LiteralPath $resumedPath -Raw | ConvertFrom-Json
        [void](Wait-SealedTimelineHour $timelinePath $toHour $transaction.transaction)
        $transactions.Add([ordered]@{ sequence = $index; transaction = [string]$transaction.transaction; from_hour = $clock.hour; to_hour = $toHour; profile_sha256 = $profileSha256; request_sha256 = (Get-FileHash -LiteralPath $requestPath -Algorithm SHA256).Hash.ToLowerInvariant(); ready_sha256 = $readySha; release_sha256 = (Get-FileHash -LiteralPath $releasePath -Algorithm SHA256).Hash.ToLowerInvariant(); resumed_sha256 = (Get-FileHash -LiteralPath $resumedPath -Algorithm SHA256).Hash.ToLowerInvariant(); steps = $steps })
    }
    $receipt = [ordered]@{ schema = 'extractor.a27.authenticated_designer_transaction_receipt.v1'; status = 'complete'; executable_path = $exePath; executable_sha256 = $exeSha256; process_id = $GamePid; process_start_utc = $observedStartUtc; profile_path = $profileFull; profile_sha256 = $profileSha256; expected_target_stable_id = [string]$profile.expected_target_stable_id; coordinate_sources = @($coordinateSources); client_size = @($width, $height); transaction_order = $requiredOrder; transactions = @($transactions); semantic_authority = 'A27 sealed native-row transition regrade only'; promotion_authorized = $false }
    Write-CreateNewUtf8Json $ReceiptPath $receipt
} finally {
    if ($handle -ne [IntPtr]::Zero) { [void][A27AuthenticatedInput]::CloseHandle($handle) }
}
