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
    [int] $ProcessTimeoutSeconds = 240,
    [int] $InitialSettleSeconds = 75,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$expectedD04SaveSha256 = 'c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735'
foreach ($path in @($RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll, $ContinueSave, $StaticBundleManifest, $StaticBundleBindingsDir)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required path does not exist: $path" }
}
if ($DurationSeconds -lt 600 -or $DurationSeconds -gt 3600) {
    throw 'DurationSeconds must be in 600..3600 for load, UI settlement, and twelve advancing hours.'
}
if ($SampleMilliseconds -lt 10 -or $SampleMilliseconds -gt 1000) {
    throw 'SampleMilliseconds must be in 10..1000.'
}
if ($ProcessTimeoutSeconds -lt 30 -or $ProcessTimeoutSeconds -gt 600) {
    throw 'ProcessTimeoutSeconds must be in 30..600.'
}
if ($InitialSettleSeconds -lt 0 -or $InitialSettleSeconds -gt 300) {
    throw 'InitialSettleSeconds must be in 0..300.'
}

$resolvedRecorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedPdxUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedSave = (Resolve-Path -LiteralPath $ContinueSave).Path
$resolvedStaticBundleManifest = (Resolve-Path -LiteralPath $StaticBundleManifest).Path
$resolvedStaticBundleBindingsDir = (Resolve-Path -LiteralPath $StaticBundleBindingsDir).Path
$resolvedStaticBundleDir = Split-Path -Parent $resolvedStaticBundleManifest
$resolvedPython = (Get-Command $Python -CommandType Application -ErrorAction Stop).Source
$resolvedOut = [IO.Path]::GetFullPath($OutDir)
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\..')).Path
$productionDriver = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1')).Path
$attestationHelpers = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1')).Path
$staticBundleVerifier = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\game_defs\static_bundle.py')).Path
$clockTransitionScript = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Invoke-A29AuthenticatedClockTransition.ps1')).Path
$verifier = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'verify_a15_counter_ui_fixture.py')).Path
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
    throw 'A15 acquisition requires the recorder and collector from the authenticated central Release attestation.'
}
if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) {
    throw 'Hoi4Exe is not exact a729 authority.'
}
if ((Get-FileHash -LiteralPath $resolvedSave -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedD04SaveSha256) {
    throw 'A15 acquisition requires the frozen New D04 Save Deep War identity.'
}
$saveDirectory = [IO.Path]::GetFullPath((Join-Path $resolvedPdxUserDir 'save games')).TrimEnd([char]'\')
$requestedDirectory = [IO.Path]::GetFullPath((Split-Path -Parent $resolvedSave)).TrimEnd([char]'\')
if (-not [string]::Equals($saveDirectory, $requestedDirectory, [StringComparison]::OrdinalIgnoreCase)) {
    throw "ContinueSave must be a direct child of $saveDirectory"
}
$continueIndex = [Array]::IndexOf($EarlyLaunchArgument, '--continuelastsave')
$gdprIndex = [Array]::IndexOf($EarlyLaunchArgument, '--gdpr-compliant')
if ($continueIndex -lt 0 -or $gdprIndex -lt $continueIndex) {
    throw 'EarlyLaunchArgument must contain --continuelastsave followed by --gdpr-compliant.'
}
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
    throw 'A15 requires a clean early launch; close every existing hoi4.exe first.'
}
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty: $resolvedOut"
    }
} else {
    [IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
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

$bindingPaths = [ordered]@{
    mod_state = Join-Path $resolvedStaticBundleBindingsDir 'mod_state.json'
    dlc_state = Join-Path $resolvedStaticBundleBindingsDir 'dlc_state.json'
    load_order = Join-Path $resolvedStaticBundleBindingsDir 'load_order.json'
    source_inventory = Join-Path $resolvedStaticBundleBindingsDir 'source_inventory.json'
    resolution_report = Join-Path $resolvedStaticBundleBindingsDir 'resolution_report.json'
}
foreach ($key in @($bindingPaths.Keys)) {
    if (-not (Test-Path -LiteralPath $bindingPaths[$key] -PathType Leaf)) {
        throw "A15 static-bundle binding $key is missing: $($bindingPaths[$key])"
    }
    $bindingPaths[$key] = (Resolve-Path -LiteralPath $bindingPaths[$key]).Path
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
if ($LASTEXITCODE -ne 0) { throw 'A15 static-bundle pre-injection verification failed.' }
$staticPreflightText = $staticPreflightOutput -join [Environment]::NewLine
$staticPreflight = $staticPreflightText | ConvertFrom-Json
if ($staticPreflight.passed -ne $true -or $staticPreflight.verification_phase -cne 'pre_injection_deferred_allowed') {
    throw 'A15 static-bundle preflight did not admit the deferred pre-injection phase.'
}
$staticPreflightReceipt = Join-Path $resolvedOut 'a15_static_bundle_preflight.json'
Write-AtomicUtf8Text $staticPreflightReceipt ($staticPreflightText + [Environment]::NewLine)

$continueGamePath = Join-Path $resolvedPdxUserDir 'continue_game.json'
$continuePriorExists = Test-Path -LiteralPath $continueGamePath -PathType Leaf
$continuePriorBytes = if ($continuePriorExists) { [IO.File]::ReadAllBytes($continueGamePath) } else { $null }
$continueObject = if ($continuePriorExists) {
    Get-Content -LiteralPath $continueGamePath -Raw | ConvertFrom-Json
} else {
    [pscustomobject]@{
        title = [IO.Path]::GetFileNameWithoutExtension($resolvedSave)
        desc = ''
        date = [DateTime]::UtcNow.ToString('R')
        filename = ''
        is_remote = $false
    }
}
$continueObject.filename = [IO.Path]::GetFileName($resolvedSave)
$continueObject.is_remote = $false
$continueReplacement = ($continueObject | ConvertTo-Json -Depth 8) + [Environment]::NewLine

$sessionId = [guid]::NewGuid().ToString()
$captureId = [guid]::NewGuid().ToString()
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
    '--scheduled-domain', 'divisions',
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
foreach ($value in $EarlyLaunchArgument) { $arguments += @('--early-launch-arg', $value) }

$captureManifest = Join-Path $resolvedOut 'native_session_manifest.json'
$readinessReceipt = Join-Path $resolvedOut 'a15_authenticated_paused_save_readiness.json'
$unpauseReceipt = Join-Path $resolvedOut 'a15_authenticated_unpause.json'
$repauseReceipt = Join-Path $resolvedOut 'a15_authenticated_repause.json'
$verification = Join-Path $resolvedOut 'a15_counter_ui_fixture_verification.json'
$recorderJob = $null
$exactProcess = $null
$exactProcessStartUtc = $null
$repauseError = $null
$captureCompleted = $false
$continueOverridden = $false
try {
    $continueOverridden = $true
    Write-AtomicUtf8Text $continueGamePath $continueReplacement -AllowReplace
    $recorderJob = Start-Job -ScriptBlock {
        param([string]$Recorder, [string[]]$RecorderArguments)
        & $Recorder @RecorderArguments
        if ($LASTEXITCODE -ne 0) { throw "collector_record exited with code $LASTEXITCODE" }
    } -ArgumentList $resolvedRecorder, (,$arguments)
    $deadline = [DateTime]::UtcNow.AddSeconds($ProcessTimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline -and $null -eq $exactProcess) {
        if ($recorderJob.State -in @('Failed', 'Stopped', 'Disconnected')) {
            throw "collector_record ended before exact-a729 exposed a window: $($recorderJob.State)"
        }
        foreach ($candidate in @(Get-Process -Name the game -ErrorAction SilentlyContinue)) {
            try {
                if ([string]::Equals($candidate.MainModule.FileName, $resolvedExe, [StringComparison]::OrdinalIgnoreCase) -and
                    $candidate.MainWindowHandle -ne [IntPtr]::Zero -and
                    (Get-FileHash -LiteralPath $candidate.MainModule.FileName -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $expectedExeSha256) {
                    $exactProcess = $candidate
                    break
                }
            } catch { }
        }
        if ($null -eq $exactProcess) { Start-Sleep -Milliseconds 500 }
    }
    if ($null -eq $exactProcess) { throw "Exact-a729 did not expose a window within $ProcessTimeoutSeconds seconds." }
    $exactProcessStartUtc = $exactProcess.StartTime.ToUniversalTime().ToString('o')
    Start-Sleep -Seconds $InitialSettleSeconds
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $clockTransitionScript `
        -Action Readiness -ProcessId $exactProcess.Id -ExpectedProcessStartUtc $exactProcessStartUtc `
        -SessionId $sessionId -CaptureId $captureId -ReceiptPath $readinessReceipt `
        -ReadinessTimeoutSeconds $ProcessTimeoutSeconds -ExpectedReadyGameHourRaw 60830441
    if ($LASTEXITCODE -ne 0) { throw "A15 authenticated paused-save readiness failed with code $LASTEXITCODE" }
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $clockTransitionScript `
        -Action Unpause -ProcessId $exactProcess.Id -ExpectedProcessStartUtc $exactProcessStartUtc `
        -SessionId $sessionId -CaptureId $captureId -ReceiptPath $unpauseReceipt
    if ($LASTEXITCODE -ne 0) { throw "A15 authenticated unpause failed with code $LASTEXITCODE" }
    Wait-Job -Job $recorderJob | Out-Null
    $state = $recorderJob.State
    Receive-Job -Job $recorderJob -ErrorAction Continue | ForEach-Object { Write-Verbose ([string]$_) }
    if ($state -ne 'Completed') { throw "collector_record background job ended in state $state" }
    $captureCompleted = $true
} finally {
    if ($null -ne $exactProcess -and -not [string]::IsNullOrWhiteSpace($exactProcessStartUtc)) {
        try {
            if ($null -ne (Get-Process -Id $exactProcess.Id -ErrorAction SilentlyContinue)) {
                & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $clockTransitionScript `
                    -Action Repause -ProcessId $exactProcess.Id -ExpectedProcessStartUtc $exactProcessStartUtc `
                    -SessionId $sessionId -CaptureId $captureId -ReceiptPath $repauseReceipt
                if ($LASTEXITCODE -ne 0) { throw "A15 authenticated repause failed with code $LASTEXITCODE" }
            }
        } catch { $repauseError = $_.Exception.Message }
    }
    if ($null -ne $recorderJob) {
        if ($recorderJob.State -notin @('Completed', 'Failed', 'Stopped', 'Disconnected')) {
            if (-not $captureCompleted) { Stop-Job -Job $recorderJob -ErrorAction SilentlyContinue }
            Wait-Job -Job $recorderJob | Out-Null
        }
        Receive-Job -Job $recorderJob -ErrorAction SilentlyContinue | Out-Null
        Remove-Job -Job $recorderJob -Force -ErrorAction SilentlyContinue
    }
    if ($continueOverridden) {
        if ($continuePriorExists) {
            Write-AtomicBytes $continueGamePath $continuePriorBytes -AllowReplace
        } elseif (Test-Path -LiteralPath $continueGamePath) {
            Remove-Item -LiteralPath $continueGamePath -Force
        }
    }
    if ($null -ne $repauseError) { throw "A15 unconditional repause failed: $repauseError" }
}

if (-not (Test-Path -LiteralPath $captureManifest -PathType Leaf)) {
    throw "Recorder capture manifest was not produced: $captureManifest"
}
$manifest = Get-Content -LiteralPath $captureManifest -Raw | ConvertFrom-Json
$replayName = [string]$manifest.replay_file
if ([string]::IsNullOrWhiteSpace($replayName) -or [IO.Path]::GetFileName($replayName) -cne $replayName) {
    throw 'Recorder manifest replay_file is absent or not a direct output filename.'
}
$replay = [IO.Path]::GetFullPath((Join-Path $resolvedOut $replayName))
if (-not (Test-Path -LiteralPath $replay -PathType Leaf)) { throw "Replay was not produced: $replay" }
if (-not (Test-Path -LiteralPath $repauseReceipt -PathType Leaf)) { throw 'Repause receipt was not produced.' }

& $resolvedPython $verifier `
    --capture-manifest $captureManifest `
    --prebuilt-attestation $resolvedAttestation `
    --static-bundle-preflight $staticPreflightReceipt `
    --readiness-receipt $readinessReceipt `
    --unpause-receipt $unpauseReceipt `
    --repause-receipt $repauseReceipt `
    --replay $replay `
    --out $verification
if ($LASTEXITCODE -ne 0) { throw "A15 fixture was rejected; inspect $verification" }
Write-Output "A15 current-hour D20 counter UI fixture accepted: $verification"
