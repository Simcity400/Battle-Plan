[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $RecorderExe,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $PdxUserDir,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [int] $DurationSeconds = 1800,
    [int] $SampleMilliseconds = 250,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave'),
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'

foreach ($path in @($RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required path does not exist: $path"
    }
}
if ($DurationSeconds -lt 30) {
    throw 'DurationSeconds must be at least 30.'
}
if ($SampleMilliseconds -lt 10) {
    throw 'SampleMilliseconds must be at least 10.'
}

$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedOut = [System.IO.Path]::GetFullPath($OutDir)
[System.IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
$copiedReceipt = Join-Path $resolvedOut 'a33_sunk_ship_activation_probe.jsonl'
$verification = Join-Path $resolvedOut 'a33_sunk_ship_activation_verification.json'
$captureManifest = Join-Path $resolvedOut 'native_session_manifest.json'
$verifier = Join-Path $PSScriptRoot '..\lanes\sunk_ship_history_live\tools\verify_activation_probe.py'
$verifier = [System.IO.Path]::GetFullPath($verifier)

$arguments = @(
    '--early-launch-exe', (Resolve-Path -LiteralPath $Hoi4Exe).Path,
    '--pdx-user-dir', (Resolve-Path -LiteralPath $PdxUserDir).Path,
    '--dll', $resolvedCollector,
    '--out-dir', $resolvedOut,
    '--duration-s', $DurationSeconds,
    '--sample-ms', $SampleMilliseconds,
    '--stream-format', 'compact',
    '--profile', 'debug',
    '--scheduled-domain', 'naval_combat_lifecycle_live',
    '--scheduled-domain', 'naval_combat_members_live',
    '--scheduled-domains-exact',
    '--sidecar-log',
    '--hour-latch'
)
foreach ($value in $EarlyLaunchArgument) {
    $arguments += @('--early-launch-arg', $value)
}

$prior = [System.Environment]::GetEnvironmentVariable(
    'BATTLE_PLAN_ENABLE_A33_SUNK_SHIP_EVIDENCE',
    [System.EnvironmentVariableTarget]::Process
)
try {
    [System.Environment]::SetEnvironmentVariable(
        'BATTLE_PLAN_ENABLE_A33_SUNK_SHIP_EVIDENCE',
        '1',
        [System.EnvironmentVariableTarget]::Process
    )
    & (Resolve-Path -LiteralPath $RecorderExe).Path @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "collector_record exited with code $LASTEXITCODE"
    }
} finally {
    [System.Environment]::SetEnvironmentVariable(
        'BATTLE_PLAN_ENABLE_A33_SUNK_SHIP_EVIDENCE',
        $prior,
        [System.EnvironmentVariableTarget]::Process
    )
}

if (-not (Test-Path -LiteralPath $captureManifest)) {
    throw "Recorder capture manifest was not produced: $captureManifest"
}
$manifestObject = Get-Content -LiteralPath $captureManifest -Raw | ConvertFrom-Json
$receiptCandidates = [System.Collections.Generic.List[string]]::new()
if ($manifestObject.dll) {
    $receiptCandidates.Add((Join-Path (Split-Path -Parent ([string] $manifestObject.dll)) 'a33_sunk_ship_activation_probe.jsonl'))
}
$receiptCandidates.Add((Join-Path (Split-Path -Parent $resolvedCollector) 'a33_sunk_ship_activation_probe.jsonl'))
Get-ChildItem -LiteralPath $resolvedOut -Recurse -File -Filter 'a33_sunk_ship_activation_probe.jsonl' -ErrorAction SilentlyContinue |
    ForEach-Object { $receiptCandidates.Add($_.FullName) }
$rawReceipt = $receiptCandidates |
    Where-Object { Test-Path -LiteralPath $_ } |
    Sort-Object { (Get-Item -LiteralPath $_).LastWriteTimeUtc } -Descending |
    Select-Object -First 1
if (-not $rawReceipt) {
    throw 'A33 raw activation receipt was not produced beside the loaded collector or under the output directory.'
}
$resolvedRawReceipt = [System.IO.Path]::GetFullPath([string] $rawReceipt)
if (-not [string]::Equals(
    $resolvedRawReceipt,
    [System.IO.Path]::GetFullPath($copiedReceipt),
    [System.StringComparison]::OrdinalIgnoreCase
)) {
    Copy-Item -LiteralPath $resolvedRawReceipt -Destination $copiedReceipt -Force
}
& $Python $verifier $copiedReceipt --capture-manifest $captureManifest --out $verification
if ($LASTEXITCODE -ne 0) {
    throw "A33 activation receipt was rejected; inspect $verification"
}

Write-Output "A33 activation candidate accepted: $verification"
