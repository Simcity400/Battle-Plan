[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $RecorderExe,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $PdxUserDir,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $CheckpointSavePath,
    [Parameter(Mandatory = $true)] [ValidatePattern('^[A-Z0-9]{3}$')] [string] $OwnerTag,
    [Parameter(Mandatory = $true)] [string] $TimedModifier,
    [Parameter(Mandatory = $true)] [string] $PermanentModifier,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [int] $DurationSeconds = 3600,
    [int] $SampleMilliseconds = 100,
    [int] $SaveStableMilliseconds = 1500,
    [string[]] $EarlyLaunchArgument = @('--continuelastsave'),
    [string] $Rakaly,
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

foreach ($path in @($RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll, $CheckpointSavePath)) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required path does not exist: $path"
    }
}
if ([string]::IsNullOrWhiteSpace($TimedModifier) -or
    [string]::IsNullOrWhiteSpace($PermanentModifier) -or
    $TimedModifier -ceq $PermanentModifier) {
    throw 'TimedModifier and PermanentModifier must be distinct nonempty ids.'
}
if (-not [string]::IsNullOrWhiteSpace($Rakaly) -and -not (Test-Path -LiteralPath $Rakaly -PathType Leaf)) {
    throw "Rakaly does not exist: $Rakaly"
}
if ($DurationSeconds -lt 60 -or $SampleMilliseconds -lt 25 -or $SaveStableMilliseconds -lt 250) {
    throw 'DurationSeconds must be >=60, SampleMilliseconds >=25, and SaveStableMilliseconds >=250.'
}

$expectedExeSha = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$resolvedRecorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedSave = (Resolve-Path -LiteralPath $CheckpointSavePath).Path
$resolvedOut = [System.IO.Path]::GetFullPath($OutDir)
[System.IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha) {
    throw 'Hoi4Exe does not match the pinned a729 SHA-256.'
}
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
    throw 'A18 owner capture requires a clean early launch; close the existing hoi4.exe first.'
}

$laneRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\lanes\country_dynamic_modifiers_live'))
$checkpointTool = Join-Path $laneRoot 'a18_capture_checkpoint.py'
$verifier = Join-Path $PSScriptRoot 'verify_a18_dynamic_modifier_fixture.py'
foreach ($tool in @($checkpointTool, $verifier)) {
    if (-not (Test-Path -LiteralPath $tool -PathType Leaf)) { throw "A18 tool missing: $tool" }
}
$captureManifest = Join-Path $resolvedOut 'native_session_manifest.json'
$hookReceipt = Join-Path $resolvedOut 'a18_dynamic_modifier_lifecycle_witness.json'
$verification = Join-Path $resolvedOut 'a18_dynamic_modifier_fixture_verification.json'
$stdout = Join-Path $resolvedOut 'collector_record.stdout.log'
$stderr = Join-Path $resolvedOut 'collector_record.stderr.log'
$phases = @('positive_before', 'positive_after', 'post_removal')
$checkpointPaths = @{}
$captureStartedUtc = [DateTime]::UtcNow
$ownedOutputs = @($captureManifest, $hookReceipt, $verification, $stdout, $stderr)
foreach ($phase in $phases) {
    $ownedOutputs += (Join-Path $resolvedOut ("a18_checkpoint_{0}.json" -f $phase))
}
foreach ($ownedOutput in $ownedOutputs) {
    if (Test-Path -LiteralPath $ownedOutput -PathType Leaf) {
        Remove-Item -LiteralPath $ownedOutput -Force
    }
}

$arguments = @(
    '--early-launch-exe', $resolvedExe,
    '--pdx-user-dir', $resolvedUserDir,
    '--dll', $resolvedCollector,
    '--out-dir', $resolvedOut,
    '--duration-s', $DurationSeconds,
    '--sample-ms', $SampleMilliseconds,
    '--stream-format', 'compact',
    '--profile', 'debug',
    '--scheduled-domain', 'country_dynamic_modifiers_live',
    '--scheduled-domains-exact',
    '--sidecar-log',
    '--hour-latch'
)
foreach ($value in $EarlyLaunchArgument) { $arguments += @('--early-launch-arg', $value) }

