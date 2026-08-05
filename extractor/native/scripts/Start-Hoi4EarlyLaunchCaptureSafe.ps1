[CmdletBinding()]
param(
    [switch]$OperatorApproved,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptDir '..\..\..\portable_replay\native\scripts\Hoi4ClientDriving.OptIn.ps1')
Assert-Hoi4ClientDrivingOptIn -Caller $MyInvocation.MyCommand.Path

# Retained as a fail-closed migration tombstone because historical evidence and
# dossiers name this path. Recorder-owned game creation, suspended-thread
# control, window control, and clock control are not product capabilities.
# Evidence setup that genuinely needs to drive the game must use the separately
# opted-in headless harness, complete all driving before its graded window, and
# then observe through the passive collector/read-only witness.
$result = [pscustomobject][ordered]@{
    schema = 'battle-plan.retired_early_launch_evidence_tool.v1'
    status = 'retired'
    admissible_for_production = $false
    client_driving_evidence_only = $true
    replacement = 'Invoke-HeadlessRawCapture.ps1 -EvidenceOnlyDriveGame'
    graded_window_policy = 'zero client commands from attach through authenticated teardown'
}

if ($DryRun -and $OperatorApproved) {
    $result | ConvertTo-Json -Depth 3
    exit 0
}

throw (
    'Recorder-owned early launch is retired. Use the explicitly opted-in ' +
    'evidence harness for pre-window setup; the shipped recorder attaches ' +
    'passively to a user-owned running game.'
)
