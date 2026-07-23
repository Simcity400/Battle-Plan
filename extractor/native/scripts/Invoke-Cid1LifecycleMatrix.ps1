[CmdletBinding()]
param(
    [ValidateSet('Plan','RegisterObservedArm','RunLegacySDArm','FinalizeLocal','Finalize')]
    [string]$Mode = 'Plan',
    [ValidateSet('S1','S2','S3','S4','D1','D2','M1','M2','M3')]
    [string]$Arm,
    [string]$OutputRoot,
    [string]$ContractPath,
    [string]$ActionDriverPath,
    [string]$OwnerProfilePath,
    [string]$PreconditionPath,
    [string[]]$EndpointDescriptorPaths = @(),
    [string]$MatrixReceiptPath,
    [string]$GateOutPath,
    [string]$Python = 'python.exe',
    [switch]$AllowLiveLifecycleActions
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$PinnedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$ContractSchema = 'battle-plan.campaign_identity_matrix_contract.v1'
$PlanSchema = 'battle-plan.campaign_identity_matrix_plan.v1'
$RequestSchema = 'battle-plan.campaign_identity_sd_arm_request.v2'
$SDArms = @('S1','S2','S3','S4','D1','D2')
$AllArms = @('S1','S2','S3','S4','D1','D2','M1','M2','M3')
if ([string]::IsNullOrWhiteSpace($ContractPath)) {
    $ContractPath = Join-Path $PSScriptRoot '..\..\re_evidence\CID1_LIFECYCLE_MATRIX_V1\matrix_contract.json'
}
if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
    $OutputRoot = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)) 'battle-plan\evidence\cid1_lifecycle_matrix'
}

function Get-Sha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function New-HexNonce {
    $bytes = [byte[]]::new(32)
    [System.Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
    return [Convert]::ToHexString($bytes).ToLowerInvariant()
}

function Write-AtomicJson([string]$Path, $Value) {
    $resolvedParent = [System.IO.Path]::GetFullPath((Split-Path -Parent $Path))
    [System.IO.Directory]::CreateDirectory($resolvedParent) | Out-Null
    $temporary = Join-Path $resolvedParent ('.' + [System.IO.Path]::GetFileName($Path) + '.' + [guid]::NewGuid().ToString('N') + '.tmp')
    try {
        $json = ($Value | ConvertTo-Json -Depth 20) + "`n"
        [System.IO.File]::WriteAllText($temporary, $json, [System.Text.UTF8Encoding]::new($false))
        Move-Item -LiteralPath $temporary -Destination $Path -Force
    }
    finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
    }
}

$resolvedContract = (Resolve-Path -LiteralPath $ContractPath).Path
$contract = Get-Content -LiteralPath $resolvedContract -Raw | ConvertFrom-Json
if ($contract.schema -ne $ContractSchema -or $contract.pinned_executable_sha256 -ne $PinnedExeSha256) {
    throw 'CID1 lifecycle matrix contract schema or executable pin mismatch.'
}
$contractSha256 = Get-Sha256 $resolvedContract
$resolvedOutput = [System.IO.Path]::GetFullPath($OutputRoot)
[System.IO.Directory]::CreateDirectory($resolvedOutput) | Out-Null
$planPath = Join-Path $resolvedOutput 'matrix_plan.json'

if ($Mode -eq 'Plan') {
    if (Test-Path -LiteralPath $planPath) {
        throw "Refusing to replace existing matrix plan: $planPath"
    }
    $armPlan = [ordered]@{}
    foreach ($name in $AllArms) {
        $armPlan[$name] = [ordered]@{
            state = 'unavailable'
            arm_nonce_hex = New-HexNonce
            endpoint_descriptor_paths = @()
            action_receipt_path = $null
            reason = if ($name -eq 'M3') { 'second-physical-machine endpoint pair is unavailable' } elseif ($name -like 'M*') { 'two passive multiplayer endpoint bundles are not registered' } else { 'two passive lifecycle endpoint bundles are not registered' }
        }
    }
    $plan = [ordered]@{
        schema = $PlanSchema
        pinned_executable_sha256 = $PinnedExeSha256
        matrix_nonce_hex = New-HexNonce
        contract_path = $resolvedContract
        contract_sha256 = $contractSha256
        created_filetime_utc = [DateTime]::UtcNow.ToFileTimeUtc()
        arms = $armPlan
    }
    Write-AtomicJson $planPath $plan
    Write-Output $planPath
    return
}

