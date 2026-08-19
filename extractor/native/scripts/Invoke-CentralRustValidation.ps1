[CmdletBinding()]
param(
    [ValidateSet("Check", "Build", "Fast", "Integration", "Test", "Final", "Soak", "Evidence", "Release", "Lock", "Stats", "Iterate")]
    [string]$Mode = "Final",

    [ValidateSet("All", "Extractor", "Replay")]
    [string]$Workspace = "All",

    [string[]]$Package = @(),

    # The base this lane branched from, for -Mode Iterate. The diff against it,
    # unioned with the working tree, is what the iteration selection is derived
    # from. It is REQUIRED rather than defaulted: a wrong base is a silently
    # narrower run, and this runner does not guess at what a lane's base was.
    #
    # THE OWNER'S RULING that funds this mode (2026-08-15): "lanes run only the
    # tests their diff can break during iteration; the full battery stays at
    # landings and nightly/CI". Iterate is therefore explicitly NOT a landing
    # gate, and it says so on every run.
    [string]$Since = "",

    [string]$FilterExpression = "",

    # Cargo features to switch on for a targeted Check/Build/Test run, spelled
    # exactly as cargo takes them (`package/feature` across a workspace).
    #
    # WHY THIS EXISTS: `Final` builds the workspace with default features, so a
    # module behind an off-by-default feature is never compiled by the battery
    # and its tests never run — a green Final says nothing about it. Lane HZ hit
    # that with `replay_decoder/dq-v3-prototype`: production edits and two new
    # tests sat in a module the complete battery could not see. A targeted
    # feature-enabled run is what closes that, and it belongs in the narrow
    # deterministic tiers rather than in `Final`, which must stay one canonical
    # configuration.
    [string[]]$Features = @(),

    [switch]$Lib,

    [string[]]$Bin = @(),

    [string[]]$TestTarget = @(),

    [switch]$AllTargets,

    [ValidateSet("Iterative", "Reusable")]
    [string]$CacheMode = "Iterative",

    [switch]$Timings,

    # A full Release without this path emits only fresh candidate artifacts for
    # a real capture. Supplying the resulting production receipt on a second
    # full Release is the only route to a portable package/production receipt.
    [string]$ProductionCaptureReceiptPath = "",

    # Where the C07 gate looks for the hash-pinned shipped installed content. Empty
    # means the generator's own default install path. A machine without the game
    # runs the reduced committed-artifact check and says so; this switch is for
    # a machine that HAS the game somewhere else, so it can carry the full
    # game-derived authority rather than the reduced one.
    [string]$C07GameRoot = "",

    # -Mode Iterate only. Adds cargo's dep-info to the iteration selection: a
    # compile-only `cargo test --no-run --message-format=json` pass whose
    # `compiler-artifact` events tie each `.d` file to a package, so every
    # source file the COMPILER read for a target is attributed to that target's
    # package. It is the only thing that sees a file spliced in by a COMPUTED
    # literal (`include!(concat!(..))`, a macro-generated `#[path]`), which the
    # lexical scan names as its own residual and which lane JD proved no regex
    # over Rust can close.
    #
    # OPT-IN, NOT AUTOMATIC, and deliberately so. On a WARM target the pass is a
    # freshness check that recompiles nothing (0.36 s for map_viewer, measured);
    # on a cold or partly-stale one it is a full compile of every root, which
    # would turn a sub-second iteration into minutes without the caller asking
    # for it. There is no cheap way to tell those apart in advance -- running
    # the pass IS the probe -- so the choice is the caller's rather than a guess
    # this file makes on their behalf. It is additive either way: it can only
    # widen the selection, never narrow it.
    [switch]$DepInfo
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# FAILED UNTIL PROVEN OTHERWISE. This is the first statement of the file that can
# fail or be skipped - only `Set-StrictMode` and `$ErrorActionPreference`, which
# are unconditional interpreter settings, run before it - and it is placed here
# for that reason.
#
# This repo's `-Command` gate launcher ends with
#
#     & './Invoke-CentralRustValidation.ps1' ...; exit $LASTEXITCODE
#
# and $LASTEXITCODE holds the status of the last NATIVE command only. Every
# refusal in this file is a PowerShell terminating error, which does not touch
# it; this file is also an ADVANCED script ([CmdletBinding()] above), so its
# terminating error reaches the caller as an ordinary error record governed by
# the CALLER's $ErrorActionPreference - Continue - and the launcher runs on to
# `exit $LASTEXITCODE`. In a fresh shell that variable is $null and `exit $null`
# exits ZERO. So a refusal that ran no tests at all announced success.
#
# Seeding 1 here and clearing it to 0 only after the run has completed makes
# EVERY failure in the body - the two refusals below, the mutex, the dot-sourced
# helpers, anything inside the main try, and the cleanup in its finally - exit
# nonzero, whether or not it is caught.
#
# THE ONE CASE THIS CANNOT REACH is a PARAMETER-BINDING failure (a bad -Mode, an
# unknown parameter name): attribute validation runs before any statement of
# this file, so nothing here has executed and $LASTEXITCODE still holds whatever
# the caller left. Only the LAUNCHER can close that, and as of lane KL it does:
# the `-Command` spelling seeds the same failure BEFORE the call, so a runner
# that never started is a runner that never cleared it.
#
#     powershell.exe -NoProfile -ExecutionPolicy Bypass -Command
#       "$global:LASTEXITCODE = 1; Set-Location '<scripts>';
#        & './Invoke-CentralRustValidation.ps1' <args>; exit $LASTEXITCODE"
#
# WHY THE SEED AND NOT `if (-not $?) { exit 1 }`, which this note used to
# propose. Honestly: not because the `$?` spelling is broken. Lane KL first
# argued the seed was strictly wider, and MEASURED THAT CLAIM TO BE FALSE -
# `if (-not $?) { exit 1 }` also exits 1 for a binding failure, for a runner path
# that does not resolve, and for a failed `Set-Location`, and `$?` comes back
# True after a successful run (`probe_launcher_exit.txt`, "the $? spelling"
# section). Either spelling closes the hole.
#
# The seed is preferred for one reason that survives measurement: it does not
# depend on `$?` at all, so it cannot be perturbed by what this file writes to
# its error stream, and it adds no CONDITIONAL after the call. The trailing
# `exit $LASTEXITCODE` is still required either way; what the seed avoids is a
# second guard clause between the call and that exit, which is the part a
# copy-pasted launcher most easily loses. It also composes with the seed above
# rather than sitting beside it: both say "failed until proven otherwise", and
# the single zero at the end of the outer `finally` is the only thing that
# clears either.
#
# The seed must go AFTER anything native in the payload, because a successful
# native command writes 0 over it - the same trap that made the seed below
# insufficient on its own. Case F pins the direction that matters: a successful
# run still exits 0 with the seed in place.
#
# The `-File` spelling used by this repo's README/ACCEPTANCE examples was never
# affected: powershell.exe -File exits 1 on a binding failure by itself
# (case I). Only the `-Command` form - which lanes need, because -Package is
# [string[]] and only -Command gets an array into it - requires the seed.
#
# Case C9 in runs/jm_compile_cache/Test-CompileCacheFallback.ps1 pins BOTH
# spellings: the unguarded one still exits 0, which is why the seed exists, and
# the guarded one exits nonzero.
$global:LASTEXITCODE = 1
$script:centralValidationFailed = $false

# ============================ SETUP GUARD, OPENS ============================
# Seeding 1 is not sufficient ON ITS OWN, because a NATIVE command that succeeds
# after the seed writes 0 over it. This is not hypothetical and it hides the
# most dangerous refusal in the file: `Test-CentralValidationTreeMatchesCaller`
# runs `git rev-parse` TWICE, and when both succeed - which is exactly the case
# where the caller's tree and this script's tree can be compared and found to
# DIFFER - $LASTEXITCODE is 0 when the refusal throws. The gate that stops this
# runner from certifying a tree the caller never changed would have reported
# success.
#
# So everything executable from here to the main try is enclosed, and the catch
# re-asserts the failure code. There is no `finally`: the mutex and the parallel
# slot do not exist yet, and the main try's own finally is what releases them.
#
# THE BODY BELOW IS DELIBERATELY NOT INDENTED. It is ~1,600 lines, almost all of
# it function definitions, and indenting it would bury a two-line structural fix
# in a 1,600-line diff that no reviewer can read. The closing brace is marked
# `SETUP GUARD, CLOSES` immediately before the main try.
try {

# BEFORE ANY EARLY RETURN AND BEFORE THE MACHINE-WIDE MUTEX. `-Features` is
# honoured by three modes; every other mode must REFUSE it rather than run and
# silently ignore it, and `Stats` returns long before the argument assembly
# below, so a check down there would let `-Mode Stats -Features …` pass
# quietly. A selector that is accepted in silence is the same defect as a gate
# that validates the wrong tree.
if ($Features.Count -gt 0 -and $Mode -notin @("Check", "Build", "Test", "Iterate")) {
    throw "-Features is accepted by Check, Build, Test and Iterate only; Final is one canonical configuration."
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path

# THE TREE THIS VALIDATES IS THE ONE THIS FILE LIVES IN, not the one you are
# standing in. Everything below derives from $PSScriptRoot, so invoking the
# MAIN checkout's copy of this script by absolute path while working in a
# worktree validates the main checkout and reports green without ever having
# compiled the caller's diff. That is the worst failure a gate can have: it does
# not fail, it certifies the wrong tree, and the lane finds out at merge.
#
# Found by lane IC (2026-08-13). Refused rather than warned, because a warning
# on a green run is exactly the thing an operator scrolls past.
#
# The refusal is deliberately narrow: it fires ONLY when both the caller's
# directory and this script's directory resolve to git working trees AND those
# trees differ. A caller standing outside any git tree, or a machine without
# git, gets a warning instead - there is nothing to compare, and refusing on
# absent evidence would break invocations that are not the defect.
function Test-CentralValidationTreeMatchesCaller {
    param([Parameter(Mandatory = $true)][string]$ScriptRepoRoot)

    $git = Get-Command git -ErrorAction SilentlyContinue
    if ($null -eq $git) {
        Write-Warning (
            'git is not on PATH, so this run cannot check that it is validating the ' +
            'tree you are standing in. It validates ' + $ScriptRepoRoot + '.'
        )
        return
    }
    # `git -C <dir>` so the answer is about that directory, and 2>$null because
    # "not a git repository" is an ANSWER here, not an error to propagate.
    $callerTree = (& git -C (Get-Location).Path rev-parse --show-toplevel 2>$null)
    $callerOk = ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($callerTree))
    $scriptTree = (& git -C $ScriptRepoRoot rev-parse --show-toplevel 2>$null)
    $scriptOk = ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($scriptTree))
    if (-not $callerOk -or -not $scriptOk) {
        Write-Warning (
            'could not establish whether this run validates the tree you are standing ' +
            'in; it validates ' + $ScriptRepoRoot + '.'
        )
        return
    }
    # git answers with forward slashes; compare as resolved full paths so the
    # separator and any 8.3/relative spelling cannot make two equal trees differ.
    $callerFull = [IO.Path]::GetFullPath(($callerTree.Trim() -replace '/', '\'))
    $scriptFull = [IO.Path]::GetFullPath(($scriptTree.Trim() -replace '/', '\'))
    if ($callerFull -ieq $scriptFull) {
        return
    }
    throw (
        "refusing to validate a different working tree than the caller's." +
        [Environment]::NewLine +
        "  this script validates: $scriptFull" +
        [Environment]::NewLine +
        "  you are standing in:   $callerFull" +
        [Environment]::NewLine +
        'Everything this runner does is derived from its own location, so this run ' +
        'would have reported on a tree that does not contain your changes. Invoke ' +
        'YOUR checkout''s copy instead:' +
        [Environment]::NewLine +
        "  $callerFull\extractor\native\scripts\Invoke-CentralRustValidation.ps1"
    )
}
Test-CentralValidationTreeMatchesCaller -ScriptRepoRoot $repoRoot

$targetDir = Join-Path $repoRoot ".cargo-target"
$timingReportDir = Join-Path $repoRoot ".cargo-timings"
$nextestConfig = Join-Path $repoRoot ".config\nextest.toml"
# Shared build machinery: the attestation covers replay_app.exe as well as the
# acquisition artifacts, and the replay side verifies the same receipt, so the
# helper (with PortableProduct.PeImports.ps1 and Hoi4ClientDriving.OptIn.ps1,
# which it dot-sources) lives in the neutral layer that both sides may reach.
$neutralScripts = Join-Path $repoRoot "portable_replay\native\scripts"
$artifactAttestationHelper = Join-Path $neutralScripts "ProductionArtifact.Attestation.ps1"
$artifactAttestationPath = Join-Path $targetDir "release\hoi4-production-prebuilt-artifacts.json"
$portablePackageScript = Join-Path $PSScriptRoot "New-PortableProductPackage.ps1"
$portablePackageVerifier = Join-Path $PSScriptRoot "Test-PortableProductPackageReceipt.ps1"
$portableStaticCrtTest = Join-Path $PSScriptRoot "Test-PortableProductStaticCrt.ps1"
$productionInteractionClosureTest = Join-Path $PSScriptRoot "Test-ProductionInteractionClosure.ps1"
$passiveCapturePolicyTest = Join-Path $PSScriptRoot "Test-PassiveCapturePolicy.ps1"
$headlessRawCaptureUnitTest = Join-Path $PSScriptRoot "Test-HeadlessRawCaptureUnits.ps1"
$promotionGateAbsenceTest = Join-Path $PSScriptRoot "Test-PromotionGateAbsence.ps1"
$readOnlyGameStateWitness = Join-Path $PSScriptRoot "Read-Hoi4GameState.ps1"
$passiveCaptureLifecycle = Join-Path $PSScriptRoot "PassiveCapture.Lifecycle.ps1"
$interactionClosureHelper = Join-Path $PSScriptRoot "PortableProduct.InteractionClosure.ps1"
$productionCapturePromotionHelper = Join-Path $PSScriptRoot "ProductionCapture.Promotion.ps1"
# The serial/parallel partition authority. See its header for why the battery is
# split at all and why the split is derived from the source rather than listed.
$batteryPartitionHelper = Join-Path $PSScriptRoot "BatteryPartition.ps1"
$batteryPartitionTests = Join-Path $PSScriptRoot "Test-BatteryPartition.ps1"
. $interactionClosureHelper
. $productionCapturePromotionHelper
. $batteryPartitionHelper
$productionSourceCapabilityPaths = [ordered]@{
    recorder_host_cargo_sha256 = Join-Path $repoRoot 'extractor\native\recorder_host\Cargo.toml'
    recorder_host_main_sha256 = Join-Path $repoRoot 'extractor\native\recorder_host\src\main.rs'
    recorder_host_early_render_launch_sha256 = Join-Path $repoRoot 'extractor\native\recorder_host\src\early_render_launch.rs'
    recorder_host_auto_attach_sha256 = Join-Path $repoRoot 'extractor\native\recorder_host\src\auto_attach.rs'
    extractor_workspace_cargo_sha256 = Join-Path $repoRoot 'extractor\native\Cargo.toml'
    collector_cargo_sha256 = Join-Path $repoRoot 'extractor\native\collector\Cargo.toml'
    character_lane_cargo_sha256 = Join-Path $repoRoot 'extractor\native\lanes\character_presentation_live\Cargo.toml'
    focus_lane_cargo_sha256 = Join-Path $repoRoot 'extractor\native\lanes\focus_research_decisions_depth_live\Cargo.toml'
    collector_lib_sha256 = Join-Path $repoRoot 'extractor\native\collector\src\lib.rs'
    passive_ready_contract_sha256 = Join-Path $repoRoot 'extractor\native\compact_contract\src\passive_ready.rs'
    interaction_closure_helper_sha256 = $interactionClosureHelper
    production_capture_promotion_helper_sha256 = $productionCapturePromotionHelper
}
$portablePackageReceiptPath = Join-Path $targetDir "release\hoi4-portable-package-receipt.json"
$worldFoldCitationVerifier = Join-Path $PSScriptRoot "verify_world_fold_citations.py"
$worldFoldCitationTests = Join-Path $PSScriptRoot "test_verify_world_fold_citations.py"
# The repo-wide half. The world-fold gate above covers citations of ONE file;
# this one covers every `<file>:<line>` / `<file>::<item>` reference in the
# source and document trees, which is where the other 1,143 measured citations
# live. Its allowance is a ratchet: it may shrink, and a NEW line citation fails.
$citationIntegrityVerifier = Join-Path $PSScriptRoot "verify_citation_integrity.py"
$citationIntegrityTests = Join-Path $PSScriptRoot "test_verify_citation_integrity.py"
$captureScorecardTests = Join-Path $PSScriptRoot "test_capture_scorecard.py"
$clockBaselineSummaryTests = Join-Path $PSScriptRoot "test_clock_baseline_summary.py"
# The sealed manifest's gap ledger is expanded by THREE implementations (Rust,
# Python, PowerShell). Rust is held to `gap_ledger_conformance.json` by its own
# suite; these two hold the other two to the same vector, so a shape change that
# reaches only one of them fails here rather than on a promoted capture.
# The CaptureReadyV2 authenticated payload is laid out by THREE implementations
# (Rust producer, PowerShell harness mirror, this Python re-derivation). The
# first two pin each other; the third had no test at all and had silently
# drifted at two wire revisions, so it disagreed with every receipt the recorder
# wrote. These are the same golden vectors the other two are held to.
$sinkReconciliationTests = Join-Path $PSScriptRoot "test_verify_sink_reconciliation.py"
$gapLedgerTests = Join-Path $PSScriptRoot "test_gap_ledger.py"
$gapLedgerExpansionTests = Join-Path $PSScriptRoot "Test-GapLedgerExpansion.ps1"
# The domain axis (`coverage_audit/reference_ledger/domain_file_map.csv`) is a
# generated inventory that nothing regenerated for 25 domains, because its
# generator used to scan its own output and so had no meaningful "re-run it".
# This suite proves the corpus is source-only and that the committed map is
# what this tree produces; the failure names the rows that moved.
$domainFileMapTests = Join-Path $PSScriptRoot "test_map_files_to_domains.py"
# Domain 59's static catalog is the only thing replay has for authored division
# templates, unit definitions and equipment definitions, and it is resolved
# across a [vanilla, mod, ...] layer list. The gate's own proof run needs an
# installed game and a real mod, so nothing ever gated the layering: these cases
# build a two-layer tree in a temp dir and drive the gate's own collectors, so a
# mod override that stops winning fails here instead of on a modded recording.
$templatesEquipmentGateTests =
    Join-Path $PSScriptRoot "test_templates_equipment_catalog_gate.py"
# The Replay-side pair. `external_producer_dependencies.json` is the
# machine-readable boundary for what Replay still needs from a producer.
# The checker also requires the renderer's own `PROMOTED_EXTERNAL_DEPENDENCY_
# PACKETS` table to BE that ledger, row for row and in order, and requires each
# shipping row to bind exactly one behavior test.
#
# None of this was gated before. Lane KK (2026-08-15) found the checker green,
# correct and invoked by nothing at all: it sat inside a replay-owned freeze
# receipt chain that no gate ran and that had been red on master for 337
# commits. The chain is deleted; these two are what it was worth keeping, and
# they are cheap - pure Python, no cargo, no game, about a second.
$replayDependencyLedgerRoot =
    Join-Path $repoRoot "replay_interface\re_evidence\rendering_laws\XUI27"
$replayDependencyLedger =
    Join-Path $replayDependencyLedgerRoot "external_dependency_ledger.py"
$replayFixtureProvenance =
    Join-Path $replayDependencyLedgerRoot "fixture_provenance.py"
$replayDependencyLedgerTests =
    Join-Path $replayDependencyLedgerRoot "test_external_dependency_ledger.py"
$c07TreeLibraryGenerator = Join-Path $repoRoot "replay_interface\re_evidence\rendering_laws\C07\build_c07_portable_tree_library.py"
$c07TreeLibraryPinSource = Join-Path $repoRoot "portable_replay\native\render_stream\src\c07_tree_content_hashes.rs"
$expectedRustVersion = "1.96.0"
$expectedNextestVersion = "0.9.140"
. $artifactAttestationHelper
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;

public static class PortableReceiptPublishNative {
    private const uint MOVEFILE_WRITE_THROUGH = 0x00000008;

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool MoveFileExW(string existingName, string newName, uint flags);

    public static void PublishNoReplaceWriteThrough(string temporary, string destination) {
        if (File.Exists(destination) || Directory.Exists(destination)) {
            throw new IOException("durable receipt publication refuses an existing destination: " + destination);
        }
        if (!MoveFileExW(temporary, destination, MOVEFILE_WRITE_THROUGH)) {
            throw new Win32Exception(Marshal.GetLastWin32Error(),
                "durable no-replace receipt publication failed: " + destination);
        }
    }
}
'@
$mutex = [Threading.Mutex]::new($false, "Local\battle_plan_central_rust_validation")
$ownsMutex = $false
# The parallel half's core budget. See Enter-CentralValidationParallelSlot.
$parallelSlotMutexes = @()
$parallelSlotIndex = -1
# Work that Final defers until it holds the machine-wide mutex, in the order it
# was queued. Each entry is @{ Describe = <string>; Action = <scriptblock> }.
$serialPhaseActions = [System.Collections.Generic.List[object]]::new()
# What an iteration run actually executed. These exist so "this run tested
# nothing" is a countable fact rather than an inference from a green exit code:
# nextest exits 0 for a selection that matches nothing, in every half of every
# root, and an iteration selection is DERIVED, so it is exactly the kind of
# selection that can quietly come out empty.
$script:iterationTestsRun = 0
$script:iterationSerialHalvesQueued = 0
$releaseInputDigestBefore = $null
$releaseBuildEnvironmentBefore = $null
$releaseArtifactFreshness = $null

function Get-CurrentInteractionPolicyInputs {
    $inputs = [ordered]@{
        read_only_game_state_witness_sha256 = (Get-FileHash -LiteralPath $readOnlyGameStateWitness -Algorithm SHA256).Hash.ToLowerInvariant()
        passive_capture_policy_test_sha256 = (Get-FileHash -LiteralPath $passiveCapturePolicyTest -Algorithm SHA256).Hash.ToLowerInvariant()
        promotion_gate_absence_test_sha256 = (Get-FileHash -LiteralPath $promotionGateAbsenceTest -Algorithm SHA256).Hash.ToLowerInvariant()
        passive_capture_lifecycle_sha256 = (Get-FileHash -LiteralPath $passiveCaptureLifecycle -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    foreach ($entry in $productionSourceCapabilityPaths.GetEnumerator()) {
        $inputs[$entry.Key] = (Get-FileHash -LiteralPath $entry.Value -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $sourceTree = Get-ProductionInteractionSourceTreeAuthority -RepoRoot $repoRoot
    $inputs['production_interaction_source_tree_file_count'] = [int]$sourceTree.file_count
    $inputs['production_interaction_source_tree_sha256'] = [string]$sourceTree.sha256
    return $inputs
}

function Get-CurrentProductionCaptureEvidence {
    if ([string]::IsNullOrWhiteSpace($ProductionCaptureReceiptPath)) {
        throw 'production capture evidence was not supplied'
    }
    return Get-ProductionCapturePromotionEvidence `
        -RepoRoot $repoRoot `
        -CaptureReceiptPath $ProductionCaptureReceiptPath `
        -CurrentArtifactAttestationPath $artifactAttestationPath
}

function Get-CurrentPinnedHookSourceAuthority {
    $path = [string]$productionSourceCapabilityPaths.passive_ready_contract_sha256
    $source = Get-Content -LiteralPath $path -Raw
    $rosterDeclaration =
        'pub const PINNED_PRODUCTION_HOOK_SPECS_V1: &[PinnedProductionHookSpecV1] = &['
    $countDeclaration =
        'pub const PINNED_PRODUCTION_HOOK_COUNT_V1: u16 = PINNED_PRODUCTION_HOOK_SPECS_V1.len() as u16;'
    $rosterStart = $source.IndexOf($rosterDeclaration, [StringComparison]::Ordinal)
    $rosterBodyStart = $rosterStart + $rosterDeclaration.Length
    $rosterEnd = if ($rosterStart -ge 0) {
        $source.IndexOf('];', $rosterBodyStart, [StringComparison]::Ordinal)
    }
    else { -1 }
    if ($rosterStart -lt 0 -or -not $source.Contains($countDeclaration) -or
        $rosterEnd -le $rosterBodyStart) {
        throw 'passive-ready source lacks a bounded slice roster and derived count declaration'
    }
    $rosterBody = $source.Substring($rosterBodyStart, $rosterEnd - $rosterBodyStart)
    $count = [regex]::Matches(
        $rosterBody,
        '(?m)^\s*pinned_(?:import|rva)!\('
    ).Count
    if ($count -le 0 -or $count -gt [uint16]::MaxValue) {
        throw 'passive-ready source has an invalid mechanically counted pinned hook roster'
    }
    return [ordered]@{
        count = [int][uint16]$count
        source_sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

# NO-PROGRESS interval for a cargo run made through the compile cache. This is
# NOT a total time budget: batteries legitimately run for tens of minutes, and a
# fixed total budget would be a machine-speed constant this repo refuses to
# carry. What is bounded is SILENCE - cargo emitting nothing at all for this
# long while sccache is installed as RUSTC_WRAPPER.
#
# Why the bound is needed: sccache 0.16's client reads server responses with a
# blocking read and no timeout, and SCCACHE_IGNORE_SERVER_IO_ERROR only helps
# once an I/O call RETURNS an error. A server that answers the preflight probe
# and then never answers a compile request would otherwise hang the battery
# forever - the preflight probe cannot protect the rest of the run.
#
# Ten minutes is far above any gap this build produces between output lines
# (the slowest single test is ~72 s, and link steps are shorter), so it does not
# fire on a slow machine - it measures progress, not speed. A FALSE positive is
# survivable by construction: the phase is re-run cache-free, costing time and
# nothing else. That matters because cargo can sit silent on the package-cache
# lock when several lanes build at once; killing that and re-running is wasteful
# but never wrong.
$script:CargoStallTimeoutSeconds = 600

function ConvertTo-WindowsCommandLineArgument {
    <#
    .SYNOPSIS
    Quote ONE argument the way CommandLineToArgvW parses it back.

    .DESCRIPTION
    PowerShell's `& $exe @array` passes an argument array with full fidelity.
    Start-Process does NOT: -ArgumentList is joined with spaces and nothing is
    quoted, so any argument containing a space arrives as several arguments.
    That is not theoretical - it broke a battery here. nextest filtersets look
    like

        not (package(=collector) | package(=fold_v3))

    and being split on spaces produced `error: expected expression / 1 | not`
    and `failed to parse filterset`, exit 94, on a tree that was perfectly fine.

    So every argument is quoted here before Start-Process sees it: wrap when it
    contains whitespace or a quote, double the backslashes that precede a quote
    (and those at the very end, which would otherwise escape the closing quote),
    and escape embedded quotes.
    #>
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value)

    if ($Value -eq '') { return '""' }
    if ($Value -notmatch '[\s"]') { return $Value }

    $builder = New-Object System.Text.StringBuilder
    $null = $builder.Append('"')
    $pendingBackslashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq '\') {
            $pendingBackslashes++
            continue
        }
        if ($character -eq '"') {
            $null = $builder.Append('\' * (($pendingBackslashes * 2) + 1))
            $null = $builder.Append('"')
            $pendingBackslashes = 0
            continue
        }
        if ($pendingBackslashes -gt 0) {
            $null = $builder.Append('\' * $pendingBackslashes)
            $pendingBackslashes = 0
        }
        $null = $builder.Append($character)
    }
    if ($pendingBackslashes -gt 0) {
        $null = $builder.Append('\' * ($pendingBackslashes * 2))
    }
    $null = $builder.Append('"')
    return $builder.ToString()
}

function Initialize-ProcessJobType {
    <#
    .SYNOPSIS
    Compile the Windows Job Object wrapper once per process. $true if the type
    is available.

    .DESCRIPTION
    WHY A JOB OBJECT, after four spellings that tried to do this with pids.

    The watchdog has to answer one question before it may start a cache-free
    retry: is cargo's whole process tree - the rustc children, the sccache
    clients inside them - gone? Everything built on process IDs answered it
    approximately, and every approximation was a defect:

      * `taskkill /T <root>` does its OWN tree walk, so it obeys none of the
        caller's decisions about what belongs to the tree, and can terminate a
        process the caller could not even identify;
      * a pid is not an identity. Windows reuses them, so a member that exits
        between the census and the kill hands its number to a stranger, and the
        stranger is what gets waited on and killed;
      * comparing creation timestamps narrows that to a window rather than
        closing it - a 2 ms wall-clock agreement is not identity, and a clock
        adjustment can make a live member look like a different process;
      * and no census can contain a descendant created AFTER it was taken, so a
        rustc spawned a moment later is invisible to the proof no matter how
        carefully the list is built.

    A job object has none of those properties, because it never names a process
    at all. Every process cargo creates is in the job automatically - children
    inherit job membership and cannot leave it, since breakaway is only possible
    if the job permits it and this one does not - so the job IS the tree,
    including descendants created after any observation. `TerminateJobObject`
    kills exactly its members and nothing else, and the number of processes
    still assigned is a categorical answer to "is it gone", with no pid resolved
    anywhere.

    The job carries NO limit flags - KILL_ON_JOB_CLOSE least of all; see
    Create() for why arming it was worse than the cleanup it bought.

    THAT GAP IS CLOSED. `Start-Process` gives no way to create a process
    suspended, so this path does its own CreateProcess and the child joins the
    job before executing one instruction (StartSuspendedInJob).

    THE RESIDUAL THAT REMAINS, at its actual scope: the job carries no limit
    flags, so a run ABANDONED while cargo is still alive - this process killed,
    a cancellation, an exceptional unwind that never reaches the stall path -
    closes an inert job and leaves the tree running. That is the behaviour that
    existed before this lane. The alternative, KILL_ON_JOB_CLOSE, was armed for
    two rounds and cost more than it bought; see Create().
    #>
    if ('JmProcessJob' -as [type]) { return $true }
    try {
        Add-Type -ErrorAction Stop -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public class JmProcessJob {
    [StructLayout(LayoutKind.Sequential)]
    struct IO_COUNTERS {
        public ulong ReadOperationCount, WriteOperationCount, OtherOperationCount;
        public ulong ReadTransferCount, WriteTransferCount, OtherTransferCount;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
        public long PerProcessUserTimeLimit;
        public long PerJobUserTimeLimit;
        public uint LimitFlags;
        public UIntPtr MinimumWorkingSetSize;
        public UIntPtr MaximumWorkingSetSize;
        public uint ActiveProcessLimit;
        public UIntPtr Affinity;
        public uint PriorityClass;
        public uint SchedulingClass;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
        public JOBOBJECT_BASIC_LIMIT_INFORMATION BasicLimitInformation;
        public IO_COUNTERS IoInfo;
        public UIntPtr ProcessMemoryLimit;
        public UIntPtr JobMemoryLimit;
        public UIntPtr PeakProcessMemoryUsed;
        public UIntPtr PeakJobMemoryUsed;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateJobObject(IntPtr attributes, string name);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetInformationJobObject(IntPtr job, int infoClass, IntPtr info, uint length);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool TerminateJobObject(IntPtr job, uint exitCode);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool QueryInformationJobObject(IntPtr job, int infoClass, IntPtr info, uint length, IntPtr returned);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr handle);

    [StructLayout(LayoutKind.Sequential)]
    struct SECURITY_ATTRIBUTES {
        public int nLength;
        public IntPtr lpSecurityDescriptor;
        public bool bInheritHandle;
    }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct STARTUPINFO {
        public int cb;
        public string lpReserved, lpDesktop, lpTitle;
        public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
        public short wShowWindow, cbReserved2;
        public IntPtr lpReserved2, hStdInput, hStdOutput, hStdError;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct PROCESS_INFORMATION {
        public IntPtr hProcess, hThread;
        public int dwProcessId, dwThreadId;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CreateProcess(string applicationName, System.Text.StringBuilder commandLine,
        IntPtr processAttributes, IntPtr threadAttributes, bool inheritHandles, uint creationFlags,
        IntPtr environment, string currentDirectory, ref STARTUPINFO startupInfo,
        out PROCESS_INFORMATION processInformation);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateFile(string fileName, uint access, uint share,
        ref SECURITY_ATTRIBUTES security, uint creationDisposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetExitCodeProcess(IntPtr process, out uint exitCode);

    const int JobObjectBasicProcessIdList = 3;
    const int JobObjectExtendedLimitInformation = 9;
    const int ERROR_MORE_DATA = 234;
    const uint CREATE_SUSPENDED = 0x4;
    const uint CREATE_NO_WINDOW = 0x8000000;
    const uint GENERIC_WRITE = 0x40000000;
    const uint GENERIC_READ = 0x80000000;
    const uint FILE_SHARE_READ_WRITE = 0x3;
    const uint CREATE_ALWAYS = 2;
    const uint OPEN_EXISTING = 3;
    const uint FILE_ATTRIBUTE_NORMAL = 0x80;
    const uint STARTF_USESTDHANDLES = 0x100;
    const int STD_INPUT_HANDLE = -10;
    const uint DUPLICATE_SAME_ACCESS = 0x2;
    const short SW_HIDE = 0;
    const uint WAIT_OBJECT_0 = 0;
    const uint STILL_ACTIVE = 259;

    IntPtr handle = IntPtr.Zero;
    IntPtr processHandle = IntPtr.Zero;
    public string LastError = "";
    public int ProcessId = -1;
    public bool IsOpen { get { return handle != IntPtr.Zero; } }

    // NO LIMIT FLAGS, AND KILL_ON_JOB_CLOSE IN PARTICULAR IS NOT SET.
    //
    // It was, for two rounds, on the reasoning that a runner killed outright
    // should take its build tree with it. That turned out to cost far more than
    // it was worth. The flag makes CLOSING THE LAST HANDLE lethal to everything
    // still in the job, and what is still in the job after an ORDINARY
    // successful cargo is whatever cargo deliberately left running - for one,
    // the long-lived sccache server its client starts when it finds none, which
    // is shared with every other lane on this machine. So the ordinary path had
    // to disarm the flag before closing, and a disarm that FAILED left no safe
    // move: refusing still runs the finally, and the finally still closes the
    // handle, so the run would go red AND take the shared server with it.
    //
    // Not arming it removes the entire failure mode. The tree is killed the one
    // way that is unambiguous: TerminateJobObject, explicitly, on the stall path
    // where this code has decided the tree must die. What is given up is cleanup
    // after an abruptly killed runner - which is the behaviour that existed
    // before this lane, and which nothing here can tell apart from a normal exit
    // anyway.
    public bool Create() {
        handle = CreateJobObject(IntPtr.Zero, null);
        if (handle == IntPtr.Zero) {
            LastError = "CreateJobObject failed with " + Marshal.GetLastWin32Error();
            return false;
        }
        return true;
    }

    // CREATE THE PROCESS SUSPENDED, ASSIGN IT, THEN LET IT RUN. Assignment is
    // not retroactive, so a process started first can spawn a child that is
    // outside the job for as long as the parent takes to be scheduled again -
    // and nothing later can find that child. Suspended means it has not
    // executed one instruction when it joins the job.
    //
    // stdout/stderr go to files opened with an inheritable handle each; the
    // caller polls those files, exactly as before. stdin is an INHERITABLE
    // DUPLICATE of the caller's own, so the child sees what Start-Process gave
    // it (NUL only when there is nothing to duplicate).
    public int StartSuspendedInJob(string application, string commandLine, string stdoutPath, string stderrPath,
            string workingDirectory) {
        if (handle == IntPtr.Zero) { LastError = "the job was never created"; return -1; }

        SECURITY_ATTRIBUTES inheritable = new SECURITY_ATTRIBUTES();
        inheritable.nLength = Marshal.SizeOf(typeof(SECURITY_ATTRIBUTES));
        inheritable.lpSecurityDescriptor = IntPtr.Zero;
        inheritable.bInheritHandle = true;

        // ALL THREE ARE DECLARED INVALID AND THE try IS ENTERED BEFORE ANY OF
        // THEM IS OPENED, so that EVERY return below - including the failure
        // returns - passes through the finally that closes them. Opening the
        // redirect files first and returning early on a later failure leaked
        // both of them: the temporary files could then not be deleted, and the
        // Start-Process fallback this failure exists to reach could fail on a
        // sharing violation against those same paths. A degraded fallback that
        // cannot run is worse than the defect it was avoiding.
        IntPtr hOut = (IntPtr)(-1);
        IntPtr hErr = (IntPtr)(-1);
        IntPtr hIn = (IntPtr)(-1);
        try {
            hOut = CreateFile(stdoutPath, GENERIC_WRITE, FILE_SHARE_READ_WRITE, ref inheritable,
                CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, IntPtr.Zero);
            hErr = CreateFile(stderrPath, GENERIC_WRITE, FILE_SHARE_READ_WRITE, ref inheritable,
                CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, IntPtr.Zero);
            if (hOut == (IntPtr)(-1) || hErr == (IntPtr)(-1)) {
                LastError = "could not open the redirect files: " + Marshal.GetLastWin32Error();
                return -1;
            }

            // THE CALLER'S OWN STDIN, not NUL, so the child sees what it saw
            // under Start-Process -NoNewWindow: a cargo that wants to prompt for
            // registry credentials, or a test binary that reads stdin, must not
            // behave differently because this runner changed how it launches
            // things.
            //
            // DUPLICATED, because bInheritHandles only carries handles that are
            // themselves INHERITABLE and GetStdHandle can hand back a perfectly
            // valid one that is not - a redirected CI host being the usual way.
            //
            // THREE OUTCOMES, and the middle one is the point. NO standard input
            // at all (a service, a host that hands back nothing) -> NUL, which is
            // what the child would have got anyway. A handle that duplicates ->
            // the child sees exactly the caller's stdin. A handle that EXISTS but
            // cannot be duplicated is NEITHER: quietly turning the caller's stdin
            // into EOF would be this launcher changing what the child reads,
            // silently, on a host that is not this machine. That case gives up
            // the job path - LOUDLY, via the named degraded notice - and lets
            // Start-Process hand on the real handle the way it always did.
            IntPtr callerStdIn = GetStdHandle(STD_INPUT_HANDLE);
            if (callerStdIn != IntPtr.Zero && callerStdIn != (IntPtr)(-1)) {
                IntPtr duplicate;
                IntPtr self = GetCurrentProcess();
                if (!DuplicateHandle(self, callerStdIn, self, out duplicate, 0, true, DUPLICATE_SAME_ACCESS)) {
                    LastError = "the caller's standard input could not be duplicated for inheritance (" +
                        Marshal.GetLastWin32Error() + "); substituting NUL would change what the child reads";
                    return -1;
                }
                hIn = duplicate;
            }
            if (hIn == (IntPtr)(-1)) {
                // There is genuinely no standard input to hand on: NUL, so a
                // child that reads stdin sees EOF rather than an invalid handle.
                hIn = CreateFile("NUL", GENERIC_READ, FILE_SHARE_READ_WRITE, ref inheritable,
                    OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, IntPtr.Zero);
                if (hIn == (IntPtr)(-1)) {
                    LastError = "could not open NUL for the child's standard input: " + Marshal.GetLastWin32Error();
                    return -1;
                }
            }

            STARTUPINFO si = new STARTUPINFO();
            si.cb = Marshal.SizeOf(typeof(STARTUPINFO));
            si.dwFlags = (int)STARTF_USESTDHANDLES;
            si.hStdInput = hIn;
            si.hStdOutput = hOut;
            si.hStdError = hErr;

            PROCESS_INFORMATION pi;
            // CreateProcess can WRITE to its command line buffer, so it must be
            // a mutable StringBuilder rather than a string.
            //
            // NO CREATE_NO_WINDOW: the child keeps this process's console,
            // exactly as Start-Process -NoNewWindow gave it. Taking the console
            // away is a behaviour change for anything that asks whether it has
            // one, and this path launches every cargo call in the battery.
            System.Text.StringBuilder mutable = new System.Text.StringBuilder(commandLine);
            // THE WORKING DIRECTORY IS PASSED EXPLICITLY, and null here was a
            // real defect that a full battery caught: CreateProcess with a null
            // lpCurrentDirectory inherits the PROCESS's current directory, and
            // PowerShell's `Set-Location` / `Push-Location` do NOT change that -
            // they move a provider location that only PowerShell knows about.
            // `Start-Process` defaulted its -WorkingDirectory to that provider
            // location, so switching to CreateProcess silently moved every cargo
            // invocation to whatever directory this process happened to start
            // in, and cargo answered "could not find Cargo.toml".
            if (!CreateProcess(application, mutable, IntPtr.Zero, IntPtr.Zero, true,
                    CREATE_SUSPENDED, IntPtr.Zero, workingDirectory, ref si, out pi)) {
                LastError = "CreateProcess failed with " + Marshal.GetLastWin32Error();
                return -1;
            }

            if (!AssignProcessToJobObject(handle, pi.hProcess)) {
                LastError = "AssignProcessToJobObject failed with " + Marshal.GetLastWin32Error();
                // -2, not -1: the caller may NOT fall back when a process it
                // created cannot be shown to have died.
                if (!KillSuspended(pi)) { LastError += "; " + LastError2; return -2; }
                return -1;
            }

            if (ResumeThread(pi.hThread) == 0xFFFFFFFF) {
                LastError = "ResumeThread failed with " + Marshal.GetLastWin32Error();
                if (!KillSuspended(pi)) { LastError += "; " + LastError2; return -2; }
                return -1;
            }

            CloseHandle(pi.hThread);
            processHandle = pi.hProcess;
            ProcessId = pi.dwProcessId;
            return ProcessId;
        }
        finally {
            // The child has its own duplicates; these are this process's copies
            // and must go, or the files stay open for writing after it exits.
            // stdin included: it is always a handle this function opened or
            // duplicated, never the caller's own.
            if (hOut != (IntPtr)(-1)) { CloseHandle(hOut); }
            if (hErr != (IntPtr)(-1)) { CloseHandle(hErr); }
            if (hIn != (IntPtr)(-1)) { CloseHandle(hIn); }
        }
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool TerminateProcess(IntPtr process, uint exitCode);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr GetStdHandle(int stdHandle);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool DuplicateHandle(IntPtr sourceProcess, IntPtr sourceHandle, IntPtr targetProcess,
        out IntPtr targetHandle, uint desiredAccess, bool inheritHandle, uint options);

    public string LastError2 = "";

    // A SUSPENDED PROCESS THAT COULD NOT BE PUT IN THE JOB MUST DIE, AND BE SEEN
    // TO. Resuming it would put a build outside the job; leaving it suspended
    // would leave it forever; and closing the handle without waiting would let
    // the fallback start a second cargo beside one this code cannot prove is
    // gone. TerminateProcess is asynchronous, so the wait is the proof.
    bool KillSuspended(PROCESS_INFORMATION pi) {
        bool dead = false;
        if (TerminateProcess(pi.hProcess, 1)) {
            dead = WaitForSingleObject(pi.hProcess, 15000) == WAIT_OBJECT_0;
            if (!dead) { LastError2 = "the suspended process did not exit within 15 s of being terminated"; }
        }
        else {
            LastError2 = "TerminateProcess failed with " + Marshal.GetLastWin32Error();
        }
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
        return dead;
    }

    public bool HasExited {
        get {
            if (processHandle == IntPtr.Zero) { return true; }
            return WaitForSingleObject(processHandle, 0) == WAIT_OBJECT_0;
        }
    }

    public bool WaitForExit(int milliseconds) {
        if (processHandle == IntPtr.Zero) { return true; }
        return WaitForSingleObject(processHandle, (uint)milliseconds) == WAIT_OBJECT_0;
    }

    // -1 when it cannot be read, which the caller must not confuse with 0.
    public int ExitCode {
        get {
            uint code;
            if (processHandle == IntPtr.Zero) { LastError = "no process was started"; return -1; }
            if (!GetExitCodeProcess(processHandle, out code)) {
                LastError = "GetExitCodeProcess failed with " + Marshal.GetLastWin32Error();
                return -1;
            }
            if (code == STILL_ACTIVE) { LastError = "the process is still running"; return -1; }
            return unchecked((int)code);
        }
    }

    public bool Assign(IntPtr processHandle) {
        if (handle == IntPtr.Zero) { LastError = "the job was never created"; return false; }
        if (!AssignProcessToJobObject(handle, processHandle)) {
            LastError = "AssignProcessToJobObject failed with " + Marshal.GetLastWin32Error();
            return false;
        }
        return true;
    }

    public bool Terminate() {
        if (handle == IntPtr.Zero) { LastError = "the job was never created"; return false; }
        if (!TerminateJobObject(handle, 1)) {
            LastError = "TerminateJobObject failed with " + Marshal.GetLastWin32Error();
            return false;
        }
        return true;
    }

    // How many processes are still assigned. -1 means the question could not be
    // asked, which the caller must treat as "not established" rather than zero.
    public int AssignedProcessCount() {
        if (handle == IntPtr.Zero) { LastError = "the job was never created"; return -1; }
        int size = 8 + (8 * 4096);
        IntPtr buffer = Marshal.AllocHGlobal(size);
        try {
            if (!QueryInformationJobObject(handle, JobObjectBasicProcessIdList, buffer, (uint)size, IntPtr.Zero)) {
                int error = Marshal.GetLastWin32Error();
                // ERROR_MORE_DATA means the id list did not fit, but the count
                // in the first field is filled in and is the only field read.
                if (error != ERROR_MORE_DATA) {
                    LastError = "QueryInformationJobObject failed with " + error;
                    return -1;
                }
            }
            return Marshal.ReadInt32(buffer, 0);
        }
        finally { Marshal.FreeHGlobal(buffer); }
    }

    public void Close() {
        if (processHandle != IntPtr.Zero) { CloseHandle(processHandle); processHandle = IntPtr.Zero; }
        if (handle != IntPtr.Zero) { CloseHandle(handle); handle = IntPtr.Zero; }
    }
}
'@
    }
    catch {
        return $false
    }
    return $null -ne ('JmProcessJob' -as [type])
}

function Wait-ProcessJobEmpty {
    <#
    .SYNOPSIS
    Poll until the job holds no processes, or the budget expires. Returns the
    number still assigned: 0 is the proof of death, and -1 means it could not be
    established - which is not the same as zero and must never be read as it.
    #>
    param(
        [Parameter(Mandatory = $true)][object]$Job,
        [Parameter(Mandatory = $true)][int]$TimeoutMs
    )

    # A STOPWATCH, not the wall clock: `Get-Date` moves when the machine's clock
    # is adjusted (NTP, DST, a VM resuming), and a backwards step would silently
    # extend a bound whose whole job is to be exact.
    $elapsed = [Diagnostics.Stopwatch]::StartNew()
    while ($true) {
        $count = $Job.AssignedProcessCount()
        if ($count -eq 0) { return 0 }
        if ($elapsed.ElapsedMilliseconds -ge $TimeoutMs) { return $count }
        Start-Sleep -Milliseconds 200
    }
}

function Invoke-WatchedProcess {
    <#
    .SYNOPSIS
    Run a program, streaming its output, and report whether it went silent for
    longer than $StallSeconds. Never throws on a stall - it reports one.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$Program,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][int]$StallSeconds,
        # When set, STDOUT is collected and returned instead of relayed to the
        # host. Needed for `nextest list`, whose stdout is the JSON the caller
        # parses. Stderr is still relayed, and both streams still count as
        # progress, so a captured call is bounded exactly like a streamed one.
        [switch]$CaptureStandardOutput
    )

    $outFile = [IO.Path]::GetTempFileName()
    $errFile = [IO.Path]::GetTempFileName()
    $outOffset = 0L
    $errOffset = 0L
    $lastProgress = Get-Date
    $stalled = $false
    $process = $null
    $job = $null
    $jobAssignFailure = ''

    # Relays whatever has been written since the last look, and reports whether
    # anything arrived - that is the definition of progress used here.
    #
    # CAPTURE MODE NEVER DECODES HERE. In capture mode the return value is the
    # census JSON the caller parses, and decoding it incrementally out of a file
    # a live process is appending to loses bytes two ways:
    #   * `ReadToEnd` stops at the EOF it sees; reading `$stream.Length`
    #     afterwards can report MORE than was read, and advancing the offset to
    #     that length marks the difference consumed without ever having captured
    #     it;
    #   * a `StreamReader` recreated at an arbitrary byte offset can start
    #     inside a multi-byte UTF-8 sequence and turn it into replacement
    #     characters.
    # Either one makes a healthy `nextest list` come back as malformed JSON and
    # fails a green tree. So capture mode watches the file's LENGTH for progress
    # and the whole stdout file is decoded ONCE, after the process has exited.
    # The relayed path keeps streaming - it is a console mirror, and it now
    # advances by what the reader actually consumed rather than by the length it
    # read back afterwards.
    $pump = {
        param([string]$Path, [ref]$Offset, [bool]$LengthOnly)
        $moved = $false
        try {
            $stream = [IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite')
            try {
                if ($stream.Length -gt $Offset.Value) {
                    if ($LengthOnly) {
                        $Offset.Value = $stream.Length
                        $moved = $true
                    }
                    else {
                        $null = $stream.Seek($Offset.Value, 'Begin')
                        $reader = New-Object IO.StreamReader($stream)
                        $text = $reader.ReadToEnd()
                        if (-not [string]::IsNullOrEmpty($text)) {
                            Write-Host $text -NoNewline
                            $moved = $true
                        }
                        # What the reader CONSUMED, not the file's length now.
                        $Offset.Value = $stream.Position
                    }
                }
            }
            finally { $stream.Dispose() }
        }
        catch { }
        return $moved
    }

    try {
        # Redirecting to FILES rather than pipes: the parent then never has to
        # drain a pipe to keep the child alive, so a stalled child cannot be
        # confused with a parent that stopped reading.
        # Quote every argument before Start-Process sees it - see
        # ConvertTo-WindowsCommandLineArgument for why this is not optional.
        $quoted = @($Arguments | ForEach-Object { ConvertTo-WindowsCommandLineArgument -Value $_ })

        # THE PROCESS IS CREATED SUSPENDED AND JOINS THE JOB BEFORE IT EXECUTES
        # ONE INSTRUCTION. Assignment is not retroactive: a process that is
        # already running can spawn a child before it is assigned, and that child
        # is outside the job permanently - invisible to the kill and to the proof
        # that follows it. There is no way to ask Start-Process for a suspended
        # process, so this path does its own CreateProcess; see
        # Initialize-ProcessJobType.
        #
        # The command line is built HERE, with the program quoted by the same
        # function as every argument, because CreateProcess takes one string
        # rather than a list. Case C6 checks argv arrives unchanged end to end.
        # RESOLVED THROUGH PATH FIRST, and this is not a detail. CreateProcess
        # given a non-null lpApplicationName does NO PATH or PATHEXT search: it
        # takes the name literally, relative to the current directory, extension
        # included. So `cargo` - the name every call site in this file passes -
        # simply does not exist, CreateProcess fails, and the code below falls
        # back to Start-Process. That is exactly what shipped for one round: the
        # suspended job path worked perfectly for the test shims (absolute paths)
        # and never once ran for a real cargo call, silently, because the
        # fallback is quiet by design.
        $resolvedProgram = $Program
        try {
            $found = @(Get-Command $Program -CommandType Application -ErrorAction Stop)
            if ($found.Count -gt 0) { $resolvedProgram = $found[0].Source }
        }
        catch { }

        if (Initialize-ProcessJobType) {
            $job = New-Object JmProcessJob
            if ($job.Create()) {
                $commandLine = ((@(ConvertTo-WindowsCommandLineArgument -Value $resolvedProgram) + $quoted) -join ' ')
                # PowerShell's location, not the process's - see the comment in
                # StartSuspendedInJob. Every cargo call in this runner is made
                # from inside its workspace directory via Push-Location, and that
                # is the directory cargo must see.
                $workingDirectory = (Get-Location -PSProvider FileSystem).ProviderPath
                $started = $job.StartSuspendedInJob($resolvedProgram, $commandLine, $outFile, $errFile, $workingDirectory)
                if ($started -lt 0) {
                    $jobAssignFailure = $job.LastError
                    $job.Close()
                    $job = $null

                    # -2 IS NOT -1. -1 means "the job could not be used", and the
                    # run falls back and compiles. -2 means a process this code
                    # CREATED could not be shown to have died - it is suspended,
                    # unassigned, holding inherited handles, and unreachable now
                    # that its handles are closed. Starting a second cargo beside
                    # it is the one thing that must not happen, so this refuses
                    # rather than degrading.
                    if ($started -eq -2) {
                        # Deliberately neutral about WHICH step failed: -2 is
                        # reached from the assign failure and from the resume
                        # failure, and in the second case the process WAS in the
                        # job. What matters to whoever reads this is the same
                        # either way - a suspended process was created and could
                        # not be shown to have died.
                        throw (("this run created {0} suspended and could not complete its job-object " +
                                "startup, then could not establish that it died ({1}); refusing to start " +
                                "a second one beside a process it can no longer reach") -f `
                                $resolvedProgram, $jobAssignFailure)
                    }
                }
            }
            else {
                $jobAssignFailure = $job.LastError
                $job = $null
            }
        }
        else {
            $jobAssignFailure = 'the job-object wrapper could not be compiled on this machine'
        }

        # FAIL OPEN. If the job could not be created or the process could not be
        # started inside it, the compile still happens - this is a RECOVERY
        # mechanism, and refusing to build because a recovery mechanism is
        # unavailable would be the tail wagging the dog. What it costs is the
        # ability to PROVE the tree died, so a stall then refuses rather than
        # retrying blind.
        if ($null -eq $job) {
            # NAMED, NEVER SILENT. A quiet fallback is how the PATH-resolution
            # defect above survived a full green battery: the mechanism was not
            # running at all and nothing said so. A run that cannot bound its own
            # compiles says which line it is on.
            $notice = (
                ('COMPILE WATCHDOG DEGRADED: {0} is being started outside a job object ({1}), so a stall ' +
                 'could not be recovered from - it would be REFUSED instead. The compile itself is ' +
                 'unaffected.') -f $Program,
                $(if ([string]::IsNullOrWhiteSpace($jobAssignFailure)) { 'reason not recorded' } else { $jobAssignFailure })
            )
            Write-Host $notice
            Write-Warning $notice

            $process = Start-Process -FilePath $Program -ArgumentList $quoted -NoNewWindow -PassThru `
                -RedirectStandardOutput $outFile -RedirectStandardError $errFile
            # TOUCH .Handle IMMEDIATELY. A process object from Start-Process
            # -PassThru does not retain the native handle, and without it
            # .ExitCode comes back EMPTY after the process ends. Empty is the
            # dangerous value here: the caller tests `-ne 0`, and $null -ne 0 is
            # TRUE, so every SUCCESSFUL cargo invocation would have been reported
            # as a failure and thrown.
            $null = $process.Handle
        }

        # ONE name for "the child", whichever path started it. The job wrapper
        # answers HasExited/ExitCode from the handle CreateProcess returned, so
        # neither path ever looks a pid up.
        $processId = if ($null -ne $job) { $job.ProcessId } else { $process.Id }

        while ($true) {
            $movedOut = & $pump $outFile ([ref]$outOffset) $CaptureStandardOutput.IsPresent
            $movedErr = & $pump $errFile ([ref]$errOffset) $false
            if ($movedOut -or $movedErr) { $lastProgress = Get-Date }

            if ($(if ($null -ne $job) { $job.HasExited } else { $process.HasExited })) { break }

            if (((Get-Date) - $lastProgress).TotalSeconds -ge $StallSeconds) {
                $stalled = $true
                break
            }
            Start-Sleep -Milliseconds 500
        }

        if ($stalled) {
            # Kill the whole TREE: cargo's rustc children (and the sccache client
            # inside them) are what is actually stuck, and killing only cargo
            # would leave them holding the cache and the target dir.
            #
            # NOTHING HERE NAMES A PROCESS. The job is the tree - every process
            # cargo created is in it, including any created since the last time
            # anyone looked - so TerminateJobObject kills exactly that set and
            # nothing else, and an empty job is a categorical proof of death
            # rather than a pid lookup that can race. Four earlier spellings
            # tried to do this with pids and each one was a defect; see
            # Initialize-ProcessJobType.
            if ($null -eq $job) {
                # PARENTHESISED before -f: the operator binds to the LAST string
                # of a concatenation, so without these brackets the message went
                # out with a literal "PID {0}" in it - which is what
                # fallback/no_job_refusal.log recorded.
                throw (("the stalled cargo process (PID {0}) could not be put in a job object{1}, so its " +
                        "rustc children cannot be shown to have stopped; refusing to start a second build " +
                        "in the same target directory") -f `
                        $processId,
                        $(if ([string]::IsNullOrWhiteSpace($jobAssignFailure)) { '' }
                          else { " ($jobAssignFailure)" }))
            }

            if (-not $job.Terminate()) {
                throw (("the stalled cargo process tree (PID {0}) could not be terminated ({1}); refusing to " +
                        "start a second build in the same target directory") -f $processId, $job.LastError)
            }

            # PROVE it before anyone starts a retry. Starting a cache-free re-run
            # beside a still-live rustc would put two builds in one target
            # directory and fight over the build lock - worse than the stall.
            $stillAssigned = Wait-ProcessJobEmpty -Job $job -TimeoutMs 30000
            if ($stillAssigned -ne 0) {
                throw (("the stalled cargo process tree (PID {0}) could not be shown to have stopped - {1}; " +
                        "refusing to start a second build in the same target directory") -f `
                        $processId,
                        $(if ($stillAssigned -lt 0) { "the job could not be queried ($($job.LastError))" }
                          else { "$stillAssigned process(es) still assigned to its job" }))
            }
        }
        else {
            $null = $(if ($null -ne $job) { $job.WaitForExit(-1) } else { $process.WaitForExit() })
            # Final drain: the child may have written after the last poll.
            $null = & $pump $outFile ([ref]$outOffset) $CaptureStandardOutput.IsPresent
            $null = & $pump $errFile ([ref]$errOffset) $false

            # NOTHING TO DISARM. The job carries no KILL_ON_JOB_CLOSE, so
            # closing its handle below is inert for anything cargo deliberately
            # left running - a shared sccache server, above all. See Create():
            # arming that flag meant the ordinary path had to disarm it, and a
            # disarm that failed left no safe move, because refusing still runs
            # the finally and the finally still closes the handle.
        }

        # ONE decode of the WHOLE file, after the writer is gone. Nothing can be
        # appended to it any more, so there is no partial read to lose and no
        # arbitrary offset to split a character on. A stalled run returns no
        # output on purpose: its caller re-runs the phase without the cache.
        $capturedOutput = ''
        if ($CaptureStandardOutput -and -not $stalled) {
            try { $capturedOutput = [IO.File]::ReadAllText($outFile) } catch { $capturedOutput = '' }
        }

        $exitCode = if ($stalled) { -1 }
                    elseif ($null -ne $job) { $job.ExitCode }
                    else { $process.ExitCode }
        return [pscustomobject]@{
            ExitCode = $exitCode
            Stalled  = $stalled
            Output   = $capturedOutput
        }
    }
    finally {
        # The job handle, released once the phase is over. It carries no limit
        # flags, so closing it terminates nothing: the only thing that ever kills
        # this tree is the explicit TerminateJobObject on the stall path.
        if ($null -ne $job) { $job.Close() }
        Remove-Item -LiteralPath $outFile, $errFile -Force -ErrorAction SilentlyContinue
    }
}

# The modes whose job is to CERTIFY a tree rather than to shorten an edit loop.
# They compile once, from cold, in a fresh worktree far more often than not, and
# they never get a second edit to amortise an incremental cache against.
$script:CentralValidationGateModes = @('Test', 'Final', 'Release')

function Set-CargoIncrementalPolicy {
    <#
    .SYNOPSIS
    Decide, once, whether this run compiles incrementally, and say so.

    .DESCRIPTION
    THE DISK. Lane JZ measured this tree both ways (runs/jz_debug_shrink):

        incremental on   6.43 GiB target dir   72.4 s clean build   11.2 s edit rebuild
        incremental off  2.73 GiB (-57.5%)     60.5 s (-16.4%)      33.2 s edit rebuild

    So incremental compilation is a 3.7 GiB tax and a 12 s PENALTY on the builds
    a gate does, and a 3x saving on the build an edit loop does. It is the right
    setting for iteration and the wrong one for certification, which is why the
    decision belongs to the MODE and not to a global.

    THE SPELLING IS MEASURED, NOT ASSUMED, and the measurement changed the
    answer. runs/kv_affected_tests/incremental_precedence.json pins cargo 1.96's
    precedence on a crate carrying this repository's own profile block:

        CARGO_INCREMENTAL=1 + CARGO_PROFILE_DEV_INCREMENTAL=false   -> incremental RAN
        CARGO_INCREMENTAL=0 + CARGO_PROFILE_DEV_INCREMENTAL=true    -> incremental did NOT run

    `CARGO_INCREMENTAL` DOMINATES the profile keys. Setting only the profile
    keys - which is what this lane was originally asked to do - would have been
    a silent no-op on every run that reaches the cache-free branch below, and
    that branch is every CI run. Both spellings are therefore set: the env var
    because it is the one that binds, the profile keys because they are what
    binds if a later change stops setting the env var.

    WHY THIS IS A FUNCTION AND NOT FOUR ASSIGNMENTS. `$env:CARGO_INCREMENTAL`
    was written at four sites, two of them in the mid-run fallback that fires
    when the compile cache stops answering. Any one of them could re-enable
    incremental compilation INSIDE a gate run, hours after the decision was
    made. Now the decision exists once and the fallbacks re-apply it rather than
    overwrite it.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$Mode,
        [Parameter(Mandatory = $true)][bool]$CompileCacheInUse,
        [switch]$Quiet
    )

    if ($Mode -in $script:CentralValidationGateModes) {
        $env:CARGO_INCREMENTAL = '0'
        $env:CARGO_PROFILE_DEV_INCREMENTAL = 'false'
        $env:CARGO_PROFILE_TEST_INCREMENTAL = 'false'
        if (-not $Quiet) {
            Write-Host (
                "Incremental compilation is OFF for -Mode $Mode (a gate compiles once, from cold): " +
                'measured -57.5% target-dir bytes and -16.4% clean build time on this tree.'
            )
        }
        return
    }
    # ITERATION MODES keep the committed configuration's incremental = true,
    # which is worth 11.2 s against 33.2 s on the rebuild after an edit. The
    # profile keys are REMOVED rather than set true, so an inherited value from
    # whatever shell started this run cannot decide it either way.
    Remove-Item Env:CARGO_PROFILE_DEV_INCREMENTAL -ErrorAction SilentlyContinue
    Remove-Item Env:CARGO_PROFILE_TEST_INCREMENTAL -ErrorAction SilentlyContinue
    # sccache cannot cache an incremental compilation: the two are alternatives,
    # not additives. That is lane JM's rule and it is unchanged.
    $env:CARGO_INCREMENTAL = if ($CompileCacheInUse) { '0' } else { '1' }
}

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Program,

        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    Write-Host ("> {0} {1}" -f $Program, ($Arguments -join " "))

    # The watchdog is only worth its complexity for the case it exists for: a
    # cargo run whose rustc calls are being driven by sccache. Everything else
    # keeps the plain, direct invocation.
    $watched = ($Program -ieq 'cargo') -and -not [string]::IsNullOrWhiteSpace($env:RUSTC_WRAPPER)
    if (-not $watched) {
        & $Program @Arguments
        if ($LASTEXITCODE -ne 0) {
            throw "$Program exited with code $LASTEXITCODE"
        }
        return
    }

    $run = Invoke-WatchedProcess -Program $Program -Arguments $Arguments -StallSeconds $script:CargoStallTimeoutSeconds
    if ($run.Stalled) {
        $notice = (
            ('COMPILE CACHE UNAVAILABLE: cargo produced no output for {0} s while sccache was installed as ' +
             'RUSTC_WRAPPER, which is how a cache server that stops answering presents itself. The process ' +
             'tree was stopped and this phase is being re-run WITHOUT the cache. The run continues; only its ' +
             'speed changes, and the cache stays off for the rest of it.') -f $script:CargoStallTimeoutSeconds
        )
        Write-Host $notice
        Write-Warning $notice

        # Drop the wrapper for the remainder of the run. Re-running the same
        # phase with the same broken cache would just stall again.
        Remove-Item Env:RUSTC_WRAPPER -ErrorAction SilentlyContinue
        # RE-APPLY the run's policy rather than assert incremental=1. This is
        # mid-run: a gate that lost its compile cache is still a gate, and the
        # old assignment here could turn incremental compilation back ON hours
        # after the mode decided it should be off - which is 3.7 GiB of disk
        # arriving in the middle of a battery.
        Set-CargoIncrementalPolicy -Mode $Mode -CompileCacheInUse $false -Quiet

        Write-Host ("> {0} {1}   (cache-free re-run)" -f $Program, ($Arguments -join " "))
        & $Program @Arguments
        if ($LASTEXITCODE -ne 0) {
            throw "$Program exited with code $LASTEXITCODE"
        }
        return
    }

    if ($run.ExitCode -ne 0) {
        throw "$Program exited with code $($run.ExitCode)"
    }
}

function Test-ConfigSelectsTargetDirectory {
    <#
    .SYNOPSIS
    Does cargo, with CARGO_TARGET_DIR absent, still put build output exactly
    where this runner expects it? Never throws; answers true or false.

    .DESCRIPTION
    THE WHOLE CROSS-WORKTREE CACHE RESTS ON THIS. sccache hashes inherited
    CARGO_* environment values, so exporting CARGO_TARGET_DIR makes every
    worktree's compilations unique and reuse across checkouts is exactly zero
    (measured 0 hits / 61). Letting the repo's own .cargo/config.toml select the
    same directory - `[build] target-dir = ".cargo-target"`, resolved per
    worktree - removes that variable from the environment and the same
    experiment returns 43 hits / 61, all of them shared dependencies.

    But the runner computes artifact paths (release ZIPs, the attestation) under
    $targetDir. If config discovery ever resolved somewhere else - a cargo root
    outside this tree, a config that stops being found - those paths would point to
    nothing while the build quietly succeeded elsewhere. So this ASKS CARGO,
    with the variable cleared, and the caller falls back to exporting it when
    the answer is not the expected directory. Slower, never wrong.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$ManifestPath,
        [Parameter(Mandatory = $true)][string]$ExpectedTargetDir
    )

    if (-not (Test-Path $ManifestPath)) { return $false }

    # ASK FROM THE DIRECTORY THE REAL BUILD RUNS IN. Cargo discovers
    # .cargo/config.toml by walking up from the CURRENT DIRECTORY, not from the
    # manifest path, so asking from anywhere else answers a different question:
    # run from outside the worktree this returned the MAIN checkout's target dir
    # for a worktree manifest. Every cargo invocation in this runner is made
    # from inside its workspace directory, so the probe stands there too.
    $savedTargetDir = $env:CARGO_TARGET_DIR
    $savedPreference = $ErrorActionPreference
    $env:CARGO_TARGET_DIR = $null
    $ErrorActionPreference = 'Continue'
    $pushed = $false
    try {
        Push-Location (Split-Path -Parent $ManifestPath)
        $pushed = $true
        $json = & cargo metadata --no-deps --format-version 1 --offline --manifest-path $ManifestPath 2>$null
        if ($LASTEXITCODE -ne 0) { return $false }
        $reported = ($json | Out-String | ConvertFrom-Json).target_directory
        if ([string]::IsNullOrWhiteSpace($reported)) { return $false }
        return ([IO.Path]::GetFullPath($reported).TrimEnd('\')) -ieq ([IO.Path]::GetFullPath($ExpectedTargetDir).TrimEnd('\'))
    }
    catch { return $false }
    finally {
        if ($pushed) { Pop-Location }
        $env:CARGO_TARGET_DIR = $savedTargetDir
        $ErrorActionPreference = $savedPreference
    }
}

function Get-CompileCacheState {
    <#
    .SYNOPSIS
    Decide whether this run can compile through sccache. Never throws.

    .DESCRIPTION
    FAIL-OPEN BY DESIGN. What this replaced was

        throw "Reusable cache mode requires sccache on PATH."

    which turned a missing OPTIONAL accelerator into a refusal to validate
    anything at all. A compile cache is a speed device; a machine without one
    must still be able to run a full battery, and the only thing that may
    change is how long it takes.

    Presence on PATH is not enough to earn the wrapper. RUSTC_WRAPPER makes
    sccache the compiler driver for EVERY rustc invocation of the run, so an
    sccache that is installed but cannot start or answer its server converts
    the entire compile into hard errors - strictly worse than not using it.
    This probes the server and only a server that answers is accepted.

    Returns Usable plus a Notice naming the reason when it is false.
    #>
    param([int]$ProbeTimeoutMs = 20000)

    if ($null -eq (Get-Command sccache -ErrorAction SilentlyContinue)) {
        return [pscustomobject]@{
            Usable = $false
            Notice = 'COMPILE CACHE UNAVAILABLE: sccache is not on PATH. This run compiles with plain cargo; nothing else about it changes.'
        }
    }

    # The probe runs as a BOUNDED child process rather than an inline call, for
    # two reasons. First, sccache writes "couldn't connect to server" to STDERR
    # in the perfectly ordinary case where no server is running yet, and
    # PowerShell turns a native command's stderr into a terminating
    # NativeCommandError while $ErrorActionPreference is Stop. Second, an
    # inline call has NO time bound: an sccache that hangs instead of failing
    # would hang the whole validation, which is the one outcome a fail-open
    # path must never produce. An unanswered probe inside the deadline is
    # treated exactly like a broken one.
    # Driven through System.Diagnostics.Process rather than Start-Process. Two
    # earlier spellings of this probe both reported a HEALTHY sccache as broken,
    # and every battery silently took the plain-cargo fallback:
    #   * `-RedirectStandardOutput 'NUL' -RedirectStandardError 'NUL'` - Start-Process
    #     refuses identical redirect targets and threw;
    #   * `-PassThru` alone - .ExitCode came back empty, so the exit-code test
    #     compared against nothing.
    # Both times the fallback did its job and the run stayed green, so the only
    # symptom was the notice in the log. This spelling owns the process object,
    # so the exit code and the output are both dependable.
    $probeExit = -1
    $statsText = $null
    try {
        $startInfo = New-Object System.Diagnostics.ProcessStartInfo
        $startInfo.FileName = 'sccache'
        $startInfo.Arguments = '--show-stats'
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true

        $probe = [System.Diagnostics.Process]::Start($startInfo)
        # BOTH streams are drained ASYNCHRONOUSLY, and WaitForExit owns the
        # deadline. A synchronous ReadToEnd() blocks until end-of-stream, so a
        # child that stays alive without closing stdout never lets the timeout
        # below be reached - the "bounded" probe would hang the entire battery,
        # which is the one outcome a fail-open path must never produce. Reading
        # the two streams one after the other is also the documented redirected
        # -stream deadlock: stderr filling its buffer stalls a process whose
        # stdout is still being waited on. Starting both reads first avoids both.
        $stdoutRead = $probe.StandardOutput.ReadToEndAsync()
        $stderrRead = $probe.StandardError.ReadToEndAsync()
        if ($probe.WaitForExit($ProbeTimeoutMs)) {
            $probeExit = $probe.ExitCode
            # The process has exited, so both reads are finished or about to be;
            # bound the collection anyway rather than trust that.
            if ($stdoutRead.Wait(5000)) { $statsText = $stdoutRead.Result }
            $null = $stderrRead.Wait(1000)
        }
        else {
            # Still running past the deadline: kill it, make sure it is really
            # gone, and refuse the cache.
            try { $probe.Kill() } catch { }
            try { $null = $probe.WaitForExit(5000) } catch { }
            return [pscustomobject]@{
                Usable = $false
                Notice = ('COMPILE CACHE UNAVAILABLE: sccache did not answer within {0} ms and was stopped. This run compiles with plain cargo; nothing else about it changes.' -f $ProbeTimeoutMs)
                StatsText = $null
            }
        }
    }
    catch {
        $probeExit = -1
    }

    if ($probeExit -ne 0) {
        return [pscustomobject]@{
            Usable = $false
            Notice = ('COMPILE CACHE UNAVAILABLE: sccache is on PATH but its server did not answer (probe exit {0}). This run compiles with plain cargo; nothing else about it changes.' -f $probeExit)
            StatsText = $null
        }
    }

    # The probe's own output is carried back so callers that want the statistics
    # do not have to ask a second time: a second unbounded `--show-stats` could
    # hang, or terminate on native stderr, after this one has already succeeded.
    return [pscustomobject]@{ Usable = $true; Notice = $null; StatsText = $statsText }
}

function Measure-NextestSelection {
    <#
    .SYNOPSIS
    How many tests a `cargo nextest` selection actually matches. Never runs any.

    .DESCRIPTION
    THE COUNT, SEPARATED FROM THE VERDICT, because two callers need it for two
    different reasons and neither may re-derive it:

      * `Invoke-CheckedNextestWithCensus` REFUSES a selection of zero;
      * `-Mode Iterate` SKIPS a half whose count is zero and refuses only when
        every half of every root came back zero. An iteration selection may
        legitimately put all of its packages on one side of the partition, and
        running `not (<everything>)` against nothing is not a defect there --
        running NOTHING AT ALL is, and that is asserted once, globally.

    The count comes from a `nextest list` of the SAME selection, whose JSON goes
    to stdout, rather than from parsing the run's own summary. That is not a
    stylistic choice. Capturing a native command's output in PowerShell 5.1
    requires `2>&1`, which wraps every stderr line in an ErrorRecord, and under
    this script's `$ErrorActionPreference = "Stop"` the FIRST cargo warning then
    terminates the run. The earlier spelling did exactly that: it died on
    cargo's pre-existing "src\main.rs found to be present in multiple build
    targets" warning before a single serial test executed. Nothing else in this
    file captures a cargo invocation, and now nothing does.
    #>
    param(
        [Parameter(Mandatory = $true)][string[]]$ListArguments,
        [Parameter(Mandatory = $true)][string]$Context,
        # Set only by a caller that passes `--run-ignored` to the RUN. Without
        # it, an ignored test is matched by the filterset and then not executed,
        # so counting it would certify a half that ran nothing.
        [switch]$RunIgnored
    )

    # The list arguments are passed in rather than derived from the run's by
    # swapping `run` for `list`: `--no-fail-fast` and `--timings` are run-only
    # and nextest exits 2 on them, which the derived spelling turned into "this
    # selection could not be listed" for every healthy run.
    #
    # No 2>&1: cargo's stderr goes to the console, where it belongs, and only
    # stdout -- the JSON -- is captured.
    # BOUNDED LIKE EVERY OTHER CARGO CALL. `nextest list` BUILDS the test
    # binaries before it can list them, so it runs rustc under RUSTC_WRAPPER
    # exactly like a run does - and a direct `& cargo` here would be the one
    # compiling invocation in the battery with no stall bound, which is
    # precisely the hang this lane exists to close. Capture mode keeps stdout
    # (the JSON) while stderr still streams to the console.
    if (-not [string]::IsNullOrWhiteSpace($env:RUSTC_WRAPPER)) {
        $listRun = Invoke-WatchedProcess -Program 'cargo' -Arguments $ListArguments `
            -StallSeconds $script:CargoStallTimeoutSeconds -CaptureStandardOutput
        if ($listRun.Stalled) {
            $notice = (
                ('COMPILE CACHE UNAVAILABLE: cargo produced no output for {0} s while listing {1} with sccache ' +
                 'installed as RUSTC_WRAPPER. The process tree was stopped and the listing is being retried ' +
                 'WITHOUT the cache; the cache stays off for the rest of this run.') -f $script:CargoStallTimeoutSeconds, $Context
            )
            Write-Host $notice
            Write-Warning $notice
            Remove-Item Env:RUSTC_WRAPPER -ErrorAction SilentlyContinue
            Set-CargoIncrementalPolicy -Mode $Mode -CompileCacheInUse $false -Quiet
            $listJson = & cargo @ListArguments
            if ($LASTEXITCODE -ne 0) {
                throw "$Context could not be listed: cargo exited with code $LASTEXITCODE"
            }
        }
        else {
            if ($listRun.ExitCode -ne 0) {
                throw "$Context could not be listed: cargo exited with code $($listRun.ExitCode)"
            }
            $listJson = $listRun.Output
        }
    }
    else {
        $listJson = & cargo @ListArguments
        if ($LASTEXITCODE -ne 0) {
            throw "$Context could not be listed: cargo exited with code $LASTEXITCODE"
        }
    }
    $listed = ($listJson | Out-String) | ConvertFrom-Json
    $count = 0
    foreach ($suite in $listed.'rust-suites'.PSObject.Properties) {
        $cases = $suite.Value.testcases
        if ($null -eq $cases) { continue }
        foreach ($case in $cases.PSObject.Properties) {
            # The JSON listing carries EVERY test in a selected binary and marks
            # each with whether the filterset matched it; only the text listing
            # prunes. Counting the properties alone reported 55 selected for a
            # filterset that then ran 0, which is precisely the reading this
            # check exists to refuse.
            if ([string]$case.Value.'filter-match'.status -cne 'matches') { continue }
            # AN IGNORED TEST IS MATCHED BUT NOT EXECUTED, and counting it is a
            # false authority rather than an approximation. Nothing in this file
            # except the Evidence tier passes `--run-ignored`, so a half whose
            # matched cases are ALL ignored runs zero tests and exits zero -- and
            # the count this function returns is the only thing standing between
            # that and a green verdict. This repository has ignored tests
            # (game_ui/tests/production_renderer.rs among them), so the case is
            # reachable rather than theoretical.
            #
            # `-not $true` rather than `-eq $false`: a listing that stops
            # emitting the field at all must read as "not ignored" for the
            # count, which is the direction that keeps a real test counted.
            if (-not $RunIgnored -and $case.Value.ignored -eq $true) { continue }
            $count++
        }
    }
    return $count
}

function Invoke-CheckedNextestWithCensus {
    <#
    .SYNOPSIS
    Run a `cargo nextest` selection, having first proved it selects tests.

    .DESCRIPTION
    THE ZERO-SELECTION REFUSAL. nextest exits ZERO for a filterset that selects
    no tests -- correctly, there is nothing to fail -- so a partition that
    stopped naming the machine-serial packages would produce a battery that
    compiled them, ran them nowhere, and reported green. The parallel half of a
    Final needs no such check: it is the complement, so if the serial half is
    non-empty the parallel half was selected from a real set too.
    #>
    param(
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string[]]$ListArguments,
        [Parameter(Mandatory = $true)][string]$Context
    )

    # Derived from the RUN's own arguments rather than passed in, so the two can
    # never disagree about whether ignored tests will execute.
    $count = Measure-NextestSelection -ListArguments $ListArguments -Context $Context `
        -RunIgnored:($Arguments -ccontains '--run-ignored')
    if ($count -le 0) {
        throw (
            "$Context selected ZERO tests. That is not a pass: the battery " +
            'compiled the machine-serial packages and then ran none of them ' +
            'under the mutex, so its seal would cover only the parallel half.'
        )
    }
    Write-Host ("{0}: {1} machine-serial tests selected; running them under the mutex." -f $Context, $count)
    Invoke-Checked -Program 'cargo' -Arguments $Arguments
}

function Get-RequiredReceiptField {
    param(
        [Parameter(Mandatory = $true)]
        [PSObject]$Receipt,

        [Parameter(Mandatory = $true)]
        [string]$Field
    )

    if ($Receipt.PSObject.Properties.Name -notcontains $Field) {
        throw "C07 tree-library generator receipt has no '$Field' field; the gate cannot tell what the run did."
    }
    # Comma-wrapped so the pipeline does not unroll it: an empty array would
    # otherwise come back as $null and a one-element array as a bare scalar,
    # which is exactly the shape confusion the two callers below reject.
    return , $Receipt.$Field
}

function Get-RequiredReceiptScalar {
    param(
        [Parameter(Mandatory = $true)]
        [PSObject]$Receipt,

        [Parameter(Mandatory = $true)]
        [string]$Field
    )

    $value = Get-RequiredReceiptField -Receipt $Receipt -Field $Field
    if ($null -eq $value) {
        throw "C07 tree-library generator receipt field '$Field' is null."
    }
    # PowerShell happily compares a one-element array to a scalar and says they
    # are equal, so `"mode": ["verify"]` or `"bucket_count": [296, 7]` would pass
    # a naive check. A receipt whose shape changed is a receipt this gate no
    # longer understands, and must fail rather than be interpreted.
    if ($value -is [System.Array]) {
        throw "C07 tree-library generator receipt field '$Field' is a list, not a scalar; the receipt shape changed and this gate would be reading it wrongly."
    }
    return $value
}

function Get-RequiredReceiptList {
    param(
        [Parameter(Mandatory = $true)]
        [PSObject]$Receipt,

        [Parameter(Mandatory = $true)]
        [string]$Field
    )

    $value = Get-RequiredReceiptField -Receipt $Receipt -Field $Field
    if ($value -isnot [System.Array]) {
        throw "C07 tree-library generator receipt field '$Field' is not a list; the receipt shape changed and this gate would be reading it wrongly."
    }
    return , $value
}

function Assert-C07GeneratedTreeLibraryReceipt {
    <#
    .SYNOPSIS
    Bind the C07 generated-artifact gate to what the generator reports it did.

    .DESCRIPTION
    A zero exit code is not the success signal. A --verify that regressed into a
    no-op would exit zero having compared nothing, and Check/Build never reach
    the render_stream test target that would otherwise catch it. So this asserts
    the same things that target asserts: the run was a verify, it compared all
    three generated artifacts, it wrote none of them, its counts are the pinned
    ones, and the two digests it re-derived from the hash-pinned installed content equal
    the literals the crate compiles. Those literals are read out of the generated
    source rather than spelled again here, so this gate cannot become a third
    place a pin has to be updated.
    #>
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$ReceiptJson,

        [Parameter(Mandatory = $true)]
        [string]$PinSource
    )

    if ([string]::IsNullOrWhiteSpace($ReceiptJson)) {
        throw "C07 tree-library generator exited zero but printed no receipt."
    }
    try {
        $receipt = $ReceiptJson | ConvertFrom-Json
    }
    catch {
        throw "C07 tree-library generator receipt is not JSON: $($_.Exception.Message)"
    }

    $mode = Get-RequiredReceiptScalar -Receipt $receipt -Field "mode"
    if ($mode -cne "verify") {
        throw "C07 tree-library generator ran in mode '$mode', not 'verify'; that run would have rewritten the artifacts instead of checking them."
    }

    $expectedOutputs = @(
        "c07_tree_complete_buckets.rs",
        "c07_tree_content_hashes.rs",
        "c07_tree_content_library.bin"
    )
    # -CaseSensitive: these are the literal filenames on disk. Accepting
    # C07_TREE_CONTENT_HASHES.RS would mean accepting a receipt that is not
    # describing the files the crate includes.
    # Assigned before it is sorted, never piped straight out of the helper: the
    # helper comma-wraps to preserve list shape, and piping that would hand
    # Sort-Object the whole array as a single item.
    $reportedList = Get-RequiredReceiptList -Receipt $receipt -Field "verified_outputs"
    $reportedOutputs = @($reportedList | Sort-Object)
    if (Compare-Object -ReferenceObject $expectedOutputs -DifferenceObject $reportedOutputs -CaseSensitive) {
        throw "C07 tree-library generator reported comparing [$($reportedOutputs -join ', ')], not all three generated artifacts."
    }

    $writtenList = Get-RequiredReceiptList -Receipt $receipt -Field "changed_outputs"
    $written = @($writtenList)
    if ($written.Count -ne 0) {
        throw "C07 tree-library generator reported writing [$($written -join ', ')] in --verify mode, which it must never do."
    }

    $counts = @{}
    foreach ($count in @(
            @{ Field = "bucket_count"; Expected = 296 },
            @{ Field = "cell_count"; Expected = 99056 },
            @{ Field = "packed_bytes"; Expected = 2377344 }
        )) {
        $observed = Get-RequiredReceiptScalar -Receipt $receipt -Field $count.Field
        if ($observed -ne $count.Expected) {
            throw "C07 exact producer reported $($count.Field) = $observed, expected $($count.Expected). Library content moved; this is a finding, not a re-stamp."
        }
        $counts[$count.Field] = $observed
    }

    if (-not (Test-Path -LiteralPath $PinSource -PathType Leaf)) {
        throw "C07 pin source is missing: $PinSource"
    }
    $pinText = Get-Content -LiteralPath $PinSource -Raw
    foreach ($pin in @(
            @{ Constant = "C07_TREE_CONTENT_LIBRARY_PACKED_BYTES_SHA256"; Field = "packed_sha256" },
            @{ Constant = "C07_TREE_CONTENT_LIBRARY_PAYLOAD_SHA256"; Field = "content_library_payload_sha256" }
        )) {
        # Anchored at the start of a line and requiring `pub const`, so a
        # commented-out or shadowed declaration cannot supply the digest this
        # gate then blesses. Exactly one declaration must match: two would mean
        # the gate is comparing against an arbitrary one of them.
        $pattern = '(?m)^pub const {0}\s*:\s*&str\s*=\s*"([0-9a-f]{{64}})"\s*;' -f $pin.Constant
        $pinMatches = [regex]::Matches($pinText, $pattern)
        if ($pinMatches.Count -eq 0) {
            throw "C07 pin $($pin.Constant) was not found as a top-level 'pub const' declaration in $PinSource; the file the crate includes is not the file this gate checked."
        }
        if ($pinMatches.Count -ne 1) {
            throw "C07 pin $($pin.Constant) is declared $($pinMatches.Count) times in $PinSource; this gate cannot tell which one the crate compiles."
        }
        $rederived = Get-RequiredReceiptScalar -Receipt $receipt -Field $pin.Field
        if ($pinMatches[0].Groups[1].Value -cne $rederived) {
            throw "C07 exact producer re-derived $($pin.Field) = $rederived but $PinSource pins $($pinMatches[0].Groups[1].Value). Never edit a pin literal to match new content."
        }
    }

    Write-Host ("C07 generated tree-library check: PASS ({0} artifacts, {1} buckets, {2} cells)" -f `
            $reportedOutputs.Count, $counts["bucket_count"], $counts["cell_count"])
}

function Assert-C07CommittedArtifactReceipt {
    <#
    .SYNOPSIS
    Bind the reduced C07 check to what it reports, when the game is not here.

    .DESCRIPTION
    A hosted CI runner has no the game install, so the game-file-derived half of the
    C07 gate genuinely cannot run there. The failure mode this function exists to
    prevent is the one this repo keeps being bitten by: absence quietly reading
    as a pass. Two things stop that here.

    First, the generator does not exit zero. It exits 3, so nothing that binds
    success to a zero exit code -- including a future caller that forgets this
    distinction entirely -- can record the full gate as having passed.

    Second, this function refuses anything but the reduced receipt, by schema and
    by its explicit list of checks. The reduced check is not weak: it re-derives
    the packed digest and the canonical library payload digest from the committed
    artifacts through the same serializer the real run uses, and both must equal
    the literals the crate compiles. What it cannot do is prove those bytes are
    what the exact producer derives from the shipped BMPs, and that gap is
    printed rather than glossed.
    #>
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$ReceiptJson,

        [Parameter(Mandatory = $true)]
        [string]$PinSource
    )

    if ([string]::IsNullOrWhiteSpace($ReceiptJson)) {
        throw "C07 tree-library generator reported the game absent but printed no committed-artifact receipt."
    }
    try {
        $receipt = $ReceiptJson | ConvertFrom-Json
    }
    catch {
        throw "C07 committed-artifact receipt is not JSON: $($_.Exception.Message)"
    }

    $schema = Get-RequiredReceiptScalar -Receipt $receipt -Field "schema"
    if ($schema -cne "hoi4-c07-committed-artifact-self-consistency-v1") {
        throw "C07 committed-artifact receipt states schema '$schema'; this gate only accepts hoi4-c07-committed-artifact-self-consistency-v1."
    }
    $mode = Get-RequiredReceiptScalar -Receipt $receipt -Field "mode"
    if ($mode -cne "verify-committed") {
        throw "C07 committed-artifact receipt states mode '$mode', not 'verify-committed'."
    }
    # The receipt must say, in its own words, that the game-derived half did not
    # run. A receipt that omits this is not the reduced receipt.
    $notRun = Get-RequiredReceiptScalar -Receipt $receipt -Field "game_derived_verification"
    if (-not $notRun.StartsWith("NOT RUN")) {
        throw "C07 committed-artifact receipt does not declare the game-derived half unrun: '$notRun'."
    }

    # Naming every check keeps this gate from silently shrinking: a reduced run
    # that stopped re-deriving the payload digest would still be a valid JSON
    # receipt with a zero-length checks list, and would pass a shape-only test.
    $expectedChecks = @(
        "bucket_record_counts_sum_to_pinned_cell_count",
        "bucket_table_complete_and_ordinal_dense",
        "bucket_table_sorted_by_bucket_id",
        "generated_rust_sources_round_trip",
        "library_payload_sha256_matches_pin",
        "missing_cell_set_pin_is_the_empty_digest",
        "packed_length_equals_bucket_table_total",
        "packed_sha256_matches_pin",
        "per_bucket_instance_sha256_matches_packed_slice"
    )
    $reportedChecksList = Get-RequiredReceiptList -Receipt $receipt -Field "checks"
    $reportedChecks = @($reportedChecksList | Sort-Object)
    if (Compare-Object -ReferenceObject $expectedChecks -DifferenceObject $reportedChecks -CaseSensitive) {
        throw "C07 committed-artifact receipt reported checks [$($reportedChecks -join ', ')], not the nine this gate requires."
    }

    $counts = @{}
    foreach ($count in @(
            @{ Field = "bucket_count"; Expected = 296 },
            @{ Field = "cell_count"; Expected = 99056 },
            @{ Field = "packed_bytes"; Expected = 2377344 }
        )) {
        $observed = Get-RequiredReceiptScalar -Receipt $receipt -Field $count.Field
        if ($observed -ne $count.Expected) {
            throw "C07 committed artifacts report $($count.Field) = $observed, expected $($count.Expected). Library content moved; this is a finding, not a re-stamp."
        }
        $counts[$count.Field] = $observed
    }

    # The same pin binding as the full gate. The digests are re-derived from the
    # committed artifacts rather than from the game, but they are still compared
    # against the literals the crate compiles, read out of that source.
    if (-not (Test-Path -LiteralPath $PinSource -PathType Leaf)) {
        throw "C07 pin source is missing: $PinSource"
    }
    $pinText = Get-Content -LiteralPath $PinSource -Raw
    foreach ($pin in @(
            @{ Constant = "C07_TREE_CONTENT_LIBRARY_PACKED_BYTES_SHA256"; Field = "packed_sha256" },
            @{ Constant = "C07_TREE_CONTENT_LIBRARY_PAYLOAD_SHA256"; Field = "content_library_payload_sha256" }
        )) {
        $pattern = '(?m)^pub const {0}\s*:\s*&str\s*=\s*"([0-9a-f]{{64}})"\s*;' -f $pin.Constant
        $pinMatches = [regex]::Matches($pinText, $pattern)
        if ($pinMatches.Count -eq 0) {
            throw "C07 pin $($pin.Constant) was not found as a top-level 'pub const' declaration in $PinSource."
        }
        if ($pinMatches.Count -ne 1) {
            throw "C07 pin $($pin.Constant) is declared $($pinMatches.Count) times in $PinSource; this gate cannot tell which one the crate compiles."
        }
        $rederived = Get-RequiredReceiptScalar -Receipt $receipt -Field $pin.Field
        if ($pinMatches[0].Groups[1].Value -cne $rederived) {
            throw "C07 committed artifacts re-derive $($pin.Field) = $rederived but $PinSource pins $($pinMatches[0].Groups[1].Value). Never edit a pin literal to match new content."
        }
    }

    # Assigned before it is used, never wrapped inline: the helper comma-wraps to
    # preserve list shape, and `@(...)` around the call would nest that wrapper
    # and print the type name instead of the items.
    $uncheckedList = Get-RequiredReceiptList -Receipt $receipt -Field "unchecked_without_the_game"
    $unchecked = @($uncheckedList)
    Write-Host ("C07 generated tree-library check: REDUCED ({0} checks, {1} buckets, {2} cells)" -f `
            $reportedChecks.Count, $counts["bucket_count"], $counts["cell_count"])
    Write-Host (
        'C07 GAME-DERIVED HALF DID NOT RUN: the pinned the game files are absent ' +
        'from this machine, so the committed artifacts were held only to each ' +
        'other and to the pins the crate compiles. This validation carries NO ' +
        'authority that the packed library is what the exact producer derives ' +
        'from the shipped map files. Unchecked here: ' + ($unchecked -join ', ') + '. ' +
        'To carry that authority, run this validation on a machine with the ' +
        'pinned the game install, or pass -C07GameRoot pointing at one.'
    )
}

function Get-DetourIdentityLanePackages {
    <#
    .SYNOPSIS
    The extractor lane packages whose detour identity only holds in an
    optimized build.

    .DESCRIPTION
    Derived from the production source, never listed by hand. A lane owns
    detour identity if it mentions `Rel5HookRoster` or
    `observe_enabled_minhook_rel5`: those are the two ways a lane binds a
    receipt to a detour function's ADDRESS, and an address is only unique in a
    build the linker has not folded.

    Why this list is needed at all: the release linker's identical-code folding
    merges two detour bodies that compile to the same bytes, and a debug build
    never folds. On 2026-08-05 D95's four detour bodies really did collapse to
    two addresses under `--release` while every debug tier stayed green, because
    the routine gate runs `nextest` in the dev profile and the only tier that
    ever compiled these crates optimized was the full Release battery.

    A hand-written package list would go stale the first time a lane is added,
    and a stale list here fails OPEN -- the new lane simply is not checked. So
    the scan is mechanical and an empty result is an error, not a pass.
    #>
    $laneRoot = Join-Path $repoRoot 'extractor\native\lanes'
    if (-not (Test-Path -LiteralPath $laneRoot -PathType Container)) {
        throw "extractor lane root is missing: $laneRoot"
    }
    $packages = [System.Collections.Generic.List[string]]::new()
    foreach ($lane in Get-ChildItem -LiteralPath $laneRoot -Directory) {
        $manifest = Join-Path $lane.FullName 'Cargo.toml'
        $source = Join-Path $lane.FullName 'src'
        if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) { continue }
        if (-not (Test-Path -LiteralPath $source -PathType Container)) { continue }
        $binds = $false
        foreach ($file in Get-ChildItem -LiteralPath $source -Recurse -File -Filter '*.rs') {
            $text = Get-Content -LiteralPath $file.FullName -Raw
            if ($text -match 'Rel5HookRoster|observe_enabled_minhook_rel5') {
                $binds = $true
                break
            }
        }
        if (-not $binds) { continue }
        $name = [regex]::Match(
            (Get-Content -LiteralPath $manifest -Raw),
            '(?m)^[ \t]*name[ \t]*=[ \t]*"(?<name>[0-9A-Za-z._-]+)"[ \t]*(?:#.*)?\r?$'
        )
        if (-not $name.Success) {
            throw "lane manifest has no package name: $manifest"
        }
        $packages.Add($name.Groups['name'].Value)
    }
    if ($packages.Count -eq 0) {
        throw (
            'no extractor lane package binds a production hook receipt to a ' +
            'detour address. That is not a pass: it means this scan stopped ' +
            'matching the production source and the release detour-identity ' +
            'gate would be checking nothing.'
        )
    }
    return @($packages | Sort-Object -Unique)
}

function Invoke-ReleaseDetourIdentityGate {
    <#
    .SYNOPSIS
    Run the detour-owning lane crates' unit tests in the RELEASE profile.

    .DESCRIPTION
    `--lib` only: the laws this exists for -- one distinct detour address per
    site, each body carrying its own per-site side effect -- are unit tests
    inside the lane crates, and building their integration targets optimized as
    well would multiply the cost for no coverage this tier is about.
    #>
    $packages = Get-DetourIdentityLanePackages
    Write-Host (
        'Release detour-identity gate: {0} lane packages ({1})' -f `
            $packages.Count, ($packages -join ', ')
    )
    $arguments = @(
        'nextest', 'run', '--locked', '--lib', '--no-fail-fast',
        '--cargo-profile', 'release'
    )
    foreach ($name in $packages) {
        $arguments += @('--package', $name)
    }
    # Several receipt-custody laws -- including "a released site's detour
    # address is free again", the one that proves the new fold check does not
    # refuse a legitimate reinstall -- are behind this feature because they need
    # the crate's test-mint helpers. Without it they are silently not run, which
    # is the failure mode this whole gate exists to remove.
    if ($packages -ccontains 'production_hook_receipt') {
        $arguments += @('--features', 'production_hook_receipt/test-support')
    }
    Invoke-Checked -Program 'cargo' -Arguments $arguments
}

function Save-CargoTimingReport {
    param(
        [Parameter(Mandatory = $true)]
        [string]$WorkspaceName,

        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    if (-not $Timings) {
        return
    }
    $source = Join-Path $targetDir "cargo-timings\cargo-timing.html"
    if (-not (Test-Path -LiteralPath $source)) {
        throw "Cargo completed with -Timings but produced no report at $source"
    }
    New-Item -ItemType Directory -Path $timingReportDir -Force | Out-Null
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss-fff"
    $destination = Join-Path $timingReportDir ("{0}-{1}-{2}.html" -f $WorkspaceName.ToLowerInvariant(), $Label, $stamp)
    Copy-Item -LiteralPath $source -Destination $destination
    Write-Host "Timing report: $destination"
}

function Test-CargoManifestHasLibraryTarget {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ManifestPath
    )

    $manifest = Get-Content -LiteralPath $ManifestPath -Raw
    if ($manifest -match '(?m)^[ \t]*\[lib\][ \t]*(?:#.*)?$') {
        return $true
    }

    # Cargo implicitly creates a library target for src/lib.rs unless the
    # package explicitly disables automatic library discovery.
    $packageSection = [regex]::Match(
        $manifest,
        '(?ms)^[ \t]*\[package\][ \t]*(?:#.*)?\r?\n(?<body>.*?)(?=^[ \t]*\[|\z)'
    )
    $autolibDisabled = $packageSection.Success -and
        $packageSection.Groups['body'].Value -match '(?m)^[ \t]*autolib[ \t]*=[ \t]*false[ \t]*(?:#.*)?\r?$'
    $implicitLibrary = Join-Path (Split-Path -Parent $ManifestPath) 'src\lib.rs'
    return ((-not $autolibDisabled) -and (Test-Path -LiteralPath $implicitLibrary -PathType Leaf))
}

function Get-CargoPackageVersion {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ManifestPath
    )

    $manifest = Get-Content -LiteralPath $ManifestPath -Raw
    $packageSection = [regex]::Match(
        $manifest,
        '(?ms)^[ \t]*\[package\][ \t]*(?:#.*)?\r?\n(?<body>.*?)(?=^[ \t]*\[|\z)'
    )
    if (-not $packageSection.Success) {
        throw "Cargo manifest has no [package] section: $ManifestPath"
    }
    $versionMatch = [regex]::Match(
        $packageSection.Groups['body'].Value,
        '(?m)^[ \t]*version[ \t]*=[ \t]*"(?<version>[0-9A-Za-z._-]+)"[ \t]*(?:#.*)?\r?$'
    )
    if (-not $versionMatch.Success) {
        throw "Cargo package version is absent or not portable-package-safe: $ManifestPath"
    }
    return $versionMatch.Groups['version'].Value
}

