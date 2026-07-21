[CmdletBinding()]
param(
    [ValidateSet('Plan','RunSDArm','Finalize')]
    [string]$Mode = 'Plan',
    [ValidateSet('S1','S2','S3','S4','D1','D2','M1','M2','M3')]
    [string]$Arm,
    [string]$OutputRoot = 'C:\hoi4_evidence\cid1_lifecycle_matrix',
    [string]$ContractPath = 'extractor\re_evidence\CID1_LIFECYCLE_MATRIX_V1\matrix_contract.json',
    [string]$ActionDriverPath,
    [string[]]$EndpointDescriptorPaths = @(),
    [string]$MatrixReceiptPath,
    [string]$GateOutPath,
    [switch]$AllowLiveLifecycleActions
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$PinnedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$ContractSchema = 'battle-plan.campaign_identity_matrix_contract.v1'
$PlanSchema = 'battle-plan.campaign_identity_matrix_plan.v1'
$RequestSchema = 'battle-plan.campaign_identity_sd_arm_request.v1'
$SDArms = @('S1','S2','S3','S4','D1','D2')
$AllArms = @('S1','S2','S3','S4','D1','D2','M1','M2','M3')

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
            reason = if ($name -like 'M*') { 'two authenticated external endpoints are not registered' } else { 'owner action driver has not executed this arm' }
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

if ($Mode -eq 'RunSDArm') {
    if ($SDArms -notcontains $Arm) {
        throw 'RunSDArm accepts only S1-S4 or D1-D2. Multiplayer arms use Invoke-Cid1MultiplayerCoordinator.ps1.'
    }
    if (-not $AllowLiveLifecycleActions) {
        throw 'Live lifecycle actions are fail-closed. Pass -AllowLiveLifecycleActions only in an owner-authorized isolated run.'
    }
    if ([string]::IsNullOrWhiteSpace($ActionDriverPath) -or -not (Test-Path -LiteralPath $ActionDriverPath -PathType Leaf)) {
        throw 'A real owner action driver is required; the orchestrator never fabricates game actions.'
    }
    if ($EndpointDescriptorPaths.Count -ne 2) {
        throw 'Every S/D arm requires exactly two real endpoint descriptor paths.'
    }
    $resolvedDescriptors = @()
    foreach ($descriptor in $EndpointDescriptorPaths) {
        $resolvedDescriptors += (Resolve-Path -LiteralPath $descriptor).Path
    }
    $driver = (Resolve-Path -LiteralPath $ActionDriverPath).Path
    $armRoot = Join-Path $resolvedOutput $Arm
    [System.IO.Directory]::CreateDirectory($armRoot) | Out-Null
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
        endpoint_descriptor_paths = $resolvedDescriptors
        output_root = $armRoot
    }
    Write-AtomicJson $requestPath $request
    & $driver -RequestPath $requestPath -ReceiptPath $receiptPath
    $driverSucceeded = $?
    if (-not $driverSucceeded -or -not (Test-Path -LiteralPath $receiptPath)) {
        throw "CID1 $Arm action driver failed or emitted no receipt."
    }
    $receipt = Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
    if ($receipt.schema -ne 'battle-plan.campaign_identity_arm_action_receipt.v1' -or $receipt.arm -ne $Arm -or $receipt.status -ne 'passed') {
        throw "CID1 $Arm action receipt is not a passing exact receipt."
    }
    Write-Output $receiptPath
    return
}

if ([string]::IsNullOrWhiteSpace($MatrixReceiptPath) -or -not (Test-Path -LiteralPath $MatrixReceiptPath -PathType Leaf)) {
    throw 'Finalize requires a complete matrix receipt assembled from nine real arm receipts.'
}
if ([string]::IsNullOrWhiteSpace($GateOutPath)) {
    $GateOutPath = Join-Path $resolvedOutput 'matrix_gate.json'
}
$verifier = 'extractor\native\scripts\verify_campaign_identity_matrix.py'
& 'C:\Program Files\Python311\python.exe' $verifier --contract $resolvedContract --expected-contract-sha256 $contractSha256 --receipt $MatrixReceiptPath --out $GateOutPath
if (-not $?) { throw 'CID1 lifecycle matrix regrade failed.' }
Write-Output $GateOutPath
