[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$harnessPath = Join-Path $scriptDir 'Invoke-HeadlessRawCapture.ps1'
$observerPath = Join-Path $scriptDir 'Read-Hoi4GameState.ps1'
$driverPath = Join-Path $scriptDir 'Control-Hoi4Game.ps1'
$optInPath = Join-Path $scriptDir 'Hoi4ClientDriving.OptIn.ps1'
$productionPath = Join-Path $scriptDir 'Start-Hoi4ProductionCaptureSafe.ps1'
$lifecyclePath = Join-Path $scriptDir 'PassiveCapture.Lifecycle.ps1'

$harness = Get-Content -LiteralPath $harnessPath -Raw
$observer = Get-Content -LiteralPath $observerPath -Raw
$driver = Get-Content -LiteralPath $driverPath -Raw
$optIn = Get-Content -LiteralPath $optInPath -Raw
$production = Get-Content -LiteralPath $productionPath -Raw
$lifecycle = Get-Content -LiteralPath $lifecyclePath -Raw

function Assert-Policy {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message
    )
    if (-not $Condition) {
        throw $Message
    }
}

Assert-Policy `
    -Condition ($harness -notmatch '(?m)^\s*''--hour-latch'',?\s*$') `
    -Message 'Passive harness still passes --hour-latch to collector_record.'
Assert-Policy `
    -Condition ($harness -match '\$reuseRunningGame\s*=\s*-not\s+\$EvidenceOnlyDriveGame') `
    -Message 'Passive reuse is not the default harness mode.'
Assert-Policy `
    -Condition ($harness -match 'if\s*\(\$Speed\s+-ne\s+5\)\s*\{\s*throw') `
    -Message 'Every graded harness mode does not hard-require speed 5.'
Assert-Policy `
    -Condition ($harness -match '''--read-only-scheduler-poll-ms''') `
    -Message 'Passive harness does not pass an explicit scheduler poll period.'
Assert-Policy `
    -Condition ($harness -match '''--read-only-scheduler-queue-cap''') `
    -Message 'Passive harness does not pass an explicit scheduler queue capacity.'
Assert-Policy `
    -Condition ($harness -match 'Assert-ExactJsonPropertySet') `
    -Message 'Typed READY/teardown receipts are not admitted with a closed property set.'
Assert-Policy `
    -Condition ($harness -match 'Test-PassiveCaptureReadyReceipt' -and
        $harness -match 'Test-PassiveCaptureTeardownReceipt') `
    -Message 'Harness does not bind READY and teardown through the shared lifecycle verifier.'
Assert-Policy `
    -Condition ($harness -match '''--read-only-scheduler''') `
    -Message 'Passive harness does not explicitly enable the read-only scheduler.'
Assert-Policy `
    -Condition ($harness -match '\[ValidateRange\(1,\s*1\)\]\s*\[int\]\$SchedulerQueueCapacity\s*=\s*1') `
    -Message 'Direct passive harness does not require scheduler queue capacity 1.'
Assert-Policy `
    -Condition ($harness -match '\$sessionId\s*=\s*\[guid\]::NewGuid\(\)\.ToString\(''D''\)\.ToLowerInvariant\(\)') `
    -Message 'Direct passive harness does not generate a canonical lowercase UUID session.'

$gradedStart = $harness.IndexOf(
    '$gradedWindowStarted = $true',
    [StringComparison]::Ordinal
)
$gradedEnd = $harness.LastIndexOf(
    '$gradedWindowStarted = $false',
    [StringComparison]::Ordinal
)
Assert-Policy `
    -Condition ($gradedStart -ge 0 -and $gradedEnd -gt $gradedStart) `
    -Message 'Could not identify the exact graded attach-through-teardown region.'