function Assert-PinnedRustToolchain {
    $rustcVersion = (& rustc --version 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $rustcVersion -notmatch "^rustc $([regex]::Escape($expectedRustVersion))(?:\s|$)") {
        throw "Pinned rustc $expectedRustVersion is required; observed '$rustcVersion'. Install the repository rust-toolchain.toml toolchain."
    }

    $cargoVersion = (& cargo --version 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $cargoVersion -notmatch "^cargo $([regex]::Escape($expectedRustVersion))(?:\s|$)") {
        throw "Pinned Cargo $expectedRustVersion is required; observed '$cargoVersion'."
    }

    $nextestVersion = (& cargo nextest --version 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $nextestVersion -notmatch "^cargo-nextest $([regex]::Escape($expectedNextestVersion))(?:\s|$)") {
        throw "Pinned cargo-nextest $expectedNextestVersion is required; observed '$nextestVersion'."
    }

    Write-Host "Pinned validation toolchain: rustc/cargo $expectedRustVersion; cargo-nextest $expectedNextestVersion"
}

function Enable-StaticMsvcCrtForCentralRelease {
    $verboseVersion = (& rustc -vV 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0 -or $verboseVersion -notmatch '(?m)^host:\s+x86_64-pc-windows-msvc\s*$') {
        throw "central Windows Release requires rustc host x86_64-pc-windows-msvc"
    }

    # Use Cargo's highest-precedence encoded form so an inherited RUSTFLAGS or
    # checkout-local setting cannot silently return product binaries to /MD.
    $separator = [char]0x1f
    $env:CARGO_ENCODED_RUSTFLAGS = @('-C', 'target-feature=+crt-static') -join $separator
    Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue
    $cfg = @(& rustc -C 'target-feature=+crt-static' --print cfg 2>&1)
    if ($LASTEXITCODE -ne 0 -or $cfg -cnotcontains 'target_env="msvc"' -or
        $cfg -cnotcontains 'target_feature="crt-static"') {
        throw "central Windows Release could not activate the static MSVC CRT target feature"
    }
    return [ordered]@{
        target = 'x86_64-pc-windows-msvc'
        rust_target_feature = '+crt-static'
        c_runtime = 'static'
        admission = 'pe_import_and_delay_import_tables'
        forbidden_import = 'VCRUNTIME*.dll'
    }
}

