[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $HarnessExe,
    [Parameter(Mandatory = $true)] [string] $Hoi4Exe,
    [Parameter(Mandatory = $true)] [string] $CollectorDll,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [string[]] $HarnessArgument = @(),
    [string] $Snapshot,
    [string] $Python = 'python'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$expectedExeSha256 = '870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad'
foreach ($path in @($HarnessExe, $Hoi4Exe, $CollectorDll)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required file does not exist: $path"
    }
}
$resolvedHarness = (Resolve-Path -LiteralPath $HarnessExe).Path
$resolvedExe = (Resolve-Path -LiteralPath $Hoi4Exe).Path
$resolvedCollector = (Resolve-Path -LiteralPath $CollectorDll).Path
$resolvedOut = [System.IO.Path]::GetFullPath($OutDir)
if ((Get-FileHash -LiteralPath $resolvedExe -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedExeSha256) {
    throw 'Hoi4Exe does not match exact a729 authority.'
}
if (Test-Path -LiteralPath $resolvedOut) {
    if (-not (Test-Path -LiteralPath $resolvedOut -PathType Container) -or
        @(Get-ChildItem -LiteralPath $resolvedOut -Force).Count -ne 0) {
        throw "OutDir must be absent or empty so stale A24 evidence cannot be selected: $resolvedOut"
    }
} else {
    [System.IO.Directory]::CreateDirectory($resolvedOut) | Out-Null
}

$repo = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$regrader = Join-Path $repo 'extractor\native\lanes\politics_tooltip_inputs_live\regrade_a24_live_fixture.py'
$producerReceiptPattern = Join-Path $resolvedOut 'a24_politics_producer_capture_*.json'
$verification = Join-Path $resolvedOut 'a24_politics_tooltip_live_fixture_regrade.json'

# The harness receives only paths and must perform the in-process chronology:
# install -> for each mutually exclusive surface: arm(frame hour) -> scripted UI
# work -> selected Present -> prepare/copy one case receipt -> uninstall. The DLL
# exports enforce each case boundary and the regrader enforces the 23-tap union;
# this runner never manufactures rows, hours, Present identities, or pointers.
$env:BATTLE_PLAN_A24_GAME_EXE = $resolvedExe
$env:BATTLE_PLAN_A24_COLLECTOR_DLL = $resolvedCollector
$env:BATTLE_PLAN_A24_OUTPUT_DIR = $resolvedOut
$env:BATTLE_PLAN_A24_PRODUCER_RECEIPT_PATTERN = $producerReceiptPattern
try {
    & $resolvedHarness @HarnessArgument
    if ($LASTEXITCODE -ne 0) {
        throw "A24 owner harness exited with code $LASTEXITCODE; evidence was retained at $resolvedOut"
    }
} finally {
    Remove-Item Env:BATTLE_PLAN_A24_GAME_EXE -ErrorAction SilentlyContinue
    Remove-Item Env:BATTLE_PLAN_A24_COLLECTOR_DLL -ErrorAction SilentlyContinue
    Remove-Item Env:BATTLE_PLAN_A24_OUTPUT_DIR -ErrorAction SilentlyContinue
    Remove-Item Env:BATTLE_PLAN_A24_PRODUCER_RECEIPT_PATTERN -ErrorAction SilentlyContinue
}
$producerReceipts = @(Get-ChildItem -Path $producerReceiptPattern -File | Sort-Object Name)
if ($producerReceipts.Count -eq 0) {
    throw 'The in-process owner harness did not write any A24 producer capture-case receipts.'
}

$regradeArgs = @($regrader)
foreach ($receipt in $producerReceipts) {
    $regradeArgs += @('--producer-receipt', $receipt.FullName)
}
$regradeArgs += @('--game-exe', $resolvedExe, '--output', $verification)
if ($Snapshot) {
    if (-not (Test-Path -LiteralPath $Snapshot -PathType Leaf)) {
        throw "Snapshot does not exist: $Snapshot"
    }
    $regradeArgs += @('--snapshot', (Resolve-Path -LiteralPath $Snapshot).Path)
}
& $Python @regradeArgs
if ($LASTEXITCODE -ne 0) {
    throw "A24 fixture is incomplete or invalid; evidence was retained at $resolvedOut"
}
Write-Output "A24 owner capture accepted for d73 latch review: $verification"
