<#
.SYNOPSIS
Retry the headless raw capture loop until one attempt finalizes cleanly.

Injecting the collector into a live the game intermittently kills the game with an
access violation calling address 0x0 on a tbb.dll worker thread. A control run
(same launch, same speed, no recorder) survived 10+ minutes untouched, so the
crash is caused by hook installation, not by the save or the engine. Roughly one
attempt in three currently survives injection.

This driver does NOT fix that race. It retries around it so a clean full-data
capture can actually be obtained, and it records how many attempts died so the
failure rate stays visible instead of being smoothed away.
#>
[CmdletBinding()]
param(
    [ValidateRange(1, 25)]
    [int]$MaxAttempts = 8,

    [ValidateRange(10, 86400)]
    [double]$DurationSeconds = 180,

    [ValidateRange(1, 5)]
    [int]$Speed = 5,

    [ValidateRange(0, 100000)]
    [int]$MinGameHours = 48,

    [ValidateRange(0, 900)]
    [int]$SettleSeconds = 30,

    [string]$RunRoot = 'extractor\native\runs',

    [string]$Tag = 'headless_clean'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$capture = Join-Path $scriptDir 'Invoke-HeadlessRawCapture.ps1'
$report = Join-Path $scriptDir 'report_raw_capture.py'
$python = @(Get-Command python.exe -CommandType Application -ErrorAction Stop)[0].Source
$crashRoot = 'C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\crashes'

$attempts = @()
for ($attempt = 1; $attempt -le $MaxAttempts; $attempt++) {
    $outDir = Join-Path $RunRoot ("{0}_a{1}" -f $Tag, $attempt)
    Remove-Item -Recurse -Force $outDir -ErrorAction SilentlyContinue
    Get-Process -Name the game, collector_record -ErrorAction SilentlyContinue | ForEach-Object {
        try { $_.Kill(); [void]$_.WaitForExit(20000) } catch { }
    }
    $crashesBefore = @(Get-ChildItem $crashRoot -ErrorAction SilentlyContinue).Count

    Write-Host ""
    Write-Host "===== attempt $attempt/$MaxAttempts -> $outDir ====="
    # A dying attempt writes to stderr; under ErrorActionPreference=Stop that
    # surfaces as a terminating NativeCommandError and would abort the whole
    # sweep after one crash. Retrying around the crash is the entire point.
    try {
        $ErrorActionPreference = 'Continue'
        & $capture -DurationSeconds $DurationSeconds -Speed $Speed -MinGameHours $MinGameHours `
            -SettleSeconds $SettleSeconds -OutDir $outDir 2>&1 | Write-Host
    } catch {
        Write-Host "attempt $attempt threw: $($_.Exception.Message)"
    } finally {
        $ErrorActionPreference = 'Stop'
    }

    $crashesAfter = @(Get-ChildItem $crashRoot -ErrorAction SilentlyContinue).Count
    $gameCrashed = $crashesAfter -gt $crashesBefore
    $manifest = Join-Path $outDir 'native_session_manifest.json'
    $haveManifest = Test-Path -LiteralPath $manifest -PathType Leaf

    $verdict = if ($gameCrashed) { 'game_crashed_on_injection' }
               elseif (-not $haveManifest) { 'no_manifest' }
               else { 'finalized' }
    $attempts += [pscustomobject]@{ Attempt = $attempt; OutDir = $outDir; Verdict = $verdict }
    Write-Host "attempt=$attempt verdict=$verdict game_crashed=$gameCrashed manifest=$haveManifest"

    if ($haveManifest) {
        $reportExit = 1
        try {
            $ErrorActionPreference = 'Continue'
            & $python $report --run-dir $outDir 2>&1 | Write-Host
            $reportExit = $LASTEXITCODE
        } catch {
            Write-Host "report threw: $($_.Exception.Message)"
        } finally {
            $ErrorActionPreference = 'Stop'
        }
        Write-Host "report_exit=$reportExit"
        if ($reportExit -eq 0) {
            Write-Host ""
            Write-Host "CLEAN CAPTURE on attempt $attempt : $outDir"
            $attempts | Format-Table -AutoSize | Out-String | Write-Host
            exit 0
        }
        # A finalized-but-flagged capture is still the useful artifact to read;
        # keep going only if attempts remain.
        Write-Host "attempt $attempt finalized but the domain report flagged problems."
    }
}

Write-Host ""
Write-Host "No fully clean capture within $MaxAttempts attempts."
$attempts | Format-Table -AutoSize | Out-String | Write-Host
$crashed = @($attempts | Where-Object { $_.Verdict -eq 'game_crashed_on_injection' }).Count
Write-Host "injection-crash rate: $crashed/$MaxAttempts"
exit 1
