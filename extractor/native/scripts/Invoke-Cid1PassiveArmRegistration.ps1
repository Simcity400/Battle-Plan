[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)]
    [ValidateSet('S1','S2','S3','S4','D1','D2','M1','M2','M3')]
    [string]$Arm,
    [Parameter(Mandatory=$true)]
    [string]$MatrixPlanPath,
    [Parameter(Mandatory=$true)]
    [string[]]$EndpointDescriptorPaths,
    [Parameter(Mandatory=$true)]
    [string]$PreconditionPath,
    [string]$OutputRoot
)

# Production CID1 evidence registration is deliberately observer-only.  This
# script never starts, stops, focuses, resizes, or sends input to HOI4.  It only
# content-addresses two already-finalized endpoint bundles and lets the
# independent matrix regrader decide whether their relationship proves the arm.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$PinnedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$PlanSchema = 'battle-plan.campaign_identity_matrix_plan.v1'
$RequestSchema = 'battle-plan.campaign_identity_passive_arm_request.v1'
$ProfileSchema = 'battle-plan.campaign_identity_passive_observer_profile.v1'
$ExecutionSchema = 'battle-plan.campaign_identity_passive_arm_execution.v1'
$ActionSchema = 'battle-plan.campaign_identity_arm_action_receipt.v1'
$Roles = @{
    S1 = @('before_save','after_same_process_reload')
    S2 = @('newer_lineage','older_lineage_reload')
    S3 = @('before_relaunch','after_relaunch')
    S4 = @('original_name','renamed_copy')
    D1 = @('new_campaign_a','new_campaign_b')
    D2 = @('campaign_a','campaign_b_after_switch')
    M1 = @('host','hotjoin_client')
    M2 = @('host','observer')
    M3 = @('original_host','rehost')
}
$Operations = @{
    S1='save_reload_same_process'; S2='load_older_same_lineage'
    S3='close_relaunch_same_save'; S4='rename_copy_load'
    D1='create_identical_start_campaign_pair'; D2='switch_campaign_a_to_b_same_process'
    M1='host_hotjoin_same_hour'; M2='host_observer_same_hour'
    M3='cross_machine_rehost_same_save'
}

