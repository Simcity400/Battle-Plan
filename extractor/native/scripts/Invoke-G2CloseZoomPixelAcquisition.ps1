[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$Hoi4Exe,
    [Parameter(Mandatory=$true)][string]$PdxUserDir,
    [Parameter(Mandatory=$true)][string]$ContinueSave,
    [Parameter(Mandatory=$true)][string]$PreparedProductModsetDir,
    [Parameter(Mandatory=$true)][string]$OutDir,
    [Parameter(Mandatory=$true)][int]$LoadedGameHourRaw,
    [Parameter(Mandatory=$true)][string]$ReplayApp,
    [string]$PrebuiltArtifactAttestationPath = '',
    [int]$DurationSeconds = 360,
    [int]$LoadTimeoutSeconds = 240,
    [string]$Python = 'python',
    [switch]$EnableLegacyClientDriving
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $EnableLegacyClientDriving) {
    throw 'This legacy owner drives and controls HOI4. It is disabled by default. Use G2_CLOSE_ZOOM_PIXEL_GATE/PASSIVE_OBSERVER_WORKFLOW.json with the shared passive observer; only an explicit -EnableLegacyClientDriving opt-in may enter this quarantined path.'
}
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$expectedD04SaveSha256 = 'c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735'
$expectedD04GameHourRaw = 60830441
$dateEpoch = 43799999
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$g2 = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\G2_CLOSE_ZOOM_PIXEL_GATE'
$harness = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\capture_harness\capture_harness.py'
$pulse = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_one_hour_pulse.ps1'
$readiness = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_wait_for_paused_save.ps1'
$helper = Join-Path $g2 'g2_autonomous_acquisition.py'
$intentBuilder = Join-Path $g2 'prepare_recorder_g2_intent.py'
$materializer = Join-Path $g2 'materialize_engine_observation.py'
$verifier = Join-Path $g2 'verify_close_zoom_pixel_gate.py'
$productVerifier = Join-Path $repo 'extractor\game_defs\verify_product_modset_capture_ready.py'
$staticVerifier = Join-Path $repo 'extractor\game_defs\static_bundle.py'
$attestationHelpers = Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1'
$driver = Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1'

