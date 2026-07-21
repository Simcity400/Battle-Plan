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
    [Parameter(ParameterSetName = 'Capture')]
    [string] $ContinueSave,
    [Parameter(ParameterSetName = 'Capture')]
    [int] $DurationSeconds = 1800,
    [Parameter(ParameterSetName = 'Capture')]
    [int] $SampleMilliseconds = 25,
    [Parameter(ParameterSetName = 'Capture')]
    [string[]] $EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string] $OutputJson = '',
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$verifier = Join-Path $repo "extractor\native\lanes\scripted_state_flags_variables_live\verify_a19_delayed_event_lifecycle.py"
if (-not (Test-Path -LiteralPath $verifier -PathType Leaf)) {
    throw "A19 verifier is missing: $verifier"
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

Write-Host @'
A19 controlled-fixture requirements:
  * Queue a delayed event for a recipient different from its originator.
  * Retain one event across an hourly decrement.
  * Consume one due event through a true trigger.
  * Consume one due event through a false trigger.
  * Cancel one queued event by removing its event definition.
The fixture may automate these actions. No operator annotations enter the
carrier; the verifier accepts only the exact hook-owned DEL2 observations.
'@

try {
    if ($null -ne $continueGameReplacement) {
        $continueGameOverridden = $true
        [System.IO.File]::WriteAllText(
            $continueGamePath,
            $continueGameReplacement,
            [System.Text.UTF8Encoding]::new($false)
        )
    }
    $priorEvidenceGate = $(if (Test-Path Env:BATTLE_PLAN_ENABLE_A19_DELAYED_EVENT_EVIDENCE) { $env:BATTLE_PLAN_ENABLE_A19_DELAYED_EVENT_EVIDENCE } else { $env:HOI4_ENABLE_A19_DELAYED_EVENT_EVIDENCE })
    $env:BATTLE_PLAN_ENABLE_A19_DELAYED_EVENT_EVIDENCE = '1'
    try {
        & $resolvedRecorder @arguments
        if ($LASTEXITCODE -ne 0) {
            throw "collector_record exited with code $LASTEXITCODE"
        }
    } finally {
        if ($null -eq $priorEvidenceGate) {
            Remove-Item Env:\BATTLE_PLAN_ENABLE_A19_DELAYED_EVENT_EVIDENCE -ErrorAction SilentlyContinue
        } else {
            $env:BATTLE_PLAN_ENABLE_A19_DELAYED_EVENT_EVIDENCE = $priorEvidenceGate
        }
    }
} finally {
    if ($continueGameOverridden) {
        if ($continueGamePriorExists) {
            [System.IO.File]::WriteAllBytes($continueGamePath, $continueGamePriorBytes)
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
Write-Output "A19 lifecycle fixture accepted: $verification"
