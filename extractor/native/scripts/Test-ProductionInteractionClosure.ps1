[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir '..\..\..')).Path
$driverPath = Join-Path $scriptDir 'Start-Hoi4ProductionCaptureSafe.ps1'
$readOnlyWitnessPath = Join-Path $scriptDir 'Read-Hoi4GameState.ps1'
$packagePath = Join-Path $scriptDir 'New-PortableProductPackage.ps1'
$packageVerifierPath = Join-Path $scriptDir 'Test-PortableProductPackageReceipt.ps1'
$interactionClosureHelperPath = Join-Path $scriptDir 'PortableProduct.InteractionClosure.ps1'
$promotionHelperPath = Join-Path $scriptDir 'ProductionCapture.Promotion.ps1'
$runnerPath = Join-Path $scriptDir 'Invoke-CentralRustValidation.ps1'
$validationPath = Join-Path $scriptDir 'ProductionCapture.Validation.ps1'
$sinkReconciliationPath = Join-Path $scriptDir 'verify_sink_reconciliation.py'
$attestationPath = Join-Path $scriptDir 'ProductionArtifact.Attestation.ps1'
$isolationPath = Join-Path $repoRoot 'extractor\documents\CLIENT_DRIVING_ISOLATION.md'
$lifecyclePath = Join-Path $scriptDir 'PassiveCapture.Lifecycle.ps1'
$hostCargoPath = Join-Path $repoRoot 'extractor\native\recorder_host\Cargo.toml'
$hostMainPath = Join-Path $repoRoot 'extractor\native\recorder_host\src\main.rs'
$earlyRenderLaunchPath = Join-Path $repoRoot 'extractor\native\recorder_host\src\early_render_launch.rs'
$autoAttachPath = Join-Path $repoRoot 'extractor\native\recorder_host\src\auto_attach.rs'
$workspaceCargoPath = Join-Path $repoRoot 'extractor\native\Cargo.toml'
$collectorCargoPath = Join-Path $repoRoot 'extractor\native\collector\Cargo.toml'
$characterLaneCargoPath = Join-Path $repoRoot 'extractor\native\lanes\character_presentation_live\Cargo.toml'
$focusLaneCargoPath = Join-Path $repoRoot 'extractor\native\lanes\focus_research_decisions_depth_live\Cargo.toml'
$collectorLibPath = Join-Path $repoRoot 'extractor\native\collector\src\lib.rs'
$passiveReadyPath = Join-Path $repoRoot 'extractor\native\compact_contract\src\passive_ready.rs'
$retiredLatchPath = Join-Path $repoRoot 'extractor\native\collector\src\hour_latch.rs'

. $attestationPath
. $interactionClosureHelperPath

# Failures are collected rather than thrown at the first hit. Throwing hid
# every case after the first one, which is how a case that could never pass sat
# in front of the rest of the file unnoticed; a closure test is only useful if
# it reports the whole state of the tree in one run.
$failedCases = [System.Collections.Generic.List[string]]::new()

function Assert-ClosureCase {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][bool]$Condition
    )
    if (-not $Condition) {
        $script:failedCases.Add($Name)
    }
}

$driver = Get-Content -LiteralPath $driverPath -Raw
$package = Get-Content -LiteralPath $packagePath -Raw
$packageVerifier = Get-Content -LiteralPath $packageVerifierPath -Raw
$interactionClosureHelper = Get-Content -LiteralPath $interactionClosureHelperPath -Raw
$promotionHelper = Get-Content -LiteralPath $promotionHelperPath -Raw
$runner = Get-Content -LiteralPath $runnerPath -Raw
$validation = Get-Content -LiteralPath $validationPath -Raw
$sinkReconciliation = Get-Content -LiteralPath $sinkReconciliationPath -Raw
$isolation = Get-Content -LiteralPath $isolationPath -Raw
$lifecycle = Get-Content -LiteralPath $lifecyclePath -Raw
$hostCargo = Get-Content -LiteralPath $hostCargoPath -Raw
$hostMain = Get-Content -LiteralPath $hostMainPath -Raw
$earlyRenderLaunch = Get-Content -LiteralPath $earlyRenderLaunchPath -Raw
$autoAttach = Get-Content -LiteralPath $autoAttachPath -Raw
$collectorCargo = Get-Content -LiteralPath $collectorCargoPath -Raw
$collectorLib = Get-Content -LiteralPath $collectorLibPath -Raw
$passiveReady = Get-Content -LiteralPath $passiveReadyPath -Raw
$cases = 0

    $fixedForbiddenMarkerCount = 121
    $fixedForbiddenMarkerRosterSha256 =
        '59dbc468c67c2abbe5fc8b7b30ae4f1136ce0c43e7429b11d339451c5feb1c4f'