function Remove-ReleasePromotionState {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Paths,

        [Parameter(Mandatory = $true)]
        [string]$Context
    )

    $problems = [System.Collections.Generic.List[string]]::new()
    foreach ($path in @($Paths | Select-Object -Unique)) {
        try {
            if (Test-Path -LiteralPath $path -ErrorAction Stop) {
                Remove-Item -LiteralPath $path -Force -ErrorAction Stop
            }
        }
        catch {
            $problems.Add("$path ($($_.Exception.Message))")
        }
    }
    foreach ($path in @($Paths | Select-Object -Unique)) {
        try {
            if (Test-Path -LiteralPath $path -ErrorAction Stop) {
                $problems.Add("$path (authoritative residue remains)")
            }
        }
        catch {
            $problems.Add("$path (absence could not be verified: $($_.Exception.Message))")
        }
    }
    if ($problems.Count -gt 0) {
        throw "$Context failed to remove and prove absence of release authority: $($problems -join '; ')"
    }
}

function Get-CentralValidationParallelSlotCount {
    <#
    .SYNOPSIS
    How many battery PARALLEL halves this machine may run at once.

    .DESCRIPTION
    The parallel half is not free of contention, it is free of CORRECTNESS
    contention: two of them still want every core the machine has. Left
    uncapped, N lanes finishing at once would each ask nextest for
    ProcessorCount test threads and each ask cargo for ProcessorCount build
    jobs, and the machine would spend its time context-switching.

    The cap is on CONCURRENT BATTERIES, not on threads per battery, and that is
    deliberate. Dividing the thread count instead would make the single-lane
    case -- by far the common one -- several times slower than it is today in
    exchange for a contention that is not happening. One slot per 8 logical
    processors keeps worst-case oversubscription at about 2x, which the
    scheduler absorbs, and leaves a lone battery exactly as fast as before.

    A machine with fewer than 16 processors gets one slot, which means its
    parallel halves serialize against each other -- still a strict improvement,
    because they no longer serialize against anyone's SERIAL half or compile.
    #>
    return [int][Math]::Max(1, [Math]::Floor([Environment]::ProcessorCount / 8))
}