if (-not (Test-Path -LiteralPath $planPath)) {
    throw "Matrix plan does not exist. Run -Mode Plan first: $planPath"
}
$plan = Get-Content -LiteralPath $planPath -Raw | ConvertFrom-Json
if ($plan.schema -ne $PlanSchema -or $plan.contract_sha256 -ne $contractSha256) {
    throw 'Matrix plan does not bind the current frozen contract.'
}

if ($Mode -eq 'RegisterObservedArm') {
    if ([string]::IsNullOrWhiteSpace($PreconditionPath) -or -not (Test-Path -LiteralPath $PreconditionPath -PathType Leaf)) {
        throw 'RegisterObservedArm requires a content-addressed arm precondition.'
    }
    if ($EndpointDescriptorPaths.Count -ne 2) {
        throw 'RegisterObservedArm requires exactly two finalized passive endpoint descriptors.'
    }
    $registrar = Join-Path $PSScriptRoot 'Invoke-Cid1PassiveArmRegistration.ps1'
    $armRoot = Join-Path (Join-Path $resolvedOutput 'registrations') $Arm
    & $registrar -Arm $Arm -MatrixPlanPath $planPath -EndpointDescriptorPaths $EndpointDescriptorPaths -PreconditionPath $PreconditionPath -OutputRoot $armRoot
    if (-not $?) { throw "CID1 $Arm passive registration failed." }
    return
}

if ($Mode -eq 'RunLegacySDArm') {
    if ($SDArms -notcontains $Arm) {
        throw 'RunLegacySDArm accepts only S1-S4 or D1-D2. Multiplayer arms use Invoke-Cid1MultiplayerCoordinator.ps1.'
    }
    if (-not $AllowLiveLifecycleActions) {
        throw 'Legacy client-driving actions are isolated and fail-closed. Pass -AllowLiveLifecycleActions only for an explicit non-production harness run.'
    }
    if ([string]::IsNullOrWhiteSpace($ActionDriverPath)) {
        $ActionDriverPath = Join-Path $PSScriptRoot 'Invoke-Cid1SdActionDriver.ps1'
    }
    if (-not (Test-Path -LiteralPath $ActionDriverPath -PathType Leaf)) {
        throw 'A real owner action driver is required; the orchestrator never fabricates game actions.'
    }
    if ([string]::IsNullOrWhiteSpace($OwnerProfilePath) -or -not (Test-Path -LiteralPath $OwnerProfilePath -PathType Leaf)) {
        throw 'RunLegacySDArm requires a content-addressed autonomous owner profile.'
    }
    $driver = (Resolve-Path -LiteralPath $ActionDriverPath).Path
    $armRoot = Join-Path $resolvedOutput $Arm
    [System.IO.Directory]::CreateDirectory($armRoot) | Out-Null
    $expectedRoles = @($contract.arms.$Arm.roles)
    if ($EndpointDescriptorPaths.Count -eq 0) {
        $EndpointDescriptorPaths = @($expectedRoles | ForEach-Object { Join-Path $armRoot ("endpoint_$_\endpoint_descriptor.json") })
    }
    if ($EndpointDescriptorPaths.Count -ne 2) {
        throw 'Every S/D arm requires exactly two endpoint descriptor targets.'
    }
    $resolvedDescriptors = @()
    foreach ($descriptor in $EndpointDescriptorPaths) {
        $target = [IO.Path]::GetFullPath($descriptor)
        if (Test-Path -LiteralPath $target) { throw "Refusing to reuse endpoint descriptor target: $target" }
        $resolvedDescriptors += $target
    }
    $requestPath = Join-Path $armRoot 'arm_request.json'
    $receiptPath = Join-Path $armRoot 'arm_action_receipt.json'
    $request = [ordered]@{
        schema = $RequestSchema
        arm = $Arm
        matrix_nonce_hex = $plan.matrix_nonce_hex
        arm_nonce_hex = $plan.arms.$Arm.arm_nonce_hex
        pinned_executable_sha256 = $PinnedExeSha256
        contract_path = $resolvedContract
        contract_sha256 = $contractSha256
        action_driver_path = $driver
        action_driver_sha256 = Get-Sha256 $driver
        owner_profile_path = (Resolve-Path -LiteralPath $OwnerProfilePath).Path
        owner_profile_sha256 = Get-Sha256 $OwnerProfilePath
        endpoint_descriptor_paths = $resolvedDescriptors
        output_root = $armRoot
    }
    Write-AtomicJson $requestPath $request
    & $driver -RequestPath $requestPath -ReceiptPath $receiptPath -EnableLegacyClientDriving
    $driverSucceeded = $?
    if (-not $driverSucceeded -or -not (Test-Path -LiteralPath $receiptPath)) {
        throw "CID1 $Arm action driver failed or emitted no receipt."
    }
    $receipt = Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
    if ($receipt.schema -ne 'battle-plan.campaign_identity_arm_action_receipt.v1' -or $receipt.arm -ne $Arm -or $receipt.status -ne 'passed') {
        throw "CID1 $Arm action receipt is not a passing exact receipt."
    }
    foreach ($descriptor in $resolvedDescriptors) {
        if (-not (Test-Path -LiteralPath $descriptor -PathType Leaf)) { throw "CID1 $Arm driver omitted endpoint descriptor: $descriptor" }
    }
    $plan.arms.$Arm.state = 'complete'
    $plan.arms.$Arm.endpoint_descriptor_paths = $resolvedDescriptors
    $plan.arms.$Arm.action_receipt_path = $receiptPath
    $plan.arms.$Arm.reason = $null
    Write-AtomicJson $planPath $plan
    Write-Output $receiptPath
    return
}

