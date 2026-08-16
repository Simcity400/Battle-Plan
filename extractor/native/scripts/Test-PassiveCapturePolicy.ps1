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
    # Offset 50's reader, which is also where a receipt that omits the field is
    # refused by name. Deleting it and inlining a constant is exactly how the
    # defect it closes was introduced.
    'Get-PassiveCaptureReadyInstallFreezeCount',
    # The ordered shape admission. It exists so the ORDER of the two checks is
    # testable at all; inlining it back into the caller is the regression.
    'Assert-PassiveCaptureReadyReceiptShape',
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
$dialectFreezeCounts = [Collections.Generic.List[uint16]]::new()
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
        'allowed_hook_count', 'active_hook_count', 'install_freeze_count',
        'clock_hour_raw',
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
        $dialectFreezeCounts.Add(
            (Get-PassiveCaptureReadyInstallFreezeCount -Ready $receipt)
        )
        # THE REBUILT DIGEST, not the fixture constant. The assertion above
        # makes the two equal, so recording the constant looked equivalent --
        # but the cardinality checks below are statements about what the REBUILD
        # produced, and a rebuild that collapsed two inputs onto one digest
        # would have gone on failing that assertion while the cardinality check
        # counted fixture text and stayed green about a different question.
        $dialectPinnedDigests.Add($observedPayloadSha)
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

#    THE SAME ARGUMENT, FOR THE FREEZE COUNT AT OFFSET 50, and it is not a
#    symmetry exercise: offset 50 stopped being reserved-must-be-zero, this
#    rebuild kept appending that zero, and because installing an inline detour
#    freezes the process, every capture that installs one was refused on
#    `ready_payload_sha256` alone until the count was published on the receipt
#    and read here (runs t57-t60 on C:, t62 and t63 on L:, all reporting 13 --
#    `runs/lp_refusal_volume/EVIDENCE.md`). Nothing caught
#    it because every pinned case at the time predated the field and genuinely
#    authenticated a zero there. Two DISTINCT nonzero counts are required for the
#    reason a constant 1 was not enough above, and 1000 additionally does not fit
#    in a byte.
$distinctFreezeCounts = @($dialectFreezeCounts | Sort-Object -Unique)
$nonzeroFreezeCounts = @($distinctFreezeCounts | Where-Object { [uint16]$_ -ne 0 })
Assert-Policy `
    -Condition ($distinctFreezeCounts -contains 0) `
    -Message 'Passive READY payload rebuild is not pinned at a zero install-freeze count, which is what every pre-offset-50 receipt authenticated.'
Assert-Policy `
    -Condition ($nonzeroFreezeCounts.Count -ge 2) `
    -Message "Passive READY payload rebuild is pinned at only $($nonzeroFreezeCounts.Count) distinct nonzero install-freeze count(s); a rebuild that hardcoded one value -- exactly as the broken one hardcoded zero -- would still pass."
Assert-Policy `
    -Condition (@($nonzeroFreezeCounts | Where-Object { [uint16]$_ -gt 255 }).Count -ge 1) `
    -Message 'Passive READY payload rebuild is not pinned at an install-freeze count above 255; a rebuild appending one byte and a pad would still pass.'

#    The digest has to move with BOTH words. Counting distinct digests against
#    distinct feature words alone stopped being the right test the moment a
#    second field could vary: the pair is what the payload distinguishes, so the
#    pair is what is counted. A rebuild that dropped either field back to a
#    constant collapses two pairs onto one digest and fails here.
$dialectPairs = @(
    for ($index = 0; $index -lt $dialectWords.Count; $index++) {
        '{0}/{1}' -f [uint32]$dialectWords[$index], [uint16]$dialectFreezeCounts[$index]
    }
)
$distinctPairs = @($dialectPairs | Sort-Object -Unique)
Assert-Policy `
    -Condition ($distinctDigests.Count -eq $distinctPairs.Count) `
    -Message "Passive READY payload rebuild produced $($distinctDigests.Count) distinct digests for $($distinctPairs.Count) distinct (feature word, install-freeze count) pairs; one of the two fields is not reaching the payload."

#    And the two dialects must not be told apart by the digest: a receipt
#    carrying the declaration pair with a ZERO word has to encode exactly what
#    a pre-declaration receipt encodes, because that is what an undeclaring
#    collector signs today.
#    The freeze count has to be held equal here too, or this stops being a
#    statement about the feature word: two receipts with a zero word and
#    different counts encode differently and should.
$zeroDigests = @(
    for ($index = 0; $index -lt $dialectWords.Count; $index++) {
        if ([uint32]$dialectWords[$index] -eq 0 -and
            [uint16]$dialectFreezeCounts[$index] -eq 0) {
            $dialectPinnedDigests[$index]
        }
    }
)
Assert-Policy `
    -Condition ($zeroDigests.Count -ge 2 -and
        @($zeroDigests | Sort-Object -Unique).Count -eq 1) `
    -Message 'A declared-but-zero feature word does not encode identically to a pre-declaration receipt.'

# 4. EVERY MIRROR-VERSUS-PRODUCER REFUSAL IS NAMED (issue #151).
#
#    Section 1 above proves the mirror currently AGREES with the contract. This
#    section proves the other half: that when it stops agreeing, the harness is
#    told WHAT stopped agreeing rather than handed a sentence.
#
#    t36 is the whole case. The mirror refused a READY receipt because the
#    reserved u32 at wire offset 20 had become the collector's feature word and
#    this file still expected `&0u32`. The refusal threw before either READY
#    authority was published, so the recorder's teardown record -- 57 of 57
#    pinned sites restored to baseline -- could not be admitted, the graded
#    window never closed, the settings guard stayed applied, and the game was left
#    running. Nothing on disk said "wire field order, offset 20"; that sentence
#    existed only in a transcript, filed under a different heading.
#
#    a) The parity and feature-dialect functions may not throw an ANONYMOUS
#       failure. Checked with the parser rather than by string search, because
#       the thing being forbidden is a syntactic form -- `throw '<sentence>'` --
#       and a substring census cannot tell one throw from another. Adding a bare
#       `throw` to any of these functions turns this red, which is exactly the
#       regression #151 describes.
#       Get-PassiveCollectorFeatureAuthority is deliberately absent: it is a
#       two-line delegator that refuses nothing itself, and both functions it
#       calls are censused here.
$namedRefusalFunctions = @(
    'Assert-PassiveReadySourceContractParity',
    'Assert-PassiveProducerFragment',
    'Get-PassiveControlTransportPayloadSource',
    'Get-PassiveCollectorFeatureAuthorityFromSource'
)
foreach ($functionName in $namedRefusalFunctions) {
    $definition = @(
        $lifecycleAst.FindAll(
            {
                param($node)
                $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
                $node.Name -ceq $functionName
            },
            $true
        )
    )
    Assert-Policy `
        -Condition ($definition.Count -eq 1) `
        -Message "Expected exactly one definition of '$functionName'; found $($definition.Count)."
    # Recursive on purpose: Get-PassiveCollectorFeatureMask is defined INSIDE
    # Get-PassiveCollectorFeatureAuthorityFromSource, and its mask refusals are
    # the ones a drifting KNOWN_MASK produces.
    $throws = @(
        $definition[0].FindAll(
            {
                param($node)
                $node -is [System.Management.Automation.Language.ThrowStatementAst]
            },
            $true
        )
    )
    Assert-Policy `
        -Condition ($throws.Count -gt 0) `
        -Message "'$functionName' no longer refuses anything; its parity checks are gone."
    foreach ($throwNode in $throws) {
        # A bare `throw` (rethrow inside a catch) has no pipeline and keeps
        # whatever structure it was already carrying, so it is admitted.
        if ($null -eq $throwNode.Pipeline) { continue }
        $throwText = [string]$throwNode.Pipeline.Extent.Text
        Assert-Policy `
            -Condition ($throwText.Contains('New-PassiveReadyContractMismatch')) `
            -Message ("'$functionName' throws an unnamed refusal at line " +
                "$($throwNode.Extent.StartLineNumber): $throwText. Every " +
                'mirror-versus-producer refusal has to carry a class, a field ' +
                'and both sides of the comparison, or the harness cannot name ' +
                'it in CAPTURE_REFUSAL_V1.json and issue #151 is back.')
    }
}