function Enter-CentralValidationParallelSlot {
    <#
    .SYNOPSIS
    Take one of the machine's parallel-half slots, waiting for one if need be.

    .DESCRIPTION
    Slots are named MUTEXES rather than a named semaphore on purpose. A
    semaphore whose owner is killed -- and a wedged battery does get killed,
    that is written into the orchestration guidance -- never gives its count
    back, so the machine would quietly lose a slot per kill until the session
    ended. An abandoned mutex is handed to the next waiter by the OS, so a
    killed battery costs nothing.

    The wait is a POLL rather than `WaitHandle::WaitAny`, which throws
    NotSupportedException for more than one handle on an STA thread -- and
    powershell.exe's console host is STA. Polling a handful of handles every two
    seconds is free at this granularity, and it makes the apartment state of
    whatever shell invoked this script stop mattering.
    #>
    param([Parameter(Mandatory = $true)][Threading.Mutex[]]$Slots)

    $deadline = (Get-Date).AddMinutes(90)
    while ($true) {
        for ($index = 0; $index -lt $Slots.Count; $index++) {
            $taken = $false
            try {
                $taken = $Slots[$index].WaitOne(0)
            }
            catch [Threading.AbandonedMutexException] {
                # The previous holder was killed. The slot is ours, and saying
                # so is the point: a silent catch here would hide a wedged
                # battery from the operator reading this log.
                Write-Host (
                    'Battery parallel slot {0} was abandoned by a killed run and has been reclaimed.' -f $index
                )
                $taken = $true
            }
            if ($taken) {
                return $index
            }
        }
        if ((Get-Date) -ge $deadline) {
            throw (
                'Another centralized Rust validation process is already active. ' +
                'No battery parallel slot became free within 90 minutes.'
            )
        }
        Start-Sleep -Seconds 2
    }
}

function Enter-CentralValidationSerialPhase {
    <#
    .SYNOPSIS
    Take the machine-wide mutex for the serial half, waiting rather than failing.

    .DESCRIPTION
    A split battery has already paid for its compile and its parallel tests by
    the time it gets here, so refusing instantly -- which is right for a caller
    that has done no work yet -- would throw all of that away and send the lane
    round its retry loop from the beginning. It waits instead.

    On expiry it throws with the SAME sentence the instant refusal uses, because
    every lane's gate runner greps for exactly that text to tell "someone else
    has the machine" apart from "the battery is red".
    #>
    param([Parameter(Mandatory = $true)][Threading.Mutex]$Mutex)

    $acquired = $false
    try {
        $acquired = $Mutex.WaitOne([TimeSpan]::FromMinutes(90))
    }
    catch [Threading.AbandonedMutexException] {
        Write-Host 'The central validation mutex was abandoned by a killed run and has been reclaimed.'
        $acquired = $true
    }
    if (-not $acquired) {
        throw (
            'Another centralized Rust validation process is already active. ' +
            'The serial half of this battery waited 90 minutes for it.'
        )
    }
    return $true
}

