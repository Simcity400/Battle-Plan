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
    [int] $DurationSeconds = 900,
    [int] $SampleMilliseconds = 25,
    [int] $NavigationProcessTimeoutSeconds = 240,
    [int] $NavigationInitialSettleSeconds = 75,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

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
if ($DurationSeconds -lt 600) {
    throw 'DurationSeconds must be at least 600 seconds for autonomous load, exact D04 research-tree navigation, and twelve-hour capture.'
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

$verifier = Join-Path $PSScriptRoot 'verify_a29_research_ahead_fixture.py'
$navigationScript = Join-Path $PSScriptRoot 'Invoke-A29AutonomousResearchNavigation.ps1'
$unpauseScript = Join-Path $PSScriptRoot '..\..\..\replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_unpause.ps1'
if (-not (Test-Path -LiteralPath $verifier -PathType Leaf)) {
    throw "A29 verifier is missing: $verifier"
}
if (-not (Test-Path -LiteralPath $navigationScript -PathType Leaf)) {
    throw "A29 autonomous navigation script is missing: $navigationScript"
}
if (-not (Test-Path -LiteralPath $unpauseScript -PathType Leaf)) {
    throw "A29 authenticated unpause script is missing: $unpauseScript"
}
$captureManifest = Join-Path $resolvedOut 'native_session_manifest.json'
$verification = Join-Path $resolvedOut 'a29_research_ahead_fixture_verification.json'
$navigationReceipt = Join-Path $resolvedOut 'a29_autonomous_research_navigation.json'
$unpauseReceipt = Join-Path $resolvedOut 'a29_authenticated_unpause.json'
$continueGamePath = Join-Path $resolvedPdxUserDir 'continue_game.json'
$continueGamePriorExists = Test-Path -LiteralPath $continueGamePath -PathType Leaf
$continueGamePriorBytes = if ($continueGamePriorExists) {
    [System.IO.File]::ReadAllBytes($continueGamePath)
} else {
    $null
}
$continueGameOverridden = $false
$continueGameReplacement = $null

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
    '--static-bundle-mod-state', (Join-Path $resolvedStaticBundleBindingsDir 'mod_state.json'),
    '--static-bundle-dlc-state', (Join-Path $resolvedStaticBundleBindingsDir 'dlc_state.json'),
    '--static-bundle-load-order', (Join-Path $resolvedStaticBundleBindingsDir 'load_order.json'),
    '--static-bundle-source-inventory', (Join-Path $resolvedStaticBundleBindingsDir 'source_inventory.json'),
    '--static-bundle-live-resolution-report', (Join-Path $resolvedStaticBundleBindingsDir 'resolution_report.json'),
    '--static-bundle-timeout-ms', '1800000',
    '--sidecar-log',
    '--hour-latch'
)
foreach ($value in $EarlyLaunchArgument) {
    $arguments += @('--early-launch-arg', $value)
}

Write-Host 'A29 autonomous acquisition: clean early launch, exact D04 physical Research/active-row navigation, future-node hover, authenticated unpause, and fail-closed branch-1/2 verification.'

$recorderJob = $null
try {
    if ($null -ne $continueGameReplacement) {
        # Mark the path overridden first so any partial write still restores
        # the exact prior bytes in the finally block.
        $continueGameOverridden = $true
        [System.IO.File]::WriteAllText(
            $continueGamePath,
            $continueGameReplacement,
            [System.Text.UTF8Encoding]::new($false)
        )
    }
    $recorderJob = Start-Job -ScriptBlock {
        param([string]$Recorder, [string[]]$RecorderArguments)
        & $Recorder @RecorderArguments
        if ($LASTEXITCODE -ne 0) {
            throw "collector_record exited with code $LASTEXITCODE"
        }
    } -ArgumentList $resolvedRecorder, (,$arguments)

    $deadline = [DateTime]::UtcNow.AddSeconds($NavigationProcessTimeoutSeconds)
    $exactProcess = $null
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

    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $navigationScript `
        -ProcessId $exactProcess.Id `
        -ReceiptPath $navigationReceipt `
        -InitialSettleSeconds $NavigationInitialSettleSeconds
    if ($LASTEXITCODE -ne 0) {
        throw "A29 autonomous research navigation failed with code $LASTEXITCODE"
    }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $unpauseScript `
        -ProcessId $exactProcess.Id `
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
} finally {
    if ($null -ne $recorderJob) {
        if ($recorderJob.State -notin @('Completed', 'Failed', 'Stopped', 'Disconnected')) {
            Wait-Job -Job $recorderJob | Out-Null
        }
        Receive-Job -Job $recorderJob -ErrorAction SilentlyContinue | Out-Null
        Remove-Job -Job $recorderJob -Force -ErrorAction SilentlyContinue
    }
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

& $Python $verifier `
    --capture-manifest $captureManifest `
    --navigation-receipt $navigationReceipt `
    --unpause-receipt $unpauseReceipt `
    --replay $replay `
    --out $verification
if ($LASTEXITCODE -ne 0) {
    throw "A29 observed-producer fixture was rejected; inspect $verification"
}

Write-Output "A29 observed-producer fixture accepted: $verification"