$startInfo = [System.Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = $resolvedRecorder
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$startInfo.RedirectStandardOutput = $true
$startInfo.RedirectStandardError = $true
$startInfo.Environment['BATTLE_PLAN_ENABLE_A18_DYNMOD_EVIDENCE'] = '1'
foreach ($value in $arguments) { [void]$startInfo.ArgumentList.Add([string]$value) }
$recorder = [System.Diagnostics.Process]::new()
$recorder.StartInfo = $startInfo
if (-not $recorder.Start()) { throw 'collector_record did not start.' }
$stdoutTask = $recorder.StandardOutput.ReadToEndAsync()
$stderrTask = $recorder.StandardError.ReadToEndAsync()

try {
    $launchDeadline = [DateTime]::UtcNow.AddMinutes(5)
    do {
        Start-Sleep -Milliseconds 250
        $game = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
        if ($recorder.HasExited) { throw "collector_record exited during early launch with code $($recorder.ExitCode)" }
    } while ($game.Count -eq 0 -and [DateTime]::UtcNow -lt $launchDeadline)
    if ($game.Count -ne 1) { throw 'Early launch did not produce exactly one hoi4.exe process.' }

    $phaseIndex = 0
    $lastAcceptedHash = $null
    $candidateHash = $null
    $candidateSince = [DateTime]::MinValue
    while (-not $recorder.HasExited -and $phaseIndex -lt $phases.Count) {
        try {
            $hash = (Get-FileHash -LiteralPath $resolvedSave -Algorithm SHA256).Hash.ToLowerInvariant()
        } catch {
            # The game's save replacement can briefly remove or exclusively
            # hold the path. That is not a checkpoint failure; wait for a new
            # stable content hash.
            $candidateHash = $null
            $candidateSince = [DateTime]::UtcNow
            Start-Sleep -Milliseconds $SampleMilliseconds
            continue
        }
        if ($hash -cne $candidateHash) {
            $candidateHash = $hash
            $candidateSince = [DateTime]::UtcNow
        }
        $stableFor = ([DateTime]::UtcNow - $candidateSince).TotalMilliseconds
        if ($hash -cne $lastAcceptedHash -and $stableFor -ge $SaveStableMilliseconds) {
            $phase = $phases[$phaseIndex]
            $checkpoint = Join-Path $resolvedOut ("a18_checkpoint_{0}.json" -f $phase)
            $checkpointArgs = @(
                $checkpointTool,
                '--save', $resolvedSave,
                '--out', $checkpoint,
                '--owner-tag', $OwnerTag,
                '--timed-modifier', $TimedModifier,
                '--permanent-modifier', $PermanentModifier,
                '--phase', $phase
            )
            if (-not [string]::IsNullOrWhiteSpace($Rakaly)) { $checkpointArgs += @('--rakaly', $Rakaly) }
            & $Python @checkpointArgs
            if ($LASTEXITCODE -eq 0 -and (Test-Path -LiteralPath $checkpoint -PathType Leaf)) {
                $checkpointPaths[$phase] = $checkpoint
                $lastAcceptedHash = $hash
                $phaseIndex++
                Write-Output "A18 accepted $phase checkpoint. Pause and save at the next requested boundary."
            } else {
                # A stable save can precede the requested lifecycle boundary.
                # Re-arm stability so the expensive save/live join is bounded
                # instead of being retried every SampleMilliseconds.
                $candidateSince = [DateTime]::UtcNow
            }
        }
        Start-Sleep -Milliseconds $SampleMilliseconds
    }
    $recorder.WaitForExit()
    [System.IO.File]::WriteAllText($stdout, $stdoutTask.Result, [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText($stderr, $stderrTask.Result, [System.Text.UTF8Encoding]::new($false))
    if ($recorder.ExitCode -ne 0) { throw "collector_record exited with code $($recorder.ExitCode); inspect $stderr" }
} finally {
    if (-not $recorder.HasExited) {
        # Do not kill the game or tear down an injected collector unsafely. The
        # recorder owns its duration and normal unload/finalization path.
        $recorder.WaitForExit()
        [System.IO.File]::WriteAllText($stdout, $stdoutTask.Result, [System.Text.UTF8Encoding]::new($false))
        [System.IO.File]::WriteAllText($stderr, $stderrTask.Result, [System.Text.UTF8Encoding]::new($false))
    }
    $recorder.Dispose()
}

if ($checkpointPaths.Count -ne 3) {
    throw "A18 capture ended with $($checkpointPaths.Count)/3 checkpoints; retain $resolvedOut and rerun from a suitable state."
}
if (-not (Test-Path -LiteralPath $captureManifest -PathType Leaf)) {
    throw "Recorder manifest missing: $captureManifest"
}
if (-not (Test-Path -LiteralPath $hookReceipt -PathType Leaf)) {
    $manifestObject = Get-Content -LiteralPath $captureManifest -Raw | ConvertFrom-Json
    $collectorDir = Split-Path -Parent $resolvedCollector
    $candidateRoots = @($resolvedOut, $collectorDir)
    if ($manifestObject.dll) {
        $candidateRoots += Split-Path -Parent ([string] $manifestObject.dll)
    }
    $candidateRoots = @($candidateRoots | Select-Object -Unique)
    $candidate = Get-ChildItem -LiteralPath $candidateRoots -Recurse -File -Filter 'a18_dynamic_modifier_lifecycle_witness.json' -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTimeUtc -ge $captureStartedUtc } |
        Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
    if ($candidate) { Copy-Item -LiteralPath $candidate.FullName -Destination $hookReceipt -Force }
}
if (-not (Test-Path -LiteralPath $hookReceipt -PathType Leaf)) {
    throw 'A18 hook receipt was not produced by the loaded collector.'
}

$verifyArgs = @($verifier)
foreach ($phase in $phases) { $verifyArgs += @('--checkpoint', $checkpointPaths[$phase]) }
$verifyArgs += @(
    '--hook-receipt', $hookReceipt,
    '--capture-manifest', $captureManifest,
    '--out', $verification
)
& $Python @verifyArgs
if ($LASTEXITCODE -ne 0) { throw "A18 lifecycle fixture rejected; inspect $verification" }
Write-Output "A18 lifecycle fixture accepted: $verification"
