param(
    [Parameter(Mandatory = $true)][int]$Hoi4Pid,
    [double]$DurationSeconds = 300,
    [int]$MinGameHours = 48,
    [int]$ReaderConcurrency = 6,
    [int]$HourLatchPollMs = 2,
    [int]$HourLatchQueueCap = 64,
    [int]$SpeedMonitorMs = 250,
    [double]$EstimatedMegabytesPerSecond = 70,
    [string]$OutDir = "",
    [string]$SessionId = "",
    [string]$CaptureId = "",
    [Parameter(Mandatory = $true)][string]$SpeedEvidenceTranscriptPath,
    [Parameter(Mandatory = $true)][string]$SpeedEvidenceTranscriptSha256,
    [Parameter(Mandatory = $true)][string]$SpeedEvidenceTranscriptFileSha256,
    [Parameter(Mandatory = $true)][string]$SpeedEvidenceSessionId,
    [Parameter(Mandatory = $true)][string]$SpeedEvidenceNonce,
    [Parameter(Mandatory = $true)][string]$SpeedEvidenceNonceLedgerRoot,
    [Parameter(Mandatory = $true)][string]$SpeedStaticDisasmSha256,
    [string]$PreparedProductModsetDir = "",
    [string]$StaticBundleDir = "",
    [string]$ModStatePath = "",
    [string]$DlcStatePath = "",
    [string]$LoadOrderPath = "",
    [string]$SourceInventoryPath = "",
    [string]$LiveResolutionReportPath = "",
    [string]$EmittedTokensPath = "",
    [string]$TokenOmissionsPath = "",
    [string]$PythonExe = "python.exe",
    [int]$MaxFinalizeSeconds = 1800,
    [int]$CancellationGraceSeconds = 5,
    [string]$AttemptStateRoot = "",
    [string]$PrebuiltArtifactAttestationPath = "",
    [string]$DeploymentProfilePath = "",
    [string]$Rung = "",
    [ValidateSet("production", "synthetic-fault", "paused")][string]$AcceptanceMode = "production",
    [string]$AcceptanceTestNonce = "",
    [switch]$SkipBuild,
    [switch]$DryRun,
    [switch]$AllowOneDriveOutput,
    [switch]$AllowExistingEmptyOutDir
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$nativeRoot = (Resolve-Path -LiteralPath (Join-Path $scriptDir "..")).Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $nativeRoot "..\..")).Path
$targetRoot = Join-Path $repoRoot '.cargo-target'
$env:CARGO_TARGET_DIR = $targetRoot
$releaseRoot = Join-Path $targetRoot "release"
$sourceDll = Join-Path $releaseRoot "collector.dll"
$recorder = Join-Path $releaseRoot "collector_record.exe"
$foldCheck = Join-Path $releaseRoot "fold_check.exe"
$compactGate = Join-Path $releaseRoot "compact_replay_gate.exe"
$packReplay = Join-Path $releaseRoot "pack_replay.exe"
$registryPath = Join-Path $nativeRoot "domain_registry.json"
$profilePath = Join-Path $repoRoot "extractor\live_capture\collector_profiles\vanilla_latest_2026_06_30.json"
$staticBundleScript = Join-Path $repoRoot "extractor\game_defs\static_bundle.py"
$productModsetPrepareScript = Join-Path $repoRoot "extractor\game_defs\prepare_product_modset_capture.py"
$speedStreamVerifier = Join-Path $scriptDir "verify_speed5_stream.py"
$speedEvidenceVerifier = Join-Path $scriptDir "verify_speed_root_evidence_v2.py"
$speedEvidenceProbe = Join-Path $scriptDir "collect_speed_root_evidence_v2.py"
$speedEvidenceSchema = Join-Path $scriptDir "speed_root_evidence_v2.schema.json"
$sinkReconciliationVerifier = Join-Path $scriptDir "verify_sink_reconciliation.py"
$expectedSpeedEvidenceVerifierSha = "1a8057e0084e8507b1c8cde6a7d7c0198652db6efca0bbcb891bc7dc99cb8e28"
$expectedSpeedEvidenceProbeSha = "7302d6981b74b5eba149a58b30e359c9e519e7b7e275273c07ca4214615b406b"
$expectedSpeedEvidenceSchemaSha = "84ff925213ada78f2302fc2115350067dd4975f3ccc6fc295a7ff86c49d22f59"
$expectedHoi4ExeSha = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
. (Join-Path $scriptDir "ProductionCapture.Validation.ps1")
. (Join-Path $scriptDir "ProductionArtifact.Attestation.ps1")
. (Join-Path $scriptDir "WindowsPath.Identity.ps1")
$pythonCommand=Get-Command $PythonExe -CommandType Application -ErrorAction Stop
$PythonExe=$pythonCommand.Source
$preparedModsetReceipt=$null
if (-not [string]::IsNullOrWhiteSpace($PreparedProductModsetDir)) {
    if (@(@($StaticBundleDir,$ModStatePath,$DlcStatePath,$LoadOrderPath,$SourceInventoryPath,$LiveResolutionReportPath) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }).Count -ne 0) {
        throw "-PreparedProductModsetDir is mutually exclusive with individual static bundle paths"
    }
    if (-not (Test-Path -LiteralPath $PreparedProductModsetDir -PathType Container)) { throw "prepared product-modset directory not found: $PreparedProductModsetDir" }
    $PreparedProductModsetDir=(Resolve-Path -LiteralPath $PreparedProductModsetDir).Path
    $preparedReceiptPath=Join-Path $PreparedProductModsetDir "product_modset_capture_ready.json"
    if (-not (Test-Path -LiteralPath $preparedReceiptPath -PathType Leaf)) { throw "prepared product-modset receipt is missing" }
    $preparedModsetReceipt=Get-Content -LiteralPath $preparedReceiptPath -Raw | ConvertFrom-Json
    if ([string]$preparedModsetReceipt.schema -cne 'extractor.product_modset_capture_ready.v1' -or [string]$preparedModsetReceipt.status -cne 'ready_for_selected_modset_capture' -or @('live_binding_resolution','authenticated_binding_corpus') -cnotcontains [string]$preparedModsetReceipt.preparation_mode -or [string]$preparedModsetReceipt.binding_corpus_sha256 -cnotmatch '^[0-9a-f]{64}$' -or [string]$preparedModsetReceipt.binding_corpus.sha256 -cne [string]$preparedModsetReceipt.binding_corpus_sha256 -or [string]$preparedModsetReceipt.product_modset_sha256 -cnotmatch '^[0-9a-f]{64}$' -or [string]$preparedModsetReceipt.preparation_tool_sha256 -cnotmatch '^[0-9a-f]{64}$' -or -not (Test-Path -LiteralPath $productModsetPrepareScript -PathType Leaf) -or (Get-FileHash -LiteralPath $productModsetPrepareScript -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$preparedModsetReceipt.preparation_tool_sha256) {
        throw "prepared product-modset receipt is malformed"
    }
    $StaticBundleDir=Join-Path $PreparedProductModsetDir "static_bundle\bundle"
    $ModStatePath=Join-Path $PreparedProductModsetDir "bindings\mod_state.json"
    $DlcStatePath=Join-Path $PreparedProductModsetDir "bindings\dlc_state.json"
    $LoadOrderPath=Join-Path $PreparedProductModsetDir "bindings\load_order.json"
    $SourceInventoryPath=Join-Path $PreparedProductModsetDir "bindings\source_inventory.json"
    $LiveResolutionReportPath=Join-Path $PreparedProductModsetDir "bindings\resolution_report.json"
} elseif (@(@($StaticBundleDir,$ModStatePath,$DlcStatePath,$LoadOrderPath,$SourceInventoryPath,$LiveResolutionReportPath) | Where-Object { [string]::IsNullOrWhiteSpace($_) }).Count -ne 0) {
    throw "pass -PreparedProductModsetDir or all six individual static bundle paths"
}

function Write-DurableJson {
    param([Parameter(Mandatory=$true)][string]$Path,[Parameter(Mandatory=$true)]$Value)
    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    $temp = "$Path.$([guid]::NewGuid().ToString('N')).tmp"
    $utf8 = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllText($temp, ($Value | ConvertTo-Json -Depth 12), $utf8)
    $stream = [System.IO.File]::Open($temp,[System.IO.FileMode]::Open,[System.IO.FileAccess]::ReadWrite,[System.IO.FileShare]::None)
    try { $stream.Flush($true) } finally { $stream.Dispose() }
    Move-Item -LiteralPath $temp -Destination $Path -Force
}

