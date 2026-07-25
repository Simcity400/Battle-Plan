<#
.SYNOPSIS
Headless, uncompressed, no-debug-scaffolding recorder test loop.

Launches the game into its last save, unpauses it and sets the game speed by
authenticated memory write (no window activation, no synthetic input, no
client interaction), attaches the production recorder with the raw
uncompressed H4CR stream, and leaves every artifact on disk in a form that
can be read directly.

Deliberately NOT included: zstd stream compression, the debug capture
profile, sidecar logs, per-read RC counters. The point of this loop is a
plain stream plus a plain manifest.
#>
[CmdletBinding()]
param(
    [string]$Hoi4Exe = 'X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe',

    [string]$PdxUserDir = 'C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV',


    [string]$OutDir = '',

    [ValidateRange(10, 86400)]
    [double]$DurationSeconds = 120,

    [ValidateRange(1, 5)]
    [int]$Speed = 5,

    [ValidateRange(0, 100000)]
    [int]$MinGameHours = 0,

    [ValidateRange(1, 10000)]
    [int]$SampleMilliseconds = 1,

    [ValidateRange(1, 64)]
    [int]$ReaderConcurrency = 12,

    # Seconds to wait for the loaded campaign to expose stable world singletons.
    [ValidateRange(30, 1800)]
    [int]$WorldReadyTimeoutSeconds = 600,

    # Startup + finalize headroom allowed on top of -DurationSeconds before the
    # recorder is treated as wedged rather than slow.
    [ValidateRange(30, 3600)]
    [int]$RecorderOverheadSeconds = 300,

    # Seconds of continuously-advancing clock required before the recorder is
    # allowed to inject.
    #
    # MITIGATION, not a fix. Injecting while the engine is still finishing its
    # parallel post-load work has crashed the game (access violation calling
    # address 0x0 on a tbb.dll worker thread); injecting into a settled process
    # has been reliable. Hook installation freezes threads per hook via MinHook,
    # and a thread created during that window can miss the freeze. Set to 0 to
    # deliberately reproduce the race.
    [ValidateRange(0, 900)]
    [int]$SettleSeconds = 90,


    # Reuse an already-running campaign instead of launching one.
    #
    # Valid only for a game process that has never been recorded. The recorder
    # takes its injection lease by assigning the target to a job object, and job
    # membership cannot be revoked for the life of a process, so a second
    # capture against the same PID always fails with "already belongs to a job".
    # A repeatable loop must launch a fresh game per capture (the default).
    [switch]$UseRunningGame,

    # Close a running hoi4.exe before launching a fresh one.
    [switch]$StopExistingGame,

    # Leave the game running after the capture instead of closing it.
    [switch]$KeepGameAfterCapture,

    # Override the derived production roster with an explicit domain list.
    # Used to measure how the injection-crash rate scales with the number of
    # installed hooks; not a production configuration.
    [string[]]$ScheduledDomainOverride = @(),

    [switch]$SkipHostFold,

    # Drop --static-gfx-icon-manifest from the capture.
    #
    # Escape hatch for a KNOWN outstanding blocker, not a default: the recorder
    # requires meta.unused_missing_texture_allowlist_entries == 0, and the
    # checked-in allowlist currently leaves 72 rows unused, so finalize rejects
    # the artifact after the whole raw stream has already been written. Use this
    # to exercise runtime domain extraction while that is being fixed; a capture
    # produced this way is missing that static artifact and is not promotable.
    # See extractor/native/runs/gfx_allowlist_staleness_triage.json.
    [switch]$SkipGfxIconManifest,

    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $scriptDir '..\..\..')).Path
$releaseRoot = Join-Path $repoRoot '.cargo-target\release'
$nativeRoot = Join-Path $repoRoot 'extractor\native'
$collectorDll = Join-Path $releaseRoot 'collector.dll'
$collectorRecord = Join-Path $releaseRoot 'collector_record.exe'
$headlessControl = Join-Path $scriptDir 'Control-Hoi4Game.ps1'
$streamReport = Join-Path $scriptDir 'report_raw_capture.py'
$gameDefsOut = Join-Path $repoRoot 'extractor\game_defs\out'
# Get-Command can return several python.exe entries (real install plus the
# WindowsApps stub); take the first rather than joining them into one string.
$pythonExe = @(Get-Command python.exe -CommandType Application -ErrorAction Stop)[0].Source

