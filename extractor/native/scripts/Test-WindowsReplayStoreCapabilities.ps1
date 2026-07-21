#Requires -Version 5.1

<#
.SYNOPSIS
Read-only preflight for the two environment-gated Windows replay-store tests.

.DESCRIPTION
This probe never creates a symlink, opens a replay payload for data access, or
changes a OneDrive pin/hydration setting.  Developer Mode is read from its
documented registry switch.  A candidate OneDrive directory store is inspected
through directory metadata only; no Get-Content/read stream is used.

The probe deliberately does not claim that the tests passed.  READY_TO_RUN
means the corresponding mutating/rehydrating integration test may now be run.
HUMAN_INPUT_REQUIRED carries a stable reason code for the exact missing gate.
#>

[CmdletBinding()]
param(
    [Parameter()]
    [string]$ReplayStore = $(if (Test-Path Env:BATTLE_PLAN_ONEDRIVE_DEHYDRATED_REPLAY_STORE) { $env:BATTLE_PLAN_ONEDRIVE_DEHYDRATED_REPLAY_STORE } else { $env:HOI4_ONEDRIVE_DEHYDRATED_REPLAY_STORE }),

    [Parameter()]
    [ValidateSet('All', 'DeveloperMode', 'OneDrive')]
    [string]$Gate = 'All',

    [Parameter()]
    [switch]$Json,

    [Parameter()]
    [string]$OutFile,

    [Parameter()]
    [switch]$RequireReady
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'WindowsPath.Identity.ps1')

function New-GateResult {
    param(
        [Parameter(Mandatory)] [string]$Gate,
        [Parameter(Mandatory)] [string]$Status,
        [Parameter(Mandatory)] [string]$ReasonCode,
        [Parameter(Mandatory)] [string]$Detail,
        [Parameter()] [hashtable]$Evidence = @{}
    )

    [pscustomobject]@{
        gate = $Gate
        status = $Status
        reason_code = $ReasonCode
        detail = $Detail
        evidence = [pscustomobject]$Evidence
    }
}