$forbiddenMarkerRoster = @(Get-ForbiddenProductionInteractionMarkers)
Assert-ClosureCase 'forbidden interaction marker roster is the fixed reviewed census' (
    $forbiddenMarkerRoster.Count -eq $fixedForbiddenMarkerCount -and
    (Get-ProductionInteractionMarkerRosterSha256) -ceq
        $fixedForbiddenMarkerRosterSha256
)
$cases++

$driverForbidden = @(
    '--hour-latch',
    'Control-Hoi4Game.ps1',
    'WriteProcessMemory',
    '.Kill('
) + @(Get-ForbiddenProductionClientCapabilitySymbols)
foreach ($symbol in $driverForbidden) {
    Assert-ClosureCase "production driver excludes $symbol" (-not $driver.Contains($symbol))
}
$cases++

Assert-ClosureCase 'production driver authenticates exact READY through teardown' (
    $driver.Contains('PassiveCapture.Lifecycle.ps1') -and
    $driver.Contains('Test-PassiveCaptureReadyReceipt') -and
    $driver.Contains('Test-PassiveCaptureTeardownReceipt') -and
    $driver.Contains('CAPTURE_READY_V2.json') -and
    $driver.Contains('CAPTURE_TEARDOWN_V1.json') -and
    $driver.Contains('post_injection_termination_forbidden')
)
$cases++

Assert-ClosureCase 'shared lifecycle verifier closes hook and DLL authority' (
    $lifecycle.Contains('hoi4-capture-ready-v2') -and
    $lifecycle.Contains('hoi4-capture-teardown-v2') -and
    $lifecycle.Contains('$allowedHookCount') -and
    $lifecycle.Contains('$activeHookCount') -and
    $lifecycle.Contains('hook_lease_released_after_eof') -and
    $lifecycle.Contains('collector_module_disposition') -and
    $lifecycle.Contains('target_process_epoch_revalidated') -and
    $lifecycle.Contains('target_code_temporarily_instrumented') -and
    $lifecycle.Contains('pinned_hook_target_code_restored') -and
    $lifecycle.Contains('semantic_game_state_mutated') -and
    $lifecycle.Contains('pinned_hook_sites_restored') -and
    $lifecycle.Contains('pinned_hook_site_baseline_sha256') -and
    $lifecycle.Contains('pinned_hook_site_restored_sha256')
)
$cases++

Assert-ClosureCase 'production driver enables the read-only scheduler' (
    $driver.Contains('"--read-only-scheduler"') -and
    $driver.Contains('"--read-only-scheduler-poll-ms"') -and
    $driver.Contains('"--read-only-scheduler-queue-cap"')
)
$cases++

Assert-ClosureCase 'production driver receipt declares passive interaction closure' (
    $driver.Contains('interaction_closure') -and
    $driver.Contains('passive_injected_observer') -and
    $driver.Contains('client_commands_during_capture = 0')
)
$cases++

Assert-ClosureCase 'canonical-to-wire domain roster is independently artifact-bound' (
    $driver.Contains('production_domain_roster_authority.json') -and
    $driver.Contains('domain_roster_authority_sha256') -and
    $validation.Contains('domain_roster_authority_pair_mismatch') -and
    $validation.Contains('domain_roster_authority_sha256') -and
    $sinkReconciliation.Contains('domain_roster_authority_file_not_digest_bound') -and
    $sinkReconciliation.Contains('domain_roster_authority_not_exact_canonical_wire_join')
)
$cases++

