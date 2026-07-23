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
    [Parameter(Mandatory = $true)] [string] $ActionProfile,
    [string] $PrebuiltArtifactAttestationPath = '',
    [int] $DurationSeconds = 1800,
    [int] $SampleMilliseconds = 25,
    [int] $ProcessTimeoutSeconds = 300,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
foreach ($path in @($RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll, $ContinueSave, $StaticBundleManifest, $StaticBundleBindingsDir, $ActionProfile)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required X10 path does not exist: $path" }
}
if ($DurationSeconds -lt 300 -or $DurationSeconds -gt 7200) { throw 'X10 DurationSeconds must be in 300..7200.' }
if ($SampleMilliseconds -lt 10 -or $SampleMilliseconds -gt 1000) { throw 'X10 SampleMilliseconds must be in 10..1000.' }
if ($ProcessTimeoutSeconds -lt 30 -or $ProcessTimeoutSeconds -gt 1800) { throw 'X10 ProcessTimeoutSeconds must be in 30..1800.' }
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) { throw 'X10 requires a clean suspended early launch; close every existing hoi4.exe first.' }

$resolvedRecorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedSave = (Resolve-Path -LiteralPath $ContinueSave).Path
$resolvedManifest = (Resolve-Path -LiteralPath $StaticBundleManifest).Path
$resolvedBindings = (Resolve-Path -LiteralPath $StaticBundleBindingsDir).Path
$resolvedProfile = (Resolve-Path -LiteralPath $ActionProfile).Path
$resolvedPython = (Get-Command $Python -CommandType Application -ErrorAction Stop).Source
$resolvedOut = [IO.Path]::GetFullPath($OutDir)
if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) { throw 'X10 Hoi4Exe is not exact a729 authority.' }
$profile = Get-Content -LiteralPath $resolvedProfile -Raw | ConvertFrom-Json
if ($profile.schema -cne 'hoi4.x10.autonomous-owner-fixture-actions.v1' -or [string]$profile.executable_sha256 -cne $expectedExeSha256) {
    throw 'X10 action profile schema/executable authority is invalid.'
}
$saveSha256 = (Get-FileHash -LiteralPath $resolvedSave -Algorithm SHA256).Hash.ToLowerInvariant()
if ($saveSha256 -cne [string]$profile.save_sha256) { throw 'X10 ContinueSave does not match the frozen action profile.' }
$saveDirectory = [IO.Path]::GetFullPath((Join-Path $resolvedUserDir 'save games')).TrimEnd([char]'\')
$requestedDirectory = [IO.Path]::GetFullPath((Split-Path -Parent $resolvedSave)).TrimEnd([char]'\')
if (-not $saveDirectory.Equals($requestedDirectory, [StringComparison]::OrdinalIgnoreCase)) { throw "X10 ContinueSave must be a direct child of $saveDirectory" }
$continueIndex = [Array]::IndexOf($EarlyLaunchArgument, '--continuelastsave')
$gdprIndex = [Array]::IndexOf($EarlyLaunchArgument, '--gdpr-compliant')
if ($continueIndex -lt 0 -or $gdprIndex -lt $continueIndex) { throw 'X10 EarlyLaunchArgument must contain --continuelastsave followed by --gdpr-compliant.' }
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "X10 OutDir must be absent or empty: $resolvedOut"
    }
} else { [IO.Directory]::CreateDirectory($resolvedOut) | Out-Null }

