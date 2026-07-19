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
    [switch]$Json,

    [Parameter()]
    [switch]$RequireReady
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

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
            -Detail ("The configured replay-store path is unavailable: {0}" -f $_.Exception.Message) `
            -Evidence @{ configured_path = $Path }
    }

    if (-not $root.PSIsContainer) {
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'HUMAN_INPUT_REQUIRED' `
            -ReasonCode 'ONEDRIVE_STORE_NOT_DIRECTORY' `
            -Detail 'The configured path is not a directory replay store.' `
            -Evidence @{ configured_path = $root.FullName }
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
    $offlineFiles = 0
    $recallOnOpenFiles = 0
    $recallOnDataAccessFiles = 0
    $skippedReparseDirectories = 0
    $sample = [System.Collections.Generic.List[string]]::new()

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
                if (($attributes -band $offline) -ne 0) { $offlineFiles++ }
                if (($attributes -band $recallOnOpen) -ne 0) { $recallOnOpenFiles++ }
                if (($attributes -band $recallOnDataAccess) -ne 0) { $recallOnDataAccessFiles++ }
                if (($attributes -band $dehydratedMask) -ne 0) {
                    $placeholderFiles++
                    if ($sample.Count -lt 8) {
                        $sample.Add($entry.FullName)
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
            -Detail ("Metadata-only traversal failed: {0}" -f $_.Exception.Message) `
            -Evidence @{ configured_path = $root.FullName; scanned_files = $scannedFiles }
    }

    $evidence = @{
        configured_path = $root.FullName
        scanned_files = $scannedFiles
        placeholder_files = $placeholderFiles
        offline_files = $offlineFiles
        recall_on_open_files = $recallOnOpenFiles
        recall_on_data_access_files = $recallOnDataAccessFiles
        skipped_reparse_directories = $skippedReparseDirectories
        placeholder_sample = @($sample)
        probe_opened_payload_bytes = $false
    }

    if ($placeholderFiles -eq 0) {
        return New-GateResult `
            -Gate 'onedrive_dehydrated_directory_store' `
            -Status 'HUMAN_INPUT_REQUIRED' `
            -ReasonCode 'ONEDRIVE_STORE_NOT_DEHYDRATED' `
            -Detail 'No payload file carries OFFLINE/RECALL metadata; a hydrated store cannot close the dehydrated-placeholder gate.' `
            -Evidence $evidence
    }

    return New-GateResult `
        -Gate 'onedrive_dehydrated_directory_store' `
        -Status 'READY_TO_RUN' `
        -ReasonCode 'DEHYDRATED_PAYLOAD_METADATA_PRESENT' `
        -Detail 'Dehydrated payload metadata is present. Running the integration test may hydrate files and therefore still requires explicit operator authorization.' `
        -Evidence $evidence
}

$gates = @(
    Get-DeveloperModeGate
    Get-DehydratedStoreGate -Path $ReplayStore
)
$ready = @($gates | Where-Object { $_.status -eq 'READY_TO_RUN' }).Count
$report = [pscustomobject]@{
    schema = 'hoi4_windows_replay_store_capability_probe_v1'
    probe_is_non_mutating = $true
    overall_status = if ($ready -eq $gates.Count) { 'READY_TO_RUN' } else { 'HUMAN_INPUT_REQUIRED' }
    ready_gate_count = $ready
    total_gate_count = $gates.Count
    gates = $gates
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
