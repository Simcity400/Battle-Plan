[CmdletBinding()]
param(
    [ValidateSet('M1','M2','M3')]
    [string]$Arm,
    [Parameter(Mandatory=$true)]
    [string]$MatrixPlanPath,
    [Parameter(Mandatory=$true)]
    [string[]]$EndpointRequestPaths,
    [string]$TransportDriverPath,
    [string]$OutputRoot = 'C:\hoi4_evidence\cid1_lifecycle_matrix\multiplayer',
    [switch]$AllowExternalEndpoints
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PinnedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$ContractPath = 'extractor\re_evidence\CID1_LIFECYCLE_MATRIX_V1\multiplayer_coordinator_contract.json'
$contract = Get-Content -LiteralPath $ContractPath -Raw | ConvertFrom-Json
if ($contract.schema -ne 'battle-plan.campaign_identity_multiplayer_coordinator_contract.v1' -or $contract.pinned_executable_sha256 -ne $PinnedExeSha256) {
    throw 'CID1 multiplayer coordinator contract mismatch.'
}
$plan = Get-Content -LiteralPath $MatrixPlanPath -Raw | ConvertFrom-Json
if ($plan.schema -ne 'battle-plan.campaign_identity_matrix_plan.v1' -or $plan.pinned_executable_sha256 -ne $PinnedExeSha256) {
    throw 'CID1 matrix plan mismatch.'
}
if ($EndpointRequestPaths.Count -ne 2) {
    throw 'A multiplayer arm requires exactly two independently registered endpoint requests.'
}
$requests = @()
foreach ($path in $EndpointRequestPaths) {
    $request = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
    $requiredFields = @($contract.endpoint_request.required_fields)
    $actualFields = @($request.PSObject.Properties.Name)
    $fieldDelta = @(Compare-Object -ReferenceObject $requiredFields -DifferenceObject $actualFields)
    if ($fieldDelta.Count -ne 0) {
        throw "Endpoint request has missing or extra fields: $path"
    }
    if ($request.schema -ne $contract.endpoint_request.schema -or $request.arm -ne $Arm -or $request.matrix_nonce_hex -ne $plan.matrix_nonce_hex -or $request.executable_sha256 -ne $PinnedExeSha256) {
        throw "Endpoint request does not bind the matrix/arm/build: $path"
    }
    if (-not (Test-Path -LiteralPath $request.collector_path -PathType Leaf) -or (Get-FileHash -LiteralPath $request.collector_path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $request.collector_sha256) {
        throw "Endpoint collector path/hash mismatch: $path"
    }
    if ([int64]$request.deadline_filetime_utc -le [int64]$request.not_before_filetime_utc) {
        throw "Endpoint request has an invalid rendezvous interval: $path"
    }
    $requests += $request
}
$expectedRoles = @($contract.arms.$Arm.roles)
$actualRoles = @($requests | ForEach-Object { $_.role })
if (($actualRoles -join ',') -ne ($expectedRoles -join ',')) {
    throw "Endpoint roles must be ordered exactly as: $($expectedRoles -join ', ')"
}
if ($requests[0].machine_id -eq $requests[1].machine_id -and $Arm -eq 'M3') {
    throw 'M3 requires two distinct real machine identities.'
}
if ($Arm -in @('M1','M2') -and $requests[0].machine_id -eq $requests[1].machine_id -and $requests[0].process_id -eq $requests[1].process_id -and $requests[0].process_creation_filetime_utc -eq $requests[1].process_creation_filetime_utc) {
    throw "$Arm requires two distinct real process endpoints."
}
if (-not $AllowExternalEndpoints) {
    throw 'External multiplayer execution is fail-closed. The validated interface is frozen, but no endpoint has been contacted.'
}
if ([string]::IsNullOrWhiteSpace($TransportDriverPath) -or -not (Test-Path -LiteralPath $TransportDriverPath -PathType Leaf)) {
    throw 'A real mutually authenticated endpoint transport driver is required; missing endpoints are never synthesized.'
}
[System.IO.Directory]::CreateDirectory([System.IO.Path]::GetFullPath($OutputRoot)) | Out-Null
& (Resolve-Path -LiteralPath $TransportDriverPath).Path -Arm $Arm -MatrixPlanPath $MatrixPlanPath -EndpointRequestPaths $EndpointRequestPaths -OutputRoot $OutputRoot -CoordinatorContractPath $ContractPath
if (-not $?) { throw "CID1 $Arm multiplayer coordinator driver failed." }
