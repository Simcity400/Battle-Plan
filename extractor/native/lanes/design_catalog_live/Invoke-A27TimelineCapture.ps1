[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [uint32]$GamePid,

    [Parameter(Mandatory = $true)]
    [ValidateRange(1, 86400)]
    [int]$DurationSeconds,

    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory,

    [string]$Python = "python"
)

$ErrorActionPreference = "Stop"
$lane = Split-Path -Parent $MyInvocation.MyCommand.Path
$outputRoot = [System.IO.Path]::GetFullPath($OutputDirectory)
[System.IO.Directory]::CreateDirectory($outputRoot) | Out-Null
$timeline = Join-Path $outputRoot "a27_live_catalog_timeline.source.json"
$report = Join-Path $outputRoot "a27_live_catalog_timeline.regrade.json"
$ready = Join-Path $outputRoot "a27_live_catalog_timeline.ready"
$stop = Join-Path $outputRoot "a27_live_catalog_timeline.stop"
$log = Join-Path $outputRoot "a27_live_catalog_timeline.log"
$errorLog = Join-Path $outputRoot "a27_live_catalog_timeline.error.log"

foreach ($path in @($timeline, $report, $ready, $stop, $log, $errorLog)) {
    if (Test-Path -LiteralPath $path) {
        Remove-Item -LiteralPath $path -Force
    }
}

$arguments = @(
    ('"{0}"' -f (Join-Path $lane "capture_timeline.py")),
    "--pid", [string]$GamePid,
    "--timeline-out", ('"{0}"' -f $timeline),
    "--ready-file", ('"{0}"' -f $ready),
    "--stop-file", ('"{0}"' -f $stop)
)
$producer = Start-Process -FilePath $Python -ArgumentList $arguments -PassThru `
    -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError $errorLog
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while (-not (Test-Path -LiteralPath $ready)) {
        if ($producer.HasExited) {
            throw "A27 timeline producer exited before the game-state readiness barrier."
        }
        if ([DateTime]::UtcNow -ge $deadline) {
            throw "A27 timeline producer did not reach the game-state readiness barrier."
        }
        Start-Sleep -Milliseconds 50
    }
    $captureDeadline = [DateTime]::UtcNow.AddSeconds($DurationSeconds)
    while ([DateTime]::UtcNow -lt $captureDeadline -and -not $producer.HasExited) {
        Start-Sleep -Milliseconds 100
    }
}
finally {
    [System.IO.File]::WriteAllText($stop, "stop")
}

$producer.WaitForExit()
if (Test-Path -LiteralPath $timeline) {
    & $Python (Join-Path $lane "regrade_timeline.py") `
        --timeline $timeline `
        --report-out $report `
        --require-mutation `
        --minimum-captures 3
    $regradeExitCode = $LASTEXITCODE
}
else {
    $regradeExitCode = -1
}
if ($producer.ExitCode -ne 0 -or $regradeExitCode -ne 0) {
    throw "A27 timeline evidence failed (producer=$($producer.ExitCode), regrade=$regradeExitCode). Retain $timeline, $report, $log, and $errorLog as the fail-closed in-process-boundary receipt."
}

Write-Host "A27 timeline PASS: $timeline"
Write-Host "Independent regrade: $report"
