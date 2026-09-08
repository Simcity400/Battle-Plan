<#
.SYNOPSIS
Observe parent-established campaign transitions without controlling HOI4.
.DESCRIPTION
Plan validates explicit inputs and never launches. Run starts only the attested
production recorder, with no arguments, then samples fresh parent checkpoints.
The parent establishes each approved reload/relaunch/rollback/identity change.
No game launch, input, focus, speed, pause, affinity, or termination occurs here.
Only the recorder started here receives a cooperative close, via the unchanged
AutomaticCapture.Process.ps1 helper. Deadlines retain unresolved processes.
This is a separate scenario; it does not change or claim the startup stable 60s
window, a capture-validation receipt, maximum-speed capture, or release approval.

Inputs: -ExpectationsPath is hoi4.campaign_transition_expectations.v1 JSON:
  checkpoints: ordered objects {name, scope_file, recording, segment_id,logical_stream_id,
    campaign:{game_unique_id:<canonical UUID>,game_unique_seed:<signed decimal>}}
  transitions: one for EVERY adjacent pair {from,to,
    kind:reload|relaunch|rollback|identity-change, attachment:same|replaced}
  recordings: {name,publication:{kind:producer-stitch,checkpoint:<last checkpoint for this recording>},
    logical_stream_id:null,expected:{segments:[...],main_slices:[...],
    main_head:{segment_id,hour_index},coverage:[...],surviving_hours:[...],
    playback_gaps:[{hour_index,gap_inputs,recorder_domain_problems,
                   coverage_warnings,map_authority_error}]}}
Each expected segment has exactly: segment_id,parent_segment,cutover_hour,
head_hour,start_reason,stop_reason,visible_spans,superseded_spans. Values use
the producer segments.json shape. All coverage/gaps are exact, including empty
arrays; no tolerance or expected-zero default. Publication references need no
future attempt UUID or path. Once the recorder completes, the helper joins the
checkpoint's anchored Complete journal, append ticket and admitted member to
that attempt's worker_stderr.log publication path/hash and complete campaign.
Ambiguous, escaped, cross-attempt, wrong-campaign and missing publications refuse.
Publication replaces the old path field; existing files also need this reference.
Same campaign selects the SAME complete stitched publication, across relaunches.
Independent logical-stream package members cannot substitute for that campaign.

Handshake: Run writes WAITING_<name>.json with an observation_id. Once actual
approved game conditions are established, parent atomically creates that stage's
previously absent scope_file, containing:
  {observation_id:<from WAITING>,checkpoint:<name>,pid:<integer>,
   creation_filetime:<decimal string>,attempt_id:<actual journal attempt>,
   session_id:<actual journal session>,manifest_path:<producer member manifest>}
The manifest must be within ProductDataRoot/run_<attempt_id>. It may still be
awaiting finalization. The observer verifies the actual live process and its
Recording journal binding and the contemporaneous producer rollover-state and
append ticket, then writes checkpoint_<name>.json. Parent waits for
that acknowledgement before the next transition. After the final acknowledgement
the recorder closes gracefully; the observer retains manifests and
the complete campaign files and runs existing product admission, whole playback,
and --report-recording-reach --report-hours all. Raw source bytes are retained
from the admitted campaign's declared embedded sources; trimmed local spools are
not required. Parent should submit the final
checkpoint only after the requested final ticks were actually recorded.
The retained <recording>.publication.json records the actual path binding and
producer log/hash custody. Binding never changes the semantic EXPECTATIONS.json.

