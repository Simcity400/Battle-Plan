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
    [int] $DurationSeconds = 1200,
    [int] $SampleMilliseconds = 100,
    [int] $ProcessTimeoutSeconds = 420,
    [int] $ExpectedGameHourRaw = 60830441,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path

$core = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'Invoke-A39A40AutonomousEvidenceCore.ps1')).Path
if ([string]::IsNullOrWhiteSpace($ActionProfile)) {
    $ActionProfile = Join-Path $PSScriptRoot '..\..\documents\dossiers\A40\A40_AUTHENTICATED_DESIGN_PEACE_ACTIONS_1920X1080.json'
}
& $core -Owner A40 -RecorderExe $RecorderExe -Hoi4Exe $Hoi4Exe -PdxUserDir $PdxUserDir -CollectorDll $CollectorDll `
    -OutDir $OutDir -ContinueSave $ContinueSave -StaticBundleManifest $StaticBundleManifest -StaticBundleBindingsDir $StaticBundleBindingsDir `
    -ActionProfile $ActionProfile -PrebuiltArtifactAttestationPath $PrebuiltArtifactAttestationPath -DurationSeconds $DurationSeconds `
    -SampleMilliseconds $SampleMilliseconds -ProcessTimeoutSeconds $ProcessTimeoutSeconds -ExpectedGameHourRaw $ExpectedGameHourRaw `
    -EarlyLaunchArgument $EarlyLaunchArgument -Python $Python
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
