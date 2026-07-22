[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$RequestPath,
    [Parameter(Mandatory=$true)][string]$ReceiptPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$expectedExeSha256='870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$operations=@{S1='save_reload_same_process';S2='load_older_same_lineage';S3='close_relaunch_same_save';S4='rename_copy_load';D1='create_identical_start_campaign_pair';D2='switch_campaign_a_to_b_same_process'}
$roles=@{S1=@('before_save','after_same_process_reload');S2=@('newer_lineage','older_lineage_reload');S3=@('before_relaunch','after_relaunch');S4=@('original_name','renamed_copy');D1=@('new_campaign_a','new_campaign_b');D2=@('campaign_a','campaign_b_after_switch')}
function Get-Sha256([string]$Path){(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()}
function New-Nonce{$b=[byte[]]::new(32);[Security.Cryptography.RandomNumberGenerator]::Fill($b);[Convert]::ToHexString($b).ToLowerInvariant()}
function Test-LowerNonzeroSha256([string]$Value){$Value-cmatch'^[0-9a-f]{64}$'-and$Value-cnotmatch'^0{64}$'}
function ConvertTo-NativeArgumentLine {
    param([Parameter(Mandatory=$true)][AllowEmptyCollection()][AllowEmptyString()][string[]]$Arguments)
    (($Arguments|ForEach-Object{if($_-eq''){'""'}elseif($_-match'[\s"]'){'"'+($_-replace '(\\*)"','$1$1\"'-replace '(\\+)$','$1$1')+'"'}else{$_}})-join' ') -replace [char]0x1f,''
}
function Write-CreateNewJson([string]$Path,$Value){$full=[IO.Path]::GetFullPath($Path);$parent=[IO.Path]::GetDirectoryName($full);[IO.Directory]::CreateDirectory($parent)|Out-Null;if([IO.File]::Exists($full)){throw "Refusing to replace immutable CID1 action receipt: $full"};$tmp=[IO.Path]::Combine($parent,([IO.Path]::GetRandomFileName()+'.tmp'));try{[IO.File]::WriteAllText($tmp,(($Value|ConvertTo-Json -Depth 30)+"`n"),[Text.UTF8Encoding]::new($false));$s=[IO.File]::Open($tmp,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None);try{$s.Flush($true)}finally{$s.Dispose()};[IO.File]::Move($tmp,$full)}finally{if([IO.File]::Exists($tmp)){[IO.File]::Delete($tmp)}}}
function Write-CreateNewText([string]$Path,[string]$Value){$full=[IO.Path]::GetFullPath($Path);$parent=[IO.Path]::GetDirectoryName($full);[IO.Directory]::CreateDirectory($parent)|Out-Null;if([IO.File]::Exists($full)){throw "Refusing to replace immutable CID1 evidence permit: $full"};$tmp=[IO.Path]::Combine($parent,([IO.Path]::GetRandomFileName()+'.tmp'));try{[IO.File]::WriteAllText($tmp,$Value,[Text.UTF8Encoding]::new($false));$s=[IO.File]::Open($tmp,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None);try{$s.Flush($true)}finally{$s.Dispose()};[IO.File]::Move($tmp,$full)}finally{if([IO.File]::Exists($tmp)){[IO.File]::Delete($tmp)}}}

$requestFull=(Resolve-Path -LiteralPath $RequestPath).Path;$requestSha=Get-Sha256 $requestFull;$request=Get-Content -LiteralPath $requestFull -Raw|ConvertFrom-Json
if($request.schema-cne 'battle-plan.campaign_identity_sd_arm_request.v2'-or-not$operations.ContainsKey([string]$request.arm)-or$request.pinned_executable_sha256-cne $expectedExeSha256){throw 'CID1 S/D action request schema/arm/build mismatch.'}
$driverFull=[IO.Path]::GetFullPath($MyInvocation.MyCommand.Path);$requestDriver=(Resolve-Path -LiteralPath ([string]$request.action_driver_path)).Path
if(-not$requestDriver.Equals($driverFull,[StringComparison]::OrdinalIgnoreCase)-or(Get-Sha256 $driverFull)-cne[string]$request.action_driver_sha256){throw 'CID1 action request does not bind this exact driver.'}
if(-not(Test-LowerNonzeroSha256 ([string]$request.matrix_nonce_hex))-or-not(Test-LowerNonzeroSha256 ([string]$request.arm_nonce_hex))-or$request.matrix_nonce_hex-ceq$request.arm_nonce_hex){throw 'CID1 action request nonces are absent, malformed, or equal.'}
$contractFull=(Resolve-Path -LiteralPath ([string]$request.contract_path)).Path;if((Get-Sha256 $contractFull)-cne[string]$request.contract_sha256){throw 'CID1 action request contract hash mismatch.'}
$arm=[string]$request.arm;$profileFull=(Resolve-Path -LiteralPath ([string]$request.owner_profile_path)).Path
$profileSha=Get-Sha256 $profileFull;if($profileSha-cne [string]$request.owner_profile_sha256){throw 'CID1 owner profile hash mismatch.'}
$profile=Get-Content -LiteralPath $profileFull -Raw|ConvertFrom-Json
if($profile.schema-cne 'battle-plan.campaign_identity_sd_owner_profile.v1'-or$profile.arm-cne $arm-or-not($profile.steps-is[array])){throw 'CID1 owner profile schema/arm/steps mismatch.'}
$preconditionFull=(Resolve-Path -LiteralPath ([string]$profile.precondition_path)).Path
if((Get-Sha256 $preconditionFull)-cne [string]$profile.precondition_sha256){throw 'CID1 owner profile precondition hash mismatch.'}
$exe=(Resolve-Path -LiteralPath ([string]$profile.hoi4_exe)).Path;if((Get-Sha256 $exe)-cne $expectedExeSha256){throw 'CID1 owner profile executable is not exact a729.'}
$endpointOwner=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Invoke-Cid1EndpointOwner.ps1')).Path
$permitPath=[IO.Path]::GetFullPath((Join-Path ([string]$request.output_root) 'cid1_lifecycle_evidence.permit'))
$permitText=@("CID1-LIFECYCLE-EVIDENCE-PERMIT-V1","arm=$arm","matrix_nonce_hex=$([string]$request.matrix_nonce_hex)","arm_nonce_hex=$([string]$request.arm_nonce_hex)","request_sha256=$requestSha","owner_profile_sha256=$profileSha","contract_sha256=$([string]$request.contract_sha256)","pinned_executable_sha256=$expectedExeSha256")-join"`n";$permitText+="`n"
Write-CreateNewText $permitPath $permitText;$permitSha=Get-Sha256 $permitPath
$started=[DateTime]::UtcNow.ToFileTimeUtc();$processes=@{};$owners=@{};$pending=@{};$saves=@{};$steps=[Collections.Generic.List[object]]::new();$notBefore=$started

Add-Type -TypeDefinition @'
using System;using System.Runtime.InteropServices;
public static class Cid1OwnerInput {
 [StructLayout(LayoutKind.Sequential)] public struct RECT{public int Left,Top,Right,Bottom;}
 [StructLayout(LayoutKind.Sequential)] public struct POINT{public int X,Y;}
 [StructLayout(LayoutKind.Sequential)] public struct MI{public int dx,dy;public uint data,flags,time;public UIntPtr extra;}
 [StructLayout(LayoutKind.Sequential)] public struct KI{public ushort vk,scan;public uint flags,time;public UIntPtr extra;}
 [StructLayout(LayoutKind.Explicit)] public struct U{[FieldOffset(0)]public MI mi;[FieldOffset(0)]public KI ki;}
 [StructLayout(LayoutKind.Sequential)] public struct I{public uint type;public U u;}
 [DllImport("user32.dll")]public static extern bool SetForegroundWindow(IntPtr h);[DllImport("user32.dll")]public static extern IntPtr GetForegroundWindow();[DllImport("user32.dll")]public static extern bool GetClientRect(IntPtr h,out RECT r);[DllImport("user32.dll")]public static extern bool ClientToScreen(IntPtr h,ref POINT p);[DllImport("user32.dll")]public static extern bool SetCursorPos(int x,int y);[DllImport("user32.dll")]public static extern uint SendInput(uint n,I[] i,int s);
 public static void Key(ushort vk){I d=new I(),u=new I();d.type=u.type=1;d.u.ki.vk=u.u.ki.vk=vk;u.u.ki.flags=2;if(SendInput(2,new[]{d,u},Marshal.SizeOf(typeof(I)))!=2)throw new Exception("key SendInput failed");}
 public static void Scan(ushort sc){I d=new I(),u=new I();d.type=u.type=1;d.u.ki.scan=u.u.ki.scan=sc;d.u.ki.flags=8;u.u.ki.flags=10;if(SendInput(2,new[]{d,u},Marshal.SizeOf(typeof(I)))!=2)throw new Exception("scan SendInput failed");}
 public static void Text(string s){foreach(char c in s){I d=new I(),u=new I();d.type=u.type=1;d.u.ki.scan=u.u.ki.scan=c;d.u.ki.flags=4;u.u.ki.flags=6;if(SendInput(2,new[]{d,u},Marshal.SizeOf(typeof(I)))!=2)throw new Exception("text SendInput failed");}}
 public static void Click(IntPtr h,int x,int y){POINT p=new POINT();p.X=x;p.Y=y;if(!ClientToScreen(h,ref p)||!SetCursorPos(p.X,p.Y))throw new Exception("CID1 click coordinate conversion failed");I d=new I(),u=new I();d.type=u.type=0;d.u.mi.flags=2;u.u.mi.flags=4;if(SendInput(2,new[]{d,u},Marshal.SizeOf(typeof(I)))!=2)throw new Exception("click SendInput failed");}
}
'@

foreach($step in $profile.steps){
    $kind=[string]$step.kind;$row=[ordered]@{kind=$kind;started_filetime_utc=[DateTime]::UtcNow.ToFileTimeUtc()}
    switch($kind){
        'start_recording_owner' {
            $slot=[string]$step.slot;if($owners.ContainsKey($slot)){throw "Duplicate recording-owner slot: $slot"}
            $path=(Resolve-Path -LiteralPath ([string]$step.path)).Path;if((Get-Sha256 $path)-cne [string]$step.sha256){throw "Recording owner hash mismatch: $slot"}
            $stdout=Join-Path ([string]$request.output_root) ("owner_${slot}.stdout.log");$stderr=Join-Path ([string]$request.output_root) ("owner_${slot}.stderr.log")
            if((Get-Sha256 $requestFull)-cne$requestSha-or(Get-Sha256 $profileFull)-cne$profileSha-or(Get-Sha256 $permitPath)-cne$permitSha){throw 'CID1 evidence permit inputs changed before recording-owner launch.'}
            $ownerArguments=@($step.arguments|ForEach-Object{[string]$_});$permitEnvironment=[ordered]@{BATTLE_PLAN_CID1_LIFECYCLE_EVIDENCE='1';BATTLE_PLAN_CID1_LIFECYCLE_EVIDENCE_PERMIT_PATH=$permitPath;BATTLE_PLAN_CID1_LIFECYCLE_EVIDENCE_PERMIT_SHA256=$permitSha};$previousEnvironment=@{}
            try{foreach($entry in $permitEnvironment.GetEnumerator()){$previousEnvironment[$entry.Key]=[Environment]::GetEnvironmentVariable($entry.Key,[EnvironmentVariableTarget]::Process);[Environment]::SetEnvironmentVariable($entry.Key,[string]$entry.Value,[EnvironmentVariableTarget]::Process)};$proc=Start-Process -FilePath $path -ArgumentList (ConvertTo-NativeArgumentLine $ownerArguments) -PassThru -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr}finally{foreach($entry in $permitEnvironment.GetEnumerator()){[Environment]::SetEnvironmentVariable($entry.Key,$previousEnvironment[$entry.Key],[EnvironmentVariableTarget]::Process)}}
            $owners[$slot]=$proc;$row.slot=$slot;$row.process_id=$proc.Id;$row.executable_sha256=[string]$step.sha256;$row.arguments_sha256=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes((@($step.arguments)-join[char]0)))).ToLowerInvariant();$row.evidence_permit_path=$permitPath;$row.evidence_permit_sha256=$permitSha
        }
        'discover_exact_process' {
            $slot=[string]$step.slot;if($processes.ContainsKey($slot)){throw "Duplicate game-process slot: $slot"};$deadline=[DateTime]::UtcNow.AddSeconds([int]$step.timeout_seconds);$found=$null
            $cutoff=if([bool]$step.allow_existing){0}else{$notBefore}
            while([DateTime]::UtcNow-lt$deadline-and$null-eq$found){$matches=@();foreach($p in @(Get-Process -Name the game -ErrorAction SilentlyContinue)){try{if($p.StartTime.ToFileTimeUtc()-lt$cutoff-or$p.MainWindowHandle-eq[IntPtr]::Zero){continue};if(-not[IO.Path]::GetFullPath([string]$p.MainModule.FileName).Equals($exe,[StringComparison]::OrdinalIgnoreCase)){continue};if((Get-Sha256 $p.MainModule.FileName)-cne$expectedExeSha256){continue};$matches+=$p}catch{}};if($matches.Count-gt 1){throw "Ambiguous exact-a729 process discovery for slot $slot"};if($matches.Count-eq 1){$found=$matches[0]}else{Start-Sleep -Milliseconds 250}}
            if($null-eq$found){throw "No exact-a729 process discovered for slot $slot"};$processes[$slot]=$found;$row.slot=$slot;$row.process_id=$found.Id;$row.process_creation_filetime_utc=$found.StartTime.ToFileTimeUtc()
        }
        'bind_save' {
            $slot=[string]$step.slot;if($saves.ContainsKey($slot)){throw "Duplicate save slot: $slot"};$save=(Resolve-Path -LiteralPath ([string]$step.path)).Path;$sha=Get-Sha256 $save;if($sha-cne[string]$step.sha256){throw "Save binding hash mismatch: $slot"};$saves[$slot]=[ordered]@{path=$save;sha256=$sha};$row.slot=$slot;$row.path=$save;$row.sha256=$sha
        }
        'copy_save_create_new' {
            $slot=[string]$step.slot;if($saves.ContainsKey($slot)){throw "Duplicate save slot: $slot"};$source=(Resolve-Path -LiteralPath ([string]$step.source_path)).Path;$sha=Get-Sha256 $source;if($sha-cne[string]$step.source_sha256){throw 'S4 source save hash mismatch.'};$target=[IO.Path]::GetFullPath([string]$step.target_path);if(Test-Path -LiteralPath $target){throw "Refusing to replace S4 copy target: $target"};[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))|Out-Null;Copy-Item -LiteralPath $source -Destination $target -ErrorAction Stop;if((Get-Sha256 $target)-cne$sha){throw 'S4 copied save differs from source bytes.'};$saves[$slot]=[ordered]@{path=$target;sha256=$sha};$row.slot=$slot;$row.source_path=$source;$row.target_path=$target;$row.sha256=$sha
        }
        'discover_created_save' {
            $slot=[string]$step.slot;if($saves.ContainsKey($slot)){throw "Duplicate save slot: $slot"};$saveRoot=(Resolve-Path -LiteralPath ([string]$step.save_root)).Path;$deadline=[DateTime]::UtcNow.AddSeconds([int]$step.timeout_seconds);$found=$null
            while([DateTime]::UtcNow-lt$deadline-and$null-eq$found){$candidates=@(Get-ChildItem -LiteralPath $saveRoot -File -Filter '*.hoi4'|Where-Object{$_.LastWriteTimeUtc.ToFileTimeUtc()-ge$started-and$_.Name-cmatch[string]$step.filename_regex});if($candidates.Count-gt1){throw "Created-save discovery is ambiguous for slot $slot"};if($candidates.Count-eq1){$candidate=$candidates[0];$length=$candidate.Length;$sha=Get-Sha256 $candidate;Start-Sleep -Milliseconds ([int]$step.stable_milliseconds);$again=Get-Item -LiteralPath $candidate.FullName;if($again.Length-eq$length-and(Get-Sha256 $again.FullName)-ceq$sha){$found=$again}}else{Start-Sleep -Milliseconds 250}}
            if($null-eq$found){throw "No stable newly created save discovered for slot $slot"};$saves[$slot]=[ordered]@{path=$found.FullName;sha256=Get-Sha256 $found.FullName};$row.slot=$slot;$row.path=$found.FullName;$row.sha256=$saves[$slot].sha256
        }
        'observe_endpoint' {
            $role=[string]$step.role;if(@($roles[$arm])-cnotcontains$role){throw "Unexpected $arm endpoint role: $role"};$p=$processes[[string]$step.process_slot];if($null-eq$p){throw "Unknown process slot: $($step.process_slot)"}
            $saveSlot=if($step.PSObject.Properties.Name-contains'save_slot'){[string]$step.save_slot}else{''};if(-not[string]::IsNullOrWhiteSpace($saveSlot)){$saveBinding=$saves[$saveSlot];if($null-eq$saveBinding){throw "Unknown save slot: $saveSlot"};$save=[string]$saveBinding.path;$saveSha=[string]$saveBinding.sha256}else{$save=(Resolve-Path -LiteralPath ([string]$step.save_path)).Path;$saveSha=[string]$step.save_sha256};if((Get-Sha256 $save)-cne$saveSha){throw "Endpoint save hash mismatch: $role"};$endpointRoot=Join-Path ([string]$request.output_root) ("endpoint_"+$role)
            $arguments=@('-NoProfile','-ExecutionPolicy','Bypass','-File',$endpointOwner,'-Mode','Observe','-EndpointRoot',$endpointRoot,'-Role',$role,'-ProcessId',[string]$p.Id,'-SavePath',$save,'-StaticDisasmPath',(Resolve-Path -LiteralPath ([string]$profile.static_disasm_path)).Path,'-Python',(Resolve-Path -LiteralPath ([string]$profile.python_path)).Path)
            if(-not[string]::IsNullOrWhiteSpace([string]$profile.rakaly_path)){$arguments+=@('-RakalyPath',(Resolve-Path -LiteralPath ([string]$profile.rakaly_path)).Path)}
            & powershell.exe @arguments;if($LASTEXITCODE-ne 0){throw "Endpoint observation failed: $role"};$pending[$role]=Join-Path $endpointRoot 'endpoint_pending.json';$row.role=$role;$row.pending_sha256=Get-Sha256 $pending[$role]
        }
        'window_input' {
            $p=$processes[[string]$step.process_slot];if($null-eq$p-or$p.HasExited){throw 'Window input process is absent.'};$window=$p.MainWindowHandle;if($window-eq[IntPtr]::Zero){throw 'Window input process has no window.'};$rect=[Cid1OwnerInput+RECT]::new();if(-not[Cid1OwnerInput]::GetClientRect($window,[ref]$rect)){throw 'GetClientRect failed.'};$w=$rect.Right-$rect.Left;$h=$rect.Bottom-$rect.Top;if($w-ne[int]$step.expected_client_width-or$h-ne[int]$step.expected_client_height){throw "CID1 profile/window size mismatch: ${w}x${h}"};if(-not[Cid1OwnerInput]::SetForegroundWindow($window)){throw 'SetForegroundWindow failed.'};Start-Sleep -Milliseconds 250;if([Cid1OwnerInput]::GetForegroundWindow()-ne$window){throw 'Authenticated CID1 window did not retain foreground ownership.'}
            switch([string]$step.input_kind){'key'{[Cid1OwnerInput]::Key([uint16]$step.value)}'scan'{[Cid1OwnerInput]::Scan([uint16]$step.value)}'text'{[Cid1OwnerInput]::Text([string]$step.value)}'click'{[Cid1OwnerInput]::Click($window,[int]$step.x,[int]$step.y)}default{throw "Unknown CID1 input kind: $($step.input_kind)"}};Start-Sleep -Milliseconds ([int]$step.settle_milliseconds);$row.process_id=$p.Id;$row.input_kind=[string]$step.input_kind
        }
        'wait' { Start-Sleep -Milliseconds ([int]$step.milliseconds) }
        'wait_for_paused_hour' {
            $p=$processes[[string]$step.process_slot];if($null-eq$p-or$p.HasExited){throw 'Paused-hour process is absent.'};$helper=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\..\replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_wait_for_paused_save.ps1')).Path;$readyReceipt=Join-Path ([string]$request.output_root) ("readiness_"+[string]$step.receipt_label+'.json');& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $helper -ProcessId $p.Id -ReceiptPath $readyReceipt -ExpectedGameHourRaw ([int]$step.game_hour_raw) -TimeoutSeconds ([int]$step.timeout_seconds) -StableSamples ([int]$step.stable_samples) -PollMilliseconds ([int]$step.poll_milliseconds);if($LASTEXITCODE-ne0){throw "Exact paused-hour readiness failed: $($step.receipt_label)"};$row.process_id=$p.Id;$row.game_hour_raw=[int]$step.game_hour_raw;$row.readiness_receipt_path=$readyReceipt;$row.readiness_receipt_sha256=Get-Sha256 $readyReceipt
        }
        'wait_recording_owner' { $o=$owners[[string]$step.slot];if($null-eq$o){throw "Unknown owner slot: $($step.slot)"};if(-not$o.WaitForExit([int]$step.timeout_seconds*1000)){throw "Recording owner timeout: $($step.slot)"};if($o.ExitCode-ne 0){throw "Recording owner failed: $($step.slot) exit=$($o.ExitCode)"};$row.exit_code=$o.ExitCode }
        'wait_process_exit' { $p=$processes[[string]$step.process_slot];if($null-eq$p){throw 'Unknown process slot.'};if(-not$p.WaitForExit([int]$step.timeout_seconds*1000)){throw 'Exact game process did not exit before deadline.'} }
        'terminate_owned_process' { $p=$processes[[string]$step.process_slot];if($null-eq$p-or$p.HasExited){throw 'Owned process is absent/already exited.'};$start=$p.StartTime.ToFileTimeUtc();$path=[string]$p.MainModule.FileName;if((Get-Sha256 $path)-cne$expectedExeSha256){throw 'Owned process identity drifted; refusing termination.'};Stop-Process -Id $p.Id -Force -ErrorAction Stop;$p.WaitForExit(30000);$row.process_id=$p.Id;$row.process_creation_filetime_utc=$start }
        'finalize_endpoint' {
            $role=[string]$step.role;if(-not$pending.ContainsKey($role)){throw "No pending endpoint for role $role"};$set=(Resolve-Path -LiteralPath ([string]$step.rollover_set_descriptor_path)).Path;$endpointRoot=[IO.Path]::GetDirectoryName([string]$pending[$role]);& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $endpointOwner -Mode Finalize -EndpointRoot $endpointRoot -Role $role -RolloverSetDescriptorPath $set -LogicalStreamId ([int]$step.logical_stream_id) -Python (Resolve-Path -LiteralPath ([string]$profile.python_path)).Path;if($LASTEXITCODE-ne 0){throw "Endpoint finalization failed: $role"};$row.role=$role;$row.descriptor_sha256=Get-Sha256 (Join-Path $endpointRoot 'endpoint_descriptor.json')
        }
        default { throw "Unknown CID1 owner step kind: $kind" }
    }
    $row.completed_filetime_utc=[DateTime]::UtcNow.ToFileTimeUtc();$steps.Add($row)
}