#    b) The structured refusal SURVIVES the throw. Everything above is worth
#       nothing if the detail is lost on the way out, and PowerShell 5.1's
#       `throw <ErrorRecord>` is the mechanism the whole design rests on.
$survivedMismatch = $null
try {
    throw (New-PassiveReadyContractMismatch `
        -Class 'wire_field_order' `
        -Field 'out.extend_from_slice(&self.collector_features.bits().to_le_bytes());' `
        -Expected "appended after 'out.extend_from_slice(&self.pid.to_le_bytes());'" `
        -Observed 'the encoder no longer appends this field at all' `
        -Message 'probe')
} catch {
    $survivedMismatch = Get-PassiveReadyContractMismatch -ErrorRecord $_
}
Assert-Policy `
    -Condition ($null -ne $survivedMismatch -and
        [string]$survivedMismatch.class -ceq 'wire_field_order' -and
        [string]$survivedMismatch.field -clike '*collector_features*' -and
        [string]$survivedMismatch.observed -clike '*no longer appends*') `
    -Message 'A structured parity mismatch does not survive being thrown; the harness would see only a sentence.'
# ...and an UNSTRUCTURED throw must not be mistaken for one. PowerShell sets
# TargetObject to the thrown string for `throw '<text>'`, so a reader that only
# null-checks would read that string as a mismatch record.
$plainMismatch = 'unset'
try { throw 'an ordinary sentence' } catch {
    $plainMismatch = Get-PassiveReadyContractMismatch -ErrorRecord $_
}
Assert-Policy `
    -Condition ($null -eq $plainMismatch) `
    -Message 'A plain string throw is being read as a structured parity mismatch.'

#    c) END TO END on the t36 defect class, against the REAL producer source.
#       Section 1 runs the mirror against the contract as it is; this runs it
#       against the contract as it would be if it drifted, by mutating the one
#       declaration out of a copy of the real payloads.rs. Nothing is written to
#       disk and the real file is untouched -- the function takes its source as
#       a parameter precisely so this is possible.
$payloadSource = Get-PassiveControlTransportPayloadSource
Assert-Policy `
    -Condition ($payloadSource.Contains('pub const KNOWN_MASK: u32')) `
    -Message 'The collector feature authority no longer declares KNOWN_MASK; this drift probe is testing nothing.'
$driftedPayload = $payloadSource -replace `
    'pub const KNOWN_MASK: u32', 'pub const KNOWN_MASK_RENAMED: u32'
$driftRefusal = $null
try {
    [void](Get-PassiveCollectorFeatureAuthorityFromSource $driftedPayload)
} catch {
    $driftRefusal = New-PassiveReadyRefusalRecord `
        -ErrorRecord $_ `
        -CustodyEstablished $false `
        -Stage 'ready_validation'
}
Assert-Policy `
    -Condition ($null -ne $driftRefusal) `
    -Message 'A collector feature dialect that dropped KNOWN_MASK was ACCEPTED by the mirror.'
Assert-Policy `
    -Condition ([string]$driftRefusal.class -ceq 'collector_feature_dialect') `
    -Message "A drifted collector feature dialect was classified '$($driftRefusal.class)', not 'collector_feature_dialect'."
Assert-Policy `
    -Condition (@($driftRefusal.clauses).Count -eq 1 -and
        [string]@($driftRefusal.clauses)[0].field -ceq 'KNOWN_MASK') `
    -Message "A drifted collector feature dialect did not name KNOWN_MASK as the field: $(@($driftRefusal.clauses) | ForEach-Object { $_.field })"

#    c2) A RECEIPT WITHOUT `install_freeze_count` IS REFUSED BY NAME.
#
#        This is the shape a recorder_host built before CaptureReadyV2 offset 50
#        writes while a collector built after it puts a real count on the wire,
#        and it is the shape the rebuild used to swallow: it read the missing
#        property as zero, produced a digest the collector could not have
#        produced, and refused six captures with nothing to look at but two
#        64-character hashes. What is tested here is that the refusal happens,
#        that it names the field and says what to do, AND that it is what the
#        admission reaches FIRST. That last part is why the two calls live
#        together in Assert-PassiveCaptureReadyReceiptShape: the closed-property
#        -set comparison would otherwise get there first and answer "has 34
#        properties; expected exactly 35", which is true and useless. Probing
#        only the rebuild would leave that ordering free to be swapped back, so
#        both entry points are probed. (Codex review
#        01a0092c-a860-76a3-a266-e34c5a6cff02, round 2 finding 2.)
$freezelessReceipt = $dialectCases[0].receipt |
    ConvertTo-Json -Depth 8 -Compress |
    ConvertFrom-Json
$freezelessReceipt.PSObject.Properties.Remove('install_freeze_count')
Assert-Policy `
    -Condition ($null -eq $freezelessReceipt.PSObject.Properties['install_freeze_count']) `
    -Message 'The freeze-count probe did not actually remove the property, so it tests nothing.'
# The roster the live admission builds for this receipt's dialect, so the
# helper below is handed what production hands it.
$freezelessExpected = @($expectedProperties)
Assert-Policy `
    -Condition ($freezelessExpected -ccontains 'install_freeze_count') `
    -Message 'The CaptureReadyV2 roster no longer requires install_freeze_count, so the ordering probe below proves nothing.'
$freezeProbes = @(
    @{ Name = 'the payload rebuild'; Action = {
            [void](Get-PassiveCaptureReadyPayloadSha256 `
                -Ready $freezelessReceipt `
                -PipeToken ([string]$dialectFixture.pipe_token))
        } },
    @{ Name = 'the receipt-shape admission'; Action = {
            Assert-PassiveCaptureReadyReceiptShape `
                -Ready $freezelessReceipt `
                -ExpectedProperties $freezelessExpected `
                -Label 'CaptureReadyV2 receipt'
        } }
)
foreach ($freezeProbe in $freezeProbes) {
$freezeProbeName = [string]$freezeProbe.Name
$freezeRefusal = $null
try {
    & $freezeProbe.Action
} catch {
    $freezeRefusal = New-PassiveReadyRefusalRecord `
        -ErrorRecord $_ `
        -CustodyEstablished $false `
        -Stage 'ready_validation'
}
Assert-Policy `
    -Condition ($null -ne $freezeRefusal) `
    -Message "A CaptureReadyV2 receipt with no install_freeze_count was admitted by $freezeProbeName; offset 50 is being assumed rather than read."
Assert-Policy `
    -Condition ([string]$freezeRefusal.class -ceq 'producer_skew') `
    -Message "A receipt missing install_freeze_count was classified '$($freezeRefusal.class)' by $freezeProbeName, not 'producer_skew'."
Assert-Policy `
    -Condition (@($freezeRefusal.clauses).Count -eq 1 -and
        [string]@($freezeRefusal.clauses)[0].field -ceq 'install_freeze_count') `
    -Message "A receipt missing install_freeze_count was not named by $freezeProbeName : $(@($freezeRefusal.clauses) | ForEach-Object { $_.field })"
Assert-Policy `
    -Condition ([string]$freezeRefusal.message -clike '*Rebuild recorder_host and collector from the same tree.*') `
    -Message "A receipt missing install_freeze_count was refused by $freezeProbeName without telling the operator what to do: $($freezeRefusal.message)"
}

#        And the shape admission still refuses a property set that is wrong for
#        a reason that is NOT the freeze count, so putting the named read first
#        did not demote the closed set to a formality.
$widenedReceipt = $dialectCases[0].receipt |
    ConvertTo-Json -Depth 8 -Compress |
    ConvertFrom-Json
$widenedReceipt | Add-Member -NotePropertyName 'unheard_of_field' -NotePropertyValue 1
$widenedRefused = $false
try {
    Assert-PassiveCaptureReadyReceiptShape `
        -Ready $widenedReceipt `
        -ExpectedProperties $freezelessExpected `
        -Label 'CaptureReadyV2 receipt'
} catch {
    $widenedRefused = $true
}
Assert-Policy `
    -Condition $widenedRefused `
    -Message 'A CaptureReadyV2 receipt carrying an unknown property was admitted by the receipt-shape admission.'

#    d) The clause-accumulator refusal keeps its per-clause structure, and the
#       CUSTODY/GRADING split survives into the refusal record. That split is
#       what decides whether the recorder's teardown record can still be
#       believed, so losing it in the refusal is losing the t20 fix.
$custodyProbe = [Collections.Generic.List[string]]::new()
$custodyProbe.Add("pid (expected='4242' actual='7')")
$gradingProbe = [Collections.Generic.List[string]]::new()
$gradingProbe.Add("active_hook_count (expected='55' actual='51')")
$gradingProbe.Add('readiness_flags')
$admissionRefusal = $null
try {
    throw (New-PassiveReadyAdmissionRefusal `
        -Message 'CaptureReadyV2 failed passive admission on 3 clause(s): probe.' `
        -CustodyFailures $custodyProbe `
        -GradingFailures $gradingProbe)
} catch {
    $admissionRefusal = New-PassiveReadyRefusalRecord `
        -ErrorRecord $_ `
        -CustodyEstablished $false `
        -Stage 'ready_validation'
}
Assert-Policy `
    -Condition ([string]$admissionRefusal.class -ceq 'receipt_admission') `
    -Message "An admission refusal was classified '$($admissionRefusal.class)'."
$admissionClauses = @($admissionRefusal.clauses)
Assert-Policy `
    -Condition ($admissionClauses.Count -eq 3) `
    -Message "An admission refusal produced $($admissionClauses.Count) clause records, not 3."
Assert-Policy `
    -Condition ([string]$admissionClauses[0].side -ceq 'custody' -and
        [string]$admissionClauses[0].field -ceq 'pid' -and
        [string]$admissionClauses[0].expected -ceq '4242' -and
        [string]$admissionClauses[0].observed -ceq '7') `
    -Message 'A custody clause did not round-trip its field, expected and observed values.'
Assert-Policy `
    -Condition ([string]$admissionClauses[1].side -ceq 'grading' -and
        [string]$admissionClauses[1].field -ceq 'active_hook_count' -and
        [string]$admissionClauses[1].expected -ceq '55') `
    -Message 'A grading clause did not round-trip, or was filed on the custody side.'
# A clause with no expected/actual pair -- Add-PassiveAdmissionFailure writes
# those too -- still has to become a named field rather than vanish.
Assert-Policy `
    -Condition ([string]$admissionClauses[2].field -ceq 'readiness_flags') `
    -Message 'A bare clause (no expected/actual pair) lost its field name.'

#    d2) THE REFUSAL SURVIVES BEING WRITTEN DOWN. CAPTURE_REFUSAL_V1.json is
#        the durable copy an operator reads after the transcript is gone, and
#        the harness writes it non-throwing -- so a serialisation that silently
#        flattened the clause list would leave a receipt that names the refusal
#        but not the field, with nothing failing anywhere. A single-element
#        clause list is the case that flattens.
$serialisedRefusal = @($admissionRefusal) | ConvertTo-Json -Depth 8 | ConvertFrom-Json
$serialisedClauses = @(@($serialisedRefusal)[0].clauses)
Assert-Policy `
    -Condition ($serialisedClauses.Count -eq 3 -and
        [string]$serialisedClauses[0].field -ceq 'pid' -and
        [string]$serialisedClauses[0].side -ceq 'custody' -and
        [string]@($serialisedRefusal)[0].class -ceq 'receipt_admission') `
    -Message 'A refusal record does not survive the JSON round trip the refusal receipt performs.'
$serialisedDrift = @($driftRefusal) | ConvertTo-Json -Depth 8 | ConvertFrom-Json
Assert-Policy `
    -Condition (@(@($serialisedDrift)[0].clauses).Count -eq 1 -and
        [string]@(@($serialisedDrift)[0].clauses)[0].field -ceq 'KNOWN_MASK') `
    -Message 'A single-clause refusal loses its clause list when serialised into the refusal receipt.'

#    e) THERE IS NO UNNAMED ROUTE. Anything the classifier cannot recognise --
#       a defect in this verifier, an unreadable receipt -- still produces a
#       class, a field and a non-empty message. "The harness could not say what
#       went wrong" has to be a statement on disk, not an absence.
foreach ($opaque in @(
    @{ Name = 'a plain string throw'; Action = { throw 'something went wrong' } },
    @{ Name = 'an empty-message exception'; Action = { throw [InvalidOperationException]::new('') } },
    @{ Name = 'a null-reference defect'; Action = { $null.ToString() } }
)) {
    $opaqueRefusal = $null
    try { & $opaque.Action } catch {
        $opaqueRefusal = New-PassiveReadyRefusalRecord `
            -ErrorRecord $_ `
            -CustodyEstablished $false `
            -Stage 'ready_validation'
    }
    Assert-Policy `
        -Condition ($null -ne $opaqueRefusal -and
            -not [string]::IsNullOrWhiteSpace([string]$opaqueRefusal.class) -and
            -not [string]::IsNullOrWhiteSpace([string]$opaqueRefusal.message) -and
            @($opaqueRefusal.clauses).Count -ge 1 -and
            -not [string]::IsNullOrWhiteSpace([string]@($opaqueRefusal.clauses)[0].field)) `
        -Message "$($opaque.Name) produced an UNNAMED refusal record; every route out must name something."
}

# 5. THE HARNESS'S SIDE OF THE SAME TWO DEFECTS.
#
#    Part index census, part parser. Each fact below is one a live capture
#    already paid for once, and the ones that are about CODE rather than about
#    text are asked of the AST -- an index census cannot tell a statement from a
#    comment mentioning it, which is a trap this file has now sprung twice.
$harnessParseErrors = $null
$harnessTokens = $null
$harnessAst = [System.Management.Automation.Language.Parser]::ParseFile(
    $harnessPath,
    [ref]$harnessTokens,
    [ref]$harnessParseErrors
)
Assert-Policy `
    -Condition ($null -ne $harnessAst -and $harnessParseErrors.Count -eq 0) `
    -Message "The passive harness does not parse: $(($harnessParseErrors | ForEach-Object { $_.Message }) -join '; ')"

$harnessSupervisionStep = $harness.IndexOf(
    "-Step 'recorder_cpu_affinity'",
    [StringComparison]::Ordinal
)
Assert-Policy `
    -Condition ($harnessSupervisionStep -ge 0) `
    -Message 'The harness no longer contains the recorder CPU-affinity pin, or no longer supervises it.'
#    a) THE RECORDER WAIT IS BOUNDED BEFORE ANYTHING THAT CAN THROW.
#
#       This is t38 exactly. The affinity pin threw; the budget, the deadline
#       and the finalize ceiling were all computed AFTER it; cleanup's wait loop
#       is guarded by `$null -ne $recorderDeadline` and
#       Update-RecorderFinalizeDeadline returns immediately on a null ceiling,
#       so the wait skipped itself and the harness walked away from a recorder
#       that went on to seal a 36 MB replay and authenticate its own teardown.
#       Moving any of these three back below the supervision step restores that.
foreach ($anchor in @(
    '$recorderBudgetSeconds = $DurationSeconds + $RecorderOverheadSeconds',
    '$recorderDeadline = [DateTime]::UtcNow.AddSeconds($recorderBudgetSeconds)',
    '$recorderFinalizeCeiling = [DateTime]::UtcNow.AddSeconds($FinalizeCeilingSeconds)'
)) {
    $anchorIndex = $harness.IndexOf($anchor, [StringComparison]::Ordinal)
    Assert-Policy `
        -Condition ($anchorIndex -ge 0 -and $anchorIndex -lt $harnessSupervisionStep) `
        -Message ("The recorder wait bound '$anchor' is not established before the " +
            'first post-launch supervision step. A throw in between leaves cleanup ' +
            'with a null deadline, and a null deadline skips the wait entirely ' +
            'instead of waiting the recorder out (run t38).')
}
$launchIndex = $harness.IndexOf(
    '$recorderProcess = Start-Process -FilePath $collectorRecord',
    [StringComparison]::Ordinal
)
Assert-Policy `
    -Condition ($launchIndex -ge 0 -and
        $launchIndex -lt $harness.IndexOf(
            '$recorderBudgetSeconds = $DurationSeconds + $RecorderOverheadSeconds',
            [StringComparison]::Ordinal
        )) `
    -Message 'The recorder budget is computed before the recorder exists, so it bounds nothing.'

#       "BEFORE THE FIRST SUPERVISION STEP" IS NOT THE CLAIM. The claim is
#       "before anything that can throw", and those are different: inserting
#       `$null = $recorderProcess.Handle` between the launch and the bounds
#       satisfies the ordering above while recreating t38 exactly -- that access
#       throws when the child is already gone, and cleanup would again find a
#       null deadline and skip the wait. So the statements BETWEEN the launch
#       and the last bound are enumerated as a closed set, and nothing may join
#       them without being written down here.
#       (Codex review 019fde5c-e4dd-7563-ad73-ba3548b40eb1, finding 3.)
#       The LAUNCH assignment specifically. `$recorderProcess = $null` is
#       declared far above with the rest of the run state, so the target alone
#       matches twice.
$launchStatement = @(
    $harnessAst.FindAll(
        {
            param($node)
            $node -is [System.Management.Automation.Language.AssignmentStatementAst] -and
            ([string]$node.Left.Extent.Text) -ceq '$recorderProcess' -and
            ([string]$node.Right.Extent.Text).StartsWith(
                'Start-Process -FilePath $collectorRecord',
                [StringComparison]::Ordinal)
        },
        $true
    )
)
Assert-Policy `
    -Condition ($launchStatement.Count -eq 1) `
    -Message "Expected exactly one assignment to `$recorderProcess; found $($launchStatement.Count)."
$launchParentStatements = @($launchStatement[0].Parent.Statements)
$launchOrdinal = -1
for ($index = 0; $index -lt $launchParentStatements.Count; $index++) {
    if ($launchParentStatements[$index] -eq $launchStatement[0]) { $launchOrdinal = $index }
}
Assert-Policy `
    -Condition ($launchOrdinal -ge 0) `
    -Message 'Could not locate the recorder launch among its own sibling statements.'
# Exactly these, and nothing else. Each is a plain assignment of a literal, a
# flag, or arithmetic over already-bound parameters: none of them can throw,
# which is the whole property being asserted.
#
# AND IN THIS ORDER. "Any order" was wrong and the gates proved nothing about
# it: the deadline is computed FROM the budget, so moving it above the budget
# assignment reads $recorderBudgetSeconds at its declared value of 0 and yields
# a deadline of "now" -- already expired by the time cleanup looks at it. The
# wait is then skipped for the same reason a null deadline skipped it, and the
# recorder is stranded exactly as in t38. That reorder is an ordinary editing
# mistake, not sabotage, and it passed every gate.
# (Codex review 019fde99-5837-7673-be74-f994c5a87277, finding 2.)
$allowedBeforeTheBounds = @(
    '$collectorInjectionStarted = $true',
    '$recorderBudgetSeconds = $DurationSeconds + $RecorderOverheadSeconds',
    '$recorderDeadline = [DateTime]::UtcNow.AddSeconds($recorderBudgetSeconds)',
    '$recorderFinalizeCeiling = [DateTime]::UtcNow.AddSeconds($FinalizeCeilingSeconds)'
)
$requiredBoundOrder = @(
    '$recorderBudgetSeconds = $DurationSeconds + $RecorderOverheadSeconds',
    '$recorderDeadline = [DateTime]::UtcNow.AddSeconds($recorderBudgetSeconds)',
    '$recorderFinalizeCeiling = [DateTime]::UtcNow.AddSeconds($FinalizeCeilingSeconds)'
)
$boundsSeen = [Collections.Generic.List[string]]::new()
$index = $launchOrdinal + 1
while ($index -lt $launchParentStatements.Count -and $boundsSeen.Count -lt 3) {
    $statement = $launchParentStatements[$index]
    $statementText = ([string]$statement.Extent.Text).Trim()
    # The standing compact-status verdict is a multi-line string assignment, so
    # it cannot be matched by whole text. Matching it by TARGET alone was not
    # enough either: `$undefinedStatusBitFailure = $(throw 'x')` satisfied the
    # prefix and threw at exactly the point this whole check exists to keep
    # clear. The exemption is therefore conditional on the right-hand side
    # being nothing but string literals joined by `+`.
    # (Codex review 019fdeb2-3e3d-79f2-a8b1-a86a8c8e26c1, finding 2.)
    if ($statementText -clike '$undefinedStatusBitFailure =*') {
        $rhsIsLiteral = $false
        if ($statement -is [System.Management.Automation.Language.AssignmentStatementAst]) {
            # Node TYPES alone were not enough: allowing any BinaryExpressionAst
            # admitted `'a' / 'b'`, which parses fine and throws at runtime. The
            # OPERATOR is checked too, and the only one permitted is string
            # concatenation.
            # (Codex review 019fde81-5d2b-73e1-8541-ef0492f5f62a, finding 2.)
            $rhsNodes = @(
                $statement.Right.FindAll({ param($node) $true }, $true) |
                    Where-Object {
                        -not (
                            $_ -is [System.Management.Automation.Language.StringConstantExpressionAst] -or
                            $_ -is [System.Management.Automation.Language.CommandExpressionAst] -or
                            $_ -is [System.Management.Automation.Language.PipelineAst] -or
                            (
                                $_ -is [System.Management.Automation.Language.BinaryExpressionAst] -and
                                $_.Operator -eq [System.Management.Automation.Language.TokenKind]::Plus
                            )
                        )
                    }
            )
            $rhsIsLiteral = ($rhsNodes.Count -eq 0)
        }
        Assert-Policy `
            -Condition $rhsIsLiteral `
            -Message ('The standing compact-status verdict between the recorder launch ' +
                'and its wait bounds is no longer a plain string literal: ' +
                "'$statementText'. Anything evaluated there can throw, and a throw " +
                'before the bounds are assigned leaves cleanup with a null deadline ' +
                'and no wait at all (run t38).')
        $index++
        continue
    }
    Assert-Policy `
        -Condition ($allowedBeforeTheBounds -ccontains $statementText) `
        -Message ("A statement sits between the recorder launch and its wait bounds " +
            "that is not in the closed no-throw set: '$statementText'. Everything " +
            'there must be incapable of throwing -- a throw before the bounds are ' +
            'assigned leaves cleanup with a null deadline and a null ceiling, which ' +
            'does not shorten the wait for the recorder, it removes it (run t38).')
    if ($statementText -cne '$collectorInjectionStarted = $true') {
        [void]$boundsSeen.Add($statementText)
    }
    $index++
}
Assert-Policy `
    -Condition ($boundsSeen.Count -eq 3) `
    -Message ("Only $($boundsSeen.Count) of the three recorder wait bounds are assigned " +
        'immediately after the launch.')
Assert-Policy `
    -Condition (($boundsSeen -join "`n") -ceq ($requiredBoundOrder -join "`n")) `
    -Message ("The recorder wait bounds are assigned out of dependency order. Expected:`n" +
        "  $($requiredBoundOrder -join "`n  ")`nGot:`n  $($boundsSeen -join "`n  ")`n" +
        'The deadline is computed FROM the budget, so assigning it first reads the ' +
        'budget at its declared 0 and produces a deadline of "now" -- already expired ' +
        'when cleanup reads it, which skips the recorder wait exactly as a null ' +
        'deadline did in run t38.')

#    a2) THE STATE THE EXECUTED PROBE SUPPLIES, PINNED AT ITS PRODUCER.
#
#        The probe below assigns injection, teardown, window, abort and
#        prefix-completion state directly, which is what lets it enumerate
#        routes -- and also what lets a harness that stopped PRODUCING any of
#        them pass every route. Each producer is therefore pinned here, at the
#        harness, so the probe tests wiring rather than its own fixtures.
#        (Codex review 019fdeb2-3e3d-79f2-a8b1-a86a8c8e26c1, finding 3.)
#        UNCONDITIONAL assignments only. `if ($false) { $harnessAborted = $true }`
#        keeps the AST count intact while making the assignment unreachable, so
#        an assignment is counted only when it is a DIRECT statement of a script
#        block body -- the script itself, a function, a catch, a finally -- and
#        not buried inside a condition, loop or switch.
#        (Codex review 019fde81-5d2b-73e1-8541-ef0492f5f62a, finding 3.)
$harnessAssignments = @(
    $harnessAst.FindAll(
        {
            param($node)
            $node -is [System.Management.Automation.Language.AssignmentStatementAst] -and
            (
                # A statement of the script's own top-level block...
                $node.Parent -is [System.Management.Automation.Language.NamedBlockAst] -or
                # ...or of a try/catch/finally body, which is where the run-state
                # flags are raised. NOT of an `if`, loop or switch body: those
                # are conditional, and a conditional assignment is exactly the
                # `if ($false) { ... }` bypass this predicate exists to reject.
                (
                    $node.Parent -is [System.Management.Automation.Language.StatementBlockAst] -and
                    (
                        $node.Parent.Parent -is [System.Management.Automation.Language.CatchClauseAst] -or
                        $node.Parent.Parent -is [System.Management.Automation.Language.TryStatementAst]
                    )
                )
            )
        },
        $true
    )
)
function Measure-HarnessAssignment {
    param(
        [Parameter(Mandatory = $true)][string]$Target,
        [Parameter(Mandatory = $true)][string]$Value
    )
    return @(
        $harnessAssignments | Where-Object {
            ([string]$_.Left.Extent.Text) -ceq $Target -and
            ([string]$_.Right.Extent.Text).Trim() -ceq $Value
        }
    ).Count
}
foreach ($producer in @(
    @{ Target = '$collectorInjectionStarted'; Value = '$true'; Count = 1
       Why = 'without it cleanup treats a live recorder as one that never existed, and every custody decision below turns on that flag' },
    @{ Target = '$collectorTeardownAuthenticated'; Value = '$false'; Count = 1
       Why = 'deleting the initial value makes an abort before authentication read an undefined variable under StrictMode, which skips late receipt admission and window closure entirely' },
    @{ Target = '$collectorTeardownAuthenticated'; Value = '$true'; Count = 2
       Why = 'the main path and the cleanup path each authenticate teardown; losing either leaves the graded window open forever' },
    @{ Target = '$collectorInjectionStarted'; Value = '$false'; Count = 1
       Why = 'the injection flag must start false or a run that never launched a recorder looks like one that did' },
    @{ Target = '$cleanupPrefixCompleted'; Value = '$false'; Count = 1
       Why = 'it must START false, or a cleanup prefix that threw still looks completed and the refusal code suppresses the exception' },
    @{ Target = '$cleanupPrefixCompleted'; Value = '$true'; Count = 1
       Why = 'and it must be set, or the refusal code can never fire at all' },
    @{ Target = '$harnessAborted'; Value = '$false'; Count = 1
       Why = 'the abort flag must start false or every run looks aborted' },
    @{ Target = '$harnessAborted'; Value = '$true'; Count = 1
       Why = 'and the outer catch must raise it, or a pending exception is invisible to the refusal code' }
)) {
    Assert-Policy `
        -Condition ((Measure-HarnessAssignment -Target $producer.Target -Value $producer.Value) -eq $producer.Count) `
        -Message ("The harness does not assign $($producer.Target) = $($producer.Value) " +
            "exactly $($producer.Count) time(s). The executed refusal probe SUPPLIES this " +
            "state, so its routes stay green without it: $($producer.Why).")
}
# The script's own last word. The probe appends its own `exit 0` after the
# lifted block, so its fall-through route cannot see what the real script
# returns when nothing was refused.
$harnessStatements = @($harnessAst.EndBlock.Statements)
Assert-Policy `
    -Condition (([string]$harnessStatements[-1].Extent.Text).Trim() -ceq 'exit $scriptExitCode') `
    -Message ("The harness no longer ends with 'exit `$scriptExitCode'; its last " +
        "statement is '$(([string]$harnessStatements[-1].Extent.Text).Trim())'. A run " +
        'that refused nothing returns whatever that statement says, and the refusal ' +
        "probe cannot see it because the probe appends its own exit.")

#    b) READY VALIDATION IS NAMED BEFORE IT PROPAGATES, and it still propagates.
#       Deleting the `throw` in that catch would convert a refused READY into a
#       capture that carried on, which is a far worse defect than the one this
#       lane is fixing.
$readyCallIndex = $harness.IndexOf(
    '$sharedReadyAuthority = Test-PassiveCaptureReadyReceipt',
    [StringComparison]::Ordinal
)
Assert-Policy `
    -Condition ($readyCallIndex -ge 0) `
    -Message 'The harness no longer validates its READY receipt through the shared verifier.'
# EVERYTHING ELSE ABOUT THAT CATCH IS CHECKED INSIDE ITS BODY, not by file
# offsets. Offsets were tried and both masked their own mutation: deleting the
# catch's `Write-CaptureRefusalReceipt` still found the cleanup one further
# down, and deleting the `phase=capture_refused` lines still matched the words
# in this script's own .SYNOPSIS block. A census that cannot tell a statement
# from a sentence about that statement is the failure mode this file already
# documents twice.
#
# Parsed, not string-matched, for the same reason: "a bare `throw` inside that
# catch" is a syntactic fact, and a regex over surrounding text also depends on
# whether git checked this file out with CRLF.
$namingCatches = @(
    $harnessAst.FindAll(
        {
            param($node)
            $node -is [System.Management.Automation.Language.CatchClauseAst] -and
            $node.Body.Extent.Text.Contains('New-PassiveReadyRefusalRecord')
        },
        $true
    )
)
Assert-Policy `
    -Condition ($namingCatches.Count -eq 1) `
    -Message "Expected exactly one catch that names a READY refusal; found $($namingCatches.Count)."
$bareRethrows = @(
    $namingCatches[0].Body.FindAll(
        {
            param($node)
            $node -is [System.Management.Automation.Language.ThrowStatementAst] -and
            $null -eq $node.Pipeline
        },
        $true
    )
)
Assert-Policy `
    -Condition ($bareRethrows.Count -eq 1) `
    -Message ('The READY refusal catch no longer re-raises the original error. Naming a ' +
        'refusal must not turn it into a capture that continues, and a bare `throw` is ' +
        'what keeps the exception -- and the abort reason the teardown check quotes -- ' +
        'exactly as it arrived.')

# UNCONDITIONALLY, and that is a separate claim from "present". Wrapping the
# whole catch body in `if ($false) { ... }` leaves every recursive census below
# satisfied while a real refusal is silently swallowed: no record, no receipt,
# no rethrow, and the run dies later on a null authority instead. The statements
# that MUST run are therefore required to be direct statements of the catch,
# not merely somewhere inside it.
# (Codex review 019fde81-5d2b-73e1-8541-ef0492f5f62a, finding 4.)
$namingCatchBodyAst = $namingCatches[0].Body
# DIRECT AND UNCONDITIONAL. Taking the direct statements and substring-matching
# them was not enough on its own: `if ($false) { Write-CaptureRefusalReceipt }`
# IS a direct statement, and its text contains the token. Conditional and
# looping statements are therefore dropped before matching, so what remains is
# only what runs every time the catch runs.
$namingCatchDirect = @(
    $namingCatchBodyAst.Statements |
        Where-Object {
            $_ -isnot [System.Management.Automation.Language.IfStatementAst] -and
            $_ -isnot [System.Management.Automation.Language.SwitchStatementAst] -and
            $_ -isnot [System.Management.Automation.Language.LoopStatementAst] -and
            $_ -isnot [System.Management.Automation.Language.TryStatementAst]
        } |
        ForEach-Object { ([string]$_.Extent.Text).Trim() }
)
foreach ($mustRun in @(
    @{ Match = 'New-PassiveReadyRefusalRecord'
       Why   = 'the refusal is classified' },
    @{ Match = '$captureRefusals.Add('
       Why   = 'the classified record is recorded' },
    @{ Match = 'Write-CaptureRefusalReceipt'
       Why   = 'the refusal receipt is written' }
)) {
    Assert-Policy `
        -Condition (@($namingCatchDirect | Where-Object {
            $_.IndexOf($mustRun.Match, [StringComparison]::Ordinal) -ge 0
        }).Count -ge 1) `
        -Message ("The READY refusal catch does not UNCONDITIONALLY run the step where " +
            "$($mustRun.Why) ('$($mustRun.Match)'). Present-but-unreachable is the one " +
            'shape a recursive census cannot tell from working code, so this step has ' +
            'to be a direct statement of the catch.')
}
Assert-Policy `
    -Condition (@($namingCatchDirect | Where-Object { $_ -ceq 'throw' }).Count -eq 1) `
    -Message ('The READY refusal catch does not UNCONDITIONALLY re-raise. A `throw` ' +
        'nested inside a condition can be skipped, and a refused READY that does not ' +
        'propagate becomes a capture that carries on.')
# CODE, NOT TEXT, inside the catch's own body.
#
# Scoping to the body was not enough on its own: raw `IndexOf` over that text
# still matched a COMMENTED-OUT call, so `# Write-CaptureRefusalReceipt` passed
# while the receipt was never written. Commands are counted as CommandAsts and
# emitted tokens as string literals, both of which a comment cannot satisfy.
# (Codex review 019fde5c-e4dd-7563-ad73-ba3548b40eb1, finding 4.)
$namingCatchCommands = @(
    $namingCatchBodyAst.FindAll(
        {
            param($node)
            $node -is [System.Management.Automation.Language.CommandAst]
        },
        $true
    ) | ForEach-Object { [string]$_.GetCommandName() }
)
Assert-Policy `
    -Condition ($namingCatchCommands -ccontains 'Write-CaptureRefusalReceipt') `
    -Message ('The READY refusal catch no longer CALLS Write-CaptureRefusalReceipt. ' +
        'A refusal that is not written down at the moment it happens is a refusal an ' +
        'operator reconstructs from a transcript, which is what run t36 had to do.')
Assert-Policy `
    -Condition ($namingCatchCommands -ccontains 'New-PassiveReadyRefusalRecord') `
    -Message 'The READY refusal catch no longer builds a named refusal record.'

# THE RECORD HAS TO REACH THE LIST THE ARBITRATION READS. Building it and
# dropping it leaves CAPTURE_REFUSAL_V1.json carrying an empty refusal array and
# the refusal code never firing -- the refusal would exist only as console text.
# The ARGUMENT too, not merely that some Add happened: `$captureRefusals.Add($null)`
# satisfied a bare call census while putting a null where the named record
# belongs, so the receipt would carry a null refusal and every field an operator
# needs would be absent. The value added has to be the variable the classifier's
# result was bound to.
# (Codex review 019fdeb2-3e3d-79f2-a8b1-a86a8c8e26c1, finding 4.)
$refusalRecordVariables = @(
    $namingCatchBodyAst.FindAll(
        {
            param($node)
            $node -is [System.Management.Automation.Language.AssignmentStatementAst] -and
            ([string]$node.Right.Extent.Text).Contains('New-PassiveReadyRefusalRecord')
        },
        $true
    ) | ForEach-Object { ([string]$_.Left.Extent.Text).Trim() }
)
Assert-Policy `
    -Condition ($refusalRecordVariables.Count -eq 1) `
    -Message ('The READY refusal catch does not bind exactly one classified refusal ' +
        "record; found $($refusalRecordVariables.Count).")
$namingCatchAdds = @(
    $namingCatchBodyAst.FindAll(
        {
            param($node)
            $node -is [System.Management.Automation.Language.InvokeMemberExpressionAst] -and
            [string]$node.Member.Extent.Text -ceq 'Add' -and
            [string]$node.Expression.Extent.Text -ceq '$captureRefusals' -and
            @($node.Arguments).Count -eq 1 -and
            ([string]@($node.Arguments)[0].Extent.Text).Trim() -ceq $refusalRecordVariables[0]
        },
        $true
    )
)
Assert-Policy `
    -Condition ($namingCatchAdds.Count -eq 1) `
    -Message ('The READY refusal catch does not add its CLASSIFIED RECORD ' +
        "($($refusalRecordVariables[0])) to `$captureRefusals. Adding anything else -- " +
        '$null in particular -- leaves the refusal receipt carrying an entry with no ' +
        'class, no field and no message, which is the unnamed refusal this lane exists ' +
        'to remove.')

# The emitted phase lines, as string literals BOUND TO A Write-Host. A comment
# mentioning them cannot satisfy this, and neither can a literal that is built
# and then dropped -- the census collects only strings that sit inside a
# Write-Host command, which is what actually puts them on the transcript.
$namingCatchStrings = @(
    $namingCatchBodyAst.FindAll(
        {
            param($node)
            $node -is [System.Management.Automation.Language.CommandAst] -and
            $node.GetCommandName() -ceq 'Write-Host'
        },
        $true
    ) | ForEach-Object {
        $_.FindAll(
            {
                param($inner)
                $inner -is [System.Management.Automation.Language.StringConstantExpressionAst] -or
                $inner -is [System.Management.Automation.Language.ExpandableStringExpressionAst]
            },
            $true
        )
    } | ForEach-Object { [string]$_.Extent.Text }
)
foreach ($required in @(
    @{ Token = 'phase=capture_refused '
       Why   = 'the machine-readable summary line' },
    @{ Token = 'phase=capture_refused_clause '
       Why   = 'the per-clause line naming field, expected and observed' }
)) {
    Assert-Policy `
        -Condition (@($namingCatchStrings | Where-Object {
            $_.IndexOf($required.Token, [StringComparison]::Ordinal) -ge 0
        }).Count -ge 1) `
        -Message ("The READY refusal catch no longer emits $($required.Why) " +
            "('$($required.Token.Trim())') as an actual string literal.")
}

# FINDING 1's GAP: the abort-in-flight guard reads $harnessAbortReason, and the
# executed probe SUPPLIES that variable, so deleting the harness assignment left
# every route green while the guard became permanently inert. The concrete cost:
# affinity supervision fails, the report clears the status verdict, the recorder
# then fails to expose an exit code and throws -- and with no recorded abort
# reason the refusal block returns 3 and suppresses it.
#
# The producer is the outer catch, and it must both RECORD and re-raise.
# (Codex review 019fde5c-e4dd-7563-ad73-ba3548b40eb1, finding 1.)
$abortRecordingCatches = @(
    $harnessAst.FindAll(
        {
            param($node)
            $node -is [System.Management.Automation.Language.CatchClauseAst] -and
            @($node.Body.FindAll(
                {
                    param($inner)
                    $inner -is [System.Management.Automation.Language.AssignmentStatementAst] -and
                    [string]$inner.Left.Extent.Text -ceq '$harnessAbortReason'
                },
                $true
            )).Count -ge 1
        },
        $true
    )
)
Assert-Policy `
    -Condition ($abortRecordingCatches.Count -eq 1) `
    -Message ('Expected exactly one catch recording $harnessAbortReason; found ' +
        "$($abortRecordingCatches.Count). It is the only producer of the abort-in-flight " +
        'signal the refusal arbitration reads, and the arbitration is what decides ' +
        'whether a pending exception gets suppressed by the refusal code.')
Assert-Policy `
    -Condition (@($abortRecordingCatches[0].Body.FindAll(
        {
            param($node)
            $node -is [System.Management.Automation.Language.ThrowStatementAst] -and
            $null -eq $node.Pipeline
        },
        $true
    )).Count -eq 1) `
    -Message 'The catch that records $harnessAbortReason no longer re-raises the error it recorded.'
Assert-Policy `
    -Condition ($harness.IndexOf('CAPTURE_REFUSAL_V1.json', [StringComparison]::Ordinal) -ge 0) `
    -Message 'The harness no longer names a refusal receipt path.'

#    c) THE REFUSAL CODE CANNOT DISPLACE THE COMPACT-STATUS VERDICT.
#
#       Exit 2 is the only code a caller keys on and it is gone forever if
#       anything exits first. The named-refusal code is therefore placed strictly
#       after it, and a refusal must not soften a custody failure either: an
#       unauthenticated teardown or an unclosed graded window still leaves at 1.
# ANCHORED ON THE ARBITRATION'S CONDITION, NOT ON THE STRING 'exit 2'. The
# literal `exit 2` first occurs inside a COMMENT roughly 150 kB earlier ("the
# raw-capture report's UNDEFINED-STATUS-BIT verdict (exit 2)"), so anchoring on
# it put every ordering test below into the wrong region of the file and they
# passed without comparing what they claimed to compare.
$verdictArbitrationIndex = $harness.IndexOf(
    'if (-not [string]::IsNullOrWhiteSpace($undefinedStatusBitFailure)) {',
    [StringComparison]::Ordinal
)
Assert-Policy `
    -Condition ($verdictArbitrationIndex -ge 0 -and
        $harness.IndexOf(
            'if (-not [string]::IsNullOrWhiteSpace($undefinedStatusBitFailure)) {',
            $verdictArbitrationIndex + 1,
            [StringComparison]::Ordinal
        ) -lt 0) `
    -Message 'Expected exactly one compact-status arbitration point in the harness.'
$exitTwoIndex = $harness.IndexOf(
    'exit 2',
    $verdictArbitrationIndex,
    [StringComparison]::Ordinal
)
Assert-Policy `
    -Condition ($exitTwoIndex -gt $verdictArbitrationIndex) `
    -Message 'The compact-status arbitration no longer returns exit 2.'
$refusalBlockIndex = $harness.IndexOf(
    'if ($captureRefusals.Count -gt 0 -or $supervisionFailures.Count -gt 0) {',
    $exitTwoIndex,
    [StringComparison]::Ordinal
)
Assert-Policy `
    -Condition ($refusalBlockIndex -gt $exitTwoIndex) `
    -Message ('The named-refusal arbitration does not sit below the compact-status ' +
        'arbitration. Exit 2 has absolute precedence: it is the only machine-readable ' +
        'verdict that is lost if another code is returned first.')
#       WHAT IS DELIBERATELY NOT CENSUSED HERE. The obvious next assertions --
#       "the cleanup-fatal guard appears before the `exit 3` statement", and so
#       on -- were written, and they matched PROSE: the block's own comment
#       contains the words `exit 3`, and the first `exit 2` in the file is
#       inside a comment 150 kB above the arbitration. A census that cannot tell
#       a statement from a sentence about that statement is the failure mode
#       this file already documents twice. Which outcome outranks a refusal is
#       therefore settled by RUNNING the block, in (d) below, where the answer
#       is an exit code rather than an offset.

#    d) THE ARBITRATION IS RUN, NOT READ.
#
#       Everything above is an ordering census, and an ordering census cannot
#       see the two defects that actually cost captures here: under
#       $ErrorActionPreference = 'Stop' a terminating write before an `exit`
#       means the `exit` never runs, and a pending exception resumes AFTER a
#       `finally` and skips anything below it. Both return 1 while the source
#       reads correctly. compact_contract's script_status_vocabulary_sweep
#       proves the exit-2 block that way; this does the same for the refusal
#       block, by LIFTING it out of the harness and running it in a child
#       process under each state combination that decides its answer.
#
#       Retyping the block here would prove nothing, so it is extracted from
#       the harness source by brace matching.
# NAIVE BRACE MATCHING, and that is sound only because of what it is pointed
# at. The arbitration block contains no brace inside any string literal or
# comment -- its interpolations are all `$(...)`, which is parens -- so counting
# braces cannot be thrown off. Anything else would need a parser.
#
# -StartIndex is required, not optional: the same condition also guards the
# cleanup-prefix accounting block that PRINTS the refusals, and lifting that one
# by accident would run a probe against code that returns no exit code at all.
function Get-PowerShellIfBlock {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Condition,
        [Parameter(Mandatory = $true)][int]$StartIndex
    )

    $marker = "if ($Condition) {"
    $start = $Source.IndexOf($marker, $StartIndex, [StringComparison]::Ordinal)
    if ($start -lt 0) { return $null }
    if ($Source.IndexOf($marker, $start + 1, [StringComparison]::Ordinal) -ge 0) {
        # More than one copy after the anchor means some route is served by a
        # duplicate that can drift, and this could not say which one ships.
        return $null
    }
    $depth = 0
    for ($index = $start; $index -lt $Source.Length; $index++) {
        if ($Source[$index] -ceq '{') { $depth++ }
        elseif ($Source[$index] -ceq '}') {
            $depth--
            if ($depth -eq 0) {
                return $Source.Substring($start, $index - $start + 1)
            }
        }
    }
    return $null
}

$refusalArbitration = Get-PowerShellIfBlock `
    -Source $harness `
    -Condition '$captureRefusals.Count -gt 0 -or $supervisionFailures.Count -gt 0' `
    -StartIndex $exitTwoIndex
Assert-Policy `
    -Condition (-not [string]::IsNullOrWhiteSpace($refusalArbitration)) `
    -Message ('Could not lift exactly one named-refusal arbitration block from the ' +
        'harness. More than one means a route is served by a copy that can drift; ' +
        'none means a named refusal has no code to return.')

#       THE PROBE SUPPLIES $cleanupPrefixCompleted, SO THE HARNESS MUST BE MADE
#       TO EARN IT SEPARATELY. Deleting the assignment from the harness leaves
#       every route above passing -- the probe sets the variable itself -- while
#       the real script would never again return 3, because the flag it reads
#       would be permanently $false. That fails safe, but it retires the code
#       silently, and a code that cannot fire is not a code.
#
#       What has to hold is a placement, so it is checked with the parser: the
#       assignment is the LAST statement of the cleanup prefix, which is the
#       inner `try` whose `finally` carries the arbitration. Anything after it
#       could throw and leave the flag claiming a prefix that did not complete.
#       The arbitration lives in an INNER finally nested inside the outer one,
#       so the outer try matches this text search too -- its finally contains
#       the inner try whole. The innermost match is the one wanted, and it is
#       the one with the shortest finally.
$cleanupPrefixTries = @(
    $harnessAst.FindAll(
        {
            param($node)
            $node -is [System.Management.Automation.Language.TryStatementAst] -and
            $null -ne $node.Finally -and
            $node.Finally.Extent.Text.Contains(
                'if ($captureRefusals.Count -gt 0 -or $supervisionFailures.Count -gt 0) {')
        },
        $true
    ) | Sort-Object { $_.Finally.Extent.Text.Length }
)
Assert-Policy `
    -Condition ($cleanupPrefixTries.Count -ge 1) `
    -Message 'No try/finally in the harness carries the refusal arbitration.'
$cleanupPrefixStatements = @($cleanupPrefixTries[0].Body.Statements)
$lastPrefixStatement = ''
if ($cleanupPrefixStatements.Count -gt 0) {
    $lastPrefixStatement = ([string]$cleanupPrefixStatements[-1].Extent.Text).Trim()
}
Assert-Policy `
    -Condition ($lastPrefixStatement -ceq '$cleanupPrefixCompleted = $true') `
    -Message ("The cleanup prefix does not END by recording that it completed; its last " +
        "statement is '$lastPrefixStatement'. Reaching that assignment is the only " +
        'evidence the refusal arbitration has that the prefix did not die on a ' +
        'terminating error, and $ErrorActionPreference is ''Stop'' so every failing ' +
        'cmdlet in there is terminating. Without it the refusal code either suppresses ' +
        'that failure or can never fire at all.')

$arbitrationProbeDir = Join-Path ([IO.Path]::GetTempPath()) "hoi4_policy_exit3_$PID"
[void](New-Item -ItemType Directory -Path $arbitrationProbeDir -Force)
try {
    $routeIndex = 0
    foreach ($route in @(
        @{  Name     = 'a named refusal on an otherwise clean run'
            Expected = 3
            Pending  = $true
            State    = @{ Refusals = 1; Supervision = 0; Injected = 'true'
                          Teardown = 'true'; Window = 'false'; CleanupFatal = '' } },
        @{  Name     = 'a supervision failure with no capture refusal'
            Expected = 3
            Pending  = $true
            State    = @{ Refusals = 0; Supervision = 1; Injected = 'true'
                          Teardown = 'true'; Window = 'false'; CleanupFatal = '' } },
        # The three that must OUTRANK a refusal. Each is normally reported by a
        # route that this block now runs AHEAD of -- the cleanup-fatal throw
        # fires after the inner `finally` returns, and the two custody guards
        # sit after the outer one -- so an omission here is silent.
        @{  Name     = 'a refusal alongside an unauthenticated teardown'
            Expected = 1
            Pending  = $true
            State    = @{ Refusals = 1; Supervision = 0; Injected = 'true'
                          Teardown = 'false'; Window = 'false'; CleanupFatal = '' } },
        @{  Name     = 'a refusal alongside an unclosed graded window'
            Expected = 1
            Pending  = $true
            State    = @{ Refusals = 1; Supervision = 0; Injected = 'true'
                          Teardown = 'true'; Window = 'true'; CleanupFatal = '' } },
        @{  Name     = 'a refusal alongside a failed cleanup'
            Expected = 1
            Pending  = $true
            State    = @{ Refusals = 1; Supervision = 1; Injected = 'true'
                          Teardown = 'true'; Window = 'false'; CleanupFatal = 'probe cleanup failure' } },
        # Nothing refused: the block must fall THROUGH and leave the run's own
        # outcome alone. A block that exited unconditionally would turn every
        # successful capture into a refusal. Run on the CLEAN shape, because
        # that is the only shape where "fell through" is observable -- on the
        # pending-exception shape the exception resumes and decides the code.
        @{  Name     = 'no refusal at all, clean route'
            Expected = 0
            Pending  = $false
            State    = @{ Refusals = 0; Supervision = 0; Injected = 'true'
                          Teardown = 'true'; Window = 'false'; CleanupFatal = '' } },
        # ...and on the pending-exception shape it must NOT swallow the
        # exception either. A block that exited 0 unconditionally would convert
        # every failed capture into a success, which is worse than anything
        # this lane set out to fix.
        @{  Name     = 'no refusal at all, exception in flight'
            Expected = 1
            Pending  = $true
            State    = @{ Refusals = 0; Supervision = 0; Injected = 'true'
                          Teardown = 'true'; Window = 'false'; CleanupFatal = '' } },
        # The recorder's own nonzero exit outranks a refusal. Everything ran,
        # nothing is unwinding, cleanup completed -- and the recorder still
        # failed, so 3 would overwrite the code that says so.
        @{  Name     = 'a refusal alongside a nonzero recorder exit'
            Expected = 5
            Pending  = $false
            RecorderExit = 5
            State    = @{ Refusals = 0; Supervision = 1; Injected = 'true'
                          Teardown = 'true'; Window = 'false'; CleanupFatal = '' } },
        # ...and the same outranking still holds when the block CANNOT PRINT.
        # Every diagnostic here can throw -- a closed or failing stderr is
        # enough -- and under 'Stop' that abandons the rest of the block. While
        # the writes were interleaved with the exits, this route returned a bare
        # 1 instead of the recorder's 5: the decision was lost to a failed
        # report. The outcome is now computed before anything is printed.
        @{  Name     = 'a refusal, a nonzero recorder exit, and stderr that throws'
            Expected = 5
            Pending  = $false
            RecorderExit = 5
            StderrClosed = $true
            State    = @{ Refusals = 0; Supervision = 1; Injected = 'true'
                          Teardown = 'true'; Window = 'false'; CleanupFatal = '' } },
        # THE TWO FAILURES THE BLOCK CANNOT SEE FROM WHERE IT SITS, and an
        # `exit` inside a `finally` beats a pending exception -- so if either is
        # unchecked the refusal code silently becomes the answer for a run that
        # failed for an entirely different reason.
        #
        # A still-unwinding abort from the try body. The outer catch records it
        # as $harnessAbortReason before re-raising, which is the only handle the
        # arbitration has on it.
        @{  Name     = 'a refusal while an abort is still unwinding'
            Expected = 1
            Pending  = $true
            Abort    = 'probe abort reason'
            State    = @{ Refusals = 1; Supervision = 0; Injected = 'true'
                          Teardown = 'true'; Window = 'false'; CleanupFatal = '' } },
        # THE SAME ABORT WITH NOTHING TO SAY. `throw [InvalidOperationException]::new('')`
        # is enough to produce it, and while the arbitration inferred "aborted"
        # from a non-empty reason STRING this route returned 3 and overwrote a
        # live exception. The reason is deliberately blank here and only the
        # flag is set.
        @{  Name     = 'a refusal while a message-less abort is unwinding'
            Expected = 1
            Pending  = $true
            Aborted  = 'true'
            State    = @{ Refusals = 1; Supervision = 0; Injected = 'true'
                          Teardown = 'true'; Window = 'false'; CleanupFatal = '' } },
        # A terminating error in the cleanup PREFIX. $ErrorActionPreference is
        # 'Stop', so every failing cmdlet there is terminating and none of them
        # sets $cleanupFatalMessage. Reaching the end of the prefix is the only
        # evidence available, and this route is the one that never reaches it.
        @{  Name     = 'a refusal after the cleanup prefix threw'
            Expected = 1
            Pending  = $false
            PrefixThrew = $true
            State    = @{ Refusals = 1; Supervision = 1; Injected = 'true'
                          Teardown = 'true'; Window = 'false'; CleanupFatal = '' } }
    )) {
        $routeIndex++
        $state = $route.State
        $addRefusal = if ($state.Refusals -gt 0) {
            '[void]$captureRefusals.Add([pscustomobject]@{ stage = ''ready_validation''; class = ''wire_field_order''; message = ''probe refusal'' })'
        } else { '' }
        $addSupervision = if ($state.Supervision -gt 0) {
            '[void]$supervisionFailures.Add(''recorder_cpu_affinity: probe'')'
        } else { '' }
        # The harness's own preamble. Anything softer proves nothing: 'Stop' is
        # what makes a failing write terminating, and StrictMode is what makes a
        # missing variable terminating. The block is run as the body of an inner
        # `finally` with an exception already in flight from the try body --
        # the route on which merely being LAST silently returns 1.
        $probeLines = @(
            "`$ErrorActionPreference = 'Stop'",
            'Set-StrictMode -Version Latest',
            '$captureRefusals = [Collections.Generic.List[object]]::new()',
            $addRefusal,
            '$supervisionFailures = [Collections.Generic.List[string]]::new()',
            $addSupervision,
            "`$captureRefusalReceiptPath = 'probe-CAPTURE_REFUSAL_V1.json'",
            "`$collectorInjectionStarted = `$$($state.Injected)",
            "`$collectorTeardownAuthenticated = `$$($state.Teardown)",
            "`$gradedWindowStarted = `$$($state.Window)",
            "`$cleanupFatalMessage = '$($state.CleanupFatal)'",
            "`$harnessAbortReason = '$(if ($route.ContainsKey('Abort')) { $route.Abort } else { '' })'",
            # The FLAG, not the text. A message-less exception leaves the reason
            # empty, and an arbitration that inferred "no abort" from an empty
            # string suppressed the pending exception and returned 3 -- which
            # the abort route could not catch while it only ever supplied a
            # non-empty reason. So one route now carries an aborted flag with a
            # BLANK reason, which is exactly that case.
            "`$harnessAborted = `$$(if ($route.ContainsKey('Aborted')) { $route.Aborted } elseif ($route.ContainsKey('Abort')) { 'true' } else { 'false' })",
            # $cleanupPrefixCompleted is set by the LAST statement of the
            # cleanup prefix, so a prefix that threw never gets there.
            "`$cleanupPrefixCompleted = `$$(if ($route.ContainsKey('PrefixThrew') -and $route.PrefixThrew) { 'false' } else { 'true' })",
            # The recorder's OWN verdict. A refusal must not return 3 over a
            # nonzero recorder exit: that would report "the harness refused
            # something" for a run whose recorder fell over, and lose the code
            # that says which.
            "`$scriptExitCode = $(if ($route.ContainsKey('RecorderExit')) { $route.RecorderExit } else { 0 })",
            # A stderr writer that is already closed, so every diagnostic in
            # the lifted block throws.
            $(if ($route.ContainsKey('StderrClosed') -and $route.StderrClosed) {
                '[Console]::SetError((New-Object IO.StreamWriter ([IO.Stream]::Null))); [Console]::Error.Close()'
            } else { '' }),
            $(if ($route.ContainsKey('PrefixThrew') -and $route.PrefixThrew) {
                # A terminating error in the cleanup prefix. The inner finally
                # still runs, with that error unwinding through it.
                "try { } finally { try { throw 'cleanup prefix' } finally {"
            } elseif ($route.Pending) {
                # The route on which merely being LAST in a block silently
                # returns 1: an exception from the try body resumes AFTER the
                # finally and skips anything placed below it.
                "try { throw 'pending' } finally { try { } finally {"
            } else {
                'try { } finally { try { } finally {'
            }),
            $refusalArbitration,
            '} }',
            # What the real script does when the refusal block declines: it
            # falls through to its own `exit $scriptExitCode`.
            'exit $scriptExitCode'
        )
        $probePath = Join-Path $arbitrationProbeDir "route_$routeIndex.ps1"
        Set-Content -LiteralPath $probePath -Value ($probeLines -join "`r`n") -Encoding UTF8
        # 'Continue' for the child call ONLY. The probe writes its diagnostics
        # to stderr exactly as the harness does, and under 'Stop' a native
        # command's stderr becomes a terminating NativeCommandError here -- the
        # probe would kill this gate instead of reporting an exit code, and it
        # would do so on the routes that are behaving correctly.
        $probeExit = 1
        try {
            $ErrorActionPreference = 'Continue'
            $null = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $probePath 2>&1
            $probeExit = $LASTEXITCODE
        } finally {
            $ErrorActionPreference = 'Stop'
        }
        $observedExit = $probeExit
        Assert-Policy `
            -Condition ($observedExit -eq $route.Expected) `
            -Message ("The named-refusal arbitration returned $observedExit for " +
                "'$($route.Name)', not $($route.Expected). This block is RUN rather than " +
                'read because both defects it can carry are invisible to reading: a ' +
                'terminating write before an `exit` means the `exit` never runs, and a ' +
                'pending exception resumes after a `finally` and skips whatever sits ' +
                'below it. Both return 1 while the source still reads correctly.')
    }
} finally {
    Remove-Item -LiteralPath $arbitrationProbeDir -Recurse -Force -ErrorAction SilentlyContinue
}

