[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$harnessPath = Join-Path $scriptDir 'Invoke-HeadlessRawCapture.ps1'
$observerPath = Join-Path $scriptDir 'Read-Hoi4GameState.ps1'
$driverPath = Join-Path $scriptDir 'Control-Hoi4Game.ps1'
$optInPath = Join-Path $scriptDir '..\..\..\portable_replay\native\scripts\Hoi4ClientDriving.OptIn.ps1'
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

# EVERYTHING ABOVE THIS LINE IS A TEXT CENSUS, AND A TEXT CENSUS IS WHY RUN t36
# LOST ITS TEARDOWN RECEIPT.
#
# The lifecycle verifier keeps a mirror of the Rust CaptureReadyV2 encoder --
# the field order it appends in, and a rebuild of the authenticated payload --
# so a receipt is admitted by something that did not merely read the recorder's
# own claim back to itself. Lane EH gave the reserved u32 at offset 20 to
# `CollectorFeatureBits` and the mirror stayed behind. Nothing here noticed:
# every string this file counts was still present. t36 found out instead, after
# a 60s window and a 718-hour fold, when the mirror refused the READY receipt,
# took its custody authority down with it, and left the recorder's teardown
# receipt -- 57 of 57 pinned sites restored to baseline -- unauthenticated, the
# graded window open, the settings guard applied and the game running.
#
# So the mirror is now RUN here, against the real contract sources and against
# real receipts, in the gate that costs seconds rather than in the one that
# costs a capture.
. $lifecyclePath

# 1. The mirror against the contract it mirrors. This is the exact assertion
#    t36 failed, and it needs no receipt and no game.
$policyHookAuthority = Assert-PassiveReadySourceContractParity `
    'vanilla_latest_2026_06_30'
Assert-Policy `
    -Condition ([uint16]$policyHookAuthority.Count -gt 0 -and
        [uint16]$policyHookAuthority.ImportSlotCount -gt 0 -and
        [string]$policyHookAuthority.Sha256 -cmatch '^[0-9a-f]{64}$') `
    -Message 'Passive READY source-contract parity returned no hook authority.'
$policyFeatureAuthority = Get-PassiveCollectorFeatureAuthority
Assert-Policy `
    -Condition ([uint32]$policyFeatureAuthority.DeclaredBit -eq 1 -and
        [uint32]$policyFeatureAuthority.KnownMask -ge 1 -and
        @($policyFeatureAuthority.FeatureNames).Count -gt 0) `
    -Message 'Collector feature-bit authority could not be read from the contract.'

# 2. The payload rebuild against receipts it did not produce. The expected
#    digests are the receipts' OWN recorded values, so the only way to satisfy
#    this is to lay the bytes out the way the collector laid them out.
$dialectFixturePath = Join-Path `
    $scriptDir 'PassiveCaptureReady.ReceiptDialects.fixture.v1.json'
Assert-Policy `
    -Condition (Test-Path -LiteralPath $dialectFixturePath -PathType Leaf) `
    -Message "Passive READY receipt-dialect fixture is missing: $dialectFixturePath"
$dialectFixture = Get-Content -LiteralPath $dialectFixturePath -Raw |
    ConvertFrom-Json
Assert-Policy `
    -Condition ([string]$dialectFixture.schema -ceq
        'hoi4-passive-capture-ready-receipt-dialects-fixture-v1') `
    -Message 'Passive READY receipt-dialect fixture carries no schema.'
$dialectCases = @($dialectFixture.cases)
Assert-Policy `
    -Condition ($dialectCases.Count -ge 3) `
    -Message 'Passive READY receipt-dialect fixture covers fewer than three dialects.'
$dialectDeclaredSeen = $false
$dialectUndeclaredSeen = $false
$dialectWords = [Collections.Generic.List[uint32]]::new()
$dialectPinnedDigests = [Collections.Generic.List[string]]::new()
foreach ($case in $dialectCases) {
    $caseName = [string]$case.name
    $receipt = $case.receipt

    $declaredProperties = @(
        Get-PassiveCaptureReadyDeclaredProperties -Ready $receipt
    )
    if ([bool]$case.expects_declaration) {
        $dialectDeclaredSeen = $true
        Assert-Policy `
            -Condition ($declaredProperties.Count -eq 2) `
            -Message "Passive READY dialect '$caseName' was not recognised as a declaring receipt."
    } else {
        $dialectUndeclaredSeen = $true
        Assert-Policy `
            -Condition ($declaredProperties.Count -eq 0) `
            -Message "Passive READY dialect '$caseName' was not recognised as a pre-declaration receipt."
    }

    # The closed property set still has to be closed, in whichever dialect.
    $expectedProperties = @(
        'schema', 'status', 'capture_policy', 'pid',
        'process_creation_filetime_utc', 'session_id', 'capture_id',
        'requested_duration_ms', 'readiness_flags', 'validated_domain_count',
        'scheduler_queue_capacity', 'scheduler_worker_count',
        'allowed_hook_count', 'active_hook_count', 'clock_hour_raw',
        'clock_observation_ordinal', 'target_executable_sha256',
        'collector_dll_sha256',
        'hook_allowlist_sha256', 'hook_outcomes_sha256',
        'validated_domain_roster_sha256', 'clock_witness_sha256',
        'scheduler_config_sha256', 'output_pipe_identity_sha256',
        'zero_mutation_policy_sha256', 'session_id_sha256',
        'capture_id_sha256', 'control_sha256', 'pipe_token_sha256',
        'ready_payload_sha256', 'host_binding_sha256', 'host_validated',
        'public_receipt_core_sha256', 'ready_ack_sha256'
    ) + $declaredProperties
    Assert-PassiveExactPropertySet `
        -Value $receipt `
        -Expected $expectedProperties `
        -Label "Passive READY dialect '$caseName'"

    $declarationFailures = @(
        Get-PassiveCollectorBuildDeclarationFailures `
            -Ready $receipt -FeatureAuthority $policyFeatureAuthority
    )
    Assert-Policy `
        -Condition ($declarationFailures.Count -eq 0) `
        -Message "Passive READY dialect '$caseName' build declaration is not self-consistent: $($declarationFailures -join '; ')"

    if ($null -ne $case.expected_ready_payload_sha256) {
        $observedPayloadSha = Get-PassiveCaptureReadyPayloadSha256 `
            -Ready $receipt `
            -PipeToken ([string]$dialectFixture.pipe_token)
        Assert-Policy `
            -Condition ($observedPayloadSha -ceq
                [string]$case.expected_ready_payload_sha256) `
            -Message "Passive READY dialect '$caseName' payload rebuild is $observedPayloadSha, not $($case.expected_ready_payload_sha256)."
        $dialectWords.Add((Get-PassiveCaptureReadyFeatureBits -Ready $receipt))
        $dialectPinnedDigests.Add([string]$case.expected_ready_payload_sha256)
    }
    if ($null -ne $case.expected_ready_ack_sha256) {
        $observedAckSha = Get-PassiveCaptureReadyAckSha256 `
            -Ready $receipt `
            -PipeToken ([string]$dialectFixture.pipe_token)
        Assert-Policy `
            -Condition ($observedAckSha -ceq
                [string]$case.expected_ready_ack_sha256) `
            -Message "Passive READY dialect '$caseName' ACK rebuild is $observedAckSha, not $($case.expected_ready_ack_sha256)."
    }
}
Assert-Policy `
    -Condition ($dialectDeclaredSeen -and $dialectUndeclaredSeen) `
    -Message 'Passive READY receipt-dialect fixture does not exercise both dialects.'

