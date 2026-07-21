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
foreach ($path in @($RecorderExe, $Hoi4Exe, $CollectorDll)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required file does not exist: $path"
    }
}
if (-not (Test-Path -LiteralPath $PdxUserDir -PathType Container)) {
    throw "Required PdxUserDir does not exist: $PdxUserDir"
}
if ($DurationSeconds -lt 600) {
    throw 'DurationSeconds must be at least 600 so the mixed UI fixture can complete.'
}
if ($SampleMilliseconds -lt 25) {
    throw 'SampleMilliseconds must be at least 25.'
}
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
    throw 'A39 evidence requires a clean early launch; close the existing hoi4.exe first.'
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
        throw "OutDir must be absent or empty so stale A39 evidence cannot be selected: $resolvedOut"
    }
} else {
    [System.IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
}

$repo = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$regrader = Join-Path $repo 'extractor\documents\dossiers\A39\verify_a39_character_presentation_live_fixture.py'
if (-not (Test-Path -LiteralPath $regrader -PathType Leaf)) {
    throw "A39 regrader missing: $regrader"
}
$loadedCollector = Join-Path $resolvedOut 'a39-evidence-collector.dll'
Copy-Item -LiteralPath $resolvedCollector -Destination $loadedCollector
$captureManifest = Join-Path $resolvedOut 'native_session_manifest.json'
$receipt = Join-Path $resolvedOut 'a39_character_presentation_evidence_capture.json'
$verification = Join-Path $resolvedOut 'a39_character_presentation_live_fixture_regrade.json'

$arguments = @(
    '--early-launch-exe', $resolvedExe,
    '--pdx-user-dir', $resolvedUserDir,
    '--dll', $loadedCollector,
    '--out-dir', $resolvedOut,
    '--duration-s', $DurationSeconds,
    '--sample-ms', $SampleMilliseconds,
    '--min-game-hours', '0',
    '--stream-format', 'compact',
    '--scheduled-domain', 'fuel_live',
    '--scheduled-domains-exact',
    '--sidecar-log',
    '--skip-host-fold',
    '--a39-character-presentation-evidence'
)
foreach ($value in $EarlyLaunchArgument) {
    $arguments += @('--early-launch-arg', $value)
}

Write-Output @'
A39 installs A25 first, waits for one complete same-launch textbox census and
one complete d47/d46 identity epoch, and only then enables the A39 observers.
After the continued save is fully loaded:
  1. Open a land commander trait tree. Exercise blocked, pickable, and picked
     nodes, including one progress-bearing node, and hover a node long enough
     for its exact reason text to materialize.
  2. Open a naval commander trait tree and repeat the refresh/hover operation.
  3. Open the ace list from a selected wing, then from a plane-type filter.
  4. Materialize at least one ace with a literal
     GFX_<TAG>_ace_<name>_<surname> portrait and one whose engine-selected
     portrait differs from that literal candidate (the fallback-pool route).
  5. Include at least one nonempty ace modifier presentation. If the engine
     produces an empty presentation, leave it empty; it is retained as a
     measured-empty string rather than converted to unavailable.
Revisit the corresponding leader and ace panels after each context change so
one complete trait/ace pair is refreshed. Do not restart the game or substitute
screenshots or human labels for the observed engine transactions.
'@

& $resolvedRecorder @arguments
if ($LASTEXITCODE -ne 0) {
    throw "collector_record exited with code $LASTEXITCODE; retain $resolvedOut for diagnosis."
}
if (-not (Test-Path -LiteralPath $captureManifest -PathType Leaf)) {
    throw "Recorder capture manifest missing: $captureManifest"
}
if (-not (Test-Path -LiteralPath $receipt -PathType Leaf)) {
    throw 'The loaded collector did not produce an A39 evidence receipt.'
}

& $Python $regrader `
    --receipt $receipt `
    --capture-manifest $captureManifest `
    --collector-dll $loadedCollector `
    --out $verification
if ($LASTEXITCODE -ne 0) {
    throw "A39 mixed fixture is incomplete or invalid; evidence was preserved and regraded at $verification"
}
Write-Output "A39 mixed fixture accepted for promotion review: $verification"
