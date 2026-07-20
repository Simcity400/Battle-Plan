[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $RecorderExe,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $PdxUserDir,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [int] $DurationSeconds = 900,
    [int] $SampleMilliseconds = 250,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave'),
    [string] $VisibleActionEvidence,
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'

foreach ($path in @($RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required path does not exist: $path"
    }
}
if ($DurationSeconds -lt 60) {
    throw 'DurationSeconds must be at least 60.'
}
if ($SampleMilliseconds -lt 10) {
    throw 'SampleMilliseconds must be at least 10.'
}
$resolvedHoi4 = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$observedExeSha256 = (Get-FileHash -LiteralPath $resolvedHoi4 -Algorithm SHA256).Hash.ToLowerInvariant()
if ($observedExeSha256 -cne $expectedExeSha256) {
    throw "hoi4.exe SHA-256 is not exact a729 authority: $observedExeSha256"
}
$resolvedOut = [System.IO.Path]::GetFullPath($OutDir)
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or
        @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty: $resolvedOut"
    }
} else {
    [System.IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
}

$arguments = @(
    '--early-launch-exe', $resolvedHoi4,
    '--pdx-user-dir', (Resolve-Path -LiteralPath $PdxUserDir).Path,
    '--dll', (Resolve-Path -LiteralPath $CollectorDll).Path,
    '--out-dir', $resolvedOut,
    '--duration-s', $DurationSeconds,
    '--sample-ms', $SampleMilliseconds,
    '--stream-format', 'compact',
    '--profile', 'debug',
    '--scheduled-domain', 'session_multiplayer_metadata_live',
    '--scheduled-domains-exact',
    '--sidecar-log',
    '--hour-latch'
)
foreach ($value in $EarlyLaunchArgument) {
    $arguments += @('--early-launch-arg', $value)
}

Write-Host 'X12 probe instructions:'
Write-Host '  1. Wait until the loaded game is responsive and at least one D57 hour can establish the baseline.'
Write-Host '  2. Within one engine hour, make two distinct speed changes.'
Write-Host '  3. In that same hour: pause, change speed while paused, then resume.'
Write-Host '  4. Let the game advance to the next engine hour so the hook queue is drained into D57.'
Write-Host 'The probe will remain non-production even if its machine gate passes.'

$prior = [System.Environment]::GetEnvironmentVariable(
    'BATTLE_PLAN_ENABLE_X12_SPEED_PAUSE_HOOK',
    [System.EnvironmentVariableTarget]::Process
)
try {
    # Early-launch creates the suspended game as a child, so this process-level
    # variable is inherited by hoi4.exe and remains visible inside the injected
    # collector. Setting it only around a late attach would not activate X12.
    [System.Environment]::SetEnvironmentVariable(
        'BATTLE_PLAN_ENABLE_X12_SPEED_PAUSE_HOOK',
        '1',
        [System.EnvironmentVariableTarget]::Process
    )
    & (Resolve-Path -LiteralPath $RecorderExe).Path @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "collector_record exited with code $LASTEXITCODE"
    }
} finally {
    [System.Environment]::SetEnvironmentVariable(
        'BATTLE_PLAN_ENABLE_X12_SPEED_PAUSE_HOOK',
        $prior,
        [System.EnvironmentVariableTarget]::Process
    )
}

$verifier = Join-Path $PSScriptRoot 'verify_x12_speed_pause_activation.py'
$report = Join-Path $resolvedOut 'x12_speed_pause_activation.json'
$verifyArguments = @(
    $verifier,
    '--run-dir', $resolvedOut,
    '--hoi4-exe', $resolvedHoi4,
    '--collector-dll', (Resolve-Path -LiteralPath $CollectorDll).Path,
    '--out', $report
)
if (-not [string]::IsNullOrWhiteSpace($VisibleActionEvidence)) {
    if (-not (Test-Path -LiteralPath $VisibleActionEvidence -PathType Leaf)) {
        throw "VisibleActionEvidence does not exist: $VisibleActionEvidence"
    }
    $verifyArguments += @(
        '--visible-action-evidence',
        (Resolve-Path -LiteralPath $VisibleActionEvidence).Path
    )
}
& $Python @verifyArguments
if ($LASTEXITCODE -ne 0) {
    throw "X12 machine regrade rejected the capture; inspect $report"
}

Write-Host "X12 machine candidate retained: $report"
Write-Host 'Production activation remains disabled pending human semantic review of visible action evidence.'
