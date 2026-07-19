param(
    [Parameter(Mandatory = $true)][string]$RunDir,
    [Parameter(Mandatory = $true)][string]$SpeedProofPath,
    [Parameter(Mandatory = $true)][string]$StaticBundleDir,
    [Parameter(Mandatory = $true)][string]$GameExePath,
    [Parameter(Mandatory = $true)][string]$ModStatePath,
    [Parameter(Mandatory = $true)][string]$DlcStatePath,
    [Parameter(Mandatory = $true)][string]$LoadOrderPath,
    [Parameter(Mandatory = $true)][string]$SourceInventoryPath,
    [Parameter(Mandatory = $true)][string]$LiveResolutionReportPath,
    [Parameter(Mandatory = $true)][string]$CapturePlanPath,
    [Parameter(Mandatory = $true)][string]$SpeedRootGatePath,
    [Parameter(Mandatory = $true)][string]$SpeedEvidenceTranscriptPath,
    [Parameter(Mandatory = $true)][string]$SpeedEvidenceNonceLedgerRoot,
    [Parameter(Mandatory = $true)][string]$ExpectedSpeedEvidenceVerifierSha256,
    [Parameter(Mandatory = $true)][string]$ExpectedSpeedEvidenceProbeSha256,
    [Parameter(Mandatory = $true)][string]$ExpectedSpeedStaticDisasmSha256,
    [Parameter(Mandatory = $true)][string]$CollectorControlPath,
    [Parameter(Mandatory = $true)][string]$SpeedMonitorPath,
    [string]$PythonExe = "python.exe",
    [string]$ExpectedStagedDll = "",
    [string]$ReportPath = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir "ProductionCapture.Validation.ps1")
$nativeRoot = (Resolve-Path -LiteralPath (Join-Path $scriptDir "..")).Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $nativeRoot "..\..")).Path
$PythonExe=(Get-Command $PythonExe -CommandType Application -ErrorAction Stop).Source
$bundleManifest=Get-Content -LiteralPath (Join-Path $StaticBundleDir 'static_bundle_manifest.json') -Raw | ConvertFrom-Json
$static=@{}
foreach($entry in @($bundleManifest.artifacts)){ $static[[string]$entry.role]=Join-Path $StaticBundleDir ([string]$entry.file) }
$targetRoot = if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) { Join-Path $repoRoot '.cargo-target' } else { [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR) }
$releaseRoot=Join-Path $targetRoot 'release'
$result = Test-ProductionCaptureArtifacts -RunDir $RunDir -RegistryPath (Join-Path $nativeRoot "domain_registry.json") -SpeedProofPath $SpeedProofPath -StaticArtifactPaths $static -ExpectedStagedDll $ExpectedStagedDll -CompactGateExe (Join-Path $releaseRoot 'compact_replay_gate.exe') -FoldCheckExe (Join-Path $releaseRoot 'fold_check.exe') -ContainerVerifyExe (Join-Path $releaseRoot 'pack_replay.exe') -PythonExe $PythonExe -StaticBundleScript (Join-Path $repoRoot 'extractor\game_defs\static_bundle.py') -StaticBundleDir $StaticBundleDir -GameExePath $GameExePath -ModStatePath $ModStatePath -DlcStatePath $DlcStatePath -LoadOrderPath $LoadOrderPath -SourceInventoryPath $SourceInventoryPath -LiveResolutionReportPath $LiveResolutionReportPath -CapturePlanPath $CapturePlanPath -SpeedRootGatePath $SpeedRootGatePath -CollectorControlPath $CollectorControlPath -SpeedMonitorPath $SpeedMonitorPath -SpeedStreamVerifier (Join-Path $scriptDir 'verify_speed5_stream.py') -SpeedEvidenceTranscriptPath $SpeedEvidenceTranscriptPath -SpeedEvidenceNonceLedgerRoot $SpeedEvidenceNonceLedgerRoot -SpeedEvidenceVerifier (Join-Path $scriptDir 'verify_speed_root_evidence_v2.py') -ExpectedSpeedEvidenceVerifierSha256 $ExpectedSpeedEvidenceVerifierSha256 -ExpectedSpeedEvidenceProbeSha256 $ExpectedSpeedEvidenceProbeSha256 -ExpectedSpeedStaticDisasmSha256 $ExpectedSpeedStaticDisasmSha256
if ([string]::IsNullOrWhiteSpace($ReportPath)) { $ReportPath = Join-Path $RunDir "production_validation_report.json" }
$result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $ReportPath -Encoding UTF8
$result | ConvertTo-Json -Depth 8
if (-not $result.ok) { exit 1 }
