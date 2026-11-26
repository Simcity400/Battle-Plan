<#
.SYNOPSIS
Automated same-save throughput benchmark loop for recorder candidates.
.DESCRIPTION
For every candidate in the plan: reloads the same benchmark save, attaches the
frozen recorder (if any), runs the game at the plan's native speed (default 4,
displayed 5, uncapped; lower speeds wait between ticks) while
sampling its clock from outside the process, pauses and detaches. A screen ends
at the planned seconds or max_hours, whichever comes first. An hour-capped
screen is not a full-duration run. These are throughput screens, not correctness
or performance acceptance. Every scheduled table is acquired once from hour 1;
there are no consistency windows to warm.

A candidate with "frozen": null is a matched no-recorder reference. The report
uses clean references from this plan for its same-run percentage. An optional
baseline_hours_per_s is labelled historical and does not establish current
performance. Runs are interleaved per repeat. Timed windows start at once:
nothing waits for builds or other load (AGENTS.md). A screen is marked
contaminated when a local build process appears mid-window; other processes'
CPU over the window is reported in other_cpu without failing the screen.

Plan JSON:
{
  "save": "benchmark.hoi4",
  "expected_hour": 100, "expected_seed": 2468,
  "seconds": 30, "max_hours": 420, "repeats": 1,
  "candidates": [
    { "name": "baseline", "frozen": null },
    { "name": "candidate", "frozen": "candidate-bin" }
  ]
}
"frozen" is a directory holding recorder.exe and collector.dll. A recorder
candidate's "capture_mode" ("full" or "map", default the recorder's own) is passed
to it as --capture-mode. "speed" is the native game speed 0-4 (default 4). "max_hours"
defaults to 420; choose a cap above the expected progress for a full-duration run.
"console" lists console lines (e.g. "observe") typed after each reload, before a
recorder attaches; they briefly take the foreground. "alternate" reverses the
recorder candidates' order on even repeats. "status_every_s" appends the
recorder's cumulative pause counters, histogram and frame waits to
status_series.jsonl at that interval during a screen.
"save_at_hour" pauses the first screen that reaches that game hour, saves the game
through its menu as "save_name" (default BENCH_H<hour>) in the user directory, and
resumes; the pause counts toward neither the screen's duration nor its elapsed time.
The name is a plain file name; a trailing .hoi4 is dropped. The UI config then needs
save_menu, save_name and save_confirm.
-ClockDirectory holds the recorder.exe whose clock command reads the game clock
(default: this checkout's .cargo-target/release); it is also passed to smoke.ps1.
Ordinary recorder candidates require -MetricsTool pointing to a reader built for
their replay format. Missing or incomplete metrics, failed drains, drops and write
errors fail the screen. Bare-game references do not need a metrics reader.
Explicit diagnostic=true candidates require a diag-* name and frozen recorder.
Their intentionally invalid output skips recording/size/exact-metrics acceptance;
h/s and status remain, with recording_valid=false and passed=false. Unload safety
still applies, and diagnostic rows never enter ordinary passed summaries.
A third candidate kind injects the synthetic cost-ladder DLL instead of the
recorder: { "name": "spin4", "ladder": { "mode": "spin_ms", "param_ms": 4 } }
Supply a separately built ladder.dll and ladder-inject.exe with -LadderDir;
the DLL's own per-hour timings are folded into the results.

Every screen samples hoi4.exe private bytes and working set once per second
with the game hour; recorder screens also sample recorder.exe private bytes
through detach and finalization. Recorder screens record the finalization
summary: raw encoded, spool and replay bytes per recorded hour, and each
table's raw encoded bytes and read times (tables.json in the screen directory).
results.md adds per-candidate means: slowdown paired with the same repeat's
no-recorder screen (95% t interval across repeats), hoi4.exe private bytes
above the no-recorder screens at matched game hours, sizes per hour, and
tables ranked by encoded bytes and by read-time p95.
.PARAMETER UiConfig
User-calibrated menu/control coordinates; bench_ui.json is an example to recalibrate.
.PARAMETER MenuPatch
Pause-menu reference image for menu reloads, with its outer-window origin in MenuPatchOrigin.
.PARAMETER GameLock
Hold this shared lock for the full benchmark. Create its directory and owner file
before running, then remove them after all owned game/recorder work has stopped.
.PARAMETER LockOwner
Exact owner-file token from the caller's held game lock.
.PARAMETER WorkerSampler
Collector profiling configuration forwarded as BATTLEPLAN_WORKER_SAMPLER to recorder candidates.
Requires a frozen collector built with its worker_sampler feature.
.PARAMETER CaptureRoot
User-selected output root; defaults to LOCALAPPDATA/BattlePlan/captures.
.EXAMPLE
powershell -File tools/bench.ps1 -Plan ./plan.json -GameExe "$(if (Test-Path Env:BATTLEPLAN_GAME_DIR) { $env:BATTLEPLAN_GAME_DIR } else { $env:HOI4_GAME_DIR })/hoi4.exe" -UserDirectory ./profile -Out "$env:LOCALAPPDATA/BattlePlan/captures/bench/run01" -MetricsTool ./metrics.exe -LockOwner $ownerToken
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Plan,
    [Parameter(Mandatory = $true)][string]$Out,
    [Parameter(Mandatory = $true)][string]$GameExe,
    [Parameter(Mandatory = $true)][string]$UserDirectory,
    [string]$UiConfig = '',
    [string]$MetricsTool = '',
    [string]$ClockDirectory = '',
    [string]$LadderDir = '',
    [string]$InputShimDir = '',
    [string]$MenuPatch = '',
    [int[]]$MenuPatchOrigin = @(1150, 440),
    [string]$CaptureRoot = (Join-Path $env:LOCALAPPDATA 'BattlePlan\captures'),
    [string]$WorkerSampler = '',
    [string]$GameLock = (Join-Path $env:TEMP 'claude\hoi4_game.lock'),
    [string]$LockOwner,
    [int]$AttachTimeoutSeconds = 300,
    [switch]$Foreground,
    [switch]$Background,
    [switch]$PostedClicks,
    [ValidateSet('menu', 'auto')][string]$ReloadMode = 'auto',
    [switch]$SummarizeOnly
)

Set-StrictMode -Version 1
$ErrorActionPreference = 'Stop'
if (-not [Environment]::Is64BitProcess) { throw 'Run bench.ps1 in 64-bit PowerShell.' }
$Background = -not $Foreground
$GameExe = (Resolve-Path -LiteralPath $GameExe).ProviderPath

