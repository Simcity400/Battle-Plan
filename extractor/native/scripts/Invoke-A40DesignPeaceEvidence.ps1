[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $RecorderExe,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $PdxUserDir,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [int] $DurationSeconds = 3600,
    [int] $SampleMilliseconds = 100,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave', '--gdpr-compliant'),
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
foreach ($path in @($RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required path does not exist: $path"
    }
}
if ($DurationSeconds -lt 300) {
    throw 'DurationSeconds must be at least 300 so the human lifecycle can complete.'
}
if ($SampleMilliseconds -lt 25) {
    throw 'SampleMilliseconds must be at least 25.'
}
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
    throw 'A40 evidence requires a clean early launch; close the existing hoi4.exe first.'
}

$resolvedRecorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedOut = [System.IO.Path]::GetFullPath($OutDir)
if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) {
    throw 'Hoi4Exe does not match exact a729 authority.'
}
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or
        @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty so stale A40 evidence cannot be selected: $resolvedOut"
    }
} else {
    [System.IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
}

$repo = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$verifier = Join-Path $repo 'extractor\documents\dossiers\A40\verify_a40_design_peace_live_fixture.py'
if (-not (Test-Path -LiteralPath $verifier -PathType Leaf)) {
    throw "A40 verifier missing: $verifier"
}
$captureManifest = Join-Path $resolvedOut 'native_session_manifest.json'
$copiedReceipt = Join-Path $resolvedOut 'a40_design_peace_evidence_capture.json'
$verification = Join-Path $resolvedOut 'a40_design_peace_live_fixture_regrade.json'
$captureStartedUtc = [DateTime]::UtcNow

$arguments = @(
    '--early-launch-exe', $resolvedExe,
    '--pdx-user-dir', $resolvedUserDir,
    '--dll', $resolvedCollector,
    '--out-dir', $resolvedOut,
    '--duration-s', $DurationSeconds,
    '--sample-ms', $SampleMilliseconds,
    '--stream-format', 'compact',
    '--profile', 'debug',
    '--scheduled-domain', 'fuel_live',
    '--scheduled-domains-exact',
    '--sidecar-log',
    '--skip-host-fold',
    '--a40-design-peace-evidence'
)
foreach ($value in $EarlyLaunchArgument) {
    $arguments += @('--early-launch-arg', $value)
}

Write-Output @'
A40 hooks will be installed before the continued save opens.
During this one exact-a729 process:
  1. Open and make a real selection in division, equipment, tank, plane, and ship designers.
  2. Open variant-upgrade for a real source variant.
  3. Enter one active peace conference, submit a demand, exercise a nonempty contest,
     and continue through a nonempty terminal allocation summary.
Do not restart the game or substitute screenshots/human labels for these transitions.
'@

& $resolvedRecorder @arguments
if ($LASTEXITCODE -ne 0) {
    throw "collector_record exited with code $LASTEXITCODE; retain $resolvedOut for diagnosis."
}
if (-not (Test-Path -LiteralPath $captureManifest -PathType Leaf)) {
    throw "Recorder capture manifest missing: $captureManifest"
}

$manifestObject = Get-Content -LiteralPath $captureManifest -Raw | ConvertFrom-Json
$candidateRoots = [System.Collections.Generic.List[string]]::new()
$candidateRoots.Add($resolvedOut)
$candidateRoots.Add((Split-Path -Parent $resolvedCollector))
if ($manifestObject.dll) {
    $candidateRoots.Add((Split-Path -Parent ([string] $manifestObject.dll)))
}
$candidate = Get-ChildItem -LiteralPath @($candidateRoots | Select-Object -Unique) `
    -Recurse -File -Filter 'a40_design_peace_evidence_capture.json' -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTimeUtc -ge $captureStartedUtc } |
    Sort-Object LastWriteTimeUtc -Descending |
    Select-Object -First 1
if (-not $candidate) {
    throw 'The loaded collector did not produce an A40 evidence receipt.'
}
$candidatePath = [System.IO.Path]::GetFullPath($candidate.FullName)
if (-not [string]::Equals(
    $candidatePath,
    [System.IO.Path]::GetFullPath($copiedReceipt),
    [System.StringComparison]::OrdinalIgnoreCase
)) {
    Copy-Item -LiteralPath $candidatePath -Destination $copiedReceipt
}

& $Python $verifier `
    --receipt $copiedReceipt `
    --capture-manifest $captureManifest `
    --collector-dll $resolvedCollector `
    --out $verification
if ($LASTEXITCODE -ne 0) {
    throw "A40 raw fixture is incomplete or invalid; evidence was preserved and regraded at $verification"
}
Write-Output "A40 raw fixture accepted for post-capture reverse engineering: $verification"
