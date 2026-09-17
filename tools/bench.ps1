<#
.SYNOPSIS
Automated same-save throughput benchmark loop for recorder candidates.
.DESCRIPTION
Keeps one game open and, for every candidate in the plan: attaches the frozen
recorder (if any), warms every consistency window to 200/0, reloads the
benchmark save through the in-game Load menu, runs the game at native speed 4
(displayed 5) for the planned seconds while sampling the game clock from
outside the process, pauses, detaches, and records hours per second against the
retained free-run baseline. A candidate with "frozen": null is a no-recorder
baseline run. Runs are interleaved per repeat so drift affects every candidate
alike. Timed windows refuse to start while build processes run and are marked
contaminated if one appears mid-window.

Plan JSON:
{
  "save": "perf_takeover_baseline.hoi4",
  "expected_hour": 60826314, "expected_seed": 327447340,
  "seconds": 30, "repeats": 1, "baseline_hours_per_s": 27.3873015897,
  "candidates": [
    { "name": "baseline", "frozen": null },
    { "name": "notif482f", "frozen": "C:\\path\\to\\bin", "warm": true }
  ]
}
"frozen" is a directory holding recorder.exe and collector.dll. "warm" defaults
to true for frozen candidates; false measures with the 200-check windows still
running (only comparable against other unwarmed runs).
A third candidate kind injects the synthetic cost-ladder DLL instead of the
recorder: { "name": "spin4", "ladder": { "mode": "spin_ms", "param_ms": 4 } }
(modes and parameters as documented in the ladder README; no warm-up; the DLL's
own per-hour timings are folded into the results).
.EXAMPLE
powershell -File tools/bench.ps1 -Plan captures\perf_bench_20260917\plan.json -Out captures\perf_bench_20260917\run01
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Plan,
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$GameExe = 'X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe',
    [string]$UserDirectory = 'captures\m11_20260909\userdir',
    [string]$UiConfig = '',
    [string]$MetricsTool = 'captures\performance_takeover_20260916\recperf2-metrics.exe',
    [string]$LadderDir = 'captures\perf_bench_20260917\builds\ladder',
    [int]$WarmTimeoutSeconds = 900,
    [int]$AttachTimeoutSeconds = 300,
    # Default: keys posted, clicks borrow the foreground briefly (the UI needs real clicks),
    # PrintWindow screenshots, game pushed behind the previous window afterwards.
    [switch]$Foreground,
    [switch]$Background,  # compatibility no-op
    [switch]$PostedClicks,  # experimental: posted mouse messages instead of the brief foreground borrow
    # menu = in-game Load menu (needed while a recorder stays attached); relaunch = -start_save,
    # no clicks, never in front; auto picks relaunch when nothing is attached.
    [ValidateSet('menu', 'relaunch', 'auto')][string]$ReloadMode = 'auto'
)

Set-StrictMode -Version 1
$ErrorActionPreference = 'Stop'
if (-not [Environment]::Is64BitProcess) { throw 'Run bench.ps1 in 64-bit PowerShell.' }
$Background = -not $Foreground

