[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $RecorderExe,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $PdxUserDir,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [string] $ContinueSave,
    [int] $DurationSeconds = 900,
    [int] $SampleMilliseconds = 25,
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
if ($ContinueSave -and -not (Test-Path -LiteralPath $ContinueSave -PathType Leaf)) {
    throw "Continue-save does not exist: $ContinueSave"
}
if ($DurationSeconds -lt 60) {
    throw 'DurationSeconds must be at least 60 seconds.'
}
if ($SampleMilliseconds -lt 10) {
    throw 'SampleMilliseconds must be at least 10 milliseconds.'
}

$resolvedRecorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedPdxUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedOut = [System.IO.Path]::GetFullPath($OutDir)
$observedExeSha256 = (Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($observedExeSha256 -cne $expectedExeSha256) {
    throw "Hoi4Exe is not exact a729 authority: $observedExeSha256"
}
$existing = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
if ($existing.Count -ne 0) {
    throw "A29 requires a clean early launch; close the game PID(s) $(@($existing.Id) -join ',') first."
}
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or
        @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty so stale A29 evidence cannot be selected: $resolvedOut"
    }
} else {
    [System.IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
}

$verifier = Join-Path $PSScriptRoot 'verify_a29_research_ahead_fixture.py'
if (-not (Test-Path -LiteralPath $verifier -PathType Leaf)) {
    throw "A29 verifier is missing: $verifier"
}
$captureManifest = Join-Path $resolvedOut 'native_session_manifest.json'
$verification = Join-Path $resolvedOut 'a29_research_ahead_fixture_verification.json'
$continueGamePath = Join-Path $resolvedPdxUserDir 'continue_game.json'
$continueGamePriorExists = Test-Path -LiteralPath $continueGamePath -PathType Leaf
$continueGamePriorBytes = if ($continueGamePriorExists) {
    [System.IO.File]::ReadAllBytes($continueGamePath)
} else {
    $null
}
$continueGameOverridden = $false
$continueGameReplacement = $null

if ($ContinueSave) {
    $continueIndex = -1
    $gdprIndex = -1
    for ($index = 0; $index -lt $EarlyLaunchArgument.Count; $index++) {
        if ($EarlyLaunchArgument[$index] -ieq '--continuelastsave') {
            if ($continueIndex -ge 0) {
                throw '-EarlyLaunchArgument must contain --continuelastsave exactly once.'
            }
            $continueIndex = $index
        }
        if ($EarlyLaunchArgument[$index] -ieq '--gdpr-compliant') {
            if ($gdprIndex -ge 0) {
                throw '-EarlyLaunchArgument must contain --gdpr-compliant exactly once.'
            }
            $gdprIndex = $index
        }
    }
    if ($continueIndex -lt 0) {
        throw '-ContinueSave requires --continuelastsave in -EarlyLaunchArgument.'
    }
    if ($gdprIndex -lt 0 -or $gdprIndex -lt $continueIndex) {
        throw '-ContinueSave requires launcher-effective --gdpr-compliant after --continuelastsave.'
    }
    $resolvedContinueSave = (Resolve-Path -LiteralPath $ContinueSave).Path
    $saveDirectory = [System.IO.Path]::GetFullPath((Join-Path $resolvedPdxUserDir 'save games'))
    $requestedDirectory = [System.IO.Path]::GetFullPath((Split-Path -Parent $resolvedContinueSave))
    if (-not [string]::Equals(
        $saveDirectory.TrimEnd([char]'\'),
        $requestedDirectory.TrimEnd([char]'\'),
        [System.StringComparison]::OrdinalIgnoreCase
    )) {
        throw "-ContinueSave must be a direct child of $saveDirectory"
    }
    $continueObject = if ($continueGamePriorExists) {
        Get-Content -LiteralPath $continueGamePath -Raw | ConvertFrom-Json
    } else {
        [pscustomobject]@{
            title = [System.IO.Path]::GetFileNameWithoutExtension($resolvedContinueSave)
            desc = ''
            date = [DateTime]::UtcNow.ToString('R')
            filename = ''
            is_remote = $false
        }
    }
    $continueObject.filename = [System.IO.Path]::GetFileName($resolvedContinueSave)
    $continueObject.is_remote = $false
    $continueGameReplacement = ($continueObject | ConvertTo-Json -Depth 8) + [Environment]::NewLine
}

$arguments = @(
    '--early-launch-exe', $resolvedExe,
    '--pdx-user-dir', $resolvedPdxUserDir,
    '--dll', $resolvedCollector,
    '--out-dir', $resolvedOut,
    '--duration-s', $DurationSeconds,
    '--sample-ms', $SampleMilliseconds,
    '--min-game-hours', '12',
    '--stream-format', 'compact',
    '--profile', 'debug',
    '--scheduled-domain', 'tech_live',
    '--scheduled-domains-exact',
    '--sidecar-log',
    '--hour-latch'
)
foreach ($value in $EarlyLaunchArgument) {
    $arguments += @('--early-launch-arg', $value)
}

Write-Host @'
A29 operator action:
  1. Let the pinned save finish loading.
  2. Select a country with an active research slot.
  3. Open that active technology and display its year/ahead-of-time tooltip.
  4. Keep the tooltip visible for several seconds while the game is running
     slowly, so a tech_live poll joins the UI producer in the same game hour.
  5. Leave the game running until the recorder exits; the debug capture gate
     requires at least twelve in-game hours in total.
The verifier will reject a capture that lacks the authenticated producer row.
'@

try {
    if ($null -ne $continueGameReplacement) {
        # Mark the path overridden first so any partial write still restores
        # the exact prior bytes in the finally block.
        $continueGameOverridden = $true
        [System.IO.File]::WriteAllText(
            $continueGamePath,
            $continueGameReplacement,
            [System.Text.UTF8Encoding]::new($false)
        )
    }
    & $resolvedRecorder @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "collector_record exited with code $LASTEXITCODE"
    }
} finally {
    if ($continueGameOverridden) {
        if ($continueGamePriorExists) {
            [System.IO.File]::WriteAllBytes($continueGamePath, $continueGamePriorBytes)
        } elseif (Test-Path -LiteralPath $continueGamePath) {
            Remove-Item -LiteralPath $continueGamePath -Force
        }
    }
}

if (-not (Test-Path -LiteralPath $captureManifest -PathType Leaf)) {
    throw "Recorder capture manifest was not produced: $captureManifest"
}
$manifestObject = Get-Content -LiteralPath $captureManifest -Raw | ConvertFrom-Json
$replayProperty = $manifestObject.PSObject.Properties['replay_file']
$replayName = if ($null -eq $replayProperty) { '' } else { [string] $replayProperty.Value }
if ([string]::IsNullOrWhiteSpace($replayName) -or
    [System.IO.Path]::GetFileName($replayName) -cne $replayName) {
    throw 'Recorder manifest replay_file is absent or is not a direct output filename.'
}
$replay = [System.IO.Path]::GetFullPath((Join-Path $resolvedOut $replayName))
if (-not (Test-Path -LiteralPath $replay -PathType Leaf)) {
    throw "Recorder replay artifact was not produced: $replay"
}

& $Python $verifier `
    --capture-manifest $captureManifest `
    --replay $replay `
    --out $verification
if ($LASTEXITCODE -ne 0) {
    throw "A29 observed-producer fixture was rejected; inspect $verification"
}

Write-Output "A29 observed-producer fixture accepted: $verification"
