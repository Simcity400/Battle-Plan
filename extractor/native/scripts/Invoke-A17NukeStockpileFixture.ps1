[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [Alias("Pid")]
    [int]$ProcessId,

    [Parameter(Mandatory = $true)]
    [string]$SaveOracle,

    [Parameter(Mandatory = $true)]
    [string]$ProductionSnapshot,

    [Parameter(Mandatory = $true)]
    [string]$OutDir,

    [string]$PythonExe = "python"
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$packet = Join-Path $repo "extractor\re_evidence\A17_NUKE_STOCKPILE"
$captureTool = Join-Path $packet "capture_a17_nuke_fixture.py"
$verifyTool = Join-Path $packet "verify_a17_nuke_fixture.py"
$oracle = (Resolve-Path -LiteralPath $SaveOracle).Path
$snapshot = (Resolve-Path -LiteralPath $ProductionSnapshot).Path

New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
$out = (Resolve-Path -LiteralPath $OutDir).Path
$capture = Join-Path $out "a17_raw_nuke_fixture.json"
$report = Join-Path $out "a17_nuke_fixture_regrade.json"
if ((Test-Path -LiteralPath $capture) -or (Test-Path -LiteralPath $report)) {
    throw "Refusing to overwrite an existing A17 evidence bundle in $out"
}

& $PythonExe $captureTool `
    --pid $ProcessId `
    --save-oracle $oracle `
    --production-snapshot $snapshot `
    --out $capture
if ($LASTEXITCODE -ne 0) {
    throw "A17 raw nuke acquisition failed"
}

& $PythonExe $verifyTool `
    --capture $capture `
    --save-oracle $oracle `
    --production-snapshot $snapshot `
    --out $report
if ($LASTEXITCODE -ne 0) {
    throw "A17 independent regrade failed; evidence and report were preserved in $out"
}

Write-Host "A17 positive nuke fixture passed: $report"