$gradedRegion = $harness.Substring(
    $gradedStart,
    $gradedEnd - $gradedStart
)
foreach ($forbidden in @(
    '-Action Play',
    '-Action Pause',
    '-RequestedSpeed',
    'Invoke-BoundedWmiLaunch',
    'CloseMainWindow',
    'Stop-ExactProcess -Process $launchedProcess',
    'Control-Hoi4Game.ps1'
)) {
    Assert-Policy `
        -Condition ($gradedRegion.IndexOf(
            $forbidden,
            [StringComparison]::Ordinal
        ) -lt 0) `
        -Message "Graded capture region contains client-driving token '$forbidden'."
}

foreach ($forbidden in @(
    'WriteProcessMemory',
    'PROCESS_VM_WRITE',
    'PROCESS_VM_OPERATION',
    'ShowWindow',
    'SendInput',
    'SetForegroundWindow'
)) {
    Assert-Policy `
        -Condition ($observer.IndexOf(
            $forbidden,
            [StringComparison]::OrdinalIgnoreCase
        ) -lt 0) `
        -Message "Read-only observer contains forbidden capability '$forbidden'."
}
Assert-Policy `
    -Condition ($observer -match '\$desiredAccess\s*=\s*\$processQueryInformation\s*-bor\s*\$processVmRead') `
    -Message 'Read-only observer does not request exactly QUERY|VM_READ.'
Assert-Policy `
    -Condition ($observer -match 'Action\s*=\s*''Status''') `
    -Message 'Read-only observer does not identify its output as a status observation.'

$driverOptIn = $driver.IndexOf(
    'Assert-Hoi4ClientDrivingOptIn',
    [StringComparison]::Ordinal
)
$driverOpen = $driver.IndexOf(
    '[Hoi4HeadlessControlNative]::OpenProcess',
    [StringComparison]::Ordinal
)
Assert-Policy `
    -Condition ($driverOptIn -ge 0 -and $driverOpen -gt $driverOptIn) `
    -Message 'Driving helper can open the game before evidence opt-in is asserted.'

foreach ($requiredInput in @(
    'extractor/native/scripts/Start-Hoi4EarlyLaunchCaptureSafe.ps1',
    'extractor/native/scripts/Invoke-HeadlessRawCapture.ps1',
    'extractor/native/scripts/Invoke-HeadlessRawCaptureUntilClean.ps1',
    'extractor/native/scripts/Measure-InjectionCrashRate.ps1',
    'extractor/native/scripts/Control-Hoi4Game.ps1'
)) {
    Assert-Policy `
        -Condition ($optIn.IndexOf(
            "'$requiredInput'",
            [StringComparison]::Ordinal
        ) -ge 0) `
        -Message "Client-driving exclusion roster is missing '$requiredInput'."
}
Assert-Policy `
    -Condition ($optIn.IndexOf(
        "'extractor/native/scripts/Start-Hoi4ProductionCaptureSafe.ps1'",
        [StringComparison]::Ordinal
    ) -lt 0) `
    -Message 'Passive production driver was incorrectly placed in the evidence-only exclusion roster.'

