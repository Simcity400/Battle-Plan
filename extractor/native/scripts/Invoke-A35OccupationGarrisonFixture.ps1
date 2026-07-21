[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $RecorderExe,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $PdxUserDir,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $OccupationOracle,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [int] $DurationSeconds = 300,
    [int] $SampleMilliseconds = 100,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
foreach ($path in @($RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll, $OccupationOracle)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required path does not exist: $path" }
}
if ($DurationSeconds -lt 60) { throw 'DurationSeconds must be at least 60.' }
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
    throw 'A35 evidence requires a clean early launch; close the existing hoi4.exe first.'
}
$recorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$exe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$userDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$dll = (Resolve-Path -LiteralPath $CollectorDll).Path
$oracle = (Resolve-Path -LiteralPath $OccupationOracle).Path
$out = [System.IO.Path]::GetFullPath($OutDir)
if ((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) {
    throw 'Hoi4Exe does not match exact a729 authority.'
}
if (Test-Path -LiteralPath $out) {
    if (-not (Test-Path -LiteralPath $out -PathType Container) -or
        @(Get-ChildItem -LiteralPath $out -Force).Count -ne 0) {
        throw "OutDir must be absent or empty: $out"
    }
} else {
    [System.IO.Directory]::CreateDirectory($out) | Out-Null
}
$repo = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$verifier = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\XUI16\verify_a35_live_fixture.py'
$manifest = Join-Path $out 'native_session_manifest.json'
$verification = Join-Path $out 'a35_live_owner_fixture_verification.json'
$arguments = @(
    '--early-launch-exe', $exe,
    '--pdx-user-dir', $userDir,
    '--dll', $dll,
    '--out-dir', $out,
    '--duration-s', $DurationSeconds,
    '--sample-ms', $SampleMilliseconds,
    '--stream-format', 'compact',
    '--profile', 'debug',
    '--scheduled-domain', 'occupation_resistance_compliance_live',
    '--scheduled-domains-exact',
    '--sidecar-log',
    '--hour-latch'
)
foreach ($value in $EarlyLaunchArgument) { $arguments += @('--early-launch-arg', $value) }
Write-Output @'
A35 active fixture: use the exact save that produced OccupationOracle. After load,
keep the game paused, hover the visible insufficient-garrison alert until its full
delayed manpower and equipment detail is displayed, and leave it visible through
at least one d52 poll.
The verifier rejects predicate-only, wrong-hour, wrong-owner, or incomplete history.
'@
& $recorder @arguments
if ($LASTEXITCODE -ne 0) { throw "collector_record exited with code $LASTEXITCODE" }
if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) {
    throw "Recorder manifest missing: $manifest"
}
$manifestObject = Get-Content -LiteralPath $manifest -Raw | ConvertFrom-Json
$replayName = [string] $manifestObject.replay_file
if ([string]::IsNullOrWhiteSpace($replayName) -or
    [System.IO.Path]::GetFileName($replayName) -cne $replayName) {
    throw 'Recorder manifest replay_file is absent or not a direct filename.'
}
$replay = Join-Path $out $replayName
& $Python $verifier --replay $replay --occupation-oracle $oracle --out $verification
if ($LASTEXITCODE -ne 0) { throw "A35 live owner fixture rejected; inspect $verification" }
Write-Output "A35 live owner fixture accepted: $verification"
