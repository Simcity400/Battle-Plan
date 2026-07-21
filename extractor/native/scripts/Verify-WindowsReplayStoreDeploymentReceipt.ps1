#Requires -Version 5.1

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Receipt,
    [Parameter(Mandatory = $true)][ValidateSet('Enabled', 'Disabled')][string]$ExpectedProviderState,
    [Parameter()][string]$EnabledBaseline
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-ExactKeys {
    param([Parameter(Mandatory = $true)]$Object, [string[]]$Keys, [string]$Label)
    $actual = @($Object.PSObject.Properties.Name | Sort-Object)
    $expected = @($Keys | Sort-Object)
    if (($actual -join ',') -cne ($expected -join ',')) {
        throw "$Label keys are noncanonical: $($actual -join ',')"
    }
}

function Assert-Sha256 {
    param([string]$Value, [string]$Label)
    if ($Value -cnotmatch '^[0-9a-f]{64}$') { throw "$Label is not lowercase SHA-256" }
}

function Assert-FileId128 {
    param([string]$Value, [string]$Label)
    if ($Value -cnotmatch '^[0-9a-f]{32}$' -or
        $Value -ceq ('0' * 32) -or $Value -ceq ('f' * 32)) {
        throw "$Label is not an available lowercase 128-bit file ID"
    }
}