$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\..')).Path
$productionDriver = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1')).Path
$attestationHelpers = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1')).Path
$staticVerifier = (Resolve-Path -LiteralPath (Join-Path $repo 'extractor\game_defs\static_bundle.py')).Path
$actionDriver = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Invoke-X10AuthenticatedFixtureActions.ps1')).Path
$verifier = (Resolve-Path -LiteralPath (Join-Path $repo 'extractor\native\lanes\direct_frd_transitions_live\tools\verify_x10_owner_fixture.py')).Path
$contract = (Resolve-Path -LiteralPath (Join-Path $repo 'extractor\native\lanes\direct_frd_transitions_live\X10_DIRECT_TRANSITION_OWNER_FIXTURE_20260720.json')).Path
if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) {
    $PrebuiltArtifactAttestationPath = Join-Path $repo '.cargo-target\release\hoi4-production-prebuilt-artifacts.json'
}
. $attestationHelpers
$prebuilt = Test-ProductionArtifactAttestation -RepoRoot $repo -ReceiptPath $PrebuiltArtifactAttestationPath -ExpectedDriverPath $productionDriver
$resolvedAttestation = (Resolve-Path -LiteralPath $prebuilt.receipt_path).Path
if (-not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.recorder_host).Equals($resolvedRecorder, [StringComparison]::OrdinalIgnoreCase) -or
    -not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.collector_dll).Equals($resolvedCollector, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'X10 recorder and collector must come from the current central Release attestation.'
}

function ConvertTo-NativeArgumentLine {
    param([Parameter(Mandatory = $true)][object[]]$Arguments)
    return (($Arguments | ForEach-Object {
        $value = [string]$_
        if ($value -match '[\s"]') { '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"' } else { $value }
    }) -join ' ')
}
function Write-AtomicBytes([string]$Path, [byte[]]$Bytes) {
    $full = [IO.Path]::GetFullPath($Path); $parent = [IO.Path]::GetDirectoryName($full)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllBytes($temporary, $Bytes)
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($full)) { [IO.File]::Replace($temporary, $full, $null, $true) } else { [IO.File]::Move($temporary, $full) }
    } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
}
function Write-AtomicUtf8([string]$Path, [string]$Text) { Write-AtomicBytes $Path ([Text.UTF8Encoding]::new($false).GetBytes($Text)) }

$bindingPaths = [ordered]@{
    mod_state = Join-Path $resolvedBindings 'mod_state.json'
    dlc_state = Join-Path $resolvedBindings 'dlc_state.json'
    load_order = Join-Path $resolvedBindings 'load_order.json'
    source_inventory = Join-Path $resolvedBindings 'source_inventory.json'
    resolution_report = Join-Path $resolvedBindings 'resolution_report.json'
}
foreach ($key in @($bindingPaths.Keys)) {
    if (-not (Test-Path -LiteralPath $bindingPaths[$key] -PathType Leaf)) { throw "X10 static-bundle binding is missing: $key" }
    $bindingPaths[$key] = (Resolve-Path -LiteralPath $bindingPaths[$key]).Path
}
$bundleDir = Split-Path -Parent $resolvedManifest
$preflightOutput = @(& $resolvedPython $staticVerifier verify --bundle $bundleDir --exe $resolvedExe `
    --mod-state $bindingPaths.mod_state --dlc-state $bindingPaths.dlc_state --load-order $bindingPaths.load_order `
    --source-inventory $bindingPaths.source_inventory --live-resolution-report $bindingPaths.resolution_report --allow-deferred-token-gate)
if ($LASTEXITCODE -ne 0) { throw 'X10 fixture static-bundle pre-injection verification failed.' }
$preflightText = $preflightOutput -join [Environment]::NewLine
$preflight = $preflightText | ConvertFrom-Json
if ($preflight.passed -ne $true -or $preflight.verification_phase -cne 'pre_injection_deferred_allowed' -or
    [string]$preflight.product_modset_sha256 -cne [string]$profile.product_modset_sha256) {
    throw 'X10 fixture modset is not the action profile product modset.'
}
$preflightReceipt = Join-Path $resolvedOut 'x10_static_bundle_preflight.json'
Write-AtomicUtf8 $preflightReceipt ($preflightText + [Environment]::NewLine)

$continueGamePath = Join-Path $resolvedUserDir 'continue_game.json'
$continuePriorExists = Test-Path -LiteralPath $continueGamePath -PathType Leaf
$continuePriorBytes = if ($continuePriorExists) { [IO.File]::ReadAllBytes($continueGamePath) } else { $null }
$continueObject = if ($continuePriorExists) { Get-Content -LiteralPath $continueGamePath -Raw | ConvertFrom-Json } else {
    [pscustomobject]@{ title = [IO.Path]::GetFileNameWithoutExtension($resolvedSave); desc = ''; date = [DateTime]::UtcNow.ToString('R'); filename = ''; is_remote = $false }
}
$continueObject.filename = [IO.Path]::GetFileName($resolvedSave); $continueObject.is_remote = $false
$continueReplacement = ($continueObject | ConvertTo-Json -Depth 8) + [Environment]::NewLine

