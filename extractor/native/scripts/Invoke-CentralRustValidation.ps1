[CmdletBinding()]
param(
    [ValidateSet("Check", "Build", "Fast", "Integration", "Test", "Final", "Soak", "Evidence", "Release", "Lock", "Stats")]
    [string]$Mode = "Final",

    [ValidateSet("All", "Extractor", "Replay")]
    [string]$Workspace = "All",

    [string[]]$Package = @(),

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

    # The portable-product package builder/verifier self-test is the one gate in
    # this runner whose cost is measured in tens of minutes rather than seconds,
    # because it drives the package builder once per mutation across four
    # forbidden-capability rosters. It is a source-only test: nothing it can
    # detect changes between two runs unless the packaging scripts or the
    # production source tree change. Running it on every Final/Release made the
    # routine gate unaffordable, so it is opt-in. It is never skipped silently -
    # the default path prints exactly what was not run and how to run it.
    [switch]$RunPackageSelfTest,

    # A full Release without this path emits only fresh candidate artifacts for
    # a real capture. Supplying the resulting production receipt on a second
    # full Release is the only route to a portable package/production receipt.
    [string]$ProductionCaptureReceiptPath = "",

    # Where the C07 gate looks for the hash-pinned shipped installed content. Empty
    # means the generator's own default install path. A machine without the game
    # runs the reduced committed-artifact check and says so; this switch is for
    # a machine that HAS the game somewhere else, so it can carry the full
    # game-derived authority rather than the reduced one.
    [string]$C07GameRoot = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# BEFORE ANY EARLY RETURN AND BEFORE THE MACHINE-WIDE MUTEX. `-Features` is
# honoured by three modes; every other mode must REFUSE it rather than run and
# silently ignore it, and `Stats` returns long before the argument assembly
# below, so a check down there would let `-Mode Stats -Features …` pass
# quietly. A selector that is accepted in silence is the same defect as a gate
# that validates the wrong tree.
if ($Features.Count -gt 0 -and $Mode -notin @("Check", "Build", "Test")) {
    throw "-Features is accepted by Check, Build and Test only; Final is one canonical configuration."
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
$portablePackageSelfTest = Join-Path $PSScriptRoot "Test-PortableProductPackageSelfTest.ps1"
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
$gapLedgerTests = Join-Path $PSScriptRoot "test_gap_ledger.py"
$gapLedgerExpansionTests = Join-Path $PSScriptRoot "Test-GapLedgerExpansion.ps1"
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

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Program,

        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    Write-Host ("> {0} {1}" -f $Program, ($Arguments -join " "))
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program exited with code $LASTEXITCODE"
    }
}