function Read-And-VerifyReceipt {
    param([string]$Path, [string]$ExpectedState)
    $full = [System.IO.Path]::GetFullPath($Path)
    if (-not (Test-Path -LiteralPath $full -PathType Leaf)) { throw "receipt not found" }
    $raw = Get-Content -LiteralPath $full -Raw
    if ($raw -match '(?i)([a-z]:[\\/]|\\\\[^\\]+\\|/users/|/home/)') {
        throw 'portable receipt embeds an absolute host path'
    }
    $value = $raw | ConvertFrom-Json
    Assert-ExactKeys $value @(
        'schema','status','provider_state','operator_authorized_payload_open','fixture',
        'expected_behavior','tooling','observations','portable','absolute_paths_embedded'
    ) 'receipt'
    if ([string]$value.schema -cne 'hoi4.windows-replay-store-deployment-observation.v1' -or
        [string]$value.provider_state -cne $ExpectedState.ToLowerInvariant() -or
        $value.operator_authorized_payload_open -cne $true -or
        $value.portable -cne $true -or $value.absolute_paths_embedded -cne $false) {
        throw 'receipt authority fields are invalid'
    }
    $expectedBehavior = if ($ExpectedState -eq 'Enabled') {
        'exact listed placeholder hydrates through identity-bound reopen and reaches typed replay metadata'
    } else {
        'exact listed placeholder refuses without logical bytes; no ordinary-file substitution'
    }
    if ([string]$value.expected_behavior -cne $expectedBehavior) {
        throw 'receipt expected_behavior does not match its provider state'
    }
    Assert-ExactKeys $value.fixture @(
        'metadata_manifest_sha256','enabled_baseline_metadata_manifest_sha256','scanned_files',
        'replay_snapshot_placeholder_files','registered_onedrive_account_label',
        'registered_onedrive_account_volume_serial_number',
        'registered_onedrive_account_file_id_128',
        'replay_store_volume_serial_number','replay_store_file_id_128',
        'selected_placeholders'
    ) 'fixture'
    Assert-Sha256 ([string]$value.fixture.metadata_manifest_sha256) 'fixture digest'
    if ([string]$value.fixture.registered_onedrive_account_label -cnotmatch
        '^(environment:(OneDrive|OneDriveConsumer|OneDriveCommercial)|registry:[A-Za-z0-9_.-]+)$' -or
        [string]$value.fixture.registered_onedrive_account_volume_serial_number -cnotmatch '^[0-9a-f]{16}$' -or
        [string]$value.fixture.replay_store_volume_serial_number -cnotmatch '^[0-9a-f]{16}$' -or
        $value.fixture.scanned_files -is [bool] -or
        $value.fixture.replay_snapshot_placeholder_files -is [bool] -or
        [int64]$value.fixture.scanned_files -lt 2 -or
        [int64]$value.fixture.replay_snapshot_placeholder_files -lt 2 -or
        [int64]$value.fixture.replay_snapshot_placeholder_files -gt
            [int64]$value.fixture.scanned_files) {
        throw 'fixture account/store identity is invalid'
    }
    Assert-FileId128 ([string]$value.fixture.registered_onedrive_account_file_id_128) 'account root file ID'
    Assert-FileId128 ([string]$value.fixture.replay_store_file_id_128) 'replay store file ID'
    if ([string]$value.fixture.registered_onedrive_account_volume_serial_number -cne
        [string]$value.fixture.replay_store_volume_serial_number) {
        throw 'account root and replay store are not on one anchored volume'
    }
    $selected = @($value.fixture.selected_placeholders)
    if ($selected.Count -ne 2) { throw 'fixture must bind exactly two selected placeholders' }
    $seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    foreach ($item in $selected) {
        Assert-ExactKeys $item @('relative_path','reparse_tag','volume_serial_number','file_id_128') 'selected placeholder'
        if ([string]$item.relative_path -cnotmatch '^[^/\\]+/[^/\\]+\.snapshot\.json$' -or
            [string]$item.reparse_tag -cnotmatch '^0x[0-9a-f]{8}$' -or
            [string]$item.volume_serial_number -cnotmatch '^[0-9a-f]{16}$' -or
            -not $seen.Add([string]$item.relative_path)) {
            throw 'selected placeholder identity is invalid or duplicated'
        }
        Assert-FileId128 ([string]$item.file_id_128) 'selected placeholder file ID'
        if ([string]$item.volume_serial_number -cne
            [string]$value.fixture.replay_store_volume_serial_number) {
            throw 'selected placeholder escaped the replay-store volume'
        }
        $tag = [Convert]::ToUInt32(([string]$item.reparse_tag).Substring(2), 16)
        $mask = [Convert]::ToUInt32('ffff0fff', 16)
        $cloud = [Convert]::ToUInt32('9000001a', 16)
        if (($tag -band $mask) -ne $cloud) {
            throw 'selected placeholder tag is outside the Cloud Files family'
        }
    }
    if (([string]$selected[0].relative_path).Split('/')[0] -cne
        ([string]$selected[1].relative_path).Split('/')[0]) {
        throw 'selected placeholders are not from one game'
    }

    $observations = @($value.observations)
    if ($observations.Count -ne 2 -or
        (@($observations.layer) -join ',') -cne 'snapshot_store,replay_core') {
        throw 'receipt must contain ordered snapshot_store and replay_core observations'
    }
    $receiptDir = Split-Path -Parent $full
    for ($index = 0; $index -lt 2; $index++) {
        $observation = $observations[$index]
        Assert-ExactKeys $observation @(
            'layer','selected_placeholder','selected_reparse_tag','exit_code',
            'transcript_file','transcript_sha256'
        ) "observation[$index]"
        if ([string]$observation.selected_placeholder -cne [string]$selected[$index].relative_path -or
            [string]$observation.selected_reparse_tag -cne [string]$selected[$index].reparse_tag -or
            [string]$observation.transcript_file -cnotmatch '^[^/\\]+$' -or
            $observation.exit_code -is [bool] -or
            $observation.exit_code -isnot [ValueType]) {
            throw "observation[$index] does not bind its selected fixture"
        }
        $log = Join-Path $receiptDir ([string]$observation.transcript_file)
        Assert-Sha256 ([string]$observation.transcript_sha256) "observation[$index] transcript"
        if (-not (Test-Path -LiteralPath $log -PathType Leaf) -or
            (Get-FileHash -LiteralPath $log -Algorithm SHA256).Hash.ToLowerInvariant() -cne
                [string]$observation.transcript_sha256) {
            throw "observation[$index] transcript is absent or changed"
        }
    }
    $exits = @($observations | ForEach-Object { [int]$_.exit_code })
    if ([string]$value.status -ceq 'PASS') {
        if (@($exits | Where-Object { $_ -ne 0 }).Count -ne 0) { throw 'PASS receipt has a failed observation' }
    } elseif ([string]$value.status -ceq 'FAIL') {
        if (@($exits | Where-Object { $_ -ne 0 }).Count -eq 0) { throw 'FAIL receipt has no failed observation' }
    } else { throw 'receipt status is invalid' }

    foreach ($property in $value.tooling.PSObject.Properties) {
        Assert-Sha256 ([string]$property.Value) "tooling.$($property.Name)"
    }
    $requiredTools = @(
        'acceptance_runner_sha256','capability_probe_sha256','path_identity_helper_sha256','central_rust_runner_sha256',
        'extractor_test_source_sha256','replay_test_source_sha256','receipt_verifier_sha256',
        'receipt_schema_sha256','capability_receipt_sha256'
    )
    Assert-ExactKeys $value.tooling $requiredTools 'tooling'

    $repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
    $toolPaths = @{
        acceptance_runner_sha256 = Join-Path $PSScriptRoot 'Invoke-WindowsReplayStoreDeploymentAcceptance.ps1'
        capability_probe_sha256 = Join-Path $PSScriptRoot 'Test-WindowsReplayStoreCapabilities.ps1'
        path_identity_helper_sha256 = Join-Path $PSScriptRoot 'WindowsPath.Identity.ps1'
        central_rust_runner_sha256 = Join-Path $PSScriptRoot 'Invoke-CentralRustValidation.ps1'
        extractor_test_source_sha256 = Join-Path $repoRoot 'extractor\native\snapshot\tests\dir_store_namespace.rs'
        replay_test_source_sha256 = Join-Path $repoRoot 'replay_interface\native\replay_core\tests\world_timeline.rs'
        receipt_verifier_sha256 = $PSCommandPath
        receipt_schema_sha256 = Join-Path $repoRoot 'extractor\re_evidence\BUNDLE_ONEDRIVE\ONEDRIVE_DEPLOYMENT_RECEIPT.schema.json'
    }
    foreach ($entry in $toolPaths.GetEnumerator()) {
        $recordedToolHash = [string]$value.tooling.PSObject.Properties[$entry.Key].Value
        if ((Get-FileHash -LiteralPath $entry.Value -Algorithm SHA256).Hash.ToLowerInvariant() -cne
            $recordedToolHash) {
            throw "tooling hash mismatch: $($entry.Key)"
        }
    }
    $stem = [System.IO.Path]::GetFileNameWithoutExtension($full)
    $capability = Join-Path $receiptDir "$stem.capability.json"
    if (-not (Test-Path -LiteralPath $capability -PathType Leaf) -or
        (Get-FileHash -LiteralPath $capability -Algorithm SHA256).Hash.ToLowerInvariant() -cne
            [string]$value.tooling.capability_receipt_sha256) {
        throw 'capability sidecar is absent or changed'
    }
    $capabilityRaw = Get-Content -LiteralPath $capability -Raw
    if ($capabilityRaw -match '(?i)([a-z]:[\\/]|\\\\[^\\]+\\|/users/|/home/)') {
        throw 'portable capability sidecar embeds an absolute host path'
    }
    $capabilityValue = $capabilityRaw | ConvertFrom-Json
    Assert-ExactKeys $capabilityValue @(
        'schema','probe_is_non_mutating','selected_gate','probe_script_sha256',
        'path_identity_helper_sha256','overall_status','ready_gate_count',
        'total_gate_count','gates'
    ) 'capability receipt'
    if ([string]$capabilityValue.schema -cne 'hoi4_windows_replay_store_capability_probe_v3' -or
        $capabilityValue.probe_is_non_mutating -cne $true -or
        [string]$capabilityValue.selected_gate -cne 'OneDrive' -or
        [string]$capabilityValue.overall_status -cne 'READY_TO_RUN' -or
        [int]$capabilityValue.ready_gate_count -ne 1 -or
        [int]$capabilityValue.total_gate_count -ne 1 -or
        @($capabilityValue.gates).Count -ne 1 -or
        [string]$capabilityValue.probe_script_sha256 -cne [string]$value.tooling.capability_probe_sha256 -or
        [string]$capabilityValue.path_identity_helper_sha256 -cne [string]$value.tooling.path_identity_helper_sha256) {
        throw 'capability receipt authority fields are invalid'
    }
    Assert-ExactKeys $capabilityValue.gates[0] @(
        'gate','status','reason_code','detail','evidence'
    ) 'capability gate'
    $capabilityEvidence = $capabilityValue.gates[0].evidence
    Assert-ExactKeys $capabilityEvidence @(
        'scanned_files','replay_store_metadata_manifest_sha256','placeholder_files',
        'replay_snapshot_placeholder_files','offline_files','recall_on_open_files',
        'recall_on_data_access_files','skipped_reparse_directories',
        'placeholder_sample','replay_snapshot_placeholder_sample',
        'selected_replay_snapshots','registered_onedrive_account_label',
        'registered_onedrive_account_volume_serial_number',
        'registered_onedrive_account_file_id_128',
        'replay_store_volume_serial_number','replay_store_file_id_128',
        'probe_opened_payload_bytes'
    ) 'capability evidence'
    if ([string]$capabilityValue.gates[0].status -cne 'READY_TO_RUN' -or
        [string]$capabilityValue.gates[0].gate -cne 'onedrive_dehydrated_directory_store' -or
        [string]$capabilityValue.gates[0].reason_code -cne 'ONEDRIVE_CLOUD_PLACEHOLDER_METADATA_PRESENT' -or
        $capabilityEvidence.probe_opened_payload_bytes -cne $false -or
        $capabilityEvidence.scanned_files -is [bool] -or
        $capabilityEvidence.replay_snapshot_placeholder_files -is [bool] -or
        [int64]$capabilityEvidence.scanned_files -cne [int64]$value.fixture.scanned_files -or
        [int64]$capabilityEvidence.replay_snapshot_placeholder_files -cne
            [int64]$value.fixture.replay_snapshot_placeholder_files -or
        [string]$capabilityEvidence.replay_store_metadata_manifest_sha256 -cne
            [string]$value.fixture.metadata_manifest_sha256 -or
        [string]$capabilityEvidence.registered_onedrive_account_label -cne
            [string]$value.fixture.registered_onedrive_account_label -or
        [string]$capabilityEvidence.registered_onedrive_account_volume_serial_number -cne
            [string]$value.fixture.registered_onedrive_account_volume_serial_number -or
        [string]$capabilityEvidence.registered_onedrive_account_file_id_128 -cne
            [string]$value.fixture.registered_onedrive_account_file_id_128 -or
        [string]$capabilityEvidence.replay_store_volume_serial_number -cne
            [string]$value.fixture.replay_store_volume_serial_number -or
        [string]$capabilityEvidence.replay_store_file_id_128 -cne
            [string]$value.fixture.replay_store_file_id_128 -or
        (@($capabilityEvidence.selected_replay_snapshots) | ConvertTo-Json -Depth 5 -Compress) -cne
            ($selected | ConvertTo-Json -Depth 5 -Compress)) {
        throw 'capability receipt does not bind the deployment fixture'
    }
    return $value
}

