[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$RepoRoot,
    [Parameter(Mandatory = $true)][string]$ReceiptPath,
    [switch]$AllowStagedReceipt
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = [IO.Path]::GetFullPath($RepoRoot)
$receiptFull = [IO.Path]::GetFullPath($ReceiptPath)
# Neutral, shared build machinery; see Invoke-ReleaseManual.ps1.
$attestationHelper = Join-Path $repo 'portable_replay\native\scripts\ProductionArtifact.Attestation.ps1'
$builder = Join-Path $repo 'extractor\native\scripts\New-PortableProductPackage.ps1'
$driver = Join-Path $repo 'extractor\native\scripts\Start-Hoi4ProductionCaptureSafe.ps1'
$readOnlyWitness = Join-Path $repo 'extractor\native\scripts\Read-Hoi4GameState.ps1'
$passivePolicy = Join-Path $repo 'extractor\native\scripts\Test-PassiveCapturePolicy.ps1'
$passiveLifecycle = Join-Path $repo 'extractor\native\scripts\PassiveCapture.Lifecycle.ps1'
. (Join-Path $repo 'extractor\native\scripts\PortableProduct.InteractionClosure.ps1')
. (Join-Path $repo 'extractor\native\scripts\ProductionCapture.Promotion.ps1')
$productionSourceCapabilityPaths = [ordered]@{
    recorder_host_cargo_sha256 = Join-Path $repo 'extractor\native\recorder_host\Cargo.toml'
    recorder_host_main_sha256 = Join-Path $repo 'extractor\native\recorder_host\src\main.rs'
    recorder_host_early_render_launch_sha256 = Join-Path $repo 'extractor\native\recorder_host\src\early_render_launch.rs'
    recorder_host_auto_attach_sha256 = Join-Path $repo 'extractor\native\recorder_host\src\auto_attach.rs'
    extractor_workspace_cargo_sha256 = Join-Path $repo 'extractor\native\Cargo.toml'
    collector_cargo_sha256 = Join-Path $repo 'extractor\native\collector\Cargo.toml'
    character_lane_cargo_sha256 = Join-Path $repo 'extractor\native\lanes\character_presentation_live\Cargo.toml'
    focus_lane_cargo_sha256 = Join-Path $repo 'extractor\native\lanes\focus_research_decisions_depth_live\Cargo.toml'
    collector_lib_sha256 = Join-Path $repo 'extractor\native\collector\src\lib.rs'
    passive_ready_contract_sha256 = Join-Path $repo 'extractor\native\compact_contract\src\passive_ready.rs'
    interaction_closure_helper_sha256 = Join-Path $repo 'extractor\native\scripts\PortableProduct.InteractionClosure.ps1'
    production_capture_promotion_helper_sha256 = Join-Path $repo 'extractor\native\scripts\ProductionCapture.Promotion.ps1'
}
$expectedReceipt = Join-Path $repo '.cargo-target\release\hoi4-portable-package-receipt.json'
$expectedAttestation = Join-Path $repo '.cargo-target\release\hoi4-production-prebuilt-artifacts.json'
$expectedReceiptFull = [IO.Path]::GetFullPath($expectedReceipt)
if ($AllowStagedReceipt) {
    $expectedParent = Split-Path -Parent $expectedReceiptFull
    $candidateName = [IO.Path]::GetFileName($receiptFull)
    if ((Split-Path -Parent $receiptFull) -ine $expectedParent -or
        $candidateName -cnotmatch '^hoi4-portable-package-receipt\.json\.[0-9a-f]{32}\.candidate$') {
        throw 'staged portable package receipt is outside the canonical release directory or has a noncanonical candidate name'
    }
}
elseif ($receiptFull -ine $expectedReceiptFull) {
    throw 'portable package receipt is outside the canonical release location'
}
. $attestationHelper

Add-Type -AssemblyName System.IO.Compression
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class PortablePackageReceiptNative {
    private const uint GENERIC_READ = 0x80000000;
    private const uint FILE_SHARE_READ = 0x1;
    private const uint OPEN_EXISTING = 3;
    private const uint FILE_FLAG_OPEN_REPARSE_POINT = 0x00200000;
    private const uint FILE_ATTRIBUTE_DIRECTORY = 0x10;
    private const uint FILE_ATTRIBUTE_REPARSE_POINT = 0x400;
    [StructLayout(LayoutKind.Sequential)] public struct FILETIME { public uint Low; public uint High; }
    [StructLayout(LayoutKind.Sequential)] public struct INFO {
        public uint Attributes; public FILETIME Creation; public FILETIME Access; public FILETIME Write;
        public uint Volume; public uint SizeHigh; public uint SizeLow; public uint Links; public uint IndexHigh; public uint IndexLow;
    }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    private static extern SafeFileHandle CreateFileW(string name, uint access, uint share, IntPtr security, uint creation, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError=true)] private static extern bool GetFileInformationByHandle(SafeFileHandle handle, out INFO info);
    public static SafeFileHandle Open(string path) {
        var handle = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, IntPtr.Zero, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, IntPtr.Zero);
        if (handle.IsInvalid) throw new Win32Exception(Marshal.GetLastWin32Error(), "open portable package authority failed: " + path);
        INFO info;
        if (!GetFileInformationByHandle(handle, out info)) { int e=Marshal.GetLastWin32Error(); handle.Dispose(); throw new Win32Exception(e); }
        if ((info.Attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)) != 0) { handle.Dispose(); throw new InvalidDataException("portable package authority is a directory or reparse point: " + path); }
        return handle;
    }
}
'@