$nativeRoot = Join-Path $repoRoot 'extractor\native'
$digestInputs = @(Get-ProductionInputFiles -Root $nativeRoot)
$resolvedDriver = (Resolve-Path -LiteralPath $driverPath).Path
Assert-ClosureCase 'production driver is digest-visible' (
    -not (Test-Hoi4ClientDrivingInputPath -RepoRoot $repoRoot -Path $resolvedDriver) -and
    @($digestInputs | Where-Object {
        [IO.Path]::GetFullPath($_) -ieq [IO.Path]::GetFullPath($resolvedDriver)
    }).Count -eq 1
)
$cases++

$resolvedWitness = (Resolve-Path -LiteralPath $readOnlyWitnessPath).Path
Assert-ClosureCase 'read-only game-state witness is digest-visible' (
    -not (Test-Hoi4ClientDrivingInputPath -RepoRoot $repoRoot -Path $resolvedWitness) -and
    @($digestInputs | Where-Object {
        [IO.Path]::GetFullPath($_) -ieq [IO.Path]::GetFullPath($resolvedWitness)
    }).Count -eq 1
)
$cases++

Assert-ClosureCase 'canonical and evidence recorder targets are feature-separated' (
    $hostCargo.Contains('name = "collector_record"') -and
    $hostCargo.Contains('name = "collector_record_evidence"') -and
    $hostCargo.Contains('required-features = ["evidence-client-driving"]') -and
    $hostMain.Contains('env!("CARGO_BIN_NAME") == "collector_record_evidence"') -and
    $hostMain.Contains('evidence-client-driving may only build collector_record_evidence') -and
    $hostMain.Contains('#[cfg(feature = "evidence-client-driving")]') -and
    $hostMain.Contains('"--early-launch-exe" =>') -and
    $earlyRenderLaunch.Contains('#[cfg(all(windows, feature = "evidence-client-driving"))]') -and
    $earlyRenderLaunch.Contains('mod windows {') -and
    $earlyRenderLaunch.Contains('fn CreateProcessW(') -and
    $earlyRenderLaunch.Contains('fn ResumeThread(') -and
    $earlyRenderLaunch.Contains('fn TerminateProcess(') -and
    $earlyRenderLaunch.Contains('#[cfg(all(windows, not(feature = "evidence-client-driving")))]') -and
    $earlyRenderLaunch.Contains('mod observer_only {') -and
    $earlyRenderLaunch.Contains('client-driving is physically excluded from the production observer binary') -and
    $earlyRenderLaunch.Contains('pub use observer_only::{')
)
$cases++

Assert-ClosureCase 'production auto-attach owns no target termination or job object' (
    -not $autoAttach.Contains('AssignProcessToJobObject') -and
    -not $autoAttach.Contains('SetInformationJobObject') -and
    -not $autoAttach.Contains('TerminateProcess') -and
    -not $autoAttach.Contains('.kill()') -and
    $autoAttach.Contains('cooperative stop')
)
$cases++

Assert-ClosureCase 'collector hour latch is physically retired' (
    -not (Test-Path -LiteralPath $retiredLatchPath) -and
    -not $collectorLib.Contains('mod hour_latch') -and
    $collectorLib.Contains('pub mod read_only_scheduler') -and
    $collectorLib.Contains('BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1')
)
$cases++

Assert-ClosureCase 'pinned hook roster is a digest-visible source-derived contract' (
    $passiveReady.Contains('pub const PINNED_PRODUCTION_HOOK_SPECS_V1: &[PinnedProductionHookSpecV1] = &[') -and
    $passiveReady.Contains(
        'pub const PINNED_PRODUCTION_HOOK_COUNT_V1: u16 = PINNED_PRODUCTION_HOOK_SPECS_V1.len() as u16;'
    ) -and
    # The recorder binds to the roster through the checked accessor, not the
    # bare constant: pinned_production_hook_count_v1() is what re-derives the
    # count from PINNED_PRODUCTION_HOOK_SPECS_V1 and can report a digest
    # failure. .Contains is ordinal and therefore case-sensitive, so asserting
    # the constant's spelling here could never have matched main.rs and was
    # asserting nothing.
    $hostMain.Contains('pinned_production_hook_count_v1()') -and
    $hostMain.Contains('pinned_production_hook_allowlist_sha256_v1()')
)
$cases++