function Get-DeveloperModeGate {
    $key = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock'
    try {
        $value = $null
        if (Test-Path -LiteralPath $key) {
            $value = (Get-ItemProperty -LiteralPath $key `
                    -Name AllowDevelopmentWithoutDevLicense `
                    -ErrorAction SilentlyContinue).AllowDevelopmentWithoutDevLicense
        }
        if ($value -eq 1) {
            return New-GateResult `
                -Gate 'developer_mode_symlink_rejection' `
                -Status 'READY_TO_RUN' `
                -ReasonCode 'DEVELOPER_MODE_REGISTRY_ENABLED' `
                -Detail 'Registry precondition is enabled; run the real temp-directory symlink rejection test to prove capability.' `
                -Evidence @{ registry_path = $key; registry_value = [int]$value }
        }

        return New-GateResult `
            -Gate 'developer_mode_symlink_rejection' `
            -Status 'HUMAN_INPUT_REQUIRED' `
            -ReasonCode 'DEVELOPER_MODE_REGISTRY_DISABLED' `
            -Detail 'Developer Mode is not enabled. This probe will not change the OS setting.' `
            -Evidence @{ registry_path = $key; registry_value = $value }
    }
    catch {
        return New-GateResult `
            -Gate 'developer_mode_symlink_rejection' `
            -Status 'HUMAN_INPUT_REQUIRED' `
            -ReasonCode 'DEVELOPER_MODE_REGISTRY_INDETERMINATE' `
            -Detail ("Could not read the Developer Mode registry precondition: {0}" -f $_.Exception.Message) `
            -Evidence @{ registry_path = $key }
    }
}

function Get-DehydratedStoreGate {
    param([AllowEmptyString()] [string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path)) {
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'HUMAN_INPUT_REQUIRED' `
            -ReasonCode 'ONEDRIVE_STORE_ENV_NOT_SET' `
            -Detail 'Set BATTLE_PLAN_ONEDRIVE_DEHYDRATED_REPLAY_STORE to a genuine dehydrated directory store. The probe will not create or dehydrate one.'
    }

    try {
        $root = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    }
    catch {
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'HUMAN_INPUT_REQUIRED' `
            -ReasonCode 'ONEDRIVE_STORE_PATH_UNAVAILABLE' `
            -Detail 'The configured replay-store path is unavailable.' `
            -Evidence @{ configured_path_supplied = $true; exception_type = $_.Exception.GetType().FullName }
    }

    if (-not $root.PSIsContainer) {
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'HUMAN_INPUT_REQUIRED' `
            -ReasonCode 'ONEDRIVE_STORE_NOT_DIRECTORY' `
            -Detail 'The configured path is not a directory replay store.' `
            -Evidence @{ configured_path_supplied = $true }
    }

    try {
        $rootIdentity = Get-WindowsPathIdentity -LiteralPath $root.FullName
        $oneDriveRoots = @(Get-OneDriveAccountRoots)
        $matchingAccounts = @($oneDriveRoots | Where-Object {
            Test-WindowsPathContainedByRoot -Candidate $rootIdentity.final_path -Root $_.final_path
        })
    }
    catch {
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'HUMAN_INPUT_REQUIRED' `
            -ReasonCode 'ONEDRIVE_ACCOUNT_IDENTITY_PROBE_FAILED' `
            -Detail 'Could not bind the replay store to a handle-resolved registered OneDrive account root.' `
            -Evidence @{ configured_path_supplied = $true; exception_type = $_.Exception.GetType().FullName }
    }
    if ($matchingAccounts.Count -eq 0) {
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'HUMAN_INPUT_REQUIRED' `
            -ReasonCode 'ONEDRIVE_ACCOUNT_ROOT_MISMATCH' `
            -Detail 'The handle-resolved replay store is not contained by any environment- or HKCU-registered OneDrive account root.' `
            -Evidence @{
                configured_path_supplied = $true
                registered_account_count = $oneDriveRoots.Count
                replay_store_volume_serial_number = $rootIdentity.volume_serial_number
                replay_store_file_id_128 = $rootIdentity.file_id_128
            }
    }

    # FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_OPEN, and
    # FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS.  Any of these is metadata evidence
    # that file data is not fully resident.  Merely having REPARSE_POINT is not
    # sufficient: ordinary junctions/symlinks and hydrated cloud files also have
    # that bit.
    [uint32]$offline = 0x00001000
    [uint32]$recallOnOpen = 0x00040000
    [uint32]$recallOnDataAccess = 0x00400000
    [uint32]$dehydratedMask = $offline -bor $recallOnOpen -bor $recallOnDataAccess
    [uint32]$reparsePoint = 0x00000400

    $pending = [System.Collections.Generic.Stack[string]]::new()
    $pending.Push($root.FullName)
    $visited = [System.Collections.Generic.HashSet[string]]::new(
        [System.StringComparer]::OrdinalIgnoreCase
    )
    $scannedFiles = 0
    $placeholderFiles = 0
    $replaySnapshotPlaceholderFiles = 0
    $offlineFiles = 0
    $recallOnOpenFiles = 0
    $recallOnDataAccessFiles = 0
    $skippedReparseDirectories = 0
    $sample = [System.Collections.Generic.List[string]]::new()
    $snapshotSample = [System.Collections.Generic.List[string]]::new()
    $snapshotCandidates = [System.Collections.Generic.List[string]]::new()
    $snapshotCandidateEvidence = @{}
    $metadataRows = [System.Collections.Generic.List[string]]::new()
    $rootPrefix = $root.FullName.TrimEnd('\', '/') + [System.IO.Path]::DirectorySeparatorChar

    try {
        while ($pending.Count -gt 0) {
            $directory = $pending.Pop()
            if (-not $visited.Add($directory)) {
                continue
            }
            foreach ($entry in Get-ChildItem -LiteralPath $directory -Force -ErrorAction Stop) {
                [uint32]$attributes = [uint32]$entry.Attributes
                if ($entry.PSIsContainer) {
                    # Never follow a descendant reparse directory during a
                    # capability probe. The production loader will classify its
                    # tag through an anchored handle; pathname traversal here
                    # would undermine the non-mutating, bounded preflight.
                    if (($attributes -band $reparsePoint) -ne 0) {
                        $skippedReparseDirectories++
                    }
                    else {
                        $pending.Push($entry.FullName)
                    }
                    continue
                }

                $scannedFiles++
                $relative = if ($entry.FullName.StartsWith(
                    $rootPrefix,
                    [System.StringComparison]::OrdinalIgnoreCase
                )) {
                    $entry.FullName.Substring($rootPrefix.Length).Replace('\', '/')
                }
                else {
                    $entry.Name
                }
                $metadataRows.Add(("{0}`t{1}`t{2:x8}" -f $relative, [int64]$entry.Length, $attributes))
                if (($attributes -band $offline) -ne 0) { $offlineFiles++ }
                if (($attributes -band $recallOnOpen) -ne 0) { $recallOnOpenFiles++ }
                if (($attributes -band $recallOnDataAccess) -ne 0) { $recallOnDataAccessFiles++ }
                if (($attributes -band $dehydratedMask) -ne 0) {
                    $placeholderFiles++
                    if ($sample.Count -lt 8) {
                        $sample.Add($relative)
                    }
                    # The integration gate loads replay snapshots. An unrelated
                    # cloud placeholder elsewhere under the selected tree must
                    # not make that gate non-vacuous.
                    $relativeParts = @($relative.Split('/'))
                    if (($attributes -band $reparsePoint) -ne 0 -and
                        $relativeParts.Count -eq 2 -and
                        $entry.Name.EndsWith('.snapshot.json', [System.StringComparison]::Ordinal)) {
                        # Bind the exact leaf to a metadata-only handle.  A
                        # generic reparse point is insufficient: require the
                        # Microsoft Cloud Files tag family and re-check final
                        # path containment through the opened handle so a
                        # junction alias cannot manufacture provenance.
                        $candidateIdentity = Get-WindowsPathIdentity `
                            -LiteralPath $entry.FullName -OpenReparsePoint
                        $underStore = Test-WindowsPathContainedByRoot `
                            -Candidate $candidateIdentity.final_path -Root $rootIdentity.final_path
                        $underAccount = @($matchingAccounts | Where-Object {
                            Test-WindowsPathContainedByRoot `
                                -Candidate $candidateIdentity.final_path -Root $_.final_path
                        }).Count -gt 0
                        if ($underStore -and $underAccount -and
                            (Test-CloudFilesReparseTag -Tag $candidateIdentity.reparse_tag)) {
                            $replaySnapshotPlaceholderFiles++
                            $snapshotCandidates.Add($relative)
                            $snapshotCandidateEvidence[$relative] = [pscustomobject]@{
                                relative_path = $relative
                                reparse_tag = ('0x{0:x8}' -f $candidateIdentity.reparse_tag)
                                volume_serial_number = $candidateIdentity.volume_serial_number
                                file_id_128 = $candidateIdentity.file_id_128
                            }
                            $metadataRows.Add(("cloud_identity`t{0}`t{1}`t{2}`t{3}" -f
                                $relative,
                                ('0x{0:x8}' -f $candidateIdentity.reparse_tag),
                                $candidateIdentity.volume_serial_number,
                                $candidateIdentity.file_id_128))
                        }
                    }
                }
            }
        }
    }
    catch {
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'HUMAN_INPUT_REQUIRED' `
            -ReasonCode 'ONEDRIVE_METADATA_PROBE_FAILED' `
            -Detail 'Metadata-only traversal failed.' `
            -Evidence @{ scanned_files = $scannedFiles; exception_type = $_.Exception.GetType().FullName }
    }

    $metadataRows.Sort([System.StringComparer]::Ordinal)
    $sample.Sort([System.StringComparer]::Ordinal)
    $snapshotCandidates.Sort([System.StringComparer]::Ordinal)
    $snapshotSample.Clear()
    foreach ($candidate in @($snapshotCandidates | Select-Object -First 8)) {
        $snapshotSample.Add($candidate)
    }
    # Both product layers must observe distinct placeholders from one game.
    # Selecting one candidate from each of two unrelated games would not prove
    # the deployment card's same-store/same-index contract.
    $candidateGroups = @($snapshotCandidates | Group-Object {
        ([string]$_).Split('/')[0]
    } | Sort-Object Name)
    $selectedGroup = @($candidateGroups | Where-Object { $_.Count -ge 2 } | Select-Object -First 1)
    $selectedSnapshots = if ($selectedGroup.Count -eq 1) {
        @($selectedGroup[0].Group | Sort-Object | Select-Object -First 2 | ForEach-Object {
            $snapshotCandidateEvidence[$_]
        })
    }
    else { @() }
    $metadataMaterial = [System.Text.Encoding]::UTF8.GetBytes(
        [string]::Join("`n", $metadataRows)
    )
    $metadataHasher = [System.Security.Cryptography.SHA256]::Create()
    try {
        $metadataDigest = [System.BitConverter]::ToString(
            $metadataHasher.ComputeHash($metadataMaterial)
        ).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $metadataHasher.Dispose()
    }

    $evidence = @{
        scanned_files = $scannedFiles
        replay_store_metadata_manifest_sha256 = $metadataDigest
        placeholder_files = $placeholderFiles
        replay_snapshot_placeholder_files = $replaySnapshotPlaceholderFiles
        offline_files = $offlineFiles
        recall_on_open_files = $recallOnOpenFiles
        recall_on_data_access_files = $recallOnDataAccessFiles
        skipped_reparse_directories = $skippedReparseDirectories
        placeholder_sample = @($sample)
        replay_snapshot_placeholder_sample = @($snapshotSample)
        selected_replay_snapshots = $selectedSnapshots
        registered_onedrive_account_label = [string]$matchingAccounts[0].account_label
        registered_onedrive_account_volume_serial_number = [string]$matchingAccounts[0].volume_serial_number
        registered_onedrive_account_file_id_128 = [string]$matchingAccounts[0].file_id_128
        replay_store_volume_serial_number = $rootIdentity.volume_serial_number
        replay_store_file_id_128 = $rootIdentity.file_id_128
        probe_opened_payload_bytes = $false
    }

    if ($selectedSnapshots.Count -ne 2) {
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'HUMAN_INPUT_REQUIRED' `
            -ReasonCode 'ONEDRIVE_REPLAY_SNAPSHOT_PAIR_NOT_DEHYDRATED' `
            -Detail 'No one game has two exact replay *.snapshot.json payloads carrying both REPARSE_POINT and OFFLINE/RECALL metadata; unrelated, cross-game, or fabricated ordinary-file attributes cannot close the replay-store gate.' `
            -Evidence $evidence
    }

    return New-GateResult `
        -Gate 'onedrive_dehydrated_directory_store' `
        -Status 'READY_TO_RUN' `
        -ReasonCode 'ONEDRIVE_CLOUD_PLACEHOLDER_METADATA_PRESENT' `
        -Detail 'Registered OneDrive-root containment and exact Cloud Files placeholder metadata are present. Running the integration test may hydrate files and therefore still requires explicit operator authorization.' `
        -Evidence $evidence
}