# 3. THE PINNED CASES MUST BE ABLE TO DISAGREE.
#
#    Two dialects and two digests are not enough on their own. While the only
#    nonzero word in the fixture was 1, an encoder that wrote a constant 1
#    whenever the declaration property was present -- and a zero otherwise --
#    reproduced every pinned digest while being wrong for every other feature
#    word. So the fixture has to pin at least three DISTINCT words, and the
#    digests they produce have to be as distinct as the words are. Both sides
#    are derived from what the loop actually rebuilt rather than restated, so a
#    fixture that quietly loses a case fails here instead of going quiet.
$distinctWords = @($dialectWords | Sort-Object -Unique)
$distinctDigests = @($dialectPinnedDigests | Sort-Object -Unique)
Assert-Policy `
    -Condition ($distinctWords.Count -ge 3) `
    -Message "Passive READY payload rebuild is pinned at only $($distinctWords.Count) distinct feature word(s); an encoder ignoring the word's value would still pass."
Assert-Policy `
    -Condition ($distinctWords -contains 0) `
    -Message 'Passive READY payload rebuild is not pinned at the undeclared (zero) feature word.'
Assert-Policy `
    -Condition ($distinctDigests.Count -eq $distinctWords.Count) `
    -Message "Passive READY payload rebuild produced $($distinctDigests.Count) distinct digests for $($distinctWords.Count) distinct feature words; the word is not reaching the payload."

#    And the two dialects must not be told apart by the digest: a receipt
#    carrying the declaration pair with a ZERO word has to encode exactly what
#    a pre-declaration receipt encodes, because that is what an undeclaring
#    collector signs today.
$zeroDigests = @(
    for ($index = 0; $index -lt $dialectWords.Count; $index++) {
        if ([uint32]$dialectWords[$index] -eq 0) { $dialectPinnedDigests[$index] }
    }
)
Assert-Policy `
    -Condition ($zeroDigests.Count -ge 2 -and
        @($zeroDigests | Sort-Object -Unique).Count -eq 1) `
    -Message 'A declared-but-zero feature word does not encode identically to a pre-declaration receipt.'

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