$staticCatalogs = [ordered]@{
    '--static-templates-equipment-catalog' = Join-Path $gameDefsOut 'templates_equipment_catalog\templates_equipment_catalog_live_domain59\domain59_static_catalog.json'
    '--static-localisation-join'           = Join-Path $gameDefsOut 'localisation_gate\static_localisation_join_r3\localisation_catalog.json'
    '--static-decisions-definitions'       = Join-Path $gameDefsOut 'decisions_static\decisions_static_definitions\decision_definitions_catalog.json'
    '--static-idea-tech-loc-xref'          = Join-Path $gameDefsOut 'idea_tech_loc_xref\idea_tech_loc_xref_static\idea_tech_loc_xref_catalog.json'
    '--static-ship-definitions'            = Join-Path $gameDefsOut 'ship_definitions_static\ship_definitions_static\ship_definitions_catalog.json'
    '--static-gfx-icon-manifest'           = Join-Path $gameDefsOut 'gfx_icon_manifest\gfx_icon_asset_manifest_static\gfx_icon_manifest_catalog.json'
}

foreach ($path in @($Hoi4Exe, $PdxUserDir, $collectorDll, $collectorRecord, $headlessControl)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required path is missing: $path" }
}
if ($SkipGfxIconManifest) {
    $staticCatalogs.Remove('--static-gfx-icon-manifest')
    Write-Warning 'Capturing WITHOUT the GFX icon manifest static artifact (-SkipGfxIconManifest). This capture is not promotable.'
}
foreach ($entry in $staticCatalogs.GetEnumerator()) {
    if (-not (Test-Path -LiteralPath $entry.Value -PathType Leaf)) {
        throw "Static catalog for $($entry.Key) is missing: $($entry.Value)"
    }
}

# `--profile production` is early-launch-only ("production/profile capture
# requires authenticated --early-launch-exe so render creation cannot be
# missed") and additionally demands a process-bound static bundle. A late
# attach therefore selects the production roster explicitly, exactly as the
# shipped product worker does. Passing no schedule at all would silently fall
# back to the eight-domain compatibility set, so the roster is mandatory.
$rosterTool = Join-Path $scriptDir 'production_domain_roster.py'
if (-not (Test-Path -LiteralPath $rosterTool -PathType Leaf)) {
    throw "Production roster tool is missing: $rosterTool"
}
if ($ScheduledDomainOverride.Count -gt 0) {
    $scheduledDomains = @($ScheduledDomainOverride)
    Write-Warning "Using an explicit $($scheduledDomains.Count)-domain roster override; this is a diagnostic configuration, not production coverage."
} else {
    $scheduledDomains = @(& $pythonExe $rosterTool --native-root $nativeRoot)
    if ($LASTEXITCODE -ne 0) { throw 'Production domain roster derivation failed.' }
    if ($scheduledDomains.Count -eq 0) { throw 'Derived production domain roster is empty.' }
}

