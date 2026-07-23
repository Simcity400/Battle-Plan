[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $RecorderExe,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $PdxUserDir,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [Parameter(Mandatory = $true)] [string] $ContinueSave,
    [Parameter(Mandatory = $true)] [string] $StaticBundleManifest,
    [Parameter(Mandatory = $true)] [string] $StaticBundleBindingsDir,
    [string] $PrebuiltArtifactAttestationPath = '',
    [int] $DurationSeconds = 900,
    [int] $SampleMilliseconds = 25,
    [int] $NavigationProcessTimeoutSeconds = 240,
    [int] $NavigationInitialSettleSeconds = 75,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$expectedD04SaveSha256 = 'c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735'
foreach ($path in @($RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll, $StaticBundleManifest, $StaticBundleBindingsDir)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required path does not exist: $path"
    }
}
if ($ContinueSave -and -not (Test-Path -LiteralPath $ContinueSave -PathType Leaf)) {
    throw "Continue-save does not exist: $ContinueSave"
}
$observedSaveSha256 = (Get-FileHash -LiteralPath $ContinueSave -Algorithm SHA256).Hash.ToLowerInvariant()
if ($observedSaveSha256 -cne $expectedD04SaveSha256) {
    throw "A29 autonomous navigation requires the frozen New D04 Save Deep War identity: $observedSaveSha256"
}
if ($DurationSeconds -lt 600 -or $DurationSeconds -gt 3600) {
    throw 'DurationSeconds must be in 600..3600 for autonomous load, exact D04 research-tree navigation, and twelve-hour capture.'
}
if ($SampleMilliseconds -lt 10) {
    throw 'SampleMilliseconds must be at least 10 milliseconds.'
}
if ($NavigationProcessTimeoutSeconds -lt 30 -or $NavigationProcessTimeoutSeconds -gt 600) {
    throw 'NavigationProcessTimeoutSeconds must be in 30..600.'
}
if ($NavigationInitialSettleSeconds -lt 0 -or $NavigationInitialSettleSeconds -gt 300) {
    throw 'NavigationInitialSettleSeconds must be in 0..300.'
}

$resolvedRecorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedPdxUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedStaticBundleManifest = (Resolve-Path -LiteralPath $StaticBundleManifest).Path
$resolvedStaticBundleBindingsDir = (Resolve-Path -LiteralPath $StaticBundleBindingsDir).Path
$resolvedStaticBundleDir = Split-Path -Parent $resolvedStaticBundleManifest
$resolvedPython = (Get-Command $Python -CommandType Application -ErrorAction Stop).Source
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\..')).Path
$productionDriver = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1')).Path
$attestationHelpers = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1')).Path
$releaseRoot = Join-Path $repoRoot '.cargo-target\release'
if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) {
    $PrebuiltArtifactAttestationPath = Join-Path $releaseRoot 'hoi4-production-prebuilt-artifacts.json'
}
. $attestationHelpers
$prebuilt = Test-ProductionArtifactAttestation `
    -RepoRoot $repoRoot `
    -ReceiptPath $PrebuiltArtifactAttestationPath `
    -ExpectedDriverPath $productionDriver
$resolvedAttestation = (Resolve-Path -LiteralPath $prebuilt.receipt_path).Path
if (-not [string]::Equals($resolvedRecorder, [string]$prebuilt.artifacts.recorder_host, [StringComparison]::OrdinalIgnoreCase) -or
    -not [string]::Equals($resolvedCollector, [string]$prebuilt.artifacts.collector_dll, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'A29 production acquisition requires the recorder and collector from the authenticated central Release attestation.'
}
$staticBundleVerifier = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\game_defs\static_bundle.py')).Path
$resolvedOut = [System.IO.Path]::GetFullPath($OutDir)
$sessionId = [guid]::NewGuid().ToString()
$captureId = [guid]::NewGuid().ToString()
$observedExeSha256 = (Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($observedExeSha256 -cne $expectedExeSha256) {
    throw "Hoi4Exe is not exact a729 authority: $observedExeSha256"
}
$existing = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
if ($existing.Count -ne 0) {
    throw "A29 requires a clean early launch; close the game PID(s) $(@($existing.Id) -join ',') first."
}
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or
        @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty so stale A29 evidence cannot be selected: $resolvedOut"
    }
} else {
    [System.IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
}

function Write-AtomicBytes([string]$Path, [byte[]]$Bytes, [switch]$AllowReplace) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($fullPath)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporaryPath = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllBytes($temporaryPath, $Bytes)
        $stream = [IO.File]::Open($temporaryPath, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($fullPath)) {
            if (-not $AllowReplace) { throw "Refusing to replace existing file: $fullPath" }
            [IO.File]::Replace($temporaryPath, $fullPath, $null, $true)
        } else {
            [IO.File]::Move($temporaryPath, $fullPath)
        }
    } finally {
        if ([IO.File]::Exists($temporaryPath)) { [IO.File]::Delete($temporaryPath) }
    }
}

function Write-AtomicUtf8Text([string]$Path, [string]$Text, [switch]$AllowReplace) {
    Write-AtomicBytes $Path ([Text.UTF8Encoding]::new($false).GetBytes($Text)) -AllowReplace:$AllowReplace
}

$verifier = Join-Path $PSScriptRoot 'verify_a29_research_ahead_fixture.py'
$navigationScript = Join-Path $PSScriptRoot 'Invoke-A29AutonomousResearchNavigation.ps1'
$clockTransitionScript = Join-Path $PSScriptRoot 'Invoke-A29AuthenticatedClockTransition.ps1'
if (-not (Test-Path -LiteralPath $verifier -PathType Leaf)) {
    throw "A29 verifier is missing: $verifier"
}
if (-not (Test-Path -LiteralPath $navigationScript -PathType Leaf)) {
    throw "A29 autonomous navigation script is missing: $navigationScript"
}
if (-not (Test-Path -LiteralPath $clockTransitionScript -PathType Leaf)) {
    throw "A29 authenticated clock-transition script is missing: $clockTransitionScript"
}
$captureManifest = Join-Path $resolvedOut 'native_session_manifest.json'
$verification = Join-Path $resolvedOut 'a29_research_ahead_fixture_verification.json'
$navigationReceipt = Join-Path $resolvedOut 'a29_autonomous_research_navigation.json'
$readinessReceipt = Join-Path $resolvedOut 'a29_authenticated_paused_save_readiness.json'
$unpauseReceipt = Join-Path $resolvedOut 'a29_authenticated_unpause.json'
$repauseReceipt = Join-Path $resolvedOut 'a29_authenticated_repause.json'
$staticBundlePreflightReceipt = Join-Path $resolvedOut 'a29_static_bundle_preflight.json'
$continueGamePath = Join-Path $resolvedPdxUserDir 'continue_game.json'
$continueGamePriorExists = Test-Path -LiteralPath $continueGamePath -PathType Leaf
$continueGamePriorBytes = if ($continueGamePriorExists) {
    [System.IO.File]::ReadAllBytes($continueGamePath)
} else {
    $null
}
$continueGameOverridden = $false
$continueGameReplacement = $null

$bindingPaths = [ordered]@{
    mod_state = Join-Path $resolvedStaticBundleBindingsDir 'mod_state.json'
    dlc_state = Join-Path $resolvedStaticBundleBindingsDir 'dlc_state.json'
    load_order = Join-Path $resolvedStaticBundleBindingsDir 'load_order.json'
    source_inventory = Join-Path $resolvedStaticBundleBindingsDir 'source_inventory.json'
    resolution_report = Join-Path $resolvedStaticBundleBindingsDir 'resolution_report.json'
}
foreach ($key in @($bindingPaths.Keys)) {
    $candidatePath = $bindingPaths[$key]
    if (-not (Test-Path -LiteralPath $candidatePath -PathType Leaf)) {
        throw "A29 static-bundle binding $key is missing: $candidatePath"
    }
    $bindingPaths[$key] = (Resolve-Path -LiteralPath $candidatePath).Path
}

$staticPreflightOutput = @(& $resolvedPython $staticBundleVerifier verify `
    --bundle $resolvedStaticBundleDir `
    --exe $resolvedExe `
    --mod-state $bindingPaths['mod_state'] `
    --dlc-state $bindingPaths['dlc_state'] `
    --load-order $bindingPaths['load_order'] `
    --source-inventory $bindingPaths['source_inventory'] `
    --live-resolution-report $bindingPaths['resolution_report'] `
    --allow-deferred-token-gate)
if ($LASTEXITCODE -ne 0) { throw 'A29 static-bundle pre-injection verification failed.' }
$staticPreflightText = ($staticPreflightOutput -join [Environment]::NewLine)
try { $staticPreflight = $staticPreflightText | ConvertFrom-Json } catch {
    throw "A29 static-bundle preflight did not emit JSON: $($_.Exception.Message)"
}
if ($staticPreflight.passed -ne $true -or $staticPreflight.verification_phase -cne 'pre_injection_deferred_allowed') {
    throw 'A29 static-bundle preflight did not admit the deferred pre-injection phase.'
}
Write-AtomicUtf8Text $staticBundlePreflightReceipt ($staticPreflightText + [Environment]::NewLine)

if ($ContinueSave) {
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
    if ($continueIndex -lt 0) {
        throw '-ContinueSave requires --continuelastsave in -EarlyLaunchArgument.'
    }
    if ($gdprIndex -lt 0 -or $gdprIndex -lt $continueIndex) {
        throw '-ContinueSave requires launcher-effective --gdpr-compliant after --continuelastsave.'
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
    '--session-id', $sessionId,
    '--capture-id', $captureId,
    '--duration-s', $DurationSeconds,
    '--sample-ms', $SampleMilliseconds,
    '--min-game-hours', '12',
    '--stream-format', 'compact',
    '--profile', 'debug',
    '--scheduled-domain', 'tech_live',
    '--scheduled-domains-exact',
    '--static-bundle-manifest', $resolvedStaticBundleManifest,
    '--static-bundle-mod-state', $bindingPaths['mod_state'],
    '--static-bundle-dlc-state', $bindingPaths['dlc_state'],
    '--static-bundle-load-order', $bindingPaths['load_order'],
    '--static-bundle-source-inventory', $bindingPaths['source_inventory'],
    '--static-bundle-live-resolution-report', $bindingPaths['resolution_report'],
    '--static-bundle-verifier', $staticBundleVerifier,
    '--static-bundle-python', $resolvedPython,
    '--static-bundle-timeout-ms', '300000',
    '--sidecar-log',
    '--hour-latch'
)
foreach ($value in $EarlyLaunchArgument) {
    $arguments += @('--early-launch-arg', $value)
}

Write-Host 'A29 autonomous acquisition: clean early launch, exact D04 physical Research/active-row navigation, future-node hover, authenticated unpause, and fail-closed branch-1/2 verification.'

$recorderJob = $null
$exactProcess = $null
$exactProcessStartUtc = $null
$repauseError = $null
$captureRunCompleted = $false
try {
    if ($null -ne $continueGameReplacement) {
        # Mark the path overridden first so any partial write still restores
        # the exact prior bytes in the finally block.
        $continueGameOverridden = $true
        Write-AtomicUtf8Text $continueGamePath $continueGameReplacement -AllowReplace
    }
    $recorderJob = Start-Job -ScriptBlock {
        param([string]$Recorder, [string[]]$RecorderArguments)
        & $Recorder @RecorderArguments
        if ($LASTEXITCODE -ne 0) {
            throw "collector_record exited with code $LASTEXITCODE"
        }
    } -ArgumentList $resolvedRecorder, (,$arguments)

    $deadline = [DateTime]::UtcNow.AddSeconds($NavigationProcessTimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline -and $null -eq $exactProcess) {
        if ($recorderJob.State -in @('Failed', 'Stopped', 'Disconnected')) {
            throw "collector_record background job terminated before exact-a729 exposed a window: $($recorderJob.State)"
        }
        foreach ($candidate in @(Get-Process -Name the game -ErrorAction SilentlyContinue)) {
            try {
                $candidatePath = $candidate.MainModule.FileName
                if (-not [string]::Equals($candidatePath, $resolvedExe, [StringComparison]::OrdinalIgnoreCase)) {
                    continue
                }
                $candidateSha256 = (Get-FileHash -LiteralPath $candidatePath -Algorithm SHA256).Hash.ToLowerInvariant()
                if ($candidateSha256 -cne $expectedExeSha256 -or $candidate.MainWindowHandle -eq [IntPtr]::Zero) {
                    continue
                }
                $exactProcess = $candidate
                break
            } catch {
                # A process still entering startup can transiently hide its
                # module/window. It is never selected without the exact checks.
            }
        }
        if ($null -eq $exactProcess) {
            Start-Sleep -Milliseconds 500
        }
    }
    if ($null -eq $exactProcess) {
        throw "Exact-a729 did not expose an authenticated main window within $NavigationProcessTimeoutSeconds seconds."
    }
    $exactProcessStartUtc = $exactProcess.StartTime.ToUniversalTime().ToString('o')

    if ($NavigationInitialSettleSeconds -gt 0) {
        Start-Sleep -Seconds $NavigationInitialSettleSeconds
    }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $clockTransitionScript `
        -Action Readiness `
        -ProcessId $exactProcess.Id `
        -ExpectedProcessStartUtc $exactProcessStartUtc `
        -SessionId $sessionId `
        -CaptureId $captureId `
        -ReceiptPath $readinessReceipt `
        -ReadinessTimeoutSeconds $NavigationProcessTimeoutSeconds `
        -ExpectedReadyGameHourRaw 60830441
    if ($LASTEXITCODE -ne 0) {
        throw "A29 authenticated paused-save readiness failed with code $LASTEXITCODE"
    }

    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $navigationScript `
        -ProcessId $exactProcess.Id `
        -ReceiptPath $navigationReceipt `
        -SessionId $sessionId `
        -CaptureId $captureId `
        -InitialSettleSeconds 0
    if ($LASTEXITCODE -ne 0) {
        throw "A29 autonomous research navigation failed with code $LASTEXITCODE"
    }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $clockTransitionScript `
        -Action Unpause `
        -ProcessId $exactProcess.Id `
        -ExpectedProcessStartUtc $exactProcessStartUtc `
        -SessionId $sessionId `
        -CaptureId $captureId `
        -ReceiptPath $unpauseReceipt
    if ($LASTEXITCODE -ne 0) {
        throw "A29 authenticated unpause failed with code $LASTEXITCODE"
    }

    Wait-Job -Job $recorderJob | Out-Null
    $recorderState = $recorderJob.State
    $recorderOutput = @(Receive-Job -Job $recorderJob -ErrorAction Continue)
    foreach ($line in $recorderOutput) {
        Write-Verbose ([string]$line)
    }
    if ($recorderState -ne 'Completed') {
        throw "collector_record background job ended in state $recorderState"
    }
    $captureRunCompleted = $true
} finally {
    if ($null -ne $exactProcess -and -not [string]::IsNullOrWhiteSpace($exactProcessStartUtc)) {
        try {
            $stillRunning = Get-Process -Id $exactProcess.Id -ErrorAction SilentlyContinue
            if ($null -ne $stillRunning) {
                & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $clockTransitionScript `
                    -Action Repause `
                    -ProcessId $exactProcess.Id `
                    -ExpectedProcessStartUtc $exactProcessStartUtc `
                    -SessionId $sessionId `
                    -CaptureId $captureId `
                    -ReceiptPath $repauseReceipt
                if ($LASTEXITCODE -ne 0) { throw "A29 authenticated repause failed with code $LASTEXITCODE" }
            }
        } catch {
            $repauseError = $_.Exception.Message
        }
    }
    if ($null -ne $recorderJob) {
        if ($recorderJob.State -notin @('Completed', 'Failed', 'Stopped', 'Disconnected')) {
            if (-not $captureRunCompleted) {
                Stop-Job -Job $recorderJob -ErrorAction SilentlyContinue
            }
            Wait-Job -Job $recorderJob | Out-Null
        }
        Receive-Job -Job $recorderJob -ErrorAction SilentlyContinue | Out-Null
        Remove-Job -Job $recorderJob -Force -ErrorAction SilentlyContinue
    }
    if ($continueGameOverridden) {
        if ($continueGamePriorExists) {
            Write-AtomicBytes $continueGamePath $continueGamePriorBytes -AllowReplace
        } elseif (Test-Path -LiteralPath $continueGamePath) {
            Remove-Item -LiteralPath $continueGamePath -Force
        }
    }
    if ($null -ne $repauseError) { throw "A29 unconditional repause failed: $repauseError" }
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

if (-not (Test-Path -LiteralPath $repauseReceipt -PathType Leaf)) {
    throw "A29 authenticated repause receipt was not produced: $repauseReceipt"
}

& $resolvedPython $verifier `
    --capture-manifest $captureManifest `
    --prebuilt-attestation $resolvedAttestation `
    --static-bundle-preflight $staticBundlePreflightReceipt `
    --navigation-receipt $navigationReceipt `
    --readiness-receipt $readinessReceipt `
    --unpause-receipt $unpauseReceipt `
    --repause-receipt $repauseReceipt `
    --replay $replay `
    --out $verification
if ($LASTEXITCODE -ne 0) {
    throw "A29 observed-producer fixture was rejected; inspect $verification"
}

Write-Output "A29 observed-producer fixture accepted: $verification"
