[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [Alias("Pid")]
    [int]$ProcessId,

    [Parameter(Mandatory = $true)]
    [string]$AbilitiesJson,

    [Parameter(Mandatory = $true)]
    [string]$ProductionSnapshot,

    [Parameter(Mandatory = $true)]
    [string]$OutDir,

    [string]$PythonExe = "python"
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$packet = Join-Path $repo "extractor\re_evidence\A5_ACE_LIFECYCLE"
$captureTool = Join-Path $packet "capture_a5_lifecycle_fixture.py"
$verifyTool = Join-Path $packet "verify_a5_lifecycle_fixture.py"
$abilities = (Resolve-Path -LiteralPath $AbilitiesJson).Path
$snapshot = (Resolve-Path -LiteralPath $ProductionSnapshot).Path

New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
$out = (Resolve-Path -LiteralPath $OutDir).Path
$capture = Join-Path $out "a5_raw_lifecycle_fixture.json"
$report = Join-Path $out "a5_lifecycle_regrade.json"
if ((Test-Path -LiteralPath $capture) -or (Test-Path -LiteralPath $report)) {
    throw "Refusing to overwrite an existing A5 evidence bundle in $out"
}

& $PythonExe $captureTool `
    --pid $ProcessId `
    --abilities $abilities `
    --production-snapshot $snapshot `
    --out $capture
if ($LASTEXITCODE -ne 0) {
    throw "A5 raw lifecycle acquisition failed"
}

& $PythonExe $verifyTool `
    --capture $capture `
    --abilities $abilities `
    --production-snapshot $snapshot `
    --out $report
if ($LASTEXITCODE -ne 0) {
    throw "A5 independent lifecycle regrade failed; evidence and report were preserved in $out"
}

Write-Host "A5 lifecycle fixture passed: $report"
