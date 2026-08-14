[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$RecorderExe,

    [Parameter(Mandatory = $true)]
    [string]$ReplayExe,

    [Parameter(Mandatory = $true)]
    [string]$CollectorDll,

    [Parameter(Mandatory = $true)]
    [string]$OutputZip,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$')]
    [string]$Version,

    [string]$ProductionCaptureReceiptPath = '',

    [string]$ProductionArtifactAttestationPath = '',

    # Deterministic fail-closed injection for the central source self-test. It
    # can only turn a valid build into a failure and exists to prove that the
    # post-creation finally block removes an unpublished temporary archive.
    [switch]$TestOnlyFailAfterTemporaryWrite,

    # Deterministic fail-closed injection for proving that an archive already
    # published into the final namespace is removed when the terminal
    # re-verification phase fails.
    [switch]$TestOnlyFailAfterPublishedWrite
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot '..\..\..\portable_replay\native\scripts\PortableProduct.PeImports.ps1')
. (Join-Path $PSScriptRoot 'PortableProduct.InteractionClosure.ps1')
. (Join-Path $PSScriptRoot 'ProductionCapture.Promotion.ps1')
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$readOnlyWitnessPath = Join-Path $PSScriptRoot 'Read-Hoi4GameState.ps1'
$passivePolicyPath = Join-Path $PSScriptRoot 'Test-PassiveCapturePolicy.ps1'
$passiveLifecyclePath = Join-Path $PSScriptRoot 'PassiveCapture.Lifecycle.ps1'
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
    interaction_closure_helper_sha256 = Join-Path $PSScriptRoot 'PortableProduct.InteractionClosure.ps1'
    production_capture_promotion_helper_sha256 = Join-Path $PSScriptRoot 'ProductionCapture.Promotion.ps1'
}
foreach ($policyInput in @(
    $readOnlyWitnessPath,
    $passivePolicyPath,
    $passiveLifecyclePath
) + @($productionSourceCapabilityPaths.Values)) {
    if (-not (Test-Path -LiteralPath $policyInput -PathType Leaf)) {
        throw "portable package interaction-policy input is missing: $policyInput"
    }
}