foreach ($forbidden in @(
    '--hour-latch',
    'Control-Hoi4Game.ps1',
    'WriteProcessMemory',
    'PROCESS_VM_WRITE',
    'PROCESS_VM_OPERATION',
    'CloseMainWindow',
    'Stop-Process',
    '.Kill(',
    'AssignProcessToJobObject',
    'SetInformationJobObject',
    'TerminateProcess',
    'PostMessageW',
    'PostThreadMessageW',
    'SetWindowsHookExW',
    'UnhookWindowsHookEx'
)) {
    Assert-Policy `
        -Condition ($production.IndexOf(
            $forbidden,
            [StringComparison]::OrdinalIgnoreCase
        ) -lt 0) `
        -Message "Passive production driver contains forbidden client-driving token '$forbidden'."
}
foreach ($required in @(
    'PassiveCapture.Lifecycle.ps1',
    'Test-PassiveCaptureReadyReceipt',
    'Test-PassiveCaptureTeardownReceipt',
    'CAPTURE_READY_V2.json',
    'CAPTURE_TEARDOWN_V1.json',
    '"--read-only-scheduler"',
    '"--read-only-scheduler-poll-ms"',
    '"--read-only-scheduler-queue-cap"',
    'ReadOnlySchedulerQueueCap -ne 1',
    'Get-LiveSample $handle $base "teardown_wait"',
    'Get-LiveSample $handle $base "post_teardown"',
    'post-teardown read-only witness detected a speed/pause violation',
    'post_injection_termination_forbidden',
    'production_domain_roster_authority.json',
    'domain_roster_authority_sha256'
)) {
    Assert-Policy `
        -Condition ($production.IndexOf(
            $required,
            [StringComparison]::Ordinal
        ) -ge 0) `
        -Message "Passive production driver lacks required lifecycle/policy token '$required'."
}
$childExitIndex = $production.IndexOf(
    'Update-AttemptMarker $attemptMarkerPath $attemptMarker "child_exited"',
    [StringComparison]::Ordinal
)
$teardownWaitSampleIndex = $production.IndexOf(
    'Get-LiveSample $handle $base "teardown_wait"',
    [StringComparison]::Ordinal
)
$teardownAuthenticationIndex = $production.IndexOf(
    '$teardownAuthority = Test-PassiveCaptureTeardownReceipt',
    $teardownWaitSampleIndex,
    [StringComparison]::Ordinal
)
$postTeardownSampleIndex = $production.IndexOf(
    'Get-LiveSample $handle $base "post_teardown"',
    [StringComparison]::Ordinal
)
Assert-Policy `
    -Condition ($childExitIndex -ge 0 -and
        $teardownWaitSampleIndex -gt $childExitIndex -and
        $teardownAuthenticationIndex -gt $teardownWaitSampleIndex -and
        $postTeardownSampleIndex -gt $teardownAuthenticationIndex) `
    -Message 'Production speed/pause witness does not span child exit through authenticated teardown.'
Assert-Policy `
    -Condition ($production -match 'phase\s*=\s*\$Phase') `
    -Message 'Production speed/pause monitor rows do not identify their lifecycle phase.'
# FUNCTIONS ARE CHECKED AS DEFINITIONS, NOT AS SUBSTRINGS. The `$required`
# list below is a substring census, and a substring census cannot tell a
# function's DEFINITION from a CALL to it. Every name here is also called
# inside the same file (`Get-PassiveCaptureReadyPayloadSha256` is defined at
# ~line 540 and called at ~line 1023, and so on), so deleting or misspelling a
# definition while leaving one call standing satisfied the substring gate --
# the gate would have gone green on a lifecycle that cannot run. That is the
# same class of miss that left this gate red for a week (see 8a7b0e9f), only
# inverted, so it is checked with the parser rather than with `IndexOf`.
$lifecycleParseErrors = $null
$lifecycleTokens = $null
$lifecycleAst = [System.Management.Automation.Language.Parser]::ParseFile(
    $lifecyclePath,
    [ref]$lifecycleTokens,
    [ref]$lifecycleParseErrors
)
Assert-Policy `
    -Condition ($null -ne $lifecycleAst -and $lifecycleParseErrors.Count -eq 0) `
    -Message "Shared passive lifecycle verifier does not parse: $(($lifecycleParseErrors | ForEach-Object { $_.Message }) -join '; ')"
