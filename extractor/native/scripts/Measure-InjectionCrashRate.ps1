<#
.SYNOPSIS
Measure how the collector's injection-crash rate scales with hook count.

Injecting the collector intermittently kills the game with an access violation
calling address 0x0 on a tbb.dll worker thread. MinHook's `Freeze` suspends
only the threads present in a point-in-time Toolhelp snapshot, so a thread
created after that snapshot executes the half-written 5-byte JMP. Per-target
`MH_EnableHook` opens one such window PER HOOK; `MH_EnableHook(MH_ALL_HOOKS)`
opens exactly one for all of them.

If that is the mechanism, crash rate should fall sharply with the number of
scheduled domains (hence hooks). This measures that before any refactor,
so a ~40-site change is not made on an unverified hypothesis.

Each trial only needs to reach injection, so it uses a short capture.
#>
[CmdletBinding()]
param(
    [ValidateRange(1, 50)]
    [int]$TrialsPerArm = 5,

    [ValidateRange(10, 600)]
    [double]$DurationSeconds = 45,

    [ValidateRange(0, 900)]
    [int]$SettleSeconds = 20,

    [string]$RunRoot = 'extractor\native\runs\crashrate'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$capture = Join-Path $scriptDir 'Invoke-HeadlessRawCapture.ps1'
$crashRoot = 'C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\crashes'

# Arms are chosen so hook count differs by roughly an order of magnitude while
# both remain valid production wire names.
$arms = @(
    [pscustomobject]@{ Name = 'minimal'; Domains = @('session_header_live', 'fuel_live') }
    [pscustomobject]@{ Name = 'full';    Domains = @() }
)

New-Item -ItemType Directory -Force -Path $RunRoot | Out-Null
$results = @()

foreach ($arm in $arms) {
    for ($trial = 1; $trial -le $TrialsPerArm; $trial++) {
        $outDir = Join-Path $RunRoot ("{0}_t{1}" -f $arm.Name, $trial)
        Remove-Item -Recurse -Force $outDir -ErrorAction SilentlyContinue
        Get-Process -Name the game, collector_record -ErrorAction SilentlyContinue | ForEach-Object {
            try { $_.Kill(); [void]$_.WaitForExit(20000) } catch { }
        }
        Start-Sleep -Seconds 2
        $before = @(Get-ChildItem $crashRoot -ErrorAction SilentlyContinue).Count

        Write-Host ""
        Write-Host "===== arm=$($arm.Name) trial=$trial/$TrialsPerArm ====="
        try {
            $ErrorActionPreference = 'Continue'
            if ($arm.Domains.Count -gt 0) {
                & $capture -DurationSeconds $DurationSeconds -SettleSeconds $SettleSeconds `
                    -MinGameHours 0 -OutDir $outDir -ScheduledDomainOverride $arm.Domains 2>&1 | Write-Host
            } else {
                & $capture -DurationSeconds $DurationSeconds -SettleSeconds $SettleSeconds `
                    -MinGameHours 0 -OutDir $outDir 2>&1 | Write-Host
            }
        } catch {
            Write-Host "trial threw: $($_.Exception.Message)"
        } finally {
            $ErrorActionPreference = 'Stop'
        }

        $after = @(Get-ChildItem $crashRoot -ErrorAction SilentlyContinue).Count
        $crashed = $after -gt $before
        $results += [pscustomobject]@{ Arm = $arm.Name; Trial = $trial; Crashed = $crashed }
        Write-Host "RESULT arm=$($arm.Name) trial=$trial crashed=$crashed"
    }
}

Get-Process -Name the game, collector_record -ErrorAction SilentlyContinue | ForEach-Object {
    try { $_.Kill() } catch { }
}

Write-Host ""
Write-Host "===== injection-crash rate by arm ====="
foreach ($arm in $arms) {
    $rows = @($results | Where-Object { $_.Arm -eq $arm.Name })
    $crashes = @($rows | Where-Object { $_.Crashed }).Count
    Write-Host ("{0,-9} {1}/{2} crashed" -f $arm.Name, $crashes, $rows.Count)
}
$results | Format-Table -AutoSize | Out-String | Write-Host
