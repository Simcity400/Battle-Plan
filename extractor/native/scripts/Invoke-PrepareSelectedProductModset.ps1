[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][int]$Hoi4Pid,
    [Parameter(Mandatory=$true)][string]$Hoi4Exe,
    [Parameter(Mandatory=$true)][string]$PdxUserDir,
    [Parameter(Mandatory=$true)][string]$Ui14CaptureRoot,
    [Parameter(Mandatory=$true)][string]$CaptureTimeDlcLoad,
    [Parameter(Mandatory=$true)][string]$OutDir,
    [string]$PythonExe='python.exe'
)

$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$scriptDir=Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot=(Resolve-Path -LiteralPath (Join-Path $scriptDir '..\..\..')).Path
$materializer=Join-Path $repoRoot 'extractor\game_defs\materialize_ui14_engine_vfs.py'
$preparer=Join-Path $repoRoot 'extractor\game_defs\prepare_product_modset_capture.py'
foreach($path in @($Hoi4Exe,$PdxUserDir,$Ui14CaptureRoot,$CaptureTimeDlcLoad,$materializer,$preparer)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required selected-modset input is missing: $path" }
}
$OutDir=[System.IO.Path]::GetFullPath($OutDir)
if (Test-Path -LiteralPath $OutDir) { throw "Selected product-modset output already exists: $OutDir" }
$parent=Split-Path -Parent $OutDir
if (-not (Test-Path -LiteralPath $parent -PathType Container)) { New-Item -ItemType Directory -Path $parent | Out-Null }
$temporaryReceipt=Join-Path $parent ".$([System.IO.Path]::GetFileName($OutDir)).$([guid]::NewGuid().ToString('N')).ui14_engine_vfs.json"
$python=@(Get-Command $PythonExe -CommandType Application -ErrorAction Stop)[0].Source
try {
    & $python $materializer `
        --capture-root (Resolve-Path -LiteralPath $Ui14CaptureRoot).Path `
        --active-dlc-load (Resolve-Path -LiteralPath $CaptureTimeDlcLoad).Path `
        --userdir (Resolve-Path -LiteralPath $PdxUserDir).Path `
        --exe (Resolve-Path -LiteralPath $Hoi4Exe).Path `
        --out $temporaryReceipt
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $temporaryReceipt -PathType Leaf)) {
        throw 'Selected-modset UI14 materialization failed.'
    }
    & $python $preparer `
        --exe (Resolve-Path -LiteralPath $Hoi4Exe).Path `
        --userdir (Resolve-Path -LiteralPath $PdxUserDir).Path `
        --pid $Hoi4Pid `
        --ui14-engine-vfs $temporaryReceipt `
        --ui14-capture-root (Resolve-Path -LiteralPath $Ui14CaptureRoot).Path `
        --out $OutDir
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath (Join-Path $OutDir 'product_modset_capture_ready.json') -PathType Leaf)) {
        throw 'Selected product-modset preparation failed.'
    }
} finally {
    if (Test-Path -LiteralPath $temporaryReceipt -PathType Leaf) { Remove-Item -LiteralPath $temporaryReceipt -Force }
}
Write-Host "Process-bound selected product-modset handoff retained: $OutDir"