$lifecycleDefinedFunctions = @(
    $lifecycleAst.FindAll(
        { param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] },
        $true
    ) | ForEach-Object { $_.Name }
)
foreach ($requiredFunction in @(
    'Get-PassivePinnedHookAllowlistAuthorityFromSource',
    'Get-PassiveCaptureReadyPayloadSha256',
    'Get-PassiveCaptureReadyAckSha256',
    'Assert-PassiveReadySourceContractParity',
    'Assert-PassiveExactPropertySet'
)) {
    Assert-Policy `
        -Condition ($lifecycleDefinedFunctions -ccontains $requiredFunction) `
        -Message "Shared passive lifecycle verifier does not DEFINE function '$requiredFunction' (a call site alone is not a definition)."
}
foreach ($required in @(
    'hoi4-capture-ready-v2',
    'hoi4-capture-teardown-v2',
    'Get-PassivePinnedHookAllowlistAuthorityFromSource',
    '$allowedHookCount -ne [uint32]$hookAuthority.Count',
    'ExpectedHookCount',
    '$pinnedHookSiteCount -ne $expectedHookCount',
    '(?:^|\.)a(?:25|28|30)(?:\.|$)',
    'data_pipe_eof_observed',
    'hook_lease_released_after_eof',
    'collector_module_disposition',
    'target_process_epoch_revalidated',
    'target_code_temporarily_instrumented',
    'pinned_hook_target_code_restored',
    'semantic_game_state_mutated',
    'pinned_hook_sites_restored',
    'pinned_hook_site_count',
    'pinned_hook_site_baseline_sha256',
    'pinned_hook_site_restored_sha256',
    'clock_observation_ordinal',
    'battle-plan.capture_ready.scheduler_config.v1',
    'battle-plan.capture_ready.read_only_clock_witness.v1',
    'battle-plan.capture_ready.output_pipe_identity.v1',
    'battle-plan.capture_ready.zero_mutation_policy.v1',
    'PRODUCTION_DOMAIN_QUEUE_CAPACITY_V1: u32 = 1',
    'PRODUCTION_OUTPUT_QUEUE_CAPACITY_V1: u32 = 2',
    'PRODUCTION_DOMAIN_TIMEOUT_US_V1: u32 = 0',
    'PRODUCTION_PROPOSAL_POOL_CAPACITY_BYTES_V1: u64 = 256 * 1024 * 1024',
    'Assert-PassiveReadySourceContractParity',
    'CAPTURE_READY_MAGIC',
    'CAPTURE_READY_ACK_MAGIC',
    'CAPTURE_READY_LEN',
    'CAPTURE_READY_ACK_LEN',
    'Get-PassiveCaptureReadyPayloadSha256',
    'Get-PassiveCaptureReadyAckSha256',
    'Assert-PassiveExactPropertySet'
)) {
    Assert-Policy `
        -Condition ($lifecycle.IndexOf(
            $required,
            [StringComparison]::Ordinal
        ) -ge 0) `
        -Message "Shared passive lifecycle verifier lacks '$required'."
}
foreach ($retired in @(
    'clock_tick',
    'hoi4.collector.production.zero-client-mutation.v1'
)) {
    Assert-Policy `
        -Condition ($lifecycle.IndexOf(
            $retired,
            [StringComparison]::Ordinal
        ) -lt 0) `
        -Message "Shared passive lifecycle verifier still contains retired READY dialect '$retired'."
}

# EMITTED AS JSON, NOT AS AN OBJECT, AND THAT IS LOAD-BEARING. Every consumer
# runs this file in a CHILD `powershell.exe -File` and pipes the child's stdout
# through `ConvertFrom-Json` (Invoke-CentralRustValidation.ps1, both the
# every-mode gate and the Final/Release gate). A bare `[pscustomobject]` reaching
# a child process's stdout is rendered by the default FORMATTER -- `Schema     :
# hoi4-passive-capture-policy-test-v1` and so on -- which `ConvertFrom-Json`
# rejects with `Invalid JSON primitive: Schema.`. Both gates therefore threw
# "returned no authority" on a PASSING policy run: the authority object was
# produced, formatted, and then unparseable. Serialising here is the single
# point that makes every `ConvertFrom-Json` consumer read what this file
# actually asserts.
([pscustomobject]@{
    Schema = 'hoi4-passive-capture-policy-test-v1'
    Status = 'pass'
    Harness = $harnessPath
    Observer = $observerPath
    Driver = $driverPath
    Production = $productionPath
    Lifecycle = $lifecyclePath
} | ConvertTo-Json -Depth 4 -Compress)
