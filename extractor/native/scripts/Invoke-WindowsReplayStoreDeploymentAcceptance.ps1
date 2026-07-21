#Requires -Version 5.1

<#
.SYNOPSIS
Runs one explicit enabled/disabled OneDrive replay-store deployment observation.

.DESCRIPTION
The operator supplies only the actual sync-provider state and authorizes the
payload open. This runner owns the metadata-only preflight, exact ignored-test
selection through the centralized Rust runner, transcript hashing, portable
receipt construction, and enabled/disabled fixture-identity join.

Run Enabled first. Re-dehydrate the same store, disable the provider, then run
Disabled with -BaselineEnabledReceipt pointing at the Enabled receipt.
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ReplayStore,

    [Parameter(Mandatory = $true)]
    [ValidateSet('Enabled', 'Disabled')]
    [string]$ProviderState,

    [Parameter(Mandatory = $true)]
    [string]$OutFile,

    [Parameter(Mandatory = $true)]
    [switch]$AuthorizePayloadOpen,

    [Parameter()]
    [string]$BaselineEnabledReceipt
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'WindowsPath.Identity.ps1')

if ($env:OS -cne 'Windows_NT') {
    throw 'The OneDrive deployment observation is Windows-only.'
}
if (-not $AuthorizePayloadOpen) {
    throw 'Payload access was not authorized; the capability probe alone never hydrates a file.'
}
if ($ProviderState -eq 'Disabled' -and [string]::IsNullOrWhiteSpace($BaselineEnabledReceipt)) {
    throw 'Disabled observation requires -BaselineEnabledReceipt from the same store.'
}

