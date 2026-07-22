[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, ParameterSetName = 'Regrade')]
    [string] $CaptureJson,
    [Parameter(Mandatory = $true, ParameterSetName = 'Capture')]
    [string] $RecorderExe,
    [Parameter(Mandatory = $true, ParameterSetName = 'Capture')]
    [string] $Hoi4Exe,
    [Parameter(Mandatory = $true, ParameterSetName = 'Capture')]
    [string] $PdxUserDir,
    [Parameter(Mandatory = $true, ParameterSetName = 'Capture')]
    [string] $CollectorDll,
    [Parameter(Mandatory = $true, ParameterSetName = 'Capture')]
    [string] $OutDir,
    [Parameter(Mandatory = $true, ParameterSetName = 'Capture')]
    [string] $ActiveFixtureEventPath,
    [Parameter(Mandatory = $true, ParameterSetName = 'Capture')]
    [ValidatePattern('^[A-Z0-9]{3}$')] [string] $OriginatorTag,
    [Parameter(Mandatory = $true, ParameterSetName = 'Capture')]
    [ValidatePattern('^[A-Z0-9]{3}$')] [string] $RecipientTag,
    [Parameter(ParameterSetName = 'Capture')]
    [string] $ContinueSave,
    [Parameter(ParameterSetName = 'Capture')]
    [int] $DurationSeconds = 300,
    [Parameter(ParameterSetName = 'Capture')]
    [int] $SampleMilliseconds = 25,
    [Parameter(ParameterSetName = 'Capture')]
    [string[]] $EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string] $OutputJson = '',
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Write-AtomicReplaceBytes([string]$Path, [byte[]]$Bytes) {
    $full = [IO.Path]::GetFullPath($Path); $parent = [IO.Path]::GetDirectoryName($full)
    $tmp = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllBytes($tmp, $Bytes)
        $stream=[IO.File]::Open($tmp,'Open','ReadWrite','None'); try{$stream.Flush($true)}finally{$stream.Dispose()}
        if ([IO.File]::Exists($full)) { [IO.File]::Replace($tmp, $full, $null, $true) }
        else { [IO.File]::Move($tmp, $full) }
    } finally { if ([IO.File]::Exists($tmp)) { [IO.File]::Delete($tmp) } }
}

$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$verifier = Join-Path $repo "extractor\native\lanes\scripted_state_flags_variables_live\verify_a19_delayed_event_lifecycle.py"
$actionDriver = Join-Path $PSScriptRoot 'Invoke-A18A19AutonomousActionDriver.ps1'
$fullFixtureSource = Join-Path $repo 'extractor\native\fixtures\a19\events\a19_delayed_event_fixture_full.txt'
$cancelledFixtureSource = Join-Path $repo 'extractor\native\fixtures\a19\events\a19_delayed_event_fixture_cancelled.txt'
foreach ($tool in @($verifier, $actionDriver, $fullFixtureSource, $cancelledFixtureSource)) {
    if (-not (Test-Path -LiteralPath $tool -PathType Leaf)) { throw "A19 tool is missing: $tool" }
}

if ($PSCmdlet.ParameterSetName -ceq 'Regrade') {
    $capture = (Resolve-Path -LiteralPath $CaptureJson).Path
    $arguments = @($verifier, $capture)
    if (-not [string]::IsNullOrWhiteSpace($OutputJson)) {
        $arguments += @('--output', [System.IO.Path]::GetFullPath($OutputJson))
    }
    & $Python @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "A19 delayed-event lifecycle regrade failed with exit code $LASTEXITCODE"
    }
    return
}