function Add-SerialPhaseAction {
    <#
    .SYNOPSIS
    Queue one machine-serial step, with the values it must run against.

    .DESCRIPTION
    -State carries what used to be captured by `.GetNewClosure()`, and it is a
    SINGLE object on purpose.

    The queued steps run after the loops that built them, so each one needs the
    loop's values AS THEY WERE, not as they end up. `.GetNewClosure()` does that
    by binding the script block to a NEW DYNAMIC MODULE - and a script block
    bound to a module resolves commands in that module and then in GLOBAL scope.
    The functions this file defines live in the SCRIPT scope, which is neither.
    Whether they are found therefore depends on how the runner was started:

        powershell -File Invoke-CentralRustValidation.ps1 ...   -> found
        powershell -Command "& './Invoke-CentralRustValidation.ps1' ..."  -> NOT found

    Under the second form - the spelling lanes must use, because -Package is
    [string[]] and only -Command gets an array into it - every queued step died
    with "The term 'Invoke-CheckedNextestWithCensus' is not recognized", and
    because the failure is a terminating error rather than a native exit code,
    `; exit $LASTEXITCODE` then reported the LAST NATIVE COMMAND's status: the
    battery announced exit 0 having run NONE of its machine-serial tests. A gate
    that reports green after running nothing is the worst failure this script
    can have.

    A plain script block stays bound to this script's session state, so it sees
    these functions under either spelling. Passing the captured values as one
    -State parameter keeps the by-value capture without the module: one object,
    because a `[object[]]` list would flatten the string ARRAYS (the cargo
    argument lists) into their elements at binding time.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$Describe,
        [Parameter(Mandatory = $true)][scriptblock]$Action,
        [Parameter(Mandatory = $true)][object]$State
    )
    if ($null -ne $Action.Module) {
        throw (
            "the queued serial step '$Describe' is bound to a module (a closure). " +
            'It would resolve commands in that module and then in global scope, ' +
            'so this script''s own functions would be invisible to it under ' +
            '`powershell -Command "& ./Invoke-CentralRustValidation.ps1"`. Pass a ' +
            'plain script block and put the captured values in -State.'
        )
    }
    $serialPhaseActions.Add([ordered]@{ Describe = $Describe; Action = $Action; State = $State })
}

function Get-CargoRootPackages {
    <#
    .SYNOPSIS
    The package names a cargo root's `--workspace` selection covers.

    .DESCRIPTION
    `--no-deps` is what makes this the WORKSPACE MEMBERS rather than the whole
    dependency graph, and `--offline` keeps a gate from reaching the network.
    Used for two things: scoping a filterset to names that exist in this root
    (nextest refuses a filterset naming a package it cannot see), and the
    fail-closed reconciliation against the partition scan.
    #>
    param([Parameter(Mandatory = $true)][string]$CargoRoot)

    Push-Location $CargoRoot
    try {
        $json = & cargo metadata --no-deps --format-version 1 --offline 2>$null
        if ($LASTEXITCODE -ne 0) {
            throw "cargo metadata failed in $CargoRoot; the battery partition cannot be scoped to it."
        }
        $metadata = ($json | Out-String) | ConvertFrom-Json
        return @($metadata.packages | ForEach-Object { [string]$_.name } | Sort-Object -Unique)
    }
    finally {
        Pop-Location
    }
}

}
# =========================== SETUP GUARD, CLOSES ============================
catch {
    # Everything above ran before the mutex existed, so there is nothing to
    # release - only the exit code to restore. Re-asserted rather than assumed,
    # because a successful `git rev-parse` (or any other native command added
    # here later) will have overwritten the seed with 0. Re-thrown unchanged:
    # the message is how a human finds out WHAT failed.
    $global:LASTEXITCODE = 1
    $script:centralValidationFailed = $true
    throw
}

