[CmdletBinding()]
param(
    # Regenerates the accepted thread-suspending code-mutation inventory from
    # what this tree currently measures, and does nothing else. Never passed by
    # Invoke-Validation.ps1: this is a human operation, it prints
    # every difference it accepts, and it refuses while any other closure case
    # is failing so it cannot be used to paper over unrelated breakage. The
    # drift message names it, because the drift it answers is otherwise a dead
    # end - see Get-ProductionSuspendingCodeMutationRepinInstruction.
    [switch]$AcceptMeasuredSuspendingBaseline
)

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
$runnerPath = Join-Path $scriptDir 'Invoke-ReleaseManual.ps1'
$validationPath = Join-Path $scriptDir 'ProductionCapture.Validation.ps1'
$sinkReconciliationPath = Join-Path $scriptDir 'verify_sink_reconciliation.py'
# Neutral, shared build machinery; see Invoke-ReleaseManual.ps1.
$attestationPath = Join-Path $repoRoot 'portable_replay\native\scripts\ProductionArtifact.Attestation.ps1'
$projectPath = Join-Path $repoRoot 'README.md'
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
$project = Get-Content -LiteralPath $projectPath -Raw
$lifecycle = Get-Content -LiteralPath $lifecyclePath -Raw
$hostCargo = Get-Content -LiteralPath $hostCargoPath -Raw
$hostMain = Get-Content -LiteralPath $hostMainPath -Raw
$earlyRenderLaunch = Get-Content -LiteralPath $earlyRenderLaunchPath -Raw
$autoAttach = Get-Content -LiteralPath $autoAttachPath -Raw
$collectorCargo = Get-Content -LiteralPath $collectorCargoPath -Raw
$collectorLib = Get-Content -LiteralPath $collectorLibPath -Raw
$passiveReady = Get-Content -LiteralPath $passiveReadyPath -Raw
$cases = 0

    # Repinned deliberately, three times. The roster gained the thread-handle
    # acquisition and consumption surface it was missing: the Zw
    # suspend/resume/terminate aliases, Nt/Zw SetContextThread, the alert and
    # alert-resume pairs, Nt/Zw SetInformationThread and ImpersonateThread, the
    # QueueUserAPC2 and QueueApcThreadEx2 forms, SetThreadToken,
    # SetThreadInformation, CancelSynchronousIo, the Nt/Zw OpenThread pair, and
    # the scheduling setters. NtOpenThread in particular meant the ban on
    # obtaining another thread's handle had never been in force at all, since
    # only the Win32 spelling was listed.
    #
    # Each of three review rounds found more of this surface, so expect this
    # census to move again. It is a census, not a proof that the thread-control
    # surface is closed. Two files pin it and both have to move together.
    $fixedForbiddenMarkerCount = 147
    $fixedForbiddenMarkerRosterSha256 =
        '3068b0722bf931718c471cbeb998998b05ff9cca2ae72a4ecb2ce8a9a8bcf617'
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

# These assertions matched each symbol's own source text inside
# New-PortableProductPackage.ps1. The roster lives in the shared helper
# (Get-ForbiddenProductionInteractionMarkers) and the enforcers consume it from
# there, so the text match reported a missing check that was in fact live in
# both of them. Matching the roster's own source text would only have re-created
# the same fragility, so membership is now read from the roster by calling it,
# and the two enforcers are cited by the roster call and the refusal they raise.
#
# Honest limits of this case. Membership is dynamic and case-sensitive, which
# the old text match was not. The link from that roster to each enforcer's byte
# scan is still only a whole-file text match, so an enforcer that kept the call
# and the message while scanning a filtered collection would pass here. Closing
# that needs the enforcers to expose their scanned roster for comparison rather
# than being read as text. Provenance note: this repo's history is squashed, so
# when the inline spelling was dropped is not recoverable from git; the claim
# above is about where the check is now, which is directly checkable.
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
        $forbiddenMarkerRoster -ccontains $symbol -and
        $package.Contains('Get-ForbiddenProductionInteractionMarkers') -and
        $package.Contains('contains forbidden client-driving or semantic-state-write symbol') -and
        $packageVerifier.Contains('Get-ForbiddenProductionInteractionMarkers') -and
        $packageVerifier.Contains('contains forbidden client-driving or semantic-state-write symbol')
    )
}
$cases++