foreach ($path in @($RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required path does not exist: $path"
    }
}
if ($OriginatorTag -ceq $RecipientTag) { throw 'A19 OriginatorTag and RecipientTag must differ.' }
$resolvedActiveFixture = [IO.Path]::GetFullPath($ActiveFixtureEventPath)
if (-not (Test-Path -LiteralPath $resolvedActiveFixture -PathType Leaf)) {
    throw 'ActiveFixtureEventPath must be an existing event file in the selected product modset.'
}
if (-not [string]::IsNullOrWhiteSpace($ContinueSave) -and
    -not (Test-Path -LiteralPath $ContinueSave -PathType Leaf)) {
    throw "Continue-save does not exist: $ContinueSave"
}
if ($DurationSeconds -lt 60 -or $SampleMilliseconds -lt 10) {
    throw 'DurationSeconds must be >= 60 and SampleMilliseconds must be >= 10.'
}

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$resolvedRecorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedPdxUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedOut = [System.IO.Path]::GetFullPath($OutDir)
$observedExeSha256 = (Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($observedExeSha256 -cne $expectedExeSha256) {
    throw "Hoi4Exe is not exact a729 authority: $observedExeSha256"
}
$existing = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
if ($existing.Count -ne 0) {
    throw "A19 requires a clean early launch; close the game PID(s) $(@($existing.Id) -join ',') first."
}
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or
        @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty so stale A19 evidence cannot be selected: $resolvedOut"
    }
} else {
    [System.IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
}

$extractor = Join-Path $PSScriptRoot 'extract_f6_replay_snapshots.py'
if (-not (Test-Path -LiteralPath $extractor -PathType Leaf)) {
    throw "Folded-snapshot extractor is missing: $extractor"
}
$captureManifest = Join-Path $resolvedOut 'native_session_manifest.json'
$snapshotDirectory = Join-Path $resolvedOut 'a19_folded_snapshots'
$extractionReport = Join-Path $resolvedOut 'a19_snapshot_extraction.json'
$verification = if ([string]::IsNullOrWhiteSpace($OutputJson)) {
    Join-Path $resolvedOut 'a19_delayed_event_lifecycle_verification.json'
} else {
    [System.IO.Path]::GetFullPath($OutputJson)
}
$actionReceipt = Join-Path $resolvedOut 'a19_autonomous_action_receipt.json'
$actionStdout = Join-Path $resolvedOut 'a19_autonomous_action.stdout.log'
$actionStderr = Join-Path $resolvedOut 'a19_autonomous_action.stderr.log'
$recorderStdout = Join-Path $resolvedOut 'collector_record.stdout.log'
$recorderStderr = Join-Path $resolvedOut 'collector_record.stderr.log'
$acquisitionReceipt = Join-Path $resolvedOut 'a19_autonomous_acquisition_receipt.json'
$activeFixturePriorBytes = [IO.File]::ReadAllBytes($resolvedActiveFixture)
$activeFixturePriorSha256 = (Get-FileHash -LiteralPath $resolvedActiveFixture -Algorithm SHA256).Hash.ToLowerInvariant()
$activeFixtureFullSha256 = $null

$continueGamePath = Join-Path $resolvedPdxUserDir 'continue_game.json'
$continueGamePriorExists = Test-Path -LiteralPath $continueGamePath -PathType Leaf
$continueGamePriorBytes = if ($continueGamePriorExists) {
    [System.IO.File]::ReadAllBytes($continueGamePath)
} else {
    $null
}
$continueGameOverridden = $false
$continueGameReplacement = $null
if (-not [string]::IsNullOrWhiteSpace($ContinueSave)) {
    $continueIndex = -1
    $gdprIndex = -1
    for ($index = 0; $index -lt $EarlyLaunchArgument.Count; $index++) {
        if ($EarlyLaunchArgument[$index] -ieq '--continuelastsave') {
            if ($continueIndex -ge 0) {
                throw '-EarlyLaunchArgument must contain --continuelastsave exactly once.'
            }
            $continueIndex = $index
        }
        if ($EarlyLaunchArgument[$index] -ieq '--gdpr-compliant') {
            if ($gdprIndex -ge 0) {
                throw '-EarlyLaunchArgument must contain --gdpr-compliant exactly once.'
            }
            $gdprIndex = $index
        }
    }
    if ($continueIndex -lt 0 -or $gdprIndex -lt $continueIndex) {
        throw '-ContinueSave requires launcher-effective --continuelastsave followed by --gdpr-compliant.'
    }
    $resolvedContinueSave = (Resolve-Path -LiteralPath $ContinueSave).Path
    $saveDirectory = [System.IO.Path]::GetFullPath((Join-Path $resolvedPdxUserDir 'save games'))
    $requestedDirectory = [System.IO.Path]::GetFullPath((Split-Path -Parent $resolvedContinueSave))
    if (-not [string]::Equals(
        $saveDirectory.TrimEnd([char]'\'),
        $requestedDirectory.TrimEnd([char]'\'),
        [System.StringComparison]::OrdinalIgnoreCase
    )) {
        throw "-ContinueSave must be a direct child of $saveDirectory"
    }
    $continueObject = if ($continueGamePriorExists) {
        Get-Content -LiteralPath $continueGamePath -Raw | ConvertFrom-Json
    } else {
        [pscustomobject]@{
            title = [System.IO.Path]::GetFileNameWithoutExtension($resolvedContinueSave)
            desc = ''
            date = [DateTime]::UtcNow.ToString('R')
            filename = ''
            is_remote = $false
        }
    }
    $continueObject.filename = [System.IO.Path]::GetFileName($resolvedContinueSave)
    $continueObject.is_remote = $false
    $continueGameReplacement = ($continueObject | ConvertTo-Json -Depth 8) + [Environment]::NewLine
}

$arguments = @(
    '--early-launch-exe', $resolvedExe,
    '--pdx-user-dir', $resolvedPdxUserDir,
    '--dll', $resolvedCollector,
    '--out-dir', $resolvedOut,
    '--duration-s', $DurationSeconds,
    '--sample-ms', $SampleMilliseconds,
    '--min-game-hours', '4',
    '--stream-format', 'compact',
    '--profile', 'debug',
    '--scheduled-domain', 'delayed_events_live',
    '--scheduled-domains-exact',
    '--sidecar-log',
    '--hour-latch'
)
foreach ($value in $EarlyLaunchArgument) {
    $arguments += @('--early-launch-arg', $value)
}

$recorder = $null
try {
    Write-AtomicReplaceBytes $resolvedActiveFixture ([IO.File]::ReadAllBytes($fullFixtureSource))
    $activeFixtureFullSha256 = (Get-FileHash -LiteralPath $resolvedActiveFixture -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($null -ne $continueGameReplacement) {
        $continueGameOverridden = $true
        Write-AtomicReplaceBytes $continueGamePath ([Text.UTF8Encoding]::new($false).GetBytes($continueGameReplacement))
    }
    $startInfo=[Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName=$resolvedRecorder; $startInfo.UseShellExecute=$false; $startInfo.CreateNoWindow=$true
    $startInfo.RedirectStandardOutput=$true; $startInfo.RedirectStandardError=$true
    $startInfo.Environment['BATTLE_PLAN_ENABLE_A19_DELAYED_EVENT_EVIDENCE']='1'
    foreach($value in $arguments){[void]$startInfo.ArgumentList.Add([string]$value)}
    $recorder=[Diagnostics.Process]::new(); $recorder.StartInfo=$startInfo
    if(-not $recorder.Start()){throw 'collector_record did not start.'}
    $recorderOutTask=$recorder.StandardOutput.ReadToEndAsync(); $recorderErrTask=$recorder.StandardError.ReadToEndAsync()
    $launchDeadline=[DateTime]::UtcNow.AddMinutes(5); $game=$null
    do {
        Start-Sleep -Milliseconds 250
        $candidates=@(Get-Process -Name the game -ErrorAction SilentlyContinue)
        if($candidates.Count -eq 1 -and $candidates[0].MainWindowHandle -ne [IntPtr]::Zero){$game=$candidates[0]}
        if($recorder.HasExited){throw "collector_record exited during launch with code $($recorder.ExitCode)"}
    } while($null -eq $game -and [DateTime]::UtcNow -lt $launchDeadline)
    if($null -eq $game){throw 'A19 early launch did not produce one owned the game window.'}
    $actionArgs=@(
        '-NoProfile','-ExecutionPolicy','Bypass','-File',$actionDriver,
        '-Mode','A19DelayedEvent','-ProcessId',[string]$game.Id,
        '-ExpectedProcessStartUtc',$game.StartTime.ToUniversalTime().ToString('o'),
        '-ReceiptPath',$actionReceipt,'-OriginatorTag',$OriginatorTag,'-RecipientTag',$RecipientTag,
        '-ActiveEventDefinitionPath',$resolvedActiveFixture,'-CancelledEventDefinitionSource',$cancelledFixtureSource,
        '-ActionTimeoutSeconds',[string]([Math]::Max(60,$DurationSeconds-15))
    )
    $actionInfo=[Diagnostics.ProcessStartInfo]::new();$actionInfo.FileName='powershell.exe';$actionInfo.UseShellExecute=$false;$actionInfo.CreateNoWindow=$true
    $actionInfo.RedirectStandardOutput=$true;$actionInfo.RedirectStandardError=$true
    foreach($value in $actionArgs){[void]$actionInfo.ArgumentList.Add([string]$value)}
    $action=[Diagnostics.Process]::new();$action.StartInfo=$actionInfo
    if(-not $action.Start()){throw 'A19 autonomous action driver did not start.'}
    $actionOutTask=$action.StandardOutput.ReadToEndAsync();$actionErrTask=$action.StandardError.ReadToEndAsync();$action.WaitForExit()
    Write-AtomicReplaceBytes $actionStdout ([Text.UTF8Encoding]::new($false).GetBytes($actionOutTask.Result));Write-AtomicReplaceBytes $actionStderr ([Text.UTF8Encoding]::new($false).GetBytes($actionErrTask.Result))
    if($action.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $actionReceipt -PathType Leaf)){throw "A19 autonomous action driver failed with code $($action.ExitCode); inspect $actionStderr"}
    $action.Dispose()
    $recorder.WaitForExit()
    Write-AtomicReplaceBytes $recorderStdout ([Text.UTF8Encoding]::new($false).GetBytes($recorderOutTask.Result));Write-AtomicReplaceBytes $recorderStderr ([Text.UTF8Encoding]::new($false).GetBytes($recorderErrTask.Result))
    if($recorder.ExitCode -ne 0){throw "collector_record exited with code $($recorder.ExitCode); inspect $recorderStderr"}
    $recorder.Dispose();$recorder=$null
} finally {
    if($null -ne $recorder){
        if(-not $recorder.HasExited){$recorder.WaitForExit()}
        $recorder.Dispose()
    }
    Write-AtomicReplaceBytes $resolvedActiveFixture $activeFixturePriorBytes
    if ($continueGameOverridden) {
        if ($continueGamePriorExists) {
            Write-AtomicReplaceBytes $continueGamePath $continueGamePriorBytes
        } elseif (Test-Path -LiteralPath $continueGamePath) {
            Remove-Item -LiteralPath $continueGamePath -Force
        }
    }
}

if (-not (Test-Path -LiteralPath $captureManifest -PathType Leaf)) {
    throw "Recorder capture manifest was not produced: $captureManifest"
}
$manifestObject = Get-Content -LiteralPath $captureManifest -Raw | ConvertFrom-Json
$replayProperty = $manifestObject.PSObject.Properties['replay_file']
$replayName = if ($null -eq $replayProperty) { '' } else { [string] $replayProperty.Value }
if ([string]::IsNullOrWhiteSpace($replayName) -or
    [System.IO.Path]::GetFileName($replayName) -cne $replayName) {
    throw 'Recorder manifest replay_file is absent or is not a direct output filename.'
}
$replay = [System.IO.Path]::GetFullPath((Join-Path $resolvedOut $replayName))
if (-not (Test-Path -LiteralPath $replay -PathType Leaf)) {
    throw "Recorder replay artifact was not produced: $replay"
}

& $Python $extractor --replay $replay --out-dir $snapshotDirectory --report $extractionReport
if ($LASTEXITCODE -ne 0) {
    throw "A19 folded snapshot extraction failed; inspect $extractionReport"
}
& $Python $verifier $snapshotDirectory --output $verification
if ($LASTEXITCODE -ne 0) {
    throw "A19 lifecycle fixture was rejected; inspect $verification"
}
$actionObject=Get-Content -LiteralPath $actionReceipt -Raw|ConvertFrom-Json
$verificationObject=Get-Content -LiteralPath $verification -Raw|ConvertFrom-Json
if($actionObject.schema -cne 'hoi4.a18_a19.autonomous_action_receipt.v1' -or
   $actionObject.mode -cne 'A19DelayedEvent' -or
   $actionObject.status -cne 'pass_queued_decremented_executed_failed_and_definition_reloaded' -or
   $actionObject.executable_sha256 -cne $expectedExeSha256 -or
   [int]$actionObject.process_id -ne [int]$manifestObject.pid -or
   $actionObject.originator_tag -cne $OriginatorTag -or $actionObject.recipient_tag -cne $RecipientTag -or
   $verificationObject.status -cne 'pass' -or
   [int]$verificationObject.observed_game_hour_min -lt [int]$actionObject.initial_game_hour -or
   [int]$verificationObject.observed_game_hour_max -gt [int]$actionObject.final_game_hour) { throw 'A19 action/capture/regrade identity or temporal join rejected.' }
$receipt=[ordered]@{
    schema='hoi4.a19.autonomous_acquisition_receipt.v1';status='pass_exact_a729_all_lifecycle_classes'
    executable_sha256=$expectedExeSha256;process_id=[int]$manifestObject.pid
    originator_tag=$OriginatorTag;recipient_tag=$RecipientTag
    active_fixture_prior_sha256=$activeFixturePriorSha256;active_fixture_full_sha256=$activeFixtureFullSha256
    active_fixture_restored_sha256=(Get-FileHash -LiteralPath $resolvedActiveFixture -Algorithm SHA256).Hash.ToLowerInvariant()
    action_receipt_sha256=(Get-FileHash -LiteralPath $actionReceipt -Algorithm SHA256).Hash.ToLowerInvariant()
    capture_manifest_sha256=(Get-FileHash -LiteralPath $captureManifest -Algorithm SHA256).Hash.ToLowerInvariant()
    replay_sha256=(Get-FileHash -LiteralPath $replay -Algorithm SHA256).Hash.ToLowerInvariant()
    extraction_report_sha256=(Get-FileHash -LiteralPath $extractionReport -Algorithm SHA256).Hash.ToLowerInvariant()
    verification_sha256=(Get-FileHash -LiteralPath $verification -Algorithm SHA256).Hash.ToLowerInvariant()
    covered_reasons=@($verificationObject.covered_reasons);single_hook_epoch=$verificationObject.single_hook_epoch
    originator_recipient_separated=$verificationObject.originator_recipient_separated;immutable_receipt=$true
    observed_game_hour_min=[int]$verificationObject.observed_game_hour_min;observed_game_hour_max=[int]$verificationObject.observed_game_hour_max
    reason_counts=$verificationObject.reason_counts;reason_event_ids=$verificationObject.reason_event_ids
}
if($receipt.active_fixture_restored_sha256 -cne $activeFixturePriorSha256){throw 'A19 fixture event file was not restored byte-identically.'}
Write-AtomicReplaceBytes $acquisitionReceipt ([Text.UTF8Encoding]::new($false).GetBytes((($receipt|ConvertTo-Json -Depth 10)+"`n")))
Write-Output "A19 lifecycle fixture accepted: $verification"
