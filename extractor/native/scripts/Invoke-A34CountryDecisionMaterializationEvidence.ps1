[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $RecorderExe,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $PdxUserDir,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [Parameter(Mandatory = $true)] [string] $FixturePlan,
    [Parameter(Mandatory = $true)] [string] $NonemptySave,
    [Parameter(Mandatory = $true)] [string] $MeasuredEmptySave,
    [Parameter(Mandatory = $true)] [string] $StaticBundleManifest,
    [Parameter(Mandatory = $true)] [string] $StaticBundleBindingsDir,
    [string] $PrebuiltArtifactAttestationPath = '',
    [int] $DurationSeconds = 120,
    [int] $SampleMilliseconds = 100,
    [int] $ProcessTimeoutSeconds = 300,
    [int] $ActionSettleMilliseconds = 8000,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
if ($DurationSeconds -lt 30 -or $DurationSeconds -gt 600) { throw 'DurationSeconds must be in 30..600.' }
if ($SampleMilliseconds -lt 10 -or $SampleMilliseconds -gt 1000) { throw 'SampleMilliseconds must be in 10..1000.' }
if ($ProcessTimeoutSeconds -lt 30 -or $ProcessTimeoutSeconds -gt 1800) { throw 'ProcessTimeoutSeconds must be in 30..1800.' }
foreach ($path in @(
    $RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll, $FixturePlan,
    $NonemptySave, $MeasuredEmptySave, $StaticBundleManifest, $StaticBundleBindingsDir
)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required A34 path does not exist: $path" }
}
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
    throw 'A34 paired evidence requires two clean early launches; close every existing hoi4.exe first.'
}

$resolvedRecorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedPdxUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedFixturePlanSource = (Resolve-Path -LiteralPath $FixturePlan).Path
$resolvedNonemptySave = (Resolve-Path -LiteralPath $NonemptySave).Path
$resolvedMeasuredEmptySave = (Resolve-Path -LiteralPath $MeasuredEmptySave).Path
$resolvedStaticBundleManifest = (Resolve-Path -LiteralPath $StaticBundleManifest).Path
$resolvedStaticBundleBindingsDir = (Resolve-Path -LiteralPath $StaticBundleBindingsDir).Path
$resolvedStaticBundleDir = Split-Path -Parent $resolvedStaticBundleManifest
$resolvedPython = (Get-Command $Python -CommandType Application -ErrorAction Stop).Source
$resolvedOut = [IO.Path]::GetFullPath($OutDir)
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\..')).Path
$productionDriver = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1')).Path
$attestationHelpers = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1')).Path
$staticBundleVerifier = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\game_defs\static_bundle.py')).Path
$readinessHelper = (Resolve-Path -LiteralPath (Join-Path $repoRoot 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_wait_for_paused_save.ps1')).Path
$actionDriver = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Invoke-A34AutonomousDecisionPanelAction.ps1')).Path
$verifier = (Resolve-Path -LiteralPath (Join-Path $repoRoot 'extractor\documents\dossiers\A34\verify_a34_country_decision_materialization.py')).Path

if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) {
    throw 'Hoi4Exe does not match exact a729 authority.'
}
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty so A34 evidence is immutable: $resolvedOut"
    }
} else {
    [IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
}

$saveDirectory = [IO.Path]::GetFullPath((Join-Path $resolvedPdxUserDir 'save games')).TrimEnd([char]'\')
foreach ($save in @($resolvedNonemptySave, $resolvedMeasuredEmptySave)) {
    $parent = [IO.Path]::GetFullPath((Split-Path -Parent $save)).TrimEnd([char]'\')
    if (-not $parent.Equals($saveDirectory, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Every A34 continue save must be a direct child of $saveDirectory"
    }
}
$continueIndex = [Array]::IndexOf($EarlyLaunchArgument, '--continuelastsave')
$gdprIndex = [Array]::IndexOf($EarlyLaunchArgument, '--gdpr-compliant')
if ($continueIndex -lt 0 -or $gdprIndex -lt $continueIndex) {
    throw 'EarlyLaunchArgument must contain --continuelastsave followed by --gdpr-compliant.'
}

$releaseRoot = Join-Path $repoRoot '.cargo-target\release'
if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) {
    $PrebuiltArtifactAttestationPath = Join-Path $releaseRoot 'hoi4-production-prebuilt-artifacts.json'
}
. $attestationHelpers
$prebuilt = Test-ProductionArtifactAttestation `
    -RepoRoot $repoRoot `
    -ReceiptPath $PrebuiltArtifactAttestationPath `
    -ExpectedDriverPath $productionDriver
if (-not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.recorder_host).Equals($resolvedRecorder, [StringComparison]::OrdinalIgnoreCase) -or
    -not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.collector_dll).Equals($resolvedCollector, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'A34 recorder and collector must come from the current authenticated central Release attestation.'
}
$resolvedAttestation = (Resolve-Path -LiteralPath ([string]$prebuilt.receipt_path)).Path

function Write-AtomicCreateNewBytes([string] $Path, [byte[]] $Bytes) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($fullPath)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    if ([IO.File]::Exists($fullPath)) { throw "Refusing to replace immutable A34 artifact: $fullPath" }
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllBytes($temporary, $Bytes)
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        [IO.File]::Move($temporary, $fullPath)
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

function Write-AtomicCreateNewUtf8([string] $Path, [string] $Text) {
    Write-AtomicCreateNewBytes $Path ([Text.UTF8Encoding]::new($false).GetBytes($Text))
}

function Write-AtomicReplaceBytes([string] $Path, [byte[]] $Bytes) {
    $fullPath = [IO.Path]::GetFullPath($Path)
    $parent = [IO.Path]::GetDirectoryName($fullPath)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        [IO.File]::WriteAllBytes($temporary, $Bytes)
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($fullPath)) { [IO.File]::Replace($temporary, $fullPath, $null, $true) }
        else { [IO.File]::Move($temporary, $fullPath) }
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

function Copy-AtomicCreateNew([string] $Source, [string] $Destination) {
    $fullDestination = [IO.Path]::GetFullPath($Destination)
    $parent = [IO.Path]::GetDirectoryName($fullDestination)
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    if ([IO.File]::Exists($fullDestination)) { throw "Refusing to replace immutable A34 artifact: $fullDestination" }
    $temporary = [IO.Path]::Combine($parent, ([IO.Path]::GetRandomFileName() + '.tmp'))
    try {
        $input = [IO.File]::Open($Source, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        try {
            $output = [IO.File]::Open($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
            try { $input.CopyTo($output); $output.Flush($true) } finally { $output.Dispose() }
        } finally { $input.Dispose() }
        [IO.File]::Move($temporary, $fullDestination)
    } finally {
        if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) }
    }
}

function ConvertTo-NativeArgumentLine {
    param([Parameter(Mandatory = $true)][object[]]$Arguments)
    return (($Arguments | ForEach-Object {
        $value = [string]$_
        if ($value -match '[\s"]') {
            '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
        } else { $value }
    }) -join ' ')
}

Copy-AtomicCreateNew $resolvedAttestation (Join-Path $resolvedOut 'prebuilt_artifact_attestation.json')

$bindingPaths = [ordered]@{
    mod_state = Join-Path $resolvedStaticBundleBindingsDir 'mod_state.json'
    dlc_state = Join-Path $resolvedStaticBundleBindingsDir 'dlc_state.json'
    load_order = Join-Path $resolvedStaticBundleBindingsDir 'load_order.json'
    source_inventory = Join-Path $resolvedStaticBundleBindingsDir 'source_inventory.json'
    resolution_report = Join-Path $resolvedStaticBundleBindingsDir 'resolution_report.json'
}
foreach ($key in @($bindingPaths.Keys)) {
    if (-not (Test-Path -LiteralPath $bindingPaths[$key] -PathType Leaf)) {
        throw "A34 static-bundle binding $key is missing: $($bindingPaths[$key])"
    }
    $bindingPaths[$key] = (Resolve-Path -LiteralPath $bindingPaths[$key]).Path
}

# Authenticate the selected product bundle before extracting the exact decision
# artifact. This packages the byte identity consumed by both fixture cases;
# neither a workspace cache nor a rebuilt lookalike is accepted.
$staticPreflightOutput = @(& $resolvedPython $staticBundleVerifier verify `
    --bundle $resolvedStaticBundleDir `
    --exe $resolvedExe `
    --mod-state $bindingPaths['mod_state'] `
    --dlc-state $bindingPaths['dlc_state'] `
    --load-order $bindingPaths['load_order'] `
    --source-inventory $bindingPaths['source_inventory'] `
    --live-resolution-report $bindingPaths['resolution_report'] `
    --allow-deferred-token-gate)
if ($LASTEXITCODE -ne 0) { throw 'A34 static-bundle pre-injection verification failed.' }
$staticPreflightText = $staticPreflightOutput -join [Environment]::NewLine
$staticPreflight = $staticPreflightText | ConvertFrom-Json
if ($staticPreflight.passed -ne $true -or $staticPreflight.verification_phase -cne 'pre_injection_deferred_allowed') {
    throw 'A34 static-bundle verifier did not admit the deferred pre-injection phase.'
}
Write-AtomicCreateNewUtf8 (Join-Path $resolvedOut 'a34_static_bundle_preflight.json') ($staticPreflightText + "`n")

$bundleManifestObject = Get-Content -LiteralPath $resolvedStaticBundleManifest -Raw | ConvertFrom-Json
$decisionEntries = @($bundleManifestObject.artifacts | Where-Object { [string]$_.role -ceq 'decisions_static_definitions' })
if ($decisionEntries.Count -ne 1) { throw 'Static bundle must contain exactly one decisions_static_definitions artifact.' }
$decisionEntry = $decisionEntries[0]
$relativeDecisionPath = [string]$decisionEntry.file
if ([string]::IsNullOrWhiteSpace($relativeDecisionPath) -or [IO.Path]::IsPathRooted($relativeDecisionPath)) {
    throw 'Decision artifact manifest path is absent or absolute.'
}
$sourceDecisionCatalog = [IO.Path]::GetFullPath((Join-Path $resolvedStaticBundleDir $relativeDecisionPath))
$bundleRootWithSeparator = [IO.Path]::GetFullPath($resolvedStaticBundleDir).TrimEnd([char]'\') + '\'
if (-not $sourceDecisionCatalog.StartsWith($bundleRootWithSeparator, [StringComparison]::OrdinalIgnoreCase) -or
    -not (Test-Path -LiteralPath $sourceDecisionCatalog -PathType Leaf)) {
    throw 'Decision artifact path escapes the authenticated static bundle or is missing.'
}
$decisionCatalogSha256 = (Get-FileHash -LiteralPath $sourceDecisionCatalog -Algorithm SHA256).Hash.ToLowerInvariant()
$decisionCatalogBytes = (Get-Item -LiteralPath $sourceDecisionCatalog).Length
if ($decisionCatalogSha256 -cne [string]$decisionEntry.sha256 -or $decisionCatalogBytes -ne [long]$decisionEntry.bytes) {
    throw 'Decision artifact bytes differ from the authenticated static-bundle manifest.'
}
$catalogRelativePath = "catalog/decisions_static_definitions.$decisionCatalogSha256.json"
$packagedDecisionCatalog = Join-Path $resolvedOut ($catalogRelativePath -replace '/', '\')
Copy-AtomicCreateNew $sourceDecisionCatalog $packagedDecisionCatalog
$catalogPackage = [ordered]@{
    schema = 'hoi4.a34.clean-decision-catalog-package.v1'
    immutable = $true
    production_activation_authorized = $false
    exact_executable_sha256 = $expectedExeSha256
    static_bundle_manifest_sha256 = (Get-FileHash -LiteralPath $resolvedStaticBundleManifest -Algorithm SHA256).Hash.ToLowerInvariant()
    catalog = [ordered]@{
        role = 'decisions_static_definitions'
        schema = [string]$decisionEntry.schema
        artifact_sha256 = [string]$decisionEntry.artifact_sha256
        relative_path = $catalogRelativePath
        bytes = $decisionCatalogBytes
        sha256 = $decisionCatalogSha256
    }
    source_gate_report_sha256 = [string]$decisionEntry.source_gate_report_sha256
    binding_sha256s = [ordered]@{
        mod_state = (Get-FileHash -LiteralPath $bindingPaths['mod_state'] -Algorithm SHA256).Hash.ToLowerInvariant()
        dlc_state = (Get-FileHash -LiteralPath $bindingPaths['dlc_state'] -Algorithm SHA256).Hash.ToLowerInvariant()
        load_order = (Get-FileHash -LiteralPath $bindingPaths['load_order'] -Algorithm SHA256).Hash.ToLowerInvariant()
        source_inventory = (Get-FileHash -LiteralPath $bindingPaths['source_inventory'] -Algorithm SHA256).Hash.ToLowerInvariant()
        resolution_report = (Get-FileHash -LiteralPath $bindingPaths['resolution_report'] -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}
$catalogPackagePath = Join-Path $resolvedOut 'a34_clean_catalog_package.json'
Write-AtomicCreateNewUtf8 $catalogPackagePath (($catalogPackage | ConvertTo-Json -Depth 8) + "`n")

$fixturePlanObject = Get-Content -LiteralPath $resolvedFixturePlanSource -Raw | ConvertFrom-Json
if ([string]$fixturePlanObject.schema -cne 'hoi4.a34.country-decision-materialization-fixture-plan.v1' -or
    $fixturePlanObject.production_activation_authorized -ne $false) {
    throw 'A34 fixture plan schema/promotion policy is invalid.'
}
$planCases = @($fixturePlanObject.cases)
if ($planCases.Count -ne 2 -or @($planCases | Select-Object -ExpandProperty id -Unique).Count -ne 2 -or
    -not (@($planCases.id) -contains 'nonempty') -or -not (@($planCases.id) -contains 'measured_empty')) {
    throw 'A34 fixture plan must contain exactly nonempty and measured_empty cases.'
}
$resolvedFixturePlan = Join-Path $resolvedOut 'fixture_plan.json'
Copy-AtomicCreateNew $resolvedFixturePlanSource $resolvedFixturePlan

$saveByCase = @{
    nonempty = $resolvedNonemptySave
    measured_empty = $resolvedMeasuredEmptySave
}
foreach ($case in $planCases) {
    $caseId = [string]$case.id
    $saveHash = (Get-FileHash -LiteralPath $saveByCase[$caseId] -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($saveHash -cne [string]$case.continue_save_sha256) {
        throw "A34 $caseId continue-save hash differs from the frozen fixture plan."
    }
    if ([string]$case.supplement.decision_catalog_sha256 -cne $decisionCatalogSha256) {
        throw "A34 $caseId supplement does not bind the packaged decision catalog."
    }
}
if ([string]$planCases[0].continue_save_sha256 -ceq [string]$planCases[1].continue_save_sha256) {
    throw 'A34 nonempty and measured-empty cases must bind distinct frozen save identities.'
}

$collectorOutputRoot = Split-Path -Parent $resolvedCollector
$collectorPayload = Join-Path $collectorOutputRoot 'a34_decision_panel_evidence.dpn1'
$collectorReceipt = Join-Path $collectorOutputRoot 'a34_decision_panel_evidence_receipt.json'
foreach ($path in @($collectorPayload, $collectorReceipt)) {
    if (Test-Path -LiteralPath $path) { throw "Move or remove the prior create-new A34 collector artifact before capture: $path" }
}

$continueGamePath = Join-Path $resolvedPdxUserDir 'continue_game.json'
$continuePriorExists = Test-Path -LiteralPath $continueGamePath -PathType Leaf
$continuePriorBytes = if ($continuePriorExists) { [IO.File]::ReadAllBytes($continueGamePath) } else { $null }
$continueOverridden = $false
try {
    foreach ($caseId in @('nonempty', 'measured_empty')) {
        if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
            throw "A34 $caseId cannot begin because an earlier hoi4.exe is still running."
        }
        $case = @($planCases | Where-Object { [string]$_.id -ceq $caseId })[0]
        $caseRoot = Join-Path $resolvedOut $caseId
        [IO.Directory]::CreateDirectory($caseRoot) | Out-Null
        $supplementPath = Join-Path $caseRoot 'a34_resolved_supplement.json'
        Write-AtomicCreateNewUtf8 $supplementPath (($case.supplement | ConvertTo-Json -Depth 20) + "`n")
        $save = $saveByCase[$caseId]
        $saveIdentity = [ordered]@{
            schema = 'hoi4.a34.fixture-save-identity.v1'
            case_id = $caseId
            filename = [IO.Path]::GetFileName($save)
            bytes = (Get-Item -LiteralPath $save).Length
            sha256 = (Get-FileHash -LiteralPath $save -Algorithm SHA256).Hash.ToLowerInvariant()
            expected_game_hour_raw = [int]$case.expected_game_hour_raw
            production_activation_authorized = $false
        }
        Write-AtomicCreateNewUtf8 (Join-Path $caseRoot 'a34_fixture_save_identity.json') (($saveIdentity | ConvertTo-Json -Depth 6) + "`n")

        $continueObject = if ($continuePriorExists) {
            Get-Content -LiteralPath $continueGamePath -Raw | ConvertFrom-Json
        } else {
            [pscustomobject]@{ title = ''; desc = ''; date = [DateTime]::UtcNow.ToString('R'); filename = ''; is_remote = $false }
        }
        $continueObject.title = [IO.Path]::GetFileNameWithoutExtension($save)
        $continueObject.filename = [IO.Path]::GetFileName($save)
        $continueObject.is_remote = $false
        $continueOverridden = $true
        $continueBytes = [Text.UTF8Encoding]::new($false).GetBytes((($continueObject | ConvertTo-Json -Depth 8) + "`n"))
        Write-AtomicReplaceBytes $continueGamePath $continueBytes

        $sessionId = [guid]::NewGuid().ToString()
        $captureId = [guid]::NewGuid().ToString()
        $arguments = @(
            '--early-launch-exe', $resolvedExe,
            '--pdx-user-dir', $resolvedPdxUserDir,
            '--dll', $resolvedCollector,
            '--out-dir', $caseRoot,
            '--session-id', $sessionId,
            '--capture-id', $captureId,
            '--duration-s', $DurationSeconds,
            '--sample-ms', $SampleMilliseconds,
            '--stream-format', 'compact',
            '--profile', 'debug',
            '--scheduled-domain', 'fuel_live',
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
            '--static-decisions-definitions', $packagedDecisionCatalog,
            '--sidecar-log',
            '--skip-host-fold',
            '--a34-decision-panel-evidence-supplement', $supplementPath
        )
        foreach ($value in $EarlyLaunchArgument) { $arguments += @('--early-launch-arg', $value) }

        $recorder = [Diagnostics.Process]::new()
        $recorderStarted = $false
        $stdoutTask = $null
        $stderrTask = $null
        $exactProcess = $null
        $stdoutPath = Join-Path $caseRoot 'collector_record.stdout.log'
        $stderrPath = Join-Path $caseRoot 'collector_record.stderr.log'
        try {
            $start = [Diagnostics.ProcessStartInfo]::new()
            $start.FileName = $resolvedRecorder
            $start.UseShellExecute = $false
            $start.CreateNoWindow = $true
            $start.RedirectStandardOutput = $true
            $start.RedirectStandardError = $true
            if ($null -ne $start.PSObject.Properties['ArgumentList']) {
                foreach ($argument in $arguments) { [void]$start.ArgumentList.Add([string]$argument) }
            } else {
                $start.Arguments = ConvertTo-NativeArgumentLine $arguments
            }
            $recorder.StartInfo = $start
            if (-not $recorder.Start()) { throw "A34 $caseId collector_record failed to start." }
            $recorderStarted = $true
            $stdoutTask = $recorder.StandardOutput.ReadToEndAsync()
            $stderrTask = $recorder.StandardError.ReadToEndAsync()

            $deadline = [DateTime]::UtcNow.AddSeconds($ProcessTimeoutSeconds)
            while ([DateTime]::UtcNow -lt $deadline -and -not $recorder.HasExited -and $null -eq $exactProcess) {
                $candidates = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
                if ($candidates.Count -gt 1) { throw "A34 $caseId clean launch produced more than one hoi4.exe." }
                foreach ($candidate in $candidates) {
                    try {
                        $cim = Get-CimInstance Win32_Process -Filter "ProcessId=$($candidate.Id)" -ErrorAction Stop
                        if ([uint32]$cim.ParentProcessId -ne [uint32]$recorder.Id -or $candidate.MainWindowHandle -eq [IntPtr]::Zero) { continue }
                        $candidatePath = [string]$cim.ExecutablePath
                        if ([string]::IsNullOrWhiteSpace($candidatePath) -or
                            -not [IO.Path]::GetFullPath($candidatePath).Equals($resolvedExe, [StringComparison]::OrdinalIgnoreCase) -or
                            (Get-FileHash -LiteralPath $candidatePath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) { continue }
                        $loaded = @($candidate.Modules | Where-Object {
                            [IO.Path]::GetFullPath([string]$_.FileName).Equals($resolvedCollector, [StringComparison]::OrdinalIgnoreCase)
                        })
                        if ($loaded.Count -eq 1) { $exactProcess = $candidate; break }
                    } catch { }
                }
                if ($null -eq $exactProcess) { Start-Sleep -Milliseconds 250 }
            }
            if ($null -eq $exactProcess) {
                if ($recorder.HasExited) { throw "A34 $caseId recorder exited before readiness (code $($recorder.ExitCode))." }
                throw "A34 $caseId did not authenticate an exact process within $ProcessTimeoutSeconds seconds."
            }

            # Dispatch immediately once the exact process/collector/window tuple
            # exists. The action driver itself polls the authenticated clock and
            # opens the panel on the first paused target-hour observation, so the
            # collector's one-shot epoch cannot be delayed behind a second
            # readiness sampling pass.
            $actionReceipt = Join-Path $caseRoot 'a34_autonomous_action_receipt.json'
            & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $actionDriver `
                -ProcessId $exactProcess.Id `
                -Hoi4Exe $resolvedExe `
                -CollectorDll $resolvedCollector `
                -ReceiptPath $actionReceipt `
                -CaseId $caseId `
                -SessionId $sessionId `
                -CaptureId $captureId `
                -ExpectedGameHourRaw ([int]$case.expected_game_hour_raw) `
                -ReadinessTimeoutSeconds $ProcessTimeoutSeconds `
                -SettleMilliseconds $ActionSettleMilliseconds
            if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $actionReceipt -PathType Leaf)) {
                throw "A34 $caseId autonomous decision-view action failed."
            }

            $readinessReceipt = Join-Path $caseRoot 'a34_authenticated_paused_save_readiness.json'
            & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $readinessHelper `
                -ProcessId $exactProcess.Id `
                -ReceiptPath $readinessReceipt `
                -ExpectedGameHourRaw ([int]$case.expected_game_hour_raw) `
                -TimeoutSeconds $ProcessTimeoutSeconds `
                -StableSamples 4 `
                -PollMilliseconds 250
            if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $readinessReceipt -PathType Leaf)) {
                throw "A34 $caseId did not authenticate its paused fixture save."
            }

            # Save selection and the read-only action are now complete; restore
            # the user's launcher state while the recorder finalizes.
            if ($continuePriorExists) { Write-AtomicReplaceBytes $continueGamePath $continuePriorBytes }
            elseif (Test-Path -LiteralPath $continueGamePath) { Remove-Item -LiteralPath $continueGamePath -Force }
            $continueOverridden = $false

            if (-not $recorder.WaitForExit([Math]::Max(60000, ($DurationSeconds + 600) * 1000))) {
                throw "A34 $caseId recorder exceeded its bounded finalization deadline."
            }
            if ($recorder.ExitCode -ne 0) { throw "A34 $caseId recorder exited with code $($recorder.ExitCode)." }
            if (-not (Test-Path -LiteralPath $collectorPayload -PathType Leaf) -or
                -not (Test-Path -LiteralPath $collectorReceipt -PathType Leaf)) {
                throw "A34 $caseId collector did not produce its one-shot DPN1 transaction."
            }
            [IO.File]::Move($collectorPayload, (Join-Path $caseRoot 'a34_decision_panel_evidence.dpn1'))
            [IO.File]::Move($collectorReceipt, (Join-Path $caseRoot 'a34_decision_panel_evidence_receipt.json'))
        } finally {
            if ($recorderStarted -and -not $recorder.HasExited) {
                if (-not $recorder.WaitForExit(30000)) { Stop-Process -Id $recorder.Id -Force -ErrorAction SilentlyContinue }
            }
            if ($null -ne $stdoutTask) {
                try { Write-AtomicCreateNewUtf8 $stdoutPath ($stdoutTask.GetAwaiter().GetResult()) } catch { }
            }
            if ($null -ne $stderrTask) {
                try { Write-AtomicCreateNewUtf8 $stderrPath ($stderrTask.GetAwaiter().GetResult()) } catch { }
            }
            if ($null -ne $exactProcess) {
                $live = Get-Process -Id $exactProcess.Id -ErrorAction SilentlyContinue
                if ($null -ne $live) {
                    Stop-Process -Id $live.Id -Force -ErrorAction SilentlyContinue
                    try { $live.WaitForExit(30000) | Out-Null } catch { }
                }
            }
        }
    }
} finally {
    if ($continueOverridden) {
        if ($continuePriorExists) { Write-AtomicReplaceBytes $continueGamePath $continuePriorBytes }
        elseif (Test-Path -LiteralPath $continueGamePath) { Remove-Item -LiteralPath $continueGamePath -Force }
    }
}

$regrade = Join-Path $resolvedOut 'a34_country_decision_materialization_regrade.json'
$immutableReceipt = Join-Path $resolvedOut 'a34_country_decision_materialization_immutable_receipt.json'
& $resolvedPython $verifier `
    --acquisition-root $resolvedOut `
    --fixture-plan $resolvedFixturePlan `
    --catalog-package $catalogPackagePath `
    --collector-dll $resolvedCollector `
    --out $regrade `
    --immutable-receipt $immutableReceipt
if ($LASTEXITCODE -ne 0) { throw "A34 fixture pair was rejected; inspect $regrade" }
Write-Output "A34 autonomous nonempty/measured-empty fixture accepted: $immutableReceipt"
