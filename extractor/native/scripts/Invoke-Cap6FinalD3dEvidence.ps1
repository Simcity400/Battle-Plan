[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$RecorderExe,
    [Parameter(Mandatory = $true)][string]$Hoi4Exe,
    [Parameter(Mandatory = $true)][string]$PdxUserDir,
    [Parameter(Mandatory = $true)][string]$CollectorDll,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [Parameter(Mandatory = $true)][string]$StaticBundleManifest,
    [Parameter(Mandatory = $true)][string]$StaticBundleBindingsDir,
    [int]$DurationSeconds = 60,
    [int]$SampleMilliseconds = 100,
    [int]$GameLoadTimeoutSeconds = 240,
    [string]$Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$pulse = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\capture_harness\authenticated_one_hour_pulse.ps1'
$verifier = Join-Path $repo 'replay_interface\re_evidence\rendering_laws\CAP6\verify_live_final_d3d_receipt.py'

function ConvertTo-NativeArgumentLine {
    param([Parameter(Mandatory = $true)][object[]]$Arguments)
    return (($Arguments | ForEach-Object {
        $value = [string]$_
        if ($value -match '[\s"]') {
            '"' + ($value -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
        } else {
            $value
        }
    }) -join ' ')
}

if ($DurationSeconds -lt 30 -or $DurationSeconds -gt 1800) {
    throw 'DurationSeconds must be in 30..1800.'
}
if ($SampleMilliseconds -lt 1 -or $SampleMilliseconds -gt 1000) {
    throw 'SampleMilliseconds must be in 1..1000.'
}
if ($GameLoadTimeoutSeconds -lt 30 -or $GameLoadTimeoutSeconds -gt 900) {
    throw 'GameLoadTimeoutSeconds must be in 30..900.'
}
foreach ($path in @($RecorderExe, $Hoi4Exe, $PdxUserDir, $CollectorDll, $StaticBundleManifest, $StaticBundleBindingsDir, $pulse, $verifier)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required path is absent: $path" }
}
if (@(Get-Process -Name the game -ErrorAction SilentlyContinue).Count -ne 0) {
    throw 'CAP6 clean early launch requires no existing hoi4.exe process; it never closes or replaces a protected session.'
}

$resolvedRecorder = (Resolve-Path -LiteralPath $RecorderExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedUserDir = (Resolve-Path -LiteralPath $PdxUserDir).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedStaticBundleManifest = (Resolve-Path -LiteralPath $StaticBundleManifest).Path
$resolvedStaticBundleBindingsDir = (Resolve-Path -LiteralPath $StaticBundleBindingsDir).Path
$resolvedOut = [IO.Path]::GetFullPath($OutDir)
$sessionId = [guid]::NewGuid().ToString()
$captureId = [guid]::NewGuid().ToString()
$exeSha256 = (Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant()
if ($exeSha256 -cne $expectedExeSha256) { throw "hoi4.exe exact-a729 SHA-256 mismatch: $exeSha256" }
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or
        @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty: $resolvedOut"
    }
} else {
    [IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
}

$arguments = @(
    '--early-launch-exe', $resolvedExe,
    '--early-launch-arg', '--continuelastsave',
    '--early-launch-arg', '--gdpr-compliant',
    '--pdx-user-dir', $resolvedUserDir,
    '--dll', $resolvedCollector,
    '--out-dir', $resolvedOut,
    '--session-id', $sessionId,
    '--capture-id', $captureId,
    '--duration-s', [string]$DurationSeconds,
    '--sample-ms', [string]$SampleMilliseconds,
    '--min-game-hours', '1',
    '--stream-format', 'compact',
    '--profile', 'debug',
    '--scheduled-domain', 'session_header_live',
    '--scheduled-domains-exact',
    '--static-bundle-manifest', $resolvedStaticBundleManifest,
    '--static-bundle-mod-state', (Join-Path $resolvedStaticBundleBindingsDir 'mod_state.json'),
    '--static-bundle-dlc-state', (Join-Path $resolvedStaticBundleBindingsDir 'dlc_state.json'),
    '--static-bundle-load-order', (Join-Path $resolvedStaticBundleBindingsDir 'load_order.json'),
    '--static-bundle-source-inventory', (Join-Path $resolvedStaticBundleBindingsDir 'source_inventory.json'),
    '--static-bundle-live-resolution-report', (Join-Path $resolvedStaticBundleBindingsDir 'resolution_report.json'),
    '--static-bundle-timeout-ms', '1800000',
    '--sidecar-log'
)

$start = [Diagnostics.ProcessStartInfo]::new()
$start.FileName = $resolvedRecorder
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.EnvironmentVariables['BATTLE_PLAN_A25_GUI_BRIDGE_EVIDENCE_OUT_DIR'] = $resolvedOut
if ($null -ne $start.PSObject.Properties['ArgumentList']) {
    foreach ($argument in $arguments) { [void]$start.ArgumentList.Add([string]$argument) }
} else {
    $start.Arguments = ConvertTo-NativeArgumentLine $arguments
}

$recorder = [Diagnostics.Process]::new()
$recorder.StartInfo = $start
$stdoutTask = $null
$stderrTask = $null
$game = $null
$pulseAccepted = $false
$attemptErrors = [Collections.Generic.List[string]]::new()
try {
    if (-not $recorder.Start()) { throw 'collector_record failed to start.' }
    $stdoutTask = $recorder.StandardOutput.ReadToEndAsync()
    $stderrTask = $recorder.StandardError.ReadToEndAsync()

    # Static-bundle admission happens while the recorder-owned game process is
    # suspended and can legitimately exceed the later UI/load budget. Discover
    # that process through its kernel-reported executable path, then start the
    # load/pulse deadline only after the exact process is identified.
    $processDiscoveryDeadline = [DateTime]::UtcNow.AddMinutes(30)
    while ([DateTime]::UtcNow -lt $processDiscoveryDeadline -and -not $recorder.HasExited) {
        $candidates = @(Get-Process -Name the game -ErrorAction SilentlyContinue)
        if ($candidates.Count -eq 1) {
            $candidatePath = $null
            try {
                $candidatePath = [string](Get-CimInstance Win32_Process -Filter "ProcessId=$($candidates[0].Id)" -ErrorAction Stop).ExecutablePath
            } catch {
                $candidatePath = $null
            }
            if ($candidates[0].MainWindowHandle -ne [IntPtr]::Zero -and
                -not [string]::IsNullOrWhiteSpace($candidatePath) -and
                [string]::Equals([IO.Path]::GetFullPath($candidatePath), $resolvedExe, [StringComparison]::OrdinalIgnoreCase)) {
                $game = $candidates[0]
                break
            }
        } elseif ($candidates.Count -gt 1) {
            throw 'Clean launch produced more than one hoi4.exe process.'
        }
        Start-Sleep -Milliseconds 250
    }
    if ($null -eq $game) {
        if ($recorder.HasExited) { throw "collector_record exited before exact-a729 the game appeared (code $($recorder.ExitCode))." }
        throw 'Exact-a729 the game did not appear before the suspended-process discovery timeout.'
    }
    $deadline = [DateTime]::UtcNow.AddSeconds($GameLoadTimeoutSeconds)

    # The pulse helper is fail-closed before its single reversible write. Retry
    # while the continued save initializes; success proves one hour advanced and
    # that the game was restored to paused without an operator action.
    $pulseReceipt = Join-Path $resolvedOut 'cap6_authenticated_one_hour_pulse.json'
    while ([DateTime]::UtcNow -lt $deadline -and -not $recorder.HasExited) {
        $pulseStdout = Join-Path $resolvedOut 'cap6_pulse_attempt.stdout.tmp.log'
        $pulseStderr = Join-Path $resolvedOut 'cap6_pulse_attempt.stderr.tmp.log'
        $pulseProcess = Start-Process -FilePath 'powershell.exe' -ArgumentList @(
            '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $pulse,
            '-ProcessId', [string]$game.Id,
            '-ReceiptPath', $pulseReceipt,
            '-TimeoutMilliseconds', '30000',
            '-PollMilliseconds', '1'
        ) -PassThru -Wait -WindowStyle Hidden -RedirectStandardOutput $pulseStdout -RedirectStandardError $pulseStderr
        if ($pulseProcess.ExitCode -eq 0 -and (Test-Path -LiteralPath $pulseReceipt -PathType Leaf)) {
            $pulseAccepted = $true
            Remove-Item -LiteralPath $pulseStdout, $pulseStderr -Force -ErrorAction SilentlyContinue
            break
        }
        $message = if (Test-Path -LiteralPath $pulseStderr -PathType Leaf) {
            (Get-Content -LiteralPath $pulseStderr -Raw).Trim()
        } else {
            "pulse attempt exited $($pulseProcess.ExitCode)"
        }
        if (-not [string]::IsNullOrWhiteSpace($message)) { $attemptErrors.Add($message) }
        Remove-Item -LiteralPath $pulseStdout, $pulseStderr -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2
    }
    if (-not $pulseAccepted) {
        $tail = @($attemptErrors | Select-Object -Last 3) -join ' | '
        throw "CAP6 clean launch never reached an authenticated paused one-hour pulse state. $tail"
    }

    $remainingMs = [Math]::Max(1000, ($DurationSeconds + 90) * 1000)
    if (-not $recorder.WaitForExit($remainingMs)) {
        throw 'collector_record exceeded the bounded CAP6 capture/finalization deadline.'
    }
    if ($recorder.ExitCode -ne 0) { throw "collector_record exited with code $($recorder.ExitCode)." }
} finally {
    if ($null -ne $stdoutTask) {
        [IO.File]::WriteAllText((Join-Path $resolvedOut 'collector_record.stdout.log'), [string]$stdoutTask.Result, [Text.UTF8Encoding]::new($false))
    }
    if ($null -ne $stderrTask) {
        [IO.File]::WriteAllText((Join-Path $resolvedOut 'collector_record.stderr.log'), [string]$stderrTask.Result, [Text.UTF8Encoding]::new($false))
    }
    $recorder.Dispose()
}

$manifest = Join-Path $resolvedOut 'native_session_manifest.json'
if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) { throw 'CAP6 run has no native session manifest.' }
$verification = Join-Path $resolvedOut 'cap6_final_d3d_verification.json'
& $Python $verifier `
    --capture-root $resolvedOut `
    --collector-dll $resolvedCollector `
    --session-manifest $manifest `
    --out $verification
if ($LASTEXITCODE -ne 0) { throw "CAP6 final-D3D verification failed; inspect $resolvedOut" }
Write-Output "CAP6 final-D3D receipt mechanically admitted: $verification"