$gates = switch ($Gate) {
    'DeveloperMode' { @(Get-DeveloperModeGate) }
    'OneDrive' { @(Get-DehydratedStoreGate -Path $ReplayStore) }
    default { @(Get-DeveloperModeGate; Get-DehydratedStoreGate -Path $ReplayStore) }
}
$ready = @($gates | Where-Object { $_.status -eq 'READY_TO_RUN' }).Count
$report = [pscustomobject]@{
    schema = 'hoi4_windows_replay_store_capability_probe_v3'
    probe_is_non_mutating = $true
    selected_gate = $Gate
    probe_script_sha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
    path_identity_helper_sha256 = (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'WindowsPath.Identity.ps1') -Algorithm SHA256).Hash.ToLowerInvariant()
    overall_status = if ($ready -eq $gates.Count) { 'READY_TO_RUN' } else { 'HUMAN_INPUT_REQUIRED' }
    ready_gate_count = $ready
    total_gate_count = $gates.Count
    gates = $gates
}

if (-not [string]::IsNullOrWhiteSpace($OutFile)) {
    $requestedOutputParent = Split-Path -Parent ([System.IO.Path]::GetFullPath($OutFile))
    $resolvedOutputParentBeforeCreate = Resolve-WindowsPathThroughExistingAncestor `
        -LiteralPath $requestedOutputParent
    if (-not [string]::IsNullOrWhiteSpace($ReplayStore) -and
        (Test-Path -LiteralPath $ReplayStore -PathType Container)) {
        $selectedStoreIdentity = Get-WindowsPathIdentity -LiteralPath $ReplayStore
        if (Test-WindowsPathContainedByRoot `
            -Candidate $resolvedOutputParentBeforeCreate `
            -Root $selectedStoreIdentity.final_path) {
            throw 'Capability evidence output must not modify the replay-store fixture.'
        }
    }
    foreach ($account in @(Get-OneDriveAccountRoots)) {
        if (Test-WindowsPathContainedByRoot `
            -Candidate $resolvedOutputParentBeforeCreate -Root $account.final_path) {
            throw 'Capability evidence output must be outside every registered OneDrive account root.'
        }
    }
    $outputAnchor = Resolve-WindowsDurableNewFilePath -LiteralPath $OutFile
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    $payload = $utf8.GetBytes((($report | ConvertTo-Json -Depth 8) + [Environment]::NewLine))
    Publish-WindowsDurableNewFileBytes `
        -LiteralPath $outputAnchor.anchored_path -Bytes $payload | Out-Null
}

if ($Json) {
    $report | ConvertTo-Json -Depth 8
}
else {
    $report
}

if ($RequireReady -and $report.overall_status -ne 'READY_TO_RUN') {
    exit 2
}
