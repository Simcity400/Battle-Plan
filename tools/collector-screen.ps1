[CmdletBinding()]
param(
    [string]$BinaryDirectory,
    [switch]$Bare,
    [switch]$RequireFreshLaunch,
    [switch]$NoRecovery,
    [switch]$RequireReadyWitness,
    [ValidateRange(0,60)][int]$ReadyWarmupSeconds=0,
    [string]$ExpectedSave,
    [string]$UiConfig,
    [string]$InputShimDir,
    [Parameter(Mandatory=$true)][string]$Out,
    [string]$GameExe=$(if (Test-Path Env:BATTLE_PLAN_GAME_EXE) { $env:BATTLE_PLAN_GAME_EXE } else { $env:HOI4_GAME_EXE }),
    [string]$UserDirectory,
    [string[]]$ConsoleLines=@(),
    [ValidateRange(1,1800)][int]$Seconds=300,
    [ValidateRange(1,2700)][int]$QueueTimeoutSeconds=2700,
    [string]$CancelFile,
    [string]$Lane='collector-screen',
    [string]$SmokeScript=(Join-Path $PSScriptRoot 'smoke.ps1'),
    [string]$CaptureRoot=$(if (Test-Path Env:BATTLE_PLAN_CAPTURES) { $env:BATTLE_PLAN_CAPTURES } else { $env:HOI4_CAPTURES }),
    [string]$GameLock=(Join-Path $env:TEMP 'claude\hoi4_game.lock'),
    [string]$LockQueueScript,
    [string]$BashExe=(Join-Path $env:ProgramFiles 'Git\usr\bin\bash.exe')
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$utf8=New-Object Text.UTF8Encoding $false
if (-not $CaptureRoot) { throw 'Supply CaptureRoot or set BATTLE_PLAN_CAPTURES.' }
$benchRoot=Join-Path $CaptureRoot 'collector-bench'
if (-not $UiConfig) { $UiConfig=Join-Path $benchRoot 'reload-ui.json' }
if (-not $InputShimDir) { $InputShimDir=Join-Path $benchRoot 'bin-inputshim' }
if (-not $UserDirectory) { $UserDirectory=Join-Path $CaptureRoot 'perfbenchuser' }
if (-not $ExpectedSave) { $ExpectedSave=(Get-Content -LiteralPath $UiConfig -Raw | ConvertFrom-Json).expected_save }
if (-not $GameExe) { throw 'Supply GameExe or set BATTLE_PLAN_GAME_EXE.' }
if (-not $LockQueueScript) { $LockQueueScript=Join-Path $CaptureRoot 'baseline-main\lock-acquire.sh' }
$GameLock=[IO.Path]::GetFullPath($GameLock)
$SmokeScript=(Resolve-Path -LiteralPath $SmokeScript).ProviderPath
$LockQueueScript=(Resolve-Path -LiteralPath $LockQueueScript).ProviderPath
if (-not $Bare -and -not $BinaryDirectory) { throw 'Recorder screens require BinaryDirectory; bare screens use -Bare.' }
$prerequisites=@($SmokeScript,$UiConfig,$GameExe,$LockQueueScript,$BashExe,(Join-Path $InputShimDir 'inputshim.dll'),(Join-Path $InputShimDir 'inputshim-inject.exe'))
$queueHelper=Join-Path (Split-Path -Parent $SmokeScript) 'collector-lock.sh'
$recorderGuard=Join-Path (Split-Path -Parent $SmokeScript) 'recorder-guard.ps1'
$prerequisites+=@($queueHelper,$recorderGuard)
if (-not $Bare) { $prerequisites+=@((Join-Path $BinaryDirectory 'collector.dll'),(Join-Path $BinaryDirectory 'recorder.exe')) }
foreach ($path in $prerequisites) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing screen prerequisite: $path" }
}
. $recorderGuard
if ([IO.File]::ReadAllText((Resolve-Path -LiteralPath $SmokeScript).ProviderPath) -notmatch "(?s)ValidateSet\([^)]*'Reload'") { throw 'SmokeScript does not support Reload; supply the implementation checkout until it lands.' }
$Out=[IO.Path]::GetFullPath($Out)
$CaptureRoot=[IO.Path]::GetFullPath($CaptureRoot).TrimEnd([char[]]'\/')
if (-not $Out.StartsWith($CaptureRoot+'\',[StringComparison]::OrdinalIgnoreCase) -or (Test-Path -LiteralPath $Out)) { throw 'Out must be a new folder beneath CaptureRoot.' }
$ui=Get-Content -LiteralPath $UiConfig -Raw | ConvertFrom-Json
if ($ui.expected_save -cne $ExpectedSave) { throw 'UiConfig is for a different save.' }
$owner="$Lane $PID $([Guid]::NewGuid().ToString('N'))"
$queue="$GameLock.queue"
$queueRun=$null; $held=$false; $game=$null; $reader=$null; $record=$null; $releaseSafe=$true
$children=New-Object Collections.Generic.List[Diagnostics.Process]
$recordRoot=Join-Path $Out 'recording'
$gameState=Join-Path (Split-Path -Parent $GameLock) 'hoi4-screen-game.json'
$recorder=if (-not $Bare) {Join-Path (Resolve-Path -LiteralPath $BinaryDirectory).ProviderPath 'recorder.exe'} else {$null}
$result=[ordered]@{passed=$false;bare=[bool]$Bare;save=$ExpectedSave;requested_window_seconds=$Seconds;reload_attempts=0;verified_reloads=0;clean_detaches=0;error=$null;cleanup_errors=@()}
function Write-Json([string]$name,$value) { [IO.File]::WriteAllText((Join-Path $Out $name),($value | ConvertTo-Json -Depth 30),$utf8) }
function Own-Lock {
    if (-not $held -or [IO.File]::ReadAllText((Join-Path $GameLock 'owner')).Trim() -cne $owner) { throw 'Shared game lock ownership was lost.' }
    (Get-Item -LiteralPath $GameLock).LastWriteTimeUtc=[DateTime]::UtcNow
}
function Drive([string]$action,[hashtable]$extra=@{}) {
    Own-Lock
    & $SmokeScript -Action $action -GameExe $GameExe -UserDirectory $UserDirectory -CaptureRoot $CaptureRoot -InputShimDir $InputShimDir -RequireInputShim @extra
}
function Start-Tool([string[]]$arguments,[string]$name) {
    $quoted=@($arguments | ForEach-Object { if ($_ -match '"') { throw 'A native argument contains a quote.' }; '"'+($_ -replace '(\\+)$','$1$1')+'"' })
    $process=Start-Process -FilePath $recorder -ArgumentList $quoted -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $Out "$name.out") -RedirectStandardError (Join-Path $Out "$name.err")
    $null=$process.Handle
    $children.Add($process)
    return $process
}
function Finish-Tool($process,[int]$timeout=180) {
    $deadline=[DateTime]::UtcNow.AddSeconds($timeout)
    while (-not $process.WaitForExit(250)) { Own-Lock; if ([DateTime]::UtcNow -gt $deadline) { throw "Recorder command timed out (PID $($process.Id))." } }
    $process.Refresh()
    if ($process.ExitCode -ne 0) { throw "Recorder command failed (PID $($process.Id), exit $($process.ExitCode))." }
}
function Clock([switch]$Paused) {
    Own-Lock
    $deadline=[DateTime]::UtcNow.AddSeconds($(if ($Paused) {30} else {5}))
    $last=$null
    do {
        $game.Refresh(); if ($game.HasExited) { throw 'Game exited during the screen.' }
        $clock=$reader.Sample()
        if ($clock) {
            $last=$clock
            if ($clock.play_session_started_at -gt 0 -and (-not $Paused -or ($clock.tick_busy -eq 0 -and $clock.presentation_valid -and $clock.paused -eq 1))) { return $clock }
        }
        Own-Lock
        Start-Sleep -Milliseconds 1
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($last) { throw "Game clock/load witness did not satisfy the requested state: paused_required=$Paused hour=$($last.hour) load=$($last.play_session_started_at) tick_busy=$($last.tick_busy) presentation_valid=$($last.presentation_valid) paused=$($last.paused)." }
    throw 'Game clock/load witness is unreadable.'
}
function Get-CollectorModules {
    $deadline=[DateTime]::UtcNow.AddSeconds(3)
    while ($true) {
        Own-Lock
        try {
            $game.Refresh()
            if ($game.HasExited) { return }
            return @($game.Modules | Where-Object ModuleName -ieq 'collector.dll')
        } catch {
            if ([DateTime]::UtcNow -ge $deadline) { throw }
            Start-Sleep -Milliseconds 100
        }
    }
}
function Test-OwnCollector($module) {
    $path=$module.FileName
    if ($path.StartsWith('\\?\',[StringComparison]::Ordinal)) { $path=$path.Substring(4) }
    $path=[IO.Path]::GetFullPath($path)
    return $path.StartsWith((Join-Path $recordRoot 'sessions')+'\',[StringComparison]::OrdinalIgnoreCase) -and $path.EndsWith('\bin\collector.dll',[StringComparison]::OrdinalIgnoreCase)
}
function Confirm-GameIdle {
    Own-Lock
    if (@(Get-GameBlockingRecorder).Count) { throw 'Another recorder is still running.' }
    if (@(Get-CollectorModules).Count) { throw 'Detach the collector before reloading or attaching.' }
}
function Pause-Game {
    $deadline=[DateTime]::UtcNow.AddSeconds(30)
    do {
        $clock=Clock
        if ($clock.presentation_valid -and $clock.paused -le 1) { break }
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Pause state remained unreadable.' }
        Start-Sleep -Milliseconds 100
    } while ($true)
    if ($clock.paused -eq 0) { $null=Drive 'Key' @{GameId=$game.Id;Key='Space'} }
    return Clock -Paused
}
function Read-RecorderFile([string]$path,[switch]$Json) {
    $deadline=[DateTime]::UtcNow.AddSeconds(30)
    while ($true) {
        Own-Lock
        try {
            $stream=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
            $textReader=New-Object IO.StreamReader $stream
            try { $text=$textReader.ReadToEnd() } finally { $textReader.Dispose() }
            if (-not $Json) { return $text }
            $value=$text | ConvertFrom-Json
            if ($null -ne $value) { return $value }
        } catch {
            $failure=$_.Exception
            while ($failure.InnerException) { $failure=$failure.InnerException }
            $sharing=($failure -is [IO.IOException] -and ($failure.HResult -band 0xffff) -in @(2,3,32,33))
            if (-not $sharing -and $failure -isnot [UnauthorizedAccessException] -and -not ($Json -and $failure -is [ArgumentException])) { throw }
        }
        if ([DateTime]::UtcNow -ge $deadline) { throw "Recorder file did not become readable within 30 seconds: $path" }
        Start-Sleep -Milliseconds 100
    }
}
function Save-Status([string]$name) {
    $file=Get-RecordingFile 'status.json' -Recurse
    $status=Read-RecorderFile $file.FullName -Json
    Write-Json $name $status
    return $status
}
function Get-RecordingFile([string]$filter,[switch]$Recurse) {
    $deadline=[DateTime]::UtcNow.AddSeconds(30)
    do {
        Own-Lock
        try {
            $files=@(Get-ChildItem -LiteralPath $recordRoot -Filter $filter -Recurse:$Recurse -File)
            if ($files.Count -eq 1) { return $files[0] }
            if ($files.Count -gt 1) { throw "Multiple recorder files match $filter." }
        } catch {
            $failure=$_.Exception
            while ($failure.InnerException) { $failure=$failure.InnerException }
            if ($failure -isnot [IO.IOException] -and $failure -isnot [UnauthorizedAccessException]) { throw }
        }
        if ([DateTime]::UtcNow -ge $deadline) { throw "Recorder file did not become available: $filter" }
        Start-Sleep -Milliseconds 100
    } while ($true)
}
function Write-Cpu {
    $game.Refresh()
    $sample=@{utc=[DateTime]::UtcNow.ToString('o');game_pid=$game.Id;game_cpu_seconds=$game.TotalProcessorTime.TotalSeconds;recorder_pid=$null;recorder_cpu_seconds=$null}
    if ($record) { $record.Refresh(); $sample.recorder_pid=$record.Id; $sample.recorder_cpu_seconds=$record.TotalProcessorTime.TotalSeconds }
    [IO.File]::AppendAllText((Join-Path $Out 'cpu-monitor.jsonl'),($sample | ConvertTo-Json -Compress)+"`n",$utf8)
}
function Launch-Game {
    $launch=Drive 'Launch' @{ContinueLastSave=$true;ExpectedSave=$ExpectedSave;GameLock=$GameLock}
    $startedGame=Get-Process -Id $launch.GameId
    [IO.File]::WriteAllText($gameState,(@{pid=$startedGame.Id;start_ticks=$startedGame.StartTime.ToUniversalTime().Ticks;exe=$startedGame.Path} | ConvertTo-Json),$utf8)
    return $startedGame
}
function Recover-Game([string]$reason) {
    if ($NoRecovery) { throw $reason }
    Own-Lock
    $game.Refresh()
    if (-not $game.HasExited) {
        if (-not (Test-Path -LiteralPath $gameState)) { throw "Cannot relaunch a game this driver did not start: $reason" }
        $owned=Get-Content -LiteralPath $gameState -Raw | ConvertFrom-Json
        if ($owned.pid -ne $game.Id -or $owned.start_ticks -ne $game.StartTime.ToUniversalTime().Ticks -or $owned.exe -ine $game.Path) { throw "Cannot terminate an inherited game: $reason" }
        Confirm-GameIdle
        $game.Kill()
        if (-not $game.WaitForExit(10000)) { throw 'The driver-owned game did not terminate.' }
    }
    $result.recovery_reason=$reason; $result.load_method='relaunch'
    return Launch-Game
}
function Close-ContaminatedGame {
    Own-Lock
    $game.Refresh()
    if (-not $game.HasExited) {
        if ($game.Path -ine (Resolve-Path -LiteralPath $GameExe).ProviderPath) { throw 'Cleanup game executable differs from GameExe.' }
        $modules=@(Get-CollectorModules)
        if (-not $modules.Count) { $result.collector_unloaded=$true; return }
        if ($modules.Count -ne 1 -or -not (Test-OwnCollector $modules[0])) { throw 'Foreign collector found during cleanup; leaving the game untouched.' }
        if (@(Get-GameBlockingRecorder).Count) { throw 'Another recorder is running during cleanup; leaving the game untouched.' }
        $game.Kill()
        if (-not $game.WaitForExit(10000)) { throw 'The contaminated game did not stop; keeping the game lock.' }
    }
    $result.cleanup_game_closed=$true
    $result.game_left_open=$false
    $result.collector_unloaded=$null
}
try {
    $null=New-Item -ItemType Directory -Path $Out
    $hashes=[ordered]@{}
    if (-not $Bare) { foreach ($name in @('collector.dll','recorder.exe')) { $hashes[$name]=(Get-FileHash -LiteralPath (Join-Path $BinaryDirectory $name) -Algorithm SHA256).Hash } }
    Write-Json 'binary-hashes.json' $hashes
    $priority=if ($env:PRIORITY -in @('1','2')) {$env:PRIORITY} else {'0'}
    $queueArguments=@($queueHelper.Replace('\','/'),$LockQueueScript.Replace('\','/'),$GameLock.Replace('\','/'),$owner,$priority)
    $quoted=@($queueArguments | ForEach-Object { if ($_ -match '"') { throw 'A queue argument contains a quote.' }; '"'+($_ -replace '(\\+)$','$1$1')+'"' })
    $queueRun=Start-Process -FilePath $BashExe -ArgumentList $quoted -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $Out 'queue.out') -RedirectStandardError (Join-Path $Out 'queue.err')
    $null=$queueRun.Handle
    $waiting=[Diagnostics.Stopwatch]::StartNew()
    while (-not $queueRun.WaitForExit(250)) {
        if ($CancelFile -and (Test-Path -LiteralPath $CancelFile)) { throw 'Screen cancelled while queued; ticket withdrawn.' }
        if ($waiting.Elapsed.TotalSeconds -gt $QueueTimeoutSeconds) { throw 'Game queue timed out; ticket withdrawn.' }
    }
    if ($queueRun.ExitCode -ne 0) { throw "Game lock queue failed with exit $($queueRun.ExitCode)." }
    if ([IO.File]::ReadAllText((Join-Path $GameLock 'owner')).Trim() -cne $owner) { throw 'Queue returned without this screen owning the game lock.' }
    $held=$true
    $queueRun.Dispose(); $queueRun=$null
    $ready=[Diagnostics.Stopwatch]::StartNew()
    $result.lock_acquired_utc=[DateTime]::UtcNow.ToString('o')
    $games=@(Get-Process -Name the game -ErrorAction SilentlyContinue)
    if ($games.Count -gt 1) { throw 'Exactly one shared game is supported.' }
    if ($games.Count -eq 1) { $result.game_pid=$games[0].Id }
    if (@(Get-GameBlockingRecorder).Count) { throw 'Another recorder is still running.' }
    if ($RequireFreshLaunch -and $games.Count) { throw 'A fresh launch was required, but a shared game is already running.' }
    if ($games.Count -eq 0) {
        $result.load_method='launch'
        $game=Launch-Game
    } else {
        $result.load_method='reload'; $game=$games[0]
        if ($game.Path -ine (Resolve-Path -LiteralPath $GameExe).ProviderPath) { throw 'Running game executable differs from GameExe.' }
        if (-not $game.Responding) {
            $responsive=[DateTime]::UtcNow.AddSeconds(30)
            do { Own-Lock; Start-Sleep -Seconds 1; $game.Refresh() } while (-not $game.HasExited -and -not $game.Responding -and [DateTime]::UtcNow -lt $responsive)
            if ($game.HasExited -or -not $game.Responding) { $game=Recover-Game 'The shared game was gone or unresponsive for 30 seconds.' }
        }
        if ($result.load_method -eq 'reload') {
            try {
                $reload=Drive 'Reload' @{GameId=$game.Id;GameLock=$GameLock;LockOwner=$owner;UiConfig=$UiConfig;ExpectedSave=$ExpectedSave;Out=$Out}
                Write-Json 'reload.json' $reload
                if (-not $reload.Verified) { throw 'Reload was not verified.' }
                $result.reload_attempts=1; $result.verified_reloads=1
            } catch {
                $result.reload_attempts=[int](Test-Path -LiteralPath (Join-Path $Out 'load-requested.json'))
                $result.reload_error=$_.Exception.Message
                if ($_.Exception.Message -notmatch '^(Reload witness failed:|Game exited during reload\.)') { throw }
                $game=Recover-Game $_.Exception.Message
            }
        }
    }
    $result.game_pid=$game.Id
    $null=$game.Handle
    $null=Drive 'Clock' @{GameId=$game.Id}
    $reader=[Hoi4ClockBaseline]::new([uint32]$game.Id,[uint64]$game.MainModule.BaseAddress.ToInt64())
    $stable=$null; $deadline=[DateTime]::UtcNow.AddSeconds(240)
    do {
        $clock=Clock
        if ($clock.hour -eq $ui.expected_hour -and $clock.seed -eq $ui.expected_seed -and $clock.tick_busy -eq 0 -and $clock.presentation_valid -and $clock.paused -eq 1) {
            if ($stable -and $stable.Same($clock) -and ([DateTime]::UtcNow-$stableAt).TotalSeconds -ge 2) { break }
            if (-not $stable -or -not $stable.Same($clock)) { $stable=$clock; $stableAt=[DateTime]::UtcNow }
        } else { $stable=$null }
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Loaded save did not match the expected stable clock and seed.' }
        Start-Sleep -Milliseconds 100
    } while ($true)
    $result.time_to_attachable_seconds=$ready.Elapsed.TotalSeconds
    $result.initial_clock=$clock
    Write-Json 'screen.json' $result
    foreach ($line in $ConsoleLines) { $null=Drive 'Console' @{GameId=$game.Id;Text=$line} }
    if (-not $Bare) {
        Confirm-GameIdle
        $record=Start-Tool @('record','--once','--out',$recordRoot,'--user-dir',$UserDirectory,'--capture-mode','full') 'recorder'
        $deadline=[DateTime]::UtcNow.AddSeconds(120)
        do {
            Own-Lock; $game.Refresh(); $record.Refresh()
            if ($record.HasExited) { throw 'Recorder exited before attach.' }
            $modules=@(Get-CollectorModules)
            if ($modules.Count -and ($modules.Count -ne 1 -or -not (Test-OwnCollector $modules[0]))) { throw 'A foreign collector attached; leaving it untouched.' }
            $readyWitness=-not $RequireReadyWitness
            if ($RequireReadyWitness) {
                $progressFiles=@(Get-ChildItem -LiteralPath $recordRoot -Filter progress -File -Recurse -ErrorAction SilentlyContinue)
                if ($progressFiles.Count -eq 1) {
                    $readyWitness=((Read-RecorderFile $progressFiles[0].FullName) -match '(?m)^\d+ ready(?:\s|$)')
                }
            }
            if ($modules.Count -eq 1 -and $readyWitness -and @(Get-ChildItem -LiteralPath $recordRoot -Filter status.json -File -Recurse -ErrorAction SilentlyContinue).Count -eq 1) { break }
            if ([DateTime]::UtcNow -gt $deadline) { throw 'Collector did not attach.' }
            Start-Sleep -Milliseconds 250
        } while ($true)
        if ($RequireReadyWitness) {
            $attachClock=Clock -Paused
            if ($attachClock.hour -ne $ui.expected_hour -or $attachClock.seed -ne $ui.expected_seed -or $attachClock.paused -ne 1) { throw 'Collector ready witness differs from the expected paused save.' }
            Write-Json 'attach-ready.json' @{verified=$true;clock=$attachClock;progress=$progressFiles[0].FullName;utc=[DateTime]::UtcNow.ToString('o')}
        }
        $null=Save-Status 'status-start.json'
    }
    for ($n=0;$n -lt 5 -and (Clock).speed -lt 4;$n++) { $null=Drive 'Click' @{GameId=$game.Id;X=[int]$ui.speed_up[0];Y=[int]$ui.speed_up[1]} }
    if ((Clock).speed -ne 4) { throw 'Game did not reach the requested speed.' }
    $null=Drive 'Key' @{GameId=$game.Id;Key='Space'}
    $initial=$result.initial_clock
    if ($ReadyWarmupSeconds) {
        $warmup=[Diagnostics.Stopwatch]::StartNew()
        while ($warmup.Elapsed.TotalSeconds -lt $ReadyWarmupSeconds) {
            $warmClock=Clock
            if ($warmClock.play_session_started_at -ne $initial.play_session_started_at -or $warmClock.seed -ne $initial.seed -or $warmClock.hour -lt $initial.hour) { throw 'Unexpected load or campaign change during warmup.' }
            Start-Sleep -Milliseconds 250
        }
        $status=Save-Status 'status-start.json'
        if (-not $status.PSObject.Properties['callback_work'] -or -not $status.PSObject.Properties['collector_thread_cpu']) { throw 'Detailed collector status is unavailable after warmup.' }
        $result.window_initial_clock=Clock
        $result.warmup_seconds=$warmup.Elapsed.TotalSeconds
    }
    $result.window_start_utc=[DateTime]::UtcNow.ToString('o')
    $window=[Diagnostics.Stopwatch]::StartNew(); $firstHours=New-Object Collections.Generic.List[object]
    Write-Cpu; $nextCpu=30
    while ($window.Elapsed.TotalSeconds -lt $Seconds) {
        $clock=Clock
        if ($clock.play_session_started_at -ne $initial.play_session_started_at -or $clock.seed -ne $initial.seed -or $clock.hour -lt $initial.hour) { throw 'Unexpected load or campaign change during screen.' }
        if ($clock.presentation_valid -and $clock.paused -ne 0) { throw 'Game paused during the requested screen window.' }
        if ($firstHours.Count -lt 3 -and $clock.hour -gt $initial.hour -and ($firstHours.Count -eq 0 -or $firstHours[$firstHours.Count-1].hour -ne $clock.hour)) { $firstHours.Add($clock) }
        if ($window.Elapsed.TotalSeconds -gt 30 -and $clock.hour -eq $initial.hour) { throw 'Game did not advance after unpausing.' }
        if ($record) { $record.Refresh(); if ($record.HasExited) { throw 'Recorder exited during screen.' } }
        if ($window.Elapsed.TotalSeconds -ge $nextCpu) { Write-Cpu; $nextCpu+=30 }
        Start-Sleep -Milliseconds $(if ($firstHours.Count -lt 3) {10} else {250})
    }
    $result.final_clock=Pause-Game; $result.window_seconds=$window.Elapsed.TotalSeconds
    $result.window_end_utc=[DateTime]::UtcNow.ToString('o')
    Write-Cpu
    Write-Json 'first-hours.json' @($firstHours.ToArray())
    if (-not $Bare) {
        $null=Save-Status 'status-end.json'
        $modules=@(Get-CollectorModules)
        if ($modules.Count -ne 1 -or -not (Test-OwnCollector $modules[0])) { throw 'This screen no longer owns the loaded collector; detach was not requested.' }
        Finish-Tool (Start-Tool @('detach','--out',$recordRoot) 'detach') 120
        Finish-Tool $record 180
        $game.Refresh()
        if (@(Get-CollectorModules).Count) { throw 'Collector remains loaded after detach.' }
        $completed=Get-RecordingFile 'completed.json' -Recurse
        $completion=Read-RecorderFile $completed.FullName -Json
        if (-not $completion.detached -or -not $completion.drain_complete -or $completion.partial -or $completion.issues.Count -or $completion.warnings.Count) { throw 'Recording did not finalize cleanly.' }
        $result.clean_detaches=1; $result.completion=$completion
        $replay=Get-RecordingFile '*.battleplan'
        if (-not $replay) { $replay = Get-RecordingFile '*.hoi4replay' }
        Finish-Tool (Start-Tool @('inspect',$replay.FullName) 'inspect') 180
        $inspection=[IO.File]::ReadAllText((Join-Path $Out 'inspect.out'))
        if ($inspection -notmatch 'content_verified=true sessions=1 branches=1 blocks=\d+') { throw 'Replay content/session/branch verification failed.' }
        $witness=[regex]::Match($inspection,'(?m)^branch=1 session=\d+ loaded_hour=(None|Some\((\d+)\)) first_hour=(\d+) ')
        if (-not $witness.Success) { throw 'Replay loaded-hour/first-hour witness is missing.' }
        $result.loaded_hour=$(if ($witness.Groups[2].Success) {[int]$witness.Groups[2].Value} else {$null})
        $result.first_recorded_hour=[int]$witness.Groups[3].Value
        if ($RequireReadyWitness -and $result.first_recorded_hour -ne $ui.expected_hour) { throw 'First recorded hour differs from the expected paused save after ready.' }
        if (($null -ne $result.loaded_hour -and $result.loaded_hour -ne $ui.expected_hour) -or $result.first_recorded_hour -lt $ui.expected_hour) { throw 'Replay loaded-hour/first-hour witness differs from the expected save.' }
    }
    $result.passed=$true
} catch { $result.error=$_.Exception.Message }
finally {
    if ($held -and $record) {
        try {
            Own-Lock
            $game.Refresh()
            $modules=@(Get-CollectorModules)
            if ($modules.Count -eq 1 -and (Test-OwnCollector $modules[0])) {
                if ($reader) { try { $null=Pause-Game } catch { $result.cleanup_errors+=$_.Exception.Message } }
                Finish-Tool (Start-Tool @('detach','--out',$recordRoot) 'cleanup-detach') 120
                Finish-Tool $record 180
            }
        }
        catch { $result.cleanup_errors+=$_.Exception.Message }
    }
    foreach ($child in $children) {
        try { if (-not $child.HasExited) { $child.Kill(); if (-not $child.WaitForExit(10000)) { throw "Child PID $($child.Id) did not stop." } } }
        catch { $result.cleanup_errors+=$_.Exception.Message }
        finally { $child.Dispose() }
    }
    if ($game) {
        $safeGame=$false
        try {
            $game.Refresh()
            $result.game_left_open=-not $game.HasExited
            if (-not $game.HasExited) {
                $modules=@(Get-CollectorModules)
                $result.collector_unloaded=($modules.Count -eq 0)
                if (-not $result.passed -and $reader -and -not $modules.Count -and @(Get-GameBlockingRecorder).Count -eq 0) {
                    try { $null=Pause-Game } catch { $result.cleanup_errors+=$_.Exception.Message }
                }
                if ($modules.Count -eq 1 -and $record -and (Test-OwnCollector $modules[0])) {
                    $result.cleanup_errors+='Collector remains loaded after screen cleanup; closing the game under the lock.'
                    Close-ContaminatedGame
                } elseif ($modules.Count -and $record) {
                    $result.cleanup_errors+='A foreign collector is loaded; leaving the game untouched.'
                }
            } else { $result.cleanup_errors+='Game exited before the lock was released.' }
            $safeGame=$true
        } catch {
            $result.cleanup_errors+=$_.Exception.Message
        }
        if ($held -and $record -and -not $safeGame) { $releaseSafe=$false; $result.cleanup_errors+='Game safety could not be established; retaining the shared lock.' }
    }
    if ($reader) { try { $reader.Dispose() } catch { $result.cleanup_errors+=$_.Exception.Message } }
    try { if ($queueRun) {
        if (-not $queueRun.HasExited) {
            $queueRun.Kill()
            if (-not $queueRun.WaitForExit(10000)) { $releaseSafe=$false; throw 'The task-owned queue process did not stop; queue cleanup was refused.' }
        }
        $queueRun.Dispose()
        $ownerFile=Join-Path $GameLock 'owner'
        if (Test-Path -LiteralPath $ownerFile) {
            $claim=[IO.File]::ReadAllText($ownerFile).Trim()
            if ($claim -ceq $owner -or $claim.StartsWith($owner+' ',[StringComparison]::Ordinal)) {
                [IO.File]::WriteAllText($ownerFile,$owner,$utf8)
                $held=$true
            }
        }
        if (Test-Path -LiteralPath $queue) {
            foreach ($entry in Get-ChildItem -LiteralPath $queue -File) {
                try {
                    $stream=[IO.File]::Open($entry.FullName,'Open','Read',([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
                    $textReader=New-Object IO.StreamReader $stream
                    try { $claim=$textReader.ReadToEnd().Trim() } finally { $textReader.Dispose() }
                } catch [IO.IOException] { continue }
                if ($claim -ceq $owner) {
                    if ($entry.LinkType) { throw 'Queue cleanup refused a linked ticket.' }
                    Remove-Item -LiteralPath $entry.FullName -ErrorAction Stop
                    if (Test-Path -LiteralPath $entry.FullName) { throw 'The task-owned queue ticket remains.' }
                }
            }
        }
    } } catch { $result.cleanup_errors+=$_.Exception.Message }
    try { if ($held -and $releaseSafe) {
        Own-Lock
        $target=(Resolve-Path -LiteralPath $GameLock).ProviderPath
        if ($target -ine [IO.Path]::GetFullPath($GameLock) -or (Get-Item -LiteralPath $target).LinkType -or @(Get-ChildItem -LiteralPath $target -Recurse -Force | Where-Object LinkType).Count) { throw 'Lock cleanup refused a redirected path.' }
        Remove-Item -LiteralPath $target -Recurse -ErrorAction Stop
        if (Test-Path -LiteralPath $target) { throw 'Shared game lock remains after release.' }
    } } catch { $result.cleanup_errors+=$_.Exception.Message }
    if ($result.cleanup_errors.Count) { $result.passed=$false }
    if (Test-Path -LiteralPath $Out) {
        Write-Json 'screen.json' $result
        [IO.File]::WriteAllText((Join-Path $Out 'results.jsonl'),($result | ConvertTo-Json -Depth 30 -Compress)+"`n",$utf8)
    }
}
if (-not $result.passed -or $result.cleanup_errors.Count) { throw "Screen failed: $($result.error) $($result.cleanup_errors -join '; ')" }
$result | ConvertTo-Json -Depth 10
