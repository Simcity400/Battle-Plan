[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $CaptureManifest,
    [Parameter(Mandatory = $true)] [string] $Replay,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $OccupationOracle,
    [Parameter(Mandatory = $true)] [string] $StaticBundlePreflight,
    [Parameter(Mandatory = $true)] [string] $PrebuiltArtifactAttestationPath,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

foreach ($path in @($CaptureManifest, $Replay, $CollectorDll, $OccupationOracle, $StaticBundlePreflight, $PrebuiltArtifactAttestationPath)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Required passive A35 input is missing: $path" }
}
$resolvedOut = [IO.Path]::GetFullPath($OutDir)
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty so stale A35 evidence cannot be selected: $resolvedOut"
    }
} else { [IO.Directory]::CreateDirectory($resolvedOut) | Out-Null }

$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\..')).Path
$resolvedPython = (Get-Command $Python -CommandType Application -ErrorAction Stop).Source
$passiveBinder = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Invoke-A35AutonomousGarrisonAlertAction.ps1')).Path
$verifier = (Resolve-Path -LiteralPath (Join-Path $repo 'replay_interface\re_evidence\rendering_laws\XUI16\verify_a35_live_fixture.py')).Path
$passiveReceipt = Join-Path $resolvedOut 'a35_passive_observer_binding.json'
$verification = Join-Path $resolvedOut 'a35_live_owner_fixture_verification.json'

# This adapter is deliberately incapable of touching the client. It consumes
# only immutable output from an independently running passive collector. The
# historical filename is retained so existing packet references fail safely
# into this observer-only path instead of invoking the retired driver.
& powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $passiveBinder `
    -CaptureManifest $CaptureManifest -Replay $Replay -CollectorDll $CollectorDll `
    -PrebuiltArtifactAttestationPath $PrebuiltArtifactAttestationPath -ReceiptPath $passiveReceipt
if ($LASTEXITCODE -ne 0) { throw 'A35 passive observer binding failed.' }

& $resolvedPython $verifier `
    --replay $Replay --occupation-oracle $OccupationOracle --capture-manifest $CaptureManifest `
    --prebuilt-attestation $PrebuiltArtifactAttestationPath --static-bundle-preflight $StaticBundlePreflight `
    --passive-observer-receipt $passiveReceipt --out $verification
if ($LASTEXITCODE -ne 0) { throw "A35 passive live owner fixture rejected; unobserved state remains unavailable: $verification" }
Write-Output "A35 passive live owner fixture accepted: $verification"