Assert-ClosureCase 'package forbids target control while retaining the injection seam' (
    $package.Contains('PermittedInjectionCapabilities') -and
    $package.Contains("'WriteProcessMemory'") -and
    $package.Contains("'CreateRemoteThread'") -and
    $package.Contains("'VirtualAllocEx'") -and
    $package.Contains('forbidden_target_client_capabilities') -and
    # Neither the union getter nor its two halves is named in either enforcer
    # any more. None of them is gone: Get-ForbiddenProductionInteractionMarkers
    # calls the union, the union calls both halves, and that roster is the one
    # both enforcers scan - so dropping the separately reported lifecycle and UI
    # verdicts took nothing out of what is scanned.
    #
    # Reachability, not spelling: every client-capability symbol must be inside
    # the roster, which a citation of the getter's name would not have shown -
    # that passes on the definition alone even if nothing calls it. This proves
    # union output is a subset of roster output. That roster output reaches the
    # byte scan is proved by running the enforcers below, where one symbol from
    # each half is rejected by both.
    # It is also a subset test on two values from the same helper, so an edit
    # that drops a symbol from the half-getter, from the roster, and from the
    # pinned count/SHA together satisfies it - the SHA pin catches unaccompanied
    # drift, not a deliberate repin, which stays a review responsibility.
    @(Get-ForbiddenProductionClientCapabilitySymbols | Where-Object {
        $forbiddenMarkerRoster -cnotcontains $_
    }).Count -eq 0 -and
    $package.Contains('Get-ForbiddenProductionInteractionMarkers') -and
    $packageVerifier.Contains('Get-ForbiddenProductionInteractionMarkers') -and
    $package.Contains('Get-ProductionSourceCapabilityClosure') -and
    $package.Contains('Assert-CollectorProductionDependencyFeatureClosure') -and
    $package.Contains('Assert-ProductionInteractionSourceTreeClosure') -and
    $package.Contains('Assert-CollectorRetiredEvidenceFeatureAbsence') -and
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

# The manifest guards are a NAME filter: re-adding the installer behind a
# differently spelled feature satisfies all of them. What makes the capability
# unreachable is that the function is not in the source, so that is asserted
# against the real lane sources, and the guard is exercised in both directions.
$laneFixtureSources = [ordered]@{
    character_presentation_live_lane =
        Join-Path $repoRoot 'extractor\native\lanes\character_presentation_live\src\a729_hook.rs'
    focus_research_decisions_depth_live =
        Join-Path $repoRoot 'extractor\native\lanes\focus_research_decisions_depth_live\src\a729_focus_item_hook.rs'
}
$laneFixtureAbsenceHolds = $true
foreach ($laneEntry in $laneFixtureSources.GetEnumerator()) {
    $laneSource = Get-Content -LiteralPath $laneEntry.Value -Raw
    try {
        Assert-LaneFixtureInstallerSourceAbsence `
            -LaneSource $laneSource -LaneName ([string]$laneEntry.Key)
    }
    catch {
        $laneFixtureAbsenceHolds = $false
    }
    # And it is not vacuous: restoring the deleted signature is refused.
    $restored = $laneSource + "`npub fn install_for_fixture_capture() {}`n"
    $restoredRefused = $false
    try {
        Assert-LaneFixtureInstallerSourceAbsence `
            -LaneSource $restored -LaneName ([string]$laneEntry.Key)
    }
    catch {
        $restoredRefused = $true
    }
    if (-not $restoredRefused) { $laneFixtureAbsenceHolds = $false }
}
Assert-ClosureCase 'retired lane fixture-hook installers are absent from lane sources' (
    $laneFixtureAbsenceHolds
)
$cases++

Assert-ClosureCase 'interaction helper structurally closes dependency features and shipped client APIs' (
    $interactionClosureHelper.Contains('Get-TomlLogicalAssignments') -and
    $interactionClosureHelper.Contains('Split-TomlDottedKey') -and
    $interactionClosureHelper.Contains('Get-TomlInlineTableFields') -and
    $interactionClosureHelper.Contains('Assert-CollectorProductionDependencyFeatureClosure') -and
    $interactionClosureHelper.Contains('Assert-CollectorRetiredEvidenceFeatureAbsence') -and
    $interactionClosureHelper.Contains('Assert-LaneFixtureInstallerSourceAbsence') -and
    $interactionClosureHelper.Contains("StartsWith('unshipped-'") -and
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

# Everything below reads only the files the lane-root derivation admits, so a
# lane that quietly leaves that derivation does not make any case here fail -
# it makes them all pass over a smaller tree. The set is therefore derived a
# second time from cargo's own resolution of the workspace (Cargo.lock, which
# this repository's PowerShell does not write) and the two have to name the
# same lanes. This is a coverage assertion, not a capability one: it says the
# gate is still looking at what production links, and says nothing about what
# it finds there.
$laneRootCoverageDifferences = @(
    try {
        Get-CollectorProductionLaneRootCoverageDifferences -RepoRoot $repoRoot
    }
    catch {
        $_.Exception.Message
    }
)
foreach ($difference in $laneRootCoverageDifferences) {
    [Console]::Error.WriteLine("  lane root coverage: $difference")
}
Assert-ClosureCase 'scanned lane roots are the lanes the resolved collector links' (
    $laneRootCoverageDifferences.Count -eq 0
)
$cases++

# This tree does perform thread-suspending code mutation: four lanes rewrite
# live game code themselves while every other thread in the process is stopped,
# and every MinHook-backed lane delegates the same behaviour to a primitive that
# patches even when its suspend attempt failed. The four no longer stop those
# threads with their own SuspendThread - they take the freeze from the shared
# inline_detour crate - and the operation is unchanged, so the finding has to
# compose the freeze in the dependency with the patch in the unit. It did not,
# and all four dropped out of this inventory the day they migrated.
#
# Those sites are pinned as an accepted inventory rather than asserted away, so
# the case fails when a NEW patcher appears, when a unit starts suspending, when
# a site moves to another file, or when a pinned site is gone. The roster is
# printed either way - accepting these sites is not the same as hiding them.
#
# The measurement itself is fenced. Get-ProductionInteractionSourceTreeAuthority
# walks the production lane roots, and a lane directory that has just been
# purged makes it throw - which killed this whole script with a raw PowerShell
# exception, before any later case ran and before anything named the pin that
# needed regenerating. An inventory that cannot be MEASURED is a failed case
# with a name, exactly like an inventory that disagrees with the pin.
$suspendingRepinInstruction =
    Get-ProductionSuspendingCodeMutationRepinInstruction -RepoRoot $repoRoot
$suspendingCodeMutationFindings = $null
$suspendingMeasurementFailure = ''
try {
    $suspendingCodeMutationFindings = Get-ProductionSuspendingCodeMutationFindings `
        -RepoRoot $repoRoot `
        -Authority (Get-ProductionInteractionSourceTreeAuthority -RepoRoot $repoRoot)
}
catch {
    $suspendingMeasurementFailure = [string]$_.Exception.Message
}
# On stderr, not stdout. This script's stdout is a single JSON document and
# Release validation consumes this script's JSON result; printing the roster to
# stdout would put prose in front of it. The roster is still
# printed - that is the point of the comment above - and a caller that wants it
# in a log redirects 2>&1, which is what the deployment-acceptance wrapper
# already does.
$suspendingBaselinePath =
    Get-ProductionSuspendingCodeMutationBaselinePath -RepoRoot $repoRoot
$measuredSuspendingBaseline = $null
if ($suspendingMeasurementFailure) {
    $suspendingBaselineDifferences = @(
        "the inventory could not be measured at all: $suspendingMeasurementFailure"
    )
}
else {
    [Console]::Error.WriteLine((Format-ProductionSuspendingCodeMutationFindings `
        -Findings $suspendingCodeMutationFindings))
    $measuredSuspendingBaseline = Get-ProductionSuspendingCodeMutationBaseline `
        -Findings $suspendingCodeMutationFindings
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
}
foreach ($difference in $suspendingBaselineDifferences) {
    [Console]::Error.WriteLine("  baseline drift: $difference")
}
if ($suspendingBaselineDifferences.Count -ne 0) {
    # The instruction goes next to the drift, not only into the throw at the
    # bottom: -Mode Final runs this script as a child and reports only its exit
    # code, so the stderr the operator actually reads has to carry the way out.
    [Console]::Error.WriteLine("  $suspendingRepinInstruction")
}
$suspendingBaselineCase = 'thread-suspending code mutation matches the accepted baseline'
Assert-ClosureCase $suspendingBaselineCase (
    $suspendingBaselineDifferences.Count -eq 0
)
$cases++

Assert-ClosureCase 'binary closure verdicts are computed from the complete fixed scan' (
    $package.Contains('foundForbiddenSymbols') -and
    $package.Contains('forbidden_symbol_roster_sha256') -and
    $package.Contains('forbidden_symbols_absent = $forbiddenSymbolsAbsent') -and
    $package.Contains('semantic_engine_state_writes = $semanticEngineStateWrites') -and
    -not $package.Contains('semantic_engine_state_writes = $false') -and
    $packageVerifier.Contains('foundForbiddenSymbols') -and
    $packageVerifier.Contains('forbidden_symbol_roster_sha256') -and
    $packageVerifier.Contains('forbidden_symbols_absent = $forbiddenSymbolsAbsent') -and
    -not $packageVerifier.Contains('semantic_engine_state_writes = $false')
)
$cases++

# The two enforcers used to publish game_process_lifecycle_control and
# game_ui_control alongside the verdicts above. Neither could ever be true on a
# returned receipt: both rosters are subsets of the interaction-marker roster,
# a permitted role capability is filtered out of both by the same expression,
# and every remaining hit throws before the receipt is built. A reader who saw
# them read "checked, and clean" where nothing was left to check. The cases
# below run the shipped enforcer functions - lifted out with the PowerShell
# parser, not re-implemented - so the removal is held to what the enforcers do,
# not to what their source text says.
function New-InteractionEnforcerProbe {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$EnforcerName,
        [Parameter(Mandatory = $true)][string]$HelperPath
    )

    $tokens = $null
    $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile(
        $Path,
        [ref]$tokens,
        [ref]$parseErrors
    )
    if (@($parseErrors).Count -ne 0) {
        throw "interaction-closure enforcer source does not parse: $Path"
    }
    $lifted = [System.Collections.Generic.List[string]]::new()
    foreach ($name in @('Find-AsciiByteSequence', $EnforcerName)) {
        $definitions = @($ast.FindAll(
            {
                param($node)
                $node -is [System.Management.Automation.Language.FunctionDefinitionAst]
            },
            $true
        ) | Where-Object { [string]$_.Name -ceq $name })
        if ($definitions.Count -ne 1) {
            throw "interaction-closure enforcer does not define exactly one $name`: $Path"
        }
        $lifted.Add("function $name $($definitions[0].Body.Extent.Text)")
    }
    $moduleText = @(
        ". '$HelperPath'"
        $lifted
        @'
function Invoke-InteractionEnforcer {
    param(
        [byte[]]$Bytes,
        [string[]]$RequiredMarkers,
        [string[]]$PermittedSelfUiCapabilities,
        [string[]]$PermittedSelfWorkerCapabilities
    )
    return ENFORCER_NAME `
        -Bytes $Bytes `
        -Name 'enforcer probe' `
        -RequiredMarkers $RequiredMarkers `
        -ExpectedPinnedHookCount ([uint16]1) `
        -PinnedHookContractSourceSha256 ('0' * 64) `
        -PermittedSelfUiCapabilities $PermittedSelfUiCapabilities `
        -PermittedSelfWorkerCapabilities $PermittedSelfWorkerCapabilities
}
'@.Replace('ENFORCER_NAME', $EnforcerName)
    ) -join [Environment]::NewLine
    return (New-Module -ScriptBlock ([scriptblock]::Create($moduleText)) -AsCustomObject)
}

function New-InteractionEnforcerBytes {
    param([string[]]$Texts)

    $encoded = [System.Collections.Generic.List[byte]]::new()
    foreach ($text in $Texts) {
        $encoded.AddRange([Text.Encoding]::ASCII.GetBytes($text))
        $encoded.Add(0)
    }
    return ,$encoded.ToArray()
}

$enforcerProbes = [ordered]@{
    builder = New-InteractionEnforcerProbe `
        -Path $packagePath `
        -EnforcerName 'Assert-ProductionInteractionClosure' `
        -HelperPath $interactionClosureHelperPath
    verifier = New-InteractionEnforcerProbe `
        -Path $packageVerifierPath `
        -EnforcerName 'Assert-VerifiedInteractionClosure' `
        -HelperPath $interactionClosureHelperPath
}
$enforcerMarkers = @(
    'BATTLE_PLAN_OBSERVER_ONLY_RECORDER_V1',
    'BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1'
)
$enforcerCleanBytes = New-InteractionEnforcerBytes -Texts $enforcerMarkers
$enforcerReceipts = [ordered]@{}
foreach ($probe in $enforcerProbes.GetEnumerator()) {
    $enforcerReceipts[[string]$probe.Key] = $probe.Value.'Invoke-InteractionEnforcer'(
        $enforcerCleanBytes,
        $enforcerMarkers,
        @(),
        @()
    )
}
Assert-ClosureCase 'both package enforcers admit an interaction-clean artifact' (
    @($enforcerReceipts.Keys).Count -eq 2 -and
    ([string]$enforcerReceipts['builder'].schema -ceq 'hoi4.interaction-closure.v1') -and
    ($enforcerReceipts['builder'].forbidden_symbols_absent -is [bool]) -and
    $enforcerReceipts['builder'].forbidden_symbols_absent -and
    (($enforcerReceipts['builder'] | ConvertTo-Json -Depth 5 -Compress) -ceq
        ($enforcerReceipts['verifier'] | ConvertTo-Json -Depth 5 -Compress))
)
$cases++

Assert-ClosureCase 'no package enforcer publishes a lifecycle or UI control verdict' (
    @(@($enforcerReceipts.Keys) | Where-Object {
        @($enforcerReceipts[$_].Keys) -ccontains 'game_process_lifecycle_control' -or
        @($enforcerReceipts[$_].Keys) -ccontains 'game_ui_control'
    }).Count -eq 0 -and
    -not $package.Contains('game_process_lifecycle_control') -and
    -not $package.Contains('game_ui_control') -and
    -not $packageVerifier.Contains('game_process_lifecycle_control') -and
    -not $packageVerifier.Contains('game_ui_control') -and
    -not $runner.Contains('game_process_lifecycle_control') -and
    -not $runner.Contains('game_ui_control')
)
$cases++

# replay_app is required to carry no linker-retained production marker, and the
# package passes that empty roster explicitly. A Mandatory parameter rejects an
# empty collection unless it says otherwise, so without AllowEmptyCollection
# neither enforcer could be called for the replay role at all - which nothing
# noticed, because every other case here hands them a non-empty roster.
$enforcerEmptyMarkerReceipts = [ordered]@{}
foreach ($probe in $enforcerProbes.GetEnumerator()) {
    $enforcerEmptyMarkerReceipts[[string]$probe.Key] =
        $probe.Value.'Invoke-InteractionEnforcer'(
            $enforcerCleanBytes,
            @(),
            @(),
            @()
        )
}
Assert-ClosureCase 'both package enforcers accept the empty required-marker roster' (
    @($enforcerEmptyMarkerReceipts.Keys).Count -eq 2 -and
    @(@($enforcerEmptyMarkerReceipts.Keys) | Where-Object {
        @($enforcerEmptyMarkerReceipts[$_].required_markers).Count -ne 0 -or
        -not $enforcerEmptyMarkerReceipts[$_].forbidden_symbols_absent
    }).Count -eq 0
)
$cases++

# "It threw" is not the assertion. A binder error, a helper that stopped
# existing, or any other accident would satisfy that while the forbidden-symbol
# path itself was gone, so each rejection has to be the exact throw, naming the
# exact symbol that was planted.
$enforcerRejections = [ordered]@{}
foreach ($probe in $enforcerProbes.GetEnumerator()) {
    foreach ($forbidden in @('TerminateProcess', 'SetCursorPos', 'CREATE_SUSPENDED')) {
        $rejection = ''
        try {
            [void]$probe.Value.'Invoke-InteractionEnforcer'(
                (New-InteractionEnforcerBytes -Texts (@($enforcerMarkers) + @($forbidden))),
                $enforcerMarkers,
                @(),
                @()
            )
        }
        catch {
            # The enforcer runs as a module method, so PowerShell wraps its
            # throw in a MethodInvocationException. Unwrap to the exception the
            # enforcer actually raised.
            $raised = $_.Exception
            while ($null -ne $raised.InnerException) {
                $raised = $raised.InnerException
            }
            $rejection = [string]$raised.Message
        }
        $enforcerRejections["$($probe.Key)/$forbidden"] = $rejection
    }
}
Assert-ClosureCase 'both package enforcers still throw on a forbidden symbol' (
    @($enforcerRejections.Keys).Count -eq 6 -and
    @(@($enforcerRejections.Keys) | Where-Object {
        [string]$enforcerRejections[$_] -cne (
            'enforcer probe contains forbidden client-driving or ' +
            "semantic-state-write symbol $(($_ -split '/')[1])"
        )
    }).Count -eq 0
)
$cases++

$enforcerPermittedReceipts = [ordered]@{}
foreach ($probe in $enforcerProbes.GetEnumerator()) {
    $enforcerPermittedReceipts[[string]$probe.Key] =
        $probe.Value.'Invoke-InteractionEnforcer'(
            (New-InteractionEnforcerBytes -Texts (
                @($enforcerMarkers) + @('SendMessageW', 'CreateProcessW')
            )),
            $enforcerMarkers,
            @('SendMessageW'),
            @('CreateProcessW')
        )
}
Assert-ClosureCase 'a permitted role capability is admitted by both enforcers and named once' (
    @($enforcerPermittedReceipts.Keys).Count -eq 2 -and
    @(@($enforcerPermittedReceipts.Keys) | Where-Object {
        (@($enforcerPermittedReceipts[$_].permitted_self_ui_capabilities) -join '|') -cne
            'SendMessageW' -or
        (@($enforcerPermittedReceipts[$_].permitted_self_worker_capabilities) -join '|') -cne
            'CreateProcessW' -or
        @($enforcerPermittedReceipts[$_].forbidden_target_client_capabilities) -ccontains
            'SendMessageW' -or
        @($enforcerPermittedReceipts[$_].forbidden_target_client_capabilities) -ccontains
            'CreateProcessW'
    }).Count -eq 0
)
$cases++

Assert-ClosureCase 'builder and receipt verifier enforce the canonical package documents' (
    $package.Contains("PortableProduct.PackageContents.ps1") -and
    $package.Contains('Get-PortableProductPackageDocuments -Version $Version -BinaryHashes $binaryHashes') -and
    $package.Contains('Assert-PortableProductPackageContents -Archive $archive -Version $Version -BinaryHashes $ExpectedBinaryHashes') -and
    $packageVerifier.Contains("PortableProduct.PackageContents.ps1") -and
    $packageVerifier.Contains('Assert-PortableProductPackageContents -Archive $archive -Version $version -BinaryHashes $binaryHashes')
)
$cases++
$packageContentsOutput = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $scriptDir 'Test-PortableProductPackageContents.ps1'))
$packageContentsExit = $LASTEXITCODE
try {
    $packageContentsReceipt = ($packageContentsOutput -join "`n") | ConvertFrom-Json
    Assert-ClosureCase 'package document and negative ZIP behavior passes' (
        $packageContentsExit -eq 0 -and
        [string]$packageContentsReceipt.schema -ceq 'battle-plan.portable_package_contents_test.v1' -and
        [string]$packageContentsReceipt.status -ceq 'PASS' -and
        [int]$packageContentsReceipt.cases -ge 24
    )
}
catch { Assert-ClosureCase 'package document tests return a structured receipt' $false }
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

Assert-ClosureCase 'ReleaseManual runs the landing source validators' (
    $runner.Contains('& $validation -Mode Landing -AllValidators')
)
$cases++

Assert-ClosureCase 'ReleaseManual attests the package interaction closure' (
    $runner.Contains('$package.interaction_closure.source_capability') -and
    $runner.Contains('package = $package')
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

Assert-ClosureCase 'ReleaseManual emits no package until a bound speed-five capture is supplied' (
    $runner.Contains('[string]$ProductionCaptureReceiptPath') -and
    $runner.Contains('No package was emitted because no production capture receipt was supplied.') -and
    $runner.Contains('-ProductionCaptureReceiptPath $ProductionCaptureReceiptPath') -and
    $package.Contains('Get-ProductionCapturePromotionEvidence') -and
    $package.Contains('admissible_for_production = ($null -ne $productionCaptureEvidence)') -and
    $packageVerifier.Contains('all-domain zero-loss speed-five promotion evidence')
)
$cases++

Assert-ClosureCase 'ReleaseManual rebuilds canonical injection artifacts with empty production features' (
    $runner.Contains("'-p', 'recorder_host'") -and
    $runner.Contains("'--bin', 'collector_record'") -and
    $runner.Contains("'-p', 'collector'") -and
    $runner.Contains("'--lib', '--no-default-features'") -and
    $runner.Contains("rebuild_order = @('recorder_host', 'collector_dll')") -and
    $runner.Contains("forbidden_features = @(") -and
    $runner.Contains("'test-support'") -and
    # The three `unshipped-*` names the roster used to carry are deleted along
    # with the fixture hooks they gated, so the roster must NOT name them and
    # the collector must not declare them. `Assert-CollectorRetiredEvidence
    # FeatureAbsence` refuses the prefix in the manifest; this refuses it in the
    # ReleaseManual contract that pins what the artifact was built without.
    -not $runner.Contains("'unshipped-") -and
    -not $collectorCargo.Contains('unshipped-')
)
$cases++

Assert-ClosureCase 'release publication is verified before and after publication' (
    $runner.Contains("-ReceiptPath', `$candidate, '-AllowStagedReceipt'") -and
    $runner.Contains('PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE') -and
    $runner.Contains('[IO.File]::Move($candidate, $packageReceiptPath)') -and
    $runner.Contains("-ReceiptPath', `$packageReceiptPath") -and
    $runner.Contains('$releaseOutputs = @($attestationPath, $packageReceiptPath, $packagePath)') -and
    $runner.Contains('Remove-ExactFiles -Paths $releaseOutputs')
)
$cases++

Assert-ClosureCase 'evidence client-driving is isolated from canonical package naming' (
    -not $runner.Contains('collector_record_evidence') -and
    -not $runner.Contains('evidence-client-driving') -and
    $hostCargo.Contains('collector_record_evidence') -and
    $hostCargo.Contains('evidence-client-driving')
)
$cases++

# 'zero client mutation' was cited here but appears nowhere in the document, and
function Test-ProjectPassiveRecordingContract {
    param([string]$Text)
    $visible = [regex]::Replace($Text, '(?s)<!--.*?(?:-->|$)', '')
    $lines = @($visible -split '\r?\n' | ForEach-Object { $_.Trim() })
    # These are the current independent recording and injection requirements,
    # not isolated keywords which unrelated prose could accidentally satisfy.
    return (
        $lines -ccontains '- Recording never pauses, slows, or controls the game. It never drives the UI, input, or focus, never freezes the game to take a snapshot, and never makes the game wait on the recorder. Running at maximum speed and auto-pausing are normal play.' -and
        $lines -ccontains '- The collector only reads. Hooking the game is a last resort, used only when plain reading cannot get a value, and every hook is registered, bounded, and restored byte-for-byte on exit, including bootstrap hooks.'
    )
}

# README.md owns the durable passive-recording boundary. Pin its independent
# client-control and teardown clauses so deleting a retired design document
# cannot weaken this closure test.
Assert-ClosureCase 'README binds passive observation and restored hooks' (
    (Test-ProjectPassiveRecordingContract -Text $project)
)
$cases++

# Each removal weakens a distinct contract requirement, even if the words are
# retained elsewhere in the document. Comments must not supply admission.
$projectMutationFailures = @()
foreach ($clause in @(
    'never pauses', 'slows, ', 'or controls the game', 'drives the UI, input, or focus',
    'never freezes the game to take a snapshot', 'never makes the game wait on the recorder',
    'The collector only reads. ', 'last resort', 'only when plain reading cannot get a value',
    'registered, bounded, and ', 'restored byte-for-byte on exit, including bootstrap hooks'
)) {
    $mutated = $project.Replace($clause, '') + "`n<!-- $clause -->"
    if ($mutated -ceq $project -or (Test-ProjectPassiveRecordingContract -Text $mutated)) {
        $projectMutationFailures += $clause
    }
}
$requiredBullets = @($project -split '\r?\n' | Where-Object {
    $_.StartsWith('- Recording never pauses,') -or $_.StartsWith('- The collector only reads.')
})
if ($requiredBullets.Count -ne 2) { $projectMutationFailures += 'missing required bullets' }
foreach ($bullet in $requiredBullets) {
    foreach ($replacement in @("<!--`n$bullet`n-->", "<!--`n$bullet")) {
        if (Test-ProjectPassiveRecordingContract -Text ($project.Replace($bullet, $replacement))) {
            $projectMutationFailures += 'commented-out required bullet'
        }
    }
}
Assert-ClosureCase 'README passive contract rejects each actual clause withdrawal' (
    $projectMutationFailures.Count -eq 0
)
$cases++

# The regeneration path. Deliberately at the very bottom: it can only run once
# every other case has had its verdict, so it is impossible to repin a tree that
# is failing for some other reason.
if ($AcceptMeasuredSuspendingBaseline) {
    $unrelated = @($failedCases | Where-Object { $_ -cne $suspendingBaselineCase })
    if ($unrelated.Count -ne 0) {
        throw (
            'refusing to accept a measured suspending-code-mutation inventory while ' +
            'other interaction-closure cases are failing: ' +
            (($unrelated | ForEach-Object { "`n  - $_" }) -join '')
        )
    }
    if ($suspendingMeasurementFailure) {
        throw (
            'refusing to accept an inventory that could not be measured: ' +
            $suspendingMeasurementFailure
        )
    }
    if ($suspendingBaselineDifferences.Count -eq 0) {
        [Console]::Error.WriteLine(
            'accepted suspending code mutation baseline already matches this tree; nothing written'
        )
    }
    else {
        foreach ($difference in $suspendingBaselineDifferences) {
            [Console]::Error.WriteLine("  accepting: $difference")
        }
        Write-ProductionSuspendingCodeMutationBaseline `
            -Path $suspendingBaselinePath `
            -Measured $measuredSuspendingBaseline
    }
    [pscustomobject]@{
        schema = 'battle-plan.production_interaction_closure_test.v1'
        status = 'REPINNED'
        cases = $cases
        accepted_differences = @($suspendingBaselineDifferences)
        baseline = [IO.Path]::GetFullPath($suspendingBaselinePath)
        driver = [IO.Path]::GetFullPath($driverPath)
        policy = 'passive_injected_observer'
    } | ConvertTo-Json -Depth 4
    # REPINNED is not PASS, and Invoke-Validation.ps1 admits only
    # PASS. Re-run the gate without this switch: the accepted inventory is a
    # review decision, and the run that certifies the tree must be one that
    # could still have failed.
    return
}

if ($failedCases.Count -ne 0) {
    $remediation = @()
    if ($failedCases -ccontains $suspendingBaselineCase) {
        $remediation = @($suspendingRepinInstruction)
    }
    [pscustomobject]@{
        schema = 'battle-plan.production_interaction_closure_test.v1'
        status = 'FAIL'
        cases = $cases
        failed_cases = @($failedCases)
        baseline_drift = @($suspendingBaselineDifferences)
        remediation = @($remediation)
        driver = [IO.Path]::GetFullPath($driverPath)
        policy = 'passive_injected_observer'
    } | ConvertTo-Json -Depth 4
    throw (
        'production interaction-closure test failed: ' +
        (($failedCases | ForEach-Object { "`n  - $_" }) -join '') +
        (($suspendingBaselineDifferences | ForEach-Object { "`n    drift: $_" }) -join '') +
        $(if ($remediation.Count -ne 0) { "`n  $suspendingRepinInstruction" } else { '' })
    )
}

[pscustomobject]@{
    schema = 'battle-plan.production_interaction_closure_test.v1'
    status = 'PASS'
    cases = $cases
    driver = [IO.Path]::GetFullPath($driverPath)
    policy = 'passive_injected_observer'
} | ConvertTo-Json -Depth 4
