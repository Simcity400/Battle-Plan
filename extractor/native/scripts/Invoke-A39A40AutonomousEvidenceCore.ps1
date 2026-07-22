[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [ValidateSet('A39', 'A40')] [string] $Owner,
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
    [int] $DurationSeconds = 900,
    [int] $SampleMilliseconds = 100,
    [int] $ProcessTimeoutSeconds = 420,
    [int] $ExpectedGameHourRaw = 60830441,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$expectedD04SaveSha256 = 'c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735'
if ($DurationSeconds -lt 300 -or $DurationSeconds -gt 7200) { throw 'DurationSeconds must be in 300..7200.' }
if ($SampleMilliseconds -lt 25 -or $SampleMilliseconds -gt 1000) { throw 'SampleMilliseconds must be in 25..1000.' }
if ($ProcessTimeoutSeconds -lt 30 -or $ProcessTimeoutSeconds -gt 1800) { throw 'ProcessTimeoutSeconds must be in 30..1800.' }
if ($ExpectedGameHourRaw -ne 60830441) { throw "$Owner production automation is frozen to New D04 Save Deep War raw hour 60830441." }
foreach ($path in @($RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll, $ContinueSave, $StaticBundleManifest, $StaticBundleBindingsDir, $ActionProfile)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required path does not exist: $path" }
}
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
    throw "$Owner autonomous evidence requires a clean early launch."
}

$resolvedRecorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollectorSource = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedSave = (Resolve-Path -LiteralPath $ContinueSave).Path
$resolvedBundleManifest = (Resolve-Path -LiteralPath $StaticBundleManifest).Path
$resolvedBindings = (Resolve-Path -LiteralPath $StaticBundleBindingsDir).Path
$resolvedProfile = (Resolve-Path -LiteralPath $ActionProfile).Path
$resolvedPython = (Get-Command $Python -CommandType Application -ErrorAction Stop).Source
$resolvedOut = [IO.Path]::GetFullPath($OutDir)
if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) { throw 'Hoi4Exe does not match exact a729 authority.' }
if ((Get-FileHash -LiteralPath $resolvedSave -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedD04SaveSha256) { throw "$Owner production automation requires the frozen New D04 Save Deep War identity." }
$saveDirectory = [IO.Path]::GetFullPath((Join-Path $resolvedUserDir 'save games')).TrimEnd([char]'\')
$requestedSaveDirectory = [IO.Path]::GetFullPath((Split-Path -Parent $resolvedSave)).TrimEnd([char]'\')
if (-not $requestedSaveDirectory.Equals($saveDirectory, [StringComparison]::OrdinalIgnoreCase)) { throw "ContinueSave must be a direct child of $saveDirectory" }
$continueIndex = [Array]::IndexOf($EarlyLaunchArgument, '--continuelastsave')
$gdprIndex = [Array]::IndexOf($EarlyLaunchArgument, '--gdpr-compliant')
if ($continueIndex -lt 0 -or $gdprIndex -lt $continueIndex) { throw 'EarlyLaunchArgument must contain --continuelastsave followed by --gdpr-compliant.' }
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty: $resolvedOut"
    }
} else { [IO.Directory]::CreateDirectory($resolvedOut) | Out-Null }

$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$productionDriver = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1')).Path
$attestationHelpers = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1')).Path
$actionDriver = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Invoke-A39A40AuthenticatedUiActions.ps1')).Path
$verifier = if ($Owner -ceq 'A39') {
    (Resolve-Path -LiteralPath (Join-Path $repo 'extractor\documents\dossiers\A39\verify_a39_character_presentation_live_fixture.py')).Path
} else {
    (Resolve-Path -LiteralPath (Join-Path $repo 'extractor\documents\dossiers\A40\verify_a40_design_peace_live_fixture.py')).Path
}
$releaseRoot = Join-Path $repo '.cargo-target\release'
if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) { $PrebuiltArtifactAttestationPath = Join-Path $releaseRoot 'hoi4-production-prebuilt-artifacts.json' }
. $attestationHelpers
$prebuilt = Test-ProductionArtifactAttestation -RepoRoot $repo -ReceiptPath $PrebuiltArtifactAttestationPath -ExpectedDriverPath $productionDriver
$resolvedAttestation = (Resolve-Path -LiteralPath $prebuilt.receipt_path).Path
if (-not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.recorder_host).Equals($resolvedRecorder, [StringComparison]::OrdinalIgnoreCase) -or
    -not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.collector_dll).Equals($resolvedCollectorSource, [StringComparison]::OrdinalIgnoreCase)) {
    throw "$Owner recorder and collector must come from the current authenticated central Release attestation."
}

