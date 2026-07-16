[CmdletBinding(SupportsShouldProcess=$true,ConfirmImpact='High')]
param(
    [Parameter(Mandatory=$true)][string]$MarkerPath
)

$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$scriptDir=Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir 'ProductionCapture.Validation.ps1')

if (-not (Test-Path -LiteralPath $MarkerPath -PathType Leaf)) { throw "attempt marker not found: $MarkerPath" }
$marker=Get-Content -LiteralPath $MarkerPath -Raw | ConvertFrom-Json
if ([string]$marker.schema -ne 'battle-plan.production_capture_attempt.v1' -or [string]$marker.state -eq 'validation_passed') {
    throw 'marker is not an unresolved production capture attempt'
}
$pidValue=[int]$marker.pid
$childProperty=$marker.PSObject.Properties['child_pid']
if ($null -ne $childProperty -and $null -ne (Get-Process -Id ([int]$childProperty.Value) -ErrorAction SilentlyContinue)) {
    throw "recorded collector_record child PID $($childProperty.Value) is still alive; refusing to resolve the attempt"
}
$target=Get-Process -Id $pidValue -ErrorAction SilentlyContinue
$resolution='target_absent'
if ($null -ne $target) {
    $expectedStart=[DateTime]::Parse([string]$marker.process_start_utc).ToUniversalTime()
    $actualStart=$target.StartTime.ToUniversalTime()
    if ([Math]::Abs(($actualStart-$expectedStart).TotalSeconds) -lt 1.0) {
        $collectorLoaded=$true
        try {
            $staged=[System.IO.Path]::GetFullPath([string]$marker.staged_dll)
            $collectorLoaded=@($target.Modules | Where-Object {
                $_.ModuleName -ieq 'collector.dll' -or ([System.IO.Path]::GetFullPath($_.FileName) -ieq $staged)
            }).Count -ne 0
        }
        catch { throw "cannot prove collector.dll absent from the original target process: $($_.Exception.Message)" }
        if ($collectorLoaded) { throw 'original the game process still has collector.dll loaded; refusing to resolve the attempt' }
        $resolution='original_target_alive_collector_absent'
    }
    else { $resolution='pid_reused_original_target_absent' }
}
if (-not $PSCmdlet.ShouldProcess($MarkerPath,"resolve attempt after proof: $resolution")) { return }
$resolved=[ordered]@{}
foreach($property in $marker.PSObject.Properties){$resolved[$property.Name]=$property.Value}
$resolved.state='manually_resolved'; $resolved.resolution=$resolution; $resolved.resolved_utc=[DateTime]::UtcNow.ToString('o')
$destination=Join-Path (Split-Path -Parent $MarkerPath) (([System.IO.Path]::GetFileNameWithoutExtension([System.IO.Path]::GetFileNameWithoutExtension($MarkerPath)))+'.resolved.json')
$resolved | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $destination -Encoding UTF8
Move-Item -LiteralPath $MarkerPath -Destination ($destination+'.consumed')
Write-Host "attempt resolved: $destination"
