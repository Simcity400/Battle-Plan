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
    [string] $ActionProfile = '',
    [int] $DurationSeconds = 300,
    [int] $SampleMilliseconds = 25,
    [int] $ProcessTimeoutSeconds = 420,
    [int] $InitialSettleSeconds = 75,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$expectedSaveSha256 = 'c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
if ([string]::IsNullOrWhiteSpace($ActionProfile)) {
    $ActionProfile = Join-Path $repo 'extractor\documents\dossiers\A31_AUTONOMOUS_NATIONAL_FOCUS_ACTIONS_1920X1080.json'
}
foreach ($path in @($RecorderExe,$Hoi4Exe,$PdxUserDir,$CollectorDll,$ContinueSave,$StaticBundleManifest,$StaticBundleBindingsDir,$ActionProfile)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required A31 path does not exist: $path" }
}
if ($DurationSeconds -lt 120 -or $DurationSeconds -gt 1800) { throw 'DurationSeconds must be in 120..1800.' }
if ($SampleMilliseconds -lt 10 -or $SampleMilliseconds -gt 1000) { throw 'SampleMilliseconds must be in 10..1000.' }
if ($ProcessTimeoutSeconds -lt 30 -or $ProcessTimeoutSeconds -gt 1800) { throw 'ProcessTimeoutSeconds must be in 30..1800.' }
if ($InitialSettleSeconds -lt 0 -or $InitialSettleSeconds -gt 300) { throw 'InitialSettleSeconds must be in 0..300.' }