function Read-LockedBytes([string]$Path) {
    $handle = [PortablePackageReceiptNative]::Open($Path)
    $stream = [IO.FileStream]::new($handle, [IO.FileAccess]::Read)
    try {
        if ($stream.Length -le 0 -or $stream.Length -gt [int]::MaxValue) { throw "invalid authority file length: $Path" }
        $bytes = [byte[]]::new([int]$stream.Length)
        $offset = 0
        while ($offset -lt $bytes.Length) {
            $read = $stream.Read($bytes, $offset, $bytes.Length - $offset)
            if ($read -eq 0) { throw "authority file truncated during locked read: $Path" }
            $offset += $read
        }
        return ,$bytes
    }
    finally { $stream.Dispose() }
}

function Get-ByteHash([byte[]]$Bytes) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { ([BitConverter]::ToString($sha.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
}

function Find-AsciiByteSequence {
    param(
        [Parameter(Mandatory = $true)][byte[]]$Bytes,
        [Parameter(Mandatory = $true)][string]$Text
    )

    $needle = [Text.Encoding]::ASCII.GetBytes($Text)
    if ($needle.Length -eq 0 -or $Bytes.Length -lt $needle.Length) { return $false }
    for ($offset = 0; $offset -le $Bytes.Length - $needle.Length; $offset++) {
        $matches = $true
        for ($index = 0; $index -lt $needle.Length; $index++) {
            if ($Bytes[$offset + $index] -ne $needle[$index]) {
                $matches = $false
                break
            }
        }
        if ($matches) { return $true }
    }
    return $false
}

function Assert-VerifiedInteractionClosure {
    param(
        [Parameter(Mandatory = $true)][byte[]]$Bytes,
        [Parameter(Mandatory = $true)][string]$Name,
        # replay_app is required to carry no linker-retained production marker,
        # and that empty roster is passed explicitly rather than omitted, so it
        # has to be bindable. Mandatory alone rejects an empty collection, which
        # made this enforcer unable to run on the replay role at all.
        [Parameter(Mandatory = $true)]
        [AllowEmptyCollection()]
        [string[]]$RequiredMarkers,
        [Parameter(Mandatory = $true)][uint16]$ExpectedPinnedHookCount,
        [Parameter(Mandatory = $true)][string]$PinnedHookContractSourceSha256,
        [string[]]$PermittedSelfUiCapabilities = @(),
        [string[]]$PermittedSelfWorkerCapabilities = @(),
        [string[]]$PermittedInjectionCapabilities = @()
    )

    $knownSelfUiCapabilities = @(
        'SendMessageW',
        'ShellExecuteW',
        'EnableWindow',
        'SetWindowTextW',
        'ShowWindow',
        'DestroyWindow'
    )
    if (@($PermittedSelfUiCapabilities | Where-Object {
        $knownSelfUiCapabilities -cnotcontains $_
    }).Count -ne 0 -or
        @($PermittedSelfUiCapabilities | Select-Object -Unique).Count -ne
            $PermittedSelfUiCapabilities.Count) {
        throw "$Name declares an invalid recorder self-UI exception"
    }
    $knownSelfWorkerCapabilities = @(
        'CreateProcessW',
        'TerminateProcess',
        'CreateJobObjectW',
        'AssignProcessToJobObject',
        'SetInformationJobObject'
    )
    if (@($PermittedSelfWorkerCapabilities | Where-Object {
        $knownSelfWorkerCapabilities -cnotcontains $_
    }).Count -ne 0 -or
        @($PermittedSelfWorkerCapabilities | Select-Object -Unique).Count -ne
            $PermittedSelfWorkerCapabilities.Count) {
        throw "$Name declares an invalid replay self-worker exception"
    }
    $permittedRoleCapabilities =
        @($PermittedSelfUiCapabilities) + @($PermittedSelfWorkerCapabilities)
    $forbiddenSymbols = @(
        @(Get-ForbiddenProductionInteractionMarkers) | Where-Object {
            $permittedRoleCapabilities -cnotcontains $_
        }
    )
    $foundForbiddenSymbols = @(
        $forbiddenSymbols | Where-Object {
            Find-AsciiByteSequence -Bytes $Bytes -Text $_
        }
    )
    # There is deliberately no separate lifecycle/UI verdict here. The lifecycle
    # and UI rosters are both subsets of the interaction-marker roster above, and
    # a permitted role capability is removed from $forbiddenSymbols by the same
    # $permittedRoleCapabilities filter, so any byte sequence that would set such
    # a verdict is already in $foundForbiddenSymbols and throws below. A field
    # reporting it could only ever be false on the returned receipt, which reads
    # as "this was checked and came back clean" when nothing was left to check.
    $foundSemanticWriteMarkers = @(
        @(
            '--hour-latch',
            'acquire_hour_latch',
            'release_hour_latch',
            'write_pause_state',
            'saved_pause_state'
        ) | Where-Object {
            Find-AsciiByteSequence -Bytes $Bytes -Text $_
        }
    )
    foreach ($marker in $RequiredMarkers) {
        if (-not (Find-AsciiByteSequence -Bytes $Bytes -Text $marker)) {
            throw "$Name lacks required linker-retained production marker $marker"
        }
    }
    if ($foundForbiddenSymbols.Count -ne 0) {
        throw "$Name contains forbidden client-driving or semantic-state-write symbol $($foundForbiddenSymbols[0])"
    }
    $forbiddenSymbolsAbsent = ($foundForbiddenSymbols.Count -eq 0)
    $semanticEngineStateWrites = ($foundSemanticWriteMarkers.Count -ne 0)
    return [ordered]@{
        schema = 'hoi4.interaction-closure.v1'
        policy = 'passive_injected_observer'
        required_markers = @($RequiredMarkers)
        exact_pinned_hook_count = [int]$ExpectedPinnedHookCount
        exact_pinned_hook_contract = 'a729.passive_ready.pinned_production_hook_specs.v1'
        pinned_hook_contract_source_sha256 = $PinnedHookContractSourceSha256
        forbidden_target_client_capabilities = @($forbiddenSymbols)
        forbidden_symbol_count = $forbiddenSymbols.Count
        forbidden_symbol_roster_sha256 = Get-ProductionInteractionMarkerRosterSha256
        forbidden_symbols_absent = $forbiddenSymbolsAbsent
        permitted_self_ui_capabilities = @($PermittedSelfUiCapabilities)
        permitted_self_worker_capabilities = @($PermittedSelfWorkerCapabilities)
        permitted_injection_capabilities = @($PermittedInjectionCapabilities)
        semantic_engine_state_writes = $semanticEngineStateWrites
    }
}

function Get-VerifiedProductionSourceCapabilityClosure {
    $hostCargo = Get-Content -LiteralPath $productionSourceCapabilityPaths.recorder_host_cargo_sha256 -Raw
    $hostMain = Get-Content -LiteralPath $productionSourceCapabilityPaths.recorder_host_main_sha256 -Raw
    $earlyRender = Get-Content -LiteralPath $productionSourceCapabilityPaths.recorder_host_early_render_launch_sha256 -Raw
    $autoAttach = Get-Content -LiteralPath $productionSourceCapabilityPaths.recorder_host_auto_attach_sha256 -Raw
    $workspaceCargo = Get-Content -LiteralPath $productionSourceCapabilityPaths.extractor_workspace_cargo_sha256 -Raw
    $collectorCargo = Get-Content -LiteralPath $productionSourceCapabilityPaths.collector_cargo_sha256 -Raw
    $characterCargo = Get-Content -LiteralPath $productionSourceCapabilityPaths.character_lane_cargo_sha256 -Raw
    $focusCargo = Get-Content -LiteralPath $productionSourceCapabilityPaths.focus_lane_cargo_sha256 -Raw
    $collectorLib = Get-Content -LiteralPath $productionSourceCapabilityPaths.collector_lib_sha256 -Raw
    $passiveReady = Get-Content -LiteralPath $productionSourceCapabilityPaths.passive_ready_contract_sha256 -Raw
    $sourceTreeAuthority = Get-ProductionInteractionSourceTreeAuthority -RepoRoot $repo
    if ([int]$expectedPolicyInputs.production_interaction_source_tree_file_count -ne
            [int]$sourceTreeAuthority.file_count -or
        [string]$expectedPolicyInputs.production_interaction_source_tree_sha256 -cne
            [string]$sourceTreeAuthority.sha256) {
        throw 'production interaction source tree differs from its receipt'
    }
    Assert-ProductionInteractionSourceTreeClosure `
        -RepoRoot $repo `
        -Authority $sourceTreeAuthority
    $passiveSourceAdmission =
        Get-ProductionPassiveSourceAdmissionClosure -RepoRoot $repo

    if (-not $hostCargo.Contains('default = []') -or
        -not $hostCargo.Contains('evidence-client-driving = []') -or
        -not $hostCargo.Contains('name = "collector_record_evidence"') -or
        -not $hostCargo.Contains('required-features = ["evidence-client-driving"]') -or
        -not $hostMain.Contains('#[cfg(not(feature = "evidence-client-driving"))]') -or
        -not $hostMain.Contains('env!("CARGO_BIN_NAME") == "collector_record_evidence"') -or
        -not $hostMain.Contains('evidence-client-driving may only build collector_record_evidence')) {
        throw 'recorder source does not physically separate the canonical observer from the evidence actuator'
    }
    $actuatorStart = $earlyRender.IndexOf(
        '#[cfg(all(windows, feature = "evidence-client-driving"))]',
        [StringComparison]::Ordinal
    )
    $observerStart = $earlyRender.IndexOf(
        '#[cfg(all(windows, not(feature = "evidence-client-driving")))]',
        [StringComparison]::Ordinal
    )
    if ($actuatorStart -lt 0 -or $observerStart -le $actuatorStart -or
        -not $earlyRender.Contains('mod windows {') -or
        -not $earlyRender.Contains('mod observer_only {') -or
        -not $earlyRender.Contains('pub use observer_only::{') -or
        -not $earlyRender.Contains('client-driving is physically excluded from the production observer binary')) {
        throw 'early-render source lacks a physical evidence/observer module boundary'
    }
    foreach ($symbol in @(
        'CreateProcessW',
        'TerminateProcess',
        'ResumeThread',
        'CreateJobObjectW',
        'SetInformationJobObject'
    )) {
        $first = $earlyRender.IndexOf($symbol, [StringComparison]::Ordinal)
        $last = $earlyRender.LastIndexOf($symbol, [StringComparison]::Ordinal)
        if ($first -le $actuatorStart -or $last -ge $observerStart) {
            throw "early-render target lifecycle capability escapes the evidence-only module: $symbol"
        }
    }
    foreach ($symbol in @(
        'TerminateProcess',
        'AssignProcessToJobObject',
        'SetInformationJobObject',
        '.kill()'
    )) {
        if ($autoAttach.Contains($symbol)) {
            throw "production auto-attach source contains forbidden target lifecycle capability: $symbol"
        }
    }
    Assert-CollectorProductionDependencyFeatureClosure `
        -CollectorCargo $collectorCargo `
        -WorkspaceCargo $workspaceCargo
    Assert-ProtectedLaneDefaultFeatureClosure `
        -Cargo $characterCargo `
        -ForbiddenFeature 'a729-fixture-hook' `
        -LaneName 'character_presentation_live_lane'
    Assert-ProtectedLaneDefaultFeatureClosure `
        -Cargo $focusCargo `
        -ForbiddenFeature 'a729-focus-fixture-hook' `
        -LaneName 'focus_research_decisions_depth_live'
    # Same change of shape as the builder: the fixture hooks are deleted, so the
    # verifier asserts they are ABSENT rather than that they are conditional.
    Assert-CollectorRetiredEvidenceFeatureAbsence -CollectorCargo $collectorCargo
    if (-not $collectorCargo.Contains('crate-type = ["cdylib", "rlib"]') -or
        -not $collectorLib.Contains('BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1') -or
        $collectorLib.Contains('mod hour_latch') -or
        -not $collectorLib.Contains('pub mod read_only_scheduler')) {
        throw 'collector source does not close the passive production cdylib boundary'
    }
    $rosterDeclaration =
        'pub const PINNED_PRODUCTION_HOOK_SPECS_V1: &[PinnedProductionHookSpecV1] = &['
    $countDeclaration =
        'pub const PINNED_PRODUCTION_HOOK_COUNT_V1: u16 = PINNED_PRODUCTION_HOOK_SPECS_V1.len() as u16;'
    if (-not $passiveReady.Contains($rosterDeclaration) -or
        -not $passiveReady.Contains($countDeclaration) -or
        -not $passiveReady.Contains(
            '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
        )) {
        throw 'passive-ready source lacks the exact-a729 pinned hook contract'
    }
    $rosterStart = $passiveReady.IndexOf($rosterDeclaration, [StringComparison]::Ordinal)
    $rosterBodyStart = $rosterStart + $rosterDeclaration.Length
    $rosterEnd = $passiveReady.IndexOf('];', $rosterBodyStart, [StringComparison]::Ordinal)
    if ($rosterEnd -le $rosterBodyStart) {
        throw 'passive-ready source has no bounded pinned production hook roster'
    }
    $rosterBody = $passiveReady.Substring($rosterBodyStart, $rosterEnd - $rosterBodyStart)
    $pinnedHookCount = [regex]::Matches(
        $rosterBody,
        '(?m)^\s*pinned_(?:import|rva)!\('
    ).Count
    if ($pinnedHookCount -le 0 -or $pinnedHookCount -gt [uint16]::MaxValue) {
        throw 'passive-ready source has an invalid mechanically counted pinned hook roster'
    }
    $pinnedHookCount = [uint16]$pinnedHookCount
    $logicalNames = @(
        [regex]::Matches(
            $rosterBody,
            '(?m)^\s*pinned_(?:import|rva)!\(\s*"([^"]+)"'
        ) | ForEach-Object { $_.Groups[1].Value }
    )
    if ($logicalNames.Count -ne $pinnedHookCount -or
        @($logicalNames | Select-Object -Unique).Count -ne $pinnedHookCount) {
        throw 'passive-ready source pinned hook names are incomplete or duplicated'
    }
    foreach ($bootstrap in @(
        'bootstrap.d3d11_create_device',
        'bootstrap.d3d_compile'
    )) {
        if ($logicalNames -cnotcontains $bootstrap) {
            throw "passive-ready source omits required bootstrap hook '$bootstrap'"
        }
    }
    $forbiddenHookNames = @(
        $logicalNames | Where-Object {
            $_ -cmatch '(?:^|\.)a(?:25|28|30)(?:\.|$)' -or
            $_ -cmatch '(?:^|\.)render(?:\.|$)' -or
            $_ -cmatch '(?:^|\.)evidence(?:\.|$)' -or
            ($_ -cnotin @(
                'bootstrap.d3d11_create_device',
                'bootstrap.d3d_compile'
            ) -and
                $_ -cnotmatch '^presentation\.d82\.[a-z0-9_.-]+$' -and
                $_ -cnotmatch '^gameplay\.d[0-9]+\.[a-z0-9_.-]+$')
        }
    )
    if ($forbiddenHookNames.Count -ne 0) {
        throw "passive-ready source contains forbidden evidence/render hook capability: $($forbiddenHookNames -join ', ')"
    }
    return [ordered]@{
        schema = 'hoi4.production-source-capability-closure.v1'
        canonical_observer_feature_isolated = $true
        target_lifecycle_actuators_evidence_only = $true
        production_auto_attach_target_control_absent = $true
        collector_fixture_hooks_default_off = $true
        collector_cdylib_explicit_empty_features = $true
        structural_dependency_feature_parse = $true
        production_source_tree_closed = $true
        passive_source_admission = $passiveSourceAdmission
        canonical_hook_name_families_closed = $true
        exact_pinned_hook_count = [int]$pinnedHookCount
        pinned_hook_contract_source_sha256 =
            [string]$expectedPolicyInputs.passive_ready_contract_sha256
        exact_target_executable_sha256 =
            '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
    }
}

$receiptBytes = Read-LockedBytes $receiptFull
$receipt = [Text.Encoding]::UTF8.GetString($receiptBytes) | ConvertFrom-Json
if ([string]$receipt.schema -cne 'battle-plan.central_portable_package.v1' -or
    [string]$receipt.status -cne 'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE' -or
    $receipt.successful_exit -isnot [bool] -or -not $receipt.successful_exit -or
    [string]$receipt.central_validation.mode -cne 'Release' -or
    [string]$receipt.central_validation.requested_workspace -cne 'All') {
    throw 'portable package receipt has no explicit ReleaseManual authority'
}
if ([string]$receipt.central_validation.release_runtime_linkage.target -cne 'x86_64-pc-windows-msvc' -or
    [string]$receipt.central_validation.release_runtime_linkage.rust_target_feature -cne '+crt-static' -or
    [string]$receipt.central_validation.release_runtime_linkage.c_runtime -cne 'static' -or
    [string]$receipt.central_validation.release_runtime_linkage.admission -cne 'pe_import_and_delay_import_tables' -or
    [string]$receipt.central_validation.release_runtime_linkage.forbidden_import -cne 'VCRUNTIME*.dll') {
    throw 'portable package receipt has no static MSVC CRT release contract'
}
$featureClosure = $receipt.central_validation.production_feature_closure
if ([string]$featureClosure.schema -cne 'battle-plan.production_feature_closure.v1' -or
    (@($featureClosure.rebuild_order) -join '|') -cne 'recorder_host|collector_dll' -or
    [string]$featureClosure.recorder_host.package -cne 'recorder_host' -or
    [string]$featureClosure.recorder_host.target -cne 'collector_record' -or
    $featureClosure.recorder_host.default_features -isnot [bool] -or
    $featureClosure.recorder_host.default_features -or
    @($featureClosure.recorder_host.enabled_features).Count -ne 0 -or
    [string]$featureClosure.collector_dll.package -cne 'collector' -or
    [string]$featureClosure.collector_dll.target -cne 'lib' -or
    (@($featureClosure.collector_dll.crate_types) -join '|') -cne 'cdylib|rlib' -or
    $featureClosure.collector_dll.default_features -isnot [bool] -or
    $featureClosure.collector_dll.default_features -or
    @($featureClosure.collector_dll.enabled_features).Count -ne 0 -or
    $featureClosure.collector_dll.all_non_default_features_disabled -isnot [bool] -or
    -not $featureClosure.collector_dll.all_non_default_features_disabled -or
    (@($featureClosure.collector_dll.forbidden_features) -join '|') -cne
        'test-support' -or
    $featureClosure.collector_dll.built_last -isnot [bool] -or
    -not $featureClosure.collector_dll.built_last -or
    [string]$featureClosure.collector_dll.collector_cargo_sha256 -cne
        [string]$receipt.package.interaction_closure.policy_inputs.collector_cargo_sha256 -or
    [int]$featureClosure.collector_dll.pinned_hook_count -ne
        [int]$receipt.package.interaction_closure.source_capability.exact_pinned_hook_count -or
    [string]$featureClosure.collector_dll.pinned_hook_contract_source_sha256 -cne
        [string]$receipt.package.interaction_closure.policy_inputs.passive_ready_contract_sha256) {
    throw 'portable package receipt has no explicit canonical production feature closure'
}
if ([IO.Path]::GetFullPath([string]$receipt.package_builder.path) -ine [IO.Path]::GetFullPath($builder) -or
    [string]$receipt.package_builder.sha256 -cne (Get-FileHash -LiteralPath $builder -Algorithm SHA256).Hash.ToLowerInvariant()) {
    throw 'portable package builder differs from its receipt'
}
if ([IO.Path]::GetFullPath([string]$receipt.artifact_attestation.path) -ine [IO.Path]::GetFullPath($expectedAttestation)) {
    throw 'portable package receipt does not bind the canonical production artifact attestation'
}
$attested = Test-ProductionArtifactAttestation -RepoRoot $repo -ReceiptPath ([string]$receipt.artifact_attestation.path) -ExpectedDriverPath $driver
Test-ProductionBuildEnvironmentRecord -Record $receipt.build_environment
if ([string]$receipt.artifact_attestation.sha256 -cne [string]$attested.receipt_sha256 -or
    [string]$receipt.source_input_sha256 -cne [string]$attested.source_input_sha256 -or
    [string]$receipt.build_environment.sha256 -cne [string]$attested.build_environment.sha256 -or
    [string]$featureClosure.recorder_host.artifact_sha256 -cne
        [string]$attested.artifact_sha256['recorder_host'] -or
    [string]$featureClosure.collector_dll.artifact_sha256 -cne
        [string]$attested.artifact_sha256['collector_dll']) {
    throw 'portable package receipt is stale against its production artifact attestation'
}
$productionCaptureEvidence = Get-ProductionCapturePromotionEvidence `
    -RepoRoot $repo `
    -CaptureReceiptPath ([string]$receipt.package.production_capture_evidence.capture_receipt_path) `
    -CurrentArtifactAttestationPath ([string]$attested.receipt_path)
if ($receipt.package.admissible_for_production -isnot [bool] -or
    -not $receipt.package.admissible_for_production -or
    $null -eq $receipt.package.production_capture_evidence -or
    (($receipt.package.production_capture_evidence | ConvertTo-Json -Depth 12 -Compress) -cne
        ($productionCaptureEvidence | ConvertTo-Json -Depth 12 -Compress))) {
    throw 'portable package lacks exact current all-domain zero-loss speed-five promotion evidence'
}
$version = [string]$receipt.package.version
if ($version -notmatch '^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$') { throw 'portable package version is invalid' }
$expectedZip = Join-Path $repo ('.cargo-target\release\battle-plan-{0}-windows-x86_64.zip' -f $version)
if ([string]$receipt.package.schema -cne 'hoi4-portable-package.v1' -or
    [IO.Path]::GetFullPath([string]$receipt.package.path) -ine [IO.Path]::GetFullPath($expectedZip) -or
    [int]$receipt.package.executable_count -ne 2 -or
    $receipt.package.static_vcruntime_admission -isnot [bool] -or
    -not $receipt.package.static_vcruntime_admission -or
    $receipt.package.signed -isnot [bool] -or $receipt.package.signed -or
    [string]$receipt.package.install_kind -cne 'portable_zip') {
    throw 'portable package identity differs from its central receipt'
}
if ([string]$receipt.package.interaction_closure.policy -cne 'passive_injected_observer' -or
    $receipt.package.interaction_closure.zip_entry_bytes_reverified -isnot [bool] -or
    -not $receipt.package.interaction_closure.zip_entry_bytes_reverified) {
    throw 'portable package receipt has no passive interaction-closure authority'
}
$expectedPolicyInputs = [ordered]@{
    read_only_game_state_witness_sha256 = (Get-FileHash -LiteralPath $readOnlyWitness -Algorithm SHA256).Hash.ToLowerInvariant()
    passive_capture_policy_test_sha256 = (Get-FileHash -LiteralPath $passivePolicy -Algorithm SHA256).Hash.ToLowerInvariant()
    passive_capture_lifecycle_sha256 = (Get-FileHash -LiteralPath $passiveLifecycle -Algorithm SHA256).Hash.ToLowerInvariant()
}
foreach ($entry in $productionSourceCapabilityPaths.GetEnumerator()) {
    if (-not (Test-Path -LiteralPath $entry.Value -PathType Leaf)) {
        throw "portable package interaction-policy source is missing: $($entry.Value)"
    }
    $expectedPolicyInputs[$entry.Key] =
        (Get-FileHash -LiteralPath $entry.Value -Algorithm SHA256).Hash.ToLowerInvariant()
}
$sourceTreeAuthority = Get-ProductionInteractionSourceTreeAuthority -RepoRoot $repo
$expectedPolicyInputs['production_interaction_source_tree_file_count'] =
    [int]$sourceTreeAuthority.file_count
$expectedPolicyInputs['production_interaction_source_tree_sha256'] =
    [string]$sourceTreeAuthority.sha256
if (($receipt.package.interaction_closure.policy_inputs | ConvertTo-Json -Compress) -cne
    ($expectedPolicyInputs | ConvertTo-Json -Compress)) {
    throw 'portable package receipt interaction-policy inputs differ from current digest-visible sources'
}
$sourceCapabilityClosure = Get-VerifiedProductionSourceCapabilityClosure
if (-not $sourceCapabilityClosure.passive_source_admission.no_statically_proven_impossible_production_sources) {
    throw 'portable package source closure contains a production domain with no admitted passive source'
}
if (($receipt.package.interaction_closure.source_capability | ConvertTo-Json -Compress) -cne
    ($sourceCapabilityClosure | ConvertTo-Json -Compress)) {
    throw 'portable package receipt source-capability closure differs from current production sources'
}
$zipBytes = Read-LockedBytes $expectedZip
if ((Get-ByteHash $zipBytes) -cne [string]$receipt.package.sha256) { throw 'portable package bytes differ from its receipt' }
$memory = [IO.MemoryStream]::new($zipBytes, $false)
try {
    $archive = [IO.Compression.ZipArchive]::new($memory, [IO.Compression.ZipArchiveMode]::Read, $false)
    try {
        $expectedNames = @('Battle Plan Recorder.exe', 'Battle Plan.exe', 'collector.dll', 'README.txt', 'SHA256SUMS.txt')
        if ($archive.Entries.Count -ne 5) { throw 'portable package does not contain exactly five entries' }
        foreach ($name in $expectedNames) {
            if (@($archive.Entries | Where-Object FullName -CEQ $name).Count -ne 1) { throw "portable package entry is missing or duplicated: $name" }
        }
        $roles = [ordered]@{
            'Battle Plan Recorder.exe' = 'recorder_host'
            'Battle Plan.exe' = 'replay_app'
            'collector.dll' = 'collector_dll'
        }
        $interactionClosures = [ordered]@{}
        foreach ($pair in $roles.GetEnumerator()) {
            $entry = @($archive.Entries | Where-Object FullName -CEQ $pair.Key)[0]
            $entryStream = $entry.Open()
            $entryMemory = $null
            try { $entryMemory = [IO.MemoryStream]::new(); $entryStream.CopyTo($entryMemory); $entryBytes = $entryMemory.ToArray() }
            finally { if ($null -ne $entryMemory) { $entryMemory.Dispose() }; $entryStream.Dispose() }
            $hash = Get-ByteHash $entryBytes
            if ($hash -cne [string]$attested.artifact_sha256[$pair.Value] -or
                $hash -cne [string]$receipt.package.input_sha256.($pair.Value)) {
                throw "portable package entry is not bound to its attested artifact: $($pair.Key)"
            }
            [void](Assert-PortableProductPeImportAttestation `
                -Bytes $entryBytes `
                -Name ([string]$pair.Key) `
                -Attestation $receipt.package.pe_imports.($pair.Value))
            [void](Assert-PortableProductPeImportAttestation `
                -Bytes $entryBytes `
                -Name ([string]$pair.Key) `
                -Attestation $attested.artifact_pe_imports[$pair.Value])
            $requiredMarkers = switch ([string]$pair.Value) {
                'recorder_host' {
                    @(
                        'BATTLE_PLAN_OBSERVER_ONLY_RECORDER_V1',
                        'BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1'
                    )
                }
                'replay_app' { @() }
                'collector_dll' { @('BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1') }
                default { throw "unexpected interaction-closure role: $($pair.Value)" }
            }
            $permittedInjectionCapabilities = if ([string]$pair.Value -ceq 'recorder_host') {
                @(
                    'OpenProcess',
                    'VirtualAllocEx',
                    'VirtualFreeEx',
                    'WriteProcessMemory',
                    'CreateRemoteThread',
                    'LoadLibraryW',
                    'FreeLibrary'
                )
            }
            else { @() }
            $permittedSelfUiCapabilities = if ([string]$pair.Value -ceq 'recorder_host') {
                @(
                    'SendMessageW',
                    'ShellExecuteW',
                    'EnableWindow',
                    'SetWindowTextW',
                    'ShowWindow',
                    'DestroyWindow'
                )
            }
            elseif ([string]$pair.Value -ceq 'replay_app') {
                @('SetWindowTextW', 'DestroyWindow')
            }
            else { @() }
            $permittedSelfWorkerCapabilities = if ([string]$pair.Value -ceq 'replay_app') {
                @(
                    'CreateProcessW',
                    'TerminateProcess',
                    'CreateJobObjectW',
                    'AssignProcessToJobObject',
                    'SetInformationJobObject'
                )
            }
            else { @() }
            $verifiedClosure = Assert-VerifiedInteractionClosure `
                -Bytes $entryBytes `
                -Name ([string]$pair.Key) `
                -RequiredMarkers @($requiredMarkers) `
                -ExpectedPinnedHookCount ([uint16]$sourceCapabilityClosure.exact_pinned_hook_count) `
                -PinnedHookContractSourceSha256 ([string]$sourceCapabilityClosure.pinned_hook_contract_source_sha256) `
                -PermittedSelfUiCapabilities @($permittedSelfUiCapabilities) `
                -PermittedSelfWorkerCapabilities @($permittedSelfWorkerCapabilities) `
                -PermittedInjectionCapabilities @($permittedInjectionCapabilities)
            $receiptClosure = $receipt.package.interaction_closure.($pair.Value)
            if (($verifiedClosure | ConvertTo-Json -Depth 5 -Compress) -cne
                ($receiptClosure | ConvertTo-Json -Depth 5 -Compress)) {
                throw "portable package interaction closure differs from ZIP entry bytes: $($pair.Key)"
            }
            $interactionClosures[$pair.Value] = $verifiedClosure
        }
    }
    finally { $archive.Dispose() }
}
finally { $memory.Dispose() }

[pscustomobject]@{
    schema = 'battle-plan.central_portable_package_verification.v1'
    status = if ($AllowStagedReceipt) { 'PASS_STAGED_CENTRAL_RELEASE_PORTABLE_PACKAGE' } else { 'PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE' }
    canonical_receipt = -not [bool]$AllowStagedReceipt
    receipt_sha256 = Get-ByteHash $receiptBytes
    package_sha256 = [string]$receipt.package.sha256
    build_environment_sha256 = [string]$receipt.build_environment.sha256
    static_vcruntime_admission = $true
    production_capture_evidence = $productionCaptureEvidence
    interaction_closure = [ordered]@{
        policy = 'passive_injected_observer'
        zip_entry_bytes_reverified = $true
        policy_inputs = $expectedPolicyInputs
        source_capability = $sourceCapabilityClosure
        recorder_host = $interactionClosures['recorder_host']
        replay_app = $interactionClosures['replay_app']
        collector_dll = $interactionClosures['collector_dll']
    }
} | ConvertTo-Json -Depth 8
