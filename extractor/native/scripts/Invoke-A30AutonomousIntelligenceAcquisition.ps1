[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $RecorderExe,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $PdxUserDir,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [Parameter(Mandatory = $true)] [string] $ContinueSave,
    [Parameter(Mandatory = $true)] [string] $StaticBundleManifest,
    [Parameter(Mandatory = $true)] [string] $StaticBundleBindingsDir,
    [string] $PrebuiltArtifactAttestationPath = '',
    [string] $ActionProfilePath = '',
    [int] $DurationSeconds = 600,
    [int] $SampleMilliseconds = 25,
    [int] $ProcessTimeoutSeconds = 300,
    [int] $InitialSettleSeconds = 75,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$expectedSaveSha256 = 'c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735'
if($DurationSeconds-lt300-or$DurationSeconds-gt1800){throw 'DurationSeconds must be in 300..1800.'}
if($SampleMilliseconds-lt10-or$SampleMilliseconds-gt1000){throw 'SampleMilliseconds must be in 10..1000.'}
if($ProcessTimeoutSeconds-lt30-or$ProcessTimeoutSeconds-gt600){throw 'ProcessTimeoutSeconds must be in 30..600.'}
foreach($path in @($RecorderExe,$Hoi4Exe,$PdxUserDir,$CollectorDll,$ContinueSave,$StaticBundleManifest,$StaticBundleBindingsDir)){if(-not(Test-Path -LiteralPath $path)){throw "Required path does not exist: $path"}}

$recorder=(Resolve-Path -LiteralPath $RecorderExe).Path;$exe=(Resolve-Path -LiteralPath $Hoi4Exe).Path;$user=(Resolve-Path -LiteralPath $PdxUserDir).Path;$dll=(Resolve-Path -LiteralPath $CollectorDll).Path;$save=(Resolve-Path -LiteralPath $ContinueSave).Path
$bundleManifest=(Resolve-Path -LiteralPath $StaticBundleManifest).Path;$bindings=(Resolve-Path -LiteralPath $StaticBundleBindingsDir).Path;$bundle=Split-Path -Parent $bundleManifest;$out=[IO.Path]::GetFullPath($OutDir)
$pythonExe=(Get-Command $Python -CommandType Application -ErrorAction Stop).Source
$repo=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\..')).Path
$productionDriver=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1')).Path
$attestationHelpers=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1')).Path
$staticVerifier=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\game_defs\static_bundle.py')).Path
$actionDriver=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Invoke-A30AutonomousIntelligenceActions.ps1')).Path
$verifier=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'verify_a30_autonomous_acquisition.py')).Path
if([string]::IsNullOrWhiteSpace($ActionProfilePath)){$ActionProfilePath=Join-Path $repo 'extractor\documents\dossiers\A30\A30_AUTONOMOUS_ACTIONS_1920X1080.json'}
$profile=(Resolve-Path -LiteralPath $ActionProfilePath).Path
if([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)){$PrebuiltArtifactAttestationPath=Join-Path $repo '.cargo-target\release\hoi4-production-prebuilt-artifacts.json'}
. $attestationHelpers
$prebuilt=Test-ProductionArtifactAttestation -RepoRoot $repo -ReceiptPath $PrebuiltArtifactAttestationPath -ExpectedDriverPath $productionDriver
if(-not[string]::Equals($recorder,[string]$prebuilt.artifacts.recorder_host,[StringComparison]::OrdinalIgnoreCase)-or-not[string]::Equals($dll,[string]$prebuilt.artifacts.collector_dll,[StringComparison]::OrdinalIgnoreCase)){throw 'A30 requires the recorder and collector from the central Release attestation.'}
if((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()-cne$expectedExeSha256){throw 'A30 executable is not exact a729.'}
if((Get-FileHash -LiteralPath $save -Algorithm SHA256).Hash.ToLowerInvariant()-cne$expectedSaveSha256){throw 'A30 requires the frozen New D04 Save Deep War identity.'}
$saveDir=[IO.Path]::GetFullPath((Join-Path $user 'save games')).TrimEnd([char]'\');$requestedDir=[IO.Path]::GetFullPath((Split-Path -Parent $save)).TrimEnd([char]'\')
if(-not[string]::Equals($saveDir,$requestedDir,[StringComparison]::OrdinalIgnoreCase)){throw "ContinueSave must be a direct child of $saveDir"}
$continueIndex=[Array]::IndexOf($EarlyLaunchArgument,'--continuelastsave');$gdprIndex=[Array]::IndexOf($EarlyLaunchArgument,'--gdpr-compliant');if($continueIndex-lt0-or$gdprIndex-lt$continueIndex){throw 'EarlyLaunchArgument must contain --continuelastsave followed by --gdpr-compliant.'}
if(@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count-ne0){throw 'A30 requires a clean early launch.'}
if(Test-Path -LiteralPath $out){if(-not(Test-Path -LiteralPath $out -PathType Container)-or@(Get-ChildItem -LiteralPath $out -Force).Count-ne0){throw "OutDir must be absent or empty: $out"}}else{[IO.Directory]::CreateDirectory($out)|Out-Null}

function Write-AtomicBytes([string]$Path,[byte[]]$Bytes,[switch]$AllowReplace){$full=[IO.Path]::GetFullPath($Path);$parent=[IO.Path]::GetDirectoryName($full);[IO.Directory]::CreateDirectory($parent)|Out-Null;$temp=[IO.Path]::Combine($parent,([IO.Path]::GetRandomFileName()+'.tmp'));try{[IO.File]::WriteAllBytes($temp,$Bytes);$stream=[IO.File]::Open($temp,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None);try{$stream.Flush($true)}finally{$stream.Dispose()};if([IO.File]::Exists($full)){if(-not$AllowReplace){throw "Refusing to replace $full"};[IO.File]::Replace($temp,$full,$null,$true)}else{[IO.File]::Move($temp,$full)}}finally{if([IO.File]::Exists($temp)){[IO.File]::Delete($temp)}}}
function Write-AtomicText([string]$Path,[string]$Text,[switch]$AllowReplace){Write-AtomicBytes $Path ([Text.UTF8Encoding]::new($false).GetBytes($Text)) -AllowReplace:$AllowReplace}

$bindingPaths=[ordered]@{mod_state=Join-Path $bindings 'mod_state.json';dlc_state=Join-Path $bindings 'dlc_state.json';load_order=Join-Path $bindings 'load_order.json';source_inventory=Join-Path $bindings 'source_inventory.json';resolution_report=Join-Path $bindings 'resolution_report.json'}
foreach($key in @($bindingPaths.Keys)){if(-not(Test-Path -LiteralPath $bindingPaths[$key] -PathType Leaf)){throw "A30 binding $key is missing."};$bindingPaths[$key]=(Resolve-Path -LiteralPath $bindingPaths[$key]).Path}
$preflightOutput=@(& $pythonExe $staticVerifier verify --bundle $bundle --exe $exe --mod-state $bindingPaths.mod_state --dlc-state $bindingPaths.dlc_state --load-order $bindingPaths.load_order --source-inventory $bindingPaths.source_inventory --live-resolution-report $bindingPaths.resolution_report --allow-deferred-token-gate)
if($LASTEXITCODE-ne0){throw 'A30 static-bundle preflight failed.'};$preflightText=$preflightOutput-join[Environment]::NewLine;$preflight=$preflightText|ConvertFrom-Json;if($preflight.passed-ne$true-or$preflight.verification_phase-cne'pre_injection_deferred_allowed'){throw 'A30 static-bundle preflight did not admit deferred injection.'};Write-AtomicText (Join-Path $out 'a30_static_bundle_preflight.json') ($preflightText+[Environment]::NewLine)

$continuePath=Join-Path $user 'continue_game.json';$priorExists=Test-Path -LiteralPath $continuePath -PathType Leaf;$priorBytes=if($priorExists){[IO.File]::ReadAllBytes($continuePath)}else{$null};$continue=if($priorExists){Get-Content -LiteralPath $continuePath -Raw|ConvertFrom-Json}else{[pscustomobject]@{title=[IO.Path]::GetFileNameWithoutExtension($save);desc='';date=[DateTime]::UtcNow.ToString('R');filename='';is_remote=$false}};$continue.filename=[IO.Path]::GetFileName($save);$continue.is_remote=$false;$continueText=($continue|ConvertTo-Json -Depth 8)+[Environment]::NewLine
$sessionId=[guid]::NewGuid().ToString();$captureId=[guid]::NewGuid().ToString();$nonceMaterial=[guid]::NewGuid().ToString('N');$nonceBytes=[Text.Encoding]::UTF8.GetBytes("$sessionId`:$captureId`:$nonceMaterial");$nonceSha256=([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($nonceBytes))).Replace('-','').ToLowerInvariant();$selectedDir=Join-Path $out 'a30_selected_present_receipts';$controllerReceipt=Join-Path $out 'a30_controller_evidence.json';$actionReceipt=Join-Path $out 'a30_autonomous_actions.json';$verification=Join-Path $out 'a30_autonomous_production_acquisition.json';$manifest=Join-Path $out 'native_session_manifest.json'
$arguments=@('--early-launch-exe',$exe,'--pdx-user-dir',$user,'--dll',$dll,'--out-dir',$out,'--session-id',$sessionId,'--capture-id',$captureId,'--duration-s',[string]$DurationSeconds,'--sample-ms',[string]$SampleMilliseconds,'--min-game-hours','0','--stream-format','compact','--profile','debug','--scheduled-domain','diplomacy_intel_operatives_live','--scheduled-domains-exact','--static-bundle-manifest',$bundleManifest,'--static-bundle-mod-state',$bindingPaths.mod_state,'--static-bundle-dlc-state',$bindingPaths.dlc_state,'--static-bundle-load-order',$bindingPaths.load_order,'--static-bundle-source-inventory',$bindingPaths.source_inventory,'--static-bundle-live-resolution-report',$bindingPaths.resolution_report,'--static-bundle-verifier',$staticVerifier,'--static-bundle-python',$pythonExe,'--static-bundle-timeout-ms','300000','--sidecar-log','--hour-latch')
foreach($value in $EarlyLaunchArgument){$arguments+=@('--early-launch-arg',$value)}

$job=$null;$process=$null;$startUtc=$null;$overridden=$false;$actionError=$null
try{
    $overridden=$true;Write-AtomicText $continuePath $continueText -AllowReplace
    $job=Start-Job -ScriptBlock {param($Recorder,$Args,$EvidenceRoot,$SelectedRoot,$ControllerPath,$SessionId,$CaptureId,$NonceSha256);$env:BATTLE_PLAN_ENABLE_A30_OPERATIVE_OPERATION_EVIDENCE='1';$env:BATTLE_PLAN_ENABLE_A30_CONTROLLER_EVIDENCE='1';$env:BATTLE_PLAN_A30_EVIDENCE_OUT_DIR=$EvidenceRoot;$env:BATTLE_PLAN_A30_SELECTED_PRESENT_OUT_DIR=$SelectedRoot;$env:BATTLE_PLAN_A30_CONTROLLER_RECEIPT_PATH=$ControllerPath;$env:BATTLE_PLAN_A30_SESSION_ID=$SessionId;$env:BATTLE_PLAN_A30_CAPTURE_ID=$CaptureId;$env:BATTLE_PLAN_A30_NONCE_SHA256=$NonceSha256;$env:BATTLE_PLAN_A30_OWNER_TAG_DB_ID='368';$env:BATTLE_PLAN_A30_EXPECTED_GAME_HOUR_RAW='60830441';& $Recorder @Args;if($LASTEXITCODE-ne0){throw "collector_record exited with code $LASTEXITCODE"}} -ArgumentList $recorder,(,$arguments),$out,$selectedDir,$controllerReceipt,$sessionId,$captureId,$nonceSha256
    $deadline=[DateTime]::UtcNow.AddSeconds($ProcessTimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline -and $null -eq $process) {
        if ($job.State -in @('Failed','Stopped','Disconnected')) {
            throw "collector_record ended before exact-a729 exposed a window: $($job.State)"
        }
        foreach ($candidate in @(Get-Process -Name the game -ErrorAction SilentlyContinue)) {
            try {
                if ([string]::Equals($candidate.MainModule.FileName,$exe,[StringComparison]::OrdinalIgnoreCase) -and
                    $candidate.MainWindowHandle -ne [IntPtr]::Zero -and
                    (Get-FileHash -LiteralPath $candidate.MainModule.FileName -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $expectedExeSha256) {
                    $process=$candidate
                    break
                }
            } catch { }
        }
        if ($null -eq $process) { Start-Sleep -Milliseconds 500 }
    }
    if($null-eq$process){throw 'A30 exact-a729 window was not observed.'};$startUtc=$process.StartTime.ToUniversalTime().ToString('o')
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $actionDriver -ProcessId $process.Id -ExpectedProcessStartUtc $startUtc -Hoi4Exe $exe -CollectorDll $dll -ProfilePath $profile -SelectedPresentDirectory $selectedDir -ReceiptPath $actionReceipt -InitialSettleSeconds $InitialSettleSeconds -SelectedPresentTimeoutSeconds 90
    if($LASTEXITCODE-ne0){throw "A30 autonomous actions failed with code $LASTEXITCODE"}
    Wait-Job -Job $job | Out-Null
    $state = $job.State
    Receive-Job -Job $job -ErrorAction Continue | ForEach-Object { Write-Verbose ([string]$_) }
    if ($state -ne 'Completed') { throw "collector_record ended in state $state" }
}catch{$actionError=$_.Exception.Message;throw}finally{
    if($null-ne$job){if($job.State-notin@('Completed','Failed','Stopped','Disconnected')){Stop-Job -Job $job -ErrorAction SilentlyContinue;Wait-Job -Job $job|Out-Null};Receive-Job -Job $job -ErrorAction SilentlyContinue|Out-Null;Remove-Job -Job $job -Force -ErrorAction SilentlyContinue}
    if($overridden){if($priorExists){Write-AtomicBytes $continuePath $priorBytes -AllowReplace}elseif(Test-Path -LiteralPath $continuePath){Remove-Item -LiteralPath $continuePath -Force}}
}
foreach($required in @($manifest,$actionReceipt,$controllerReceipt)){if(-not(Test-Path -LiteralPath $required -PathType Leaf)){throw "A30 required acquisition artifact missing: $required"}}
& $pythonExe $verifier --capture-root $out --action-receipt $actionReceipt --profile $profile --controller-receipt $controllerReceipt --session-manifest $manifest --save $save --output $verification
if($LASTEXITCODE-ne0){throw "A30 autonomous production acquisition was rejected; inspect console output and $out"}
Write-Output "A30 all-six-group autonomous acquisition accepted: $verification"