function ConvertTo-NativeArgumentLine {
    param([Parameter(Mandatory = $true)][object[]]$Arguments)
    return (($Arguments | ForEach-Object {
        $value = [string]$_
        if ($value -match '[\s"]') { '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"' } else { $value }
    }) -join ' ')
}
function Write-AtomicBytes([string]$Path, [byte[]]$Bytes, [switch]$AllowReplace) {
    $full = [IO.Path]::GetFullPath($Path); $parent = [IO.Path]::GetDirectoryName($full)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllBytes($temporary, $Bytes)
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($full)) {
            if (-not $AllowReplace) { throw "Refusing to replace existing file: $full" }
            [IO.File]::Replace($temporary, $full, $null, $true)
        } else { [IO.File]::Move($temporary, $full) }
    } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
}
function Write-AtomicUtf8([string]$Path, [string]$Text, [switch]$AllowReplace) {
    Write-AtomicBytes $Path ([Text.UTF8Encoding]::new($false).GetBytes($Text)) -AllowReplace:$AllowReplace
}

$loadedCollector = Join-Path $resolvedOut ("{0}-evidence-collector.dll" -f $Owner.ToLowerInvariant())
Write-AtomicBytes $loadedCollector ([IO.File]::ReadAllBytes($resolvedCollectorSource))
if ((Get-FileHash -LiteralPath $loadedCollector -Algorithm SHA256).Hash.ToLowerInvariant() -cne
    (Get-FileHash -LiteralPath $resolvedCollectorSource -Algorithm SHA256).Hash.ToLowerInvariant()) { throw 'Copied collector identity mismatch.' }
$rawReceipt = Join-Path $resolvedOut ($(if ($Owner -ceq 'A39') { 'a39_character_presentation_evidence_capture.json' } else { 'a40_design_peace_evidence_capture.json' }))
$manifestPath = Join-Path $resolvedOut 'native_session_manifest.json'
$actionReceipt = Join-Path $resolvedOut ("{0}_authenticated_actions.json" -f $Owner.ToLowerInvariant())
$regradePath = Join-Path $resolvedOut ($(if ($Owner -ceq 'A39') { 'a39_character_presentation_live_fixture_regrade.json' } else { 'a40_design_peace_live_fixture_regrade.json' }))
$immutablePath = Join-Path $resolvedOut ("{0}_autonomous_evidence_receipt.json" -f $Owner.ToLowerInvariant())
$sessionId = [guid]::NewGuid().ToString().ToLowerInvariant(); $captureId = [guid]::NewGuid().ToString().ToLowerInvariant()

$bindingPaths = [ordered]@{
    mod_state = Join-Path $resolvedBindings 'mod_state.json'; dlc_state = Join-Path $resolvedBindings 'dlc_state.json'; load_order = Join-Path $resolvedBindings 'load_order.json'
    source_inventory = Join-Path $resolvedBindings 'source_inventory.json'; resolution_report = Join-Path $resolvedBindings 'resolution_report.json'
}
foreach ($key in @($bindingPaths.Keys)) {
    if (-not (Test-Path -LiteralPath $bindingPaths[$key] -PathType Leaf)) { throw "Static-bundle binding $key is missing." }
    $bindingPaths[$key] = (Resolve-Path -LiteralPath $bindingPaths[$key]).Path
}

$arguments = @(
    '--early-launch-exe', $resolvedExe, '--pdx-user-dir', $resolvedUserDir, '--dll', $loadedCollector, '--out-dir', $resolvedOut,
    '--session-id', $sessionId, '--capture-id', $captureId, '--duration-s', $DurationSeconds, '--sample-ms', $SampleMilliseconds,
    '--min-game-hours', '0', '--stream-format', 'compact', '--profile', 'debug', '--scheduled-domain', 'fuel_live', '--scheduled-domains-exact',
    '--static-bundle-manifest', $resolvedBundleManifest, '--static-bundle-mod-state', $bindingPaths.mod_state, '--static-bundle-dlc-state', $bindingPaths.dlc_state,
    '--static-bundle-load-order', $bindingPaths.load_order, '--static-bundle-source-inventory', $bindingPaths.source_inventory,
    '--static-bundle-live-resolution-report', $bindingPaths.resolution_report, '--static-bundle-timeout-ms', '1800000', '--sidecar-log', '--skip-host-fold',
    $(if ($Owner -ceq 'A39') { '--a39-character-presentation-evidence' } else { '--a40-design-peace-evidence' })
)
foreach ($value in $EarlyLaunchArgument) { $arguments += @('--early-launch-arg', $value) }