Use Plan first, then a different fresh OutDir for Run. Both require explicit
central candidate attestation, game executable/hash, game/profile/product roots,
checkpoint deadline, finalization deadline, and product command deadline. Store
OutDir on C: outside the repository and product root. Missing inputs yield a
durable refusal. The returned receipt names evidence limitations and all files.
.EXAMPLE
& .\extractor\native\scripts\Invoke-CampaignTransitionObservation.ps1 `
  -Mode Plan -RepoRoot C:\work\hoi4 -ExpectationsPath C:\evidence\expected.json `
  -ArtifactAttestationPath C:\candidate\hoi4-production-prebuilt-artifacts.json `
  -GameExe 'X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe' `
  -GameExeSha256 <actual-sha256> -UserData C:\explicit\profile `
  -ProductDataRoot "$env:LOCALAPPDATA\Battle Plan Recorder" `
  -OutDir C:\evidence\transition-plan -CheckpointTimeoutSeconds 300 `
  -FinalizeTimeoutSeconds 1800 -ProductTimeoutSeconds 1800
#>
[CmdletBinding()]
param(
    [ValidateSet('Plan','Run')][string]$Mode = 'Plan',
    [Parameter(Mandatory=$true)][string]$RepoRoot,
    [Parameter(Mandatory=$true)][string]$ExpectationsPath,
    [Parameter(Mandatory=$true)][string]$ArtifactAttestationPath,
    [Parameter(Mandatory=$true)][string]$GameExe,
    [Parameter(Mandatory=$true)][ValidatePattern('^[0-9a-f]{64}$')][string]$GameExeSha256,
    [Parameter(Mandatory=$true)][string]$UserData,
    [Parameter(Mandatory=$true)][string]$ProductDataRoot,
    [Parameter(Mandatory=$true)][string]$OutDir,
    [Parameter(Mandatory=$true)][ValidateRange(1,86400)][int]$CheckpointTimeoutSeconds,
    [Parameter(Mandatory=$true)][ValidateRange(1,86400)][int]$FinalizeTimeoutSeconds,
    [Parameter(Mandatory=$true)][ValidateRange(1,86400)][int]$ProductTimeoutSeconds
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'AutomaticCapture.Process.ps1')

function Write-TransitionJson {
    param([string]$Path, $Value)
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($Value | ConvertTo-Json -Depth 60) + "`n")
    $file = [IO.File]::Open($Path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::Read)
    try { $file.Write($bytes,0,$bytes.Length); $file.Flush($true) } finally { $file.Dispose() }
}