if ([string]::IsNullOrWhiteSpace($MatrixReceiptPath)) {
    $MatrixReceiptPath = Join-Path $resolvedOutput 'matrix_receipt.json'
    if (Test-Path -LiteralPath $MatrixReceiptPath) { throw "Refusing to replace matrix receipt: $MatrixReceiptPath" }
    $matrixArms = [ordered]@{}
    foreach ($name in $AllArms) {
        $state = $plan.arms.$name
        if ($Mode -eq 'FinalizeLocal' -and $name -eq 'M3' -and $state.state -ne 'complete') {
            $matrixArms[$name] = [ordered]@{status='blocked_external_second_machine';action_receipt_path='';action_receipt_sha256='';endpoints=@()}
            continue
        }
        if ($state.state -ne 'complete' -or [string]::IsNullOrWhiteSpace([string]$state.action_receipt_path) -or @($state.endpoint_descriptor_paths).Count -ne 2) {
            throw "Finalize requires completed real endpoint/action receipts for arm $name."
        }
        $endpoints = @($state.endpoint_descriptor_paths | ForEach-Object { Get-Content -LiteralPath $_ -Raw | ConvertFrom-Json })
        $matrixArms[$name] = [ordered]@{status='complete';action_receipt_path=[string]$state.action_receipt_path;action_receipt_sha256=Get-Sha256 ([string]$state.action_receipt_path);endpoints=$endpoints}
    }
    Write-AtomicJson $MatrixReceiptPath ([ordered]@{schema='battle-plan.campaign_identity_matrix_receipt.v1';contract_path=$resolvedContract;contract_sha256=$contractSha256;matrix_nonce_hex=[string]$plan.matrix_nonce_hex;created_filetime_utc=[DateTime]::UtcNow.ToFileTimeUtc();arms=$matrixArms})
}
if (-not (Test-Path -LiteralPath $MatrixReceiptPath -PathType Leaf)) {
    throw 'Finalize requires a complete matrix receipt assembled from nine real arm receipts.'
}
if ([string]::IsNullOrWhiteSpace($GateOutPath)) {
    $GateOutPath = Join-Path $resolvedOutput 'matrix_gate.json'
}
$verifier = Join-Path $PSScriptRoot 'verify_campaign_identity_matrix.py'
$verifyArguments = @($verifier,'--contract',$resolvedContract,'--expected-contract-sha256',$contractSha256,'--receipt',$MatrixReceiptPath,'--out',$GateOutPath)
if ($Mode -eq 'FinalizeLocal') { $verifyArguments += '--allow-external-m3-pending' }
& $Python @verifyArguments
if (-not $?) { throw 'CID1 lifecycle matrix regrade failed.' }
Write-Output $GateOutPath
