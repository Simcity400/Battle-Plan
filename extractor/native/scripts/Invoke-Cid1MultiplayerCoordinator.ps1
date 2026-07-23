[CmdletBinding()]
param(
    [ValidateSet('RegisterObserved','RunLegacyTransport')]
    [string]$Mode = 'RegisterObserved',
    [ValidateSet('M1','M2','M3')]
    [string]$Arm,
    [Parameter(Mandatory=$true)]
    [string]$MatrixPlanPath,
    [string[]]$EndpointRequestPaths = @(),
    [string[]]$EndpointDescriptorPaths = @(),
    [string]$PreconditionPath,
    [string]$TransportDriverPath,
    [string]$MultiplayerOwnerProfilePath,
    [string]$OutputRoot,
    [switch]$AllowExternalEndpoints
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PinnedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$ContractPath = Join-Path $PSScriptRoot '..\..\re_evidence\CID1_LIFECYCLE_MATRIX_V1\multiplayer_coordinator_contract.json'
if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
    $OutputRoot = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)) 'battle-plan\evidence\cid1_lifecycle_matrix\multiplayer'
}
function Get-Sha256([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Write-AtomicJson([string]$Path,$Value) {
    $full=[IO.Path]::GetFullPath($Path);$parent=[IO.Path]::GetDirectoryName($full);[IO.Directory]::CreateDirectory($parent)|Out-Null
    $tmp=[IO.Path]::Combine($parent,([IO.Path]::GetRandomFileName()+'.tmp'))
    try{[IO.File]::WriteAllText($tmp,(($Value|ConvertTo-Json -Depth 30)+"`n"),[Text.UTF8Encoding]::new($false));Move-Item -LiteralPath $tmp -Destination $full -Force}finally{if(Test-Path -LiteralPath $tmp){Remove-Item -LiteralPath $tmp -Force}}
}
$contract = Get-Content -LiteralPath $ContractPath -Raw | ConvertFrom-Json
if ($contract.schema -ne 'battle-plan.campaign_identity_multiplayer_coordinator_contract.v1' -or $contract.pinned_executable_sha256 -ne $PinnedExeSha256) {
    throw 'CID1 multiplayer coordinator contract mismatch.'
}
$plan = Get-Content -LiteralPath $MatrixPlanPath -Raw | ConvertFrom-Json
if ($plan.schema -ne 'battle-plan.campaign_identity_matrix_plan.v1' -or $plan.pinned_executable_sha256 -ne $PinnedExeSha256) {
    throw 'CID1 matrix plan mismatch.'
}
if ($Mode -eq 'RegisterObserved') {
    if ($EndpointDescriptorPaths.Count -ne 2) {
        throw "$Arm passive registration requires exactly two finalized endpoint descriptors."
    }
    if ([string]::IsNullOrWhiteSpace($PreconditionPath) -or -not (Test-Path -LiteralPath $PreconditionPath -PathType Leaf)) {
        throw "$Arm passive registration requires a content-addressed precondition."
    }
    $registrar = Join-Path $PSScriptRoot 'Invoke-Cid1PassiveArmRegistration.ps1'
    & $registrar -Arm $Arm -MatrixPlanPath $MatrixPlanPath -EndpointDescriptorPaths $EndpointDescriptorPaths -PreconditionPath $PreconditionPath -OutputRoot (Join-Path ([IO.Path]::GetFullPath($OutputRoot)) $Arm)
    if (-not $?) { throw "CID1 $Arm passive multiplayer registration failed." }
    return
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
    throw 'Legacy transport execution is isolated and fail-closed. Use RegisterObserved for the production passive path.'
}
if ([string]::IsNullOrWhiteSpace($TransportDriverPath) -or -not (Test-Path -LiteralPath $TransportDriverPath -PathType Leaf)) {
    throw 'A real mutually authenticated endpoint transport driver is required; missing endpoints are never synthesized.'
}
if ([string]::IsNullOrWhiteSpace($MultiplayerOwnerProfilePath) -or -not (Test-Path -LiteralPath $MultiplayerOwnerProfilePath -PathType Leaf)) {
    throw 'A content-addressed multiplayer owner profile is required.'
}
[System.IO.Directory]::CreateDirectory([System.IO.Path]::GetFullPath($OutputRoot)) | Out-Null
$armRoot=Join-Path ([IO.Path]::GetFullPath($OutputRoot)) $Arm
if(Test-Path -LiteralPath $armRoot){throw "Refusing to reuse CID1 multiplayer arm root: $armRoot"}
[IO.Directory]::CreateDirectory($armRoot)|Out-Null
$transport=(Resolve-Path -LiteralPath $TransportDriverPath).Path;$ownerProfile=(Resolve-Path -LiteralPath $MultiplayerOwnerProfilePath).Path;$actionRequest=Join-Path $armRoot 'multiplayer_action_request.json'
Write-AtomicJson $actionRequest ([ordered]@{schema='battle-plan.campaign_identity_multiplayer_action_request.v1';arm=$Arm;matrix_plan_path=(Resolve-Path -LiteralPath $MatrixPlanPath).Path;matrix_plan_sha256=Get-Sha256 $MatrixPlanPath;transport_driver_path=$transport;transport_driver_sha256=Get-Sha256 $transport;owner_profile_path=$ownerProfile;owner_profile_sha256=Get-Sha256 $ownerProfile;endpoint_request_paths=@($EndpointRequestPaths|ForEach-Object{(Resolve-Path -LiteralPath $_).Path});endpoint_request_sha256s=@($EndpointRequestPaths|ForEach-Object{Get-Sha256 $_});output_root=$armRoot})
& $transport -Arm $Arm -MatrixPlanPath $MatrixPlanPath -EndpointRequestPaths $EndpointRequestPaths -OutputRoot $armRoot -CoordinatorContractPath $ContractPath -ActionRequestPath $actionRequest -OwnerProfilePath $ownerProfile
if (-not $?) { throw "CID1 $Arm multiplayer coordinator driver failed." }
$actionReceipt=Join-Path $armRoot 'arm_action_receipt.json'
$descriptorPaths=@($expectedRoles|ForEach-Object{Join-Path $armRoot ("endpoint_$_\endpoint_descriptor.json")})
if(-not(Test-Path -LiteralPath $actionReceipt -PathType Leaf)){throw "CID1 $Arm transport owner emitted no action receipt."}
$action=Get-Content -LiteralPath $actionReceipt -Raw|ConvertFrom-Json
if($action.schema-cne 'battle-plan.campaign_identity_arm_action_receipt.v1'-or$action.arm-cne$Arm-or$action.status-cne'passed'){throw "CID1 $Arm transport action receipt is not passing."}
foreach($descriptor in $descriptorPaths){if(-not(Test-Path -LiteralPath $descriptor -PathType Leaf)){throw "CID1 $Arm transport owner omitted endpoint descriptor: $descriptor"}}
$plan.arms.$Arm.state='complete';$plan.arms.$Arm.endpoint_descriptor_paths=$descriptorPaths;$plan.arms.$Arm.action_receipt_path=$actionReceipt;$plan.arms.$Arm.reason=$null
Write-AtomicJson (Resolve-Path -LiteralPath $MatrixPlanPath).Path $plan
Write-Output $actionReceipt