# THE WATCHDOG'S TWO NEW HELPERS ARE POLICED HERE ONLY FOR THEIR EXISTENCE AND
# THEIR FENCE. Their DECISION TABLES are driven, over temporary receipts and a
# dead pid, by Test-RecorderFinalizeProgressBound.ps1 -- the executable self-test
# that already lifts this harness's finalize bound by AST and exists for exactly
# that machinery. This file is a policy and AST census and is not the right home
# for behaviour (Codex review round 3, finding D4).
$watchdogHelperAsts = @(
    $harnessAst.FindAll(
        {
            param($node)
            $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
            @('Get-RecorderFinalizeTerminalReason', 'Test-TargetProcessGone') -contains $node.Name
        },
        $true
    )
)
Assert-Policy `
    -Condition ($watchdogHelperAsts.Count -eq 2) `
    -Message ('The passive harness must define both Get-RecorderFinalizeTerminalReason and ' +
        'Test-TargetProcessGone. Without the first, a run that timed out on a stalled ' +
        'finalize reports the last extension it was granted as its cause; without the ' +
        'second, a recorder whose target is already gone is left running unsupervised ' +
        '(run t60 ran fifty more minutes that way and filled the system disk).')

# EVERY STOP OF THE RECORDER MUST BE A DESCENDANT OF THE BRANCH THAT PROVED NO
# HOOKS CAN BE LIVE -- established by AST ANCESTRY, not by string order.
#
# Textual ordering was the first spelling of this fence and it is not a fence at
# all: a second, unguarded stop placed further down the file passes it, and so
# does moving the stop out of the `if` while leaving the tokens in the same
# order. Ancestry is the property actually claimed -- the invocation is INSIDE
# the guarded branch -- so it is the property checked (Codex review round 4,
# finding R5.4).
$stopInvocations = @(
    $harnessAst.FindAll(
        {
            param($node)
            $node -is [System.Management.Automation.Language.CommandAst] -and
            ([string]$node.GetCommandName()) -in @('Stop-ExactProcess', 'Stop-Process')
        },
        $true
    )
)
# The recorder handle is the only process this fence governs. Stop-ExactProcess
# is also used on the game itself elsewhere in the harness, under rules of its own.
$recorderStops = @(
    $stopInvocations | Where-Object {
        ([string]$_.Extent.Text).IndexOf('$recorderProcess', [StringComparison]::Ordinal) -ge 0
    }
)
Assert-Policy `
    -Condition ($recorderStops.Count -eq 1) `
    -Message ("The passive harness must contain exactly ONE stop of the recorder handle; " +
        "found $($recorderStops.Count). Each one is a place the post-injection termination " +
        'prohibition has to be reasoned past, and a second is how an unguarded one arrives ' +
        'looking like the guarded one.')
