<#
Consumes explicit parent-approved package/verification/recording identities.
Prepare validates bytes and emits a plan without extracting or launching.
Run extracts the verified ZIP and launches only its viewer's CLI consumers.
Desktop execution remains the existing Invoke-ViewerDesktopPlaybackValidation seam.
#>
[CmdletBinding()]
param(
    [ValidateSet('Prepare','Run')][string]$Mode = 'Prepare',
    [Parameter(Mandatory=$true)][string]$PackageZip,
    [Parameter(Mandatory=$true)][string]$ExpectedPackageSha256,
    [Parameter(Mandatory=$true)][string]$PackageReceiptPath,
    [Parameter(Mandatory=$true)][string]$ExpectedPackageReceiptSha256,
    [Parameter(Mandatory=$true)][string]$PackageVerificationPath,
    [Parameter(Mandatory=$true)][string]$ExpectedPackageVerificationSha256,
    [Parameter(Mandatory=$true)][string]$RecordingPath,
    [Parameter(Mandatory=$true)][string]$ExpectedRecordingSha256,
    [Parameter(Mandatory=$true)][string]$GameRoot,
    [Parameter(Mandatory=$true)][string]$ExpectedGameExeSha256,
    [Parameter(Mandatory=$true)][string]$UserDataRoot,
    [Parameter(Mandatory=$true)][string]$ExpectedDlcLoadSha256,
    [Parameter(Mandatory=$true)][string]$ExpectedContentAuthoritySha256,
    [Parameter(Mandatory=$true)][string]$ExpectedEffectiveVfsManifestSha256,
    [Parameter(Mandatory=$true)][string]$OutputDirectory,
    [ValidateRange(5,900)][int]$TimeoutSeconds = 60
)
$ErrorActionPreference = 'Stop'
# Keep the request independent of parameter names in the reused desktop script.
$request = @{} + $PSBoundParameters
$request.Mode = $Mode; $request.TimeoutSeconds = $TimeoutSeconds
. (Join-Path $PSScriptRoot 'PortablePackage.Acceptance.ps1')
$result = Invoke-PpaAcceptance $request
$result | ConvertTo-Json -Depth 30
if ($result.status -ceq 'fail') { exit 1 }
exit 0