function Update-AttemptMarker {
    param([string]$Path,[hashtable]$Marker,[string]$State,[hashtable]$Details=@{})
    $Marker.state=$State; $Marker.updated_utc=[DateTime]::UtcNow.ToString('o')
    foreach($entry in $Details.GetEnumerator()){ $Marker[$entry.Key]=$entry.Value }
    Write-DurableJson -Path $Path -Value $Marker
}

if ($DurationSeconds -le 0 -or $MinGameHours -lt 1) { throw "duration and minimum game hours must be positive" }
if ($ReaderConcurrency -lt 1) { throw "reader concurrency must be at least 1" }
if ($HourLatchPollMs -lt 1 -or $HourLatchPollMs -gt 25) { throw "hour latch poll must be 1..25 ms" }
if ($HourLatchQueueCap -lt 1 -or $HourLatchQueueCap -gt 65535) { throw "hour latch queue cap must be 1..65535" }
if ($SpeedMonitorMs -lt 50 -or $SpeedMonitorMs -gt 1000) { throw "speed monitor must be 50..1000 ms" }
if ($MaxFinalizeSeconds -lt 1 -or $CancellationGraceSeconds -lt 1) { throw "finalize and cancellation limits must be positive" }
if ($AcceptanceMode -eq "production") {
    if (-not [string]::IsNullOrWhiteSpace($AcceptanceTestNonce)) { throw "production mode forbids an acceptance-test nonce" }
} else {
    if ($AcceptanceTestNonce -cnotmatch '^[0-9a-f]{64}$') { throw "acceptance test mode requires a 64-character lowercase-hex nonce" }
    $expectedRung = if ($AcceptanceMode -eq "synthetic-fault") { "synthetic-fault" } else { "paused-short-proof" }
    if ($Rung -cne $expectedRung) { throw "acceptance mode $AcceptanceMode requires rung $expectedRung" }
}
$deploymentProfileFile = $null
$deploymentProfileSha256 = $null
$deploymentProfileSourceFile = $null
if ($Rung -ceq "six-hour-soak") {
    if ([string]::IsNullOrWhiteSpace($DeploymentProfilePath) -or -not (Test-Path -LiteralPath $DeploymentProfilePath -PathType Leaf)) {
        throw "six-hour-soak requires the product-selected deployment profile file"
    }
    $DeploymentProfilePath = (Resolve-Path -LiteralPath $DeploymentProfilePath).Path
    $deploymentProfileSourceFile = Split-Path -Leaf $DeploymentProfilePath
    $deploymentProfileFile = "selected_deployment_profile.bin"
    $deploymentProfileSha256 = (Get-FileHash -LiteralPath $DeploymentProfilePath -Algorithm SHA256).Hash.ToLowerInvariant()
} elseif (-not [string]::IsNullOrWhiteSpace($DeploymentProfilePath)) {
    throw "DeploymentProfilePath is accepted only for the six-hour-soak rung"
}
if ([string]::IsNullOrWhiteSpace($AttemptStateRoot)) {
    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) { throw "LOCALAPPDATA is unavailable; pass -AttemptStateRoot" }
    $AttemptStateRoot = Join-Path $env:LOCALAPPDATA "Hoi4MpStats\capture_attempts"
}
$AttemptStateRoot = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($AttemptStateRoot)
if (-not (Test-Path -LiteralPath $SpeedEvidenceTranscriptPath -PathType Leaf)) { throw "speed evidence transcript not found: $SpeedEvidenceTranscriptPath" }
$SpeedEvidenceTranscriptPath=(Resolve-Path -LiteralPath $SpeedEvidenceTranscriptPath).Path
if (-not (Test-Path -LiteralPath $SpeedEvidenceNonceLedgerRoot -PathType Container)) { throw "speed evidence nonce ledger root not found: $SpeedEvidenceNonceLedgerRoot" }
$SpeedEvidenceNonceLedgerRoot=(Resolve-Path -LiteralPath $SpeedEvidenceNonceLedgerRoot).Path
foreach($pin in @($SpeedEvidenceTranscriptSha256,$SpeedEvidenceTranscriptFileSha256,$SpeedEvidenceNonce,$SpeedStaticDisasmSha256)) { if ([string]$pin -cnotmatch '^[0-9a-f]{64}$') { throw "speed evidence pins/nonces must be lowercase SHA-256-shaped values" } }
if ((Get-FileHash -LiteralPath $SpeedEvidenceTranscriptPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $SpeedEvidenceTranscriptFileSha256) { throw "raw speed evidence transcript file hash differs from the caller pin" }
if ($SpeedEvidenceSessionId -cnotmatch '^[A-Za-z0-9_.-]{8,128}$') { throw "speed evidence session id is invalid" }
foreach($toolPin in @{$speedEvidenceVerifier=$expectedSpeedEvidenceVerifierSha;$speedEvidenceProbe=$expectedSpeedEvidenceProbeSha;$speedEvidenceSchema=$expectedSpeedEvidenceSchemaSha}.GetEnumerator()) {
    if (-not (Test-Path -LiteralPath $toolPin.Key -PathType Leaf) -or (Get-FileHash -LiteralPath $toolPin.Key -Algorithm SHA256).Hash.ToLowerInvariant() -cne $toolPin.Value) { throw "canonical speed evidence tooling hash mismatch: $($toolPin.Key)" }
}

$nonce = [guid]::NewGuid().ToString("N")
if ([string]::IsNullOrWhiteSpace($SessionId)) { $SessionId = $SpeedEvidenceSessionId }
elseif ($SessionId -cne $SpeedEvidenceSessionId) { throw "capture session id must equal the independently pinned speed-evidence session id" }
if ([string]::IsNullOrWhiteSpace($CaptureId)) { $CaptureId = "capture-$SessionId" }
if ($CaptureId -cnotmatch '^[A-Za-z0-9_.-]{8,160}$') { throw "capture id is invalid" }
if ([string]::IsNullOrWhiteSpace($OutDir)) {
    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) { throw "LOCALAPPDATA is unavailable; pass an explicit non-OneDrive -OutDir" }
    $OutDir = Join-Path $env:LOCALAPPDATA "Hoi4MpStats\captures\$SessionId"
}
$OutDir = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutDir)
function Assert-CaptureOutputNotSynced {
    param([Parameter(Mandatory = $true)][string]$Candidate)
    if ($AllowOneDriveOutput) { return }

    # Resolve through an opened existing ancestor.  This detects a textual
    # local path whose parent is a junction into OneDrive, including registry-
    # only business accounts absent from the process environment.
    $resolvedCandidate = Resolve-WindowsPathThroughExistingAncestor -LiteralPath $Candidate
    $blockedRoots = [System.Collections.Generic.List[object]]::new()
    foreach ($account in @(Get-OneDriveAccountRoots)) {
        $blockedRoots.Add([pscustomobject]@{
            label = $account.account_label
            final_path = $account.final_path
        })
    }
    $repoIdentity = Get-WindowsPathIdentity -LiteralPath $repoRoot
    $blockedRoots.Add([pscustomobject]@{
        label = 'repository_root'
        final_path = $repoIdentity.final_path
    })
    foreach ($blocked in $blockedRoots) {
        if (Test-WindowsPathContainedByRoot `
            -Candidate $resolvedCandidate -Root $blocked.final_path) {
            throw "capture output resolves under blocked sync/repository identity $($blocked.label); choose a local non-synced -OutDir or explicitly pass -AllowOneDriveOutput"
        }
    }
}
Assert-CaptureOutputNotSynced -Candidate $OutDir
if (Test-Path -LiteralPath $OutDir) {
    # Default (fail-closed) behaviour is unchanged: an existing OutDir is refused so a
    # stale/planted directory can never be adopted. -AllowExistingEmptyOutDir is an
    # explicit opt-in for a caller (the acceptance ladder) that pre-allocates a FRESH,
    # EMPTY run directory of its own: an existing dir is then permitted ONLY when it is
    # a directory AND contains nothing, so the anti-stale-artifact property still holds.
    # A non-empty existing dir is still refused, switch or not.
    $existingEmptyOk = $AllowExistingEmptyOutDir -and (Test-Path -LiteralPath $OutDir -PathType Container) -and (@(Get-ChildItem -LiteralPath $OutDir -Force).Count -eq 0)
    if (-not $existingEmptyOk) { throw "refusing to reuse an existing session directory: $OutDir" }
} else {
    New-Item -ItemType Directory -Path $OutDir | Out-Null
}
# Re-open the now-existing destination and repeat the identity check.  The
# pre-create ancestor check and this post-create check together fail closed if
# a junction/reparse ancestor is swapped during directory creation.
Assert-CaptureOutputNotSynced -Candidate $OutDir
if ($Rung -ceq "six-hour-soak") {
    $retainedDeploymentProfilePath = Join-Path $OutDir $deploymentProfileFile
    [IO.File]::Copy($DeploymentProfilePath, $retainedDeploymentProfilePath, $false)
    if ((Get-FileHash -LiteralPath $retainedDeploymentProfilePath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $deploymentProfileSha256) {
        throw "retained deployment-profile bytes differ from the selected source"
    }
}
if ([string]::IsNullOrWhiteSpace($EmittedTokensPath)) {
    $EmittedTokensPath = Join-Path $OutDir "emitted_token_inventory.json"
} else {
    $EmittedTokensPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($EmittedTokensPath)
}
if (-not [string]::IsNullOrWhiteSpace($TokenOmissionsPath)) {
    if (-not (Test-Path -LiteralPath $TokenOmissionsPath -PathType Leaf)) { throw "token omissions inventory not found: $TokenOmissionsPath" }
    $TokenOmissionsPath = (Resolve-Path -LiteralPath $TokenOmissionsPath).Path
}
$finalizedStaticBundleDir = Join-Path $OutDir "finalized_static_bundle"
$SpeedRootGatePath=Join-Path $OutDir "speed_root_gate_v2.json"
$speedEvidenceAdmission=Invoke-IndependentCaptureGate -Name "speed_root_evidence_v2" -FilePath $PythonExe -Arguments @($speedEvidenceVerifier,"--transcript",$SpeedEvidenceTranscriptPath,"--nonce-ledger-root",$SpeedEvidenceNonceLedgerRoot,"--expected-transcript-sha256",$SpeedEvidenceTranscriptSha256,"--expected-session-id",$SpeedEvidenceSessionId,"--expected-evidence-nonce",$SpeedEvidenceNonce,"--expected-exe-sha256",$expectedHoi4ExeSha,"--expected-probe-tool-sha256",$expectedSpeedEvidenceProbeSha,"--expected-static-disasm-sha256",$SpeedStaticDisasmSha256,"--out",$SpeedRootGatePath) -LogDir (Join-Path $OutDir "preflight_speed_evidence") -TimeoutSeconds 120 -CancellationGraceSeconds $CancellationGraceSeconds
if (-not $speedEvidenceAdmission.passed -or -not (Test-Path -LiteralPath $SpeedRootGatePath -PathType Leaf)) { throw "speed-root evidence v2 admission failed; see $($speedEvidenceAdmission.stderr)" }
$speedRootGate=Get-Content -LiteralPath $SpeedRootGatePath -Raw | ConvertFrom-Json
$expectedGateKeys=@('schema','verdict','transcript_path','transcript_sha256','transcript_file_id','nonce_seal_path','nonce_seal_sha256','verifier_path','verifier_sha256','probe_tool_sha256','static_disasm_sha256','session_id','evidence_nonce_hex','pid','process_creation_filetime_utc','exe_sha256','module_base','speed_sample_sequences','pause_sample_sequences') | Sort-Object
$actualGateKeys=@($speedRootGate.PSObject.Properties.Name | Sort-Object)
if (($actualGateKeys -join ',') -cne ($expectedGateKeys -join ',') -or [string]$speedRootGate.schema -cne 'battle-plan.speed_root_gate.v2' -or [string]$speedRootGate.verdict -cne 'passed') { throw "speed-root verifier emitted a noncanonical v2 gate" }
if ([string]$speedRootGate.transcript_sha256 -cne $SpeedEvidenceTranscriptSha256 -or [string]$speedRootGate.session_id -cne $SpeedEvidenceSessionId -or [string]$speedRootGate.evidence_nonce_hex -cne $SpeedEvidenceNonce -or [int]$speedRootGate.pid -ne $Hoi4Pid -or [string]$speedRootGate.exe_sha256 -cne $expectedHoi4ExeSha -or [string]$speedRootGate.verifier_sha256 -cne $expectedSpeedEvidenceVerifierSha -or [string]$speedRootGate.probe_tool_sha256 -cne $expectedSpeedEvidenceProbeSha -or [string]$speedRootGate.static_disasm_sha256 -cne $SpeedStaticDisasmSha256) { throw "speed-root v2 gate does not match out-of-band pins" }
if ([System.IO.Path]::GetFullPath([string]$speedRootGate.transcript_path) -cne [System.IO.Path]::GetFullPath($SpeedEvidenceTranscriptPath) -or [System.IO.Path]::GetFullPath([string]$speedRootGate.verifier_path) -cne [System.IO.Path]::GetFullPath($speedEvidenceVerifier) -or (@($speedRootGate.speed_sample_sequences) -join ',') -cne '1,2,3,4,5' -or (@($speedRootGate.pause_sample_sequences) -join ',') -cne '6,7,8') { throw "speed-root v2 gate paths/sequences are noncanonical" }
$fileIdKeys=@($speedRootGate.transcript_file_id.PSObject.Properties.Name | Sort-Object)
if (($fileIdKeys -join ',') -cne 'file_index,file_size,volume_serial_number' -or $speedRootGate.transcript_file_id.file_index -is [bool] -or $speedRootGate.transcript_file_id.file_size -is [bool] -or $speedRootGate.transcript_file_id.volume_serial_number -is [bool] -or [int64]$speedRootGate.transcript_file_id.file_size -ne (Get-Item -LiteralPath $SpeedEvidenceTranscriptPath).Length) { throw "speed-root v2 transcript file identity is noncanonical" }
$expectedNonceSealPath=Join-Path $SpeedEvidenceNonceLedgerRoot "$SpeedEvidenceNonce.consumed_success.json"
if ([System.IO.Path]::GetFullPath([string]$speedRootGate.nonce_seal_path) -cne [System.IO.Path]::GetFullPath($expectedNonceSealPath) -or -not (Test-Path -LiteralPath $expectedNonceSealPath -PathType Leaf) -or (Get-FileHash -LiteralPath $expectedNonceSealPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$speedRootGate.nonce_seal_sha256) { throw "speed-root v2 nonce outcome seal is noncanonical" }
$speedRootGateSha=(Get-FileHash -LiteralPath $SpeedRootGatePath -Algorithm SHA256).Hash.ToLowerInvariant()
$retainedTranscriptDir=Join-Path $OutDir "speed_root_evidence_v2";New-Item -ItemType Directory -Path $retainedTranscriptDir | Out-Null
$retainedTranscriptPath=Join-Path $retainedTranscriptDir "transcript.json";Copy-Item -LiteralPath $SpeedEvidenceTranscriptPath -Destination $retainedTranscriptPath
if ((Get-FileHash -LiteralPath $retainedTranscriptPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $SpeedEvidenceTranscriptFileSha256) { throw "retained raw speed evidence transcript file hash mismatch" }

$driveRoot = [System.IO.Path]::GetPathRoot($OutDir)
if ([string]::IsNullOrWhiteSpace($driveRoot) -or $driveRoot.Length -lt 2 -or $driveRoot[1] -ne ':') { throw "capture output must be on a local drive" }
$drive = [System.IO.DriveInfo]::new($driveRoot)
$requiredBytes = [int64]($DurationSeconds * $EstimatedMegabytesPerSecond * 1MB * 1.25)
if ($drive.AvailableFreeSpace -lt $requiredBytes) { throw "insufficient disk: need $requiredBytes bytes, free $($drive.AvailableFreeSpace) bytes" }

if ([string]::IsNullOrWhiteSpace($PrebuiltArtifactAttestationPath)) {
    $PrebuiltArtifactAttestationPath = Join-Path $releaseRoot "hoi4-production-prebuilt-artifacts.json"
}
$prebuiltArtifacts = Test-ProductionArtifactAttestation `
    -RepoRoot $repoRoot `
    -ReceiptPath $PrebuiltArtifactAttestationPath `
    -ExpectedDriverPath $MyInvocation.MyCommand.Path
$sourceDll = [string]$prebuiltArtifacts.artifacts.collector_dll
$recorder = [string]$prebuiltArtifacts.artifacts.recorder_host
$foldCheck = [string]$prebuiltArtifacts.artifacts.fold_check
$compactGate = [string]$prebuiltArtifacts.artifacts.compact_replay_gate
$packReplay = [string]$prebuiltArtifacts.artifacts.container_verify
if (-not $SkipBuild) {
    Write-Host "Using centralized prebuilt artifacts; the capture launcher never invokes Cargo."
}
foreach ($file in @($sourceDll, $recorder, $foldCheck, $compactGate, $packReplay, $registryPath, $profilePath,$staticBundleScript,$speedStreamVerifier,$ModStatePath,$DlcStatePath,$LoadOrderPath,$SourceInventoryPath,$LiveResolutionReportPath)) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "required file missing: $file" }
}

$stageDir = Join-Path $OutDir "staging\$nonce"
New-Item -ItemType Directory -Path $stageDir | Out-Null
$retainedPrebuiltAttestationPath = Join-Path $stageDir "hoi4-production-prebuilt-artifacts.json"
Copy-Item -LiteralPath $prebuiltArtifacts.receipt_path -Destination $retainedPrebuiltAttestationPath
if ((Get-FileHash -LiteralPath $retainedPrebuiltAttestationPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$prebuiltArtifacts.receipt_sha256) {
    throw "retained prebuilt artifact attestation hash mismatch"
}
$stagedDll = Join-Path $stageDir "collector.dll"
$collectorControlPath = Join-Path $stageDir "collector_control.json"
Copy-Item -LiteralPath $sourceDll -Destination $stagedDll
$sourceSha = (Get-FileHash -LiteralPath $sourceDll -Algorithm SHA256).Hash.ToLowerInvariant()
$stagedSha = (Get-FileHash -LiteralPath $stagedDll -Algorithm SHA256).Hash.ToLowerInvariant()
if ($sourceSha -ne $stagedSha -or $sourceSha -cne [string]$prebuiltArtifacts.artifact_sha256.collector_dll) { throw "staged DLL hash mismatch" }

if (-not (Test-Path -LiteralPath $StaticBundleDir -PathType Container)) { throw "static bundle directory missing: $StaticBundleDir" }
$StaticBundleDir=(Resolve-Path -LiteralPath $StaticBundleDir).Path
$ModStatePath=(Resolve-Path -LiteralPath $ModStatePath).Path
$DlcStatePath=(Resolve-Path -LiteralPath $DlcStatePath).Path
$LoadOrderPath=(Resolve-Path -LiteralPath $LoadOrderPath).Path
$SourceInventoryPath=(Resolve-Path -LiteralPath $SourceInventoryPath).Path
$LiveResolutionReportPath=(Resolve-Path -LiteralPath $LiveResolutionReportPath).Path
$bundleManifestPath = Join-Path $StaticBundleDir "static_bundle_manifest.json"
if (-not (Test-Path -LiteralPath $bundleManifestPath -PathType Leaf)) { throw "static bundle manifest missing" }
$bundleManifest = Get-Content -LiteralPath $bundleManifestPath -Raw | ConvertFrom-Json
if (
    [string]$bundleManifest.schema -cne 'extractor.static_bundle.v3' -or
    [string]$bundleManifest.bindings.product_modset.schema -cne 'extractor.product_modset_key.v1' -or
    [string]$bundleManifest.bindings.product_modset.product_modset_sha256 -cnotmatch '^[0-9a-f]{64}$'
) { throw "production capture requires a static-bundle-v3 manifest with a canonical product-modset key" }
$productModsetSha256 = [string]$bundleManifest.bindings.product_modset.product_modset_sha256
if ($null -ne $preparedModsetReceipt) {
    if ($productModsetSha256 -cne [string]$preparedModsetReceipt.product_modset_sha256) {
        throw "prepared product-modset key differs from the static bundle"
    }
    $preparedFiles=@{
        static_bundle=@($bundleManifestPath,$preparedModsetReceipt.static_bundle)
        mod_state=@($ModStatePath,$preparedModsetReceipt.bindings.mod_state)
        dlc_state=@($DlcStatePath,$preparedModsetReceipt.bindings.dlc_state)
        load_order=@($LoadOrderPath,$preparedModsetReceipt.bindings.load_order)
        source_inventory=@($SourceInventoryPath,$preparedModsetReceipt.bindings.source_inventory)
        resolution_report=@($LiveResolutionReportPath,$preparedModsetReceipt.bindings.resolution_report)
        ui14_engine_vfs=@((Join-Path $PreparedProductModsetDir "bindings\ui14_engine_vfs.json"),$preparedModsetReceipt.engine_vfs_receipt)
    }
    foreach($entry in $preparedFiles.GetEnumerator()) {
        $actualPath=[System.IO.Path]::GetFullPath([string]$entry.Value[0])
        $receiptFile=$entry.Value[1]
        $receiptPath=[System.IO.Path]::GetFullPath((Join-Path $PreparedProductModsetDir ([string]$receiptFile.file)))
        if ($actualPath -ine $receiptPath -or [int64]$receiptFile.bytes -ne (Get-Item -LiteralPath $actualPath).Length -or [string]$receiptFile.sha256 -cnotmatch '^[0-9a-f]{64}$' -or (Get-FileHash -LiteralPath $actualPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$receiptFile.sha256) {
            throw "prepared product-modset $($entry.Key) receipt differs from retained bytes"
        }
    }
    $bindingCorpusFiles=@{
        'mod_state.json'=$ModStatePath
        'dlc_state.json'=$DlcStatePath
        'load_order.json'=$LoadOrderPath
        'source_inventory.json'=$SourceInventoryPath
        'resolution_report.json'=$LiveResolutionReportPath
        'ui14_engine_vfs.json'=(Join-Path $PreparedProductModsetDir "bindings\ui14_engine_vfs.json")
    }
    $corpusRows=@($preparedModsetReceipt.binding_corpus.files)
    if ([string]$preparedModsetReceipt.binding_corpus.schema -cne 'extractor.product_modset_binding_corpus.v1' -or $corpusRows.Count -ne $bindingCorpusFiles.Count) {
        throw "prepared product-modset binding corpus is malformed"
    }
    foreach($entry in $bindingCorpusFiles.GetEnumerator()) {
        $matches=@($corpusRows | Where-Object { [string]$_.name -ceq [string]$entry.Key })
        $actualPath=[System.IO.Path]::GetFullPath([string]$entry.Value)
        if ($matches.Count -ne 1 -or [int64]$matches[0].bytes -ne (Get-Item -LiteralPath $actualPath).Length -or [string]$matches[0].sha256 -cnotmatch '^[0-9a-f]{64}$' -or (Get-FileHash -LiteralPath $actualPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne [string]$matches[0].sha256) {
            throw "prepared product-modset binding corpus differs for $($entry.Key)"
        }
    }
}
$static = @{}
foreach ($entry in @($bundleManifest.artifacts)) {
    $role=[string]$entry.role; $path=Join-Path $StaticBundleDir ([string]$entry.file)
    if ($script:RequiredStaticArtifactNames -notcontains $role -or $static.ContainsKey($role)) { throw "static bundle has duplicate/unknown role: $role" }
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "bundled static artifact missing ($role): $path" }
    $static[$role]=$path
}
if ($static.Count -ne $script:RequiredStaticArtifactNames.Count) { throw "static bundle does not contain the exact seven required roles" }

$args = @(
    "--pid", "$Hoi4Pid", "--dll", $stagedDll, "--out-dir", $OutDir,
    "--duration-s", ([string]::Format([Globalization.CultureInfo]::InvariantCulture, "{0:0.###}", $DurationSeconds)),
    "--sample-ms", "1", "--min-game-hours", "$MinGameHours", "--profile", "production",
    "--reader-concurrency", "$ReaderConcurrency",
    "--session-id", $SessionId, "--capture-id", $CaptureId, "--hour-latch", "--hour-latch-poll-ms", "$HourLatchPollMs",
    "--hour-latch-queue-cap", "$HourLatchQueueCap",
    "--domain57-game-speed-gate-v2", $SpeedRootGatePath,
    "--domain57-game-speed-transcript-file-sha256", $SpeedEvidenceTranscriptFileSha256,
    "--static-bundle-manifest", $bundleManifestPath,
    "--static-bundle-mod-state", $ModStatePath,
    "--static-bundle-dlc-state", $DlcStatePath,
    "--static-bundle-load-order", $LoadOrderPath,
    "--static-bundle-source-inventory", $SourceInventoryPath,
    "--static-bundle-live-resolution-report", $LiveResolutionReportPath,
    "--static-bundle-verifier", $staticBundleScript,
    "--static-bundle-python", $PythonExe,
    "--static-bundle-timeout-ms", "300000"
)
if ($AcceptanceMode -ne "production") {
    $args += @("--acceptance-test-mode", $AcceptanceMode, "--acceptance-test-nonce", $AcceptanceTestNonce)
}
$plan = [ordered]@{
    schema = "battle-plan.production_capture_plan.v1"; capture_id = $CaptureId; session_id = $SessionId; pid = $Hoi4Pid
    injection_attempt_budget = 1; source_dll_sha256 = $sourceSha; staged_dll = $stagedDll
    staged_dll_sha256 = $stagedSha; output_dir = $OutDir; speed_required_raw = 4
    prebuilt_artifact_attestation_path = $retainedPrebuiltAttestationPath
    prebuilt_artifact_attestation_authority_path = $prebuiltArtifacts.receipt_path
    prebuilt_artifact_attestation_sha256 = $prebuiltArtifacts.receipt_sha256
    prebuilt_source_input_sha256 = $prebuiltArtifacts.source_input_sha256
    unpaused_required = ($AcceptanceMode -ne "paused"); hour_latch_required = $true
    domain57_game_speed_gate_v2 = $SpeedRootGatePath; domain57_game_speed_gate_v2_sha256 = $speedRootGateSha
    speed_evidence_transcript_sha256 = $SpeedEvidenceTranscriptSha256; speed_evidence_session_id = $SpeedEvidenceSessionId
    speed_evidence_transcript_file_sha256 = $SpeedEvidenceTranscriptFileSha256
    speed_evidence_nonce = $SpeedEvidenceNonce; speed_evidence_transcript_path = $SpeedEvidenceTranscriptPath
    speed_evidence_nonce_ledger_root = $SpeedEvidenceNonceLedgerRoot
    speed_evidence_transcript_file_id = $speedRootGate.transcript_file_id; retained_speed_evidence_transcript = $retainedTranscriptPath
    speed_evidence_nonce_seal_path = $speedRootGate.nonce_seal_path; speed_evidence_nonce_seal_sha256 = $speedRootGate.nonce_seal_sha256
    static_bundle = $StaticBundleDir
    static_bundle_source_inventory = $SourceInventoryPath
    static_bundle_source_inventory_sha256 = (Get-FileHash -LiteralPath $SourceInventoryPath -Algorithm SHA256).Hash.ToLowerInvariant()
    static_bundle_live_resolution_report = $LiveResolutionReportPath
    static_bundle_live_resolution_report_sha256 = (Get-FileHash -LiteralPath $LiveResolutionReportPath -Algorithm SHA256).Hash.ToLowerInvariant()
    static_bundle_manifest_sha256 = (Get-FileHash -LiteralPath $bundleManifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
    product_modset_sha256 = $productModsetSha256
    static_bundle_phase = "pre_injection_deferred"
    finalized_static_bundle = $finalizedStaticBundleDir
    emitted_token_inventory = $EmittedTokensPath
    token_omissions_inventory = $TokenOmissionsPath
    production_profile_domain_ids = @(Get-RegistryCaptureDomains $registryPath | ForEach-Object { [int]$_.id })
    recorder_arguments = $args
}
if ($AcceptanceMode -ne "production") {
    $plan["acceptance_mode"] = $AcceptanceMode
    $plan["acceptance_test_nonce"] = $AcceptanceTestNonce
    $plan["admissible_for_production"] = $false
}
# Optional rung-identity binding (additive): when the acceptance ladder drives this
# launcher for a specific F6 rung it passes -Rung, which is recorded in the plan so the
# produced run is BOUND to that rung (Invoke-AcceptanceLadder Get-RunDirBoundRung). Absent
# (default) the plan carries no `rung` key and every prior behaviour is unchanged.
if (-not [string]::IsNullOrWhiteSpace($Rung)) { $plan["rung"] = $Rung }
if ($Rung -ceq "six-hour-soak") {
    $plan["deployment_profile_source_file"] = $deploymentProfileSourceFile
    $plan["deployment_profile_file"] = $deploymentProfileFile
    $plan["deployment_profile_sha256"] = $deploymentProfileSha256
}
$planPath = Join-Path $OutDir "production_capture_plan.json"
Write-DurableJson -Path $planPath -Value $plan
if ($DryRun) { Write-Host "DRY RUN: one recorder invocation planned; no process opened and no injection performed"; exit 0 }

if (-not ('Hoi4ProductionCapture.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace Hoi4ProductionCapture {
  public static class Native {
    [DllImport("kernel32.dll", SetLastError=true)] public static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll", SetLastError=true)] public static extern bool ReadProcessMemory(IntPtr process, IntPtr address, byte[] buffer, UIntPtr size, out UIntPtr read);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr handle);
  }
}
'@
}
function Read-LiveBytes([IntPtr]$Handle, [Int64]$Address, [int]$Count) {
    $buffer = [byte[]]::new($Count); [UIntPtr]$read = [UIntPtr]::Zero
    if (-not [Hoi4ProductionCapture.Native]::ReadProcessMemory($Handle, [IntPtr]$Address, $buffer, [UIntPtr]$Count, [ref]$read) -or $read.ToUInt64() -ne [uint64]$Count) { throw ("ReadProcessMemory failed at 0x{0:X}" -f $Address) }
    return $buffer
}
function Get-LiveSample([IntPtr]$Handle, [Int64]$Base) {
    $gs = [BitConverter]::ToInt64((Read-LiveBytes $Handle ($Base + 0x3316E90) 8), 0)
    if ($gs -lt 0x10000 -or ($gs -band 7) -ne 0) { throw "invalid CGameState pointer" }
    $clockA = [BitConverter]::ToInt64((Read-LiveBytes $Handle ($Base + 0x33172E0) 8), 0)
    $clockB = [BitConverter]::ToInt64((Read-LiveBytes $Handle ($Base + 0x33172E8) 8), 0)
    if ($clockA -ne $clockB -or $clockA -lt 0x10000) { throw "clock singleton disagreement" }
    $clockVtable = [BitConverter]::ToInt64((Read-LiveBytes $Handle $clockA 8), 0)
    if ($clockVtable -ne ($Base + 0x2952510)) { throw "clock vtable sentinel mismatch" }
    [pscustomobject]@{
        observed_utc = [DateTime]::UtcNow.ToString("o")
        speed_raw = [BitConverter]::ToInt32((Read-LiveBytes $Handle ($gs + 0x4BC) 4), 0)
        paused = ((Read-LiveBytes $Handle ($clockA + 0x6C1) 1)[0] -ne 0)
        game_hour_raw = [BitConverter]::ToInt32((Read-LiveBytes $Handle ($gs + 0x468) 4), 0)
    }
}

$game = Get-Process -Id $Hoi4Pid -ErrorAction Stop
if ($game.ProcessName -ne "the game") { throw "PID $Hoi4Pid is not hoi4.exe" }
$exePath = $game.MainModule.FileName
$base = $game.MainModule.BaseAddress.ToInt64()
$profile = Get-Content -LiteralPath $profilePath -Raw | ConvertFrom-Json
$exeSha = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToUpperInvariant()
if ($exeSha -ne ([string]$profile.exe_sha256).ToUpperInvariant()) { throw "hoi4.exe hash does not match the supported extractor profile" }
if ($exeSha -ne ([string]$speedRootGate.exe_sha256).ToUpperInvariant()) { throw "speed-root gate is for a different hoi4.exe hash" }
$processCreationFiletimeUtc = $game.StartTime.ToUniversalTime().ToFileTimeUtc()
if ([int64]$speedRootGate.process_creation_filetime_utc -ne $processCreationFiletimeUtc -or [Convert]::ToInt64(([string]$speedRootGate.module_base).Substring(2),16) -ne $base) { throw "speed-root gate is for a different the game process identity/module" }
$liveResolution = Get-Content -LiteralPath $LiveResolutionReportPath -Raw | ConvertFrom-Json
if (
    [int]$liveResolution.process.pid -ne $Hoi4Pid -or
    [int64]$liveResolution.process.creation_filetime_utc -ne $processCreationFiletimeUtc
) { throw "static live-resolution report is for a different the game process identity" }
if ($null -ne $preparedModsetReceipt -and (
    [int]$preparedModsetReceipt.process.pid -ne $Hoi4Pid -or
    [int64]$preparedModsetReceipt.process.creation_filetime_utc -ne $processCreationFiletimeUtc -or
    [string]$preparedModsetReceipt.process.executable_sha256 -cne $exeSha.ToLowerInvariant()
)) { throw "prepared product-modset receipt is for a different the game process identity" }
$plan["exe_sha256"]=$exeSha.ToLowerInvariant()
$plan["domain57_game_speed_gate_v2_sha256"]=$speedRootGateSha
$plan["process_start_utc"]=$game.StartTime.ToUniversalTime().ToString('o')
$plan["process_creation_filetime_utc"]=$processCreationFiletimeUtc
Write-DurableJson -Path $planPath -Value $plan
$planSha=(Get-FileHash -LiteralPath $planPath -Algorithm SHA256).Hash.ToLowerInvariant()

$preflightStatic = Invoke-IndependentCaptureGate -Name "preflight_static_bundle" -FilePath $PythonExe -Arguments @($staticBundleScript,"verify","--bundle",$StaticBundleDir,"--exe",$exePath,"--mod-state",$ModStatePath,"--dlc-state",$DlcStatePath,"--load-order",$LoadOrderPath,"--source-inventory",$SourceInventoryPath,"--live-resolution-report",$LiveResolutionReportPath,"--allow-deferred-token-gate") -LogDir (Join-Path $OutDir "preflight") -TimeoutSeconds 900 -CancellationGraceSeconds $CancellationGraceSeconds
if (-not $preflightStatic.passed) { throw "static bundle failed binding verification; see $($preflightStatic.stderr)" }

New-Item -ItemType Directory -Path $AttemptStateRoot -Force | Out-Null
$attemptMarkerPath=Join-Path $AttemptStateRoot ("hoi4_pid_{0}.unresolved.json" -f $Hoi4Pid)
$resolvedMarker=Join-Path $AttemptStateRoot ("$SessionId.resolved.json")
if (Test-Path -LiteralPath $attemptMarkerPath -PathType Leaf) { throw "unresolved prior injection attempt exists: $attemptMarkerPath; use Resolve-Hoi4ProductionCaptureAttempt.ps1 after proving the child/module is gone" }
if (Test-Path -LiteralPath $resolvedMarker) { throw "resolved attempt destination already exists: $resolvedMarker; refusing to overwrite retained evidence" }
$attemptMarker=@{
    schema="battle-plan.production_capture_attempt.v1"; state="committed_pre_launch"; session_id=$SessionId; capture_id=$CaptureId; pid=$Hoi4Pid
    process_start_utc=$game.StartTime.ToUniversalTime().ToString('o'); exe_sha256=$exeSha.ToLowerInvariant()
    staged_dll=$stagedDll; staged_dll_sha256=$stagedSha; capture_plan_sha256=$planSha
    prebuilt_artifact_attestation_path=$retainedPrebuiltAttestationPath; prebuilt_artifact_attestation_sha256=$prebuiltArtifacts.receipt_sha256; prebuilt_source_input_sha256=$prebuiltArtifacts.source_input_sha256
    domain57_game_speed_gate_v2=$SpeedRootGatePath; domain57_game_speed_gate_v2_sha256=$speedRootGateSha
    speed_evidence_session_id=$SpeedEvidenceSessionId; speed_evidence_process_creation_filetime_utc=$speedRootGate.process_creation_filetime_utc; speed_evidence_module_base=$speedRootGate.module_base
    speed_evidence_transcript_path=$SpeedEvidenceTranscriptPath; speed_evidence_transcript_sha256=$SpeedEvidenceTranscriptSha256; speed_evidence_transcript_file_sha256=$SpeedEvidenceTranscriptFileSha256
    speed_evidence_transcript_file_id=$speedRootGate.transcript_file_id; retained_speed_evidence_transcript=$retainedTranscriptPath
    speed_evidence_nonce=$SpeedEvidenceNonce; speed_evidence_nonce_ledger_root=$SpeedEvidenceNonceLedgerRoot
    speed_evidence_nonce_seal_path=$speedRootGate.nonce_seal_path; speed_evidence_nonce_seal_sha256=$speedRootGate.nonce_seal_sha256
    created_utc=[DateTime]::UtcNow.ToString('o'); updated_utc=[DateTime]::UtcNow.ToString('o')
}
Write-DurableJson -Path $attemptMarkerPath -Value $attemptMarker

$handle = [Hoi4ProductionCapture.Native]::OpenProcess(0x410, $false, [uint32]$Hoi4Pid)
if ($handle -eq [IntPtr]::Zero) { Update-AttemptMarker $attemptMarkerPath $attemptMarker "prelaunch_failed" @{failure="OpenProcess failed"}; throw "OpenProcess(PROCESS_QUERY_INFORMATION|PROCESS_VM_READ) failed" }
$speedLog = Join-Path $OutDir "speed5_monitor.jsonl"
$stdoutLog = Join-Path $OutDir "collector_record.stdout.log"
$stderrLog = Join-Path $OutDir "collector_record.stderr.log"
$argumentLine = ($args | ForEach-Object {
    if ($_ -match '[\s"]') {
        $escaped = $_ -replace '"', '\"'
        '"' + $escaped + '"'
    } else { $_ }
}) -join ' '
$process=$null
$recorderStdoutTask=$null; $recorderStderrTask=$null
$samples = [System.Collections.Generic.List[object]]::new()
$monitorWriter=$null
$expectedPaused = ($AcceptanceMode -eq "paused")
try {
    $dailyExpected = @(([string]$profile.hooks.daily_update.sentinel_bytes -split ' ') | ForEach-Object { [Convert]::ToByte($_,16) })
    $dailyActual = @(Read-LiveBytes $handle ($base + 0x1D4390) 16)
    if (($dailyActual -join ',') -ne ($dailyExpected -join ',')) { throw "daily-update profile sentinel mismatch" }
    $monitorWriter=[System.IO.StreamWriter]::new($speedLog,$false,[System.Text.UTF8Encoding]::new($false)); $monitorWriter.AutoFlush=$true
    1..3 | ForEach-Object {
        $sample = Get-LiveSample $handle $base
        $samples.Add($sample); $monitorWriter.WriteLine(($sample | ConvertTo-Json -Compress))
        if ($sample.speed_raw -ne 4 -or [bool]$sample.paused -ne $expectedPaused) { throw "preflight speed/pause state does not match acceptance mode $AcceptanceMode" }
        Start-Sleep -Milliseconds 100
    }
    $prelaunchArtifacts = Test-ProductionArtifactAttestation `
        -RepoRoot $repoRoot `
        -ReceiptPath $PrebuiltArtifactAttestationPath `
        -ExpectedDriverPath $MyInvocation.MyCommand.Path
    if ([string]$prelaunchArtifacts.receipt_sha256 -cne [string]$prebuiltArtifacts.receipt_sha256) {
        throw "prebuilt artifact attestation changed after capture planning"
    }
    # This is the sole recorder launch and therefore the sole injection attempt.
    $recorderStart=[System.Diagnostics.ProcessStartInfo]::new()
    $recorderStart.FileName=$recorder; $recorderStart.Arguments=$argumentLine
    $recorderStart.UseShellExecute=$false; $recorderStart.CreateNoWindow=$true
    $recorderStart.RedirectStandardOutput=$true; $recorderStart.RedirectStandardError=$true
    $process=[System.Diagnostics.Process]::new(); $process.StartInfo=$recorderStart
    if (-not $process.Start()) { throw "collector_record process start returned false" }
    $recorderStdoutTask=$process.StandardOutput.ReadToEndAsync(); $recorderStderrTask=$process.StandardError.ReadToEndAsync()
    Update-AttemptMarker $attemptMarkerPath $attemptMarker "child_started" @{child_pid=$process.Id; child_started_utc=[DateTime]::UtcNow.ToString('o')}
    $deadline=[DateTime]::UtcNow.AddSeconds($DurationSeconds + $MaxFinalizeSeconds)
    while (-not $process.HasExited) {
        if ([DateTime]::UtcNow -ge $deadline) { throw "collector_record exceeded bounded capture/finalize deadline" }
        $sample = Get-LiveSample $handle $base
        $samples.Add($sample); $monitorWriter.WriteLine(($sample | ConvertTo-Json -Compress))
        if ($sample.speed_raw -ne 4 -or [bool]$sample.paused -ne $expectedPaused) { throw "live monitor detected a speed/pause violation for acceptance mode $AcceptanceMode" }
        Start-Sleep -Milliseconds $SpeedMonitorMs
        $process.Refresh()
    }
    [System.IO.File]::WriteAllText($stdoutLog,[string]$recorderStdoutTask.Result,[System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText($stderrLog,[string]$recorderStderrTask.Result,[System.Text.UTF8Encoding]::new($false))
    if ($process.ExitCode -ne 0) { throw "collector_record exited nonzero: $($process.ExitCode)" }
    Update-AttemptMarker $attemptMarkerPath $attemptMarker "child_exited" @{child_exit_code=$process.ExitCode; child_exited_utc=[DateTime]::UtcNow.ToString('o')}
    $monitorWriter.Flush(); $monitorWriter.Dispose(); $monitorWriter=$null
    $manifestPath=Join-Path $OutDir "native_session_manifest.json"
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { throw "collector_record exited without a manifest" }
    $captureManifest=Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if (-not (Test-Path -LiteralPath $collectorControlPath -PathType Leaf)) { throw "recorder did not persist the validated collector_control.json beside the staged DLL" }
    $collectorControlSha=(Get-FileHash -LiteralPath $collectorControlPath -Algorithm SHA256).Hash.ToLowerInvariant()
    Update-AttemptMarker $attemptMarkerPath $attemptMarker "child_exited" @{child_exit_code=$process.ExitCode; collector_control_sha256=$collectorControlSha}
    $monitorSha=(Get-FileHash -LiteralPath $speedLog -Algorithm SHA256).Hash.ToLowerInvariant()
    $proof = [ordered]@{
        schema = "battle-plan.speed5_proof.v2"; session_id=$SessionId; pid = $Hoi4Pid; exe_path = $exePath; exe_sha256 = $exeSha.ToLowerInvariant()
        domain57_game_speed_gate_v2_sha256 = $speedRootGateSha; capture_plan_sha256=$planSha; monitor_sha256=$monitorSha
        collector_control_sha256=$collectorControlSha
        sample_count = $samples.Count; speed_raw = 4; paused = $expectedPaused; acceptance_mode = $AcceptanceMode
        first_game_hour_raw = [int]$samples[0].game_hour_raw; last_game_hour_raw = [int]$samples[$samples.Count-1].game_hour_raw
        capture_first_game_hour_raw=[int]$captureManifest.first_game_hour_raw; capture_last_game_hour_raw=[int]$captureManifest.last_game_hour_raw
    }
    $speedProofPath = Join-Path $OutDir "speed5_proof.json"
    Write-DurableJson -Path $speedProofPath -Value $proof

    if ([string]$captureManifest.session_id -cne $SessionId -or [string]$captureManifest.capture_id -cne $CaptureId) { throw "capture manifest capture/session identity differs from the capture plan" }
    if (
        [string]$captureManifest.static_bundle.product_modset_sha256 -cne $productModsetSha256 -or
        [string]$captureManifest.portable_content_manifest.schema -cne 'hoi4.portable-content-manifest.v2' -or
        [string]$captureManifest.portable_content_manifest.product_modset_sha256 -cne $productModsetSha256
    ) { throw "capture publication lost or changed the admitted product-modset key" }
    $captureStreamSha256 = [string]$captureManifest.host_finalize.stream_sha256
    if ($captureStreamSha256 -cnotmatch '^[0-9a-f]{64}$') { throw "capture manifest has no canonical finalized stream SHA-256" }
    if (-not (Test-Path -LiteralPath $EmittedTokensPath -PathType Leaf)) {
        throw "post-capture emitted token inventory missing: $EmittedTokensPath"
    }
    $finalizeArguments = @(
        $staticBundleScript,"finalize",
        "--bundle",$StaticBundleDir,
        "--out",$finalizedStaticBundleDir,
        "--exe",$exePath,
        "--mod-state",$ModStatePath,
        "--dlc-state",$DlcStatePath,
        "--load-order",$LoadOrderPath,
        "--source-inventory",$SourceInventoryPath,
        "--live-resolution-report",$LiveResolutionReportPath,
        "--emitted-tokens",$EmittedTokensPath,
        "--expected-capture-id",$CaptureId,
        "--expected-session-id",$SessionId,
        "--expected-stream-sha256",$captureStreamSha256
    )
    if (-not [string]::IsNullOrWhiteSpace($TokenOmissionsPath)) {
        $finalizeArguments += @("--omissions",$TokenOmissionsPath)
    }
    $finalizeStatic = Invoke-IndependentCaptureGate -Name "finalize_static_bundle" -FilePath $PythonExe -Arguments $finalizeArguments -LogDir (Join-Path $OutDir "post_capture_static") -TimeoutSeconds $MaxFinalizeSeconds -CancellationGraceSeconds $CancellationGraceSeconds
    if (-not $finalizeStatic.passed) { throw "static bundle finalization failed; see $($finalizeStatic.stderr)" }
    $strictStatic = Invoke-IndependentCaptureGate -Name "verify_finalized_static_bundle" -FilePath $PythonExe -Arguments @($staticBundleScript,"verify","--bundle",$finalizedStaticBundleDir,"--exe",$exePath,"--mod-state",$ModStatePath,"--dlc-state",$DlcStatePath,"--load-order",$LoadOrderPath,"--source-inventory",$SourceInventoryPath,"--live-resolution-report",$LiveResolutionReportPath,"--expected-capture-id",$CaptureId,"--expected-session-id",$SessionId,"--expected-stream-sha256",$captureStreamSha256) -LogDir (Join-Path $OutDir "post_capture_static") -TimeoutSeconds $MaxFinalizeSeconds -CancellationGraceSeconds $CancellationGraceSeconds
    if (-not $strictStatic.passed) { throw "finalized static bundle failed strict provenance verification; see $($strictStatic.stderr)" }
    $StaticBundleDir = (Resolve-Path -LiteralPath $finalizedStaticBundleDir).Path
    $bundleManifestPath = Join-Path $StaticBundleDir "static_bundle_manifest.json"
    $bundleManifest = Get-Content -LiteralPath $bundleManifestPath -Raw | ConvertFrom-Json
    if (
        [string]$bundleManifest.schema -cne 'extractor.static_bundle.v3' -or
        [string]$bundleManifest.bindings.product_modset.product_modset_sha256 -cne $productModsetSha256
    ) { throw "finalized static bundle changed the admitted product-modset key" }
    $static = @{}
    foreach ($entry in @($bundleManifest.artifacts)) {
        $role=[string]$entry.role; $path=Join-Path $StaticBundleDir ([string]$entry.file)
        if ($script:RequiredStaticArtifactNames -notcontains $role -or $static.ContainsKey($role)) { throw "finalized static bundle has duplicate/unknown role: $role" }
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "finalized bundled static artifact missing ($role): $path" }
        $static[$role]=$path
    }
    if ($static.Count -ne $script:RequiredStaticArtifactNames.Count) { throw "finalized static bundle does not contain the exact required roles" }
    Update-AttemptMarker $attemptMarkerPath $attemptMarker "static_finalized" @{finalized_static_bundle=$StaticBundleDir; finalized_static_bundle_id=[string]$bundleManifest.bundle_id; finalized_static_bundle_manifest_sha256=(Get-FileHash -LiteralPath $bundleManifestPath -Algorithm SHA256).Hash.ToLowerInvariant()}
}
catch {
    $failure=$_.Exception.Message
    Update-AttemptMarker $attemptMarkerPath $attemptMarker "failed_unresolved" @{failure=$failure}
    throw
}
finally {
    if ($null -ne $monitorWriter) { try { $monitorWriter.Flush(); $monitorWriter.Dispose() } catch {} }
    if ($null -ne $process -and -not $process.HasExited) {
        $closeRequested=$false; try { $closeRequested=$process.CloseMainWindow() } catch {}
        $graceful=$process.WaitForExit($CancellationGraceSeconds*1000)
        if (-not $graceful) { try { $process.Kill() } catch {}; [void]$process.WaitForExit(5000) }
        try { Update-AttemptMarker $attemptMarkerPath $attemptMarker "failed_unresolved" @{cancellation_requested=$true; close_main_window_requested=$closeRequested; child_exited=$process.HasExited} } catch {}
    }
    [void][Hoi4ProductionCapture.Native]::CloseHandle($handle)
    if ($null -ne $process) { $process.Dispose() }
}

$validationReportPath = Join-Path $OutDir "production_validation_report.json"
$sinkReconciliationReport = $null
if ($AcceptanceMode -eq "production") {
    $validation = Test-ProductionCaptureArtifacts -RunDir $OutDir -RegistryPath $registryPath -SpeedProofPath (Join-Path $OutDir "speed5_proof.json") -StaticArtifactPaths $static -ExpectedStagedDll $stagedDll -CompactGateExe $compactGate -FoldCheckExe $foldCheck -ContainerVerifyExe $packReplay -PythonExe $PythonExe -StaticBundleScript $staticBundleScript -StaticBundleDir $StaticBundleDir -GameExePath $exePath -ModStatePath $ModStatePath -DlcStatePath $DlcStatePath -LoadOrderPath $LoadOrderPath -SourceInventoryPath $SourceInventoryPath -LiveResolutionReportPath $LiveResolutionReportPath -CapturePlanPath $planPath -SpeedRootGatePath $SpeedRootGatePath -CollectorControlPath $collectorControlPath -SpeedMonitorPath $speedLog -SpeedStreamVerifier $speedStreamVerifier -SpeedEvidenceTranscriptPath $SpeedEvidenceTranscriptPath -SpeedEvidenceNonceLedgerRoot $SpeedEvidenceNonceLedgerRoot -SpeedEvidenceVerifier $speedEvidenceVerifier -ExpectedSpeedEvidenceVerifierSha256 $expectedSpeedEvidenceVerifierSha -ExpectedSpeedEvidenceProbeSha256 $expectedSpeedEvidenceProbeSha -ExpectedSpeedStaticDisasmSha256 $SpeedStaticDisasmSha256 -ToolTimeoutSeconds $MaxFinalizeSeconds -CancellationGraceSeconds $CancellationGraceSeconds
    $validation | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $validationReportPath -Encoding UTF8
    if (-not $validation.ok) { throw "production validation failed: $($validation.problems -join ', ')" }
    $sinkReconciliationReport = Join-Path $OutDir "sink_reconciliation_verification.json"
    $sinkGate = Invoke-IndependentCaptureGate -Name "verify_sink_reconciliation" -FilePath $PythonExe -Arguments @($sinkReconciliationVerifier,"--run-dir",$OutDir,"--expect","success","--out",$sinkReconciliationReport) -LogDir (Join-Path $OutDir "sink_reconciliation_logs") -TimeoutSeconds $MaxFinalizeSeconds -CancellationGraceSeconds $CancellationGraceSeconds
    if (-not $sinkGate.passed -or -not (Test-Path -LiteralPath $sinkReconciliationReport -PathType Leaf)) { throw "independent sink reconciliation failed" }
    $sinkResult = Get-Content -LiteralPath $sinkReconciliationReport -Raw | ConvertFrom-Json
    if ([string]$sinkResult.schema -cne "extractor.sink-reconciliation-verification.v1" -or [string]$sinkResult.verdict -cne "passed" -or @($sinkResult.problems).Count -ne 0) {
        throw "sink reconciliation returned a noncanonical pass"
    }
} else {
    $manifest = Get-Content -LiteralPath (Join-Path $OutDir "native_session_manifest.json") -Raw | ConvertFrom-Json
    $replayFile = [string]$manifest.replay_file
    if ([string]::IsNullOrWhiteSpace($replayFile)) { throw "acceptance manifest has no replay_file" }
    $replayPath = Join-Path $OutDir $replayFile
    $snapshotDir = Join-Path $OutDir "acceptance_snapshots"
    $extractionReport = Join-Path $OutDir "acceptance_snapshot_extraction.json"
    $extractGate = Invoke-IndependentCaptureGate -Name "extract_acceptance_snapshots" -FilePath $PythonExe -Arguments @((Join-Path $scriptDir "extract_f6_replay_snapshots.py"),"--replay",$replayPath,"--out-dir",$snapshotDir,"--report",$extractionReport) -LogDir (Join-Path $OutDir "acceptance_proof_logs") -TimeoutSeconds $MaxFinalizeSeconds -CancellationGraceSeconds $CancellationGraceSeconds
    if (-not $extractGate.passed -or -not (Test-Path -LiteralPath $extractionReport -PathType Leaf)) { throw "acceptance snapshot extraction failed" }
    $proofScript = if ($AcceptanceMode -eq "synthetic-fault") { Join-Path $scriptDir "verify_f6_synthetic_fault_proof.py" } else { Join-Path $scriptDir "verify_f6_pause_proof.py" }
    $proofReport = Join-Path $OutDir "acceptance_validation_report.json"
    $proofGate = Invoke-IndependentCaptureGate -Name "verify_acceptance_proof" -FilePath $PythonExe -Arguments @($proofScript,"--run-dir",$OutDir,"--snapshot-dir",$snapshotDir,"--extraction-report",$extractionReport,"--out",$proofReport) -LogDir (Join-Path $OutDir "acceptance_proof_logs") -TimeoutSeconds $MaxFinalizeSeconds -CancellationGraceSeconds $CancellationGraceSeconds
    if (-not $proofGate.passed -or -not (Test-Path -LiteralPath $proofReport -PathType Leaf)) { throw "acceptance proof verification failed" }
    $validation = Get-Content -LiteralPath $proofReport -Raw | ConvertFrom-Json
    $expectedAcceptanceSchema = if ($AcceptanceMode -eq "synthetic-fault") { "battle-plan.f6_synthetic_fault_proof.v1" } else { "battle-plan.f6_pause_proof.v1" }
    if (
        [string]$validation.schema -cne $expectedAcceptanceSchema -or
        $validation.ok -isnot [bool] -or
        $validation.ok -ne $true
    ) { throw "acceptance proof failed or returned a noncanonical verdict: $($validation.problems -join ', ')" }
    $validationReportPath = $proofReport
}
$validationReceiptPath=Join-Path $OutDir "capture_validation_receipt.json"
$transportedResolvedMarkerPath=Join-Path $OutDir "resolved_attempt_marker.json"
$validationState = if ($AcceptanceMode -eq "production") { "validation_passed" } else { "acceptance_validation_passed" }
$manifestPath = Join-Path $OutDir "native_session_manifest.json"
$validationReceipt = [ordered]@{
    schema = "battle-plan.production_capture_validation_receipt.v1"
    session_id = $SessionId
    capture_id = $CaptureId
    acceptance_mode = $AcceptanceMode
    rung = if ([string]::IsNullOrWhiteSpace($Rung)) { $null } else { $Rung }
    capture_plan_path = $planPath
    capture_plan_sha256 = (Get-FileHash -LiteralPath $planPath -Algorithm SHA256).Hash.ToLowerInvariant()
    manifest_path = $manifestPath
    manifest_sha256 = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
    validation_report_path = $validationReportPath
    validation_report_sha256 = (Get-FileHash -LiteralPath $validationReportPath -Algorithm SHA256).Hash.ToLowerInvariant()
    validation_report_schema = [string]$validation.schema
    prebuilt_artifact_attestation_path = $retainedPrebuiltAttestationPath
    prebuilt_artifact_attestation_sha256 = $prebuiltArtifacts.receipt_sha256
    prebuilt_source_input_sha256 = $prebuiltArtifacts.source_input_sha256
    receipt_path = $validationReceiptPath
    resolved_attempt_marker_path = $resolvedMarker
    transported_resolved_attempt_marker_path = $transportedResolvedMarkerPath
    resolved_attempt_marker_schema = "battle-plan.production_capture_attempt.v1"
    resolved_attempt_marker_state = $validationState
    admissible_for_production = ($AcceptanceMode -eq "production")
}
if ($AcceptanceMode -ne "production") {
    $validationReceipt["acceptance_test_nonce"] = $AcceptanceTestNonce
    $validationReceipt["snapshot_extraction_report_path"] = $extractionReport
    $validationReceipt["snapshot_extraction_report_sha256"] = (Get-FileHash -LiteralPath $extractionReport -Algorithm SHA256).Hash.ToLowerInvariant()
    $validationReceipt["replay_path"] = $replayPath
    $validationReceipt["replay_sha256"] = (Get-FileHash -LiteralPath $replayPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $validationReceipt["acceptance_proof"] = $validation
    $validationReceipt["acceptance_test_observation"] = $manifest.acceptance_test_observation
} else {
    $validationReceipt["sink_reconciliation_report_path"] = "sink_reconciliation_verification.json"
    $validationReceipt["sink_reconciliation_report_sha256"] = (Get-FileHash -LiteralPath $sinkReconciliationReport -Algorithm SHA256).Hash.ToLowerInvariant()
}
Write-DurableJson -Path $validationReceiptPath -Value $validationReceipt
$validationReceiptSha=(Get-FileHash -LiteralPath $validationReceiptPath -Algorithm SHA256).Hash.ToLowerInvariant()
Update-AttemptMarker $attemptMarkerPath $attemptMarker $validationState @{validation_report=$validationReportPath; validation_receipt_path=$validationReceiptPath; validation_receipt_sha256=$validationReceiptSha; validation_receipt=$validationReceipt; admissible_for_production=($AcceptanceMode -eq "production")}
Copy-Item -LiteralPath $attemptMarkerPath -Destination $transportedResolvedMarkerPath
$transportedMarkerStream=[System.IO.File]::Open($transportedResolvedMarkerPath,[System.IO.FileMode]::Open,[System.IO.FileAccess]::ReadWrite,[System.IO.FileShare]::None)
try { $transportedMarkerStream.Flush($true) } finally { $transportedMarkerStream.Dispose() }
if ((Get-FileHash -LiteralPath $attemptMarkerPath -Algorithm SHA256).Hash -cne (Get-FileHash -LiteralPath $transportedResolvedMarkerPath -Algorithm SHA256).Hash) { throw "transported resolved-attempt marker hash mismatch" }
Move-Item -LiteralPath $attemptMarkerPath -Destination $resolvedMarker
Write-Host "$AcceptanceMode capture validated: $OutDir"