$resolvedRecorder=(Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedExe=(Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedUserDir=(Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollector=(Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedSave=(Resolve-Path -LiteralPath $ContinueSave).Path
$resolvedBundle=(Resolve-Path -LiteralPath $StaticBundleManifest).Path
$resolvedBindings=(Resolve-Path -LiteralPath $StaticBundleBindingsDir).Path
$resolvedProfile=(Resolve-Path -LiteralPath $ActionProfile).Path
$pythonExe=(Get-Command $Python -CommandType Application -ErrorAction Stop).Source
$resolvedOut=[IO.Path]::GetFullPath($OutDir)
if ((Get-FileHash $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) { throw 'Hoi4Exe is not exact a729.' }
if ((Get-FileHash $resolvedSave -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedSaveSha256) { throw 'A31 requires the frozen New D04 Save Deep War identity.' }
$saveRoot=[IO.Path]::GetFullPath((Join-Path $resolvedUserDir 'save games')).TrimEnd([char]'\')
if (-not [IO.Path]::GetFullPath((Split-Path -Parent $resolvedSave)).TrimEnd([char]'\').Equals($saveRoot,[StringComparison]::OrdinalIgnoreCase)) { throw "ContinueSave must be a direct child of $saveRoot" }
if ([Array]::IndexOf($EarlyLaunchArgument,'--continuelastsave') -lt 0 -or [Array]::IndexOf($EarlyLaunchArgument,'--gdpr-compliant') -lt [Array]::IndexOf($EarlyLaunchArgument,'--continuelastsave')) { throw 'EarlyLaunchArgument must contain --continuelastsave followed by --gdpr-compliant.' }
if (@(Get-Process the game -ErrorAction SilentlyContinue).Count -ne 0) { throw 'A31 requires a clean early launch; close every existing hoi4.exe first.' }
if (Test-Path $resolvedOut) {
    if (-not (Test-Path $resolvedOut -PathType Container) -or @(Get-ChildItem $resolvedOut -Force).Count -ne 0) { throw "OutDir must be absent or empty: $resolvedOut" }
} else { [IO.Directory]::CreateDirectory($resolvedOut)|Out-Null }

function Write-AtomicBytes([string]$Path,[byte[]]$Bytes,[switch]$AllowReplace) {
    $full=[IO.Path]::GetFullPath($Path);$parent=[IO.Path]::GetDirectoryName($full);[IO.Directory]::CreateDirectory($parent)|Out-Null
    $temporary=[IO.Path]::Combine($parent,([IO.Path]::GetRandomFileName()+'.tmp'))
    try {
        [IO.File]::WriteAllBytes($temporary,$Bytes);$stream=[IO.File]::Open($temporary,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
        try{$stream.Flush($true)}finally{$stream.Dispose()}
        if([IO.File]::Exists($full)){if(-not $AllowReplace){throw "Refusing to replace $full"};[IO.File]::Replace($temporary,$full,$null,$true)}else{[IO.File]::Move($temporary,$full)}
    } finally {if([IO.File]::Exists($temporary)){[IO.File]::Delete($temporary)}}
}
function Write-AtomicText([string]$Path,[string]$Text,[switch]$AllowReplace){Write-AtomicBytes $Path ([Text.UTF8Encoding]::new($false).GetBytes($Text)) -AllowReplace:$AllowReplace}

$productionDriver=(Resolve-Path (Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1')).Path
$attestationHelpers=(Resolve-Path (Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1')).Path
if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) {$PrebuiltArtifactAttestationPath=Join-Path $repo '.cargo-target\release\hoi4-production-prebuilt-artifacts.json'}
. $attestationHelpers
$prebuilt=Test-ProductionArtifactAttestation -RepoRoot $repo -ReceiptPath $PrebuiltArtifactAttestationPath -ExpectedDriverPath $productionDriver
$resolvedAttestation=(Resolve-Path $prebuilt.receipt_path).Path
if(-not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.recorder_host).Equals($resolvedRecorder,[StringComparison]::OrdinalIgnoreCase) -or -not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.collector_dll).Equals($resolvedCollector,[StringComparison]::OrdinalIgnoreCase)){throw 'A31 recorder and collector must come from the current central Release attestation.'}

$staticVerifier=(Resolve-Path (Join-Path $repo 'extractor\game_defs\static_bundle.py')).Path
$driver=(Resolve-Path (Join-Path $repo 'extractor\native\lanes\focus_research_decisions_depth_live\tools\capture_a31_fixture_pair.py')).Path
$admissionVerifier=(Resolve-Path (Join-Path $repo 'extractor\native\lanes\focus_research_decisions_depth_live\tools\verify_a31_autonomous_admission.py')).Path
$readinessHelper=(Resolve-Path (Join-Path $repo 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_wait_for_paused_save.ps1')).Path
$pauseHelper=(Resolve-Path (Join-Path $PSScriptRoot 'Invoke-A29AuthenticatedClockTransition.ps1')).Path
$bindingPaths=[ordered]@{mod_state='mod_state.json';dlc_state='dlc_state.json';load_order='load_order.json';source_inventory='source_inventory.json';resolution_report='resolution_report.json'}
foreach($key in @($bindingPaths.Keys)){$path=Join-Path $resolvedBindings $bindingPaths[$key];if(-not(Test-Path $path -PathType Leaf)){throw "A31 static binding missing: $path"};$bindingPaths[$key]=(Resolve-Path $path).Path}
$bundleDir=Split-Path -Parent $resolvedBundle
$preflightOutput=@(& $pythonExe $staticVerifier verify --bundle $bundleDir --exe $resolvedExe --mod-state $bindingPaths.mod_state --dlc-state $bindingPaths.dlc_state --load-order $bindingPaths.load_order --source-inventory $bindingPaths.source_inventory --live-resolution-report $bindingPaths.resolution_report --allow-deferred-token-gate)
if($LASTEXITCODE -ne 0){throw 'A31 static bundle preflight failed.'}
$preflightText=$preflightOutput -join [Environment]::NewLine;$preflight=$preflightText|ConvertFrom-Json
if($preflight.passed -ne $true -or $preflight.verification_phase -cne 'pre_injection_deferred_allowed'){throw 'A31 static bundle preflight did not admit deferred launch.'}
$preflightReceipt=Join-Path $resolvedOut 'a31_static_bundle_preflight.json';Write-AtomicText $preflightReceipt ($preflightText+"`n")

$continuePath=Join-Path $resolvedUserDir 'continue_game.json';$priorExists=Test-Path $continuePath -PathType Leaf;$priorBytes=if($priorExists){[IO.File]::ReadAllBytes($continuePath)}else{$null}
$continue=if($priorExists){Get-Content $continuePath -Raw|ConvertFrom-Json}else{[pscustomobject]@{title=[IO.Path]::GetFileNameWithoutExtension($resolvedSave);desc='';date=[DateTime]::UtcNow.ToString('R');filename='';is_remote=$false}}
$continue.filename=[IO.Path]::GetFileName($resolvedSave);$continue.is_remote=$false;$replacement=($continue|ConvertTo-Json -Depth 8)+"`n"
$sessionId=[guid]::NewGuid().ToString();$captureId=[guid]::NewGuid().ToString()
$arguments=@('--early-launch-exe',$resolvedExe,'--pdx-user-dir',$resolvedUserDir,'--dll',$resolvedCollector,'--out-dir',$resolvedOut,'--session-id',$sessionId,'--capture-id',$captureId,'--duration-s',$DurationSeconds,'--sample-ms',$SampleMilliseconds,'--min-game-hours','1','--stream-format','compact','--profile','debug','--scheduled-domain','country_identity','--scheduled-domains-exact','--static-bundle-manifest',$resolvedBundle,'--static-bundle-mod-state',$bindingPaths.mod_state,'--static-bundle-dlc-state',$bindingPaths.dlc_state,'--static-bundle-load-order',$bindingPaths.load_order,'--static-bundle-source-inventory',$bindingPaths.source_inventory,'--static-bundle-live-resolution-report',$bindingPaths.resolution_report,'--static-bundle-verifier',$staticVerifier,'--static-bundle-python',$pythonExe,'--static-bundle-timeout-ms','300000','--sidecar-log','--hour-latch')
foreach($value in $EarlyLaunchArgument){$arguments+=@('--early-launch-arg',$value)}
$manifest=Join-Path $resolvedOut 'native_session_manifest.json';$readiness=Join-Path $resolvedOut 'a31_authenticated_paused_save_readiness.json';$repause=Join-Path $resolvedOut 'a31_authenticated_terminal_repause.json';$admission=Join-Path $resolvedOut 'a31_autonomous_national_focus_admission.json'
$job=$null;$game=$null;$startUtc=$null;$overridden=$false;$completed=$false;$repauseError=$null;$driverResult=$null
try{
    $overridden=$true;Write-AtomicText $continuePath $replacement -AllowReplace
    $job=Start-Job -ScriptBlock {param($exe,$argv);& $exe @argv;if($LASTEXITCODE-ne 0){throw "collector_record exited $LASTEXITCODE"}} -ArgumentList $resolvedRecorder,(,$arguments)
    $deadline=[DateTime]::UtcNow.AddSeconds($ProcessTimeoutSeconds)
    while([DateTime]::UtcNow-lt$deadline-and$null-eq$game){
        if($job.State-in@('Failed','Stopped','Disconnected')){throw "collector_record ended before A31 process readiness: $($job.State)"}
        $candidates=@(Get-Process the game -ErrorAction SilentlyContinue);if($candidates.Count-gt 1){throw 'A31 clean launch produced multiple hoi4.exe processes.'}
        foreach($candidate in $candidates){try{$path=$candidate.MainModule.FileName;$loaded=@($candidate.Modules|Where-Object{[IO.Path]::GetFullPath($_.FileName).Equals($resolvedCollector,[StringComparison]::OrdinalIgnoreCase)});if($candidate.MainWindowHandle-ne[IntPtr]::Zero-and[IO.Path]::GetFullPath($path).Equals($resolvedExe,[StringComparison]::OrdinalIgnoreCase)-and(Get-FileHash $path -Algorithm SHA256).Hash.ToLowerInvariant()-ceq$expectedExeSha256-and$loaded.Count-eq 1){$game=$candidate;break}}catch{}}
        if($null-eq$game){Start-Sleep -Milliseconds 250}
    }
    if($null-eq$game){throw 'A31 did not observe the authenticated exact-a729 process/window/collector tuple.'}
    $startUtc=$game.StartTime.ToUniversalTime().ToString('o');if($InitialSettleSeconds-gt 0){Start-Sleep -Seconds $InitialSettleSeconds}
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $readinessHelper -ProcessId $game.Id -ReceiptPath $readiness -ExpectedGameHourRaw 60830441 -TimeoutSeconds $ProcessTimeoutSeconds -StableSamples 4 -PollMilliseconds 250
    if($LASTEXITCODE-ne 0-or-not(Test-Path $readiness -PathType Leaf)){throw 'A31 paused D04 readiness failed.'}
    if($overridden){if($priorExists){Write-AtomicBytes $continuePath $priorBytes -AllowReplace}else{Remove-Item $continuePath -Force};$overridden=$false}
    $driverOutput=@(& $pythonExe $driver --collector-dll $resolvedCollector --output-root $resolvedOut --action-profile $resolvedProfile)
    if($LASTEXITCODE-ne 0){throw "A31 autonomous fixture driver failed: $($driverOutput -join [Environment]::NewLine)"}
    $driverResult=($driverOutput -join [Environment]::NewLine)|ConvertFrom-Json
    Wait-Job $job|Out-Null;$state=$job.State;Receive-Job $job -ErrorAction Continue|ForEach-Object{Write-Verbose([string]$_)};if($state-ne'Completed'){throw "collector_record ended in state $state"};$completed=$true
}finally{
    if($null-ne$game-and$null-ne(Get-Process -Id $game.Id -ErrorAction SilentlyContinue)){
        try{& powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $pauseHelper -Action Repause -ProcessId $game.Id -ExpectedProcessStartUtc $startUtc -SessionId $sessionId -CaptureId $captureId -ReceiptPath $repause;if($LASTEXITCODE-ne 0){throw 'terminal repause rejected'}}catch{$repauseError=$_.Exception.Message}
    }
    if($null-ne$job){if($job.State-notin@('Completed','Failed','Stopped','Disconnected')){if(-not$completed){Stop-Job $job -ErrorAction SilentlyContinue};Wait-Job $job|Out-Null};Receive-Job $job -ErrorAction SilentlyContinue|Out-Null;Remove-Job $job -Force -ErrorAction SilentlyContinue}
    if($overridden){if($priorExists){Write-AtomicBytes $continuePath $priorBytes -AllowReplace}elseif(Test-Path $continuePath){Remove-Item $continuePath -Force}}
    if($null-ne$repauseError){throw "A31 unconditional repause failed: $repauseError"}
}
if($null-eq$driverResult-or-not(Test-Path $manifest -PathType Leaf)){throw 'A31 driver result or native manifest is absent.'}
$actionReceipt=[string]$driverResult.action_receipt_path;$pair=[string]$driverResult.pair_result
& $pythonExe $admissionVerifier --capture-root $resolvedOut --capture-manifest $manifest --prebuilt-attestation $resolvedAttestation --static-bundle-preflight $preflightReceipt --readiness-receipt $readiness --terminal-repause $repause --action-profile $resolvedProfile --action-receipt $actionReceipt --pair-regrade $pair --continue-save $resolvedSave --collector-dll $resolvedCollector --recorder-exe $resolvedRecorder --out $admission
if($LASTEXITCODE-ne 0-or-not(Test-Path $admission -PathType Leaf)){throw "A31 immutable admission failed: $admission"}
Write-Output "A31 autonomous national-focus fixture pair admitted: $admission"