function Get-Sha256([string]$Path) {
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}
function Write-CreateNewJson([string]$Path,$Value) {
    $full=[IO.Path]::GetFullPath($Path)
    $parent=[IO.Path]::GetDirectoryName($full)
    [IO.Directory]::CreateDirectory($parent)|Out-Null
    if([IO.File]::Exists($full)){throw "Refusing to replace immutable passive CID1 artifact: $full"}
    $temporary=[IO.Path]::Combine($parent,([IO.Path]::GetRandomFileName()+'.tmp'))
    try {
        [IO.File]::WriteAllText($temporary,(($Value|ConvertTo-Json -Depth 30)+"`n"),[Text.UTF8Encoding]::new($false))
        $stream=[IO.File]::Open($temporary,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
        try{$stream.Flush($true)}finally{$stream.Dispose()}
        [IO.File]::Move($temporary,$full)
    } finally { if([IO.File]::Exists($temporary)){[IO.File]::Delete($temporary)} }
}
function Write-AtomicJson([string]$Path,$Value) {
    $full=[IO.Path]::GetFullPath($Path)
    $parent=[IO.Path]::GetDirectoryName($full)
    [IO.Directory]::CreateDirectory($parent)|Out-Null
    $temporary=[IO.Path]::Combine($parent,([IO.Path]::GetRandomFileName()+'.tmp'))
    try {
        [IO.File]::WriteAllText($temporary,(($Value|ConvertTo-Json -Depth 30)+"`n"),[Text.UTF8Encoding]::new($false))
        Move-Item -LiteralPath $temporary -Destination $full -Force
    } finally { if([IO.File]::Exists($temporary)){[IO.File]::Delete($temporary)} }
}

$planFull=(Resolve-Path -LiteralPath $MatrixPlanPath).Path
$plan=Get-Content -LiteralPath $planFull -Raw|ConvertFrom-Json
if($plan.schema-cne$PlanSchema-or$plan.pinned_executable_sha256-cne$PinnedExeSha256){throw 'CID1 matrix plan schema/build mismatch.'}
if($plan.arms.$Arm.state-cne'unavailable'){throw "CID1 $Arm is already registered or has an invalid plan state."}
if($EndpointDescriptorPaths.Count-ne 2){throw 'Passive CID1 arm registration requires exactly two endpoint descriptors.'}

$expectedRoles=@($Roles[$Arm])
$descriptors=[Collections.Generic.List[object]]::new()
$descriptorPaths=[Collections.Generic.List[string]]::new()
$descriptorHashes=[Collections.Generic.List[string]]::new()
for($index=0;$index-lt 2;$index++){
    $path=(Resolve-Path -LiteralPath $EndpointDescriptorPaths[$index]).Path
    $descriptor=Get-Content -LiteralPath $path -Raw|ConvertFrom-Json
    if([string]$descriptor.role-cne$expectedRoles[$index]){throw "CID1 $Arm endpoint role order must be $($expectedRoles -join ', ')."}
    foreach($name in @('transcript_path','gate_path','oracle_path','stream_boundary_path')){
        $hashName=if($name-ceq'transcript_path'){'transcript_file_sha256'}else{$name.Replace('_path','_sha256')}
        $artifact=(Resolve-Path -LiteralPath ([string]$descriptor.$name)).Path
        if((Get-Sha256 $artifact)-cne[string]$descriptor.$hashName){throw "CID1 $Arm endpoint $($expectedRoles[$index]) $name hash mismatch."}
    }
    $descriptorPaths.Add($path)
    $descriptorHashes.Add((Get-Sha256 $path))
    $descriptors.Add($descriptor)
}
if($Arm -ceq 'M3') {
    $leftMachine = [string]$descriptors[0].machine_id
    $rightMachine = [string]$descriptors[1].machine_id
    if([string]::IsNullOrWhiteSpace($leftMachine) -or $leftMachine -ceq $rightMachine) {
        throw 'M3 requires endpoint descriptors from two distinct physical-machine identities.'
    }
}
$preconditionFull=(Resolve-Path -LiteralPath $PreconditionPath).Path
$precondition=Get-Content -LiteralPath $preconditionFull -Raw|ConvertFrom-Json
if($precondition.schema-cne'battle-plan.campaign_identity_arm_precondition.v1'-or$precondition.arm-cne$Arm-or$precondition.executable_sha256-cne$PinnedExeSha256){throw 'CID1 arm precondition schema/arm/build mismatch.'}

if([string]::IsNullOrWhiteSpace($OutputRoot)){$OutputRoot=Join-Path ([IO.Path]::GetDirectoryName($planFull)) $Arm}
$armRoot=[IO.Path]::GetFullPath($OutputRoot)
if(Test-Path -LiteralPath $armRoot){throw "Refusing to reuse passive CID1 arm root: $armRoot"}
[IO.Directory]::CreateDirectory($armRoot)|Out-Null
$driverFull=[IO.Path]::GetFullPath($MyInvocation.MyCommand.Path)
$started=[DateTime]::UtcNow.ToFileTimeUtc()
$requestPath=Join-Path $armRoot 'passive_arm_request.json'
$profilePath=Join-Path $armRoot 'passive_observer_profile.json'
$executionPath=Join-Path $armRoot 'passive_execution_receipt.json'
$actionPath=Join-Path $armRoot 'arm_action_receipt.json'

Write-CreateNewJson $requestPath ([ordered]@{
    schema=$RequestSchema;arm=$Arm;matrix_nonce_hex=[string]$plan.matrix_nonce_hex
    arm_nonce_hex=[string]$plan.arms.$Arm.arm_nonce_hex;pinned_executable_sha256=$PinnedExeSha256
    contract_path=[string]$plan.contract_path;contract_sha256=[string]$plan.contract_sha256
    registration_driver_path=$driverFull;registration_driver_sha256=Get-Sha256 $driverFull
    precondition_path=$preconditionFull;precondition_sha256=Get-Sha256 $preconditionFull
    endpoint_descriptor_paths=@($descriptorPaths);endpoint_descriptor_sha256s=@($descriptorHashes)
    output_root=$armRoot
})
Write-CreateNewJson $profilePath ([ordered]@{
    schema=$ProfileSchema;arm=$Arm;precondition_path=$preconditionFull
    precondition_sha256=Get-Sha256 $preconditionFull;observation_policy='user_drives_client_observer_records_only'
    endpoint_roles=$expectedRoles
})
$steps=[Collections.Generic.List[object]]::new()
for($index=0;$index-lt 2;$index++){
    $steps.Add([ordered]@{kind='register_endpoint';role=$expectedRoles[$index];descriptor_path=$descriptorPaths[$index];descriptor_sha256=$descriptorHashes[$index];transcript_sha256=[string]$descriptors[$index].transcript_sha256})
}
$steps.Add([ordered]@{kind='classify_relationship';classifier='verify_campaign_identity_matrix.py';policy='independent_fail_closed_regrade'})
$completed=[DateTime]::UtcNow.ToFileTimeUtc();if($completed-le$started){$completed=$started+1}
Write-CreateNewJson $executionPath ([ordered]@{
    schema=$ExecutionSchema;arm=$Arm;request_path=$requestPath;request_sha256=Get-Sha256 $requestPath
    owner_profile_path=$profilePath;owner_profile_sha256=Get-Sha256 $profilePath
    started_filetime_utc=$started;completed_filetime_utc=$completed;steps=$steps
})
Write-CreateNewJson $actionPath ([ordered]@{
    schema=$ActionSchema;arm=$Arm;status='passed';operation=$Operations[$Arm];evidence_nonce_hex=[string]$plan.arms.$Arm.arm_nonce_hex
    driver_path=$driverFull;driver_sha256=Get-Sha256 $driverFull
    precondition_path=$preconditionFull;precondition_sha256=Get-Sha256 $preconditionFull
    started_filetime_utc=$started;completed_filetime_utc=$completed;endpoint_roles=$expectedRoles
    endpoint_transcript_sha256s=@($descriptors|ForEach-Object{[string]$_.transcript_sha256})
    request_path=$requestPath;request_sha256=Get-Sha256 $requestPath
    owner_profile_path=$profilePath;owner_profile_sha256=Get-Sha256 $profilePath
    execution_receipt_path=$executionPath;execution_receipt_sha256=Get-Sha256 $executionPath
})

$plan.arms.$Arm.state='complete'
$plan.arms.$Arm.endpoint_descriptor_paths=@($descriptorPaths)
$plan.arms.$Arm.action_receipt_path=$actionPath
$plan.arms.$Arm.reason=$null
Write-AtomicJson $planFull $plan
Write-Output $actionPath