$sourceBoundaryInputs = @(
    $hostCargoPath,
    $hostMainPath,
    $earlyRenderLaunchPath,
    $autoAttachPath,
    $workspaceCargoPath,
    $collectorCargoPath,
    $characterLaneCargoPath,
    $focusLaneCargoPath,
    $collectorLibPath,
    $passiveReadyPath,
    $lifecyclePath
)
foreach ($sourceBoundaryInput in $sourceBoundaryInputs) {
    $resolvedInput = (Resolve-Path -LiteralPath $sourceBoundaryInput).Path
    Assert-ClosureCase "source boundary is digest-visible: $resolvedInput" (
        @($digestInputs | Where-Object {
            [IO.Path]::GetFullPath($_) -ieq [IO.Path]::GetFullPath($resolvedInput)
        }).Count -eq 1
    )
}
$cases++

Assert-ClosureCase 'package requires both production markers' (
    $package.Contains('BATTLE_PLAN_OBSERVER_ONLY_RECORDER_V1') -and
    $package.Contains('BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1')
)
$cases++

foreach ($symbol in @(
    'BATTLE_PLAN_CLIENT_DRIVING_EVIDENCE_V1',
    '--early-launch-exe',
    'CREATE_SUSPENDED',
    'created_suspended',
    'primary_thread_resumed',
    'early_render_launch_manifest',
    'SendInput',
    'write_pause_state'
)) {
    Assert-ClosureCase "package rejects $symbol" (
        $package.Contains("'$symbol'") -and
        $package.Contains('contains forbidden client-driving or semantic-state-write symbol')
    )
}
$cases++

Assert-ClosureCase 'package forbids target control while retaining the injection seam' (
    $package.Contains('PermittedInjectionCapabilities') -and
    $package.Contains("'WriteProcessMemory'") -and
    $package.Contains("'CreateRemoteThread'") -and
    $package.Contains("'VirtualAllocEx'") -and
    $package.Contains('forbidden_target_client_capabilities') -and
    $package.Contains('Get-ForbiddenProductionClientCapabilitySymbols') -and
    $packageVerifier.Contains('Get-ForbiddenProductionClientCapabilitySymbols') -and
    $package.Contains('Get-ProductionSourceCapabilityClosure') -and
    $package.Contains('Assert-CollectorProductionDependencyFeatureClosure') -and
    $package.Contains('Assert-ProductionInteractionSourceTreeClosure') -and
    $package.Contains('collector production dependencies retain an unconditional fixture-only hook capability') -and
    $package.Contains('target lifecycle capability escapes the evidence-only module') -and
    $package.Contains('canonical_hook_name_families_closed = $true') -and
    $package.Contains('(?:^|\.)a(?:25|28|30)(?:\.|$)') -and
    $package.Contains('^presentation\.d82\.[a-z0-9_.-]+$') -and
    $package.Contains('^gameplay\.d[0-9]+\.[a-z0-9_.-]+$') -and
    $package.Contains('hoi4.production-source-capability-closure.v1') -and
    $package.Contains('pinned_(?:import|rva)!') -and
    $package.Contains('PINNED_PRODUCTION_HOOK_SPECS_V1.len() as u16') -and
    $package.Contains('pinned_hook_contract_source_sha256') -and
    $package.Contains("exact_pinned_hook_contract = 'a729.passive_ready.pinned_production_hook_specs.v1'")
)
$cases++