$continueGamePath = Join-Path $resolvedUserDir 'continue_game.json'
$continuePriorExists = Test-Path -LiteralPath $continueGamePath -PathType Leaf
$continuePriorBytes = if ($continuePriorExists) { [IO.File]::ReadAllBytes($continueGamePath) } else { $null }
$continueObject = if ($continuePriorExists) { Get-Content -LiteralPath $continueGamePath -Raw | ConvertFrom-Json } else {
    [pscustomobject]@{ title = [IO.Path]::GetFileNameWithoutExtension($resolvedSave); desc = ''; date = [DateTime]::UtcNow.ToString('R'); filename = ''; is_remote = $false }
}
$continueObject.filename = [IO.Path]::GetFileName($resolvedSave); $continueObject.is_remote = $false
$continueReplacement = ($continueObject | ConvertTo-Json -Depth 8) + [Environment]::NewLine

$recorder = [Diagnostics.Process]::new(); $recorderStarted = $false; $continueOverridden = $false
$stdoutTask = $null; $stderrTask = $null; $exactProcess = $null; $captureComplete = $false
$exactProcessStartUtc = $null
try {
    $continueOverridden = $true; Write-AtomicUtf8 $continueGamePath $continueReplacement -AllowReplace
    $start = [Diagnostics.ProcessStartInfo]::new(); $start.FileName = $resolvedRecorder; $start.UseShellExecute = $false; $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true
    if ($null -ne $start.PSObject.Properties['ArgumentList']) { foreach ($argument in $arguments) { [void]$start.ArgumentList.Add([string]$argument) } }
    else { $start.Arguments = ConvertTo-NativeArgumentLine $arguments }
    $recorder.StartInfo = $start
    if (-not $recorder.Start()) { throw 'collector_record failed to start.' }
    $recorderStarted = $true; $stdoutTask = $recorder.StandardOutput.ReadToEndAsync(); $stderrTask = $recorder.StandardError.ReadToEndAsync()

    $deadline = [DateTime]::UtcNow.AddSeconds($ProcessTimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline -and -not $recorder.HasExited -and $null -eq $exactProcess) {
        $candidates = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
        if ($candidates.Count -gt 1) { throw "$Owner clean launch produced more than one hoi4.exe process." }
        foreach ($candidate in $candidates) {
            try {
                $cim = Get-CimInstance Win32_Process -Filter "ProcessId=$($candidate.Id)" -ErrorAction Stop
                if ([uint32]$cim.ParentProcessId -ne [uint32]$recorder.Id -or $candidate.MainWindowHandle -eq [IntPtr]::Zero -or
                    -not [IO.Path]::GetFullPath([string]$cim.ExecutablePath).Equals($resolvedExe, [StringComparison]::OrdinalIgnoreCase) -or
                    (Get-FileHash -LiteralPath ([string]$cim.ExecutablePath) -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) { continue }
                $loaded = @($candidate.Modules | Where-Object { [IO.Path]::GetFullPath([string]$_.FileName).Equals($loadedCollector, [StringComparison]::OrdinalIgnoreCase) })
                if ($loaded.Count -eq 1) { $exactProcess = $candidate; break }
            } catch { }
        }
        if ($null -eq $exactProcess) { Start-Sleep -Milliseconds 250 }
    }
    if ($null -eq $exactProcess) { throw "$Owner did not authenticate an exact-a729 process/collector tuple within $ProcessTimeoutSeconds seconds." }
    $exactProcessStartUtc = $exactProcess.StartTime.ToUniversalTime()

    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $actionDriver -Owner $Owner -Hoi4Exe $resolvedExe -CollectorDll $loadedCollector -ProfilePath $resolvedProfile -ReceiptPath $actionReceipt -ExistingPid $exactProcess.Id -ProcessTimeoutSeconds $ProcessTimeoutSeconds
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $actionReceipt -PathType Leaf)) { throw "$Owner autonomous action driver failed." }

    if (-not $recorder.WaitForExit([Math]::Max(60000, ($DurationSeconds + 600) * 1000))) { throw "collector_record exceeded the bounded $Owner finalization deadline." }
    if ($recorder.ExitCode -ne 0) { throw "collector_record exited with code $($recorder.ExitCode)." }
    $captureComplete = $true
    if ($null -ne (Get-Process -Id $exactProcess.Id -ErrorAction SilentlyContinue)) {
        Stop-Process -Id $exactProcess.Id -Force -ErrorAction Stop
    }
} finally {
    if ($continueOverridden) {
        if ($continuePriorExists) { Write-AtomicBytes $continueGamePath $continuePriorBytes -AllowReplace }
        elseif (Test-Path -LiteralPath $continueGamePath -PathType Leaf) { Remove-Item -LiteralPath $continueGamePath -Force }
    }
    if ($recorderStarted -and -not $recorder.HasExited) {
        if (-not $recorder.WaitForExit(30000)) { Stop-Process -Id $recorder.Id -Force -ErrorAction SilentlyContinue }
    }
    if ($recorderStarted -and $null -ne $stdoutTask -and $null -ne $stderrTask) {
        try { Write-AtomicUtf8 (Join-Path $resolvedOut 'recorder.stdout.log') ($stdoutTask.GetAwaiter().GetResult()) } catch { }
        try { Write-AtomicUtf8 (Join-Path $resolvedOut 'recorder.stderr.log') ($stderrTask.GetAwaiter().GetResult()) } catch { }
    }
}
if (-not $captureComplete -or -not (Test-Path -LiteralPath $manifestPath -PathType Leaf) -or -not (Test-Path -LiteralPath $rawReceipt -PathType Leaf)) {
    throw "$Owner collector did not finalize its exact raw receipt and session manifest."
}