$captureRoot = [IO.Path]::GetFullPath($CaptureRoot).TrimEnd([char[]]'\/') + [IO.Path]::DirectorySeparatorChar
$Out = [IO.Path]::GetFullPath($Out)
if (-not $Out.StartsWith($captureRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Out must be below CaptureRoot.' }
$null = New-Item -ItemType Directory -Force -Path $Out
$utf8 = New-Object Text.UTF8Encoding $false
$spec = Get-Content -LiteralPath $Plan -Raw | ConvertFrom-Json
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $UiConfig) { $UiConfig = Join-Path $scriptDir 'bench_ui.json' }
$ui = Get-Content -LiteralPath $UiConfig -Raw | ConvertFrom-Json
$smoke = Join-Path $scriptDir 'smoke.ps1'
$script:planSaved = $false
$script:menuPatchPath = $MenuPatch
if ($MenuPatchOrigin.Count -ne 2 -or $MenuPatchOrigin[0] -lt 0 -or $MenuPatchOrigin[1] -lt 0) { throw 'MenuPatchOrigin requires two nonnegative pixel coordinates.' }
$userPath = (Resolve-Path -LiteralPath $UserDirectory).ProviderPath
$savePath = Join-Path (Join-Path $userPath 'save games') ([string]$spec.save)
if (Test-Path -LiteralPath $savePath -PathType Leaf) { $saveSha = (Get-FileHash -LiteralPath $savePath -Algorithm SHA256).Hash }
elseif ($SummarizeOnly) { $saveSha = 'not checked' }
else { throw "Benchmark save missing: $savePath" }
$expectedHour = [int]$spec.expected_hour
$expectedSeed = [int]$spec.expected_seed
$seconds = if ($spec.seconds) { [int]$spec.seconds } else { 30 }
$maxHours = $(if ($spec.PSObject.Properties['max_hours'] -and $spec.max_hours) { [int64]$spec.max_hours } else { 420 })
$repeats = if ($spec.repeats) { [int]$spec.repeats } else { 1 }
$speed = if ($spec.PSObject.Properties['speed'] -and $null -ne $spec.speed) { [int]$spec.speed } else { 4 }
if ($speed -lt 0 -or $speed -gt 4) { throw "Plan speed must be 0-4, found $speed." }
$historicalBaselineRate = if ($spec.PSObject.Properties['baseline_hours_per_s'] -and $spec.baseline_hours_per_s) { [double]$spec.baseline_hours_per_s } else { $null }
foreach ($candidate in $spec.candidates) {
    if ($candidate.PSObject.Properties['diagnostic'] -and $candidate.diagnostic -isnot [bool]) { throw 'diagnostic must be a JSON boolean.' }
    $diagnostic = [bool]($candidate.PSObject.Properties['diagnostic'] -and $candidate.diagnostic)
    if ($diagnostic -and (-not $candidate.frozen -or [string]$candidate.name -notlike 'diag-*')) { throw 'Diagnostic candidates require a frozen recorder and a diag-* name.' }
    if (($candidate.PSObject.Properties['warm'] -and $candidate.warm) -or $candidate.PSObject.Properties['warm_gate']) {
        throw 'Consistency warmup was retired. Remove warm=true and warm_gate from the benchmark plan.'
    }
    if ($candidate.PSObject.Properties['capture_mode']) {
        if (-not $candidate.frozen) { throw 'capture_mode applies only to frozen recorder candidates.' }
        if ([string]$candidate.capture_mode -cnotin @('full', 'map')) { throw 'capture_mode must be "full" or "map".' }
    }
    if ($candidate.frozen -and -not $diagnostic -and -not $SummarizeOnly -and (-not $MetricsTool -or -not (Test-Path -LiteralPath $MetricsTool -PathType Leaf))) {
        throw 'Recorder candidates require -MetricsTool with a reader built for their replay format.'
    }
}
$buildNames = @('cargo', 'rustc', 'lld-link', 'clippy-driver', 'link', 'cl')
$lock = [IO.Path]::GetFullPath($GameLock)
if (-not $SummarizeOnly -and (-not $LockOwner -or -not (Test-Path -LiteralPath (Join-Path $lock 'owner')) -or [IO.File]::ReadAllText((Join-Path $lock 'owner')).Trim() -cne $LockOwner)) { throw "Supply the exact LockOwner token from your game-lock acquisition: $lock" }

. (Join-Path $scriptDir 'game-clock.ps1')
$clockRecorder = if ($SummarizeOnly) { $null } else { Resolve-ClockRecorder $ClockDirectory }
$driverSource = Get-Content -LiteralPath $smoke -Encoding UTF8 -Raw
foreach ($definition in [regex]::Matches($driverSource, "(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@")) {
    $source = $definition.Groups[1].Value
    if ($source -match 'class Hoi4ProbeInput' -and -not ('Hoi4ProbeInput' -as [type])) { Add-Type -TypeDefinition $source }
}
if (-not ('Hoi4ProbeInput' -as [type])) { throw 'Could not load the smoke driver helper classes.' }
[void][Hoi4ProbeInput]::SetProcessDPIAware()

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

$script:game = $null; $script:window = [IntPtr]::Zero; $script:reader = $null; $script:rect = $null
$script:recorders = New-Object Collections.ArrayList
$script:freshLaunch = $false
function Start-SaveLaunch {
    $extra = @('-start_save=' + [string]$spec.save)
    if ($spec.PSObject.Properties['launch_args'] -and $spec.launch_args) { $extra += @($spec.launch_args | ForEach-Object { [string]$_ }) }
    & $smoke -GameExe $GameExe -CaptureRoot $CaptureRoot -InputShimDir $InputShimDir -BinaryDirectory (Split-Path -Parent $clockRecorder) -Action Launch -Out $Out -UserDirectory $userPath -ExtraArgs $extra -Foreground:(-not $Background)
}
function Connect-Game {
    $games = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
    if ($games.Count -gt 1) { throw 'Two hoi4.exe processes; close one.' }
    if ($games.Count -eq 0) {
        Log 'No game running; launching into the benchmark save.'
        $null = Start-SaveLaunch
        $script:freshLaunch = $true
        $limit = [DateTime]::UtcNow.AddSeconds(300)
        do {
            Start-Sleep -Milliseconds 250
            $games = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
            if ([DateTime]::UtcNow -gt $limit) { throw 'Game did not start.' }
        } while ($games.Count -ne 1 -or $games[0].MainWindowHandle -eq [IntPtr]::Zero)
    }
    $script:game = $games[0]
    if ($script:game.Path -ine $GameExe) { throw "Running game is not the pinned executable: $($script:game.Path)" }
    if ($(if (Test-Path Env:BATTLEPLAN_BENCH_AFFINITY) { $env:BATTLEPLAN_BENCH_AFFINITY } else { $env:HOI4_BENCH_AFFINITY })) { $script:game.ProcessorAffinity = [IntPtr][Convert]::ToInt64($(if (Test-Path Env:BATTLEPLAN_BENCH_AFFINITY) { $env:BATTLEPLAN_BENCH_AFFINITY } else { $env:HOI4_BENCH_AFFINITY }), 16); Log "game affinity $($(if (Test-Path Env:BATTLEPLAN_BENCH_AFFINITY) { $env:BATTLEPLAN_BENCH_AFFINITY } else { $env:HOI4_BENCH_AFFINITY }))" }
    if ($(if (Test-Path Env:BATTLEPLAN_BENCH_PRIORITY) { $env:BATTLEPLAN_BENCH_PRIORITY } else { $env:HOI4_BENCH_PRIORITY })) { $script:game.PriorityClass = [Diagnostics.ProcessPriorityClass]$(if (Test-Path Env:BATTLEPLAN_BENCH_PRIORITY) { $env:BATTLEPLAN_BENCH_PRIORITY } else { $env:HOI4_BENCH_PRIORITY }); Log "game priority $($(if (Test-Path Env:BATTLEPLAN_BENCH_PRIORITY) { $env:BATTLEPLAN_BENCH_PRIORITY } else { $env:HOI4_BENCH_PRIORITY }))" }
    $limit = [DateTime]::UtcNow.AddSeconds(300); $r = New-Object Hoi4ProbeInput+Rect
    do {
        $script:game.Refresh(); if ($script:game.HasExited) { throw 'The game exited during startup.' }
        $script:window = $script:game.MainWindowHandle
        $ok = $script:window -ne [IntPtr]::Zero -and [Hoi4ProbeInput]::GetWindowRect($script:window, [ref]$r) -and ($r.Right - $r.Left) -gt 0
        if (-not $ok) { Start-Sleep -Milliseconds 500 }
        if ([DateTime]::UtcNow -gt $limit) { throw 'The game window did not appear.' }
    } while (-not $ok)
    $script:reader = Open-GameClock $script:game.Id $clockRecorder $Out
    $script:rect = $r
    $w = $r.Right - $r.Left; $h = $r.Bottom - $r.Top
    if ($w -ne [int]$ui.width -or $h -ne [int]$ui.height) { throw "Game window is ${w}x${h}; UI calibration expects $($ui.width)x$($ui.height)." }
}
function Read-Clock([int]$timeoutSeconds = 10) {
    $limit = [DateTime]::UtcNow.AddSeconds($timeoutSeconds)
    do {
        $script:game.Refresh(); if ($script:game.HasExited) { throw 'The game exited.' }
        $clock = $script:reader.Sample()
        if ($null -ne $clock -and $null -ne $clock.paused -and $clock.tick_busy -eq 0) { return $clock }
        Start-Sleep -Milliseconds 10
    } while ([DateTime]::UtcNow -lt $limit)
    throw 'No stable authenticated game clock.'
}
function Invoke-Focused([scriptblock]$block) {
    for ($attempt = 1; ; $attempt++) {
        if ([Hoi4ProbeInput]::IsIconic($script:window)) { [void][Hoi4ProbeInput]::ShowWindow($script:window, 9) }
        $previous = [Hoi4ProbeInput]::AcquireTargetFocus($script:window, [uint32]$script:game.Id)
        try { Start-Sleep -Milliseconds 150; return (& $block) }
        catch {
            if ($attempt -ge 5 -or $_.Exception.Message -notmatch 'focus was lost') { throw }
            Log "Game focus was lost; retrying ($attempt)."
        }
        finally { [Hoi4ProbeInput]::RestoreForeground($previous, $script:window) }
        Start-Sleep -Milliseconds 400
    }
}
function Get-ClientPoint($point) {
    $client = New-Object Hoi4ProbeInput+Rect; $origin = New-Object Hoi4ProbeInput+Point
    if (-not [Hoi4ProbeInput]::GetClientRect($script:window, [ref]$client) -or -not [Hoi4ProbeInput]::ClientToScreen($script:window, [ref]$origin)) { throw 'Cannot locate the game client area.' }
    @(($script:rect.Left + [int]$point[0]) - $origin.X, ($script:rect.Top + [int]$point[1]) - $origin.Y)
}
function Send-Key([uint16]$vk) {
    if ($Background) { [Hoi4ProbeInput]::PostKey($script:window, $vk); Start-Sleep -Milliseconds 60; return }
    Invoke-Focused { [Hoi4ProbeInput]::KeyPress($script:window, [uint32]$script:game.Id, $vk) }
}
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
        for ($attempt = 1; ; $attempt++) {
            $flash = [Hoi4ProbeInput]::AcquireTargetFocus($script:window, [uint32]$script:game.Id)
            try { Start-Sleep -Milliseconds 120; [Hoi4ProbeInput]::Click($script:window, [uint32]$script:game.Id, $x, $y); return }
            catch {
                if ($attempt -ge 5 -or $_.Exception.Message -notmatch 'focus was lost') { throw }
                Log "Game focus was lost during a click; retrying ($attempt)."
            }
            finally { [Hoi4ProbeInput]::RestoreForeground($flash, $script:window); [void][Hoi4ProbeInput]::PushBehind($script:window, $flash) }
            Start-Sleep -Milliseconds 400
        }
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
function Take-Screenshot([string]$path) { try { $null = & $smoke -GameExe $GameExe -CaptureRoot $CaptureRoot -InputShimDir $InputShimDir -Action Screenshot -GameId $script:game.Id -Out $path -Foreground:(-not $Background) } catch { Log "screenshot failed: $($_.Exception.Message)" } }
function Set-Paused([bool]$paused) {
    for ($attempt = 0; $attempt -lt 8; $attempt++) {
        $clock = Read-Clock
        if ($clock.paused -eq [int]$paused) { return $clock }
        Send-Key 0x20
        $limit = [DateTime]::UtcNow.AddSeconds(3)
        do {
            Start-Sleep -Milliseconds 100
            $clock = Read-Clock
            if ($clock.paused -eq [int]$paused) { return $clock }
        } while ([DateTime]::UtcNow -lt $limit)
    }
    throw "Pause state did not become $paused."
}
function Set-Speed([int]$speed) {
    for ($attempt = 0; $attempt -lt 10; $attempt++) {
        $clock = Read-Clock
        if ($clock.speed -eq $speed) { return }
        $before = $clock.speed
        if ($before -lt $speed) { Send-Key 0x6B } else { Send-Key 0x6D }
        $limit = [DateTime]::UtcNow.AddSeconds(3)
        do {
            Start-Sleep -Milliseconds 100
            $clock = Read-Clock
        } while ($clock.speed -eq $before -and [DateTime]::UtcNow -lt $limit)
    }
    throw "Speed did not reach $speed."
}
function Assert-Identity($clock, $identity) {
    if ($clock.play_session_started_at -ne $identity.play_session_started_at -or $clock.seed -ne $identity.seed -or $clock.game_state -ne $identity.game_state) {
        throw 'Game identity (load signal, seed or state pointer) changed unexpectedly.'
    }
}
function Get-BuildProcesses { @(Get-Process -Name $buildNames -ErrorAction SilentlyContinue | ForEach-Object { $_.ProcessName }) }

$OtherCpuMachineShare = 0.08
$OtherCpuProcessCores = 1.0
$OtherCpuExcluded = @('the game', 'recorder', 'Idle', 'System')
function Get-OtherCpuSnapshot {
    $snapshot = @{}
    foreach ($process in (Get-Process -ErrorAction SilentlyContinue)) {
        if ($process.Id -eq $PID -or $OtherCpuExcluded -contains $process.ProcessName) { continue }
        try {
            $cpu = $process.TotalProcessorTime.TotalSeconds
            $started = $process.StartTime.ToUniversalTime().Ticks
        } catch { continue }
        $snapshot['{0}:{1}' -f $process.Id, $started] = [pscustomobject]@{ name = $process.ProcessName; id = $process.Id; cpu = [double]$cpu; started = [int64]$started }
    }
    $snapshot
}
function New-OtherCpu([hashtable]$snapshot, [int64]$windowStartTicks) {
    $tally = @{ window_start = $windowStartTicks; processes = @{} }
    Update-OtherCpu $tally $snapshot
    $tally
}
function Update-OtherCpu([hashtable]$tally, [hashtable]$snapshot) {
    foreach ($key in $snapshot.Keys) {
        $sample = $snapshot[$key]
        $known = $tally.processes[$key]
        if ($known) { $known.last = $sample.cpu; continue }
        $first = $(if ($sample.started -ge $tally.window_start) { 0.0 } else { $sample.cpu })
        $tally.processes[$key] = [pscustomobject]@{ name = $sample.name; id = $sample.id; first = $first; last = $sample.cpu }
    }
}
function Measure-OtherCpu([hashtable]$tally, [double]$elapsedSeconds, [int]$cores) {
    $used = @(@(foreach ($entry in $tally.processes.Values) {
        $seconds = [math]::Max(0.0, $entry.last - $entry.first)
        if ($seconds -gt 0) { [pscustomobject]@{ name = $entry.name; id = $entry.id; cpu_s = [math]::Round($seconds, 2) } }
    }) | Sort-Object cpu_s -Descending)
    $total = 0.0; foreach ($entry in $used) { $total += $entry.cpu_s }
    $share = $(if ($elapsedSeconds -gt 0 -and $cores -gt 0) { $total / ($elapsedSeconds * $cores) } else { 0.0 })
    $top = @($used | Select-Object -First 5)
    $reasons = New-Object Collections.ArrayList
    $describe = { param($entry) '{0}({1}) {2:n1}s' -f $entry.name, $entry.id, $entry.cpu_s }
    if ($share -gt $OtherCpuMachineShare) {
        [void]$reasons.Add(('other CPU {0:p1} of {1} cores: {2}' -f $share, $cores, (($top | ForEach-Object { & $describe $_ }) -join ', ')))
    }
    foreach ($entry in $used) {
        if ($elapsedSeconds -gt 0 -and $entry.cpu_s / $elapsedSeconds -gt $OtherCpuProcessCores) {
            [void]$reasons.Add(('{0} at {1:n2} cores' -f (& $describe $entry), ($entry.cpu_s / $elapsedSeconds)))
        }
    }
    [ordered]@{
        share = [math]::Round($share, 4); cpu_s = [math]::Round($total, 2); elapsed_s = [math]::Round($elapsedSeconds, 2); cores = $cores
        machine_share_limit = $OtherCpuMachineShare; process_cores_limit = $OtherCpuProcessCores
        top = @($top); reasons = @($reasons)
    }
}
function Get-RecorderPrivate($attached) {
    if (-not $attached -or $attached.process.HasExited) { return $null }
    try { $attached.process.Refresh(); return [int64]$attached.process.PrivateMemorySize64 } catch { return $null }
}
function Get-MemorySample($attached) {
    $script:game.Refresh()
    [ordered]@{ game_private = [int64]$script:game.PrivateMemorySize64; game_working_set = [int64]$script:game.WorkingSet64; recorder_private = (Get-RecorderPrivate $attached) }
}

function Invoke-Reload([string]$runDir, [bool]$attached = $false, [bool]$reference = $false) {
    Complete-Pending
    $fresh=$script:freshLaunch; $script:freshLaunch=$false
    $mode=$ReloadMode
    if ($spec.PSObject.Properties['reload_mode'] -and $spec.reload_mode) { $mode=[string]$spec.reload_mode }
    if ($mode -eq 'auto') { $mode='menu' }
    if ($mode -notin @('menu','auto')) { throw "Unsupported reload mode: $mode. Only the shared screen driver relaunches the game." }
    if ($fresh -and $reference) {
        $clock=Read-Clock
        if ($clock.hour -ne $expectedHour -or $clock.seed -ne $expectedSeed -or $clock.paused -ne 1 -or -not $clock.presentation_valid) { throw 'Fresh launch did not reach the expected paused save.' }
        return $clock
    }
    $reloadUi=$ui | ConvertTo-Json -Depth 10 | ConvertFrom-Json
    $reloadUi | Add-Member -NotePropertyName expected_save -NotePropertyValue ([string]$spec.save) -Force
    $reloadUi | Add-Member -NotePropertyName expected_hour -NotePropertyValue $expectedHour -Force
    $reloadUi | Add-Member -NotePropertyName expected_seed -NotePropertyValue $expectedSeed -Force
    $config=Join-Path $runDir 'reload-ui.json'
    Write-Json $config $reloadUi
    $reload=& $smoke -Action Reload -BinaryDirectory (Split-Path -Parent $clockRecorder) -GameExe $GameExe -UserDirectory $userPath -GameId $script:game.Id -GameLock $lock -LockOwner $LockOwner -UiConfig $config -MenuPatch $MenuPatch -MenuPatchOrigin $MenuPatchOrigin -InputShimDir $InputShimDir -CaptureRoot $CaptureRoot -ExpectedSave ([string]$spec.save) -Out $runDir
    Write-Json (Join-Path $runDir 'reload.json') $reload
    return $reload.Clock
}

function Convert-Win32Path([string]$path) {
    if ($path.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) { return '\\' + $path.Substring(8) }
    if ($path.StartsWith('\\?\', [StringComparison]::OrdinalIgnoreCase)) { return $path.Substring(4) }
    return $path
}
function Read-Status([string]$path) {
    for ($attempt = 0; $attempt -lt 8; $attempt++) {
        try {
            $stream = [IO.FileStream]::new($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
            try { $text = [IO.StreamReader]::new($stream, [Text.Encoding]::UTF8).ReadToEnd() } finally { $stream.Dispose() }
            return $text | ConvertFrom-Json
        } catch { Start-Sleep -Milliseconds 100 }
    }
    return $null
}
function Get-HookCost($before, $after) {
    if (-not $before.hook_timing -or -not $after.hook_timing) { return $null }
    $hours = [double]$after.hook_timing.families.hourly.calls - [double]$before.hook_timing.families.hourly.calls
    if ($hours -le 0) { return $null }
    $delta = { param($a, $b) if ($null -eq $a -or $null -eq $b) { $null } else { [double]$a - [double]$b } }
    $families = [ordered]@{}
    foreach ($property in $after.hook_timing.families.PSObject.Properties) {
        $a = $property.Value; $b = $before.hook_timing.families.($property.Name)
        if (-not $b) { continue }
        $detour = & $delta $a.estimated_detour_ns $b.estimated_detour_ns
        $original = & $delta $a.estimated_original_ns $b.estimated_original_ns
        $net = & $delta $a.approximate_net_detour_ns $b.approximate_net_detour_ns
        $families[$property.Name] = [ordered]@{
            kind = $a.kind
            calls_per_hour = [math]::Round((& $delta $a.calls $b.calls) / $hours, 1)
            detour_ms_per_hour = $(if ($null -ne $detour) { [math]::Round($detour / $hours / 1e6, 3) } else { $null })
            approximate_net_detour_ms_per_hour = $(if ($null -ne $net) { [math]::Round($net / $hours / 1e6, 3) } else { $null })
            original_ms_per_hour = $(if ($null -ne $original) { [math]::Round($original / $hours / 1e6, 3) } else { $null })
            capture_waits_per_hour = $(if ($null -ne $a.capture_waits -and $null -ne $b.capture_waits) { [math]::Round((& $delta $a.capture_waits $b.capture_waits) / $hours, 2) } else { $null })
        }
    }
    $capture = $(if ($before.capture -and $after.capture) { $after.capture } else { $null })
    $perHour = { param($a, $b, $scale) $d = & $delta $a $b; if ($null -ne $d) { [math]::Round($d / $hours / $scale, 3) } else { $null } }
    $gateWait = $(if ($capture) { & $perHour $capture.writer_wait_ns $before.capture.writer_wait_ns 1e6 } else { $null })
    $captures = $(if ($capture) { [ordered]@{
        placement = $capture.placement
        held_per_hour = & $perHour $capture.held $before.capture.held 1
        beside_per_hour = & $perHour $capture.beside $before.capture.beside 1
        beside_wall_ms_per_hour = & $perHour $capture.beside_ns $before.capture.beside_ns 1e6
        gate_wait_ms_per_hour = $gateWait
    } } else { $null })
    $threads = [ordered]@{}
    $cpuBefore = $before.collector_thread_cpu; $cpuAfter = $after.collector_thread_cpu
    if ($cpuBefore -and $cpuAfter) {
        foreach ($thread in @($cpuAfter.threads)) {
            $match = @($cpuBefore.threads) | Where-Object { $_.thread_id -eq $thread.thread_id -and $_.role -eq $thread.role } | Select-Object -First 1
            $cycles = $(if ($match) { & $delta $thread.cycle_ns $match.cycle_ns } else { $null })
            $name = $(if ($null -ne $thread.index) { '{0}-{1}' -f $thread.role, $thread.index } else { [string]$thread.role })
            $threads[$name] = $(if ($null -ne $cycles) { [math]::Round($cycles / $hours / 1e6, 3) } else { $null })
        }
    }
    $total = $(if ($cpuBefore -and $cpuAfter) { & $delta $cpuAfter.collector_total.cycle_ns $cpuBefore.collector_total.cycle_ns } else { $null })
    $captureThread = @(@($cpuAfter.threads) | Where-Object { $_.role -eq 'capture' }).Count -gt 0
    $process = $(if ($cpuBefore.process -and $cpuAfter.process) { & $delta $cpuAfter.process.cycle_ns $cpuBefore.process.cycle_ns } else { $null })
    $native = $(if ($families.Contains('hourly')) { $families['hourly'].original_ms_per_hour } else { $null })
    $direct = 0.0; $directNet = $null
    foreach ($entry in $families.GetEnumerator()) {
        if ($null -ne $entry.Value.detour_ms_per_hour) { $direct += [double]$entry.Value.detour_ms_per_hour }
        if ($null -ne $entry.Value.approximate_net_detour_ms_per_hour) { $directNet = [double]$directNet + [double]$entry.Value.approximate_net_detour_ms_per_hour }
    }
    if ($null -ne $gateWait) { $direct += [double]$gateWait; if ($null -ne $directNet) { $directNet += [double]$gateWait } }
    return [ordered]@{
        game_hours = $hours
        direct_ms_per_hour = [math]::Round($direct, 3)
        direct_approximate_net_ms_per_hour = $(if ($null -ne $directNet) { [math]::Round($directNet, 3) } else { $null })
        native_ms_per_hour = $native
        direct_fraction_of_native = $(if ($native) { [math]::Round($direct / [double]$native, 4) } else { $null })
        hook_ms_per_hour = $families
        captures = $captures
        thread_cpu_ms_per_hour = $threads
        collector_cpu_ms_per_hour = $(if ($null -ne $total) { [math]::Round($total / $hours / 1e6, 3) } else { $null })
        collector_cpu_includes_capture_thread = $(if ($null -ne $total) { $captureThread } else { $null })
        process_cpu_ms_per_hour = $(if ($null -ne $process) { [math]::Round($process / $hours / 1e6, 3) } else { $null })
    }
}
function Start-Candidate($candidate, [string]$runDir) {
    $diagnostic = [bool]($candidate.PSObject.Properties['diagnostic'] -and $candidate.diagnostic)
    if ($diagnostic -and [string]$candidate.name -notlike 'diag-*') { throw 'Diagnostic candidates must be named diag-*.' }
    $frozen = [string]$candidate.frozen
    $recorderExe = Join-Path $frozen 'recorder.exe'
    if (-not (Test-Path -LiteralPath $recorderExe) -or -not (Test-Path -LiteralPath (Join-Path $frozen 'collector.dll'))) { throw "Frozen candidate incomplete: $frozen" }
    if (@($script:game.Modules | Where-Object { $_.ModuleName -ieq 'collector.dll' }).Count -ne 0) { throw 'A collector is already loaded; detach it first.' }
    $recording = Join-Path $runDir 'recording'
    $null = New-Item -ItemType Directory -Force -Path $recording
    $previousSampler = $(if (Test-Path Env:BATTLEPLAN_WORKER_SAMPLER) { $env:BATTLEPLAN_WORKER_SAMPLER } else { $env:HOI4_WORKER_SAMPLER })
    try {
        if ($WorkerSampler) { $env:BATTLEPLAN_WORKER_SAMPLER = $WorkerSampler }
        $arguments = @('record', '--once', '--out', ('"' + $recording + '"'), '--user-dir', ('"' + $userPath + '"'))
        if ($candidate.PSObject.Properties['capture_mode']) { $arguments += @('--capture-mode', [string]$candidate.capture_mode) }
        if ($candidate.PSObject.Properties['recorder_args']) { $arguments += @($candidate.recorder_args | ForEach-Object { [string]$_ }) }
        $process = Invoke-WithRecorderCache $runDir {
            Start-Process -FilePath $recorderExe -ArgumentList $arguments `
                -WorkingDirectory $frozen -WindowStyle Hidden -PassThru `
                -RedirectStandardOutput (Join-Path $runDir 'recorder.stdout.log') -RedirectStandardError (Join-Path $runDir 'recorder.stderr.log')
        }
    } finally {
        $env:BATTLEPLAN_WORKER_SAMPLER = $previousSampler
    }
    $null = $process.Handle
    [void]$script:recorders.Add($process)
    if ($(if (Test-Path Env:BATTLEPLAN_BENCH_AFFINITY) { $env:BATTLEPLAN_BENCH_AFFINITY } else { $env:HOI4_BENCH_AFFINITY })) { try { $process.ProcessorAffinity = [IntPtr][Convert]::ToInt64($(if (Test-Path Env:BATTLEPLAN_BENCH_AFFINITY) { $env:BATTLEPLAN_BENCH_AFFINITY } else { $env:HOI4_BENCH_AFFINITY }), 16) } catch { Log "recorder affinity not set: $($_.Exception.Message)" } }
    if ($(if (Test-Path Env:BATTLEPLAN_BENCH_PRIORITY) { $env:BATTLEPLAN_BENCH_PRIORITY } else { $env:HOI4_BENCH_PRIORITY })) { try { $process.PriorityClass = [Diagnostics.ProcessPriorityClass]$(if (Test-Path Env:BATTLEPLAN_BENCH_PRIORITY) { $env:BATTLEPLAN_BENCH_PRIORITY } else { $env:HOI4_BENCH_PRIORITY }) } catch { Log "recorder priority not set: $($_.Exception.Message)" } }
    $attached = [pscustomobject]@{ process = $process; recording = $recording; status = $null; active = $null; frozen = $frozen; diagnostic = $diagnostic }
    $script:lastDiagnostic = $diagnostic
    try {
        $activePath = Join-Path $recording 'active.json'
        $limit = [DateTime]::UtcNow.AddSeconds($AttachTimeoutSeconds)
        $active = $null
        do {
            Start-Sleep -Milliseconds 100
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
            Start-Sleep -Milliseconds 100
            $status = Read-Status $attached.status
            if ($status -and $status.state -eq 'recording') { break }
            if ($process.HasExited) { throw 'Recorder exited during collector startup.' }
            if ([DateTime]::UtcNow -gt $limit) { throw 'Collector did not reach the recording state in time.' }
        } while ($true)
        $script:game.Refresh()
        if (@($script:game.Modules | Where-Object { $_.ModuleName -ieq 'collector.dll' }).Count -ne 1) { throw 'collector.dll is not loaded after attach.' }
    } catch {
        Log "attach failed ($($_.Exception.Message)); requesting detach"
        try { $null = Stop-Candidate $attached $runDir } catch { Log "detach after failed attach also failed: $($_.Exception.Message)" }
        throw
    }
    Log "Attached $($candidate.name); collector $((Get-FileHash -LiteralPath (Join-Path $frozen 'collector.dll') -Algorithm SHA256).Hash.Substring(0,12))"
    $attached
}
function Test-CollectorLoaded {
    $script:game.Refresh()
    try { return @($script:game.Modules | Where-Object { $_.ModuleName -ieq 'collector.dll' }).Count -ne 0 } catch { return $true }
}
function Get-ProgressLength($attached) {
    $session = $null
    if ($attached.active -and $attached.active.session_dir) { $session = Convert-Win32Path ([string]$attached.active.session_dir) }
    elseif ($attached.recording) {
        $sessions = Join-Path $attached.recording 'sessions'
        $newest = Get-ChildItem -LiteralPath $sessions -Directory -ErrorAction SilentlyContinue | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
        if ($newest) { $session = $newest.FullName }
    }
    if (-not $session) { return $null }
    $log = Get-Item -LiteralPath (Join-Path $session 'progress') -ErrorAction SilentlyContinue
    if ($log) { return [int64]$log.Length }
    return $null
}
function Wait-CollectorUnload {
    param(
        [scriptblock]$Loaded, [scriptblock]$ProgressLength, [scriptblock]$Exited,
        [scriptblock]$Now = { [DateTime]::UtcNow }, [scriptblock]$Sleep = { Start-Sleep -Milliseconds 250 },
        [int]$StallSeconds = 180, [int]$LimitSeconds = 900
    )
    $start = & $Now; $lastStep = $start; $length = & $ProgressLength
    while (& $Loaded) {
        if (& $Exited) { return $null }
        $at = & $Now
        $current = & $ProgressLength
        if ($current -ne $length) { $length = $current; $lastStep = $at }
        if (($at - $start).TotalSeconds -ge $LimitSeconds) { return "not unloaded within $LimitSeconds s of the detach request" }
        if ($null -ne $length -and ($at - $lastStep).TotalSeconds -ge $StallSeconds) { return "no collector detach step for $StallSeconds s" }
        & $Sleep
    }
    return $null
}
function Update-RecorderPeak($attached) {
    $private = Get-RecorderPrivate $attached
    if ($null -ne $private -and $private -gt $attached.recorder_private_peak_detach) { $attached.recorder_private_peak_detach = [int64]$private }
}
function Begin-Stop($attached, [string]$runDir) {
    if ($attached.PSObject.Properties['detach'] -and $attached.detach) { return }
    $recorderExe = Join-Path $attached.frozen 'recorder.exe'
    $detach = Invoke-WithRecorderCache $runDir {
        Start-Process -FilePath $recorderExe -ArgumentList @('detach', '--out', ('"' + $attached.recording + '"')) -WorkingDirectory $attached.frozen -WindowStyle Hidden -PassThru `
            -RedirectStandardOutput (Join-Path $runDir 'detach.stdout.log') -RedirectStandardError (Join-Path $runDir 'detach.stderr.log')
    }
    $null = $detach.Handle
    [void]$script:recorders.Add($detach)
    $attached | Add-Member -Force -NotePropertyName detach -NotePropertyValue $detach
    $attached | Add-Member -Force -NotePropertyName recorder_private_peak_detach -NotePropertyValue ([int64]0)
    $limit = [DateTime]::UtcNow.AddSeconds(900)
    while (Test-CollectorLoaded) {
        Update-RecorderPeak $attached
        if ($detach.HasExited -or $attached.process.HasExited) { break }
        if ([DateTime]::UtcNow -gt $limit) { throw 'collector.dll did not unload within 15 minutes of the detach request.' }
        Start-Sleep -Milliseconds 250
    }
}
function Stop-Candidate($attached, [string]$runDir) {
    Begin-Stop $attached $runDir
    $detach = $attached.detach
    $exitLimit = $null
    while (-not ($detach.HasExited -and $attached.process.HasExited)) {
        Update-RecorderPeak $attached
        if ($detach.HasExited -and $null -eq $exitLimit) { $exitLimit = [DateTime]::UtcNow.AddSeconds(180) }
        if ($exitLimit -and [DateTime]::UtcNow -gt $exitLimit) { throw 'Recorder did not exit after detach.' }
        Start-Sleep -Milliseconds 250
    }
    $detach.WaitForExit(); $attached.process.WaitForExit()
    $why = Wait-CollectorUnload -Loaded { Test-CollectorLoaded } -ProgressLength { Get-ProgressLength $attached } `
        -Exited { $script:game.HasExited }
    $script:game.Refresh()
    if (@($script:game.Modules | Where-Object { $_.ModuleName -ieq 'collector.dll' }).Count -ne 0) { throw "collector.dll remains loaded after detach ($why)." }
    if ($attached.diagnostic) {
        if ($detach.ExitCode -ne 0) { throw "Diagnostic detach command failed: exit $($detach.ExitCode)." }
        if ($attached.process.ExitCode -notin @(0, 1)) { throw "Diagnostic recorder exited abnormally: $($attached.process.ExitCode)." }
        return [ordered]@{ diagnostic_only = $true; recording_valid = $false;
            detach_exit = $detach.ExitCode; recorder_exit = $attached.process.ExitCode }
    }
    $completed = @(Get-ChildItem -LiteralPath $attached.recording -Filter completed.json -Recurse -ErrorAction SilentlyContinue)
    if ($detach.ExitCode -ne 0 -or $attached.process.ExitCode -ne 0) { throw "Recorder detach/finalization failed: detach exit $($detach.ExitCode), recorder exit $($attached.process.ExitCode)." }
    if ($completed.Count -ne 1) { throw "Expected one recording completion, found $($completed.Count)." }
    $summary = Get-Content -LiteralPath $completed[0].FullName -Raw | ConvertFrom-Json
    if ($summary.drain_complete -ne $true -or $summary.detached -ne $true -or $summary.partial -ne $false -or
        -not $summary.PSObject.Properties['issues'] -or $null -eq $summary.issues -or @($summary.issues).Count -ne 0) {
        throw 'Recording did not finish a complete, nonpartial drain with no issues.'
    }
    $status = Read-Status $attached.status
    if (-not $status -or $null -eq $status.dropped_frames -or $status.dropped_frames -ne 0 -or
        $null -eq $status.write_errors -or $status.write_errors -ne 0) {
        throw 'Final recording status is missing or reports dropped frames or write errors.'
    }
    Log 'Detached and finalized: complete drain, no drops or write errors.'
    return $summary
}
function Start-Ladder($candidate, [string]$runDir) {
    if (-not $LadderDir) { throw 'Ladder candidates require -LadderDir with ladder.dll and ladder-inject.exe.' }
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
    if (-not $MetricsTool -or -not (Test-Path -LiteralPath $MetricsTool -PathType Leaf)) { throw 'The required exact-metrics reader is missing.' }
    $replays = @(Get-ChildItem -LiteralPath $attached.recording -File -ErrorAction SilentlyContinue | Where-Object { $_.Extension -in '.battleplan' })
    if ($replays.Count -ne 1) { throw "Expected one finalized replay for exact metrics, found $($replays.Count)." }
    $expectedFrames = [int64]$lastHour - [int64]$firstHour
    if ($expectedFrames -le 0) { throw 'The measurement contains no completed game hours.' }
    $failures = @()
    foreach ($branch in @(1, 2, 3)) {
        try {
            $text = Invoke-WithRecorderCache $runDir {
                & $MetricsTool $replays[0].FullName $branch $firstHour ($lastHour - 1) 2>(Join-Path $runDir ('metrics_branch' + $branch + '.stderr.log')) | Out-String
                if ($LASTEXITCODE -ne 0) { throw "reader exit $LASTEXITCODE (see metrics_branch$branch.stderr.log)" }
            }
            $metrics = $text | ConvertFrom-Json
            if ($metrics.hours -ne $expectedFrames -or $metrics.observed_first -ne $firstHour -or
                $metrics.observed_last -ne ($lastHour - 1) -or $metrics.callback.samples -ne $expectedFrames) {
                throw "incomplete metrics for requested hours $firstHour..$($lastHour - 1): frames=$($metrics.hours), first=$($metrics.observed_first), last=$($metrics.observed_last)"
            }
            foreach ($field in @('mean_ns', 'p50_ns', 'p95_ns', 'max_ns')) {
                $value = $metrics.callback.$field
                if ($null -eq $value -or [double]::IsNaN([double]$value) -or [double]::IsInfinity([double]$value) -or [double]$value -lt 0) {
                    throw "callback metric $field is missing or invalid"
                }
            }
            if (-not $metrics.PSObject.Properties['failures'] -or $null -eq $metrics.failures -or @($metrics.failures.PSObject.Properties).Count -ne 0) {
                throw 'exact metrics lack table-failure coverage or report unavailable/truncated tables'
            }
            [IO.File]::WriteAllText((Join-Path $runDir ('exact_metrics_branch' + $branch + '.json')), $text, $utf8)
            return [pscustomobject]@{ branch = $branch; frames = $metrics.hours; callback_mean_ms = [math]::Round($metrics.callback.mean_ns / 1e6, 3);
                callback_p50_ms = [math]::Round($metrics.callback.p50_ns / 1e6, 3); callback_p95_ms = [math]::Round($metrics.callback.p95_ns / 1e6, 3);
                callback_max_ms = [math]::Round($metrics.callback.max_ns / 1e6, 3); pool_bytes_max = $metrics.pool_bytes_max;
                failures = 0 }
        } catch { $failures += "branch ${branch}: $($_.Exception.Message)" }
    }
    throw ('Exact callback metrics could not be verified; check that -MetricsTool supports this replay format. ' + ($failures -join '; '))
}
function Get-SizeSummary([string]$runDir) {
    $lines = @(Get-Content -LiteralPath (Join-Path $runDir 'recorder.stdout.log') -Encoding UTF8)
    $size = [ordered]@{ stop_to_save_ready_s = $null; raw_encoded_bytes = $null; raw_bytes_per_hour = $null; spool_bytes_per_hour = $null; replay_bytes_per_hour = $null }
    $tables = New-Object Collections.ArrayList; $inTables = $false; $finalizations = 0; $deferred = $false; $ready = $null
    foreach ($line in $lines) {
        if ($line -match '^finalize_s=([\d.]+|not_observed) replay_bytes=(\d+) spool_bytes=(\d+)$') {
            $finalizations++; $size.finalize_s = if ($Matches[1] -eq 'not_observed') { $null } else { [double]$Matches[1] }
            $size.replay_bytes = [int64]$Matches[2]; $size.spool_bytes = [int64]$Matches[3]; continue
        }
        if ($line -match '^Stop to save-ready: (\{.*\})$') {
            $ready = $Matches[1] | ConvertFrom-Json
            $size.stop_to_save_ready_s = $ready.stop_to_save_ready_s
            $size.stop_diagnostics = $ready
            continue
        }
        if ($line -match '^hours=(\d+) total_p50_ms=(\S+)') {
            $size.hours = [int64]$Matches[1]
            $deferred = $Matches[2] -in @('not_observed', 'not_summarized')
            continue
        }
        if ($line -match '^table raw_encoded_bytes ') { $inTables = $true; continue }
        if (-not $inTables) { continue }
        $f = $line -split ' '
        if ($f.Count -ne 11 -or $f[1] -notmatch '^\d+$') { $inTables = $false; continue }
        [void]$tables.Add([ordered]@{ table = $f[0]; raw_bytes = [int64]$f[1]; read_p50_ms = [double]$f[3]; read_p95_ms = [double]$f[4]; read_max_ms = [double]$f[5]
            snapshots = [int64]$f[6]; deltas = [int64]$f[7]; unchanged = [int64]$f[8]; truncated = [int64]$f[9]; unavailable = [int64]$f[10] })
    }
    if ($finalizations -ne 1 -or -not $size.Contains('hours') -or (-not $deferred -and ($size.hours -le 0 -or $tables.Count -eq 0))) {
        throw "Finalization summary missing or ambiguous in recorder.stdout.log (finalizations=$finalizations, tables=$($tables.Count))."
    }
    if ($size.spool_bytes -le 0 -or $size.replay_bytes -le 0) { throw 'Finalization summary reports zero bytes.' }
    if ($deferred) {
        if (-not $ready -or $ready.save_ready -ne $true) { throw 'Deferred statistics lack a durable save-ready status.' }
        $size.summary_scope = 'known evidence; full capture statistics deferred to offline inspection'
        [IO.File]::WriteAllText((Join-Path $runDir 'tables.json'), '[]', $utf8)
    } else {
        $raw = [int64]0; foreach ($t in $tables) { $raw += $t.raw_bytes }
        if ($raw -le 0) { throw 'Finalization summary reports zero raw bytes.' }
        Write-Json (Join-Path $runDir 'tables.json') @($tables)
        $size.raw_encoded_bytes = $raw
        $size.raw_bytes_per_hour = [math]::Round($raw / $size.hours, 1)
        $size.spool_bytes_per_hour = [math]::Round($size.spool_bytes / $size.hours, 1)
        $size.replay_bytes_per_hour = [math]::Round($size.replay_bytes / $size.hours, 1)
    }
    $size
}

function Invoke-PlanConsole {
    if (-not ($spec.PSObject.Properties['console'] -and $spec.console)) { return }
    foreach ($line in @($spec.console)) {
        Log "console: $line"
        $null = & $smoke -GameExe $GameExe -CaptureRoot $CaptureRoot -InputShimDir $InputShimDir -Action Console -GameId $script:game.Id -Text ([string]$line) -Foreground
    }
}
function Send-MenuInput([hashtable]$extra) {
    $null = & $smoke -GameExe $GameExe -CaptureRoot $CaptureRoot -InputShimDir $InputShimDir -UserDirectory $userPath -GameId $script:game.Id -Foreground -KeepForeground @extra
}
function Save-Game($clock) {
    $name = $(if ($spec.PSObject.Properties['save_name'] -and $spec.save_name) { [string]$spec.save_name } else { "BENCH_H$($clock.hour)" })
    if ($name -match '\.hoi4$') { $name = $name.Substring(0, $name.Length - 5) }
    if (-not $name -or $name -ne [IO.Path]::GetFileName($name) -or $name.IndexOfAny([IO.Path]::GetInvalidFileNameChars()) -ge 0) { throw "save_name must be a plain file name: $name" }
    $path = Join-Path (Join-Path $userPath 'save games') ($name + '.hoi4')
    if (Test-Path -LiteralPath $path) { throw "The plan's save already exists: $path" }
    $paused = Set-Paused $true
    $pausedAt = [Diagnostics.Stopwatch]::GetTimestamp()
    Log "Saving at hour $($paused.hour) as $name."
    $previous = [Hoi4ProbeInput]::AcquireTargetFocus($script:window, [uint32]$script:game.Id)
    try {
        foreach ($widget in 'menu', 'save_menu', 'save_name') {
            $point = $ui.$widget
            Send-MenuInput @{ Action = 'Click'; X = [int]$point[0]; Y = [int]$point[1] }
            Start-Sleep -Milliseconds 600
        }
        Send-MenuInput @{ Action = 'Key'; Key = 'CtrlA' }
        Send-MenuInput @{ Action = 'Text'; Text = $name }
        $point = $ui.save_confirm
        Send-MenuInput @{ Action = 'Click'; X = [int]$point[0]; Y = [int]$point[1] }
        $limit = [DateTime]::UtcNow.AddSeconds(120); $complete = $false
        do {
            Start-Sleep -Milliseconds 250
            if (Test-Path -LiteralPath $path) {
                try {
                    $stream = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
                    try { $complete = $stream.Length -gt 10000 } finally { $stream.Dispose() }
                } catch { $complete = $false }
            }
            if (-not $complete -and [DateTime]::UtcNow -gt $limit) { throw "The game did not write a complete save: $path" }
        } until ($complete)
    } finally {
        [Hoi4ProbeInput]::RestoreForeground($previous, $script:window)
        if ($Background) { [void][Hoi4ProbeInput]::PushBehind($script:window, $previous) }
    }
    if ((Read-Clock).hour -ne $paused.hour) { throw 'The game advanced during the save.' }
    Send-Move $ui.cursor_park
    $resumedAt = [Diagnostics.Stopwatch]::GetTimestamp()
    $null = Set-Paused $false; Set-Speed $speed
    Log "Saved $path."
    [pscustomobject]@{ path = $path; paused_ticks = $resumedAt - $pausedAt }
}
function Invoke-Screen($attached, [string]$runDir) {
    $identity = Set-Paused $true
    if ($identity.hour -ne $expectedHour) { throw "Screen must start at hour $expectedHour, found $($identity.hour)." }
    Set-Speed $speed; Send-Move $ui.cursor_park
    $statusBefore = $null
    if ($attached) { $statusBefore = Read-Status $attached.status; Write-Json (Join-Path $runDir 'status_before.json') $statusBefore }
    $log = [IO.StreamWriter]::new((Join-Path $runDir 'samples.jsonl'), $false, $utf8)
    $contaminants = @{}
    $first = $null; $last = $null; $firstQpc = 0L; $lastQpc = 0L; $valid = 0; $missing = 0
    $firstCpuMs = 0.0; $lastCpuMs = 0.0
    $gameCpuMs = { $script:game.Refresh(); $script:game.TotalProcessorTime.TotalMilliseconds }
    $gameFaults = { try { [int64](Get-CimInstance Win32_Process -Filter "ProcessId=$($script:game.Id)" -ErrorAction Stop).PageFaults } catch { $null } }
    $firstFaults = $null
    $saveCpuMs = 0.0; $saveKernelMs = 0.0; $saveFaults = 0L
    $gameKernelMs = { $script:game.PrivilegedProcessorTime.TotalMilliseconds }
    $firstKernelMs = 0.0; $lastKernelMs = 0.0
    $memoryStart = Get-MemorySample $attached
    $memorySeries = New-Object Collections.ArrayList
    [void]$memorySeries.Add(@([int64]$identity.hour, $memoryStart.game_private))
    $gamePeak = $memoryStart.game_private; $workingSetPeak = $memoryStart.game_working_set; $memoryEnd = $memoryStart
    $recorderPeak = $(if ($null -ne $memoryStart.recorder_private) { $memoryStart.recorder_private } else { 0L })
    $otherCpu = New-OtherCpu (Get-OtherCpuSnapshot) ([DateTime]::UtcNow.Ticks)
    $nextOtherCpu = $null
    Log 'Timed window started.'
    $null = Set-Paused $false
    $startQpc = [Diagnostics.Stopwatch]::GetTimestamp()
    $frequency = [double]$script:reader.Frequency
    $nextGuard = $startQpc
    $nextMemory = $startQpc + [int64]$frequency
    $nextOtherCpu = $startQpc + 5 * [int64]$frequency
    $statusEvery = $(if ($attached -and $spec.PSObject.Properties['status_every_s'] -and $spec.status_every_s) { [double]$spec.status_every_s } else { 0 })
    $statusSeries = $(if ($statusEvery -gt 0) { [IO.StreamWriter]::new((Join-Path $runDir 'status_series.jsonl'), $false, $utf8) } else { $null })
    $nextStatus = $startQpc
    $saveAt = $(if (-not $script:planSaved -and $spec.PSObject.Properties['save_at_hour'] -and $spec.save_at_hour) { [int64]$spec.save_at_hour } else { 0L })
    $savedTo = $null; $pausedTicks = 0L
    $stopReason = 'duration_limit'
    try {
        while (([Diagnostics.Stopwatch]::GetTimestamp() - $pausedTicks - $startQpc) / $frequency -lt $seconds) {
            if ($null -ne $last -and ([int64]$last.hour - [int64]$first.hour) -ge $maxHours) { $stopReason = 'max_hours'; break }
            $qpcNow = [Diagnostics.Stopwatch]::GetTimestamp()
            if ($qpcNow -ge $nextGuard) {
                if ($attached -and $attached.diagnostic -and $attached.process.HasExited) { throw 'Diagnostic recorder exited during the timed window.' }
                foreach ($name in (Get-BuildProcesses)) { $contaminants[$name] = $true }
                $nextGuard = $qpcNow + [int64]$frequency
            }
            if ($qpcNow -ge $nextOtherCpu) {
                Update-OtherCpu $otherCpu (Get-OtherCpuSnapshot)
                $nextOtherCpu = $qpcNow + 5 * [int64]$frequency
            }
            $clock = $script:reader.Sample()
            $wallQpc = [Diagnostics.Stopwatch]::GetTimestamp()
            $qpc = $wallQpc - $pausedTicks
            if ($null -eq $clock) { $missing++ }
            else {
                Assert-Identity $clock $identity
                if ($clock.paused -ne 0 -or $clock.speed -ne $speed) { throw 'Speed or pause changed during the timed window.' }
                if ($null -ne $last -and $clock.hour -lt $last.hour) { throw 'Backwards clock.' }
                $valid++
                if ($null -eq $first) { $first = $clock; $firstQpc = $qpc; $firstCpuMs = & $gameCpuMs; $firstKernelMs = & $gameKernelMs; $firstFaults = & $gameFaults }
                if ($null -eq $last -or $clock.hour -ne $last.hour) { $lastCpuMs = & $gameCpuMs; $lastKernelMs = & $gameKernelMs }
                $last = $clock; $lastQpc = $qpc
                if ($saveAt -and -not $savedTo -and $clock.hour -ge $saveAt) {
                    $cpuBefore = & $gameCpuMs; $kernelBefore = & $gameKernelMs; $faultsBefore = & $gameFaults
                    $save = Save-Game $clock
                    $saveCpuMs += (& $gameCpuMs) - $cpuBefore; $saveKernelMs += (& $gameKernelMs) - $kernelBefore
                    $faultsAfter = & $gameFaults
                    $saveFaults = $(if ($null -ne $faultsBefore -and $null -ne $faultsAfter -and $null -ne $saveFaults) { $saveFaults + $faultsAfter - $faultsBefore } else { $null })
                    $savedTo = $save.path
                    $script:planSaved = $true
                    $pausedTicks += $save.paused_ticks
                }
            }
            $entry = [ordered]@{ qpc = $wallQpc; elapsed_s = ($qpc - $startQpc) / $frequency; hour = $(if ($clock) { $clock.hour } else { $null }) }
            if ($qpc -ge $nextMemory -and $clock) {
                $nextMemory = $qpc + [int64]$frequency
                $memory = Get-MemorySample $attached
                foreach ($key in $memory.Keys) { $entry[$key] = $memory[$key] }
                [void]$memorySeries.Add(@([int64]$clock.hour, $memory.game_private))
                $gamePeak = [math]::Max($gamePeak, $memory.game_private); $workingSetPeak = [math]::Max($workingSetPeak, $memory.game_working_set)
                if ($null -ne $memory.recorder_private) { $recorderPeak = [math]::Max($recorderPeak, $memory.recorder_private) }
                $memoryEnd = $memory
            }
            $log.WriteLine(($entry | ConvertTo-Json -Compress))
            if ($statusSeries -and $qpc -ge $nextStatus) {
                $nextStatus = $qpc + [int64]($statusEvery * $frequency)
                $status = $null
                try { $status = Read-Status $attached.status } catch { }
                if ($status) {
                    $statusSeries.WriteLine(([ordered]@{
                        elapsed_s = $entry.elapsed_s; hour = $entry.hour
                        callback_work = $status.callback_work; native_tick_work = $status.native_tick_work
                        callback_histogram = $status.callback_tail.histogram
                        acquisition_work = $status.acquisition_work
                        callback_profile = $status.callback_profile
                        spool_bytes = $status.spool_bytes; pool_bytes = $status.pool_bytes
                        dropped_frames = $status.dropped_frames; write_errors = $status.write_errors
                    } | ConvertTo-Json -Compress -Depth 16))
                    $statusSeries.Flush()
                }
            }
            Start-Sleep -Milliseconds 125
        }
    } finally {
        $null = Set-Paused $true
        $log.Dispose()
        if ($statusSeries) { $statusSeries.Dispose() }
    }
    $lastFaults = & $gameFaults
    Update-OtherCpu $otherCpu (Get-OtherCpuSnapshot)
    $otherCpuWindow = Measure-OtherCpu $otherCpu (([Diagnostics.Stopwatch]::GetTimestamp() - $startQpc) / $frequency) ([Environment]::ProcessorCount)
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
        requested_seconds = $seconds; max_hours = $maxHours; stop_reason = $stopReason
        hours_per_s = [math]::Round($rate, 4)
        game_cpu_ms_per_hour = $(if ($hours -gt 0) { [math]::Round(($lastCpuMs - $firstCpuMs - $saveCpuMs) / $hours, 3) } else { $null })
        game_kernel_ms_per_hour = $(if ($hours -gt 0) { [math]::Round(($lastKernelMs - $firstKernelMs - $saveKernelMs) / $hours, 3) } else { $null })
        game_page_faults_per_hour = $(if ($hours -gt 0 -and $null -ne $firstFaults -and $null -ne $lastFaults -and $null -ne $saveFaults) { [math]::Round(($lastFaults - $firstFaults - $saveFaults) / $hours, 1) } else { $null })
        historical_baseline_fraction = $(if ($historicalBaselineRate) { [math]::Round($rate / $historicalBaselineRate, 4) } else { $null })
        valid_samples = $valid; missing_samples = $missing; contaminated_by = @($contaminants.Keys)
        saved_game = $savedTo; save_pause_s = [math]::Round($pausedTicks / $frequency, 3)
        other_cpu = $otherCpuWindow
        load_signal = $identity.play_session_started_at
        memory = [ordered]@{
            game_private_start = $memoryStart.game_private; game_private_peak = [int64]$gamePeak; game_private_end = $memoryEnd.game_private
            game_working_set_peak = [int64]$workingSetPeak; recorder_private_peak_window = $(if ($attached) { [int64]$recorderPeak } else { $null })
        }
        memory_series = @($memorySeries)
    }
    if ($statusBefore -and $statusAfter) {
        $calls = [int64]$statusAfter.callback_work.calls - [int64]$statusBefore.callback_work.calls
        $ticks = [int64]$statusAfter.native_tick_work.calls - [int64]$statusBefore.native_tick_work.calls
        $beforeHistogram = @($statusBefore.callback_tail.histogram)
        $afterHistogram = @($statusAfter.callback_tail.histogram)
        $callbackHistogram = @(for ($bucket = 0; $bucket -lt $afterHistogram.Count; $bucket++) {
            $beforeCount = if ($bucket -lt $beforeHistogram.Count) { [int64]$beforeHistogram[$bucket] } else { 0 }
            [int64]$afterHistogram[$bucket] - $beforeCount
        })
        $result.status_delta = [ordered]@{
            frames = $calls
            callback_mean_ms = $(if ($calls -gt 0) { [math]::Round(([double]$statusAfter.callback_work.total_ns - [double]$statusBefore.callback_work.total_ns) / $calls / 1e6, 3) } else { $null })
            native_tick_mean_ms = $(if ($ticks -gt 0) { [math]::Round(([double]$statusAfter.native_tick_work.elapsed_ns - [double]$statusBefore.native_tick_work.elapsed_ns) / $ticks / 1e6, 3) } else { $null })
            session_callback_p50_ms = [math]::Round([double]$statusAfter.callback_work.p50_ns / 1e6, 3)
            session_callback_p95_ms = [math]::Round([double]$statusAfter.callback_work.p95_ns / 1e6, 3)
            session_callback_p99_ms = [math]::Round([double]$statusAfter.callback_work.p99_ns / 1e6, 3)
            session_callback_max_ms = [math]::Round([double]$statusAfter.callback_work.max_ns / 1e6, 3)
            callback_histogram = $callbackHistogram
            pool_bytes = $statusAfter.pool_bytes
            dropped_frames = $statusAfter.dropped_frames; write_errors = $statusAfter.write_errors
            collector_cost = Get-HookCost $statusBefore $statusAfter
        }
    }
    return $result
}

$resultsPath = Join-Path $Out 'results.jsonl'
$rows = @()
if ($SummarizeOnly) {
    $rows = @(Get-Content -LiteralPath $resultsPath -Encoding UTF8 | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json })
    foreach ($row in $rows) {
        if (-not ($row.PSObject.Properties['status_delta'] -and $row.status_delta)) { continue }
        $beforePath = Join-Path $row.run_dir 'status_before.json'; $afterPath = Join-Path $row.run_dir 'status_after.json'
        if (-not ((Test-Path -LiteralPath $beforePath) -and (Test-Path -LiteralPath $afterPath))) { continue }
        $cost = Get-HookCost (Get-Content -LiteralPath $beforePath -Raw | ConvertFrom-Json) (Get-Content -LiteralPath $afterPath -Raw | ConvertFrom-Json)
        if ($cost) { $row.status_delta | Add-Member -NotePropertyName collector_cost -NotePropertyValue $cost -Force }
    }
    $repeats = 0
} else {
    Connect-Game
    $spec.candidates | ForEach-Object { if ($_.frozen -and -not (Test-Path -LiteralPath (Join-Path ([string]$_.frozen) 'collector.dll'))) { throw "Candidate $($_.name): collector.dll missing under $($_.frozen)" } }
}
Log ("Plan: {0} candidates x {1} repeats, screens capped at {2}s or {3} hours, save {4} ({5})" -f @($spec.candidates).Count, $repeats, $seconds, $maxHours, $spec.save, $saveSha.Substring(0, [math]::Min(12, $saveSha.Length)))
$rowDicts = New-Object Collections.ArrayList
function Save-Row($row) {
    $row.finished_utc = [DateTime]::UtcNow.ToString('o')
    Write-Json (Join-Path $row.run_dir 'result.json') $row
    [IO.File]::AppendAllText($resultsPath, (($row | ConvertTo-Json -Depth 20 -Compress) + "`n"), $utf8)
    [void]$rowDicts.Add($row)
}
$script:pending = $null
function Complete-Pending {
    $p = $script:pending
    if (-not $p) { return }
    $script:pending = $null
    try {
        $p.row.completion = Stop-Candidate $p.attached $p.row.run_dir
        $p.row.memory.recorder_private_peak_detach = $p.attached.recorder_private_peak_detach
        if ($p.attached.diagnostic) {
            $p.row.diagnostic_only = $true
            $p.row.recording_valid = $false
            $p.row.performance_screen_clean = $p.passed
            $p.row.passed = $false
        } else {
            $p.row.size = Get-SizeSummary $p.row.run_dir
            $p.row.passed = $p.passed
        }
    } catch {
        $p.row.error = $_.Exception.Message
        Log "FAILED $($p.row.name) finalization: $($_.Exception.Message)"
    }
    Save-Row $p.row
}
$alternate = [bool]($spec.PSObject.Properties['alternate'] -and $spec.alternate)
for ($rep = 1; $rep -le $repeats; $rep++) {
    $index = 0
    $order = @($spec.candidates)
    if ($alternate -and $rep % 2 -eq 0) {
        $reversed = @($order | Where-Object { $_.frozen }); [array]::Reverse($reversed)
        $order = @($order | Where-Object { -not $_.frozen }) + $reversed
    }
    foreach ($candidate in $order) {
        $index++
        $name = [string]$candidate.name
        $runDir = Join-Path $Out ("r{0:d2}_{1:d2}_{2}" -f $rep, $index, $name)
        $null = New-Item -ItemType Directory -Force -Path $runDir
        $row = [ordered]@{ name = $name; repeat = $rep; frozen = $candidate.frozen; run_dir = $runDir; started_utc = [DateTime]::UtcNow.ToString('o'); passed = $false }
        $row.capture_mode = $(if ($candidate.PSObject.Properties['capture_mode']) { [string]$candidate.capture_mode } else { $null })
        if ($candidate.PSObject.Properties['diagnostic'] -and $candidate.diagnostic) {
            $row.diagnostic_only = $true; $row.recording_valid = $false; $row.performance_screen_clean = $false
        }
        $attached = $null; $ladder = $null
        try { (Get-Item -LiteralPath $lock).LastWriteTime = Get-Date } catch { Log "could not refresh the game lock time: $($_.Exception.Message)" }
        try {
            Log "=== $name (repeat $rep) ==="
            if ($candidate.PSObject.Properties['ladder'] -and $candidate.ladder) {
                $null = Invoke-Reload $runDir $false
                Complete-Pending
                $ladder = Start-Ladder $candidate $runDir
                $row.ladder_config = $ladder.config
            } elseif ($candidate.frozen) {
                $row.collector_sha256 = (Get-FileHash -LiteralPath (Join-Path ([string]$candidate.frozen) 'collector.dll') -Algorithm SHA256).Hash
                $null = Invoke-Reload $runDir $false
                Invoke-PlanConsole
                Complete-Pending
                $attached = Start-Candidate $candidate $runDir
                $row.recording = $attached.recording
            } else {
                $null = Invoke-Reload $runDir $false $true
                Invoke-PlanConsole
                Complete-Pending
            }
            $screen = Invoke-Screen $attached $runDir
            foreach ($key in $screen.Keys) { $row[$key] = $screen[$key] }
            $passed = ($screen.contaminated_by.Count -eq 0 -and -not $screen.camera_moved)
            if ($ladder) {
                $row.ladder = Stop-Ladder $ladder $runDir
                $ladder = $null
            }
            if ($screen.camera_moved) { Log 'WARNING: the camera moved during the timed window (edge scroll?); run flagged.' }
            Log ("{0}: {1} h/s ({2} hours / {3}s; stopped at {4}){5}" -f $name, $screen.hours_per_s, $screen.hours, $screen.elapsed_s, $screen.stop_reason, $(if ($screen.contaminated_by.Count) { ' CONTAMINATED by ' + ($screen.contaminated_by -join ',') } else { '' }))
            if ($attached) {
                Begin-Stop $attached $runDir
                $script:pending = [pscustomobject]@{ attached = $attached; row = $row; passed = $passed }
                $attached = $null
                continue
            }
            $row.passed = $passed
        } catch {
            $row.error = $_.Exception.Message
            Log "FAILED ${name}: $($_.Exception.Message)"
            Take-Screenshot (Join-Path $runDir 'failure.png')
            if ($attached) { try { $null = Stop-Candidate $attached $runDir } catch { Log "detach after failure also failed: $($_.Exception.Message)" } }
            if ($ladder) { try { $null = Stop-Ladder $ladder $runDir } catch { Log "ladder detach after failure also failed: $($_.Exception.Message)" } }
            try { $null = Set-Paused $true } catch { }
            Complete-Pending
        }
        Save-Row $row
    }
}
Complete-Pending
if ($script:reader) { $script:reader.Dispose(); $script:reader=$null }
foreach ($row in $rowDicts) {
    if ($row.Contains('diagnostic_only') -and $row.diagnostic_only) { continue }
    if (-not $row.frozen -or -not $row.Contains('size') -or -not $row.size -or $row.Contains('exact')) { continue }
    try {
        $row.exact = Get-ExactMetrics ([pscustomobject]@{ recording = $row.recording }) $row.run_dir ([int]$row.first_hour) ([int]$row.last_hour)
        Write-Json (Join-Path $row.run_dir 'result.json') $row
        if (-not ($spec.PSObject.Properties['keep_spools'] -and $spec.keep_spools)) {
            Get-ChildItem -LiteralPath $row.recording -Recurse -Filter 'spool.*.h4s' -File -ErrorAction SilentlyContinue | Remove-Item -Force
        }
    } catch {
        $row.error = $_.Exception.Message; $row.passed = $false
        Log "FAILED $($row.name) exact metrics: $($_.Exception.Message)"
    }
}
if (-not $SummarizeOnly) {
    [IO.File]::WriteAllText($resultsPath, (($rowDicts | ForEach-Object { $_ | ConvertTo-Json -Depth 20 -Compress }) -join "`n") + "`n", $utf8)
    $rows = @($rowDicts | ForEach-Object { [pscustomobject]$_ })
}
$sameRunRates = @($rows | Where-Object { $_.passed -and -not $_.frozen -and -not ($_.PSObject.Properties['ladder_config'] -and $_.ladder_config) -and $_.PSObject.Properties['hours_per_s'] } | ForEach-Object { [double]$_.hours_per_s })
$sameRunBaseline = if ($sameRunRates.Count) { ($sameRunRates | Measure-Object -Average).Average } else { $null }
$table = foreach ($r in $rows) {
    $sameRun = if ($sameRunBaseline -and $r.PSObject.Properties['hours_per_s'] -and $r.hours_per_s) { '{0:p1}' -f ([double]$r.hours_per_s / $sameRunBaseline) } else { '' }
    $extraMs = if ($sameRunBaseline -and $r.PSObject.Properties['hours_per_s'] -and $r.hours_per_s) { '{0:n2}' -f (1000.0 / [double]$r.hours_per_s - 1000.0 / $sameRunBaseline) } else { '' }
    $exactText = if ($r.PSObject.Properties['exact'] -and $r.exact) { "{0}/{1}/{2}" -f $r.exact.callback_mean_ms, $r.exact.callback_p50_ms, $r.exact.callback_p95_ms } else { '' }
    $deltaCallback = if ($r.PSObject.Properties['status_delta'] -and $r.status_delta) { $r.status_delta.callback_mean_ms } elseif ($r.PSObject.Properties['ladder'] -and $r.ladder) { "{0} (p95 {1})" -f $r.ladder.own_mean_ms, $r.ladder.own_p95_ms } else { '' }
    $deltaTick = if ($r.PSObject.Properties['status_delta'] -and $r.status_delta) { $r.status_delta.native_tick_mean_ms } elseif ($r.PSObject.Properties['ladder'] -and $r.ladder) { "{0} (p95 {1})" -f $r.ladder.tick_mean_ms, $r.ladder.tick_p95_ms } else { '' }
    $cost = if ($r.PSObject.Properties['status_delta'] -and $r.status_delta) { $r.status_delta.collector_cost } else { $null }
    $directText = if ($cost -and $null -ne $cost.direct_fraction_of_native) { '{0:n2} ({1:p1})' -f $cost.direct_ms_per_hour, [double]$cost.direct_fraction_of_native } else { '' }
    $cpuText = if ($cost -and $null -ne $cost.collector_cpu_ms_per_hour) { '{0:n1}{1}' -f $cost.collector_cpu_ms_per_hour, $(if ($cost.collector_cpu_includes_capture_thread -eq $false) { ' (without capture thread)' } else { '' }) } else { '' }
    $captureText = if ($cost -and $cost.captures) { '{0}/{1}' -f $cost.captures.held_per_hour, $cost.captures.beside_per_hour } else { '' }
    $err = if ($r.PSObject.Properties['error']) { $r.error } else { '' }
    "| {0} | {1} | {2} | {3} | {4} | {5} | {6} | {7} | {8} | {9} | {10} | {11} | {12} | {13} | {14} | {15} |" -f $r.name, $r.repeat, $(if ($r.PSObject.Properties['hours_per_s']) { $r.hours_per_s } else { '' }), $sameRun, $extraMs, $(if ($r.PSObject.Properties['historical_baseline_fraction'] -and $null -ne $r.historical_baseline_fraction) { ('{0:p1}' -f $r.historical_baseline_fraction) } else { '' }), $deltaCallback, $deltaTick, $directText, $cpuText, $captureText, $exactText, $(if ($r.PSObject.Properties['elapsed_s']) { $r.elapsed_s } else { '' }), $(if ($r.PSObject.Properties['stop_reason']) { $r.stop_reason } else { '' }), $(if ($r.PSObject.Properties['contaminated_by'] -and $r.contaminated_by.Count) { (@($r.contaminated_by) -join '; ') -replace '\|', '/' } else { '' }), $err
}
$baselineText = if ($sameRunBaseline) { ('same-run no-recorder baseline {0:n3} h/s (mean of {1})' -f $sameRunBaseline, $sameRunRates.Count) } else { 'no same-run baseline' }
$historicalText = if ($historicalBaselineRate) { "Optional historical reference $historicalBaselineRate h/s; it is not the matched baseline." } else { 'No historical reference supplied.' }
$md = @("# Benchmark $(Split-Path -Leaf $Out)", '', "Save $($spec.save) (SHA256 $saveSha), screens capped at ${seconds}s or $maxHours game hours at native speed $speed; $baselineText. $historicalText", '',
    'Elapsed time and stop reason are reported per screen. max_hours runs are shorter diagnostic screens, including when requested_seconds is 120. This report does not establish correctness or performance acceptance.', '',
    'Direct ms/game-hour is the game thread''s time in the recorder (the hourly pause, in-tick hooks, and waits at the capture gate while a capture reads beside the game), with its share of the game''s own hourly update timed in the same run: steady under contention, but blind to the game''s own code running slower beside the recorder. Recorder CPU is the collector threads'' cycle time per game hour, the capture thread''s included where the collector gives it a role. Captures per game hour are counted as held on the game thread and read beside it.', '',
    '| candidate | rep | h/s | % same-run baseline | extra ms/game-hour | % historical reference | callback own/mean ms | native tick mean ms | direct ms/game-hour (of native) | recorder CPU ms/game-hour | captures held/beside per game-hour | exact callback mean/p50/p95 ms | elapsed s | stop reason | contaminated | error |',
    '|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|') + $table

function Get-Interval($values) {
    $list = @($values | Where-Object { $null -ne $_ } | ForEach-Object { [double]$_ })
    if ($list.Count -eq 0) { return $null }
    $mean = ($list | Measure-Object -Average).Average
    if ($list.Count -lt 2) { return [pscustomobject]@{ mean = $mean; half = $null; n = 1 } }
    $ss = 0.0; foreach ($v in $list) { $ss += ($v - $mean) * ($v - $mean) }
    $t = @(0, 12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.160, 2.145, 2.131, 2.120, 2.110, 2.101, 2.093, 2.086)
    $tv = $(if ($list.Count - 1 -lt $t.Count) { $t[$list.Count - 1] } else { 1.96 })
    [pscustomobject]@{ mean = $mean; half = $tv * [math]::Sqrt($ss / ($list.Count - 1)) / [math]::Sqrt($list.Count); n = $list.Count }
}
function Format-Interval($interval, [string]$format) {
    if (-not $interval) { return '' }
    if ($null -eq $interval.half) { return ($format -f $interval.mean) }
    ("$format +/- $($format -replace '\{0', '{1')" -f $interval.mean, $interval.half)
}
function Get-SeriesValue($series, [int64]$hour) {
    $below = $null; $above = $null
    foreach ($point in $series) {
        $h = [int64]$point[0]
        if ($h -le $hour -and ($null -eq $below -or $h -ge [int64]$below[0])) { $below = $point }
        if ($h -ge $hour -and ($null -eq $above -or $h -le [int64]$above[0])) { $above = $point }
    }
    if ($null -eq $below -or $null -eq $above) { return $null }
    $h0 = [int64]$below[0]; $h1 = [int64]$above[0]
    if ($h1 -eq $h0) { return [double]$below[1] }
    [double]$below[1] + ([double]$above[1] - [double]$below[1]) * ($hour - $h0) / ($h1 - $h0)
}
function Test-Reference($r) { $r.passed -and -not $r.frozen -and -not ($r.PSObject.Properties['ladder_config'] -and $r.ladder_config) -and $r.PSObject.Properties['hours_per_s'] }
$references = @($rows | Where-Object { Test-Reference $_ })
$mb = 1048576.0
$summaryLines = New-Object Collections.ArrayList
$costSections = New-Object Collections.ArrayList
foreach ($candidate in @($spec.candidates | Where-Object { $_.frozen })) {
    $screens = @($rows | Where-Object { $_.name -eq [string]$candidate.name -and $_.passed -and $_.PSObject.Properties['size'] -and $_.size })
    if ($screens.Count -eq 0) { continue }
    $slowdowns = New-Object Collections.ArrayList; $extras = New-Object Collections.ArrayList
    $peakDeltas = New-Object Collections.ArrayList; $endDeltas = New-Object Collections.ArrayList
    foreach ($r in $screens) {
        $paired = @($references | Where-Object { $_.repeat -eq $r.repeat })
        if ($paired.Count) {
            $reference = (@($paired | ForEach-Object { [double]$_.hours_per_s }) | Measure-Object -Average).Average
            [void]$slowdowns.Add(100.0 * (1.0 - [double]$r.hours_per_s / $reference))
            [void]$extras.Add(1000.0 / [double]$r.hours_per_s - 1000.0 / $reference)
        }
        if ($references.Count) {
            $deltas = New-Object Collections.ArrayList
            foreach ($point in $r.memory_series) {
                $values = New-Object Collections.ArrayList
                foreach ($reference in $references) { $value = Get-SeriesValue $reference.memory_series ([int64]$point[0]); if ($null -ne $value) { [void]$values.Add($value) } }
                if ($values.Count -eq $references.Count) { [void]$deltas.Add([double]$point[1] - ($values | Measure-Object -Average).Average) }
            }
            if ($deltas.Count) { [void]$peakDeltas.Add((($deltas | Measure-Object -Maximum).Maximum) / $mb); [void]$endDeltas.Add($deltas[$deltas.Count - 1] / $mb) }
        }
    }
    $recorderPeaks = @($screens | ForEach-Object { [math]::Max([double]$_.memory.recorder_private_peak_window, [double]$_.memory.recorder_private_peak_detach) / $mb })
    [void]$summaryLines.Add(("| {0} | {1} | {2} | {3} | {4} | {5} | {6} | {7} | {8} | {9} | {10} | {11} | {12} | {13} |" -f $candidate.name, $screens.Count,
        (Format-Interval (Get-Interval $slowdowns) '{0:n2}'), (Format-Interval (Get-Interval $extras) '{0:n2}'),
        (Format-Interval (Get-Interval @($screens | ForEach-Object { $_.exact.callback_p95_ms })) '{0:n2}'),
        (Format-Interval (Get-Interval $peakDeltas) '{0:n0}'), (Format-Interval (Get-Interval $endDeltas) '{0:n0}'),
        (Format-Interval (Get-Interval $recorderPeaks) '{0:n0}'), (Format-Interval (Get-Interval @($screens | ForEach-Object { [double]$_.exact.pool_bytes_max / $mb })) '{0:n1}'),
        (Format-Interval (Get-Interval @($screens | ForEach-Object { $_.size.raw_bytes_per_hour })) '{0:n0}'),
        (Format-Interval (Get-Interval @($screens | ForEach-Object { $_.size.spool_bytes_per_hour })) '{0:n0}'),
        (Format-Interval (Get-Interval @($screens | ForEach-Object { $_.size.replay_bytes_per_hour })) '{0:n0}'),
        (Format-Interval (Get-Interval @($screens | ForEach-Object { $_.size.finalize_s })) '{0:n2}'),
        (Format-Interval (Get-Interval @($screens | ForEach-Object { $_.size.stop_to_save_ready_s })) '{0:n2}')))
    $byTable = @{}
    foreach ($r in $screens) {
        foreach ($t in (Get-Content -LiteralPath (Join-Path $r.run_dir 'tables.json') -Raw | ConvertFrom-Json)) {
            if (-not $byTable.ContainsKey($t.table)) { $byTable[$t.table] = New-Object Collections.ArrayList }
            [void]$byTable[$t.table].Add([pscustomobject]@{ per_hour = [double]$t.raw_bytes / [double]$r.size.hours; p95 = [double]$t.read_p95_ms; max = [double]$t.read_max_ms; snapshots = [int64]$t.snapshots; deltas = [int64]$t.deltas; unchanged = [int64]$t.unchanged })
        }
    }
    if ($byTable.Count -eq 0) { continue }
    $total = 0.0
    $cost = foreach ($key in $byTable.Keys) {
        $samples = $byTable[$key]
        $entry = [pscustomobject]@{ table = $key; per_hour = ($samples | Measure-Object -Property per_hour -Average).Average; p95 = ($samples | Measure-Object -Property p95 -Average).Average
            max = ($samples | Measure-Object -Property max -Maximum).Maximum; snapshots = ($samples | Measure-Object -Property snapshots -Sum).Sum
            deltas = ($samples | Measure-Object -Property deltas -Sum).Sum; unchanged = ($samples | Measure-Object -Property unchanged -Sum).Sum }
        $total += $entry.per_hour
        $entry
    }
    $header = @('| table | raw B/hour | share | read p95 ms | read max ms | snapshots | deltas | unchanged |', '|---|---|---|---|---|---|---|---|')
    $format = { param($e) "| {0} | {1:n0} | {2:p1} | {3:n3} | {4:n3} | {5} | {6} | {7} |" -f $e.table, $e.per_hour, $(if ($total -gt 0) { $e.per_hour / $total } else { 0 }), $e.p95, $e.max, $e.snapshots, $e.deltas, $e.unchanged }
    $bySize = @($cost | Sort-Object -Property per_hour -Descending)
    $byTime = @($cost | Sort-Object -Property p95 -Descending)
    $full = @("# Table costs: $($candidate.name)", '', "Mean over $($screens.Count) screens; raw encoded bytes per recorded hour total $('{0:n0}' -f $total).", '', '## By encoded bytes', '') + $header + @($bySize | ForEach-Object { & $format $_ }) +
        @('', '## By read-time p95', '') + $header + @($byTime | ForEach-Object { & $format $_ })
    [IO.File]::WriteAllText((Join-Path $Out ('cost_' + ($candidate.name -replace '[^\w.-]', '_') + '.md')), ($full -join "`n") + "`n", $utf8)
    [void]$costSections.AddRange(@('', "## Largest tables: $($candidate.name)", '', "Top 25 of $($cost.Count) by raw encoded bytes per recorded hour; full lists in cost_$($candidate.name -replace '[^\w.-]', '_').md.", '') + $header + @($bySize | Select-Object -First 25 | ForEach-Object { & $format $_ }) +
        @('', "## Slowest table reads: $($candidate.name)", '') + $header + @($byTime | Select-Object -First 25 | ForEach-Object { & $format $_ }))
}
if ($summaryLines.Count) {
    $md += @('', '## Candidate summary', '',
        "Mean +/- 95% t interval across passed screens. Slowdown and extra ms pair each screen with the same repeat's no-recorder screens. hoi4.exe private bytes compare with the mean no-recorder screen at the same game hour (peak and end of window). Recorder peak covers the window, drain and finalization. Sizes are per recorded hour when complete evidence exists; blank cells are unavailable, not zero. Stop to ready is the true durable completion timer, not cumulative finalize_s.", '',
        '| candidate | screens | slowdown % | extra ms/game-hour | callback p95 ms | the game private delta peak MB | the game private delta end MB | recorder peak MB | pool max MB | raw B/hour | spool B/hour | replay B/hour | finalize s | Stop to ready s |',
        '|---|---|---|---|---|---|---|---|---|---|---|---|---|---|') + @($summaryLines) + @($costSections)
}
[IO.File]::WriteAllText((Join-Path $Out 'results.md'), ($md -join "`n") + "`n", $utf8)
$md -join "`n"