$descriptorPaths=@($request.endpoint_descriptor_paths);if($descriptorPaths.Count-ne 2){throw 'CID1 request did not carry two descriptor targets.'};$transcriptHashes=[Collections.Generic.List[string]]::new()
for($i=0;$i-lt 2;$i++){$expectedRole=@($roles[$arm])[$i];$actual=Join-Path ([string]$request.output_root) ("endpoint_${expectedRole}\endpoint_descriptor.json");if(-not(Test-Path -LiteralPath $actual -PathType Leaf)){throw "Missing finalized endpoint descriptor: $expectedRole"};$target=[IO.Path]::GetFullPath([string]$descriptorPaths[$i]);if(-not$target.Equals([IO.Path]::GetFullPath($actual),[StringComparison]::OrdinalIgnoreCase)){throw "Endpoint target path differs from owner path for $expectedRole"};$d=Get-Content -LiteralPath $actual -Raw|ConvertFrom-Json;if($d.role-cne$expectedRole){throw 'Endpoint role mismatch after finalization.'};$transcriptHashes.Add([string]$d.transcript_sha256)}
$completed=[DateTime]::UtcNow.ToFileTimeUtc();if($completed-le$started){$completed=$started+1}
$executionPath=Join-Path ([string]$request.output_root) 'owner_execution_receipt.json'
if((Get-Sha256 $requestFull)-cne$requestSha-or(Get-Sha256 $profileFull)-cne$profileSha-or(Get-Sha256 $permitPath)-cne$permitSha){throw 'CID1 evidence permit inputs changed during owner execution.'}
Write-CreateNewJson $executionPath ([ordered]@{schema='battle-plan.campaign_identity_sd_owner_execution.v1';arm=$arm;request_path=$requestFull;request_sha256=$requestSha;owner_profile_path=$profileFull;owner_profile_sha256=$profileSha;evidence_permit_path=$permitPath;evidence_permit_sha256=$permitSha;started_filetime_utc=$started;completed_filetime_utc=$completed;steps=$steps})
Write-CreateNewJson $ReceiptPath ([ordered]@{schema='battle-plan.campaign_identity_arm_action_receipt.v1';arm=$arm;status='passed';operation=$operations[$arm];evidence_nonce_hex=New-Nonce;driver_path=$MyInvocation.MyCommand.Path;driver_sha256=Get-Sha256 $MyInvocation.MyCommand.Path;precondition_path=$preconditionFull;precondition_sha256=Get-Sha256 $preconditionFull;started_filetime_utc=$started;completed_filetime_utc=$completed;endpoint_roles=@($roles[$arm]);endpoint_transcript_sha256s=@($transcriptHashes);request_path=$requestFull;request_sha256=Get-Sha256 $requestFull;owner_profile_path=$profileFull;owner_profile_sha256=Get-Sha256 $profileFull;execution_receipt_path=$executionPath;execution_receipt_sha256=Get-Sha256 $executionPath})
Write-Output $ReceiptPath
