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
    [string] $SceneProfile = '',
    [int] $DurationSeconds = 600,
    [int] $SampleMilliseconds = 25,
    [int] $ProcessTimeoutSeconds = 300,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$expectedSaveSha256 = 'c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735'
$expectedInitialHour = 60830441

foreach ($path in @($RecorderExe,$Hoi4Exe,$PdxUserDir,$CollectorDll,$ContinueSave,$StaticBundleManifest,$StaticBundleBindingsDir)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required path does not exist: $path" }
}
if ($DurationSeconds -lt 300 -or $DurationSeconds -gt 1800) { throw 'DurationSeconds must be in 300..1800.' }
if ($SampleMilliseconds -lt 10 -or $SampleMilliseconds -gt 1000) { throw 'SampleMilliseconds must be in 10..1000.' }
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) { throw 'A24 requires a clean early launch; close every existing hoi4.exe.' }

$recorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$exe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$userDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$collector = (Resolve-Path -LiteralPath $CollectorDll).Path
$save = (Resolve-Path -LiteralPath $ContinueSave).Path
$staticManifest = (Resolve-Path -LiteralPath $StaticBundleManifest).Path
$bindings = (Resolve-Path -LiteralPath $StaticBundleBindingsDir).Path
$output = [IO.Path]::GetFullPath($OutDir)
$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\..')).Path
$lane = Join-Path $repo 'extractor\native\lanes\politics_tooltip_inputs_live'
$driver = Join-Path $lane 'capture_a24_autonomous_scenes.py'
$regrader = Join-Path $lane 'regrade_a24_live_fixture.py'
$admitter = Join-Path $lane 'verify_a24_autonomous_admission.py'
$readiness = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_wait_for_paused_save.ps1'
$productionDriver = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Start-Hoi4ProductionCaptureSafe.ps1')).Path
$attestationHelpers = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'ProductionArtifact.Attestation.ps1')).Path
if ([string]::IsNullOrWhiteSpace($SceneProfile)) { $SceneProfile = Join-Path $lane 'A24_AUTONOMOUS_SCENES_1920X1080.json' }
$profile = (Resolve-Path -LiteralPath $SceneProfile).Path
if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) { $PrebuiltArtifactAttestationPath = Join-Path $repo '.cargo-target\release\hoi4-production-prebuilt-artifacts.json' }
. $attestationHelpers
$prebuilt = Test-ProductionArtifactAttestation -RepoRoot $repo -ReceiptPath $PrebuiltArtifactAttestationPath -ExpectedDriverPath $productionDriver
$attestation = (Resolve-Path -LiteralPath $prebuilt.receipt_path).Path
if (-not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.recorder_host).Equals($recorder,[StringComparison]::OrdinalIgnoreCase) -or
    -not [IO.Path]::GetFullPath([string]$prebuilt.artifacts.collector_dll).Equals($collector,[StringComparison]::OrdinalIgnoreCase)) {
    throw 'A24 requires recorder and collector from the current central Release attestation.'
}
if ((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) { throw 'Hoi4Exe is not exact a729.' }
if ((Get-FileHash -LiteralPath $save -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedSaveSha256) { throw 'A24 requires frozen New D04 Save Deep War.' }
$saveRoot = [IO.Path]::GetFullPath((Join-Path $userDir 'save games')).TrimEnd([char]'\')
if (-not [IO.Path]::GetFullPath((Split-Path -Parent $save)).TrimEnd([char]'\').Equals($saveRoot,[StringComparison]::OrdinalIgnoreCase)) { throw "ContinueSave must be a direct child of $saveRoot" }
$continueIndex=[Array]::IndexOf($EarlyLaunchArgument,'--continuelastsave'); $gdprIndex=[Array]::IndexOf($EarlyLaunchArgument,'--gdpr-compliant')
if($continueIndex -lt 0 -or $gdprIndex -lt $continueIndex){throw 'EarlyLaunchArgument must contain --continuelastsave followed by --gdpr-compliant.'}
if (Test-Path -LiteralPath $output) {
    if (-not (Test-Path -LiteralPath $output -PathType Container) -or @(Get-ChildItem -LiteralPath $output -Force).Count -ne 0) { throw "OutDir must be absent or empty: $output" }
} else { [IO.Directory]::CreateDirectory($output) | Out-Null }

function Write-AtomicBytes([string]$Path,[byte[]]$Bytes,[switch]$Replace) {
    $full=[IO.Path]::GetFullPath($Path); $parent=[IO.Path]::GetDirectoryName($full); [IO.Directory]::CreateDirectory($parent)|Out-Null
    $tmp=[IO.Path]::Combine($parent,[IO.Path]::GetRandomFileName()+'.tmp')
    try { [IO.File]::WriteAllBytes($tmp,$Bytes); $stream=[IO.File]::Open($tmp,'Open','ReadWrite','None');try{$stream.Flush($true)}finally{$stream.Dispose()}
        if([IO.File]::Exists($full)){if(-not $Replace){throw "Refusing to replace $full"};[IO.File]::Replace($tmp,$full,$null,$true)}else{[IO.File]::Move($tmp,$full)}
    } finally {if([IO.File]::Exists($tmp)){[IO.File]::Delete($tmp)}}
}
function Write-AtomicText([string]$Path,[string]$Text,[switch]$Replace){Write-AtomicBytes $Path ([Text.UTF8Encoding]::new($false).GetBytes($Text)) -Replace:$Replace}
function ConvertTo-NativeArgumentLine([object[]]$Arguments) {
    return (($Arguments | ForEach-Object {
        $value=[string]$_
        if($value -match '[\s"]'){ '"'+($value -replace '(\\*)"','$1$1\"' -replace '(\\+)$','$1$1')+'"' }else{$value}
    }) -join ' ')
}

$bindingPaths=[ordered]@{mod_state='mod_state.json';dlc_state='dlc_state.json';load_order='load_order.json';source_inventory='source_inventory.json';resolution_report='resolution_report.json'}
foreach($key in @($bindingPaths.Keys)){$path=Join-Path $bindings $bindingPaths[$key];if(-not(Test-Path -LiteralPath $path -PathType Leaf)){throw "Missing static binding $key"};$bindingPaths[$key]=(Resolve-Path -LiteralPath $path).Path}
$continuePath=Join-Path $userDir 'continue_game.json';$priorExists=Test-Path -LiteralPath $continuePath -PathType Leaf;$priorBytes=if($priorExists){[IO.File]::ReadAllBytes($continuePath)}else{$null}
$continue=if($priorExists){Get-Content -LiteralPath $continuePath -Raw|ConvertFrom-Json}else{[pscustomobject]@{title='';desc='';date=[DateTime]::UtcNow.ToString('R');filename='';is_remote=$false}}
$continue.filename=[IO.Path]::GetFileName($save);$continue.is_remote=$false;$replacement=($continue|ConvertTo-Json -Depth 8)+[Environment]::NewLine

$session=[guid]::NewGuid().ToString();$capture=[guid]::NewGuid().ToString()
$arguments=@('--early-launch-exe',$exe,'--pdx-user-dir',$userDir,'--dll',$collector,'--out-dir',$output,'--session-id',$session,'--capture-id',$capture,'--duration-s',$DurationSeconds,'--sample-ms',$SampleMilliseconds,'--stream-format','compact','--profile','debug','--scheduled-domain','politics_live','--scheduled-domain','politics_residuals','--scheduled-domains-exact','--static-bundle-manifest',$staticManifest,'--static-bundle-mod-state',$bindingPaths.mod_state,'--static-bundle-dlc-state',$bindingPaths.dlc_state,'--static-bundle-load-order',$bindingPaths.load_order,'--static-bundle-source-inventory',$bindingPaths.source_inventory,'--static-bundle-live-resolution-report',$bindingPaths.resolution_report,'--static-bundle-timeout-ms','300000','--sidecar-log')
foreach($value in $EarlyLaunchArgument){$arguments+=@('--early-launch-arg',$value)}
$stdout=Join-Path $output 'collector_record.stdout.log';$stderr=Join-Path $output 'collector_record.stderr.log';$readyReceipt=Join-Path $output 'a24_paused_save_readiness.json';$actions=Join-Path $output 'a24_autonomous_scene_actions.json';$grade=Join-Path $output 'a24_politics_tooltip_live_fixture_regrade.json';$admission=Join-Path $output 'a24_politics_tooltip_immutable_admission.json'
$process=$null;$captureProcess=$null;$continueOverridden=$false;$completed=$false
try {
    $continueOverridden=$true;Write-AtomicText $continuePath $replacement -Replace
    $start=[Diagnostics.ProcessStartInfo]::new();$start.FileName=$recorder;$start.UseShellExecute=$false;$start.CreateNoWindow=$true
    if($null -ne $start.PSObject.Properties['ArgumentList']){foreach($arg in $arguments){[void]$start.ArgumentList.Add([string]$arg)}}else{$start.Arguments=ConvertTo-NativeArgumentLine $arguments}
    $captureProcess=[Diagnostics.Process]::Start($start)
    $deadline=[DateTime]::UtcNow.AddSeconds($ProcessTimeoutSeconds)
    while([DateTime]::UtcNow -lt $deadline -and -not $captureProcess.HasExited -and $null -eq $process){
        foreach($candidate in @(Get-Process -Name the game -ErrorAction SilentlyContinue)){try{$cim=Get-CimInstance Win32_Process -Filter "ProcessId=$($candidate.Id)" -ErrorAction Stop;if([uint32]$cim.ParentProcessId -ne [uint32]$captureProcess.Id){continue};if($candidate.MainWindowHandle -eq [IntPtr]::Zero){continue};$loaded=@($candidate.Modules|Where-Object{[IO.Path]::GetFullPath([string]$_.FileName).Equals($collector,[StringComparison]::OrdinalIgnoreCase)});if($loaded.Count -eq 1){$process=$candidate;break}}catch{}}
        if($null -eq $process){Start-Sleep -Milliseconds 250}
    }
    if($null -eq $process){throw 'A24 did not observe authenticated early-launch process/collector readiness.'}
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $readiness -ProcessId $process.Id -ReceiptPath $readyReceipt -ExpectedGameHourRaw $expectedInitialHour -TimeoutSeconds $ProcessTimeoutSeconds -StableSamples 4 -PollMilliseconds 250
    if($LASTEXITCODE -ne 0){throw 'A24 paused-save readiness failed.'}
    if($continueOverridden){if($priorExists){Write-AtomicBytes $continuePath $priorBytes -Replace}elseif(Test-Path -LiteralPath $continuePath){Remove-Item -LiteralPath $continuePath -Force};$continueOverridden=$false}
    & $Python $driver --process-id $process.Id --collector-dll $collector --profile $profile --output $output
    if($LASTEXITCODE -ne 0){throw 'A24 autonomous scene owner failed.'}
    if(-not $captureProcess.WaitForExit([Math]::Max(60000,($DurationSeconds+300)*1000))){throw 'A24 recorder finalization timed out.'}
    if($captureProcess.ExitCode -ne 0){throw "A24 recorder exited with code $($captureProcess.ExitCode)."}
    $completed=$true
} finally {
    if($continueOverridden){if($priorExists){Write-AtomicBytes $continuePath $priorBytes -Replace}elseif(Test-Path -LiteralPath $continuePath){Remove-Item -LiteralPath $continuePath -Force}}
    if($null -ne $captureProcess -and -not $captureProcess.HasExited){if(-not $captureProcess.WaitForExit(30000)){Stop-Process -Id $captureProcess.Id -Force -ErrorAction SilentlyContinue}}
    if($null -ne $captureProcess){$captureProcess.Dispose()}
}
if(-not $completed){throw 'A24 capture did not complete.'}
$receipts=@(Get-ChildItem -LiteralPath $output -File -Filter 'a24_politics_producer_capture_*.json'|Sort-Object Name)
if($receipts.Count -ne 6){throw "A24 owner emitted $($receipts.Count) cases; exactly six required."}
$regradeArgs=@($regrader);foreach($receipt in $receipts){$regradeArgs+=@('--producer-receipt',$receipt.FullName)};$regradeArgs+=@('--game-exe',$exe,'--output',$grade)
& $Python @regradeArgs;if($LASTEXITCODE -ne 0){throw "A24 exact regrade rejected the capture: $grade"}
& $Python $admitter --actions $actions --regrade $grade --prebuilt-attestation $attestation --game-exe $exe --output $admission
if($LASTEXITCODE -ne 0){throw "A24 immutable admission rejected the capture: $admission"}
Write-Output "A24 autonomous six-surface acquisition admitted: $admission"