$sessionId = [guid]::NewGuid().ToString(); $captureId = [guid]::NewGuid().ToString()
$arguments = @(
    '--early-launch-exe', $resolvedExe,
    '--pdx-user-dir', $resolvedUserDir,
    '--dll', $resolvedCollector,
    '--out-dir', $resolvedOut,
    '--session-id', $sessionId,
    '--capture-id', $captureId,
    '--duration-s', $DurationSeconds,
    '--sample-ms', $SampleMilliseconds,
    '--min-game-hours', '2',
    '--stream-format', 'compact',
    '--profile', 'debug',
    '--scheduled-domain', 'focus_research_decisions_depth_live',
    '--scheduled-domains-exact',
    '--static-bundle-manifest', $resolvedManifest,
    '--static-bundle-mod-state', $bindingPaths.mod_state,
    '--static-bundle-dlc-state', $bindingPaths.dlc_state,
    '--static-bundle-load-order', $bindingPaths.load_order,
    '--static-bundle-source-inventory', $bindingPaths.source_inventory,
    '--static-bundle-live-resolution-report', $bindingPaths.resolution_report,
    '--static-bundle-verifier', $staticVerifier,
    '--static-bundle-python', $resolvedPython,
    '--static-bundle-timeout-ms', '1800000',
    '--sidecar-log',
    '--hour-latch'
)
foreach ($value in $EarlyLaunchArgument) { $arguments += @('--early-launch-arg', $value) }

