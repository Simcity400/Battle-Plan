[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [uint32] $ExistingPid,
    [Parameter(Mandatory = $true)] [string] $CaptureManifest,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [string] $ProfilePath = '',
    [string] $SourceInventory = '',
    [string] $Python = 'python',
    [int] $ProcessTimeoutSeconds = 300
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$extractorRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if ([string]::IsNullOrWhiteSpace($ProfilePath)) {
    $ProfilePath = Join-Path $extractorRoot 'documents\dossiers\A28\A28_AIR_MIO_AUTONOMOUS_ACTIONS_1920X1080.json'
}
$resolvedOut = [IO.Path]::GetFullPath($OutDir)
[IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
$actionReceipt = Join-Path $resolvedOut 'a28_air_mio_action_receipt.json'
$actionArguments = @(
    '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
    '-File', (Join-Path $PSScriptRoot 'Invoke-A28AirMioActions.ps1'),
    '-Hoi4Exe', $Hoi4Exe, '-CollectorDll', $CollectorDll, '-ProfilePath', $ProfilePath,
    '-ReceiptPath', $actionReceipt, '-ExistingPid', $ExistingPid,
    '-Python', $Python, '-ProcessTimeoutSeconds', $ProcessTimeoutSeconds
)
if (-not [string]::IsNullOrWhiteSpace($SourceInventory)) {
    $actionArguments += @('-SourceInventory', $SourceInventory)
}
& powershell.exe @actionArguments
if ($LASTEXITCODE -ne 0) { throw 'A28 authenticated action phase failed.' }

$names = @(
    'a28_air_selection_controller_observation.json',
    'a28_mio_aggregated_bonus_controller_observation.json',
    'a28_mio_variant_history_controller_observation.json',
    'a28_mio_production_history_controller_observation.json'
)
$arguments = @(
    (Join-Path $extractorRoot 'native\lanes\dynamic_collection_trace_live\verify_a28_air_mio_acquisition.py'),
    '--action-receipt', $actionReceipt,
    '--profile-verification', (Join-Path $resolvedOut 'a28_action_profile.static_verification.json'),
    '--capture-manifest', (Resolve-Path -LiteralPath $CaptureManifest).Path,
    '--collector-dll', (Resolve-Path -LiteralPath $CollectorDll).Path,
    '--out', (Join-Path $resolvedOut 'a28_air_mio_acquisition_admission.json')
)
foreach ($name in $names) {
    $path = Join-Path $resolvedOut $name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "A28 raw controller observation is absent (unavailable, never measured-empty): $name"
    }
    $arguments += @('--observation', $path)
}
& $Python @arguments
if ($LASTEXITCODE -ne 0) { throw 'A28 four-family aggregate admission failed closed.' }