$current = Read-And-VerifyReceipt -Path $Receipt -ExpectedState $ExpectedProviderState
if ($ExpectedProviderState -eq 'Enabled') {
    if ($null -ne $current.fixture.enabled_baseline_metadata_manifest_sha256) {
        throw 'Enabled receipt must not name an Enabled baseline'
    }
} else {
    if ([string]::IsNullOrWhiteSpace($EnabledBaseline)) {
        throw 'Disabled receipt verification requires -EnabledBaseline'
    }
    $baseline = Read-And-VerifyReceipt -Path $EnabledBaseline -ExpectedState Enabled
    foreach ($field in @(
        'metadata_manifest_sha256','registered_onedrive_account_label',
        'registered_onedrive_account_volume_serial_number',
        'registered_onedrive_account_file_id_128',
        'replay_store_volume_serial_number','replay_store_file_id_128'
    )) {
        if ([string]$current.fixture.$field -cne [string]$baseline.fixture.$field) {
            throw "Disabled/Enabled fixture join differs at $field"
        }
    }
    if ([string]$current.fixture.enabled_baseline_metadata_manifest_sha256 -cne
        [string]$baseline.fixture.metadata_manifest_sha256) {
        throw 'Disabled receipt does not bind the Enabled metadata digest'
    }
}

Write-Host "windows_replay_store_receipt_verified=$([System.IO.Path]::GetFullPath($Receipt))"