$captureManifest = Join-Path $resolvedOut 'native_session_manifest.json'
$actionReceipt = Join-Path $resolvedOut 'x10_authenticated_fixture_actions.json'
$verification = Join-Path $resolvedOut 'x10_direct_transition_owner_fixture_verification.json'
$recorder = [Diagnostics.Process]::new(); $recorderStarted = $false; $stdoutTask = $null; $stderrTask = $null
$continueOverridden = $false; $captureCompleted = $false; $exactProcess = $null
try {
    $continueOverridden = $true; Write-AtomicUtf8 $continueGamePath $continueReplacement
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $resolvedRecorder; $start.UseShellExecute = $false; $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true
    $start.EnvironmentVariables['BATTLE_PLAN_D45_X10_OWNER_FIXTURE'] = '1'
    if ($null -ne $start.PSObject.Properties['ArgumentList']) { foreach ($argument in $arguments) { [void]$start.ArgumentList.Add([string]$argument) } }
    else { $start.Arguments = ConvertTo-NativeArgumentLine $arguments }
    $recorder.StartInfo = $start
    if (-not $recorder.Start()) { throw 'X10 collector_record failed to start.' }
    $recorderStarted = $true; $stdoutTask = $recorder.StandardOutput.ReadToEndAsync(); $stderrTask = $recorder.StandardError.ReadToEndAsync()
    $deadline = [DateTime]::UtcNow.AddSeconds($ProcessTimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline -and -not $recorder.HasExited -and $null -eq $exactProcess) {
        $candidates = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
        if ($candidates.Count -gt 1) { throw 'X10 clean launch produced more than one hoi4.exe process.' }
        foreach ($candidate in $candidates) {
            try {
                $cim = Get-CimInstance Win32_Process -Filter "ProcessId=$($candidate.Id)" -ErrorAction Stop
                if ([uint32]$cim.ParentProcessId -ne [uint32]$recorder.Id -or $candidate.MainWindowHandle -eq [IntPtr]::Zero -or
                    -not [IO.Path]::GetFullPath([string]$cim.ExecutablePath).Equals($resolvedExe, [StringComparison]::OrdinalIgnoreCase) -or
                    (Get-FileHash -LiteralPath ([string]$cim.ExecutablePath) -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) { continue }
                $loaded = @($candidate.Modules | Where-Object { [IO.Path]::GetFullPath($_.FileName).Equals($resolvedCollector, [StringComparison]::OrdinalIgnoreCase) })
                if ($loaded.Count -eq 1) { $exactProcess = $candidate; break }
            } catch { }
        }
        if ($null -eq $exactProcess) { Start-Sleep -Milliseconds 250 }
    }
    if ($null -eq $exactProcess) {
        if ($recorder.HasExited) { throw "X10 recorder exited before the authenticated game became ready (code $($recorder.ExitCode))." }
        throw 'X10 exact-a729 process/collector tuple did not become ready before the deadline.'
    }
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $actionDriver `
        -Hoi4Exe $resolvedExe -CollectorDll $resolvedCollector -ProfilePath $resolvedProfile -ReceiptPath $actionReceipt `
        -ExistingPid $exactProcess.Id -SessionId $sessionId -CaptureId $captureId -ProcessTimeoutSeconds $ProcessTimeoutSeconds
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $actionReceipt -PathType Leaf)) { throw 'X10 authenticated fixture action driver failed.' }
    # The action driver's first step has now authenticated the selected paused
    # fixture save. Restore launcher state before the long recorder finalization.
    if ($continuePriorExists) { Write-AtomicBytes $continueGamePath $continuePriorBytes } elseif (Test-Path -LiteralPath $continueGamePath) { Remove-Item -LiteralPath $continueGamePath -Force }
    $continueOverridden = $false
    if (-not $recorder.WaitForExit([Math]::Max(60000, ($DurationSeconds + 600) * 1000))) { throw 'X10 recorder exceeded its bounded finalization deadline.' }
    if ($recorder.ExitCode -ne 0) { throw "X10 collector_record exited with code $($recorder.ExitCode)." }
    $captureCompleted = $true
} finally {
    Remove-Item Env:BATTLE_PLAN_D45_X10_OWNER_FIXTURE -ErrorAction SilentlyContinue
    if ($continueOverridden) {
        if ($continuePriorExists) { Write-AtomicBytes $continueGamePath $continuePriorBytes } elseif (Test-Path -LiteralPath $continueGamePath) { Remove-Item -LiteralPath $continueGamePath -Force }
    }
    if ($recorderStarted -and -not $recorder.HasExited) { if (-not $recorder.WaitForExit(30000)) { Stop-Process -Id $recorder.Id -Force -ErrorAction SilentlyContinue } }
    if ($recorderStarted -and $recorder.HasExited) {
        if ($null -ne $stdoutTask) { Write-AtomicUtf8 (Join-Path $resolvedOut 'collector_record.stdout.log') ([string]$stdoutTask.Result) }
        if ($null -ne $stderrTask) { Write-AtomicUtf8 (Join-Path $resolvedOut 'collector_record.stderr.log') ([string]$stderrTask.Result) }
    }
    $recorder.Dispose()
}
if (-not $captureCompleted -or -not (Test-Path -LiteralPath $captureManifest -PathType Leaf)) { throw 'X10 capture did not complete with a recorder manifest.' }
$manifest = Get-Content -LiteralPath $captureManifest -Raw | ConvertFrom-Json
$replayName = [string]$manifest.replay_file
if ([string]::IsNullOrWhiteSpace($replayName) -or [IO.Path]::GetFileName($replayName) -cne $replayName) { throw 'X10 recorder replay_file is absent or not a direct output filename.' }
$replay = [IO.Path]::GetFullPath((Join-Path $resolvedOut $replayName))
if (-not (Test-Path -LiteralPath $replay -PathType Leaf)) { throw "X10 replay was not produced: $replay" }

& $resolvedPython $verifier --contract $contract --action-profile $resolvedProfile --action-receipt $actionReceipt `
    --capture-manifest $captureManifest --prebuilt-attestation $resolvedAttestation --static-bundle-preflight $preflightReceipt `
    --replay $replay --out $verification
if ($LASTEXITCODE -ne 0) { throw "X10 owner fixture was rejected; inspect $verification" }
Write-Output "X10 direct-transition owner fixture accepted: $verification"