$captureRoot = 'captures\'
$Out = [IO.Path]::GetFullPath($Out)
if (-not $Out.StartsWith($captureRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Out must be below captures.' }
$null = New-Item -ItemType Directory -Force -Path $Out
$utf8 = New-Object Text.UTF8Encoding $false
$spec = Get-Content -LiteralPath $Plan -Raw | ConvertFrom-Json
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $UiConfig) { $UiConfig = Join-Path $scriptDir 'bench_ui.json' }
$ui = Get-Content -LiteralPath $UiConfig -Raw | ConvertFrom-Json
$smoke = Join-Path $scriptDir 'smoke.ps1'
$script:menuPatchPath = 'captures\perf_bench_20260917\calib\menu_patch.png'
$script:chainStarted = $false
$userPath = (Resolve-Path -LiteralPath $UserDirectory).ProviderPath
$savePath = Join-Path (Join-Path $userPath 'save games') ([string]$spec.save)
if (-not (Test-Path -LiteralPath $savePath -PathType Leaf)) { throw "Benchmark save missing: $savePath" }
$saveSha = (Get-FileHash -LiteralPath $savePath -Algorithm SHA256).Hash
$expectedHour = [int]$spec.expected_hour
$expectedSeed = [int]$spec.expected_seed
$seconds = if ($spec.seconds) { [int]$spec.seconds } else { 30 }
$maxHours = $(if ($spec.PSObject.Properties['max_hours'] -and $spec.max_hours) { [int64]$spec.max_hours } else { 420 })
$repeats = if ($spec.repeats) { [int]$spec.repeats } else { 1 }
$baselineRate = if ($spec.baseline_hours_per_s) { [double]$spec.baseline_hours_per_s } else { 27.3873015897 }
$buildNames = @('cargo', 'rustc', 'lld-link', 'clippy-driver', 'link', 'cl')
$lock = Join-Path $env:TEMP 'claude\hoi4_game.lock'
if (-not (Test-Path -LiteralPath (Join-Path $lock 'owner'))) { throw "Take the game lock first: $lock" }

# Reuse the smoke driver's read-only clock reader and guarded input helper.
$driverSource = Get-Content -LiteralPath $smoke -Encoding UTF8 -Raw
foreach ($definition in [regex]::Matches($driverSource, "(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@")) {
    $source = $definition.Groups[1].Value
    if ($source -match 'class Hoi4ClockBaseline' -and -not ('Hoi4ClockBaseline' -as [type])) { Add-Type -TypeDefinition $source }
    if ($source -match 'class Hoi4ProbeInput' -and -not ('Hoi4ProbeInput' -as [type])) { Add-Type -TypeDefinition $source }
}
if (-not ('Hoi4ClockBaseline' -as [type]) -or -not ('Hoi4ProbeInput' -as [type])) { throw 'Could not load the smoke driver helper classes.' }
[void][Hoi4ProbeInput]::SetProcessDPIAware()

# Input shim: while attached (smoke Launch injects it), the game's user32 import thunks answer from this
# mapping so a posted click lands with the cursor, button state and focus the UI polls, with no foreground
# borrow. Absent mapping -> the click falls back to the brief borrow. See C:\hoi4_scratch\input_shim_20260917.
$script:ShimMapping = 'Local\hoi4_inputshim_v1'
$script:ShimMagic = [uint32]0x4D485348
function Open-InputShim {
    try { $map = [IO.MemoryMappedFiles.MemoryMappedFile]::OpenExisting($script:ShimMapping) } catch { return $null }
    $view = $map.CreateViewAccessor(0, 4096)
    if ($view.ReadUInt32(0) -ne $script:ShimMagic) { $view.Dispose(); $map.Dispose(); return $null }
    [pscustomobject]@{ Map = $map; View = $view }
}
function Close-InputShim($shim) { if ($shim) { $shim.View.Dispose(); $shim.Map.Dispose() } }
function Invoke-ShimClick($shim, [int]$screenX, [int]$screenY) {
    # flags: bit0 cursor override, bit1 focus override, bit2 left down. Offsets 12 flags, 16/20 cursor, 24 hwnd.
    $origin = New-Object Hoi4ProbeInput+Point
    if (-not [Hoi4ProbeInput]::ClientToScreen($script:window, [ref]$origin)) { throw 'Cannot locate the game client area.' }
    $logical = [Hoi4ProbeInput]::LogicalPoint($script:window, $screenX - $origin.X, $screenY - $origin.Y)
    $virtual = [Hoi4ProbeInput]::LogicalPoint($script:window, $screenX, $screenY)
    $shim.View.Write(24, [uint64]$script:window.ToInt64())
    $shim.View.Write(16, [int]$virtual[0]); $shim.View.Write(20, [int]$virtual[1])
    $shim.View.Write(12, [uint32]3)
    try {
        [Hoi4ProbeInput]::PostMove($script:window, [int]$logical[0], [int]$logical[1]); Start-Sleep -Milliseconds 60
        $shim.View.Write(12, [uint32]7); [Hoi4ProbeInput]::PostButton($script:window, 0x0201, 1, [int]$logical[0], [int]$logical[1]); Start-Sleep -Milliseconds 60
        $shim.View.Write(12, [uint32]3); [Hoi4ProbeInput]::PostButton($script:window, 0x0202, 0, [int]$logical[0], [int]$logical[1]); Start-Sleep -Milliseconds 60
    } finally { $shim.View.Write(12, [uint32]0) }
}

function Write-Json([string]$path, $value) { [IO.File]::WriteAllText($path, ($value | ConvertTo-Json -Depth 20), $utf8) }
function Log([string]$text) { Write-Host ("[{0}] {1}" -f (Get-Date -Format 'HH:mm:ss'), $text) }

# ---- game process, window, clock -------------------------------------------
$script:game = $null; $script:window = [IntPtr]::Zero; $script:reader = $null; $script:rect = $null
function Connect-Game {
    $games = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
    if ($games.Count -gt 1) { throw 'Two hoi4.exe processes; close one.' }
    if ($games.Count -eq 0) {
        Log 'No game running; launching the continue selection.'
        $launch = & $smoke -Action Launch -GameExe $GameExe -UserDirectory $userPath -ContinueLastSave -Foreground:(-not $Background)
        $limit = [DateTime]::UtcNow.AddSeconds(300)
        do {
            Start-Sleep -Seconds 2
            $games = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
            if ([DateTime]::UtcNow -gt $limit) { throw 'Game did not start.' }
        } while ($games.Count -ne 1 -or $games[0].MainWindowHandle -eq [IntPtr]::Zero)
    }
    $script:game = $games[0]
    if ($script:game.Path -ine $GameExe) { throw "Running game is not the pinned executable: $($script:game.Path)" }
    # The window appears some seconds after launch; wait for it and for a readable rectangle.
    $limit = [DateTime]::UtcNow.AddSeconds(300); $r = New-Object Hoi4ProbeInput+Rect
    do {
        $script:game.Refresh(); if ($script:game.HasExited) { throw 'The game exited during startup.' }
        $script:window = $script:game.MainWindowHandle
        $ok = $script:window -ne [IntPtr]::Zero -and [Hoi4ProbeInput]::GetWindowRect($script:window, [ref]$r) -and ($r.Right - $r.Left) -gt 0
        if (-not $ok) { Start-Sleep -Milliseconds 500 }
        if ([DateTime]::UtcNow -gt $limit) { throw 'The game window did not appear.' }
    } while (-not $ok)
    $script:reader = [Hoi4ClockBaseline]::new([uint32]$script:game.Id, [uint64]$script:game.MainModule.BaseAddress.ToInt64())
    $script:rect = $r
    $w = $r.Right - $r.Left; $h = $r.Bottom - $r.Top
    if ($w -ne [int]$ui.width -or $h -ne [int]$ui.height) { throw "Game window is ${w}x${h}; UI calibration expects $($ui.width)x$($ui.height)." }
}
function Read-Clock([int]$timeoutSeconds = 10) {
    $limit = [DateTime]::UtcNow.AddSeconds($timeoutSeconds)
    do {
        $script:game.Refresh(); if ($script:game.HasExited) { throw 'The game exited.' }
        $clock = $script:reader.Sample()
        if ($null -ne $clock -and $clock.paused -le 1 -and $clock.tick_busy -eq 0) { return $clock }
        Start-Sleep -Milliseconds 10
    } while ([DateTime]::UtcNow -lt $limit)
    throw 'No stable authenticated game clock.'
}
function Invoke-Focused([scriptblock]$block) {
    if ([Hoi4ProbeInput]::IsIconic($script:window)) { [void][Hoi4ProbeInput]::ShowWindow($script:window, 9) }
    $previous = [Hoi4ProbeInput]::AcquireTargetFocus($script:window, [uint32]$script:game.Id)
    try { Start-Sleep -Milliseconds 150; & $block } finally { [Hoi4ProbeInput]::RestoreForeground($previous, $script:window) }
}
function Get-ClientPoint($point) {
    # Outer-window coordinates (the calibration) to client coordinates for posted messages.
    $client = New-Object Hoi4ProbeInput+Rect; $origin = New-Object Hoi4ProbeInput+Point
    if (-not [Hoi4ProbeInput]::GetClientRect($script:window, [ref]$client) -or -not [Hoi4ProbeInput]::ClientToScreen($script:window, [ref]$origin)) { throw 'Cannot locate the game client area.' }
    @(($script:rect.Left + [int]$point[0]) - $origin.X, ($script:rect.Top + [int]$point[1]) - $origin.Y)
}
function Send-Key([uint16]$vk) {
    if ($Background) { [Hoi4ProbeInput]::PostKey($script:window, $vk); Start-Sleep -Milliseconds 60; return }
    Invoke-Focused { [Hoi4ProbeInput]::KeyPress($script:window, [uint32]$script:game.Id, $vk) }
}
# Posted mouse messages carry client coordinates in the window's DPI-virtualised space.
function Get-LogicalClient($point) {
    $origin = New-Object Hoi4ProbeInput+Point
    if (-not [Hoi4ProbeInput]::ClientToScreen($script:window, [ref]$origin)) { throw 'Cannot locate the game client area.' }
    [Hoi4ProbeInput]::LogicalPoint($script:window, $script:rect.Left + [int]$point[0] - $origin.X, $script:rect.Top + [int]$point[1] - $origin.Y)
}
function Send-Click($point) {
    $x = $script:rect.Left + [int]$point[0]; $y = $script:rect.Top + [int]$point[1]
    if ($Background -and $PostedClicks) {
        $logical = Get-LogicalClient $point
        [Hoi4ProbeInput]::PostClick($script:window, [int]$logical[0], [int]$logical[1])
        return
    }
    if ($Background) {
        $shim = Open-InputShim
        if ($shim) { try { Invoke-ShimClick $shim $x $y } finally { Close-InputShim $shim }; return }
        # No shim attached: borrow the foreground for the click only.
        $flash = [Hoi4ProbeInput]::AcquireTargetFocus($script:window, [uint32]$script:game.Id)
        try { Start-Sleep -Milliseconds 120; [Hoi4ProbeInput]::Click($script:window, [uint32]$script:game.Id, $x, $y) }
        finally { [Hoi4ProbeInput]::RestoreForeground($flash, $script:window); [void][Hoi4ProbeInput]::PushBehind($script:window, $flash) }
        return
    }
    Invoke-Focused { [Hoi4ProbeInput]::Click($script:window, [uint32]$script:game.Id, $x, $y) }
}
function Send-Move($point) {
    if ($Background) {
        $logical = Get-LogicalClient $point
        [Hoi4ProbeInput]::PostMove($script:window, [int]$logical[0], [int]$logical[1])
        return
    }
    $x = $script:rect.Left + [int]$point[0]; $y = $script:rect.Top + [int]$point[1]
    Invoke-Focused { [Hoi4ProbeInput]::Move($script:window, [uint32]$script:game.Id, $x, $y) }
}
function Take-Screenshot([string]$path) { try { $null = & $smoke -Action Screenshot -GameId $script:game.Id -Out $path -Foreground:(-not $Background) } catch { Log "screenshot failed: $($_.Exception.Message)" } }
function Set-Paused([bool]$paused) {
    for ($attempt = 0; $attempt -lt 6; $attempt++) {
        $clock = Read-Clock
        if ($clock.paused -eq [int]$paused) { return $clock }
        Send-Key 0x20
        Start-Sleep -Milliseconds 250
    }
    throw "Pause state did not become $paused."
}
function Set-Speed([int]$speed) {
    for ($attempt = 0; $attempt -lt 10; $attempt++) {
        $clock = Read-Clock
        if ($clock.speed -eq $speed) { return }
        if ($clock.speed -lt $speed) { Send-Click $ui.speed_up } else { Send-Click $ui.speed_down }
        Start-Sleep -Milliseconds 150
    }
    throw "Speed did not reach $speed."
}
function Assert-Identity($clock, $identity) {
    if ($clock.play_session_started_at -ne $identity.play_session_started_at -or $clock.seed -ne $identity.seed -or $clock.game_state -ne $identity.game_state) {
        throw 'Game identity (load signal, seed or state pointer) changed unexpectedly.'
    }
}
function Get-BuildProcesses { @(Get-Process -Name $buildNames -ErrorAction SilentlyContinue | ForEach-Object { $_.ProcessName }) }
function Wait-Quiet {
    $quietSince = [DateTime]::UtcNow; $warned = $false
    while ($true) {
        $busy = Get-BuildProcesses
        if ($busy.Count -ne 0) {
            if (-not $warned) { Log ("Waiting for build processes to finish: " + ($busy -join ', ')); $warned = $true }
            $quietSince = [DateTime]::UtcNow
        } elseif (([DateTime]::UtcNow - $quietSince).TotalSeconds -ge 2) { return }
        Start-Sleep -Seconds 1
    }
}

# ---- reload ------------------------------------------------------------------
function Invoke-Relaunch([string]$runDir) {
    $script:game.Refresh()
    if (-not $script:game.HasExited) {
        # A recorder session must be detached first; a leftover ladder DLL dies with the process.
        if (@($script:game.Modules | Where-Object { $_.ModuleName -ieq 'collector.dll' }).Count -ne 0) { throw 'Relaunch reload requested while the collector is attached.' }
        if (@(Get-Process -Name recorder -ErrorAction SilentlyContinue).Count) { throw 'Relaunch reload requested while a recorder is running.' }
        if ($script:reader) { $script:reader.Dispose(); $script:reader = $null }
        $script:game.Kill(); if (-not $script:game.WaitForExit(30000)) { throw 'The game did not exit.' }
        Start-Sleep -Seconds 2
    }
    $started = [Diagnostics.Stopwatch]::StartNew()
    $extra = @('-start_save=' + [string]$spec.save)
    if ($spec.PSObject.Properties['launch_args'] -and $spec.launch_args) { $extra += @($spec.launch_args | ForEach-Object { [string]$_ }) }   # e.g. -debug_smooth=no
    $launch = & $smoke -Action Launch -GameExe $GameExe -UserDirectory $userPath -ExtraArgs $extra -Foreground:(-not $Background)
    Connect-Game
    $limit = [DateTime]::UtcNow.AddSeconds(240)
    do {
        $clock = $script:reader.Sample()
        if ($clock -and $clock.hour -eq $expectedHour -and $clock.seed -eq $expectedSeed -and $clock.paused -eq 1 -and $clock.tick_busy -eq 0 -and $clock.presentation_valid) { break }
        if ([DateTime]::UtcNow -gt $limit) { throw 'Relaunched game did not reach the paused baseline state.' }
        Start-Sleep -Milliseconds 500
    } while ($true)
    Start-Sleep -Seconds 3
    $clock = Read-Clock
    Log ("Relaunched into {0} in {1:n1}s; pid {2}, load signal {3}, foreground pushes {4}" -f $spec.save, $started.Elapsed.TotalSeconds, $script:game.Id, $clock.play_session_started_at, $launch.ForegroundPushes)
    return $clock
}
function Test-PauseMenu([string]$runDir) {
    # Compare the menu's button column against the calibrated reference patch (mean abs diff over the patch).
    $shot = Join-Path $runDir ('menu_probe_' + [DateTime]::UtcNow.ToString('HHmmssfff') + '.png')
    try { $null = & $smoke -Action Screenshot -GameId $script:game.Id -Out $shot -Foreground:(-not $Background) } catch { return $false }
    Add-Type -AssemblyName System.Drawing
    $ref = [Drawing.Bitmap]::FromFile($script:menuPatchPath); $img = [Drawing.Bitmap]::FromFile($shot)
    try {
        $sum = 0.0; $n = 0
        for ($y = 0; $y -lt $ref.Height; $y += 4) { for ($x = 0; $x -lt $ref.Width; $x += 4) {
            $a = $ref.GetPixel($x, $y); $b = $img.GetPixel(1150 + $x, 440 + $y)
            $sum += [math]::Abs($a.R - $b.R) + [math]::Abs($a.G - $b.G) + [math]::Abs($a.B - $b.B); $n++
        } }
        $mad = $sum / (3.0 * $n)
    } finally { $ref.Dispose(); $img.Dispose() }
    Remove-Item -LiteralPath $shot -ErrorAction SilentlyContinue
    return ($mad -lt 22)
}
function Open-PauseMenu([string]$runDir) {
    # Escape first closes popups and panels; repeat until the pause menu itself is visible.
    for ($attempt = 0; $attempt -lt 6; $attempt++) {
        Send-Key 0x1B; Start-Sleep -Milliseconds 700
        if (Test-PauseMenu $runDir) { return }
    }
    throw 'Pause menu did not open after six Escape presses.'
}
function Invoke-Reload([string]$runDir, [bool]$attached = $false) {
    $mode = $ReloadMode
    if ($spec.PSObject.Properties['reload_mode'] -and $spec.reload_mode) { $mode = [string]$spec.reload_mode }
    if ($mode -eq 'auto') { $mode = $(if ($attached) { 'menu' } else { 'relaunch' }) }
    if ($mode -eq 'relaunch') { return Invoke-Relaunch $runDir }
    if ($mode -eq 'none') {
        # chain runs without reloading (state drifts forward; only for comparisons that tolerate it)
        $null = Set-Paused $true
        $clock = Read-Clock
        if ($clock.hour -eq $expectedHour -or $script:chainStarted) { $script:chainStarted = $true; return $clock }
        $script:chainStarted = $true
        $mode = 'menu'   # first run of a chain starts from the baseline hour
    }
    (Get-Item -LiteralPath $savePath).LastWriteTime = Get-Date   # sorts first in the Local list
    $null = Set-Paused $true
    $before = Read-Clock
    Open-PauseMenu $runDir
    $shim = $(if ($Background -and -not $PostedClicks) { Open-InputShim } else { $null })
    if ($shim) {
        try {
            Invoke-ShimClick $shim ($script:rect.Left + [int]$ui.load_game[0]) ($script:rect.Top + [int]$ui.load_game[1]); Start-Sleep -Milliseconds 1000
            Invoke-ShimClick $shim ($script:rect.Left + [int]$ui.load_row[0]) ($script:rect.Top + [int]$ui.load_row[1]); Start-Sleep -Milliseconds 350
            Invoke-ShimClick $shim ($script:rect.Left + [int]$ui.load_button[0]) ($script:rect.Top + [int]$ui.load_button[1])
        } finally { Close-InputShim $shim }
    } elseif ($Background -and -not $PostedClicks) {
        # No shim: one foreground borrow for the two clicks.
        $flash = [Hoi4ProbeInput]::AcquireTargetFocus($script:window, [uint32]$script:game.Id)
        try {
            Start-Sleep -Milliseconds 120
            [Hoi4ProbeInput]::Click($script:window, [uint32]$script:game.Id, $script:rect.Left + [int]$ui.load_game[0], $script:rect.Top + [int]$ui.load_game[1]); Start-Sleep -Milliseconds 1000
            [Hoi4ProbeInput]::Click($script:window, [uint32]$script:game.Id, $script:rect.Left + [int]$ui.load_row[0], $script:rect.Top + [int]$ui.load_row[1]); Start-Sleep -Milliseconds 350
            [Hoi4ProbeInput]::Click($script:window, [uint32]$script:game.Id, $script:rect.Left + [int]$ui.load_button[0], $script:rect.Top + [int]$ui.load_button[1])
        } finally { [Hoi4ProbeInput]::RestoreForeground($flash, $script:window); [void][Hoi4ProbeInput]::PushBehind($script:window, $flash) }
    } else {
        Send-Click $ui.load_game; Start-Sleep -Milliseconds 1200
        Send-Click $ui.load_row; Start-Sleep -Milliseconds 400
        Send-Click $ui.load_button
    }
    $started = [Diagnostics.Stopwatch]::StartNew()
    $stable = $null
    while ($started.Elapsed.TotalSeconds -lt 240) {
        Start-Sleep -Milliseconds 250
        $script:game.Refresh(); if ($script:game.HasExited) { throw 'The game exited during reload.' }
        $clock = $script:reader.Sample()
        if ($null -eq $clock -or $clock.play_session_started_at -eq $before.play_session_started_at) { $stable = $null; continue }
        if ($clock.hour -ne $expectedHour -or $clock.seed -ne $expectedSeed -or $clock.paused -ne 1 -or $clock.tick_busy -ne 0 -or -not $clock.presentation_valid) { $stable = $null; continue }
        if ($null -eq $stable) { $stable = $clock; continue }
        if ($clock.play_session_started_at -eq $stable.play_session_started_at) { break }
        $stable = $null
    }
    if ($null -eq $stable) { Take-Screenshot (Join-Path $runDir 'reload_timeout.png'); throw 'Reload did not produce the expected paused baseline state within 240 s.' }
    Start-Sleep -Seconds 3
    Send-Move $ui.cursor_park
    $clock = Read-Clock
    if ($clock.hour -ne $expectedHour -or $clock.paused -ne 1) { throw 'Baseline state drifted after reload.' }
    Log ("Reloaded {0} in {1:n1}s; load signal {2}" -f $spec.save, $started.Elapsed.TotalSeconds, $clock.play_session_started_at)
    return $clock
}

# ---- recorder attach / warm / detach -----------------------------------------
function Convert-Win32Path([string]$path) {
    # The recorder reports canonical \\?\ paths; PowerShell 5.1 cmdlets cannot join them.
    if ($path.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) { return '\\' + $path.Substring(8) }
    if ($path.StartsWith('\\?\', [StringComparison]::OrdinalIgnoreCase)) { return $path.Substring(4) }
    return $path
}
function Read-Status([string]$path) {
    for ($attempt = 0; $attempt -lt 8; $attempt++) {
        try { return Get-Content -LiteralPath $path -Encoding UTF8 -Raw | ConvertFrom-Json } catch { Start-Sleep -Milliseconds 100 }
    }
    return $null
}
function Get-Gate($status, [string]$mode = 'strict') {
    $checks = @($status.consistency.successful_checks.PSObject.Properties)
    $mismatched = @($status.consistency.mismatches.PSObject.Properties | Where-Object { [int64]$_.Value -ne 0 } | ForEach-Object { $_.Name })
    $ready = @($checks | Where-Object { [int64]$_.Value -ge 200 }).Count
    [pscustomobject]@{
        windows = $checks.Count; ready = $ready; mismatched = $mismatched
        matrix = [int64]$status.war_matrix_checks; matrix_failures = [int64]$status.war_matrix_failures
        dropped = [int64]$status.dropped_frames; write_errors = [int64]$status.write_errors
        complete = $(switch ($mode) {
            'lenient' { ($checks.Count -gt 0 -and $ready -ge $checks.Count - 1) }
            'fast'    { ($checks.Count -gt 0 -and $ready -ge $checks.Count - 25 -and [int64]$status.war_matrix_checks -ge 200) }   # hourly windows only; staggered ones keep checking
            default   { ($checks.Count -gt 0 -and $ready -eq $checks.Count -and [int64]$status.war_matrix_checks -ge 200) }
        })
    }
}
function Start-Candidate($candidate, [string]$runDir) {
    $frozen = [string]$candidate.frozen
    $recorderExe = Join-Path $frozen 'recorder.exe'
    if (-not (Test-Path -LiteralPath $recorderExe) -or -not (Test-Path -LiteralPath (Join-Path $frozen 'collector.dll'))) { throw "Frozen candidate incomplete: $frozen" }
    if (@($script:game.Modules | Where-Object { $_.ModuleName -ieq 'collector.dll' }).Count -ne 0) { throw 'A collector is already loaded; detach it first.' }
    $recording = Join-Path $runDir 'recording'
    $null = New-Item -ItemType Directory -Force -Path $recording
    $process = Start-Process -FilePath $recorderExe -ArgumentList @('record', '--once', '--out', ('"' + $recording + '"'), '--user-dir', ('"' + $userPath + '"')) `
        -WorkingDirectory $frozen -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput (Join-Path $runDir 'recorder.stdout.log') -RedirectStandardError (Join-Path $runDir 'recorder.stderr.log')
    $attached = [pscustomobject]@{ process = $process; recording = $recording; status = $null; active = $null; frozen = $frozen }
    try {
        $activePath = Join-Path $recording 'active.json'
        $limit = [DateTime]::UtcNow.AddSeconds($AttachTimeoutSeconds)
        $active = $null
        do {
            Start-Sleep -Milliseconds 500
            if ($process.HasExited) { throw "Recorder exited before attaching (see $runDir\recorder.stderr.log)." }
            if (Test-Path -LiteralPath $activePath) {
                try { $candidateActive = Get-Content -LiteralPath $activePath -Raw | ConvertFrom-Json } catch { $candidateActive = $null }
                if ($candidateActive -and $candidateActive.session_dir -and $candidateActive.dll) { $active = $candidateActive }
            }
            if ([DateTime]::UtcNow -gt $limit) { throw 'Recorder did not attach in time.' }
        } while ($null -eq $active)
        $attached.active = $active
        $attached.status = Join-Path (Convert-Win32Path ([string]$active.session_dir)) 'status.json'
        do {
            Start-Sleep -Milliseconds 500
            $status = Read-Status $attached.status
            if ($status -and $status.state -eq 'recording') { break }
            if ($process.HasExited) { throw 'Recorder exited during collector startup.' }
            if ([DateTime]::UtcNow -gt $limit) { throw 'Collector did not reach the recording state in time.' }
        } while ($true)
        $script:game.Refresh()
        if (@($script:game.Modules | Where-Object { $_.ModuleName -ieq 'collector.dll' }).Count -ne 1) { throw 'collector.dll is not loaded after attach.' }
    } catch {
        # Never leave a half-attached recorder behind: request a detach and wait for it.
        Log "attach failed ($($_.Exception.Message)); requesting detach"
        try { $null = Stop-Candidate $attached $runDir } catch { Log "detach after failed attach also failed: $($_.Exception.Message)" }
        throw
    }
    Log "Attached $($candidate.name); collector $((Get-FileHash -LiteralPath (Join-Path $frozen 'collector.dll') -Algorithm SHA256).Hash.Substring(0,12))"
    $attached
}
function Invoke-Warm($attached, [string]$runDir, [string]$gateMode = 'strict') {
    $status = Read-Status $attached.status
    $gate = Get-Gate $status $gateMode
    if ($gate.complete) { Log 'Warm gate already complete.'; return 0 }
    Wait-Quiet
    Set-Speed 4; Send-Move $ui.cursor_park
    $identity = Set-Paused $false
    $timer = [Diagnostics.Stopwatch]::StartNew()
    try {
        do {
            Start-Sleep -Seconds 3
            $clock = Read-Clock; Assert-Identity $clock $identity
            if ($clock.paused -ne 0) { throw 'Game paused itself during warmup (event popup?).' }
            $status = Read-Status $attached.status
            if ($status) {
                $gate = Get-Gate $status $gateMode
                if ($gate.mismatched.Count -ne 0) { Write-Json (Join-Path $runDir 'warm_failed_status.json') $status; throw ('Consistency mismatches: ' + ($gate.mismatched -join ', ')) }
                if ($gate.dropped -ne 0 -or $gate.write_errors -ne 0 -or $gate.matrix_failures -ne 0) { throw 'Drops, write errors or matrix failures during warmup.' }
                if ([int]($timer.Elapsed.TotalSeconds) % 15 -lt 3) { Log ("warm {0:n0}s: hour {1}, windows {2}/{3}, matrix {4}" -f $timer.Elapsed.TotalSeconds, $clock.hour, $gate.ready, $gate.windows, $gate.matrix) }
            }
            if ($timer.Elapsed.TotalSeconds -gt $WarmTimeoutSeconds) { throw 'Warmup timed out.' }
        } while (-not $gate.complete)
    } finally { $null = Set-Paused $true }
    Write-Json (Join-Path $runDir 'warm_status.json') (Read-Status $attached.status)
    Log ("Warm complete in {0:n1}s" -f $timer.Elapsed.TotalSeconds)
    return [math]::Round($timer.Elapsed.TotalSeconds, 1)
}
function Stop-Candidate($attached, [string]$runDir) {
    $recorderExe = Join-Path $attached.frozen 'recorder.exe'
    $detach = Start-Process -FilePath $recorderExe -ArgumentList @('detach', '--out', ('"' + $attached.recording + '"')) -WorkingDirectory $attached.frozen -WindowStyle Hidden -PassThru -Wait `
        -RedirectStandardOutput (Join-Path $runDir 'detach.stdout.log') -RedirectStandardError (Join-Path $runDir 'detach.stderr.log')
    if (-not $attached.process.WaitForExit(180000)) { throw 'Recorder did not exit after detach.' }
    $script:game.Refresh()
    if (@($script:game.Modules | Where-Object { $_.ModuleName -ieq 'collector.dll' }).Count -ne 0) { throw 'collector.dll remains loaded after detach.' }
    $completed = @(Get-ChildItem -LiteralPath $attached.recording -Filter completed.json -Recurse -ErrorAction SilentlyContinue)
    $summary = $null
    if ($completed.Count -eq 1) { $summary = Get-Content -LiteralPath $completed[0].FullName -Raw | ConvertFrom-Json }
    Log ("Detached (exit {0}); drain_complete={1} partial={2}" -f $detach.ExitCode, $(if ($summary) { $summary.drain_complete } else { '?' }), $(if ($summary) { $summary.partial } else { '?' }))
    return $summary
}
function Start-Ladder($candidate, [string]$runDir) {
    $dll = Join-Path $LadderDir 'ladder.dll'; $inject = Join-Path $LadderDir 'ladder-inject.exe'
    if (-not (Test-Path -LiteralPath $dll) -or -not (Test-Path -LiteralPath $inject)) { throw "Ladder binaries missing under $LadderDir" }
    $script:game.Refresh()
    if (@($script:game.Modules | Where-Object { $_.ModuleName -ieq 'ladder.dll' -or $_.ModuleName -ieq 'collector.dll' }).Count -ne 0) { throw 'A ladder or collector DLL is already loaded.' }
    $spec = $candidate.ladder
    $config = [ordered]@{
        mode = [string]$spec.mode
        param_mb = $(if ($spec.PSObject.Properties['param_mb'] -and $null -ne $spec.param_mb) { [double]$spec.param_mb } else { 0 })
        param_ms = $(if ($spec.PSObject.Properties['param_ms'] -and $null -ne $spec.param_ms) { [double]$spec.param_ms } else { 0 })
        param_n = $(if ($spec.PSObject.Properties['param_n'] -and $null -ne $spec.param_n) { [int]$spec.param_n } else { 0 })
        output = (Join-Path $runDir 'ladder_result.json')
        detach_event = ('Local\hoi4_ladder_' + [Guid]::NewGuid().ToString('N').Substring(0, 12))
        max_hours = 262144
    }
    if ($spec.PSObject.Properties['param_hex_ranges'] -and $spec.param_hex_ranges) { $config.param_hex_ranges = @($spec.param_hex_ranges) }
    $configPath = Join-Path $runDir 'ladder.json'
    # The hook installer can lose a race with a thread that exits during the attach; retry with fresh events.
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        $config.detach_event = ('Local\hoi4_ladder_' + [Guid]::NewGuid().ToString('N').Substring(0, 12))
        Write-Json $configPath $config
        $text = & $inject attach $script:game.Id $dll $configPath 2>&1 | Out-String
        $script:game.Refresh()
        $loaded = @($script:game.Modules | Where-Object { $_.ModuleName -ieq 'ladder.dll' }).Count
        if ($LASTEXITCODE -eq 0 -and $text -match 'attached' -and $loaded -eq 1) { break }
        if ($loaded -ne 0) { throw "ladder attach failed and ladder.dll stayed loaded: $text" }
        if ($attempt -eq 3) { throw "ladder attach failed: $text" }
        Log ("ladder attach attempt {0} failed, retrying: {1}" -f $attempt, ($text -replace '\s+', ' ').Trim())
        Start-Sleep -Seconds 2
    }
    Log ("Ladder attached: {0} mb={1} ms={2} n={3}" -f $config.mode, $config.param_mb, $config.param_ms, $config.param_n)
    [pscustomobject]@{ dll = $dll; inject = $inject; output = $config.output; config = $config }
}
function Stop-Ladder($ladder, [string]$runDir) {
    $text = & $ladder.inject detach $script:game.Id $ladder.dll 2>&1 | Out-String
    [IO.File]::WriteAllText((Join-Path $runDir 'ladder_detach.log'), $text, $utf8)
    if ($LASTEXITCODE -ne 0) { throw "ladder detach failed: $text" }
    $script:game.Refresh()
    if (@($script:game.Modules | Where-Object { $_.ModuleName -ieq 'ladder.dll' }).Count -ne 0) { throw 'ladder.dll remains loaded after detach.' }
    if (-not (Test-Path -LiteralPath $ladder.output)) { throw 'ladder results file missing after detach.' }
    $r = Get-Content -LiteralPath $ladder.output -Raw | ConvertFrom-Json
    Log ("Ladder detached: {0} hours, own mean {1:n3} ms, original tick mean {2:n3} ms, faults {3}, overruns {4}" -f $r.hours, ($r.callback_own_ns.mean / 1e6), ($r.original_tick_ns.mean / 1e6), $r.faults, $r.overruns)
    return [ordered]@{
        mode = $r.mode; params = $r.params; hours = $r.hours; sampled_hours = $r.sampled_hours
        own_mean_ms = [math]::Round($r.callback_own_ns.mean / 1e6, 3); own_p50_ms = [math]::Round($r.callback_own_ns.p50 / 1e6, 3)
        own_p95_ms = [math]::Round($r.callback_own_ns.p95 / 1e6, 3); own_max_ms = [math]::Round($r.callback_own_ns.max / 1e6, 3)
        tick_mean_ms = [math]::Round($r.original_tick_ns.mean / 1e6, 3); tick_p50_ms = [math]::Round($r.original_tick_ns.p50 / 1e6, 3)
        tick_p95_ms = [math]::Round($r.original_tick_ns.p95 / 1e6, 3); tick_max_ms = [math]::Round($r.original_tick_ns.max / 1e6, 3)
        first_hour = $r.first_hour; last_hour = $r.last_hour; faults = $r.faults; guarded_reads = $r.guarded_reads
        overruns = $r.overruns; bytes_per_hour = $r.bytes_per_hour; regions = $r.regions; notes = $r.notes
    }
}
function Get-ExactMetrics($attached, [string]$runDir, [int]$firstHour, [int]$lastHour) {
    if (-not (Test-Path -LiteralPath $MetricsTool)) { return $null }
    $replays = @(Get-ChildItem -LiteralPath $attached.recording -Filter '*.battleplan' -File -ErrorAction SilentlyContinue)
    if ($replays.Count -ne 1) { return $null }
    foreach ($branch in @(2, 1, 3)) {
        try {
            $text = & $MetricsTool $replays[0].FullName $branch $firstHour ($lastHour - 1) 2>$null | Out-String
            $metrics = $text | ConvertFrom-Json
            if ($metrics.hours -gt 0 -and $metrics.observed_first -eq $firstHour) {
                [IO.File]::WriteAllText((Join-Path $runDir ('exact_metrics_branch' + $branch + '.json')), $text, $utf8)
                return [pscustomobject]@{ branch = $branch; frames = $metrics.hours; callback_mean_ms = [math]::Round($metrics.callback.mean_ns / 1e6, 3);
                    callback_p50_ms = [math]::Round($metrics.callback.p50_ns / 1e6, 3); callback_p95_ms = [math]::Round($metrics.callback.p95_ns / 1e6, 3);
                    callback_max_ms = [math]::Round($metrics.callback.max_ns / 1e6, 3); pool_bytes_max = $metrics.pool_bytes_max;
                    failures = @($metrics.failures.PSObject.Properties).Count }
            }
        } catch { }
    }
    return $null
}

# ---- timed screen --------------------------------------------------------------
function Invoke-Screen($attached, [string]$runDir) {
    Wait-Quiet
    $identity = Set-Paused $true
    if ($identity.hour -ne $expectedHour -and -not $script:chainStarted) { throw "Screen must start at hour $expectedHour, found $($identity.hour)." }
    Set-Speed 4; Send-Move $ui.cursor_park
    $statusBefore = $null
    if ($attached) { $statusBefore = Read-Status $attached.status; Write-Json (Join-Path $runDir 'status_before.json') $statusBefore }
    $log = [IO.StreamWriter]::new((Join-Path $runDir 'samples.jsonl'), $false, $utf8)
    $contaminants = @{}
    $first = $null; $last = $null; $firstQpc = 0L; $lastQpc = 0L; $valid = 0; $missing = 0
    $null = Set-Paused $false
    $startQpc = [Hoi4ClockBaseline]::Qpc()
    $frequency = [double]$script:reader.Frequency
    $nextGuard = $startQpc
    try {
        while (([Hoi4ClockBaseline]::Qpc() - $startQpc) / $frequency -lt $seconds) {
            if ($null -ne $last -and ([int64]$last.hour - [int64]$first.hour) -ge $maxHours) { break }   # stay clear of the save's first player event
            $qpcNow = [Hoi4ClockBaseline]::Qpc()
            if ($qpcNow -ge $nextGuard) {
                foreach ($name in (Get-BuildProcesses)) { $contaminants[$name] = $true }
                $nextGuard = $qpcNow + [int64]$frequency
            }
            $clock = $script:reader.Sample()
            $qpc = [Hoi4ClockBaseline]::Qpc()
            if ($null -eq $clock) { $missing++ }
            else {
                Assert-Identity $clock $identity
                if ($clock.paused -ne 0 -or $clock.speed -ne 4) { throw 'Speed or pause changed during the timed window.' }
                if ($null -ne $last -and $clock.hour -lt $last.hour) { throw 'Backwards clock.' }
                $valid++
                if ($null -eq $first) { $first = $clock; $firstQpc = $qpc }
                $last = $clock; $lastQpc = $qpc
            }
            $log.WriteLine((@{ qpc = $qpc; elapsed_s = ($qpc - $startQpc) / $frequency; hour = $(if ($clock) { $clock.hour } else { $null }) } | ConvertTo-Json -Compress))
            Start-Sleep -Milliseconds 125
        }
    } finally {
        $null = Set-Paused $true
        $log.Dispose()
    }
    $statusAfter = $null
    if ($attached) { $statusAfter = Read-Status $attached.status; Write-Json (Join-Path $runDir 'status_after.json') $statusAfter }
    if ($null -eq $first -or $null -eq $last -or $lastQpc -le $firstQpc) { throw 'No valid timed samples.' }
    $elapsed = ($lastQpc - $firstQpc) / $frequency
    $hours = [int64]$last.hour - [int64]$first.hour
    $rate = $hours / $elapsed
    $cameraMoved = $false
    if ($identity.eye -and $last.eye) { $cameraMoved = [bool](@(0, 1, 2) | Where-Object { [math]::Abs([double]$identity.eye[$_] - [double]$last.eye[$_]) -gt 0.5 -or [math]::Abs([double]$identity.look_at[$_] - [double]$last.look_at[$_]) -gt 0.5 }).Count }
    $result = [ordered]@{
        camera_moved = $cameraMoved; camera_start = $(if ($identity.eye) { @($identity.eye) } else { $null }); camera_end = $(if ($last.eye) { @($last.eye) } else { $null })
        first_hour = $first.hour; last_hour = $last.hour; hours = $hours; elapsed_s = [math]::Round($elapsed, 4)
        hours_per_s = [math]::Round($rate, 4); baseline_fraction = [math]::Round($rate / $baselineRate, 4)
        valid_samples = $valid; missing_samples = $missing; contaminated_by = @($contaminants.Keys)
        load_signal = $identity.play_session_started_at
    }
    if ($statusBefore -and $statusAfter) {
        $calls = [int64]$statusAfter.callback_work.calls - [int64]$statusBefore.callback_work.calls
        $ticks = [int64]$statusAfter.native_tick_work.calls - [int64]$statusBefore.native_tick_work.calls
        $result.status_delta = [ordered]@{
            frames = $calls
            callback_mean_ms = $(if ($calls -gt 0) { [math]::Round(([double]$statusAfter.callback_work.total_ns - [double]$statusBefore.callback_work.total_ns) / $calls / 1e6, 3) } else { $null })
            native_tick_mean_ms = $(if ($ticks -gt 0) { [math]::Round(([double]$statusAfter.native_tick_work.elapsed_ns - [double]$statusBefore.native_tick_work.elapsed_ns) / $ticks / 1e6, 3) } else { $null })
            session_callback_p50_ms = [math]::Round([double]$statusAfter.callback_work.p50_ns / 1e6, 3)
            session_callback_p95_ms = [math]::Round([double]$statusAfter.callback_work.p95_ns / 1e6, 3)
            pool_bytes = $statusAfter.pool_bytes
            dropped_frames = $statusAfter.dropped_frames; write_errors = $statusAfter.write_errors
        }
    }
    return $result
}

# ---- main loop -----------------------------------------------------------------
Connect-Game
$spec.candidates | ForEach-Object { if ($_.frozen -and -not (Test-Path -LiteralPath (Join-Path ([string]$_.frozen) 'collector.dll'))) { throw "Candidate $($_.name): collector.dll missing under $($_.frozen)" } }
$resultsPath = Join-Path $Out 'results.jsonl'
$rows = @()
Log ("Plan: {0} candidates x {1} repeats, {2}s screens, save {3} ({4})" -f @($spec.candidates).Count, $repeats, $seconds, $spec.save, $saveSha.Substring(0, 12))
for ($rep = 1; $rep -le $repeats; $rep++) {
    $index = 0
    foreach ($candidate in $spec.candidates) {
        $index++
        $name = [string]$candidate.name
        $runDir = Join-Path $Out ("r{0:d2}_{1:d2}_{2}" -f $rep, $index, $name)
        $null = New-Item -ItemType Directory -Force -Path $runDir
        $row = [ordered]@{ name = $name; repeat = $rep; frozen = $candidate.frozen; started_utc = [DateTime]::UtcNow.ToString('o'); passed = $false }
        $attached = $null; $ladder = $null
        try {
            Log "=== $name (repeat $rep) ==="
            # Unwarmed runs reload first (relaunch, no clicks) then attach; warmed recorders reload via the menu.
            if ($candidate.PSObject.Properties['ladder'] -and $candidate.ladder) {
                $null = Invoke-Reload $runDir $false
                Wait-Quiet
                $ladder = Start-Ladder $candidate $runDir
                $row.ladder_config = $ladder.config
            } elseif ($candidate.frozen) {
                $row.collector_sha256 = (Get-FileHash -LiteralPath (Join-Path ([string]$candidate.frozen) 'collector.dll') -Algorithm SHA256).Hash
                $warm = $true
                if ($candidate.PSObject.Properties['warm'] -and $null -ne $candidate.warm) { $warm = [bool]$candidate.warm }
                $gateMode = 'strict'
                if ($candidate.PSObject.Properties['warm_gate'] -and $candidate.warm_gate) { $gateMode = [string]$candidate.warm_gate }
                if ($warm) {
                    $null = Invoke-Reload $runDir $false                 # warm from the baseline hour
                    $attached = Start-Candidate $candidate $runDir
                    $row.warm_seconds = Invoke-Warm $attached $runDir $gateMode
                    $row.warm_gate = $gateMode
                    $null = Invoke-Reload $runDir $true                  # back to the baseline hour, recorder kept
                } else {
                    $row.warm_seconds = $null
                    $null = Invoke-Reload $runDir $false
                    $attached = Start-Candidate $candidate $runDir
                }
            } else {
                $null = Invoke-Reload $runDir $false
            }
            Take-Screenshot (Join-Path $runDir 'reloaded.png')
            $screen = Invoke-Screen $attached $runDir
            foreach ($key in $screen.Keys) { $row[$key] = $screen[$key] }
            if ($attached) {
                $row.completion = Stop-Candidate $attached $runDir
                $exact = Get-ExactMetrics $attached $runDir ([int]$screen.first_hour) ([int]$screen.last_hour)
                if ($exact) { $row.exact = $exact }
                $attached = $null
            }
            if ($ladder) {
                $row.ladder = Stop-Ladder $ladder $runDir
                $ladder = $null
            }
            $row.passed = ($screen.contaminated_by.Count -eq 0 -and -not $screen.camera_moved)
            if ($screen.camera_moved) { Log 'WARNING: the camera moved during the timed window (edge scroll?); run flagged.' }
            Log ("{0}: {1} h/s = {2:p1} of baseline ({3} hours / {4}s){5}" -f $name, $screen.hours_per_s, $screen.baseline_fraction, $screen.hours, $screen.elapsed_s, $(if ($screen.contaminated_by.Count) { ' CONTAMINATED by ' + ($screen.contaminated_by -join ',') } else { '' }))
        } catch {
            $row.error = $_.Exception.Message
            Log "FAILED ${name}: $($_.Exception.Message)"
            Take-Screenshot (Join-Path $runDir 'failure.png')
            if ($attached) { try { $null = Stop-Candidate $attached $runDir } catch { Log "detach after failure also failed: $($_.Exception.Message)" } }
            if ($ladder) { try { $null = Stop-Ladder $ladder $runDir } catch { Log "ladder detach after failure also failed: $($_.Exception.Message)" } }
            try { $null = Set-Paused $true } catch { }
        }
        $row.finished_utc = [DateTime]::UtcNow.ToString('o')
        Write-Json (Join-Path $runDir 'result.json') $row
        [IO.File]::AppendAllText($resultsPath, (($row | ConvertTo-Json -Depth 20 -Compress) + "`n"), $utf8)
        $rows += [pscustomobject]$row
    }
}
$sameRunRates = @($rows | Where-Object { $_.passed -and -not $_.frozen -and -not ($_.PSObject.Properties['ladder_config'] -and $_.ladder_config) -and $_.PSObject.Properties['hours_per_s'] } | ForEach-Object { [double]$_.hours_per_s })
$sameRunBaseline = if ($sameRunRates.Count) { ($sameRunRates | Measure-Object -Average).Average } else { $null }
$table = foreach ($r in $rows) {
    $sameRun = if ($sameRunBaseline -and $r.PSObject.Properties['hours_per_s'] -and $r.hours_per_s) { '{0:p1}' -f ([double]$r.hours_per_s / $sameRunBaseline) } else { '' }
    $extraMs = if ($sameRunBaseline -and $r.PSObject.Properties['hours_per_s'] -and $r.hours_per_s) { '{0:n2}' -f (1000.0 / [double]$r.hours_per_s - 1000.0 / $sameRunBaseline) } else { '' }
    $exactText = if ($r.PSObject.Properties['exact'] -and $r.exact) { "{0}/{1}/{2}" -f $r.exact.callback_mean_ms, $r.exact.callback_p50_ms, $r.exact.callback_p95_ms } else { '' }
    $deltaCallback = if ($r.PSObject.Properties['status_delta'] -and $r.status_delta) { $r.status_delta.callback_mean_ms } elseif ($r.PSObject.Properties['ladder'] -and $r.ladder) { "{0} (p95 {1})" -f $r.ladder.own_mean_ms, $r.ladder.own_p95_ms } else { '' }
    $deltaTick = if ($r.PSObject.Properties['status_delta'] -and $r.status_delta) { $r.status_delta.native_tick_mean_ms } elseif ($r.PSObject.Properties['ladder'] -and $r.ladder) { "{0} (p95 {1})" -f $r.ladder.tick_mean_ms, $r.ladder.tick_p95_ms } else { '' }
    $err = if ($r.PSObject.Properties['error']) { $r.error } else { '' }
    "| {0} | {1} | {2} | {3} | {4} | {5} | {6} | {7} | {8} | {9} | {10} | {11} |" -f $r.name, $r.repeat, $(if ($r.PSObject.Properties['hours_per_s']) { $r.hours_per_s } else { '' }), $sameRun, $extraMs, $(if ($r.PSObject.Properties['baseline_fraction']) { ('{0:p1}' -f $r.baseline_fraction) } else { '' }), $deltaCallback, $deltaTick, $exactText, $(if ($r.PSObject.Properties['warm_seconds']) { $r.warm_seconds } else { '' }), $(if ($r.PSObject.Properties['contaminated_by'] -and $r.contaminated_by.Count) { 'yes' } else { '' }), $err
}
$baselineText = if ($sameRunBaseline) { ('same-run no-recorder baseline {0:n3} h/s (mean of {1})' -f $sameRunBaseline, $sameRunRates.Count) } else { 'no same-run baseline' }
$md = @("# Benchmark $(Split-Path -Leaf $Out)", '', "Save $($spec.save) (SHA256 $saveSha), screens ${seconds}s at native speed 4; retained baseline $baselineRate h/s; $baselineText.", '',
    '| candidate | rep | h/s | % same-run baseline | extra ms/game-hour | % retained baseline | callback own/mean ms | native tick mean ms | exact callback mean/p50/p95 ms | warm s | contaminated | error |',
    '|---|---|---|---|---|---|---|---|---|---|---|---|') + $table
[IO.File]::WriteAllText((Join-Path $Out 'results.md'), ($md -join "`n") + "`n", $utf8)
$md -join "`n"