Assert-ClosureCase 'interaction helper structurally closes dependency features and shipped client APIs' (
    $interactionClosureHelper.Contains('Get-TomlLogicalAssignments') -and
    $interactionClosureHelper.Contains('Split-TomlDottedKey') -and
    $interactionClosureHelper.Contains('Get-TomlInlineTableFields') -and
    $interactionClosureHelper.Contains('Assert-CollectorProductionDependencyFeatureClosure') -and
    $interactionClosureHelper.Contains('a729-fixture-hook') -and
    $interactionClosureHelper.Contains('a729-focus-fixture-hook') -and
    $interactionClosureHelper.Contains('Get-ProductionInteractionSourceTreeAuthority') -and
    $interactionClosureHelper.Contains('Get-CollectorProductionLaneRoots') -and
    $interactionClosureHelper.Contains('Get-TomlProductionDependencyRecords') -and
    $interactionClosureHelper.Contains("'workspace.dependencies'") -and
    $interactionClosureHelper.Contains('System.Collections.Generic.Queue[string]') -and
    $interactionClosureHelper.Contains('[IO.FileAttributes]::ReparsePoint') -and
    $interactionClosureHelper.Contains('[IO.Path]::GetFullPath($earlyRenderEvidencePath)') -and
    $interactionClosureHelper.Contains('Assert-ProductionInteractionSourceTreeClosure') -and
    $interactionClosureHelper.Contains("'CreateProcessW'") -and
    $interactionClosureHelper.Contains("'TerminateProcess'") -and
    $interactionClosureHelper.Contains("'PostMessageW'") -and
    $interactionClosureHelper.Contains("'PostThreadMessageW'") -and
    $interactionClosureHelper.Contains("'SuspendThread'") -and
    $interactionClosureHelper.Contains("'ResumeThread'") -and
    $interactionClosureHelper.Contains("'AssignProcessToJobObject'") -and
    $interactionClosureHelper.Contains("'SetInformationJobObject'") -and
    $interactionClosureHelper.Contains("'QueryInformationJobObject'")
)
$cases++

Assert-ClosureCase 'code-mutation closure is keyed on behaviour, not on a vendor name' (
    $interactionClosureHelper.Contains('Get-ProductionThreadSuspensionEntryPoints') -and
    $interactionClosureHelper.Contains('Get-ProductionCodeMutationEntryPoints') -and
    $interactionClosureHelper.Contains('Get-ProductionCapabilityEvidence') -and
    $interactionClosureHelper.Contains('Get-ProductionCrateSourceCapabilityScan') -and
    $interactionClosureHelper.Contains('Get-ProductionDependencyCodeMutationClassification') -and
    $interactionClosureHelper.Contains('Get-ProductionSuspendingCodeMutationFindings') -and
    $interactionClosureHelper.Contains("'Wow64SuspendThread'") -and
    $interactionClosureHelper.Contains("'NtSuspendThread'") -and
    $interactionClosureHelper.Contains("'VirtualProtect'") -and
    $interactionClosureHelper.Contains("'VirtualProtectEx'") -and
    $interactionClosureHelper.Contains("'NtProtectVirtualMemory'") -and
    $interactionClosureHelper.Contains("'FlushInstructionCache'") -and
    # Every local spelling the fixed export can be reached under has to be
    # followed, or the roster is one rename away from being evadable again.
    $interactionClosureHelper.Contains('link_name') -and
    $interactionClosureHelper.Contains("'alias_call'") -and
    $interactionClosureHelper.Contains("'capability_reference'") -and
    $interactionClosureHelper.Contains('Test-ProductionExportResolverCall') -and
    # Run-time lookups are matched against lexed string literals, never the raw
    # text, so a comment that quotes an API name cannot fire the gate; and the
    # name has to be passed to something, so a diagnostic constant holding the
    # same text cannot either.
    $interactionClosureHelper.Contains('Get-RustStringLiteralValues') -and
    $interactionClosureHelper.Contains('Test-ProductionCallArgumentContext') -and
    # Declaring an ABI is not exercising it, in either language, and importing it
    # is not exercising it either.
    $interactionClosureHelper.Contains('Test-ProductionCapabilityDeclaration') -and
    $interactionClosureHelper.Contains('Get-RustImportItemRanges') -and
    # The vendor-name filter this replaced could be satisfied by moving the
    # same behaviour onto any other primitive, so it must not come back.
    -not $interactionClosureHelper.Contains("'minhook::'") -and
    -not $interactionClosureHelper.Contains("'MinHook::create_hook'") -and
    -not $interactionClosureHelper.Contains("'MH_CreateHook'")
)
$cases++

