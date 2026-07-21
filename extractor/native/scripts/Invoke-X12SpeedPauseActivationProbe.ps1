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
    [switch] $ManualActions,
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
    throw "Required PDX user directory does not exist: $PdxUserDir"
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

$actionWorker = $null
$actionReceipt = Join-Path $resolvedOut 'x12_authenticated_action_attempt.json'
if ($ManualActions) {
    Write-Host 'X12 manual fixture instructions:'
    Write-Host '  1. Within one engine hour, pause the game.'
    Write-Host '  2. While paused, make two distinct speed changes.'
    Write-Host '  3. Resume and let the game advance to the next engine hour.'
    Write-Host 'The exact in-process mutation history is evidence; operator intent is not.'
} else {
    $actionScript = Join-Path $PSScriptRoot 'Invoke-X12AuthenticatedActionSequence.ps1'
    if (-not (Test-Path -LiteralPath $actionScript -PathType Leaf)) {
        throw "X12 authenticated action driver is missing: $actionScript"
    }
    $existingExact = @(
        Get-Process -Name the game -ErrorAction SilentlyContinue |
            Where-Object {
                try {
                    [IO.Path]::GetFullPath($_.MainModule.FileName) -ieq
                        [IO.Path]::GetFullPath($resolvedHoi4)
                } catch {
                    $false
                }
            }
    )
    if ($existingExact.Count -ne 0) {
        throw 'Automated X12 early-launch requires no pre-existing exact-a729 the game process.'
    }
    $notBefore = [DateTime]::UtcNow.ToFileTimeUtc()
    $actionStdout = Join-Path $resolvedOut 'x12_action_driver.stdout.log'
    $actionStderr = Join-Path $resolvedOut 'x12_action_driver.stderr.log'
    $quotedActionScript = '"' + $actionScript.Replace('"', '\"') + '"'
    $quotedHoi4 = '"' + $resolvedHoi4.Replace('"', '\"') + '"'
    $quotedReceipt = '"' + $actionReceipt.Replace('"', '\"') + '"'
    $actionArguments = @(
        '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
        '-File', $quotedActionScript,
        '-Hoi4Exe', $quotedHoi4,
        '-ReceiptPath', $quotedReceipt,
        '-NotBeforeFileTimeUtc', $notBefore,
        '-TimeoutSeconds', ([Math]::Min(3600, [Math]::Max(60, $DurationSeconds - 5)))
    )
    $actionWorker = Start-Process -FilePath 'powershell.exe' `
        -ArgumentList $actionArguments `
        -PassThru `
        -WindowStyle Hidden `
        -RedirectStandardOutput $actionStdout `
        -RedirectStandardError $actionStderr
}

$prior = [System.Environment]::GetEnvironmentVariable(
    'BATTLE_PLAN_ENABLE_X12_SPEED_PAUSE_HOOK',
    [System.EnvironmentVariableTarget]::Process
)
$recorderFailure = $null
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
} catch {
    $recorderFailure = $_
} finally {
    [System.Environment]::SetEnvironmentVariable(
        'BATTLE_PLAN_ENABLE_X12_SPEED_PAUSE_HOOK',
        $prior,
        [System.EnvironmentVariableTarget]::Process
    )
}
if ($null -ne $recorderFailure) {
    if ($null -ne $actionWorker -and -not $actionWorker.HasExited) {
        Stop-Process -Id $actionWorker.Id -Force -ErrorAction SilentlyContinue
    }
    throw $recorderFailure
}

if ($null -ne $actionWorker) {
    if (-not $actionWorker.WaitForExit(30000)) {
        Stop-Process -Id $actionWorker.Id -Force -ErrorAction SilentlyContinue
        throw 'X12 action driver did not finish after the recorder returned.'
    }
    if ($actionWorker.ExitCode -ne 0 -or
        -not (Test-Path -LiteralPath $actionReceipt -PathType Leaf)) {
        throw "X12 authenticated action driver failed; inspect $resolvedOut"
    }
    $VisibleActionEvidence = $actionReceipt
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
Write-Host 'The machine receipt is sufficient for X12 promotion; any action-attempt receipt is diagnostic only.'
