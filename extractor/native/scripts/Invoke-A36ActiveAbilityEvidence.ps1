[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $RecorderExe,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $PdxUserDir,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [int] $DurationSeconds = 1800,
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
    throw 'DurationSeconds must be at least 300 so all four fixture cards can be exercised.'
}
if ($SampleMilliseconds -lt 1 -or $SampleMilliseconds -gt 1000) {
    throw 'SampleMilliseconds must be in 1..1000.'
}
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
    throw 'A36 evidence requires a clean early launch; close the existing hoi4.exe first.'
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
        throw "OutDir must be absent or empty so stale A36 evidence cannot be selected: $resolvedOut"
    }
} else {
    [System.IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
}

$repo = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$verifier = Join-Path $repo 'extractor\documents\dossiers\A36\verify_a36_active_ability_live_fixture.py'
$captureManifest = Join-Path $resolvedOut 'native_session_manifest.json'
$copiedReceipt = Join-Path $resolvedOut 'a36_active_ability_evidence_capture.json'
$verification = Join-Path $resolvedOut 'a36_active_ability_live_fixture_regrade.json'
$collectorReceipt = Join-Path (Split-Path -Parent $resolvedCollector) 'a36_active_ability_evidence_capture.json'
if (Test-Path -LiteralPath $collectorReceipt) {
    throw "Move or remove the prior create-new A36 receipt before capture: $collectorReceipt"
}
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
    '--scheduled-domain', 'divisions',
    '--scheduled-domain', 'oob_command',
    '--scheduled-domain', 'characters_leaders_cabinet_live',
    '--scheduled-domains-exact',
    '--sidecar-log',
    '--skip-host-fold',
    '--a36-active-ability-evidence'
)
foreach ($value in $EarlyLaunchArgument) {
    $arguments += @('--early-launch-arg', $value)
}

Write-Output @'
A36 hooks will be installed before the continued save opens. In this one process:
  C01: select a context whose ability list simultaneously has an available row and a
       disabled row, then hover every row so both final tooltip and exact reason are captured;
       keep that selection until the game crosses the next game-hour boundary.
  C02: select a field marshal with at least two subordinate leaders and open/hover its abilities;
       keep it selected through the next game-hour boundary;
       then select one of those exact child leaders and hover the corresponding ability so the
       regrader can prove that the bulk result differs from the single-leader result, and keep
       the child selected through the next game-hour boundary.
  C03: keep one context and ability stable while observing available, activate it, wait for active,
       then cooldown, then available again; hover the same row in each relevant state and retain
       each witnessed state through the following game-hour boundary.
  C04: select a real leader, field marshal, army, or army group whose ability list is measured
       empty, then keep it selected through the next game-hour boundary.
Do not pause while waiting for an hour boundary: the exact portable root catalog is sampled by
the scheduled compact sweep, and every accepted card must carry its own pointer-free ABL1 join.
Do not restart the game. The raw receipt is diagnostic and cannot promote d88 by itself.
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
    -Recurse -File -Filter 'a36_active_ability_evidence_capture.json' -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTimeUtc -ge $captureStartedUtc } |
    Sort-Object LastWriteTimeUtc -Descending |
    Select-Object -First 1
if (-not $candidate) {
    throw 'The loaded collector did not produce an A36 evidence receipt.'
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
    throw "A36 raw fixture is incomplete or invalid; evidence was preserved and regraded at $verification"
}
Write-Output "A36 four-card raw fixture accepted for promotion review: $verification"
