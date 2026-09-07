<#
.SYNOPSIS
Prepare or run one bounded automatic capture scenario at this historical tool path.
.DESCRIPTION
Replaces retry/kill/delete behavior. Plan never launches. Run starts the attested
recorder with no arguments and observes externally established games/campaigns.
The only lifecycle control is graceful close of that owned recorder. No game
launch, UI, focus, speed, affinity or termination commands. Prior attempts stay.
60 seconds of joint Recording observations follow a separate startup deadline.
Finalization has its own deadline; unresolved processes are retained and named.
This writes scenario evidence, NOT production_capture_validation_receipt.v1.
#>
[CmdletBinding()]
param(
    [ValidateSet('Plan','Run')][string]$Mode = 'Plan',
    [Parameter(Mandatory=$true)][ValidateSet('app-first','game-first','simultaneous')][string]$Scenario,
    [Parameter(Mandatory=$true)][string]$RepoRoot,
    [Parameter(Mandatory=$true)][string]$ArtifactAttestationPath,
    [Parameter(Mandatory=$true)][string]$GameExe,
    [Parameter(Mandatory=$true)][ValidatePattern('^[0-9a-f]{64}$')][string]$GameExeSha256,
    [Parameter(Mandatory=$true)][string]$UserData,
    [Parameter(Mandatory=$true)][string]$ProductDataRoot,
    [Parameter(Mandatory=$true)][string]$OutDir,
    [Parameter(Mandatory=$true)][ValidateRange(1,3600)][int]$ReadyTimeoutSeconds,
    [Parameter(Mandatory=$true)][ValidateRange(1,86400)][int]$FinalizeTimeoutSeconds
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'AutomaticCapture.Process.ps1')

function Write-ScenarioJson {
    param([string]$Path, $Value)
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($Value | ConvertTo-Json -Depth 30) + "`n")
    $file = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try { $file.Write($bytes,0,$bytes.Length); $file.Flush($true) } finally { $file.Dispose() }
}

