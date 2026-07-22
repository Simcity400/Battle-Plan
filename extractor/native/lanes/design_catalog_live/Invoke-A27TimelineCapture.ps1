[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [uint32]$GamePid,

    [Parameter(Mandatory = $true)]
    [ValidateRange(60, 86400)]
    [int]$DurationSeconds,

    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory,

    [string]$Python = "python",

    [string]$ActionProfile = (Join-Path $PSScriptRoot "A27_AUTHENTICATED_TRANSACTIONS_1920X1080.json")
)

$ErrorActionPreference = "Stop"
$lane = Split-Path -Parent $MyInvocation.MyCommand.Path
$outputRoot = [System.IO.Path]::GetFullPath($OutputDirectory)
[string]$profile = (Resolve-Path -LiteralPath $ActionProfile).Path
if (Test-Path -LiteralPath $outputRoot) {
    throw "A27 OutputDirectory must be new so the receipt set is immutable: $outputRoot"
}
[System.IO.Directory]::CreateDirectory($outputRoot) | Out-Null
$timeline = Join-Path $outputRoot "a27_live_catalog_timeline.source.json"
$report = Join-Path $outputRoot "a27_live_catalog_timeline.regrade.json"
$ready = Join-Path $outputRoot "a27_live_catalog_timeline.ready"
$stop = Join-Path $outputRoot "a27_live_catalog_timeline.stop"
$log = Join-Path $outputRoot "a27_live_catalog_timeline.log"
$errorLog = Join-Path $outputRoot "a27_live_catalog_timeline.error.log"
$actionDirectory = Join-Path $outputRoot "authenticated_actions"
$actionReceipt = Join-Path $outputRoot "a27_authenticated_designer_transactions.receipt.json"

$arguments = @(
    ('"{0}"' -f (Join-Path $lane "capture_timeline.py")),
    "--pid", [string]$GamePid,
    "--timeline-out", ('"{0}"' -f $timeline),
    "--ready-file", ('"{0}"' -f $ready),
    "--stop-file", ('"{0}"' -f $stop),
    "--action-directory", ('"{0}"' -f $actionDirectory)
)
$producer = Start-Process -FilePath $Python -ArgumentList $arguments -PassThru `
    -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError $errorLog
$actionFailure = $null
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
    $game = Get-Process -Id $GamePid -ErrorAction Stop
    & (Join-Path $lane "Invoke-A27AuthenticatedTransactions.ps1") `
        -GamePid $GamePid `
        -ExpectedProcessStartUtc ($game.StartTime.ToUniversalTime().ToString('o')) `
        -ProfilePath $profile `
        -ActionDirectory $actionDirectory `
        -CaptureReadyFile $ready `
        -ReceiptPath $actionReceipt `
        -HandshakeTimeoutSeconds $DurationSeconds
}
catch {
    $actionFailure = $_
}
finally {
    [System.IO.File]::WriteAllText($stop, "stop")
}

$producer.WaitForExit()
if (Test-Path -LiteralPath $timeline) {
    $regradeArguments = @(
        (Join-Path $lane "regrade_timeline.py"),
        "--timeline", $timeline,
        "--report-out", $report,
        "--require-mutation",
        "--minimum-captures", "5",
        "--require-transaction", "rename",
        "--require-transaction", "edit",
        "--require-transaction", "obsolete",
        "--require-transaction", "reactivate"
    )
    if (Test-Path -LiteralPath $actionReceipt -PathType Leaf) {
        $regradeArguments += @("--action-receipt", $actionReceipt)
    }
    & $Python @regradeArguments
    $regradeExitCode = $LASTEXITCODE
}
else {
    $regradeExitCode = -1
}
if ($null -ne $actionFailure -or $producer.ExitCode -ne 0 -or $regradeExitCode -ne 0) {
    $actionMessage = if ($null -eq $actionFailure) { "none" } else { $actionFailure.Exception.Message }
    throw "A27 timeline evidence failed (action=$actionMessage, producer=$($producer.ExitCode), regrade=$regradeExitCode). Retain $timeline, $report, $actionDirectory, $log, and $errorLog as the fail-closed receipt set."
}

Write-Host "A27 timeline PASS: $timeline"
Write-Host "Independent regrade: $report"
Write-Host "Authenticated action receipt: $actionReceipt"
