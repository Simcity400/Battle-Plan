param(
    [Parameter(Mandatory = $true)]
    [string]$Manifest,
    [Parameter(Mandatory = $true)]
    [string]$Output
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$verify = Join-Path $repo "extractor\re_evidence\A6_A7_SUPPLY_CONVOY\verify_h_convoy_fixture.py"
& python $verify ([System.IO.Path]::GetFullPath($Manifest)) --out ([System.IO.Path]::GetFullPath($Output))
if ($LASTEXITCODE -ne 0) {
    throw "H-CONVOY fixture verification failed with exit code $LASTEXITCODE"
}
Write-Output ([System.IO.Path]::GetFullPath($Output))