$Hoi4Exe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$PdxUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$observedExeSha256 = (Get-FileHash -LiteralPath $Hoi4Exe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($observedExeSha256 -cne $expectedExeSha256) {
    throw "hoi4.exe is not the pinned exact-a729 build: $observedExeSha256"
}

$stamp = Get-Date -Format 'yyyyMMdd_HHmmss'
if ([string]::IsNullOrWhiteSpace($OutDir)) {
    $OutDir = Join-Path $repoRoot "extractor\native\runs\headless_raw_$stamp"
}
$OutDir = [System.IO.Path]::GetFullPath($OutDir)
if (Test-Path -LiteralPath $OutDir) {
    if (@(Get-ChildItem -LiteralPath $OutDir -Force).Count -ne 0) {
        throw "OutDir must be absent or empty: $OutDir"
    }
} else {
    New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
}

function ConvertTo-NativeArgumentLine {
    param([Parameter(Mandatory = $true)][object[]]$Arguments)
    return (($Arguments | ForEach-Object {
        $value = [string]$_
        if ($value -match '[\s"]') { '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"' }
        else { $value }
    }) -join ' ')
}

function Get-Hoi4Processes {
    # PowerShell unwraps single-element arrays on return, which breaks `.Count`
    # under StrictMode; callers wrap this in @() to restore array semantics.
    return @(Get-Process -Name the game -ErrorAction SilentlyContinue)
}

function Wait-Hoi4WorldReady {
    param(
        [Parameter(Mandatory = $true)][int]$TargetProcessId,
        [Parameter(Mandatory = $true)][int]$TimeoutSeconds
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $lastError = 'not attempted'
    while ([DateTime]::UtcNow -lt $deadline) {
        $probe = Get-Process -Id $TargetProcessId -ErrorAction SilentlyContinue
        if ($null -eq $probe) { throw "the game PID $TargetProcessId exited before the world became ready." }
        try {
            $state = & $headlessControl -TargetPid $TargetProcessId
            if ($null -ne $state) { return $state }
        } catch {
            # CGameState / CInGameIdler do not exist until the save is loaded.
            $lastError = $_.Exception.Message
        }
        Start-Sleep -Milliseconds 750
    }
    throw "the game PID $TargetProcessId did not expose stable world singletons within $TimeoutSeconds s. Last probe error: $lastError"
}

# --- Phase 1: obtain a live campaign process -------------------------------
$launchedProcess = $null
$gamePid = 0

if ($UseRunningGame) {
    $running = @(Get-Hoi4Processes)
    if ($running.Count -ne 1) { throw "-UseRunningGame needs exactly one hoi4.exe; found $($running.Count)." }
    $gamePid = [int]$running[0].Id
    Write-Host "phase=reuse pid=$gamePid"
} else {
    $running = @(Get-Hoi4Processes)
    if ($running.Count -ne 0) {
        if (-not $StopExistingGame) {
            throw "hoi4.exe PID(s) $(@($running.Id) -join ',') are running. Pass -StopExistingGame to close them, or -UseRunningGame to attach to the existing campaign."
        }
        foreach ($proc in $running) {
            Write-Host "phase=stop_existing pid=$($proc.Id)"
            if ($DryRun) { continue }
            try { $proc.CloseMainWindow() | Out-Null } catch { }
            if (-not $proc.WaitForExit(15000)) { $proc.Kill(); [void]$proc.WaitForExit(15000) }
        }
    }

    $continueGame = Join-Path $PdxUserDir 'continue_game.json'
    if (-not (Test-Path -LiteralPath $continueGame -PathType Leaf)) {
        throw "Launcher continue selection is missing: $continueGame"
    }
    $continueTitle = (Get-Content -LiteralPath $continueGame -Raw | ConvertFrom-Json).title
    Write-Host "phase=launch continue_title=$continueTitle"

    # Launch through WMI rather than Start-Process. The recorder refuses a
    # target that already belongs to a job object (it takes its own injection
    # lease as a job), and a process started directly inherits the caller's job
    # when this script runs under an automation host that uses one. Win32_Process
    # creates the game from the WMI service, outside any such job.
    $launchCommandLine = '"' + $Hoi4Exe + '" --continuelastsave --gdpr-compliant "-userdir=' + $PdxUserDir + '"'
    if ($DryRun) {
        Write-Host "DRY RUN launch: $launchCommandLine"
    } else {
        $created = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
            CommandLine      = $launchCommandLine
            CurrentDirectory = (Split-Path -Parent $Hoi4Exe)
        }
        if ([int]$created.ReturnValue -ne 0) {
            throw "Win32_Process.Create failed for hoi4.exe with return value $($created.ReturnValue)."
        }
        $gamePid = [int]$created.ProcessId
        $launchedProcess = Get-Process -Id $gamePid -ErrorAction Stop
        Write-Host "phase=launched pid=$gamePid"
    }
}

if ($DryRun) {
    Write-Host "DRY RUN: stopping before world-ready wait. out_dir=$OutDir"
    return
}

# --- Phase 2: headless world-ready + unpause + speed ------------------------
$readyState = Wait-Hoi4WorldReady -TargetProcessId $gamePid -TimeoutSeconds $WorldReadyTimeoutSeconds
Write-Host "phase=world_ready pid=$gamePid initial_paused=$($readyState.Paused) initial_speed=$($readyState.Speed)"

# The world singletons become readable before the save has finished loading,
# and the engine sets its own paused flag when the load completes. A single
# unpause write can therefore be silently reverted, which previously produced
# a capture whose clock never advanced. Re-apply until the hour actually moves.
$control = $null
$controlDeadline = [DateTime]::UtcNow.AddSeconds(120)
while ([DateTime]::UtcNow -lt $controlDeadline) {
    $attempt = & $headlessControl -TargetPid $gamePid -Speed $Speed -Play
    if ([int]$attempt.Speed -eq $Speed -and -not [bool]$attempt.Paused -and [bool]$attempt.Advancing) {
        $control = $attempt
        break
    }
    Write-Host "phase=control_retry speed=$($attempt.Speed) paused=$($attempt.Paused) advancing=$($attempt.Advancing)"
    Start-Sleep -Milliseconds 1000
}
if ($null -eq $control) {
    throw "Headless control could not hold speed $Speed with an advancing clock within 120 s."
}
Write-Host "phase=controlled speed=$($control.Speed) paused=$($control.Paused) hour_delta=$($control.HourDelta) advancing=$($control.Advancing)"
$controlPath = Join-Path $OutDir 'headless_control.json'
($control | ConvertTo-Json -Depth 8) | Set-Content -LiteralPath $controlPath -Encoding UTF8

# Require the clock to keep advancing for the whole settle window. A single
# advancing sample only proves the save finished loading, not that the engine's
# parallel post-load work has drained -- and it is that work that the injection
# race kills the game in.
if ($SettleSeconds -gt 0) {
    Write-Host "phase=settle seconds=$SettleSeconds"
    $settleDeadline = [DateTime]::UtcNow.AddSeconds($SettleSeconds)
    while ([DateTime]::UtcNow -lt $settleDeadline) {
        Start-Sleep -Seconds 10
        $sample = & $headlessControl -TargetPid $gamePid -Speed $Speed -Play
        if (-not $sample.Advancing -or [int]$sample.Speed -ne $Speed) {
            Write-Host "phase=settle_restart speed=$($sample.Speed) advancing=$($sample.Advancing)"
            $settleDeadline = [DateTime]::UtcNow.AddSeconds($SettleSeconds)
        }
    }
    Write-Host 'phase=settled'
}

# --- Phase 3: raw uncompressed capture --------------------------------------
$sessionId = "headless_raw_$stamp"
$captureId = "headless-raw-capture-$stamp"
$durationArg = [string]::Format([Globalization.CultureInfo]::InvariantCulture, '{0:0.###}', $DurationSeconds)
$recorderArgs = @(
    '--pid', "$gamePid",
    '--dll', $collectorDll,
    '--out-dir', $OutDir,
    '--duration-s', $durationArg,
    '--sample-ms', "$SampleMilliseconds",
    '--min-game-hours', "$MinGameHours",
    '--stream-format', 'compact',
    '--stream-compression', 'none',
    '--compact-country-frame',
    '--hour-latch',
    '--reader-concurrency', "$ReaderConcurrency",
    '--session-id', $sessionId,
    '--capture-id', $captureId
)
foreach ($domain in $scheduledDomains) {
    $recorderArgs += '--scheduled-domain'
    $recorderArgs += $domain
}
foreach ($entry in $staticCatalogs.GetEnumerator()) {
    $recorderArgs += $entry.Key
    $recorderArgs += $entry.Value
}
if ($SkipHostFold) { $recorderArgs += '--skip-host-fold' }

($recorderArgs -join "`n") | Set-Content -LiteralPath (Join-Path $OutDir 'recorder_args.txt') -Encoding UTF8

$stdoutPath = Join-Path $OutDir 'collector_record.stdout.log'
$stderrPath = Join-Path $OutDir 'collector_record.stderr.log'
Write-Host "phase=capture duration_s=$durationArg compression=none scheduled_domains=$($scheduledDomains.Count) out_dir=$OutDir"

# Redirect straight to files rather than in-memory pipes: an in-memory read
# hides the recorder's progress until it exits, which makes a startup hang
# indistinguishable from a slow capture.
$recorderProcess = Start-Process -FilePath $collectorRecord `
    -ArgumentList (ConvertTo-NativeArgumentLine $recorderArgs) `
    -NoNewWindow -PassThru `
    -RedirectStandardOutput $stdoutPath `
    -RedirectStandardError $stderrPath

# Bound the wait: capture duration plus a startup/finalize margin. A recorder
# that outlives this budget is wedged, not slow, and hanging forever here is
# what previously left orphaned recorders holding machine-wide capture state.
#
# Also watch the game itself. Collector injection can kill the game, and the
# recorder does not always notice promptly -- it sat idle for the whole budget,
# turning a dead attempt into an 8-minute stall. There is nothing left to
# capture once the target is gone, so end the attempt immediately.
$recorderBudgetSeconds = $DurationSeconds + $RecorderOverheadSeconds
$recorderDeadline = [DateTime]::UtcNow.AddSeconds($recorderBudgetSeconds)
$gameDied = $false
while (-not $recorderProcess.HasExited) {
    if ([DateTime]::UtcNow -ge $recorderDeadline) {
        Write-Warning "collector_record exceeded its $recorderBudgetSeconds s budget (capture $DurationSeconds s + $RecorderOverheadSeconds s overhead); terminating so it cannot wedge later captures."
        try { $recorderProcess.Kill() } catch { }
        [void]$recorderProcess.WaitForExit(30000)
        Write-Host 'phase=capture_timeout'
        break
    }
    if (-not (Get-Process -Id $gamePid -ErrorAction SilentlyContinue)) {
        $gameDied = $true
        Write-Host "phase=game_died pid=$gamePid"
        Write-Warning "the game PID $gamePid exited during capture (collector injection can crash the game); ending this attempt."
        # Give the recorder a short grace period to finalize the valid prefix.
        if (-not $recorderProcess.WaitForExit(30000)) {
            try { $recorderProcess.Kill() } catch { }
            [void]$recorderProcess.WaitForExit(30000)
        }
        break
    }
    Start-Sleep -Seconds 2
}
if (-not $recorderProcess.HasExited) { [void]$recorderProcess.WaitForExit(30000) }
$recorderExit = $recorderProcess.ExitCode
Write-Host "phase=capture_done exit=$recorderExit"

# --- Phase 4: leave the machine in a known state ----------------------------
if (-not $KeepGameAfterCapture -and $null -ne $launchedProcess) {
    $launchedProcess.Refresh()
    if (-not $launchedProcess.HasExited) {
        Write-Host "phase=stop_game pid=$gamePid"
        try { $launchedProcess.CloseMainWindow() | Out-Null } catch { }
        if (-not $launchedProcess.WaitForExit(20000)) { $launchedProcess.Kill(); [void]$launchedProcess.WaitForExit(20000) }
    }
}

# --- Phase 5: read the outputs directly -------------------------------------
Write-Host ''
Get-ChildItem -LiteralPath $OutDir -Force | Sort-Object Length -Descending |
    Select-Object Name, Length | Format-Table -AutoSize | Out-String | Write-Host

# An attempt whose game died mid-capture legitimately has no manifest. Reading
# the outputs is reporting, not a gate: it must never turn a failed attempt into
# a terminating error for whatever is driving this script.
$manifestPath = Join-Path $OutDir 'native_session_manifest.json'
if ((Test-Path -LiteralPath $streamReport -PathType Leaf) -and
    (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    try {
        $previous = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        & $pythonExe $streamReport --run-dir $OutDir 2>&1 | Write-Host
    } catch {
        Write-Host "report failed: $($_.Exception.Message)"
    } finally {
        $ErrorActionPreference = $previous
    }
} elseif ($gameDied) {
    Write-Host 'no manifest: the game died during capture, so there is nothing to finalize'
} else {
    Write-Host "no manifest under $OutDir"
}

Write-Host "out_dir=$OutDir"
Write-Host "recorder_exit=$recorderExit"
Write-Host "game_died=$gameDied"
if ($null -eq $recorderExit) { exit 1 }
exit [int]$recorderExit