function ConvertTo-NativeArgumentLine([object[]]$Arguments) {
    return (($Arguments | ForEach-Object {
        $value=[string]$_
        if ($value -match '[\s"]') { '"' + ($value -replace '(\\*)"','$1$1\"' -replace '(\\+)$','$1$1') + '"' } else { $value }
    }) -join ' ')
}
function Write-AtomicJson([string]$Path, $Value) {
    $parent=[IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($Path)); [IO.Directory]::CreateDirectory($parent)|Out-Null
    $temporary=[IO.Path]::Combine($parent,([IO.Path]::GetRandomFileName()+'.tmp'))
    try {
        [IO.File]::WriteAllText($temporary,(($Value|ConvertTo-Json -Depth 20)+"`n"),[Text.UTF8Encoding]::new($false))
        $stream=[IO.File]::Open($temporary,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        [IO.File]::Move($temporary,$Path)
    } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
}
function Write-AtomicText([string]$Path, [string]$Value) {
    $parent=[IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($Path)); [IO.Directory]::CreateDirectory($parent)|Out-Null
    $temporary=[IO.Path]::Combine($parent,([IO.Path]::GetRandomFileName()+'.tmp'))
    try {
        [IO.File]::WriteAllText($temporary,$Value,[Text.UTF8Encoding]::new($false))
        $stream=[IO.File]::Open($temporary,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        [IO.File]::Move($temporary,$Path)
    } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
}
function Write-AtomicReplaceBytes([string]$Path, [byte[]]$Bytes) {
    $fullPath=[IO.Path]::GetFullPath($Path); $parent=[IO.Path]::GetDirectoryName($fullPath)
    [IO.Directory]::CreateDirectory($parent)|Out-Null
    $temporary=[IO.Path]::Combine($parent,([IO.Path]::GetRandomFileName()+'.tmp'))
    try {
        [IO.File]::WriteAllBytes($temporary,$Bytes)
        $stream=[IO.File]::Open($temporary,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
        try { $stream.Flush($true) } finally { $stream.Dispose() }
        if ([IO.File]::Exists($fullPath)) { [IO.File]::Replace($temporary,$fullPath,$null,$true) }
        else { [IO.File]::Move($temporary,$fullPath) }
    } finally { if ([IO.File]::Exists($temporary)) { [IO.File]::Delete($temporary) } }
}
function Get-Sha256Bytes([byte[]]$Bytes) {
    $sha=[Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash($Bytes))).Replace('-','').ToLowerInvariant() }
    finally { $sha.Dispose() }
}

if ($LoadedGameHourRaw -ne $expectedD04GameHourRaw) { throw "LoadedGameHourRaw must be exact D04 raw hour $expectedD04GameHourRaw." }
if ($DurationSeconds -lt 120 -or $DurationSeconds -gt 1800) { throw 'DurationSeconds must be 120..1800.' }
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) { throw 'G2 requires a clean launch with no existing hoi4.exe.' }
$exe=(Resolve-Path -LiteralPath $Hoi4Exe).Path
$user=(Resolve-Path -LiteralPath $PdxUserDir).Path
$save=(Resolve-Path -LiteralPath $ContinueSave).Path
$prepared=(Resolve-Path -LiteralPath $PreparedProductModsetDir).Path
$out=[IO.Path]::GetFullPath($OutDir)
$pythonExe=(Get-Command $Python -CommandType Application -ErrorAction Stop).Source
$replay=(Resolve-Path -LiteralPath $ReplayApp).Path
if ((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) { throw 'hoi4.exe is not exact a729.' }
if ((Get-FileHash -LiteralPath $save -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedD04SaveSha256) { throw 'ContinueSave is not the frozen New D04 Save Deep War identity.' }
$saveDirectory=[IO.Path]::GetFullPath((Join-Path $user 'save games')).TrimEnd([char]'\')
if (-not [string]::Equals((Split-Path -Parent $save).TrimEnd([char]'\'),$saveDirectory,[StringComparison]::OrdinalIgnoreCase)) { throw "ContinueSave must be a direct child of $saveDirectory" }
foreach($requiredHelper in @($readiness,$pulse,$helper,$intentBuilder,$materializer,$verifier,$productVerifier,$staticVerifier,$attestationHelpers,$driver)) { if (-not (Test-Path -LiteralPath $requiredHelper -PathType Leaf)) { throw "Missing G2 helper: $requiredHelper" } }
if (Test-Path -LiteralPath $out) { throw 'G2 OutDir must be absent (immutable acquisition).' }
[IO.Directory]::CreateDirectory($out)|Out-Null

$preflight=Join-Path $out 'g2_preflight.json'
& $pythonExe $harness preflight --packets G2 --mode launch --phase synchronized_capture --coordinator direct_api_present --output $preflight
if ($LASTEXITCODE -ne 0) { throw 'G2 capture-harness preflight failed.' }
$camera=Join-Path $out 'camera_payload.json'
Write-AtomicJson $camera ([ordered]@{schema='hoi4-g2-camera-target-v1';required_height_f32_bits='0x424829c8';source='exact-a729-C03-minimum-clamp'})
$contextDir=Join-Path $out 'context'
& $pythonExe $harness snapshot-context --destination $contextDir --packets G2 --control $camera
if ($LASTEXITCODE -ne 0) { throw 'G2 context snapshot failed.' }
$context=Join-Path $contextDir 'capture_context.json'
$contextJson=Get-Content -LiteralPath $context -Raw|ConvertFrom-Json
$language=[string]$contextJson.inventory.settings.language
if ($language.Length -ge 2 -and $language[0] -eq $language[$language.Length-1] -and ($language[0] -eq '"' -or $language[0] -eq "'")) { $language=$language.Substring(1,$language.Length-2) }
$selectedHour=($LoadedGameHourRaw + 1) - $dateEpoch
$intent=Join-Path $out 'g2_recorder_intent.json'
& $pythonExe $intentBuilder --capture-context $context --settings-file (Join-Path $user 'settings.txt') --camera-payload $camera --localisation-language $language --selected-hour $selectedHour --observation-id ("g2-"+[guid]::NewGuid().ToString('N')) --output $intent
if ($LASTEXITCODE -ne 0) { throw 'G2 recorder intent creation failed.' }

. $attestationHelpers
if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) { $PrebuiltArtifactAttestationPath=Join-Path $repo '.cargo-target\release\hoi4-production-prebuilt-artifacts.json' }
$prebuilt=Test-ProductionArtifactAttestation -RepoRoot $repo -ReceiptPath $PrebuiltArtifactAttestationPath -ExpectedDriverPath $driver
$admission=Join-Path $out 'prepared_product_modset_admission.json'
& $pythonExe $productVerifier --exe $exe --prepared $prepared --out $admission
if ($LASTEXITCODE -ne 0) { throw 'Prepared product-modset admission failed.' }
$a=Get-Content -LiteralPath $admission -Raw|ConvertFrom-Json
$sessionId=[guid]::NewGuid().ToString(); $captureId=[guid]::NewGuid().ToString()
$args=@('--early-launch-exe',$exe,'--early-launch-arg','--continuelastsave','--early-launch-arg','--gdpr-compliant','--pdx-user-dir',$user,'--dll',[string]$prebuilt.artifacts.collector_dll,'--out-dir',$out,'--session-id',$sessionId,'--capture-id',$captureId,'--duration-s',[string]$DurationSeconds,'--sample-ms','100','--min-game-hours','1','--stream-format','compact','--profile','production','--g2-capture-intent',$intent,'--static-bundle-manifest',[string]$a.paths.static_bundle_manifest,'--static-bundle-mod-state',[string]$a.paths.mod_state,'--static-bundle-dlc-state',[string]$a.paths.dlc_state,'--static-bundle-load-order',[string]$a.paths.load_order,'--static-bundle-source-inventory',[string]$a.paths.source_inventory,'--static-bundle-live-resolution-report',[string]$a.paths.resolution_report,'--static-bundle-verifier',$staticVerifier,'--static-bundle-python',$pythonExe,'--static-bundle-timeout-ms','300000')
$start=[Diagnostics.ProcessStartInfo]::new(); $start.FileName=[string]$prebuilt.artifacts.recorder_host; $start.Arguments=ConvertTo-NativeArgumentLine $args; $start.UseShellExecute=$false; $start.CreateNoWindow=$true; $start.RedirectStandardOutput=$true; $start.RedirectStandardError=$true
$recorder=[Diagnostics.Process]::new(); $recorder.StartInfo=$start
$continueGamePath=Join-Path $user 'continue_game.json'
$continuePriorExists=Test-Path -LiteralPath $continueGamePath -PathType Leaf
$continuePriorBytes=if($continuePriorExists){[IO.File]::ReadAllBytes($continueGamePath)}else{$null}
$continuePriorSha256=if($continuePriorExists){Get-Sha256Bytes $continuePriorBytes}else{$null}
$continueObject=if($continuePriorExists){Get-Content -LiteralPath $continueGamePath -Raw|ConvertFrom-Json}else{[pscustomobject]@{title=[IO.Path]::GetFileNameWithoutExtension($save);desc='';date=[DateTime]::UtcNow.ToString('R');filename='';is_remote=$false}}
if($null -eq $continueObject.PSObject.Properties['filename']){$continueObject|Add-Member -NotePropertyName filename -NotePropertyValue ''}
if($null -eq $continueObject.PSObject.Properties['is_remote']){$continueObject|Add-Member -NotePropertyName is_remote -NotePropertyValue $false}
if($null -eq $continueObject.PSObject.Properties['title']){$continueObject|Add-Member -NotePropertyName title -NotePropertyValue ''}
$continueObject.title=[IO.Path]::GetFileNameWithoutExtension($save); $continueObject.filename=[IO.Path]::GetFileName($save); $continueObject.is_remote=$false
$continueReplacement=[Text.UTF8Encoding]::new($false).GetBytes((($continueObject|ConvertTo-Json -Depth 10)+"`n"))
$continueReplacementSha256=Get-Sha256Bytes $continueReplacement
$continueOverridden=$false; $stdout=$null; $stderr=$null; $recorderStarted=$false; $recorderProcessId=$null
$recorderExitCode=$null
$game=$null; $ownedProcess=$null; $ownedProcessStartUtc=$null; $ownedParentId=$null; $gameStartUtc=$null; $transactionError=$null; $cleanupError=$null
$cleanupStatus='not_started'; $continueRestoreStatus='not_started'; $continueRestoredSha256=$null
$readinessReceipt=Join-Path $out 'g2_paused_d04_readiness.json'
$cleanupReceipt=Join-Path $out 'g2_owned_process_cleanup.json'
$launchSelectionReceipt=Join-Path $out 'g2_launch_selection.json'
try {
$continueOverridden=$true
Write-AtomicReplaceBytes $continueGamePath $continueReplacement
if ((Get-FileHash -LiteralPath $continueGamePath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $continueReplacementSha256) { throw 'continue_game.json replacement did not retain exact bytes.' }
Write-AtomicJson $launchSelectionReceipt ([ordered]@{schema='hoi4-g2-exact-d04-launch-selection-v1';session_id=$sessionId;capture_id=$captureId;executable_sha256=$expectedExeSha256;continue_save_file=[IO.Path]::GetFileName($save);continue_save_sha256=$expectedD04SaveSha256;expected_game_hour_raw=$expectedD04GameHourRaw;continue_game_prior_existed=$continuePriorExists;continue_game_prior_sha256=$continuePriorSha256;continue_game_override_sha256=$continueReplacementSha256})
if (-not $recorder.Start()) { throw 'collector_record failed to start.' }
$recorderStarted=$true; $recorderProcessId=$recorder.Id
$stdout=$recorder.StandardOutput.ReadToEndAsync(); $stderr=$recorder.StandardError.ReadToEndAsync()
$deadline=[DateTime]::UtcNow.AddSeconds($LoadTimeoutSeconds)
while ([DateTime]::UtcNow -lt $deadline -and -not $recorder.HasExited) {
    $p=@(Get-Process -Name the game -ErrorAction SilentlyContinue)
    if($p.Count -gt 1){throw 'Clean G2 launch produced more than one hoi4.exe process.'}
    if($p.Count -eq 1){
        try {
            $candidate=$p[0]; $candidatePath=[string]$candidate.MainModule.FileName
            $candidateCim=Get-CimInstance Win32_Process -Filter "ProcessId=$($candidate.Id)" -ErrorAction Stop
            if(-not [IO.Path]::GetFullPath($candidatePath).Equals($exe,[StringComparison]::OrdinalIgnoreCase) -or (Get-FileHash -LiteralPath $candidatePath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256){throw 'Observed hoi4.exe is not exact a729.'}
            if([int]$candidateCim.ParentProcessId -ne $recorderProcessId){throw "Exact-a729 PID $($candidate.Id) is not owned by collector_record PID $recorderProcessId."}
            $ownedProcess=$candidate; $ownedProcessStartUtc=$candidate.StartTime.ToUniversalTime().ToString('o'); $ownedParentId=[int]$candidateCim.ParentProcessId
            if($candidate.MainWindowHandle -ne [IntPtr]::Zero){$game=$candidate;break}
        } catch { }
    }
    Start-Sleep -Milliseconds 250
}
if ($null -eq $game) { throw 'Exact-a729 window did not become ready.' }
$gameStartUtc=$game.StartTime.ToUniversalTime().ToString('o')
Start-Sleep -Seconds 3
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $readiness -ProcessId $game.Id -ReceiptPath $readinessReceipt -ExpectedGameHourRaw $expectedD04GameHourRaw -TimeoutSeconds $LoadTimeoutSeconds -StableSamples 4 -PollMilliseconds 250
if($LASTEXITCODE -ne 0){throw 'Exact D04 paused-save readiness failed before G2 input.'}
$readyJson=Get-Content -LiteralPath $readinessReceipt -Raw|ConvertFrom-Json
if($readyJson.game_hour -ne $expectedD04GameHourRaw -or $readyJson.exact_expected_game_hour -ne $true -or $readyJson.process_start_utc -cne $gameStartUtc){throw 'Paused-save readiness receipt did not bind the owned exact D04 process/hour.'}

Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class G2Input {
 [StructLayout(LayoutKind.Sequential)] public struct MI { public int dx,dy; public uint data,flags,time; public UIntPtr extra; }
 [StructLayout(LayoutKind.Sequential)] public struct KI { public ushort vk,scan; public uint flags,time; public UIntPtr extra; }
 [StructLayout(LayoutKind.Explicit)] public struct U { [FieldOffset(0)] public MI mi; [FieldOffset(0)] public KI ki; }
 [StructLayout(LayoutKind.Sequential)] public struct I { public uint type; public U u; }
 [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h); [DllImport("user32.dll")] static extern uint SendInput(uint n,I[] i,int s);
 public static void Key(IntPtr h, ushort v) { SetForegroundWindow(h); I d=new I(),u=new I(); d.type=u.type=1; d.u.ki.vk=v; u.u.ki.vk=v; u.u.ki.flags=2; if(SendInput(2,new[]{d,u},Marshal.SizeOf(typeof(I)))!=2) throw new Exception("key SendInput failed"); }
 public static void Text(IntPtr h,string s) { SetForegroundWindow(h); foreach(char c in s){ I d=new I(),u=new I(); d.type=u.type=1; d.u.ki.scan=c; d.u.ki.flags=4; u.u.ki.scan=c; u.u.ki.flags=6; if(SendInput(2,new[]{d,u},Marshal.SizeOf(typeof(I)))!=2) throw new Exception("text SendInput failed"); } }
 public static void Wheel(IntPtr h,int delta) { SetForegroundWindow(h); I i=new I(); i.type=0; i.u.mi.data=unchecked((uint)delta); i.u.mi.flags=0x0800; if(SendInput(1,new[]{i},Marshal.SizeOf(typeof(I)))!=1) throw new Exception("wheel SendInput failed"); }
}
'@
[G2Input]::Key($game.MainWindowHandle,0xC0); [G2Input]::Text($game.MainWindowHandle,'observe'); [G2Input]::Key($game.MainWindowHandle,0x0D); Start-Sleep -Seconds 1; [G2Input]::Key($game.MainWindowHandle,0xC0)
$ready=$false
for($i=0;$i -lt 80 -and -not $ready;$i++) {
    [G2Input]::Wheel($game.MainWindowHandle,120); Start-Sleep -Milliseconds 200
    $feedback=Join-Path $out ("g2_feedback_{0:D3}.json" -f $i)
    & $pythonExe $helper observe --require-game-hour-raw $LoadedGameHourRaw --require-paused --output $feedback | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "G2 durable live feedback failed at wheel $i." }
    $ready=[bool](Get-Content -LiteralPath $feedback -Raw|ConvertFrom-Json).exact_retained_close_zoom_band
}
if (-not $ready) { throw 'Camera never reached exact retained 0x424829c8 current+target height.' }
$pulseReceipt=Join-Path $out 'g2_authenticated_one_hour_pulse.json'
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $pulse -ProcessId $game.Id -ReceiptPath $pulseReceipt -TimeoutMilliseconds 60000 -PollMilliseconds 1
if ($LASTEXITCODE -ne 0) { throw 'Authenticated one-hour G2 pulse failed.' }
if (-not $recorder.WaitForExit(($DurationSeconds+120)*1000)) { throw 'collector_record did not finalize before deadline.' }
$recorderExitCode=$recorder.ExitCode
} catch { $transactionError=$_ } finally {
    if($continueOverridden){
        try {
            if(-not (Test-Path -LiteralPath $continueGamePath -PathType Leaf)){throw 'continue_game.json disappeared before restoration.'}
            $currentContinueSha=(Get-FileHash -LiteralPath $continueGamePath -Algorithm SHA256).Hash.ToLowerInvariant()
            if($currentContinueSha -cne $continueReplacementSha256){throw "continue_game.json changed during owned launch: $currentContinueSha"}
            if($continuePriorExists){Write-AtomicReplaceBytes $continueGamePath $continuePriorBytes; $continueRestoredSha256=(Get-FileHash -LiteralPath $continueGamePath -Algorithm SHA256).Hash.ToLowerInvariant(); if($continueRestoredSha256 -cne $continuePriorSha256){throw 'continue_game.json prior bytes were not restored exactly.'}; $continueRestoreStatus='restored_exact_prior_bytes'}
            else{Remove-Item -LiteralPath $continueGamePath -Force; $continueRestoreStatus='removed_owned_create_new_override'}
        }catch{$continueRestoreStatus='failed_closed_restore';$cleanupError=$_}
        $continueOverridden=$false
    }
    if ($recorderStarted -and -not $recorder.HasExited) {
        Stop-Process -Id $recorder.Id -Force -ErrorAction SilentlyContinue
        [void]$recorder.WaitForExit(30000)
    }
    if ($recorderStarted -and $recorder.HasExited -and $null -eq $recorderExitCode) { $recorderExitCode=$recorder.ExitCode }
    if ($recorderStarted -and $recorder.HasExited -and $null -ne $stdout -and $null -ne $stderr) {
        Write-AtomicText (Join-Path $out 'collector_record.stdout.log') ([string]$stdout.Result)
        Write-AtomicText (Join-Path $out 'collector_record.stderr.log') ([string]$stderr.Result)
    }
    if($null -eq $ownedProcess){$cleanupStatus='no_owned_process_observed'}else{
        $live=Get-Process -Id $ownedProcess.Id -ErrorAction SilentlyContinue
        if($null -eq $live){$cleanupStatus='owned_process_already_exited'}else{
            try{
                $liveStart=$live.StartTime.ToUniversalTime().ToString('o'); $livePath=[string]$live.MainModule.FileName
                if($liveStart -cne $ownedProcessStartUtc){$cleanupStatus='owned_process_already_exited_pid_reused_refused'}
                elseif(-not [IO.Path]::GetFullPath($livePath).Equals($exe,[StringComparison]::OrdinalIgnoreCase) -or (Get-FileHash -LiteralPath $livePath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256){throw 'Owned PID executable identity changed; refusing termination.'}
                else{Stop-Process -Id $live.Id -Force -ErrorAction Stop; $live.WaitForExit(30000); if(-not $live.HasExited){throw 'Owned exact-a729 process did not exit.'}; $cleanupStatus='terminated_owned_exact_a729'}
            }catch{$cleanupStatus='failed_closed_refused_or_failed_termination';if($null -eq $cleanupError){$cleanupError=$_}}
        }
    }
    Write-AtomicJson $cleanupReceipt ([ordered]@{schema='hoi4-g2-owned-process-cleanup-v1';session_id=$sessionId;capture_id=$captureId;launch_selection_receipt_sha256=if(Test-Path -LiteralPath $launchSelectionReceipt -PathType Leaf){(Get-FileHash -LiteralPath $launchSelectionReceipt -Algorithm SHA256).Hash.ToLowerInvariant()}else{$null};recorder_process_id=$recorderProcessId;owned_process_id=if($null -eq $ownedProcess){$null}else{$ownedProcess.Id};owned_process_start_utc=$ownedProcessStartUtc;owned_process_parent_id=$ownedParentId;executable_sha256=$expectedExeSha256;continue_save_sha256=$expectedD04SaveSha256;expected_game_hour_raw=$expectedD04GameHourRaw;readiness_receipt_file=[IO.Path]::GetFileName($readinessReceipt);readiness_receipt_sha256=if(Test-Path -LiteralPath $readinessReceipt -PathType Leaf){(Get-FileHash -LiteralPath $readinessReceipt -Algorithm SHA256).Hash.ToLowerInvariant()}else{$null};continue_game_override_sha256=$continueReplacementSha256;continue_game_restore_status=$continueRestoreStatus;continue_game_prior_sha256=$continuePriorSha256;continue_game_restored_sha256=$continueRestoredSha256;cleanup_status=$cleanupStatus;scope='only the exact-a729 child PID of this collector_record process with matching start time is eligible for termination'})
    $recorder.Dispose()
}
if($null -ne $transactionError){throw $transactionError}
if($null -ne $cleanupError){throw $cleanupError}
if($cleanupStatus -cnotin @('terminated_owned_exact_a729','owned_process_already_exited','owned_process_already_exited_pid_reused_refused')){throw "G2 owned-process cleanup failed: $cleanupStatus"}
if ($recorderExitCode -ne 0) { throw "collector_record failed with $recorderExitCode." }
$recording=Join-Path $out 'native_session.battleplan'; $extracted=Join-Path $out 'g2_extracted'
& $pythonExe $helper extract --recording $recording --output-dir $extracted; if ($LASTEXITCODE -ne 0) { throw 'Packaged G2 extraction failed.' }
$replayBmp=Join-Path $extracted 'replay.bmp'
& $replay --non-interactive --recording $recording --dump-frame $replayBmp --dump-hour $selectedHour; if ($LASTEXITCODE -ne 0) { throw 'Production replay G2 dump failed.' }
$receipt="$replayBmp.pixel-gate-receipt.json"; $observation=Join-Path $extracted 'engine_observation.json'
& $pythonExe $materializer --witness (Join-Path $extracted 'g2_engine_pixel_witness.json') --engine-bmp (Join-Path $extracted 'g2_engine_map_boundary.bmp') --receipt $receipt --recording $recording --capture-context $context --packaged-capture-declaration (Join-Path $extracted 'g2_engine_capture.json') --output $observation
if ($LASTEXITCODE -ne 0) { throw 'G2 observation materialization failed.' }
$golden=Join-Path $out 'g2_close_zoom_pixel_golden.json'; $verification=Join-Path $out 'g2_close_zoom_pixel_verification.json'
& $pythonExe $verifier --receipt $receipt --replay-bmp $replayBmp --engine-bmp (Join-Path $extracted 'g2_engine_map_boundary.bmp') --engine-observation-manifest $observation --engine-pixel-witness (Join-Path $extracted 'g2_engine_pixel_witness.json') --capture-context $context --packaged-capture-declaration (Join-Path $extracted 'g2_engine_capture.json') --authority-exe $exe --recording $recording --golden $golden --admit-zero-delta-golden --output $verification
if ($LASTEXITCODE -ne 0) { throw 'G2 exact pixel verification/admission failed.' }
$acquisitionReceipt=Join-Path $out 'g2_acquisition_receipt.json'
Write-AtomicJson $acquisitionReceipt ([ordered]@{schema='hoi4-g2-autonomous-acquisition-receipt-v1';status='PASS_AUTHENTICATED_D04_EXACT_ZERO_DELTA';session_id=$sessionId;capture_id=$captureId;executable_sha256=$expectedExeSha256;continue_save_sha256=$expectedD04SaveSha256;loaded_game_hour_raw=$expectedD04GameHourRaw;launch_selection_receipt_file=[IO.Path]::GetFileName($launchSelectionReceipt);launch_selection_receipt_sha256=(Get-FileHash -LiteralPath $launchSelectionReceipt -Algorithm SHA256).Hash.ToLowerInvariant();readiness_receipt_file=[IO.Path]::GetFileName($readinessReceipt);readiness_receipt_sha256=(Get-FileHash -LiteralPath $readinessReceipt -Algorithm SHA256).Hash.ToLowerInvariant();pulse_receipt_file=[IO.Path]::GetFileName($pulseReceipt);pulse_receipt_sha256=(Get-FileHash -LiteralPath $pulseReceipt -Algorithm SHA256).Hash.ToLowerInvariant();cleanup_receipt_file=[IO.Path]::GetFileName($cleanupReceipt);cleanup_receipt_sha256=(Get-FileHash -LiteralPath $cleanupReceipt -Algorithm SHA256).Hash.ToLowerInvariant();prepared_product_modset_admission_file=[IO.Path]::GetFileName($admission);prepared_product_modset_admission_sha256=(Get-FileHash -LiteralPath $admission -Algorithm SHA256).Hash.ToLowerInvariant();capture_context_sha256=(Get-FileHash -LiteralPath $context -Algorithm SHA256).Hash.ToLowerInvariant();recorder_intent_sha256=(Get-FileHash -LiteralPath $intent -Algorithm SHA256).Hash.ToLowerInvariant();recording_file=[IO.Path]::GetFileName($recording);recording_sha256=(Get-FileHash -LiteralPath $recording -Algorithm SHA256).Hash.ToLowerInvariant();golden_file=[IO.Path]::GetFileName($golden);golden_sha256=(Get-FileHash -LiteralPath $golden -Algorithm SHA256).Hash.ToLowerInvariant();verification_file=[IO.Path]::GetFileName($verification);verification_sha256=(Get-FileHash -LiteralPath $verification -Algorithm SHA256).Hash.ToLowerInvariant()})
Write-Output "G2 exact zero-delta acquisition admitted: $acquisitionReceipt"