try {
    # WHY THIS IS NOT ALWAYS THE INSTANT REFUSAL ANY MORE
    #
    # The machine-wide mutex used to cover the entire run, so a lane waiting for
    # the battery was waiting for every other lane's COMPILE as well -- into a
    # build directory that is private to each worktree and could never have
    # conflicted. Lanes routinely burned eight or more 45-second retries against
    # that. `Final` therefore splits: everything the partition clears runs with
    # no mutex at all, and only the machine-serial half takes it.
    #
    # The split is refused for a NARROWED Final (-Package or -FilterExpression)
    # for the same reason the release detour-identity gate refuses one: those
    # runs select their own test set, and intersecting a caller's filterset with
    # the partition's is a second place the selection could be got wrong for no
    # gain -- a narrowed Final is short anyway. Every other mode keeps the whole
    # run under the mutex, unchanged, so the mutex remains the one gate that
    # anything serial passes through.
    #
    # ITERATE SPLITS FOR THE SAME REASON AND WITH THE SAME PARTITION. It has to:
    # an iteration run that took the machine-wide mutex would be refused
    # INSTANTLY whenever any other lane held it, which is most of the time on
    # this machine, and a mode lanes cannot run is not a mode. Its
    # parallel-classified affected packages run under a slot; its
    # machine-serial ones are queued to the same serial phase everything else
    # uses. Nothing about the mutex's meaning changes.
    $splitBattery = ((($Mode -eq 'Final' -and $Package.Count -eq 0) -or $Mode -eq 'Iterate') -and
        [string]::IsNullOrWhiteSpace($FilterExpression))
    $batteryPartition = $null
    $iterationSelection = $null

    # -Since belongs to Iterate and to nothing else. Accepting it elsewhere and
    # ignoring it would silently accept a caller-supplied base that this mode
    # never uses.
    if ($Mode -eq 'Iterate') {
        if ([string]::IsNullOrWhiteSpace($Since)) {
            throw (
                '-Mode Iterate requires -Since <base-commit>: the commit this lane ' +
                'branched from. The selection is derived from the diff against it, ' +
                'and guessing a base would produce a silently narrower run. ' +
                'Typically: -Since (git merge-base HEAD master).'
            )
        }
        if ($Package.Count -gt 0) {
            throw (
                '-Mode Iterate derives its own package selection from the diff; ' +
                '-Package would be a second, conflicting answer to the same question. ' +
                'Use -Mode Test -Package <name> for a hand-picked selection.'
            )
        }
        # -FilterExpression is refused for the same reason, and for a second one:
        # a filter turns the split off, so Iterate would take the machine-wide
        # mutex outright and then dereference a partition it never computed.
        if (-not [string]::IsNullOrWhiteSpace($FilterExpression)) {
            throw (
                '-Mode Iterate derives its own selection from the diff; -FilterExpression ' +
                'would narrow it a second time and would also disable the parallel-slot ' +
                'split this mode depends on. Use -Mode Test -FilterExpression for a ' +
                'hand-picked selection.'
            )
        }
        # -Workspace is a THIRD narrowing, and the one that silently breaks the
        # promise this mode's own summary makes. The non-cargo gates are
        # workspace-conditional: with -Workspace Extractor, a changed Replay
        # python gate does not run, its path is merely "unowned", and nothing
        # reconciles it. Iterate derives its scope from the diff, across every
        # cargo root, or it does not run.
        if ($Workspace -ne 'All') {
            throw (
                "-Mode Iterate derives its scope from the diff and drives every cargo " +
                "root; -Workspace $Workspace would narrow it again AND would skip the " +
                'non-cargo gates of the other workspace, whose inputs the diff may have ' +
                'changed. Run -Mode Iterate with -Workspace All.'
            )
        }
        if ($Features.Count -eq 0) {
            # NAMED, because it is the failure this file's own -Features
            # documentation describes: a module behind an off-by-default feature
            # is not compiled, its tests do not exist, and the package's ordinary
            # tests supply a perfectly positive census.
            Write-Host (
                'Iteration run uses DEFAULT FEATURES. Code behind an off-by-default feature is ' +
                'not compiled and its tests do not run; pass -Features <package>/<feature> if ' +
                'your diff is inside one.'
            )
        }
    }
    elseif (-not [string]::IsNullOrWhiteSpace($Since)) {
        throw "-Since is only meaningful for -Mode Iterate; mode '$Mode' derives no selection from a diff."
    }

    # ONE graph read for everything that needs one. The partition, the iteration
    # selection and the -Package expansion all ask cargo the same question, and
    # asking it three times is three chances to be answered differently.
    # HOISTED from the selector-validation block far below, because the graph
    # decision needs it. $Lib/$Bin/$TestTarget are PARAMETERS and are never
    # reassigned anywhere in this file, so computing it here and computing it
    # there could only ever have produced the same answer; there is now one
    # definition rather than two.
    $hasSpecificTarget = $Lib -or $Bin.Count -gt 0 -or $TestTarget.Count -gt 0

    # Read only when something will actually USE it. Reading the graph walks
    # `cargo metadata` in all eleven cargo roots, so building it for a -Package
    # run whose expansion is then suppressed would make a previously
    # single-root invocation (a targeted -Bin, the Evidence tier) fail on an
    # unrelated root's metadata -- a caller that worked before this change.
    $packageExpansionWanted = ($Package.Count -gt 0 -and -not $hasSpecificTarget -and $Mode -ne 'Evidence')
    $batteryDependencyGraph = $null
    if ($splitBattery -or $packageExpansionWanted) {
        $batteryDependencyGraph = Get-BatteryWorkspaceDependencyEdges -RepoRoot $repoRoot
    }

    if ($Mode -eq 'Iterate') {
        # Derived BEFORE any slot is taken, like the partition: a selection that
        # cannot be derived is a refusal, never a run that tests a subset it
        # could not name.
        # -Features REACHES THE DEP-INFO PASS TOO. Discovering on the default
        # feature set while the battery builds the caller's would hide exactly
        # the computed include that sits behind an off-by-default feature --
        # the case they passed both flags for.
        $iterationSelection = Get-BatteryAffectedSelection -RepoRoot $repoRoot -Since $Since `
            -DependencyGraph $batteryDependencyGraph -DepInfo:$DepInfo -Features $Features
        Write-BatteryAffectedSelectionSummary -Selection $iterationSelection
    }
    elseif ($DepInfo) {
        # NOT a silent no-op. -DepInfo only has an input in the mode that
        # derives a selection from a diff; accepting it elsewhere and doing
        # nothing would let a caller believe a precision layer ran.
        throw '-DepInfo applies to -Mode Iterate only: it refines the diff-derived iteration selection, and no other mode has one to refine.'
    }

    if ($splitBattery) {
        # Computed BEFORE any slot is taken: a partition that cannot be derived
        # is a refusal, not a battery that runs everything in one half.
        $batteryPartition = Get-BatteryPartition -RepoRoot $repoRoot -DependencyGraph $batteryDependencyGraph
        Write-BatteryPartitionSummary -Partition $batteryPartition
        $slotCount = Get-CentralValidationParallelSlotCount
        $parallelSlotMutexes = @(0..($slotCount - 1) | ForEach-Object {
                [Threading.Mutex]::new($false, ("Local\battle_plan_central_rust_validation_parallel_{0}" -f $_))
            })
        Write-Host (
            'Battery parallel half: waiting for 1 of {0} machine slot(s) ({1} logical processors).' -f `
                $slotCount, [Environment]::ProcessorCount
        )
        $parallelSlotIndex = Enter-CentralValidationParallelSlot -Slots $parallelSlotMutexes
        Write-Host ("Battery parallel half: holding slot {0}." -f $parallelSlotIndex)
    }
    else {
        $ownsMutex = $mutex.WaitOne(0)
        if (-not $ownsMutex) {
            throw "Another centralized Rust validation process is already active."
        }
    }

    # WHY EVERY GATE FROM HERE TO THE CARGO RUNS IS IN THE PARALLEL HALF
    #
    # The partition classifies cargo TESTS. These gates are not tests in it, so
    # the fail-closed rule cannot decide them and each was read instead. All of
    # them are parallel-safe for the same structural reason: what they touch is
    # scoped either to the caller's own worktree or to its own process id.
    #
    #  * the python verifiers and unit suites read this worktree's source and
    #    write only into it (--repo-root $repoRoot), and the one that writes -
    #    verify_citation_integrity.py's allowance file - writes a per-worktree
    #    path; test_map_files_to_domains.py is stricter still, since its repo
    #    half runs the generator in --check, which writes nothing at all, and
    #    its synthetic half builds a throwaway git tree under TemporaryDirectory;
    #  * Test-PassiveCapturePolicy.ps1 and Test-HeadlessRawCaptureUnits.ps1 build
    #    their scratch under GetTempPath() with $PID / a fresh GUID in the name;
    #  * the C07 gate runs --verify, which writes nothing at all and reports the
    #    files it did not write;
    #  * Assert-PinnedRustToolchain only asks three binaries their version.
    #
    if ($Workspace -in @("All", "Extractor") -and
        $Mode -in @("Check", "Build", "Fast", "Integration", "Test", "Iterate", "Final", "Release")) {
        # These source checks run before Cargo and, for Release, before stale
        # promotion authority is removed. A changed shipping fold must first
        # force an explicit review of every wire-format transcription it backs.
        Invoke-Checked -Program "python" -Arguments @($worldFoldCitationTests)
        Invoke-Checked -Program "python" -Arguments @($citationIntegrityTests)
        Invoke-Checked -Program "python" -Arguments @($captureScorecardTests)
        Invoke-Checked -Program "python" -Arguments @($clockBaselineSummaryTests)
        Invoke-Checked -Program "python" -Arguments @($sinkReconciliationTests)
        Invoke-Checked -Program "python" -Arguments @($gapLedgerTests)
        Invoke-Checked -Program "python" -Arguments @($domainFileMapTests)
        Invoke-Checked -Program "python" -Arguments @($templatesEquipmentGateTests)
        Invoke-Checked -Program "powershell.exe" -Arguments @(
            "-NoProfile",
            "-ExecutionPolicy", "Bypass",
            "-File", $gapLedgerExpansionTests
        )
        # The partition decides which tests keep the machine-wide mutex. It runs
        # in every mode that compiles this workspace, not only in the Final that
        # uses it, because a partition helper that is only exercised by the tier
        # it gates is a helper nobody notices going wrong until a lane's battery
        # is the thing that noticed.
        $partitionTestOutput = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $batteryPartitionTests)
        if ($LASTEXITCODE -ne 0) {
            throw "battery partition source tests exited with code $LASTEXITCODE"
        }
        $partitionTestResult = (@($partitionTestOutput | Where-Object { $_ -and ([string]$_).Trim() }) |
                Select-Object -Last 1 | ConvertFrom-Json)
        if ([string]$partitionTestResult.schema -cne 'battle-plan.battery_partition_test.v1' -or
            [string]$partitionTestResult.status -cne 'PASS') {
            throw "battery partition source tests returned no authority"
        }

        Invoke-Checked -Program "python" -Arguments @(
            $worldFoldCitationVerifier,
            "--repo-root",
            $repoRoot
        )
        # -Full: THE ONE VERIFICATION PER BATTERY THAT CONSULTS NO CACHE.
        #
        # The gate remembers each file's result, keyed on that file's bytes, on
        # the allowance, on the script and on every question its resolution asked
        # about the rest of the tree; a file whose recorded inputs all still hold
        # is not swept again. That is what took `recorder_host`'s
        # `citation_integrity` test - the same script, run from inside the cargo
        # suite - from 70.8 s to a warm couple of seconds, and it is 66% of the
        # time the Extractor serial half holds the machine-wide mutex.
        #
        # This invocation is the belt to that braces. It runs OUTSIDE the mutex,
        # where a minute costs the machine nothing it was holding, so a landing
        # never depends on the cache being right - and it leaves the cache warm
        # for the cargo run that follows it in the same battery.
        Invoke-Checked -Program "python" -Arguments @(
            $citationIntegrityVerifier,
            "--repo-root",
            $repoRoot,
            "--full"
        )
    }

    # The Replay-owned source gate. It is deliberately NOT in the Extractor
    # block above: its inputs are `production_renderer.rs`, the Replay
    # dependency ledger and two committed Replay test fixtures, so a
    # `-Workspace Replay` run is exactly the run that can break it and exactly
    # the run that must catch it. Gating it on Extractor would have left the
    # Replay lane unable to see its own drift.
    #
    # Both are parallel-safe on the same terms as the verifiers above: they read
    # this worktree's source and write nothing into it. Every scratch file the
    # self-tests create lives under the system temporary directory, so a hard
    # kill mid-battery cannot leave debris in a tracked path.
    #
    # Evidence and Soak are in the list because they compile and run Replay
    # source, which is the whole criterion: the run that can break this gate is
    # the run that must catch it. Lock and Stats are out because they do not.
    if ($Workspace -in @("All", "Replay") -and
        $Mode -in @("Check", "Build", "Fast", "Integration", "Test", "Iterate",
                    "Final", "Release", "Evidence", "Soak")) {
        # The suites first: they are what proves the two gates below still fail
        # when they should, rather than passing because they stopped looking.
        Invoke-Checked -Program "python" -Arguments @($replayDependencyLedgerTests)
        Invoke-Checked -Program "python" -Arguments @($replayDependencyLedger)
        Invoke-Checked -Program "python" -Arguments @($replayFixtureProvenance)
    }

    # The passive capture-policy gate runs over a WIDER mode set than the source
    # checks above, and the difference is deliberate rather than incidental.
    #
    # The checks above validate the shipping world fold and the two report
    # readers: their inputs are the fold citation tables and the Python readers,
    # and an Evidence run - which requires an explicit -Package/-Lib/-Bin/
    # -TestTarget/-FilterExpression selection and exists to drive the RE-evidence
    # binaries - cannot reach them. Running them there would only lengthen a
    # targeted harness invocation.
    #
    # This gate's input is the shipped COLLECTOR SOURCE, which an Evidence run
    # does compile and can change. Its own history is the argument: it sat red
    # for a week (a 2026-07-29 rename landed with a stale required-substring in
    # the same commit) because nothing lighter than Final ever executed it, and
    # leaving Evidence out reproduces that hole for exactly the tier most likely
    # to be iterated on. It costs seconds. The Final/Release path below
    # re-checks the returned authority object; here an exit or schema failure is
    # enough.
    #
    # Soak, Lock and Stats are out for structural reasons, not by omission: Soak
    # refuses any -Workspace but Replay (see the guard below), so the -Workspace
    # condition already excludes it, and Lock and Stats are lockfile/cache
    # utility modes that compile no collector source.
    if ($Workspace -in @("All", "Extractor") -and
        $Mode -in @("Check", "Build", "Fast", "Integration", "Test", "Iterate", "Evidence", "Final", "Release")) {
        $passivePolicyEveryMode = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $passiveCapturePolicyTest)
        if ($LASTEXITCODE -ne 0) {
            throw "passive capture-policy source tests exited with code $LASTEXITCODE"
        }
        # The verdict is the LAST non-empty line: suites are allowed to narrate
        # (the harness unit suite prints its executed supervision probes'
        # phase= lines) and the JSON authority line always comes last. Feeding
        # the whole transcript to ConvertFrom-Json failed on the first narrated
        # line and read as "no authority" for a suite that passed.
        $passivePolicyEveryModeResult = (@($passivePolicyEveryMode | Where-Object { $_ -and ([string]$_).Trim() }) | Select-Object -Last 1 | ConvertFrom-Json)
        if ([string]$passivePolicyEveryModeResult.Schema -cne 'hoi4-passive-capture-policy-test-v1' -or
            [string]$passivePolicyEveryModeResult.Status -cne 'pass') {
            throw "passive capture-policy source tests returned no authority"
        }

        # The policy test above is a source census plus the lifecycle mirror.
        # This one EXECUTES the harness's own decision logic -- parameter
        # admission, the CPU-affinity plan, receipt integer admission, the
        # closed property set, the child-command quoting and the
        # collector_record argument plan -- against tables and lane EQ's real
        # receipt fixture, with the harness itself never run. It is here rather
        # than in a lane's own gate because every one of those was previously
        # exercised only by a live capture: a dropped collector flag or a mask
        # built from an index range cost a game hour to find and could not be
        # found any other way. Runs in about a second.
        $harnessUnits = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $headlessRawCaptureUnitTest)
        if ($LASTEXITCODE -ne 0) {
            throw "headless raw-capture harness unit tests exited with code $LASTEXITCODE"
        }
        $harnessUnitsResult = (@($harnessUnits | Where-Object { $_ -and ([string]$_).Trim() }) | Select-Object -Last 1 | ConvertFrom-Json)
        if ([string]$harnessUnitsResult.Schema -cne 'hoi4-headless-raw-capture-unit-test-v1' -or
            [string]$harnessUnitsResult.Status -cne 'pass' -or
            [int]$harnessUnitsResult.Cases -lt 200) {
            throw "headless raw-capture harness unit tests returned no authority"
        }
    }

    if ($Mode -in @("Check", "Build", "Fast", "Integration", "Test", "Iterate", "Final", "Release")) {
        # The three C07 tree-library artifacts under
        # portable_replay\native\render_stream\src are generated whole from the
        # hash-pinned shipped installed content and compiled straight into the crate,
        # so a hand-edit to a pin literal or a bucket row changes what the
        # complete static tree pool admits. This re-derives all three and fails
        # on any drift. It is unconditional on -Workspace because the portable
        # crates are validated for every selection: both halves link them.
        #
        # The render_stream crate runs the identical script as a test target, so
        # a lane editing the pins sees this without central validation. Running
        # it here as well is what covers the Modes that never reach that crate's
        # tests, and costs about three seconds.
        if (-not (Test-Path -LiteralPath $c07TreeLibraryGenerator -PathType Leaf)) {
            throw "C07 tree-library generator is missing: $c07TreeLibraryGenerator"
        }
        $c07Python = $null
        try { $c07Python = Get-Command "python" -ErrorAction Stop } catch { $c07Python = $null }
        if (-not $c07Python) {
            throw "python is not on PATH; the C07 generated tree-library gate cannot run."
        }
        # `--allow-committed-fallback` does NOT weaken the gate on a machine with
        # the game: that path still runs the full exact-producer verify and still
        # exits zero, and the assertion below is unchanged. The switch only
        # changes what happens when the pinned installed content are ABSENT, which used
        # to be reported as artifact drift -- a false statement, and the reason
        # master CI was red on every run. Absence now takes the reduced
        # committed-artifact check and a non-zero exit of its own.
        $c07Arguments = @("--verify", "--allow-committed-fallback")
        if (-not [string]::IsNullOrWhiteSpace($C07GameRoot)) {
            if (-not (Test-Path -LiteralPath $C07GameRoot -PathType Container)) {
                throw "-C07GameRoot is not an existing directory: $C07GameRoot"
            }
            $c07Arguments += @("--game-root", $C07GameRoot)
        }
        Write-Host ("> python {0} {1}" -f $c07TreeLibraryGenerator, ($c07Arguments -join " "))
        $c07ReceiptLines = & python $c07TreeLibraryGenerator @c07Arguments
        $c07Exit = $LASTEXITCODE
        if ($c07Exit -eq 0) {
            Assert-C07GeneratedTreeLibraryReceipt `
                -ReceiptJson (($c07ReceiptLines | Out-String)) `
                -PinSource $c07TreeLibraryPinSource
        }
        elseif ($c07Exit -eq 3) {
            # Release is the one tier that MINTS AUTHORITY, and the reduced check
            # cannot carry the claim a release makes: that the compiled tree
            # library is what the exact producer derives from the shipped map
            # files. A release cut on a machine without the game would ship that
            # claim unproven, so absence is fatal here even though it is
            # survivable everywhere else.
            if ($Mode -eq "Release") {
                throw "the C07 tree-library gate cannot run without the pinned the game install, and Release will not accept the reduced committed-artifact check. Cut the release on a machine with the game, or pass -C07GameRoot pointing at one."
            }
            Assert-C07CommittedArtifactReceipt `
                -ReceiptJson (($c07ReceiptLines | Out-String)) `
                -PinSource $c07TreeLibraryPinSource
        }
        else {
            throw "C07 generated tree-library artifacts drifted from the exact producer (generator exited $c07Exit)."
        }
    }

    $extractorReleaseRequested = $Mode -eq "Release" -and $Workspace -in @("All", "Extractor")
    $fullExtractorRelease = $Mode -eq "Release" -and $Workspace -eq "All" -and $Package.Count -eq 0 -and [string]::IsNullOrWhiteSpace($FilterExpression)
    if (-not [string]::IsNullOrWhiteSpace($ProductionCaptureReceiptPath) -and
        -not $fullExtractorRelease) {
        throw 'production capture evidence is accepted only by an unfiltered All-workspace Release'
    }
    if ($extractorReleaseRequested) {
        # Remove authority before any toolchain or release preflight can fail.
        # A ZIP without both canonical receipts is deliberately non-promotable.
        Remove-ReleasePromotionState `
            -Paths @($artifactAttestationPath, $portablePackageReceiptPath) `
            -Context 'stale release authority cleanup'
        $recorderVersion = Get-CargoPackageVersion -ManifestPath (Join-Path $repoRoot "extractor\native\recorder_host\Cargo.toml")
        $replayVersion = Get-CargoPackageVersion -ManifestPath (Join-Path $repoRoot "replay_interface\native\replay_app\Cargo.toml")
        $candidatePackagePaths = @(
            @($recorderVersion, $replayVersion) |
                Select-Object -Unique |
                ForEach-Object { Join-Path $targetDir ("release\battle-plan-{0}-windows-x86_64.zip" -f $_) }
        )
        Remove-ReleasePromotionState `
            -Paths $candidatePackagePaths `
            -Context 'stale release package cleanup'
        if ($recorderVersion -cne $replayVersion) {
            throw "product executable versions differ: recorder_host=$recorderVersion replay_app=$replayVersion"
        }
        $portablePackagePath = $candidatePackagePaths[0]
    }

    Assert-PinnedRustToolchain

    $releaseRuntimeLinkage = $null
    if ($Mode -eq 'Release') {
        $releaseRuntimeLinkage = Enable-StaticMsvcCrtForCentralRelease
    }

    # The safe default. It is REMOVED again further down when, and only when,
    # cargo can be shown to select this very directory without it - see the
    # cross-worktree block after the cache decision.
    $env:CARGO_TARGET_DIR = $targetDir

    # THE COMPILE CACHE, AND EXACTLY WHAT IT IS WORTH.
    # Measured by lane JM; evidence in extractor/native/runs/jm_compile_cache/.
    #
    # sccache hashes the CARGO_* values it inherits, and CARGO_TARGET_DIR is one
    # of them. Exporting it makes every worktree's compilations unique: a second
    # checkout of the same commit, compiling identical sources against a cache
    # the first checkout had just filled, scored ZERO hits out of 61. Removing
    # that one variable and letting the repo's .cargo/config.toml select the
    # SAME directory turned the same experiment into 43 hits out of 61 (70.5%),
    # and the Replay workspace's compile from 222.8 s to 169.4 s.
    #
    # What shares and what does not is the important part, and it is the right
    # split rather than a lucky one: the 43 are registry DEPENDENCIES, whose
    # manifest directory is identical in every worktree. The 18 misses are the
    # workspace's own crates, which sccache keeps apart because their
    # CARGO_MANIFEST_DIR differs - and they MUST stay apart, because this repo
    # bakes that path into test binaries that then scan the tree
    # (inline_detour source bans, citation integrity, the script vocabulary
    # sweep). A workspace artifact shared from another worktree would produce a
    # battery that certifies a tree the lane never changed.
    #
    # Each worktree still gets its own target dir, so no cargo build lock is
    # shared and lane isolation is untouched.
    #
    # SCCACHE_DIR is the location knob and is deliberately NOT set here: unset
    # means sccache's own per-user default, which is where this machine's
    # existing cache already sits. Export SCCACHE_DIR to put it elsewhere.
    if (Get-Command sccache -ErrorAction SilentlyContinue) {
        if (-not $env:SCCACHE_CACHE_SIZE) {
            $env:SCCACHE_CACHE_SIZE = "30G"
        }
        if (-not $env:SCCACHE_SERVER_PORT) {
            $env:SCCACHE_SERVER_PORT = "4227"
        }
    }

    $compileCache = [pscustomobject]@{ Usable = $false; Notice = $null }
    if ($CacheMode -eq "Reusable") {
        # Probed AFTER the size and port above, so the server this starts is
        # the one those settings describe.
        $compileCache = Get-CompileCacheState
    }
    if ($compileCache.Usable) {
        $env:RUSTC_WRAPPER = "sccache"
        # sccache cannot cache an incremental compilation: the two are
        # alternatives, not additives. That, and the MODE's own decision about
        # incremental compilation, are both made in one place now.
        Set-CargoIncrementalPolicy -Mode $Mode -CompileCacheInUse $true
        # THE PROBE ABOVE ONLY PROVES THE SERVER WAS ALIVE AT PREFLIGHT.
        # RUSTC_WRAPPER makes sccache the driver for every rustc call for the
        # rest of the run, so a server that dies mid-battery would otherwise
        # turn each remaining compile into a hard error - a preflight check
        # cannot fail open on its own. This is sccache's own switch for
        # "if you cannot reach the server, run the real compiler instead",
        # which converts that class from a failed battery into a slow one.
        $env:SCCACHE_IGNORE_SERVER_IO_ERROR = "1"
    }
    else {
        if ($null -ne $compileCache.Notice) {
            # NAMED, never silent, and on BOTH streams. Write-Warning alone is
            # suppressible with -WarningAction SilentlyContinue or a
            # $WarningPreference of SilentlyContinue, and a notice an operator
            # can lose is the same defect as no notice: a run that quietly
            # changed its own compiler configuration. A slower battery is
            # never a reason to refuse one.
            Write-Host $compileCache.Notice
            Write-Warning $compileCache.Notice
        }
        Remove-Item Env:RUSTC_WRAPPER -ErrorAction SilentlyContinue
        Set-CargoIncrementalPolicy -Mode $Mode -CompileCacheInUse $false
    }

    # CROSS-WORKTREE REUSE: drop CARGO_TARGET_DIR from the environment, but only
    # after cargo itself confirms it lands in the same place without it. This is
    # attempted only when the cache is actually in use - with no wrapper there is
    # nothing to gain, and exporting the variable is the cheaper, duller path.
    #
    # FAIL-SAFE, NOT FAIL-FAST: every workspace this run may touch has to agree.
    # If any of them resolves elsewhere, or cargo cannot be asked, the variable
    # stays exported and the only thing lost is cross-worktree sharing. A wrong
    # target dir would strand the release ZIP and attestation paths the runner
    # computes from $targetDir while the build itself quietly succeeded
    # somewhere else, which is far worse than a slower compile.
    if ($compileCache.Usable) {
        # EVERY cargo root this runner can enter, not just the two workspaces.
        # The standalone roots below are separate cargo roots with their own
        # Cargo.lock, and the runner invokes cargo (and nextest through cargo)
        # from inside each of them. Today they all descend from the single
        # .cargo/config.toml at the repo root and therefore resolve to the same
        # target dir - but the guard is not allowed to ASSUME that: a
        # .cargo/config.toml added under any one of them would override the
        # root config for that root alone, and the run would still have unset
        # CARGO_TARGET_DIR on the strength of the two workspaces agreeing.
        # Then that root would build somewhere the cleanup, timing and release
        # paths never look. Ask all of them.
        # DERIVED, NOT RESTATED. This list was a hand-kept copy of the root
        # inventory in BatteryPartition.ps1, and a copy is exactly how
        # `portable_replay\native\terrain_fixture` came to be absent from three
        # separate lists at once (lane LD). A root this guard does not ask about
        # is a root that may build somewhere the cleanup, timing and release
        # paths never look.
        # NOT filtered to manifests that exist. A missing manifest makes
        # Test-ConfigSelectsTargetDirectory return false, which KEEPS
        # CARGO_TARGET_DIR set -- the safe direction. Filtering it out instead
        # would let a root the guard could not read count as agreement.
        $targetDirManifests = @(
            Get-BatteryCargoRootPaths -RepoRoot $repoRoot |
                ForEach-Object { Join-Path $_ "Cargo.toml" }
        )

        $configSelectsSameDir = $targetDirManifests.Count -gt 0
        foreach ($manifest in $targetDirManifests) {
            if (-not (Test-ConfigSelectsTargetDirectory -ManifestPath $manifest -ExpectedTargetDir $targetDir)) {
                $configSelectsSameDir = $false
                break
            }
        }

        if ($configSelectsSameDir) {
            Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
            Write-Host ("Compile cache: cross-worktree reuse ENABLED (CARGO_TARGET_DIR unset; cargo selects {0} from .cargo/config.toml)." -f $targetDir)
        }
        else {
            Write-Host ("Compile cache: cross-worktree reuse DISABLED - cargo does not select {0} without CARGO_TARGET_DIR, so the variable stays set. Dependencies will not be shared with other checkouts; nothing else about this run changes." -f $targetDir)
        }
    }
    if (Get-Command lld-link -ErrorAction SilentlyContinue) {
        $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = "lld-link.exe"
    }

    if ($Mode -eq "Stats") {
        # Same fail-open rule: a report about an absent or broken optional tool
        # is a notice, not a failure. This asks the SAME question the compile
        # path asks - checking only Get-Command would send a present-but-broken
        # sccache into Invoke-Checked, where PowerShell 5.1 can terminate on the
        # native command's stderr before the exit-code check is ever reached.
        $statsCache = Get-CompileCacheState
        if (-not $statsCache.Usable) {
            Write-Host $statsCache.Notice
            Write-Warning $statsCache.Notice
            return
        }
        # Print what the BOUNDED probe already read. Asking sccache a second
        # time through Invoke-Checked reopens both holes this mode is meant to
        # be free of: that call has no timeout, and PowerShell 5.1 can
        # terminate on its native stderr before the exit-code check runs. The
        # server can also die between the two calls.
        Write-Host $statsCache.StatsText
        return
    }

    # THE ORPHAN GUARD, BEFORE ANY MODE COMPILES ANYTHING. A package under the
    # battery trees that no registered cargo root has as a MEMBER is invisible
    # to every selection this runner can make: `--workspace` does not reach it,
    # `cargo -p` panics on it, and its tests have therefore never run. The
    # battery still goes green, because green means "everything I selected
    # passed" and the package was never selected.
    #
    # `portable_terrain_fixture` sat in that state for eleven days with three
    # tests nobody ran (lane LD). It is placed here, after the Stats early
    # return and before the first compile, because every mode below reports a
    # verdict about "the repository" and none of them could see this crate.
    # It costs one `cargo metadata --no-deps --offline` per root: 1.4 s for all
    # eleven, measured, and it is not conditional on the diff -- an orphan nobody
    # edited is exactly the one that stays hidden.
    Assert-BatteryEveryPackageIsRegistered -RepoRoot $repoRoot

    if ($Mode -in @('Final', 'Release')) {
        $interactionClosureOutput = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $productionInteractionClosureTest)
        if ($LASTEXITCODE -ne 0) {
            throw "production interaction-closure source tests exited with code $LASTEXITCODE"
        }
        $interactionClosureResult = (($interactionClosureOutput -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
        if ([string]$interactionClosureResult.schema -cne 'battle-plan.production_interaction_closure_test.v1' -or
            [string]$interactionClosureResult.status -cne 'PASS' -or
            [string]$interactionClosureResult.policy -cne 'passive_injected_observer') {
            throw "production interaction-closure source tests returned no authority"
        }

        # A gate that passes because its evidence is ABSENT is the inverse of a
        # gate, and PowerShell's $null-to-zero coercion makes that the DEFAULT
        # spelling. ProductionCapture.Promotion.ps1 required a per-domain
        # effective_failure_status_flags_or the manifest never published, and
        # passed every domain of every capture on the resulting 0. These cases
        # prove the coercion is real, that the Required* accessors refuse it,
        # and that no gate clause reads a zero-required field through a cast.
        $promotionGateAbsenceOutput = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $promotionGateAbsenceTest)
        if ($LASTEXITCODE -ne 0) {
            throw "promotion-gate absence proof exited with code $LASTEXITCODE"
        }
        $promotionGateAbsenceResult = (($promotionGateAbsenceOutput -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
        if ([string]$promotionGateAbsenceResult.schema -cne 'battle-plan.promotion_gate_absence_test.v1' -or
            [string]$promotionGateAbsenceResult.status -cne 'PASS') {
            throw "promotion-gate absence proof returned no authority"
        }

        $staticCrtTestOutput = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $portableStaticCrtTest)
        if ($LASTEXITCODE -ne 0) {
            throw "portable-product static-CRT admission tests exited with code $LASTEXITCODE"
        }
        $staticCrtTestResult = (($staticCrtTestOutput -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
        if ([string]$staticCrtTestResult.schema -cne 'battle-plan.portable_product_static_crt_test.v1' -or
            [string]$staticCrtTestResult.status -cne 'PASS') {
            throw "portable-product static-CRT admission tests returned no authority"
        }

    }

    if ($fullExtractorRelease) {
        # Cargo does not delete outputs for targets removed or renamed since a
        # prior build. Prove that every canonical product path starts absent so
        # a successful current Release is the only possible producer.
        $releaseArtifactFreshness = Clear-ProductionArtifactOutputs -RepoRoot $repoRoot
        $releaseBuildEnvironmentBefore = Get-ProductionBuildEnvironment
        $releaseInputDigestBefore = Get-ProductionRustInputDigest -RepoRoot $repoRoot
    }

    if ($Mode -eq "Lock" -and
        ($Package.Count -gt 0 -or -not [string]::IsNullOrWhiteSpace($FilterExpression) -or
         $AllTargets -or $hasSpecificTarget -or $Timings)) {
        throw "Lock regenerates the selected top-level workspace lockfile and does not accept package, filter, target, or timing selectors."
    }
    if ($AllTargets -and $hasSpecificTarget) {
        throw "-AllTargets cannot be combined with -Lib, -Bin, or -TestTarget."
    }
    # Iterate joins the tiers that own their own target selection: it always
    # passes `--all-targets`, so a caller's target selector would either be
    # redundant or would silently narrow a selection this mode derived.
    if ($Mode -in @("Final", "Soak", "Release", "Iterate") -and ($AllTargets -or $hasSpecificTarget)) {
        throw "Final, Soak, and Release own their target sets; target selectors are supported by Check, Build, Fast, Integration, Test, and Evidence."
    }
    if ($Mode -eq "Soak" -and $Workspace -ne "Replay") {
        throw "Soak is a replay-only tier. Use -Workspace Replay."
    }
    if ($Mode -eq "Fast" -and $TestTarget.Count -gt 0) {
        throw "Fast runs unit-test binaries. Use Integration or Test with -TestTarget for integration tests."
    }
    if ($Mode -eq "Integration" -and ($Lib -or $Bin.Count -gt 0)) {
        throw "Integration runs integration-test binaries. Use Fast or Test with -Lib/-Bin for unit tests."
    }
    if ($Mode -in @("Fast", "Integration") -and $AllTargets) {
        throw "Fast and Integration are narrow tiers and do not accept -AllTargets. Use Test or Final for broader validation."
    }
    if ($Mode -eq "Evidence" -and -not ($FilterExpression -or $hasSpecificTarget -or $Package.Count -gt 0)) {
        throw "Evidence tests require an explicit -Package, -Lib, -Bin, -TestTarget, or -FilterExpression selection."
    }

    $targetSelection = @()
    if ($AllTargets) {
        $targetSelection += "--all-targets"
    }
    else {
        if ($Lib) {
            $targetSelection += "--lib"
        }
        foreach ($name in $Bin) {
            $targetSelection += @("--bin", $name)
        }
        foreach ($name in $TestTarget) {
            $targetSelection += @("--test", $name)
        }
    }
    $timingSelection = @()
    if ($Timings) {
        $timingSelection += "--timings"
    }
    $featureSelection = @()
    # Explicit features are honoured by the narrow deterministic tiers only.
    # `Final` is the canonical configuration and must not vary with a flag, and
    # the Evidence tier owns its own feature selection below.
    # The mode restriction itself is enforced at the top of the script, ahead of
    # the `Stats` early return and the mutex; by here it has already held.
    $requestedFeatureSelection = @()
    if ($Features.Count -gt 0) {
        $requestedFeatureSelection = @('--features', ($Features -join ','))
    }
    $evidenceClientDrivingRequested = $Bin -ccontains 'collector_record_evidence'
    if ($evidenceClientDrivingRequested) {
        if ($Mode -cne 'Evidence' -or $Workspace -cne 'Extractor' -or
            $Package.Count -ne 1 -or $Package[0] -cne 'recorder_host') {
            throw "collector_record_evidence is isolated to explicit Evidence -Workspace Extractor -Package recorder_host"
        }
        $featureSelection = @('--features', 'recorder_host/evidence-client-driving')
    }
    elseif ($Mode -cne 'Evidence' -and @($Bin | Where-Object { $_ -like '*evidence*' }).Count -gt 0) {
        throw "evidence binaries are accepted only by the explicit Evidence tier"
    }

    # BOTH workspace roots, independent of the -Workspace selection. The
    # standalone-root derivation below subtracts the workspaces from the root
    # inventory, and subtracting only the SELECTED ones would turn the
    # unselected workspace into a "standalone root" and try to enter it.
    $workspaceRootsAll = @(
        (Join-Path $repoRoot "extractor\native"),
        (Join-Path $repoRoot "replay_interface\native")
    )
    $workspaceRoots = switch ($Workspace) {
        "Extractor" { @(Join-Path $repoRoot "extractor\native") }
        "Replay" { @(Join-Path $repoRoot "replay_interface\native") }
        default { $workspaceRootsAll }
    }

    # ======================= THE SELECTED PACKAGE SET =========================
    #
    # `--package X` builds X and X's DEPENDENCIES. It does NOT build anything
    # that IMPORTS X, so a gate narrowed to a leaf crate never compiles the
    # crates that use it: `-Package runtime_text_inputs_live_lane` compiled 3
    # repository packages while 5 direct importers - collector,
    # character_presentation_live_lane, decision_panel_live_lane,
    # production_render_submit_live_lane, scripted_state_flags_variables_live_lane -
    # were not compiled at all. Three lanes did that expansion by hand before it
    # became the runner's job.
    #
    # THE RAW `$Package` LIST IS NEVER REASSIGNED. Every policy guard in this
    # file asks `$Package.Count -eq 0` to tell a narrowed run from a full one -
    # the Final split, the full-Release authority, the Evidence isolation, the
    # detour gate, the standalone roots - and an expanded list would tell some of
    # them a different story. The expansion lives in its own variable and only
    # the cargo SELECTION reads it.
    $selectedPackageNames = @()
    $expansionSuppressed = $false
    if ($Package.Count -gt 0) {
        if ($hasSpecificTarget) {
            # -Lib / -Bin / -TestTarget select TARGETS BY NAME across every
            # selected package, so adding a dependent that has no target of that
            # name either fails the run or silently contributes nothing. The
            # caller asked for one target; expansion is skipped and SAID, never
            # done quietly and never done wrong.
            $expansionSuppressed = $true
            $selectedPackageNames = @($Package)
            $notice = (
                'PACKAGE EXPANSION SKIPPED: -Lib/-Bin/-TestTarget name targets across every ' +
                'selected package, so the dependents of ' + ($Package -join ', ') + ' were NOT ' +
                'compiled by this run. Re-run without a target selector to compile them.'
            )
            Write-Host $notice
            Write-Warning $notice
        }
        elseif ($Mode -eq 'Evidence') {
            # The Evidence tier is isolated to one package with one feature by
            # the guard above; widening it would change what that feature
            # unifies over.
            $selectedPackageNames = @($Package)
        }
        else {
            $scopePackages = [System.Collections.Generic.SortedSet[string]]::new([StringComparer]::Ordinal)
            foreach ($root in $workspaceRoots) {
                foreach ($name in (Get-CargoRootPackages -CargoRoot $root)) { [void]$scopePackages.Add([string]$name) }
            }
            # Names -> manifests, scoped to the roots THIS RUN ENTERS, so
            # `-Workspace Replay -Package <an extractor crate>` is a refusal here
            # rather than a run that skips every root and reports green.
            $seedManifests = Resolve-BatteryPackageNames -RepoRoot $repoRoot -Names $Package `
                -ScopePackages @($scopePackages) -DependencyGraph $batteryDependencyGraph `
                -ScopeRoots @($workspaceRoots)
            $selectedPackageNames = @(Get-BatteryDependentClosure -RepoRoot $repoRoot `
                    -SeedManifests $seedManifests -DependencyGraph $batteryDependencyGraph)
            $added = @($selectedPackageNames | Where-Object { $Package -notcontains $_ })
            Write-Host (
                '-Package expanded to its dependents: {0} requested -> {1} selected ({2} importer(s) added).' -f `
                    $Package.Count, $selectedPackageNames.Count, $added.Count
            )
            if ($added.Count -gt 0) { Write-Host ('  added: {0}' -f ($added -join ', ')) }
        }
    }
    elseif ($Mode -eq 'Iterate') {
        $selectedPackageNames = @($iterationSelection.AffectedPackages)
        if ($selectedPackageNames.Count -eq 0) {
            # REFUSED RATHER THAN RUN AS A NO-OP, and the reason is not
            # bookkeeping. A diff that touches only scripts, gates, documents or
            # evidence still has Rust tests that can break: this repository has
            # tests that SCAN THE TREE at run time - the script status
            # vocabulary sweep, the inline-detour source bans, citation
            # integrity - and those are exactly the tests no dependency graph
            # can attribute to a changed file. "No package affected" therefore
            # means "this derivation cannot tell you what broke", not "nothing
            # broke", and the honest answer is the full battery.
            throw (
                'the iteration selection names no cargo package: every changed path ' +
                'belongs to no cargo package. That is NOT the same as "no test is ' +
                'affected" -- this repository has tests that scan the source tree at ' +
                'run time, and no dependency graph can attribute a script or document ' +
                'change to them. Run Invoke-Validation.ps1 -Mode Landing.'
            )
        }
    }

    # A PACKAGE NO CARGO ROOT HAS AS A MEMBER IS A REFUSAL, NOT A DROP.
    #
    # Such a package cannot be selected by any battery, and asking for it is
    # worse than useless: `cargo check -p <it>` from a root where it is a path
    # DEPENDENCY but not a MEMBER makes cargo PANIC (exit 101, "did not find
    # features for (PackageId ...) within activated_features"). `--workspace`
    # never reaches it either, so NO test of that package runs anywhere in this
    # repository.
    #
    # This used to print a notice and drop the package. Lane LD retired that
    # arm, because the one package it was written for -- `portable_terrain_fixture`
    # -- turned out not to be a structural fact at all: it had a live consumer,
    # three tests of its own, and five structurally identical sibling crates
    # that WERE registered. It was simply missing from the root inventory, and a
    # notice is exactly how that survived from 2026-08-05 to now. The condition
    # the notice described is gone; the guard against it recurring is not.
    #
    # So the generic guard stays and it THROWS. An unregistered package is a
    # gap in the battery's coverage, and this file exists to refuse rather than
    # to report green having tested nothing. The cost of being wrong here is one
    # refusal with a one-line fix named in it; the cost of dropping is a crate
    # whose tests nobody runs and nobody notices.
    if ($selectedPackageNames.Count -gt 0) {
        $allRootMembers = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        foreach ($root in (Get-BatteryCargoRootPaths -RepoRoot $repoRoot)) {
            if (-not (Test-Path -LiteralPath (Join-Path $root 'Cargo.toml') -PathType Leaf)) { continue }
            foreach ($name in (Get-CargoRootPackages -CargoRoot $root)) { [void]$allRootMembers.Add([string]$name) }
        }
        $unselectable = @($selectedPackageNames | Where-Object { -not $allRootMembers.Contains([string]$_) } | Sort-Object)
        if ($unselectable.Count -gt 0) {
            throw (
                ('{0} affected package(s) are a member of NO cargo root: {1}. No battery can ' +
                 'select them, `--workspace` does not reach them, and `cargo -p` PANICS on them, ' +
                 'so no test of theirs runs anywhere in this repository -- this run would have ' +
                 'compiled nothing for a change that touched them. Register each one as a cargo ' +
                 'root (or as a member of an existing workspace) in the $script:BatteryCargoRoots ' +
                 'inventory in BatteryPartition.ps1, which is the single list the runner derives ' +
                 'every root from.') -f $unselectable.Count, ($unselectable -join ', ')
            )
        }
    }

    # THE GLOBAL NON-EMPTY INVARIANT. A per-root selection that is empty is
    # correctly skipped, but if EVERY root skips, the run compiled nothing, ran
    # nothing, and would return 0. Counted across the whole loop and asserted
    # after it.
    $cargoRootsEntered = 0
    $selectedPackagesCovered = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)

    foreach ($workspaceRoot in $workspaceRoots) {
        $workspaceName = if ($workspaceRoot -like "*\extractor\native") { "Extractor" } else { "Replay" }
        $nextestConfigSelection = if ($workspaceName -eq "Replay") { @("--config-file", $nextestConfig) } else { @() }
        $nextestCiProfileSelection = if ($workspaceName -eq "Replay") { @("--profile", "ci") } else { @() }
        Push-Location $workspaceRoot
        try {
            if ($Mode -eq "Lock") {
                Invoke-Checked -Program "cargo" -Arguments @("generate-lockfile")
                continue
            }

            $selection = @("--locked")
            if ($selectedPackageNames.Count -gt 0) {
                # SCOPED TO THIS ROOT. `--package` naming a package the current
                # cargo root cannot see is a hard cargo error, and the expansion
                # deliberately spans roots.
                $rootMembers = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
                foreach ($name in (Get-CargoRootPackages -CargoRoot $workspaceRoot)) { [void]$rootMembers.Add([string]$name) }
                $scopedNames = @($selectedPackageNames | Where-Object { $rootMembers.Contains([string]$_) } | Sort-Object)
                if ($scopedNames.Count -eq 0) {
                    # This root holds none of the selection. It is SKIPPED, and
                    # it must never fall through to `--workspace`, which would
                    # run the whole root the caller narrowed away from. The
                    # invariant after the loop is what refuses the case where
                    # EVERY root skipped.
                    Write-Host (
                        '{0}: no selected package lives in this cargo root; skipping it.' -f $workspaceName
                    )
                    continue
                }
                foreach ($name in $scopedNames) {
                    $selection += @("--package", $name)
                    [void]$selectedPackagesCovered.Add([string]$name)
                }
            }
            else {
                $selection += "--workspace"
            }
            $cargoRootsEntered++

            switch ($Mode) {
                "Check" {
                    Invoke-Checked -Program "cargo" -Arguments (@("check") + $timingSelection + $requestedFeatureSelection + $targetSelection + $selection)
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "check"
                }
                "Build" {
                    Invoke-Checked -Program "cargo" -Arguments (@("build") + $timingSelection + $requestedFeatureSelection + $targetSelection + $selection)
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "build"
                }
                "Test" {
                    $arguments = @("nextest", "run") + $nextestConfigSelection +
                        $timingSelection + $requestedFeatureSelection + $targetSelection + $selection
                    if ($FilterExpression) {
                        $arguments += @("-E", $FilterExpression)
                    }
                    Invoke-Checked -Program "cargo" -Arguments $arguments
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "test"
                }
                "Iterate" {
                    # THE ITERATION RUN. The cargo SELECTION is the affected
                    # packages scoped to this root; the partition then decides
                    # only WHICH SIDE OF THE MUTEX each of their tests runs on,
                    # exactly as it does for Final. Two things are deliberately
                    # not reinvented here: the halves come from
                    # Get-BatteryRootHalves, and the machine-serial half is
                    # queued through Add-SerialPhaseAction into the same serial
                    # phase every other mode uses.
                    # THE SAME fail-closed reconciliation Final does. A selected
                    # package the partition classified as neither serial nor
                    # parallel would fall out of `Get-BatteryRootHalves` into
                    # the complement -- that is, silently into the half with no
                    # mutex. It refuses instead.
                    Assert-BatteryPartitionCoversPackages `
                        -ObservedPackages $scopedNames `
                        -SerialPackages $batteryPartition.Serial `
                        -ParallelPackages $batteryPartition.Parallel
                    $rootHalves = Get-BatteryRootHalves -Partition $batteryPartition -RootPackages $scopedNames
                    $baseArguments = @("nextest", "run", "--all-targets", "--no-fail-fast") +
                        $nextestConfigSelection + $nextestCiProfileSelection +
                        $timingSelection + $requestedFeatureSelection + $selection
                    $baseListArguments = @("nextest", "list", "--all-targets") +
                        $nextestConfigSelection + $nextestCiProfileSelection +
                        $requestedFeatureSelection + $selection

                    $parallelArguments = $baseArguments
                    $parallelListArguments = $baseListArguments + @("--message-format", "json")
                    if ($null -ne $rootHalves.Parallel) {
                        $parallelArguments = $baseArguments + @("-E", $rootHalves.Parallel)
                        $parallelListArguments = $baseListArguments + @("-E", $rootHalves.Parallel, "--message-format", "json")
                    }
                    # MEASURED, then run. A half of an iteration selection may
                    # legitimately be empty -- every affected package can sit on
                    # one side of the partition -- and running an empty half
                    # costs a compile for nothing. The global invariant below is
                    # what refuses "every half was empty".
                    $parallelCount = Measure-NextestSelection -ListArguments $parallelListArguments `
                        -Context ("{0} iteration parallel-safe tests" -f $workspaceName)
                    if ($parallelCount -gt 0) {
                        Write-Host ("{0}: {1} parallel-safe test(s) selected." -f $workspaceName, $parallelCount)
                        Invoke-Checked -Program "cargo" -Arguments $parallelArguments
                        $script:iterationTestsRun += $parallelCount
                        Save-CargoTimingReport -WorkspaceName $workspaceName -Label "iterate-nextest"
                    }
                    else {
                        Write-Host ("{0}: no parallel-safe test in the affected packages; the whole selection is machine-serial." -f $workspaceName)
                    }

                    if ($null -ne $rootHalves.Serial) {
                        $serialState = [pscustomobject]@{
                            Root          = $workspaceRoot
                            Name          = $workspaceName
                            RunArguments  = $baseArguments + @("-E", $rootHalves.Serial)
                            ListArguments = $baseListArguments + @("-E", $rootHalves.Serial, "--message-format", "json")
                        }
                        Add-SerialPhaseAction -Describe (
                            "$workspaceName iteration machine-serial tests ({0})" -f ($rootHalves.SerialPackages -join ', ')
                        ) -State $serialState -Action {
                            param($State)
                            Push-Location $State.Root
                            try {
                                Invoke-CheckedNextestWithCensus `
                                    -Arguments $State.RunArguments `
                                    -ListArguments $State.ListArguments `
                                    -Context ("{0} iteration machine-serial tests" -f $State.Name)
                                Save-CargoTimingReport -WorkspaceName $State.Name -Label "iterate-nextest-serial"
                            }
                            finally { Pop-Location }
                        }
                        $script:iterationSerialHalvesQueued++
                    }
                }
                "Fast" {
                    $tierFilter = "kind(lib) | kind(bin)"
                    if ($FilterExpression) {
                        $tierFilter = "($tierFilter) & ($FilterExpression)"
                    }
                    $arguments = @("nextest", "run") + $nextestConfigSelection +
                        @("-E", $tierFilter) + $timingSelection + $targetSelection + $selection
                    Invoke-Checked -Program "cargo" -Arguments $arguments
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "fast"
                }
                "Integration" {
                    $tierFilter = "kind(test)"
                    if ($FilterExpression) {
                        $tierFilter = "($tierFilter) & ($FilterExpression)"
                    }
                    $arguments = @("nextest", "run") + $nextestConfigSelection +
                        @("-E", $tierFilter) + $timingSelection + $targetSelection + $selection
                    Invoke-Checked -Program "cargo" -Arguments $arguments
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "integration"
                }
                "Soak" {
                    $arguments = @(
                        "nextest", "run", "--locked", "--no-fail-fast",
                        "--config-file", $nextestConfig, "--profile", "soak",
                        "--package", "replay_app", "--test", "soak", "--test", "soak_f2",
                        "--test-threads", "1"
                    ) + $timingSelection
                    if ($FilterExpression) {
                        $arguments += @("-E", $FilterExpression)
                    }
                    Invoke-Checked -Program "cargo" -Arguments $arguments
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "soak"
                }
                "Evidence" {
                    # Both workspaces now define an `evidence` profile: Replay's
                    # in the config passed with --config-file, Extractor's in
                    # its own .config\nextest.toml, which nextest discovers from
                    # the workspace root. Selecting it for Extractor too is what
                    # gives the ignored, deliberately long tests a bound of
                    # their own instead of the default profile's, which is sized
                    # for the routine gate and would kill them.
                    $evidenceProfileSelection = @("--profile", "evidence")
                    $arguments = @("nextest", "run", "--no-fail-fast", "--run-ignored", "all") +
                        $nextestConfigSelection + $evidenceProfileSelection +
                        $timingSelection + $featureSelection + $targetSelection + $selection
                    if ($FilterExpression) {
                        $arguments += @("-E", $FilterExpression)
                    }
                    Invoke-Checked -Program "cargo" -Arguments $arguments
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "evidence"
                }
                "Final" {
                    # THE SPLIT. The parallel half runs here and now, holding no
                    # machine-wide mutex; the serial half is QUEUED and run
                    # later, under it. Both are derived from one partition, and
                    # the parallel filterset is always the complement of the
                    # serial one, so no test can fall out of both halves. The
                    # run is green only if every half of every root passed:
                    # Invoke-Checked throws on any non-zero exit and the serial
                    # queue is drained before this script can return 0.
                    #
                    # The build is the point. `nextest run` compiles the whole
                    # `--all-targets --workspace` selection before it runs
                    # anything, filterset or no filterset, so with the split the
                    # compile -- the majority of a battery's wall clock, and the
                    # phase that never had a reason to be serialized -- happens
                    # outside the mutex.
                    $rootHalves = $null
                    if ($splitBattery) {
                        $rootPackages = Get-CargoRootPackages -CargoRoot $workspaceRoot
                        Assert-BatteryPartitionCoversPackages `
                            -ObservedPackages $rootPackages `
                            -SerialPackages $batteryPartition.Serial `
                            -ParallelPackages $batteryPartition.Parallel
                        $rootHalves = Get-BatteryRootHalves `
                            -Partition $batteryPartition -RootPackages $rootPackages
                    }
                    $arguments = @("nextest", "run", "--all-targets", "--no-fail-fast") +
                        $nextestConfigSelection + $nextestCiProfileSelection +
                        $timingSelection + $selection
                    if ($FilterExpression) {
                        $arguments += @("-E", $FilterExpression)
                    }
                    elseif ($null -ne $rootHalves -and $rootHalves.Parallel) {
                        $arguments += @("-E", $rootHalves.Parallel)
                    }
                    Invoke-Checked -Program "cargo" -Arguments $arguments
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "final-nextest"
                    if ($null -ne $rootHalves -and $rootHalves.Serial) {
                        # Captured by value into -State: $workspaceRoot and the
                        # argument list are loop variables, and a script block
                        # that read them at drain time would run every queued
                        # action against the LAST workspace.
                        $serialState = [pscustomobject]@{
                            Root          = $workspaceRoot
                            Name          = $workspaceName
                            RunArguments  = @("nextest", "run", "--all-targets", "--no-fail-fast") +
                                $nextestConfigSelection + $nextestCiProfileSelection +
                                $timingSelection + $selection + @("-E", $rootHalves.Serial)
                            ListArguments = @("nextest", "list", "--all-targets") +
                                $nextestConfigSelection + $nextestCiProfileSelection +
                                $selection + @("-E", $rootHalves.Serial, "--message-format", "json")
                        }
                        Add-SerialPhaseAction -Describe (
                            "$workspaceName machine-serial tests ({0})" -f ($rootHalves.SerialPackages -join ', ')
                        ) -State $serialState -Action {
                            param($State)
                            Push-Location $State.Root
                            try {
                                Invoke-CheckedNextestWithCensus `
                                    -Arguments $State.RunArguments `
                                    -ListArguments $State.ListArguments `
                                    -Context ("{0} machine-serial tests" -f $State.Name)
                                Save-CargoTimingReport -WorkspaceName $State.Name -Label "final-nextest-serial"
                            }
                            finally { Pop-Location }
                        }
                    }
                    # DOCTESTS ARE NOT SPLIT, and the reason is fidelity rather
                    # than caution.
                    #
                    # The nextest halves above are safe to split because the
                    # CARGO selection is identical in both -- `--workspace
                    # --all-targets` -- and only which of the built tests get
                    # EXECUTED differs. Cargo's feature unification is computed
                    # over the selected package set, so both halves compile
                    # byte-for-byte what the un-split battery compiled.
                    #
                    # Doctests have no such filter. Splitting them means splitting
                    # the cargo selection (`--workspace --exclude X` for one half,
                    # `-p X` for the other), and each of those unifies features
                    # over a DIFFERENT package set than `--workspace` does. The
                    # halves would then be compiling their doctests against
                    # differently-featured dependencies from the ones the battery
                    # is supposed to certify -- a seal that no longer means what it
                    # meant. The first attempt at this also failed outright:
                    # `-p wire_format` re-unified compact_contract and rustdoc was
                    # handed an extern rlib path that the excluded half had never
                    # produced.
                    #
                    # So the whole doctest run is queued into the serial half, once,
                    # with exactly today's arguments. That is also what the
                    # fail-closed rule requires of anything the partition does not
                    # classify. Measured warm on this tree it costs 9.9 s of mutex
                    # time for the Extractor workspace.
                    $doctestArguments = @("test", "--doc", "--no-fail-fast") + $timingSelection + $selection
                    if ($splitBattery) {
                        $doctestState = [pscustomobject]@{
                            Root      = $workspaceRoot
                            Name      = $workspaceName
                            Arguments = $doctestArguments
                        }
                        Add-SerialPhaseAction -Describe "$workspaceName doctests (unsplit: cargo feature unification)" `
                            -State $doctestState -Action {
                            param($State)
                            Push-Location $State.Root
                            try {
                                Invoke-Checked -Program "cargo" -Arguments $State.Arguments
                                Save-CargoTimingReport -WorkspaceName $State.Name -Label "final-doctest"
                            }
                            finally { Pop-Location }
                        }
                    }
                    else {
                        Invoke-Checked -Program "cargo" -Arguments $doctestArguments
                        Save-CargoTimingReport -WorkspaceName $workspaceName -Label "final-doctest"
                    }
                    # Everything above ran in the DEV profile, and a detour
                    # identity that only breaks under the release linker's
                    # identical-code folding is invisible to all of it. Before
                    # 2026-08-05 the only tier that compiled a lane crate
                    # optimized was the full Release battery, and D95's four
                    # detour bodies really did fold onto two addresses in the
                    # optimized build of its own test binary while every debug
                    # tier reported green.
                    if ($workspaceName -ceq 'Extractor') {
                        if ($Package.Count -eq 0 -and
                            [string]::IsNullOrWhiteSpace($FilterExpression)) {
                            # No Save-CargoTimingReport here on purpose: this
                            # step does not pass --timings, so the only report
                            # on disk would be the previous invocation's and
                            # saving it under this label would attribute one
                            # run's cost to another.
                            #
                            # This gate is the battery's second-largest compile
                            # -- the lane crates again, optimized -- so which
                            # half it lands in matters. It is classified by the
                            # same partition as everything else, and by the
                            # fail-closed rule: if ANY package it names is
                            # machine-serial, the whole gate is.
                            $detourSerial = $false
                            if ($splitBattery) {
                                $detourPackages = Get-DetourIdentityLanePackages
                                $detourSerial = @($detourPackages |
                                        Where-Object { $batteryPartition.Serial -contains $_ }).Count -gt 0
                            }
                            if ($detourSerial) {
                                Add-SerialPhaseAction -Describe 'release detour-identity gate' `
                                    -State ([pscustomobject]@{ Root = $workspaceRoot }) -Action {
                                    param($State)
                                    Push-Location $State.Root
                                    try { Invoke-ReleaseDetourIdentityGate }
                                    finally { Pop-Location }
                                }
                            }
                            else {
                                Invoke-ReleaseDetourIdentityGate
                            }
                        }
                        else {
                            Write-Host (
                                'SKIPPED BY POLICY: the release detour-identity ' +
                                'gate did NOT run, because this Final was ' +
                                'narrowed by -Package or -FilterExpression. It ' +
                                'owns its own package set and narrowing it would ' +
                                'report a pass over crates it never compiled. ' +
                                'Use Invoke-Validation.ps1 -Mode ReleaseManual ' +
                                '-AllowReleaseManual only when the owner requests it.'
                            )
                        }
                    }
                }
                "Release" {
                    Invoke-Checked -Program "cargo" -Arguments (@("build", "--release", "--all-targets") + $timingSelection + $selection)
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "release-build"
                    $arguments = @(
                        "nextest", "run", "--all-targets", "--no-fail-fast",
                        "--cargo-profile", "release"
                    ) + $nextestConfigSelection + $nextestCiProfileSelection +
                        $timingSelection + $selection
                    if ($FilterExpression) {
                        $arguments += @("-E", $FilterExpression)
                    }
                    Invoke-Checked -Program "cargo" -Arguments $arguments
                    Save-CargoTimingReport -WorkspaceName $workspaceName -Label "release-test"
                    if ($workspaceName -ceq 'Extractor') {
                        # Rebuild both shipped injection components after every
                        # all-target Release build/test. The canonical recorder
                        # has an explicit empty feature set; the collector cdylib
                        # is rebuilt last with both default and unshipped
                        # evidence features disabled. Target-cache ordering can
                        # therefore never select an all-target/evidence variant
                        # as a package input.
                        Invoke-Checked -Program "cargo" -Arguments @(
                            "build", "--locked", "--release",
                            "--package", "recorder_host",
                            "--bin", "collector_record",
                            "--no-default-features"
                        )
                        Invoke-Checked -Program "cargo" -Arguments @(
                            "build", "--locked", "--release",
                            "--package", "collector",
                            "--lib",
                            "--no-default-features"
                        )
                    }
                }
            }
        }
        finally {
            Pop-Location
        }
    }

    # THE STANDALONE ROOTS, DERIVED FROM THE ONE INVENTORY RATHER THAN RESTATED.
    # These were two hand-kept lists. Lane LD found what a hand-kept list costs:
    # `portable_replay\native\terrain_fixture` was missing from the portable list
    # here, from the target-dir guard's list above, and from the inventory in
    # BatteryPartition.ps1 -- three copies that all had to be edited and only
    # ever two were, so one crate's tests had never run anywhere since it was
    # added on 2026-08-05. Everything below is now computed from
    # Get-BatteryCargoRootPaths, and a new standalone crate is registered by
    # adding it in exactly ONE place.
    $allCargoRoots = @(Get-BatteryCargoRootPaths -RepoRoot $repoRoot)
    $standaloneCargoRoots = @($allCargoRoots | Where-Object {
            $candidate = [IO.Path]::GetFullPath($_)
            @($workspaceRootsAll | Where-Object { [IO.Path]::GetFullPath($_) -ieq $candidate }).Count -eq 0
        })

    # The extractor-owned standalone roots belong to one workspace's TREE but
    # are deliberately not members of it, so they follow that workspace's
    # -Workspace selection.
    $extractorStandaloneRoots = @($standaloneCargoRoots | Where-Object {
            Test-PathIsInside -Root (Join-Path $repoRoot "extractor\native") -Path $_
        })

    # The neutral portable_replay crates CANNOT be members of either workspace:
    # both trees path-depend on them, and a cargo package belongs to at most one
    # workspace. Each is therefore a cargo root in its own right, with its own
    # committed Cargo.lock, and is built (never tested) as a path dependency of
    # the two workspaces above.
    #
    # That is a structural fact, not a decision to leave them unverified.
    # Until they were listed nothing ran their `#[cfg(test)]` modules at all, so
    # `--workspace` reported green while ~190 tests in crates the two halves
    # link had never executed. They are validated for every -Workspace selection
    # because at least one half depends on each of them -- MOST are linked by
    # both halves, but `portable_terrain_fixture` is a replay-side
    # dev-dependency only, so "both halves link every one of these" would be an
    # overclaim. What is true of all of them is that no `--workspace` reaches
    # them.
    $portableStandaloneRoots = @($standaloneCargoRoots | Where-Object {
            Test-PathIsInside -Root (Join-Path $repoRoot "portable_replay\native") -Path $_
        })
    if ($portableStandaloneRoots.Count -eq 0) {
        throw (
            'the battery root inventory names no portable_replay cargo root. Those crates are ' +
            'linked into the workspaces and are members of neither, so nothing else runs their ' +
            'tests; proceeding would lock and validate the repository with that coverage ' +
            'silently absent.'
        )
    }
    # EVERY standalone root must fall into one of the two buckets above, or it
    # is a root this block silently never enters -- the same shape of hole the
    # missing crate was. A root under neither tree is a refusal, not a skip.
    $unclassifiedStandaloneRoots = @($standaloneCargoRoots | Where-Object {
            $extractorStandaloneRoots -notcontains $_ -and $portableStandaloneRoots -notcontains $_
        })
    if ($unclassifiedStandaloneRoots.Count -gt 0) {
        throw (
            ('{0} registered cargo root(s) sit under neither extractor\native nor ' +
             'portable_replay\native and would be locked and validated by nothing: {1}. Add the ' +
             'tree to this classification rather than leaving the root unentered.') -f `
                $unclassifiedStandaloneRoots.Count, ($unclassifiedStandaloneRoots -join ', ')
        )
    }

    if ($Mode -eq "Lock") {
        $standaloneRoots = @()
        if ($Workspace -in @("All", "Extractor")) {
            $standaloneRoots += $extractorStandaloneRoots
        }
        $standaloneRoots += $portableStandaloneRoots
        foreach ($standaloneRoot in $standaloneRoots) {
            Push-Location $standaloneRoot
            try {
                Invoke-Checked -Program "cargo" -Arguments @("generate-lockfile")
            }
            finally {
                Pop-Location
            }
        }
    }

    # THE STANDALONE ROOTS, FOR ITERATE. Nine of this repository's eleven cargo
    # roots are not members of either workspace, so the loop above never enters
    # them. An affected package living in one of them would otherwise be
    # selected, counted, and never run - which is the exact shape of "reported
    # green having tested nothing" this file exists to refuse. The root
    # inventory comes from BatteryPartition.ps1 rather than a fourth hand-kept
    # list in this file.
    if ($Mode -eq 'Iterate') {
        foreach ($standaloneRoot in (Get-BatteryCargoRootPaths -RepoRoot $repoRoot)) {
            # The two workspaces were driven above.
            if (@($workspaceRoots | Where-Object {
                        [IO.Path]::GetFullPath($_) -ieq [IO.Path]::GetFullPath($standaloneRoot)
                    }).Count -gt 0) { continue }
            if ([IO.Path]::GetFullPath($standaloneRoot) -ieq [IO.Path]::GetFullPath((Join-Path $repoRoot 'extractor\native')) -or
                [IO.Path]::GetFullPath($standaloneRoot) -ieq [IO.Path]::GetFullPath((Join-Path $repoRoot 'replay_interface\native'))) {
                # A workspace this run's -Workspace excluded. It stays excluded.
                continue
            }
            if (-not (Test-Path -LiteralPath (Join-Path $standaloneRoot 'Cargo.toml') -PathType Leaf)) { continue }
            $standalonePackages = @(Get-CargoRootPackages -CargoRoot $standaloneRoot)
            $scopedStandalone = @($standalonePackages | Where-Object { $selectedPackageNames -contains $_ })
            if ($scopedStandalone.Count -eq 0) { continue }
            foreach ($name in $scopedStandalone) { [void]$selectedPackagesCovered.Add([string]$name) }
            $cargoRootsEntered++
            # Classified WHOLE, like Final does: these roots hold one or two
            # crates each, and "any serial package makes the root serial" is the
            # fail-closed reading.
            $standaloneSerial = @($scopedStandalone | Where-Object { $batteryPartition.Serial -contains $_ }).Count -gt 0
            # -Features REACHES THESE ROOTS TOO. Omitting it here was a
            # false-green path: a caller passing -Features because the diff is
            # inside an off-by-default module would get default features
            # compiled in a standalone root, the root's ordinary tests would
            # supply a positive census, and the run would report having covered
            # code it never built. If the feature names a package this root
            # cannot see, cargo says so and the run fails -- which is the right
            # direction for a selector the caller asked for by name.
            $standaloneRunArguments = @("nextest", "run", "--locked", "--all-targets", "--no-fail-fast") +
                $requestedFeatureSelection
            $standaloneListArguments = @("nextest", "list", "--locked", "--all-targets") +
                $requestedFeatureSelection + @("--message-format", "json")
            $standaloneState = [pscustomobject]@{
                Root          = $standaloneRoot
                Name          = (Split-Path -Leaf $standaloneRoot)
                RunArguments  = $standaloneRunArguments
                ListArguments = $standaloneListArguments
            }
            if ($standaloneSerial) {
                Add-SerialPhaseAction -Describe (
                    "iteration standalone root {0} ({1})" -f (Split-Path -Leaf $standaloneRoot), ($scopedStandalone -join ', ')
                ) -State $standaloneState -Action {
                    param($State)
                    Push-Location $State.Root
                    try {
                        Invoke-CheckedNextestWithCensus `
                            -Arguments $State.RunArguments `
                            -ListArguments $State.ListArguments `
                            -Context ("iteration standalone root {0}" -f $State.Name)
                    }
                    finally { Pop-Location }
                }
                $script:iterationSerialHalvesQueued++
            }
            else {
                Push-Location $standaloneRoot
                try {
                    $standaloneCount = Measure-NextestSelection `
                        -ListArguments $standaloneListArguments `
                        -Context ("iteration standalone root {0}" -f (Split-Path -Leaf $standaloneRoot))
                    if ($standaloneCount -gt 0) {
                        Invoke-Checked -Program "cargo" -Arguments $standaloneRunArguments
                        $script:iterationTestsRun += $standaloneCount
                    }
                }
                finally { Pop-Location }
            }
        }
    }

    # ================= THE GLOBAL NON-EMPTY-EXECUTION INVARIANT ================
    #
    # Per-root skipping is correct and per-half skipping is correct, but a run in
    # which EVERY root and EVERY half was skipped has compiled nothing, run
    # nothing, and would return 0 to a lane that asked it for a verdict. Asserted
    # once, here, against counters the loops above incremented.
    if ($selectedPackageNames.Count -gt 0) {
        if ($cargoRootsEntered -eq 0) {
            throw (
                'this run selected ' + $selectedPackageNames.Count + ' package(s) and then ' +
                'entered NO cargo root at all. Nothing was compiled and nothing was run, ' +
                'so there is no verdict to report. Check -Workspace against the packages ' +
                'named.'
            )
        }
        $uncovered = @($selectedPackageNames | Where-Object { -not $selectedPackagesCovered.Contains([string]$_) } | Sort-Object)
        if ($uncovered.Count -gt 0) {
            throw (
                'this run selected ' + $uncovered.Count + ' package(s) that no cargo root it ' +
                'entered contains: ' + ($uncovered -join ', ') + '. They were selected and ' +
                'never compiled, which is a narrower run than the one reported. Widen ' +
                '-Workspace, or add the missing cargo root to $script:BatteryCargoRoots in ' +
                'BatteryPartition.ps1.'
            )
        }
    }
    if ($Mode -eq 'Iterate' -and $script:iterationTestsRun -eq 0 -and $script:iterationSerialHalvesQueued -eq 0) {
        throw (
            'the iteration run selected packages but matched ZERO tests in every half of ' +
            'every root. nextest exits 0 for a selection that matches nothing, so this ' +
            'would have been a green run that executed no test at all.'
        )
    }

    if ($Package.Count -eq 0 -and $Mode -in @("Final", "Release")) {
        $standaloneRoots = @()
        if ($Workspace -in @("All", "Extractor")) {
            $standaloneRoots += $extractorStandaloneRoots
        }
        $standaloneRoots += $portableStandaloneRoots
        foreach ($standaloneRoot in $standaloneRoots) {
            $standaloneManifest = Join-Path $standaloneRoot "Cargo.toml"
            # A standalone root is classified WHOLE rather than per package.
            # These roots hold one or two crates each, so a filterset inside one
            # would buy nothing, and "any serial package makes the root serial"
            # is the fail-closed reading. A root the partition says nothing
            # serial about runs in the parallel half exactly as it runs today.
            if ($splitBattery -and $Mode -eq "Final") {
                $standalonePackages = Get-CargoRootPackages -CargoRoot $standaloneRoot
                Assert-BatteryPartitionCoversPackages `
                    -ObservedPackages $standalonePackages `
                    -SerialPackages $batteryPartition.Serial `
                    -ParallelPackages $batteryPartition.Parallel
                $standaloneSerialPackages = @($standalonePackages |
                        Where-Object { $batteryPartition.Serial -contains $_ })
                if ($standaloneSerialPackages.Count -gt 0) {
                    $standaloneState = [pscustomobject]@{
                        Root     = $standaloneRoot
                        Manifest = $standaloneManifest
                    }
                    Add-SerialPhaseAction -Describe (
                        "standalone root {0} ({1})" -f `
                            (Split-Path -Leaf $standaloneRoot), ($standaloneSerialPackages -join ', ')
                    ) -State $standaloneState -Action {
                        param($State)
                        Push-Location $State.Root
                        try {
                            Invoke-CheckedNextestWithCensus `
                                -Arguments @("nextest", "run", "--locked", "--all-targets", "--no-fail-fast") `
                                -ListArguments @("nextest", "list", "--locked", "--all-targets", "--message-format", "json") `
                                -Context ("standalone root {0}" -f (Split-Path -Leaf $State.Root))
                            if (Test-CargoManifestHasLibraryTarget -ManifestPath $State.Manifest) {
                                Invoke-Checked -Program "cargo" -Arguments @("test", "--locked", "--doc", "--no-fail-fast")
                            }
                            else {
                                Write-Host ("Skipping standalone doctests: no library target in {0}" -f $State.Manifest)
                            }
                        }
                        finally { Pop-Location }
                    }
                    continue
                }
            }
            Push-Location $standaloneRoot
            try {
                if ($Mode -eq "Final") {
                    Invoke-Checked -Program "cargo" -Arguments @("nextest", "run", "--locked", "--all-targets", "--no-fail-fast")
                    if (Test-CargoManifestHasLibraryTarget -ManifestPath $standaloneManifest) {
                        Invoke-Checked -Program "cargo" -Arguments @("test", "--locked", "--doc", "--no-fail-fast")
                    }
                    else {
                        Write-Host "Skipping standalone doctests: no library target in $standaloneManifest"
                    }
                }
                else {
                    Invoke-Checked -Program "cargo" -Arguments @("build", "--locked", "--release", "--all-targets")
                    Invoke-Checked -Program "cargo" -Arguments @("nextest", "run", "--locked", "--all-targets", "--no-fail-fast", "--cargo-profile", "release")
                }
            }
            finally {
                Pop-Location
            }
        }
    }

    # THE SERIAL HALF. Everything queued above runs here, and only here, holding
    # the same machine-wide mutex the whole run used to hold. The parallel slot
    # is given back FIRST: a battery sitting in its serial half has no claim on
    # the machine's cores, and holding a slot through it would block another
    # lane's compile for no reason. That ordering is also what makes deadlock
    # impossible -- no run ever holds a slot and waits for the mutex.
    #
    # A queue that came out EMPTY is a failure, not a fast run: it would mean the
    # partition classified nothing as machine-serial for a battery that compiled
    # recorder_host, and the run would be claiming a seal it never earned.
    if ($splitBattery) {
        if ($parallelSlotIndex -ge 0) {
            $parallelSlotMutexes[$parallelSlotIndex].ReleaseMutex()
            $parallelSlotIndex = -1
        }
        # ITERATE IS THE ONE MODE ALLOWED AN EMPTY SERIAL QUEUE, and the reason
        # is not leniency. A Final compiles the whole workspace, so a partition
        # that named nothing machine-serial has certainly gone wrong. An
        # iteration selection is a handful of packages that the partition may
        # legitimately classify entirely parallel-safe -- and when it does, the
        # invariant that matters has already been asserted above: this run
        # matched a nonzero number of tests SOMEWHERE. An empty queue here with
        # zero tests run cannot reach this line.
        if ($serialPhaseActions.Count -eq 0) {
            if ($Mode -ne 'Iterate') {
                throw (
                    'the battery split queued NO machine-serial work. That is not a ' +
                    'pass: this run compiled the packages the partition calls ' +
                    'machine-serial and then ran none of them under the mutex, so ' +
                    'its verdict covers only the parallel half.'
                )
            }
            Write-Host (
                'Iteration run: every affected package is parallel-safe, so nothing needed the machine-wide mutex.'
            )
        }
        if ($serialPhaseActions.Count -gt 0) {
        Write-Host (
            'Battery serial half: {0} queued step(s); waiting for the central validation mutex.' -f `
                $serialPhaseActions.Count
        )
        $ownsMutex = Enter-CentralValidationSerialPhase -Mutex $mutex
        # The number this whole change is about: how long ONE battery denies the
        # machine to every other lane. Before the split it was the run's entire
        # wall clock, compile included. Printing it makes a regression -- a
        # package drifting into the serial half, or the split silently not
        # applying -- visible in the gate log instead of only in the queue.
        $serialPhaseStart = Get-Date
        foreach ($queued in $serialPhaseActions) {
            Write-Host ("Battery serial half: {0}" -f $queued.Describe)
            & $queued.Action $queued.State
        }
        $serialPhaseSeconds = ((Get-Date) - $serialPhaseStart).TotalSeconds
        if ($Mode -eq 'Iterate') {
            # NOT A SEAL, and it does not say "seal". An iteration run has
            # certified the tests its diff was computed to reach and nothing
            # else, and the word this file uses for a complete verdict is not
            # available to it.
            Write-Host (
                ('Iteration run complete: {0} machine-serial step(s); central validation mutex held for {1:N1} s. ' +
                 'THIS IS NOT A LANDING GATE - run Invoke-Validation.ps1 -Mode Landing before landing.') -f `
                    $serialPhaseActions.Count, $serialPhaseSeconds
            )
        }
        else {
            $sealTemplate =
                'Battery seal: BOTH halves passed ({0} machine-serial step(s) after the ' +
                'parallel half); central validation mutex held for {1:N1} s.'
            Write-Host ($sealTemplate -f $serialPhaseActions.Count, $serialPhaseSeconds)
        }
        }
    }

    if ($fullExtractorRelease) {
        $releaseBuildEnvironmentAfter = Get-ProductionBuildEnvironment
        if ([string]$releaseBuildEnvironmentBefore.sha256 -cne [string]$releaseBuildEnvironmentAfter.sha256) {
            throw "production build environment changed during centralized Release; no artifact attestation was emitted"
        }
        $releaseInputDigestAfter = Get-ProductionRustInputDigest -RepoRoot $repoRoot
        if ([string]$releaseInputDigestBefore.sha256 -cne [string]$releaseInputDigestAfter.sha256 -or
            [int]$releaseInputDigestBefore.file_count -ne [int]$releaseInputDigestAfter.file_count) {
            throw "production Rust inputs changed during centralized Release; no artifact attestation was emitted"
        }
        try {
            Write-ProductionArtifactAttestation `
                -RepoRoot $repoRoot `
                -ReceiptPath $artifactAttestationPath `
                -RequestedWorkspace $Workspace `
                -SourceDigest $releaseInputDigestAfter `
                -ReleaseRuntimeLinkage $releaseRuntimeLinkage `
                -BuildEnvironment $releaseBuildEnvironmentAfter `
                -ArtifactFreshness $releaseArtifactFreshness

            $captureDriver = Join-Path $repoRoot "extractor\native\scripts\Start-Hoi4ProductionCaptureSafe.ps1"
            $attested = Test-ProductionArtifactAttestation `
                -RepoRoot $repoRoot `
                -ReceiptPath $artifactAttestationPath `
                -ExpectedDriverPath $captureDriver `
                -ExpectedBuildEnvironment $releaseBuildEnvironmentAfter `
                -CurrentSourceDigest $releaseInputDigestAfter

            if ([string]::IsNullOrWhiteSpace($ProductionCaptureReceiptPath)) {
                Write-Host "Candidate production artifacts: $artifactAttestationPath"
                Write-Host "No portable package was emitted: rerun this exact full Release with -ProductionCaptureReceiptPath after a production capture passes."
                return
            }

            $productionCaptureEvidence = Get-CurrentProductionCaptureEvidence

            $packageBuilderHash = (Get-FileHash -LiteralPath $portablePackageScript -Algorithm SHA256).Hash.ToLowerInvariant()
            $packageStdout = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass `
                -File $portablePackageScript `
                -RecorderExe ([string]$attested.artifacts['recorder_host']) `
                -ReplayExe ([string]$attested.artifacts['replay_app']) `
                -CollectorDll ([string]$attested.artifacts['collector_dll']) `
                -OutputZip $portablePackagePath `
                -Version $recorderVersion `
                -ProductionCaptureReceiptPath $ProductionCaptureReceiptPath `
                -ProductionArtifactAttestationPath $artifactAttestationPath)
            if ($LASTEXITCODE -ne 0) {
                throw "portable product package builder exited with code $LASTEXITCODE"
            }
            $packageResult = (($packageStdout -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
            $currentInteractionPolicyInputs = Get-CurrentInteractionPolicyInputs
            $currentPinnedHookAuthority = Get-CurrentPinnedHookSourceAuthority
            $actualPackagePath = [System.IO.Path]::GetFullPath([string]$packageResult.path)
            $expectedPackagePath = [System.IO.Path]::GetFullPath($portablePackagePath)
            if ([string]$packageResult.schema -cne 'hoi4-portable-package.v1' -or
                [string]$packageResult.version -cne $recorderVersion -or
                $actualPackagePath -ine $expectedPackagePath -or
                [string]$packageResult.sha256 -cne (Get-FileHash -LiteralPath $expectedPackagePath -Algorithm SHA256).Hash.ToLowerInvariant() -or
                [string]$packageResult.input_sha256.recorder_host -cne [string]$attested.artifact_sha256['recorder_host'] -or
                [string]$packageResult.input_sha256.replay_app -cne [string]$attested.artifact_sha256['replay_app'] -or
                [string]$packageResult.input_sha256.collector_dll -cne [string]$attested.artifact_sha256['collector_dll'] -or
                $packageResult.admissible_for_production -isnot [bool] -or
                -not $packageResult.admissible_for_production -or
                (($packageResult.production_capture_evidence | ConvertTo-Json -Depth 12 -Compress) -cne
                    ($productionCaptureEvidence | ConvertTo-Json -Depth 12 -Compress)) -or
                [string]$packageResult.interaction_closure.policy -cne 'passive_injected_observer' -or
                $packageResult.interaction_closure.zip_entry_bytes_reverified -isnot [bool] -or
                -not $packageResult.interaction_closure.zip_entry_bytes_reverified -or
                ($packageResult.interaction_closure.policy_inputs | ConvertTo-Json -Compress) -cne
                    ($currentInteractionPolicyInputs | ConvertTo-Json -Compress) -or
                [string]$packageResult.interaction_closure.source_capability.schema -cne
                    'hoi4.production-source-capability-closure.v1' -or
                -not $packageResult.interaction_closure.source_capability.canonical_observer_feature_isolated -or
                -not $packageResult.interaction_closure.source_capability.target_lifecycle_actuators_evidence_only -or
                -not $packageResult.interaction_closure.source_capability.production_auto_attach_target_control_absent -or
                -not $packageResult.interaction_closure.source_capability.collector_fixture_hooks_default_off -or
                -not $packageResult.interaction_closure.source_capability.collector_cdylib_explicit_empty_features -or
                -not $packageResult.interaction_closure.source_capability.structural_dependency_feature_parse -or
                -not $packageResult.interaction_closure.source_capability.production_source_tree_closed -or
                -not $packageResult.interaction_closure.source_capability.canonical_hook_name_families_closed -or
                [int]$packageResult.interaction_closure.source_capability.exact_pinned_hook_count -ne
                    [int]$currentPinnedHookAuthority.count -or
                [string]$packageResult.interaction_closure.source_capability.pinned_hook_contract_source_sha256 -cne
                    [string]$currentPinnedHookAuthority.source_sha256 -or
                [string]$packageResult.interaction_closure.source_capability.exact_target_executable_sha256 -cne
                    '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad' -or
                [string]$packageResult.interaction_closure.recorder_host.schema -cne 'hoi4.interaction-closure.v1' -or
                [string]$packageResult.interaction_closure.replay_app.schema -cne 'hoi4.interaction-closure.v1' -or
                [string]$packageResult.interaction_closure.collector_dll.schema -cne 'hoi4.interaction-closure.v1' -or
                [int]$packageResult.interaction_closure.recorder_host.exact_pinned_hook_count -ne
                    [int]$currentPinnedHookAuthority.count -or
                [int]$packageResult.interaction_closure.collector_dll.exact_pinned_hook_count -ne
                    [int]$currentPinnedHookAuthority.count -or
                [int]$packageResult.interaction_closure.replay_app.exact_pinned_hook_count -ne
                    [int]$currentPinnedHookAuthority.count -or
                [string]$packageResult.interaction_closure.recorder_host.exact_pinned_hook_contract -cne
                    'a729.passive_ready.pinned_production_hook_specs.v1' -or
                [string]$packageResult.interaction_closure.collector_dll.exact_pinned_hook_contract -cne
                    'a729.passive_ready.pinned_production_hook_specs.v1' -or
                [string]$packageResult.interaction_closure.recorder_host.pinned_hook_contract_source_sha256 -cne
                    [string]$currentPinnedHookAuthority.source_sha256 -or
                [string]$packageResult.interaction_closure.collector_dll.pinned_hook_contract_source_sha256 -cne
                    [string]$currentPinnedHookAuthority.source_sha256 -or
                (@($packageResult.interaction_closure.recorder_host.permitted_injection_capabilities) -join '|') -cne
                    'OpenProcess|VirtualAllocEx|VirtualFreeEx|WriteProcessMemory|CreateRemoteThread|LoadLibraryW|FreeLibrary' -or
                (@($packageResult.interaction_closure.recorder_host.permitted_self_ui_capabilities) -join '|') -cne
                    'SendMessageW|ShellExecuteW|EnableWindow|SetWindowTextW|ShowWindow|DestroyWindow' -or
                (@($packageResult.interaction_closure.replay_app.permitted_self_ui_capabilities) -join '|') -cne
                    'SetWindowTextW|DestroyWindow' -or
                @($packageResult.interaction_closure.collector_dll.permitted_self_ui_capabilities).Count -ne 0 -or
                (@($packageResult.interaction_closure.replay_app.permitted_self_worker_capabilities) -join '|') -cne
                    'CreateProcessW|TerminateProcess|CreateJobObjectW|AssignProcessToJobObject|SetInformationJobObject' -or
                @($packageResult.interaction_closure.recorder_host.permitted_self_worker_capabilities).Count -ne 0 -or
                @($packageResult.interaction_closure.collector_dll.permitted_self_worker_capabilities).Count -ne 0 -or
                @($packageResult.interaction_closure.replay_app.permitted_injection_capabilities).Count -ne 0 -or
                @($packageResult.interaction_closure.collector_dll.permitted_injection_capabilities).Count -ne 0 -or
                -not $packageResult.interaction_closure.recorder_host.forbidden_symbols_absent -or
                -not $packageResult.interaction_closure.replay_app.forbidden_symbols_absent -or
                -not $packageResult.interaction_closure.collector_dll.forbidden_symbols_absent -or
                $packageResult.interaction_closure.recorder_host.semantic_engine_state_writes -or
                $packageResult.interaction_closure.replay_app.semantic_engine_state_writes -or
                $packageResult.interaction_closure.collector_dll.semantic_engine_state_writes -or
                # Target-process lifecycle control and game-UI control are not
                # separate receipt verdicts to read here. Both rosters are
                # subsets of the interaction-marker roster the package enforcers
                # throw on, and a permitted role capability is filtered out of
                # both, so a build that produced a receipt at all has already
                # rejected every symbol either verdict could report.
                $packageResult.static_vcruntime_admission -isnot [bool] -or
                -not $packageResult.static_vcruntime_admission -or
                [int]$packageResult.executable_count -ne 2 -or
                $packageResult.signed -isnot [bool] -or $packageResult.signed -or
                [string]$packageResult.install_kind -cne 'portable_zip') {
                throw "portable product package result failed the centralized Release contract"
            }
            foreach ($role in @('recorder_host', 'replay_app', 'collector_dll')) {
                [void](Assert-PortableProductPeImportAttestation `
                    -Bytes ([System.IO.File]::ReadAllBytes([string]$attested.artifacts[$role])) `
                    -Name $role `
                    -Attestation $packageResult.pe_imports.$role)
                [void](Assert-PortableProductPeImportAttestation `
                    -Bytes ([System.IO.File]::ReadAllBytes([string]$attested.artifacts[$role])) `
                    -Name $role `
                    -Attestation $attested.artifact_pe_imports[$role])
            }
            $releaseInputDigestPackaged = Get-ProductionRustInputDigest -RepoRoot $repoRoot
            if ([string]$releaseInputDigestPackaged.sha256 -cne [string]$releaseInputDigestAfter.sha256 -or
                [int]$releaseInputDigestPackaged.file_count -ne [int]$releaseInputDigestAfter.file_count -or
                $packageBuilderHash -cne (Get-FileHash -LiteralPath $portablePackageScript -Algorithm SHA256).Hash.ToLowerInvariant()) {
                throw "production inputs changed during portable packaging"
            }

            $packageReceipt = [ordered]@{
                schema = 'battle-plan.central_portable_package.v1'
                status = 'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE'
                successful_exit = $true
                central_validation = [ordered]@{
                    mode = 'Release'
                    requested_workspace = 'All'
                    release_runtime_linkage = $releaseRuntimeLinkage
                    production_feature_closure = [ordered]@{
                        schema = 'battle-plan.production_feature_closure.v1'
                        rebuild_order = @('recorder_host', 'collector_dll')
                        recorder_host = [ordered]@{
                            package = 'recorder_host'
                            target = 'collector_record'
                            default_features = $false
                            enabled_features = @()
                            artifact_sha256 = [string]$attested.artifact_sha256['recorder_host']
                        }
                        collector_dll = [ordered]@{
                            package = 'collector'
                            target = 'lib'
                            crate_types = @('cdylib', 'rlib')
                            default_features = $false
                            enabled_features = @()
                            all_non_default_features_disabled = $true
                            # The three `unshipped-*` names that stood here are
                            # not forbidden any more, they no longer exist:
                            # the features and the fixture hooks they gated are
                            # deleted, and the package builder refuses the whole
                            # `unshipped-` prefix in the collector manifest.
                            # `test-support` remains a real, declarable feature
                            # that must not enter a production artifact.
                            forbidden_features = @(
                                'test-support'
                            )
                            built_last = $true
                            collector_cargo_sha256 =
                                [string]$packageResult.interaction_closure.policy_inputs.collector_cargo_sha256
                            pinned_hook_count = [int]$currentPinnedHookAuthority.count
                            pinned_hook_contract_source_sha256 =
                                [string]$currentPinnedHookAuthority.source_sha256
                            artifact_sha256 = [string]$attested.artifact_sha256['collector_dll']
                        }
                    }
                }
                artifact_attestation = [ordered]@{
                    path = [string]$attested.receipt_path
                    sha256 = [string]$attested.receipt_sha256
                }
                package_builder = [ordered]@{
                    path = [System.IO.Path]::GetFullPath($portablePackageScript)
                    sha256 = $packageBuilderHash
                }
                package = $packageResult
                source_input_sha256 = [string]$attested.source_input_sha256
                build_environment = $attested.build_environment
            }
            $packageReceiptCandidate = "$portablePackageReceiptPath.$([guid]::NewGuid().ToString('N')).candidate"
            try {
                [System.IO.File]::WriteAllText(
                    $packageReceiptCandidate,
                    ($packageReceipt | ConvertTo-Json -Depth 12),
                    [System.Text.UTF8Encoding]::new($false)
                )
                $packageReceiptStream = [System.IO.File]::Open(
                    $packageReceiptCandidate,
                    [System.IO.FileMode]::Open,
                    [System.IO.FileAccess]::ReadWrite,
                    [System.IO.FileShare]::None
                )
                try { $packageReceiptStream.Flush($true) } finally { $packageReceiptStream.Dispose() }
                $preVerificationSourceDigest = Get-ProductionRustInputDigest -RepoRoot $repoRoot
                if ([string]$preVerificationSourceDigest.sha256 -cne [string]$releaseInputDigestAfter.sha256 -or
                    [int]$preVerificationSourceDigest.file_count -ne [int]$releaseInputDigestAfter.file_count -or
                    ((Get-CurrentInteractionPolicyInputs) | ConvertTo-Json -Compress) -cne
                        ($packageResult.interaction_closure.policy_inputs | ConvertTo-Json -Compress) -or
                    (Get-FileHash -LiteralPath $expectedPackagePath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$packageResult.sha256 -or
                    (Get-FileHash -LiteralPath $artifactAttestationPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$attested.receipt_sha256 -or
                    (Get-FileHash -LiteralPath $portablePackageScript -Algorithm SHA256).Hash.ToLowerInvariant() -cne $packageBuilderHash) {
                    throw "portable package promotion inputs changed before receipt publication"
                }
                $candidateReceiptHash = (Get-FileHash -LiteralPath $packageReceiptCandidate -Algorithm SHA256).Hash.ToLowerInvariant()
                $verificationStdout = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass `
                    -File $portablePackageVerifier `
                    -RepoRoot $repoRoot `
                    -ReceiptPath $packageReceiptCandidate `
                    -AllowStagedReceipt)
                if ($LASTEXITCODE -ne 0) {
                    throw "portable package receipt verifier exited with code $LASTEXITCODE"
                }
                $verificationResult = (($verificationStdout -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
                if ([string]$verificationResult.schema -cne 'battle-plan.central_portable_package_verification.v1' -or
                    [string]$verificationResult.status -cne 'PASS_STAGED_CENTRAL_RELEASE_PORTABLE_PACKAGE' -or
                    $verificationResult.canonical_receipt -isnot [bool] -or $verificationResult.canonical_receipt -or
                    $verificationResult.static_vcruntime_admission -isnot [bool] -or
                    -not $verificationResult.static_vcruntime_admission -or
                    [string]$verificationResult.interaction_closure.policy -cne 'passive_injected_observer' -or
                    $verificationResult.interaction_closure.zip_entry_bytes_reverified -isnot [bool] -or
                    -not $verificationResult.interaction_closure.zip_entry_bytes_reverified -or
                    ($verificationResult.interaction_closure.policy_inputs | ConvertTo-Json -Compress) -cne
                        ((Get-CurrentInteractionPolicyInputs) | ConvertTo-Json -Compress) -or
                    ($verificationResult.interaction_closure.source_capability | ConvertTo-Json -Compress) -cne
                        ($packageResult.interaction_closure.source_capability | ConvertTo-Json -Compress) -or
                    -not $verificationResult.interaction_closure.recorder_host.forbidden_symbols_absent -or
                    -not $verificationResult.interaction_closure.replay_app.forbidden_symbols_absent -or
                    -not $verificationResult.interaction_closure.collector_dll.forbidden_symbols_absent -or
                    [string]$verificationResult.receipt_sha256 -cne $candidateReceiptHash -or
                    [string]$verificationResult.package_sha256 -cne [string]$packageResult.sha256 -or
                    (($verificationResult.production_capture_evidence | ConvertTo-Json -Depth 12 -Compress) -cne
                        ($productionCaptureEvidence | ConvertTo-Json -Depth 12 -Compress)) -or
                    [string]$verificationResult.build_environment_sha256 -cne [string]$releaseBuildEnvironmentAfter.sha256) {
                    throw "portable package receipt verifier returned no promotion authority"
                }
                $postVerificationSourceDigest = Get-ProductionRustInputDigest -RepoRoot $repoRoot
                $postVerificationCaptureEvidence = Get-CurrentProductionCaptureEvidence
                if ([string]$postVerificationSourceDigest.sha256 -cne [string]$releaseInputDigestAfter.sha256 -or
                    [int]$postVerificationSourceDigest.file_count -ne [int]$releaseInputDigestAfter.file_count -or
                    ((Get-CurrentInteractionPolicyInputs) | ConvertTo-Json -Compress) -cne
                        ($packageResult.interaction_closure.policy_inputs | ConvertTo-Json -Compress) -or
                    (Get-FileHash -LiteralPath $packageReceiptCandidate -Algorithm SHA256).Hash.ToLowerInvariant() -cne $candidateReceiptHash -or
                    (Get-FileHash -LiteralPath $expectedPackagePath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$packageResult.sha256 -or
                    (Get-FileHash -LiteralPath $artifactAttestationPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$attested.receipt_sha256 -or
                    (Get-FileHash -LiteralPath $portablePackageScript -Algorithm SHA256).Hash.ToLowerInvariant() -cne $packageBuilderHash -or
                    (($postVerificationCaptureEvidence | ConvertTo-Json -Depth 12 -Compress) -cne
                        ($productionCaptureEvidence | ConvertTo-Json -Depth 12 -Compress))) {
                    throw "portable package promotion inputs changed after independent receipt verification"
                }
                Write-Host "Production artifact attestation: $artifactAttestationPath"
                Write-Host "Portable product package: $portablePackagePath"
                Write-Host "Publishing verified portable package receipt: $portablePackageReceiptPath"
                # This atomic, no-replace, write-through same-directory move is
                # deliberately terminal. It durably commits the parent namespace
                # only after the independent verifier accepts the candidate.
                [PortableReceiptPublishNative]::PublishNoReplaceWriteThrough(
                    $packageReceiptCandidate,
                    $portablePackageReceiptPath
                )

                # Publication is not success authority by itself. Re-open the
                # canonical receipt, package, and attestation through the
                # independent verifier, then repeat every mutable source/hash
                # check. Any mismatch flows to the outer fail-closed cleanup,
                # which removes all three published promotion artifacts.
                $publishedReceiptHash =
                    (Get-FileHash -LiteralPath $portablePackageReceiptPath -Algorithm SHA256).Hash.ToLowerInvariant()
                if ($publishedReceiptHash -cne $candidateReceiptHash) {
                    throw 'published portable package receipt differs from the verified candidate'
                }
                $canonicalVerificationStdout = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass `
                    -File $portablePackageVerifier `
                    -RepoRoot $repoRoot `
                    -ReceiptPath $portablePackageReceiptPath)
                if ($LASTEXITCODE -ne 0) {
                    throw "canonical portable package receipt verifier exited with code $LASTEXITCODE"
                }
                $canonicalVerification =
                    (($canonicalVerificationStdout -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
                if ([string]$canonicalVerification.schema -cne
                        'battle-plan.central_portable_package_verification.v1' -or
                    [string]$canonicalVerification.status -cne
                        'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE' -or
                    $canonicalVerification.canonical_receipt -isnot [bool] -or
                    -not $canonicalVerification.canonical_receipt -or
                    [string]$canonicalVerification.receipt_sha256 -cne $publishedReceiptHash -or
                    [string]$canonicalVerification.package_sha256 -cne [string]$packageResult.sha256 -or
                    (($canonicalVerification.production_capture_evidence | ConvertTo-Json -Depth 12 -Compress) -cne
                        ($productionCaptureEvidence | ConvertTo-Json -Depth 12 -Compress)) -or
                    [string]$canonicalVerification.build_environment_sha256 -cne
                        [string]$releaseBuildEnvironmentAfter.sha256 -or
                    [string]$canonicalVerification.interaction_closure.policy -cne
                        'passive_injected_observer' -or
                    -not $canonicalVerification.interaction_closure.zip_entry_bytes_reverified -or
                    ($canonicalVerification.interaction_closure.source_capability | ConvertTo-Json -Compress) -cne
                        ($packageResult.interaction_closure.source_capability | ConvertTo-Json -Compress)) {
                    throw 'published portable package receipt has no canonical verification authority'
                }
                $finalPublishedSourceDigest = Get-ProductionRustInputDigest -RepoRoot $repoRoot
                $finalPublishedCaptureEvidence = Get-CurrentProductionCaptureEvidence
                if ([string]$finalPublishedSourceDigest.sha256 -cne
                        [string]$releaseInputDigestAfter.sha256 -or
                    [int]$finalPublishedSourceDigest.file_count -ne
                        [int]$releaseInputDigestAfter.file_count -or
                    ((Get-CurrentInteractionPolicyInputs) | ConvertTo-Json -Compress) -cne
                        ($packageResult.interaction_closure.policy_inputs | ConvertTo-Json -Compress) -or
                    (Get-FileHash -LiteralPath $portablePackageReceiptPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne
                        $publishedReceiptHash -or
                    (Get-FileHash -LiteralPath $expectedPackagePath -Algorithm SHA256).Hash.ToLowerInvariant() -cne
                        [string]$packageResult.sha256 -or
                    (Get-FileHash -LiteralPath $artifactAttestationPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne
                        [string]$attested.receipt_sha256 -or
                    (Get-FileHash -LiteralPath $portablePackageScript -Algorithm SHA256).Hash.ToLowerInvariant() -cne
                        $packageBuilderHash -or
                    (($finalPublishedCaptureEvidence | ConvertTo-Json -Depth 12 -Compress) -cne
                        ($productionCaptureEvidence | ConvertTo-Json -Depth 12 -Compress))) {
                    throw 'published portable package promotion inputs changed during canonical re-verification'
                }
            }
            catch {
                $candidateFailure = $_
                try {
                    Remove-ReleasePromotionState `
                        -Paths @($packageReceiptCandidate) `
                        -Context 'staged receipt cleanup'
                }
                catch {
                    throw "portable receipt staging failed ($($candidateFailure.Exception.Message)); candidate cleanup also failed closed ($($_.Exception.Message))"
                }
                throw $candidateFailure
            }
        }
        catch {
            $releaseFailure = $_
            try {
                Remove-ReleasePromotionState `
                    -Paths @($artifactAttestationPath, $portablePackageReceiptPath, $portablePackagePath) `
                    -Context 'failed release cleanup'
            }
            catch {
                throw "centralized Release failed ($($releaseFailure.Exception.Message)); cleanup also failed closed ($($_.Exception.Message))"
            }
            throw $releaseFailure
        }
    }
}
catch {
    # A FAILING BATTERY MUST LEAVE A FAILING EXIT CODE, and without this it did
    # not have to.
    #
    # $LASTEXITCODE only ever holds the status of the last NATIVE command. Every
    # refusal in this file - the zero-tests-selected throw, the unstoppable
    # process tree, a rejected receipt, a partition that covers nothing - is a
    # PowerShell terminating error, which does not touch it. So after any cargo
    # or python step had succeeded, a later throw returned to the caller with
    # $LASTEXITCODE still 0, and this repo's `-Command` gate launcher ends with
    #
    #     & './Invoke-CentralRustValidation.ps1' ...; exit $LASTEXITCODE
    #
    # which then exits 0. A battery that refused to certify the tree announced
    # success. This is not hypothetical: it is how the serial half's queued
    # steps failed silently until 2026-08-15, and the same masking is available
    # to every other refusal in the file.
    #
    # $global: because this runs in the script's scope and the caller reads the
    # variable in its own. The error is re-thrown unchanged: the message is how
    # a human finds out WHAT failed, the exit code is how a script does.
    $global:LASTEXITCODE = 1
    $script:centralValidationFailed = $true
    throw
}
finally {
    # THE CLEANUP IS INSIDE ITS OWN try/catch, and the 0 is written LAST.
    #
    # A finally runs AFTER the catch above, so a failure in here is not covered
    # by it: `ReleaseMutex` on a mutex this thread does not own throws, and that
    # throw would have left whatever the last cargo step wrote in $LASTEXITCODE.
    # Equally, the run is not entitled to a zero until the cleanup has actually
    # succeeded - a released slot that was not released is a defect the next
    # battery pays for.
    try {
        # The parallel slot is released here as well as on the success path: a
        # throw anywhere in the parallel half would otherwise leave a slot held
        # until this process exited, and a run that fails early is exactly when
        # the machine should get its slot back soonest.
        if ($parallelSlotIndex -ge 0) {
            $parallelSlotMutexes[$parallelSlotIndex].ReleaseMutex()
            $parallelSlotIndex = -1
        }
        foreach ($slot in $parallelSlotMutexes) {
            $slot.Dispose()
        }
        if ($ownsMutex) {
            $mutex.ReleaseMutex()
        }
        $mutex.Dispose()

        # The ONLY place a zero is written. Reached only when the body did not
        # throw (the catch sets the flag) and the cleanup above did not either.
        if (-not $script:centralValidationFailed) {
            $global:LASTEXITCODE = 0
        }
    }
    catch {
        $global:LASTEXITCODE = 1
        $script:centralValidationFailed = $true
        throw
    }
}