function Get-InteractionPolicyInputs {
    $inputs = [ordered]@{
        read_only_game_state_witness_sha256 = (Get-FileHash -LiteralPath $readOnlyWitnessPath -Algorithm SHA256).Hash.ToLowerInvariant()
        passive_capture_policy_test_sha256 = (Get-FileHash -LiteralPath $passivePolicyPath -Algorithm SHA256).Hash.ToLowerInvariant()
        passive_capture_lifecycle_sha256 = (Get-FileHash -LiteralPath $passiveLifecyclePath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    foreach ($entry in $productionSourceCapabilityPaths.GetEnumerator()) {
        $inputs[$entry.Key] = (Get-FileHash -LiteralPath $entry.Value -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $sourceTree = Get-ProductionInteractionSourceTreeAuthority -RepoRoot $repoRoot
    $inputs['production_interaction_source_tree_file_count'] = [int]$sourceTree.file_count
    $inputs['production_interaction_source_tree_sha256'] = [string]$sourceTree.sha256
    return $inputs
}
$interactionPolicyInputs = Get-InteractionPolicyInputs
if ([string]::IsNullOrWhiteSpace($ProductionCaptureReceiptPath) -ne
    [string]::IsNullOrWhiteSpace($ProductionArtifactAttestationPath)) {
    throw 'production capture receipt and artifact attestation must be supplied together'
}
$productionCaptureEvidence = if ([string]::IsNullOrWhiteSpace($ProductionCaptureReceiptPath)) {
    $null
}
else {
    Get-ProductionCapturePromotionEvidence `
        -RepoRoot $repoRoot `
        -CaptureReceiptPath $ProductionCaptureReceiptPath `
        -CurrentArtifactAttestationPath $ProductionArtifactAttestationPath
}

function Get-ProductionSourceCapabilityClosure {
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
    $sourceTreeAuthority = Get-ProductionInteractionSourceTreeAuthority -RepoRoot $repoRoot
    if ([int]$interactionPolicyInputs.production_interaction_source_tree_file_count -ne
            [int]$sourceTreeAuthority.file_count -or
        [string]$interactionPolicyInputs.production_interaction_source_tree_sha256 -cne
            [string]$sourceTreeAuthority.sha256) {
        throw 'production interaction source tree changed during source closure'
    }
    Assert-ProductionInteractionSourceTreeClosure `
        -RepoRoot $repoRoot `
        -Authority $sourceTreeAuthority
    $passiveSourceAdmission =
        Get-ProductionPassiveSourceAdmissionClosure -RepoRoot $repoRoot

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
    # This used to REQUIRE the collector manifest to forward both lane fixture
    # features, because what it could prove then was that the fixture hooks were
    # CONDITIONAL. The hooks are deleted, so absence is asserted instead.
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
            [string]$interactionPolicyInputs.passive_ready_contract_sha256
        exact_target_executable_sha256 =
            '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
    }
}
$sourceCapabilityClosure = Get-ProductionSourceCapabilityClosure
if ($null -ne $productionCaptureEvidence -and
    -not $sourceCapabilityClosure.passive_source_admission.no_statically_proven_impossible_production_sources) {
    $blocked = @(
        $sourceCapabilityClosure.passive_source_admission.statically_impossible_domains |
            ForEach-Object { [string]$_.id }
    ) -join ','
    throw "portable production promotion has statically impossible passive sources: $blocked"
}
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

public static class PortablePackageNative {
    public const uint FILE_ATTRIBUTE_DIRECTORY = 0x10;
    public const uint FILE_ATTRIBUTE_REPARSE_POINT = 0x400;
    private const uint GENERIC_READ = 0x80000000;
    private const uint FILE_SHARE_READ = 0x1;
    private const uint OPEN_EXISTING = 3;
    private const uint FILE_FLAG_OPEN_REPARSE_POINT = 0x00200000;
    private const uint FILE_FLAG_SEQUENTIAL_SCAN = 0x08000000;
    private const uint MOVEFILE_WRITE_THROUGH = 0x00000008;

    [StructLayout(LayoutKind.Sequential)]
    public struct FILETIME { public uint Low; public uint High; }

    [StructLayout(LayoutKind.Sequential)]
    public struct BY_HANDLE_FILE_INFORMATION {
        public uint FileAttributes;
        public FILETIME CreationTime;
        public FILETIME LastAccessTime;
        public FILETIME LastWriteTime;
        public uint VolumeSerialNumber;
        public uint FileSizeHigh;
        public uint FileSizeLow;
        public uint NumberOfLinks;
        public uint FileIndexHigh;
        public uint FileIndexLow;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern SafeFileHandle CreateFileW(
        string name, uint access, uint share, IntPtr security, uint creation,
        uint flags, IntPtr template);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool GetFileInformationByHandle(
        SafeFileHandle handle, out BY_HANDLE_FILE_INFORMATION information);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool MoveFileExW(string existingName, string newName, uint flags);

    public static void PublishNoReplaceWriteThrough(string temporary, string destination) {
        if (File.Exists(destination) || Directory.Exists(destination)) {
            throw new IOException("durable publication refuses an existing destination: " + destination);
        }
        // Omitting MOVEFILE_REPLACE_EXISTING preserves atomic no-replace
        // publication. WRITE_THROUGH makes the namespace/parent-directory
        // update durable before success is returned.
        if (!MoveFileExW(temporary, destination, MOVEFILE_WRITE_THROUGH)) {
            throw new Win32Exception(Marshal.GetLastWin32Error(),
                "durable no-replace package publication failed: " + destination);
        }
    }

    public static SafeFileHandle OpenReadNoFollow(
        string path, out BY_HANDLE_FILE_INFORMATION information) {
        SafeFileHandle handle = CreateFileW(
            path, GENERIC_READ, FILE_SHARE_READ, IntPtr.Zero, OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_SEQUENTIAL_SCAN, IntPtr.Zero);
        if (handle.IsInvalid) {
            throw new Win32Exception(Marshal.GetLastWin32Error(),
                "open package artifact without reparse traversal failed: " + path);
        }
        if (!GetFileInformationByHandle(handle, out information)) {
            int error = Marshal.GetLastWin32Error();
            handle.Dispose();
            throw new Win32Exception(error, "inspect opened package artifact failed: " + path);
        }
        if ((information.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0 ||
            (information.FileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0) {
            handle.Dispose();
            throw new InvalidDataException("package artifact is a directory or reparse point: " + path);
        }
        return handle;
    }

    public static bool BytesEqual(byte[] left, byte[] right) {
        if (Object.ReferenceEquals(left, right)) return true;
        if (left == null || right == null || left.Length != right.Length) return false;
        int difference = 0;
        for (int i = 0; i < left.Length; ++i) difference |= left[i] ^ right[i];
        return difference == 0;
    }
}
'@

function Resolve-ArtifactPath {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $full = [IO.Path]::GetFullPath($Path)
    if ([IO.Path]::GetFileName($full)) {
        return $full
    }
    throw "$Label must name a file: $Path"
}

function Assert-PeKind {
    param(
        [Parameter(Mandatory = $true)][byte[]]$Bytes,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][bool]$ExpectDll,
        [Parameter(Mandatory = $true)][bool]$ExpectGui
    )

    $stream = [IO.MemoryStream]::new($Bytes, $false)
    try {
        $reader = [IO.BinaryReader]::new($stream)
        if ($reader.ReadUInt16() -ne 0x5A4D) {
            throw "$Name is not a PE image"
        }
        $stream.Position = 0x3c
        $peOffset = $reader.ReadUInt32()
        if ($peOffset -gt ($stream.Length - 24)) {
            throw "$Name has an invalid PE header offset"
        }
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) {
            throw "$Name has no PE signature"
        }
        $machine = $reader.ReadUInt16()
        if ($machine -ne 0x8664) {
            throw "$Name is not an AMD64 PE image"
        }
        [void]$reader.ReadUInt16()
        [void]$reader.ReadUInt32()
        [void]$reader.ReadUInt32()
        [void]$reader.ReadUInt32()
        $optionalHeaderSize = $reader.ReadUInt16()
        $characteristics = $reader.ReadUInt16()
        if (($characteristics -band 0x0002) -eq 0) {
            throw "$Name is not marked as an executable image"
        }
        $isDll = ($characteristics -band 0x2000) -ne 0
        if ($isDll -ne $ExpectDll) {
            $wanted = if ($ExpectDll) { 'DLL' } else { 'executable' }
            throw "$Name is not a PE $wanted"
        }
        $optionalHeaderOffset = [uint64]$peOffset + 24
        if ($optionalHeaderSize -lt 2 -or
            $optionalHeaderOffset + $optionalHeaderSize -gt [uint64]$stream.Length) {
            throw "$Name has a truncated PE optional header"
        }
        $stream.Position = [int64]$optionalHeaderOffset
        if ($reader.ReadUInt16() -ne 0x020B) {
            throw "$Name is not a PE32+ image"
        }
        if ($ExpectGui) {
            if ($optionalHeaderSize -lt 70) {
                throw "$Name has no complete PE optional header"
            }
            $subsystemOffset = $optionalHeaderOffset + 68
            $stream.Position = [int64]$subsystemOffset
            if ($reader.ReadUInt16() -ne 2) {
                throw "$Name is not linked as a Windows GUI-subsystem executable"
            }
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Read-LockedBytes {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Label
    )

    # Open the pathname itself (not its target) and reject a reparse handle.
    # Share-read only denies write/delete until the authoritative bytes and
    # handle identity/type have been retained.
    $information = [PortablePackageNative+BY_HANDLE_FILE_INFORMATION]::new()
    $handle = [PortablePackageNative]::OpenReadNoFollow($Path, [ref]$information)
    $stream = [IO.FileStream]::new($handle, [IO.FileAccess]::Read)
    try {
        if ($stream.Length -le 0) {
            throw "$Label is empty: $Path"
        }
        if ($stream.Length -gt [int]::MaxValue) {
            throw "$Label exceeds the supported portable-package artifact size"
        }
        $bytes = [byte[]]::new([int]$stream.Length)
        $offset = 0
        while ($offset -lt $bytes.Length) {
            $read = $stream.Read($bytes, $offset, $bytes.Length - $offset)
            if ($read -eq 0) {
                throw "$Label ended before its retained handle length"
            }
            $offset += $read
        }
        if ($stream.ReadByte() -ne -1) {
            throw "$Label grew during its locked read"
        }
        return ,$bytes
    }
    finally {
        $stream.Dispose()
    }
}

function Add-BytesEntry {
    param(
        [Parameter(Mandatory = $true)][IO.Compression.ZipArchive]$Archive,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][byte[]]$Bytes
    )

    $entry = $Archive.CreateEntry($Name, [IO.Compression.CompressionLevel]::Optimal)
    $entry.LastWriteTime = [DateTimeOffset]::new(1980, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
    $entryStream = $entry.Open()
    try {
        $entryStream.Write($Bytes, 0, $Bytes.Length)
    }
    finally {
        $entryStream.Dispose()
    }
}

function Get-BytesSha256 {
    param([Parameter(Mandatory = $true)][byte[]]$Bytes)

    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha256.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $sha256.Dispose()
    }
}

function Find-AsciiByteSequence {
    param(
        [Parameter(Mandatory = $true)][byte[]]$Bytes,
        [Parameter(Mandatory = $true)][string]$Text
    )

    $needle = [Text.Encoding]::ASCII.GetBytes($Text)
    if ($needle.Length -eq 0 -or $Bytes.Length -lt $needle.Length) {
        return $false
    }
    for ($offset = 0; $offset -le $Bytes.Length - $needle.Length; $offset++) {
        $matches = $true
        for ($index = 0; $index -lt $needle.Length; $index++) {
            if ($Bytes[$offset + $index] -ne $needle[$index]) {
                $matches = $false
                break
            }
        }
        if ($matches) {
            return $true
        }
    }
    return $false
}

function Assert-ProductionInteractionClosure {
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

function Read-ZipEntryBytes {
    param([Parameter(Mandatory = $true)][IO.Compression.ZipArchiveEntry]$Entry)

    if ($Entry.Length -gt [int]::MaxValue) {
        throw "ZIP entry exceeds supported verification size: $($Entry.FullName)"
    }
    $stream = $Entry.Open()
    try {
        $memory = [IO.MemoryStream]::new([int]$Entry.Length)
        try {
            $stream.CopyTo($memory)
            return ,$memory.ToArray()
        }
        finally {
            $memory.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Assert-PackageZipBytes {
    param(
        [Parameter(Mandatory = $true)][byte[]]$ZipBytes,
        [Parameter(Mandatory = $true)]$ExpectedEntries,
        [Parameter(Mandatory = $true)]$ExpectedBinaryHashes,
        [Parameter(Mandatory = $true)]$ExpectedImportAttestations,
        [Parameter(Mandatory = $true)]$ExpectedInteractionClosures
    )

    $memory = [IO.MemoryStream]::new($ZipBytes, $false)
    try {
        $archive = [IO.Compression.ZipArchive]::new($memory, [IO.Compression.ZipArchiveMode]::Read, $false)
        try {
            $names = @($archive.Entries | ForEach-Object FullName)
            $exeCount = @($names | Where-Object { $_.EndsWith('.exe', [StringComparison]::OrdinalIgnoreCase) }).Count
            if ($exeCount -ne 2 -or $names.Count -ne 5 -or $names -contains '') {
                throw "portable package shape is invalid: entries=$($names.Count), executables=$exeCount"
            }
            foreach ($expected in @('Battle Plan Recorder.exe', 'Battle Plan.exe', 'collector.dll', 'README.txt', 'SHA256SUMS.txt')) {
                $matching = @($archive.Entries | Where-Object FullName -CEQ $expected)
                if ($matching.Count -ne 1) {
                    throw "portable package requires exactly one $expected entry"
                }
                $expectedBytes = [byte[]]$ExpectedEntries[$expected]
                $entry = $matching[0]
                if ($entry.Length -ne $expectedBytes.Length) {
                    throw "portable package entry length mismatch for $expected"
                }
                $actualBytes = Read-ZipEntryBytes -Entry $entry
                if (-not [PortablePackageNative]::BytesEqual($actualBytes, $expectedBytes)) {
                    throw "portable package entry bytes mismatch for $expected"
                }
                $actualHash = Get-BytesSha256 -Bytes $actualBytes
                $expectedHash = Get-BytesSha256 -Bytes $expectedBytes
                if ($actualHash -cne $expectedHash) {
                    throw "portable package entry hash mismatch for $expected"
                }
                if ($ExpectedBinaryHashes.Contains($expected) -and $actualHash -cne $ExpectedBinaryHashes[$expected]) {
                    throw "portable package entry disagrees with SHA256SUMS.txt for $expected"
                }
                if ($ExpectedImportAttestations.Contains($expected)) {
                    [void](Assert-PortableProductPeImportAttestation `
                        -Bytes $actualBytes `
                        -Name $expected `
                        -Attestation $ExpectedImportAttestations[$expected])
                }
                if ($ExpectedInteractionClosures.Contains($expected)) {
                    $closure = Assert-ProductionInteractionClosure `
                        -Bytes $actualBytes `
                        -Name $expected `
                        -RequiredMarkers @($ExpectedInteractionClosures[$expected].required_markers) `
                        -ExpectedPinnedHookCount ([uint16]$ExpectedInteractionClosures[$expected].exact_pinned_hook_count) `
                        -PinnedHookContractSourceSha256 ([string]$ExpectedInteractionClosures[$expected].pinned_hook_contract_source_sha256) `
                        -PermittedSelfUiCapabilities @($ExpectedInteractionClosures[$expected].permitted_self_ui_capabilities) `
                        -PermittedSelfWorkerCapabilities @($ExpectedInteractionClosures[$expected].permitted_self_worker_capabilities) `
                        -PermittedInjectionCapabilities @($ExpectedInteractionClosures[$expected].permitted_injection_capabilities)
                    if (($closure | ConvertTo-Json -Depth 5 -Compress) -cne
                        ($ExpectedInteractionClosures[$expected] | ConvertTo-Json -Depth 5 -Compress)) {
                        throw "portable package interaction closure changed for $expected"
                    }
                }
            }
        }
        finally {
            $archive.Dispose()
        }
    }
    finally {
        $memory.Dispose()
    }
}