$storeIdentityBefore = Get-WindowsPathIdentity -LiteralPath $ReplayStore
$requestedOutputParent = Split-Path -Parent ([System.IO.Path]::GetFullPath($OutFile))
$resolvedOutputParentBeforeCreate = Resolve-WindowsPathThroughExistingAncestor `
    -LiteralPath $requestedOutputParent
if (Test-WindowsPathContainedByRoot `
    -Candidate $resolvedOutputParentBeforeCreate -Root $storeIdentityBefore.final_path) {
    throw 'Deployment evidence output must not modify the replay-store fixture.'
}
foreach ($account in @(Get-OneDriveAccountRoots)) {
    if (Test-WindowsPathContainedByRoot `
        -Candidate $resolvedOutputParentBeforeCreate -Root $account.final_path) {
        throw 'Deployment evidence output must be outside every registered OneDrive account root.'
    }
}
$outputAnchor = Resolve-WindowsDurableNewFilePath -LiteralPath $OutFile
$resolvedOut = [string]$outputAnchor.anchored_path
$outParent = [string]$outputAnchor.parent_final_path

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$capabilityScript = Join-Path $PSScriptRoot 'Test-WindowsReplayStoreCapabilities.ps1'
$pathIdentityHelper = Join-Path $PSScriptRoot 'WindowsPath.Identity.ps1'
$centralRunner = Join-Path $PSScriptRoot 'Invoke-CentralRustValidation.ps1'
$receiptVerifier = Join-Path $PSScriptRoot 'Verify-WindowsReplayStoreDeploymentReceipt.ps1'
$receiptSchema = Join-Path $repoRoot 'extractor\re_evidence\BUNDLE_ONEDRIVE\ONEDRIVE_DEPLOYMENT_RECEIPT.schema.json'
$extractorTestSource = Join-Path $repoRoot 'extractor\native\snapshot\tests\dir_store_namespace.rs'
$replayTestSource = Join-Path $repoRoot 'replay_interface\native\replay_core\tests\world_timeline.rs'
$stem = [System.IO.Path]::GetFileNameWithoutExtension($resolvedOut)
$capabilityPath = Join-Path $outParent "$stem.capability.json"
$extractorLog = Join-Path $outParent "$stem.extractor-evidence.log"
$replayLog = Join-Path $outParent "$stem.replay-evidence.log"

foreach ($path in @($capabilityPath, $extractorLog, $replayLog)) {
    if (Test-Path -LiteralPath $path) {
        throw "Acceptance sidecar already exists and will not be overwritten: $path"
    }
}

& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $capabilityScript `
    -ReplayStore $ReplayStore -Gate OneDrive -OutFile $capabilityPath -RequireReady | Out-Null
if ($LASTEXITCODE -ne 0) {
    throw "OneDrive metadata preflight failed with exit code $LASTEXITCODE"
}
$capability = Get-Content -LiteralPath $capabilityPath -Raw | ConvertFrom-Json
if ([string]$capability.schema -cne 'hoi4_windows_replay_store_capability_probe_v3' -or
    [string]$capability.overall_status -cne 'READY_TO_RUN' -or
    @($capability.gates).Count -ne 1) {
    throw 'OneDrive metadata preflight did not produce one ready v3 gate.'
}
$fixtureDigest = [string]$capability.gates[0].evidence.replay_store_metadata_manifest_sha256
if ($fixtureDigest -cnotmatch '^[0-9a-f]{64}$') {
    throw 'OneDrive metadata preflight lacks a canonical fixture-manifest digest.'
}
$placeholderCount = [int64]$capability.gates[0].evidence.replay_snapshot_placeholder_files
$selectedPlaceholders = @($capability.gates[0].evidence.selected_replay_snapshots)
if ($placeholderCount -lt 2 -or $selectedPlaceholders.Count -ne 2) {
    throw 'Deployment observation requires two dehydrated listed snapshots so the snapshot and replay layers consume distinct exact placeholders.'
}
foreach ($selected in $selectedPlaceholders) {
    if ([string]$selected.relative_path -cnotmatch '^[^/\\]+/[^/\\]+\.snapshot\.json$' -or
        [string]$selected.reparse_tag -cnotmatch '^0x[0-9a-f]{8}$' -or
        [string]$selected.volume_serial_number -cnotmatch '^[0-9a-f]{16}$' -or
        [string]$selected.file_id_128 -cnotmatch '^[0-9a-f]{32}$') {
        throw 'OneDrive preflight selected a noncanonical placeholder identity.'
    }
}
if (([string]$selectedPlaceholders[0].relative_path).Split('/')[0] -cne
    ([string]$selectedPlaceholders[1].relative_path).Split('/')[0]) {
    throw 'OneDrive preflight did not select two placeholders from one game.'
}
$accountLabel = [string]$capability.gates[0].evidence.registered_onedrive_account_label
$accountVolume = [string]$capability.gates[0].evidence.registered_onedrive_account_volume_serial_number
$accountFileId = [string]$capability.gates[0].evidence.registered_onedrive_account_file_id_128
$storeVolume = [string]$capability.gates[0].evidence.replay_store_volume_serial_number
$storeFileId = [string]$capability.gates[0].evidence.replay_store_file_id_128
if ($accountLabel -cnotmatch '^(environment:(OneDrive|OneDriveConsumer|OneDriveCommercial)|registry:[A-Za-z0-9_.-]+)$' -or
    $accountVolume -cnotmatch '^[0-9a-f]{16}$' -or $accountFileId -cnotmatch '^[0-9a-f]{32}$' -or
    $storeVolume -cnotmatch '^[0-9a-f]{16}$' -or $storeFileId -cnotmatch '^[0-9a-f]{32}$') {
    throw 'OneDrive preflight lacks a canonical registered-account/store identity.'
}
if ($accountVolume -cne $storeVolume -or
    @($selectedPlaceholders | Where-Object {
        [string]$_.volume_serial_number -cne $storeVolume
    }).Count -ne 0) {
    throw 'OneDrive preflight identity chain does not remain on one anchored volume.'
}
if ($storeIdentityBefore.volume_serial_number -cne $storeVolume -or
    $storeIdentityBefore.file_id_128 -cne $storeFileId) {
    throw 'Replay-store final-handle identity changed across output anchoring and capability preflight.'
}

$baselineDigest = $null
if ($ProviderState -eq 'Disabled') {
    $baselinePath = [System.IO.Path]::GetFullPath($BaselineEnabledReceipt)
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $receiptVerifier `
        -Receipt $baselinePath -ExpectedProviderState Enabled
    if ($LASTEXITCODE -ne 0) {
        throw 'Independent verification rejected the Enabled baseline receipt.'
    }
    $baseline = Get-Content -LiteralPath $baselinePath -Raw | ConvertFrom-Json
    if ([string]$baseline.schema -cne 'hoi4.windows-replay-store-deployment-observation.v1' -or
        [string]$baseline.status -cne 'PASS' -or
        [string]$baseline.provider_state -cne 'enabled') {
        throw 'Baseline receipt is not a passing Enabled deployment observation.'
    }
    $baselineDigest = [string]$baseline.fixture.metadata_manifest_sha256
    if ($baselineDigest -cne $fixtureDigest) {
        throw 'Disabled observation fixture differs from the Enabled baseline metadata manifest.'
    }
    if ([string]$baseline.fixture.registered_onedrive_account_label -cne $accountLabel -or
        [string]$baseline.fixture.registered_onedrive_account_volume_serial_number -cne $accountVolume -or
        [string]$baseline.fixture.registered_onedrive_account_file_id_128 -cne $accountFileId -or
        [string]$baseline.fixture.replay_store_volume_serial_number -cne $storeVolume -or
        [string]$baseline.fixture.replay_store_file_id_128 -cne $storeFileId) {
        throw 'Disabled observation is not bound to the Enabled baseline OneDrive account/store identity.'
    }
}

$env:BATTLE_PLAN_ONEDRIVE_DEHYDRATED_REPLAY_STORE = [System.IO.Path]::GetFullPath($ReplayStore)
$env:BATTLE_PLAN_ONEDRIVE_SYNC_PROVIDER_STATE = $ProviderState.ToLowerInvariant()
$filter = 'test(windows_onedrive_dehydrated)'

function Invoke-CentralEvidence {
    param(
        [Parameter(Mandatory = $true)] [string]$Workspace,
        [Parameter(Mandatory = $true)] [string]$Package,
        [Parameter(Mandatory = $true)] [string]$TestTarget,
        [Parameter(Mandatory = $true)] [string]$SelectedPlaceholder,
        [Parameter(Mandatory = $true)] [string]$ExpectedReparseTag,
        [Parameter(Mandatory = $true)] [string]$ExpectedVolumeSerialNumber,
        [Parameter(Mandatory = $true)] [string]$ExpectedFileId128,
        [Parameter(Mandatory = $true)] [string]$LogPath
    )

    $env:BATTLE_PLAN_ONEDRIVE_SELECTED_REPLAY_SNAPSHOT = $SelectedPlaceholder
    $env:BATTLE_PLAN_ONEDRIVE_SELECTED_REPARSE_TAG = $ExpectedReparseTag
    $env:BATTLE_PLAN_ONEDRIVE_SELECTED_VOLUME_SERIAL_NUMBER = $ExpectedVolumeSerialNumber
    $env:BATTLE_PLAN_ONEDRIVE_SELECTED_FILE_ID_128 = $ExpectedFileId128
    $env:BATTLE_PLAN_ONEDRIVE_STORE_VOLUME_SERIAL_NUMBER = $storeVolume
    $env:BATTLE_PLAN_ONEDRIVE_STORE_FILE_ID_128 = $storeFileId
    $transcript = @(& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $centralRunner `
        -Mode Evidence -Workspace $Workspace -Package $Package `
        -TestTarget $TestTarget -FilterExpression $filter 2>&1)
    $exitCode = $LASTEXITCODE
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    $text = ([string]::Join(
        [Environment]::NewLine,
        @($transcript | ForEach-Object { [string]$_ })
    ) + [Environment]::NewLine)
    $bytes = $utf8.GetBytes($text)
    Publish-WindowsDurableNewFileBytes -LiteralPath $LogPath -Bytes $bytes | Out-Null
    return [pscustomobject]@{
        exit_code = $exitCode
        transcript_sha256 = (
            Get-FileHash -LiteralPath $LogPath -Algorithm SHA256
        ).Hash.ToLowerInvariant()
    }
}

$extractorObservation = Invoke-CentralEvidence `
    -Workspace Extractor -Package snapshot -TestTarget dir_store_namespace `
    -SelectedPlaceholder ([string]$selectedPlaceholders[0].relative_path) `
    -ExpectedReparseTag ([string]$selectedPlaceholders[0].reparse_tag) `
    -ExpectedVolumeSerialNumber ([string]$selectedPlaceholders[0].volume_serial_number) `
    -ExpectedFileId128 ([string]$selectedPlaceholders[0].file_id_128) `
    -LogPath $extractorLog
$replayObservation = Invoke-CentralEvidence `
    -Workspace Replay -Package replay_core -TestTarget world_timeline `
    -SelectedPlaceholder ([string]$selectedPlaceholders[1].relative_path) `
    -ExpectedReparseTag ([string]$selectedPlaceholders[1].reparse_tag) `
    -ExpectedVolumeSerialNumber ([string]$selectedPlaceholders[1].volume_serial_number) `
    -ExpectedFileId128 ([string]$selectedPlaceholders[1].file_id_128) `
    -LogPath $replayLog

$extractorExit = [int]$extractorObservation.exit_code
$replayExit = [int]$replayObservation.exit_code
$success = $extractorExit -eq 0 -and $replayExit -eq 0
$receipt = [ordered]@{
    schema = 'hoi4.windows-replay-store-deployment-observation.v1'
    status = if ($success) { 'PASS' } else { 'FAIL' }
    provider_state = $ProviderState.ToLowerInvariant()
    operator_authorized_payload_open = $true
    fixture = [ordered]@{
        metadata_manifest_sha256 = $fixtureDigest
        enabled_baseline_metadata_manifest_sha256 = $baselineDigest
        scanned_files = [int64]$capability.gates[0].evidence.scanned_files
        replay_snapshot_placeholder_files = $placeholderCount
        registered_onedrive_account_label = $accountLabel
        registered_onedrive_account_volume_serial_number = $accountVolume
        registered_onedrive_account_file_id_128 = $accountFileId
        replay_store_volume_serial_number = $storeVolume
        replay_store_file_id_128 = $storeFileId
        selected_placeholders = @($selectedPlaceholders | ForEach-Object {
            [ordered]@{
                relative_path = [string]$_.relative_path
                reparse_tag = [string]$_.reparse_tag
                volume_serial_number = [string]$_.volume_serial_number
                file_id_128 = [string]$_.file_id_128
            }
        })
    }
    expected_behavior = if ($ProviderState -eq 'Enabled') {
        'exact listed placeholder hydrates through identity-bound reopen and reaches typed replay metadata'
    }
    else {
        'exact listed placeholder refuses without logical bytes; no ordinary-file substitution'
    }
    tooling = [ordered]@{
        acceptance_runner_sha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
        capability_probe_sha256 = (Get-FileHash -LiteralPath $capabilityScript -Algorithm SHA256).Hash.ToLowerInvariant()
        path_identity_helper_sha256 = (Get-FileHash -LiteralPath $pathIdentityHelper -Algorithm SHA256).Hash.ToLowerInvariant()
        central_rust_runner_sha256 = (Get-FileHash -LiteralPath $centralRunner -Algorithm SHA256).Hash.ToLowerInvariant()
        extractor_test_source_sha256 = (Get-FileHash -LiteralPath $extractorTestSource -Algorithm SHA256).Hash.ToLowerInvariant()
        replay_test_source_sha256 = (Get-FileHash -LiteralPath $replayTestSource -Algorithm SHA256).Hash.ToLowerInvariant()
        receipt_verifier_sha256 = (Get-FileHash -LiteralPath $receiptVerifier -Algorithm SHA256).Hash.ToLowerInvariant()
        receipt_schema_sha256 = (Get-FileHash -LiteralPath $receiptSchema -Algorithm SHA256).Hash.ToLowerInvariant()
        capability_receipt_sha256 = (Get-FileHash -LiteralPath $capabilityPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    observations = @(
        [ordered]@{
            layer = 'snapshot_store'
            selected_placeholder = [string]$selectedPlaceholders[0].relative_path
            selected_reparse_tag = [string]$selectedPlaceholders[0].reparse_tag
            exit_code = $extractorExit
            transcript_file = [System.IO.Path]::GetFileName($extractorLog)
            transcript_sha256 = [string]$extractorObservation.transcript_sha256
        },
        [ordered]@{
            layer = 'replay_core'
            selected_placeholder = [string]$selectedPlaceholders[1].relative_path
            selected_reparse_tag = [string]$selectedPlaceholders[1].reparse_tag
            exit_code = $replayExit
            transcript_file = [System.IO.Path]::GetFileName($replayLog)
            transcript_sha256 = [string]$replayObservation.transcript_sha256
        }
    )
    portable = $true
    absolute_paths_embedded = $false
}

$utf8 = New-Object System.Text.UTF8Encoding($false)
$payload = $utf8.GetBytes((($receipt | ConvertTo-Json -Depth 10) + [Environment]::NewLine))
Publish-WindowsDurableNewFileBytes -LiteralPath $resolvedOut -Bytes $payload | Out-Null

$verifyArguments = @(
    '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $receiptVerifier,
    '-Receipt', $resolvedOut, '-ExpectedProviderState', $ProviderState
)
if ($ProviderState -eq 'Disabled') {
    $verifyArguments += @('-EnabledBaseline', ([System.IO.Path]::GetFullPath($BaselineEnabledReceipt)))
}
& powershell.exe @verifyArguments
if ($LASTEXITCODE -ne 0) {
    throw "Independent verification rejected the new OneDrive $ProviderState receipt."
}

if (-not $success) {
    throw "OneDrive $ProviderState deployment observation failed; receipt retained at $resolvedOut"
}

Write-Host "onedrive_deployment_receipt=$resolvedOut"