foreach ($stop in $recorderStops) {
    $ancestor = $stop.Parent
    $guarded = $false
    while ($null -ne $ancestor) {
        if ($ancestor -is [System.Management.Automation.Language.IfStatementAst]) {
            foreach ($clause in $ancestor.Clauses) {
                # THE STOP MUST BE IN THAT CLAUSE'S BODY, not merely somewhere
                # under an `if` that happens to mention the predicate. Checking
                # the condition alone would pass a stop sitting in the ELSE
                # branch -- the branch that runs when the hooks were NOT proven
                # released, which is precisely the case the rule forbids.
                # (Codex review round 5, finding S4.)
                if (([string]$clause.Item1.Extent.Text).IndexOf(
                        '$hooksProvenReleased', [StringComparison]::Ordinal) -ge 0 -and
                    $stop.Extent.StartOffset -ge $clause.Item2.Extent.StartOffset -and
                    $stop.Extent.EndOffset -le $clause.Item2.Extent.EndOffset) {
                    $guarded = $true
                }
            }
        }
        $ancestor = $ancestor.Parent
    }
    Assert-Policy `
        -Condition $guarded `
        -Message ('The passive harness may stop collector_record only INSIDE the branch whose ' +
            'condition is $hooksProvenReleased -- the one that has already established either ' +
            'that the target process is gone or that teardown was authenticated. A stop ' +
            'reachable without that proof is the post-injection termination prohibition ' +
            'broken: it can orphan hooks and the injected DLL inside a live game.')
}
# And the predicate itself must still be computed from the identity check rather
# than assigned a constant, which ancestry alone would not catch.
Assert-Policy `
    -Condition ($harness.IndexOf(
        '$hooksProvenReleased = $targetGone -or $collectorTeardownAuthenticated',
        [StringComparison]::Ordinal) -ge 0) `
    -Message ('$hooksProvenReleased must be derived from Test-TargetProcessGone and the ' +
        'authenticated-teardown flag. Assigning it any other way makes the branch above a ' +
        'formality.')
Assert-Policy `
    -Condition ($harness.IndexOf(
        '$targetGone = Test-TargetProcessGone',
        [StringComparison]::Ordinal) -ge 0) `
    -Message 'The harness must establish $targetGone through Test-TargetProcessGone.'
Assert-Policy `
    -Condition ($harness.IndexOf('Stop-Process', [StringComparison]::OrdinalIgnoreCase) -lt 0) `
    -Message ('The passive harness must not reach for Stop-Process. The only admissible stop ' +
        'is the identity-checked one on the recorder handle this run launched; Stop-Process ' +
        'takes a pid, and a pid is not an identity.')

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