function Get-ScenarioFileIdentity {
    param([string]$Path)
    $item = Get-Item -LiteralPath $Path -ErrorAction Stop
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "not a regular file: $Path" }
    return @{ path=$item.FullName; bytes=$item.Length; sha256=(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
}

function Assert-ScenarioNoReparse {
    param([string]$Path)
    $cursor = [IO.Path]::GetFullPath($Path)
    while ($cursor) {
        if (Test-Path -LiteralPath $cursor) {
            if ((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "reparse point in scenario path: $cursor" }
        }
        $cursor = [IO.Path]::GetDirectoryName($cursor)
    }
}

function Read-ScenarioJournal {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return }
    # The producer keeps a writable handle open; do not deny that writer sharing.
    $file = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
    try {
        $length = $file.Length
        if ($length -gt 128MB) { throw 'journal exceeds bounded scenario read limit (128 MiB)' }
        $bytes = [byte[]]::new([int]$length)
        $offset = 0
        while ($offset -lt $length) {
            $read = $file.Read($bytes,$offset,[int]$length-$offset)
            if ($read -eq 0) { throw 'journal truncated during sample' }
            $offset += $read
        }
        $complete = $bytes.Length
        while ($complete -gt 0 -and $bytes[$complete-1] -ne 10) { $complete-- }
        $raw = [Text.UTF8Encoding]::new($false,$true).GetString($bytes,0,$complete)
    } finally { $file.Dispose() }
    $lines = $raw -split "`n"
    # Do not observe an in-flight final line; final verification rejects one.
    for ($index=0; $index -lt $lines.Count-1; $index++) {
        if ($lines[$index].Length -gt 0) { $lines[$index] | ConvertFrom-Json }
    }
}

function Get-ScenarioProcessKey {
    param($ProcessIdentity)
    return "$($ProcessIdentity.pid):$($ProcessIdentity.creation_filetime)"
}

function Test-ScenarioJointRecording {
    param([object[]]$Games, [object[]]$Rows, [int]$PrefixCount, [int]$TargetCount)
    if ($Games.Count -ne $TargetCount) { return $false }
    $latest = @{}
    for ($index=$PrefixCount; $index -lt $Rows.Count; $index++) {
        $row = $Rows[$index]
        $latest["$($row.pid):$($row.process_creation_filetime)"] = [string]$row.state_to
    }
    foreach ($game in $Games) {
        if ($latest[(Get-ScenarioProcessKey $game)] -cne 'Recording') { return $false }
    }
    return $true
}

$RepoRoot = [IO.Path]::GetFullPath($RepoRoot).TrimEnd('\')
$ProductDataRoot = [IO.Path]::GetFullPath($ProductDataRoot).TrimEnd('\')
$OutDir = [IO.Path]::GetFullPath($OutDir).TrimEnd('\')
if ($OutDir -notmatch '^[Cc]:\\' -or $OutDir -ieq $RepoRoot -or $OutDir.StartsWith($RepoRoot+'\',[StringComparison]::OrdinalIgnoreCase) -or $OutDir -ieq $ProductDataRoot -or $OutDir.StartsWith($ProductDataRoot+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'scenario output must be on C: outside repository and product data root' }
if (Test-Path -LiteralPath $OutDir) { throw 'scenario output exists; choose a fresh directory and preserve this attempt' }
Assert-ScenarioNoReparse $OutDir
Assert-ScenarioNoReparse $ProductDataRoot
[void](New-Item -ItemType Directory -Path $OutDir)
$plan = [ordered]@{ schema='hoi4.automatic_capture_plan.v1'; scenario=$Scenario; mode=$Mode; status='blocked'; capture_seconds=60; ready_timeout_seconds=$ReadyTimeoutSeconds; finalize_timeout_seconds=$FinalizeTimeoutSeconds; game_exe=$GameExe; user_data=$UserData; product_data_root=$ProductDataRoot; out_dir=$OutDir; launch_arguments=@(); limitation='scenario evidence, not package-promotion or speed5/zero-loss acceptance' }
try {
    . (Join-Path $RepoRoot 'portable_replay/native/scripts/ProductionArtifact.Attestation.ps1')
    $admission = Test-ProductionArtifactAttestation -RepoRoot $RepoRoot -ReceiptPath $ArtifactAttestationPath
    Write-ScenarioJson (Join-Path $OutDir 'ARTIFACT_ADMISSION.json') $admission
    $retainedAttestation = Join-Path $OutDir 'CANDIDATE_ATTESTATION.json'
    [IO.File]::Copy($admission.receipt_path, $retainedAttestation, $false)
    if ((Get-ScenarioFileIdentity $retainedAttestation).sha256 -cne $admission.receipt_sha256) { throw 'candidate attestation changed while retaining it' }
    $python = @(Get-Command python.exe -CommandType Application -ErrorAction Stop)[0].Source
    $GameExe = (Resolve-Path -LiteralPath $GameExe).Path
    $UserData = (Resolve-Path -LiteralPath $UserData).Path
    $gameIdentity = Get-ScenarioFileIdentity $GameExe
    if ($gameIdentity.sha256 -cne $GameExeSha256) { throw 'explicit game executable hash differs' }
    $profilePath = Join-Path $UserData 'dlc_load.json'
    $profile = Get-Content -LiteralPath $profilePath -Raw | ConvertFrom-Json
    if ($null -eq $profile.enabled_mods -or @($profile.enabled_mods).Count -ne 0) { throw 'profile does not explicitly select zero enabled mods' }
    $knownRoot = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Battle Plan Recorder'
    if ($ProductDataRoot -ine $knownRoot) { throw 'explicit product root differs from the recorder Known Folder path' }
    $plan.recorder = $admission.artifacts['recorder_host']
    $plan.game_identity = $gameIdentity
    $plan.profile_identity = Get-ScenarioFileIdentity $profilePath
    $plan.status = 'prepared_start_conditions_not_observed'
} catch {
    $plan.error = $_.Exception.Message
    Write-ScenarioJson (Join-Path $OutDir 'PLAN.json') $plan
    throw
}
Write-ScenarioJson (Join-Path $OutDir 'PLAN.json') $plan
if ($Mode -ceq 'Plan') { return }

$observed = [ordered]@{ schema='hoi4.automatic_capture_observations.v1'; scenario=$Scenario; initial_games=@(); selected_games=@(); game_exe=$GameExe; user_data=$UserData; product_data_root=$ProductDataRoot; journal_prefix_records=0; preserved_files=@(); samples=@(); recorder=$null; stop_requested=$false; recorder_exit_code=$null; capture_window_started_ms=$null; capture_window_ended_ms=$null; runtime_error=$null }
$recorder = $null
$clock = [Diagnostics.Stopwatch]::StartNew()
try {
    Initialize-ScenarioProcessAccess
    $owner = [AutomaticScenarioProcess]::Read($PID)
    $journalPath = Join-Path $ProductDataRoot 'auto_attach_journal.jsonl'
    $beforeRows = @(Read-ScenarioJournal $journalPath)
    $journalBefore = [byte[]]@()
    if (Test-Path -LiteralPath $journalPath) { $journalBefore = [IO.File]::ReadAllBytes($journalPath) }
    [IO.File]::WriteAllBytes((Join-Path $OutDir 'JOURNAL_BEFORE.jsonl'), $journalBefore)
    $anchorBefore = [byte[]]@()
    if (Test-Path -LiteralPath "$journalPath.anchor") { $anchorBefore = [IO.File]::ReadAllBytes("$journalPath.anchor") }
    [IO.File]::WriteAllBytes((Join-Path $OutDir 'ANCHOR_BEFORE.jsonl'), $anchorBefore)
    & $python -B (Join-Path $PSScriptRoot 'automatic_capture_scenarios.py') --out-dir $OutDir --check-prefix > (Join-Path $OutDir 'PREFIX_PREFLIGHT.json')
    if ($LASTEXITCODE -ne 0) { throw 'retained journal/anchor prefix failed offline preflight' }
    $observed.journal_prefix_records = $beforeRows.Count
    $lastBefore = @{}
    foreach ($row in $beforeRows) { $lastBefore["$($row.pid):$($row.process_creation_filetime)"] = $row.state_to }
    if (@($lastBefore.Values | Where-Object { $_ -cnotin @('Complete','Failed','ProcessExitedBeforeAttach') }).Count) { throw 'prior journal has unfinished sessions; resolve ownership before a new scenario' }
    if (@(Get-Process -Name collector_record,'Battle Plan Recorder' -ErrorAction SilentlyContinue).Count) { throw 'a recorder already exists; it is not owned by this attempt' }
    $initial = @(Get-ScenarioGames $GameExe $owner)
    $observed.initial_games = $initial
    $targetCount = if ($Scenario -ceq 'simultaneous') { 2 } else { 1 }
    $initialCount = if ($Scenario -ceq 'app-first') { 0 } else { $targetCount }
    if ($initial.Count -ne $initialCount) { throw "scenario needs $initialCount already-running games; observed $($initial.Count). No game will be started or controlled." }
    if (Test-Path -LiteralPath $ProductDataRoot) {
        $observed.preserved_files = @(Get-ChildItem -LiteralPath $ProductDataRoot -File -Recurse | Where-Object { $_.Extension -in @('.json','.battleplan','.h4ct') } | ForEach-Object { Get-ScenarioFileIdentity $_.FullName })
    }
    # A candidate release is consumed, never built or repackaged here.
    $launchAdmission = Test-ProductionArtifactAttestation -RepoRoot $RepoRoot -ReceiptPath $ArtifactAttestationPath
    if ($launchAdmission.receipt_sha256 -cne $admission.receipt_sha256) { throw 'candidate attestation changed before recorder startup' }
    if ((Get-ScenarioFileIdentity $profilePath).sha256 -cne $plan.profile_identity.sha256) { throw 'profile changed before recorder startup' }
    if ((Get-ScenarioFileIdentity $GameExe).sha256 -cne $GameExeSha256) { throw 'game bytes changed before recorder startup' }
    $recorder = Start-Process -FilePath $plan.recorder -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $OutDir 'recorder.stdout.txt') -RedirectStandardError (Join-Path $OutDir 'recorder.stderr.txt')
    $observed.recorder = [AutomaticScenarioProcess]::Read($recorder.Id)
    Write-ScenarioJson (Join-Path $OutDir 'RECORDER_OWNERSHIP.json') $observed.recorder
    while ($true) {
        if ($recorder.HasExited) { throw 'recorder exited before the capture window completed' }
        $games = @(Get-ScenarioGames $GameExe $owner)
        if ($games.Count -gt $targetCount) { throw 'unexpected extra game process appeared' }
        $rows = @(Read-ScenarioJournal $journalPath)
        $sample = @{ elapsed_ms=[long]$clock.ElapsedMilliseconds; games=$games; journal_records=$rows.Count }
        $observed.samples += $sample
        Write-ScenarioJson (Join-Path $OutDir ('sample_{0:D6}.json' -f $observed.samples.Count)) $sample
        $joint = Test-ScenarioJointRecording $games $rows $beforeRows.Count $targetCount
        if ($null -eq $observed.capture_window_started_ms) {
            if ($joint) {
                $observed.selected_games = $games
                $observed.capture_window_started_ms = $sample.elapsed_ms
            } elseif ($clock.Elapsed.TotalSeconds -ge $ReadyTimeoutSeconds) { throw 'deadline waiting for actual game/campaign Recording state' }
        } elseif (-not $joint -or (@($games | ForEach-Object { Get-ScenarioProcessKey $_ } | Sort-Object) -join '|') -cne (@($observed.selected_games | ForEach-Object { Get-ScenarioProcessKey $_ } | Sort-Object) -join '|')) {
            throw 'game incarnation or joint Recording state changed during the capture window'
        }
        if ($null -ne $observed.capture_window_started_ms -and $sample.elapsed_ms - $observed.capture_window_started_ms -ge 60000) {
            $observed.capture_window_ended_ms = $sample.elapsed_ms
            break
        }
        Start-Sleep -Milliseconds 500
    }
} catch { $observed.runtime_error = $_.Exception.Message }
finally {
    if ($null -ne $recorder -and $null -ne $observed.recorder) {
        $finalize = [Diagnostics.Stopwatch]::StartNew()
        try {
            while (-not $recorder.HasExited -and $finalize.Elapsed.TotalSeconds -lt $FinalizeTimeoutSeconds) {
                if ([AutomaticScenarioProcess]::RequestRecorderStop($recorder.Id,$observed.recorder.creation_filetime,$plan.recorder)) { $observed.stop_requested = $true }
                Start-Sleep -Milliseconds 500
                $recorder.Refresh()
            }
            if ($recorder.HasExited) { $recorder.WaitForExit(); $observed.recorder_exit_code = $recorder.ExitCode }
            else { $observed.runtime_error = "$($observed.runtime_error); finalization deadline exceeded; owned recorder retained, no forced termination" }
        } catch { $observed.runtime_error = "$($observed.runtime_error); graceful-stop failure: $($_.Exception.Message)" }
    }
    Write-ScenarioJson (Join-Path $OutDir 'OBSERVATIONS.json') $observed
}
& $python -B (Join-Path $PSScriptRoot 'automatic_capture_scenarios.py') --out-dir $OutDir
exit $LASTEXITCODE