& $resolvedPython $verifier --receipt $rawReceipt --capture-manifest $manifestPath --collector-dll $loadedCollector --action-receipt $actionReceipt --out $regradePath
if ($LASTEXITCODE -ne 0) { throw "$Owner raw fixture failed its independent regrade; evidence is retained at $resolvedOut" }
$regrade = Get-Content -LiteralPath $regradePath -Raw | ConvertFrom-Json
if ($regrade.accepted_raw_fixture -ne $true -or $regrade.production_activation_authorized -ne $false -or [int]$regrade.process_id -ne [int]$exactProcess.Id) {
    throw "$Owner regrade did not retain its fail-closed acceptance contract."
}

$immutable = [ordered]@{
    schema = "hoi4.$($Owner.ToLowerInvariant()).autonomous-evidence-acquisition.v1"
    status = 'pass_exact_collector_join_and_independent_regrade'
    timestamp_utc = [DateTime]::UtcNow.ToString('o')
    executable_sha256 = $expectedExeSha256
    save = [ordered]@{ logical_name = [IO.Path]::GetFileName($resolvedSave); sha256 = $expectedD04SaveSha256; initial_game_hour_raw = $ExpectedGameHourRaw }
    process = [ordered]@{ id = [int]$exactProcess.Id; start_utc = $exactProcessStartUtc.ToString('o') }
    session_id = $sessionId; capture_id = $captureId
    files = [ordered]@{
        collector_receipt = [ordered]@{ logical_name = [IO.Path]::GetFileName($rawReceipt); sha256 = (Get-FileHash -LiteralPath $rawReceipt -Algorithm SHA256).Hash.ToLowerInvariant() }
        recorder_manifest = [ordered]@{ logical_name = [IO.Path]::GetFileName($manifestPath); sha256 = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant() }
        action_receipt = [ordered]@{ logical_name = [IO.Path]::GetFileName($actionReceipt); sha256 = (Get-FileHash -LiteralPath $actionReceipt -Algorithm SHA256).Hash.ToLowerInvariant() }
        independent_regrade = [ordered]@{ logical_name = [IO.Path]::GetFileName($regradePath); sha256 = (Get-FileHash -LiteralPath $regradePath -Algorithm SHA256).Hash.ToLowerInvariant() }
    }
    frozen_inputs = [ordered]@{
        central_release_attestation_sha256 = (Get-FileHash -LiteralPath $resolvedAttestation -Algorithm SHA256).Hash.ToLowerInvariant()
        static_bundle_manifest_sha256 = (Get-FileHash -LiteralPath $resolvedBundleManifest -Algorithm SHA256).Hash.ToLowerInvariant()
        action_profile_sha256 = (Get-FileHash -LiteralPath $resolvedProfile -Algorithm SHA256).Hash.ToLowerInvariant()
        action_driver_sha256 = (Get-FileHash -LiteralPath $actionDriver -Algorithm SHA256).Hash.ToLowerInvariant()
        independent_regrader_sha256 = (Get-FileHash -LiteralPath $verifier -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    exact_collector_process_join = $true
    semantic_authority = 'collector receipt plus independent regrade; action receipt is provenance only'
    production_activation_authorized = $false
}
Write-AtomicUtf8 $immutablePath (($immutable | ConvertTo-Json -Depth 16) + "`n")
Write-Output "$Owner autonomous evidence accepted: $immutablePath"
