[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][int]$ProcessId,
    [Parameter(Mandatory = $true)][string]$CollectorDll,
    [Parameter(Mandatory = $true)][string]$OutputRoot,
    [Parameter(Mandatory = $true)][ValidatePattern('^[A-Z0-9]{3}$')][string]$CountryTag,
    [Parameter(Mandatory = $true)][string]$CountryObjectAddress,
    [Parameter(Mandatory = $true)][UInt64]$StableFrameHour,
    [Parameter(Mandatory = $true)][int]$StableGameHourRaw,
    [Parameter(Mandatory = $true)][UInt64]$TransitionFrameHour,
    [Parameter(Mandatory = $true)][int]$TransitionGameHourRaw,
    [string]$Python = 'python'
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$driver = Join-Path $repo 'extractor\native\lanes\focus_research_decisions_depth_live\tools\capture_a31_fixture_pair.py'
$collector = (Resolve-Path -LiteralPath $CollectorDll).Path
$output = [System.IO.Path]::GetFullPath($OutputRoot)

if (-not [System.IO.Path]::IsPathRooted($output)) {
    throw 'A31 output root must be absolute'
}
if ($TransitionFrameHour -le $StableFrameHour) {
    throw 'A31 transition frame hour must be later than the stable frame hour'
}

& $Python $driver `
    --process-id $ProcessId `
    --collector-dll $collector `
    --output-root $output `
    --country-tag $CountryTag `
    --country-object-address $CountryObjectAddress `
    --stable-frame-hour $StableFrameHour `
    --stable-game-hour-raw $StableGameHourRaw `
    --transition-frame-hour $TransitionFrameHour `
    --transition-game-hour-raw $TransitionGameHourRaw

if ($LASTEXITCODE -ne 0) {
    throw "A31 fixture-pair capture/regrade failed with exit code $LASTEXITCODE"
}
