[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $RecorderExe,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $PdxUserDir,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [Parameter(Mandatory = $true)] [string] $ContinueSave,
    [Parameter(Mandatory = $true)] [string] $ActiveFixtureEventPath,
    [string] $PrebuiltArtifactAttestationPath = '',
    [int] $DurationSeconds = 600,
    [int] $SampleMilliseconds = 25,
    [int] $ProcessTimeoutSeconds = 300,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave','--gdpr-compliant'),
    [string] $Python = 'python'
)

$ErrorActionPreference='Stop';Set-StrictMode -Version Latest
$expectedExeSha256='870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$expectedSaveSha256='c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735'
if($DurationSeconds-lt 300-or$DurationSeconds-gt 1800){throw 'DurationSeconds must be in 300..1800.'}
if($SampleMilliseconds-lt 10-or$SampleMilliseconds-gt 1000){throw 'SampleMilliseconds must be in 10..1000.'}
foreach($path in @($RecorderExe,$Hoi4Exe,$PdxUserDir,$CollectorDll,$ContinueSave,$ActiveFixtureEventPath)){if(-not(Test-Path -LiteralPath $path)){throw "Required path does not exist: $path"}}
$recorder=(Resolve-Path -LiteralPath $RecorderExe).Path;$exe=(Resolve-Path -LiteralPath $Hoi4Exe).Path;$pdx=(Resolve-Path -LiteralPath $PdxUserDir).Path
$dll=(Resolve-Path -LiteralPath $CollectorDll).Path;$sourceSave=(Resolve-Path -LiteralPath $ContinueSave).Path;$activeFixture=(Resolve-Path -LiteralPath $ActiveFixtureEventPath).Path
$pythonExe=(Get-Command $Python -CommandType Application -ErrorAction Stop).Source;$out=[IO.Path]::GetFullPath($OutDir)
$repo=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\..')).Path
$fixture=(Resolve-Path -LiteralPath (Join-Path $repo 'extractor\native\fixtures\xui21\events\xui21_popup_matrix.txt')).Path
$profile=(Resolve-Path -LiteralPath (Join-Path $repo 'replay_interface\re_evidence\rendering_laws\XUI21\H_POPUP_AUTONOMOUS_ACTION_PROFILE_1920X1080.json')).Path
$driver=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Invoke-Xui21AutonomousPopupActions.ps1')).Path
$runtimeRegrader=(Resolve-Path -LiteralPath (Join-Path $repo 'replay_interface\re_evidence\rendering_laws\XUI21\verify_h_popup_runtime_matrix.py')).Path
$verifier=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'verify_xui21_popup_matrix_acquisition.py')).Path
$productionDriver=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1')).Path
$attestationHelpers=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1')).Path
if([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)){$PrebuiltArtifactAttestationPath=Join-Path $repo '.cargo-target\release\hoi4-production-prebuilt-artifacts.json'}
. $attestationHelpers
$prebuilt=Test-ProductionArtifactAttestation -RepoRoot $repo -ReceiptPath $PrebuiltArtifactAttestationPath -ExpectedDriverPath $productionDriver
$attestation=(Resolve-Path -LiteralPath $prebuilt.receipt_path).Path
if(-not[string]::Equals($recorder,[string]$prebuilt.artifacts.recorder_host,[StringComparison]::OrdinalIgnoreCase)-or-not[string]::Equals($dll,[string]$prebuilt.artifacts.collector_dll,[StringComparison]::OrdinalIgnoreCase)){throw 'XUI21 acquisition requires centrally attested release artifacts.'}
if((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()-cne$expectedExeSha256){throw 'Hoi4Exe is not exact a729.'}
if((Get-FileHash -LiteralPath $sourceSave -Algorithm SHA256).Hash.ToLowerInvariant()-cne$expectedSaveSha256){throw 'ContinueSave is not frozen New D04 Save Deep War.'}
$saveDir=[IO.Path]::GetFullPath((Join-Path $pdx 'save games')).TrimEnd([char]'\');if(-not[string]::Equals($saveDir,[IO.Path]::GetFullPath((Split-Path -Parent $sourceSave)).TrimEnd([char]'\'),[StringComparison]::OrdinalIgnoreCase)){throw "ContinueSave must be a direct child of $saveDir"}
if([Array]::IndexOf($EarlyLaunchArgument,'--continuelastsave')-lt 0-or[Array]::IndexOf($EarlyLaunchArgument,'--gdpr-compliant')-lt[Array]::IndexOf($EarlyLaunchArgument,'--continuelastsave')){throw 'Early launch arguments must contain ordered --continuelastsave/--gdpr-compliant.'}
if(@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count-ne 0){throw 'XUI21 acquisition requires a clean process boundary.'}
if(Test-Path -LiteralPath $out){if(-not(Test-Path -LiteralPath $out -PathType Container)-or@(Get-ChildItem -LiteralPath $out -Force).Count-ne 0){throw "OutDir must be absent or empty: $out"}}else{[IO.Directory]::CreateDirectory($out)|Out-Null}

function Write-AtomicBytes([string]$Path,[byte[]]$Bytes,[switch]$Replace){$full=[IO.Path]::GetFullPath($Path);$parent=[IO.Path]::GetDirectoryName($full);[IO.Directory]::CreateDirectory($parent)|Out-Null;$tmp=[IO.Path]::Combine($parent,([IO.Path]::GetRandomFileName()+'.tmp'));try{[IO.File]::WriteAllBytes($tmp,$Bytes);$s=[IO.File]::Open($tmp,'Open','ReadWrite','None');try{$s.Flush($true)}finally{$s.Dispose()};if([IO.File]::Exists($full)){if(-not$Replace){throw "Refusing to replace $full"};[IO.File]::Replace($tmp,$full,$null,$true)}else{[IO.File]::Move($tmp,$full)}}finally{if([IO.File]::Exists($tmp)){[IO.File]::Delete($tmp)}}}
function Write-Continue([string]$SavePath){$o=if(Test-Path -LiteralPath $script:continuePath){Get-Content -LiteralPath $script:continuePath -Raw|ConvertFrom-Json}else{[pscustomobject]@{title='';desc='';date=[DateTime]::UtcNow.ToString('R');filename='';is_remote=$false}};$o.filename=[IO.Path]::GetFileName($SavePath);$o.title=[IO.Path]::GetFileNameWithoutExtension($SavePath);$o.is_remote=$false;Write-AtomicBytes $script:continuePath ([Text.UTF8Encoding]::new($false).GetBytes((($o|ConvertTo-Json -Depth 8)+"`n"))) -Replace}
function Start-CaptureStage([string]$Name,[string]$SavePath,[bool]$EnableXui21,[string]$ActionMode,[string]$ActionReceipt,[string]$SeedSavePath){
    if(@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count-ne 0){throw "$Name did not start clean."}
    $stage=Join-Path $out $Name;[IO.Directory]::CreateDirectory($stage)|Out-Null;$session=[guid]::NewGuid().ToString();$capture=[guid]::NewGuid().ToString()
    Write-Continue $SavePath
    $args=@('--early-launch-exe',$exe,'--pdx-user-dir',$pdx,'--dll',$dll,'--out-dir',$stage,'--session-id',$session,'--capture-id',$capture,'--duration-s',$DurationSeconds,'--sample-ms',$SampleMilliseconds,'--min-game-hours','0','--stream-format','compact','--profile','debug','--scheduled-domain',$(if($EnableXui21){'fired_event_popups_live'}else{'divisions'}),'--scheduled-domains-exact','--sidecar-log','--hour-latch');foreach($v in $EarlyLaunchArgument){$args+=@('--early-launch-arg',$v)}
    $coverage=Join-Path $stage 'xui21_runtime_receipts';if($EnableXui21){[IO.Directory]::CreateDirectory($coverage)|Out-Null}
    $job=$null;$game=$null;$processStartUtc='';$stageSucceeded=$false
    try{
        $job=Start-Job -ScriptBlock {param($Recorder,$Arguments,$Coverage,$Enable);if($Enable){$env:BATTLE_PLAN_XUI21_COVERAGE_RECEIPT_DIR=$Coverage}else{Remove-Item Env:BATTLE_PLAN_XUI21_COVERAGE_RECEIPT_DIR -ErrorAction SilentlyContinue};& $Recorder @Arguments;if($LASTEXITCODE-ne 0){throw "collector_record exited $LASTEXITCODE"}} -ArgumentList $recorder,(,$args),$coverage,$EnableXui21
        $deadline=[DateTime]::UtcNow.AddSeconds($ProcessTimeoutSeconds);while($null-eq$game-and[DateTime]::UtcNow-lt$deadline){if($job.State -in @('Failed','Stopped','Disconnected')){throw "$Name recorder ended before launch."};foreach($p in @(Get-Process -Name the game -ErrorAction SilentlyContinue)){try{if([string]::Equals($p.MainModule.FileName,$exe,[StringComparison]::OrdinalIgnoreCase)-and$p.MainWindowHandle-ne[IntPtr]::Zero){$game=$p;break}}catch{}};if($null-eq$game){Start-Sleep -Milliseconds 500}}
        if($null-eq$game){throw "$Name exact process did not expose a window."}
        $processStartUtc=$game.StartTime.ToUniversalTime().ToString('o');$gamePid=$game.Id
        $actionArgs=@('-NoProfile','-ExecutionPolicy','Bypass','-File',$driver,'-Mode',$ActionMode,'-ProcessId',[string]$gamePid,'-ExpectedProcessStartUtc',$processStartUtc,'-ActionProfile',$profile,'-ReceiptPath',$ActionReceipt,'-ReadyTimeoutSeconds',[string]$ProcessTimeoutSeconds)
        if($ActionMode-ceq'SeedPendingSave'){$actionArgs+=@('-SeedSavePath',$SeedSavePath)}else{$actionArgs+=@('-CoverageReceiptDirectory',$coverage,'-MatrixTimeoutSeconds',[string]([Math]::Min(540,$DurationSeconds-30)))}
        & powershell.exe @actionArgs;if($LASTEXITCODE-ne 0){throw "$Name autonomous action driver failed with code $LASTEXITCODE"}
        if($null-ne(Get-Process -Id $gamePid -ErrorAction SilentlyContinue)){Stop-Process -Id $gamePid -Force -ErrorAction Stop}
        Wait-Job -Job $job|Out-Null;$state=$job.State
        if($state-ne'Completed'){throw "$Name recorder ended in state $state"}
        $manifest=Join-Path $stage 'native_session_manifest.json';if(-not(Test-Path -LiteralPath $manifest -PathType Leaf)){throw "$Name manifest absent."}
        $stageSucceeded=$true
        return [ordered]@{stage=$stage;manifest=$manifest;coverage=$coverage;pid=$gamePid;process_start_utc=$processStartUtc}
    }finally{
        if($null-ne$game-and$null-ne(Get-Process -Id $game.Id -ErrorAction SilentlyContinue)){Stop-Process -Id $game.Id -Force -ErrorAction SilentlyContinue}
        if($null-ne$job){if($job.State-notin@('Completed','Failed','Stopped','Disconnected')){if(-not$stageSucceeded){Stop-Job -Job $job -ErrorAction SilentlyContinue};Wait-Job -Job $job|Out-Null};Receive-Job -Job $job -ErrorAction SilentlyContinue|ForEach-Object{Write-Verbose([string]$_)};Remove-Job -Job $job -Force -ErrorAction SilentlyContinue}
    }
}

$priorFixture=[IO.File]::ReadAllBytes($activeFixture);$priorFixtureSha=(Get-FileHash -LiteralPath $activeFixture -Algorithm SHA256).Hash.ToLowerInvariant();$script:continuePath=Join-Path $pdx 'continue_game.json';$continueExisted=Test-Path -LiteralPath $script:continuePath;$continuePrior=if($continueExisted){[IO.File]::ReadAllBytes($script:continuePath)}else{$null}
$seededSave=Join-Path $saveDir ("xui21_autonomous_seed_{0}.hoi4" -f [guid]::NewGuid().ToString('N'));$seedAction=Join-Path $out 'xui21_seed_action_receipt.json';$matrixAction=Join-Path $out 'xui21_matrix_action_receipt.json'
$stagesCompleted=$false
try{
    Write-AtomicBytes $activeFixture ([IO.File]::ReadAllBytes($fixture)) -Replace
    $seedStage=Start-CaptureStage 'seed_pending_save' $sourceSave $false 'SeedPendingSave' $seedAction $seededSave
    $matrixStage=Start-CaptureStage 'matrix_capture' $seededSave $true 'DriveMatrix' $matrixAction ''
    $stagesCompleted=$true
}finally{
    $restoreFailures=[Collections.Generic.List[string]]::new()
    try{Write-AtomicBytes $activeFixture $priorFixture -Replace}catch{$restoreFailures.Add("fixture: $($_.Exception.Message)")}
    try{if($continueExisted){Write-AtomicBytes $script:continuePath $continuePrior -Replace}elseif(Test-Path -LiteralPath $script:continuePath){Remove-Item -LiteralPath $script:continuePath -Force}}catch{$restoreFailures.Add("continue_game: $($_.Exception.Message)")}
    try{if(-not$stagesCompleted-and(Test-Path -LiteralPath $seededSave -PathType Leaf)){$failedSeed=Join-Path $out 'xui21_seeded_pending_save.failed.hoi4';if(-not(Test-Path -LiteralPath $failedSeed)){[IO.File]::Move($seededSave,$failedSeed)}}}catch{$restoreFailures.Add("seeded_save: $($_.Exception.Message)")}
    if($restoreFailures.Count-ne 0){throw "XUI21 unconditional restoration failed: $($restoreFailures -join '; ')"}
}
if((Get-FileHash -LiteralPath $activeFixture -Algorithm SHA256).Hash.ToLowerInvariant()-cne$priorFixtureSha){throw 'Active fixture path was not restored byte-identically.'}
$retainedSeededSave=Join-Path $out 'xui21_seeded_pending_save.hoi4'
if(Test-Path -LiteralPath $retainedSeededSave){throw 'Refusing to replace retained seeded save.'}
[IO.File]::Move($seededSave,$retainedSeededSave)
$seededSave=$retainedSeededSave
$action=Get-Content -LiteralPath $matrixAction -Raw|ConvertFrom-Json;$runtime=Join-Path $matrixStage.coverage ([string]$action.raw_coverage_receipt_file);if(-not(Test-Path -LiteralPath $runtime -PathType Leaf)){throw 'Matrix action raw receipt is absent.'}
$regrade=Join-Path $out 'xui21_runtime_coverage_regrade.json';& $pythonExe $runtimeRegrader $runtime --output $regrade;if($LASTEXITCODE-ne 0){throw 'XUI21 strict runtime receipt regrader rejected the capture.'}
$verification=Join-Path $out 'xui21_autonomous_acquisition_verification.json';& $pythonExe $verifier --seed-action $seedAction --matrix-action $matrixAction --seed-manifest $seedStage.manifest --matrix-manifest $matrixStage.manifest --runtime-receipt $runtime --runtime-regrade $regrade --profile $profile --fixture $fixture --source-save $sourceSave --seeded-save $seededSave --prebuilt-attestation $attestation --output $verification;if($LASTEXITCODE-ne 0){throw "XUI21 acquisition rejected; inspect $verification"}
$v=Get-Content -LiteralPath $verification -Raw|ConvertFrom-Json
$receipt=[ordered]@{schema='hoi4-xui21-autonomous-popup-matrix-acquisition-v1';status='pass_exact_a729_machine_owned_20_cells_4_edges';timestamp_utc=[DateTime]::UtcNow.ToString('o');executable_sha256=$expectedExeSha256;source_save_sha256=$expectedSaveSha256;seeded_save_sha256=(Get-FileHash -LiteralPath $seededSave -Algorithm SHA256).Hash.ToLowerInvariant();active_fixture_prior_sha256=$priorFixtureSha;active_fixture_restored_sha256=(Get-FileHash -LiteralPath $activeFixture -Algorithm SHA256).Hash.ToLowerInvariant();fixture_sha256=(Get-FileHash -LiteralPath $fixture -Algorithm SHA256).Hash.ToLowerInvariant();profile_sha256=(Get-FileHash -LiteralPath $profile -Algorithm SHA256).Hash.ToLowerInvariant();raw_runtime_receipt_sha256=(Get-FileHash -LiteralPath $runtime -Algorithm SHA256).Hash.ToLowerInvariant();runtime_regrade_sha256=(Get-FileHash -LiteralPath $regrade -Algorithm SHA256).Hash.ToLowerInvariant();verification_sha256=(Get-FileHash -LiteralPath $verification -Algorithm SHA256).Hash.ToLowerInvariant();runtime_session_id=$v.runtime_session_id;matrix_bits=[int64]$v.matrix_bits;cell_count=[int]$v.cell_count;lifecycle_witness_count=[int]$v.lifecycle_witness_count;fresh_process_boundary=$v.fresh_process_boundary;immutable_receipt=$true;promotion_authorized=$false}
$final=Join-Path $out 'xui21_autonomous_popup_matrix_acquisition_receipt.json';Write-AtomicBytes $final ([Text.UTF8Encoding]::new($false).GetBytes((($receipt|ConvertTo-Json -Depth 10)+"`n")))
Write-Output "XUI21 autonomous popup matrix accepted: $final"