# This tree does perform thread-suspending code mutation: four lanes install
# detours into live game code while suspending the game's other threads, and
# every MinHook-backed lane delegates the same behaviour to a primitive that
# patches even when its suspend attempt failed. Those sites are pinned as an
# accepted inventory rather than asserted away, so the case fails when a NEW
# patcher appears, when a unit starts suspending, when a site moves to another
# file, or when a pinned site is gone. The roster is printed either way -
# accepting these sites is not the same as hiding them.
$suspendingCodeMutationFindings = Get-ProductionSuspendingCodeMutationFindings `
    -RepoRoot $repoRoot `
    -Authority (Get-ProductionInteractionSourceTreeAuthority -RepoRoot $repoRoot)
Write-Host (Format-ProductionSuspendingCodeMutationFindings `
    -Findings $suspendingCodeMutationFindings)
$measuredSuspendingBaseline = Get-ProductionSuspendingCodeMutationBaseline `
    -Findings $suspendingCodeMutationFindings
$suspendingBaselinePath =
    Get-ProductionSuspendingCodeMutationBaselinePath -RepoRoot $repoRoot
$suspendingBaselineDifferences = @(
    try {
        Compare-ProductionSuspendingCodeMutationBaseline `
            -Measured $measuredSuspendingBaseline `
            -Pinned (Read-ProductionSuspendingCodeMutationBaseline `
                -Path $suspendingBaselinePath)
    }
    catch {
        $_.Exception.Message
    }
)
foreach ($difference in $suspendingBaselineDifferences) {
    Write-Host "  baseline drift: $difference"
}
Assert-ClosureCase 'thread-suspending code mutation matches the accepted baseline' (
    $suspendingBaselineDifferences.Count -eq 0
)
$cases++

Assert-ClosureCase 'binary closure verdicts are computed from the complete fixed scan' (
    $package.Contains('foundForbiddenSymbols') -and
    $package.Contains('forbidden_symbol_roster_sha256') -and
    $package.Contains('forbidden_symbols_absent = $forbiddenSymbolsAbsent') -and
    $package.Contains('semantic_engine_state_writes = $semanticEngineStateWrites') -and
    $package.Contains('game_process_lifecycle_control = $gameProcessLifecycleControl') -and
    $package.Contains('game_ui_control = $gameUiControl') -and
    -not $package.Contains('semantic_engine_state_writes = $false') -and
    -not $package.Contains('game_process_lifecycle_control = $false') -and
    -not $package.Contains('game_ui_control = $false') -and
    $packageVerifier.Contains('foundForbiddenSymbols') -and
    $packageVerifier.Contains('forbidden_symbol_roster_sha256') -and
    $packageVerifier.Contains('forbidden_symbols_absent = $forbiddenSymbolsAbsent') -and
    -not $packageVerifier.Contains('semantic_engine_state_writes = $false') -and
    -not $packageVerifier.Contains('game_process_lifecycle_control = $false') -and
    -not $packageVerifier.Contains('game_ui_control = $false')
)
$cases++

Assert-ClosureCase 'package rechecks interaction closure on ZIP entry bytes' (
    $package.Contains('ExpectedInteractionClosures') -and
    $package.Contains('zip_entry_bytes_reverified = $true') -and
    $package.Contains('read_only_game_state_witness_sha256') -and
    $package.Contains('passive_capture_policy_test_sha256') -and
    $package.Contains('passive_capture_lifecycle_sha256') -and
    $package.Contains('recorder_host_early_render_launch_sha256') -and
    $package.Contains('passive_ready_contract_sha256')
)
$cases++

Assert-ClosureCase 'central Final and Release run source closure tests' (
    $runner.Contains('$productionInteractionClosureTest') -and
    $runner.Contains('battle-plan.production_interaction_closure_test.v1')
)
$cases++

Assert-ClosureCase 'central Release attests package interaction closure' (
    $runner.Contains('packageResult.interaction_closure') -and
    $runner.Contains('passive_injected_observer')
)
$cases++

Assert-ClosureCase 'production promotion requires real-source completeness for every domain' (
    $promotionHelper.Contains(
        'all_domains_real_source_complete = $allDomainsRealSourceComplete'
    ) -and
    $promotionHelper.Contains('per_domain_real_source_receipt') -and
    $promotionHelper.Contains('real_source_complete') -and
    $promotionHelper.Contains('zero_loss_evidence') -and
    $promotionHelper.Contains('speed5_evidence') -and
    $promotionHelper.Contains('Test-PassiveCaptureReadyReceipt') -and
    $promotionHelper.Contains('Test-PassiveCaptureTeardownReceipt') -and
    $promotionHelper.Contains('speed5_stream_gate') -and
    $promotionHelper.Contains('ProductionPromotionUnknownStatusMask') -and
    $promotionHelper.Contains('sample_count') -and
    $promotionHelper.Contains('870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad') -and
    $promotionHelper.Contains('status_flags_or') -and
    $promotionHelper.Contains('effective_failure_status_flags_or') -and
    $promotionHelper.Contains('data_record_count') -and
    $promotionHelper.Contains('production domain $($expectedDomains[$index].id) lacks real-source completeness') -and
    $validation.Contains('domain_${id}_raw_failure_status') -and
    $sinkReconciliation.Contains('CANONICAL_COMPACT_FAILURE_MASK') -and
    $sinkReconciliation.Contains('raw_failure_status') -and
    $packageVerifier.Contains('all-domain zero-loss speed-five promotion evidence') -and
    $package.Contains('production_capture_evidence = $productionCaptureEvidence')
)
$cases++

Assert-ClosureCase 'central Release emits no package until a bound speed-five capture is supplied' (
    $runner.Contains('[string]$ProductionCaptureReceiptPath') -and
    $runner.Contains('No portable package was emitted') -and
    $runner.Contains('Get-ProductionCapturePromotionEvidence') -and
    $package.Contains('admissible_for_production = ($null -ne $productionCaptureEvidence)') -and
    $packageVerifier.Contains('all-domain zero-loss speed-five promotion evidence')
)
$cases++

Assert-ClosureCase 'central Release rebuilds canonical injection artifacts with empty production features' (
    $runner.Contains('"--package", "recorder_host"') -and
    $runner.Contains('"--bin", "collector_record"') -and
    $runner.Contains('"--package", "collector"') -and
    $runner.Contains('"--lib"') -and
    $runner.Contains('"--no-default-features"') -and
    $runner.Contains("rebuild_order = @('recorder_host', 'collector_dll')") -and
    $runner.Contains("'unshipped-a25-runtime-text-evidence'") -and
    $runner.Contains("'unshipped-a39-character-presentation-evidence'") -and
    $runner.Contains("'unshipped-a31-focus-presentation-evidence'") -and
    $collectorCargo.Contains('unshipped-a25-runtime-text-evidence = []')
)
$cases++

Assert-ClosureCase 'central publication is canonically reverified and removed on failure' (
    $runner.Contains('$canonicalVerificationStdout') -and
    $runner.Contains('PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE') -and
    $runner.Contains('$finalPublishedSourceDigest') -and
    $runner.Contains('published portable package promotion inputs changed during canonical re-verification') -and
    $runner.Contains('-Paths @($artifactAttestationPath, $portablePackageReceiptPath, $portablePackagePath)')
)
$cases++

Assert-ClosureCase 'evidence client-driving is isolated from canonical package naming' (
    $runner.Contains('collector_record_evidence') -and
    $runner.Contains('evidence-client-driving') -and
    $runner.Contains('Evidence')
)
$cases++

Assert-ClosureCase 'isolation document binds injected observation and minimal hooks' (
    $isolation.Contains('Injected collection is allowed') -and
    $isolation.Contains('evidence-backed') -and
    $isolation.Contains('zero client mutation')
)
$cases++

if ($failedCases.Count -ne 0) {
    [pscustomobject]@{
        schema = 'battle-plan.production_interaction_closure_test.v1'
        status = 'FAIL'
        cases = $cases
        failed_cases = @($failedCases)
        driver = [IO.Path]::GetFullPath($driverPath)
        policy = 'passive_injected_observer'
    } | ConvertTo-Json -Depth 4
    throw (
        'production interaction-closure test failed: ' +
        (($failedCases | ForEach-Object { "`n  - $_" }) -join '')
    )
}

[pscustomobject]@{
    schema = 'battle-plan.production_interaction_closure_test.v1'
    status = 'PASS'
    cases = $cases
    driver = [IO.Path]::GetFullPath($driverPath)
    policy = 'passive_injected_observer'
} | ConvertTo-Json -Depth 4
