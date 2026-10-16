param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$Recording,
    [Parameter(Mandatory = $true)][string]$Evidence,
    [Parameter(Mandatory = $true)][ValidateSet('serial', 'parallel')][string]$Mode,
    [string]$GameDir = $(if (Test-Path Env:BATTLE_PLAN_GAME_DIR) { $env:BATTLE_PLAN_GAME_DIR } else { $env:HOI4_GAME_DIR }),
    [string]$UserDir = $(if (Test-Path Env:BATTLE_PLAN_USER_DIR) { $env:BATTLE_PLAN_USER_DIR } else { $env:HOI4_USER_DIR }),
    [ValidateRange(1, 2147483647)][int]$MeasureSeconds = 120,
    [ValidateRange(1, 2147483647)][int]$WallSeconds = 300,
    [ValidateRange(1, 2147483647)][int]$SilenceSeconds = 120,
    [ValidateRange(1, 9223372036854775807)][long]$MaxPrivateBytes = 16GB,
    [ValidateRange(1, 9223372036854775807)][long[]]$ThroughHours = @(),
    [switch]$Resume,
    [ValidateRange(0, 9223372036854775807)][long]$ProcessorAffinity = 0
)
$ErrorActionPreference = 'Stop'
if (-not $GameDir -or -not $UserDir) { throw 'Supply the real game and user folders for all production observers.' }
if (Test-Path -LiteralPath $Evidence) {
    if (-not $Resume -and (Get-ChildItem -LiteralPath $Evidence | Select-Object -First 1)) {
        throw 'Evidence must be empty: serial and overlapped runs need separate fresh caches.'
    }
}
if ($Resume -and -not (Test-Path -LiteralPath (Join-Path $Evidence 'measurement-input.json'))) {
    throw 'Resume requires the existing measurement evidence and private cache roots.'
}
$workspace = Split-Path -Parent $PSScriptRoot
$executablePath = (Resolve-Path -LiteralPath $Executable).Path
$recordingPath = (Resolve-Path -LiteralPath $Recording).Path
$gamePath = (Resolve-Path -LiteralPath $GameDir).Path
$userPath = (Resolve-Path -LiteralPath $UserDir).Path
$evidencePath = [IO.Path]::GetFullPath($Evidence)
if (Test-Path -LiteralPath (Join-Path $workspace '.git')) {
    $workspacePath = [IO.Path]::GetFullPath($workspace).TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    if ($evidencePath.StartsWith($workspacePath, [StringComparison]::OrdinalIgnoreCase) -or $evidencePath -eq $workspacePath.TrimEnd('\', '/')) {
        throw 'Evidence must stay outside the Git worktree.'
    }
}
$null = New-Item -ItemType Directory -Path $evidencePath -Force
$runStamp = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss-fff')
$outputPath = Join-Path $evidencePath $(if ($Resume) { "stdout-$runStamp.log" } else { 'stdout.log' })
$errorPath = Join-Path $evidencePath $(if ($Resume) { "stderr-$runStamp.log" } else { 'stderr.log' })
$journalPath = Join-Path $evidencePath 'blocks.jsonl'
$priorProcess = $null
if ($Resume -and (Test-Path -LiteralPath (Join-Path $evidencePath 'process-measurement.json'))) {
    $resumeMetadataPath = Join-Path $evidencePath 'process-measurement-metadata.json'
    if (-not (Test-Path -LiteralPath $resumeMetadataPath)) { $resumeMetadataPath = Join-Path $evidencePath 'process-measurement.json' }
    $priorProcess = Get-Content -LiteralPath $resumeMetadataPath -Raw | ConvertFrom-Json
    Copy-Item -LiteralPath (Join-Path $evidencePath 'process-measurement.json') -Destination (Join-Path $evidencePath "process-$runStamp.previous.json")
}
$cachePaths = @{}
foreach ($pair in @(@('LOCALAPPDATA', 'local'), @('APPDATA', 'roaming'), @('TEMP', 'temp'), @('TMP', 'temp'))) {
    $cachePaths[$pair[0]] = Join-Path $evidencePath ('cache/' + $pair[1])
    $null = New-Item -ItemType Directory -Path $cachePaths[$pair[0]] -Force
}
if (-not ('PrepMeasurementExit' -as [type])) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class PrepMeasurementExit {
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool GetExitCodeProcess(IntPtr process, out uint code);
}
'@
}
function ConvertTo-ProcessArgument([string]$Value) {
    '"' + [regex]::Replace([regex]::Replace($Value, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1') + '"'
}
$arguments = @('--recording', $recordingPath, '--evidence', $evidencePath, '--game-dir', $gamePath, '--user-dir', $userPath,
    '--preparation-measure', $Mode, '--measure-seconds', $MeasureSeconds.ToString())
if ($ThroughHours.Count -gt 0) { $arguments += @('--through-hours', (($ThroughHours | Sort-Object -Unique) -join ',')) }
if ($Resume) { $arguments += '--resume' }
$argumentText = ($arguments | ForEach-Object { ConvertTo-ProcessArgument $_ }) -join ' '
$machine = Get-CimInstance Win32_ComputerSystem -OperationTimeoutSec 5
$oldEnvironment = @{}
$taskProcess = $null
try {
    foreach ($name in $cachePaths.Keys) {
        $oldEnvironment[$name] = [Environment]::GetEnvironmentVariable($name)
        [Environment]::SetEnvironmentVariable($name, $cachePaths[$name])
    }
    $taskProcess = Start-Process -FilePath $executablePath -WorkingDirectory $workspace -ArgumentList $argumentText -PassThru -WindowStyle Hidden -RedirectStandardOutput $outputPath -RedirectStandardError $errorPath
    # Capture before exit: the PowerShell Process.ExitCode property can be empty.
    $processHandle = $taskProcess.Handle
    if ($ProcessorAffinity -ne 0) {
        try { $taskProcess.ProcessorAffinity = [IntPtr]$ProcessorAffinity }
        catch { Stop-Process -Id $taskProcess.Id -Force -ErrorAction SilentlyContinue; throw }
    }
} catch {
    if ($null -ne $taskProcess) { Stop-Process -Id $taskProcess.Id -Force -ErrorAction SilentlyContinue }
    throw
} finally {
    foreach ($name in $oldEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $oldEnvironment[$name])
    }
}
$sampleWriter = $null
try {
    $samplePath = Join-Path $evidencePath 'memory-samples.csv'
    $sampleWriter = [IO.StreamWriter]::new($samplePath, [bool]$Resume)
    $sampleWriter.AutoFlush = $true
    if (-not $Resume -or (Get-Item -LiteralPath $samplePath).Length -eq 0) {
        $sampleWriter.WriteLine('run,seconds,phase,privateBytes,workingSetBytes,machineCpuPercent,processCpuSeconds')
    }
    $phaseSamples = @{}
    $cpuSum = 0.0
    $cpuCount = 0L
    $started = [Diagnostics.Stopwatch]::StartNew()
    $lastOutputTime = 0.0
    $lastOutputLength = 0L
    $lastLoadTime = -2.0
    $machineCpu = $null
    $cpuSampleFailures = 0
    $lastCpuSampleError = $null
    $phase = 'startup'
    $stopReason = $null
    $peakWorkingSet = 0L
    $peakPrivate = 0L
    $cumulativePeakPrivate = if ($null -ne $priorProcess) { [long]$priorProcess.peakSampledPrivateBytes } else { 0L }
    if ($Resume -and (Test-Path -LiteralPath (Join-Path $evidencePath 'memory-current.json'))) {
        try {
            $priorMemory = Get-Content -LiteralPath (Join-Path $evidencePath 'memory-current.json') -Raw | ConvertFrom-Json
            $cumulativePeakPrivate = [Math]::Max($cumulativePeakPrivate, [long]$priorMemory.peakPrivateBytes)
        } catch { } # A torn last sample does not discard the durable block journal.
    }
    $lastCommitTime = 0.0
    $lastCommitOrdinal = -1L
    $blockTimes = [Collections.Generic.Queue[double]]::new()
    if ($Resume -and (Test-Path -LiteralPath $journalPath)) {
        foreach ($line in @(Get-Content -LiteralPath $journalPath -Tail 64)) {
            $row = $null
            try { $row = $line | ConvertFrom-Json } catch { continue }
            $lastCommitOrdinal = [Math]::Max($lastCommitOrdinal, [long]$row.ordinal)
            if ($row.recordKind -eq 'storage') { continue }
            $blockTimes.Enqueue([double]$row.wallSeconds)
            while ($blockTimes.Count -gt 31) { $null = $blockTimes.Dequeue() }
        }
    }
    $lastJournalCheck = -1.0
    while (-not $taskProcess.HasExited) {
        $taskProcess.Refresh()
        if ($taskProcess.HasExited) { break }
        $outputLength = 0L
        foreach ($logPath in @($outputPath, $errorPath)) {
            if (Test-Path -LiteralPath $logPath) { $outputLength += (Get-Item -LiteralPath $logPath).Length }
        }
        if ($outputLength -gt $lastOutputLength) {
            $lastOutputTime = $started.Elapsed.TotalSeconds
            $lastOutputLength = $outputLength
        }
        if ($started.Elapsed.TotalSeconds - $lastLoadTime -ge 1) {
            try {
                $machineCpu = (Get-CimInstance Win32_PerfFormattedData_PerfOS_Processor -Filter "Name='_Total'" -OperationTimeoutSec 5).PercentProcessorTime
            } catch {
                # A transient WMI timeout must not terminate the real preparation.
                # Keep the last successful CPU sample and retain the failure evidence.
                $cpuSampleFailures += 1
                $lastCpuSampleError = $_.Exception.Message
            }
            $lastLoadTime = $started.Elapsed.TotalSeconds
        }
        if ($started.Elapsed.TotalSeconds - $lastJournalCheck -ge 1) {
            if (Test-Path -LiteralPath $journalPath) {
                foreach ($line in @(Get-Content -LiteralPath $journalPath -Tail 64)) {
                    $row = $null
                    try { $row = $line | ConvertFrom-Json } catch { continue }
                    if ([long]$row.ordinal -gt $lastCommitOrdinal) {
                        $lastCommitOrdinal = [long]$row.ordinal
                        $lastCommitTime = $started.Elapsed.TotalSeconds
                        $blockTimes.Enqueue([double]$row.wallSeconds)
                        while ($blockTimes.Count -gt 31) { $null = $blockTimes.Dequeue() }
                    }
                }
            }
            $lastJournalCheck = $started.Elapsed.TotalSeconds
        }
        # Read bounded log metadata. FRAME lines also identify preparation after
        # an earlier phase marker has scrolled out of this tail.
        foreach ($line in @(Get-Content -LiteralPath $outputPath -Tail 32)) {
            if ($line -match '^PHASE phase=(\S+)') { $phase = $Matches[1] }
            elseif ($line -match '^(FRAME|PROGRESS)\s') { $phase = 'preparation' }
            elseif ($line -match '^RESULT\s') { $phase = 'complete' }
        }
        try {
            $private = $taskProcess.PrivateMemorySize64
            $working = $taskProcess.WorkingSet64
            $cpu = $taskProcess.TotalProcessorTime.TotalSeconds
            $peakWorkingSet = [Math]::Max($peakWorkingSet, $taskProcess.PeakWorkingSet64)
        } catch {
            if ($taskProcess.HasExited) { break }
            throw
        }
        $peakPrivate = [Math]::Max($peakPrivate, $private)
        $cumulativePeakPrivate = [Math]::Max($cumulativePeakPrivate, $private)
        if (-not $phaseSamples.ContainsKey($phase)) {
            $phaseSamples[$phase] = @{ private = 0L; working = 0L; cpuSum = 0.0; cpuCount = 0L }
        }
        $phaseSample = $phaseSamples[$phase]
        $phaseSample.private = [Math]::Max($phaseSample.private, $private)
        $phaseSample.working = [Math]::Max($phaseSample.working, $working)
        if ($null -ne $machineCpu) {
            $cpuSum += $machineCpu; $cpuCount += 1
            $phaseSample.cpuSum += $machineCpu; $phaseSample.cpuCount += 1
        }
        $sampleWriter.WriteLine(('{0},{1},{2},{3},{4},{5},{6}' -f $runStamp,
            $started.Elapsed.TotalSeconds.ToString('R', [Globalization.CultureInfo]::InvariantCulture), $phase, $private, $working,
            $machineCpu, $cpu.ToString('R', [Globalization.CultureInfo]::InvariantCulture)))
        [IO.File]::WriteAllText((Join-Path $evidencePath 'memory-current.json'),
            (@{ privateBytes = $private; peakPrivateBytes = $cumulativePeakPrivate } | ConvertTo-Json -Compress))
        $blockLimit = [double]$WallSeconds
        $silenceLimit = [double]$SilenceSeconds
        if ($ThroughHours.Count -gt 0 -and $blockTimes.Count -gt 0) {
            $orderedTimes = @($blockTimes.ToArray() | Sort-Object)
            $median = $orderedTimes[[int][Math]::Floor($orderedTimes.Count / 2)]
            $blockLimit = [Math]::Max($WallSeconds, 10 * $median)
            $silenceLimit = [Math]::Max($SilenceSeconds, 10 * $median)
        }
        if ($private -gt $MaxPrivateBytes) { $stopReason = "private bytes $private exceeded $MaxPrivateBytes" }
        elseif ($ThroughHours.Count -gt 0 -and $started.Elapsed.TotalSeconds - $lastCommitTime -ge $blockLimit) {
            $stopReason = "block-growth/wall: block after ordinal $lastCommitOrdinal exceeded $blockLimit seconds (max of wall floor and 10x median)"
        }
        elseif ($ThroughHours.Count -eq 0 -and $started.Elapsed.TotalSeconds -ge $WallSeconds) { $stopReason = "wall time exceeded $WallSeconds seconds" }
        elseif ($started.Elapsed.TotalSeconds - $lastOutputTime -ge $silenceLimit) { $stopReason = "block-silence: no output for $silenceLimit seconds after ordinal $lastCommitOrdinal" }
        if ($null -ne $stopReason) { Stop-Process -Id $taskProcess.Id -Force -ErrorAction SilentlyContinue; break }
        Start-Sleep -Milliseconds 1000
    }
} finally {
    if ($null -ne $sampleWriter) { $sampleWriter.Dispose() }
    $taskProcess.Refresh()
    if (-not $taskProcess.HasExited) { Stop-Process -Id $taskProcess.Id -Force -ErrorAction SilentlyContinue }
}
$taskProcess.WaitForExit()
$exitCode = [uint32]0
if (-not [PrepMeasurementExit]::GetExitCodeProcess($processHandle, [ref]$exitCode)) {
    throw "GetExitCodeProcess failed: $([Runtime.InteropServices.Marshal]::GetLastWin32Error())"
}
$perPhase = @($phaseSamples.Keys | Sort-Object | ForEach-Object {
    $value = $phaseSamples[$_]
    [pscustomobject]@{ phase = $_; peakSampledPrivateBytes = $value.private; peakSampledWorkingSetBytes = $value.working; meanMachineCpuPercent = $(if ($value.cpuCount -gt 0) { $value.cpuSum / $value.cpuCount } else { $null }) }
})
$processSummary = [pscustomobject]@{
    mode = $Mode
    allocator = 'mimalloc'
    exitCode = $exitCode
    stoppedByGuard = $stopReason
    peakWorkingSetBytes = $peakWorkingSet
    peakSampledPrivateBytes = $cumulativePeakPrivate
    runPeakSampledPrivateBytes = $peakPrivate
    meanMachineCpuPercent = $(if ($cpuCount -gt 0) { $cpuSum / $cpuCount } else { $null })
    machineLoadSamplesFile = 'memory-samples.csv'
    run = $runStamp
    resumed = [bool]$Resume
    processorAffinityMask = $ProcessorAffinity.ToString()
    throughHours = $ThroughHours
    cpuSampleFailures = $cpuSampleFailures
    lastCpuSampleError = $lastCpuSampleError
    logicalCpus = $machine.NumberOfLogicalProcessors
    physicalRamBytes = $machine.TotalPhysicalMemory
    perPhase = $perPhase
    wallLimitSeconds = $WallSeconds
    silenceLimitSeconds = $SilenceSeconds
    privateByteLimit = $MaxPrivateBytes
    blockGuard = 'ThroughHours uses per-block max(WallSeconds, 10x rolling median of up to 31 blocks); silence uses the same median with SilenceSeconds as a floor.'
    scope = 'Actual dashboard preparation with all observers. Overall process samples include setup and source opening; perPhase separates those from preparation. Private-byte peaks and mean total-machine CPU are sampled; working-set peak uses the process lifetime counter. Timings and decode/observe split are in preparation-measurement.json.'
}
$summaryJson = $processSummary | ConvertTo-Json -Depth 6
[IO.File]::WriteAllText((Join-Path $evidencePath 'process-measurement-metadata.json'), $summaryJson)
# Include the machine-load samples in the JSON without collecting a history-sized
# array in PowerShell. The small metadata file above supports bounded resume reads.
$processReportTemporary = Join-Path $evidencePath 'process-measurement.json.tmp'
$processReportWriter = [IO.StreamWriter]::new($processReportTemporary, $false)
$loadReader = [IO.StreamReader]::new($samplePath)
try {
    $trimmedSummary = $summaryJson.TrimEnd()
    $processReportWriter.Write($trimmedSummary.Substring(0, $trimmedSummary.Length - 1))
    $processReportWriter.Write(',"machineLoadSamples":[')
    $null = $loadReader.ReadLine() # CSV header.
    $firstSample = $true
    while ($null -ne ($sampleLine = $loadReader.ReadLine())) {
        $fields = $sampleLine.Split(',')
        if ($fields.Count -ne 7) { throw 'Malformed machine-load CSV sample; evidence retained.' }
        $loadSample = [pscustomobject]@{
            run = $fields[0]
            seconds = [double]::Parse($fields[1], [Globalization.CultureInfo]::InvariantCulture)
            phase = $fields[2]
            machineCpuPercent = $(if ($fields[5] -ne '') { [double]::Parse($fields[5], [Globalization.CultureInfo]::InvariantCulture) } else { $null })
        }
        if (-not $firstSample) { $processReportWriter.Write(',') }
        $processReportWriter.Write(($loadSample | ConvertTo-Json -Compress))
        $firstSample = $false
    }
    $processReportWriter.Write(']}')
} finally {
    $loadReader.Dispose()
    $processReportWriter.Dispose()
}
Move-Item -LiteralPath $processReportTemporary -Destination (Join-Path $evidencePath 'process-measurement.json') -Force
if ($null -ne $stopReason) { throw "Measurement stopped: $stopReason; evidence retained in $evidencePath" }
if ($exitCode -ne 0) { throw "Measurement process exited $exitCode; inspect $errorPath" }
if (-not (Get-Content -LiteralPath $outputPath | Select-String '^RESULT\s')) { throw 'No RESULT line; this was not a completed preparation measurement.' }
Get-Content -LiteralPath (Join-Path $evidencePath 'preparation-measurement.json')