function Assert-TransitionPath {
    param([string]$Path)
    $cursor = [IO.Path]::GetFullPath($Path)
    while ($cursor) {
        if ((Test-Path -LiteralPath $cursor) -and ((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "reparse point: $cursor" }
        $cursor = [IO.Path]::GetDirectoryName($cursor)
    }
}

function Read-TransitionBytes {
    param([string]$Path, [switch]$AllowAbsent)
    Assert-TransitionPath $Path
    if ($AllowAbsent -and -not (Test-Path -LiteralPath $Path)) { return ,([byte[]]@()) }
    # Rollover state is atomically replaced by the producer. Share deletion so
    # observing that file cannot deny its publication rename.
    $file = [IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
    try {
        if ($file.Length -gt 128MB) { throw 'input exceeds 128 MiB read bound' }
        $bytes = [byte[]]::new([int]$file.Length)
        $offset = 0
        while ($offset -lt $bytes.Length) {
            $count = $file.Read($bytes,$offset,$bytes.Length-$offset)
            if ($count -eq 0) { throw 'input truncated during read' }
            $offset += $count
        }
        return ,$bytes
    } finally { $file.Dispose() }
}

function Get-TransitionRows {
    param([byte[]]$Bytes)
    $end = $Bytes.Length
    while ($end -gt 0 -and $Bytes[$end-1] -ne 10) { $end-- }
    $raw = [Text.UTF8Encoding]::new($false,$true).GetString($Bytes,0,$end)
    foreach ($line in ($raw -split "`n")) { if ($line) { $line | ConvertFrom-Json } }
}

$RepoRoot = [IO.Path]::GetFullPath($RepoRoot).TrimEnd('\')
$OutDir = [IO.Path]::GetFullPath($OutDir).TrimEnd('\')
$ProductDataRoot = [IO.Path]::GetFullPath($ProductDataRoot).TrimEnd('\')
foreach ($parent in @($RepoRoot,$ProductDataRoot)) {
    if ($OutDir -ieq $parent -or $OutDir.StartsWith($parent+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'output must be outside repository and product root' }
}
if ($OutDir -notmatch '^[Cc]:\\' -or (Test-Path -LiteralPath $OutDir)) { throw 'choose a fresh output directory on C:' }
Assert-TransitionPath $OutDir
[void](New-Item -ItemType Directory -Path $OutDir)
$plan = [ordered]@{ schema='hoi4.campaign_transition_plan.v1'; mode=$Mode; observation_id=[guid]::NewGuid().ToString(); status='refused'; product_data_root=$ProductDataRoot; game_exe=$GameExe; game_sha256=$GameExeSha256; user_data=$UserData; product_timeout_seconds=$ProductTimeoutSeconds; checkpoint_timeout_seconds=$CheckpointTimeoutSeconds; finalize_timeout_seconds=$FinalizeTimeoutSeconds; recorder_argv=@(); invocation=$PSBoundParameters }
$observed = [ordered]@{ runtime_error=$null; recorder=$null; recorder_launch_pid=$null; stop_requested=$false; recorder_exit_code=$null }
$recorder = $null
$python = $null
try {
    $python = @(Get-Command python.exe -CommandType Application -ErrorAction Stop)[0].Source
    $helper = Join-Path $PSScriptRoot 'campaign_transition_observation.py'
    $plan.harness_files = @(@($PSCommandPath,$helper,(Join-Path $PSScriptRoot 'AutomaticCapture.Process.ps1'),(Join-Path $PSScriptRoot 'automatic_capture_scenarios.py'),(Join-Path $PSScriptRoot 'capture_scorecard.py'),(Join-Path $PSScriptRoot 'gap_ledger.py')) | ForEach-Object {
        @{ path=$_; bytes=(Get-Item -LiteralPath $_).Length; sha256=(Get-FileHash -LiteralPath $_).Hash.ToLowerInvariant() }
    })
    $plan.python = @{ path=$python; sha256=(Get-FileHash -LiteralPath $python).Hash.ToLowerInvariant() }
    Assert-TransitionPath $ExpectationsPath
    [IO.File]::WriteAllBytes((Join-Path $OutDir 'EXPECTATIONS.json'), (Read-TransitionBytes $ExpectationsPath))
    & $python -B $helper --out-dir $OutDir --check-expectations (Join-Path $OutDir 'EXPECTATIONS.json') > (Join-Path $OutDir 'expectations.stdout.txt') 2> (Join-Path $OutDir 'expectations.stderr.txt')
    if ($LASTEXITCODE -ne 0) { throw 'invalid or missing exact expectations; see expectations logs' }
    $spec = Get-Content -LiteralPath (Join-Path $OutDir 'EXPECTATIONS.json') -Raw | ConvertFrom-Json
    foreach ($stage in $spec.checkpoints) {
        Assert-TransitionPath $stage.scope_file
        if (Test-Path -LiteralPath $stage.scope_file) { throw 'checkpoint files must be fresh; parent publishes them after WAITING acknowledgement' }
    }
    . (Join-Path $RepoRoot 'portable_replay/native/scripts/ProductionArtifact.Attestation.ps1')
    $admission = Test-ProductionArtifactAttestation -RepoRoot $RepoRoot -ReceiptPath $ArtifactAttestationPath
    Write-TransitionJson (Join-Path $OutDir 'ARTIFACT_ADMISSION.json') $admission
    [IO.File]::Copy($admission.receipt_path,(Join-Path $OutDir 'CANDIDATE_ATTESTATION.json'),$false)
    if ((Get-FileHash -LiteralPath (Join-Path $OutDir 'CANDIDATE_ATTESTATION.json')).Hash.ToLowerInvariant() -cne $admission.receipt_sha256) { throw 'attestation changed during retention' }
    $GameExe = (Resolve-Path -LiteralPath $GameExe).Path
    $UserData = (Resolve-Path -LiteralPath $UserData).Path
    Assert-TransitionPath $GameExe
    Assert-TransitionPath $ProductDataRoot
    if ((Get-FileHash -LiteralPath $GameExe).Hash.ToLowerInvariant() -cne $GameExeSha256) { throw 'explicit game executable hash differs' }
    $knownRoot = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Battle Plan Recorder'
    if ($ProductDataRoot -ine $knownRoot) { throw 'product root differs from recorder Known Folder' }
    $plan.game_exe = $GameExe
    $plan.game_root = Split-Path -Parent $GameExe
    $plan.user_data = $UserData
    $plan.status = 'prepared_conditions_not_observed'
    Write-TransitionJson (Join-Path $OutDir 'PLAN.json') $plan
    if ($Mode -ceq 'Plan') { return }

    Initialize-ScenarioProcessAccess
    $owner = [AutomaticScenarioProcess]::Read($PID)
    if (@(Get-Process -Name collector_record,'Battle Plan Recorder' -ErrorAction SilentlyContinue).Count) { throw 'existing recorder is not owned by this attempt' }
    $journal = Join-Path $ProductDataRoot 'auto_attach_journal.jsonl'
    foreach ($entry in @(@('JOURNAL_BEFORE.jsonl',$journal),@('ANCHOR_BEFORE.jsonl',"$journal.anchor"))) {
        [IO.File]::WriteAllBytes((Join-Path $OutDir $entry[0]),(Read-TransitionBytes $entry[1] -AllowAbsent))
    }
    & $python -B $helper --out-dir $OutDir --check-prefix > (Join-Path $OutDir 'prefix.stdout.txt') 2> (Join-Path $OutDir 'prefix.stderr.txt')
    if ($LASTEXITCODE -ne 0) { throw 'preexisting journal custody or unfinished sessions refused' }
    $prefixCount = @(Get-TransitionRows (Read-TransitionBytes $journal -AllowAbsent)).Count
    $launchAdmission = Test-ProductionArtifactAttestation -RepoRoot $RepoRoot -ReceiptPath $ArtifactAttestationPath
    if ($launchAdmission.receipt_sha256 -cne $admission.receipt_sha256) { throw 'attestation changed before launch' }
    Write-TransitionJson (Join-Path $OutDir 'recorder.argv.json') @{ executable=$admission.artifacts['recorder_host']; argv=@(); sha256=$admission.artifact_sha256['recorder_host'] }
    $recorder = Start-Process -FilePath $admission.artifacts['recorder_host'] -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $OutDir 'recorder.stdout.txt') -RedirectStandardError (Join-Path $OutDir 'recorder.stderr.txt')
    $observed.recorder_launch_pid = $recorder.Id
    $observed.recorder = [AutomaticScenarioProcess]::Read($recorder.Id)
    Write-TransitionJson (Join-Path $OutDir 'RECORDER_OWNERSHIP.json') $observed.recorder
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $previousGame = $null
    foreach ($stage in $spec.checkpoints) {
        $deadline = [Diagnostics.Stopwatch]::StartNew()
        Write-TransitionJson (Join-Path $OutDir ('WAITING_'+$stage.name+'.json')) @{ observation_id=$plan.observation_id; checkpoint=$stage.name; scope_file=$stage.scope_file }
        while ($true) {
            if ($recorder.HasExited) { throw 'recorder exited before final checkpoint' }
            if ($deadline.Elapsed.TotalSeconds -ge $CheckpointTimeoutSeconds) { throw "checkpoint deadline: $($stage.name)" }
            if (Test-Path -LiteralPath $stage.scope_file) {
                $scopeBytes = Read-TransitionBytes $stage.scope_file
                $scope = [Text.UTF8Encoding]::new($false,$true).GetString($scopeBytes) | ConvertFrom-Json
                if ($scope.observation_id -cne $plan.observation_id -or $scope.checkpoint -cne $stage.name) { throw 'stale or out-of-order parent checkpoint' }
                $games = @(Get-ScenarioGames $GameExe $owner)
                $selected = @($games | Where-Object { $_.pid -eq $scope.pid -and $_.creation_filetime -ceq $scope.creation_filetime })
                if ($selected.Count -ne 1) { throw 'parent attachment is not actually alive in scoped census' }
                $bytes = Read-TransitionBytes $journal
                $rows = @(Get-TransitionRows $bytes)
                $matching = @($rows | Select-Object -Skip $prefixCount | Where-Object { $_.pid -eq $scope.pid -and $_.process_creation_filetime -ceq $scope.creation_filetime })
                if ($matching.Count -gt 0 -and $matching[-1].state_to -ceq 'Recording' -and $matching[-1].attempt_id -ceq $scope.attempt_id -and $matching[-1].session_id -ceq $scope.session_id) {
                    if ($scope.attempt_id -cnotmatch '^[A-Za-z0-9_-]+$') { throw 'unsafe producer attempt id' }
                    $run = Join-Path $ProductDataRoot ('run_'+$scope.attempt_id)
                    $statePath = Join-Path $run 'logical-streams/rollover-state.json'
                    $ticketPath = Join-Path $run ('stitching/tickets/{0:D20}.json' -f [ulong]$stage.logical_stream_id)
                    if (-not (Test-Path -LiteralPath $statePath) -or -not (Test-Path -LiteralPath $ticketPath)) {
                        Start-Sleep -Milliseconds 500
                        continue
                    }
                    $stateBefore = Read-TransitionBytes $statePath
                    $state = [Text.UTF8Encoding]::new($false,$true).GetString($stateBefore) | ConvertFrom-Json
                    if ($state.schema -cne 'cid1-rollover-fs-v1' -or $state.phase -cne 'streaming' -or $state.logical_stream_id -ne $stage.logical_stream_id) { throw 'parent checkpoint does not name the currently streaming producer epoch' }
                    $ticketBytes = Read-TransitionBytes $ticketPath
                    $stateAfter = Read-TransitionBytes $statePath
                    if ([Convert]::ToBase64String($stateBefore) -cne [Convert]::ToBase64String($stateAfter)) { throw 'producer epoch changed while observing checkpoint' }
                    $gameAfter = [AutomaticScenarioProcess]::Read([int]$scope.pid)
                    if ($gameAfter.creation_filetime -cne $scope.creation_filetime -or $gameAfter.exe_path -ine $GameExe) { throw 'attachment changed across checkpoint epoch observation' }
                    $absent = $false
                    if ($null -ne $previousGame) { $absent = @($games | Where-Object { $_.pid -eq $previousGame.pid -and $_.creation_filetime -ceq $previousGame.creation_filetime }).Count -eq 0 }
                    [IO.File]::WriteAllBytes((Join-Path $OutDir ('checkpoint_'+$stage.name+'.scope.json')),$scopeBytes)
                    [IO.File]::WriteAllBytes((Join-Path $OutDir ('checkpoint_'+$stage.name+'.journal.jsonl')),$bytes)
                    [IO.File]::WriteAllBytes((Join-Path $OutDir ('checkpoint_'+$stage.name+'.rollover-before.json')),$stateBefore)
                    [IO.File]::WriteAllBytes((Join-Path $OutDir ('checkpoint_'+$stage.name+'.rollover-after.json')),$stateAfter)
                    [IO.File]::WriteAllBytes((Join-Path $OutDir ('checkpoint_'+$stage.name+'.ticket.json')),$ticketBytes)
                    Write-TransitionJson (Join-Path $OutDir ('checkpoint_'+$stage.name+'.json')) @{ scope=$scope; game=$selected[0]; games=$games; elapsed_ms=[long]$clock.ElapsedMilliseconds; journal_records=$rows.Count; previous_attachment_absent=$absent }
                    $previousGame = $selected[0]
                    break
                }
            }
            Start-Sleep -Milliseconds 500
        }
    }
} catch {
    $observed.runtime_error = $_.Exception.Message
    if (-not (Test-Path -LiteralPath (Join-Path $OutDir 'PLAN.json'))) {
        $plan.error = $observed.runtime_error
        Write-TransitionJson (Join-Path $OutDir 'PLAN.json') $plan
    }
} finally {
    if ($null -ne $recorder -and $null -ne $observed.recorder) {
        try {
            $deadline = [Diagnostics.Stopwatch]::StartNew()
            while (-not $recorder.HasExited -and $deadline.Elapsed.TotalSeconds -lt $FinalizeTimeoutSeconds) {
                if ([AutomaticScenarioProcess]::RequestRecorderStop($recorder.Id,$observed.recorder.creation_filetime,$admission.artifacts['recorder_host'])) { $observed.stop_requested = $true }
                Start-Sleep -Milliseconds 500
                $recorder.Refresh()
            }
            if ($recorder.HasExited) { $recorder.WaitForExit(); $observed.recorder_exit_code = $recorder.ExitCode }
            else { throw 'finalization deadline exceeded; recorder retained without forced termination' }
        } catch { $observed.runtime_error = "$($observed.runtime_error); graceful finalization: $($_.Exception.Message)" }
    }
    if ($Mode -ceq 'Run' -or $observed.runtime_error) { Write-TransitionJson (Join-Path $OutDir 'OBSERVATIONS.json') $observed }
}
if ($Mode -ceq 'Plan' -or $null -eq $python) {
    Write-TransitionJson (Join-Path $OutDir 'TRANSITION_RECEIPT.json') @{ schema='hoi4.campaign_transition_observation.v1'; status='refused'; issues=@($observed.runtime_error) }
    exit 1
}
& $python -B $helper --out-dir $OutDir > (Join-Path $OutDir 'assessment.stdout.txt') 2> (Join-Path $OutDir 'assessment.stderr.txt')
exit $LASTEXITCODE