function Invoke-CheckedNextestWithCensus {
    <#
    .SYNOPSIS
    Run a `cargo nextest` selection, having first proved it selects tests.

    .DESCRIPTION
    Used for the SERIAL half only, and it exists for one defect: a serial
    filterset that matches nothing. nextest exits ZERO for a filterset that
    selects no tests -- correctly, there is nothing to fail -- so a partition
    that stopped naming the machine-serial packages would produce a battery
    that compiled them, ran them nowhere, and reported green. The parallel half
    needs no such check: it is the complement, so if the serial half is
    non-empty the parallel half was selected from a real set too.

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
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string[]]$ListArguments,
        [Parameter(Mandatory = $true)][string]$Context
    )

    # The list arguments are passed in rather than derived from the run's by
    # swapping `run` for `list`: `--no-fail-fast` and `--timings` are run-only
    # and nextest exits 2 on them, which the derived spelling turned into "this
    # selection could not be listed" for every healthy run.
    #
    # No 2>&1: cargo's stderr goes to the console, where it belongs, and only
    # stdout -- the JSON -- is captured.
    $listJson = & cargo @ListArguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Context could not be listed: cargo exited with code $LASTEXITCODE"
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
            if ([string]$case.Value.'filter-match'.status -ceq 'matches') {
                $count++
            }
        }
    }
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
    param(
        [Parameter(Mandatory = $true)][string]$Describe,
        [Parameter(Mandatory = $true)][scriptblock]$Action
    )
    $serialPhaseActions.Add([ordered]@{ Describe = $Describe; Action = $Action })
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
    $splitBattery = ($Mode -eq 'Final' -and
        $Package.Count -eq 0 -and
        [string]::IsNullOrWhiteSpace($FilterExpression))
    $batteryPartition = $null

    if ($splitBattery) {
        # Computed BEFORE any slot is taken: a partition that cannot be derived
        # is a refusal, not a battery that runs everything in one half.
        $batteryPartition = Get-BatteryPartition -RepoRoot $repoRoot
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

    # The package self-test only has a place in the Final and Release batteries.
    # Accepting the switch elsewhere and quietly doing nothing would be the same
    # silent skip the switch exists to prevent - worse, because the caller asked
    # for the test by name and would be told nothing.
    if ($RunPackageSelfTest -and $Mode -notin @("Final", "Release")) {
        throw (
            "-RunPackageSelfTest is only meaningful for -Mode Final or -Mode Release; " +
            "mode '$Mode' does not run the portable-product package self-test. Run it " +
            "on its own instead: powershell.exe -NoProfile -ExecutionPolicy Bypass " +
            "-File $portablePackageSelfTest"
        )
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
    #    path;
    #  * Test-PassiveCapturePolicy.ps1 and Test-HeadlessRawCaptureUnits.ps1 build
    #    their scratch under GetTempPath() with $PID / a fresh GUID in the name;
    #  * the C07 gate runs --verify, which writes nothing at all and reports the
    #    files it did not write;
    #  * Assert-PinnedRustToolchain only asks three binaries their version.
    #
    # -RunPackageSelfTest is the one opt-in gate here whose cost is measured in
    # tens of minutes. It is source-only and emits into this worktree's own
    # .cargo-target, so it belongs in the parallel half too - which is also where
    # it does the most good, since it is exactly the kind of long run nobody
    # should be holding the machine for.
    if ($Workspace -in @("All", "Extractor") -and
        $Mode -in @("Check", "Build", "Fast", "Integration", "Test", "Final", "Release")) {
        # These source checks run before Cargo and, for Release, before stale
        # promotion authority is removed. A changed shipping fold must first
        # force an explicit review of every wire-format transcription it backs.
        Invoke-Checked -Program "python" -Arguments @($worldFoldCitationTests)
        Invoke-Checked -Program "python" -Arguments @($citationIntegrityTests)
        Invoke-Checked -Program "python" -Arguments @($captureScorecardTests)
        Invoke-Checked -Program "python" -Arguments @($clockBaselineSummaryTests)
        Invoke-Checked -Program "python" -Arguments @($gapLedgerTests)
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
            [string]$partitionTestResult.status -cne 'PASS' -or
            # Pinned for the same reason every other case count here is: cases
            # quietly ceasing to run is the failure this whole file guards
            # against, and it looks exactly like a pass.
            [int]$partitionTestResult.cases -ne 57) {
            throw "battery partition source tests returned no authority"
        }

        Invoke-Checked -Program "python" -Arguments @(
            $worldFoldCitationVerifier,
            "--repo-root",
            $repoRoot
        )
        Invoke-Checked -Program "python" -Arguments @(
            $citationIntegrityVerifier,
            "--repo-root",
            $repoRoot
        )
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
        $Mode -in @("Check", "Build", "Fast", "Integration", "Test", "Evidence", "Final", "Release")) {
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

    if ($Mode -in @("Check", "Build", "Fast", "Integration", "Test", "Final", "Release")) {
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

    $env:CARGO_TARGET_DIR = $targetDir
    if (Get-Command sccache -ErrorAction SilentlyContinue) {
        if (-not $env:SCCACHE_CACHE_SIZE) {
            $env:SCCACHE_CACHE_SIZE = "30G"
        }
        if (-not $env:SCCACHE_SERVER_PORT) {
            $env:SCCACHE_SERVER_PORT = "4227"
        }
    }
    if ($CacheMode -eq "Reusable") {
        if (-not (Get-Command sccache -ErrorAction SilentlyContinue)) {
            throw "Reusable cache mode requires sccache on PATH."
        }
        $env:RUSTC_WRAPPER = "sccache"
        $env:CARGO_INCREMENTAL = "0"
    }
    else {
        Remove-Item Env:RUSTC_WRAPPER -ErrorAction SilentlyContinue
        $env:CARGO_INCREMENTAL = "1"
    }
    if (Get-Command lld-link -ErrorAction SilentlyContinue) {
        $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = "lld-link.exe"
    }

    if ($Mode -eq "Stats") {
        Invoke-Checked -Program "sccache" -Arguments @("--show-stats")
        return
    }

    if ($Mode -in @('Final', 'Release')) {
        $interactionClosureOutput = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $productionInteractionClosureTest)
        if ($LASTEXITCODE -ne 0) {
            throw "production interaction-closure source tests exited with code $LASTEXITCODE"
        }
        $interactionClosureResult = (($interactionClosureOutput -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
        if ([string]$interactionClosureResult.schema -cne 'battle-plan.production_interaction_closure_test.v1' -or
            [string]$interactionClosureResult.status -cne 'PASS' -or
            # Test-ProductionInteractionClosure.ps1 emits 36. This pin said 30
            # for at least three commits while the test emitted 35, which means
            # every Final and Release threw here before reaching any of the
            # gates below - the case-count pin is the second pin site that gets
            # missed when cases are added, exactly like the marker census.
            [int]$interactionClosureResult.cases -ne 36 -or
            [string]$interactionClosureResult.policy -cne 'passive_injected_observer') {
            throw "production interaction-closure source tests returned no authority"
        }

        $passivePolicyOutput = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $passiveCapturePolicyTest)
        if ($LASTEXITCODE -ne 0) {
            throw "passive capture-policy source tests exited with code $LASTEXITCODE"
        }
        $passivePolicyResult = (($passivePolicyOutput -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
        if ([string]$passivePolicyResult.Schema -cne 'hoi4-passive-capture-policy-test-v1' -or
            [string]$passivePolicyResult.Status -cne 'pass') {
            throw "passive capture-policy source tests returned no authority"
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
            [string]$promotionGateAbsenceResult.status -cne 'PASS' -or
            # Pinned so that cases silently ceasing to run is itself a failure --
            # the same "absence is not evidence" property the cases assert.
            [int]$promotionGateAbsenceResult.cases -ne 80) {
            throw "promotion-gate absence proof returned no authority"
        }

        $staticCrtTestOutput = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $portableStaticCrtTest)
        if ($LASTEXITCODE -ne 0) {
            throw "portable-product static-CRT admission tests exited with code $LASTEXITCODE"
        }
        $staticCrtTestResult = (($staticCrtTestOutput -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
        if ([string]$staticCrtTestResult.schema -cne 'battle-plan.portable_product_static_crt_test.v1' -or
            [string]$staticCrtTestResult.status -cne 'PASS' -or
            [int]$staticCrtTestResult.cases -ne 4) {
            throw "portable-product static-CRT admission tests returned no authority"
        }

        if ($RunPackageSelfTest) {
            $packageSelfTestOutput = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $portablePackageSelfTest)
            if ($LASTEXITCODE -ne 0) {
                throw "portable-product package builder/verifier self-tests exited with code $LASTEXITCODE"
            }
            $packageSelfTestResult = (($packageSelfTestOutput -join [Environment]::NewLine).Trim() | ConvertFrom-Json)
            if ([string]$packageSelfTestResult.schema -cne 'battle-plan.portable_package_selftest.v1' -or
                [string]$packageSelfTestResult.status -cne 'PASS' -or
                # 61 + 13. Five are the three pinned manifest-graph cases, the
                # case that the derived fixture reaches exactly the delegating unit
                # closure the committed baseline accepts, and the case that a
                # drifted accepted-inventory pin is rejected. The other eight cover
                # the ways this authority can go blind without failing: a
                # suspending code mutation whose freeze and patch sit on opposite
                # sides of a dependency edge (in this repository, and outside it
                # both as a direct call and through a function pointer, with the
                # name-definition case that must NOT count), a reachability flag
                # pinned as a string, and a lane leaving the manifest walk that
                # decides which files are scanned at all.
                [int]$packageSelfTestResult.cases -ne 74) {
                throw "portable-product package builder/verifier self-tests returned no authority"
            }
        }
        else {
            Write-Host (
                'SKIPPED BY POLICY: portable-product package builder/verifier ' +
                'self-tests (Test-PortableProductPackageSelfTest.ps1, 66 cases) ' +
                'did NOT run. This validation carries no package ' +
                'builder/verifier authority. Re-run this command with ' +
                '-RunPackageSelfTest, or run the test on its own: ' +
                "powershell.exe -NoProfile -ExecutionPolicy Bypass -File $portablePackageSelfTest"
            )
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

    $hasSpecificTarget = $Lib -or $Bin.Count -gt 0 -or $TestTarget.Count -gt 0
    if ($Mode -eq "Lock" -and
        ($Package.Count -gt 0 -or -not [string]::IsNullOrWhiteSpace($FilterExpression) -or
         $AllTargets -or $hasSpecificTarget -or $Timings)) {
        throw "Lock regenerates the selected top-level workspace lockfile and does not accept package, filter, target, or timing selectors."
    }
    if ($AllTargets -and $hasSpecificTarget) {
        throw "-AllTargets cannot be combined with -Lib, -Bin, or -TestTarget."
    }
    if ($Mode -in @("Final", "Soak", "Release") -and ($AllTargets -or $hasSpecificTarget)) {
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

    $workspaceRoots = switch ($Workspace) {
        "Extractor" { @(Join-Path $repoRoot "extractor\native") }
        "Replay" { @(Join-Path $repoRoot "replay_interface\native") }
        default {
            @(
                (Join-Path $repoRoot "extractor\native"),
                (Join-Path $repoRoot "replay_interface\native")
            )
        }
    }

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
            foreach ($name in $Package) {
                $selection += @("--package", $name)
            }
            if ($Package.Count -eq 0) {
                $selection += "--workspace"
            }

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
                        # Captured by value into the closure: $workspaceRoot and
                        # the argument list are loop variables, and a script
                        # block that read them at drain time would run every
                        # queued action against the LAST workspace.
                        $serialRoot = $workspaceRoot
                        $serialName = $workspaceName
                        $serialArguments = @("nextest", "run", "--all-targets", "--no-fail-fast") +
                            $nextestConfigSelection + $nextestCiProfileSelection +
                            $timingSelection + $selection + @("-E", $rootHalves.Serial)
                        $serialListArguments = @("nextest", "list", "--all-targets") +
                            $nextestConfigSelection + $nextestCiProfileSelection +
                            $selection + @("-E", $rootHalves.Serial, "--message-format", "json")
                        Add-SerialPhaseAction -Describe (
                            "$serialName machine-serial tests ({0})" -f ($rootHalves.SerialPackages -join ', ')
                        ) -Action {
                            Push-Location $serialRoot
                            try {
                                Invoke-CheckedNextestWithCensus `
                                    -Arguments $serialArguments `
                                    -ListArguments $serialListArguments `
                                    -Context "$serialName machine-serial tests"
                                Save-CargoTimingReport -WorkspaceName $serialName -Label "final-nextest-serial"
                            }
                            finally { Pop-Location }
                        }.GetNewClosure()
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
                    $doctestRoot = $workspaceRoot
                    $doctestName = $workspaceName
                    $doctestArguments = @("test", "--doc", "--no-fail-fast") + $timingSelection + $selection
                    if ($splitBattery) {
                        Add-SerialPhaseAction -Describe "$doctestName doctests (unsplit: cargo feature unification)" -Action {
                            Push-Location $doctestRoot
                            try {
                                Invoke-Checked -Program "cargo" -Arguments $doctestArguments
                                Save-CargoTimingReport -WorkspaceName $doctestName -Label "final-doctest"
                            }
                            finally { Pop-Location }
                        }.GetNewClosure()
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
                                $detourRoot = $workspaceRoot
                                Add-SerialPhaseAction -Describe 'release detour-identity gate' -Action {
                                    Push-Location $detourRoot
                                    try { Invoke-ReleaseDetourIdentityGate }
                                    finally { Pop-Location }
                                }.GetNewClosure()
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
                                'Run an unnarrowed -Mode Final, or -Mode Release.'
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

    # The extractor-owned standalone cargo roots. They belong to one workspace's
    # tree but are deliberately not members of it.
    $extractorStandaloneRoots = @(
        (Join-Path $repoRoot "extractor\native\project_progress_live_lane"),
        (Join-Path $repoRoot "extractor\native\lanes\occupation_resistance_compliance_live\replay_verify")
    )

    # The neutral portable_replay crates. These CANNOT be members of either
    # workspace: both path-depend on them, and a cargo package belongs to at
    # most one workspace. Each is therefore a cargo root in its own right, with
    # its own committed Cargo.lock, and is built (never tested) as a path
    # dependency of the two workspaces above.
    #
    # That is a structural fact, not a decision to leave them unverified.
    # Until they were listed here nothing ran their `#[cfg(test)]` modules at
    # all, so `--workspace` reported green while ~190 tests in the crates BOTH
    # halves statically link had never executed. They are validated for every
    # -Workspace selection because both halves depend on them.
    $portableStandaloneRoots = @(
        (Join-Path $repoRoot "portable_replay\native\admission"),
        (Join-Path $repoRoot "portable_replay\native\compact_stream"),
        (Join-Path $repoRoot "portable_replay\native\content_manifest"),
        (Join-Path $repoRoot "portable_replay\native\render_stream"),
        (Join-Path $repoRoot "portable_replay\native\snapshot")
    )

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
                    $queuedRoot = $standaloneRoot
                    $queuedManifest = $standaloneManifest
                    Add-SerialPhaseAction -Describe (
                        "standalone root {0} ({1})" -f `
                            (Split-Path -Leaf $standaloneRoot), ($standaloneSerialPackages -join ', ')
                    ) -Action {
                        Push-Location $queuedRoot
                        try {
                            Invoke-CheckedNextestWithCensus `
                                -Arguments @("nextest", "run", "--locked", "--all-targets", "--no-fail-fast") `
                                -ListArguments @("nextest", "list", "--locked", "--all-targets", "--message-format", "json") `
                                -Context ("standalone root {0}" -f (Split-Path -Leaf $queuedRoot))
                            if (Test-CargoManifestHasLibraryTarget -ManifestPath $queuedManifest) {
                                Invoke-Checked -Program "cargo" -Arguments @("test", "--locked", "--doc", "--no-fail-fast")
                            }
                            else {
                                Write-Host "Skipping standalone doctests: no library target in $queuedManifest"
                            }
                        }
                        finally { Pop-Location }
                    }.GetNewClosure()
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
        if ($serialPhaseActions.Count -eq 0) {
            throw (
                'the battery split queued NO machine-serial work. That is not a ' +
                'pass: this run compiled the packages the partition calls ' +
                'machine-serial and then ran none of them under the mutex, so ' +
                'its verdict covers only the parallel half.'
            )
        }
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
            & $queued.Action
        }
        $serialPhaseSeconds = ((Get-Date) - $serialPhaseStart).TotalSeconds
        $sealTemplate =
            'Battery seal: BOTH halves passed ({0} machine-serial step(s) after the ' +
            'parallel half); central validation mutex held for {1:N1} s.'
        Write-Host ($sealTemplate -f $serialPhaseActions.Count, $serialPhaseSeconds)
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
finally {
    # The parallel slot is released here as well as on the success path: a throw
    # anywhere in the parallel half would otherwise leave a slot held until this
    # process exited, and a run that fails early is exactly when the machine
    # should get its slot back soonest.
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
}