$recorder = Resolve-ArtifactPath -Path $RecorderExe -Label 'Recorder executable'
$replay = Resolve-ArtifactPath -Path $ReplayExe -Label 'Replay executable'
$collector = Resolve-ArtifactPath -Path $CollectorDll -Label 'Collector DLL'
$recorderBytes = Read-LockedBytes -Path $recorder -Label 'Recorder executable'
$replayBytes = Read-LockedBytes -Path $replay -Label 'Replay executable'
$collectorBytes = Read-LockedBytes -Path $collector -Label 'Collector DLL'
Assert-PeKind -Bytes $recorderBytes -Name ([IO.Path]::GetFileName($recorder)) -ExpectDll $false -ExpectGui $true
Assert-PeKind -Bytes $replayBytes -Name ([IO.Path]::GetFileName($replay)) -ExpectDll $false -ExpectGui $true
Assert-PeKind -Bytes $collectorBytes -Name ([IO.Path]::GetFileName($collector)) -ExpectDll $true -ExpectGui $false
$binaryImportAttestations = [ordered]@{
    'Battle Plan Recorder.exe' = Assert-PortableProductStaticCrt -Bytes $recorderBytes -Name 'Battle Plan Recorder.exe'
    'Battle Plan.exe' = Assert-PortableProductStaticCrt -Bytes $replayBytes -Name 'Battle Plan.exe'
    'collector.dll' = Assert-PortableProductStaticCrt -Bytes $collectorBytes -Name 'collector.dll'
}
$binaryInteractionClosures = [ordered]@{
    'Battle Plan Recorder.exe' = Assert-ProductionInteractionClosure `
        -Bytes $recorderBytes `
        -Name 'Battle Plan Recorder.exe' `
        -RequiredMarkers @(
            'BATTLE_PLAN_OBSERVER_ONLY_RECORDER_V1',
            'BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1'
        ) `
        -ExpectedPinnedHookCount ([uint16]$sourceCapabilityClosure.exact_pinned_hook_count) `
        -PinnedHookContractSourceSha256 ([string]$sourceCapabilityClosure.pinned_hook_contract_source_sha256) `
        -PermittedSelfUiCapabilities @(
            'SendMessageW',
            'ShellExecuteW',
            'EnableWindow',
            'SetWindowTextW',
            'ShowWindow',
            'DestroyWindow'
        ) `
        -PermittedInjectionCapabilities @(
            'OpenProcess',
            'VirtualAllocEx',
            'VirtualFreeEx',
            'WriteProcessMemory',
            'CreateRemoteThread',
            'LoadLibraryW',
            'FreeLibrary'
        )
    'Battle Plan.exe' = Assert-ProductionInteractionClosure `
        -Bytes $replayBytes `
        -Name 'Battle Plan.exe' `
        -RequiredMarkers @() `
        -ExpectedPinnedHookCount ([uint16]$sourceCapabilityClosure.exact_pinned_hook_count) `
        -PinnedHookContractSourceSha256 ([string]$sourceCapabilityClosure.pinned_hook_contract_source_sha256) `
        -PermittedSelfUiCapabilities @('SetWindowTextW', 'DestroyWindow') `
        -PermittedSelfWorkerCapabilities @(
            'CreateProcessW',
            'TerminateProcess',
            'CreateJobObjectW',
            'AssignProcessToJobObject',
            'SetInformationJobObject'
        )
    'collector.dll' = Assert-ProductionInteractionClosure `
        -Bytes $collectorBytes `
        -Name 'collector.dll' `
        -RequiredMarkers @('BATTLE_PLAN_NO_SEMANTIC_ENGINE_STATE_WRITES_V1') `
        -ExpectedPinnedHookCount ([uint16]$sourceCapabilityClosure.exact_pinned_hook_count) `
        -PinnedHookContractSourceSha256 ([string]$sourceCapabilityClosure.pinned_hook_contract_source_sha256)
}

$outputFull = [IO.Path]::GetFullPath($OutputZip)
$outputParent = Split-Path -Parent $outputFull
if (-not $outputParent) {
    throw 'OutputZip must have a parent directory'
}
[IO.Directory]::CreateDirectory($outputParent) | Out-Null
if (Test-Path -LiteralPath $outputFull) {
    throw "Output ZIP already exists; refusing overwrite: $outputFull"
}

$entries = [ordered]@{
    'Battle Plan Recorder.exe' = $recorderBytes
    'Battle Plan.exe' = $replayBytes
    'collector.dll' = $collectorBytes
}
$binaryHashes = [ordered]@{}
$hashLines = foreach ($name in $entries.Keys) {
    $hash = Get-BytesSha256 -Bytes $entries[$name]
    $binaryHashes[$name] = $hash
    "$hash  $name"
}
$readme = @"
Battle Plan $Version

This is a portable Windows package. It has no installer, service, tray process,
administrator requirement, automatic updater, or network dependency.

Run "Battle Plan Recorder.exe" once and leave its window open while you play.
It watches for supported the game sessions automatically. Use "Stop and
finish recordings" (or close the window) to stop cooperatively and verify any
active recording before exit. The window reports recording start, verified
completion, or failure and shows the exact output path. Use "Open recordings
folder" to reveal the current output directory.

The recorder is a passive injected observer. It does not pause the game, change game
speed, send input, move or focus the game window, or start/stop the game process.
Evidence-backed observation hooks are allowed only for values that cannot be
sampled reliably; they do not grant authority to mutate the simulation. Capture
is designed to remain passive while the player runs at speed five.

Run "Battle Plan.exe" to open and view a replay. Its picker starts in the
recorder output root when that folder exists. On first use, select the Hearts of
Iron IV installation folder and user-data folder when prompted.

Recorder data is stored under the current user's LocalAppData in
"Battle Plan Recorder" with a protected user/SYSTEM ACL. SHA256SUMS.txt detects
accidental package corruption; it is not a publisher signature.
"@
$utf8 = [Text.UTF8Encoding]::new($false)
$readmeBytes = $utf8.GetBytes($readme.Replace("`r`n", "`n"))
$checksumBytes = $utf8.GetBytes((($hashLines -join "`n") + "`n"))
$packageEntries = [ordered]@{
    'Battle Plan Recorder.exe' = $recorderBytes
    'Battle Plan.exe' = $replayBytes
    'collector.dll' = $collectorBytes
    'README.txt' = $readmeBytes
    'SHA256SUMS.txt' = $checksumBytes
}

$temporary = Join-Path $outputParent ('.' + [IO.Path]::GetFileName($outputFull) + '.' + [Guid]::NewGuid().ToString('N') + '.tmp')
try {
    $file = [IO.File]::Open(
        $temporary,
        [IO.FileMode]::CreateNew,
        [IO.FileAccess]::ReadWrite,
        [IO.FileShare]::None
    )
    try {
        $archive = [IO.Compression.ZipArchive]::new($file, [IO.Compression.ZipArchiveMode]::Create, $true)
        try {
            foreach ($name in $packageEntries.Keys) {
                Add-BytesEntry -Archive $archive -Name $name -Bytes $packageEntries[$name]
            }
        }
        finally {
            $archive.Dispose()
        }
        $file.Flush($true)
    }
    finally {
        $file.Dispose()
    }

    if ($TestOnlyFailAfterTemporaryWrite) {
        throw 'test-only failure after temporary portable package write'
    }

    $temporaryBytes = Read-LockedBytes -Path $temporary -Label 'Completed temporary product ZIP'
    Assert-PackageZipBytes `
        -ZipBytes $temporaryBytes `
        -ExpectedEntries $packageEntries `
        -ExpectedBinaryHashes $binaryHashes `
        -ExpectedImportAttestations $binaryImportAttestations `
        -ExpectedInteractionClosures $binaryInteractionClosures
    [PortablePackageNative]::PublishNoReplaceWriteThrough($temporary, $outputFull)
}
finally {
    if (Test-Path -LiteralPath $temporary) {
        Remove-Item -LiteralPath $temporary -Force
    }
}

try {
    if ($TestOnlyFailAfterPublishedWrite) {
        throw 'test-only failure after published portable package write'
    }
    $zipBytes = Read-LockedBytes -Path $outputFull -Label 'Completed product ZIP'
    Assert-PackageZipBytes `
        -ZipBytes $zipBytes `
        -ExpectedEntries $packageEntries `
        -ExpectedBinaryHashes $binaryHashes `
        -ExpectedImportAttestations $binaryImportAttestations `
        -ExpectedInteractionClosures $binaryInteractionClosures
    if (((Get-InteractionPolicyInputs) | ConvertTo-Json -Compress) -cne
        ($interactionPolicyInputs | ConvertTo-Json -Compress)) {
        throw 'portable package interaction-policy sources changed during packaging'
    }
    $zipHash = Get-BytesSha256 -Bytes $zipBytes
    $packageResult = [pscustomobject]@{
        schema = 'hoi4-portable-package.v1'
        version = $Version
        path = $outputFull
        sha256 = $zipHash
        input_sha256 = [ordered]@{
            recorder_host = $binaryHashes['Battle Plan Recorder.exe']
            replay_app = $binaryHashes['Battle Plan.exe']
            collector_dll = $binaryHashes['collector.dll']
        }
        pe_imports = [ordered]@{
            recorder_host = $binaryImportAttestations['Battle Plan Recorder.exe']
            replay_app = $binaryImportAttestations['Battle Plan.exe']
            collector_dll = $binaryImportAttestations['collector.dll']
        }
        interaction_closure = [ordered]@{
            policy = 'passive_injected_observer'
            zip_entry_bytes_reverified = $true
            policy_inputs = $interactionPolicyInputs
            source_capability = $sourceCapabilityClosure
            recorder_host = $binaryInteractionClosures['Battle Plan Recorder.exe']
            replay_app = $binaryInteractionClosures['Battle Plan.exe']
            collector_dll = $binaryInteractionClosures['collector.dll']
        }
        admissible_for_production = ($null -ne $productionCaptureEvidence)
        production_capture_evidence = $productionCaptureEvidence
        static_vcruntime_admission = $true
        executable_count = 2
        signed = $false
        install_kind = 'portable_zip'
    }
    $packageResultJson = $packageResult | ConvertTo-Json -Depth 8
}
catch {
    $verificationFailure = $_
    try {
        if (Test-Path -LiteralPath $outputFull) {
            Remove-Item -LiteralPath $outputFull -Force -ErrorAction Stop
        }
        if (Test-Path -LiteralPath $outputFull) {
            throw 'published package residue remains'
        }
    }
    catch {
        throw "portable package post-publication verification failed ($($verificationFailure.Exception.Message)); fail-closed package removal also failed ($($_.Exception.Message))"
    }
    throw $verificationFailure
}
$packageResultJson
